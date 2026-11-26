//! The front door: what a launch opens, the start page, a new project, a folder made a project, and a project in a second window.
//!
//! ⚠ **Nothing here draws, and nothing here reads the disk on the UI thread.** The cards of the start page are
//! read on a worker, and so is a Create, which analyses an adopted folder; the panels read what they left.

mod adopt;
pub mod cards;
pub mod new_project;
mod sample;
mod template;
mod words;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

pub use adopt::{recognisers, Offer, Preview, Previews};
pub use cards::{Card, Cards};
pub use new_project::{workspace_for, Created, GameChoice, Loaded, NewProject, Step, Template};
pub use sample::{Corner, TOUR};
pub use words::{counted, spoken, KindWords};

use super::jobs::{JobKind, RunningJob};
use super::{save_prefs, AppState};
use crate::ui::dock::Workspace;

/// What the front door is doing: the start page raised over the shell, a dialog or a question open.
#[derive(Default)]
pub struct FrontDoor {
    /// The workspace on screen when the start page was raised; it stays up while that one is.
    start_page: Option<Workspace>,
    /// The New project dialog, while it is open.
    pub new_project: Option<NewProject>,
    /// A project to open while another is open, waiting on whether it opens here or in a new window.
    pub switching: Option<PathBuf>,
    /// A project being made on a worker; the dialog stays open, and Create waits, until it answers.
    creating: Option<Receiver<Result<Created, String>>>,
    pub cards: Cards,
    /// What the folder the dialog or the banner is about holds, read off the UI thread.
    pub previews: Previews,
    /// Folders whose offer to become a project was declined in this session.
    declined: HashSet<PathBuf>,
    /// The project file each opened folder holds and when that was read, so the banner reads no disk in a frame.
    held: RefCell<HashMap<PathBuf, (Instant, Option<PathBuf>)>>,
    /// The Workbench guide, while its window is open.
    pub guide: Option<Guide>,
    /// The step of the tour a first launch can take, while it is under way.
    pub tour: Option<usize>,
}

/// A document the binary carries that the guide's window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuideDocument {
    /// `docs/guides/Workbench_Guide.md`.
    #[default]
    Workbench,
    /// `docs/guides/In_Game_Acceptance.md`.
    Acceptance,
}

/// The guide's window: the document it shows, the heading it was opened at, and whether it has scrolled there yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guide {
    pub document: GuideDocument,
    /// A heading of the guide, or empty to open it from the top.
    pub section: String,
    pub scrolled: bool,
}

impl FrontDoor {
    /// Whether the start page is raised over the workspace on screen.
    pub fn raised(&self, on_screen: Workspace) -> bool {
        self.start_page == Some(on_screen)
    }

    /// Open the guide at the heading `section`; an empty one opens it from the top.
    pub fn show_guide(&mut self, section: &str) {
        self.show_document(GuideDocument::Workbench, section);
    }

    /// Open `document` in the guide's window at its heading `section`; an empty one opens it from the top.
    pub fn show_document(&mut self, document: GuideDocument, section: &str) {
        self.guide = Some(Guide {
            document,
            section: section.to_string(),
            scrolled: false,
        });
    }
}

/// How the question about a second project was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Close this window's project and open the other here.
    Here,
    NewWindow,
    Cancel,
}

/// Whether `path` is spelled as a project file.
pub fn is_project_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(or_project::PROJECT_EXTENSION))
}

impl AppState {
    /// Put the start page over the workspace on screen, until a document, a project or another workspace is shown.
    pub fn show_start_page(&mut self) {
        self.front_door.start_page = Some(self.active_workspace);
        self.front_door.cards.refresh();
    }

    pub fn hide_start_page(&mut self) {
        self.front_door.start_page = None;
    }

    /// The project that was open when OpenRage last ran, while the setting says to reopen it.
    pub fn last_project_to_reopen(&self) -> Option<PathBuf> {
        self.prefs
            .last_project
            .clone()
            .filter(|_| self.prefs.reopen_last_project)
    }

    /// Open the project at `path`, which a launch or a command line named; a missing one shows the start page.
    pub fn open_named_project(&mut self, path: PathBuf) {
        if path.is_file() {
            self.load_project_from(path);
        } else {
            self.project_not_found(&path);
        }
    }

    /// Open the project a launch reopens, on the workspace that was on screen when OpenRage last ran.
    ///
    /// ⚠ The workspace is the machine's (`prefs.json` keeps it) and not the project file's: the file
    /// is shared, and a project in Git must not change every time someone switches workspace. The
    /// file's own workspace stands when no layout was saved.
    pub fn reopen_project(&mut self, path: PathBuf) {
        let reopens = path.is_file();
        self.open_named_project(path);
        if let Some(left) = crate::ui::dock_layout::left_on(&self.prefs.dock_layouts) {
            if reopens && self.project_path.is_some() {
                self.active_workspace = left;
            }
        }
    }

    /// Show the start page, where a project that is not there is listed with Locate… and Remove.
    pub fn project_not_found(&mut self, path: &Path) {
        self.show_start_page();
        if !self.prefs.recent_projects.iter().any(|p| p == path) {
            // ⚠ A button, and for longer than a warning lasts: a path nobody can type and a toast that is gone leave nothing to do.
            self.toasts.push(
                crate::state::toasts::Toast::warning(tr!(
                    "That project is no longer at {path}. It was moved or deleted.",
                    path = crate::ui::panels::status_bar::elide_middle(
                        &path.display().to_string(),
                        60
                    )
                ))
                .with_action(crate::state::toasts::ToastAction::OpenProject),
            );
        }
    }

