//! The World panel beside the view, in the order a person uses it: what stands in the way, the
//! hour and the weather, the places to go, the camera, what streams, the capture, and then what
//! is streaming and what the mount found.

use super::log::grouped;
use super::world_viewer::{offered, Capture, Pose};
use super::world_viewer_map as map;
use super::world_viewer_places::{self as places, Place};
use super::world_viewer_prefs::{LOD_SCALES, SPEEDS};
use crate::render::RenderBridge;
use crate::state::gta6_world::worldview::{self as gta6_worldview, Viewer};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::panels::settings::SettingsCategory;
use crate::ui::theme;

/// How many script-managed groups the list shows before it scrolls.
const GROUP_ROWS: f32 = 12.0;

fn said(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::TEXT_SECONDARY));
}

fn warned(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::WARNING));
}

fn count(n: impl Into<u64>) -> String {
    grouped(n.into())
}

/// What the last frame drew, as the Streaming section says it.
#[derive(Default, Clone, Copy)]
struct Drawn {
    seen: usize,
    selected: usize,
    draws: usize,
    models: usize,
    megabytes: u64,
    textures: usize,
    unread: usize,
    lit_by_the_game: bool,
}

/// The places found in the index the panel shows, by the index they were found in.
#[derive(Default)]
pub struct Places {
    of: (usize, usize),
    found: Vec<Place>,
}

impl crate::ui::panel_state::PanelState for Places {
    const KEY: &'static str = "world-places";
}

/// The text the groups list is filtered by.
#[derive(Default)]
pub struct GroupFilter(String);

impl crate::ui::panel_state::PanelState for GroupFilter {
    const KEY: &'static str = "world-group-filter";
}

/// The name the vocabulary gives `hash`, else the hash itself.
fn name_of(hash: u32) -> String {
    crate::state::gta6_names::name32(hash).unwrap_or_else(|| format!("{hash:08X}"))
}

/// The name the vocabulary gives `group`, by its word or its hash, else its hash.
fn group_name(
    vocabulary: &or_io::gta6::names::Vocabulary,
    group: &gta6_worldview::index::Group,
) -> String {
    let named = (group.word != 0)
        .then(|| vocabulary.name64(group.word))
        .flatten();
    (named.or_else(|| vocabulary.name32(group.name)))
        .map_or_else(|| or_maps::Name::of(group.name).shown(), str::to_string)
}

/// Which interior and room the camera stands in, where it stands in one.
fn where_inside(inside: gta6_worldview::Inside) -> String {
    tr!(
        "In the interior {interior}, room {room}",
        interior = name_of(inside.interior),
        room = name_of(inside.room),
    )
}

/// The panel beside the view.
pub fn draw_side(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    if !offered(state) {
        said(
            ui,
            tr!("Set the GTA VI folder in Settings ▸ Games to fly through its world"),
        );
        if ui.button(tr!("Settings ▸ Games…")).clicked() {
            super::settings_window::open_settings_at(state, SettingsCategory::Games);
        }
        return;
    }
    let drawn = bridge.map(|b| b.world_drawn()).map(|d| Drawn {
        seen: d.seen,
        selected: d.selected,
        draws: d.draws,
        models: d.models,
        megabytes: d.bytes >> 20,
        textures: d.textures,
        unread: d.textures_unread,
        lit_by_the_game: d.lit_by_the_game,
    });
    let mut viewer = gta6_worldview::viewer();
    egui::ScrollArea::vertical().show(ui, |ui| {
        if blocked(ui, state, &mut viewer, drawn) {
            ui.separator();
        }
        day(ui, state, &mut viewer);
        ui.separator();
        camera(ui, state, &mut viewer);
        ui.separator();
        streams(ui, state, &mut viewer);
        ui.separator();
        // ⚠ Folded at first: at 720 points high an open Capture pushes the groups list below the fold.
        egui::CollapsingHeader::new(tr!("Capture"))
            .id_salt("world_viewer_capture")
            .show(ui, |ui| capture(ui, state, &mut viewer));
        egui::CollapsingHeader::new(tr!("Streaming"))
            .id_salt("world_viewer_streaming")
            .default_open(true)
            .show(ui, |ui| streaming(ui, &viewer, drawn.unwrap_or_default()));
        egui::CollapsingHeader::new(tr!("Mount"))
            .id_salt("world_viewer_mount")
            .show(ui, |ui| mount(ui, &mut viewer));
    });
}

