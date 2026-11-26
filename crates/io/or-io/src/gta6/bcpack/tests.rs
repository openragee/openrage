// bcpack-rs by Ahmet Dedeler (https://github.com/Ahmet-Dedeler, https://ahmetdedeler.com)
// Source: https://github.com/Ahmet-Dedeler/bcpack-rs
// SPDX-License-Identifier: MIT OR Apache-2.0
// If you copy, vendor or port this file, keep this header and credit bcpack-rs in your README.
//
// Origin: bcpack-rs, `src/encode.rs` (the fragment coder, the interleave and the sampler), commit
// 5bf5cc56ed56f1ababf1f9592300bd182ff90bd8, taken under its MIT licence. The tests are OpenRage's.
//! Streams coded here from made-up symbols, decoded back.
//!
//! The console's coder is not public. A lane is coded as flushed fragments, each a rANS stream
//! coded backwards from the state a fragment ends in, and the lanes' bytes are interleaved by
//! running the decoder over one queue for each lane and noting which lane takes each byte.
// Outside a test build the made-up fixtures take a few of these helpers and the rest lie unused.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use super::bc1::{difference, distance, index_word};
use super::bc1_tables::{PRESET_ENDPOINTS, QUADRANT_INDICES};
use super::rans::{decode_lanes, Model, L};
use super::*;

/// Symbols of one lane coded with model `q`, their width narrowed by `z` bits.
struct Fragment {
    symbols: Vec<u16>,
    q: u8,
    z: u8,
}

/// One flushed fragment: its header, then its bytes.
fn fragment_bytes(alphabet_bits: u8, fragment: &Fragment) -> Vec<u8> {
    let eab = alphabet_bits - fragment.z;
    let model = Model::of(eab.into(), fragment.q.into());
    let last = fragment.symbols.len() - 1;
    let mut x = u64::from(L);
    let mut reversed = Vec::new();
    let mut before_last = 0;
    for (k, &symbol) in fragment.symbols.iter().enumerate().rev() {
        assert_eq!(
            u32::from(symbol) >> (eab + 1),
            0,
            "symbol {symbol} is too wide"
        );
        let (frequency, start) = model.forward(symbol);
        assert!(frequency > 0, "model {} never codes {symbol}", fragment.q);
        while x >= u64::from(frequency) << 16 {
            reversed.push(x as u8);
            x >>= 8;
        }
        if k == last {
            before_last = reversed.len();
        }
        x = ((x / u64::from(frequency)) << 16) + x % u64::from(frequency) + u64::from(start);
    }
    // The bytes pushed while coding the last symbol are zeros a decoder need not read: up to two go.
    let mut trimmed = 0;
    while trimmed < 2 && trimmed < before_last && reversed[trimmed] == 0 {
        trimmed += 1;
    }
    let mut payload = (x as u32).to_be_bytes().to_vec();
    payload.extend(reversed[trimmed..].iter().rev());

    let len = payload.len();
    assert!(len <= 65535 + 257, "a fragment of {len} bytes");
    let long = len > 256;
    let first = fragment.z << 6 | u8::from(long) << 5 | 1 << 4 | fragment.q;
    let mut out = if long {
        vec![first, (len - 257) as u8, ((len - 257) >> 8) as u8]
    } else {
        vec![first, (len - 1) as u8]
    };
    out.extend(payload);
    out
}

/// The one stream a decoder of `target` symbols a lane reads `lanes`' bytes from.
fn interleave(alphabet_bits: &[u8], target: usize, lanes: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = lanes.iter().map(Vec::len).sum();
    let mut cursors = vec![0usize; lanes.len()];
    let mut stream = Vec::with_capacity(total);
    let next = |lane: usize| {
        let byte = lanes[lane].get(cursors[lane]).copied();
        if let Some(byte) = byte {
            cursors[lane] += 1;
            stream.push(byte);
        }
        byte
    };
    decode_lanes(next, alphabet_bits, target, total).expect("each lane decodes from its own bytes");
    assert_eq!(stream.len(), total, "a lane left bytes unread");
    stream
}

fn alphabet_bits(kind: u8) -> Vec<u8> {
    let widths = lane_widths(kind).expect("a BCPack mode");
    widths.iter().map(|width| width - 1).collect()
}

