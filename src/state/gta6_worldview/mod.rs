//! The GTA VI world viewer: the index of the install it flies through, built on a worker and kept,
//! the maps streamed in around its camera, and the models read for what they draw.
//!
//! ⚠ What is on the device is the render bridge's: a frame asks here what draws and which models
//! arrived, and says which are on the device.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use glam::Vec3;
use or_job::{Cancel, Progress};
use or_renderer::FlyCamera;
use or_world::streaming_rules::FOCUS_JUMP_DISTANCE;

use crate::state::jobs::{JobKind, Outcome, RunningJob, Then};
use crate::ui::panels::log::grouped;

pub(crate) mod index;
mod interior;
mod interior_rooms;
pub(crate) mod lights;
mod lights_corona;
mod lights_distant;
pub(crate) mod lights_frame;
pub(crate) mod lights_index;
mod lights_law;
mod lights_lod;
mod lights_vars;
mod model;
mod occluders;
mod pool;
pub(crate) mod sky;
mod sky_night;
mod sky_weather;
mod slod_look;
pub(crate) mod stream;
pub(crate) mod terrain;
mod texture;
pub(crate) mod water;
mod wind;

pub use interior::Inside;
pub use model::{Bucket, CpuMaterial, CpuModel, Family, RegionTint};
#[cfg(test)]
pub use model::{Constants, Emissive, Interior, Layer};
pub use texture::CpuTexture;

use index::{Enabled, ModelRef, WorldIndex};
use pool::{soonest, Pool};
use stream::{Selected, Streaming, World};
use texture::{OpenPacks, Places};

/// How high over the middle of the maps the camera starts, in metres.
const START_HEIGHT: f32 = 150.0;
/// The hour of the day the viewer starts at.
const NOON: f32 = 12.0;
/// The bytes of models and textures the readers read ahead of what a frame takes: past them a
/// reader waits, or what it reads piles up faster than a frame puts it on the device.
// ponytail: about ten frames of what a frame takes at most; the device's own budgets if a reader idles while there is room.
const READ_AHEAD: u64 = 512 << 20;

/// How many readers the models get, and as many the textures: half the machine's threads, from 2 to 12.
fn readers() -> usize {
    (std::thread::available_parallelism().map_or(4, usize::from) / 2).clamp(2, 12)
}

