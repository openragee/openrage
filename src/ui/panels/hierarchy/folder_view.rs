//! One folder of the Files view in the centre: its Details, a row per entry
//! with its columns, or its Icons, a tile per entry with a picture of it.

use egui_extras::{Column, TableBuilder};

use super::files_view::{go_into, view_ui};
use super::pictures::{picture, Shown};
use super::rows::{
    finish_rows, leave_folder_shown, maybe_drop_into_folder, note_row, select_row,
    selection_modifier_click, start_drag,
};
use super::rpf::{classify_rename_response, finish_rename, rpf_context_menu, rpf_path_supported};
use super::*;
use crate::render::RenderBridge;
use crate::state::explorer::{GroupKey, ViewMode};
use crate::state::files_view::{row_facts, RowFacts};
use crate::ui::commands::{self, Action, Command, Target};
use crate::ui::panels::registry::ids;

/// A tile's picture, in points.
const THUMB: f32 = 96.0;
/// A tile: its picture over up to three lines of name.
const TILE: egui::Vec2 = egui::vec2(116.0, 146.0);
const GAP: f32 = 6.0;
const HEADING: f32 = 24.0;
const ROW: f32 = 20.0;
/// The name's font in a tile, and how many lines it wraps to before it is cut.
const TILE_FONT: f32 = 11.5;
const TILE_LINES: usize = 3;
/// ⚠ No dragging to scroll: a drag moves a row, and a scroll area's drag sense
/// over the empty space would swallow its right click.
const NO_DRAG: egui::scroll_area::ScrollSource = egui::scroll_area::ScrollSource {
    drag: false,
    ..egui::scroll_area::ScrollSource::ALL
};

/// One line of a folder: a group's heading, or an entry.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Line {
    Heading(String),
    Entry(RpfNodeId),
}

/// What one frame of the Files view lists.
pub(super) struct Listing {
    pub doc: DocumentId,
    pub folder: RpfNodeId,
    pub parent: Option<RpfNodeId>,
    pub lines: Vec<Line>,
    /// The entries alone, in the order shown: what a range, an arrow and Ctrl+A walk.
    pub order: Vec<RpfNodeId>,
    /// The folder is one on disk, which Refresh reads again.
    pub on_disk: bool,
}

impl Listing {
    pub(super) fn of(state: &AppState, doc: DocumentId, folder: RpfNodeId) -> Option<Self> {
        let DocPayload::Rpf(rpf) = &state.document(doc)?.payload else {
            return None;
        };
        let node = rpf.session.node(folder)?;
        let groups = explorer::grouped_children(
            &rpf.session,
            folder,
            state.prefs.explorer.sort,
            state.prefs.explorer.group,
        );
        let grouped = state.prefs.explorer.group != GroupKey::None;
        let mut lines = Vec::new();
        let mut order = Vec::new();
        for group in groups {
            if grouped && !group.rows.is_empty() {
                lines.push(Line::Heading(format!(
                    "{} ({})",
                    group.label,
                    group.rows.len()
                )));
            }
            lines.extend(group.rows.iter().map(|&id| Line::Entry(id)));
            order.extend(group.rows);
        }
        Some(Self {
            doc,
            folder,
            parent: node.parent,
            lines,
            order,
            on_disk: matches!(node.backing, RpfNodeBacking::DiskFolder { .. }),
        })
    }
}

/// How many tiles fit across `width`.
pub(super) fn tiles_per_row(width: f32) -> usize {
    ((width + GAP) / (TILE.x + GAP)).floor().max(1.0) as usize
}

