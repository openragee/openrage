//! A GTA VI map body a scan found, on the Map screen: each entity a box turned and scaled as GTA V's
//! are, its archetype's box taken from a map types body that is open or that a search of the scan
//! found, a marker where none holds it, and each instance list as its own box. The search runs on a
//! worker as a row of the job list, a body at a time with its progress and Cancel.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use glam::{Quat, Vec3};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9ZlibRun};
use or_io::gta6::map::{self, Archetype, Entity, MapBody, MapData};
use or_job::{Cancel, Progress};
use or_maps::gizmo::Obb;
use or_maps::{Name, Placed, Scene};

use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::rpf9_scan::{scanned_body, view_of, Row, View};
use super::{AppState, DocPayload};
use crate::ui::dock::Workspace;

/// The bytes a body is read to: more than any map types body was measured to hold.
const BODY_LIMIT: u64 = 256 << 20;

type Opened = BTreeMap<PathBuf, Result<Arc<MapBody>, String>>;

/// What each scanned map body opened read to, by its path.
// ponytail: one map for the whole app, which no close empties, as the skeletons' is; a map body is small once its lists are counted.
fn opened() -> MutexGuard<'static, Opened> {
    static OPENED: Mutex<Opened> = Mutex::new(BTreeMap::new());
    OPENED.lock().unwrap_or_else(|p| p.into_inner())
}

/// An archetype a search of a scan found, with the offset of the body that holds it.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub offset: u64,
    pub archetype: Archetype,
}

/// The archetypes found so far, by the archive searched and their name word.
type FoundBy = BTreeMap<PathBuf, BTreeMap<u64, Found>>;

fn found() -> MutexGuard<'static, FoundBy> {
    static FOUND: Mutex<FoundBy> = Mutex::new(BTreeMap::new());
    FOUND.lock().unwrap_or_else(|p| p.into_inner())
}

/// The drawable and fragment bodies found so far, by the archive searched and the word their stated
/// name hashes to: each one's offset.
type ModelsBy = BTreeMap<PathBuf, BTreeMap<u64, u64>>;

fn models() -> MutexGuard<'static, ModelsBy> {
    static MODELS: Mutex<ModelsBy> = Mutex::new(BTreeMap::new());
    MODELS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Bumped whenever an archetype becomes known, so a scene placed before is placed again.
static KNOWN: AtomicU64 = AtomicU64::new(0);

/// Keep what the map reader reads of the scanned body at `path`, when its root is a map class.
pub(super) fn keep(path: &Path, virtual_section: &[u8], physical: &[u8]) -> bool {
    let Some(root) = map::root_type(virtual_section) else {
        return false;
    };
    if ![map::C_MAP_DATA, map::C_MAP_TYPES].contains(&root) {
        return false;
    }
    let read = map::open(virtual_section, physical).map(Arc::new);
    let kept = read.is_ok();
    if kept {
        links::keep(path, virtual_section, physical);
    }
    if matches!(read, Ok(ref body) if matches!(**body, MapBody::Types(_))) {
        KNOWN.fetch_add(1, Ordering::Relaxed);
    }
    opened().insert(path.to_path_buf(), read.map_err(|e| e.to_string()));
    kept
}

/// Keep `body` as what the scanned body at `path` read to.
#[cfg(test)]
pub(crate) fn keep_read(path: &Path, body: MapBody) {
    KNOWN.fetch_add(1, Ordering::Relaxed);
    opened().insert(path.to_path_buf(), Ok(Arc::new(body)));
}

/// What the map reader read of the scanned body at `path`, or why it refused it; `None` for a body that is no map body.
pub(crate) fn opened_map(path: &Path) -> Option<Result<Arc<MapBody>, String>> {
    opened().get(path).cloned()
}

/// Where an entity's archetype was found.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A map types body open in the app, by its path.
    Open(PathBuf),
    /// A body of the scanned archive, by its offset.
    Scan(u64),
}

