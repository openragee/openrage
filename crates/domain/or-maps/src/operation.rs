//! The vocabulary of world edits: each thing [`crate::edit`] does to the sources of a pack of maps, as a named operation that is data.
//!
//! An [`Operation`] names a source of the pack and what to do to the file it cooks to. [`apply`] does it and returns the operation that
//! takes it back, so the step Undo moves along, a line of `openrage-cli maps apply` and what a second person is sent are one thing, and
//! [`apply_all`] does a list of them, all or none. A list is the file [`Operations`] reads and writes, with a format name and a schema.
//!
//! The `source` of an operation is a source of the pack as its `maps.json` names it, a folder of the module with `/`, and its `entity`
//! or `archetype` is an index into the file that source cooks to. The operations that carry a `record` or `extents` are what [`apply`]
//! gives as inverses, and are operations like the others: a list may hold them.
//!
//! ⚠ **An inverse puts the XML back byte for byte, and is checked before it is given.** An inverse says what to put back (the record an
//! edit replaced, the entity a removal took with the links that named it, the extents a move grew), and it is given only once the source's
//! XML is the export of the file it cooks to and the inverse, read back from its JSON and applied to the edited file, saves the file as it
//! was. Otherwise the inverse is [`Operation::RewriteSource`] with the XML as it was: XML a person typed by hand, which the first edit
//! rewrites whole, and a file whose links no renumbering restores, are put back that way.
//!
//! ⚠ **The boxes the extents grow to hold are the pack's own.** An operation that places an entity reads the type files of every map set
//! that names its source, the first to declare a name winning, so what it writes does not depend on which set a screen shows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CBaseArchetypeDefAny, CEntityDefAny, CMapData, CMapTypes};
use or_io::meta::Value;
use or_source::metadata::Inputs;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::edit::{self, in_parent_map, Edited, Pose};
use crate::facts::{stays_inside, Name};
use crate::model::{self, Maps};

/// The file a list of operations states it is.
pub const FORMAT: &str = "openrage-map-operations";
/// The one schema of a list this build reads and writes.
pub const SCHEMA: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a list of map operations: {1}")]
    Json(String, serde_json::Error),
    #[error("{0} is not a list of map operations: its format is `{1}`, not `{FORMAT}`")]
    Format(String, String),
    #[error(
        "{0} is a list of map operations of schema {1}, and this OpenRage reads schema {SCHEMA}"
    )]
    Schema(String, u64),
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("{name} of `{at}`: {why}")]
    Refused {
        /// The name of the operation, as its JSON states it.
        name: &'static str,
        /// The source it names.
        at: String,
        why: String,
    },
    #[error("operation {number} of {of}, {refused}")]
    InList {
        /// Counted from one.
        number: usize,
        of: usize,
        refused: Box<Error>,
    },
    #[error("{0}")]
    Write(String),
}

/// A pose as data: where an entity stands, the rotation that places it as `x, y, z, w`, which is stored at length one, and its two scales.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoseData {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale_xy: f32,
    pub scale_z: f32,
}

impl From<Pose> for PoseData {
    fn from(pose: Pose) -> Self {
        Self {
            position: pose.position.to_array(),
            rotation: pose.rotation.to_array(),
            scale_xy: pose.scale_xy,
            scale_z: pose.scale_z,
        }
    }
}

impl PoseData {
    /// The pose with its rotation at length one, refused when a number of it is not finite or its rotation has no length.
    fn checked(&self) -> Result<Pose, String> {
        let pose = Pose {
            position: Vec3::from_array(self.position),
            rotation: Quat::from_array(self.rotation),
            scale_xy: self.scale_xy,
            scale_z: self.scale_z,
        };
        if !(pose.position.is_finite()
            && pose.rotation.is_finite()
            && pose.scale_xy.is_finite()
            && pose.scale_z.is_finite())
        {
            return Err(
                "a position, rotation or scale that is not a finite number cannot be stored"
                    .to_string(),
            );
        }
        if pose.rotation.length_squared() <= f32::EPSILON {
            return Err("a rotation of no length turns nothing".to_string());
        }
        // ⚠ The file stores the conjugate, which is the turn back only of a rotation of length one.
        Ok(Pose {
            rotation: pose.rotation.normalize(),
            ..pose
        })
    }
}

/// The four corners a map file states: the box of its entities and the box it streams by.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extents {
    pub entities_extents_min: [f32; 3],
    pub entities_extents_max: [f32; 3],
    pub streaming_extents_min: [f32; 3],
    pub streaming_extents_max: [f32; 3],
}