/// The folder's rows, as Details or as Icons.
pub(super) fn draw_listing(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bridge: Option<&mut RenderBridge>,
    listing: &Listing,
) {
    let doc = listing.doc;
    let view = state.prefs.explorer.view;
    let per_row = match view {
        ViewMode::Icons => tiles_per_row(ui.available_width()),
        ViewMode::Details => 1,
    };
    if ui.ui_contains_pointer() && !ui.ctx().egui_wants_keyboard_input() {
        super::files_keys::handle(ui, state, listing, per_row);
    }

    // ⚠ First, so every row drawn over it takes the pointer before it does.
    let background = crate::ui::accessible_name::named(
        ui.interact(
            ui.max_rect(),
            ui.id().with(("files_view_background", doc)),
            egui::Sense::click(),
        ),
        tr!("Folder contents"),
    );
    note_row(state, doc, listing.folder, ui.max_rect());
    super::ui_state(state)
        .visible_rows
        .insert(doc, listing.order.clone());

    let mut asked: Option<Command> = None;
    if listing.order.is_empty() {
        let busy = state.expander.is_in_flight(doc, listing.folder);
        // An archive that did not open is not an empty folder: it says why it did not.
        let refused = match state.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => {
                rpf.session
                    .node(listing.folder)
                    .and_then(|node| match &node.backing {
                        RpfNodeBacking::Archive { open_error, .. }
                        | RpfNodeBacking::Rpf9Archive { open_error, .. } => open_error.clone(),
                        _ => None,
                    })
            }
            _ => None,
        };
        let message = if busy {
            tr!("Reading this folder…")
        } else if let Some(error) = refused.as_ref() {
            error
        } else {
            tr!("This folder is empty")
        };
        empty_centered(ui, message);
    } else {
        match view {
            ViewMode::Details => details(ui, state, listing, &mut asked),
            ViewMode::Icons => icons(ui, state, bridge, listing, per_row, &mut asked),
        }
    }
    finish_rows(state, doc);
    view_ui(state).scroll_to_focus = false;

    if background.clicked() {
        state.selection.select(doc, listing.folder);
    }
    super::files_menu::background_menu(&background, state, doc, listing.folder, &mut asked);

    if let Some(command) = asked {
        commands::run(state, ui.ctx(), &command);
    }
    view_ui(state).listed = listing.order.clone();
}

/// Open a row as a double click does: a folder or an archive is gone into, a file opens.
pub(super) fn open_row(state: &mut AppState, listing: &Listing, node: RpfNodeId) {
    let doc = listing.doc;
    let is_dir = match state.document(doc).map(|d| &d.payload) {
        Some(DocPayload::Rpf(rpf)) => rpf.session.node(node).is_some_and(|n| n.is_dir()),
        _ => return,
    };
    if is_dir {
        go_into(state, doc, listing.folder, node);
    } else {
        state.open_rpf_entry(doc, node);
    }
}

/// What a row is drawn with: its node, its columns, and whether a reader names it.
struct Row {
    node: RpfSessionNode,
    facts: RowFacts,
    supported: bool,
    in_flight: bool,
    unsaved: bool,
}

fn row_of(state: &AppState, doc: DocumentId, id: RpfNodeId) -> Option<Row> {
    let DocPayload::Rpf(rpf) = &state.document(doc)?.payload else {
        return None;
    };
    let node = rpf.session.node(id)?.clone();
    let facts = row_facts(&rpf.session, id)?;
    Some(Row {
        supported: node.is_dir() || node.rpf9_type().is_some() || rpf_path_supported(&node.name),
        in_flight: state.expander.is_in_flight(doc, id),
        unsaved: rpf.session.archive_unsaved(id),
        node,
        facts,
    })
}

/// What a click, a double click, a drag, a drop and a right click on a row do.
fn interact(
    state: &mut AppState,
    doc: DocumentId,
    row: &Row,
    resp: &egui::Response,
    asked: &mut Option<Command>,
) {
    let id = row.node.id;
    note_row(state, doc, id, resp.rect);
    if view_ui(state).scroll_to_focus && state.selection.selected(doc) == Some(id) {
        resp.scroll_to_me(None);
    }
    if resp.double_clicked() {
        *asked = Target::rows(state, doc, vec![id], true)
            .and_then(|row| commands::on(state, Action::OpenSelection, row));
    } else if resp.clicked() {
        leave_folder_shown(state, doc, row.node.parent);
        if !selection_modifier_click(&resp.ctx, state, doc, id) {
            select_row(state, doc, id);
        }
    }
    if resp.drag_started() {
        // The folder shown is selected by going into it and is not a row the drag was meant to carry
        leave_folder_shown(state, doc, row.node.parent);
        start_drag(&resp.ctx, state, doc, id, ids::DOCUMENT);
    }
    if row.node.is_dir() {
        maybe_drop_into_folder(&resp.ctx, state, doc, id, resp.rect);
    }
    rpf_context_menu(resp, state, doc, &row.node, asked, true);
}

