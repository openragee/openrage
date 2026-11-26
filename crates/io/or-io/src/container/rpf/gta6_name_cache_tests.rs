use super::*;

const NAMES: [&str; 3] = ["x/one.ydr", "x/two.ytd", "y/three.ybn"];

/// A names folder of synthetic paths, one a line, and an empty configuration folder.
fn folders() -> (tempfile::TempDir, tempfile::TempDir) {
    let names = tempfile::tempdir().expect("a names folder");
    fs::write(names.path().join("vocabulary.txt"), NAMES.join("\n")).expect("write names");
    (names, tempfile::tempdir().expect("a config folder"))
}

fn load(names: Option<&Path>, config: Option<&Path>) -> NameResolver {
    NameResolver::load_cached(names, config)
}

/// The cache `load` keeps for `names` under `config`, and the key it is kept under.
fn kept(names: Option<&Path>, config: &Path) -> (PathBuf, Vec<u8>) {
    let sources = sources(names, Some(config));
    (cache_file(config, names, &sources), stamp(&sources))
}

fn names_in(resolver: &NameResolver, names: &[&str]) -> Vec<bool> {
    let hash = |name: &str| name_hash64(name.as_bytes());
    names
        .iter()
        .map(|n| resolver.lookup(hash(n)).is_some())
        .collect()
}

/// A row the vocabulary holds, with the hashes `name` has.
fn held(name: &str) -> Row<'_> {
    Row {
        hash64: name_hash64(name.as_bytes()),
        hash32: name_hash32(name.as_bytes()),
        held: true,
        name,
    }
}

#[test]
fn a_kept_cache_reads_back_what_was_gathered() {
    let (names, config) = folders();
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(names_in(&resolver, &NAMES), [true; 3]);

    let (file, stamp) = kept(Some(names.path()), config.path());
    let warm = read(&file, &stamp).expect("the cache reads back");
    let gathered = gather(Some(names.path()), Some(config.path()));
    assert_eq!(gathered.len(), NAMES.len());
    assert_eq!(warm.len(), gathered.len());
    for (hash, name) in gathered {
        assert_eq!(warm.lookup(hash), Some(name.as_str()));
    }
}

#[test]
fn a_warm_load_names_what_the_cold_one_did() {
    let (names, config) = folders();
    let mixed = "X/Mixed.YDR";
    let lower = "x/lower.ydr";
    let rows = format!(
        "0x{:016x}\t{mixed}\n0x2a\tfrom/list\n0x{:016x}\twith space\n0x{:016x}\t{lower}\n",
        name_hash64(mixed.as_bytes()),
        name_hash64(b"with space"),
        name_hash64(lower.as_bytes())
    );
    fs::write(names.path().join("resolved-names.tsv"), rows).expect("write a list");
    let mut cold = load(Some(names.path()), Some(config.path()));
    let mut warm = load(Some(names.path()), Some(config.path()));
    assert_eq!(warm.vocabulary(), cold.vocabulary());
    assert_eq!(warm.len(), cold.len());
    let more = [mixed, "with space"];
    let named = NAMES.iter().chain(&more);
    for hash in named.map(|n| name_hash64(n.as_bytes())).chain([0x2a]) {
        assert!(cold.lookup(hash).is_some(), "{hash:#x}");
        assert_eq!(warm.lookup(hash), cold.lookup(hash), "{hash:#x}");
    }
    assert_eq!(warm.lookup(name_hash64(mixed.as_bytes())), Some(mixed));
    let lowered = name_hash32(b"x/mixed.ydr");
    assert_eq!(warm.lookup_name32(lowered), Some("x/mixed.ydr"));

    // Another spelling of a held name inserted later, as the app adds its default index, leaves
    // both naming the hash alike, by the name the list gave it first.
    let hash = name_hash64(lower.as_bytes());
    cold.insert(hash, "X/Lower.YDR");
    warm.insert(hash, "X/Lower.YDR");
    assert_eq!(cold.lookup(hash), Some(lower));
    assert_eq!(warm.lookup(hash), cold.lookup(hash));
}

