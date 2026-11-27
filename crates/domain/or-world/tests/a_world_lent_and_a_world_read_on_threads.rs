//! Two worlds the fake title is not: one whose install borrows the world it reads, streamed and
//! selected on the caller's thread, and one that owns its world and whose bodies are read on
//! worker threads. Both stand behind the contract with no unsafe code and no copy of the world.

use std::ops::Deref;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::ThreadId;

use glam::Vec3;
use or_world::caps::WorldCaps;
use or_world::contract::{
    GameId, MapBody, MapReader, ModelReader, Worker, WorldError, WorldIndexer, WorldInstall,
    WorldLook, WorldSpec,
};
use or_world::driver::{Driver, InteriorAt};
use or_world::kept::{self, Input, Inputs, Section, Stamp};
use or_world::model::{
    rows_at, signed_fade, AssetKey, Drawn, Draws, Feed, FrameFacts, Gate, Index, Instance,
    Instances, InteriorPlaced, Light, MapId, MapRow, Model, Moment, Occluders, PlacementRow,
    Switched, Texture, TextureAsk,
};
use or_world::policy::StreamPolicy;
use or_world::slots::{Extras, SwitchKind};
use or_world::streaming::{Category, Extent, Kind, Level, MapBox, Node, NodeState};

/// A world as its owner holds it.
// ⚠ Not `Clone`, on purpose: a test that compiles made no copy of it.
struct Their {
    maps: Vec<TheirMap>,
}

struct TheirMap {
    name: u64,
    centre: Vec3,
    /// Where each of its nodes stands, from its centre.
    nodes: Vec<Vec3>,
    /// The node that places an interior of two nodes, where one does.
    interior: Option<u32>,
}

const MODEL: AssetKey = AssetKey(7);
/// The half extent of every map's box, in metres.
const HALF: f32 = 200.0;
/// How many nodes each map holds.
const NODES: usize = 4;
/// Where every map of [`their`] streams from.
const EYE: Vec3 = Vec3::new(150.0, 0.0, 0.0);

/// Three maps whose boxes all hold [`EYE`], the first placing an interior at its first node.
fn their() -> Their {
    let map = |at: usize| TheirMap {
        name: 0xA0 + at as u64,
        centre: Vec3::new(at as f32 * 150.0, 0.0, 0.0),
        nodes: (0..NODES)
            .map(|node| Vec3::new(0.0, node as f32 * 5.0, 0.0))
            .collect(),
        interior: (at == 0).then_some(0),
    };
    Their {
        maps: (0..3).map(map).collect(),
    }
}

fn node(position: Vec3) -> Node {
    Node {
        position,
        extent: Extent::Point,
        lod_dist: 400.0,
        child_lod_dist: 0.0,
        parent: None,
        instance: None,
        level: Level::OrphanHd,
        kind: Kind::Entity,
        category: Category::Other,
        radius: 1.0,
        num_children: 0,
        batch: false,
    }
}

struct Body {
    nodes: Vec<Node>,
    interiors: Vec<InteriorPlaced>,
    occluders: Occluders,
}

impl MapBody for Body {
    fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    fn draws(&self, _at: &Moment, out: &mut Vec<Draws>) {
        let places = |node: u32| self.interiors.iter().any(|placed| placed.node == node);
        out.extend((0..self.nodes.len() as u32).map(|node| match places(node) {
            true => Draws::Nothing,
            false => Draws::Model(MODEL),
        }));
    }

    fn lights(&self) -> &[Light] {
        &[]
    }

    fn occluders(&self) -> &Occluders {
        &self.occluders
    }

    fn interiors(&self) -> &[InteriorPlaced] {
        &self.interiors
    }

    fn lay(&self, drawn: &Drawn<'_>, _at: &Moment, out: &mut Instances) {
        for draw in drawn.nodes {
            let Some(node) = self.nodes.get(draw.node.1 as usize) else {
                continue;
            };
            let at = node.position;
            out.push(Instance {
                node: draw.node,
                model: MODEL,
                rows: rows_at(at),
                sphere: [at.x, at.y, at.z, node.radius],
                fade: signed_fade(draw.fade, draw.yields),
                words: &[],
            });
        }
    }

    fn bytes(&self) -> u64 {
        std::mem::size_of_val(&self.nodes[..]) as u64
    }
}

/// What reads a world it is lent or owns: `W` is `&Their` or `Box<Their>`.
struct World<W> {
    their: W,
    /// The thread each body was read on.
    read_on: Mutex<Vec<ThreadId>>,
}

impl<W> World<W> {
    fn over(their: W) -> Self {
        Self {
            their,
            read_on: Mutex::default(),
        }
    }

    fn read_here(&self) {
        let mut read_on = self.read_on.lock().unwrap_or_else(PoisonError::into_inner);
        read_on.push(std::thread::current().id());
    }
}

impl<W: Deref<Target = Their> + Send + Sync> WorldIndexer for World<W> {
    fn rules(&self) -> u32 {
        1
    }

