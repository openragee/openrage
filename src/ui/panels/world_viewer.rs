//! The World screen: GTA VI's world as the game streams it at rest, flown through with a camera
//! that has no pivot, and the panel that says what the mount found and what is streaming.
//!
//! ⚠ No orbit: a drag turns the camera where it stands, the keys move it, and the wheel sets how
//! fast. The keys are read only while the pointer is over the view or the view holds the focus.

use crate::render::RenderBridge;
use crate::state::gta6_worldview::{self, Viewer};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::theme;

/// The least the view is drawn when the dock is far too short.
const VIEW_MIN: f32 = 96.0;

/// The folder of the GTA VI install the app was given, where it still stands.
fn install(state: &AppState) -> Option<std::path::PathBuf> {
    (state.prefs.game_paths.get(or_title::Game::GtaVI))
        .filter(|root| root.is_dir())
        .cloned()
}

/// Whether the World screen has an install to show.
pub fn offered(state: &AppState) -> bool {
    install(state).is_some()
}

fn said(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::TEXT_SECONDARY));
}

/// How far the keys held move the camera: forward, to the right and up, each −1 to 1, and whether fast.
fn keys(ui: &egui::Ui) -> (f32, f32, f32, bool) {
    use egui::Key;
    ui.input(|i| {
        let held = |keys: &[Key]| keys.iter().any(|k| i.key_down(*k)) as i32 as f32;
        // W A S D, and Z Q S D of a keyboard laid out the other way: a key means one thing in both.
        let along = held(&[Key::W, Key::Z, Key::ArrowUp]) - held(&[Key::S, Key::ArrowDown]);
        let across = held(&[Key::D, Key::ArrowRight]) - held(&[Key::A, Key::Q, Key::ArrowLeft]);
        let up = held(&[Key::E, Key::Space]) - held(&[Key::C]);
        (along, across, up, i.modifiers.shift)
    })
}