fn encode_fragments(kind: u8, lanes: &[Vec<Fragment>]) -> Vec<u8> {
    let bits = alphabet_bits(kind);
    assert_eq!(lanes.len(), bits.len());
    let target: usize = lanes[0].iter().map(|f| f.symbols.len()).sum();
    let bytes: Vec<Vec<u8>> = lanes
        .iter()
        .zip(&bits)
        .map(|(fragments, &bits)| {
            let symbols: usize = fragments.iter().map(|f| f.symbols.len()).sum();
            assert_eq!(symbols, target, "every lane holds as many symbols");
            fragments
                .iter()
                .flat_map(|fragment| fragment_bytes(bits, fragment))
                .collect()
        })
        .collect();
    interleave(&bits, target, &bytes)
}

/// A body of codec kind `kind` whose lanes carry `lanes`, each lane in fragments of the uniform model.
pub(in crate::gta6) fn encode(kind: u8, lanes: &[Vec<u16>]) -> Vec<u8> {
    let fragments: Vec<Vec<Fragment>> = lanes
        .iter()
        .map(|lane| {
            lane.chunks(1000)
                .map(|symbols| Fragment {
                    symbols: symbols.to_vec(),
                    q: 0,
                    z: 0,
                })
                .collect()
        })
        .collect();
    encode_fragments(kind, &fragments)
}

/// The symbol of the difference that takes 0 to `value` in a field of `bits` bits.
fn difference_symbol(value: u32, bits: u32) -> u16 {
    if value < 1 << (bits - 1) {
        (2 * value) as u16
    } else {
        (2 * ((1 << bits) - value) - 1) as u16
    }
}

/// The 12 lanes that rebuild BC1 `blocks`, each block predicted from nothing.
pub(in crate::gta6) fn bc1_lanes(blocks: &[u8]) -> Vec<Vec<u16>> {
    let mut lanes = vec![Vec::new(); 12];
    for block in blocks.as_chunks::<8>().0 {
        let colours = [
            u32::from(u16::from_le_bytes([block[0], block[1]])),
            u32::from(u16::from_le_bytes([block[2], block[3]])),
        ];
        let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
        lanes[0].push(0);
        lanes[1].push(0);
        for (k, colour) in colours.into_iter().enumerate() {
            lanes[2 + 3 * k].push(difference_symbol(colour >> 11, 5));
            lanes[3 + 3 * k].push(difference_symbol((colour >> 5) & 63, 6));
            lanes[4 + 3 * k].push(difference_symbol(colour & 31, 5));
        }
        for (quadrant, (left, top)) in [(0, 0), (2, 0), (0, 2), (2, 2)].into_iter().enumerate() {
            let at = |x: u32, y: u32| ((indices >> (2 * (4 * (top + y) + left + x))) & 3) as u8;
            let wanted = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
            let symbol = QUADRANT_INDICES
                .iter()
                .position(|&entry| entry == wanted)
                .expect("the table holds every quadrant");
            lanes[8 + quadrant].push(symbol as u16);
        }
    }
    lanes
}

/// A xorshift generator, so that a test draws the same values every run.
struct Rng(u64);

impl Rng {
    fn draw(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.draw() % n
    }
}

/// Lanes of `target` symbols for kind `kind`, in fragments of at most `longest` symbols, each
/// fragment under a model and a narrowing of its own and its symbols drawn from that model.
fn drawn(kind: u8, target: usize, longest: u64, seed: u64) -> (Vec<Vec<Fragment>>, Vec<Vec<u16>>) {
    let mut rng = Rng(seed | 1);
    let mut all = Vec::new();
    let fragments = alphabet_bits(kind)
        .into_iter()
        .map(|bits| {
            let (mut lane, mut symbols) = (Vec::new(), Vec::new());
            while symbols.len() < target {
                let n = (target - symbols.len()).min(1 + rng.below(longest) as usize);
                let z = if rng.below(4) == 0 {
                    rng.below(u64::from(bits) + 1).min(3) as u8
                } else {
                    0
                };
                let (eab, q) = (bits - z, rng.below(16) as u8);
                let model = Model::of(eab.into(), q.into());
                let drawn: Vec<u16> =
                    std::iter::repeat_with(|| model.inverse(1 + rng.below(0xFFFF) as u16))
                        .filter(|&s| u32::from(s) >> (eab + 1) == 0 && model.forward(s).0 > 0)
                        .take(n)
                        .collect();
                symbols.extend(&drawn);
                lane.push(Fragment {
                    symbols: drawn,
                    q,
                    z,
                });
            }
            all.push(symbols);
            lane
        })
        .collect();
    (fragments, all)
}

#[test]
fn every_model_names_the_symbol_whose_interval_holds_a_value() {
    for eab in 0..9 {
        for q in 0..16 {
            let model = Model::of(eab, q);
            for cf in 1..=0xFFFFu16 {
                let symbol = model.inverse(cf);
                let (frequency, start) = model.forward(symbol);
                assert!(
                    start <= u32::from(cf) && u32::from(cf) < start + frequency,
                    "alphabet {eab}, model {q}, value {cf} gives symbol {symbol}"
                );
            }
        }
    }
}

