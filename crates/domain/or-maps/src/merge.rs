//! Two copies of one map merged by object, with the file both were made from as the base: the game's file.
//!
//! [`merge_maps`] takes the base, ours and theirs as typed roots of one map and gives the merged root with a [`Report`], and
//! [`merge_types`] does it for the archetypes of a type file, an archetype being the one of its name. [`merge_files`] reads three
//! files and saves the result, and [`merge_source`] merges another copy into a metadata source, whose kept base is the base.
//!
//! What a side changed is what [`crate::changes`] says it changed, and nothing here compares two files another way. A change only
//! one side made is taken, a change both made to one value is one change, and the rest is a [`Conflict`], which the policy of the
//! caller decides (ours, theirs, or stop) and the report keeps with what was answered.
//!
//! ⚠ **A conflict is preferred to a guess.** An entity either comparison lists as ambiguous is never paired by this module: its
//! twins are one conflict, and a side is taken for all of them or the merge stops. An entity one side gave another archetype is
//! never merged member by member, since a guid that a deletion freed may have been taken by another entity. Two entities of one
//! archetype that both sides added at one position are kept once only when they state the same of everything and name no parent
//! and no child of their own file.
//!
//! ⚠ **The merged file states its links in its own indexes.** The entities both hold keep the order of the base, the entities a
//! side added follow them in an order that does not depend on which side is called ours, and the entities that are gone are taken
//! out by the operation that takes several out of a map at once, so the links after them are renumbered as an edit renumbers
//! them, and a merge that would move an entity a container LOD names is refused as that edit is. A count of children is the
//! count the side states less the children of its own file, plus the children of the merged file: what a child map adds to it is
//! kept. A child of twins names the twin its own side has it under when that side is taken for the twins; a link from the other
//! side to such a twin follows the order of the files alone, and a note of the report says so.
//!
//! ⚠ **When one side states what the base states, the result is the other side as it is**, saved byte for byte, with no guid
//! given anew and no extents grown, once every conflict (a twin that side moved) was answered with that side.
//!
//! What is not merged inside is said in [`Report::not_compared`]: what no typed member names is taken whole from the side that
//! changed it, the flags of a file are one number, and the extents are the union of what the sides state, grown without the box
//! of any archetype.
//!
//! ⚠ **What no typed member of the file names comes with the root it is in**, so a side that changed it gives the merged file its
//! container: one in another container than the base is refused, and where both state the same of it the side in the container of
//! the base is the one taken.
//!
//! The submodules are the parts of an entity (`entity`), the entities of a map (`map`), the archetypes of a type file (`types`)
//! and the files (`files`); what they share is here.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use or_io::decl::File;
use or_io::meta::typed::{CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapData, CMapTypes};
use or_io::meta::{detect_binary_format, BinaryFormat};
use serde::Serialize;

use crate::changes::{
    archetype_changes, entity_ref, link_of, map_changes, map_changes_paired, own_children,
    pose_key, types_changes, ArchetypeKept, ArchetypeRef, EntityChange, EntityRef, Kept, Link,
    MapChanges, MemberChange, SourceChanges, Stated, IN_PARENT_MAP,
};
use crate::edit::{pose_of, Edited};
use crate::facts::{Fact, Facts, Inside, SourceFact};
use crate::model::{MapSet, Maps};
use crate::operation::{apply_to_map, apply_to_types, Extents, Operation, Record};
use crate::rules::Severity;

mod entity;
mod files;
mod map;
mod types;

pub use files::{merge_files, merge_source, SourceMerge};
pub use map::merge_maps;
pub use types::merge_types;

/// One of the two copies that are merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Ours,
    Theirs,
}

impl Side {
    fn other(self) -> Self {
        match self {
            Self::Ours => Self::Theirs,
            Self::Theirs => Self::Ours,
        }
    }

    /// The word a sentence names it by.
    pub fn word(self) -> &'static str {
        match self {
            Self::Ours => "ours",
            Self::Theirs => "theirs",
        }
    }
}

/// What a policy answers a conflict with: the side whose copy is taken, or that the merge stops and makes no file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Take {
    Ours,
    Theirs,
    Stop,
}

/// Which side a change that was applied is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum By {
    Ours,
    Theirs,
    /// Both made it, to one value.
    Both,
}

impl From<Side> for By {
    fn from(side: Side) -> Self {
        match side {
            Side::Ours => Self::Ours,
            Side::Theirs => Self::Theirs,
        }
    }
}

/// An entity or an archetype as one of the three files indexes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Ref {
    Entity(EntityRef),
    Archetype(ArchetypeRef),
}

impl Ref {
    /// Its index in the file that holds it.
    pub fn index(&self) -> usize {
        match self {
            Self::Entity(entity) => entity.index,
            Self::Archetype(archetype) => archetype.index,
        }
    }

