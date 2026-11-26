//! Taking entities out of the map files of a pack with their chain of far models, as a plan that is data.
//!
//! A building is several entities: the model seen close, the far model that stands for it at a distance, and often a farther one that
//! stands for a block. Each names the next as its LOD parent, in its own file or, with `flags` bit 3, in the map its file names as
//! parent, and each parent counts its children. [`plan`] reads every map file of a pack and says, for the entities chosen, what goes
//! with them (their children down the tree, and each parent left with no child, up the chain), which parents stay because they stand
//! for other entities too, and what changes in each file. Its [`Deletion::operations`] are that deletion as a list of
//! [`crate::operation`], which [`apply`] does whole or not at all and whose inverses put every file back byte for byte.
//!
//! The typed roots save with no schema snapshot, so the deletion is written on any machine; the links are followed the way
//! `or_world::execute` follows them over value trees, which it writes only where a snapshot is staged.
//!
//! ⚠ **A plan sees the sources of the pack and nothing else.** A map whose parent is one of these and that is not among the sources
//! keeps the indexes it had, which then name other entities. What the sources themselves show of that is said as a [`Note`]: a link
//! into a map that is not among them, and a count of children they do not hold. A parent that counts children no source holds stays.
//! [`plan_reaching`] is the plan of a caller that holds more, the map files of a mounted install: it asks a [`Beyond`] for the parent
//! map such a note names, and for the maps under a map that loses an entity, until the chain is whole.
//!
//! ⚠ **A plan is of the files as they were read.** Its operations name entities by index, so a plan applied after another edit of
//! one of its sources may name other entities: plan again. The extents a file states are left as they are, since a box that still
//! holds what is gone streams a map a little early and nothing late.
//!
//! ⚠ **An entity is named by its index, and a twin cannot be.** Two entities of one archetype at one pose are one object to whoever
//! picks it on a screen or hides it from a script, so a chosen entity with a twin that is not chosen too is left and said as
//! [`Ambiguous`], unless the chain of another chosen entity takes it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use glam::Vec3;
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CEntityDefAny, CMapData};
use or_io::meta::MetaDocument;
use or_source::metadata::Inputs;
use or_world::MapData;

use crate::edit::in_parent_map;
use crate::facts::{map_name, named_paths, stays_inside, Name};
use crate::model::{self, Maps};
use crate::operation::{self, Applied, At, Operation};
use crate::script_text::{self, ScriptForm};

/// Why a deletion was not planned.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("the source `{0}` is not read, and it may be a map file, whose links a deletion follows: {1}")]
    Source(String, String),
    #[error("no entity is named")]
    Nothing,
    #[error("no map file of the pack is the source `{0}`")]
    NoSuchSource(String),
    #[error("`{at}` holds {len} entities, so there is no entity {index}")]
    NoSuchEntity {
        /// The source named.
        at: String,
        index: usize,
        len: usize,
    },
    #[error("`{0}` carries container LODs, whose indexes a deletion does not renumber")]
    ContainerLods(String),
    #[error("{} each hold entities of one map, so an index into it does not say which file", .0.join(" and "))]
    SplitMap(Vec<String>),
    #[error("{0}")]
    Beyond(String),
}

/// One map file of a pack, as a deletion reads it.
struct MapFile {
    /// The source, as the pack names it.
    source: String,
    /// The hash of the name of the file it cooks to, which is the map's name to the game and to the maps under it (`crate::facts::map_name`).
    name: u32,
    map: CMapData,
    /// The container LODs the file carries, whose indexes no deletion renumbers.
    container_lods: usize,
}

/// An entity as a plan names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub at: At,
    pub archetype: Name,
    pub position: [f32; 3],
}

/// Why an entity goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// It was asked for.
    Chosen,
    /// Its LOD parent, which this names, goes.
    Under(At),
    /// It stood for entities that go and for no other.
    LeftWithNoChild,
}

/// An entity the deletion takes out.
#[derive(Debug, Clone, PartialEq)]
pub struct Going {
    pub entity: Entity,
    pub why: Why,
}

/// A LOD parent of an entity that goes, which stays because it stands for others too.
#[derive(Debug, Clone, PartialEq)]
pub struct Staying {
    pub entity: Entity,
    /// The entities among the sources that still name it.
    pub children_left: usize,
    /// The children it counted that no source holds.
    pub children_elsewhere: u32,
    /// The children it counts before and after.
    pub counted: u32,
    pub counts: u32,
}

/// A chosen entity that is left, because the entities of `twins` are of its archetype at its pose and were not all chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct Ambiguous {
    pub entity: Entity,
    pub twins: Vec<At>,
}

