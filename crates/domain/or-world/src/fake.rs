//! A title invented to prove the shared code with no game: a few maps held in memory, a parent
//! and its children at two levels of detail, a map by the clock, a map behind a switch, an
//! interior, a list, a light, an occluder, a cube with a flat texture, and an edit store that keeps
//! its edits in memory. It stands in no registry a person sees.
//!
//! ⚠ It counts: every read of rows, of a body, every layout and every asking of what draws a
//! body's nodes is counted in [`Counts`], so a test can say a held map was read once.
//!
//! A [`Fault`] makes it break one rule of the conformance kit and no other, which is how the kit
//! is shown to refuse by the rule's name.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use glam::Vec3;

use crate::caps::{Capability, WorldCaps};
use crate::collision::{GroundProbe, Hit, Part};
use crate::contract::{
    Address, Applied, Base, EditError, EditPlace, EditSession, EditTitle, Envelope, GameId, Guard,
    MapBody, MapReader, ModelReader, Opening, OperationId, OperationKind, Worker, WorldError,
    WorldIndexer, WorldInstall, WorldLook, WorldSpec, WorldTitle, Written,
};
use crate::kept::{Fnv, Input, Inputs, Section, SectionReader, Stamp};
use crate::kit::{
    EditFixture, Fixture, Grounded, HeldBack, InteriorAt, Ladder, ListAt, Lit, Scene, Timed,
};
use crate::model::{
    interior_key, rows_at, signed_fade, skipped_of, Alpha, AssetKey, Drawn, Draws, Element,
    ElementFormat, Feed, FeedName, FrameFacts, Gate, Index, Instance, Instances, InteriorPlaced,
    Light, LightKind, Lod, MapId, MapRow, Mesh, Model, Moment, OccluderBox, Occluders,
    PlacementRow, Semantic, Surface, SwitchId, Texture, TextureAsk, TextureFormat, VertexLayout,
    EVERY_HOUR,
};
use crate::policy::StreamPolicy;
use crate::sentence;
use crate::slots::{Choices, Extras, OptionId, OptionKind, Slot, SwitchKind};
use crate::streaming::{Category, Children, Extent, Kind, Level, MapBox, Node, ParentRef};

/// The game the fake title opens a world of: an id no profile registers.
pub const GAME: GameId = GameId("fake-world");
/// The version of the fake title's rules.
pub const RULES: u32 = 1;
/// The map that holds the parent of the ladder.
pub const FAR: u64 = 0xFA01;
/// The map that holds the parent's two children.
pub const NEAR: u64 = 0xFA02;
/// The map the clock lets in from 20 h to 6 h.
pub const NIGHT: u64 = 0xFA03;
/// The map only [`EVENT`] or a request lets in.
pub const HELD: u64 = 0xFA04;
/// The map that places the interior and holds the list, the light and the occluder.
pub const YARD: u64 = 0xFA05;
/// The switch that lets [`HELD`] in.
pub const EVENT: SwitchId = SwitchId(1);
/// The model the high level draws with.
pub const CUBE: AssetKey = AssetKey(1);
/// The model the parent of the ladder draws with.
pub const SLAB: AssetKey = AssetKey(2);
/// The fake title's one operation: move a node of a map.
pub const MOVE: OperationId = OperationId(1);
/// How many words of its own the fake title gives an instance: its level, then its place.
pub const WORDS: usize = 2;
/// The one weather the fake title has.
pub const WEATHER: u64 = 1;

/// Where the parent of the ladder draws alone.
pub const FAR_EYE: Vec3 = Vec3::new(0.0, 500.0, 0.0);
/// Where the parent's children draw in its place.
pub const NEAR_EYE: Vec3 = Vec3::new(0.0, 50.0, 0.0);
/// Where [`NIGHT`] streams when its hours let it in.
pub const NIGHT_EYE: Vec3 = Vec3::new(0.0, 5000.0, 0.0);
/// Where [`HELD`] streams when something asks for it.
pub const HELD_EYE: Vec3 = Vec3::new(0.0, 10_000.0, 0.0);
/// Where the interior, the list, the light and the occluder of [`YARD`] are in reach.
pub const YARD_EYE: Vec3 = Vec3::new(5000.0, 20.0, 0.0);
/// Where nothing streams.
pub const NOWHERE: Vec3 = Vec3::new(-50_000.0, -50_000.0, 0.0);
/// How many nodes the interior of [`YARD`] holds.
pub const INTERIOR_NODES: usize = 2;
/// How many instances the list of [`YARD`] lays out.
pub const LIST_INSTANCES: usize = 3;

const YARD_AT: Vec3 = Vec3::new(5000.0, 0.0, 0.0);
const INTERIOR: u64 = 0x77;

