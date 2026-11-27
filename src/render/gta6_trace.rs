//! The first traced picture in Shaded: the sun's mask of a GTA VI model written by one of the
//! game's own walks, where the developer's aid `traced` asks for it and the device runs it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use or_io::gta6::accel::Triangle;
use or_io::gta6::headerless::Model;
use or_renderer::game_shaders::meshlets::MeshletScene;
use or_renderer::game_shaders::trace::{self, query, SceneArena, TracedMask, WalkModule};

use crate::state::{AppState, DocumentId};

/// What a worker builds: nothing yet, then the thing or why it was not built.
type Slot<T> = Arc<Mutex<Option<Result<Arc<T>, String>>>>;
/// What an arena is of: the document, what it held when it was read, the drawable picked in it
/// and the level drawn.
type Of = (DocumentId, u64, Option<u64>, usize);

/// What the traced writer is made of so far, across frames.
#[derive(Default)]
struct Glue {
    /// The walk file the aid names, empty for those `OPENRAGE_RDNA_WALKS` lists, whether its test
    /// of a node asks the device's query, and its walk.
    asked: Option<(String, bool)>,
    walk: Option<Slot<WalkModule>>,
    /// Why the walk through the device's query did not run, after which the walk tests in software.
    query_failed: Option<String>,
    /// The model's arena, numbered: a scene told another number is handed a new pass.
    arena: Option<(Of, u64, Slot<SceneArena>)>,
    built: u64,
    /// The arena a device refused the pass over, and why.
    refused: Option<(u64, String)>,
    line: Option<String>,
}

static GLUE: Mutex<Option<Glue>> = Mutex::new(None);

/// `work` run on the trace's worker, off the frame.
fn start<T: Send + Sync + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Slot<T> {
    let slot: Slot<T> = Arc::default();
    let theirs = Arc::clone(&slot);
    let task = move || {
        let made = work().map(Arc::new);
        *theirs.lock().unwrap_or_else(PoisonError::into_inner) = Some(made);
    };
    if let Err(error) = trace::worker().run(task) {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(Err(error.to_string()));
    }
    slot
}

fn peek<T>(slot: &Slot<T>) -> Option<Result<Arc<T>, String>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The walk of a shadow ray, as the translation cache lowers it alone; with `query`, its test of a node asks the device's ray query.
fn walk(named: Option<PathBuf>, query: bool) -> Result<WalkModule, String> {
    let lowered = or_cli::gta6_kept::shadow_walk(named.as_deref(), query)?;
    tracing::info!(
        file = %lowered.file.display(),
        program = lowered.program,
        query,
        "the walk of the traced sun's mask was lowered"
    );
    Ok(WalkModule {
        module: Arc::new(lowered.module),
        given: lowered.given,
        left: lowered.left,
        threads: lowered.threads,
        shared_dwords: lowered.shared_dwords,
        bound: lowered.walk.bound,
        unbounded: lowered.loops_not_shown_to_end,
    })
}

/// The triangles of each geometry the view draws of `models`: a geometry numbered by its place
/// among them and a triangle by its place in its geometry, every one opaque.
// ⚠ A cutout surface is traced solid: the alpha test is the game's candidate code, which the walk
// lowered alone does not run.
fn triangles_of(models: &[Model]) -> Vec<Triangle> {
    let mut triangles = Vec::new();
    for (number, geometry) in crate::render::gta6_meshlets::drawn(models).enumerate() {
        let Ok(positions) = &geometry.positions else {
            continue;
        };
        for (primitive, corners) in geometry.triangles.iter().enumerate() {
            let [Some(a), Some(b), Some(c)] = corners.map(|v| positions.get(v as usize).copied())
            else {
                continue;
            };
            if [a, b, c].iter().flatten().all(|x| x.is_finite()) {
                triangles.push(Triangle {
                    corners: [a, b, c],
                    geometry: number as u32,
                    primitive: primitive as u32,
                    opaque: true,
                });
            }
        }
    }
    triangles
}

fn of(state: &AppState) -> Option<Of> {
    let doc = state.active_document()?;
    let selection = state.selection.selected(doc.id);
    Some((
        doc.id,
        doc.payload_epoch(),
        selection,
        state.preview_scene.lod,
    ))
}

fn shadow_map(why: &str) -> String {
    tr!(
        "The sun's shadow: the shadow map, since the traced pass does not run: {why}",
        why = why,
    )
    .to_string()
}

fn building() -> String {
    tr!("The sun's shadow: the shadow map, while the traced pass is built").to_string()
}

/// Whether the walk of a device is lowered with its test of a node asking the device's query: where the aid asks for it, the device offers it, and no such walk has failed on it.
fn queried(device: &wgpu::Device, failed: bool) -> bool {
    !failed
        && query::asked(
            trace::query_aid(),
            device.adapter_info().backend,
            device.features(),
        )
}

/// What the Shaded line says of the body that tests the walk's nodes.
fn body(queried: bool) -> String {
    match queried {
        true => tr!("The traced walk tests each node by the device's ray query").to_string(),
        false => tr!("The traced walk tests each node in software").to_string(),
    }
}