/// What the sources do not hold of the chain of an entity that goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// Its LOD parent is in the map `parent`, which is not among the sources: the far model stays, and counts a child it has lost.
    ParentMapNotAmongSources { entity: At, parent: Name },
    /// Its link leads into a parent map, and its own map names none.
    NoParentMap { entity: At },
    /// Its LOD parent is in a map that several sources each hold entities of, so its index names none of them.
    ParentMapSplit { entity: At, sources: Vec<String> },
    /// Its link into the parent map is past the entities that map holds.
    ParentPastTheEnd {
        entity: At,
        index: usize,
        len: usize,
    },
    /// It counts more children than the sources hold: the rest are in a map that is not among them, and lose their parent.
    ChildrenNotAmongSources {
        entity: At,
        counted: u32,
        found: usize,
    },
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let named = |at: &At| format!("{} entity {}", at.source, at.index);
        match self {
            Self::ParentMapNotAmongSources { entity, parent } => write!(
                f,
                "{} names its far model in the map {}, which is not among the sources: the far model stays, and counts a child it lost",
                named(entity),
                parent.shown()
            ),
            Self::NoParentMap { entity } => write!(
                f,
                "{} names a far model in a parent map, and its map names none",
                named(entity)
            ),
            Self::ParentMapSplit { entity, sources } => write!(
                f,
                "{} names its far model in a map that {} each hold entities of, so its link says none of them",
                named(entity),
                sources.join(" and ")
            ),
            Self::ParentPastTheEnd { entity, index, len } => write!(
                f,
                "{} names entity {index} of its parent map, which holds {len}",
                named(entity)
            ),
            Self::ChildrenNotAmongSources {
                entity,
                counted,
                found,
            } => write!(
                f,
                "{} counts {counted} children and the sources hold {found}: the others are in a map that is not among the sources, and lose their parent",
                named(entity)
            ),
        }
    }
}

/// An entity whose count of children the deletion lowers, by its index before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lowered {
    pub entity: usize,
    pub from: u32,
    pub to: u32,
}

/// What the deletion changes in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub source: String,
    /// The file it writes, inside the pack with `/`.
    pub file: String,
    /// The entities taken out, by their index before.
    pub removed: Vec<usize>,
    pub lowered: Vec<Lowered>,
    /// The LOD links of the entities that stay which name another index afterwards: inside the file, and into its parent map.
    pub renumbered: usize,
    pub renumbered_into_parent: usize,
}

/// A model at a place, which a script line hides in place of a deletion.
#[derive(Debug, Clone, PartialEq)]
pub struct Hidden {
    pub archetype: Name,
    pub position: [f32; 3],
}

/// The plan of a deletion: what goes, what stays and why, what is left alone, and every file it writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Deletion {
    /// The entities taken out: the chosen first, then what is under them, then the parents left with no child.
    pub goes: Vec<Going>,
    pub stays: Vec<Staying>,
    pub ambiguous: Vec<Ambiguous>,
    pub notes: Vec<Note>,
    /// Every file the operations write, and no other.
    pub files: Vec<FileChange>,
    /// The deletion as one list, which applies whole or not at all.
    pub operations: Vec<Operation>,
    /// The other way, which changes no file: each model a script hides at its place, for the chosen and what goes with them.
    pub hidden: Vec<Hidden>,
}

impl Deletion {
    /// The script lines that hide what the deletion would take out, within `radius` metres of each; none of a form that has no such line.
    pub fn script(
        &self,
        form: &ScriptForm,
        radius: f32,
        decimals: usize,
    ) -> Result<Vec<String>, script_text::Error> {
        let mut lines = Vec::with_capacity(self.hidden.len());
        for hidden in &self.hidden {
            let name = hidden.archetype.shown();
            let position = Vec3::from_array(hidden.position);
            let line = form.hide_model(&name, hidden.archetype.hash, position, radius, decimals)?;
            lines.extend(line);
        }
        Ok(lines)
    }
}

