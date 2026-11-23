//! BC6H's blocks from the 16 lanes of a BCPack body.
//!
//! Lane 15's first symbol is a block's mode in the format's order and its second the partition.
//! Lanes 2 to 4 carry the base of red, green and blue, lanes 5 to 7 the first region's other end,
//! lanes 8 to 10 the second region's two ends; each field is a difference from the same field of
//! the block the endpoints refer to, cut to this block's depth. The two distances are BC7's and
//! so are the indices, but that an index with no reference is a difference from half its range,
//! as a field is. A reference to before a body's first block reads two presets that are constants
//! of the coder and in no stream, so [`Presets`] is fitted on the user's machine.

use std::sync::OnceLock;

use super::bc1::difference;
use super::bptc::{anchors, nearest, place, subset_of, through_nibbles, weights};
use super::planes::{interleave, Rebuilt};
use super::BcPackError;

/// The fields of a block: red, green and blue of the base, then of each of the three other ends.
pub(super) const FIELDS: usize = 12;
/// The values a fit tries for a preset field in one pass: 8 of its 16 bits.
pub(super) const CANDIDATES: usize = 256;
/// The bytes [`Presets::to_bytes`] writes.
pub(super) const PRESETS_BYTES: usize = 256 * (2 * FIELDS + FIELDS + 16 + 1);
/// The blocks a reference reaches back over: a walk keeps this many.
const REACH: usize = 256;
/// What a field is a difference from where its block refers to none: the top bit alone.
const HALF: u16 = 0x8000;
/// The lanes of the four rows of indices.
const INDEX_LANES: [usize; 4] = [11, 12, 13, 14];