/// The name editor in place of `id`'s name while it is renamed here; `true` when drawn.
fn rename_editor(ui: &mut egui::Ui, state: &mut AppState, doc: DocumentId, row: &Row) -> bool {
    let id = row.node.id;
    if !super::ui_state(state).is_renaming_in_centre(doc, id) {
        return false;
    }
    if super::rows::is_rpf9(state, doc, id) {
        super::ui_state(state).renaming = None;
        return false;
    }
    let Some(mut rename) = super::ui_state(state).renaming.take() else {
        return false;
    };
    let resp = inline_rename_text_edit(ui, &mut rename);
    let action = classify_rename_response(ui, &resp);
    finish_rename(state, doc, id, &row.node.name, rename, action);
    true
}

/// ⚠ A selected row is marked by its fill, so its name keeps the text colour:
/// the accent on the selection's blue could not be read.
fn name_colour(row: &Row) -> egui::Color32 {
    if row.supported {
        theme::TEXT_PRIMARY.get()
    } else {
        theme::TEXT_SECONDARY.get()
    }
}

/// The Details columns: the header's title, what it sorts by, and how wide it starts.
const COLUMNS: [(&str, SortKey, f32); 8] = [
    (tr_key!("Name"), SortKey::Name, 0.0),
    (tr_key!("Type"), SortKey::Type, 124.0),
    (tr_key!("Size"), SortKey::Size, 70.0),
    (tr_key!("Stored"), SortKey::StoredSize, 70.0),
    (tr_key!("Compression"), SortKey::Compression, 88.0),
    (tr_key!("Encryption"), SortKey::Encryption, 76.0),
    (tr_key!("Edition"), SortKey::Edition, 66.0),
    (tr_key!("Version"), SortKey::Version, 58.0),
];
/// The Name column's narrowest; past it the table scrolls sideways.
const NAME_WIDTH: f32 = 180.0;

fn sort_header(ui: &mut egui::Ui, state: &mut AppState, title: &str, key: SortKey) {
    let current = state.prefs.explorer.sort;
    let arrow = match (current.key == key, current.descending) {
        (true, false) => " ⏶",
        (true, true) => " ⏷",
        _ => "",
    };
    let text = egui::RichText::new(format!("{title}{arrow}"))
        .strong()
        .color(theme::TEXT_SECONDARY);
    if ui
        .add(egui::Label::new(text).sense(egui::Sense::click()))
        .on_hover_text(tr!("Sort by {key}; again to reverse", key = key.label()))
        .clicked()
    {
        let descending = current.key == key && !current.descending;
        state.set_explorer_sort(SortOrder { key, descending });
    }
}

fn details(
    ui: &mut egui::Ui,
    state: &mut AppState,
    listing: &Listing,
    asked: &mut Option<Command>,
) {
    // ⚠ Sideways too: beside the preview pane and the Inspector, eight
    // columns are wider than the centre leaves them.
    egui::ScrollArea::horizontal()
        .id_salt(("files_view_details_across", listing.doc))
        .auto_shrink([false, false])
        .scroll_source(NO_DRAG)
        .show(ui, |ui| details_table(ui, state, listing, asked));
}

