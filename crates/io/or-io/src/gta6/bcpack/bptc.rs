//! BC7's blocks from the 16 lanes of a BCPack body.
//!
//! Lane 15 names a block's mode, partition, rotation and index selection; lanes 0 and 1 give two
//! distances back, to the block its indices and to the block its endpoints are differences from.
//! A distance past a body's first block reads one of two 256-slot presets that are constants of
//! the coder and in no stream, and which mode each rank of lane 15 names is in no stream either, so
//! [`Presets`] is fitted on the user's machine.

use std::sync::OnceLock;

use super::bc1::difference;
use super::planes::{interleave, Rebuilt};
use super::BcPackError;

/// The values of a block's endpoints: 3 subsets by 4 channels by 2 ends, each left-aligned in 8 bits.
pub(super) const VALUES: usize = 24;
/// A position of the partition order that no fit has placed.
pub(super) const UNPLACED: u8 = u8::MAX;
/// The bytes [`Presets::to_bytes`] writes.
pub(super) const PRESETS_BYTES: usize = 256 * (2 * VALUES + 2 * 16 + 2) + 16 + 64 + RANKS;
/// The pairs of lane 15's symbols, a 5-bit and a 4-bit one.
pub(super) const RANKS: usize = 512;
/// The blocks a reference reaches back over: a walk keeps this many.
const REACH: usize = 256;
/// The preset values a trace is kept for, eight to a word, and the bytes of a word.
const TRACE_WORDS: usize = 32;
const BYTES: u64 = 0x0101_0101_0101_0101;
const NIBBLES: u64 = 0x1111_1111_1111_1111;
/// A lane's two symbols of a block, for lanes 2 to 14.
pub(super) const WIDTHS: usize = 26;

/// The lane of each channel of each subset's two ends: subset 2's green and blue take the lanes of the others' alpha.
const END_LANES: [[usize; 4]; 3] = [[2, 3, 4, 5], [6, 7, 8, 9], [10, 5, 9, 9]];
/// The lanes of the four rows of the first index set, and of the second.
const FIRST_LANES: [usize; 4] = [11, 12, 13, 14];
const SECOND_LANES: [usize; 4] = [6, 7, 8, 10];

/// The subset of each pixel, a bit for each from pixel 0, for each partition of two subsets.
const TWO_SUBSETS: [u16; 64] = [
    0xcccc, 0x8888, 0xeeee, 0xecc8, 0xc880, 0xfeec, 0xfec8, 0xec80, 0xc800, 0xffec, 0xfe80, 0xe800,
    0xffe8, 0xff00, 0xfff0, 0xf000, 0xf710, 0x008e, 0x7100, 0x08ce, 0x008c, 0x7310, 0x3100, 0x8cce,
    0x088c, 0x3110, 0x6666, 0x366c, 0x17e8, 0x0ff0, 0x718e, 0x399c, 0xaaaa, 0xf0f0, 0x5a5a, 0x33cc,
    0x3c3c, 0x55aa, 0x9696, 0xa55a, 0x73ce, 0x13c8, 0x324c, 0x3bdc, 0x6996, 0xc33c, 0x9966, 0x0660,
    0x0272, 0x04e4, 0x4e40, 0x2720, 0xc936, 0x936c, 0x39c6, 0x639c, 0x9336, 0x9cc6, 0x817e, 0xe718,
    0xccf0, 0x0fcc, 0x7744, 0xee22,
];

/// The subset of each pixel, two bits for each from pixel 0, for each partition of three subsets.
const THREE_SUBSETS: [u32; 64] = [
    0xaa685050, 0x6a5a5040, 0x5a5a4200, 0x5450a0a8, 0xa5a50000, 0xa0a05050, 0x5555a0a0, 0x5a5a5050,
    0xaa550000, 0xaa555500, 0xaaaa5500, 0x90909090, 0x94949494, 0xa4a4a4a4, 0xa9a59450, 0x2a0a4250,
    0xa5945040, 0x0a425054, 0xa5a5a500, 0x55a0a0a0, 0xa8a85454, 0x6a6a4040, 0xa4a45000, 0x1a1a0500,
    0x0050a4a4, 0xaaa59090, 0x14696914, 0x69691400, 0xa08585a0, 0xaa821414, 0x50a4a450, 0x6a5a0200,
    0xa9a58000, 0x5090a0a8, 0xa8a09050, 0x24242424, 0x00aa5500, 0x24924924, 0x24499224, 0x50a50a50,
    0x500aa550, 0xaaaa4444, 0x66660000, 0xa5a0a5a0, 0x50a050a0, 0x69286928, 0x44aaaa44, 0x66666600,
    0xaa444444, 0x54a854a8, 0x95809580, 0x96969600, 0xa85454a8, 0x80959580, 0xaa141414, 0x96960000,
    0xaaaa1414, 0xa05050a0, 0xa0a5a5a0, 0x96000000, 0x40804080, 0xa9a8a9a8, 0xaaaaaa44, 0x2a4a5254,
];

