//! The Files view, the centre of Browse: the folder or archive selected in
//! Files, under an address bar with Back, Forward, Up and Refresh, beside a
//! preview pane and over a status line.

use super::folder_view::{draw_listing, Listing};
use super::*;
use crate::render::RenderBridge;
use crate::state::explorer::{GroupKey, ViewMode};
use crate::ui::panels::registry::ids;

/// What the Files view keeps between frames, past the tree's own state.
#[derive(Default)]
pub struct FilesViewUi {
    /// The preview pane is put away.
    pub preview_hidden: bool,
    /// The address bar's text while it is being typed into.
    pub address: Option<String>,
    pub address_focus: bool,
    /// The row whose Properties window is open.
    pub properties: Option<(DocumentId, RpfNodeId)>,
    /// What has been typed to jump to a name, and when its last letter came.
    pub typed: String,
    pub typed_at: f64,
    /// Bring the selected row into view this frame.
    pub scroll_to_focus: bool,
    /// Where the user was last frame, so a move made elsewhere brings its row into view.
    pub seen_at: Option<Location>,
    /// The rows the last frame listed, in order.
    pub listed: Vec<RpfNodeId>,
}

impl crate::ui::panel_state::PanelState for FilesViewUi {
    const KEY: &'static str = "files-view";
}

pub(super) fn view_ui(state: &mut AppState) -> &mut FilesViewUi {
    state.panels.get::<FilesViewUi>()
}

/// The location a folder is gone to by: the document itself for its root, which has no row.
fn location_of(state: &AppState, doc: DocumentId, folder: RpfNodeId) -> Location {
    let root = match state.document(doc).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => rpf.session.root_id(),
        _ => folder,
    };
    if folder == root {
        Location::document(doc)
    } else {
        Location { doc, node: folder }
    }
}

/// Go from `from` into a folder or an archive, as a double click on it does.
///
/// ⚠ **The folder left is recorded first**, as Explorer's history is the
/// folders the user was in: the first step from a freshly opened install
/// would otherwise leave nothing to go Back to.
pub(crate) fn go_into(state: &mut AppState, doc: DocumentId, from: RpfNodeId, into: RpfNodeId) {
    let left = location_of(state, doc, from);
    state.navigation.visit(left);
    let at = location_of(state, doc, into);
    state.navigate_to(at);
}

/// Go to the folder holding this one, with the one left selected, as Explorer does.
pub(super) fn go_up(state: &mut AppState, listing: &Listing) {
    let Some(parent) = listing.parent else {
        return;
    };
    go_into(state, listing.doc, listing.folder, parent);
    state.selection.select(listing.doc, listing.folder);
    view_ui(state).scroll_to_focus = true;
}

