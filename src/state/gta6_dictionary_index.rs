//! The texture dictionaries the whole scans of the open GTA VI archives found, indexed by the
//! 64-bit keys of their textures' names, so a key a drawable binds opens the dictionary that
//! holds it beside the drawable, as a GTA V model opens its own dictionary. The index holds keys
//! and the bodies they are in, never pixels.

use crate::state::Rpf9Document;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9ZlibRun};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{read_dictionary, Entry};

use super::gta6_texture_naming::{NamingEnded, TextureNaming};
use super::rpf9_scan::{body_path, scanned_body, view_of, Ended, Row, READ_ONLY_BODY};
use super::{AppState, DocPayload, Document};

/// The most a dictionary body read for the index or opened from it inflates to.
const BODY_LIMIT: u64 = 1 << 30;
/// The most dictionaries one index reads.
const MOST_DICTIONARIES: usize = 1 << 20;

/// Each dictionary body by the path its scan row opens under, and the body each key is in.
#[derive(Default)]
pub(super) struct Built {
    bodies: Vec<PathBuf>,
    keys: HashMap<u64, u32>,
}

impl Built {
    /// The index of `dictionaries`, each body by its path with its textures' keys, of at most [`MOST_DICTIONARIES`].
    pub(super) fn of(dictionaries: impl IntoIterator<Item = (PathBuf, Vec<u64>)>) -> Self {
        let mut built = Self::default();
        for (path, keys) in dictionaries.into_iter().take(MOST_DICTIONARIES) {
            let at = built.bodies.len() as u32;
            for key in keys {
                built.keys.entry(key).or_insert(at);
            }
            built.bodies.push(path);
        }
        built
    }

    /// The body that holds `key`, by the path it opens under.
    pub(super) fn body_of(&self, key: u64) -> Option<&PathBuf> {
        (self.keys.get(&key)).and_then(|&at| self.bodies.get(at as usize))
    }
}

/// Whether the index `built` holds, if any, holds `key`.
fn holds_in(built: &Mutex<Option<Built>>, key: u64) -> bool {
    let built = built.lock().unwrap_or_else(|p| p.into_inner());
    built.as_ref().is_some_and(|b| b.keys.contains_key(&key))
}

/// A dictionary read on a worker for a key, by its path, and what the read gave.
type Opening = Arc<Mutex<Option<(PathBuf, Result<DocPayload, String>)>>>;

#[derive(Default)]
pub(super) struct DictionaryIndex {
    /// The archives whose whole scans the index was built from, sorted.
    archives: Vec<PathBuf>,
    built: Arc<Mutex<Option<Built>>>,
    /// The bodies opened or tried, each once.
    tried: HashSet<PathBuf>,
    /// The read in flight, if any.
    opening: Option<Opening>,
    /// The dictionaries every archive of an install holds, as its scan's harvest found them.
    pub(super) harvest: super::gta6_install_harvest::Harvest,
}

impl DictionaryIndex {
    /// Whether a dictionary of the index, or of an install's harvest, holds `key`.
    pub(super) fn holds(&self, key: u64) -> bool {
        holds_in(&self.built, key) || holds_in(&self.harvest.built, key)
    }
}

/// The archive of each open scan of a whole GTA VI archive that ended whole, sorted, and the
/// dictionary bodies their rows hold, by the paths they open under; the bodies only when `bodies`.
pub(super) fn whole_scans(documents: &[Document], bodies: bool) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut archives, mut paths) = (Vec::new(), Vec::new());
    for doc in documents {
        let DocPayload::Rpf9(Rpf9Document::Scan(scan)) = &doc.payload else {
            continue;
        };
        let found = scan.found();
        if scan.start != 0 || found.ended != Some(Ended::Whole) {
            continue;
        }
        archives.push(scan.path.clone());
        for row in found.rows.iter().filter(|_| bodies) {
            let Row::Body(find) = row else {
                continue;
            };
            let dictionary = matches!(
                find.kind,
                Rpf9BodyKind::Zlib(Rpf9ZlibRun { signature: Some(s), .. })
                    if s.role() == Some(Rpf9Role::Dictionary)
            );
            if let (true, Ok(view)) = (dictionary, view_of(find)) {
                if paths.len() < MOST_DICTIONARIES {
                    paths.push(body_path(&scan.path, find.offset, view));
                }
            }
        }
    }
    archives.sort();
    archives.dedup();
    (archives, paths)
}

/// The keys of the textures that read in each body of `bodies`, each body read and dropped in
/// turn; a body that does not read holds none. `None` once `cancel` is set.
pub(super) fn index(
    bodies: Vec<PathBuf>,
    cancel: &or_job::Cancel,
    progress: &or_job::Progress,
) -> Option<Built> {
    let mut built = Built::default();
    let total = bodies.len() as u64;
    let mut open: Option<(PathBuf, File)> = None;
    for (done, path) in bodies.into_iter().enumerate() {
        if cancel.is_set() {
            return None;
        }
        progress.reached(done as u64, total);
        let Some((archive, offset)) = scanned_body(&path).map(|(a, o)| (a.to_path_buf(), o)) else {
            continue;
        };
        if open.as_ref().is_none_or(|(at, _)| *at != archive) {
            open = File::open(&archive).ok().map(|file| (archive, file));
        }
        let Some((_, file)) = open.as_ref() else {
            continue;
        };
        let Ok((find, bytes)) = read_rpf9_body(file, offset, BODY_LIMIT) else {
            continue;
        };
        let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) = find.kind
        else {
            continue;
        };
        let (virtual_section, physical) = resource.sections(&bytes);
        let Ok(dict) = read_dictionary(virtual_section, Some(physical)) else {
            continue;
        };
        super::ttpf_document::learn_names(&dict);
        let at = built.bodies.len() as u32;
        for (_, entry) in &dict.entries {
            if let Ok(Entry::Texture(t)) = entry {
                if t.texture.is_ok() {
                    let key = name_hash64(t.name.as_bytes());
                    built.keys.entry(key).or_insert(at);
                }
            }
        }
        built.bodies.push(path);
    }
    Some(built)
}

