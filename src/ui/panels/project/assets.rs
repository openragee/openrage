//! The Assets table: one row for each asset of any content, virtualised, with a filter, a quick look and the keys to move through it.

use egui_extras::{Column, TableBuilder};

use super::{command_button, elided_in_the_middle, open_in_submenu, state_colour, REFRESH};
use crate::state::front_door::counted;
use crate::state::workbench::{content_name, AssetRow, Glimpse, Kept, Want};
use crate::state::AppState;
use crate::ui::commands::{self, Action};
use crate::ui::panel_state::PanelState;
use crate::ui::shortcuts;
use crate::ui::theme;

const ROW_HEIGHT: f32 = 22.0;

/// The height a panel needs for the quick look to dock under the table; a shorter one gives it the whole panel.
const DOCK_ROOM: f32 = 320.0;

/// How tall the picture of the quick look is, and so how tall its strip is.
const LOOK: f32 = 128.0;

/// What the strip of the quick look takes besides its picture: its title row and the margins of the panel.
const STRIP_AROUND: f32 = 34.0;

/// How much of a column must be in view for it not to count as hidden.
const HIDDEN_BELOW: f32 = 24.0;

/// What the filter row says and what it left, kept across frames.
#[derive(Default)]
pub struct AssetsUi {
    pub filter: String,
    pub only_problems: bool,
    /// Only the assets of this content.
    pub content: Option<String>,
    kept: Option<Kept>,
    cache: Option<Cached>,
    /// A row the table scrolls to once, because a key moved the selection.
    scroll_to: Option<usize>,
    /// The columns of the table the last frame cut off at the panel's edge.
    hidden: Vec<&'static str>,
    /// Which of the optional columns the project's rows have something to say in.
    used: Used,
    /// The width of the panel the table was last drawn in.
    table_width: f32,
    /// The quick look's picture as the screen holds it, by the id of the pixels it was made from.
    picture: Option<(u64, egui::TextureHandle)>,
    /// What the quick look draws for the garment it is on, found once for the rows as they are: finding it reads the disk.
    garment_want: Option<(String, u64, Option<Want>)>,
}

impl AssetsUi {
    /// Whether anything narrows the list: the text, a kind of source, Only problems or one content.
    fn is_filtering(&self) -> bool {
        !self.filter.trim().is_empty()
            || self.only_problems
            || self.kept.is_some()
            || self.content.is_some()
    }

    fn clear(&mut self) {
        self.filter.clear();
        self.only_problems = false;
        self.kept = None;
        self.content = None;
    }
}

impl PanelState for AssetsUi {
    const KEY: &'static str = "assets-table";
}

/// The optional columns of the table that say something: one that is empty on every row, or alike on every row, is left out.
#[derive(Clone, Copy, Default)]
struct Used {
    target: bool,
    content: bool,
    kept: bool,
}

impl Used {
    fn of(rows: &[AssetRow]) -> Self {
        let first = rows.first();
        Self {
            target: rows.iter().any(|row| row.target.is_some()),
            content: first.is_some_and(|first| rows.iter().any(|row| row.content != first.content)),
            kept: first.is_some_and(|first| rows.iter().any(|row| row.kept != first.kept)),
        }
    }
}

/// What the rows listed were filtered by, so they are filtered again only when it changes.
#[derive(Clone, PartialEq)]
struct Filtered {
    filter: String,
    only_problems: bool,
    content: Option<String>,
    kept: Option<Kept>,
    generation: u64,
    findings: usize,
}

