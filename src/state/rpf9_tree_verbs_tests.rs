//! Hermetic checks of a GTA VI archive opened as the tree, on archives built here and enciphered
//! under a synthetic white-box key, so no game byte is involved; one install-gated check of a real one.

use std::io::Cursor;
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey};
use or_io::container::rpf::{Gta6Context, Gta6Entry, NameResolver, Rpf9Form};
use or_io::container::NoOpCryptoProvider;
use or_io::gta6::hash::name_hash64;
use or_job::{Cancel, Progress};

use super::*;
use crate::state::extract_job::{plan_selection, run_extraction, OnClash};
use crate::state::opening_an_rpf9_archive::{headerless_rbm, said};
use crate::state::rpf_session::RpfDocument;
use crate::state::{Document, ToastType};

const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
const KEY_ID: u32 = 0xDEAD_BEEF;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
/// The round tables feeding each output lane of the cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
const WORDS: usize = 60;
const MAPS: usize = 16;
const MAP_LEN: usize = 256;
const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;

pub(crate) const PSO: &str = "data/table.pso";
pub(crate) const RESOURCE: &str = "data/headerless.ymt";

fn push_perm(out: &mut Vec<u8>, k: usize) {
    out.extend((0..256usize).map(|x| ((x + k * 7 + 1) % 256) as u8));
}

fn mix8(b: u8) -> u8 {
    let (mut y, mut acc) = (0u8, 0u8);
    for i in (0..8).rev() {
        acc ^= (b >> i) & 1;
        y |= acc << i;
    }
    y
}

/// A synthetic white-box snapshot holding the bootstrap key and one archive key: byte permutations
/// for the maps and invertible affine round tables, the shape the encrypt side inverts.
fn snapshot() -> Vec<u8> {
    let mut slot_of = [0usize; 16];
    for lane in LANES.iter() {
        for (slot, &k) in lane.iter().enumerate() {
            slot_of[k] = slot;
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"ORGTAVIK");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0x4342u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 32]);
    out.extend_from_slice(&[0x11u8; 16]);
    out.extend_from_slice(&((MAPS * MAP_LEN) as u32).to_le_bytes());
    for k in 0..MAPS {
        push_perm(&mut out, k);
    }
    out.extend_from_slice(&((MAPS * MAP_LEN * 4) as u32).to_le_bytes());
    for (k, &slot) in slot_of.iter().enumerate() {
        let constant = 0x0A0B_0C0Du32.wrapping_mul(k as u32 + 1);
        for x in 0..256u32 {
            let s = (x as u8).rotate_left(3) ^ (k as u8).wrapping_mul(0x5B);
            out.extend_from_slice(&(constant ^ (u32::from(mix8(s)) << (slot * 8))).to_le_bytes());
        }
    }
    out.extend_from_slice(&2u32.to_le_bytes());
    for &id in &[BOOTSTRAP_REGISTRY_ID, REGISTRY_ID] {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(BLOB_LEN as u32).to_le_bytes());
        out.extend_from_slice(&[0x8e, 0x9b, 0x3b, 0x4e]);
        for w in 0..WORDS as u32 {
            out.extend_from_slice(&(w.wrapping_mul(2_654_435_761) ^ id as u32).to_le_bytes());
        }
        for k in 0..MAPS {
            push_perm(&mut out, k + 3);
        }
    }
    out
}

pub(crate) fn synthetic_keys() -> Result<Arc<GtaViKeys>, String> {
    GtaViKeys::parse(&snapshot())
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(ident);
    out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(content);
}

