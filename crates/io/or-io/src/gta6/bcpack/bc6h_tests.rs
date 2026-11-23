//! Made-up BC6H bodies: blocks coded here into lanes through made-up presets, which the decoder
//! has to give back and a calibration has to find again from a plain level made up to match.

use super::super::calibrate::{calibrate, LevelData, LevelIn, TextureIn};
use super::super::tests::encode;
use super::super::{decode_with, Decode};
use super::*;

/// A block of the mode of one region whose ends are not differences: red, green and blue of its
/// two ends, 10 bits each, and its 16 indices.
#[derive(Clone, Copy)]
struct Ten {
    ends: [[u16; 3]; 2],
    indices: [u8; 16],
}

/// A made-up block: two ends well apart, every index 0 or 15, pixel 0's 0.
fn ten(seed: usize) -> Ten {
    let ends = std::array::from_fn(|end| {
        std::array::from_fn(|channel| ((seed * 37 + channel * 59 + end * 601 + 11) % 1024) as u16)
    });
    let indices = std::array::from_fn(|pixel| {
        let high = pixel == 15 || (pixel != 0 && (seed + pixel).is_multiple_of(3));
        if high {
            15
        } else {
            0
        }
    });
    Ten { ends, indices }
}

/// `block`'s fields, each left-aligned in 16 bits.
fn ten_fields(block: &Ten) -> [u16; FIELDS] {
    std::array::from_fn(|field| match field {
        0..6 => block.ends[field / 3][field % 3] << 6,
        // What the block leaves for the second region: its base, then its other end.
        field => block.ends[(field - 6) / 3][field % 3] << 6,
    })
}

/// `block` as the 16 bytes of a BC6H block.
fn ten_bytes(block: &Ten) -> [u8; 16] {
    write(&shapes()[10], &ten_fields(block), 0, &block.indices)
}

/// The symbol of the difference `delta` in a field of `bits` bits.
fn symbol(delta: i32, bits: u32) -> u16 {
    let size = 1 << bits;
    let delta = delta.rem_euclid(size);
    if delta < size / 2 {
        (2 * delta) as u16
    } else {
        (2 * (size - delta) - 1) as u16
    }
}

/// A symbol as the two symbols of half its bits that interleave into it.
fn halves(symbol: u16) -> [u16; 2] {
    std::array::from_fn(|half| {
        (0..8).fold(0, |all, bit| {
            all | (((symbol >> (2 * bit + half)) & 1) << bit)
        })
    })
}

/// The lanes of `blocks`: block `b`'s endpoints and indices are differences from the blocks
/// `backs(b)` blocks back, 0 for nothing, a reference past the first block reading `presets`.
fn ten_lanes(
    blocks: &[Ten],
    presets: &Presets,
    backs: impl Fn(usize) -> (usize, usize),
) -> Vec<Vec<u16>> {
    let mut lanes = vec![vec![0u16; 2 * blocks.len()]; 16];
    for (b, block) in blocks.iter().enumerate() {
        let (end_back, index_back) = backs(b);
        let at = 2 * b;
        let moved = symbol(end_back as i32 - index_back as i32, 8);
        lanes[0][at..at + 2].copy_from_slice(&halves(moved));
        lanes[1][at..at + 2].copy_from_slice(&halves(index_back as u16));
        lanes[15][at] = 10;
        let fields = ten_fields(block);
        for (field, &value) in fields.iter().enumerate().take(6) {
            let from = match end_back {
                0 => 0x8000,
                back if back <= b => ten_fields(&blocks[b - back])[field],
                back => presets.ends[256 + b - back][field],
            };
            let delta = i32::from(value >> 6) - i32::from(from >> 6);
            lanes[2 + field][at..at + 2].copy_from_slice(&halves(symbol(delta, 10)));
        }
        for pixel in 0..16 {
            // An index with no reference is a difference from half its range.
            let from = match index_back {
                0 => 8,
                back if back <= b => blocks[b - back].indices[pixel],
                back => presets.first[256 + b - back][pixel],
            };
            let field = symbol(i32::from(block.indices[pixel]) - i32::from(from), 4);
            let (row, half, odd) = place(pixel);
            for bit in 0..4 {
                lanes[11 + row][at + half] |= ((field >> bit) & 1) << (2 * bit + odd);
            }
        }
    }
    lanes
}