/// What stands between the person and the world, each with what to do: whether anything was said.
fn blocked(
    ui: &mut egui::Ui,
    state: &mut AppState,
    viewer: &mut Viewer,
    drawn: Option<Drawn>,
) -> bool {
    if let Some(done) = viewer.building() {
        ui.strong(tr!(
            "Indexing the GTA VI world: {percent} %",
            percent = (done * 100.0).round() as u32
        ));
        ui.add(egui::ProgressBar::new(done));
        said(
            ui,
            tr!("The first visit reads every archive of the level, which takes a few minutes; the index is kept, so the next visit opens at once. Background jobs can stop it."),
        );
        return true;
    }
    if let Some(why) = viewer.failed.clone() {
        warned(
            ui,
            &tr!("The GTA VI world was not indexed: {why}", why = why),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr!("Settings ▸ Games…")).clicked() {
                super::settings_window::open_settings_at(state, SettingsCategory::Games);
            }
            rebuild_button(ui, viewer);
        });
        return true;
    }
    let Some(index) = viewer.index.clone() else {
        return false;
    };
    let mut any = false;
    if index.report.by_table == 0 {
        warned(
            ui,
            tr!("No GTA VI key snapshot was found, so the maps were bound by their block maps and no hour switches a script-managed group. Put gtavi_magic.dat in the configuration folder, or name it with OPENRAGE_GTA6_KEYS, then build the index again."),
        );
        rebuild_button(ui, viewer);
        any = true;
    }
    if viewer.world_given_up() {
        warned(
            ui,
            &tr!(
                "The world stopped on an internal error {times} times and is not started again: it stands as it was last drawn. Build the index again to start afresh.",
                times = viewer.world_restarts
            ),
        );
        rebuild_button(ui, viewer);
        any = true;
    } else if viewer.world_restarts > 0 {
        warned(
            ui,
            &tr!(
                "The world stopped on an internal error and was started again ({times} times so far)",
                times = viewer.world_restarts
            ),
        );
        any = true;
    }
    if !viewer.skipped.is_empty() {
        warned(
            ui,
            &tr!(
                "The world leaves out {maps} maps it stopped on as they came in",
                maps = viewer.skipped.len()
            ),
        );
        any = true;
    }
    if drawn.is_some_and(|d| d.draws > 0 && !d.lit_by_the_game) {
        warned(
            ui,
            tr!("Lit by the plain pass: no game shader pack is read"),
        );
        said(
            ui,
            tr!("The game's shaders are not baked yet: bake them in Automate ▸ Bake game shaders"),
        );
        any = true;
    }
    any
}

fn rebuild_button(ui: &mut egui::Ui, viewer: &mut Viewer) {
    let again = ui.add_enabled(
        viewer.building().is_none(),
        egui::Button::new(tr!("Build the index again")),
    );
    if again
        .on_hover_text(tr!(
            "Read the registration documents and every map of the level again, and keep the index in place of the one kept"
        ))
        .clicked()
    {
        viewer.rebuild();
    }
}

/// The hour and the weather the world is drawn at.
fn day(ui: &mut egui::Ui, state: &mut AppState, viewer: &mut Viewer) {
    ui.strong(tr!("Time and weather"));
    let hour = wide(
        ui,
        tr!("Hour"),
        egui::Slider::new(&mut viewer.hour, 0.0..=23.99),
    );
    let hour = named(
        hour,
        tr!("The hour of the day: what is placed for some hours draws inside them, and the world is lit by its day at that hour"),
    );
    if hour.changed() {
        // ⚠ The Time bar and the world say one hour: a day left running would move the bar alone.
        let day = &mut state.drivers.time_of_day;
        day.set_hour(viewer.hour);
        day.playing = false;
    }
    ui.horizontal(|ui| {
        let current = state.drivers.weather.state;
        let chosen = egui::ComboBox::from_id_salt("world_viewer_weather")
            .selected_text(current.label())
            .show_ui(ui, |ui| {
                let mut chosen = current;
                for weather in crate::state::drivers::WeatherState::ALL {
                    if ui
                        .selectable_label(weather == current, weather.label())
                        .clicked()
                    {
                        chosen = weather;
                    }
                }
                chosen
            });
        named(chosen.response.clone(), tr!("Weather"));
        ui.label(tr!("Weather"));
        if let Some(weather) = chosen.inner.filter(|w| *w != current) {
            state.set_weather_state(weather);
        }
    });
}

/// A slider under its label, as wide as the panel, so that a long label is never cut at the edge.
fn wide(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'_>) -> egui::Response {
    ui.label(label);
    // Room for the value's field beside the slider.
    ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(60.0);
    ui.add(slider)
}

