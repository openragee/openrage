//! The rules: what is wrong with a module's map sets, as data with a code, a severity and a waiver.
//!
//! A rule is a row of [`RULES`], a finding is one place a rule applies, and a waiver is a line of the model that says why a finding
//! stands. Only an error that no reason could mend, such as a source that is not there, refuses to be waived.
//!
//! ⚠ Every rule is one this build can decide from the files: a source missing, a file that does not read, an index the engine trusts and
//! the reader does not enforce, a name two type files hold, a placement stated twice. Which of them fire on the game's own files was
//! measured over the owner's Legacy install (`what_the_rules_say_of_the_install`), and a rule that fires on a tenth of the game's maps
//! only warns. Nothing here says a map loads in game.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::facts::{ArchetypeFact, Fact, Facts, Role, SourceFact};
use crate::model::{MapSet, Maps};

pub use or_project::content::rule::{Rule, Severity};

const fn rule(
    code: &'static str,
    severity: Severity,
    waivable: bool,
    summary: &'static str,
) -> Rule {
    Rule::new(code, severity, waivable, summary)
}

use Severity::{Error, Info, Warning};

/// Every rule, in the order a report lists them.
pub const RULES: &[Rule] = &[
    rule(
        "source-escapes",
        Error,
        false,
        "a source is named by a path that leaves the module's folder",
    ),
    rule(
        "source-missing",
        Error,
        false,
        "a source a set names is not there",
    ),
    rule(
        "source-unreadable",
        Error,
        false,
        "a source a set names cannot be cooked, or is of a kind a map set does not hold",
    ),
    rule(
        "file-unreadable",
        Error,
        false,
        "the file a map source cooks to does not read as a map file or a type file",
    ),
    rule(
        "file-twice",
        Error,
        false,
        "two sources cook to one file of the resource",
    ),
    rule(
        "hierarchy-broken",
        Error,
        false,
        "a map names a LOD parent that is not in it, a parent cycle, or a child count the entities do not give: the engine trusts these and a save refuses them",
    ),
    rule(
        "interior-broken",
        Error,
        false,
        "an interior attaches an entity past its list, names a room it does not have, or holds an entity set whose two arrays differ",
    ),
    rule(
        "parent-link-broken",
        Warning,
        true,
        "an entity names a LOD parent in the map its own names as parent, past the entities that map holds in the pack",
    ),
    rule(
        "lod-children-uncounted",
        Warning,
        true,
        "an entity counts fewer LOD children than the entities of the pack's map files name it as their parent, as a removal alone in a parent map leaves it",
    ),
    rule(
        "archetype-twice",
        Warning,
        true,
        "two archetypes of a set's type files have one name, and the game resolves a name once",
    ),
    rule(
        "archetype-missing",
        Warning,
        true,
        "an entity places an archetype that no type file of the set declares and the game does not know",
    ),
    rule(
        "archetype-undeclared",
        Info,
        true,
        "an entity places an archetype that no type file of the set declares, so the game or another pack has to",
    ),
    rule(
        "entity-twice",
        Warning,
        true,
        "two entities of a map place one archetype at one position and one rotation",
    ),
    rule(
        "guid-twice",
        Warning,
        true,
        "two entities of a map state one guid",
    ),
    rule(
        "extents-short",
        Warning,
        true,
        "the extents a map states do not hold the entities it places, and the game streams a map by them",
    ),
    rule(
        "bounds-inverted",
        Warning,
        true,
        "an archetype's bounding box has a minimum above its maximum",
    ),
    rule(
        "source-twice",
        Warning,
        true,
        "two sets name one source, which a build ships once",
    ),
    rule(
        "source-unused",
        Warning,
        true,
        "a source folder of the module is named by no set, so a build ships nothing of it",
    ),
    rule(
        "model-unused",
        Info,
        true,
        "a model of a set is named as the asset of no archetype of its type files",
    ),
    rule(
        "archetype-without-model",
        Info,
        true,
        "an archetype of a set's type files names an asset that no model of the set holds, so the game or another pack has to",
    ),
    rule(
        "no-maps",
        Info,
        true,
        "the module names no map set, so a build ships nothing of it",
    ),
    rule(
        "waiver-without-reason",
        Error,
        false,
        "a waiver gives no reason",
    ),
    rule("waiver-unknown", Warning, false, "a waiver names no rule"),
];

