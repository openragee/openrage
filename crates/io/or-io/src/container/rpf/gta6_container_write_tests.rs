//! Hermetic checks of the RPF9 container writer on archives laid out by hand here (not by the writer)
//! and enciphered under a synthetic white-box key, so no game byte is involved.

use std::io::Cursor;

use super::*;
use crate::container::rpf::gta6_archive::GROUP_CONTENT_HASH;
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
                slices.push(author(&ChunkTable { mode: 3, chunks }).unwrap());
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
    LE::write_u16(&mut t[0x20..], 5);
    LE::write_u16(&mut t[0x22..], 1);
    LE::write_u32(&mut t[0x24..], n as u32);
    LE::write_u32(&mut t[0x28..], HEADER_28);
    t[0x2C] = 1;
    t[0x2D] = 0x0F;
    t[0x2E] = 0x09;
    LE::write_u32(&mut t[0x30..], 0x10);
    for (i, (kind, off)) in [(1u16, 0u32), (5, 0x20), (6, 0x40), (9, 0x140)]
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

    let toc = Rpf9Toc {
        bytes: t,
        key_id: KEY_ID,
        registry_id: REGISTRY_ID,
        entry_count: n as u32,
        groups: Vec::new(),
    };
    let head = Rpf9Builder::serialize(&toc, &keys()).expect("the synthetic table enciphers");
    let names_at = at as usize;
    let mut file = vec![0u8; names_at + NAMES_HEAP];
    file[..head.len()].copy_from_slice(&head);
    for (start, (_, s, _)) in starts.iter().zip(&stored) {
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
fn a_body_the_reader_would_not_read_back_is_refused() {
    let file = sample();
    // Plain text stored raw, and a chunk that does not compress, are not bodies the reader detects.
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
    assert!(write(&file, |w| {
        w.replace(&[1], noise);
    })
    .is_err());
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