impl Extents {
    fn of(map: &CMapData) -> Self {
        let corner = |v: or_io::decl::Vec3| [v.x, v.y, v.z];
        Self {
            entities_extents_min: corner(map.entities_extents_min),
            entities_extents_max: corner(map.entities_extents_max),
            streaming_extents_min: corner(map.streaming_extents_min),
            streaming_extents_max: corner(map.streaming_extents_max),
        }
    }

    fn corners(&self) -> [[f32; 3]; 4] {
        [
            self.entities_extents_min,
            self.entities_extents_max,
            self.streaming_extents_min,
            self.streaming_extents_max,
        ]
    }

    /// State these as the extents of `map`; a number that is not finite is refused.
    fn state(&self, map: &mut CMapData) -> Result<(), String> {
        if !self.corners().iter().flatten().all(|n| n.is_finite()) {
            return Err("extents that are not finite numbers cannot be stored".to_string());
        }
        let corner = |[x, y, z]: [f32; 3]| or_io::decl::Vec3 { x, y, z };
        map.entities_extents_min = corner(self.entities_extents_min);
        map.entities_extents_max = corner(self.entities_extents_max);
        map.streaming_extents_min = corner(self.streaming_extents_min);
        map.streaming_extents_max = corner(self.streaming_extents_max);
        Ok(())
    }

    /// These, when `map` no longer states them bit for bit: what an inverse carries to put them back.
    ///
    /// ⚠ By the bits and not by `==`, under which a zero that changed its sign is unchanged and the file is not.
    fn unless_stated_by(self, map: &CMapData) -> Option<Self> {
        let bits = |e: &Self| e.corners().map(|c| c.map(f32::to_bits));
        (bits(&Self::of(map)) != bits(&self)).then_some(self)
    }
}

/// An entity of a map file or an archetype of a type file, by the source it is in and its index there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct At {
    pub source: String,
    pub index: usize,
}

/// One entity or archetype as the value tree its file saves it as, which an operation carries so that what it puts back loses nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct Record(pub Value);

impl Record {
    /// The record of `entity`.
    pub fn entity(entity: &CEntityDefAny) -> Result<Self, String> {
        entity.to_value().map(Self).map_err(|e| e.to_string())
    }

    /// The record of `archetype`.
    pub fn archetype(archetype: &CBaseArchetypeDefAny) -> Result<Self, String> {
        archetype.to_value().map(Self).map_err(|e| e.to_string())
    }

    fn as_entity(&self) -> Result<CEntityDefAny, String> {
        CEntityDefAny::from_value(&self.0).map_err(|e| e.to_string())
    }

    fn as_archetype(&self) -> Result<CBaseArchetypeDefAny, String> {
        CBaseArchetypeDefAny::from_value(&self.0).map_err(|e| e.to_string())
    }
}

/// The key a float of a value tree is written under when it is a single-precision number, which every float of a binary file is.
///
/// ⚠ A tree holds a float as a double, whose seventeen digits a JSON reader may bring back one step off. Written as the short decimal
/// of the single it is, and read through a single, it comes back to the bit.
const SINGLE: &str = "Float32";
/// The key the value tree's own JSON gives a float.
const DOUBLE: &str = "Float";

/// The double a single-precision `number` is written as: the one nearest its shortest decimal.
fn short(number: f64) -> Option<f64> {
    let single = number as f32;
    (single.is_finite() && f64::from(single) == number)
        .then(|| single.to_string().parse().ok())
        .flatten()
}

/// Write every float of `json` that is a single as its short decimal, under [`SINGLE`] where the tree holds it as a double.
fn shorten(json: &mut serde_json::Value) {
    use serde_json::Value as Json;
    match json {
        Json::Number(number) if number.is_f64() => {
            if let Some(short) = number
                .as_f64()
                .and_then(short)
                .and_then(serde_json::Number::from_f64)
            {
                *number = short;
            }
        }
        Json::Array(items) => items.iter_mut().for_each(shorten),
        Json::Object(members) => {
            // Asked before the number is shortened, after which it is a single no more.
            let single = members.len() == 1
                && (members.get(DOUBLE).filter(|held| held.is_f64()))
                    .and_then(Json::as_f64)
                    .and_then(short)
                    .is_some();
            members.values_mut().for_each(shorten);
            if let Some(held) = single.then(|| members.remove(DOUBLE)).flatten() {
                members.insert(SINGLE.to_string(), held);
            }
        }
        _ => {}
    }
}

/// Put every float written under [`SINGLE`] back as the double the tree holds.
fn widen(json: &mut serde_json::Value) {
    use serde_json::Value as Json;
    match json {
        Json::Array(items) => items.iter_mut().for_each(widen),
        Json::Object(members) => {
            members.values_mut().for_each(widen);
            if members.len() == 1 {
                let single = members.get(SINGLE).and_then(Json::as_f64);
                if let Some(double) =
                    single.and_then(|n| serde_json::Number::from_f64(f64::from(n as f32)))
                {
                    members.clear();
                    members.insert(DOUBLE.to_string(), Json::Number(double));
                }
            }
        }
        _ => {}
    }
}

