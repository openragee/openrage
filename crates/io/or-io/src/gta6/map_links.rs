//! What a GTA VI map body links to: a `CMapData`'s level-of-detail parent maps and each element's
//! pick of one, its instances' points, and a `CMapTypes`' interiors with their rooms, portals and
//! entity sets.
//!
//! ⚠ A prop list's `numChildren`, the archetype each instance of a grass list with several takes,
//! and where an interior's contents stand in the world are not known, so nothing here reads them.

use super::{
    aabb, broken, f32_at, field, floats, u16_at, u32_at, u64_at, Blocks, MapBody, MapError, Member,
    Result, ARRAY, C_ENTITY_DEF, C_MAP_DATA, C_MAP_TYPES, C_MLO_ARCHETYPE_DEF, C_MLO_INSTANCE_DEF,
    ENUM8, GRASS_INSTANCE, GRASS_LIST, GRASS_LIST_SIZE, MAP_DATA_SIZE, MAP_TYPES_SIZE, ORPHAN_HD,
    PROP_INSTANCE, PROP_LIST, PROP_LIST_SIZE, S16, STRING_HASH, STRUCT, U32, VEC3V,
};
use crate::hash::literal_string_hash as lit;
use crate::meta::binary::rbm::schema::RbmModel;

const C_MLO_ROOM_DEF: u32 = lit(b"CMloRoomDef");
const C_MLO_PORTAL_DEF: u32 = lit(b"CMloPortalDef");
const C_MLO_ENTITY_SET: u32 = lit(b"CMloEntitySet");
const C_MLO_TIME_CYCLE_MODIFIER: u32 = lit(b"CMloTimeCycleModifier");
const C_MILO_INTERIOR: u32 = lit(b"CMiloInterior");

const S8: u8 = 0x10;
const U16: u8 = 0x13;
const S32: u8 = 0x14;
const COUNTED: u8 = 0x5C;

const SELECTOR: u32 = 0x8479_9847;
const MLO_SIZE: usize = 0x220;
const ROOM_SIZE: usize = 0x70;
const PORTAL_SIZE: usize = 0x58;
const ENTITY_SET_SIZE: usize = 0x28;
const TIME_CYCLE_MODIFIER_SIZE: usize = 0x30;
/// The bytes of a `CEntityDef` this reads, as the map reader reads its entities.
const ENTITY_READ: usize = 0x84;
/// An entity-set location with bit 31 set names a portal, not a room.
const PORTAL_BIT: u32 = 0x8000_0000;

