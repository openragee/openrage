//! The start page, which the Document panel draws while nothing is open, and
//! the installs found that it and the status bar's game switcher offer.

use std::path::PathBuf;

use egui_phosphor::regular as icon;
use or_title::game::Platform;

use crate::platform::file_dialogs;
use crate::state::{AppState, Game, RecentFile};
use crate::ui::panels::settings::{open_settings_at, SettingsCategory};
use crate::ui::theme;

/// The widest the start page's column grows, in points.
const COLUMN_WIDTH: f32 = 880.0;

/// How many of each recent list the start page offers.
const RECENT_SHOWN: usize = 5;

/// `game`'s install folder, while it is on disk.
pub(crate) fn found_install(state: &AppState, game: Game) -> Option<PathBuf> {
    state
        .prefs
        .game_paths
        .get(game)
        .filter(|p| p.is_dir())
        .cloned()
}

/// What to open, and whether the keys are ready.
pub fn draw_start_page(ui: &mut egui::Ui, state: &mut AppState) {
    egui::ScrollArea::vertical()
        .id_salt("start_page")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width().min(COLUMN_WIDTH);
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - width) / 2.0);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(theme::SPACING_SECTION);
                    super::start_page::draw_back(ui, state);
                    draw_app_branding(ui);
                    // ⚠ What is installed and what was open come first: they are where looking at the game and going back to work start, and a first launch shows all of the page with no scroll.
                    heading(ui, tr!("Open an install"));
                    draw_installs(ui, state);
                    let first_launch = state.prefs.recent_projects.is_empty();
                    if !first_launch {
                        super::start_page::draw_projects(ui, state);
                    }
                    draw_recents(ui, state);
                    super::start_page::draw_new(ui, state);
                    super::start_page::draw_ways_in(ui, state);
                    if first_launch {
                        super::start_page::draw_tips(ui, state);
                    }
                });
            });
        });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(theme::SPACING_RELAXED * 2.0);
    ui.label(egui::RichText::new(text).size(16.0).strong());
    ui.add_space(theme::SPACING_TIGHT);
}

fn draw_installs(ui: &mut egui::Ui, state: &mut AppState) {
    let mut chosen: Option<(Game, PathBuf)> = None;
    let mut key_details = false;
    for game in builds_by_title() {
        let found = found_install(state, game);
        // ⚠ A store can find a PC build, so its row stands while it is missing; a console build is a folder the user adds, and has a row once it was.
        let profile = game.profile();
        if found.is_none() && !(profile.is_supported() && profile.platform() == Platform::Pc) {
            continue;
        }
        // The keys are read from the executable of the install in use, so its row is the one that says how that went.
        let keys = (state.prefs.active_game == Some(game))
            .then(|| keys_line(state))
            .flatten();
        key_details |= draw_game_row(ui, game, found, keys, &mut chosen);
        ui.add_space(theme::SPACING_SNUG);
    }
    if key_details {
        open_settings_at(state, SettingsCategory::Games);
    }
    if ui.button(tr!("Add a game folder…")).clicked() {
        if let Some(dir) =
            file_dialogs::pick_game_install_folder(tr!("Select the folder of a game"))
        {
            match or_title::game::registry::detect(&dir).and_then(|p| Game::by_id(p.id())) {
                Some(game) => chosen = Some((game, dir)),
                None => state.show_error_toast(tr!(
                    "No game OpenRage knows is in {folder}",
                    folder = dir.display()
                )),
            }
        }
    }
    if state.discovery.is_running() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(
                egui::RichText::new(tr!(
                    "Looking for installs (Steam, Rockstar Games Launcher, Epic)…"
                ))
                .small()
                .color(theme::TEXT_SECONDARY),
            );
        });
    }
    if let Some((game, root)) = chosen {
        if game.profile().is_supported() {
            state.request_game_switch(game, root);
        } else {
            state.open_title_folder(game, root);
        }
    }
}

/// Every build, those of one title together, titles in the order the registry first names them.
fn builds_by_title() -> Vec<Game> {
    let mut builds: Vec<Game> = Game::all().collect();
    let titles: Vec<&str> = builds.iter().map(|g| g.profile().title()).collect();
    let first = |game: &Game| titles.iter().position(|t| *t == game.profile().title());
    builds.sort_by_key(first);
    builds
}

