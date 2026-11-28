//! The two ambient scales an instance takes of the frame's light, as the selection states them and
//! the G-buffer's program reads them.

/// The ambient scales of what stands outdoors: the whole of the natural light and none of the artificial.
pub const OUTDOORS: [f32; 2] = [1.0, 0.0];

/// The ambient scales of what stands in an opening between a room and the outdoors: a face whose normal along the model's `axis` (0 to 2) has the sign of `out` takes [`OUTDOORS`], any other face `inward`, whose natural share is 0 to 1.
// ⚠ Packed in the natural share as `-(2 + 2·(2·axis + [out < 0]) + natural)`, which the G-buffer's program unpacks: a negative share is no other instance's.
pub fn by_face(axis: usize, out: f32, inward: [f32; 2]) -> [f32; 2] {
    let code = 2 * axis.min(2) + usize::from(out < 0.0);
    [
        -(2.0 + 2.0 * code as f32 + inward[0].clamp(0.0, 1.0)),
        inward[1],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The axis, the sign of the way out and the natural share of a packed share, by the law the G-buffer's program unpacks it by.
    fn unpacked(natural: f32) -> (usize, f32, f32) {
        let v = -natural - 2.0;
        let code = (v * 0.5 + 0.25).floor();
        let out = if code as usize & 1 == 1 { -1.0 } else { 1.0 };
        (code as usize / 2, out, v - 2.0 * code)
    }

    #[test]
    fn a_share_packed_by_face_unpacks_to_its_axis_its_way_out_and_the_room_s_share() {
        for axis in 0..3 {
            for out in [1.0, -1.0] {
                for natural in [0.0, 0.375, 1.0] {
                    let [packed, artificial] = by_face(axis, out, [natural, 0.25]);
                    assert!(packed < 0.0, "no share of another instance is negative");
                    assert_eq!(unpacked(packed), (axis, out, natural));
                    assert_eq!(artificial, 0.25);
                }
            }
        }
        // An axis past the third is the third, and a share past the whole is the whole.
        assert_eq!(by_face(7, 1.0, [3.0, 0.0]), by_face(2, 1.0, [1.0, 0.0]));
        assert!(by_face(0, 1.0, OUTDOORS)[0] < OUTDOORS[0]);
    }
}
