//! GTA VI's map placement (`CMapData`) and archetypes (`CMapTypes`), read typed from a headerless
//! RBM body's class table and data blocks; [`grow`] adds and removes entities.
//!
//! ⚠ A member a reader uses is checked against the body's own class table first: a class whose
//! member sits at another offset or type would read the wrong bytes, so the body is refused.

use crate::hash::literal_string_hash as lit;
use crate::meta::binary::rbm::schema::{RbmDataBlock, RbmModel};

pub const C_MAP_DATA: u32 = lit(b"CMapData");
pub const C_MAP_TYPES: u32 = lit(b"CMapTypes");
const C_ENTITY_DEF: u32 = lit(b"CEntityDef");
const C_MLO_INSTANCE_DEF: u32 = lit(b"CMloInstanceDef");
const C_BASE_ARCHETYPE_DEF: u32 = lit(b"CBaseArchetypeDef");
const C_TIME_ARCHETYPE_DEF: u32 = lit(b"CTimeArchetypeDef");
const C_MLO_ARCHETYPE_DEF: u32 = lit(b"CMloArchetypeDef");
const INSTANCED_MAP_DATA: u32 = lit(b"rage__fwInstancedMapData");
const PROP_LIST: u32 = lit(b"rage__fwPropInstanceListDef");
const GRASS_LIST: u32 = lit(b"rage__fwGrassInstanceListDef");
const PROP_INSTANCE: u32 = lit(b"rage__fwPropInstanceListDef__InstanceData");
const GRASS_INSTANCE: u32 = lit(b"rage__fwGrassInstanceListDef__InstanceData");
const AABB: u32 = lit(b"rage__spdAABB");

const STRING_HASH: u8 = 0x4A;
const HASH64: u8 = 0x4F;
const WORD64: u8 = 0x08;
const STRUCT: u8 = 0x05;
const ARRAY: u8 = 0x52;
const U32: u8 = 0x15;
const S16: u8 = 0x12;
const FLOAT: u8 = 0x21;
const VEC3: u8 = 0x32;
const VEC3V: u8 = 0x33;
const VEC4V: u8 = 0x34;
const ENUM8: u8 = 0x60;
const ENUM32: u8 = 0x62;
const ARRAY_MEMBER: u8 = 0x50;

/// A member this reader reads: its name's hash, its offset and its type code.
type Member = (u32, u32, u8);

