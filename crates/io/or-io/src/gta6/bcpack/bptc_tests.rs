//! Made-up BC7 bodies: blocks coded here into lanes through made-up presets, which the decoder
//! has to give back and a calibration has to find again from a plain level made up to match.
// Outside a test build the made-up fixtures take a few of these helpers and the rest lie unused.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use std::collections::BTreeSet;

use super::super::calibrate::{
    calibrate, probe_tables, Among, LevelData, LevelIn, Moved, Tables, TextureIn,
};
use super::super::tests::encode;
use super::super::{decode_with, Decode};
use super::*;

/// How many ranks of lane 15 each mode takes: its partitions, or its rotations and selections.
const COUNTS: [usize; 8] = [16, 64, 64, 64, 8, 4, 1, 64];

/// A table of modes no pack holds: each mode's ranks in one run, the modes in the order of their numbers.
fn made_up_modes() -> [u8; RANKS] {
    let mut modes = [UNPLACED; RANKS];
    let mut rank = 0;
    for (mode, count) in COUNTS.into_iter().enumerate() {
        modes[rank..rank + count].fill(mode as u8);
        rank += count;
    }
    modes
}

/// Presets that hold the made-up table of modes and nothing else.
fn blank() -> Presets {
    Presets {
        modes: made_up_modes(),
        ..Presets::empty()
    }
}

/// A mode 6 block: red, green, blue and alpha at each of its two ends, left-aligned in 8 bits, and its 16 indices.
#[derive(Clone, Copy)]
struct Six {
    ends: [[u8; 2]; 4],
    indices: [u8; 16],
}

/// A made-up block: red takes all 8 bits, its lowest the end's p-bit, the others 7; every index is 0 or 15, pixel 0's 0.
fn six(seed: usize) -> Six {
    let ends = std::array::from_fn(|channel| {
        std::array::from_fn(|end| {
            let value = ((seed * 37 + (2 * channel + end) * 59 + 11) % 256) as u8;
            let mask = if channel == 0 { 0xFF } else { 0xFE };
            value & mask
        })
    });
    let indices = std::array::from_fn(|pixel| {
        let high = pixel == 15 || (pixel != 0 && (seed + pixel).is_multiple_of(3));
        if high {
            15
        } else {
            0
        }
    });
    Six { ends, indices }
}

