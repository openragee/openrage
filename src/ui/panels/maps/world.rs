//! GTA VI's world data over the plan of a scanned map: the lights, the lane graph and the entities
//! of the other maps, each behind its own checkbox, from the bodies opened or, once a fill has read
//! the map's archive, from every map of it.
//!
//! ⚠ Thin by design, as the other panels of the screen are: what is drawn is what
//! `state::gta6_world` keeps of the bodies read.
//!
//! ⚠ A fill's entities are drawn from grids, by the cells in view: far off a cell is one square
//! that says how many stand in it, and no frame walks every entity.

use std::sync::Arc;

use or_io::gta6::map::world::{Grid, PlacementRow};
use or_io::gta6::map::LOD_LEVELS;

use crate::render::map_overlay::level_colour;
use crate::state::gta6_world::{self, Area, Light};
use crate::state::AppState;
use crate::ui::accessible_name::named;
use crate::ui::theme;

/// The most marks of one kind the plan draws in a frame; past it every n-th in view is drawn.
const MOST: usize = 20_000;
/// The fewest pixels a cell drawn as one square is wide.
const CELL_PIXELS: f32 = 16.0;
/// How near the pointer, in pixels, an entity is picked.
const PICK_PIXELS: f32 = 8.0;

const ROAD: egui::Color32 = egui::Color32::from_rgb(90, 200, 220);
const SIGNAL: egui::Color32 = egui::Color32::from_rgb(235, 80, 70);
const LIT: egui::Color32 = egui::Color32::from_rgb(250, 215, 110);
const PICKED: egui::Color32 = egui::Color32::from_rgb(255, 235, 90);

/// Which world data the plan draws over a scanned map.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct Shown {
    lights: bool,
    lanes: bool,
    others: bool,
    /// The row of the fill's index the pointer picked.
    picked: Option<usize>,
    /// The part of the ground plane the plan last showed, its lowest corner and its highest.
    seen: Option<([f32; 2], [f32; 2])>,
}

impl Shown {
    /// Pick the entity of the fill nearest `at`, a click on the plan where a metre is
    /// `pixels_per_metre` pixels; none where the click took an entity of the map shown.
    pub(super) fn pick(&mut self, at: [f32; 2], pixels_per_metre: f32, taken: bool) {
        self.picked = match (taken || !self.others, gta6_world::world().area.as_ref()) {
            (false, Some(area)) => area.pick(at, PICK_PIXELS / pixels_per_metre),
            _ => None,
        };
    }
}

/// The step that keeps `count` marks to at most [`MOST`].
fn stride(count: usize) -> usize {
    count.div_ceil(MOST).max(1)
}

/// How the entities of a fill are drawn where a metre is so many pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Band {
    /// Too far for an entity: each cell of this grid of [`Area::far`] is one square.
    Cells(usize),
    /// The levels of detail below the full one, each entity its own mark.
    Lower,
    /// The full level of detail, each entity its own mark.
    Full,
}

fn band(pixels_per_metre: f32) -> Band {
    if pixels_per_metre >= 2.0 {
        Band::Full
    } else if pixels_per_metre >= 0.5 {
        Band::Lower
    } else if 512.0 * pixels_per_metre >= CELL_PIXELS {
        Band::Cells(0)
    } else {
        Band::Cells(1)
    }
}

/// What a click on the tools asks for, done once the row is drawn.
enum Asked {
    Fill(Option<([f32; 2], [f32; 2])>),
    Stop,
    Open(std::path::PathBuf, u64),
}

