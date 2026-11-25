//! Hermetic checks of a GTA VI project build on archives laid out by hand here and enciphered under
//! a synthetic white-box key, so no game byte is involved.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use flate2::write::ZlibEncoder;
use flate2::Compression;
use or_io::container::rpf::gta6_chunk_table::{author, Chunk, Table as ChunkTable};
use or_io::container::rpf::{
    Gta6Mount, GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH,
    GROUP_ENTRY_ROW, GROUP_NAME32, GROUP_NAME64, RPF9_MAGIC,
};
use or_io::gta6::hash::{name_hash32, name_hash64};
use or_project::gta6::{
    build, conflicts, revert, Gta6Edit, Gta6Op, Gta6Project, Gta6ProjectError, GTA6_FORMAT,
    GTA6_MANIFEST,
};
use sha2::{Digest, Sha256};

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

/// A clean synthetic white-box snapshot holding the bootstrap key and one archive key.
fn keys() -> GtaViKeys {
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
    GtaViKeys::parse(&out).expect("a clean synthetic snapshot parses")
}

/// `body` as 64 KiB zlib chunks, each 16-aligned, and the chunks a chunk table states.
fn chunk_body(body: &[u8]) -> (Vec<u8>, Vec<Chunk>) {
    let mut out = Vec::new();
    let mut chunks = Vec::new();
    for piece in body.chunks(0x10000) {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(piece).unwrap();
        let deflated = encoder.finish().unwrap();
        chunks.push(Chunk {
            inflated: piece.len() as u32,
            stored: deflated.len() as u32,
            zlib: true,
        });
        out.extend_from_slice(&deflated);
        out.resize(round16(out.len()), 0);
    }
    (out, chunks)
}

