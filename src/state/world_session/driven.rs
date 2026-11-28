//! A world session driven through the world contract: a title of the registry opens the install,
//! the core's `Driver` streams and selects on the session's own thread, and maps, models and
//! textures are read in the core's reader pools.
//!
//! ⚠ Nothing is read on the thread that draws: a frame takes what arrived and the last selection,
//! and never waits for either.
//!
//! ⚠ A model the device was handed and has not answered for is asked for by no one: the thread
//! counts it as on its way until the frame tells where it stands.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use glam::Vec3;
use or_job::{Cancel, Progress};
use or_renderer::FlyCamera;
use or_world::caps::{Capability, WorldCaps};
use or_world::contract::{
    Keys, Opening, Refused, Worker, WorldError, WorldIndexer, WorldInstall, WorldSpec, WorldTitle,
};
use or_world::driver::Driver;
use or_world::kept::{self, Section};
use or_world::model::{
    AssetKey, Feed, FeedName, FrameFacts, Index, Instances, MapId, Moment, PlacementRow, Switched,
    Texture, TextureAsk, WaterQuad,
};
use or_world::pool::Pool;
use or_world::sentence;
use or_world::sentence::Sentence;
use or_world::slots::{OptionKind, WeatherKind};
use or_world::streaming::NodeState;

use super::mesh::{arrived, ArrivedModel};
use super::{
    Kind, Reason, Selection, Standing, Undrawn, Unread, Waiting, WorldFeed, WorldKey, WorldSession,
};
use crate::state::prefs::Prefs;

/// The threads that read map bodies, models and textures.
const READERS: [usize; 3] = [2, 4, 4];
/// The most bytes of models and of textures read and not yet taken by a frame before their readers wait.
const AHEAD: [u64; 2] = [64 << 20, 96 << 20];
/// The most models and textures handed to the frames and not yet taken before no more are taken from the readers.
const QUEUED: usize = 256;
/// The interiors read in one turn of the thread, which reads them itself.
const INTERIORS_A_TURN: usize = 2;
/// How long the thread sleeps between turns while something is on its way, and while nothing is.
const TURN: (Duration, Duration) = (Duration::from_millis(12), Duration::from_millis(120));
/// The weather a session streams under where its title names none.
const NO_WEATHER: u64 = 0;
/// The model of GTA V's stand-in box and any title's like it is asked of no reader: the plain pass draws no stand-in.
const STAND_IN_ABOVE: u64 = 1 << 32;

/// The keys a session opens its install with, owned so that its thread can hold them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedKeys {
    None,
    Snapshot(PathBuf),
    Executable(PathBuf),
}

impl OwnedKeys {
    fn lent(&self) -> Keys<'_> {
        match self {
            Self::None => Keys::None,
            Self::Snapshot(file) => Keys::Snapshot(file),
            Self::Executable(exe) => Keys::Executable(exe),
        }
    }
}

/// What a reader answers: `None` where it stopped on an internal error.
struct Read<T>(Option<Result<T, Refused>>);

impl<T> Default for Read<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> Read<T> {
    fn answer(self) -> Result<T, Refused> {
        let stopped = sentence!("Its reader stopped on an internal error");
        self.0.unwrap_or(Err(Refused::because(stopped)))
    }
}

/// What the frames and the thread hand each other.
struct State {
    stop: bool,
    /// Where the camera stands and its vertical field of view; `None` until a frame was drawn.
    eye: Option<(Vec3, f32)>,
    hour: f32,
    /// The weather a person chose; `None` for the first the title names.
    weather: Option<u64>,
    weathers: Vec<WeatherKind>,
    water: Option<Arc<[WaterQuad]>>,
    undrawn: Undrawn,
    /// Where each model stands on the device, not yet taken by the thread.
    told: Vec<(AssetKey, NodeState)>,
    /// The textures the last frame waits for, where it asked since the thread last took them.
    asks: Option<Vec<(f32, TextureAsk)>>,
    standing: Standing,
    caps: WorldCaps,
    selection: Option<Arc<Selection>>,
    models: VecDeque<(AssetKey, Option<ArrivedModel>)>,
    textures: VecDeque<(TextureAsk, Option<Texture>)>,
    waiting: Waiting,
    unread: Vec<Unread>,
    facts: FrameFacts,
}