impl Glue {
    /// Where the traced writer stands this frame, with what it lacks started on the worker.
    fn advance(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        scene: Option<&MeshletScene>,
        state: &AppState,
        asked: String,
    ) -> String {
        let asked = (asked, queried(device, self.query_failed.is_some()));
        if self.asked.as_ref() != Some(&asked) {
            let (named, query) = (
                (!asked.0.is_empty()).then(|| PathBuf::from(&asked.0)),
                asked.1,
            );
            self.walk = Some(start(move || walk(named, query)));
            self.asked = Some(asked);
        }
        let walk = self.walk.as_ref().and_then(peek);
        let Some((of, models)) = of(state).zip(crate::render::gta6_meshlets::drawn_level(state))
        else {
            return building();
        };
        if self.arena.as_ref().map(|(was, ..)| *was) != Some(of) {
            let triangles = triangles_of(models.1);
            self.built += 1;
            let made = start(move || SceneArena::build(&triangles).map_err(|e| e.to_string()));
            self.arena = Some((of, self.built, made));
        }
        let Some((_, number, arena)) = &self.arena else {
            return building();
        };
        let number = *number;
        let said = match (walk, peek(arena)) {
            (Some(Err(why)), _) if self.walk_queried() => self.fall_back(why),
            (Some(Err(why)), _) | (_, Some(Err(why))) => shadow_map(&why),
            (Some(Ok(walk)), Some(Ok(arena))) => {
                self.hand((device, queue), scene, (walk, arena, number))
            }
            _ => building(),
        };
        match state.anim.skin_matrices.is_empty() {
            true => said,
            false => format!(
                "{said}\n{}",
                tr!("The traced shadow is of the model at rest: a posed model's triangles are not computed on the CPU")
            ),
        }
    }

    /// Whether the walk asked for last tests its nodes by the device's query.
    fn walk_queried(&self) -> bool {
        self.asked.as_ref().is_some_and(|(_, query)| *query)
    }

    /// The walk through the device's query did not run, for `why`: the walk is lowered again to test its nodes in software.
    fn fall_back(&mut self, why: String) -> String {
        tracing::warn!(%why, "the traced walk through the device's query does not run; it tests its nodes in software");
        self.query_failed = Some(why);
        building()
    }

    /// Hand `scene` the pass of the walk over the arena `number`, where it has not that one.
    fn hand(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        scene: Option<&MeshletScene>,
        (walk, arena, number): (Arc<WalkModule>, Arc<SceneArena>, u64),
    ) -> String {
        let Some(scene) = scene else {
            return building();
        };
        if let Some((at, why)) = &self.refused {
            if *at == number {
                return shadow_map(why);
            }
        }
        let queried = self.walk_queried();
        if scene.traced_of() != Some(number) {
            match TracedMask::new(device, queue, walk, arena, number) {
                Ok(traced) => scene.trace_with(Some(traced)),
                Err(why) if queried => {
                    scene.trace_with(None);
                    return self.fall_back(why);
                }
                Err(why) => {
                    scene.trace_with(None);
                    let said = shadow_map(&why);
                    self.refused = Some((number, why));
                    return said;
                }
            }
        }
        match scene.traced_state() {
            Some(Some(Ok(()))) => format!(
                "{}\n{}",
                tr!("The sun's shadow: traced by GTA VI's own walk, translated, under the editor's rays in a compute pass; its cutout surfaces are traced solid"),
                body(queried)
            ),
            Some(Some(Err(why))) if queried => {
                scene.trace_with(None);
                self.fall_back(why)
            }
            Some(Some(Err(why))) => shadow_map(&why),
            _ => building(),
        }
    }
}

/// Advance the traced writer of the sun's mask for this frame of `scene`, where the aid asks for
/// it: whether what the Shaded line says of it changed.
pub(crate) fn refresh(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    scene: Option<&MeshletScene>,
    state: &AppState,
) -> bool {
    let mut held = GLUE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(asked) = trace::traced_aid() else {
        return false;
    };
    let glue = held.get_or_insert_with(Glue::default);
    let line = glue.advance((device, queue), scene, state, asked);
    if glue.line.as_ref() == Some(&line) {
        return false;
    }
    tracing::info!(said = %line, "the sun's mask of a GTA VI model");
    glue.line = Some(line);
    true
}

/// `said` with what the traced writer says of the sun's mask, where the aid asks for one.
pub(crate) fn said(said: String) -> String {
    let held = GLUE.lock().unwrap_or_else(PoisonError::into_inner);
    match held.as_ref().and_then(|glue| glue.line.as_ref()) {
        Some(line) => format!("{said}\n{line}"),
        None => said,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{Geometry, Palette};

    fn geometry(triangles: Vec<[u32; 3]>, positions: Vec<[f32; 3]>) -> Geometry {
        Geometry {
            shader: 0,
            vertex_count: positions.len() as u32,
            channels: Vec::new(),
            triangles,
            positions: Ok(positions),
            vertices: Default::default(),
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        }
    }

    #[test]
    fn the_arena_is_made_of_every_drawn_triangle_whose_corners_read() {
        let square = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        let model = Model {
            geometries: vec![
                Ok(geometry(vec![[0, 1, 2], [1, 3, 2]], square.clone())),
                // A corner past the vertices, and one that is no number: neither is traced.
                Ok(geometry(
                    vec![[0, 1, 9], [0, 1, 2]],
                    vec![[0.0; 3], [1.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]],
                )),
            ],
            palette: Ok(Palette::Own),
            matrix_index: 0,
        };
        let triangles = triangles_of(std::slice::from_ref(&model));
        let numbered: Vec<(u32, u32)> = triangles
            .iter()
            .map(|t| (t.geometry, t.primitive))
            .collect();
        assert_eq!(numbered, [(0, 0), (0, 1)]);
        assert!(triangles.iter().all(|t| t.opaque));
        assert_eq!(triangles[1].corners, [square[1], square[3], square[2]]);
        // And the arena built of them is walked.
        let arena = SceneArena::build(&triangles).unwrap();
        assert!(arena.words().len() > 64);
    }
}