const MAP_DATA: &[Member] = &[
    (lit(b"name"), 0x08, STRING_HASH),
    (0x8BFB_6095, 0x10, HASH64),
    (0x6C0F_9C8C, 0x50, STRING_HASH),
    (lit(b"parent"), 0x68, STRING_HASH),
    (0x9E8D_32A0, 0x6C, STRING_HASH),
    (0xEC16_451D, 0x70, STRING_HASH),
    (0x36C8_98AA, 0x74, STRING_HASH),
    (lit(b"flags"), 0x80, U32),
    (lit(b"contentFlags"), 0x84, U32),
    (lit(b"streamingExtentsMin"), 0x90, VEC3V),
    (lit(b"streamingExtentsMax"), 0xA0, VEC3V),
    (lit(b"entitiesExtentsMin"), 0xB0, VEC3V),
    (lit(b"entitiesExtentsMax"), 0xC0, VEC3V),
    (lit(b"entities"), 0xD0, ARRAY),
    (lit(b"instancedData"), 0x140, STRUCT),
];
const ENTITY: &[Member] = &[
    (lit(b"id"), 0x08, WORD64),
    (lit(b"rotation"), 0x10, VEC4V),
    (0xC8A7_B90A, 0x20, HASH64),
    (lit(b"position"), 0x38, VEC3),
    (lit(b"archetypeName"), 0x44, STRING_HASH),
    (lit(b"flags"), 0x48, U32),
    (lit(b"scaleXY"), 0x54, FLOAT),
    (lit(b"scaleZ"), 0x58, FLOAT),
    (lit(b"parentIndex"), 0x5C, S16),
    (0xE74C_3C23, 0x5E, S16),
    (lit(b"lodDist"), 0x60, FLOAT),
    (lit(b"childLodDist"), 0x64, FLOAT),
    (lit(b"numChildren"), 0x68, U32),
    (lit(b"tintValue"), 0x70, U32),
    (lit(b"priorityLevel"), 0x7C, ENUM8),
    (lit(b"lodLevel"), 0x7E, ENUM8),
];
const INSTANCED: &[Member] = &[
    (lit(b"PropInstanceList"), 0x08, ARRAY),
    (lit(b"GrassInstanceList"), 0x18, ARRAY),
];
const PROPS: &[Member] = &[
    (0xC8A7_B90A, 0x20, HASH64),
    (lit(b"archetypeName"), 0x44, STRING_HASH),
    (lit(b"lodDist"), 0x60, FLOAT),
    (lit(b"childLodDist"), 0x64, FLOAT),
    (lit(b"InstanceList"), 0x90, ARRAY),
    (lit(b"BatchAABB"), 0xA0, STRUCT),
];
const GRASS: &[Member] = &[
    (lit(b"archetypeName"), 0x44, STRING_HASH),
    (lit(b"BatchAABB"), 0x80, STRUCT),
    (lit(b"InstanceList"), 0x178, ARRAY),
];
const BOX: &[Member] = &[(lit(b"min"), 0x00, VEC4V), (lit(b"max"), 0x10, VEC4V)];
const PROP_DATA: &[Member] = &[(lit(b"InstMat"), 0x00, ARRAY_MEMBER)];
const GRASS_DATA: &[Member] = &[(lit(b"Position"), 0x00, ARRAY_MEMBER)];
const MAP_TYPES: &[Member] = &[
    (lit(b"archetypes"), 0x18, ARRAY),
    (lit(b"name"), 0x28, STRING_HASH),
];
const ARCHETYPE: &[Member] = &[
    (0x8BFB_6095, 0x08, HASH64),
    (0x3594_390A, 0x10, HASH64),
    (0x4101_BA4D, 0x18, HASH64),
    (0x99D5_6B7B, 0x20, HASH64),
    (0xD76A_C020, 0x28, HASH64),
    (lit(b"lodDist"), 0x38, FLOAT),
    (lit(b"specialAttribute"), 0x3C, U32),
    (lit(b"bbMin"), 0x40, VEC3),
    (lit(b"bbMax"), 0x4C, VEC3),
    (lit(b"bsCentre"), 0x58, VEC3),
    (lit(b"bsRadius"), 0x64, FLOAT),
    (lit(b"flags"), 0x68, U32),
    (lit(b"hdTextureDist"), 0x70, FLOAT),
    (lit(b"name"), 0x78, STRING_HASH),
    (lit(b"clipDictionary"), 0x7C, STRING_HASH),
    (lit(b"physicsDictionary"), 0x80, STRING_HASH),
    (lit(b"assetType"), 0x84, ENUM32),
    (lit(b"assetName"), 0x88, STRING_HASH),
];

/// Each class this reader reads, its members, and the size it reads its records at.
const LAYOUTS: &[(u32, &str, &[Member], Option<u32>)] = &[
    (C_MAP_DATA, "CMapData", MAP_DATA, None),
    (C_ENTITY_DEF, "CEntityDef", ENTITY, Some(ENTITY_SIZE)),
    (C_MLO_INSTANCE_DEF, "CMloInstanceDef", ENTITY, None),
    (
        INSTANCED_MAP_DATA,
        "rage__fwInstancedMapData",
        INSTANCED,
        None,
    ),
    (
        PROP_LIST,
        "rage__fwPropInstanceListDef",
        PROPS,
        Some(PROP_LIST_SIZE),
    ),
    (
        GRASS_LIST,
        "rage__fwGrassInstanceListDef",
        GRASS,
        Some(GRASS_LIST_SIZE),
    ),
    (
        PROP_INSTANCE,
        "rage__fwPropInstanceListDef__InstanceData",
        PROP_DATA,
        Some(0x40),
    ),
    (
        GRASS_INSTANCE,
        "rage__fwGrassInstanceListDef__InstanceData",
        GRASS_DATA,
        Some(0x10),
    ),
    (AABB, "rage__spdAABB", BOX, None),
    (C_MAP_TYPES, "CMapTypes", MAP_TYPES, None),
    (C_BASE_ARCHETYPE_DEF, "CBaseArchetypeDef", ARCHETYPE, None),
    (C_TIME_ARCHETYPE_DEF, "CTimeArchetypeDef", ARCHETYPE, None),
    (C_MLO_ARCHETYPE_DEF, "CMloArchetypeDef", ARCHETYPE, None),
];

