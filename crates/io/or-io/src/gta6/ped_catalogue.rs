//! GTA VI's ped catalogues: a headerless RBM body whose root (`hash_8DCB4CF5`) holds two lists of
//! kits, typed from the value tree the table-form reader decodes.
//!
//! A kit of the first list states the assets of a part of a ped, a state at a time: a drawable,
//! its albedo, normal and material maps, each a 64-bit name word. A kit of the second list names
//! kits and a state of each. ⚠ A member is found by its name hash in the body's own table, never
//! by an offset, and the members this reader does not follow stay in the generic tree.

use crate::hash::literal_string_hash as lit;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::pso::table_form;
use crate::meta::{MetaInstance, Value};

/// The root class of a ped catalogue.
pub const CATALOGUE: u32 = 0x8DCB_4CF5;
/// The root's list of asset kit lists.
const ASSET_LISTS: u32 = 0x0526_9A03;
/// The root's list of set kit lists.
const SET_LISTS: u32 = 0x9BCF_5F90;
/// A kit's own name hash, in both kinds of kit and in a set kit's member.
const KIT_ID: u32 = 0x6512_1749;
/// A state's name hash, in an asset kit's state and in a set kit's member.
const STATE_NAME: u32 = 0x3EAD_DCD7;
/// The structure of a state that holds its four assets.
const STATE_ASSETS: u32 = 0xE7CB_9E1B;
/// The map of a state's assets beyond the four.
const MORE_ASSETS: u32 = 0xB70A_22C1;
/// A set kit's members.
const SET_MEMBERS: u32 = 0xFB5B_B20D;

/// Why a ped catalogue is not read.
#[derive(Debug, thiserror::Error)]
pub enum PedCatalogueError {
    #[error(transparent)]
    Meta(#[from] crate::meta::MetaError),
    #[error("the root structure {0:#010X} is no ped catalogue (hash_8DCB4CF5)")]
    NotACatalogue(u32),
    #[error("{what}: {why}")]
    Broken { what: String, why: &'static str },
}

type Result<T> = std::result::Result<T, PedCatalogueError>;

fn broken<T>(what: impl Into<String>, why: &'static str) -> Result<T> {
    Err(PedCatalogueError::Broken {
        what: what.into(),
        why,
    })
}

/// A ped catalogue: its lists of asset kits and of set kits, as the body orders them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PedCatalogue {
    pub asset_lists: Vec<Vec<AssetKit>>,
    pub set_lists: Vec<Vec<SetKit>>,
}

/// A kit that states the assets of one part, a state at a time.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetKit {
    /// The kit's own name hash (`hash_65121749`).
    pub id: u32,
    pub template_name: u32,
    pub category: u32,
    pub states: Vec<AssetState>,
}

/// One state of an asset kit: the 64-bit name words of its drawable and its three maps.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetState {
    /// The state's name hash (`hash_3EADDCD7`).
    pub name: u32,
    pub drawable: u64,
    pub albedo: u64,
    pub normal: u64,
    pub material: u64,
    /// The map of further assets (`hash_B70A22C1`), in the body's order: each key with its
    /// `AssetName` word, 0 for an asset that states none (one of four words).
    pub more: Vec<(u32, u64)>,
}

impl AssetState {
    /// The `AssetName` word of the further asset of key `key`, where the state has one.
    pub fn more_asset(&self, key: u32) -> Option<u64> {
        (self.more.iter()).find_map(|(k, word)| (*k == key && *word != 0).then_some(*word))
    }
}

/// A kit that names asset kits and a state of each.
#[derive(Debug, Clone, PartialEq)]
pub struct SetKit {
    pub id: u32,
    pub template_name: u32,
    pub scale: f32,
    pub members: Vec<SetMember>,
}

/// A member of a set kit: a kit's name hash and a state's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetMember {
    pub kit: u32,
    pub state: u32,
}

impl PedCatalogue {
    pub fn asset_kits(&self) -> impl Iterator<Item = &AssetKit> {
        self.asset_lists.iter().flatten()
    }

    pub fn set_kits(&self) -> impl Iterator<Item = &SetKit> {
        self.set_lists.iter().flatten()
    }

    /// How many members of its set kits name an asset kit of this catalogue and a state of it.
    pub fn members_resolved(&self) -> usize {
        let states: std::collections::HashSet<(u32, u32)> = (self.asset_kits())
            .flat_map(|kit| kit.states.iter().map(|s| (kit.id, s.name)))
            .collect();
        (self.set_kits().flat_map(|kit| &kit.members))
            .filter(|m| states.contains(&(m.kit, m.state)))
            .count()
    }
}

/// Whether a headerless RBM virtual section's root block is a ped catalogue's.
pub fn is_ped_catalogue(virtual_section: &[u8]) -> bool {
    super::map::root_type(virtual_section) == Some(CATALOGUE)
}

