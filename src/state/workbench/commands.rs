//! What the project's commands do, and the poll that keeps the workbench current each frame.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use or_project::container::is_name;
use or_project::{
    build_root, built_resources, clean, project_folder, shadow_root, Module, ModuleKind,
    OutputKind, Target,
};
use or_source::cook::Recook;

use super::builds::{Job, LastBuild};
use super::cook::Cooks;
use super::scan::Kept;
use super::watch::{self, PackWatch};
use super::{leaf, scan, Question, Signature};
use crate::state::front_door::new_project::GAME_KEY;
use crate::state::front_door::GameChoice;
use crate::state::retail::{self, PathClass};
use crate::state::toasts::ToastAction;
use crate::state::{AppState, Document, DocumentKind, ProjectDocument};
use crate::ui::dock::Workspace;
use crate::ui::panels::registry::ids;

/// Whether a document of this kind is somewhere to look, which Open shows and no project holds as a file.
pub(in crate::state) fn is_a_place(kind: DocumentKind) -> bool {
    matches!(kind, DocumentKind::GameRoot | DocumentKind::Folder)
}

impl AppState {
    /// The open project's name, which is its file's.
    pub fn project_name(&self) -> Option<String> {
        let path = self.project_path.as_ref()?;
        let stem = path.file_stem().and_then(|s| s.to_str());
        Some(stem.unwrap_or(tr!("Untitled")).to_string())
    }

    /// The files the open project holds: its documents, and every file open in a tab, as a save would write them.
    ///
    /// ⚠ The install is the app's own mount, an opened folder is a place to look in and an archive entry's path is a display path no load can reopen, so none is a file.
    pub fn member_paths(&self) -> Vec<PathBuf> {
        let carried = self
            .project_carried
            .members
            .iter()
            .filter(|m| !m.kind.is_some_and(is_a_place))
            .map(|m| m.path.clone());
        let in_tabs = self
            .documents
            .iter()
            .filter(|d| !is_a_place(d.kind) && d.rpf_entry_binding.is_none())
            .filter_map(|d| d.path.clone());
        let mut seen = HashSet::new();
        carried
            .chain(in_tabs)
            .filter(|path| seen.insert(path.clone()))
            .collect()
    }

    /// Whether `doc` is a file of the open project: one the editor writes and the game's edit mode does not guard.
    pub fn is_project_file(&self, doc: &Document) -> bool {
        self.project_path.is_some()
            && doc.rpf_entry_binding.is_none()
            && !matches!(
                doc.kind,
                DocumentKind::GameRoot
                    | DocumentKind::Rpf
                    | DocumentKind::Folder
                    | DocumentKind::Compare
            )
            && doc
                .path
                .as_deref()
                .is_some_and(|path| retail::classify(path, &self.prefs) == PathClass::Outside)
    }

    fn workbench_signature(&self) -> Signature {
        Signature {
            members: self.project_carried.members.len(),
            open: self.documents.iter().filter(|d| d.path.is_some()).count(),
        }
    }

