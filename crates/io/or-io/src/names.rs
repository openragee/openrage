//! A reverse index from a RAGE string hash back to the string.

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::hash::{literal_string_hash, string_hash};

/// The shared hash candidates: words only, with no build-specific paths, hashes or file extensions.
pub fn bundled_words() -> impl Iterator<Item = &'static str> {
    [
        include_str!("../data/hash_words_common.txt"),
        include_str!("../data/hash_words_vehicles.txt"),
        include_str!("../data/hash_words_peds.txt"),
        include_str!("../data/hash_words_textures.txt"),
    ]
    .into_iter()
    .flat_map(str::lines)
    .filter(|word| !word.is_empty())
}

fn bundled_index() -> &'static NameIndex {
    static WORDS: std::sync::OnceLock<NameIndex> = std::sync::OnceLock::new();
    WORDS.get_or_init(|| {
        let mut index = NameIndex::default();
        index.extend(bundled_words());
        index
    })
}

/// Which of the two hash functions a number was produced by.
///
/// ⚠ Not interchangeable. Measured over the audio name tables of a retail
/// Legacy install: every name in a `.datNNN.nametable` hashes under
/// [`CaseFolding`](HashFlavour::CaseFolding) to a word present in the sibling
/// `.dat`, and under [`Literal`](HashFlavour::Literal) to one present in none
/// of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HashFlavour {
    /// `atStringHash`: folds case and turns `\` into `/`. Asset and audio
    /// names.
    CaseFolding,
    /// No normalisation. PSO metadata type and field names.
    Literal,
}

impl HashFlavour {
    pub const ALL: [HashFlavour; 2] = [HashFlavour::CaseFolding, HashFlavour::Literal];

    pub fn of(self, name: &str) -> u32 {
        match self {
            HashFlavour::CaseFolding => string_hash(name.as_bytes()),
            HashFlavour::Literal => literal_string_hash(name.as_bytes()),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            HashFlavour::CaseFolding => "case-folding",
            HashFlavour::Literal => "literal",
        }
    }
}

/// A name recovered from a hash, and the function that recovered it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution<'a> {
    pub name: &'a str,
    pub flavour: HashFlavour,
}

/// Two names that hash to the same number under the same function.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Collision {
    pub hash: u32,
    pub flavour: HashFlavour,
    pub kept: String,
    pub dropped: String,
}

/// Every name known for one title, indexed by both hashes.
///
/// ⚠ **One title per index.** An index that spans titles answers a GTA V hash
/// with a Red Dead name whenever the two collide or the same number means
/// different things, and the answer looks exactly like a correct one. The
/// default is the **untitled** index, which answers only with what was put
/// into it; [`NameBook`] is what partitions across titles.
#[derive(Debug, Clone, Default)]
pub struct NameIndex {
    title: String,
    bundled: bool,
    by_hash: HashMap<(HashFlavour, u32), String>,
    names: usize,
    collisions: Vec<Collision>,
}