/// The second subset's anchor pixel for each partition of two subsets.
const SECOND_ANCHOR: [u8; 64] = [
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 2, 8, 2, 2, 8, 8, 15, 2, 8,
    2, 2, 8, 8, 2, 2, 15, 15, 6, 8, 2, 8, 15, 15, 2, 8, 2, 2, 2, 15, 15, 6, 6, 2, 6, 8, 15, 15, 2,
    2, 15, 15, 15, 15, 15, 2, 2, 15,
];

/// The second and third subsets' anchor pixels for each partition of three subsets.
const THIRD_ANCHORS: [(u8, u8); 64] = [
    (3, 15),
    (3, 8),
    (15, 8),
    (15, 3),
    (8, 15),
    (3, 15),
    (15, 3),
    (15, 8),
    (8, 15),
    (8, 15),
    (6, 15),
    (6, 15),
    (6, 15),
    (5, 15),
    (3, 15),
    (3, 8),
    (3, 15),
    (3, 8),
    (8, 15),
    (15, 3),
    (3, 15),
    (3, 8),
    (6, 15),
    (10, 8),
    (5, 3),
    (8, 15),
    (8, 6),
    (6, 10),
    (8, 15),
    (5, 15),
    (15, 10),
    (15, 8),
    (8, 15),
    (15, 3),
    (3, 15),
    (5, 10),
    (6, 10),
    (10, 8),
    (8, 9),
    (15, 10),
    (15, 6),
    (3, 15),
    (15, 8),
    (5, 15),
    (15, 3),
    (15, 6),
    (15, 6),
    (15, 8),
    (3, 15),
    (15, 3),
    (5, 15),
    (5, 15),
    (5, 15),
    (8, 15),
    (5, 15),
    (10, 15),
    (5, 15),
    (10, 15),
    (8, 15),
    (13, 15),
    (15, 3),
    (12, 15),
    (3, 15),
    (3, 8),
];

/// The weight, of 64, of each index of 2, 3 and 4 bits.
const WEIGHTS: [&[u8]; 3] = [
    &[0, 21, 43, 64],
    &[0, 9, 18, 27, 37, 46, 55, 64],
    &[0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64],
];

/// What a mode's block holds, in bits of each field; `p` is 0 for no p-bit, 1 for one at each end, 2 for one a subset shares.
struct Shape {
    subsets: usize,
    partition: u32,
    rotation: u32,
    selection: u32,
    colour: u32,
    alpha: u32,
    p: u8,
    index: u32,
    second: u32,
}

impl Shape {
    /// The bits of a mode's place among its ranks: its partition, rotation and selection.
    fn ranks(&self) -> u32 {
        self.partition + self.rotation + self.selection
    }
}

/// A mode's shape from its subsets, partition bits, rotation and selection bits, colour and alpha bits, p-bits and index bits.
const fn shape(
    subsets: usize,
    partition: u32,
    turns: [u32; 2],
    depths: [u32; 2],
    p: u8,
    indices: [u32; 2],
) -> Shape {
    Shape {
        subsets,
        partition,
        rotation: turns[0],
        selection: turns[1],
        colour: depths[0],
        alpha: depths[1],
        p,
        index: indices[0],
        second: indices[1],
    }
}

static SHAPES: [Shape; 8] = [
    shape(3, 4, [0, 0], [4, 0], 1, [3, 0]),
    shape(2, 6, [0, 0], [6, 0], 2, [3, 0]),
    shape(3, 6, [0, 0], [5, 0], 0, [2, 0]),
    shape(2, 6, [0, 0], [7, 0], 1, [2, 0]),
    shape(1, 0, [2, 1], [5, 6], 0, [2, 3]),
    shape(1, 0, [2, 0], [7, 8], 0, [2, 2]),
    shape(1, 0, [0, 0], [7, 7], 1, [4, 0]),
    shape(2, 6, [0, 0], [5, 5], 1, [2, 0]),
];

/// What lane 15 says of a block: its mode, and its place among the ranks of that mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Sig {
    pub(super) mode: usize,
    pub(super) position: usize,
}

/// The rank of each pair of lane 15's symbols: a 5-bit and a 4-bit symbol, ordered by the product of their successors, then by the first.
fn ranks() -> &'static [u16; RANKS] {
    static KEPT: OnceLock<[u16; RANKS]> = OnceLock::new();
    KEPT.get_or_init(|| {
        let mut pairs: Vec<(u16, u16)> =
            (0..32).flat_map(|a| (0..16).map(move |b| (a, b))).collect();
        pairs.sort_by_key(|&(a, b)| ((a + 1) * (b + 1), a));
        let mut ranks = [0u16; RANKS];
        for (rank, &(a, b)) in pairs.iter().enumerate() {
            ranks[usize::from(16 * a + b)] = rank as u16;
        }
        ranks
    })
}

/// The rank lane 15's two symbols of a block have; `None` for symbols too wide for the lane.
pub(super) fn rank(first: u16, second: u16) -> Option<usize> {
    (first < 32 && second < 16).then(|| usize::from(ranks()[usize::from(16 * first + second)]))
}

