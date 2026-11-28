//! The plan: the shown set seen from above, each entity its box's footprint coloured by level of detail, drawn by the egui painter so it
//! needs no model, and an entity picked, dragged along the ground and let go is one placement written to its map file.

use glam::{Vec2 as World, Vec3};
use or_maps::{Placed, Scene};
use or_world::gizmo::{footprint, Obb};

use super::MapsUi;
use crate::render::map_overlay::{is_full_detail, level_colour};
use crate::state::workbench::{Pick, PickKind, PlanView};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::theme;

/// The fewest and the most pixels a metre may be on the plan.
const ZOOM: (f32, f32) = (0.02, 400.0);

/// A placement being dragged on the plan.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlanDrag {
    index: usize,
    /// Where the pointer was in the world when the drag began, and the pose it began with.
    from: World,
    start: or_world::transform::Pose,
}

/// The world to the screen and back for a plan laid out over `rect`.
#[derive(Clone, Copy)]
struct Map {
    rect: egui::Rect,
    view: PlanView,
}

impl Map {
    fn to_screen(self, world: World) -> egui::Pos2 {
        let s = self.view.pixels_per_metre;
        self.rect.center()
            + egui::vec2(
                (world.x - self.view.centre[0]) * s,
                -(world.y - self.view.centre[1]) * s,
            )
    }

    fn to_world(self, at: egui::Pos2) -> World {
        let s = self.view.pixels_per_metre;
        let d = at - self.rect.center();
        World::new(self.view.centre[0] + d.x / s, self.view.centre[1] - d.y / s)
    }
}

fn colour(level: Option<&str>, declared: bool) -> egui::Color32 {
    let c = if declared {
        level_colour(level)
    } else {
        [0.55, 0.58, 0.66, 1.0]
    };
    egui::Color32::from_rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

/// Whether `p` is inside the convex polygon `hull`.
fn inside(hull: &[World], p: World) -> bool {
    if hull.len() < 3 {
        return false;
    }
    hull.iter()
        .zip(hull.iter().cycle().skip(1))
        .all(|(a, b)| (*b - *a).perp_dot(p - *a) >= -1e-6)
}

fn area(hull: &[World]) -> f32 {
    hull.iter()
        .zip(hull.iter().cycle().skip(1))
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f32>()
        .abs()
        / 2.0
}

fn hull_of(obb: &Obb) -> Vec<World> {
    footprint(&obb.corners())
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    if !state.shows_maps() {
        crate::ui::panels::empty_state::draw(ui, state);
        return;
    }
    let mut shown = state.panels.take::<MapsUi>();
    shown.drop_plan = Some((ui.max_rect(), ui.ctx().cumulative_pass_nr()));
    tools(ui, state);
    super::world::tools(ui, state, &mut shown.world);
    canvas(ui, state, &mut shown);
    state.panels.put(shown);
}

fn tools(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        if named(ui.button(tr!("Fit")), tr!("Fit the plan to the map")).clicked() {
            state.workbench.maps.plan.fitted = false;
        }
        let lod = &mut state.workbench.maps.show_lod;
        let response = ui.checkbox(lod, tr!("Levels of detail"));
        named(
            response,
            tr!("Show the entities of the levels of detail below the full one"),
        );
    });
}