/// `block` as the 16 bytes of a BC7 block of mode 6.
fn six_bytes(block: &Six) -> [u8; 16] {
    let (mut word, mut at) = (0u128, 0u32);
    let mut put = |value: u32, bits: u32| {
        word |= u128::from(value) << at;
        at += bits;
    };
    put(1 << 6, 7);
    for channel in block.ends {
        for end in channel {
            put(u32::from(end >> 1), 7);
        }
    }
    put(u32::from(block.ends[0][0] & 1), 1);
    put(u32::from(block.ends[0][1] & 1), 1);
    for (pixel, &index) in block.indices.iter().enumerate() {
        put(u32::from(index), if pixel == 0 { 3 } else { 4 });
    }
    word.to_le_bytes()
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

/// An 8-bit symbol as the two 4-bit symbols that interleave into it.
fn halves(symbol: u16) -> [u16; 2] {
    std::array::from_fn(|half| {
        (0..4).fold(0, |all, bit| {
            all | (((symbol >> (2 * bit + half)) & 1) << bit)
        })
    })
}

/// Lane 15's two symbols for a block of signature `sig`.
fn pair_of(sig: Sig) -> [u16; 2] {
    (0..32)
        .flat_map(|first| (0..16).map(move |second| [first, second]))
        .find(|&[first, second]| Sig::of(first, second, &made_up_modes()) == Some(sig))
        .expect("a pair names every signature")
}

/// The lanes of `blocks`, each of mode 6: block `b`'s endpoints and indices are differences from
/// the blocks `backs(b)` blocks back, 0 for nothing, a reference past the first block reading `presets`.
fn six_lanes(
    blocks: &[Six],
    presets: &Presets,
    backs: impl Fn(usize) -> (usize, usize),
) -> Vec<Vec<u16>> {
    let mut lanes = vec![vec![0u16; 2 * blocks.len()]; 16];
    let sig = pair_of(Sig {
        mode: 6,
        position: 0,
    });
    for (b, block) in blocks.iter().enumerate() {
        let (end_back, index_back) = backs(b);
        let at = 2 * b;
        let moved = symbol(end_back as i32 - index_back as i32, 8);
        lanes[0][at..at + 2].copy_from_slice(&halves(moved));
        lanes[1][at..at + 2].copy_from_slice(&halves(index_back as u16));
        lanes[15][at..at + 2].copy_from_slice(&sig);
        for channel in 0..4 {
            let bits = if channel == 0 { 8 } else { 7 };
            for end in 0..2 {
                let from = match end_back {
                    0 => 0,
                    back if back <= b => blocks[b - back].ends[channel][end],
                    back => presets.ends[256 + b - back][2 * channel + end],
                };
                let delta = i32::from(block.ends[channel][end] >> (8 - bits))
                    - i32::from(from >> (8 - bits));
                lanes[2 + channel][at + end] = symbol(delta, bits);
            }
        }
        for pixel in 0..16 {
            let from = match index_back {
                0 => 0,
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

/// The stream of `count` made-up blocks of mode 6 coded through `presets`, at most 127, and the
/// size it decodes to. Block `b` takes its ends and its indices from slot `255 - b`, and its
/// anchor's index is 7 in an even block and 0 in an odd one: the two ends of what an anchor holds.
pub(in crate::gta6::bcpack) fn body_over_the_slots(
    presets: &Presets,
    count: usize,
) -> (Vec<u8>, u64) {
    (
        body_from_the_slots(presets, count, count),
        16 * count as u64,
    )
}

/// [`body_over_the_slots`] with only the first `slots` blocks taken from a slot, and the blocks
/// after them from nothing.
pub(in crate::gta6::bcpack) fn body_from_the_slots(
    presets: &Presets,
    count: usize,
    slots: usize,
) -> Vec<u8> {
    let made: Vec<Six> = (0..count)
        .map(|b| {
            let mut block = six(b);
            block.indices[0] = if b % 2 == 0 { 7 } else { 0 };
            block
        })
        .collect();
    let lanes = six_lanes(&made, presets, |b| match b < slots {
        true => (1 + 2 * b, 1 + 2 * b),
        false => (0, 0),
    });
    encode(7, &lanes)
}

/// Presets no pack holds: a value for everything a mode 6 block reads of `slots`, each marked known.
fn made_up_presets(slots: &[usize]) -> Presets {
    let mut presets = blank();
    for &slot in slots {
        for (value, end) in presets.ends[slot].iter_mut().enumerate() {
            // Past red's two ends a mode 6 value has 7 bits.
            let mask = if value < 2 { 0xFF } else { 0xFE };
            *end = ((slot * 13 + value * 29 + 5) % 256) as u8 & mask;
        }
        presets.ends_known[slot] = [true; VALUES];
        for (pixel, index) in presets.first[slot].iter_mut().enumerate() {
            *index = ((slot + 5 * pixel) % 16) as u8;
        }
        presets.first_known[slot] = true;
    }
    presets
}

/// The level above `level`, `width` by `height` blocks of [`six`]: twice as wide and as high, each pixel repeated 2 by 2.
fn six_level_above(level: &[Six], width: usize, height: usize) -> Vec<u8> {
    (six_above(level, width, height).iter())
        .flat_map(six_bytes)
        .collect()
}

/// The blocks of [`six_level_above`].
fn six_above(level: &[Six], width: usize, height: usize) -> Vec<Six> {
    let mut out = Vec::with_capacity(4 * level.len());
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
                for channel in &mut block.ends {
                    channel.swap(0, 1);
                }
                for index in &mut block.indices {
                    *index = 15 - *index;
                }
            }
            out.push(block);
        }
    }
    out
}

#[test]
fn lane_15_names_every_signature_of_every_mode_once() {
    let mut seen = BTreeSet::new();
    let mut modes = [0usize; 8];
    let mut none = 0;
    for first in 0..32 {
        for second in 0..16 {
            match Sig::of(first, second, &made_up_modes()) {
                Some(sig) => {
                    assert!(seen.insert((sig.mode, sig.position)), "{sig:?} twice");
                    modes[sig.mode] += 1;
                }
                None => none += 1,
            }
        }
    }
    // Partitions, or rotations and index selections: 16, 64, 64, 64, 4 by 2, 4, 1 and 64.
    assert_eq!(modes, COUNTS);
    assert_eq!(none, 32 * 16 - 285);
    let lowest = Sig {
        mode: 0,
        position: 0,
    };
    assert_eq!(Sig::of(0, 0, &made_up_modes()), Some(lowest));
    assert_eq!(Sig::of(0, 16, &made_up_modes()), None);
    assert_eq!(Sig::of(32, 0, &made_up_modes()), None);
    // A table with a rank too many for a mode names nothing there, and an empty one names nothing at all.
    let mut crowded = made_up_modes();
    crowded[285] = 6;
    assert_eq!(Sig::of(0, 0, &crowded), Some(lowest));
    let past = (0..32)
        .flat_map(|first| (0..16).map(move |second| (first, second)))
        .find(|&(first, second)| rank(first, second) == Some(285))
        .expect("a pair has every rank");
    assert_eq!(Sig::of(past.0, past.1, &crowded), None);
    assert_eq!(Sig::of(0, 0, &[UNPLACED; RANKS]), None);
}

#[test]
fn a_walk_keeps_no_more_of_a_long_body_than_of_a_short_one() {
    // Mode 6 blocks that each refer 255 blocks back, the furthest a reference reaches, over more blocks than a walk keeps.
    let made: Vec<Six> = (0..1000).map(six).collect();
    let presets = made_up_presets(&(1..256).collect::<Vec<_>>());
    let lanes = six_lanes(&made, &presets, |_| (255, 255));
    let truth: Vec<u8> = made.iter().flat_map(six_bytes).collect();
    let rebuilt = blocks(&lanes, &presets).expect("decodes");
    assert!(rebuilt.bytes == truth);
    assert_eq!(rebuilt.unknown, 0);
}

#[test]
fn a_body_of_mode_6_blocks_decodes_to_the_blocks_it_was_coded_from() {
    let made: Vec<Six> = (0..40).map(six).collect();
    let presets = made_up_presets(&[253, 254]);
    // Block 0 reads slot 253 for its endpoints and slot 254 for its indices; every seventh block's
    // endpoints come from nothing; the rest refer back, to different blocks for the two.
    let backs = |b: usize| match b {
        0 => (3, 2),
        b if b.is_multiple_of(7) => (0, b),
        b => (1 + b / 2, b - b / 3),
    };
    let lanes = six_lanes(&made, &presets, backs);
    let truth: Vec<u8> = made.iter().flat_map(six_bytes).collect();

    let rebuilt = blocks(&lanes, &presets).expect("decodes");
    assert!(rebuilt.bytes == truth);
    assert_eq!(rebuilt.unknown, 0);

    // Every block's indices chain back to block 0's, which slot 254 gives.
    let blind = blocks(&lanes, &blank()).expect("decodes");
    // With no table of modes at all, lane 15 names nothing.
    assert!(blocks(&lanes, &Presets::empty()).is_err());
    assert_eq!(blind.unknown, 40);
    assert!(blind.bytes != truth);

    // Another decoder draws block 0's first pixel, whose index is 0, as the block's first ends.
    let mut rgba = [0u8; 64];
    bcdec_rs::bc7(&truth[..16], &mut rgba, 16);
    let [red, green, blue, alpha] = made[0].ends.map(|channel| channel[0]);
    let p = red & 1;
    assert_eq!(rgba[..4], [red, green | p, blue | p, alpha | p]);
}

#[test]
fn a_narrower_mode_keeps_the_top_bits_and_its_second_subset_takes_the_first_ones_ends() {
    let first = six(5);
    let mut lanes = six_lanes(&[first], &blank(), |_| (0, 0));
    // Block 1 is of mode 1: its endpoints are block 0's with nothing added, its indices all 0.
    for lane in &mut lanes {
        lane.extend([0, 0]);
    }
    let narrower = Sig {
        mode: 1,
        position: 20,
    };
    lanes[15][2..].copy_from_slice(&pair_of(narrower));
    lanes[0][2..].copy_from_slice(&halves(symbol(1, 8)));

    let rebuilt = blocks(&lanes, &blank()).expect("decodes");
    assert_eq!(rebuilt.unknown, 0);
    let mut rgba = [0u8; 64];
    bcdec_rs::bc7(&rebuilt.bytes[16..], &mut rgba, 16);
    // Red keeps 7 of its 8 bits, the lowest of them the p-bit a subset's ends share; green and
    // blue keep 6 and end on that p-bit. Seven bits are drawn with their top bit repeated.
    let red = u32::from(first.ends[0][0] >> 1);
    let seven = |value: u8| (u32::from(value >> 2) << 1) | (red & 1);
    let wide = |value: u32| ((value << 1) | (value >> 6)) as u8;
    let pixel = [
        wide(red),
        wide(seven(first.ends[1][0])),
        wide(seven(first.ends[2][0])),
        255,
    ];
    for drawn in rgba.as_chunks::<4>().0 {
        assert_eq!(*drawn, pixel);
    }
}

#[test]
fn a_block_no_encoder_writes_is_counted_as_stray() {
    let mut made: Vec<Six> = (0..4).map(six).collect();
    let stray = |made: &[Six]| {
        let lanes = six_lanes(made, &blank(), |_| (0, 0));
        blocks(&lanes, &blank()).expect("decodes").stray
    };
    assert_eq!(stray(&made), 0);
    // Pixel 0 is the anchor of a block of one subset, and a block does not store its top bit.
    made[2].indices[0] = 9;
    assert_eq!(stray(&made), 1);

    // A block of mode 1 that adds nothing to a block of mode 6: its red ends keep 7 bits, the
    // lowest of them the p-bit the two share.
    let after = |red: [u8; 2]| {
        let mut first = six(5);
        first.ends[0] = red;
        let mut lanes = six_lanes(&[first], &blank(), |_| (0, 0));
        for lane in &mut lanes {
            lane.extend([0, 0]);
        }
        let narrower = Sig {
            mode: 1,
            position: 20,
        };
        lanes[15][2..].copy_from_slice(&pair_of(narrower));
        lanes[0][2..].copy_from_slice(&halves(symbol(1, 8)));
        blocks(&lanes, &blank()).expect("decodes").stray
    };
    assert_eq!(after([0b1100_0110, 0b0100_0110]), 0);
    assert_eq!(after([0b1100_0100, 0b0100_0110]), 1);
}

/// An 8 by 8 block level coded through made-up presets of slots 254 and 255, under the level
/// above it: the mip chain as a fit reads it, the presets, the level's lanes and its blocks.
fn chain() -> (impl Fn() -> TextureIn, Presets, Vec<Vec<u16>>, Vec<u8>) {
    let (width, height) = (8, 8);
    let level: Vec<Six> = (0..width * height).map(six).collect();
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
    let lanes = six_lanes(&level, &made, backs);
    let truth: Vec<u8> = level.iter().flat_map(six_bytes).collect();
    let top = six_level_above(&level, width, height);
    let coded = lanes.clone();
    let texture = move || TextureIn {
        signed: false,
        kind: 7,
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
                    lanes: coded.clone(),
                    at: (0..(width * height) as u32).collect(),
                },
            },
        ],
    };
    (texture, made, lanes, truth)
}

