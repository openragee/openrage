use crate::state::AppState;
use crate::ui::panel_registry::{Panel, PanelContext, PanelId, PanelRegistry};
use crate::ui::panels;
use crate::ui::Workspace;

pub mod ids {
    use super::PanelId;

    pub const FILES: PanelId = "files";
    pub const OUTLINE: PanelId = "outline";
    pub const VIEWPORT_3D: PanelId = "viewport3d";
    pub const TEXTURE_PREVIEW: PanelId = "texture_preview";
    pub const GALLERY: PanelId = "gallery";
    pub const DETAILS: PanelId = "details";
    pub const TIMELINE: PanelId = "timeline";
    pub const TEXT_EDITOR: PanelId = "text_editor";
    pub const META_EDITOR: PanelId = "meta_editor";
    pub const CLIP_TABLE: PanelId = "clip_table";
    pub const SEARCH: PanelId = "search";
    pub const LOG: PanelId = "log";
    pub const FILE_PREVIEW: PanelId = "file_preview";
    pub const STATISTICS: PanelId = "statistics";
    pub const DOCUMENT: PanelId = "document";
    pub const HISTORY: PanelId = "history";
    pub const MODS: PanelId = "mods";
    pub const PROJECT: PanelId = "project";
    pub const ASSETS: PanelId = "assets";
    pub const PROBLEMS: PanelId = "problems";
    pub const BUILD: PanelId = "build";
    pub const JOBS: PanelId = "jobs";
    pub const WARDROBE: PanelId = "wardrobe";
    pub const DRESSED: PanelId = "dressed_ped";
    pub const WEAPONS: PanelId = "weapons";
    pub const WEAPON_PREVIEW: PanelId = "weapon_preview";
    pub const MAPS: PanelId = "maps";
    pub const MAP_PLAN: PanelId = "map_plan";
    pub const MAP_SCENE: PanelId = "map_scene";
    pub const WORLD_VIEW: PanelId = "world_view";
    pub const WORLD: PanelId = "world";
    pub const EFFECTS: PanelId = "effects";
    pub const COMMAND_LOG: PanelId = "command_log";
    pub const PLUGINS: PanelId = "plugins";
}

macro_rules! state_only_panel {
    ($ty:ident, $id:expr, $title:expr, $draw:path) => {
        pub struct $ty;

        impl Panel for $ty {
            fn id(&self) -> PanelId {
                $id
            }

            fn title(&self, _state: &AppState) -> String {
                crate::i18n::tr($title).to_string()
            }

            fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
                $draw(ui, ctx.state)
            }
        }
    };
}

state_only_panel!(Files, ids::FILES, tr_key!("Files"), panels::hierarchy::draw);
state_only_panel!(
    Outline,
    ids::OUTLINE,
    tr_key!("Outline"),
    panels::hierarchy::draw_outline
);
state_only_panel!(
    TexturePreview,
    ids::TEXTURE_PREVIEW,
    tr_key!("Preview"),
    panels::texture_preview::draw
);
pub struct Gallery;

impl Panel for Gallery {
    fn id(&self) -> PanelId {
        ids::GALLERY
    }

    fn title(&self, _state: &AppState) -> String {
        tr!("Gallery").to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
        let bridge = ctx.bridge.as_deref_mut();
        panels::gallery::draw(ui, ctx.state, bridge);
    }
}
/// The one panel of what is selected on the screen in front: what a task screen's row names, a project's properties, else the fields of the open file.
pub struct Details;

/// Whether Details draws the fields of the open file on the screen in front, and neither a task screen's selection nor a project's properties.
pub fn details_are_the_files_fields(state: &AppState) -> bool {
    let ws = state.active_workspace;
    ws != Workspace::Project
        && crate::ui::task_screens::of_workspace(ws).is_none_or(|row| row.details.is_none())
}

impl Panel for Details {
    fn id(&self) -> PanelId {
        ids::DETAILS
    }