fn details_table(
    ui: &mut egui::Ui,
    state: &mut AppState,
    listing: &Listing,
    asked: &mut Option<Command>,
) {
    let doc = listing.doc;
    let focus_line = view_ui(state)
        .scroll_to_focus
        .then(|| state.selection.selected(doc))
        .flatten()
        .and_then(|focus| listing.lines.iter().position(|l| *l == Line::Entry(focus)));
    let mut table = TableBuilder::new(ui)
        .id_salt(("files_view_details", doc))
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click_and_drag())
        .drag_to_scroll(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .auto_shrink([false, false])
        .column(Column::remainder().at_least(NAME_WIDTH).clip(true));
    for (_, _, width) in &COLUMNS[1..] {
        table = table.column(Column::initial(*width).at_least(40.0).clip(true));
    }
    if let Some(line) = focus_line {
        table = table.scroll_to_row(line, None);
    }
    table
        .header(ROW + 2.0, |mut header| {
            for (title, key, _) in COLUMNS {
                header.col(|ui| sort_header(ui, state, crate::i18n::tr(title), key));
            }
        })
        .body(|body| {
            body.rows(ROW, listing.lines.len(), |mut table_row| {
                let id = match &listing.lines[table_row.index()] {
                    Line::Heading(label) => {
                        table_row.col(|ui| {
                            ui.label(egui::RichText::new(label).small().strong());
                        });
                        return;
                    }
                    Line::Entry(id) => *id,
                };
                let Some(row) = row_of(state, doc, id) else {
                    return;
                };
                table_row.set_selected(state.selection.is_selected(doc, id));
                // ⚠ A cell is a Tab stop, and an empty one has no text to be called by, so each says its column.
                let (_, name_cell) = table_row.col(|ui| {
                    if !rename_editor(ui, state, doc, &row) {
                        let icon = icons::rpf_node_icon(
                            &row.node,
                            row.unsaved,
                            row.in_flight,
                            row.supported,
                            false,
                        );
                        let colour = name_colour(&row);
                        ui.add(
                            egui::Label::new(icons::icon_label(
                                &icon,
                                row.node.display_name(),
                                colour,
                            ))
                            .selectable(false)
                            .truncate(),
                        );
                    }
                });
                crate::ui::accessible_name::named_with(name_cell, || {
                    tr!("Name: {name}", name = row.node.display_name())
                });
                for ((title, _, _), cell) in COLUMNS[1..].iter().zip(detail_cells(&row.facts)) {
                    let (_, response) = table_row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(cell.as_str()).color(theme::TEXT_SECONDARY),
                            )
                            .selectable(false)
                            .truncate(),
                        );
                    });
                    crate::ui::accessible_name::named_with(response, || {
                        let title = crate::i18n::tr(title);
                        if cell.is_empty() {
                            tr!("{title}: empty", title = title)
                        } else {
                            format!("{title}: {cell}")
                        }
                    });
                }
                let resp = table_row.response().on_hover_ui(|ui| hover(ui, &row));
                interact(state, doc, &row, &resp, asked);
            });
        });
}

/// The cells after the name, in [`COLUMNS`]' order.
fn detail_cells(facts: &RowFacts) -> [String; 7] {
    let bytes = |n: Option<u64>| n.map(or_job::human_bytes).unwrap_or_default();
    let text = |t: Option<&str>| t.unwrap_or_default().to_string();
    [
        facts.kind.clone(),
        bytes(facts.size),
        bytes(facts.stored),
        text(facts.compression),
        text(facts.encryption),
        text(facts.edition),
        facts.version.map(|v| v.to_string()).unwrap_or_default(),
    ]
}

/// A row's tooltip, whose words are made only while the pointer rests on the row.
fn hover(ui: &mut egui::Ui, row: &Row) {
    ui.set_max_width(ui.spacing().tooltip_width);
    ui.label(hover_of(row));
}

fn hover_of(row: &Row) -> String {
    let text = match row.facts.size {
        Some(size) if !row.node.is_dir() => format!(
            "{}\n{} · {}",
            row.node.display_name(),
            row.facts.kind,
            or_job::human_bytes(size)
        ),
        _ => format!("{}\n{}", row.node.display_name(), row.facts.kind),
    };
    match row.facts.rpf9_entry {
        Some((index, hash)) => format!("{text}\n#{index} · {hash:016X}"),
        None => text,
    }
}

