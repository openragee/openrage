//! Hermetic checks of the RPF9 container writer on archives laid out by hand here (not by the writer)
//! and enciphered under a synthetic white-box key, so no game byte is involved.

use std::io::Cursor;

use super::*;
use crate::container::rpf::gta6_archive::GROUP_CONTENT_HASH;
use crate::container::rpf::gta6_chunk_table::entry_chunks;
use crate::container::rpf::gta6_dependencies::{self, GROUP_DEPENDENCIES};
use crate::container::rpf::rpf9::RPF9_MAGIC;

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
const GAP: u64 = 0x20;
const HEADER_28: u32 = 0xC0FF_EE11;
const HASH_BASE: u64 = 0x1111_0000;

fn push_perm(out: &mut Vec<u8>, k: usize) {
    for x in 0..256usize {
        out.push(((x + k * 7 + 1) % 256) as u8);
    }
}

fn mix8(b: u8) -> u8 {
    let mut y = 0u8;
    let mut acc = 0u8;
    for i in (0..8).rev() {
        acc ^= (b >> i) & 1;
        y |= acc << i;
    }
    y
}

/// A clean synthetic white-box snapshot holding the bootstrap key and one archive key: byte
/// permutations for the maps and invertible affine round tables, the shape the encrypt side inverts.
fn clean_snapshot() -> Vec<u8> {
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

fn keys() -> GtaViKeys {
    GtaViKeys::parse(&clean_snapshot()).expect("a clean synthetic snapshot parses")
}

enum Body {
    Raw(Vec<u8>),
    Chunked(Vec<u8>),
    Nested(Vec<u8>),
}

/// An archive laid out by hand: a header, four descriptors (an index key, then stand-ins for types
/// 5, 6 and 9) in 0x150 bytes of metadata, the columns row24, name64, name32 and `0xe0c1` in the main
/// region and `0xb6f5` external, bodies `GAP` apart, and a names heap of 0x5A bytes.
fn archive(entries: &[(&str, Body)]) -> Vec<u8> {
    archive_in(entries, ROW24_CHUNK_MODE)
}

/// As [`archive`], its chunk tables in `mode`.
fn archive_in(entries: &[(&str, Body)], mode: u8) -> Vec<u8> {
    let n = entries.len();
    let mut stored = Vec::new();
    let mut slices = Vec::new();
    let mut chunk_slot = Vec::new();
    for (_, body) in entries {
        chunk_slot.push(0xFFu8);
        match body {
            Body::Raw(b) => {
                let mut s = b.clone();
                s.resize(round16(s.len() as u64) as usize, 0);
                stored.push((0xFFu64, s, b.len() as u64));
            }
            Body::Chunked(b) => {
                let (s, chunks) = chunk_body(b).unwrap();
                *chunk_slot.last_mut().unwrap() = slices.len() as u8;
                slices.push(author(&ChunkTable { mode, chunks }).unwrap());
                stored.push((0x0B, s, b.len() as u64));
            }
            Body::Nested(b) => stored.push((0x00, b.clone(), b.len() as u64)),
        }
    }
    let groups_start = 0x1B0;
    let main_index = groups_start + 5 * 24;
    let main_start = round8(main_index + 4 * n);
    let (name64, name32, e0c1) = (24 * n, 32 * n, round8(36 * n));
    let heap = round16((main_start + e0c1 + 8 * slices.len()) as u64) as usize;
    let heap_len: usize = slices.iter().map(|(_, w)| 8 * w.len()).sum();
    let split = round16((heap + heap_len) as u64) as usize;
    let ext_start = round8(split + n);
    let toc_end = round16((ext_start + 8 * n) as u64) as usize;

    let mut t = head(toc_end, split, n, 5, 5);
    let directory = [
        (GROUP_ENTRY_ROW, n, 0, 24, 0),
        (GROUP_NAME64, n, name64, 8, 0),
        (GROUP_NAME32, n, name32, 4, 0),
        (GROUP_CHUNK_RECORD, slices.len(), e0c1, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        LE::write_u64(&mut t[at..], hash);
        LE::write_u32(&mut t[at + 8..], count as u32);
        LE::write_u32(&mut t[at + 12..], offset as u32);
        LE::write_u32(&mut t[at + 16..], width | (external << 16));
    }

    let mut at = round16(toc_end as u64 + GAP);
    let mut starts = Vec::new();
    for (i, ((name, _), (class, s, real))) in entries.iter().zip(&stored).enumerate() {
        let row = main_start + 24 * i;
        LE::write_u64(&mut t[row..], (class << 40) | (at / 16));
        LE::write_u64(&mut t[row + 8..], s.len() as u64 / 16);
        LE::write_u64(&mut t[row + 16..], *real);
        LE::write_u64(
            &mut t[main_start + name64 + 8 * i..],
            name_hash64(name.as_bytes()),
        );
        LE::write_u32(
            &mut t[main_start + name32 + 4 * i..],
            name_hash32(name.as_bytes()),
        );
        LE::write_u64(&mut t[ext_start + 8 * i..], HASH_BASE + i as u64);
        t[main_index + 4 * i..main_index + 4 * i + 3].fill(i as u8);
        t[main_index + 4 * i + 3] = chunk_slot[i];
        t[split + i] = i as u8;
        starts.push(at);
        at += s.len() as u64 + GAP;
    }
    let mut h = heap;
    for (k, (u1, words)) in slices.iter().enumerate() {
        let cell = main_start + e0c1 + 8 * k;
        LE::write_u32(&mut t[cell..], (h - cell) as u32);
        LE::write_u32(&mut t[cell + 4..], *u1);
        for w in words {
            LE::write_u64(&mut t[h..], *w);
            h += 8;
        }
    }

    let bodies: Vec<(u64, &[u8])> = starts
        .iter()
        .zip(&stored)
        .map(|(start, (_, s, _))| (*start, &s[..]))
        .collect();
    seal(t, n, &bodies, at)
}

/// A table's first 0x1B0 bytes as every fixture lays them: the header, then four descriptors (an
/// index key, then `second` and stand-ins for types 6 and 9) in 0x150 bytes of metadata.
fn head(toc_end: usize, split: usize, n: usize, groups: u16, second: u16) -> Vec<u8> {
    let mut t = vec![0u8; toc_end];
    LE::write_u32(&mut t[0x00..], RPF9_MAGIC);
    LE::write_u32(&mut t[0x04..], toc_end as u32);
    LE::write_u32(&mut t[0x08..], split as u32);
    LE::write_u32(&mut t[0x0C..], NAMES_HEAP as u32);
    for (i, b) in t[0x10..0x18].iter_mut().enumerate() {
        *b = i as u8 + 1;
    }
    t[0x19] = 0x7A;
    t[0x1A] = 4;
    LE::write_u32(&mut t[0x1C..], 0x150);
    LE::write_u16(&mut t[0x20..], groups);
    LE::write_u16(&mut t[0x22..], 1);
    LE::write_u32(&mut t[0x24..], n as u32);
    LE::write_u32(&mut t[0x28..], HEADER_28);
    t[0x2C] = 1;
    t[0x2D] = 0x0F;
    t[0x2E] = 0x09;
    LE::write_u32(&mut t[0x30..], 0x10);
    for (i, (kind, off)) in [(1u16, 0u32), (second, 0x20), (6, 0x40), (9, 0x140)]
        .iter()
        .enumerate()
    {
        LE::write_u16(&mut t[0x40 + 8 * i..], *kind);
        LE::write_u32(&mut t[0x44 + 8 * i..], *off);
    }
    LE::write_u32(&mut t[0x60..], KEY_ID);
    LE::write_u16(&mut t[0x64..], 16);
    t[0x66..0x76].fill(0x33);
    for (i, b) in t[0x80..0x1B0].iter_mut().enumerate() {
        *b = (i * 7 + 1) as u8;
    }
    t
}

/// The archive file of plaintext table `t`: enciphered, then each body at its start, then the
/// names heap at `names_at`.
fn seal(t: Vec<u8>, n: usize, bodies: &[(u64, &[u8])], names_at: u64) -> Vec<u8> {
    let toc = Rpf9Toc {
        bytes: t,
        key_id: KEY_ID,
        registry_id: REGISTRY_ID,
        entry_count: n as u32,
        groups: Vec::new(),
    };
    let head = Rpf9Builder::serialize(&toc, &keys()).expect("the synthetic table enciphers");
    let names_at = names_at as usize;
    let mut file = vec![0u8; names_at + NAMES_HEAP];
    file[..head.len()].copy_from_slice(&head);
    for (start, s) in bodies {
        file[*start as usize..*start as usize + s.len()].copy_from_slice(s);
    }
    file[names_at..].fill(0x5A);
    file
}

fn text(seed: &str, len: usize) -> Vec<u8> {
    seed.bytes().cycle().take(len).collect()
}

fn alpha() -> Vec<u8> {
    b"alpha body, stored raw".to_vec()
}

fn beta() -> Vec<u8> {
    text("beta chunked text ", 150_000)
}

fn sample() -> Vec<u8> {
    archive(&[
        ("alpha.bin", Body::Raw(alpha())),
        ("beta.txt", Body::Chunked(beta())),
        ("gamma.bin", Body::Raw(vec![0x47; 40])),
    ])
}

fn write(file: &[u8], edit: impl FnOnce(&mut Rpf9ArchiveWriter)) -> Result<(Rpf9Plan, Vec<u8>)> {
    let mut writer = Rpf9ArchiveWriter::new();
    edit(&mut writer);
    let mut src = Cursor::new(file);
    let plan = writer.plan(&mut src, &keys())?;
    let mut out = Vec::new();
    plan.write_to(&mut src, &mut out)?;
    assert_eq!(out.len() as u64, plan.size());
    Ok((plan, out))
}

fn open(file: &[u8]) -> Gta6Mount {
    Gta6Mount::open_root(&mut Cursor::new(file), &keys()).expect("the archive reopens")
}

/// An entry's raw body, `[start, start + real)`.
fn raw(file: &[u8], mount: &Gta6Mount, e: usize) -> Vec<u8> {
    let entry = &mount.archive.entries[e];
    let start = mount.absolute_start(entry).unwrap() as usize;
    file[start..start + entry.real.unwrap() as usize].to_vec()
}

fn chunked(file: &[u8], mount: &Gta6Mount, e: usize) -> Vec<u8> {
    mount
        .read_entry(Cursor::new(file), &mount.archive.entries[e])
        .expect("the chunked body reads")
}

#[test]
fn the_fixture_reads_back_and_lays_out_as_the_writer_does() {
    let file = sample();
    let mount = open(&file);
    assert_eq!(mount.archive.entries.len(), 3);
    assert_eq!(raw(&file, &mount, 0), alpha());
    assert_eq!(chunked(&file, &mount, 1), beta());
    let table = Table::from_toc(&mount.archive.toc).unwrap();
    assert!(
        table.faithful,
        "the hand layout and the writer's layout agree"
    );
    assert_eq!(table.patch(), mount.archive.toc.bytes);
}

#[test]
fn a_copy_is_byte_identical_through_decrypt_and_serialize() {
    let file = sample();
    let (plan, out) = write(&file, |_| {}).unwrap();
    assert_eq!(out, file);
    assert_eq!(plan.tables_changed, 0);
    assert_eq!(plan.pieces.len(), 2);
}

#[test]
fn remove_lists_one_fewer_and_keeps_the_rest() {
    let file = sample();
    let before = open(&file);
    let (plan, out) = write(&file, |w| {
        w.remove(&[0]);
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 1);
    let after = open(&out);
    assert_eq!(after.archive.entries.len(), 2);
    for (kept, was) in after
        .archive
        .entries
        .iter()
        .zip(&before.archive.entries[1..])
    {
        assert_eq!((kept.name64, kept.name32), (was.name64, was.name32));
    }
    assert_eq!(chunked(&out, &after, 0), beta());
    assert_eq!(raw(&out, &after, 1), vec![0x47; 40]);
    let extras = after.archive.entry_extras();
    assert_eq!(extras[0].content_hash, Some(HASH_BASE + 1));
    assert_eq!(extras[0].chunk_count, Some(3));

    // The header's carried words and the metadata (signature and type-9 stand-ins) are as read.
    let (old, new) = (&before.archive.toc.bytes, &after.archive.toc.bytes);
    assert_eq!(old[0x10..0x18], new[0x10..0x18]);
    assert_eq!(old[0x28..0x2E], new[0x28..0x2E]);
    assert_eq!(old[0x40..0x1B0], new[0x40..0x1B0]);

    // The edited table re-enciphers to ciphertext that deciphers to it exactly.
    let mut table = Table::from_toc(&before.archive.toc).unwrap();
    table.remove(0);
    let plain = table.emit().unwrap();
    let edited = Rpf9Toc {
        bytes: plain.clone(),
        ..before.archive.toc.clone()
    };
    let cipher = Rpf9Builder::serialize(&edited, &keys()).unwrap();
    assert_eq!(Rpf9Toc::decrypt(&cipher, &keys()).unwrap().bytes, plain);
}

#[test]
fn rename_changes_both_name_columns_together() {
    let file = sample();
    let name = "renamed/gamma.bin";
    let (_, out) = write(&file, |w| {
        w.rename(&[2], name);
    })
    .unwrap();
    let (before, after) = (open(&file), open(&out));
    let renamed = &after.archive.entries[2];
    assert_eq!(renamed.name64, name_hash64(name.as_bytes()));
    assert_eq!(renamed.name32, Some(name_hash32(name.as_bytes())));
    assert_eq!(renamed.start, before.archive.entries[2].start);
    assert_eq!(after.archive.entries[..2], before.archive.entries[..2]);
    assert!(write(&file, |w| {
        w.rename(&[2], "alpha.bin");
    })
    .is_err());
}

#[test]
fn defragment_is_idempotent_and_preserves_every_body() {
    let file = sample();
    let (_, once) = write(&file, |w| {
        w.defragment();
    })
    .unwrap();
    let (plan, twice) = write(&once, |w| {
        w.defragment();
    })
    .unwrap();
    assert_eq!(twice, once);
    assert_eq!(plan.tables_changed, 0);
    assert!(once.len() < file.len());

    let (before, after) = (open(&file), open(&once));
    let mut next = before.archive.entries[0].start.unwrap();
    for (was, now) in before.archive.entries.iter().zip(&after.archive.entries) {
        let (s0, s1, len) = (
            was.start.unwrap() as usize,
            now.start.unwrap() as usize,
            was.stored.unwrap() as usize,
        );
        assert_eq!(now.start, Some(next), "bodies packed in order");
        assert_eq!(once[s1..s1 + len], file[s0..s0 + len]);
        assert_eq!(
            (now.name64, now.stored, now.real),
            (was.name64, was.stored, was.real)
        );
        next += len as u64;
    }
    assert_eq!(once[once.len() - NAMES_HEAP..], [0x5A; NAMES_HEAP]);
}

#[test]
fn replace_then_reopen_reads_the_new_bytes() {
    let file = sample();
    let new_beta = text("other text ", 70_000);
    let (plan, out) = write(&file, |w| {
        w.replace(&[1], new_beta.clone());
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 1);
    assert_eq!(plan.bodies_changed, 1);
    let mount = open(&out);
    assert_eq!(raw(&out, &mount, 0), alpha());
    assert_eq!(chunked(&out, &mount, 1), new_beta);
    assert_eq!(raw(&out, &mount, 2), vec![0x47; 40]);
    let extras = mount.archive.entry_extras();
    assert_eq!(
        extras[1].content_hash,
        Some(0),
        "a changed entry's hash is zero"
    );
    assert_eq!(
        extras[2].content_hash,
        Some(HASH_BASE + 2),
        "an unchanged one keeps its own"
    );
    assert_eq!(extras[1].chunk_count, Some(2));
    assert_eq!(out[out.len() - NAMES_HEAP..], [0x5A; NAMES_HEAP]);

    // A body too long for its old extent moves, and still reads back through the reader.
    let long = text("a much longer body ", 600_000);
    let (_, out) = write(&file, |w| {
        w.replace(&[1], long.clone());
    })
    .unwrap();
    let mount = open(&out);
    assert!(out.len() > file.len());
    assert_eq!(chunked(&out, &mount, 1), long);
}

#[test]
fn a_body_the_reader_would_not_read_back_is_refused_and_a_stored_chunk_reads_by_its_table() {
    let file = sample();
    // Plain text stored raw is no body the reader detects, under a row with no chunk table.
    assert!(write(&file, |w| {
        w.replace(&[0], text("a longer alpha ", 300));
    })
    .is_err());
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let noise: Vec<u8> = (0..70_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    // A chunk that does not compress is stored, and the row's chunk table reads it back.
    let (_, out) = write(&file, |w| {
        w.replace(&[1], noise.clone());
    })
    .expect("a stored chunk is written");
    assert_eq!(chunked(&out, &open(&out), 1), noise);
}

#[test]
fn replace_by_itself_is_byte_identical() {
    let file = sample();
    let (plan, out) = write(&file, |w| {
        w.replace(&[0], alpha()).replace(&[1], beta());
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 0);
    assert_eq!(out, file);
}

#[test]
fn an_edit_inside_a_nested_archive_rewrites_the_child_then_the_parents_row() {
    let parent = archive(&[
        ("child.rpf", Body::Nested(sample())),
        ("top.bin", Body::Raw(b"top".to_vec())),
    ]);
    let new = text("nested replacement ", 90_000);
    let (plan, out) = write(&parent, |w| {
        w.replace(&[0, 1], new.clone());
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 2);
    let mount = open(&out);
    let row = &mount.archive.entries[0];
    let child = mount
        .open_nested(&mut Cursor::new(&out[..]), row, &keys())
        .expect("the nested archive reopens");
    assert_eq!(chunked(&out, &child, 1), new);
    assert_eq!(raw(&out, &child, 0), alpha());
    let child_len = child.archive.entries[2].start.unwrap()
        + child.archive.entries[2].stored.unwrap()
        + GAP
        + NAMES_HEAP as u64;
    assert_eq!((row.stored, row.real), (Some(child_len), Some(child_len)));
    let extras = mount.archive.entry_extras();
    assert_eq!(extras[0].content_hash, Some(0));
    assert_eq!(extras[1].content_hash, Some(HASH_BASE + 1));
    assert_eq!(raw(&out, &mount, 1), b"top".to_vec());
}

#[test]
fn what_cannot_be_written_is_refused_by_name() {
    let file = sample();
    let parent = archive(&[
        ("child.rpf", Body::Nested(sample())),
        ("top.bin", Body::Raw(b"top".to_vec())),
    ]);
    let refused = |file: &[u8], edit: &dyn Fn(&mut Rpf9ArchiveWriter)| {
        matches!(write(file, edit), Err(IoError::Invalid { .. }))
    };
    assert!(refused(&file, &|w| {
        w.remove(&[9]);
    }));
    assert!(refused(&file, &|w| {
        w.remove(&[0]).remove(&[1]).remove(&[2]);
    }));
    assert!(refused(&file, &|w| {
        w.replace(&[0, 1], Vec::new());
    }));
    assert!(refused(&parent, &|w| {
        w.replace(&[0], b"not an archive".to_vec());
    }));

    // A table the writer cannot lay out again takes no edit that drops a row.
    let mut table = Table::from_toc(&open(&file).archive.toc).unwrap();
    table.faithful = false;
    table.remove(0);
    assert!(matches!(table.emit(), Err(IoError::Invalid { .. })));
}

/// The keys of the heaped fixture's type-4 dependency table, ascending.
const KEYS: [u64; 3] = [
    0x0100_0000_0000_00A1,
    0x0200_0000_0000_00B2,
    0x0300_0000_0000_00C3,
];
/// Where the heaped fixture's metadata starts, past its five descriptors, and its length.
const HEAPED_METADATA: usize = 0x70;
const HEAPED_METADATA_LEN: usize = 0x170;
/// Where its type-4 table sits, metadata offset 0x140.
const KEYS_AT: usize = HEAPED_METADATA + 0x140;
/// Where its type-9 payload sits, past the type-4 table, and its bytes: a count of two and two
/// bytes, padded to 8, then the metadata's padding to 16.
const TYPE9_AT: usize = KEYS_AT + 8 + 8 * KEYS.len();
const TYPE9: [u8; 16] = [2, 2, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// A headerless resource of the heaped fixture: its sections, its dependency list as indices into
/// [`KEYS`], and its `0xd07a` floats.
struct Res {
    name: &'static str,
    type_code: u32,
    v: Vec<u8>,
    p: Vec<u8>,
    deps: &'static [u8],
    floats: Option<[f32; 4]>,
}

/// A virtual section of `v_len` bytes with its block map at 0x40, and a physical one of `p_len`.
fn sections(v_len: usize, p_len: usize, seed: usize) -> (Vec<u8>, Vec<u8>) {
    let mut v: Vec<u8> = (0..v_len)
        .map(|i| ((i * 13 + seed) % 251) as u8 | 0x80)
        .collect();
    v[..8].fill(0);
    v[8..16].copy_from_slice(&0x5000_0040u64.to_le_bytes());
    v[0x40..0x70].fill(0);
    let p = (0..p_len)
        .map(|i| ((i * 7 + seed) % 253) as u8 | 1)
        .collect();
    (v, p)
}

/// The loose file `write_rpf9_resource` writes for `v` and `p`.
fn loose(v: &[u8], p: &[u8]) -> Vec<u8> {
    write_rpf9_resource_chunks(v, p, STREAM_CAP).unwrap().bytes
}

fn resources() -> Vec<Res> {
    let (v0, p0) = sections(0x1_0000 + 0x2F0, 0x340, 1);
    let (v1, p1) = sections(0x200, 0, 2);
    let (v2, p2) = sections(0x480, 0x1_0010, 3);
    vec![
        Res {
            name: "maps/a.ydr",
            type_code: 0x02,
            v: v0,
            p: p0,
            deps: &[0, 2],
            floats: Some([1.0, 2.0, 3.0, 4.0]),
        },
        Res {
            name: "maps/b.ytd",
            type_code: 0x04,
            v: v1,
            p: p1,
            deps: &[1],
            floats: None,
        },
        Res {
            name: "maps/c.ybn",
            type_code: 0x06,
            v: v2,
            p: p2,
            deps: &[],
            floats: Some([5.0, 6.0, 7.0, 8.0]),
        },
    ]
}

/// A nested archive of headerless resources laid out by hand: a type-4 key table opening the
/// metadata's index-keyed tail and a type-9 payload of two bytes ending it, the main columns name64,
/// name32, row20, `0x97c2`, `0xd07a` and `0xe0c1`, then from a 16-byte boundary their three heaps
/// (the lists packed, the floats 4-aligned, the descriptors 8-aligned), and `0xb6f5` external.
fn heaped(res: &[Res]) -> Vec<u8> {
    heaped_in(res, ROW20_CHUNK_MODE)
}

/// As [`heaped`], its chunk tables in `mode`.
fn heaped_in(res: &[Res], mode: u8) -> Vec<u8> {
    let n = res.len();
    let written: Vec<_> = res
        .iter()
        .map(|r| write_rpf9_resource_chunks(&r.v, &r.p, STREAM_CAP).unwrap())
        .collect();
    let lists: Vec<(usize, &[u8])> = (res.iter().enumerate())
        .filter(|(_, r)| !r.deps.is_empty())
        .map(|(i, r)| (i, r.deps))
        .collect();
    let floats: Vec<(usize, [f32; 4])> = (res.iter().enumerate())
        .filter_map(|(i, r)| r.floats.map(|f| (i, f)))
        .collect();
    let tables: Vec<(u32, Vec<u64>)> = written
        .iter()
        .map(|w| {
            author(&ChunkTable {
                mode,
                chunks: w.chunks.clone(),
            })
            .unwrap()
        })
        .collect();

    let groups_start = HEAPED_METADATA + HEAPED_METADATA_LEN;
    let main_index = groups_start + 7 * 24;
    let main_start = round8(main_index + 6 * n);
    let o32 = round8(8 * n);
    let o20 = round8(o32 + 4 * n);
    let o97 = round8(o20 + 20 * n);
    let od0 = o97 + 8 * lists.len();
    let oe0 = od0 + 8 * floats.len();
    let h97 = round16((main_start + oe0 + 8 * n) as u64) as usize;
    let hd0 = (h97 + lists.iter().map(|l| l.1.len()).sum::<usize>()).next_multiple_of(4);
    let he0 = (hd0 + 16 * floats.len()).next_multiple_of(8);
    let e0_len: usize = tables.iter().map(|t| 8 * t.1.len()).sum();
    let split = round16((he0 + e0_len) as u64) as usize;
    let ext_start = round8(split + n);
    let toc_end = round16((ext_start + 8 * n) as u64) as usize;

    // Five descriptors, as a measured table with a type-4 table holds them: that table opens the
    // index-keyed tail at metadata offset 0x140, and the type-9 payload follows it.
    let mut t = head(toc_end, split, n, 7, 5);
    t[0x1A] = 5;
    LE::write_u32(&mut t[0x1C..], HEAPED_METADATA_LEN as u32);
    LE::write_u32(&mut t[0x30..], (HEAPED_METADATA_LEN - 0x140) as u32);
    t[0x40..HEAPED_METADATA_LEN + HEAPED_METADATA].fill(0);
    let type9 = TYPE9_AT - HEAPED_METADATA;
    for (i, (kind, off)) in [(1u16, 0), (5, 0x20), (6, 0x40), (4, 0x140), (9, type9)]
        .iter()
        .enumerate()
    {
        LE::write_u16(&mut t[0x40 + 8 * i..], *kind);
        LE::write_u32(&mut t[0x44 + 8 * i..], *off as u32);
    }
    let m = HEAPED_METADATA;
    LE::write_u32(&mut t[m..], KEY_ID);
    LE::write_u16(&mut t[m + 4..], 16);
    t[m + 6..m + 0x16].fill(0x33);
    for (i, b) in t[m + 0x20..m + 0x140].iter_mut().enumerate() {
        *b = (i * 7 + 1) as u8;
    }
    t[TYPE9_AT..TYPE9_AT + 16].copy_from_slice(&TYPE9);
    LE::write_u32(&mut t[KEYS_AT..], KEYS.len() as u32);
    t[KEYS_AT + 4..KEYS_AT + 8].copy_from_slice(&[1, 0, 0, 0]);
    for (i, key) in KEYS.iter().enumerate() {
        LE::write_u64(&mut t[KEYS_AT + 8 + 8 * i..], *key);
    }
    let directory = [
        (GROUP_NAME64, n, 0, 8, 0),
        (GROUP_NAME32, n, o32, 4, 0),
        (GROUP_ENTRY_ROW_20, n, o20, 20, 0),
        (GROUP_DEPENDENCIES, lists.len(), o97, 8, 0),
        (table::HEAP_D07A, floats.len(), od0, 8, 0),
        (GROUP_CHUNK_RECORD, n, oe0, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        LE::write_u64(&mut t[at..], hash);
        LE::write_u32(&mut t[at + 8..], count as u32);
        LE::write_u32(&mut t[at + 12..], offset as u32);
        LE::write_u32(&mut t[at + 16..], width | (external << 16));
    }

    let mut at = round16(toc_end as u64 + GAP);
    let mut bodies = Vec::new();
    let mut eh = he0;
    for (i, (r, w)) in res.iter().zip(&written).enumerate() {
        let stored = gta6_form_h::stored_body(&[&w.bytes]);
        let mut row = gta6_form_h::row(r.type_code, w.virtual_len, w.physical_len, &w.chunks)
            .expect("a fixture row");
        LE::write_u32(&mut row[12..], (at / 16) as u32);
        t[main_start + o20 + 20 * i..][..20].copy_from_slice(&row);
        LE::write_u64(&mut t[main_start + 8 * i..], name_hash64(r.name.as_bytes()));
        LE::write_u32(
            &mut t[main_start + o32 + 4 * i..],
            name_hash32(r.name.as_bytes()),
        );
        LE::write_u64(&mut t[ext_start + 8 * i..], HASH_BASE + i as u64);
        let cell = main_start + oe0 + 8 * i;
        LE::write_u32(&mut t[cell..], (eh - cell) as u32);
        LE::write_u32(&mut t[cell + 4..], tables[i].0);
        for word in &tables[i].1 {
            LE::write_u64(&mut t[eh..], *word);
            eh += 8;
        }
        let slot = main_index + 6 * i;
        t[slot..slot + 3].fill(i as u8);
        t[slot + 3] = lists
            .iter()
            .position(|l| l.0 == i)
            .map_or(0xFF, |k| k as u8);
        t[slot + 4] = floats
            .iter()
            .position(|f| f.0 == i)
            .map_or(0xFF, |k| k as u8);
        t[slot + 5] = i as u8;
        t[split + i] = i as u8;
        let len = stored.len() as u64;
        bodies.push((at, stored));
        at += len + GAP;
    }
    let mut lh = h97;
    for (k, (_, list)) in lists.iter().enumerate() {
        let cell = main_start + o97 + 8 * k;
        LE::write_u32(&mut t[cell..], (lh - cell) as u32);
        LE::write_u32(&mut t[cell + 4..], (1 << 29) | list.len() as u32);
        t[lh..lh + list.len()].copy_from_slice(list);
        lh += list.len();
    }
    for (k, (_, f)) in floats.iter().enumerate() {
        let (cell, h) = (main_start + od0 + 8 * k, hd0 + 16 * k);
        LE::write_u32(&mut t[cell..], (h - cell) as u32);
        LE::write_u32(&mut t[cell + 4..], 4);
        for (j, x) in f.iter().enumerate() {
            LE::write_f32(&mut t[h + 4 * j..], *x);
        }
    }
    let bodies: Vec<(u64, &[u8])> = bodies.iter().map(|(a, s)| (*a, &s[..])).collect();
    seal(t, n, &bodies, at)
}

/// What a read past entry `e`'s zero slot returns: its resource's sections.
fn resource(file: &[u8], mount: &Gta6Mount, e: usize) -> Rpf9Sections {
    let start = mount.absolute_start(&mount.archive.entries[e]).unwrap();
    let (find, bytes) = read_rpf9_body(Cursor::new(file), start + RPF9_SLOT, u64::MAX)
        .expect("the resource reads past its slot");
    Rpf9Sections::of(&find, &bytes).expect("a resource")
}

/// The sections a loose file reads as.
fn sections_of(loose: &[u8]) -> Rpf9Sections {
    let (find, bytes) = read_rpf9_body(Cursor::new(loose), 0, u64::MAX).expect("it reads");
    Rpf9Sections::of(&find, &bytes).expect("a resource")
}

/// Entry `e`'s `0xd07a` slice, where it has one.
fn floats(mount: &Gta6Mount, e: usize) -> Option<Vec<u8>> {
    let table = Table::from_toc(&mount.archive.toc).unwrap();
    let c = table.col(table::HEAP_D07A, 8)?;
    table
        .row_of(e, c)
        .map(|r| table.columns[c].rows[r].slice.clone())
}

fn dependencies(mount: &Gta6Mount, e: usize) -> Option<Vec<u64>> {
    let deps = gta6_dependencies::read(&mount.archive.toc)
        .unwrap()
        .unwrap();
    deps.entry(e).map(<[u64]>::to_vec)
}

fn index_of(mount: &Gta6Mount, name: &str) -> usize {
    let h = name_hash64(name.as_bytes());
    (mount.archive.entries.iter())
        .position(|e| e.name64 == h)
        .expect("the entry is listed")
}

#[test]
fn a_table_with_heaps_lays_out_again_byte_identical() {
    let res = resources();
    let file = heaped(&res);
    let mount = open(&file);
    assert_eq!(mount.archive.entries.len(), 3);
    let table = Table::from_toc(&mount.archive.toc).unwrap();
    assert!(table.faithful, "the hand layout and the writer's agree");
    assert!(relays_identically(&mount.archive.toc).unwrap());
    assert_eq!(table.patch(), mount.archive.toc.bytes);
    for (e, r) in res.iter().enumerate() {
        assert_eq!(resource(&file, &mount, e), sections_of(&loose(&r.v, &r.p)));
    }
    assert_eq!(dependencies(&mount, 0), Some(vec![KEYS[0], KEYS[2]]));
    assert_eq!(dependencies(&mount, 2), None);

    // A heap column the writer did not know would be dropped from the layout, which then differs.
    let mut toc = mount.archive.toc.clone();
    let d07a = HEAPED_METADATA + HEAPED_METADATA_LEN + 24 * 4;
    LE::write_u64(&mut toc.bytes[d07a..], 0x0123_4567_89AB_CDEF);
    toc.groups[4].name_hash = 0x0123_4567_89AB_CDEF;
    assert!(!relays_identically(&toc).unwrap());
}

#[test]
fn removing_from_a_table_with_heaps_keeps_every_other_slice() {
    let file = heaped(&resources());
    let before = open(&file);
    let (plan, out) = write(&file, |w| {
        w.remove(&[1]);
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 1);
    let after = open(&out);
    assert_eq!(after.archive.entries.len(), 2);
    assert_eq!(dependencies(&after, 0), Some(vec![KEYS[0], KEYS[2]]));
    assert_eq!(dependencies(&after, 1), None);
    assert_eq!(floats(&after, 0), floats(&before, 0));
    assert_eq!(floats(&after, 1), floats(&before, 2));
    assert_eq!(resource(&out, &after, 1), resource(&file, &before, 2));
    assert!(relays_identically(&after.archive.toc).unwrap());
}

#[test]
fn replace_resource_reopens_and_reads_the_new_resource() {
    let file = heaped(&resources());
    let before = open(&file);
    let (v, p) = sections(0x900, 0x2_0040, 9);
    let new = loose(&v, &p);
    let (plan, out) = write(&file, |w| {
        w.replace_resource(&[0], new.clone(), None);
    })
    .unwrap();
    assert_eq!((plan.tables_changed, plan.bodies_changed), (1, 1));
    let mount = open(&out);
    assert_eq!(resource(&out, &mount, 0), sections_of(&new));
    assert_eq!(resource(&out, &mount, 1), resource(&file, &before, 1));
    let extras = mount.archive.entry_extras();
    let row = extras[0].nested.expect("a 20-byte row");
    assert_eq!(
        (row.virtual_size & !0xF, row.flags & !0xF, row.resource_type),
        (0x900, 0x2_0040, 0x02)
    );
    assert_eq!(extras[0].chunk_count, Some(3));
    assert_eq!(
        extras[0].content_hash,
        Some(0),
        "a changed entry's hash is zero"
    );
    assert_eq!(extras[1].content_hash, Some(HASH_BASE + 1));
    // What nothing in the body states is carried.
    assert_eq!(dependencies(&mount, 0), Some(vec![KEYS[0], KEYS[2]]));
    assert_eq!(floats(&mount, 0), floats(&before, 0));

    // Inside a nested archive, the child is rewritten, then the parent's row.
    let parent = archive(&[
        ("child.rpf", Body::Nested(file.clone())),
        ("top.bin", Body::Raw(b"top".to_vec())),
    ]);
    let (plan, out) = write(&parent, |w| {
        w.replace_resource(&[0, 2], new.clone(), None);
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 2);
    let top = open(&out);
    let child = top
        .open_nested(&mut Cursor::new(&out[..]), &top.archive.entries[0], &keys())
        .expect("the nested archive reopens");
    assert_eq!(resource(&out, &child, 2), sections_of(&new));
    assert_eq!(resource(&out, &child, 0), resource(&file, &before, 0));
}

#[test]
fn replace_resource_by_its_own_loose_form_is_byte_identical() {
    let res = resources();
    let file = heaped(&res);
    let (plan, out) = write(&file, |w| {
        for (e, r) in res.iter().enumerate() {
            w.replace_resource(&[e as u32], loose(&r.v, &r.p), None);
        }
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 0);
    assert_eq!(out, file);
}

#[test]
fn replace_resource_sets_a_dependency_list_from_the_table_s_keys() {
    let res = resources();
    let file = heaped(&res);
    let same = |e: usize| loose(&res[e].v, &res[e].p);
    let (plan, out) = write(&file, |w| {
        w.replace_resource(&[1], same(1), Some(vec![KEYS[2], KEYS[0], KEYS[2]]));
    })
    .unwrap();
    assert_eq!((plan.tables_changed, plan.bodies_changed), (1, 0));
    let mount = open(&out);
    assert_eq!(dependencies(&mount, 1), Some(vec![KEYS[0], KEYS[2]]));
    assert_eq!(dependencies(&mount, 0), Some(vec![KEYS[0], KEYS[2]]));
    assert_eq!(resource(&out, &mount, 1), sections_of(&same(1)));

    let (_, out) = write(&file, |w| {
        w.replace_resource(&[0], same(0), Some(Vec::new()));
    })
    .unwrap();
    assert_eq!(dependencies(&open(&out), 0), None);

    // A key the table lacks is added to it; 0x42 sorts first, so every other list's indices move.
    let (_, out) = write(&file, |w| {
        w.replace_resource(&[0], same(0), Some(vec![0x42]));
    })
    .unwrap();
    let mount = open(&out);
    assert_eq!(dependencies(&mount, 0), Some(vec![0x42]));
    assert_eq!(dependencies(&mount, 1), Some(vec![KEYS[1]]));
}

/// A key between `KEYS[1]` and `KEYS[2]`, so adding it moves the index of `KEYS[2]`.
const NEW_KEY: u64 = 0x0250_0000_0000_00D4;

/// The metadata length and the index-keyed tail's length a table's header states.
fn metadata_lengths(mount: &Gta6Mount) -> (u32, u32) {
    let b = &mount.archive.toc.bytes;
    (LE::read_u32(&b[0x1C..]), LE::read_u32(&b[0x30..]))
}

#[test]
fn a_dependency_key_the_table_lacks_is_added_and_every_list_re_indexed() {
    let res = resources();
    let file = heaped(&res);
    let before = open(&file);
    let same = |e: usize| loose(&res[e].v, &res[e].p);
    let (plan, out) = write(&file, |w| {
        w.replace_resource(&[1], same(1), Some(vec![NEW_KEY, KEYS[1]]));
    })
    .unwrap();
    assert_eq!((plan.tables_changed, plan.bodies_changed), (1, 0));
    let mount = open(&out);
    let deps = gta6_dependencies::read(&mount.archive.toc)
        .unwrap()
        .unwrap();
    assert_eq!(deps.keys, [KEYS[0], KEYS[1], NEW_KEY, KEYS[2]]);
    assert_eq!(deps.width, 1);
    assert_eq!(deps.entry(0), Some(&[KEYS[0], KEYS[2]][..]));
    assert_eq!(deps.entry(1), Some(&[KEYS[1], NEW_KEY][..]));
    assert_eq!(deps.entry(2), None);
    for e in 0..3 {
        assert_eq!(resource(&out, &mount, e), resource(&file, &before, e));
    }
    assert_eq!(floats(&mount, 0), floats(&before, 0));

    // One key more fills the type-9 payload's padding: that payload moves by 8 and keeps its
    // bytes, what comes before the table is as it was, and the metadata keeps its length.
    let (old, new) = (&before.archive.toc.bytes, &mount.archive.toc.bytes);
    assert_eq!(metadata_lengths(&mount), (0x170, 0x30));
    assert_eq!(
        (LE::read_u32(&new[0x5C..]), LE::read_u32(&new[0x64..])),
        (0x140, 0x168)
    );
    assert_eq!(new[HEAPED_METADATA..KEYS_AT], old[HEAPED_METADATA..KEYS_AT]);
    assert_eq!(new[TYPE9_AT + 8..TYPE9_AT + 16], TYPE9[..8]);

    // Two keys more pass it, and the metadata grows to the next 16 bytes.
    let (_, out) = write(&file, |w| {
        w.replace_resource(&[1], same(1), Some(vec![NEW_KEY, NEW_KEY + 1]));
    })
    .unwrap();
    let mount = open(&out);
    assert_eq!(metadata_lengths(&mount), (0x180, 0x40));
    assert_eq!(dependencies(&mount, 1), Some(vec![NEW_KEY, NEW_KEY + 1]));
    assert_eq!(dependencies(&mount, 0), Some(vec![KEYS[0], KEYS[2]]));
    assert_eq!(resource(&out, &mount, 2), resource(&file, &before, 2));
    assert!(relays_identically(&mount.archive.toc).unwrap());
}

#[test]
fn growing_the_key_table_past_256_keys_widens_every_list() {
    let res = resources();
    let file = heaped(&res);
    let same = loose(&res[1].v, &res[1].p);
    let added = |n: u64| {
        (0..n)
            .map(|i| 0x0400_0000_0000_0000 + i)
            .collect::<Vec<u64>>()
    };
    let grown = |n: u64| {
        let (_, out) = write(&file, |w| {
            w.replace_resource(&[1], same.clone(), Some(added(n)));
        })
        .unwrap();
        let deps = gta6_dependencies::read(&open(&out).archive.toc)
            .unwrap()
            .unwrap();
        (deps, out)
    };
    let (at_256, _) = grown(253);
    assert_eq!((at_256.keys.len(), at_256.width), (256, 1));

    let (past, out) = grown(254);
    assert_eq!((past.keys.len(), past.width), (257, 2));
    assert_eq!(past.entry(0), Some(&[KEYS[0], KEYS[2]][..]));
    assert_eq!(past.entry(1), Some(&added(254)[..]));
    let mount = open(&out);
    let table = Table::from_toc(&mount.archive.toc).unwrap();
    let c = table.col(GROUP_DEPENDENCIES, 8).unwrap();
    assert!((table.columns[c].rows.iter()).all(|r| LE::read_u32(&r.cell[4..]) >> 29 == 2));
    assert_eq!(resource(&out, &mount, 1), sections_of(&same));
    assert!(relays_identically(&mount.archive.toc).unwrap());
}

#[test]
fn growing_the_key_table_moves_what_follows_and_refuses_what_it_cannot_lay_out() {
    let prefix = open(&heaped(&resources())).archive.toc.bytes
        [..HEAPED_METADATA + HEAPED_METADATA_LEN]
        .to_vec();
    let grown = |prefix: &[u8], keys: &[u64]| gta6_dependencies::grow_table(prefix, keys);
    assert_eq!(
        grown(&prefix, &KEYS).unwrap(),
        prefix,
        "the same keys lay out as they were"
    );
    assert!(
        grown(&prefix, &[KEYS[0], KEYS[2]]).is_err(),
        "a key dropped"
    );
    assert!(
        grown(&prefix, &[KEYS[1], KEYS[0], KEYS[2]]).is_err(),
        "out of order"
    );
    let more = [KEYS[0], KEYS[1], NEW_KEY, KEYS[2]];
    let mut before_tail = prefix.clone();
    LE::write_u32(&mut before_tail[0x30..], 0x10);
    assert!(
        grown(&before_tail, &more).is_err(),
        "a table before the index-keyed tail"
    );

    // Padding that holds a byte is no padding: every byte is kept, the metadata padded past it.
    let mut held = prefix.clone();
    held[TYPE9_AT + 12] = 7;
    let out = grown(&held, &more).unwrap();
    assert_eq!(LE::read_u32(&out[0x1C..]), 0x180);
    assert_eq!(
        out[TYPE9_AT + 8..TYPE9_AT + 24],
        held[TYPE9_AT..TYPE9_AT + 16]
    );
    assert!(out[TYPE9_AT + 24..HEAPED_METADATA + 0x180]
        .iter()
        .all(|&b| b == 0));
}

#[test]
fn an_added_resource_takes_a_dependency_list_and_grows_the_table() {
    let parent = archive(&[
        ("child.rpf", Body::Nested(heaped(&resources()))),
        ("top.bin", Body::Raw(b"top".to_vec())),
    ]);
    let (v, p) = sections(0x300, 0x100, 7);
    let new = loose(&v, &p);
    let (plan, out) = write(&parent, |w| {
        w.add_with_dependencies(
            &[0],
            "maps/d.ydr",
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: new.clone(),
            },
            vec![NEW_KEY, KEYS[1]],
        );
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 2);
    let top = open(&out);
    let child = top
        .open_nested(&mut Cursor::new(&out[..]), &top.archive.entries[0], &keys())
        .expect("the nested archive reopens");
    let d = index_of(&child, "maps/d.ydr");
    assert_eq!(resource(&out, &child, d), sections_of(&new));
    assert_eq!(dependencies(&child, d), Some(vec![KEYS[1], NEW_KEY]));
    let a = index_of(&child, "maps/a.ydr");
    assert_eq!(dependencies(&child, a), Some(vec![KEYS[0], KEYS[2]]));

    // An archive with no dependency lists has no table to set one against.
    assert!(write(&sample(), |w| {
        w.add_with_dependencies(&[], "x.txt", Rpf9NewBody::Chunked(beta()), vec![KEYS[0]]);
    })
    .is_err());
}

/// Entry `e`'s chunk-table mode, the top byte of its `0xe0c1` cell.
fn chunk_mode(mount: &Gta6Mount, e: usize) -> Option<u8> {
    let table = Table::from_toc(&mount.archive.toc).unwrap();
    let k = table.col(GROUP_CHUNK_RECORD, 8)?;
    table.cell(e, k).map(|cell| cell[7])
}

#[test]
fn a_body_written_into_a_mode_4_table_takes_mode_4() {
    let res = resources();
    let (v, p) = sections(0x900, 0x2_0040, 9);
    let new = loose(&v, &p);
    let edits = |w: &mut Rpf9ArchiveWriter| {
        w.replace_resource(&[0], new.clone(), None);
        w.add(
            &[],
            "maps/d.ydr",
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: new.clone(),
            },
        );
    };
    // Each written row takes the mode the table's rows take.
    for mode in [MODE4, ROW20_CHUNK_MODE] {
        let file = heaped_in(&res, mode);
        assert_eq!(chunk_mode(&open(&file), 1), Some(mode));
        let (_, out) = write(&file, edits).unwrap();
        let mount = open(&out);
        for name in ["maps/a.ydr", "maps/d.ydr"] {
            let e = index_of(&mount, name);
            assert_eq!(chunk_mode(&mount, e), Some(mode), "{name} in mode {mode}");
            assert_eq!(resource(&out, &mount, e), sections_of(&new));
            let chunks = entry_chunks(&mount.archive, e).unwrap().unwrap();
            let inflated: u64 = chunks.iter().map(|c| u64::from(c.inflated)).sum();
            assert_eq!(inflated, 0x900 + 0x2_0040);
        }
        assert_eq!(
            chunk_mode(&mount, index_of(&mount, "maps/b.ytd")),
            Some(mode)
        );
    }

    // A 24-byte row's chunked body in a mode-4 table, replaced and added.
    let file = archive_in(
        &[
            ("alpha.bin", Body::Raw(alpha())),
            ("beta.txt", Body::Chunked(beta())),
        ],
        MODE4,
    );
    let body = text("mode four text ", 90_000);
    let (_, out) = write(&file, |w| {
        w.replace(&[1], body.clone());
        w.add(&[], "added.txt", Rpf9NewBody::Chunked(body.clone()));
    })
    .unwrap();
    let mount = open(&out);
    for name in ["beta.txt", "added.txt"] {
        let e = index_of(&mount, name);
        assert_eq!(chunk_mode(&mount, e), Some(MODE4), "{name}");
        assert_eq!(chunked(&out, &mount, e), body);
    }
}

#[test]
fn add_puts_an_entry_in_name_order_and_reads_it_back() {
    let file = sample();
    let before = open(&file);
    let name = "added/new.txt";
    let body = text("an added text ", 90_000);
    let (plan, out) = write(&file, |w| {
        w.add(&[], name, Rpf9NewBody::Chunked(body.clone()));
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 1);
    let after = open(&out);
    assert_eq!(after.archive.entries.len(), 4);
    let e = index_of(&after, name);
    let h32 = name_hash32(name.as_bytes());
    let expected = (before.archive.entries.iter())
        .position(|x| x.name32.is_some_and(|k| k > h32))
        .unwrap_or(3);
    assert_eq!(e, expected);
    assert_eq!(after.archive.entries[e].name32, Some(h32));
    assert_eq!(chunked(&out, &after, e), body);
    assert_eq!(after.archive.entry_extras()[e].content_hash, Some(0));
    assert_eq!(raw(&out, &after, index_of(&after, "alpha.bin")), alpha());
    assert_eq!(chunked(&out, &after, index_of(&after, "beta.txt")), beta());

    // A resource into a nested table with heaps: neither carried column gives it a row.
    let parent = archive(&[
        ("child.rpf", Body::Nested(heaped(&resources()))),
        ("top.bin", Body::Raw(b"top".to_vec())),
    ]);
    let (v, p) = sections(0x300, 0x100, 7);
    let new = loose(&v, &p);
    let (plan, out) = write(&parent, |w| {
        w.add(
            &[0],
            "maps/d.ydr",
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: new.clone(),
            },
        );
    })
    .unwrap();
    assert_eq!(plan.tables_changed, 2);
    let top = open(&out);
    let child = top
        .open_nested(&mut Cursor::new(&out[..]), &top.archive.entries[0], &keys())
        .expect("the nested archive reopens");
    assert_eq!(child.archive.entries.len(), 4);
    let d = index_of(&child, "maps/d.ydr");
    assert_eq!(resource(&out, &child, d), sections_of(&new));
    assert_eq!((dependencies(&child, d), floats(&child, d)), (None, None));
    let a = index_of(&child, "maps/a.ydr");
    assert_eq!(dependencies(&child, a), Some(vec![KEYS[0], KEYS[2]]));
    assert!(relays_identically(&child.archive.toc).unwrap());
}

#[test]
fn build_makes_an_archive_from_nothing_that_reopens_and_reads_back() {
    let child = sample();
    let (v, p) = sections(0x1_0000 + 0x80, 0x40, 5);
    let resource_loose = loose(&v, &p);
    let note = text("built text ", 80_000);
    let entries = vec![
        (
            "docs/readme.txt".to_owned(),
            Rpf9NewBody::Chunked(note.clone()),
        ),
        (
            "maps/built.ydr".to_owned(),
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: resource_loose.clone(),
            },
        ),
        ("child.rpf".to_owned(), Rpf9NewBody::Archive(child.clone())),
    ];
    let key = Rpf9IndexKey::of(&open(&child).archive.toc).unwrap();
    assert_eq!((key.key_id, key.iv), (KEY_ID, [0x33; 16]));
    let built_once = || {
        let plan = build(&entries, &key, &keys()).expect("built");
        let mut out = Vec::new();
        plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut out)
            .expect("written");
        out
    };
    let built = built_once();
    assert_eq!(built_once(), built, "a build is deterministic");

    let mount = open(&built);
    let listed = &mount.archive.entries;
    assert_eq!(listed.len(), 3);
    assert!(listed.windows(2).all(|w| w[0].name32 < w[1].name32));
    let readme = index_of(&mount, "docs/readme.txt");
    assert_eq!(chunked(&built, &mount, readme), note);
    let r = index_of(&mount, "maps/built.ydr");
    assert_eq!(resource(&built, &mount, r), sections_of(&resource_loose));
    let nested = mount
        .open_nested(
            &mut Cursor::new(&built[..]),
            &listed[index_of(&mount, "child.rpf")],
            &keys(),
        )
        .expect("the nested archive opens");
    assert_eq!(raw(&built, &nested, 0), alpha());

    // The type-5, signature and type-9 payloads are empty, and the table lays out again as it is.
    let toc = &mount.archive.toc.bytes;
    assert!(toc[0x60 + 0x18..0x60 + 0x150].iter().all(|&b| b == 0));
    assert!(relays_identically(&mount.archive.toc).unwrap());
    assert_eq!(write(&built, |_| {}).unwrap().1, built);

    let refused = |entries: &[(String, Rpf9NewBody)]| build(entries, &key, &keys()).is_err();
    assert!(refused(&[]));
    assert!(refused(&[
        ("same".to_owned(), Rpf9NewBody::Chunked(note.clone())),
        ("same".to_owned(), Rpf9NewBody::Chunked(note.clone())),
    ]));
    assert!(refused(&[(
        "unknown.bin".to_owned(),
        Rpf9NewBody::Resource {
            type_code: 0x01,
            loose: resource_loose.clone(),
        }
    )]));
}

#[test]
fn what_the_new_edits_cannot_write_is_refused_by_name() {
    let file = sample();
    let res = resources();
    let heaped_file = heaped(&res);
    let r = &res[0];
    let refused = |file: &[u8], edit: &dyn Fn(&mut Rpf9ArchiveWriter)| {
        matches!(write(file, edit), Err(IoError::Invalid { .. }))
    };
    // A 24-byte entry has no headerless form, and a 20-byte one takes no plain replace.
    assert!(refused(&file, &|w| {
        w.replace_resource(&[0], loose(&r.v, &r.p), None);
    }));
    assert!(refused(&heaped_file, &|w| {
        w.replace(&[0], beta());
    }));
    assert!(refused(&heaped_file, &|w| {
        w.replace_resource(&[0], b"not a resource".to_vec(), None);
    }));
    // A name taken, a table with no 20-byte column, an archive that is none.
    assert!(refused(&file, &|w| {
        w.add(&[], "alpha.bin", Rpf9NewBody::Chunked(beta()));
    }));
    assert!(refused(&file, &|w| {
        w.add(
            &[],
            "new.ydr",
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: loose(&r.v, &r.p),
            },
        );
    }));
    assert!(refused(&file, &|w| {
        w.add(
            &[],
            "x.rpf",
            Rpf9NewBody::Archive(b"not an archive".to_vec()),
        );
    }));
}
