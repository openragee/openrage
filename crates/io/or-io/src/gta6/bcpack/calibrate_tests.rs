//! Bodies coded here from made-up blocks through made-up tables, which a calibration then has to
//! find again from a plain level made up to match.
// Outside a test build the made-up fixtures take a few of these helpers and the rest lie unused.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use super::super::bc1;
use super::super::planes::{blocks, built_in, mode, Half, ModeTables, FIELDS, UNKNOWN};
use super::super::tests::encode;
use super::super::{bc1_reads_unknown_slot, decode, decode_with, lane_widths, lanes};
use super::super::{BcPackError, Decode, Reading};
use super::*;
use crate::gta6::bcpack::bptc::{rank, Sig, RANKS, UNPLACED};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The preset slots the made-up bodies read: slot 4, which BC1 has no value for, and three more.
const SLOTS: [usize; 4] = [4, 100, 101, 102];

/// Tables no pack holds: each pair and quadrant symbol names a pattern of its own, and the slots
/// of [`SLOTS`] hold fields; BC1 keeps its built-in values and takes only slot 4.
fn made_up(kind: u8) -> ModeTables {
    let mut tables = ModeTables::empty();
    for (symbol, pair) in tables.pairs.iter_mut().enumerate() {
        *pair = ((symbol * 37 + 11) % 64) as u8;
    }
    for (symbol, quadrant) in tables.quadrants.iter_mut().enumerate() {
        *quadrant = ((symbol * 77 + 5) % 256) as u16;
    }
    let mode = mode(kind).expect("a mode of halves");
    for slot in SLOTS {
        for plane in mode.planes {
            for k in 0..plane.fields.len() {
                let value = (slot * 7 + 33 * (plane.base + k) + 9) % (1 << plane.bits(k));
                tables.preset[slot][plane.base + k] = value as u16;
            }
        }
    }
    tables.with_built_in(kind)
}

/// The symbol of the difference `delta`, already reduced to `bits` bits.
fn difference_symbol(delta: u32, bits: u32) -> u16 {
    if delta < 1 << (bits - 1) {
        (2 * delta) as u16
    } else {
        (2 * ((1 << bits) - delta) - 1) as u16
    }
}

/// The lane 0 symbol of a reference `back` blocks back.
fn distance_symbol(back: usize) -> u16 {
    if back < 128 {
        (2 * back) as u16
    } else {
        (2 * (255 - back) + 1) as u16
    }
}

/// The symbol whose entry in `table` is `pattern`.
fn symbol_of<T: PartialEq + Copy>(table: &[T], pattern: T) -> u16 {
    let symbol = table.iter().position(|&entry| entry == pattern);
    symbol.expect("the table holds every pattern") as u16
}

