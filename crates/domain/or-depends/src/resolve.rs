//! An archetype's dependency set, and why each edge is in it.

use std::collections::BTreeSet;

use or_io::gta5::data::asset_name::AssetName;
use or_world_gta5::archetype_assets::{ArchetypeAssetIndex, ArchetypeAssets, Via};
use or_world_gta5::catalogue::{Catalogue, Kind, Located};
use or_world_gta5::contents::ModelContents;
use or_world_gta5::tables::InstallTables;
use or_world_gta5::AssetType;

/// Where the install holds what an edge points at.
///
/// ⚠ `Entry` is a claim the entry table cannot settle: only a resolver
/// [`Resolver::reading`] the `.ydd` or `.ydr` body has confirmed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// A file of its own, at this logical path.
    File(String),
    /// An entry inside the file at this logical path.
    Entry(String),
    /// Nothing in this install holds it.
    Missing,
}

impl Found {
    pub fn path(&self) -> Option<&str> {
        match self {
            Found::File(p) | Found::Entry(p) => Some(p),
            Found::Missing => None,
        }
    }
}

/// One thing an asset needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub via: Via,
    pub kind: Kind,
    pub name: u32,
    pub found: Found,
}

impl Edge {
    pub fn is_resolved(&self) -> bool {
        self.found != Found::Missing
    }

    pub fn logical(&self) -> Option<&str> {
        self.found.path()
    }
}

/// Everything one archetype needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencySet {
    /// The archetype this is for.
    pub of: u32,
    pub edges: Vec<Edge>,
    /// The `assetType` the archetype stated, whether or not it was nameable.
    pub stated_asset_type: Option<AssetType>,
    /// The family the install actually holds `assetName` under.
    pub found_asset_kind: Option<Kind>,
}

impl DependencySet {
    pub fn resolved(&self) -> usize {
        self.edges.iter().filter(|e| e.is_resolved()).count()
    }

    pub fn unresolved(&self) -> usize {
        self.edges.len() - self.resolved()
    }

    /// The edges of one kind, in declaration order.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.kind == kind)
    }

    /// Whether the install corroborated the `assetType` the file stated.
    ///
    /// ⚠ `None` where the archetype names no asset, which is not a
    /// disagreement — an `ASSET_TYPE_ASSETLESS` archetype is correct to
    /// resolve to nothing.
    pub fn asset_type_agrees(&self) -> Option<bool> {
        let stated = self.stated_asset_type?;
        let expected = match stated {
            AssetType::Fragment => Kind::Fragment,
            AssetType::Drawable => Kind::Drawable,
            AssetType::DrawableDictionary => Kind::DrawableDict,
            _ => return None,
        };
        Some(self.found_asset_kind == Some(expected))
    }

    /// The edge carrying the archetype's own model.
    pub fn asset_edge(&self) -> Option<&Edge> {
        self.edges.iter().find(|e| e.via == Via::ArchetypeAsset)
    }
}

/// Resolves a name to the files an install would load for it.
pub struct Resolver<'a> {
    catalogue: &'a Catalogue,
    archetypes: &'a ArchetypeAssetIndex,
    tables: &'a InstallTables,
    contents: Option<&'a ModelContents>,
}

/// How deep a texture-dictionary parent chain is followed.
///
/// ⚠ `gtxd.meta` is data: a cycle stops at the first repeated dictionary,
/// and a chain longer than this is truncated without being reported.
const MAX_TXD_DEPTH: usize = 16;

impl<'a> Resolver<'a> {
    pub fn new(
        catalogue: &'a Catalogue,
        archetypes: &'a ArchetypeAssetIndex,
        tables: &'a InstallTables,
    ) -> Self {
        Self {
            catalogue,
            archetypes,
            tables,
            contents: None,
        }
    }

    /// Settle `.ydd` entries and embedded bounds from bodies read, not names.
    pub fn reading(mut self, contents: &'a ModelContents) -> Self {
        self.contents = Some(contents);
        self
    }

    pub fn catalogue(&self) -> &Catalogue {
        self.catalogue
    }

    pub fn archetypes(&self) -> &ArchetypeAssetIndex {
        self.archetypes
    }

