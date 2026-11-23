//! The GTA VI texture packs under the install or folder a drawable is in, indexed by the keys
//! their directories hold, so a key the drawable binds opens the pack that holds it, as a GTA V
//! model opens its own dictionary. The index holds keys and the packs they are in, never pixels.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use or_io::container::rpf::{seal_rpf9_scan, unseal_rpf9_scan};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::TexturePack;

use super::gta6_texture_naming::{NamingEnded, TextureNaming};
use super::rpf9_kept::word;
use super::{AppState, DocPayload, Document, DocumentId, DocumentKind};

/// The most folders a walk reads and the most packs it indexes.
const MOST_FOLDERS: usize = 100_000;
const MOST_PACKS: usize = 10_000;
/// The version of what this file keeps of an index: a change to it takes another.
const VERSION: u64 = 2;
/// The most a kept index inflates to, some thirty million keys.
const MOST_BYTES: usize = 1 << 28;

/// A pack's size and modification time when it was read, then a key for each of its textures, 0
/// for one that is not on disk or has no key, with the format code its directory states.
type KeptPack = ((u64, u64), Vec<(u64, u16)>);

/// Where the index of one folder is kept in the configuration folder, a file a folder, each pack
/// by its path under the folder: a pack is read again only once its size or its modification time
/// is not what is kept.
// ponytail: the index of a folder since moved or deleted stays there, as an archive's kept scan does.
struct Kept {
    file: PathBuf,
    /// What a kept index starts with: this file's version, then the folder's path.
    of: Vec<u8>,
}

impl Kept {
    /// Where the index of the packs under `root` is kept; `None` with no configuration folder.
    fn of(root: &Path) -> Option<Self> {
        let path = root.to_string_lossy();
        let mut of = VERSION.to_le_bytes().to_vec();
        put(&mut of, path.as_bytes());
        let name = name_hash64(path.as_bytes());
        let folder = super::prefs::config_root()?.join("gta6_texture_indexes");
        let file = folder.join(format!("{name:016x}.index"));
        Some(Self { file, of })
    }

    /// Each pack of the index kept of `root` by its path; none where no index is kept, or it does not read.
    fn packs(&self, root: &Path) -> HashMap<PathBuf, KeptPack> {
        let Ok(sealed) = std::fs::read(&self.file) else {
            return HashMap::new();
        };
        self.read(root, &sealed)
            .inspect_err(|e| tracing::info!("{}: {e}", self.file.display()))
            .unwrap_or_default()
    }

    fn read(&self, root: &Path, sealed: &[u8]) -> io::Result<HashMap<PathBuf, KeptPack>> {
        let plain = unseal_rpf9_scan(sealed, MOST_BYTES)?;
        let other = "a kept index of another version or folder";
        let bytes = &mut (plain.strip_prefix(self.of.as_slice()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, other))?;
        let mut packs = HashMap::new();
        while !bytes.is_empty() {
            let path = root.join(&*String::from_utf8_lossy(stated(bytes)?));
            let stat = (word(bytes)?, word(bytes)?);
            let (keys, _) = stated(bytes)?.as_chunks();
            let (codes, _) = stated(bytes)?.as_chunks();
            if keys.len() != codes.len() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, other));
            }
            let keys = (keys.iter().zip(codes))
                .map(|(key, code)| (u64::from_le_bytes(*key), u16::from_le_bytes(*code)))
                .collect();
            packs.insert(path, (stat, keys));
        }
        Ok(packs)
    }

    /// Keep `plain`, what [`Self::read`] reads, in place of any index kept before.
    fn keep(&self, plain: &[u8]) -> io::Result<()> {
        if let Some(folder) = self.file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&self.file, seal_rpf9_scan(plain)?)
    }
}

/// `bytes` after their length, as a kept index states a path, a pack's keys and their format codes.
pub(super) fn put(plain: &mut Vec<u8>, bytes: &[u8]) {
    plain.extend((bytes.len() as u64).to_le_bytes());
    plain.extend(bytes);
}

/// The bytes the next length in `bytes` states, which moves past them.
pub(super) fn stated<'a>(bytes: &mut &'a [u8]) -> io::Result<&'a [u8]> {
    let len = usize::try_from(word(bytes)?).ok();
    let (stated, rest) =
        (len.and_then(|len| bytes.split_at_checked(len))).ok_or(io::ErrorKind::UnexpectedEof)?;
    *bytes = rest;
    Ok(stated)
}

/// The size of the file at `path` and its modification time, in nanoseconds.
pub(super) fn stat(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos() as u64))
}

