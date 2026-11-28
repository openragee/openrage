//! `_manifest.ymf`: which `.ytyp` each `.ymap` needs before it can load.
//!
//! A map places archetypes by name and does not say where they are declared.
//! The manifest does: `imapDependencies_2` pairs a map name with the list of
//! type files it draws archetypes from, and the streamer loads those before
//! the map. A map absent from every manifest still loads and renders nothing
//! it did not bring with it.
//!
//! ⚠ **So a manifest is the reason create and delete are set operations.** A
//! new map is a file *and* an entry; a deleted map is neither; and either
//! half alone leaves a set that is individually valid and collectively
//! broken.

use std::collections::{BTreeMap, BTreeSet};

use or_io::meta::MetaInstance;

use crate::archetypes::ArchetypeIndex;
use crate::field;
use crate::keys;
use crate::map_data::MapData;

/// A `CPackFileMetaData`, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct Manifest<'a> {
    root: &'a MetaInstance,
}

impl<'a> Manifest<'a> {
    pub fn new(root: &'a MetaInstance) -> Option<Self> {
        field::is_type(root, keys::types::C_PACK_FILE_META_DATA).then_some(Self { root })
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.root
    }

    /// Each map's stated type-file dependencies.
    ///
    /// ⚠ Read from `imapDependencies_2` only. The older flat pairing is
    /// [`Self::flat_imap_dependencies`], and the corpus still ships both
    /// shapes; reading only one under-reports.
    pub fn imap_dependencies(&self) -> impl Iterator<Item = (u32, BTreeSet<u32>)> + 'a {
        field::structs_at(self.root, keys::IMAP_DEPENDENCIES_2).filter_map(|inst| {
            let name = field::hash_at(inst, keys::IMAP_NAME)?;
            Some((name, hashes(inst, keys::ITYP_DEP_ARRAY)))
        })
    }

    /// The older flat pairing, as one entry per `(map, type)`.
    pub fn flat_imap_dependencies(&self) -> impl Iterator<Item = (u32, u32)> + 'a {
        field::structs_at(self.root, keys::IMAP_DEPENDENCIES).filter_map(|inst| {
            Some((
                field::hash_at(inst, keys::IMAP_NAME)?,
                field::hash_at(inst, keys::ITYP_NAME)?,
            ))
        })
    }

    /// The high-detail texture bindings, as `(asset, HD txd)`.
    pub fn hd_txd_bindings(&self) -> impl Iterator<Item = (u32, u32)> + 'a {
        field::structs_at(self.root, keys::HD_TXD_BINDING_ARRAY).filter_map(|inst| {
            Some((
                field::hash_at(inst, keys::TARGET_ASSET)?,
                field::hash_at(inst, keys::HD_TXD)?,
            ))
        })
    }
}

pub(crate) fn hashes(inst: &MetaInstance, key: u32) -> BTreeSet<u32> {
    field::array_at(inst, key)
        .iter()
        .filter_map(|v| match v {
            or_io::meta::Value::Hash(h) => Some(h.hash),
            or_io::meta::Value::UInt(n) => Some(*n as u32),
            _ => None,
        })
        .collect()
}

/// Type files no manifest states because the game never unloads them.
///
/// ⚠ Over a mounted retail Legacy install, 326 maps place an archetype
/// declared in `v_proc1` and **not one of the 1,802 manifests states it** —
/// so a derivation that treats it as a dependency disagrees with every
/// manifest in the game rather than finding a defect. One manifest
/// mentioning any of these falsifies the list.
pub const ALWAYS_RESIDENT: &[&str] = &["v_proc1"];

/// The hashes [`ALWAYS_RESIDENT`] names.
pub fn resident_type_files() -> BTreeSet<u32> {
    ALWAYS_RESIDENT
        .iter()
        .map(|name| or_io::string_hash(name.as_bytes()))
        .collect()
}

/// What a map's own contents say it depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dependencies {
    /// Type files the map's archetypes are declared in.
    pub types: BTreeSet<u32>,
    /// Archetypes no type file in the index declares.
    pub unresolved: usize,
}

/// The archetype each entity and each grass batch of a map places, the empty name left out.
pub fn placed(map: &MapData<'_>) -> Vec<u32> {
    // ⚠ A grass batch places an archetype too, and a map whose only content
    // is grass still needs the type file that declares it.
    map.entities()
        .map(|entity| entity.archetype())
        .chain(map.grass_batches().map(|batch| batch.archetype()))
        .filter(|archetype| *archetype != 0)
        .collect()
}

/// Derive a map's type-file dependencies from what it places.
///
/// ⚠ **Unresolved archetypes are counted, not dropped.** A derived list
/// missing a type file is a map the streamer loads without its models, so a
/// caller comparing against a stated list has to know the derivation was
/// incomplete rather than smaller.
pub fn dependencies_of(map: &MapData<'_>, index: &ArchetypeIndex) -> Dependencies {
    dependencies_of_placed(&placed(map), |archetype| index.owner_of(archetype))
}