pub(super) const LAYOUTS: &[(u32, &str, &[Member], Option<u32>)] = &[
    (
        C_MAP_DATA,
        "CMapData",
        &[(0x86AA_B4BF, 0x18, COUNTED), (0x75D1_6ADF, 0x58, ARRAY)],
        None,
    ),
    (C_ENTITY_DEF, "CEntityDef", &[(SELECTOR, 0x7F, S8)], None),
    (
        C_MLO_INSTANCE_DEF,
        "CMloInstanceDef",
        &[(SELECTOR, 0x7F, S8)],
        None,
    ),
    (
        PROP_LIST,
        "rage__fwPropInstanceListDef",
        &[
            (SELECTOR, 0x7F, S8),
            (lit(b"parentIndex"), 0x5C, S16),
            (0xE74C_3C23, 0x5E, S16),
            (lit(b"numChildren"), 0x68, U32),
            (lit(b"lodLevel"), 0x7E, ENUM8),
        ],
        Some(PROP_LIST_SIZE),
    ),
    (
        GRASS_LIST,
        "rage__fwGrassInstanceListDef",
        &[
            (lit(b"parentIndex"), 0x5C, S16),
            (0xB451_FC98, 0x158, ARRAY),
            (0xDDD4_7DA7, 0x168, ARRAY),
        ],
        Some(GRASS_LIST_SIZE),
    ),
    (
        GRASS_INSTANCE,
        "rage__fwGrassInstanceListDef__InstanceData",
        &[(0xFC42_6F96, 0x0E, U16)],
        None,
    ),
    (
        C_MLO_ARCHETYPE_DEF,
        "CMloArchetypeDef",
        &[
            (lit(b"entities"), 0xC0, ARRAY),
            (lit(b"rooms"), 0xD0, ARRAY),
            (lit(b"portals"), 0xE0, ARRAY),
            (lit(b"entitySets"), 0xF0, ARRAY),
            (lit(b"timeCycleModifiers"), 0x100, ARRAY),
            (0x62C5_83E1, 0x158, STRUCT),
        ],
        Some(MLO_SIZE as u32),
    ),
    (
        C_MLO_ROOM_DEF,
        "CMloRoomDef",
        &[
            (lit(b"name"), 0x00, STRING_HASH),
            (lit(b"bbMin"), 0x10, VEC3V),
            (lit(b"bbMax"), 0x20, VEC3V),
            (lit(b"portalCount"), 0x40, U32),
            (lit(b"floorId"), 0x44, S32),
            (lit(b"attachedObjects"), 0x50, ARRAY),
        ],
        Some(ROOM_SIZE as u32),
    ),
    (
        C_MLO_PORTAL_DEF,
        "CMloPortalDef",
        &[
            (lit(b"roomFrom"), 0x08, U32),
            (lit(b"roomTo"), 0x0C, U32),
            (lit(b"flags"), 0x10, U32),
            (lit(b"corners"), 0x38, ARRAY),
            (lit(b"attachedObjects"), 0x48, ARRAY),
        ],
        Some(PORTAL_SIZE as u32),
    ),
    (
        C_MLO_ENTITY_SET,
        "CMloEntitySet",
        &[
            (lit(b"name"), 0x00, STRING_HASH),
            (lit(b"locations"), 0x08, ARRAY),
            (lit(b"entities"), 0x18, ARRAY),
        ],
        Some(ENTITY_SET_SIZE as u32),
    ),
    (
        C_MLO_TIME_CYCLE_MODIFIER,
        "CMloTimeCycleModifier",
        &[],
        Some(TIME_CYCLE_MODIFIER_SIZE as u32),
    ),
    (
        C_MILO_INTERIOR,
        "CMiloInterior",
        &[
            (0xE5D6_D4D4, 0x50, ARRAY),
            (0x425E_F9AD, 0x60, ARRAY),
            (0xD5DB_D10A, 0x70, ARRAY),
            (0xAE97_C3BE, 0x80, ARRAY),
        ],
        None,
    ),
];

/// A map a `CMapData` names as a level-of-detail parent: its 32-bit name and its 64-bit word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodParentMap {
    pub name: u32,
    pub word: u64,
}

/// An entity's or prop list's link to its level-of-detail parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodLink {
    /// The index into [`MapLinks::lod_parents`] (`hash_84799847`), `None` for none.
    pub map: Option<usize>,
    /// An index into the selected map's entities, then its prop lists, then its grass lists.
    pub parent_index: i16,
    /// The instance of a parent prop list (`hash_E74C3C23`), −1 for none.
    pub instance: i16,
}

/// What a [`LodLink`]'s `parent_index` names in its parent map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LodTarget {
    Entity(usize),
    PropList {
        list: usize,
        instance: Option<usize>,
    },
    GrassList(usize),
}

impl LodLink {
    /// What this link names in a parent map of these counts; `None` past them all or for no index.
    pub fn target(
        &self,
        entities: usize,
        prop_lists: usize,
        grass_lists: usize,
    ) -> Option<LodTarget> {
        let at = usize::try_from(self.parent_index).ok()?;
        if at < entities {
            return Some(LodTarget::Entity(at));
        }
        let at = at - entities;
        if at < prop_lists {
            return Some(LodTarget::PropList {
                list: at,
                instance: usize::try_from(self.instance).ok(),
            });
        }
        let at = at - prop_lists;
        (at < grass_lists).then_some(LodTarget::GrassList(at))
    }
}