/// The map body the Map screen shows and what it places.
#[derive(Debug, Clone)]
pub struct ScannedMap {
    pub path: PathBuf,
    pub map: Arc<MapBody>,
    pub scene: Scene,
    /// Each entity's archetype and where it was found, `None` for one drawn as a marker.
    pub archetypes: Vec<Option<(Source, Archetype)>>,
    /// The scan offset of each entity's drawable, fragment or dictionary body, where one was found.
    pub models: Vec<Option<u64>>,
    /// The bump of [`KNOWN`] and the open map types bodies the scene was placed with.
    key: (u64, usize),
    /// What the map links to: its parents, its instances' points.
    pub links: Arc<links::Shown>,
}

impl ScannedMap {
    pub fn data(&self) -> Option<&MapData> {
        match &*self.map {
            MapBody::Data(m) => Some(m),
            MapBody::Types(_) => None,
        }
    }

    /// The entities drawn as markers, as no archetype was found for them.
    pub fn markers(&self) -> usize {
        self.archetypes.iter().filter(|a| a.is_none()).count()
    }

    /// The entities whose archetype names a drawable or a fragment that no body found holds.
    pub fn unmodelled(&self) -> usize {
        (self.archetypes.iter().zip(&self.models))
            .filter(|(a, m)| m.is_none() && a.as_ref().is_some_and(|(_, a)| wants_a_model(a)))
            .count()
    }
}

/// Whether `a` names its model by a drawable's or a fragment's word, which a body found by a scan
/// states; a drawable dictionary's names no single body.
fn wants_a_model(a: &Archetype) -> bool {
    a.drawable | a.fragment != 0
}

/// What a map places: each entity's box from its archetype, a marker where none is known, then each
/// instance list's box after the entities.
fn placed(
    label: &str,
    m: &MapData,
    known: &[(Source, &Archetype)],
    bodies: &BTreeMap<u64, u64>,
) -> Placing {
    // The first to state a word or a name wins; an entity joins on the 64-bit word, else on the 32-bit name.
    let mut by_word = HashMap::new();
    let mut by_name = HashMap::new();
    for (i, (_, a)) in known.iter().enumerate() {
        by_word.entry(a.name_word).or_insert(i);
        by_name.entry(a.name).or_insert(i);
    }
    let joined = |e: &Entity| {
        (by_word.get(&e.archetype_word))
            .or_else(|| by_name.get(&e.archetype_name))
            .map(|i| &known[*i])
    };
    let mut scene = Scene::default();
    let mut archetypes = Vec::with_capacity(m.entities.len());
    let mut models = Vec::with_capacity(m.entities.len());
    let half = Vec3::splat(or_maps::scene::UNKNOWN_HALF);
    for (index, e) in m.entities.iter().enumerate() {
        let found = joined(e);
        let model = found
            .filter(|(_, a)| wants_a_model(a))
            .and_then(|(_, a)| bodies.get(&(a.drawable | a.fragment)).copied());
        models.push(model);
        let (min, max) = found.map_or((-half, half), |(_, a)| {
            (Vec3::from(a.bb_min), Vec3::from(a.bb_max))
        });
        let rotation = Quat::from_array(e.model_rotation());
        let lod_dist = match (e.lod_dist < 0.0, found) {
            (true, Some((_, a))) => a.lod_dist,
            _ => e.lod_dist,
        };
        scene.placed.push(Placed {
            source: label.to_string(),
            index,
            archetype: Name::of(e.archetype_name),
            declared: found.is_some(),
            model: model.map(|offset| format!("{offset:#X}")),
            obb: Obb {
                position: Vec3::from(e.position),
                rotation: if rotation.is_finite() {
                    rotation.normalize()
                } else {
                    Quat::IDENTITY
                },
                scale: or_world::transform::scale_of(e.scale_xy, e.scale_z),
                min,
                max,
            },
            interior: e.interior,
            lod_dist,
            parent_index: i32::from(e.parent_index),
            guid: e.id as u32,
            lod_level: Some(e.lod_level_name().to_string()),
        });
        archetypes.push(found.map(|(source, a)| (source.clone(), (*a).clone())));
    }
    let lists = m.prop_lists.iter().chain(&m.grass_lists);
    for (k, list) in lists.enumerate() {
        let (lo, hi) = (
            Vec3::from(list.batch_aabb.min),
            Vec3::from(list.batch_aabb.max),
        );
        scene.placed.push(Placed {
            source: label.to_string(),
            index: m.entities.len() + k,
            archetype: Name::of(list.archetype_name),
            declared: true,
            model: None,
            obb: Obb {
                position: (lo + hi) * 0.5,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                min: (lo - hi) * 0.5,
                max: (hi - lo) * 0.5,
            },
            interior: false,
            lod_dist: 0.0,
            parent_index: -1,
            guid: 0,
            lod_level: None,
        });
    }
    let corners = scene.placed.iter().flat_map(|p| p.obb.corners());
    scene.bounds = corners.fold(None, |b: Option<(Vec3, Vec3)>, c| {
        Some(b.map_or((c, c), |(lo, hi)| (lo.min(c), hi.max(c))))
    });
    Placing {
        scene,
        archetypes,
        models,
    }
}

