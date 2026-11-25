use std::collections::{HashMap, HashSet};

use egui_dock::{DockArea, DockState, NodeIndex, Style, TabViewer};

use crate::render::RenderBridge;
use crate::state::AppState;
use crate::ui::panel_registry::{self, PanelContext, PanelId};
use crate::ui::panels::registry::ids;

#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Workspace {
    #[default]
    Browse,
    // ⚠ The aliases are names an earlier build wrote into a layout or a
    // project, read as the workspace that took that work over.
    #[serde(alias = "Geometry")]
    Model,
    #[serde(alias = "Explorer")]
    Texture,
    Animation,
    Particle,
    #[serde(alias = "Text", alias = "Meta")]
    Data,
    /// What a project holds and builds; offered only while a project is open.
    Project,
    /// A clothing pack's garments; offered only while the project holds clothing.
    Wardrobe,
    /// The weapons of a weapon pack; offered only while the project holds weapons.
    Weapons,
    /// The maps of a map pack; offered only while the project holds maps.
    Map,
    /// GTA VI's world flown through; offered while a GTA VI folder is set.
    World,
}

impl Workspace {
    /// Every workspace in the order of the buttons: the neutral Project view first, and the task screens of a kind of content last.
    pub const ALL: [Workspace; 11] = [
        Workspace::Project,
        Workspace::Browse,
        Workspace::Model,
        Workspace::Texture,
        Workspace::Animation,
        Workspace::Particle,
        Workspace::Data,
        Workspace::Wardrobe,
        Workspace::Weapons,
        Workspace::Map,
        Workspace::World,
    ];

    /// What is offered with no project open, which is every workspace but Project.
    pub const WITHOUT_A_PROJECT: [Workspace; 6] = [
        Workspace::Browse,
        Workspace::Model,
        Workspace::Texture,
        Workspace::Animation,
        Workspace::Particle,
        Workspace::Data,
    ];

