//! GTA VI's programs translated for Shaded: the effect containers the app has read, the vertex
//! and pixel programs a drawable's effects run, read from their archive by seek and translated on
//! a worker once a session, and the pack of what is done so far, held in memory and never written.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use or_cli::{
    gta6_drawing_pass_at, gta6_held_streams, gta6_view_passes, translate_gta6_program, Gta6Program,
    Gta6ShadingPass,
};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Scan};
use or_io::gta5::shaders::sgd5::{Sgd5Effect, Sgd5File, Sgd5Program, Sgd5Table};
use or_io::gta5::shaders::sgd5_pack::{pack_effect, vertex_pixel_passes};
use or_io::gta5::shaders::spirv_pack::{
    Effect, NagaReach, Outcome, Pass, Production, Program, Reach, Reflection, ShaderStageKind,
    SpirvPack,
};
use or_io::gta6::hash::name_hash32;
use or_job::{Cancel, Progress};
use or_renderer::game_shaders::pack::GamePack;

use super::jobs::{JobKind, JobLog, Outcome as Ended, RunningJob, Then};

/// The bytes one effect container is read to while an install's archive is searched for them.
const CONTAINER_LIMIT: u64 = 1 << 30;
/// The bytes a program's stream may hold before it is left unread.
const MAX_STREAM: usize = 64 << 20;

/// Where an effect container's programs are read from: the archive, the container's offset in
/// it, and the container as read, its programs located rather than copied.
struct ShaderSource {
    archive: PathBuf,
    base: u64,
    file: Sgd5File,
    /// Whether it is the container as its archive holds it, and not one effect of it kept by a
    /// session before this one.
    whole: bool,
}

/// A program by where it lies: its archive, its container's offset and its offset in that.
type Place = (PathBuf, u64, usize);

/// A program read: its key, and whether its inputs read blend indices.
type Keyed = (String, bool);

/// One pass that shades a model with an effect, as far as it is known: where its mesh and its
/// pixel program lie in their container, the streams the mesh program reads, and the pass once a
/// geometry picked it, or why the renderer cannot be handed it.
#[derive(Clone)]
pub(crate) struct Candidate {
    at: (usize, usize),
    streams: BTreeSet<u32>,
    pass: Option<Result<Arc<Gta6ShadingPass>, String>>,
}

/// The passes that shade a model with one effect, one for each mesh program its techniques run:
/// a geometry is drawn by the one that reads the streams it has, and only that one is translated.
pub(crate) type Passes = Vec<Candidate>;

/// The pass a geometry is drawn by, or why the renderer cannot be handed one; `None` before a job
/// has translated it.
pub(crate) type Shading = Option<Result<Arc<Gta6ShadingPass>, String>>;

/// An effect by the hash of its name, with the streams a geometry that wears it holds
/// ([`gta6_held_streams`]): what a pass is picked by.
pub(crate) type Worn = (u32, u32);

/// The stages of the session's passes by what they hold: two effects whose passes come to the
/// same two modules hold the same two, and the device builds one pass for both.
type Stages = Arc<Mutex<HashMap<[u64; 2], Arc<wgpu::naga::Module>>>>;

/// Put the session's stage that holds what `module` holds in its place, or keep `module` as that
/// stage; whether the session held one.
fn share(stages: &Stages, module: &mut Arc<wgpu::naga::Module>) -> bool {
    let held = digest(module);
    let mut stages = stages.lock().unwrap_or_else(|p| p.into_inner());
    let known = stages.contains_key(&held);
    *module = Arc::clone(stages.entry(held).or_insert_with(|| Arc::clone(module)));
    known
}

/// Two hashes of `module` as it prints, which is all it holds: a stage the binder or one of its
/// options changed prints otherwise.
fn digest(module: &wgpu::naga::Module) -> [u64; 2] {
    use std::hash::Hasher;
    struct Both([std::collections::hash_map::DefaultHasher; 2]);
    impl std::fmt::Write for Both {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0
                .iter_mut()
                .for_each(|hash| hash.write(text.as_bytes()));
            Ok(())
        }
    }
    let mut both = Both(Default::default());
    // The second starts from another state, so that the two do not meet together by chance.
    both.0[1].write_u8(1);
    let _ = std::fmt::Write::write_fmt(&mut both, format_args!("{module:?}"));
    both.0.map(|hash| hash.finish())
}

/// A program translated or refused, with the stage its pass runs it at.
struct Done {
    stage: ShaderStageKind,
    program: Gta6Program,
}