/// The overlay's checkboxes and the fill's buttons, in rows of the plan's tools while a scanned
/// GTA VI map is shown, and what to do where a kind that is asked for has nothing to draw.
pub(super) fn tools(ui: &mut egui::Ui, state: &mut AppState, shown: &mut Shown) {
    let Some(scanned) = state.scanned_map() else {
        return;
    };
    let area = (gta6_world::world().area.clone()).filter(|a| a.holds(&scanned.path));
    let running = gta6_world::fill::running();
    let mut asked = None;
    let mut hints: Vec<String> = Vec::new();
    ui.horizontal_wrapped(|ui| {
        let response = ui.checkbox(&mut shown.lights, tr!("Lights"));
        named(
            response,
            tr!("Show the lights of the map and of the archetypes it places"),
        );
        let response = ui.checkbox(&mut shown.lanes, tr!("Lane graph"));
        named(
            response,
            tr!("Show the nodes and links of the vehicle lane graph and its signalised junctions"),
        );
        let response = ui.checkbox(&mut shown.others, tr!("Other maps"));
        named(
            response,
            tr!("Show the entities of the other map bodies opened as points"),
        );
        if running {
            let response = ui.button(tr!("Stop reading"));
            if named(response, tr!("Stop reading the world of the archive")).clicked() {
                asked = Some(Asked::Stop);
            }
        } else {
            let response = ui.button(tr!("Read the whole archive"));
            let response = named(
                response,
                tr!("Read every map, light and lane of the archive this map is a body of"),
            );
            if response.clicked() {
                asked = Some(Asked::Fill(None));
            }
            let response = ui.add_enabled(
                shown.seen.is_some(),
                egui::Button::new(tr!("Read the area in view")),
            );
            let response = named(
                response,
                tr!("Read the maps of the archive that meet the part of the plan in view"),
            );
            if response.clicked() {
                asked = Some(Asked::Fill(shown.seen));
            }
        }
    });
    match &area {
        Some(area) => {
            hints.push(tr!(
                "{maps} maps, {entities} entities, {lights} lights and {links} lane links read of the archive",
                maps = area.filled.index.maps.len(),
                entities = area.filled.index.rows.len(),
                lights = area.filled.lights.len(),
                links = area.filled.lane_segments.len()
            ));
        }
        None => {
            let mut world = gta6_world::world();
            if shown.lights && world.lights_of(scanned).is_empty() {
                hints
                    .push(tr!("This map and the archetypes read so far hold no light").to_string());
            }
            if shown.lanes && world.lanes.points.is_empty() {
                hints.push(
                    tr!("Open the lane graph's node list from the scan to draw it").to_string(),
                );
            }
            let own = usize::from(scanned.data().is_some_and(|m| !m.entities.is_empty()));
            if shown.others && world.indexed().0 <= own {
                hints.push(
                    tr!("Open other map bodies of the scan to draw their entities").to_string(),
                );
            }
        }
    }
    for hint in &hints {
        ui.weak(hint.as_str());
    }
    let picked = (area.as_ref()).and_then(|area| {
        let row = area.filled.index.rows.get(shown.picked?)?;
        Some((area, row, area.map_of(shown.picked?)?))
    });
    if let Some((area, row, map)) = picked {
        ui.horizontal_wrapped(|ui| {
            ui.monospace(gta6_world::archetype_name(row));
            ui.weak(tr!(
                "{level}, in the map at {offset}",
                level = LOD_LEVELS[usize::from(row.lod_level)],
                offset = format!("{:#X}", map.offset)
            ));
            let response = ui.button(tr!("Open its map"));
            if named(
                response,
                tr!("Open the map body that places the picked entity"),
            )
            .clicked()
            {
                asked = Some(Asked::Open(area.archive.clone(), map.offset));
            }
        });
    }
    match asked {
        Some(Asked::Fill(region)) => {
            if let Err(why) = state.fill_gta6_world(region) {
                state.show_info_toast(why);
            }
        }
        Some(Asked::Stop) => gta6_world::fill::stop(),
        Some(Asked::Open(archive, offset)) => state.open_gta6_map_body(&archive, offset),
        None => {}
    }
}

/// The marks of `grid` in the cells that meet `lo` to `hi`, every n-th of them where they are more
/// than [`MOST`].
fn in_view(grid: &Grid, lo: [f32; 2], hi: [f32; 2]) -> impl Iterator<Item = usize> + '_ {
    let cells = grid.cells_within(lo, hi);
    let step = stride(cells.iter().map(|(_, held)| held.len()).sum());
    (cells.into_iter())
        .flat_map(|(_, held)| held.iter())
        .step_by(step)
        .map(|k| *k as usize)
}

/// Each cell of `grid` in view as one square of `colour`, the stronger the more it holds.
fn squares(
    painter: &egui::Painter,
    grid: &Grid,
    seen: ([f32; 2], [f32; 2]),
    colour: egui::Color32,
    at: &impl Fn([f32; 2]) -> egui::Pos2,
) {
    for (corner, held) in grid.cells_within(seen.0, seen.1) {
        let far = [corner[0] + grid.cell(), corner[1] + grid.cell()];
        let strength = ((held.len() as f32).ln_1p() / 10.0).clamp(0.06, 0.7);
        let square = egui::Rect::from_two_pos(at(corner), at(far)).shrink(0.5);
        painter.rect_filled(square, 0.0, colour.gamma_multiply(strength));
    }
}

