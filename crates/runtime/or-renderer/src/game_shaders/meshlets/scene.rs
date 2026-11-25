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
    /// The light from all around as a probe holds it, [`nine_of`]; none where the sky and the
    /// ground are two colours and no more.
    pub around: Option<[Vec3; 9]>,
    /// How bright the sky is along each way it is gathered by, for a meter that takes the whole
    /// frame; none where no sky is computed.
    pub dome: Option<Dome>,
}

/// How bright the sky is along seventy-two ways over the upper half: six rings of twelve, from
/// the top down, each ring begun along +x and turned towards +y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dome(pub [f32; 72]);

/// What each of a probe's nine numbers weighs along `way`: one that is the same every way,
/// three that lean along y, z and x, and five that bend.
fn weights(way: Vec3) -> [f32; 9] {
    let Vec3 { x, y, z } = way;
    [
        0.282_095,
        0.488_603 * y,
        0.488_603 * z,
        0.488_603 * x,
        1.092_548 * x * y,
        1.092_548 * y * z,
        0.315_392 * (3.0 * z * z - 1.0),
        1.092_548 * x * z,
        0.546_274 * (x * x - y * y),
    ]
}

/// The nine numbers a probe holds of the light round a model that stands in the open: `sky` is
/// the light that comes down each of a set of ways over the upper half with the angle its patch
/// of the sky fills, and `ground` what comes up every way of the lower half.
pub fn nine_of(sky: impl IntoIterator<Item = (Vec3, Vec3, f32)>, ground: Vec3) -> [Vec3; 9] {
    let mut nine = [Vec3::ZERO; 9];
    for (way, light, patch) in sky {
        for (held, weight) in nine.iter_mut().zip(weights(way)) {
            *held += light * weight * patch;
        }
    }
    // The lower half whole: the number that is the same every way and the one that leans
    // along z are all that a light the same every way below leaves.
    nine[0] += ground * 0.282_095 * std::f32::consts::TAU;
    nine[2] -= ground * 0.488_603 * std::f32::consts::PI;
    nine
}

/// What falls on a surface that faces `n` of the light a probe's `nine` numbers hold, as
/// GTA VI's lighting program takes it of a probe.
pub fn falls_on(nine: &[Vec3; 9], n: Vec3) -> Vec3 {
    let Vec3 { x, y, z } = n;
    let fallen = 0.886_227 * nine[0]
        + 1.023_328 * (nine[1] * y + nine[2] * z + nine[3] * x)
        + 0.858_086 * (nine[4] * x * y + nine[5] * y * z + nine[7] * x * z)
        + (0.743_125 * z * z - 0.247_708) * nine[6]
        + 0.429_043 * (x * x - y * y) * nine[8];
    fallen.max(Vec3::ZERO)
}

/// What a cycle states of the clouds over a model and of the directional light under them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clouds {
    /// The share of the directional light the clouds let through, from nought to one: the
    /// mask of that light holds it with the model's own shadow.
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
pub(super) const WORDS: usize = 120;

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
            words[8..12].copy_from_slice(&plane);
        }
        // The nine numbers, each a row; the first row's last word says they are stated.
        for (row, held) in (self.around.iter().flatten()).enumerate() {
            words[12 + row * 4..15 + row * 4].copy_from_slice(&held.to_array());
            words[15] = 1.0;
        }
        if let Some(Dome(dome)) = &self.dome {
            words[48..].copy_from_slice(dome);
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
        assert_eq!(scene.words()[4..], [0.0; 116]);
        scene.bounds = Some((Vec3::new(-1.0, 2.0, 3.0), Vec3::new(3.0, 4.0, 5.0)));
        let words = scene.words();
        assert_eq!(words[4..8], [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(words[8..12], [1.0, 3.0, 3.0 - 0.002 * 4.0, 6.0]);
        scene.laid = None;
        assert_eq!(scene.words()[4..], [0.0; 116], "a box and no ground");
        scene.clouds.open = 1.5;
        assert_eq!(scene.words()[0], 1.0, "held to one");
        // The nine numbers follow in rows of four, and the first row says they are there.
        let mut nine = [Vec3::ZERO; 9];
        nine[8] = Vec3::new(1.0, 2.0, 3.0);
        scene.around = Some(nine);
        let words = scene.words();
        assert_eq!(words[12..16], [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(words[44..48], [1.0, 2.0, 3.0, 0.0]);
        // Then the sky's seventy-two ways, as they are.
        assert_eq!(words[48..], [0.0; 72]);
        let mut dome = [0.5; 72];
        dome[71] = 9.0;
        scene.dome = Some(Dome(dome));
        assert_eq!((scene.words()[48], scene.words()[119]), (0.5, 9.0));
    }

    /// Ways over the upper half, each with the angle its patch fills: `rings` of twice as many.
    fn upper(rings: usize) -> impl Iterator<Item = (Vec3, f32)> {
        use std::f32::consts::PI;
        (0..rings * rings * 2).map(move |at| {
            let tilt = ((at / (rings * 2)) as f32 + 0.5) / rings as f32 * (PI / 2.0);
            let turn = ((at % (rings * 2)) as f32 + 0.5) / rings as f32 * PI;
            let way = Vec3::new(tilt.sin() * turn.cos(), tilt.sin() * turn.sin(), tilt.cos());
            (
                way,
                tilt.sin() * (PI / 2.0 / rings as f32) * (PI / rings as f32),
            )
        })
    }

    #[test]
    fn a_probe_s_nine_numbers_give_back_what_falls_on_a_surface_from_the_sky_and_the_ground() {
        use std::f32::consts::PI;
        // A sky of one every way and a ground of a quarter: a surface that faces up takes pi of
        // the sky, one that faces down pi of the ground, and one that stands takes half of each.
        let sky = upper(8).map(|(way, patch)| (way, Vec3::ONE, patch));
        let even = nine_of(sky, Vec3::splat(0.25));
        let near = |fallen: Vec3, wanted: f32| (fallen.x - wanted).abs() < 0.06 * PI;
        let up = falls_on(&even, Vec3::Z);
        assert!(near(up, PI), "{up:?}");
        assert!(near(falls_on(&even, -Vec3::Z), 0.25 * PI));
        assert!(near(falls_on(&even, Vec3::X), 0.625 * PI));
        // A sky bright towards +x alone lights a surface that faces it, and none that faces away.
        let bright = |way: Vec3| Vec3::splat(way.x.max(0.0).powi(4));
        let sky = upper(8).map(|(way, patch)| (way, bright(way), patch));
        let leaning = nine_of(sky, Vec3::ZERO);
        let (towards, away) = (falls_on(&leaning, Vec3::X), falls_on(&leaning, -Vec3::X));
        assert!(
            towards.x > 0.3 && away.x < 0.1 * towards.x,
            "{towards:?} {away:?}"
        );
        assert_eq!(falls_on(&[Vec3::ZERO; 9], Vec3::Y), Vec3::ZERO);
    }
}
