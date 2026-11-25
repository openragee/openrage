use or_io::names::{bundled_words, HashFlavour, NameIndex};
use std::collections::HashSet;

#[test]
fn bundled_words_are_unique_plain_candidates_without_paths_hashes_or_extensions() {
    let mut seen = HashSet::new();
    for word in bundled_words() {
        assert!(
            word.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_+-@".contains(&b)),
            "not a word: {word}"
        );
        assert!(word.bytes().any(|b| b.is_ascii_alphabetic()));
        assert!(!word.starts_with("0x"), "hash rather than word: {word}");
        assert!(seen.insert(word), "repeated word: {word}");
    }
    assert!(!seen.is_empty());
}

#[test]
fn each_hash_flavour_uses_the_same_words_without_seeding_empty_indexes() {
    let word = "diffuse";
    let names = NameIndex::with_bundled("gta5");
    for flavour in HashFlavour::ALL {
        assert_eq!(names.resolve_with(flavour, flavour.of(word)), Some(word));
    }
    assert!(NameIndex::new("gta5")
        .resolve(HashFlavour::CaseFolding.of(word))
        .is_none());
    let mut names = NameIndex::with_bundled("gta5");
    names.insert("DIFFUSE");
    assert_eq!(
        names.resolve_with(HashFlavour::CaseFolding, HashFlavour::CaseFolding.of(word)),
        Some("DIFFUSE")
    );
}