/// A run of the Icons view: a heading, or a row of tiles.
enum Strip {
    Heading(String),
    Tiles(Vec<RpfNodeId>),
}

impl Strip {
    fn height(&self) -> f32 {
        match self {
            Self::Heading(_) => HEADING,
            Self::Tiles(_) => TILE.y + GAP,
        }
    }
}

fn strips(listing: &Listing, per_row: usize) -> Vec<Strip> {
    let mut out = Vec::new();
    let mut run: Vec<RpfNodeId> = Vec::new();
    for line in &listing.lines {
        match line {
            Line::Heading(label) => {
                out.extend(run.chunks(per_row).map(|c| Strip::Tiles(c.to_vec())));
                run.clear();
                out.push(Strip::Heading(label.clone()));
            }
            Line::Entry(id) => run.push(*id),
        }
    }
    out.extend(run.chunks(per_row).map(|c| Strip::Tiles(c.to_vec())));
    out
}

fn icons(
    ui: &mut egui::Ui,
    state: &mut AppState,
    mut bridge: Option<&mut RenderBridge>,
    listing: &Listing,
    per_row: usize,
    asked: &mut Option<Command>,
) {
    let doc = listing.doc;
    let strips = strips(listing, per_row);
    let focus = state.selection.selected(doc);
    egui::ScrollArea::vertical()
        .id_salt(("files_view_icons", doc))
        .auto_shrink([false, false])
        .scroll_source(NO_DRAG)
        .show_viewport(ui, |ui, viewport| {
            let total: f32 = strips.iter().map(Strip::height).sum();
            ui.set_height(total);
            let origin = ui.max_rect().min;
            let mut y = 0.0;
            for strip in &strips {
                let height = strip.height();
                let shown = y + height >= viewport.min.y && y <= viewport.max.y;
                let top = origin + egui::vec2(0.0, y);
                match strip {
                    Strip::Heading(label) if shown => {
                        ui.painter().text(
                            top + egui::vec2(4.0, HEADING / 2.0),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(12.0),
                            theme::TEXT_SECONDARY.get(),
                        );
                    }
                    Strip::Tiles(ids) => {
                        for (column, &id) in ids.iter().enumerate() {
                            let rect = egui::Rect::from_min_size(
                                top + egui::vec2(column as f32 * (TILE.x + GAP), 0.0),
                                TILE,
                            );
                            if view_ui(state).scroll_to_focus && focus == Some(id) {
                                ui.scroll_to_rect(rect, None);
                            }
                            if shown {
                                tile(ui, state, bridge.as_deref_mut(), doc, id, rect, asked);
                            }
                        }
                    }
                    Strip::Heading(_) => {}
                }
                y += height;
            }
        });
}

