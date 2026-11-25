//! The first edit of an object the game placed: the map file that holds it comes into the project as a source that keeps the game's
//! file as its base, the edit is applied, and the two are one [`Record`] that is taken back whole.
//!
//! An [`Edit`] names an entity by the logical path of its map file in the mounted install, or by a source of the content, and by its
//! index or its guid there. [`plan`] says what it would import and do, and writes nothing. [`apply`] imports each map file the edit
//! touches that the content does not hold yet (for a move the one map, for a deletion every file its chain of far models changes),
//! through [`crate::import::add_one`], tells the content of them, and applies the operations through a
//! [`Session`](crate::operation::Session) that writes once. [`take_back`] applies the inverses of a record and removes the sources
//! it made. An entity of a map the content already holds, a map a person made or one imported before, is edited there and nothing is
//! imported.
//!
//! This crate sees no mount, so [`Game`] is what the caller that has one answers: the bytes at a logical path, where in the install's
//! archives that file lies, the map file of a name, and the map files under a map.
//!
//! ⚠ **Nothing stays written when any part is refused.** What can be known before a write is asked first: the entity, the edition,
//! the chain. A map file that cannot become a source is refused by the import itself, and the sources made before it in the same call
//! are removed; an operation or a flush refused after the import removes them too and puts the content's `maps.json` back.
//!
//! ⚠ **A record is taken back in order.** Its inverses state the digest of what the edit wrote, so a record is refused once a later
//! edit changed one of its sources, until that edit is taken back first; and a source it made is removed only when, the inverses
//! applied, it is again byte for byte what the import made.
//!
//! ⚠ **A map is shipped as it is and never converted**, so a map file of an install of the other edition than a build of the content
//! targets is refused before it is opened.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;
use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_project::{Container, Module, Target};
use or_source::asset::{folder_name, sha256_hex, AssetManifest, MANIFEST};
use or_source::game_import::{tree_digest, Created};
use serde::{Deserialize, Serialize};

use crate::delete::{self, Beyond, Deletion, Reached};
use crate::edit::{pose_of, Pose};
use crate::facts::{named_paths, stays_inside};
use crate::import::{self, Incoming};
use crate::model::{self, Maps};
use crate::operation::{self, digest_of, At, Done, Operation, PoseData, Session, Written};

/// The file a record states it is.
pub const FORMAT: &str = "openrage-first-edit";
/// The one schema of a record this build reads and writes.
pub const SCHEMA: u32 = 1;

/// The English texts a first edit is told with, each the key a catalog of the app is to translate it by; `{name}` stands for an argument. ⚠ No catalog holds one yet, and no screen shows them: the command line prints the English.
pub mod told {
    pub const MOVED: &str = "Moved an object of {map}";
    /// One entity gone, and several: the object with the far models that stood for it alone.
    pub const DELETED: (&str, &str) = (
        "Deleted an object of {map}",
        "Deleted an object of {map} with its far models: {count} entities",
    );
    /// One map brought in, and several.
    pub const IMPORTED: (&str, &str) = (
        "{map} is now a map of this project",
        "{count} maps of the game are now maps of this project",
    );
    /// What a map taken from the game is to a repository of a project that keeps the default commit policy.
    pub const LEFT_OUT: (&str, &str) = (
        "It is a source taken from the game, which is the game's own content. The project's Git ignore rules leave it out and Check before sharing lists it, so someone who clones the project gets it without this change until Commit sources taken from the game is turned on in the project's Properties",
        "They are sources taken from the game, which are the game's own content. The project's Git ignore rules leave them out and Check before sharing lists them, so someone who clones the project gets it without these changes until Commit sources taken from the game is turned on in the project's Properties",
    );
    /// The same, of a project whose owner turned on Commit sources taken from the game.
    pub const COMMITTED: (&str, &str) = (
        "It is a source taken from the game, which is the game's own content. This project commits such sources, so a repository of it holds the game's file, and Check before sharing still lists it",
        "They are sources taken from the game, which are the game's own content. This project commits such sources, so a repository of it holds the game's files, and Check before sharing still lists them",
    );
    pub const UNDO: &str = "Undo";
}

/// What a first edit reads of the mounted install, answered by the caller that has the mount.
pub trait Game {
    /// The edition of the install.
    fn edition(&self) -> Target;

    /// The whole of the file at the logical path `logical`; none where the mount holds no such file or cannot read it.
    fn read(&self, logical: &str) -> Option<Vec<u8>>;

    /// The path inside the install's archives of the file at `logical`, which a source made of it states as where it was taken from; none where the mount cannot say.
    ///
    /// ⚠ Not the logical path: the check against the game reads a source's original at the path its manifest states, below the install's folder.
    fn lies_at(&self, logical: &str) -> Option<String>;

    /// The logical path of the map file the mount loads for the map called `name`, by the hash of its name.
    fn map_file(&self, name: u32) -> Option<String>;

    /// The logical paths of the map files whose map names `parent` as its parent map.
    fn maps_under(&self, parent: u32) -> Vec<String>;
}

/// The maps content of a project an edit goes to.
#[derive(Debug, Clone, Copy)]
pub struct Place<'a> {
    /// The project file.
    pub project: &'a Path,
    pub container: &'a Container,
    pub module: &'a Module,
}

/// Which entity of a map file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Which {
    /// By its place among the entities of the file.
    Index(usize),
    /// By the guid it states, which names it only while no other entity of the file states the same.
    Guid(u32),
}

/// What to do to the entity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    /// Stand it at `position`, with the turn and the scale it has.
    Move { position: [f32; 3] },
    /// Take it out with its chain of far models, as [`crate::delete`] plans it.
    Delete,
}

/// An edit of one entity, addressed where the game or the content holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    /// The logical path of the map file in the mounted install, or a source of the content as its `maps.json` names it.
    pub map: String,
    pub entity: Which,
    pub change: Change,
}