#[test]
fn a_warm_load_hashes_no_name() {
    let (names, config) = folders();
    let hashed = NameResolver::names_hashed_on_this_thread();
    load(Some(names.path()), Some(config.path()));
    let after_cold = NameResolver::names_hashed_on_this_thread();
    assert_eq!(after_cold, hashed + NAMES.len());
    load(Some(names.path()), Some(config.path()));
    assert_eq!(NameResolver::names_hashed_on_this_thread(), after_cold);

    // A row kept with hashes its name does not have is held under those, so none was computed.
    let (file, stamp) = kept(Some(names.path()), config.path());
    let stated = Row {
        hash64: 0x1234,
        hash32: 0x5678,
        held: true,
        name: "z/stated",
    };
    write(&file, &stamp, &[stated]).expect("keep a cache");
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(NameResolver::names_hashed_on_this_thread(), after_cold);
    assert_eq!(resolver.vocabulary().name64(0x1234), Some("z/stated"));
    assert_eq!(resolver.lookup_name32(0x5678), Some("z/stated"));
}

#[test]
fn a_cache_whose_key_holds_is_read_and_the_sources_are_not() {
    let (names, config) = folders();
    let (file, stamp) = kept(Some(names.path()), config.path());
    write(&file, &stamp, &[held("z/cached")]).expect("keep a cache");

    let before = NameResolver::sources_read_on_this_thread();
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(NameResolver::sources_read_on_this_thread(), before);
    assert_eq!(names_in(&resolver, &["z/cached", NAMES[0]]), [true, false]);
}

#[test]
fn two_source_lists_keep_two_caches() {
    let (first, config) = folders();
    let second = tempfile::tempdir().expect("a second names folder");
    fs::write(second.path().join("vocabulary.txt"), "s/second.ydr").expect("write names");
    let mut harvested = Vocabulary::default();
    harvested.add("h/harvested.ydr", None);
    (harvested.save(&vocabulary_file(config.path()))).expect("keep a vocabulary");
    let lists = [Some(first.path()), Some(second.path()), None];
    for names in lists {
        load(names, Some(config.path()));
    }
    let files: Vec<PathBuf> = lists.map(|names| kept(names, config.path()).0).into();
    assert_eq!(files[2], config.path().join(FILE_NAME));
    assert!(files[0] != files[1] && files[1] != files[2] && files[0] != files[2]);
    assert!(files.iter().all(|file| file.exists()));

    let before = NameResolver::sources_read_on_this_thread();
    for names in lists {
        load(names, Some(config.path()));
    }
    assert_eq!(NameResolver::sources_read_on_this_thread(), before);
    let resolver = load(Some(second.path()), Some(config.path()));
    assert_eq!(
        names_in(&resolver, &["s/second.ydr", NAMES[0]]),
        [true, false]
    );
}

#[test]
fn a_source_whose_size_changes_rebuilds_the_cache() {
    let (names, config) = folders();
    load(Some(names.path()), Some(config.path()));
    let mut more = NAMES.join("\n");
    more.push_str("\nz/four.ydd");
    fs::write(names.path().join("vocabulary.txt"), more).expect("grow the names");

    let before = NameResolver::sources_read_on_this_thread();
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(NameResolver::sources_read_on_this_thread(), before + 1);
    assert_eq!(names_in(&resolver, &["z/four.ydd"]), [true]);
    let (file, stamp) = kept(Some(names.path()), config.path());
    let warm = read(&file, &stamp).expect("kept again");
    assert_eq!(warm.len(), NAMES.len() + 1);
}

#[test]
fn a_harvested_vocabulary_is_named_beside_the_folder() {
    let (names, config) = folders();
    let mut harvested = Vocabulary::default();
    harvested.add("h/harvested.ydr", None);
    harvested
        .save(&vocabulary_file(config.path()))
        .expect("keep a vocabulary");
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(
        names_in(&resolver, &["h/harvested.ydr", NAMES[1]]),
        [true; 2]
    );
}