impl Serialize for Record {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut json = serde_json::to_value(&self.0).map_err(serde::ser::Error::custom)?;
        shorten(&mut json);
        json.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut json = serde_json::Value::deserialize(deserializer)?;
        widen(&mut json);
        serde_json::from_value(json)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

/// One edit of a source of a pack of maps, by the name its JSON states as `op`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Stand an entity of a map file at a pose, and grow the extents the map states to hold it.
    PlaceEntity {
        source: String,
        entity: usize,
        pose: PoseData,
    },
    /// Add an entity that places `archetype` at `pose` to a map file, as a copy of `donor` (else of the map's first entity) with a guid of its own, and grow the extents.
    AddEntity {
        source: String,
        /// The name of the archetype, or `hash_XXXXXXXX` for one known by its hash alone.
        archetype: String,
        pose: PoseData,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        donor: Option<At>,
    },
    /// Take an entity out of a map file with the LOD links renumbered, and state `extents` when it carries some.
    RemoveEntity {
        source: String,
        entity: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// Put an entity into a map file at an index with the LOD links a removal there took: what takes a removal back.
    InsertEntity {
        source: String,
        entity: usize,
        record: Record,
        /// The children the LOD parent the record names is to count, when it names one in this map.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_num_children: Option<u32>,
        /// The entities, by their index before the insertion, that name the record as their LOD parent.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        children: Vec<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// Put a record in the place of an entity of a map file, and state `extents` when it carries some: what takes a pose or a name back.
    ReplaceEntity {
        source: String,
        entity: usize,
        record: Record,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// Grow the extents a map file states to hold its entities, never shrinking them.
    GrowExtents { source: String },
    /// State the extents of a map file.
    SetExtents { source: String, extents: Extents },
    /// Add an archetype called `name` to a type file, as a copy of `donor` (else of the file's first plain archetype).
    AddArchetype {
        source: String,
        name: String,
        /// The asset it names; its own name when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        donor: Option<At>,
    },
    /// Take an archetype out of a type file.
    RemoveArchetype { source: String, archetype: usize },
    /// Put an archetype into a type file at an index: what takes a removal back.
    InsertArchetype {
        source: String,
        archetype: usize,
        record: Record,
    },
    /// Put a record in the place of an archetype of a type file.
    ReplaceArchetype {
        source: String,
        archetype: usize,
        record: Record,
    },
    /// Hold `xml` as the XML of a source, as it is given: what takes back an edit of XML that was not the export of its file.
    RewriteSource { source: String, xml: String },
}

impl Operation {
    /// The name the JSON form states as `op`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::PlaceEntity { .. } => "place_entity",
            Self::AddEntity { .. } => "add_entity",
            Self::RemoveEntity { .. } => "remove_entity",
            Self::InsertEntity { .. } => "insert_entity",
            Self::ReplaceEntity { .. } => "replace_entity",
            Self::GrowExtents { .. } => "grow_extents",
            Self::SetExtents { .. } => "set_extents",
            Self::AddArchetype { .. } => "add_archetype",
            Self::RemoveArchetype { .. } => "remove_archetype",
            Self::InsertArchetype { .. } => "insert_archetype",
            Self::ReplaceArchetype { .. } => "replace_archetype",
            Self::RewriteSource { .. } => "rewrite_source",
        }
    }

    /// The source it edits, as the pack names it.
    pub fn source(&self) -> &str {
        match self {
            Self::PlaceEntity { source, .. }
            | Self::AddEntity { source, .. }
            | Self::RemoveEntity { source, .. }
            | Self::InsertEntity { source, .. }
            | Self::ReplaceEntity { source, .. }
            | Self::GrowExtents { source }
            | Self::SetExtents { source, .. }
            | Self::AddArchetype { source, .. }
            | Self::RemoveArchetype { source, .. }
            | Self::InsertArchetype { source, .. }
            | Self::ReplaceArchetype { source, .. }
            | Self::RewriteSource { source, .. } => source,
        }
    }
}

/// A list of operations as a file: what `openrage-cli maps apply` takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operations {
    pub format: String,
    pub schema: u32,
    pub operations: Vec<Operation>,
}

impl Operations {
    pub fn new(operations: Vec<Operation>) -> Self {
        Self {
            format: FORMAT.to_string(),
            schema: SCHEMA,
            operations,
        }
    }