#[derive(Default)]
struct Shaders {
    sources: Vec<Arc<ShaderSource>>,
    /// The install archives searched for containers this session.
    searched: HashSet<PathBuf>,
    keys: HashMap<Place, Keyed>,
    /// By key: each program is translated once a session, whatever passes run it.
    done: HashMap<String, Arc<Done>>,
    /// Each translated module of `done`, shared by every pack built of them rather than copied.
    modules: HashMap<String, Arc<wgpu::naga::Module>>,
    /// Each effect's manifest entry, by the hash of its name.
    effects: HashMap<u32, Effect>,
    /// The pass that shades a model with each effect, by the hash of the effect's name, or why
    /// the renderer cannot be handed it.
    shading: HashMap<u32, Result<Passes, String>>,
    /// What a job has looked for since the containers last changed, found or not.
    tried: HashSet<Worn>,
    stages: Stages,
    running: Option<Running>,
    /// The pack of what is done, numbered so that a new one is told from the last.
    pack: Option<(u64, Arc<GamePack>)>,
}

struct Running {
    wanted: Vec<Worn>,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Result<Found, String>>>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// What a job is given: the containers known, the programs read so far, the effects wanted and
/// the install archive to search where no container holds one.
struct Input {
    sources: Vec<Arc<ShaderSource>>,
    keys: HashMap<Place, Keyed>,
    known: HashSet<String>,
    wanted: Vec<Worn>,
    /// What jobs before this one found of the wanted effects' passes.
    shading: HashMap<u32, Result<Passes, String>>,
    stages: Stages,
    install: Option<PathBuf>,
}

/// What a job found: the containers its search added and the archive it searched, the programs
/// it read and translated, and the effects it built of them.
#[derive(Default)]
struct Found {
    sources: Vec<Arc<ShaderSource>>,
    searched: Option<PathBuf>,
    keys: Vec<(Place, Keyed)>,
    done: Vec<(String, Done)>,
    effects: Vec<(u32, Effect)>,
    shading: Vec<(u32, Result<Passes, String>)>,
}

/// Where the drawable's effects stand, for the Shaded button to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    /// No container read holds any of them.
    NoSource,
    Translating,
    /// How many programs of their passes are refused, and why the first is.
    Done {
        refused: usize,
        first: Option<String>,
    },
}

// ponytail: one session's translations for the whole app, as the effects database is.
static SHADERS: Mutex<Option<Shaders>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Shaders) -> T) -> T {
    let mut held = SHADERS.lock().unwrap_or_else(|p| p.into_inner());
    f(held.get_or_insert_with(Shaders::default))
}

fn stopped() -> String {
    tr!("Stopped before it finished").to_string()
}

/// Keep the `SGD5` container `bytes`, read at `base` of `archive`, as a place programs are read from.
// ponytail: opens the container a second time beside the effects read; one open when it shows.
pub(super) fn add_source(archive: &Path, base: u64, bytes: &[u8]) {
    let Ok(file) = Sgd5File::open(bytes) else {
        return;
    };
    let source = ShaderSource {
        archive: archive.to_path_buf(),
        base,
        file,
        whole: true,
    };
    with(|s| keep(s, vec![Arc::new(source)]));
}

fn keep(s: &mut Shaders, sources: Vec<Arc<ShaderSource>>) {
    if sources.is_empty() {
        return;
    }
    for source in sources {
        s.sources.retain(|kept| {
            (kept.archive.as_path(), kept.base) != (source.archive.as_path(), source.base)
        });
        s.sources.push(source);
    }
    // An effect not found before may be in a container new to the app.
    let effects = &s.effects;
    s.tried.retain(|(hash, _)| effects.contains_key(hash));
}

/// Start translating, on a worker, the pass that draws each geometry of `worn` and the vertex
/// and pixel passes of an effect with none, unless each was looked for already; a job running
/// for others is stopped. `install` is the GTA VI install's `shaders.rpf`, asked for when a job
/// starts and searched once a session where no container read holds one.
pub(crate) fn request(worn: &[Worn], install: impl FnOnce() -> Option<PathBuf>) {
    with(|s| {
        if let Some((input, wanted)) = asked(s, worn, install) {
            s.running = Some(Running::start(input, wanted));
        }
    });
}

/// The job a request for `worn` starts, with what it is given; none where one runs for the same,
/// or each was looked for already.
// ⚠ Looked for is an effect with the streams of a geometry, never the effect alone: a second
// geometry that wears it with other streams draws by another pass, which no job translated.
fn asked(
    s: &Shaders,
    worn: &[Worn],
    install: impl FnOnce() -> Option<PathBuf>,
) -> Option<(Input, Vec<Worn>)> {
    let mut wanted = worn.to_vec();
    wanted.sort_unstable();
    wanted.dedup();
    let running = s.running.as_ref().is_some_and(|r| r.wanted == wanted);
    if running || wanted.iter().all(|h| s.tried.contains(h)) {
        return None;
    }
    Some((input_of(s, &wanted, install), wanted))
}