/// A grass list's archetypes, when it names several, and each instance's position.
#[derive(Debug, Clone, PartialEq)]
pub struct GrassLinks {
    /// The archetypes' names (`hash_B451FC98`) and words (`hash_DDD47DA7`); empty for one archetype.
    pub archetypes: Vec<(u32, u64)>,
    /// Each instance's `Position`, dequantised over the list's `BatchAABB`.
    pub points: Vec<[f32; 3]>,
}

/// What a `CMapData` links to.
#[derive(Debug, Clone, PartialEq)]
pub struct MapLinks {
    pub lod_parents: Vec<LodParentMap>,
    /// Each entity's link, in the order of the map's entities.
    pub entities: Vec<LodLink>,
    /// Each prop list's link, in the order of its prop lists.
    pub prop_lists: Vec<LodLink>,
    /// Each prop list's instances, each the translation its `InstMat` rows hold.
    pub prop_points: Vec<Vec<[f32; 3]>>,
    pub grass_lists: Vec<GrassLinks>,
}

/// An entity of an interior or of one of its entity sets, in the interior's own space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteriorEntity {
    pub archetype_name: u32,
    pub archetype_word: u64,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}

/// A `CMloRoomDef`.
#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    pub name: u32,
    pub bb_min: [f32; 3],
    pub bb_max: [f32; 3],
    pub flags: u32,
    /// The portals whose `roomFrom` or `roomTo` is this room.
    pub portal_count: u32,
    pub floor_id: i32,
    /// Indices into the interior's entities.
    pub attached_objects: Vec<u32>,
}

/// A `CMloPortalDef`.
#[derive(Debug, Clone, PartialEq)]
pub struct Portal {
    pub room_from: u32,
    pub room_to: u32,
    pub flags: u32,
    pub corners: [[f32; 3]; 4],
    /// Indices into the interior's entities.
    pub attached_objects: Vec<u32>,
}

/// Where an entity set's entity stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitySetLocation {
    Room(u32),
    Portal(u32),
}

/// A `CMloEntitySet`: each entity with its location.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySet {
    pub name: u32,
    pub entities: Vec<(EntitySetLocation, InteriorEntity)>,
}

/// A `CMloTimeCycleModifier`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeCycleModifier {
    pub name: u32,
    pub sphere: [f32; 4],
    pub percentage: f32,
    pub range: f32,
    pub start_hour: u32,
    pub end_hour: u32,
}

/// A `CMloArchetypeDef`'s contents; each entity is held by exactly one room or portal.
#[derive(Debug, Clone, PartialEq)]
pub struct Interior {
    /// The archetype's index in its `CMapTypes`.
    pub archetype: usize,
    pub name: u32,
    pub name_word: u64,
    pub entities: Vec<InteriorEntity>,
    pub rooms: Vec<Room>,
    pub portals: Vec<Portal>,
    pub entity_sets: Vec<EntitySet>,
    pub time_cycle_modifiers: Vec<TimeCycleModifier>,
}

/// What a map body links to.
#[derive(Debug, Clone, PartialEq)]
pub enum Links {
    Data(MapLinks),
    /// A `CMapTypes`' interiors, in the order of its archetypes.
    Types(Vec<Interior>),
}

/// The map body a headerless RBM resource holds and what it links to, from its two sections.
pub fn open(virtual_section: &[u8], physical: &[u8]) -> Result<(MapBody, Result<Links>)> {
    let model = crate::meta::binary::rbm::gta6::read_model(virtual_section, physical)?;
    Ok((super::read(&model)?, read(&model)))
}