    /// The list as JSON with a two-space indent and LF; a record no JSON number holds is the error.
    pub fn to_json(&self) -> Result<String, Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Json("the list".to_string(), e))?;
        text.push('\n');
        Ok(text)
    }

    /// The list `bytes` hold; `shown` names them in an error. A format that is not [`FORMAT`] and a schema that is not [`SCHEMA`] are refused before an operation is read.
    pub fn from_json(bytes: &[u8], shown: &str) -> Result<Self, Error> {
        let json = |e| Error::Json(shown.to_string(), e);
        let top: serde_json::Value = serde_json::from_slice(bytes).map_err(json)?;
        let format = top.get("format").and_then(|f| f.as_str()).unwrap_or("");
        if format != FORMAT {
            return Err(Error::Format(shown.to_string(), format.to_string()));
        }
        let schema = top.get("schema").and_then(serde_json::Value::as_u64);
        if schema != Some(u64::from(SCHEMA)) {
            return Err(Error::Schema(shown.to_string(), schema.unwrap_or(0)));
        }
        serde_json::from_value(top).map_err(json)
    }
}

/// What an operation did.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The operation that puts the source back byte for byte.
    pub inverse: Operation,
    /// The XML of the source it edited, as it was and as the operation left it.
    pub edited: Edited,
    /// The index of the entity or the archetype it added, when it added one.
    pub added: Option<usize>,
}

/// The local box and the stream distance of an archetype, by the hash of its name.
type Boxes = BTreeMap<u32, (Vec3, Vec3, f32)>;

/// What an operation reads besides the file it edits.
#[derive(Default)]
struct Context {
    boxes: Boxes,
    /// The entity an added one is a copy of, when it is in another source.
    entity: Option<CEntityDefAny>,
    /// The archetype an added one is a copy of, when it is in another source.
    archetype: Option<CBaseArchetypeDefAny>,
}

/// The operation that takes a change back, and the index of what the change added.
type Changed = (Operation, Option<usize>);

fn entity_at(map: &CMapData, index: usize) -> Result<&CEntityDefAny, String> {
    let held = map.entities.len();
    map.entities
        .get(index)
        .ok_or_else(|| format!("entity {index} is past the {held} the map holds"))
}

fn archetype_at(types: &CMapTypes, index: usize) -> Result<&CBaseArchetypeDefAny, String> {
    let held = types.archetypes.len();
    types
        .archetypes
        .get(index)
        .ok_or_else(|| format!("archetype {index} is past the {held} the type file holds"))
}

/// The name `text` spells: a hash alone for `hash_XXXXXXXX`.
fn name_of(text: &str) -> Name {
    let spelled = HashedString::from_xml_text(text);
    Name {
        hash: spelled.hash,
        text: (!spelled.raw.is_empty()).then_some(spelled.raw),
    }
}