/// The packs under one folder, the pack each key is in, and the format code its directory states.
#[derive(Default)]
struct Index {
    packs: Vec<PathBuf>,
    keys: HashMap<u64, u32>,
    formats: HashMap<u64, u16>,
    /// Whether every pack under the folder was read with every key on disk, so a key it lacks is in none.
    whole: bool,
}

#[derive(Default)]
pub(super) struct PackIndex {
    root: Option<PathBuf>,
    built: Arc<Mutex<Option<Index>>>,
    /// The folder each drawable asked for is served by, found once, and the packs opened or tried, each once.
    asked: HashMap<DocumentId, Option<PathBuf>>,
    tried: HashSet<PathBuf>,
    /// The dictionaries the open archives' scans found, by their textures' keys.
    pub(super) dictionaries: super::gta6_dictionary_index::DictionaryIndex,
}

impl PackIndex {
    /// Walk the packs again at the next ask, and try each again.
    fn forget(&mut self) {
        self.root = None;
        self.built = Arc::default();
        self.tried.clear();
    }
}

/// The folder whose packs serve `doc`: the install or folder open around its file, else the folder its file, or its archive, is in.
fn root_of(state: &AppState, doc: &Document) -> Option<PathBuf> {
    let file = doc.path.as_deref()?;
    let open = (state.documents.iter())
        .filter(|d| matches!(d.kind, DocumentKind::GameRoot | DocumentKind::Folder))
        .filter_map(|d| d.path.as_deref())
        .filter(|root| file.starts_with(root))
        .min_by_key(|root| root.components().count());
    let folder = || file.ancestors().skip(1).find(|a| a.is_dir());
    open.or_else(folder).map(Path::to_path_buf)
}

/// Every pack under `root`, then each key each holds, read from the pack where `kept` does not
/// hold it as it is now, and how many packs were read; `None` once `cancel` is set.
fn index(
    root: &Path,
    kept: Option<&Kept>,
    cancel: &or_job::Cancel,
    progress: &or_job::Progress,
) -> Option<(Index, usize)> {
    let (mut folders, mut packs) = (vec![root.to_path_buf()], Vec::new());
    let (mut read, mut whole) = (0, true);
    while let Some(folder) = folders.pop() {
        if cancel.is_set() {
            return None;
        }
        read += 1;
        let Ok(entries) = std::fs::read_dir(&folder) else {
            whole = false;
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                match read + folders.len() < MOST_FOLDERS {
                    true => folders.push(path),
                    false => whole = false,
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ttpf"))
            {
                match packs.len() < MOST_PACKS {
                    true => packs.push(path),
                    false => whole = false,
                }
            }
        }
    }
    packs.sort();
    let mut was = kept.map(|kept| kept.packs(root)).unwrap_or_default();
    let mut now = kept.map(|kept| kept.of.clone()).unwrap_or_default();
    let (was_packs, mut opened) = (was.len(), 0);
    let (mut index, total) = (Index::default(), packs.len() as u64);
    for (done, path) in packs.into_iter().enumerate() {
        if cancel.is_set() {
            return None;
        }
        progress.reached(done as u64, total);
        // ⚠ Taken before the pack is read: one written in between is then read again at the next index.
        let stat = stat(&path);
        let unchanged = was.remove(&path).filter(|kept| Some(kept.0) == stat);
        let keys = match unchanged {
            Some((_, keys)) => keys,
            None => {
                let Ok(pack) = TexturePack::open(&path) else {
                    whole = false;
                    continue;
                };
                opened += 1;
                let entries = pack.entries.iter();
                entries
                    .map(|e| (if e.on_disk { e.key } else { 0 }, e.format_code))
                    .collect()
            }
        };
        let at = index.packs.len() as u32;
        for &(key, format) in &keys {
            if key != 0 {
                index.keys.entry(key).or_insert(at);
                (index.formats.entry(key)).or_insert(format);
            } else {
                whole = false;
            }
        }
        if let Some((len, modified)) = stat {
            let under = path.strip_prefix(root).unwrap_or(&path);
            put(&mut now, under.to_string_lossy().as_bytes());
            now.extend([len, modified].into_iter().flat_map(u64::to_le_bytes));
            let words: Vec<u8> = keys.iter().flat_map(|(key, _)| key.to_le_bytes()).collect();
            put(&mut now, &words);
            let codes: Vec<u8> = keys
                .iter()
                .flat_map(|(_, code)| code.to_le_bytes())
                .collect();
            put(&mut now, &codes);
        }
        index.packs.push(path);
    }
    index.whole = whole;
    // With no pack read, the index is the kept one unless a pack of it is gone.
    let differs = opened > 0 || index.packs.len() != was_packs;
    // ponytail: an index past MOST_BYTES is written and never read back; its packs are then read at each index.
    if let Some(Err(error)) = kept.filter(|_| differs).map(|kept| kept.keep(&now)) {
        tracing::warn!(%error, "the index of the texture packs was not kept");
    }
    Some((index, opened))
}

/// The key a texture name a GTA VI drawable states stands for: `hash_` and the key, or a name's hash.
pub(super) fn key_of(name: &str) -> Option<u64> {
    match name.strip_prefix("hash_") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => Some(name_hash64(name.as_bytes())),
    }
}