/// What a probe of `texture`, read through `tables`, found for the mode's ends and for its indices.
fn probed(texture: &(impl Fn() -> TextureIn + Sync), tables: &Tables) -> (Vec<Moved>, Vec<Moved>) {
    let read = |_: usize, _: bool| vec![texture()];
    let found = probe_tables(1, &read, tables, &mut |_, _| {}, &|| false).expect("not stopped");
    let of = |indices: bool| found.of(7, indices).expect("its table").values.clone();
    (of(false), of(true))
}

#[test]
fn a_probe_finds_a_preset_s_own_values_no_worse_and_one_moved_a_step_beaten() {
    let (texture, made, _, _) = chain();
    let through = |presets: &Presets| Tables {
        bptc: presets.clone(),
        ..Tables::empty()
    };
    // The level above holds the coded level's own pixels: no value moved a step does better.
    let (ends, first) = probed(&texture, &through(&made));
    let beaten = |moved: &Moved| moved.among() == Some(Among::Beaten);
    assert!(!ends.iter().chain(&first).any(beaten));
    // The odd blocks hang on slot 254's ends and the even ones on slot 255's indices.
    let (green, pixel) = (VALUES * 254 + 2, 16 * 255 + 5);
    assert!(ends[green].blocks > 0 && first[pixel].blocks > 0);
    assert_eq!(
        ends[VALUES * 255 + 2].among(),
        None,
        "a value no block reads"
    );

    // A mode 6 block keeps 7 bits of green, so the least step it answers to is 2.
    let mut off = made.clone();
    off.ends[254][2] = off.ends[254][2].wrapping_add(2);
    off.first[255][5] = (off.first[255][5] + 1) & 15;
    let (ends, first) = probed(&texture, &through(&off));
    assert!(beaten(&ends[green]) && beaten(&first[pixel]));
    assert!(ends[green].lower < ends[green].held && first[pixel].lower < first[pixel].held);
    // Every other value still stands.
    let others = |values: &[Moved], at: usize| {
        (values.iter().enumerate())
            .filter(|&(other, moved)| other != at && beaten(moved))
            .count()
    };
    assert_eq!((others(&ends, green), others(&first, pixel)), (0, 0));
}

