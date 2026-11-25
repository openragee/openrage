//! The drawables a scanned GTA VI particle list names by its model rules and its mesh domains,
//! looked for among the dictionaries the open archives' whole scans found, on a worker as a row of
//! the job list, a body at a time with its progress and Cancel. A drawable found is drawn as its
//! rules' model, and its triangles are the surface its mesh domains emit from.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use glam::Vec3;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{read_dictionary, Drawable, Entry, ParticleEffects};
use or_job::{Cancel, Progress};
use or_simulation::domain_mesh::DomainMesh;

use super::gta6_set_finding::FindEnded;
use super::jobs::{JobKind, Outcome, RunningJob, Then};
use super::rpf9_scan::scanned_body;
use super::{AppState, DocumentId};

/// The bytes a dictionary body is read to, as the dictionary index reads them.
const BODY_LIMIT: u64 = 1 << 30;
/// The shape of a mesh domain.
const MESH: i32 = 4;

/// What a list names, by name hash: each drawable its model rules draw and each model its mesh
/// domains emit from, and the model of each mesh domain of an emitter rule, by its slot.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Named {
    pub wanted: BTreeSet<u32>,
    pub domains: BTreeMap<String, [Option<u32>; 3]>,
}

// ponytail: one entry per scanned list opened and per drawable or surface found, never emptied, as the list's lines are.
static KEPT: Mutex<BTreeMap<PathBuf, Arc<Named>>> = Mutex::new(BTreeMap::new());
static FOUND: Mutex<BTreeMap<u32, Arc<Drawable>>> = Mutex::new(BTreeMap::new());
static SURFACES: Mutex<BTreeMap<u32, Arc<DomainMesh>>> = Mutex::new(BTreeMap::new());

