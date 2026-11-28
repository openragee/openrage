//! A GTA V install built by hand as the conformance kit's scene states one: a parent and its two
//! children in two maps, a map by the clock, a map only a request lets in, a map that places an
//! interior over a level floor, a drawable for each archetype that draws, the texture
//! dictionaries its surfaces ask through, and two maps that state far lights and occluders.

#![allow(dead_code)]

use glam::Vec3;
use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexLayout};
use or_io::decl::{File, Gen8, Hash32, Shared};
use or_io::gta5::drawable::gen8;
use or_io::gta5::gen8::{EntryData, GrcInstanceDataEntry, GrcTexture};
use or_io::gta5::shaders::shader_defs::hash_name;
use or_io::gta5::{
    FragDrawable, GrmGeometry, GrmModel, GrmShaderGroup, GtaDrawable, GtaFragType, PgDictionary,
    RmcDrawable, RmcLod, SpdAABB, TextureSlot,
};
use or_io::meta::canonical;
use or_io::meta::instance::HashedString;
use or_io::meta::{build_library, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_world::contract::WorldSpec;
use or_world::kit::{Fixture, Grounded, HeldBack, InteriorAt, Ladder, Lit, Scene, Timed};
use or_world::model::SwitchId;
use or_world_gta5::state::{flags as group_flags, NOT_BY_DEFAULT};
use or_world_gta5::{Gta5Title, LEGACY};

use crate::forest_fixture::{
    arch, ent, install, manifest_bytes, map_around, map_file, types_file, Arch, Ent, Files, MapOf,
    LOD,
};
use crate::world_fixture::Span;
use crate::world_fixture::{cube, hash, level_square, mesh_bound_bytes};

/// Where the parent of the ladder draws alone: past its children's 100 m.
pub const FAR_EYE: Vec3 = Vec3::new(240.0, 0.0, 0.0);
/// Where its two children draw in its place.
pub const NEAR_EYE: Vec3 = Vec3::new(70.0, 0.0, 0.0);
pub const NIGHT_EYE: Vec3 = Vec3::new(5000.0, 0.0, 0.0);
pub const EVENT_EYE: Vec3 = Vec3::new(0.0, 5000.0, 0.0);
pub const YARD_EYE: Vec3 = Vec3::new(-5000.0, 10.0, 5.0);
pub const NOWHERE: Vec3 = Vec3::new(-50_000.0, -50_000.0, 0.0);
/// The height of the floor under the yard.
pub const FLOOR: f32 = 3.0;
/// The hours the night map streams in: 20 h to 5 h.
pub const NIGHT_HOURS: u32 = 0b1111 << 20 | 0b11_1111;
/// The archetypes that draw, each a drawable of its own name.
pub const DRAWN: [&str; 7] = [
    "lod_p",
    "hd_a",
    "hd_b",
    "chair",
    "table",
    "night_prop",
    "event_prop",
];

/// How far each level of a drawable of [`drawable_bytes`] draws, in metres.
pub const LOD_DISTANCES: [f32; 4] = [20.0, 60.0, 180.0, 540.0];

/// The weather every check is made under.
pub fn weather() -> u64 {
    u64::from(hash("CLEAR"))
}

/// The texture dictionary every archetype of the scene names, which holds [`MOSS`] alone.
pub const OWN_TXD: &str = "scene_txd";
/// The dictionary [`OWN_TXD`] falls through to, of which the scene holds no file.
pub const MID_TXD: &str = "scene_mid";
/// The dictionary [`MID_TXD`] falls through to, which holds [`WALL`].
pub const ROOT_TXD: &str = "scene_root";
/// The texture every drawable of the scene binds as its `DiffuseSampler`, by name.
pub const WALL: &str = "wall";
pub const MOSS: &str = "moss";
/// The texture the drawable named `name` carries in its own dictionary and binds as its `BumpSampler`: one name a drawable, since a dictionary of drawables holds no two textures of one name.
pub fn badge(name: &str) -> String {
    format!("{name}_badge")
}
/// The bytes of a texture's two levels: 8 by 8 pixels in four blocks of eight bytes, then 4 by 4 in one.
pub const LEVELS: [usize; 2] = [32, 8];

/// A Legacy texture of 8 by 8 pixels in DXT1 with two levels, each byte of its store the low byte of its place plus `seed`.
pub fn texture(name: &str, seed: u8) -> GrcTexture {
    let mut texture = GrcTexture {
        name: name.into(),
        width: 8,
        height: 8,
        depth: 1,
        mip_stride: 16,
        format: u32::from_le_bytes(*b"DXT1"),
        mip_count: LEVELS.len() as u8,
        // The store's size is stated in units of 128 bytes.
        backing_store: (0..128usize)
            .map(|at| (at as u8).wrapping_add(seed))
            .collect(),
        ..Default::default()
    };
    texture.set_template_type(0);
    texture
}

/// A Legacy texture dictionary of `textures`, each `(name, seed)`.
pub fn textures_of(textures: &[(&str, u8)]) -> PgDictionary<GrcTexture> {
    let mut dictionary = PgDictionary::<GrcTexture>::default();
    for (name, seed) in textures {
        dictionary.insert(texture(name, *seed));
    }
    dictionary
}

/// A Legacy texture dictionary as the bytes of a file of its own.
pub fn textures_bytes(textures: &[(&str, u8)]) -> Vec<u8> {
    textures_of(textures)
        .to_bytes()
        .expect("a texture dictionary")
}

/// A relationship file that says each `(child, parent)` falls through so.
pub fn relationships_bytes(pairs: &[(&str, &str)]) -> Vec<u8> {
    let items = pairs.iter().map(|(child, parent)| {
        format!("<Item><parent>{parent}</parent><child>{child}</child></Item>")
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CMapParentTxds><txdRelationships>{}</txdRelationships></CMapParentTxds>",
        items.collect::<String>()
    )
    .into_bytes()
}

/// A Legacy drawable of one triangle a level, at `levels` levels of detail, as the bytes of a file of its own.
pub fn drawable_bytes(name: &str, levels: usize) -> Vec<u8> {
    drawable(name, levels).to_bytes().expect("a drawable")
}

/// The drawables `named`, each `(name, levels)`, as the bytes of a Legacy drawable dictionary.
pub fn dictionary_bytes(named: &[(&str, usize)]) -> Vec<u8> {
    let mut held: Vec<(u32, GtaDrawable<Gen8>)> = (named.iter())
        .map(|(name, levels)| (hash(name), drawable(name, *levels)))
        .collect();
    held.sort_by_key(|(code, _)| *code);
    let dictionary = PgDictionary::<GtaDrawable<Gen8>> {
        codes: held.iter().map(|(code, _)| *code).collect(),
        entries: (held.into_iter())
            .map(|(_, drawable)| Some(Box::new(drawable)))
            .collect(),
        ..Default::default()
    };
    dictionary.to_bytes().expect("a drawable dictionary")
}

/// A Legacy fragment whose drawable is [`drawable`] of `name` and `levels`, as the bytes of a file of its own.
pub fn fragment_bytes(name: &str, levels: usize) -> Vec<u8> {
    let common = FragDrawable {
        base: drawable(name, levels).base,
        ..Default::default()
    };
    let mut fragment = GtaFragType::<Gen8>::default();
    // Through the engine's fragment the game's own derefs to.
    *fragment = or_io::gta5::FragType {
        common_drawable: Some(Box::new(common)),
        ..Default::default()
    };
    fragment.to_bytes().expect("a fragment")
}

/// `bytes`, a Legacy file whose leaf is `leaf`, as the Enhanced game lays it out.
pub fn enhanced(leaf: &str, bytes: &[u8]) -> Vec<u8> {
    use or_io::gta5::shaders::shader_defs::{
        ParamRemap, ShaderDef, ShaderDefinitions, ShaderParamDef, ShaderParamKind,
    };
    // The two spellings of each texture parameter of the scene's shader: the sampler's name, which a Legacy file hashes, and the program's, which an Enhanced one does.
    let texture = |name: &str, spelled: &str| ShaderParamDef {
        name: name.into(),
        name_hash: hash_name(name),
        kind: ShaderParamKind::Texture,
        array_len: 1,
        register: 0,
        cbuffer: None,
        drawable_place: None,
        semantic: Some(spelled.into()),
        semantic_hash: Some(hash_name(spelled)),
        ui_name: None,
        default: None,
        preset_default: None,
    };
    let mut defs = ShaderDefinitions::new("gta5", "gen8");
    defs.shaders.push(ShaderDef {
        name: "default".into(),
        name_hash: hash_name("default"),
        effect: "default".into(),
        draw_bucket: None,
        vertex_type: 0,
        traits: Vec::new(),
        parameters: vec![
            texture("DiffuseSampler", "DiffuseTex"),
            texture("BumpSampler", "BumpTex"),
        ],
    });
    let remap = ParamRemap::of(&defs);
    let made = or_io::gta5::retarget::retarget_file(leaf, bytes, &remap).expect("it retargets");
    assert!(made.to.is_gen9());
    made.bytes
}

/// A Legacy drawable of one triangle a level, at `levels` levels of detail, whose one shader binds [`WALL`] by name and carries its [`badge`].
pub fn drawable(name: &str, levels: usize) -> GtaDrawable<Gen8> {
    let channels = [
        (GrcFvfChannel::Position, GrcDataSize::Float3),
        (GrcFvfChannel::Normal, GrcDataSize::Float3),
        (GrcFvfChannel::Tex0, GrcDataSize::Float2),
    ];
    let mut fvf = gen8::GrcFvf::default();
    for (channel, size) in channels {
        fvf.fvf |= 1 << channel as u32;
        fvf.channel_sizes |= (size as u64) << (channel as u64 * 4);
    }
    fvf.channel_count = channels.len() as u8;
    fvf.fvf_size = VertexLayout::of(fvf.fvf, fvf.channel_sizes).packed_stride() as u8;
    let mut data = Vec::new();
    for corner in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for number in corner
            .into_iter()
            .chain([0.0, 0.0, 1.0, corner[0], corner[1]])
        {
            data.extend_from_slice(&number.to_le_bytes());
        }
    }
    let mut geometry = GrmGeometry::<Gen8> {
        primitive_type: 3,
        vertex_buffer: Box::new(gen8::GrcVertexBuffer {
            data,
            fvf: Some(Box::new(fvf)),
            lock_ptr: Shared::Same,
            ..Default::default()
        }),
        index_buffer: Box::new(gen8::GrcIndexBuffer {
            indices: vec![0, 1, 2],
            ..Default::default()
        }),
        vertex_data: Shared::Same,
        ..Default::default()
    };
    geometry.take_counts_from_runs();
    let mut drawable = RmcDrawable::<Gen8> {
        name: name.into(),
        shader_group: Some(Box::new(group(name))),
        ..Default::default()
    };
    drawable.lod_group.lod_distances = LOD_DISTANCES;
    for lod in drawable.lod_group.lods.iter_mut().take(levels) {
        *lod = RmcLod {
            models: vec![Box::new(GrmModel {
                aabbs: vec![SpdAABB::default(); or_io::codec::mesh::aabb_slots(1)],
                shader_indices: vec![0],
                geometries: vec![Box::new(geometry.clone())],
                ..Default::default()
            })],
            ..Default::default()
        }
        .into();
    }
    GtaDrawable {
        base: drawable,
        ..Default::default()
    }
}

/// One shader of the cut-out bucket that binds [`WALL`] by name as its `DiffuseSampler` and carries the [`badge`] of `name` as its `BumpSampler`.
fn group(name: &str) -> GrmShaderGroup<Gen8> {
    let badge = badge(name);
    let binds = |parameter: &str, texture: &str| GrcInstanceDataEntry {
        register: 0,
        name_hash: Hash32::from_raw(hash_name(parameter)),
        data: EntryData::Texture(TextureSlot::Reference(Box::new(
            gen8::GrcTextureReference::named(texture),
        ))),
    };
    let mut group = GrmShaderGroup::<Gen8> {
        shaders: vec![Box::new(gen8::GrcInstanceData {
            basis_hash: Hash32::of("default"),
            draw_bucket: CUTOUT_BUCKET,
            // The bump before the diffuse, so a surface that took the first binding would take the wrong one.
            entries: vec![binds("BumpSampler", &badge), binds("DiffuseSampler", WALL)],
            ..Default::default()
        })],
        ..Default::default()
    };
    group.embed(texture(&badge, BADGE_SEED));
    group
}

/// The draw bucket of the scene's shader: the one that draws where its alpha passes a test.
pub const CUTOUT_BUCKET: u8 = 3;
pub const BADGE_SEED: u8 = 7;
pub const WALL_SEED: u8 = 100;
pub const MOSS_SEED: u8 = 200;

/// The members of a map that state its far lights and its occluders, beside those a forest reads.
const RECORDS: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="BoxOccluder">
    <s16 name="iCenterX"/>
    <s16 name="iCenterY"/>
    <s16 name="iCenterZ"/>
    <s16 name="iCosZ"/>
    <s16 name="iLength"/>
    <s16 name="iWidth"/>
    <s16 name="iHeight"/>
    <s16 name="iSinZ"/>
  </structdef>
  <structdef type="OccludeModel">
    <Vector3 name="bmin"/>
    <Vector3 name="bmax"/>
    <u32 name="dataSize"/>
    <array name="verts" type="pointer" sizeVar="dataSize" align="16"><u8/></array>
    <u16 name="numVertsInBytes"/>
    <u16 name="numTris"/>
    <u32 name="flags"/>
  </structdef>
  <structdef type="FloatXYZ">
    <float name="x"/>
    <float name="y"/>
    <float name="z"/>
  </structdef>
  <structdef type="CLODLight">
    <array name="direction" type="atArray"><struct type="FloatXYZ"/></array>
    <array name="falloff" type="atArray"><float/></array>
    <array name="falloffExponent" type="atArray"><float/></array>
    <array name="timeAndStateFlags" type="atArray"><u32/></array>
    <array name="hash" type="atArray"><u32/></array>
    <array name="coneInnerAngle" type="atArray"><u8/></array>
    <array name="coneOuterAngleOrCapExt" type="atArray"><u8/></array>
    <array name="coronaIntensity" type="atArray"><u8/></array>
  </structdef>
  <structdef type="CDistantLODLight">
    <array name="position" type="atArray"><struct type="FloatXYZ"/></array>
    <array name="RGBI" type="atArray"><u32/></array>
    <u16 name="numStreetLights"/>
    <u16 name="category"/>
  </structdef>
  <structdef type="rage__fwGrassInstanceListDef">
    <float name="lodDist"/>
  </structdef>
  <structdef type="rage__fwPropInstanceListDef">
    <float name="lodDist"/>
  </structdef>
  <structdef type="rage__fwInstancedMapData">
    <array name="PropInstanceList" type="atArray"><struct type="rage__fwPropInstanceListDef"/></array>
    <array name="GrassInstanceList" type="atArray"><struct type="rage__fwGrassInstanceListDef"/></array>
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
    <array name="boxOccluders" type="atArray"><struct type="BoxOccluder"/></array>
    <array name="occludeModels" type="atArray"><struct type="OccludeModel"/></array>
    <struct name="LODLightsSOA" type="CLODLight"/>
    <struct name="DistantLODLightsSOA" type="CDistantLODLight"/>
    <struct name="instancedData" type="rage__fwInstancedMapData"/>
  </structdef>
</ParserSchema>"#;

/// One far light as a map of far lights and its parent state it between them.
#[derive(Clone, Copy)]
pub struct Far {
    pub at: [f32; 3],
    pub direction: [f32; 3],
    pub falloff: f32,
    /// `timeAndStateFlags`: the hours in the low 24 bits and the type in bits 26 to 28.
    pub flags: u32,
    pub rgbi: u32,
    pub cone: (u8, u8),
}

/// One box occluder, every member a number as the file stores it: `[x, y, z, cos, length, width, height, sin]`.
pub type Boxed = [i16; 8];

/// One occlude model: its points, its triangles, and what its `numVertsInBytes` and `numTris` state where the file states them wrongly.
#[derive(Clone)]
pub struct Occluding {
    pub points: Vec<[f32; 3]>,
    pub triangles: Vec<[u8; 3]>,
    /// What `numTris` states in place of the count of `triangles`, for a model a file states badly.
    pub stated_triangles: Option<u16>,
}

/// What a map states beside entities.
#[derive(Clone, Default)]
pub struct Records {
    pub parent: &'static str,
    /// What the far lights of this map state but where they stand and their colour.
    pub lights: Vec<Far>,
    /// Where the far lights of the map that names this one as its parent stand, and their colours.
    pub stands: Vec<Far>,
    pub boxes: Vec<Boxed>,
    pub models: Vec<Occluding>,
    /// How many grass lists and how many prop lists it states, each with no instance.
    pub lists: (usize, usize),
}

fn held(items: Vec<MetaInstance>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|i| Value::Struct(Box::new(i)))
            .collect(),
    )
}