/// Wait while what was read and not taken (`held`) is past [`READ_AHEAD`].
fn wait_for_room(held: &AtomicU64) {
    while held.load(Ordering::Relaxed) > READ_AHEAD {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
/// The bytes of models, and of textures, read and not yet taken onto the device, past which the readers wait: about five frames of what a frame puts on the device.
// ⚠ Unbounded, the models read faster than the frames take them pile up in memory while the camera flies.
const MODELS_WAITING: u64 = 256 << 20;
const TEXTURES_WAITING: u64 = 64 << 20;
/// How many times the world's thread is started again after it dies: one that dies on what it
/// reads dies again on it.
const MOST_RESTARTS: u32 = 3;

/// How a build of the index ended.
enum Built {
    Index(Box<WorldIndex>, Option<Places>),
    Stopped,
    Failed(String),
}

struct Build {
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Built>>>,
}

impl Drop for Build {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// What the world viewer holds.
pub struct Viewer {
    /// The install it shows.
    root: Option<PathBuf>,
    pub index: Option<Arc<WorldIndex>>,
    world: Option<World>,
    /// The models on the device (`Some(true)`), there but held back for their textures
    /// (`Some(false)`) or known never to come (`None`), which are not asked for again.
    have: HashMap<ModelRef, Option<bool>>,
    /// How many times the world's thread was found dead and started again.
    pub world_restarts: u32,
    /// How many of those deaths named no map it had taken in: only these give the thread up.
    unexplained: u32,
    /// The maps the world's thread died on, which it is started again without.
    pub skipped: HashSet<u32>,
    /// The selection whose waited-for models the readers were last given.
    wanted_of: u64,
    /// The selection under way whose waited-for models the readers were last given.
    asked_of: u64,
    /// The models that selection waits for, and those the far stand-ins' atlas waits for.
    wanted: Vec<(f32, ModelRef)>,
    species_models: Vec<(f32, ModelRef)>,
    /// The band each model of that selection is asked for in out of view, and those a frame last saw something wait for.
    bands: HashMap<ModelRef, f32>,
    in_view: HashSet<ModelRef>,
    /// How soon each model and each texture asked for last is needed: what arrived is put on the device soonest first.
    soon: (HashMap<ModelRef, f32>, HashMap<u64, f32>),
    models: Option<Pool<ModelRef, Option<CpuModel>>>,
    /// A texture is asked for by its key and the screen size it is needed at.
    textures: Option<Pool<(u64, u32), Option<CpuTexture>>>,
    /// The bytes of the models and textures the workers read that were not taken yet.
    pub held: Arc<AtomicU64>,
    /// The hour and what is enabled, as the world's thread was last told them.
    sent: Option<(f32, Enabled)>,
    build: Option<Build>,
    /// How the last build ended, for the job list to say once.
    ended: Option<(Outcome, String)>,
    /// Why there is no index, where a build failed.
    pub failed: Option<String>,
    pub camera: FlyCamera,
    /// How many times the camera was put down somewhere else rather than flown there: each is a cut the exposure starts again at.
    pub cuts: u32,
    /// The scale every level-of-detail distance is multiplied by.
    pub lod_scale: f32,
    /// The hour of the day, from 0 to 24: what is placed for some hours alone draws inside them.
    pub hour: f32,
    /// Which of the game's modes and script-managed groups stream.
    pub enabled: Enabled,
    /// Whether the person chose the script-managed groups, which the hour then no longer sets.
    pub groups_chosen: bool,
    /// Whether the groups at rest are those the data alone proves, without what a fresh game is taken to have.
    pub groups_proven: bool,
    /// The lights of the maps around the camera, once an index is held.
    pub lights: Option<lights::Lights>,
    /// How many models wait to be read.
    pub models_waiting: usize,
    /// How many textures wait to be read.
    pub textures_waiting: usize,
    /// The models and the textures taken that nothing asked for any more when they were.
    pub stale: [usize; 2],
    /// Where the camera stood the frame before, and how many times it jumped from there.
    last_eye: Option<Vec3>,
    pub jumps: usize,
    /// How long the last frame took to work out its day, before the lights about the camera.
    pub day_took: std::time::Duration,
    /// The game's wind about the camera.
    wind: wind::Wind,
    /// The change of weather the frame is drawn in.
    weathering: sky::Weathering,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            root: None,
            index: None,
            world: None,
            have: HashMap::new(),
            world_restarts: 0,
            unexplained: 0,
            skipped: HashSet::new(),
            wanted_of: 0,
            asked_of: 0,
            wanted: Vec::new(),
            species_models: Vec::new(),
            bands: HashMap::new(),
            in_view: HashSet::new(),
            soon: Default::default(),
            models: None,
            textures: None,
            held: Arc::default(),
            sent: None,
            build: None,
            ended: None,
            failed: None,
            camera: FlyCamera {
                position: Vec3::new(0.0, 0.0, START_HEIGHT),
                yaw: 0.0,
                pitch: -0.35,
                speed: 30.0,
                fov_y: 60f32.to_radians(),
                near: 0.5,
                far: 20_000.0,
            },
            cuts: 0,
            lod_scale: 1.0,
            hour: NOON,
            enabled: Enabled::default(),
            groups_chosen: false,
            groups_proven: false,
            lights: None,
            models_waiting: 0,
            textures_waiting: 0,
            stale: [0; 2],
            last_eye: None,
            jumps: 0,
            day_took: std::time::Duration::ZERO,
            wind: wind::Wind::default(),
            weathering: sky::Weathering::default(),
        }
    }
}

/// The viewer, locked: the screen draws from it and the render bridge asks it what draws.
// ponytail: one viewer for the whole app, of the install it was last opened on.
pub fn viewer() -> MutexGuard<'static, Viewer> {
    static VIEWER: LazyLock<Mutex<Viewer>> = LazyLock::new(Mutex::default);
    VIEWER.lock().unwrap_or_else(|p| p.into_inner())
}