struct Placing {
    scene: Scene,
    archetypes: Vec<Option<(Source, Archetype)>>,
    models: Vec<Option<u64>>,
}

impl AppState {
    /// The scanned GTA VI map body the active document is, as the Map screen shows it.
    pub fn scanned_map(&self) -> Option<&ScannedMap> {
        self.workbench.maps.scanned.as_ref()
    }

    /// `each` given every archetype an open map types body holds or a search of the shown map's
    /// archive found, none of them copied.
    // ⚠ `each` runs while the search's map is locked, so it must not lock it again.
    pub(crate) fn each_known_archetype(&self, mut each: impl FnMut(&Archetype)) {
        for d in &self.documents {
            if let Some(Ok(b)) = d.path.as_deref().and_then(opened_map) {
                if let MapBody::Types(t) = &*b {
                    t.archetypes.iter().for_each(&mut each);
                }
            }
        }
        let archive = (self.scanned_map()).and_then(|s| scanned_body(&s.path));
        if let Some((archive, _)) = archive {
            if let Some(held) = found().get(archive) {
                held.values().for_each(|f| each(&f.archetype));
            }
        }
    }

    /// Place the active document's map again when it, or an archetype it may name, changed since.
    pub(super) fn refresh_scanned_map(&mut self) {
        let shown = (self.active_document())
            .and_then(|d| d.path.clone())
            .and_then(|path| match opened_map(&path) {
                Some(Ok(body)) if matches!(*body, MapBody::Data(_)) => Some((path, body)),
                _ => None,
            });
        let Some((path, body)) = shown else {
            if self.workbench.maps.scanned.take().is_some()
                && self.active_workspace == Workspace::Map
                && !self.has_maps()
            {
                self.active_workspace = Workspace::Data;
            }
            return;
        };
        let open: Vec<(PathBuf, Arc<MapBody>)> = (self.documents.iter())
            .filter_map(|d| d.path.clone())
            .filter_map(|p| match opened_map(&p) {
                Some(Ok(b)) if matches!(*b, MapBody::Types(_)) => Some((p, b)),
                _ => None,
            })
            .collect();
        // An edited tree's revision only grows, so the sum changes whenever either does.
        let (revision, tree) = self.edited_map_tree(&path);
        let key = (
            KNOWN.load(Ordering::Relaxed).wrapping_add(revision),
            open.len(),
        );
        let same = |s: &ScannedMap| s.path == path && s.key == key;
        if self.workbench.maps.scanned.as_ref().is_some_and(same) {
            return;
        }
        let edited = match (&*body, tree) {
            (MapBody::Data(read), Some(tree)) => Some(Arc::new(MapBody::Data(
                super::gta6_map_edit::synced(read, tree),
            ))),
            _ => None,
        };
        let body = edited.unwrap_or(body);
        let MapBody::Data(m) = &*body else {
            return;
        };
        let archive = scanned_body(&path).map(|(a, _)| a.to_path_buf());
        let from_scan: Vec<Found> = (archive.as_ref())
            .and_then(|a| found().get(a).map(|f| f.values().cloned().collect()))
            .unwrap_or_default();
        let bodies = (archive.as_ref())
            .and_then(|a| models().get(a).cloned())
            .unwrap_or_default();
        let mut known: Vec<(Source, &Archetype)> = Vec::new();
        for (p, b) in &open {
            if let MapBody::Types(t) = &**b {
                known.extend(t.archetypes.iter().map(|a| (Source::Open(p.clone()), a)));
            }
        }
        known.extend(
            from_scan
                .iter()
                .map(|f| (Source::Scan(f.offset), &f.archetype)),
        );
        let placed = placed(&path.display().to_string(), m, &known, &bodies);
        let mut models = placed.models;
        links::dictionary_models(archive.as_deref(), &placed.archetypes, &mut models);
        let links = Arc::new(links::shown(&path, m));
        let work = &mut self.workbench.maps;
        if work.scanned.as_ref().is_none_or(|s| s.path != path) {
            work.selected = None;
            work.preview = None;
            work.needs_frame = true;
            work.plan.fitted = false;
        }
        work.scanned = Some(ScannedMap {
            path,
            map: Arc::clone(&body),
            scene: placed.scene,
            archetypes: placed.archetypes,
            models,
            key,
            links,
        });
    }

