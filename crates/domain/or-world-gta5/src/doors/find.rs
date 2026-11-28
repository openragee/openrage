//! Which entities of a set of maps are doors: by the door extension the entity or its archetype states, then by the model's name.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use or_io::meta::Value;

use super::arc::Obstacle;
use crate::extension::ExtensionKind;
use crate::field;
use crate::keys;
use crate::map_data::{Entity, MapData};
use crate::map_types::{Archetype, Bounds, MapTypes};
use or_world::collision::OrientedBox;

/// The text of a name by its hash, where the caller knows one: the game's index of names, or the names a person typed.
pub type Names<'a> = &'a dyn Fn(u32) -> Option<String>;

/// Why a list of name patterns is not used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    #[error("the door pattern {0:?} matches every name, so it would call every model a door")]
    MatchesAll(String),
    #[error("the pattern {0:?} that says a name is no door's matches every name, so no model would be a door by its name")]
    ExcludesAll(String),
    #[error("a door pattern is empty, and no model has an empty name")]
    Empty,
}

/// The name patterns that say a model is a door, `*` standing for any run of characters and case counting for nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamePatterns {
    /// A name one of these matches is a door's.
    pub doors: Vec<String>,
    /// Unless one of these matches it too.
    pub not_doors: Vec<String>,
}

impl NamePatterns {
    /// Refuses a list one of whose patterns is empty, or matches every name: every model would be a door, or none would be one by its name.
    pub fn check(&self) -> Result<(), PatternError> {
        if self
            .doors
            .iter()
            .chain(&self.not_doors)
            .any(String::is_empty)
        {
            return Err(PatternError::Empty);
        }
        let stars_alone = |patterns: &[String]| {
            let all = patterns.iter().find(|p| p.chars().all(|c| c == '*'));
            all.cloned()
        };
        if let Some(pattern) = stars_alone(&self.doors) {
            return Err(PatternError::MatchesAll(pattern));
        }
        match stars_alone(&self.not_doors) {
            Some(pattern) => Err(PatternError::ExcludesAll(pattern)),
            None => Ok(()),
        }
    }

    /// The door pattern `name` matches, none where it matches none or one that says it is no door.
    pub fn door_by(&self, name: &str) -> Option<&str> {
        if self.not_doors.iter().any(|p| matches(p, name)) {
            return None;
        }
        (self.doors.iter())
            .find(|p| matches(p, name))
            .map(String::as_str)
    }
}

/// Whether `name` is `pattern`, each `*` of which stands for any run of characters.
fn matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let name: Vec<char> = name.chars().map(|c| c.to_ascii_lowercase()).collect();
    let (mut p, mut n) = (0, 0);
    // Where the last star stands and how much of the name it has taken so far.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if p < pattern.len() && pattern[p] == name[n] {
            p += 1;
            n += 1;
        } else if let Some((at, taken)) = star {
            star = Some((at, taken + 1));
            p = at + 1;
            n = taken + 1;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// Where an entity is among the maps handed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityAt {
    /// The map, by its place in the slice handed in.
    pub map: usize,
    /// The entity, by its place among the map's entities.
    pub entity: usize,
    /// For an entity an interior places: its place among the interior's entities, `entity` being the interior's instance.
    pub inside: Option<usize>,
}

/// One entity where the maps put it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEntity {
    pub at: EntityAt,
    /// The hash of the archetype it places, which is its model's name.
    pub archetype: u32,
    /// The text of that name, where the files or the lookup give one that hashes to it.
    pub name: Option<String>,
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// Its archetype's box in the model's own space, none where no type file handed in declares the archetype.
    pub bounds: Option<Bounds>,
    /// Whether it places an interior.
    pub interior_instance: bool,
}

