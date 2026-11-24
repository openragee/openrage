//! The rail at the window's left edge: the rooms a person goes between, Home, Game, World, Project and Automate, with Help and Settings at its foot.
//!
//! World is a screen and not a side panel: it is on the rail while a GTA VI folder is set, and its button goes to the World workspace.
//!
//! ⚠ A room is the panel in front in the side column of the screen, and no state of its own: a button brings that panel forward where the screen stands, so the tab in front stays in front.

use egui_phosphor::regular as ph;

use crate::state::AppState;
use crate::ui::commands::{self, Action, Command};
use crate::ui::dock::Workspace;
use crate::ui::panels::registry::ids;
use crate::ui::theme;

const SIDE: f32 = 30.0;
const GLYPH: f32 = 18.0;
const BADGE: f32 = 10.0;

/// The rooms of the rail's head, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Room {
    Home,
    Game,
    World,
    Project,
    Automate,
}

impl Room {
    pub const ALL: [Room; 5] = [
        Room::Home,
        Room::Game,
        Room::World,
        Room::Project,
        Room::Automate,
    ];

    fn glyph(self) -> &'static str {
        match self {
            Room::Home => ph::HOUSE,
            Room::Game => ph::GAME_CONTROLLER,
            Room::World => ph::GLOBE_HEMISPHERE_WEST,
            Room::Project => ph::CUBE,
            Room::Automate => ph::LIGHTNING,
        }
    }

    /// The small glyph at the corner of the room's own, which says what the room is of.
    fn badge(self) -> Option<&'static str> {
        (self == Room::World).then_some(ph::GAME_CONTROLLER)
    }

    pub fn label(self) -> &'static str {
        match self {
            Room::Home => tr!("Home"),
            Room::Game => tr!("Game"),
            Room::World => tr!("World"),
            Room::Project => tr!("Project"),
            Room::Automate => tr!("Automate"),
        }
    }

    /// The command a click on the room runs: it brings the room's panel forward in the side column of the screen in front, which stays in front. Home is the start page, and with no project Project is the command that makes one.
    pub fn command(self, state: &AppState) -> Option<Command> {
        match self {
            Room::Home => commands::of(state, Action::StartPage),
            Room::Game => Some(commands::side(
                ids::FILES,
                self.label(),
                tr!(
                    "The files of the game, of a folder or of an archive, beside what is on screen"
                ),
            )),
            // Only while a GTA VI folder is set: with none the room is not on the rail.
            Room::World => commands::workspace(state, Workspace::World),
            Room::Project if state.project_path.is_some() => Some(commands::side(
                ids::PROJECT,
                self.label(),
                tr!("What the project holds and what it builds, beside what is on screen"),
            )),
            Room::Project => commands::of(state, Action::NewProject),
            Room::Automate => commands::of(state, Action::CommandLog),
        }
    }
}