    /// Look for the archetypes the map shown names and nothing known holds, among the headerless
    /// bodies with no physical section the open scan of its archive found, then for the drawable
    /// and fragment bodies the archetypes name, among its drawable and fragment bodies.
    pub(crate) fn find_gta6_archetypes(&mut self) -> Result<(), String> {
        let Some(shown) = self.scanned_map() else {
            return Ok(());
        };
        let Some(m) = shown.data() else {
            return Ok(());
        };
        let wanted: HashSet<u64> = (m.entities.iter().zip(&shown.archetypes))
            .filter(|(_, a)| a.is_none())
            .map(|(e, _)| e.archetype_word)
            .collect();
        let wanted_models: HashSet<u64> = (shown.archetypes.iter().zip(&shown.models))
            .filter_map(|(a, model)| match (a, model) {
                (Some((_, a)), None) if wants_a_model(a) => Some(a.drawable | a.fragment),
                _ => None,
            })
            .collect();
        let dictionaries = links::unresolved_dictionaries(shown) > 0;
        if wanted.is_empty() && wanted_models.is_empty() && !dictionaries {
            return Ok(());
        }
        let archive = scanned_body(&shown.path)
            .map(|(archive, _)| archive.to_path_buf())
            .ok_or_else(|| {
                tr!("This map is no body of a scanned archive, so no scan holds its archetypes.")
                    .to_string()
            })?;
        let (mut types, mut bodies, mut dictionary_bodies) = (Vec::new(), Vec::new(), Vec::new());
        let scans = (self.documents.iter()).filter_map(|d| match &d.payload {
            DocPayload::Rpf9(scan) if scan.path == archive => Some(scan),
            _ => None,
        });
        for scan in scans {
            for row in &scan.found().rows {
                let Row::Body(find) = row else {
                    continue;
                };
                let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                    resource: Some(r),
                    signature,
                    ..
                }) = find.kind
                else {
                    continue;
                };
                match view_of(find) {
                    // A map types body has no physical section.
                    Ok(View::Headerless) if r.physical_len == 0 => types.push(find.offset),
                    Ok(View::Drawable)
                        if signature.and_then(|s| s.role()).is_some_and(|role| {
                            matches!(role, Rpf9Role::Drawable | Rpf9Role::DrawableWithBound)
                        }) =>
                    {
                        bodies.push(find.offset)
                    }
                    Ok(View::Facts(s)) if s.role() == Some(Rpf9Role::Dictionary) => {
                        dictionary_bodies.push(find.offset)
                    }
                    _ => {}
                }
            }
        }
        if types.is_empty() && bodies.is_empty() && dictionary_bodies.is_empty() {
            return Err(tr!(
                "Open the scan of {archive}: the archetypes and the models they name are looked for among the bodies it finds.",
                archive = archive.display()
            ));
        }
        let looked_for = Wanted {
            archetypes: wanted,
            models: wanted_models,
            dictionaries,
        };
        let offsets = [types, bodies, dictionary_bodies];
        (self.map_search).push(ArchetypeSearch::start(archive, offsets, looked_for));
        Ok(())
    }

    /// Open the drawable or fragment body at `offset` of the archive the shown map is a body of.
    pub(crate) fn open_gta6_model(&mut self, offset: u64) {
        let archive = (self.scanned_map())
            .and_then(|s| scanned_body(&s.path))
            .map(|(archive, _)| archive.to_path_buf());
        if let Some(archive) = archive {
            self.load_file(super::rpf9_scan::body_path(
                &archive,
                offset,
                View::Drawable,
            ));
        }
    }

    /// The searches still running, as the job list draws them.
    pub(super) fn map_search_jobs(&self) -> impl Iterator<Item = RunningJob> + '_ {
        (self.map_search.iter().filter_map(ArchetypeSearch::job))
            .chain(self.models_in_place_job())
            .chain(super::gta6_world::fill::job())
            .chain(entity::job())
            .chain(super::gta6_world::worldview::viewer().job())
    }

    /// Ask every search to stop; each says it stopped once its worker sees the stop.
    pub(super) fn stop_map_search(&self) {
        self.map_search.iter().for_each(|s| s.cancel.set());
        self.stop_models_in_place();
        super::gta6_world::fill::stop();
        entity::stop();
        super::gta6_world::worldview::viewer().stop();
    }

    /// Say in the job list how each search that ended went, and drop it.
    pub(super) fn poll_map_search(&mut self) {
        let mut ended = Vec::new();
        self.map_search.retain(|s| match s.ended() {
            Some(e) => {
                ended.push(e);
                false
            }
            None => true,
        });
        for e in ended {
            let (outcome, said) = match e {
                SearchEnded::Found { archetypes, models } => {
                    let mut said = Vec::new();
                    if archetypes.1 > 0 {
                        said.push(tr_n!(
                            "{count} of {of} GTA VI archetype found in the scan",
                            "{count} of {of} GTA VI archetypes found in the scan",
                            archetypes.0,
                            of = archetypes.1
                        ));
                    }
                    if models.1 > 0 {
                        said.push(tr_n!(
                            "{count} of {of} GTA VI model found in the scan",
                            "{count} of {of} GTA VI models found in the scan",
                            models.0,
                            of = models.1
                        ));
                    }
                    (Outcome::Done(Then::Nothing), said.join("; "))
                }
                SearchEnded::Stopped => (
                    Outcome::Stopped(Then::Nothing),
                    tr!("Stopped before it finished").to_string(),
                ),
                SearchEnded::Failed(why) => (
                    Outcome::Failed,
                    tr!("The archetypes were not looked for: {why}", why = why),
                ),
            };
            self.jobs.record(JobKind::ArchetypeSearch, outcome, said);
        }
        // ponytail: a fill of the world is listed as an archetype search, whose Stop stops it; a kind of its own if the two are ever told apart in the list.
        if let Some((outcome, said)) = super::gta6_world::fill::ended() {
            self.jobs.record(JobKind::ArchetypeSearch, outcome, said);
        }
        self.poll_entity_edit();
        // ponytail: the world index is listed as an archetype search too, whose Stop stops it.
        let indexed = super::gta6_world::worldview::viewer().ended();
        if let Some((outcome, said)) = indexed {
            self.jobs.record(JobKind::ArchetypeSearch, outcome, said);
        }
        self.refresh_scanned_map();
        self.refresh_models_in_place();
    }
}

