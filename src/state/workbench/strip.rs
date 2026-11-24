//! The strip of workspace buttons: which workspaces a window offers, which of them it shows, and how a narrow window fits them.

use crate::state::{AppState, DocumentId};
use crate::ui::dock::Workspace;
use crate::ui::task_screens;

impl AppState {
    /// The workspaces on offer: with a project open every one, a task screen only while the project holds its content; else the six of a window with none. A task screen is offered too while its row says the active document asks for it.
    pub fn workspaces_offered(&self) -> Vec<Workspace> {
        let asked = |ws: Workspace| {
            task_screens::of_workspace_alone(ws).is_some_and(|screen| (screen.offered_by)(self))
        };
        // The World screen is offered by a GTA VI folder, with a project or without.
        let world = crate::ui::panels::world_viewer::offered(self);
        if self.project_path.is_some() {
            (Workspace::ALL.iter().copied())
                .filter(|ws| *ws != Workspace::World || world)
                .filter(|ws| {
                    task_screens::of_workspace_alone(*ws).is_none_or(|screen| {
                        asked(*ws)
                            || (self.project_carried.container.modules.iter())
                                .any(|m| m.kind == screen.kind)
                    })
                })
                .collect()
        } else {
            let mut offered = Workspace::WITHOUT_A_PROJECT.to_vec();
            let more = Workspace::ALL.iter().copied().filter(|ws| asked(*ws));
            offered.extend(more.filter(|ws| !Workspace::WITHOUT_A_PROJECT.contains(ws)));
            offered.extend(world.then_some(Workspace::World));
            offered
        }
    }

    /// Show each workspace that is also the editor of a kind of file in the arrangement for a project while the project holds the content of its screen, and in its own with none.
    pub(in crate::state) fn follow_the_arrangements_of_shared_screens(&mut self) {
        for row in task_screens::all()
            .into_iter()
            .filter(|row| row.edits_files)
        {
            let held = self.project_path.is_some()
                && (self.project_carried.container.modules.iter()).any(|m| m.kind == row.kind);
            self.workspace_docks.show_for_a_project(row.workspace, held);
        }
    }

    /// Keep the window on a workspace the project offers: a window on a task screen when the last content of its kind goes is moved to the Project workspace, and goes back when the content returns (Undo of what took it) if the person has not gone elsewhere since.
    pub(super) fn follow_the_offered_workspaces(&mut self) {
        if self.project_path.is_none() {
            return;
        }
        let offered = self.workspaces_offered();
        if self.active_workspace.is_task_screen() && !offered.contains(&self.active_workspace) {
            self.workbench.left_screen = Some(self.active_workspace);
            self.active_workspace = Workspace::Project;
            return;
        }
        let Some(left) = self.workbench.left_screen else {
            return;
        };
        if self.active_workspace != Workspace::Project {
            self.workbench.left_screen = None;
        } else if offered.contains(&left) {
            self.active_workspace = left;
            self.workbench.left_screen = None;
        }
    }

    /// The workspaces the project pinned as buttons, in the order of the strip; all of them when it pinned none.
    pub fn workspaces_pinned(&self) -> Vec<Workspace> {
        let offered = self.workspaces_offered();
        match (&self.project_path, &self.project_carried.shown) {
            (Some(_), Some(pinned)) => {
                let kept: Vec<Workspace> = offered
                    .iter()
                    .copied()
                    .filter(|ws| pinned.contains(ws))
                    .collect();
                if kept.is_empty() {
                    offered
                } else {
                    kept
                }
            }
            _ => offered,
        }
    }

    /// The buttons the strip shows: what the project pinned, and the workspace on screen whether it did or not.
    pub fn workspaces_shown(&self) -> Vec<Workspace> {
        let pinned = self.workspaces_pinned();
        self.workspaces_offered()
            .into_iter()
            .filter(|ws| pinned.contains(ws) || *ws == self.active_workspace)
            .collect()
    }