fn problems(found: Vec<or_source::Problem>) -> String {
    found
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// A source of a pack that cooks to a map file: what its folder holds, and the file.
pub(crate) struct MapSource {
    /// The source, as the pack names it.
    pub(crate) path: String,
    pub(crate) opened: or_source::asset::Source,
    pub(crate) inputs: Inputs,
    pub(crate) cooked: Vec<u8>,
}

/// The sources of `model` in `folder` that cook to a map file, by their name.
pub(crate) fn map_sources(folder: &Path, model: &Maps) -> Result<Vec<MapSource>, Error> {
    let mut sources = Vec::new();
    let mut paths: Vec<&str> = named_paths(model).collect();
    paths.sort_unstable();
    for path in paths {
        let unread = |why: String| Error::Source(path.to_string(), why);
        if !stays_inside(path) {
            return Err(unread("it leaves the folder of the pack".to_string()));
        }
        let dir = folder.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        // ⚠ Only what may be a map file refuses a deletion: a folder that names another kind is none, whether it opens or not, and
        // neither is a metadata source that says it cooks to another file, whatever else of it does not read.
        let another_kind = (path.rsplit('/').next())
            .and_then(or_source::asset::parse_folder_name)
            .is_some_and(|(_, kind)| kind != or_source::metadata::KIND);
        let source = match or_source::asset::Source::open(&dir) {
            Ok(source) => source,
            Err(_) if another_kind => continue,
            Err(why) => return Err(unread(why.to_string())),
        };
        if source.manifest.kind != or_source::metadata::KIND {
            continue;
        }
        let cooks_to = or_source::metadata::cooked_name(&source.manifest);
        if cooks_to.is_ok_and(|file| !file.ends_with(&format!(".{}", CMapData::EXTENSION))) {
            continue;
        }
        let inputs = Inputs::read(&source).map_err(|found| unread(problems(found)))?;
        if inputs.extension() != CMapData::EXTENSION {
            continue;
        }
        let cooked = (inputs.compile())
            .map_err(|found| unread(problems(found)))?
            .bytes;
        sources.push(MapSource {
            path: path.to_string(),
            opened: source,
            inputs,
            cooked,
        });
    }
    Ok(sources)
}

/// The map files the sources of `model` cook to, in `folder`, by the name of their source.
fn map_files(folder: &Path, model: &Maps) -> Result<Vec<MapFile>, Error> {
    (map_sources(folder, model)?.into_iter())
        .map(|source| {
            let unread = |why: String| Error::Source(source.path.clone(), why);
            MapFile::read(&source.path, &source.opened.manifest.name, source.cooked).map_err(unread)
        })
        .collect()
}

impl MapFile {
    /// The map file `bytes` hold, called `stem`, which the source `source` cooks to or is to be made of.
    fn read(source: &str, stem: &str, bytes: Vec<u8>) -> Result<Self, String> {
        let map = CMapData::from_bytes(&bytes).map_err(|e| e.to_string())?;
        let document = MetaDocument::parse(or_io::meta::default_library(), bytes)
            .map_err(|e| e.to_string())?;
        let container_lods =
            MapData::new(&document.instance).map_or(0, |data| data.container_lod_count());
        Ok(Self {
            source: source.to_string(),
            name: map_name(stem),
            map,
            container_lods,
        })
    }
}

/// A map file the pack does not hold, which a plan reads beside the pack's own: a file of the game the chain of a deletion leads into.
pub struct Reached(MapFile);

impl Reached {
    /// The map file `bytes` hold, under the name `source` its source is to have in the pack.
    pub fn read(source: &str, bytes: Vec<u8>) -> Result<Self, String> {
        let folder = source.rsplit('/').next().unwrap_or(source);
        let stem = or_source::asset::parse_folder_name(folder).map_or(folder, |(name, _)| name);
        MapFile::read(source, stem, bytes).map(Self)
    }

    /// The name its source is to have in the pack.
    pub fn source(&self) -> &str {
        &self.0.source
    }

    pub fn map(&self) -> &CMapData {
        &self.0.map
    }
}

/// What lies beyond the pack, which a plan asks for the map files its links lead into: the caller that has the mount answers.
// ⚠ An error stops the plan: a map file that is there and does not read may hold links a deletion would break.
pub trait Beyond {
    /// The map file of the map called `name`, by the hash of its file's name; none where there is no such map.
    fn named(&mut self, name: u32) -> Result<Option<Reached>, String>;

    /// The map files of the maps that name `parent` as their parent map.
    fn under(&mut self, parent: u32) -> Result<Vec<Reached>, String>;
}

/// An entity of the files: the file by its place in the list, and the index.
type Node = (usize, usize);

/// Where the LOD link of an entity leads.
enum Parent {
    At(Node),
    /// Into a parent map the sources do not resolve, with what to say of it.
    Unreached(Note),
}

/// The files of a pack with the LOD links of every entity followed.
struct Links<'a> {
    files: &'a [MapFile],
    /// Where the link of each entity leads, for the entities that have one.
    parents: BTreeMap<Node, Parent>,
    children: BTreeMap<Node, Vec<Node>>,
    /// The file that holds the entities of each map, by the name of its file, which the links into that map index.
    owner: BTreeMap<u32, usize>,
    /// The names that several files are each called and hold entities under, with those files.
    split: BTreeMap<u32, Vec<usize>>,
}

impl<'a> Links<'a> {
    fn of(files: &'a [MapFile], names: &dyn Fn(u32) -> Option<String>) -> Self {
        // The file that holds the entities of each map, by the name of its file: one with none indexes nothing.
        let mut holding: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (file, held) in files.iter().enumerate() {
            if !held.map.entities.is_empty() {
                holding.entry(held.name).or_default().push(file);
            }
        }
        let mut links = Self {
            files,
            parents: BTreeMap::new(),
            children: BTreeMap::new(),
            owner: BTreeMap::new(),
            split: BTreeMap::new(),
        };
        for (name, parts) in holding {
            if let [one] = parts.as_slice() {
                links.owner.insert(name, *one);
            } else {
                links.split.insert(name, parts);
            }
        }
        for (file, held) in files.iter().enumerate() {
            for (index, entity) in held.map.entities.iter().enumerate() {
                let Ok(named) = usize::try_from(entity.parent_index) else {
                    continue;
                };
                let at = || links.at((file, index));
                let parent = if !in_parent_map(entity.flags) {
                    // An entity that names itself, or past its own file, has no parent a deletion can follow.
                    if named == index || named >= held.map.entities.len() {
                        continue;
                    }
                    Parent::At((file, named))
                } else {
                    let parent = &held.map.parent;
                    let among = || files.iter().any(|other| other.name == parent.hash);
                    let len = (links.owner.get(&parent.hash))
                        .map_or(0, |owner| files[*owner].map.entities.len());
                    match (links.owner.get(&parent.hash), links.split.get(&parent.hash)) {
                        _ if parent.hash == 0 => {
                            Parent::Unreached(Note::NoParentMap { entity: at() })
                        }
                        (Some(owner), _) if named < len => Parent::At((*owner, named)),
                        (None, Some(parts)) => Parent::Unreached(Note::ParentMapSplit {
                            entity: at(),
                            sources: links.sources(parts),
                        }),
                        // A parent map among the sources that holds no entity is one whose every index is past its end.
                        (owner, _) if owner.is_some() || among() => {
                            Parent::Unreached(Note::ParentPastTheEnd {
                                entity: at(),
                                index: named,
                                len,
                            })
                        }
                        _ => Parent::Unreached(Note::ParentMapNotAmongSources {
                            entity: at(),
                            parent: name_of(parent, names),
                        }),
                    }
                };
                if let Parent::At(node) = &parent {
                    links.children.entry(*node).or_default().push((file, index));
                }
                links.parents.insert((file, index), parent);
            }
        }
        links
    }

