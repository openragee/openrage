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

/// Every byte of `copy` outside the stored extent of the entry `name` of `before` is as in `before`.
fn only_the_body_differs(before: &[u8], copy: &[u8], name: &str) {
    let (mount, start) = start_of(before, name);
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(name.as_bytes()))
        .expect("the entry");
    let (start, end) = (
        start as usize,
        (start + entry.stored.expect("stored")) as usize,
    );
    assert_eq!(
        copy.len(),
        before.len(),
        "the copy keeps the archive's size"
    );
    assert_eq!(copy[..start], before[..start], "every byte before the body");
    assert_eq!(copy[end..], before[end..], "every byte after the body");
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
            &["copy.rpf", "in its entry's place"]
        ),
        "{:?}",
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
fn the_command_asks_where_the_copy_goes_under_a_short_title_and_writes_it_there() {
    use crate::ui::commands::{run_action, Action};

    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = source(dir.path());
    let (_, start) = start_of(&std::fs::read(&source).expect("the source"), PSO);
    let meta = MetaDocument::from_bytes(table_form_pso());
    let mut state = showing(body_path(&source, start, View::Bytes), edited(meta, 81));

    // The command reaches its dialog, which a stand-in given no answer cancels.
    let ctx = egui::Context::default();
    run_action(&mut state, &ctx, Action::SaveIntoArchiveCopy);
    assert_eq!(file_dialogs::last_dialog().as_deref(), Some("Save a copy"));

    let out = dir.path().join("copy.rpf");
    file_dialogs::answer_next(out.clone());
    assert!(state.ask_and_save_with(synthetic_keys));
    assert_eq!(file_dialogs::last_dialog().as_deref(), Some("Save a copy"));
    finished(&mut state);
    assert!(
        said(
            &state,
            ToastType::Success,
            &["copy.rpf", "in its entry's place"]
        ),
        "{:?}",
        state.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
    );
    let copy = std::fs::read(&out).expect("the copy");
    let (mount, _) = start_of(&copy, PSO);
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(PSO.as_bytes()))
        .expect("the entry");
    let body = (mount.read_entry(Cursor::new(&copy), entry)).expect("the body reads");
    let reopened = or_io::meta::MetaDocument::parse(&Default::default(), body).expect("it reads");
    assert_eq!(reopened.instance.fields[0].1, Value::UInt(81));
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
    // A headerless resource's loose form is stored under a new row, so the copy is rebuilt.
    let named = format!("{:#X} is a headerless resource", start + RPF9_SLOT);
    assert!(
        said(
            &state,
            ToastType::Success,
            &[
                "with the edit in its entry's place",
                "kept it from being patched in place",
                &named
            ]
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
fn a_body_of_its_entrys_size_saves_as_a_patched_copy_and_one_of_another_size_is_rebuilt() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = source(dir.path());
    let before = std::fs::read(&source).expect("the source");
    let (_, start) = start_of(&before, PSO);
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    // The root's u32, 77, sits 24 bytes into the PSIN frame's content.
    let mut same_size = table_form_pso();
    assert_eq!(same_size[32], 77);
    same_size[32] = 78;

    let out = dir.path().join("patched.rpf");
    let saved = write_copy(&source, start, None, same_size.clone(), &out, &keys).expect("written");
    assert_eq!(saved, Saved::Patched);
    let copy = std::fs::read(&out).expect("the copy");
    only_the_body_differs(&before, &copy, PSO);
    let (mount, _) = start_of(&copy, PSO);
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(PSO.as_bytes()))
        .expect("the entry");
    let body = (mount.read_entry(Cursor::new(&copy), entry)).expect("the body reads");
    assert_eq!(body, same_size);

    let mut longer = table_form_pso();
    longer.extend([0u8; 16]);
    let out = dir.path().join("rebuilt.rpf");
    let saved = write_copy(&source, start, None, longer, &out, &keys).expect("written");
    let Saved::Rebuilt { tables, kept, .. } = saved else {
        panic!("a body of another size changes its row");
    };
    assert!(tables);
    let size = table_form_pso().len();
    let why = format!(
        "{start:#X} is {} bytes, not the {size} its row states, 16 bytes over",
        size + 16
    );
    assert_eq!(kept, vec![why]);
    assert_eq!(std::fs::read(&source).expect("the source"), before);
    // Each save wrote its one copy and nothing beside it.
    let mut listed: Vec<String> = (std::fs::read_dir(dir.path()).expect("the folder lists"))
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    listed.sort();
    assert_eq!(listed, ["patched.rpf", "rebuilt.rpf", "source.rpf"]);
}

#[test]
fn the_archive_read_is_never_written_and_what_names_no_entry_is_refused() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let source = source(dir.path());
    let before = std::fs::read(&source).expect("the source");
    let (_, start) = start_of(&before, PSO);
    let keys = synthetic_keys().expect("the synthetic snapshot parses");

    let over = write_copy(&source, start, None, table_form_pso(), &source, &keys)
        .expect_err("over the source");
    assert!(over.contains("never written"), "{over}");
    let existing = dir.path().join("existing.rpf");
    std::fs::write(&existing, b"kept").expect("written");
    assert!(write_copy(&source, start, None, table_form_pso(), &existing, &keys).is_err());
    assert_eq!(std::fs::read(&existing).expect("kept"), b"kept");
    let none = dir.path().join("none.rpf");
    let why = write_copy(&source, start + 4, None, table_form_pso(), &none, &keys)
        .expect_err("no entry there");
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
