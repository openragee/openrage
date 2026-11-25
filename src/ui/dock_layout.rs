//! A dock arrangement written down, and what makes one safe to read back.
//!
//! ⚠ **A saved layout is a format, and it will outlive several panel sets.**
//! It is versioned by [`LAYOUT_SCHEMA`] and a layout from another version is
//! discarded whole rather than merged, because a half-restored arrangement is
//! harder to recognise than a default one.
//!
//! ⚠ **An id nothing provides keeps its tab**: dropping the tab loses the
//! user's arrangement of a panel that a plugin, or the next release, puts
//! back. [`crate::ui::panel_registry::MissingPanel`] explains itself in its
//! place instead.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use egui_dock::DockState;
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use crate::ui::dock::{Tab, Workspace, WorkspaceDocks};
use crate::ui::panel_registry;

/// Bumped when the shape below, or the workspaces and panels it arranges,
/// change in a way an older file cannot describe.
pub const LAYOUT_SCHEMA: u32 = 3;

/// The revision of a workspace's default arrangement a saved one must be of: one saved before it changed starts over, and the other workspaces keep theirs.
const REVISIONS: [(Workspace, u32); 1] = [(Workspace::World, 1)];

fn revision(ws: Workspace) -> u32 {
    (REVISIONS.iter())
        .find(|(of, _)| *of == ws)
        .map_or(0, |(_, n)| *n)
}

/// ⚠ **The bound on what a file can make this program allocate.** Every tab a
/// saved layout names that no panel provides is interned for the process's
/// lifetime, so a file is the input that decides how much is leaked. A
/// workspace of this program has ten or so; sixty-four is far past any
/// arrangement a person makes and far short of a problem.
const MAX_TABS_PER_WORKSPACE: usize = 64;

/// ⚠ Long enough that a wrong id in a hand-edited file is legible in the
/// placeholder rather than filling the panel.
const MAX_ID_LEN: usize = 64;

/// How long an arrangement has to stop changing before it is written.
///
/// Dragging a splitter changes the layout on every frame of the drag, and
/// `prefs.json` is rewritten whole each time it is saved.
const SETTLE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedLayouts {
    #[serde(default)]
    pub schema: u32,
    /// The workspace on screen when the arrangement was written.
    #[serde(default)]
    pub active: Workspace,
    #[serde(default)]
    pub workspaces: BTreeMap<Workspace, DockState<String>>,
    /// The arrangement of a workspace that is also a task screen, for a project that holds its content: a file written before it lacks it, and the workspace then starts from the row's layout.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub for_a_project: BTreeMap<Workspace, DockState<String>>,
    /// The revision of each workspace's arrangement when it was written; a file written before it lacks it, which reads as 0.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub revisions: BTreeMap<Workspace, u32>,
}

impl SavedLayouts {
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }
}

/// ⚠ **A layout this cannot read must cost only the layout.** It is the one
/// field in `Prefs` whose shape a third party decides — `egui_dock` can
/// change `DockState` on a minor bump — and the rest of the document is the
/// user's game paths, shortcuts and recent files. Deserialising through a
/// `Value` means a layout serde error is an empty arrangement, not a reset
/// machine.
pub fn readable<'de, D>(deserializer: D) -> Result<SavedLayouts, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match serde_json::from_value::<SavedLayouts>(value) {
        Ok(layouts) => Ok(layouts),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Saved dock layouts could not be read; starting from the default \
                 arrangement. Every other preference is unaffected."
            );
            Ok(SavedLayouts::default())
        }
    }
}

pub fn capture(docks: &WorkspaceDocks, active: Workspace) -> SavedLayouts {
    let mut workspaces = BTreeMap::new();
    let mut for_a_project = BTreeMap::new();
    for ws in Workspace::ALL {
        let has_one = crate::ui::task_screens::of_workspace(ws).is_some_and(|row| row.edits_files);
        if let Some(dock) = docks.arrangement_for_a_project(ws).filter(|_| has_one) {
            let mut described = dock.map_tabs(|tab| tab.id().to_string());
            make_geometry_writable(&mut described);
            for_a_project.insert(ws, described);
        }
        if let Some(dock) = docks.arrangement_without_a_project(ws) {
            let mut described = dock.map_tabs(|tab| tab.id().to_string());
            make_geometry_writable(&mut described);
            workspaces.insert(ws, described);
        }
    }
    SavedLayouts {
        schema: LAYOUT_SCHEMA,
        active,
        workspaces,
        for_a_project,
        revisions: REVISIONS.into_iter().collect(),
    }
}