    fn at(&self, (file, index): Node) -> At {
        At {
            source: self.files[file].source.clone(),
            index,
        }
    }

    fn sources(&self, files: &[usize]) -> Vec<String> {
        (files.iter())
            .map(|file| self.files[*file].source.clone())
            .collect()
    }

    fn entity(&self, (file, index): Node) -> &CEntityDefAny {
        &self.files[file].map.entities[index]
    }

    fn children(&self, node: Node) -> &[Node] {
        self.children.get(&node).map_or(&[], Vec::as_slice)
    }

    fn parent(&self, node: Node) -> Option<Node> {
        match self.parents.get(&node)? {
            Parent::At(parent) => Some(*parent),
            Parent::Unreached(_) => None,
        }
    }

    /// The children `node` counts that no source holds.
    fn children_elsewhere(&self, node: Node) -> u32 {
        let found = u32::try_from(self.children(node).len()).unwrap_or(u32::MAX);
        self.entity(node).num_children.saturating_sub(found)
    }
}

/// The name `stated` is, its text from the file when the file gives one that hashes to it, else from `names`.
fn name_of(stated: &HashedString, names: &dyn Fn(u32) -> Option<String>) -> Name {
    let hash = stated.hash;
    let known = |text: &String| or_io::string_hash(text.as_bytes()) == hash;
    let text = (Some(stated.raw.clone()).filter(|raw| !raw.is_empty() && known(raw)))
        .or_else(|| names(hash).filter(known));
    Name { hash, text }
}

/// Whether two entities are of one archetype at one pose.
fn twins(a: &CEntityDefAny, b: &CEntityDefAny) -> bool {
    let vec3 = |v: or_io::decl::Vec3| [v.x, v.y, v.z];
    let vec4 = |v: or_io::decl::Vec4| [v.x, v.y, v.z, v.w];
    a.archetype_name.hash == b.archetype_name.hash
        && vec3(a.position) == vec3(b.position)
        && vec4(a.rotation) == vec4(b.rotation)
        && a.scale_xy == b.scale_xy
        && a.scale_z == b.scale_z
}

/// What the plan says of the file `file`, made when it is first said to change.
fn change<'a>(
    changes: &'a mut BTreeMap<usize, FileChange>,
    files: &[MapFile],
    file: usize,
) -> &'a mut FileChange {
    changes.entry(file).or_insert_with(|| {
        let source = files[file].source.clone();
        FileChange {
            file: format!("{source}/{}", or_source::metadata::XML),
            source,
            removed: Vec::new(),
            lowered: Vec::new(),
            renumbered: 0,
            renumbered_into_parent: 0,
        }
    })
}

