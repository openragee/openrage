//! GTA VI's anim scenes: a headerless RBM body whose root is one scene (`hash_1EBDDB5B`) or a slot
//! map of scenes (`hash_233FA41B`), typed from the value tree the table-form reader decodes.
//!
//! ⚠ The scene class is checked against the body's own class table first: a body that states
//! another layout is refused here and stays the generic tree. An event whose start is after its
//! end is kept as stated.

use crate::hash::literal_string_hash as lit;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::pso::table_form;
use crate::meta::binary::rbm::schema::{RbmClass, RbmModel};
use crate::meta::{MetaInstance, Value};

/// The root class of a body that holds one scene.
pub const SCENE: u32 = 0x1EBD_DB5B;
/// The root class of a body that holds a slot map of scenes.
pub const SCENE_MAP: u32 = 0x233F_A41B;
/// The event list a scene and each entity hold: a counted array of event pointers at 0x08.
const EVENT_LIST: u32 = 0xEED7_E46F;
/// The entity an event acts on: `entityId` at 0x08.
const ENTITY_REF: u32 = 0xED16_C5B0;
/// The clip an event plays: `clipDictionaryName` at 0x08, `clipName` at 0x0C.
const CLIP_REF: u32 = 0x2474_9343;
/// The layered play-anim event.
const LAYERED_PLAY: u32 = 0xF41A_59B9;

const ARRAY_INFO: u32 = 0x100;
const COUNTED_ARRAY: u8 = 0x5C;
const STRING_HASH: u8 = 0x4A;

const ENTITIES: u32 = 0xCCAB_9307;
const EVENTS: u32 = 0xDB01_2F3B;
const SCENES: u32 = 0x19D5_86EA;

/// Every member of the 0x160-byte scene class but its array descriptors, in order: name hash,
/// offset and type code.
const SCENE_MEMBERS: [(u32, u32, u8); 21] = [
    (0x314A_4051, 0x08, 0x07),
    (0xA176_F880, 0x10, 0x05),
    (0x1178_C0B5, 0x60, 0x07),
    (0x6C60_E551, 0x68, 0x07),
    (0x76E0_3F38, 0x70, 0x07),
    (0xDB66_E9D1, 0x78, 0x5C),
    (ENTITIES, 0x88, 0x75),
    (0x32C6_9AAE, 0x98, 0x75),
    (0xA60B_9780, 0xA8, 0x75),
    (EVENTS, 0xB8, 0x05),
    (0x29C2_5EC9, 0xD0, 0x52),
    (0xE6CA_E80A, 0xE0, 0x05),
    (0x3D51_ABF0, 0x130, 0x75),
    (0x59C7_C8EE, 0x140, 0x05),
    (0x68C2_7E33, 0x148, 0x15),
    (0x9079_1387, 0x14C, 0x14),
    (0x34CC_92BF, 0x150, 0x4A),
    (0x8FB2_F5A4, 0x154, 0x4A),
    (0x67D2_6872, 0x158, 0x65),
    (0x0558_B627, 0x15C, 0x01),
    (0x8A98_6F09, 0x15D, 0x01),
];
const SCENE_MAP_MEMBERS: [(u32, u32, u8); 1] = [(SCENES, 0x00, 0x73)];
/// The one member at 0x60 of [`LAYERED_PLAY`]: `hash_82FEB24D`, a structure `hash_82336229`.
const LAYERED_PLAY_60: (u32, u8, u32) = (0x82FE_B24D, 0x05, 0x8233_6229);

/// Why an anim-scene body is not read.
#[derive(Debug, thiserror::Error)]
pub enum AnimSceneError {
    #[error(transparent)]
    Meta(#[from] crate::meta::MetaError),
    #[error("the root structure {0:#010X} is neither one scene (hash_1EBDDB5B) nor a map of scenes (hash_233FA41B)")]
    NotAScene(u32),
    #[error("{0}")]
    Layout(&'static str),
    #[error("{what}: {why}")]
    Broken { what: String, why: String },
}

type Result<T> = std::result::Result<T, AnimSceneError>;

fn broken(what: impl Into<String>, why: impl Into<String>) -> AnimSceneError {
    AnimSceneError::Broken {
        what: what.into(),
        why: why.into(),
    }
}

/// What an anim-scene body holds: one scene, or the scenes of a map with their keys.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimSceneBody {
    /// Whether the root is a map of scenes rather than one scene.
    pub map: bool,
    pub scenes: Vec<Scene>,
}

/// A scene: its own events, and its entities with the events each carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The scene's key in its map; `None` for a body of one scene.
    pub key: Option<String>,
    pub events: Vec<Event>,
    pub entities: Vec<Entity>,
}

impl Scene {
    /// The scene's own events, then each entity's in turn.
    pub fn all_events(&self) -> impl Iterator<Item = &Event> {
        (self.events.iter()).chain(self.entities.iter().flat_map(|e| &e.events))
    }
}

/// An entity of a scene: its key in the entity map, its class and its own events.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub key: String,
    pub class: String,
    pub events: Vec<Event>,
}

