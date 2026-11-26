//! A bounded search for the names of the things a program states two hashes of: words are joined
//! as names are written, a join is found by the 32-bit hash and kept only where the 64-bit hash
//! of one of its spellings is the one stated beside it.
//!
//! ⚠ Nothing is named by 32 bits here. One join in 2^32 has a target's 32-bit hash by chance,
//! and of those the 64-bit hash refuses all but one in 2^64: a find is a proof, whatever the
//! words were.

use std::collections::{BTreeSet, HashMap};

use super::Names;
use crate::gta6::hash::literal_hash64;
use crate::gta6::script::program::Program;
use crate::hash::{finalize_hash, partial_literal_hash};

/// The most words one name is joined of.
pub const MAX_JOIN: usize = 4;

/// The longest word joined, in bytes.
const MAX_WORD: usize = 32;

/// The bits of a 32-bit hash that place it in the filter a join is tried on first.
const FILTER_BITS: u32 = 22;

/// What a search looks for the name of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// A type or a field: the 64-bit hash of its name as spelled, beside the 32-bit hash of the name in lower case.
    Member,
    /// A program: the 64-bit hash of its file's name beside its 32-bit hash, both of the name in lower case.
    Program,
}

/// The pairs of hashes a search looks for one name of.
#[derive(Clone, Debug, Default)]
pub struct Targets {
    /// What is sought and its 64-bit hash, by the 32-bit hash stated beside it.
    by32: HashMap<u32, Vec<(Target, u64)>>,
    /// One bit for each value of the top [`FILTER_BITS`] bits of a 32-bit hash sought; empty until one is.
    filter: Vec<u64>,
    pairs: usize,
}

impl Targets {
    /// Seeks the name of a type or a field whose record states `kept64` and `folded32`.
    pub fn member(&mut self, kept64: u64, folded32: u32) {
        self.seek(Target::Member, folded32, kept64);
    }

    /// Seeks the name of the program of 32-bit hash `program` whose file has the 64-bit hash `file`.
    pub fn program(&mut self, program: u32, file: u64) {
        self.seek(Target::Program, program, file);
    }

    /// Seeks the name of every type and field of `program` that `names` has none for.
    // A record with no hash of a name states none.
    pub fn of_program(&mut self, program: &Program, names: &Names) {
        for ty in &program.types {
            if ty.name != 0 && names.type_name(ty.name).is_none() {
                self.member(ty.name, ty.name32);
            }
            for field in &ty.fields {
                if field.name != 0 && names.field_name(field.name).is_none() {
                    self.member(field.name, field.name32);
                }
            }
        }
    }

    /// The pairs sought, each once.
    pub fn len(&self) -> usize {
        self.pairs
    }

    pub fn is_empty(&self) -> bool {
        self.pairs == 0
    }

    fn seek(&mut self, kind: Target, hash32: u32, hash64: u64) {
        let sought = self.by32.entry(hash32).or_default();
        if sought.contains(&(kind, hash64)) {
            return;
        }
        sought.push((kind, hash64));
        self.pairs += 1;
        if self.filter.is_empty() {
            self.filter = vec![0; 1 << (FILTER_BITS - 6)];
        }
        let (word, bit) = place(hash32);
        if let Some(word) = self.filter.get_mut(word) {
            *word |= bit;
        }
    }

    /// What is sought beside the 32-bit hash `hash`; `None` for nearly every hash nothing is.
    fn sought(&self, hash: u32) -> Option<&[(Target, u64)]> {
        let (word, bit) = place(hash);
        if self.filter.get(word).is_none_or(|held| held & bit == 0) {
            return None;
        }
        self.by32.get(&hash).map(Vec::as_slice)
    }
}

/// The word of the filter and the bit in it that `hash` falls on.
fn place(hash: u32) -> (usize, u64) {
    let n = hash >> (32 - FILTER_BITS);
    ((n >> 6) as usize, 1 << (n & 63))
}

/// How far a search goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// The most words joined into one name; no more than [`MAX_JOIN`] are.
    pub join: usize,
    /// The most joins hashed before the search stops.
    pub candidates: u64,
}

/// Two words a name, and 2^28 joins.
impl Default for Budget {
    fn default() -> Budget {
        Budget {
            join: 2,
            candidates: 1 << 28,
        }
    }
}

/// A name both stated hashes are the hashes of.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub kind: Target,
    pub hash64: u64,
    pub hash32: u32,
    pub name: String,
}

/// What a search found, and how far it went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    /// In the order of kind, hash and name, each once.
    pub found: Vec<Found>,
    /// The joins hashed.
    pub tried: u64,
    /// Whether every join of the budget's count of words was tried; `false` where its count of joins ended the search first.
    pub complete: bool,
}

impl Search {
    /// The finds as a names list: a row's hashes are both hashes of its name, so a reader proves each row again.
    pub fn list(&self) -> String {
        let mut text = String::from(
            "# found by a word search: the two hashes of a row are both hashes of its name\n",
        );
        for found in &self.found {
            let (hash64, hash32, name) = (found.hash64, found.hash32, &found.name);
            // A program's row opens with its 32-bit hash, a type's and a field's with the other.
            text.push_str(&match found.kind {
                Target::Member => format!("0x{hash64:016X}\t0x{hash32:08X}\t{name}\n"),
                Target::Program => format!("0x{hash32:08X}\t0x{hash64:016X}\t{name}\n"),
            });
        }
        text
    }
}

