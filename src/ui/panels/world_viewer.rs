//! The World screen: GTA VI's world as the game streams it at rest, flown through with a camera
//! that has no pivot; the panel beside it is `world_viewer_side`.
//!
//! ⚠ No orbit: a drag turns the camera where it stands, the keys move it, and the wheel sets how
//! fast. The keys are read only while the pointer is over the view or the view holds the focus.

use std::path::{Path, PathBuf};

use glam::Vec3;
use or_renderer::FlyCamera;

use super::world_viewer_prefs::WorldViewerPrefs;
use super::world_viewer_side::draw_side as draw_own_side;
use crate::render::RenderBridge;
use crate::state::gta6_world::worldview::{self as gta6_worldview, Viewer};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::theme;

/// The least the view is drawn when the dock is far too short.
pub(super) const VIEW_MIN: f32 = 96.0;
/// The steepest a camera line looks up or down, in degrees: the fly camera's own bound.
const MOST_PITCH: f32 = 89.0;
/// The vertical fields of view a camera line may state, in degrees.
const FIELDS_OF_VIEW: std::ops::RangeInclusive<f32> = 5.0..=150.0;
/// The area and the layer a clean frame is sensed and painted through.
pub(super) const CLEAN: &str = "world_viewer_clean";

/// Where a camera line puts the camera: metres, then degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    /// Degrees about Z: 0 looks along +Y, and a growing angle turns towards −X.
    pub yaw: f32,
    /// Degrees above the horizon.
    pub pitch: f32,
    /// The vertical field of view in degrees; `None` leaves the camera's own.
    pub fov: Option<f32>,
}

impl Pose {
    /// The pose of `camera`, its yaw within one turn.
    pub fn of(camera: &FlyCamera) -> Self {
        Self {
            position: camera.position,
            yaw: camera.yaw.to_degrees().rem_euclid(360.0),
            pitch: camera.pitch.to_degrees(),
            fov: Some(camera.fov_y.to_degrees()),
        }
    }

    /// Put `camera` where the pose says, leaving its speed and its planes.
    pub fn put(&self, camera: &mut FlyCamera) {
        camera.position = self.position;
        camera.yaw = self.yaw.to_radians();
        camera.pitch = self.pitch.to_radians();
        if let Some(fov) = self.fov {
            camera.fov_y = fov.to_radians();
        }
    }

    /// The line a shots script and the Capture section read back: `world camera X Y Z YAW PITCH FOV`.
    pub fn line(&self) -> String {
        let Vec3 { x, y, z } = self.position;
        let (yaw, pitch) = (self.yaw, self.pitch);
        match self.fov {
            Some(fov) => format!("world camera {x:.2} {y:.2} {z:.2} {yaw:.2} {pitch:.2} {fov:.2}"),
            None => format!("world camera {x:.2} {y:.2} {z:.2} {yaw:.2} {pitch:.2}"),
        }
    }

    /// The pose `line` states, with or without its leading `world camera`.
    pub fn read(line: &str) -> Result<Self, String> {
        let mut words: Vec<&str> = line.split_whitespace().collect();
        if words.starts_with(&["world", "camera"]) {
            words.drain(..2);
        }
        let mut numbers = Vec::with_capacity(words.len());
        for word in words {
            let number = word.parse::<f32>().ok().filter(|n| n.is_finite());
            numbers.push(number.ok_or_else(|| format!("`{word}` is not a number"))?);
        }
        let (x, y, z, yaw, pitch, fov) = match numbers[..] {
            [x, y, z, yaw, pitch] => (x, y, z, yaw, pitch, None),
            [x, y, z, yaw, pitch, fov] => (x, y, z, yaw, pitch, Some(fov)),
            _ => {
                return Err(format!(
                    "a camera is X Y Z YAW PITCH and an optional field of view, found {} number(s)",
                    numbers.len()
                ))
            }
        };
        if pitch.abs() > MOST_PITCH {
            return Err(format!(
                "a pitch is within {MOST_PITCH} degrees of the horizon"
            ));
        }
        if fov.is_some_and(|fov| !FIELDS_OF_VIEW.contains(&fov)) {
            return Err(format!(
                "a field of view is from {} to {} degrees",
                FIELDS_OF_VIEW.start(),
                FIELDS_OF_VIEW.end()
            ));
        }
        Ok(Self {
            position: Vec3::new(x, y, z),
            yaw,
            pitch,
            fov,
        })
    }
}

/// What the Capture section holds between frames.
#[derive(Default)]
pub struct Capture {
    /// The view is drawn alone over the whole window.
    pub clean: bool,
    /// The camera line typed or pasted into the field.
    pub(super) line: String,
    /// The line last entered was not a camera line.
    pub(super) refused: bool,
    /// The install the kept controls were last given to.
    kept_for: Option<PathBuf>,
}

impl crate::ui::panel_state::PanelState for Capture {
    const KEY: &'static str = "world-capture";
}

/// The folder of the GTA VI install the World screen shows, where the world it shows is GTA VI's.
fn install(state: &AppState) -> Option<std::path::PathBuf> {
    let world = crate::state::world_session::wanted(&state.prefs, &state.world)?;
    world.game.is_gta_vi().then_some(world.install)
}

