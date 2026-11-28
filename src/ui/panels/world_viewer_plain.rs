//! The World screen of a world drawn through the seam: the view flown through, and beside it the
//! sections the session's capabilities and its title's options state, with no section of GTA VI's.
//!
//! ⚠ An option is chosen at open: a change of one opens the world again, with an index of its own.

use glam::Vec3;
use or_world::caps::{Capability, WorldCaps};
use or_world::slots::{Choices, OptionId, OptionKind, OptionValue};

use super::log::grouped;
use super::world_viewer::{keys, whole_window, Capture, Pose, CLEAN, VIEW_MIN};
use crate::render::RenderBridge;
use crate::state::prefs::Game;
use crate::state::world_session::{Kind, Standing, Undrawn, Unread, Waiting};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::panels::settings::SettingsCategory;
use crate::ui::theme;

/// How many reasons the list of what could not be read shows before it scrolls.
const UNREAD_ROWS: f32 = 10.0;

/// One section of the panel beside a fed world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// What stands between the person and the world.
    Blocked,
    Hour,
    GoTo,
    /// What the title lets a person choose at open.
    Options,
    Streaming,
    /// The weather the world is looked at under, for a world whose maps a weather gates.
    Weather,
    Unread,
}

/// The sections a world of `caps` and `options` shows while it stands as `standing` with `unread` reasons unread, in their order.
pub fn sections(
    caps: WorldCaps,
    options: &[OptionKind],
    standing: &Standing,
    unread: usize,
) -> Vec<Section> {
    let open = matches!(standing, Standing::Open);
    let mut out = Vec::new();
    if !open {
        out.push(Section::Blocked);
    }
    if open {
        out.extend([Section::Hour, Section::GoTo]);
    }
    if !options.is_empty() {
        out.push(Section::Options);
    }
    if open {
        out.push(Section::Streaming);
        if caps.has(Capability::WeatherGate) {
            out.push(Section::Weather);
        }
        if unread > 0 {
            out.push(Section::Unread);
        }
    }
    out
}

/// What the panel holds between frames: the place typed, and the options being chosen.
#[derive(Default)]
pub struct Fields {
    place: String,
    refused: bool,
    /// The game the options below are being chosen for.
    of: Option<Game>,
    options: Vec<(OptionId, OptionValue)>,
}

impl crate::ui::panel_state::PanelState for Fields {
    const KEY: &'static str = "world-plain-fields";
}

fn said(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::TEXT_SECONDARY));
}

fn warned(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::WARNING));
}

/// What stands in the way of the world of `game`, as one line; `None` once it is open.
fn blocked_line(game: Game, standing: &Standing) -> Option<String> {
    match standing {
        Standing::Opening(done) => Some(tr!(
            "Indexing the world of {game}: {percent} %",
            game = game.label(),
            percent = (done * 100.0).round() as u32
        )),
        Standing::Failed(why) => Some(tr!(
            "The world of {game} did not open: {why}",
            game = game.label(),
            why = why
        )),
        Standing::Open => None,
    }
}

/// What a world still waits for, as the corner of the view says it; `None` once nothing is.
fn loading_line(waiting: Waiting) -> Option<String> {
    let mut parts = Vec::new();
    if waiting.maps > 0 {
        parts.push(tr_n!("{count} map", "{count} maps", waiting.maps));
    }
    if waiting.models > 0 {
        parts.push(tr_n!("{count} model", "{count} models", waiting.models));
    }
    if waiting.textures > 0 {
        parts.push(tr_n!(
            "{count} texture",
            "{count} textures",
            waiting.textures
        ));
    }
    (!parts.is_empty()).then(|| tr!("Still loading: {what}", what = parts.join(", ")))
}

/// The position or the camera line `text` states: three numbers are a place the camera is put over, looking as it did.
fn place_of(text: &str, from: &Pose) -> Result<Pose, String> {
    let numbers: Vec<f32> = (text.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|word| !word.is_empty())
        .map_while(|word| word.parse::<f32>().ok().filter(|n| n.is_finite()))
        .collect();
    let words = text.split(|c: char| c.is_whitespace() || c == ',');
    match (&numbers[..], words.filter(|word| !word.is_empty()).count()) {
        (&[x, y, z], 3) => Ok(Pose {
            position: Vec3::new(x, y, z),
            ..*from
        }),
        _ => Pose::read(text),
    }
}

