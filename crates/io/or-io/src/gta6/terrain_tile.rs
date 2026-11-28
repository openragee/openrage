//! A GTA VI terrain tile's two metadata bodies: its bounds, cell by cell, and its material, the
//! textures of its slots and its ground layers. A tile is 512 m, eight by eight cells of one
//! drawable each; both bodies sit in the `<group>_terrain_images_merged` archive beside the tiles.

use crate::gta6::hash::name_hash64;
use crate::hash::literal_string_hash as lit;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::pso::table_form;
use crate::meta::{MetaInstance, Value};

/// The root structure of a tile's bounds body.
pub const TILE_BOUNDS: u32 = 0x40FE_4E4C;
/// The root structure of a tile's material body.
pub const TILE_MATERIAL: u32 = 0xB518_20FF;
/// The cells of a tile, eight by eight.
pub const CELLS: usize = 64;
/// The texture slots a material body states, by key.
pub const SLOTS: usize = 12;
/// The floats of a layer the body states: the first 112 bytes of the record a stage reads.
pub const LAYER_PARAMS: usize = 28;

const NAME: u32 = lit(b"name");
const MIN: u32 = lit(b"min");
const MAX: u32 = lit(b"max");
const PLACE: u32 = 0x697A_9113;
const CORNER: u32 = 0x2C67_1A82;
const PLACE_SECOND: u32 = 0xED8E_650A;
const BOX: u32 = 0x144D_A862;
const CELL_BOXES: u32 = 0xF03E_12ED;
const SLOT_NAMES: u32 = 0x34FB_D46F;
const SLOT_KEYS: u32 = 0x649E_DCF4;
const LAYERS: u32 = 0x9A59_341E;
const LAYER_TEXTURES: u32 = 0x0354_738B;
const LAYER_MASKS: u32 = 0xAE9D_D8AD;
const MASK: u32 = 0x3E1D_2F15;
/// A layer's members in the order of its record, each with the floats it holds.
const LAYER_MEMBERS: [(u32, usize); 16] = [
    (0xD22F_51D4, 4),
    (0x33E8_4D06, 1),
    (0xC1E3_F9FA, 1),
    (0x7696_770C, 1),
    (lit(b"bumpiness"), 1),
    (0xE332_AFA5, 1),
    (0x804D_042A, 1),
    (0xFC90_BEFA, 1),
    (lit(b"translucency"), 1),
    (0x2EAA_58A3, 1),
    (0x16A9_ECDF, 1),
    (0xB5FE_3ACB, 1),
    (0x61EA_57C4, 1),
    (0xE9A0_4754, 4),
    (0x65AD_666E, 4),
    (0x57EF_4AF2, 4),
];
/// A layer's three textures: names, then keys.
const TEXTURE_NAMES: [u32; 3] = [0x47EC_DD52, 0x87D0_8271, 0x10F4_1699];
const TEXTURE_KEYS: [u32; 3] = [0x1ED3_BE1A, 0x6B3D_24EE, 0x1966_405B];

/// Why a tile's body does not read.
#[derive(Debug, thiserror::Error)]
pub enum TerrainTileError {
    #[error(transparent)]
    Meta(#[from] crate::meta::MetaError),
    #[error("the body's root is 0x{found:08X}, not the tile's 0x{wanted:08X}")]
    NotTheTile { found: u32, wanted: u32 },
    #[error("the tile's {0} is missing or not of its stated shape")]
    Broken(&'static str),
}

/// A box, its two corners.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// A tile's bounds body.
#[derive(Debug, Clone, PartialEq)]
pub struct TileBounds {
    /// The tile's name, which the drawables of its cells start with: `n04_w19`.
    pub name: String,
    /// The tile's south-west corner and lowest height, from which the stages place a pixel.
    pub corner: [f32; 3],
    /// The second vector of that structure, whose purpose is not read.
    pub place_second: [f32; 3],
    /// The tile's whole box.
    pub bounds: Bounds,
    /// The box of each cell, from the north-west, west to east.
    pub cells: Vec<Bounds>,
}

/// The three textures one layer names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayerTextures {
    /// The 32-bit hashes of their names: albedo, normal, height and parameters.
    pub names: [u32; 3],
    /// Their texture keys, in the same order.
    pub keys: [u64; 3],
}

/// A tile's material body.
#[derive(Debug, Clone, PartialEq)]
pub struct TileMaterial {
    /// The 32-bit hash of each slot's texture name; nought for a slot with none.
    pub slot_names: [u32; SLOTS],
    /// Each slot's texture key; nought for a slot with none.
    pub slot_keys: [u64; SLOTS],
    /// Each layer's floats, in the order of the record a stage reads.
    pub layers: Vec<[f32; LAYER_PARAMS]>,
    /// Each layer's textures.
    pub textures: Vec<LayerTextures>,
    /// Each layer's cells: bit `i` set where cell `i` uses the layer.
    pub masks: Vec<u64>,
}

fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn object(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(s) | Value::Pointer(Some(s)) => Some(s),
        _ => None,
    }
}

