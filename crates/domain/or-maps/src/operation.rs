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
//! rewrites whole, a file whose links no renumbering restores, and a file the last record of a class was taken out of, which read
//! again from its XML saves no record of that class (a map file left with no entity is one), are put back that way.
//!
//! ⚠ **An inverse states what it applies to.** An operation may state `expects`, the digest of the XML its source is to hold
//! ([`digest_of`]), and is refused when the source holds another ([`Error::Stale`]). Every inverse states the digest of the XML its
//! operation wrote, so an inverse applied after another edit of its source is refused, and a step of Undo is an operation and its
//! inverse with no copy of the XML.
//!
//! [`Session`] holds the sources of a pack open as typed roots, does an operation to a root with no XML produced, and writes the
//! sources when it is flushed: what an edit costs on a large map is in `tests/what_one_pose_edit_costs.rs`.
//!
//! ⚠ **A LOD link into a parent map is another file's to keep.** An entity whose `flags` bit 3 is set names an index of the map its own
//! names as parent, so taking entities out of one file moves indexes a second file holds and lowers a count a third states. The
//! operations that say so are `remove_entities`, `set_num_children` and `renumber_for_removed`, with the two that take them back, and
//! [`crate::delete`] writes the list of them that removes an entity with its chain.
//!
//! ⚠ **The boxes the extents grow to hold are the pack's own.** An operation that places an entity reads the type files of every map set
//! that names its source, the first to declare a name winning, so what it writes does not depend on which set a screen shows. An entity
//! of an archetype none of them declares, as any prop of the game is, is held where it stands, and [`Applied::notes`] says so.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{
    CBaseArchetypeDef, CBaseArchetypeDefAny, CEntityDefAny, CMapData, CMapTypes,
};
use or_io::meta::Value;
use or_source::metadata::Inputs;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::edit::{self, in_parent_map, Edited, Pose};
use crate::facts::{stays_inside, Name};
use crate::model::{self, Maps};

mod session;
pub use session::{Done, Session, Written};

/// The file a list of operations states it is.
pub const FORMAT: &str = "openrage-map-operations";
/// The one schema of a list this build reads and writes.
pub const SCHEMA: u32 = 1;

/// Why a list of operations was not read, or an operation not applied.
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
    #[error(
        "{name} of `{at}`: the source is not as the operation expects it, so it was edited since the operation was made: \
         take back what was done after it first (it expects the digest {expects}, and the source has {found})"
    )]
    Stale {
        name: &'static str,
        at: String,
        expects: String,
        /// The digest of the XML the source holds, or the name a session gives a state it has not written.
        found: String,
    },
    #[error(
        "`{at}` cannot be written: {why}; it could be until operation {number} of the {of} done to it since it was last written ({name})"
    )]
    Unwritten {
        at: String,
        why: String,
        /// Counted from one, among the operations done to the source since it was last written.
        number: usize,
        of: usize,
        name: &'static str,
    },
    #[error("`{at}`: {why}")]
    Source { at: String, why: String },
    #[error(
        "`{at}` was changed on disk since the session read it, so what the session holds of it was not written over it; no source was written"
    )]
    Changed { at: String },
    #[error("operation {number} of {of}, {refused}")]
    InList {
        /// Counted from one.
        number: usize,
        of: usize,
        refused: Box<Error>,
    },
    #[error(
        "{refused}; and operation {number} of the list, done before it, could not be taken back ({why}), so the session holds part of the list: discard it"
    )]
    NotTakenBack {
        refused: Box<Error>,
        /// Counted from one.
        number: usize,
        why: String,
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

    /// Refuse a number of these that is not finite.
    fn finite(&self) -> Result<(), String> {
        if self.corners().iter().flatten().all(|n| n.is_finite()) {
            Ok(())
        } else {
            Err("extents that are not finite numbers cannot be stored".to_string())
        }
    }

    /// State these as the extents of `map`; a number that is not finite is refused.
    fn state(&self, map: &mut CMapData) -> Result<(), String> {
        self.finite()?;
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

/// One entity an [`Operation::InsertEntities`] puts into a map file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inserted {
    /// Its index once every entity of the operation is in.
    pub entity: usize,
    pub record: Record,
    /// The entities, by their index before the insertion, that name the record as their LOD parent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<usize>,
}

/// The children an entity of a map file is to count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counted {
    pub entity: usize,
    pub num_children: u32,
}