fn tile(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bridge: Option<&mut RenderBridge>,
    doc: DocumentId,
    id: RpfNodeId,
    rect: egui::Rect,
    asked: &mut Option<Command>,
) {
    let Some(row) = row_of(state, doc, id) else {
        return;
    };
    let resp = ui
        .interact(
            rect,
            ui.id().with(("files_view_tile", id)),
            egui::Sense::click_and_drag(),
        )
        .on_hover_ui(|ui| hover(ui, &row));
    let selected = state.selection.is_selected(doc, id);
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect(
            rect,
            4.0,
            theme::SELECTION_BG.get(),
            egui::Stroke::new(1.0_f32, theme::ACCENT.get()),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, theme::RADIUS_SMALL, theme::BG_L2.get());
    }

    let thumb = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 6.0 + THUMB / 2.0),
        egui::vec2(THUMB, THUMB),
    );
    let side = (THUMB * ui.ctx().pixels_per_point()).round() as u32;
    match picture(ui.ctx(), state, bridge, doc, id, side) {
        Shown::Ready { id: texture, size } => {
            let scale = (thumb.width() / size.x)
                .min(thumb.height() / size.y)
                .min(1.0);
            let drawn = egui::Rect::from_center_size(thumb.center(), size * scale);
            // ⚠ Behind it, so a dark or see-through texture still shows its shape.
            painter.rect_filled(drawn, 0.0, theme::BG_L3.get());
            painter.image(
                texture,
                drawn,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        shown => {
            let icon =
                icons::rpf_node_icon(&row.node, row.unsaved, row.in_flight, row.supported, false);
            let colour = if shown == Shown::Waiting {
                icon.color.gamma_multiply(0.5)
            } else {
                icon.color
            };
            painter.text(
                thumb.center(),
                egui::Align2::CENTER_CENTER,
                icon.glyph,
                egui::FontId::proportional(52.0),
                colour,
            );
        }
    }

    let name_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 3.0, thumb.bottom() + 4.0),
        egui::pos2(rect.right() - 3.0, rect.bottom() - 2.0),
    );
    let edited = ui
        .scope_builder(egui::UiBuilder::new().max_rect(name_rect), |ui| {
            rename_editor(ui, state, doc, &row)
        })
        .inner;
    if !edited {
        let galley = ui.painter().layout_job(tile_name(
            &row.node.display_name(),
            name_rect.width(),
            name_colour(&row),
        ));
        // ⚠ A centred galley is laid out about its position, so that is the middle.
        painter.galley(
            egui::pos2(name_rect.center().x, name_rect.top()),
            galley,
            theme::TEXT_PRIMARY.get(),
        );
    }
    interact(state, doc, &row, &resp, asked);
}

