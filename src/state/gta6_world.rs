//! GTA VI world data kept for the Map screen's overlay: the lights of the map bodies read, the
//! lists of the lane graph, and the entity rows of every map body opened, which the plan draws
//! for many maps at once; and the [`Area`] a fill of a whole archive read, in grids the plan
//! draws from without a walk of every entity.
//!
//! ⚠ An interior's lights stand in the world only through an entity that places the interior: they
//! are drawn at each instance of it the map shown places, once a map types body read states them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use or_io::gta6::map::lanes::{self, Junction, LaneBody, Link, NodeAddress, PathNode};
use or_io::gta6::map::lights::{self, LightEffect, LightOwner, MapLights, Pose};
use or_io::gta6::map::world::{Grid, PlacementIndex, PlacementRow, WorldKind};
use or_io::gta6::map::{self, MapBody, MapData, MapTypes};
use or_io::gta6::names::Vocabulary;

use super::gta6_map::ScannedMap;

/// A light as the plan draws it: where it stands on the ground plane, its colour and its range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub at: [f32; 2],
    pub colour: [u8; 3],
    /// Its `falloff`, in metres.
    pub range: f32,
}

/// The lane graph as the plan draws it, from the lists read so far.
#[derive(Debug, Default)]
pub struct Lanes {
    nodes: Option<Vec<PathNode>>,
    links: Option<Vec<Link>>,
    junctions: Option<Vec<Junction>>,
    /// Each node that stands at a finite position.
    pub points: Vec<[f32; 2]>,
    /// Each pair of linked nodes once, known once the nodes and the links are read.
    pub segments: Vec<[[f32; 2]; 2]>,
    /// The node of each junction traffic lights run, known once the nodes and the junctions are.
    pub signals: Vec<[f32; 2]>,
}

fn ground(p: [f32; 3]) -> Option<[f32; 2]> {
    (p[0].is_finite() && p[1].is_finite()).then_some([p[0], p[1]])
}

impl Lanes {
    fn keep(&mut self, body: LaneBody) {
        match body {
            LaneBody::Nodes(n) => self.nodes = Some(n),
            LaneBody::Links(l) => self.links = Some(l),
            LaneBody::Junctions(j) => self.junctions = Some(j),
        }
        let nodes = self.nodes.as_deref().unwrap_or_default();
        self.points = nodes.iter().filter_map(|n| ground(n.position)).collect();
        // Of two nodes with one address the first is the one named, as the graph's join has it.
        let mut by_address: HashMap<NodeAddress, usize> = HashMap::with_capacity(nodes.len());
        let mut by_junction_address: HashMap<NodeAddress, usize> = HashMap::new();
        for (k, n) in nodes.iter().enumerate() {
            by_address.entry(n.address).or_insert(k);
            by_junction_address.entry(n.junction_address).or_insert(k);
        }
        let links = self.links.as_deref().unwrap_or_default();
        self.segments.clear();
        for (from, n) in nodes.iter().enumerate() {
            let end = nodes
                .get(from + 1)
                .map_or(links.len(), |n| n.first_link as usize);
            let held = links.get(n.first_link as usize..end).unwrap_or_default();
            for to in held.iter().filter_map(|l| by_address.get(&l.target)) {
                // A link and the link back are one segment, drawn from the lower node.
                let back = *to < from && links_to(nodes, links, *to, n.address);
                if let (false, Some(a), Some(b)) =
                    (back, ground(n.position), ground(nodes[*to].position))
                {
                    self.segments.push([a, b]);
                }
            }
        }
        self.signals = (self.junctions.iter().flatten())
            .filter(|j| j.is_signalised())
            .filter_map(|j| ground(nodes.get(*by_junction_address.get(&j.node)?)?.position))
            .collect();
    }
}

/// Whether node `from` holds a link to the node at `target`.
fn links_to(nodes: &[PathNode], links: &[Link], from: usize, target: NodeAddress) -> bool {
    let end = nodes
        .get(from + 1)
        .map_or(links.len(), |n| n.first_link as usize);
    (links
        .get(nodes[from].first_link as usize..end)
        .unwrap_or_default())
    .iter()
    .any(|l| l.target == target)
}

/// What a fill read of an archive: the rows of its maps, each archetype's box by its word, every
/// light in the world, and the lane graph as it is drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filled {
    pub index: PlacementIndex,
    pub boxes: HashMap<u64, [[f32; 3]; 2]>,
    pub lights: Vec<Light>,
    pub lane_points: Vec<[f32; 2]>,
    pub lane_segments: Vec<[[f32; 2]; 2]>,
    pub lane_signals: Vec<[f32; 2]>,
}