    fn title(&self, _state: &AppState) -> String {
        tr!("Details").to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
        // The row of Files whose Properties were asked for stands above whatever the screen selects.
        panels::hierarchy::draw_properties(ui, ctx.state);
        let ws = ctx.state.active_workspace;
        let of_the_screen = crate::ui::task_screens::of_workspace(ws).and_then(|row| row.details);
        match of_the_screen {
            Some(draw) => draw(ui, ctx.state),
            None if ws == Workspace::Project => panels::project::draw_properties(ui, ctx.state),
            None => panels::inspector::draw(ui, ctx.state),
        }
    }
}
state_only_panel!(
    MetaEditor,
    ids::META_EDITOR,
    tr_key!("Visual"),
    panels::meta_editor::draw
);
state_only_panel!(
    ClipTable,
    ids::CLIP_TABLE,
    tr_key!("Clips"),
    panels::ycd_clips::draw
);

state_only_panel!(Search, ids::SEARCH, tr_key!("Search"), panels::search::draw);
state_only_panel!(Log, ids::LOG, tr_key!("Log"), panels::log::draw);
state_only_panel!(
    Statistics,
    ids::STATISTICS,
    tr_key!("Statistics"),
    panels::statistics::draw
);

/// A panel that also draws through the renderer: a model's render is a thumbnail it makes.
macro_rules! rendering_panel {
    ($ty:ident, $id:expr, $title:expr, $draw:path) => {
        pub struct $ty;

        impl Panel for $ty {
            fn id(&self) -> PanelId {
                $id
            }

            fn title(&self, _state: &AppState) -> String {
                crate::i18n::tr($title).to_string()
            }

            fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
                let bridge = ctx.bridge.as_deref_mut();
                $draw(ui, ctx.state, bridge)
            }
        }
    };
}

rendering_panel!(
    FilePreview,
    ids::FILE_PREVIEW,
    tr_key!("File preview"),
    panels::file_preview::draw
);
rendering_panel!(
    Document,
    ids::DOCUMENT,
    tr_key!("Document"),
    panels::document_view::draw
);
state_only_panel!(
    History,
    ids::HISTORY,
    tr_key!("History"),
    panels::history::draw
);
state_only_panel!(
    Mods,
    ids::MODS,
    tr_key!("Installed mods"),
    panels::mods::draw
);
state_only_panel!(
    ProjectContent,
    ids::PROJECT,
    tr_key!("Project"),
    panels::project::draw_content
);
state_only_panel!(
    Assets,
    ids::ASSETS,
    tr_key!("Assets"),
    panels::project::draw_assets
);
state_only_panel!(
    Problems,
    ids::PROBLEMS,
    tr_key!("Problems"),
    panels::project::draw_problems
);
state_only_panel!(
    Build,
    ids::BUILD,
    tr_key!("Build"),
    panels::project::draw_build
);
state_only_panel!(Jobs, ids::JOBS, tr_key!("Jobs"), panels::jobs::draw_panel);
state_only_panel!(
    Wardrobe,
    ids::WARDROBE,
    tr_key!("Wardrobe"),
    panels::wardrobe::draw_list
);
rendering_panel!(
    Dressed,
    ids::DRESSED,
    tr_key!("Dressed ped"),
    panels::wardrobe::draw_preview
);
state_only_panel!(
    Weapons,
    ids::WEAPONS,
    tr_key!("Weapons"),
    panels::weapons::draw_list
);
rendering_panel!(
    WeaponPreview,
    ids::WEAPON_PREVIEW,
    tr_key!("Assembled weapon"),
    panels::weapons::draw_preview
);
state_only_panel!(Maps, ids::MAPS, tr_key!("Maps"), panels::maps::draw_list);
state_only_panel!(
    MapPlan,
    ids::MAP_PLAN,
    tr_key!("Plan"),
    panels::maps::draw_plan
);
rendering_panel!(
    MapScene,
    ids::MAP_SCENE,
    tr_key!("Scene"),
    panels::maps::draw_scene
);
rendering_panel!(
    WorldView,
    ids::WORLD_VIEW,
    tr_key!("World view"),
    panels::world_viewer::draw_view
);
rendering_panel!(
    World,
    ids::WORLD,
    tr_key!("World"),
    panels::world_viewer::draw_side
);
state_only_panel!(
    Effects,
    ids::EFFECTS,
    tr_key!("Effects"),
    panels::particles::draw_effects
);
// ⚠ In no layout: Automate ▸ Command log brings it forward in the workspace on screen, as Help ▸ Log does the Log.
state_only_panel!(
    Commands,
    ids::COMMAND_LOG,
    tr_key!("Commands"),
    panels::command_log::draw
);

