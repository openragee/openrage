//! The GTA VI world viewer: the index of the install it flies through, built on a worker and kept,
//! the maps streamed in around its camera, and the models read for what they draw.
//!
//! ⚠ What is on the device is the render bridge's: a frame asks here what draws and which models
//! arrived, and says which are on the device.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use glam::Vec3;
use or_io::codec::texture::decode_level;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::headerless::{self, Entry, Resource};
use or_io::gta6::TexturePack;
use or_job::{Cancel, Progress};
use or_renderer::{FlyCamera, MeshVertex};

use crate::state::jobs::{JobKind, Outcome, RunningJob, Then};

pub(crate) mod index;
mod pool;
pub(crate) mod stream;

use index::{ModelRef, WorldIndex};
use pool::Pool;
use stream::{Selected, Streaming, World};

/// The bytes a model's body is read to.
const BODY_LIMIT: u64 = 512 << 20;
/// The longest side a texture is drawn at, in pixels: its first level no larger is the one read.
// ponytail: one size for every texture; the level by the distance it is seen from once the memory allows.
const MOST_SIDE: u32 = 256;
/// How high over the middle of the maps the camera starts, in metres.
const START_HEIGHT: f32 = 150.0;

/// One mesh of a model as a worker read it, before it is on the device.
pub struct CpuMesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    /// The key its material's colour map is found by in the texture packs.
    pub texture: Option<u64>,
}

/// A texture as a worker decoded it: 8-bit RGBA levels, the largest first, each half the last.
pub struct CpuTexture {
    pub width: u32,
    pub height: u32,
    pub srgb: bool,
    pub levels: Vec<Vec<u8>>,
}

/// The texture packs of the install, and the pack each key is in.
struct Places {
    packs: Vec<PathBuf>,
    keys: HashMap<u64, u32>,
    /// The format code each key's texture states.
    formats: HashMap<u64, u16>,
}

impl Places {
    /// Whether the texture of `key` states an sRGB format, as a colour map does; `false` for a key no pack holds.
    fn is_colour(&self, key: u64) -> bool {
        let srgb =
            |code: &u16| or_io::codec::texture::DxgiFormat::from_raw((code >> 3).into()).is_srgb();
        self.formats.get(&key).is_some_and(srgb)
    }
}

/// A pack a worker opened, with the place of each key in it; `None` for one that did not open.
type OpenPack = Option<(TexturePack, HashMap<u64, usize>)>;

/// The texture of the key `key`, from the pack that holds it, at its first level no larger than
/// [`MOST_SIDE`] and every level under it; `None` where no pack holds it or it does not read.
fn read_texture(
    open: &mut HashMap<u32, OpenPack>,
    places: &Places,
    key: u64,
) -> Option<CpuTexture> {
    let at = *places.keys.get(&key)?;
    let pack = open.entry(at).or_insert_with(|| {
        let pack = TexturePack::open(places.packs.get(at as usize)?).ok()?;
        let entries = (pack.entries.iter().enumerate())
            .filter(|(_, e)| e.on_disk)
            .map(|(place, e)| (e.key, place))
            .collect();
        Some((pack, entries))
    });
    let (pack, entries) = pack.as_ref()?;
    // ponytail: every row of the texture is read for its small levels; the rows of those levels alone once a pack's read shows.
    let texture = pack.texture(*entries.get(&key)?).ok()?;
    let texture = match texture.faces {
        1 => texture,
        _ => texture.face(0)?,
    };
    let mut level = 0;
    while (texture.width.max(texture.height) >> level) > MOST_SIDE && level + 1 < texture.levels {
        level += 1;
    }
    let top = decode_level(&texture, level).ok()?;
    let (width, height, srgb) = (top.width, top.height, top.is_srgb);
    let mut levels = vec![top.data];
    for under in level + 1..texture.levels {
        let Ok(image) = decode_level(&texture, under) else {
            break;
        };
        let k = levels.len() as u32;
        if (image.width, image.height) != ((width >> k).max(1), (height >> k).max(1)) {
            break;
        }
        levels.push(image.data);
    }
    Some(CpuTexture {
        width,
        height,
        srgb,
        levels,
    })
}

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
    /// The models on the device or known never to come, which are not asked for again.
    have: HashSet<ModelRef>,
    /// The selection whose waited-for models the readers were last given.
    wanted_of: u64,
    models: Option<Pool<ModelRef, Option<Vec<CpuMesh>>>>,
    textures: Option<Pool<u64, Option<CpuTexture>>>,
    build: Option<Build>,
    /// How the last build ended, for the job list to say once.
    ended: Option<(Outcome, String)>,
    /// Why there is no index, where a build failed.
    pub failed: Option<String>,
    pub camera: FlyCamera,
    /// The scale every level-of-detail distance is multiplied by.
    pub lod_scale: f32,
    /// How many models wait to be read.
    pub models_waiting: usize,
    /// How many textures wait to be read.
    pub textures_waiting: usize,
}

