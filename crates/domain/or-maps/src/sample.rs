//! Small map files, type files and manifests for the suites that need some, made the way a synthetic file is made anywhere in the tree: a schema
//! of the classes written inline, a value tree, and the layout the writer derives from the two.
//!
//! The classes are the engine's, with the members the typed world family reads (`or_io::meta::typed`), so a file made here opens as
//! `CMapData` or `CMapTypes`, saves, and cooks from a source. It is a test fixture and not a way to make a map: its RBM classes carry
//! a zero signature no shipped file has. `or_world::create` makes a map from nothing.

use or_io::meta::instance::HashedString;
use or_io::meta::{build_library, canonical, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_world::Placement;

const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <u32 name="numChildren"/>
    <s32 name="ambientOcclusionMultiplier"/>
    <s32 name="artificialAmbientOcclusion"/>
    <u32 name="tintValue"/>
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
    <u32 name="flags"/>
    <u32 name="specialAttribute"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <Vector3 name="bsCentre"/>
    <float name="bsRadius"/>
    <float name="hdTextureDist"/>
    <string name="name" type="atHashString"/>
    <string name="textureDictionary" type="atHashString"/>
    <string name="clipDictionary" type="atHashString"/>
    <string name="drawableDictionary" type="atHashString"/>
    <string name="physicsDictionary" type="atHashString"/>
    <string name="assetName" type="atHashString"/>
  </structdef>
  <structdef type="CMapTypes">
    <string name="name" type="atHashString"/>
    <array name="archetypes" type="atArray"><pointer type="CBaseArchetypeDef" policy="owner"/></array>
    <array name="dependencies" type="atArray"><string type="atHashString"/></array>
  </structdef>
</ParserSchema>"#;

/// The classes of a manifest, with the members `or-world` reads of one.
const MANIFEST_PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CImapDependencies">
    <string name="imapName" type="atHashString"/>
    <u32 name="manifestFlags"/>
    <array name="itypDepArray" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CPackFileMetaData">
    <array name="imapDependencies_2" type="atArray"><struct type="CImapDependencies"/></array>
  </structdef>
</ParserSchema>"#;

/// What a sample manifest states of one map: the flags of its entry and the type files it is said to need.
#[derive(Debug, Clone, Copy)]
pub struct Stated<'a> {
    pub map: &'a str,
    pub flags: u32,
    pub types: &'a [&'a str],
}

/// A `_manifest.ymf` that states `stated`, in the container the game ships its manifests in.
pub fn manifest_file(stated: &[Stated<'_>]) -> Vec<u8> {
    // ⚠ A member the public table of names lacks is spelled by its hash in a document, and the writer looks it up under that spelling.
    let flags = format!("hash_{:08X}", or_world::field::name_hash("manifestFlags"));
    let entries = stated
        .iter()
        .map(|said| {
            let mut entry = MetaInstance::new("CImapDependencies");
            entry.set("imapName", Value::Hash(HashedString::new(said.map)));
            entry.set(&flags, Value::UInt(u64::from(said.flags)));
            entry.set(
                "itypDepArray",
                Value::Array(
                    (said.types.iter())
                        .map(|name| Value::Hash(HashedString::new(*name)))
                        .collect(),
                ),
            );
            Value::Struct(Box::new(entry))
        })
        .collect();
    let mut root = MetaInstance::new("CPackFileMetaData");
    root.set("imapDependencies_2", Value::Array(entries));
    let library =
        build_library([("inline.psc", MANIFEST_PSC.as_bytes())]).expect("the schema parses");
    // ⚠ Written with the public table of names alone, whatever schema the process installed, so the spelling above is the one looked for.
    or_io::meta::binary::hash_index::with_public_names_only(|| {
        let layout = canonical::layout_for(&library, &root, BinaryFormat::Pso, None)
            .expect("the manifest lays out");
        write_meta_binary(BinaryFormat::Pso, Some(&layout), &root).expect("the manifest encodes")
    })
}

/// One entity of a sample map.
#[derive(Debug, Clone, Copy)]
pub struct Entity<'a> {
    pub archetype: &'a str,
    pub position: [f32; 3],
    /// The turn about the vertical axis in radians, as the pose of the entity says it.
    pub yaw: f32,
    pub scale: f32,
    pub guid: u32,
    pub parent: i32,
    pub children: u32,
}

impl<'a> Entity<'a> {
    pub fn at(archetype: &'a str, position: [f32; 3], guid: u32) -> Self {
        Self {
            archetype,
            position,
            yaw: 0.0,
            scale: 1.0,
            guid,
            parent: -1,
            children: 0,
        }
    }
}