impl PlacedEntity {
    /// Its box in the world, none where its archetype has none or the box is no finite size.
    pub fn world_box(&self) -> Option<OrientedBox> {
        let bounds = self.bounds?;
        let (a, b) = (bounds.min * self.scale, bounds.max * self.scale);
        let (min, max) = (a.min(b), a.max(b));
        let shape = OrientedBox {
            centre: self.position + self.rotation * ((min + max) * 0.5),
            rotation: self.rotation,
            half_extents: (max - min) * 0.5,
        };
        (shape.centre.is_finite() && shape.half_extents.is_finite()).then_some(shape)
    }
}

/// What said an entity is a door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoundBy {
    /// It states the door extension, or its archetype does.
    Extension,
    /// Its model's name matches this pattern.
    Name(String),
}

/// An entity that is a door.
#[derive(Debug, Clone, PartialEq)]
pub struct Door {
    pub entity: PlacedEntity,
    pub found_by: FoundBy,
}

/// The doors of a set of maps, and what the search could not judge.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub doors: Vec<Door>,
    /// Every other entity, in the order the maps place them.
    pub others: Vec<PlacedEntity>,
    /// How many entities were looked at, those an interior places among them.
    pub entities: usize,
    /// The entities that state no door extension, whose archetype states none and whose model has no name: a door among them is a door missed.
    pub unnamed: usize,
}

impl Found {
    /// The boxes the leaf of the door at `door` may meet: every other entity that has one, the other doors among them.
    pub fn obstacles_of(&self, door: usize) -> Vec<Obstacle> {
        let doors = self.doors.iter().enumerate();
        doors
            .filter(|(index, _)| *index != door)
            .map(|(_, other)| &other.entity)
            .chain(&self.others)
            .filter_map(|entity| {
                Some(Obstacle {
                    at: entity.at,
                    shape: entity.world_box()?,
                })
            })
            .collect()
    }
}

/// The hash a hash member states, with its text where the file or `names` gives one that hashes to it.
fn named(value: Option<&Value>, names: Names<'_>) -> (u32, Option<String>) {
    let hash = match value {
        Some(Value::Hash(h)) => {
            if !h.raw.is_empty() && or_io::string_hash(h.raw.as_bytes()) == h.hash {
                return (h.hash, Some(h.raw.clone()));
            }
            h.hash
        }
        Some(Value::UInt(v)) => *v as u32,
        _ => 0,
    };
    // A lookup that is wrong about a hash is not believed.
    let text = names(hash).filter(|text| or_io::string_hash(text.as_bytes()) == hash);
    (hash, text)
}

struct Declared<'a> {
    archetype: Archetype<'a>,
    name: Option<String>,
    states_door: bool,
}