/// The plan that takes the entities `chosen` out of `files`, the map files of one pack, with `names` giving the text of a hash.
fn plan_over(
    files: &[MapFile],
    chosen: &[At],
    names: &dyn Fn(u32) -> Option<String>,
) -> Result<Deletion, Error> {
    if chosen.is_empty() {
        return Err(Error::Nothing);
    }
    let links = Links::of(files, names);
    let entity = |node: Node| {
        let held = links.entity(node);
        Entity {
            at: links.at(node),
            archetype: name_of(&held.archetype_name, names),
            position: [held.position.x, held.position.y, held.position.z],
        }
    };

    let mut asked: Vec<Node> = Vec::new();
    for at in chosen {
        let file = (files.iter().position(|held| held.source == at.source))
            .ok_or_else(|| Error::NoSuchSource(at.source.clone()))?;
        let len = files[file].map.entities.len();
        if at.index >= len {
            return Err(Error::NoSuchEntity {
                at: at.source.clone(),
                index: at.index,
                len,
            });
        }
        if !asked.contains(&(file, at.index)) {
            asked.push((file, at.index));
        }
    }

    // A chosen entity with a twin that is not chosen too is left.
    let mut unsure: Vec<(Node, Vec<Node>)> = Vec::new();
    let mut roots = Vec::new();
    for &node in &asked {
        let held = links.entity(node);
        let alike: Vec<Node> = (files.iter().enumerate())
            .flat_map(|(file, other)| (0..other.map.entities.len()).map(move |index| (file, index)))
            .filter(|other| *other != node && twins(held, links.entity(*other)))
            .collect();
        if alike.iter().all(|twin| asked.contains(twin)) {
            roots.push(node);
        } else {
            unsure.push((node, alike));
        }
    }

    // Down the tree: what names an entity that goes, goes.
    let mut goes: Vec<(Node, Why)> = roots.iter().map(|root| (*root, Why::Chosen)).collect();
    let mut doomed: BTreeSet<Node> = roots.iter().copied().collect();
    let mut next = 0;
    while let Some((node, _)) = goes.get(next).cloned() {
        next += 1;
        for &child in links.children(node) {
            if doomed.insert(child) {
                goes.push((child, Why::Under(links.at(node))));
            }
        }
    }
    // Up the chain: a parent every child of which goes, and that counts none elsewhere, goes too.
    let mut next = 0;
    while let Some((node, _)) = goes.get(next).cloned() {
        next += 1;
        let Some(parent) = links.parent(node).filter(|parent| !doomed.contains(parent)) else {
            continue;
        };
        let left = (links.children(parent).iter()).any(|child| !doomed.contains(child));
        if !left && links.children_elsewhere(parent) == 0 {
            doomed.insert(parent);
            goes.push((parent, Why::LeftWithNoChild));
        }
    }

    // ⚠ A twin that the chain of another entity takes is not left, and is not said to be.
    let ambiguous: Vec<Ambiguous> = (unsure.into_iter())
        .filter(|(node, _)| !doomed.contains(node))
        .map(|(node, alike)| Ambiguous {
            entity: entity(node),
            twins: alike.into_iter().map(|twin| links.at(twin)).collect(),
        })
        .collect();

    let mut notes = Vec::new();
    // The parents that stay, each with the children it loses by a link inside its file and by a link from a map whose parent its own is.
    let mut losing: BTreeMap<Node, (u32, u32)> = BTreeMap::new();
    for &(node, _) in &goes {
        match links.parents.get(&node) {
            Some(Parent::At(parent)) if !doomed.contains(parent) => {
                let (here, below) = losing.entry(*parent).or_default();
                if in_parent_map(links.entity(node).flags) {
                    *below += 1;
                } else {
                    *here += 1;
                }
            }
            Some(Parent::Unreached(note)) => notes.push(note.clone()),
            _ => {}
        }
        let elsewhere = links.children_elsewhere(node);
        if elsewhere > 0 {
            notes.push(Note::ChildrenNotAmongSources {
                entity: links.at(node),
                counted: links.entity(node).num_children,
                found: links.children(node).len(),
            });
        }
    }
    let stays: Vec<Staying> = (losing.iter())
        .map(|(&parent, &(here, below))| {
            let counted = links.entity(parent).num_children;
            let left = (links.children(parent).iter()).filter(|child| !doomed.contains(child));
            Staying {
                entity: entity(parent),
                children_left: left.count(),
                children_elsewhere: links.children_elsewhere(parent),
                counted,
                counts: counted.saturating_sub(here).saturating_sub(below),
            }
        })
        .collect();

    // What each file loses, and the list that does it: the counts first, by the indexes the files have now, then the entities, then the links into a map that lost some.
    let mut removed: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(file, index) in &doomed {
        removed.entry(file).or_default().push(index);
    }
    for file in removed.keys() {
        let held = &files[*file];
        if held.container_lods > 0 {
            return Err(Error::ContainerLods(held.source.clone()));
        }
        if let Some(parts) = links.split.get(&held.name) {
            return Err(Error::SplitMap(links.sources(parts)));
        }
    }
    let mut changes: BTreeMap<usize, FileChange> = BTreeMap::new();
    let mut operations = Vec::new();
    for (&(file, index), &(here, below)) in &losing {
        let from = links.entity((file, index)).num_children;
        let to = from.saturating_sub(here).saturating_sub(below);
        if to != from {
            change(&mut changes, files, file).lowered.push(Lowered {
                entity: index,
                from,
                to,
            });
        }
        // ⚠ A child lost by a link inside the file is counted off by the removal there, which this must not count off again.
        let stated = from.saturating_sub(below);
        if stated != from {
            operations.push(Operation::SetNumChildren {
                source: files[file].source.clone(),
                expects: None,
                entity: index,
                num_children: stated,
            });
        }
    }
    // A link names another index afterwards when an entity before the one it names goes.
    let moves = |node: Node, into_parent: bool, gone: &[usize]| {
        !doomed.contains(&node)
            && in_parent_map(links.entity(node).flags) == into_parent
            && matches!(links.parent(node), Some((_, parent)) if gone.first().is_some_and(|first| *first < parent))
    };
    for (&file, gone) in &removed {
        let renumbered = (0..files[file].map.entities.len())
            .filter(|index| moves((file, *index), false, gone))
            .count();
        let changed = change(&mut changes, files, file);
        changed.removed = gone.clone();
        changed.renumbered = renumbered;
        operations.push(Operation::RemoveEntities {
            source: files[file].source.clone(),
            expects: None,
            entities: gone.clone(),
            extents: None,
        });
    }
    for (file, held) in files.iter().enumerate() {
        // What the file that holds the entities of this one's parent map loses.
        let parent_map = links.owner.get(&held.map.parent.hash);
        let Some(gone) = parent_map.and_then(|into| removed.get(into)) else {
            continue;
        };
        let renumbered = (0..held.map.entities.len())
            .filter(|index| moves((file, *index), true, gone))
            .count();
        if renumbered == 0 {
            continue;
        }
        change(&mut changes, files, file).renumbered_into_parent = renumbered;
        operations.push(Operation::RenumberForRemoved {
            source: held.source.clone(),
            expects: None,
            removed: gone.clone(),
        });
    }

    let mut hidden: Vec<Hidden> = Vec::new();
    let left = ambiguous.iter().map(|left| left.entity.clone());
    for named in (goes.iter().map(|(node, _)| entity(*node))).chain(left) {
        let place = Hidden {
            archetype: named.archetype,
            position: named.position,
        };
        if !hidden.contains(&place) {
            hidden.push(place);
        }
    }
    Ok(Deletion {
        goes: (goes.into_iter())
            .map(|(node, why)| Going {
                entity: entity(node),
                why,
            })
            .collect(),
        stays,
        ambiguous,
        notes,
        files: changes.into_values().collect(),
        operations,
        hidden,
    })
}

