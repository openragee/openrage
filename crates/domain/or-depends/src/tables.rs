//! The install-wide tables a per-file read cannot supply.
//!
//! A texture dictionary's parent, an asset's high-detail dictionary, a map
//! group's static bounds and an interior's bounds are each stated about a
//! file by something else — `gtxd.meta` and `_manifest.ymf` — so none is
//! reachable from the file that needs it.

use std::collections::HashMap;

use or_io::hash::literal_string_hash;
use or_io::meta::MetaInstance;

/// `CTxdRelationship`'s two fields, and the array holding them.
mod keys {
    use or_io::hash::literal_string_hash;

    pub const TXD_RELATIONSHIPS: u32 = literal_string_hash(b"txdRelationships");
    pub const PARENT: u32 = literal_string_hash(b"parent");
    pub const CHILD: u32 = literal_string_hash(b"child");
    pub const CONTENT: u32 = literal_string_hash(b"Content");
    pub const MAP_DATA_GROUPS: u32 = literal_string_hash(b"MapDataGroups");
    pub const INTERIORS: u32 = literal_string_hash(b"Interiors");
    pub const NAME: u32 = literal_string_hash(b"Name");
    pub const BOUNDS: u32 = literal_string_hash(b"Bounds");
}

/// A `parent` or `child`, whichever depth the container put it at.
///
/// ⚠ RBF wraps the string one level deeper than XML: `gtxd.meta` writes
/// `<parent>GlobalRoadsLODs</parent>` and `gtxd.ymt` writes a `parent` struct
/// holding a `Content` string, so reading the field directly returns nothing
/// for the base game's 4,000-odd relationships while working on every DLC's
/// fifteen.
fn dictionary_name(entry: &MetaInstance, key: u32) -> Option<u32> {
    if let Some(hash) = or_world::field::hash_at(entry, key) {
        return Some(hash);
    }
    let inner = or_world::field::struct_at(entry, key)?;
    or_world::field::hash_at(inner, keys::CONTENT)
}

/// What an install states about files, beyond what the files state.
#[derive(Debug, Default, Clone)]
pub struct InstallTables {
    txd_parent: HashMap<u32, u32>,
    hd_texture: HashMap<u32, u32>,
    static_bounds: HashMap<u32, Vec<u32>>,
    interior_bounds: HashMap<u32, Vec<u32>>,
    relationship_files: usize,
    manifests: usize,
}

impl InstallTables {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a `gtxd.meta`'s `txdRelationships`.
    ///
    /// ⚠ `child` is the key, not `parent`: each entry says "this one falls
    /// through to that one", so a table keyed the other way resolves every
    /// chain backwards.
    pub fn add_relationships(&mut self, root: &MetaInstance) -> usize {
        let before = self.txd_parent.len();
        for entry in relationship_entries(root) {
            let (Some(child), Some(parent)) = (
                dictionary_name(entry, keys::CHILD),
                dictionary_name(entry, keys::PARENT),
            ) else {
                continue;
            };
            if child == 0 || parent == 0 || child == parent {
                continue;
            }
            self.txd_parent.insert(child, parent);
        }
        let added = self.txd_parent.len() - before;
        if added > 0 {
            self.relationship_files += 1;
        }
        added
    }

    /// Read a manifest's `HDTxdBindingArray`, its `MapDataGroups` bounds and
    /// its `Interiors` bounds.
    ///
    /// ⚠ The first manifest to name a group or an interior keeps it, as the
    /// game stores it; every repeated group (21) and interior (14) on both
    /// retail installs is the same list.
    pub fn add_manifest(&mut self, manifest: or_world::Manifest<'_>) {
        self.manifests += 1;
        for (asset, hd) in manifest.hd_txd_bindings() {
            if asset != 0 && hd != 0 {
                self.hd_texture.insert(asset, hd);
            }
        }
        let (groups, interiors) = stated_bounds(manifest);
        self.add_bounds(groups, interiors);
    }

    /// Take what [`stated_bounds`] read of one manifest, the first to name a group or an interior keeping it.
    pub(crate) fn add_bounds(&mut self, groups: Vec<NamedBounds>, interiors: Vec<NamedBounds>) {
        for (name, bounds) in groups {
            self.static_bounds.entry(name).or_insert(bounds);
        }
        for (name, bounds) in interiors {
            self.interior_bounds.entry(name).or_insert(bounds);
        }
    }