fn canvas(ui: &mut egui::Ui, state: &mut AppState, shown: &mut MapsUi) {
    let size = ui.available_size().max(egui::vec2(60.0, 60.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = named(response, tr!("Plan of the map set, seen from above"));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::BG_L1.get());
    let mut view = state.workbench.maps.plan;
    let Some(scene) = state.shown_scene() else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr!("Reading the maps…"),
            egui::FontId::default(),
            theme::TEXT_SECONDARY.get(),
        );
        return;
    };
    if scene.placed.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr!("This set places nothing yet"),
            egui::FontId::default(),
            theme::TEXT_SECONDARY.get(),
        );
        return;
    }
    if !view.fitted {
        view = fitted(scene, rect);
    }
    let mut map = Map { rect, view };

    // Zoom about the pointer, pan with the middle button or with shift.
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            if let Some(at) = response.hover_pos() {
                let before = map.to_world(at);
                let factor = (scroll / 200.0).exp();
                map.view.pixels_per_metre =
                    (map.view.pixels_per_metre * factor).clamp(ZOOM.0, ZOOM.1);
                let after = map.to_world(at);
                map.view.centre[0] += before.x - after.x;
                map.view.centre[1] += before.y - after.y;
            }
        }
    }
    let selected = state
        .workbench
        .maps
        .selected
        .as_ref()
        .and_then(|p| match p.kind {
            PickKind::Entity(i) => scene.find(&p.source, i),
            PickKind::Archetype(_) => None,
        });
    let show_lod = state.workbench.maps.show_lod;
    let preview = state.workbench.maps.preview;
    let snap = state.workbench.maps.snap;
    let editable = state.places_selected();

    grid(&painter, &map);
    let visible = |placed: &Placed| show_lod || is_full_detail(placed.lod_level.as_deref());
    for (at, placed) in scene.placed.iter().enumerate() {
        if Some(at) == selected || !visible(placed) {
            continue;
        }
        draw_entity(&painter, &map, &placed.obb, placed, false);
    }
    if let Some(at) = selected {
        let placed = &scene.placed[at];
        let mut obb = placed.obb;
        if let Some(p) = preview.filter(|p| p.index == placed.index) {
            obb.position = p.pose.position;
            obb.rotation = p.pose.rotation;
            obb.scale = Vec3::new(p.pose.scale_xy, p.pose.scale_xy, p.pose.scale_z);
        }
        draw_entity(&painter, &map, &obb, placed, true);
    }
    super::scanned::draw_on_plan(&painter, state, |w| map.to_screen(w));
    let seen = (
        map.to_world(rect.left_bottom()),
        map.to_world(rect.right_top()),
    );
    super::world::draw_on_plan(&painter, state, &mut shown.world, seen, |w| {
        map.to_screen(w)
    });
    scale_bar(&painter, &map);

    // What the pointer does.
    let mut pick: Option<Option<Pick>> = None;
    let mut commit: Option<(usize, or_world::transform::Pose)> = None;
    let mut new_preview: Option<(usize, or_world::transform::Pose)> = None;
    let mut ended = false;
    let shift = ui.input(|i| i.modifiers.shift);
    let pan = response.dragged_by(egui::PointerButton::Middle)
        || (response.dragged_by(egui::PointerButton::Primary)
            && shift
            && shown.plan_drag.is_none());
    if pan {
        let d = response.drag_delta();
        map.view.centre[0] -= d.x / map.view.pixels_per_metre;
        map.view.centre[1] += d.y / map.view.pixels_per_metre;
    }
    if response.drag_started_by(egui::PointerButton::Primary) && !shift && editable {
        if let (Some(at), Some(sel)) = (response.interact_pointer_pos(), selected) {
            let world = map.to_world(at);
            let placed = &scene.placed[sel];
            if inside(&hull_of(&placed.obb), world) {
                shown.plan_drag = Some(PlanDrag {
                    index: sel,
                    from: world,
                    start: crate::state::workbench::pose_of_box(&placed.obb),
                });
            }
        }
    }
    if let Some(drag) = shown.plan_drag {
        if let Some(at) = response.interact_pointer_pos() {
            let delta = map.to_world(at) - drag.from;
            let invert = ui.input(|i| i.modifiers.command);
            let held = snap.for_drag(invert);
            let mut pose = drag.start;
            pose.position += Vec3::new(delta.x, delta.y, 0.0);
            if let Some(step) = held.grid {
                pose.position.x = (pose.position.x / step).round() * step;
                pose.position.y = (pose.position.y / step).round() * step;
            }
            new_preview = Some((drag.index, pose));
        }
        if response.drag_stopped() {
            if let Some((index, pose)) = new_preview {
                commit = (pose != drag.start).then_some((index, pose));
            }
            ended = true;
        }
    } else if response.clicked() {
        if let Some(at) = response.interact_pointer_pos() {
            let world = map.to_world(at);
            let hit = scene
                .placed
                .iter()
                .enumerate()
                .filter(|(_, p)| visible(p))
                .filter_map(|(i, p)| {
                    let hull = hull_of(&p.obb);
                    inside(&hull, world).then(|| (i, area(&hull)))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            (shown.world).pick(world.to_array(), map.view.pixels_per_metre, hit.is_some());
            pick = Some(hit.map(|(i, _)| Pick {
                source: scene.placed[i].source.clone(),
                kind: PickKind::Entity(scene.placed[i].index),
            }));
        }
    }
    let module = state.maps_pack().map(|p| p.module.clone());
    let set = state.shown_set().map(|(_, s)| s.id.clone());
    let placed_index = commit.map(|(i, _)| (scene.placed[i].source.clone(), scene.placed[i].index));
    let drag_index = new_preview.map(|(i, _)| scene.placed[i].index);

    state.workbench.maps.plan = map.view;
    state.workbench.maps.plan.fitted = true;
    if ended {
        shown.plan_drag = None;
    }
    if let (Some((_, pose)), Some(index)) = (new_preview, drag_index) {
        if !ended {
            state.workbench.maps.preview = Some(crate::state::workbench::Preview { index, pose });
        }
    }
    if let (Some((_, pose)), Some((source, index))) = (commit, placed_index) {
        state.place_entity_in(
            &source,
            index,
            pose,
            tr!("Moved entity {index} on the plan", index = index).to_string(),
        );
    } else if ended {
        state.workbench.maps.preview = None;
    }
    if let Some(pick) = pick {
        state.pick_shown(module, set, pick);
    }
    if response.double_clicked() {
        state.open_selected_gta6_model();
    }
}

/// The plan fitted so that the whole set shows in `rect`.
fn fitted(scene: &Scene, rect: egui::Rect) -> PlanView {
    let (lo, hi) = scene.bounds.unwrap_or((Vec3::ZERO, Vec3::ONE));
    let span = (hi - lo).truncate().max(World::splat(1.0));
    let fit = (rect.width() * 0.9 / span.x).min(rect.height() * 0.9 / span.y);
    PlanView {
        centre: [(lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0],
        pixels_per_metre: fit.clamp(ZOOM.0, ZOOM.1),
        fitted: true,
    }
}

fn draw_entity(painter: &egui::Painter, map: &Map, obb: &Obb, placed: &Placed, selected: bool) {
    let hull = hull_of(obb);
    if hull.len() < 3 {
        return;
    }
    let points: Vec<egui::Pos2> = hull.iter().map(|w| map.to_screen(*w)).collect();
    let bounds = egui::Rect::from_points(&points);
    if !bounds.intersects(map.rect) {
        return;
    }
    let base = colour(placed.lod_level.as_deref(), placed.declared);
    // A footprint below a pixel is a dot, so a far-off map is not a smear of polygons.
    if bounds.width().max(bounds.height()) < 1.5 {
        painter.circle_filled(bounds.center(), 0.9, base);
        return;
    }
    let (fill, stroke) = if selected {
        (
            egui::Color32::from_rgba_unmultiplied(255, 235, 90, 110),
            egui::Stroke::new(
                2.0_f32,
                theme::graphic(egui::Color32::from_rgb(255, 235, 90)),
            ),
        )
    } else {
        (base.gamma_multiply(0.28), egui::Stroke::new(1.0_f32, base))
    };
    painter.add(egui::Shape::convex_polygon(points, fill, stroke));
}

/// Lines every power of ten of metres that keeps them well apart.
fn grid(painter: &egui::Painter, map: &Map) {
    let per_metre = map.view.pixels_per_metre;
    let step = 10f32.powf((60.0_f32 / per_metre).log10().ceil());
    let (lo, hi) = (
        map.to_world(map.rect.left_bottom()),
        map.to_world(map.rect.right_top()),
    );
    let stroke = egui::Stroke::new(1.0_f32, theme::BORDER.get().gamma_multiply(0.6));
    each_line(lo.x, hi.x, step, |x| {
        let (a, b) = (
            map.to_screen(World::new(x, lo.y)),
            map.to_screen(World::new(x, hi.y)),
        );
        painter.line_segment([a, b], stroke);
    });
    each_line(lo.y, hi.y, step, |y| {
        let (a, b) = (
            map.to_screen(World::new(lo.x, y)),
            map.to_screen(World::new(hi.x, y)),
        );
        painter.line_segment([a, b], stroke);
    });
}

/// The most lines a grid draws: they are at least 60 pixels apart, so a screen holds far fewer.
const LINES: i64 = 256;

/// Call `line` with each multiple of `step` from `lo` to `hi`.
///
/// ⚠ Counted in whole steps and never by adding the step to a coordinate: where the step is under half the gap between two `f32` there, the sum is the coordinate again and a loop on it never ends.
fn each_line(lo: f32, hi: f32, step: f32, mut line: impl FnMut(f32)) {
    if !(lo.is_finite() && hi.is_finite()) {
        return;
    }
    let first = (lo / step).floor() as i64;
    let last = ((hi / step).floor() as i64).min(first.saturating_add(LINES));
    for at in first..=last {
        line(at as f32 * step);
    }
}

/// A bar at the foot of the plan as long as a round number of metres.
fn scale_bar(painter: &egui::Painter, map: &Map) {
    let per_metre = map.view.pixels_per_metre;
    let metres = 10f32.powf((100.0_f32 / per_metre).log10().floor());
    let start = map.rect.left_bottom() + egui::vec2(10.0, -14.0);
    let end = start + egui::vec2(metres * per_metre, 0.0);
    let stroke = egui::Stroke::new(2.0_f32, theme::TEXT_SECONDARY.get());
    painter.line_segment([start, end], stroke);
    painter.text(
        start + egui::vec2(0.0, -4.0),
        egui::Align2::LEFT_BOTTOM,
        tr!("{metres} m", metres = metres),
        egui::FontId::proportional(11.0),
        theme::TEXT_SECONDARY.get(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many lines the grid draws from `lo` to `hi`; a grid that does not end fails here instead of freezing the test.
    fn lines(lo: f32, hi: f32, step: f32) -> usize {
        let mut drawn = 0;
        each_line(lo, hi, step, |_| {
            drawn += 1;
            assert!(drawn < 10_000, "no end to the grid from {lo} to {hi}");
        });
        drawn
    }

    /// A plan centred far from the origin has a coordinate whose spacing is wider than the step of the grid, and a loop that adds the step to it never moves.
    #[test]
    fn a_grid_over_numbers_a_step_cannot_tell_apart_still_ends() {
        for centre in [0.0_f32, 5e3, 1e9, 3e11, 1e12, 1e15, 1e30] {
            let drawn = lines(centre - 20_000.0, centre + 20_000.0, 10_000.0);
            assert!(drawn <= 10, "{drawn} lines about {centre}");
        }
        // Zoomed in on far-off ground, a metre apart.
        assert!(lines(1e8, 1e8 + 2.0, 1.0) <= 10);
        assert_eq!(
            lines(-25.0, 25.0, 10.0),
            6,
            "an ordinary plan keeps its lines"
        );
        assert_eq!(lines(f32::NAN, f32::NAN, 10.0), 0);
        assert_eq!(lines(f32::NEG_INFINITY, f32::INFINITY, 10.0), 0);
    }
}