fn numbers<T: Copy + Into<u64>>(items: impl Iterator<Item = T>) -> Value {
    Value::Array(items.map(|i| Value::UInt(i.into())).collect())
}

fn points(of: impl Iterator<Item = [f32; 3]>) -> Value {
    let point = |[x, y, z]: [f32; 3]| {
        let mut inst = MetaInstance::new("FloatXYZ");
        inst.set("x", Value::Float(f64::from(x)));
        inst.set("y", Value::Float(f64::from(y)));
        inst.set("z", Value::Float(f64::from(z)));
        inst
    };
    held(of.map(point).collect())
}

/// A map named `name` with no entity that streams from 500 m about `extents` and states `records`.
pub fn records_bytes(name: &str, extents: Span, records: &Records) -> Vec<u8> {
    let named = |name: &str| Value::Hash(HashedString::new(name));
    let mut root = MetaInstance::new("CMapData");
    root.set("name", named(name));
    root.set("parent", named(records.parent));
    root.set("flags", Value::UInt(0));
    root.set("contentFlags", Value::UInt(1));
    root.set(
        "streamingExtentsMin",
        Value::Vec3(extents.0.map(|v| v - 500.0)),
    );
    root.set(
        "streamingExtentsMax",
        Value::Vec3(extents.1.map(|v| v + 500.0)),
    );
    root.set("entitiesExtentsMin", Value::Vec3(extents.0));
    root.set("entitiesExtentsMax", Value::Vec3(extents.1));
    let boxes = records.boxes.iter().map(|stated| {
        let members = [
            "iCenterX", "iCenterY", "iCenterZ", "iCosZ", "iLength", "iWidth", "iHeight", "iSinZ",
        ];
        let mut inst = MetaInstance::new("BoxOccluder");
        for (member, value) in members.into_iter().zip(stated) {
            inst.set(member, Value::Int(i64::from(*value)));
        }
        inst
    });
    root.set("boxOccluders", held(boxes.collect()));
    let models = records.models.iter().map(|model| {
        let mut verts: Vec<u8> = (model.points.iter().flatten())
            .flat_map(|number| number.to_le_bytes())
            .collect();
        let in_bytes = verts.len();
        verts.extend(model.triangles.iter().flatten());
        let triangles = (model.stated_triangles).unwrap_or(0x8000 | model.triangles.len() as u16);
        let mut inst = MetaInstance::new("OccludeModel");
        inst.set("bmin", Value::Vec3([0.0; 3]));
        inst.set("bmax", Value::Vec3([0.0; 3]));
        inst.set("dataSize", Value::UInt(verts.len() as u64));
        inst.set("verts", Value::ByteArray(verts));
        inst.set("numVertsInBytes", Value::UInt(in_bytes as u64));
        inst.set("numTris", Value::UInt(u64::from(triangles)));
        inst.set("flags", Value::UInt(0));
        inst
    });
    root.set("occludeModels", held(models.collect()));
    let lights = &records.lights;
    let mut far = MetaInstance::new("CLODLight");
    far.set("direction", points(lights.iter().map(|l| l.direction)));
    let floats =
        |of: Vec<f32>| Value::Array(of.into_iter().map(|f| Value::Float(f64::from(f))).collect());
    far.set(
        "falloff",
        floats(lights.iter().map(|l| l.falloff).collect()),
    );
    far.set(
        "falloffExponent",
        floats(lights.iter().map(|_| 1.0).collect()),
    );
    far.set("timeAndStateFlags", numbers(lights.iter().map(|l| l.flags)));
    far.set("hash", numbers(lights.iter().map(|_| 0u32)));
    far.set("coneInnerAngle", numbers(lights.iter().map(|l| l.cone.0)));
    far.set(
        "coneOuterAngleOrCapExt",
        numbers(lights.iter().map(|l| l.cone.1)),
    );
    far.set("coronaIntensity", numbers(lights.iter().map(|_| 0u8)));
    root.set("LODLightsSOA", Value::Struct(Box::new(far)));
    let mut stands = MetaInstance::new("CDistantLODLight");
    stands.set("position", points(records.stands.iter().map(|l| l.at)));
    stands.set("RGBI", numbers(records.stands.iter().map(|l| l.rgbi)));
    stands.set("numStreetLights", Value::UInt(0));
    stands.set("category", Value::UInt(0));
    root.set("DistantLODLightsSOA", Value::Struct(Box::new(stands)));
    let lists = |kind: &str, count: usize| {
        let list = |_| {
            let mut inst = MetaInstance::new(kind);
            inst.set("lodDist", Value::Float(60.0));
            inst
        };
        held((0..count).map(list).collect())
    };
    let mut instanced = MetaInstance::new("rage__fwInstancedMapData");
    instanced.set(
        "PropInstanceList",
        lists("rage__fwPropInstanceListDef", records.lists.1),
    );
    instanced.set(
        "GrassInstanceList",
        lists("rage__fwGrassInstanceListDef", records.lists.0),
    );
    root.set("instancedData", Value::Struct(Box::new(instanced)));
    let library = build_library([("records.psc", RECORDS.as_bytes())]).expect("the schema");
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).expect("lays out");
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).expect("encodes")
}