/// A PSO body in GTA VI's table form whose root holds one u32, 77, with a `zppp` checksum word.
pub(crate) fn table_form_pso() -> Vec<u8> {
    const ROOT: u32 = 0x1111_0001;
    let mut psin = [ROOT.to_le_bytes(), 4u32.to_le_bytes()].concat();
    psin.extend([0xAA; 8]);
    psin.extend([0x70; 8]);
    psin.extend(77u32.to_le_bytes());
    psin.resize(40, 0);
    let mut lsch = [ROOT.to_le_bytes(), 0x5151_5151u32.to_le_bytes()].concat();
    lsch.extend([0, 3, 0, 0, 0, 0, 0, 0]);
    lsch.extend([0xAA; 8]);
    lsch.extend([4, 0, 0, 0, 0, 0, 1, 0]);
    lsch.extend(0x2222_0000u32.to_le_bytes());
    lsch.extend([0, 0, 0, 0, 0x15, 0, 0, 0, 0, 0, 0, 0]);
    let mut lrsc = vec![0u8; 0x50];
    lrsc[0..4].copy_from_slice(b"0DRP");
    lrsc[4..6].copy_from_slice(&0x7Au16.to_le_bytes());
    lrsc[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
    lrsc[0x18..0x38].fill(0xBB);
    lrsc[0x38..0x3A].copy_from_slice(&1u16.to_le_bytes());
    lrsc[0x3C..0x3E].copy_from_slice(&1u16.to_le_bytes());
    lrsc[0x40..0x48].fill(0xBB);
    let mut out = Vec::new();
    frame(&mut out, b"PSIN", &psin);
    frame(&mut out, b"LSCH", &lsch);
    frame(&mut out, b"LENM", &[]);
    frame(&mut out, b"LSNS", &0u32.to_le_bytes());
    frame(&mut out, b"LGID", &[]);
    frame(&mut out, b"LRSC", &lrsc);
    let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
    chks.extend([0xDE, 0xAD, 0xBE, 0xEF]);
    chks.extend(b"zppp");
    frame(&mut out, b"CHKS", &chks);
    out
}

/// A headerless RBM resource whose root holds one u32, 77, as a loose file.
/// The RBM's virtual section, long enough for the 24 bytes of its block map at 0xF0, which the
/// reader's fixture cuts at 0x100.
pub(crate) fn rbm_section() -> Vec<u8> {
    let mut virtual_section = headerless_rbm(0x5000_00E0);
    virtual_section.resize(0x110, 0);
    virtual_section
}

fn loose_rbm() -> Vec<u8> {
    write_rpf9_resource(&Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: rbm_section(),
        physical: Vec::new(),
    })
    .expect("a loose resource")
}

/// An archive of `entries`, built from nothing as `dir/source.rpf`.
fn archive_of(dir: &Path, entries: Vec<(String, Rpf9NewBody)>) -> PathBuf {
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let plan = build(&entries, &key, &keys).expect("the archive builds");
    let mut bytes = Vec::new();
    plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)
        .expect("the archive is laid out");
    let path = dir.join("source.rpf");
    std::fs::write(&path, bytes).expect("the archive is written");
    path
}

/// The RBM resource as an entry under a 20-byte row.
fn resource_entry() -> (String, Rpf9NewBody) {
    let loose = loose_rbm();
    let body = Rpf9NewBody::Resource {
        type_code: 0x02,
        loose,
    };
    (RESOURCE.to_owned(), body)
}

/// An archive of the PSO body under a 24-byte row and the RBM resource under a 20-byte row, built
/// from nothing in `dir`.
pub(crate) fn source(dir: &Path) -> PathBuf {
    let pso = (PSO.to_owned(), Rpf9NewBody::Chunked(table_form_pso()));
    archive_of(dir, vec![pso, resource_entry()])
}

const OTHER: &str = "data/other.bin";

fn other() -> Vec<u8> {
    b"a second clear body, chunked under its own row. ".repeat(40)
}

/// The two entries of [`source`] and a third clear one.
fn three(dir: &Path) -> PathBuf {
    let pso = (PSO.to_owned(), Rpf9NewBody::Chunked(table_form_pso()));
    let other = (OTHER.to_owned(), Rpf9NewBody::Chunked(other()));
    archive_of(dir, vec![pso, other, resource_entry()])
}

/// Archives opened while this is held are read with the keys it was made with; the context held
/// before comes back once it is dropped.
struct Reading(Option<Arc<Gta6Context>>);

impl Drop for Reading {
    fn drop(&mut self) {
        Gta6Context::swap_given(self.0.take());
    }
}

fn reading_with(keys: Arc<GtaViKeys>) -> Reading {
    let names = Arc::new(NameResolver::new());
    let context = Gta6Context { keys, names };
    Reading(Gta6Context::swap_given(Some(Arc::new(context))))
}