/// The plan that takes the entities `chosen` out of the map files of the pack of maps in `folder`; nothing is written.
pub fn plan(folder: &Path, chosen: &[At]) -> Result<Deletion, Error> {
    let model = Maps::read(folder)?;
    let files = map_files(folder, &model)?;
    plan_over(&files, chosen, &|hash| model.name_of(hash))
}

/// [`plan`] over the map files of the pack and the map files `reached` beside them, with every map file `beyond` holds that the chain leads into; the plan, and the map files read beside the pack's.
// A map file is asked of `beyond` when an entity that goes names a parent in it, and when a file loses an entity, whose index the maps
// under it may name and whose children they may hold. A map the files already hold under its name is not taken again, so a source of
// the pack stands for the game's file it was made of.
//
// ⚠ The operations name a map file read beside the pack's by the source it is to have: they apply once it is one.
pub fn plan_reaching(
    folder: &Path,
    reached: Vec<Reached>,
    chosen: &[At],
    beyond: &mut dyn Beyond,
) -> Result<(Deletion, Vec<Reached>), Error> {
    let model = Maps::read(folder)?;
    let mut files = map_files(folder, &model)?;
    let own = files.len();
    files.extend(reached.into_iter().map(|reached| reached.0));
    let (mut named, mut under): (BTreeSet<u32>, BTreeSet<u32>) = Default::default();
    let planned = loop {
        let planned = plan_over(&files, chosen, &|hash| model.name_of(hash))?;
        let map_of =
            |source: &str| (files.iter().find(|file| file.source == source)).map(|file| file.name);
        let mut found: Vec<Reached> = Vec::new();
        for note in &planned.notes {
            match note {
                Note::ParentMapNotAmongSources { parent, .. } if named.insert(parent.hash) => {
                    found.extend(beyond.named(parent.hash).map_err(Error::Beyond)?);
                }
                _ => {}
            }
        }
        // ⚠ The maps that hold the children an entity that goes counts are among these: its file loses it.
        for change in planned
            .files
            .iter()
            .filter(|change| !change.removed.is_empty())
        {
            if let Some(map) = map_of(&change.source).filter(|map| under.insert(*map)) {
                found.extend(beyond.under(map).map_err(Error::Beyond)?);
            }
        }
        let before = files.len();
        for reached in found {
            let held =
                |file: &MapFile| file.name == reached.0.name || file.source == reached.0.source;
            if !files.iter().any(held) {
                files.push(reached.0);
            }
        }
        if files.len() == before {
            break planned;
        }
    };
    Ok((planned, files.drain(own..).map(Reached).collect()))
}

/// Do the deletion `planned` to the sources of the pack of maps in `folder`, whole or not at all; the inverses of what it did, the last first, put every file back.
pub fn apply(folder: &Path, planned: &Deletion) -> Result<Vec<Applied>, operation::Error> {
    operation::apply_all(folder, &planned.operations, false)
}