const ENTITY_SIZE: u32 = 0x90;
const PROP_LIST_SIZE: u32 = 0x170;
const GRASS_LIST_SIZE: u32 = 0x1A0;
const MAP_DATA_SIZE: usize = 0x460;
const MAP_TYPES_SIZE: usize = 0x50;
const ARCHETYPE_SIZE: usize = 0xB8;

/// The `LODTYPES_DEPTH_*` names of `lodLevel`, by value.
pub const LOD_LEVELS: [&str; 7] = [
    "LODTYPES_DEPTH_HD",
    "LODTYPES_DEPTH_LOD",
    "LODTYPES_DEPTH_SLOD1",
    "LODTYPES_DEPTH_SLOD2",
    "LODTYPES_DEPTH_SLOD3",
    "LODTYPES_DEPTH_ORPHANHD",
    "LODTYPES_DEPTH_SLOD4",
];
const ORPHAN_HD: u8 = 5;

/// The `ASSET_TYPE_*` names of `assetType`, by value.
pub const ASSET_TYPES: [&str; 4] = [
    "ASSET_TYPE_UNINITIALIZED",
    "ASSET_TYPE_FRAGMENT",
    "ASSET_TYPE_DRAWABLE",
    "ASSET_TYPE_DRAWABLEDICTIONARY",
];

/// Why a map body is not read.
#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error(transparent)]
    Meta(#[from] crate::meta::MetaError),
    #[error("the root structure {0:#010X} is neither CMapData nor CMapTypes")]
    NotAMap(u32),
    #[error("{class}'s member {member:#010X} is not a type {code:#04X} member at {offset:#X}, where every measured body has it")]
    Layout {
        class: &'static str,
        member: u32,
        offset: u32,
        code: u8,
    },
    #[error("{class} is {found:#X} bytes, where every measured body's is {expected:#X}")]
    Size {
        class: &'static str,
        found: u32,
        expected: u32,
    },
    #[error("{what}: {why}")]
    Broken { what: String, why: String },
}

type Result<T> = std::result::Result<T, MapError>;

fn broken(what: impl Into<String>, why: impl Into<String>) -> MapError {
    MapError::Broken {
        what: what.into(),
        why: why.into(),
    }
}

/// A box in world space, as two corners.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// What a map body holds.
#[derive(Debug, Clone, PartialEq)]
pub enum MapBody {
    Data(MapData),
    Types(MapTypes),
}

/// A `CMapData`: a map's own fields, its entities, and its instance lists counted.
#[derive(Debug, Clone, PartialEq)]
pub struct MapData {
    /// The 32-bit hash of the lower-cased map name.
    pub name: u32,
    /// Murmur3 x64-128's low half of the same name (`hash_8BFB6095`).
    pub name_word: u64,
    /// The map whose entities and prop lists `parentIndex` indexes, 0 for none.
    pub parent: u32,
    /// Four words that each name another map body when not zero; what those maps are is not known.
    pub hash_6c0f9c8c: u32,
    pub hash_9e8d32a0: u32,
    pub hash_ec16451d: u32,
    pub hash_36c898aa: u32,
    pub flags: u32,
    pub content_flags: u32,
    pub streaming_extents: Aabb,
    pub entities_extents: Aabb,
    pub entities: Vec<Entity>,
    pub prop_lists: Vec<InstanceList>,
    pub grass_lists: Vec<InstanceList>,
}

/// A `CEntityDef`, or the `CMloInstanceDef` that places an interior.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub interior: bool,
    pub id: u64,
    /// x, y, z, w as stored; see [`Entity::model_rotation`].
    pub rotation: [f32; 4],
    /// Murmur3 low of the archetype's name, the archetype's `name_word` (`hash_C8A7B90A`).
    pub archetype_word: u64,
    pub position: [f32; 3],
    pub archetype_name: u32,
    pub flags: u32,
    pub scale_xy: f32,
    pub scale_z: f32,
    /// An index into the entities, then the prop and grass lists, of the map its selector picks.
    pub parent_index: i16,
    /// Below the instance count of a prop list `parent_index` names; what it holds is not measured.
    pub hash_e74c3c23: i16,
    /// −1 where the archetype's applies.
    pub lod_dist: f32,
    pub child_lod_dist: f32,
    pub num_children: u32,
    pub tint_value: u32,
    pub priority_level: u8,
    /// An index into [`LOD_LEVELS`].
    pub lod_level: u8,
}

