//! The Map workspace's state: the maps of a maps content as the last scan read them, what is selected in them, how the manipulator holds a
//! placement, and the edits made to the map files with the Undo that takes each back.
//!
//! ⚠ **Thin by design.** Every rule, every read and every write of a map file is `or-maps`'s, through the typed world family of `or-io`;
//! this keeps what the scan read, what is selected and the steps Undo moves along, and asks for a change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use or_maps::edit::Pose;
use or_maps::gizmo::{Handle, Mode};
use or_maps::operation::Operation;
use or_maps::{Depth, Facts, Finding, MapSet, Maps, Placed, Scene};
use or_project::{Container, Module, ModuleKind};
use or_renderer::{LineVertex, OrbitCamera};
use or_source::game_import::{Created, Report, Verdict, Watch};

use super::pack_import::{self, each_file, made, refused, PackJob};
use super::Workbench;
use crate::state::AppState;
use crate::ui::dock::Workspace;

/// What the scan read of one maps content.
#[derive(Debug, Clone)]
pub struct MapsRead {
    /// The id of the content.
    pub module: String,
    pub folder: PathBuf,
    /// The folder is another project's, which alone writes it.
    pub linked: bool,
    pub read: Result<MapsPack, String>,
}

/// A maps content read: its model, what the disk said of its sources, what is wrong with it and what each set places.
#[derive(Debug, Clone)]
pub struct MapsPack {
    pub model: Maps,
    pub facts: Facts,
    pub findings: Vec<Finding>,
    /// The scene of each set, by the id of the set.
    pub scenes: Vec<(String, Scene)>,
}

impl MapsPack {
    pub fn scene(&self, set: &str) -> Option<&Scene> {
        self.scenes
            .iter()
            .find(|(id, _)| id == set)
            .map(|(_, scene)| scene)
    }
}

/// Every maps content of `container`, read in the project in `project_folder`.
pub fn read_all(container: &Container, project_folder: &Path) -> Vec<MapsRead> {
    (container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Maps)
        .map(|module| MapsRead {
            module: module.id.clone(),
            folder: or_maps::module_folder(project_folder, module).unwrap_or_default(),
            linked: module.is_link(),
            read: or_maps::planned(project_folder, module, Depth::Open, None)
                .map(|(model, facts, planned)| {
                    let scenes = model
                        .maps
                        .iter()
                        .map(|set| (set.id.clone(), or_maps::scene(set, &facts)))
                        .collect();
                    MapsPack {
                        model,
                        facts,
                        findings: planned.findings,
                        scenes,
                    }
                })
                .map_err(|e| e.to_string()),
        })
        .collect()
}

/// One change Undo takes back: what it was called, and the operations that take it back and make it again.
#[derive(Debug, Clone)]
pub struct Step {
    pub label: String,
    /// The folder of the pack the operations name their sources in.
    pub folder: PathBuf,
    /// The inverses the change gave, the last first, each of which states the state of its source in the pack's session that it applies to.
    pub undo: Vec<Operation>,
    /// What makes the change again: its operations, then the inverses `undo` gave when it was applied.
    pub redo: Vec<Operation>,
}

/// How many steps Undo keeps.
pub(super) const KEPT: usize = 50;

/// What is selected in the shown set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// The source folder of the module the thing is in, with `/`.
    pub source: String,
    pub kind: PickKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    /// The entity at this index of a map file.
    Entity(usize),
    /// The archetype at this index of a type file.
    Archetype(usize),
}

/// How the manipulator holds a placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapping {
    pub grid_on: bool,
    /// The step of a position in metres.
    pub grid: f32,
    pub turn_on: bool,
    /// The step of a turn in degrees.
    pub turn: f32,
    pub scale_on: bool,
    pub scale: f32,
    /// Whether a move holds the entity on the ground.
    pub ground: bool,
    /// The height of the ground where nothing else is under the entity.
    pub floor: f32,
}

impl Default for Snapping {
    fn default() -> Self {
        Self {
            grid_on: true,
            grid: 0.5,
            turn_on: true,
            turn: 15.0,
            scale_on: false,
            scale: 0.1,
            ground: false,
            floor: 0.0,
        }
    }
}