/// The lanes of `count` made-up blocks of codec kind `kind` coded through `tables`.
///
/// Blocks 0 to 3 read slot 4, blocks 4 to 39 slots 100 to 102, and the rest refer to nothing, to
/// the block before or to one 37 back. Endpoint 0 of each half is its brighter one, and each
/// block holds pixels of index 0 and of index 1.
pub(in crate::gta6::bcpack) fn coded_lanes(
    kind: u8,
    count: usize,
    tables: &ModeTables,
) -> Vec<Vec<u16>> {
    let mode = mode(kind).expect("a mode of halves");
    let (per, fields) = (mode.elements, mode.fields());
    let widths = lane_widths(kind).expect("a mode");
    let mut lanes = vec![vec![0u16; per * count]; widths.len()];
    let mut values = vec![0u32; count * fields];
    for i in 0..count {
        let back = match i {
            0..=3 => 252 + i,
            4..=39 => 156 + i - i % 3,
            _ => [0, 1, 37][i % 3],
        };
        let symbol = distance_symbol(back);
        if per == 1 {
            lanes[0][i] = symbol;
        } else {
            // The first of the two symbols is the even bits.
            for bit in 0..4 {
                lanes[0][2 * i] |= ((symbol >> (2 * bit)) & 1) << bit;
                lanes[0][2 * i + 1] |= ((symbol >> (2 * bit + 1)) & 1) << bit;
            }
        }
        for (p, plane) in mode.planes.iter().enumerate() {
            let wanted: Vec<usize> = match plane.half {
                Half::Alpha => vec![140 + (i * 5 + 17 * p) % 100, 20 + (i * 3 + 29 * p) % 100],
                Half::Colour => vec![
                    16 + (i * 3) % 16,
                    32 + (i * 5) % 32,
                    16 + (i * 7) % 16,
                    (i * 5) % 12,
                    (i * 3) % 24,
                    (i * 11) % 12,
                ],
            };
            for (k, &(lane, sub)) in plane.fields.iter().enumerate() {
                let (field, bits) = (plane.base + k, plane.bits(k));
                let predicted = if back == 0 {
                    u32::from(plane.default())
                } else if back <= i {
                    values[(i - back) * fields + field]
                } else {
                    u32::from(tables.preset[256 + i - back][field])
                };
                let value = wanted[k] as u32;
                values[i * fields + field] = value;
                let delta = (value + (1 << bits) - predicted) & ((1 << bits) - 1);
                lanes[lane][per * i + sub] = difference_symbol(delta, bits);
            }
            for (k, &(lane, sub)) in plane.indices.iter().enumerate() {
                lanes[lane][per * i + sub] = match (plane.half, k) {
                    // Indices 0 and 1, side by side.
                    (Half::Alpha, 0) => symbol_of(&tables.pairs, 1 << 3),
                    (Half::Alpha, _) => ((i * 7 + k + 13 * p) % 64) as u16,
                    // A quadrant of index 0, then one of index 1.
                    (Half::Colour, 0) => symbol_of(&tables.quadrants, 0),
                    (Half::Colour, 1) => symbol_of(&tables.quadrants, 0x55),
                    (Half::Colour, _) => ((i * 5 + k * 97) % 256) as u16,
                };
            }
        }
    }
    lanes
}

/// The level above a level of `width` by `height` blocks `level`, of codec kind `kind`: each
/// pixel repeated 2 by 2, which blocks of the same endpoints hold exactly.
pub(in crate::gta6) fn level_above(kind: u8, level: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mode = mode(kind).expect("a mode of halves");
    let size = mode.block_bytes();
    let mut out = vec![0u8; 4 * level.len()];
    for uy in 0..2 * height {
        for ux in 0..2 * width {
            let from = &level[((uy / 2) * width + ux / 2) * size..][..size];
            let to = &mut out[(uy * 2 * width + ux) * size..][..size];
            for plane in mode.planes {
                // The endpoints, then the index word: 3 bits a pixel after 2 bytes, or 2 after 4.
                let (head, bits) = match plane.half {
                    Half::Alpha => (2, 3),
                    Half::Colour => (4, 2),
                };
                let (from, to) = (&from[plane.at..][..8], &mut to[plane.at..][..8]);
                to[..head].copy_from_slice(&from[..head]);
                let word = (from[head..].iter().rev()).fold(0u64, |w, &b| w << 8 | u64::from(b));
                let mut above = 0u64;
                for pixel in 0..16 {
                    let (x, y) = (
                        2 * (ux % 2) + (pixel % 4) / 2,
                        2 * (uy % 2) + (pixel / 4) / 2,
                    );
                    let index = (word >> (bits * (4 * y + x))) & ((1 << bits) - 1);
                    above |= index << (bits * pixel);
                }
                to[head..].copy_from_slice(&above.to_le_bytes()[..8 - head]);
            }
        }
    }
    out
}