/// One edit of a source of a pack of maps, by the name its JSON states as `op`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Stand an entity of a map file at a pose, and grow the extents the map states to hold it.
    PlaceEntity {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        entity: usize,
        pose: PoseData,
    },
    /// Add an entity that places `archetype` at `pose` to a map file, as a copy of `donor` (else of the map's first entity) with a guid of its own, and grow the extents; the first of a map that holds none is built with what `donor` states and is no copy.
    AddEntity {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        /// The name of the archetype, or `hash_XXXXXXXX` for one known by its hash alone.
        archetype: String,
        pose: PoseData,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        donor: Option<At>,
    },
    /// Take an entity out of a map file with the LOD links renumbered, and state `extents` when it carries some.
    RemoveEntity {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        entity: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// Put an entity into a map file at an index with the LOD links a removal there took: what takes a removal back.
    InsertEntity {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
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
    /// Take several entities out of a map file at once, with the LOD links inside the file renumbered once, and state `extents` when it carries some.
    RemoveEntities {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        entities: Vec<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// Put several entities into a map file with the LOD links a removal of them took: what takes that removal back.
    InsertEntities {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        entities: Vec<Inserted>,
        /// The children the entities named here are to count, by their index after the insertion.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        num_children: Vec<Counted>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// State the children an entity of a map file counts: what a deletion in a map whose parent this one is lowers.
    SetNumChildren {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        entity: usize,
        num_children: u32,
    },
    /// Renumber the LOD links of a map file into its parent map for entities taken out of that map, named by their indexes there before.
    RenumberForRemoved {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        removed: Vec<usize>,
    },
    /// Renumber the LOD links of a map file into its parent map for entities put into that map, named by their indexes there after.
    RenumberForInserted {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        inserted: Vec<usize>,
    },
    /// Put a record in the place of an entity of a map file, and state `extents` when it carries some: what takes a pose or a name back.
    ReplaceEntity {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        entity: usize,
        record: Record,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extents: Option<Extents>,
    },
    /// Grow the extents a map file states to hold its entities, never shrinking them.
    GrowExtents {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
    },
    /// State the extents of a map file.
    SetExtents {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        extents: Extents,
    },
    /// Add an archetype called `name` to a type file, as a copy of `donor` (else of the file's first plain archetype); the first of a file that holds none is built with what `donor` states and is no copy.
    AddArchetype {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        name: String,
        /// The asset it names; its own name when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        donor: Option<At>,
    },
    /// Take an archetype out of a type file.
    RemoveArchetype {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        archetype: usize,
    },
    /// Put an archetype into a type file at an index: what takes a removal back.
    InsertArchetype {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        archetype: usize,
        record: Record,
    },
    /// Put a record in the place of an archetype of a type file.
    ReplaceArchetype {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        archetype: usize,
        record: Record,
    },
    /// Hold `xml` as the XML of a source, as it is given: what takes back an edit of XML that was not the export of its file.
    RewriteSource {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        xml: String,
    },
}

impl Operation {
    /// The name the JSON form states as `op`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::PlaceEntity { .. } => "place_entity",
            Self::AddEntity { .. } => "add_entity",
            Self::RemoveEntity { .. } => "remove_entity",
            Self::InsertEntity { .. } => "insert_entity",
            Self::RemoveEntities { .. } => "remove_entities",
            Self::InsertEntities { .. } => "insert_entities",
            Self::SetNumChildren { .. } => "set_num_children",
            Self::RenumberForRemoved { .. } => "renumber_for_removed",
            Self::RenumberForInserted { .. } => "renumber_for_inserted",
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
            | Self::RemoveEntities { source, .. }
            | Self::InsertEntities { source, .. }
            | Self::SetNumChildren { source, .. }
            | Self::RenumberForRemoved { source, .. }
            | Self::RenumberForInserted { source, .. }
            | Self::ReplaceEntity { source, .. }
            | Self::GrowExtents { source, .. }
            | Self::SetExtents { source, .. }
            | Self::AddArchetype { source, .. }
            | Self::RemoveArchetype { source, .. }
            | Self::InsertArchetype { source, .. }
            | Self::ReplaceArchetype { source, .. }
            | Self::RewriteSource { source, .. } => source,
        }
    }

    /// The digest its source is to have when it is applied, when it states one: every inverse does.
    pub fn expects(&self) -> Option<&str> {
        match self {
            Self::PlaceEntity { expects, .. }
            | Self::AddEntity { expects, .. }
            | Self::RemoveEntity { expects, .. }
            | Self::InsertEntity { expects, .. }
            | Self::RemoveEntities { expects, .. }
            | Self::InsertEntities { expects, .. }
            | Self::SetNumChildren { expects, .. }
            | Self::RenumberForRemoved { expects, .. }
            | Self::RenumberForInserted { expects, .. }
            | Self::ReplaceEntity { expects, .. }
            | Self::GrowExtents { expects, .. }
            | Self::SetExtents { expects, .. }
            | Self::AddArchetype { expects, .. }
            | Self::RemoveArchetype { expects, .. }
            | Self::InsertArchetype { expects, .. }
            | Self::ReplaceArchetype { expects, .. }
            | Self::RewriteSource { expects, .. } => expects.as_deref(),
        }
    }

    /// State the digest its source is to have, or that any state will do.
    pub fn set_expects(&mut self, digest: Option<String>) {
        match self {
            Self::PlaceEntity { expects, .. }
            | Self::AddEntity { expects, .. }
            | Self::RemoveEntity { expects, .. }
            | Self::InsertEntity { expects, .. }
            | Self::RemoveEntities { expects, .. }
            | Self::InsertEntities { expects, .. }
            | Self::SetNumChildren { expects, .. }
            | Self::RenumberForRemoved { expects, .. }
            | Self::RenumberForInserted { expects, .. }
            | Self::ReplaceEntity { expects, .. }
            | Self::GrowExtents { expects, .. }
            | Self::SetExtents { expects, .. }
            | Self::AddArchetype { expects, .. }
            | Self::RemoveArchetype { expects, .. }
            | Self::InsertArchetype { expects, .. }
            | Self::ReplaceArchetype { expects, .. }
            | Self::RewriteSource { expects, .. } => *expects = digest,
        }
    }

    /// The names it states as text, which a file keeps the hash of alone: the archetype an entity is added of, and the name and the asset of an archetype added; none of one spelled `hash_XXXXXXXX`.
    pub fn names(&self) -> Vec<&str> {
        let stated: Vec<&str> = match self {
            Self::AddEntity { archetype, .. } => vec![archetype],
            Self::AddArchetype { name, asset, .. } => {
                (std::iter::once(name).chain(asset).map(String::as_str)).collect()
            }
            _ => Vec::new(),
        };
        (stated.into_iter().map(str::trim))
            .filter(|text| name_of(text).text.is_some())
            .collect()
    }

    /// The operation, refused unless its source has the digest `digest`.
    pub fn expecting(mut self, digest: impl Into<String>) -> Self {
        self.set_expects(Some(digest.into()));
        self
    }
}

/// The digest of the XML of a source: what an operation that states `expects` is held to.
pub fn digest_of(xml: &[u8]) -> String {
    or_source::asset::sha256_hex(xml)
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
    /// The operation that puts the source back byte for byte, which states the digest of the XML this one left.
    pub inverse: Operation,
    /// The XML of the source it edited, as it was and as the operation left it.
    pub edited: Edited,
    /// The index of the entity or the archetype it added, when it added one.
    pub added: Option<usize>,
    /// What it could not read and went without, or left to another file, a sentence each: an archetype with no box in the pack, a type file that does not cook, the links of another map file into entities it moved.
    pub notes: Vec<String>,
}

/// The local box and the stream distance of an archetype, by the hash of its name.
type Boxes = BTreeMap<u32, (Vec3, Vec3, f32)>;

/// What an operation reads besides the file it edits.
#[derive(Default)]
struct Context {
    boxes: Arc<Boxes>,
    /// The entity an added one is a copy of, when it is in another source.
    entity: Option<CEntityDefAny>,
    /// The archetype an added one is a copy of, when it is in another source.
    archetype: Option<CBaseArchetypeDefAny>,
    /// What the operation went without, a sentence each, for [`Applied::notes`].
    notes: RefCell<Vec<String>>,
}

/// The first entity of a map that holds none: `placed` at `pose`, built here with what `like` states of itself.
///
/// ⚠ Never a copy. A map with no entity states no class of one, and a copy keeps the tree of the file it was read from, which this
/// file cannot lay out; a record built here is laid out as `or-io` declares its class, in a PSO, and refused by name in an RBM file.
fn first_entity(
    placed: Name,
    pose: Pose,
    like: Option<&CEntityDefAny>,
) -> Result<CEntityDefAny, String> {
    if like.is_some_and(|like| like.as_mlo_instance().is_some()) {
        return Err(
            "the first entity of a map is built here and not copied, and an interior instance cannot be built from nothing yet: name a `donor` that is no interior"
                .to_string(),
        );
    }
    let archetype = match placed.text {
        Some(text) => HashedString::new(text),
        None => HashedString::from_hash(placed.hash),
    };
    let chosen = or_world::create::Placed {
        rotation: pose.rotation,
        scale_xy: pose.scale_xy,
        scale_z: pose.scale_z,
        flags: like.map_or(0, |like| like.flags),
        ..or_world::create::Placed::at(archetype, pose.position, 0.0)
    };
    let mut entity = or_world::create::entity(&chosen, 1).map_err(|e| e.to_string())?;
    if let Some(like) = like {
        entity.lod_dist = like.lod_dist;
        entity.child_lod_dist = like.child_lod_dist;
        entity.ambient_occlusion_multiplier = like.ambient_occlusion_multiplier;
        entity.artificial_ambient_occlusion = like.artificial_ambient_occlusion;
        entity.tint_value = like.tint_value;
    }
    Ok(entity)
}

/// The first archetype of a type file that holds none: what `like`, a plain archetype, states, built here and called `name`.
///
/// ⚠ Never a copy, for the reason [`first_entity`] is none.
fn first_archetype(
    like: &CBaseArchetypeDefAny,
    name: &str,
    asset: Option<&str>,
) -> Result<CBaseArchetypeDefAny, String> {
    let CBaseArchetypeDefAny::Base(like) = like else {
        return Err(
            "the first archetype of a type file is built here from a plain one, and the donor is an interior or a timed one"
                .to_string(),
        );
    };
    let mut built = CBaseArchetypeDef::default();
    built.lod_dist = like.lod_dist;
    built.flags = like.flags;
    built.special_attribute = like.special_attribute;
    built.bb_min = like.bb_min;
    built.bb_max = like.bb_max;
    built.bs_centre = like.bs_centre;
    built.bs_radius = like.bs_radius;
    built.hd_texture_dist = like.hd_texture_dist;
    built.name = HashedString::new(name);
    built.texture_dictionary = like.texture_dictionary.clone();
    built.clip_dictionary = like.clip_dictionary.clone();
    built.drawable_dictionary = like.drawable_dictionary.clone();
    built.physics_dictionary = like.physics_dictionary.clone();
    built.asset_name = HashedString::new(asset.unwrap_or(name));
    Ok(CBaseArchetypeDefAny::Base(built))
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

/// `indexes` in ascending order; a list that names none, or one twice, is refused.
fn ascending(indexes: &[usize]) -> Result<Vec<usize>, String> {
    let mut sorted = indexes.to_vec();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return Err("no entity is named".to_string());
    }
    match sorted.windows(2).find(|pair| pair[0] == pair[1]) {
        Some(pair) => Err(format!("entity {} is named twice", pair[0])),
        None => Ok(sorted),
    }
}

/// Where the entity at `index` stands once the entities at `removed`, ascending, are taken out; it is not one of them.
fn after_removal(index: usize, removed: &[usize]) -> usize {
    index - removed.partition_point(|gone| *gone < index)
}

/// Where the entity at `index` stands once entities stand at `inserted`, ascending.
fn after_insertion(index: usize, inserted: &[usize]) -> usize {
    (inserted.iter()).fold(index, |at, put| if at >= *put { at + 1 } else { at })
}

/// `index` as a LOD link states it.
fn link(index: usize) -> Result<i32, String> {
    i32::try_from(index).map_err(|_| format!("entity {index} is past what a LOD link can name"))
}

/// The entity `entity` names as its LOD parent in its own map file, when it names one that is not itself.
fn local_parent(entity: &CEntityDefAny, index: usize) -> Option<usize> {
    usize::try_from(entity.parent_index)
        .ok()
        .filter(|parent| *parent != index && !in_parent_map(entity.flags))
}

/// Refuse to take out or put in entities of `map` from the index `lowest` up while a container LOD of the file names an entity there.
///
/// ⚠ A container LOD states the index of an entity, which no operation renumbers: an entity at or after `lowest` moves or goes.
fn container_lods_hold(map: &CMapData, lowest: usize) -> Result<(), String> {
    let Some(Value::Array(lods)) = map.carried("containerLods") else {
        return Ok(());
    };
    let parent_index = or_world::field::name_hash("parentIndex");
    let moved = lods.iter().find_map(|lod| {
        let named = match lod {
            Value::Struct(lod) | Value::Pointer(Some(lod)) => {
                or_world::field::i32_at(lod, parent_index)
            }
            _ => None,
        };
        match named.map(usize::try_from) {
            // A container LOD that names no entity is moved by nothing.
            Some(Err(_)) => None,
            Some(Ok(named)) if named < lowest => None,
            Some(Ok(named)) => Some(format!("entity {named}")),
            None => Some("an entity this build does not read".to_string()),
        }
    });
    match moved {
        Some(named) => Err(format!(
            "the map carries container LODs, whose indexes no operation renumbers, and one names {named}, which would move or go"
        )),
        None => Ok(()),
    }
}

/// Take the entities `named` out of `map` with one renumbering, and say what puts them back; a refusal leaves `map` as it was.
fn remove_entities(
    map: &mut CMapData,
    named: &[usize],
) -> Result<(Vec<Inserted>, Vec<Counted>), String> {
    let removed = ascending(named)?;
    let held = map.entities.len();
    if let Some(past) = removed.iter().find(|index| **index >= held) {
        return Err(format!("entity {past} is past the {held} the map holds"));
    }
    container_lods_hold(map, removed[0])?;
    let goes = |index: usize| removed.binary_search(&index).is_ok();
    // The entities that stay and name one that goes, by where they stand afterwards.
    let mut orphans: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, entity) in map.entities.iter().enumerate() {
        if goes(index) {
            continue;
        }
        if let Some(parent) = local_parent(entity, index).filter(|parent| goes(*parent)) {
            (orphans.entry(parent).or_default()).push(after_removal(index, &removed));
        }
    }
    // ⚠ Every record is read before the first change: a session keeps the map an operation was refused on.
    let mut inserted = Vec::with_capacity(removed.len());
    for &index in &removed {
        inserted.push(Inserted {
            entity: index,
            record: Record::entity(&map.entities[index])?,
            children: orphans.remove(&index).unwrap_or_default(),
        });
    }
    // The parents that stay, with the children each counted before one of them went.
    let mut counted: BTreeMap<usize, u32> = BTreeMap::new();
    for &index in &removed {
        let parent = local_parent(&map.entities[index], index).filter(|parent| !goes(*parent));
        if let Some((parent, stays)) = parent.and_then(|p| Some((p, map.entities.get_mut(p)?))) {
            counted.entry(parent).or_insert(stays.num_children);
            stays.num_children = stays.num_children.saturating_sub(1);
        }
    }
    for (index, entity) in map.entities.iter_mut().enumerate() {
        if goes(index) || in_parent_map(entity.flags) {
            continue;
        }
        let Ok(parent) = usize::try_from(entity.parent_index) else {
            continue;
        };
        // A link to an entity that stays names one no later than it did, which a link can state.
        entity.parent_index = match link(after_removal(parent, &removed)) {
            Ok(stated) if !goes(parent) => stated,
            _ => -1,
        };
    }
    let mut index = 0;
    map.entities.retain(|_| {
        index += 1;
        !goes(index - 1)
    });
    let counted = (counted.into_iter())
        .map(|(entity, num_children)| Counted {
            entity,
            num_children,
        })
        .collect();
    Ok((inserted, counted))
}

