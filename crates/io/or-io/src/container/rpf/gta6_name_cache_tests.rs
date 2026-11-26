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
        spelled: false,
        name,
    }
}

/// `bytes` with the checksum at `crc_at` made right for what follows it again.
fn rechecked(mut bytes: Vec<u8>, crc_at: usize) -> Vec<u8> {
    let crc = checksum(&bytes[crc_at + 4..]);
    bytes[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
    bytes
}

#[test]
fn a_kept_cache_reads_back_what_was_gathered() {
    let (names, config) = folders();
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(names_in(&resolver, &NAMES), [true; 3]);

    let (file, stamp) = kept(Some(names.path()), config.path());
    let warm = read(&file, &stamp).expect("the cache reads back");
    assert_eq!(warm.len(), NAMES.len());
    for name in NAMES {
        assert_eq!(warm.lookup(name_hash64(name.as_bytes())), Some(name));
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

/// Every hash a cold load, a warm one and a resolver given the same names one at a time, first
/// name first, are asked about, with the names of the 64-bit and 32-bit hashes they answer.
fn answers(resolver: &NameResolver, words: &[u64], hashes32: &[u32]) -> String {
    let mut out = String::new();
    for word in words {
        out += &format!("{word:016x} {:?}\n", resolver.lookup(*word));
    }
    for hash in hashes32 {
        out += &format!("{hash:08x} {:?}\n", resolver.lookup_name32(*hash));
    }
    out + &format!("{} {:?}", resolver.len(), resolver.vocabulary().counts())
}

#[test]
fn a_warm_and_a_cold_load_answer_alike_as_names_given_one_at_a_time() {
    // Two pairs of names that share a 32-bit hash and not a 64-bit one.
    let clashes = [
        ["c/n213241.ydr", "c/n221100.ydr"],
        ["c/n213242.ydr", "c/n221101.ydr"],
    ];
    for [one, other] in clashes {
        assert_eq!(name_hash32(one.as_bytes()), name_hash32(other.as_bytes()));
        assert_ne!(name_hash64(one.as_bytes()), name_hash64(other.as_bytes()));
    }
    let names = tempfile::tempdir().expect("a names folder");
    let config = tempfile::tempdir().expect("a config folder");
    let too_long = format!("l/{}", "a".repeat(MAX_NAME));
    let toc = format!(
        "archive\tentry\tname64\tname32\tname\tsource\na\t0\t{:#018x}\t0x0\tT/Upper.YDR\tt\n",
        name_hash64(b"t/upper.ydr")
    );
    let listed = format!(
        "0x{:016x}\tX/Mixed.YDR\n0x2a\tfrom/list\n0x{:016x}\twith space\n0x2b\t{}\n",
        name_hash64(b"x/mixed.ydr"),
        name_hash64(b"with space"),
        clashes[1][1],
    );
    let vocabulary = [
        "x/mixed.ydr",
        clashes[0][0],
        clashes[0][1],
        clashes[1][0],
        clashes[1][1],
        "Y/Case.YTD",
        "y/case.ytd",
        "// a comment",
        "has space",
        too_long.as_str(),
        "  t/upper.ydr  ",
    ];
    fs::write(names.path().join("toc-entries-resolved.tsv"), &toc).expect("write a table");
    fs::write(names.path().join("resolved-names.tsv"), &listed).expect("write a list");
    fs::write(names.path().join("vocabulary.txt"), vocabulary.join("\n")).expect("write names");
    let mut harvest = Vocabulary::default();
    harvest.add("H/Harvested.ydr", Some(*b"dr"));
    harvest.add("y/case.ytd", None);
    (harvest.save(&vocabulary_file(config.path()))).expect("keep a vocabulary");

    // The lines the vocabulary refuses, as `Vocabulary::add` judges them.
    for refused in ["has space", too_long.as_str()] {
        assert!(!Vocabulary::default().add(refused, None), "{refused}");
    }
    // The sources' names in the order a hash is first named from them, each as it is gathered: a
    // table's as written under the hash it states, a vocabulary's lower-cased under its own.
    let mut given = vec![(name_hash64(b"t/upper.ydr"), "T/Upper.YDR".to_owned())];
    given.extend(table_rows(&listed).map(|(hash, name)| (hash, name.to_owned())));
    let spelled = [
        "x/mixed.ydr",
        clashes[0][0],
        clashes[0][1],
        clashes[1][0],
        clashes[1][1],
        "y/case.ytd",
        "t/upper.ydr",
        "h/harvested.ydr",
    ];
    given.extend(spelled.map(|name| (name_hash64(name.as_bytes()), name.to_owned())));
    let mut one_at_a_time = NameResolver::new();
    for (hash, name) in &given {
        if one_at_a_time.lookup(*hash).is_none() {
            one_at_a_time.insert(*hash, name.clone());
        }
    }

    let cold = load(Some(names.path()), Some(config.path()));
    let hashed = NameResolver::names_hashed_on_this_thread();
    let warm = load(Some(names.path()), Some(config.path()));
    assert_eq!(
        NameResolver::names_hashed_on_this_thread(),
        hashed,
        "hashed warm"
    );
    assert!(warm.lookup(name_hash64(too_long.as_bytes())).is_none());
    assert!(warm.lookup(name_hash64(b"has space")).is_none());
    assert_eq!(warm.lookup(0x2b), Some(clashes[1][1]), "a stated hash");
    assert_eq!(
        warm.lookup(name_hash64(b"t/upper.ydr")),
        Some("T/Upper.YDR")
    );
    assert_eq!(warm.lookup(name_hash64(b"y/case.ytd")), Some("y/case.ytd"));
    for [one, _] in clashes {
        assert_eq!(warm.lookup_name32(name_hash32(one.as_bytes())), None);
    }

    let mut words: Vec<u64> = given.iter().map(|(hash, _)| *hash).collect();
    let mut hashes32: Vec<u32> = given
        .iter()
        .map(|(_, n)| name_hash32(n.as_bytes()))
        .collect();
    let more = ["n/new.ydr", "diffuse", "x/MIXED.ydr", "z/clash"];
    words.extend(
        more.map(|name| name_hash64(name.as_bytes()))
            .into_iter()
            .chain([7]),
    );
    hashes32.extend(
        more.map(|name| name_hash32(name.as_bytes()))
            .into_iter()
            .chain([7]),
    );
    let mut all = [one_at_a_time, cold, warm];
    let [reference, cold, warm] = all.each_ref().map(|r| answers(r, &words, &hashes32));
    assert_eq!(cold, reference, "cold");
    assert_eq!(warm, reference, "warm");

    // What the app and the CLI add after a load is answered alike too: another spelling of a held
    // name, a second name for a hash the cache names, a new name, and the bundled words.
    let mut later = Vocabulary::bundled();
    later.add("n/merged.ydr", None);
    for resolver in &mut all {
        resolver.insert(name_hash64(b"x/mixed.ydr"), "x/MIXED.ydr");
        resolver.insert(0x2a, "other/name");
        resolver.insert(name_hash64(b"n/new.ydr"), "n/new.ydr");
        resolver.insert(name_hash64(b"from/list"), "from/list");
        resolver.merge_vocabulary(&later);
    }
    words.push(name_hash64(b"n/merged.ydr"));
    words.push(name_hash64(b"from/list"));
    let [reference, cold, warm] = all.each_ref().map(|r| answers(r, &words, &hashes32));
    assert_eq!(cold, reference, "cold, later");
    assert_eq!(warm, reference, "warm, later");
    assert_eq!(
        all[2].lookup(name_hash64(b"x/mixed.ydr")),
        Some("X/Mixed.YDR")
    );
    assert_eq!(all[2].lookup(name_hash64(b"diffuse")), Some("diffuse"));
}

#[test]
fn a_warm_load_owns_no_name_until_its_vocabulary_is_asked_for() {
    let (names, config) = folders();
    load(Some(names.path()), Some(config.path()));
    let owned = NameResolver::names_owned_on_this_thread();
    let warm = load(Some(names.path()), Some(config.path()));
    assert_eq!(names_in(&warm, &NAMES), [true; 3]);
    assert_eq!(warm.len(), NAMES.len());
    assert!(!warm.is_empty());
    assert_eq!(NameResolver::names_owned_on_this_thread(), owned);
    assert_eq!(warm.vocabulary().len(), NAMES.len());
    assert_eq!(
        NameResolver::names_owned_on_this_thread(),
        owned + NAMES.len()
    );
    // Built once: asked again, it copies nothing more.
    warm.vocabulary();
    assert_eq!(
        NameResolver::names_owned_on_this_thread(),
        owned + NAMES.len()
    );
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
        spelled: false,
        name: "z/stated",
    };
    write(&file, &encode(VERSION, &stamp, &[stated])).expect("keep a cache");
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(NameResolver::names_hashed_on_this_thread(), after_cold);
    assert_eq!(resolver.vocabulary().name64(0x1234), Some("z/stated"));
    assert_eq!(resolver.lookup_name32(0x5678), Some("z/stated"));
}

#[test]
fn a_cache_whose_key_holds_is_read_and_the_sources_are_not() {
    let (names, config) = folders();
    let (file, stamp) = kept(Some(names.path()), config.path());
    write(&file, &encode(VERSION, &stamp, &[held("z/cached")])).expect("keep a cache");

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
fn a_truncated_bit_flipped_disordered_or_older_cache_is_rebuilt_and_not_trusted() {
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
    let crc_at = name_at - ROW - 4;
    let row_at = crc_at + 4;
    // Damage the checksum is made right for, which the rows themselves must refuse.
    let set = |at: usize, value: &[u8]| {
        let mut bytes = whole.clone();
        bytes[at..at + value.len()].copy_from_slice(value);
        rechecked(bytes, crc_at)
    };
    let mut huge = encode(VERSION, &stamp, &[]);
    let count_at = huge.len() - 4 - 8;
    huge[count_at..count_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut past = whole.clone();
    past.push(0);
    let mut descending = [held("z/fake"), held("z/other")];
    descending.sort_by_key(|row| std::cmp::Reverse(row.hash64));
    let cases = [
        ("older", encode(VERSION - 1, &stamp, &fake)),
        ("cut short", whole[..whole.len() - 1].to_vec()),
        ("cut in its rows", whole[..crc_at + 6].to_vec()),
        ("past its end", past.clone()),
        ("past its end, rechecked", rechecked(past, crc_at)),
        ("a count past its bytes", huge),
        ("a name bit flipped", flipped(name_at)),
        ("a 64-bit hash bit flipped", flipped(row_at)),
        ("a 32-bit hash bit flipped", flipped(row_at + 8)),
        ("its end flipped", flipped(row_at + 12)),
        ("its form flipped", flipped(row_at + 20)),
        ("its checksum flipped", flipped(crc_at)),
        ("rows out of order", encode(VERSION, &stamp, &descending)),
        (
            "an end past its names",
            set(row_at + 12, &7u64.to_le_bytes()),
        ),
        (
            "an end short of its names",
            set(row_at + 12, &5u64.to_le_bytes()),
        ),
        ("a form unknown", set(row_at + 20, &[2])),
        ("a name not text", set(name_at, &[0xFF])),
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
