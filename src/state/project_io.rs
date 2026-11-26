//! Opening, saving and closing a `.openrage-proj`.

use std::path::{Path, PathBuf};

use super::project::{load_project, save_project, Carried, ProjectFile, PROJECT_SCHEMA};
use super::{save_prefs, AfterClose, AppState, DocumentKind};

impl AppState {
    pub fn snapshot_project(&self) -> ProjectFile {
        let mut carried = self.project_carried.clone();
        let mut open = Vec::new();
        let mut active = None;
        for doc in &self.documents {
            let Some(path) = &doc.path else { continue };
            // The install is the app's own mount, kept across projects, and an opened folder is a place to look in.
            if super::workbench::is_a_place(doc.kind) {
                continue;
            }
            let index = carried.member(path, doc.kind);
            if !open.contains(&index) {
                open.push(index);
            }
            if self.active_document == Some(doc.id) {
                active = Some(index);
            }
        }
        ProjectFile {
            workspace: carried.workspace,
            active,
            open,
            documents: carried.members,
            shown: carried.shown,
            container: carried.container,
            extra: carried.top,
        }
    }

    /// Whether the open project holds something Save project is yet to write: a change to its members, or an edit of its maps.
    pub fn project_unsaved(&self) -> bool {
        self.project_carried.changed || self.map_edits_unwritten()
    }

    pub fn save_project_to(&mut self, path: PathBuf) -> std::io::Result<()> {
        if let (Some(why), true) = (
            self.read_only_reason(),
            self.project_path.as_ref() == Some(&path),
        ) {
            return Err(std::io::Error::other(why));
        }
        // The edits of the Map screen are the project's, held in memory until here: a pack that cannot be written stops the save.
        self.flush_map_sessions().map_err(std::io::Error::other)?;
        // ⚠ Read before the save adds the open tabs to the project, or the file would read as changed outside.
        self.begin_history();
        // ⚠ A project made by saving a session states the workspace on screen; one that has a file states what the file did, whatever the person has switched to since.
        if self.project_path.is_none() {
            self.project_carried.workspace = self.active_workspace;
        }
        let project = self.snapshot_project();
        save_project(&path, &project)?;
        let moved = self.project_path.as_deref() != Some(path.as_path());
        self.project_path = Some(path.clone());
        self.project_carried = Carried::from_project(&project);
        // ⚠ An open archive entry is written but is no member, so its tab closing drops it.
        self.project_carried.members.retain(|m| {
            !self
                .documents
                .iter()
                .any(|d| d.rpf_entry_binding.is_some() && d.path.as_ref() == Some(&m.path))
        });
        self.prefs.touch_recent_project(path);
        save_prefs(&self.prefs);
        if moved {
            // ⚠ A project in another folder has a history of its own, which begins with the file just written.
            self.workbench.reset(self.project_path.clone());
            // ⚠ The reset let go of the lock of the project this window had: the new one is taken before its history, as an open does.
            self.hold_the_open_project();
            self.begin_history();
            self.reread_through_project();
        } else {
            self.project_saved();
        }
        Ok(())
    }

    /// Save the open project, or create one where `choose` says when none is open.
    pub fn save_active_project(&mut self, choose: impl FnOnce(&Self) -> Option<PathBuf>) -> bool {
        let Some(path) = self.project_path.clone().or_else(|| choose(self)) else {
            return false;
        };
        match self.save_project_to(path.clone()) {
            Ok(()) => {
                self.show_toast(tr!("Project saved"));
                true
            }
            Err(e) => {
                let msg = tr!("Failed to save project: {e}", e = e);
                tracing::error!(error = %msg);
                self.show_error_toast(msg);
                false
            }
        }
    }

    /// Close every document but the install and leave the project, once nothing unsaved is lost.
    pub fn new_project(&mut self) {
        self.request_close(
            self.every_document_but_the_install(),
            AfterClose::NewSession,
        );
    }

    pub(super) fn start_new_session(&mut self) {
        self.close_all_but_the_install();
        self.leave_project();
    }

    fn close_all_but_the_install(&mut self) {
        for id in self.every_document_but_the_install() {
            self.drop_document(id);
        }
        self.texture_cache.clear();
    }

    /// Leave the project and show the start page, with the install still mounted; a project with a change nothing wrote down asks first.
    pub fn close_project(&mut self) {
        self.request_close(Vec::new(), AfterClose::CloseProject);
    }

    pub(super) fn leave_project_for_the_start_page(&mut self) {
        self.leave_project();
        self.show_start_page();
    }

    fn leave_project(&mut self) {
        let had_shadow = self.project_shadow_root().is_some();
        self.project_path = None;
        self.project_carried = Carried::default();
        self.workbench.reset(None);
        // ⚠ A project that was closed is not the one a launch reopens.
        if self.prefs.last_project.take().is_some() {
            save_prefs(&self.prefs);
        }
        // ⚠ The Project workspace is offered only while a project is open.
        if self.active_workspace.needs_a_project() {
            self.active_workspace = crate::ui::dock::Workspace::default();
        }
        if had_shadow {
            self.reread_through_project();
        }
    }