impl Snapping {
    /// The snap for the manipulator, the grid held or not as the person ticked, inverted for as long as Ctrl is down.
    pub fn for_drag(&self, invert: bool) -> or_maps::gizmo::Snap {
        let on = |on: bool| on != invert;
        or_maps::gizmo::Snap {
            grid: on(self.grid_on).then_some(self.grid),
            degrees: on(self.turn_on).then_some(self.turn),
            scale: on(self.scale_on).then_some(self.scale),
        }
    }
}

/// A placement being dragged: the pose the entity is shown at until the drag is let go and written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preview {
    pub index: usize,
    pub pose: Pose,
}

/// The plan view's window on the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanView {
    /// The world position at the middle of the plan.
    pub centre: [f32; 2],
    /// How many pixels a metre is.
    pub pixels_per_metre: f32,
    /// Set once the plan has been fitted to what the set places.
    pub fitted: bool,
}

impl Default for PlanView {
    fn default() -> Self {
        Self {
            centre: [0.0; 2],
            pixels_per_metre: 1.0,
            fitted: false,
        }
    }
}

/// The Map workspace's own state, reset with the project.
#[derive(Default)]
pub struct MapsWork {
    /// The maps content the workspace shows; the first when none is chosen.
    pub module: Option<String>,
    /// The id of the set shown; the first when none is chosen.
    pub set: Option<String>,
    pub selected: Option<Pick>,
    /// The text the entity and archetype lists are filtered by.
    pub filter: String,
    /// The source folders whose entities or archetypes the list folds away.
    pub folded: BTreeSet<String>,
    /// The changes Undo can take back, newest last.
    pub undo: Vec<Step>,
    /// What Undo took back and Redo can make again, newest last; a new change empties it.
    pub redo: Vec<Step>,
    /// Bumped whenever a map file changes, so what was read of it is read again.
    pub generation: u64,
    /// The packs an edit was made to, each held open with what Save is yet to write.
    pub(super) sessions: super::maps_session::Sessions,
    pub tool: Mode,
    pub snap: Snapping,
    /// The placement being dragged, if one is.
    pub preview: Option<Preview>,
    /// The handle of the manipulator under the pointer.
    pub hovered: Option<Handle>,
    pub camera: OrbitCamera,
    /// The camera is placed on the set the next time it is on screen.
    pub needs_frame: bool,
    /// The set the camera was last placed on, so that one shown for the first time (a project just opened) is framed too.
    pub framed_set: Option<String>,
    pub plan: PlanView,
    /// The lines the scene panel built for this frame, which the viewport draws over the set.
    pub overlay: Vec<LineVertex>,
    /// Where the scene was last drawn on the screen: the left, the top, the width and the height in points.
    pub view: Option<[f32; 4]>,
    /// Whether the entities of the levels of detail below the full one are drawn.
    pub show_lod: bool,
    /// The names being typed into the selection's fields, kept until the field is left.
    pub typing: Option<(String, String)>,
    /// The scanned GTA VI map body the active document is, which the screen shows read-only in place of a set.
    pub scanned: Option<crate::state::gta6_map::ScannedMap>,
    /// Whether the scanned map's models are drawn in place round the entity selected, and those read.
    pub in_place: crate::state::gta6_map::links::InPlace,
}

impl Workbench {
    /// The maps contents as the last scan read them.
    pub fn maps_read(&self) -> &[MapsRead] {
        self.scan.as_ref().map_or(&[], |scan| &scan.maps)
    }
}

/// The files under `paths`: a folder is walked for the files in it, a bounded way down.
fn files_under(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    const DEPTH: usize = 8;
    const FILES: usize = 2000;
    fn walk(path: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if out.len() >= FILES {
            return;
        }
        if path.is_dir() {
            if depth == 0 {
                return;
            }
            let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .collect();
            entries.sort();
            for entry in entries {
                walk(&entry, depth - 1, out);
            }
        } else {
            out.push(path.to_path_buf());
        }
    }
    let mut out = Vec::new();
    for path in paths {
        walk(&path, DEPTH, &mut out);
    }
    out
}

impl AppState {
    /// Whether the project holds maps content.
    pub fn has_maps(&self) -> bool {
        (self.project_carried.container.modules.iter()).any(|m| m.kind == ModuleKind::Maps)
    }