/// What a fill read of the archive at `archive`, in the grids the plan draws from.
#[derive(Debug)]
pub struct Area {
    pub archive: PathBuf,
    pub filled: Filled,
    /// The rows of the full level of detail, and those of the levels below it.
    pub full: Grid,
    pub lower: Grid,
    /// Every row in cells of 512 m and of 4096 m, for a view too far to draw an entity in.
    pub far: [Grid; 2],
    pub lights: Grid,
    pub lights_far: Grid,
    /// The lane graph's segments, each by its middle.
    pub segments: Grid,
}

/// Whether a row of level of detail `level` is of the full one: HD, or an HD with no parent.
fn is_full(level: u8) -> bool {
    matches!(level, 0 | 5)
}

impl Area {
    pub(super) fn new(archive: PathBuf, filled: Filled) -> Self {
        let at = |r: &PlacementRow| [r.position[0], r.position[1]];
        // A point that is not finite is in no cell and still counts in the order.
        let nowhere = [f32::NAN; 2];
        let rows = &filled.index.rows;
        let pick = |full: bool| {
            let points = rows.iter().map(|r| match is_full(r.lod_level) == full {
                true => at(r),
                false => nowhere,
            });
            Grid::new(if full { 64.0 } else { 256.0 }, points)
        };
        let (full, lower) = (pick(true), pick(false));
        let far = [512.0, 4096.0].map(|cell| Grid::new(cell, rows.iter().map(at)));
        let lights = Grid::new(64.0, filled.lights.iter().map(|l| l.at));
        let lights_far = Grid::new(1024.0, filled.lights.iter().map(|l| l.at));
        let middle = |s: &[[f32; 2]; 2]| [(s[0][0] + s[1][0]) / 2.0, (s[0][1] + s[1][1]) / 2.0];
        let segments = Grid::new(256.0, filled.lane_segments.iter().map(middle));
        Self {
            archive,
            filled,
            full,
            lower,
            far,
            lights,
            lights_far,
            segments,
        }
    }

    /// Whether the scanned body at `path` is a body of the archive this was read of.
    pub fn holds(&self, path: &Path) -> bool {
        path.parent() == Some(self.archive.as_path())
    }

    /// The row that stands nearest `at` on the ground plane, within `radius` metres of it.
    pub fn pick(&self, at: [f32; 2], radius: f32) -> Option<usize> {
        let lo = [at[0] - radius, at[1] - radius];
        let hi = [at[0] + radius, at[1] + radius];
        let rows = &self.filled.index.rows;
        [&self.full, &self.lower]
            .into_iter()
            .flat_map(|grid| grid.cells_within(lo, hi))
            .flat_map(|(_, held)| held.iter())
            .filter_map(|k| {
                let p = rows.get(*k as usize)?.position;
                let away = (p[0] - at[0]).hypot(p[1] - at[1]);
                (away <= radius).then_some((away, *k as usize))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, row)| row)
    }

    /// The map of the index that row `row` is a row of.
    pub fn map_of(&self, row: usize) -> Option<&or_io::gta6::map::world::IndexedMap> {
        let maps = &self.filled.index.maps;
        // The maps hold their rows in order, map after map.
        (maps.get(maps.partition_point(|m| m.rows.end <= row))).filter(|m| m.rows.contains(&row))
    }
}

/// The name of the archetype `row` places where the names kept of the bodies harvested hold it,
/// else its hash.
pub fn archetype_name(row: &PlacementRow) -> String {
    // ponytail: read once a session, so names harvested while the app runs show at its next start.
    static NAMES: LazyLock<Vocabulary> = LazyLock::new(|| {
        super::prefs::config_root().map_or_else(Vocabulary::default, |root| {
            Vocabulary::load(&or_io::gta6::names::vocabulary_file(&root))
        })
    });
    (NAMES.confirmed(row.archetype_name, row.archetype_word))
        .map_or_else(|| format!("{:#010X}", row.archetype_name), str::to_string)
}