/// The Files view of the active install, folder or archive.
pub(crate) fn draw_files_view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    mut bridge: Option<&mut RenderBridge>,
) {
    super::rows::start_row_rects(state, ui.ctx().cumulative_pass_nr());
    super::rows::focus_on_drop(ui, state, ids::DOCUMENT);
    let archive = state.active_document.filter(|&id| {
        matches!(
            state.document(id).map(|d| &d.payload),
            Some(DocPayload::Rpf(_))
        )
    });
    let Some(doc) = archive else {
        empty_centered(
            ui,
            tr!("Open an install, a folder or an archive to browse it"),
        );
        return;
    };
    let kept = ui_state(state).open_folder.get(&doc).copied();
    let Some(folder) = state.files_view_folder(doc, kept) else {
        return;
    };
    ui_state(state).open_folder.insert(doc, folder);
    let unread = match state.document(doc).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => rpf.session.needs_expansion(folder),
        _ => false,
    };
    if unread {
        state.request_expand(doc, folder);
    }
    let at = state.navigation.current();
    if view_ui(state).seen_at != at {
        let view = view_ui(state);
        view.seen_at = at;
        view.scroll_to_focus = true;
        view.address = None;
    }
    let Some(listing) = Listing::of(state, doc, folder) else {
        return;
    };
    if ui.ui_contains_pointer() {
        history_keys(ui, state, &listing);
    }

    let id = ui.id();
    if let Some(offer) = state.adoption_offer() {
        egui::Panel::top(id.with("adopt_banner"))
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
            .show_inside(ui, |ui| {
                crate::ui::panels::adopt_banner::draw(ui, state, &offer)
            });
    }
    egui::Panel::top(id.with("files_view_bar"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 4)))
        .show_inside(ui, |ui| bar(ui, state, &listing));
    super::gta6_install::draw(ui, state, doc);
    egui::Panel::bottom(id.with("files_view_status"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 3)))
        .show_inside(ui, |ui| status_line(ui, state, &listing));
    if !view_ui(state).preview_hidden {
        // ⚠ With no row of the folder selected, the pane describes the folder.
        let target = state
            .preview_target()
            .filter(|(_, node)| listing.order.contains(node))
            .or(Some((doc, folder)));
        egui::Panel::right(id.with("files_view_preview"))
            .resizable(true)
            .default_size(300.0)
            .size_range(200.0..=900.0)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(8)))
            .show_inside(ui, |ui| {
                crate::ui::panels::file_preview::draw_for(ui, state, bridge.as_deref_mut(), target)
            });
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(4, 2)))
        .show_inside(ui, |ui| draw_listing(ui, state, bridge, &listing));
}

/// Back, Forward and Up from the keyboard and the mouse's side buttons, over the view.
///
/// ⚠ The keys wait while a field has focus: Backspace there deletes a letter.
fn history_keys(ui: &egui::Ui, state: &mut AppState, listing: &Listing) {
    use egui::{Key, Modifiers, PointerButton};
    let typing = ui.ctx().egui_wants_keyboard_input();
    let (back, forward, up, address) = ui.ctx().input_mut(|i| {
        let side = |i: &egui::InputState, button| i.pointer.button_pressed(button);
        let (side_back, side_forward) = (
            side(i, PointerButton::Extra1),
            side(i, PointerButton::Extra2),
        );
        let mut key = |modifiers, key| !typing && i.consume_key(modifiers, key);
        (
            side_back || key(Modifiers::ALT, Key::ArrowLeft),
            side_forward || key(Modifiers::ALT, Key::ArrowRight),
            key(Modifiers::ALT, Key::ArrowUp) || key(Modifiers::NONE, Key::Backspace),
            key(Modifiers::COMMAND, Key::L),
        )
    });
    if back {
        state.navigate_back();
    } else if forward {
        state.navigate_forward();
    } else if up {
        go_up(state, listing);
    } else if address {
        start_typing_address(state, listing);
    }
}

fn start_typing_address(state: &mut AppState, listing: &Listing) {
    let text = address_text(state, listing.doc, listing.folder);
    let view = view_ui(state);
    view.address = Some(text);
    view.address_focus = true;
}

/// The path the address bar holds while typed into: the folder on disk, else inside its document's file.
fn address_text(state: &AppState, doc: DocumentId, folder: RpfNodeId) -> String {
    let Some(document) = state.document(doc) else {
        return String::new();
    };
    let DocPayload::Rpf(rpf) = &document.payload else {
        return String::new();
    };
    if let Some(on_disk) = rpf.session.node(folder).and_then(|n| n.backing.fs_path()) {
        return on_disk.display().to_string();
    }
    let inner = rpf.session.path_of(folder).unwrap_or_default();
    match &document.path {
        Some(root) if inner.is_empty() => root.display().to_string(),
        Some(root) => format!("{}\\{}", root.display(), inner.replace('/', "\\")),
        None => format!("/{inner}"),
    }
}