impl Entity {
    /// The quaternion that turns the model: a `CEntityDef` stores its conjugate, an interior the quaternion itself.
    pub fn model_rotation(&self) -> [f32; 4] {
        let [x, y, z, w] = self.rotation;
        match self.interior {
            true => self.rotation,
            false => [-x, -y, -z, w],
        }
    }

    pub fn lod_level_name(&self) -> &'static str {
        LOD_LEVELS[usize::from(self.lod_level)]
    }
}

/// A prop or grass instance list: its archetype, how many instances it holds, and their box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceList {
    pub archetype_name: u32,
    pub instances: usize,
    pub batch_aabb: Aabb,
}

/// A `CMapTypes`: its name and archetypes.
#[derive(Debug, Clone, PartialEq)]
pub struct MapTypes {
    pub name: u32,
    pub archetypes: Vec<Archetype>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchetypeClass {
    Base,
    Time,
    Interior,
}

impl ArchetypeClass {
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "CBaseArchetypeDef",
            Self::Time => "CTimeArchetypeDef",
            Self::Interior => "CMloArchetypeDef",
        }
    }
}

/// A `CBaseArchetypeDef`, `CTimeArchetypeDef` or `CMloArchetypeDef`, through the members they share.
#[derive(Debug, Clone, PartialEq)]
pub struct Archetype {
    pub class: ArchetypeClass,
    pub name: u32,
    /// Murmur3 low of the lower-cased name (`hash_8BFB6095`), which an entity names it by.
    pub name_word: u64,
    /// The texture dictionary's word, inferred from its namespace (`hash_3594390A`).
    pub texture_dictionary: u64,
    /// Set in a drawable dictionary archetype alone (`hash_4101BA4D`).
    pub drawable_dictionary: u64,
    /// Set in a drawable archetype alone, and equal to `name_word` (`hash_99D56B7B`).
    pub drawable: u64,
    /// Set in a fragment archetype alone (`hash_D76AC020`).
    pub fragment: u64,
    pub lod_dist: f32,
    pub special_attribute: u32,
    /// The model's box, in its own space.
    pub bb_min: [f32; 3],
    pub bb_max: [f32; 3],
    pub bs_centre: [f32; 3],
    pub bs_radius: f32,
    pub flags: u32,
    pub hd_texture_dist: f32,
    pub clip_dictionary: u32,
    pub physics_dictionary: u32,
    /// An index into [`ASSET_TYPES`].
    pub asset_type: u32,
    pub asset_name: u32,
}

impl Archetype {
    pub fn asset_type_name(&self) -> &'static str {
        ASSET_TYPES[self.asset_type as usize]
    }

    /// The word of the model the asset type names: the drawable, the fragment or the drawable dictionary; 0 for an interior.
    pub fn model_word(&self) -> u64 {
        self.drawable | self.fragment | self.drawable_dictionary
    }
}

/// The type hash of the root block a headerless RBM virtual section names, read from its header
/// and block table alone; `None` where they do not hold one.
pub fn root_type(virtual_section: &[u8]) -> Option<u32> {
    let u32_at = |at: usize| {
        Some(u32::from_le_bytes(
            virtual_section.get(at..at + 4)?.try_into().ok()?,
        ))
    };
    let root = u32_at(0x20)?;
    let table = u64::from_le_bytes(virtual_section.get(0x38..0x40)?.try_into().ok()?);
    let count = u16::from_le_bytes(virtual_section.get(0x4C..0x4E)?.try_into().ok()?);
    if table >> 28 != 5 || root == 0 || root > u32::from(count) {
        return None;
    }
    u32_at(usize::try_from(table & 0x0FFF_FFFF).ok()? + 16 * (root as usize - 1))
}

/// The 64-bit word a map data body states of its own name, from its root block alone; `None` for
/// any other body, and for one whose classes are not laid out as measured.
pub fn stated_name_word(virtual_section: &[u8], physical: &[u8]) -> Option<u64> {
    if root_type(virtual_section)? != C_MAP_DATA {
        return None;
    }
    name_word_of(&crate::meta::binary::rbm::gta6::read_model(virtual_section, physical).ok()?)
}

