//! Putting an entity of a map source back as the base its source keeps has it, as a plan that is data.
//!
//! [`plan`] compares the map sources of a pack with their bases the way [`crate::changes`] does, and says for the entity named what
//! makes it state what the base states again, as a list of [`crate::operation`] that [`apply`] does whole or not at all and whose
//! inverses, the last first, put every source back as it was edited.
//!
//! An entity both files hold is put back by one record in its place: every member as the base states it, its LOD parent the entity
//! the base names, found through the matching. An entity the map file lacks comes back at the place the order of the base gives it,
//! and with it the chain a deletion took ([`crate::delete`]): each far model above it that the map file lacks too, in its own file
//! or in the map its file names as parent, and each entity the base places under it that the map file lacks. An entity the base
//! places under one that comes back, and that names no parent now, names it again.
//!
//! ⚠ **A count of children follows the entities that name it.** An entity that comes back counts the children the base counts less
//! the ones the sources still lack, and a far model that stays counts one more for each entity that comes back under it, so a chain
//! put back whole counts what the base counts and one put back in part counts what stands under it.
//!
//! ⚠ **The extents are the base's, grown to hold what stands otherwise.** A file an entity is put back into states the extents its
//! base states, grown to hold each entity it places that the base does not place the same way: a map put back whole states the
//! extents of the game's file, short where the game's own are.
//!
//! ⚠ **Nothing is guessed.** An entity that cannot be told from a twin is refused, and so is one whose far model cannot be; an
//! entity the base lacks was added, and putting things back is then taking it out, which is a deletion and is said so.
//!
//! ⚠ **A plan is of the sources as they were read.** The first operation of each source states the digest of the XML the plan was
//! made of ([`operation::digest_of`]), so a plan applied after another edit of one of its sources is refused whole.
//!
//! ⚠ **An entity found by its guid is put back where it stands.** An entity added after the one with the largest guid of its map
//! went takes that guid, and compares as that entity changed: putting it back makes it state what the base states, at the index the
//! added one has, so the map then states what its base states in another order.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use or_io::decl::File;
use or_io::meta::typed::{CEntityDefAny, CMapData};
use or_source::metadata::Inputs;
use serde::Serialize;

use crate::changes::{
    self, entity_ref, link_of, map_changes_paired, EntityChange, EntityRef, Link,
};
use crate::delete;
use crate::edit::{grow_extents, in_parent_map};
use crate::facts::map_name;
use crate::model::{self, Maps};
use crate::operation::{
    self, apply_to_map, boxes_of, digest_of, Applied, At, Boxes, Counted, Extents, Inserted,
    Operation, Record,
};

/// Why nothing was planned to be put back.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("the source `{at}` is not read, and it may be a map file, whose links a plan follows: {why}")]
    Unread { at: String, why: String },
    #[error("{0}")]
    Sources(delete::Error),
    #[error("no map file of the pack is the source `{0}`")]
    NoSuchSource(String),
    #[error("`{at}`: {why}")]
    Compare {
        /// The source that could not be compared with its base.
        at: String,
        why: changes::Error,
    },
    #[error("{of}`{at}` holds {len} entities, so there is no entity {index}")]
    NoSuchEntity {
        at: String,
        index: usize,
        len: usize,
        /// `the base of ` for an entity named by its index in the base, and nothing for one named by its index now.
        of: &'static str,
    },
    #[error(
        "`{at}` entity {index} is not in the base: the source added it, so putting the map back as the game has it is taking that entity out, which `openrage-cli maps delete` plans"
    )]
    Added { at: String, index: usize },
    #[error(
        "entity {index} of {of}`{at}` cannot be told from a twin: entities {} of the base and entities {} of the map file place one archetype and nothing says which is which, so none is put back by a guess",
        said(.base), said(.now)
    )]
    Ambiguous {
        at: String,
        index: usize,
        of: &'static str,
        /// The entities of the base and of the map file in its group.
        base: Vec<usize>,
        now: Vec<usize>,
    },
    #[error(
        "the base places entity {index} of `{at}` under entity {parent} of `{above}`, which cannot be told from a twin, so it is not known which entity to put it back under"
    )]
    FarModelAmbiguous {
        at: String,
        index: usize,
        /// The source that holds the far model, and its index in the base of that source.
        above: String,
        parent: usize,
    },
    #[error(
        "`{at}` entity {index} states what the base states of it, or differs only by a count of children that follows the entities under it, so there is nothing to put back"
    )]
    Unchanged { at: String, index: usize },
    #[error("{} each hold entities of one map, so an index into it does not say which file", .0.join(" and "))]
    SplitMap(Vec<String>),
    #[error("{name} of `{at}`: {why}")]
    Refused {
        /// The operation of the plan that could not be done to the map file as it is.
        name: &'static str,
        at: String,
        why: String,
    },
    #[error(
        "`{at}` cannot be written with the entity put back: {why}; and the file of its base with the entities of the source would not state what the source states otherwise"
    )]
    Unwritable { at: String, why: String },
}

fn said(indexes: &[usize]) -> String {
    let said: Vec<String> = indexes.iter().map(usize::to_string).collect();
    said.join(", ")
}

fn problems(found: Vec<or_source::Problem>) -> String {
    let said: Vec<String> = found.iter().map(ToString::to_string).collect();
    said.join("; ")
}

