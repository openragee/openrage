//! What a GTA VI archive's row gives the tree's verbs that its name does not: the hash its table
//! names it by, and the preview tab its body opens in through the read its open asks for.

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::{Rpf9Kind, Rpf9Place};

use super::{body_path, rpf9_table, View};
use crate::state::document::{DocumentId, RpfNodeId};
use crate::state::files_view::name_hash;
use crate::state::rpf_archive_open::tree_open;
use crate::state::rpf_session::RpfEditSession;
use crate::state::{
    AppState, ArchiveSource, DocPayload, DocumentKind, RpfNodeBacking, RpfSessionNode,
};

/// What Copy hash gives for `node`: a GTA VI row's path hash in the `0x` form the filter finds it by, else its name's hash.
pub(crate) fn copied_hash(session: &RpfEditSession, node: &RpfSessionNode) -> String {
    match gta6_hash(session, node) {
        Some(hash) => format!("0x{hash:016x}"),
        None => format!("0x{:08X}", name_hash(&node.name)),
    }
}

/// The path hash a GTA VI archive's table names `node` by; `None` for a row of any other archive.
fn gta6_hash(session: &RpfEditSession, node: &RpfSessionNode) -> Option<u64> {
    let (archive_id, entry_index) = match &node.backing {
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            ..
        } => (*archive_id, *entry_index),
        RpfNodeBacking::Archive {
            source:
                ArchiveSource::Nested {
                    parent_archive_id,
                    parent_entry_index,
                    ..
                },
            ..
        } => (*parent_archive_id, *parent_entry_index),
        _ => return None,
    };
    let archive = &session.archive(archive_id)?.archive;
    Some(archive.rpf9_row(entry_index)?.name64)
}

/// The body a save into a row left there.
type Edited = Option<Arc<Vec<u8>>>;

/// Where the GTA VI file row `node`'s open reads its body: its place, its row's start, its kind,
/// and the body a save into the row left there.
fn gta6_body(
    state: &AppState,
    doc: DocumentId,
    node: RpfNodeId,
) -> Option<(Rpf9Place, u64, Option<Rpf9Kind>, Edited)> {
    let DocPayload::Rpf(rpf) = &state.document(doc)?.payload else {
        return None;
    };
    let RpfNodeBacking::ArchiveFile {
        archive_id,
        entry_index,
        override_bytes,
    } = &rpf.session.node(node)?.backing
    else {
        return None;
    };
    let archive = &rpf.session.archive(*archive_id)?.archive;
    let place = archive.rpf9_place(*entry_index)?;
    let row_start = place.base + archive.binary_entry(*entry_index)?.file_offset;
    let kind = archive.rpf9_kind(*entry_index);
    Some((place, row_start, kind, override_bytes.clone()))
}

impl AppState {
    /// Open a GTA VI file row in the preview tab by the read its open leaves the load worker, where the workspace shows its kind; `false` for any other row.
    pub(crate) fn preview_gta6_row(&mut self, doc: DocumentId, node: RpfNodeId) -> bool {
        let Some((place, row_start, kind, edited)) = gta6_body(self, doc, node) else {
            return false;
        };
        // The row's kind stands for the extension a GTA V row's name gives its preview.
        let shown = (kind.and_then(|kind| kind.extension))
            .and_then(|ext| DocumentKind::from_path(Path::new(&format!("row.{ext}"))));
        if shown.is_some_and(|kind| self.active_workspace.handles_kind(kind)) {
            let path = body_path(&place.file, place.offset, View::Bytes);
            tree_open::leave(&path, &place, row_start, rpf9_table::keys, edited);
            tree_open::name(self, &path, doc, node);
            self.open_scanned_path(path, true);
        }
        true
    }