/// A worker's work that gives every key the packs under `root` hold, read as the pack index reads them and kept as it keeps them; `None` once cancelled.
// ⚠ Where the index is kept is found here, on the caller's thread: a test's configuration folder is its own thread's.
pub(super) fn pack_keys(
    root: &Path,
) -> impl FnOnce(&or_job::Cancel, &or_job::Progress) -> Option<HashSet<u64>> + Send + 'static {
    let (root, kept) = (root.to_path_buf(), Kept::of(root));
    move |cancel, progress| {
        let (index, _) = index(&root, kept.as_ref(), cancel, progress)?;
        Some(index.keys.into_keys().collect())
    }
}

impl AppState {
    /// Open the pack that holds a key the active GTA VI drawable binds and nothing open holds,
    /// indexing the packs of its install or folder first, as a job; whether one opened.
    pub fn try_open_texture_pack(&mut self) -> bool {
        // A drawable that asked and closed reopens under another id, so its folder is walked again.
        let (documents, asked) = (&self.documents, self.texture_packs.asked.len());
        (self.texture_packs.asked).retain(|id, _| documents.iter().any(|d| d.id == *id));
        if self.texture_packs.asked.len() < asked {
            self.texture_packs.forget();
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
        let root = match self.texture_packs.asked.get(&doc_id) {
            Some(root) => root.clone(),
            None => root_of(self, doc),
        };
        self.texture_packs.asked.insert(doc_id, root.clone());
        // ponytail: one index at a time, walked again when a drawable under another folder asks.
        if root.is_some() && root != self.texture_packs.root {
            self.index_texture_packs(root);
            return false;
        }
        let wanted: Vec<PathBuf> = {
            let built = self.texture_packs.built.lock();
            let built = built.unwrap_or_else(|p| p.into_inner());
            let Some(index) = built.as_ref() else {
                return false;
            };
            (self.texture_report.missing().into_iter())
                .filter_map(key_of)
                .filter_map(|key| index.keys.get(&key))
                .filter_map(|&at| index.packs.get(at as usize).cloned())
                .collect()
        };
        for path in wanted {
            if self.texture_packs.tried.insert(path.clone()) && self.open_texture_pack(&path) {
                return true;
            }
        }
        false
    }

    /// The format code the directory of the indexed pack that holds `key` states, open or not.
    pub(crate) fn indexed_texture_format(&self, key: u64) -> Option<u16> {
        let built = self.texture_packs.built.lock();
        (built
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()?
            .formats
            .get(&key))
        .copied()
    }

    /// The folder that serves `doc`, once every pack under it is indexed whole and none holds
    /// texture `name`, which no open document resolves either: there is no texture to load for it.
    pub(crate) fn held_by_no_pack(&self, doc: DocumentId, name: &str) -> Option<PathBuf> {
        let packs = &self.texture_packs;
        // ⚠ Another drawable's folder may be the one indexed now: its packs say nothing of this one's keys.
        let root = (packs.asked.get(&doc)?.as_ref()).filter(|r| packs.root.as_ref() == Some(r))?;
        let key = key_of(name)?;
        if packs.dictionaries.holds(key) {
            return None;
        }
        {
            let built = packs.built.lock().unwrap_or_else(|p| p.into_inner());
            let index = built.as_ref()?;
            if !index.whole || index.keys.contains_key(&key) {
                return None;
            }
        }
        let found = super::texture_resolution::probe(self, None, None, name).found;
        found.is_none().then(|| root.clone())
    }

    /// 0 until the packs are indexed, then a count that moves with each index: what was built from [`Self::held_by_no_pack`] is built again when it moves.
    pub(crate) fn texture_packs_indexed(&self) -> u64 {
        let built = self.texture_packs.built.lock();
        let built = built.unwrap_or_else(|p| p.into_inner());
        built
            .as_ref()
            .map_or(0, |index| 1 + index.packs.len() as u64)
    }

    /// Forget the index once `path`, which the app wrote, moved or removed, is under its folder.
    pub(super) fn texture_packs_changed(&mut self, path: &Path) {
        if (self.texture_packs.root.as_deref()).is_some_and(|root| path.starts_with(root)) {
            self.texture_packs.forget();
        }
    }

    /// Index the packs under `root` on a worker, in place of any index before it.
    fn index_texture_packs(&mut self, root: Option<PathBuf>) {
        let Some(at) = root.clone() else {
            return;
        };
        let built = Arc::new(Mutex::new(None));
        self.texture_packs = PackIndex {
            root,
            built: Arc::clone(&built),
            asked: std::mem::take(&mut self.texture_packs.asked),
            tried: HashSet::new(),
            dictionaries: std::mem::take(&mut self.texture_packs.dictionaries),
        };
        let doing = tr!(
            "Indexing the GTA VI texture packs under {folder}",
            folder = at.display()
        );
        // ⚠ Found here and not on the worker: a test's configuration folder is its own thread's.
        let kept = Kept::of(&at);
        let run = TextureNaming::run(doing, move |cancel, progress| {
            let Some((index, opened)) = index(&at, kept.as_ref(), cancel, progress) else {
                return NamingEnded::Stopped;
            };
            let packs = index.packs.len();
            *built.lock().unwrap_or_else(|p| p.into_inner()) = Some(index);
            NamingEnded::Indexed(packs, opened)
        });
        self.texture_naming.push(run);
    }

    /// Open the pack at `path` beside the drawable that asked for it; `false` when it is open or does not read.
    fn open_texture_pack(&mut self, path: &Path) -> bool {
        if (self.documents.iter()).any(|d| d.path.as_deref() == Some(path)) {
            return false;
        }
        let ttpf = match super::ttpf_document::TtpfDocument::open(path) {
            Ok(ttpf) => ttpf,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "texture pack failed to open");
                return false;
            }
        };
        let id = self.alloc_doc_id();
        let payload = DocPayload::Ttpf(Arc::new(ttpf));
        (self.documents).push(Document::new(id, Some(path.to_path_buf()), payload));
        self.auto_opened_texture_docs.insert(id);
        tracing::info!(path = %path.display(), "opened the texture pack that holds a drawable's key");
        true
    }
}

