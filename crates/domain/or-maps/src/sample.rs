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

/// What the schema gains for a type file one of whose archetypes states the door extension: the member on the archetype, and the
/// extension's classes.
const EXTENSIONS_MEMBER: &str = r#"    <string name="assetName" type="atHashString"/>
    <array name="extensions" type="atArray"><pointer type="rage::fwExtensionDef" policy="owner"/></array>"#;
const DOOR_CLASSES: &str = r#"  <structdef type="rage::fwExtensionDef">
    <string name="name" type="atHashString"/>
  </structdef>
  <structdef type="CExtensionDef" base="rage::fwExtensionDef">
    <Vector3 name="offsetPosition"/>
  </structdef>
  <structdef type="CExtensionDefDoor" base="CExtensionDef">
    <bool name="enableLimitAngle"/>
    <bool name="startsLocked"/>
    <bool name="canBreak"/>
    <float name="limitAngle"/>
    <float name="doorTargetRatio"/>
    <string name="audioHash" type="atHashString"/>
  </structdef>
</ParserSchema>"#;

/// What the schema gains for a map file that carries container LODs: the member on the map, and the class of one.
const CONTAINER_LODS_MEMBER: &str = r#"    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
    <array name="containerLods" type="atArray"><struct type="rage::fwContainerLodDef"/></array>"#;
const CONTAINER_LOD_CLASS: &str = r#"  <structdef type="rage::fwContainerLodDef">
    <string name="name" type="atHashString"/>
    <u32 name="parentIndex"/>
  </structdef>
</ParserSchema>"#;

/// The schema of a sample file, with the door extension where `doors` asks for it.
fn schema(doors: bool) -> String {
    if !doors {
        return PSC.to_string();
    }
    PSC.replace(
        r#"    <string name="assetName" type="atHashString"/>"#,
        EXTENSIONS_MEMBER,
    )
    .replace("</ParserSchema>", DOOR_CLASSES)
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
    /// Whether `parent` is an index into the entities of the map this one names as its parent.
    pub in_parent_map: bool,
    pub children: u32,
}

impl<'a> Entity<'a> {
    pub const fn at(archetype: &'a str, position: [f32; 3], guid: u32) -> Self {
        Self {
            archetype,
            position,
            yaw: 0.0,
            scale: 1.0,
            guid,
            parent: -1,
            in_parent_map: false,
            children: 0,
        }
    }
}

/// The flags of a sample entity that names its LOD parent in this map.
const FLAGS: u64 = 32;
/// The bit of an entity's flags that says its `parentIndex` is an index into the parent map.
const LOD_IN_PARENT: u64 = 1 << 3;

fn entity(e: &Entity<'_>) -> MetaInstance {
    let mut inst = MetaInstance::new("CEntityDef");
    inst.set("archetypeName", Value::Hash(HashedString::new(e.archetype)));
    let in_parent = if e.in_parent_map { LOD_IN_PARENT } else { 0 };
    inst.set("flags", Value::UInt(FLAGS | in_parent));
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

fn encode(root: &MetaInstance, doors: bool) -> Vec<u8> {
    encode_with(root, &schema(doors))
}

fn encode_with(root: &MetaInstance, schema: &str) -> Vec<u8> {
    let library = build_library([("inline.psc", schema.as_bytes())]).expect("the schema parses");
    let layout =
        canonical::layout_for(&library, root, BinaryFormat::Rbm, None).expect("the file lays out");
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), root).expect("the file encodes")
}