impl Viewer {
    /// Show the install at `root`: the index kept of it where one stands, else one built on a worker.
    pub fn open(&mut self, root: &Path) {
        if self.root.as_deref() == Some(root) {
            return;
        }
        // What is enabled is a choice among one install's groups: another install starts at rest.
        *self = Self {
            root: Some(root.to_path_buf()),
            camera: self.camera.clone(),
            lod_scale: self.lod_scale,
            hour: self.hour,
            ..Self::default()
        };
        self.start(false);
    }

    /// Build the index again on a worker, in place of the one held and of the one kept.
    pub fn rebuild(&mut self) {
        self.start(true);
    }

    /// Take the index kept of the install on a worker, unless `afresh`, else build one there.
    fn start(&mut self, afresh: bool) {
        let Some(root) = self.root.clone() else {
            return;
        };
        (self.index, self.world, self.models, self.failed) = (None, None, None, None);
        (self.have, self.world_restarts, self.unexplained) = (HashMap::new(), 0, 0);
        self.skipped.clear();
        (self.textures, self.lights) = (None, None);
        // What the workers held goes with them, and a new world's thread is told everything again.
        (self.held, self.sent) = (Arc::default(), None);
        // ⚠ Where the packs' index is kept is found here, on this thread.
        let places = crate::state::gta6_texture_index::pack_places(&root);
        let build = Build {
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            build.cancel.clone(),
            build.progress.clone(),
            Arc::clone(&build.ended),
        );
        std::thread::spawn(move || {
            let build = || {
                let kept = (crate::state::prefs::config_root())
                    .map(|config| index::kept_at(&config, &root));
                let held =
                    (kept.as_ref().filter(|_| !afresh)).and_then(|file| WorldIndex::kept(file));
                let places = |cancel: &Cancel| {
                    let (packs, keys, formats, dictionaries) = places(cancel, &Progress::new())?;
                    Some(Places {
                        packs,
                        keys,
                        formats,
                        dictionaries,
                    })
                };
                // ⚠ A folder with no game builds an index of nothing, which is no world: it is not kept, and says so.
                let no_game = || {
                    Built::Failed(
                        tr!(
                            "no GTA VI files were found in {folder}. Settings ▸ Games must name the folder GTA VI is installed in, the one that holds its .rpf archives",
                            folder = root.display()
                        )
                        .to_string(),
                    )
                };
                match held {
                    Some(index) if index.report.maps == 0 => no_game(),
                    Some(index) => Built::Index(Box::new(index), places(&cancel)),
                    None => {
                        let fresh = kept.as_deref().map(index::fresh_beside);
                        match index::build(&root, fresh.as_deref(), &cancel, &progress) {
                            Ok(Some(index)) if index.report.maps == 0 => no_game(),
                            Ok(Some(index)) => {
                                if let Some(Err(error)) = kept.map(|file| index.keep(&file)) {
                                    tracing::warn!(%error, "the world index was not kept");
                                }
                                Built::Index(Box::new(index), places(&cancel))
                            }
                            Ok(None) => Built::Stopped,
                            Err(why) => Built::Failed(why),
                        }
                    }
                }
            };
            let failed = Built::Failed(tr!("It stopped on an internal error").to_string());
            let built = pool::or_on_panic("the index", build, failed);
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(built);
        });
        self.build = Some(build);
    }