    /// Everything the install would load to render one archetype.
    ///
    /// ⚠ `assetName` is resolved against the install rather than through
    /// `assetType`: the type is an enum, which a build with no schema
    /// snapshot reads as a bare number. [`DependencySet::asset_type_agrees`]
    /// compares the two wherever both are available.
    pub fn of_archetype(&self, name: u32) -> Option<DependencySet> {
        let assets = self.archetypes.get(name)?;
        let mut set = DependencySet {
            of: name,
            stated_asset_type: Some(assets.asset_type),
            ..DependencySet::default()
        };

        if assets.owner != 0 {
            self.push(
                &mut set,
                Via::DeclaringTypeFile,
                Kind::MapTypes,
                assets.owner,
            );
        }

        // ⚠ A declared drawable dictionary outranks a loose file of the same
        // stem: an archetype declaring a `drawableDictionary` holds
        // `assetName` as an entry inside it, so `assetName` is not a filename
        // and looking for one finds nothing or a different asset sharing the
        // stem. The other order reported 76,898 disagreements over a retail
        // install.
        let mut own_bound: Option<&Located> = None;
        if assets.asset_name != 0 {
            let dictionary = (assets.drawable_dictionary != 0)
                .then(|| {
                    self.catalogue
                        .find(assets.drawable_dictionary, Kind::DrawableDict)
                })
                .flatten();
            // ⚠ `.ybn` last: an `ASSET_TYPE_ASSETLESS` archetype can name a
            // collision-only asset (391 on Enhanced, 388 on Legacy, every one
            // assetless), and a stem that is also a model is the model.
            let loose = self.catalogue.find_any(
                assets.asset_name,
                &[Kind::Fragment, Kind::Drawable, Kind::Bounds],
            );

            let holds = self
                .contents
                .and_then(|c| c.dictionary_holds(assets.drawable_dictionary, assets.asset_name));
            let (kind, found) = match (dictionary, loose) {
                (Some(_), _) if holds == Some(false) => (Kind::DrawableDict, Found::Missing),
                (Some(dict), _) => (Kind::DrawableDict, Found::Entry(dict.logical.clone())),
                (None, Some(file)) => (file.kind, Found::File(file.logical.clone())),
                (None, None) => (Kind::Other, Found::Missing),
            };
            set.found_asset_kind = (found != Found::Missing).then_some(kind);
            set.edges.push(Edge {
                via: Via::ArchetypeAsset,
                kind,
                name: assets.asset_name,
                found,
            });
            // A companion is a sibling *file*, so only a loose asset has one.
            if dictionary.is_none() {
                self.push_companion(&mut set, assets.asset_name, loose);
                own_bound = loose.filter(|l| l.kind == Kind::Bounds);
            }
        }

        if assets.drawable_dictionary != 0 {
            self.push(
                &mut set,
                Via::ArchetypeDrawableDictionary,
                Kind::DrawableDict,
                assets.drawable_dictionary,
            );
        }
        if assets.clip_dictionary != 0 {
            self.push(
                &mut set,
                Via::ArchetypeClipDictionary,
                Kind::ClipDict,
                assets.clip_dictionary,
            );
        }
        // ⚠ A `physicsDictionary` that names a `.ybn` or `.ybd` is that file,
        // and both are asked. Nearly every one names no file at all: it marks
        // an archetype whose `.ydr` carries its own bound, and that bound is
        // the collision (13,748 of 13,767 such models on Enhanced, against 0
        // of 20,000 models of archetypes stating none), or one whose asset is
        // itself a `.ybn`, and that file is the collision.
        if assets.physics_dictionary != 0 {
            let file = self
                .catalogue
                .find_any(assets.physics_dictionary, &[Kind::Bounds, Kind::BoundsDict]);
            let embedded = self
                .contents
                .filter(|c| c.drawable_has_bound(assets.asset_name) == Some(true))
                .and_then(|_| self.catalogue.find(assets.asset_name, Kind::Drawable));
            let (kind, found) = match (file, embedded) {
                (Some(file), _) => (file.kind, Found::File(file.logical.clone())),
                (None, Some(model)) => (Kind::Bounds, Found::Entry(model.logical.clone())),
                (None, None) => match own_bound {
                    Some(own) => (Kind::Bounds, Found::File(own.logical.clone())),
                    None => (Kind::Bounds, Found::Missing),
                },
            };
            set.edges.push(Edge {
                via: Via::ArchetypePhysicsDictionary,
                kind,
                name: assets.physics_dictionary,
                found,
            });
        }
        // ⚠ Only a manifest's `Interiors` names an interior's collision, keyed
        // by the archetype; no field of the archetype states it.
        for &bound in self.tables.interior_bounds_of(name) {
            self.push(&mut set, Via::ManifestInterior, Kind::Bounds, bound);
        }

        self.push_textures(&mut set, assets);
        Some(set)
    }