/// Presets no pack holds: a value for everything a block of [`ten`] reads of `slots`, each marked settled.
fn made_up_presets(slots: &[usize]) -> Presets {
    let mut presets = Presets::empty();
    for &slot in slots {
        for (field, value) in presets.ends[slot].iter_mut().enumerate().take(6) {
            *value = (((slot * 13 + field * 229 + 5) % 1024) as u16) << 6;
        }
        presets.ends_bits[slot] = [16; FIELDS];
        for (pixel, index) in presets.first[slot].iter_mut().enumerate() {
            *index = ((slot + 5 * pixel) % 16) as u8;
        }
        presets.first_known[slot] = true;
    }
    presets
}

/// The level above `level`, `width` by `height` blocks of [`ten`]: twice as wide and as high, each pixel repeated 2 by 2.
fn level_above(level: &[Ten], width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * 16 * level.len());
    for y in 0..2 * height {
        for x in 0..2 * width {
            let below = &level[(y / 2) * width + x / 2];
            let mut block = *below;
            for (texel, index) in block.indices.iter_mut().enumerate() {
                let (across, down) = (2 * (x % 2) + (texel % 4) / 2, 2 * (y % 2) + texel / 8);
                *index = below.indices[4 * down + across];
            }
            // A block whose first index would have its top bit set is stored with its ends swapped.
            if block.indices[0] == 15 {
                block.ends.swap(0, 1);
                for index in &mut block.indices {
                    *index = 15 - *index;
                }
            }
            out.extend(ten_bytes(&block));
        }
    }
    out
}

#[test]
fn every_mode_s_block_is_written_as_another_decoder_reads_it() {
    for (mode, shape) in shapes().iter().enumerate() {
        for seed in 0..40usize {
            let fields: [u16; FIELDS] = std::array::from_fn(|field| {
                let made = (seed * 7919 + field * 104_729 + mode * 31 + 1) as u32;
                let made = made.wrapping_mul(2_654_435_761) >> 16;
                (made as u16) & ((0xFFFFu32 << (16 - shape.depth(field).max(1))) as u16)
            });
            let partition = (seed * 5 + mode) % 32;
            let anchors = anchors(shape.regions, partition);
            let depth = shape.index_depth();
            let indices: [u8; 16] = std::array::from_fn(|pixel| {
                let index = ((seed + 3 * pixel + mode) % (1 << depth)) as u8;
                if anchors.contains(&pixel) {
                    index >> 1
                } else {
                    index
                }
            });
            let bytes = write(shape, &fields, partition, &indices);
            let mut halves = [0u16; 48];
            bcdec_rs::bc6h_half(&bytes, &mut halves, 12, false);
            let drawn = draw(shape, &fields, partition, &indices);
            for (pixel, drawn) in drawn.iter().enumerate() {
                // The format's last step, from the 16 bits it interpolates to a half's bits.
                let want = drawn.map(|value| ((value * 31) >> 6) as u16);
                assert_eq!(
                    halves[3 * pixel..3 * pixel + 3],
                    want,
                    "mode {mode}, seed {seed}"
                );
            }
        }
    }
}