/// [`dependencies_of`] for a caller that holds what a map places ([`placed`]) and not the map.
pub fn dependencies_of_placed(
    placed: &[u32],
    owner_of: impl Fn(u32) -> Option<u32>,
) -> Dependencies {
    let mut out = Dependencies::default();
    for archetype in placed {
        match owner_of(*archetype) {
            Some(owner) => {
                out.types.insert(owner);
            }
            None => out.unresolved += 1,
        }
    }
    out
}

/// Every map's stated dependencies, gathered from any number of manifests.
///
/// ⚠ Keyed by map name and **merged**, because a map's dependencies can be
/// stated across more than one manifest — the same reason a map's own content
/// is spread over more than one file.
#[derive(Debug, Clone, Default)]
pub struct StatedDependencies {
    by_map: BTreeMap<u32, BTreeSet<u32>>,
    manifests: usize,
}

impl StatedDependencies {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, manifest: Manifest<'_>) {
        self.manifests += 1;
        for (map, types) in manifest.imap_dependencies() {
            self.by_map.entry(map).or_default().extend(types);
        }
        for (map, ityp) in manifest.flat_imap_dependencies() {
            self.by_map.entry(map).or_default().insert(ityp);
        }
    }

    pub fn of(&self, map: u32) -> Option<&BTreeSet<u32>> {
        self.by_map.get(&map)
    }

    pub fn is_stated(&self, map: u32) -> bool {
        self.by_map.contains_key(&map)
    }

    /// Every map name a manifest states.
    pub fn names(&self) -> impl Iterator<Item = u32> + '_ {
        self.by_map.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.by_map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_map.is_empty()
    }

    pub fn manifest_count(&self) -> usize {
        self.manifests
    }

    /// Roughly how many bytes this holds.
    pub fn footprint(&self) -> usize {
        self.by_map
            .values()
            .map(|set| set.len() * std::mem::size_of::<u32>())
            .sum::<usize>()
            + self.by_map.len() * 48
    }
}

/// How a derived dependency list compares with the stated one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Agreement {
    pub maps: u64,
    /// Maps whose derived and stated lists are identical.
    pub exact: u64,
    /// Maps the manifests state a superset for.
    pub stated_superset: u64,
    /// Maps whose derivation found a type file the manifests do not state.
    ///
    /// ⚠ The direction that breaks a map: a type file the streamer is not
    /// told to load is a model that does not appear.
    pub derived_extra: u64,
    /// Maps no manifest mentions at all.
    pub unstated: u64,
    /// Maps with at least one archetype no type file declares.
    pub incomplete: u64,
}

impl Agreement {
    pub fn record(&mut self, derived: &Dependencies, stated: Option<&BTreeSet<u32>>) {
        self.record_with(derived, stated, &BTreeSet::new())
    }