/// The name word of the map data body `model` is, read without its entities and lists.
fn name_word_of(model: &RbmModel) -> Option<u64> {
    check_layouts(model).ok()?;
    let blocks = Blocks::new(&model.data_blocks);
    let root = blocks
        .block(u64::from(model.root_block_id), "the root block")
        .ok()?;
    (root.data_type == C_MAP_DATA)
        .then(|| u64_at(&root.data, 0x10).ok())
        .flatten()
}

/// The map body a headerless RBM resource holds, from its two inflated sections.
pub fn open(virtual_section: &[u8], physical: &[u8]) -> Result<MapBody> {
    read(&crate::meta::binary::rbm::gta6::read_model(
        virtual_section,
        physical,
    )?)
}

/// The map body of a headerless RBM resource's class table and data blocks.
pub fn read(model: &RbmModel) -> Result<MapBody> {
    check_layouts(model)?;
    let blocks = Blocks::new(&model.data_blocks);
    let root = blocks.block(u64::from(model.root_block_id), "the root block")?;
    match root.data_type {
        C_MAP_DATA => read_map_data(&blocks, &root.data).map(MapBody::Data),
        C_MAP_TYPES => read_map_types(&blocks, &root.data).map(MapBody::Types),
        other => Err(MapError::NotAMap(other)),
    }
}

fn check_layouts(model: &RbmModel) -> Result<()> {
    for (hash, class, members, size) in LAYOUTS {
        let Some(declared) = model.classes.get(hash) else {
            continue;
        };
        if let Some(expected) = size.filter(|s| *s != declared.data_size) {
            return Err(MapError::Size {
                class,
                found: declared.data_size,
                expected,
            });
        }
        for &(member, offset, code) in *members {
            let fits = declared.entries.iter().any(|e| {
                e.name_hash == member && e.offset == offset && e.data_type.to_byte() == code
            });
            if !fits {
                return Err(MapError::Layout {
                    class,
                    member,
                    offset,
                    code,
                });
            }
        }
    }
    Ok(())
}

/// The data blocks, which a pointer inside data names by 1-based id, and how many more array
/// elements a read may take: arrays that alias one block cannot multiply past what its bytes hold.
struct Blocks<'m>(&'m [RbmDataBlock], std::cell::Cell<usize>);

impl<'m> Blocks<'m> {
    fn new(blocks: &'m [RbmDataBlock]) -> Self {
        // An element is at least four bytes, so the arrays of a body that aliases none fit this.
        let bytes: usize = blocks.iter().map(|b| b.data.len()).sum();
        Self(blocks, std::cell::Cell::new(bytes / 4))
    }

