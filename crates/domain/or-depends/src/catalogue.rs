//! Turning a name into a file.
//!
//! Every reference in this crate is a `string_hash` of a name with no
//! extension and no path: an archetype's `textureDictionary` is the number
//! `prop_wood` hashes to, and the file is whichever `.ytd` in the mount has
//! that stem. This is the table that says which.

use std::collections::HashMap;

use or_title::vfs::EntryFacts;

/// Which family a file belongs to, by extension.
///
/// ⚠ A model and its textures routinely share a stem — `v_int_1.ydd` beside
/// `v_int_1.ytd` — so an index keyed on the name alone resolves a texture
/// dictionary to a drawable dictionary and reports the wrong file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Drawable,
    Fragment,
    DrawableDict,
    TextureDict,
    Bounds,
    BoundsDict,
    ClipDict,
    MapData,
    MapTypes,
    Manifest,
    Other,
}

impl Kind {
    pub const ALL: &'static [Kind] = &[
        Kind::Drawable,
        Kind::Fragment,
        Kind::DrawableDict,
        Kind::TextureDict,
        Kind::Bounds,
        Kind::BoundsDict,
        Kind::ClipDict,
        Kind::MapData,
        Kind::MapTypes,
        Kind::Manifest,
        Kind::Other,
    ];

    pub fn of_extension(ext: &str) -> Self {
        match ext {
            "ydr" => Kind::Drawable,
            "yft" => Kind::Fragment,
            "ydd" => Kind::DrawableDict,
            "ytd" => Kind::TextureDict,
            "ybn" => Kind::Bounds,
            "ybd" => Kind::BoundsDict,
            "ycd" => Kind::ClipDict,
            "ymap" => Kind::MapData,
            "ytyp" => Kind::MapTypes,
            "ymf" => Kind::Manifest,
            _ => Kind::Other,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Kind::Drawable => "ydr",
            Kind::Fragment => "yft",
            Kind::DrawableDict => "ydd",
            Kind::TextureDict => "ytd",
            Kind::Bounds => "ybn",
            Kind::BoundsDict => "ybd",
            Kind::ClipDict => "ycd",
            Kind::MapData => "ymap",
            Kind::MapTypes => "ytyp",
            Kind::Manifest => "ymf",
            Kind::Other => "",
        }
    }

    pub fn of_leaf(leaf: &str) -> Self {
        match leaf.rsplit_once('.') {
            Some((_, ext)) => Kind::of_extension(&ext.to_ascii_lowercase()),
            None => Kind::Other,
        }
    }
}

/// A file the mount holds, addressed the way a reference addresses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub logical: String,
    pub stem: String,
    pub kind: Kind,
    pub size: u32,
}

/// Every file in a mount, by the hash of its stem and its kind.
#[derive(Debug, Default, Clone)]
pub struct Catalogue {
    by_name: HashMap<(u32, Kind), Located>,
    /// Each `.ybn` whose stem names a map, keyed by that map's name.
    by_map_stem: HashMap<u32, Vec<u32>>,
    entries: u64,
    indexed: u64,
    /// How often a later entry replaced an earlier one under the same key.
    shadowed: u64,
}

impl Catalogue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index every entry a mount enumerates.
    ///
    /// ⚠ Last wins, which is the mount's own overlay order: a DLC pack
    /// shipping `prop_x.ydr` replaces the base game's, and an index fed in
    /// enumeration order resolves the name to the copy the engine loads.
    /// [`Self::shadowed`] counts the replacements, so a mount fed twice shows.
    pub fn index(&mut self, facts: &EntryFacts) {
        self.entries += 1;
        let kind = Kind::of_leaf(&facts.leaf);
        if kind == Kind::Other {
            return;
        }
        let Some((stem, _)) = facts.leaf.rsplit_once('.') else {
            return;
        };
        let stem = stem.to_ascii_lowercase();
        // ⚠ `string_hash`, not `literal_string_hash`. The reference side is
        // an `archetypeName` or a `textureDictionary`, and those are the
        // case-folding flavour; hashing a stem the other way resolves
        // nothing and looks like an install with no textures in it.
        let hash = or_io::string_hash(stem.as_bytes());
        let map = (kind == Kind::Bounds)
            .then(|| map_of_bound_stem(&stem))
            .flatten()
            .map(|map| or_io::string_hash(map.as_bytes()));
        let located = Located {
            logical: facts.logical.clone(),
            stem,
            kind,
            size: facts.size,
        };
        if self.by_name.insert((hash, kind), located).is_some() {
            self.shadowed += 1;
        } else {
            self.indexed += 1;
            if let Some(map) = map {
                self.by_map_stem.entry(map).or_default().push(hash);
            }
        }
    }

    /// The `.ybn` named `<map>_N`, `hi@<map>_N` or `ma@<map>_N`, by stem hash.
    pub fn stem_bounds_of(&self, map: u32) -> &[u32] {
        self.by_map_stem.get(&map).map_or(&[], Vec::as_slice)
    }

    pub fn find(&self, name: u32, kind: Kind) -> Option<&Located> {
        self.by_name.get(&(name, kind))
    }

    pub fn find_named(&self, name: &str, kind: Kind) -> Option<&Located> {
        self.find(or_io::string_hash(name.as_bytes()), kind)
    }

    /// The first of `kinds` this name resolves under.
    ///
    /// ⚠ Order is the caller's claim about precedence, not a fact about the
    /// install: a stem can exist as both a `.ydr` and a `.yft` and only the
    /// archetype's `assetType` says which one is meant.
    pub fn find_any(&self, name: u32, kinds: &[Kind]) -> Option<&Located> {
        kinds.iter().find_map(|k| self.find(name, *k))
    }

    /// Every file of one kind.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Located> {
        self.by_name
            .iter()
            .filter(move |((_, k), _)| *k == kind)
            .map(|(_, located)| located)
    }

    pub fn count_of(&self, kind: Kind) -> usize {
        self.by_name.keys().filter(|(_, k)| *k == kind).count()
    }

    /// Entries seen, including the ones no kind claimed.
    pub fn entries(&self) -> u64 {
        self.entries
    }

    /// Distinct `(name, kind)` pairs held.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn indexed(&self) -> u64 {
        self.indexed
    }

    pub fn shadowed(&self) -> u64 {
        self.shadowed
    }
}

