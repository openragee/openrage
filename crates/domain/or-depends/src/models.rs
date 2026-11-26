//! The models of a mounted install as a list: each archetype its type files declare, once, with what a browser of models
//! filters and sorts by, and the questions such a browser asks, each answered a page at a time with its total.
//!
//! ⚠ No file is opened and no archive listed to build the list or to ask it: the world index keeps how often each map
//! places each archetype, how often the interiors of each type file hold it, and where each model file lies, so the
//! catalogue of an index read from its kept file costs what reading that file cost.
//!
//! ⚠ A type file states an archetype's hash and never its name. A model is named where a file the index lists has that
//! name as its stem (a model, a texture dictionary, a map, a type file or a bound), and is listed by its hash where none
//! has; [`ModelCatalogue::unnamed`] counts those.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use glam::Vec3;
use or_maps::place::Refused;
use or_world::ArchetypeKind;

use crate::catalogue::Kind;
use crate::find::{models_ranked, FileNamed, Rank, SearchNames};
use crate::world_index::{tree::is_a_box, Span, WorldIndex};

/// One model: an archetype as the last type file to declare it does.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub hash: u32,
    /// In lower case; `None` where no file the index lists has it as its stem.
    pub name: Option<String>,
    /// Whether its type file declares it as a base, a time or an interior archetype.
    pub kind: ArchetypeKind,
    /// `bbMin` and `bbMax`.
    pub span: Option<Span>,
    /// `bsCentre` and `bsRadius`, in metres; `None` where the type file states no box.
    pub sphere: Option<(Vec3, f32)>,
    /// The longest side of the box, in metres; `None` where the type file states none that is a box.
    pub size: Option<f32>,
    /// Metres.
    pub lod_dist: f32,
    /// `assetName`: the drawable or the fragment that draws it, or its entry in [`Self::drawable_dictionary`]; zero where it states none.
    pub asset_name: u32,
    /// Zero where it states none.
    pub drawable_dictionary: u32,
    /// Zero where it states none.
    pub texture_dictionary: u32,
    /// How often the maps the mount would load place it, their entities and their grass batches alike.
    pub placed: u32,
    /// How often the interiors of the type files the mount would load hold it, among their entities and their entity sets.
    pub inside: u32,
    /// Its place in the catalogue's type files.
    type_file: u32,
    /// The place in the catalogue's files of the one that holds its model.
    held: Option<u32>,
}

impl Model {
    /// How often something places it: the maps, and the interiors among their own entities.
    pub fn times_placed(&self) -> u64 {
        u64::from(self.placed) + u64::from(self.inside)
    }

    /// Whether a map places it or an interior holds it.
    pub fn is_placed(&self) -> bool {
        self.times_placed() > 0
    }
}

/// A type file that declares a model, and the pack it lies in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Declared<'a> {
    pub type_file: &'a FileNamed,
    /// The device of the pack: `platform`, `update`, or the one a pack's setup names.
    pub pack: &'a str,
}

/// The file of the mount that holds a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held<'a> {
    /// A drawable, a fragment, or the drawable dictionary the model is an entry of.
    pub kind: Kind,
    /// The root archive that lists the file, by its place in mount order, which is where a world source opens it from.
    pub archive: usize,
    pub logical: &'a str,
}

/// What a page of models is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// By name, the models no file names after the others and by their hash.
    Name,
    /// By the longest side of the box, the models that state no box last.
    Size,
    /// By how often something places the model.
    Placed,
}

/// One question a browser of models asks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Query<'q> {
    /// What a person typed, read as a search of the install reads it: a part of a name, a name in full, or a hash.
    pub text: Option<&'q str>,
    /// The device of a pack, in any case: the models whose kept declaration lies in it.
    pub pack: Option<&'q str>,
    pub kind: Option<ArchetypeKind>,
    /// The least and the most of [`Model::size`], in metres, each end included; a model with no size is in no range.
    pub size: (Option<f32>, Option<f32>),
    /// `Some(true)` for the models something places, `Some(false)` for those nothing does.
    pub placed: Option<bool>,
    /// `None` for the order a search gives, best first, where there is a text, and by name where there is none.
    pub sort: Option<Sort>,
    /// Last first; a model with no name, or with no size, stays after those that have one.
    pub descending: bool,
    /// How many models of the answer come before the page.
    pub offset: usize,
    /// The most models of the page.
    pub count: usize,
}

impl Default for Query<'_> {
    fn default() -> Self {
        Self {
            text: None,
            pack: None,
            kind: None,
            size: (None, None),
            placed: None,
            sort: None,
            descending: false,
            offset: 0,
            count: usize::MAX,
        }
    }
}

/// One model of a page, and how its name answers the text where there was one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listed<'a> {
    pub model: &'a Model,
    pub rank: Option<Rank>,
}