impl Sig {
    /// The signature lane 15's two symbols of a block name under `modes`, each rank's mode; `None` for a pair that names none.
    ///
    /// A mode's ranks count its partitions, rotations and selections in rank order.
    pub(super) fn of(first: u16, second: u16, modes: &[u8; RANKS]) -> Option<Self> {
        let rank = rank(first, second)?;
        let mode = modes[rank];
        let position = modes[..rank].iter().filter(|&&other| other == mode).count();
        let fits = mode < 8 && position < 1 << SHAPES[usize::from(mode)].ranks();
        fits.then_some(Self {
            mode: usize::from(mode),
            position,
        })
    }

    /// The bits the widest symbol of each of lanes 2 to 14 can take in a block of this signature, a lane's first symbol then its second.
    pub(super) fn widths(&self) -> [u8; WIDTHS] {
        let mut out = [0u8; WIDTHS];
        let mut widen = |lane: usize, bits: u32| {
            for symbol in &mut out[2 * (lane - 2)..2 * (lane - 1)] {
                *symbol = (*symbol).max(bits as u8);
            }
        };
        for (subset, lanes) in END_LANES.iter().enumerate() {
            for (channel, &lane) in lanes.iter().enumerate() {
                widen(lane, self.depth(subset, channel));
            }
        }
        for lane in FIRST_LANES {
            widen(lane, 2 * self.first_depth());
        }
        for lane in SECOND_LANES.into_iter().filter(|_| self.two_sets()) {
            widen(lane, 4);
        }
        out
    }

    fn shape(&self) -> &'static Shape {
        &SHAPES[self.mode]
    }

    pub(super) fn subsets(&self) -> usize {
        self.shape().subsets
    }

    /// Whether the block carries two index sets, which modes 4 and 5 do.
    pub(super) fn two_sets(&self) -> bool {
        self.mode == 4 || self.mode == 5
    }

    /// The channel the block's alpha field is swapped with, counted from 1; 0 for none.
    fn rotation(&self) -> usize {
        match self.mode {
            4 => 3 - self.position / 2,
            5 => 3 - self.position,
            _ => 0,
        }
    }

    /// Mode 4's index selection: 1 where the 3-bit set weighs the colour.
    fn selection(&self) -> usize {
        usize::from(self.mode == 4 && self.position.is_multiple_of(2))
    }

    /// The output channel that the block's alpha field holds.
    fn alpha_channel(&self) -> usize {
        match self.rotation() {
            0 => 3,
            turned => turned - 1,
        }
    }

    /// The bits of output channel `channel` of subset `subset`, a p-bit included in channel 0's; 0 where the mode has no such field.
    pub(super) fn depth(&self, subset: usize, channel: usize) -> u32 {
        let shape = self.shape();
        if subset >= shape.subsets {
            0
        } else if self.two_sets() {
            if channel == self.alpha_channel() {
                shape.alpha
            } else {
                shape.colour
            }
        } else {
            match channel {
                3 => shape.alpha,
                0 => shape.colour + u32::from(shape.p != 0),
                _ => shape.colour,
            }
        }
    }

    /// The bits of an index of the first set, the one lanes 11 to 14 carry.
    pub(super) fn first_depth(&self) -> u32 {
        match self.mode {
            4 => 3,
            _ => self.shape().index,
        }
    }

    /// Whether the first index set, not the second, weighs output channel `channel`.
    pub(super) fn first_weighs(&self, channel: usize) -> bool {
        let colour_by_first = match self.mode {
            4 => self.selection() == 1,
            _ => true,
        };
        !self.two_sets() || ((channel == self.alpha_channel()) != colour_by_first)
    }
}

/// The subset of pixel `pixel` under partition `partition` of a mode of `subsets` subsets.
pub(super) fn subset_of(subsets: usize, partition: usize, pixel: usize) -> usize {
    match subsets {
        2 => usize::from((TWO_SUBSETS[partition] >> pixel) & 1),
        3 => ((THREE_SUBSETS[partition] >> (2 * pixel)) & 3) as usize,
        _ => 0,
    }
}

/// The pixel of each subset whose index gives up its top bit; pixel 0 again where there are fewer than three.
pub(super) fn anchors(subsets: usize, partition: usize) -> [usize; 3] {
    match subsets {
        2 => [0, usize::from(SECOND_ANCHOR[partition]), 0],
        3 => {
            let (second, third) = THIRD_ANCHORS[partition];
            [0, usize::from(second), usize::from(third)]
        }
        _ => [0; 3],
    }
}

/// The weights, of 64, of the indices of `depth` bits.
pub(super) fn weights(depth: u32) -> &'static [u8] {
    WEIGHTS[depth.clamp(2, 4) as usize - 2]
}

/// The value `weight` of 64 of the way from `span`'s first end to its second.
pub(super) fn mix(span: [u8; 2], weight: u8) -> f32 {
    let (low, high, weight) = (u32::from(span[0]), u32::from(span[1]), u32::from(weight));
    ((low * (64 - weight) + high * weight + 32) >> 6) as f32
}

/// Where pixel `pixel`'s index lies: its row, which of the row's two symbols, and the parity of its bits there.
///
/// A symbol holds two pixels' indices bit by bit, lowest bit first: rows 0 to 2 keep the left
/// pixel on the odd bits, row 3 on the even ones.
pub(super) fn place(pixel: usize) -> (usize, usize, u32) {
    let row = pixel / 4;
    let odd = pixel.is_multiple_of(2) == (row < 3);
    (row, (pixel % 4) / 2, u32::from(odd))
}