/// What a job for `wanted` is given of the session's translations.
fn input_of(s: &Shaders, wanted: &[Worn], install: impl FnOnce() -> Option<PathBuf>) -> Input {
    // What a job has looked for is not translated again beside what none has.
    let fresh: Vec<Worn> = (wanted.iter().copied())
        .filter(|worn| !s.tried.contains(worn))
        .collect();
    let shading = (fresh.iter())
        .filter_map(|(hash, _)| Some((*hash, s.shading.get(hash)?.clone())))
        .collect();
    Input {
        sources: s.sources.clone(),
        keys: s.keys.clone(),
        known: s.done.keys().cloned().collect(),
        wanted: fresh,
        shading,
        stages: Arc::clone(&s.stages),
        install: install().filter(|path| !s.searched.contains(path)),
    }
}

impl Running {
    /// Start the job of `input` for a request of the effects `wanted`.
    fn start(input: Input, wanted: Vec<Worn>) -> Self {
        let running = Self {
            wanted,
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            running.cancel.clone(),
            running.progress.clone(),
            Arc::clone(&running.ended),
        );
        if let Some(folder) = kept_folder() {
            or_cli::gta6_kept::keep_translations_in(folder.join("translations"));
        }
        std::thread::spawn(move || {
            // A panic still ends the job, or the Shaded button says Translating for ever.
            let started = std::time::Instant::now();
            let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(input, &cancel, &progress)
            }))
            .unwrap_or_else(|_| Err(tr!("It stopped on an internal error").to_string()));
            tracing::info!(elapsed = ?started.elapsed(), "GTA VI's programs for Shaded were read and translated");
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(found);
        });
        running
    }
}

/// The folder a session keeps what it read and translated of GTA VI's programs in, for the next.
fn kept_folder() -> Option<PathBuf> {
    Some(super::prefs::config_root()?.join("gta6_shaders"))
}

/// The file the effect whose name hashes to `hash` is kept in as `archive` holds it now: by the
/// archive's path, its size and when it was last written.
fn kept_effect(archive: &Path, hash: u32) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let stat = super::gta6_texture_index::stat(archive)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (archive, stat).hash(&mut hasher);
    let folder = format!("{:016x}", hasher.finish());
    Some(
        kept_folder()?
            .join(folder)
            .join(format!("{hash:08x}.effect")),
    )
}

/// The effect a session before this one kept in `kept`, as a container of that one effect.
fn read_kept(archive: &Path, kept: &Path) -> Option<Arc<ShaderSource>> {
    let bytes = std::fs::read(kept).ok()?;
    let (base, file) = (or_cli::gta6_kept::decode_containers(&bytes)?.into_iter()).next()?;
    Some(Arc::new(ShaderSource {
        archive: archive.to_path_buf(),
        base,
        file,
        whole: false,
    }))
}

/// Keep `effect` of `source` in `kept` for the next session: a container of the one effect and
/// the programs its passes run, which reads in a moment where the whole container takes seconds.
fn keep_effect(source: &ShaderSource, effect: &Sgd5Effect, kept: &Path) {
    let listed = |program: &&Sgd5Program| {
        let (Some(bank), Some(index)) = (program.bank, program.index) else {
            return false;
        };
        (effect.programs.get(bank)).is_some_and(|list| list.contains(&index))
    };
    let file = Sgd5File {
        head_word: source.file.head_word,
        layouts: Vec::new(),
        globals: Sgd5Table::default(),
        bank_counts: source.file.bank_counts,
        programs: source
            .file
            .programs
            .iter()
            .filter(listed)
            .cloned()
            .collect(),
        gaps: Vec::new(),
        effect_count: None,
        effects: vec![effect.clone()],
    };
    // Beside its place, then moved there: a session that reads it never sees half a file.
    let partial = kept.with_extension(format!("{}.part", std::process::id()));
    let bytes = or_cli::gta6_kept::encode_containers(&[(source.base, &file)]);
    let written = (kept.parent().map_or(Ok(()), std::fs::create_dir_all))
        .and_then(|()| std::fs::write(&partial, bytes))
        .and_then(|()| std::fs::rename(&partial, kept));
    if let Err(error) = written {
        tracing::debug!(%error, "a GTA VI effect is not kept for the next session");
    }
}

/// The effect of `source` whose name hashes to `hash`.
fn effect_of(source: &ShaderSource, hash: u32) -> Option<&Sgd5Effect> {
    (source.file.effects.iter()).find(|e| name_hash32(e.name.as_bytes()) == hash)
}

/// Each program the wanted effects' vertex and pixel passes run, read and translated unless read
/// before, and each effect built of the passes whose two programs read.
fn work(input: Input, cancel: &Cancel, progress: &Progress) -> Result<Found, String> {
    work_with(&Translator, input, cancel, progress)
}

