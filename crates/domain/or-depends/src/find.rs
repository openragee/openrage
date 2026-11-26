//! Asking an install where something is: what a person typed, read as a place, a hash or a name, and answered from the indexes of
//! a mounted install as rows in groups, best first.
//!
//! ⚠ A question opens no file but the bounds a height is asked of, and visits no placement but those of the models it lists.
//! Names are searched through [`SearchNames`], built once: the names in order, and each part of a name after an underscore in
//! order, so a whole name and a prefix are one binary search, a prefix of a part is another, and only "contained" passes over
//! the names, stopping at the count asked for.
//!
//! ⚠ A type file states an archetype's hash and never its name, so a model is found by a part of its name only where a file of
//! the install states that name ([`or_io::NameIndex`]). Typed in full, a name is hashed and found all the same.

mod placed;

use std::collections::HashSet;

use glam::Vec3;
use or_io::names::HashFlavour;
use or_io::NameIndex;
use or_maps::place::{read_pasted, spell_hash, HashSpellings, Pasted, PastedPlace, Refused};
use or_maps::script_text::{self, ScriptForm};
use or_world::collision::GroundProbe;

use crate::catalogue::{Catalogue, Kind};
use crate::placement::{Placed, PlacementIndex};
use crate::world_collision::WorldCollision;
use crate::world_index::{Map, Span, WorldIndex, WorldSource};

pub use placed::{
    place_maps, placements_of, Budget, Detail, ModelPlacements, PlacementFound, Unread,
};

/// How far above a place given with its height the ground under it may be, in metres.
pub const RISE: f32 = 1.0;

/// The steepest surface taken for the ground under a place, in degrees from level.
pub const SLOPE: f32 = 60.0;

/// How a name answers the text, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    /// The name is the text, or the hash typed is the name's.
    Exact,
    /// The name begins with the text.
    Prefix,
    /// A part of the name after an underscore begins with the text.
    PartPrefix,
    /// The name holds the text elsewhere.
    Contained,
}

/// What a question cost, which is what holds it to the size of its answer and not of the install.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Work {
    /// Names compared with the text.
    pub names: usize,
    /// Placements visited, to find the nearest of a model listed.
    pub placements: usize,
}

/// One list of names in the two orders a search reads it in.
#[derive(Debug, Default)]
struct NameOrder {
    /// Each name once, in lower case, with the hash it is known by, by name.
    names: Vec<(String, u32)>,
    /// Each part of a name after an underscore, as the name's place in `names` and the byte the part begins at, by the part.
    parts: Vec<(u32, u32)>,
}

impl NameOrder {
    fn new(mut names: Vec<(String, u32)>) -> Self {
        names.sort();
        names.dedup_by(|a, b| a.0 == b.0);
        let mut parts: Vec<(u32, u32)> = Vec::new();
        for (at, (name, _)) in names.iter().enumerate() {
            let begins = name.match_indices('_').map(|(byte, _)| byte + 1);
            parts.extend(
                begins
                    .filter(|begin| *begin < name.len())
                    .map(|begin| (at as u32, begin as u32)),
            );
        }
        let part = |(at, begin): (u32, u32)| &names[at as usize].0[begin as usize..];
        parts.sort_by(|a, b| part(*a).cmp(part(*b)).then(a.cmp(b)));
        Self { names, parts }
    }

    fn part(&self, (at, begin): (u32, u32)) -> &str {
        &self.names[at as usize].0[begin as usize..]
    }