    /// Keeps the workbench current: reads a scan's result, asks for the next scan, hands saved files to the cook and takes what builds and cooks finished.
    pub fn poll_workbench(&mut self) {
        if self.workbench.project != self.project_path {
            self.workbench.reset(self.project_path.clone());
            self.hold_the_open_project();
            self.begin_history();
        }
        self.follow_the_offered_workspaces();
        self.follow_the_arrangements_of_shared_screens();
        self.remember_workspace();
        self.take_ended_builds();
        self.poll_wardrobe();
        self.poll_mesh();
        self.poll_textures();
        self.poll_dressed();
        self.poll_imports();
        self.poll_reopen();
        self.poll_geometry();
        self.poll_originals();
        let Some(project) = self.project_path.clone() else {
            return;
        };
        if self.workbench.cooks.is_none() {
            self.workbench.cooks = Some(Cooks::new(shadow_root(&project)));
        }
        let now = Instant::now();

        let unsaved: HashSet<PathBuf> = (self.documents.iter())
            .filter(|d| d.dirty)
            .filter_map(|d| d.path.clone())
            .collect();
        if unsaved != self.workbench.unsaved {
            self.workbench.unsaved = unsaved;
            self.workbench.findings_stale = true;
        }

        if self.workbench.take_scan() {
            self.follow_the_map_sources_on_disk();
        }
        self.follow_moved_members();
        let signature = self.workbench_signature();
        // ⚠ The Wardrobe's packs are heard when they change; the clock stays for the Project workspace and for a pack that cannot be watched.
        let watch = self.workbench.pack_watch.get_or_insert_with(PackWatch::new);
        let heard = watch.changed(now);
        let watching = watch.covers();
        if heard {
            self.workbench.wanted = true;
        }
        let since = (self.workbench.last_scan).map(|at| now.duration_since(at));
        let stale = watch::due_by_the_clock(self.active_workspace, watching, since);
        let wanted = self.workbench.wanted || self.workbench.signature != Some(signature) || stale;
        if wanted && self.workbench.scanning.is_none() {
            self.workbench.wanted = false;
            self.workbench.signature = Some(signature);
            self.workbench.last_scan = Some(now);
            let extensions = self.extensions();
            let ask = scan::Ask {
                folder: project_folder(&project),
                project,
                container: self.project_carried.container.clone(),
                members: self.member_paths(),
                extensions,
            };
            self.workbench.scanning = Some(scan::start(ask));
        }
        self.finish_sharing_check();

        let cooked = match self.workbench.cooks.as_mut() {
            Some(cooks) => cooks.poll(now),
            None => Vec::new(),
        };
        for recook in cooked {
            self.took_recook(recook);
        }

        if self.workbench.findings_stale {
            self.workbench.recompute_findings();
        }
    }

    /// What one finished recook changes: a line in the history, every open tab of the file it wrote read again, and the rows and problems read afresh.
    ///
    /// ⚠ A failed cook reads nothing again: it kept the previous output, and the tabs keep showing it.
    pub(in crate::state) fn took_recook(&mut self, recook: Recook) {
        match recook {
            Recook::Cooked { source, output } => {
                self.workbench
                    .note(tr!("Compiled {source}", source = leaf(&source)));
                // ⚠ A model opened before its textures were first cooked was told there were none: it is allowed to ask for them again.
                self.attempted_texture_project.clear();
                let was_waiting = self.reload_conflicts.contains(&output);
                let read = self.reload_open_file(&output, false);
                // ⚠ A tab with unsaved edits asks first, and a file OpenRage recooked itself is not a file somebody else changed.
                if !was_waiting && self.reload_conflicts.contains(&output) {
                    self.workbench.refreshed.insert(output.clone());
                }
                if let Err(why) = read {
                    let said = tr!(
                        "Could not read {output} again: {why}",
                        why = why,
                        output = leaf(&output)
                    );
                    self.workbench.note(said.clone());
                    if let Some(cooks) = self.workbench.cooks.as_mut() {
                        cooks.reload_failed(&source, said);
                    }
                }
            }
            Recook::Failed { source, problems } => {
                let said = tr!(
                    "Could not compile {source}: {why}, and its last output stays",
                    source = leaf(&source),
                    why = crate::state::front_door::counted(problems.len(), "problem")
                );
                self.workbench.note(said);
            }
            Recook::Unchanged { .. } | Recook::Ignored(_) => {}
        }
        self.workbench.findings_stale = true;
        self.workbench.wanted = true;
    }

