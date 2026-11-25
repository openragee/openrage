use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::{Gta6EntryType, GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_ENTRY_ROW_20};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{write_dictionary, DictionaryModel, ModelTexture};

use super::document::rpf9::Rpf9TableDocument;
use super::opening_a_ydd::finish_load;
use super::rpf_session::{RpfDocument, RpfEditSession};
use super::{
    AppState, DocPayload, Document, DocumentId, DocumentKind, Prefs, RpfNodeBacking, RpfNodeId,
};

const CHILD_NAME: &[u8] = b"rpf9_files_nested";
const FILE_NAME: &[u8] = b"rpf9_files_notes";
const TEXT: &[u8; 16] = b"table only bytes";

fn keys() -> GtaViKeys {
    GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap()
}

fn encrypt(bytes: Vec<u8>) -> Vec<u8> {
    Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes,
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .unwrap()
}

struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
    table: Arc<Rpf9TableDocument>,
    archive: Vec<u8>,
    body: Vec<u8>,
}

impl Fixture {
    fn new(corrupt_child: bool) -> Self {
        let sections = write_dictionary(&DictionaryModel {
            textures: vec![
                ModelTexture::tiled("files_preview", 64, 64, 71, &[vec![0x5a; 2048]]).unwrap(),
            ],
            capacity: 1,
        })
        .unwrap();
        let body = [
            sections.virtual_section.as_slice(),
            sections.physical.as_slice(),
        ]
        .concat();
        let count = u32::try_from(body.len()).unwrap();
        let mut toc = Rpf9Toc::decrypt(
            &encrypt(or_test_support::rpf9::resource_table(&[(
                count, count, false,
            )])),
            &keys(),
        )
        .unwrap();
        let row = toc
            .groups
            .iter()
            .find(|group| group.name_hash == GROUP_ENTRY_ROW_20)
            .unwrap()
            .data
            .start;
        for (offset, value) in [
            (0, u32::try_from(sections.virtual_section.len()).unwrap()),
            (4, u32::try_from(sections.physical.len()).unwrap() | 3),
            (16, 5),
        ] {
            toc.bytes[row + offset..row + offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut child = Rpf9Builder::serialize(&toc, &keys()).unwrap();
        if corrupt_child {
            child[..4].copy_from_slice(b"bad!");
        }
        child.resize(0x1010, 0);
        child.extend(&body);
        let mut archive = encrypt(or_test_support::rpf9::table(&[
            (name_hash64(CHILD_NAME), 0x1000, child.len() as u64, 0),
            (name_hash64(FILE_NAME), 0x800, TEXT.len() as u64, 0xff),
        ]));
        archive.resize(0x800, 0);
        archive.extend(TEXT);
        archive.resize(0x1000, 0);
        archive.extend(child);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("textures.rpf");
        std::fs::write(&path, &archive).unwrap();
        let table = Arc::new(
            Rpf9TableDocument::open_using(&path, || Ok(keys()))
                .unwrap()
                .unwrap(),
        );
        Self {
            dir,
            path,
            table,
            archive,
            body,
        }
    }

    fn unchanged(&self) {
        assert_eq!(std::fs::read(&self.path).unwrap(), self.archive);
        assert_eq!(std::fs::read_dir(self.dir.path()).unwrap().count(), 1);
    }
}

fn session(state: &AppState, doc: DocumentId) -> &RpfEditSession {
    let DocPayload::Rpf(rpf) = &state.document(doc).unwrap().payload else {
        panic!("the Files document must retain its ordinary tree");
    };
    &rpf.session
}

fn session_mut(state: &mut AppState, doc: DocumentId) -> &mut RpfEditSession {
    let DocPayload::Rpf(rpf) = &mut state.document_mut(doc).unwrap().payload else {
        panic!("the Files document must retain its ordinary tree");
    };
    &mut rpf.session
}

fn settle(state: &mut AppState, doc: DocumentId) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while state.expander.is_listing(doc) {
        state.poll_expansions();
        assert!(Instant::now() < deadline, "Files expansion did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn folder(fixture: &Fixture, install: bool) -> (AppState, DocumentId, RpfNodeId) {
    let mut state = AppState::from_prefs(Prefs::default());
    let doc = state.alloc_doc_id();
    let path = fixture.dir.path().to_path_buf();
    let document = if install {
        Document::new_game_root(
            doc,
            path.clone(),
            RpfDocument::new_game_root(path, state.keys.provider()),
        )
    } else {
        Document::new_folder(
            doc,
            path.clone(),
            RpfDocument::new_folder(path, state.keys.provider()),
        )
    };
    state.documents.push(document);
    state.set_active_document(doc);
    let root = session(&state, doc).root_id();
    state.request_expand(doc, root);
    settle(&mut state, doc);
    let rows = session(&state, doc).children_of(root);
    assert_eq!(rows.len(), 1);
    let archive = rows[0];
    let node = session_mut(&mut state, doc).node_mut(archive).unwrap();
    assert_eq!(node.name, "textures.rpf");
    assert!(matches!(node.backing, RpfNodeBacking::Archive { .. }));
    node.backing = RpfNodeBacking::Rpf9Archive {
        table: fixture.table.clone(),
        owner: None,
        opened_base: None,
        open_error: None,
    };
    (state, doc, archive)
}

fn child_named(state: &AppState, doc: DocumentId, parent: RpfNodeId, name: &str) -> RpfNodeId {
    let tree = session(state, doc);
    tree.children_of(parent)
        .into_iter()
        .find(|id| tree.node(*id).is_some_and(|row| row.name == name))
        .unwrap_or_else(|| panic!("Files has no child {name}"))
}

fn nested(state: &AppState, doc: DocumentId, parent: RpfNodeId) -> RpfNodeId {
    let tree = session(state, doc);
    tree.children_of(parent)
        .into_iter()
        .find(|id| tree.node(*id).unwrap().rpf9_type() == Some(Gta6EntryType::Archive))
        .expect("the nested archive is identified without a filename extension")
}

#[test]
fn folders_and_installs_expand_rpf9_tables_and_nested_archives_lazily() {
    for install in [false, true] {
        let fixture = Fixture::new(false);
        let (mut state, doc, archive) = folder(&fixture, install);
        assert!(session(&state, doc).children_of(archive).is_empty());
        assert_eq!(fixture.table.tree().nodes.len(), 1);
        state.request_expand(doc, archive);
        settle(&mut state, doc);
        assert_eq!(state.documents.len(), 1);
        assert_eq!(session(&state, doc).children_of(archive).len(), 2);
        let child = nested(&state, doc, archive);
        let row = session(&state, doc).node(child).unwrap();
        assert_eq!(row.name, format!("hash_{:016x}", name_hash64(CHILD_NAME)));
        assert_eq!(row.rpf9_entry(), Some((0, name_hash64(CHILD_NAME))));
        assert_eq!(
            row.display_name(),
            format!("hash_{:016x}.rpf", name_hash64(CHILD_NAME))
        );
        assert!(row.children.is_empty());
        assert_eq!(
            fixture.table.tree().nodes.len(),
            1,
            "listing must not mount a child table"
        );
        let loaded_rows = session(&state, doc).all_nodes().len();
        state.request_expand(doc, archive);
        settle(&mut state, doc);
        assert_eq!(session(&state, doc).all_nodes().len(), loaded_rows);
        state.request_expand(doc, child);
        settle(&mut state, doc);
        assert_eq!(fixture.table.tree().nodes.len(), 2);
        let files = session(&state, doc).children_of(child);
        assert_eq!(files.len(), 1);
        let file = session(&state, doc).node(files[0]).unwrap();
        assert_eq!(file.rpf9_type(), Some(Gta6EntryType::TextureDictionary));
        assert_eq!(file.display_name(), format!("{}.ytd", file.name));
        assert_eq!(
            fixture.table.entry(0x1000, 0).unwrap().name,
            file.display_name()
        );
        assert!(session(&state, doc).is_rpf9(file.id));
        let icon = crate::ui::icons::rpf_node_icon(file, false, false, true, false);
        let expected = crate::ui::icons::doc_kind_icon(DocumentKind::Ytd, false);
        assert_eq!(icon.glyph, expected.glyph);
        assert_eq!(icon.badge, None);
        let facts = super::files_view::row_facts(session(&state, doc), file.id).unwrap();
        assert_eq!(facts.edition, Some("GTA VI"));
        assert_eq!(facts.version, None);
        assert_eq!(facts.rpf9_entry, file.rpf9_entry());
        let expected_dialog = format!("Export {}", file.display_name());
        let target = crate::ui::commands::Target::Rows {
            doc,
            rows: vec![file.id],
            into: child,
            centre: false,
        };
        let command =
            crate::ui::commands::on(&state, crate::ui::commands::Action::ExportAs, target)
                .expect("the file offers Export as");
        assert!(command.enabled.is_ok());
        crate::ui::commands::run(&mut state, &egui::Context::default(), &command);
        assert_eq!(
            crate::platform::file_dialogs::last_dialog(),
            Some(expected_dialog)
        );
        assert!(
            !state.extract_jobs.is_running(),
            "the stand-in cancelled the dialog"
        );
        fixture.unchanged();
    }
}

#[test]
fn known_rpf9_paths_make_virtual_folders_without_changing_the_table_address() {
    let fixture = Fixture::new(false);
    {
        let mut tree = fixture.table.tree();
        let entries = &mut tree.nodes.get_mut(&0).unwrap().mount.archive.entries;
        entries[0].name = Some("packs/child.rpf".into());
        entries[1].name = Some("texts/readme.txt".into());
    }
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let packs = child_named(&state, doc, archive, "packs");
    let texts = child_named(&state, doc, archive, "texts");
    for directory in [packs, texts] {
        assert!(matches!(
            session(&state, doc).node(directory).unwrap().backing,
            RpfNodeBacking::Rpf9Dir { .. }
        ));
    }
    let child = child_named(&state, doc, packs, "child.rpf");
    assert_eq!(
        session(&state, doc).node(child).unwrap().display_name(),
        "child.rpf"
    );
    assert_eq!(
        session(&state, doc).node(child).unwrap().rpf9_entry(),
        Some((0, name_hash64(CHILD_NAME)))
    );
    let file = child_named(&state, doc, texts, "readme.txt");
    assert_eq!(
        session(&state, doc).node(file).unwrap().display_name(),
        "readme.txt"
    );
    assert_eq!(
        session(&state, doc).node(file).unwrap().rpf9_entry(),
        Some((1, name_hash64(FILE_NAME)))
    );
    state.open_rpf_entry(doc, file);
    let opened = finish_load(&mut state).unwrap();
    let document = state.document(opened).unwrap();
    assert!(matches!(document.payload, DocPayload::Text(_)));
    assert_eq!(
        document.read_only,
        Some(super::document::rpf9::entry::READ_ONLY)
    );
    let path = document.path.as_ref().unwrap();
    assert!(path.to_string_lossy().contains(".rpf9-table/1/"));
    assert!(!path.exists());
    fixture.unchanged();
}

#[test]
fn opening_a_files_texture_uses_the_table_preview_and_refuses_mutations() {
    let fixture = Fixture::new(false);
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let child = nested(&state, doc, archive);
    state.request_expand(doc, child);
    settle(&mut state, doc);
    let file = session(&state, doc).children_of(child)[0];
    state.open_rpf_entry(doc, file);
    let opened = finish_load(&mut state).unwrap();
    let document = state.document(opened).unwrap();
    assert_eq!(
        document.read_only,
        Some(super::document::rpf9::entry::READ_ONLY)
    );
    let DocPayload::Resource(or_io::Asset::TextureDict(dictionary)) = &document.payload else {
        panic!("the Files row must open the existing texture preview");
    };
    let texture = dictionary.get("files_preview").unwrap();
    assert_eq!((texture.width, texture.height), (64, 64));
    assert_eq!(texture.backing_store, vec![0x5a; 2048]);
    assert!(!document.path.as_ref().unwrap().exists());
    state.open_rpf_entry(doc, file);
    assert_eq!(state.active_document, Some(opened));
    assert_eq!(state.documents.len(), 2);

    let tree = session_mut(&mut state, doc);
    let planned = super::extract_job::plan(tree, file, &super::extract_job::TypeFilter::All);
    assert_eq!(planned.len(), 1);
    assert_eq!(
        planned[0]
            .read(or_io::container::rpf::BodyForm::Decoded)
            .unwrap(),
        fixture.body
    );
    let root = tree.root_id();
    assert!(!tree.is_rpf9(root));
    assert!(tree.set_bytes_for(file, b"replacement".to_vec()).is_err());
    assert!(tree.rename_node(file, "changed.ytd").is_err());
    assert!(tree.move_node(file, root).is_err());
    assert!(tree.delete_node(file).is_err());
    assert!(tree.add_folder(child, "new").is_err());
    assert!(tree.add_file_copy(child, "new.bin", Vec::new()).is_err());
    assert!(tree.dirty_archive_ids().is_empty());
    assert!(tree.node(file).is_some());
    let mut extracted = Vec::new();
    fixture
        .table
        .entry(0x1000, 0)
        .unwrap()
        .extract(&mut extracted)
        .unwrap();
    assert_eq!(extracted, fixture.body);
    fixture.unchanged();
}

#[test]
fn a_failed_nested_rpf9_table_stays_an_error_in_files_without_a_scan_document() {
    let fixture = Fixture::new(true);
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let child = nested(&state, doc, archive);
    state.request_expand(doc, child);
    settle(&mut state, doc);
    let row = session(&state, doc).node(child).unwrap();
    assert!(row.children.is_empty());
    assert!(matches!(
        &row.backing,
        RpfNodeBacking::Rpf9Archive {
            open_error: Some(_),
            opened_base: None,
            ..
        }
    ));
    assert_eq!(state.documents.len(), 1);
    assert_eq!(fixture.table.tree().nodes.len(), 1);
    fixture.unchanged();
}

#[test]
fn drawing_a_closed_rpf9_row_reads_nothing_and_its_arrow_expands_it() {
    use super::nothing_draws_over_what_it_belongs_to::{painted, Panel};

    let _serial = crate::ui::theme::serial();
    let fixture = Fixture::new(false);
    let (mut state, doc, archive) = folder(&fixture, false);
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 900.0),
    ));
    let draw = |ui: &mut egui::Ui, state: &mut AppState| {
        crate::ui::panels::hierarchy::draw(ui, state);
    };
    panel.frame(Vec::new(), |ui| draw(ui, &mut state));
    let output = panel.frame(Vec::new(), |ui| draw(ui, &mut state));
    let row = painted(&output)
        .into_iter()
        .find(|item| item.text.ends_with("textures.rpf"))
        .expect("the archive row is drawn");
    let at = output
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            if node.label() != Some("Expand") {
                return None;
            }
            let bounds = node.bounds()?;
            let centre = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            (centre.y >= row.rect.top() && centre.y <= row.rect.bottom()).then_some(centre)
        })
        .next()
        .expect("the archive row's expand arrow");
    assert!(!state.expander.is_listing(doc));
    assert!(session(&state, doc).children_of(archive).is_empty());
    assert_eq!(fixture.table.tree().nodes.len(), 1);
    panel.frame(vec![egui::Event::PointerMoved(at)], |ui| {
        draw(ui, &mut state)
    });
    for pressed in [true, false] {
        panel.frame(
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            |ui| draw(ui, &mut state),
        );
    }
    panel.frame(Vec::new(), |ui| draw(ui, &mut state));
    settle(&mut state, doc);
    assert_eq!(session(&state, doc).children_of(archive).len(), 2);
    assert_eq!(fixture.table.tree().nodes.len(), 1);
    assert_eq!(state.documents.len(), 1);
    fixture.unchanged();
}