/// An event as its body states it, a start after its end included.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub class: String,
    /// `startTime`, in seconds.
    pub start: Option<f32>,
    /// `endTime`, in seconds.
    pub end: Option<f32>,
    /// `preloadTime`, whose unit is not known.
    pub preload: Option<f32>,
    /// The `entityId` of the entity the event acts on.
    pub entity: Option<String>,
    pub clip: Option<Clip>,
}

impl Event {
    /// The start and end as stated, where both are present and finite.
    pub fn span(&self) -> Option<(f32, f32)> {
        let (start, end) = (self.start?, self.end?);
        (start.is_finite() && end.is_finite()).then_some((start, end))
    }
}

/// The clip an event plays, by its dictionary's name and its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub dictionary: String,
    pub name: String,
}

/// Whether a headerless RBM virtual section's root block is of an anim-scene class.
pub fn is_anim_scene(virtual_section: &[u8]) -> bool {
    matches!(
        super::map::root_type(virtual_section),
        Some(SCENE | SCENE_MAP)
    )
}

/// The anim scene a headerless RBM resource holds, from its two inflated sections.
pub fn open(virtual_section: &[u8], physical: &[u8]) -> Result<AnimSceneBody> {
    read(&crate::meta::binary::rbm::gta6::read_model(
        virtual_section,
        physical,
    )?)
}

/// The anim scene of a headerless RBM resource's class table and data blocks.
pub fn read(model: &RbmModel) -> Result<AnimSceneBody> {
    if scene_root(model).is_none() {
        return Err(AnimSceneError::NotAScene(root_class(model)));
    }
    read_tree(model, &table_form::decode(model)?)
}

/// The root block's class, 0 where the root names no block.
fn root_class(model: &RbmModel) -> u32 {
    (model.block(model.root_block_id as i32)).map_or(0, |b| b.data_type)
}

/// The root block's class where it is one scene or a map of scenes.
pub fn scene_root(model: &RbmModel) -> Option<u32> {
    Some(root_class(model)).filter(|c| [SCENE, SCENE_MAP].contains(c))
}

/// The anim scene `tree` states, the value tree the table-form reader decodes of `model`.
pub fn read_tree(model: &RbmModel, tree: &MetaInstance) -> Result<AnimSceneBody> {
    let _names = crate::meta::binary::hash_index::title_names();
    let root = root_class(model);
    let map = match root {
        SCENE => false,
        SCENE_MAP => true,
        other => return Err(AnimSceneError::NotAScene(other)),
    };
    check_layouts(model, map)?;
    let names = Names::of(model)?;
    if tree.type_name != default_hash_index().name_or_literal(root) {
        return Err(broken(
            "the root",
            "the value tree is not of its block's class",
        ));
    }
    let scenes = match map {
        false => vec![scene(&names, None, tree)?],
        true => scenes_of_map(&names, tree)?,
    };
    Ok(AnimSceneBody { map, scenes })
}

fn named(class: &RbmClass) -> impl Iterator<Item = (u32, u32, u8)> + '_ {
    (class.entries.iter())
        .filter(|e| e.name_hash != ARRAY_INFO)
        .map(|e| (e.name_hash, e.offset, e.data_type.to_byte()))
}

fn check_layouts(model: &RbmModel, map: bool) -> Result<()> {
    let fits = |hash: u32, size: u32, members: &[(u32, u32, u8)]| {
        (model.classes.get(&hash))
            .is_some_and(|c| c.data_size == size && named(c).eq(members.iter().copied()))
    };
    if !fits(SCENE, 0x160, &SCENE_MEMBERS[..]) {
        return Err(AnimSceneError::Layout(
            "the scene class hash_1EBDDB5B is not the 0x160-byte layout every measured body states",
        ));
    }
    if map && !fits(SCENE_MAP, 0x10, &SCENE_MAP_MEMBERS[..]) {
        return Err(AnimSceneError::Layout(
            "the scene map class hash_233FA41B is not the slot map of scenes at 0 every measured body states",
        ));
    }
    if let Some(class) = model.classes.get(&LAYERED_PLAY) {
        let at_60: Vec<(u32, u8, u32)> = (class.entries.iter())
            .filter(|e| e.offset == 0x60 && e.name_hash != ARRAY_INFO)
            .map(|e| (e.name_hash, e.data_type.to_byte(), e.referent_hash()))
            .collect();
        if at_60 != [LAYERED_PLAY_60] {
            return Err(AnimSceneError::Layout(
                "the member at 0x60 of hash_F41A59B9 is not hash_82FEB24D, the structure hash_82336229 every measured body states",
            ));
        }
    }
    Ok(())
}

/// The name of each member and class this reader follows, as the value tree names it: the name
/// the hash index gives its hash.
struct Names {
    scene: String,
    entities: String,
    events: String,
    scenes: String,
    list_class: String,
    list: String,
    entity_ref: String,
    entity_id: Option<String>,
    clip_ref: String,
    clip_dictionary: Option<String>,
    clip_name: Option<String>,
    start: String,
    end: String,
    preload: String,
}