/// A `.ymap` called `name` that places `entities`, whose extents are `extents` (the lowest and highest corner).
pub fn map_file(name: &str, entities: &[Entity<'_>], extents: ([f32; 3], [f32; 3])) -> Vec<u8> {
    map_under(name, HashedString::from_hash(0), entities, extents)
}

/// A `.ymap` called `name` whose parent map is `parent`, as [`map_file`] makes one.
pub fn child_map_file(
    name: &str,
    parent: &str,
    entities: &[Entity<'_>],
    extents: ([f32; 3], [f32; 3]),
) -> Vec<u8> {
    map_under(name, HashedString::new(parent), entities, extents)
}

/// A `.ymap` as [`map_file`] makes one, that carries a container LOD for each index of `container_lods`, the entity it names.
pub fn map_file_with_container_lods(
    name: &str,
    entities: &[Entity<'_>],
    container_lods: &[u32],
    extents: ([f32; 3], [f32; 3]),
) -> Vec<u8> {
    let mut root = map_root(name, HashedString::from_hash(0), entities, extents);
    let lods = (container_lods.iter().enumerate())
        .map(|(at, parent)| {
            let mut lod = MetaInstance::new("rage::fwContainerLodDef");
            lod.set(
                "name",
                Value::Hash(HashedString::new(format!("{name}_lod{at}"))),
            );
            lod.set("parentIndex", Value::UInt(u64::from(*parent)));
            Value::Struct(Box::new(lod))
        })
        .collect();
    root.set("containerLods", Value::Array(lods));
    let schema = PSC
        .replace(
            r#"    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>"#,
            CONTAINER_LODS_MEMBER,
        )
        .replace("</ParserSchema>", CONTAINER_LOD_CLASS);
    encode_with(&root, &schema)
}

fn map_under(
    name: &str,
    parent: HashedString,
    entities: &[Entity<'_>],
    extents: ([f32; 3], [f32; 3]),
) -> Vec<u8> {
    encode(&map_root(name, parent, entities, extents), false)
}

fn map_root(
    name: &str,
    parent: HashedString,
    entities: &[Entity<'_>],
    extents: ([f32; 3], [f32; 3]),
) -> MetaInstance {
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set("parent", Value::Hash(parent));
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
    root
}

/// One archetype of a sample type file.
#[derive(Debug, Clone, Copy)]
pub struct Archetype<'a> {
    pub name: &'a str,
    pub asset: &'a str,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Whether it states the door extension.
    pub door: bool,
}

impl<'a> Archetype<'a> {
    pub fn new(name: &'a str, min: [f32; 3], max: [f32; 3]) -> Self {
        Self {
            name,
            asset: name,
            min,
            max,
            door: false,
        }
    }
}

fn door_extension() -> MetaInstance {
    let mut inst = MetaInstance::new("CExtensionDefDoor");
    inst.set("name", Value::Hash(HashedString::from_hash(0)));
    inst.set("offsetPosition", Value::Vec3([0.0; 3]));
    for flag in ["enableLimitAngle", "startsLocked", "canBreak"] {
        inst.set(flag, Value::Bool(false));
    }
    inst.set("limitAngle", Value::Float(0.0));
    inst.set("doorTargetRatio", Value::Float(0.0));
    inst.set("audioHash", Value::Hash(HashedString::from_hash(0)));
    inst
}

fn archetype(a: &Archetype<'_>, extensions: bool) -> MetaInstance {
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
    if extensions {
        let stated = a.door.then(door_extension);
        inst.set(
            "extensions",
            Value::Array(
                stated
                    .into_iter()
                    .map(|e| Value::Pointer(Some(Box::new(e))))
                    .collect(),
            ),
        );
    }
    inst
}

/// A `.ytyp` called `name` that declares `archetypes`.
pub fn types_file(name: &str, archetypes: &[Archetype<'_>]) -> Vec<u8> {
    // ⚠ A file with no door keeps the schema it always had, so its bytes do not move for the suites that weigh them.
    let doors = archetypes.iter().any(|a| a.door);
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set(
        "archetypes",
        Value::Array(
            archetypes
                .iter()
                .map(|a| Value::Pointer(Some(Box::new(archetype(a, doors)))))
                .collect(),
        ),
    );
    root.set("dependencies", Value::Array(Vec::new()));
    encode(&root, doors)
}

/// What the schema gains for a type file that declares an interior: its class and the four records it holds, each with the members
/// the typed world family reads of it and no other.
const INTERIOR_CLASSES: &str = r#"  <structdef type="CMloRoomDef">
    <string name="name" type="atString"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <float name="blend"/>
    <string name="timecycleName" type="atHashString"/>
    <string name="secondaryTimecycleName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="portalCount"/>
    <s32 name="floorId"/>
    <s32 name="exteriorVisibiltyDepth" init="-1"/>
    <array name="attachedObjects" type="atArray"><u32/></array>
  </structdef>
  <structdef type="CMloPortalDef">
    <u32 name="roomFrom"/>
    <u32 name="roomTo"/>
    <u32 name="flags"/>
    <u32 name="mirrorPriority"/>
    <u32 name="opacity"/>
    <u32 name="audioOcclusion"/>
    <array name="corners" type="atArray"><Vector3/></array>
    <array name="attachedObjects" type="atArray"><u32/></array>
  </structdef>
  <structdef type="CMloEntitySet">
    <string name="name" type="atHashString"/>
    <array name="locations" type="atArray"><u32/></array>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CMloTimeCycleModifier">
    <string name="name" type="atHashString"/>
    <Vector4 name="sphere"/>
    <float name="percentage"/>
    <float name="range"/>
    <u32 name="startHour"/>
    <u32 name="endHour"/>
  </structdef>
  <structdef type="CMloArchetypeDef" base="CBaseArchetypeDef">
    <u32 name="mloFlags"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
    <array name="rooms" type="atArray"><struct type="CMloRoomDef"/></array>
    <array name="portals" type="atArray"><struct type="CMloPortalDef"/></array>
    <array name="entitySets" type="atArray"><struct type="CMloEntitySet"/></array>
    <array name="timeCycleModifiers" type="atArray"><struct type="CMloTimeCycleModifier"/></array>
  </structdef>
</ParserSchema>"#;

/// One room of a sample interior, with the entities of the interior it attaches by their index.
#[derive(Debug, Clone, Copy)]
pub struct Room<'a> {
    pub name: &'a str,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub attached: &'a [u32],
    /// The portals the room states it counts: the ones that name it, as a room of the game counts them, when none is stated.
    pub portal_count: Option<u32>,
}

/// One portal of a sample interior: the two rooms it joins by their index, its four corners, and the entities it attaches.
#[derive(Debug, Clone, Copy)]
pub struct Portal<'a> {
    pub from: u32,
    pub to: u32,
    pub corners: [[f32; 3]; 4],
    pub flags: u32,
    pub attached: &'a [u32],
}

/// One entity set of a sample interior: each entity with the room it stands in.
#[derive(Debug, Clone, Copy)]
pub struct EntitySet<'a> {
    pub name: &'a str,
    pub entities: &'a [(u32, Entity<'a>)],
}