/// Do `operation` to `map`.
fn change_map(
    map: &mut CMapData,
    operation: &Operation,
    context: &Context,
) -> Result<Changed, String> {
    let source = operation.source().to_string();
    let was = Extents::of(map);
    let grow = |map: &mut CMapData| {
        edit::grow_extents(map, |hash| context.boxes.get(&hash).copied());
    };
    match operation {
        Operation::PlaceEntity { entity, pose, .. } => {
            let record = Record::entity(entity_at(map, *entity)?)?;
            edit::place_entity(map, *entity, pose.checked()?)?;
            grow(map);
            let inverse = Operation::ReplaceEntity {
                source,
                entity: *entity,
                record,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, None))
        }
        Operation::AddEntity {
            archetype,
            pose,
            donor,
            ..
        } => {
            let pose = pose.checked()?;
            let placed = name_of(archetype);
            if placed.is_empty() {
                return Err("an entity places an archetype, and none is named".to_string());
            }
            let copied = match (&context.entity, donor) {
                (Some(entity), _) => entity.clone(),
                (None, Some(at)) => entity_at(map, at.index)?.clone(),
                (None, None) => map.entities.first().cloned().ok_or_else(|| {
                    "the map holds no entity to copy: name a `donor` in a map file that has one"
                        .to_string()
                })?,
            };
            let added = edit::add_entity(map, &copied, placed, pose)?;
            grow(map);
            let inverse = Operation::RemoveEntity {
                source,
                entity: added,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, Some(added)))
        }
        Operation::RemoveEntity {
            entity, extents, ..
        } => {
            let index = *entity;
            let removed = entity_at(map, index)?;
            let record = Record::entity(removed)?;
            let parent_num_children = usize::try_from(removed.parent_index)
                .ok()
                .filter(|parent| *parent != index && !in_parent_map(removed.flags))
                .and_then(|parent| map.entities.get(parent))
                .map(|parent| parent.num_children);
            let children = (map.entities.iter().enumerate())
                .filter(|(at, e)| {
                    *at != index
                        && !in_parent_map(e.flags)
                        && usize::try_from(e.parent_index) == Ok(index)
                })
                .map(|(at, _)| if at > index { at - 1 } else { at })
                .collect();
            edit::remove_entity(map, index)?;
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::InsertEntity {
                source,
                entity: index,
                record,
                parent_num_children,
                children,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, None))
        }
        Operation::InsertEntity {
            entity,
            record,
            parent_num_children,
            children,
            extents,
            ..
        } => {
            let index = *entity;
            let held = map.entities.len();
            if index > held {
                return Err(format!(
                    "entity {index} is past the end of the {held} the map holds"
                ));
            }
            let stated = i32::try_from(index)
                .map_err(|_| format!("entity {index} is past what a LOD link can name"))?;
            let inserted = record.as_entity()?;
            if let Some(past) = children.iter().find(|child| **child >= held) {
                return Err(format!("child {past} is past the {held} the map holds"));
            }
            // What named an entity at or after the index names the one after it now.
            for e in map.entities.iter_mut() {
                if !in_parent_map(e.flags) && e.parent_index >= stated {
                    e.parent_index = e.parent_index.checked_add(1).ok_or_else(|| {
                        "a LOD link past what an index can state cannot be renumbered".to_string()
                    })?;
                }
            }
            for child in children {
                map.entities[*child].parent_index = stated;
            }
            map.entities.insert(index, inserted);
            if let Some(count) = parent_num_children {
                let parent = usize::try_from(map.entities[index].parent_index)
                    .ok()
                    .filter(|parent| *parent != index && !in_parent_map(map.entities[index].flags))
                    .and_then(|parent| map.entities.get_mut(parent))
                    .ok_or_else(|| {
                        "the record names no LOD parent in this map to count children for"
                            .to_string()
                    })?;
                parent.num_children = *count;
            }
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::RemoveEntity {
                source,
                entity: index,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, Some(index)))
        }
        Operation::ReplaceEntity {
            entity,
            record,
            extents,
            ..
        } => {
            let before = Record::entity(entity_at(map, *entity)?)?;
            map.entities[*entity] = record.as_entity()?;
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::ReplaceEntity {
                source,
                entity: *entity,
                record: before,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, None))
        }
        Operation::GrowExtents { .. } => {
            grow(map);
            Ok((
                Operation::SetExtents {
                    source,
                    extents: was,
                },
                None,
            ))
        }
        Operation::SetExtents { extents, .. } => {
            extents.state(map)?;
            Ok((
                Operation::SetExtents {
                    source,
                    extents: was,
                },
                None,
            ))
        }
        Operation::AddArchetype { .. }
        | Operation::RemoveArchetype { .. }
        | Operation::InsertArchetype { .. }
        | Operation::ReplaceArchetype { .. }
        | Operation::RewriteSource { .. } => Err(format!(
            "{} does not edit a map file, which this source is",
            operation.name()
        )),
    }
}

/// Do `operation` to `types`.
fn change_types(
    types: &mut CMapTypes,
    operation: &Operation,
    context: &Context,
) -> Result<Changed, String> {
    let source = operation.source().to_string();
    let once = "the type file already declares that name, and the game resolves a name once";
    match operation {
        Operation::AddArchetype {
            name, asset, donor, ..
        } => {
            let name = name.trim();
            if name.is_empty() {
                return Err("an archetype has a name, and none is given".to_string());
            }
            let copied = match (&context.archetype, donor) {
                (Some(archetype), _) => Some(archetype.clone()),
                (None, Some(at)) => Some(archetype_at(types, at.index)?.clone()),
                (None, None) => None,
            };
            let added = edit::add_archetype(types, copied.as_ref(), name, asset.as_deref())?;
            let inverse = Operation::RemoveArchetype {
                source,
                archetype: added,
            };
            Ok((inverse, Some(added)))
        }
        Operation::RemoveArchetype { archetype, .. } => {
            let record = Record::archetype(archetype_at(types, *archetype)?)?;
            edit::remove_archetype(types, *archetype)?;
            let inverse = Operation::InsertArchetype {
                source,
                archetype: *archetype,
                record,
            };
            Ok((inverse, None))
        }
        Operation::InsertArchetype {
            archetype, record, ..
        } => {
            let held = types.archetypes.len();
            if *archetype > held {
                return Err(format!(
                    "archetype {archetype} is past the end of the {held} the type file holds"
                ));
            }
            let inserted = record.as_archetype()?;
            if (types.archetypes.iter()).any(|a| a.name.hash == inserted.name.hash) {
                return Err(once.to_string());
            }
            types.archetypes.insert(*archetype, inserted);
            let inverse = Operation::RemoveArchetype {
                source,
                archetype: *archetype,
            };
            Ok((inverse, Some(*archetype)))
        }
        Operation::ReplaceArchetype {
            archetype, record, ..
        } => {
            let held = archetype_at(types, *archetype)?;
            let before = Record::archetype(held)?;
            let replacing = record.as_archetype()?;
            // A name the file declares twice already is not this operation's to refuse: only a name it would newly share is.
            if replacing.name.hash != held.name.hash
                && (types.archetypes.iter()).any(|a| a.name.hash == replacing.name.hash)
            {
                return Err(once.to_string());
            }
            types.archetypes[*archetype] = replacing;
            let inverse = Operation::ReplaceArchetype {
                source,
                archetype: *archetype,
                record: before,
            };
            Ok((inverse, None))
        }
        Operation::PlaceEntity { .. }
        | Operation::AddEntity { .. }
        | Operation::RemoveEntity { .. }
        | Operation::InsertEntity { .. }
        | Operation::ReplaceEntity { .. }
        | Operation::GrowExtents { .. }
        | Operation::SetExtents { .. }
        | Operation::RewriteSource { .. } => Err(format!(
            "{} does not edit a type file, which this source is",
            operation.name()
        )),
    }
}