    /// The names `text` answers, at most `most`, each by its place in `names`: the name itself, those that begin with it by name, those a part of which does by that part, then those that hold it by name.
    fn matching(&self, text: &str, most: usize, work: &mut Work) -> Vec<(usize, Rank)> {
        let mut found: Vec<(usize, Rank)> = Vec::new();
        if text.is_empty() {
            return found;
        }
        let mut listed: HashSet<usize> = HashSet::new();
        // The names that begin with `text` stand together, the name that is `text` first of them.
        let from = self.names.partition_point(|(name, _)| {
            work.names += 1;
            name.as_str() < text
        });
        for (at, (name, _)) in self.names.iter().enumerate().skip(from) {
            if found.len() >= most {
                return found;
            }
            work.names += 1;
            if !name.starts_with(text) {
                break;
            }
            let rank = if name == text {
                Rank::Exact
            } else {
                Rank::Prefix
            };
            listed.insert(at);
            found.push((at, rank));
        }
        let from = self.parts.partition_point(|part| {
            work.names += 1;
            self.part(*part) < text
        });
        for part in &self.parts[from..] {
            if found.len() >= most {
                return found;
            }
            work.names += 1;
            if !self.part(*part).starts_with(text) {
                break;
            }
            if listed.insert(part.0 as usize) {
                found.push((part.0 as usize, Rank::PartPrefix));
            }
        }
        for (at, (name, _)) in self.names.iter().enumerate() {
            if found.len() >= most {
                break;
            }
            work.names += 1;
            if name.contains(text) && listed.insert(at) {
                found.push((at, Rank::Contained));
            }
        }
        found
    }
}

/// The names of an install's models and maps, in the orders a search reads them in.
#[derive(Debug, Default)]
pub struct SearchNames {
    models: NameOrder,
    maps: NameOrder,
    unnamed: usize,
}

impl SearchNames {
    /// The archetypes of `world` that `names` states a name for, and its maps by their own.
    pub fn new(world: &WorldIndex, names: &NameIndex) -> Self {
        let mut unnamed = 0;
        let models = world.archetypes().filter_map(|archetype| {
            let name = name_of(names, archetype.name);
            unnamed += usize::from(name.is_none());
            Some((name?, archetype.name))
        });
        let models = NameOrder::new(models.collect());
        let maps = world.maps().iter().map(|map| (map.name.clone(), map.hash));
        Self {
            models,
            maps: NameOrder::new(maps.collect()),
            unnamed,
        }
    }

    /// How many archetypes a part of a name finds.
    pub fn models(&self) -> usize {
        self.models.names.len()
    }

    /// How many archetypes no file of the install names, which their hash and their whole name alone find.
    pub fn unnamed(&self) -> usize {
        self.unnamed
    }
}

/// The indexes of one mounted install that a question is answered from.
#[derive(Clone, Copy)]
pub struct Install<'a> {
    pub world: &'a WorldIndex,
    pub catalogue: &'a Catalogue,
    pub names: &'a NameIndex,
    pub search: &'a SearchNames,
    /// ⚠ A model's count is what this holds of it: an index that watched other models says none.
    pub placements: &'a PlacementIndex,
}

/// From where a question is asked, and how much of an answer is wanted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Asked {
    /// The view, in metres, which a model's placements are ordered from.
    pub near: Option<Vec3>,
    /// The most models, the most maps, and the most maps and bounds named under a place.
    pub limit: usize,
}

/// A file of the mount, by its stem and where the mount lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileNamed {
    pub name: String,
    pub logical: String,
}

/// The ground under a place.
#[derive(Debug, Clone, PartialEq)]
pub enum GroundFound {
    /// The question was handed no collision to ask.
    Unasked,
    /// No bounds file of the column holds ground there.
    Nothing,
    At {
        /// Metres.
        height: f32,
        /// The bounds file whose surface it is.
        bound: Option<String>,
    },
}

/// The text read as a place, and what of the install stands there.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceFound {
    pub x: f32,
    pub y: f32,
    /// As it was typed; `None` where two numbers were.
    pub z: Option<f32>,
    /// Degrees, as typed.
    pub heading: Option<f32>,
    /// The maps whose streaming extents hold it, or its column where no height was typed, in the mount's order.
    pub maps: Vec<FileNamed>,
    /// How many maps do, of which `maps` names the first.
    pub maps_held: usize,
    /// The bounds something carries whose box holds it, or its column.
    pub bounds: Vec<FileNamed>,
    pub bounds_held: usize,
    pub ground: GroundFound,
}