    fn block(&self, id: u64, what: &str) -> Result<&'m RbmDataBlock> {
        (id.checked_sub(1))
            .and_then(|i| self.0.get(usize::try_from(i).ok()?))
            .ok_or_else(|| broken(what, format!("it names block {id} of {}", self.0.len())))
    }

    /// The block a pointer names and its bytes from the offset on; `None` for null.
    fn target(&self, raw: u64, what: &str) -> Result<Option<(u32, &'m [u8])>> {
        if raw & 0xFFF == 0 {
            return match raw {
                0 => Ok(None),
                _ => Err(broken(
                    what,
                    format!("the null pointer {raw:#X} carries an offset"),
                )),
            };
        }
        if (raw >> 12) & 0xFF != 0 {
            return Err(broken(
                what,
                format!("the pointer {raw:#X} sets bits 12 to 19"),
            ));
        }
        let block = self.block(raw & 0xFFF, what)?;
        let at = usize::try_from(raw >> 20).unwrap_or(usize::MAX);
        match block.data.get(at..).filter(|rest| !rest.is_empty()) {
            Some(rest) => Ok(Some((block.data_type, rest))),
            None => Err(broken(
                what,
                format!(
                    "the pointer {raw:#X} is past its {}-byte block",
                    block.data.len()
                ),
            )),
        }
    }

    /// An `AtArray64`'s element block, its bytes from the first element on, and its count.
    fn array(&self, bytes: &[u8], at: usize, what: &str) -> Result<Option<(u32, &'m [u8], usize)>> {
        let (raw, count, capacity) = (
            u64_at(bytes, at)?,
            u16_at(bytes, at + 8)?,
            u16_at(bytes, at + 10)?,
        );
        if count > capacity {
            return Err(broken(
                what,
                format!("its count {count} is over its capacity {capacity}"),
            ));
        }
        match self.target(raw, what)? {
            None if count == 0 => Ok(None),
            None => Err(broken(
                what,
                format!("{count} elements are behind a null pointer"),
            )),
            Some((kind, rest)) => {
                let left = (self.1.get().checked_sub(usize::from(count))).ok_or_else(|| {
                    broken(
                        what,
                        "the arrays of this body name more elements than its blocks hold",
                    )
                })?;
                self.1.set(left);
                Ok(Some((kind, rest, usize::from(count))))
            }
        }
    }

    /// The `count` records of `size` bytes an array of in-place structures of `class` holds.
    fn records(
        &self,
        bytes: &[u8],
        at: usize,
        class: u32,
        size: usize,
        what: &str,
    ) -> Result<Vec<&'m [u8]>> {
        let Some((kind, rest, count)) = self.array(bytes, at, what)? else {
            return Ok(Vec::new());
        };
        if kind != class || rest.len() < size * count {
            return Err(broken(
                what,
                format!("its {count} elements do not fit a block of their type, {class:#010X}"),
            ));
        }
        Ok(rest.chunks_exact(size).take(count).collect())
    }

    /// The records each element of an array of pointers names, with the class of each.
    fn pointed(
        &self,
        bytes: &[u8],
        at: usize,
        size: usize,
        what: &str,
    ) -> Result<Vec<(u32, &'m [u8])>> {
        let Some((_, rest, count)) = self.array(bytes, at, what)? else {
            return Ok(Vec::new());
        };
        if rest.len() < 8 * count {
            return Err(broken(
                what,
                format!("its {count} pointers run past their block"),
            ));
        }
        (0..count)
            .map(|k| {
                let what = format!("{what}[{k}]");
                match self.target(u64_at(rest, 8 * k)?, &what)? {
                    Some((class, record)) if record.len() >= size => Ok((class, &record[..size])),
                    Some(_) => Err(broken(what, "it runs past its block")),
                    None => Err(broken(what, "it is a null pointer")),
                }
            })
            .collect()
    }
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    (bytes.get(at..at + N).and_then(|b| b.try_into().ok())).ok_or_else(|| {
        broken(
            format!("the field at {at:#X}"),
            format!("it runs past the {}-byte record", bytes.len()),
        )
    })
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    field(bytes, at).map(u16::from_le_bytes)
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    field(bytes, at).map(u32::from_le_bytes)
}

fn u64_at(bytes: &[u8], at: usize) -> Result<u64> {
    field(bytes, at).map(u64::from_le_bytes)
}

fn f32_at(bytes: &[u8], at: usize, what: &str) -> Result<f32> {
    let value = field(bytes, at).map(f32::from_le_bytes)?;
    match value.is_finite() {
        true => Ok(value),
        false => Err(broken(what, format!("the number at {at:#X} is not finite"))),
    }
}

fn floats<const N: usize>(bytes: &[u8], at: usize, what: &str) -> Result<[f32; N]> {
    let mut out = [0.0; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = f32_at(bytes, at + 4 * i, what)?;
    }
    Ok(out)
}

fn aabb(bytes: &[u8], at: usize, what: &str) -> Result<Aabb> {
    Ok(Aabb {
        min: floats(bytes, at, what)?,
        max: floats(bytes, at + 0x10, what)?,
    })
}

fn read_map_data(blocks: &Blocks, root: &[u8]) -> Result<MapData> {
    if root.len() < MAP_DATA_SIZE {
        return Err(broken(
            "the root block",
            format!(
                "its {} bytes are fewer than CMapData's {MAP_DATA_SIZE:#X}",
                root.len()
            ),
        ));
    }
    let entities = blocks
        .pointed(root, 0xD0, 0x84, "entities")?
        .into_iter()
        .enumerate()
        .map(|(k, (class, record))| entity(k, class, record))
        .collect::<Result<Vec<_>>>()?;
    let lists =
        |at: usize, class: u32, size: u32, what: &str, prop: bool| -> Result<Vec<InstanceList>> {
            let records = blocks.records(root, 0x140 + at, class, size as usize, what)?;
            (records.into_iter().enumerate())
                .map(|(k, record)| instance_list(blocks, record, &format!("{what}[{k}]"), prop))
                .collect()
        };
    Ok(MapData {
        name: u32_at(root, 0x08)?,
        name_word: u64_at(root, 0x10)?,
        parent: u32_at(root, 0x68)?,
        hash_6c0f9c8c: u32_at(root, 0x50)?,
        hash_9e8d32a0: u32_at(root, 0x6C)?,
        hash_ec16451d: u32_at(root, 0x70)?,
        hash_36c898aa: u32_at(root, 0x74)?,
        flags: u32_at(root, 0x80)?,
        content_flags: u32_at(root, 0x84)?,
        streaming_extents: aabb(root, 0x90, "streamingExtents")?,
        entities_extents: aabb(root, 0xB0, "entitiesExtents")?,
        entities,
        prop_lists: lists(0x08, PROP_LIST, PROP_LIST_SIZE, "PropInstanceList", true)?,
        grass_lists: lists(
            0x18,
            GRASS_LIST,
            GRASS_LIST_SIZE,
            "GrassInstanceList",
            false,
        )?,
    })
}