    /// What the builds that ended since the last frame said: a toast, and a line in the project's history.
    ///
    /// ⚠ Taken even with no project open, or a build that ended after its project was closed would stay running for ever.
    fn take_ended_builds(&mut self) {
        // ⚠ A run outlives the project that asked for it, and what it built is that project's: here it is said once, and written nowhere in the project that is open.
        let owner = self
            .workbench
            .builds
            .running_folder()
            .map(Path::to_path_buf);
        let open = self.project_path.as_deref().map(project_folder);
        let closed = owner.filter(|owner| open.as_ref() != Some(owner));
        let names = self.workbench.builds.running_names();
        for (id, last) in self.workbench.builds.poll() {
            if let Some(owner) = &closed {
                self.workbench.builds.forget(&id);
                let project = leaf(owner);
                let name = names.get(&id).unwrap_or(&id);
                match last {
                    LastBuild::Built(done) => self.show_toast(tr!(
                        "Built {name} of {project}, which was closed: {files}, {checked} read back",
                        name = name,
                        project = project,
                        files = crate::state::front_door::counted(done.files() as usize, "file"),
                        checked = done.checked()
                    )),
                    LastBuild::Failed(reasons) => self.show_warning_toast(tr!(
                        "The build {name} of {project}, which was closed, stopped: {why}",
                        name = name,
                        project = project,
                        why = crate::state::front_door::spoken(
                            reasons.first().map_or("", String::as_str)
                        )
                    )),
                }
                continue;
            }
            match last {
                LastBuild::Built(done) => {
                    let files = crate::state::front_door::counted(done.files() as usize, "file");
                    let read_back = if done.checked() as u64 == done.files() {
                        tr!("all read back").to_string()
                    } else {
                        tr!("{checked} read back", checked = done.checked())
                    };
                    // ⚠ The history and the toast say the build by the name of the folder it wrote, which is what a person calls it.
                    let name = super::build_name(&self.project_carried.container, &id);
                    self.workbench.note(tr!(
                        "Built {name}: {files}, {read_back}",
                        name = name,
                        files = files,
                        read_back = read_back
                    ));
                    match done.pack_problems().first() {
                        Some(first) => {
                            self.show_error_toast(tr!(
                                "Built {name}, but the pack check found a problem: {first}",
                                name = name,
                                first = first
                            ));
                        }
                        None => self.show_toast_with_action(
                            tr!(
                                "Built {name}: {files}, {read_back}",
                                name = name,
                                files = files,
                                read_back = read_back
                            ),
                            ToastAction::ShowBuild,
                        ),
                    }
                }
                LastBuild::Failed(reasons) => {
                    let first = reasons.first().cloned().unwrap_or_default();
                    let name = super::build_name(&self.project_carried.container, &id);
                    self.workbench.note(tr!("Build {name} stopped: {first}; the last good build is as it was ({count} reason(s))", name = name, first = first, count = reasons.len()));
                    self.show_error_toast(tr!(
                        "Build {name} stopped: {why}",
                        name = name,
                        why = crate::state::front_door::spoken(&first)
                    ));
                    // The step it stopped at and its reasons are the Build tab's, which a failure brings to the front where the screen docks it; a screen without it keeps its arrangement.
                    let build = crate::ui::panels::registry::ids::BUILD;
                    if self.workspace_docks.shows(self.active_workspace, build) {
                        self.workspace_docks.reveal(self.active_workspace, build);
                    }
                }
            }
            self.workbench.wanted = true;
            // ⚠ A build that stopped is a row of Problems until another goes through, so it is read again at once.
            self.workbench.findings_stale = true;
        }
    }

    /// The id of the build the Build button builds: the one chosen in the panel, else the first.
    pub fn active_output_id(&self) -> Option<String> {
        let outputs = &self.project_carried.container.outputs;
        (self.workbench.selected_output.as_ref())
            .filter(|id| outputs.iter().any(|o| &o.id == *id))
            .cloned()
            .or_else(|| outputs.first().map(|o| o.id.clone()))
    }

    fn build_job(&mut self, outputs: Vec<String>) -> Option<Job> {
        let extensions = self.extensions();
        let project = self.project_path.as_ref()?;
        Some(Job {
            container: self.project_carried.container.clone(),
            folder: project_folder(project),
            root: build_root(project),
            members: self.member_paths(),
            outputs,
            remap: Arc::clone(&self.shader_defs.remap),
            extensions,
            reproducible: false,
        })
    }