#[test]
fn lanes_in_fragments_of_many_models_decode_to_their_symbols() {
    // 12 lanes and 16, with fragments short enough that every lane holds several.
    for (kind, target, longest, seed) in [(1, 700, 90, 5), (7, 300, 40, 9), (4, 64, 500, 2)] {
        let (fragments, symbols) = drawn(kind, target, longest, seed);
        let stream = encode_fragments(kind, &fragments);
        let decoded = lanes(kind, &stream, 8 * target as u64).expect("decodes");
        assert!(decoded == symbols, "kind {kind}");
    }
}

#[test]
fn a_fragment_over_256_bytes_states_its_length_in_two_bytes() {
    let lane: Vec<u16> = (0..400u16).map(|i| (i * 7 + 3) % 256).collect();
    let mut all = vec![vec![0u16; 400]; 12];
    all[8] = lane;
    let fragments: Vec<Vec<Fragment>> = all
        .iter()
        .map(|symbols| {
            vec![Fragment {
                symbols: symbols.clone(),
                q: 0,
                z: 0,
            }]
        })
        .collect();
    let lone = fragment_bytes(7, &fragments[8][0]);
    assert!(
        lone[0] & 0x20 != 0,
        "400 one-byte symbols take the long form"
    );
    let stream = encode_fragments(1, &fragments);
    assert!(lanes(1, &stream, 8 * 400).expect("decodes") == all);
}

#[test]
fn the_symbol_maps_are_the_zigzags_and_the_first_four_quadrants_are_flat() {
    assert_eq!([0, 2, 4, 254].map(distance), [0, 1, 2, 127]);
    assert_eq!([1, 3, 255].map(distance), [255, 254, 128]);
    assert_eq!([0, 1, 2, 3, 4].map(difference), [0, -1, 1, -2, 2]);
    for symbol in 0..4u8 {
        assert_eq!(QUADRANT_INDICES[usize::from(symbol)], [symbol ^ 2; 4]);
    }
    let mut seen = [false; 256];
    for entry in QUADRANT_INDICES {
        let at = entry
            .iter()
            .fold(0, |at, &index| at << 2 | usize::from(index));
        assert!(
            entry.iter().all(|&index| index < 4) && !seen[at],
            "{entry:?}"
        );
        seen[at] = true;
    }
    // Symbols 2, 3, 0 and 1 are flat quadrants of indices 0, 1, 2 and 3, at the top left, the
    // top right, the bottom left and the bottom right.
    let mut word = 0u32;
    for (y, x) in (0..4u32).flat_map(|y| (0..4u32).map(move |x| (y, x))) {
        let index = 2 * (y / 2) + x / 2;
        word |= index << (2 * (4 * y + x));
    }
    assert_eq!(index_word([2, 3, 0, 1]), word);
}

#[test]
fn bc1_blocks_follow_their_references_and_their_quadrants() {
    // One row for each block: the lane 0 symbol, the six difference symbols, the quadrant symbols.
    let rows: [(u16, [u16; 6], [u16; 4]); 5] = [
        // No reference: R0 3, G0 -2, B0 0, R1 1, G1 0, B1 -1, from zero.
        (0, [6, 3, 0, 2, 0, 1], [2, 2, 2, 2]),
        // One block back, unchanged.
        (2, [0; 6], [3, 3, 3, 3]),
        // Two blocks back, R0 one more.
        (4, [2, 0, 0, 0, 0, 0], [2, 3, 0, 1]),
        // No reference and no difference.
        (0, [0; 6], [2, 2, 2, 2]),
        // 255 back from block 4: slot 5 of the preset, G1 one less.
        (1, [0, 0, 0, 0, 1, 0], [2, 2, 2, 2]),
    ];
    let mut all = vec![Vec::new(); 12];
    for (reference, differences, quadrants) in rows {
        all[0].push(reference);
        all[1].push(0);
        for (k, symbol) in differences.into_iter().enumerate() {
            all[2 + k].push(symbol);
        }
        for (k, symbol) in quadrants.into_iter().enumerate() {
            all[8 + k].push(symbol);
        }
    }
    let preset = PRESET_ENDPOINTS[5].expect("slot 5 is known");
    let colour = |r: u8, g: u8, b: u8| u16::from(r) << 11 | u16::from(g) << 5 | u16::from(b);
    let first = (colour(3, 62, 0), colour(1, 0, 31));
    let expected: [(u16, u16, u32); 5] = [
        (first.0, first.1, 0),
        (first.0, first.1, 0x5555_5555),
        (colour(4, 62, 0), first.1, index_word([2, 3, 0, 1])),
        (0, 0, 0),
        (
            colour(preset[0], preset[1], preset[2]),
            colour(preset[3], (preset[4] + 63) & 63, preset[5]),
            0,
        ),
    ];
    let expected: Vec<u8> = expected
        .into_iter()
        .flat_map(|(c0, c1, indices)| {
            let mut block = c0.to_le_bytes().to_vec();
            block.extend(c1.to_le_bytes());
            block.extend(indices.to_le_bytes());
            block
        })
        .collect();
    let stream = encode(1, &all);
    assert_eq!(decode(1, &stream, 40), Ok(expected));
}

