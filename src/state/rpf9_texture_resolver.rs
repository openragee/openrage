//! External texture dictionaries opened from RPF9 table addresses on a bounded, cancellable worker.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use or_io::gta6::hash::name_hash64;
use or_job::{Cancel, Progress};

use super::document::rpf9::entry::{EntrySource, READ_ONLY};
use super::jobs::{JobKind, Outcome, RunningJob, Then};
use super::rpf9_texture_index::{Index, Tables};
use super::rpf9_texture_roots::Scope;
use super::{AppState, DocPayload, Document, DocumentId};

const BODY_LIMIT: u64 = 256 << 20;

#[derive(Default)]
pub(super) struct State {
    scope: Vec<PathBuf>,
    table_versions: Vec<usize>,
    index: Option<Index>,
    asked: HashSet<u64>,
    tried: HashSet<PathBuf>,
    pending: Option<Pending>,
    checked: Option<Instant>,
    changed: bool,
    owned: Vec<Owned>,
}

struct Owned {
    id: DocumentId,
    epoch: u64,
    archive: PathBuf,
    stamp: Option<(u64, u64)>,
}

struct Pending {
    model: DocumentId,
    epoch: u64,
    wanted: HashSet<u64>,
    cancel: Cancel,
    progress: Progress,
    result: mpsc::Receiver<Finished>,
}

struct Finished {
    index: Option<Index>,
    tried: HashSet<PathBuf>,
    opened: Option<(EntrySource, DocPayload)>,
    error: Option<String>,
}