/// What stands before the first word of a name and between each two.
struct Shape {
    before: &'static str,
    between: &'static str,
}

impl Shape {
    const fn of(before: &'static str, between: &'static str) -> Shape {
        Shape { before, between }
    }
}

/// The ways words are joined, each tried in every case its words can be written in.
// `DoorState`, `doorState` and `doorstate`; `door_state` and `DOOR_STATE`; `_doorState`;
// `m_doorState`; `IDoorState`.
const SHAPES: [Shape; 5] = [
    Shape::of("", ""),
    Shape::of("", "_"),
    Shape::of("_", ""),
    Shape::of("m_", ""),
    Shape::of("I", ""),
];

/// The words of a search, each once whatever its case.
struct Lexicon {
    /// As the list spells each.
    seen: Vec<String>,
    /// Each in lower case, which is what the 32-bit hash is of.
    lower: Vec<Vec<u8>>,
}

impl Lexicon {
    /// The words of `words` that are letters, digits and `_`, no longer than [`MAX_WORD`].
    fn of<'w>(words: impl IntoIterator<Item = &'w str>) -> Lexicon {
        let mut lexicon = Lexicon {
            seen: Vec::new(),
            lower: Vec::new(),
        };
        let mut held = BTreeSet::new();
        for word in words {
            let plain = word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            if word.is_empty() || word.len() > MAX_WORD || !plain {
                continue;
            }
            let lower = word.to_ascii_lowercase();
            if held.insert(lower.clone()) {
                lexicon.seen.push(word.to_owned());
                lexicon.lower.push(lower.into_bytes());
            }
        }
        lexicon
    }
}

/// Searches the joins of up to `budget.join` of `words` for the names of `targets`.
// The joins of one word are tried before those of two, and so on, so a budget that runs out
// has tried the likelier names.
pub fn search<'w>(
    words: impl IntoIterator<Item = &'w str>,
    targets: &Targets,
    budget: Budget,
) -> Search {
    let lexicon = Lexicon::of(words);
    let mut run = Run {
        targets,
        lexicon: &lexicon,
        found: BTreeSet::new(),
        tried: 0,
        left: budget.candidates,
        first: (0, 1),
    };
    let mut complete = true;
    if !targets.is_empty() && !lexicon.seen.is_empty() {
        'joins: for length in 1..=budget.join.min(MAX_JOIN) {
            for shape in &SHAPES {
                // One word has nothing between it and another.
                if length == 1 && !shape.between.is_empty() {
                    continue;
                }
                if !run.joins(length, shape) {
                    complete = false;
                    break 'joins;
                }
            }
        }
    }
    Search {
        found: run.found.into_iter().collect(),
        tried: run.tried,
        complete,
    }
}

/// Searches as [`search`] does, on `threads` threads, each taking every `threads`th first word.
// ⚠ The budget is spent a length and a shape at a time: a run whose count of joins it does not
// cover is not started, where [`search`] tries its first joins.
pub fn search_threads<'w>(
    words: impl IntoIterator<Item = &'w str>,
    targets: &Targets,
    budget: Budget,
    threads: usize,
) -> Search {
    let lexicon = Lexicon::of(words);
    let threads = threads.max(1);
    let count = lexicon.lower.len() as u64;
    let mut done = Search {
        complete: true,
        ..Search::default()
    };
    let mut found = BTreeSet::new();
    let mut left = budget.candidates;
    if targets.is_empty() || count == 0 {
        return done;
    }
    'joins: for length in 1..=budget.join.min(MAX_JOIN) {
        for shape in &SHAPES {
            if length == 1 && !shape.between.is_empty() {
                continue;
            }
            let joins = count.checked_pow(length as u32).unwrap_or(u64::MAX);
            if joins > left {
                done.complete = false;
                break 'joins;
            }
            left -= joins;
            done.tried += joins;
            let parts: Vec<BTreeSet<Found>> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..threads)
                    .map(|offset| {
                        let lexicon = &lexicon;
                        scope.spawn(move || {
                            let mut run = Run {
                                targets,
                                lexicon,
                                found: BTreeSet::new(),
                                tried: 0,
                                left: u64::MAX,
                                first: (offset, threads),
                            };
                            run.joins(length, shape);
                            run.found
                        })
                    })
                    .collect();
                // A thread that panicked carries its panic on.
                (handles.into_iter())
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|e| std::panic::resume_unwind(e))
                    })
                    .collect()
            });
            for part in parts {
                found.extend(part);
            }
        }
    }
    done.found = found.into_iter().collect();
    done
}