    /// As [`Self::record`], treating `resident` as stated by everything.
    pub fn record_with(
        &mut self,
        derived: &Dependencies,
        stated: Option<&BTreeSet<u32>>,
        resident: &BTreeSet<u32>,
    ) {
        self.maps += 1;
        if derived.unresolved > 0 {
            self.incomplete += 1;
        }
        let Some(stated) = stated else {
            self.unstated += 1;
            return;
        };
        let needed: BTreeSet<u32> = derived.types.difference(resident).copied().collect();
        if needed == *stated {
            self.exact += 1;
        } else if needed.is_subset(stated) {
            self.stated_superset += 1;
        } else {
            self.derived_extra += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::Value;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn manifest(entries: &[(&str, &[&str])]) -> MetaInstance {
        let items = entries
            .iter()
            .map(|(map, types)| {
                let mut inst = MetaInstance::new("CImapDependencies");
                inst.set("imapName", Value::Hash(HashedString::new(*map)));
                inst.set(
                    "itypDepArray",
                    Value::Array(
                        types
                            .iter()
                            .map(|t| Value::Hash(HashedString::new(*t)))
                            .collect(),
                    ),
                );
                Value::Struct(Box::new(inst))
            })
            .collect();
        let mut root = MetaInstance::new("CPackFileMetaData");
        root.set("imapDependencies_2", Value::Array(items));
        root
    }

    fn map_placing(name: &str, archetypes: &[&str]) -> MetaInstance {
        let items = archetypes
            .iter()
            .map(|a| {
                let mut ent = MetaInstance::new("CEntityDef");
                ent.set("archetypeName", Value::Hash(HashedString::new(*a)));
                Value::Pointer(Some(Box::new(ent)))
            })
            .collect();
        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::Hash(HashedString::new(name)));
        root.set("entities", Value::Array(items));
        root
    }

    fn types_declaring(name: &str, archetypes: &[&str]) -> MetaInstance {
        let items = archetypes
            .iter()
            .map(|a| {
                let mut arch = MetaInstance::new("CBaseArchetypeDef");
                arch.set("name", Value::Hash(HashedString::new(*a)));
                arch.set("bbMin", Value::Vec3([-1.0; 3]));
                arch.set("bbMax", Value::Vec3([1.0; 3]));
                Value::Pointer(Some(Box::new(arch)))
            })
            .collect();
        let mut root = MetaInstance::new("CMapTypes");
        root.set("name", Value::Hash(HashedString::new(name)));
        root.set("archetypes", Value::Array(items));
        root
    }

    fn index_of(trees: &[MetaInstance]) -> ArchetypeIndex {
        let mut index = ArchetypeIndex::new();
        for tree in trees {
            if let Some(types) = crate::map_types::MapTypes::new(tree) {
                index.add(types);
            }
        }
        index
    }

    #[test]
    fn a_manifest_states_a_maps_type_files() {
        let tree = manifest(&[("m1", &["t1", "t2"]), ("m2", &["t1"])]);
        let m = Manifest::new(&tree).unwrap();
        let stated: BTreeMap<u32, BTreeSet<u32>> = m.imap_dependencies().collect();
        assert_eq!(stated.len(), 2);
        assert_eq!(
            stated[&hash("m1")],
            BTreeSet::from([hash("t1"), hash("t2")])
        );
    }

    #[test]
    fn dependencies_come_from_the_archetypes_a_map_places() {
        let types = [
            types_declaring("t1", &["prop_a", "prop_b"]),
            types_declaring("t2", &["prop_c"]),
        ];
        let index = index_of(&types);
        let tree = map_placing("m1", &["prop_a", "prop_c"]);
        let derived = dependencies_of(&MapData::new(&tree).unwrap(), &index);
        assert_eq!(derived.types, BTreeSet::from([hash("t1"), hash("t2")]));
        assert_eq!(derived.unresolved, 0);
    }

    #[test]
    fn an_unresolved_archetype_is_counted_rather_than_dropped() {
        let index = index_of(&[types_declaring("t1", &["prop_a"])]);
        let tree = map_placing("m1", &["prop_a", "prop_missing"]);
        let derived = dependencies_of(&MapData::new(&tree).unwrap(), &index);
        assert_eq!(derived.types, BTreeSet::from([hash("t1")]));
        assert_eq!(derived.unresolved, 1);
    }

    /// ⚠ The two directions of disagreement are not equally bad. A stated
    /// superset is a map that loads more than it needs; a derived extra is a
    /// map that loads without its models.
    #[test]
    fn the_two_directions_of_disagreement_are_reported_apart() {
        let mut agreement = Agreement::default();
        let derived = Dependencies {
            types: BTreeSet::from([1, 2]),
            unresolved: 0,
        };

        agreement.record(&derived, Some(&BTreeSet::from([1, 2])));
        agreement.record(&derived, Some(&BTreeSet::from([1, 2, 3])));
        agreement.record(&derived, Some(&BTreeSet::from([1])));
        agreement.record(&derived, None);

        assert_eq!(agreement.maps, 4);
        assert_eq!(agreement.exact, 1);
        assert_eq!(agreement.stated_superset, 1);
        assert_eq!(agreement.derived_extra, 1);
        assert_eq!(agreement.unstated, 1);
    }

    #[test]
    fn stated_dependencies_merge_across_manifests() {
        let a = manifest(&[("m1", &["t1"])]);
        let b = manifest(&[("m1", &["t2"])]);
        let mut stated = StatedDependencies::new();
        stated.add(Manifest::new(&a).unwrap());
        stated.add(Manifest::new(&b).unwrap());
        assert_eq!(stated.manifest_count(), 2);
        assert_eq!(
            stated.of(hash("m1")).unwrap(),
            &BTreeSet::from([hash("t1"), hash("t2")])
        );
    }

    #[test]
    fn the_flat_pairing_is_read_as_well() {
        let mut dep = MetaInstance::new("CImapDependency");
        dep.set("imapName", Value::Hash(HashedString::new("m1")));
        dep.set("itypName", Value::Hash(HashedString::new("t9")));
        let mut root = MetaInstance::new("CPackFileMetaData");
        root.set(
            "imapDependencies",
            Value::Array(vec![Value::Struct(Box::new(dep))]),
        );

        let mut stated = StatedDependencies::new();
        stated.add(Manifest::new(&root).unwrap());
        assert_eq!(
            stated.of(hash("m1")).unwrap(),
            &BTreeSet::from([hash("t9")])
        );
    }

    #[test]
    fn a_resident_type_file_is_treated_as_stated_by_everything() {
        let resident = resident_type_files();
        assert_eq!(resident.len(), ALWAYS_RESIDENT.len());
        let v_proc1 = hash("v_proc1");
        assert!(resident.contains(&v_proc1));

        let derived = Dependencies {
            types: BTreeSet::from([hash("t1"), v_proc1]),
            unresolved: 0,
        };
        let stated = BTreeSet::from([hash("t1")]);

        let mut without = Agreement::default();
        without.record(&derived, Some(&stated));
        assert_eq!(without.derived_extra, 1, "resident looks like a defect");

        let mut with = Agreement::default();
        with.record_with(&derived, Some(&stated), &resident);
        assert_eq!(with.exact, 1);
        assert_eq!(with.derived_extra, 0);
    }

    #[test]
    fn a_manifest_view_refuses_another_root() {
        assert!(Manifest::new(&MetaInstance::new("CMapData")).is_none());
        assert!(Manifest::new(&MetaInstance::new("0x93A68A2F")).is_some());
    }
}