/// The archive at `path` as the tree every archive opens as.
fn tree(path: &Path) -> RpfDocument {
    let crypto = Arc::new(NoOpCryptoProvider::new());
    let archive = RpfArchive::open_with(path, crypto).expect("the archive opens as a tree");
    RpfDocument::new(archive).expect("the tree lists")
}

/// Every file row of the tree.
fn files(session: &RpfEditSession) -> Vec<RpfNodeId> {
    (session.all_nodes().into_iter())
        .filter(|&id| session.node(id).is_some_and(|n| !n.is_dir()))
        .collect()
}

/// The file row whose body is `body`.
fn row_holding(session: &RpfEditSession, body: &[u8]) -> RpfNodeId {
    (files(session).into_iter())
        .find(|&id| session.bytes_for(id).is_some_and(|b| b.as_slice() == body))
        .expect("a row holds the body")
}

/// The bytes of each file in `dir`, sorted.
fn contents(dir: &Path) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = (std::fs::read_dir(dir).expect("the folder lists"))
        .map(|e| std::fs::read(e.expect("an entry").path()).expect("the file reads"))
        .collect();
    out.sort();
    out
}

fn mount_of(bytes: &[u8]) -> Gta6Mount {
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    Gta6Mount::open_root(&mut Cursor::new(bytes), &keys).expect("the archive opens")
}

/// The top-level entry `name` of `mount`.
fn entry_named<'m>(mount: &'m Gta6Mount, name: &str) -> &'m Gta6Entry {
    (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(name.as_bytes()))
        .unwrap_or_else(|| panic!("no entry {name}"))
}

/// The body of entry `name` of the archive `bytes`.
fn body_of(bytes: &[u8], name: &str) -> Vec<u8> {
    let mount = mount_of(bytes);
    (mount.read_entry(Cursor::new(bytes), entry_named(&mount, name))).expect("the body reads")
}

/// The sections of the headerless resource `name` of the archive `bytes`, read past its slot.
fn stored_sections(bytes: &[u8], name: &str) -> Rpf9Sections {
    let mount = mount_of(bytes);
    let start = (mount.absolute_start(entry_named(&mount, name))).expect("a start");
    let (find, body) = read_rpf9_body(Cursor::new(bytes), start + RPF9_SLOT, u64::MAX)
        .expect("the resource reads past its slot");
    Rpf9Sections::of(&find, &body).expect("a resource")
}

const DOC: DocumentId = 9_100;

/// An app showing `tree` as the archive at `path` opened on its own.
fn showing(path: PathBuf, tree: RpfDocument) -> AppState {
    let mut state = AppState::new();
    let doc = Document::new(DOC, Some(path), DocPayload::Rpf(tree));
    state.documents.push(doc);
    state.active_document = Some(DOC);
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

fn toasts(state: &AppState) -> Vec<&str> {
    state.toasts.iter().map(|t| t.message.as_str()).collect()
}

#[test]
fn three_rows_of_a_gta_vi_tree_extract_as_three_files_of_their_real_sizes() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(synthetic_keys().expect("the synthetic snapshot parses"));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = three(dir.path());
    let bytes = std::fs::read(&path).expect("the archive");
    let tree = tree(&path);
    let rows = files(&tree.session);
    assert_eq!(rows.len(), 3, "a row for each entry");

    let (items, collided) = plan_selection(&tree.session, &rows, &TypeFilter::All);
    assert_eq!(collided, 0);
    let out = dir.path().join("out");
    let report = run_extraction(
        items,
        out.clone(),
        BodyForm::Decoded,
        false,
        OnClash::Replace,
        Cancel::new(),
        Progress::new(),
    );
    assert_eq!(report.counts.succeeded, 3, "{:?}", report.failures);

    let loose = write_rpf9_resource(&stored_sections(&bytes, RESOURCE)).expect("a loose resource");
    let mut expected = vec![table_form_pso(), other(), loose];
    expected.sort();
    let written = contents(&out);
    let sizes = |files: &[Vec<u8>]| files.iter().map(Vec::len).collect::<Vec<_>>();
    assert_eq!(sizes(&written), sizes(&expected));
    assert_eq!(written, expected);
    // The headerless resource is written in the loose form that reopens as the sections it holds.
    let reopened = (written.iter()).find_map(|file| {
        let (find, body) = read_rpf9_body(Cursor::new(file), 0, u64::MAX).ok()?;
        Rpf9Sections::of(&find, &body)
    });
    assert_eq!(reopened, Some(stored_sections(&bytes, RESOURCE)));
}