struct Cached {
    by: Filtered,
    /// Indices into the workbench's rows.
    listed: Vec<usize>,
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    if state.project_path.is_none() {
        crate::ui::panels::empty_state::draw(ui, state);
        return;
    }
    ui.ctx().request_repaint_after(REFRESH);
    let mut shown = state.panels.take::<AssetsUi>();
    let panel = ui.max_rect();
    if let Some(why) = state.read_only_reason() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tr!("Read only: {why}", why = why)).color(theme::WARNING),
            )
            .wrap(),
        );
    }
    controls(ui, state, &mut shown);
    // ⚠ Before the table, so the table takes what the quick look leaves.
    let took_all = quick_look(ui, state, &mut shown);
    let mut drawn = false;

    if took_all {
    } else if !state.workbench.is_scanned() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(egui::RichText::new(tr!("Reading the project…")).color(theme::TEXT_SECONDARY));
        });
    } else if state.workbench.rows().is_empty() {
        nothing_yet(ui, state);
    } else {
        let listed = listed(state, &mut shown);
        if listed.is_empty() {
            no_match(ui, &mut shown);
        } else {
            keys(ui, state, &mut shown, panel, &listed);
            // ⚠ Laid out in the rect the quick look leaves, so no row is sliced and no column rule runs down through the strip under it.
            let left = ui.available_rect_before_wrap();
            ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                table(ui, state, &mut shown, &listed)
            });
            drawn = true;
        }
    }
    // ⚠ What the table's header said is of the table that drew it: with none drawn, no column is hidden.
    if !drawn && !took_all {
        shown.hidden.clear();
    }
    state.panels.put(shown);
}

/// What the table says when the filters leave no row: that none matches, and the way back.
fn no_match(ui: &mut egui::Ui, shown: &mut AssetsUi) {
    ui.vertical_centered(|ui| {
        ui.add_space(theme::SPACING_SECTION);
        ui.add(
            egui::Label::new(
                egui::RichText::new(tr!("No asset matches the filter"))
                    .size(16.0)
                    .color(theme::TEXT_SECONDARY),
            )
            .wrap(),
        );
        ui.add_space(theme::SPACING_DEFAULT);
        if ui.button(tr!("Clear the filter")).clicked() {
            shown.clear();
        }
    });
}

fn nothing_yet(ui: &mut egui::Ui, state: &mut AppState) {
    ui.vertical_centered(|ui| {
        ui.add_space(theme::SPACING_SECTION);
        ui.add(
            egui::Label::new(
                egui::RichText::new(tr!("This project holds no files yet"))
                    .size(16.0)
                    .color(theme::TEXT_SECONDARY),
            )
            .wrap(),
        );
        ui.add_space(theme::SPACING_DEFAULT);
        // ⚠ Each door says what it takes under its button, since two buttons that both bring files in cannot be told apart by their names: one primary action, game files, which Import is for, and a clothing pack.
        landing_button(ui, state, Action::AddContent, true);
        landing_note(ui, tr!("Your own files: images, models, scripts, folders."));
        landing_button(ui, state, Action::ImportGameFiles, false);
        landing_note(
            ui,
            tr!("Files from the game (.ytd, .ydr, .ydd…): turned into sources you can edit. The originals are not changed."),
        );
        for screen in crate::ui::task_screens::all() {
            landing_button(ui, state, screen.add, false);
            landing_note(ui, crate::i18n::tr(screen.landing_note));
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(tr!("You can also drop files or folders here."))
                    .small()
                    .color(theme::TEXT_SECONDARY),
            )
            .wrap(),
        );
    });
}

/// The line under a button of the landing that says what it takes.
fn landing_note(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .small()
                .color(theme::TEXT_SECONDARY),
        )
        .wrap(),
    );
    ui.add_space(theme::SPACING_DEFAULT);
}

/// `action` as a button of the landing, named as its command is named and filled when it is the one next action: its key is in its hint.
fn landing_button(ui: &mut egui::Ui, state: &mut AppState, action: Action, primary: bool) {
    let Some(command) = commands::of(state, action) else {
        return;
    };
    let text = command.label.clone();
    let button = match primary {
        true => egui::Button::new(egui::RichText::new(text).color(theme::ON_ACCENT.get()))
            .fill(theme::ACCENT.get()),
        false => egui::Button::new(text),
    }
    .min_size(egui::vec2(180.0, 26.0));
    let response = ui.add_enabled(command.enabled.is_ok(), button);
    if commands::hints(response, &command).clicked() {
        commands::run(state, ui.ctx(), &command);
    }
}