impl Names {
    fn of(model: &RbmModel) -> Result<Self> {
        let name = |hash: u32| default_hash_index().name_or_literal(hash);
        let at = |class: u32, offset: u32, code: u8| {
            let class = model.classes.get(&class)?;
            (class.entries.iter())
                .find(|e| {
                    e.name_hash != ARRAY_INFO && e.offset == offset && e.data_type.to_byte() == code
                })
                .map(|e| name(e.name_hash))
        };
        let list = at(EVENT_LIST, 0x08, COUNTED_ARRAY).ok_or(AnimSceneError::Layout(
            "the event list hash_EED7E46F has no counted array at 0x08",
        ))?;
        Ok(Self {
            scene: name(SCENE),
            entities: name(ENTITIES),
            events: name(EVENTS),
            scenes: name(SCENES),
            list_class: name(EVENT_LIST),
            list,
            entity_ref: name(ENTITY_REF),
            entity_id: at(ENTITY_REF, 0x08, STRING_HASH),
            clip_ref: name(CLIP_REF),
            clip_dictionary: at(CLIP_REF, 0x08, STRING_HASH),
            clip_name: at(CLIP_REF, 0x0C, STRING_HASH),
            start: name(lit(b"startTime")),
            end: name(lit(b"endTime")),
            preload: name(lit(b"preloadTime")),
        })
    }
}

/// The text a hash value is shown by: the name it hashes, else `hash_XXXXXXXX`.
fn hash_text(value: &Value) -> Option<String> {
    match value {
        Value::Hash(h) if h.raw.is_empty() => Some(h.to_literal()),
        Value::Hash(h) => Some(h.raw.clone()),
        _ => None,
    }
}

fn scenes_of_map(n: &Names, map: &MetaInstance) -> Result<Vec<Scene>> {
    let Some(Value::Map(pairs)) = map.get(&n.scenes) else {
        return Err(broken("the scene map", "its scenes are not a map"));
    };
    (pairs.iter())
        .map(|(key, item)| match item {
            Value::Struct(s) if s.type_name == n.scene => scene(n, hash_text(key), s),
            _ => Err(broken(
                "the scene map",
                "an item of it is not a scene in place",
            )),
        })
        .collect()
}

fn scene(n: &Names, key: Option<String>, instance: &MetaInstance) -> Result<Scene> {
    let what = match &key {
        Some(key) => format!("scene {key}"),
        None => "the scene".to_string(),
    };
    let events = match instance.get(&n.events) {
        Some(Value::Struct(list)) => event_list(n, list, &what)?,
        _ => return Err(broken(what, "its events are not a structure")),
    };
    let entities = match instance.get(&n.entities) {
        Some(Value::Map(pairs)) => (pairs.iter())
            .map(|(key, item)| entity(n, key, item))
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(broken(what, "its entities are not a map")),
    };
    Ok(Scene {
        key,
        events,
        entities,
    })
}

fn entity(n: &Names, key: &Value, item: &Value) -> Result<Entity> {
    let key = hash_text(key).ok_or_else(|| broken("an entity", "its key is not a hash"))?;
    let what = format!("entity {key}");
    let object = match item {
        Value::Pointer(Some(object)) | Value::Struct(object) => object,
        _ => return Err(broken(what, "it names no object")),
    };
    let lists = object.fields.iter().filter_map(|(_, value)| match value {
        Value::Struct(list) if list.type_name == n.list_class => Some(list),
        _ => None,
    });
    let mut events = Vec::new();
    for list in lists {
        events.extend(event_list(n, list, &what)?);
    }
    Ok(Entity {
        key,
        class: object.type_name.clone(),
        events,
    })
}

fn event_list(n: &Names, list: &MetaInstance, what: &str) -> Result<Vec<Event>> {
    let Some(Value::Array(items)) = list.get(&n.list) else {
        return Err(broken(what, "its event list is not an array"));
    };
    (items.iter())
        .map(|item| match item {
            Value::Pointer(Some(event)) => Ok(read_event(n, event)),
            _ => Err(broken(what, "an element of its event list names no event")),
        })
        .collect()
}

fn read_event(n: &Names, event: &MetaInstance) -> Event {
    let float = |name: &str| match event.get(name) {
        Some(Value::Float(v)) => Some(*v as f32),
        _ => None,
    };
    let text =
        |s: &MetaInstance, name: Option<&str>| name.and_then(|f| s.get(f)).and_then(hash_text);
    let (mut entity, mut clip) = (None, None);
    for (_, value) in &event.fields {
        let Value::Struct(s) = value else {
            continue;
        };
        let s: &MetaInstance = s;
        if s.type_name == n.entity_ref && entity.is_none() {
            entity = text(s, n.entity_id.as_deref());
        } else if s.type_name == n.clip_ref && clip.is_none() {
            clip = (text(s, n.clip_dictionary.as_deref()))
                .zip(text(s, n.clip_name.as_deref()))
                .map(|(dictionary, name)| Clip { dictionary, name });
        }
    }
    Event {
        class: event.type_name.clone(),
        start: float(n.start.as_str()),
        end: float(n.end.as_str()),
        preload: float(n.preload.as_str()),
        entity,
        clip,
    }
}

#[cfg(test)]
#[path = "anim_scene_tests.rs"]
mod tests;