/// Do the deletion `planned` in `session`, whole or not at all: an operation the session refuses leaves every root as it was, and nothing is written until the session is flushed.
pub fn apply_in(
    session: &mut operation::Session,
    planned: &Deletion,
) -> Result<Vec<operation::Done>, operation::Error> {
    session.apply_all(&planned.operations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{child_map_file, map_file, Entity};

    const BOX: ([f32; 3], [f32; 3]) = ([0.0; 3], [100.0; 3]);

    fn file(source: &str, bytes: &[u8]) -> MapFile {
        // A source is named here by the stem of its file, behind any folder.
        MapFile {
            source: source.to_string(),
            name: map_name(source.rsplit('/').next().unwrap_or(source)),
            map: CMapData::from_bytes(bytes).expect("a sample map opens"),
            container_lods: 0,
        }
    }

    fn at(source: &str, index: usize) -> At {
        At {
            source: source.to_string(),
            index,
        }
    }

    fn unnamed(_: u32) -> Option<String> {
        None
    }

    /// An entity under the entity `parent` of the parent map.
    fn under(guid: u32, parent: i32) -> Entity<'static> {
        Entity {
            parent,
            in_parent_map: true,
            ..Entity::at("prop_crate_a", [guid as f32, 0.0, 0.0], guid)
        }
    }

    /// `far`, whose one entity stands for the two of `near`, the map under it.
    fn far_and_near() -> Vec<MapFile> {
        let stands = Entity {
            children: 2,
            ..Entity::at("prop_crate_far", [0.0; 3], 1)
        };
        vec![
            file("far", &map_file("far", &[stands], BOX)),
            file(
                "near",
                &child_map_file("near", "far", &[under(2, 0), under(3, 0)], BOX),
            ),
        ]
    }

    #[test]
    fn a_file_that_carries_container_lods_is_refused_where_entities_would_leave_it_and_not_where_a_count_is_lowered(
    ) {
        let mut files = far_and_near();
        files[0].container_lods = 1;
        let planned =
            plan_over(&files, &[at("near", 0)], &unnamed).expect("no index of `far` moves");
        assert_eq!(planned.goes.len(), 1);
        assert_eq!(planned.files.len(), 2);

        files[1].container_lods = 1;
        let refused = plan_over(&files, &[at("near", 0)], &unnamed).unwrap_err();
        assert!(
            matches!(&refused, Error::ContainerLods(source) if source == "near"),
            "{refused}"
        );
        // Both of `near` chosen take `far`'s entity with them, and `far` carries container LODs too.
        files[1].container_lods = 0;
        let refused = plan_over(&files, &[at("near", 0), at("near", 1)], &unnamed).unwrap_err();
        assert!(
            matches!(&refused, Error::ContainerLods(source) if source == "far"),
            "{refused}"
        );
    }

    #[test]
    fn two_files_that_each_hold_entities_of_one_map_are_refused_and_a_part_that_holds_none_is_not_one_of_them(
    ) {
        // Each part stands its entity elsewhere: two at one pose would be twins, which is another refusal.
        let part = |guid: u32| {
            let stands = Entity::at("prop_crate_far", [guid as f32, 0.0, 0.0], guid);
            map_file("far", &[stands], BOX)
        };
        let near = child_map_file("near", "far", &[under(2, 0)], BOX);
        // Two sources of one pack that each cook to `far.ymap`, in two folders.
        let split = vec![
            file("a/far", &part(1)),
            file("b/far", &part(9)),
            file("near", &near),
        ];
        let refused = plan_over(&split, &[at("a/far", 0)], &unnamed).unwrap_err();
        assert!(
            matches!(&refused, Error::SplitMap(sources) if sources == &["a/far", "b/far"]),
            "{refused}"
        );
        // A link into the split map names neither file: the entity goes alone, and the plan says what it could not follow.
        let planned = plan_over(&split, &[at("near", 0)], &unnamed).unwrap();
        assert_eq!(planned.goes.len(), 1);
        assert_eq!(
            planned.notes,
            [Note::ParentMapSplit {
                entity: at("near", 0),
                sources: vec!["a/far".to_string(), "b/far".to_string()],
            }]
        );

        // A second file of the name that holds no entity: the links index the one that holds some.
        let stub = map_file("far", &[], BOX);
        let parts = vec![
            file("a/far", &part(1)),
            file("b/far", &stub),
            file("near", &near),
        ];
        let planned = plan_over(&parts, &[at("near", 0)], &unnamed).unwrap();
        let gone: Vec<At> = planned
            .goes
            .iter()
            .map(|going| going.entity.at.clone())
            .collect();
        assert_eq!(gone, [at("near", 0), at("a/far", 0)]);
        assert_eq!(planned.notes, Vec::new());

        // ⚠ A file that states the name of another map is not a part of it: a map is the file called so, and the link names no source.
        let stating = vec![file("depot", &part(1)), file("near", &near)];
        let planned = plan_over(&stating, &[at("near", 0)], &unnamed).unwrap();
        assert_eq!(planned.goes.len(), 1);
        assert!(
            matches!(
                planned.notes.as_slice(),
                [Note::ParentMapNotAmongSources { .. }]
            ),
            "{:?}",
            planned.notes
        );
    }

    #[test]
    fn a_link_that_leads_nowhere_is_said_and_followed_no_further() {
        let lone = Entity {
            parent: 0,
            in_parent_map: true,
            ..Entity::at("prop_crate_a", [1.0, 0.0, 0.0], 2)
        };
        let files = vec![
            file(
                "far",
                &map_file("far", &[Entity::at("prop_crate_far", [0.0; 3], 1)], BOX),
            ),
            file("near", &child_map_file("near", "far", &[under(2, 1)], BOX)),
            file("alone", &map_file("alone", &[lone], BOX)),
            file("empty", &map_file("empty", &[], BOX)),
            file(
                "over_empty",
                &child_map_file("over_empty", "empty", &[under(4, 0)], BOX),
            ),
        ];
        let chosen = [at("near", 0), at("alone", 0), at("over_empty", 0)];
        let planned = plan_over(&files, &chosen, &unnamed).unwrap();
        assert_eq!(planned.goes.len(), 3);
        assert_eq!(
            planned.notes,
            [
                // The first index past the one entity `far` holds.
                Note::ParentPastTheEnd {
                    entity: at("near", 0),
                    index: 1,
                    len: 1,
                },
                Note::NoParentMap {
                    entity: at("alone", 0),
                },
                Note::ParentPastTheEnd {
                    entity: at("over_empty", 0),
                    index: 0,
                    len: 0,
                },
            ]
        );
        assert_eq!(planned.stays, Vec::new());
    }

    #[test]
    fn a_link_inside_a_file_that_names_its_own_entity_or_none_of_the_file_is_no_parent() {
        let own = Entity {
            parent: 0,
            ..Entity::at("prop_crate_a", [1.0, 0.0, 0.0], 1)
        };
        let past = Entity {
            parent: 2,
            ..Entity::at("prop_crate_a", [2.0, 0.0, 0.0], 2)
        };
        let files = vec![file("lone", &map_file("lone", &[own, past], BOX))];
        for index in [0, 1] {
            let planned = plan_over(&files, &[at("lone", index)], &unnamed).unwrap();
            let gone: Vec<At> = (planned.goes.iter())
                .map(|going| going.entity.at.clone())
                .collect();
            assert_eq!(gone, [at("lone", index)]);
            assert_eq!(planned.stays, Vec::new());
            assert_eq!(planned.notes, Vec::new());
        }
    }

    /// `far`, whose one entity stands for the entities of `near` given, each at the place of `Entity::at` unless a case moves it.
    fn far_over(near: &[Entity<'static>]) -> Vec<MapFile> {
        let stands = Entity {
            children: near.len() as u32,
            ..Entity::at("prop_crate_far", [0.0; 3], 1)
        };
        vec![
            file("far", &map_file("far", &[stands], BOX)),
            file("near", &child_map_file("near", "far", near, BOX)),
        ]
    }

    /// An entity of `near` under the one of `far`, at one place whatever its guid.
    fn alike(guid: u32) -> Entity<'static> {
        Entity {
            parent: 0,
            in_parent_map: true,
            ..Entity::at("prop_crate_a", [5.0, 6.0, 7.0], guid)
        }
    }

    fn left(planned: &Deletion) -> Vec<(At, Vec<At>)> {
        (planned.ambiguous.iter())
            .map(|left| (left.entity.at.clone(), left.twins.clone()))
            .collect()
    }

    fn gone(planned: &Deletion) -> Vec<(At, Why)> {
        (planned.goes.iter())
            .map(|going| (going.entity.at.clone(), going.why.clone()))
            .collect()
    }

    #[test]
    fn a_twin_is_of_one_archetype_at_one_place_turn_and_scale_and_a_difference_in_any_tells_two_apart(
    ) {
        let files = far_over(&[alike(2), alike(3)]);
        let planned = plan_over(&files, &[at("near", 0)], &unnamed).unwrap();
        assert_eq!(left(&planned), [(at("near", 0), vec![at("near", 1)])]);
        assert_eq!(planned.goes, Vec::new());

        type Change = fn(&mut CEntityDefAny);
        let changes: [(&str, Change); 7] = [
            ("the archetype", |e| {
                e.archetype_name = HashedString::new("prop_crate_b")
            }),
            ("x", |e| e.position.x += 0.5),
            ("y", |e| e.position.y += 0.5),
            ("z", |e| e.position.z += 0.5),
            ("the turn", |e| {
                let turned = glam::Quat::from_rotation_z(0.5).to_array();
                [e.rotation.x, e.rotation.y, e.rotation.z, e.rotation.w] = turned;
            }),
            ("the scale across", |e| e.scale_xy = 2.0),
            ("the scale up", |e| e.scale_z = 2.0),
        ];
        for (what, change) in changes {
            let mut files = far_over(&[alike(2), alike(3)]);
            change(&mut files[1].map.entities[1]);
            let planned = plan_over(&files, &[at("near", 0)], &unnamed).unwrap();
            assert_eq!(left(&planned), Vec::new(), "{what} tells the two apart");
            assert_eq!(
                gone(&planned),
                [(at("near", 0), Why::Chosen)],
                "{what} tells the two apart"
            );
        }
    }

    #[test]
    fn an_entity_with_two_twins_is_left_until_all_three_are_chosen() {
        let files = far_over(&[alike(2), alike(3), alike(4)]);
        let two = [at("near", 0), at("near", 2)];
        let planned = plan_over(&files, &two, &unnamed).unwrap();
        assert_eq!(
            left(&planned),
            [
                (at("near", 0), vec![at("near", 1), at("near", 2)]),
                (at("near", 2), vec![at("near", 0), at("near", 1)]),
            ]
        );
        assert_eq!(planned.goes, Vec::new());
        assert_eq!(planned.operations, Vec::new());
        // The one place is offered to a script once, whichever of the three was named.
        assert_eq!(planned.hidden.len(), 1);

        let all = [at("near", 0), at("near", 2), at("near", 1)];
        let planned = plan_over(&files, &all, &unnamed).unwrap();
        assert_eq!(left(&planned), Vec::new());
        assert_eq!(planned.goes.len(), 4, "the three and the far model");
        assert_eq!(planned.hidden.len(), 2, "the three are one place to hide");
    }

    #[test]
    fn a_twin_that_goes_with_the_chain_of_another_entity_is_not_also_said_to_be_left() {
        // The far model is chosen with one of two twins under it: both twins go under it, so none is left.
        let files = far_over(&[alike(2), alike(3)]);
        let planned = plan_over(&files, &[at("near", 0), at("far", 0)], &unnamed).unwrap();
        assert_eq!(left(&planned), Vec::new());
        assert_eq!(
            gone(&planned),
            [
                (at("far", 0), Why::Chosen),
                (at("near", 0), Why::Under(at("far", 0))),
                (at("near", 1), Why::Under(at("far", 0))),
            ]
        );

        // A far model with a twin in another map, chosen with the one entity it stands for: it is left standing for nothing, and goes.
        let stands = |guid: u32| Entity {
            children: 1,
            ..Entity::at("prop_crate_far", [0.0; 3], guid)
        };
        let files = vec![
            file("far", &map_file("far", &[stands(1)], BOX)),
            file("near", &child_map_file("near", "far", &[under(2, 0)], BOX)),
            file("other", &map_file("other", &[stands(9)], BOX)),
        ];
        let planned = plan_over(&files, &[at("far", 0), at("near", 0)], &unnamed).unwrap();
        assert_eq!(left(&planned), Vec::new());
        assert_eq!(
            gone(&planned),
            [
                (at("near", 0), Why::Chosen),
                (at("far", 0), Why::LeftWithNoChild),
            ]
        );
        // Alone, the far model with a twin is left, and what it stands for stays under it.
        let planned = plan_over(&files, &[at("far", 0)], &unnamed).unwrap();
        assert_eq!(left(&planned), [(at("far", 0), vec![at("other", 0)])]);
        assert_eq!(planned.goes, Vec::new());
    }
}