    /// The screens of the project that are tabs before the tabs of the files: its overview and the screen of each kind of content it holds, among the buttons it pinned, the one on show always among them.
    pub fn screens_as_tabs(&self) -> Vec<Workspace> {
        if self.project_path.is_none() {
            return Vec::new();
        }
        (self.workspaces_shown().into_iter())
            .filter(|ws| ws.needs_a_project())
            .collect()
    }

    /// The buttons the strip draws: the workspaces of a kind of file and the screens asked for that can show the document in front, while more than one can, and the editor of a kind of file that is also the screen of a content the project holds.
    ///
    /// ⚠ A workspace of a kind of file is a way to see a file and not a place to go: with no such file in front its button led to an empty editor, and with one way to see the file there is nothing to choose. The screens of the project are tabs ([`screens_as_tabs`](Self::screens_as_tabs)); View ▸ Workspace, the palette and the keys still reach every one.
    pub fn workspaces_in_the_strip(&self) -> Vec<Workspace> {
        // A screen the document in front asks for, as a scanned map asks for the Map screen, shows that document.
        let asked = |ws: Workspace| {
            task_screens::of_workspace_alone(ws).is_some_and(|screen| (screen.offered_by)(self))
        };
        // Any other screen of the project in front draws no document, so nothing is seen another way.
        let kind = (self.active_document().map(|doc| doc.kind))
            .filter(|_| !self.active_workspace.needs_a_project() || asked(self.active_workspace));
        let ways: Vec<Workspace> = (Workspace::WITHOUT_A_PROJECT.into_iter())
            .filter(|ws| kind.is_some_and(|kind| ws.handles_kind(kind)))
            .chain((Workspace::ALL.iter().copied()).filter(|ws| kind.is_some() && asked(*ws)))
            .collect();
        let held = |ws: Workspace| {
            task_screens::of_workspace(ws).is_some_and(|row| {
                (self.project_carried.container.modules.iter()).any(|m| m.kind == row.kind)
            })
        };
        let shown = self.workspaces_shown();
        self.workspaces_offered()
            .into_iter()
            .filter(|ws| {
                let shared = !ws.needs_a_project() && self.project_path.is_some() && held(*ws);
                (shared && shown.contains(ws)) || (ways.len() > 1 && ways.contains(ws))
            })
            .collect()
    }

    /// The content a screen shows: the one its row was told to while the project still holds it, else the first of its kind.
    fn content_on_show(&self, row: &task_screens::TaskScreen) -> Option<usize> {
        let modules = &self.project_carried.container.modules;
        let kept = (row.showing)(self);
        (modules.iter())
            .position(|m| m.kind == row.kind && Some(&m.id) == kept)
            .or_else(|| modules.iter().position(|m| m.kind == row.kind))
    }

    /// The tabs of the project's screens: its overview, and for the screen of a kind of content one tab, or a tab for each content of that kind while the project holds more than one.
    pub fn screen_tabs(&self) -> Vec<TabPlace> {
        let modules = &self.project_carried.container.modules;
        let mut tabs = Vec::new();
        for ws in self.screens_as_tabs() {
            let of_its_kind: Vec<usize> = match task_screens::of_workspace(ws) {
                Some(row) => (modules.iter().enumerate())
                    .filter(|(_, m)| m.kind == row.kind)
                    .map(|(at, _)| at)
                    .collect(),
                None => Vec::new(),
            };
            match of_its_kind.len() {
                0 | 1 => tabs.push(TabPlace::Screen(ws)),
                _ => tabs.extend(of_its_kind.into_iter().map(|at| TabPlace::Content(ws, at))),
            }
        }
        tabs
    }

    /// The tabs of the window in the order the row draws them, which is the order Ctrl+1 to Ctrl+9 go in: the screens of the project, then the documents that have a tab.
    pub fn tab_places(&self) -> Vec<TabPlace> {
        (self.screen_tabs().into_iter())
            .chain(self.tab_order().into_iter().map(TabPlace::Document))
            .collect()
    }