// ⚠ In no layout, as the log of commands is: Automate brings it forward, and it took the place of a page of Settings.
state_only_panel!(
    PluginsPanel,
    ids::PLUGINS,
    tr_key!("Plugins"),
    panels::settings_plugins::draw_panel
);

pub struct Viewport3D;

impl Panel for Viewport3D {
    fn id(&self) -> PanelId {
        ids::VIEWPORT_3D
    }

    fn title(&self, _state: &AppState) -> String {
        tr!("Viewport").to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
        let bridge = ctx.bridge.as_deref_mut();
        panels::viewport::draw(ui, ctx.state, bridge);
    }
}

pub struct Timeline;

impl Panel for Timeline {
    fn id(&self) -> PanelId {
        ids::TIMELINE
    }

    fn title(&self, _state: &AppState) -> String {
        tr!("Timeline").to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
        let gta6 = (ctx.state.active_document())
            .filter(|d| d.kind == crate::state::DocumentKind::Gta6Anim)
            .map(|d| d.id);
        match ctx.state.active_workspace {
            Workspace::Animation => match gta6 {
                Some(doc) => panels::inspector::gta6_timeline(ui, ctx.state, doc),
                None => panels::timeline_ycd::draw(ui, ctx.state),
            },
            _ => panels::timeline::draw(ui, ctx.state),
        }
    }
}

pub struct TextEditor;

impl Panel for TextEditor {
    fn id(&self) -> PanelId {
        ids::TEXT_EDITOR
    }

    fn title(&self, _state: &AppState) -> String {
        tr!("Text").to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
        let is_gxt2 = ctx
            .state
            .active_document()
            .is_some_and(|d| matches!(d.payload, crate::state::DocPayload::Gxt2(_)));
        if is_gxt2 {
            panels::gxt2_table::draw(ui, ctx.state);
        } else {
            panels::text_editor::draw(ui, ctx.state);
        }
    }
}

static FILES: Files = Files;
static OUTLINE: Outline = Outline;
static VIEWPORT_3D: Viewport3D = Viewport3D;
static TEXTURE_PREVIEW: TexturePreview = TexturePreview;
static GALLERY: Gallery = Gallery;
static DETAILS: Details = Details;
static TIMELINE: Timeline = Timeline;
static TEXT_EDITOR: TextEditor = TextEditor;
static META_EDITOR: MetaEditor = MetaEditor;
static CLIP_TABLE: ClipTable = ClipTable;
static SEARCH: Search = Search;
static LOG: Log = Log;
static FILE_PREVIEW: FilePreview = FilePreview;
static STATISTICS: Statistics = Statistics;
static DOCUMENT: Document = Document;
static HISTORY: History = History;
static MODS: Mods = Mods;
static PROJECT: ProjectContent = ProjectContent;
static ASSETS: Assets = Assets;
static PROBLEMS: Problems = Problems;
static BUILD: Build = Build;
static JOBS: Jobs = Jobs;
static WARDROBE: Wardrobe = Wardrobe;
static DRESSED: Dressed = Dressed;
static WEAPONS: Weapons = Weapons;
static WEAPON_PREVIEW: WeaponPreview = WeaponPreview;
static MAPS: Maps = Maps;
static MAP_PLAN: MapPlan = MapPlan;
static MAP_SCENE: MapScene = MapScene;
static WORLD_VIEW: WorldView = WorldView;
static WORLD: World = World;
static EFFECTS: Effects = Effects;
static COMMANDS: Commands = Commands;
static PLUGINS: PluginsPanel = PluginsPanel;