/// The rule `code` names, the door rules of [`crate::doors::DOOR_RULES`] among them.
pub fn rule_of(code: &str) -> Option<&'static Rule> {
    RULES
        .iter()
        .chain(crate::doors::DOOR_RULES)
        .find(|r| r.code == code)
}

/// How much the extents may fall short of an entity before a rule says so, in world units.
///
/// Measured: 17,191 of 19,339 maps of the owner's Legacy install state extents that hold every entity within a unit, and the rest fall
/// short by a good deal more, so a unit is what separates rounding from a map the engine will stream wrongly.
pub const EXTENTS_SLACK: f32 = 1.0;

/// Where a finding is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Place {
    /// The source folder, with `/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The index of the entity in its map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<usize>,
    /// The index of the archetype in its type file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archetype: Option<usize>,
}

/// One place a rule applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    /// The id of the set, none for the module.
    pub map: Option<String>,
    pub place: Place,
    pub message: String,
    /// The reason of the waiver that sets it aside.
    pub waived: Option<String>,
    /// For a finding about a waiver, which one: its index in the model's list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiver: Option<usize>,
}

impl Finding {
    /// Whether a build must stop for it.
    pub fn blocks(&self) -> bool {
        self.severity == Severity::Error && self.waived.is_none()
    }
}

/// What to call a set in a sentence.
pub fn name_of(set: &MapSet) -> String {
    format!("`{}`", set.name)
}

/// What the game is asked about a name: whether it knows an archetype of that hash. None when nobody could ask.
pub type Known<'a> = Option<&'a dyn Fn(u32) -> bool>;

/// Every rule applied to `model`, `facts` being what the disk said of the sources it names.
///
/// A finding of a rule a waiver names carries the waiver's reason and no longer blocks; a waiver without a reason, or for a rule that
/// does not exist, is itself a finding.
pub fn check(model: &Maps, facts: &Facts, known: Known<'_>) -> Vec<Finding> {
    let mut found: Vec<Finding> = Vec::new();
    let mut say = |code: &'static str, set: Option<&MapSet>, place: Place, message: String| {
        let severity = rule_of(code).map_or(Error, |r| r.severity);
        found.push(Finding {
            code,
            severity,
            map: set.map(|s| s.id.clone()),
            place,
            message,
            waived: None,
            waiver: None,
        });
    };
    let at = |source: &str| Place {
        source: Some(source.to_string()),
        ..Place::default()
    };

    if model.maps.is_empty() {
        say(
            "no-maps",
            None,
            Place::default(),
            "the module names no map set, so a build ships nothing of it".to_string(),
        );
    }
    let mut named: BTreeMap<&str, Vec<&MapSet>> = BTreeMap::new();
    let mut cooked_to: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    // How many entities each map file of the pack holds, by the hash of its name: what a link into a parent map indexes.
    let held: BTreeMap<u32, usize> = (model.maps.iter())
        .flat_map(|set| &set.sources)
        .filter_map(|path| facts.source(path)?.map())
        .map(|map| (map.name.hash, map.entities.len()))
        .collect();
    let children = Children::of(model, facts);
    for set in &model.maps {
        let who = name_of(set);
        for path in &set.sources {
            let path = path.as_str();
            let places = named.entry(path).or_default();
            if !places.iter().any(|s| s.id == set.id) {
                places.push(set);
            }
            match facts.get(path) {
                Some(Fact::Escapes) | None => say(
                    "source-escapes",
                    Some(set),
                    at(path),
                    format!("{who} names `{path}`, which leaves the folder of its module"),
                ),
                Some(Fact::Missing) => say(
                    "source-missing",
                    Some(set),
                    at(path),
                    format!("{who} names `{path}`, which is not in the folder of its module"),
                ),
                Some(Fact::Unreadable(why)) => say(
                    "source-unreadable",
                    Some(set),
                    at(path),
                    format!("`{path}` of {who} {why}"),
                ),
                Some(Fact::Source(source)) => {
                    let places = cooked_to
                        .entry(crate::layout::stream_path(&source.file_name()).to_ascii_lowercase())
                        .or_default();
                    if !places.contains(&path) {
                        places.push(path);
                    }
                    if let Some(Err(why)) = &source.inside {
                        say(
                            "file-unreadable",
                            Some(set),
                            at(path),
                            format!("`{path}` of {who} {why}"),
                        );
                    }
                }
            }
        }
        check_set(set, facts, known, &held, &children, &mut say);
    }
    for (path, held) in named.iter().filter(|(_, held)| held.len() > 1) {
        for set in held {
            say(
                "source-twice",
                Some(set),
                at(path),
                format!("`{path}` is named by {} sets", held.len()),
            );
        }
    }
    for path in &facts.unused {
        say(
            "source-unused",
            None,
            at(path),
            format!("`{path}` is named by no set, so a build ships nothing of it"),
        );
    }
    for (file, places) in cooked_to.iter().filter(|(_, places)| places.len() > 1) {
        say(
            "file-twice",
            None,
            Place::default(),
            format!("`{file}` is what {} all cook to", places.join(", ")),
        );
    }
    waive(model, &mut found);
    found
}