/// The entity to put back: one the map file holds, by its index there, or one of the base, by its index in the base, which is how a deleted one is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    Now(At),
    Base(At),
}

/// Why an entity comes back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
    /// It was asked for.
    Chosen,
    /// It is the far model of an entity that comes back or is put back, by the base.
    FarModel,
    /// The base places it under an entity that comes back: the source of that one, and its index in the base.
    Under { source: String, index: usize },
}

/// An entity the map file lacks that comes back.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Returning {
    pub source: String,
    /// What the base states of it, with its index in the base.
    pub base: EntityRef,
    /// Its index in the map file once every entity of the plan is in.
    pub index: usize,
    pub why: Why,
}

/// An entity the map file holds that is put back as the base states it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Restored {
    pub source: String,
    pub base: EntityRef,
    pub now: EntityRef,
    /// What it states differently, as [`crate::changes`] names it for one source read alone.
    pub changes: Vec<EntityChange>,
}

/// An entity of the base under one that comes back, which stays out because it cannot be told from a twin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Left {
    pub source: String,
    pub base: EntityRef,
}

/// An entity the map file keeps whose count of children the plan changes, by its index once the plan is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recounted {
    pub source: String,
    pub entity: usize,
    pub from: u32,
    pub to: u32,
}

/// The plan that puts one entity back: what is put back and what comes back with it, and the list of operations that does it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PutBack {
    pub restored: Option<Restored>,
    /// The entities that come back: the chosen first, then what the base places under it, then its far models.
    pub returns: Vec<Returning>,
    pub left: Vec<Left>,
    pub recounted: Vec<Recounted>,
    /// A link of the base that names nothing, which is stated as it is, and what the extents were grown without, a sentence each.
    pub notes: Vec<String>,
    /// Every source the operations write, and no other.
    pub sources: Vec<String>,
    /// The plan as one list, which applies whole or not at all.
    pub operations: Vec<Operation>,
}

/// An entity of the bases: the file by its place in the list, and the index in the base of that file.
type Node = (usize, usize);

/// What an entity of a base is in the map file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Was {
    /// The entity at this index.
    Is(usize),
    Deleted,
    /// One of a group nothing tells apart.
    Unsure,
}

/// Where the base says the LOD parent of an entity is.
enum Above {
    None,
    At(Node),
    /// In a map the sources do not resolve, so the link is stated as the base states it, with what to say of that when the base itself names nothing by it.
    Beyond(Option<String>),
}

/// A map source compared with its base.
struct Compared {
    base: CMapData,
    /// The file the base cooks to.
    cooked: Vec<u8>,
    changes: changes::MapChanges,
    now_of: Vec<Option<usize>>,
    base_of: Vec<Option<usize>>,
    /// The group of each entity of the base and of the map file that nothing tells from a twin, by its index.
    unsure_base: BTreeMap<usize, usize>,
    unsure_now: BTreeMap<usize, usize>,
}

impl Compared {
    fn of(base: CMapData, cooked: Vec<u8>, now: &CMapData) -> Self {
        let (changes, now_of) = map_changes_paired(&base, now);
        let mut base_of = vec![None; now.entities.len()];
        for (index, now) in now_of.iter().enumerate() {
            if let Some(slot) = now.and_then(|now| base_of.get_mut(now)) {
                *slot = Some(index);
            }
        }
        let (mut unsure_base, mut unsure_now) = (BTreeMap::new(), BTreeMap::new());
        for (group, held) in changes.ambiguous.iter().enumerate() {
            unsure_base.extend(held.base.iter().map(|entity| (entity.index, group)));
            unsure_now.extend(held.now.iter().map(|entity| (entity.index, group)));
        }
        Self {
            base,
            cooked,
            changes,
            now_of,
            base_of,
            unsure_base,
            unsure_now,
        }
    }
}

/// One map source of a pack: the file it cooks to, and its base once that is asked for.
struct MapFile {
    source: String,
    /// The hash of the name of the file it cooks to, which is the map's name to the game and to the maps under it (`crate::facts::map_name`).
    name: u32,
    opened: or_source::asset::Source,
    inputs: Inputs,
    now: CMapData,
    compared: OnceCell<Result<Compared, changes::Error>>,
}

impl MapFile {
    fn compare(&self) -> Result<Compared, changes::Error> {
        let cooked = or_source::metadata::compile_base(&self.opened)
            .map_err(|found| changes::Error::Base(problems(found)))?
            .bytes;
        let base = CMapData::from_bytes(&cooked).map_err(|e| changes::Error::Read {
            what: "the base",
            root: "map file",
            why: e.to_string(),
        })?;
        Ok(Compared::of(base, cooked, &self.now))
    }

    /// The source compared with its base, which is cooked the first time it is asked for.
    fn compared(&self) -> Result<&Compared, Error> {
        let compared = self.compared.get_or_init(|| self.compare());
        compared.as_ref().map_err(|why| Error::Compare {
            at: self.source.clone(),
            why: why.clone(),
        })
    }
}

/// Where the entities of a file stand once the ones that come back are in.
struct Order {
    /// The index of each entity the map file holds.
    of_now: Vec<usize>,
    /// The index of each entity that comes back, by its index in the base.
    of_base: BTreeMap<usize, usize>,
}