    /// What it is, for a person to find it: an entity by what it places, where, and its guid, an archetype by its name.
    pub fn described(&self) -> String {
        match self {
            Self::Entity(entity) => format!(
                "{} at {}, guid {}",
                entity.archetype.shown(),
                numbers(&entity.position),
                entity.guid
            ),
            Self::Archetype(archetype) => format!("the archetype {}", archetype.name.shown()),
        }
    }
}

/// What two sides disagree on.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Disagreement {
    /// Both changed one member of a record both hold, to different values.
    Member {
        member: String,
        ours: String,
        theirs: String,
    },
    /// Both changed a record that is not merged member by member, one having given it another archetype or another type, and they do not state the same: what each changed of it, by the names of the members.
    Record {
        ours: Vec<String>,
        theirs: Vec<String>,
    },
    /// One side changed what the other deleted: the members it changed, by their names.
    ChangedAndDeleted {
        deleted_by: Side,
        changed: Vec<String>,
    },
    /// Both added an entity of one archetype at one position, or an archetype of one name, and the two do not state the same: a likely double, with the members the two differ in.
    Double { differ: Vec<String> },
    /// A comparison could not tell these from a twin, so nothing says which is which.
    Ambiguous,
    /// Both changed one thing the file states of itself, to different values.
    File {
        member: String,
        ours: String,
        theirs: String,
    },
    /// Both changed the link of one entity into the parent map: a place among that map's entities, which a deletion there renumbers in every map under it, so neither number is known to name the right entity of the merged parent map, the same number least of all.
    ParentMapLink { ours: String, theirs: String },
}

/// One thing the two sides disagree on, with the records it is about as each file indexes them, and what the policy answered.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conflict {
    pub what: Disagreement,
    pub base: Vec<Ref>,
    pub ours: Vec<Ref>,
    pub theirs: Vec<Ref>,
    /// [`Take::Stop`] while the policy is asked.
    pub taken: Take,
}

/// What became of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Did {
    Changed,
    Deleted,
    Added,
}

/// A member, and the side its value is from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemberBy {
    /// The engine's name of the member, or the words for what no typed member names.
    pub member: String,
    pub by: By,
}

/// One change that was applied with no conflict: what a conflict decided is said by the conflict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Applied {
    pub did: Did,
    pub by: By,
    pub base: Option<Ref>,
    pub ours: Option<Ref>,
    pub theirs: Option<Ref>,
    /// Its index in the merged file, none for one deleted.
    pub merged: Option<usize>,
    /// The members of a changed record, each with the side it is from.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberBy>,
    /// The guid an added entity states in the merged map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guid: Option<u32>,
}

/// A finding of the rules about the merged file that does not refuse it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// What a merge did, as data.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// What ours changed of the base, as [`crate::changes`] says it.
    pub ours: SourceChanges,
    /// What theirs changed of the base.
    pub theirs: SourceChanges,
    pub applied: Vec<Applied>,
    /// What the file states of itself that a side changed, each member with the side it is from.
    pub file: Vec<MemberBy>,
    pub conflicts: Vec<Conflict>,
    /// Whether a merged file was made: false when a conflict stopped it.
    pub merged: bool,
    /// What the merge does not look into, a sentence each.
    pub not_compared: Vec<String>,
    pub notes: Vec<String>,
    /// What the rules warn of in the merged file.
    pub findings: Vec<Found>,
}

impl Report {
    /// How many conflicts the policy answered with [`Take::Stop`].
    pub fn stopped(&self) -> usize {
        (self.conflicts.iter())
            .filter(|c| c.taken == Take::Stop)
            .count()
    }
}

/// A merged file, none when a conflict stopped the merge, and what the merge did.
#[derive(Debug, Clone, PartialEq)]
pub struct Merge<T> {
    pub merged: Option<T>,
    pub report: Report,
}

/// Why nothing was merged.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{which} does not read as a {root}: {why}")]
    Read {
        which: &'static str,
        root: &'static str,
        why: String,
    },
    #[error("a .{0} file is neither a map file nor a type file, and a merge is of one of those")]
    Kind(String),
    #[error("{which} cannot be merged: {why}")]
    Input { which: &'static str, why: String },
    #[error("the merged file would not be valid, so nothing was merged: {0}")]
    Invalid(String),
    #[error("{0}")]
    Source(String),
    #[error("the merged file was not written to the source: {0}")]
    Write(String),
}

/// The words for what a record holds that no typed member names.
const OTHER: &str = "what no typed member names";

const PARENT: &str = "parentIndex";

const COUNT: &str = "numChildren";

/// The members of every entity that the comparison names in a list.
const NAMED: [&str; 4] = [
    "childLodDist",
    "ambientOcclusionMultiplier",
    "artificialAmbientOcclusion",
    "tintValue",
];