fn problems(found: Vec<or_source::Problem>) -> String {
    found
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// `inverse` as its JSON reads back, once doing that to the file `after` the change saves the bytes `was`.
fn proven<R: File + Clone>(
    after: &R,
    inverse: &Operation,
    was: &[u8],
    change: fn(&mut R, &Operation, &Context) -> Result<Changed, String>,
) -> Option<Operation> {
    let read_back: Operation = serde_json::from_str(&serde_json::to_string(inverse).ok()?).ok()?;
    let mut back = after.clone();
    change(&mut back, &read_back, &Context::default()).ok()?;
    (back.to_bytes().ok()? == was).then_some(read_back)
}

/// Do `operation` to the file `inputs` cook to; the XML that makes them cook to the result, the inverse and the index of what was added.
fn through<R: File + Clone>(
    inputs: &Inputs,
    operation: &Operation,
    context: &Context,
    change: fn(&mut R, &Operation, &Context) -> Result<Changed, String>,
) -> Result<(Vec<u8>, Operation, Option<usize>), String> {
    let cooked = inputs.compile().map_err(problems)?.bytes;
    let mut root = R::from_bytes(&cooked).map_err(|e| e.to_string())?;
    // What the file saves as before the change: what an inverse has to save again.
    let untouched = root.to_bytes().ok();
    let (inverse, added) = change(&mut root, operation, context)?;
    let bytes = root.to_bytes().map_err(|e| e.to_string())?;
    let xml = inputs.xml_of(&bytes).map_err(problems)?;
    let inverse = match untouched
        .filter(|was| inputs.is_the_export_of(was))
        .and_then(|was| proven(&root, &inverse, &was, change))
    {
        Some(inverse) => inverse,
        None => Operation::RewriteSource {
            source: operation.source().to_string(),
            xml: text_of(inputs)?,
        },
    };
    Ok((xml, inverse, added))
}

/// The XML `inputs` hold, which an inverse carries as text.
fn text_of(inputs: &Inputs) -> Result<String, String> {
    String::from_utf8(inputs.xml().to_vec()).map_err(|_| {
        "the XML of the source is not UTF-8, so the operation could not be taken back".to_string()
    })
}

/// A source the work has opened: its inputs with the XML the operations so far leave, and the XML on disk.
struct Held {
    dir: PathBuf,
    inputs: Inputs,
    on_disk: Vec<u8>,
}

/// The sources of a pack while operations are applied: nothing is written until [`Work::commit`].
struct Work<'a> {
    folder: &'a Path,
    model: Maps,
    held: BTreeMap<String, Held>,
}

impl<'a> Work<'a> {
    fn open(folder: &'a Path) -> Result<Self, Error> {
        Ok(Self {
            folder,
            model: Maps::read(folder)?,
            held: BTreeMap::new(),
        })
    }

    /// The source `source`, opened once; a name no set of the pack holds, or one that leaves its folder, is refused, so an operation reaches nothing else.
    fn hold(&mut self, source: &str) -> Result<&mut Held, String> {
        if !self.held.contains_key(source) {
            let named = (self.model.maps.iter()).any(|set| set.sources.iter().any(|s| s == source));
            if !named {
                return Err("no map set of the pack names this source".to_string());
            }
            // ⚠ The pack's file is a file anyone can edit, and a name in it with `..` would be written beside the pack.
            if !stays_inside(source) {
                return Err(
                    "the source leaves the folder of the pack, and an operation edits what is inside it"
                        .to_string(),
                );
            }
            let dir = self
                .folder
                .join(source.replace('/', std::path::MAIN_SEPARATOR_STR));
            let opened = or_source::asset::Source::open(&dir).map_err(|p| p.to_string())?;
            let inputs = Inputs::read(&opened).map_err(problems)?;
            let on_disk = inputs.xml().to_vec();
            let held = Held {
                dir,
                inputs,
                on_disk,
            };
            self.held.insert(source.to_string(), held);
        }
        self.held
            .get_mut(source)
            .ok_or_else(|| "the source was not opened".to_string())
    }

