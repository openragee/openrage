//! How a world streams, as numbers, tables and the one rule asked once a map: the defaults are
//! written here, and a title overrides only what it disagrees with.
//!
//! ⚠ Nothing of a policy is called once a node: what a node's category and kind make of its
//! distance is the forest's own.

use glam::Vec3;

use crate::streaming::{camera_scale, Children, Level, MapBox, View, FADE};

/// How one opened world streams.
#[derive(Debug, Clone, Copy)]
pub struct StreamPolicy {
    /// How many children a parent waits for before it fades out for them.
    pub children: Children,
    /// Whether a camera at the position streams in a map of the boxes.
    pub streams_in: fn(&MapBox, Vec3) -> bool,
    /// What the distances of each level are multiplied by, by `Level as usize`, before the camera's own scale.
    pub scales: [f32; Level::COUNT],
    /// The band past its distance each level fades out over, by `Level as usize`, in metres.
    pub fade: [f32; Level::COUNT],
    /// The most bytes of map bodies held, those in the forest among them, unless one view's own maps pass it.
    pub budget: u64,
}

/// Within the sphere of the box a map streams by.
fn within_the_sphere(boxes: &MapBox, eye: Vec3) -> bool {
    boxes.wants(eye)
}

impl Default for StreamPolicy {
    fn default() -> Self {
        Self {
            children: Children::RaisedToLoaded,
            streams_in: within_the_sphere,
            scales: [1.0; Level::COUNT],
            fade: [FADE; Level::COUNT],
            budget: 256 << 20,
        }
    }
}

impl StreamPolicy {
    /// The view of a camera at `eye` moving at `velocity`, of the vertical field of view `fov_y` in radians.
    pub fn view(&self, eye: Vec3, velocity: Vec3, fov_y: f32) -> View {
        let camera = camera_scale(fov_y);
        View {
            eye,
            velocity,
            scales: self.scales.map(|scale| scale * camera),
            fade: self.fade,
        }
    }

    /// The first table or number that states nothing a level can stream by, as a sentence for an error.
    pub fn fault(&self) -> Option<&'static str> {
        let sound = |value: f32| value.is_finite() && value > 0.0;
        if !self.scales.iter().all(|scale| sound(*scale)) {
            return Some("a level's scale is not a number above nought");
        }
        if !self.fade.iter().all(|band| sound(*band)) {
            return Some("a level's fade band is not a number above nought");
        }
        (self.budget == 0).then_some("its budget holds no map")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_scales_each_level_by_its_table_and_the_camera() {
        let policy = StreamPolicy {
            scales: [1.0, 2.0, 1.0, 1.0, 1.0, 1.0, 0.5],
            ..StreamPolicy::default()
        };
        let fov = std::f32::consts::FRAC_PI_8;
        let view = policy.view(Vec3::ZERO, Vec3::ZERO, fov);
        let camera = camera_scale(fov);
        assert!(camera > 1.0);
        assert_eq!(view.scales[Level::Lod as usize], 2.0 * camera);
        assert_eq!(view.scales[Level::Slod4 as usize], 0.5 * camera);
        assert_eq!(view.fade, [FADE; Level::COUNT]);
    }

    #[test]
    fn a_policy_with_a_band_of_nought_states_its_fault() {
        assert_eq!(StreamPolicy::default().fault(), None);
        let mut policy = StreamPolicy::default();
        policy.fade[Level::Slod4 as usize] = 0.0;
        assert!(policy.fault().is_some());
    }
}
