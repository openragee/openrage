//! Folding a whole install into counts, with the denominators kept.

use std::collections::BTreeMap;

use or_world_gta5::AssetType;

use crate::resolve::{DependencySet, Found};
use or_world_gta5::catalogue::{Catalogue, Kind};
use or_world_gta5::Via;

/// A count and what it was out of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Coverage {
    pub considered: u64,
    pub resolved: u64,
}

impl Coverage {
    pub fn record(&mut self, resolved: bool) {
        self.considered += 1;
        self.resolved += u64::from(resolved);
    }

    pub fn unresolved(&self) -> u64 {
        self.considered - self.resolved
    }

    /// ⚠ `None` at a zero denominator rather than zero. A ratio over
    /// nothing is the shape a vacuous run prints as a plausible number.
    pub fn ratio(&self) -> Option<f64> {
        (self.considered > 0).then(|| self.resolved as f64 / self.considered as f64)
    }
}

/// What a run over an install resolved, and how much of it there was.
#[derive(Debug, Clone, Default)]
pub struct Census {
    /// Archetypes the resolver was asked about.
    pub asked: u64,
    /// Archetypes it produced a set for.
    pub answered: u64,
    /// Every edge of every set.
    pub edges: Coverage,
    pub by_via: BTreeMap<Via, Coverage>,
    pub by_kind: BTreeMap<Kind, Coverage>,
    /// Sets in which every edge resolved.
    pub complete: u64,
    /// The install held `assetName` under the family `assetType` stated.
    pub asset_type_agreed: u64,
    pub asset_type_disagreed: u64,
    /// `assetType` was a bare number, so there was nothing to compare.
    pub asset_type_unnameable: u64,
    /// The archetype names no asset, so nothing was expected.
    pub asset_type_names_nothing: u64,
    /// Edges resolved to an entry inside another file rather than to a file.
    pub as_entry: u64,
    /// For an unresolved edge, which other family does hold that name.
    ///
    /// Whether a miss is the install's or the resolver's: a name absent
    /// everywhere is a dependency the install does not hold, a name under a
    /// different extension is this crate looking in the wrong family, and a
    /// total cannot tell the two apart.
    pub missing_found_as: BTreeMap<(Via, Kind), u64>,
    /// Unresolved edges whose name is nowhere in the install under any
    /// family.
    pub missing_entirely: BTreeMap<Via, u64>,
    pub physics: PhysicsRivals,
}

/// Rival readings of `physicsDictionary`, counted together over the same
/// edges.
///
/// Only 4 `.ybd` exist in a retail Legacy install against 15,130 `.ybn`, so
/// `physicsDictionary` cannot usually name a bounds dictionary. ⚠ The
/// readings are counted side by side because testing them in sequence is how
/// a containment artefact gets adopted. They are disjoint by construction and
/// their sum is the denominator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhysicsRivals {
    /// A `.ybn` or `.ybd` of that name exists.
    pub a_bound_file: u64,
    /// No such file, and the archetype's own `.ydr` body carries a bound.
    pub in_the_model: u64,
    /// No such file, and the archetype's asset is itself a `.ybn`.
    pub the_assets_own_bound: u64,
    /// No such file, and the name is the archetype's own `assetName` — so
    /// the bound is inside the model rather than beside it.
    pub the_assets_own_name: u64,
    /// No such file, and the name is the archetype's own `name`.
    pub the_archetypes_own_name: u64,
    /// No such file, and the name is the archetype's `textureDictionary`.
    pub the_texture_dictionary: u64,
    /// None of the above.
    pub neither: u64,
}

impl PhysicsRivals {
    pub fn total(&self) -> u64 {
        self.a_bound_file
            + self.in_the_model
            + self.the_assets_own_bound
            + self.the_assets_own_name
            + self.the_archetypes_own_name
            + self.the_texture_dictionary
            + self.neither
    }
}

/// Where a shader's texture reference is satisfied from.
///
/// ⚠ The tiers are asked in the engine's own fall-through order and each
/// reference is counted once, so the columns sum to the denominator; counting
/// a texture present in two tiers twice would report more satisfied
/// references than there are references.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextureSources {
    /// The model's own embedded dictionary.
    pub embedded: u64,
    /// The `.ytd` the archetype names.
    pub archetype_dictionary: u64,
    /// A dictionary that one falls through to.
    pub parent_dictionary: u64,
    /// The high-detail dictionary a manifest binds to the asset.
    pub manifest_hd: u64,
    /// No tier holds it.
    pub nowhere: u64,
}

impl TextureSources {
    pub fn total(&self) -> u64 {
        self.embedded
            + self.archetype_dictionary
            + self.parent_dictionary
            + self.manifest_hd
            + self.nowhere
    }

    pub fn satisfied(&self) -> u64 {
        self.total() - self.nowhere
    }
}

impl Census {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn miss(&mut self) {
        self.asked += 1;
    }

    pub fn record(&mut self, set: &DependencySet) {
        self.record_probing(set, None)
    }

