//! The doors of a pack's map sets: found, read, paired, opened against what stands near them, and held to the door rules.
//!
//! The finding and the geometry are `or_world::doors`'. This is what a pack adds: the name patterns that say a model is a door, kept
//! as data in `door_patterns.json`; the door rules, as data with the codes and the waivers of [`crate::rules`]; and one row for each
//! door of a set, which `openrage-cli maps doors` lists as text and as JSON.
//!
//! ⚠ **A door's hinge, motion and arc are read from its archetype's box.** The game's door tuning is not read, so an arc opens to
//! [`DOOR_OPENS_TO`] each way whatever the game allows the door, and a rule about an arc is a warning.

use std::fmt;
use std::path::Path;

use glam::{Quat, Vec3};
use or_io::meta::MetaDocument;
use or_world::collision::{CollisionSet, OrientedBox};
use or_world::doors::{
    clearance, find, pair, read_leaf, Blocker, Clearance, EntityAt, FoundBy, HingeSide, Leaf,
    Motion, NamePatterns, PatternError, PlacedEntity, Unread, Way, WidthAxis,
};
use or_world::{MapData, MapTypes};
use serde::{Deserialize, Serialize};

use crate::edit::Pose;
use crate::facts::{cooked, inspect, Depth, Fact, Name, Names, Role};
use crate::model::{MapSet, Maps};
use crate::rules::{Finding, Place, Rule, Severity};
use crate::script_text::heading_of;

/// What the `format` of a file of door patterns says.
pub const PATTERNS_FORMAT: &str = "openrage-door-patterns";
/// The newest schema of a file of door patterns this build reads.
pub const PATTERNS_SCHEMA: u32 = 1;

const BUILT_IN_PATTERNS: &str = include_str!("door_patterns.json");

/// How far a door is opened each way when its arc is held against what stands near it, in radians: a quarter turn.
pub const DOOR_OPENS_TO: f32 = std::f32::consts::FRAC_PI_2;

/// The turn short of which a door stopped both ways is said not to open, in radians: half of a quarter turn, a judgement and no measure of the game.
pub const DOOR_OPENS_FROM: f32 = std::f32::consts::FRAC_PI_4;

/// How far two closed leaves may reach into each other before they are said to overlap, in metres on each face: the rebate where two leaves meet.
pub const DOORS_OVERLAP_SLACK: f32 = 0.05;

/// The door rules, in the order a report lists them.
pub const DOOR_RULES: &[Rule] = &[
    Rule::new(
        "doors-overlap",
        Severity::Warning,
        true,
        "two doors stand in one place: their closed leaves reach into each other",
    ),
    Rule::new(
        "door-blocked",
        Severity::Warning,
        true,
        "a door cannot open either way: something stands in its arc both ways before it has opened far enough to pass",
    ),
];

/// Why a file of door patterns was not read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the door patterns are not read: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the door patterns are not read: their format is `{0}`, not `{PATTERNS_FORMAT}`")]
    Format(String),
    #[error(
        "the door patterns were written by a newer OpenRage (schema {0}) than this one reads (schema {PATTERNS_SCHEMA})"
    )]
    Newer(u32),
    #[error("the door patterns are not used: {0}")]
    Pattern(#[from] PatternError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternFile {
    format: String,
    schema: u32,
    /// What the file says of itself to the person who extends it.
    #[serde(default)]
    #[allow(dead_code)]
    note: String,
    doors: Vec<String>,
    #[serde(default)]
    not_doors: Vec<String>,
}

/// The name patterns the tree holds: a starting guess, which `door_patterns.json` says of itself.
pub fn built_in_patterns() -> Result<NamePatterns, Error> {
    patterns_from_json(BUILT_IN_PATTERNS)
}

/// Read a file of door patterns, refusing one a pattern of which would call every model a door.
pub fn patterns_from_json(text: &str) -> Result<NamePatterns, Error> {
    let file: PatternFile = serde_json::from_str(text)?;
    if file.format != PATTERNS_FORMAT {
        return Err(Error::Format(file.format));
    }
    if file.schema > PATTERNS_SCHEMA {
        return Err(Error::Newer(file.schema));
    }
    let patterns = NamePatterns {
        doors: file.doors,
        not_doors: file.not_doors,
    };
    patterns.check()?;
    Ok(patterns)
}