fn entity(k: usize, class: u32, e: &[u8]) -> Result<Entity> {
    let what = format!("entities[{k}]");
    let interior = match class {
        C_ENTITY_DEF => false,
        C_MLO_INSTANCE_DEF => true,
        other => {
            return Err(broken(
                what,
                format!("it points at a block of type {other:#010X}, no entity class"),
            ))
        }
    };
    let rotation: [f32; 4] = floats(e, 0x10, &what)?;
    let norm = rotation.iter().map(|x| x * x).sum::<f32>().sqrt();
    if (norm - 1.0).abs() >= 1e-3 {
        return Err(broken(
            what,
            format!("its rotation's norm is {norm:.5}, not 1"),
        ));
    }
    let read = Entity {
        interior,
        id: u64_at(e, 0x08)?,
        rotation,
        archetype_word: u64_at(e, 0x20)?,
        position: floats(e, 0x38, &what)?,
        archetype_name: u32_at(e, 0x44)?,
        flags: u32_at(e, 0x48)?,
        scale_xy: f32_at(e, 0x54, &what)?,
        scale_z: f32_at(e, 0x58, &what)?,
        parent_index: i16::from_le_bytes(field(e, 0x5C)?),
        hash_e74c3c23: i16::from_le_bytes(field(e, 0x5E)?),
        lod_dist: f32_at(e, 0x60, &what)?,
        child_lod_dist: f32_at(e, 0x64, &what)?,
        num_children: u32_at(e, 0x68)?,
        tint_value: u32_at(e, 0x70)?,
        priority_level: field::<1>(e, 0x7C)?[0],
        lod_level: field::<1>(e, 0x7E)?[0],
    };
    let why = if read.archetype_name == 0 {
        Some("its archetypeName is 0".to_string())
    } else if read.parent_index < -1 {
        Some(format!("its parentIndex {} is below -1", read.parent_index))
    } else if usize::from(read.lod_level) >= LOD_LEVELS.len() {
        Some(format!(
            "its lodLevel {} is no LODTYPES_DEPTH value",
            read.lod_level
        ))
    } else if read.lod_level == ORPHAN_HD && (read.parent_index != -1 || read.num_children != 0) {
        Some(format!(
            "an ORPHANHD entity has parentIndex {} and {} children",
            read.parent_index, read.num_children
        ))
    } else if read.lod_level == 0 && read.num_children != 0 {
        Some(format!("an HD entity has {} children", read.num_children))
    } else if !matches!(read.lod_level, 0 | ORPHAN_HD) && read.num_children == 0 {
        Some(format!(
            "a lodLevel {} entity has no children",
            read.lod_level
        ))
    } else {
        None
    };
    match why {
        Some(why) => Err(broken(what, why)),
        None => Ok(read),
    }
}

fn instance_list(blocks: &Blocks, list: &[u8], what: &str, prop: bool) -> Result<InstanceList> {
    let (box_at, array_at, class, stride) = match prop {
        true => (0xA0, 0x90, PROP_INSTANCE, 0x40),
        false => (0x80, 0x178, GRASS_INSTANCE, 0x10),
    };
    let batch_aabb = aabb(list, box_at, what)?;
    if (0..3).any(|i| batch_aabb.min[i] > batch_aabb.max[i]) {
        return Err(broken(what, "its BatchAABB's min is over its max"));
    }
    let instances = blocks.records(
        list,
        array_at,
        class,
        stride,
        &format!("{what}.InstanceList"),
    )?;
    Ok(InstanceList {
        archetype_name: u32_at(list, 0x44)?,
        instances: instances.len(),
        batch_aabb,
    })
}

