//! Frames lit through a mask a test states: how the lighting reads the mask and a tile's word.

use super::scene_tests::{frames_stated, leaning};
use super::tests::{surface, SIZE};
use super::*;
use glam::Vec4;

/// A floor across the whole frame at a depth of a tenth, and a sheet at `over` across the frame's
/// upper part: from the place 0.2 of the clip space up.
fn sheet_over_a_floor(over: f32) -> MeshletSurface {
    let floor: [[f32; 3]; 3] = [[-1.0, -1.0, 0.1], [3.0, -1.0, 0.1], [-1.0, 3.0, 0.1]];
    let sheet: [[f32; 3]; 3] = [[-3.0, 0.2, over], [3.0, 0.2, over], [0.0, 6.0, over]];
    let positions: Vec<u32> = (floor.iter().chain(&sheet))
        .flatten()
        .map(|v| v.to_bits())
        .collect();
    let mut surface = surface();
    (surface.feeds).insert(
        (0, 0),
        Feed::Bytes(bytemuck::cast_slice(&positions).to_vec()),
    );
    surface
}

/// State the scene's mask: `roots` a pixel in rows from the top, and the word of its one tile.
fn state(scene: &mut MeshletScene, queue: &wgpu::Queue, roots: &[f32], word: u32) {
    scene.mask.stated = true;
    // The mask is written behind the scene's back.
    scene.forget();
    let write = |view: &wgpu::TextureView, bytes: &[u8], side: u32| {
        queue.write_texture(
            view.texture().as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
    };
    write(&scene.mask.roots, bytemuck::cast_slice(roots), SIZE);
    write(&scene.mask.tiles, bytemuck::bytes_of(&word), 1);
}

/// A root for every pixel of the frame: what `of` gives for the pixel (`x`, `y`) from the top.
fn roots(of: impl Fn(u32, u32) -> f32) -> Vec<f32> {
    (0..SIZE * SIZE)
        .map(|at| of(at % SIZE, at / SIZE))
        .collect()
}

/// The red of the pixel (`x`, `y`) of `frame`, from the top.
fn red(frame: &[u8], (x, y): (u32, u32)) -> u8 {
    frame[((y * SIZE + x) * 4) as usize]
}

/// A light of `strength` that leans, dim enough that the frame's curve does not flatten it.
fn lit(strength: f32) -> Sky {
    Sky {
        sun: Vec3::splat(strength),
        ..leaning()
    }
}

/// A view that takes a place as far again from the camera's plane as it lies deep, so that the
/// floor and what is over it are seen at different depths.
fn deepening() -> Mat4 {
    Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::new(0.0, 0.0, 1.0, 1.0), Vec4::W)
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_frame_is_lit_by_the_square_of_the_mask_s_first_channel() {
    // A root of a half under a light is a root of one under a quarter of that light, and not
    // under half of it: the share is the root's square.
    let frames = [
        (lit(2.0), 0.5),
        (lit(0.5), 1.0),
        (lit(1.0), 1.0),
        (lit(2.0), 1.0),
    ];
    let skies: Vec<(Mat4, Sky)> = (frames.iter())
        .map(|(sky, _)| (Mat4::IDENTITY, *sky))
        .collect();
    let Some(seen) = frames_stated(
        "the_frame_is_lit_by_the_square_of_the_mask",
        sheet_over_a_floor(0.6),
        &skies,
        |scene, queue, at| state(scene, queue, &roots(|_, _| frames[at].1), 0),
    ) else {
        return;
    };
    let at = (8, 11);
    let [masked, quarter, half, whole] = [0, 1, 2, 3].map(|frame| red(&seen[frame], at));
    assert!(
        quarter > 30 && whole > half && half > quarter + 10,
        "{quarter} {half} {whole}"
    );
    assert!(masked.abs_diff(quarter) <= 1, "{masked} {quarter}");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_tile_s_word_has_a_pixel_s_share_rebuilt_from_the_pixels_about_it_that_lie_as_deep() {
    let dim = lit(1.0);
    // A column of the mask at nought and the rest at a root of eight tenths, which its square
    // is not. Of a pixel of the column and the twelve about it, eight hold that square; of the
    // pixel beside the column, eleven.
    let root = 0.8f32;
    let column = roots(|x, _| if x == 8 { 0.0 } else { root });
    let mean = |held: f32| held * root * root / 13.0;
    let uniform = |share: f32| roots(move |_, _| share.sqrt());
    // The sheet's rows at nought and the floor's at one: the sheet ends between the rows 6 and 7.
    let rows = roots(|_, y| f32::from(u8::from(y > 6)));
    let flat = Mat4::IDENTITY;
    type Stated = (Mat4, Vec<f32>, u32);
    let frames: [Stated; 8] = [
        (flat, column.clone(), 0),
        (flat, column.clone(), sun_mask::REBUILD),
        (flat, column.clone(), !sun_mask::REBUILD),
        (flat, uniform(mean(8.0)), 0),
        (flat, uniform(mean(11.0)), 0),
        (deepening(), rows.clone(), sun_mask::REBUILD),
        (deepening(), uniform(1.0), 0),
        (deepening(), uniform(11.0 / 13.0), 0),
    ];
    let skies: Vec<(Mat4, Sky)> = frames.iter().map(|(view, _, _)| (*view, dim)).collect();
    let stating = |scene: &mut MeshletScene, queue: &wgpu::Queue, at: usize| {
        state(scene, queue, &frames[at].1, frames[at].2);
    };
    let test = "a_tile_s_word_has_a_pixel_s_share_rebuilt";
    let Some(seen) = frames_stated(test, sheet_over_a_floor(0.6), &skies, stating) else {
        return;
    };
    let (on, beside) = ((8, 11), (9, 11));
    // With no bit of the tile's set, and with every bit but the one, a pixel has its own share.
    assert_eq!(red(&seen[0], on), 0);
    assert!(red(&seen[0], beside) > 60, "{}", red(&seen[0], beside));
    assert_eq!(seen[2], seen[0], "the other bits of a word ask for nothing");
    // With the bit set, the mean of the thirteen.
    assert!(red(&seen[1], on).abs_diff(red(&seen[3], on)) <= 1);
    assert!(red(&seen[1], beside).abs_diff(red(&seen[4], beside)) <= 1);
    assert!(red(&seen[1], on) + 5 < red(&seen[1], beside));
    // Seen with depth, a floor pixel two rows under the sheet has two of its twelve on the
    // sheet, which lies nearer: they are left out, and the floor's share is the floor's.
    let under = (8, 8);
    assert_eq!(red(&seen[5], under), red(&seen[6], under));
    // With the sheet laid on the floor the same two lie as deep, and are taken in.
    let Some(level) = frames_stated(
        test,
        sheet_over_a_floor(0.1),
        &skies[5..],
        |scene, queue, at| {
            state(scene, queue, &frames[5 + at].1, frames[5 + at].2);
        },
    ) else {
        return;
    };
    assert!(red(&level[0], under).abs_diff(red(&level[2], under)) <= 1);
    assert!(red(&level[0], under) + 5 < red(&level[1], under));
}