#[cfg(test)]
mod tests {
    use or_io::gta6::headerless::{Drawable, Geometry, Model, Palette, Resource, Shader, Vertices};

    use super::*;
    use crate::state::gta6_resource::texture_name;

    #[test]
    fn a_key_the_pack_index_states_as_srgb_is_the_base_colour_before_its_pack_opens() {
        let (normal, colour) = (0x0123_4567_89AB_CDEFu64, 0x0000_0000_000C_0105u64);
        let geometry = Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 0,
            vertex_count: 3,
            channels: Vec::new(),
            triangles: vec![[0, 1, 2]],
            positions: Ok(vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]),
            vertices: Vertices::default(),
        };
        let drawable = Drawable {
            name: "d".to_string(),
            box_min: [0.0; 3],
            box_max: [1.0, 0.0, 1.0],
            shaders: vec![Shader {
                name_hash: 1,
                texture_keys: vec![Some(normal), Some(colour)],
                parameters: Vec::new(),
            }],
            levels: [
                Some(vec![Model {
                    geometries: vec![Ok(geometry)],
                    palette: Ok(Palette::Own),
                    matrix_index: 0,
                }]),
                None,
                None,
                None,
            ],
            lod_distances: [10.0; 4],
            bound: None,
            embedded: None,
            extra_skeletons: Vec::new(),
        };
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        let payload = DocPayload::Gta6Drawable(Arc::new(Resource::Drawable(drawable)));
        (state.documents).push(Document::new(1, Some("0x10.drawable".into()), payload));
        state.active_document = Some(1);
        let base = |state: &AppState| {
            let (_, parts) =
                crate::render::gta6_drawable::drawn_geometries(state, 1).expect("drawn");
            parts[0].2.texture_name.clone()
        };
        assert_eq!(
            base(&state),
            Some(texture_name(normal)),
            "no pack states a format"
        );

        // The colour key's pack states an sRGB format and the first key's a linear one; neither is open.
        let index = Index {
            packs: vec![PathBuf::from("a.ttpf")],
            keys: HashMap::from([(normal, 0), (colour, 0)]),
            formats: HashMap::from([(normal, 71 << 3), (colour, 72 << 3)]),
            whole: true,
        };
        state.texture_packs.built = Arc::new(Mutex::new(Some(index)));
        assert_eq!(state.indexed_texture_format(colour), Some(72 << 3));
        assert_eq!(state.indexed_texture_format(colour ^ 1), None);
        assert_eq!(base(&state), Some(texture_name(colour)));
    }
}