    pub fn set_txd_parent(&mut self, child: u32, parent: u32) {
        self.txd_parent.insert(child, parent);
    }

    pub fn txd_parent_of(&self, child: u32) -> Option<u32> {
        self.txd_parent.get(&child).copied()
    }

    pub fn hd_texture_for(&self, asset: u32) -> Option<u32> {
        self.hd_texture.get(&asset).copied()
    }

    /// The `.ybn` a manifest names for the map group `map`, in its order.
    pub fn static_bounds_of(&self, map: u32) -> &[u32] {
        self.static_bounds.get(&map).map_or(&[], Vec::as_slice)
    }

    /// The map groups a manifest names static bounds for.
    pub fn bounded_groups(&self) -> impl Iterator<Item = u32> + '_ {
        self.static_bounds.keys().copied()
    }

    /// How many map groups a manifest names static bounds for.
    pub fn static_bound_groups(&self) -> usize {
        self.static_bounds.len()
    }

    /// The `.ybn` a manifest names for the interior archetype `interior`, in its order.
    pub fn interior_bounds_of(&self, interior: u32) -> &[u32] {
        self.interior_bounds
            .get(&interior)
            .map_or(&[], Vec::as_slice)
    }

    /// The interior archetypes a manifest names bounds for.
    pub fn bounded_interiors(&self) -> impl Iterator<Item = u32> + '_ {
        self.interior_bounds.keys().copied()
    }

    pub fn relationships(&self) -> usize {
        self.txd_parent.len()
    }

    pub fn hd_bindings(&self) -> usize {
        self.hd_texture.len()
    }

    pub fn relationship_files(&self) -> usize {
        self.relationship_files
    }

    pub fn manifests(&self) -> usize {
        self.manifests
    }
}

/// A map group or an interior archetype, and the `.ybn` a manifest names for it, in its order.
pub(crate) type NamedBounds = (u32, Vec<u32>);

/// What a manifest's `MapDataGroups` and its `Interiors` name bounds for, each in the manifest's order.
pub(crate) fn stated_bounds(
    manifest: or_world::Manifest<'_>,
) -> (Vec<NamedBounds>, Vec<NamedBounds>) {
    let root = manifest.instance();
    (
        named_bounds(root, keys::MAP_DATA_GROUPS).collect(),
        named_bounds(root, keys::INTERIORS).collect(),
    )
}

/// Each `{ Name, Bounds }` item of the manifest array `key` that names both.
fn named_bounds(root: &MetaInstance, key: u32) -> impl Iterator<Item = (u32, Vec<u32>)> + '_ {
    or_world::field::structs_at(root, key).filter_map(|item| {
        let name = or_world::field::hash_at(item, keys::NAME)?;
        let bounds: Vec<u32> = or_world::field::array_at(item, keys::BOUNDS)
            .iter()
            .filter_map(|v| match v {
                or_io::meta::Value::Hash(h) => Some(h.hash),
                or_io::meta::Value::String(s) => Some(or_io::string_hash(s.as_bytes())),
                _ => None,
            })
            .filter(|b| *b != 0)
            .collect();
        (name != 0 && !bounds.is_empty()).then_some((name, bounds))
    })
}

/// The relationship entries, whichever shape the container gave them.
///
/// ⚠ The same document is a list in XML and not in RBF: the base game's
/// `gtxd.ymt` is 8,950 structs and zero arrays, its entries repeated fields
/// of one struct because RBF has no array node. Walking only `Value::Array`
/// reads every DLC's relationships and none of the base game's.
fn relationship_entries(root: &MetaInstance) -> Vec<&MetaInstance> {
    match or_world::field::get(root, keys::TXD_RELATIONSHIPS) {
        Some(or_io::meta::Value::Array(items)) => items
            .iter()
            .filter_map(or_world::field::as_struct)
            .collect(),
        Some(value) => match or_world::field::as_struct(value) {
            Some(inst) => inst
                .fields
                .iter()
                .filter_map(|(_, v)| or_world::field::as_struct(v))
                .collect(),
            None => Vec::new(),
        },
        None => Vec::new(),
    }
}