#[test]
fn recovered_path_conflicts_and_duplicate_hashes_keep_every_table_index() {
    for (first, second, duplicate_hash) in [
        (Some("same"), Some("same"), false),
        (Some("folder/child.rpf"), Some("folder"), false),
        (Some("known/child.rpf"), Some("../escape.txt"), false),
        (None, None, true),
    ] {
        let fixture = Fixture::new(false);
        {
            let mut tree = fixture.table.tree();
            let entries = &mut tree.nodes.get_mut(&0).unwrap().mount.archive.entries;
            entries[0].name = first.map(str::to_string);
            entries[1].name = second.map(str::to_string);
            if duplicate_hash {
                entries[1].name64 = entries[0].name64;
            }
        }
        let (mut state, doc, archive) = folder(&fixture, false);
        state.request_expand(doc, archive);
        settle(&mut state, doc);
        let tree = session(&state, doc);
        let mut indices = tree
            .all_nodes()
            .into_iter()
            .filter_map(|id| {
                let node = tree.node(id).unwrap();
                let (index, _) = node.rpf9_entry()?;
                assert!(tree.is_descendant(id, archive));
                assert!(!matches!(node.name.as_str(), "." | ".." | "escape.txt"));
                Some(index)
            })
            .collect::<Vec<_>>();
        indices.sort_unstable();
        assert_eq!(indices, [0, 1], "{first:?}, {second:?}");
        let mut names = tree
            .children_of(archive)
            .into_iter()
            .map(|id| tree.node(id).unwrap().name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), tree.children_of(archive).len());
        fixture.unchanged();
    }
}