/// A tile's name, wrapped onto up to [`TILE_LINES`] lines and centred.
///
/// ⚠ Cut only past the last line: a name the game files make long, such as
/// `prop_bench_01a_lod.ydr`, reads whole where one row cut it to "…".
pub(super) fn tile_name(name: &str, width: f32, colour: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple(
        name.to_string(),
        egui::FontId::proportional(TILE_FONT),
        colour,
        width,
    );
    job.wrap.max_rows = TILE_LINES;
    job.wrap.overflow_character = Some('…');
    job.halign = egui::Align::Center;
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::explorer::{GroupKey, SortOrder, ViewMode};

    /// Draws `draw`, returning each labelled widget and where it is when `ctx` has accesskit on.
    fn frame(
        ctx: &egui::Context,
        state: &mut AppState,
        n: usize,
        events: Vec<egui::Event>,
        draw: fn(&mut egui::Ui, &mut AppState),
    ) -> Vec<(String, egui::Pos2)> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 2000.0),
            )),
            time: Some(n as f64 * 0.05),
            events,
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| draw(ui, state));
        let Some(update) = output.platform_output.accesskit_update else {
            return Vec::new();
        };
        update
            .nodes
            .iter()
            .filter_map(|(_, node)| {
                let b = node.bounds()?;
                let centre = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
                Some((node.label()?.to_string(), centre))
            })
            .collect()
    }

    fn files_view(ui: &mut egui::Ui, state: &mut AppState) {
        super::super::draw_files_view(ui, state, None);
    }

    fn files_panel(ui: &mut egui::Ui, state: &mut AppState) {
        super::super::draw(ui, state);
    }

    fn archive(view: ViewMode, group: GroupKey) -> (AppState, DocumentId, RpfNodeId) {
        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Rpf);
        let DocPayload::Rpf(rpf) = &mut state.document_mut(doc).unwrap().payload else {
            panic!("an empty RPF document holds an RPF payload");
        };
        let root = rpf.session.root_id();
        let props = rpf.session.add_folder(root, "props").unwrap();
        rpf.session
            .add_file_copy(props, "inner.ydr", vec![0; 4])
            .unwrap();
        rpf.session
            .add_file_copy(root, "big.ydr", vec![0; 20 * 1024])
            .unwrap();
        rpf.session
            .add_file_copy(root, "a.ytd", vec![0; 10])
            .unwrap();
        state.set_active_document(doc);
        state.prefs.explorer.sort = SortOrder::default();
        state.prefs.explorer.view = view;
        state.prefs.explorer.group = group;
        (state, doc, props)
    }

    /// The names the Files view listed last frame, in order.
    fn listed(state: &mut AppState, doc: DocumentId) -> Vec<String> {
        let ids = view_ui(state).listed.clone();
        let DocPayload::Rpf(rpf) = &state.document(doc).unwrap().payload else {
            unreachable!()
        };
        ids.iter()
            .map(|id| rpf.session.node(*id).unwrap().name.clone())
            .collect()
    }

    #[test]
    fn grouping_by_size_lays_rows_out_bucket_by_bucket() {
        let (mut state, doc, _) = archive(ViewMode::Details, GroupKey::Size);
        state.prefs.explorer.sort.descending = true;
        let ctx = egui::Context::default();
        for n in 0..3 {
            frame(&ctx, &mut state, n, vec![], files_view);
        }
        assert_eq!(listed(&mut state, doc), ["props", "a.ytd", "big.ydr"]);

        state.prefs.explorer.group = GroupKey::None;
        frame(&ctx, &mut state, 3, vec![], files_view);
        assert_eq!(listed(&mut state, doc), ["props", "big.ydr", "a.ytd"]);
    }

    #[test]
    fn going_to_a_file_shows_its_folder_with_it_selected() {
        let (mut state, doc, props) = archive(ViewMode::Icons, GroupKey::None);
        let DocPayload::Rpf(rpf) = &state.document(doc).unwrap().payload else {
            unreachable!()
        };
        let inner = rpf.session.children_of(props)[0];
        let ctx = egui::Context::default();
        frame(&ctx, &mut state, 0, vec![], files_view);
        state.navigate_to(Location { doc, node: inner });
        for n in 1..4 {
            frame(&ctx, &mut state, n, vec![], files_view);
        }
        assert_eq!(listed(&mut state, doc), ["inner.ydr"]);
        assert_eq!(state.selection.selected(doc), Some(inner));
    }

    #[test]
    fn an_entrys_menu_offers_open_and_open_opens_it_under_its_archive() {
        let (mut state, doc, _) = archive(ViewMode::Details, GroupKey::None);
        let DocPayload::Rpf(rpf) = &mut state.document_mut(doc).unwrap().payload else {
            unreachable!()
        };
        let root = rpf.session.root_id();
        let notes = rpf
            .session
            .add_file_copy(root, "notes.txt", b"opened".to_vec())
            .unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        frame(&ctx, &mut state, 0, vec![], files_panel);
        frame(&ctx, &mut state, 1, vec![], files_panel);

        let at = super::super::ui_state(&mut state)
            .row_rects
            .iter()
            .find(|(d, n, _)| *d == doc && *n == notes)
            .map(|(_, _, r)| r.center())
            .expect("the tree drew the row");
        let secondary = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        frame(
            &ctx,
            &mut state,
            2,
            vec![egui::Event::PointerMoved(at), secondary(true)],
            files_panel,
        );
        frame(&ctx, &mut state, 3, vec![secondary(false)], files_panel);
        let menu = frame(&ctx, &mut state, 4, vec![], files_panel);
        assert!(
            !menu.iter().any(|(label, _)| label == "Open in new tab"),
            "the menu offers a tab, and an archive entry gets none"
        );
        let open = menu
            .iter()
            .find(|(label, _)| label == "Open")
            .map(|(_, at)| *at)
            .expect("an entry's menu offers Open");
        let click = |pressed| egui::Event::PointerButton {
            pos: open,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        frame(
            &ctx,
            &mut state,
            5,
            vec![egui::Event::PointerMoved(open), click(true)],
            files_panel,
        );
        frame(&ctx, &mut state, 6, vec![click(false)], files_panel);

        let opened = |state: &AppState| {
            matches!(
                state.document(doc).map(|d| &d.payload),
                Some(DocPayload::Rpf(rpf)) if rpf.entry_documents.contains_key(&notes)
            )
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !opened(&state) && std::time::Instant::now() < deadline {
            state.poll_load_result();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(opened(&state), "Open opened nothing under the archive");
    }
}
