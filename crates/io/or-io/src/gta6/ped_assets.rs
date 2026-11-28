//! GTA VI's ped asset lists (`MetaPedAssetList`, a PSO body), typed from the value tree, and the
//! join from a ped part's drawable to the texture key of each name its component asks by.
//!
//! A catalogue's state names its drawable, albedo, normal and material by 64-bit words; the three
//! maps' words, and the words its further assets name, are keys of an asset list's entries, and
//! each texture entry maps 32-bit names to the keys of its textures.

use std::collections::BTreeMap;

use super::hash::{name_hash32, name_hash64};
use super::ped_catalogue::{AssetKit, AssetState, PedCatalogue};
use crate::meta::{MetaInstance, Value};

/// The class of a ped asset list.
pub const ASSET_LIST: u32 = 0x1EDE_E47E;
/// Its texture maps: albedos, normals, materials, and the entries of a part's wrinkle maps.
const ALBEDOS: u32 = 0x4EEB_5CCD;
const NORMALS: u32 = 0xEBC1_D7A8;
const MATERIALS: u32 = name_hash32(b"materials");
const WRINKLES: u32 = 0xFE2D_8293;
/// A texture entry's own word, and its map of names to texture keys.
const GUID: u32 = name_hash32(b"guid");
const NAMES: u32 = 0x5376_0AF2;

/// The names a ped component asks its textures by.
pub const MA: u32 = name_hash32(b"ma");
pub const MB: u32 = name_hash32(b"mb");
pub const MC: u32 = name_hash32(b"mc");
pub const WRINKLE: u32 = name_hash32(b"wrinkle");
pub const WRINKLE_MASK: u32 = name_hash32(b"wrinkleMask");
pub const WRINKLE_INDEX: u32 = name_hash32(b"wrinkleIndex");
pub const BLOOD_MASK: u32 = name_hash32(b"bloodMask");

/// The keys of a state's further assets the join follows: the wrinkle entry, the entry of the
/// wrinkle mask and index, and the blood mask's texture.
const WRINKLE_ASSET: u32 = 0x2633_887D;
const WRINKLE_MASK_ASSET: u32 = 0xE125_85D1;
const BLOOD_ASSET: u32 = 0x8585_8F5E;
/// The state the game falls back to.
pub const BASE: u32 = name_hash32(b"Base");

/// Why an asset list is not read.
#[derive(Debug, thiserror::Error)]
pub enum PedAssetsError {
    #[error("{0}: is not a map the body states")]
    NoMap(&'static str),
}

/// A texture entry of an asset list: its own word, and its names' texture keys in the body's order.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct TextureEntry {
    pub word: u64,
    pub names: Vec<(u32, u64)>,
}

/// An asset list's texture maps, each entry by its key.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PedAssetList {
    pub albedos: BTreeMap<u64, TextureEntry>,
    pub normals: BTreeMap<u64, TextureEntry>,
    pub materials: BTreeMap<u64, TextureEntry>,
    pub wrinkles: BTreeMap<u64, TextureEntry>,
}

/// Whether `name`, a member's name in a value tree, is the member of hash `hash`: spelt, or
/// written as its hash.
fn is_member(name: &str, hash: u32) -> bool {
    name == format!("hash_{hash:08X}")
        || name_hash32(name.as_bytes()) == hash
        || crate::hash::literal_string_hash(name.as_bytes()) == hash
}

fn member(of: &MetaInstance, hash: u32) -> Option<&Value> {
    (of.fields.iter()).find_map(|(name, value)| is_member(name, hash).then_some(value))
}

/// A word a value states: a 64-bit name word, a 32-bit hash widened.
fn word(value: &Value) -> Option<u64> {
    match value {
        Value::UInt(word) => Some(*word),
        Value::Int(word) => Some(*word as u64),
        Value::Hash(h) => Some(u64::from(h.hash)),
        _ => None,
    }
}

fn structure(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(s) | Value::Pointer(Some(s)) => Some(s),
        _ => None,
    }
}

/// The entries of the map member `hash` of `list`, each by its key; an entry that does not read
/// is left out.
fn entries(
    list: &MetaInstance,
    hash: u32,
    what: &'static str,
) -> Result<BTreeMap<u64, TextureEntry>, PedAssetsError> {
    let Some(Value::Map(pairs)) = member(list, hash) else {
        return Err(PedAssetsError::NoMap(what));
    };
    let entry = |(key, value): &(Value, Value)| {
        let (key, entry) = (word(key)?, structure(value)?);
        let names = match member(entry, NAMES) {
            Some(Value::Map(names)) => (names.iter())
                .filter_map(|(name, key)| Some((word(name)? as u32, word(key)?)))
                .collect(),
            _ => Vec::new(),
        };
        let own = member(entry, GUID).and_then(word).unwrap_or(key);
        Some((key, TextureEntry { word: own, names }))
    };
    Ok(pairs.iter().filter_map(entry).collect())
}

/// One of an asset list's texture maps.
#[derive(Debug, Clone, Copy)]
enum Map {
    Albedos,
    Normals,
    Materials,
    Wrinkles,
}

impl PedAssetList {
    fn map(&self, map: Map) -> &BTreeMap<u64, TextureEntry> {
        match map {
            Map::Albedos => &self.albedos,
            Map::Normals => &self.normals,
            Map::Materials => &self.materials,
            Map::Wrinkles => &self.wrinkles,
        }
    }
}