#[test]
fn a_stop_leaves_the_file_in_hand_whole_and_writes_no_row_after_it() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(synthetic_keys().expect("the synthetic snapshot parses"));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let tree = tree(&three(dir.path()));
    let cancel = Cancel::new();
    let stop = cancel.clone();
    let first: Arc<crate::state::gta6_other_bodies_save::Write> = Arc::new(move || {
        stop.set();
        Ok(b"written whole".to_vec())
    });
    let mut items = vec![ExtractItem::written("first.bin".into(), first)];
    items.extend(plan_selection(&tree.session, &files(&tree.session), &TypeFilter::All).0);
    let out = dir.path().join("out");
    let report = run_extraction(
        items,
        out.clone(),
        BodyForm::Decoded,
        false,
        OnClash::Replace,
        cancel,
        Progress::new(),
    );
    assert!(report.cancelled);
    assert_eq!(contents(&out), vec![b"written whole".to_vec()]);
}

#[test]
fn a_replaced_row_saved_to_a_new_file_reopens_with_the_new_body_and_the_source_is_untouched() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(synthetic_keys().expect("the synthetic snapshot parses"));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = three(dir.path());
    let before = std::fs::read(&path).expect("the source");
    let mut tree = tree(&path);
    let row = row_holding(&tree.session, &other());
    let shown = (tree.session.node(row).map(|n| n.name.clone())).expect("the row");
    let replaced = b"a replaced body, which deflates. ".repeat(40);
    (tree.session.set_bytes_for(row, replaced.clone())).expect("the row takes the bytes");
    let mut state = showing(path.clone(), tree);
    let offered = state.gta6_tree_copy_name(DOC);
    assert_eq!(offered.as_deref(), Some("source_edited.rpf"));

    let out = dir.path().join("copy.rpf");
    let started = state.save_gta6_tree_with(DOC, &out, synthetic_keys);
    assert_eq!(started, Some(true));
    finished(&mut state);
    // A body of another size changes its row, so the copy is rebuilt, and the toast names the row.
    let sizes = format!("{shown} is 1320 bytes, not the 1920 its row states, 600 bytes short");
    let refused = [
        "copy.rpf",
        "the game refuses",
        "kept it from being patched in place",
        sizes.as_str(),
    ];
    assert!(
        said(&state, ToastType::Success, &refused),
        "{:?}",
        toasts(&state)
    );
    let copy = std::fs::read(&out).expect("the copy");
    assert_eq!(body_of(&copy, OTHER), replaced);
    assert_eq!(
        body_of(&copy, PSO),
        table_form_pso(),
        "the other rows are kept"
    );
    assert_eq!(
        stored_sections(&copy, RESOURCE),
        stored_sections(&before, RESOURCE)
    );
    assert_eq!(std::fs::read(&path).expect("the source"), before);
}

/// Every byte of `copy` outside the stored extent of the entry `name` of `before` is as in `before`,
/// so every table span is byte-identical.
fn only_the_body_differs(before: &[u8], copy: &[u8], name: &str) {
    let mount = mount_of(before);
    let entry = entry_named(&mount, name);
    let start = mount.absolute_start(entry).expect("a start") as usize;
    let end = start + entry.stored.expect("a stored extent") as usize;
    assert_eq!(
        copy.len(),
        before.len(),
        "the copy keeps the archive's size"
    );
    assert_eq!(copy[..start], before[..start], "every byte before the body");
    assert_eq!(copy[end..], before[end..], "every byte after the body");
    assert_ne!(copy[start..end], before[start..end], "the body changed");
}