    /// A map's static bounds, or `None` where the install holds no such map.
    ///
    /// ⚠ Only a manifest's `MapDataGroups` names a map's static bounds, and
    /// only for a map that is an imap group (409 groups on Legacy, 401 on
    /// Enhanced). Every other map's `.ybn` streams by its own extents and
    /// nothing names it, so those maps take the `.ybn` their stem names, as a
    /// convention: box overlap cannot pick a bound's map, since one overlaps
    /// hundreds, so it only flags a stem bound outside the map's streaming
    /// box. `physicsDictionaries` lists the placed archetypes' physics
    /// dictionaries, not the map's bounds.
    pub fn of_map(&self, name: u32) -> Option<DependencySet> {
        self.catalogue.find(name, Kind::MapData)?;
        let mut set = DependencySet {
            of: name,
            ..DependencySet::default()
        };
        let stated = self.tables.static_bounds_of(name);
        for &bound in stated {
            self.push(&mut set, Via::ManifestMapDataGroup, Kind::Bounds, bound);
        }
        if stated.is_empty() {
            for &bound in self.catalogue.stem_bounds_of(name) {
                let inside_streaming_box =
                    self.contents.and_then(|c| c.bound_inside_map(bound, name));
                let via = Via::MapStemConvention {
                    inside_streaming_box,
                };
                self.push(&mut set, via, Kind::Bounds, bound);
            }
        }
        Some(set)
    }

    /// The texture dictionaries an archetype's model can draw from, in the
    /// order a lookup falls through them.
    fn push_textures(&self, set: &mut DependencySet, assets: &ArchetypeAssets) {
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        if assets.texture_dictionary != 0 {
            self.push(
                set,
                Via::ArchetypeTextureDictionary,
                Kind::TextureDict,
                assets.texture_dictionary,
            );
            seen.insert(assets.texture_dictionary);
            let mut cursor = assets.texture_dictionary;
            for _ in 0..MAX_TXD_DEPTH {
                let Some(parent) = self.tables.txd_parent_of(cursor) else {
                    break;
                };
                if !seen.insert(parent) {
                    break;
                }
                self.push(set, Via::TextureDictionaryParent, Kind::TextureDict, parent);
                cursor = parent;
            }
        }
        // The manifest binds a high-detail dictionary to the *asset*, not to
        // the archetype, so this is keyed on `assetName`.
        if let Some(hd) = self.tables.hd_texture_for(assets.asset_name) {
            if seen.insert(hd) {
                self.push(set, Via::ManifestHdTexture, Kind::TextureDict, hd);
            }
        }
    }

    /// The `_hi` / `+hi` companion, where the install holds one.
    ///
    /// ⚠ Pushed only when it exists, unlike every other edge here: the
    /// companion is a convention, so reporting an absent one would put a
    /// missing dependency beside every asset in the install.
    fn push_companion(&self, set: &mut DependencySet, name: u32, found: Option<&Located>) {
        let Some(found) = found else { return };
        let companion = match found.kind {
            Kind::Fragment | Kind::Drawable | Kind::DrawableDict => {
                AssetName::parse(&found.stem).companion_with("_hi")
            }
            Kind::TextureDict => AssetName::parse(&found.stem).companion_with("+hi"),
            _ => return,
        };
        if or_io::string_hash(companion.as_bytes()) == name {
            return;
        }
        if let Some(located) = self.catalogue.find_named(&companion, found.kind) {
            set.edges.push(Edge {
                via: Via::HighDetailCompanion,
                kind: found.kind,
                name: or_io::string_hash(companion.as_bytes()),
                found: Found::File(located.logical.clone()),
            });
        }
    }