impl NameIndex {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            bundled: false,
            by_hash: HashMap::new(),
            names: 0,
            collisions: Vec::new(),
        }
    }

    /// An index whose title-specific names take precedence over the shared word candidates.
    pub fn with_bundled(title: impl Into<String>) -> Self {
        Self {
            bundled: true,
            ..Self::new(title)
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Record `name` under both hash functions.
    ///
    /// The empty string is not a name: it hashes to a fixed number under both
    /// functions and would answer every zero-length field in the game.
    pub fn insert(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        let mut fresh = false;
        for flavour in HashFlavour::ALL {
            let hash = flavour.of(name);
            match self.by_hash.get(&(flavour, hash)) {
                Some(kept) if kept == name => {}
                Some(kept) => self.collisions.push(Collision {
                    hash,
                    flavour,
                    kept: kept.clone(),
                    dropped: name.to_string(),
                }),
                None => {
                    self.by_hash.insert((flavour, hash), name.to_string());
                    fresh = true;
                }
            }
        }
        if fresh {
            self.names += 1;
        }
    }

    /// Record `name`, and also the part before its last `.`.
    ///
    /// Both spellings are live: an archive entry is `props.ytd` and the thing
    /// that references it is hashed as `props`.
    pub fn insert_with_stem(&mut self, name: &str) {
        self.insert(name);
        if let Some((stem, _)) = name.rsplit_once('.') {
            self.insert(stem);
        }
    }

    pub fn resolve_with(&self, flavour: HashFlavour, hash: u32) -> Option<&str> {
        self.by_hash
            .get(&(flavour, hash))
            .map(String::as_str)
            .or_else(|| {
                self.bundled
                    .then(|| bundled_index().resolve_with(flavour, hash))
                    .flatten()
            })
    }

    /// Resolve without being told the function, trying the case-folding one
    /// first because that is what names use.
    pub fn resolve(&self, hash: u32) -> Option<Resolution<'_>> {
        HashFlavour::ALL.into_iter().find_map(|flavour| {
            self.resolve_with(flavour, hash)
                .map(|name| Resolution { name, flavour })
        })
    }

    /// The spelling to show when nothing resolves.
    pub fn display(&self, hash: u32) -> String {
        match self.resolve(hash) {
            Some(found) => found.name.to_string(),
            None => format!("hash_{hash:08X}"),
        }
    }

    pub fn names(&self) -> usize {
        self.names
    }

    pub fn is_empty(&self) -> bool {
        self.names == 0
    }

    pub fn collisions(&self) -> &[Collision] {
        &self.collisions
    }

    pub fn extend<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            self.insert(name.as_ref());
        }
    }

    /// Every name, sorted, for writing a name table back out.
    pub fn sorted_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self
            .by_hash
            .iter()
            .filter(|((flavour, _), _)| *flavour == HashFlavour::CaseFolding)
            .map(|(_, name)| name.as_str())
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The indexes for every title, which is the only way to reach one.
#[derive(Debug, Clone, Default)]
pub struct NameBook {
    titles: BTreeMap<String, NameIndex>,
}

