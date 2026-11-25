use super::*;
use or_io::container::rpf::{GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_ENTRY_ROW_20};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{write_dictionary, DictionaryModel, ModelTexture};

fn encode(bytes: Vec<u8>, keys: &GtaViKeys) -> Vec<u8> {
    Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes,
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        keys,
    )
    .unwrap()
}

pub(crate) fn fixture(
    name: &str,
    nested: bool,
    corrupt_child: bool,
) -> (tempfile::TempDir, Arc<Rpf9TableDocument>) {
    let keys = GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap();
    let sections = write_dictionary(&DictionaryModel {
        textures: vec![ModelTexture::tiled(name, 64, 64, 71, &[vec![0x55; 2048]]).unwrap()],
        capacity: 1,
    })
    .unwrap();
    let body = [
        sections.virtual_section.as_slice(),
        sections.physical.as_slice(),
    ]
    .concat();
    let size = u32::try_from(body.len()).unwrap();
    let mut toc = Rpf9Toc::decrypt(
        &encode(
            or_test_support::rpf9::resource_table(&[(size, size, false)]),
            &keys,
        ),
        &keys,
    )
    .unwrap();
    let row = toc
        .groups
        .iter()
        .find(|group| group.name_hash == GROUP_ENTRY_ROW_20)
        .unwrap()
        .data
        .start;
    for (offset, value) in [
        (0, u32::try_from(sections.virtual_section.len()).unwrap()),
        (4, u32::try_from(sections.physical.len()).unwrap() | 3),
        (16, 5),
    ] {
        toc.bytes[row + offset..row + offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut bytes = Rpf9Builder::serialize(&toc, &keys).unwrap();
    bytes.resize(0x1010, 0);
    bytes.extend(body);
    if corrupt_child {
        bytes[..4].copy_from_slice(b"bad!");
    }
    if nested {
        let mut parent = encode(
            or_test_support::rpf9::table(&[(
                name_hash64(b"nested"),
                0x1000,
                bytes.len() as u64,
                0,
            )]),
            &keys,
        );
        parent.resize(0x1000, 0);
        parent.extend(bytes);
        bytes = parent;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("textures.rpf");
    std::fs::write(&path, bytes).unwrap();
    let table = Arc::new(
        Rpf9TableDocument::open_using(&path, || Ok(keys))
            .unwrap()
            .unwrap(),
    );
    (dir, table)
}

#[test]
fn table_texture_keys_keep_nested_addresses_and_candidates_without_output_files() {
    let (first, one) = fixture("table_external", true, false);
    let (second, two) = fixture("table_external", false, false);
    let key = name_hash64(b"table_external");
    let catalog = Tables::open(
        vec![
            Root::Mounted(Arc::clone(&one)),
            Root::Mounted(Arc::clone(&two)),
        ],
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();
    assert_eq!(
        one.tree().nodes.len(),
        1,
        "cataloging must not expand the UI tree"
    );
    one.expand_table(0, 0).unwrap();
    let entry_hash = one.tree().nodes[&0x1000].archive.entries[0].name64;
    assert_eq!(catalog.entries_named(entry_hash, "ytd").len(), 2);
    assert!(catalog.entries_named(entry_hash, "ydr").is_empty());
    let mut built = Index::new(catalog);
    assert_eq!(
        built.dictionaries(),
        0,
        "cataloging never decodes a texture dictionary"
    );
    assert!(built.extend_for(&HashSet::from([key]), &Cancel::new(), &Progress::new()));
    assert_eq!(
        built.dictionaries(),
        1,
        "stop once the first requested key is found"
    );
    assert!(!built.exhausted());
    let absent = name_hash64(b"absent_texture");
    assert!(built.extend_for(&HashSet::from([absent]), &Cancel::new(), &Progress::new()));
    assert!(built.exhausted());
    assert_eq!(built.dictionaries(), 2);
    assert!(built.holds(key));
    assert_eq!(
        built.candidates(key).count(),
        2,
        "shared keys must keep both candidates"
    );
    let paths: Vec<_> = built
        .candidates(key)
        .map(|source| source.path.clone())
        .collect();
    assert_eq!(
        paths,
        [
            one.entry(0x1000, 0).unwrap().path,
            two.entry(0, 0).unwrap().path
        ]
    );
    assert!(!built.holds(absent));
    assert_eq!(built.tables.failed, 0);
    assert!(!built.tables.truncated);
    assert!(built.is_current());
    for dir in [&first, &second] {
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    assert!(paths.iter().all(|path| !path.exists()));
}

#[test]
fn a_changed_table_invalidates_its_texture_key_index() {
    let (_dir, table) = fixture("table_external", false, false);
    let catalog = Tables::open(
        vec![Root::Mounted(Arc::clone(&table))],
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();
    let mut built = Index::new(catalog);
    let key = name_hash64(b"table_external");
    assert!(built.extend_for(&HashSet::from([key]), &Cancel::new(), &Progress::new()));
    assert!(built.is_current());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&table.path)
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(!built.is_current());
    assert!(built.candidates(key).next().unwrap().payload().is_err());
}

#[test]
fn cancellation_and_limits_stop_table_texture_indexing() {
    let (_dir, table) = fixture("table_external", true, false);
    let cancel = Cancel::new();
    cancel.set();
    assert!(Tables::open(
        vec![Root::Mounted(Arc::clone(&table))],
        &cancel,
        &Progress::new()
    )
    .is_none());
    assert_eq!(table.tree().nodes.len(), 1);
    let catalog = Tables::open_with_limits(
        vec![Root::Mounted(Arc::clone(&table))],
        &Cancel::new(),
        &Progress::new(),
        Limits {
            tables: 1,
            ..LIMITS
        },
    )
    .unwrap();
    assert!(catalog.truncated);
    assert!(catalog.dictionaries.is_empty());
    assert_eq!(
        table.tree().nodes.len(),
        1,
        "the next table must not be opened past the budget"
    );
    let catalog =
        Tables::open(vec![Root::Mounted(table)], &Cancel::new(), &Progress::new()).unwrap();
    let mut built = Index::new(catalog);
    let wanted = HashSet::from([name_hash64(b"table_external")]);
    assert!(!built.extend_for(&wanted, &cancel, &Progress::new()));
    assert_eq!(built.dictionaries(), 0);
    assert!(built.extend_with_limits(
        &wanted,
        &Cancel::new(),
        &Progress::new(),
        Limits {
            total_virtual_bytes: 0,
            ..LIMITS
        }
    ));
    assert!(built.tables.truncated);
    assert!(built.exhausted());
    assert_eq!(built.dictionaries(), 0);
}

#[test]
fn bad_child_tables_are_reported_without_searching_their_texture_bodies() {
    let (dir, table) = fixture("table_external", true, true);
    let catalog =
        Tables::open(vec![Root::Mounted(table)], &Cancel::new(), &Progress::new()).unwrap();
    assert_eq!(catalog.failed, 1);
    assert_eq!(catalog.failures.len(), 1);
    assert!(catalog.dictionaries.is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_texture_signature_in_an_untyped_row_is_not_probed() {
    let (_dir, table) = fixture("table_external", false, false);
    table.tree().nodes.get_mut(&0).unwrap().archive.entries[0].class = Some(255);
    let catalog =
        Tables::open(vec![Root::Mounted(table)], &Cancel::new(), &Progress::new()).unwrap();
    assert!(catalog.dictionaries.is_empty());
    assert_eq!(catalog.failed, 0);
}

#[test]
fn rejecting_a_dictionary_resumes_at_the_next_candidate_for_its_key() {
    let (_first, one) = fixture("table_external", false, false);
    let (_second, two) = fixture("table_external", false, false);
    let catalog = Tables::open(
        vec![
            Root::Mounted(Arc::clone(&one)),
            Root::Mounted(Arc::clone(&two)),
        ],
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();
    let mut index = Index::new(catalog);
    let key = name_hash64(b"table_external");
    assert!(index.extend_for(&HashSet::from([key]), &Cancel::new(), &Progress::new()));
    let first = one.entry(0, 0).unwrap().path;
    assert_eq!(index.candidates(key).next().unwrap().path, first);
    index.reject(&first);
    assert!(!index.holds(key));
    assert!(index.extend_for(&HashSet::from([key]), &Cancel::new(), &Progress::new()));
    assert_eq!(
        index.candidates(key).next().unwrap().path,
        two.entry(0, 0).unwrap().path
    );
    assert_eq!(index.candidates(key).count(), 1);
}

#[test]
fn a_root_that_failed_to_open_invalidates_when_it_appears() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.rpf");
    let catalog = Tables::open(
        vec![Root::File(path.clone())],
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();
    assert_eq!(catalog.failed, 1);
    assert!(catalog.is_current());
    std::fs::write(path, b"new file").unwrap();
    assert!(!catalog.is_current());
}

#[test]
fn one_missing_key_does_not_delay_a_dictionary_holding_another_key() {
    let (_first, one) = fixture("first_texture", false, false);
    let (_second, two) = fixture("second_texture", false, false);
    let catalog = Tables::open(
        vec![Root::Mounted(one), Root::Mounted(two)],
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();
    let mut index = Index::new(catalog);
    let present = name_hash64(b"first_texture");
    let wanted = HashSet::from([name_hash64(b"absent_texture"), present]);
    assert!(index.extend_until_any(&wanted, &Cancel::new(), &Progress::new()));
    assert!(index.holds(present));
    assert_eq!(index.dictionaries(), 1);
    assert!(!index.exhausted());
}

#[test]
fn the_worker_opens_a_nested_dictionary_and_resumes_after_an_already_tried_candidate() {
    let (first_dir, one) = fixture("table_external", true, false);
    let (second_dir, two) = fixture("table_external", false, false);
    let catalog = Tables::open(
        vec![
            Root::Mounted(Arc::clone(&one)),
            Root::Mounted(Arc::clone(&two)),
        ],
        &Cancel::new(),
        &Progress::new(),
    )
    .unwrap();
    let mut index = Index::new(catalog);
    let wanted = HashSet::from([name_hash64(b"table_external")]);
    let mut tried = HashSet::new();
    let (first, error) = crate::state::rpf9_texture_resolver::read_next(
        &mut index,
        &wanted,
        &mut tried,
        &Cancel::new(),
        &Progress::new(),
    );
    assert!(error.is_none());
    let (source, payload) = first.unwrap();
    assert_eq!(one.tree().nodes.len(), 1);
    one.expand_table(0, 0).unwrap();
    assert_eq!(source.path, one.entry(0x1000, 0).unwrap().path);
    let crate::state::DocPayload::Resource(or_io::Asset::TextureDict(dictionary)) = payload else {
        panic!("a texture dictionary")
    };
    assert!(dictionary.get("table_external").is_some());
    let (second, error) = crate::state::rpf9_texture_resolver::read_next(
        &mut index,
        &wanted,
        &mut tried,
        &Cancel::new(),
        &Progress::new(),
    );
    assert!(error.is_none());
    assert_eq!(second.unwrap().0.path, two.entry(0, 0).unwrap().path);
    assert_eq!(tried.len(), 2);
    for dir in [&first_dir, &second_dir] {
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