/// Each byte of `word` cut to the bits of `keep`, then `adds` added to it, no carry passing between bytes.
fn through_bytes(word: u64, keep: u8, adds: u8) -> u64 {
    let (kept, adds) = (word & (BYTES * u64::from(keep)), BYTES * u64::from(adds));
    ((kept & (0x7F * BYTES)) + (adds & (0x7F * BYTES))) ^ ((kept ^ adds) & (0x80 * BYTES))
}

/// The same for each 4 bits of `word`.
pub(super) fn through_nibbles(word: u64, keep: u8, adds: u8) -> u64 {
    let (kept, adds) = (
        word & (NIBBLES * u64::from(keep)),
        NIBBLES * u64::from(adds),
    );
    ((kept & (7 * NIBBLES)) + (adds & (7 * NIBBLES))) ^ ((kept ^ adds) & (8 * NIBBLES))
}

/// `through`, a value left-aligned in 8 bits, as the 8 bits a block draws, by what [`Walk::widening`] gave for its field.
pub(super) fn widen((mut bits, p): (u32, Option<u32>), through: u8) -> u8 {
    let mut value = u32::from(through) >> (8 - bits);
    if let Some(p) = p {
        value = (value << 1) | p;
        bits += 1;
    }
    ((value << (8 - bits)) | (value >> (2 * bits - 8))) as u8
}

/// The coder's two presets and the partition order of modes 0 and 2, as far as a fit knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Presets {
    /// The endpoint values a reference to before a body's first block reads, by slot, and which of them are known.
    pub(super) ends: Vec<[u8; VALUES]>,
    pub(super) ends_known: Vec<[bool; VALUES]>,
    /// The first indices such a reference reads, left-aligned in 4 bits, and the slots known.
    pub(super) first: Vec<[u8; 16]>,
    pub(super) first_known: Vec<bool>,
    /// The second indices it reads, 2 bits each, and the slots known.
    pub(super) second: Vec<[u8; 16]>,
    pub(super) second_known: Vec<bool>,
    /// The partition of each position of mode 0's ranks and of mode 2's; [`UNPLACED`] where not fitted.
    pub(super) order0: [u8; 16],
    pub(super) order2: [u8; 64],
    /// The mode each rank of lane 15 names; [`UNPLACED`] for a rank no body showed.
    pub(super) modes: [u8; RANKS],
}

/// The slot nearest `slot` that `knows`, the lower of two as near.
pub(super) fn nearest(slot: usize, knows: impl Fn(usize) -> bool) -> Option<usize> {
    (1..256)
        .filter(|&other| knows(other))
        .min_by_key(|&other| other.abs_diff(slot))
}

/// `order` with each position not placed given a partition left over, the highest first.
fn filled<const N: usize>(order: &[u8; N]) -> [u8; N] {
    let mut left = (0..N as u8)
        .rev()
        .filter(|partition| !order.contains(partition));
    order.map(|placed| match placed {
        UNPLACED => left.next().unwrap_or(0),
        placed => placed,
    })
}

impl Presets {
    pub(super) fn empty() -> Self {
        Self {
            ends: vec![[0; VALUES]; 256],
            ends_known: vec![[false; VALUES]; 256],
            first: vec![[0; 16]; 256],
            first_known: vec![false; 256],
            second: vec![[0; 16]; 256],
            second_known: vec![false; 256],
            order0: [UNPLACED; 16],
            order2: [UNPLACED; 64],
            modes: [UNPLACED; RANKS],
        }
    }

    /// Whether a fit gave any slot a value.
    pub(super) fn any(&self) -> bool {
        self.first_known.contains(&true) || self.ends_known.iter().flatten().any(|&known| known)
    }

    /// The presets as a body is drawn through them: what is not known stands as the nearest slot
    /// that knows it has it, and is no more known for that.
    pub(super) fn stood_in(&self) -> Self {
        let mut out = self.clone();
        for slot in 1..256 {
            for value in 0..VALUES {
                let near = nearest(slot, |other| self.ends_known[other][value]);
                if let Some(near) = near.filter(|_| !self.ends_known[slot][value]) {
                    out.ends[slot][value] = self.ends[near][value];
                }
            }
            let near = nearest(slot, |other| self.first_known[other]);
            if let Some(near) = near.filter(|_| !self.first_known[slot]) {
                out.first[slot] = self.first[near];
            }
            let near = nearest(slot, |other| self.second_known[other]);
            if let Some(near) = near.filter(|_| !self.second_known[slot]) {
                out.second[slot] = self.second[near];
            }
        }
        out
    }

