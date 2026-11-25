//! What an open project changes about what the application reads: its copies first.
//!
//! ⚠ **Read-only.** A project's `vfs` folder is read ahead of the overlay and
//! the install; nothing here copies a file into it, writes to `mods`, or asks
//! the edit gate anything.

use std::path::PathBuf;

use or_title::FileDevice;
use or_title::FolderDevice;

use super::keys::gta_install_root_from_prefs;
use super::project::shadow_root;
use super::texture_resolution::{stem_missing_textures, wanted_dictionaries};
use super::AppState;

impl AppState {
    /// The open project's `vfs` folder, when there is a project and it has one.
    pub fn project_shadow_root(&self) -> Option<PathBuf> {
        let root = shadow_root(self.project_path.as_deref()?);
        root.is_dir().then_some(root)
    }

    /// Re-read what the install supplies, through whichever project is now open.
    ///
    /// ⚠ **The re-read is what makes it live.** The clip-region table is
    /// built once from the install, so a project whose copy is only consulted
    /// on the next launch is one a user concludes does nothing.
    pub(super) fn reread_through_project(&mut self) {
        self.attempted_texture_project.clear();
        if !self.keys.has_keys() {
            return;
        }
        let Some(install_root) = gta_install_root_from_prefs(&self.prefs) else {
            return;
        };
        let shadow = self.project_shadow_root();
        self.clip_regions.start_load(
            install_root,
            self.keys.provider(),
            self.prefs.mount_options(),
            shadow,
        );
    }

    /// Files in the project's `vfs` folder that could serve a drawable called `stem`, best first.
    ///
    /// ⚠ By name, anywhere in the folder: the shadow mirrors the VFS, and a
    /// drawable does not say which archive its dictionary lives in.
    pub fn project_texture_candidates(&self, stem: &str) -> Vec<PathBuf> {
        let Some(root) = self.project_shadow_root() else {
            return Vec::new();
        };
        let mut files = FolderDevice::new(&root).walk().unwrap_or_default();
        files.sort_by_key(|f| f.to_ascii_lowercase());
        let mut out = Vec::new();
        for want in wanted_dictionaries(stem) {
            for file in &files {
                let leaf = file.rsplit('/').next().unwrap_or(file);
                if leaf.eq_ignore_ascii_case(&want) {
                    out.push(root.join(file));
                }
            }
        }
        out
    }

    /// Open the project's copy of a dictionary the active drawable is missing textures from.
    pub fn try_open_texture_from_project(&mut self) -> bool {
        if self.project_shadow_root().is_none() {
            return false;
        }
        let Some((doc_id, stem)) = stem_missing_textures(self) else {
            return false;
        };
        if !self.attempted_texture_project.insert(doc_id) {
            return false;
        }
        for path in self.project_texture_candidates(&stem) {
            if self.open_texture_dictionary_file(&path) {
                return true;
            }
        }
        false
    }

    /// Open the dictionary the active model is drawn with, best source first; whether one opened.
    ///
    /// ⚠ **The project outranks the archive sibling.** The sibling is the
    /// game's copy of the same dictionary, and a project exists to be read in
    /// preference to it.
    pub fn try_open_missing_textures(&mut self) -> bool {
        if self
            .active_document()
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return self.try_open_table_texture_dictionary() || self.try_open_texture_pack();
        }

        self.ensure_garment_dressed()
            || self.try_open_texture_from_project()
            || self.try_auto_open_texture_sibling()
            || self.try_auto_open_shared_dictionary()
            || self.try_open_texture_from_search_paths()
            || self.try_open_texture_pack()
            || self.try_open_texture_dictionary()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use super::super::document::{DocPayload, Document, DocumentId};
    use super::super::project::{save_project, ProjectDocument, ProjectFile, PROJECT_SCHEMA};
    use super::super::texture_resolution::report;
    use super::*;

    fn write(path: &Path, body: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn open(state: &mut AppState, path: &Path) -> DocumentId {
        let id = state.documents.len() as DocumentId + 1;
        state.documents.push(Document::new(
            id,
            Some(path.to_path_buf()),
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
        ));
        id
    }

    #[test]
    fn the_shadow_root_follows_the_open_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState::new();
        assert_eq!(state.project_shadow_root(), None);

        state.project_path = Some(dir.path().join("p.openrage-proj"));
        assert_eq!(
            state.project_shadow_root(),
            None,
            "a project with no `vfs` folder shadows nothing"
        );

        std::fs::create_dir_all(dir.path().join("vfs")).unwrap();
        assert_eq!(state.project_shadow_root(), Some(dir.path().join("vfs")));

        state.close_project();
        assert_eq!(state.project_shadow_root(), None);
    }