#[test]
fn a_replaced_row_of_its_own_size_saves_as_a_patched_copy_whose_table_is_as_read() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(synthetic_keys().expect("the synthetic snapshot parses"));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = three(dir.path());
    let before = std::fs::read(&path).expect("the source");
    let mut tree = tree(&path);
    let row = row_holding(&tree.session, &other());
    // One letter over the row's size deflates far under the chunk's stored size, so it fits.
    let replaced = vec![b'a'; other().len()];
    (tree.session.set_bytes_for(row, replaced.clone())).expect("the row takes the bytes");
    let mut state = showing(path.clone(), tree);

    let out = dir.path().join("copy.rpf");
    assert_eq!(
        state.save_gta6_tree_with(DOC, &out, synthetic_keys),
        Some(true)
    );
    finished(&mut state);
    let patched = [
        "copy.rpf",
        "is a patched copy",
        "byte-identical",
        "has not been run",
    ];
    assert!(
        said(&state, ToastType::Success, &patched),
        "{:?}",
        toasts(&state)
    );
    assert!(!toasts(&state).iter().any(|t| t.contains("refuses")));
    let copy = std::fs::read(&out).expect("the copy");
    only_the_body_differs(&before, &copy, OTHER);
    assert_eq!(body_of(&copy, OTHER), replaced);
    assert_eq!(body_of(&copy, PSO), table_form_pso());
    assert_eq!(std::fs::read(&path).expect("the source"), before);
}

/// `len` bytes no deflater shrinks into a chunk that held [`other`].
fn noise(len: usize) -> Vec<u8> {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// The names of the files in `dir`, sorted.
fn listed(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = (std::fs::read_dir(dir).expect("the folder lists"))
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    out.sort();
    out
}

/// The archive of [`three`] in `dir`, its [`OTHER`] row replaced by `body`, as the tree opens it,
/// and that row's name.
fn edited_tree(dir: &Path, body: Vec<u8>) -> (PathBuf, RpfDocument, String) {
    let path = three(dir);
    let mut tree = tree(&path);
    let row = row_holding(&tree.session, &other());
    let shown = (tree.session.node(row).map(|n| n.name.clone())).expect("the row");
    (tree.session.set_bytes_for(row, body)).expect("the row takes the bytes");
    (path, tree, shown)
}

#[test]
fn a_save_that_will_be_rebuilt_is_planned_writing_nothing_and_then_writes_only_that_copy() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading_with(Arc::clone(&keys));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, tree, shown) = edited_tree(dir.path(), noise(other().len()));
    let edits = tree_edits(&tree.session).expect("the tree's edits");

    let planned = plan_tree(&path, &edits, &keys).expect("the save plans");
    assert!(!planned.patched(), "{:?}", planned.kept);
    assert_eq!(
        listed(dir.path()),
        ["source.rpf"],
        "planning writes nothing"
    );
    // The row of the right size whose body does not fit its chunk is named, with by how much.
    let named = |why: &String| why.contains(&shown) && why.ends_with("bytes over");
    assert!(planned.kept.iter().any(named), "{:?}", planned.kept);

    let out = dir.path().join("copy.rpf");
    let saved = write_planned(&path, &out, &keys, &planned).expect("the copy is written");
    assert!(
        matches!(saved, Saved::Rebuilt { tables: true, .. }),
        "{saved:?}"
    );
    assert_eq!(listed(dir.path()), ["copy.rpf", "source.rpf"]);
    assert_eq!(
        body_of(&std::fs::read(&out).expect("the copy"), OTHER),
        noise(other().len())
    );
}

#[test]
fn a_save_that_will_be_patched_is_planned_writing_nothing_and_then_writes_only_that_copy() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading_with(Arc::clone(&keys));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, tree, _) = edited_tree(dir.path(), vec![b'a'; other().len()]);
    let edits = tree_edits(&tree.session).expect("the tree's edits");

    let planned = plan_tree(&path, &edits, &keys).expect("the save plans");
    assert!(planned.patched(), "{:?}", planned.kept);
    assert_eq!(
        listed(dir.path()),
        ["source.rpf"],
        "planning writes nothing"
    );

    let out = dir.path().join("copy.rpf");
    let saved = write_planned(&path, &out, &keys, &planned).expect("the copy is written");
    assert_eq!(saved, Saved::Patched);
    assert_eq!(listed(dir.path()), ["copy.rpf", "source.rpf"]);
    let before = std::fs::read(&path).expect("the source");
    only_the_body_differs(&before, &std::fs::read(&out).expect("the copy"), OTHER);
}