impl Order {
    /// Each entity that comes back stands after the last entity of the base before it that the file then holds, so a file put back whole is in the order of its base.
    fn of(compared: &Compared, held: usize, coming: &BTreeSet<usize>) -> Self {
        // An entity the map file holds by its index there, and one that comes back by its index in the base.
        let mut sequence: Vec<Result<usize, usize>> = (0..held).map(Ok).collect();
        for &index in coming {
            let before = (0..index)
                .rev()
                .find(|at| coming.contains(at) || compared.now_of[*at].is_some());
            let after = before.and_then(|at| {
                let anchor = compared.now_of[at].ok_or(at);
                sequence.iter().position(|held| *held == anchor)
            });
            sequence.insert(after.map_or(0, |at| at + 1), Err(index));
        }
        let mut order = Self {
            of_now: vec![0; held],
            of_base: BTreeMap::new(),
        };
        for (at, entity) in sequence.into_iter().enumerate() {
            match entity {
                Ok(now) => order.of_now[now] = at,
                Err(base) => {
                    order.of_base.insert(base, at);
                }
            }
        }
        order
    }
}

/// What is to be put back, before the counts of children are known.
struct Draft {
    /// The entity the map file holds that is put back: its file, its index in the base and its index now.
    restored: Option<(Node, usize)>,
    /// Whether that entity keeps the parent it names, which is among the twins of the one the base names.
    keeps_parent: bool,
    returns: Vec<(Node, Why)>,
    orders: BTreeMap<usize, Order>,
}

/// The counts of children the plan states.
#[derive(Default)]
struct Counts {
    /// What each entity that comes back, and the one put back, counts.
    of_base: BTreeMap<Node, u32>,
    /// What an entity the file keeps counts once the entities that come back under it in its own file are in, by its file and its index now.
    with_returns: BTreeMap<Node, u32>,
    /// What an entity the file keeps counts in the end, by its file and its index now.
    in_the_end: BTreeMap<Node, u32>,
}

/// The operations of a plan with the map files as they leave them.
struct Built {
    operations: Vec<(usize, Operation)>,
    after: BTreeMap<usize, CMapData>,
}

/// `index` as a LOD link states it.
fn link(index: usize) -> Result<i32, String> {
    i32::try_from(index).map_err(|_| format!("entity {index} is past what a LOD link can name"))
}

/// A count of children that is `stated` with `more` entities naming it, never below nought.
fn recount(stated: u32, more: i64) -> u32 {
    u32::try_from((i64::from(stated) + more).max(0)).unwrap_or(u32::MAX)
}

/// The extents `after` is to state: those of `base`, grown to hold each entity `after` places that `base` does not place the same way; none when the base states a number that is not finite.
fn extents_for(base: &CMapData, after: &CMapData, boxes: &Boxes) -> Option<Extents> {
    let (changes, _) = map_changes_paired(base, after);
    let stands = |change: &EntityChange| {
        matches!(
            change,
            EntityChange::Moved { .. }
                | EntityChange::Turned { .. }
                | EntityChange::Scaled { .. }
                | EntityChange::Archetype { .. }
                | EntityChange::LodDist { .. }
        )
    };
    let mut otherwise: BTreeSet<usize> = changes.added.iter().map(|e| e.index).collect();
    otherwise
        .extend((changes.ambiguous.iter()).flat_map(|group| group.now.iter().map(|e| e.index)));
    otherwise.extend(
        (changes.changed.iter())
            .filter(|kept| kept.changes.iter().any(stands))
            .map(|kept| kept.now.index),
    );
    let mut held = after.clone();
    let mut index = 0;
    held.entities.retain(|_| {
        index += 1;
        otherwise.contains(&(index - 1))
    });
    Extents::of(base).state(&mut held).ok()?;
    grow_extents(&mut held, |hash| boxes.get(&hash).copied());
    Some(Extents::of(&held))
}

/// Whether `after` holds the entities of `base` in its order, each stating what it states there, and states of itself what `base` states.
fn as_the_base(base: &CMapData, after: &CMapData) -> bool {
    let (changes, pairs) = map_changes_paired(base, after);
    changes.is_empty()
        && base.entities.len() == after.entities.len()
        && pairs.iter().enumerate().all(|(at, now)| *now == Some(at))
}

/// The file `after` saves as once the file of `base` holds its entities, the records it types beside them and what it states of itself: that file states every class the base holds a record of. None where the two differ in what no typed member names, which would be the base's, or where what is saved does not read back as `after`.
fn on_the_base(base: &CMapData, after: &CMapData) -> Option<Vec<u8>> {
    if map_changes_paired(base, after).0.other {
        return None;
    }
    let mut whole = base.clone();
    whole.name = after.name.clone();
    whole.parent = after.parent.clone();
    whole.flags = after.flags;
    whole.content_flags = after.content_flags;
    whole.streaming_extents_min = after.streaming_extents_min;
    whole.streaming_extents_max = after.streaming_extents_max;
    whole.entities_extents_min = after.entities_extents_min;
    whole.entities_extents_max = after.entities_extents_max;
    whole.entities = after.entities.clone();
    whole.physics_dictionaries = after.physics_dictionaries.clone();
    // ⚠ The parked vehicles, occluders, time cycle modifiers and far lights are the source's: putting an entity back changes none.
    whole.car_generators = after.car_generators.clone();
    whole.box_occluders = after.box_occluders.clone();
    whole.occlude_models = after.occlude_models.clone();
    whole.time_cycle_modifiers = after.time_cycle_modifiers.clone();
    whole.lod_lights_soa = after.lod_lights_soa.clone();
    whole.distant_lod_lights_soa = after.distant_lod_lights_soa.clone();
    let bytes = whole.to_bytes().ok()?;
    let read = CMapData::from_bytes(&bytes).ok()?;
    as_the_base(after, &read).then_some(bytes)
}