/// The workspace a start opens in: the one last on screen, unless the
/// layout it was saved with came from another schema.
pub fn restored_workspace(saved: &SavedLayouts) -> Workspace {
    left_on(saved).unwrap_or_default()
}

/// The workspace that was last on screen, when a layout of this schema says so.
pub fn left_on(saved: &SavedLayouts) -> Option<Workspace> {
    (saved.schema == LAYOUT_SCHEMA).then_some(saved.active)
}

/// ⚠ **A `DockState` that has not been laid out yet cannot be written as
/// JSON, and the failure is not local to the layout.** Every node starts at
/// `Rect::NOTHING`, whose corners are infinite; `serde_json` cannot represent
/// an infinity, writes `null`, and then refuses the file on the way back with
/// `invalid type: null, expected f32`. Because the layout lives inside
/// `Prefs`, that one `null` would cost the user every other preference in the
/// document.
///
/// Nothing is lost by flattening them: `egui_dock` recomputes every rect from
/// its split fractions on the first frame.
fn make_geometry_writable(dock: &mut DockState<String>) {
    fn finite(rect: egui::Rect) -> egui::Rect {
        if rect.min.x.is_finite()
            && rect.min.y.is_finite()
            && rect.max.x.is_finite()
            && rect.max.y.is_finite()
        {
            rect
        } else {
            egui::Rect::ZERO
        }
    }

    for (_, node) in dock.iter_all_nodes_mut() {
        match node {
            egui_dock::Node::Leaf(leaf) => {
                leaf.rect = finite(leaf.rect);
                leaf.viewport = finite(leaf.viewport);
                if !leaf.scroll.is_finite() {
                    leaf.scroll = 0.0;
                }
            }
            egui_dock::Node::Vertical(split) | egui_dock::Node::Horizontal(split) => {
                split.rect = finite(split.rect);
                if !split.fraction.is_finite() {
                    split.fraction = 0.5;
                }
            }
            egui_dock::Node::Empty => {}
        }
    }
}

/// Why a saved workspace was not restored, for the log line that says so.
#[derive(Debug, PartialEq, Eq)]
pub enum Rejected {
    Schema { found: u32 },
    TooManyTabs { found: usize },
    UnreadableId,
}

/// Every workspace a saved layout supplies, and the ones it could not.
#[derive(Default)]
pub struct Restored {
    pub usable: Vec<(Workspace, DockState<Tab>)>,
    /// The arrangements for a project that holds the content, of the workspaces that have one.
    pub for_a_project: Vec<(Workspace, DockState<Tab>)>,
    pub rejected: Vec<(String, Rejected)>,
}

pub fn restore(saved: &SavedLayouts) -> Restored {
    // A first start's empty layout reads as schema 0; there is nothing to set aside.
    if saved.is_empty() {
        return Restored::default();
    }
    if saved.schema != LAYOUT_SCHEMA {
        return Restored {
            usable: Vec::new(),
            for_a_project: Vec::new(),
            rejected: vec![(
                tr!("(whole file)").to_string(),
                Rejected::Schema {
                    found: saved.schema,
                },
            )],
        };
    }

    let mut out = Restored::default();
    for (ws, dock) in &saved.workspaces {
        let ids: Vec<&String> = dock.iter_all_tabs().map(|(_, id)| id).collect();
        if ids.len() > MAX_TABS_PER_WORKSPACE {
            out.rejected.push((
                ws.label().to_string(),
                Rejected::TooManyTabs { found: ids.len() },
            ));
            continue;
        }
        if ids
            .iter()
            .any(|id| id.is_empty() || id.len() > MAX_ID_LEN || !is_plausible_id(id))
        {
            out.rejected
                .push((ws.label().to_string(), Rejected::UnreadableId));
            continue;
        }
        // ⚠ A task screen saved before a panel its row now requires, such as the Wardrobe before the dressed ped, has no place for it and the default arrangement has: only that workspace starts over.
        if crate::ui::task_screens::of_workspace_alone(*ws).is_some_and(|screen| {
            (screen.must_dock.iter()).any(|needed| !ids.iter().any(|id| id.as_str() == *needed))
        }) || names_a_panel_twice(&ids)
            || saved.revisions.get(ws).copied().unwrap_or(0) < revision(*ws)
        {
            continue;
        }
        out.usable.push((*ws, tabs_of(dock)));
    }
    for (ws, dock) in &saved.for_a_project {
        let ids: Vec<&String> = dock.iter_all_tabs().map(|(_, id)| id).collect();
        let plausible = ids.len() <= MAX_TABS_PER_WORKSPACE
            && (ids.iter())
                .all(|id| !id.is_empty() && id.len() <= MAX_ID_LEN && is_plausible_id(id));
        // ⚠ Only a task screen that is also the editor of a kind of file has an arrangement for a project, and it must hold the panels its row requires.
        let row = crate::ui::task_screens::of_workspace(*ws).filter(|row| row.edits_files);
        let holds = row.is_some_and(|row| {
            (row.must_dock.iter()).all(|needed| ids.iter().any(|id| id.as_str() == *needed))
        });
        if plausible && holds && !names_a_panel_twice(&ids) {
            out.for_a_project.push((*ws, tabs_of(dock)));
        }
    }
    out
}