/// Plan the save of the tree shown, have its dialog pick `out`, and poll until the save ends; the
/// dialog's title.
fn saved_through_the_dialog(state: &mut AppState, out: &Path) -> String {
    assert!(state.plan_tree_save(DOC, synthetic_keys));
    file_dialogs::answer_next(out.to_path_buf());
    finished(state);
    file_dialogs::last_dialog().expect("the save dialog was shown")
}

#[test]
fn the_save_dialog_says_which_copy_the_save_writes_and_names_the_row_that_does_not_fit() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(synthetic_keys().expect("the synthetic snapshot parses"));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, tree, shown) = edited_tree(dir.path(), noise(other().len()));
    let mut state = showing(path, tree);
    let out = dir.path().join("copy.rpf");

    let title = saved_through_the_dialog(&mut state, &out);
    let rebuilt = [
        "source.rpf",
        "rebuilt archive",
        "the game refuses",
        shown.as_str(),
        "bytes over",
    ];
    assert!(rebuilt.iter().all(|w| title.contains(w)), "{title}");
    assert!(!title.contains("patched copy"), "{title}");
    let refused = ["copy.rpf", "the game refuses"];
    assert!(
        said(&state, ToastType::Success, &refused),
        "{:?}",
        toasts(&state)
    );
    assert_eq!(listed(dir.path()), ["copy.rpf", "source.rpf"]);

    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, tree, _) = edited_tree(dir.path(), vec![b'a'; other().len()]);
    let mut state = showing(path, tree);
    let out = dir.path().join("copy.rpf");

    let title = saved_through_the_dialog(&mut state, &out);
    let patched = [
        "source.rpf",
        "patched copy",
        "byte-identical",
        "has not been run",
    ];
    assert!(patched.iter().all(|w| title.contains(w)), "{title}");
    assert!(!title.contains("refuses"), "{title}");
    let written = ["copy.rpf", "is a patched copy"];
    assert!(
        said(&state, ToastType::Success, &written),
        "{:?}",
        toasts(&state)
    );
    assert_eq!(listed(dir.path()), ["copy.rpf", "source.rpf"]);
}

#[test]
fn a_rename_a_removal_and_an_add_in_the_tree_are_written_as_the_tree_shows_them() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading_with(Arc::clone(&keys));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = three(dir.path());
    let mut tree = tree(&path);
    let session = &mut tree.session;
    let pso = row_holding(session, &table_form_pso());
    let gone = row_holding(session, &other());
    session.rename_node(pso, "renamed.pso").expect("renamed");
    session.delete_node(gone).expect("removed");
    let root = session.root_id();
    let added = b"an added body, which deflates. ".repeat(40);
    let new_row = (session.add_file_copy(root, "added.bin", added.clone())).expect("added");
    let renamed = session.path_of(pso).expect("a path");
    let new_name = session.path_of(new_row).expect("a path");

    let edits = tree_edits(session).expect("the tree's edits");
    let out = dir.path().join("copy.rpf");
    let saved = write_tree(&path, &edits, &out, &keys).expect("the copy is written");
    let Saved::Rebuilt { tables, kept, .. } = saved else {
        panic!("a rename, a removal and an add are never patched in place");
    };
    assert!(tables);
    let named = |what: &str| kept.iter().any(|why| why.ends_with(what));
    assert!(
        named("is renamed") && named("is removed") && named("is added"),
        "{kept:?}"
    );
    let copy = std::fs::read(&out).expect("the copy");
    let names: BTreeSet<u64> = (mount_of(&copy).archive.entries.iter())
        .map(|e| e.name64)
        .collect();
    let expected: BTreeSet<u64> = [renamed.as_str(), new_name.as_str(), RESOURCE]
        .iter()
        .map(|name| name_hash64(name.as_bytes()))
        .collect();
    assert_eq!(names, expected);
    assert_eq!(body_of(&copy, &renamed), table_form_pso());
    assert_eq!(body_of(&copy, &new_name), added);
}