/// The ped catalogue a headerless RBM resource holds, from its two inflated sections.
pub fn open(virtual_section: &[u8], physical: &[u8]) -> Result<PedCatalogue> {
    let root = super::map::root_type(virtual_section).unwrap_or(0);
    if root != CATALOGUE {
        return Err(PedCatalogueError::NotACatalogue(root));
    }
    let model = crate::meta::binary::rbm::gta6::read_model(virtual_section, physical)?;
    read_tree(&table_form::decode(&model)?)
}

/// The members of `instance` this reader follows, by the name the value tree gives each hash.
struct Members<'a>(&'a MetaInstance);

impl<'a> Members<'a> {
    fn get(&self, hash: u32) -> Option<&'a Value> {
        self.0.get(&default_hash_index().name_or_literal(hash))
    }

    /// A hash string member's hash; 0 where the body leaves it empty.
    fn hash(&self, hash: u32) -> u32 {
        match self.get(hash) {
            Some(Value::Hash(h)) => h.hash,
            _ => 0,
        }
    }

    /// A 64-bit name word; 0 where the body leaves it empty.
    fn word(&self, hash: u32) -> u64 {
        match self.get(hash) {
            Some(Value::UInt(word)) => *word,
            _ => 0,
        }
    }

    /// The structures of an array member, which a body must state.
    fn structures(&self, hash: u32, what: &str) -> Result<Vec<Members<'a>>> {
        let Some(Value::Array(items)) = self.get(hash) else {
            return broken(what, "is not an array the body states");
        };
        (items.iter())
            .map(|item| match item {
                Value::Struct(s) => Ok(Members(s)),
                _ => broken(what, "holds an item that is no structure in place"),
            })
            .collect()
    }
}

/// The ped catalogue `tree` states, the value tree the table-form reader decodes of its body.
pub fn read_tree(tree: &MetaInstance) -> Result<PedCatalogue> {
    let _names = crate::meta::binary::hash_index::title_names();
    if tree.type_name != default_hash_index().name_or_literal(CATALOGUE) {
        return broken("the root", "the value tree is not of the catalogue's class");
    }
    let root = Members(tree);
    let kits = |lists: u32, what: &str| -> Result<Vec<Vec<Members>>> {
        (root.structures(lists, what)?.iter())
            .map(|list| list.structures(lit(b"Kits"), what))
            .collect()
    };
    let asset_lists = (kits(ASSET_LISTS, "the asset kit lists")?.iter())
        .map(|list| list.iter().map(asset_kit).collect())
        .collect::<Result<_>>()?;
    let set_lists = (kits(SET_LISTS, "the set kit lists")?.iter())
        .map(|list| list.iter().map(set_kit).collect())
        .collect::<Result<_>>()?;
    Ok(PedCatalogue {
        asset_lists,
        set_lists,
    })
}

fn asset_kit(kit: &Members) -> Result<AssetKit> {
    let states = (kit
        .structures(lit(b"States"), "an asset kit's states")?
        .iter())
    .map(|state| {
        let Some(Value::Struct(assets)) = state.get(STATE_ASSETS) else {
            return broken("an asset kit's state", "has no structure of assets");
        };
        let assets = Members(assets);
        Ok(AssetState {
            name: state.hash(STATE_NAME),
            drawable: assets.word(lit(b"Drawable")),
            albedo: assets.word(lit(b"Albedo")),
            normal: assets.word(lit(b"Normal")),
            material: assets.word(lit(b"Material")),
            more: match assets.get(MORE_ASSETS) {
                Some(Value::Map(pairs)) => pairs.iter().map(further_asset).collect(),
                _ => Vec::new(),
            },
        })
    })
    .collect::<Result<_>>()?;
    Ok(AssetKit {
        id: kit.hash(KIT_ID),
        template_name: kit.hash(lit(b"TemplateName")),
        category: kit.hash(lit(b"Category")),
        states,
    })
}

/// A pair of a state's map of further assets: its key, and the `AssetName` word its structure
/// states (0 where it states none).
fn further_asset((key, value): &(Value, Value)) -> (u32, u64) {
    let key = match key {
        Value::Hash(h) => h.hash,
        Value::UInt(word) => *word as u32,
        _ => 0,
    };
    let word = match value {
        Value::Struct(asset) | Value::Pointer(Some(asset)) => {
            Members(asset).word(lit(b"AssetName"))
        }
        _ => 0,
    };
    (key, word)
}

fn set_kit(kit: &Members) -> Result<SetKit> {
    let members = (kit.structures(SET_MEMBERS, "a set kit's members")?.iter())
        .map(|member| SetMember {
            kit: member.hash(KIT_ID),
            state: member.hash(STATE_NAME),
        })
        .collect();
    Ok(SetKit {
        id: kit.hash(KIT_ID),
        template_name: kit.hash(lit(b"TemplateName")),
        scale: match kit.get(lit(b"Scale")) {
            Some(Value::Float(scale)) => *scale as f32,
            _ => 0.0,
        },
        members,
    })
}

#[cfg(test)]
#[path = "ped_catalogue_tests.rs"]
mod tests;