/// Whether the World screen has an install to show.
pub fn offered(state: &AppState) -> bool {
    crate::state::world_session::wanted(&state.prefs, &state.world).is_some()
}

/// Whether the world the World screen shows is drawn through the seam, by the sections its capabilities state, and not by GTA VI's own screen.
pub(crate) fn fed(state: &AppState) -> bool {
    crate::state::world_session::wanted(&state.prefs, &state.world)
        .is_some_and(|world| !world.game.is_gta_vi())
}

fn said(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::TEXT_SECONDARY));
}

/// How far the keys held move the camera: forward, to the right and up, each −1 to 1, and whether fast.
pub(super) fn keys(ui: &egui::Ui) -> (f32, f32, f32, bool) {
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

/// The panel beside the view: the sections the world's capabilities state, or GTA VI's own.
pub fn draw_side(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    match fed(state) {
        true => super::world_viewer_plain::draw_side(ui, state, bridge),
        false => draw_own_side(ui, state, bridge),
    }
}

/// The view: the world through the fly camera.
pub fn draw_view(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    if fed(state) {
        return super::world_viewer_plain::draw_view(ui, state, bridge);
    }
    let Some(root) = install(state) else {
        ui.vertical_centered(|ui| {
            ui.add_space(theme::SPACING_SECTION);
            said(
                ui,
                tr!("Set the GTA VI folder in Settings ▸ Games to fly through its world"),
            );
            if ui.button(tr!("Settings ▸ Games…")).clicked() {
                super::settings_window::open_settings_at(
                    state,
                    super::settings::SettingsCategory::Games,
                );
            }
        });
        return;
    };
    let held = state.panels.get::<Capture>();
    // A clean frame draws no control, so Escape is the way back from it.
    if held.clean && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        held.clean = false;
    }
    let clean = held.clean;
    // The session of this install is held: another game's, where one was, ends here.
    state.world_session();
    let mut viewer = gta6_worldview::viewer();
    viewer.open(&root);
    viewer.poll();
    remember(ui, state, &mut viewer, &root);

    let (rect, response) = match clean {
        true => whole_window(ui.ctx()),
        false => {
            let size = egui::vec2(ui.available_width(), ui.available_height().max(VIEW_MIN));
            ui.allocate_exact_size(size, egui::Sense::click_and_drag())
        }
    };
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
    let indexed = viewer.index.is_some();
    let loading = Loading::of(&viewer);
    let failed = (viewer.failed.clone())
        .map(|why| tr!("The GTA VI world was not indexed: {why}", why = why));
    let speed = viewer.camera.speed;
    drop(viewer);
    // The frame is drawn again while the camera moves and while anything is still arriving.
    if moving || loading.any() || building.is_some() {
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
    match bridge.filter(|_| indexed) {
        // ⚠ With no index held the frame holds only the water and the ground at the origin, which reads as the world: a plain backdrop until it is.
        Some(bridge) => {
            let ppp = ui.ctx().pixels_per_point();
            bridge.set_desired_size(
                (rect.width() * ppp).round() as u32,
                (rect.height() * ppp).round() as u32,
            );
            bridge.refresh_world(state.drivers.time_of_day.days, state.drivers.weather.state);
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
    if clean {
        return;
    }
    let line = match (building, failed) {
        (Some(done), _) => tr!(
            "Indexing the GTA VI world: {percent} %",
            percent = (done * 100.0).round() as u32
        ),
        (None, Some(why)) => why,
        (None, None) => tr!(
            "Drag to look, W A S D or Z Q S D to move, E and C up and down, Shift faster; the wheel sets the speed: {speed} m/s",
            speed = speed.round() as u32
        ),
    };
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
    match indexed {
        true => chip(
            line,
            rect.left_top() + egui::vec2(8.0, 6.0),
            egui::Align2::LEFT_TOP,
        ),
        false => chip(line, rect.center(), egui::Align2::CENTER_CENTER),
    }
    // What is still on its way, in the corner below; nothing is said once nothing is.
    if let Some(said) = loading.said().filter(|_| building.is_none()) {
        let corner = rect.left_bottom() + egui::vec2(8.0, -6.0);
        chip(said, corner, egui::Align2::LEFT_BOTTOM);
    }
}

/// What the world still waits for: maps, models, textures, maps' lights, and the water's ground and fog.
struct Loading {
    counts: [usize; 4],
    water: bool,
}

impl Loading {
    fn of(viewer: &Viewer) -> Self {
        Self {
            counts: [
                viewer.streaming().maps_waiting,
                viewer.models_waiting,
                viewer.textures_waiting,
                viewer.lights.as_ref().map_or(0, |lights| lights.waiting()),
            ],
            water: viewer.water_reading(),
        }
    }

    fn any(&self) -> bool {
        self.water || self.counts.iter().any(|n| *n > 0)
    }

    /// The readout, `None` once nothing is still loading.
    fn said(&self) -> Option<String> {
        let [maps, models, textures, lights] = self.counts;
        let mut parts = Vec::new();
        if maps > 0 {
            parts.push(tr_n!("{count} map", "{count} maps", maps));
        }
        if models > 0 {
            parts.push(tr_n!("{count} model", "{count} models", models));
        }
        if textures > 0 {
            parts.push(tr_n!("{count} texture", "{count} textures", textures));
        }
        if lights > 0 {
            parts.push(tr_n!(
                "the lights of {count} map",
                "the lights of {count} maps",
                lights
            ));
        }
        if self.water {
            parts.push(tr!("the ground and fog under the water").to_string());
        }
        (!parts.is_empty()).then(|| tr!("Still loading: {what}", what = parts.join(", ")))
    }
}

/// The whole window as the view, sensed over everything the dock draws.
pub(super) fn whole_window(ctx: &egui::Context) -> (egui::Rect, egui::Response) {
    let window = ctx.content_rect();
    egui::Area::new(egui::Id::new(CLEAN))
        .order(egui::Order::Foreground)
        .fixed_pos(window.min)
        .show(ctx, |ui| {
            ui.allocate_exact_size(window.size(), egui::Sense::click_and_drag())
        })
        .inner
}

/// Give a newly opened install the controls kept from the last run, and keep them again once changed.
fn remember(ui: &egui::Ui, state: &mut AppState, viewer: &mut Viewer, root: &Path) {
    let held = state.panels.get::<Capture>();
    if held.kept_for.as_deref() != Some(root) {
        held.kept_for = Some(root.to_path_buf());
        let kept = state.prefs.world_viewer.clone();
        kept.put(viewer);
        // ⚠ The Time bar and the world say one hour, as the Hour slider has them.
        let day = &mut state.drivers.time_of_day;
        day.set_hour(kept.hour);
        day.playing = false;
        state.set_weather_state(kept.weather());
        return;
    }
    // A slider being dragged is kept once it is let go, not a file written a frame.
    if ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = WorldViewerPrefs::of(viewer, state.drivers.weather.state);
    if now != state.prefs.world_viewer {
        state.prefs.world_viewer = now;
        crate::state::save_prefs(&state.prefs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> FlyCamera {
        FlyCamera {
            position: Vec3::new(-1234.5, 678.25, 91.0),
            yaw: 2.0,
            pitch: -0.35,
            fov_y: 60f32.to_radians(),
            ..FlyCamera::default()
        }
    }

    #[test]
    fn a_copied_camera_line_puts_another_camera_where_the_first_stood() {
        let from = camera();
        let line = Pose::of(&from).line();
        assert!(line.starts_with("world camera "), "{line}");
        let mut to = FlyCamera {
            speed: 7.0,
            ..FlyCamera::default()
        };
        Pose::read(&line).expect("its own line").put(&mut to);
        // A line holds two decimals of a metre and of a degree.
        assert!(to.position.distance(from.position) < 0.01, "{line}");
        for (got, want) in [
            (to.yaw, from.yaw),
            (to.pitch, from.pitch),
            (to.fov_y, from.fov_y),
        ] {
            assert!(
                (got - want).abs() < 0.01f32.to_radians(),
                "{line}: {got} for {want}"
            );
        }
        assert_eq!(to.speed, 7.0, "a line says nothing of the speed");
        assert_eq!(
            Pose::of(&to).line(),
            line,
            "a line read and copied again is the same line"
        );
    }

    #[test]
    fn a_camera_line_reads_with_or_without_its_step_and_its_field_of_view() {
        let with = Pose {
            position: Vec3::new(1.0, 2.0, 3.0),
            yaw: 90.0,
            pitch: -10.0,
            fov: Some(50.0),
        };
        assert_eq!(Pose::read("world camera 1 2 3 90 -10 50"), Ok(with));
        assert_eq!(Pose::read("  1 2 3 90 -10 50 "), Ok(with));
        let without = Pose::read("1 2 3 90 -10").expect("a line with no field of view");
        assert_eq!(without.fov, None);
        let mut camera = camera();
        let own = camera.fov_y;
        without.put(&mut camera);
        assert_eq!(
            camera.fov_y, own,
            "a line with no field of view leaves the camera's own"
        );
        // A yaw of 90 degrees looks along −X: the angles are the fly camera's own, in degrees.
        let pitch = (-10f32).to_radians();
        let ahead = Vec3::new(-pitch.cos(), 0.0, pitch.sin());
        assert!(
            camera.forward().distance(ahead) < 1e-5,
            "{}",
            camera.forward()
        );
    }

    #[test]
    fn a_line_that_is_no_camera_is_refused() {
        for bad in [
            "",
            "world camera",
            "1 2 3 4",
            "1 2 3 4 5 6 7",
            "1 2 three 4 5",
            "1 2 3 4 inf",
            "1 2 3 0 89.5",
            "1 2 3 0 -90",
            "1 2 3 0 0 0",
            "1 2 3 0 0 170",
            "world hour 9",
        ] {
            assert!(Pose::read(bad).is_err(), "`{bad}` should be refused");
        }
        assert!(
            Pose::read("1 2 3 0 89").is_ok(),
            "the bound itself is a pitch"
        );
    }
}