/// What a door's leaf was read as.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeafRow {
    /// `swing`, `slide` or `lift`: a proposal read from the box.
    pub motion: &'static str,
    /// What in the box the proposal rests on.
    pub reason: String,
    /// The end of the width the hinge is at, `low` or `high`, for a leaf that swings.
    pub hinge: Option<&'static str>,
    /// The model's axis the width runs along, `x` or `y`.
    pub width_axis: &'static str,
    /// In metres, as the other two.
    pub width: f32,
    pub thickness: f32,
    pub height: f32,
}

/// How far a door opens one way.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRow {
    /// `anticlockwise` or `clockwise`, seen from above.
    pub way: &'static str,
    /// The angle it reaches, in degrees.
    pub reaches: f32,
    /// What it met first, none where it opens all the way.
    pub blocked_by: Option<String>,
}

/// One door of a set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DoorRow {
    /// The map source it is in, a folder of the module with `/`.
    pub source: String,
    /// Its index among the entities of that map file; for a door an interior places, the index of the interior's instance.
    pub entity: usize,
    /// For a door an interior places, its index among the interior's entities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside: Option<usize>,
    /// The model's name, or `hash_XXXXXXXX` where none is known.
    pub model: String,
    pub hash: u32,
    /// `extension` where its archetype states the door extension, `name` where a pattern matched its model's name.
    pub found_by: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    pub position: [f32; 3],
    /// The turn about the vertical axis a script takes, in degrees from 0 up to 360.
    pub heading: f32,
    pub leaf: Option<LeafRow>,
    /// Why its leaf was not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread: Option<String>,
    /// The other leaf of its double door, by its place among the set's doors.
    pub pair: Option<usize>,
    /// How far it opens each way, for a leaf that swings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub opens: Vec<OpenRow>,
}

impl DoorRow {
    /// What the door is called in a line of a list: its source and its place in it.
    pub fn id(&self) -> String {
        match self.inside {
            Some(inside) => format!("{}#{}.{inside}", self.source, self.entity),
            None => format!("{}#{}", self.source, self.entity),
        }
    }
}

/// The doors of one map set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetDoors {
    /// The id of the set.
    pub set: String,
    pub name: String,
    /// How many entities were looked at, those the set's interiors place among them.
    pub entities: usize,
    /// The entities whose archetype states no door extension and whose model has no name: a door among them is a door missed.
    pub unnamed: usize,
    /// How many bounds of the map's collision the arcs were held against; with none, an arc meets the boxes of other entities alone.
    pub colliders: usize,
    pub doors: Vec<DoorRow>,
    /// The double doors, each as the places of its two leaves among `doors`.
    pub pairs: Vec<[usize; 2]>,
    pub findings: Vec<Finding>,
    /// The map files and type files of the set that were not read, each with the reason.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<(String, String)>,
}

fn word_of(motion: Motion) -> &'static str {
    match motion {
        Motion::Swing => "swing",
        Motion::Slide => "slide",
        Motion::Lift => "lift",
    }
}

fn way_of(way: Way) -> &'static str {
    match way {
        Way::Anticlockwise => "anticlockwise",
        Way::Clockwise => "clockwise",
    }
}

fn leaf_row(leaf: &Leaf) -> LeafRow {
    LeafRow {
        motion: word_of(leaf.proposal.motion),
        reason: leaf.proposal.reason.to_string(),
        hinge: leaf.hinge.map(|hinge| match hinge.side {
            HingeSide::Low => "low",
            HingeSide::High => "high",
        }),
        width_axis: match leaf.width_axis {
            WidthAxis::X => "x",
            WidthAxis::Y => "y",
        },
        width: leaf.width,
        thickness: leaf.thickness,
        height: leaf.height,
    }
}

/// `leaf`'s closed box, each face drawn in by [`DOORS_OVERLAP_SLACK`].
fn drawn_in(leaf: &Leaf) -> OrientedBox {
    let mut closed = leaf.box_at(0.0);
    closed.half_extents = (closed.half_extents - DOORS_OVERLAP_SLACK).max(Vec3::ZERO);
    closed
}

