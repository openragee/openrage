//! Hermetic checks of why an archive is listed by its scan, on archives laid out here and
//! enciphered under a synthetic white-box key, so no game byte is involved.

use std::time::{Duration, Instant};

use or_io::container::rpf::{
    Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_NAME32,
    GROUP_NAME64,
};
use or_io::gta6::hash::{name_hash32, name_hash64};

use super::*;
use crate::state::{AppState, ToastType};

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

#[test]
fn an_archive_whose_table_opens_with_the_key_held_has_nothing_to_say() {
    let (_dir, path) = sample();
    assert_eq!(table_opens(&path, Ok(&synthetic_keys())), Ok(()));
}

#[test]
fn without_a_key_the_open_falls_back_to_the_scan_and_says_so() {
    let (_dir, path) = sample();
    let why = table_opens(&path, Err("no GTA VI key snapshot was found".into()))
        .expect_err("no key, no table");
    assert!(
        why.contains("top.rpf") && why.contains("scan without its table"),
        "{why}"
    );

    // A key that is not the archive's leaves its table closed too: noise is refused, never listed.
    // Here the bootstrap IV (bytes 48..64) differs in the bit that makes the first descriptor type 1.
    let mut other = snapshot();
    other[48] ^= 0x01;
    let other = GtaViKeys::parse(&other).expect("the altered snapshot parses");
    let why = table_opens(&path, Ok(&other)).expect_err("a key not the archive's");
    assert!(why.contains("scan without its table"), "{why}");
}

#[test]
fn the_open_of_an_archive_no_held_key_opens_is_a_scan_and_a_toast_says_why() {
    let (_dir, path) = sample();
    let mut state = AppState::default();
    state.load_file(path.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() || !state.pending_loads.is_empty() {
        state.poll_load_result();
        assert!(Instant::now() < deadline, "the load never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let opened = (state.documents.iter()).find(|d| d.path.as_deref() == Some(path.as_path()));
    assert!(
        matches!(opened.map(|d| &d.payload), Some(DocPayload::Rpf9(_))),
        "the archive opens as its scan"
    );
    // Whatever snapshot this machine holds is not the synthetic key, so the table stays closed.
    state.poll_jobs();
    let said = (state.toasts.iter()).any(|t| {
        t.toast_type == ToastType::Info
            && t.message.contains("top.rpf")
            && t.message.contains("scan without its table")
    });
    assert!(said, "no toast said the scan lists the archive");
}