    pub fn label(&self) -> &'static str {
        // A task screen says its own name: it is the row's.
        if let Some(screen) = crate::ui::task_screens::of_workspace_alone(*self) {
            return crate::i18n::tr(screen.label);
        }
        match self {
            Workspace::Project => tr!("Project"),
            Workspace::Browse => tr!("Browse"),
            Workspace::Model => tr!("Model"),
            Workspace::Texture => tr!("Texture"),
            Workspace::Animation => tr!("Animation"),
            Workspace::Particle => tr!("Particle"),
            Workspace::Data => tr!("Data"),
            Workspace::World => tr!("World"),
            // A task screen with no row is a screen of no content, which has no name.
            _ => "",
        }
    }

    pub fn tooltip(&self) -> &'static str {
        if let Some(screen) = crate::ui::task_screens::of_workspace_alone(*self) {
            return crate::i18n::tr(screen.tooltip);
        }
        match self {
            Workspace::Project => {
                tr!("The project's content and builds: every asset it holds, its problems and its builds")
            }
            Workspace::Browse => {
                tr!("Archives, the install, folders, video and files with no editor of their own (.rpf, .bik, …)")
            }
            Workspace::Model => {
                tr!("Drawables, fragments, physics bounds and cloth (.ydr, .ydd, .yft, .ybn, .yld) — viewport and gallery")
            }
            Workspace::Texture => {
                tr!("Texture dictionaries (.ytd), loose .dds and the world's maps — gallery and preview")
            }
            Workspace::Animation => {
                tr!("Animation clip dictionaries (.ycd) — play clips on a skeleton")
            }
            Workspace::Particle => tr!("Particle effects (.ypt)"),
            Workspace::Data => {
                tr!("Metadata, text, string tables, game data tables and audio (.meta, .ymt, .xml, .gxt2, .dat, .rel, .awc, …)")
            }
            Workspace::World => {
                tr!("GTA VI's world as the game streams it at rest, flown through with a free camera")
            }
            _ => "",
        }
    }

    /// Whether the screen belongs to a kind of content and not to a kind of file: it has a row in the table of task screens, and follows a separator in the strip.
    pub fn is_task_screen(self) -> bool {
        crate::ui::task_screens::of_workspace_alone(self).is_some()
    }

    /// Whether the screen is about the open project and not about a document: the Project workspace and every task screen, which are offered only while a project is open.
    pub fn needs_a_project(self) -> bool {
        self == Workspace::Project || self.is_task_screen()
    }

    pub fn for_kind(kind: crate::state::DocumentKind) -> Workspace {
        kind.info().workspace
    }

    pub fn handles_kind(self, kind: crate::state::DocumentKind) -> bool {
        use crate::state::DocumentKind;
        // ⚠ No document is edited on the Project workspace or a task screen: opening one moves to the workspace that suits its kind.
        if self.needs_a_project() {
            return false;
        }
        match self {
            Workspace::Browse => matches!(
                kind,
                DocumentKind::Rpf
                    | DocumentKind::Rpf9
                    | DocumentKind::GameRoot
                    | DocumentKind::Folder
                    | DocumentKind::Bik
                    | DocumentKind::Hex
                    | DocumentKind::Compare
                    | DocumentKind::Registered(_)
            ),
            Workspace::Particle => matches!(kind, DocumentKind::Ypt),
            Workspace::Texture => matches!(
                kind,
                DocumentKind::Ytd
                    | DocumentKind::Ypt
                    | DocumentKind::Ydr
                    | DocumentKind::Ydd
                    | DocumentKind::Yft
                    | DocumentKind::Dds
                    | DocumentKind::Ttpf
                    | DocumentKind::Gta6Drawable
                    | DocumentKind::HeightMap
                    | DocumentKind::WaterMap
                    | DocumentKind::WorldBlanket
                    | DocumentKind::NavPrecalc
            ),
            Workspace::Model => matches!(
                kind,
                DocumentKind::Ydr
                    | DocumentKind::Ydd
                    | DocumentKind::Yft
                    | DocumentKind::Ybn
                    | DocumentKind::Yld
                    | DocumentKind::Ynd
                    | DocumentKind::Ynv
                    | DocumentKind::Fxc
                    | DocumentKind::Gta6Drawable
            ),
            Workspace::Data => matches!(
                kind,
                DocumentKind::Text
                    | DocumentKind::Meta
                    | DocumentKind::Gxt2
                    | DocumentKind::NameTable
                    | DocumentKind::Registered(or_io::container::resource::ResourceType::Ysc)
                    | DocumentKind::DatTable
                    | DocumentKind::DistantLights
                    | DocumentKind::Rel
                    | DocumentKind::Gfx
                    | DocumentKind::Awc
                    | DocumentKind::Gta6Text
                    | DocumentKind::Gta6AudioData
                    | DocumentKind::Gta6Save
                    | DocumentKind::Gta6Script
                    | DocumentKind::Compare
            ),
            Workspace::Animation => matches!(
                kind,
                DocumentKind::Ycd
                    | DocumentKind::Yed
                    | DocumentKind::Yfd
                    | DocumentKind::Ypdb
                    | DocumentKind::MoveNetwork
                    | DocumentKind::Gta6Anim
                    | DocumentKind::Gta6Expressions
            ),
            // The Project workspace and the task screens, which return above.
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tab(pub PanelId);

impl Tab {
    pub const FILES: Tab = Tab(ids::FILES);
    pub const OUTLINE: Tab = Tab(ids::OUTLINE);
    pub const VIEWPORT_3D: Tab = Tab(ids::VIEWPORT_3D);
    pub const TEXTURE_PREVIEW: Tab = Tab(ids::TEXTURE_PREVIEW);
    pub const GALLERY: Tab = Tab(ids::GALLERY);
    pub const DETAILS: Tab = Tab(ids::DETAILS);
    pub const TIMELINE: Tab = Tab(ids::TIMELINE);
    pub const TEXT_EDITOR: Tab = Tab(ids::TEXT_EDITOR);
    pub const DOCUMENT: Tab = Tab(ids::DOCUMENT);
    pub const CLIP_TABLE: Tab = Tab(ids::CLIP_TABLE);
    pub const PROJECT: Tab = Tab(ids::PROJECT);
    pub const WARDROBE: Tab = Tab(ids::WARDROBE);
    pub const DRESSED: Tab = Tab(ids::DRESSED);
    pub const WEAPONS: Tab = Tab(ids::WEAPONS);
    pub const WEAPON_PREVIEW: Tab = Tab(ids::WEAPON_PREVIEW);
    pub const MAPS: Tab = Tab(ids::MAPS);
    pub const MAP_PLAN: Tab = Tab(ids::MAP_PLAN);
    pub const MAP_SCENE: Tab = Tab(ids::MAP_SCENE);
    pub const WORLD_VIEW: Tab = Tab(ids::WORLD_VIEW);
    pub const WORLD: Tab = Tab(ids::WORLD);
    pub const EFFECTS: Tab = Tab(ids::EFFECTS);
    pub const ASSETS: Tab = Tab(ids::ASSETS);
    pub const PROBLEMS: Tab = Tab(ids::PROBLEMS);
    pub const BUILD: Tab = Tab(ids::BUILD);
    pub const HISTORY: Tab = Tab(ids::HISTORY);
    pub const JOBS: Tab = Tab(ids::JOBS);
    pub const LOG: Tab = Tab(ids::LOG);

    pub fn id(self) -> PanelId {
        self.0
    }
}

impl std::fmt::Display for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

pub struct WorkspaceDocks {
    docks: HashMap<Workspace, DockState<Tab>>,
    /// The panel a workspace shows alone, over an arrangement left as it was.
    maximized: HashMap<Workspace, Tab>,
    /// Panels to bring forward once the dock is drawn.
    ///
    /// ⚠ Queued: while the dock draws, `get_mut` hands a panel a copy of the
    /// arrangement, which the drawn arrangement then replaces.
    revealing: Vec<(Workspace, PanelId)>,
    /// Per workspace, the panel focused when its frame was last drawn, and the one focused before it.
    focus_trail: HashMap<Workspace, (Option<PanelId>, Option<PanelId>)>,
    /// The arrangement a workspace that is also a task screen keeps for the other case: the one for a project that holds its content while the other is on screen, and the one with no project while that is.
    stashed: HashMap<Workspace, DockState<Tab>>,
    /// The workspaces now showing their arrangement for a project.
    for_a_project: HashSet<Workspace>,
}

impl Default for WorkspaceDocks {
    fn default() -> Self {
        let mut docks = HashMap::new();
        for ws in Workspace::ALL {
            docks.insert(ws, init_default_layout(ws));
        }
        let stashed = (crate::ui::task_screens::all().into_iter())
            .filter(|row| row.edits_files)
            .map(|row| (row.workspace, (row.layout)()))
            .collect();
        Self {
            docks,
            maximized: HashMap::new(),
            revealing: Vec::new(),
            focus_trail: HashMap::new(),
            stashed,
            for_a_project: HashSet::new(),
        }
    }
}

impl WorkspaceDocks {
    /// The defaults, with every workspace a saved layout could supply
    /// replaced by what it says.
    pub fn restored(saved: &crate::ui::dock_layout::SavedLayouts) -> Self {
        let mut docks = Self::default();
        let restored = crate::ui::dock_layout::restore(saved);
        for (ws, dock) in restored.usable {
            docks.docks.insert(ws, dock);
        }
        for (ws, why) in restored.rejected {
            tracing::warn!(workspace = %ws, reason = ?why, "Saved dock layout not restored");
        }
        for (ws, dock) in restored.for_a_project {
            docks.stashed.insert(ws, dock);
        }
        docks
    }

    /// The arrangement, to change; a maximized panel goes back into it so the change shows.
    pub fn get_mut(&mut self, ws: Workspace) -> &mut DockState<Tab> {
        self.maximized.remove(&ws);
        self.docks
            .entry(ws)
            .or_insert_with(|| init_default_layout(ws))
    }

    pub fn peek(&self, ws: Workspace) -> Option<&DockState<Tab>> {
        self.docks.get(&ws)
    }

    pub fn reset(&mut self, ws: Workspace) {
        self.maximized.remove(&ws);
        let layout = match crate::ui::task_screens::of_workspace(ws) {
            Some(row) if row.edits_files && self.for_a_project.contains(&ws) => (row.layout)(),
            _ => init_default_layout(ws),
        };
        self.docks.insert(ws, layout);
    }

    /// Show the arrangement `ws` keeps for a project that holds its content, or the one it has with none.
    ///
    /// ⚠ The other arrangement is kept aside, whole, and comes back as it was left: the Particle workspace is used with no project open, and a window with none must not find the project's panels in it.
    pub fn show_for_a_project(&mut self, ws: Workspace, project: bool) {
        if self.for_a_project.contains(&ws) == project || !self.stashed.contains_key(&ws) {
            return;
        }
        self.maximized.remove(&ws);
        let aside = self.stashed.remove(&ws).expect("kept aside");
        let shown = self
            .docks
            .insert(ws, aside)
            .unwrap_or_else(|| init_default_layout(ws));
        self.stashed.insert(ws, shown);
        if project {
            self.for_a_project.insert(ws);
        } else {
            self.for_a_project.remove(&ws);
        }
    }

    /// The arrangement of `ws` for a project that holds its content, wherever it is kept; none for a workspace that has none.
    pub fn arrangement_for_a_project(&self, ws: Workspace) -> Option<&DockState<Tab>> {
        if self.for_a_project.contains(&ws) {
            self.docks.get(&ws)
        } else {
            self.stashed.get(&ws)
        }
    }

    /// The arrangement of `ws` with no project, wherever it is kept.
    pub fn arrangement_without_a_project(&self, ws: Workspace) -> Option<&DockState<Tab>> {
        if self.for_a_project.contains(&ws) {
            self.stashed.get(&ws)
        } else {
            self.docks.get(&ws)
        }
    }

    pub fn maximized(&self, ws: Workspace) -> Option<Tab> {
        self.maximized.get(&ws).copied()
    }

    /// Whether `ws` draws the panel `id`: in its arrangement, or alone while maximized.
    pub fn shows(&self, ws: Workspace, id: PanelId) -> bool {
        match self.maximized(ws) {
            Some(alone) => alone.id() == id,
            None => self
                .peek(ws)
                .is_some_and(|dock| dock.iter_all_tabs().any(|(_, tab)| tab.id() == id)),
        }
    }

    /// Bring a panel forward in `ws`, adding it if absent, once the dock has been drawn.
    pub fn reveal(&mut self, ws: Workspace, id: PanelId) {
        self.revealing.push((ws, id));
    }

    /// Whether `id` is queued to come forward in `ws` once the dock is drawn.
    #[cfg(test)]
    pub(crate) fn is_revealing(&self, ws: Workspace, id: PanelId) -> bool {
        self.revealing.contains(&(ws, id))
    }

    /// Apply the queued reveals; `false` when none was queued.
    fn apply_reveals(&mut self) -> bool {
        let queued = std::mem::take(&mut self.revealing);
        for &(ws, id) in &queued {
            crate::ui::dock_commands::reveal(self.get_mut(ws), id);
        }
        !queued.is_empty()
    }

    /// The panel the user last clicked in `ws`: the one shown alone, else the focused leaf's open tab.
    pub fn focused(&self, ws: Workspace) -> Option<PanelId> {
        if let Some(alone) = self.maximized(ws) {
            return Some(alone.id());
        }
        let dock = self.peek(ws)?;
        let leaf = dock.leaf(dock.focused_leaf()?).ok()?;
        leaf.tabs.get(leaf.active.0).map(|tab| tab.id())
    }

    /// The panel that had focus in `ws` before the focused one took it.
    pub fn focused_before(&self, ws: Workspace) -> Option<PanelId> {
        let (drawn, before) = self.focus_trail.get(&ws).copied().unwrap_or_default();
        // ⚠ Focus can move between frames, as View ▸ Panels moves it to the panel it shows.
        if drawn == self.focused(ws) {
            before
        } else {
            drawn
        }
    }

    /// Note the panel focused in `ws` once its frame is drawn, keeping the one it took focus from.
    fn note_focus(&mut self, ws: Workspace) {
        let now = self.focused(ws);
        let trail = self.focus_trail.entry(ws).or_default();
        if trail.0 != now {
            *trail = (now, trail.0);
        }
    }

    /// Show the focused panel alone, or the arrangement again; `false` when no panel has focus.
    pub fn toggle_maximized(&mut self, ws: Workspace) -> bool {
        if self.maximized.remove(&ws).is_some() {
            return true;
        }
        let focused = self
            .docks
            .get_mut(&ws)
            .and_then(|dock| dock.find_active_focused())
            .map(|(_, tab)| *tab);
        let Some(tab) = focused else {
            return false;
        };
        self.maximized.insert(ws, tab);
        true
    }
}

fn init_default_layout(ws: Workspace) -> DockState<Tab> {
    layout_for(ws)
}

fn center_tabs(ws: Workspace) -> Vec<Tab> {
    match ws {
        Workspace::Project => vec![Tab::ASSETS],
        Workspace::Browse => vec![Tab::DOCUMENT],
        Workspace::Model | Workspace::Particle => vec![Tab::VIEWPORT_3D],
        Workspace::Texture => vec![Tab::GALLERY],
        Workspace::Animation => vec![Tab::VIEWPORT_3D, Tab::CLIP_TABLE],
        Workspace::Data => vec![Tab::DOCUMENT, Tab::TEXT_EDITOR],
        Workspace::World => vec![Tab::WORLD_VIEW],
        // A task screen's layout names its own centre: the panel it opens with the focus on.
        task => crate::ui::task_screens::of_workspace_alone(task)
            .and_then(|screen| (screen.layout)().find_active_focused().map(|(_, tab)| *tab))
            .into_iter()
            .collect(),
    }
}

/// The Project workspace: content and builds beside the outline, the assets between, their properties on the right, and what needs attention along the bottom.
fn project_layout() -> DockState<Tab> {
    let mut state = DockState::new(center_tabs(Workspace::Project));
    let surface = state.main_surface_mut();
    // ⚠ The outline is a tab beside Content and Builds, which need the whole column.
    let [center, _] = surface.split_left(NodeIndex::root(), 0.22, vec![Tab::PROJECT, Tab::OUTLINE]);
    let [center, _] = surface.split_right(center, 0.74, vec![Tab::DETAILS]);
    let [center, _] = surface.split_below(
        center,
        0.64,
        vec![Tab::PROBLEMS, Tab::BUILD, Tab::HISTORY, Tab::JOBS, Tab::LOG],
    );
    state.set_focused_node_and_surface(egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: center,
    });
    state
}

fn layout_for(ws: Workspace) -> DockState<Tab> {
    if ws == Workspace::Project {
        return project_layout();
    }
    if let Some(screen) = crate::ui::task_screens::of_workspace_alone(ws) {
        return (screen.layout)();
    }
    if ws == Workspace::World {
        return world_layout();
    }
    let mut state = DockState::new(center_tabs(ws));
    let surface = state.main_surface_mut();

    // ⚠ Browse is where the game's files are looked for, so the search of what files hold and the installed mods are tabs beside Files there, and not panels only View ▸ Panels leads to.
    let files = match ws {
        Workspace::Browse => vec![Tab::FILES, Tab(ids::SEARCH), Tab(ids::MODS)],
        _ => vec![Tab::FILES],
    };
    let [center, left] = surface.split_left(NodeIndex::root(), 0.22, files);
    surface.split_below(left, 0.55, vec![Tab::OUTLINE]);
    let [mut center, _] = surface.split_right(center, 0.72, inspector_leaf_tabs(ws));
    match ws {
        // The gallery beside the view it feeds, so picking a dictionary's
        // tile shows it without switching anything.
        Workspace::Model => [center, _] = surface.split_right(center, 0.72, vec![Tab::GALLERY]),
        Workspace::Texture => {
            [center, _] = surface.split_right(center, 0.5, vec![Tab::TEXTURE_PREVIEW]);
        }
        Workspace::Animation | Workspace::Particle => {
            [center, _] = surface.split_below(center, 0.78, vec![Tab::TIMELINE]);
        }
        _ => {}
    }
    // The centre starts focused, so Maximize has a panel before any click.
    state.set_focused_node_and_surface(egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: center,
    });

    state
}