/// Finds the doors among the entities `maps` place, by the archetypes `types` declare and the names `names` and `patterns` know.
pub fn find(
    maps: &[MapData<'_>],
    types: &[MapTypes<'_>],
    names: Names<'_>,
    patterns: &NamePatterns,
) -> Found {
    // ⚠ Of two type files that declare one name the later is believed, as `ArchetypeIndex` believes it, so the type files are
    // handed in mount order.
    let mut declared: HashMap<u32, Declared<'_>> = HashMap::new();
    for archetype in types.iter().flat_map(|file| file.archetypes()) {
        let (hash, name) = named(field::get(archetype.instance(), keys::NAME), names);
        if hash == 0 {
            continue;
        }
        let states_door = archetype
            .extensions()
            .any(|extension| extension.kind() == ExtensionKind::Door);
        declared.insert(
            hash,
            Declared {
                archetype,
                name,
                states_door,
            },
        );
    }

    let mut found = Found::default();
    let mut look = |entity: &Entity<'_>, at: EntityAt, within: Option<(Vec3, Quat)>| {
        let (archetype, own) = named(field::get(entity.instance(), keys::ARCHETYPE_NAME), names);
        let known = declared.get(&archetype);
        let (position, rotation) = match within {
            Some((origin, turn)) => (origin + turn * entity.position(), turn * entity.rotation()),
            None => (entity.position(), entity.rotation()),
        };
        let placed = PlacedEntity {
            at,
            archetype,
            name: own.or_else(|| known.and_then(|d| d.name.clone())),
            position,
            rotation,
            scale: entity.scale(),
            bounds: known.and_then(|d| d.archetype.bounds()),
            interior_instance: entity.is_interior_instance(),
        };
        found.entities += 1;
        // ⚠ The entity is asked first: in both installs measured the door extension is stated by entities, of the maps and
        // of the interiors, and by no archetype.
        let states_door = entity
            .extensions()
            .any(|extension| extension.kind() == ExtensionKind::Door);
        // ⚠ A placed interior is a building, never a leaf, whatever its name says.
        let found_by = if placed.interior_instance {
            None
        } else if states_door || known.is_some_and(|d| d.states_door) {
            Some(FoundBy::Extension)
        } else {
            match &placed.name {
                Some(name) => patterns
                    .door_by(name)
                    .map(|pattern| FoundBy::Name(pattern.to_string())),
                None => {
                    found.unnamed += 1;
                    None
                }
            }
        };
        match found_by {
            Some(found_by) => found.doors.push(Door {
                entity: placed,
                found_by,
            }),
            None => found.others.push(placed),
        }
    };
    for (map, data) in maps.iter().enumerate() {
        for (index, entity) in data.entities().enumerate() {
            let at = EntityAt {
                map,
                entity: index,
                inside: None,
            };
            look(&entity, at, None);
            if !entity.is_interior_instance() {
                continue;
            }
            // ⚠ An interior's own entities are stated in the interior's space, and its instance's rotation is stored as it is.
            // The instance's scale is not applied to them: whether the game scales an interior is not known here.
            // The entities of its entity sets are not looked at: `EntityAt` has no place for one.
            let Some(interior) = declared
                .get(&entity.archetype())
                .and_then(|d| d.archetype.interior())
            else {
                continue;
            };
            let within = (entity.position(), entity.rotation());
            for (inside, child) in interior.entities().enumerate() {
                let at = EntityAt {
                    inside: Some(inside),
                    ..at
                };
                look(&child, at, Some(within));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_stands_for_any_run_and_case_counts_for_nothing() {
        for (pattern, name, is) in [
            ("*door*", "shed_Door_01", true),
            ("*door*", "door", true),
            ("*door*", "shed_dor_01", false),
            ("yard_*_gate", "yard_side_gate", true),
            ("yard_*_gate", "yard_side_gate_l", false),
            ("a*b*c", "axxbxxc", true),
            ("a*b*c", "axxcxxb", false),
            ("exact", "exact", true),
            ("exact", "exactly", false),
            ("", "", true),
            ("", "x", false),
        ] {
            assert_eq!(matches(pattern, name), is, "{pattern:?} on {name:?}");
        }
    }

    #[test]
    fn a_pattern_of_stars_alone_is_refused_and_an_excluded_name_is_no_door() {
        let all = NamePatterns {
            doors: vec!["*door*".into(), "**".into()],
            not_doors: Vec::new(),
        };
        assert_eq!(all.check(), Err(PatternError::MatchesAll("**".into())));
        let empty = NamePatterns {
            doors: vec!["*door*".into()],
            not_doors: vec![String::new()],
        };
        assert_eq!(empty.check(), Err(PatternError::Empty));
        let none = NamePatterns {
            doors: vec!["*door*".into()],
            not_doors: vec!["*_lod".into(), "*".into()],
        };
        assert_eq!(none.check(), Err(PatternError::ExcludesAll("*".into())));
        let patterns = NamePatterns {
            doors: vec!["*door*".into()],
            not_doors: vec!["*frame*".into()],
        };
        assert_eq!(patterns.check(), Ok(()));
        assert_eq!(patterns.door_by("shed_door_a"), Some("*door*"));
        assert_eq!(patterns.door_by("shed_doorframe_a"), None);
        assert_eq!(patterns.door_by("shed_crate"), None);
    }
}