    /// The presets as a calibration file holds them.
    pub(super) fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PRESETS_BYTES);
        out.extend(self.ends.iter().flatten());
        out.extend(
            self.ends_known
                .iter()
                .flatten()
                .map(|&known| u8::from(known)),
        );
        out.extend(self.first.iter().flatten());
        out.extend(self.first_known.iter().map(|&known| u8::from(known)));
        out.extend(self.second.iter().flatten());
        out.extend(self.second_known.iter().map(|&known| u8::from(known)));
        out.extend(self.order0);
        out.extend(self.order2);
        out.extend(self.modes);
        out
    }

    /// The presets [`Self::to_bytes`] wrote; `None` for bytes of another length or an order that names no partition.
    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != PRESETS_BYTES {
            return None;
        }
        let mut presets = Self::empty();
        let mut rest = bytes.iter().copied();
        for value in presets.ends.iter_mut().flatten() {
            *value = rest.next()?;
        }
        for known in presets.ends_known.iter_mut().flatten() {
            *known = rest.next()? != 0;
        }
        for index in presets.first.iter_mut().flatten() {
            *index = rest.next()? & 0xF;
        }
        for known in &mut presets.first_known {
            *known = rest.next()? != 0;
        }
        for index in presets.second.iter_mut().flatten() {
            *index = rest.next()? & 3;
        }
        for known in &mut presets.second_known {
            *known = rest.next()? != 0;
        }
        let orders = presets.order0.iter_mut().chain(&mut presets.order2);
        for value in orders.chain(&mut presets.modes) {
            *value = rest.next()?;
        }
        let placed = |order: &[u8], count: u8| {
            (order.iter()).all(|&partition| partition < count || partition == UNPLACED)
        };
        let whole =
            placed(&presets.order0, 16) && placed(&presets.order2, 64) && placed(&presets.modes, 8);
        whole.then_some(presets)
    }
}

/// A body's blocks as its lanes chain them, each rebuilt through the presets as they stand.
///
/// A reference reaches at most 255 blocks back, so a walk keeps the last 256 blocks and its memory
/// does not grow with the body; what it gives is of the block last stepped.
pub(super) struct Walk<'a> {
    lanes: &'a [Vec<u16>],
    presets: &'a Presets,
    /// The signature each rank of lane 15 names.
    sigs: Vec<Option<Sig>>,
    order0: [u8; 16],
    order2: [u8; 64],
    pub(super) blocks: usize,
    /// Each block's endpoint values, and the preset value each hangs on: 0 for none, else 1 + 24 * slot + value.
    ends: Vec<[u8; VALUES]>,
    ends_from: Vec<[u16; VALUES]>,
    /// The bits the first block of each value's chain cut from the preset value it read: the value
    /// is the same for every preset value that differs only in those bits.
    ends_cut: Vec<[u8; VALUES]>,
    /// Each block's first indices, left-aligned in 4 bits, and its second.
    first: Vec<[u8; 16]>,
    second: Vec<[u8; 16]>,
    /// The preset slot each index set hangs on; 0 for none.
    first_from: Vec<u8>,
    second_from: Vec<u8>,
    /// What each second index adds to its preset slot's.
    second_adds: Vec<[u8; 16]>,
    depths: Vec<u8>,
    /// For each of the last 256 blocks, where traced: each endpoint value, and each first index, as a
    /// function of the preset value it hangs on. A difference is added below a truncation that
    /// depends on the value, so no sum stands for it. An endpoint value's 256 are bytes of 32
    /// words, a first index's 16 the 4-bit digits of one, the lowest first.
    ends_trace: Vec<u64>,
    first_trace: Vec<u64>,
}

impl<'a> Walk<'a> {
    /// A walk over the blocks of `lanes`, which [`Self::step`] takes one by one; `trace` keeps what a fit reads.
    pub(super) fn new(
        lanes: &'a [Vec<u16>],
        presets: &'a Presets,
        trace: bool,
    ) -> Result<Self, BcPackError> {
        let symbols = lanes.first().map_or(0, Vec::len);
        let whole = lanes.len() == 16
            && symbols.is_multiple_of(2)
            && lanes.iter().all(|lane| lane.len() == symbols);
        if !whole {
            return Err(BcPackError::Size {
                size: 8 * symbols as u64,
            });
        }
        let blocks = symbols / 2;
        let rank_of = ranks();
        let mut sigs = vec![None; RANKS];
        for first in 0..32 {
            for second in 0..16 {
                sigs[usize::from(rank_of[usize::from(16 * first + second)])] =
                    Sig::of(first, second, &presets.modes);
            }
        }
        Ok(Self {
            lanes,
            presets,
            sigs,
            order0: filled(&presets.order0),
            order2: filled(&presets.order2),
            blocks,
            ends: vec![[0; VALUES]; REACH],
            ends_from: vec![[0; VALUES]; REACH],
            ends_cut: vec![[0; VALUES]; REACH],
            first: vec![[0; 16]; REACH],
            second: vec![[0; 16]; REACH],
            first_from: vec![0; REACH],
            second_from: vec![0; REACH],
            second_adds: vec![[0; 16]; REACH],
            depths: vec![0; REACH],
            ends_trace: vec![
                0;
                if trace {
                    REACH * VALUES * TRACE_WORDS
                } else {
                    0
                }
            ],
            first_trace: vec![0; if trace { REACH * 16 } else { 0 }],
        })
    }

