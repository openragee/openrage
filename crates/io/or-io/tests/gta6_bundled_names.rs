#![cfg(feature = "gta6")]

use or_io::container::rpf::{Gta6Entry, NameResolver, GTA6_NAMES_ENV};
use or_io::gta6::hash::{name_hash32, name_hash64};
use std::{fs, path::Path, process::Command};

fn table(dir: &Path, names: &[&str]) {
    fs::write(
        dir.join("resolved-names.tsv"),
        names
            .iter()
            .map(|n| format!("0x{:016x}\t{n}\n", name_hash64(n.as_bytes())))
            .collect::<String>(),
    )
    .expect("write table");
}

#[test]
fn common_words_resolve_without_an_external_dictionary() {
    let dir = tempfile::tempdir().expect("scratch");
    let names = NameResolver::load_with_defaults(Some(dir.path()));
    assert_eq!(names.lookup(name_hash64(b"diffuse")), Some("diffuse"));
    assert_eq!(
        names.lookup_name32(name_hash32(b"diffuse")),
        Some("diffuse")
    );
    assert!(NameResolver::new().is_empty());
}

#[test]
fn external_names_take_precedence_over_the_cache_and_shared_words() {
    let external = tempfile::tempdir().expect("external");
    let local = tempfile::tempdir().expect("local");
    table(external.path(), &["DIFFUSE", "external/model.ydr"]);
    table(
        local.path(),
        &["diffuse", "local/long_directory/model.meta"],
    );
    fs::write(
        local.path().join("vocabulary.txt"),
        "openrage gta6 names 1\n-- local_vocabulary_word\n",
    )
    .expect("vocabulary");
    let names = NameResolver::load_with_sources(Some(external.path()), Some(local.path()));
    for n in [
        "DIFFUSE",
        "external/model.ydr",
        "local/long_directory/model.meta",
        "local_vocabulary_word",
        "texture",
    ] {
        assert_eq!(names.lookup(name_hash64(n.as_bytes())), Some(n));
    }
    assert_eq!(
        names
            .vocabulary()
            .name32(name_hash32(b"external/model.ydr")),
        Some("external/model.ydr")
    );
}

#[test]
fn missing_external_folder_keeps_long_paths_and_ignores_bad_cache_hashes() {
    let local = tempfile::tempdir().expect("local");
    let long = format!("{}/model.ytd", "long_directory/".repeat(15));
    table(local.path(), &[&long]);
    let cache = local.path().join("resolved-names.tsv");
    let mut text = fs::read_to_string(&cache).expect("read");
    text.push_str(&format!(
        "0x{:016x}\twrong_name\n",
        name_hash64(b"not_wrong_name")
    ));
    fs::write(cache, text).expect("bad row");
    let names =
        NameResolver::load_with_sources(Some(&local.path().join("absent")), Some(local.path()));
    assert_eq!(
        names.lookup(name_hash64(long.as_bytes())),
        Some(long.as_str())
    );
    assert_eq!(names.lookup(name_hash64(b"not_wrong_name")), None);
    assert_eq!(names.lookup(name_hash64(b"diffuse")), Some("diffuse"));
}

#[test]
fn cache_merges_preserve_rows_and_reject_invalid_names_before_writing() {
    let dir = tempfile::tempdir().expect("scratch");
    let original = "existing/path.meta";
    table(dir.path(), &[original]);
    let n = "discovered/model.ydr";
    assert_eq!(
        NameResolver::cache_names(dir.path(), [(name_hash64(n.as_bytes()), n)]).expect("cache"),
        1
    );
    assert_eq!(
        NameResolver::cache_names(dir.path(), [(name_hash64(n.as_bytes()), n)]).expect("repeat"),
        0
    );
    let file = dir.path().join("resolved-names.tsv");
    let before = fs::read(&file).expect("before");
    assert!(
        NameResolver::cache_names(dir.path(), [(name_hash64(b"something_else"), "invalid")])
            .is_err()
    );
    assert_eq!(fs::read(&file).expect("after"), before);
    assert!(!dir.path().join(".resolved-names.tsv.tmp").exists());
    let names = NameResolver::load_with_sources(Some(dir.path()), None);
    for n in [original, n] {
        assert_eq!(names.lookup(name_hash64(n.as_bytes())), Some(n));
    }
}

#[test]
fn concurrent_cache_writers_keep_each_others_matches() {
    let dir = tempfile::tempdir().expect("scratch");
    let writers: Vec<_> = (0..8)
        .map(|i| {
            let path = dir.path().to_path_buf();
            std::thread::spawn(move || {
                let name = format!("thread_{i}/model.ytd");
                NameResolver::cache_names(&path, [(name_hash64(name.as_bytes()), name.as_str())])
                    .expect("merge");
            })
        })
        .collect();
    for writer in writers {
        writer.join().expect("writer");
    }
    let names = NameResolver::load(dir.path());
    for i in 0..8 {
        let name = format!("thread_{i}/model.ytd");
        assert_eq!(
            names.lookup(name_hash64(name.as_bytes())),
            Some(name.as_str())
        );
    }
    let text = fs::read_to_string(dir.path().join("resolved-names.tsv")).expect("cache");
    let hashes: Vec<u64> = text
        .lines()
        .map(|l| {
            u64::from_str_radix(
                l.split_once('\t').expect("row").0.trim_start_matches("0x"),
                16,
            )
            .expect("hex")
        })
        .collect();
    assert!(hashes.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn only_table_matches_are_cached_not_every_candidate() {
    let dir = tempfile::tempdir().expect("scratch");
    let names = NameResolver::load_with_defaults(Some(dir.path()));
    let entry = Gta6Entry {
        index: 0,
        name64: name_hash64(b"diffuse"),
        name32: None,
        name: None,
        start: None,
        stored: None,
        real: None,
        class: None,
        cipher: None,
    };
    assert_eq!(
        names
            .cache_entries(&[entry], dir.path())
            .expect("cache entries"),
        1
    );
    let text = fs::read_to_string(dir.path().join("resolved-names.tsv")).expect("read");
    assert_eq!(
        text,
        format!("0x{:016x}\tdiffuse\n", name_hash64(b"diffuse"))
    );
}

#[test]
fn environment_names_are_loaded_only_when_no_directory_is_explicit() {
    const CHILD: &str = "OPENRAGE_SHARED_WORDS_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let names = NameResolver::load_with_defaults(None);
        let n = "user/vehicles/environment_model";
        assert_eq!(names.lookup(name_hash64(n.as_bytes())), Some(n));
        assert_eq!(names.lookup(name_hash64(b"diffuse")), Some("diffuse"));
        let explicit = tempfile::tempdir().expect("explicit");
        let names = NameResolver::load_with_defaults(Some(explicit.path()));
        assert_eq!(names.lookup(name_hash64(n.as_bytes())), None);
        return;
    }
    let dir = tempfile::tempdir().expect("external");
    table(dir.path(), &["user/vehicles/environment_model"]);
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "environment_names_are_loaded_only_when_no_directory_is_explicit",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(GTA6_NAMES_ENV, dir.path())
        .output()
        .expect("child");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn resolvers_without_a_local_directory_do_not_persist_names() {
    let names = NameResolver::new();
    assert_eq!(names.cache_local_entries(&[]).expect("no cache"), 0);
}