    #[test]
    fn the_project_copy_of_a_dictionary_is_found_by_name_anywhere_in_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let vfs = dir.path().join("vfs");
        write(&vfs.join("update/x64/levels/foo+hi.ytd"), b"x");
        write(&vfs.join("platform/levels/props.rpf/FOO.ytd"), b"x");
        write(&vfs.join("common/foo_other.ytd"), b"x");
        let mut state = AppState::new();
        state.project_path = Some(dir.path().join("p.openrage-proj"));

        let found: Vec<String> = state
            .project_texture_candidates("foo")
            .iter()
            .map(|p| {
                p.strip_prefix(&vfs)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(
            found,
            vec![
                "platform/levels/props.rpf/FOO.ytd",
                "update/x64/levels/foo+hi.ytd"
            ],
            "the exact name first, its companion after, and nothing that only contains it"
        );
    }

    #[test]
    fn a_drawable_missing_textures_asks_the_project_once_per_project() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("vfs/platform/foo.ytd"),
            b"not a dictionary",
        );
        let mut state = AppState::new();
        let drawable = open(&mut state, Path::new("C:/a/foo.ydd"));
        state.active_document = Some(drawable);
        state.texture_report = report(&state, None, [(None, "diffuse".to_string())], 1);

        assert!(!state.try_open_texture_from_project());
        assert!(
            state.attempted_texture_project.is_empty(),
            "with no project open nothing was asked, so opening one later can still ask"
        );

        state.project_path = Some(dir.path().join("p.openrage-proj"));
        assert!(
            !state.try_open_texture_from_project(),
            "the copy does not parse"
        );
        assert!(state.attempted_texture_project.contains(&drawable));