/// A page of the models that answer a question.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page<'a> {
    /// How many models answer, of which `rows` holds those of the page.
    pub total: usize,
    pub rows: Vec<Listed<'a>>,
}

/// A model file the catalogue names: the root archive that lists it and its logical path.
#[derive(Debug)]
struct FileAt {
    kind: Kind,
    archive: u32,
    logical: String,
}

/// Every model of an install, and the orders a browser lists them in.
#[derive(Debug)]
pub struct ModelCatalogue {
    /// By name, the models no file names after the others and by their hash.
    models: Vec<Model>,
    by_hash: HashMap<u32, u32>,
    /// Each type file the mount would load, and the place in `packs` of its pack.
    type_files: Vec<(FileNamed, u32)>,
    /// The device of each root archive, each once, in mount order.
    packs: Vec<String>,
    files: Vec<FileAt>,
    /// By a model's hash, the places in `type_files` of the declarations a later one replaced, in the mount's order.
    replaced: HashMap<u32, Vec<u32>>,
    /// The name of each dictionary a model states that a file the index lists has as its stem, in lower case.
    dictionaries: HashMap<u32, String>,
    search: SearchNames,
    /// The models in each order asked for so far, ascending and then descending of each [`Sort`].
    orders: [OnceLock<Vec<u32>>; 6],
}

/// The stem of a logical path as it is spelled, and its extension's kind.
fn stem_and_kind(logical: &str) -> (&str, Kind) {
    let leaf = logical.rsplit('/').next().unwrap_or(logical);
    match leaf.rsplit_once('.') {
        Some((stem, _)) => (stem, Kind::of_leaf(leaf)),
        None => (leaf, Kind::Other),
    }
}

/// Add each count of `counted` to its archetype's in `total`.
// ⚠ A kept file states these counts and anything may write to its folder: a sum stops at the most a count holds, where
// one that overflowed would stop the list.
fn add(total: &mut HashMap<u32, u32>, counted: &[(u32, u32)]) {
    for (archetype, count) in counted {
        let sum = total.entry(*archetype).or_default();
        *sum = sum.saturating_add(*count);
    }
}