    /// Keep the GTA VI row `node`'s preview tab past the next single click, as opening the row asks.
    // ponytail: a preview opened while it is still loading stays the preview tab; keep it once it
    // arrives if that shows.
    pub(crate) fn keep_gta6_preview(&mut self, doc: DocumentId, node: RpfNodeId) {
        let Some((place, ..)) = gta6_body(self, doc, node) else {
            return;
        };
        let path = body_path(&place.file, place.offset, View::Bytes);
        if let Some(id) = self.document_holding(&path, None) {
            self.keep_tab(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{
        write_rpf9_resource, Gta6Context, Gta6Mount, GtaViKeys, NameResolver, Rpf9Form,
        Rpf9Sections,
    };
    use or_io::container::{NoOpCryptoProvider, RpfArchive};
    use or_io::gta6::hash::name_hash64;

    use super::*;
    use crate::state::explorer::row_matches;
    use crate::state::navigation::Location;
    use crate::state::rpf_archive_open::tree_verbs::tests::{rbm_section, synthetic_keys};
    use crate::state::rpf_session::RpfDocument;
    use crate::state::Document;
    use crate::ui::commands::{self, Action, Target};
    use crate::ui::dock::Workspace;

    /// The archive key the synthetic snapshot holds.
    const KEY_ID: u32 = 0xDEAD_BEEF;
    const DOC: DocumentId = 9_300;
    const NAMED: &str = "data/named.bin";
    const UNNAMED: &str = "data/unnamed.bin";
    const NESTED: &str = "child.rpf";
    /// A nested archive named as the game names one, with no `.rpf`, which the tree shows with it.
    const BARE: &str = "zq/other";
    const INNER: &str = "inner/a.bin";
    const RESOURCE: &str = "data/model.ydr";

    /// Archives opened while this is held are read with the keys and names it was made with.
    struct Reading(Option<Arc<Gta6Context>>);

    impl Drop for Reading {
        fn drop(&mut self) {
            Gta6Context::swap_given(self.0.take());
        }
    }

    fn reading_with(keys: Arc<GtaViKeys>, names: NameResolver) -> Reading {
        let context = Gta6Context {
            keys,
            names: Arc::new(names),
        };
        Reading(Gta6Context::swap_given(Some(Arc::new(context))))
    }

    /// Names for the archive's named row and its nested archive only.
    fn some_names() -> NameResolver {
        let mut names = NameResolver::new();
        for name in [NAMED, NESTED, BARE] {
            names.insert(name_hash64(name.as_bytes()), name);
        }
        names
    }

    fn built(entries: &[(String, Rpf9NewBody)]) -> Vec<u8> {
        let key = Rpf9IndexKey {
            key_id: KEY_ID,
            iv: [0x33; 16],
        };
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let plan = build(entries, &key, &keys).expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
            .expect("the archive is laid out");
        bytes
    }

    fn chunked(name: &str) -> (String, Rpf9NewBody) {
        let body = format!("the body of {name}, which deflates. ").repeat(40);
        (name.to_owned(), Rpf9NewBody::Chunked(body.into_bytes()))
    }

    /// An archive of a named row, an unnamed one, a drawable's headerless resource and a nested
    /// archive of one row, built from nothing as `dir/source.rpf`.
    fn source(dir: &Path) -> PathBuf {
        let inner = built(&[chunked(INNER)]);
        let loose = write_rpf9_resource(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: rbm_section(),
            physical: Vec::new(),
        })
        .expect("a loose resource");
        let resource = Rpf9NewBody::Resource {
            type_code: 0x02,
            loose,
        };
        let bytes = built(&[
            chunked(NAMED),
            chunked(UNNAMED),
            (RESOURCE.to_owned(), resource),
            (NESTED.to_owned(), Rpf9NewBody::Archive(inner)),
            (
                BARE.to_owned(),
                Rpf9NewBody::Archive(built(&[chunked(INNER)])),
            ),
        ]);
        let path = dir.join("source.rpf");
        std::fs::write(&path, bytes).expect("the archive is written");
        path
    }

    fn showing(path: &Path) -> AppState {
        let crypto = Arc::new(NoOpCryptoProvider::new());
        let archive = RpfArchive::open_with(path, crypto).expect("the archive opens as a tree");
        let tree = RpfDocument::new(archive).expect("the tree lists");
        let mut state = AppState::new();
        let doc = Document::new(DOC, Some(path.to_path_buf()), DocPayload::Rpf(tree));
        state.documents.push(doc);
        state.active_document = Some(DOC);
        state.toasts.clear();
        state
    }

    fn session(state: &AppState) -> &RpfEditSession {
        match state.document(DOC).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => &rpf.session,
            _ => panic!("the tree is open"),
        }
    }

    /// The row whose name is `name`.
    fn row(state: &AppState, name: &str) -> RpfNodeId {
        let session = session(state);
        (session.all_nodes().into_iter())
            .find(|&id| session.node(id).is_some_and(|n| n.name == name))
            .unwrap_or_else(|| panic!("no row {name}"))
    }

    /// What an unnamed row of a GTA VI table is called: its path hash.
    fn unnamed(path: &str) -> String {
        format!("hash_{:016x}", name_hash64(path.as_bytes()))
    }

    fn hash_of(state: &AppState, id: RpfNodeId) -> String {
        let session = session(state);
        copied_hash(session, session.node(id).expect("the row"))
    }

    fn wait(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done() {
            assert!(Instant::now() < deadline, "{what} never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn copy_hash_gives_a_gta_vi_rows_table_hash_in_the_form_the_filter_finds_it_by() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(keys, some_names());
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut state = showing(&source(dir.path()));

        let named = row(&state, "named.bin");
        let expected = format!("0x{:016x}", name_hash64(NAMED.as_bytes()));
        assert_eq!(hash_of(&state, named), expected, "a named row");
        let target = Target::rows(&state, DOC, vec![named], false).expect("a row");
        let copy = commands::on(&state, Action::CopyHash, target).expect("a command");
        assert_eq!(copy.enabled, Ok(()));
        assert_eq!(copy.label, format!("Copy hash  {expected}"));

        let name = unnamed(UNNAMED);
        let copied = hash_of(&state, row(&state, &name));
        assert_eq!(
            copied,
            format!("0x{:016x}", name_hash64(UNNAMED.as_bytes()))
        );
        // Copy name of an unnamed row gives the hash it is shown by, and the filter finds it by either.
        let target = Target::rows(&state, DOC, vec![row(&state, &name)], false).expect("a row");
        let copy = commands::on(&state, Action::CopyName, target).expect("a command");
        assert_eq!(copy.enabled, Ok(()));
        for needle in [copied.as_str(), name.as_str()] {
            assert!(
                row_matches(&name, None, &needle.to_ascii_lowercase()),
                "{needle}"
            );
        }
        assert!(!row_matches(&name, None, "0x0123456789abcdef"));

        let nested = row(&state, NESTED);
        let expected = format!("0x{:016x}", name_hash64(NESTED.as_bytes()));
        assert_eq!(hash_of(&state, nested), expected, "a nested archive's row");
        let bare = row(&state, "other.rpf");
        let expected = format!("0x{:016x}", name_hash64(BARE.as_bytes()));
        assert_eq!(
            hash_of(&state, bare),
            expected,
            "the table's hash, not the shown name's"
        );

        // A row of no GTA VI table keeps the hash GTA V names a file by.
        let session = match state.document_mut(DOC).map(|d| &mut d.payload) {
            Some(DocPayload::Rpf(rpf)) => &mut rpf.session,
            _ => panic!("the tree is open"),
        };
        let root = session.root_id();
        let added = (session.add_file_copy(root, "added.ydr", b"added".to_vec())).expect("added");
        let expected = format!("0x{:08X}", or_io::hash::string_hash(b"added"));
        assert_eq!(hash_of(&state, added), expected);
    }

    #[test]
    fn the_files_table_names_a_gta_vi_rows_kind_and_a_stored_row_is_not_deflated() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(keys, some_names());
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("stored.rpf");
        // Bytes that do not deflate, which the writer stores as they are.
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let noise: Vec<u8> = (0..4000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        let noisy = ("data/noise.bin".to_owned(), Rpf9NewBody::Chunked(noise));
        std::fs::write(&path, built(&[chunked(NAMED), noisy])).expect("written");
        let state = showing(&path);
        let session = session(&state);
        let facts = |name: &str| {
            crate::state::files_view::row_facts(session, row(&state, name)).expect("its facts")
        };
        let named = row(&state, "named.bin");
        let kind = crate::state::explorer::gta6_kind(session, session.node(named).expect("a row"))
            .expect("a kind");
        assert_eq!(facts("named.bin").kind, kind.label);
        assert_ne!(facts("named.bin").kind, "File");
        assert_eq!(facts("named.bin").compression, Some("Deflate"));
        assert_eq!(facts(&unnamed("data/noise.bin")).compression, Some("None"));
    }

    #[test]
    fn a_nested_gta_vi_archive_opens_as_a_folder_listed_from_its_own_table() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(keys, some_names());
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut state = showing(&source(dir.path()));
        let nested = row(&state, NESTED);
        assert!(
            session(&state).needs_expansion(nested),
            "listed only once opened"
        );

        let target = Target::rows(&state, DOC, vec![nested], false).expect("a row");
        let open = commands::on(&state, Action::OpenSelection, target).expect("a command");
        commands::run(&mut state, &egui::Context::default(), &open);
        let at = Location {
            doc: DOC,
            node: nested,
        };
        assert_eq!(state.navigation.current(), Some(at), "Open goes into it");
        assert_eq!(state.documents.len(), 1, "it opens in the tree it is in");

        state.request_expand(DOC, nested);
        wait("the nested archive's listing", || {
            state.poll_expansions();
            !session(&state).needs_expansion(nested)
        });
        let session = session(&state);
        let rows = session.children_of(nested);
        assert_eq!(rows.len(), 1, "one row, from its own table");
        let inner = session.node(rows[0]).expect("its row");
        assert_eq!(inner.name, unnamed(INNER));
        let expected = format!("0x{:016x}", name_hash64(INNER.as_bytes()));
        assert_eq!(copied_hash(session, inner), expected);
    }

    #[test]
    fn a_gta_vi_rows_preview_asks_the_load_worker_for_the_read_its_open_does() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading_with(keys, some_names());
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = source(dir.path());
        let loading = |state: &AppState| (state.load_receiver.as_ref()).map(|(p, _)| p.clone());

        let mut opened = showing(&path);
        opened.active_workspace = Workspace::Model;
        let model = row(&opened, &unnamed(RESOURCE));
        let target = Target::rows(&opened, DOC, vec![model], false).expect("a row");
        let open = commands::on(&opened, Action::OpenSelection, target).expect("a command");
        commands::run(&mut opened, &egui::Context::default(), &open);
        let read = loading(&opened).expect("the open reads on the load worker");

        let mut previewed = showing(&path);
        previewed.active_workspace = Workspace::Model;
        previewed.preview_row(DOC, model);
        assert_eq!(loading(&previewed), Some(read.clone()), "the same read");
        assert_eq!(
            previewed.documents.len(),
            1,
            "nothing is read on this thread"
        );
        wait("the preview's load", || {
            previewed.poll_load_result();
            previewed.load_receiver.is_none()
        });
        let shown = previewed
            .document_holding(&read, None)
            .expect("the body opened");
        assert!(previewed.is_preview(shown), "in the preview tab");

        // Opened, as a double click asks, it is kept.
        let target = Target::rows(&previewed, DOC, vec![model], false).expect("a row");
        let open = commands::on(&previewed, Action::OpenSelection, target).expect("a command");
        commands::run(&mut previewed, &egui::Context::default(), &open);
        assert!(!previewed.is_preview(shown), "kept in a tab of its own");

        // In a workspace that shows no drawable, a click opens nothing.
        let mut elsewhere = showing(&path);
        elsewhere.active_workspace = Workspace::Data;
        elsewhere.preview_row(DOC, model);
        assert_eq!(loading(&elsewhere), None);
    }

    /// Copy hash gives the hash the table states for every row of a few of the build's smallest
    /// archives, named or not. Install-gated: it needs the key snapshot and the build.
    #[test]
    fn copy_hash_gives_the_table_hash_of_every_row_of_the_builds_smallest_archives() {
        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        const WHAT: &str = "gta6 tree copy hash";
        const ARCHIVES: usize = 4;
        let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
        let keys_set =
            std::env::var_os("OPENRAGE_GTA6_KEYS").is_some_and(|p| Path::new(&p).exists());
        let keys = (build.as_ref().filter(|_| keys_set))
            .and_then(|_| GtaViKeys::for_install(or_title::config::config_dir().as_deref()).ok())
            .flatten();
        let (Some(build), Some(keys)) = (build, keys) else {
            let why = format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}");
            or_test_support::skip_install(WHAT, why);
            return;
        };
        let _serial = crate::ui::theme::serial();
        let keys = Arc::new(keys);
        let names = Arc::unwrap_or_clone(rpf9_table::resolver());
        let _reading = reading_with(Arc::clone(&keys), names);
        let mut archives: Vec<(u64, PathBuf)> = (std::fs::read_dir(&build).into_iter().flatten())
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
            .collect();
        archives.sort();
        // Names are game-derived, so a row that differs is reported by its archive's file and index.
        let (mut compared, mut named, mut differ) = (0usize, 0usize, Vec::new());
        for (_, path) in archives.iter().take(ARCHIVES) {
            let crypto = Arc::new(NoOpCryptoProvider::new());
            let Ok(tree) = (RpfArchive::open_with(path, crypto).map_err(|e| e.to_string()))
                .and_then(RpfDocument::new)
            else {
                continue;
            };
            let mut file = File::open(path).expect("the archive");
            let mount = Gta6Mount::open_root(&mut file, &keys).expect("its table opens");
            // Each row's start in the archive names its entry, where no two rows share it.
            let mut by_start: std::collections::HashMap<u64, Vec<u64>> = Default::default();
            for entry in &mount.archive.entries {
                if let Some(start) = entry.start.filter(|&s| s != 0) {
                    by_start.entry(start).or_default().push(entry.name64);
                }
            }
            let session = &tree.session;
            let RpfNodeBacking::ArchiveDir { archive_id } =
                &session.node(session.root_id()).expect("a root").backing
            else {
                continue;
            };
            let top = *archive_id;
            let archive = &session.archive(top).expect("the top archive").archive;
            for id in session.all_nodes() {
                let node = session.node(id).expect("a row");
                let index = match &node.backing {
                    RpfNodeBacking::ArchiveFile {
                        archive_id,
                        entry_index,
                        ..
                    } if *archive_id == top => *entry_index,
                    RpfNodeBacking::Archive {
                        source:
                            ArchiveSource::Nested {
                                parent_archive_id,
                                parent_entry_index,
                                ..
                            },
                        ..
                    } if *parent_archive_id == top => *parent_entry_index,
                    _ => continue,
                };
                let start = archive.binary_entry(index).map_or(0, |e| e.file_offset);
                let Some([stated]) = by_start.get(&start).map(Vec::as_slice) else {
                    continue;
                };
                compared += 1;
                named += usize::from(!node.name.starts_with("hash_"));
                let copied = copied_hash(session, node);
                if copied != format!("0x{stated:016x}") {
                    let file = crate::state::jobs::file_name(path);
                    differ.push(format!("{file} entry {index}"));
                }
            }
        }
        if compared == 0 {
            or_test_support::skip_install(WHAT, format_args!("no row of {BUILD_ENV} compared"));
            return;
        }
        let shown = &differ[..differ.len().min(10)];
        let what = format!("{} of {compared} rows ({named} named)", differ.len());
        assert!(differ.is_empty(), "{what} copy another hash: {shown:?}");
    }
}