fn lock<T>(held: &'static Mutex<T>) -> MutexGuard<'static, T> {
    held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What `list` names.
pub(crate) fn named(list: &ParticleEffects) -> Named {
    let domains = (list.emitters.iter())
        .filter_map(|e| {
            let models = e.domains.each_ref().map(|d| {
                let (model, _) = d.as_ref()?.model.as_ref()?;
                (!model.is_empty()).then(|| name_hash32(model.as_bytes()))
            });
            models
                .iter()
                .any(Option::is_some)
                .then(|| (e.name.clone(), models))
        })
        .collect();
    let wanted = (list.drawable_hashes.iter())
        .chain(&list.mesh_hashes)
        .copied()
        .collect();
    Named { wanted, domains }
}

/// Keep what `list`, opened from `path`, names.
pub(super) fn keep(path: &Path, list: &ParticleEffects) {
    lock(&KEPT).insert(path.to_path_buf(), Arc::new(named(list)));
}

/// What the list opened from `path` names, if a GTA VI list opened from it.
pub(crate) fn kept(path: &Path) -> Option<Arc<Named>> {
    lock(&KEPT).get(path).cloned()
}

/// How many drawables the searches have found, which grows as one finds more.
pub(crate) fn found_count() -> usize {
    lock(&FOUND).len()
}

/// Whether the list opened from `path` names a drawable or a mesh domain's model.
pub(crate) fn names_drawables(path: &Path) -> bool {
    kept(path).is_some_and(|named| !named.wanted.is_empty())
}

/// The drawable of name hash `hash` a search found, if one did.
pub(crate) fn found_drawable(hash: u32) -> Option<Arc<Drawable>> {
    lock(&FOUND).get(&hash).cloned()
}

/// The triangles of the first level of `drawable` that holds models, in the drawable's frame; a
/// triangle with a corner its positions do not hold is left out.
fn triangles(drawable: &Drawable) -> Vec<[Vec3; 3]> {
    let level = (drawable.levels.iter().flatten()).find(|models| !models.is_empty());
    let geometries = (level.into_iter().flatten()).flat_map(|m| m.geometries.iter().flatten());
    let mut out = Vec::new();
    for g in geometries {
        let Ok(positions) = &g.positions else {
            continue;
        };
        let at = |i: u32| positions.get(i as usize).copied().map(Vec3::from);
        let corners = |&[a, b, c]: &[u32; 3]| Some([at(a)?, at(b)?, at(c)?]);
        out.extend(g.triangles.iter().filter_map(corners));
    }
    out
}

/// The surface of the model of name hash `hash`, once a search found it and it covers an area.
fn surface(hash: u32) -> Option<Arc<DomainMesh>> {
    if let Some(surface) = lock(&SURFACES).get(&hash) {
        return Some(Arc::clone(surface));
    }
    let drawable = found_drawable(hash)?;
    let surface = Arc::new(DomainMesh::new(triangles(&drawable))?);
    lock(&SURFACES).insert(hash, Arc::clone(&surface));
    Some(surface)
}

/// The surface of the model `hash` names for a domain of shape `shape`, where that is a mesh.
fn mesh_of(shape: i32, hash: Option<u32>) -> Option<Arc<DomainMesh>> {
    hash.filter(|_| shape == MESH).and_then(surface)
}

/// The drawables of `wanted`'s name hashes the dictionary at `offset` of `file` holds, if it is one.
fn drawables_at(file: &File, offset: u64, wanted: &BTreeSet<u32>) -> Vec<(u32, Drawable)> {
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
    let Ok(dictionary) = read_dictionary(virtual_section, Some(physical)) else {
        return Vec::new();
    };
    (dictionary.entries.into_iter())
        .filter(|(code, _)| wanted.contains(code))
        .filter_map(|(code, entry)| match entry {
            Ok(Entry::Drawable(drawable)) => Some((code, *drawable)),
            _ => None,
        })
        .collect()
}

/// A search; a new one replaces the one running, and dropping it stops it.
pub(super) struct ModelFinding {
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<FindEnded>>>,
}

impl ModelFinding {
    /// Read the dictionary bodies `bodies` until every drawable of `wanted` is found, on a worker.
    fn start(bodies: Vec<PathBuf>, mut wanted: BTreeSet<u32>) -> Self {
        let finding = Self {
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            finding.cancel.clone(),
            finding.progress.clone(),
            Arc::clone(&finding.ended),
        );
        std::thread::spawn(move || {
            let (looked_for, total) = (wanted.len(), bodies.len() as u64);
            let mut stopped = false;
            let mut open: Option<(PathBuf, File)> = None;
            for (done, path) in bodies.iter().enumerate() {
                if cancel.is_set() {
                    stopped = true;
                    break;
                }
                if wanted.is_empty() {
                    break;
                }
                progress.reached(done as u64, total);
                let Some((archive, offset)) = scanned_body(path) else {
                    continue;
                };
                if open.as_ref().is_none_or(|(at, _)| at.as_path() != archive) {
                    let file = File::open(archive).ok();
                    open = file.map(|file| (archive.to_path_buf(), file));
                }
                let Some((_, file)) = open.as_ref() else {
                    continue;
                };
                for (code, drawable) in drawables_at(file, offset, &wanted) {
                    wanted.remove(&code);
                    lock(&FOUND).insert(code, Arc::new(drawable));
                }
            }
            let outcome = match stopped {
                true => FindEnded::Stopped,
                false => FindEnded::Found(looked_for - wanted.len(), looked_for),
            };
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        finding
    }

    fn ended(&self) -> Option<FindEnded> {
        self.ended.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for ModelFinding {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

impl AppState {
    /// The job list's row for the search, while it runs.
    pub(super) fn particle_model_finding_job(&self) -> Option<RunningJob> {
        let finding = self.particle_model_finding.as_ref()?;
        if finding.ended().is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::SetSearch,
            doing: tr!("Looking for the GTA VI drawables a particle list names").to_string(),
            fraction: Some(finding.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: finding.cancel.is_set(),
        })
    }

    /// Ask the search to stop; it says it stopped once its worker sees the stop.
    pub(super) fn stop_particle_model_finding(&self) {
        if let Some(finding) = &self.particle_model_finding {
            finding.cancel.set();
        }
    }

    /// Look for the drawables the scanned GTA VI list `doc` names that no earlier search found,
    /// among the dictionaries the open archives' whole scans found.
    pub(crate) fn find_gta6_particle_models(&mut self, doc: DocumentId) -> Result<(), String> {
        if self
            .document(doc)
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return Ok(());
        }
        let path = self.document(doc).and_then(|d| d.path.as_deref());
        let Some(named) = path.and_then(kept) else {
            return Ok(());
        };
        let missing = |hash: &u32| found_drawable(*hash).is_none();
        let wanted: BTreeSet<u32> = named.wanted.iter().copied().filter(missing).collect();
        if wanted.is_empty() {
            return Ok(());
        }
        let (_, bodies) = super::gta6_dictionary_index::whole_scans(&self.documents, true);
        if bodies.is_empty() {
            return Err(
                tr!("Open the whole scan of an archive to look for the drawables in.").to_string(),
            );
        }
        self.particle_model_finding = Some(ModelFinding::start(bodies, wanted));
        Ok(())
    }

    /// Say in the job list how the search ended, once it has, and drop it; a drawable found
    /// rebuilds the effect the preview plays, so its mesh domains take the model's surface.
    pub(super) fn poll_particle_model_finding(&mut self) {
        let Some(ended) = (self.particle_model_finding.as_ref()).and_then(ModelFinding::ended)
        else {
            return;
        };
        self.particle_model_finding = None;
        let (outcome, said) = match ended {
            FindEnded::Found(found, of) => {
                self.sim.pending_rebuild |= found > 0;
                (
                    Outcome::Done(Then::Nothing),
                    tr!(
                        "GTA VI drawables found in the open scans: {found} of {of}",
                        found = found,
                        of = of
                    ),
                )
            }
            FindEnded::Stopped => (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished").to_string(),
            ),
            FindEnded::Failed(why) => (
                Outcome::Failed,
                tr!("The drawables were not looked for: {why}", why = why),
            ),
        };
        self.jobs.record(JobKind::SetSearch, outcome, said);
    }

    /// Give each mesh domain of the effects the preview plays from `doc`, of any slot, the surface
    /// of the model it names, where a search found it; one whose model was not found keeps its shape.
    /// Each emitter takes its rule's third spawner too, and every effect spawned later its own.
    pub(super) fn give_gta6_domain_meshes(&mut self, doc: DocumentId) {
        let path = self.document(doc).and_then(|d| d.path.as_deref());
        let spawners = path.and_then(super::gta6_particles::spawners::of);
        let named = path.and_then(kept);
        let manager = &mut self.sim.manager;
        manager.zcull_spawners = spawners.map(|s| (*s).clone()).unwrap_or_default();
        let effects = (manager.effect.iter_mut()).chain(manager.spawned_effects.iter_mut());
        for effect in effects {
            or_simulation::zcull_spawner::give(effect, &manager.zcull_spawners);
        }
        let Some(named) = named else {
            return;
        };
        let effects = (manager.effect.iter_mut()).chain(manager.spawned_effects.iter_mut());
        for emitter in effects.flat_map(|e| e.emitters.iter_mut()) {
            let models = named.domains.get(&emitter.rules.rule.name);
            let [creation, target, attractor] = models.copied().unwrap_or_default();
            emitter.creation_domain.mesh = mesh_of(emitter.rules.creation_domain.shape, creation);
            emitter.velocity_domain.mesh = mesh_of(emitter.rules.velocity_domain.shape, target);
            if let Some(rule) = &emitter.rules.attractor_domain {
                let inst = emitter
                    .attractor_domain
                    .get_or_insert_with(Default::default);
                inst.mesh = mesh_of(rule.shape, attractor);
            }
        }
    }
}

/// Hold `drawable` as a search would have found it under `hash`.
#[cfg(test)]
pub(crate) fn found_for_tests(hash: u32, drawable: Drawable) {
    lock(&FOUND).insert(hash, Arc::new(drawable));
}

/// Keep `named` as a list opened from `path` would have it kept.
#[cfg(test)]
pub(crate) fn named_for_tests(path: &Path, named: Named) {
    lock(&KEPT).insert(path.to_path_buf(), Arc::new(named));
}

/// A drawable of one triangle in its first level, in a box from the origin to `box_max`.
#[cfg(test)]
pub(crate) fn one_triangle(box_max: [f32; 3]) -> Drawable {
    use or_io::gta6::headerless::{Geometry, Model, Palette, Vertices};
    let geometry = Geometry {
        vertex_data: Vec::new(),
        index_data: Vec::new(),
        shader: 0,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: Ok(vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]),
        vertices: Vertices::default(),
    };
    let model = Model {
        geometries: vec![Ok(geometry)],
        palette: Ok(Palette::Own),
        matrix_index: 0,
    };
    Drawable {
        name: "d".to_string(),
        box_min: [0.0; 3],
        box_max,
        shaders: Vec::new(),
        levels: [Some(vec![model]), None, None, None],
        lod_distances: [10.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use or_io::gta5::particle::{PtxDomain, PtxEmitterRule};
    use or_io::gta6::headerless::{Geometry, HeaderlessError, Model, Palette, Vertices};
    use or_simulation::{EffectInst, EmitterInst, EmitterRules};

    use super::*;
    use crate::state::{DocPayload, Document};

    #[test]
    fn a_table_particle_list_never_starts_a_model_scan() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::new();
        let path = Path::new("particles.rpf/.rpf9-table/0/hash_models.ypt");
        named_for_tests(
            path,
            Named {
                wanted: BTreeSet::from([name_hash32(b"table_missing_particle_model")]),
                domains: BTreeMap::new(),
            },
        );
        state.documents.push(Document::new(
            1,
            Some(path.to_path_buf()),
            DocPayload::Resource(or_io::Asset::Particles(Default::default())),
        ));
        assert!(state.document(1).unwrap().is_from_rpf9_table());
        assert!(names_drawables(path));
        state
            .find_gta6_particle_models(1)
            .expect("table previews never request a scan");
        assert!(state.particle_model_finding.is_none());
    }

    #[test]
    fn a_surface_is_the_triangles_of_the_first_level_that_holds_models() {
        let corners = vec![[0.0; 3], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]];
        let geometry = |positions, triangles| Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 0,
            vertex_count: 4,
            channels: Vec::new(),
            triangles,
            positions,
            vertices: Vertices::default(),
        };
        let unread = Err(HeaderlessError::Particles("its positions did not read"));
        let model = Model {
            geometries: vec![
                Ok(geometry(Ok(corners.clone()), vec![[0, 1, 2], [1, 2, 9]])),
                Ok(geometry(unread, vec![[0, 1, 2]])),
                Err(HeaderlessError::Particles("it did not read")),
            ],
            palette: Ok(Palette::Own),
            matrix_index: 0,
        };
        let mut drawable = one_triangle([1.0; 3]);
        drawable.levels = [None, Some(Vec::new()), Some(vec![model]), None];
        let expected = [corners[0], corners[1], corners[2]].map(Vec3::from);
        assert_eq!(triangles(&drawable), vec![expected]);
        // Its one triangle covers an area, so it is a surface to emit from.
        assert!(DomainMesh::new(triangles(&drawable)).is_some());
    }

    #[test]
    fn a_mesh_domain_takes_the_surface_of_the_model_a_search_found() {
        const HASH: u32 = 0x6D0D_E1A1;
        let path = PathBuf::from("a_mesh_domain_takes_the_surface.particles");
        let named = Named {
            wanted: BTreeSet::from([HASH]),
            domains: BTreeMap::from([
                ("on the model".to_string(), [Some(HASH), None, None]),
                ("at the model".to_string(), [None, Some(HASH), Some(HASH)]),
            ]),
        };
        lock(&KEPT).insert(path.clone(), Arc::new(named));
        let mut state = AppState::new();
        let payload = DocPayload::Resource(or_io::Asset::Particles(Default::default()));
        state.documents.push(Document::new(7, Some(path), payload));
        let mut on_model = PtxEmitterRule {
            name: "on the model".to_string(),
            ..Default::default()
        };
        let mesh = PtxDomain {
            shape: MESH,
            ..Default::default()
        };
        on_model.creation_domain_obj.domain = Some(Box::new(mesh.clone()));
        let mut at_model = PtxEmitterRule {
            name: "at the model".to_string(),
            ..Default::default()
        };
        at_model.target_domain_obj.domain = Some(Box::new(mesh.clone()));
        at_model.attractor_domain_obj.domain = Some(Box::new(mesh));
        let boxed = PtxEmitterRule {
            name: "in a box".to_string(),
            ..Default::default()
        };
        let emitter = |rule| {
            EmitterInst::new(EmitterRules::from_event(
                rule,
                Default::default(),
                &Default::default(),
            ))
        };
        let emitters = vec![emitter(on_model), emitter(at_model), emitter(boxed)];
        state.sim.manager.effect = Some(EffectInst::new(Default::default(), emitters));
        // Each emitter's creation, target and attractor domain: whether it holds a surface.
        let meshes = |state: &AppState| -> Vec<[bool; 3]> {
            let effect = state.sim.manager.effect.as_ref().expect("an effect plays");
            let emitters = effect.emitters.iter();
            emitters
                .map(|e| {
                    let attractor = e.attractor_domain.as_ref();
                    [
                        e.creation_domain.mesh.is_some(),
                        e.velocity_domain.mesh.is_some(),
                        attractor.is_some_and(|d| d.mesh.is_some()),
                    ]
                })
                .collect()
        };

        state.give_gta6_domain_meshes(7);
        let none = [false; 3];
        assert_eq!(meshes(&state), [none; 3], "no search found the model");

        found_for_tests(HASH, one_triangle([1.0; 3]));
        state.give_gta6_domain_meshes(7);
        let expected = [[true, false, false], [false, true, true], none];
        assert_eq!(meshes(&state), expected, "only the mesh domains");
    }
}
