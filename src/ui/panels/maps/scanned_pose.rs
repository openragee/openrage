//! A scanned GTA VI map entity's placement as fields that take what is typed, and the button that
//! stands it on the ground; each change is one step of Undo.

use or_world::transform::Pose;

use super::entity::number;
use crate::ui::accessible_name::named;

/// What a person asked of the placement this frame.
pub(super) enum Asked {
    /// The entity stood at this pose.
    Pose(Pose),
    /// The entity stood on the ground under it.
    Ground,
}

/// The position, turn and scale of `pose` as fields, then the button that stands it on the ground.
pub(super) fn fields(ui: &mut egui::Ui, pose: Pose) -> Option<Asked> {
    let mut next = pose;
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(tr!("Position"));
        let axes = [
            tr!("Entity position X"),
            tr!("Entity position Y"),
            tr!("Entity position Z"),
        ];
        for (at, name) in axes.into_iter().enumerate() {
            if let Some(v) = number(ui, &format!("gta6_pos{at}"), name, pose.position[at]) {
                next.position[at] = v;
                changed = true;
            }
        }
    });
    let degrees = pose.degrees();
    ui.horizontal_wrapped(|ui| {
        ui.label(tr!("Rotation"));
        let mut turned = degrees;
        let axes = [
            tr!("Entity rotation about X in degrees"),
            tr!("Entity rotation about Y in degrees"),
            tr!("Entity rotation about Z in degrees"),
        ];
        for (at, name) in axes.into_iter().enumerate() {
            if let Some(v) = number(ui, &format!("gta6_rot{at}"), name, degrees[at]) {
                turned[at] = v;
                next.rotation = Pose::rotation_of_degrees(turned);
                changed = true;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(tr!("Scale"));
        let name = tr!("Entity scale of X and Y");
        if let Some(v) = number(ui, "gta6_scale_xy", name, pose.scale_xy) {
            next.scale_xy = v.max(0.01);
            changed = true;
        }
        if let Some(v) = number(ui, "gta6_scale_z", tr!("Entity scale of Z"), pose.scale_z) {
            next.scale_z = v.max(0.01);
            changed = true;
        }
    });
    let ground = ui.button(tr!("Stand on the ground"));
    if named(ground, tr!("Stand on the ground")).clicked() {
        return Some(Asked::Ground);
    }
    changed.then_some(Asked::Pose(next))
}