/// The entry of key `key` in the map `map` of the first of `lists` that holds one.
fn entry(lists: &[PedAssetList], map: Map, key: u64) -> Option<&TextureEntry> {
    lists.iter().find_map(|list| list.map(map).get(&key))
}

/// Whether a PSO body's bytes name the asset list's class and its map of albedos, as its schema does.
pub fn is_asset_list(body: &[u8]) -> bool {
    let holds =
        |hash: u32| (body.windows(4)).any(|w| w == hash.to_le_bytes() || w == hash.to_be_bytes());
    holds(ASSET_LIST) && holds(ALBEDOS)
}

/// The asset list `tree` states, the value tree of its body.
pub fn read_tree(tree: &MetaInstance) -> Result<PedAssetList, PedAssetsError> {
    Ok(PedAssetList {
        albedos: entries(tree, ALBEDOS, "the albedos")?,
        normals: entries(tree, NORMALS, "the normals")?,
        materials: entries(tree, MATERIALS, "the materials")?,
        wrinkles: entries(tree, WRINKLES, "the wrinkle entries")?,
    })
}

/// What a drawable joins to: the state that wears it and the key of each of its textures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PedTextures {
    /// The kit's and the state's name hashes.
    pub kit: u32,
    pub state: u32,
    /// How many sets of albedo, normal and material the states that wear the drawable state.
    pub sets: usize,
    /// The albedo's and the normal's keys: the state's own words.
    pub albedo: u64,
    pub normal: u64,
    /// Whether the state's albedo is an entry of a list: where it is not, what the game resolves
    /// the state to is not known.
    pub listed: bool,
    /// Whether the state, or a template it takes it from, names a wrinkle entry: the component
    /// then has a wrinkle map, whether or not a list read holds the entry.
    pub wrinkled: bool,
    /// Each other name with its texture key, the first entry that states a name giving it.
    pub named: Vec<(u32, u64)>,
}

impl PedTextures {
    /// The key of the texture the component asks for by `name`.
    pub fn key(&self, name: u32) -> Option<u64> {
        (self.named.iter()).find_map(|(n, key)| (*n == name).then_some(*key))
    }
}

/// The word a catalogue's state names the drawable `name` by: its stem's 64-bit hash.
pub fn drawable_word(name: &str) -> u64 {
    name_hash64(super::texture_names::stem(name).as_bytes())
}

/// The word of the further asset `key` of `state` of `kit`, else of the state of that name of
/// the kit `kit` takes its template from, and so on up.
fn further(
    catalogues: &[PedCatalogue],
    kit: &AssetKit,
    state: &AssetState,
    key: u32,
) -> Option<u64> {
    let kits = || catalogues.iter().flat_map(PedCatalogue::asset_kits);
    let mut at = (kit, state.more_asset(key));
    // A chain of templates no longer than the kits there are: one that loops ends.
    for _ in 0..kits().count() {
        if let (_, Some(word)) = at {
            return Some(word);
        }
        let template = kits().find(|k| k.id == at.0.template_name && k.id != 0)?;
        let named = template.states.iter().find(|s| s.name == state.name);
        at = (template, named.and_then(|s| s.more_asset(key)));
    }
    at.1
}

/// The state of `catalogues` that wears the drawable `drawable`, with its textures' keys from
/// `lists`; `None` where no state wears it.
// Of several states the one named `Base` is taken, the game's own fallback, else the first.
pub fn join(
    drawable: u64,
    catalogues: &[PedCatalogue],
    lists: &[PedAssetList],
) -> Option<PedTextures> {
    let worn: Vec<(&AssetKit, &AssetState)> =
        (catalogues.iter().flat_map(PedCatalogue::asset_kits))
            .flat_map(|kit| kit.states.iter().map(move |state| (kit, state)))
            .filter(|(_, state)| state.drawable == drawable)
            .collect();
    let picked = (worn.iter().find(|(_, s)| s.name == BASE)).or_else(|| worn.first());
    let &(kit, state) = picked?;
    let mut sets: Vec<(u64, u64, u64)> = (worn.iter())
        .map(|(_, s)| (s.albedo, s.normal, s.material))
        .collect();
    sets.sort_unstable();
    sets.dedup();
    let albedo = entry(lists, Map::Albedos, state.albedo);
    let asset = |key| further(catalogues, kit, state, key);
    // In the order the game appends their names: albedo, normal, material, then the two
    // further assets'.
    let entries = [
        albedo,
        entry(lists, Map::Normals, state.normal),
        entry(lists, Map::Materials, state.material),
        asset(WRINKLE_ASSET).and_then(|w| entry(lists, Map::Wrinkles, w)),
        asset(WRINKLE_MASK_ASSET).and_then(|w| entry(lists, Map::Wrinkles, w)),
    ];
    let mut named: Vec<(u32, u64)> = Vec::new();
    for (name, key) in entries.iter().flatten().flat_map(|e| &e.names) {
        if named.iter().all(|(had, _)| had != name) {
            named.push((*name, *key));
        }
    }
    if let Some(blood) = asset(BLOOD_ASSET) {
        named.retain(|(name, _)| *name != BLOOD_MASK);
        named.push((BLOOD_MASK, blood));
    }
    Some(PedTextures {
        kit: kit.id,
        state: state.name,
        sets: sets.len(),
        albedo: state.albedo,
        normal: state.normal,
        listed: albedo.is_some(),
        wrinkled: asset(WRINKLE_ASSET).is_some(),
        named,
    })
}

#[cfg(test)]
#[path = "ped_assets_tests.rs"]
mod tests;