/// The members of an interior instance.
const OF_INTERIOR: [&str; 5] = [
    "groupId",
    "floorId",
    "defaultEntitySets",
    "numExitPortals",
    "MLOInstflags",
];

fn numbers(values: &[f32]) -> String {
    let said: Vec<String> = values.iter().map(ToString::to_string).collect();
    format!("({})", said.join(", "))
}

fn stated(value: &Stated) -> String {
    match value {
        Stated::Float(value) => value.to_string(),
        Stated::Number(value) => value.to_string(),
        Stated::Vector(value) => numbers(value),
        Stated::Name(name) => name.shown(),
        Stated::Text(text) if text.is_empty() => "(none)".to_string(),
        Stated::Text(text) => text.clone(),
    }
}

/// The policy, and what it answered.
struct Asked<'p> {
    policy: &'p mut dyn FnMut(&Conflict) -> Take,
    conflicts: Vec<Conflict>,
    stopped: bool,
}

impl Asked<'_> {
    /// The side the policy takes for this conflict; ours while the rest is looked at, once it answered stop.
    fn ask(
        &mut self,
        what: Disagreement,
        base: Vec<Ref>,
        ours: Vec<Ref>,
        theirs: Vec<Ref>,
    ) -> Side {
        let mut conflict = Conflict {
            what,
            base,
            ours,
            theirs,
            taken: Take::Stop,
        };
        conflict.taken = (self.policy)(&conflict);
        let side = match conflict.taken {
            Take::Ours => Side::Ours,
            Take::Theirs => Side::Theirs,
            Take::Stop => {
                self.stopped = true;
                Side::Ours
            }
        };
        self.conflicts.push(conflict);
        side
    }

    /// Whether every conflict so far was answered with `side`.
    fn all_took(&self, side: Side) -> bool {
        let take = match side {
            Side::Ours => Take::Ours,
            Side::Theirs => Take::Theirs,
        };
        self.conflicts.iter().all(|c| c.taken == take)
    }
}

/// Which of two sides changed a thing: the side its value is from, and none when no conflict decided that, else who made the change.
fn three_way(
    ours: bool,
    theirs: bool,
    same: impl FnOnce() -> bool,
    ask: impl FnOnce() -> Side,
) -> Option<(Side, Option<By>)> {
    match (ours, theirs) {
        (false, false) => None,
        (true, false) => Some((Side::Ours, Some(By::Ours))),
        (false, true) => Some((Side::Theirs, Some(By::Theirs))),
        (true, true) if same() => Some((Side::Ours, Some(By::Both))),
        (true, true) => Some((ask(), None)),
    }
}

/// The findings of the rules that judge `bytes`, a map file or a type file, by itself; an error among them is the refusal.
fn judged(bytes: &[u8], extension: &str) -> Result<Vec<Found>, Error> {
    const ALONE: [&str; 8] = [
        "file-unreadable",
        "hierarchy-broken",
        "interior-broken",
        "guid-twice",
        "entity-twice",
        "extents-short",
        "archetype-twice",
        "bounds-inverted",
    ];
    let inside: Result<Inside, String> = crate::facts::inside_of(bytes, extension, None);
    let path = "merged";
    let mut set = MapSet::new(path);
    set.sources.push(path.to_string());
    let mut model = Maps::new();
    model.maps.push(set);
    let mut facts = Facts::default();
    facts.insert(
        path,
        Fact::Source(Box::new(SourceFact {
            kind: or_source::metadata::KIND.to_string(),
            name: path.to_string(),
            extension: extension.to_string(),
            bytes: bytes.len() as u64,
            game: Vec::new(),
            digest: None,
            inside: Some(inside),
        })),
    );
    let found: Vec<Found> = (crate::rules::check(&model, &facts, None).into_iter())
        .filter(|finding| ALONE.contains(&finding.code))
        .map(|finding| Found {
            code: finding.code,
            severity: finding.severity,
            message: finding.message,
        })
        .collect();
    let errors: Vec<&str> = (found.iter())
        .filter(|f| f.severity == Severity::Error)
        .map(|f| f.message.as_str())
        .collect();
    if errors.is_empty() {
        Ok(found)
    } else {
        Err(Error::Invalid(errors.join("; ")))
    }
}