/// The name a place is listed by, and what its entry says on hover.
fn place_label(place: &Place) -> (String, &'static str) {
    let (name, hint) = match &place.head {
        Some(head) if place.from_place => (
            head.clone(),
            tr!("Look north over the square of 512 m where most maps of this place lie"),
        ),
        Some(head) => (
            head.clone(),
            tr!("Look north over the square of 512 m where most maps whose names start with these letters lie"),
        ),
        None => (
            format!(
                "X {} Y {}",
                place.middle.x.round() as i32,
                place.middle.y.round() as i32
            ),
            tr!("Look north over one of the squares of 512 m where most maps lie; once Harvest names has run, a place takes the letters its maps' names start with"),
        ),
    };
    let label = tr_n!(
        "{name}, {count} map",
        "{name}, {count} maps",
        place.maps,
        name = name
    );
    (label, hint)
}

/// The places found in the index, to put the camera over one.
fn go_to(ui: &mut egui::Ui, state: &mut AppState, viewer: &mut Viewer) {
    let Some(index) = viewer.index.clone() else {
        return;
    };
    let held = state.panels.get::<Places>();
    // ⚠ The vocabulary is read on a worker after the first ask: the places are named again once it is served.
    let vocabulary = crate::state::gta6_names::vocabulary();
    let of = (
        std::sync::Arc::as_ptr(&index) as usize,
        std::sync::Arc::as_ptr(&vocabulary) as usize,
    );
    if held.of != of {
        // A pass over every map at rest, once an index and a vocabulary.
        (held.of, held.found) = (of, places::of_index(&index));
    }
    if held.found.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        let chosen = egui::ComboBox::from_id_salt("world_viewer_go_to")
            .selected_text(tr!("Choose a place"))
            .show_ui(ui, |ui| {
                let mut chosen = None;
                for place in &held.found {
                    let (label, hint) = place_label(place);
                    if ui
                        .selectable_label(false, label)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        chosen = Some(place.pose());
                    }
                }
                chosen
            });
        named(chosen.response.clone(), tr!("Go to"));
        ui.label(tr!("Go to"));
        if let Some(pose) = chosen.inner.flatten() {
            pose.put(&mut viewer.camera);
            viewer.cuts += 1;
            ui.ctx().request_repaint();
        }
    });
}

/// Where the camera stands, how fast it flies and how far the levels of detail reach.
fn camera(ui: &mut egui::Ui, state: &mut AppState, viewer: &mut Viewer) {
    ui.strong(tr!("Camera"));
    go_to(ui, state, viewer);
    let overview = state.panels.get::<map::Overview>();
    map::draw(ui, overview, viewer);
    // Inside, the interior and its room say where the camera is; outside, the map under it and the ground.
    match viewer.inside() {
        Some(inside) => said(ui, &where_inside(inside)),
        None if viewer.index.is_some() => {
            let words = map::place_words(overview, viewer);
            if let Some(place) = overview.place.clone() {
                said(ui, &tr!("In {place}", place = place));
            }
            said(ui, &words);
        }
        None => {}
    }
    ui.horizontal_wrapped(|ui| {
        let position = &mut viewer.camera.position;
        for (axis, value) in [
            ("X", &mut position.x),
            ("Y", &mut position.y),
            ("Z", &mut position.z),
        ] {
            let field = ui.add(
                egui::DragValue::new(value)
                    .speed(1.0)
                    .max_decimals(1)
                    .prefix(format!("{axis} "))
                    .suffix(" m"),
            );
            named(
                field,
                &tr!("Camera position {axis}, in metres", axis = axis),
            );
        }
    });
    let speed = wide(
        ui,
        tr!("Speed (m/s)"),
        egui::Slider::new(&mut viewer.camera.speed, SPEEDS).logarithmic(true),
    );
    named(speed, tr!("How fast the camera flies, in metres a second"));
    let scale = wide(
        ui,
        tr!("Level-of-detail scale"),
        egui::Slider::new(&mut viewer.lod_scale, LOD_SCALES),
    );
    named(
        scale,
        tr!("The scale every level-of-detail distance is multiplied by"),
    );
}