/// One rule of the conformance kit the fake title can be built to break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fault {
    /// Its link reads a gate from what the open that read the rows remembers, not from the sections.
    LinkForgets,
    /// The children's map names no parent map.
    ParentUnlinked,
    /// The parent of the ladder states one child more than it has.
    ChildrenMiscounted,
    /// The map by the clock streams at every hour.
    HoursIgnored,
    /// The map behind the switch streams with nothing asking for it.
    HeldStreams,
    /// The interior's body does not read.
    InteriorUnread,
    /// The list lays out one instance however many it holds.
    ListLaysOne,
    /// An instance carries one word fewer than the title declares.
    WordsShort,
    /// The body states no light and no occluder.
    LightsDropped,
    /// Its capabilities say it has no placement rows, and it answers them.
    CapsLie,
    /// The edit session answers no body after an apply.
    SessionHidesBody,
    /// The edit session applies an operation whatever its guard.
    GuardIgnored,
    /// An operation's inverse puts the node a metre from where it stood.
    InverseWrong,
    /// The look lists its one switch twice.
    SwitchTwice,
    /// Every map streams from everywhere.
    StreamsEverywhere,
    /// The last level fades out over a band of nought.
    BandOfNought,
    /// The cube's last triangle names a vertex it does not hold.
    IndexPastVertices,
    /// The model the parent of the ladder draws with does not read.
    ModelUnread,
    /// The sun's direction is twice a unit vector.
    SunTwiceAUnit,
    /// The flat texture the cube's surface asks for does not read.
    TextureUnread,
    /// The cube's texture coordinate is stated past the end of its vertex.
    ElementPastStride,
    /// The body of the children's map says what draws every node but its last.
    DrawsShort,
    /// The edit session makes the body of an edited map afresh each time it is asked.
    SessionBodyAfresh,
    /// Its capabilities state a base, and its edit session answers none.
    BaseStated,
    /// Its capabilities state a base and no edit store.
    StoreUnstated,
    /// Its capabilities state no height of the ground, and it answers one.
    GroundUnstated,
}

impl Fault {
    pub const ALL: [Self; 26] = [
        Self::LinkForgets,
        Self::ParentUnlinked,
        Self::ChildrenMiscounted,
        Self::HoursIgnored,
        Self::HeldStreams,
        Self::InteriorUnread,
        Self::ListLaysOne,
        Self::WordsShort,
        Self::LightsDropped,
        Self::CapsLie,
        Self::SessionHidesBody,
        Self::GuardIgnored,
        Self::InverseWrong,
        Self::SwitchTwice,
        Self::StreamsEverywhere,
        Self::BandOfNought,
        Self::IndexPastVertices,
        Self::ModelUnread,
        Self::SunTwiceAUnit,
        Self::TextureUnread,
        Self::ElementPastStride,
        Self::DrawsShort,
        Self::SessionBodyAfresh,
        Self::BaseStated,
        Self::StoreUnstated,
        Self::GroundUnstated,
    ];
}

/// What draws a node of the fake world.
#[derive(Debug, Clone, PartialEq)]
enum What {
    Model(AssetKey),
    /// A list: an instance at each offset from the box's centre.
    List(Vec<Vec3>),
    /// The instance of an interior: it has no model, and its nodes are a body of their own.
    Interior(u64),
}

#[derive(Debug, Clone, PartialEq)]
struct NodeDef {
    at: Vec3,
    lod_dist: f32,
    level: Level,
    /// Its parent, by the parent map's name and its place there.
    parent: Option<(u64, u32)>,
    num_children: u32,
    what: What,
}

#[derive(Debug, Clone, PartialEq)]
struct MapDef {
    name: u64,
    label: &'static str,
    /// The centre of its box and the box's half extent.
    centre: Vec3,
    half: f32,
    parent: Option<u64>,
    gate: Gate,
    nodes: Vec<NodeDef>,
    lights: Vec<Light>,
    occluders: Occluders,
}

#[derive(Debug, Clone, PartialEq)]
struct Archive {
    name: &'static str,
    stamp: Stamp,
    maps: Vec<MapDef>,
}

/// The fake world as its archives hold it.
#[derive(Debug, Clone, PartialEq)]
struct World {
    archives: Vec<Archive>,
    /// The nodes of the one interior, each by its offset from the instance.
    interior: Vec<NodeDef>,
}

fn entity(at: Vec3, lod_dist: f32, level: Level, model: AssetKey) -> NodeDef {
    NodeDef {
        at,
        lod_dist,
        level,
        parent: None,
        num_children: 0,
        what: What::Model(model),
    }
}

fn map(name: u64, label: &'static str, centre: Vec3, half: f32, nodes: Vec<NodeDef>) -> MapDef {
    MapDef {
        name,
        label,
        centre,
        half,
        parent: None,
        gate: Gate::Always,
        nodes,
        lights: Vec::new(),
        occluders: Occluders::default(),
    }
}

impl World {
    fn standard() -> Self {
        let far = map(
            FAR,
            "far",
            Vec3::ZERO,
            600.0,
            vec![NodeDef {
                num_children: 2,
                ..entity(Vec3::ZERO, 1000.0, Level::Lod, SLAB)
            }],
        );
        let child = |x: f32| NodeDef {
            parent: Some((FAR, 0)),
            ..entity(Vec3::new(x, 0.0, 0.0), 100.0, Level::Hd, CUBE)
        };
        let near = MapDef {
            parent: Some(FAR),
            ..map(
                NEAR,
                "near",
                Vec3::ZERO,
                200.0,
                vec![child(-10.0), child(10.0)],
            )
        };
        let night = MapDef {
            gate: Gate::When {
                hours: EVERY_HOUR & !(((1 << 20) - 1) & !((1 << 6) - 1)),
                weathers: Vec::new(),
            },
            ..map(
                NIGHT,
                "night",
                NIGHT_EYE,
                200.0,
                vec![entity(NIGHT_EYE, 100.0, Level::OrphanHd, CUBE)],
            )
        };
        let held = MapDef {
            gate: Gate::Held {
                switch: Some(EVENT),
            },
            ..map(
                HELD,
                "held",
                HELD_EYE,
                200.0,
                vec![entity(HELD_EYE, 100.0, Level::OrphanHd, CUBE)],
            )
        };
        let list = (0..LIST_INSTANCES).map(|k| Vec3::new(k as f32 * 4.0 - 4.0, 0.0, 0.0));
        let yard = MapDef {
            lights: vec![Light {
                kind: LightKind::Point,
                position: YARD_AT + Vec3::new(0.0, 0.0, 4.0),
                direction: Vec3::NEG_Z,
                colour: [1.0, 0.9, 0.8],
                intensity: 8.0,
                range: 12.0,
                fade: (60.0, 80.0),
                hours: EVERY_HOUR,
            }],
            occluders: Occluders {
                boxes: vec![OccluderBox {
                    rows: rows_at(YARD_AT + Vec3::new(0.0, -30.0, 0.0)),
                }],
                meshes: Vec::new(),
            },
            ..map(
                YARD,
                "yard",
                YARD_AT,
                300.0,
                vec![
                    NodeDef {
                        what: What::Interior(INTERIOR),
                        ..entity(YARD_AT, 150.0, Level::OrphanHd, CUBE)
                    },
                    NodeDef {
                        what: What::List(list.collect()),
                        ..entity(
                            YARD_AT + Vec3::new(50.0, 0.0, 0.0),
                            300.0,
                            Level::OrphanHd,
                            CUBE,
                        )
                    },
                ],
            )
        };
        Self {
            archives: vec![
                Archive {
                    name: "base",
                    stamp: Stamp {
                        size: 1,
                        modified: 1,
                    },
                    maps: vec![far, near],
                },
                Archive {
                    name: "extra",
                    stamp: Stamp {
                        size: 2,
                        modified: 1,
                    },
                    maps: vec![night, held, yard],
                },
            ],
            interior: vec![
                entity(Vec3::new(-3.0, 0.0, 0.0), 60.0, Level::OrphanHd, CUBE),
                entity(Vec3::new(3.0, 0.0, 0.0), 60.0, Level::OrphanHd, CUBE),
            ],
        }
    }