/// The view: the world through the fly camera.
pub fn draw_view(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    let held = state.panels.get::<Capture>();
    // A clean frame draws no control, so Escape is the way back from it.
    if held.clean && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        held.clean = false;
    }
    let clean = held.clean;
    let Some(session) = state.world_session() else {
        return;
    };
    session.poll();
    let game = session.key().game;

    let (rect, response) = match clean {
        true => whole_window(ui.ctx()),
        false => {
            let size = egui::vec2(ui.available_width(), ui.available_height().max(VIEW_MIN));
            ui.allocate_exact_size(size, egui::Sense::click_and_drag())
        }
    };
    let response = named(
        response,
        &tr!("The world of {game}, flown through", game = game.label()),
    );
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let mut camera = session.camera();
    if response.dragged() {
        let by = response.drag_delta();
        camera.look(by.x, by.y);
    }
    let over = response.hovered() || response.has_focus();
    let mut moving = response.dragged();
    if over {
        // ⚠ The arrows are the camera's: without the lock egui moves the keyboard to the widget beside the view.
        crate::ui::focus_ring::reads_typed_text(ui.ctx(), response.id);
        let (along, across, up, fast) = keys(ui);
        if (along, across, up) != (0.0, 0.0, 0.0) {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            camera.fly(along, across, up, fast, dt);
            moving = true;
        }
    }
    if response.hovered() {
        let notches = ui.input(|i| i.smooth_scroll_delta.y) / 50.0;
        if notches != 0.0 {
            camera.scale_speed(notches);
        }
    }
    let speed = camera.speed;
    session.set_camera(camera);
    let (standing, waiting) = (session.standing(), session.waiting());
    let fading = session
        .feed()
        .is_some_and(|feed| feed.selection().is_some_and(|selection| selection.fading));
    // The frame is drawn again while the camera moves and while anything is still arriving.
    if moving || fading || waiting.any() || !matches!(standing, Standing::Open) {
        ui.ctx().request_repaint();
    }

    let painter = match clean {
        // ⚠ Over every other layer, the toasts among them: a clean frame holds the world alone.
        true => {
            let layer = egui::LayerId::new(egui::Order::Debug, egui::Id::new(CLEAN));
            ui.ctx().layer_painter(layer)
        }
        false => ui.painter_at(rect),
    };
    match (bridge, session.feed()) {
        (Some(bridge), Some(feed)) => {
            let ppp = ui.ctx().pixels_per_point();
            bridge.set_desired_size(
                (rect.width() * ppp).round() as u32,
                (rect.height() * ppp).round() as u32,
            );
            bridge.refresh_world_feed(feed);
            bridge.push_inputs(Default::default());
            match bridge.texture_id() {
                Some(texture) => {
                    let whole =
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.image(texture, rect, whole, egui::Color32::WHITE);
                }
                None => {
                    painter.rect_filled(rect, 0.0, theme::BG_L2);
                }
            }
        }
        _ => {
            painter.rect_filled(rect, 0.0, theme::BG_L2);
        }
    }
    if clean {
        return;
    }
    let wrap = (rect.width() - 22.0).max(VIEW_MIN);
    let chip = |text: String, at: egui::Pos2, anchor: egui::Align2| {
        let font = egui::FontId::proportional(12.0);
        let galley = painter.layout(text, font, theme::TEXT_PRIMARY.get(), wrap);
        let at = anchor.anchor_size(at, galley.size()).min;
        let back = egui::Rect::from_min_size(at, galley.size()).expand(3.0);
        painter.rect_filled(
            back,
            egui::CornerRadius::same(theme::RADIUS_SMALL),
            theme::BG_L2,
        );
        painter.galley(at, galley, theme::TEXT_PRIMARY.get());
    };
    // With no world to fly yet, what stands in the way is said in the middle of the view.
    match blocked_line(game, &standing) {
        Some(line) => chip(line, rect.center(), egui::Align2::CENTER_CENTER),
        None => chip(
            tr!(
                "Drag to look, W A S D or Z Q S D to move, E and C up and down, Shift faster; the wheel sets the speed: {speed} m/s",
                speed = speed.round() as u32
            ),
            rect.left_top() + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
        ),
    }
    if let Some(said) = loading_line(waiting).filter(|_| matches!(standing, Standing::Open)) {
        let corner = rect.left_bottom() + egui::vec2(8.0, -6.0);
        chip(said, corner, egui::Align2::LEFT_BOTTOM);
    }
}