struct Shared {
    state: Mutex<State>,
    woken: Condvar,
    cancel: Cancel,
    progress: Progress,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The plain day before any world states its own: noon's.
fn no_facts() -> FrameFacts {
    FrameFacts {
        sun: Vec3::Z,
        key: [1.0; 3],
        ambient: [0.3; 3],
        exposure: 1.0,
        clear: [0.1, 0.2, 0.4],
    }
}

/// A session whose world a title of the registry opened.
pub struct DrivenSession {
    key: WorldKey,
    id: u64,
    option_kinds: Vec<OptionKind>,
    camera: FlyCamera,
    hour: f32,
    weather: Option<u64>,
    weathers: Vec<WeatherKind>,
    water: Option<Arc<[WaterQuad]>>,
    undrawn: Undrawn,
    shared: Arc<Shared>,
    /// What the last poll took from the thread.
    standing: Standing,
    caps: WorldCaps,
    waiting: Waiting,
    unread: Vec<Unread>,
    facts: FrameFacts,
    selection: Option<Arc<Selection>>,
}

/// A number no other session of the process bears.
fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl DrivenSession {
    /// The session of `key`, its world opened on a thread of its own by `title` as `spec` with `keys`, its index kept under `config`.
    pub fn open(
        key: WorldKey,
        title: &'static dyn WorldTitle,
        spec: WorldSpec,
        keys: OwnedKeys,
        config: PathBuf,
    ) -> Self {
        let session = Self::idle(key, title.options());
        let thread = Arc::clone(&session.shared);
        let spawned = std::thread::Builder::new()
            .name("world-session".into())
            .spawn(move || run(&thread, title, &spec, &keys, &config));
        if let Err(why) = spawned {
            session.shared.state().standing = Standing::Failed(why.to_string());
        }
        session
    }

    /// A session with no thread yet, standing as one that opens.
    fn idle(key: WorldKey, option_kinds: Vec<OptionKind>) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                stop: false,
                eye: None,
                hour: 12.0,
                weather: None,
                weathers: Vec::new(),
                water: None,
                undrawn: Undrawn::default(),
                told: Vec::new(),
                asks: None,
                standing: Standing::Opening(0.0),
                caps: WorldCaps::none(),
                selection: None,
                models: VecDeque::new(),
                textures: VecDeque::new(),
                waiting: Waiting::default(),
                unread: Vec::new(),
                facts: no_facts(),
            }),
            woken: Condvar::new(),
            cancel: Cancel::new(),
            progress: Progress::new(),
        });
        Self {
            key,
            id: next_id(),
            option_kinds,
            camera: FlyCamera {
                position: Vec3::new(0.0, 0.0, 300.0),
                yaw: 0.0,
                pitch: -0.35,
                speed: 30.0,
                fov_y: 60f32.to_radians(),
                near: 0.5,
                far: 20_000.0,
            },
            hour: 12.0,
            weather: None,
            weathers: Vec::new(),
            water: None,
            undrawn: Undrawn::default(),
            shared,
            standing: Standing::Opening(0.0),
            caps: WorldCaps::none(),
            waiting: Waiting::default(),
            unread: Vec::new(),
            facts: no_facts(),
            selection: None,
        }
    }

    /// The session of the install `key` names, opened by the title the registry lists for its game with the keys the app keeps for it.
    pub fn of_install(key: WorldKey, prefs: &Prefs) -> Self {
        let Some((entry, game)) = or_worlds::of_game(key.game) else {
            return Self::failed(
                key,
                tr!("No world of this game is opened by this build of OpenRage").to_string(),
            );
        };
        let Some(config) = crate::state::prefs::config_root() else {
            return Self::failed(
                key,
                tr!("There is no configuration folder to keep the world's index in").to_string(),
            );
        };
        let spec = WorldSpec {
            options: key.options.clone(),
            ..WorldSpec::of(game, key.install.clone())
        };
        let cache =
            crate::state::prefs::keys_cache_path_for(key.game).filter(|file| file.is_file());
        let executable = (prefs.gta_keys.gta5_path.clone())
            .filter(|exe| exe.starts_with(&key.install) && exe.is_file())
            .or_else(|| key.game.find_executable(&key.install));
        let keys = match (cache, executable) {
            (Some(cache), _) => OwnedKeys::Snapshot(cache),
            (None, Some(executable)) => OwnedKeys::Executable(executable),
            (None, None) => OwnedKeys::None,
        };
        Self::open(key, entry.title, spec, keys, config)
    }

    /// A session that never opened, and why.
    fn failed(key: WorldKey, why: String) -> Self {
        let mut session = Self::idle(key, Vec::new());
        session.shared.state().standing = Standing::Failed(why.clone());
        session.standing = Standing::Failed(why);
        session
    }
}