/// Put the entities `named` into `map` at the indexes they state, with one renumbering, and state the counts of `counted`; the indexes they stand at. A refusal leaves `map` as it was.
fn insert_entities(
    map: &mut CMapData,
    named: &[Inserted],
    counted: &[Counted],
) -> Result<Vec<usize>, String> {
    let mut sorted: Vec<&Inserted> = named.iter().collect();
    sorted.sort_by_key(|put| put.entity);
    let inserted = ascending(&sorted.iter().map(|put| put.entity).collect::<Vec<_>>())?;
    let held = map.entities.len();
    for (earlier, index) in inserted.iter().enumerate() {
        if *index > held + earlier {
            return Err(format!(
                "entity {index} is past the end of the {} the map holds by then",
                held + earlier
            ));
        }
    }
    let total = held + inserted.len();
    link(total)?;
    container_lods_hold(map, inserted[0])?;
    let records = (sorted.iter())
        .map(|put| put.record.as_entity())
        .collect::<Result<Vec<_>, _>>()?;
    let children = sorted.iter().flat_map(|put| &put.children);
    if let Some(past) = children.clone().find(|child| **child >= held) {
        return Err(format!("child {past} is past the {held} the map holds"));
    }
    if let Some(past) = counted.iter().find(|count| count.entity >= total) {
        return Err(format!(
            "entity {} is past the {total} the map holds once they are in",
            past.entity
        ));
    }
    for (index, record) in inserted.iter().zip(&records) {
        links_hold(record, *index, total)?;
    }
    // ⚠ Every refusal comes before the first change: a session keeps the map an operation was refused on.
    let mut renumbered = Vec::new();
    for (at, entity) in map.entities.iter().enumerate() {
        if in_parent_map(entity.flags) {
            continue;
        }
        if let Ok(parent) = usize::try_from(entity.parent_index) {
            renumbered.push((at, link(after_insertion(parent, &inserted))?));
        }
    }
    for (at, stated) in renumbered {
        map.entities[at].parent_index = stated;
    }
    for put in &sorted {
        for child in &put.children {
            // An index of an entity that goes in, which `total` showed a link can state.
            map.entities[*child].parent_index = link(put.entity)?;
        }
    }
    for (index, record) in inserted.iter().zip(records) {
        map.entities.insert(*index, record);
    }
    for count in counted {
        map.entities[count.entity].num_children = count.num_children;
    }
    Ok(inserted)
}