/// Why a first edit was not planned, not done or not taken back.
#[derive(Debug, thiserror::Error)]
pub enum Refusal {
    #[error("{0}")]
    Folder(#[from] crate::plan::Error),
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("the install holds no file at `{map}`, and no source of the content is called that")]
    NoSuchMap { map: String },
    #[error("`{file}` is not a map file, and an object the game placed is an entity of a .ymap")]
    NotAMap { file: String },
    #[error("`{file}` does not read as a map file, so nothing was imported and no source was changed: {why}")]
    Unread { file: String, why: String },
    #[error("`{file}` cannot become a source of the project, so nothing was imported and no source was changed: {why}")]
    NotASource { file: String, why: String },
    #[error(
        "`{file}` is a map of a GTA V {install} install, and the build `{build}` of this content is for GTA V {target}. A map ships as it is and is never converted, so edit this object from an install of GTA V {target}. Nothing was imported and no source was changed"
    )]
    OtherEdition {
        file: String,
        /// The edition of the install, as a person reads it.
        install: &'static str,
        /// The name of the build that takes the content.
        build: String,
        target: &'static str,
    },
    #[error(
        "the content holds `{source_there}`, which is named as the source of `{file}` would be and was not taken from it. If it is the content's own map of that name, edit the object there; if it is not, move it out of the way. Nothing was imported and no source was changed"
    )]
    InTheWay { file: String, source_there: String },
    #[error(
        "`{file}` is not the copy of that map the install loads: a later archive holds the map again, at `{loaded}`, and a source made of this copy would put back what the install no longer loads. Name the object in `{loaded}`. Nothing was imported and no source was changed"
    )]
    NotTheCopyLoaded { file: String, loaded: String },
    #[error("`{map}` holds {len} entities, so there is no entity {index}")]
    NoSuchEntity {
        map: String,
        index: usize,
        len: usize,
    },
    #[error("no entity of `{map}` states the guid {guid}")]
    NoSuchGuid { map: String, guid: u32 },
    #[error(
        "entities {} of `{map}` state the guid {guid}, so the guid does not say which one is meant: name the entity by its index",
        list(.entities)
    )]
    AmbiguousGuid {
        map: String,
        guid: u32,
        entities: Vec<usize>,
    },
    #[error(
        "entity {index} of `{map}` cannot be told from {}, of the same archetype at the same pose, so it is not deleted alone",
        twins_said(.twins)
    )]
    Twin {
        map: String,
        index: usize,
        twins: Vec<At>,
    },
    #[error(
        "entity {index} of `{map}` is not in `{source_there}`, the project's source of that map file, where it was deleted: there is nothing to edit"
    )]
    GoneHere {
        map: String,
        index: usize,
        source_there: String,
    },
    #[error(
        "entity {index} of `{map}` cannot be told from another of its archetype in `{source_there}`, the project's source of that map file, so its place there is not known: name it by its place in the source, with `{source_there}` as the map"
    )]
    NotToldHere {
        map: String,
        index: usize,
        source_there: String,
    },
    #[error("a position that is not a finite number cannot be stored")]
    NotFinite,
    #[error("{0}; nothing was imported and no source was changed")]
    Plan(delete::Error),
    #[error("{0}; what was imported for it was removed, and no source was changed")]
    Operation(operation::Error),
    #[error("{0}")]
    Io(String),
    #[error("the record is not one of a first edit: its format is `{0}`, not `{FORMAT}`")]
    Format(String),
    #[error("the record is of schema {0}, and this OpenRage reads schema {SCHEMA}")]
    Schema(u32),
    #[error("the record is of a plan, which wrote nothing, so there is nothing to take back")]
    NotDone,
    #[error("the record names `{0}` as a source it made, which is no folder inside the content, so nothing was taken back")]
    Outside(String),
    #[error("{0}; nothing was taken back")]
    Stale(operation::Error),
    #[error(
        "{} changed since the edit in more than the edit wrote ({}), so the edit was not taken back and every source is as it was",
        .sources.join(" and "),
        strangers_said(.added)
    )]
    Touched {
        sources: Vec<String>,
        /// The files those sources hold that the import did not make, each inside the content's folder with `/`.
        added: Vec<String>,
    },
}

fn list(indexes: &[usize]) -> String {
    let said: Vec<String> = indexes.iter().map(usize::to_string).collect();
    said.join(" and ")
}

fn strangers_said(added: &[String]) -> String {
    match added {
        [] => "a file the import made was changed by hand or removed".to_string(),
        added => format!("the edit did not make `{}`", added.join("` and `")),
    }
}

fn twins_said(twins: &[At]) -> String {
    let said: Vec<String> = (twins.iter())
        .map(|twin| format!("entity {} of {}", twin.index, twin.source))
        .collect();
    said.join(" and ")
}

/// A sentence a person is told, as the key a catalog is to translate it by, with what its names stand for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// The English text, which is the key; `{name}` stands for the argument of that name.
    pub key: String,
    /// The text for a count that is not one, where the sentence has a plural: `{count}` stands for the count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, String>,
}

impl Message {
    fn of(key: &str) -> Self {
        Self {
            key: key.to_string(),
            other: None,
            count: None,
            args: BTreeMap::new(),
        }
    }

    fn counted((one, other): (&str, &str), count: usize) -> Self {
        Self {
            other: Some(other.to_string()),
            count: Some(count as u64),
            ..Self::of(one)
        }
    }

    fn with(mut self, name: &str, value: &str) -> Self {
        self.args.insert(name.to_string(), value.to_string());
        self
    }

    /// The sentence in English, which is what a command line prints.
    pub fn text(&self) -> String {
        let form = match (&self.other, self.count) {
            (Some(other), Some(count)) if count != 1 => other,
            _ => &self.key,
        };
        let mut text = form.clone();
        if let Some(count) = self.count {
            text = text.replace("{count}", &count.to_string());
        }
        for (name, value) in &self.args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }
}

/// What the person is told of a first edit: the data of its toast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Told {
    /// What the edit did, in one line.
    pub line: Message,
    /// The maps that came into the project; none when the edit imported nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported: Option<Message>,
    /// What a map taken from the game is to a repository of the project; none when the edit imported nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharing: Option<Message>,
    /// The label of what takes the edit and the import back as one.
    pub undo: Message,
}

/// A map file of the game the edit made a source of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Imported {
    /// Its logical path in the mounted install.
    pub map: String,
    /// The source made of it, as the content names it.
    pub source: String,
    /// The SHA-256 of the game's file, which the source keeps as its base.
    pub base_sha256: String,
    /// The digest of the source folder as the import made it, before the edit: what it has again once the inverses are applied. Empty in a plan.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub digest: String,
    /// The files of the source folder as the import made it, each inside it with `/`, by which a file added since is named. Empty in a plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// A file of the project that is no source and that the edit rewrote: the content's `maps.json`, the project's ignore rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rewritten {
    /// Inside the project's folder, with `/`.
    pub file: String,
    /// What it held; none when it was not there.
    pub before: Option<String>,
    /// The SHA-256 of what the edit wrote, by which it is known to be untouched since.
    pub after: String,
}