    /// As [`Self::record`], asking the catalogue what an unresolved name is
    /// held as instead.
    pub fn record_probing(&mut self, set: &DependencySet, probe: Option<&Catalogue>) {
        self.asked += 1;
        self.answered += 1;
        for edge in &set.edges {
            let resolved = edge.is_resolved();
            self.edges.record(resolved);
            self.by_via.entry(edge.via).or_default().record(resolved);
            self.by_kind.entry(edge.kind).or_default().record(resolved);
            if matches!(edge.found, Found::Entry(_)) {
                self.as_entry += 1;
            }
            if let (false, Some(cat)) = (resolved, probe) {
                match Kind::ALL
                    .iter()
                    .filter(|k| **k != Kind::Other)
                    .find(|k| cat.find(edge.name, **k).is_some())
                {
                    Some(elsewhere) => {
                        *self
                            .missing_found_as
                            .entry((edge.via, *elsewhere))
                            .or_default() += 1;
                    }
                    None => *self.missing_entirely.entry(edge.via).or_default() += 1,
                }
            }
        }
        if let Some(physics) = set
            .edges
            .iter()
            .find(|e| e.via == Via::ArchetypePhysicsDictionary)
        {
            let asset_edge = set.asset_edge();
            let asset = asset_edge.map(|e| e.name);
            let txd = set
                .edges
                .iter()
                .find(|e| e.via == Via::ArchetypeTextureDictionary)
                .map(|e| e.name);
            let own_bound = asset_edge.is_some_and(|a| {
                a.kind == Kind::Bounds && a.name != physics.name && a.found == physics.found
            });
            if matches!(physics.found, Found::File(_)) && own_bound {
                self.physics.the_assets_own_bound += 1;
            } else if matches!(physics.found, Found::File(_)) {
                self.physics.a_bound_file += 1;
            } else if matches!(physics.found, Found::Entry(_)) {
                self.physics.in_the_model += 1;
            } else if asset == Some(physics.name) {
                self.physics.the_assets_own_name += 1;
            } else if physics.name == set.of {
                self.physics.the_archetypes_own_name += 1;
            } else if txd == Some(physics.name) {
                self.physics.the_texture_dictionary += 1;
            } else {
                self.physics.neither += 1;
            }
        }
        if set.unresolved() == 0 {
            self.complete += 1;
        }
        match (set.stated_asset_type, set.asset_type_agrees()) {
            (_, Some(true)) => self.asset_type_agreed += 1,
            (_, Some(false)) => self.asset_type_disagreed += 1,
            (Some(AssetType::Ordinal(_)), None) => self.asset_type_unnameable += 1,
            _ => self.asset_type_names_nothing += 1,
        }
    }

    /// The single number a caller asserts a floor on, because a check that
    /// exercises nothing passes: an exercised count once fell from 2,338 to 7
    /// with every exercised case passing, and only an asserted floor caught it.
    pub fn exercised(&self) -> u64 {
        self.edges.considered
    }

    /// Whether this run checked anything at all.
    pub fn is_vacuous(&self) -> bool {
        self.exercised() == 0
    }

    pub fn coverage_of(&self, kind: Kind) -> Coverage {
        self.by_kind.get(&kind).copied().unwrap_or_default()
    }

    pub fn coverage_via(&self, via: Via) -> Coverage {
        self.by_via.get(&via).copied().unwrap_or_default()
    }

    /// Archetypes the type files declared and the resolver could not answer.
    pub fn unanswered(&self) -> u64 {
        self.asked - self.answered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::Resolver;
    use crate::tests_support::{archetype_types, catalogue_of};
    use or_world_gta5::tables::InstallTables;
    use or_world_gta5::ArchetypeAssetIndex;
    use or_world_gta5::MapTypes;

    #[test]
    fn a_coverage_over_nothing_has_no_ratio() {
        let empty = Coverage::default();
        assert_eq!(empty.ratio(), None);
        assert!(Census::new().is_vacuous());

        let mut one = Coverage::default();
        one.record(true);
        assert_eq!(one.ratio(), Some(1.0));
    }

    #[test]
    fn a_census_keeps_the_denominator_per_kind_and_per_route() {
        let cat = catalogue_of(&[
            "platform:/a.ydr",
            "platform:/b.ydr",
            "platform:/shared.ytd",
            "platform:/t.ytyp",
        ]);
        let tree = archetype_types(
            "t",
            &[
                ("a", "a", "shared", ""),
                ("b", "b", "gone", ""),
                ("c", "c", "shared", ""),
            ],
        );
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let tables = InstallTables::default();
        let resolver = Resolver::new(&cat, &index, &tables);

        let mut census = Census::new();
        for name in index.names() {
            match resolver.of_archetype(name) {
                Some(set) => census.record(&set),
                None => census.miss(),
            }
        }

        assert_eq!(census.asked, 3);
        assert_eq!(census.answered, 3);
        assert_eq!(census.complete, 1, "only `a` resolves end to end");
        // Three texture-dictionary edges considered, two of them `shared`.
        let txd = census.coverage_of(Kind::TextureDict);
        assert_eq!((txd.considered, txd.resolved), (3, 2));
        let declaring = census.coverage_via(Via::DeclaringTypeFile);
        assert_eq!((declaring.considered, declaring.resolved), (3, 3));
        assert!(!census.is_vacuous());
        assert_eq!(census.exercised(), census.edges.considered);
    }

    /// The archetype `c` has no file of any family, so its `assetType`
    /// disagrees with an install that holds nothing.
    #[test]
    fn a_stated_type_with_no_file_behind_it_counts_as_a_disagreement() {
        let cat = catalogue_of(&["platform:/t.ytyp"]);
        let tree = archetype_types("t", &[("c", "c", "", "")]);
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let tables = InstallTables::default();
        let resolver = Resolver::new(&cat, &index, &tables);

        let mut census = Census::new();
        census.record(&resolver.of_archetype(or_io::string_hash(b"c")).unwrap());
        assert_eq!(census.asset_type_disagreed, 1);
        assert_eq!(census.asset_type_agreed, 0);
    }
}