    fn push(&self, set: &mut DependencySet, via: Via, kind: Kind, name: u32) {
        set.edges.push(Edge {
            via,
            kind,
            name,
            found: self
                .catalogue
                .find(name, kind)
                .map(|l| Found::File(l.logical.clone()))
                .unwrap_or(Found::Missing),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{archetype_types, catalogue_of};
    use or_world_gta5::MapTypes;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    #[test]
    fn an_archetype_resolves_its_model_textures_and_collision() {
        let cat = catalogue_of(&[
            "platform:/prop_bench.ydr",
            "platform:/props_txd.ytd",
            "platform:/props_bounds.ybn",
            "platform:/v_props.ytyp",
        ]);
        let tree = archetype_types(
            "v_props",
            &[("prop_bench", "prop_bench", "props_txd", "props_bounds")],
        );
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let tables = InstallTables::default();
        let resolver = Resolver::new(&cat, &index, &tables);

        let set = resolver.of_archetype(hash("prop_bench")).unwrap();
        assert_eq!(set.unresolved(), 0, "{:#?}", set.edges);
        assert_eq!(set.found_asset_kind, Some(Kind::Drawable));
        assert_eq!(
            set.of_kind(Kind::TextureDict).count(),
            1,
            "one dictionary, no parent chain"
        );
        assert!(set.of_kind(Kind::Bounds).next().unwrap().is_resolved());
        assert!(set
            .edges
            .iter()
            .any(|e| e.via == Via::DeclaringTypeFile && e.is_resolved()));
    }

    #[test]
    fn the_stated_asset_type_is_checked_against_what_the_install_holds() {
        let cat = catalogue_of(&["platform:/prop_x.yft", "platform:/t.ytyp"]);
        let tree = archetype_types("t", &[("prop_x", "prop_x", "", "")]);
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let tables = InstallTables::default();
        let resolver = Resolver::new(&cat, &index, &tables);

        let set = resolver.of_archetype(hash("prop_x")).unwrap();
        // `archetype_types` states ASSET_TYPE_DRAWABLE; the install holds a
        // fragment, so the two disagree and the resolver says so.
        assert_eq!(set.found_asset_kind, Some(Kind::Fragment));
        assert_eq!(set.asset_type_agrees(), Some(false));
    }

    /// ⚠ Dropping an edge the install cannot satisfy would make a broken
    /// archetype indistinguishable from a complete one.
    #[test]
    fn a_dependency_the_install_does_not_hold_is_reported_not_dropped() {
        let cat = catalogue_of(&["platform:/prop_y.ydr", "platform:/t.ytyp"]);
        let tree = archetype_types("t", &[("prop_y", "prop_y", "missing_txd", "")]);
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let tables = InstallTables::default();
        let resolver = Resolver::new(&cat, &index, &tables);

        let set = resolver.of_archetype(hash("prop_y")).unwrap();
        let txd = set.of_kind(Kind::TextureDict).next().unwrap();
        assert_eq!(txd.name, hash("missing_txd"));
        assert!(!txd.is_resolved());
        assert_eq!(set.unresolved(), 1);
    }

    #[test]
    fn a_parent_chain_is_followed_and_a_cycle_terminates() {
        let cat = catalogue_of(&[
            "platform:/child.ytd",
            "platform:/mid.ytd",
            "platform:/root.ytd",
            "platform:/t.ytyp",
            "platform:/m.ydr",
        ]);
        let mut tables = InstallTables::default();
        tables.set_txd_parent(hash("child"), hash("mid"));
        tables.set_txd_parent(hash("mid"), hash("root"));
        tables.set_txd_parent(hash("root"), hash("child"));

        let tree = archetype_types("t", &[("m", "m", "child", "")]);
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let resolver = Resolver::new(&cat, &index, &tables);

        let set = resolver.of_archetype(hash("m")).unwrap();
        let txds: Vec<u32> = set.of_kind(Kind::TextureDict).map(|e| e.name).collect();
        assert_eq!(txds, vec![hash("child"), hash("mid"), hash("root")]);
    }

    #[test]
    fn a_high_detail_companion_is_reported_only_where_one_exists() {
        let with = catalogue_of(&[
            "platform:/prop_a.yft",
            "platform:/prop_a_hi.yft",
            "platform:/t.ytyp",
        ]);
        let without = catalogue_of(&["platform:/prop_a.yft", "platform:/t.ytyp"]);
        let tree = archetype_types("t", &[("prop_a", "prop_a", "", "")]);
        let mut index = ArchetypeAssetIndex::new();
        index.add(MapTypes::new(&tree).unwrap());
        let tables = InstallTables::default();

        let has = Resolver::new(&with, &index, &tables)
            .of_archetype(hash("prop_a"))
            .unwrap();
        assert_eq!(
            has.edges
                .iter()
                .filter(|e| e.via == Via::HighDetailCompanion)
                .count(),
            1
        );
        assert_eq!(has.unresolved(), 0);

        let has_not = Resolver::new(&without, &index, &tables)
            .of_archetype(hash("prop_a"))
            .unwrap();
        assert!(!has_not
            .edges
            .iter()
            .any(|e| e.via == Via::HighDetailCompanion));
        assert_eq!(has_not.unresolved(), 0);
    }

    #[test]
    fn an_inferred_edge_is_distinguishable_from_a_stated_one() {
        assert!(!Via::HighDetailCompanion.is_stated());
        assert!(Via::ArchetypeTextureDictionary.is_stated());
        assert!(Via::TextureDictionaryParent.is_stated());
    }

    #[test]
    fn an_unknown_archetype_resolves_to_nothing_rather_than_an_empty_set() {
        let cat = Catalogue::new();
        let index = ArchetypeAssetIndex::new();
        let tables = InstallTables::default();
        assert!(Resolver::new(&cat, &index, &tables)
            .of_archetype(0xDEAD_BEEF)
            .is_none());
    }
}