/// The whole of a first edit: what it imported, what it did, and what takes both back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub format: String,
    pub schema: u32,
    /// Whether it was done; a record of a plan wrote nothing and takes nothing back.
    pub done: bool,
    pub edit: Edit,
    /// The entity the edit named, by the source that holds it now and its index there.
    pub entity: At,
    pub imported: Vec<Imported>,
    /// The operations the edit is, in the order they are done.
    pub operations: Vec<Operation>,
    /// What takes them back, the last first, each source's first stating the digest of what the edit wrote; empty in a plan.
    pub inverses: Vec<Operation>,
    /// What the edit went without or left to another file, a sentence each.
    pub notes: Vec<String>,
    /// The files of the project beside the sources that the edit rewrote, in the order it wrote them; empty in a plan.
    pub rewritten: Vec<Rewritten>,
    pub told: Told,
}

/// What [`plan`] found: the record of what an edit would do, with the plan of its deletion when it is one.
#[derive(Debug, Clone)]
pub struct Planned {
    pub record: Record,
    /// What goes, what stays and each file changed, for an edit that deletes.
    pub deletion: Option<Deletion>,
}

/// What [`take_back`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenBack {
    /// The sources the record made, removed.
    pub removed: Vec<String>,
    /// The sources the edit changed and did not make, whose XML is as it was.
    pub restored: Vec<String>,
    /// The files beside the sources put back as they were.
    pub rewritten: Vec<String>,
    pub notes: Vec<String>,
}

/// The file name a logical path ends in.
fn leaf(logical: &str) -> &str {
    logical.rsplit(['/', '\\']).next().unwrap_or(logical)
}

/// The name of the source a map file at `logical` becomes; refused for a file that is no `.ymap`.
fn source_name(logical: &str) -> Result<String, Refusal> {
    match leaf(logical).rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty() && extension.eq_ignore_ascii_case(CMapData::EXTENSION) =>
        {
            Ok(folder_name(stem, or_source::metadata::KIND))
        }
        _ => Err(Refusal::NotAMap {
            file: logical.to_string(),
        }),
    }
}

fn dir_of(folder: &Path, source: &str) -> PathBuf {
    folder.join(source.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Every file under `dir` but what a file browser leaves, each inside it with `/`, sorted; what cannot be listed is left out.
fn files_in(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, inside: &str, into: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, &format!("{inside}{name}/"), into);
            } else if !or_source::game_import::LEFT_BY_A_FILE_BROWSER.contains(&name.as_str()) {
                into.push(format!("{inside}{name}"));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files);
    files.sort();
    files
}

/// The source of the content that `map` names: one called that, else the one taken from the game's file at that logical path.
fn held_as(folder: &Path, model: &Maps, game: &dyn Game, map: &str) -> Option<String> {
    if named_paths(model).any(|source| source == map) {
        return Some(map.to_string());
    }
    let lies_at = game.lies_at(map)?;
    let taken_from = |source: &&str| {
        stays_inside(source)
            && std::fs::read_to_string(dir_of(folder, source).join(MANIFEST))
                .ok()
                .and_then(|text| AssetManifest::parse(&text).ok())
                .is_some_and(|manifest| {
                    let at = manifest.origin.virtual_path.as_deref();
                    manifest.kind == or_source::metadata::KIND
                        && manifest.origin.from_game
                        && at.is_some_and(|at| at.eq_ignore_ascii_case(&lies_at))
                })
    };
    named_paths(model).find(taken_from).map(str::to_string)
}

/// What the content holds where `source`, the source a map file of the game would become, would go: a source a set names by that folder name, or a folder there.
///
/// ⚠ A source of that name in a folder of the content is the same map held twice once the game's comes in beside it.
fn in_the_way(folder: &Path, model: &Maps, source: &str) -> Option<String> {
    let named = named_paths(model).find(|path| leaf(path).eq_ignore_ascii_case(source));
    (named.map(str::to_string))
        .or_else(|| (dir_of(folder, source).exists()).then(|| source.to_string()))
}

/// A map file of the game read for an edit.
struct Fetched {
    logical: String,
    bytes: Vec<u8>,
}

/// The map files of the game a deletion asks for, read through [`Game`] and kept by the source each is to be.
struct Reach<'a> {
    game: &'a dyn Game,
    fetched: BTreeMap<String, Fetched>,
    /// Why a file the game lists was not read, which stops the plan.
    refused: Option<Refusal>,
}

impl Reach<'_> {
    /// Read the map file at `logical`, whose bytes are `bytes`, as the source it is to be.
    fn take(&mut self, logical: &str, bytes: Vec<u8>) -> Result<Reached, Refusal> {
        let source = source_name(logical)?;
        let reached = Reached::read(&source, bytes.clone()).map_err(|why| Refusal::Unread {
            file: logical.to_string(),
            why,
        })?;
        let fetched = Fetched {
            logical: logical.to_string(),
            bytes,
        };
        self.fetched.insert(source, fetched);
        Ok(reached)
    }

    fn fetch(&mut self, logical: &str) -> Result<Reached, Refusal> {
        let bytes = self.game.read(logical).ok_or_else(|| Refusal::Unread {
            file: logical.to_string(),
            why: "the install lists it and gives no bytes for it".to_string(),
        })?;
        self.take(logical, bytes)
    }

    fn fetch_all(&mut self, logical: Vec<String>) -> Result<Vec<Reached>, String> {
        let mut reached = Vec::with_capacity(logical.len());
        for logical in logical {
            match self.fetch(&logical) {
                Ok(one) => reached.push(one),
                Err(refused) => {
                    let said = refused.to_string();
                    self.refused = Some(refused);
                    return Err(said);
                }
            }
        }
        Ok(reached)
    }
}

impl Beyond for Reach<'_> {
    fn named(&mut self, name: u32) -> Result<Option<Reached>, String> {
        let logical: Vec<String> = self.game.map_file(name).into_iter().collect();
        self.fetch_all(logical).map(|mut one| one.pop())
    }

    fn under(&mut self, parent: u32) -> Result<Vec<Reached>, String> {
        let logical = self.game.maps_under(parent);
        self.fetch_all(logical)
    }
}

