//! What the install suite of the doors, bounds and path regions counts and compares: the body it runs over a mount, and a default
//! suite runs over files built by hand.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use or_io::decl::File;
use or_io::gta5::bounds::PhBoundAny;
use or_io::gta5::CPathRegion;
use or_io::meta::MetaInstance;
use or_source::asset::Source;
use or_source::textures::Extraction;
use or_world_gta5::doors::{find, Found, FoundBy, NamePatterns};
use or_world_gta5::{ExtensionKind, MapData, MapTypes};

/// What the searches of doors over a set of maps came to.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DoorCount {
    /// The entities looked at, those an interior places among them.
    pub entities: usize,
    /// The doors the door extension found: the entity states it, or its archetype does.
    pub by_extension: usize,
    /// The doors a name found, by the pattern that found each.
    pub by_name: BTreeMap<String, usize>,
    /// The entities no search could judge: no door extension, and a model with no name.
    pub unnamed_entities: usize,
    /// The models of those entities, and the models of every entity that is no placed interior.
    pub unnamed_models: BTreeSet<u32>,
    pub models: BTreeSet<u32>,
    /// The entities of the maps, no placed interior among them, that state the door extension themselves.
    pub stated_by_the_entity: usize,
}

impl DoorCount {
    fn add(&mut self, found: &Found) {
        self.entities += found.entities;
        self.unnamed_entities += found.unnamed;
        for door in &found.doors {
            self.models.insert(door.entity.archetype);
            match &door.found_by {
                FoundBy::Extension => self.by_extension += 1,
                FoundBy::Name(pattern) => *self.by_name.entry(pattern.clone()).or_default() += 1,
            }
        }
        for other in found.others.iter().filter(|e| !e.interior_instance) {
            self.models.insert(other.archetype);
            if other.name.is_none() {
                self.unnamed_models.insert(other.archetype);
            }
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.entities += other.entities;
        self.by_extension += other.by_extension;
        self.unnamed_entities += other.unnamed_entities;
        for (pattern, doors) in other.by_name {
            *self.by_name.entry(pattern).or_default() += doors;
        }
        self.unnamed_models.extend(other.unnamed_models);
        self.models.extend(other.models);
        self.stated_by_the_entity += other.stated_by_the_entity;
    }
}

/// Whether one of `extensions` is the door extension.
fn states_a_door<'a>(mut extensions: impl Iterator<Item = or_world_gta5::Extension<'a>>) -> bool {
    extensions.any(|extension| extension.kind() == ExtensionKind::Door)
}

/// How many archetypes of `types` state the door extension, and how many entities of their interiors do.
pub fn doors_stated_in(types: &[MetaInstance]) -> (usize, usize) {
    let archetypes = || {
        let files = types.iter().filter_map(MapTypes::new);
        files.flat_map(|file| file.archetypes())
    };
    let inside = archetypes()
        .filter_map(|archetype| archetype.interior())
        .flat_map(|interior| interior.entities())
        .filter(|entity| states_a_door(entity.extensions()));
    let stating = archetypes().filter(|archetype| states_a_door(archetype.extensions()));
    (stating.count(), inside.count())
}

/// The doors among the entities of `maps`, by the archetypes of `types`, which are in mount order.
pub fn doors_of(
    maps: &[MetaInstance],
    types: &[MetaInstance],
    names: &HashMap<u32, String>,
    patterns: &NamePatterns,
) -> DoorCount {
    let maps: Vec<MapData<'_>> = maps.iter().filter_map(MapData::new).collect();
    let types: Vec<MapTypes<'_>> = types.iter().filter_map(MapTypes::new).collect();
    let mut count = DoorCount::default();
    count.add(&find(
        &maps,
        &types,
        &|hash| names.get(&hash).cloned(),
        patterns,
    ));
    // A placed interior is never a door, whatever it states, so it is none the search owes.
    let entities = maps.iter().flat_map(|map| map.entities());
    count.stated_by_the_entity = entities
        .filter(|entity| !entity.is_interior_instance() && states_a_door(entity.extensions()))
        .count();
    count
}

/// What became of one game file made a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cooked {
    /// The source cooks to the canonical save of the file.
    Same,
    /// The file does not open and save, so it has no canonical save.
    Unopened(String),
    /// The file opens and no source is made of it, or its source does not cook.
    Refused(String),
    /// The source cooks to other bytes.
    Differs,
}

/// The canonical save of the `.ybn` or `.ynd` whose bytes are `bytes`, or why it has none.
pub fn canonical_save(extension: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let saved = match extension {
        "ybn" => PhBoundAny::from_bytes(bytes).and_then(|bound| bound.to_bytes()),
        _ => CPathRegion::from_bytes(bytes).and_then(|region| region.to_bytes()),
    };
    saved.map_err(|why| why.to_string())
}

/// Make a source of the `.ybn` or `.ynd` whose bytes are `bytes` under `work`, cook it and hold what it cooks to against `canonical`.
pub fn cooked_to(extension: &str, bytes: &[u8], work: &Path, canonical: &[u8]) -> Cooked {
    let how = Extraction {
        from_game: true,
        virtual_path: None,
    };
    let said = |problems: Vec<or_source::Problem>| {
        let said: Vec<String> = problems.into_iter().map(|p| p.message).collect();
        said.join("; ")
    };
    let folder = match extension {
        "ybn" => or_source::bounds::extract(bytes, "subject", work, &how),
        _ => or_source::paths::extract(bytes, "subject", work, &how),
    };
    let folder = match folder {
        Ok(folder) => folder,
        Err(why) => return Cooked::Refused(said(why)),
    };
    let source = match Source::open(&folder) {
        Ok(source) => source,
        Err(why) => return Cooked::Refused(why.message),
    };
    let compiled = match extension {
        "ybn" => or_source::bounds::compile(&source).map(|compiled| compiled.bytes),
        _ => or_source::paths::compile(&source).map(|compiled| compiled.bytes),
    };
    match compiled {
        Ok(cooked) if cooked == canonical => Cooked::Same,
        Ok(_) => Cooked::Differs,
        Err(why) => Cooked::Refused(said(why)),
    }
}

/// Make a source of the `.ybn` or `.ynd` whose bytes are `bytes` under `work`, cook it and hold it to the file's canonical save.
pub fn cooked_back(extension: &str, bytes: &[u8], work: &Path) -> Cooked {
    match canonical_save(extension, bytes) {
        Ok(canonical) => cooked_to(extension, bytes, work, &canonical),
        Err(why) => Cooked::Unopened(why),
    }
}

/// The copy a mount would load of each file of `extension`, as its logical path, in the mount's order.
///
/// ⚠ `listed` is every file of the mount as `(logical, leaf)`, in the mount's order: the last copy of a leaf is the one loaded.
pub fn winners_of(listed: &[(String, String)], extension: &str) -> Vec<String> {
    let mut last: BTreeMap<String, (usize, &str)> = BTreeMap::new();
    let mut at = 0;
    for (logical, leaf) in listed {
        let leaf = leaf.to_ascii_lowercase();
        if let Some(stem) = leaf.strip_suffix(extension) {
            last.insert(stem.to_string(), (at, logical));
            at += 1;
        }
    }
    let mut winners: Vec<(usize, &str)> = last.into_values().collect();
    winners.sort();
    let logicals = winners.into_iter().map(|(_, logical)| logical.to_string());
    logicals.collect()
}