/// The filter row: a text filter, how the source is kept, Only problems, and how many rows are listed.
fn controls(ui: &mut egui::Ui, state: &mut AppState, shown: &mut AssetsUi) {
    // ⚠ A share of the row and not a width of its own: at the default window the row is a few points from wrapping, and the count it ends on would wrap under the field.
    let filter_width = (ui.available_width() * 0.26).clamp(120.0, 220.0);
    ui.horizontal_wrapped(|ui| {
        let filter = ui.add(
            egui::TextEdit::singleline(&mut shown.filter)
                .hint_text(tr!("Filter assets…"))
                .desired_width(filter_width),
        );
        filter.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, tr!("Filter assets"))
        });
        for (kept, words) in [
            (None, tr!("All")),
            (Some(Kept::File), tr!("Files")),
            (Some(Kept::Source), tr!("Sources")),
        ] {
            if ui.selectable_label(shown.kept == kept, words).clicked() {
                shown.kept = kept;
            }
        }
        ui.checkbox(&mut shown.only_problems, tr!("Only problems"));
        if let Some(content) = shown.content.clone() {
            let named = content_name(&state.project_carried.container, &content);
            if ui
                .button(tr!(
                    "Content: {named} {icon}",
                    named = named,
                    icon = egui_phosphor::regular::X
                ))
                .on_hover_text(tr!("Show every content again"))
                .clicked()
            {
                shown.content = None;
            }
        }
        let on = shown.is_filtering();
        if ui
            .add_enabled(on, egui::Button::new(tr!("Clear filters")))
            .on_hover_text(tr!("List every asset again"))
            .on_disabled_hover_text(tr!("No filter is on"))
            .clicked()
        {
            shown.clear();
        }
        let rows = state.workbench.rows().len();
        // ⚠ Only what the rows now are: the cache of a project that is gone lists rows this one has not.
        let listed = (shown.cache.as_ref())
            .filter(|c| c.by.generation == state.workbench.generation())
            .map_or(rows, |c| c.listed.len());
        ui.label(
            egui::RichText::new(tr!(
                "{listed} of {assets}",
                listed = listed,
                assets = counted(rows, "asset")
            ))
            .small()
            .color(theme::TEXT_SECONDARY),
        );
        if !shown.hidden.is_empty() {
            // ⚠ One unbreakable phrase: the row wraps between phrases and never inside one.
            ui.add(
                egui::Label::new(
                    egui::RichText::new(counted_columns(shown.hidden.len()))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                )
                .wrap_mode(egui::TextWrapMode::Extend),
            )
            .on_hover_text(tr!(
                "{hidden}. Make this panel wider, or maximize it, to see them",
                hidden = shown.hidden.join(", ")
            ));
        }
        if state.workbench.is_truncated() {
            ui.label(
                egui::RichText::new(tr!(
                    "The folder is large: only part of it was searched for sources"
                ))
                .small()
                .color(theme::WARNING),
            );
        }
    });
    ui.separator();
}

/// The indices of the rows the filters keep, filtered again only when what they depend on changed.
fn listed(state: &AppState, shown: &mut AssetsUi) -> Vec<usize> {
    let by = Filtered {
        filter: shown.filter.clone(),
        only_problems: shown.only_problems,
        content: shown.content.clone(),
        kept: shown.kept,
        generation: state.workbench.generation(),
        findings: state.workbench.findings().len(),
    };
    if let Some(cached) = shown.cache.as_ref().filter(|cached| cached.by == by) {
        return cached.listed.clone();
    }
    let needle = by.filter.trim().to_lowercase();
    let wanted = |row: &AssetRow| {
        by.content.as_ref().is_none_or(|c| *c == row.content)
            && by.kept.is_none_or(|kept| kept == row.kept)
            && (!by.only_problems || state.workbench.state_of(row).is_problem())
            && (needle.is_empty() || matches_text(row, &needle))
    };
    let listed: Vec<usize> = (state.workbench.rows().iter().enumerate())
        .filter(|(_, row)| wanted(row))
        .map(|(i, _)| i)
        .collect();
    shown.used = Used::of(state.workbench.rows());
    shown.cache = Some(Cached {
        by,
        listed: listed.clone(),
    });
    listed
}