/// A point light lit every hour, a spot lit by night and a capsule, and a fourth of a type the contract has no light for.
pub fn far_lights() -> Vec<Far> {
    let light = |at, kind: u32, hours: u32, rgbi, cone| Far {
        at,
        direction: [0.0, 0.0, -2.0],
        falloff: 12.5,
        flags: kind << 26 | hours,
        rgbi,
        cone,
    };
    vec![
        light([-5001.0, 1.0, 9.0], 1, (1 << 24) - 1, 0xFF33_66CC, (0, 0)),
        light([-5002.0, 2.0, 9.0], 2, NIGHT_HOURS, 0x80FF_0000, (51, 102)),
        light([-5003.0, 3.0, 9.0], 4, 1, 0x4000_FF00, (0, 12)),
        light([-5004.0, 4.0, 9.0], 3, 1, 0, (0, 0)),
    ]
}

/// A box of 10 by 4 by 2 m unturned, and one of 8 by 2 by 6 m turned.
pub fn boxes() -> Vec<Boxed> {
    vec![
        [-20_000, 40, 12, 0, 40, 16, 8, 0],
        [-20_040, 80, 20, 3, 32, 8, 24, 4],
    ]
}

/// How many grass lists and prop lists the map of occluders states.
pub const LISTS: (usize, usize) = (2, 1);