fn read_map_types(blocks: &Blocks, root: &[u8]) -> Result<MapTypes> {
    if root.len() < MAP_TYPES_SIZE {
        return Err(broken(
            "the root block",
            format!(
                "its {} bytes are fewer than CMapTypes' {MAP_TYPES_SIZE:#X}",
                root.len()
            ),
        ));
    }
    let archetypes = blocks
        .pointed(root, 0x18, ARCHETYPE_SIZE, "archetypes")?
        .into_iter()
        .enumerate()
        .map(|(k, (class, record))| archetype(k, class, record))
        .collect::<Result<Vec<_>>>()?;
    Ok(MapTypes {
        name: u32_at(root, 0x28)?,
        archetypes,
    })
}

fn archetype(k: usize, class: u32, a: &[u8]) -> Result<Archetype> {
    let what = format!("archetypes[{k}]");
    let class = match class {
        C_BASE_ARCHETYPE_DEF => ArchetypeClass::Base,
        C_TIME_ARCHETYPE_DEF => ArchetypeClass::Time,
        C_MLO_ARCHETYPE_DEF => ArchetypeClass::Interior,
        other => {
            return Err(broken(
                what,
                format!("it points at a block of type {other:#010X}, no archetype class"),
            ))
        }
    };
    let read = Archetype {
        class,
        name: u32_at(a, 0x78)?,
        name_word: u64_at(a, 0x08)?,
        texture_dictionary: u64_at(a, 0x10)?,
        drawable_dictionary: u64_at(a, 0x18)?,
        drawable: u64_at(a, 0x20)?,
        fragment: u64_at(a, 0x28)?,
        lod_dist: f32_at(a, 0x38, &what)?,
        special_attribute: u32_at(a, 0x3C)?,
        bb_min: floats(a, 0x40, &what)?,
        bb_max: floats(a, 0x4C, &what)?,
        bs_centre: floats(a, 0x58, &what)?,
        bs_radius: f32_at(a, 0x64, &what)?,
        flags: u32_at(a, 0x68)?,
        hd_texture_dist: f32_at(a, 0x70, &what)?,
        clip_dictionary: u32_at(a, 0x7C)?,
        physics_dictionary: u32_at(a, 0x80)?,
        asset_type: u32_at(a, 0x84)?,
        asset_name: u32_at(a, 0x88)?,
    };
    // The model word the asset type names is the one set: +0x20 drawable, +0x28 fragment, +0x18 dictionary.
    let set = [
        read.drawable != 0,
        read.drawable_dictionary != 0,
        read.fragment != 0,
    ];
    let wanted = match read.asset_type {
        0 => [false, false, false],
        1 => [false, false, true],
        2 => [true, false, false],
        3 => [false, true, false],
        other => {
            return Err(broken(
                what,
                format!("its assetType {other} is none of the four measured"),
            ))
        }
    };
    let why = if (0..3).any(|i| read.bb_min[i] > read.bb_max[i]) {
        Some("its bbMin is over its bbMax".to_string())
    } else if read.bs_radius < 0.0 {
        Some("its bsRadius is below 0".to_string())
    } else if read.name == 0 {
        Some("its name is 0".to_string())
    } else if set != wanted {
        Some(format!(
            "its assetType {} disagrees with the model words it sets",
            read.asset_type
        ))
    } else if read.asset_type == 2 && read.drawable != read.name_word {
        Some("its drawable word differs from its name word".to_string())
    } else {
        None
    };
    match why {
        Some(why) => Err(broken(what, why)),
        None => Ok(read),
    }
}

#[path = "map_links.rs"]
pub mod links;

#[path = "map_grow.rs"]
pub mod grow;

#[path = "map_lights.rs"]
pub mod lights;

#[path = "map_light_ext.rs"]
pub mod light_ext;

#[path = "map_lanes.rs"]
pub mod lanes;

#[path = "map_world.rs"]
pub mod world;

#[path = "map_author.rs"]
pub mod author;

#[path = "map_stream.rs"]
pub mod stream;

#[path = "map_interior.rs"]
pub mod interior;

#[path = "map_occluders.rs"]
pub mod occluders;

#[path = "map_index_types.rs"]
pub mod index_types;

#[path = "map_lights_lod.rs"]
pub mod lights_lod;

#[path = "map_lights_index.rs"]
pub mod lights_index;

#[cfg(test)]
#[path = "map_tests.rs"]
mod tests;