/// `3 columns hidden`, for a table that is wider than its panel.
fn counted_columns(n: usize) -> String {
    tr!("{columns} hidden", columns = counted(n, "column"))
}

fn matches_text(row: &AssetRow, needle: &str) -> bool {
    [
        &row.name,
        &row.kind,
        &row.content,
        &row.path.display().to_string(),
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(needle))
}

/// Up and Down move the selection, Enter opens it and Space looks at it, while the pointer is over the table and no text is being typed.
fn keys(
    ui: &mut egui::Ui,
    state: &mut AppState,
    shown: &mut AssetsUi,
    panel: egui::Rect,
    listed: &[usize],
) {
    if listed.is_empty() || !ui.rect_contains_pointer(panel) || ui.ctx().egui_wants_keyboard_input()
    {
        return;
    }
    let (down, up, enter, asked) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowDown),
            i.key_pressed(egui::Key::ArrowUp),
            i.key_pressed(egui::Key::Enter),
            i.events.iter().find_map(|event| {
                shortcuts::action_for_event(crate::ui::commands::Place::Lists, event, &state.prefs)
            }),
        )
    });
    let at = state.workbench.selected.as_ref().and_then(|key| {
        listed.iter().position(|&i| {
            state
                .workbench
                .rows()
                .get(i)
                .is_some_and(|row| row.key == *key)
        })
    });
    let moved = match (down, up, at) {
        (true, _, None) | (_, true, None) => Some(0),
        (true, _, Some(at)) => Some((at + 1).min(listed.len() - 1)),
        (_, true, Some(at)) => Some(at.saturating_sub(1)),
        _ => None,
    };
    if let Some(place) = moved {
        let Some(key) = (state.workbench.rows().get(listed[place])).map(|row| row.key.clone())
        else {
            return;
        };
        state.select_asset(Some(key));
        shown.scroll_to = Some(place);
    }
    if enter {
        commands::run_action(state, ui.ctx(), Action::OpenAsset);
    }
    if asked == Some(Action::QuickLook) {
        commands::run_action(state, ui.ctx(), Action::QuickLook);
    }
}