impl Drop for DrivenSession {
    fn drop(&mut self) {
        self.shared.cancel.set();
        self.shared.state().stop = true;
        self.shared.woken.notify_all();
    }
}

impl WorldSession for DrivenSession {
    fn key(&self) -> &WorldKey {
        &self.key
    }

    fn poll(&mut self) {
        let mut state = self.shared.state();
        let eye = Some((self.camera.position, self.camera.fov_y));
        let moved = state.eye != eye || state.hour != self.hour || state.weather != self.weather;
        (state.eye, state.hour, state.weather) = (eye, self.hour, self.weather);
        // Both are stated once, when the world opens.
        if self.weathers.len() != state.weathers.len() {
            self.weathers.clone_from(&state.weathers);
        }
        if self.water.is_none() {
            self.water.clone_from(&state.water);
        }
        self.undrawn = state.undrawn;
        self.standing = match &state.standing {
            Standing::Opening(_) => Standing::Opening(self.shared.progress.snapshot().fraction()),
            other => other.clone(),
        };
        self.caps = state.caps;
        self.waiting = state.waiting;
        self.unread.clone_from(&state.unread);
        self.facts = state.facts;
        self.selection.clone_from(&state.selection);
        drop(state);
        if moved {
            self.shared.woken.notify_all();
        }
    }

    fn standing(&self) -> Standing {
        self.standing.clone()
    }

    fn caps(&self) -> WorldCaps {
        self.caps
    }

    fn option_kinds(&self) -> Vec<OptionKind> {
        self.option_kinds.clone()
    }

    fn camera(&self) -> FlyCamera {
        self.camera.clone()
    }

    fn set_camera(&mut self, camera: FlyCamera) {
        self.camera = camera;
    }

    fn hour(&self) -> f32 {
        self.hour
    }

    fn set_hour(&mut self, hour: f32) {
        self.hour = hour.rem_euclid(24.0);
    }

    fn weathers(&self) -> Vec<WeatherKind> {
        self.weathers.clone()
    }

    fn weather(&self) -> u64 {
        let first = self.weathers.first().map(|weather| weather.id);
        self.weather.or(first).unwrap_or(NO_WEATHER)
    }

    fn set_weather(&mut self, weather: u64) {
        self.weather = Some(weather);
    }

    fn undrawn(&self) -> Undrawn {
        self.undrawn
    }

    fn waiting(&self) -> Waiting {
        self.waiting
    }

    fn unread(&self) -> Vec<Unread> {
        self.unread.clone()
    }

    fn feed(&mut self) -> Option<&mut dyn WorldFeed> {
        matches!(self.standing, Standing::Open).then_some(self as &mut dyn WorldFeed)
    }
}

impl WorldFeed for DrivenSession {
    fn id(&self) -> u64 {
        self.id
    }

    fn camera(&self) -> FlyCamera {
        self.camera.clone()
    }