/// A mode's regions, whether its other ends are differences from the base, the bits of the base
/// and of the other ends' red, green and blue, its header and the header's bits, and the fields
/// its block holds after the header, bit by bit: `gy4` is bit 4 of the second region's first
/// end's green, `rw0-9` ten bits of the base's red from the lowest, `d` the partition.
type Stated = (usize, bool, [u32; 4], (u32, u32), &'static str);

/// The 14 modes in the block format's order.
const STATED: [Stated; 14] = [
    (2, true, [10, 5, 5, 5], (0x00, 2), "gy4 by4 bz4 rw0-9 gw0-9 bw0-9 rx0-4 gz4 gy0-3 gx0-4 bz0 gz0-3 bx0-4 bz1 by0-3 ry0-4 bz2 rz0-4 bz3 d0-4"),
    (2, true, [7, 6, 6, 6], (0x01, 2), "gy5 gz4 gz5 rw0-6 bz0 bz1 by4 gw0-6 by5 bz2 gy4 bw0-6 bz3 bz5 bz4 rx0-5 gy0-3 gx0-5 gz0-3 bx0-5 by0-3 ry0-5 rz0-5 d0-4"),
    (2, true, [11, 5, 4, 4], (0x02, 5), "rw0-9 gw0-9 bw0-9 rx0-4 rw10 gy0-3 gx0-3 gw10 bz0 gz0-3 bx0-3 bw10 bz1 by0-3 ry0-4 bz2 rz0-4 bz3 d0-4"),
    (2, true, [11, 4, 5, 4], (0x06, 5), "rw0-9 gw0-9 bw0-9 rx0-3 rw10 gz4 gy0-3 gx0-4 gw10 gz0-3 bx0-3 bw10 bz1 by0-3 ry0-3 bz0 bz2 rz0-3 gy4 bz3 d0-4"),
    (2, true, [11, 4, 4, 5], (0x0a, 5), "rw0-9 gw0-9 bw0-9 rx0-3 rw10 by4 gy0-3 gx0-3 gw10 bz0 gz0-3 bx0-4 bw10 by0-3 ry0-3 bz1 bz2 rz0-3 bz4 bz3 d0-4"),
    (2, true, [9, 5, 5, 5], (0x0e, 5), "rw0-8 by4 gw0-8 gy4 bw0-8 bz4 rx0-4 gz4 gy0-3 gx0-4 bz0 gz0-3 bx0-4 bz1 by0-3 ry0-4 bz2 rz0-4 bz3 d0-4"),
    (2, true, [8, 6, 5, 5], (0x12, 5), "rw0-7 gz4 by4 gw0-7 bz2 gy4 bw0-7 bz3 bz4 rx0-5 gy0-3 gx0-4 bz0 gz0-3 bx0-4 bz1 by0-3 ry0-5 rz0-5 d0-4"),
    (2, true, [8, 5, 6, 5], (0x16, 5), "rw0-7 bz0 by4 gw0-7 gy5 gy4 bw0-7 gz5 bz4 rx0-4 gz4 gy0-3 gx0-5 gz0-3 bx0-4 bz1 by0-3 ry0-4 bz2 rz0-4 bz3 d0-4"),
    (2, true, [8, 5, 5, 6], (0x1a, 5), "rw0-7 bz1 by4 gw0-7 by5 gy4 bw0-7 bz5 bz4 rx0-4 gz4 gy0-3 gx0-4 bz0 gz0-3 bx0-5 by0-3 ry0-4 bz2 rz0-4 bz3 d0-4"),
    (2, false, [6, 6, 6, 6], (0x1e, 5), "rw0-5 gz4 bz0 bz1 by4 gw0-5 gy5 by5 bz2 gy4 bw0-5 gz5 bz3 bz5 bz4 rx0-5 gy0-3 gx0-5 gz0-3 bx0-5 by0-3 ry0-5 rz0-5 d0-4"),
    (1, false, [10, 10, 10, 10], (0x03, 5), "rw0-9 gw0-9 bw0-9 rx0-9 gx0-9 bx0-9"),
    (1, true, [11, 9, 9, 9], (0x07, 5), "rw0-9 gw0-9 bw0-9 rx0-8 rw10 gx0-8 gw10 bx0-8 bw10"),
    (1, true, [12, 8, 8, 8], (0x0b, 5), "rw0-9 gw0-9 bw0-9 rx0-7 rw11-10 gx0-7 gw11-10 bx0-7 bw11-10"),
    (1, true, [16, 4, 4, 4], (0x0f, 5), "rw0-9 gw0-9 bw0-9 rx0-3 rw15-10 gx0-3 gw15-10 bx0-3 bw15-10"),
];

/// What a mode's block holds.
pub(super) struct Shape {
    pub(super) regions: usize,
    transformed: bool,
    /// The bits of the base.
    pub(super) base: u32,
    /// The bits of the other ends' red, green and blue.
    delta: [u32; 3],
    header: (u32, u32),
    /// The field and the bit of it that each bit after the header holds; field [`FIELDS`] is the partition.
    layout: Vec<(usize, u32)>,
}

impl Shape {
    /// The bits of field `field` in a block's lanes; 0 for a field of the second region in a mode of one.
    pub(super) fn depth(&self, field: usize) -> u32 {
        if field < 3 {
            self.base
        } else if field >= 6 && self.regions == 1 {
            0
        } else {
            self.delta[field % 3]
        }
    }

    /// The bits of an index.
    pub(super) fn index_depth(&self) -> u32 {
        if self.regions == 2 {
            3
        } else {
            4
        }
    }
}

/// The fields `text` states, bit by bit.
fn layout(text: &str) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        let split = (token.find(|c: char| c.is_ascii_digit())).unwrap_or(token.len());
        let (name, range) = token.split_at(split);
        let field = match *name.as_bytes() {
            [channel, end] => {
                let at = |all: &str, one: u8| all.bytes().position(|c| c == one).unwrap_or(0);
                3 * at("wxyz", end) + at("rgb", channel)
            }
            _ => FIELDS,
        };
        let number = |digits: &str| digits.parse::<u32>().unwrap_or(0);
        let (from, to) = match range.split_once('-') {
            Some((from, to)) => (number(from), number(to)),
            None => (number(range), number(range)),
        };
        if from <= to {
            out.extend((from..=to).map(|bit| (field, bit)));
        } else {
            out.extend((to..=from).rev().map(|bit| (field, bit)));
        }
    }
    out
}

/// The 14 modes.
pub(super) fn shapes() -> &'static [Shape] {
    static KEPT: OnceLock<Vec<Shape>> = OnceLock::new();
    KEPT.get_or_init(|| {
        (STATED.iter())
            .map(|&(regions, transformed, bits, header, stated)| Shape {
                regions,
                transformed,
                base: bits[0],
                delta: [bits[1], bits[2], bits[3]],
                header,
                layout: layout(stated),
            })
            .collect()
    })
}

/// Two 8-bit symbols as one 16-bit field, the first on the even bits.
fn interleave_bytes(first: u16, second: u16) -> u16 {
    interleave(first, second) | interleave(first >> 4, second >> 4) << 8
}