#[test]
fn files_pictures_follow_table_types_and_unknown_entries_wait_for_open() {
    use super::files_view::pictures::{picture_kind_for, Made, PictureKey, PictureKind, Pictures};

    let fixture = Fixture::new(false);
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let unknown = session(&state, doc)
        .children_of(archive)
        .into_iter()
        .find(|id| {
            session(&state, doc)
                .node(*id)
                .unwrap()
                .rpf9_entry()
                .is_some_and(|(index, _)| index == 1)
        })
        .unwrap();
    session_mut(&mut state, doc).node_mut(unknown).unwrap().name = "misleading.ytd".into();
    let node = session(&state, doc).node(unknown).unwrap();
    assert_eq!(picture_kind_for(node), None);
    assert!(matches!(
        super::entry_preview::preview(session(&state, doc), unknown)
            .unwrap()
            .body,
        super::entry_preview::PreviewBody::Open
    ));
    assert_eq!(
        fixture.table.tree().entry_type(0, 1),
        (Gta6EntryType::Unknown, false)
    );

    let child = nested(&state, doc, archive);
    state.request_expand(doc, child);
    settle(&mut state, doc);
    let file = session(&state, doc).children_of(child)[0];
    assert_eq!(
        picture_kind_for(session(&state, doc).node(file).unwrap()),
        Some(PictureKind::Texture)
    );
    assert!(matches!(
        super::entry_preview::preview(session(&state, doc), file)
            .unwrap()
            .body,
        super::entry_preview::PreviewBody::Picture
    ));
    assert_eq!(
        fixture.table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, false)
    );
    let key = PictureKey {
        doc,
        node: file,
        side: 32,
        size: fixture.body.len() as u64,
    };
    let mut pictures = Pictures::default();
    assert!(pictures.ask(session(&state, doc), key, state.keys.provider()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let (done, made) = loop {
        if let Some(result) = pictures.finished().pop() {
            break result;
        }
        assert!(Instant::now() < deadline, "RPF9 thumbnail never completed");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(done, key);
    let image = match made {
        Made::Image(image) => image,
        Made::Failed(error) => panic!("RPF9 thumbnail: {error}"),
        Made::Model { .. } => panic!("a dictionary must make a texture picture"),
    };
    assert_eq!(
        (image.width, image.height, image.data.len()),
        (32, 32, 32 * 32 * 4)
    );
    assert_eq!(
        fixture.table.tree().entry_type(0x1000, 0),
        (Gta6EntryType::TextureDictionary, true)
    );
    fixture.unchanged();
}

#[test]
fn queued_files_keep_their_rpf9_row_bindings_and_the_files_tree() {
    use super::nothing_draws_over_what_it_belongs_to::{painted, Panel};

    let _serial = crate::ui::theme::serial();
    let fixture = Fixture::new(false);
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let child = nested(&state, doc, archive);
    let notes = session(&state, doc)
        .children_of(archive)
        .into_iter()
        .find(|id| !session(&state, doc).node(*id).unwrap().is_dir())
        .unwrap();
    state.request_expand(doc, child);
    settle(&mut state, doc);
    let texture = session(&state, doc).children_of(child)[0];

    state.open_rpf_entry(doc, texture);
    assert!(state.load_receiver.is_some());
    state.open_rpf_entry(doc, notes);
    finish_load(&mut state).unwrap();
    assert_eq!(state.documents.len(), 3);
    for (row, kind) in [(texture, DocumentKind::Ytd), (notes, DocumentKind::Hex)] {
        let binding = super::RpfEntryBinding {
            parent_doc_id: doc,
            entry_id: row,
        };
        let opened = state
            .documents
            .iter()
            .find(|document| document.rpf_entry_binding == Some(binding))
            .unwrap_or_else(|| panic!("queued row {row} lost its binding"));
        assert_eq!(opened.kind, kind);
        assert_eq!(
            opened.read_only,
            Some(super::document::rpf9::entry::READ_ONLY)
        );
    }
    let notes_doc = state.active_document().unwrap();
    assert_eq!(notes_doc.rpf_entry_binding.unwrap().entry_id, notes);
    state.open_rpf_entry(doc, texture);
    assert_eq!(state.documents.len(), 3);
    assert_eq!(
        state
            .active_document()
            .unwrap()
            .rpf_entry_binding
            .unwrap()
            .entry_id,
        texture
    );
    assert_eq!(state.document(doc).unwrap().kind, DocumentKind::Folder);

    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 900.0),
    ));
    panel.frame(Vec::new(), |ui| {
        crate::ui::panels::hierarchy::draw(ui, &mut state)
    });
    let output = panel.frame(Vec::new(), |ui| {
        crate::ui::panels::hierarchy::draw(ui, &mut state)
    });
    assert!(painted(&output)
        .iter()
        .any(|item| item.text.ends_with("textures.rpf")));
    assert_eq!(session(&state, doc).children_of(archive).len(), 2);
    fixture.unchanged();
}