/// The note of every first edit: an entity is a record of a map file, and what collides for it is in another file.
const COLLISION_STAYS: &str = "the static collision of the area is not changed, so what the object collides with stays where it was: `openrage-cli maps collision` proposes it, takes it out or moves it";

/// What an edit comes to, before anything is written.
struct Prepared {
    folder: PathBuf,
    entity: At,
    /// The map files to import, by the source each becomes.
    imports: Vec<(String, Fetched)>,
    operations: Vec<Operation>,
    deletion: Option<Deletion>,
    notes: Vec<String>,
    /// How many entities a deletion takes out.
    gone: usize,
}

fn index_of(map: &CMapData, named: &str, which: Which) -> Result<usize, Refusal> {
    let len = map.entities.len();
    match which {
        Which::Index(index) if index < len => Ok(index),
        Which::Index(index) => Err(Refusal::NoSuchEntity {
            map: named.to_string(),
            index,
            len,
        }),
        Which::Guid(guid) => {
            let stating: Vec<usize> = (0..len)
                .filter(|index| map.entities[*index].guid == guid)
                .collect();
            match stating.as_slice() {
                [] => Err(Refusal::NoSuchGuid {
                    map: named.to_string(),
                    guid,
                }),
                [one] => Ok(*one),
                _ => Err(Refusal::AmbiguousGuid {
                    map: named.to_string(),
                    guid,
                    entities: stating,
                }),
            }
        }
    }
}

/// The place in `now`, the map the content's `source` cooks to, of the entity at `place` of the game's file at `logical`.
///
/// ⚠ A path of the install comes with a place in the game's file, as `world find` and `world select` print one, and the source of
/// that file holds other entities before it once one was deleted or added there: the two files are paired by what they hold.
fn carried(
    game: &dyn Game,
    logical: &str,
    source: &str,
    now: &CMapData,
    place: usize,
) -> Result<usize, Refusal> {
    let bytes = game.read(logical).ok_or_else(|| Refusal::NoSuchMap {
        map: logical.to_string(),
    })?;
    let shipped = CMapData::from_bytes(&bytes).map_err(|e| Refusal::Unread {
        file: logical.to_string(),
        why: e.to_string(),
    })?;
    let len = shipped.entities.len();
    if place >= len {
        return Err(Refusal::NoSuchEntity {
            map: logical.to_string(),
            index: place,
            len,
        });
    }
    let (changes, pairs) = crate::changes::map_changes_paired(&shipped, now);
    let (map, source_there) = (logical.to_string(), source.to_string());
    match pairs.get(place).copied().flatten() {
        Some(there) => Ok(there),
        None if (changes.ambiguous.iter())
            .any(|group| group.base.iter().any(|entity| entity.index == place)) =>
        {
            Err(Refusal::NotToldHere {
                map,
                index: place,
                source_there,
            })
        }
        None => Err(Refusal::GoneHere {
            map,
            index: place,
            source_there,
        }),
    }
}

/// Refuse a map file at `logical` that is not the copy the mount loads for the map of its name.
///
/// ⚠ By where each lies and not by its path: the copy a title update patches answers at the pack's path and at its own.
fn loaded_copy(game: &dyn Game, logical: &str) -> Result<(), Refusal> {
    let name = leaf(logical).to_ascii_lowercase();
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    let loaded = game.map_file(or_io::string_hash(stem.as_bytes()));
    match loaded.filter(|loaded| game.lies_at(loaded) != game.lies_at(logical)) {
        Some(loaded) => Err(Refusal::NotTheCopyLoaded {
            file: logical.to_string(),
            loaded,
        }),
        None => Ok(()),
    }
}

/// Refuse an import from an install of another edition than a build that takes the content targets.
fn same_edition(place: &Place<'_>, game: &dyn Game, file: &str) -> Result<(), Refusal> {
    let install = game.edition();
    let other = (place.container.outputs.iter())
        .filter(|output| output.take.contains(&place.module.id))
        .find_map(|output| Some((output, output.target.filter(|target| *target != install)?)));
    match other {
        Some((output, target)) => Err(Refusal::OtherEdition {
            file: file.to_string(),
            install: install.generation().edition_label(),
            build: output.name.clone(),
            target: target.generation().edition_label(),
        }),
        None => Ok(()),
    }
}