/// The map a static bound's stem names: `<map>_N`, with an optional `hi@` or `ma@`.
pub(crate) fn map_of_bound_stem(stem: &str) -> Option<&str> {
    let stem = (stem.strip_prefix("hi@"))
        .or_else(|| stem.strip_prefix("ma@"))
        .unwrap_or(stem);
    let (map, n) = stem.rsplit_once('_')?;
    (!map.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::RpfEntryKind;

    fn facts(logical: &str) -> EntryFacts {
        let leaf = logical.rsplit('/').next().unwrap_or(logical).to_string();
        EntryFacts {
            logical: logical.to_string(),
            container: "x64a.rpf".into(),
            leaf,
            kind: RpfEntryKind::Resource,
            stored_size: 16,
            size: 16,
            compressed: false,
            paging: Some((0x8000_0000, 0x1000_0000)),
        }
    }

    #[test]
    fn a_stem_shared_by_two_kinds_resolves_to_each_separately() {
        let mut cat = Catalogue::new();
        cat.index(&facts("platform:/levels/v_int_1.ydd"));
        cat.index(&facts("platform:/levels/v_int_1.ytd"));

        let name = or_io::string_hash(b"v_int_1");
        assert_eq!(
            cat.find(name, Kind::DrawableDict).map(|l| &l.logical),
            Some(&"platform:/levels/v_int_1.ydd".to_string())
        );
        assert_eq!(
            cat.find(name, Kind::TextureDict).map(|l| &l.logical),
            Some(&"platform:/levels/v_int_1.ytd".to_string())
        );
        assert_eq!(cat.len(), 2);
        assert_eq!(cat.shadowed(), 0);
    }

    /// ⚠ The reference side hashes case-folding, so the index must too: a
    /// stem indexed under the literal flavour resolves nothing and the
    /// install looks empty rather than mis-keyed.
    ///
    /// ⚠ The two flavours are the same function on a lowercase name, so the
    /// discriminating input has to carry a capital: asserting against
    /// `literal_string_hash` of the folded stem passes whichever flavour the
    /// index uses.
    #[test]
    fn a_reference_hash_is_the_case_folding_flavour() {
        let mut cat = Catalogue::new();
        cat.index(&facts("platform:/levels/Prop_Wood_01.ydr"));

        assert!(cat.find_named("prop_wood_01", Kind::Drawable).is_some());
        assert!(cat.find_named("PROP_WOOD_01", Kind::Drawable).is_some());

        let literal = or_io::hash::literal_string_hash(b"Prop_Wood_01");
        assert_ne!(
            literal,
            or_io::string_hash(b"Prop_Wood_01"),
            "a name with a capital is where the two flavours part"
        );
        assert_eq!(
            cat.find(literal, Kind::Drawable),
            None,
            "indexed under the literal flavour, every reference would miss"
        );
    }

    #[test]
    fn a_later_copy_shadows_an_earlier_one_and_is_counted() {
        let mut cat = Catalogue::new();
        cat.index(&facts("platform:/base/prop_x.ydr"));
        cat.index(&facts("dlc_mpbeach:/prop_x.ydr"));

        assert_eq!(
            cat.find_named("prop_x", Kind::Drawable).map(|l| &l.logical),
            Some(&"dlc_mpbeach:/prop_x.ydr".to_string())
        );
        assert_eq!(cat.len(), 1);
        assert_eq!(cat.indexed(), 1);
        assert_eq!(cat.shadowed(), 1);
    }

    #[test]
    fn an_unclassified_extension_is_seen_and_not_indexed() {
        let mut cat = Catalogue::new();
        cat.index(&facts("common:/data/levels/gta5/notes.txt"));
        assert_eq!(cat.entries(), 1);
        assert!(cat.is_empty());
    }

    #[test]
    fn every_kind_but_other_round_trips_through_its_extension() {
        for kind in Kind::ALL.iter().filter(|k| **k != Kind::Other) {
            assert_eq!(Kind::of_extension(kind.extension()), *kind);
            assert_eq!(Kind::of_leaf(&format!("x.{}", kind.extension())), *kind);
        }
    }
}