    fn facts(&self) -> FrameFacts {
        self.facts
    }

    fn selection(&self) -> Option<Arc<Selection>> {
        self.selection.clone()
    }

    fn take_model(&mut self) -> Option<(AssetKey, Option<ArrivedModel>)> {
        self.shared.state().models.pop_front()
    }

    fn take_texture(&mut self) -> Option<(TextureAsk, Option<Texture>)> {
        self.shared.state().textures.pop_front()
    }

    fn tell(&mut self, model: AssetKey, stands: NodeState) {
        self.shared.state().told.push((model, stands));
        self.shared.woken.notify_all();
    }

    fn want_textures(&mut self, asks: Vec<(f32, TextureAsk)>) {
        self.shared.state().asks = Some(asks);
        self.shared.woken.notify_all();
    }

    fn water(&self) -> Option<Arc<[WaterQuad]>> {
        self.water.clone()
    }
}

/// An indexer that counts the archives read towards a progress.
struct Counted<'a> {
    of: &'a dyn WorldIndexer,
    progress: &'a Progress,
    archives: u64,
    read: AtomicU64,
}

impl WorldIndexer for Counted<'_> {
    fn rules(&self) -> u32 {
        self.of.rules()
    }

    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError> {
        let rows = self.of.rows(archive, out);
        let read = self.read.fetch_add(1, Ordering::Relaxed) + 1;
        self.progress.reached(read, self.archives);
        rows
    }

    fn whole(&self, archives: &[Section], out: &mut Section) -> Result<(), WorldError> {
        self.of.whole(archives, out)
    }

    fn link(&self, archives: &[Section], whole: &Section) -> Result<Index, WorldError> {
        self.of.link(archives, whole)
    }

    fn placements(
        &self,
        index: &Index,
        map: MapId,
        out: &mut Vec<PlacementRow>,
    ) -> Result<(), WorldError> {
        self.of.placements(index, map, out)
    }
}

/// A feed that takes nothing: the plain pass reads the frame's facts alone.
struct Unfed;