/// A square of two triangles, a model whose count of triangles is not what its bytes hold, and one whose triangle names a point it does not hold.
pub fn occlude_models() -> Vec<Occluding> {
    let square = Occluding {
        points: vec![
            [-5000.0, 0.0, 4.0],
            [-4990.0, 0.0, 4.0],
            [-4990.0, 0.0, 9.0],
            [-5000.0, 0.0, 9.0],
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        stated_triangles: None,
    };
    let bad = Occluding {
        stated_triangles: Some(0x8000 | 5),
        ..square.clone()
    };
    let past = Occluding {
        triangles: vec![[0, 1, 4]],
        ..square.clone()
    };
    vec![square, bad, past]
}

/// A parent at the origin drawn to 1,000 m that states two children and 100 m for them.
pub fn parent() -> Ent {
    Ent {
        lod_dist: 1000.0,
        child_lod_dist: 100.0,
        level: LOD,
        num_children: 2,
        ..ent("lod_p", [0.0; 3])
    }
}

/// The two children, 300 m apart, each naming entity 0 of the parent map as its parent.
pub fn children() -> Vec<Ent> {
    let child = |archetype, x| Ent {
        lod_dist: 40.0,
        parent: Some((0, true)),
        ..ent(archetype, [x, 0.0, 0.0])
    };
    vec![child("hd_a", 150.0), child("hd_b", -150.0)]
}

/// A map of one entity of `archetype` at `at`, held back until something asks.
fn held_back(archetype: &'static str, at: [f32; 3]) -> MapOf {
    MapOf {
        flags: NOT_BY_DEFAULT,
        ..map_around(cube(at.map(|v| v - 10.0), 20.0), vec![ent(archetype, at)])
    }
}

/// The files of the one archive of the scene.
pub fn files() -> Files {
    let lod = map_around(cube([-200.0; 3], 400.0), vec![parent()]);
    let hd = MapOf {
        parent: "m_lod",
        ..map_around(cube([-200.0; 3], 400.0), children())
    };
    let shop = Ent {
        interior: true,
        lod_dist: 100.0,
        ..ent("shop", [-5000.0, 0.0, 5.0])
    };
    let yard = map_around(cube([-5010.0, -10.0, 0.0], 20.0), vec![shop]);
    let textured = |name: &'static str| Arch {
        files: (name, "", OWN_TXD),
        ..arch(name, 500.0)
    };
    let mut types: Vec<Arch> = DRAWN.iter().copied().map(textured).collect();
    types.push(Arch {
        interior: Some(vec![
            ent("chair", [1.0, 0.0, 0.0]),
            ent("table", [2.0, 0.0, 0.0]),
        ]),
        ..arch("shop", 100.0)
    });
    let mut files = vec![
        map_file("m_lod", &lod),
        map_file("m_hd", &hd),
        map_file("m_night", &held_back("night_prop", [5000.0, 0.0, 0.0])),
        map_file("m_event", &held_back("event_prop", [0.0, 5000.0, 0.0])),
        map_file("m_yard", &yard),
        types_file("scene", &types),
        (
            "levels/_manifest.ymf".to_owned(),
            manifest_bytes(&[("m_night", group_flags::TIMED, NIGHT_HOURS, &[])]),
        ),
        (
            "levels/m_yard_0.ybn".to_owned(),
            mesh_bound_bytes(level_square([-5050.0, -50.0, FLOOR], 100.0, 0x21)),
        ),
    ];
    for name in DRAWN {
        let levels = if name == "lod_p" { 1 } else { 2 };
        files.push((format!("models/{name}.ydr"), drawable_bytes(name, levels)));
    }
    let about_the_yard = cube([-5010.0, -10.0, 0.0], 20.0);
    let lights = Records {
        parent: "m_lights_far",
        lights: far_lights(),
        ..Records::default()
    };
    let stands = Records {
        stands: far_lights(),
        ..Records::default()
    };
    let hides = Records {
        boxes: boxes(),
        models: occlude_models(),
        lists: LISTS,
        ..Records::default()
    };
    for (name, records) in [
        ("m_lights", lights),
        ("m_lights_far", stands),
        ("m_hides", hides),
    ] {
        let bytes = records_bytes(name, about_the_yard, &records);
        files.push((format!("levels/{name}.ymap"), bytes));
    }
    files.extend([
        (
            format!("textures/{OWN_TXD}.ytd"),
            textures_bytes(&[(MOSS, MOSS_SEED)]),
        ),
        (
            format!("textures/{ROOT_TXD}.ytd"),
            textures_bytes(&[(WALL, WALL_SEED)]),
        ),
        (
            "data/gtxd.meta".to_owned(),
            relationships_bytes(&[(OWN_TXD, MID_TXD), (MID_TXD, ROOT_TXD)]),
        ),
    ]);
    files
}