    /// The maps content the workspace shows: the one chosen, else the first.
    pub fn maps_pack(&self) -> Option<&MapsRead> {
        let packs = self.workbench.maps_read();
        let chosen = self.workbench.maps.module.as_deref();
        chosen
            .and_then(|id| packs.iter().find(|p| p.module == id))
            .or_else(|| packs.first())
    }

    /// The set shown and the content it is in; none while the content is unread or has no set.
    pub fn shown_set(&self) -> Option<(&MapsRead, &MapSet)> {
        let pack = self.maps_pack()?;
        let model = &pack.read.as_ref().ok()?.model;
        let chosen = self.workbench.maps.set.as_deref();
        let set = chosen
            .and_then(|id| model.map(id))
            .or_else(|| model.maps.first())?;
        Some((pack, set))
    }

    /// What the shown set places, or the scanned GTA VI map when one is shown.
    pub fn shown_scene(&self) -> Option<&Scene> {
        if let Some(scanned) = &self.workbench.maps.scanned {
            return Some(&scanned.scene);
        }
        let (pack, set) = self.shown_set()?;
        pack.read.as_ref().ok()?.scene(&set.id)
    }

    /// The entity selected, as the scene places it.
    pub fn selected_placed(&self) -> Option<&Placed> {
        let pick = self.workbench.maps.selected.as_ref()?;
        let PickKind::Entity(index) = pick.kind else {
            return None;
        };
        let scene = self.shown_scene()?;
        scene.placed.get(scene.find(&pick.source, index)?)
    }

    /// The pose of the entity selected, or the one being dragged.
    pub fn selected_pose(&self) -> Option<Pose> {
        let placed = self.selected_placed()?;
        if let Some(preview) = self
            .workbench
            .maps
            .preview
            .filter(|p| p.index == placed.index)
        {
            return Some(preview.pose);
        }
        Some(pose_of_box(&placed.obb))
    }

    /// Whether the Map screen has anything to show: maps content, or a scanned GTA VI map.
    pub fn shows_maps(&self) -> bool {
        self.has_maps() || self.workbench.maps.scanned.is_some()
    }

    /// Select `pick` in what the screen shows: the scanned GTA VI map, else the set `set` of the content `module`.
    pub fn pick_shown(&mut self, module: Option<String>, set: Option<String>, pick: Option<Pick>) {
        if self.workbench.maps.scanned.is_some() {
            self.workbench.maps.selected = pick;
        } else if let (Some(module), Some(set)) = (module, set) {
            self.select_in_maps(&module, &set, pick);
        }
    }

    /// Select `pick` in the set `set` of the content `module`, or nothing.
    pub fn select_in_maps(&mut self, module: &str, set: &str, pick: Option<Pick>) {
        let work = &mut self.workbench.maps;
        if work.selected != pick {
            work.typing = None;
            work.preview = None;
        }
        work.module = Some(module.to_string());
        work.set = Some(set.to_string());
        work.selected = pick;
    }

    /// Make the project hold maps: a `maps` folder with an empty model, a content for it, and a place for it in every build.
    pub fn add_map_content(&mut self) {
        if self.refused_as_read_only() {
            return;
        }
        let Some(project) = self.project_path.clone() else {
            return;
        };
        let taken: Vec<&str> = (self.project_carried.container.modules.iter())
            .map(|m| m.id.as_str())
            .collect();
        let id = super::commands::unique("maps", taken.into_iter());
        let module = Module::maps(&id, &id);
        let folder = or_project::project_folder(&project).join(&id);
        let refused = or_project::links::refuse_write(
            &self.project_carried.container,
            &project,
            &folder.join(or_maps::FILE),
        );
        if let Err(why) = refused {
            self.show_error_toast(why);
            return;
        }
        if let Err(why) =
            or_maps::adopt::start_module(&or_project::project_folder(&project), &module)
        {
            self.show_error_toast(why);
            return;
        }
        let container = &mut self.project_carried.container;
        container.modules.push(module);
        for output in &mut container.outputs {
            output.take.push(id.clone());
        }
        self.workbench.want_scan();
        self.workbench.maps.module = Some(id.clone());
        self.record_step(tr!("Added the maps content {id}", id = id));
        self.active_workspace = Workspace::Map;
        self.show_toast(tr!(
            "Maps added: drop .ymap and .ytyp files on the Map screen to fill it"
        ));
    }