/// One time cycle modifier of a sample interior, applied inside `sphere` (a centre and a radius).
#[derive(Debug, Clone, Copy)]
pub struct Modifier<'a> {
    pub name: &'a str,
    pub sphere: [f32; 4],
}

/// One interior of a sample type file.
#[derive(Debug, Clone, Copy, Default)]
pub struct Interior<'a> {
    pub name: &'a str,
    pub entities: &'a [Entity<'a>],
    pub rooms: &'a [Room<'a>],
    pub portals: &'a [Portal<'a>],
    pub entity_sets: &'a [EntitySet<'a>],
    pub modifiers: &'a [Modifier<'a>],
}

/// The quad of a sample portal: a door two metres wide and a little over two high, in the wall at `y`, between `x` and `x + 2`, from the floor `z`.
const fn door(x: f32, y: f32, z: f32) -> [[f32; 3]; 4] {
    [
        [x, y, z],
        [x + 2.0, y, z],
        [x + 2.0, y, z + 2.25],
        [x, y, z + 2.25],
    ]
}

/// A sound interior for the suites: the outside and three rooms in a row, the hall opening to the outside and each room to the next, six entities each attached once (one of them to the front door's portal and one to the outside), two entity sets and two time cycle modifiers.
pub const VILLA: Interior<'static> = Interior {
    name: "villa",
    entities: &[
        Entity::at("villa_shell", [0.0, 0.0, 0.0], 1),
        Entity::at("prop_chair", [-2.0, 1.0, 0.0], 2),
        Entity::at("prop_table", [8.0, 1.0, 0.0], 3),
        Entity::at("prop_lamp", [8.0, -2.0, -3.0], 4),
        Entity::at("villa_front_door", [-1.0, -5.0, 0.0], 5),
        Entity::at("villa_porch", [0.0, -7.0, 0.0], 6),
    ],
    rooms: &[
        Room {
            name: "limbo",
            min: [-5.0, -8.0, -3.0],
            max: [12.0, 5.0, 3.0],
            attached: &[5],
            portal_count: None,
        },
        Room {
            name: "hall",
            min: [-5.0, -5.0, 0.0],
            max: [5.0, 5.0, 3.0],
            attached: &[0, 1],
            portal_count: None,
        },
        Room {
            name: "kitchen",
            min: [5.0, -5.0, 0.0],
            max: [12.0, 5.0, 3.0],
            attached: &[2],
            portal_count: None,
        },
        Room {
            name: "cellar",
            min: [5.0, -5.0, -3.0],
            max: [12.0, 5.0, 0.0],
            attached: &[3],
            portal_count: None,
        },
    ],
    portals: &[
        Portal {
            from: 1,
            to: 0,
            corners: door(-1.0, -5.0, 0.0),
            flags: 0,
            attached: &[4],
        },
        Portal {
            from: 1,
            to: 2,
            corners: door(5.0, 0.0, 0.0),
            flags: 0,
            attached: &[],
        },
        Portal {
            from: 2,
            to: 3,
            corners: door(8.0, 3.0, 0.0),
            flags: 0,
            attached: &[],
        },
    ],
    entity_sets: &[
        EntitySet {
            name: "villa_party",
            entities: &[
                (2, Entity::at("prop_balloon", [9.0, 2.0, 1.0], 20)),
                (3, Entity::at("prop_crate", [9.0, -2.0, -3.0], 21)),
            ],
        },
        EntitySet {
            name: "villa_night",
            entities: &[(1, Entity::at("prop_candle", [-2.0, 1.0, 1.0], 22))],
        },
    ],
    modifiers: &[
        Modifier {
            name: "villa_warm",
            sphere: [0.0, 0.0, 1.5, 6.0],
        },
        Modifier {
            name: "villa_cold",
            sphere: [8.0, 0.0, -1.5, 4.0],
        },
    ],
};

