//! What the Map screen draws over the set shown: a box for each entity, the entity selected lit, the ground under the view, and the
//! manipulator at the entity selected.
//!
//! The scene panel builds the lines for the frame (`lines_of`) and keeps them in `state.workbench.maps.overlay`; the viewport draws them
//! with the ones plugins draw, in front of everything, through `append`, the one place this joins the viewport's line list.

use glam::{Quat, Vec3};
use or_maps::{Placed, Scene};
use or_renderer::LineVertex;
use or_world::gizmo::{Gizmo, Handle, Obb};

use crate::state::AppState;
use crate::ui::dock::Workspace;

/// How many entity boxes are drawn: past this the plan view is the way to see them all.
pub const MOST_BOXES: usize = 8_000;

const SELECTED: [f32; 4] = [1.0, 0.92, 0.35, 1.0];
const UNKNOWN: [f32; 4] = [0.55, 0.58, 0.66, 0.9];
const GRID: [f32; 4] = [0.42, 0.45, 0.5, 0.45];
const HOVERED: [f32; 4] = [1.0, 0.95, 0.4, 1.0];

/// Add the lines of the Map screen to `lines`, while it is the workspace on screen.
pub fn append(lines: &mut Vec<LineVertex>, state: &AppState) {
    if state.active_workspace == Workspace::Map {
        lines.extend_from_slice(&state.workbench.maps.overlay);
    }
}

/// The colour a box of an entity of this level of detail is drawn in.
pub fn level_colour(level: Option<&str>) -> [f32; 4] {
    match level {
        None | Some("LODTYPES_DEPTH_HD") => [0.62, 0.74, 0.88, 1.0],
        Some("LODTYPES_DEPTH_LOD") => [0.92, 0.72, 0.32, 1.0],
        Some("LODTYPES_DEPTH_SLOD1") => [0.9, 0.52, 0.25, 1.0],
        Some("LODTYPES_DEPTH_SLOD2") => [0.88, 0.4, 0.3, 1.0],
        Some("LODTYPES_DEPTH_SLOD3" | "LODTYPES_DEPTH_SLOD4") => [0.8, 0.32, 0.45, 1.0],
        Some("LODTYPES_DEPTH_ORPHANHD") => [0.5, 0.8, 0.52, 1.0],
        Some(_) => [0.7, 0.7, 0.7, 1.0],
    }
}

/// Whether an entity of this level is one of the full detail the set is mostly made of.
pub fn is_full_detail(level: Option<&str>) -> bool {
    matches!(
        level,
        None | Some("LODTYPES_DEPTH_HD" | "LODTYPES_DEPTH_ORPHANHD")
    )
}

/// The twelve edges of a box, as pairs of its corners.
fn edges(corners: &[Vec3; 8], colour: [f32; 4], out: &mut Vec<LineVertex>) {
    for a in 0..8usize {
        for bit in [1usize, 2, 4] {
            let b = a ^ bit;
            if a < b {
                out.push(LineVertex::new(corners[a].to_array(), colour));
                out.push(LineVertex::new(corners[b].to_array(), colour));
            }
        }
    }
}

/// The ground under the view: lines every `step` metres in a square of `cells` cells each way round `centre`, at height `z`.
pub fn grid(centre: Vec3, step: f32, cells: i32, z: f32) -> Vec<LineVertex> {
    let mut out = Vec::new();
    let step = step.max(0.01);
    let (cx, cy) = (
        (centre.x / step).round() * step,
        (centre.y / step).round() * step,
    );
    let reach = cells as f32 * step;
    for i in -cells..=cells {
        let along = i as f32 * step;
        out.push(LineVertex::new([cx + along, cy - reach, z], GRID));
        out.push(LineVertex::new([cx + along, cy + reach, z], GRID));
        out.push(LineVertex::new([cx - reach, cy + along, z], GRID));
        out.push(LineVertex::new([cx + reach, cy + along, z], GRID));
    }
    out
}

/// The step of the grid under a view `distance` metres from what it looks at: a power of ten that keeps its lines a fair way apart on the screen.
pub fn grid_step(distance: f32) -> f32 {
    10f32.powf((distance.max(0.1) / 12.0).log10().floor())
}