impl Default for Viewer {
    fn default() -> Self {
        Self {
            root: None,
            index: None,
            world: None,
            have: HashSet::new(),
            wanted_of: 0,
            models: None,
            textures: None,
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
            lod_scale: 1.0,
            models_waiting: 0,
            textures_waiting: 0,
        }
    }
}

/// The viewer, locked: the screen draws from it and the render bridge asks it what draws.
// ponytail: one viewer for the whole app, of the install it was last opened on.
pub fn viewer() -> MutexGuard<'static, Viewer> {
    static VIEWER: LazyLock<Mutex<Viewer>> = LazyLock::new(Mutex::default);
    VIEWER.lock().unwrap_or_else(|p| p.into_inner())
}

/// The meshes of the model `model` names, read from its archive; `None` where it does not read.
fn read_model(
    files: &mut HashMap<u32, File>,
    index: &WorldIndex,
    places: Option<&Places>,
    model: ModelRef,
) -> Option<Vec<CpuMesh>> {
    let file = match files.entry(model.archive) {
        std::collections::hash_map::Entry::Occupied(open) => open.into_mut(),
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(File::open(index.archives.get(model.archive as usize)?).ok()?)
        }
    };
    let (find, bytes) = read_rpf9_body(&*file, model.offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let read = headerless::read(signature, virtual_section, Some(physical)).ok()?;
    let drawable = match (&read, model.entry) {
        (Resource::Drawable(d), None) => d,
        (Resource::DrawableWithBound(r), None) => &r.drawable,
        (Resource::Dictionary(dictionary), Some(code)) => {
            (dictionary.entries.iter()).find_map(|(c, e)| match e {
                Ok(Entry::Drawable(d)) if *c == code => Some(&**d),
                _ => None,
            })?
        }
        _ => return None,
    };
    // ponytail: the first level of detail of every model; its own lower levels by distance once the memory shows.
    // With the packs indexed a key is a colour map or it is not; with none nothing is known of it.
    let srgb = |key: u64| places.map(|places| places.is_colour(key));
    let meshes = crate::render::gta6_drawable::meshes_at(drawable, 0, None, None, &srgb);
    Some(
        (meshes.into_iter())
            .filter(|m| !m.vertices.is_empty() && !m.indices.is_empty())
            .map(|m| CpuMesh {
                vertices: m.vertices,
                indices: m.indices,
                // A normal map is no colour: a material with that alone draws plain.
                texture: (m.texture_name.as_deref())
                    .filter(|_| !m.bump)
                    .and_then(crate::state::gta6_texture_index::key_of),
            })
            .collect(),
    )
}