#[test]
fn a_cached_child_table_is_not_reused_after_its_archive_changes() {
    let fixture = Fixture::new(false);
    let (mut state, doc, archive) = folder(&fixture, false);
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let child = nested(&state, doc, archive);
    fixture.table.expand_table(0, 0).unwrap();
    assert_eq!(fixture.table.tree().nodes.len(), 2);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&fixture.path)
        .unwrap()
        .set_len(fixture.archive.len() as u64 + 16)
        .unwrap();
    state.request_expand(doc, child);
    settle(&mut state, doc);
    let row = session(&state, doc).node(child).unwrap();
    assert!(row.children.is_empty());
    let RpfNodeBacking::Rpf9Archive {
        open_error: Some(error),
        opened_base: None,
        ..
    } = &row.backing
    else {
        panic!("a stale child table must remain an unopened error");
    };
    assert!(error.contains("changed on disk"), "{error}");
    assert_eq!(state.documents.len(), 1);
    assert_eq!(std::fs::read_dir(fixture.dir.path()).unwrap().count(), 1);
}

#[test]
fn renaming_a_disk_parent_unloads_its_rpf9_tables_and_updates_the_archive_path() {
    let fixture = Fixture::new(false);
    let outer = tempfile::tempdir().unwrap();
    let before = outer.path().join("before");
    std::fs::create_dir(&before).unwrap();
    let old_archive = before.join("textures.rpf");
    std::fs::write(&old_archive, &fixture.archive).unwrap();
    let table = Arc::new(
        Rpf9TableDocument::open_using(&old_archive, || Ok(keys()))
            .unwrap()
            .unwrap(),
    );

    let mut state = AppState::from_prefs(Prefs::default());
    let doc = state.alloc_doc_id();
    let root_path = outer.path().to_path_buf();
    let document = Document::new_folder(
        doc,
        root_path.clone(),
        RpfDocument::new_folder(root_path, state.keys.provider()),
    );
    state.documents.push(document);
    state.set_active_document(doc);
    let root = session(&state, doc).root_id();
    state.request_expand(doc, root);
    settle(&mut state, doc);
    let directory = child_named(&state, doc, root, "before");
    state.request_expand(doc, directory);
    settle(&mut state, doc);
    let archive = child_named(&state, doc, directory, "textures.rpf");
    session_mut(&mut state, doc)
        .node_mut(archive)
        .unwrap()
        .backing = RpfNodeBacking::Rpf9Archive {
        table,
        owner: None,
        opened_base: None,
        open_error: None,
    };
    state.request_expand(doc, archive);
    settle(&mut state, doc);
    let child = nested(&state, doc, archive);
    state.request_expand(doc, child);
    settle(&mut state, doc);
    assert!(!session(&state, doc).children_of(child).is_empty());

    state.rpf_rename_node(doc, directory, "after").unwrap();
    let new_archive = outer.path().join("after/textures.rpf");
    let tree = session(&state, doc);
    let row = tree.node(archive).unwrap();
    assert_eq!(row.backing.fs_path(), Some(new_archive.as_path()));
    assert!(row.children.is_empty());
    assert!(matches!(
        row.backing,
        RpfNodeBacking::Archive {
            scanned: false,
            open_error: None,
            ..
        }
    ));
    assert!(tree.node(child).is_none());
    assert!(!tree.is_rpf9(archive));
    assert!(!old_archive.exists());
    assert_eq!(std::fs::read(new_archive).unwrap(), fixture.archive);
    fixture.unchanged();
}

