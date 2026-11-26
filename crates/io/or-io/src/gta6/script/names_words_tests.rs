use std::collections::{BTreeSet, HashMap};

use super::*;
use crate::gta6::hash::name_hash64;

fn folded32(word: &str) -> u32 {
    literal_string_hash(word.to_ascii_lowercase().as_bytes())
}

fn kept32(word: &str) -> u32 {
    literal_string_hash(word.as_bytes())
}

fn held(words: &[&str]) -> Words {
    let mut held = Words::default();
    for word in words {
        assert!(held.add(word), "{word}");
    }
    held
}

#[test]
fn a_word_is_found_by_each_of_its_four_hashes_and_by_no_other() {
    let words = held(&["QuillGrommet", "vexa"]);
    assert_eq!(words.len(), 2);
    assert_eq!(words.iter().collect::<Vec<_>>(), ["QuillGrommet", "vexa"]);

    let kept = literal_hash64(b"QuillGrommet");
    let folded = name_hash64(b"QuillGrommet");
    assert_ne!(kept, folded, "the two 64-bit hashes of a capital differ");
    assert_eq!(words.kept64(kept), Some("QuillGrommet"));
    assert_eq!(words.folded64(folded), Some("QuillGrommet"));
    assert_eq!(words.kept64(folded), None);
    assert_eq!(words.folded64(kept), None);

    assert_eq!(words.method(kept32("QuillGrommet")), Some("QuillGrommet"));
    assert_eq!(
        words.folded32(folded32("QuillGrommet")),
        Some("QuillGrommet")
    );
    assert_eq!(words.method(folded32("QuillGrommet")), None);
    assert_eq!(words.folded32(kept32("QuillGrommet")), None);

    // A word in lower case is its own folding: each pair of functions agrees on it.
    assert_eq!(words.kept64(name_hash64(b"VEXA")), Some("vexa"));
    assert_eq!(words.method(folded32("Vexa")), Some("vexa"));
}

#[test]
fn what_is_no_word_or_is_held_already_is_not_added() {
    let mut words = held(&["zorb"]);
    assert!(!words.add("zorb"));
    let long = "z".repeat(MAX_WORD + 1);
    for no_word in ["", "two words", "tab\tbed", long.as_str()] {
        assert!(!words.add(no_word), "`{no_word}`");
    }
    assert!(words.add(&"z".repeat(MAX_WORD)));
    assert_eq!(words.len(), 2);
    assert!(!words.is_empty() && Words::default().is_empty());
}

#[test]
fn two_spellings_of_one_word_fold_to_one_text_and_two_words_of_one_hash_name_nothing_by_it() {
    let words = held(&["Thimble", "thimble"]);
    assert_eq!(
        words.folded32(folded32("thimble")),
        Some("Thimble"),
        "the first spelling read is the text"
    );
    assert_eq!(words.folded64(name_hash64(b"thimble")), Some("Thimble"));
    assert_eq!(words.method(kept32("thimble")), Some("thimble"));
    assert_eq!(words.kept64(literal_hash64(b"Thimble")), Some("Thimble"));

    // Two names of one 32-bit hash, found by trying: eighty thousand hold such a pair as often as not.
    let mut seen: HashMap<u32, String> = HashMap::new();
    let pair = (0..1_000_000).find_map(|n| {
        let name = format!("name{n}");
        let first = seen.insert(kept32(&name), name.clone())?;
        Some((first, name))
    });
    let (first, second) = pair.expect("a million names hold two of one hash");
    let words = held(&[first.as_str(), second.as_str()]);
    assert_eq!(words.method(kept32(&first)), None);
    assert_eq!(words.folded32(folded32(&second)), None);
    // By 64 bits each is still itself.
    assert_eq!(
        words.kept64(literal_hash64(first.as_bytes())),
        Some(first.as_str())
    );
    assert_eq!(
        words.kept64(literal_hash64(second.as_bytes())),
        Some(second.as_str())
    );
}

#[test]
fn past_the_count_32_bits_can_name_by_no_word_names_a_method_and_64_bits_still_name() {
    let mut words = Words::default();
    for n in 0..METHOD_WORDS {
        words.add(&format!("w{n}"));
    }
    assert_eq!(words.len(), METHOD_WORDS);
    assert_eq!(words.method(kept32("w7")), Some("w7"));

    assert!(words.add("one_more"));
    assert_eq!(words.method(kept32("w7")), None);
    assert_eq!(words.kept64(literal_hash64(b"w7")), Some("w7"));
    assert_eq!(words.folded64(name_hash64(b"w7")), Some("w7"));
}

#[test]
fn the_words_a_search_may_be_handed_are_plain_ordered_and_each_once() {
    let words: Vec<&str> = bundled().collect();
    assert!((1_000..=5_000).contains(&words.len()), "{}", words.len());
    for word in &words {
        assert!(
            word.bytes().all(|b| b.is_ascii_lowercase()),
            "not a plain word: {word}"
        );
    }
    let once: BTreeSet<&str> = words.iter().copied().collect();
    assert_eq!(once.len(), words.len(), "a word stands twice");
    assert!(
        words.windows(2).all(|pair| pair[0] < pair[1]),
        "out of order"
    );
    for held in ["get", "is", "update", "vehicle", "index"] {
        assert!(once.contains(held), "{held}");
    }
}