impl PlaceFound {
    /// Where to go: the height typed, else the ground's; `None` where neither is known.
    pub fn position(&self) -> Option<Vec3> {
        let ground = match self.ground {
            GroundFound::At { height, .. } => Some(height),
            _ => None,
        };
        Some(Vec3::new(self.x, self.y, self.z.or(ground)?))
    }

    /// The place as `form` writes one, with its heading where one was typed; `None` where it has no height.
    pub fn script(
        &self,
        form: &ScriptForm,
        decimals: usize,
    ) -> Result<Option<String>, script_text::Error> {
        self.position()
            .map(|at| form.place(at, self.heading, decimals))
            .transpose()
    }
}

/// What a hash names in the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// An archetype, with the type file that declares it.
    Archetype {
        type_file: Option<FileNamed>,
    },
    Map {
        logical: String,
    },
    /// A file whose stem it is, or whose whole name.
    File {
        kind: Kind,
        logical: String,
    },
}

/// The text read as a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashFound {
    pub hash: u32,
    /// Its three spellings, and its name where a file of the install states one.
    pub spellings: HashSpellings,
    pub names: Vec<Named>,
}

/// The nearest placement of a model to the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nearest {
    pub position: Vec3,
    /// The `CMapData::name` of the map that places it.
    pub map: u32,
    /// Metres.
    pub distance: f32,
}

/// One model the text answers.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelFound {
    /// `None` for a hash no file of the install names.
    pub name: Option<String>,
    pub hash: u32,
    pub rank: Rank,
    /// The type file that declares it.
    pub type_file: Option<FileNamed>,
    /// How many placements the placement index holds of it; none means the model is opened, not gone to.
    pub placements: usize,
    /// With a view, the entity nearest it.
    pub nearest: Option<Nearest>,
}

/// One map the text answers.
#[derive(Debug, Clone, PartialEq)]
pub struct MapFound {
    pub name: String,
    pub hash: u32,
    pub rank: Rank,
    pub logical: String,
    /// The box its entities stand in, else the one it streams from; `None` where the file states neither as a box.
    pub extents: Option<Span>,
    /// The middle of `extents`.
    pub centre: Option<Vec3>,
}

/// The answer to a question, in groups: at most one place, at most one hash, models and maps.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub place: Option<PlaceFound>,
    pub hash: Option<HashFound>,
    pub models: Vec<ModelFound>,
    /// Whether more models answer than the count asked for.
    pub more_models: bool,
    pub maps: Vec<MapFound>,
    pub more_maps: bool,
    pub work: Work,
}

fn file_named(name: &str, logical: &str) -> FileNamed {
    FileNamed {
        name: name.to_string(),
        logical: logical.to_string(),
    }
}

/// The name a file of the install states for `hash`, in lower case, which is the case the hash of a name folds it to.
fn name_of(names: &NameIndex, hash: u32) -> Option<String> {
    let stated = names.resolve_with(HashFlavour::CaseFolding, hash);
    stated.map(str::to_ascii_lowercase)
}

/// The models `text`, in lower case, answers: at most `most`, each with its name, its hash and its rank.
fn models_named(
    text: &str,
    world: &WorldIndex,
    search: &SearchNames,
    most: usize,
    work: &mut Work,
) -> Vec<(String, u32, Rank)> {
    let matched = search.models.matching(text, most, work);
    let mut rows = Vec::with_capacity(matched.len() + 1);
    // ⚠ A name typed in full is an archetype by its hash, though no file of the install states the name.
    let typed = or_io::string_hash(text.as_bytes());
    let listed = matches!(matched.first(), Some((_, Rank::Exact)));
    if !text.is_empty() && !listed && world.archetype(typed).is_some() {
        rows.push((text.to_string(), typed, Rank::Exact));
    }
    rows.extend(matched.into_iter().map(|(at, rank)| {
        let (name, hash) = &search.models.names[at];
        (name.clone(), *hash, rank)
    }));
    rows.truncate(most);
    rows
}