/// The stream of `count` made-up blocks of codec kind `kind` coded through [`made_up`] tables,
/// and the blocks.
pub(in crate::gta6) fn made_up_body(kind: u8, count: usize) -> (Vec<u8>, Vec<u8>) {
    let tables = made_up(kind);
    let made = coded_lanes(kind, count, &tables);
    let truth = blocks(kind, &made, &tables).expect("rebuilds");
    assert_eq!(truth.unknown, 0, "every slot and symbol is made up");
    (encode(kind, &made), truth.bytes)
}

/// A 16 by 16 block level of codec kind `kind` coded through [`made_up`] tables, as a
/// calibration reads it under the plain level above it; and its stream and its blocks.
fn chain(kind: u8) -> (TextureIn, Vec<u8>, Vec<u8>) {
    let (width, height) = (16, 16);
    let (stream, truth) = made_up_body(kind, width * height);
    let read = lanes(kind, &stream, truth.len() as u64).expect("the lanes decode");
    let levels = vec![
        LevelIn {
            width: 2 * width,
            height: 2 * height,
            data: LevelData::Plain(level_above(kind, &truth, width, height)),
        },
        LevelIn {
            width,
            height,
            data: LevelData::Coded {
                lanes: read,
                at: (0..(width * height) as u32).collect(),
            },
        },
    ];
    (
        TextureIn {
            signed: false,
            kind,
            levels,
        },
        stream,
        truth,
    )
}

/// `texture` again, as a calibration reads a texture in every pass.
fn again(texture: &TextureIn) -> TextureIn {
    let levels = (texture.levels.iter())
        .map(|level| LevelIn {
            width: level.width,
            height: level.height,
            data: match &level.data {
                LevelData::Plain(blocks) => LevelData::Plain(blocks.clone()),
                LevelData::Coded { lanes, at } => LevelData::Coded {
                    lanes: lanes.clone(),
                    at: at.clone(),
                },
                LevelData::Large => LevelData::Large,
                LevelData::Absent => LevelData::Absent,
            },
        })
        .collect();
    TextureIn {
        signed: false,
        kind: texture.kind,
        levels,
    }
}

/// The tables a calibration fits from `texture` alone.
fn fitted(texture: TextureIn) -> (Tables, Vec<Summary>) {
    let read = |_: usize, _: bool| vec![again(&texture)];
    calibrate(1, &read, &mut |_, _| {}, &|| false).expect("not stopped")
}

#[test]
fn a_calibration_finds_made_up_tables_again_and_the_body_then_decodes_as_calibrated() {
    for kind in [4, 5, 3] {
        let (texture, stream, truth) = chain(kind);
        let size = truth.len() as u64;
        assert_eq!(
            decode(kind, &stream, size),
            Err(BcPackError::NoBlockTables { kind }),
            "no table, no blocks"
        );
        assert!(decode_with(kind, &stream, size, Some(&Tables::empty())).is_err());

        let (tables, summaries) = fitted(texture);
        let (made, found) = (made_up(kind), tables.of(kind).expect("a fitted mode"));
        let mode = mode(kind).expect("a mode of halves");
        assert!(found.pairs == made.pairs, "BC{kind}: every pair symbol");
        if mode.planes.iter().any(|plane| plane.half == Half::Colour) {
            assert!(
                found.quadrants == made.quadrants,
                "BC{kind}: every quadrant symbol"
            );
        }
        for slot in SLOTS {
            assert_eq!(
                found.preset[slot][..mode.fields()],
                made.preset[slot][..mode.fields()],
                "BC{kind} slot {slot}"
            );
        }
        let summary = summaries
            .iter()
            .find(|s| s.kind == kind)
            .expect("its summary");
        assert_eq!(
            (summary.bodies, summary.blocks, summary.unknown),
            (1, 256, 0)
        );
        let mismatch = summary.mean_mismatch().expect("a plain level beside it");
        assert!(
            mismatch < 0.001,
            "BC{kind} meets the level above: {mismatch}"
        );
        assert_eq!(summary.slots as usize, SLOTS.len());

        let (decoded, how) = decode_with(kind, &stream, size, Some(&tables)).expect("decodes");
        assert!(decoded == truth, "BC{kind}: the blocks coded");
        assert_eq!(
            how,
            Decode {
                calibrated: 256,
                ..Decode::default()
            }
        );
        assert_eq!(how.reading(Standing::default()), Reading::Calibrated);
    }
}