#[test]
fn a_calibration_finds_made_up_presets_again_and_the_body_then_decodes() {
    let (texture, made, lanes, truth) = chain();

    let (tables, summaries) =
        calibrate(1, &|_, _| vec![texture()], &mut |_, _| {}, &|| false).expect("not stopped");
    let found = tables.bptc();
    // The one rank the body carries is given its mode; no other rank is.
    let named: Vec<u8> = (found.modes.iter().copied())
        .filter(|&mode| mode != UNPLACED)
        .collect();
    assert_eq!(named, [6]);
    assert_eq!(found.ends[254][..8], made.ends[254][..8]);
    assert!(found.ends_known[254][..8].iter().all(|&known| known));
    assert_eq!(found.first[255], made.first[255]);
    assert!(found.first_known[255]);
    let summary = (summaries.iter())
        .find(|summary| summary.kind == 7)
        .expect("BC7's");
    assert_eq!(
        (summary.bodies, summary.blocks, summary.unknown),
        (1, 64, 0)
    );
    assert!(summary.mean_mismatch().is_some_and(|mean| mean < 0.01));

    let stream = encode(7, &lanes);
    let size = truth.len() as u64;
    let (decoded, how) = decode_with(7, &stream, size, Some(&tables)).expect("decodes");
    assert!(decoded == truth);
    let all_fitted = Decode {
        calibrated: 64,
        ..Decode::default()
    };
    assert_eq!(how, all_fitted);
    assert!(decode_with(7, &stream, size, None).is_err());
    let kept = super::super::Tables::from_bytes(&tables.to_bytes()).expect("reads back");
    assert!(kept == tables);
}