// ⚠ The decoder and the hash index can name one member differently (`bumpiness` beside
// `hash_F6712B81`) once a names snapshot is installed, so a member is matched by its hash too.
fn member(of: &MetaInstance, hash: u32) -> Option<&Value> {
    let hash_of = |n: &str| match crate::meta::binary::hash_index::spelled_hash(n) {
        Some(spelled) => spelled == hash,
        None => lit(n.as_bytes()) == hash || crate::hash::string_hash(n.as_bytes()) == hash,
    };
    let by_hash = || of.fields.iter().find(|(n, _)| hash_of(n)).map(|(_, v)| v);
    of.get(&name(hash)).or_else(by_hash)
}

fn lanes(value: &Value) -> Option<[f32; 4]> {
    Some(match *value {
        Value::Vec2([x, y]) => [x, y, 0.0, 0.0],
        Value::Vec3([x, y, z]) => [x, y, z, 0.0],
        Value::Vec4(v) => v,
        Value::Float(v) => [v as f32, 0.0, 0.0, 0.0],
        _ => return None,
    })
}

fn vec3(value: &Value) -> Option<[f32; 3]> {
    let [x, y, z, _] = lanes(value)?;
    Some([x, y, z])
}

/// A number or a hash as a value states it; an empty value is nought.
fn word(value: &Value) -> Option<u64> {
    Some(match value {
        Value::UInt(v) => *v,
        Value::Int(v) => *v as u64,
        Value::Hash(h) => u64::from(h.hash),
        Value::Null => 0,
        Value::String(s) if s.is_empty() => 0,
        _ => return None,
    })
}

fn array<'a>(
    of: &'a MetaInstance,
    hash: u32,
    what: &'static str,
) -> Result<&'a [Value], TerrainTileError> {
    match member(of, hash) {
        Some(Value::Array(items)) => Ok(items),
        _ => Err(TerrainTileError::Broken(what)),
    }
}

fn bounds(of: &MetaInstance) -> Option<Bounds> {
    Some(Bounds {
        min: vec3(member(of, MIN)?)?,
        max: vec3(member(of, MAX)?)?,
    })
}

/// The root of the table-form document `bytes`, held to be `wanted`.
fn root_of(bytes: &[u8], wanted: u32) -> Result<MetaInstance, TerrainTileError> {
    let model = table_form::model_of(bytes)?;
    let found = (model.block(model.root_block_id as i32)).map_or(0, |b| b.data_type);
    if found != wanted {
        return Err(TerrainTileError::NotTheTile { found, wanted });
    }
    Ok(table_form::decode(&model)?)
}

impl TileBounds {
    /// The bounds of the document `bytes`.
    pub fn open(bytes: &[u8]) -> Result<Self, TerrainTileError> {
        Self::from_tree(&root_of(bytes, TILE_BOUNDS)?)
    }

    /// The bounds a decoded document states.
    pub fn from_tree(root: &MetaInstance) -> Result<Self, TerrainTileError> {
        let Some(Value::String(tile)) = member(root, NAME) else {
            return Err(TerrainTileError::Broken("name"));
        };
        let place = member(root, PLACE).and_then(object);
        let corner = (place.and_then(|p| member(p, CORNER)).and_then(vec3))
            .ok_or(TerrainTileError::Broken("corner"))?;
        let place_second = (place.and_then(|p| member(p, PLACE_SECOND)).and_then(vec3))
            .ok_or(TerrainTileError::Broken("corner"))?;
        let whole = (member(root, BOX).and_then(object).and_then(bounds))
            .ok_or(TerrainTileError::Broken("box"))?;
        let cells = (array(root, CELL_BOXES, "cells")?.iter())
            .map(|cell| object(cell).and_then(bounds))
            .collect::<Option<Vec<_>>>()
            .filter(|cells| cells.len() == CELLS)
            .ok_or(TerrainTileError::Broken("cells"))?;
        Ok(Self {
            name: tile.clone(),
            corner,
            place_second,
            bounds: whole,
            cells,
        })
    }
}

impl TileMaterial {
    /// The material of the document `bytes`.
    pub fn open(bytes: &[u8]) -> Result<Self, TerrainTileError> {
        Self::from_tree(&root_of(bytes, TILE_MATERIAL)?)
    }

    /// The material a decoded document states.
    pub fn from_tree(root: &MetaInstance) -> Result<Self, TerrainTileError> {
        let slots = |hash, what| -> Result<[u64; SLOTS], TerrainTileError> {
            let words: Vec<u64> = (array(root, hash, what)?.iter())
                .map(word)
                .collect::<Option<_>>()
                .ok_or(TerrainTileError::Broken(what))?;
            words.try_into().map_err(|_| TerrainTileError::Broken(what))
        };
        let slot_names = slots(SLOT_NAMES, "slot names")?.map(|w| w as u32);
        let slot_keys = slots(SLOT_KEYS, "slot keys")?;
        let layers = (array(root, LAYERS, "layers")?.iter())
            .map(|layer| object(layer).and_then(layer_params))
            .collect::<Option<Vec<_>>>()
            .ok_or(TerrainTileError::Broken("layers"))?;
        let textures = (array(root, LAYER_TEXTURES, "layer textures")?.iter())
            .map(|layer| object(layer).and_then(layer_textures))
            .collect::<Option<Vec<_>>>()
            .ok_or(TerrainTileError::Broken("layer textures"))?;
        let masks = (array(root, LAYER_MASKS, "layer masks")?.iter())
            .map(|layer| object(layer).and_then(|m| member(m, MASK)).and_then(word))
            .collect::<Option<Vec<_>>>()
            .ok_or(TerrainTileError::Broken("layer masks"))?;
        if textures.len() != layers.len() || masks.len() != layers.len() {
            return Err(TerrainTileError::Broken("layer counts"));
        }
        Ok(Self {
            slot_names,
            slot_keys,
            layers,
            textures,
            masks,
        })
    }