/// The archetypes a question of `text` lists as models: what a caller with no placement index watches as it walks the maps.
pub fn models_asked(
    text: &str,
    world: &WorldIndex,
    search: &SearchNames,
    limit: usize,
) -> Result<Vec<u32>, Refused> {
    Ok(match read_pasted(text)? {
        Pasted::Place(_) => Vec::new(),
        Pasted::Hash(hash) => (world.archetype(hash).map(|archetype| archetype.name))
            .into_iter()
            .take(limit)
            .collect(),
        Pasted::Text(text) => {
            let text = text.to_ascii_lowercase();
            let rows = models_named(&text, world, search, limit, &mut Work::default());
            rows.into_iter().map(|(_, hash, _)| hash).collect()
        }
    })
}

fn model_found(
    name: Option<String>,
    hash: u32,
    rank: Rank,
    install: &Install<'_>,
    asked: &Asked,
    work: &mut Work,
) -> ModelFound {
    let world = install.world;
    let declared = world.archetype(hash);
    let type_file = declared
        .and_then(|archetype| world.type_file(archetype.type_file))
        .map(|file| file_named(&file.name, &file.logical));
    let placed = install.placements.of(hash);
    let nearest = asked.near.and_then(|near| {
        work.placements += placed.len();
        // ⚠ A grass batch is held with no position of its own, so it is nearest to nothing.
        let entities = placed.iter().filter(|p| p.placed != Placed::GrassBatch);
        let nearest = entities
            .map(|placement| (placement, placement.position.distance(near)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        nearest.map(|(placement, distance)| Nearest {
            position: placement.position,
            map: placement.map,
            distance,
        })
    });
    ModelFound {
        name,
        hash,
        rank,
        type_file,
        placements: placed.len(),
        nearest,
    }
}

fn map_found(map: &Map, rank: Rank) -> MapFound {
    let is_box = |(min, max): &Span| min.is_finite() && max.is_finite() && min.cmple(*max).all();
    let extents = [map.entities, map.streaming]
        .into_iter()
        .flatten()
        .find(is_box);
    MapFound {
        name: map.name.clone(),
        hash: map.hash,
        rank,
        logical: map.logical.clone(),
        extents,
        centre: extents.map(|(min, max)| (min + max) * 0.5),
    }
}

fn place_found(
    place: &PastedPlace,
    world: &WorldIndex,
    limit: usize,
    collision: Option<&mut WorldCollision<'_>>,
) -> PlaceFound {
    let (x, y) = (place.x, place.y);
    let (maps, bounds) = match place.z {
        Some(z) => {
            let at = Vec3::new(x, y, z);
            (world.maps_at(at), world.bounds_at(at))
        }
        None => (world.maps_in_column(x, y), world.bounds_in_column(x, y)),
    };
    let ground = match collision {
        None => GroundFound::Unasked,
        Some(collision) => {
            let max_slope = SLOPE.to_radians();
            let answer = match place.z {
                Some(z) => collision.ground(
                    Vec3::new(x, y, z),
                    &GroundProbe {
                        max_rise: RISE,
                        max_drop: f32::INFINITY,
                        max_slope,
                    },
                ),
                None => collision.highest_ground(x, y, max_slope),
            };
            match answer.ground {
                Some(ground) => GroundFound::At {
                    height: ground.hit.position.z,
                    bound: ground.bound.map(|bound| bound.name.clone()),
                },
                None => GroundFound::Nothing,
            }
        }
    };
    PlaceFound {
        x,
        y,
        z: place.z,
        heading: place.heading,
        maps_held: maps.len(),
        maps: (maps.iter().take(limit))
            .map(|map| file_named(&map.name, &map.logical))
            .collect(),
        bounds_held: bounds.len(),
        bounds: (bounds.iter().take(limit))
            .map(|found| file_named(&found.bound.name, &found.bound.logical))
            .collect(),
        ground,
    }
}

fn hash_found(hash: u32, install: &Install<'_>) -> HashFound {
    let world = install.world;
    let mut names = Vec::new();
    if let Some(archetype) = world.archetype(hash) {
        let type_file = world
            .type_file(archetype.type_file)
            .map(|file| file_named(&file.name, &file.logical));
        names.push(Named::Archetype { type_file });
    }
    let map = world.map(hash);
    if let Some(map) = map {
        names.push(Named::Map {
            logical: map.logical.clone(),
        });
    }
    // The map is said once, by the index that also knows where it stands.
    let kinds = Kind::ALL
        .iter()
        .filter(|kind| !(map.is_some() && **kind == Kind::MapData));
    names.extend(kinds.filter_map(|kind| {
        let file = install.catalogue.find(hash, *kind)?;
        Some(Named::File {
            kind: *kind,
            logical: file.logical.clone(),
        })
    }));
    let spellings = spell_hash(hash, |hash| name_of(install.names, hash));
    // ⚠ An archive lists a file by the hash of its whole name, where everything above is named by a stem: the name the
    // install states for such a hash says which file it is.
    let whole = (spellings.name.as_deref()).and_then(|leaf| Some((leaf, leaf.rsplit_once('.')?.0)));
    if let Some((leaf, stem)) = whole {
        let file = install.catalogue.find_named(stem, Kind::of_leaf(leaf));
        let file = file.filter(|file| file.stem == stem);
        names.extend(file.map(|file| Named::File {
            kind: file.kind,
            logical: file.logical.clone(),
        }));
    }
    HashFound {
        hash,
        spellings,
        names,
    }
}

/// Answer `text` from `install`: a place with what stands there and, with `collision`, its ground; a hash with what it names; or the models and the maps whose name it answers, best first.
pub fn find(
    text: &str,
    install: &Install<'_>,
    asked: &Asked,
    collision: Option<&mut WorldCollision<'_>>,
) -> Result<Found, Refused> {
    let mut found = Found::default();
    let world = install.world;
    match read_pasted(text)? {
        Pasted::Place(place) => {
            found.place = Some(place_found(&place, world, asked.limit, collision));
        }
        Pasted::Hash(hash) => {
            found.hash = Some(hash_found(hash, install));
            if asked.limit > 0 && world.archetype(hash).is_some() {
                let name = name_of(install.names, hash);
                let model = model_found(name, hash, Rank::Exact, install, asked, &mut found.work);
                found.models.push(model);
            }
            if let Some(map) = world.map(hash).filter(|_| asked.limit > 0) {
                found.maps.push(map_found(map, Rank::Exact));
            }
        }
        Pasted::Text(text) => {
            let text = text.to_ascii_lowercase();
            // One past the count asked for, which says whether more answer.
            let most = asked.limit.saturating_add(1);
            let search = install.search;
            let mut models = models_named(&text, world, search, most, &mut found.work);
            found.more_models = models.len() > asked.limit;
            models.truncate(asked.limit);
            for (name, hash, rank) in models {
                let model = model_found(Some(name), hash, rank, install, asked, &mut found.work);
                found.models.push(model);
            }
            let mut maps = search.maps.matching(&text, most, &mut found.work);
            found.more_maps = maps.len() > asked.limit;
            maps.truncate(asked.limit);
            found.maps.extend(maps.into_iter().filter_map(|(at, rank)| {
                Some(map_found(world.map(search.maps.names[at].1)?, rank))
            }));
        }
    }
    Ok(found)
}

/// The files of `source` by name, and every name its entries state: each leaf, and its stem.
pub fn names_of(source: &dyn WorldSource) -> (Catalogue, NameIndex) {
    let mut catalogue = Catalogue::new();
    let mut names = NameIndex::new("gta5");
    for archive in 0..source.archives().len() {
        source.entries(archive, &mut |facts| {
            catalogue.index(facts);
            names.insert_with_stem(&facts.leaf);
        });
    }
    (catalogue, names)
}
