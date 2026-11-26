//! Check before sharing: what a project folder holds that is not the person's to hand to another machine or a repository.

use std::path::PathBuf;
use std::time::Instant;

use or_project::project_folder;
use or_source::git::{self, Commit};

pub use or_content::sharing::{Concern, Kind};

use super::scan::{AssetRow, Kept, Seen};
use super::Question;
use crate::state::AppState;

impl AppState {
    /// What the project's managed Git block says its repository holds: the sources by default.
    pub fn commit_choice(&mut self) -> Commit {
        if let (None, Some(project)) = (self.workbench.commit, self.project_path.as_ref()) {
            self.workbench.commit = Some(git::commit_of(&project_folder(project)));
        }
        self.workbench.commit.unwrap_or_default()
    }

    /// Write what the owner chose to commit into the managed Git block, which keeps every line of the person's own.
    ///
    /// ⚠ The block is the only place the choice lives, so an import or a new project that rewrites it keeps the choice.
    pub fn choose_what_to_commit(&mut self, commit: Commit) -> Result<(), String> {
        if let Some(why) = self.read_only_reason() {
            return Err(why);
        }
        let project = self.project_path.clone().ok_or(tr!("No project is open"))?;
        let folder = project_folder(&project);
        git::choose(&folder, commit).map_err(|e| format!("{}: {e}", folder.display()))?;
        self.workbench.commit = Some(commit);
        self.workbench.note(match (commit.sources, commit.build) {
            (true, false) => tr!("The repository holds the sources").to_string(),
            (false, true) => {
                tr!("The repository holds the build instead of the sources").to_string()
            }
            (true, true) => tr!("The repository holds the sources and the build").to_string(),
            (false, false) => {
                tr!("The repository holds neither the sources nor the build").to_string()
            }
        });
        Ok(())
    }

    /// Ask for the folders to be read again, and show what sharing this project's folder would hand over once they have been.
    ///
    /// ⚠ The last scan is not trusted: nothing watches `sources/`, so a source taken from the game may have come
    /// since, and the window opens only on a scan that began after the command.
    pub fn check_before_sharing(&mut self) {
        if self.project_path.is_none() {
            return;
        }
        self.workbench.want_scan();
        self.workbench.asking = Some(Question::SharingRead {
            asked: Instant::now(),
        });
    }

    /// Show the check's findings once a scan that began after it was asked for has landed.
    pub(super) fn finish_sharing_check(&mut self) {
        let Some(Question::SharingRead { asked }) = self.workbench.asking else {
            return;
        };
        let read = self.workbench.scanning.is_none()
            && self.workbench.last_scan.is_some_and(|at| at > asked);
        if read {
            let concerns = self.sharing_concerns();
            self.workbench.asking = Some(Question::Sharing { concerns });
        }
    }

    /// What sharing this project's folder would hand over.
    ///
    /// ⚠ The saved file is what gets sent, so it is the file that is read; the sources are the last scan's.
    fn sharing_concerns(&self) -> Vec<Concern> {
        let Some(project) = self.project_path.as_deref() else {
            return Vec::new();
        };
        let games: Vec<PathBuf> = (self.workbench.rows().iter())
            .filter(|row| is_the_games(row))
            .map(|row| row.path.clone())
            .collect();
        let packs: Vec<PathBuf> = (self.workbench.packs().iter())
            .filter(|p| !p.linked)
            .map(|p| p.folder.clone())
            .collect();
        let mut found =
            or_content::sharing::concerns(project, &games, &packs, self.workbench.is_truncated());
        if self.project_unsaved() {
            found.push(Concern {
                kind: Kind::Unsaved,
                text:
                    tr!("The project has changes Save project has not written; this read the saved file")
                        .to_string(),
            });
        }
        found
    }
}

/// Whether the source of `row` is the game's own file; a source the scan could not read says so in its manifest all the same.
fn is_the_games(row: &AssetRow) -> bool {
    if row.from_game {
        return true;
    }
    row.kept == Kept::Source
        && matches!(row.seen, Seen::Unreadable(_))
        && or_content::sharing::is_the_games(&row.path)
}
