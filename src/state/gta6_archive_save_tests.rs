//! Hermetic checks of saving into a copy of a GTA VI archive, on archives built here and enciphered
//! under a synthetic white-box key, so no game byte is involved.

use std::io::Cursor;
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Form, Rpf9Sections};
use or_io::gta6::hash::name_hash64;
use or_io::meta::Value;

use super::*;
use crate::state::opening_an_rpf9_archive::{headerless_rbm, said};
use crate::state::rpf9_scan::{body_path, View};
use crate::state::{MetaDocument, ToastType};

const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
const KEY_ID: u32 = 0xDEAD_BEEF;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
/// The round tables feeding each output lane of the cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
const WORDS: usize = 60;
const MAPS: usize = 16;
const MAP_LEN: usize = 256;
const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;

const PSO: &str = "data/table.pso";
const RESOURCE: &str = "data/headerless.ymt";

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

fn synthetic_keys() -> Result<Arc<GtaViKeys>, String> {
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
fn table_form_pso() -> Vec<u8> {
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
fn rbm_section() -> Vec<u8> {
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

/// An archive of the PSO body under a 24-byte row and the RBM resource under a 20-byte row, built
/// from nothing in `dir`.
fn source(dir: &Path) -> PathBuf {
    let entries = vec![
        (PSO.to_owned(), Rpf9NewBody::Chunked(table_form_pso())),
        (
            RESOURCE.to_owned(),
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: loose_rbm(),
            },
        ),
    ];
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