#[test]
fn a_body_decodes_to_the_blocks_it_was_coded_from() {
    let made: Vec<Ten> = (0..40).map(ten).collect();
    let presets = made_up_presets(&[253, 254]);
    // Block 0 reads slot 253 for its endpoints and slot 254 for its indices; every seventh block's
    // endpoints come from nothing; the rest refer back, to different blocks for the two.
    let backs = |b: usize| match b {
        0 => (3, 2),
        b if b.is_multiple_of(7) => (0, b),
        b => (1 + b / 2, b - b / 3),
    };
    let lanes = ten_lanes(&made, &presets, backs);
    let truth: Vec<u8> = made.iter().flat_map(ten_bytes).collect();

    let rebuilt = blocks(&lanes, &presets).expect("decodes");
    assert!(rebuilt.bytes == truth);
    assert_eq!(rebuilt.unknown, 0);
    // Every block's indices chain back to block 0's, which slot 254 gives.
    let blind = blocks(&lanes, &Presets::empty()).expect("decodes");
    assert_eq!(blind.unknown, 40);
    assert!(blind.bytes != truth);

    // A walk keeps no more of a long body than of a short one: every block refers 255 back.
    let long: Vec<Ten> = (0..1000).map(ten).collect();
    let presets = made_up_presets(&(1..256).collect::<Vec<_>>());
    let lanes = ten_lanes(&long, &presets, |_| (255, 255));
    let truth: Vec<u8> = long.iter().flat_map(ten_bytes).collect();
    assert!(blocks(&lanes, &presets).expect("decodes").bytes == truth);

    let mut lanes = lanes;
    lanes[15][0] = 14;
    assert!(blocks(&lanes, &presets).is_err(), "no mode 14");
}

#[test]
fn a_block_of_one_region_leaves_its_base_and_its_other_end_for_the_second_region() {
    // A root block of the mode of an 11-bit base and 9-bit differences, then a block of two
    // regions, 10 bits and 5, that refers to it and adds nothing.
    let mut lanes = vec![vec![0u16; 4]; 16];
    lanes[15][0] = 11;
    let (base, other) = (0b101_0110_0111_u16, 0b1_0011_0101_u16);
    for channel in 0..3 {
        // A root's field is a difference from the top bit alone.
        let from_half = |value: u16, bits: u32| symbol(i32::from(value) - (1 << (bits - 1)), bits);
        lanes[2 + channel][..2].copy_from_slice(&halves(from_half(base, 11)));
        lanes[5 + channel][..2].copy_from_slice(&halves(from_half(other, 9)));
    }
    lanes[0][2..].copy_from_slice(&halves(symbol(1, 8)));
    let presets = Presets::empty();
    let mut walk = Walk::new(&lanes, &presets, false).expect("whole lanes");
    let (first, _) = walk.step(0).expect("a mode");
    assert_eq!((first.regions, first.base), (1, 11));
    assert_eq!(walk.fields(0)[0], base << 5);
    assert_eq!(walk.fields(0)[3], other << 7);
    // The base's low 9 bits with the top one of them flipped, and the other end as it is.
    assert_eq!(walk.fields(0)[6], ((base & 0x1FF) ^ 0x100) << 7);
    assert_eq!(walk.fields(0)[9], other << 7);

    let (second, _) = walk.step(1).expect("a mode");
    assert_eq!((second.regions, second.base), (2, 10));
    let fields = *walk.fields(1);
    assert_eq!(fields[0], (base >> 1) << 6);
    assert_eq!(fields[3], (other >> 4) << 11);
    assert_eq!(fields[6], (((base & 0x1FF) ^ 0x100) >> 4) << 11);
    assert_eq!(fields[9], (other >> 4) << 11);

    // The mode whose ends are not differences leaves its base whole.
    lanes[15][0] = 10;
    let mut walk = Walk::new(&lanes, &presets, false).expect("whole lanes");
    walk.step(0).expect("a mode");
    assert_eq!(walk.fields(0)[6], walk.fields(0)[0]);
    assert_eq!(walk.fields(0)[9], walk.fields(0)[3]);
}

