#![cfg(feature = "gta6")]

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use or_io::container::rpf::NameResolver;
use or_io::gta6::hash::name_hash64;

const BUNDLED: &str = include_str!("../../../../assets/gta6_names/resolved-names.tsv");
const ENV_NAME: &str = "user/vehicles/test_environment_model";
const CHILD: &str = "OPENRAGE_NAMES_ENV_TEST_CHILD";

fn assert_bundled(resolver: &NameResolver) {
    for name in [
        "content.xml",
        "data/vehicles/controls/vehicle_abilities.meta",
    ] {
        assert_eq!(resolver.lookup(name_hash64(name.as_bytes())), Some(name));
    }
}

fn table(dir: &Path, name: &str) {
    let hash = name_hash64(name.as_bytes());
    fs::write(
        dir.join("resolved-names.tsv"),
        format!("0x{hash:016x}\t{name}\n"),
    )
    .expect("write supplemental names");
}

#[test]
fn bundled_names_are_sorted_unique_and_hash_their_paths() {
    let mut seen = HashSet::new();
    let mut previous = None;
    for (index, line) in BUNDLED.lines().enumerate() {
        let (hash, name) = line.split_once('\t').expect("hash and name columns");
        let hash = u64::from_str_radix(hash.strip_prefix("0x").expect("hex prefix"), 16)
            .expect("64-bit hash");
        assert_eq!(hash, name_hash64(name.as_bytes()), "line {}", index + 1);
        assert!(seen.insert(hash), "duplicate hash on line {}", index + 1);
        assert!(previous.is_none_or(|p| p < hash), "table is not sorted");
        previous = Some(hash);
    }
    assert!(!seen.is_empty());
}

#[test]
fn absent_external_names_keep_the_bundled_names() {
    let temp = tempfile::tempdir().expect("scratch directory");
    assert_bundled(&NameResolver::load_with_defaults(Some(
        &temp.path().join("missing"),
    )));
}

#[test]
fn explicit_names_supplement_the_bundle_and_take_precedence() {
    let temp = tempfile::tempdir().expect("scratch directory");
    let name = "user/peds/test_explicit_model";
    let hash = name_hash64(name.as_bytes());
    let shared = name_hash64(b"content.xml");
    fs::write(
        temp.path().join("resolved-names.tsv"),
        format!("0x{hash:016x}\t{name}\n0x{shared:016x}\tCONTENT.XML\n"),
    )
    .expect("write supplemental names");
    let vocabulary_name = "user/vehicles/test_vocabulary_model";
    fs::write(
        temp.path().join("vocabulary.txt"),
        format!("openrage gta6 names 1\n-- {vocabulary_name}\n"),
    )
    .expect("write supplemental vocabulary");
    let resolver = NameResolver::load_with_defaults(Some(temp.path()));
    assert_eq!(
        resolver.lookup(name_hash64(vocabulary_name.as_bytes())),
        Some(vocabulary_name)
    );
    assert_eq!(resolver.lookup(hash), Some(name));
    assert_eq!(resolver.lookup(shared), Some("CONTENT.XML"));
    let bundled = "data/vehicles/controls/vehicle_abilities.meta";
    assert_eq!(
        resolver.lookup(name_hash64(bundled.as_bytes())),
        Some(bundled)
    );
}

#[test]
fn environment_names_supplement_the_bundle_unless_a_directory_is_explicit() {
    if std::env::var_os(CHILD).is_some() {
        let resolver = NameResolver::load_with_defaults(None);
        assert_bundled(&resolver);
        assert_eq!(
            resolver.lookup(name_hash64(ENV_NAME.as_bytes())),
            Some(ENV_NAME)
        );
        let explicit = tempfile::tempdir().expect("explicit directory");
        let name = "user/vehicles/test_explicit_directory";
        table(explicit.path(), name);
        let resolver = NameResolver::load_with_defaults(Some(explicit.path()));
        assert_bundled(&resolver);
        assert_eq!(resolver.lookup(name_hash64(name.as_bytes())), Some(name));
        assert_eq!(resolver.lookup(name_hash64(ENV_NAME.as_bytes())), None);
        return;
    }
    let temp = tempfile::tempdir().expect("environment directory");
    table(temp.path(), ENV_NAME);
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "environment_names_supplement_the_bundle_unless_a_directory_is_explicit",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("OPENRAGE_GTA6_NAMES", temp.path())
        .output()
        .expect("run with an isolated environment");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