#[test]
fn a_probe_finds_a_table_s_own_values_no_worse_and_one_moved_a_step_beaten() {
    for kind in [4, 3] {
        let (texture, _, _) = chain(kind);
        let at = Tables::at(kind).expect("a mode of halves");
        let mut tables = Tables::empty();
        tables.modes[at] = made_up(kind);
        let probed = |tables: &Tables| {
            let read = |_: usize, _: bool| vec![again(&texture)];
            let found =
                probe_tables(1, &read, tables, &mut |_, _| {}, &|| false).expect("not stopped");
            let of = |indices: bool| found.of(kind, indices).expect("its table").values.clone();
            (of(false), of(true))
        };
        // The level above holds the coded level's own pixels: no value moved a step does better.
        let (ends, indices) = probed(&tables);
        let beaten = |moved: &Moved| moved.among() == Some(Among::Beaten);
        assert!(!ends.iter().chain(&indices).any(beaten), "BC{kind}");
        // Slot 100's first field is the brighter end of an alpha half, which pixels of every
        // block that reads it take: a step either way costs them.
        let field = 100 * FIELDS;
        assert!(ends[field].blocks > 0);
        assert_eq!(ends[field].among(), Some(Among::Least), "BC{kind}");
        assert_eq!(ends[5 * FIELDS].among(), None, "a slot no block reads");
        // The pair of the two ends side by side, which every block carries.
        let symbol = usize::from(symbol_of(&tables.modes[at].pairs, 1 << 3));
        assert_eq!(indices[2 * symbol].among(), Some(Among::Least), "BC{kind}");

        // The field a step higher, and the pair's first pixel a place from the first end: the
        // value below each meets the level, and the probe says so of those two alone.
        let mut off = tables.clone();
        off.modes[at].preset[100][0] += 1;
        off.modes[at].pairs[symbol] = 2 | 1 << 3;
        let (ends, indices) = probed(&off);
        assert!(
            beaten(&ends[field]) && beaten(&indices[2 * symbol]),
            "BC{kind}"
        );
        assert!(ends[field].lower < ends[field].held && ends[field].held < ends[field].higher);
        assert!(indices[2 * symbol].lower < indices[2 * symbol].held);

        // Two plain levels, the upper one the lower one's pixels repeated: their own blocks'
        // indices are moved the same way under the symbol the table gives each pattern, and
        // none does better moved.
        let (_, _, truth) = chain(kind);
        let level = |width: usize, blocks: Vec<u8>| LevelIn {
            width,
            height: width,
            data: LevelData::Plain(blocks),
        };
        let plain = |_: usize, _: bool| {
            vec![TextureIn {
                signed: false,
                kind,
                levels: vec![
                    level(32, level_above(kind, &truth, 16, 16)),
                    level(16, truth.clone()),
                ],
            }]
        };
        let found =
            probe_tables(1, &plain, &tables, &mut |_, _| {}, &|| false).expect("not stopped");
        let stored = (found.tables.iter()).find(|table| table.stored && table.kind == kind);
        let stored = &stored.expect("a mode of halves").values;
        assert!(stored[2 * symbol].blocks > 0 && !stored.iter().any(beaten));
        assert!(found
            .of(kind, true)
            .is_some_and(|coded| coded.values.iter().all(|moved| moved.blocks == 0)));
    }
}