    fn rows(&self, _archive: usize, out: &mut Section) -> Result<(), WorldError> {
        out.put_u32(self.their.maps.len() as u32);
        for map in &self.their.maps {
            out.put_u64(map.name);
            for value in map.centre.to_array() {
                out.put_f32(value);
            }
        }
        Ok(())
    }

    fn whole(&self, _archives: &[Section], _out: &mut Section) -> Result<(), WorldError> {
        Ok(())
    }

    fn link(&self, archives: &[Section], _whole: &Section) -> Result<Index, WorldError> {
        let mut rows = Vec::new();
        for section in archives {
            let mut from = section.reader();
            for _ in 0..from.u32()? {
                let name = from.u64()?;
                let centre = Vec3::new(from.f32()?, from.f32()?, from.f32()?);
                let span = (centre - Vec3::splat(HALF), centre + Vec3::splat(HALF));
                rows.push(MapRow {
                    name,
                    label: format!("{name:#x}"),
                    boxes: Some(MapBox {
                        streaming: span,
                        entities: span,
                    }),
                    parents: Vec::new(),
                    gate: Gate::Always,
                    archive: 0,
                });
            }
            from.end()?;
        }
        Ok(Index::new(rows)?)
    }

    fn placements(
        &self,
        _index: &Index,
        _map: MapId,
        _out: &mut Vec<PlacementRow>,
    ) -> Result<(), WorldError> {
        Err(WorldError::Lacks("placement rows"))
    }
}

impl<W: Deref<Target = Their> + Send + Sync> MapReader for World<W> {
    fn read(
        &self,
        index: &Index,
        map: MapId,
        worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError> {
        self.read_here();
        *worker.state::<usize>() += 1;
        let row = index.map(map).ok_or(WorldError::NoSuchMap(map))?;
        let their = (self.their.maps.iter()).find(|their| their.name == row.name);
        let their = their.ok_or(WorldError::NoSuchMap(map))?;
        Ok(Arc::new(Body {
            nodes: (their.nodes.iter())
                .map(|at| node(their.centre + *at))
                .collect(),
            interiors: (their.interior.iter())
                .map(|node| InteriorPlaced {
                    node: *node,
                    interior: 1,
                })
                .collect(),
            occluders: Occluders::default(),
        }))
    }

    fn interior(
        &self,
        _index: &Index,
        _map: MapId,
        body: &dyn MapBody,
        placed: &InteriorPlaced,
        worker: &mut Worker,
    ) -> Result<Arc<dyn MapBody>, WorldError> {
        self.read_here();
        *worker.state::<usize>() += 1;
        let instance = (body.nodes().get(placed.node as usize))
            .ok_or(WorldError::Lacks("such instance in the body"))?;
        let beside = |x: f32| node(instance.position + Vec3::new(x, 0.0, 2.0));
        Ok(Arc::new(Body {
            nodes: vec![beside(-1.0), beside(1.0)],
            interiors: Vec::new(),
            occluders: Occluders::default(),
        }))
    }
}

impl<W: Deref<Target = Their> + Send + Sync> ModelReader for World<W> {
    fn models(
        &self,
        _index: &Index,
        _asks: &[AssetKey],
        _worker: &mut Worker,
        _out: &mut Vec<(AssetKey, Model)>,
    ) {
    }

    fn textures(
        &self,
        _index: &Index,
        _asks: &[TextureAsk],
        _worker: &mut Worker,
        _out: &mut Vec<(TextureAsk, Texture)>,
    ) {
    }
}

impl<W: Deref<Target = Their> + Send + Sync> WorldLook for World<W> {
    fn switches(&self) -> Vec<SwitchKind> {
        Vec::new()
    }

    fn frame(&self, _at: &Moment, _out: &mut dyn Feed) -> FrameFacts {
        FrameFacts {
            sun: Vec3::Z,
            key: [1.0; 3],
            ambient: [0.1; 3],
            exposure: 1.0,
            clear: [0.0; 3],
        }
    }
}

impl<W: Deref<Target = Their> + Send + Sync> WorldInstall for World<W> {
    fn caps(&self) -> WorldCaps {
        WorldCaps::none()
    }

    fn policy(&self) -> StreamPolicy {
        StreamPolicy::default()
    }

    fn extras(&self) -> Extras {
        Extras::default()
    }