    /// What the tab in `place` is called: the screen's name, the content's, or the document's as its tab says it.
    pub fn tab_place_name(&self, place: TabPlace) -> String {
        match place {
            TabPlace::Screen(ws) => ws.label().to_string(),
            TabPlace::Content(_, at) => {
                let container = &self.project_carried.container;
                (container.modules.get(at))
                    .map(|m| super::content_name(container, &m.id))
                    .unwrap_or_default()
            }
            TabPlace::Document(id) => (self.document(id))
                .map(|doc| self.tab_title(doc))
                .unwrap_or_default(),
        }
    }

    /// Whether the tab in `place` is the one on show.
    ///
    /// ⚠ A screen of the project in front draws no document, so no file's tab is on show then.
    pub fn tab_on_show(&self, place: TabPlace) -> bool {
        match place {
            TabPlace::Screen(ws) => self.active_workspace == ws,
            TabPlace::Content(ws, at) => {
                let shown =
                    task_screens::of_workspace(ws).and_then(|row| self.content_on_show(row));
                self.active_workspace == ws && shown == Some(at)
            }
            TabPlace::Document(id) => {
                self.active_document == Some(id) && !self.active_workspace.needs_a_project()
            }
        }
    }

    /// Bring the tab in `place` in front.
    pub fn go_to_place(&mut self, place: TabPlace) {
        match place {
            TabPlace::Screen(ws) => self.active_workspace = ws,
            TabPlace::Content(ws, at) => {
                let id = (self.project_carried.container.modules.get(at)).map(|m| m.id.clone());
                if let (Some(row), Some(id)) = (task_screens::of_workspace(ws), id) {
                    (row.show)(self, &id);
                }
                self.active_workspace = ws;
            }
            TabPlace::Document(id) => self.set_active_document(id),
        }
    }

    /// Pin the workspace's button, or unpin it; one button always stays pinned.
    pub fn toggle_pinned(&mut self, ws: Workspace) {
        if self.project_path.is_none() {
            return;
        }
        let mut pinned = self.workspaces_pinned();
        if pinned.contains(&ws) {
            if pinned.len() == 1 {
                return;
            }
            pinned.retain(|p| *p != ws);
        } else {
            pinned.push(ws);
        }
        let offered = self.workspaces_offered();
        pinned.sort_by_key(|p| offered.iter().position(|o| o == p));
        self.project_carried.shown = (pinned != offered).then_some(pinned);
        let word = if self.workspaces_pinned().contains(&ws) {
            tr!("Pinned")
        } else {
            tr!("Unpinned")
        };
        self.record_step(tr!("{word} the {ws} button", word = word, ws = ws.label()));
    }

    /// Bring the `n`th tab of [`tab_places`](Self::tab_places) in front, counting from 1.
    pub fn go_to_tab_number(&mut self, n: usize) -> bool {
        let Some(place) = n
            .checked_sub(1)
            .and_then(|i| self.tab_places().get(i).copied())
        else {
            return false;
        };
        self.go_to_place(place);
        true
    }
}

/// A place in the tab row: a screen of the project, or the tab of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabPlace {
    Screen(Workspace),
    /// A screen of a kind of content the project holds more than one of, showing the content at this place among the project's.
    Content(Workspace, usize),
    Document(DocumentId),
}