/// One install with its button and, for the install in use, what `keys` says; whether Key details… was asked for.
fn draw_game_row(
    ui: &mut egui::Ui,
    game: Game,
    found: Option<PathBuf>,
    keys: Option<KeysLine>,
    chosen: &mut Option<(Game, PathBuf)>,
) -> bool {
    const LOGO_SIZE: f32 = 48.0;
    let profile = game.profile();
    let mut key_details = false;
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            draw_game_logo(ui, game, LOGO_SIZE);
            ui.add_space(theme::SPACING_LOOSE);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(game.label()).strong().size(15.0));
                let detail = found.as_deref().map_or_else(
                    || tr!("Not found on this machine").to_string(),
                    |dir| match profile.platform() {
                        Platform::Pc => dir.display().to_string(),
                        console => format!("{} · {}", console.label(), dir.display()),
                    },
                );
                ui.label(
                    egui::RichText::new(detail)
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
                if let Some(keys) = keys {
                    ui.horizontal(|ui| {
                        let text = format!("{} {}", icon::KEY, keys.text);
                        ui.label(egui::RichText::new(text).small().color(keys.colour));
                        key_details = keys.failed && ui.small_button(tr!("Key details…")).clicked();
                    });
                }
            });
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match found {
                    Some(dir) => {
                        let browsed = crate::state::gta_vi::read_by_scan(game);
                        let mut open = ui.button(if profile.is_supported() || browsed {
                            tr!("Open")
                        } else {
                            tr!("Open as a folder")
                        });
                        if !profile.is_supported() {
                            open = open.on_hover_text(profile.description());
                        }
                        if open.clicked() {
                            *chosen = Some((game, dir));
                        }
                    }
                    None => {
                        if ui.button(tr!("Choose its folder…")).clicked() {
                            let title =
                                tr!("Select install folder for {game}", game = game.label());
                            if let Some(dir) = file_dialogs::pick_game_install_folder(&title) {
                                *chosen = Some((game, dir));
                            }
                        }
                    }
                },
            );
        });
    });
    key_details
}

fn draw_recents(ui: &mut egui::Ui, state: &mut AppState) {
    let files = state.prefs.recent_files.clone();
    if let Some(recent) = recent_files(ui, tr!("Recent files"), &files) {
        state.open_recent(&recent);
    }
    if let Some(path) = recent_list(ui, tr!("Recent folders"), &state.prefs.recent_folders) {
        state.open_folder(path);
    }
}

/// The first of the `files` still on disk, each a link naming the file and when it was opened; the one clicked.
pub(crate) fn recent_files(
    ui: &mut egui::Ui,
    title: &str,
    files: &[RecentFile],
) -> Option<RecentFile> {
    let shown: Vec<&RecentFile> = files
        .iter()
        .filter(|r| r.exists())
        .take(RECENT_SHOWN)
        .collect();
    if shown.is_empty() {
        return None;
    }
    ui.add_space(theme::SPACING_DEFAULT);
    ui.label(
        egui::RichText::new(title)
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    let mut clicked = None;
    for recent in shown {
        ui.horizontal(|ui| {
            if ui
                .link(recent.name())
                .on_hover_text(recent.location())
                .clicked()
            {
                clicked = Some(recent.clone());
            }
            if let Some(when) = recent.opened_ago() {
                ui.label(
                    egui::RichText::new(when)
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
            }
        });
    }
    clicked
}

/// The first of `paths`, each a link naming its file; the one clicked.
pub(crate) fn recent_list(ui: &mut egui::Ui, title: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    ui.add_space(theme::SPACING_DEFAULT);
    ui.label(
        egui::RichText::new(title)
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    let mut clicked = None;
    for path in paths.iter().take(RECENT_SHOWN) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if ui
            .link(name)
            .on_hover_text(path.display().to_string())
            .clicked()
        {
            clicked = Some(path.clone());
        }
    }
    clicked
}

/// What an install's row says of its keys, beside a key glyph.
struct KeysLine {
    colour: theme::Themed,
    text: String,
    /// Reading them failed, which is when the row offers Key details….
    failed: bool,
}

/// How reading the keys of the install in use went, and nothing before it was tried.
fn keys_line(state: &AppState) -> Option<KeysLine> {
    let extraction = &state.keys.extraction;
    let (colour, text, failed) = if extraction.is_running {
        let stage = &extraction.stage;
        let text = tr!(
            "Reading the keys from the game's executable… {stage}",
            stage = stage
        );
        (theme::INFO, text, false)
    } else if state.keys.has_keys() {
        let text = tr!("Ready: encrypted archives open.").to_string();
        (theme::SUCCESS, text, false)
    } else {
        let error = extraction.last_error.as_ref()?;
        let text = tr!("Reading the keys failed: {error}", error = error);
        (theme::ERROR, text, true)
    };
    Some(KeysLine {
        colour,
        text,
        failed,
    })
}

fn draw_app_branding(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        draw_app_logo(ui, 56.0);
        ui.add_space(theme::SPACING_LOOSE);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("OpenRage").size(28.0).strong());
            ui.label(
                egui::RichText::new(crate::i18n::tr(crate::ui::branding::TAGLINE))
                    .color(theme::TEXT_SECONDARY),
            );
        });
    });
}