/// The World workspace: the view, what the mount found and what is streaming beside it, and the jobs under it.
fn world_layout() -> DockState<Tab> {
    let mut state = DockState::new(center_tabs(Workspace::World));
    let surface = state.main_surface_mut();
    let [center, _] = surface.split_right(NodeIndex::root(), 0.78, vec![Tab::WORLD]);
    let [center, _] = surface.split_below(center, 0.86, vec![Tab::JOBS, Tab::LOG]);
    state.set_focused_node_and_surface(egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: center,
    });
    state
}

fn inspector_leaf_tabs(ws: Workspace) -> Vec<Tab> {
    match ws {
        Workspace::Particle => vec![Tab::DETAILS, Tab::TEXTURE_PREVIEW],
        _ => vec![Tab::DETAILS],
    }
}

struct TabViewerImpl<'a> {
    state: &'a mut AppState,
    bridge: Option<&'a mut RenderBridge>,
}

impl TabViewer for TabViewerImpl<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        panel_registry::title_of(tab.id(), self.state).into()
    }

    // ⚠ egui_dock paints a tab's title itself and names nothing for AccessKit.
    fn on_tab_button(&mut self, tab: &mut Self::Tab, response: &egui::Response) {
        let title = panel_registry::title_of(tab.id(), self.state);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title.clone())
        });
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        let mut ctx = PanelContext {
            state: self.state,
            bridge: self.bridge.as_deref_mut(),
        };
        panel_registry::draw_panel(tab.id(), ui, &mut ctx);
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    let active = state.active_workspace;

    let mut style = Style::from_egui(ui.ctx().global_style().as_ref());
    style.tab_bar.fill_tab_bar = true;
    style.tab_bar.height = 22.0;
    style.tab_bar.hline_color = crate::ui::theme::BORDER.get();
    style.tab_bar.bg_fill = crate::ui::theme::BG_L0.get();
    // The focused panel, which Maximize shows alone.
    for focused in [&mut style.tab.focused, &mut style.tab.focused_with_kb_focus] {
        focused.text_color = crate::ui::theme::ACCENT_TEXT.get();
        focused.outline_color = crate::ui::theme::ACCENT.get();
    }

    // ⚠ The start page raised over the workspace takes the whole centre, and so does the page of a window with nothing open: panels that each say they are empty are not drawn around it.
    let nothing_open =
        active == Workspace::Browse && state.documents.is_empty() && state.project_path.is_none();
    if state.front_door.raised(active) || nothing_open {
        egui::CentralPanel::default()
            // ⚠ The dock paints the panel colour under a workspace; with it gone the window's clear colour shows.
            .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
            .show_inside(ui, |ui| {
                crate::ui::panels::game_picker::draw_start_page(ui, state)
            });
        return;
    }
    // ⚠ The page covers the workspace it was raised over and no other: a frame drawn on another one
    // ends it, or going back to the first would find it there again.
    state.hide_start_page();
    state.workspace_docks.apply_reveals();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            if active == Workspace::Project && state.project_path.is_some() {
                crate::ui::os_drop::note_project_area(ui.ctx(), state, ui.max_rect());
            }
            let alone = state.workspace_docks.maximized(active);
            let mut dock_state = match alone {
                // ⚠ A dock of its own every frame, so nothing done to it
                // reaches the arrangement under it or `prefs.json`.
                Some(tab) => DockState::new(vec![tab]),
                // ⚠ A copy stays while the arrangement is out being drawn,
                // so a panel asking what the workspace shows is answered.
                None => {
                    let docked = state.workspace_docks.get_mut(active);
                    let copy = docked.clone();
                    std::mem::replace(docked, copy)
                }
            };
            DockArea::new(&mut dock_state)
                .style(style)
                .show_close_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_leaf_collapse_buttons(false)
                .draggable_tabs(alone.is_none())
                .show_inside(ui, &mut TabViewerImpl { state, bridge });
            if alone.is_none() {
                *state.workspace_docks.get_mut(active) = dock_state;
            }
        });
    if state.workspace_docks.apply_reveals() {
        ui.ctx().request_repaint();
    }
    state.workspace_docks.note_focus(active);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state with a folder open, so that a frame draws the dock: a window with nothing open draws the start page in its place.
    fn with_a_folder_open() -> (AppState, tempfile::TempDir) {
        let folder = tempfile::tempdir().expect("a scratch folder");
        let mut state = AppState::default();
        state.open_folder(folder.path().to_path_buf());
        (state, folder)
    }

    /// The panels a frame of the active workspace drew: an id nothing
    /// provides draws as `MissingPanel`, which names it.
    ///
    /// ⚠ A frame's accesskit tree can name one panel more than once, so each name counts once.
    fn drawn(ctx: &egui::Context, state: &mut AppState) -> Vec<String> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| draw(ui, state, None));
        let mut names: Vec<String> = output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let text = node.value()?;
                        let id = text
                            .strip_prefix("Nothing provides `")?
                            .strip_suffix("`.")?;
                        Some(id.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names.dedup();
        names
    }

    #[test]
    fn video_and_audio_play_in_the_centre_and_never_under_a_timeline() {
        use crate::state::DocumentKind;
        let docks = WorkspaceDocks::default();
        for kind in [DocumentKind::Bik, DocumentKind::Awc] {
            let ws = Workspace::for_kind(kind);
            let tabs: Vec<PanelId> = docks
                .peek(ws)
                .expect("built at default")
                .iter_all_tabs()
                .map(|(_, tab)| tab.id())
                .collect();
            assert!(
                tabs.contains(&ids::DOCUMENT),
                "{kind:?} opens into {ws:?}, and nothing there plays it: {tabs:?}"
            );
            assert!(
                !tabs.contains(&ids::TIMELINE),
                "{kind:?} plays under a timeline in {ws:?}"
            );
        }
    }

    #[test]
    fn maximize_shows_the_focused_panel_alone_and_again_puts_the_arrangement_back() {
        let (mut state, _folder) = with_a_folder_open();
        let ws = state.active_workspace;
        let dock = state.workspace_docks.get_mut(ws);
        *dock = DockState::new(vec![Tab("probe-left")]);
        dock.main_surface_mut()
            .split_right(NodeIndex::root(), 0.5, vec![Tab("probe-right")]);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let both = ["probe-left", "probe-right"];
        assert_eq!(drawn(&ctx, &mut state), both);
        assert!(
            !state.workspace_docks.toggle_maximized(ws),
            "no panel has focus yet"
        );

        crate::ui::dock_commands::reveal(state.workspace_docks.get_mut(ws), "probe-left");
        state.workspace_docks.toggle_maximized(ws);
        assert_eq!(drawn(&ctx, &mut state), ["probe-left"]);
        assert_eq!(drawn(&ctx, &mut state), ["probe-left"], "and stays alone");

        assert!(state.workspace_docks.toggle_maximized(ws));
        assert_eq!(drawn(&ctx, &mut state), both);

        state.workspace_docks.toggle_maximized(ws);
        state.workspace_docks.get_mut(ws);
        assert_eq!(
            drawn(&ctx, &mut state),
            both,
            "a change to the arrangement shows the arrangement"
        );
    }

    fn screen(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 900.0),
            )),
            events,
            ..Default::default()
        }
    }

    /// Every text a frame of the dock paints where it can be seen, with the colour it is painted in.
    ///
    /// ⚠ A tab bar of no height still paints its titles, clipped to nothing.
    fn painted(ctx: &egui::Context, state: &mut AppState) -> Vec<(String, egui::Color32)> {
        fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<(String, egui::Color32)>) {
            match shape {
                egui::Shape::Text(text)
                    if clip.intersect(text.visual_bounding_rect()).is_positive() =>
                {
                    out.push((text.galley.text().to_string(), text.fallback_color))
                }
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
                _ => {}
            }
        }
        let output = ctx.run_ui(screen(Vec::new()), |ui| draw(ui, state, None));
        let mut out = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, clipped.clip_rect, &mut out);
        }
        out
    }

    fn titles(state: &AppState, ws: Workspace) -> Vec<String> {
        state
            .workspace_docks
            .peek(ws)
            .expect("built at default")
            .iter_all_tabs()
            .map(|(_, tab)| panel_registry::title_of(tab.id(), state))
            .collect()
    }

    #[test]
    fn every_panels_title_is_drawn_whatever_its_leaf_holds() {
        let (mut state, _folder) = with_a_folder_open();
        let ctx = egui::Context::default();
        for ws in Workspace::ALL {
            state.active_workspace = ws;
            state.workspace_docks.reset(ws);
            painted(&ctx, &mut state);
            let texts: Vec<String> = painted(&ctx, &mut state)
                .into_iter()
                .map(|(text, _)| text)
                .collect();
            for title in titles(&state, ws) {
                assert!(
                    texts.contains(&title),
                    "{ws:?} draws no title for its {title} panel"
                );
            }
        }
    }

    #[test]
    fn the_focused_panel_is_marked_and_maximize_shows_it_without_a_click() {
        let _serial = crate::ui::theme::serial();
        let (mut state, _folder) = with_a_folder_open();
        let ctx = egui::Context::default();
        for ws in Workspace::ALL {
            state.active_workspace = ws;
            state.workspace_docks.reset(ws);
            let centre = center_tabs(ws)[0];
            let titles = titles(&state, ws);
            painted(&ctx, &mut state);
            let marked: Vec<String> = painted(&ctx, &mut state)
                .into_iter()
                .filter(|(text, colour)| {
                    titles.contains(text) && *colour == crate::ui::theme::ACCENT_TEXT.get()
                })
                .map(|(text, _)| text)
                .collect();
            assert_eq!(
                marked,
                [panel_registry::title_of(centre.id(), &state)],
                "{ws:?} marks another panel, or none, as focused"
            );
            assert!(
                state.workspace_docks.toggle_maximized(ws),
                "{ws:?} asked for a click before it would maximize"
            );
            assert_eq!(state.workspace_docks.maximized(ws), Some(centre));
            state.workspace_docks.toggle_maximized(ws);
        }
    }

    /// The eye button in the tree's toolbar shows the preview pane from inside the dock's own draw.
    #[test]
    fn a_reveal_from_inside_a_panels_draw_shows_its_panel() {
        use crate::ui::dock_commands::{placement, Placement};
        let (mut state, _folder) = with_a_folder_open();
        let ws = state.active_workspace;
        state.workspace_docks.reset(ws);
        let shown = |state: &AppState| {
            placement(
                state.workspace_docks.peek(ws).expect("built at default"),
                ids::FILE_PREVIEW,
            )
        };
        assert_eq!(shown(&state), Placement::Absent, "no default shows it");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let frame = |state: &mut AppState, events| -> Vec<(String, egui::Pos2)> {
            let output = ctx.run_ui(screen(events), |ui| draw(ui, state, None));
            output
                .platform_output
                .accesskit_update
                .map(|update| {
                    update
                        .nodes
                        .iter()
                        .filter_map(|(_, node)| {
                            let b = node.bounds()?;
                            let centre = egui::pos2(
                                ((b.x0 + b.x1) / 2.0) as f32,
                                ((b.y0 + b.y1) / 2.0) as f32,
                            );
                            Some((node.label()?.to_string(), centre))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        frame(&mut state, Vec::new());
        let eye = frame(&mut state, Vec::new())
            .into_iter()
            .find(|(label, _)| label == "Show the preview pane")
            .map(|(_, at)| at)
            .expect("the tree's toolbar offers the preview pane");
        let press = |pressed| egui::Event::PointerButton {
            pos: eye,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut state,
            vec![egui::Event::PointerMoved(eye), press(true)],
        );
        frame(&mut state, vec![press(false)]);
        frame(&mut state, Vec::new());
        assert_eq!(
            shown(&state),
            Placement::Docked,
            "the click's reveal was lost under the arrangement the dock drew"
        );
    }
}