impl State {
    pub fn job(&self) -> Option<RunningJob> {
        let pending = self.pending.as_ref()?;
        Some(RunningJob {
            kind: JobKind::TextureNames,
            doing: tr!("Texture dictionary").to_string(),
            fraction: Some(pending.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: pending.cancel.is_set(),
        })
    }

    pub fn stop(&self) {
        if let Some(pending) = &self.pending {
            pending.cancel.set();
        }
    }

    fn reset(&mut self, scope: Vec<PathBuf>, table_versions: Vec<usize>) {
        self.stop();
        let mut empty = Self::default();
        empty.scope = scope;
        empty.table_versions = table_versions;
        *self = empty;
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) fn read_next(
    index: &mut Index,
    wanted: &HashSet<u64>,
    tried: &mut HashSet<PathBuf>,
    cancel: &Cancel,
    progress: &Progress,
) -> (Option<(EntrySource, DocPayload)>, Option<String>) {
    let mut error = None;
    let mut keys: Vec<_> = wanted.iter().copied().collect();
    keys.sort_unstable();
    loop {
        if !index.extend_until_any(wanted, cancel, progress) || cancel.is_set() {
            return (None, error);
        }
        let source = keys
            .iter()
            .find_map(|key| index.candidates(*key).next())
            .cloned();
        let Some(source) = source else {
            return (None, error);
        };
        if !tried.insert(source.path.clone()) {
            index.reject(&source.path);
            continue;
        }
        match source.payload_limited(BODY_LIMIT) {
            Ok(payload @ DocPayload::Resource(or_io::Asset::TextureDict(_))) => {
                return (Some((source, payload)), error);
            }
            other => {
                let why = other
                    .err()
                    .unwrap_or_else(|| tr!("That file is not a texture dictionary").to_string());
                tracing::warn!(path = %source.path.display(), error = %why, "table texture dictionary failed to open");
                if error.is_none() {
                    error = Some(why);
                }
                index.reject(&source.path);
            }
        }
    }
}

fn resolve(
    scope: Scope,
    held: Option<Index>,
    wanted: HashSet<u64>,
    mut tried: HashSet<PathBuf>,
    cancel: &Cancel,
    progress: &Progress,
) -> Finished {
    let mut index = match held {
        Some(index) => index,
        None => {
            let roots = scope.roots(cancel);
            let Some(tables) = Tables::open(roots, cancel, progress) else {
                return Finished {
                    index: None,
                    tried,
                    opened: None,
                    error: None,
                };
            };
            Index::new(tables)
        }
    };
    let (opened, error) = read_next(&mut index, &wanted, &mut tried, cancel, progress);
    Finished {
        index: Some(index),
        tried,
        opened,
        error,
    }
}

impl AppState {
    fn reset_table_texture_cache(&mut self, scope: Vec<PathBuf>, table_versions: Vec<usize>) {
        let owned = std::mem::take(&mut self.rpf9_textures.owned);
        let mut changed = false;
        for cached in owned {
            let automatic = self.auto_opened_texture_docs.contains(&cached.id);
            let untouched = self.document(cached.id).is_some_and(|doc| {
                !doc.dirty
                    && doc.read_only == Some(READ_ONLY)
                    && doc.payload_epoch() == cached.epoch
                    && doc.rpf9_source.is_some()
            });
            if automatic && untouched {
                self.drop_document(cached.id);
                changed = true;
            }
        }
        self.rpf9_textures.reset(scope, table_versions);
        self.rpf9_textures.changed = changed;
    }

    /// Resolve the active table model's missing keys through table dictionaries, without changing its active tab.
    pub fn try_open_table_texture_dictionary(&mut self) -> bool {
        self.poll_table_texture_dictionary();
        if std::mem::take(&mut self.rpf9_textures.changed) {
            return true;
        }
        let Some(doc) = self.active_document() else {
            return false;
        };
        if !doc.is_from_rpf9_table() || !matches!(doc.payload, DocPayload::Gta6Drawable(_)) {
            return false;
        }
        let (model, epoch) = (doc.id, doc.payload_epoch());
        if self.texture_report.document != Some(model) {
            return false;
        }
        let scope = self.rpf9_texture_scope(Some(model));
        let key = scope.key();
        let versions = scope.table_versions();
        if self.rpf9_textures.scope != key || self.rpf9_textures.table_versions != versions {
            self.reset_table_texture_cache(key, versions);
            if std::mem::take(&mut self.rpf9_textures.changed) {
                return true;
            }
        }
        let mut wanted: HashSet<_> = self
            .texture_report
            .missing()
            .into_iter()
            .filter_map(super::gta6_texture_index::key_of)
            .collect();
        if let Some(Document {
            payload: DocPayload::Gta6Drawable(body),
            ..
        }) = self.document(model)
        {
            // The viewport report may contain only base colour; normal and material slots still need their dictionaries.
            let bound = super::gta6_resource::dictionary_drawables(body)
                .map(|(_, drawable)| drawable)
                .chain(super::gta6_resource::drawable_of(body, None))
                .flat_map(|drawable| &drawable.shaders)
                .flat_map(|shader| shader.texture_keys.iter().flatten().copied());
            wanted.extend(bound.filter(|key| {
                let name = format!("hash_{key:016X}");
                super::texture_resolution::probe(self, None, None, &name)
                    .found
                    .is_none()
            }));
        }
        let held = &mut self.rpf9_textures;
        if held.pending.is_some() {
            return false;
        }
        let wanted: HashSet<_> = wanted.difference(&held.asked).copied().collect();
        if wanted.is_empty() {
            return false;
        }
        let index = held.index.take();
        let tried = std::mem::take(&mut held.tried);
        let cancel = Cancel::new();
        let progress = Progress::new();
        let (sender, result) = mpsc::channel();
        held.pending = Some(Pending {
            model,
            epoch,
            wanted: wanted.clone(),
            cancel: cancel.clone(),
            progress: progress.clone(),
            result,
        });
        std::thread::spawn(move || {
            let result = resolve(scope, index, wanted, tried, &cancel, &progress);
            let _ = sender.send(result);
        });
        false
    }

    /// Accept a table resolver's result only while the requesting model and archive snapshots still match.
    pub(super) fn poll_table_texture_dictionary(&mut self) {
        let check = self
            .rpf9_textures
            .checked
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        if check {
            let held = &self.rpf9_textures;
            let outdated = held.index.as_ref().is_some_and(|index| !index.is_current())
                || held.owned.iter().any(|owned| {
                    self.document(owned.id).is_none()
                        || super::gta6_texture_index::stat(&owned.archive) != owned.stamp
                });
            if outdated {
                self.reset_table_texture_cache(
                    self.rpf9_textures.scope.clone(),
                    self.rpf9_textures.table_versions.clone(),
                );
            }
            self.rpf9_textures.checked = Some(Instant::now());
        }
        let Some(pending) = &self.rpf9_textures.pending else {
            return;
        };
        if !self
            .document(pending.model)
            .is_some_and(|doc| doc.is_from_rpf9_table() && doc.payload_epoch() == pending.epoch)
        {
            self.reset_table_texture_cache(Vec::new(), Vec::new());
            return;
        }
        let mut result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Finished {
                index: None,
                tried: HashSet::new(),
                opened: None,
                error: Some(tr!("The worker making it stopped").to_string()),
            },
        };
        let Some(pending) = self.rpf9_textures.pending.take() else {
            return;
        };
        let same_model = self
            .document(pending.model)
            .is_some_and(|doc| doc.is_from_rpf9_table() && doc.payload_epoch() == pending.epoch);
        let current = self.rpf9_texture_scope(Some(pending.model));
        if !same_model
            || current.key() != self.rpf9_textures.scope
            || current.table_versions() != self.rpf9_textures.table_versions
        {
            self.reset_table_texture_cache(Vec::new(), Vec::new());
            return;
        }
        if result
            .index
            .as_ref()
            .is_some_and(|index| !index.is_current())
        {
            self.reset_table_texture_cache(Vec::new(), Vec::new());
            return;
        }
        let stopped = pending.cancel.is_set();
        let (count, limited) = result
            .index
            .as_ref()
            .map(|index| (index.dictionaries(), index.tables.truncated))
            .unwrap_or_default();
        if let Some(index) = &result.index {
            for (path, error) in &index.tables.failures {
                tracing::warn!(path = %path.display(), %error, "RPF9 table texture index could not read an entry");
            }
            if result.error.is_none() && result.opened.is_none() {
                result.error = index
                    .tables
                    .failures
                    .first()
                    .map(|(_, error)| error.clone());
            }
        }
        let found = !stopped && result.opened.is_some();
        if !stopped {
            if let Some((source, payload)) = result.opened.take() {
                let keys = match &payload {
                    DocPayload::Resource(or_io::Asset::TextureDict(dictionary)) => dictionary
                        .entries
                        .iter()
                        .flatten()
                        .map(|texture| name_hash64(texture.name.as_bytes()))
                        .collect::<HashSet<_>>(),
                    _ => HashSet::new(),
                };
                self.rpf9_textures
                    .asked
                    .extend(pending.wanted.intersection(&keys).copied());
                if !self
                    .documents
                    .iter()
                    .any(|doc| doc.path.as_deref() == Some(source.path.as_path()))
                {
                    let id = self.alloc_doc_id();
                    let mut doc = Document::new(id, Some(source.path.clone()), payload);
                    doc.read_only = Some(READ_ONLY);
                    doc.rpf9_source = Some(source);
                    if let Some(archive) = doc
                        .rpf9_source
                        .as_ref()
                        .and_then(|source| super::document::rpf9::entry::archive_path(&source.path))
                    {
                        self.rpf9_textures.owned.push(Owned {
                            id,
                            epoch: doc.payload_epoch(),
                            archive: archive.to_path_buf(),
                            stamp: super::gta6_texture_index::stat(archive),
                        });
                    }
                    self.documents.push(doc);
                    self.auto_opened_texture_docs.insert(id);
                    self.rpf9_textures.changed = true;
                }
            }
        }
        if !found {
            self.rpf9_textures.asked.extend(pending.wanted);
        }
        self.rpf9_textures.index = result.index;
        self.rpf9_textures.tried = result.tried;
        let (outcome, said) = if stopped || limited {
            (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished").to_string(),
            )
        } else if !found && result.error.is_some() {
            (Outcome::Failed, result.error.unwrap_or_default())
        } else {
            (
                Outcome::Done(Then::Nothing),
                tr_n!(
                    "{count} GTA VI texture dictionary indexed by its textures' keys",
                    "{count} GTA VI texture dictionaries indexed by their textures' keys",
                    count
                ),
            )
        };
        self.jobs.record(JobKind::TextureNames, outcome, said);
    }
}

#[cfg(test)]
mod tests {
    use super::super::document::rpf9::Rpf9TableDocument;
    use super::super::rpf9_texture_index::{tests::fixture, Root};
    use super::*;
    use std::sync::Arc;