    /// The file the source `source` cooks to, as the root `R` whose files have the extension it states.
    fn root<R: File>(&mut self, source: &str) -> Result<R, String> {
        let held = self.hold(source)?;
        let extension = held.inputs.extension();
        if extension != R::EXTENSION {
            return Err(format!(
                "it is a .{extension} source, and a .{} is asked of it",
                R::EXTENSION
            ));
        }
        let cooked = held.inputs.compile().map_err(problems)?.bytes;
        R::from_bytes(&cooked).map_err(|e| e.to_string())
    }

    /// The boxes of the archetypes that the type files of every set naming `source` declare; a source that does not read as a type file declares none.
    fn boxes(&mut self, source: &str) -> Boxes {
        let paths: Vec<String> = (self.model.maps.iter())
            .filter(|set| set.sources.iter().any(|s| s == source))
            .flat_map(|set| set.sources.iter().cloned())
            .collect();
        let corner = |v: or_io::decl::Vec3| Vec3::new(v.x, v.y, v.z);
        let mut boxes = Boxes::new();
        for path in paths {
            let Ok(types) = self.root::<CMapTypes>(&path) else {
                continue;
            };
            for archetype in &types.archetypes {
                boxes.entry(archetype.name.hash).or_insert((
                    corner(archetype.bb_min),
                    corner(archetype.bb_max),
                    archetype.lod_dist,
                ));
            }
        }
        boxes
    }

    /// What `operation` reads besides the file it edits.
    fn context(&mut self, operation: &Operation) -> Result<Context, String> {
        let source = operation.source();
        let elsewhere = |donor: &Option<At>| donor.clone().filter(|at| at.source != source);
        let mut context = Context::default();
        match operation {
            Operation::PlaceEntity { .. } | Operation::GrowExtents { .. } => {
                context.boxes = self.boxes(source);
            }
            Operation::AddEntity { donor, .. } => {
                context.boxes = self.boxes(source);
                if let Some(at) = elsewhere(donor) {
                    let copied = (self.root::<CMapData>(&at.source))
                        .and_then(|map| entity_at(&map, at.index).cloned())
                        .map_err(|why| format!("the donor in `{}`: {why}", at.source))?;
                    context.entity = Some(copied);
                }
            }
            Operation::AddArchetype { donor, .. } => {
                if let Some(at) = elsewhere(donor) {
                    let copied = (self.root::<CMapTypes>(&at.source))
                        .and_then(|types| archetype_at(&types, at.index).cloned())
                        .map_err(|why| format!("the donor in `{}`: {why}", at.source))?;
                    context.archetype = Some(copied);
                }
            }
            Operation::RemoveEntity { .. }
            | Operation::InsertEntity { .. }
            | Operation::ReplaceEntity { .. }
            | Operation::SetExtents { .. }
            | Operation::RemoveArchetype { .. }
            | Operation::InsertArchetype { .. }
            | Operation::ReplaceArchetype { .. }
            | Operation::RewriteSource { .. } => {}
        }
        Ok(context)
    }

    /// Do `operation` to the XML the work holds of its source.
    fn apply(&mut self, operation: &Operation) -> Result<Applied, String> {
        let source = operation.source();
        let extension = self.hold(source)?.inputs.extension().to_string();
        let (xml, inverse, added) = match (operation, extension.as_str()) {
            (Operation::RewriteSource { xml, .. }, _) => {
                let held = self.hold(source)?;
                let was = text_of(&held.inputs)?;
                // ⚠ XML that does not compile would be a source nothing cooks.
                let mut candidate = held.inputs.clone();
                candidate.set_xml(xml.clone().into_bytes());
                candidate.compile().map_err(problems)?;
                let inverse = Operation::RewriteSource {
                    source: source.to_string(),
                    xml: was,
                };
                (xml.clone().into_bytes(), inverse, None)
            }
            (_, CMapData::EXTENSION) => {
                let context = self.context(operation)?;
                through::<CMapData>(&self.hold(source)?.inputs, operation, &context, change_map)?
            }
            (_, CMapTypes::EXTENSION) => {
                let context = self.context(operation)?;
                let inputs = &self.hold(source)?.inputs;
                through::<CMapTypes>(inputs, operation, &context, change_types)?
            }
            (_, other) => {
                return Err(format!(
                    "it is a .{other} source, and an operation edits a map file or a type file"
                ))
            }
        };
        let held = self.hold(source)?;
        let before = held.inputs.xml().to_vec();
        held.inputs.set_xml(xml.clone());
        Ok(Applied {
            inverse,
            edited: Edited {
                file: held.dir.join(or_source::metadata::XML),
                before,
                after: xml,
            },
            added,
        })
    }