/// The heading a script takes for an entity turned by `rotation`, in degrees.
pub fn heading(rotation: Quat) -> f32 {
    let heading = heading_of(&Pose {
        position: Vec3::ZERO,
        rotation,
        scale_xy: 1.0,
        scale_z: 1.0,
    });
    // ⚠ No turn at all comes out as a nought with a sign, which a script would be handed as `-0`.
    heading + 0.0
}

/// The doors of `set`, whose map files are `maps`, each with the source it is, and whose type files are `types`, each arc held against `colliders`.
pub fn survey<K: Ord + Clone + fmt::Display>(
    model: &Maps,
    set: &MapSet,
    maps: &[(&str, MapData<'_>)],
    types: &[MapTypes<'_>],
    names: or_world::doors::Names<'_>,
    patterns: &NamePatterns,
    colliders: &CollisionSet<K>,
) -> SetDoors {
    let views: Vec<MapData<'_>> = maps.iter().map(|(_, map)| *map).collect();
    let found = find(&views, types, names, patterns);
    let source_of = |at: EntityAt| maps.get(at.map).map_or("", |(source, _)| *source);
    let shown = |entity: &PlacedEntity| {
        Name {
            hash: entity.archetype,
            text: entity.name.clone(),
        }
        .shown()
    };
    let called = |at: EntityAt| {
        let entity = (found.doors.iter().map(|door| &door.entity))
            .chain(&found.others)
            .find(|entity| entity.at == at);
        let place = match at.inside {
            Some(inside) => format!("#{}.{inside}", at.entity),
            None => format!("#{}", at.entity),
        };
        match entity {
            Some(entity) => format!("`{}` at {}{place}", shown(entity), source_of(at)),
            None => format!("the entity at {}{place}", source_of(at)),
        }
    };

    let leaves: Vec<Result<Leaf, Unread>> = (found.doors.iter())
        .map(|door| read_leaf(&door.entity))
        .collect();
    let read: Vec<Option<Leaf>> = leaves.iter().map(|leaf| leaf.ok()).collect();
    let pairs = pair(&read);
    let partner = |door: usize| {
        pairs.iter().find_map(|p| match (p.a == door, p.b == door) {
            (true, _) => Some(p.b),
            (_, true) => Some(p.a),
            _ => None,
        })
    };

    let mut say = Findings {
        set,
        found: Vec::new(),
    };
    let mut doors = Vec::with_capacity(found.doors.len());
    for (index, (door, leaf)) in found.doors.iter().zip(&leaves).enumerate() {
        let entity = &door.entity;
        let source = source_of(entity.at).to_string();
        let mut opens = Vec::new();
        if let Some(leaf) = leaf.as_ref().ok().filter(|leaf| leaf.hinge.is_some()) {
            // ⚠ The other leaf of a double door opens with this one: it is no furniture, however far its box reaches into the arc.
            let other_leaf = partner(index).map(|other| found.doors[other].entity.at);
            let mut obstacles = found.obstacles_of(index);
            obstacles.retain(|obstacle| Some(obstacle.at) != other_leaf);
            let both: Vec<Clearance<K>> = [Way::Anticlockwise, Way::Clockwise]
                .into_iter()
                .filter_map(|way| clearance(leaf, way, DOOR_OPENS_TO, colliders, &obstacles).ok())
                .collect();
            for cleared in &both {
                opens.push(OpenRow {
                    way: way_of(cleared.way),
                    reaches: cleared.reaches().to_degrees(),
                    blocked_by: cleared.blocked.as_ref().map(|blocked| match &blocked.by {
                        Blocker::Collider { key, .. } => format!("the bound {key}"),
                        Blocker::Entity(at) => called(*at),
                    }),
                });
            }
            if both.len() == 2 && both.iter().all(|c| c.reaches() < DOOR_OPENS_FROM) {
                say.of(
                    "door-blocked",
                    &source,
                    entity.at.entity,
                    format!(
                        "the door {} cannot open either way: {}",
                        called(entity.at),
                        opens
                            .iter()
                            .map(|open| format!(
                                "{} it stops at {:.1} degrees on {}",
                                open.way,
                                open.reaches,
                                open.blocked_by.as_deref().unwrap_or("nothing")
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
        }
        doors.push(DoorRow {
            source,
            entity: entity.at.entity,
            inside: entity.at.inside,
            model: shown(entity),
            hash: entity.archetype,
            found_by: match door.found_by {
                FoundBy::Extension => "extension",
                FoundBy::Name(_) => "name",
            },
            pattern: match &door.found_by {
                FoundBy::Extension => None,
                FoundBy::Name(pattern) => Some(pattern.clone()),
            },
            position: entity.position.to_array(),
            heading: heading(entity.rotation),
            leaf: leaf.as_ref().ok().map(leaf_row),
            unread: leaf.as_ref().err().map(ToString::to_string),
            pair: partner(index),
            opens,
        });
    }
    for (first, a) in read.iter().enumerate() {
        let Some(a) = a else { continue };
        for (second, b) in read.iter().enumerate().skip(first + 1) {
            let Some(b) = b else { continue };
            if drawn_in(a).touches(&drawn_in(b)) {
                let (one, other) = (&found.doors[first].entity, &found.doors[second].entity);
                say.of(
                    "doors-overlap",
                    source_of(other.at),
                    other.at.entity,
                    format!(
                        "the doors {} and {} stand in one place: their closed leaves reach into each other",
                        called(one.at),
                        called(other.at)
                    ),
                );
            }
        }
    }
    let mut findings = say.found;
    waive(model, &mut findings);
    SetDoors {
        set: set.id.clone(),
        name: set.name.clone(),
        entities: found.entities,
        unnamed: found.unnamed,
        colliders: colliders.len(),
        doors,
        pairs: pairs.iter().map(|p| [p.a, p.b]).collect(),
        findings,
        unread: Vec::new(),
    }
}

struct Findings<'a> {
    set: &'a MapSet,
    found: Vec<Finding>,
}

impl Findings<'_> {
    fn of(&mut self, code: &'static str, source: &str, entity: usize, message: String) {
        let severity = (DOOR_RULES.iter())
            .find(|rule| rule.code == code)
            .map_or(Severity::Warning, |rule| rule.severity);
        self.found.push(Finding {
            code,
            severity,
            map: Some(self.set.id.clone()),
            place: Place {
                source: Some(source.to_string()),
                entity: Some(entity),
                archetype: None,
            },
            message,
            waived: None,
            waiver: None,
        });
    }
}

/// Set aside the door findings a waiver of the model names; a waiver with no reason sets nothing aside, and [`crate::check`] says so.
fn waive(model: &Maps, found: &mut [Finding]) {
    for waiver in (model.waivers.iter()).filter(|w| !w.reason.trim().is_empty()) {
        let waivable = DOOR_RULES
            .iter()
            .any(|rule| rule.code == waiver.code && rule.waivable);
        if !waivable {
            continue;
        }
        for finding in found.iter_mut().filter(|f| {
            let of_set = waiver
                .map
                .as_ref()
                .is_none_or(|id| f.map.as_ref() == Some(id));
            f.code == waiver.code && f.waived.is_none() && of_set
        }) {
            finding.waived = Some(waiver.reason.clone());
        }
    }
}

/// The doors of every set of the module in `folder`, read from the map files and type files its sources cook to.
pub fn doors_of(
    folder: &Path,
    model: &Maps,
    names: Names<'_>,
    patterns: &NamePatterns,
) -> Vec<SetDoors> {
    // ⚠ A pack holds no collision of its maps, so every arc here is held against the boxes of the other entities alone.
    let facts = inspect(folder, model, Depth::Stat);
    // The names the pack remembers come first: they are the person's own.
    let known = |hash: u32| {
        model
            .name_of(hash)
            .or_else(|| names.and_then(|names| names(hash)))
    };
    let nothing: CollisionSet<String> = CollisionSet::new();
    let mut out = Vec::new();
    for set in &model.maps {
        let mut documents: Vec<(&str, Role, MetaDocument)> = Vec::new();
        let mut unread: Vec<(String, String)> = Vec::new();
        for path in &set.sources {
            // ⚠ A source that is not there may be a map file, so it is said: its doors are doors missed.
            let source = match facts.get(path) {
                Some(Fact::Source(source)) => source,
                Some(Fact::Missing) => {
                    unread.push((path.clone(), "is not in the folder of its module".into()));
                    continue;
                }
                Some(Fact::Unreadable(why)) => {
                    unread.push((path.clone(), why.clone()));
                    continue;
                }
                Some(Fact::Escapes) | None => {
                    unread.push((path.clone(), "leaves the folder of its module".into()));
                    continue;
                }
            };
            let Some(role @ (Role::Map | Role::Types)) = source.role() else {
                continue;
            };
            let dir = folder.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let document = cooked(&dir).and_then(|bytes| {
                MetaDocument::parse(or_io::meta::default_library(), bytes)
                    .map_err(|e| format!("does not read as a metadata document: {e}"))
            });
            match document {
                Ok(document) => documents.push((path, role, document)),
                Err(why) => unread.push((path.clone(), why)),
            }
        }
        let mut maps: Vec<(&str, MapData<'_>)> = Vec::new();
        let mut types: Vec<MapTypes<'_>> = Vec::new();
        for (path, role, document) in &documents {
            let read = match role {
                Role::Map => MapData::new(&document.instance).map(|map| maps.push((path, map))),
                _ => MapTypes::new(&document.instance).map(|file| types.push(file)),
            };
            if read.is_none() {
                let what = "its root is not the one its extension says";
                unread.push((path.to_string(), what.to_string()));
            }
        }
        // ⚠ Of two type files of a set that declare one name the pack believes the first, as the scene, the rules and the extents an
        // operation grows do, and `find` believes the last it is handed: the type files go to it last first.
        types.reverse();
        let mut doors = survey(model, set, &maps, &types, &known, patterns, &nothing);
        doors.unread = unread;
        out.push(doors);
    }
    out
}

impl SetDoors {
    /// The set as lines of text: a line of counts, a line for each door and a line for each finding.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "set {}: {} door(s) among {} entity(ies), {} double door(s), {} entity(ies) whose model has no name",
            self.name,
            self.doors.len(),
            self.entities,
            self.pairs.len(),
            self.unnamed
        )];
        for (source, why) in &self.unread {
            lines.push(format!("  not read {source}: {why}"));
        }
        for door in &self.doors {
            let mut said = vec![match &door.pattern {
                Some(pattern) => format!("found by the name pattern `{pattern}`"),
                None => "found by its door extension".to_string(),
            }];
            match (&door.leaf, &door.unread) {
                (Some(leaf), _) => {
                    said.push(match leaf.hinge {
                        Some(end) => format!(
                            "read as {}, hinged at the {end} end of its {}, {:.2} m wide",
                            leaf.motion,
                            leaf.width_axis.to_ascii_uppercase(),
                            leaf.width
                        ),
                        None => format!("read as {}, {:.2} m wide", leaf.motion, leaf.width),
                    });
                }
                (None, Some(why)) => said.push(format!("not read: {why}")),
                (None, None) => {}
            }
            if let Some(other) = door.pair.and_then(|other| self.doors.get(other)) {
                said.push(format!("double with {}", other.id()));
            }
            for open in &door.opens {
                said.push(match &open.blocked_by {
                    Some(by) => {
                        format!("{} stops at {:.1} degrees on {by}", open.way, open.reaches)
                    }
                    None => format!("{} opens {:.1} degrees", open.way, open.reaches),
                });
            }
            lines.push(format!(
                "  door {} `{}` 0x{:08X} at {:.3}, {:.3}, {:.3} heading {:.1}: {}",
                door.id(),
                door.model,
                door.hash,
                door.position[0],
                door.position[1],
                door.position[2],
                door.heading,
                said.join("; ")
            ));
        }
        for finding in &self.findings {
            let word = match finding.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "note",
            };
            lines.push(match &finding.waived {
                Some(reason) => format!(
                    "  waived  {:<14} {} (waived: {reason})",
                    finding.code, finding.message
                ),
                None => format!("  {word:<7} {:<14} {}", finding.code, finding.message),
            });
        }
        lines
    }
}