#[test]
fn made_up_bc1_blocks_come_back_from_the_lanes_that_rebuild_them() {
    let mut rng = Rng(0x00C0_FFEE);
    let blocks: Vec<u8> = (0..8 * 2500).map(|_| rng.draw() as u8).collect();
    let stream = encode(1, &bc1_lanes(&blocks));
    assert!(decode(1, &stream, blocks.len() as u64) == Ok(blocks));
}

#[test]
fn a_preset_slot_with_no_value_and_a_lane_1_that_is_not_zero_are_refused() {
    assert!(PRESET_ENDPOINTS[4].is_none() && PRESET_ENDPOINTS[1].is_some());
    let mut all = vec![vec![0u16]; 12];
    // 252 back from block 0 is slot 4; 255 back is slot 1.
    all[0][0] = 7;
    assert_eq!(distance(7), 252);
    let refused = decode(1, &encode(1, &all), 8);
    assert_eq!(
        refused,
        Err(BcPackError::PresetNotKnown { block: 0, slot: 4 })
    );
    all[0][0] = 1;
    assert!(decode(1, &encode(1, &all), 8).is_ok());
    all[1][0] = 9;
    assert_eq!(
        decode(1, &encode(1, &all), 8),
        Err(BcPackError::Lane1 {
            block: 0,
            symbol: 9
        })
    );
}

#[test]
fn no_reference_reaches_preset_slot_0() {
    for symbol in 0..256u16 {
        let back = distance(symbol);
        assert!(back <= 255, "symbol {symbol}");
        for block in 0..back {
            assert!((1..256).contains(&(256 + block - back)));
        }
    }
}

#[test]
fn a_stream_cut_short_or_followed_by_other_than_zeros_is_refused() {
    let blocks: Vec<u8> = (0..8 * 300u32).map(|i| (i * 31 + 7) as u8).collect();
    let stream = encode(1, &bc1_lanes(&blocks));
    let size = blocks.len() as u64;

    let mut padded = stream.clone();
    padded.extend([0; 200]);
    assert!(decode(1, &padded, size) == Ok(blocks), "zeros may follow");
    padded[stream.len() + 3] = 1;
    assert_eq!(
        decode(1, &padded, size),
        Err(BcPackError::Trailing {
            consumed: stream.len(),
            rest: 200
        })
    );
    assert!(matches!(
        decode(1, &stream[..stream.len() / 2], size),
        Err(BcPackError::Truncated { .. })
    ));
    assert!(matches!(
        decode(1, &[], size),
        Err(BcPackError::Truncated { consumed: 0 })
    ));
    for size in [0, 12, size + 4] {
        assert_eq!(decode(1, &stream, size), Err(BcPackError::Size { size }));
    }
}

#[test]
fn the_other_modes_decode_to_lanes_and_are_refused_as_blocks() {
    for kind in [3, 4, 5, 6, 7] {
        let widths = lane_widths(kind).expect("a mode");
        let symbols: Vec<Vec<u16>> = widths
            .iter()
            .enumerate()
            .map(|(lane, &width)| {
                (0..48u16)
                    .map(|i| (i * 5 + lane as u16) % (1 << width))
                    .collect()
            })
            .collect();
        let stream = encode(kind, &symbols);
        assert!(lanes(kind, &stream, 8 * 48) == Ok(symbols), "kind {kind}");
        assert_eq!(
            decode(kind, &stream, 8 * 48),
            Err(BcPackError::NoBlockTables { kind })
        );
        assert!(!decodes(kind));
        let why = BcPackError::NoBlockTables { kind }.to_string();
        assert!(why.contains(&format!("BC{kind}")) && why.contains("preset"));
    }
    assert!(decodes(1));
    for kind in [0, 2, 8, 9] {
        assert_eq!(
            decode(kind, &[0; 64], 64),
            Err(BcPackError::NotAMode { kind })
        );
    }
}