    /// Whether the slot `slot`'s texture is the tile's `<tile>_tint`: how a material body is
    /// told to be a tile's, where its tint map (slot 7) is.
    // ponytail: the game finds the body by a hash of a name its archive does not hold; this
    // identity is measured on every material body of one group's archive.
    pub fn is_of(&self, tile: &str) -> bool {
        self.slot_keys[7] == name_hash64(format!("{tile}_tint").as_bytes())
    }
}

fn layer_params(of: &MetaInstance) -> Option<[f32; LAYER_PARAMS]> {
    let mut out = [0f32; LAYER_PARAMS];
    let mut at = 0;
    for (hash, floats) in LAYER_MEMBERS {
        let value = lanes(member(of, hash)?)?;
        out[at..at + floats].copy_from_slice(&value[..floats]);
        at += floats;
    }
    Some(out)
}

fn layer_textures(of: &MetaInstance) -> Option<LayerTextures> {
    let mut out = LayerTextures::default();
    for i in 0..3 {
        out.names[i] = word(member(of, TEXTURE_NAMES[i])?)? as u32;
        out.keys[i] = word(member(of, TEXTURE_KEYS[i])?)?;
    }
    Some(out)
}

/// The texture slots a tile's record numbers, the twelve its material states and two the game
/// names from the tile: the grass mask and a texture of its own.
pub const RECORD_SLOTS: usize = 14;
/// The four kinds of a tile's sub-tile textures, by the suffix of their names.
pub const KINDS: [&str; 4] = ["ab", "nm", "ma", "mb"];
/// The slot each kind stands in for where the tile has no sub-tiles.
pub const KIND_SLOTS: [usize; 4] = [0, 1, 2, 11];
/// The sub-tiles of a tile, four by four.
pub const SUB_TILES: usize = 16;

/// The key of the texture the tile `tile`'s record numbers in slot `slot`: the material's, but
/// slot 11 is `<tile>_lod_mb` and slot 12 `<tile>_grass_mask` whatever the material states, and
/// slot 13 names none.
pub fn slot_key(material: &TileMaterial, tile: &str, slot: usize) -> u64 {
    match slot {
        11 => name_hash64(format!("{tile}_lod_mb").as_bytes()),
        12 => name_hash64(format!("{tile}_grass_mask").as_bytes()),
        _ => material.slot_keys.get(slot).copied().unwrap_or(0),
    }
}

/// The key of sub-tile `sub_tile` (from 0) of kind `kind`: `<tile>_lod_<sub_tile + 1>_<kind>`.
pub fn sub_tile_key(tile: &str, sub_tile: usize, kind: usize) -> u64 {
    let kind = KINDS.get(kind).copied().unwrap_or_default();
    name_hash64(format!("{tile}_lod_{:03}_{kind}", sub_tile + 1).as_bytes())
}

/// The sub-tile cell `cell` (from 1) lies in: a sub-tile is two by two cells, counted as the cells
/// are, from the south-west.
pub fn sub_tile_of(cell: u32) -> usize {
    let at = cell.saturating_sub(1) as usize;
    let (column, row) = (at % 8, at / 8);
    column / 2 + 4 * (row / 2)
}

/// The tile a drawable of a cell belongs to, and the cell's number from 1, by the drawable's name:
/// `n04_w19_027_hd` is cell 27 of `n04_w19`. Cells are numbered from the south-west, west to east.
pub fn cell_of(drawable: &str) -> Option<(&str, u32)> {
    let stem = drawable.split('.').next()?;
    let stem = stem.strip_suffix("_hd")?;
    let (tile, number) = stem.rsplit_once('_')?;
    let cell: u32 = (number.len() == 3).then(|| number.parse().ok())??;
    (1..=CELLS as u32).contains(&cell).then_some((tile, cell))
}

/// The archive beside a tile's that holds its bounds and material bodies: `…/<group>/<tile>` has
/// `…/<group>/<group>_terrain_images_merged`.
pub fn images_beside(tile_archive: &str) -> Option<String> {
    let (folder, _) = tile_archive.rsplit_once('/')?;
    let group = folder.rsplit('/').next()?;
    Some(format!("{folder}/{group}_terrain_images_merged"))
}

#[cfg(test)]
#[path = "terrain_tile_tests.rs"]
mod tests;