impl NameBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_title(&mut self, title: &str) -> &mut NameIndex {
        self.titles
            .entry(title.to_string())
            .or_insert_with(|| NameIndex::new(title))
    }

    pub fn get(&self, title: &str) -> Option<&NameIndex> {
        self.titles.get(title)
    }

    pub fn resolve(&self, title: &str, hash: u32) -> Option<Resolution<'_>> {
        self.titles.get(title)?.resolve(hash)
    }

    pub fn display(&self, title: &str, hash: u32) -> String {
        match self.titles.get(title) {
            Some(index) => index.display(hash),
            None => format!("hash_{hash:08X}"),
        }
    }

    pub fn titles(&self) -> impl Iterator<Item = &str> {
        self.titles.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠ A known answer from shipped game data, not from this function.
    ///
    /// `BIFTA` is an audio object named in
    /// `x64/audio/dlcbeach_game.dat149.nametable`, and 0xEB298297 is a 32-bit
    /// word in the `dlcbeach_game.dat149` beside it. All 42 names across that
    /// pack's three `game.dat` tables land the same way, and the literal hash
    /// lands on none of them. Without this every test of `string_hash` in
    /// this crate is the function agreeing with itself.
    #[test]
    fn the_case_folding_hash_is_the_one_the_game_writes() {
        assert_eq!(HashFlavour::CaseFolding.of("BIFTA"), 0xEB29_8297);
        assert_eq!(HashFlavour::CaseFolding.of("bifta"), 0xEB29_8297);
        assert_eq!(HashFlavour::CaseFolding.of("KALAHARI"), 0x0585_2838);
        assert_eq!(HashFlavour::CaseFolding.of("PARADISE"), 0x58B3_979C);
        assert_eq!(
            HashFlavour::CaseFolding.of("AUDIO_ITEM_SNSPISTOL"),
            0xCF01_6CEF
        );

        // The other half, and what makes the first half load-bearing.
        assert_ne!(HashFlavour::Literal.of("BIFTA"), 0xEB29_8297);
        assert_eq!(HashFlavour::Literal.of("BIFTA"), 0x828C_917B);
    }

    #[test]
    fn a_resolution_says_which_function_found_it() {
        let mut index = NameIndex::new("gta5");
        index.insert("BIFTA");

        let folded = index.resolve(0xEB29_8297).expect("case-folding");
        assert_eq!(folded.name, "BIFTA");
        assert_eq!(folded.flavour, HashFlavour::CaseFolding);

        let literal = index.resolve(0x828C_917B).expect("literal");
        assert_eq!(literal.flavour, HashFlavour::Literal);

        assert_eq!(index.resolve_with(HashFlavour::Literal, 0xEB29_8297), None);
    }

    #[test]
    fn an_unresolved_hash_says_so_rather_than_guessing() {
        let index = NameIndex::new("gta5");
        assert_eq!(index.display(0xDEAD_BEEF), "hash_DEADBEEF");
        assert!(index.resolve(0xDEAD_BEEF).is_none());
    }

    #[test]
    fn a_stem_is_indexed_beside_the_whole_name() {
        let mut index = NameIndex::new("gta5");
        index.insert_with_stem("props.ytd");
        assert_eq!(
            index
                .resolve(HashFlavour::CaseFolding.of("props.ytd"))
                .map(|r| r.name),
            Some("props.ytd")
        );
        assert_eq!(
            index
                .resolve(HashFlavour::CaseFolding.of("props"))
                .map(|r| r.name),
            Some("props")
        );
    }

    /// ⚠ The empty string hashes to a fixed number under both functions, so
    /// indexing it answers every empty field in the game with `""`.
    #[test]
    fn the_empty_string_is_not_a_name() {
        let mut index = NameIndex::new("gta5");
        index.insert("");
        index.insert_with_stem("");
        assert!(index.is_empty());
        assert!(index.resolve(HashFlavour::CaseFolding.of("")).is_none());
    }

    #[test]
    fn a_collision_is_recorded_rather_than_overwriting_the_first_name() {
        let mut index = NameIndex::new("gta5");
        index.insert("BIFTA");
        // Case folding makes these one key and leaves the literal hashes
        // distinct, which is a collision under exactly one flavour.
        index.insert("bifta");

        assert_eq!(
            index.resolve_with(HashFlavour::CaseFolding, 0xEB29_8297),
            Some("BIFTA"),
            "the first name must survive"
        );
        assert_eq!(index.collisions().len(), 1);
        assert_eq!(index.collisions()[0].flavour, HashFlavour::CaseFolding);
        assert_eq!(index.collisions()[0].dropped, "bifta");
        assert_eq!(
            index.resolve_with(HashFlavour::Literal, HashFlavour::Literal.of("bifta")),
            Some("bifta"),
            "the literal hashes differ, so both names are still reachable"
        );
    }

    #[test]
    fn inserting_the_same_name_twice_is_not_a_collision() {
        let mut index = NameIndex::new("gta5");
        index.insert("BIFTA");
        index.insert("BIFTA");
        assert_eq!(index.names(), 1);
        assert!(index.collisions().is_empty());
    }

    /// ⚠ One title's names must never answer another's hash.
    #[test]
    fn a_title_never_answers_with_another_titles_name() {
        let mut book = NameBook::new();
        book.for_title("gta5").insert("BIFTA");
        book.for_title("rdr2").insert("BUCKSKIN");

        assert_eq!(
            book.resolve("gta5", 0xEB29_8297).map(|r| r.name),
            Some("BIFTA")
        );
        assert_eq!(book.resolve("rdr2", 0xEB29_8297), None);
        assert_eq!(book.display("rdr2", 0xEB29_8297), "hash_EB298297");
        assert_eq!(book.resolve("a-title-with-no-names", 0xEB29_8297), None);

        assert_eq!(book.titles().collect::<Vec<_>>(), vec!["gta5", "rdr2"]);
    }

    #[test]
    fn the_untitled_index_is_its_own_thing_rather_than_a_titles() {
        let mut untitled = NameIndex::default();
        untitled.insert("BIFTA");
        assert_eq!(untitled.title(), "");
        assert_eq!(untitled.resolve(0xEB29_8297).map(|r| r.name), Some("BIFTA"));

        // A book never hands one out, so nothing can reach it by title.
        let book = NameBook::new();
        assert_eq!(book.resolve("", 0xEB29_8297), None);
        assert_eq!(book.titles().count(), 0);
    }

    #[test]
    fn sorted_names_round_trip_the_population_that_went_in() {
        let mut index = NameIndex::new("gta5");
        index.extend(["KALAHARI", "BIFTA", "PARADISE"]);
        assert_eq!(index.sorted_names(), vec!["BIFTA", "KALAHARI", "PARADISE"]);
        assert_eq!(index.names(), 3);
    }
}