#[test]
fn a_truncated_bit_flipped_or_older_cache_is_rebuilt_and_not_trusted() {
    let (names, config) = folders();
    let (file, stamp) = kept(Some(names.path()), config.path());
    let fake = [held("z/fake")];
    let whole = encode(VERSION, &stamp, &fake);
    let flipped = |at: usize| {
        let mut bytes = whole.clone();
        bytes[at] ^= 1;
        bytes
    };
    let name_at = whole.len() - "z/fake".len();
    let crc_at = whole.len() - ROW_HEAD - "z/fake".len() - 4;
    let mut huge = encode(VERSION, &stamp, &[]);
    let count_at = huge.len() - 4 - 8;
    huge[count_at..count_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut past = whole.clone();
    past.push(0);
    let cases = [
        ("older", encode(VERSION - 1, &stamp, &fake)),
        ("cut short", whole[..whole.len() - 1].to_vec()),
        ("cut in its rows", whole[..crc_at + 6].to_vec()),
        ("past its end", past),
        ("a count past its bytes", huge),
        ("a name bit flipped", flipped(name_at)),
        ("a 64-bit hash bit flipped", flipped(crc_at + 4)),
        ("a 32-bit hash bit flipped", flipped(crc_at + 4 + 8)),
        ("its form flipped", flipped(crc_at + 4 + 12)),
        ("its checksum flipped", flipped(crc_at)),
        ("not a cache", b"garbage".to_vec()),
    ];
    for (what, bytes) in cases {
        fs::write(&file, bytes).expect("write the cache");
        assert!(read(&file, &stamp).is_err(), "{what}: taken");
        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(
            names_in(&resolver, &["z/fake", NAMES[2]]),
            [false, true],
            "{what}"
        );
        assert_eq!(
            read(&file, &stamp).map(|warm| warm.len()),
            Ok(NAMES.len()),
            "{what}"
        );
    }
}

#[test]
fn a_32_bit_row_survives_a_read_and_a_write() {
    let dir = tempfile::tempdir().expect("a names folder");
    let class = "ZqWidgetMeta";
    let member = "zqIntensity";
    let rows = format!(
        "0x{:08X}\t{class}\n0x{:08X}\t{member}\n0x{:08X}\tnot_its_name\n",
        literal_string_hash(class.as_bytes()),
        name_hash32(member.as_bytes()),
        name_hash32(b"something_else"),
    );
    fs::write(dir.path().join("resolved-names.tsv"), rows).expect("write the table");
    let found = "zq/found.ydr";
    let added = NameResolver::cache_names(dir.path(), [(name_hash64(found.as_bytes()), found)]);
    assert_eq!(added.expect("merged"), 1);

    let table = read_table(dir.path()).expect("read back");
    let names32: Vec<(u32, &str)> = (table.names32.iter())
        .map(|(hash, name)| (*hash, name.as_str()))
        .collect();
    let mut expected = vec![
        (literal_string_hash(class.as_bytes()), class),
        (name_hash32(member.as_bytes()), member),
    ];
    expected.sort();
    assert_eq!(names32, expected);
    let found_row = table.names64.get(&name_hash64(found.as_bytes()));
    assert_eq!(found_row.map(String::as_str), Some(found));
    assert_eq!(read_cache(dir.path()).expect("the 64-bit rows").len(), 1);
}

#[test]
fn the_folders_name_tables_name_a_hash_first() {
    let (names, config) = folders();
    let hash = name_hash64(NAMES[0].as_bytes());
    let toc = format!(
        "archive\tentry\tname64\tname32\tname\tsource\na\t0\t{hash:#018x}\t0x0\tfrom/table\tt\n"
    );
    fs::write(names.path().join("toc-entries-resolved.tsv"), toc).expect("write a table");
    fs::write(
        names.path().join("resolved-names.tsv"),
        "0x2a\tfrom/list\nnot a row\n",
    )
    .expect("write a list");
    for pass in ["cold", "warm"] {
        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(resolver.lookup(hash), Some("from/table"), "{pass}");
        assert_eq!(resolver.lookup(0x2a), Some("from/list"), "{pass}");
    }
}

#[test]
fn the_mined_table_of_the_configuration_folder_names_a_hash() {
    let (names, config) = folders();
    let mined = super::mined(config.path());
    fs::create_dir_all(mined.parent().expect("a folder")).expect("make the folder");
    fs::write(&mined, "0x2b\tfrom/mined\nnot a row\n").expect("write the table");
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(resolver.lookup(0x2b), Some("from/mined"));
    assert_eq!(
        load(None, Some(config.path())).lookup(0x2b),
        Some("from/mined")
    );
}

#[test]
fn no_source_keeps_no_cache() {
    let config = tempfile::tempdir().expect("a config folder");
    assert!(load(None, Some(config.path())).is_empty());
    assert!(!config.path().join(FILE_NAME).exists());
}