/// The game's modes and the script-managed groups that stream.
fn streams(ui: &mut egui::Ui, state: &mut AppState, viewer: &mut Viewer) {
    use or_title::gta6_mount::Mode;
    ui.strong(tr!("What streams"));
    // The two modes side by side where the panel is wide enough.
    ui.horizontal_wrapped(|ui| {
        for (mode, label, hint) in [
            (
                Mode::Story,
                tr!("Story pack"),
                tr!("Stream the maps the story pack adds"),
            ),
            (
                Mode::Minigames,
                tr!("Shared minigames"),
                tr!("Stream the maps the shared minigames add"),
            ),
        ] {
            let mut on = viewer.enabled.modes & mode.bit() != 0;
            if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
                viewer.enabled.modes ^= mode.bit();
            }
        }
    });
    let Some(index) = viewer.index.clone() else {
        return;
    };
    let groups = index.groups();
    if groups.is_empty() {
        said(ui, tr!("The index holds no script-managed group"));
        return;
    }
    let title = tr!(
        "Script-managed groups: {on} of {groups} on",
        on = count(viewer.enabled.groups.len() as u64),
        groups = count(groups.len() as u64),
    );
    egui::CollapsingHeader::new(title)
        .id_salt("world_viewer_groups")
        .show(ui, |ui| {
            said(
                ui,
                match viewer.groups_chosen {
                    false => tr!("The game's own at this hour; a group ticked streams its maps in"),
                    true => tr!("As you chose them; the hour no longer changes them"),
                },
            );
            let proven = ui
                .checkbox(&mut viewer.groups_proven, tr!("Only what the data proves"))
                .on_hover_text(tr!(
                    "Leave out what a fresh game is taken to have beyond the data: requests of its scripts and the flags it starts with"
                ));
            // The groups follow the hour again, by what is now taken of the game.
            if proven.changed() {
                viewer.groups_chosen = false;
            }
            if viewer.groups_chosen {
                let own = ui.button(tr!("The game's own groups")).on_hover_text(tr!(
                    "Switch on the groups the game's world state switches on at this hour, in place of the ones chosen"
                ));
                if own.clicked() {
                    viewer.enabled.groups =
                        index.default_on_in(viewer.hour, viewer.enabled.modes, viewer.groups_proven);
                    viewer.groups_chosen = false;
                }
            }
            let filter = &mut state.panels.get::<GroupFilter>().0;
            let field = egui::TextEdit::singleline(filter)
                .hint_text(tr!("Filter groups by name"))
                .desired_width(f32::INFINITY);
            named(ui.add(field), tr!("Filter groups by name"));
            let needle = filter.to_lowercase();
            let vocabulary = crate::state::gta6_names::vocabulary();
            let mut rows: Vec<(&gta6_worldview::index::Group, String)> = (groups.iter())
                .map(|group| (group, group_name(&vocabulary, group)))
                .filter(|(_, name)| needle.is_empty() || name.to_lowercase().contains(&needle))
                .collect();
            // The groups the vocabulary names come first, then those shown by their hash.
            rows.sort_by_cached_key(|(group, name)| (name.starts_with("hash_"), group.id));
            if rows.is_empty() {
                said(ui, tr!("No group's name holds that text"));
                return;
            }
            let row_height = ui.spacing().interact_size.y;
            egui::ScrollArea::vertical()
                .id_salt("world_viewer_group_rows")
                // ⚠ Inside the panel's own scroll area a list takes only the height left on screen, a few rows near the bottom, unless it is given its least.
                .max_height(row_height * GROUP_ROWS)
                .min_scrolled_height(row_height * GROUP_ROWS)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, rows.len(), |ui, range| {
                    for (group, name) in &rows[range] {
                        let mut on = viewer.enabled.groups.contains(&group.id);
                        let mut label = tr_n!("{name}, {count} map", "{name}, {count} maps", group.maps, name = name).to_string();
                        if group.by_inference(viewer.hour, viewer.enabled.modes) {
                            label = tr!("{label} (inferred)", label = label).to_string();
                        }
                        if ui.checkbox(&mut on, label).changed() {
                            // ⚠ A list the person changed is theirs: the hour no longer resets it.
                            viewer.groups_chosen = true;
                            match on {
                                true => viewer.enabled.groups.push(group.id),
                                false => viewer.enabled.groups.retain(|id| *id != group.id),
                            }
                        }
                    }
                });
        });
}

/// The Capture section: a clean frame, the camera as a line a shots script reads, and a line to jump to.
fn capture(ui: &mut egui::Ui, state: &mut AppState, viewer: &mut Viewer) {
    let held = state.panels.get::<Capture>();
    ui.checkbox(&mut held.clean, tr!("Clean frame")).on_hover_text(tr!(
        "Draw the world alone over the whole window, with no line and no panel over it; Escape brings them back"
    ));
    let copy = ui.button(tr!("Copy the camera")).on_hover_text(tr!(
        "Copy where the camera stands and looks, as a line a shots script reads and the field below takes"
    ));
    if copy.clicked() {
        ui.ctx().copy_text(Pose::of(&viewer.camera).line());
    }
    let field = egui::TextEdit::singleline(&mut held.line)
        .hint_text(tr!("Paste a camera line, then press Enter"))
        .desired_width(f32::INFINITY);
    let field = named(ui.add(field), tr!("A camera line to jump to"));
    if field.changed() {
        held.refused = false;
    }
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        match Pose::read(&held.line) {
            Ok(pose) => {
                pose.put(&mut viewer.camera);
                viewer.cuts += 1;
                held.line.clear();
                ui.ctx().request_repaint();
            }
            Err(_) => held.refused = true,
        }
    }
    if held.refused {
        ui.colored_label(
            theme::WARNING,
            tr!("That is not a camera line: copy one with Copy the camera"),
        );
    }
}