    /// Show the folder the chosen build was written to in the file manager: its resource's folder, or `build/` when a split wrote several.
    pub fn show_the_build(&mut self) {
        let Some(project) = self.project_path.clone() else {
            return;
        };
        let root = build_root(&project);
        let folder = (self.active_output_id())
            .and_then(|id| {
                self.project_carried
                    .container
                    .output(&id)
                    .map(|o| root.join(&o.name))
            })
            .filter(|folder| folder.is_dir())
            .unwrap_or(root);
        crate::platform::shell::reveal(&folder);
    }

    /// Build the build the panel has chosen.
    pub fn build_selected(&mut self) {
        match self.active_output_id() {
            Some(id) => self.start_build(vec![id]),
            None => self.show_info_toast(tr!(
                "The project declares no build: Project ▸ Add build… adds one"
            )),
        }
    }

    /// Build every build the project declares, one after the other.
    pub fn build_all(&mut self) {
        let ids = (self.project_carried.container.outputs.iter())
            .map(|o| o.id.clone())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            self.show_info_toast(tr!(
                "The project declares no build: Project ▸ Add build… adds one"
            ));
        } else {
            self.start_build(ids);
        }
    }

    fn start_build(&mut self, outputs: Vec<String>) {
        if self.refused_as_read_only() {
            return;
        }
        if self.workbench.builds.is_running() {
            self.show_info_toast(tr!("A build is already running"));
            return;
        }
        let Some(job) = self.build_job(outputs.clone()) else {
            return;
        };
        self.workbench.builds.start(job);
        let names: Vec<String> = (outputs.iter())
            .map(|id| super::build_name(&self.project_carried.container, id))
            .collect();
        self.workbench
            .note(tr!("Building {names}", names = names.join(", ")));
    }

    /// Ask before removing every resource OpenRage built in the project's `build/` folder, naming them.
    pub fn ask_to_clean(&mut self) {
        let Some(project) = &self.project_path else {
            return;
        };
        // ⚠ Read here, not while the question draws, so a frame never lists a folder.
        let root = build_root(project);
        let resources = built_resources(&root).unwrap_or_default();
        let (files, bytes) = resources
            .iter()
            .map(|resource| extent(&root, resource))
            .fold((0, 0), |(f, b), (files, bytes)| (f + files, b + bytes));
        self.workbench.asking = Some(Question::Clean {
            resources,
            files,
            bytes,
        });
    }

    /// Remove every resource in the project's `build/` folder that a hidden marker says OpenRage built, whatever the project declares now, and nothing else.
    ///
    /// ⚠ Found by marker and not by the outputs' names: a build that was removed from the project, or
    /// a split output's `<name>_1`, is in `build/` and in no output's name.
    pub fn clean_builds(&mut self) {
        if self.refused_as_read_only() {
            return;
        }
        let Some(project) = self.project_path.clone() else {
            return;
        };
        if self.workbench.builds.is_running() {
            self.show_info_toast(tr!("Wait for the build to end before cleaning"));
            return;
        }
        let root = build_root(&project);
        let resources = match built_resources(&root) {
            Ok(resources) => resources,
            Err(e) => {
                self.show_error_toast(e.to_string());
                return;
            }
        };
        let mut removed = 0;
        let mut refused = Vec::new();
        for resource in &resources {
            match clean(&root, resource) {
                Ok(cleaned) => removed += cleaned.removed.len(),
                Err(e) => refused.push(e.to_string()),
            }
        }
        for output in &self.project_carried.container.outputs {
            self.workbench.builds.forget(&output.id);
        }
        self.workbench.findings_stale = true;
        self.workbench.want_scan();
        if let Some(first) = refused.first() {
            self.show_error_toast(first.clone());
        } else {
            self.show_toast(tr!(
                "Cleaned: {removed} folder(s) and marker(s) removed",
                removed = removed
            ));
        }
        self.workbench.note(tr!(
            "Cleaned the builds ({removed} removed)",
            removed = removed
        ));
    }

    /// The name a new build of `kind` for `target` is offered under: the project's, or one for what the build is when that is taken, and never `<name>_2`, where a split writes its second resource; none when the project's name cannot name a folder.
    pub fn default_build_name(&self, kind: &OutputKind, target: Option<Target>) -> String {
        let Some(base) = self.project_name().filter(|name| is_name(name)) else {
            return String::new();
        };
        // ⚠ The project's name as the New project dialog turns it into a resource's (`street-wear` is `street_wear`): a name offered raw is refused by the form that offers it for a singleplayer DLC, which takes no hyphen.
        let base = crate::state::front_door::new_project::resource_name_for(&base);
        (self.project_carried.container).free_build_name(&base, kind, target)
    }

    /// The edition a new build ships for: the game the project was made for, and as authored for both or none.
    pub fn default_edition(&self) -> Option<Target> {
        let game = self.project_carried.top.get(GAME_KEY)?.as_str()?;
        GameChoice::from_word(game)?.default_target()
    }

    /// Add `files` to the project's Files content, which is made if it is not there.
    ///
    /// ⚠ A file outside the project's folder has no place in a build, and one inside linked content is
    /// already read through the link, so neither is added: each is said, with what to do about it.
    pub fn add_files_to_project(&mut self, files: Vec<PathBuf>) -> Added {
        let done = self.add_files_without_a_step(files);
        if done.added > 0 {
            self.record_step(tr!("Added {added} file(s)", added = done.added));
        }
        done
    }

    /// [`Self::add_files_to_project`] for a caller that makes the step itself, as an import does.
    pub(super) fn add_files_without_a_step(&mut self, files: Vec<PathBuf>) -> Added {
        let mut done = Added::default();
        if let Some(why) = self.read_only_reason() {
            done.refused.push(why);
            return done;
        }
        let Some(project) = self.project_path.clone() else {
            return done;
        };
        let held = self.member_paths();
        for file in files {
            let twice = self.project_carried.members.iter().any(|m| m.path == file);
            if held.contains(&file) || twice {
                continue;
            }
            if let Some(why) =
                or_project::links::refuse_member(&self.project_carried.container, &project, &file)
            {
                done.refused.push(why);
                continue;
            }
            let kind = DocumentKind::from_path(&file);
            self.project_carried
                .members
                .push(ProjectDocument::new(file, kind));
            done.added += 1;
        }
        let added = done.added;
        if added == 0 {
            return done;
        }
        let container = &mut self.project_carried.container;
        // ⚠ A linked Files content holds another project's files: the project's own are a content of its own, which every build takes.
        if !container
            .modules
            .iter()
            .any(|m| m.kind == ModuleKind::Files && !m.is_link())
        {
            let id = unique("files", container.modules.iter().map(|m| m.id.as_str()));
            container.modules.push(Module::files(&id));
            for output in &mut container.outputs {
                output.take.push(id.clone());
            }
        }
        self.workbench.want_scan();
        done
    }

    /// Declare a build that writes `name` as a resource of `kind` from all the project's content, with every drawable and texture dictionary moved to `target` when there is one; its id.
    #[cfg(test)]
    pub fn add_build(
        &mut self,
        name: &str,
        kind: OutputKind,
        target: Option<Target>,
    ) -> Result<String, String> {
        let id = self.declare_build(name, kind, target)?;
        self.record_step(tr!("Added the build {name}", name = name));
        Ok(id)
    }

    /// [`Self::add_build`] for a caller that makes the step itself, once it has set the build up.
    pub(super) fn declare_build(
        &mut self,
        name: &str,
        kind: OutputKind,
        target: Option<Target>,
    ) -> Result<String, String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        let id = self
            .project_carried
            .container
            .add_build(name, kind, target)?;
        self.workbench.selected_output = Some(id.clone());
        self.workbench.want_scan();
        Ok(id)
    }

    /// Stop declaring the build `id`; what it already wrote stays until Clean.
    pub fn remove_build(&mut self, id: &str) {
        if self.refused_as_read_only() {
            return;
        }
        let name = super::build_name(&self.project_carried.container, id);
        if !self.project_carried.container.remove_build(id) {
            return;
        }
        self.workbench.builds.forget(id);
        self.workbench.findings_stale = true;
        if self.workbench.selected_output.as_deref() == Some(id) {
            self.workbench.selected_output = None;
        }
        self.workbench.want_scan();
        self.record_step(tr!("Removed the build {name}", name = name));
    }

    /// Select the asset with this key; none selects the project itself.
    pub fn select_asset(&mut self, key: Option<String>) {
        if self.workbench.selected != key {
            self.workbench.quick_look = false;
        }
        self.workbench.selected = key;
    }

    /// The selected asset's row, when one is selected and the scan still has it.
    pub fn selected_asset(&self) -> Option<&scan::AssetRow> {
        self.workbench.row(self.workbench.selected.as_deref()?)
    }

    /// Open the asset in `workspace` when that one shows its kind, else in the workspace that suits it; a source opens the file its cook wrote, and a garment opens the Wardrobe on it.
    pub fn open_asset(&mut self, key: &str, workspace: Option<Workspace>) {
        let Some(row) = self.workbench.row(key) else {
            return;
        };
        if let (Kept::Pack, Some(id)) = (row.kept, row.id.clone()) {
            let content = row.content.clone();
            self.show_garment(&content, id);
            return;
        }
        let path = match &row.output {
            Some(output) if output.is_file() => output.clone(),
            Some(_) => {
                self.show_info_toast(tr!("{name} is not compiled yet: save a file in it, or use Compile again, and it opens here", name = row.name));
                return;
            }
            None => row.path.clone(),
        };
        if !path.is_file() {
            self.show_error_toast(tr!(
                "{name} is not a file: {path}",
                name = row.name,
                path = path.display()
            ));
            return;
        }
        let Some(kind) = DocumentKind::from_path(&path) else {
            // ⚠ Loaded anyway it would open as bytes in a workspace this one never shows, and nothing would seem to happen.
            let words = match path.extension().and_then(|e| e.to_str()) {
                Some(extension) => tr!(".{extension} files", extension = extension),
                None => tr!("files with no extension").to_string(),
            };
            let said = tr!("{path} was not opened: OpenRage has no editor for {words}. Reveal asset shows where it is", words = words, path = leaf(&path));
            self.show_info_toast(said);
            return;
        };
        // ⚠ Set first, so the file opens where it is edited instead of bouncing off this workspace.
        self.active_workspace = workspace
            .filter(|ws| ws.handles_kind(kind))
            .unwrap_or_else(|| Workspace::for_kind(kind));
        self.load_file(path);
    }

    /// Go to workspace `ws` though it shows nothing of `what`: a workspace is a layout and not a kind of content, and the toast says why it shows none of it.
    pub fn open_workspace_that_shows_nothing_of(&mut self, ws: Workspace, what: &str, why: &str) {
        self.active_workspace = ws;
        self.show_info_toast(tr!(
            "{ws} does not show {what}. {why}",
            what = what,
            why = why,
            ws = ws.label()
        ));
    }

    /// Cook the selected source again now, as a saved file in it would.
    pub fn recook_asset(&mut self, key: &str) {
        let Some(row) = self.workbench.row(key) else {
            return;
        };
        let manifest = row.path.join(or_source::asset::MANIFEST);
        // ⚠ Dated a second ago, so the cook's quiet time has already passed.
        let then = Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        self.workbench.file_saved(manifest, then);
    }

    /// Take the file out of the project; it stays on disk.
    pub fn remove_asset(&mut self, key: &str) {
        if self.refused_as_read_only() {
            return;
        }
        let Some(row) = self.workbench.row(key) else {
            return;
        };
        let path = row.path.clone();
        if self.remove_from_project(&path) {
            self.workbench.want_scan();
        }
    }

    /// Point the member at `old` at where the file is now, `new`; a file outside the project's folder is refused with a sentence.
    pub fn locate_member(&mut self, old: &Path, new: PathBuf) {
        if self.refused_as_read_only() {
            return;
        }
        if let Some(project) = self.project_path.clone() {
            let container = &self.project_carried.container;
            if let Some(why) = or_project::links::refuse_member(container, &project, &new) {
                self.show_warning_toast(why);
                return;
            }
        }
        let Some(member) = (self.project_carried.members.iter_mut()).find(|m| m.path == old) else {
            return;
        };
        member.kind = DocumentKind::from_path(&new).or(member.kind);
        member.path = new;
        self.workbench.want_scan();
        self.record_step(tr!("Located {old}", old = leaf(old)));
    }

    /// Show the project's own properties in the Project workspace.
    pub fn show_project_properties(&mut self) {
        self.select_asset(None);
        self.active_workspace = Workspace::Project;
        self.workspace_docks
            .reveal(Workspace::Project, ids::DETAILS);
    }
}

