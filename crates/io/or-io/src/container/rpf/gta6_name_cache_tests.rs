use std::collections::BTreeMap;

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

fn names_in(resolver: &NameResolver, names: &[&str]) -> Vec<bool> {
    let hash = |name: &str| name_hash64(name.as_bytes());
    names
        .iter()
        .map(|n| resolver.lookup(hash(n)).is_some())
        .collect()
}

fn sorted(pairs: impl IntoIterator<Item = (u64, String)>) -> BTreeMap<u64, String> {
    pairs.into_iter().collect()
}

#[test]
fn a_kept_cache_reads_back_what_was_gathered() {
    let (names, config) = folders();
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(names_in(&resolver, &NAMES), [true; 3]);

    let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
    let kept = read(&config.path().join(FILE_NAME), &stamp).expect("the cache reads back");
    let gathered = gather(Some(names.path()), Some(config.path()));
    assert_eq!(gathered.len(), NAMES.len());
    assert_eq!(sorted(kept), sorted(gathered));
}

#[test]
fn a_cache_whose_key_holds_is_read_and_the_sources_are_not() {
    let (names, config) = folders();
    let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
    let only = vec![(name_hash64(b"z/cached"), "z/cached".to_owned())];
    write(&config.path().join(FILE_NAME), &stamp, &only).expect("keep a cache");

    let before = NameResolver::sources_read_on_this_thread();
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(NameResolver::sources_read_on_this_thread(), before);
    assert_eq!(names_in(&resolver, &["z/cached", NAMES[0]]), [true, false]);
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
    let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
    let kept = read(&config.path().join(FILE_NAME), &stamp).expect("kept again");
    assert_eq!(kept.len(), NAMES.len() + 1);
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
fn a_corrupt_or_older_cache_is_rebuilt_and_not_trusted() {
    let (names, config) = folders();
    let file = config.path().join(FILE_NAME);
    let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
    let fake = vec![(name_hash64(b"z/fake"), "z/fake".to_owned())];
    let mut cut = encode(VERSION, &stamp, &fake);
    cut.pop();
    let mut past = encode(VERSION, &stamp, &fake);
    past.push(0);
    let mut huge = encode(VERSION, &stamp, &[]);
    let at = huge.len() - 8 - 32;
    huge[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let mut renamed = encode(VERSION, &stamp, &fake);
    *renamed.last_mut().expect("a name byte") ^= 1;
    let mut rehashed = encode(VERSION, &stamp, &fake);
    let hash_at = rehashed.len() - "z/fake".len() - 2 - 8;
    rehashed[hash_at] ^= 1;
    let cases = [
        ("older", encode(VERSION - 1, &stamp, &fake)),
        ("cut short", cut),
        ("past its end", past),
        ("a count past its bytes", huge),
        ("a name byte flipped", renamed),
        ("a hash byte flipped", rehashed),
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
            read(&file, &stamp).map(|p| p.len()),
            Ok(NAMES.len()),
            "{what}"
        );
    }
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
    let resolver = load(Some(names.path()), Some(config.path()));
    assert_eq!(resolver.lookup(hash), Some("from/table"));
    assert_eq!(resolver.lookup(0x2a), Some("from/list"));
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
