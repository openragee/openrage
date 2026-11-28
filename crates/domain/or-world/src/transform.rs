//! The neutral half of a placement: a scale from the two numbers that carry it, the corners of a placed box, and a pose.

use glam::{EulerRot, Quat, Vec3};

/// An entity's scale, from the two floats that carry it.
///
/// ⚠ `scaleXY` is one number for two axes, so a non-uniform scale in X and Y
/// is not expressible and an editor must not offer one.
pub fn scale_of(scale_xy: f32, scale_z: f32) -> Vec3 {
    Vec3::new(scale_xy, scale_xy, scale_z)
}

/// The world-space corners of a local box, placed.
pub fn placed_corners(bb_min: Vec3, bb_max: Vec3, rotation: Quat, position: Vec3) -> [Vec3; 8] {
    let mut out = [Vec3::ZERO; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        let corner = Vec3::new(
            if i & 1 == 0 { bb_min.x } else { bb_max.x },
            if i & 2 == 0 { bb_min.y } else { bb_max.y },
            if i & 4 == 0 { bb_min.z } else { bb_max.z },
        );
        *slot = rotation * corner + position;
    }
    out
}

/// Where an entity stands: its position, its rotation as the rotation that places it, and its scale.
///
/// A file may store the rotation another way and stores one number of scale for the two horizontal axes beside one for the
/// vertical, so a pose is what a person means and the title's edit stores it the way its file does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
    /// The scale of X and Y together.
    pub scale_xy: f32,
    pub scale_z: f32,
}

impl Pose {
    /// The rotation as degrees about X, Y and Z, which `rotation_of_degrees` turns back into the same rotation.
    pub fn degrees(&self) -> [f32; 3] {
        let (x, y, z) = self.rotation.to_euler(EulerRot::XYZ);
        [x.to_degrees(), y.to_degrees(), z.to_degrees()]
    }

    /// The rotation that is `degrees` about X, then Y, then Z.
    pub fn rotation_of_degrees(degrees: [f32; 3]) -> Quat {
        Quat::from_euler(
            EulerRot::XYZ,
            degrees[0].to_radians(),
            degrees[1].to_radians(),
            degrees[2].to_radians(),
        )
        .normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_cover_the_box() {
        let corners = placed_corners(
            Vec3::splat(-1.0),
            Vec3::splat(2.0),
            Quat::IDENTITY,
            Vec3::ZERO,
        );
        let min = corners.iter().fold(Vec3::splat(f32::MAX), |a, b| a.min(*b));
        let max = corners.iter().fold(Vec3::splat(f32::MIN), |a, b| a.max(*b));
        assert_eq!(min, Vec3::splat(-1.0));
        assert_eq!(max, Vec3::splat(2.0));
    }
}
