//! Each archive tool run on an archive laid out by hand here and enciphered under a synthetic
//! white-box key, so no game byte is involved; the verb runs in this process rather than a child.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use or_io::container::rpf::gta6_chunk_table::{author, Chunk, Table as ChunkTable};
use or_io::container::rpf::{
    Gta6Mount, GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH,
    GROUP_ENTRY_ROW, GROUP_NAME32, GROUP_NAME64, RPF9_MAGIC,
};
use or_io::gta6::hash::{name_hash32, name_hash64};
use serde_json::json;

use super::*;

// --- the synthetic key and archive -----------------------------------------------------------------

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
/// The inflated size of one chunk of a compressed body.
const CHUNK: usize = 0x10000;

fn round8(x: usize) -> usize {
    (x + 7) & !7
}

fn round16(x: u64) -> u64 {
    (x + 15) & !15
}

fn put(t: &mut [u8], at: usize, bytes: &[u8]) {
    t[at..at + bytes.len()].copy_from_slice(bytes);
}

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
}

/// `body` cut into zlib chunks, each padded to 16 bytes, and the chunks' table.
fn chunk_body(body: &[u8]) -> (Vec<u8>, Vec<Chunk>) {
    let mut out = Vec::new();
    let mut chunks = Vec::new();
    for piece in body.chunks(CHUNK) {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(piece).unwrap();
        let deflated = encoder.finish().unwrap();
        let zlib = deflated.len() < piece.len();
        let stored = if zlib { &deflated[..] } else { piece };
        chunks.push(Chunk {
            inflated: piece.len() as u32,
            stored: stored.len() as u32,
            zlib,
        });
        out.extend_from_slice(stored);
        out.resize(round16(out.len() as u64) as usize, 0);
    }
    (out, chunks)
}