#[test]
fn a_save_of_a_gta_vi_tree_in_place_is_refused_by_name_and_writes_nothing() {
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(synthetic_keys().expect("the synthetic snapshot parses"));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = three(dir.path());
    let before = std::fs::read(&path).expect("the source");
    let mut tree = tree(&path);
    let row = row_holding(&tree.session, &other());
    (tree.session.set_bytes_for(row, b"edited".to_vec())).expect("the row takes the bytes");
    let mut state = showing(path.clone(), tree);
    let refused = ["source.rpf", "never written in place"];

    assert!(!state.save_active_document());
    assert!(
        said(&state, ToastType::Error, &refused),
        "{:?}",
        toasts(&state)
    );
    state.toasts.clear();
    let started = state.save_gta6_tree_with(DOC, &path, synthetic_keys);
    assert_eq!(started, Some(false));
    assert!(
        said(&state, ToastType::Error, &refused),
        "{:?}",
        toasts(&state)
    );
    assert_eq!(std::fs::read(&path).expect("the source"), before);

    // A folder's tree writes none of its GTA VI archives in place either.
    let crypto = Arc::new(NoOpCryptoProvider::new());
    let archive = RpfArchive::open_with(&path, crypto).expect("the archive opens");
    let why = refuse_in_place(&archive).expect_err("refused");
    assert!(refused.iter().all(|w| why.contains(w)), "{why}");
}

/// A small clear row of a real archive, replaced in the tree and saved into a copy, reads back.
#[test]
fn a_small_clear_row_of_a_real_archive_replaced_into_a_copy_reads_back() {
    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    const WHAT: &str = "gta6 tree save of a real archive";
    let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
    let keys_set = std::env::var_os("OPENRAGE_GTA6_KEYS").is_some_and(|p| Path::new(&p).exists());
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
    let _reading = reading_with(Arc::clone(&keys));
    // The smallest archives of the build, since the save copies the whole archive.
    let mut archives: Vec<(u64, PathBuf)> = (std::fs::read_dir(&build).into_iter().flatten())
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .collect();
    archives.sort();
    let replaced = b"OpenRage replaced this body, which deflates. ".repeat(40);
    for (_, path) in archives.iter().take(8) {
        let crypto = Arc::new(NoOpCryptoProvider::new());
        let opened = (RpfArchive::open_with(path, crypto).map_err(|e| e.to_string()))
            .and_then(RpfDocument::new);
        let Ok(mut tree) = opened else {
            continue;
        };
        let Some((row, entry)) = small_clear_row(&tree.session) else {
            continue;
        };
        (tree.session.set_bytes_for(row, replaced.clone())).expect("the row takes the bytes");
        let edits = tree_edits(&tree.session).expect("the tree's edits");
        let dir = tempfile::tempdir().expect("a scratch directory");
        let out = dir.path().join("copy.rpf");
        write_tree(path, &edits, &out, &keys).expect("the copy is written");
        let mut copy = File::open(&out).expect("the copy");
        let mount = Gta6Mount::open_root(&mut copy, &keys).expect("the copy opens");
        let entry = (mount.archive.entries.iter())
            .find(|e| e.index == entry)
            .expect("the entry is kept");
        let body = mount.read_entry(&mut copy, entry).expect("the body reads");
        assert_eq!(body, replaced);
        return;
    }
    let why = format_args!("no small clear row in the smallest archives of {BUILD_ENV}");
    or_test_support::skip_install(WHAT, why);
}