    fn automatic(state: &mut AppState, table: &Rpf9TableDocument) -> DocumentId {
        let source = table.entry(0, 0).unwrap();
        let id = state.alloc_doc_id();
        let mut doc = Document::new(id, Some(source.path.clone()), source.payload().unwrap());
        doc.rpf9_source = Some(source);
        state.rpf9_textures.owned.push(Owned {
            id,
            epoch: doc.payload_epoch(),
            archive: table.path.clone(),
            stamp: super::super::gta6_texture_index::stat(&table.path),
        });
        state.documents.push(doc);
        state.auto_opened_texture_docs.insert(id);
        id
    }

    #[test]
    fn changing_scope_closes_only_dictionaries_still_owned_by_the_resolver() {
        let (_dir, table) = fixture("table_external", false, false);
        let mut state = AppState::from_prefs(super::super::Prefs::default());
        let automatic = automatic(&mut state, &table);
        let adopted = self::automatic(&mut state, &table);
        state.auto_opened_texture_docs.remove(&adopted);
        let source = table.entry(0, 0).unwrap();
        let explicit = state.alloc_doc_id();
        state.documents.push(Document::new(
            explicit,
            Some(source.path.clone()),
            source.payload().unwrap(),
        ));
        state.reset_table_texture_cache(vec![PathBuf::from("another-install")], Vec::new());
        assert!(state.document(automatic).is_none());
        assert!(state.document(adopted).is_some());
        assert!(state.document(explicit).is_some());
        assert!(state.rpf9_textures.changed);
    }