/// The map sources of a pack as a plan reads them.
struct Pack {
    files: Vec<MapFile>,
}

impl Pack {
    fn read(folder: &Path, model: &Maps) -> Result<Self, Error> {
        let mut files = Vec::new();
        let unread = |at: String, why: String| Error::Unread { at, why };
        let sources = delete::map_sources(folder, model).map_err(|refused| match refused {
            delete::Error::Source(at, why) => unread(at, why),
            other => Error::Sources(other),
        })?;
        for source in sources {
            let now = CMapData::from_bytes(&source.cooked)
                .map_err(|e| unread(source.path.clone(), e.to_string()))?;
            files.push(MapFile {
                source: source.path,
                name: map_name(&source.opened.manifest.name),
                opened: source.opened,
                inputs: source.inputs,
                now,
                compared: OnceCell::new(),
            });
        }
        Ok(Self { files })
    }

    fn at(&self, (file, index): Node) -> At {
        At {
            source: self.files[file].source.clone(),
            index,
        }
    }

    /// The file that holds the entities of the map called `name`, by the name of its file, which a link into that map indexes; none when no source is that map.
    // ⚠ By the name of its file and not the name it states: a file that states the name of another map is another map.
    fn holder(&self, name: u32) -> Result<Option<usize>, Error> {
        if name == 0 {
            return Ok(None);
        }
        let named: Vec<usize> = (0..self.files.len())
            .filter(|file| self.files[*file].name == name)
            .collect();
        let mut holding: Vec<usize> = (named.iter().copied())
            .filter(|file| !self.files[*file].now.entities.is_empty())
            .collect();
        if holding.is_empty() && named.len() > 1 {
            // A map file a deletion emptied holds its entities in its base.
            for &file in &named {
                if !self.files[file].compared()?.base.entities.is_empty() {
                    holding.push(file);
                }
            }
        }
        match (holding.as_slice(), named.as_slice()) {
            ([one], _) | ([], [one]) => Ok(Some(*one)),
            ([], _) => Ok(None),
            (several, _) => Err(Error::SplitMap(
                (several.iter())
                    .map(|file| self.files[*file].source.clone())
                    .collect(),
            )),
        }
    }

    fn state(&self, (file, index): Node) -> Result<Was, Error> {
        let compared = self.files[file].compared()?;
        Ok(match compared.now_of.get(index).copied().flatten() {
            Some(now) => Was::Is(now),
            None if compared.unsure_base.contains_key(&index) => Was::Unsure,
            None => Was::Deleted,
        })
    }

    fn above(&self, (file, index): Node) -> Result<Above, Error> {
        let compared = self.files[file].compared()?;
        let named = |at: &At| format!("{} entity {} of the base", at.source, at.index);
        let at = self.at((file, index));
        Ok(match link_of(&compared.base.entities[index]) {
            Link::None => Above::None,
            // An entity that names itself, or past its own file, has no parent to follow.
            Link::Own(parent) if parent == index || parent >= compared.base.entities.len() => {
                Above::None
            }
            Link::Own(parent) => Above::At((file, parent)),
            Link::ParentMap(stated) => {
                let Ok(parent) = usize::try_from(stated) else {
                    return Ok(Above::None);
                };
                let map = &compared.base.parent;
                match self.holder(map.hash)? {
                    _ if map.hash == 0 => Above::Beyond(Some(format!(
                        "{} names a far model in a parent map, and its map names none: the link is stated as the base states it",
                        named(&at)
                    ))),
                    // A map that is not among the sources is as the game has it, and so is an index into it.
                    None => Above::Beyond(None),
                    Some(holder) => {
                        let len = self.files[holder].compared()?.base.entities.len();
                        if parent < len {
                            Above::At((holder, parent))
                        } else {
                            Above::Beyond(Some(format!(
                                "{} names entity {parent} of its parent map, whose base holds {len}: the link is stated as the base states it",
                                named(&at)
                            )))
                        }
                    }
                }
            }
        })
    }

    /// The entities the bases place under the entity `node`: in its own file, and in each map file whose parent map its own is.
    fn below(&self, (file, index): Node) -> Result<Vec<Node>, Error> {
        let compared = self.files[file].compared()?;
        let mut found: Vec<Node> = (compared.base.entities.iter().enumerate())
            .filter(|(at, entity)| *at != index && link_of(entity) == Link::Own(index))
            .map(|(at, _)| (file, at))
            .collect();
        let name = self.files[file].name;
        if self.holder(name)? != Some(file) {
            return Ok(found);
        }
        let Ok(stated) = i32::try_from(index) else {
            return Ok(found);
        };
        for (other, held) in self.files.iter().enumerate() {
            if other == file || held.now.parent.hash != name {
                continue;
            }
            let under = held.compared()?;
            if under.base.parent.hash != name {
                continue;
            }
            found.extend(
                (under.base.entities.iter().enumerate())
                    .filter(|(_, entity)| link_of(entity) == Link::ParentMap(stated))
                    .map(|(at, _)| (other, at)),
            );
        }
        Ok(found)
    }