impl AppState {
    /// Open beside the active GTA VI drawable the scanned dictionary holding a key it misses, indexing the open scans' dictionaries first as a job; whether one opened.
    pub fn try_open_texture_dictionary(&mut self) -> bool {
        if self.take_opened_dictionary() {
            return true;
        }
        let Some(doc) = self.active_document() else {
            return false;
        };
        let doc_id = doc.id;
        let misses = self.texture_report.document == Some(doc_id)
            && !self.texture_report.missing().is_empty();
        if !misses || !matches!(doc.payload, DocPayload::Gta6Drawable(_)) {
            return false;
        }
        if self.texture_packs.dictionaries.opening.is_some() {
            return false;
        }
        let (archives, _) = whole_scans(&self.documents, false);
        if archives != self.texture_packs.dictionaries.archives {
            self.index_texture_dictionaries(archives);
            return false;
        }
        let wanted: Vec<PathBuf> = {
            let held = &self.texture_packs.dictionaries;
            let open = held.built.lock().unwrap_or_else(|p| p.into_inner());
            let harvest = held.harvest.built.lock();
            let harvest = harvest.unwrap_or_else(|p| p.into_inner());
            let indexes = [open.as_ref(), harvest.as_ref()];
            (self.texture_report.missing().into_iter())
                .filter_map(super::gta6_texture_index::key_of)
                .filter_map(|key| indexes.into_iter().flatten().find_map(|i| i.body_of(key)))
                .cloned()
                .collect()
        };
        let tried = &self.texture_packs.dictionaries.tried;
        let open = |path: &Path| (self.documents.iter()).any(|d| d.path.as_deref() == Some(path));
        let next = (wanted.into_iter()).find(|path| !open(path) && !tried.contains(path));
        if let Some(path) = next {
            self.texture_packs.dictionaries.tried.insert(path.clone());
            self.read_texture_dictionary(path);
        }
        false
    }

    /// Index the dictionaries of the whole scans of `archives` on a worker, in place of any index before it.
    fn index_texture_dictionaries(&mut self, archives: Vec<PathBuf>) {
        let (_, bodies) = whole_scans(&self.documents, true);
        let held = &mut self.texture_packs.dictionaries;
        *held = DictionaryIndex {
            archives,
            harvest: std::mem::take(&mut held.harvest),
            ..DictionaryIndex::default()
        };
        if held.archives.is_empty() {
            return;
        }
        let built = Arc::clone(&held.built);
        let doing = tr!("Indexing the GTA VI texture dictionaries the open archives' scans found");
        let run = TextureNaming::run(doing.to_string(), move |cancel, progress| {
            let Some(found) = index(bodies, cancel, progress) else {
                return NamingEnded::Stopped;
            };
            let read = found.bodies.len();
            *built.lock().unwrap_or_else(|p| p.into_inner()) = Some(found);
            NamingEnded::Dictionaries(read)
        });
        self.texture_naming.push(run);
    }

    /// Read the dictionary at `path`, a scan row's body, on a worker.
    fn read_texture_dictionary(&mut self, path: PathBuf) {
        let opening: Opening = Arc::default();
        self.texture_packs.dictionaries.opening = Some(Arc::clone(&opening));
        std::thread::spawn(move || {
            let read = match scanned_body(&path) {
                Some((archive, offset)) => super::rpf9_scan::read_body(&path, archive, offset),
                None => Err(path.display().to_string()),
            };
            *opening.lock().unwrap_or_else(|p| p.into_inner()) = Some((path, read));
        });
    }

    /// Open the dictionary its worker read beside the drawable that asked for it, once read; whether one opened.
    fn take_opened_dictionary(&mut self) -> bool {
        let Some(opening) = self.texture_packs.dictionaries.opening.as_ref() else {
            return false;
        };
        let taken = opening.lock().unwrap_or_else(|p| p.into_inner()).take();
        let Some((path, read)) = taken else {
            return false;
        };
        self.texture_packs.dictionaries.opening = None;
        let payload = match read {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "texture dictionary failed to open");
                return false;
            }
        };
        let textures = matches!(
            payload,
            DocPayload::Resource(or_io::Asset::TextureDict(_)) | DocPayload::Gta6Drawable(_)
        );
        if !textures || (self.documents.iter()).any(|d| d.path.as_deref() == Some(path.as_path())) {
            return false;
        }
        let id = self.alloc_doc_id();
        // ⚠ Not marked as opened by the app: a dictionary so marked serves only the model it opened for.
        let mut doc = Document::new(id, Some(path.clone()), payload);
        doc.read_only = Some(READ_ONLY_BODY);
        self.documents.push(doc);
        tracing::info!(path = %path.display(), "opened the dictionary that holds a key");
        true
    }
}

#[cfg(test)]
#[path = "a_drawable_s_key_opens_the_scanned_dictionary_that_holds_it.rs"]
mod tests;
