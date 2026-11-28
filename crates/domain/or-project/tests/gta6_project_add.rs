//! Hermetic checks of a GTA VI project's `add` edit on archives the container writer builds from
//! nothing, enciphered under a synthetic white-box key, so no game byte is involved.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_container_write::{
    build as build_archive, Rpf9IndexKey, Rpf9NewBody,
};
use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys};
use or_io::gta6::hash::name_hash64;
use or_project::gta6::{
    build, Gta6Edit, Gta6Op, Gta6Project, Gta6ProjectError, GTA6_FORMAT, GTA6_MANIFEST,
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

/// An archive of `entries`, each stored chunked or, beginning with an RPF9 header, as a nested one.
fn archive(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let entries: Vec<(String, Rpf9NewBody)> = (entries.iter())
        .map(|(name, bytes)| {
            let body = if bytes.starts_with(b"9FPR") {
                Rpf9NewBody::Archive(bytes.clone())
            } else {
                Rpf9NewBody::Chunked(bytes.clone())
            };
            ((*name).to_owned(), body)
        })
        .collect();
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let plan = build_archive(&entries, or_io::Platform::XboxSeries, &key, &keys())
        .expect("the fixture builds");
    let mut out = Vec::new();
    plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut out)
        .expect("the fixture writes");
    out
}

fn text(seed: &str, len: usize) -> Vec<u8> {
    seed.bytes().cycle().take(len).collect()
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn add(archive: &str, within: &[&str], entry: &str, with: &Path) -> Gta6Edit {
    Gta6Edit {
        archive: PathBuf::from(archive),
        within: within.iter().map(|w| (*w).to_owned()).collect(),
        entry: entry.to_owned(),
        op: Gta6Op::Add {
            with: with.to_path_buf(),
            type_code: None,
        },
    }
}

fn named<'a>(mount: &'a Gta6Mount, name: &str) -> &'a Gta6Entry {
    let hash = name_hash64(name.as_bytes());
    (mount.archive.entries.iter())
        .find(|e| e.name64 == hash)
        .unwrap_or_else(|| panic!("{name} is listed"))
}

#[test]
fn an_add_puts_a_new_entry_into_a_copy_at_the_top_level_and_inside_a_nested_archive() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("install");
    std::fs::create_dir_all(install.join("x64")).unwrap();
    let child = archive(&[("inner.txt", text("inner text ", 3_000))]);
    let source = archive(&[("alpha.bin", text("alpha ", 500)), ("child.rpf", child)]);
    std::fs::write(install.join("x64/one.rpf"), &source).unwrap();
    let (top_body, nested_body) = (text("added text ", 90_000), text("nested add ", 5_000));
    std::fs::write(dir.path().join("top.txt"), &top_body).unwrap();
    std::fs::write(dir.path().join("nested.bin"), &nested_body).unwrap();

    let project = Gta6Project {
        name: "adding mod".into(),
        edits: vec![
            add(
                "x64/one.rpf",
                &[],
                "added/top.txt",
                &dir.path().join("top.txt"),
            ),
            add(
                "x64/one.rpf",
                &["child.rpf"],
                "inner/added.bin",
                &dir.path().join("nested.bin"),
            ),
        ],
    };
    let out = dir.path().join("out");
    let keys = keys();
    let manifest = build(&project, &install, &out, &keys).expect("the project builds");
    assert_eq!(std::fs::read(install.join("x64/one.rpf")).unwrap(), source);

    let written = std::fs::read(out.join("x64/one.rpf")).unwrap();
    let mount = Gta6Mount::open_root(&mut Cursor::new(&written[..]), &keys).unwrap();
    assert_eq!(mount.archive.entries.len(), 3);
    let read = |mount: &Gta6Mount, name: &str| {
        mount
            .read_entry(Cursor::new(&written[..]), named(mount, name))
            .expect("the entry reads")
    };
    assert_eq!(read(&mount, "added/top.txt"), top_body);
    assert_eq!(read(&mount, "alpha.bin"), text("alpha ", 500));
    let nested = mount
        .open_nested(
            &mut Cursor::new(&written[..]),
            named(&mount, "child.rpf"),
            &keys,
        )
        .expect("the nested archive reopens");
    assert_eq!(read(&nested, "inner/added.bin"), nested_body);
    assert_eq!(read(&nested, "inner.txt"), text("inner text ", 3_000));

    assert_eq!(manifest.changes.len(), 2);
    let added = &manifest.changes[0];
    assert_eq!(
        (added.op.as_str(), added.old.as_str()),
        ("add", ""),
        "an added entry had no content before"
    );
    assert_eq!(added.new, Some(sha(&top_body)));
    assert_eq!(
        added.entry,
        format!("{:#018x}", name_hash64(b"added/top.txt"))
    );
    assert_eq!(manifest.changes[1].within, ["child.rpf"]);
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join(GTA6_MANIFEST)).unwrap()).unwrap();
    assert!(on_disk["changes"][0].get("old").is_none());
}

#[test]
fn an_add_of_a_name_taken_or_a_hash_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("install");
    std::fs::create_dir_all(&install).unwrap();
    std::fs::write(
        install.join("one.rpf"),
        archive(&[("alpha.bin", text("a", 64))]),
    )
    .unwrap();
    let with = dir.path().join("new.bin");
    std::fs::write(&with, b"new").unwrap();
    for entry in ["alpha.bin", "0x0123456789abcdef"] {
        let project = Gta6Project {
            name: "refused".into(),
            edits: vec![add("one.rpf", &[], entry, &with)],
        };
        let out = dir.path().join("out");
        let refused = build(&project, &install, &out, &keys());
        assert!(
            matches!(refused, Err(Gta6ProjectError::Archive { .. })),
            "{entry}: {refused:?}"
        );
        assert!(!out.exists(), "{entry}: a refused build leaves no folder");
    }
}

#[test]
fn an_add_reads_from_a_project_file_with_its_type_and_its_path_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mod.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"format": "{GTA6_FORMAT}", "name": "m", "edits": [
                {{"archive": "a.rpf", "entry": "maps/new.ydr", "op": "add", "with": "new.ydr", "type": 2}}
            ]}}"#
        ),
    )
    .unwrap();
    let project = Gta6Project::load(&path).unwrap();
    assert_eq!(
        project.edits[0].op,
        Gta6Op::Add {
            with: dir.path().join("new.ydr"),
            type_code: Some(2),
        }
    );
}