/// What is streamed in around the camera and on the device.
fn streaming(ui: &mut egui::Ui, viewer: &Viewer, drawn: Drawn) {
    let s = viewer.streaming();
    said(
        ui,
        &tr!(
            "{loaded} of {wanted} maps streamed in, {waiting} waited for; {nodes} entities and lists",
            loaded = count(s.maps_loaded as u64),
            wanted = count(s.maps_wanted as u64),
            waiting = count(s.maps_waiting as u64),
            nodes = count(s.nodes as u64),
        ),
    );
    said(
        ui,
        &tr!(
            "{seen} of {selected} instances in view, in {draws} draws",
            seen = count(drawn.seen as u64),
            selected = count(drawn.selected as u64),
            draws = count(drawn.draws as u64),
        ),
    );
    said(
        ui,
        &tr!(
            "{models} models on the device ({megabytes} MB), {waiting} waited for",
            models = count(drawn.models as u64),
            megabytes = count(drawn.megabytes),
            waiting = count(viewer.models_waiting as u64),
        ),
    );
    said(
        ui,
        &tr!(
            "{textures} textures on the device, {unread} not read, {waiting} waited for",
            textures = count(drawn.textures as u64),
            unread = count(drawn.unread as u64),
            waiting = count(viewer.textures_waiting as u64),
        ),
    );
    if let Some(lights) = viewer.lights.as_ref().map(|l| l.report()) {
        said(
            ui,
            &tr!(
                "Lights of {maps} maps: {placed} placed, {lod} level-of-detail, {distant} distant, {probes} reflection probes; {drawn} drawn",
                maps = count(lights.maps_read as u64),
                placed = count(lights.placed as u64),
                lod = count(lights.lod as u64),
                distant = count(lights.distant as u64),
                probes = count(lights.probes as u64),
                drawn = count(lights.drawn as u64),
            ),
        );
    }
    if drawn.lit_by_the_game {
        said(ui, tr!("Lit by the game's own passes"));
    }
}

/// What the mount found: the documents read, the archives bound and the maps that stream at rest.
fn mount(ui: &mut egui::Ui, viewer: &mut Viewer) {
    let Some(index) = viewer.index.clone() else {
        said(
            ui,
            match viewer.building() {
                Some(_) => tr!("The registration documents and the level's maps are being read"),
                None => tr!("No index of this install"),
            },
        );
        return;
    };
    let r = &index.report;
    said(
        ui,
        &tr!(
            "{packs} packs; the level's change sets: {read} read, {unread} not readable",
            packs = r.packs,
            read = r.sets_read,
            unread = r.sets_unread,
        ),
    );
    said(
        ui,
        &tr!(
            "{bound} archives bound to the {named} the level names",
            bound = count(r.bound_archives),
            named = count(r.named_archives),
        ),
    );
    said(
        ui,
        &tr!(
            "{registered} of {maps} maps are the level's: {rest} stream at rest, {scripted} are script managed, {empty} hold nothing",
            registered = count(r.registered),
            maps = count(r.maps),
            rest = count(r.at_rest),
            scripted = count(r.script_managed),
            empty = count(r.empty),
        ),
    );
    said(
        ui,
        &tr!(
            "{story} maps are the story pack's and {minigames} the shared minigames'; {groups} script-managed groups",
            story = count(r.story),
            minigames = count(r.minigames),
            groups = count(r.groups),
        ),
    );
    said(
        ui,
        &match r.by_table {
            0 => tr!("Bound by block maps: no key snapshot was found").to_string(),
            _ => tr!(
                "Bound by the archives' tables; {ciphered} of the level's change sets are ciphered",
                ciphered = r.sets_ciphered,
            ),
        },
    );
    said(
        ui,
        &tr!(
            "{archetypes} archetypes, {models} models; {unread} bodies did not read",
            archetypes = count(r.archetypes),
            models = count(r.models),
            unread = count(r.unread),
        ),
    );
    rebuild_button(ui, viewer);
}