/// The first of the base, ours and theirs that the rules refuse by itself, as the refusal of a merge names it: a merged file is
/// then refused for what that file holds, and the person is told which file to look into.
fn refused_of_itself<T: File>(files: [(&'static str, &T); 3]) -> Option<Error> {
    files.into_iter().find_map(
        |(which, root)| match judged(&root.to_bytes().ok()?, T::EXTENSION) {
            Err(Error::Invalid(why)) => Some(Error::Input { which, why }),
            _ => None,
        },
    )
}

/// The container a root saves in, none for one that does not save.
fn container_of<T: File>(root: &T) -> Option<BinaryFormat> {
    detect_binary_format(&root.to_bytes().ok()?)
}

/// The side whose root the merged file is made of, which is the side whose copy of what no typed member of the file names is
/// taken, and the refusal of a side that is in another container than the base.
///
/// ⚠ What no typed member names cannot be put into another root, so the merged file is saved in the container of the root it
/// came with. Where both sides state the same of it, the side in the container of the base is the one taken, so that the two
/// orders of a merge give one file.
fn root_from<T: File>(
    other: Option<(Side, Option<By>)>,
    base: &T,
    sides: [&T; 2],
) -> Result<Option<(Side, Option<By>)>, Error> {
    let Some((mut side, by)) = other else {
        return Ok(None);
    };
    let of_base = container_of(base);
    let like_base = |side: Side| container_of(sides[side as usize]) == of_base;
    if by == Some(By::Both) && !like_base(side) && like_base(side.other()) {
        side = side.other();
    }
    if like_base(side) {
        return Ok(Some((side, by)));
    }
    let named = |format: Option<BinaryFormat>| format.map_or("no file", |f| f.label());
    Err(Error::Input {
        which: side.word(),
        why: format!(
            "it changed {OTHER} of the file and is a {} file where the base is a {} file: the merged file would be saved as {} states it, and not in the container of the base",
            named(container_of(sides[side as usize])),
            named(of_base),
            side.word()
        ),
    })
}

fn not_compared(entities: bool) -> Vec<String> {
    let mut said = vec![format!(
        "{OTHER} of the file, such as a car generator, an occluder or an extension, is taken whole from the side that changed it and is not merged inside"
    )];
    if entities {
        said.push(format!(
            "{OTHER} of an entity is taken whole from the side that changed it"
        ));
        said.push(
            "the flags and the content flags are each one number, taken from the side that changed it, and are not derived from the entities again"
                .to_string(),
        );
        said.push(
            "the extents are not merged: the merged map states the union of what the two sides state, grown to hold each entity where it stands and as far as it states it is drawn, with the box of no archetype"
                .to_string(),
        );
        said.push(
            "a link into the parent map is the number a side states, and no other map of the group is read: two copies of which either deleted an entity with its chain of far models are not merged file by file, since that deletion renumbers the links of every map under the parent map"
                .to_string(),
        );
    } else {
        said.push(
            "the entities, the rooms and the portals of an interior are taken whole from the side that changed them"
                .to_string(),
        );
    }
    said
}

/// The members of a file's own that both sides may change, merged one by one; the side each changed member is from.
fn merge_file_members(
    members: &[&'static str],
    ours: &[MemberChange],
    theirs: &[MemberChange],
    asked: &mut Asked<'_>,
) -> Vec<(&'static str, Side, Option<By>)> {
    let mut taken = Vec::new();
    for &member in members {
        let of = |root: &[MemberChange]| root.iter().find(|c| c.member == member).cloned();
        let (a, b) = (of(ours), of(theirs));
        let said = |change: &Option<MemberChange>| {
            change
                .as_ref()
                .map_or(String::new(), |change| stated(&change.to))
        };
        let from = three_way(
            a.is_some(),
            b.is_some(),
            || a.as_ref().map(|c| &c.to) == b.as_ref().map(|c| &c.to),
            || {
                let what = Disagreement::File {
                    member: member.to_string(),
                    ours: said(&a),
                    theirs: said(&b),
                };
                asked.ask(what, Vec::new(), Vec::new(), Vec::new())
            },
        );
        if let Some((side, by)) = from {
            taken.push((member, side, by));
        }
    }
    taken
}

/// What a side changed of the file's own with no conflict, as the report lists it.
fn file_said(
    members: &[(&'static str, Side, Option<By>)],
    other: Option<(Side, Option<By>)>,
) -> Vec<MemberBy> {
    (members.iter())
        .map(|(member, _, by)| (member.to_string(), *by))
        .chain(other.map(|(_, by)| (OTHER.to_string(), by)))
        .filter_map(|(member, by)| Some(MemberBy { member, by: by? }))
        .collect()
}

/// The side whose copy of what no typed member of the file names is taken, none when neither changed it.
fn other_from(
    ours: bool,
    theirs: bool,
    same: impl FnOnce() -> bool,
    asked: &mut Asked<'_>,
) -> Option<(Side, Option<By>)> {
    three_way(ours, theirs, same, || {
        let what = Disagreement::File {
            member: OTHER.to_string(),
            ours: "changed".to_string(),
            theirs: "changed".to_string(),
        };
        asked.ask(what, Vec::new(), Vec::new(), Vec::new())
    })
}