    /// How many entities name each entity as its LOD parent, by the file and the index of the one named, over the map `of` gives for each file.
    fn census<'a>(&'a self, of: impl Fn(usize) -> &'a CMapData) -> BTreeMap<Node, i64> {
        let mut named: BTreeMap<Node, i64> = BTreeMap::new();
        for file in 0..self.files.len() {
            let map = of(file);
            // ⚠ A link into a map two files each hold entities of names neither.
            let above =
                (self.holder(map.parent.hash).ok().flatten()).filter(|above| *above != file);
            for (index, entity) in map.entities.iter().enumerate() {
                let parent = match link_of(entity) {
                    Link::Own(parent) if parent != index => Some((file, parent)),
                    Link::ParentMap(parent) => above.zip(usize::try_from(parent).ok()),
                    _ => None,
                };
                if let Some(parent) = parent {
                    *named.entry(parent).or_default() += 1;
                }
            }
        }
        named
    }

    /// Whether the entity `now` of `file` names a parent among the twins of `parent`, an entity of the bases that nothing tells from them.
    fn stays_among(&self, file: usize, now: usize, parent: Node) -> Result<bool, Error> {
        let above = self.files[parent.0].compared()?;
        let named = match link_of(&self.files[file].now.entities[now]) {
            Link::Own(to) if parent.0 == file => Some(to),
            Link::ParentMap(to) if parent.0 != file => usize::try_from(to).ok(),
            _ => None,
        };
        let group = above.unsure_base.get(&parent.1);
        Ok(group.is_some() && named.and_then(|to| above.unsure_now.get(&to)) == group)
    }

    fn ambiguous(&self, file: usize, index: usize, in_base: bool) -> Result<Error, Error> {
        let compared = self.files[file].compared()?;
        let group = match in_base {
            true => compared.unsure_base.get(&index),
            false => compared.unsure_now.get(&index),
        };
        let held = group.and_then(|group| compared.changes.ambiguous.get(*group));
        let indexes = |of: &[EntityRef]| of.iter().map(|entity| entity.index).collect();
        Ok(Error::Ambiguous {
            at: self.files[file].source.clone(),
            index,
            of: if in_base { "the base of " } else { "" },
            base: held.map(|held| indexes(&held.base)).unwrap_or_default(),
            now: held.map(|held| indexes(&held.now)).unwrap_or_default(),
        })
    }

    /// The index the entity `node` of the bases has once the plan is done; none for one the map file lacks that does not come back.
    fn index_after(&self, draft: &Draft, (file, index): Node) -> Result<Option<usize>, Error> {
        let order = draft.orders.get(&file);
        if let Some(at) = order.and_then(|order| order.of_base.get(&index)) {
            return Ok(Some(*at));
        }
        Ok(match self.state((file, index))? {
            Was::Is(now) => Some(order.map_or(now, |order| order.of_now[now])),
            Was::Deleted | Was::Unsure => None,
        })
    }

    /// The record of the entity `node` as the base states it, with its LOD parent named by the index that one has once the plan is done.
    fn record(
        &self,
        draft: &Draft,
        node: Node,
        counts: Option<&Counts>,
    ) -> Result<CEntityDefAny, Error> {
        let mut record = self.files[node.0].compared()?.base.entities[node.1].clone();
        let kept = (draft.restored).filter(|(restored, _)| draft.keeps_parent && *restored == node);
        if let Some((_, now)) = kept {
            // ⚠ No entity comes back with one that keeps its parent, so the index it names is the index it will name.
            record.parent_index = self.files[node.0].now.entities[now].parent_index;
        } else if let Above::At(parent) = self.above(node)? {
            let refused = |why: String| Error::Refused {
                name: "put back",
                at: self.files[node.0].source.clone(),
                why,
            };
            let after = self.index_after(draft, parent)?.ok_or_else(|| {
                refused(format!(
                    "the far model the base names for entity {} is not in the map file and does not come back",
                    node.1
                ))
            })?;
            record.parent_index = link(after).map_err(refused)?;
        }
        if let Some(counted) = counts.and_then(|counts| counts.of_base.get(&node)) {
            record.num_children = *counted;
        }
        Ok(record)
    }

    /// The operations of `draft`, done to the map files as they are made: one that would change nothing is left out.
    fn build(&self, draft: &Draft, counts: Option<&Counts>) -> Result<Built, Error> {
        let mut built = Built {
            operations: Vec::new(),
            after: BTreeMap::new(),
        };
        let refused = |operation: &Operation, why: String| Error::Refused {
            name: operation.name(),
            at: operation.source().to_string(),
            why,
        };
        for (file, held) in self.files.iter().enumerate() {
            let source = || held.source.clone();
            let mut operations = Vec::new();

            // The links into the parent map, for the entities that come back into it.
            let above = (self.holder(held.now.parent.hash).ok().flatten())
                .filter(|above| *above != file)
                .and_then(|above| draft.orders.get(&above));
            let inserted: Vec<usize> = above
                .map(|order| order.of_base.values().copied().collect::<BTreeSet<_>>())
                .unwrap_or_default()
                .into_iter()
                .collect();
            let first = inserted.first().copied();
            let moves = |entity: &CEntityDefAny| {
                in_parent_map(entity.flags)
                    && usize::try_from(entity.parent_index)
                        .is_ok_and(|parent| first.is_some_and(|first| first <= parent))
            };
            if held.now.entities.iter().any(moves) {
                operations.push(Operation::RenumberForInserted {
                    source: source(),
                    expects: None,
                    inserted,
                });
            }

            let order = draft.orders.get(&file);
            let after_of = |now: usize| order.map_or(now, |order| order.of_now[now]);
            if let Some(order) = order {
                let compared = held.compared()?;
                let mut entities = Vec::new();
                for (&index, &at) in &order.of_base {
                    let record = self.record(draft, (file, index), counts)?;
                    // An entity the base places under this one that names no parent now names it again.
                    let children = (compared.base.entities.iter().enumerate())
                        .filter(|(child, entity)| {
                            *child != index && link_of(entity) == Link::Own(index)
                        })
                        .filter_map(|(child, _)| compared.now_of[child])
                        .filter(|now| link_of(&held.now.entities[*now]) == Link::None)
                        .collect();
                    entities.push(Inserted {
                        entity: at,
                        record: Record::entity(&record).map_err(|why| Error::Refused {
                            name: "insert_entities",
                            at: source(),
                            why,
                        })?,
                        children,
                    });
                }
                let num_children = counts
                    .into_iter()
                    .flat_map(|counts| counts.with_returns.iter())
                    .filter(|((of, _), _)| *of == file)
                    .map(|((_, now), counted)| Counted {
                        entity: after_of(*now),
                        num_children: *counted,
                    })
                    .collect();
                operations.push(Operation::InsertEntities {
                    source: source(),
                    expects: None,
                    entities,
                    num_children,
                    extents: None,
                });
            }

            if let Some((node, now)) = draft.restored.filter(|(node, _)| node.0 == file) {
                let record = self.record(draft, node, counts)?;
                operations.push(Operation::ReplaceEntity {
                    source: source(),
                    expects: None,
                    entity: after_of(now),
                    record: Record::entity(&record).map_err(|why| Error::Refused {
                        name: "replace_entity",
                        at: source(),
                        why,
                    })?,
                    extents: None,
                });
            }

            let counted = counts
                .into_iter()
                .flat_map(|counts| counts.in_the_end.iter())
                .filter(|((of, _), _)| *of == file);
            for ((_, now), num_children) in counted {
                operations.push(Operation::SetNumChildren {
                    source: source(),
                    expects: None,
                    entity: after_of(*now),
                    num_children: *num_children,
                });
            }

            for operation in operations {
                let map = (built.after.entry(file)).or_insert_with(|| held.now.clone());
                // ⚠ A record put in the place of the same record, and a count stated that is stated, are left out: a list
                // holds what changes a source, and a source it leaves as it was is none it writes.
                let nothing = match &operation {
                    Operation::ReplaceEntity { entity, record, .. } => (map.entities.get(*entity))
                        .and_then(|held| Record::entity(held).ok())
                        .is_some_and(|held| held == *record),
                    Operation::SetNumChildren {
                        entity,
                        num_children,
                        ..
                    } => (map.entities.get(*entity))
                        .is_some_and(|held| held.num_children == *num_children),
                    _ => false,
                };
                if nothing {
                    continue;
                }
                apply_to_map(map, &operation).map_err(|why| refused(&operation, why))?;
                built.operations.push((file, operation));
            }
        }
        built
            .after
            .retain(|file, _| built.operations.iter().any(|(of, _)| of == file));
        Ok(built)
    }

    fn plan(&self, folder: &Path, named: &Named) -> Result<PutBack, Error> {
        let (at, in_base) = match named {
            Named::Now(at) => (at, false),
            Named::Base(at) => (at, true),
        };
        let file = (self.files.iter().position(|held| held.source == at.source))
            .ok_or_else(|| Error::NoSuchSource(at.source.clone()))?;
        let compared = self.files[file].compared()?;
        let len = match in_base {
            true => compared.base.entities.len(),
            false => self.files[file].now.entities.len(),
        };
        let of = if in_base { "the base of " } else { "" };
        if at.index >= len {
            return Err(Error::NoSuchEntity {
                at: at.source.clone(),
                index: at.index,
                len,
                of,
            });
        }
        // The entity as the base holds it, and as the map file does when it holds it.
        let (base, now) = if in_base {
            match self.state((file, at.index))? {
                Was::Is(now) => (at.index, Some(now)),
                Was::Deleted => (at.index, None),
                Was::Unsure => return Err(self.ambiguous(file, at.index, true)?),
            }
        } else {
            match compared.base_of[at.index] {
                Some(base) => (base, Some(at.index)),
                None if compared.unsure_now.contains_key(&at.index) => {
                    return Err(self.ambiguous(file, at.index, false)?)
                }
                None => {
                    return Err(Error::Added {
                        at: at.source.clone(),
                        index: at.index,
                    })
                }
            }
        };
        let chosen: Node = (file, base);

        let mut returns: Vec<(Node, Why)> = Vec::new();
        let mut coming: BTreeSet<Node> = BTreeSet::new();
        let mut left: Vec<Node> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        if now.is_none() {
            coming.insert(chosen);
            returns.push((chosen, Why::Chosen));
            // Down the tree: what the base places under an entity that comes back, and the map file lacks, comes back.
            let mut next = 0;
            while let Some((node, _)) = returns.get(next).cloned() {
                next += 1;
                for child in self.below(node)? {
                    match self.state(child)? {
                        Was::Deleted if coming.insert(child) => {
                            let under = self.at(node);
                            returns.push((
                                child,
                                Why::Under {
                                    source: under.source,
                                    index: under.index,
                                },
                            ));
                        }
                        Was::Unsure if !left.contains(&child) => left.push(child),
                        _ => {}
                    }
                }
            }
        }
        // Up the chain: the far model the base names, where the map file lacks it, comes back.
        let mut keeps_parent = false;
        let mut under = chosen;
        loop {
            let parent = match self.above(under)? {
                Above::At(parent) => parent,
                Above::None => break,
                Above::Beyond(note) => {
                    notes.extend(note);
                    break;
                }
            };
            match self.state(parent)? {
                Was::Is(_) => break,
                Was::Deleted => {
                    if !coming.insert(parent) {
                        break;
                    }
                    returns.push((parent, Why::FarModel));
                    under = parent;
                }
                Was::Unsure => {
                    // ⚠ A parent among twins is the same parent while the link stays among them, as the comparison reads it.
                    let stays = match now.filter(|_| under == chosen) {
                        Some(now) => self.stays_among(file, now, parent)?,
                        None => false,
                    };
                    if !stays {
                        return Err(Error::FarModelAmbiguous {
                            at: self.files[under.0].source.clone(),
                            index: under.1,
                            above: self.files[parent.0].source.clone(),
                            parent: parent.1,
                        });
                    }
                    keeps_parent = true;
                    break;
                }
            }
        }

        let mut by_file: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for &(of, index) in &coming {
            by_file.entry(of).or_default().insert(index);
        }
        let mut draft = Draft {
            restored: now.map(|now| (chosen, now)),
            keeps_parent,
            returns,
            orders: BTreeMap::new(),
        };
        for (of, indexes) in &by_file {
            let held = &self.files[*of];
            // A link into a file that gains entities has to say which file it is.
            self.holder(held.name)?;
            let order = Order::of(held.compared()?, held.now.entities.len(), indexes);
            draft.orders.insert(*of, order);
        }

        // The links first, with the counts the base states: who names whom afterwards is what the counts are made of.
        let linked = self.build(&draft, None)?;
        let before = self.census(|of| &self.files[of].now);
        let after = self.census(|of| linked.after.get(&of).unwrap_or(&self.files[of].now));
        let named_after = |node: Node| -> Result<i64, Error> {
            let at = self.index_after(&draft, node)?;
            Ok(at
                .and_then(|at| after.get(&(node.0, at)))
                .copied()
                .unwrap_or(0))
        };
        let mut counts = Counts::default();
        let stated = draft.returns.iter().map(|(node, _)| *node);
        for node in stated.chain(draft.restored.map(|(node, _)| node)) {
            let counted = self.files[node.0].compared()?.base.entities[node.1].num_children;
            let more = named_after(node)? - self.below(node)?.len() as i64;
            counts.of_base.insert(node, recount(counted, more));
        }
        // The entities that come back under an entity the file keeps, in its own file: what the removal that takes them out again takes off its count.
        let mut returned_under: BTreeMap<Node, i64> = BTreeMap::new();
        for (node, _) in &draft.returns {
            if let Above::At(parent) = self.above(*node)? {
                if let (true, Was::Is(now)) = (parent.0 == node.0, self.state(parent)?) {
                    *returned_under.entry((parent.0, now)).or_default() += 1;
                }
            }
        }
        let mut recounted = Vec::new();
        for (of, held) in self.files.iter().enumerate() {
            let order = draft.orders.get(&of);
            for (now, entity) in held.now.entities.iter().enumerate() {
                if draft
                    .restored
                    .is_some_and(|(node, at)| node.0 == of && at == now)
                {
                    continue;
                }
                let at = order.map_or(now, |order| order.of_now[now]);
                let named = |census: &BTreeMap<Node, i64>, at: usize| {
                    census.get(&(of, at)).copied().unwrap_or(0)
                };
                let more = named(&after, at) - named(&before, now);
                let returned = returned_under.get(&(of, now)).copied().unwrap_or(0);
                if more == 0 && returned == 0 {
                    continue;
                }
                let with_returns = recount(entity.num_children, returned);
                let in_the_end = recount(entity.num_children, more);
                if returned != 0 {
                    counts.with_returns.insert((of, now), with_returns);
                }
                if in_the_end != with_returns {
                    counts.in_the_end.insert((of, now), in_the_end);
                }
                if in_the_end != entity.num_children {
                    recounted.push(Recounted {
                        source: held.source.clone(),
                        entity: at,
                        from: entity.num_children,
                        to: in_the_end,
                    });
                }
            }
        }
        let mut built = self.build(&draft, Some(&counts))?;

        // The extents of each file an entity was put into or back in.
        let placed: BTreeSet<usize> = (built.operations.iter())
            .filter(|(_, operation)| {
                matches!(
                    operation,
                    Operation::InsertEntities { .. } | Operation::ReplaceEntity { .. }
                )
            })
            .map(|(of, _)| *of)
            .collect();
        for &of in &placed {
            let held = &self.files[of];
            let (boxes, without) = boxes_of(folder, &held.source).map_err(|e| Error::Refused {
                name: "set_extents",
                at: held.source.clone(),
                why: e.to_string(),
            })?;
            notes.extend(without);
            let Some(map) = built.after.get_mut(&of) else {
                continue;
            };
            let extents = extents_for(&held.compared()?.base, map, &boxes)
                .and_then(|extents| extents.unless_stated_by(map));
            if let Some(extents) = extents {
                let operation = Operation::SetExtents {
                    source: held.source.clone(),
                    expects: None,
                    extents,
                };
                apply_to_map(map, &operation).map_err(|why| Error::Refused {
                    name: operation.name(),
                    at: held.source.clone(),
                    why,
                })?;
                built.operations.push((of, operation));
            }
        }

        // ⚠ A map file read again from its XML states the classes its records are of and no other, so a record that was the last of
        // its class does not go back into it. Such a file is written whole, as the XML of what the file of its base saves with the
        // entities the plan leaves: that file states every class the base holds a record of.
        for &of in &placed {
            let (held, Some(map)) = (&self.files[of], built.after.get(&of)) else {
                continue;
            };
            let written = (map.to_bytes().map_err(|e| e.to_string()))
                .and_then(|bytes| held.inputs.xml_of(&bytes).map_err(problems));
            let Err(why) = written else {
                continue;
            };
            let compared = held.compared()?;
            let unwritable = |why: String| Error::Unwritable {
                at: held.source.clone(),
                why,
            };
            let whole = match as_the_base(&compared.base, map) {
                true => compared.cooked.clone(),
                false => on_the_base(&compared.base, map).ok_or_else(|| unwritable(why))?,
            };
            let xml = (held.inputs.xml_of(&whole))
                .map_err(|found| unwritable(problems(found)))
                .and_then(|xml| {
                    String::from_utf8(xml)
                        .map_err(|_| unwritable("its XML would not be UTF-8".to_string()))
                })?;
            built.operations.retain(|(file, _)| *file != of);
            let whole = Operation::RewriteSource {
                source: held.source.clone(),
                expects: None,
                xml,
            };
            built.operations.push((of, whole));
        }

        if built.operations.is_empty() {
            return Err(Error::Unchanged {
                at: at.source.clone(),
                index: at.index,
            });
        }
        // The first operation of each source states the XML the plan was made of.
        let mut stated: BTreeSet<usize> = BTreeSet::new();
        let mut operations = Vec::with_capacity(built.operations.len());
        built.operations.sort_by_key(|(of, _)| *of);
        for (of, operation) in built.operations {
            operations.push(match stated.insert(of) {
                true => operation.expecting(digest_of(self.files[of].inputs.xml())),
                false => operation,
            });
        }

        let entity_of = |(of, index): Node| -> Result<EntityRef, Error> {
            let compared = self.files[of].compared()?;
            Ok(entity_ref(index, &compared.base.entities[index]))
        };
        let restored = match draft.restored {
            Some((node, now)) => {
                let kept = (compared.changes.changed.iter()).find(|kept| kept.now.index == now);
                Some(Restored {
                    source: at.source.clone(),
                    base: entity_of(node)?,
                    now: entity_ref(now, &self.files[file].now.entities[now]),
                    changes: kept.map(|kept| kept.changes.clone()).unwrap_or_default(),
                })
            }
            None => None,
        };
        let mut coming_back = Vec::with_capacity(draft.returns.len());
        for (node, why) in &draft.returns {
            coming_back.push(Returning {
                source: self.files[node.0].source.clone(),
                base: entity_of(*node)?,
                index: self.index_after(&draft, *node)?.unwrap_or(node.1),
                why: why.clone(),
            });
        }
        let mut left_out = Vec::with_capacity(left.len());
        for node in left {
            left_out.push(Left {
                source: self.files[node.0].source.clone(),
                base: entity_of(node)?,
            });
        }
        Ok(PutBack {
            restored,
            returns: coming_back,
            left: left_out,
            recounted,
            notes,
            sources: (stated.iter())
                .map(|of| self.files[*of].source.clone())
                .collect(),
            operations,
        })
    }
}

/// The plan that puts the entity `named` of the pack of maps in `folder` back as the base of its source has it; nothing is written.
pub fn plan(folder: &Path, named: &Named) -> Result<PutBack, Error> {
    let model = Maps::read(folder)?;
    Pack::read(folder, &model)?.plan(folder, named)
}

/// Do the plan `planned` to the sources of the pack of maps in `folder`, whole or not at all; the inverses of what it did, the last first, put every source back as it was edited.
pub fn apply(folder: &Path, planned: &PutBack) -> Result<Vec<Applied>, operation::Error> {
    operation::apply_all(folder, &planned.operations, false)
}

/// Do the plan `planned` in `session`, whole or not at all, with nothing written until the session is flushed; a session that holds operations it has not written refuses it, since the plan was made of the sources as the disk holds them.
pub fn apply_in(
    session: &mut operation::Session,
    planned: &PutBack,
) -> Result<Vec<operation::Done>, operation::Error> {
    session.apply_all(&planned.operations)
}