    fn map(&self, name: u64) -> Option<&MapDef> {
        (self.archives.iter().flat_map(|archive| &archive.maps)).find(|map| map.name == name)
    }
}

fn lock<T>(of: &Mutex<T>) -> MutexGuard<'_, T> {
    of.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the fake title was asked to read, counted.
#[derive(Debug, Default)]
pub struct Counts {
    rows: Mutex<HashMap<usize, usize>>,
    reads: Mutex<HashMap<u64, usize>>,
    lays: Mutex<HashMap<u64, usize>>,
    draws: Mutex<HashMap<u64, usize>>,
    interiors: Mutex<usize>,
    flushes: Mutex<usize>,
}

impl Counts {
    /// How many times the rows of the archive at `archive` were read.
    pub fn rows_of(&self, archive: usize) -> usize {
        lock(&self.rows).get(&archive).copied().unwrap_or(0)
    }

    /// How many times the body of the map named `name` was read.
    pub fn reads_of(&self, name: u64) -> usize {
        lock(&self.reads).get(&name).copied().unwrap_or(0)
    }

    /// How many times the body under the forest key `key` was laid out.
    pub fn lays_of(&self, key: u64) -> usize {
        lock(&self.lays).get(&key).copied().unwrap_or(0)
    }

    /// How many times the body under the forest key `key` was asked what draws its nodes.
    pub fn draws_of(&self, key: u64) -> usize {
        lock(&self.draws).get(&key).copied().unwrap_or(0)
    }

    /// How many times the body of an interior was asked for, read or refused.
    pub fn interior_reads(&self) -> usize {
        *lock(&self.interiors)
    }

    /// How many times an edit session was flushed.
    pub fn flushes(&self) -> usize {
        *lock(&self.flushes)
    }
}

/// What the fake title, its worlds and its edit sessions share.
#[derive(Debug)]
struct Shared {
    world: RwLock<World>,
    fault: Option<Fault>,
    counts: Counts,
}

impl Shared {
    fn has(&self, fault: Fault) -> bool {
        self.fault == Some(fault)
    }

    fn world(&self) -> std::sync::RwLockReadGuard<'_, World> {
        self.world.read().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The fake title.
#[derive(Debug, Clone)]
pub struct FakeTitle {
    shared: Arc<Shared>,
}

impl Default for FakeTitle {
    fn default() -> Self {
        Self::built(None)
    }
}

impl FakeTitle {
    /// The fake title as it passes the kit.
    pub fn new() -> Self {
        Self::default()
    }

    /// The fake title built to break the rule `fault` names.
    pub fn broken(fault: Fault) -> Self {
        Self::built(Some(fault))
    }

    fn built(fault: Option<Fault>) -> Self {
        Self {
            shared: Arc::new(Shared {
                world: RwLock::new(World::standard()),
                fault,
                counts: Counts::default(),
            }),
        }
    }

    /// The world of the fake title at `install`, which no folder need hold.
    pub fn spec() -> WorldSpec {
        WorldSpec::of(GAME, "fake-install")
    }

    pub fn counts(&self) -> &Counts {
        &self.shared.counts
    }

    /// Change the stamp of the archive at `archive`, as a file written again changes it.
    pub fn touch(&self, archive: usize) {
        let mut world = (self.shared.world.write()).unwrap_or_else(PoisonError::into_inner);
        if let Some(archive) = world.archives.get_mut(archive) {
            archive.stamp.modified += 1;
        }
    }

    /// List the archives in the opposite order, each as it stands.
    pub fn reverse_archives(&self) {
        let mut world = (self.shared.world.write()).unwrap_or_else(PoisonError::into_inner);
        world.archives.reverse();
    }