/// The sources of `set` that are of `role`, with what the disk said of them.
fn of_role<'f>(
    set: &'f MapSet,
    facts: &'f Facts,
    role: Role,
) -> impl Iterator<Item = (&'f str, &'f SourceFact)> {
    set.sources.iter().filter_map(move |path| {
        let source = facts.source(path)?;
        (source.role() == Some(role)).then_some((path.as_str(), source))
    })
}

/// How many entities of the map files of a pack name each entity as their LOD parent.
struct Children {
    /// By the hash of the name of the map the parent is in, and its index there.
    named: BTreeMap<(u32, usize), u32>,
    /// The maps two files of the pack each hold entities of: an index into one is into the two put together, which no file states.
    split: BTreeSet<u32>,
}

impl Children {
    fn of(model: &Maps, facts: &Facts) -> Self {
        let mut named: BTreeMap<(u32, usize), u32> = BTreeMap::new();
        let mut files: BTreeMap<u32, u32> = BTreeMap::new();
        let maps = crate::facts::named_paths(model).filter_map(|path| facts.source(path)?.map());
        for map in maps.filter(|map| !map.entities.is_empty()) {
            *files.entry(map.name.hash).or_default() += 1;
            for entity in &map.entities {
                let Ok(index) = usize::try_from(entity.parent_index) else {
                    continue;
                };
                let into = match entity.flags & (1 << 3) != 0 {
                    true => map.parent.hash,
                    false => map.name.hash,
                };
                *named.entry((into, index)).or_default() += 1;
            }
        }
        let split = (files.into_iter())
            .filter(|(_, files)| *files > 1)
            .map(|(name, _)| name)
            .collect();
        Self { named, split }
    }

    /// The entities of `map` that count fewer children than name them: each with the count it states and the count that name it.
    fn uncounted(&self, map: &crate::facts::MapInside) -> Vec<(usize, u32, u32)> {
        if map.name.hash == 0 || self.split.contains(&map.name.hash) {
            return Vec::new();
        }
        (map.entities.iter().enumerate())
            .filter_map(|(index, entity)| {
                let named = *self.named.get(&(map.name.hash, index))?;
                (named > entity.num_children).then_some((index, entity.num_children, named))
            })
            .collect()
    }
}