    /// Write the XML of every source the operations changed; when one cannot be written, the ones written before it are put back.
    fn commit(self) -> Result<(), Error> {
        let mut written: Vec<&Held> = Vec::new();
        for held in self.held.values() {
            if held.inputs.xml() == held.on_disk.as_slice() {
                continue;
            }
            if let Err(why) = or_source::metadata::write_xml(&held.dir, held.inputs.xml()) {
                let mut said = problems(why);
                for earlier in written {
                    if let Err(left) =
                        or_source::metadata::write_xml(&earlier.dir, &earlier.on_disk)
                    {
                        said.push_str(&format!(
                            "; and {} could not be put back as it was: {}",
                            earlier.dir.display(),
                            problems(left)
                        ));
                    }
                }
                return Err(Error::Write(said));
            }
            written.push(held);
        }
        Ok(())
    }
}

/// Apply `operation` to the sources of the pack of maps in `folder`, and say what takes it back; a refusal leaves every source as it was.
pub fn apply(folder: &Path, operation: &Operation) -> Result<Applied, Error> {
    let mut work = Work::open(folder)?;
    let applied = work.apply(operation).map_err(|why| Error::Refused {
        name: operation.name(),
        at: operation.source().to_string(),
        why,
    })?;
    work.commit()?;
    Ok(applied)
}

/// Apply `operations` in order to the sources of the pack of maps in `folder`, all of them or none, and say what each did; with `dry_run` nothing is written.
pub fn apply_all(
    folder: &Path,
    operations: &[Operation],
    dry_run: bool,
) -> Result<Vec<Applied>, Error> {
    let mut work = Work::open(folder)?;
    let mut applied = Vec::with_capacity(operations.len());
    for (index, operation) in operations.iter().enumerate() {
        let done = work.apply(operation).map_err(|why| Error::InList {
            number: index + 1,
            of: operations.len(),
            refused: Box::new(Error::Refused {
                name: operation.name(),
                at: operation.source().to_string(),
                why,
            }),
        })?;
        applied.push(done);
    }
    if !dry_run {
        work.commit()?;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::MetaInstance;

    /// A record holding `floats` as the doubles a value tree holds a file's singles as.
    fn record_of(floats: &[f32]) -> Record {
        let mut inst = MetaInstance::new("CEntityDef");
        for (at, float) in floats.iter().enumerate() {
            inst.set(format!("member{at}"), Value::Float(f64::from(*float)));
        }
        Record(Value::Pointer(Some(Box::new(inst))))
    }

    fn bits_of(record: &Record) -> Vec<u64> {
        let Value::Pointer(Some(inst)) = &record.0 else {
            panic!("a record behind a pointer");
        };
        (inst.fields.iter())
            .map(|(_, value)| match value {
                Value::Float(float) => float.to_bits(),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_float_of_a_record_comes_back_from_its_json_to_the_bit() {
        let mut floats: Vec<f32> = (0..20_000u16)
            .map(|step| 0.1 + f32::from(step) * 0.37)
            .collect();
        floats.extend([-0.0, f32::MIN_POSITIVE, f32::MAX, -1.0e-30, 16_777_216.0]);
        let record = record_of(&floats);
        let text = serde_json::to_string(&record).unwrap();
        assert!(
            text.contains("{\"Float32\":0.1}"),
            "a single is written as its own short decimal"
        );
        let back: Record = serde_json::from_str(&text).unwrap();
        assert_eq!(bits_of(&back), bits_of(&record));
        assert_eq!(back, record);
    }

    #[test]
    fn a_double_that_is_no_single_keeps_its_own_key_and_a_float_json_has_no_number_for_is_not_read_back(
    ) {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set("double", Value::Float(0.1));
        inst.set("vector", Value::Vec3([0.1, -0.0, 3.0e9]));
        let record = Record(Value::Struct(Box::new(inst)));
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("{\"Float\":0.1}"), "{text}");
        assert!(text.contains("[0.1,-0.0,3000000000.0]"), "{text}");
        assert_eq!(serde_json::from_str::<Record>(&text).unwrap(), record);

        let text = serde_json::to_string(&record_of(&[f32::NAN])).unwrap();
        assert!(serde_json::from_str::<Record>(&text).is_err(), "{text}");
    }
}