#[test]
fn a_calibration_finds_made_up_presets_again_and_the_body_then_decodes() {
    let (width, height) = (8, 8);
    let level: Vec<Ten> = (0..width * height).map(ten).collect();
    let made = made_up_presets(&[254, 255]);
    // Even blocks take their endpoints from nothing and their indices, through block 0, from slot
    // 255; odd blocks take their endpoints, through block 1, from slot 254 and their indices from
    // nothing, but for some of the later ones, which hang on both slots.
    let backs = |b: usize| match b {
        0 => (0, 1),
        1 => (3, 0),
        b if b >= 32 && b % 4 == 3 => (2, 1),
        _ => (2, 2),
    };
    let lanes = ten_lanes(&level, &made, backs);
    let truth: Vec<u8> = level.iter().flat_map(ten_bytes).collect();
    let top = level_above(&level, width, height);
    let texture = || TextureIn {
        kind: 6,
        levels: vec![
            LevelIn {
                width: 2 * width,
                height: 2 * height,
                data: LevelData::Plain(top.clone()),
            },
            LevelIn {
                width,
                height,
                data: LevelData::Coded {
                    lanes: lanes.clone(),
                    at: (0..(width * height) as u32).collect(),
                },
            },
        ],
    };

    let (tables, summaries) =
        calibrate(1, &|_| vec![texture()], &mut |_, _| {}, &|| false).expect("not stopped");
    let found = tables.hdr();
    assert_eq!(found.ends[254][..6], made.ends[254][..6]);
    assert_eq!(found.ends_bits[254][..6], [16; 6]);
    assert_eq!(found.first[255], made.first[255]);
    assert!(found.first_known[255]);
    let summary = (summaries.iter())
        .find(|summary| summary.kind == 6)
        .expect("BC6H's");
    assert_eq!(
        (summary.bodies, summary.blocks, summary.unknown),
        (1, 64, 0)
    );
    assert!(summary.mean_mismatch().is_some_and(|mean| mean < 0.01));

    let stream = encode(6, &lanes);
    let size = truth.len() as u64;
    let (decoded, how) = decode_with(6, &stream, size, Some(&tables)).expect("decodes");
    assert!(decoded == truth);
    let all_fitted = Decode {
        calibrated: 64,
        unknown: 0,
    };
    assert_eq!(how, all_fitted);
    assert!(decode_with(6, &stream, size, None).is_err());
    let kept = super::super::Tables::from_bytes(&tables.to_bytes()).expect("reads back");
    assert!(kept == tables);
}

#[test]
fn plain_levels_that_are_not_one_another_averaged_down_are_no_targets() {
    let (width, height) = (4, 4);
    let level: Vec<Ten> = (0..width * height).map(ten).collect();
    let made = made_up_presets(&[254, 255]);
    // As in the fit above: even blocks hang on the index slot alone, odd ones on the endpoint slot alone.
    let backs = |b: usize| match b {
        0 => (0, 1),
        1 => (3, 0),
        _ => (2, 2),
    };
    let lanes = ten_lanes(&level, &made, backs);
    let top = level_above(&level, width, height);
    // A level above that one which is as bright as a block gets everywhere.
    let bright = Ten {
        ends: [[1023; 3]; 2],
        indices: [0; 16],
    };
    let far: Vec<u8> = (0..16 * width * height)
        .flat_map(|_| ten_bytes(&bright))
        .collect();
    let plain = |blocks: &Vec<u8>, scale: usize| LevelIn {
        width: scale * width,
        height: scale * height,
        data: LevelData::Plain(blocks.clone()),
    };
    let coded = || LevelIn {
        width,
        height,
        data: LevelData::Coded {
            lanes: lanes.clone(),
            at: (0..(width * height) as u32).collect(),
        },
    };
    let run = |with_far: bool| {
        let texture = || TextureIn {
            kind: 6,
            levels: (with_far.then(|| plain(&far, 4)).into_iter())
                .chain([plain(&top, 2), coded()])
                .collect(),
        };
        calibrate(1, &|_| vec![texture()], &mut |_, _| {}, &|| false).expect("not stopped")
    };
    let (tables, _) = run(false);
    assert!(tables.covers(6));
    let (tables, summaries) = run(true);
    assert!(!tables.covers(6));
    let summary = (summaries.iter())
        .find(|summary| summary.kind == 6)
        .expect("BC6H's");
    assert_eq!((summary.blocks, summary.compared), (16, 0));
}