    fn take(&mut self, index: WorldIndex, places: Option<Places>) {
        let places = places.map(Arc::new);
        if let Some(places) = places.clone() {
            sky_night::read(Arc::clone(&places));
            let held = Arc::clone(&self.held);
            let weigh = |read: &Option<CpuTexture>| read.as_ref().map_or(0, CpuTexture::bytes);
            let open = OpenPacks::default();
            // ⚠ A texture read decodes its blocks on the CPU and costs as much as a model's: four readers alone held a street scene's settle to its textures.
            self.textures = Some(Pool::bounded(
                readers(),
                (TEXTURES_WAITING, weigh),
                || (),
                move |(), (key, pixels)| {
                    wait_for_room(&held);
                    let read = texture::read_texture(&open, &places, key, pixels);
                    let bytes = read.as_ref().map_or(0, CpuTexture::bytes);
                    held.fetch_add(bytes, Ordering::Relaxed);
                    read
                },
            ));
        }
        if let Some(middle) = index.middle() {
            self.camera.position = middle + Vec3::Z * START_HEIGHT;
            self.cuts += 1;
        }
        let index = Arc::new(index);
        self.lights = Some(lights::Lights::new(Arc::clone(&index)));
        let shared = Arc::clone(&index);
        let held = Arc::clone(&self.held);
        let weigh = |read: &Option<CpuModel>| read.as_ref().map_or(0, CpuModel::bytes);
        self.models = Some(Pool::bounded(
            readers(),
            (MODELS_WAITING, weigh),
            HashMap::<u32, File>::new,
            move |files, model| {
                wait_for_room(&held);
                let read = model::read_model(files, &shared, places.as_deref(), model);
                held.fetch_add(read.as_ref().map_or(0, CpuModel::bytes), Ordering::Relaxed);
                read
            },
        ));
        let (eye, scale) = (self.camera.position, self.distance_scale());
        self.world = Some(World::new(
            Arc::clone(&index),
            eye,
            scale,
            self.skipped.clone(),
        ));
        self.index = Some(index);
    }

    /// Take what a build that ended built.
    pub fn poll(&mut self) {
        let built = (self.build.as_ref())
            .and_then(|b| b.ended.lock().unwrap_or_else(|p| p.into_inner()).take());
        let Some(built) = built else {
            return;
        };
        self.build = None;
        self.ended = Some(match built {
            Built::Index(index, places) => {
                let said = tr!(
                    "The GTA VI world is indexed: {maps} maps stream at rest, {archetypes} archetypes, {models} models",
                    maps = grouped(index.report.at_rest),
                    archetypes = grouped(index.report.archetypes),
                    models = grouped(index.report.models),
                );
                self.take(*index, places);
                (Outcome::Done(Then::Nothing), said)
            }
            Built::Stopped => (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished").to_string(),
            ),
            Built::Failed(why) => {
                self.failed = Some(why.clone());
                (
                    Outcome::Failed,
                    tr!("The GTA VI world was not indexed: {why}", why = why),
                )
            }
        });
    }

    /// Whether an index is being built, and how far it is.
    pub fn building(&self) -> Option<f32> {
        (self.build.as_ref()).map(|b| b.progress.snapshot().fraction())
    }

