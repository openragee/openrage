//! Value trees and catalogues this crate's own tests build by hand.

use or_io::container::rpf::RpfEntryKind;
use or_io::meta::instance::value::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_title::vfs::EntryFacts;

use or_world_gta5::catalogue::Catalogue;

/// A catalogue over a list of logical paths, as a mount would produce them.
pub fn catalogue_of(paths: &[&str]) -> Catalogue {
    let mut cat = Catalogue::new();
    for path in paths {
        let leaf = path.rsplit('/').next().unwrap_or(path).to_string();
        cat.index(&EntryFacts {
            logical: (*path).to_string(),
            container: "test.rpf".into(),
            leaf,
            kind: RpfEntryKind::Resource,
            stored_size: 32,
            size: 32,
            compressed: false,
            paging: Some((0x8000_0000, 0x1000_0000)),
        });
    }
    cat
}

/// A `CMapTypes` declaring `(name, assetName, textureDictionary,
/// physicsDictionary)`, every archetype stating `ASSET_TYPE_DRAWABLE`.
pub fn archetype_types(owner: &str, archetypes: &[(&str, &str, &str, &str)]) -> MetaInstance {
    let items = archetypes
        .iter()
        .map(|(name, asset, txd, ybn)| {
            let mut arch = MetaInstance::new("CBaseArchetypeDef");
            arch.set("name", Value::Hash(HashedString::new(*name)));
            arch.set("assetName", Value::Hash(HashedString::new(*asset)));
            arch.set(
                "assetType",
                Value::Enum {
                    type_name: "assetType".into(),
                    variant: "ASSET_TYPE_DRAWABLE".into(),
                },
            );
            if !txd.is_empty() {
                arch.set("textureDictionary", Value::Hash(HashedString::new(*txd)));
            }
            if !ybn.is_empty() {
                arch.set("physicsDictionary", Value::Hash(HashedString::new(*ybn)));
            }
            arch.set("bbMin", Value::Vec3([-1.0; 3]));
            arch.set("bbMax", Value::Vec3([1.0; 3]));
            Value::Pointer(Some(Box::new(arch)))
        })
        .collect();
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new(owner)));
    root.set("archetypes", Value::Array(items));
    root
}

/// A `CMapData` placing the named archetypes as entities and grass batches.
pub fn map_placing(name: &str, entities: &[&str], grass: &[&str]) -> MetaInstance {
    let placed = entities
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut ent = MetaInstance::new("CEntityDef");
            ent.set("archetypeName", Value::Hash(HashedString::new(*a)));
            ent.set("position", Value::Vec3([i as f32, 0.0, 0.0]));
            Value::Pointer(Some(Box::new(ent)))
        })
        .collect();
    let batches = grass
        .iter()
        .map(|a| {
            let mut batch = MetaInstance::new("rage__spdGrassInstanceCluster");
            batch.set("archetypeName", Value::Hash(HashedString::new(*a)));
            Value::Struct(Box::new(batch))
        })
        .collect();

    let mut instanced = MetaInstance::new("CMapData__CInstanceData");
    instanced.set("GrassInstanceList", Value::Array(batches));

    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set("entities", Value::Array(placed));
    root.set("instancedData", Value::Struct(Box::new(instanced)));
    root
}
