//! The lamps that light what blends over the lit frame, which the frame's light passes cannot reach: a lamp's own glass is lit by it from a hand's breadth away, which is what makes the head of a lamp glow.

use glam::Vec3;

/// How many lamps light what blends: the nearest of those the frame lights, which a dense street's lamp heads within a hundred metres need.
pub const WORLD_LAMPS: usize = 256;
/// The floats a lamp takes in the light's buffer: three rows of four.
pub(crate) const LAMP_FLOATS: usize = 12;
/// The most gusts the game's wind keeps alive at once.
pub const WORLD_GUSTS: usize = 2;
/// The floats of the wind after the lamps: the wind's two rows, then each gust's.
pub(crate) const WIND_FLOATS: usize = 8 + 4 * WORLD_GUSTS;

/// The wind the grass bends in, in metres and metres a second, x east and y north: the way it blows, the speeds the game's wind volumes are filled with at the ground and from the falloff height up with one over that height (nought where none), the most speed the game's wind reaches, then each live gust's centre, its strength times its ramp, and the half-width of the square it wraps in; a gust of no strength adds nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorldWind {
    pub direction: [f32; 2],
    pub prevailing: [f32; 2],
    pub rise: f32,
    pub most: f32,
    pub gusts: [[f32; 4]; WORLD_GUSTS],
}

impl WorldWind {
    /// The rows the light's buffer holds after the lamps.
    pub(crate) fn rows(&self) -> [[f32; 4]; 2 + WORLD_GUSTS] {
        let [x, y] = self.direction;
        let [bottom, top] = self.prevailing;
        let mut out = [[x, y, bottom, top]; 2 + WORLD_GUSTS];
        out[1] = [self.rise, self.most, 0.0, 0.0];
        out[2..].copy_from_slice(&self.gusts);
        out
    }
}
/// The floats of the light the world's program reads: the sun's, the ambient's and the frame's five rows, the lamps, then the wind.
pub(crate) const LIGHT_FLOATS: usize = 20 + WORLD_LAMPS * LAMP_FLOATS + WIND_FLOATS;

/// A lamp that lights what blends: where it stands and how far it reaches, its colour times its strength in the frame's units, and for a spot the way it shines with the cosine of its outer cone and one over the span from it to the inner one's; a light that shines all round has no way.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorldLamp {
    pub position: Vec3,
    pub radius: f32,
    pub colour: Vec3,
    pub direction: Vec3,
    pub cone: [f32; 2],
}

impl WorldLamp {
    /// This lamp as a segment from `position - half` to `position + half`, lit from its nearest point: the program knows it by a span below nought.
    pub fn capsule(self, half: Vec3) -> Self {
        Self {
            direction: half,
            cone: [self.cone[0], -1.0],
            ..self
        }
    }
}

/// The rows the program reads of the first [`WORLD_LAMPS`] of `lamps`, each slot past them nought.
pub(crate) fn rows(lamps: &[WorldLamp]) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0; 4]; WORLD_LAMPS * 3];
    let (slots, _) = out.as_chunks_mut::<3>();
    for (slot, l) in slots.iter_mut().zip(lamps) {
        *slot = [
            l.position.extend(l.radius).to_array(),
            l.colour.extend(l.cone[0]).to_array(),
            l.direction.extend(l.cone[1]).to_array(),
        ];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lamp_is_three_rows_and_every_slot_past_the_lamps_is_nought() {
        let lamp = WorldLamp {
            position: Vec3::new(1.0, 2.0, 3.0),
            radius: 9.0,
            colour: Vec3::new(4.0, 5.0, 6.0),
            direction: Vec3::NEG_Z,
            cone: [0.5, 2.0],
        };
        let rows = rows(&vec![lamp; WORLD_LAMPS + 3]);
        assert_eq!(
            rows.len() * 4,
            WORLD_LAMPS * LAMP_FLOATS,
            "the most, and no more"
        );
        assert_eq!(
            rows[..3],
            [
                [1.0, 2.0, 3.0, 9.0],
                [4.0, 5.0, 6.0, 0.5],
                [0.0, 0.0, -1.0, 2.0]
            ]
        );
        let one = super::rows(&[lamp]);
        assert!(one[3..].iter().all(|row| *row == [0.0; 4]));
        let declared = format!("lamps: array<Lamp, {WORLD_LAMPS}>");
        assert!(super::super::shader::WORLD_SHADER.contains(&declared));
    }

    #[test]
    fn the_wind_is_its_way_and_speeds_then_its_rise_and_most_then_a_row_a_gust() {
        let wind = WorldWind {
            direction: [0.0, -1.0],
            prevailing: [3.0, 7.0],
            rise: 0.02,
            most: 20.0,
            gusts: [[400.0, -400.0, 5.0, 800.0], [0.0; 4]],
        };
        let rows = wind.rows();
        assert_eq!(rows.len() * 4, WIND_FLOATS);
        assert_eq!(rows[0], [0.0, -1.0, 3.0, 7.0]);
        assert_eq!(rows[1], [0.02, 20.0, 0.0, 0.0]);
        assert_eq!(rows[2], [400.0, -400.0, 5.0, 800.0]);
        assert_eq!(rows[3], [0.0; 4]);
        let declared = format!("gusts: array<vec4<f32>, {WORLD_GUSTS}>");
        assert!(super::super::shader::WORLD_SHADER.contains(&declared));
    }
}