/// The scene the kit checks the install of [`files`] by.
pub fn scene() -> Scene {
    Scene {
        weather: weather(),
        nowhere: NOWHERE,
        ladder: Ladder {
            parent_map: u64::from(hash("m_lod")),
            child_map: u64::from(hash("m_hd")),
            far: FAR_EYE,
            near: NEAR_EYE,
        },
        timed: Timed {
            map: u64::from(hash("m_night")),
            eye: NIGHT_EYE,
            on: 22.0,
            off: 12.0,
        },
        held: HeldBack {
            map: u64::from(hash("m_event")),
            eye: EVENT_EYE,
            switch: SwitchId(hash("m_event")),
        },
        interior: InteriorAt {
            eye: YARD_EYE,
            instances: 2,
        },
        // No map of GTA V lays out a list: nothing types what one instance of a list states.
        list: None,
        // Each of its far lights of a type the contract has, both boxes, and the occlude model whose bytes are whole.
        lit: Lit {
            eye: YARD_EYE,
            lights: 3,
            occluders: 3,
        },
        placed: (u64::from(hash("m_yard")), 1),
        ground: Grounded {
            at: Vec3::new(-5000.0, 0.0, FLOOR + 5.0),
            height: FLOOR,
        },
    }
}

/// The scene written to a scratch folder, which lives as long as the fixture.
pub fn fixture() -> Fixture {
    let dir = install(&[("x64a.rpf", files())]);
    Fixture {
        title: Box::new(Gta5Title),
        world: WorldSpec::of(LEGACY, dir.path()),
        scene: scene(),
        edit: None,
        keeps: Some(Box::new(dir)),
        executable: None,
    }
}