fn table(ui: &mut egui::Ui, state: &mut AppState, shown: &mut AssetsUi, listed: &[usize]) {
    let selected = state.workbench.selected.clone();
    let mut clicked: Option<String> = None;
    let mut opened: Option<String> = None;
    let mut secondary: Option<String> = None;
    // ⚠ A column past the panel's edge is cut off, so the filter row says which, from what this frame's header laid out.
    let (right, mut hidden) = (ui.max_rect().right(), Vec::new());

    // ⚠ The width goes to the name, which tells the rows apart: it takes what the other columns leave, and a column that says nothing on any row is left out.
    let used = shown.used;
    let mut titles = vec![tr!("State"), tr!("Name"), tr!("Kind")];
    let mut columns = vec![
        Column::initial(112.0).at_least(70.0).clip(true),
        Column::remainder().at_least(120.0).clip(true),
        Column::initial(132.0).at_least(80.0).clip(true),
    ];
    for (on, title, width) in [
        (used.target, tr!("Target"), 62.0),
        (used.content, tr!("Content"), 76.0),
        (used.kept, tr!("Stored as"), 128.0),
    ] {
        if on {
            titles.push(title);
            columns.push(Column::initial(width).at_least(50.0).clip(true));
        }
    }
    // ⚠ A table keeps the widths its columns were drawn at, so the name, which takes what the others leave, is laid out again when the panel it is in changes width.
    let room = ui.available_width();
    let resized = (shown.table_width - room).abs() > 1.0;
    shown.table_width = room;
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .min_scrolled_height(0.0)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    if resized {
        builder.reset();
    }
    for column in columns {
        builder = builder.column(column);
    }
    if let Some(place) = shown.scroll_to.take() {
        builder = builder.scroll_to_row(place, None);
    }
    builder
        .header(20.0, |mut header| {
            for &title in &titles {
                let (cell, _) = header.col(|ui| {
                    ui.label(egui::RichText::new(title).color(theme::TEXT_SECONDARY));
                });
                // Less than a few letters of it in view is as good as none.
                if cell.min.x + HIDDEN_BELOW > right {
                    hidden.push(title);
                }
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, listed.len(), |mut row| {
                // ⚠ Never an index the rows no longer have: a scan can land between the filter and the draw.
                let Some(asset) = state.workbench.rows().get(listed[row.index()]) else {
                    return;
                };
                let current = state.workbench.state_of(asset);
                row.set_selected(selected.as_deref() == Some(asset.key.as_str()));
                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(format!("● {}", current.label()))
                            .color(state_colour(current)),
                    );
                });
                row.col(|ui| {
                    // ⚠ Cut in the middle when it does not fit, so the end that tells one garment from the next stays; the whole name is in the hover.
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let width = ui.available_width();
                    let name = elided_in_the_middle(ui, &asset.name, &font, width);
                    let cut = name != asset.name;
                    let label =
                        ui.add(egui::Label::new(name).wrap_mode(egui::TextWrapMode::Extend));
                    if cut {
                        label.on_hover_text(&asset.name);
                    }
                });
                row.col(|ui| {
                    ui.label(&asset.kind);
                });
                if used.target {
                    row.col(|ui| {
                        ui.label(asset.target.unwrap_or_default());
                    });
                }
                if used.content {
                    // The content as the Content panel names it, and not the id the project file keeps it by.
                    row.col(|ui| {
                        ui.label(content_name(
                            &state.project_carried.container,
                            &asset.content,
                        ));
                    });
                }
                if used.kept {
                    row.col(|ui| {
                        ui.label(asset.kept.label());
                    });
                }
                let response = row.response();
                if response.clicked() {
                    clicked = Some(asset.key.clone());
                }
                if response.double_clicked() {
                    opened = Some(asset.key.clone());
                }
                if response.secondary_clicked() {
                    secondary = Some(asset.key.clone());
                }
                response.context_menu(|ui| {
                    crate::ui::popup_of_the_shell(ui.ctx());
                    command_button(ui, state, Action::OpenAsset);
                    open_in_submenu(ui, state);
                    for action in [
                        Action::QuickLook,
                        Action::RevealAsset,
                        Action::RecookAsset,
                        Action::ReplaceGeometry,
                        Action::CheckAgainstGame,
                        Action::RemoveAsset,
                    ] {
                        command_button(ui, state, action);
                    }
                });
            });
        });
    if shown.hidden != hidden {
        shown.hidden = hidden;
        ui.ctx().request_repaint();
    }
    // ⚠ A right click selects its row, as it does in Files, so what the menu offers acts on it.
    if let Some(key) = secondary.or(clicked) {
        state.select_asset(Some(key));
    }
    if let Some(key) = opened {
        state.open_asset(&key, None);
    }
}

/// What the picture of the quick look is, once asked for.
enum Picture {
    None,
    Reading,
    Drawn(std::sync::Arc<crate::state::workbench::Thumb>),
    Cannot(String),
}

