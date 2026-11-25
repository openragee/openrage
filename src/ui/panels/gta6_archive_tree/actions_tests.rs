//! Hermetic checks of the row actions, on an archive built here and enciphered under a synthetic
//! white-box key, so no game byte is involved.

use std::io::Cursor;

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::Rpf9Form;
use or_io::gta6::hash::name_hash64;

use super::*;

const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
const KEY_ID: u32 = 0xDEAD_BEEF;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
/// The round tables feeding each output lane of the cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
const WORDS: usize = 60;
const MAPS: usize = 16;
const MAP_LEN: usize = 256;
const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;

const MEDIA: &str = "data/notes.txt";
const RESOURCE: &str = "maps/a.ydr";

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

/// A synthetic white-box snapshot holding the bootstrap key and one archive key.
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
    GtaViKeys::parse(&out).expect("the synthetic snapshot parses")
}

/// A loose resource of a block-mapped virtual section of 0x200 bytes and no physical one.
fn loose() -> Vec<u8> {
    let mut v: Vec<u8> = (0..0x200)
        .map(|i| ((i * 13 + 2) % 251) as u8 | 0x80)
        .collect();
    v[..8].fill(0);
    v[8..16].copy_from_slice(&0x5000_0040u64.to_le_bytes());
    v[0x40..0x70].fill(0);
    write_rpf9_resource(&Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: v,
        physical: Vec::new(),
    })
    .expect("a loose resource")
}

fn media() -> Vec<u8> {
    (0..0x3000).map(|i| (i % 97) as u8).collect()
}

/// An archive of a chunked text body under a 24-byte row and a drawable under a 20-byte row.
fn source(dir: &Path) -> PathBuf {
    let entries = vec![
        (MEDIA.to_owned(), Rpf9NewBody::Chunked(media())),
        (
            RESOURCE.to_owned(),
            Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: loose(),
            },
        ),
    ];
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let plan = build(&entries, &key, &keys()).expect("the archive builds");
    let mut bytes = Vec::new();
    plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)
        .expect("the archive is laid out");
    let path = dir.join("source.rpf");
    std::fs::write(&path, bytes).expect("the archive is written");
    path
}

/// The row of the top-level entry named `name` in the archive at `disk`.
fn row(disk: &Path, name: &str) -> Row {
    let mut file = File::open(disk).expect("the archive");
    let mount = Gta6Mount::open_root(&mut file, &keys()).expect("the archive opens");
    let entry = (mount.archive.entries.iter())
        .find(|e| e.name64 == name_hash64(name.as_bytes()))
        .unwrap_or_else(|| panic!("no entry {name}"));
    Row {
        disk: disk.to_path_buf(),
        node: Vec::new(),
        entry: entry.index,
        name: name.to_owned(),
    }
}

#[test]
fn each_row_of_a_table_is_labelled_by_its_class_or_its_type_code() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let kinds = kinds(&disk, 0, &keys()).expect("the table opens");
    let mut by_row = [
        (row(&disk, MEDIA).entry, "media"),
        (row(&disk, RESOURCE).entry, "drawable"),
    ];
    by_row.sort();
    let want: Vec<&str> = by_row.iter().map(|(_, k)| *k).collect();
    assert_eq!(kinds, want);
}

#[test]
fn a_row_extracts_its_body_and_a_headerless_one_its_loose_form() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let keys = keys();

    let out = dir.path().join("notes.txt");
    extract(&row(&disk, MEDIA), &out, &keys).expect("the body is extracted");
    assert_eq!(std::fs::read(&out).expect("written"), media());

    let out = dir.path().join("a.ydr");
    extract(&row(&disk, RESOURCE), &out, &keys).expect("the resource is extracted");
    assert_eq!(std::fs::read(&out).expect("written"), loose());

    let over = extract(&row(&disk, MEDIA), &disk, &keys).expect_err("over the archive");
    assert!(over.contains("never written"), "{over}");
}

#[test]
fn a_row_replaced_with_a_file_is_written_into_a_new_archive_that_reads_it_back() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let disk = source(dir.path());
    let before = std::fs::read(&disk).expect("the source");
    let keys = keys();

    let with = dir.path().join("new.txt");
    // A text body long enough to be chunked, which the reader reads back by what it holds.
    let new = "a new body for the entry ".repeat(3_000).into_bytes();
    std::fs::write(&with, &new).expect("written");
    let out = dir.path().join("copy.rpf");
    assert_eq!(
        replace(&row(&disk, MEDIA), &with, &out, &keys),
        Ok((true, true))
    );
    let copy = row(&out, MEDIA);
    let mut file = File::open(&out).expect("the copy");
    let mount = Gta6Mount::open_root(&mut file, &keys).expect("the copy opens");
    let entry = &mount.archive.entries[copy.entry as usize];
    let body = mount.read_entry(&mut file, entry).expect("the body reads");
    assert_eq!(body, new);
    assert_eq!(
        std::fs::read(&disk).expect("the source"),
        before,
        "the source is kept"
    );

    // The resource's own loose form, extracted then put back, changes nothing.
    let resource = dir.path().join("a.ydr");
    extract(&row(&disk, RESOURCE), &resource, &keys).expect("the resource is extracted");
    let same = dir.path().join("same.rpf");
    assert_eq!(
        replace(&row(&disk, RESOURCE), &resource, &same, &keys),
        Ok((false, false))
    );

    // A copy is only ever a new file: neither the source nor one that exists is written.
    let over = replace(&row(&disk, MEDIA), &with, &disk, &keys).expect_err("over the source");
    assert!(over.contains("never written"), "{over}");
    assert!(replace(&row(&disk, MEDIA), &with, &out, &keys).is_err());
    assert_eq!(std::fs::read(&disk).expect("the source"), before);
}
