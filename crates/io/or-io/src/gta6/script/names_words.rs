//! The words of the hashless lists a user supplies. A word states no hash, so it is a candidate:
//! it names a thing only where one of the four hashes of the word is the hash that thing states.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::Hash;

use crate::gta6::hash::literal_hash64;
use crate::hash::literal_string_hash;

/// The longest word held, in bytes.
pub const MAX_WORD: usize = 160;

/// The most words that may name a method by the one 32-bit hash its record states.
// ⚠ A word found by 32 bits alone is the record's name by chance once in 2^32 over the count of
// words: at this many, one method record in 65,536 takes a wrong name, and past it none is named.
// A type, a field and a module are found by 64 bits and a program by both of its hashes, so those
// hold for a list of any size.
pub const METHOD_WORDS: usize = 1 << 16;

/// The place two different words of one hash share: no word's.
const SHARED: u32 = u32::MAX;

/// Words by each of the four hashes a program may state of a name.
#[derive(Clone, Debug, Default)]
pub struct Words {
    /// Each word once, as its list spells it.
    held: Vec<Box<str>>,
    kept64: HashMap<u64, u32>,
    folded64: HashMap<u64, u32>,
    kept32: HashMap<u32, u32>,
    folded32: HashMap<u32, u32>,
}

impl Words {
    /// Holds `word`; `false` where it is held already, or is no word: empty, over [`MAX_WORD`] bytes, or with a space or a control character in it.
    pub fn add(&mut self, word: &str) -> bool {
        let at = u32::try_from(self.held.len()).unwrap_or(SHARED);
        if at == SHARED || !holdable(word) {
            return false;
        }
        let kept = literal_hash64(word.as_bytes());
        if let Some(held) = self.kept64.get(&kept).copied() {
            if self.at(Some(held)) != Some(word) {
                self.kept64.insert(kept, SHARED);
            }
            return false;
        }
        let lower = word.to_ascii_lowercase();
        self.kept64.insert(kept, at);
        // Two spellings of one word fold to one text, which is no second name of the hash.
        let same = |held: &str| held.eq_ignore_ascii_case(word);
        let folded = literal_hash64(lower.as_bytes());
        enter(&mut self.folded64, folded, at, &self.held, same);
        let folded = literal_string_hash(lower.as_bytes());
        enter(&mut self.folded32, folded, at, &self.held, same);
        let kept = literal_string_hash(word.as_bytes());
        enter(&mut self.kept32, kept, at, &self.held, |_| false);
        self.held.push(word.into());
        true
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Every word held, in the order read.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.held.iter().map(|word| &**word)
    }

    /// The one word whose 64-bit hash, as it is spelled, is `hash`: what a type's and a field's record state.
    pub fn kept64(&self, hash: u64) -> Option<&str> {
        self.at(self.kept64.get(&hash).copied())
    }

    /// The one word whose 64-bit hash in lower case is `hash`: a module's, and a 64-bit hash constant's.
    pub fn folded64(&self, hash: u64) -> Option<&str> {
        self.at(self.folded64.get(&hash).copied())
    }

    /// The one word whose 32-bit hash in lower case is `hash`: a 32-bit hash constant's.
    pub fn folded32(&self, hash: u32) -> Option<&str> {
        self.at(self.folded32.get(&hash).copied())
    }

    /// The one word whose 32-bit hash, as it is spelled, is `hash`, where the words are few enough for 32 bits to name a method by.
    pub fn method(&self, hash: u32) -> Option<&str> {
        let few = self.held.len() <= METHOD_WORDS;
        self.at(self.kept32.get(&hash).copied().filter(|_| few))
    }

    fn at(&self, place: Option<u32>) -> Option<&str> {
        let word = self.held.get(usize::try_from(place?).ok()?)?;
        Some(&**word)
    }
}

/// Puts the word at `at` under `key`; where a word that is not the `same` one is there, the key is no word's from then on.
fn enter<K: Eq + Hash>(
    map: &mut HashMap<K, u32>,
    key: K,
    at: u32,
    held: &[Box<str>],
    same: impl Fn(&str) -> bool,
) {
    match map.entry(key) {
        Entry::Vacant(free) => {
            free.insert(at);
        }
        Entry::Occupied(mut first) => {
            let other = held.get(*first.get() as usize).is_some_and(|w| !same(w));
            if other {
                first.insert(SHARED);
            }
        }
    }
}

fn holdable(word: &str) -> bool {
    let printed = |b: u8| b.is_ascii_graphic() || !b.is_ascii();
    !word.is_empty() && word.len() <= MAX_WORD && word.bytes().all(printed)
}

/// Ordinary English and programming words for a search to join into names; no list of names, and read by nothing unless a search is handed it.
pub fn bundled() -> impl Iterator<Item = &'static str> {
    let lines = include_str!("names_words.txt").lines().map(str::trim);
    lines.filter(|word| !word.is_empty())
}

#[cfg(test)]
#[path = "names_words_tests.rs"]
mod tests;