#[test]
fn a_standalone_rpf9_table_and_an_unrelated_folder_share_files() {
    use super::nothing_draws_over_what_it_belongs_to::{painted, Panel};

    let _serial = crate::ui::theme::serial();
    let fixture = Fixture::new(false);
    fixture.table.expand_table(0, 0).unwrap();
    fixture.table.tree().expanded.insert((0, 0));
    let mut state = AppState::from_prefs(Prefs::default());
    let archive = state.insert_document(
        fixture.path.clone(),
        DocPayload::Rpf9Table(fixture.table.clone()),
        None,
        None,
    );
    let other = tempfile::tempdir().unwrap();
    let folder = other.path().join("unrelated_folder");
    std::fs::create_dir(&folder).unwrap();
    let doc = state.alloc_doc_id();
    state.documents.push(Document::new_folder(
        doc,
        folder.clone(),
        RpfDocument::new_folder(folder, state.keys.provider()),
    ));
    state.set_active_document(archive);
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(900.0, 1000.0),
    ));
    panel.frame(Vec::new(), |ui| {
        crate::ui::panels::hierarchy::draw(ui, &mut state)
    });
    let output = panel.frame(Vec::new(), |ui| {
        crate::ui::panels::hierarchy::draw(ui, &mut state)
    });
    let shown = painted(&output);
    assert!(shown.iter().any(|item| item.text.ends_with("textures")));
    assert!(shown
        .iter()
        .any(|item| item.text.ends_with("unrelated_folder")));
    let kind = super::files_view::rpf9_type_name(Gta6EntryType::TextureDictionary);
    assert!(shown.iter().any(|item| item.text == kind));
    assert_eq!(state.documents.len(), 2);
    fixture.unchanged();
}