    /// The folder of the maps content to write, none having said why when the window only reads the project or the content is linked.
    pub(super) fn maps_folder_to_write(&mut self) -> Option<(String, PathBuf)> {
        if self.refused_as_read_only() || self.workbench.maps.scanned.is_some() {
            return None;
        }
        // ⚠ The content as the project file states it, not as the scan read it: a pack just made has not been scanned yet.
        let chosen = self.workbench.maps.module.as_deref();
        let modules = &self.project_carried.container.modules;
        let module = chosen
            .and_then(|id| {
                modules
                    .iter()
                    .find(|m| m.id == id && m.kind == ModuleKind::Maps)
            })
            .or_else(|| modules.iter().find(|m| m.kind == ModuleKind::Maps));
        let (Some(module), Some(project)) = (module, self.project_path.as_ref()) else {
            self.show_warning_toast(tr!(
                "The project holds no maps: Project ▸ Add a map pack… adds some"
            ));
            return None;
        };
        if module.is_link() {
            let why = tr!("These maps are linked from another project, which alone edits them: open that project to change them");
            self.show_warning_toast(why);
            return None;
        }
        let folder = or_maps::module_folder(&or_project::project_folder(project), module);
        match folder {
            Ok(folder) => Some((module.id.clone(), folder)),
            Err(why) => {
                self.show_error_toast(why.to_string());
                None
            }
        }
    }

    /// Add the game files `paths` (and the files in the folders among them) to the maps: each becomes a source, and the sources join the set shown, made by the import job.
    ///
    /// ⚠ Not in the frame: a map file is exported, compiled, read again and compared, which takes seconds for a large one.
    pub fn add_maps(&mut self, paths: Vec<PathBuf>) {
        let Some((module, folder)) = self.maps_folder_to_write() else {
            return;
        };
        let files = files_under(paths);
        if files.is_empty() {
            self.show_toast(tr!("No map file was found among those dropped"));
            return;
        }
        let into = self.workbench.maps.set.clone();
        self.start_pack_import(
            ModuleKind::Maps,
            module,
            folder,
            PackJob::Add { files, into },
        );
    }

    /// The import of map files ended: its report is the import report, its step takes back what it made, and the Map screen shows the pack.
    pub(super) fn finish_maps_import(&mut self, module: String, report: Report) {
        let [made, _, _, refused] = super::imports::counts(&report);
        let first = || {
            (report.files.iter())
                .find(|file| file.verdict == Verdict::Refused)
                .map(|file| format!("{} {}", file.input, file.reason))
                .unwrap_or_default()
        };
        let step = match (made, refused) {
            (0, _) => None,
            (made, 0) => Some(tr!(
                "Added {made} to the maps",
                made = crate::state::front_door::counted(made, "source")
            )),
            (made, refused) => Some(tr!(
                "Added {made}; {refused} could not be added: {first}",
                made = crate::state::front_door::counted(made, "source"),
                refused = refused,
                first = first()
            )),
        };
        self.workbench.maps.generation += 1;
        self.workbench.maps.module = Some(module);
        self.workbench.maps.needs_frame = true;
        self.active_workspace = Workspace::Map;
        self.finish_import_as(report, step);
    }

    /// A source folder that Undo took away is no source of a set: each pack goes back to what it held before the files were added.
    pub(super) fn forget_removed_map_sources(&mut self, created: &[Created]) {
        self.edit_map_packs(|folder| {
            let gone = pack_import::pack_sources(
                folder,
                created
                    .iter()
                    .map(|made| &made.path)
                    .filter(|path| !path.exists()),
            );
            !gone.is_empty() && or_maps::import::forget(folder, &gone).unwrap_or(false)
        });
    }

    /// A source folder that Redo put back is a source of its set again.
    pub(super) fn adopt_restored_map_sources(&mut self, restored: &[PathBuf]) {
        self.edit_map_packs(|folder| {
            let dirs: Vec<PathBuf> =
                pack_import::pack_sources(folder, restored.iter().filter(|p| p.is_dir()))
                    .into_iter()
                    .map(|name| folder.join(name))
                    .collect();
            !dirs.is_empty() && or_maps::import::learn(folder, &dirs, None).is_ok()
        });
    }