    /// Rebuilds block `block`, the blocks before it being rebuilt already, and gives its signature.
    pub(super) fn step(&mut self, block: usize) -> Result<Sig, BcPackError> {
        let lanes = self.lanes;
        let at = 2 * block;
        let (first, second) = (lanes[15][at], lanes[15][at + 1]);
        let sig = (rank(first, second).and_then(|rank| self.sigs[rank])).ok_or(
            BcPackError::Signature {
                block,
                first,
                second,
            },
        )?;
        let index_back = i32::from(interleave(lanes[1][at], lanes[1][at + 1]) & 0xFF);
        let moved = difference(interleave(lanes[0][at], lanes[0][at + 1]));
        let end_back = (index_back + moved) & 0xFF;
        self.step_ends(block, sig, end_back as usize);
        self.step_indices(block, sig, index_back as usize);
        Ok(sig)
    }

    fn end_trace_at(block: usize, value: usize) -> usize {
        ((block % REACH) * VALUES + value) * TRACE_WORDS
    }

    fn first_trace_at(block: usize, pixel: usize) -> usize {
        (block % REACH) * 16 + pixel
    }

    fn step_ends(&mut self, block: usize, sig: Sig, back: usize) {
        let shape = sig.shape();
        let traced = !self.ends_trace.is_empty();
        let mut ends = [0u8; VALUES];
        let mut from = [0u16; VALUES];
        let mut cut = [0u8; VALUES];
        for subset in 0..3 {
            for channel in 0..4 {
                let depth = sig.depth(subset, channel);
                for end in 0..2 {
                    let value = 2 * (4 * subset + channel) + end;
                    let to = Self::end_trace_at(block, value);
                    if depth == 0 {
                        // A subset the mode lacks repeats subset 0; an alpha it lacks is zero.
                        if subset >= shape.subsets && (channel < 3 || shape.alpha > 0) {
                            let source = 2 * channel + end;
                            ends[value] = ends[source];
                            from[value] = from[source];
                            cut[value] = cut[source];
                            if traced && from[source] != 0 {
                                let source = Self::end_trace_at(block, source);
                                (self.ends_trace).copy_within(source..source + TRACE_WORDS, to);
                            }
                        }
                        continue;
                    }
                    let lane = &self.lanes[END_LANES[subset][channel]];
                    let shift = 8 - depth;
                    let adds = ((difference(lane[2 * block + end]) << shift) & 0xFF) as u8;
                    let keep = (0xFFu32 << shift) as u8;
                    if back == 0 {
                        ends[value] = adds;
                    } else if back <= block {
                        let named = block - back;
                        let kept = named % REACH;
                        ends[value] = (self.ends[kept][value] & keep).wrapping_add(adds);
                        from[value] = self.ends_from[kept][value];
                        cut[value] = self.ends_cut[kept][value];
                        if traced && from[value] != 0 {
                            let named = Self::end_trace_at(named, value);
                            for x in 0..TRACE_WORDS {
                                self.ends_trace[to + x] =
                                    through_bytes(self.ends_trace[named + x], keep, adds);
                            }
                        }
                    } else {
                        let slot = 256 + block - back;
                        ends[value] = (self.presets.ends[slot][value] & keep).wrapping_add(adds);
                        from[value] = (1 + VALUES * slot + value) as u16;
                        cut[value] = shift as u8;
                        if traced {
                            let through = &mut self.ends_trace[to..to + TRACE_WORDS];
                            for (x, through) in through.iter_mut().enumerate() {
                                // The eight values from 8 * x on, the lowest in the lowest byte.
                                let plain = 0x0706_0504_0302_0100 + BYTES * (8 * x as u64);
                                *through = through_bytes(plain, keep, adds);
                            }
                        }
                    }
                }
            }
        }
        self.ends[block % REACH] = ends;
        self.ends_from[block % REACH] = from;
        self.ends_cut[block % REACH] = cut;
    }