    #[test]
    fn closing_a_cached_dictionary_allows_its_texture_keys_to_be_requested_again() {
        let (_dir, table) = fixture("table_external", false, false);
        let mut state = AppState::from_prefs(super::super::Prefs::default());
        let id = automatic(&mut state, &table);
        let source = table.entry(0, 0).unwrap();
        let key = name_hash64(b"table_external");
        state.rpf9_textures.asked.insert(key);
        state.rpf9_textures.tried.insert(source.path.clone());
        state.drop_document(id);
        state.poll_table_texture_dictionary();
        assert!(state.document(id).is_none());
        assert!(!state.rpf9_textures.asked.contains(&key));
        assert!(!state.rpf9_textures.tried.contains(&source.path));
    }

    #[test]
    fn changing_a_source_discards_cached_pixels_even_without_a_pending_request() {
        let (_dir, table) = fixture("table_external", false, false);
        let mut state = AppState::from_prefs(super::super::Prefs::default());
        let id = automatic(&mut state, &table);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&table.path)
            .unwrap()
            .set_len(0)
            .unwrap();
        state.poll_table_texture_dictionary();
        assert!(state.document(id).is_none());
        assert!(state.rpf9_textures.changed);
        assert!(state.rpf9_textures.pending.is_none());
    }

    #[test]
    fn a_worker_result_is_discarded_when_its_archive_changed_before_delivery() {
        let (_dir, table) = fixture("table_external", false, false);
        let catalog = Tables::open(
            vec![Root::Mounted(Arc::clone(&table))],
            &Cancel::new(),
            &Progress::new(),
        )
        .unwrap();
        let source = table.entry(0, 0).unwrap();
        let payload = source.payload().unwrap();
        let mut state = AppState::from_prefs(super::super::Prefs::default());
        let model = state.alloc_doc_id();
        let drawable = super::super::opening_a_ttpf::drawable_keyed(name_hash64(b"table_external"));
        state.documents.push(Document::new(
            model,
            Some(table.path.join(".rpf9-table/99/model.ydr")),
            DocPayload::Gta6Drawable(Arc::new(or_io::gta6::headerless::Resource::Drawable(
                drawable,
            ))),
        ));
        let scope = state.rpf9_texture_scope(Some(model)).key();
        state.rpf9_textures.scope = scope;
        let (sender, receiver) = mpsc::channel();
        state.rpf9_textures.pending = Some(Pending {
            model,
            epoch: 0,
            wanted: HashSet::from([name_hash64(b"table_external")]),
            cancel: Cancel::new(),
            progress: Progress::new(),
            result: receiver,
        });
        sender
            .send(Finished {
                index: Some(Index::new(catalog)),
                tried: HashSet::new(),
                opened: Some((source, payload)),
                error: None,
            })
            .unwrap_or_else(|_| panic!("the receiver exists"));
        std::fs::OpenOptions::new()
            .write(true)
            .open(&table.path)
            .unwrap()
            .set_len(0)
            .unwrap();
        state.poll_table_texture_dictionary();
        assert_eq!(state.documents.len(), 1);
        assert!(state.document(model).is_some());
        assert!(state.rpf9_textures.pending.is_none());
        assert!(state.rpf9_textures.index.is_none());
    }
    #[test]
    fn the_table_worker_resolves_external_textures_for_ydr_ydd_and_yft() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        use or_io::codec::texture::decode_level;
        use or_io::gta6::hash::{name_hash32, name_hash64};
        use or_io::gta6::headerless::{Dictionary, DrawableWithBound, Entry, Fragment, Resource};