impl Feed for Unfed {
    fn set(&mut self, _name: FeedName<'_>, _value: &[f32]) {}
}

type Opened = (Arc<dyn WorldInstall>, Arc<Index>);

/// Open `spec` through `title` and take its index from the kept file under `config`.
fn open(
    shared: &Shared,
    title: &dyn WorldTitle,
    spec: &WorldSpec,
    keys: &OwnedKeys,
    config: &Path,
) -> Result<Opened, String> {
    let opening = Opening {
        config,
        keys: keys.lent(),
        cancel: &shared.cancel,
        progress: &shared.progress,
    };
    let install = title.open(spec, &opening).map_err(|e| e.to_string())?;
    let file = kept::file_for(config, title.id(), spec);
    let inputs = install.inputs();
    let counted = Counted {
        of: install.indexer(),
        progress: &shared.progress,
        archives: inputs.archives.len() as u64,
        read: AtomicU64::new(0),
    };
    let (index, report) =
        kept::refresh_with(title.id(), spec, inputs, &counted, &file).map_err(|e| e.to_string())?;
    tracing::info!(
        title = title.id(),
        maps = index.maps().len(),
        read = report.read.len(),
        verdict = ?report.verdict,
        "a world's index is held"
    );
    Ok((install, Arc::new(index)))
}

/// What the thread counts as unread, each thing once.
#[derive(Default)]
struct Refusals {
    maps: BTreeMap<MapId, String>,
    models: HashMap<AssetKey, Sentence>,
    textures: HashMap<(AssetKey, u64), Sentence>,
}

impl Refusals {
    /// Each kind and reason once with its count, the most first.
    fn counted(&self) -> Vec<Unread> {
        let mut counts: BTreeMap<(Kind, Reason), usize> = BTreeMap::new();
        for why in self.maps.values() {
            *counts
                .entry((Kind::Map, Reason::Told(why.clone())))
                .or_default() += 1;
        }
        for why in self.models.values() {
            *counts.entry((Kind::Model, Reason::Said(*why))).or_default() += 1;
        }
        for why in self.textures.values() {
            *counts
                .entry((Kind::Texture, Reason::Said(*why)))
                .or_default() += 1;
        }
        let mut unread: Vec<Unread> = (counts.into_iter())
            .map(|((kind, why), count)| Unread { kind, why, count })
            .collect();
        unread.sort_by_key(|unread| std::cmp::Reverse(unread.count));
        unread
    }
}

/// The nearest each model of `instances` stands to `eye`, by its place among the models.
fn nearest(instances: &Instances, eye: Vec3) -> Vec<f32> {
    let mut near = vec![f32::INFINITY; instances.models.len()];
    for (model, sphere) in instances.model.iter().zip(&instances.sphere) {
        let away = (Vec3::new(sphere[0], sphere[1], sphere[2]).distance(eye) - sphere[3]).max(0.0);
        if let Some(least) = near.get_mut(*model as usize) {
            *least = least.min(away);
        }
    }
    near
}

/// What `selected` holds that the plain pass does not draw: the instances of a stand-in, and the lists that laid out no instance or that their bodies lay out none of.
fn undrawn_of(selected: &or_world::driver::Selected) -> Undrawn {
    let instances = &selected.instances;
    let stands_in = |model: &u32| {
        let key = instances.models.get(*model as usize);
        key.is_some_and(|key| key.0 >= STAND_IN_ABOVE)
    };
    let laid: HashSet<_> = instances.node.iter().collect();
    let lists = selected.selection.lists.iter();
    let unlaid = selected
        .bodies
        .iter()
        .map(|(_, body)| body.unlaid_lists() as usize);
    Undrawn {
        stand_ins: instances
            .model
            .iter()
            .filter(|model| stands_in(model))
            .count(),
        lists: lists.filter(|list| !laid.contains(&list.node)).count() + unlaid.sum::<usize>(),
    }
}

/// The session's thread: open the world, then stream, select and hand over until told to stop.
fn run(
    shared: &Arc<Shared>,
    title: &'static dyn WorldTitle,
    spec: &WorldSpec,
    keys: &OwnedKeys,
    config: &Path,
) {
    let (install, index) = match open(shared, title, spec, keys, config) {
        Ok(opened) => opened,
        Err(why) => {
            tracing::warn!(install = %spec.install.display(), %why, "a world did not open");
            let mut state = shared.state();
            if !state.stop {
                state.standing = Standing::Failed(why);
            }
            return;
        }
    };
    let policy = install.policy();
    let weathers = install.look().weathers(&index);
    let first_weather = weathers.first().map(|weather| weather.id);
    let water = match install.caps().has(Capability::Water) {
        true => match install.water(&index) {
            Ok(quads) => Some(Arc::from(quads)),
            Err(why) => {
                tracing::warn!(%why, "the water of a world did not read");
                None
            }
        },
        false => None,
    };
    {
        let mut state = shared.state();
        (state.standing, state.caps) = (Standing::Open, install.caps());
        (state.weathers, state.water) = (weathers, water);
    }
    let maps = {
        let (install, index) = (Arc::clone(&install), Arc::clone(&index));
        Pool::<MapId, Read<Arc<dyn or_world::contract::MapBody>>>::new(
            READERS[0],
            Worker::new,
            move |worker, map| {
                let body = install.maps().read(&index, map, worker);
                Read(Some(body.map_err(|why| Refused {
                    why: sentence!("The map does not read"),
                    detail: why.to_string(),
                })))
            },
        )
    };
    let models = {
        let (install, index) = (Arc::clone(&install), Arc::clone(&index));
        let weigh: fn(&Read<ArrivedModel>) -> u64 = |read| {
            read.0
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .map_or(0, ArrivedModel::bytes)
        };
        Pool::<AssetKey, Read<ArrivedModel>>::bounded(
            READERS[1],
            (AHEAD[0], weigh),
            Worker::new,
            move |worker, ask| {
                let mut out = Vec::with_capacity(1);
                (install.models()).models_or_why(&index, &[ask], worker, &mut out);
                let undecoded = sentence!("No mesh of the model has a layout the plain pass draws");
                Read(out.pop().map(|(_, model)| {
                    model.and_then(|model| arrived(&model).ok_or(Refused::because(undecoded)))
                }))
            },
        )
    };
    let textures = {
        let (install, index) = (Arc::clone(&install), Arc::clone(&index));
        let weigh: fn(&Read<Texture>) -> u64 = |read| {
            let texture = read.0.as_ref().and_then(|r| r.as_ref().ok());
            texture.map_or(0, |t| t.levels.iter().map(|level| level.len() as u64).sum())
        };
        Pool::<TextureAsk, Read<Texture>>::bounded(
            READERS[2],
            (AHEAD[1], weigh),
            Worker::new,
            move |worker, ask| {
                let mut out = Vec::with_capacity(1);
                (install.models()).textures_or_why(&index, &[ask], worker, &mut out);
                Read(out.pop().map(|(_, texture)| texture))
            },
        )
    };
    let mut driver = Driver::new(Arc::clone(&install), Arc::clone(&index));
    let mut worker = Worker::new();
    let switched = Switched::default();
    let mut ready: HashMap<AssetKey, NodeState> = HashMap::new();
    // The models handed to the frames whose place on the device was not told yet.
    let mut handed: HashSet<AssetKey> = HashSet::new();
    let mut refusals = Refusals::default();
    let mut texture_asks: Vec<(f32, TextureAsk)> = Vec::new();
    let (mut version, mut last) = (0u64, None::<Instances>);
    let mut undrawn = Undrawn::default();
    loop {
        let (eye, hour, weather, told, asks) = {
            let mut state = shared.state();
            if state.stop {
                return;
            }
            let told = std::mem::take(&mut state.told);
            let weather = state.weather.or(first_weather).unwrap_or(NO_WEATHER);
            (state.eye, state.hour, weather, told, state.asks.take())
        };
        for (model, stands) in told {
            handed.remove(&model);
            match stands {
                NodeState::Loading => ready.remove(&model),
                stands => ready.insert(model, stands),
            };
        }
        let Some((eye, fov_y)) = eye else {
            let state = shared.state();
            drop(shared.woken.wait_timeout(state, TURN.1));
            continue;
        };
        let moment = Moment { hour, weather };
        let view = policy.view(eye, Vec3::ZERO, fov_y);

        let wanted = driver.want(&view, &moment, &switched, None);
        maps.want(
            (wanted.iter().enumerate())
                .map(|(rank, map)| (rank as f32, *map))
                .collect(),
        );
        for (map, read) in maps.take(|| true) {
            let body = read.answer().map_err(|refused| WorldError::Unread {
                what: (index.map(map)).map_or_else(String::new, |row| row.label.clone()),
                why: refused.detail,
            });
            driver.give(map, body);
        }
        let streamed = driver.settle(&view);
        for interior in streamed.interiors_waiting.iter().take(INTERIORS_A_TURN) {
            let Some(placing) = driver.placing(interior.map).cloned() else {
                continue;
            };
            let (reader, placed) = (install.maps(), &interior.placed);
            let body = reader.interior(&index, interior.map, &*placing, placed, &mut worker);
            driver.give_interior(*interior, &placing, body);
        }
        for (map, why) in &streamed.unread {
            refusals.maps.entry(*map).or_insert_with(|| why.clone());
        }

        // ⚠ What the readers answered is taken before anything is asked: an ask made first would name again what this turn takes, and it would be read twice.
        let room = QUEUED.saturating_sub(shared.state().models.len());
        let mut left = room;
        let taken = models.take(|| {
            let more = left > 0;
            left = left.saturating_sub(1);
            more
        });
        let mut arrived_models = Vec::with_capacity(taken.len());
        for (ask, read) in taken {
            match read.answer() {
                Ok(model) => {
                    handed.insert(ask);
                    arrived_models.push((ask, Some(model)));
                }
                Err(refused) => {
                    tracing::debug!(model = ask.0, why = refused.why.key(), detail = %refused.detail, "a model gave the world nothing");
                    refusals.models.insert(ask, refused.why);
                    ready.insert(ask, NodeState::Absent);
                    arrived_models.push((ask, None));
                }
            }
        }
        let room = QUEUED.saturating_sub(shared.state().textures.len());
        let mut left = room;
        let taken = textures.take(|| {
            let more = left > 0;
            left = left.saturating_sub(1);
            more
        });
        let mut arrived_textures = Vec::with_capacity(taken.len());
        for (ask, read) in taken {
            texture_asks.retain(|(_, asked)| *asked != ask);
            match read.answer() {
                Ok(texture) => arrived_textures.push((ask, Some(texture))),
                Err(refused) => {
                    tracing::debug!(texture = ask.name, why = refused.why.key(), detail = %refused.detail, "a texture gave the world nothing");
                    refusals.textures.insert((ask.scope, ask.name), refused.why);
                    arrived_textures.push((ask, None));
                }
            }
        }

        let stands = |model: AssetKey| ready.get(&model).copied().unwrap_or(NodeState::Loading);
        let selected = driver.select(&view, &moment, &stands);
        let near = nearest(&selected.instances, eye);
        let unasked = |model: &AssetKey| {
            model.0 < STAND_IN_ABOVE && !ready.contains_key(model) && !handed.contains(model)
        };
        // A model a parent waits for draws nothing yet, so it stands nowhere: asked for in the order the selection lists it, behind what draws.
        let drawing = selected.instances.models.iter().zip(&near);
        let behind = selected.waiting.iter().enumerate();
        let mut asked = HashSet::new();
        let model_asks: Vec<(f32, AssetKey)> = drawing
            .map(|(model, away)| (*away, *model))
            .chain(behind.map(|(rank, model)| (1e6 + rank as f32, *model)))
            .filter(|(_, model)| unasked(model) && asked.insert(*model))
            .collect();
        let models_waiting = model_asks.len() + handed.len();
        models.want(model_asks);

        if let Some(asks) = asks {
            // ⚠ An answer a frame has not taken yet is still asked for by that frame: read again, it would be read once a frame.
            let mut queued: HashSet<TextureAsk> =
                (shared.state().textures.iter().map(|(ask, _)| *ask)).collect();
            queued.extend(arrived_textures.iter().map(|(ask, _)| *ask));
            texture_asks = asks;
            texture_asks.retain(|(_, ask)| {
                !queued.contains(ask) && !refusals.textures.contains_key(&(ask.scope, ask.name))
            });
            textures.want(texture_asks.clone());
        }

        let changed = last.as_ref() != Some(&selected.instances);
        if changed {
            version += 1;
            last = Some(selected.instances.clone());
            undrawn = undrawn_of(&selected);
        }
        let waiting = Waiting {
            maps: streamed.waiting.len() + streamed.interiors_waiting.len(),
            models: models_waiting,
            textures: texture_asks.len(),
        };
        let facts = install.look().frame(&moment, &mut Unfed);
        let busy = waiting.any() || selected.fading || changed;
        let mut state = shared.state();
        if changed {
            state.selection = Some(Arc::new(Selection {
                version,
                instances: selected.instances,
                fading: selected.fading,
            }));
        }
        state.models.extend(arrived_models);
        state.textures.extend(arrived_textures);
        (state.waiting, state.facts, state.undrawn) = (waiting, facts, undrawn);
        state.unread = refusals.counted();
        let turn = match busy {
            true => TURN.0,
            false => TURN.1,
        };
        // ⚠ What a frame told while this turn ran is taken by the next at once: the wait is only where nothing was.
        if state.told.is_empty() && state.asks.is_none() && !state.stop {
            drop(shared.woken.wait_timeout(state, turn));
        }
    }
}