        state.reread_through_project();
        assert!(
            state.attempted_texture_project.is_empty(),
            "a different project is a different answer, so it is asked again"
        );
    }

    #[test]
    fn opening_a_project_that_names_a_missing_file_still_opens_it_and_raises_no_toast() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let gone = dir.path().join("models").join("gone.ydr");
        let project = ProjectFile {
            documents: vec![ProjectDocument::new(gone.clone(), None)],
            ..ProjectFile::default()
        };
        save_project(&file, &project).unwrap();
        let mut state = AppState::new();

        state.load_project_from(file.clone());

        assert_eq!(state.project_path.as_deref(), Some(file.as_path()));
        // ⚠ Problems says it, with Locate… and Remove (`the_project_polish`), so a toast would say it twice.
        let named = state
            .toasts
            .iter()
            .filter(|t| t.message.contains(&gone.display().to_string()))
            .count();
        assert_eq!(named, 0, "no toast names the missing file");
    }

    /// ⚠ The app rebuilds a project from its open tabs on every save, so this
    /// is the half of "unknown keys survive" a file-level test cannot reach.
    #[test]
    fn keys_a_newer_build_wrote_survive_being_saved_by_this_one() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("proj");
        let file = folder.join("p.openrage-proj");
        let text = json!({
            "schema": 4,
            "from_the_future": [1, 2],
            "documents": [{ "path": "models/a.ydr", "camera": { "yaw": 0.5 } }],
        });
        write(&file, text.to_string().as_bytes());
        let mut state = AppState::new();
        state.load_project_from(file.clone());
        open(&mut state, &folder.join("models").join("a.ydr"));

        let moved = dir.path().join("elsewhere").join("q.openrage-proj");
        state.save_project_to(moved.clone()).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&moved).unwrap()).unwrap();
        assert_eq!(written["from_the_future"], json!([1, 2]));
        assert_eq!(
            written["documents"],
            json!([]),
            "saved into another folder, the file is outside it, and a shared file names no path of this machine"
        );
        // ⚠ Kept beside the project with the keys it carried, so the person who saved it loses nothing.
        let apart: Value = serde_json::from_slice(
            &std::fs::read(or_project::snapshot::private_dir(&moved).join("machine.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(apart["documents"][0]["camera"], json!({ "yaw": 0.5 }));
        assert_eq!(
            apart["documents"][0]["path"],
            json!(folder.join("models").join("a.ydr").to_string_lossy())
        );
    }

    #[test]
    fn closing_a_tab_keeps_its_file_in_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let mut state = AppState::new();
        open(&mut state, &dir.path().join("a.ydr"));
        let b = open(&mut state, &dir.path().join("b.ydr"));
        state.save_project_to(file.clone()).unwrap();

        let c = open(&mut state, &dir.path().join("c.ydr"));
        state.close_document(b);
        state.close_document(c);
        state.save_project_to(file.clone()).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let paths: Vec<&Value> = written["documents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| &d["path"])
            .collect();
        assert_eq!(
            paths,
            [&json!("a.ydr"), &json!("b.ydr"), &json!("c.ydr")],
            "membership is not the tab strip, even for a file opened since the last save"
        );
        assert_eq!(written["open"], json!([0]));
    }

    #[test]
    fn a_removed_member_stays_out_of_the_project_and_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let (a, b, c) = (
            dir.path().join("a.ydr"),
            dir.path().join("b.ydr"),
            dir.path().join("c.ydr"),
        );
        for path in [&a, &b, &c] {
            write(path, b"drawable");
        }
        let mut state = AppState::new();
        open(&mut state, &a);
        open(&mut state, &b);
        let closed = open(&mut state, &c);
        state.save_project_to(file.clone()).unwrap();
        state.close_document(closed);

        assert!(state.remove_from_project(&b), "an open member");
        assert!(
            state.remove_from_project(&c),
            "a member whose tab is closed"
        );
        assert!(state.project_carried.changed);
        assert!(
            state.window_title().contains('●'),
            "{}",
            state.window_title()
        );
        assert!(
            state
                .documents
                .iter()
                .all(|d| d.path.as_deref() != Some(&b)),
            "an open tab would rejoin at the next save"
        );
        state.save_project_to(file.clone()).unwrap();
        assert!(!state.project_carried.changed);

        let paths: Vec<PathBuf> = super::super::project::load_project(&file)
            .unwrap()
            .project
            .documents
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert_eq!(paths, [a]);
        assert!(b.is_file() && c.is_file(), "removing keeps the file");
    }

    #[test]
    fn a_member_with_unsaved_edits_is_not_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState::new();
        let a = dir.path().join("a.ydr");
        let id = open(&mut state, &a);
        state
            .save_project_to(dir.path().join("p.openrage-proj"))
            .unwrap();
        state.document_mut(id).unwrap().mark_edited();

        assert!(!state.remove_from_project(&a));
        assert!(state.document(id).is_some());
        assert!(!state.project_carried.changed);
    }

    fn open_entry(state: &mut AppState, path: &Path) -> DocumentId {
        let id = open(state, path);
        state.document_mut(id).unwrap().rpf_entry_binding = Some(super::super::RpfEntryBinding {
            parent_doc_id: 99,
            entry_id: 1,
        });
        id
    }

    #[test]
    fn an_archive_entry_is_written_only_while_its_tab_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let mut state = AppState::new();
        let entry = open_entry(&mut state, &dir.path().join("x.rpf/models/a.ydr"));
        state.save_project_to(file.clone()).unwrap();
        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(written["documents"][0]["path"], json!("x.rpf/models/a.ydr"));
        assert_eq!(written["open"], json!([0]));

        state.close_document(entry);
        state.save_project_to(file.clone()).unwrap();
        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(written["documents"], json!([]), "{written}");
    }

    #[test]
    fn closing_an_archive_entry_tab_does_not_join_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let mut state = AppState::new();
        state.save_project_to(file.clone()).unwrap();

        let entry = open_entry(&mut state, &dir.path().join("x.rpf/models/a.ydr"));
        state.close_document(entry);
        state.save_project_to(file.clone()).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(written["documents"], json!([]), "{written}");

        let mut reloaded = AppState::new();
        reloaded.load_project_from(file);
        let missing = reloaded
            .toasts
            .iter()
            .filter(|t| t.message.contains("missing file"))
            .count();
        assert_eq!(missing, 0, "an entry's display path is no file to reopen");
    }

    #[test]
    fn a_dictionary_detaching_closes_does_not_join_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let mut state = AppState::new();
        open(&mut state, &dir.path().join("a.ydr"));
        state.save_project_to(file.clone()).unwrap();

        let dict = open(&mut state, &dir.path().join("d.ytd"));
        state
            .attached_texture_docs
            .push(super::super::texture_resolution::AttachedDictionary {
                document: dict,
                opened_by_attach: true,
            });
        state.detach_texture_dictionary(dict);
        assert!(state.document(dict).is_none());
        state.save_project_to(file.clone()).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            written["documents"].as_array().unwrap().len(),
            1,
            "{written}"
        );
    }

    #[test]
    fn saving_with_no_project_open_creates_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.openrage-proj");
        let mut state = AppState::new();
        open(&mut state, &dir.path().join("a.ydr"));

        assert!(
            !state.save_active_project(|_| None),
            "declining the dialog saves nothing"
        );
        assert_eq!(state.project_path, None);

        assert!(state.save_active_project(|_| Some(file.clone())));
        assert_eq!(state.project_path.as_deref(), Some(file.as_path()));
        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(written["documents"][0]["path"], json!("a.ydr"));

        assert!(
            state.save_active_project(|_| panic!("an open project is saved where it is")),
            "the second save asks for no path"
        );
    }

    #[test]
    fn a_project_saved_by_the_app_writes_its_own_files_relative() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("p.openrage-proj");
        let mut state = AppState::new();
        open(&mut state, &dir.path().join("models").join("a.ydr"));

        state.save_project_to(file.clone()).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(written["documents"][0]["path"], json!("models/a.ydr"));
        assert_eq!(written["schema"], json!(PROJECT_SCHEMA));
    }
}