/// A base as its lanes hold it, as its value: the bits past the tenth lie in the block's order, the reverse of their weight.
fn base_value(stored: u32, bits: u32) -> u32 {
    let Some(more) = bits.checked_sub(10).filter(|&more| more > 1) else {
        return stored;
    };
    let turned = (0..more).fold(0, |all, bit| {
        all | ((stored >> (10 + bit)) & 1) << (more - 1 - bit)
    });
    (stored & 0x3FF) | turned << 10
}

/// End `end` of channel `channel` of a block of `shape` whose fields are `fields`, each left-aligned in 16 bits, in the bits of the base.
pub(super) fn end_value(shape: &Shape, fields: &[u16; FIELDS], end: usize, channel: usize) -> u32 {
    let base = base_value(u32::from(fields[channel]) >> (16 - shape.base), shape.base);
    if end == 0 {
        return base;
    }
    let bits = shape.delta[channel];
    let other = u32::from(fields[3 * end + channel]) >> (16 - bits);
    if !shape.transformed {
        return other;
    }
    // A difference is signed and is added to the base within the base's bits.
    let signed = ((other << (32 - bits)) as i32) >> (32 - bits);
    (base as i32 + signed) as u32 & ((1 << shape.base) - 1)
}

/// An end of `bits` bits as the 16 bits the unsigned format interpolates.
pub(super) fn unquantised(value: u32, bits: u32) -> u32 {
    if bits >= 15 {
        value
    } else if value == 0 {
        0
    } else if value == (1 << bits) - 1 {
        0xFFFF
    } else {
        ((value << 15) + 0x4000) >> (bits - 1)
    }
}

/// The value `weight` of 64 of the way from `low` to `high`, as the unsigned format interpolates two ends.
pub(super) fn mix(low: u32, high: u32, weight: u8) -> u32 {
    let weight = u32::from(weight);
    (low * (64 - weight) + high * weight + 32) >> 6
}

/// The coder's two presets, as far as a fit knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Presets {
    /// The fields a reference to before a body's first block reads, by slot, each left-aligned
    /// in 16 bits, and how many of each's top bits a fit has settled: 0, 8 or 16.
    pub(super) ends: Vec<[u16; FIELDS]>,
    pub(super) ends_bits: Vec<[u8; FIELDS]>,
    /// The indices such a reference reads, left-aligned in 4 bits, and the slots known.
    pub(super) first: Vec<[u8; 16]>,
    pub(super) first_known: Vec<bool>,
}

impl Presets {
    pub(super) fn empty() -> Self {
        Self {
            ends: vec![[0; FIELDS]; 256],
            ends_bits: vec![[0; FIELDS]; 256],
            first: vec![[0; 16]; 256],
            first_known: vec![false; 256],
        }
    }

    /// Whether a fit gave any slot a value.
    pub(super) fn any(&self) -> bool {
        self.first_known.contains(&true) || self.ends_bits.iter().flatten().any(|&bits| bits > 0)
    }

    /// The presets as a body is drawn through them: what is not known stands as the nearest slot
    /// that knows it has it, and is no more known for that.
    pub(super) fn stood_in(&self) -> Self {
        let mut out = self.clone();
        for slot in 1..256 {
            for field in 0..FIELDS {
                let near = nearest(slot, |other| self.ends_bits[other][field] > 0);
                if let Some(near) = near.filter(|_| self.ends_bits[slot][field] == 0) {
                    out.ends[slot][field] = self.ends[near][field];
                }
            }
            let near = nearest(slot, |other| self.first_known[other]);
            if let Some(near) = near.filter(|_| !self.first_known[slot]) {
                out.first[slot] = self.first[near];
            }
        }
        out
    }

    /// The value a fit tries as number `candidate` for field `field` of slot `slot`: each value
    /// of the top 8 bits first, then the 256 values around the one that pass took, which is the
    /// nearest of its kind and so at most 128 away.
    pub(super) fn candidate(&self, slot: usize, field: usize, candidate: usize) -> u16 {
        let value = self.ends[slot][field];
        match self.ends_bits[slot][field] {
            0 => (candidate << 8) as u16,
            8 => value.wrapping_add(candidate as u16).wrapping_sub(127),
            _ => value,
        }
    }