    fn step_indices(&mut self, block: usize, sig: Sig, back: usize) {
        let depth = sig.first_depth();
        let shift = 4 - depth;
        let keep = (0xFu8 << shift) & 0xF;
        let two = sig.two_sets();
        let traced = !self.first_trace.is_empty();
        let named = (back > 0 && back <= block).then(|| block - back);
        let slot = (back > block).then(|| 256 + block - back);
        let hangs = |from: &[u8]| match (named, slot) {
            (Some(named), _) => from[named % REACH],
            (None, Some(slot)) => slot as u8,
            (None, None) => 0,
        };
        let first_from = hangs(&self.first_from);
        let second_from = if two { hangs(&self.second_from) } else { 0 };
        let (mut first, mut second, mut adds_up) = ([0u8; 16], [0u8; 16], [0u8; 16]);
        for pixel in 0..16 {
            let (row, half, odd) = place(pixel);
            let symbol = self.lanes[FIRST_LANES[row]][2 * block + half];
            let field = (0..depth).fold(0, |all, bit| {
                all | (((symbol >> (2 * bit + odd)) & 1) << bit)
            });
            let adds = ((difference(field) << shift) & 0xF) as u8;
            let to = Self::first_trace_at(block, pixel);
            first[pixel] = match (named, slot) {
                (Some(named), _) => {
                    if traced && first_from != 0 {
                        let named = Self::first_trace_at(named, pixel);
                        self.first_trace[to] = through_nibbles(self.first_trace[named], keep, adds);
                    }
                    ((self.first[named % REACH][pixel] & keep) + adds) & 0xF
                }
                (None, Some(slot)) => {
                    if traced {
                        self.first_trace[to] = through_nibbles(0xFEDC_BA98_7654_3210, keep, adds);
                    }
                    ((self.presets.first[slot][pixel] & keep) + adds) & 0xF
                }
                (None, None) => adds,
            };
            if two {
                let symbol = self.lanes[SECOND_LANES[row]][2 * block + half];
                let field = ((symbol >> odd) & 1) | (((symbol >> (2 + odd)) & 1) << 1);
                let adds = (difference(field) & 3) as u8;
                let (base, carried) = match (named, slot) {
                    (Some(named), _) => (
                        self.second[named % REACH][pixel],
                        self.second_adds[named % REACH][pixel],
                    ),
                    (None, Some(slot)) => (self.presets.second[slot][pixel], 0),
                    (None, None) => (0, 0),
                };
                second[pixel] = (base + adds) & 3;
                adds_up[pixel] = (carried + adds) & 3;
            }
        }
        let at = block % REACH;
        self.first[at] = first;
        self.second[at] = second;
        self.second_adds[at] = adds_up;
        self.first_from[at] = first_from;
        self.second_from[at] = second_from;
        self.depths[at] = depth as u8;
    }

    /// The partition of a block of signature `sig`: modes 1, 3 and 7 count theirs down from 63.
    pub(super) fn partition(&self, sig: Sig) -> usize {
        match sig.mode {
            0 => usize::from(self.order0[sig.position]),
            2 => usize::from(self.order2[sig.position]),
            1 | 3 | 7 => 63 - sig.position,
            _ => 0,
        }
    }

    /// Whether every endpoint value block `block` uses is free of the presets or hangs on a known value.
    pub(super) fn ends_known(&self, block: usize, sig: Sig) -> bool {
        (self.ends_from[block % REACH].iter().enumerate()).all(|(value, &from)| {
            let unused = sig.depth(value / 8, (value / 2) % 4) == 0;
            let at = usize::from(from.saturating_sub(1));
            from == 0 || unused || self.presets.ends_known[at / VALUES][at % VALUES]
        })
    }

    /// Whether block `block`'s indices are free of the presets or hang on known slots.
    pub(super) fn indices_known(&self, block: usize) -> bool {
        let (first, second) = self.index_slots(block);
        (first == 0 || self.presets.first_known[usize::from(first)])
            && (second == 0 || self.presets.second_known[usize::from(second)])
    }

    /// The preset slots block `block`'s first and second indices hang on; 0 for none.
    pub(super) fn index_slots(&self, block: usize) -> (u8, u8) {
        let at = block % REACH;
        (self.first_from[at], self.second_from[at])
    }

    /// The preset value that endpoint value `value` of block `block` hangs on: 0 for none, else 1 + 24 * slot + value.
    pub(super) fn end_from(&self, block: usize, value: usize) -> u16 {
        self.ends_from[block % REACH][value]
    }

    /// The low bits of its preset value that endpoint value `value` of block `block` does not depend on.
    pub(super) fn end_cut(&self, block: usize, value: usize) -> u32 {
        u32::from(self.ends_cut[block % REACH][value])
    }

    /// What second index `pixel` of block `block` adds to its preset slot's.
    pub(super) fn second_add(&self, block: usize, pixel: usize) -> u8 {
        self.second_adds[block % REACH][pixel]
    }

    /// Endpoint value `value` of block `block` for each value its preset value could take, eight to a word from the lowest byte; only for the block last stepped, in a traced walk.
    pub(super) fn end_trace(&self, block: usize, value: usize) -> &[u64] {
        let at = Self::end_trace_at(block, value);
        &self.ends_trace[at..at + TRACE_WORDS]
    }

    /// First index `pixel` of block `block`, left-aligned in 4 bits, for each value its preset index could take, the 4-bit digits of a word from the lowest; as [`Self::end_trace`].
    pub(super) fn first_trace(&self, block: usize, pixel: usize) -> u64 {
        self.first_trace[Self::first_trace_at(block, pixel)]
    }

    /// The two ends of output channel `channel` of subset `subset` of block `block` as 8 bits; `None` where its mode has no such field.
    pub(super) fn span(
        &self,
        block: usize,
        sig: Sig,
        subset: usize,
        channel: usize,
    ) -> Option<[u8; 2]> {
        if sig.depth(subset, channel) == 0 {
            return None;
        }
        let ends = &self.ends[block % REACH];
        Some(std::array::from_fn(|end| {
            let value = ends[2 * (4 * subset + channel) + end];
            self.wide(block, sig, (subset, channel, end), value)
        }))
    }

    /// `through`, a value left-aligned in 8 bits of the field `of` names by subset, output channel and end, as the 8 bits block `block` draws; the mode must have the field.
    ///
    /// Past channel 0, a mode with p-bits ends each value on the p-bit that channel 0's value
    /// carries; a value of fewer than 8 bits has its top bits repeated below it.
    pub(super) fn wide(
        &self,
        block: usize,
        sig: Sig,
        of: (usize, usize, usize),
        through: u8,
    ) -> u8 {
        widen(self.widening(block, sig, of), through)
    }