#[test]
fn a_word_of_traced_values_is_cut_and_added_to_as_each_of_its_values_alone() {
    let word = 0x0F80_7FFF_0180_FE31_u64;
    for (keep, adds) in [(0xFF_u8, 0x01_u8), (0xF0, 0x90), (0x80, 0x80), (0xFC, 0xFC)] {
        let bytes = word
            .to_le_bytes()
            .map(|value| (value & keep).wrapping_add(adds));
        assert_eq!(through_bytes(word, keep, adds), u64::from_le_bytes(bytes));
    }
    for (keep, adds) in [(0xF_u8, 0x1_u8), (0xC, 0xC), (0x8, 0x8), (0xE, 0x6)] {
        let digits = (0..16).fold(0u64, |all, x| {
            let digit = ((word >> (4 * x)) & 0xF) as u8;
            all | u64::from(((digit & keep) + adds) & 0xF) << (4 * x)
        });
        assert_eq!(through_nibbles(word, keep, adds), digits);
    }
}

#[test]
fn a_slot_that_is_not_known_is_drawn_as_the_nearest_that_is_and_its_blocks_are_counted() {
    let made: Vec<Six> = (0..8).map(six).collect();
    let presets = made_up_presets(&[250, 254]);
    // Every block hangs on slot 253, three blocks before the body, which no fit gave a value.
    let lanes = six_lanes(&made, &presets, |b| if b == 0 { (3, 3) } else { (1, 1) });
    let rebuilt = blocks(&lanes, &presets).expect("decodes");
    assert_eq!(rebuilt.unknown, 8);
    // Slot 254 is the nearest that is known, and stands in.
    let mut stood = presets.clone();
    stood.ends[253] = presets.ends[254];
    stood.ends_known[253] = presets.ends_known[254];
    stood.first[253] = presets.first[254];
    stood.first_known[253] = true;
    let as_254 = blocks(&lanes, &stood).expect("decodes");
    assert_eq!(as_254.unknown, 0);
    assert!(rebuilt.bytes == as_254.bytes);
}