/// The arrangement a file holds as tabs of panels, without the tabs of a panel that is gone.
fn tabs_of(dock: &DockState<String>) -> DockState<Tab> {
    dock.filter_map_tabs(|id| {
        (!panel_registry::retired(id)).then(|| Tab(panel_registry::intern(id)))
    })
}
/// ⚠ Two tabs a file names can be one panel now, as an Inspector docked beside a Garment are both Details: one workspace cannot hold a panel twice, so it starts over.
fn names_a_panel_twice(ids: &[&String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    !(ids.iter()).all(|id| seen.insert(panel_registry::current(id)))
}

/// The shape every registered id has, so a file cannot name something that is
/// not an id at all.
fn is_plausible_id(id: &str) -> bool {
    id.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// What was last written, and what is waiting to settle.
///
/// Lives in the panel store rather than on `AppState`: this is bookkeeping for
/// one surface, not application state (`no_panel_reserves_a_field_on_app_state`).
#[derive(Default)]
pub struct LayoutPersist {
    seeded: bool,
    written: Option<String>,
    pending: Option<(String, Instant)>,
}

impl crate::ui::panel_state::PanelState for LayoutPersist {
    const KEY: &'static str = "dock-layout-persist";
}

impl LayoutPersist {
    /// ⚠ Seeded from what was loaded, so a startup that changes nothing does
    /// not rewrite `prefs.json` on its first idle second.
    fn seed_from(&mut self, loaded: &SavedLayouts) {
        if self.seeded {
            return;
        }
        self.seeded = true;
        if !loaded.is_empty() {
            self.written = serde_json::to_string(loaded).ok();
        }
    }

    /// `None` when there is nothing new to write.
    fn what_to_write(&mut self, current: String, now: Option<Instant>) -> Option<String> {
        if self.written.as_deref() == Some(current.as_str()) {
            self.pending = None;
            return None;
        }
        let Some(now) = now else {
            return Some(current);
        };
        match &self.pending {
            Some((seen, since)) if *seen == current && now.duration_since(*since) >= SETTLE => {
                Some(current)
            }
            Some((seen, _)) if *seen == current => None,
            _ => {
                self.pending = Some((current, now));
                None
            }
        }
    }
}

/// Write the arrangement once it has stopped changing.
pub fn tick(state: &mut AppState) {
    write_if_changed(state, Some(Instant::now()));
}

/// Write it now, whatever the clock says.
pub fn flush(state: &mut AppState) {
    write_if_changed(state, None);
}

fn write_if_changed(state: &mut AppState, now: Option<Instant>) {
    let captured = capture(&state.workspace_docks, state.active_workspace);
    let current = match serde_json::to_string(&captured) {
        Ok(json) => json,
        Err(e) => {
            tracing::warn!(error = %e, "Could not describe the dock layout; not saving it");
            return;
        }
    };

    let mut persist = state.panels.take::<LayoutPersist>();
    persist.seed_from(&state.prefs.dock_layouts);
    let to_write = persist.what_to_write(current, now);
    if let Some(written) = to_write {
        state.prefs.dock_layouts = captured;
        crate::state::save_prefs(&state.prefs);
        persist.written = Some(written);
        persist.pending = None;
    }
    state.panels.put(persist);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_with(ids: &[&str]) -> SavedLayouts {
        let dock = DockState::new(ids.iter().map(|s| (*s).to_string()).collect());
        let mut workspaces = BTreeMap::new();
        workspaces.insert(Workspace::Particle, dock);
        SavedLayouts {
            schema: LAYOUT_SCHEMA,
            active: Workspace::default(),
            workspaces,
            for_a_project: BTreeMap::new(),
            revisions: BTreeMap::new(),
        }
    }

    fn tab_ids(dock: &DockState<Tab>) -> Vec<&'static str> {
        dock.iter_all_tabs().map(|(_, tab)| tab.id()).collect()
    }

    #[test]
    fn what_was_arranged_is_what_comes_back() {
        let mut docks = WorkspaceDocks::default();
        docks.get_mut(Workspace::Model);
        let before = capture(&docks, Workspace::default());

        let restored = restore(&before);
        assert!(restored.rejected.is_empty(), "{:?}", restored.rejected);
        let after: BTreeMap<Workspace, DockState<Tab>> = restored.usable.into_iter().collect();

        for ws in Workspace::ALL {
            let original = docks.peek(ws).expect("every workspace is built at default");
            let round_tripped = after.get(&ws).expect("every workspace should come back");
            assert_eq!(
                tab_ids(original),
                tab_ids(round_tripped),
                "{ws:?} lost or reordered its tabs"
            );
        }
    }

    #[test]
    fn a_layout_naming_a_panel_that_is_gone_comes_back_without_its_tab() {
        let restored = restore(&saved_with(&["viewport3d", "preview_settings", "files"]));
        let (_, dock) = restored.usable.first().expect("the workspace is kept");
        assert_eq!(tab_ids(dock), vec!["viewport3d", "files"]);
    }

    #[test]
    fn a_layout_naming_a_panel_details_replaced_comes_back_with_details_and_one_naming_two_starts_over(
    ) {
        let restored = restore(&saved_with(&["viewport3d", "inspector"]));
        let (_, dock) = restored.usable.first().expect("the workspace is kept");
        assert_eq!(tab_ids(dock), vec!["viewport3d", "details"]);

        let restored = restore(&saved_with(&["viewport3d", "inspector", "garment"]));
        assert!(
            restored.usable.is_empty(),
            "one workspace held Details twice"
        );
    }

    /// A Wardrobe saved before the dressed ped had a panel has no place for it: that workspace starts over and the others keep theirs.
    #[test]
    fn a_wardrobe_saved_before_the_dressed_ped_starts_over_and_the_other_workspaces_keep_theirs() {
        let mut docks = WorkspaceDocks::default();
        docks.get_mut(Workspace::Model);
        docks.get_mut(Workspace::Wardrobe);
        let mut saved = capture(&docks, Workspace::Wardrobe);
        // The arrangement an older build wrote: the same tabs but the one that draws the ped.
        let old = DockState::new(vec!["wardrobe".to_string(), "garment".to_string()]);
        saved.workspaces.insert(Workspace::Wardrobe, old);

        let restored = restore(&saved);
        let kept: Vec<Workspace> = restored.usable.iter().map(|(ws, _)| *ws).collect();
        assert!(
            !kept.contains(&Workspace::Wardrobe),
            "the old Wardrobe is not read back: {kept:?}"
        );
        assert!(
            kept.contains(&Workspace::Model),
            "another workspace keeps its arrangement"
        );
        assert!(restored.rejected.is_empty(), "{:?}", restored.rejected);

        // What this build writes of the Wardrobe has the panel, so it is read back as it is.
        let current = capture(&docks, Workspace::Wardrobe);
        let again: Vec<Workspace> = restore(&current).usable.iter().map(|(ws, _)| *ws).collect();
        assert!(again.contains(&Workspace::Wardrobe));
    }

    /// A World saved before its arrangement changed, with the jobs under the view, starts over once; what this build writes of it is read back.
    #[test]
    fn a_world_saved_before_its_arrangement_changed_starts_over_and_the_other_workspaces_keep_theirs(
    ) {
        let mut docks = WorkspaceDocks::default();
        docks.get_mut(Workspace::Model);
        docks.get_mut(Workspace::World);
        let current = capture(&docks, Workspace::World);
        let kept = |saved: &SavedLayouts| -> Vec<Workspace> {
            restore(saved).usable.iter().map(|(ws, _)| *ws).collect()
        };
        assert!(kept(&current).contains(&Workspace::World));
        let mut old = current.clone();
        old.revisions.clear();
        let after = kept(&old);
        assert!(!after.contains(&Workspace::World), "{after:?}");
        assert!(after.contains(&Workspace::Model), "{after:?}");
        let written = serde_json::to_string(&current).expect("a layout is written");
        let read: SavedLayouts = serde_json::from_str(&written).expect("and read back");
        assert!(kept(&read).contains(&Workspace::World));
    }

    #[test]
    fn an_id_nothing_provides_keeps_its_tab() {
        let saved = saved_with(&["files", "a-panel-from-a-plugin"]);
        let restored = restore(&saved);
        assert!(restored.rejected.is_empty(), "{:?}", restored.rejected);

        let (_, dock) = &restored.usable[0];
        assert_eq!(tab_ids(dock), vec!["files", "a-panel-from-a-plugin"]);
        assert!(
            panel_registry::get("a-panel-from-a-plugin").is_none(),
            "the point of the row above is that nothing provides it"
        );
        assert_eq!(
            panel_registry::title_of("a-panel-from-a-plugin", &AppState::default()),
            "a-panel-from-a-plugin?"
        );
    }

    #[test]
    fn interning_the_same_unknown_id_twice_yields_one_pointer() {
        let a = panel_registry::intern("interned-once");
        let b = panel_registry::intern("interned-once");
        assert!(std::ptr::eq(a, b), "a file should not leak per load");
    }

    #[test]
    fn a_layout_from_another_version_is_discarded_whole() {
        let mut saved = saved_with(&["hierarchy"]);
        saved.schema = LAYOUT_SCHEMA + 1;
        let restored = restore(&saved);
        assert!(restored.usable.is_empty());
        assert_eq!(
            restored.rejected,
            vec![(
                "(whole file)".to_string(),
                Rejected::Schema {
                    found: LAYOUT_SCHEMA + 1
                }
            )]
        );
    }

    /// ⚠ A first start's preferences hold an empty layout, whose schema reads as 0.
    #[test]
    fn a_first_start_has_no_saved_layout_and_sets_none_aside() {
        let restored = restore(&SavedLayouts::default());
        assert!(restored.usable.is_empty());
        assert!(
            restored.rejected.is_empty(),
            "a first start warned that a layout it never had was set aside: {:?}",
            restored.rejected
        );
    }

    #[test]
    fn a_file_cannot_ask_for_an_unbounded_number_of_names() {
        let many: Vec<String> = (0..MAX_TABS_PER_WORKSPACE + 1)
            .map(|i| format!("panel-{i}"))
            .collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let restored = restore(&saved_with(&refs));
        assert!(restored.usable.is_empty());
        assert_eq!(
            restored.rejected,
            vec![(
                "Particle".to_string(),
                Rejected::TooManyTabs {
                    found: MAX_TABS_PER_WORKSPACE + 1
                }
            )]
        );
    }

    #[test]
    fn a_name_that_is_not_an_id_is_refused_rather_than_interned() {
        for bad in ["", "Hierarchy", "hier archy", &"h".repeat(MAX_ID_LEN + 1)] {
            let restored = restore(&saved_with(&[bad]));
            assert!(restored.usable.is_empty(), "`{bad}` should not restore");
            assert_eq!(restored.rejected.len(), 1, "`{bad}`");
        }
    }

    #[test]
    fn a_layout_is_written_only_once_it_has_stopped_moving() {
        let start = Instant::now();
        let mut persist = LayoutPersist::default();
        persist.seed_from(&SavedLayouts::default());

        assert_eq!(
            persist.what_to_write("a".to_string(), Some(start)),
            None,
            "the first sighting only starts the clock"
        );
        assert_eq!(
            persist.what_to_write("a".to_string(), Some(start + SETTLE / 2)),
            None,
            "still moving as far as this knows"
        );
        assert_eq!(
            persist.what_to_write("b".to_string(), Some(start + SETTLE)),
            None,
            "a different arrangement restarts the clock rather than inheriting it"
        );
        assert_eq!(
            persist.what_to_write("b".to_string(), Some(start + SETTLE * 2)),
            Some("b".to_string())
        );

        persist.written = Some("b".to_string());
        assert_eq!(
            persist.what_to_write("b".to_string(), Some(start + SETTLE * 3)),
            None,
            "what is already on disk is not written again"
        );
        assert_eq!(
            persist.what_to_write("c".to_string(), None),
            Some("c".to_string()),
            "a flush does not wait"
        );
    }

    /// Through a real file and `load_prefs_from`, inside the whole `Prefs`
    /// document: every other test here checks a value in memory.
    #[test]
    fn an_arrangement_survives_the_file_it_is_written_to() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("prefs.json");

        let mut docks = WorkspaceDocks::default();
        // An arrangement no default produces: the search panel is registered
        // and in no default layout, so finding it on the way back could only
        // have come from the file.
        docks
            .get_mut(Workspace::Model)
            .main_surface_mut()
            .push_to_focused_leaf(Tab(crate::ui::panels::registry::ids::SEARCH));

        let prefs = crate::state::Prefs {
            dock_layouts: capture(&docks, Workspace::default()),
            ..Default::default()
        };
        crate::state::save_prefs_to(&path, &prefs);

        let read = crate::state::load_prefs_from(&path);
        assert!(
            !read.dock_layouts.is_empty(),
            "the whole preferences document failed to parse, so nothing below \
             is about layouts: {:?}",
            serde_json::from_slice::<crate::state::Prefs>(
                &std::fs::read(&path).expect("written above")
            )
            .err()
        );
        let outcome = restore(&read.dock_layouts);
        assert!(outcome.rejected.is_empty(), "{:?}", outcome.rejected);
        let restored = WorkspaceDocks::restored(&read.dock_layouts);

        for ws in Workspace::ALL {
            assert_eq!(
                tab_ids(docks.peek(ws).expect("built at default")),
                tab_ids(restored.peek(ws).expect("should come back")),
                "{ws:?} did not survive the file"
            );
        }
        assert!(
            tab_ids(restored.peek(Workspace::Model).expect("present"))
                .contains(&crate::ui::panels::registry::ids::SEARCH),
            "the one tab no default layout provides is the one that proves \
             this came from the file"
        );
    }

    /// See `make_geometry_writable`.
    #[test]
    fn a_layout_captured_before_the_first_frame_is_still_writable() {
        let docks = WorkspaceDocks::default();
        let json = serde_json::to_string(&capture(&docks, Workspace::default()))
            .expect("captures serialise");

        // ⚠ The message stays short on purpose: this document is 12 kB and
        // printing it buried the one line that said what was wrong.
        for null_number in [
            "\"x\":null",
            "\"y\":null",
            "\"fraction\":null",
            "\"scroll\":null",
        ] {
            assert!(
                !json.contains(null_number),
                "`{null_number}` is an infinity serde_json could not write, and \
                 the whole document is refused on the way back"
            );
        }
        serde_json::from_str::<SavedLayouts>(&json).expect("what was written has to read back");
    }

    #[test]
    fn a_layout_this_cannot_read_costs_only_the_layout() {
        let mut prefs = crate::state::Prefs::default();
        prefs.touch_recent_file(
            crate::state::explorer::Place::new(std::path::Path::new("keep-me.ypt"), "")
                .expect("a path"),
        );
        let mut document = serde_json::to_value(&prefs).expect("prefs serialise");
        document["dock_layouts"] = serde_json::json!({
            "schema": LAYOUT_SCHEMA,
            "workspaces": { "Particle": "this is not a DockState at all" }
        });

        let read: crate::state::Prefs =
            serde_json::from_value(document).expect("the document still parses");
        assert!(
            read.dock_layouts.is_empty(),
            "an unreadable layout should be no layout"
        );
        assert_eq!(
            read.recent_files[0].source, "keep-me.ypt",
            "everything else in the document has to survive it"
        );
    }

    #[test]
    fn a_startup_that_changes_nothing_writes_nothing() {
        let docks = WorkspaceDocks::default();
        let loaded = capture(&docks, Workspace::default());

        let mut persist = LayoutPersist::default();
        persist.seed_from(&loaded);

        let current = serde_json::to_string(&capture(&docks, Workspace::default()))
            .expect("captures serialise");
        assert_eq!(persist.what_to_write(current, None), None);
    }

    /// Through a real file, as `an_arrangement_survives_the_file_it_is_written_to` is.
    #[test]
    fn the_workspace_on_screen_survives_a_save_and_reload_of_the_preferences() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("prefs.json");
        for ws in Workspace::ALL {
            let prefs = crate::state::Prefs {
                dock_layouts: capture(&WorkspaceDocks::default(), ws),
                ..Default::default()
            };
            crate::state::save_prefs_to(&path, &prefs);
            let read = crate::state::load_prefs_from(&path);
            assert_eq!(restored_workspace(&read.dock_layouts), ws);
        }
    }

    /// Explorer, Geometry, Text and Meta: what a layout an older build wrote names.
    #[test]
    fn a_layout_naming_workspaces_that_are_gone_reads_and_falls_back_to_the_defaults_once() {
        let mut prefs = crate::state::Prefs::default();
        prefs.touch_recent_file(
            crate::state::explorer::Place::new(std::path::Path::new("keep-me.ypt"), "")
                .expect("a path"),
        );
        let mut document = serde_json::to_value(&prefs).expect("prefs serialise");
        let defaults = WorkspaceDocks::default();
        let arranged = serde_json::to_value(
            &capture(&defaults, Workspace::Particle).workspaces[&Workspace::Particle],
        )
        .expect("a layout serialises");
        document["dock_layouts"] = serde_json::json!({
            "schema": 1,
            "workspaces": {
                "Particle": arranged,
                "Explorer": arranged,
                "Texture": arranged,
                "Geometry": arranged,
                "Text": arranged,
                "Meta": arranged,
                "Animation": arranged,
            }
        });

        let read: crate::state::Prefs =
            serde_json::from_value(document).expect("the document still parses");
        assert_eq!(
            read.dock_layouts.schema, 1,
            "the old layout should read, and be set aside for its schema rather than as unreadable"
        );
        assert_eq!(read.recent_files[0].source, "keep-me.ypt");

        let docks = WorkspaceDocks::restored(&read.dock_layouts);
        for ws in Workspace::ALL {
            assert_eq!(
                tab_ids(docks.peek(ws).expect("restored")),
                tab_ids(defaults.peek(ws).expect("built at default")),
                "{ws:?} should fall back to its default"
            );
        }
        assert_eq!(restored_workspace(&read.dock_layouts), Workspace::Browse);

        let written = capture(&docks, Workspace::Data);
        assert!(
            restore(&written).rejected.is_empty(),
            "what this build writes back restores, so the fallback happens once"
        );
        assert_eq!(restored_workspace(&written), Workspace::Data);
    }

    /// What a layout written before Files and the Outline took the Hierarchy's place names.
    #[test]
    fn an_old_layout_naming_the_hierarchy_reads_and_falls_back_to_the_defaults_once() {
        let mut prefs = crate::state::Prefs::default();
        prefs.touch_recent_file(
            crate::state::explorer::Place::new(std::path::Path::new("keep-me.ypt"), "")
                .expect("a path"),
        );
        let mut document = serde_json::to_value(&prefs).expect("prefs serialise");
        let mut old = DockState::new(vec!["document".to_string()]);
        let [_, left] = old.main_surface_mut().split_left(
            egui_dock::NodeIndex::root(),
            0.22,
            vec!["hierarchy".to_string()],
        );
        old.main_surface_mut()
            .split_below(left, 0.55, vec!["inspector".to_string()]);
        make_geometry_writable(&mut old);
        let old = serde_json::to_value(&old).expect("a layout serialises");
        document["dock_layouts"] = serde_json::json!({
            "schema": 2,
            "active": "Model",
            "workspaces": { "Browse": old, "Model": old, "Data": old }
        });

        let read: crate::state::Prefs =
            serde_json::from_value(document).expect("the document still parses");
        assert_eq!(
            read.dock_layouts.schema, 2,
            "the old layout should read, and be set aside for its schema rather than as unreadable"
        );
        assert_eq!(read.recent_files[0].source, "keep-me.ypt");

        let defaults = WorkspaceDocks::default();
        let docks = WorkspaceDocks::restored(&read.dock_layouts);
        for ws in Workspace::ALL {
            let tabs = tab_ids(docks.peek(ws).expect("restored"));
            assert!(!tabs.contains(&"hierarchy"), "{ws:?} kept the Hierarchy");
            assert_eq!(
                tabs,
                tab_ids(defaults.peek(ws).expect("built at default")),
                "{ws:?} should fall back to its default"
            );
        }
        assert_eq!(restored_workspace(&read.dock_layouts), Workspace::Browse);

        let written = capture(&docks, Workspace::Model);
        assert!(
            restore(&written).rejected.is_empty(),
            "what this build writes back restores, so the fallback happens once"
        );
        assert_eq!(restored_workspace(&written), Workspace::Model);
    }
}
