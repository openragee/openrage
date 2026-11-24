//! What stands round the model a frame lights: the box it fills, the clouds over it and the
//! ground under it, and the words the resolve reads of them.

use glam::Vec3;

/// What stands round the model a frame lights.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scene {
    /// The box the model fills, from its least corner to its greatest: the sun's shadow is
    /// fitted to it, and a ground lies under it.
    pub bounds: Option<(Vec3, Vec3)>,
    pub clouds: Clouds,
    /// What a ground laid under the model sends back of each colour; none where none is laid.
    pub laid: Option<Vec3>,
}

/// What a cycle states of the clouds over a model and of the directional light under them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clouds {
    /// The share of the directional light the clouds let through, from nought to one.
    pub open: f32,
    /// How far the light's roughness is taken towards one in a cloud's shadow.
    pub roughness_mod: f32,
    /// How far a light the clouds hide is taken towards what the surroundings give along its
    /// way, and the strength of that.
    pub haze_blend: f32,
    pub haze: f32,
}

impl Default for Clouds {
    /// No cloud.
    fn default() -> Self {
        Self {
            open: 1.0,
            roughness_mod: 0.0,
            haze_blend: 0.0,
            haze: 1.0,
        }
    }
}

/// The words the resolve reads of a scene, after the shadow's.
pub(super) const WORDS: usize = 12;

impl Scene {
    /// The ground under the box: the middle of its square, how high it lies and half its side.
    // As a ground is laid under a model of GTA V: a hair under its lowest point, three times as
    // wide as the box.
    fn plane(&self) -> Option<[f32; 4]> {
        let (min, max) = self.bounds?;
        let size = max - min;
        Some([
            (min.x + max.x) * 0.5,
            (min.y + max.y) * 0.5,
            min.z - 0.002 * size.max_element(),
            1.5 * size.x.max(size.y),
        ])
    }

    /// What the resolve reads: the clouds, the ground's colour with whether one is laid, and
    /// its plane.
    pub(super) fn words(&self) -> [f32; WORDS] {
        let clouds = &self.clouds;
        let mut words = [0.0; WORDS];
        words[..4].copy_from_slice(&[
            clouds.open.clamp(0.0, 1.0),
            clouds.roughness_mod,
            clouds.haze_blend,
            clouds.haze,
        ]);
        if let (Some(laid), Some(plane)) = (self.laid, self.plane()) {
            words[4..8].copy_from_slice(&[laid.x, laid.y, laid.z, 1.0]);
            words[8..].copy_from_slice(&plane);
        }
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ground_is_laid_a_hair_under_the_box_and_three_times_as_wide() {
        let mut scene = Scene {
            laid: Some(Vec3::new(0.25, 0.5, 0.75)),
            ..Scene::default()
        };
        // With no box there is nowhere to lay it: the clouds' words alone.
        assert_eq!(scene.words()[..4], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(scene.words()[4..], [0.0; 8]);
        scene.bounds = Some((Vec3::new(-1.0, 2.0, 3.0), Vec3::new(3.0, 4.0, 5.0)));
        let words = scene.words();
        assert_eq!(words[4..8], [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(words[8..], [1.0, 3.0, 3.0 - 0.002 * 4.0, 6.0]);
        scene.laid = None;
        assert_eq!(scene.words()[4..], [0.0; 8], "a box and no ground");
        scene.clouds.open = 1.5;
        assert_eq!(scene.words()[0], 1.0, "held to one");
    }
}