/// The selected asset without opening it, docked under the table: the picture of a texture, captioned with its file and size, and the asset's problems.
///
/// ⚠ Inside the table's panel and never a window over the screen, so it cannot cover a button of Properties
/// whatever the window's size. The Space key and its close button take it down.
fn quick_look(ui: &mut egui::Ui, state: &mut AppState, shown: &mut AssetsUi) -> bool {
    if !state.workbench.quick_look {
        return false;
    }
    let Some(row) = state.selected_asset().cloned() else {
        state.workbench.quick_look = false;
        return false;
    };
    let crypto = state.keys.provider();
    // ⚠ A garment's picture is its first texture, which lives in its pack: found once and not on every frame.
    let want = match row.kept {
        Kept::Pack => {
            let held = state.workbench.generation();
            let cached = (shown.garment_want.as_ref())
                .filter(|(key, generation, _)| *key == row.key && *generation == held);
            match cached {
                Some((_, _, want)) => want.clone(),
                None => {
                    let want = Want::of_garment(state.workbench.packs(), &row);
                    shown.garment_want = Some((row.key.clone(), held, want.clone()));
                    want
                }
            }
        }
        _ => Want::of(&row),
    };
    let picture = match state.workbench.glance.look(want, &crypto) {
        Glimpse::Nothing => Picture::None,
        Glimpse::Reading => Picture::Reading,
        Glimpse::Picture(thumb) => Picture::Drawn(std::sync::Arc::clone(thumb)),
        Glimpse::Cannot(why) => Picture::Cannot(why.to_string()),
    };
    if state.workbench.glance.is_reading() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    let room = ui.available_height();
    // ⚠ A panel too short to hold the quick look and the table gives it the whole panel, so nothing is drawn over the table.
    let docked = room >= DOCK_ROOM;
    let height = if docked {
        LOOK
    } else {
        (room - 40.0).max(60.0)
    };
    let mut open = true;
    let mut body = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr!("Quick look: {name}", name = row.name)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.small_button(tr!("{icon} Close", icon = egui_phosphor::regular::X));
                close.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        tr!("Close quick look"),
                    )
                });
                if close
                    .on_hover_text(tr!("Close the quick look (Space or Escape)"))
                    .clicked()
                {
                    open = false;
                }
            });
        });
        // ⚠ The picture on the left and what is known of the asset beside it, so the strip is as tall as the picture and no taller: a picture over its words took more than the panel had and was cut.
        ui.horizontal_top(|ui| {
            let caption = picture_of(ui, shown, &row, &picture, height);
            egui::ScrollArea::vertical()
                .id_salt("asset_quick_look_body")
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        if let Some((caption, colour)) = caption {
                            ui.add(
                                egui::Label::new(egui::RichText::new(caption).color(colour)).wrap(),
                            );
                        }
                        facts_of(ui, &row);
                        for finding in state.workbench.findings_of(&row) {
                            ui.add_space(theme::SPACING_TIGHT);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!(
                                        "{}: {}",
                                        finding.severity.label(),
                                        finding.text
                                    ))
                                    .color(super::severity_colour(finding.severity)),
                                )
                                .wrap(),
                            );
                        }
                    });
                });
        });
    };
    if docked {
        // ⚠ A height stated and not measured: a panel measures itself on its first frame, when the table has not left it its room, and keeps the cut.
        egui::Panel::bottom("asset_quick_look")
            .resizable(false)
            .exact_size(LOOK + STRIP_AROUND)
            .show_inside(ui, body);
    } else {
        body(ui);
    }
    if !open || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        state.workbench.quick_look = false;
    }
    !docked
}

/// What Properties states of `row` in two lines, so a quick look says what the asset is without Properties: its kind, edition, size and facts, and the textures a source holds.
fn facts_of(ui: &mut egui::Ui, row: &AssetRow) {
    let mut said = vec![row.kind.clone()];
    said.extend(row.target.map(str::to_string));
    said.extend(row.bytes.map(super::size));
    // ⚠ The short facts only: an id is bookkeeping and a list of the files it compiles from is Properties'.
    said.extend(
        (row.facts.iter())
            .filter(|(name, value)| {
                ![tr!("Stable id"), tr!("Created with")].contains(name) && !value.contains('\n')
            })
            .map(|(name, value)| format!("{name} {value}")),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(said.join("  ·  "))
                .small()
                .color(theme::TEXT_SECONDARY),
        )
        .wrap(),
    );
    if !row.textures.is_empty() {
        // ⚠ A few are named; many are counted with the first, on one line that ends with an ellipsis and not in the middle of a name.
        let said = match row.textures.len() {
            1..=3 => tr!("Textures: {names}", names = row.textures.join(", ")),
            n => tr!(
                "{n} textures, the first {first}",
                n = n,
                first = row.textures[0]
            ),
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(said)
                    .small()
                    .color(theme::TEXT_SECONDARY),
            )
            .truncate(),
        );
    }
}