/// Renumber the links of `map` into its parent map by `moved`, which is given each index a link names; how many links there are. A refusal leaves `map` as it was.
fn renumber_into_parent(
    map: &mut CMapData,
    moved: impl Fn(usize, usize) -> Result<usize, String>,
) -> Result<usize, String> {
    let mut renumbered = Vec::new();
    for (index, entity) in map.entities.iter().enumerate() {
        if !in_parent_map(entity.flags) {
            continue;
        }
        if let Ok(parent) = usize::try_from(entity.parent_index) {
            renumbered.push((index, link(moved(index, parent)?)?));
        }
    }
    let links = renumbered.len();
    for (index, stated) in renumbered {
        map.entities[index].parent_index = stated;
    }
    Ok(links)
}

/// Refuse `record` at `index` of a map of `len` entities when the LOD parent it names in this map is itself or past the entities: an index the game trusts, which a save refuses.
fn links_hold(record: &CEntityDefAny, index: usize, len: usize) -> Result<(), String> {
    if in_parent_map(record.flags) {
        return Ok(());
    }
    match usize::try_from(record.parent_index) {
        Ok(parent) if parent == index => Err(format!("entity {index} would be its own LOD parent")),
        Ok(parent) if parent >= len => Err(format!(
            "entity {index} would name LOD parent {parent}, past the {len} entities of the map"
        )),
        _ => Ok(()),
    }
}

/// The names of the classes `value` holds a record of.
fn classes(value: &Value, into: &mut BTreeSet<String>) {
    match value {
        Value::Struct(inst) | Value::Pointer(Some(inst)) => {
            into.insert(inst.type_name.clone());
            inst.fields.iter().for_each(|(_, held)| classes(held, into));
        }
        Value::Array(items) => items.iter().for_each(|item| classes(item, into)),
        Value::Map(pairs) => pairs.iter().for_each(|(key, held)| {
            classes(key, into);
            classes(held, into);
        }),
        _ => {}
    }
}

/// Whether the file still holds a record of every class of the records `taken` of `held` once those are taken out, or `replacing` stands in their place.
///
/// ⚠ A file read again from the XML an operation wrote names the classes its records are of and saves no record of another, so the
/// record that was the last of its class is not put back there, though it is on the file the operation was done to.
fn classes_stay<E>(
    held: &[E],
    taken: &[usize],
    replacing: Option<&Record>,
    record: fn(&E) -> Result<Record, String>,
) -> bool {
    let mut needed = BTreeSet::new();
    for index in taken {
        // What the operation refuses needs no inverse.
        let Some(Ok(taken)) = held.get(*index).map(record) else {
            return true;
        };
        classes(&taken.0, &mut needed);
    }
    let mut stay = BTreeSet::new();
    if let Some(replacing) = replacing {
        classes(&replacing.0, &mut stay);
    }
    // The other records are read only until every class is found, which among records of one class is the first of them.
    let mut others = (held.iter().enumerate()).filter(|(at, _)| !taken.contains(at));
    while !needed.is_subset(&stay) {
        match others.next() {
            Some((_, other)) => {
                if let Ok(other) = record(other) {
                    classes(&other.0, &mut stay);
                }
            }
            None => return false,
        }
    }
    true
}

/// Whether the inverse [`change_map`] gives for `operation` puts `map` back as it is with nothing to prove, given that `map` saves as it is.
///
/// ⚠ A map that saves has every LOD parent inside it, which is what makes a removal and an insertion each other's inverse. What is
/// not sure is proven by a save, as [`proven`] and [`restores`] do it: extents that are not finite, which an inverse could not state
/// again, an insertion that claims a child another entity has, or leaves its parent a count a removal would not bring back, an
/// entity taken out whose class may leave the file with it ([`classes_stay`]), the first entity of a map that holds none,
/// which is built here ([`first_entity`]) and which only a save shows the file to lay out, and several entities put in at once,
/// whose children and counts nothing short of the removal that gave them shows to be the ones a removal brings back.
fn sure_in_map(map: &CMapData, operation: &Operation) -> bool {
    if Extents::of(map).finite().is_err() {
        return false;
    }
    match operation {
        Operation::AddEntity { .. } => return !map.entities.is_empty(),
        Operation::RemoveEntity { entity, .. } => {
            return classes_stay(&map.entities, &[*entity], None, Record::entity)
        }
        Operation::RemoveEntities { entities, .. } => {
            return classes_stay(&map.entities, entities, None, Record::entity)
        }
        Operation::InsertEntities { .. } => return false,
        Operation::ReplaceEntity { entity, record, .. } => {
            return classes_stay(&map.entities, &[*entity], Some(record), Record::entity)
        }
        _ => {}
    }
    let Operation::InsertEntity {
        entity,
        record,
        parent_num_children,
        children,
        ..
    } = operation
    else {
        return true;
    };
    // What the operation refuses needs no inverse.
    let Ok(inserted) = record.as_entity() else {
        return true;
    };
    let free = |child: &usize| {
        (map.entities.get(*child)).is_none_or(|e| !in_parent_map(e.flags) && e.parent_index == -1)
    };
    let parent = usize::try_from(inserted.parent_index)
        .ok()
        .filter(|parent| !in_parent_map(inserted.flags) && parent != entity)
        // The parent by its index before the insertion.
        .and_then(|parent| map.entities.get(parent - usize::from(parent > *entity)));
    let counted = parent.is_none_or(|parent| {
        parent_num_children
            .unwrap_or(parent.num_children)
            .saturating_sub(1)
            == parent.num_children
    });
    children.iter().all(free) && counted
}