/// The rules that read inside the files of one set.
fn check_set(
    set: &MapSet,
    facts: &Facts,
    known: Known<'_>,
    held: &BTreeMap<u32, usize>,
    children: &Children,
    say: &mut impl FnMut(&'static str, Option<&MapSet>, Place, String),
) {
    let who = name_of(set);
    // The archetypes the set's type files declare, by hash, with where each is.
    let mut declared: BTreeMap<u32, Vec<(&str, usize, &ArchetypeFact)>> = BTreeMap::new();
    let mut asset_names: Vec<u32> = Vec::new();
    for (path, source) in of_role(set, facts, Role::Types) {
        let Some(types) = source.types() else {
            continue;
        };
        for (index, archetype) in types.archetypes.iter().enumerate() {
            declared
                .entry(archetype.name.hash)
                .or_default()
                .push((path, index, archetype));
            asset_names.push(archetype.asset_name.hash);
            asset_names.push(archetype.name.hash);
            if archetype
                .bb_min
                .iter()
                .zip(&archetype.bb_max)
                .any(|(a, b)| a > b)
            {
                say(
                    "bounds-inverted",
                    Some(set),
                    Place {
                        source: Some(path.to_string()),
                        archetype: Some(index),
                        ..Place::default()
                    },
                    format!(
                        "the archetype `{}` of `{path}` has a bounding box whose minimum is above its maximum",
                        archetype.name.shown()
                    ),
                );
            }
            if let Some(interior) = &archetype.interior {
                for problem in &interior.problems {
                    say(
                        "interior-broken",
                        Some(set),
                        Place {
                            source: Some(path.to_string()),
                            archetype: Some(index),
                            ..Place::default()
                        },
                        format!(
                            "the interior `{}` of `{path}`: {problem}",
                            archetype.name.shown()
                        ),
                    );
                }
            }
        }
    }
    for (hash, held) in declared.iter().filter(|(_, held)| held.len() > 1) {
        let _ = hash;
        let (path, index, archetype) = held[0];
        say(
            "archetype-twice",
            Some(set),
            Place {
                source: Some(path.to_string()),
                archetype: Some(index),
                ..Place::default()
            },
            format!(
                "the archetype `{}` is declared {} times in {who}: {}",
                archetype.name.shown(),
                held.len(),
                held.iter()
                    .map(|(p, i, _)| format!("`{p}` #{i}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }

    let mut placed_assets: Vec<u32> = Vec::new();
    for (path, source) in of_role(set, facts, Role::Map) {
        let Some(map) = source.map() else {
            continue;
        };
        for problem in &map.problems {
            say(
                "hierarchy-broken",
                Some(set),
                Place {
                    source: Some(path.to_string()),
                    ..Place::default()
                },
                format!("`{path}`: {problem}"),
            );
        }
        // ⚠ An entity whose `flags` bit 3 is set indexes the entities of the parent map, so an entity taken out of that map leaves
        // the link past its end or on another entity; only the first is something the files can show.
        let parent_holds = held.get(&map.parent.hash).filter(|_| map.parent.hash != 0);
        let past = parent_holds.and_then(|len| {
            let past = |e: &crate::facts::EntityFact| {
                e.flags & (1 << 3) != 0 && usize::try_from(e.parent_index).is_ok_and(|p| p >= *len)
            };
            let first = map.entities.iter().position(past)?;
            Some((first, map.entities.iter().filter(|e| past(e)).count(), *len))
        });
        if let Some((first, count, len)) = past {
            say(
                "parent-link-broken",
                Some(set),
                Place {
                    source: Some(path.to_string()),
                    entity: Some(first),
                    ..Place::default()
                },
                format!(
                    "{count} entities of `{path}` name a LOD parent in the parent map `{}` past the {len} entities it holds, the first #{first} naming entity {}",
                    map.parent.shown(),
                    map.entities[first].parent_index
                ),
            );
        }
        // ⚠ A link that is not past the end names another entity once an entity before it is taken out of the parent map alone,
        // which the counts show: a pack whose counts held before has one too low after it, or a link past the end.
        if let [(first, stated, named), rest @ ..] = children.uncounted(map).as_slice() {
            say(
                "lod-children-uncounted",
                Some(set),
                Place {
                    source: Some(path.to_string()),
                    entity: Some(*first),
                    ..Place::default()
                },
                format!(
                    "{} entities of `{path}` count fewer LOD children than name them among the map files of the pack, the first #{first} counting {stated} where {named} name it: a removal alone in a parent map leaves the links into it on other entities, and `openrage-cli maps delete` plans one with its links",
                    rest.len() + 1
                ),
            );
        }
        let mut guids: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        let mut placements: BTreeMap<(u32, [u32; 3], [u32; 4]), Vec<usize>> = BTreeMap::new();
        let mut undeclared: BTreeMap<u32, (usize, String)> = BTreeMap::new();
        for (index, entity) in map.entities.iter().enumerate() {
            placed_assets.push(entity.archetype.hash);
            if entity.guid != 0 {
                guids.entry(entity.guid).or_default().push(index);
            }
            placements
                .entry((
                    entity.archetype.hash,
                    entity.position.map(f32::to_bits),
                    entity.rotation.map(f32::to_bits),
                ))
                .or_default()
                .push(index);
            if !declared.contains_key(&entity.archetype.hash) {
                undeclared
                    .entry(entity.archetype.hash)
                    .or_insert_with(|| (index, entity.archetype.shown()));
            }
        }
        let on_entity = |index: usize| Place {
            source: Some(path.to_string()),
            entity: Some(index),
            ..Place::default()
        };
        for (guid, held) in guids.iter().filter(|(_, held)| held.len() > 1) {
            say(
                "guid-twice",
                Some(set),
                on_entity(held[1]),
                format!(
                    "{} entities of `{path}` state the guid {guid}: {}",
                    held.len(),
                    numbers(held)
                ),
            );
        }
        for held in placements.values().filter(|held| held.len() > 1) {
            let archetype = map.entities[held[0]].archetype.shown();
            say(
                "entity-twice",
                Some(set),
                on_entity(held[1]),
                format!(
                    "{} entities of `{path}` place `{archetype}` at one position and one rotation: {}",
                    held.len(),
                    numbers(held)
                ),
            );
        }
        for (hash, (index, shown)) in &undeclared {
            match known {
                Some(game_knows) if game_knows(*hash) => {}
                Some(_) => say(
                    "archetype-missing",
                    Some(set),
                    on_entity(*index),
                    format!(
                        "`{path}` places `{shown}`, which no type file of {who} declares and the game does not know"
                    ),
                ),
                None => say(
                    "archetype-undeclared",
                    Some(set),
                    on_entity(*index),
                    format!("`{path}` places `{shown}`, which no type file of {who} declares"),
                ),
            }
        }
        if let Some((short, at_least)) = extents_short(map, &declared) {
            let at_least = if at_least { "at least " } else { "" };
            say(
                "extents-short",
                Some(set),
                at_source(path),
                format!(
                    "the extents `{path}` states fall {at_least}{short:.1} short of the entities it places"
                ),
            );
        }
    }

    for (path, source) in of_role(set, facts, Role::Model) {
        let name = or_io::string_hash(source.name.to_ascii_lowercase().as_bytes());
        if !asset_names.contains(&name) && !placed_assets.contains(&name) {
            say(
                "model-unused",
                Some(set),
                at_source(path),
                format!(
                    "`{path}` of {who} is the asset of no archetype of its type files, and no entity places it by name"
                ),
            );
        }
    }
    let models: Vec<u32> = of_role(set, facts, Role::Model)
        .map(|(_, s)| or_io::string_hash(s.name.to_ascii_lowercase().as_bytes()))
        .collect();
    for (hash, held) in &declared {
        let (path, index, archetype) = held[0];
        let asset = if archetype.asset_name.is_empty() {
            *hash
        } else {
            archetype.asset_name.hash
        };
        let dictionary = archetype.drawable_dictionary.hash;
        if !models.contains(&asset) && !(dictionary != 0 && models.contains(&dictionary)) {
            say(
                "archetype-without-model",
                Some(set),
                Place {
                    source: Some(path.to_string()),
                    archetype: Some(index),
                    ..Place::default()
                },
                format!(
                    "the archetype `{}` of `{path}` names the asset `{}`, which no model of {who} holds",
                    archetype.name.shown(),
                    if archetype.asset_name.is_empty() {
                        archetype.name.shown()
                    } else {
                        archetype.asset_name.shown()
                    }
                ),
            );
        }
    }
}

fn at_source(path: &str) -> Place {
    Place {
        source: Some(path.to_string()),
        ..Place::default()
    }
}

fn numbers(held: &[usize]) -> String {
    held.iter()
        .map(|i| format!("#{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How far the extents a map states fall short of its entities, none when they hold them, and whether that is a lower bound.
///
/// ⚠ Only an entity whose archetype a type file of the set declares has a box to place. Any other is judged by where it stands, which
/// the box of its entities and the box the map streams by both have to hold whatever its box is, so a map that places one is short
/// by at least what this says (`or_world::extents::Extents::is_complete`).
fn extents_short(
    map: &crate::facts::MapInside,
    declared: &BTreeMap<u32, Vec<(&str, usize, &ArchetypeFact)>>,
) -> Option<(f32, bool)> {
    let (min, max) = map.stored_entities?;
    let (min, max) = (glam::Vec3::from(min), glam::Vec3::from(max));
    let streaming = map
        .stored_streaming
        .map(|(min, max)| (glam::Vec3::from(min), glam::Vec3::from(max)));
    let outside = |(min, max): (glam::Vec3, glam::Vec3), point: glam::Vec3| {
        (min - point).max_element().max((point - max).max_element())
    };
    let mut over = 0.0_f32;
    let mut at_least = false;
    for entity in &map.entities {
        let position = glam::Vec3::from(entity.position);
        let Some((_, _, archetype)) = declared.get(&entity.archetype.hash).and_then(|d| d.first())
        else {
            at_least = true;
            over = over.max(outside((min, max), position));
            if let Some(streaming) = streaming {
                over = over.max(outside(streaming, position));
            }
            continue;
        };
        let placement = if entity.interior {
            or_world::Placement::InteriorInstance
        } else {
            or_world::Placement::Entity
        };
        let scale = or_world::transform::scale_of(entity.scale_xy, entity.scale_z);
        let corners = or_world::transform::placed_corners(
            glam::Vec3::from(archetype.bb_min) * scale,
            glam::Vec3::from(archetype.bb_max) * scale,
            placement.rotation_of(entity.rotation),
            position,
        );
        for corner in corners {
            over = over.max(outside((min, max), corner));
        }
    }
    (over > EXTENTS_SLACK).then_some((over, at_least))
}

/// Set aside the findings a waiver of the model names, and add the findings about the waivers.
fn waive(model: &Maps, found: &mut Vec<Finding>) {
    for (index, waiver) in model.waivers.iter().enumerate() {
        let Some(rule) = rule_of(&waiver.code) else {
            found.push(Finding {
                code: "waiver-unknown",
                severity: Warning,
                map: waiver.map.clone(),
                place: Place::default(),
                message: format!("a waiver names `{}`, which is no rule", waiver.code),
                waived: None,
                waiver: Some(index),
            });
            continue;
        };
        if waiver.reason.trim().is_empty() {
            found.push(Finding {
                code: "waiver-without-reason",
                severity: Error,
                map: waiver.map.clone(),
                place: Place::default(),
                message: format!("the waiver of `{}` gives no reason", waiver.code),
                waived: None,
                waiver: Some(index),
            });
            continue;
        }
        if !rule.waivable {
            continue;
        }
        for finding in found.iter_mut().filter(|f| {
            f.code == waiver.code
                && f.waived.is_none()
                && waiver
                    .map
                    .as_ref()
                    .is_none_or(|id| f.map.as_ref() == Some(id))
        }) {
            finding.waived = Some(waiver.reason.clone());
        }
    }
}
