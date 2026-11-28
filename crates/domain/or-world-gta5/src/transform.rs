//! Placement conventions, and the one asymmetry that is pure folklore.

use glam::Quat;

/// What a placement's stored rotation means.
///
/// ⚠ **The two differ, and nothing in the file says so.** A `CEntityDef`
/// stores the **conjugate** of the rotation that places it; a
/// `CMloInstanceDef` — the same struct with five fields appended, placing an
/// interior — stores the rotation itself. An interior's *contents* are
/// `CEntityDef`s again and go back to the conjugate, so the exception is one
/// struct deep and reverses twice on the way into a room.
///
/// Read the wrong one and the object is still the right size in the right
/// place, mirrored about its own axes, which is why this survives review.
///
/// ⚠ Recomputing entity extents over 2,878 retail Legacy `.ymap` and
/// comparing with the extents those files store: conjugating reproduces 807
/// exactly and 1,559 to within a centimetre, and not conjugating reproduces
/// 552 and 867. The 552 that survive the wrong convention are the maps whose
/// entities are all unrotated, where the two agree — see
/// `identity_hides_the_asymmetry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A `CEntityDef`: stored conjugated.
    Entity,
    /// A `CMloInstanceDef`: stored as-is.
    InteriorInstance,
}

impl Placement {
    /// The rotation that places the object, from the four stored floats.
    ///
    /// The stored order is `x, y, z, w`.
    pub fn rotation_of(self, stored: [f32; 4]) -> Quat {
        let q = Quat::from_xyzw(stored[0], stored[1], stored[2], stored[3]);
        let q = if q.length_squared() > f32::EPSILON {
            q.normalize()
        } else {
            Quat::IDENTITY
        };
        match self {
            Placement::Entity => q.conjugate(),
            Placement::InteriorInstance => q,
        }
    }

    /// The four floats to store for a rotation.
    pub fn store(self, rotation: Quat) -> [f32; 4] {
        let q = match self {
            Placement::Entity => rotation.conjugate(),
            Placement::InteriorInstance => rotation,
        };
        [q.x, q.y, q.z, q.w]
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn an_interior_instance_stores_the_rotation_an_entity_stores_conjugated() {
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let stored = [quarter.x, quarter.y, quarter.z, quarter.w];

        let as_entity = Placement::Entity.rotation_of(stored);
        let as_interior = Placement::InteriorInstance.rotation_of(stored);

        assert!((as_interior * Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
        assert!((as_entity * Vec3::X).abs_diff_eq(-Vec3::Y, 1e-5));
        assert!(as_entity.abs_diff_eq(as_interior.conjugate(), 1e-6));
    }

    /// Identity is the case where the two conventions agree, which is why a
    /// corpus full of unrotated props cannot tell them apart.
    #[test]
    fn identity_hides_the_asymmetry() {
        let stored = [0.0, 0.0, 0.0, 1.0];
        assert!(Placement::Entity
            .rotation_of(stored)
            .abs_diff_eq(Placement::InteriorInstance.rotation_of(stored), 1e-6));
    }

    #[test]
    fn storing_a_rotation_reverses_reading_one() {
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.3, -1.1, 2.0);
        for placement in [Placement::Entity, Placement::InteriorInstance] {
            let back = placement.rotation_of(placement.store(q));
            assert!(back.abs_diff_eq(q, 1e-6), "{placement:?}");
        }
    }

    /// A zero quaternion is not a rotation, and normalising one yields NaN.
    #[test]
    fn a_zero_quaternion_reads_as_no_rotation() {
        let r = Placement::Entity.rotation_of([0.0; 4]);
        assert!(r.is_finite());
        assert!(r.abs_diff_eq(Quat::IDENTITY, 1e-6));
    }
}