/// An archive laid out by hand: a header, four descriptors in 0x150 bytes of metadata, the columns
/// row24, name64, name32 and `0xe0c1` in the main region and `0xb6f5` external, bodies `GAP` apart,
/// and a names heap. A `true` entry is chunked, a `false` one raw.
fn archive(entries: &[(&str, bool, Vec<u8>)]) -> Vec<u8> {
    let n = entries.len();
    let mut stored = Vec::new();
    let mut slices = Vec::new();
    let mut chunk_slot = Vec::new();
    for (_, chunked, b) in entries {
        if *chunked {
            let (s, chunks) = chunk_body(b);
            chunk_slot.push(slices.len() as u8);
            slices.push(author(&ChunkTable { mode: 3, chunks }).unwrap());
            stored.push((0x0Bu64, s, b.len() as u64));
        } else {
            chunk_slot.push(0xFF);
            let mut s = b.clone();
            s.resize(round16(s.len()), 0);
            stored.push((0xFF, s, b.len() as u64));
        }
    }
    let groups_start = 0x1B0;
    let main_index = groups_start + 5 * 24;
    let main_start = round8(main_index + 4 * n);
    let (name64, name32, e0c1) = (24 * n, 32 * n, round8(36 * n));
    let heap = round16(main_start + e0c1 + 8 * slices.len());
    let heap_len: usize = slices.iter().map(|(_, w)| 8 * w.len()).sum();
    let split = round16(heap + heap_len);
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
        (GROUP_CHUNK_RECORD, slices.len(), e0c1, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        put64(&mut t, at, hash);
        put32(&mut t, at + 8, count as u32);
        put32(&mut t, at + 12, offset as u32);
        put32(&mut t, at + 16, width | (external << 16));
    }

    let mut at = round16(toc_end + GAP as usize) as u64;
    let mut starts = Vec::new();
    for (i, ((name, _, _), (class, s, real))) in entries.iter().zip(&stored).enumerate() {
        let row = main_start + 24 * i;
        put64(&mut t, row, (class << 40) | (at / 16));
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
        put64(&mut t, ext_start + 8 * i, 0x1111_0000 + i as u64);
        t[main_index + 4 * i..main_index + 4 * i + 3].fill(i as u8);
        t[main_index + 4 * i + 3] = chunk_slot[i];
        t[split + i] = i as u8;
        starts.push(at);
        at += s.len() as u64 + GAP;
    }
    let mut h = heap;
    for (k, (u1, words)) in slices.iter().enumerate() {
        let cell = main_start + e0c1 + 8 * k;
        put32(&mut t, cell, (h - cell) as u32);
        put32(&mut t, cell + 4, *u1);
        for w in words {
            put64(&mut t, h, *w);
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

fn beta() -> Vec<u8> {
    text("beta chunked text ", 150_000)
}

fn sample() -> Vec<u8> {
    archive(&[
        ("alpha.bin", false, b"alpha body, stored raw".to_vec()),
        ("beta.txt", true, beta()),
        ("gamma.bin", false, vec![0x47; 40]),
    ])
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn edit(archive: &str, entry: &str, op: Gta6Op) -> Gta6Edit {
    Gta6Edit {
        archive: PathBuf::from(archive),
        within: Vec::new(),
        entry: entry.to_owned(),
        op,
    }
}

/// Every file under `dir` with its bytes, by path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(snapshot(&path));
        } else {
            files.push((path.clone(), std::fs::read(&path).unwrap()));
        }
    }
    files.sort();
    files
}

#[test]
fn a_build_applies_two_edits_to_a_copy_and_never_writes_the_install() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("install");
    std::fs::create_dir_all(install.join("x64/data")).unwrap();
    std::fs::write(install.join("x64/data/one.rpf"), sample()).unwrap();
    std::fs::write(install.join("x64/data/two.rpf"), sample()).unwrap();
    let new_beta = text("other text ", 70_000);
    std::fs::write(dir.path().join("beta.txt"), &new_beta).unwrap();
    let before = snapshot(&install);

    let gamma_hash = format!("{:#018x}", name_hash64(b"gamma.bin"));
    let project = Gta6Project {
        name: "first mod".into(),
        edits: vec![
            edit(
                "x64/data/one.rpf",
                "beta.txt",
                Gta6Op::Replace {
                    with: dir.path().join("beta.txt"),
                },
            ),
            edit(
                "x64/data/one.rpf",
                &gamma_hash,
                Gta6Op::Rename {
                    name: "renamed/gamma.bin".into(),
                },
            ),
        ],
    };
    let out = dir.path().join("out");
    let keys = keys();
    let manifest = build(&project, &install, &out, &keys).expect("the project builds");

    assert_eq!(snapshot(&install), before, "the install is byte-identical");
    assert!(
        !out.join("x64/data/two.rpf").exists(),
        "an untouched archive is not copied"
    );
    let written = std::fs::read(out.join("x64/data/one.rpf")).unwrap();
    let mount = Gta6Mount::open_root(&mut Cursor::new(&written[..]), &keys).unwrap();
    let entries = &mount.archive.entries;
    assert_eq!(
        mount
            .read_entry(Cursor::new(&written[..]), &entries[1])
            .unwrap(),
        new_beta
    );
    assert_eq!(entries[2].name64, name_hash64(b"renamed/gamma.bin"));
    assert_eq!(entries[0].name64, name_hash64(b"alpha.bin"));

    assert_eq!(manifest.changes.len(), 2);
    assert_eq!(manifest.tables_changed, 1);
    let (replaced, renamed) = (&manifest.changes[0], &manifest.changes[1]);
    assert_eq!(
        replaced.entry,
        format!("{:#018x}", name_hash64(b"beta.txt"))
    );
    assert_eq!(replaced.old, sha(&beta()));
    assert_eq!(replaced.new, Some(sha(&new_beta)));
    assert_eq!(renamed.entry, gamma_hash);
    assert_eq!(renamed.old, sha(&[0x47; 40]));
    assert_eq!(renamed.new.as_ref(), Some(&renamed.old));
    assert_eq!(
        renamed.renamed_to,
        Some(format!("{:#018x}", name_hash64(b"renamed/gamma.bin")))
    );
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join(GTA6_MANIFEST)).unwrap()).unwrap();
    assert_eq!(on_disk["changes"][0]["new"], sha(&new_beta).as_str());

    // A second build into the same folder is refused; revert deletes it, and only a build output.
    assert!(matches!(
        build(&project, &install, &out, &keys),
        Err(Gta6ProjectError::OutputExists(_))
    ));
    assert!(matches!(
        revert(&install),
        Err(Gta6ProjectError::NotABuild(_))
    ));
    revert(&out).unwrap();
    assert!(!out.exists());
    assert!(matches!(
        build(&project, &install, &install.join("out"), &keys),
        Err(Gta6ProjectError::OutputInInstall(_))
    ));
    assert_eq!(snapshot(&install), before);
}