/// What the map body of a headerless RBM resource's class table and data blocks links to.
pub fn read(model: &RbmModel) -> Result<Links> {
    check_layouts(model)?;
    let blocks = Blocks::new(&model.data_blocks);
    let root = blocks.block(u64::from(model.root_block_id), "the root block")?;
    match root.data_type {
        C_MAP_DATA if root.data.len() >= MAP_DATA_SIZE => {
            map_links(&blocks, &root.data).map(Links::Data)
        }
        C_MAP_TYPES if root.data.len() >= MAP_TYPES_SIZE => {
            interiors(&blocks, &root.data).map(Links::Types)
        }
        C_MAP_DATA | C_MAP_TYPES => Err(broken("the root block", "it is shorter than its class")),
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
            let fits = (declared.entries.iter()).any(|e| {
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

/// The `count` elements of `size` bytes an `AtArray64` at `at` addresses, of any element class.
fn elements<'m>(
    blocks: &Blocks<'m>,
    bytes: &[u8],
    at: usize,
    size: usize,
    what: &str,
) -> Result<Vec<&'m [u8]>> {
    let Some((_, rest, count)) = blocks.array(bytes, at, what)? else {
        return Ok(Vec::new());
    };
    if rest.len() < size * count {
        return Err(broken(
            what,
            format!("its {count} elements run past their block"),
        ));
    }
    Ok(rest.chunks_exact(size).take(count).collect())
}

fn words(blocks: &Blocks, bytes: &[u8], at: usize, what: &str) -> Result<Vec<u32>> {
    (elements(blocks, bytes, at, 4, what)?.into_iter())
        .map(|w| u32_at(w, 0))
        .collect()
}

fn map_links(blocks: &Blocks, root: &[u8]) -> Result<MapLinks> {
    let name = u32_at(root, 0x08)?;
    // 0x18: {u32 count, the same count, u64 pointer} of the parents' 64-bit words.
    let count = u32_at(root, 0x18)?;
    if u32_at(root, 0x1C)? != count {
        return Err(broken("CMapData 0x18", "its two counts differ"));
    }
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let parent_words: Vec<u64> = match blocks.target(u64_at(root, 0x20)?, "CMapData 0x18")? {
        None if count == 0 => Vec::new(),
        None => {
            return Err(broken(
                "CMapData 0x18",
                format!("{count} words are behind a null pointer"),
            ))
        }
        Some(_) if count == 0 => {
            return Err(broken("CMapData 0x18", "no words are behind a pointer"))
        }
        Some((_, rest)) if rest.len() / 8 < count => {
            return Err(broken(
                "CMapData 0x18",
                format!("its {count} words run past their block"),
            ))
        }
        Some((_, rest)) => (0..count)
            .map(|k| u64_at(rest, 8 * k))
            .collect::<Result<_>>()?,
    };
    let names = words(blocks, root, 0x58, "CMapData 0x58")?;
    if names.len() != parent_words.len() {
        return Err(broken(
            "CMapData 0x58",
            format!(
                "it names {} maps, where 0x18 holds {}",
                names.len(),
                parent_words.len()
            ),
        ));
    }
    if names.iter().any(|n| *n == 0 || *n == name) {
        return Err(broken("CMapData 0x58", "it names 0 or the map itself"));
    }
    let lod_parents: Vec<LodParentMap> = (names.into_iter().zip(parent_words))
        .map(|(name, word)| LodParentMap { name, word })
        .collect();
    let parents = lod_parents.len();
    let entities = (blocks
        .pointed(root, 0xD0, ENTITY_READ, "entities")?
        .into_iter()
        .enumerate())
    .map(|(k, (_, e))| link(e, parents, &format!("entities[{k}]")))
    .collect::<Result<Vec<_>>>()?;
    let props = blocks.records(
        root,
        0x148,
        PROP_LIST,
        PROP_LIST_SIZE as usize,
        "PropInstanceList",
    )?;
    let mut prop_lists = Vec::with_capacity(props.len());
    let mut prop_points = Vec::with_capacity(props.len());
    for (k, list) in props.into_iter().enumerate() {
        let what = format!("PropInstanceList[{k}]");
        prop_lists.push(link(list, parents, &what)?);
        let instances = blocks.records(list, 0x90, PROP_INSTANCE, 0x40, &what)?;
        // Each of the three `InstMat` rows ends in one coordinate of the translation.
        let points = (instances.into_iter())
            .map(|i| {
                Ok([
                    f32_at(i, 0x0C, &what)?,
                    f32_at(i, 0x1C, &what)?,
                    f32_at(i, 0x2C, &what)?,
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        prop_points.push(points);
    }
    let grass = blocks.records(
        root,
        0x158,
        GRASS_LIST,
        GRASS_LIST_SIZE as usize,
        "GrassInstanceList",
    )?;
    let grass_lists = (grass.into_iter().enumerate())
        .map(|(k, list)| grass_links(blocks, list, &format!("GrassInstanceList[{k}]")))
        .collect::<Result<Vec<_>>>()?;
    Ok(MapLinks {
        lod_parents,
        entities,
        prop_lists,
        prop_points,
        grass_lists,
    })
}

/// The link of an entity or prop list record, which holds its selector at 0x7F.
fn link(record: &[u8], parents: usize, what: &str) -> Result<LodLink> {
    let selector = field::<1>(record, 0x7F)?[0] as i8;
    let parent_index = i16::from_le_bytes(field(record, 0x5C)?);
    let instance = i16::from_le_bytes(field(record, 0x5E)?);
    let level = field::<1>(record, 0x7E)?[0];
    let map = match selector {
        -1 => None,
        s if s < -1 => {
            return Err(broken(
                what,
                format!("its parent-map selector {s} is below -1"),
            ))
        }
        s if s as usize >= parents => {
            return Err(broken(
                what,
                format!("its parent-map selector {s} is past the {parents} maps"),
            ))
        }
        s if parent_index < 0 => {
            return Err(broken(
                what,
                format!("its parent-map selector {s} comes with parentIndex {parent_index}"),
            ))
        }
        _ if level == ORPHAN_HD => {
            return Err(broken(what, "an ORPHANHD element names a parent map"))
        }
        s => Some(s as usize),
    };
    Ok(LodLink {
        map,
        parent_index,
        instance,
    })
}

fn grass_links(blocks: &Blocks, list: &[u8], what: &str) -> Result<GrassLinks> {
    if i16::from_le_bytes(field(list, 0x5C)?) != -1 {
        return Err(broken(what, "a grass list names a parent"));
    }
    let names = words(blocks, list, 0x158, what)?;
    let archetype_words = elements(blocks, list, 0x168, 8, what)?;
    if names.len() != archetype_words.len() {
        return Err(broken(
            what,
            format!(
                "it has {} archetype names and {} words",
                names.len(),
                archetype_words.len()
            ),
        ));
    }
    let archetypes = (names.into_iter().zip(archetype_words))
        .map(|(name, word)| Ok((name, u64_at(word, 0)?)))
        .collect::<Result<Vec<_>>>()?;
    let batch = aabb(list, 0x80, what)?;
    let instances = blocks.records(list, 0x178, GRASS_INSTANCE, 0x10, what)?;
    let mut points = Vec::with_capacity(instances.len());
    for i in instances {
        // 0x0E is set in every instance of a list with an archetype array and in none of another.
        if (u16_at(i, 0x0E)? == 0) != archetypes.is_empty() {
            return Err(broken(
                what,
                "an instance's word 0x0E disagrees with the list's archetype array",
            ));
        }
        let mut p = [0.0; 3];
        for (axis, slot) in p.iter_mut().enumerate() {
            let q = f32::from(u16_at(i, 2 * axis)?) / 65535.0;
            *slot = batch.min[axis] + (batch.max[axis] - batch.min[axis]) * q;
        }
        points.push(p);
    }
    Ok(GrassLinks { archetypes, points })
}

fn interiors(blocks: &Blocks, root: &[u8]) -> Result<Vec<Interior>> {
    let Some((_, rest, count)) = blocks.array(root, 0x18, "archetypes")? else {
        return Ok(Vec::new());
    };
    if rest.len() < 8 * count {
        return Err(broken(
            "archetypes",
            format!("its {count} pointers run past their block"),
        ));
    }
    let mut out = Vec::new();
    for k in 0..count {
        let what = format!("archetypes[{k}]");
        match blocks.target(u64_at(rest, 8 * k)?, &what)? {
            Some((C_MLO_ARCHETYPE_DEF, a)) if a.len() >= MLO_SIZE => {
                out.push(interior(blocks, k, a, &what)?)
            }
            Some((C_MLO_ARCHETYPE_DEF, _)) => return Err(broken(what, "it runs past its block")),
            _ => {}
        }
    }
    Ok(out)
}

fn interior_entities(
    blocks: &Blocks,
    bytes: &[u8],
    at: usize,
    what: &str,
) -> Result<Vec<InteriorEntity>> {
    (blocks
        .pointed(bytes, at, ENTITY_READ, what)?
        .into_iter()
        .enumerate())
    .map(|(k, (class, e))| match class {
        C_ENTITY_DEF => Ok(InteriorEntity {
            archetype_name: u32_at(e, 0x44)?,
            archetype_word: u64_at(e, 0x20)?,
            position: floats(e, 0x38, what)?,
            rotation: floats(e, 0x10, what)?,
        }),
        other => Err(broken(
            format!("{what}[{k}]"),
            format!("it points at a block of type {other:#010X}, no CEntityDef"),
        )),
    })
    .collect()
}

fn interior(blocks: &Blocks, archetype: usize, a: &[u8], what: &str) -> Result<Interior> {
    if u32_at(a, 0x84)? != 0
        || [0x10, 0x18, 0x20, 0x28]
            .iter()
            .any(|at| u64_at(a, *at).is_ok_and(|w| w != 0))
    {
        return Err(broken(
            what,
            "an interior sets an assetType or a model word",
        ));
    }
    // The arrays of `CMiloInterior` and the two at 0x110 and 0x120 are empty in every body measured.
    for at in [
        0x178, 0x188, 0x198, 0x1A8, 0x1B8, 0x1C8, 0x1D8, 0x110, 0x120,
    ] {
        if u16_at(a, at + 8)? != 0 {
            return Err(broken(
                what,
                format!("its array at {at:#X} is not empty, and its layout is not known"),
            ));
        }
    }
    let entities = interior_entities(blocks, a, 0xC0, &format!("{what}.entities"))?;
    let n = entities.len();
    let room_records =
        blocks.records(a, 0xD0, C_MLO_ROOM_DEF, ROOM_SIZE, &format!("{what}.rooms"))?;
    if room_records.is_empty() {
        return Err(broken(what, "an interior has no room"));
    }
    let portal_records = blocks.records(
        a,
        0xE0,
        C_MLO_PORTAL_DEF,
        PORTAL_SIZE,
        &format!("{what}.portals"),
    )?;
    let rooms_n = room_records.len();
    let mut touching = vec![0u32; rooms_n];
    let mut held = vec![0u32; n];
    let mut hold = |indices: &[u32], what: &str| -> Result<()> {
        for &i in indices {
            match held.get_mut(i as usize) {
                Some(count) => *count += 1,
                None => return Err(broken(what, format!("it attaches entity {i} of {n}"))),
            }
        }
        Ok(())
    };
    let mut portals = Vec::with_capacity(portal_records.len());
    for (j, p) in portal_records.into_iter().enumerate() {
        let what = format!("{what}.portals[{j}]");
        let (room_from, room_to) = (u32_at(p, 0x08)?, u32_at(p, 0x0C)?);
        if room_from as usize >= rooms_n || room_to as usize >= rooms_n {
            return Err(broken(
                what,
                format!("it joins room {room_from} and {room_to} of {rooms_n}"),
            ));
        }
        touching[room_from as usize] += 1;
        if room_to != room_from {
            touching[room_to as usize] += 1;
        }
        let corners = elements(blocks, p, 0x38, 0x10, &what)?;
        let corners: [&[u8]; 4] = corners
            .try_into()
            .map_err(|c: Vec<&[u8]>| broken(&what, format!("it has {} corners, not 4", c.len())))?;
        let mut at = [[0.0; 3]; 4];
        for (slot, c) in at.iter_mut().zip(corners) {
            *slot = floats(c, 0, &what)?;
        }
        let attached_objects = words(blocks, p, 0x48, &what)?;
        hold(&attached_objects, &what)?;
        portals.push(Portal {
            room_from,
            room_to,
            flags: u32_at(p, 0x10)?,
            corners: at,
            attached_objects,
        });
    }
    let mut rooms = Vec::with_capacity(rooms_n);
    for (j, r) in room_records.into_iter().enumerate() {
        let what = format!("{what}.rooms[{j}]");
        let portal_count = u32_at(r, 0x40)?;
        if portal_count != touching[j] {
            return Err(broken(
                what,
                format!(
                    "its portalCount {portal_count} is not the {} portals that touch it",
                    touching[j]
                ),
            ));
        }
        let attached_objects = words(blocks, r, 0x50, &what)?;
        hold(&attached_objects, &what)?;
        rooms.push(Room {
            name: u32_at(r, 0x00)?,
            bb_min: floats(r, 0x10, &what)?,
            bb_max: floats(r, 0x20, &what)?,
            flags: u32_at(r, 0x3C)?,
            portal_count,
            floor_id: i32::from_le_bytes(field(r, 0x44)?),
            attached_objects,
        });
    }
    if let Some(i) = held.iter().position(|c| *c != 1) {
        return Err(broken(
            what,
            format!(
                "its rooms and portals attach entity {i} {} times, not once",
                held[i]
            ),
        ));
    }
    let set_records = blocks.records(
        a,
        0xF0,
        C_MLO_ENTITY_SET,
        ENTITY_SET_SIZE,
        &format!("{what}.entitySets"),
    )?;
    let mut entity_sets = Vec::with_capacity(set_records.len());
    for (j, s) in set_records.into_iter().enumerate() {
        let what = format!("{what}.entitySets[{j}]");
        let locations = words(blocks, s, 0x08, &what)?;
        let set_entities = interior_entities(blocks, s, 0x18, &what)?;
        if locations.len() != set_entities.len() {
            return Err(broken(
                what,
                format!(
                    "it has {} locations for {} entities",
                    locations.len(),
                    set_entities.len()
                ),
            ));
        }
        let entities = (locations.into_iter().zip(set_entities))
            .map(|(l, e)| match l & PORTAL_BIT {
                0 if (l as usize) < rooms_n => Ok((EntitySetLocation::Room(l), e)),
                PORTAL_BIT if ((l & !PORTAL_BIT) as usize) < portals.len() => {
                    Ok((EntitySetLocation::Portal(l & !PORTAL_BIT), e))
                }
                _ => Err(broken(
                    &what,
                    format!(
                        "its location {l:#X} is past {rooms_n} rooms and {} portals",
                        portals.len()
                    ),
                )),
            })
            .collect::<Result<Vec<_>>>()?;
        entity_sets.push(EntitySet {
            name: u32_at(s, 0x00)?,
            entities,
        });
    }
    let time_cycle_modifiers = (blocks
        .records(
            a,
            0x100,
            C_MLO_TIME_CYCLE_MODIFIER,
            TIME_CYCLE_MODIFIER_SIZE,
            &format!("{what}.timeCycleModifiers"),
        )?
        .into_iter())
    .map(|t| {
        Ok(TimeCycleModifier {
            name: u32_at(t, 0x00)?,
            sphere: floats(t, 0x10, what)?,
            percentage: f32_at(t, 0x20, what)?,
            range: f32_at(t, 0x24, what)?,
            start_hour: u32_at(t, 0x28)?,
            end_hour: u32_at(t, 0x2C)?,
        })
    })
    .collect::<Result<Vec<_>>>()?;
    Ok(Interior {
        archetype,
        name: u32_at(a, 0x78)?,
        name_word: u64_at(a, 0x08)?,
        entities,
        rooms,
        portals,
        entity_sets,
        time_cycle_modifiers,
    })
}

#[cfg(test)]
#[path = "map_links_tests.rs"]
mod tests;