/// An archive laid out by hand: a header, four descriptors in 0x150 bytes of metadata, the columns
/// row24, name64, name32 and `0xe0c1` in the main region and `0xb6f5` external, bodies `GAP` apart,
/// and a names heap.
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
                let (s, chunks) = chunk_body(b);
                *chunk_slot.last_mut().unwrap() = slices.len() as u8;
                slices.push(author(&ChunkTable { mode: 3, chunks }).unwrap());
                stored.push((0x0B, s, b.len() as u64));
            }
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
    put(&mut t, 0x00, &RPF9_MAGIC.to_le_bytes());
    put(&mut t, 0x04, &(toc_end as u32).to_le_bytes());
    put(&mut t, 0x08, &(split as u32).to_le_bytes());
    put(&mut t, 0x0C, &(NAMES_HEAP as u32).to_le_bytes());
    for (i, b) in t[0x10..0x18].iter_mut().enumerate() {
        *b = i as u8 + 1;
    }
    t[0x19] = 0x7A;
    t[0x1A] = 4;
    put(&mut t, 0x1C, &0x150u32.to_le_bytes());
    put(&mut t, 0x20, &5u16.to_le_bytes());
    put(&mut t, 0x22, &1u16.to_le_bytes());
    put(&mut t, 0x24, &(n as u32).to_le_bytes());
    put(&mut t, 0x28, &0xC0FF_EE11u32.to_le_bytes());
    t[0x2C] = 1;
    t[0x2D] = 0x0F;
    t[0x2E] = 0x09;
    put(&mut t, 0x30, &0x10u32.to_le_bytes());
    for (i, (kind, off)) in [(1u16, 0u32), (5, 0x20), (6, 0x40), (9, 0x140)]
        .iter()
        .enumerate()
    {
        put(&mut t, 0x40 + 8 * i, &kind.to_le_bytes());
        put(&mut t, 0x44 + 8 * i, &off.to_le_bytes());
    }
    put(&mut t, 0x60, &KEY_ID.to_le_bytes());
    put(&mut t, 0x64, &16u16.to_le_bytes());
    t[0x66..0x76].fill(0x33);
    for (i, b) in t[0x80..0x1B0].iter_mut().enumerate() {
        *b = (i * 7 + 1) as u8;
    }
    let directory = [
        (GROUP_ENTRY_ROW, n, 0, 24u32, 0u32),
        (GROUP_NAME64, n, name64, 8, 0),
        (GROUP_NAME32, n, name32, 4, 0),
        (GROUP_CHUNK_RECORD, slices.len(), e0c1, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        put(&mut t, at, &hash.to_le_bytes());
        put(&mut t, at + 8, &(count as u32).to_le_bytes());
        put(&mut t, at + 12, &(offset as u32).to_le_bytes());
        put(&mut t, at + 16, &(width | (external << 16)).to_le_bytes());
    }

    let mut at = round16(toc_end as u64 + GAP);
    let mut starts = Vec::new();
    for (i, ((name, _), (class, s, real))) in entries.iter().zip(&stored).enumerate() {
        let row = main_start + 24 * i;
        put(&mut t, row, &((class << 40) | (at / 16)).to_le_bytes());
        put(&mut t, row + 8, &(s.len() as u64 / 16).to_le_bytes());
        put(&mut t, row + 16, &real.to_le_bytes());
        let name = name.as_bytes();
        put(
            &mut t,
            main_start + name64 + 8 * i,
            &name_hash64(name).to_le_bytes(),
        );
        put(
            &mut t,
            main_start + name32 + 4 * i,
            &name_hash32(name).to_le_bytes(),
        );
        put(
            &mut t,
            ext_start + 8 * i,
            &(0x1111_0000 + i as u64).to_le_bytes(),
        );
        t[main_index + 4 * i..main_index + 4 * i + 3].fill(i as u8);
        t[main_index + 4 * i + 3] = chunk_slot[i];
        t[split + i] = i as u8;
        starts.push(at);
        at += s.len() as u64 + GAP;
    }
    let mut h = heap;
    for (k, (u1, words)) in slices.iter().enumerate() {
        let cell = main_start + e0c1 + 8 * k;
        put(&mut t, cell, &((h - cell) as u32).to_le_bytes());
        put(&mut t, cell + 4, &u1.to_le_bytes());
        for w in words {
            put(&mut t, h, &w.to_le_bytes());
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

fn text_of(seed: &str, len: usize) -> Vec<u8> {
    seed.bytes().cycle().take(len).collect()
}

fn beta() -> Vec<u8> {
    text_of("beta chunked text ", 150_000)
}

fn sample() -> Vec<u8> {
    archive(&[
        ("alpha.bin", Body::Raw(b"alpha body, stored raw".to_vec())),
        ("beta.txt", Body::Chunked(beta())),
        ("gamma.bin", Body::Raw(vec![0x47; 40])),
    ])
}

// --- running a tool --------------------------------------------------------------------------------

/// A folder holding `sample.rpf` and the synthetic key snapshot.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("sample.rpf"), sample()).unwrap();
        std::fs::write(dir.path().join("keys.snapshot"), clean_snapshot()).unwrap();
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn archive(&self) -> PathBuf {
        self.path("sample.rpf")
    }

    /// Run `tool` with `pairs`, the key snapshot and, unless `pairs` names a folder, the archive.
    fn run(&self, tool: &str, pairs: &[(&str, &Path)]) -> Result<Vec<Value>, String> {
        let mut args = Map::new();
        if !pairs.iter().any(|(key, _)| *key == "folder") {
            args.insert(
                "archive".into(),
                json!(self.archive().display().to_string()),
            );
        }
        args.insert(
            "keys".into(),
            json!(self.path("keys.snapshot").display().to_string()),
        );
        for (key, value) in pairs {
            args.insert((*key).into(), json!(value.display().to_string()));
        }
        answer(tool, &args, in_process)
    }
}

/// The verb run in this process, as the child `openrage-cli` would run it.
fn in_process(line: &[String]) -> Result<(bool, String), String> {
    assert_eq!(line[0], "rpf9");
    Ok((
        crate::rpf9::run(&line[1..]) == ExitCode::SUCCESS,
        line.join(" "),
    ))
}

fn open(path: &Path) -> (Vec<u8>, Gta6Mount) {
    let bytes = std::fs::read(path).expect("the written archive");
    let mount = Gta6Mount::open_root(&mut Cursor::new(&bytes[..]), &keys()).expect("it reopens");
    (bytes, mount)
}

fn body(bytes: &[u8], mount: &Gta6Mount, e: usize) -> Vec<u8> {
    mount
        .read_entry(Cursor::new(bytes), &mount.archive.entries[e])
        .expect("the body reads")
}

// --- the tools -------------------------------------------------------------------------------------

#[test]
fn every_tool_names_its_verb_and_every_write_tool_says_the_game_refuses_it() {
    let listed = tools();
    assert_eq!(listed.len(), TOOLS.len());
    for ((name, verb, _, writes, _), listed_tool) in TOOLS.iter().zip(&listed) {
        assert_eq!(listed_tool["name"], json!(name));
        assert!(name.starts_with("rpf9-") && !verb.is_empty());
        let about = listed_tool["description"].as_str().unwrap();
        assert_eq!(
            *writes,
            about.contains("The game refuses the result"),
            "{name}"
        );
        assert_eq!(
            *writes,
            about.contains("never changes the source"),
            "{name}"
        );
    }
    assert!(is_tool("rpf9-copy") && !is_tool("cli"));
}

#[test]
fn a_tool_gives_positional_words_in_order_and_refuses_a_missing_one_before_running() {
    let mut args = Map::new();
    args.insert("entry".into(), json!("2"));
    args.insert("outDir".into(), json!("D:/out"));
    args.insert("archive".into(), json!("D:/a.rpf"));
    args.insert("inner".into(), json!("x.rpf"));
    let (_, verb, _, _, params) = TOOLS.iter().find(|t| t.0 == "rpf9-read").unwrap();
    assert_eq!(
        line_of(verb, params, &args).unwrap(),
        [
            "rpf9",
            "extract",
            "--decrypt",
            "D:/a.rpf",
            "D:/out",
            "x.rpf",
            "--entry",
            "2"
        ]
    );
    args.remove("archive");
    let refused = answer("rpf9-read", &args, |_| panic!("nothing runs")).unwrap_err();
    assert!(refused.contains("'archive' is required"), "{refused}");
    args.insert("archive".into(), json!(3));
    assert!(answer("rpf9-read", &args, |_| panic!("nothing runs")).is_err());

    let mut args = Map::new();
    for (key, value) in [
        ("archive", "a"),
        ("entry", "2"),
        ("with", "w"),
        ("out", "o"),
    ] {
        args.insert(key.into(), json!(value));
    }
    let (_, verb, _, _, params) = TOOLS.iter().find(|t| t.0 == "rpf9-replace").unwrap();
    let flagged = |args: &Map<String, Value>| {
        line_of(verb, params, args)
            .unwrap()
            .contains(&"--resource".to_string())
    };
    assert!(!flagged(&args), "no flag unless asked");
    args.insert("resource".into(), json!(true));
    assert!(flagged(&args), "a true boolean is its flag");
}

#[test]
fn rpf9_list_lists_the_archive_and_a_file_that_is_not_one_is_an_error() {
    let fx = Fixture::new();
    fx.run("rpf9-list", &[]).expect("the archive lists");
    std::fs::write(fx.path("junk.rpf"), b"not an archive").unwrap();
    assert!(fx
        .run("rpf9-list", &[("archive", fx.path("junk.rpf").as_path())])
        .is_err());
}

#[test]
fn rpf9_find_finds_an_entry_across_the_folder_by_hash() {
    let fx = Fixture::new();
    let hash = format!("{:016x}", name_hash64(b"gamma.bin"));
    let mut args = Map::new();
    args.insert("folder".into(), json!(fx.dir.path().display().to_string()));
    args.insert("hash".into(), json!(hash));
    args.insert(
        "keys".into(),
        json!(fx.path("keys.snapshot").display().to_string()),
    );
    answer("rpf9-find", &args, in_process).expect("the folder is searched");
}

#[test]
fn rpf9_read_writes_the_entrys_inflated_body() {
    let fx = Fixture::new();
    let out = fx.path("read");
    fx.run(
        "rpf9-read",
        &[("outDir", out.as_path()), ("entry", Path::new("1"))],
    )
    .expect("the entry is read");
    let written: Vec<PathBuf> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(std::fs::read(&written[0]).unwrap(), beta());
}

#[test]
fn rpf9_census_counts_the_folder_into_the_report_it_is_given() {
    let fx = Fixture::new();
    let out = fx.path("census");
    fx.run(
        "rpf9-census",
        &[("folder", fx.dir.path()), ("out", out.as_path())],
    )
    .expect("the folder is counted");
    let report = std::fs::read_to_string(out.join("rpf9-census.md")).unwrap();
    assert!(report.contains("| sample.rpf | 3 | 0 | 3 |"), "{report}");
}

#[test]
fn rpf9_copy_writes_a_new_archive_and_never_the_source() {
    let fx = Fixture::new();
    let out = fx.path("copy.rpf");
    fx.run("rpf9-copy", &[("out", out.as_path())])
        .expect("the copy is written");
    assert_eq!(std::fs::read(&out).unwrap(), sample());
    assert!(fx
        .run("rpf9-copy", &[("out", fx.archive().as_path())])
        .is_err());
    assert_eq!(std::fs::read(fx.archive()).unwrap(), sample());
}

#[test]
fn rpf9_replace_writes_an_archive_whose_entry_reads_the_new_bytes() {
    let fx = Fixture::new();
    let (with, out) = (fx.path("new.txt"), fx.path("replaced.rpf"));
    let new = text_of("other text ", 70_000);
    std::fs::write(&with, &new).unwrap();
    fx.run(
        "rpf9-replace",
        &[
            ("entry", Path::new("1")),
            ("with", with.as_path()),
            ("out", out.as_path()),
        ],
    )
    .expect("the entry is replaced");
    let (bytes, mount) = open(&out);
    assert_eq!(body(&bytes, &mount, 1), new);
}

#[test]
fn rpf9_remove_writes_an_archive_with_one_entry_fewer() {
    let fx = Fixture::new();
    let out = fx.path("removed.rpf");
    fx.run(
        "rpf9-remove",
        &[("entry", Path::new("0")), ("out", out.as_path())],
    )
    .expect("the entry is removed");
    let (bytes, mount) = open(&out);
    assert_eq!(mount.archive.entries.len(), 2);
    assert_eq!(body(&bytes, &mount, 0), beta());
}

#[test]
fn rpf9_rename_writes_an_archive_whose_entry_hashes_the_new_name() {
    let fx = Fixture::new();
    let out = fx.path("renamed.rpf");
    let name = "renamed/gamma.bin";
    fx.run(
        "rpf9-rename",
        &[
            ("entry", Path::new("2")),
            ("name", Path::new(name)),
            ("out", out.as_path()),
        ],
    )
    .expect("the entry is renamed");
    let (_, mount) = open(&out);
    assert_eq!(
        mount.archive.entries[2].name64,
        name_hash64(name.as_bytes())
    );
}

#[test]
fn rpf9_defrag_writes_a_smaller_archive_holding_every_body() {
    let fx = Fixture::new();
    let out = fx.path("packed.rpf");
    fx.run("rpf9-defrag", &[("out", out.as_path())])
        .expect("the archive is packed");
    let (bytes, mount) = open(&out);
    assert!(bytes.len() < sample().len());
    assert_eq!(mount.archive.entries.len(), 3);
    assert_eq!(body(&bytes, &mount, 1), beta());
}