#[test]
fn bc1_s_slot_with_no_value_is_fitted_and_only_a_body_that_reads_it_is_calibrated() {
    let (texture, stream, truth) = chain(1);
    let size = truth.len() as u64;
    assert_eq!(
        decode(1, &stream, size),
        Err(BcPackError::PresetNotKnown { block: 0, slot: 4 })
    );
    assert!(bc1_reads_unknown_slot(&stream, size));
    assert!(bc1_reads_unknown_slot(
        &stream[..stream.len().min(4096)],
        size
    ));

    let (tables, _) = fitted(texture);
    let (made, found) = (made_up(1), tables.of(1).expect("a fitted mode"));
    assert_eq!(found.preset[4][..6], made.preset[4][..6]);
    let others = (0..256).filter(|&slot| slot != 4);
    assert!(
        others
            .clone()
            .all(|slot| found.preset[slot] == [UNKNOWN; FIELDS]),
        "a built-in slot is not fitted"
    );
    assert!(others.clone().all(|slot| slot == 0 || built_in(1, slot)));
    assert!(tables.covers(1) && !tables.covers(4) && !tables.covers(7));

    let (decoded, how) = decode_with(1, &stream, size, Some(&tables)).expect("decodes");
    assert!(decoded == truth);
    assert_eq!((how.calibrated, how.unknown), (256, 0));

    // A body that reads only built-in slots decodes through no fitted value, with the tables or
    // without: through the built-in tables alone, which nothing confirms value for value, so
    // every block is counted and the read is not called exact.
    let mut built = coded_lanes(1, 64, &made);
    built[0][..4].fill(0);
    let built = encode(1, &built);
    assert!(!bc1_reads_unknown_slot(&built, 8 * 64));
    let plain = decode(1, &built, 8 * 64).expect("decodes");
    let with = decode_with(1, &built, 8 * 64, Some(&tables)).expect("decodes");
    assert!(with.0 == plain && Ok(&with) == decode_with(1, &built, 8 * 64, None).as_ref());
    let alone = Decode {
        unseconded: 64,
        ..Decode::default()
    };
    assert_eq!(with.1, alone);
    assert_eq!(with.1.reading(Standing::default()), Reading::BuiltIn);
}

#[test]
fn a_bc1_block_that_reads_a_value_two_tables_give_differently_is_counted_and_not_called_exact() {
    // Three blocks: block 0 reads preset slot `slot`, block 1 is a difference from block 0, and
    // block 2 refers to nothing and carries `symbol` for its first quadrant.
    let body = |slot: usize, symbol: u16| {
        let mut lanes = vec![vec![0u16; 3]; 12];
        // A distance of d blocks is the symbol 2 d up to 127, and 2 (255 - d) + 1 past it.
        let back = |d: usize| (if d < 128 { 2 * d } else { 2 * (255 - d) + 1 }) as u16;
        lanes[0] = vec![back(256 - slot), back(1), 0];
        lanes[8][2] = symbol;
        encode(1, &lanes)
    };
    // Tables supplied that are the built-in ones but for a slot and a symbol given another
    // value, and a slot the built-in tables give none for: all three made up here.
    let mut held = (1..256).filter(|&slot| built_in(1, slot));
    let (same, moved) = (held.next().expect("a slot"), held.next().expect("another"));
    let lone = (1..256).find(|&slot| !built_in(1, slot));
    let (lone, odd) = (lone.expect("a slot with no built-in value"), 9);
    let at = Tables::at(1).expect("a mode of halves");
    let mut supplied = Tables::empty();
    supplied.modes[at] = ModeTables::empty().with_built_in(1);
    supplied.modes[at].preset[moved][0] ^= 1;
    supplied.modes[at].preset[lone][..6].fill(1);
    supplied.modes[at].quadrants[usize::from(odd)] ^= 1;
    supplied.supplied = 1 << 1;
    assert_eq!(supplied.bc1_apart_counts(), Some((2, 1)));
    let read = |slot: usize, symbol: u16| {
        let (_, how) = decode_with(1, &body(slot, symbol), 24, Some(&supplied)).expect("decodes");
        assert_eq!((how.supplied, how.disagree()), (3, 0));
        how
    };

    // Every value read is one both tables give: the read is exact on that ground, whatever a
    // check has held, unless one has failed the file.
    assert_eq!(read(same, 0).unseconded, 0);
    assert_eq!(read(same, 0).reading(Standing::default()), Reading::Exact);
    let against = Checked {
        blocks: 9,
        disagree: 1,
    };
    let failed = Standing::default().after(&[(7, against)]);
    assert!(matches!(
        read(same, 0).reading(failed),
        Reading::NotTheCoders { .. }
    ));
    // The block that reads the slot, and the block chained to it.
    assert_eq!(read(moved, 0).unseconded, 2);
    assert_eq!(
        read(moved, 0).reading(Standing::default()),
        Reading::Unseconded { blocks: 2 }
    );
    assert_eq!(read(lone, 0).unseconded, 2);
    assert_eq!(read(same, odd).unseconded, 1);
    assert_eq!(read(moved, odd).unseconded, 3);

    // With no tables supplied every block rests on the built-in ones alone.
    let (_, how) = decode_with(1, &body(same, 0), 24, None).expect("decodes");
    assert_eq!(how.unseconded, 3);
    assert_eq!(how.reading(Standing::default()), Reading::BuiltIn);
    assert_eq!(Tables::empty().bc1_apart_counts(), None);

    // The same tables with the preset moved round by one slot: the block that read a value
    // both tables gave now reads one only the tables supplied give, and so does the next.
    let mut round = supplied.clone();
    round.modes[at].preset.rotate_left(1);
    let (_, how) = decode_with(1, &body(same, 0), 24, Some(&round)).expect("decodes");
    let two_of_them = Reading::Unseconded { blocks: 2 };
    assert_eq!(how.reading(Standing::default()), two_of_them);
}