/// The view: the world through the fly camera.
pub fn draw_view(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    let Some(root) = install(state) else {
        ui.vertical_centered(|ui| {
            ui.add_space(theme::SPACING_SECTION);
            said(
                ui,
                tr!("Set the GTA VI folder in Settings ▸ Games to fly through its world"),
            );
        });
        return;
    };
    let mut viewer = gta6_worldview::viewer();
    viewer.open(&root);
    viewer.poll();

    let size = egui::vec2(ui.available_width(), ui.available_height().max(VIEW_MIN));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = named(response, tr!("The GTA VI world, flown through"));
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    if response.dragged() {
        let by = response.drag_delta();
        viewer.camera.look(by.x, by.y);
    }
    let over = response.hovered() || response.has_focus();
    let mut moving = response.dragged();
    if over {
        // ⚠ The arrows are the camera's: without the lock egui moves the keyboard to the widget beside the view.
        crate::ui::focus_ring::reads_typed_text(ui.ctx(), response.id);
        let (along, across, up, fast) = keys(ui);
        if (along, across, up) != (0.0, 0.0, 0.0) {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            viewer.camera.fly(along, across, up, fast, dt);
            moving = true;
        }
    }
    if response.hovered() {
        let notches = ui.input(|i| i.smooth_scroll_delta.y) / 50.0;
        if notches != 0.0 {
            viewer.camera.scale_speed(notches);
        }
    }
    let building = viewer.building();
    let waiting = viewer.streaming().maps_waiting + viewer.models_waiting + viewer.textures_waiting;
    let failed = viewer.failed.clone();
    let speed = viewer.camera.speed;
    drop(viewer);
    // The frame is drawn again while the camera moves and while anything is still arriving.
    if moving || waiting > 0 || building.is_some() {
        ui.ctx().request_repaint();
    }

    let painter = ui.painter_at(rect);
    match bridge {
        Some(bridge) => {
            let ppp = ui.ctx().pixels_per_point();
            bridge.set_desired_size(
                (rect.width() * ppp).round() as u32,
                (rect.height() * ppp).round() as u32,
            );
            bridge.refresh_world();
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
        None => {
            painter.rect_filled(rect, 0.0, theme::BG_L2);
        }
    }
    let line = match (building, failed) {
        (Some(done), _) => tr!(
            "Indexing the GTA VI world: {percent} %",
            percent = (done * 100.0).round() as u32
        ),
        (None, Some(why)) => tr!("The GTA VI world was not indexed: {why}", why = why),
        (None, None) => tr!(
            "Drag to look, W A S D or Z Q S D to move, E and C up and down, Shift faster; the wheel sets the speed: {speed} m/s",
            speed = speed.round() as u32
        ),
    };
    let at = rect.left_top() + egui::vec2(8.0, 6.0);
    let font = egui::FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(line, font, theme::TEXT_PRIMARY.get());
    let back = egui::Rect::from_min_size(at, galley.size()).expand(3.0);
    painter.rect_filled(
        back,
        egui::CornerRadius::same(theme::RADIUS_SMALL),
        theme::BG_L2,
    );
    painter.galley(at, galley, theme::TEXT_PRIMARY.get());
}

fn count(n: u64) -> String {
    n.to_string()
}

/// The panel beside the view: what the mount found, what is streaming, and the camera.
pub fn draw_side(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    if !offered(state) {
        said(
            ui,
            tr!("Set the GTA VI folder in Settings ▸ Games to fly through its world"),
        );
        return;
    }
    let drawn = bridge.map(|b| b.world_drawn()).unwrap_or_default();
    let mut viewer = gta6_worldview::viewer();
    egui::ScrollArea::vertical().show(ui, |ui| {
        mount(ui, &viewer);
        ui.separator();
        ui.strong(tr!("Streaming"));
        let s = viewer.streaming();
        said(
            ui,
            &tr!(
                "{loaded} of {wanted} maps streamed in, {waiting} waited for; {nodes} entities and lists",
                loaded = s.maps_loaded,
                wanted = s.maps_wanted,
                waiting = s.maps_waiting,
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
                megabytes = count(drawn.bytes >> 20),
                waiting = viewer.models_waiting,
            ),
        );
        said(
            ui,
            &tr!(
                "{textures} textures on the device, {unread} not read, {waiting} waited for",
                textures = count(drawn.textures as u64),
                unread = count(drawn.textures_unread as u64),
                waiting = viewer.textures_waiting,
            ),
        );
        said(
            ui,
            match drawn.lit_by_the_game {
                true => tr!("Lit by the game's own passes"),
                false => tr!("Lit by the plain pass: no game shader pack is read"),
            },
        );
        ui.separator();
        ui.strong(tr!("Camera"));
        ui.horizontal(|ui| {
            let position = &mut viewer.camera.position;
            for (axis, value) in [("X", &mut position.x), ("Y", &mut position.y), ("Z", &mut position.z)] {
                let field = ui.add(egui::DragValue::new(value).speed(1.0).prefix(format!("{axis} ")));
                named(field, &tr!("Camera position {axis}, in metres", axis = axis));
            }
        });
        let speed = ui.add(
            egui::Slider::new(&mut viewer.camera.speed, 1.0..=2000.0)
                .logarithmic(true)
                .text(tr!("Speed (m/s)")),
        );
        named(speed, tr!("How fast the camera flies, in metres a second"));
        let scale = ui.add(
            egui::Slider::new(&mut viewer.lod_scale, 0.5..=4.0).text(tr!("Level-of-detail scale")),
        );
        named(
            scale,
            tr!("The scale every level-of-detail distance is multiplied by"),
        );
        ui.separator();
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
    });
}

/// What the mount found: the documents read, the archives bound and the maps that stream at rest.
fn mount(ui: &mut egui::Ui, viewer: &Viewer) {
    ui.strong(tr!("Mount"));
    let Some(index) = &viewer.index else {
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
            "{archetypes} archetypes, {models} models; {unread} bodies did not read",
            archetypes = count(r.archetypes),
            models = count(r.models),
            unread = count(r.unread),
        ),
    );
}