/// The picture of `row` where there is one, and the line that goes beside it: its file and size, or why there is no picture.
fn picture_of(
    ui: &mut egui::Ui,
    shown: &mut AssetsUi,
    row: &AssetRow,
    picture: &Picture,
    room: f32,
) -> Option<(String, theme::Themed)> {
    match picture {
        Picture::None => {
            let said = match row.kept {
                Kept::Pack => tr!("This garment has no texture to draw"),
                _ => tr!("No picture for a file of this kind"),
            };
            Some((said.to_string(), theme::TEXT_SECONDARY))
        }
        Picture::Reading => {
            ui.spinner();
            Some((
                tr!("Reading the texture…").to_string(),
                theme::TEXT_SECONDARY,
            ))
        }
        Picture::Cannot(why) => Some((
            tr!("The texture cannot be drawn: {why}", why = why),
            theme::WARNING,
        )),
        Picture::Drawn(thumb) => {
            let texture = match &shown.picture {
                Some((id, handle)) if *id == thumb.id => handle.clone(),
                _ => {
                    let handle = ui.ctx().load_texture(
                        format!("quick-look-{}", thumb.id),
                        egui::ColorImage::from_rgba_unmultiplied(thumb.size, &thumb.rgba),
                        egui::TextureOptions::LINEAR,
                    );
                    shown.picture = Some((thumb.id, handle.clone()));
                    handle
                }
            };
            let [w, h] = thumb.size;
            // ⚠ Fitted to the room it has in both ways, so a short panel shows the whole picture smaller and never cuts it.
            let fit = (ui.available_width() / w.max(1) as f32)
                .min(room / h.max(1) as f32)
                .min(1.0);
            let drawn = egui::vec2(w as f32, h as f32) * fit;
            let file = Want::of(row)
                .and_then(|want| {
                    want.file
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| row.name.clone());
            let said = tr!(
                "{width} by {height}",
                width = thumb.full[0],
                height = thumb.full[1]
            );
            ui.add(egui::Image::new((texture.id(), drawn)).alt_text(tr!(
                "Picture of {file}, {said}",
                file = file,
                said = said
            )));
            Some((format!("{file}, {said}"), theme::TEXT_SECONDARY))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::state::{AppState, Prefs};

    fn project_of(dir: &Path, name: &str, files: &[&str]) -> std::path::PathBuf {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        let members: Vec<std::path::PathBuf> = files
            .iter()
            .map(|file| {
                std::fs::write(folder.join(file), b"x").unwrap();
                folder.join(file)
            })
            .collect();
        let file = folder.join(format!("{name}.openrage-proj"));
        or_project::Project::create(&file, &or_project::Container::default(), &members).unwrap();
        file
    }

    fn scanned(state: &mut AppState) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !state.workbench.is_scanned() {
            assert!(std::time::Instant::now() < until, "the scan never ended");
            state.poll_workbench();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn draw_frames(state: &mut AppState, ctx: &egui::Context) {
        for _ in 0..3 {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| super::draw(ui, state),
            );
        }
    }

    /// ⚠ The filter keeps indices into the rows, so a second project's table must not read the first one's.
    #[test]
    fn a_second_project_never_draws_with_the_rows_of_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let first = project_of(dir.path(), "a", &["1.lua", "2.lua", "3.lua"]);
        let second = project_of(dir.path(), "b", &["only.lua"]);
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        let ctx = egui::Context::default();

        state.load_project_from(first);
        scanned(&mut state);
        draw_frames(&mut state, &ctx);
        assert_eq!(state.workbench.rows().len(), 3);

        state.load_project_from(second);
        scanned(&mut state);
        draw_frames(&mut state, &ctx);
        assert_eq!(state.workbench.rows().len(), 1);
    }
}