    fn inputs(&self) -> Inputs {
        Inputs {
            archives: vec![Input {
                name: "all".into(),
                stamp: Some(Stamp {
                    size: self.their.maps.len() as u64,
                    modified: 1,
                }),
            }],
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

/// The index of `install`, built through the one kept-file mechanism in a folder of its own.
fn index_of(install: &dyn WorldInstall) -> Arc<Index> {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let world = WorldSpec::of(GameId("lent"), "nowhere");
    let file = kept::file_for(folder.path(), "lent", &world);
    let (index, _) = kept::refresh("lent", &world, install, &file).expect("the index builds");
    Arc::new(index)
}

fn ready(_: AssetKey) -> NodeState {
    NodeState::Ready
}

/// Every node of the three maps but the interior's instance, and the interior's two.
const DRAWN: usize = 3 * NODES - 1 + 2;

#[test]
fn a_world_that_is_lent_is_streamed_and_selected_with_no_copy_of_it() {
    let their = their();
    {
        let install: Arc<dyn WorldInstall + '_> = Arc::new(World::over(&their));
        let index = index_of(&*install);
        let mut driver = Driver::new(Arc::clone(&install), index);
        let view = (driver.policy()).view(EYE, Vec3::ZERO, std::f32::consts::FRAC_PI_4);
        let (noon, none) = (Moment::noon(0), Switched::default());
        let streamed = driver.stream(&view, &noon, &none, None, usize::MAX);
        assert_eq!((streamed.wanted.len(), streamed.interiors.len()), (3, 1));
        let selected = driver.select(&view, &noon, &ready);
        assert_eq!(selected.instances.len(), DRAWN);
    }
    // Lent throughout, and its owner's still.
    let Their { maps } = their;
    assert_eq!(maps.len(), 3);
}

enum Ask {
    Map(MapId),
    Interior(InteriorAt, Arc<dyn MapBody>),
}

type Read = Result<Arc<dyn MapBody>, WorldError>;

enum Answer {
    Map(MapId, Read),
    Interior(InteriorAt, Arc<dyn MapBody>, Read),
}

#[test]
fn a_world_that_is_owned_is_read_on_worker_threads_and_never_on_the_drivers() {
    let world = Arc::new(World::over(Box::new(their())));
    let install: Arc<dyn WorldInstall> = world.clone();
    let index = index_of(&*install);
    let mut driver = Driver::new(Arc::clone(&install), Arc::clone(&index));
    let view = (driver.policy()).view(EYE, Vec3::ZERO, std::f32::consts::FRAC_PI_4);
    let (noon, none) = (Moment::noon(0), Switched::default());

    let (ask, asked) = mpsc::channel::<Ask>();
    let asked = Arc::new(Mutex::new(asked));
    let (answer, answers) = mpsc::channel::<Answer>();
    let workers: Vec<_> = (0..3)
        .map(|_| {
            let (install, index) = (Arc::clone(&install), Arc::clone(&index));
            let (asked, answer) = (Arc::clone(&asked), answer.clone());
            std::thread::spawn(move || {
                // What a worker keeps between its reads is its own: here, how many it made.
                let mut worker = Worker::new();
                loop {
                    let next = asked.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    let read = match next {
                        Ok(Ask::Map(map)) => {
                            Answer::Map(map, install.maps().read(&index, map, &mut worker))
                        }
                        Ok(Ask::Interior(at, placing)) => {
                            let maps = install.maps();
                            let body =
                                maps.interior(&index, at.map, &*placing, &at.placed, &mut worker);
                            Answer::Interior(at, placing, body)
                        }
                        Err(_) => return *worker.state::<usize>(),
                    };
                    if answer.send(read).is_err() {
                        return *worker.state::<usize>();
                    }
                }
            })
        })
        .collect();

    let wanted = driver.want(&view, &noon, &none, None);
    assert_eq!(wanted.len(), 3);
    for map in &wanted {
        ask.send(Ask::Map(*map)).expect("a worker listens");
    }
    for _ in &wanted {
        match answers.recv().expect("a worker answers") {
            Answer::Map(map, body) => driver.give(map, body),
            Answer::Interior(..) => panic!("no interior was asked for yet"),
        }
    }
    let streamed = driver.settle(&view);
    assert_eq!(streamed.wanted.len(), 3);
    assert_eq!(
        key_of(&streamed.interiors_waiting),
        key_of(&[first_interior(&index)])
    );
    assert!(streamed.interiors.is_empty());

    for at in &streamed.interiors_waiting {
        let placing = Arc::clone(driver.placing(at.map).expect("its map is held"));
        ask.send(Ask::Interior(*at, placing))
            .expect("a worker listens");
    }
    match answers.recv().expect("a worker answers") {
        Answer::Interior(at, placing, body) => driver.give_interior(at, &placing, body),
        Answer::Map(..) => panic!("every map was answered"),
    }
    let streamed = driver.settle(&view);
    assert_eq!(
        (streamed.interiors.len(), streamed.interiors_waiting.len()),
        (1, 0)
    );
    let selected = driver.select(&view, &noon, &ready);
    assert_eq!(selected.instances.len(), DRAWN);

    drop(ask);
    let reads: usize = (workers.into_iter())
        .map(|worker| worker.join().expect("a worker ends"))
        .sum();
    assert_eq!(reads, 4, "three maps and an interior, each read once");
    let read_on = world.read_on.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(read_on.len(), 4);
    assert!(
        !read_on.contains(&std::thread::current().id()),
        "a body was read on the driver's thread"
    );
}

fn first_interior(index: &Index) -> InteriorAt {
    InteriorAt {
        map: index.named(0xA0).expect("the first map"),
        placed: InteriorPlaced {
            node: 0,
            interior: 1,
        },
    }
}

fn key_of(interiors: &[InteriorAt]) -> Vec<u64> {
    interiors.iter().map(InteriorAt::key).collect()
}
