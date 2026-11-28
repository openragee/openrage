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