fn entity(e: &Entity<'_>) -> MetaInstance {
    let mut inst = MetaInstance::new("CEntityDef");
    inst.set("archetypeName", Value::Hash(HashedString::new(e.archetype)));
    inst.set("flags", Value::UInt(32));
    inst.set("guid", Value::UInt(u64::from(e.guid)));
    inst.set("position", Value::Vec3(e.position));
    let stored = Placement::Entity.store(glam::Quat::from_rotation_z(e.yaw));
    inst.set("rotation", Value::Vec4(stored));
    inst.set("scaleXY", Value::Float(f64::from(e.scale)));
    inst.set("scaleZ", Value::Float(f64::from(e.scale)));
    inst.set("parentIndex", Value::Int(i64::from(e.parent)));
    inst.set("lodDist", Value::Float(100.0));
    inst.set("childLodDist", Value::Float(0.0));
    inst.set("numChildren", Value::UInt(u64::from(e.children)));
    inst.set("ambientOcclusionMultiplier", Value::Int(255));
    inst.set("artificialAmbientOcclusion", Value::Int(255));
    inst.set("tintValue", Value::UInt(0));
    inst
}

fn encode(root: &MetaInstance) -> Vec<u8> {
    let library = build_library([("inline.psc", PSC.as_bytes())]).expect("the schema parses");
    let layout =
        canonical::layout_for(&library, root, BinaryFormat::Rbm, None).expect("the file lays out");
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), root).expect("the file encodes")
}

/// A `.ymap` called `name` that places `entities`, whose extents are `extents` (the lowest and highest corner).
pub fn map_file(name: &str, entities: &[Entity<'_>], extents: ([f32; 3], [f32; 3])) -> Vec<u8> {
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set("parent", Value::Hash(HashedString::from_hash(0)));
    root.set("flags", Value::UInt(0));
    root.set("contentFlags", Value::UInt(65));
    root.set("streamingExtentsMin", Value::Vec3(extents.0));
    root.set("streamingExtentsMax", Value::Vec3(extents.1));
    root.set("entitiesExtentsMin", Value::Vec3(extents.0));
    root.set("entitiesExtentsMax", Value::Vec3(extents.1));
    root.set(
        "entities",
        Value::Array(
            entities
                .iter()
                .map(|e| Value::Pointer(Some(Box::new(entity(e)))))
                .collect(),
        ),
    );
    encode(&root)
}

/// One archetype of a sample type file.
#[derive(Debug, Clone, Copy)]
pub struct Archetype<'a> {
    pub name: &'a str,
    pub asset: &'a str,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl<'a> Archetype<'a> {
    pub fn new(name: &'a str, min: [f32; 3], max: [f32; 3]) -> Self {
        Self {
            name,
            asset: name,
            min,
            max,
        }
    }
}

fn archetype(a: &Archetype<'_>) -> MetaInstance {
    let mut inst = MetaInstance::new("CBaseArchetypeDef");
    inst.set("lodDist", Value::Float(100.0));
    inst.set("flags", Value::UInt(0));
    inst.set("specialAttribute", Value::UInt(0));
    inst.set("bbMin", Value::Vec3(a.min));
    inst.set("bbMax", Value::Vec3(a.max));
    let centre = [
        (a.min[0] + a.max[0]) / 2.0,
        (a.min[1] + a.max[1]) / 2.0,
        (a.min[2] + a.max[2]) / 2.0,
    ];
    inst.set("bsCentre", Value::Vec3(centre));
    inst.set("bsRadius", Value::Float(3.0));
    inst.set("hdTextureDist", Value::Float(5.0));
    inst.set("name", Value::Hash(HashedString::new(a.name)));
    for empty in [
        "textureDictionary",
        "clipDictionary",
        "drawableDictionary",
        "physicsDictionary",
    ] {
        inst.set(empty, Value::Hash(HashedString::from_hash(0)));
    }
    inst.set("assetName", Value::Hash(HashedString::new(a.asset)));
    inst
}

/// A `.ytyp` called `name` that declares `archetypes`.
pub fn types_file(name: &str, archetypes: &[Archetype<'_>]) -> Vec<u8> {
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set(
        "archetypes",
        Value::Array(
            archetypes
                .iter()
                .map(|a| Value::Pointer(Some(Box::new(archetype(a)))))
                .collect(),
        ),
    );
    root.set("dependencies", Value::Array(Vec::new()));
    encode(&root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::decl::File;
    use or_io::meta::typed::{CMapData, CMapTypes};

    #[test]
    fn a_sample_map_and_a_sample_type_file_open_as_the_typed_roots_and_say_what_they_were_made_of()
    {
        let map = map_file(
            "harbour",
            &[
                Entity::at("prop_crate_a", [1.0, 2.0, 3.0], 7),
                Entity {
                    yaw: 1.0,
                    scale: 2.0,
                    ..Entity::at("prop_crate_b", [4.0, 5.0, 6.0], 8)
                },
            ],
            ([0.0; 3], [10.0; 3]),
        );
        let typed = CMapData::from_bytes(&map).expect("opens");
        assert_eq!(typed.entities.len(), 2);
        assert_eq!(typed.entities[1].guid, 8);
        assert_eq!(typed.entities[1].scale_xy, 2.0);
        assert!(typed.entities[0].archetype_name == "prop_crate_a");

        let types = types_file(
            "harbour_types",
            &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
        );
        let typed = CMapTypes::from_bytes(&types).expect("opens");
        assert_eq!(typed.archetypes.len(), 1);
        assert!(typed.archetypes[0].name == "prop_crate_a");
        assert_eq!(typed.archetypes[0].bb_max.x, 1.0);
    }
}
