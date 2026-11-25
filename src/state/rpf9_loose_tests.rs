use std::sync::Arc;

use or_io::container::rpf::{GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_ENTRY_ROW_20, GROUP_NAME64};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::Resource;
use or_job::Progress;

use super::*;
use crate::state::document::rpf9::Rpf9TableDocument;
use crate::state::rpf9_texture_index::Root;

fn raw() -> Vec<u8> {
    let mut bytes = vec![0; 0x180];
    for (at, target) in [(0x08, 0xE0), (0x10, 0x100), (0x50, 0x130), (0xA8, 0x140)] {
        bytes[at..at + 8].copy_from_slice(&(0x5000_0000u64 + target).to_le_bytes());
    }
    bytes[0xE8] = 1;
    bytes[0x7C..0x80].copy_from_slice(&100.0f32.to_le_bytes());
    let name = b"preview.#dr\0";
    bytes[0x140..0x140 + name.len()].copy_from_slice(name);
    bytes
}

fn archive(
    bytes: &[u8],
    hash: u64,
    virtual_len: u32,
) -> (tempfile::TempDir, Arc<Rpf9TableDocument>) {
    archive_with_compression(bytes, hash, virtual_len, false)
}

fn archive_with_compression(
    bytes: &[u8],
    hash: u64,
    virtual_len: u32,
    compressed: bool,
) -> (tempfile::TempDir, Arc<Rpf9TableDocument>) {
    use std::io::Write;

    let stored = if compressed {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    } else {
        bytes.to_vec()
    };
    let keys = GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap();
    let count = u32::try_from(bytes.len()).unwrap();
    let encrypted = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: or_test_support::rpf9::resource_table(&[(
                count,
                u32::try_from(stored.len()).unwrap(),
                compressed,
            )]),
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys,
    )
    .unwrap();
    let mut toc = Rpf9Toc::decrypt(&encrypted, &keys).unwrap();
    let row = toc
        .groups
        .iter()
        .find(|g| g.name_hash == GROUP_ENTRY_ROW_20)
        .unwrap()
        .data
        .start;
    for (offset, value) in [
        (0, virtual_len | 11),
        (4, (count - virtual_len) | 1),
        (16, 2),
    ] {
        toc.bytes[row + offset..row + offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let name = toc
        .groups
        .iter()
        .find(|g| g.name_hash == GROUP_NAME64)
        .unwrap()
        .data
        .start;
    toc.bytes[name..name + 8].copy_from_slice(&hash.to_le_bytes());
    let mut file = Rpf9Builder::serialize(&toc, &keys).unwrap();
    file.resize(0x1010, 0);
    file.extend(stored);
    file.resize(file.len().next_multiple_of(16), 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.rpf");
    std::fs::write(&path, file).unwrap();
    let table = Rpf9TableDocument::open_using(&path, || Ok(keys))
        .unwrap()
        .unwrap();
    (dir, Arc::new(table))
}

fn tables(roots: Vec<Arc<Rpf9TableDocument>>) -> Tables {
    Tables::open(
        roots.into_iter().map(Root::Mounted).collect(),
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap()
}

#[test]
fn a_renamed_loose_drawable_opens_from_its_exact_table_entry_without_writing_a_file() {
    let bytes = raw();
    let (dir, table) = archive(&bytes, name_hash64(b"preview"), bytes.len() as u32);
    let catalog = tables(vec![table]);
    let renamed = dir.path().join("any_name.ydr");
    let opened = open(&renamed, &bytes, &catalog, &Cancel::new())
        .unwrap()
        .unwrap();
    let DocPayload::Gta6Drawable(resource) = &opened.payload else {
        panic!("a matching GTA VI drawable must use its own preview");
    };
    let Resource::Drawable(drawable) = &**resource else {
        panic!("not a drawable")
    };
    assert_eq!(drawable.name, "preview.#dr");
    assert_eq!(opened.source.bytes().unwrap(), bytes);
    assert!(crate::state::gta6_head::head_of(&renamed).is_some());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(!renamed.exists());
}

#[test]
fn a_matching_hash_is_insufficient_and_an_unmounted_body_is_never_searched() {
    let bytes = raw();
    let (dir, table) = archive(&bytes, name_hash64(b"preview"), bytes.len() as u32);
    let catalog = tables(vec![table]);
    let path = dir.path().join("preview.ydr");
    let mut changed = bytes.clone();
    changed[0x170] = 1;
    assert!(open(&path, &changed, &catalog, &Cancel::new()).is_err());
    let missing = Tables::default();
    assert!(open(&path, &bytes, &missing, &Cancel::new()).is_err());
    let (_dir, other) = archive(&bytes, name_hash64(b"another"), bytes.len() as u32);
    assert!(open(&path, &bytes, &tables(vec![other]), &Cancel::new()).is_err());
}

#[test]
fn identical_bytes_with_conflicting_table_splits_are_refused() {
    let bytes = raw();
    let (dir, first) = archive(&bytes, name_hash64(b"preview"), bytes.len() as u32);
    let (_other_dir, second) = archive(&bytes, name_hash64(b"preview"), bytes.len() as u32 - 16);
    let catalog = tables(vec![first, second]);
    let result = open(
        &dir.path().join("renamed.ydr"),
        &bytes,
        &catalog,
        &Cancel::new(),
    );
    assert!(matches!(result, Err(error) if error.contains("disagree")));
}

#[test]
fn rsc7_stays_on_its_reader_and_cancellation_stops_a_table_match() {
    let bytes = raw();
    let (dir, table) = archive(&bytes, name_hash64(b"preview"), bytes.len() as u32);
    let catalog = tables(vec![table]);
    let path = dir.path().join("renamed.ydr");
    let mut rsc7 = bytes.clone();
    rsc7[..4].copy_from_slice(b"RSC7");
    assert!(open(&path, &rsc7, &catalog, &Cancel::new())
        .unwrap()
        .is_none());
    let cancelled = Cancel::new();
    cancelled.set();
    assert!(open(&path, &bytes, &catalog, &cancelled).is_err());
}

#[test]
fn an_archive_changed_after_cataloguing_cannot_supply_a_loose_drawables_split() {
    let bytes = raw();
    let (dir, table) = archive(&bytes, name_hash64(b"preview"), bytes.len() as u32);
    let path = table.path.clone();
    let catalog = tables(vec![table]);
    let mut changed = std::fs::read(&path).unwrap();
    changed.extend([0; 16]);
    std::fs::write(&path, changed).unwrap();
    let result = open(
        &dir.path().join("renamed.ydr"),
        &bytes,
        &catalog,
        &Cancel::new(),
    );
    assert!(matches!(result, Err(error) if error.contains("changed on disk")));
}

#[test]
fn a_loose_drawable_opens_when_compressed_storage_exceeds_its_decoded_length() {
    let bytes = raw();
    let (dir, table) =
        archive_with_compression(&bytes, name_hash64(b"preview"), bytes.len() as u32, true);
    let source = table.entry(0, 0).unwrap();
    assert!(source.stored_len() > source.decoded_len());
    assert!(
        source.bytes_limited(source.decoded_len()).is_err(),
        "the decoded length is not a stored-input budget"
    );
    let opened = open(
        &dir.path().join("renamed.ydr"),
        &bytes,
        &tables(vec![table]),
        &Cancel::new(),
    )
    .unwrap()
    .unwrap();
    assert!(matches!(opened.payload, DocPayload::Gta6Drawable(_)));
    assert_eq!(opened.source.bytes().unwrap(), bytes);
}
