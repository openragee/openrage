use super::super::tests::keys;
use super::*;
use or_io::container::rpf::{Rpf9Builder, Rpf9Toc, GROUP_ENTRY_ROW_20};
use or_io::gta6::headerless::Resource;

fn empty_drawable() -> Vec<u8> {
    let mut bytes = vec![0; 0x180];
    for (at, target) in [(0x08, 0xE0), (0x10, 0xF0), (0x50, 0x130), (0xA8, 0x140)] {
        bytes[at..at + 8].copy_from_slice(&(0x5000_0000u64 + target).to_le_bytes());
    }
    bytes[0xE8] = 1;
    bytes[0x7C..0x80].copy_from_slice(&100.0f32.to_le_bytes());
    bytes[0x140..0x14B].copy_from_slice(b"preview#dr\0");
    bytes
}

pub(super) fn preview_archive(
    bytes: &[u8],
    resource_type: u32,
) -> (tempfile::TempDir, Rpf9TableDocument) {
    let count = u32::try_from(bytes.len()).unwrap();
    let encrypted = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: or_test_support::rpf9::resource_table(&[(count, count, false)]),
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys(),
    )
    .unwrap();
    let mut toc = Rpf9Toc::decrypt(&encrypted, &keys()).unwrap();
    let row = toc
        .groups
        .iter()
        .find(|group| group.name_hash == GROUP_ENTRY_ROW_20)
        .unwrap()
        .data
        .start;
    for (offset, value) in [(0, count | 11), (4, 1), (16, resource_type)] {
        toc.bytes[row + offset..row + offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut archive = Rpf9Builder::serialize(&toc, &keys()).unwrap();
    archive.resize(0x1010, 0);
    archive.extend(bytes);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.rpf");
    std::fs::write(&path, archive).unwrap();
    let table = Rpf9TableDocument::open_using(&path, || Ok(keys()))
        .unwrap()
        .unwrap();
    (dir, table)
}

#[test]
fn a_table_drawable_opens_in_the_gta_vi_preview_and_keeps_its_bytes() {
    let bytes = empty_drawable();
    let (dir, table) = preview_archive(&bytes, 2);
    let source = table.entry(0, 0).unwrap();
    let payload = source.payload_limited(bytes.len() as u64).unwrap();
    let DocPayload::Gta6Drawable(resource) = &payload else {
        panic!("the table drawable must open in GTA VI's preview");
    };
    let Resource::Drawable(drawable) = &**resource else {
        panic!("the table type must select the drawable reader");
    };
    assert_eq!(drawable.name, "preview#dr");
    assert_eq!(drawable.levels[0].as_ref().unwrap().len(), 0);
    assert_eq!(source.path.extension().unwrap(), "ydr");
    assert_eq!(address(&source.path).unwrap().1, [0]);
    let mut output = Vec::new();
    assert_eq!(source.extract(&mut output).unwrap(), bytes.len() as u64);
    assert_eq!(output, bytes);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(!source.path.exists());
}

#[test]
fn table_preview_refuses_a_bad_split_wrong_root_and_a_read_past_its_budget() {
    let bytes = empty_drawable();
    let (_dir, table) = preview_archive(&bytes, 2);
    let source = table.entry(0, 0).unwrap();
    assert!(source.payload_limited(bytes.len() as u64 - 1).is_err());
    let mut wrong_size = source.clone();
    wrong_size.extra.nested.as_mut().unwrap().virtual_size += 16;
    assert!(
        wrong_size.payload().is_err(),
        "the split must cover exactly the entry"
    );
    for resource_type in [4, 73, 74] {
        let mut wrong_root = source.clone();
        wrong_root.extra.nested.as_mut().unwrap().resource_type = resource_type;
        assert!(
            wrong_root.payload().is_err(),
            "type {resource_type} must refuse a drawable root"
        );
    }
    assert!(table.tree().errors.contains_key(&(0, 0)));
    assert_eq!(table.tree().nodes.len(), 1);
}