fn prepare(
    place: &Place<'_>,
    game: &dyn Game,
    edit: &Edit,
    to_write: bool,
) -> Result<Prepared, Refusal> {
    let folder = if to_write {
        crate::plan::module_folder_to_edit(place.project, place.container, place.module)?
    } else {
        let project_folder = or_project::project_folder(place.project);
        crate::plan::module_folder(&project_folder, place.module)?
    };
    let model = Maps::read(&folder)?;
    if let Change::Move { position } = edit.change {
        if !position.iter().all(|number| number.is_finite()) {
            return Err(Refusal::NotFinite);
        }
    }

    let mut reach = Reach {
        game,
        fetched: BTreeMap::new(),
        refused: None,
    };
    // The map file the entity is in: a source of the content, or the game's file, read here and made a source later.
    let held = held_as(&folder, &model, game, &edit.map);
    let (source, map, mut start) = match &held {
        Some(source) => {
            let unread = |why: String| Refusal::Unread {
                file: source.clone(),
                why,
            };
            let cooked = crate::facts::cooked(&dir_of(&folder, source)).map_err(unread)?;
            let map = CMapData::from_bytes(&cooked).map_err(|e| unread(e.to_string()))?;
            (source.clone(), map, Vec::new())
        }
        None => {
            let bytes = game.read(&edit.map).ok_or_else(|| Refusal::NoSuchMap {
                map: edit.map.clone(),
            })?;
            let source = source_name(&edit.map)?;
            // ⚠ Before the entity is looked for: under that name the entity would be found in the content's own map.
            if let Some(source_there) = in_the_way(&folder, &model, &source) {
                return Err(Refusal::InTheWay {
                    file: edit.map.clone(),
                    source_there,
                });
            }
            same_edition(place, game, &edit.map)?;
            loaded_copy(game, &edit.map)?;
            let reached = reach.take(&edit.map, bytes)?;
            let map = reached.map().clone();
            (source, map, vec![reached])
        }
    };
    let (index, of_the_game) = match (edit.entity, &held) {
        (Which::Index(place), Some(held)) if *held != edit.map => {
            let there = carried(game, &edit.map, held, &map, place)?;
            (there, (there != place).then_some(place))
        }
        (which, _) => (index_of(&map, &edit.map, which)?, None),
    };
    let entity = At {
        source: source.clone(),
        index,
    };

    let (operations, deletion, touched, mut notes, gone) = match edit.change {
        Change::Move { position } => {
            let pose = Pose {
                position: Vec3::from_array(position),
                ..pose_of(&map.entities[index])
            };
            let place_entity = Operation::PlaceEntity {
                source: source.clone(),
                expects: None,
                entity: index,
                pose: PoseData::from(pose),
            };
            let touched: BTreeSet<String> =
                held.is_none().then(|| source.clone()).into_iter().collect();
            // ⚠ A move changes one entity of one map: what stands for the object from afar and what it stands for stay.
            let moved = &map.entities[index];
            let far_model = match crate::changes::link_of(moved) {
                crate::changes::Link::None => None,
                crate::changes::Link::Own(parent) => Some(format!("entity {parent} of `{source}`")),
                crate::changes::Link::ParentMap(parent) => {
                    let file = game.map_file(map.parent.hash);
                    let named = file.map_or_else(
                        || "its parent map".to_string(),
                        |file| format!("`{}`", leaf(&file)),
                    );
                    Some(format!("entity {parent} of {named}"))
                }
            };
            let mut notes = Vec::new();
            if let Some(far_model) = far_model {
                notes.push(format!(
                    "the far model of the object, {far_model}, stays where it was, and is drawn for it there from afar until it is moved too"
                ));
            }
            if moved.num_children > 0 {
                notes.push(format!(
                    "the object is a far model that stands for {} nearer model(s), which stay where they were",
                    moved.num_children
                ));
            }
            notes.push(COLLISION_STAYS.to_string());
            (vec![place_entity], None, touched, notes, 0)
        }
        Change::Delete => {
            let chosen = [entity.clone()];
            let start = std::mem::take(&mut start);
            let planned = delete::plan_reaching(&folder, start, &chosen, &mut reach);
            let (deletion, beside) = match planned {
                Ok(planned) => planned,
                Err(delete::Error::Beyond(said)) => {
                    return Err(reach.refused.take().unwrap_or(Refusal::Io(said)));
                }
                Err(refused) => return Err(Refusal::Plan(refused)),
            };
            if let Some(left) = (deletion.ambiguous.first()).filter(|_| deletion.goes.is_empty()) {
                return Err(Refusal::Twin {
                    map: edit.map.clone(),
                    index,
                    twins: left.twins.clone(),
                });
            }
            // ⚠ Of the map files read beside the content's own: a file of the game whose map the content holds a source of was
            // read and set aside, and is not what the plan changes under that name.
            let touched = (deletion.files.iter())
                .map(|change| change.source.clone())
                .filter(|source| beside.iter().any(|reached| reached.source() == source))
                .collect();
            let mut notes: Vec<String> = deletion.notes.iter().map(ToString::to_string).collect();
            notes.push(COLLISION_STAYS.to_string());
            let gone = deletion.goes.len();
            let operations = deletion.operations.clone();
            (operations, Some(deletion), touched, notes, gone)
        }
    };

    if let Some(place) = of_the_game {
        notes.insert(
            0,
            format!(
                "entity {place} of the game's `{}` is entity {index} of `{source}`, the project's source of it, which holds other entities before it",
                leaf(&edit.map)
            ),
        );
    }

    // Only what the edit changes comes in: a map file read to follow the chain and left as it is stays the game's.
    let imports: Vec<(String, Fetched)> = (reach.fetched.into_iter())
        .filter(|(source, _)| touched.contains(source))
        .collect();
    if let Some((_, first)) = imports.first() {
        same_edition(place, game, &first.logical)?;
    }
    // ⚠ Asked of every map file the chain brings in and not only of the entity's own, so that a plan refuses what the import would.
    for (source, fetched) in &imports {
        if let Some(source_there) = in_the_way(&folder, &model, source) {
            return Err(Refusal::InTheWay {
                file: fetched.logical.clone(),
                source_there,
            });
        }
    }
    Ok(Prepared {
        folder,
        entity,
        imports,
        operations,
        deletion,
        notes,
        gone,
    })
}

/// What the person is told of the edit `prepared` is.
fn told_of(edit: &Edit, prepared: &Prepared, commits_game_sources: bool) -> Told {
    let map = leaf(&edit.map);
    let line = match edit.change {
        Change::Move { .. } => Message::of(told::MOVED),
        Change::Delete => Message::counted(told::DELETED, prepared.gone),
    }
    .with("map", map);
    let count = prepared.imports.len();
    let first = (prepared.imports.first()).map_or(map, |(_, fetched)| leaf(&fetched.logical));
    let sharing = if commits_game_sources {
        told::COMMITTED
    } else {
        told::LEFT_OUT
    };
    Told {
        line,
        imported: (count > 0).then(|| Message::counted(told::IMPORTED, count).with("map", first)),
        sharing: (count > 0).then(|| Message::counted(sharing, count)),
        undo: Message::of(told::UNDO),
    }
}

fn record_of(place: &Place<'_>, edit: &Edit, prepared: &Prepared) -> Record {
    let commit = or_source::git::commit_of(&or_project::project_folder(place.project));
    Record {
        format: FORMAT.to_string(),
        schema: SCHEMA,
        done: false,
        edit: edit.clone(),
        entity: prepared.entity.clone(),
        imported: (prepared.imports.iter())
            .map(|(source, fetched)| Imported {
                map: fetched.logical.clone(),
                source: source.clone(),
                base_sha256: sha256_hex(&fetched.bytes),
                digest: String::new(),
                files: Vec::new(),
            })
            .collect(),
        operations: prepared.operations.clone(),
        inverses: Vec::new(),
        notes: prepared.notes.clone(),
        rewritten: Vec::new(),
        told: told_of(edit, prepared, commit.game_sources),
    }
}