fn bar(ui: &mut egui::Ui, state: &mut AppState, listing: &Listing) {
    ui.horizontal(|ui| {
        let back = ui.add_enabled(state.navigation.can_go_back(), egui::Button::new(ph::ARROW_LEFT));
        let back = crate::ui::accessible_name::named(back, tr!("Back"))
            .on_hover_text(tr!("Back (Alt+Left, or the mouse's back button)"));
        if back.clicked() {
            state.navigate_back();
        }
        let forward = ui.add_enabled(
            state.navigation.can_go_forward(),
            egui::Button::new(ph::ARROW_RIGHT),
        );
        let forward = crate::ui::accessible_name::named(forward, tr!("Forward")).on_hover_text(tr!("Forward (Alt+Right)"));
        if forward.clicked() {
            state.navigate_forward();
        }
        let up = ui.add_enabled(listing.parent.is_some(), egui::Button::new(ph::ARROW_UP));
        let up = crate::ui::accessible_name::named(up, tr!("Up one folder"))
            .on_hover_text(tr!("Up to the folder holding this one (Backspace)"));
        if up.clicked() {
            go_up(state, listing);
        }
        let refresh = ui.add_enabled(listing.on_disk, egui::Button::new(ph::ARROW_CLOCKWISE));
        let refresh = crate::ui::accessible_name::named(refresh, tr!("Refresh"))
            .on_hover_text(tr!("Read this folder from disk again"))
            .on_disabled_hover_text(
                tr!("An archive's contents are read when it opens; Refresh reads a folder on disk again"),
            );
        if refresh.clicked() {
            if let Err(e) = state.refresh_folder(listing.doc, listing.folder) {
                state.show_error_toast(e);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            view_controls(ui, state);
            address_bar(ui, state, listing);
        });
    });
}

/// The preview pane's switch, the grouping, and Details or Icons, laid right to left.
fn view_controls(ui: &mut egui::Ui, state: &mut AppState) {
    let hidden = view_ui(state).preview_hidden;
    let sidebar = crate::ui::accessible_name::named(
        ui.selectable_label(!hidden, ph::SIDEBAR_SIMPLE),
        tr!("Preview pane"),
    );
    if sidebar
        .on_hover_text(tr!("Show or hide the preview pane (Space)"))
        .clicked()
    {
        view_ui(state).preview_hidden = !hidden;
    }
    let group = state.prefs.explorer.group;
    let label = match group {
        GroupKey::None => tr!("{icon} Group", icon = ph::STACK),
        other => format!("{} {}", ph::STACK, other.label()),
    };
    ui.menu_button(label, |ui| {
        for key in GroupKey::ALL {
            if ui.radio(group == key, key.label()).clicked() {
                state.set_explorer_group(key);
                ui.close_kind(egui::UiKind::Menu);
            }
        }
        ui.label(
            egui::RichText::new(tr!("Size groups are Explorer's: Tiny, Small, Medium…"))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    })
    .response
    .on_hover_text(tr!("Group the folder's entries by type or size"));
    let current = state.prefs.explorer.view;
    for (view, glyph, what) in [
        (
            ViewMode::Icons,
            ph::SQUARES_FOUR,
            tr!("a tile per entry, with a picture of it"),
        ),
        (
            ViewMode::Details,
            ph::LIST_BULLETS,
            tr!("a row per entry, with its columns"),
        ),
    ] {
        let mode = crate::ui::accessible_name::named(
            ui.selectable_label(current == view, glyph),
            view.label(),
        );
        if mode
            .on_hover_text(format!("{}: {what}", view.label()))
            .clicked()
        {
            state.set_explorer_view(view);
        }
    }
}

fn address_bar(ui: &mut egui::Ui, state: &mut AppState, listing: &Listing) {
    egui::Frame::new()
        .fill(theme::BG_L1.get())
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER.get()))
        .corner_radius(crate::ui::theme::RADIUS_TINY)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| match view_ui(state).address.take() {
                    Some(text) => typed_address(ui, state, listing, text),
                    None => crumbs(ui, state, listing),
                },
            );
        });
}

