use crate::state::Rpf9Document;

use super::*;
use or_io::container::rpf::{Rpf9Builder, Rpf9Toc};
use or_test_support::rpf9;

pub(super) fn keys() -> GtaViKeys {
    GtaViKeys::parse(&rpf9::keys()).expect("synthetic keys")
}

fn encrypted(rows: &[(u64, u64, u64, u8)]) -> Vec<u8> {
    Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: rpf9::table(rows),
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .expect("synthetic encrypted table")
}

pub(super) fn fixture(child_size: u64, valid_child: bool) -> (tempfile::TempDir, PathBuf) {
    let name = or_io::gta6::hash::name_hash64(b"child");
    let mut bytes = encrypted(&[(name, 0x1000, child_size, 0), (0, 0x800, 16, 0xff)]);
    bytes.resize(0x1000, 0x78);
    if valid_child {
        bytes.extend(encrypted(&[(0, 0x800, 16, 0xff)]));
    }
    bytes.resize(0x2000, 0x78);
    let dir = tempfile::tempdir().expect("scratch directory");
    let path = dir.path().join("table.rpf");
    std::fs::write(&path, bytes).expect("archive fixture");
    (dir, path)
}

pub(super) fn wait(doc: &Rpf9TableDocument) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let mut tree = doc.tree();
        tree.poll();
        if tree.pending().is_none() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child table never completed"
        );
        drop(tree);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn table_open_requires_keys_and_never_uses_the_clear_bodies() {
    let (_dir, path) = fixture(0x1000, true);
    let result = Rpf9TableDocument::open_using(&path, || Err("missing test keys".into()));
    assert!(matches!(result, Err(error) if error == "missing test keys"));
    let doc = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    let tree = doc.tree();
    assert_eq!(tree.nodes.len(), 1, "children stay unopened");
    assert_eq!(tree.nodes[&0].archive.entries.len(), 2);
    assert_eq!(tree.nodes[&0].archive.entries[1].name, None);
    assert_eq!(
        tree.nodes[&0].archive.entries[1].display_name(),
        "hash_0000000000000000"
    );
}

#[test]
fn expanding_mounts_only_the_selected_child_and_caches_its_table() {
    let (_dir, path) = fixture(0x1000, true);
    let doc = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    doc.toggle(0, 0);
    wait(&doc);
    assert_eq!(doc.tree().nodes.len(), 2);
    assert_eq!(doc.tree().nodes[&0x1000].archive.entries.len(), 1);
    assert!(doc.tree().expanded.contains(&(0, 0)));
    doc.toggle(0, 0);
    assert!(!doc.tree().expanded.contains(&(0, 0)));
    doc.toggle(0, 0);
    assert!(doc.tree().pending().is_none());
    assert!(doc.tree().expanded.contains(&(0, 0)));
}

#[test]
fn an_invalid_child_stays_an_error_in_the_parent_tree() {
    for (size, valid) in [(0x1000, false), (0x2000, true)] {
        let (_dir, path) = fixture(size, valid);
        let doc = Rpf9TableDocument::open_using(&path, || Ok(keys()))
            .unwrap()
            .unwrap();
        doc.toggle(0, 0);
        wait(&doc);
        let tree = doc.tree();
        assert_eq!(tree.nodes.len(), 1);
        assert!(tree.errors.contains_key(&(0, 0)));
        assert!(tree.expanded.is_empty());
    }
}

#[test]
fn a_changed_archive_is_not_read_through_a_stale_table() {
    let (_dir, path) = fixture(0x1000, true);
    let doc = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(0x3000)
        .unwrap();
    doc.toggle(0, 0);
    wait(&doc);
    assert!(doc.tree().errors[&(0, 0)].contains("changed on disk"));
}

#[test]
fn another_archive_generation_does_not_ask_for_gta_vi_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.rpf");
    std::fs::write(&path, b"7FPR").unwrap();
    let result = Rpf9TableDocument::open_using(&path, || panic!("not an RPF9"));
    assert!(matches!(result, Ok(None)));
}

#[test]
fn the_tree_arrow_loads_and_collapses_the_child_table() {
    use crate::state::nothing_draws_over_what_it_belongs_to::{painted, Panel};
    use crate::state::{AppState, DocPayload, Prefs};
    let _serial = crate::ui::theme::serial();
    let (_dir, path) = fixture(0x1000, true);
    let doc = Arc::new(
        Rpf9TableDocument::open_using(&path, || Ok(keys()))
            .unwrap()
            .unwrap(),
    );
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        path,
        DocPayload::Rpf9(Rpf9Document::Table(doc.clone())),
        None,
        None,
    );
    let mut panel = Panel::new(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(560.0, 900.0),
    ));
    let state = std::cell::RefCell::new(state);
    let draw = |ui: &mut egui::Ui| {
        assert!(crate::ui::panels::inspector::draw_archive_table(
            ui,
            &mut state.borrow_mut()
        ));
    };
    panel.frame(Vec::new(), draw);
    let before = panel.frame(Vec::new(), draw);
    let at = painted(&before)
        .into_iter()
        .find(|p| p.text == "▸")
        .expect("expand arrow")
        .rect
        .center();
    assert_eq!(doc.tree().nodes.len(), 1, "drawing must not mount children");
    let click = |panel: &mut Panel| {
        panel.frame(vec![egui::Event::PointerMoved(at)], draw);
        for pressed in [true, false] {
            panel.frame(
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
                draw,
            );
        }
    };
    click(&mut panel);
    wait(&doc);
    assert!(
        doc.tree().expanded.contains(&(0, 0)),
        "the arrow must expand its row"
    );
    assert_eq!(doc.tree().nodes.len(), 2);
    let expanded = panel.frame(Vec::new(), draw);
    assert!(painted(&expanded).iter().any(|p| p.text == "▾"));
    click(&mut panel);
    assert!(
        doc.tree().expanded.is_empty(),
        "the second click must collapse its row"
    );
}

#[test]
fn a_child_table_is_a_job_that_can_be_cancelled() {
    use crate::state::{jobs::JobKind, AppState, DocPayload, Prefs};
    let (_dir, path) = fixture(0x1000, true);
    let doc = Arc::new(
        Rpf9TableDocument::open_using(&path, || Ok(keys()))
            .unwrap()
            .unwrap(),
    );
    let mut state = AppState::from_prefs(Prefs::default());
    state.insert_document(
        path,
        DocPayload::Rpf9(Rpf9Document::Table(doc.clone())),
        None,
        None,
    );
    assert!(doc.job().is_none());
    doc.toggle(0, 0);
    assert!(state
        .running_jobs()
        .iter()
        .any(|job| job.kind == JobKind::ArchiveTable));
    state.cancel_job(JobKind::ArchiveTable);
    assert!(doc.job().is_none());
    state.poll_jobs();
    assert_eq!(
        doc.tree().nodes.len(),
        1,
        "a cancelled result must not mount the child"
    );
}