    /// The build while it runs, as the job list draws it.
    pub(in crate::state) fn job(&self) -> Option<RunningJob> {
        let build = self.build.as_ref()?;
        Some(RunningJob {
            kind: JobKind::WorldIndex,
            doing: tr!("Indexing the GTA VI world").to_string(),
            fraction: Some(build.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: build.cancel.is_set(),
        })
    }

    pub(in crate::state) fn stop(&self) {
        if let Some(build) = &self.build {
            build.cancel.set();
        }
    }

    /// How the build ended, as the job list says it, once.
    pub(in crate::state) fn ended(&mut self) -> Option<(Outcome, String)> {
        self.ended.take()
    }

    /// How the streaming stands.
    pub fn streaming(&self) -> Streaming {
        (self.world.as_ref()).map_or_else(Streaming::default, |w| w.latest().streaming)
    }

    /// Whether the world's thread was told something its last selection does not stand on yet.
    pub fn selecting(&self) -> bool {
        self.world.as_ref().is_some_and(World::unselected)
    }

    /// Whether the last selection has a node fading in by time, which each selection after it takes a step further.
    pub fn fading(&self) -> bool {
        (self.world.as_ref()).is_some_and(|w| w.latest().fading)
    }

    /// Whether the last selection was made for where the camera stands now: until it is, what it waits for is the last place's.
    pub fn caught_up(&self) -> bool {
        (self.world.as_ref()).is_none_or(|w| w.latest().eye == self.camera.position)
    }

    /// The interior and room the camera stood in at the last selection, where it stood in one.
    pub fn inside(&self) -> Option<interior::Inside> {
        self.world.as_ref()?.latest().inside
    }

    /// Whether the camera saw anything of the outside at the last selection: from a room, only through a portal to it.
    pub fn sees_outside(&self) -> bool {
        (self.world.as_ref()).is_none_or(|w| {
            (w.latest().outside_through.as_ref()).is_none_or(|through| !through.is_empty())
        })
    }

    /// What was selected last for the camera, after the world's thread is told where the camera
    /// is and which models came to the device, will never come (`None`) or left it (`false`).
    pub fn selected(
        &mut self,
        models: &mut Vec<(ModelRef, Option<bool>)>,
    ) -> Option<Arc<Selected>> {
        if self.world.as_ref().is_some_and(World::died) && self.unexplained < MOST_RESTARTS {
            self.restart_world();
        }
        self.world.as_ref()?;
        for (model, state) in models.iter() {
            match state {
                Some(false) => self.have.remove(model),
                state => self.have.insert(*model, *state),
            };
        }
        let told = !models.is_empty();
        let eye = self.camera.position;
        if self
            .last_eye
            .is_some_and(|was| was.distance(eye) > FOCUS_JUMP_DISTANCE)
        {
            self.forget_asks();
        }
        self.last_eye = Some(eye);
        let world = self.world.as_ref()?;
        world.look(eye, self.distance_scale());
        // The groups the world-state data has on at this hour and these modes, unless the person chose them.
        if let (false, Some(index)) = (self.groups_chosen, &self.index) {
            let on = index.default_on_in(self.hour, self.enabled.modes, self.groups_proven);
            if on != self.enabled.groups {
                self.enabled.groups = on;
            }
        }
        let hour = (self.sent.as_ref()).is_none_or(|(hour, _)| *hour != self.hour);
        let enabled = (self.sent.as_ref()).is_none_or(|(_, enabled)| *enabled != self.enabled);
        if hour {
            world.set_hour(self.hour);
        }
        if enabled {
            world.set_enabled(self.enabled.clone());
        }
        if hour || enabled {
            self.sent = Some((self.hour, self.enabled.clone()));
        }
        world.tell(models);
        let latest = world.latest();
        let mut want = told;
        // What a selection made before a jump waits for is not asked for.
        let here = |at: Vec3| at.distance(eye) <= FOCUS_JUMP_DISTANCE;
        if self.wanted_of != latest.version && here(latest.eye) {
            self.wanted_of = latest.version;
            self.wanted.clone_from(&latest.wanted);
            self.bands.clone_from(&latest.bands);
            want = true;
        }
        // What a selection under way waits for is asked for before it is whole, in place of the last whole one's.
        let asked = world.asked();
        if asked.version > latest.version && asked.version != self.asked_of && here(asked.eye) {
            self.asked_of = asked.version;
            self.wanted.clone_from(&asked.wanted);
            self.bands.clone_from(&asked.bands);
            want = true;
        }
        if want {
            self.want_models();
        }
        Some(latest)
    }

    /// After a jump of the camera, ask for nothing and let go of what was read and not taken: it was for the place left.
    // ⚠ The game calls a handler on a jump that was not read: this treatment is inferred.
    fn forget_asks(&mut self) {
        self.jumps += 1;
        (self.wanted, self.species_models) = (Vec::new(), Vec::new());
        self.bands.clear();
        self.in_view.clear();
        let models = self.models.as_ref().map_or(0, Pool::forget);
        let textures = self.textures.as_ref().map_or(0, Pool::forget);
        self.held.fetch_sub(models + textures, Ordering::Relaxed);
    }

    /// Whether the last selection was made for a place the camera has jumped from.
    fn left(&self) -> bool {
        (self.world.as_ref())
            .is_some_and(|w| w.latest().eye.distance(self.camera.position) > FOCUS_JUMP_DISTANCE)
    }

    /// Start the world's thread again in place of one that died, told what the dead one was, and without the maps it took in since its last whole selection: what it died on.
    fn restart_world(&mut self) {
        self.world_restarts += 1;
        let died_on = (self.world.as_ref()).map_or_else(Vec::new, World::unproven);
        match died_on.is_empty() {
            true => {
                self.unexplained += 1;
                tracing::error!(
                    times = self.world_restarts,
                    "the GTA VI world's thread stopped on an internal error and is started again"
                );
            }
            false => {
                let maps: Vec<String> = died_on.iter().map(|m| format!("{m:08X}")).collect();
                tracing::error!(
                    times = self.world_restarts,
                    maps = %maps.join(" "),
                    "the GTA VI world's thread stopped on an internal error with these maps just taken in, and is started again without them"
                );
                self.skipped.extend(died_on);
            }
        }
        let Some(index) = &self.index else {
            return;
        };
        let world = World::new(
            Arc::clone(index),
            self.camera.position,
            self.distance_scale(),
            self.skipped.clone(),
        );
        world.tell(
            &mut self
                .have
                .iter()
                .map(|(model, state)| (*model, *state))
                .collect(),
        );
        self.world = Some(world);
        // The hour and what is enabled are told the new thread again.
        self.sent = None;
    }

    /// Whether the world's thread died too often, on nothing it had just taken in, to be started again: the world stands as it was last selected.
    pub fn world_given_up(&self) -> bool {
        self.unexplained >= MOST_RESTARTS && self.world.as_ref().is_some_and(World::died)
    }
    /// Ask for the models the far stand-ins' atlas waits for, after what the selection waits for.
    pub fn want_species_models(&mut self, models: Vec<(f32, ModelRef)>) {
        if models != self.species_models {
            self.species_models = models;
            self.want_models();
        }
    }

    /// Say which models waited for a frame saw something wait for, in view: they are asked for first.
    pub fn see_models(&mut self, in_view: HashSet<ModelRef>) {
        if in_view != self.in_view {
            self.in_view = in_view;
            self.want_models();
        }
    }

    /// Give the readers every model waited for that is not on the device, highest request priority first.
    fn want_models(&mut self) {
        use or_world::streaming_rules::{
            request_priority, BAND_LOW_DETAIL, BAND_OTHER, BAND_VISIBLE,
        };
        // ⚠ The readers serve the least first: a priority is handed them negated.
        let selected = self.wanted.iter().map(|(away, model)| {
            let band = match self.in_view.contains(model) {
                // ponytail: in view is the frustum alone; the build's second test (band 6 when it fails) is not read, so nothing takes 6.
                true => BAND_VISIBLE,
                false => self.bands.get(model).copied().unwrap_or(BAND_OTHER),
            };
            (-request_priority(band, *away, None), *model)
        });
        // The far stand-ins' atlas waits for far levels' models.
        let species = (self.species_models.iter())
            .map(|(away, model)| (-request_priority(BAND_LOW_DETAIL, *away, None), *model));
        // ⚠ A selection made before a model arrived still waits for it: it is not read twice.
        let left = self.left();
        let wanted: Vec<(f32, ModelRef)> = (selected.chain(species))
            .filter(|(_, model)| !left && !self.have.contains_key(model))
            .collect();
        self.models_waiting = wanted.len();
        self.soon.0 = soonest(wanted.iter().map(|(soon, model)| (*model, *soon)));
        if let Some(models) = &self.models {
            models.want(wanted);
        }
    }

    /// Ask for `wanted` (how soon, key, longest side seen in pixels) in place of what was asked before.
    pub fn want_textures(&mut self, wanted: Vec<(f32, u64, u32)>) {
        // What a frame laid out for the place the camera jumped from asks for is not read.
        let wanted = if self.left() { Vec::new() } else { wanted };
        // ⚠ With no pack indexed nothing reads a texture: none waits, or what waits for none never ends.
        self.textures_waiting = if self.textures.is_some() {
            wanted.len()
        } else {
            0
        };
        self.soon.1 = soonest(wanted.iter().map(|(soon, key, _)| (*key, *soon)));
        if let Some(textures) = &self.textures {
            let asked = wanted.into_iter();
            // ⚠ A frame asks for a texture at the size its distance needs then, which changes as the
            // camera moves: a key being read at one size is not read again at another meanwhile.
            let asked = asked.map(|(soon, key, pixels)| (soon, (key, pixels)));
            textures.want_unheld(asked.collect(), |(key, _)| *key);
        }
    }

    /// Say `model` is on the device though not yet told ready: it is not read again meanwhile.
    pub fn on_device(&mut self, model: ModelRef) {
        self.have.entry(model).or_insert(Some(false));
    }

    /// Look at each model read and not yet taken.
    pub fn each_read(&self, mut look: impl FnMut(&ModelRef, &CpuModel)) {
        if let Some(models) = &self.models {
            models.each_arrived(|model, read| {
                if let Some(read) = read {
                    look(model, read);
                }
            });
        }
    }

    /// The models and the textures read and taken, the seconds they waited from read to taken in all, and the most.
    pub fn reads_waited(&self) -> [(usize, f64, f32); 2] {
        [
            self.models
                .as_ref()
                .map_or_else(Default::default, Pool::waited),
            self.textures
                .as_ref()
                .map_or_else(Default::default, Pool::waited),
        ]
    }

    /// Whether any texture is read: with no pack indexed none is.
    pub fn reads_textures(&self) -> bool {
        self.textures.is_some()
    }

    /// The next texture that arrived, with the pixels asked: `None` where none of it reads, smaller than asked where nothing larger does.
    pub fn take_texture(&mut self) -> Option<(u64, u32, Option<CpuTexture>)> {
        let soon = &self.soon.1;
        let ((key, pixels), texture) = (self.textures.as_ref()?)
            .take_soonest(|(key, _)| soon.get(key).copied().unwrap_or(f32::INFINITY))?;
        self.stale[1] += usize::from(!self.soon.1.contains_key(&key));
        let bytes = texture.as_ref().map_or(0, CpuTexture::bytes);
        self.held.fetch_sub(bytes, Ordering::Relaxed);
        Some((key, pixels, texture))
    }

    /// The next model that arrived, `None` where it did not read.
    pub fn take_model(&mut self) -> Option<(ModelRef, Option<CpuModel>)> {
        let soon = &self.soon.0;
        let (model, read) = (self.models.as_ref()?)
            .take_soonest(|model| soon.get(model).copied().unwrap_or(f32::INFINITY))?;
        self.stale[0] += usize::from(!self.soon.0.contains_key(&model));
        let bytes = read.as_ref().map_or(0, CpuModel::bytes);
        self.held.fetch_sub(bytes, Ordering::Relaxed);
        Some((model, read))
    }

    /// The day the frame is lit by, the lights around the camera, and the exposure a material's own light is given at, at the viewer's hour, exposed by the frame's own luminance where `metered` holds it.
    pub fn sky(
        &mut self,
        days: f32,
        weathers: sky::Weathers,
        metered: Option<&or_renderer::viewport::Metered>,
    ) -> (
        Option<or_renderer::game_shaders::lighting::Day>,
        lights_frame::FrameLights,
        f32,
    ) {
        let started = std::time::Instant::now();
        let (eye, hour) = (self.camera.position, self.hour);
        let tables = self.root.as_deref().and_then(sky::tables);
        let inside = self.inside();
        let moment = (tables.as_ref()).and_then(|tables| {
            // Inside a room, the day takes the room's own modifier over the weather's.
            let room = (inside.as_ref())
                .map(|inside| ([inside.timecycle, inside.secondary_timecycle], inside.blend));
            let moment = sky::moment(tables, hour, weathers, eye.y, room)?;
            Some(sky::exposed(
                tables,
                moment,
                (hour, weathers.target()),
                self.cuts,
                metered,
            ))
        });
        let scale = sky::scale_of(moment.as_ref());
        let table = (tables.as_ref()).and_then(|t| t.executable.variables.as_ref());
        let light_frame = lights::LightFrame {
            vars: (table.zip(moment.as_ref())).map_or_else(Default::default, |(table, m)| {
                lights::LightVars::of(table, &m.own)
            }),
            day_phase: moment.as_ref().map(|m| m.day_phase),
        };
        let day = (tables.as_ref())
            .zip(moment)
            .map(|(tables, moment)| sky::day_of_moment(tables, moment, (hour, days), weathers, eye))
            .map(|mut day| {
                if let (Some(view), Some(root)) = (day.sky_view.as_mut(), self.root.as_deref()) {
                    view.stars = sky_night::stars(root);
                }
                day
            });
        self.day_took = started.elapsed();
        if let (Some(tables), Some(world)) = (&tables, &self.world) {
            world.set_fade(stream::fade_of(&tables.visual_settings));
        }
        let enabled = self.enabled.clone();
        let selected = self.world.as_ref().map(|world| world.latest());
        let rooms = selected.as_ref().map(|s| &s.rooms_seen);
        let lights = self
            .lights
            .as_mut()
            .map_or_else(Default::default, |lights| {
                if let Some(tables) = &tables {
                    lights.set_reach(lights_frame::LodReach::of(&tables.visual_settings));
                    lights.set_distant_reach(lights_distant::DistantReach::of(
                        &tables.visual_settings,
                    ));
                    (lights)
                        .set_corona_reach(lights_corona::CoronaReach::of(&tables.visual_settings));
                }
                lights.set_enabled(&enabled);
                lights.look(eye);
                let view = lights_corona::View {
                    eye,
                    forward: self.camera.forward(),
                    tan_half: (self.camera.fov_y * 0.5).tan(),
                };
                let none = interior::RoomsSeen::new();
                let rooms = rooms.unwrap_or(&none);
                lights.lights(&view, (hour, scale, lights::MOST), rooms, &light_frame)
            });
        // The lights take the exposure over pi; a material's own light is a radiance and takes it whole.
        let exposure = match day {
            Some(_) => scale * std::f32::consts::PI,
            None => 1.0,
        };
        (day, lights, exposure)
    }

    /// The water of the install shown once its quads are read, with the ground under it and the day it is seen in.
    pub fn water(&self, weathers: sky::Weathers) -> Option<water::Seen> {
        let root = self.root.as_deref()?;
        water::seen(root, self.hour, weathers, self.camera.position.y)
    }

    /// The weathers this frame is drawn in, `chosen` the one the Weather bar names: the change towards it stepped on by the frame's time.
    pub fn weathers(&mut self, chosen: crate::state::drivers::WeatherState) -> sky::Weathers {
        self.weathering.step(chosen)
    }

    /// The scale every distance is multiplied by: the person's, and the lens's where it is narrower than the game's.
    fn distance_scale(&self) -> f32 {
        self.lod_scale * or_world::streaming::camera_scale(self.camera.fov_y)
    }
}