impl Viewer {
    /// Show the install at `root`: the index kept of it where one stands, else one built on a worker.
    pub fn open(&mut self, root: &Path) {
        if self.root.as_deref() == Some(root) {
            return;
        }
        *self = Self {
            root: Some(root.to_path_buf()),
            camera: self.camera.clone(),
            lod_scale: self.lod_scale,
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
        self.have.clear();
        self.textures = None;
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
            let kept =
                crate::state::prefs::config_root().map(|config| index::kept_at(&config, &root));
            let held = (kept.as_ref().filter(|_| !afresh)).and_then(|file| WorldIndex::kept(file));
            let places = |cancel: &Cancel| {
                let (packs, keys, formats) = places(cancel, &Progress::new())?;
                Some(Places {
                    packs,
                    keys,
                    formats,
                })
            };
            let built = match held {
                Some(index) => Built::Index(Box::new(index), places(&cancel)),
                None => {
                    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        index::build(&root, &cancel, &progress)
                    }));
                    match built {
                        Ok(Ok(Some(index))) => {
                            if let Some(Err(error)) = kept.map(|file| index.keep(&file)) {
                                tracing::warn!(%error, "the world index was not kept");
                            }
                            Built::Index(Box::new(index), places(&cancel))
                        }
                        Ok(Ok(None)) => Built::Stopped,
                        Ok(Err(why)) => Built::Failed(why),
                        Err(_) => Built::Failed(tr!("It stopped on an internal error").to_string()),
                    }
                }
            };
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(built);
        });
        self.build = Some(build);
    }

    fn take(&mut self, index: WorldIndex, places: Option<Places>) {
        let places = places.map(Arc::new);
        if let Some(places) = places.clone() {
            self.textures = Some(Pool::new(
                4,
                HashMap::<u32, OpenPack>::new,
                move |open, key| read_texture(open, &places, key),
            ));
        }
        if let Some(middle) = index.middle() {
            self.camera.position = middle + Vec3::Z * START_HEIGHT;
        }
        let index = Arc::new(index);
        let shared = Arc::clone(&index);
        let workers = std::thread::available_parallelism().map_or(4, usize::from) / 2;
        self.models = Some(Pool::new(
            workers.clamp(2, 12),
            HashMap::<u32, File>::new,
            move |files, model| read_model(files, &shared, places.as_deref(), model),
        ));
        let (eye, scale) = (self.camera.position, self.lod_scale);
        self.world = Some(World::new(Arc::clone(&index), eye, scale));
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
                    maps = index.report.at_rest,
                    archetypes = index.report.archetypes,
                    models = index.report.models,
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
            kind: JobKind::ArchetypeSearch,
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

    /// What was selected last for the camera, after the world's thread is told where the camera
    /// is and which models came to the device, will never come (`None`) or left it (`false`).
    pub fn selected(
        &mut self,
        models: &mut Vec<(ModelRef, Option<bool>)>,
    ) -> Option<Arc<Selected>> {
        let world = self.world.as_ref()?;
        for (model, state) in models.iter() {
            match state {
                Some(false) => self.have.remove(model),
                _ => self.have.insert(*model),
            };
        }
        let told = !models.is_empty();
        world.look(self.camera.position, self.lod_scale);
        world.tell(models);
        let latest = world.latest();
        if told || self.wanted_of != latest.version {
            self.wanted_of = latest.version;
            // ⚠ A selection made before a model arrived still waits for it: it is not read twice.
            let wanted: Vec<(f32, ModelRef)> = (latest.wanted.iter().copied())
                .filter(|(_, model)| !self.have.contains(model))
                .collect();
            self.models_waiting = wanted.len();
            if let Some(models) = &self.models {
                models.want(wanted);
            }
        }
        Some(latest)
    }

    /// Ask for the textures of `wanted`, each with how far the nearest thing it colours is.
    pub fn want_textures(&mut self, wanted: Vec<(f32, u64)>) {
        self.textures_waiting = wanted.len();
        if let Some(textures) = &self.textures {
            textures.want(wanted);
        }
    }

    /// The next texture that arrived, `None` where no pack holds it or it did not read.
    pub fn take_texture(&mut self) -> Option<(u64, Option<CpuTexture>)> {
        let mut first = true;
        (self.textures.as_ref()?)
            .take(|| std::mem::take(&mut first))
            .pop()
    }

    /// The next model that arrived, its meshes `None` where it did not read.
    pub fn take_model(&mut self) -> Option<(ModelRef, Option<Vec<CpuMesh>>)> {
        let mut first = true;
        (self.models.as_ref()?)
            .take(|| std::mem::take(&mut first))
            .pop()
    }
}