/// The room the window is in: Home while the start page is up, else the room of the panel in front in the side column, else the room the screen in front belongs to.
pub fn room_of(state: &AppState) -> Room {
    let ws = state.active_workspace;
    let nothing_open =
        ws == Workspace::Browse && state.documents.is_empty() && state.project_path.is_none();
    if state.front_door.raised(ws) || nothing_open {
        return Room::Home;
    }
    if ws == Workspace::World {
        return Room::World;
    }
    let side = (state.workspace_docks.peek(ws)).and_then(crate::ui::dock_commands::side_in_front);
    match side {
        Some(ids::PROJECT) => Room::Project,
        Some(ids::COMMAND_LOG | ids::PLUGINS) => Room::Automate,
        Some(ids::FILES | ids::SEARCH | ids::MODS) => Room::Game,
        _ if ws.needs_a_project() => Room::Project,
        _ => Room::Game,
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::left("rail")
        .resizable(false)
        .frame(
            egui::Frame::NONE
                .fill(theme::TAB_STRIP_BG.get())
                .inner_margin(egui::Margin::symmetric(4, 6)),
        )
        .exact_size(SIDE + 8.0)
        .show_inside(ui, |ui| {
            let mut asked = None;
            let here = room_of(state);
            ui.spacing_mut().item_spacing.y = 4.0;
            for room in Room::ALL {
                if let Some(command) = room.command(state) {
                    let on = room == here;
                    if button(ui, room.glyph(), room.badge(), room.label(), on, &command) {
                        asked = Some(command);
                    }
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                let foot = [
                    (Action::OpenSettings, ph::GEAR),
                    (Action::OpenGuide, ph::QUESTION),
                ];
                for (action, glyph) in foot {
                    if let Some(command) = commands::of(state, action) {
                        if button(ui, glyph, None, &command.label, false, &command) {
                            asked = Some(command);
                        }
                    }
                }
            });
            if let Some(command) = asked {
                commands::run(state, ui.ctx(), &command);
            }
        });
}

/// One button of the rail: a glyph, with `badge` small at its corner where it has one, named to a screen reader and a hover by `label`, lit while its room is the one on show.
fn button(
    ui: &mut egui::Ui,
    glyph: &str,
    badge: Option<&str>,
    label: &str,
    on: bool,
    command: &Command,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(SIDE), egui::Sense::click());
    let enabled = command.enabled.is_ok();
    let corner = egui::CornerRadius::same(theme::RADIUS_SMALL);
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, corner, theme::document_chip_fill(true));
        let bar = egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(0.0, 5.0),
            rect.left_bottom() + egui::vec2(2.5, -5.0),
        );
        painter.rect_filled(bar, egui::CornerRadius::same(2), theme::ACCENT.get());
    } else if response.hovered() && enabled {
        painter.rect_filled(rect, corner, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if enabled {
        if on {
            theme::ACCENT_TEXT.get()
        } else {
            theme::document_chip_fg(false)
        }
    } else {
        ui.visuals().weak_text_color()
    };
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(GLYPH),
        color,
    );
    if let Some(badge) = badge {
        painter.text(
            rect.right_bottom() - egui::vec2(1.0, 1.0),
            egui::Align2::RIGHT_BOTTOM,
            badge,
            egui::FontId::proportional(BADGE),
            color,
        );
    }
    // ⚠ Named as a room: Project and Automate are also menus of the bar, and two controls of one name are told apart by nothing a screen reader says.
    let named = tr!("Room: {room}", room = label);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, on, &named)
    });
    // ⚠ The name first: the command's own hover says what it does and never what the glyph is called.
    let response = response.on_hover_text(label);
    commands::hints(response, command).clicked() && enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_room_leads_somewhere_and_the_window_is_in_one() {
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        assert_eq!(room_of(&state), Room::Home, "nothing is open");
        for room in Room::ALL {
            // World needs a GTA VI folder, and with none it is not on the rail.
            let leads = room.command(&state).is_some();
            assert_eq!(leads, room != Room::World, "{room:?}");
        }
        let make = Room::Project.command(&state).expect("a command");
        assert_eq!(make.id, "new_project", "with no project the room makes one");

        let doc = state.new_empty_document(crate::state::DocumentKind::Text);
        state.set_active_document(doc);
        assert_eq!(room_of(&state), Room::Game);

        // A room swaps the side column and leaves the tab in front where it is.
        let ctx = egui::Context::default();
        state.project_path = Some(std::path::PathBuf::from("C:/mods/pack.openrage-proj"));
        let on_screen = state.active_workspace;
        for (room, panel) in [
            (Room::Project, ids::PROJECT),
            (Room::Automate, ids::COMMAND_LOG),
            (Room::Game, ids::FILES),
        ] {
            let command = room.command(&state).expect("a command");
            commands::run(&mut state, &ctx, &command);
            assert_eq!(room_of(&state), room, "{room:?} is not the room on show");
            assert_eq!(
                state.active_workspace, on_screen,
                "{room:?} left the screen"
            );
            assert_eq!(state.active_document, Some(doc), "{room:?} left the tab");
            let dock = state.workspace_docks.peek(on_screen).expect("a dock");
            assert_eq!(
                crate::ui::dock_commands::side_in_front(dock),
                Some(panel),
                "{room:?} did not bring its panel forward"
            );
        }

        // The panel of each room stands in one leaf, the side column, and not in the centre.
        let dock = state.workspace_docks.peek(on_screen).expect("a dock");
        let column = |panel| {
            (dock.iter_all_tabs())
                .find(|(_, tab)| tab.id() == panel)
                .map(|(path, _)| path.node)
        };
        assert_eq!(column(ids::PROJECT), column(ids::FILES));
        assert_eq!(column(ids::COMMAND_LOG), column(ids::FILES));

        state.active_workspace = Workspace::Project;
        assert_eq!(room_of(&state), Room::Project);
    }
}