fn draw_app_logo(ui: &mut egui::Ui, size: f32) {
    ui.add_sized(
        egui::vec2(size, size),
        crate::ui::branding::app_logo_image(size),
    );
}

fn draw_game_logo(ui: &mut egui::Ui, game: Game, size: f32) {
    if let Some(uri) = resolve_game_logo_uri(game) {
        let image = egui::Image::new(uri)
            .fit_to_exact_size(egui::vec2(size, size))
            .maintain_aspect_ratio(true)
            .corner_radius(egui::CornerRadius::same((size * 0.16).round() as u8));
        ui.add_sized(egui::vec2(size, size), image);
        return;
    }
    draw_painted_game_logo(ui, game, size);
}

fn resolve_game_logo_uri(game: Game) -> Option<String> {
    use std::path::PathBuf;

    let filename = format!("{}.png", game.asset_slug());
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        candidates.push(dir.join("assets").join("logos").join(&filename));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("logos")
            .join(&filename),
    );
    if let Some(cached) = crate::state::game_discovery::cached_icon_path(game) {
        candidates.push(cached);
    }

    let path = candidates.into_iter().find(|p| p.is_file())?;
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let mut uri = canonical.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = uri.strip_prefix("//?/") {
        uri = stripped.to_string();
    }
    let uri = if uri.starts_with('/') {
        format!("file://{uri}")
    } else {
        format!("file:///{uri}")
    };
    Some(uri)
}

fn draw_painted_game_logo(ui: &mut egui::Ui, game: Game, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let brand = game.profile().brand();
    let rgb = |[r, g, b]: [u8; 3]| egui::Color32::from_rgb(r, g, b);
    let (bg_top, bg_bottom) = (rgb(brand.background.0), rgb(brand.background.1));
    let accent = rgb(brand.accent);
    let caption_color = rgb(brand.caption_color);
    let caption = brand.caption;
    let caption_size = size * brand.caption_scale;

    let radius = (size * 0.16).round() as u8;
    painter.rect_filled(rect, egui::CornerRadius::same(radius), bg_top);

    let half = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.center().y),
        rect.right_bottom(),
    );
    painter.rect_filled(half, egui::CornerRadius::same(radius), bg_bottom);

    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(radius),
        egui::Stroke::new(1.0_f32, accent),
        egui::epaint::StrokeKind::Inside,
    );

    let stripe = egui::Rect::from_min_max(
        egui::pos2(rect.left() + size * 0.10, rect.bottom() - size * 0.18),
        egui::pos2(rect.right() - size * 0.10, rect.bottom() - size * 0.14),
    );
    painter.rect_filled(stripe, egui::CornerRadius::same(1), accent);

    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        caption,
        egui::FontId::proportional(caption_size.max(10.0)),
        caption_color,
    );
}
