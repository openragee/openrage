//! An RPF9 archive laid out by hand and its table enciphered under a synthetic white-box key, so the
//! verbs' tests read, extract and search an archive with no game byte in it.

use or_io::container::rpf::gta6_chunk_table::{author, Chunk, Table as ChunkTable};
use or_io::container::rpf::{
    GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW,
    GROUP_NAME32, GROUP_NAME64, RPF9_MAGIC,
};
use or_io::gta6::hash::{name_hash32, name_hash64};

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

/// The keys the fixture's tables are enciphered under.
pub(super) fn keys() -> GtaViKeys {
    GtaViKeys::parse(&snapshot()).expect("the synthetic snapshot parses")
}

/// An entry's body: raw bytes (class `0xFF`), bytes stored as one zlib stream at its best level,
/// described by a chunk table (class `0xFF`), a whole nested archive (class `0x00`), or
/// chunks, each `(inflated bytes, deflated?)`, 16-aligned under a mode-3 `0xe0c1` table (class `0xFF`).
pub(super) enum Body {
    Raw(Vec<u8>),
    Zlib(Vec<u8>),
    Nested(Vec<u8>),
    // Not every test that includes this file builds a chunked body.
    #[allow(dead_code)]
    Chunked(Vec<(Vec<u8>, bool)>),
}

fn deflate(b: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(b).expect("zlib into memory");
    z.finish().expect("zlib into memory")
}

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

/// An entry's class byte, stored bytes, real size and chunks, if chunk-tabled.
type Stored = (u64, Vec<u8>, u64, Option<Vec<Chunk>>);

/// An archive laid out by hand: a header, four descriptors (an index key, then stand-ins for types
/// 5, 6 and 9) in 0x150 bytes of metadata, the columns row24, name64, name32 and `0xe0c1` in the main
/// region and `0xb6f5` external, bodies `GAP` apart, and a names heap. Each entry is named by the
/// hashes of its path. `0xe0c1` holds a cell per chunked entry only, its descriptors after every cell.
pub(super) fn archive(entries: &[(&str, Body)]) -> Vec<u8> {
    let n = entries.len();
    let stored: Vec<Stored> = entries
        .iter()
        .map(|(_, body)| match body {
            Body::Raw(b) => {
                let mut s = b.clone();
                s.resize(round16(s.len()), 0);
                (0xFF, s, b.len() as u64, None)
            }
            Body::Zlib(b) => {
                let mut s = deflate(b);
                let chunk = Chunk {
                    inflated: b.len() as u32,
                    stored: s.len() as u32,
                    zlib: true,
                };
                s.resize(round16(s.len()), 0);
                (0xFF, s, b.len() as u64, Some(vec![chunk]))
            }
            Body::Nested(b) => (0x00, b.clone(), b.len() as u64, None),
            Body::Chunked(pieces) => {
                let (mut s, mut chunks, mut real) = (Vec::new(), Vec::new(), 0u64);
                for (data, zlib) in pieces {
                    let piece = if *zlib { deflate(data) } else { data.clone() };
                    chunks.push(Chunk {
                        inflated: data.len() as u32,
                        stored: piece.len() as u32,
                        zlib: *zlib,
                    });
                    s.extend_from_slice(&piece);
                    s.resize(round16(s.len()), 0);
                    real += data.len() as u64;
                }
                (0xFF, s, real, Some(chunks))
            }
        })
        .collect();
    let tables: Vec<Option<(u32, Vec<u64>)>> = stored
        .iter()
        .map(|(_, _, _, chunks)| {
            chunks.as_ref().map(|chunks| {
                author(&ChunkTable {
                    mode: 3,
                    chunks: chunks.clone(),
                })
                .expect("the fixture's chunks fit mode 3")
            })
        })
        .collect();
    let cells = tables.iter().flatten().count();
    let words: usize = tables.iter().flatten().map(|(_, w)| w.len()).sum();
    let groups_start = 0x1B0;
    let main_index = groups_start + 5 * 24;
    let main_start = round8(main_index + 4 * n);
    let (name64, name32, e0c1) = (24 * n, 32 * n, round8(36 * n));
    let heap = main_start + e0c1 + 8 * (cells + words);
    let split = round16(heap);
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
    put32(&mut t, 0x28, HEADER_28);
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
        (GROUP_CHUNK_RECORD, cells, e0c1, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        put64(&mut t, at, hash);
        put32(&mut t, at + 8, count as u32);
        put32(&mut t, at + 12, offset as u32);
        put32(&mut t, at + 16, width | (external << 16));
    }

    let (mut cell, mut desc) = (main_start + e0c1, main_start + e0c1 + 8 * cells);
    let mut at = round16(toc_end + GAP);
    let mut starts = Vec::new();
    for (i, ((name, _), (class, s, real, _))) in entries.iter().zip(&stored).enumerate() {
        let row = main_start + 24 * i;
        put64(&mut t, row, (class << 40) | (at as u64 / 16));
        put64(&mut t, row + 8, s.len() as u64 / 16);
        put64(&mut t, row + 16, *real);
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
        put64(&mut t, ext_start + 8 * i, HASH_BASE + i as u64);
        t[main_index + 4 * i..main_index + 4 * i + 3].fill(i as u8);
        t[main_index + 4 * i + 3] = 0xFF;
        if let Some((u1, table)) = &tables[i] {
            t[main_index + 4 * i + 3] = ((cell - main_start - e0c1) / 8) as u8;
            put32(&mut t, cell, (desc - cell) as u32);
            put32(&mut t, cell + 4, *u1);
            for word in table {
                put64(&mut t, desc, *word);
                desc += 8;
            }
            cell += 8;
        }
        t[split + i] = i as u8;
        starts.push(at);
        at += s.len() + GAP;
    }

    let toc = Rpf9Toc {
        bytes: t,
        key_id: KEY_ID,
        registry_id: REGISTRY_ID,
        entry_count: n as u32,
        groups: Vec::new(),
    };
    let head = Rpf9Builder::serialize(&toc, &keys()).expect("the synthetic table enciphers");
    let mut file = vec![0u8; at + NAMES_HEAP];
    file[..head.len()].copy_from_slice(&head);
    for (start, (_, s, _, _)) in starts.iter().zip(&stored) {
        file[*start..*start + s.len()].copy_from_slice(s);
    }
    file[at..].fill(0x5A);
    file
}

/// A PSO document whose `LSNS` holds `text` in one clear run, closed by `CHKS` stating its length, so
/// the body reader finds its end.
pub(super) fn pso(text: &str) -> Vec<u8> {
    let frame = |body: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]| {
        body.extend_from_slice(ident);
        body.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(content);
    };
    let mut run = text.as_bytes().to_vec();
    run.resize(round16(run.len() + 1), 0);
    let mut blob = 1u32.to_le_bytes().to_vec();
    blob.extend_from_slice(&0x101u32.to_le_bytes());
    blob.extend_from_slice(&(run.len() as u32).to_le_bytes());
    blob.extend_from_slice(&run);
    let mut body = Vec::new();
    frame(&mut body, b"PSIN", &[0; 8]);
    frame(&mut body, b"LSNS", &blob);
    let len = body.len() + 20;
    let mut chks = [0u8; 12];
    chks[..4].copy_from_slice(&(len as u32).to_be_bytes());
    frame(&mut body, b"CHKS", &chks);
    body
}