impl ModelCatalogue {
    /// The models of `index`, which opens no file and lists no archive.
    pub fn new(index: &WorldIndex) -> Self {
        let mut packs: Vec<String> = Vec::new();
        let mut pack_at = |device: &str| -> u32 {
            let at = packs.iter().position(|pack| pack == device);
            let at = at.unwrap_or_else(|| {
                packs.push(device.to_string());
                packs.len() - 1
            });
            at as u32
        };
        for archive in index.archives() {
            pack_at(&archive.device);
        }
        let mut type_file_at: HashMap<u32, u32> = HashMap::new();
        let mut type_files = Vec::with_capacity(index.type_files().len());
        let mut inside: HashMap<u32, u32> = HashMap::new();
        for file in index.type_files() {
            type_file_at.insert(file.hash, type_files.len() as u32);
            let named = FileNamed {
                name: file.name.clone(),
                logical: file.logical.clone(),
            };
            type_files.push((named, pack_at(index.pack_of(file.archive, &file.logical))));
            add(&mut inside, &file.inside);
        }
        let mut placed: HashMap<u32, u32> = HashMap::new();
        for map in index.maps() {
            add(&mut placed, &map.placed);
        }

        // The names a model or its dictionaries could have, and the model files, the last copy of a leaf winning.
        let mut wanted: HashSet<u32> = HashSet::new();
        for archetype in index.archetypes() {
            wanted.extend([
                archetype.name,
                archetype.drawable_dictionary,
                archetype.texture_dictionary,
            ]);
        }
        wanted.remove(&0);
        let mut names: HashMap<u32, String> = HashMap::new();
        let mut name = |stem: &str| {
            let hash = or_io::string_hash(stem.as_bytes());
            if wanted.contains(&hash) {
                names
                    .entry(hash)
                    .or_insert_with(|| stem.to_ascii_lowercase());
            }
            hash
        };
        let mut model_files: HashMap<(u32, Kind), (usize, &str)> = HashMap::new();
        for (archive, logical) in index.files() {
            let (stem, kind) = stem_and_kind(logical);
            let hash = name(stem);
            if kind != Kind::TextureDict {
                model_files.insert((hash, kind), (archive, logical));
            }
        }
        let maps = index.maps().iter().map(|map| map.name.as_str());
        let types = index.type_files().iter().map(|file| file.name.as_str());
        let bounds = index.bounds().iter().map(|bound| bound.name.as_str());
        for stem in maps.chain(types).chain(bounds) {
            name(stem);
        }

        let mut files: Vec<FileAt> = Vec::new();
        let mut file_at: HashMap<(u32, Kind), u32> = HashMap::new();
        let mut models: Vec<Model> = Vec::with_capacity(index.archetype_count());
        // By hash, so two catalogues of one index hold their files in one order.
        let mut declared: Vec<_> = index.archetypes().collect();
        declared.sort_unstable_by_key(|archetype| archetype.name);
        for archetype in declared {
            // ⚠ A drawable dictionary the archetype states and the mount holds outranks a loose file of the model's
            // stem, as the resolver of dependencies has it: the asset is then an entry of the dictionary and no file's name.
            let dictionary = (archetype.drawable_dictionary, Kind::DrawableDict);
            let loose = [Kind::Fragment, Kind::Drawable].map(|kind| (archetype.asset_name, kind));
            let holders = [dictionary].into_iter().chain(loose);
            let holder = holders
                .filter(|(name, _)| *name != 0 && archetype.asset_name != 0)
                .find(|key| model_files.contains_key(key));
            let held = holder.map(|key| {
                *file_at.entry(key).or_insert_with(|| {
                    let (archive, logical) = model_files[&key];
                    files.push(FileAt {
                        kind: key.1,
                        archive: archive as u32,
                        logical: logical.to_string(),
                    });
                    files.len() as u32 - 1
                })
            });
            let size = (archetype.span)
                .filter(|(min, max)| is_a_box(*min, *max))
                .map(|(min, max)| (max - min).max_element());
            models.push(Model {
                hash: archetype.name,
                name: names.get(&archetype.name).cloned(),
                kind: archetype.kind,
                span: archetype.span,
                sphere: archetype.span.map(|_| archetype.sphere),
                size,
                lod_dist: archetype.lod_dist,
                asset_name: archetype.asset_name,
                drawable_dictionary: archetype.drawable_dictionary,
                texture_dictionary: archetype.texture_dictionary,
                placed: placed.get(&archetype.name).copied().unwrap_or(0),
                inside: inside.get(&archetype.name).copied().unwrap_or(0),
                type_file: type_file_at.get(&archetype.type_file).copied().unwrap_or(0),
                held,
            });
        }
        models.sort_unstable_by(|a, b| {
            let unnamed = a.name.is_none().cmp(&b.name.is_none());
            unnamed
                .then_with(|| a.name.cmp(&b.name))
                .then(a.hash.cmp(&b.hash))
        });
        let by_hash = (models.iter().enumerate())
            .map(|(at, model)| (model.hash, at as u32))
            .collect();
        let replaced = (models.iter())
            .filter_map(|model| {
                let earlier = index.redeclared(model.hash);
                let earlier: Vec<u32> = (earlier.iter())
                    .filter_map(|file| type_file_at.get(file).copied())
                    .collect();
                (!earlier.is_empty()).then_some((model.hash, earlier))
            })
            .collect();
        let search = SearchNames::named_by(index, |hash| names.get(&hash).cloned());
        let dictionaries = (models.iter())
            .flat_map(|model| [model.drawable_dictionary, model.texture_dictionary])
            .filter_map(|hash| Some((hash, names.get(&hash)?.clone())))
            .collect();
        Self {
            models,
            by_hash,
            type_files,
            packs,
            files,
            replaced,
            dictionaries,
            search,
            orders: Default::default(),
        }
    }

    /// How many models the install declares, each name once.
    pub fn len(&self) -> usize {
        self.models.len()
    }

    /// Whether the install declares no model.
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Every model, by name, the models no file names after the others and by their hash.
    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// The model whose name hashes to `hash`.
    pub fn model(&self, hash: u32) -> Option<&Model> {
        self.by_hash.get(&hash).map(|at| &self.models[*at as usize])
    }

    /// How many models no file the index lists names, which are listed by their hash.
    pub fn unnamed(&self) -> usize {
        self.search.unnamed()
    }