/// [`work`], its shading passes read by `translate`.
fn work_with(
    translate: &impl Translate,
    input: Input,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Found, String> {
    let Input {
        mut sources,
        mut keys,
        mut known,
        wanted,
        mut shading,
        stages,
        install,
    } = input;
    // Each effect wanted, with the streams the geometries that wear it hold.
    let mut worn: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (hash, streams) in wanted {
        worn.entry(hash).or_default().push(streams);
    }
    let wanted: Vec<u32> = worn.keys().copied().collect();
    let mut found = Found::default();
    let held = |sources: &[Arc<ShaderSource>], hash: u32| {
        sources.iter().any(|s| effect_of(s, hash).is_some())
    };
    // What a session before this one kept of the install's effects stands in for its containers.
    let lacking: Vec<(u32, PathBuf)> = (wanted.iter())
        .filter(|hash| !held(&sources, **hash))
        .filter_map(|hash| Some((*hash, kept_effect(install.as_deref()?, *hash)?)))
        .collect();
    if let Some(archive) = &install {
        let kept = lacking
            .iter()
            .filter_map(|(_, kept)| read_kept(archive, kept));
        sources.extend(kept);
    }
    let missing = wanted.iter().any(|hash| !held(&sources, *hash));
    if let Some(archive) = install.filter(|_| missing) {
        match search(&archive, cancel, progress) {
            Ok(more) => found.sources = more,
            Err(_) if cancel.is_set() => return Err(stopped()),
            Err(why) => {
                tracing::warn!(archive = %archive.display(), %why, "GTA VI's effect containers are not searched");
            }
        }
        found.searched = Some(archive);
        sources.extend(found.sources.iter().cloned());
    }
    // An effect found to have no pass of its own was given its vertex and pixel passes then.
    let effects: Vec<(u32, &ShaderSource, &Sgd5Effect, Option<Passes>)> = (wanted.iter())
        .filter_map(|h| {
            let before = match shading.remove(h) {
                Some(Err(_)) => return None,
                Some(Ok(passes)) => Some(passes),
                None => None,
            };
            sources
                .iter()
                .find_map(|s| Some((*h, &**s, effect_of(s, *h)?, before.clone())))
        })
        .collect();
    let total: usize = (effects.iter())
        .map(|(_, s, e, _)| 2 * vertex_pixel_passes(&s.file, e).len())
        .sum();
    // The effects side by side: each is its own programs, and a worker's files are its own.
    // A worker an effect leaves once none is left to start is another's to read passes on.
    let workers = Workers {
        cancel,
        spare: AtomicUsize::new(or_job::workers().saturating_sub(effects.len())),
    };
    let begun = AtomicUsize::new(0);
    let looked = or_job::side_by_side(
        &effects,
        or_job::workers(),
        cancel,
        |(hash, source, effect, before)| {
            begun.fetch_add(1, Ordering::SeqCst);
            // An effect read from a whole container is kept for the next session, once.
            let kept = Some(*source)
                .filter(|source| source.whole)
                .and_then(|source| kept_effect(&source.archive, *hash))
                .filter(|kept| !kept.exists());
            if let Some(kept) = kept {
                keep_effect(source, effect, &kept);
            }
            let started = std::time::Instant::now();
            let (shading, did) = shading_passes(
                translate,
                source,
                effect,
                &worn[hash],
                before.clone(),
                &stages,
                &workers,
            );
            if begun.load(Ordering::SeqCst) == effects.len() {
                workers.give(1);
            }
            tracing::info!(
                effect = %effect.name,
                elapsed = ?started.elapsed(),
                passes = shading.as_ref().map_or(0, Vec::len),
                read = did.read,
                translated = did.translated,
                refused = did.refused,
                drawn = did.drawn,
                shared = did.shared,
                "a GTA VI effect's shading passes were translated"
            );
            shading
        },
    );
    let mut files = HashMap::new();
    let mut at = 0;
    for ((hash, source, effect, before), shading) in effects.iter().zip(looked) {
        let Some(shading) = shading.filter(|_| !cancel.is_set()) else {
            return Err(stopped());
        };
        // An effect a model draws with by its own mesh and pixel programs needs none of its
        // vertex and pixel passes, which are many: they are translated only where it has none.
        let own = shading.is_ok();
        found.shading.push((*hash, shading));
        // Its entry stands as the job that first looked for it made it.
        if own && before.is_some() {
            continue;
        }
        let mut passes = Vec::new();
        let pairs = match own {
            true => Vec::new(),
            false => vertex_pixel_passes(&source.file, effect),
        };
        for (_, vertex, pixel) in pairs {
            let mut stages = BTreeMap::new();
            let mut skinned = false;
            for (program, stage) in [
                (vertex, ShaderStageKind::Vertex),
                (pixel, ShaderStageKind::Pixel),
            ] {
                if cancel.is_set() {
                    return Err(stopped());
                }
                progress.reached(at, total as u64);
                at += 1;
                let place = (source.archive.clone(), source.base, program.at);
                let keyed = match keys.get(&place) {
                    Some(keyed) => keyed.clone(),
                    None => match read(source, program, &effect.parameters, &mut files) {
                        Ok(read) => {
                            let keyed = (read.key.clone(), read.skinned);
                            keys.insert(place.clone(), keyed.clone());
                            found.keys.push((place, keyed.clone()));
                            if known.insert(read.key.clone()) {
                                let done = Done {
                                    stage,
                                    program: read,
                                };
                                found.done.push((keyed.0.clone(), done));
                            }
                            keyed
                        }
                        Err(why) => {
                            tracing::debug!(program = %program.name, %why, "a GTA VI program is not read");
                            break;
                        }
                    },
                };
                if stage == ShaderStageKind::Vertex {
                    skinned = keyed.1;
                }
                stages.insert(stage, keyed.0);
            }
            if stages.len() == 2 {
                passes.push((Pass { stages }, skinned));
            }
        }
        found
            .effects
            .push((*hash, pack_effect(&effect.name, &passes)));
    }
    Ok(found)
}

/// `program`'s stream, read alone from its archive, through the front end its payload takes;
/// `effect` is the first table of the effect that runs it.
// ponytail: a program two effects run keeps the names the first gives it.
fn read(
    source: &ShaderSource,
    program: &Sgd5Program,
    effect: &Sgd5Table,
    files: &mut HashMap<PathBuf, File>,
) -> Result<Gta6Program, String> {
    let stream = stream(source, program, files)?;
    translate_gta6_program(program, &stream, Some(effect))
}

/// What a job did for one effect: the mesh programs it read the streams of, the passes it
/// translated whole and how many of those were refused, the passes its geometries draw by, and
/// the stages of its passes the session held already.
#[derive(Debug, Default, PartialEq, Eq)]
struct Did {
    read: usize,
    translated: usize,
    refused: usize,
    drawn: usize,
    shared: usize,
}

/// The passes that shade a model with `effect`: each technique whose first pass is a mesh
/// program no earlier one ran and a pixel program that writes the targets whose contents are
/// established, with the streams its mesh program reads; of those, the pass that draws a geometry
/// holding each of the streams `held` is translated, and no other. `before` is what an earlier
/// job found of them. `Err` says why the renderer cannot be handed any.
fn shading_passes<T: Translate>(
    translate: &T,
    source: &ShaderSource,
    effect: &Sgd5Effect,
    held: &[u32],
    before: Option<Passes>,
    stages: &Stages,
    workers: &Workers<'_>,
) -> (Result<Passes, String>, Did) {
    let cancel = workers.cancel;
    let mut did = Did::default();
    let mut files = HashMap::new();
    let mut container = |program: &Sgd5Program| {
        let stream = stream(source, program, &mut files)?;
        program.inflate(&stream).map_err(|e| e.to_string())
    };
    let parameters = &effect.parameters;
    let listed = gta6_view_passes(&source.file, effect);
    let techniques = or_cli::gta6_view_techniques(&source.file, effect);
    let pair_at = |at: (usize, usize)| {
        (listed.iter().copied()).find(|(first, pixel)| (first.at, pixel.at) == at)
    };
    // The two containers of each pass, inflated when it is first asked about.
    let mut inflated: HashMap<(usize, usize), (Vec<u8>, Vec<u8>)> = HashMap::new();
    // What an earlier job found stands while its container still lists every pass of it.
    let before = before.and_then(|passes| {
        let pairs: Option<Vec<_>> = passes.iter().map(|pass| pair_at(pass.at)).collect();
        Some((passes, pairs?))
    });
    let (mut passes, pairs) = match before {
        Some(found) => found,
        None => {
            let mut seen = HashSet::new();
            let (mut passes, mut pairs) = (Vec::new(), Vec::new());
            // A forward pass is taken only where no technique has a pass that shades a model.
            for forward in [false, true] {
                if forward && !passes.is_empty() {
                    break;
                }
                let mut taken = Vec::new();
                for (first, pixel) in &listed {
                    if seen.contains(&first.at) {
                        continue;
                    }
                    let (Ok(mesh), Ok(fragment)) = (container(first), container(pixel)) else {
                        continue;
                    };
                    let shades = match forward {
                        false => translate.shades(&mesh, &fragment),
                        true => translate.forward(&mesh, &fragment),
                    };
                    if shades {
                        seen.insert(first.at);
                        taken.push((*first, *pixel, mesh, fragment));
                    }
                }
                // Each mesh program is lowered to read its streams: side by side, on the workers
                // the job spares.
                let more = workers.take(taken.len().saturating_sub(1));
                let read = or_job::side_by_side(&taken, 1 + more, cancel, |taken| {
                    translate.streams(&taken.2, &taken.0.variables, parameters)
                });
                workers.give(more);
                for ((first, pixel, mesh, fragment), streams) in taken.into_iter().zip(read) {
                    did.read += 1;
                    let (streams, pass) = match streams.unwrap_or_else(|| Err(stopped())) {
                        Ok(streams) => (streams, None),
                        Err(why) => (BTreeSet::new(), Some(Err(why))),
                    };
                    let at = (first.at, pixel.at);
                    inflated.insert(at, (mesh, fragment));
                    passes.push(Candidate { at, streams, pass });
                    pairs.push((first, pixel));
                }
            }
            (passes, pairs)
        }
    };
    // Several passes of an effect run one pixel program: it is translated once.
    let mut pixels = HashMap::new();
    type Pair<'f> = (&'f Sgd5Program, &'f Sgd5Program);
    let mut whole = |pass: &mut Candidate, (first, pixel): Pair<'_>, did: &mut Did| {
        let read = match inflated.entry(pass.at) {
            Entry::Occupied(read) => Ok(read.into_mut()),
            Entry::Vacant(slot) => match (container(first), container(pixel)) {
                (Ok(mesh), Ok(fragment)) => Ok(slot.insert((mesh, fragment))),
                (Err(why), _) | (_, Err(why)) => Err(why),
            },
        };
        let translated = read.and_then(|(mesh, fragment)| {
            let stage = pixels
                .entry(pixel.at)
                .or_insert_with(|| translate.pixel(fragment, &pixel.variables, parameters));
            let stage = stage.as_ref().map_err(|why: &String| why.clone())?;
            translate.mesh(mesh, &first.variables, parameters, stage)
        });
        // A forward pass takes its technique's state, and its technique's later passes.
        let translated = translated.and_then(|translated| {
            match forward_passes::technique_at(&techniques, pass.at) {
                Some(its) => forward_passes::in_technique(
                    translate,
                    translated,
                    its,
                    parameters,
                    &mut container,
                ),
                None => Ok(translated),
            }
        });
        did.translated += 1;
        did.refused += usize::from(translated.is_err());
        let translated = translated.map(|mut translated| {
            did.shared += usize::from(share(stages, &mut translated.mesh.module));
            did.shared += usize::from(share(stages, &mut translated.pixel.module));
            Arc::new(translated)
        });
        pass.pass = Some(translated);
    };
    // ⚠ A pass is picked by the streams its mesh program was read to read before its pixel
    // program said which slots cross as halves. Were the pass as translated to read others,
    // every pass is translated and picked among as translated.
    let mut every = false;
    let mut drawn = BTreeSet::new();
    for streams in held {
        loop {
            if cancel.is_set() {
                return (Err(stopped()), did);
            }
            if every {
                for (pass, pair) in passes.iter_mut().zip(&pairs) {
                    if pass.pass.is_none() {
                        whole(pass, *pair, &mut did);
                    }
                }
                for pass in &mut passes {
                    if let Some(Ok(translated)) = &pass.pass {
                        pass.streams = translated.mesh.streams.clone();
                    }
                }
            }
            let Some(picked) = pick(&passes, *streams) else {
                break;
            };
            if passes[picked].pass.is_some() {
                drawn.insert(picked);
                break;
            }
            whole(&mut passes[picked], pairs[picked], &mut did);
            let pass = &passes[picked];
            let other = matches!(&pass.pass, Some(Ok(whole)) if whole.mesh.streams != pass.streams);
            if other && !every {
                tracing::warn!(effect = %effect.name, "a GTA VI mesh program reads other streams as its pass's stage than alone");
                every = true;
            }
        }
    }
    did.drawn = drawn.len();
    let refusal = |pass: &Candidate| match &pass.pass {
        Some(Err(why)) => Some(why.clone()),
        _ => None,
    };
    if passes.iter().all(|pass| refusal(pass).is_some()) {
        let why = passes.iter().find_map(refusal).unwrap_or_else(|| {
            tr!("none of its techniques is a mesh pass that writes the deferred targets")
                .to_string()
        });
        return (Err(why), did);
    }
    (Ok(passes), did)
}