    /// Open the project at `path`; while another is open, ask whether it replaces that one or opens in a new window.
    pub fn open_project_asking(&mut self, path: PathBuf) {
        if !path.is_file() {
            self.project_not_found(&path);
        } else if self.project_path.as_ref().is_some_and(|open| *open != path) {
            self.front_door.switching = Some(path);
        } else {
            self.load_project_from(path);
        }
    }

    pub fn answer_switch(&mut self, answer: Switch) {
        let Some(path) = self.front_door.switching.take() else {
            return;
        };
        match answer {
            Switch::Here => self.load_project_from(path),
            Switch::NewWindow => self.open_project_in_new_window(&path),
            Switch::Cancel => {}
        }
    }

    /// Start another OpenRage on the project at `path`.
    pub fn open_project_in_new_window(&mut self, path: &Path) {
        match crate::platform::new_window::open(path) {
            Ok(()) => {
                let name = path.file_stem().map_or_else(
                    || path.display().to_string(),
                    |stem| stem.to_string_lossy().into_owned(),
                );
                self.show_info_toast(tr!("Opening {name} in a new window", name = name));
            }
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Drop `path` from the recent projects; its files are not touched.
    pub fn forget_recent_project(&mut self, path: &Path) {
        self.prefs.forget_recent_project(path);
        save_prefs(&self.prefs);
        self.front_door.cards.refresh();
    }

    /// The project that was at `old` is at `new`: it leaves the recent list under the old place and opens.
    pub fn locate_recent_project(&mut self, old: &Path, new: PathBuf) {
        self.forget_recent_project(old);
        self.open_project_asking(new);
    }

    /// Open the New project dialog on `template`, in the folder the last project was made in, with the templates that were saved.
    pub fn start_new_project(&mut self, template: Template) {
        self.start_new_project_on(template, Step::Gallery);
    }

    /// As [`Self::start_new_project`] on `step`: a goal that already chose its template starts at the details, and Back reaches the gallery.
    pub fn start_new_project_on(&mut self, template: Template, step: Step) {
        let place = new_project::default_place(&self.prefs);
        let mut draft = NewProject::new(template, place);
        draft.step = step;
        draft.name_after_the_template();
        draft.loaded = new_project::saved_templates();
        // A folder may have changed since it was last read.
        self.front_door.previews.forget();
        self.front_door.new_project = Some(draft);
    }

    /// Save the open project's content, builds and layout as a template at `file`; what it left out, one sentence each.
    pub fn save_project_as_template(&mut self, file: &Path) -> Result<Vec<String>, String> {
        let Some(project) = self.project_path.clone() else {
            return Err(tr!("No project is open").into());
        };
        let name = project
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        let top = &self.project_carried.top;
        let word = |key: &str| top.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let shown: Option<Vec<String>> = self.project_carried.shown.as_ref().map(|buttons| {
            buttons
                .iter()
                .filter_map(|ws| serde_json::to_value(ws).ok())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        });
        // ⚠ Every template opens on the Project workspace, whichever one the person left the project on.
        let (template, left_out) = or_project::template::Template::from_project(
            &name,
            "",
            &self.project_carried.container,
            Some(tr!("Project")),
            shown.as_deref(),
            word(new_project::GAME_KEY).as_deref(),
        );
        if file.exists() {
            return Err(tr!(
                "{file} is there already, and a template is never written over a file",
                file = file.display()
            ));
        }
        or_project::replace_file(file, template.to_json().as_bytes())
            .map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(left_out)
    }

    /// Make the project `draft` describes on a worker and open it once it is made; one at a time.
    pub fn start_creating(&mut self, draft: NewProject) {
        if self.front_door.creating.is_some() {
            return;
        }
        let (sender, receiver) = channel();
        std::thread::spawn(move || {
            let _ = sender.send(draft.create());
        });
        self.front_door.creating = Some(receiver);
    }

    pub fn is_creating(&self) -> bool {
        self.front_door.creating.is_some()
    }

    /// Take what the worker made: the dialog closes and the project opens, or the dialog says why it could not be made.
    pub fn poll_creating(&mut self) {
        let Some(receiver) = &self.front_door.creating else {
            return;
        };
        let made = match receiver.try_recv() {
            Ok(made) => made,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(tr!("The worker making it stopped").to_string()),
        };
        self.front_door.creating = None;
        match made {
            Ok(created) => {
                self.front_door.new_project = None;
                self.say_what_was_made(&created);
                self.open_project_asking(created.file);
            }
            Err(why) => match self.front_door.new_project.as_mut() {
                Some(draft) => draft.error = Some(why),
                None => self.show_error_toast(why),
            },
        }
    }

    /// The open project's files being read: its content is listed once the scan has left them.
    pub(super) fn project_read_job(&self) -> Option<RunningJob> {
        let name = self.project_name()?;
        (!self.workbench.is_scanned()).then(|| RunningJob {
            kind: JobKind::ProjectRead,
            doing: tr!("Reading {name}", name = name),
            fraction: None,
            detail: String::new(),
            stopping: false,
        })
    }
}