/// One entity of a fill: the footprint of its archetype's box where a body states one and it is
/// wider than a few pixels, else a dot.
fn entity(
    painter: &egui::Painter,
    area: &Area,
    row: &PlacementRow,
    pixels_per_metre: f32,
    at: &impl Fn([f32; 2]) -> egui::Pos2,
) {
    let [r, g, b, _] = level_colour(Some(LOD_LEVELS[usize::from(row.lod_level)]));
    let colour = egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
    let boxed = area
        .filled
        .boxes
        .get(&row.archetype_word)
        .filter(|[min, max]| {
            let wide = (max[0] - min[0]).max(max[1] - min[1]) * row.scale_xy.abs();
            wide * pixels_per_metre >= 4.0
        });
    let Some([min, max]) = boxed else {
        painter.circle_filled(at([row.position[0], row.position[1]]), 1.5, colour);
        return;
    };
    let corners = row.corners(*min, *max).map(glam::Vec3::from);
    let hull: Vec<egui::Pos2> = (or_world::gizmo::footprint(&corners).iter())
        .map(|p| at(p.to_array()))
        .collect();
    if hull.len() >= 3 {
        let stroke = egui::Stroke::new(1.0_f32, colour);
        painter.add(egui::Shape::convex_polygon(
            hull,
            colour.gamma_multiply(0.2),
            stroke,
        ));
    }
}

fn light(painter: &egui::Painter, light: &Light, centre: egui::Pos2, pixels_per_metre: f32) {
    let [r, g, b] = light.colour;
    let colour = egui::Color32::from_rgb(r, g, b);
    let reach = light.range * pixels_per_metre;
    if reach.is_finite() && reach >= 3.0 {
        let ring = egui::Stroke::new(1.0_f32, colour.gamma_multiply(0.35));
        painter.circle_stroke(centre, reach, ring);
    }
    painter.circle_filled(centre, 2.5, colour);
    painter.circle_stroke(centre, 2.5, egui::Stroke::new(1.0_f32, theme::BG_L0.get()));
}

/// What a fill read, drawn from its grids.
fn draw_area(
    painter: &egui::Painter,
    area: &Area,
    shown: Shown,
    show_lod: bool,
    seen: ([f32; 2], [f32; 2]),
    pixels_per_metre: f32,
    at: &impl Fn([f32; 2]) -> egui::Pos2,
) {
    let (lo, hi) = seen;
    let inside = |p: &[f32; 2]| (0..2).all(|i| p[i] >= lo[i] && p[i] <= hi[i]);
    let rows = &area.filled.index.rows;
    if shown.others {
        let dim = theme::TEXT_SECONDARY.get();
        let each = |grid: &Grid| {
            for row in in_view(grid, lo, hi).filter_map(|k| rows.get(k)) {
                if inside(&[row.position[0], row.position[1]]) {
                    entity(painter, area, row, pixels_per_metre, at);
                }
            }
        };
        match band(pixels_per_metre) {
            Band::Cells(far) => squares(painter, &area.far[far], seen, dim, at),
            Band::Lower => {
                // The full level is too many marks here: its cells say where it stands.
                squares(painter, &area.full, seen, dim, at);
                each(&area.lower);
            }
            Band::Full => {
                if show_lod {
                    each(&area.lower);
                }
                each(&area.full);
            }
        }
        if let Some(row) = shown.picked.and_then(|k| rows.get(k)) {
            let centre = at([row.position[0], row.position[1]]);
            painter.circle_stroke(
                centre,
                6.0,
                egui::Stroke::new(2.0_f32, theme::graphic(PICKED)),
            );
        }
    }
    if shown.lanes {
        let stroke = egui::Stroke::new(1.0_f32, theme::graphic(ROAD).gamma_multiply(0.6));
        let segments = &area.filled.lane_segments;
        for s in in_view(&area.segments, lo, hi).filter_map(|k| segments.get(k)) {
            painter.line_segment([at(s[0]), at(s[1])], stroke);
        }
        for p in area.filled.lane_signals.iter().filter(|p| inside(p)) {
            let mark = egui::Rect::from_center_size(at(*p), egui::vec2(5.0, 5.0));
            painter.rect_filled(mark, 0.0, theme::graphic(SIGNAL));
        }
    }
    if shown.lights {
        if pixels_per_metre >= 0.5 {
            let lights = &area.filled.lights;
            for l in in_view(&area.lights, lo, hi).filter_map(|k| lights.get(k)) {
                if inside(&l.at) {
                    light(painter, l, at(l.at), pixels_per_metre);
                }
            }
        } else {
            squares(painter, &area.lights_far, seen, theme::graphic(LIT), at);
        }
    }
}