/// The words a name is written in: split at `_`, where a lower-case letter meets a capital, before the capital that opens a word after capitals, and around digits.
pub fn words_of(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    for part in name.split('_') {
        let chars: Vec<char> = part.chars().collect();
        let mut word = String::new();
        for (n, c) in chars.iter().enumerate() {
            let prev = n.checked_sub(1).and_then(|p| chars.get(p));
            let next = chars.get(n + 1);
            let cut = match prev {
                None => false,
                Some(p) => {
                    (p.is_ascii_lowercase() && c.is_ascii_uppercase())
                        || (p.is_ascii_digit() != c.is_ascii_digit())
                        || (p.is_ascii_uppercase()
                            && c.is_ascii_uppercase()
                            && next.is_some_and(char::is_ascii_lowercase))
                }
            };
            if cut && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            word.push(*c);
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    words
}

/// One search under way.
struct Run<'a> {
    targets: &'a Targets,
    lexicon: &'a Lexicon,
    found: BTreeSet<Found>,
    tried: u64,
    /// The joins the budget still allows.
    left: u64,
    /// The first words this run takes: those at `.0` and every `.1`th after it.
    first: (usize, usize),
}

impl Run<'_> {
    /// Tries every join of `length` words in `shape`; `false` where the budget ended it first.
    // ⚠ `length` is 1 to `MAX_JOIN`. The hash of a join is carried word by word: `state[d]` is
    // the hash, not yet finished, of everything before the word at place `d`.
    fn joins(&mut self, length: usize, shape: &Shape) -> bool {
        let (lexicon, targets) = (self.lexicon, self.targets);
        let count = lexicon.lower.len() as u64;
        let last = length.clamp(1, MAX_JOIN) - 1;
        let (offset, stride) = self.first;
        let mut at = [0usize; MAX_JOIN];
        at[0] = offset;
        let mut state = [0u32; MAX_JOIN];
        state[0] = partial_literal_hash(shape.before.to_ascii_lowercase().as_bytes(), 0);
        let mut d = 0;
        loop {
            if d < last {
                let Some(lower) = lexicon.lower.get(at[d]) else {
                    return true;
                };
                let word = partial_literal_hash(lower, state[d]);
                state[d + 1] = partial_literal_hash(shape.between.as_bytes(), word);
                d += 1;
                at[d] = 0;
                continue;
            }
            if self.left < count {
                return false;
            }
            self.left -= count;
            self.tried += count;
            for (n, lower) in lexicon.lower.iter().enumerate() {
                // A one-word join's last word is its first.
                if last == 0 && n % stride != offset {
                    continue;
                }
                let hash = finalize_hash(partial_literal_hash(lower, state[d]));
                if let Some(sought) = targets.sought(hash) {
                    at[d] = n;
                    self.hit(hash, sought, &at[..=last], shape);
                }
            }
            // The next join of the words before the last, or the end where there is none.
            loop {
                if d == 0 {
                    return true;
                }
                d -= 1;
                at[d] += if d == 0 { stride } else { 1 };
                if at[d] < lexicon.lower.len() {
                    break;
                }
            }
        }
    }

    /// Keeps each spelling of the join `at` whose 64-bit hash is one sought beside the 32-bit hash `hash`.
    fn hit(&mut self, hash: u32, sought: &[(Target, u64)], at: &[usize], shape: &Shape) {
        for (kind, hash64) in sought {
            // A program's name is hashed in lower case both times: it has the one spelling.
            let cases = match kind {
                Target::Program => LOWER..LOWER + 1,
                Target::Member => 0..CASES.pow(at.len() as u32),
            };
            for case in cases {
                let name = self.spelled(at, shape, case, *kind);
                if literal_hash64(name.as_bytes()) == *hash64 {
                    self.found.insert(Found {
                        kind: *kind,
                        hash64: *hash64,
                        hash32: hash,
                        name,
                    });
                    break;
                }
            }
        }
    }

    /// The join `at` with word `n` in the case the `n`th digit of `cases`, base [`CASES`], says.
    fn spelled(&self, at: &[usize], shape: &Shape, mut cases: u32, kind: Target) -> String {
        let mut name = match kind {
            Target::Program => shape.before.to_ascii_lowercase(),
            Target::Member => shape.before.to_owned(),
        };
        for (n, word) in at.iter().enumerate() {
            if n > 0 {
                name.push_str(shape.between);
            }
            let seen = self.lexicon.seen.get(*word).map_or("", String::as_str);
            match cases % CASES {
                0 => name.push_str(seen),
                1 => {
                    let mut letters = seen.chars();
                    name.extend(letters.next().map(|first| first.to_ascii_uppercase()));
                    name.push_str(&letters.as_str().to_ascii_lowercase());
                }
                LOWER_CASE => name.push_str(&seen.to_ascii_lowercase()),
                _ => name.push_str(&seen.to_ascii_uppercase()),
            }
            cases /= CASES;
        }
        name
    }
}

/// The cases a word is written in: as its list spells it, with a capital first, lower, upper.
const CASES: u32 = 4;

/// The case that is lower, as a digit of a join's cases.
const LOWER_CASE: u32 = 2;

/// The cases of a join every word of which is in lower case, whatever its length.
// Each base-4 digit of `0xAA` is 2.
const LOWER: u32 = 0xAA;

#[cfg(test)]
#[path = "names_search_tests.rs"]
mod tests;