/// What adding files to the project did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Added {
    pub added: usize,
    /// One sentence for each file that was not added, and why.
    pub refused: Vec<String>,
}

/// How many files, and how many bytes, `clean` removes for `resource` under `root`: the resource, the build it replaced and a build that died midway.
///
/// ⚠ The places `or_project::clean` removes, and nothing else: the marker and the record are not counted.
/// `a_clean_removes_exactly_what_its_question_counted` holds the two together.
fn extent(root: &Path, resource: &str) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    let mut stack = vec![
        root.join(resource),
        root.join(format!(".{resource}.previous")),
        root.join(format!(".{resource}.new")),
    ];
    while let Some(place) = stack.pop() {
        // ⚠ Not followed: `remove_dir_all` removes a link and not what it leads to.
        let Ok(meta) = std::fs::symlink_metadata(&place) else {
            continue;
        };
        if meta.is_dir() {
            stack.extend(
                (std::fs::read_dir(&place).into_iter().flatten().flatten()).map(|e| e.path()),
            );
        } else {
            files += 1;
            bytes += meta.len();
        }
    }
    (files, bytes)
}

/// `base`, or `base-2`, `base-3` and so on until no id of `taken` is it.
pub(super) fn unique<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    or_project::container::unique_id(base, taken)
}