    /// The presets as a calibration file holds them.
    pub(super) fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PRESETS_BYTES);
        out.extend((self.ends.iter().flatten()).flat_map(|value| value.to_le_bytes()));
        out.extend(self.ends_bits.iter().flatten());
        out.extend(self.first.iter().flatten());
        out.extend(self.first_known.iter().map(|&known| u8::from(known)));
        out
    }

    /// The presets [`Self::to_bytes`] wrote; `None` for bytes of another length or a count of settled bits no fit leaves.
    pub(super) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != PRESETS_BYTES {
            return None;
        }
        let mut presets = Self::empty();
        let mut rest = bytes.iter().copied();
        for value in presets.ends.iter_mut().flatten() {
            *value = u16::from_le_bytes([rest.next()?, rest.next()?]);
        }
        for bits in presets.ends_bits.iter_mut().flatten() {
            *bits = rest.next()?;
        }
        for index in presets.first.iter_mut().flatten() {
            *index = rest.next()? & 0xF;
        }
        for known in &mut presets.first_known {
            *known = rest.next()? != 0;
        }
        let settled = |&bits: &u8| matches!(bits, 0 | 8 | 16);
        (presets.ends_bits.iter().flatten().all(settled)).then_some(presets)
    }
}

/// A body's blocks as its lanes chain them, each rebuilt through the presets as they stand.
///
/// A reference reaches at most 255 blocks back, so a walk keeps the last 256 blocks and its memory
/// does not grow with the body; what it gives is of the block last stepped.
pub(super) struct Walk<'a> {
    lanes: &'a [Vec<u16>],
    presets: &'a Presets,
    pub(super) blocks: usize,
    /// Each block's fields, and the preset field each hangs on: 0 for none, else 1 + 12 * slot + field.
    fields: Vec<[u16; FIELDS]>,
    from: Vec<[u16; FIELDS]>,
    /// Each block's indices, left-aligned in 4 bits, and the preset slot they hang on.
    first: Vec<[u8; 16]>,
    first_from: Vec<u8>,
    depths: Vec<u8>,
    /// For each of the last 256 blocks, where traced: each field for each value a fit tries for
    /// the preset field it hangs on, and each index for each value its preset index could take,
    /// the 4-bit digits of a word from the lowest.
    trace: Vec<u16>,
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
        Ok(Self {
            lanes,
            presets,
            blocks: symbols / 2,
            fields: vec![[0; FIELDS]; REACH],
            from: vec![[0; FIELDS]; REACH],
            first: vec![[0; 16]; REACH],
            first_from: vec![0; REACH],
            depths: vec![0; REACH],
            trace: vec![
                0;
                if trace {
                    REACH * FIELDS * CANDIDATES
                } else {
                    0
                }
            ],
            first_trace: vec![0; if trace { REACH * 16 } else { 0 }],
        })
    }

    /// Rebuilds block `block`, the blocks before it being rebuilt already, and gives its mode and its partition.
    pub(super) fn step(&mut self, block: usize) -> Result<(&'static Shape, usize), BcPackError> {
        let lanes = self.lanes;
        let at = 2 * block;
        let (first, second) = (lanes[15][at], lanes[15][at + 1]);
        let shape = (shapes().get(usize::from(first)))
            .filter(|_| second < 32)
            .ok_or(BcPackError::Signature {
                block,
                first,
                second,
            })?;
        let index_back = i32::from(interleave(lanes[1][at], lanes[1][at + 1]) & 0xFF);
        let moved = difference(interleave(lanes[0][at], lanes[0][at + 1]));
        let end_back = (index_back + moved) & 0xFF;
        self.step_ends(block, shape, end_back as usize);
        self.step_indices(block, shape, index_back as usize);
        Ok((shape, usize::from(second)))
    }

    fn trace_at(block: usize, field: usize) -> usize {
        ((block % REACH) * FIELDS + field) * CANDIDATES
    }

    fn step_ends(&mut self, block: usize, shape: &Shape, back: usize) {
        let traced = !self.trace.is_empty();
        let named = (back > 0 && back <= block).then(|| block - back);
        let slot = (back > block).then(|| 256 + block - back);
        let mut fields = [0u16; FIELDS];
        let mut from = [0u16; FIELDS];
        for field in 0..FIELDS {
            let to = Self::trace_at(block, field);
            let depth = shape.depth(field);
            if depth == 0 {
                // A block of one region leaves its other end as the second region's last end, and
                // its base as the first: whole where its ends are not differences, else the low
                // bits of a difference's width, the top one of them flipped.
                let channel = field % 3;
                let (source, cut) = match (field >= 9, shape.transformed) {
                    (true, _) => (3 + channel, None),
                    (false, false) => (channel, None),
                    (false, true) => (channel, Some(shape.base - shape.delta[channel])),
                };
                let left = |value: u16| match cut {
                    Some(cut) => (value << cut) ^ HALF,
                    None => value,
                };
                fields[field] = left(fields[source]);
                from[field] = from[source];
                if traced && from[source] != 0 {
                    let source = Self::trace_at(block, source);
                    for candidate in 0..CANDIDATES {
                        self.trace[to + candidate] = left(self.trace[source + candidate]);
                    }
                }
                continue;
            }
            let symbol = if field < 6 {
                let lane = &self.lanes[2 + field];
                interleave_bytes(lane[2 * block], lane[2 * block + 1])
            } else {
                self.lanes[8 + field % 3][2 * block + usize::from(field >= 9)]
            };
            let shift = 16 - depth;
            let adds = (difference(symbol) << shift) as u16;
            let keep = (0xFFFFu32 << shift) as u16;
            match (named, slot) {
                (Some(named), _) => {
                    let kept = named % REACH;
                    fields[field] = (self.fields[kept][field] & keep).wrapping_add(adds);
                    from[field] = self.from[kept][field];
                    if traced && from[field] != 0 {
                        let named = Self::trace_at(named, field);
                        for candidate in 0..CANDIDATES {
                            self.trace[to + candidate] =
                                (self.trace[named + candidate] & keep).wrapping_add(adds);
                        }
                    }
                }
                (None, Some(slot)) => {
                    fields[field] = (self.presets.ends[slot][field] & keep).wrapping_add(adds);
                    from[field] = (1 + FIELDS * slot + field) as u16;
                    if traced {
                        for candidate in 0..CANDIDATES {
                            let tried = self.presets.candidate(slot, field, candidate);
                            self.trace[to + candidate] = (tried & keep).wrapping_add(adds);
                        }
                    }
                }
                (None, None) => fields[field] = (HALF & keep).wrapping_add(adds),
            }
        }
        self.fields[block % REACH] = fields;
        self.from[block % REACH] = from;
    }

    fn step_indices(&mut self, block: usize, shape: &Shape, back: usize) {
        let depth = shape.index_depth();
        let shift = 4 - depth;
        let keep = (0xFu8 << shift) & 0xF;
        let traced = !self.first_trace.is_empty();
        let named = (back > 0 && back <= block).then(|| block - back);
        let slot = (back > block).then(|| 256 + block - back);
        let first_from = match (named, slot) {
            (Some(named), _) => self.first_from[named % REACH],
            (None, Some(slot)) => slot as u8,
            (None, None) => 0,
        };
        let mut first = [0u8; 16];
        for (pixel, index) in first.iter_mut().enumerate() {
            let (row, half, odd) = place(pixel);
            let symbol = self.lanes[INDEX_LANES[row]][2 * block + half];
            let field = (0..depth).fold(0, |all, bit| {
                all | (((symbol >> (2 * bit + odd)) & 1) << bit)
            });
            let adds = ((difference(field) << shift) & 0xF) as u8;
            let to = (block % REACH) * 16 + pixel;
            *index = match (named, slot) {
                (Some(named), _) => {
                    if traced && first_from != 0 {
                        let named = (named % REACH) * 16 + pixel;
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
                (None, None) => (0x8 + adds) & 0xF,
            };
        }
        let at = block % REACH;
        self.first[at] = first;
        self.first_from[at] = first_from;
        self.depths[at] = depth as u8;
    }

    /// Block `block`'s fields, each left-aligned in 16 bits.
    pub(super) fn fields(&self, block: usize) -> &[u16; FIELDS] {
        &self.fields[block % REACH]
    }

    /// The preset field that field `field` of block `block` hangs on: 0 for none, else 1 + 12 * slot + field.
    pub(super) fn from(&self, block: usize, field: usize) -> u16 {
        self.from[block % REACH][field]
    }

    /// The top bits a fit has settled of what field `field` of block `block` hangs on: 16 for a field that hangs on no preset.
    pub(super) fn settled(&self, block: usize, field: usize) -> u8 {
        match usize::from(self.from(block, field)).checked_sub(1) {
            Some(at) => self.presets.ends_bits[at / FIELDS][at % FIELDS],
            None => 16,
        }
    }

    /// Whether a fit gave a value to everything the fields a block of `shape` draws with hang on.
    pub(super) fn ends_known(&self, block: usize, shape: &Shape) -> bool {
        (0..6 * shape.regions).all(|field| self.settled(block, field) > 0)
    }

    /// Whether block `block`'s indices are free of the preset or hang on a known slot.
    pub(super) fn indices_known(&self, block: usize) -> bool {
        let slot = self.index_slot(block);
        slot == 0 || self.presets.first_known[usize::from(slot)]
    }

    /// The preset slot block `block`'s indices hang on; 0 for none.
    pub(super) fn index_slot(&self, block: usize) -> u8 {
        self.first_from[block % REACH]
    }

    /// Field `field` of block `block` for each value a fit tries for the preset field it hangs on; only for the block last stepped, in a traced walk.
    pub(super) fn trace(&self, block: usize, field: usize) -> &[u16] {
        let at = Self::trace_at(block, field);
        &self.trace[at..at + CANDIDATES]
    }

    /// Index `pixel` of block `block`, left-aligned in 4 bits, for each value its preset index could take, the 4-bit digits of a word from the lowest; as [`Self::trace`].
    pub(super) fn first_trace(&self, block: usize, pixel: usize) -> u64 {
        self.first_trace[(block % REACH) * 16 + pixel]
    }

    /// Block `block`'s 16 indices.
    pub(super) fn indices(&self, block: usize) -> [u8; 16] {
        let depth = u32::from(self.depths[block % REACH]);
        self.first[block % REACH].map(|index| index >> (4 - depth))
    }
}

/// The 16 pixels a block of `shape` draws, each as red, green and blue in the 16 bits the format interpolates.
pub(super) fn draw(
    shape: &Shape,
    fields: &[u16; FIELDS],
    partition: usize,
    indices: &[u8; 16],
) -> [[u32; 3]; 16] {
    let ends: [[u32; 3]; 4] = std::array::from_fn(|end| {
        std::array::from_fn(|channel| {
            if end < 2 * shape.regions {
                unquantised(end_value(shape, fields, end, channel), shape.base)
            } else {
                0
            }
        })
    });
    let weights = weights(shape.index_depth());
    std::array::from_fn(|pixel| {
        let region = subset_of(shape.regions, partition, pixel);
        let weight = weights[usize::from(indices[pixel])];
        std::array::from_fn(|channel| {
            mix(
                ends[2 * region][channel],
                ends[2 * region + 1][channel],
                weight,
            )
        })
    })
}

/// A block of `shape` as the 16 bytes of a BC6H block.
pub(super) fn write(
    shape: &Shape,
    fields: &[u16; FIELDS],
    partition: usize,
    indices: &[u8; 16],
) -> [u8; 16] {
    let (mut word, mut at) = (0u128, 0u32);
    let mut put = |value: u32, bits: u32| {
        word |= u128::from(value & ((1 << bits) - 1)) << at;
        at += bits;
    };
    put(shape.header.0, shape.header.1);
    for &(field, bit) in &shape.layout {
        let value = match field {
            0..3 => base_value(u32::from(fields[field]) >> (16 - shape.base), shape.base),
            FIELDS => partition as u32,
            field => u32::from(fields[field]) >> (16 - shape.depth(field)),
        };
        put(value >> bit, 1);
    }
    // An anchor's top bit is not stored.
    let anchors = anchors(shape.regions, partition);
    let depth = shape.index_depth();
    for (pixel, &index) in indices.iter().enumerate() {
        put(
            u32::from(index),
            depth - u32::from(anchors.contains(&pixel)),
        );
    }
    word.to_le_bytes()
}

/// The BC6H blocks that `lanes` rebuild through `presets`, and how many hang on a preset value that is not known.
///
/// A value that is not known is taken from the nearest slot that knows it, and its block counted.
pub(super) fn blocks(lanes: &[Vec<u16>], presets: &Presets) -> Result<Rebuilt, BcPackError> {
    let presets = &presets.stood_in();
    let mut walk = Walk::new(lanes, presets, false)?;
    let mut bytes = Vec::with_capacity(16 * walk.blocks);
    let mut unknown = 0;
    for block in 0..walk.blocks {
        let (shape, partition) = walk.step(block)?;
        let known = walk.ends_known(block, shape) && walk.indices_known(block);
        unknown += usize::from(!known);
        bytes.extend(write(
            shape,
            walk.fields(block),
            partition,
            &walk.indices(block),
        ));
    }
    Ok(Rebuilt { bytes, unknown })
}

#[cfg(test)]
#[path = "bc6h_tests.rs"]
mod tests;