#[test]
fn a_queued_rpf9_open_keeps_the_requested_mount_when_another_mount_holds_the_same_path() {
    let fixture = Fixture::new(false);
    let (mut state, first, first_archive) = folder(&fixture, false);
    let (mut other_state, _, second_archive) = folder(&fixture, false);
    let mut second_document = other_state.documents.remove(0);
    let second = state.alloc_doc_id();
    second_document.id = second;
    state.documents.push(second_document);

    let mut textures = Vec::new();
    for (doc, archive) in [(first, first_archive), (second, second_archive)] {
        state.request_expand(doc, archive);
        settle(&mut state, doc);
        let child = nested(&state, doc, archive);
        state.request_expand(doc, child);
        settle(&mut state, doc);
        textures.push(session(&state, doc).children_of(child)[0]);
    }
    state.open_rpf_entry(first, textures[0]);
    let first_texture_document = finish_load(&mut state).unwrap();
    let first_path = state.document(first_texture_document).unwrap().path.clone();
    assert_eq!(state.documents.len(), 3);

    let notes = session(&state, first)
        .children_of(first_archive)
        .into_iter()
        .find(|id| !session(&state, first).node(*id).unwrap().is_dir())
        .unwrap();
    state.open_rpf_entry(first, notes);
    assert!(state.load_receiver.is_some());
    state.open_rpf_entry(second, textures[1]);
    let second_texture_document = finish_load(&mut state).unwrap();
    assert_ne!(first_texture_document, second_texture_document);
    assert_eq!(state.documents.len(), 5);
    let second_texture = state.document(second_texture_document).unwrap();
    assert_eq!(second_texture.path, first_path);
    assert_eq!(second_texture.kind, DocumentKind::Ytd);
    for (doc, row, opened) in [
        (first, textures[0], first_texture_document),
        (second, textures[1], second_texture_document),
    ] {
        let binding = super::RpfEntryBinding {
            parent_doc_id: doc,
            entry_id: row,
        };
        assert_eq!(
            state.document(opened).unwrap().rpf_entry_binding,
            Some(binding)
        );
        assert_eq!(state.entry_document(binding), Some(opened));
    }
    state.open_rpf_entry(first, textures[0]);
    assert_eq!(state.active_document, Some(first_texture_document));
    state.open_rpf_entry(second, textures[1]);
    assert_eq!(state.active_document, Some(second_texture_document));
    assert_eq!(state.documents.len(), 5);
    fixture.unchanged();
}

#[path = "rpf9_files_tests/copied_paths.rs"]
mod copied_paths;