/// How a search ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SearchEnded {
    /// How many archetypes and models it found, each with how many it looked for.
    Found {
        archetypes: (usize, usize),
        models: (usize, usize),
    },
    Stopped,
    Failed(String),
}

/// The words a search looks for: of archetypes an entity names, and of models an archetype names;
/// and whether the drawable dictionaries are indexed by their keys.
struct Wanted {
    archetypes: HashSet<u64>,
    models: HashSet<u64>,
    dictionaries: bool,
}

pub(super) struct ArchetypeSearch {
    doing: String,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<SearchEnded>>>,
}

impl ArchetypeSearch {
    /// Read the map types bodies at `offsets[0]` of `archive` until each archetype `wanted` names is
    /// found, then the drawable and fragment bodies at `offsets[1]` until each model it names or
    /// those archetypes name is, then indexes the dictionary bodies at `offsets[2]` by their
    /// drawables' keys where a drawable dictionary is wanted, on a worker.
    fn start(archive: PathBuf, offsets: [Vec<u64>; 3], mut wanted: Wanted) -> Self {
        let search = Self {
            doing: tr!(
                "Looking for GTA VI archetypes in {archive}",
                archive = file_name(&archive)
            ),
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            search.cancel.clone(),
            search.progress.clone(),
            Arc::clone(&search.ended),
        );
        std::thread::spawn(move || {
            let total = offsets.iter().map(Vec::len).sum::<usize>() as u64;
            let outcome = match File::open(&archive) {
                Err(e) => SearchEnded::Failed(format!("{}: {e}", archive.display())),
                Ok(file) => {
                    let archetypes = wanted.archetypes.len();
                    let mut done = 0u64;
                    for offset in &offsets[0] {
                        if cancel.is_set() || wanted.archetypes.is_empty() {
                            break;
                        }
                        progress.reached(done, total);
                        done += 1;
                        for archetype in types_at(&archive, &file, *offset) {
                            if wanted.archetypes.remove(&archetype.name_word) {
                                wanted.dictionaries |= archetype.drawable_dictionary != 0;
                                if wants_a_model(&archetype) {
                                    wanted
                                        .models
                                        .insert(archetype.drawable | archetype.fragment);
                                }
                                let at = Found {
                                    offset: *offset,
                                    archetype,
                                };
                                (found().entry(archive.clone()).or_default())
                                    .insert(at.archetype.name_word, at);
                                KNOWN.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    // A model a search found before is not looked for again.
                    if let Some(known) = models().get(&archive) {
                        wanted.models.retain(|w| !known.contains_key(w));
                    }
                    let models = wanted.models.len();
                    for offset in &offsets[1] {
                        if cancel.is_set() || wanted.models.is_empty() {
                            break;
                        }
                        progress.reached(done, total);
                        done += 1;
                        let Some(word) = model_at(&file, *offset) else {
                            continue;
                        };
                        if wanted.models.remove(&word) {
                            (self::models().entry(archive.clone()).or_default())
                                .insert(word, *offset);
                            KNOWN.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    if wanted.dictionaries {
                        let at = (&mut done, total);
                        links::index_dictionaries(
                            &archive,
                            &file,
                            &offsets[2],
                            (&cancel, &progress),
                            at,
                        );
                    }
                    match cancel.is_set() {
                        true => SearchEnded::Stopped,
                        false => SearchEnded::Found {
                            archetypes: (archetypes - wanted.archetypes.len(), archetypes),
                            models: (models - wanted.models.len(), models),
                        },
                    }
                }
            };
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        search
    }

    fn job(&self) -> Option<RunningJob> {
        if self.ended().is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::ArchetypeSearch,
            doing: self.doing.clone(),
            fraction: Some(self.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: self.cancel.is_set(),
        })
    }

    fn ended(&self) -> Option<SearchEnded> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for ArchetypeSearch {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// The archetypes of the map types body at `offset` of `file`, the archive at `archive`, none when
/// it is another body; its interiors are kept.
fn types_at(archive: &Path, file: &File, offset: u64) -> Vec<Archetype> {
    let Ok((find, bytes)) = read_rpf9_body(file, offset, BODY_LIMIT) else {
        return Vec::new();
    };
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return Vec::new();
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    if map::root_type(virtual_section) != Some(map::C_MAP_TYPES) {
        return Vec::new();
    }
    match map::links::open(virtual_section, physical) {
        Ok((MapBody::Types(t), read)) => {
            links::keep_read_interiors(archive, &read);
            super::gta6_world::keep_searched_types(&t, virtual_section, physical);
            t.archetypes
        }
        _ => Vec::new(),
    }
}

/// The word the drawable or fragment body at `offset` names itself by, `None` where it states no name.
// ponytail: the whole body is read for a name in its first chunk; read that chunk alone if a search tells.
fn model_at(file: &File, offset: u64) -> Option<u64> {
    let (find, bytes) = read_rpf9_body(file, offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    or_io::gta6::headerless::stated_model_word(resource.sections(&bytes).0).ok()
}

#[path = "gta6_map_links.rs"]
pub(crate) mod links;

#[path = "gta6_map_entity.rs"]
mod entity;

#[path = "gta6_map_types_edit.rs"]
mod types_edit;

#[cfg(test)]
#[path = "gta6_map_tests.rs"]
mod tests;