/// What the app keeps of GTA VI's world data.
// ponytail: one store for the whole app, which no close empties and which holds one lane graph, the last read; key it by archive if two installs are open at once.
#[derive(Default)]
pub struct World {
    /// The lights of each map body opened, by its path.
    lights: BTreeMap<PathBuf, Arc<MapLights>>,
    /// The light effects of each archetype a map types body read holds, by the archetype's word.
    archetype_lights: HashMap<u64, Vec<LightEffect>>,
    /// The light effects inside each interior a map types body read holds, by the interior's
    /// word, each with the pose of the entity that holds it.
    interior_lights: HashMap<u64, Vec<(Pose, LightEffect)>>,
    pub lanes: Lanes,
    /// The rows of every map data body opened, and the path of each map of the index.
    placed: PlacementIndex,
    indexed: Vec<PathBuf>,
    /// Bumped whenever a light is kept, so the lights placed before are placed again.
    version: u64,
    /// The lights of the map last shown, with the map and the version they were placed for.
    shown: Option<(usize, u64, Arc<Vec<Light>>)>,
    /// What the last fill read, of the archive it read.
    pub area: Option<Arc<Area>>,
}

/// What is kept, locked: the plan draws from it and the readers add to it.
pub fn world() -> MutexGuard<'static, World> {
    static WORLD: LazyLock<Mutex<World>> = LazyLock::new(Mutex::default);
    WORLD.lock().unwrap_or_else(|p| p.into_inner())
}

/// Keep what the world readers read of the scanned body at `path`, after the map reader kept it.
pub(super) fn keep(path: &Path, virtual_section: &[u8], physical: &[u8]) {
    let Some(kind) = map::root_type(virtual_section).and_then(WorldKind::of_root) else {
        return;
    };
    match kind {
        WorldKind::MapData | WorldKind::MapTypes => {
            let body = super::gta6_map::opened_map(path).and_then(Result::ok);
            let found = lights::open(virtual_section, physical).ok();
            let mut world = world();
            match (body.as_deref(), &found) {
                (Some(MapBody::Data(m)), _) => world.index(path, m),
                (Some(MapBody::Types(t)), Some(found)) => world.keep_archetype_lights(t, found),
                _ => {}
            }
            if let Some(found) = found {
                world.lights.insert(path.to_path_buf(), Arc::new(found));
                world.version += 1;
            }
        }
        WorldKind::LaneNodes | WorldKind::LaneLinks | WorldKind::LaneJunctions => {
            if let Ok(body) = lanes::open(virtual_section, physical) {
                world().lanes.keep(body);
            }
        }
        WorldKind::EntityGroup => {}
    }
}

/// Keep the archetype lights of `types`, a map types body a search of a scan read from these two
/// sections.
// ponytail: the light reader reads the body's tables again after the map reader; hand it the model if a search is slow.
pub(super) fn keep_searched_types(types: &MapTypes, virtual_section: &[u8], physical: &[u8]) {
    if let Ok(found) = lights::open(virtual_section, physical) {
        if !found.effects.is_empty() {
            world().keep_archetype_lights(types, &found);
        }
    }
}

impl World {
    fn keep_archetype_lights(&mut self, types: &MapTypes, found: &MapLights) {
        for (owner, effect) in &found.effects {
            let (k, inside) = match owner {
                LightOwner::Archetype(k) => (*k, None),
                LightOwner::InteriorEntity {
                    archetype, pose, ..
                }
                | LightOwner::EntitySetEntity {
                    archetype, pose, ..
                } => (*archetype, Some(*pose)),
                LightOwner::Entity(_) => continue,
            };
            let Some(archetype) = types.archetypes.get(k) else {
                continue;
            };
            // A body read twice, or an archetype two bodies state, adds its effect once.
            let added = match inside {
                None => {
                    let held = self
                        .archetype_lights
                        .entry(archetype.name_word)
                        .or_default();
                    let new = !held.contains(effect);
                    if new {
                        held.push(effect.clone());
                    }
                    new
                }
                Some(pose) => {
                    let held = self.interior_lights.entry(archetype.name_word).or_default();
                    let new = !held.iter().any(|(p, e)| *p == pose && e == effect);
                    if new {
                        held.push((pose, effect.clone()));
                    }
                    new
                }
            };
            self.version += u64::from(added);
        }
    }

    // ponytail: a map is indexed as it first opened; index it again if an edit must move its points.
    fn index(&mut self, path: &Path, map: &MapData) {
        if map.entities.is_empty() || self.indexed.iter().any(|p| p == path) {
            return;
        }
        let offset = super::rpf9_scan::scanned_body(path).map_or(0, |(_, offset)| offset);
        self.placed.add(offset, map);
        self.indexed.push(path.to_path_buf());
    }

    /// How many maps the index holds, and how many entity rows.
    pub fn indexed(&self) -> (usize, usize) {
        (self.placed.maps.len(), self.placed.rows.len())
    }