/// Whether the inverse [`change_types`] gives for `operation` puts `types` back as it is with nothing to prove.
///
/// ⚠ An archetype is put back by an insertion or a replacement, and both refuse a name another archetype of the file declares; its
/// class may leave the file with it ([`classes_stay`]); and the first archetype of a file that holds none is built here
/// ([`first_archetype`]), which only a save shows the file to lay out.
fn sure_in_types(types: &CMapTypes, operation: &Operation) -> bool {
    let shared = |hash: u32, but: usize| {
        (types.archetypes.iter().enumerate()).any(|(at, a)| at != but && a.name.hash == hash)
    };
    let stay = |index: usize, replacing: Option<&Record>| {
        classes_stay(&types.archetypes, &[index], replacing, Record::archetype)
    };
    match operation {
        Operation::AddArchetype { .. } => !types.archetypes.is_empty(),
        Operation::RemoveArchetype { archetype, .. } => {
            (types.archetypes.get(*archetype))
                .is_none_or(|held| !shared(held.name.hash, *archetype))
                && stay(*archetype, None)
        }
        Operation::ReplaceArchetype {
            archetype, record, ..
        } => match (types.archetypes.get(*archetype), record.as_archetype()) {
            (Some(held), Ok(replacing)) => {
                (replacing.name.hash == held.name.hash || !shared(held.name.hash, *archetype))
                    && stay(*archetype, Some(record))
            }
            _ => true,
        },
        _ => true,
    }
}