        use crate::state::document::rpf9::entry::READ_ONLY;
        use crate::state::{
            texture_resolution, AppState, DocPayload, Document, DocumentKind, Prefs,
        };

        let _serial = crate::ui::theme::serial();
        for (extension, kind) in [
            ("ydr", DocumentKind::Ydr),
            ("ydd", DocumentKind::Ydd),
            ("yft", DocumentKind::Yft),
        ] {
            let name = format!("table_family_{extension}");
            let key = name_hash64(name.as_bytes());
            let (dir, table) =
                crate::state::rpf9_texture_index::tests::fixture(&name, false, false);
            let drawable = crate::state::opening_a_ttpf::drawable_keyed(key);
            let resource = match kind {
                DocumentKind::Ydr => Resource::Drawable(drawable),
                DocumentKind::Ydd => Resource::Dictionary(Dictionary {
                    entries: vec![(
                        name_hash32(drawable.name.as_bytes()),
                        Ok(Entry::Drawable(Box::new(drawable))),
                    )],
                }),
                DocumentKind::Yft => Resource::DrawableWithBound(DrawableWithBound {
                    name: "table_fragment".to_string(),
                    drawable,
                    bound: None,
                    fragment: Box::new(Fragment::default()),
                }),
                _ => unreachable!(),
            };
            let mut state = AppState::from_prefs(Prefs::default());
            let archive_id = state.alloc_doc_id();
            state.documents.push(Document::new(
                archive_id,
                Some(table.path.clone()),
                DocPayload::Rpf9Table(Arc::clone(&table)),
            ));
            let model_id = state.alloc_doc_id();
            let model_path = table
                .path
                .join(".rpf9-table")
                .join("model")
                .join(format!("model.{extension}"));
            let model = Document::new(
                model_id,
                Some(model_path.clone()),
                DocPayload::Gta6Drawable(Arc::new(resource)),
            );
            assert_eq!(model.kind, DocumentKind::Gta6Drawable);
            assert_eq!(model.read_only, Some(READ_ONLY));
            state.documents.push(model);
            state.active_document = Some(model_id);
            let requested = format!("hash_{key:016X}");
            state.texture_report = texture_resolution::report(&state, None, [], 0);
            assert!(state.texture_report.missing().is_empty());
            let deadline = Instant::now() + Duration::from_secs(10);
            while !state.try_open_table_texture_dictionary() {
                assert!(
                    Instant::now() < deadline,
                    "{extension}: worker did not finish"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(state.active_document, Some(model_id));
            assert_eq!(state.auto_opened_texture_docs.len(), 1);
            let resolved = texture_resolution::resolve(&state, None, None, &requested)
                .unwrap_or_else(|| panic!("{extension}: the external texture must resolve"));
            assert!(resolved.origin.implicitly_opened);
            assert_eq!(resolved.texture.name(), name);
            assert!(resolved.texture.has_pixel_data());
            let image =
                decode_level(&resolved.texture, 0).expect("the resolved texture has pixels");
            assert_eq!((image.width, image.height), (64, 64));
            let dictionary = state.document(resolved.origin.document).unwrap();
            assert_eq!(dictionary.read_only, Some(READ_ONLY));
            assert!(dictionary.rpf9_source.is_some());
            assert!(!dictionary.path.as_ref().unwrap().exists());
            assert!(!model_path.exists());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn closing_the_requesting_model_cancels_before_the_worker_replies() {
        let (_dir, table) = fixture("table_external", false, false);
        let mut state = AppState::from_prefs(super::super::Prefs::default());
        let model = state.alloc_doc_id();
        let drawable = super::super::opening_a_ttpf::drawable_keyed(name_hash64(b"table_external"));
        state.documents.push(Document::new(
            model,
            Some(table.path.join(".rpf9-table/99/model.ydr")),
            DocPayload::Gta6Drawable(Arc::new(or_io::gta6::headerless::Resource::Drawable(
                drawable,
            ))),
        ));
        let cancel = Cancel::new();
        let (_sender, receiver) = mpsc::channel();
        state.rpf9_textures.pending = Some(Pending {
            model,
            epoch: 0,
            wanted: HashSet::from([name_hash64(b"table_external")]),
            cancel: cancel.clone(),
            progress: Progress::new(),
            result: receiver,
        });
        state.drop_document(model);
        state.poll_table_texture_dictionary();
        assert!(
            cancel.is_set(),
            "do not wait for the worker's result to cancel its reads"
        );
        assert!(state.rpf9_textures.pending.is_none());
    }

    #[test]
    fn reopening_a_table_at_the_same_path_retries_the_keys_of_its_previous_snapshot() {
        let (_dir, table) = fixture("table_external", false, false);
        let mut state = AppState::from_prefs(super::super::Prefs::default());
        let archive = state.alloc_doc_id();
        state.documents.push(Document::new(
            archive,
            Some(table.path.clone()),
            DocPayload::Rpf9Table(Arc::clone(&table)),
        ));
        let model = state.alloc_doc_id();
        let key = name_hash64(b"table_external");
        let drawable = super::super::opening_a_ttpf::drawable_keyed(key);
        state.documents.push(Document::new(
            model,
            Some(table.path.join(".rpf9-table/99/model.ydr")),
            DocPayload::Gta6Drawable(Arc::new(or_io::gta6::headerless::Resource::Drawable(
                drawable,
            ))),
        ));
        state.active_document = Some(model);
        let previous = state.rpf9_texture_scope(Some(model));
        state.rpf9_textures.scope = previous.key();
        state.rpf9_textures.table_versions = previous.table_versions();
        state.rpf9_textures.asked.insert(key);
        let replacement = Arc::new(
            Rpf9TableDocument::open_using(&table.path, || {
                or_io::container::rpf::GtaViKeys::parse(&or_test_support::rpf9::keys())
                    .map_err(|error| error.to_string())
            })
            .unwrap()
            .unwrap(),
        );
        state.document_mut(archive).unwrap().payload = DocPayload::Rpf9Table(replacement);
        let now = state.rpf9_texture_scope(Some(model));
        assert_eq!(now.key(), previous.key());
        assert_ne!(now.table_versions(), previous.table_versions());
        let name = format!("hash_{key:016x}");
        state.texture_report =
            super::super::texture_resolution::report(&state, None, [(None, name)], 0);
        assert!(!state.try_open_table_texture_dictionary());
        assert!(!state.rpf9_textures.asked.contains(&key));
        assert!(
            state.rpf9_textures.pending.is_some(),
            "the replacement table must retry the previously missing key"
        );
        state.rpf9_textures.stop();
    }
}