#[test]
fn two_projects_editing_one_entry_conflict_by_name() {
    let beta_hash = format!("{:#018x}", name_hash64(b"beta.txt"));
    let first = Gta6Project {
        name: "first mod".into(),
        edits: vec![
            edit("x64/data/one.rpf", "beta.txt", Gta6Op::Remove),
            edit("x64/data/one.rpf", "alpha.bin", Gta6Op::Remove),
        ],
    };
    let second = Gta6Project {
        name: "second mod".into(),
        edits: vec![
            edit(
                "X64\\Data\\ONE.rpf",
                &beta_hash,
                Gta6Op::Rename {
                    name: "other.txt".into(),
                },
            ),
            edit("x64/data/two.rpf", "alpha.bin", Gta6Op::Remove),
        ],
    };
    let found = conflicts(&[first.clone(), second]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        found[0].projects,
        ["first mod".to_owned(), "second mod".to_owned()]
    );
    assert_eq!(found[0].entry, beta_hash);
    assert!(found[0].to_string().contains("first mod and second mod"));
    assert!(conflicts(std::slice::from_ref(&first)).is_empty());

    // One project editing an entry twice is refused before anything is read.
    let mut twice = first;
    twice
        .edits
        .push(edit("x64/data/one.rpf", "BETA.TXT", Gta6Op::Remove));
    let dir = tempfile::tempdir().unwrap();
    let refused = build(&twice, dir.path(), &dir.path().join("out"), &keys());
    assert!(matches!(refused, Err(Gta6ProjectError::Conflicts(c)) if c.len() == 1));
}

#[test]
fn a_project_file_reads_with_its_paths_beside_it_and_refuses_what_it_cannot_do() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mod.json");
    let file =
        |edits: &str| format!(r#"{{"format": "{GTA6_FORMAT}", "name": "m", "edits": [{edits}]}}"#);
    std::fs::write(
        &path,
        file(r#"{"archive": "a.rpf", "entry": "b", "op": "replace", "with": "new.bin"},
                {"archive": "a.rpf", "within": ["c.rpf"], "entry": "d", "op": "rename", "name": "e"}"#),
    )
    .unwrap();
    let project = Gta6Project::load(&path).unwrap();
    assert_eq!(
        project.edits[0].op,
        Gta6Op::Replace {
            with: dir.path().join("new.bin")
        }
    );
    assert_eq!(project.edits[1].within, ["c.rpf"]);

    std::fs::write(
        &path,
        file(r#"{"archive": "a.rpf", "entry": "b", "op": "add"}"#),
    )
    .unwrap();
    assert!(matches!(
        Gta6Project::load(&path),
        Err(Gta6ProjectError::NotAProject { .. })
    ));
    std::fs::write(&path, r#"{"name": "m", "edits": []}"#).unwrap();
    assert!(matches!(
        Gta6Project::load(&path),
        Err(Gta6ProjectError::NotAProject { .. })
    ));
}