/// A small clear row of a real archive, changed by one byte at its own size, saves as a patched copy
/// whose bytes outside that body are the archive's.
#[test]
fn a_small_clear_row_of_a_real_archive_changed_at_its_size_saves_as_a_patched_copy() {
    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    const WHAT: &str = "gta6 patched tree save of a real archive";
    let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
    let keys_set = std::env::var_os("OPENRAGE_GTA6_KEYS").is_some_and(|p| Path::new(&p).exists());
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
    let _reading = reading_with(Arc::clone(&keys));
    let mut archives: Vec<(u64, PathBuf)> = (std::fs::read_dir(&build).into_iter().flatten())
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .collect();
    archives.sort();
    let mut rebuilt = Vec::new();
    for (_, path) in archives.iter().take(8) {
        let crypto = Arc::new(NoOpCryptoProvider::new());
        let opened = (RpfArchive::open_with(path, crypto).map_err(|e| e.to_string()))
            .and_then(RpfDocument::new);
        let Ok(mut tree) = opened else {
            continue;
        };
        let Some((row, entry)) = small_clear_row(&tree.session) else {
            continue;
        };
        let Some(mut body) = tree.session.bytes_for(row).map(|b| b.to_vec()) else {
            continue;
        };
        let Some(last) = body.last_mut() else {
            continue;
        };
        *last ^= 1;
        (tree.session.set_bytes_for(row, body.clone())).expect("the row takes the bytes");
        let edits = tree_edits(&tree.session).expect("the tree's edits");
        // A deflated chunk OpenRage's encoder cannot fit under its ceiling (nearly every one, as
        // measured) plans a rebuild, which is never written, and another row may fit; a rebuild for
        // any other reason fails. So the run writes at most one copy, the patched one.
        let planned = plan_tree(path, &edits, &keys).expect("the save plans");
        if !planned.patched() {
            if !planned
                .kept
                .iter()
                .all(|why| why.contains("deflates at best"))
            {
                rebuilt.push(planned.kept);
            }
            continue;
        }
        let dir = tempfile::tempdir().expect("a scratch directory");
        let out = dir.path().join("copy.rpf");
        let saved = write_planned(path, &out, &keys, &planned).expect("the copy is written");
        assert_eq!(saved, Saved::Patched);
        assert_eq!(
            listed(dir.path()),
            ["copy.rpf"],
            "one copy and nothing beside it"
        );
        let before = std::fs::read(path).expect("the source");
        let copy = std::fs::read(&out).expect("the copy");
        let mount = Gta6Mount::open_root(&mut Cursor::new(&before), &keys).expect("it opens");
        let held = (mount.archive.entries.iter())
            .find(|e| e.index == entry)
            .expect("the entry");
        let start = mount.absolute_start(held).expect("a start") as usize;
        let end = start + held.stored.expect("a stored extent") as usize;
        assert_eq!(copy.len(), before.len());
        assert_eq!(copy[..start], before[..start], "every byte before the body");
        assert_eq!(copy[end..], before[end..], "every byte after the body");
        let reopened = Gta6Mount::open_root(&mut Cursor::new(&copy), &keys).expect("it opens");
        let held = (reopened.archive.entries.iter())
            .find(|e| e.index == entry)
            .expect("the entry is kept");
        let read = reopened
            .read_entry(Cursor::new(&copy), held)
            .expect("the body reads");
        assert_eq!(read, body);
        assert_eq!(std::fs::read(path).expect("the source"), before);
        return;
    }
    assert!(
        rebuilt.is_empty(),
        "a row changed at its size was rebuilt for another reason than a chunk that does not fit: \
         {rebuilt:?}"
    );
    let why = format_args!(
        "no small clear row of the smallest archives of {BUILD_ENV} whose changed chunk fits"
    );
    or_test_support::skip_install(WHAT, why);
}

/// A row of the top archive of at most 64 KiB, clear and not headerless, and its entry's index.
fn small_clear_row(session: &RpfEditSession) -> Option<(RpfNodeId, u32)> {
    let archive = top_rpf9(session)?;
    files(session).into_iter().find_map(|id| {
        let RpfNodeBacking::ArchiveFile { entry_index, .. } = &session.node(id)?.backing else {
            return None;
        };
        let place = archive.rpf9_place(*entry_index)?;
        let row = archive.binary_entry(*entry_index)?;
        let small = (1..=0x10000).contains(&row.uncompressed_size);
        let plain = place.base == 0 && !place.ciphered && place.offset == row.file_offset;
        (small && plain).then_some((id, place.entry))
    })
}