/// Which of buttons `widths` wide fit in `room`, and which go under More, as two lists of indices in strip order.
///
/// `more` is the width More takes once anything is under it, `gap` the space between buttons, and `keep` a
/// button that stays whatever the room, as the active workspace's does.
pub fn split_strip(
    widths: &[f32],
    gap: f32,
    room: f32,
    more: f32,
    keep: Option<usize>,
) -> (Vec<usize>, Vec<usize>) {
    let all: f32 = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    if all <= room {
        return ((0..widths.len()).collect(), Vec::new());
    }
    // Each button costs its width and a gap, the last one being the gap before More.
    let budget = room - more;
    let mut shown = vec![false; widths.len()];
    let mut used = 0.0;
    if let Some(keep) = keep.filter(|k| *k < widths.len()) {
        shown[keep] = true;
        used = widths[keep] + gap;
    }
    for (i, width) in widths.iter().enumerate() {
        if shown[i] {
            continue;
        }
        if used + width + gap > budget {
            break;
        }
        shown[i] = true;
        used += width + gap;
    }
    let visible = (0..widths.len()).filter(|i| shown[*i]).collect();
    let hidden = (0..widths.len()).filter(|i| !shown[*i]).collect();
    (visible, hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTHS: [f32; 7] = [70.0, 70.0, 60.0, 70.0, 80.0, 70.0, 50.0];

    fn width_of(widths: &[f32], picked: &[usize], gap: f32) -> f32 {
        picked.iter().map(|i| widths[*i]).sum::<f32>() + gap * picked.len().saturating_sub(1) as f32
    }

    #[test]
    fn when_every_button_fits_none_goes_under_more() {
        let (shown, hidden) = split_strip(&WIDTHS, 4.0, 2000.0, 60.0, Some(3));
        assert_eq!(shown, (0..7).collect::<Vec<_>>());
        assert!(hidden.is_empty());
    }

    #[test]
    fn a_narrow_window_puts_the_buttons_that_do_not_fit_under_more_and_never_wraps() {
        for room in [120.0, 200.0, 300.0, 420.0, 480.0] {
            let (shown, hidden) = split_strip(&WIDTHS, 4.0, room, 60.0, None);
            assert_eq!(shown.len() + hidden.len(), WIDTHS.len());
            assert!(!hidden.is_empty(), "room {room}");
            let used = width_of(&WIDTHS, &shown, 4.0) + 60.0 + 4.0;
            assert!(
                used <= room || shown.is_empty(),
                "room {room}: {shown:?} and More take {used}"
            );
            let mut sorted = shown.clone();
            sorted.sort_unstable();
            assert_eq!(shown, sorted, "the buttons keep the order of the strip");
        }
    }

    #[test]
    fn the_workspace_on_screen_is_always_shown() {
        for keep in 0..WIDTHS.len() {
            let (shown, hidden) = split_strip(&WIDTHS, 4.0, 200.0, 60.0, Some(keep));
            assert!(shown.contains(&keep), "{keep} is under More: {hidden:?}");
        }
    }

    #[test]
    fn the_buttons_a_project_pins_are_the_strip_and_the_active_one_joins_them() {
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        assert_eq!(
            state.workspaces_shown(),
            Workspace::WITHOUT_A_PROJECT,
            "with no project the strip is the six of today"
        );
        state.project_path = Some(std::path::PathBuf::from("p.openrage-proj"));
        state.project_carried.shown = Some(vec![Workspace::Texture, Workspace::Project]);
        state.active_workspace = Workspace::Browse;
        assert_eq!(
            state.workspaces_shown(),
            [Workspace::Project, Workspace::Browse, Workspace::Texture],
            "the strip keeps its order and the active workspace is never hidden"
        );
    }

    #[test]
    fn the_screens_of_the_project_are_tabs_and_the_strip_keeps_only_the_ways_to_see_a_file() {
        use crate::state::DocumentKind;

        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        assert!(
            state.screens_as_tabs().is_empty(),
            "no project, no screen of one"
        );
        state.project_path = Some(std::path::PathBuf::from("p.openrage-proj"));
        state.active_workspace = Workspace::Project;
        assert_eq!(state.screens_as_tabs(), [Workspace::Project]);
        assert!(
            state.workspaces_in_the_strip().is_empty(),
            "a screen of the project is a tab and no button"
        );

        // A model is seen as a model or as its textures: the strip is that choice, and it goes when the overview comes in front.
        let doc = state.new_empty_document(DocumentKind::Ydr);
        state.set_active_document(doc);
        assert_eq!(
            state.workspaces_in_the_strip(),
            [Workspace::Model, Workspace::Texture]
        );
        state.active_workspace = Workspace::Project;
        assert!(state.workspaces_in_the_strip().is_empty());
    }

    #[test]
    fn a_project_with_two_contents_of_one_kind_has_a_tab_for_each_and_one_is_on_show() {
        use or_project::{Module, ModuleKind};

        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.project_path = Some(std::path::PathBuf::from("p.openrage-proj"));
        let pack = |id: &str| Module {
            id: id.into(),
            kind: ModuleKind::Clothing,
            extra: Default::default(),
        };
        state.project_carried.container.modules.push(pack("summer"));
        assert_eq!(
            state.screen_tabs(),
            [
                TabPlace::Screen(Workspace::Project),
                TabPlace::Screen(Workspace::Wardrobe)
            ],
            "one content of a kind is the screen's one tab"
        );

        state.project_carried.container.modules.push(pack("winter"));
        let tabs = state.screen_tabs();
        let packs: Vec<TabPlace> = (tabs.iter().copied())
            .filter(|tab| matches!(tab, TabPlace::Content(Workspace::Wardrobe, _)))
            .collect();
        assert_eq!(packs.len(), 2, "{tabs:?}");
        assert_ne!(
            state.tab_place_name(packs[0]),
            state.tab_place_name(packs[1]),
            "each tab is called as its content is"
        );

        // The second pack's tab shows the second pack, and is then the one on show.
        state.go_to_place(packs[1]);
        assert_eq!(state.active_workspace, Workspace::Wardrobe);
        assert_eq!(state.wardrobe_module_id().as_deref(), Some("winter"));
        assert!(state.tab_on_show(packs[1]) && !state.tab_on_show(packs[0]));
    }
    #[test]
    fn a_number_goes_to_the_tab_of_that_place_in_the_row() {
        use crate::state::DocumentKind;

        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        assert!(
            !state.go_to_tab_number(1),
            "a window with nothing open has no tab"
        );
        assert!(!state.go_to_tab_number(0));

        let text = state.new_empty_document(DocumentKind::Text);
        let model = state.new_empty_document(DocumentKind::Ydr);
        state.set_active_document(model);
        assert!(state.go_to_tab_number(1));
        assert_eq!(state.active_document, Some(text));
        assert_eq!(state.active_workspace, Workspace::Data);

        // With a project its overview is the first tab, and the files follow it.
        state.project_path = Some(std::path::PathBuf::from("p.openrage-proj"));
        assert_eq!(
            state.tab_places(),
            [
                TabPlace::Screen(Workspace::Project),
                TabPlace::Document(text),
                TabPlace::Document(model)
            ]
        );
        assert!(state.go_to_tab_number(1));
        assert_eq!(state.active_workspace, Workspace::Project);
        assert!(state.go_to_tab_number(3));
        assert_eq!(state.active_document, Some(model));
        assert_eq!(state.active_workspace, Workspace::Model);
        assert_eq!(
            state.tab_place_name(TabPlace::Screen(Workspace::Project)),
            "Project"
        );
    }

    #[test]
    fn pinning_a_button_is_remembered_by_the_project_and_one_always_stays() {
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.toggle_pinned(Workspace::Texture);
        assert_eq!(state.project_carried.shown, None, "no project is open");

        state.project_path = Some(std::path::PathBuf::from("p.openrage-proj"));
        state.active_workspace = Workspace::Project;
        state.toggle_pinned(Workspace::Texture);
        let shown = state.project_carried.shown.clone().expect("pinned");
        assert!(!shown.contains(&Workspace::Texture));
        assert!(
            state.project_carried.changed,
            "the project has an edit to save"
        );

        state.toggle_pinned(Workspace::Texture);
        assert_eq!(state.project_carried.shown, None, "every button again");

        for ws in Workspace::ALL {
            state.toggle_pinned(ws);
        }
        assert_eq!(
            state.workspaces_pinned().len(),
            1,
            "the last button cannot be unpinned"
        );
    }
}