/// The lines for the frame: every box the set places (the selected one lit, at `shown` when it is being dragged), the manipulator, and the ground.
pub fn lines_of(
    scene: &Scene,
    selected: Option<usize>,
    shown: Option<Obb>,
    show_lod: bool,
    gizmo: Option<(&Gizmo, Option<Handle>)>,
    ground: Option<(Vec3, f32, f32)>,
) -> Vec<LineVertex> {
    let mut out = Vec::new();
    if let Some((centre, step, z)) = ground {
        out.extend(grid(centre, step, 30, z));
    }
    for (at, placed) in scene.placed.iter().enumerate().take(MOST_BOXES) {
        if selected == Some(at) {
            continue;
        }
        if !show_lod && !is_full_detail(placed.lod_level.as_deref()) {
            continue;
        }
        edges(&placed.obb.corners(), colour_of(placed), &mut out);
    }
    if let Some(at) = selected {
        if let Some(placed) = scene.placed.get(at) {
            let obb = shown.unwrap_or(placed.obb);
            edges(&obb.corners(), SELECTED, &mut out);
            // The centre, so a small box is found.
            let c = obb.position;
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                out.push(LineVertex::new((c - axis * 0.15).to_array(), SELECTED));
                out.push(LineVertex::new((c + axis * 0.15).to_array(), SELECTED));
            }
        }
    }
    if let Some((gizmo, hovered)) = gizmo {
        for (handle, [a, b]) in gizmo.lines() {
            let colour = if Some(handle) == hovered {
                HOVERED
            } else {
                match handle {
                    Handle::Axis(axis) | Handle::Plane(axis) | Handle::Ring(axis) => axis.colour(),
                    Handle::Centre => [0.95, 0.95, 0.95, 1.0],
                }
            };
            out.push(LineVertex::new(a.to_array(), colour));
            out.push(LineVertex::new(b.to_array(), colour));
        }
    }
    out
}

fn colour_of(placed: &Placed) -> [f32; 4] {
    if placed.declared {
        level_colour(placed.lod_level.as_deref())
    } else {
        UNKNOWN
    }
}

/// The frame the manipulator's axes follow: the entity's own turn for a rotate and a scale, the world's for a move.
pub fn frame_of(mode: or_world::gizmo::Mode, rotation: Quat) -> Quat {
    match mode {
        or_world::gizmo::Mode::Move => Quat::IDENTITY,
        _ => rotation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_maps::scene::Placed;

    fn crate_at(x: f32, level: Option<&str>) -> Placed {
        Placed {
            source: "a.metadata.src".into(),
            index: 0,
            archetype: or_maps::Name::from_text("prop_crate_a"),
            declared: true,
            model: None,
            obb: Obb {
                position: Vec3::new(x, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                min: Vec3::splat(-1.0),
                max: Vec3::splat(1.0),
            },
            interior: false,
            lod_dist: 100.0,
            parent_index: -1,
            guid: 1,
            lod_level: level.map(str::to_string),
        }
    }

    #[test]
    fn each_box_is_twelve_edges_and_the_selected_one_is_lit_and_drawn_where_it_is_dragged() {
        let scene = Scene {
            placed: vec![crate_at(0.0, None), crate_at(10.0, None)],
            bounds: None,
        };
        let plain = lines_of(&scene, None, None, true, None, None);
        assert_eq!(plain.len(), 2 * 12 * 2);
        assert!(plain.iter().all(|v| v.colour != SELECTED));

        let mut moved = scene.placed[1].obb;
        moved.position.x = 40.0;
        let lit = lines_of(&scene, Some(1), Some(moved), true, None, None);
        let lit_ones: Vec<_> = lit.iter().filter(|v| v.colour == SELECTED).collect();
        assert!(!lit_ones.is_empty());
        assert!(
            lit_ones.iter().all(|v| v.position[0] > 30.0),
            "the selected box is drawn where it is dragged to"
        );
    }

    #[test]
    fn a_level_of_detail_below_the_full_one_is_left_out_until_asked_for() {
        let scene = Scene {
            placed: vec![
                crate_at(0.0, Some("LODTYPES_DEPTH_HD")),
                crate_at(5.0, Some("LODTYPES_DEPTH_LOD")),
            ],
            bounds: None,
        };
        assert_eq!(lines_of(&scene, None, None, false, None, None).len(), 24);
        assert_eq!(lines_of(&scene, None, None, true, None, None).len(), 48);
    }

    #[test]
    fn the_grid_step_follows_the_distance_in_powers_of_ten() {
        assert_eq!(grid_step(10.0), 0.1);
        assert_eq!(grid_step(130.0), 10.0);
        assert_eq!(grid_step(1.0), 0.01);
    }
}