#[test]
fn the_halves_rebuild_bc1_as_its_own_decoder_does() {
    let made = made_up(1);
    let mut all = coded_lanes(1, 300, &made);
    all[0][..4].fill(0);
    let own = bc1::blocks(&all).expect("BC1's own decoder");
    let halves = blocks(1, &all, &ModeTables::empty().with_built_in(1)).expect("the halves");
    assert!(halves.bytes == own && halves.unknown == 0);
}

#[test]
fn an_alpha_half_is_two_endpoints_and_sixteen_three_bit_indices() {
    // Pair symbol s names the left index s % 8 and the right index s / 8.
    let mut tables = ModeTables::empty();
    for (symbol, pair) in tables.pairs.iter_mut().enumerate() {
        *pair = symbol as u8;
    }
    // One block from nothing: endpoint 0 is 128 + 72, endpoint 1 is 128 - 28.
    let mut all = vec![vec![0u16]; 12];
    all[2][0] = difference_symbol(72, 8);
    all[3][0] = difference_symbol(256 - 28, 8);
    let pairs: [u16; 8] = [0o10, 0o32, 0o54, 0o76, 0o01, 0o23, 0o45, 0o67];
    for (k, pair) in pairs.into_iter().enumerate() {
        all[4 + k][0] = pair;
    }
    let rebuilt = blocks(4, &all, &tables).expect("rebuilds");
    let indices: [u64; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 1, 0, 3, 2, 5, 4, 7, 6];
    let word = (indices.iter().enumerate())
        .fold(0u64, |word, (pixel, index)| word | *index << (3 * pixel));
    let mut expected = vec![200, 100];
    expected.extend(&word.to_le_bytes()[..6]);
    assert_eq!(rebuilt.bytes, expected);
    assert_eq!(rebuilt.unknown, 0);
}

