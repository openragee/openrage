use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::render::RenderBridge;
use crate::state::AppState;

pub type PanelId = &'static str;

pub struct PanelContext<'a> {
    pub state: &'a mut AppState,
    pub bridge: Option<&'a mut RenderBridge>,
}

pub trait Panel: Send + Sync + 'static {
    fn id(&self) -> PanelId;

    fn title(&self, state: &AppState) -> String;

    fn draw(&self, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>);
}

pub struct MissingPanel {
    pub id: String,
}

impl MissingPanel {
    pub fn draw(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(crate::ui::theme::SPACING_SECTION);
            ui.label(egui::RichText::new(tr!("Panel unavailable")).strong());
            ui.add_space(crate::ui::theme::SPACING_TIGHT);
            ui.label(
                egui::RichText::new(tr!("Nothing provides `{id}`.", id = self.id))
                    .weak()
                    .small(),
            );
            ui.add_space(crate::ui::theme::SPACING_HAIR);
            ui.label(
                egui::RichText::new(tr!(
                    "The saved layout still references it, so it will come back if \
                     whatever provides it is available again."
                ))
                .weak()
                .small(),
            );
        });
    }
}

/// The ids of the five panels Details took the place of, which a saved layout or a plugin may still name.
const BECAME_DETAILS: [&str; 5] = [
    "inspector",
    "properties",
    "garment",
    "weapon",
    "map_selection",
];

/// The ids of panels that are gone and have no panel in their place: a saved layout that names one loses that tab and keeps the rest.
const RETIRED: [&str; 1] = ["preview_settings"];

/// Whether `id` named a panel that is gone, whose tab a saved layout drops.
pub fn retired(id: &str) -> bool {
    RETIRED.contains(&id)
}

/// The id a panel goes by now, for one a file or a plugin names.
pub fn current(id: &str) -> &str {
    if BECAME_DETAILS.contains(&id) {
        crate::ui::panels::registry::ids::DETAILS
    } else {
        id
    }
}

#[derive(Default)]
pub struct PanelRegistry {
    panels: HashMap<PanelId, &'static dyn Panel>,
    order: Vec<PanelId>,
}

impl PanelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, panel: &'static dyn Panel) {
        let id = panel.id();
        if self.panels.insert(id, panel).is_none() {
            self.order.push(id);
        }
    }

    pub fn get(&self, id: PanelId) -> Option<&'static dyn Panel> {
        self.resolve(id)
    }

    /// The same lookup for an id whose lifetime is not `'static` yet, which
    /// is every id that came out of a file.
    pub fn resolve(&self, id: &str) -> Option<&'static dyn Panel> {
        self.panels.get(current(id)).copied()
    }

    pub fn panels(&self) -> impl Iterator<Item = &'static dyn Panel> + '_ {
        self.order.iter().filter_map(|id| self.get(id))
    }
}

fn live() -> &'static RwLock<PanelRegistry> {
    static LIVE: OnceLock<RwLock<PanelRegistry>> = OnceLock::new();
    LIVE.get_or_init(|| RwLock::new(crate::ui::panels::registry::builtin()))
}

// ⚠ Refuses an id already present rather than replacing it, unlike the other
// three registries, so a plugin cannot supersede a builtin panel.
pub fn register(panel: &'static dyn Panel) -> bool {
    let mut registry = live().write().expect("panel registry poisoned");
    if registry.get(panel.id()).is_some() {
        return false;
    }
    registry.register(panel);
    true
}

pub fn get(id: PanelId) -> Option<&'static dyn Panel> {
    live().read().expect("panel registry poisoned").get(id)
}