pub fn register_builtin(registry: &mut PanelRegistry) {
    registry.register(&FILES);
    registry.register(&OUTLINE);
    registry.register(&VIEWPORT_3D);
    registry.register(&TEXTURE_PREVIEW);
    registry.register(&GALLERY);
    registry.register(&DETAILS);
    registry.register(&TIMELINE);
    registry.register(&TEXT_EDITOR);
    registry.register(&META_EDITOR);
    registry.register(&CLIP_TABLE);
    registry.register(&SEARCH);
    registry.register(&LOG);
    registry.register(&FILE_PREVIEW);
    registry.register(&STATISTICS);
    registry.register(&DOCUMENT);
    registry.register(&HISTORY);
    registry.register(&MODS);
    registry.register(&PROJECT);
    registry.register(&ASSETS);
    registry.register(&PROBLEMS);
    registry.register(&BUILD);
    registry.register(&JOBS);
    registry.register(&WARDROBE);
    registry.register(&DRESSED);
    registry.register(&WEAPONS);
    registry.register(&WEAPON_PREVIEW);
    registry.register(&MAPS);
    registry.register(&MAP_PLAN);
    registry.register(&MAP_SCENE);
    registry.register(&WORLD_VIEW);
    registry.register(&WORLD);
    registry.register(&EFFECTS);
    registry.register(&COMMANDS);
    registry.register(&PLUGINS);
}

pub fn builtin() -> PanelRegistry {
    let mut registry = PanelRegistry::new();
    register_builtin(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_id_is_registered() {
        let registry = builtin();
        for id in [
            ids::FILES,
            ids::OUTLINE,
            ids::VIEWPORT_3D,
            ids::TEXTURE_PREVIEW,
            ids::GALLERY,
            ids::DETAILS,
            ids::TIMELINE,
            ids::TEXT_EDITOR,
            ids::META_EDITOR,
            ids::CLIP_TABLE,
            ids::SEARCH,
            ids::LOG,
            ids::FILE_PREVIEW,
            ids::STATISTICS,
            ids::DOCUMENT,
            ids::HISTORY,
            ids::MODS,
            ids::PROJECT,
            ids::ASSETS,
            ids::PROBLEMS,
            ids::BUILD,
            ids::JOBS,
            ids::WARDROBE,
            ids::DRESSED,
            ids::WEAPONS,
            ids::WEAPON_PREVIEW,
            ids::MAPS,
            ids::MAP_PLAN,
            ids::MAP_SCENE,
            ids::WORLD_VIEW,
            ids::WORLD,
            ids::EFFECTS,
            ids::COMMAND_LOG,
            ids::PLUGINS,
        ] {
            assert!(
                registry.get(id).is_some(),
                "`{id}` is declared but not registered"
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        assert_eq!(builtin().panels().count(), 34);
    }

    /// ⚠ View ▸ Panels lists every panel, which is a list and not a door: a panel only it leads to is one nobody finds.
    #[test]
    fn every_panel_is_docked_by_a_default_layout_or_shown_by_something_named() {
        use crate::ui::dock::{Workspace, WorkspaceDocks};

        // Each panel no default layout docks, with what shows it.
        const SHOWN_BY: [(PanelId, &str); 5] = [
            (ids::FILE_PREVIEW, "the eye button of Files"),
            (ids::STATISTICS, "Help ▸ Statistics"),
            (ids::COMMAND_LOG, "Automate ▸ Command log"),
            (ids::PLUGINS, "Automate ▸ Run plugin ▸ Plugin settings…"),
            (
                ids::META_EDITOR,
                "the Document panel, which draws it for a metadata file",
            ),
        ];
        let docks = WorkspaceDocks::default();
        let mut docked: Vec<PanelId> = Vec::new();
        for ws in Workspace::ALL {
            let dock = docks.peek(ws).expect("every workspace is built at default");
            docked.extend(dock.iter_all_tabs().map(|(_, tab)| tab.id()));
        }
        for row in crate::ui::task_screens::all() {
            docked.extend((row.layout)().iter_all_tabs().map(|(_, tab)| tab.id()));
        }
        for panel in builtin().panels() {
            let id = panel.id();
            let named = SHOWN_BY.iter().any(|(shown, _)| *shown == id);
            assert!(
                docked.contains(&id) || named,
                "`{id}` is in no default layout and nothing is named as showing it"
            );
            assert!(
                !(docked.contains(&id) && named),
                "`{id}` is docked by a layout: take it off the list"
            );
        }
    }

    #[test]
    fn ids_are_lowercase_and_stable_looking() {
        for panel in builtin().panels() {
            let id = panel.id();
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "`{id}` should be lowercase ascii with underscores"
            );
        }
    }
}