#[test]
fn a_block_through_a_slot_or_a_symbol_that_is_not_known_is_drawn_and_counted() {
    let (texture, _, _) = chain(4);
    let (tables, _) = fitted(texture);
    // Block 0 reads slot 50, which no made-up body read; block 1 reads nothing.
    let mut all = vec![vec![0u16; 2]; 12];
    all[0][0] = distance_symbol(206);
    let stream = encode(4, &all);
    let (decoded, how) = decode_with(4, &stream, 16, Some(&tables)).expect("decodes");
    assert_eq!(decoded.len(), 16);
    assert_eq!(
        how,
        Decode {
            calibrated: 2,
            unknown: 1,
            ..Decode::default()
        }
    );
    assert_eq!(how.unknown_percent(), 50.0);
    assert_eq!(decoded[8..10], [128, 128], "block 1 from nothing");
}

#[test]
fn a_census_names_each_rank_s_mode_from_the_widths_of_its_symbols() {
    // A level whose blocks take every rank of the made-up table in turn, each lane's symbols as wide as the rank's mode lets them be.
    let mut modes = [UNPLACED; RANKS];
    let mut filled = 0;
    for (mode, count) in [16, 64, 64, 64, 8, 4, 1, 64].into_iter().enumerate() {
        modes[filled..filled + count].fill(mode as u8);
        filled += count;
    }
    let ranks: Vec<usize> = (0..RANKS).filter(|&rank| modes[rank] != UNPLACED).collect();
    let blocks = 40 * ranks.len();
    let mut lanes = vec![vec![0u16; 2 * blocks]; 16];
    for block in 0..blocks {
        let at = ranks[block % ranks.len()];
        let pair = (0..32)
            .flat_map(|first| (0..16).map(move |second| [first, second]))
            .find(|&[first, second]| rank(first, second) == Some(at))
            .expect("a pair has every rank");
        let sig = Sig::of(pair[0], pair[1], &modes).expect("the table names it");
        lanes[15][2 * block..2 * block + 2].copy_from_slice(&pair);
        for (symbol, bits) in sig.widths().into_iter().enumerate() {
            // A block in eight carries the widest symbol; the others less.
            let widest = (1u16 << bits) - 1;
            lanes[2 + symbol / 2][2 * block + symbol % 2] = match (block / ranks.len()) % 8 {
                0 => widest,
                other => widest >> other.min(usize::from(bits)),
            };
        }
    }
    let texture = TextureIn {
        signed: false,
        kind: 7,
        levels: vec![LevelIn {
            width: blocks,
            height: 1,
            data: LevelData::Coded {
                lanes,
                at: (0..blocks as u32).collect(),
            },
        }],
    };
    let mut census = bptc_fit::Census::empty();
    census.take(&texture);
    assert!(census.modes() == modes);

    // Read twice by two workers, the counts add and the table is the same.
    let mut other = bptc_fit::Census::empty();
    other.take(&texture);
    census.add(&other);
    assert!(census.modes() == modes);
}