/// What `edit` would import into the content at `place` and do there, with nothing written; a map file that would not become a source refuses it as [`apply`] would.
pub fn plan(place: &Place<'_>, game: &dyn Game, edit: &Edit) -> Result<Planned, Refusal> {
    let prepared = prepare(place, game, edit, false)?;
    for (_, fetched) in &prepared.imports {
        let name = leaf(&fetched.logical);
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        or_source::metadata::check(&fetched.bytes, stem, CMapData::EXTENSION).map_err(
            |problems| Refusal::NotASource {
                file: fetched.logical.clone(),
                why: said(&problems),
            },
        )?;
    }
    Ok(Planned {
        record: record_of(place, edit, &prepared),
        deletion: prepared.deletion,
    })
}

fn said(problems: &[or_source::Problem]) -> String {
    let said: Vec<&str> = problems.iter().map(|p| p.message.as_str()).collect();
    said.join("; ")
}

/// A file beside the sources as it is before the edit writes it.
struct Beside {
    path: PathBuf,
    /// Inside the project's folder, with `/`.
    file: String,
    before: Option<Vec<u8>>,
}

impl Beside {
    fn read(project_folder: &Path, path: PathBuf) -> Option<Self> {
        let file = or_project::relative_inside(&path, project_folder)?;
        let before = std::fs::read(&path).ok();
        Some(Self { path, file, before })
    }

    /// Put the file back as it was read, when it holds anything else.
    fn put_back(&self) -> std::io::Result<()> {
        if std::fs::read(&self.path).ok() == self.before {
            return Ok(());
        }
        match &self.before {
            Some(bytes) => std::fs::write(&self.path, bytes),
            None if self.path.exists() => std::fs::remove_file(&self.path),
            None => Ok(()),
        }
    }

    /// What the edit made of the file, when it holds another text than it did.
    fn rewritten(&self) -> Option<Rewritten> {
        let now = std::fs::read(&self.path).ok();
        if now == self.before {
            return None;
        }
        let before = match &self.before {
            Some(bytes) => Some(String::from_utf8(bytes.clone()).ok()?),
            None => None,
        };
        Some(Rewritten {
            file: self.file.clone(),
            before,
            after: sha256_hex(&now?),
        })
    }
}

/// The inverses of `done` as anything that reads the disk applies them, the last first; `before` is the XML the source of each held before the list.
///
/// ⚠ A session names a state it has not written by a token no other reader knows. The first inverse of each source states the digest
/// of what the flush wrote, and the ones after it state nothing: the list is applied whole or not at all, so the first decides for
/// them. A source whose inverses the flush did not see put it back gets the XML it held instead.
fn durable(done: &[Done], written: &[Written], before: &[Arc<str>]) -> Vec<Operation> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut inverses = Vec::with_capacity(done.len());
    for (step, was) in done.iter().zip(before).rev() {
        let source = step.inverse.source();
        let first = seen.insert(source);
        let flushed = written.iter().find(|written| written.source == source);
        // A source the operations left as it was is written by no flush, and holds what it held.
        let digest = flushed.map_or_else(
            || digest_of(was.as_bytes()),
            |written| written.digest.clone(),
        );
        if flushed.is_some_and(|written| !written.proven) {
            if first {
                inverses.push(Operation::RewriteSource {
                    source: source.to_string(),
                    expects: Some(digest),
                    xml: was.to_string(),
                });
            }
            continue;
        }
        let mut inverse = step.inverse.clone();
        inverse.set_expects(first.then_some(digest));
        inverses.push(inverse);
    }
    inverses
}

/// The XML the source of each of `operations` holds, in their order.
fn xml_of(folder: &Path, operations: &[Operation]) -> Result<Vec<Arc<str>>, operation::Error> {
    let mut held: BTreeMap<&str, Arc<str>> = BTreeMap::new();
    let mut each = Vec::with_capacity(operations.len());
    for operation in operations {
        let source = operation.source();
        if let Some(xml) = held.get(source) {
            each.push(Arc::clone(xml));
            continue;
        }
        let unread = |why: String| operation::Error::Source {
            at: source.to_string(),
            why,
        };
        if !stays_inside(source) {
            return Err(unread("it leaves the folder of the content".to_string()));
        }
        let file = dir_of(folder, source).join(or_source::metadata::XML);
        let xml = std::fs::read(&file).map_err(|e| unread(format!("{}: {e}", file.display())))?;
        let xml: Arc<str> = String::from_utf8(xml)
            .map_err(|_| unread("its XML is not UTF-8".to_string()))?
            .into();
        held.insert(source, Arc::clone(&xml));
        each.push(xml);
    }
    Ok(each)
}

/// Do `operations` to the sources of the content in `folder` through a session flushed once, and give the inverses the disk takes.
fn done_in_a_session(
    folder: &Path,
    operations: &[Operation],
) -> Result<(Vec<Operation>, Vec<String>), operation::Error> {
    let before = xml_of(folder, operations)?;
    let mut session = Session::open(folder)?;
    let flushed = (session.apply_all(operations))
        .and_then(|done| session.flush().map(|written| (done, written)));
    let (done, written) = match flushed {
        Ok(flushed) => flushed,
        Err(refused) => {
            // ⚠ Dropped as it is, a session writes what it holds.
            session.discard();
            return Err(refused);
        }
    };
    let inverses = durable(&done, &written, &before);
    let notes = done.into_iter().flat_map(|step| step.notes).collect();
    Ok((inverses, notes))
}