    /// The type file whose declaration of `model` the mount keeps, and its pack.
    pub fn declared(&self, model: &Model) -> Declared<'_> {
        self.declaration(model.type_file)
    }

    /// The declarations of `model` that a later type file's replaced, in the mount's order.
    pub fn replaced(&self, model: &Model) -> Vec<Declared<'_>> {
        let earlier = self.replaced.get(&model.hash);
        let earlier = earlier.into_iter().flatten();
        earlier.map(|file| self.declaration(*file)).collect()
    }

    fn declaration(&self, type_file: u32) -> Declared<'_> {
        let (file, pack) = &self.type_files[type_file as usize];
        Declared {
            type_file: file,
            pack: &self.packs[*pack as usize],
        }
    }

    /// The file of the mount that holds the model of `model`; `None` where it names no asset or the mount holds no file of it.
    pub fn held(&self, model: &Model) -> Option<Held<'_>> {
        let file = &self.files[model.held? as usize];
        Some(Held {
            kind: file.kind,
            archive: file.archive as usize,
            logical: &file.logical,
        })
    }

    /// The name of the dictionary `hash` of a model, in lower case, where a file the index lists has it as its stem.
    pub fn dictionary_name(&self, hash: u32) -> Option<&str> {
        self.dictionaries.get(&hash).map(String::as_str)
    }

    /// Each pack of the mount, in mount order, with how many models' kept declaration lies in it.
    pub fn packs(&self) -> Vec<(&str, usize)> {
        let mut counts = vec![0usize; self.packs.len()];
        for model in &self.models {
            counts[self.type_files[model.type_file as usize].1 as usize] += 1;
        }
        let packs = self.packs.iter().map(String::as_str);
        packs.zip(counts).collect()
    }

    /// How many models are of each kind: base, time and interior.
    pub fn kinds(&self) -> [(ArchetypeKind, usize); 3] {
        let count = |kind| (kind, self.models.iter().filter(|m| m.kind == kind).count());
        [
            count(ArchetypeKind::Base),
            count(ArchetypeKind::Time),
            count(ArchetypeKind::Mlo),
        ]
    }

    /// How two models stand in an order: a model with no name or no size after those that have one whichever way, and models that tie by name.
    fn compare(&self, sort: Sort, descending: bool, a: u32, b: u32) -> Ordering {
        let (first, second) = (&self.models[a as usize], &self.models[b as usize]);
        let way = |order: Ordering| if descending { order.reverse() } else { order };
        let ordered = match sort {
            // The models are held by name, so a place is a name's rank; those no file names stay after the others.
            Sort::Name => (first.name.is_none().cmp(&second.name.is_none())).then(way(a.cmp(&b))),
            Sort::Size => match (first.size, second.size) {
                (Some(one), Some(other)) => way(one.total_cmp(&other)),
                (one, other) => other.is_some().cmp(&one.is_some()),
            },
            Sort::Placed => way(first.times_placed().cmp(&second.times_placed())),
        };
        ordered.then(a.cmp(&b))
    }

    /// Every model's place in `models`, in the order of `sort`, sorted the first time it is asked for.
    fn order(&self, sort: Sort, descending: bool) -> &[u32] {
        let at = match sort {
            Sort::Name => 0,
            Sort::Size => 2,
            Sort::Placed => 4,
        };
        self.orders[at + usize::from(descending)].get_or_init(|| {
            let mut order: Vec<u32> = (0..self.models.len() as u32).collect();
            order.sort_unstable_by(|a, b| self.compare(sort, descending, *a, *b));
            order
        })
    }

    /// The page of the models that answer `query`, with how many answer; an error where the text has the shape of a place or a hash and is none.
    pub fn page(&self, query: &Query<'_>) -> Result<Page<'_>, Refused> {
        let pack = match query.pack {
            None => None,
            Some(name) => {
                let at = (self.packs.iter()).position(|pack| pack.eq_ignore_ascii_case(name));
                match at {
                    Some(at) => Some(at as u32),
                    None => return Ok(Page::default()),
                }
            }
        };
        let answers = |model: &Model| {
            let (least, most) = query.size;
            pack.is_none_or(|pack| self.type_files[model.type_file as usize].1 == pack)
                && query.kind.is_none_or(|kind| model.kind == kind)
                && (least.is_none() && most.is_none()
                    || model.size.is_some_and(|size| {
                        least.is_none_or(|least| size >= least)
                            && most.is_none_or(|most| size <= most)
                    }))
                && query
                    .placed
                    .is_none_or(|placed| model.is_placed() == placed)
        };
        let mut page = Page::default();
        let mut take = |at: u32, rank: Option<Rank>| {
            let model = &self.models[at as usize];
            if answers(model) {
                if page.total >= query.offset && page.rows.len() < query.count {
                    page.rows.push(Listed { model, rank });
                }
                page.total += 1;
            }
        };
        match query.text.map(str::trim).filter(|text| !text.is_empty()) {
            None => {
                let sort = query.sort.unwrap_or(Sort::Name);
                for at in self.order(sort, query.descending) {
                    take(*at, None);
                }
            }
            Some(text) => {
                let declared = |hash: u32| self.by_hash.contains_key(&hash);
                let ranked = models_ranked(text, declared, &self.search, usize::MAX)?;
                let mut ranked: Vec<(u32, Rank)> = (ranked.into_iter())
                    .filter_map(|(hash, rank)| Some((*self.by_hash.get(&hash)?, rank)))
                    .collect();
                match query.sort {
                    Some(sort) => ranked
                        .sort_unstable_by(|a, b| self.compare(sort, query.descending, a.0, b.0)),
                    // Best first is the order a search gives, so last first is its reverse.
                    None if query.descending => ranked.reverse(),
                    None => {}
                }
                for (at, rank) in ranked {
                    take(at, Some(rank));
                }
            }
        }
        Ok(page)
    }
}