    /// Drop the member at `path`, closing its tab, and leave the file where it is.
    ///
    /// ⚠ **An open tab is a member at every save**, so its tab closes too, and
    /// a tab with unsaved edits refuses rather than losing them.
    pub fn remove_from_project(&mut self, path: &Path) -> bool {
        if self.project_path.is_none() {
            return false;
        }
        let open: Vec<_> = self
            .documents
            .iter()
            .filter(|d| d.path.as_deref() == Some(path))
            .map(|d| (d.id, d.dirty))
            .collect();
        if open.iter().any(|&(_, dirty)| dirty) {
            self.show_warning_toast(tr!(
                "Save {path} before removing it from the project",
                path = path.display()
            ));
            return false;
        }
        let held = self.project_carried.members.iter().any(|m| m.path == path);
        self.project_carried.members.retain(|m| m.path != path);
        if !held && open.is_empty() {
            return false;
        }
        for (id, _) in open {
            self.drop_document(id);
        }
        // ⚠ A step, which Undo takes back whenever it is asked: no toast has a button that runs out.
        let said = tr!(
            "Removed {name} from the project",
            name = super::workbench::leaf(path)
        );
        if held {
            self.record_step(said.clone());
            self.show_toast(tr!("{said}; Undo brings it back", said = said));
        } else {
            self.project_carried.changed = true;
            self.workbench.note(said);
        }
        true
    }

    /// "document ● — project — OpenRage", leaving out what is not open; ● marks unsaved work.
    pub fn window_title(&self) -> String {
        let marked = |name: String, unsaved: bool| {
            if unsaved {
                format!("{name} ●")
            } else {
                name
            }
        };
        let mut parts = Vec::new();
        if let Some(doc) = self.active_document() {
            parts.push(marked(doc.file_name(), self.loses_work(doc)));
        }
        if let Some(p) = &self.project_path {
            let name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(tr!("Untitled"));
            parts.push(marked(name.to_string(), self.project_unsaved()));
        }
        parts.push("OpenRage".to_string());
        parts.join(" — ")
    }

    /// The window title when it differs from the one last handed to the window.
    pub fn changed_window_title(&mut self) -> Option<String> {
        let title = self.window_title();
        if title == self.last_window_title {
            return None;
        }
        self.last_window_title.clone_from(&title);
        Some(title)
    }

    /// Open the project at `path` in place of every open document but the install, once nothing unsaved is lost.
    pub fn load_project_from(&mut self, path: PathBuf) {
        self.request_close(
            self.every_document_but_the_install(),
            AfterClose::OpenProject(path),
        );
    }

    pub(super) fn open_project_now(&mut self, path: PathBuf) {
        let loaded = match load_project(&path) {
            Ok(p) => p,
            Err(e) => {
                let msg = tr!("Failed to read project: {e}", e = e);
                tracing::error!(path = %path.display(), error = %msg);
                self.set_error(tr!("Could not open the project"), msg);
                return;
            }
        };
        for finding in &loaded.findings {
            tracing::warn!(path = %path.display(), %finding, "Project field not read");
        }
        if let Some(first) = loaded.findings.first() {
            self.show_warning_toast(tr!(
                "{count} field(s) of the project could not be read and were skipped: {first}",
                first = first,
                count = loaded.findings.len()
            ));
        }
        if loaded.schema < PROJECT_SCHEMA {
            tracing::info!(
                path = %path.display(),
                from = loaded.schema,
                to = PROJECT_SCHEMA,
                "Project migrates on its next save"
            );
        }
        let project = loaded.project;

        self.close_all_but_the_install();
        // ⚠ The workspace the project was left on beats the one its file states, which is the person's to set.
        let wanted = super::workbench::remembered_workspace(&path).unwrap_or(project.workspace);
        self.active_workspace = super::front_door::workspace_for(&project.container, wanted);
        self.hide_start_page();

        // ⚠ No toast for a file the project names that is gone: Problems lists each with Locate… and Remove.
        for &index in &project.open {
            // ⚠ A project an older build saved lists the install, a folder no load reads.
            if let Some(doc) = project
                .documents
                .get(index)
                .filter(|d| d.path.exists() && d.kind != Some(DocumentKind::GameRoot))
            {
                self.load_file(doc.path.clone());
            }
        }
        // ⚠ Loads run in order and each leaves the keyboard on its tab, so the tab the project was saved on is asked for last: a file already open is only brought forward, and no tab moves.
        let saved_on = (project.active)
            .filter(|index| project.open.contains(index) && project.open.last() != Some(index))
            .and_then(|index| project.documents.get(index))
            .filter(|d| d.path.exists() && d.kind != Some(DocumentKind::GameRoot));
        if let Some(doc) = saved_on {
            self.load_file(doc.path.clone());
        }

        self.project_path = Some(path.clone());
        self.project_carried = Carried::from_project(&project);
        self.workbench.reset(Some(path.clone()));
        self.workbench.opened_on(self.active_workspace);
        self.hold_the_open_project();
        // ⚠ Before anything changes the project, or the first step would hold the project as the change left it.
        self.begin_history();
        self.workbench.note_findings(loaded.findings);
        self.prefs.touch_recent_project(path);
        save_prefs(&self.prefs);
        self.reread_through_project();
    }
}

#[cfg(test)]
mod tests {
    use or_project::{Container, Output, Project};

    use super::*;
    use crate::state::Prefs;

    /// ⚠ Not on the install's account: `from_prefs` reads no `prefs.json`, which a parallel test may be writing.
    #[test]
    fn the_builds_a_project_declares_survive_the_editor_opening_and_saving_it() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let file = dir.path().join("p.openrage-proj");
        let mut container = Container::default();
        container
            .outputs
            .push(Output::fivem_resource("fivem", "pack", &["files"]));
        Project::create(&file, &container, &[]).expect("a project another program made");
        let mut state = AppState::from_prefs(Prefs::default());

        state.load_project_from(file.clone());
        state
            .save_project_to(file.clone())
            .expect("the project saved");

        let saved = load_project(&file).expect("it reads").project;
        assert_eq!(saved.container, container);
    }
}