/// Whether a value tree is a texture-relationship document.
pub fn is_txd_relationships(root: &MetaInstance) -> bool {
    or_world::field::get(root, keys::TXD_RELATIONSHIPS).is_some()
        || or_world::field::name_hash(&root.type_name) == literal_string_hash(b"CTxdRelationships")
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::Value;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn relationships(pairs: &[(&str, &str)]) -> MetaInstance {
        let items = pairs
            .iter()
            .map(|(parent, child)| {
                let mut inst = MetaInstance::new("CTxdRelationship");
                inst.set("parent", Value::Hash(HashedString::new(*parent)));
                inst.set("child", Value::Hash(HashedString::new(*child)));
                Value::Struct(Box::new(inst))
            })
            .collect();
        let mut root = MetaInstance::new("CTxdRelationships");
        root.set("txdRelationships", Value::Array(items));
        root
    }

    #[test]
    fn a_relationship_is_keyed_by_the_dictionary_that_falls_through() {
        let tree = relationships(&[("root_txd", "leaf_txd")]);
        let mut tables = InstallTables::new();
        assert_eq!(tables.add_relationships(&tree), 1);

        assert_eq!(
            tables.txd_parent_of(hash("leaf_txd")),
            Some(hash("root_txd"))
        );
        assert_eq!(
            tables.txd_parent_of(hash("root_txd")),
            None,
            "keyed the wrong way this would answer"
        );
        assert_eq!(tables.relationship_files(), 1);
    }

    #[test]
    fn a_self_parent_is_refused_at_the_table() {
        let tree = relationships(&[("same", "same")]);
        let mut tables = InstallTables::new();
        assert_eq!(tables.add_relationships(&tree), 0);
        assert_eq!(tables.relationships(), 0);
    }

    #[test]
    fn entries_read_whether_they_are_a_list_or_repeated_fields() {
        let item = |parent: &str, child: &str| {
            let mut inst = MetaInstance::new("Item");
            inst.set("parent", Value::Hash(HashedString::new(parent)));
            inst.set("child", Value::Hash(HashedString::new(child)));
            inst
        };

        let mut repeated = MetaInstance::new("CMapParentTxds");
        let mut list = MetaInstance::new("txdRelationships");
        list.fields
            .push(("Item".into(), Value::Struct(Box::new(item("p1", "c1")))));
        list.fields
            .push(("Item".into(), Value::Struct(Box::new(item("p2", "c2")))));
        repeated.set("txdRelationships", Value::Struct(Box::new(list)));

        let mut tables = InstallTables::new();
        assert_eq!(tables.add_relationships(&repeated), 2);
        assert_eq!(tables.txd_parent_of(hash("c2")), Some(hash("p2")));

        let as_array = relationships(&[("p1", "c1"), ("p2", "c2")]);
        let mut other = InstallTables::new();
        assert_eq!(other.add_relationships(&as_array), 2);
    }

    #[test]
    fn a_name_reads_whether_it_is_the_field_or_sits_under_content() {
        let wrapped = |name: &str| {
            let mut inst = MetaInstance::new("String");
            inst.set("Content", Value::String(name.into()));
            Value::Struct(Box::new(inst))
        };
        let mut item = MetaInstance::new("item");
        item.set("parent", wrapped("deep_parent"));
        item.set("child", wrapped("deep_child"));

        let mut list = MetaInstance::new("txdRelationships");
        list.fields
            .push(("item".into(), Value::Struct(Box::new(item))));
        let mut root = MetaInstance::new("CMapParentTxds");
        root.set("txdRelationships", Value::Struct(Box::new(list)));

        let mut tables = InstallTables::new();
        assert_eq!(tables.add_relationships(&root), 1);
        assert_eq!(
            tables.txd_parent_of(hash("deep_child")),
            Some(hash("deep_parent"))
        );
    }

    #[test]
    fn a_relationships_document_is_recognised_under_either_spelling() {
        assert!(is_txd_relationships(&relationships(&[("a", "b")])));
        assert!(is_txd_relationships(&MetaInstance::new(
            "CTxdRelationships"
        )));
        assert!(!is_txd_relationships(&MetaInstance::new("CMapData")));
    }
}