fn indexes(of: &[u32]) -> Value {
    Value::Array(of.iter().map(|i| Value::UInt(u64::from(*i))).collect())
}

fn structs(of: impl Iterator<Item = MetaInstance>) -> Value {
    Value::Array(of.map(|inst| Value::Struct(Box::new(inst))).collect())
}

fn pointers(of: impl Iterator<Item = MetaInstance>) -> Value {
    Value::Array(
        of.map(|inst| Value::Pointer(Some(Box::new(inst))))
            .collect(),
    )
}

fn interior(i: &Interior<'_>) -> MetaInstance {
    // The box of an interior holds its rooms; one with no room states a box of nothing.
    let corners = |pick: fn(f32, f32) -> f32, of: fn(&Room<'_>) -> [f32; 3]| {
        let rooms = i.rooms.iter().map(of);
        rooms
            .reduce(|held, room| [0, 1, 2].map(|axis| pick(held[axis], room[axis])))
            .unwrap_or([0.0; 3])
    };
    let (min, max) = (corners(f32::min, |r| r.min), corners(f32::max, |r| r.max));
    let mut inst = archetype(&Archetype::new(i.name, min, max), false);
    inst.type_name = "CMloArchetypeDef".to_string();
    inst.set("mloFlags", Value::UInt(0));
    inst.set("entities", pointers(i.entities.iter().map(entity)));
    let rooms = i.rooms.iter().enumerate().map(|(at, room)| {
        let named = (i.portals.iter())
            .filter(|p| p.from as usize == at || p.to as usize == at)
            .count();
        let portals = room.portal_count.map_or(named as u64, u64::from);
        let mut inst = MetaInstance::new("CMloRoomDef");
        inst.set("name", Value::String(room.name.to_string()));
        inst.set("bbMin", Value::Vec3(room.min));
        inst.set("bbMax", Value::Vec3(room.max));
        inst.set("blend", Value::Float(1.0));
        inst.set("timecycleName", Value::Hash(HashedString::from_hash(0)));
        inst.set(
            "secondaryTimecycleName",
            Value::Hash(HashedString::from_hash(0)),
        );
        inst.set("flags", Value::UInt(0));
        inst.set("portalCount", Value::UInt(portals));
        inst.set("floorId", Value::Int(0));
        inst.set("exteriorVisibiltyDepth", Value::Int(-1));
        inst.set("attachedObjects", indexes(room.attached));
        inst
    });
    inst.set("rooms", structs(rooms));
    let portals = i.portals.iter().map(|portal| {
        let mut inst = MetaInstance::new("CMloPortalDef");
        inst.set("roomFrom", Value::UInt(u64::from(portal.from)));
        inst.set("roomTo", Value::UInt(u64::from(portal.to)));
        inst.set("flags", Value::UInt(u64::from(portal.flags)));
        inst.set("mirrorPriority", Value::UInt(0));
        inst.set("opacity", Value::UInt(0));
        inst.set("audioOcclusion", Value::UInt(0));
        inst.set(
            "corners",
            Value::Array(portal.corners.iter().map(|c| Value::Vec3(*c)).collect()),
        );
        inst.set("attachedObjects", indexes(portal.attached));
        inst
    });
    inst.set("portals", structs(portals));
    let sets = i.entity_sets.iter().map(|set| {
        let locations: Vec<u32> = set.entities.iter().map(|(room, _)| *room).collect();
        let mut inst = MetaInstance::new("CMloEntitySet");
        inst.set("name", Value::Hash(HashedString::new(set.name)));
        inst.set("locations", indexes(&locations));
        inst.set(
            "entities",
            pointers(set.entities.iter().map(|(_, e)| entity(e))),
        );
        inst
    });
    inst.set("entitySets", structs(sets));
    let modifiers = i.modifiers.iter().map(|modifier| {
        let mut inst = MetaInstance::new("CMloTimeCycleModifier");
        inst.set("name", Value::Hash(HashedString::new(modifier.name)));
        inst.set("sphere", Value::Vec4(modifier.sphere));
        inst.set("percentage", Value::Float(100.0));
        inst.set("range", Value::Float(1.0));
        inst.set("startHour", Value::UInt(0));
        inst.set("endHour", Value::UInt(23));
        inst
    });
    inst.set("timeCycleModifiers", structs(modifiers));
    inst
}

/// A `.ytyp` called `name` that declares `archetypes`, then `interiors`, each with its rooms, portals, entity sets and modifiers.
pub fn types_file_with_interiors(
    name: &str,
    archetypes: &[Archetype<'_>],
    interiors: &[Interior<'_>],
) -> Vec<u8> {
    let plain = archetypes.iter().map(|a| archetype(a, false));
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set(
        "archetypes",
        pointers(plain.chain(interiors.iter().map(interior))),
    );
    root.set("dependencies", Value::Array(Vec::new()));
    encode_with(&root, &PSC.replace("</ParserSchema>", INTERIOR_CLASSES))
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
