//! Hermetic checks of `rpf9-diff` on archives laid out by hand here and enciphered under a synthetic
//! white-box key, so no game byte is involved.

use std::io::Cursor;

use super::*;
use or_io::container::rpf::{
    Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_NAME32,
    GROUP_NAME64, RPF9_MAGIC,
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
const HASH_BASE: u64 = 0x1111_0000;

fn push_perm(out: &mut Vec<u8>, k: usize) {
    for x in 0..256usize {
        out.push(((x + k * 7 + 1) % 256) as u8);
    }
}

fn mix8(b: u8) -> u8 {
    let (mut y, mut acc) = (0u8, 0u8);
    for i in (0..8).rev() {
        acc ^= (b >> i) & 1;
        y |= acc << i;
    }
    y
}

/// A synthetic white-box snapshot holding the bootstrap key and one archive key.
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

fn keys() -> GtaViKeys {
    GtaViKeys::parse(&snapshot()).expect("the synthetic snapshot parses")
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

enum Body {
    Raw(&'static [u8]),
    Nested(Vec<u8>),
}

/// An archive laid out by hand: a header, four metadata descriptors, the columns row24, name64,
/// name32 and an empty `0xe0c1` in the main region and `0xb6f5` external, bodies `GAP` apart, and a
/// names heap. A raw body is stored as media (class `0xFF`), a nested one as class `0x00`.
fn archive(entries: &[(&str, Body)]) -> Vec<u8> {
    let n = entries.len();
    let stored: Vec<(u64, Vec<u8>, u64)> = entries
        .iter()
        .map(|(_, body)| match body {
            Body::Raw(b) => {
                let mut s = b.to_vec();
                s.resize(round16(b.len()), 0);
                (0xFF, s, b.len() as u64)
            }
            Body::Nested(b) => (0x00, b.clone(), b.len() as u64),
        })
        .collect();
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
    let mut starts = Vec::new();
    for (i, ((name, _), (class, s, real))) in entries.iter().zip(&stored).enumerate() {
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
    for (&start, (_, s, _)) in starts.iter().zip(&stored) {
        file[start..start + s.len()].copy_from_slice(s);
    }
    file[at..].fill(0x5A);
    file
}

const NAMES: [&str; 9] = [
    "keep.bin",
    "gone.bin",
    "new.txt",
    "old.txt",
    "moved.txt",
    "edit.txt",
    "child.rpf",
    "inner.bin",
    "level.bin",
];

fn resolver() -> NameResolver {
    let mut resolver = NameResolver::new();
    for name in NAMES {
        resolver.insert(name_hash64(name.as_bytes()), name);
    }
    resolver
}

fn diff(a: &[u8], b: &[u8]) -> Vec<Change> {
    let (mut ca, mut cb) = (Cursor::new(a), Cursor::new(b));
    diff_archives(&mut ca, &mut cb, &keys(), &resolver()).expect("the archives compare")
}

/// A and B differ by one entry of each kind; B lists its entries in another order.
fn pair() -> (Vec<u8>, Vec<u8>) {
    let a = archive(&[
        ("keep.bin", Body::Raw(b"kept as it was")),
        ("gone.bin", Body::Raw(b"gone")),
        ("old.txt", Body::Raw(b"renamed body")),
        ("edit.txt", Body::Raw(b"line one\nline two\n")),
        (
            "child.rpf",
            Body::Nested(archive(&[
                ("inner.bin", Body::Raw(b"x")),
                ("level.bin", Body::Raw(b"same inside")),
            ])),
        ),
    ]);
    let b = archive(&[
        (
            "child.rpf",
            Body::Nested(archive(&[
                ("inner.bin", Body::Raw(b"y")),
                ("level.bin", Body::Raw(b"same inside")),
            ])),
        ),
        ("new.txt", Body::Raw(b"fresh")),
        ("edit.txt", Body::Raw(b"line one\nline 2\n")),
        ("moved.txt", Body::Raw(b"renamed body")),
        ("keep.bin", Body::Raw(b"kept as it was")),
    ]);
    (a, b)
}

#[test]
fn an_archive_against_itself_has_no_change() {
    let (a, _) = pair();
    assert!(diff(&a, &a).is_empty());
    assert_eq!(render("a", "b", &[]), "a and b: no entry differs\n");
}

#[test]
fn each_change_kind_is_found_by_name_not_position() {
    let (a, b) = pair();
    let changes = diff(&a, &b);
    let lines = |added: &[&str], removed: &[&str]| {
        Some(Summary::Lines {
            added: added.iter().map(|s| s.to_string()).collect(),
            removed: removed.iter().map(|s| s.to_string()).collect(),
        })
    };
    let summary = |c: &Change| match c {
        Change::Changed { summary, .. } => summary.clone(),
        _ => None,
    };
    let paths: Vec<(&str, String)> = changes
        .iter()
        .map(|c| match c {
            Change::Added { path, .. } => ("added", path.clone()),
            Change::Removed { path, .. } => ("removed", path.clone()),
            Change::Renamed { from, to, .. } => ("renamed", format!("{from} -> {to}")),
            Change::Changed { path, .. } => ("changed", path.clone()),
        })
        .collect();
    assert_eq!(
        paths,
        vec![
            ("changed", "edit.txt".to_string()),
            ("changed", "child.rpf".to_string()),
            ("changed", "child.rpf/inner.bin".to_string()),
            ("removed", "gone.bin".to_string()),
            ("renamed", "old.txt -> moved.txt".to_string()),
            ("added", "new.txt".to_string()),
        ]
    );
    assert_eq!(summary(&changes[0]), lines(&["line 2"], &["line two"]));
    assert_eq!(summary(&changes[1]), Some(Summary::Nested { changes: 1 }));
    assert_eq!(summary(&changes[2]), lines(&["y"], &["x"]));
    assert_eq!(
        changes[5],
        Change::Added {
            path: "new.txt".into(),
            size: Size {
                stored: Some(16),
                real: Some(5),
            },
        }
    );
}

#[test]
fn a_same_sized_body_with_other_bytes_is_no_rename() {
    // "gone" and "fresh" are both stored in 16 bytes; only the bytes tell them apart.
    let a = archive(&[("gone.bin", Body::Raw(b"gone"))]);
    let b = archive(&[("new.txt", Body::Raw(b"fresh"))]);
    let changes = diff(&a, &b);
    assert_eq!(changes.len(), 2);
    assert!(matches!(&changes[0], Change::Removed { path, .. } if path == "gone.bin"));
    assert!(matches!(&changes[1], Change::Added { path, .. } if path == "new.txt"));
}

#[test]
fn the_text_report_and_the_json_name_each_change() {
    let (a, b) = pair();
    let changes = diff(&a, &b);
    let text = render("a.rpf", "b.rpf", &changes);
    assert!(text.starts_with("a.rpf -> b.rpf: 1 added, 1 removed, 1 renamed, 3 changed\n"));
    for line in [
        "  + new.txt  stored 16, real 5",
        "  - gone.bin",
        "  > old.txt -> moved.txt",
        "  ~ edit.txt  stored 32, real 18 -> stored 16, real 16",
        "      text: 1 line(s) added, 1 removed",
        "        + line 2",
        "        - line two",
        "      nested archive: 1 change(s) inside",
        "  ~ child.rpf/inner.bin",
    ] {
        assert!(text.contains(line), "{line:?} missing from\n{text}");
    }
    let json = serde_json::to_value(&changes).expect("the changes serialise");
    assert_eq!(json[0]["kind"], "changed");
    assert_eq!(json[0]["summary"]["type"], "lines");
    assert_eq!(json[1]["summary"]["changes"], 1);
    assert_eq!(json[4]["kind"], "renamed");
    assert_eq!(json[4]["to"], "moved.txt");
    assert_eq!(json[5]["size"]["stored"], 16);
}

#[test]
fn lines_are_counted_as_a_multiset() {
    assert_eq!(
        line_diff(b"x\nx\ny\n", b"x\ny\nz\n"),
        Summary::Lines {
            added: vec!["z".into()],
            removed: vec!["x".into()],
        }
    );
}

#[test]
fn fields_are_compared_by_path() {
    let mut a = MetaInstance::new("CThing");
    a.set("count", Value::Int(1));
    a.set("label", Value::String("x".into()));
    a.set("list", Value::Array(vec![Value::UInt(7)]));
    let mut b = MetaInstance::new("CThing");
    b.set("count", Value::Int(2));
    b.set("list", Value::Array(vec![Value::UInt(7), Value::UInt(8)]));
    b.set("flag", Value::Bool(true));
    assert_eq!(
        field_diff("PSO", &a, &b),
        Summary::Fields {
            format: "PSO",
            added: vec!["CThing.flag = true".into(), "CThing.list[1] = 8".into()],
            removed: vec!["CThing.label = \"x\"".into()],
            changed: vec![FieldChange {
                path: "CThing.count".into(),
                before: "1".into(),
                after: "2".into(),
            }],
        }
    );
}

#[test]
fn a_body_is_summarised_by_its_head() {
    assert_eq!(head_kind(b"PSIN...."), Some(Head::Pso));
    assert_eq!(head_kind(b"RBF0...."), Some(Head::Rbf));
    assert_eq!(head_kind(b"<?xml version"), Some(Head::Text));
    assert_eq!(head_kind(b"plain list\n"), Some(Head::Text));
    assert_eq!(head_kind(&[0x00, 0x01, 0xFE]), None);
    assert_eq!(head_kind(b""), None);
    // A metadata head goes to the metadata reader, never to the line diff, even when it is printable.
    let meta = summarize(b"RBF0 one", b"RBF0 two");
    assert!(
        matches!(
            meta,
            Some(Summary::Fields { format: "RBF", .. } | Summary::Unread { .. })
        ),
        "{meta:?}"
    );
    assert_eq!(
        summarize(b"PSIN", b"text"),
        None,
        "two kinds are not compared"
    );
    assert_eq!(
        summarize(&[0, 1], &[0, 2]),
        None,
        "an unknown head is not read"
    );
}