    /// Where each entity of the maps indexed stands inside the rectangle `min` to `max` of the
    /// ground plane, the map at `except` left out: the plan draws that one as boxes.
    pub fn placements<'a>(
        &'a self,
        min: [f32; 2],
        max: [f32; 2],
        except: Option<&'a Path>,
    ) -> impl Iterator<Item = [f32; 2]> + 'a {
        (self.placed.maps_within(min, max))
            .filter(move |(i, _)| except.is_none_or(|p| self.indexed[*i] != p))
            .flat_map(move |(i, _)| self.placed.rows_of(i))
            .map(|r| [r.position[0], r.position[1]])
            .filter(move |p| (0..2).all(|i| p[i] >= min[i] && p[i] <= max[i]))
    }

    /// The lights of the map shown, in the world: each entity's own, those of the archetype it
    /// places and, for an entity that places an interior, those inside the interior, where a map
    /// types body read holds them.
    pub fn lights_of(&mut self, shown: &ScannedMap) -> Arc<Vec<Light>> {
        let key = (Arc::as_ptr(&shown.map) as usize, self.version);
        if let Some((map, version, lights)) = &self.shown {
            if (*map, *version) == key {
                return Arc::clone(lights);
            }
        }
        let placed = Arc::new(match shown.data() {
            Some(m) => self.place(&shown.path, m),
            None => Vec::new(),
        });
        self.shown = Some((key.0, key.1, Arc::clone(&placed)));
        placed
    }

    fn place(&self, path: &Path, m: &MapData) -> Vec<Light> {
        let mut out = Vec::new();
        let mut add = |effect: &LightEffect, pose: Pose| {
            for light in effect.lights() {
                let at = pose.place(effect, light).position;
                out.push(Light {
                    at: [at[0], at[1]],
                    colour: light.colour,
                    range: light.falloff,
                });
            }
        };
        for (owner, effect) in self
            .lights
            .get(path)
            .into_iter()
            .flat_map(|l| l.effects.iter())
        {
            if let LightOwner::Entity(k) = owner {
                if let Some(entity) = m.entities.get(*k) {
                    add(effect, Pose::of(entity));
                }
            }
        }
        for entity in &m.entities {
            for effect in (self.archetype_lights.get(&entity.archetype_word))
                .into_iter()
                .flatten()
            {
                add(effect, Pose::of(entity));
            }
            if !entity.interior {
                continue;
            }
            for (inside, effect) in (self.interior_lights.get(&entity.archetype_word))
                .into_iter()
                .flatten()
            {
                add(effect, inside.within(&Pose::of(entity)));
            }
        }
        out
    }
}

impl super::AppState {
    /// Read the world of the archive the map shown is a body of, on a worker: every map of it, or
    /// those whose streaming box meets `region` of the ground plane. The bodies are those the open
    /// scan of the archive lists once it has ended, else the archive is scanned for them.
    pub(crate) fn fill_gta6_world(&mut self, region: Option<fill::Region>) -> Result<(), String> {
        let archive = (self.scanned_map())
            .and_then(|s| super::rpf9_scan::scanned_body(&s.path))
            .map(|(archive, _)| archive.to_path_buf())
            .ok_or_else(|| {
                tr!("This map is no body of a scanned archive, so no archive holds the world around it.")
                    .to_string()
            })?;
        let offsets = self.documents.iter().find_map(|d| match &d.payload {
            super::DocPayload::Rpf9(scan) if scan.path == archive && scan.start == 0 => {
                let found = scan.found();
                let whole = found.ended == Some(super::rpf9_scan::Ended::Whole);
                whole.then(|| {
                    (found.rows.iter())
                        .filter_map(|row| match row {
                            super::rpf9_scan::Row::Body(find) if fill::starts_a_resource(find) => {
                                Some(find.offset)
                            }
                            _ => None,
                        })
                        .collect::<Vec<u64>>()
                })
            }
            _ => None,
        });
        fill::start(&archive, offsets, region);
        Ok(())
    }

    /// Open the map body at `offset` of the archive at `archive`, as its row of the scan opens it.
    pub(crate) fn open_gta6_map_body(&mut self, archive: &Path, offset: u64) {
        let view = super::rpf9_scan::View::Headerless;
        self.open_scanned_path(super::rpf9_scan::body_path(archive, offset, view), false);
    }
}

#[path = "gta6_world_fill.rs"]
pub(crate) mod fill;

#[path = "gta6_worldview/mod.rs"]
pub(crate) mod worldview;

#[cfg(test)]
#[path = "gta6_world_tests.rs"]
mod tests;