/// The place among `passes` of the one that draws a geometry holding `streams`, of those the
/// renderer can be handed or may yet be.
fn pick(passes: &[Candidate], streams: u32) -> Option<usize> {
    let open: Vec<usize> = (0..passes.len())
        .filter(|at| !matches!(passes[*at].pass, Some(Err(_))))
        .collect();
    let read: Vec<&BTreeSet<u32>> = open.iter().map(|at| &passes[*at].streams).collect();
    Some(open[gta6_drawing_pass_at(&read, streams)?])
}

/// `program`'s stream, read alone from its archive.
fn stream(
    source: &ShaderSource,
    program: &Sgd5Program,
    files: &mut HashMap<PathBuf, File>,
) -> Result<Vec<u8>, String> {
    let len = program.stream.len();
    if len > MAX_STREAM {
        return Err(format!(
            "a {len}-byte stream is past the {MAX_STREAM} bytes one is read to"
        ));
    }
    let at = (source.base)
        .checked_add(program.stream.start as u64)
        .ok_or_else(|| "the stream lies past any archive's end".to_string())?;
    let file = match files.entry(source.archive.clone()) {
        Entry::Occupied(open) => open.into_mut(),
        Entry::Vacant(slot) => slot.insert(
            File::open(&source.archive)
                .map_err(|e| format!("{}: {e}", source.archive.display()))?,
        ),
    };
    file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
    let mut stream = vec![0; len];
    file.read_exact(&mut stream).map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Every `SGD5` container a scan of `archive` finds, its place the job's progress; one that does
/// not read is left out.
fn search(
    archive: &Path,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Vec<Arc<ShaderSource>>, String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut scan = Rpf9Scan::new(&file, 0..u64::MAX).map_err(|e| e.to_string())?;
    let len = scan.file_len();
    let mut found = Vec::new();
    while let Some(find) = scan.next() {
        if cancel.is_set() {
            return Err(stopped());
        }
        progress.reached(scan.position().min(len), len);
        let find = find.map_err(|e| e.to_string())?;
        if !matches!(find.kind, Rpf9BodyKind::Sgd5 { .. }) {
            continue;
        }
        let (_, bytes) =
            read_rpf9_body(&file, find.offset, CONTAINER_LIMIT).map_err(|e| e.to_string())?;
        match Sgd5File::open(&bytes) {
            Ok(read) => found.push(Arc::new(ShaderSource {
                archive: archive.to_path_buf(),
                base: find.offset,
                file: read,
                whole: true,
            })),
            Err(e) => tracing::warn!(offset = find.offset, "an effect container is not read: {e}"),
        }
    }
    Ok(found)
}

/// Take what a job that ended found into the session's translations, build their pack, and say
/// in the job list how it went.
pub(super) fn poll(jobs: &mut JobLog) {
    let said = with(|s| {
        let result = (s.running.as_ref()?.ended.lock())
            .unwrap_or_else(|p| p.into_inner())
            .take()?;
        let running = s.running.take()?;
        let said = match took(s, &running.wanted, result) {
            Ok((translated, refused)) => {
                let said = tr!(
                    "GTA VI's programs for Shaded: translated {translated}, refused {refused}",
                    translated = translated,
                    refused = refused,
                );
                (Ended::Done(Then::Nothing), said)
            }
            Err(_) if running.cancel.is_set() => (Ended::Stopped(Then::Nothing), stopped()),
            Err(why) => (
                Ended::Failed,
                tr!("GTA VI's programs were not translated: {why}", why = why),
            ),
        };
        Some(said)
    });
    if let Some((outcome, said)) = said {
        jobs.record(JobKind::EffectsRead, outcome, said);
    }
}

/// Take what the job for `wanted` came to into the session's translations: how many programs it
/// translated and refused.
fn took(
    s: &mut Shaders,
    wanted: &[Worn],
    found: Result<Found, String>,
) -> Result<(usize, usize), String> {
    let merged = found.map(|found| merge(s, found));
    // A stopped or failed job is not started again for the same effects.
    s.tried.extend(wanted.iter().copied());
    merged
}

/// `found` kept, and the pack built anew: how many programs it translated and refused.
fn merge(s: &mut Shaders, found: Found) -> (usize, usize) {
    if let Some(archive) = found.searched {
        s.searched.insert(archive);
    }
    keep(s, found.sources);
    let (mut translated, mut refused) = (0, 0);
    for (key, done) in found.done {
        match &done.program.translated {
            Ok(_) => translated += 1,
            Err(_) => refused += 1,
        }
        if let (Entry::Vacant(slot), Ok(t)) =
            (s.modules.entry(key.clone()), &done.program.translated)
        {
            slot.insert(Arc::new(t.module.clone()));
        }
        s.done.entry(key).or_insert_with(|| Arc::new(done));
    }
    s.keys.extend(found.keys);
    s.effects.extend(found.effects);
    for (hash, pass) in found.shading {
        s.shading.insert(hash, pass);
    }
    let number = s.pack.as_ref().map_or(1, |(number, _)| number + 1);
    s.pack = Some((number, Arc::new(pack_of(s))));
    (translated, refused)
}

/// The pack of every effect and program done: each translated module by its key, each refused
/// program with its reason, each constant buffer layout by its program's key and place.
fn pack_of(s: &Shaders) -> GamePack {
    let mut manifest = SpirvPack::new(Production {
        translator: or_cli::DXIL_FRONT.to_string(),
        legalizer: String::new(),
    });
    let mut modules = HashMap::new();
    for (key, done) in &s.done {
        let (outcome, reflection) = match &done.program.translated {
            Ok(t) => {
                let mut reflection = t.reflection.clone();
                for (i, buffer) in t.constant_buffers.iter().enumerate() {
                    let at = format!("{key}.{i}");
                    reflection.constant_buffers.push(at.clone());
                    manifest.constant_buffers.insert(at, buffer.clone());
                }
                if let Some(module) = s.modules.get(key) {
                    modules.insert(key.clone(), Arc::clone(module));
                }
                let outcome = Outcome::Translated {
                    spirv_bytes: 0,
                    fidelity: t.fidelity,
                    lowerings: t.lowerings.clone(),
                    naga: NagaReach {
                        reached: Reach::Validated,
                        cause: None,
                    },
                };
                (outcome, reflection)
            }
            Err(refusal) => (
                Outcome::Refused {
                    reason: refusal.to_string(),
                },
                Reflection::default(),
            ),
        };
        let program = Program {
            stage: done.stage,
            source_bytes: done.program.source_bytes,
            outcome,
            reflection,
        };
        manifest.programs.insert(key.clone(), program);
    }
    manifest.effects = s.effects.values().cloned().collect();
    GamePack::in_memory(manifest, modules)
}

/// The pack of what is translated so far and its number; `None` before a job has ended.
pub(crate) fn pack() -> Option<(u64, Arc<GamePack>)> {
    with(|s| s.pack.clone())
}

/// The pass that draws a geometry whose vertex data is `vertex_data` with the effect whose name
/// hashes to `hash`, or why the renderer cannot be handed one; `None` before a job has
/// translated it.
pub(crate) fn shading(hash: u32, vertex_data: &[u8]) -> Shading {
    with(|s| shading_in(s, hash, vertex_data))
}

/// [`shading`], of the session `s`.
fn shading_in(s: &Shaders, hash: u32, vertex_data: &[u8]) -> Shading {
    picked(s, hash, gta6_held_streams(vertex_data))
}

fn picked(s: &Shaders, hash: u32, streams: u32) -> Shading {
    match s.shading.get(&hash)? {
        Ok(passes) => passes[pick(passes, streams)?].pass.clone(),
        Err(why) => Some(Err(why.clone())),
    }
}

/// Whether a translation job runs.
pub(crate) fn running() -> bool {
    with(|s| s.running.is_some())
}

/// Where the effects `hashes` name stand.
pub(crate) fn status(hashes: &[u32]) -> Status {
    with(|s| status_of(s, hashes))
}

fn status_of(s: &Shaders, hashes: &[u32]) -> Status {
    if s.running.is_some() {
        return Status::Translating;
    }
    let keys: BTreeSet<&String> = (hashes.iter())
        .filter_map(|h| s.effects.get(h))
        .flat_map(|e| &e.techniques)
        .flat_map(|t| &t.passes)
        .flat_map(|p| p.stages.values())
        .collect();
    if !hashes.iter().any(|h| s.effects.contains_key(h)) {
        return Status::NoSource;
    }
    let mut refusals = (keys.iter())
        .filter_map(|key| s.done.get(*key))
        .filter_map(|done| done.program.translated.as_ref().err());
    let first = refusals.next().map(|refusal| refusal.to_string());
    Status::Done {
        refused: usize::from(first.is_some()) + refusals.count(),
        first,
    }
}

/// The translation while it runs, as the job list draws it.
pub(super) fn job() -> Option<RunningJob> {
    with(|s| {
        let running = s.running.as_ref()?;
        let ended = (running.ended.lock())
            .unwrap_or_else(|p| p.into_inner())
            .is_some();
        (!ended).then(|| RunningJob {
            kind: JobKind::EffectsRead,
            doing: tr!("Translating GTA VI's programs for Shaded").to_string(),
            fraction: Some(running.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: running.cancel.is_set(),
        })
    })
}

/// Ask the translation to stop; its end is said by [`poll`].
pub(super) fn stop() {
    with(|s| {
        if let Some(running) = &s.running {
            running.cancel.set();
        }
    });
}

impl super::AppState {
    /// What the editor does once its graphics device is lost while the view draws in `mode`: the
    /// view goes back to Material, what a GTA VI drawable's own passes were drawing is kept from
    /// the next device, and a person is told.
    pub(crate) fn graphics_device_lost(&mut self, mode: or_renderer::ShadingMode) {
        use or_renderer::ShadingMode;
        let own = self.active_document().map(|doc| &doc.payload);
        if mode == ShadingMode::Shaded && matches!(own, Some(super::DocPayload::Gta6Drawable(_))) {
            crate::render::gta6_meshlets::device_lost();
        }
        self.remember_shading_mode(ShadingMode::Material);
        self.show_warning_toast(tr!(
            "The graphics device was lost. The view is back to Material, and what the Shaded view was drawing when it was lost draws as Material until OpenRage is started again"
        ));
    }
}

/// The catalogues and asset lists of an install, which join a ped's drawable to its textures.
#[path = "gta6_ped_assets.rs"]
pub(crate) mod ped_assets;

/// The textures of an install's texture archive the game binds by name whatever is drawn.
#[path = "gta6_global_textures.rs"]
pub(crate) mod global_textures;

/// The terrain tile an open terrain drawable is a cell of: its record, layers and cells.
#[path = "gta6_terrain_tile.rs"]
pub(crate) mod terrain_tile;

#[path = "gta6_layer_index.rs"]
pub(crate) mod layer_index;

#[path = "gta6_shader_translator.rs"]
mod translator;
use translator::{Translate, Translator, Workers};

#[path = "gta6_forward_passes.rs"]
mod forward_passes;

#[path = "gta6_timecycle.rs"]
pub(crate) mod timecycle;

#[cfg(test)]
#[path = "gta6_shaders_tests.rs"]
mod tests;