#[test]
fn a_fit_s_tallies_do_not_grow_with_the_bodies_it_reads_and_many_workers_fit_what_one_does() {
    let (texture, _, _) = chain(5);
    let mode = mode(5).expect("a mode of halves");
    let tables = ModeTables::empty().with_built_in(5);
    let bytes = |votes: &Votes| {
        let held = [&votes.aware, &votes.boot, &votes.pairs, &votes.quadrants];
        held.map(|tally| tally.capacity()).iter().sum::<usize>()
    };
    let mut one = Votes::new();
    vote(5, mode, &prepare(&texture, mode), &tables, &mut one, false);
    let mut many = Votes::new();
    for _ in 0..50 {
        vote(5, mode, &prepare(&texture, mode), &tables, &mut many, false);
    }
    assert_eq!(bytes(&one), bytes(&many));
    assert!(many.pairs.iter().sum::<u32>() == 50 * one.pairs.iter().sum::<u32>());

    // Forty copies read by as many workers as the machine gives, at most one texture each at a
    // time: the tables are the same from one run to the next, and no more than the workers' textures are held.
    let (held, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let read = |_: usize, _: bool| {
        let now = held.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(now, Ordering::SeqCst);
        // A worker asks for its next texture only after it is done with the last.
        std::thread::sleep(std::time::Duration::from_millis(1));
        held.fetch_sub(1, Ordering::SeqCst);
        vec![again(&texture)]
    };
    let (of_many, _) = calibrate(40, &read, &mut |_, _| {}, &|| false).expect("not stopped");
    let (again_of_many, _) = calibrate(40, &read, &mut |_, _| {}, &|| false).expect("not stopped");
    assert!(of_many == again_of_many);
    assert!(of_many.covers(5));
    assert!(most.load(Ordering::SeqCst) <= MAX_WORKERS);
}

#[test]
fn a_calibration_stops_when_asked_and_tells_how_far_it_is() {
    let (texture, _, _) = chain(4);
    let reads = AtomicUsize::new(0);
    let read = |_: usize, _: bool| {
        reads.fetch_add(1, Ordering::Relaxed);
        vec![again(&texture)]
    };
    assert!(calibrate(1, &read, &mut |_, _| {}, &|| true).is_none());
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "nothing is read once stopped"
    );

    let mut told = Vec::new();
    let mut progress = |done: u64, total: u64| told.push((done, total));
    calibrate(1, &read, &mut progress, &|| false).expect("not stopped");
    assert_eq!(
        reads.load(Ordering::Relaxed),
        7,
        "a texture is read once in each pass that does something with its mode"
    );
    assert_eq!(told.first(), Some(&(0, 14)));
    assert!(told
        .windows(2)
        .all(|pair| pair[0].0 < pair[1].0 && pair[1].1 == 14));
}

#[test]
fn a_calibration_file_is_found_again_by_a_pack_it_lists() {
    let (texture, _, _) = chain(4);
    let (tables, _) = fitted(texture);
    assert_eq!(Tables::from_bytes(&tables.to_bytes()), Some(tables.clone()));
    assert_eq!(Tables::from_bytes(&tables.to_bytes()[1..]), None);
    assert_eq!(Tables::from_bytes(b"ORBP"), None);

    let dir = tempfile::tempdir().expect("a scratch folder");
    let cache = dir.path().join("bcpack");
    let ours = member_key(std::path::Path::new("zq/Made_Up.ttpf"), 4096);
    assert_eq!(
        ours,
        member_key(std::path::Path::new("elsewhere/made_up.ttpf"), 4096)
    );
    let other = member_key(std::path::Path::new("zq/made_up.ttpf"), 8192);
    assert_ne!(ours, other);
    assert!(calibration(&cache, ours).is_none(), "no folder yet");
    let path = save_cache(&cache, &tables, &[ours]).expect("written");
    assert!(path.starts_with(&cache));
    assert_eq!(calibration(&cache, ours), Some(tables));
    assert!(
        calibration(&cache, other).is_none(),
        "a pack it did not read"
    );
}

#[test]
fn the_refusal_of_a_mode_a_calibration_fits_names_the_command() {
    for kind in [3, 4, 5, 6, 7] {
        let why = BcPackError::NoBlockTables { kind }.to_string();
        assert!(
            why.contains("ttpf calibrate") && why.contains("ttpf tables"),
            "{why}"
        );
    }
    let why = BcPackError::PresetNotKnown { block: 0, slot: 4 }.to_string();
    assert!(
        why.contains("ttpf calibrate") && why.contains("ttpf tables"),
        "{why}"
    );
}

#[test]
fn a_symbol_named_already_keeps_its_pattern_and_the_others_take_what_is_left() {
    // Three symbols by three patterns: symbol 1 votes hardest for pattern 0, which symbol 0 holds.
    let votes = [0, 0, 0, 9, 5, 4, 0, 7, 4];
    let anew = assign(&votes, 3, vec![None; 3]);
    assert_eq!(anew, [None, Some(0), Some(1)]);
    let kept = assign(&votes, 3, vec![Some(0), None, None]);
    assert_eq!(kept, [Some(0), Some(2), Some(1)]);
}
