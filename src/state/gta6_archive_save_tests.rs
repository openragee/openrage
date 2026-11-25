//! Hermetic checks of saving into a copy of a GTA VI archive, on archives built here and enciphered
//! under a synthetic white-box key, so no game byte is involved.

use std::io::Cursor;
use std::time::{Duration, Instant};

use or_io::container::rpf::{read_rpf9_body, Gta6Mount, Rpf9Sections, RPF9_SLOT};
use or_io::gta6::hash::name_hash64;
use or_io::meta::Value;

use super::*;
use crate::state::opening_an_rpf9_archive::said;
use crate::state::rpf9_scan::{body_path, View};
use crate::state::rpf_archive_open::tree_verbs::tests::{
    rbm_section, source, synthetic_keys, table_form_pso, PSO, RESOURCE,
};
use crate::state::{MetaDocument, ToastType};

/// The mount of `bytes` and the start of its entry `name`.
fn start_of(bytes: &[u8], name: &str) -> (Gta6Mount, u64) {
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let mount = Gta6Mount::open_root(&mut Cursor::new(bytes), &keys).expect("the archive opens");
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(name.as_bytes()))
        .unwrap_or_else(|| panic!("no entry {name}"));
    let start = mount.absolute_start(entry).expect("a start");
    (mount, start)
}

/// The resource of entry `name` of `bytes`, read past its slot.
fn resource(bytes: &[u8], name: &str) -> Rpf9Sections {
    let (_, start) = start_of(bytes, name);
    let (find, body) = read_rpf9_body(Cursor::new(bytes), start + RPF9_SLOT, u64::MAX)
        .expect("the resource reads past its slot");
    Rpf9Sections::of(&find, &body).expect("a resource")
}

/// `meta` with its root's u32 set to `value`, as the form does.
fn edited(mut meta: MetaDocument, value: u64) -> DocPayload {
    let parsed = meta.parsed.as_mut().expect("it parsed");
    parsed.instance.fields[0].1 = Value::UInt(value);
    meta.after_visual_edit_in_place();
    DocPayload::Meta(meta)
}

/// An app showing `payload` as the document opened from `path`.
fn showing(path: PathBuf, payload: DocPayload) -> AppState {
    let mut state = AppState::new();
    let id = 9_000;
    state.documents.push(Document::new(id, Some(path), payload));
    state.active_document = Some(id);
    state.toasts.clear();
    state
}

/// Poll until the save says how it ended.
fn finished(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        state.poll_archive_saves();
        let ended = (state.toasts.iter())
            .any(|t| matches!(t.toast_type, ToastType::Success | ToastType::Error));
        if ended {
            return;
        }
        assert!(Instant::now() < deadline, "the save never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_edited_pso_body_saved_into_a_copy_reopens_with_the_edit() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = source(dir.path());
    let before = std::fs::read(&source).expect("the source");
    let (_, start) = start_of(&before, PSO);
    let meta = MetaDocument::from_bytes(table_form_pso());
    let mut state = showing(body_path(&source, start, View::Bytes), edited(meta, 78));
    assert_eq!(state.can_save_into_archive_copy(), Ok(()));
    assert_eq!(
        state.archive_copy_name().as_deref(),
        Some("source_edited.rpf")
    );

    let out = dir.path().join("copy.rpf");
    assert!(state.save_into_archive_copy_with(out.clone(), synthetic_keys));
    finished(&mut state);
    assert!(
        said(
            &state,
            ToastType::Success,
            &["copy.rpf", "with the edit in its entry's place"]
        ),
        "the result says the game refuses the copy: {:?}",
        state.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
    );
    let copy = std::fs::read(&out).expect("the copy");
    let (mount, _) = start_of(&copy, PSO);
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(PSO.as_bytes()))
        .expect("the entry");
    let body = (mount.read_entry(Cursor::new(&copy), entry)).expect("the body reads");
    let reopened = or_io::meta::MetaDocument::parse(&Default::default(), body).expect("it reads");
    assert_eq!(reopened.instance.fields[0].1, Value::UInt(78));
    assert_eq!(
        resource(&copy, RESOURCE),
        resource(&before, RESOURCE),
        "the other entry is kept"
    );
}

#[test]
fn an_edited_headerless_body_saved_into_a_copy_reopens_with_the_edit() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = source(dir.path());
    let before = std::fs::read(&source).expect("the source");
    let (_, start) = start_of(&before, RESOURCE);
    let meta =
        MetaDocument::from_gta_vi_headerless(&rbm_section(), &[]).expect("the resource opens");
    // A scan opens a headerless resource past its slot.
    let path = body_path(&source, start + RPF9_SLOT, View::Headerless);
    let mut state = showing(path, edited(meta, 79));

    let out = dir.path().join("copy.rpf");
    assert!(state.save_into_archive_copy_with(out.clone(), synthetic_keys));
    finished(&mut state);
    let toasts: Vec<&str> = state.toasts.iter().map(|t| t.message.as_str()).collect();
    assert!(
        said(
            &state,
            ToastType::Success,
            &["with the edit in its entry's place"]
        ),
        "{toasts:?}"
    );
    let copy = std::fs::read(&out).expect("the copy");
    let sections = resource(&copy, RESOURCE);
    let (instance, _) =
        or_io::meta::binary::rbm::gta6::open(&sections.virtual_section, &sections.physical)
            .expect("the resource opens");
    assert_eq!(instance.fields[0].1, Value::UInt(79));
}

#[test]
fn the_archive_read_is_never_written_and_what_names_no_entry_is_refused() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = source(dir.path());
    let before = std::fs::read(&source).expect("the source");
    let (_, start) = start_of(&before, PSO);
    let keys = synthetic_keys().expect("the synthetic snapshot parses");

    let over =
        write_copy(&source, start, table_form_pso(), &source, &keys).expect_err("over the source");
    assert!(over.contains("never written"), "{over}");
    let existing = dir.path().join("existing.rpf");
    std::fs::write(&existing, b"kept").expect("written");
    assert!(write_copy(&source, start, table_form_pso(), &existing, &keys).is_err());
    assert_eq!(std::fs::read(&existing).expect("kept"), b"kept");
    let none = dir.path().join("none.rpf");
    let why =
        write_copy(&source, start + 4, table_form_pso(), &none, &keys).expect_err("no entry there");
    assert!(why.contains("no entry"), "{why}");
    assert!(!none.exists(), "a refused save leaves no file");

    // An edit that saves still leaves the source as it was.
    let meta = MetaDocument::from_bytes(table_form_pso());
    let mut state = showing(body_path(&source, start, View::Bytes), edited(meta, 80));
    assert!(state.save_into_archive_copy_with(dir.path().join("copy.rpf"), synthetic_keys));
    finished(&mut state);
    assert!(said(&state, ToastType::Success, &["copy.rpf"]));
    assert_eq!(std::fs::read(&source).expect("the source"), before);

    // A document of a file of its own is no archive's.
    let loose = dir.path().join("loose.pso");
    std::fs::write(&loose, table_form_pso()).expect("written");
    let state = showing(
        loose,
        DocPayload::Meta(MetaDocument::from_bytes(table_form_pso())),
    );
    assert!(state.can_save_into_archive_copy().is_err());
}