impl AppState {
    /// The build running, as the job list draws it.
    pub fn project_build_job(&self) -> Option<crate::state::jobs::RunningJob> {
        use crate::state::jobs::{JobKind, RunningJob};
        use crate::state::workbench::STEPS;
        let builds = self.workbench.builds();
        let (current, total) = builds.running()?;
        let step = builds.step().unwrap_or(0).min(STEPS.len() - 1);
        let (at, _) = builds.place_in_the_run().unwrap_or((0, total));
        let step_words = tr!(
            "Step {step} of {steps}: {name}",
            step = step + 1,
            steps = STEPS.len(),
            name = crate::i18n::tr(STEPS[step])
        );
        // ⚠ Only the steps that write and read back count bytes; the others are one whole step each.
        let counted = builds.progress().filter(|_| matches!(step, 2 | 3));
        let within = counted.as_ref().map_or(0.0, |p| p.fraction().min(0.999));
        let step_words = match counted.as_ref().map(|p| p.item.as_str()) {
            Some(item) if !item.is_empty() => format!("{step_words} · {item}"),
            _ => step_words,
        };
        Some(RunningJob {
            kind: JobKind::ProjectBuild,
            // ⚠ The name the person gave the build, so three builds in a row are told apart in the list and on the status bar.
            doing: tr!(
                "Building {build}",
                build = super::build_name(&self.project_carried.container, current)
            ),
            // How many steps are behind it, of every build of the run, and how far the one it is on has got.
            fraction: Some(
                ((at * STEPS.len() + step) as f32 + within) / (total * STEPS.len()) as f32,
            ),
            detail: if total > 1 {
                tr!(
                    "Build {at} of {total} · {steps}",
                    at = at + 1,
                    total = total,
                    steps = step_words
                )
            } else {
                step_words
            },
            stopping: builds.is_stopping(),
        })
    }
}