    /// Let `edit` change each maps content the project holds in its own folder; it says whether it did, and the scan is asked for again then.
    fn edit_map_packs(&mut self, edit: impl Fn(&Path) -> bool) {
        let Some(project) = self.project_path.as_deref() else {
            return;
        };
        let root = or_project::project_folder(project);
        let mut changed = false;
        for module in (self.project_carried.container.modules.iter())
            .filter(|m| m.kind == ModuleKind::Maps && !m.is_link())
        {
            if let Ok(folder) = or_maps::module_folder(&root, module) {
                changed |= edit(&folder);
            }
        }
        if changed {
            self.workbench.maps.generation += 1;
            self.workbench.want_scan();
        }
    }

    /// Step the camera of the Map workspace by what the pointer did, and place it on the set when it is asked to.
    pub(crate) fn step_map_camera(
        &mut self,
        inputs: crate::render::viewport::CameraInputs,
    ) -> &OrbitCamera {
        let frame_inputs = inputs.frame_active || inputs.frame_selection;
        let bounds = self.shown_scene().and_then(|s| s.bounds);
        let selected = self.selected_placed().map(|p| p.obb.position);
        let set = self.shown_set().map(|(_, set)| set.id.clone());
        let work = &mut self.workbench.maps;
        inputs.apply_to(&mut work.camera);
        if bounds.is_some() && work.framed_set != set {
            work.framed_set = set;
            work.needs_frame = true;
        }
        if frame_inputs {
            work.needs_frame = true;
        }
        if work.needs_frame {
            if let Some((lo, hi)) = bounds {
                let centre = (lo + hi) * 0.5;
                let radius = ((hi - lo).length() * 0.5).max(5.0);
                match (inputs.frame_selection, selected) {
                    (true, Some(at)) => work.camera.frame_sphere(at, 8.0),
                    _ => work.camera.frame_sphere(centre, radius),
                }
                work.needs_frame = false;
            }
        }
        &work.camera
    }
}

/// The pose a box stands at.
pub fn pose_of_box(obb: &or_maps::gizmo::Obb) -> Pose {
    Pose {
        position: obb.position,
        rotation: obb.rotation,
        scale_xy: obb.scale.x,
        scale_z: obb.scale.z,
    }
}

/// The work of the import job for map files: each of `files` becomes a source of the pack in `folder`, one at a time, so that the job names the file in hand and a stop keeps what is made; then the pack learns of the sources.
///
/// ⚠ A file with no source form, or that cannot be read, is refused in the report with its sentence and the others still come in.
pub(super) fn add_to_pack(
    folder: &Path,
    files: &[PathBuf],
    into: Option<&str>,
    watch: &Watch<'_>,
) -> Report {
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut report = each_file(files, watch, |path, name, report| {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return refused(report, name, format!("cannot be read: {e}")),
        };
        let incoming = or_maps::import::Incoming {
            name,
            bytes: &bytes,
            from_game: false,
            virtual_path: None,
        };
        match or_maps::import::add_one(folder, &incoming) {
            Ok(dir) => {
                sources.push(dir.clone());
                made(report, name, dir);
            }
            Err(why) => refused(report, name, why),
        }
    });
    if let Err(why) = or_maps::import::learn(folder, &sources, into) {
        report
            .notes
            .push(format!("the pack of maps was not updated: {why}"));
    }
    report
}

/// What a maps content holds as the last scan read it, in words.
pub fn held_by(workbench: &Workbench, module: &Module) -> String {
    match workbench.maps_read().iter().find(|p| p.module == module.id) {
        None => tr!("Reading…").to_string(),
        Some(pack) => match &pack.read {
            Ok(read) => super::held(&module.kind, read.model.maps.len()),
            Err(_) => tr!("could not be read").to_string(),
        },
    }
}

/// How many map sets a maps content holds as the disk says, for a card of the start page that opens nothing.
pub fn on_disk(project: &Path, module: &Module) -> Option<usize> {
    let folder = or_maps::module_folder(&or_project::project_folder(project), module).ok()?;
    Maps::read(&folder).ok().map(|pack| pack.maps.len())
}