/// The panel beside the view.
pub fn draw_side(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    let drawn = bridge.map(|bridge| bridge.world_drawn());
    let Some(session) = state.world_session() else {
        return;
    };
    let game = session.key().game;
    let (standing, caps, kinds) = (session.standing(), session.caps(), session.option_kinds());
    let (waiting, unread) = (session.waiting(), session.unread());
    let (mut hour, camera) = (session.hour(), session.camera());
    let (weathers, mut weather, undrawn) =
        (session.weathers(), session.weather(), session.undrawn());
    let opened_with = session.key().options.clone();
    let fields = state.panels.get::<Fields>();
    if fields.of != Some(game) {
        (fields.of, fields.options) = (Some(game), opened_with.clone());
    }
    let (mut go, mut reopen, mut settings) = (None, None, false);
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (at, section) in sections(caps, &kinds, &standing, unread.len())
            .into_iter()
            .enumerate()
        {
            if at > 0 {
                ui.separator();
            }
            match section {
                Section::Blocked => {
                    if let Some(line) = blocked_line(game, &standing) {
                        match standing {
                            Standing::Opening(done) => {
                                ui.strong(line);
                                ui.add(egui::ProgressBar::new(done));
                                said(
                                    ui,
                                    tr!("The first visit reads every archive of the install, which takes several minutes; the index is kept, so the next visit opens at once."),
                                );
                            }
                            _ => {
                                warned(ui, &line);
                                settings = ui.button(tr!("Settings ▸ Games…")).clicked();
                            }
                        }
                    }
                }
                Section::Hour => {
                    ui.strong(tr!("Hour"));
                    ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(60.0);
                    let slider = ui.add(egui::Slider::new(&mut hour, 0.0..=23.99));
                    named(
                        slider,
                        tr!("The hour of the day: what is placed for some hours draws inside them, and the sun stands where that hour puts it"),
                    );
                }
                Section::GoTo => {
                    ui.strong(tr!("Go to"));
                    ui.horizontal(|ui| {
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut fields.place)
                                .hint_text(tr!("X Y Z in metres, or a camera line"))
                                .desired_width((ui.available_width() - 56.0).max(60.0)),
                        );
                        let field = named(field, tr!("The place to put the camera at"));
                        let entered =
                            field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.button(tr!("Go")).clicked() || entered {
                            match place_of(&fields.place, &Pose::of(&camera)) {
                                Ok(pose) => (go, fields.refused) = (Some(pose), false),
                                Err(_) => fields.refused = true,
                            }
                        }
                    });
                    if fields.refused {
                        warned(
                            ui,
                            tr!("That is no place: write X Y Z in metres, or paste a camera line"),
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        said(ui, &Pose::of(&camera).line());
                    });
                }
                Section::Options => {
                    ui.strong(tr!("Options"));
                    for kind in &kinds {
                        option(ui, kind, &mut fields.options);
                    }
                    let mut chosen = fields.options.clone();
                    chosen.sort();
                    let same = chosen == opened_with;
                    let open = ui
                        .add_enabled(!same, egui::Button::new(tr!("Open the world with these")))
                        .on_hover_text(tr!(
                            "A world opened with other options has an index of its own, read from the install the first time"
                        ));
                    if open.clicked() {
                        reopen = Some(chosen);
                    }
                }
                Section::Streaming => {
                    ui.strong(tr!("Streaming"));
                    let drawn = drawn.unwrap_or_default();
                    said(
                        ui,
                        &tr!(
                            "{seen} of {selected} instances in view, in {draws} draws",
                            seen = grouped(drawn.seen as u64),
                            selected = grouped(drawn.selected as u64),
                            draws = grouped(drawn.draws as u64),
                        ),
                    );
                    said(
                        ui,
                        &tr!(
                            "{models} models on the device ({megabytes} MB), {waiting} waited for",
                            models = grouped(drawn.models as u64),
                            megabytes = grouped(drawn.bytes >> 20),
                            waiting = grouped(waiting.models as u64),
                        ),
                    );
                    said(
                        ui,
                        &tr!(
                            "{textures} textures on the device, {unread} not read, {waiting} waited for",
                            textures = grouped(drawn.textures as u64),
                            unread = grouped(drawn.textures_unread as u64),
                            waiting = grouped(waiting.textures as u64),
                        ),
                    );
                    if waiting.maps > 0 {
                        said(
                            ui,
                            &tr_n!(
                                "{count} map is still read",
                                "{count} maps are still read",
                                waiting.maps
                            ),
                        );
                    }
                    for line in undrawn_lines(undrawn) {
                        said(ui, &line);
                    }
                    said(
                        ui,
                        tr!("Drawn by the plain pass: one sun and level water, and no shadow, sky or shader of the game's own"),
                    );
                }
                Section::Weather => {
                    ui.strong(tr!("Weather"));
                    let name = |id: u64| {
                        let named = weathers.iter().find(|weather| weather.id == id);
                        named.map_or_else(String::new, |weather| weather.name.clone())
                    };
                    let combo = egui::ComboBox::from_id_salt("world_plain_weather")
                        .selected_text(name(weather))
                        .show_ui(ui, |ui| {
                            for each in &weathers {
                                ui.selectable_value(&mut weather, each.id, &each.name);
                            }
                        });
                    named(
                        combo.response,
                        tr!("The weather the world is looked at under: a map placed for some weathers streams under them alone"),
                    );
                }
                Section::Unread => {
                    ui.strong(tr!("Could not be read"));
                    let row = ui.text_style_height(&egui::TextStyle::Body);
                    egui::ScrollArea::vertical()
                        .id_salt("world_plain_unread")
                        .max_height(row * UNREAD_ROWS)
                        .show(ui, |ui| {
                            for each in &unread {
                                said(ui, &unread_line(each));
                            }
                        });
                }
            }
        }
    });
    if settings {
        super::settings_window::open_settings_at(state, SettingsCategory::Games);
    }
    if let Some(chosen) = reopen {
        state.world.choose(game, chosen);
    }
    let Some(session) = state.world_session() else {
        return;
    };
    session.set_hour(hour);
    if weather != session.weather() {
        session.set_weather(weather);
    }
    if let Some(pose) = go {
        let mut camera = session.camera();
        pose.put(&mut camera);
        session.set_camera(camera);
    }
}