fn typed_address(ui: &mut egui::Ui, state: &mut AppState, listing: &Listing, mut text: String) {
    let id = egui::Id::new(("files_view_address", listing.doc));
    let resp = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(&mut view_ui(state).address_focus) {
        resp.request_focus();
        if let Some(mut edit) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            );
            edit.cursor.set_char_range(Some(all));
            edit.store(ui.ctx(), id);
        }
        view_ui(state).address = Some(text);
        return;
    }
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if resp.lost_focus() && enter {
        if let Err(e) = state.go_to_address(listing.doc, &text) {
            state.show_error_toast(e);
            let view = view_ui(state);
            view.address = Some(text);
            view.address_focus = true;
        }
    } else if !(escape || resp.lost_focus()) {
        view_ui(state).address = Some(text);
    }
}

/// The path to the folder, a step a place to go, and past it room to click and type one.
fn crumbs(ui: &mut egui::Ui, state: &mut AppState, listing: &Listing) {
    let steps: Vec<(String, Location)> = match state.document(listing.doc) {
        Some(document) => {
            let mut steps = Vec::new();
            if let DocPayload::Rpf(rpf) = &document.payload {
                let mut at = Some(listing.folder);
                while let Some(node) = at.and_then(|id| rpf.session.node(id)) {
                    if node.parent.is_none() {
                        break;
                    }
                    steps.push((
                        node.name.clone(),
                        Location {
                            doc: listing.doc,
                            node: node.id,
                        },
                    ));
                    at = node.parent;
                }
            }
            steps.push((document.title(), Location::document(listing.doc)));
            steps.reverse();
            steps
        }
        None => Vec::new(),
    };
    let mut go = None;
    ui.spacing_mut().item_spacing.x = 2.0;
    let last = steps.len().saturating_sub(1);
    for (index, (label, to)) in steps.into_iter().enumerate() {
        if index > 0 {
            ui.label(egui::RichText::new("›").color(theme::TEXT_DISABLED));
        }
        let colour = if index == last {
            theme::TEXT_PRIMARY
        } else {
            theme::TEXT_SECONDARY
        };
        if ui
            .add(egui::Button::new(egui::RichText::new(label).color(colour)).frame(false))
            .on_hover_text(tr!("Go here"))
            .clicked()
        {
            go = Some(to);
        }
    }
    let rest = crate::ui::accessible_name::named(
        ui.allocate_response(
            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
            egui::Sense::click(),
        ),
        tr!("Type a path"),
    )
    .on_hover_text(tr!("Type a path to go to (Ctrl+L)"));
    if rest.clicked() {
        start_typing_address(state, listing);
    }
    if let Some(to) = go {
        state.navigate_to(to);
    }
}

/// How many items the folder holds, how many are selected and how much they hold.
fn status_line(ui: &mut egui::Ui, state: &mut AppState, listing: &Listing) {
    let (selected, chosen_bytes, total_bytes) =
        match state.document(listing.doc).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => {
                let size =
                    |id: &RpfNodeId| crate::state::files_view::listed_bytes(&rpf.session, *id);
                let selected: Vec<RpfNodeId> = state
                    .selection
                    .selected_ids(listing.doc)
                    .into_iter()
                    .filter(|id| listing.order.contains(id))
                    .collect();
                let chosen: u64 = selected.iter().map(size).sum();
                let total: u64 = listing.order.iter().map(size).sum();
                (selected.len(), chosen, total)
            }
            _ => (0, 0, 0),
        };
    let count = listing.order.len();
    let mut parts = vec![format!("{count} item{}", if count == 1 { "" } else { "s" })];
    if selected > 0 {
        parts.push(if chosen_bytes > 0 {
            format!("{selected} selected, {}", or_job::human_bytes(chosen_bytes))
        } else {
            format!("{selected} selected")
        });
    }
    parts.push(tr!(
        "{size} in this folder",
        size = or_job::human_bytes(total_bytes)
    ));
    ui.label(
        egui::RichText::new(parts.join("   ·   "))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
}