    /// How [`widen`] draws a value of the field `of` names in block `block`: the field's bits, and the p-bit that ends it, if one does.
    pub(super) fn widening(
        &self,
        block: usize,
        sig: Sig,
        of: (usize, usize, usize),
    ) -> (u32, Option<u32>) {
        let (subset, channel, end) = of;
        let p = (sig.shape().p != 0 && channel != 0).then(|| {
            let first = u32::from(self.ends[block % REACH][8 * subset + end]);
            (first >> (8 - sig.depth(subset, 0))) & 1
        });
        (sig.depth(subset, channel), p)
    }

    /// The weight, of 64, that pixel `pixel` of block `block` gives output channel `channel`.
    pub(super) fn weight(&self, block: usize, sig: Sig, pixel: usize, channel: usize) -> u8 {
        if sig.first_weighs(channel) {
            let depth = u32::from(self.depths[block % REACH]);
            weights(depth)[usize::from(self.first[block % REACH][pixel] >> (4 - depth))]
        } else {
            weights(2)[usize::from(self.second[block % REACH][pixel])]
        }
    }

    /// Block `block` as the 16 bytes of a BC7 block.
    fn write(&self, block: usize, sig: Sig) -> [u8; 16] {
        let shape = sig.shape();
        let partition = self.partition(sig);
        let ends = &self.ends[block % REACH];
        let (mut word, mut at) = (0u128, 0u32);
        let mut put = |value: u32, bits: u32| {
            word |= u128::from(value & ((1 << bits) - 1)) << at;
            at += bits;
        };
        put(1 << sig.mode, sig.mode as u32 + 1);
        put(partition as u32, shape.partition);
        put(sig.rotation() as u32, shape.rotation);
        put(sig.selection() as u32, shape.selection);
        // A block's alpha field holds the output channel it is rotated with, and that channel's field the alpha.
        let alpha = sig.alpha_channel();
        let field = |subset: usize, stored: usize, end: usize| {
            let channel = match stored {
                3 => alpha,
                stored if stored == alpha => 3,
                stored => stored,
            };
            let value = u32::from(ends[2 * (4 * subset + channel) + end]);
            let whole = value >> (8 - sig.depth(subset, channel));
            if shape.p != 0 && channel == 0 {
                whole >> 1
            } else {
                whole
            }
        };
        for channel in 0..3 {
            for subset in 0..shape.subsets {
                for end in 0..2 {
                    put(field(subset, channel, end), shape.colour);
                }
            }
        }
        if shape.alpha > 0 {
            for subset in 0..shape.subsets {
                for end in 0..2 {
                    put(field(subset, 3, end), shape.alpha);
                }
            }
        }
        // A p-bit is the lowest bit of channel 0's value; a shared one is the first end's.
        for subset in 0..shape.subsets {
            let p =
                |end: usize| (u32::from(ends[8 * subset + end]) >> (8 - sig.depth(subset, 0))) & 1;
            match shape.p {
                1 => {
                    put(p(0), 1);
                    put(p(1), 1);
                }
                2 => put(p(0), 1),
                _ => {}
            }
        }
        let depth = sig.first_depth();
        let first = |pixel: usize| u32::from(self.first[block % REACH][pixel]) >> (4 - depth);
        let second = |pixel: usize| u32::from(self.second[block % REACH][pixel]);
        // Mode 4 stores its 2-bit set first, which lanes 6, 7, 8 and 10 carry; an anchor's top bit is not stored.
        let anchors = anchors(shape.subsets, partition);
        for pixel in 0..16 {
            let index = match sig.mode {
                4 => second(pixel),
                _ => first(pixel),
            };
            put(index, shape.index - u32::from(anchors.contains(&pixel)));
        }
        if shape.second > 0 {
            for pixel in 0..16 {
                let index = match sig.mode {
                    4 => first(pixel),
                    _ => second(pixel),
                };
                put(index, shape.second - u32::from(pixel == 0));
            }
        }
        word.to_le_bytes()
    }
}

/// The BC7 blocks that `lanes` rebuild through `presets`, and how many hang on a preset value that is not known.
///
/// A value that is not known is taken from the nearest slot that knows it, and its block counted.
pub(super) fn blocks(lanes: &[Vec<u16>], presets: &Presets) -> Result<Rebuilt, BcPackError> {
    let presets = &presets.stood_in();
    let mut walk = Walk::new(lanes, presets, false)?;
    let mut bytes = Vec::with_capacity(16 * walk.blocks);
    let mut unknown = 0;
    for block in 0..walk.blocks {
        let sig = walk.step(block)?;
        let known = walk.ends_known(block, sig) && walk.indices_known(block);
        unknown += usize::from(!known);
        bytes.extend(walk.write(block, sig));
    }
    Ok(Rebuilt { bytes, unknown })
}

#[cfg(test)]
#[path = "bptc_tests.rs"]
mod tests;