/// Import what `edit` needs into the content at `place`, apply it, and give the record of the whole; a refusal leaves the project as it was.
pub fn apply(place: &Place<'_>, game: &dyn Game, edit: &Edit) -> Result<Record, Refusal> {
    let prepared = prepare(place, game, edit, true)?;
    let mut record = record_of(place, edit, &prepared);
    let folder = &prepared.folder;
    let project_folder = or_project::project_folder(place.project);
    let beside: Vec<Beside> = [
        folder.join(model::FILE),
        project_folder.join(or_source::git::IGNORE),
        project_folder.join(or_source::git::ATTRIBUTES),
    ]
    .into_iter()
    .filter_map(|path| Beside::read(&project_folder, path))
    .collect();

    // What the import made, removed when anything after it is refused.
    let mut made: Vec<PathBuf> = Vec::new();
    let remove = |made: &[PathBuf]| {
        for dir in made {
            if let Err(e) = std::fs::remove_dir_all(dir) {
                tracing::error!(folder = %dir.display(), error = %e, "a source made for an edit that was refused could not be removed");
            }
        }
    };
    for (imported, (source, fetched)) in record.imported.iter_mut().zip(&prepared.imports) {
        let lies_at = game.lies_at(&fetched.logical);
        let incoming = Incoming {
            name: leaf(&fetched.logical),
            bytes: &fetched.bytes,
            from_game: true,
            virtual_path: lies_at.as_deref(),
        };
        let dir = match import::add_one(folder, &incoming) {
            Ok(dir) => dir,
            Err(why) => {
                remove(&made);
                return Err(Refusal::NotASource {
                    file: fetched.logical.clone(),
                    why,
                });
            }
        };
        made.push(dir.clone());
        // ⚠ The operations name the source by the folder it was to become.
        let digest = (dir == dir_of(folder, source))
            .then(|| tree_digest(&dir).ok())
            .flatten();
        let Some(digest) = digest else {
            remove(&made);
            return Err(Refusal::Io(format!(
                "the source made of `{}` is not the folder `{source}` the edit names, or could not be read back",
                fetched.logical
            )));
        };
        imported.digest = digest;
        imported.files = files_in(&dir);
    }
    let undo = |made: &[PathBuf]| {
        remove(made);
        for file in &beside {
            if let Err(e) = file.put_back() {
                tracing::error!(file = %file.path.display(), error = %e, "a file an edit that was refused rewrote could not be put back");
            }
        }
    };
    if !made.is_empty() {
        if let Err(why) = import::learn(folder, &made, None) {
            undo(&made);
            return Err(Refusal::Io(why));
        }
    }
    let (inverses, notes) = match done_in_a_session(folder, &prepared.operations) {
        Ok(done) => done,
        Err(refused) => {
            undo(&made);
            return Err(Refusal::Operation(refused));
        }
    };
    record.inverses = inverses;
    record.notes.extend(notes);
    // The ignore rules name a source taken from the game, so a repository leaves out what was just made.
    if !made.is_empty() {
        if let Err(e) = or_source::git::update(&project_folder) {
            record.notes.push(format!(
                "the ignore rules of the project are not updated, so they do not leave out what was imported: {e}"
            ));
        }
    }
    record.rewritten = beside.iter().filter_map(Beside::rewritten).collect();
    record.done = true;
    Ok(record)
}

/// Take back what `record` did in the content at `place`: the operations by their inverses, then the sources it made and the files it rewrote; all of it or nothing.
pub fn take_back(place: &Place<'_>, record: &Record) -> Result<TakenBack, Refusal> {
    if record.format != FORMAT {
        return Err(Refusal::Format(record.format.clone()));
    }
    if record.schema != SCHEMA {
        return Err(Refusal::Schema(record.schema));
    }
    if !record.done {
        return Err(Refusal::NotDone);
    }
    // ⚠ A record is a file anyone can write, and a source it says it made is removed: only a folder inside the content is.
    if let Some(outside) = (record.imported.iter()).find(|made| !stays_inside(&made.source)) {
        return Err(Refusal::Outside(outside.source.clone()));
    }
    let folder = crate::plan::module_folder_to_edit(place.project, place.container, place.module)?;
    let project_folder = or_project::project_folder(place.project);
    let undone = operation::apply_all(&folder, &record.inverses, false).map_err(Refusal::Stale)?;

    // A source the record made goes only as the import made it: anything else in it is a person's.
    let made: Vec<(String, Created)> = (record.imported.iter())
        .map(|imported| {
            let created = Created {
                path: dir_of(&folder, &imported.source),
                digest: imported.digest.clone(),
            };
            (imported.source.clone(), created)
        })
        .collect();
    let touched: Vec<String> = (made.iter())
        .filter(|(_, created)| !created.untouched())
        .map(|(source, _)| source.clone())
        .collect();
    if !touched.is_empty() {
        operation::apply_all(&folder, &operation::inverses(&undone), false).map_err(|e| {
            Refusal::Io(format!(
                "{} changed since the edit, and the edit could not be put back after its inverses were applied: {e}",
                touched.join(" and ")
            ))
        })?;
        let made_then =
            |source: &String| record.imported.iter().find(|made| made.source == *source);
        let added = (touched.iter().filter_map(made_then))
            .filter(|made| !made.files.is_empty())
            .flat_map(|made| {
                let now = files_in(&dir_of(&folder, &made.source));
                let strangers = now.into_iter().filter(|file| !made.files.contains(file));
                strangers.map(|file| format!("{}/{file}", made.source))
            })
            .collect();
        return Err(Refusal::Touched {
            sources: touched,
            added,
        });
    }

    let mut taken = TakenBack {
        removed: Vec::new(),
        restored: Vec::new(),
        rewritten: Vec::new(),
        notes: Vec::new(),
    };
    for (source, created) in &made {
        match created.undo() {
            Ok(true) => taken.removed.push(source.clone()),
            Ok(false) => taken.notes.push(format!(
                "`{source}` changed while it was taken back, and stays"
            )),
            Err(e) => return Err(Refusal::Io(format!("`{source}` could not be removed: {e}"))),
        }
    }
    for done in &undone {
        let source = done.inverse.source();
        let known = taken.removed.iter().chain(&taken.restored);
        if done.edited.changed() && !known.clone().any(|known| known == source) {
            taken.restored.push(source.to_string());
        }
    }

    // The files beside the sources go back to what they held, the last written first, while they hold what the edit wrote.
    for rewritten in record.rewritten.iter().rev() {
        let inside = stays_inside(&rewritten.file);
        let path = dir_of(&project_folder, &rewritten.file);
        let as_written =
            inside && std::fs::read(&path).is_ok_and(|now| sha256_hex(&now) == rewritten.after);
        if as_written {
            let put_back = match &rewritten.before {
                Some(text) => std::fs::write(&path, text),
                None => std::fs::remove_file(&path),
            };
            put_back.map_err(|e| Refusal::Io(format!("{}: {e}", path.display())))?;
            taken.rewritten.push(rewritten.file.clone());
            continue;
        }
        // Changed since: what the record added is taken out of it, and the rest is whoever's changed it.
        let settled = if path == folder.join(model::FILE) {
            import::forget(&folder, &taken.removed).map(drop)
        } else {
            (or_source::git::update(&project_folder).map(drop)).map_err(|e| e.to_string())
        };
        taken.notes.push(match settled {
            Ok(()) => format!(
                "`{}` was changed since the edit, so it was not put back whole: what the edit added to it was taken out",
                rewritten.file
            ),
            Err(why) => format!(
                "`{}` was changed since the edit and still names what was removed: {why}",
                rewritten.file
            ),
        });
    }
    Ok(taken)
}

