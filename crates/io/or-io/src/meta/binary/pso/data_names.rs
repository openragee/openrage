//! Names for GTA VI's data-side hash values, which hash lower-cased text: the 32-bit `atStringHash`
//! and, with the `gta6` feature, the 64-bit string hash. Schema names hash literally and are not here.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use crate::meta::binary::hash_index::{default_hash_index, HashIndex};
use crate::meta::instance::value::HashedString;

/// Longest text a document's own string may be to name a hash value.
const NAME_LEN: usize = 256;

#[derive(Debug, Default)]
struct Names {
    by32: HashMap<u32, String>,
    by64: HashMap<u64, String>,
}

impl Names {
    fn add(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.by32
            .entry(crate::string_hash(bytes))
            .or_insert_with(|| text.to_string());
        #[cfg(feature = "gta6")]
        self.by64
            .entry(crate::gta6::hash::name_hash64(bytes))
            .or_insert_with(|| text.to_string());
    }
}

/// The names `index` carries, worked out once for each index the process makes.
// ⚠ Hashing every name of an index twice costs far more than reading a small document.
fn of_index(index: &'static HashIndex) -> &'static Names {
    static MADE: Mutex<Vec<(usize, &'static Names)>> = Mutex::new(Vec::new());
    let key = std::ptr::from_ref(index) as usize;
    let mut made = MADE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, names)) = made.iter().find(|(made, _)| *made == key) {
        return names;
    }
    // ⚠ Names that differ in case alone hash alike here, and the first added names the hash. An
    // index holds its names in an order of its own, so they are added sorted: every index that
    // holds the same spellings, and every run, takes the same one.
    let mut held: Vec<&str> = index.names().collect();
    held.sort_unstable();
    let mut names = Names::default();
    for name in held {
        names.add(name);
    }
    // At most one for each of the few indexes the process makes, each held to its end.
    let names: &'static Names = Box::leak(Box::new(names));
    made.push((key, names));
    names
}

#[derive(Debug)]
pub(crate) struct DataNames {
    /// The names of the tree's hash index, which stand before a document's own.
    tree: &'static Names,
    own: Names,
}

impl Default for DataNames {
    fn default() -> Self {
        static NONE: LazyLock<Names> = LazyLock::new(Names::default);
        Self {
            tree: &NONE,
            own: Names::default(),
        }
    }
}

impl DataNames {
    /// The names the tree's hash index carries.
    pub(crate) fn of_tree() -> Self {
        Self {
            tree: of_index(default_hash_index()),
            own: Names::default(),
        }
    }

    fn add(&mut self, text: &str) {
        let bytes = text.as_bytes();
        if !self.tree.by32.contains_key(&crate::string_hash(bytes)) {
            self.own
                .by32
                .entry(crate::string_hash(bytes))
                .or_insert_with(|| text.to_string());
        }
        #[cfg(feature = "gta6")]
        if !(self.tree.by64).contains_key(&crate::gta6::hash::name_hash64(bytes)) {
            self.own
                .by64
                .entry(crate::gta6::hash::name_hash64(bytes))
                .or_insert_with(|| text.to_string());
        }
    }

    /// A 32-bit data hash, with the name a lower-cased text gives it where one does.
    pub(crate) fn hashed(&self, hash: u32) -> HashedString {
        let name = (self.tree.by32.get(&hash)).or_else(|| self.own.by32.get(&hash));
        HashedString {
            raw: name.cloned().unwrap_or_default(),
            hash,
        }
    }

    /// A namespaced 32-bit hash (0x4D), which hashes lower-cased text mostly and the text as
    /// written for some members, so a literal name names it where no lower-cased one does.
    pub(crate) fn ns_hashed(&self, hash: u32) -> HashedString {
        let mut hashed = self.hashed(hash);
        if hashed.raw.is_empty() {
            if let Some(name) = default_hash_index().lookup(hash) {
                hashed.raw = name.to_string();
            }
        }
        hashed
    }

    pub(crate) fn name64(&self, hash: u64) -> Option<&str> {
        let name = (self.tree.by64.get(&hash)).or_else(|| self.own.by64.get(&hash));
        name.map(String::as_str)
    }

    /// Adds the texts of `unnamed` that name one of its hash values; false when none does.
    pub(crate) fn add_naming(&mut self, unnamed: &Unnamed) -> bool {
        let mut added = false;
        for text in &unnamed.texts {
            let bytes = text.as_bytes();
            #[cfg(feature = "gta6")]
            let wide = unnamed
                .wide
                .contains(&crate::gta6::hash::name_hash64(bytes));
            #[cfg(not(feature = "gta6"))]
            let wide = false;
            if wide || unnamed.narrow.contains(&crate::string_hash(bytes)) {
                self.add(text);
                added = true;
            }
        }
        added
    }
}

/// The hash values a read left without a name, and the texts it read that might name them.
#[derive(Debug, Default)]
pub(crate) struct Unnamed {
    pub(crate) narrow: HashSet<u32>,
    pub(crate) wide: HashSet<u64>,
    pub(crate) texts: Vec<String>,
}

impl Unnamed {
    pub(crate) fn text(&mut self, text: &str) {
        if !text.is_empty() && text.len() <= NAME_LEN {
            self.texts.push(text.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_s_names_are_worked_out_once_and_a_document_s_own_are_added_beside_them() {
        assert!(std::ptr::eq(
            DataNames::of_tree().tree,
            DataNames::of_tree().tree
        ));
        let mut names = DataNames::of_tree();
        let hash = crate::string_hash(b"a text no index names");
        assert!(names.add_naming(&Unnamed {
            narrow: HashSet::from([hash]),
            wide: HashSet::new(),
            texts: vec!["a text no index names".into()],
        }));
        assert_eq!(names.hashed(hash).raw, "a text no index names");
        assert_eq!(DataNames::of_tree().hashed(hash).raw, "");
    }

    #[test]
    fn of_two_names_that_differ_in_case_alone_every_index_takes_the_same() {
        // Each index holds the two in an order of its own: a map's order is its own.
        let hash = crate::string_hash(b"zqname");
        for _ in 0..16 {
            let index = HashIndex::default().with_pairs([
                (1, "zqName".to_string()),
                (2, "zqname".to_string()),
                (3, "ZQNAME".to_string()),
            ]);
            let names = of_index(Box::leak(Box::new(index)));
            assert_eq!(names.by32[&hash], "ZQNAME");
        }
    }
}