/// Do `operation` to `map`; a refusal leaves `map` as it was.
fn change_map(
    map: &mut CMapData,
    operation: &Operation,
    context: &Context,
) -> Result<Changed, String> {
    let source = operation.source().to_string();
    let was = Extents::of(map);
    let grow = |map: &mut CMapData| {
        let without_box = edit::grow_extents(map, |hash| context.boxes.get(&hash).copied());
        if !without_box.is_empty() {
            let named: Vec<String> = (without_box.iter().take(3))
                .map(|hash| format!("`{}`", HashedString::from_hash(*hash).to_xml_text()))
                .collect();
            context.notes.borrow_mut().push(format!(
                "no type file of the pack declares {} archetype(s) the map places ({}{}), so the extents hold each such entity where it stands and as far as it states it is drawn, which is less than its box",
                without_box.len(),
                named.join(", "),
                if without_box.len() > named.len() { ", ..." } else { "" }
            ));
        }
    };
    match operation {
        Operation::PlaceEntity { entity, pose, .. } => {
            let record = Record::entity(entity_at(map, *entity)?)?;
            edit::place_entity(map, *entity, pose.checked()?)?;
            grow(map);
            let inverse = Operation::ReplaceEntity {
                source,
                expects: None,
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
            let added = if map.entities.is_empty() {
                if let (None, Some(at)) = (&context.entity, donor) {
                    entity_at(map, at.index)?;
                }
                map.entities
                    .push(first_entity(placed, pose, context.entity.as_ref())?);
                0
            } else {
                let copied = match (&context.entity, donor) {
                    (Some(entity), _) => entity.clone(),
                    (None, Some(at)) => entity_at(map, at.index)?.clone(),
                    (None, None) => map.entities[0].clone(),
                };
                edit::add_entity(map, &copied, placed, pose)?
            };
            grow(map);
            let inverse = Operation::RemoveEntity {
                source,
                expects: None,
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
            container_lods_hold(map, index)?;
            if let Some(extents) = extents {
                extents.finite()?;
            }
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
                expects: None,
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
            // ⚠ Every refusal comes before the first change: a session keeps the map an operation was refused on.
            links_hold(&inserted, index, held + 1)?;
            container_lods_hold(map, index)?;
            let parent = usize::try_from(inserted.parent_index)
                .ok()
                .filter(|_| !in_parent_map(inserted.flags));
            if parent_num_children.is_some() && parent.is_none() {
                return Err(
                    "the record names no LOD parent in this map to count children for".to_string(),
                );
            }
            if (map.entities.iter()).any(|e| !in_parent_map(e.flags) && e.parent_index == i32::MAX)
            {
                return Err(
                    "a LOD link past what an index can state cannot be renumbered".to_string(),
                );
            }
            if let Some(extents) = extents {
                extents.finite()?;
            }
            // What named an entity at or after the index names the one after it now.
            for e in map.entities.iter_mut() {
                if !in_parent_map(e.flags) && e.parent_index >= stated {
                    e.parent_index += 1;
                }
            }
            for child in children {
                map.entities[*child].parent_index = stated;
            }
            map.entities.insert(index, inserted);
            if let (Some(count), Some(parent)) = (
                parent_num_children,
                parent.and_then(|parent| map.entities.get_mut(parent)),
            ) {
                parent.num_children = *count;
            }
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::RemoveEntity {
                source,
                expects: None,
                entity: index,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, Some(index)))
        }
        Operation::RemoveEntities {
            entities, extents, ..
        } => {
            if let Some(extents) = extents {
                extents.finite()?;
            }
            let (inserted, num_children) = remove_entities(map, entities)?;
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::InsertEntities {
                source,
                expects: None,
                entities: inserted,
                num_children,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, None))
        }
        Operation::InsertEntities {
            entities,
            num_children,
            extents,
            ..
        } => {
            if let Some(extents) = extents {
                extents.finite()?;
            }
            let inserted = insert_entities(map, entities, num_children)?;
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::RemoveEntities {
                source,
                expects: None,
                entities: inserted,
                extents: was.unless_stated_by(map),
            };
            Ok((inverse, None))
        }
        Operation::SetNumChildren {
            entity,
            num_children,
            ..
        } => {
            let held = map.entities.len();
            let counting = (map.entities.get_mut(*entity))
                .ok_or_else(|| format!("entity {entity} is past the {held} the map holds"))?;
            let inverse = Operation::SetNumChildren {
                source,
                expects: None,
                entity: *entity,
                num_children: counting.num_children,
            };
            counting.num_children = *num_children;
            Ok((inverse, None))
        }
        Operation::RenumberForRemoved { removed, .. } => {
            let removed = ascending(removed)?;
            renumber_into_parent(map, |index, parent| {
                if removed.binary_search(&parent).is_ok() {
                    // ⚠ A link with nothing to name would be put back by no renumbering: the entity goes first.
                    return Err(format!(
                        "entity {index} names entity {parent} of the parent map as its LOD parent, which is taken out: take entity {index} out of this map first"
                    ));
                }
                Ok(after_removal(parent, &removed))
            })?;
            let inverse = Operation::RenumberForInserted {
                source,
                expects: None,
                inserted: removed,
            };
            Ok((inverse, None))
        }
        Operation::RenumberForInserted { inserted, .. } => {
            let inserted = ascending(inserted)?;
            renumber_into_parent(map, |_, parent| Ok(after_insertion(parent, &inserted)))?;
            let inverse = Operation::RenumberForRemoved {
                source,
                expects: None,
                removed: inserted,
            };
            Ok((inverse, None))
        }
        Operation::ReplaceEntity {
            entity,
            record,
            extents,
            ..
        } => {
            let before = Record::entity(entity_at(map, *entity)?)?;
            let replacing = record.as_entity()?;
            links_hold(&replacing, *entity, map.entities.len())?;
            if let Some(extents) = extents {
                extents.finite()?;
            }
            map.entities[*entity] = replacing;
            if let Some(extents) = extents {
                extents.state(map)?;
            }
            let inverse = Operation::ReplaceEntity {
                source,
                expects: None,
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
                    expects: None,
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
                    expects: None,
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

/// Do `operation` to `types`; a refusal leaves `types` as it was.
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
            let added = match (types.archetypes.is_empty(), &copied) {
                (true, Some(like)) => {
                    edit::add_new_archetype(types, first_archetype(like, name, asset.as_deref())?)?
                }
                _ => edit::add_archetype(types, copied.as_ref(), name, asset.as_deref())?,
            };
            let inverse = Operation::RemoveArchetype {
                source,
                expects: None,
                archetype: added,
            };
            Ok((inverse, Some(added)))
        }
        Operation::RemoveArchetype { archetype, .. } => {
            let record = Record::archetype(archetype_at(types, *archetype)?)?;
            edit::remove_archetype(types, *archetype)?;
            let inverse = Operation::InsertArchetype {
                source,
                expects: None,
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
                expects: None,
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
                expects: None,
                archetype: *archetype,
                record: before,
            };
            Ok((inverse, None))
        }
        Operation::PlaceEntity { .. }
        | Operation::AddEntity { .. }
        | Operation::RemoveEntity { .. }
        | Operation::InsertEntity { .. }
        | Operation::RemoveEntities { .. }
        | Operation::InsertEntities { .. }
        | Operation::SetNumChildren { .. }
        | Operation::RenumberForRemoved { .. }
        | Operation::RenumberForInserted { .. }
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
///
/// ⚠ `after` is the root the change was made to, which still holds what the file it was read from declared: an entity goes back into
/// that root, and into no map file read again with none where no schema snapshot is staged. [`restores`] is the proof on that file.
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

/// Whether `inverse`, done to the file `inputs` cook to once they hold `xml`, writes the XML `inputs` hold: what applying it to the source as it was written does.
///
/// ⚠ That file is read again from the XML, and is not the file [`proven`] did the inverse to: it names no class it holds no record of.
fn restores<R: File>(
    inputs: &Inputs,
    xml: &[u8],
    inverse: &Operation,
    change: fn(&mut R, &Operation, &Context) -> Result<Changed, String>,
) -> bool {
    let mut written = inputs.clone();
    written.set_xml(xml.to_vec());
    let saved = (written.compile().ok())
        .and_then(|cooked| R::from_bytes(&cooked.bytes).ok())
        .and_then(|mut back| {
            change(&mut back, inverse, &Context::default()).ok()?;
            back.to_bytes().ok()
        });
    saved.is_some_and(|saved| {
        written
            .xml_of(&saved)
            .is_ok_and(|back| back == inputs.xml())
    })
}

/// Do `operation` to the file `inputs` cook to; the XML that makes them cook to the result, the inverse and the index of what was added.
fn through<R: File + Clone>(
    inputs: &Inputs,
    operation: &Operation,
    context: &Context,
    change: fn(&mut R, &Operation, &Context) -> Result<Changed, String>,
    sure: fn(&R, &Operation) -> bool,
) -> Result<(Vec<u8>, Operation, Option<usize>), String> {
    let cooked = inputs.compile().map_err(problems)?.bytes;
    let mut root = R::from_bytes(&cooked).map_err(|e| e.to_string())?;
    // What the file saves as before the change: what an inverse has to save again.
    let untouched = root.to_bytes().ok();
    let sure = sure(&root, operation);
    let (inverse, added) = change(&mut root, operation, context)?;
    let bytes = root.to_bytes().map_err(|e| e.to_string())?;
    let xml = inputs.xml_of(&bytes).map_err(problems)?;
    // ⚠ An inverse is proven on the root the change was made to, and, unless it holds by construction, on the file the new XML cooks
    // to as well: that root keeps the type table of the file it was read from, which states a class the cooked file loses with its
    // last element.
    let inverse = match untouched
        .filter(|was| inputs.is_the_export_of(was))
        .and_then(|was| proven(&root, &inverse, &was, change))
        .filter(|inverse| sure || restores(inputs, &xml, inverse, change))
    {
        Some(inverse) => inverse,
        None => Operation::RewriteSource {
            source: operation.source().to_string(),
            expects: None,
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

/// The folder and the inputs of the source `source` of the pack `model` in `folder`; a name no set of the pack holds, or one that leaves its folder, is refused, so an operation reaches nothing else.
fn opened(folder: &Path, model: &Maps, source: &str) -> Result<(PathBuf, Inputs), String> {
    let named = (model.maps.iter()).any(|set| set.sources.iter().any(|s| s == source));
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
    let dir = folder.join(source.replace('/', std::path::MAIN_SEPARATOR_STR));
    let opened = or_source::asset::Source::open(&dir).map_err(|p| p.to_string())?;
    let inputs = Inputs::read(&opened).map_err(problems)?;
    Ok((dir, inputs))
}

/// Why `operation` was not done.
fn refused(operation: &Operation, why: String) -> Error {
    Error::Refused {
        name: operation.name(),
        at: operation.source().to_string(),
        why,
    }
}

/// Add the boxes of the archetypes `types` declares to `boxes`, a name declared already keeping the box it has.
fn declare(boxes: &mut Boxes, types: &CMapTypes) {
    let corner = |v: or_io::decl::Vec3| Vec3::new(v.x, v.y, v.z);
    for archetype in &types.archetypes {
        boxes.entry(archetype.name.hash).or_insert((
            corner(archetype.bb_min),
            corner(archetype.bb_max),
            archetype.lod_dist,
        ));
    }
}

/// The sentence an operation says of the type file `path`, which declares no box because it does not cook.
fn does_not_cook(path: &str, why: &str) -> String {
    format!(
        "the type file `{path}` does not cook, so the extents are grown without the boxes it declares: {why}"
    )
}

/// What a map file states of its place among the maps of a pack.
#[derive(Clone)]
struct Linked {
    /// The hash of its name, which a map under it states as its parent.
    name: u32,
    /// The hash of the name of the map it names as parent; nought for none.
    parent: u32,
    /// The index each of its LOD links into the parent map states.
    into_parent: Vec<usize>,
}

impl Linked {
    fn of(map: &CMapData) -> Self {
        Self {
            name: map.name.hash,
            parent: map.parent.hash,
            into_parent: (map.entities.iter())
                .filter(|entity| in_parent_map(entity.flags))
                .filter_map(|entity| usize::try_from(entity.parent_index).ok())
                .collect(),
        }
    }
}

/// The words by which a note of [`links_left`] names the operation that settles it: the renumbering `by` of the map file `other`.
fn renumbers(by: &str, other: &str) -> String {
    format!("`{by}` of `{other}` renumbers them")
}

/// A sentence for each other map file of the pack whose LOD links into the map `source` state an index at or after `lowest`, where an operation takes entities out of `source` (`removed`) or puts some in.
///
/// ⚠ Those indexes are the other file's to keep, and the operation edits one file: each names another entity than it did until a
/// renumbering of the other file follows, which is the entity it should name only where an insertion takes back a removal done alone.
fn links_left(
    pack: &mut impl Pack,
    source: &str,
    lowest: Option<usize>,
    removed: bool,
) -> Vec<String> {
    let Some(lowest) = lowest else {
        return Vec::new();
    };
    let Some(name) = pack.linked(source).map(|linked| linked.name) else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    for other in pack.sources().into_iter().filter(|other| other != source) {
        let Some(linked) = pack.linked(&other) else {
            continue;
        };
        let moved = (linked.into_parent.iter())
            .filter(|index| **index >= lowest)
            .count();
        if linked.parent != name || name == 0 || moved == 0 {
            continue;
        }
        let links = format!(
            "the map file `{other}` names {moved} LOD parent(s) in this map by an index at or after entity {lowest}, which this operation does not renumber"
        );
        notes.push(if removed {
            format!(
                "{links}, so each names another entity until {}: `openrage-cli maps delete` plans a removal with the links into it",
                renumbers("renumber_for_removed", &other)
            )
        } else {
            format!(
                "{links}, so each names the entity before the one it named, which is right only where this insertion takes back a removal done alone: otherwise each names another entity until {}",
                renumbers("renumber_for_inserted", &other)
            )
        });
    }
    notes
}

/// Take out of the notes of the operations of a list done whole what another operation of the list settles: the links a removal or an insertion left to a renumbering the list holds.
fn settle_notes<'a>(operations: &[Operation], notes: impl Iterator<Item = &'a mut Vec<String>>) {
    let settling: Vec<String> = (operations.iter())
        .filter(|operation| {
            matches!(
                operation,
                Operation::RenumberForRemoved { .. } | Operation::RenumberForInserted { .. }
            )
        })
        .map(|operation| renumbers(operation.name(), operation.source()))
        .collect();
    for notes in notes {
        notes.retain(|note| !settling.iter().any(|settled| note.contains(settled)));
    }
}

/// The sources of a pack as an operation reads them besides the file it edits.
trait Pack {
    /// The boxes of the archetypes that the type files of every set naming `source` declare, and a sentence for each type file that does not cook, which declares none.
    fn boxes(&mut self, source: &str) -> (Arc<Boxes>, Vec<String>);
    /// The entity `at` names.
    fn entity(&mut self, at: &At) -> Result<CEntityDefAny, String>;
    /// The archetype `at` names.
    fn archetype(&mut self, at: &At) -> Result<CBaseArchetypeDefAny, String>;
    /// Every source the pack names, once each.
    fn sources(&self) -> Vec<String>;
    /// What the map file `source` states of its place among the maps; none for a source that is no map file or does not cook.
    fn linked(&mut self, source: &str) -> Option<Linked>;
}

/// What `operation` reads besides the file it edits.
fn context(pack: &mut impl Pack, operation: &Operation) -> Result<Context, String> {
    let source = operation.source();
    let elsewhere = |donor: &Option<At>| donor.clone().filter(|at| at.source != source);
    let named = |at: &At, why: String| format!("the donor in `{}`: {why}", at.source);
    let mut context = Context::default();
    match operation {
        Operation::PlaceEntity { .. } | Operation::GrowExtents { .. } => {
            let (boxes, notes) = pack.boxes(source);
            context.boxes = boxes;
            context.notes = RefCell::new(notes);
        }
        Operation::AddEntity { donor, .. } => {
            let (boxes, notes) = pack.boxes(source);
            context.boxes = boxes;
            context.notes = RefCell::new(notes);
            if let Some(at) = elsewhere(donor) {
                context.entity = Some(pack.entity(&at).map_err(|why| named(&at, why))?);
            }
        }
        Operation::AddArchetype { donor, .. } => {
            if let Some(at) = elsewhere(donor) {
                context.archetype = Some(pack.archetype(&at).map_err(|why| named(&at, why))?);
            }
        }
        Operation::RemoveEntity { entity, .. } => {
            let left = links_left(pack, source, Some(*entity), true);
            context.notes = RefCell::new(left);
        }
        Operation::RemoveEntities { entities, .. } => {
            let lowest = entities.iter().min().copied();
            let left = links_left(pack, source, lowest, true);
            context.notes = RefCell::new(left);
        }
        Operation::InsertEntity { entity, .. } => {
            let left = links_left(pack, source, Some(*entity), false);
            context.notes = RefCell::new(left);
        }
        Operation::InsertEntities { entities, .. } => {
            let lowest = entities.iter().map(|put| put.entity).min();
            let left = links_left(pack, source, lowest, false);
            context.notes = RefCell::new(left);
        }
        Operation::SetNumChildren { .. }
        | Operation::RenumberForRemoved { .. }
        | Operation::RenumberForInserted { .. }
        | Operation::ReplaceEntity { .. }
        | Operation::SetExtents { .. }
        | Operation::RemoveArchetype { .. }
        | Operation::InsertArchetype { .. }
        | Operation::ReplaceArchetype { .. }
        | Operation::RewriteSource { .. } => {}
    }
    Ok(context)
}

/// Write the XML of each source of `changed` (its folder, the XML to write and the XML it holds); when one cannot be written, the ones written before it are put back.
fn write_all(changed: &[(&Path, &[u8], &[u8])]) -> Result<(), Error> {
    for (at, (dir, xml, _)) in changed.iter().enumerate() {
        if let Err(why) = or_source::metadata::write_xml(dir, xml) {
            let mut said = problems(why);
            for (earlier, _, was) in &changed[..at] {
                if let Err(left) = or_source::metadata::write_xml(earlier, was) {
                    said.push_str(&format!(
                        "; and {} could not be put back as it was: {}",
                        earlier.display(),
                        problems(left)
                    ));
                }
            }
            return Err(Error::Write(said));
        }
    }
    Ok(())
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
    /// What each map source read so far states of its place among the maps, until an operation changes it.
    linked: BTreeMap<String, Option<Linked>>,
}

impl<'a> Work<'a> {
    fn open(folder: &'a Path) -> Result<Self, Error> {
        Ok(Self {
            folder,
            model: Maps::read(folder)?,
            held: BTreeMap::new(),
            linked: BTreeMap::new(),
        })
    }

    /// The source `source`, opened once.
    fn hold(&mut self, source: &str) -> Result<&mut Held, String> {
        if !self.held.contains_key(source) {
            let (dir, inputs) = opened(self.folder, &self.model, source)?;
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

    /// The boxes of the archetypes that the type files of every set naming `source` declare, and a sentence for each type file that does not cook, which declares none.
    fn boxes(&mut self, source: &str) -> (Boxes, Vec<String>) {
        let paths: Vec<String> = (self.model.maps.iter())
            .filter(|set| set.sources.iter().any(|s| s == source))
            .flat_map(|set| set.sources.iter().cloned())
            .collect();
        let mut boxes = Boxes::new();
        let mut notes = Vec::new();
        for path in paths {
            // A source of another kind, a map file or a model, declares nothing and is no one's to mention.
            let is_types = (self.hold(&path))
                .is_ok_and(|held| held.inputs.extension() == CMapTypes::EXTENSION);
            if !is_types {
                continue;
            }
            match self.root::<CMapTypes>(&path) {
                Ok(types) => declare(&mut boxes, &types),
                Err(why) => notes.push(does_not_cook(&path, &why)),
            }
        }
        (boxes, notes)
    }

    /// Do `operation` to the XML the work holds of its source, once that XML has the digest the operation states.
    fn apply(&mut self, operation: &Operation) -> Result<Applied, Error> {
        let held = (self.hold(operation.source())).map_err(|why| refused(operation, why))?;
        if let Some(expects) = operation.expects() {
            let found = digest_of(held.inputs.xml());
            if found != expects {
                return Err(Error::Stale {
                    name: operation.name(),
                    at: operation.source().to_string(),
                    expects: expects.to_string(),
                    found,
                });
            }
        }
        (self.change(operation)).map_err(|why| refused(operation, why))
    }

    /// Do `operation` to the XML the work holds of its source.
    fn change(&mut self, operation: &Operation) -> Result<Applied, String> {
        let source = operation.source();
        let extension = self.hold(source)?.inputs.extension().to_string();
        let mut notes = Vec::new();
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
                    expects: None,
                    xml: was,
                };
                (xml.clone().into_bytes(), inverse, None)
            }
            (_, CMapData::EXTENSION) => {
                let context = context(self, operation)?;
                let inputs = &self.hold(source)?.inputs;
                let done =
                    through::<CMapData>(inputs, operation, &context, change_map, sure_in_map)?;
                notes = context.notes.into_inner();
                done
            }
            (_, CMapTypes::EXTENSION) => {
                let context = context(self, operation)?;
                let inputs = &self.hold(source)?.inputs;
                through::<CMapTypes>(inputs, operation, &context, change_types, sure_in_types)?
            }
            (_, other) => {
                return Err(format!(
                    "it is a .{other} source, and an operation edits a map file or a type file"
                ))
            }
        };
        self.linked.remove(source);
        let held = self.hold(source)?;
        let before = held.inputs.xml().to_vec();
        held.inputs.set_xml(xml.clone());
        Ok(Applied {
            inverse: inverse.expecting(digest_of(&xml)),
            edited: Edited {
                file: held.dir.join(or_source::metadata::XML),
                before,
                after: xml,
            },
            added,
            notes,
        })
    }

    /// Write the XML of every source the operations changed; when one cannot be written, the ones written before it are put back.
    fn commit(self) -> Result<(), Error> {
        let changed: Vec<(&Path, &[u8], &[u8])> = (self.held.values())
            .filter(|held| held.inputs.xml() != held.on_disk.as_slice())
            .map(|held| {
                (
                    held.dir.as_path(),
                    held.inputs.xml(),
                    held.on_disk.as_slice(),
                )
            })
            .collect();
        write_all(&changed)
    }
}

impl Pack for Work<'_> {
    fn boxes(&mut self, source: &str) -> (Arc<Boxes>, Vec<String>) {
        let (boxes, notes) = Work::boxes(self, source);
        (Arc::new(boxes), notes)
    }

    fn entity(&mut self, at: &At) -> Result<CEntityDefAny, String> {
        (self.root::<CMapData>(&at.source)).and_then(|map| entity_at(&map, at.index).cloned())
    }

    fn archetype(&mut self, at: &At) -> Result<CBaseArchetypeDefAny, String> {
        (self.root::<CMapTypes>(&at.source))
            .and_then(|types| archetype_at(&types, at.index).cloned())
    }

    fn sources(&self) -> Vec<String> {
        sources_of(&self.model)
    }

    fn linked(&mut self, source: &str) -> Option<Linked> {
        if let Some(known) = self.linked.get(source) {
            return known.clone();
        }
        let is_map =
            (self.hold(source)).is_ok_and(|held| held.inputs.extension() == CMapData::EXTENSION);
        let linked = (is_map.then(|| self.root::<CMapData>(source).ok()).flatten())
            .map(|map| Linked::of(&map));
        self.linked.insert(source.to_string(), linked.clone());
        linked
    }
}

/// Every source `model` names, once each.
fn sources_of(model: &Maps) -> Vec<String> {
    (crate::facts::named_paths(model).map(str::to_string)).collect()
}

/// Apply `operation` to the sources of the pack of maps in `folder`, and say what takes it back; a refusal leaves every source as it was.
pub fn apply(folder: &Path, operation: &Operation) -> Result<Applied, Error> {
    let mut work = Work::open(folder)?;
    let applied = work.apply(operation)?;
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
        let done = work.apply(operation).map_err(|refused| Error::InList {
            number: index + 1,
            of: operations.len(),
            refused: Box::new(refused),
        })?;
        applied.push(done);
    }
    if !dry_run {
        work.commit()?;
    }
    settle_notes(operations, applied.iter_mut().map(|done| &mut done.notes));
    Ok(applied)
}

/// The list that takes back the list whose operations did `applied`: the inverse of each, the last first.
pub fn inverses(applied: &[Applied]) -> Vec<Operation> {
    (applied.iter().rev())
        .map(|done| done.inverse.clone())
        .collect()
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