/// What a selection holds that is not drawn, a line a kind; none where everything is.
fn undrawn_lines(undrawn: Undrawn) -> Vec<String> {
    let mut lines = Vec::new();
    if undrawn.stand_ins > 0 {
        lines.push(tr_n!(
            "{count} thing placed has no model named and is not drawn",
            "{count} things placed have no model named and are not drawn",
            undrawn.stand_ins
        ));
    }
    if undrawn.lists > 0 {
        lines.push(tr_n!(
            "{count} list of instances (grass, batches of props) is not drawn",
            "{count} lists of instances (grass, batches of props) are not drawn",
            undrawn.lists
        ));
    }
    lines
}

/// One reason things could not be read, with how many.
fn unread_line(unread: &Unread) -> String {
    let why = unread.why.shown();
    match unread.kind {
        Kind::Map => tr_n!(
            "{count} map: {why}",
            "{count} maps: {why}",
            unread.count,
            why = why
        ),
        Kind::Model => tr_n!(
            "{count} model: {why}",
            "{count} models: {why}",
            unread.count,
            why = why
        ),
        Kind::Texture => tr_n!(
            "{count} texture: {why}",
            "{count} textures: {why}",
            unread.count,
            why = why
        ),
    }
}

/// One option of the title, set in `chosen`; an option left as the title's own is in it under no value.
fn option(ui: &mut egui::Ui, kind: &OptionKind, chosen: &mut Vec<(OptionId, OptionValue)>) {
    let label = crate::i18n::tr(kind.label.key());
    let at = chosen.iter().position(|(id, _)| *id == kind.id);
    let unset = tr!("As the install has it");
    match &kind.choices {
        Choices::Toggle => {
            let mut on = matches!(at.map(|at| &chosen[at].1), Some(OptionValue::On(true)));
            if ui.checkbox(&mut on, label).changed() {
                chosen.retain(|(id, _)| *id != kind.id);
                chosen.push((kind.id, OptionValue::On(on)));
            }
        }
        Choices::OneOf(choices) => {
            let current = match at.map(|at| &chosen[at].1) {
                Some(OptionValue::Choice(choice)) => Some(*choice),
                _ => None,
            };
            let name = |choice: Option<u32>| {
                let found = choices.iter().find(|(id, _)| Some(*id) == choice);
                found.map_or(unset, |(_, sentence)| crate::i18n::tr(sentence.key()))
            };
            ui.horizontal(|ui| {
                let combo = egui::ComboBox::from_id_salt(("world_plain_option", kind.id.0))
                    .selected_text(name(current))
                    .show_ui(ui, |ui| {
                        let mut picked = current;
                        let all = std::iter::once(None).chain(choices.iter().map(|c| Some(c.0)));
                        for choice in all {
                            if ui
                                .selectable_label(choice == current, name(choice))
                                .clicked()
                            {
                                picked = choice;
                            }
                        }
                        picked
                    });
                named(combo.response, label);
                ui.label(label);
                if let Some(picked) = combo.inner.filter(|picked| *picked != current) {
                    chosen.retain(|(id, _)| *id != kind.id);
                    if let Some(choice) = picked {
                        chosen.push((kind.id, OptionValue::Choice(choice)));
                    }
                }
            });
        }
        Choices::Named => {
            let mut text = match at.map(|at| &chosen[at].1) {
                Some(OptionValue::Named(name)) => name.clone(),
                _ => String::new(),
            };
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text(unset)
                        .desired_width(120.0),
                );
                let changed = named(field, label).changed();
                ui.label(label);
                if changed {
                    chosen.retain(|(id, _)| *id != kind.id);
                    if !text.trim().is_empty() {
                        chosen.push((kind.id, OptionValue::Named(text.trim().to_owned())));
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use or_world::sentence;

    use super::*;

    fn pose() -> Pose {
        Pose {
            position: Vec3::new(1.0, 2.0, 3.0),
            yaw: 40.0,
            pitch: -10.0,
            fov: Some(60.0),
        }
    }

    #[test]
    fn three_numbers_move_the_camera_and_keep_where_it_looks_and_a_camera_line_puts_all_of_it() {
        let at = place_of("-800, 150.5 90", &pose()).expect("three numbers");
        assert_eq!(at.position, Vec3::new(-800.0, 150.5, 90.0));
        assert_eq!((at.yaw, at.pitch, at.fov), (40.0, -10.0, Some(60.0)));
        let line = place_of("world camera 5 6 7 90 -20 50", &pose()).expect("a camera line");
        assert_eq!(line.position, Vec3::new(5.0, 6.0, 7.0));
        assert_eq!((line.yaw, line.pitch, line.fov), (90.0, -20.0, Some(50.0)));
        for bad in ["", "1 2", "1 2 three", "1 2 3 4", "north of the pier"] {
            assert!(place_of(bad, &pose()).is_err(), "`{bad}` should be refused");
        }
    }

    #[test]
    fn what_is_not_drawn_is_said_a_line_a_kind_with_its_count_and_nothing_where_all_is_drawn() {
        assert!(undrawn_lines(Undrawn::default()).is_empty());
        let boxes = undrawn_lines(Undrawn {
            stand_ins: 1,
            lists: 0,
        });
        assert_eq!(
            boxes,
            ["1 thing placed has no model named and is not drawn"]
        );
        let both = undrawn_lines(Undrawn {
            stand_ins: 12,
            lists: 3,
        });
        assert_eq!(
            both,
            [
                "12 things placed have no model named and are not drawn",
                "3 lists of instances (grass, batches of props) are not drawn"
            ]
        );
    }

    #[test]
    fn the_sections_follow_what_the_world_states_and_where_it_stands() {
        let options = [OptionKind {
            id: OptionId(1),
            label: sentence!("DLC pack"),
            choices: Choices::Named,
        }];
        let gated = WorldCaps::none().with(Capability::WeatherGate);
        // Until it is open, what stands in the way and what it may be opened with, and nothing of a world.
        assert_eq!(
            sections(gated, &options, &Standing::Opening(0.2), 3),
            [Section::Blocked, Section::Options]
        );
        assert_eq!(
            sections(gated, &[], &Standing::Failed("why".into()), 0),
            [Section::Blocked]
        );
        assert_eq!(
            sections(gated, &options, &Standing::Open, 2),
            [
                Section::Hour,
                Section::GoTo,
                Section::Options,
                Section::Streaming,
                Section::Weather,
                Section::Unread
            ]
        );
        // A world that states no weather gate and no option, with nothing unread, shows neither.
        assert_eq!(
            sections(WorldCaps::none(), &[], &Standing::Open, 0),
            [Section::Hour, Section::GoTo, Section::Streaming]
        );
    }

    #[test]
    fn what_could_not_be_read_is_said_with_its_count_and_its_reason() {
        use crate::state::world_session::Reason;
        let unread = Unread {
            kind: Kind::Model,
            why: Reason::Told("zq why".into()),
            count: 3,
        };
        let line = unread_line(&unread);
        assert!(line.contains('3') && line.contains("zq why"), "{line}");
        assert_eq!(
            blocked_line(Game::GtaVLegacy, &Standing::Open),
            None,
            "an open world has nothing in its way"
        );
        let failed = blocked_line(Game::GtaVLegacy, &Standing::Failed("zq why".into()));
        assert!(failed.is_some_and(|line| line.contains("zq why")));
        assert_eq!(loading_line(Waiting::default()), None);
        let waiting = Waiting {
            maps: 2,
            models: 0,
            textures: 1,
        };
        assert!(loading_line(waiting).is_some_and(|line| line.contains('2')));
    }
}