impl Edit {
    /// The edit in words, as a command line says it.
    pub fn said(&self) -> String {
        let entity = match self.entity {
            Which::Index(index) => format!("entity {index}"),
            Which::Guid(guid) => format!("the entity of guid {guid}"),
        };
        match self.change {
            Change::Move {
                position: [x, y, z],
            } => {
                format!("move {entity} of {} to {x}, {y}, {z}", self.map)
            }
            Change::Delete => format!(
                "delete {entity} of {} with its chain of far models",
                self.map
            ),
        }
    }
}

impl Record {
    /// The record as JSON with a two-space indent and LF.
    pub fn to_json(&self) -> Result<String, Refusal> {
        let mut text =
            serde_json::to_string_pretty(self).map_err(|e| Refusal::Io(e.to_string()))?;
        text.push('\n');
        Ok(text)
    }

    /// The record `bytes` hold; a format that is not [`FORMAT`] and a schema that is not [`SCHEMA`] are refused before the rest is read.
    pub fn from_json(bytes: &[u8]) -> Result<Self, Refusal> {
        let json = |e: serde_json::Error| Refusal::Io(format!("the record does not read: {e}"));
        let top: serde_json::Value = serde_json::from_slice(bytes).map_err(json)?;
        let format = top.get("format").and_then(|f| f.as_str()).unwrap_or("");
        if format != FORMAT {
            return Err(Refusal::Format(format.to_string()));
        }
        let schema = top.get("schema").and_then(serde_json::Value::as_u64);
        if schema != Some(u64::from(SCHEMA)) {
            let stated = schema.and_then(|schema| u32::try_from(schema).ok());
            return Err(Refusal::Schema(stated.unwrap_or(0)));
        }
        serde_json::from_value(top).map_err(json)
    }

    /// What the record says, a line each: the edit, each map imported, each operation, each note, and what the person is told.
    pub fn lines(&self) -> Vec<String> {
        let (imported, did) = if self.done {
            ("imported", "did")
        } else {
            ("would import", "would do")
        };
        let mut lines = vec![format!(
            "{}: entity {} of {}",
            self.edit.said(),
            self.entity.index,
            self.entity.source
        )];
        for import in &self.imported {
            lines.push(format!(
                "{imported} {} as {}: a source taken from the game, whose base is the game's file (sha256 {})",
                import.map, import.source, import.base_sha256
            ));
        }
        if self.imported.is_empty() {
            lines.push(format!(
                "{imported} nothing: {} is a source of the content already",
                self.entity.source
            ));
        }
        for (at, operation) in self.operations.iter().enumerate() {
            lines.push(format!(
                "{did} {}. {} of {}",
                at + 1,
                operation.name(),
                operation.source()
            ));
        }
        for note in &self.notes {
            lines.push(format!("note: {note}"));
        }
        for rewritten in &self.rewritten {
            lines.push(format!("rewrote {}", rewritten.file));
        }
        let told = [
            Some(&self.told.line),
            self.told.imported.as_ref(),
            self.told.sharing.as_ref(),
        ];
        for message in told.into_iter().flatten() {
            lines.push(format!("told: {}", message.text()));
        }
        if self.done {
            lines.push(format!(
                "{}: takes back the edit and the import as one, by {} inverse operation(s) and the removal of {} source(s)",
                self.told.undo.text(),
                self.inverses.len(),
                self.imported.len()
            ));
        }
        lines
    }
}

impl TakenBack {
    /// What was taken back, a line each.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for source in &self.restored {
            lines.push(format!("put back {source} as it was"));
        }
        for source in &self.removed {
            lines.push(format!("removed {source}, which the edit imported"));
        }
        for file in &self.rewritten {
            lines.push(format!("put back {file} as it was"));
        }
        for note in &self.notes {
            lines.push(format!("note: {note}"));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(source: &str, entity: usize, token: &str) -> Done {
        Done {
            inverse: Operation::SetNumChildren {
                source: source.to_string(),
                expects: Some(token.to_string()),
                entity,
                num_children: 1,
            },
            added: None,
            notes: Vec::new(),
        }
    }

    fn wrote(source: &str, digest: &str, proven: bool) -> Written {
        Written {
            source: source.to_string(),
            file: PathBuf::from(source).join(or_source::metadata::XML),
            digest: digest.to_string(),
            proven,
        }
    }

    #[test]
    fn the_inverses_a_record_keeps_state_what_the_disk_holds_and_no_token_of_a_session() {
        // Three steps: two of one source the flush wrote and proved, then one of a source it wrote and did not prove, with one more
        // of that source, and one of a source the steps left as it was.
        let done = [
            counted("a.metadata.src", 0, "unwritten-1"),
            counted("a.metadata.src", 1, "unwritten-2"),
            counted("b.metadata.src", 0, "unwritten-3"),
            counted("b.metadata.src", 1, "unwritten-4"),
            counted("c.metadata.src", 0, "unwritten-5"),
        ];
        let written = [
            wrote("a.metadata.src", "digest-of-a", true),
            wrote("b.metadata.src", "digest-of-b", false),
        ];
        let before: Vec<Arc<str>> = ["<a/>", "<a/>", "<b/>", "<b/>", "<c/>"]
            .map(Arc::from)
            .to_vec();

        let inverses = durable(&done, &written, &before);
        let said: Vec<(&str, &str, Option<&str>)> = (inverses.iter())
            .map(|inverse| (inverse.name(), inverse.source(), inverse.expects()))
            .collect();
        let held = digest_of(b"<c/>");
        assert_eq!(
            said,
            [
                ("set_num_children", "c.metadata.src", Some(held.as_str())),
                ("rewrite_source", "b.metadata.src", Some("digest-of-b")),
                ("set_num_children", "a.metadata.src", Some("digest-of-a")),
                ("set_num_children", "a.metadata.src", None),
            ]
        );
        // The last step of a source is taken back first, and the XML put back is the one the source held before the list.
        assert!(matches!(
            &inverses[2],
            Operation::SetNumChildren { entity: 1, .. }
        ));
        assert!(matches!(
            &inverses[1],
            Operation::RewriteSource { xml, .. } if xml == "<b/>"
        ));
    }
}