    /// The fake title's edit store.
    pub fn edits(&self) -> FakeEdits {
        FakeEdits {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl WorldTitle for FakeTitle {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn games(&self) -> &'static [GameId] {
        &[GAME]
    }

    fn options(&self) -> Vec<OptionKind> {
        vec![OptionKind {
            id: OptionId(1),
            label: sentence!("Mode"),
            choices: Choices::Toggle,
        }]
    }

    fn open(
        &self,
        world: &WorldSpec,
        _with: &Opening<'_>,
    ) -> Result<Arc<dyn WorldInstall>, WorldError> {
        if world.game != GAME {
            return Err(WorldError::NotItsGame(world.game.0));
        }
        if let Some((id, _)) = world.options.iter().find(|(id, _)| *id != OptionId(1)) {
            return Err(WorldError::Option(*id));
        }
        Ok(Arc::new(Install {
            shared: Arc::clone(&self.shared),
            remembered: Mutex::new(HashMap::new()),
        }))
    }
}

/// One opened fake world.
struct Install {
    shared: Arc<Shared>,
    /// The gates the rows read by this open stated, by map: what [`Fault::LinkForgets`] links from.
    remembered: Mutex<HashMap<u64, Gate>>,
}

/// What a link of the fake title leaves on the index: where each map stands in its archives.
struct Linked {
    places: Vec<(usize, usize)>,
}

fn linked(index: &Index) -> Result<&Linked, WorldError> {
    (index.title::<Linked>()).ok_or(WorldError::Lacks("index of the fake title's own linking"))
}

fn put_gate(out: &mut Section, gate: &Gate) {
    match gate {
        Gate::Always => out.put_u8(0),
        Gate::Held { switch } => {
            out.put_u8(1);
            out.put_u32(switch.map_or(0, |switch| switch.0));
        }
        Gate::When { hours, weathers } => {
            out.put_u8(2);
            out.put_u32(*hours);
            out.put_u32(weathers.len() as u32);
            for weather in weathers {
                out.put_u64(*weather);
            }
        }
    }
}

fn gate(from: &mut SectionReader<'_>) -> Result<Gate, WorldError> {
    Ok(match from.u8()? {
        0 => Gate::Always,
        1 => Gate::Held {
            switch: Some(from.u32()?)
                .filter(|switch| *switch != 0)
                .map(SwitchId),
        },
        2 => {
            let hours = from.u32()?;
            let mut weathers = Vec::new();
            for _ in 0..from.u32()? {
                weathers.push(from.u64()?);
            }
            Gate::When { hours, weathers }
        }
        other => {
            return Err(WorldError::Unread {
                what: "a fake row".into(),
                why: format!("its gate is of the kind {other}"),
            })
        }
    })
}

/// One row as a section holds it: the map's name, label, box, parent by name and gate.
struct Row {
    name: u64,
    label: String,
    centre: Vec3,
    half: f32,
    parent: Option<u64>,
    gate: Gate,
}

fn rows_of(section: &Section) -> Result<Vec<Row>, WorldError> {
    let mut from = section.reader();
    let mut out = Vec::new();
    for _ in 0..from.u32()? {
        let (name, label) = (from.u64()?, from.text()?.to_owned());
        let centre = Vec3::new(from.f32()?, from.f32()?, from.f32()?);
        let half = from.f32()?;
        let parent = Some(from.u64()?).filter(|parent| *parent != 0);
        out.push(Row {
            name,
            label,
            centre,
            half,
            parent,
            gate: gate(&mut from)?,
        });
    }
    from.end()?;
    Ok(out)
}

impl WorldIndexer for Install {
    fn rules(&self) -> u32 {
        RULES
    }

    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError> {
        let world = self.shared.world();
        let held = (world.archives.get(archive)).ok_or(WorldError::Lacks("such archive"))?;
        *lock(&self.shared.counts.rows).entry(archive).or_default() += 1;
        out.put_u32(held.maps.len() as u32);
        for map in &held.maps {
            out.put_u64(map.name);
            out.put_text(map.label);
            for value in map.centre.to_array() {
                out.put_f32(value);
            }
            out.put_f32(map.half);
            let unlinked = self.shared.has(Fault::ParentUnlinked);
            out.put_u64(map.parent.filter(|_| !unlinked).unwrap_or(0));
            let stated = match &map.gate {
                Gate::When { .. } if self.shared.has(Fault::HoursIgnored) => &Gate::Always,
                Gate::Held { .. } if self.shared.has(Fault::HeldStreams) => &Gate::Always,
                Gate::When { .. } if self.shared.has(Fault::LinkForgets) => {
                    lock(&self.remembered).insert(map.name, map.gate.clone());
                    &Gate::Always
                }
                gate => gate,
            };
            put_gate(out, stated);
        }
        Ok(())
    }

    fn whole(&self, archives: &[Section], out: &mut Section) -> Result<(), WorldError> {
        let mut maps = 0;
        for section in archives {
            maps += rows_of(section)?.len();
        }
        out.put_u32(maps as u32);
        Ok(())
    }

    fn link(&self, archives: &[Section], whole: &Section) -> Result<Index, WorldError> {
        let mut rows = Vec::new();
        for (archive, section) in archives.iter().enumerate() {
            for (at, row) in rows_of(section)?.into_iter().enumerate() {
                rows.push((archive, at, row));
            }
        }
        let mut stated = whole.reader();
        if stated.u32()? as usize != rows.len() {
            return Err(WorldError::Unread {
                what: "the fake index".into(),
                why: "what was derived across its archives counts other maps".into(),
            });
        }
        stated.end()?;
        let ids: HashMap<u64, MapId> = (0u32..)
            .zip(&rows)
            .map(|(id, (_, _, row))| (row.name, MapId(id)))
            .collect();
        let remembered = lock(&self.remembered);
        let maps = (rows.iter())
            .map(|(archive, _, row)| MapRow {
                name: row.name,
                label: row.label.clone(),
                boxes: Some(boxes(row.centre, row.half)),
                parents: (row.parent.iter())
                    .filter_map(|parent| ids.get(parent).copied())
                    .collect(),
                gate: (remembered.get(&row.name).cloned()).unwrap_or_else(|| row.gate.clone()),
                archive: *archive as u32,
            })
            .collect();
        let places = rows
            .iter()
            .map(|(archive, at, _)| (*archive, *at))
            .collect();
        Ok(Index::new(maps)?.with_title(Arc::new(Linked { places })))
    }

    fn placements(
        &self,
        index: &Index,
        map: MapId,
        out: &mut Vec<PlacementRow>,
    ) -> Result<(), WorldError> {
        let (archive, at) =
            *(linked(index)?.places.get(map.0 as usize)).ok_or(WorldError::NoSuchMap(map))?;
        let world = self.shared.world();
        let def = (world.archives.get(archive).and_then(|a| a.maps.get(at)))
            .ok_or(WorldError::NoSuchMap(map))?;
        out.extend((0u32..).zip(&def.nodes).map(|(node, def)| PlacementRow {
            node,
            archetype: match &def.what {
                What::Model(model) => model.0,
                What::List(_) => CUBE.0,
                What::Interior(interior) => *interior,
            },
            position: def.at,
        }));
        Ok(())
    }
}

fn boxes(centre: Vec3, half: f32) -> MapBox {
    let span = (centre - Vec3::splat(half), centre + Vec3::splat(half));
    MapBox {
        streaming: span,
        entities: span,
    }
}

/// A body of the fake world: a map's, or the interior's.
struct Body {
    key: u64,
    nodes: Vec<Node>,
    what: Vec<What>,
    lights: Vec<Light>,
    occluders: Occluders,
    interiors: Vec<InteriorPlaced>,
    shared: Arc<Shared>,
}

/// The half diagonal of the cube every model of the fake title is.
const RADIUS: f32 = 0.866_025_4;

impl Body {
    fn of(key: u64, defs: &[NodeDef], origin: Vec3, index: &Index, shared: &Arc<Shared>) -> Self {
        let node = |def: &NodeDef| Node {
            position: origin + def.at,
            extent: match &def.what {
                What::List(_) => {
                    let half = Vec3::new(20.0, 20.0, 1.0);
                    Extent::Box(origin + def.at - half, origin + def.at + half)
                }
                What::Model(_) | What::Interior(_) => Extent::Point,
            },
            lod_dist: def.lod_dist,
            child_lod_dist: 0.0,
            parent: def.parent.and_then(|(map, node)| {
                Some(ParentRef {
                    map: index.named(map)?.key(),
                    node,
                })
            }),
            instance: None,
            level: def.level,
            kind: Kind::Entity,
            category: Category::Other,
            radius: RADIUS,
            num_children: match shared.has(Fault::ChildrenMiscounted) {
                true if def.num_children > 0 => def.num_children + 1,
                _ => def.num_children,
            },
            batch: false,
        };
        Self {
            key,
            nodes: defs.iter().map(node).collect(),
            what: defs.iter().map(|def| def.what.clone()).collect(),
            lights: Vec::new(),
            occluders: Occluders::default(),
            interiors: (0u32..)
                .zip(defs)
                .filter_map(|(node, def)| match def.what {
                    What::Interior(interior) => Some(InteriorPlaced { node, interior }),
                    What::Model(_) | What::List(_) => None,
                })
                .collect(),
            shared: Arc::clone(shared),
        }
    }

    fn of_map(id: MapId, def: &MapDef, index: &Index, shared: &Arc<Shared>) -> Self {
        let dropped = shared.has(Fault::LightsDropped);
        Self {
            lights: def.lights.iter().filter(|_| !dropped).copied().collect(),
            occluders: match dropped {
                true => Occluders::default(),
                false => def.occluders.clone(),
            },
            ..Self::of(id.key(), &def.nodes, Vec3::ZERO, index, shared)
        }
    }

    fn push(&self, out: &mut Instances, node: u32, at: Vec3, fade: f32, level: Level) {
        let words = [level as u32, node];
        let short = usize::from(self.shared.has(Fault::WordsShort));
        out.push(Instance {
            node: (self.key, node),
            model: match self.what.get(node as usize) {
                Some(What::Model(model)) => *model,
                _ => CUBE,
            },
            rows: rows_at(at),
            sphere: [at.x, at.y, at.z, RADIUS],
            fade,
            words: &words[..WORDS - short],
        });
    }
}

impl MapBody for Body {
    fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    fn draws(&self, _at: &Moment, out: &mut Vec<Draws>) {
        *lock(&self.shared.counts.draws).entry(self.key).or_default() += 1;
        out.extend(self.what.iter().map(|what| match what {
            What::Model(model) => Draws::Model(*model),
            What::List(_) => Draws::StandIn,
            What::Interior(_) => Draws::Nothing,
        }));
        let of_children = self.nodes.last().is_some_and(|node| node.parent.is_some());
        if of_children && self.shared.has(Fault::DrawsShort) {
            out.pop();
        }
    }

    fn lights(&self) -> &[Light] {
        &self.lights
    }

    fn occluders(&self) -> &Occluders {
        &self.occluders
    }

    fn interiors(&self) -> &[InteriorPlaced] {
        &self.interiors
    }

    fn lay(&self, drawn: &Drawn<'_>, _at: &Moment, out: &mut Instances) {
        *lock(&self.shared.counts.lays).entry(self.key).or_default() += 1;
        for draw in drawn.nodes {
            if let Some(node) = self.nodes.get(draw.node.1 as usize) {
                let fade = signed_fade(draw.fade, draw.yields);
                self.push(out, draw.node.1, node.position, fade, node.level);
            }
        }
        for list in drawn.lists {
            let at = list.node.1;
            let (Some(node), Some(What::List(offsets))) =
                (self.nodes.get(at as usize), self.what.get(at as usize))
            else {
                continue;
            };
            let skipped = skipped_of(drawn, list);
            let most = match self.shared.has(Fault::ListLaysOne) {
                true => 1,
                false => offsets.len(),
            };
            for (instance, offset) in (0u32..).zip(offsets).take(most) {
                if !skipped.contains(&instance) {
                    let fade = signed_fade(list.fade, list.yields);
                    self.push(out, at, node.position + *offset, fade, node.level);
                }
            }
        }
    }

    fn bytes(&self) -> u64 {
        (self.nodes.len() * std::mem::size_of::<Node>()) as u64
    }
}

impl MapReader for Install {
    fn read(
        &self,
        index: &Index,
        map: MapId,
        _worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError> {
        let (archive, at) =
            *(linked(index)?.places.get(map.0 as usize)).ok_or(WorldError::NoSuchMap(map))?;
        let world = self.shared.world();
        let def = (world.archives.get(archive).and_then(|a| a.maps.get(at)))
            .ok_or(WorldError::NoSuchMap(map))?;
        *lock(&self.shared.counts.reads).entry(def.name).or_default() += 1;
        Ok(Arc::new(Body::of_map(map, def, index, &self.shared)))
    }

    fn interior(
        &self,
        index: &Index,
        map: MapId,
        body: &dyn MapBody,
        placed: &InteriorPlaced,
        _worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError> {
        *lock(&self.shared.counts.interiors) += 1;
        if placed.interior != INTERIOR || self.shared.has(Fault::InteriorUnread) {
            return Err(WorldError::Unread {
                what: format!("the interior {:#x}", placed.interior),
                why: "nothing of the fake world declares it".into(),
            });
        }
        let placing: &dyn Any = body;
        let instance = (placing.downcast_ref::<Body>())
            .and_then(|body| body.nodes.get(placed.node as usize))
            .ok_or(WorldError::Lacks(
                "such instance in a body of the fake title",
            ))?;
        let key = interior_key(map, placed.node);
        let world = self.shared.world();
        Ok(Arc::new(Body::of(
            key,
            &world.interior,
            instance.position,
            index,
            &self.shared,
        )))
    }
}

/// The unit cube about the origin: a position and a texture coordinate a vertex.
fn cube(shared: &Shared) -> Mesh {
    let mut vertices = Vec::new();
    for corner in 0..8u32 {
        let at = |bit: u32| {
            if corner & (1 << bit) != 0 {
                0.5f32
            } else {
                -0.5
            }
        };
        for value in [at(0), at(1), at(2), at(0) + 0.5, at(1) + 0.5] {
            vertices.extend(value.to_le_bytes());
        }
    }
    let mut indices = vec![
        0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6, 4,
        1, 5, 7, 1, 7, 3,
    ];
    if shared.has(Fault::IndexPastVertices) {
        if let Some(last) = indices.last_mut() {
            *last = 8;
        }
    }
    Mesh {
        layout: VertexLayout {
            stride: 20,
            elements: vec![
                Element {
                    semantic: Semantic::Position,
                    format: ElementFormat::F32x3,
                    offset: 0,
                },
                Element {
                    semantic: Semantic::TexCoord(0),
                    format: ElementFormat::F32x2,
                    offset: match shared.has(Fault::ElementPastStride) {
                        true => u32::MAX,
                        false => 12,
                    },
                },
            ],
        },
        vertices,
        indices,
        surface: Surface {
            colour: Some(FLAT),
            normal: None,
            alpha: Alpha::Opaque,
            tint: [1.0; 4],
        },
    }
}

/// The one texture of the fake title: four pixels by four of one grey.
const FLAT: TextureAsk = TextureAsk {
    scope: CUBE,
    name: 1,
    pixels: 4,
};

impl ModelReader for Install {
    fn models(
        &self,
        _index: &Index,
        asks: &[AssetKey],
        _worker: &mut Worker,
        out: &mut Vec<(AssetKey, Model)>,
    ) {
        let unread = self.shared.has(Fault::ModelUnread);
        let reads = |ask: &&AssetKey| **ask == CUBE || (**ask == SLAB && !unread);
        for ask in asks.iter().filter(reads) {
            let lod = Lod {
                distance: 1000.0,
                meshes: vec![cube(&self.shared)],
            };
            out.push((
                *ask,
                Model {
                    lods: vec![lod],
                    title: None,
                },
            ));
        }
    }

    fn textures(
        &self,
        _index: &Index,
        asks: &[TextureAsk],
        _worker: &mut Worker,
        out: &mut Vec<(TextureAsk, Texture)>,
    ) {
        let unread = self.shared.has(Fault::TextureUnread);
        let reads = |ask: &&TextureAsk| ask.scope == FLAT.scope && ask.name == FLAT.name && !unread;
        for ask in asks.iter().filter(reads) {
            out.push((
                *ask,
                Texture {
                    width: 4,
                    height: 4,
                    format: TextureFormat::Rgba8,
                    srgb: true,
                    levels: vec![[128, 128, 128, 255].repeat(16)],
                },
            ));
        }
    }
}

impl WorldLook for Install {
    fn switches(&self, _index: &Index) -> Vec<SwitchKind> {
        let event = SwitchKind {
            id: EVENT,
            label: sentence!("Mode"),
            on: false,
        };
        match self.shared.has(Fault::SwitchTwice) {
            true => vec![event, event],
            false => vec![event],
        }
    }

    fn frame(&self, at: &Moment, out: &mut dyn Feed) -> FrameFacts {
        // The sun stands overhead at noon and at the horizon at 6 h and 18 h.
        let turn = (at.hour - 12.0) / 24.0 * std::f32::consts::TAU;
        let day = turn.cos().max(0.0);
        out.set(FeedName::Name("fake_hour"), &[at.hour]);
        let long = match self.shared.has(Fault::SunTwiceAUnit) {
            true => 2.0,
            false => 1.0,
        };
        FrameFacts {
            sun: Vec3::new(turn.sin(), 0.0, turn.cos()) * long,
            key: [day; 3],
            ambient: [0.05 + 0.25 * day; 3],
            exposure: 1.0,
            clear: [0.1 * day, 0.2 * day, 0.4 * day],
        }
    }
}

impl WorldInstall for Install {
    fn caps(&self) -> WorldCaps {
        let has = |fault| self.shared.has(fault);
        let mut caps = WorldCaps::none().with_instance_words(WORDS);
        if !has(Fault::StoreUnstated) {
            caps = caps.with(Capability::Edits);
        }
        if has(Fault::BaseStated) || has(Fault::StoreUnstated) {
            caps = caps.with(Capability::Base);
        }
        if !has(Fault::GroundUnstated) {
            caps = caps.with(Capability::Ground);
        }
        match has(Fault::CapsLie) {
            true => caps,
            false => caps.with(Capability::Placements),
        }
    }

    // The fake world's ground is level, at the height of nought everywhere.
    fn ground(
        &self,
        _index: &Index,
        at: Vec3,
        probe: &GroundProbe,
        _worker: &mut Worker,
    ) -> Result<Option<Hit>, WorldError> {
        let within = -at.z <= probe.max_rise && at.z <= probe.max_drop;
        Ok(within.then_some(Hit {
            distance: at.z.abs(),
            position: Vec3::new(at.x, at.y, 0.0),
            normal: Vec3::Z,
            material: None,
            part: Part {
                leaf: 0,
                polygon: None,
            },
        }))
    }

    fn policy(&self) -> StreamPolicy {
        let mut policy = StreamPolicy {
            children: Children::Stated,
            ..StreamPolicy::default()
        };
        if self.shared.has(Fault::StreamsEverywhere) {
            policy.streams_in = |_, _| true;
        }
        if self.shared.has(Fault::BandOfNought) {
            policy.fade[Level::Slod4 as usize] = 0.0;
        }
        policy
    }

    fn extras(&self) -> Extras {
        Extras {
            layers: vec![Slot {
                id: 1,
                label: sentence!("Lights"),
            }],
            rules: vec![Slot {
                id: 1,
                label: sentence!("Collision"),
            }],
            sections: vec![Slot {
                id: 1,
                label: sentence!("Streaming"),
            }],
            streamed: Vec::new(),
        }
    }

    fn inputs(&self) -> Inputs {
        let world = self.shared.world();
        Inputs {
            archives: (world.archives.iter())
                .map(|archive| Input {
                    name: archive.name.to_owned(),
                    stamp: Some(archive.stamp),
                })
                .collect(),
            others: Vec::new(),
        }
    }

    fn indexer(&self) -> &dyn WorldIndexer {
        self
    }

    fn maps(&self) -> &dyn MapReader {
        self
    }

    fn models(&self) -> &dyn ModelReader {
        self
    }

    fn look(&self) -> &dyn WorldLook {
        self
    }
}

/// The fake title's edit store: every edit is held in memory, and a flush puts it in the fake world.
#[derive(Debug, Clone)]
pub struct FakeEdits {
    shared: Arc<Shared>,
}

impl EditTitle for FakeEdits {
    fn operations(&self) -> Vec<OperationKind> {
        vec![OperationKind {
            id: MOVE,
            label: sentence!("Move"),
        }]
    }

    fn open<'w>(&self, place: &EditPlace<'_, 'w>) -> Result<Box<dyn EditSession + 'w>, EditError> {
        linked(place.index)?;
        Ok(Box::new(Session {
            shared: Arc::clone(&self.shared),
            index: Arc::clone(place.index),
            edited: HashMap::new(),
        }))
    }
}

/// The operation that moves node `node` of the map named `map` to `to`, guarded by how `session` holds the map now.
pub fn move_node(session: &dyn EditSession, map: u64, node: u32, to: Vec3) -> Envelope {
    let at = Address(format!("{map:#x}"));
    let guard = session.guard(&at);
    let mut body = Vec::with_capacity(24);
    body.extend(map.to_le_bytes());
    body.extend(node.to_le_bytes());
    for value in to.to_array() {
        body.extend(value.to_le_bytes());
    }
    Envelope {
        kind: MOVE,
        at: vec![(at, guard)],
        body: body.into_boxed_slice(),
    }
}

struct Session {
    shared: Arc<Shared>,
    index: Arc<Index>,
    /// Each edited map's nodes as the session holds them, and the body made of them.
    edited: HashMap<MapId, (Vec<NodeDef>, Arc<dyn MapBody>)>,
}

impl Session {
    /// The map `at` names, and its nodes as the session holds them.
    fn held(&self, at: &Address) -> Option<(MapId, Vec<NodeDef>)> {
        let name = u64::from_str_radix(at.0.strip_prefix("0x")?, 16).ok()?;
        let id = self.index.named(name)?;
        match self.edited.get(&id) {
            Some((nodes, _)) => Some((id, nodes.clone())),
            None => Some((id, self.shared.world().map(name)?.nodes.clone())),
        }
    }
}

impl Session {
    /// A body of the map `id` with `nodes` in place of those the fake world holds of it.
    fn body_of(&self, id: MapId, nodes: &[NodeDef]) -> Option<Arc<dyn MapBody>> {
        let name = self.index.map(id)?.name;
        let world = self.shared.world();
        let edited = MapDef {
            nodes: nodes.to_vec(),
            ..world.map(name)?.clone()
        };
        Some(Arc::new(Body::of_map(
            id,
            &edited,
            &self.index,
            &self.shared,
        )))
    }
}

fn digest(nodes: &[NodeDef]) -> Guard {
    let mut hash = Fnv::default();
    for node in nodes {
        for value in node.at.to_array() {
            hash.feed(&value.to_le_bytes());
        }
    }
    Guard::Written(Box::new(hash.value().to_le_bytes()))
}

impl EditSession for Session {
    fn guard(&self, at: &Address) -> Guard {
        match self.held(at) {
            Some((_, nodes)) => digest(&nodes),
            None => Guard::Unwritten(0),
        }
    }

    fn apply(&mut self, op: &Envelope) -> Result<Applied, EditError> {
        if op.kind != MOVE {
            return Err(EditError::NoSuchOperation(op.kind));
        }
        let (Some((name, rest)), [(at, guard)]) = (op.body.split_first_chunk::<8>(), &op.at[..])
        else {
            return Err(EditError::Malformed(
                "a move is a map, a node and a place".into(),
            ));
        };
        let (Some((node, rest)), name) = (rest.split_first_chunk::<4>(), u64::from_le_bytes(*name))
        else {
            return Err(EditError::Malformed("a move names a node".into()));
        };
        let Ok(to) = <[u8; 12]>::try_from(rest) else {
            return Err(EditError::Malformed("a move ends on three numbers".into()));
        };
        let number = |k: usize| f32::from_le_bytes([to[k], to[k + 1], to[k + 2], to[k + 3]]);
        let (node, to) = (
            u32::from_le_bytes(*node) as usize,
            Vec3::new(number(0), number(4), number(8)),
        );
        let (id, mut nodes) =
            (self.held(at)).ok_or_else(|| EditError::NoSuchAddress(at.clone()))?;
        if *at != Address(format!("{name:#x}")) {
            return Err(EditError::Malformed(
                "a move guards the map it moves in".into(),
            ));
        }
        if !self.shared.has(Fault::GuardIgnored) && digest(&nodes) != *guard {
            return Err(EditError::Stale(at.clone()));
        }
        let Some(moved) = nodes.get_mut(node) else {
            return Err(EditError::Malformed(
                "a move names a node the map holds".into(),
            ));
        };
        let mut back = std::mem::replace(&mut moved.at, to);
        if self.shared.has(Fault::InverseWrong) {
            back.x += 1.0;
        }
        let body =
            (self.body_of(id, &nodes)).ok_or_else(|| EditError::NoSuchAddress(at.clone()))?;
        self.edited.insert(id, (nodes, body));
        Ok(Applied {
            inverse: move_node(self, name, node as u32, back),
            maps: vec![id],
        })
    }

    fn body(&self, map: MapId) -> Option<Arc<dyn MapBody>> {
        if self.shared.has(Fault::SessionHidesBody) {
            return None;
        }
        let (nodes, body) = self.edited.get(&map)?;
        match self.shared.has(Fault::SessionBodyAfresh) {
            true => self.body_of(map, nodes),
            false => Some(Arc::clone(body)),
        }
    }

    fn flush(&mut self) -> Result<Vec<Written>, EditError> {
        *lock(&self.shared.counts.flushes) += 1;
        let mut world = (self.shared.world.write()).unwrap_or_else(PoisonError::into_inner);
        let mut written = Vec::new();
        let mut edited: Vec<_> = self.edited.drain().collect();
        edited.sort_by_key(|(id, _)| *id);
        for (id, (nodes, _)) in edited {
            let Some(name) = self.index.map(id).map(|row| row.name) else {
                continue;
            };
            let maps = world
                .archives
                .iter_mut()
                .flat_map(|archive| &mut archive.maps);
            for map in maps.filter(|map| map.name == name) {
                written.push(Written {
                    at: Address(format!("{name:#x}")),
                    bytes: (nodes.len() * 12) as u64,
                });
                map.nodes.clone_from(&nodes);
            }
        }
        Ok(written)
    }

    fn base(&self, _at: &Address) -> Option<Base> {
        None
    }
}

/// Where the fixture's operation moves the first child of the ladder.
pub const MOVED_TO: Vec3 = Vec3::new(-10.0, 30.0, 0.0);

/// What the conformance kit checks `title` over: the fake world, as the kit's scene states it.
pub fn fixture(title: FakeTitle) -> Fixture {
    let edit = EditFixture {
        title: Box::new(title.edits()),
        folder: None,
        map: NEAR,
        node: 0,
        to: MOVED_TO,
        eye: NEAR_EYE,
        op: Box::new(|session| move_node(session, NEAR, 0, MOVED_TO)),
    };
    Fixture {
        title: Box::new(title),
        world: FakeTitle::spec(),
        scene: Scene {
            weather: WEATHER,
            nowhere: NOWHERE,
            ladder: Ladder {
                parent_map: FAR,
                child_map: NEAR,
                far: FAR_EYE,
                near: NEAR_EYE,
            },
            timed: Timed {
                map: NIGHT,
                eye: NIGHT_EYE,
                on: 22.0,
                off: 12.0,
            },
            held: HeldBack {
                map: HELD,
                eye: HELD_EYE,
                switch: EVENT,
            },
            interior: InteriorAt {
                eye: YARD_EYE,
                instances: INTERIOR_NODES,
            },
            list: Some(ListAt {
                eye: YARD_EYE,
                instances: LIST_INSTANCES,
            }),
            lit: Lit {
                eye: YARD_EYE,
                lights: 1,
                occluders: 1,
            },
            placed: (YARD, 2),
            ground: Grounded {
                at: Vec3::new(0.0, 0.0, 5.0),
                height: 0.0,
            },
        },
        edit: Some(edit),
        keeps: None,
        executable: None,
    }
}
