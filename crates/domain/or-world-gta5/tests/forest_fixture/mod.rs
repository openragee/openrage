//! Maps, type files and a manifest written with what a level-of-detail forest reads of them: each
//! entity's link, level and distances, an interior's entities, an archetype's files and a group's gate.

#![allow(dead_code)]

use std::collections::HashMap;

use or_io::meta::canonical;
use or_io::meta::instance::HashedString;
use or_io::meta::{build_library, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_world_gta5::world_forest::{interior_key, Model, NoModel};
use or_world_gta5::world_index::Map;
use or_world_gta5::{WorldForest, WorldIndex};

use or_world::streaming::{NodeId, NodeState, View};
use or_world_gta5::WorldState;

use crate::world_fixture::{write_archive, Span};

/// The members a forest reads of a map, a type file and a manifest, and no other.
const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <enumdef type="rage__eLodType">
    <enumval name="LODTYPES_DEPTH_HD"/>
    <enumval name="LODTYPES_DEPTH_LOD"/>
    <enumval name="LODTYPES_DEPTH_SLOD1"/>
    <enumval name="LODTYPES_DEPTH_SLOD2"/>
    <enumval name="LODTYPES_DEPTH_SLOD3"/>
    <enumval name="LODTYPES_DEPTH_ORPHANHD"/>
    <enumval name="LODTYPES_DEPTH_SLOD4"/>
  </enumdef>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <enum name="lodLevel" type="rage__eLodType"/>
    <u32 name="numChildren"/>
  </structdef>
  <structdef type="CMloInstanceDef" base="CEntityDef">
    <u32 name="groupId"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <string name="parent" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="contentFlags"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CBaseArchetypeDef">
    <float name="lodDist"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <string name="name" type="atHashString"/>
    <string name="textureDictionary" type="atHashString"/>
    <string name="drawableDictionary" type="atHashString"/>
    <string name="assetName" type="atHashString"/>
  </structdef>
  <structdef type="CMloArchetypeDef" base="CBaseArchetypeDef">
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CMapTypes">
    <array name="archetypes" type="atArray"><pointer type="CBaseArchetypeDef" policy="owner"/></array>
    <string name="name" type="atHashString"/>
  </structdef>
  <structdef type="CMapDataGroup">
    <string name="Name" type="atHashString"/>
    <array name="Bounds" type="atArray"><string type="atHashString"/></array>
    <u32 name="Flags"/>
    <array name="WeatherTypes" type="atArray"><string type="atHashString"/></array>
    <u32 name="HoursOnOff"/>
  </structdef>
  <structdef type="CPackFileMetaData">
    <array name="MapDataGroups" type="atArray"><struct type="CMapDataGroup"/></array>
  </structdef>
</ParserSchema>"#;

fn encoded(root: &MetaInstance, format: BinaryFormat) -> Vec<u8> {
    let library = build_library([("forest_fixture.psc", PSC.as_bytes())]).expect("the schema");
    let layout = canonical::layout_for(&library, root, format, None).expect("lays out");
    write_meta_binary(format, Some(&layout), root).expect("encodes")
}

fn named(name: &str) -> Value {
    Value::Hash(HashedString::new(name))
}

pub const HD: &str = "LODTYPES_DEPTH_HD";
pub const LOD: &str = "LODTYPES_DEPTH_LOD";
pub const ORPHAN: &str = "LODTYPES_DEPTH_ORPHANHD";
/// A number the level's own table does not hold, which is how a file states no level.
pub const NO_LEVEL: &str = "0x00000009";

/// Bit 3 of an entity's flags: the parent it names by index is an entity of the parent map.
pub const IN_PARENT_MAP: u32 = 1 << 3;

/// One entity as a file states it.
#[derive(Clone)]
pub struct Ent {
    pub archetype: &'static str,
    pub at: [f32; 3],
    /// The four floats stored, which mean one thing of an entity and another of an interior's instance.
    pub rotation: [f32; 4],
    /// One number for the three axes.
    pub scale: f32,
    pub lod_dist: f32,
    pub child_lod_dist: f32,
    /// The index it names, and whether the parent map's entities are meant.
    pub parent: Option<(i32, bool)>,
    pub level: &'static str,
    pub num_children: u32,
    pub interior: bool,
}

/// An entity of the high level with no parent that states no distance.
pub fn ent(archetype: &'static str, at: [f32; 3]) -> Ent {
    Ent {
        archetype,
        at,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        lod_dist: 0.0,
        child_lod_dist: 0.0,
        parent: None,
        level: HD,
        num_children: 0,
        interior: false,
    }
}

/// An entity as one item of a map's or an interior's `entities`.
pub fn entity_value(e: &Ent) -> Value {
    let mut inst = MetaInstance::new(match e.interior {
        true => "CMloInstanceDef",
        false => "CEntityDef",
    });
    let (index, in_parent_map) = e.parent.unwrap_or((-1, false));
    inst.set("archetypeName", named(e.archetype));
    inst.set(
        "flags",
        Value::UInt(u64::from(match in_parent_map {
            true => IN_PARENT_MAP,
            false => 0,
        })),
    );
    inst.set("position", Value::Vec3(e.at));
    inst.set("rotation", Value::Vec4(e.rotation));
    inst.set("scaleXY", Value::Float(f64::from(e.scale)));
    inst.set("scaleZ", Value::Float(f64::from(e.scale)));
    inst.set("parentIndex", Value::Int(i64::from(index)));
    inst.set("lodDist", Value::Float(f64::from(e.lod_dist)));
    inst.set("childLodDist", Value::Float(f64::from(e.child_lod_dist)));
    inst.set(
        "lodLevel",
        Value::Enum {
            type_name: "rage__eLodType".into(),
            variant: e.level.into(),
        },
    );
    inst.set("numChildren", Value::UInt(u64::from(e.num_children)));
    if e.interior {
        inst.set("groupId", Value::UInt(0));
    }
    Value::Pointer(Some(Box::new(inst)))
}

/// One map as its file states it.
#[derive(Clone)]
pub struct MapOf {
    pub parent: &'static str,
    pub flags: u32,
    pub streaming: Span,
    pub extents: Span,
    pub entities: Vec<Ent>,
}

/// A map with no parent that nothing holds back, whose entities stand in `extents` and that streams from 500 m around them.
pub fn map_around(extents: Span, entities: Vec<Ent>) -> MapOf {
    MapOf {
        parent: "",
        flags: 0,
        streaming: (extents.0.map(|v| v - 500.0), extents.1.map(|v| v + 500.0)),
        extents,
        entities,
    }
}

pub fn map_bytes(name: &str, map: &MapOf) -> Vec<u8> {
    map_bytes_of(name, map, map.entities.iter().map(entity_value).collect())
}

/// [`map_bytes`] with `entities` in place of the map's own, for one a file states badly.
pub fn map_bytes_of(name: &str, map: &MapOf, entities: Vec<Value>) -> Vec<u8> {
    let mut root = MetaInstance::new("CMapData");
    root.set("name", named(name));
    root.set("parent", named(map.parent));
    root.set("flags", Value::UInt(u64::from(map.flags)));
    root.set("contentFlags", Value::UInt(1));
    root.set("streamingExtentsMin", Value::Vec3(map.streaming.0));
    root.set("streamingExtentsMax", Value::Vec3(map.streaming.1));
    root.set("entitiesExtentsMin", Value::Vec3(map.extents.0));
    root.set("entitiesExtentsMax", Value::Vec3(map.extents.1));
    root.set("entities", Value::Array(entities));
    encoded(&root, BinaryFormat::Rbm)
}

/// One archetype as a type file declares it.
#[derive(Clone)]
pub struct Arch {
    pub name: &'static str,
    pub lod_dist: f32,
    /// `(assetName, drawableDictionary, textureDictionary)`, each empty where it states none.
    pub files: (&'static str, &'static str, &'static str),
    /// The entities of an interior; `None` for an archetype that is none.
    pub interior: Option<Vec<Ent>>,
    /// An interior's entities begin with a pointer to nothing, as a file states one badly.
    pub holed: bool,
}

/// An archetype drawn by a drawable of its own name, with no dictionary of either kind.
pub fn arch(name: &'static str, lod_dist: f32) -> Arch {
    Arch {
        name,
        lod_dist,
        files: (name, "", ""),
        interior: None,
        holed: false,
    }
}

pub fn types_bytes(name: &str, archetypes: &[Arch]) -> Vec<u8> {
    let items = archetypes.iter().map(|a| {
        let mut inst = MetaInstance::new(match a.interior {
            Some(_) => "CMloArchetypeDef",
            None => "CBaseArchetypeDef",
        });
        inst.set("lodDist", Value::Float(f64::from(a.lod_dist)));
        inst.set("bbMin", Value::Vec3([-1.0; 3]));
        inst.set("bbMax", Value::Vec3([1.0; 3]));
        inst.set("name", named(a.name));
        inst.set("textureDictionary", named(a.files.2));
        inst.set("drawableDictionary", named(a.files.1));
        inst.set("assetName", named(a.files.0));
        if let Some(entities) = &a.interior {
            let hole = a.holed.then_some(Value::Pointer(None));
            let stated = hole.into_iter().chain(entities.iter().map(entity_value));
            inst.set("entities", Value::Array(stated.collect()));
        }
        Value::Pointer(Some(Box::new(inst)))
    });
    let mut root = MetaInstance::new("CMapTypes");
    root.set("archetypes", Value::Array(items.collect()));
    root.set("name", named(name));
    encoded(&root, BinaryFormat::Rbm)
}

/// A manifest whose groups are `(name, flags, hours, weather types)`.
pub fn manifest_bytes(groups: &[(&str, u32, u32, &[&str])]) -> Vec<u8> {
    let items = groups.iter().map(|(name, flags, hours, weather)| {
        let mut item = MetaInstance::new("CMapDataGroup");
        item.set("Name", named(name));
        item.set("Bounds", Value::Array(Vec::new()));
        item.set("Flags", Value::UInt(u64::from(*flags)));
        let weather = weather.iter().map(|w| named(w));
        item.set("WeatherTypes", Value::Array(weather.collect()));
        item.set("HoursOnOff", Value::UInt(u64::from(*hours)));
        Value::Struct(Box::new(item))
    });
    let mut root = MetaInstance::new("CPackFileMetaData");
    root.set("MapDataGroups", Value::Array(items.collect()));
    encoded(&root, BinaryFormat::Pso)
}

/// Bytes of maps a test's forest may hold let go of: more than any install here.
pub const ROOMY: u64 = 1 << 30;

/// The files of one archive, each its path there and its bytes.
pub type Files = Vec<(String, Vec<u8>)>;

/// An install of `archives`, each `(archive, files)`, in a scratch folder the caller keeps.
pub fn install(archives: &[(&str, Files)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch directory");
    // An install mounts only with its `common.rpf`, which here holds nothing of the world.
    write_archive(dir.path(), "common.rpf", &[("data/nothing.txt", vec![0])]);
    for (archive, files) in archives {
        let files: Vec<(&str, Vec<u8>)> = (files.iter())
            .map(|(path, bytes)| (path.as_str(), bytes.clone()))
            .collect();
        write_archive(dir.path(), archive, &files);
    }
    dir
}

pub fn map_file(name: &str, map: &MapOf) -> (String, Vec<u8>) {
    (format!("levels/{name}.ymap"), map_bytes(name, map))
}

pub fn types_file(name: &str, archetypes: &[Arch]) -> (String, Vec<u8>) {
    (format!("types/{name}.ytyp"), types_bytes(name, archetypes))
}

/// What a forest selected over the places asked of it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Selections {
    pub instances: usize,
    /// Instances that stand as a box.
    pub boxes: usize,
    /// Interiors brought in, a place at a time.
    pub interiors: usize,
    /// Entities drawn whole that stand in for the whole of a parent drawn whole too.
    pub beside: usize,
    /// Those of them whose parent states more children than the forest holds of it, which the rules have it wait for.
    pub states_more: usize,
    /// Those of them whose parent states fewer children than the forest holds of it, which the rules never count as all found.
    pub states_fewer: usize,
    /// The rest of them, each by its map, its entity and its archetype: what the rules do not give.
    pub unaccounted: Vec<String>,
}

/// Select from the centre of the entities' extents of each of `maps`, at noon in clear weather with every model ready, and hold each instance to `index`: one with files names an archetype it holds, and a box one it lacks.
pub fn selected_at_the_centres(
    index: &WorldIndex,
    forest: &mut WorldForest<'_>,
    maps: &[&Map],
) -> Selections {
    let state = WorldState::at(12, "CLEAR");
    let mut held = Selections::default();
    for map in maps {
        let (min, max) = map.entities.expect("it states its extents");
        let view = View::at((min + max) / 2.0, 1.0);
        let streamed = forest.stream(&view, &state);
        assert!(
            (streamed.wanted.iter()).any(|w| w.map.hash == map.hash)
                || streamed.unread.iter().any(|(m, _)| m.hash == map.hash),
            "{}: a view at its centre does not want it",
            map.name
        );
        held.interiors += streamed.interiors.len();
        // How many nodes of the forest name each node as their parent, which is the count the rules hold a parent's own to.
        let maps = streamed.wanted.iter().map(|w| u64::from(w.map.hash));
        let interiors = (streamed.interiors.iter()).map(|at| interior_key(at.map, at.entity));
        let mut named: HashMap<NodeId, u32> = HashMap::new();
        for key in maps.chain(interiors) {
            let nodes = (0u32..).map_while(|at| forest.node((key, at)));
            for parent in nodes.filter_map(|node| node.parent) {
                *named.entry((parent.map, parent.node)).or_default() += 1;
            }
        }
        let selected = forest.select(&view, &|_| NodeState::Ready);
        let whole: std::collections::HashSet<_> = (selected.instances.iter())
            .filter(|i| i.fade >= 1.0)
            .map(|i| i.node)
            .collect();
        for instance in &selected.instances {
            let declared = index.archetype(instance.archetype).is_some();
            match instance.model {
                Model::Files(_) => assert!(declared, "{}: {instance:?}", map.name),
                Model::Box(NoModel::Undeclared) => {
                    assert!(!declared, "{}: {instance:?}", map.name)
                }
                Model::Box(NoModel::Unnamed) => assert_eq!(instance.archetype, 0),
            }
            held.boxes += usize::from(matches!(instance.model, Model::Box(_)));
            let node = forest.node(instance.node).expect("a selected node");
            let parent = node.parent.filter(|_| node.instance.is_none());
            let parent = parent.map(|p| (p.map, p.node));
            let Some(parent) = parent.filter(|p| whole.contains(p) && instance.fade >= 1.0) else {
                continue;
            };
            held.beside += 1;
            let stated = forest.node(parent).map_or(0, |node| node.num_children);
            let found = named.get(&parent).copied().unwrap_or(0);
            held.states_more += usize::from(stated > found);
            held.states_fewer += usize::from(stated < found);
            if stated == found {
                let (map, entity) = (instance.map, instance.entity);
                let said = format!(
                    "entity {entity} of {map:#010X}, {:#010X}, beside a parent that states {stated}",
                    instance.archetype
                );
                held.unaccounted.push(said);
            }
        }
        held.instances += selected.instances.len();
    }
    held
}