/// A `PanelId` for a name read from outside the program.
///
/// ⚠ **An id nothing provides still has to become a `PanelId`**, or a saved
/// layout could not carry one and [`MissingPanel`] would never render. A
/// registered id resolves to the registry's own `'static`; anything else is
/// leaked once and reused, so the cost is one allocation per *distinct*
/// unknown name and not one per load. The caller is what bounds how many
/// distinct names can arrive — see `src/ui/dock_layout.rs`.
pub fn intern(id: &str) -> PanelId {
    if let Some(panel) = live().read().expect("panel registry poisoned").resolve(id) {
        return panel.id();
    }

    static LEAKED: OnceLock<RwLock<HashMap<String, PanelId>>> = OnceLock::new();
    let leaked = LEAKED.get_or_init(|| RwLock::new(HashMap::new()));

    if let Some(found) = leaked.read().expect("intern table poisoned").get(id) {
        return found;
    }
    let mut table = leaked.write().expect("intern table poisoned");
    table
        .entry(id.to_string())
        .or_insert_with(|| Box::leak(id.to_string().into_boxed_str()))
}

/// Every registered panel, in registration order.
pub fn all() -> Vec<&'static dyn Panel> {
    live()
        .read()
        .expect("panel registry poisoned")
        .panels()
        .collect()
}

pub fn title_of(id: PanelId, state: &AppState) -> String {
    match get(id) {
        Some(panel) => panel.title(state),
        None => format!("{id}?"),
    }
}

pub fn draw_panel(id: PanelId, ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
    match get(id) {
        Some(panel) => panel.draw(ui, ctx),
        None => MissingPanel { id: id.to_string() }.draw(ui),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(&'static str);

    impl Panel for Stub {
        fn id(&self) -> PanelId {
            self.0
        }
        fn title(&self, _state: &AppState) -> String {
            self.0.to_string()
        }
        fn draw(&self, _ui: &mut egui::Ui, _ctx: &mut PanelContext<'_>) {}
    }

    static A: Stub = Stub("a");
    static B: Stub = Stub("b");
    static A_AGAIN: Stub = Stub("a");

    #[test]
    fn registration_order_is_preserved() {
        let mut registry = PanelRegistry::new();
        registry.register(&A);
        registry.register(&B);

        let ids: Vec<PanelId> = registry.panels().map(|p| p.id()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn re_registering_an_id_replaces_without_duplicating_it() {
        let mut registry = PanelRegistry::new();
        registry.register(&A);
        registry.register(&A_AGAIN);

        assert_eq!(registry.panels().count(), 1);
    }

    #[test]
    fn an_unknown_id_is_reported_rather_than_resolved() {
        let registry = PanelRegistry::new();
        assert!(registry.get("nope").is_none());
    }

    static LATE: Stub = Stub("registered-after-startup");
    static IMPOSTOR: Stub = Stub("files");

    #[test]
    fn a_panel_can_be_registered_after_the_builtins() {
        assert!(get("registered-after-startup").is_none());
        assert!(register(&LATE));
        assert_eq!(
            get("registered-after-startup").map(|p| p.id()),
            Some("registered-after-startup")
        );
    }

    #[test]
    fn registering_over_a_builtin_is_refused() {
        assert!(get("files").is_some(), "the builtins should be live");
        assert!(!register(&IMPOSTOR));
        assert_eq!(get("files").map(|p| p.id()), Some("files"));
    }

    #[test]
    fn an_id_of_a_panel_details_replaced_resolves_to_details() {
        for old in BECAME_DETAILS {
            assert!(
                crate::ui::panels::registry::builtin().get(old).is_some(),
                "`{old}` no longer resolves"
            );
            assert_eq!(intern(old), "details", "`{old}` did not become Details");
        }
        assert_eq!(current("files"), "files");
    }

    #[test]
    fn a_gta_vi_archive_has_no_panel_of_its_own_beside_the_files_tree() {
        let state = AppState::default();
        let titled = (live().read().expect("panel registry poisoned").panels())
            .find(|p| p.id() == "gta6_archive_tree" || p.title(&state) == "Archives")
            .map(|p| p.id());
        assert_eq!(titled, None, "a GTA VI archive is browsed in Files");
    }

    #[test]
    fn an_unregistered_id_still_has_a_title() {
        assert_eq!(
            title_of("no-such-panel", &AppState::default()),
            "no-such-panel?"
        );
    }
}
