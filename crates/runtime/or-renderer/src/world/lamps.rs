//! The lamps that light what blends over the lit frame, which the frame's light passes cannot reach: a lamp's own glass is lit by it from a hand's breadth away, which is what makes the head of a lamp glow.

use glam::Vec3;

/// How many lamps light what blends: the nearest of those the frame lights, which a dense street's lamp heads within a hundred metres need.
pub const WORLD_LAMPS: usize = 256;
/// The floats a lamp takes in the light's buffer: three rows of four.
pub(crate) const LAMP_FLOATS: usize = 12;
/// The floats of the light the world's program reads: the sun's, the ambient's and the frame's five rows, then the lamps.
pub(crate) const LIGHT_FLOATS: usize = 20 + WORLD_LAMPS * LAMP_FLOATS;

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
}