#[test]
fn a_level_with_no_plain_level_beside_it_is_fitted_from_the_coded_level_above_it() {
    let (width, height) = (4, 4);
    let low: Vec<Six> = (0..width * height).map(|b| six(b + 100)).collect();
    let mid = six_above(&low, width, height);
    let top = six_level_above(&mid, 2 * width, 2 * height);
    let made = made_up_presets(&[250, 251, 254, 255]);
    // The middle level hangs on slots 254 and 255, which the plain level above it fits; the
    // lowest hangs on slots 250 and 251, which only the middle level, as drawn, can fit.
    let backs = |b: usize| match b {
        0 => (0, 1),
        1 => (3, 0),
        _ => (2, 2),
    };
    let lower = |b: usize| match b {
        0 => (0, 5),
        1 => (7, 0),
        _ => (2, 2),
    };
    let levels = [
        (2 * width, 2 * height, six_lanes(&mid, &made, backs)),
        (width, height, six_lanes(&low, &made, lower)),
    ];
    let texture = || {
        let mut all = vec![LevelIn {
            width: 4 * width,
            height: 4 * height,
            data: LevelData::Plain(top.clone()),
        }];
        all.extend(levels.iter().map(|(width, height, lanes)| LevelIn {
            width: *width,
            height: *height,
            data: LevelData::Coded {
                lanes: lanes.clone(),
                at: (0..(width * height) as u32).collect(),
            },
        }));
        TextureIn {
            signed: false,
            kind: 7,
            levels: all,
        }
    };
    let (tables, summaries) =
        calibrate(1, &|_, _| vec![texture()], &mut |_, _| {}, &|| false).expect("not stopped");
    let found = tables.bptc();
    assert_eq!(found.ends[250][..8], made.ends[250][..8]);
    assert!(found.ends_known[250][..8].iter().all(|&known| known));
    assert_eq!(found.first[251], made.first[251]);
    assert!(found.first_known[251]);
    let summary = (summaries.iter())
        .find(|summary| summary.kind == 7)
        .expect("BC7's");
    assert_eq!((summary.bodies, summary.unknown), (2, 0));
}
