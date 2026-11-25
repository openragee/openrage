//! Hermetic checks of an archive opened as its table, on archives laid out here and enciphered under
//! a synthetic white-box key, so no game byte is involved.

use std::path::PathBuf;

use or_io::container::rpf::{
    Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_NAME32,
    GROUP_NAME64,
};
use or_io::gta6::hash::{name_hash32, name_hash64};

use super::*;
use crate::state::DocumentKind;

const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
const KEY_ID: u32 = 0xDEAD_BEEF;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
/// The round tables feeding each output lane of the cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
const WORDS: usize = 60;
const MAPS: usize = 16;
const MAP_LEN: usize = 256;
const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;
const NAMES_HEAP: usize = 0x40;
const GAP: usize = 0x20;

/// The class bytes of a 24-byte row.
const NESTED: u64 = 0x00;
const RESOURCE: u64 = 0x0B;
const MEDIA: u64 = 0xFF;

fn round8(x: usize) -> usize {
    (x + 7) & !7
}

fn round16(x: usize) -> usize {
    (x + 15) & !15
}

fn put16(t: &mut [u8], at: usize, v: u16) {
    t[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(t: &mut [u8], at: usize, v: u32) {
    t[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(t: &mut [u8], at: usize, v: u64) {
    t[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

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

fn synthetic_keys() -> GtaViKeys {
    GtaViKeys::parse(&snapshot()).expect("the synthetic snapshot parses")
}

/// An archive laid out by hand: a header, four descriptors in 0x150 bytes of metadata, the columns
/// row24, name64, name32 and an empty `0xe0c1` in the main region and `0xb6f5` external, each body
/// stored as given `GAP` apart, and a names heap. Each entry is `(path, class byte, body)`.
fn archive(entries: &[(&str, u64, Vec<u8>)]) -> Vec<u8> {
    let n = entries.len();
    let groups_start = 0x1B0;
    let main_index = groups_start + 5 * 24;
    let main_start = round8(main_index + 4 * n);
    let (name64, name32, e0c1) = (24 * n, 32 * n, round8(36 * n));
    let split = round16(main_start + e0c1);
    let ext_start = round8(split + n);
    let toc_end = round16(ext_start + 8 * n);

    let mut t = vec![0u8; toc_end];
    put32(&mut t, 0x00, RPF9_MAGIC);
    put32(&mut t, 0x04, toc_end as u32);
    put32(&mut t, 0x08, split as u32);
    put32(&mut t, 0x0C, NAMES_HEAP as u32);
    for (i, b) in t[0x10..0x18].iter_mut().enumerate() {
        *b = i as u8 + 1;
    }
    t[0x19] = 0x7A;
    t[0x1A] = 4;
    put32(&mut t, 0x1C, 0x150);
    put16(&mut t, 0x20, 5);
    put16(&mut t, 0x22, 1);
    put32(&mut t, 0x24, n as u32);
    put32(&mut t, 0x28, 0xC0FF_EE11);
    t[0x2C] = 1;
    t[0x2D] = 0x0F;
    t[0x2E] = 0x09;
    put32(&mut t, 0x30, 0x10);
    for (i, (kind, off)) in [(1u16, 0u32), (5, 0x20), (6, 0x40), (9, 0x140)]
        .iter()
        .enumerate()
    {
        put16(&mut t, 0x40 + 8 * i, *kind);
        put32(&mut t, 0x44 + 8 * i, *off);
    }
    put32(&mut t, 0x60, KEY_ID);
    put16(&mut t, 0x64, 16);
    t[0x66..0x76].fill(0x33);
    for (i, b) in t[0x80..0x1B0].iter_mut().enumerate() {
        *b = (i * 7 + 1) as u8;
    }
    let directory = [
        (GROUP_ENTRY_ROW, n, 0, 24, 0),
        (GROUP_NAME64, n, name64, 8, 0),
        (GROUP_NAME32, n, name32, 4, 0),
        (GROUP_CHUNK_RECORD, 0, e0c1, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        put64(&mut t, at, hash);
        put32(&mut t, at + 8, count as u32);
        put32(&mut t, at + 12, offset as u32);
        put32(&mut t, at + 16, width | (external << 16));
    }

    let mut at = round16(toc_end + GAP);
    let mut placed = Vec::new();
    for (i, (name, class, body)) in entries.iter().enumerate() {
        let mut stored = body.clone();
        stored.resize(round16(stored.len()), 0);
        let row = main_start + 24 * i;
        put64(&mut t, row, (class << 40) | (at as u64 / 16));
        put64(&mut t, row + 8, stored.len() as u64 / 16);
        put64(&mut t, row + 16, body.len() as u64);
        put64(
            &mut t,
            main_start + name64 + 8 * i,
            name_hash64(name.as_bytes()),
        );
        put32(
            &mut t,
            main_start + name32 + 4 * i,
            name_hash32(name.as_bytes()),
        );
        put64(&mut t, ext_start + 8 * i, 0x1111_0000 + i as u64);
        t[main_index + 4 * i..main_index + 4 * i + 3].fill(i as u8);
        t[main_index + 4 * i + 3] = 0xFF;
        t[split + i] = i as u8;
        let len = stored.len();
        placed.push((at, stored));
        at += len + GAP;
    }

    let toc = Rpf9Toc {
        bytes: t,
        key_id: KEY_ID,
        registry_id: REGISTRY_ID,
        entry_count: n as u32,
        groups: Vec::new(),
    };
    let head = Rpf9Builder::serialize(&toc, &synthetic_keys()).expect("the table enciphers");
    let mut file = vec![0x5Au8; at + NAMES_HEAP];
    file[..head.len()].copy_from_slice(&head);
    for (start, stored) in placed {
        file[start..start + stored.len()].copy_from_slice(&stored);
    }
    file
}

const ALPHA: &str = "common/data/alpha.meta";
const UNNAMED: &str = "models/unnamed.ydr";
const NESTED_NAME: &str = "levels/inner.rpf";
const DEEP: &str = "inner/deep.xml";

/// A top-level archive of a named media body, a resource no name resolves and a nested archive of
/// one body, written to a scratch folder.
fn sample() -> (tempfile::TempDir, PathBuf) {
    let nested = archive(&[(DEEP, MEDIA, b"<deep/>".to_vec())]);
    let top = archive(&[
        (ALPHA, MEDIA, b"alpha body, stored raw".to_vec()),
        (UNNAMED, RESOURCE, vec![0x47; 40]),
        (NESTED_NAME, NESTED, nested),
    ]);
    let dir = tempfile::tempdir().expect("scratch");
    let path = dir.path().join("top.rpf");
    std::fs::write(&path, top).expect("the archive is written");
    (dir, path)
}

fn names() -> NameResolver {
    let mut resolver = NameResolver::new();
    for name in [ALPHA, NESTED_NAME, DEEP] {
        resolver.insert(name_hash64(name.as_bytes()), name);
    }
    resolver
}

/// The sample archive's table document, listed with the synthetic key.
pub(crate) fn sample_document() -> (tempfile::TempDir, Rpf9TableDocument) {
    let (dir, path) = sample();
    let header = header_of(&path).expect("the sample opens with an RPF9 header");
    let keys = synthetic_keys();
    let doc = Rpf9TableDocument::open_with(&path, header, Ok(&keys), &names());
    let Ok(doc) = doc else {
        panic!("the synthetic key lists the sample's table");
    };
    (dir, doc)
}

/// A state whose tab in front is `doc`'s archive.
pub(crate) fn state_showing(doc: Rpf9TableDocument) -> (AppState, DocumentId) {
    let mut state = AppState::default();
    let id = state.new_empty_document(DocumentKind::Text);
    let d = state.document_mut(id).expect("the blank document");
    d.path = Some(doc.path.clone());
    d.kind = DocumentKind::Rpf9;
    d.payload = DocPayload::Rpf9Table(Arc::new(doc));
    state.active_document = Some(id);
    (state, id)
}

#[test]
fn an_archive_a_key_opens_is_its_table_and_no_scan_is_made() {
    let (_dir, doc) = sample_document();
    let path = doc.path.clone();
    let Ok(Some(DocPayload::Rpf9Table(table))) = payload_of(&path, Some(Ok(doc))) else {
        panic!("the table is the payload, and no scan document is made");
    };
    let tree = table.tree();
    let root = tree.root.as_ref().expect("the root is mounted at the open");
    let shown: Vec<String> = root.rows.iter().map(TableRow::display_name).collect();
    assert_eq!(shown.len(), 3);
    assert_eq!(shown[0], ALPHA);
    assert!(shown[1].starts_with("hash_"), "{}", shown[1]);
    assert_eq!(shown[2], NESTED_NAME);
    assert!(root.rows[2].is_archive);
    assert!(
        root.children.is_empty(),
        "a nested archive waits to be expanded"
    );
    drop(tree);
    assert!(table.held_bytes() > 0);
}

#[test]
fn without_a_key_the_archive_opens_as_its_scan_and_says_so() {
    let (_dir, path) = sample();
    let header = header_of(&path).expect("an RPF9 head");
    let no_key = Err("no GTA VI key snapshot was found".to_string());
    let table = Rpf9TableDocument::open_with(&path, header, no_key, &names());
    let Err(why) = &table else {
        panic!("no key, no table");
    };
    assert!(
        why.contains("top.rpf") && why.contains("scan without its table"),
        "{why}"
    );
    assert!(
        matches!(
            payload_of(&path, Some(table)),
            Ok(Some(DocPayload::Rpf9(_)))
        ),
        "the scan lists the archive no key opens"
    );

    let other = path.with_file_name("other.rpf");
    std::fs::write(&other, b"RPF7, and no GTA VI archive").expect("written");
    assert!(header_of(&other).is_none());
    assert!(
        matches!(payload_of(&other, None), Ok(None)),
        "a file that is no GTA VI archive is neither"
    );
}

#[test]
fn the_files_list_of_an_archive_and_of_a_body_of_it_is_its_table() {
    let (_dir, doc) = sample_document();
    let archive = doc.path.clone();
    let (mut state, id) = state_showing(doc);
    assert_eq!(state.table_listing_the_active_tab(), Some(id));
    assert_eq!(
        state.scan_listing_the_active_tab(),
        None,
        "no scan lists it"
    );

    let body = state.new_empty_document(DocumentKind::Text);
    state.document_mut(body).expect("the body").path = Some(body_path(&archive, 0x40, View::Bytes));
    state.active_document = Some(body);
    assert_eq!(state.table_listing_the_active_tab(), Some(id));
}

#[test]
fn a_row_opens_its_body_by_the_path_a_scan_names_or_says_it_has_no_start() {
    let (_dir, doc) = sample_document();
    let archive = doc.path.clone();
    let row = doc.tree().root.as_ref().expect("the root").rows[0].clone();
    let start = row.absolute_start.expect("the row states a start");
    let mut state = AppState::default();
    state.open_table_entry(&archive, 0, &row, true);
    let loading = (state.load_receiver.as_ref().map(|(path, _)| path.clone()))
        .or_else(|| state.pending_loads.back().cloned());
    assert_eq!(loading, Some(body_path(&archive, start, View::Bytes)));

    let unplaced = TableRow {
        absolute_start: None,
        ..row
    };
    state.open_table_entry(&archive, 0, &unplaced, true);
    assert!(
        (state.toasts.iter()).any(|t| t.message.contains("states no start")),
        "no toast said the row has no start"
    );
}