/// The world data asked for, drawn on the plan through `to_screen`; `seen` is the part of the
/// ground plane the plan shows, its lowest corner and its highest.
pub(super) fn draw_on_plan(
    painter: &egui::Painter,
    state: &AppState,
    shown: &mut Shown,
    seen: (glam::Vec2, glam::Vec2),
    to_screen: impl Fn(glam::Vec2) -> egui::Pos2,
) {
    let Some(scanned) = state.scanned_map() else {
        return;
    };
    let (lo, hi) = (seen.0.to_array(), seen.1.to_array());
    shown.seen = Some((lo, hi));
    if !(shown.lights || shown.lanes || shown.others) {
        return;
    }
    let inside = |p: &[f32; 2]| (0..2).all(|i| p[i] >= lo[i] && p[i] <= hi[i]);
    let at = |p: [f32; 2]| to_screen(glam::Vec2::from(p));
    // The pixels a metre is, from two points a metre apart.
    let per_metre = (at([lo[0] + 1.0, lo[1]]).x - at(lo).x).abs();
    let area: Option<Arc<Area>> =
        (gta6_world::world().area.clone()).filter(|a| a.holds(&scanned.path));
    if let Some(area) = area {
        let show_lod = state.workbench.maps.show_lod;
        draw_area(painter, &area, *shown, show_lod, (lo, hi), per_metre, &at);
        return;
    }
    let mut world = gta6_world::world();
    if shown.others {
        let dot = theme::TEXT_SECONDARY.get();
        let except = Some(scanned.path.as_path());
        let step = stride(world.placements(lo, hi, except).count());
        for p in world.placements(lo, hi, except).step_by(step) {
            painter.circle_filled(at(p), 1.2, dot);
        }
    }
    if shown.lanes {
        let lanes = &world.lanes;
        let stroke = egui::Stroke::new(1.0_f32, theme::graphic(ROAD).gamma_multiply(0.6));
        let in_view = |s: &&[[f32; 2]; 2]| inside(&s[0]) || inside(&s[1]);
        let step = stride(lanes.segments.iter().filter(in_view).count());
        for s in lanes.segments.iter().filter(in_view).step_by(step) {
            painter.line_segment([at(s[0]), at(s[1])], stroke);
        }
        let step = stride(lanes.points.iter().filter(|p| inside(p)).count());
        for p in lanes.points.iter().filter(|p| inside(p)).step_by(step) {
            painter.circle_filled(at(*p), 1.5, theme::graphic(ROAD));
        }
        for p in lanes.signals.iter().filter(|p| inside(p)) {
            let mark = egui::Rect::from_center_size(at(*p), egui::vec2(5.0, 5.0));
            painter.rect_filled(mark, 0.0, theme::graphic(SIGNAL));
        }
    }
    if shown.lights {
        let lights = world.lights_of(scanned);
        let in_view = |l: &&Light| inside(&l.at);
        let step = stride(lights.iter().filter(in_view).count());
        for l in lights.iter().filter(in_view).step_by(step) {
            light(painter, l, at(l.at), per_metre);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_past_the_most_a_frame_draws_are_thinned_and_fewer_are_all_drawn() {
        assert_eq!(stride(0), 1);
        assert_eq!(stride(MOST), 1);
        assert_eq!(stride(MOST + 1), 2);
        assert_eq!(stride(3 * MOST), 3);
        assert!((0..5 * MOST + 7).step_by(stride(5 * MOST + 7)).count() <= MOST);
    }

    #[test]
    fn the_entities_of_a_fill_are_drawn_by_how_many_pixels_a_metre_is() {
        assert_eq!(band(4.0), Band::Full);
        assert_eq!(band(2.0), Band::Full);
        assert_eq!(band(1.0), Band::Lower);
        assert_eq!(band(0.5), Band::Lower);
        assert_eq!(band(0.1), Band::Cells(0));
        // The plan's farthest view: a cell of 512 m would be under ten pixels.
        assert_eq!(band(0.02), Band::Cells(1));
        for pixels_per_metre in [0.02_f32, 0.03, 0.031_25, 0.2, 0.49] {
            let cell = match band(pixels_per_metre) {
                Band::Cells(0) => 512.0,
                Band::Cells(_) => 4096.0,
                other => panic!("{pixels_per_metre}: {other:?}"),
            };
            assert!(cell * pixels_per_metre >= CELL_PIXELS, "{pixels_per_metre}");
        }
    }

    #[test]
    fn the_marks_in_view_come_from_the_cells_the_view_meets_and_are_thinned() {
        let grid = Grid::new(10.0, [[1.0, 1.0], [55.0, 55.0], [2.0, 3.0]]);
        let mut near: Vec<usize> = in_view(&grid, [0.0, 0.0], [9.0, 9.0]).collect();
        near.sort_unstable();
        assert_eq!(near, [0, 2]);
        assert_eq!(in_view(&grid, [100.0, 100.0], [200.0, 200.0]).count(), 0);
        let many = Grid::new(1000.0, (0..3 * MOST).map(|k| [k as f32 * 0.01, 0.0]));
        let drawn = in_view(&many, [0.0, 0.0], [999.0, 1.0]).count();
        assert!(drawn <= MOST && drawn > MOST / 2, "{drawn}");
    }
}
