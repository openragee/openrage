use super::super::tests::{fixture, keys, wait};
use super::*;
use crate::state::{Document, Prefs};

fn opened() -> (tempfile::TempDir, Arc<Rpf9TableDocument>) {
    let (dir, path) = fixture(0x1000, true);
    let doc = Arc::new(
        Rpf9TableDocument::open_using(&path, || Ok(keys()))
            .unwrap()
            .unwrap(),
    );
    (dir, doc)
}

#[test]
fn a_nested_file_opens_and_extracts_the_same_exact_bytes() {
    let _serial = crate::ui::theme::serial();
    let (dir, table) = opened();
    table.toggle(0, 0);
    wait(&table);
    let source = table.entry(0x1000, 0).unwrap();
    assert_eq!(source.bytes().unwrap(), vec![0x78; 16]);
    let (archive, chain) = address(&source.path).unwrap();
    assert_eq!(archive, table.path);
    assert_eq!(chain, [0, 0]);
    let reloaded = Rpf9TableDocument::open_using(&archive, || Ok(keys()))
        .unwrap()
        .unwrap();
    assert_eq!(
        read_chain(&reloaded, &chain).unwrap().bytes().unwrap(),
        source.bytes().unwrap()
    );

    let mut state = AppState::from_prefs(Prefs::default());
    state.open_rpf9_entry(&table, 0x1000, 0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() {
        state.poll_load_result();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let doc = state.active_document().unwrap();
    assert_eq!(doc.read_only, Some(READ_ONLY));
    let DocPayload::Hex(hex) = &doc.payload else {
        panic!("entry was not opened as bytes")
    };
    assert_eq!(hex.bytes, source.bytes().unwrap());
    state.open_rpf9_entry(&table, 0x1000, 0);
    assert_eq!(
        state.documents.len(),
        1,
        "opening twice selects the same tab"
    );

    let dest = dir.path().join("out");
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join(&source.name), b"keep me").unwrap();
    assert!(state.extract_rpf9_entry(&table, 0x1000, 0, dest.clone()));
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(dest.join(&source.name)).unwrap(), b"keep me");
    assert_eq!(
        std::fs::read(dest.join(format!("{} (2)", source.name))).unwrap(),
        source.bytes().unwrap()
    );
}

#[test]
fn a_changed_archive_fails_open_and_extraction_without_writing_a_file() {
    use crate::state::extract_job::{run_extraction, ExtractItem, OnClash};
    let (dir, table) = opened();
    let source = table.entry(0, 1).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&table.path)
        .unwrap()
        .set_len(0x3000)
        .unwrap();
    assert!(source.payload().is_err());
    let dest = dir.path().join("out");
    let item = ExtractItem::written(source.name.clone(), Arc::new(move || source.bytes()));
    let report = run_extraction(
        vec![item],
        dest.clone(),
        or_io::container::rpf::BodyForm::Decoded,
        false,
        OnClash::KeepBoth,
        or_job::Cancel::new(),
        or_job::Progress::new(),
    );
    assert_eq!(report.counts.failed, 1);
    assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
}

#[test]
fn a_name_cannot_escape_the_destination_or_alias_another_entry() {
    let (_dir, table) = opened();
    for name in [
        "../../evil.txt",
        "C:\\outside\\evil.txt",
        "/absolute/evil.txt",
        "..",
        "CON",
        "x:stream",
        "end.",
    ] {
        table.tree().nodes.get_mut(&0).unwrap().archive.entries[1].name = Some(name.into());
        let source = table.entry(0, 1).unwrap();
        assert_eq!(Path::new(&source.name).components().count(), 1);
        assert!(!source.name.contains(['/', '\\', ':']));
        assert!(!["..", "CON", "end."].contains(&source.name.as_str()));
        assert_eq!(address(&source.path).unwrap().1, [1]);
        let doc = Document::new(
            1,
            Some(source.path),
            DocPayload::Hex(super::super::super::HexDocument {
                bytes: vec![],
                banner: String::new(),
            }),
        );
        assert_eq!(doc.read_only, Some(READ_ONLY));
    }
    assert!(address(&table.path.join(MARKER).join("bad").join("x")).is_err());
}

#[test]
fn unsupported_entry_opening_does_not_start_a_scan() {
    let _serial = crate::ui::theme::serial();
    let (_dir, table) = opened();
    table.tree().nodes.get_mut(&0).unwrap().archive.entries[1].class = Some(14);
    let mut state = AppState::from_prefs(Prefs::default());
    state.open_rpf9_entry(&table, 0, 1);
    assert!(state.load_receiver.is_none());
    assert!(state.documents.is_empty());
    assert!(state.show_error_dialog);
    assert!(state
        .error_dialog_message
        .contains("unsupported entry layout"));
}

#[test]
fn the_open_button_loads_the_selected_table_entry() {
    use crate::state::nothing_draws_over_what_it_belongs_to::{painted, Panel};
    let _serial = crate::ui::theme::serial();
    let (_dir, table) = opened();
    table.tree().selected = Some((0, 1));
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        table.path.clone(),
        DocPayload::Rpf9(crate::state::Rpf9Document::Table(table.clone())),
        None,
        None,
    );
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800.0, 600.0),
    ));
    panel.frame(Vec::new(), |ui| {
        crate::ui::panels::inspector::draw_body(ui, &mut state)
    });
    let before = panel.frame(Vec::new(), |ui| {
        crate::ui::panels::inspector::draw_body(ui, &mut state)
    });
    assert!(
        state.load_receiver.is_none(),
        "drawing alone must not read an entry"
    );
    let at = painted(&before)
        .iter()
        .find(|p| p.text == "Open")
        .expect("open button")
        .rect
        .center();
    panel.frame(vec![egui::Event::PointerMoved(at)], |ui| {
        crate::ui::panels::inspector::draw_body(ui, &mut state)
    });
    for pressed in [true, false] {
        panel.frame(
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            |ui| crate::ui::panels::inspector::draw_body(ui, &mut state),
        );
    }
    assert!(
        state.load_receiver.is_some(),
        "clicking must start the entry read"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while state.load_receiver.is_some() {
        state.poll_load_result();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(
        state.active_document().unwrap().payload,
        DocPayload::Hex(_)
    ));
    panel.frame(Vec::new(), |ui| {
        assert!(
            crate::ui::panels::inspector::draw_archive_table(ui, &mut state),
            "the parent tree stays available while its file is open"
        );
    });
}

#[test]
fn text_opens_as_text_and_a_gta_vi_resource_is_not_given_a_gta_v_reader() {
    let (_dir, table) = opened();
    table.tree().nodes.get_mut(&0).unwrap().archive.entries[1].name = Some("info.txt".into());
    assert!(matches!(
        table.entry(0, 1).unwrap().payload().unwrap(),
        DocPayload::Text(_)
    ));
    table.tree().nodes.get_mut(&0).unwrap().archive.entries[1].name = Some("model.ydr".into());
    assert!(matches!(
        table.entry(0, 1).unwrap().payload().unwrap(),
        DocPayload::Hex(_)
    ));
}
