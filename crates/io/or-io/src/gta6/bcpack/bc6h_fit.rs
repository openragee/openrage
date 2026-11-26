//! BC6H's two presets, fitted against the plain levels beside coded ones.
//!
//! A preset field has 16 bits, too many to try at once, so a pass tries 8 of them: the top 8
//! first, then the 8 below. A pass votes each index slot from the blocks whose ends are known, as
//! the index nearest each pixel's target, and each preset field from the blocks whose indices are
//! known, as the least squares of a region's two ends over its pixels. An end is its base plus a
//! difference, so a field is voted only once the other is settled or free of the presets.
//!
//! An encoder's difference never takes an end out of its base's range, which needs no pixel to
//! check: every block of every coded level counts, against each value tried for a field, the
//! ends that value sends out of range, each of which costs that value what a block that fits
//! nothing does.
//!
//! A level with no plain level beside it is voted last, for what is still not known: against the
//! coded levels beside it as the fit draws them by then, and against the pixels across each
//! block's edges in its own level.
//!
//! Everything is held in the 16 bits the format interpolates, which run with the logarithm of
//! the light: 2,114 of them to a stop.

use super::super::bc6h::{self, Presets, Shape, Walk, CANDIDATES, FIELDS};
use super::super::bptc::{subset_of, weights};
use super::{
    bilinear_up, box_down, edges, nearest_up, LevelData, Moved, Pixels, Summary, TextureIn,
    MIN_VOTES,
};

/// What a vote counts a difference past it as: a block that fits nothing does not outvote the rest.
const CAP: u32 = 6144;
/// The span of a region's ends, summed over its channels, under which its indices say nothing.
const FLAT: u32 = 2048;
/// How far, in stops, a texture's plain levels may lie from one another averaged down before none
/// of them is taken as a target.
const AGREE: f64 = 0.4;
/// The 16-bit steps of one stop.
const STOP: f64 = 65536.0 / 31.0;
/// A light under 2^-10 is drawn as that: under it a stop is no difference anyone sees.
const FLOOR: f32 = 5120.0 * 64.0 / 31.0;

/// One pass's votes.
pub(super) struct Votes {
    /// For each slot, pixel and value of an index, the blocks it suits.
    first: Vec<u32>,
    /// For each preset field and each value tried for it, how far that leaves the blocks hanging on it, and how many voted.
    ends: Vec<u32>,
    ends_count: Vec<u32>,
    /// For each preset field and each value tried for it, the ends that value sends out of their base's range.
    wraps: Vec<u32>,
}

impl Votes {
    pub(super) fn empty() -> Self {
        Self {
            first: vec![0; 256 * 16 * 16],
            ends: vec![0; 256 * FIELDS * CANDIDATES],
            ends_count: vec![0; 256 * FIELDS],
            wraps: vec![0; 256 * FIELDS * CANDIDATES],
        }
    }

    /// Adds `other`'s votes: every tally is a sum of whole numbers, so the order votes are added in changes nothing.
    pub(super) fn add(&mut self, other: &Self) {
        let sum = |into: &mut [u32], more: &[u32]| {
            for (into, more) in into.iter_mut().zip(more) {
                *into = into.saturating_add(*more);
            }
        };
        sum(&mut self.first, &other.first);
        sum(&mut self.ends, &other.ends);
        sum(&mut self.ends_count, &other.ends_count);
        sum(&mut self.wraps, &other.wraps);
    }
}

/// A light as the 16 bits the format interpolates to draw it; 0 for none or less.
fn interpolated(light: f32) -> f32 {
    if light.is_nan() || light <= 0.0 {
        return 0.0;
    }
    let bits = light.to_bits();
    let exponent = (bits >> 23) as i32 - 127;
    // A half's bits: 1,024 to each power of two, the smallest powers sharing one scale.
    let half = if exponent < -14 {
        light * 16_777_216.0
    } else if exponent > 15 {
        31_743.0
    } else {
        ((exponent + 15) * 1024) as f32 + (bits & 0x7F_FFFF) as f32 / 8192.0
    };
    (half * 64.0 / 31.0).min(65535.0)
}

/// The light that [`interpolated`] gives `value` for.
fn light(value: f32) -> f32 {
    let half = value * 31.0 / 64.0;
    let exponent = (half / 1024.0) as u32;
    let fraction = half - 1024.0 * exponent as f32;
    if exponent == 0 {
        fraction / 16_777_216.0
    } else {
        f32::from_bits((exponent + 112) << 23 | (fraction * 8192.0) as u32)
    }
}

/// A plain level's blocks as pixels, each channel a light.
fn plain(blocks: &[u8], width: usize, height: usize) -> Option<Pixels> {
    if width == 0 || blocks.len() != 16 * width * height {
        return None;
    }
    let (w, h) = (4 * width, 4 * height);
    let mut channels = vec![vec![0f32; w * h]; 3];
    let mut rgb = [0f32; 48];
    let (cells, _) = blocks.as_chunks::<16>();
    for (i, block) in cells.iter().enumerate() {
        bcdec_rs::bc6h_float(block, &mut rgb, 12, false);
        let origin = 4 * (i / width) * w + 4 * (i % width);
        for (pixel, colour) in rgb.as_chunks::<3>().0.iter().enumerate() {
            let at = origin + (pixel / 4) * w + pixel % 4;
            for (channel, &value) in channels.iter_mut().zip(colour) {
                channel[at] = value;
            }
        }
    }
    Some(Pixels {
        width: w,
        height: h,
        channels,
    })
}

/// Whether the plain levels of a texture are one another averaged down, as far as any two of
/// them lie one above the other.
///
/// A map filtered anew for each level is not, and then no plain level says what a coded one holds.
fn agree(plains: &[Option<Pixels>]) -> bool {
    let (mut off, mut count) = (0f64, 0u64);
    for pair in plains.windows(2) {
        let [Some(above), Some(below)] = pair else {
            continue;
        };
        if above.width != 2 * below.width || above.height != 2 * below.height {
            continue;
        }
        let (width, height) = (below.width, below.height);
        for (above, below) in above.channels.iter().zip(&below.channels) {
            let down = box_down(above, width, height);
            for (&down, &below) in down.iter().zip(below) {
                let apart = interpolated(down).max(FLOOR) - interpolated(below).max(FLOOR);
                off += f64::from(apart.abs());
                count += 1;
            }
        }
    }
    count == 0 || off / STOP <= AGREE * count as f64
}

/// What a coded level is held against: its lanes, where each block lies, its width in blocks, and a target for each pixel, NaN where none is known.
struct Coded<'a> {
    lanes: &'a [Vec<u16>],
    at: &'a [u32],
    width: usize,
    target: Option<Vec<Vec<f32>>>,
    /// Whether the target is the level below, which has one pixel for each 2 by 2.
    quads: bool,
    /// Whether a plain level beside it gave the target.
    plain: bool,
}

/// A coded level through `presets` as pixels in the 16 bits the format interpolates, NaN in a
/// block that hangs on something not known, and whether a block of it hangs on something not settled.
fn drawn(
    lanes: &[Vec<u16>],
    at: &[u32],
    (width, height): (usize, usize),
    presets: &Presets,
) -> Option<(Pixels, bool)> {
    let mut walk = Walk::new(lanes, presets, false).ok()?;
    let (w, h) = (4 * width, 4 * height);
    let mut channels = vec![vec![f32::NAN; w * h]; 3];
    let mut open = false;
    for (block, &place) in at.iter().enumerate().take(walk.blocks) {
        let (shape, partition) = walk.step(block).ok()?;
        open |= walk.open(block, shape);
        let known = walk.ends_known(block, shape) && walk.indices_known(block);
        let place = place as usize;
        if place >= width * height || !known {
            continue;
        }
        let pixels = bc6h::draw(shape, walk.fields(block), partition, &walk.indices(block));
        let origin = 4 * (place / width) * w + 4 * (place % width);
        for (pixel, colour) in pixels.iter().enumerate() {
            for (plane, &value) in channels.iter_mut().zip(colour) {
                plane[origin + (pixel / 4) * w + pixel % 4] = value as f32;
            }
        }
    }
    let pixels = Pixels {
        width: w,
        height: h,
        channels,
    };
    Some((pixels, open))
}

/// `source`, twice `width` by twice `height` in the 16 bits the format interpolates, with the lights of each 2 by 2 pixels averaged; NaN where one is.
fn lights_down(source: &[f32], width: usize, height: usize) -> Vec<f32> {
    let wide = 2 * width;
    (0..width * height)
        .map(|i| {
            let at = 2 * (i / width) * wide + 2 * (i % width);
            let sum: f32 = [at, at + 1, at + wide, at + wide + 1]
                .iter()
                .map(|&at| light(source[at]))
                .sum();
            if sum.is_nan() {
                f32::NAN
            } else {
                interpolated(sum / 4.0)
            }
        })
        .collect()
}

/// The pixels a coded level with no plain level beside it is held against: the pixels across
/// each block's edges in level `i` itself and, where `levels`, those of the coded level below as
/// `draws` has it, smoothed up, and of the one above, averaged down; a pixel takes the last that knows it.
fn beside(draws: &[Option<Pixels>], i: usize, levels: bool) -> Option<Vec<Vec<f32>>> {
    let own = draws[i].as_ref()?;
    let (w, h) = (own.width, own.height);
    let above = (i.checked_sub(1))
        .and_then(|above| draws[above].as_ref())
        .filter(|above| levels && above.width == 2 * w && above.height == 2 * h);
    let below = (draws.get(i + 1))
        .and_then(Option::as_ref)
        .filter(|below| levels && 2 * below.width == w && 2 * below.height == h);
    let planes = (0..3).map(|c| {
        let mut plane = edges(&own.channels[c], w, h);
        let others = [
            below.map(|below| bilinear_up(&below.channels[c], w, h)),
            above.map(|above| lights_down(&above.channels[c], w, h)),
        ];
        for other in others.into_iter().flatten() {
            for (pixel, other) in plane.iter_mut().zip(other) {
                if !other.is_nan() {
                    *pixel = other;
                }
            }
        }
        plane
    });
    Some(planes.collect())
}

/// The coded levels of `texture`: a level's target is the plain level above averaged down, else
/// the one below, smoothed up for a vote and each pixel repeated 2 by 2 where `repeated`, for a score.
///
/// With `through`, the presets as they stand, a level with neither that has a block hanging on
/// something not settled is held against what the fit draws beside it ([`beside`]).
fn coded<'a>(texture: &'a TextureIn, repeated: bool, through: Option<&Presets>) -> Vec<Coded<'a>> {
    let plains: Vec<Option<Pixels>> = (texture.levels.iter())
        .map(|level| match &level.data {
            LevelData::Plain(blocks) => plain(blocks, level.width, level.height),
            _ => None,
        })
        .collect();
    let scaled = |lights: Vec<f32>| -> Vec<f32> { lights.into_iter().map(interpolated).collect() };
    // ponytail: a signed level is drawn as an unsigned one, which is the same picture where no
    // value is negative; none has a plain level beside it, so only its edges are read. A signed
    // texture with plain levels needs the signed draw before they can be its targets.
    let targets = !texture.signed && agree(&plains);
    let mut out = Vec::new();
    for (i, level) in texture.levels.iter().enumerate() {
        let LevelData::Coded { lanes, at } = &level.data else {
            continue;
        };
        if lanes.len() != 16 || 2 * at.len() != lanes[0].len() {
            continue;
        }
        let (w, h) = (4 * level.width, 4 * level.height);
        let above: Option<Vec<Vec<f32>>> = (i.checked_sub(1))
            .and_then(|above| plains[above].as_ref())
            .filter(|above| above.width == 2 * w && above.height == 2 * h)
            .map(|above| (above.channels.iter().map(|c| scaled(box_down(c, w, h)))).collect());
        let up = if repeated { nearest_up } else { bilinear_up };
        let below = || {
            (plains.get(i + 1))
                .and_then(Option::as_ref)
                .filter(|below| 2 * below.width == w && 2 * below.height == h)
                .map(|below| (below.channels.iter().map(|c| scaled(up(c, w, h)))).collect())
        };
        let quads = above.is_none();
        let target = above.or_else(below).filter(|_| targets);
        let coded = Coded {
            lanes,
            at,
            width: level.width,
            quads,
            plain: target.is_some(),
            target,
        };
        out.push((i, coded));
    }
    if let Some(presets) = through {
        let (mut draws, mut opens) = (Vec::new(), Vec::new());
        for level in &texture.levels {
            let drawn = match &level.data {
                LevelData::Coded { lanes, at } if lanes.len() == 16 => {
                    drawn(lanes, at, (level.width, level.height), presets)
                }
                _ => None,
            };
            opens.push(drawn.as_ref().is_some_and(|drawn| drawn.1));
            draws.push(drawn.map(|drawn| drawn.0));
        }
        // A level with nothing left to settle votes nothing; the levels of a texture whose plain
        // levels disagree say nothing of one another.
        for (i, level) in &mut out {
            if !opens[*i] {
                level.target = None;
            } else if !level.plain {
                level.target = beside(&draws, *i, targets);
            }
        }
    }
    out.into_iter().map(|(_, level)| level).collect()
}

/// The 16 pixels of a block, each as red, green and blue.
type BlockPixels = [[f32; 3]; 16];

/// The target of the block that lies at `place` of a level `width` blocks wide; `None` for a block outside the level.
fn wanted(target: &[Vec<f32>], width: usize, place: u32) -> Option<BlockPixels> {
    let place = usize::try_from(place).ok().filter(|_| place != u32::MAX)?;
    let origin = 16 * (place / width) * width + 4 * (place % width);
    let mut out = [[0f32; 3]; 16];
    for (pixel, colour) in out.iter_mut().enumerate() {
        let at = origin + (pixel / 4) * 4 * width + pixel % 4;
        for (value, channel) in colour.iter_mut().zip(target) {
            *value = *channel.get(at)?;
        }
    }
    Some(out)
}

/// Votes the preset indices block `block` hangs on, its ends being known.
fn vote_indices(
    walk: &Walk<'_>,
    block: usize,
    (shape, partition): (&Shape, usize),
    wanted: &BlockPixels,
    votes: &mut Votes,
) {
    let slot = usize::from(walk.index_slot(block));
    let depth = shape.index_depth();
    let weights = weights(depth);
    let fields = walk.fields(block);
    // For each region whose ends are apart, what each index draws.
    let draws: [Option<[[f32; 3]; 16]>; 2] = std::array::from_fn(|region| {
        if region >= shape.regions {
            return None;
        }
        let ends: [[u32; 3]; 2] = std::array::from_fn(|end| {
            std::array::from_fn(|channel| {
                let value = bc6h::end_value(shape, fields, 2 * region + end, channel);
                bc6h::unquantised(value, shape.base)
            })
        });
        let spread: u32 = (0..3).map(|c| ends[0][c].abs_diff(ends[1][c])).sum();
        (spread >= FLAT).then(|| {
            std::array::from_fn(|index| {
                let weight = weights.get(index).copied().unwrap_or(0);
                std::array::from_fn(|c| bc6h::mix(ends[0][c], ends[1][c], weight) as f32)
            })
        })
    });
    for (pixel, target) in wanted.iter().enumerate() {
        let Some(draws) = &draws[subset_of(shape.regions, partition, pixel)] else {
            continue;
        };
        if target[0].is_nan() {
            continue;
        }
        let off = |index: usize| -> f32 {
            (draws[index].iter().zip(target))
                .map(|(drawn, wanted)| (drawn - wanted).abs())
                .sum()
        };
        let best = (1..weights.len()).fold(
            0,
            |best, index| {
                if off(index) < off(best) {
                    index
                } else {
                    best
                }
            },
        );
        let trace = walk.first_trace(block, pixel);
        let at = 16 * (16 * slot + pixel);
        for (x, vote) in votes.first[at..at + 16].iter_mut().enumerate() {
            let through = ((trace >> (4 * x)) & 0xF) as usize;
            *vote += u32::from(through >> (4 - depth) == best);
        }
    }
}

/// Votes the preset fields block `block` hangs on, its indices being known.
fn vote_ends(
    walk: &Walk<'_>,
    block: usize,
    (shape, partition): (&Shape, usize),
    wanted: &BlockPixels,
    votes: &mut Votes,
) {
    let indices = walk.indices(block);
    let weights = weights(shape.index_depth());
    let standing = walk.fields(block);
    for region in 0..shape.regions {
        for channel in 0..3 {
            // The two ends that leave the region's pixels nearest their targets, by least squares.
            let (mut aa, mut ab, mut bb, mut at, mut bt, mut count) =
                (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
            for (pixel, target) in wanted.iter().enumerate() {
                if subset_of(shape.regions, partition, pixel) != region || target[0].is_nan() {
                    continue;
                }
                let b = f32::from(weights[usize::from(indices[pixel])]) / 64.0;
                let a = 1.0 - b;
                aa += a * a;
                ab += a * b;
                bb += b * b;
                at += a * target[channel];
                bt += b * target[channel];
                count += 1.0;
            }
            let det = aa * bb - ab * ab;
            let found = if det > 0.02 * count * count {
                [
                    Some((bb * at - ab * bt) / det),
                    Some((aa * bt - ab * at) / det),
                ]
            } else if bb < 0.05 * aa {
                [Some(at / aa), None]
            } else if aa < 0.05 * bb {
                [None, Some(bt / bb)]
            } else {
                [None, None]
            };
            for (side, found) in found.into_iter().enumerate() {
                let Some(found) = found else {
                    continue;
                };
                let found = found.clamp(0.0, 65535.0) as u32;
                // An end is its base and, past the base, its own field: it votes the one of the
                // two that a fit has not settled, the other being settled or hanging on the same.
                let end = 2 * region + side;
                let own = 3 * end + channel;
                let open = |field: usize| walk.settled(block, field) < 16;
                let voted = match (open(channel), end > 0 && open(own)) {
                    (true, false) => channel,
                    (false, true) => own,
                    (true, true) if walk.from(block, channel) == walk.from(block, own) => own,
                    _ => continue,
                };
                let hangs = walk.from(block, voted);
                let Some(at) = usize::from(hangs).checked_sub(1) else {
                    continue;
                };
                votes.ends_count[at] += 1;
                let costs = &mut votes.ends[CANDIDATES * at..CANDIDATES * (at + 1)];
                let mut tried = *standing;
                for (candidate, cost) in costs.iter_mut().enumerate() {
                    for field in [channel, own] {
                        if walk.from(block, field) == hangs {
                            tried[field] = walk.trace(block, field)[candidate];
                        }
                    }
                    let value = bc6h::end_value(shape, &tried, end, channel);
                    let off = bc6h::unquantised(value, shape.base).abs_diff(found);
                    *cost = cost.saturating_add(off.min(CAP) >> 4);
                }
            }
        }
    }
}

/// Counts, for each value tried for a preset field that block `block` hangs on, the ends that
/// value sends out of their base's range, the other of a base and its difference being settled.
///
/// In the signed format a base's top bit is its sign, and the range is the signed one.
fn vote_range(walk: &Walk<'_>, block: usize, shape: &Shape, signed: bool, votes: &mut Votes) {
    if !shape.transformed {
        return;
    }
    let standing = walk.fields(block);
    let top = 1 << shape.base;
    let (low, high) = if signed {
        (-(top / 2), top / 2)
    } else {
        (0, top)
    };
    let base_of = |field: u16| match bc6h::base_of(shape, field) {
        base if base >= high => base - top,
        base => base,
    };
    for end in 1..2 * shape.regions {
        for channel in 0..3 {
            let own = 3 * end + channel;
            let open = |field: usize| walk.settled(block, field) < 16;
            let (hangs, both) = match (open(channel), open(own)) {
                (true, false) => (walk.from(block, channel), false),
                (false, true) => (walk.from(block, own), false),
                (true, true) if walk.from(block, channel) == walk.from(block, own) => {
                    (walk.from(block, own), true)
                }
                _ => continue,
            };
            let Some(at) = usize::from(hangs).checked_sub(1) else {
                continue;
            };
            // A value tried for a field's top 8 bits stands for the 256 that share them, which
            // reach past it by what the bits below add at this block's depth.
            let slack = |field: usize| match walk.settled(block, field) {
                0 => 0xFF >> (16 - shape.depth(field)),
                _ => 0,
            };
            let base = base_of(standing[channel]);
            let other = bc6h::difference_of(shape, standing[own], channel);
            let wraps = &mut votes.wraps[CANDIDATES * at..CANDIDATES * (at + 1)];
            let out = |sum: i32, slack: i32| u32::from(sum + slack < low || sum >= high);
            if both {
                let slack = slack(channel) + slack(own);
                let (bases, others) = (walk.trace(block, channel), walk.trace(block, own));
                for ((wraps, &tried), &other) in wraps.iter_mut().zip(bases).zip(others) {
                    let other = bc6h::difference_of(shape, other, channel);
                    *wraps += out(base_of(tried) + other, slack);
                }
            } else if open(own) {
                let slack = slack(own);
                for (wraps, &tried) in wraps.iter_mut().zip(walk.trace(block, own)) {
                    *wraps += out(base + bc6h::difference_of(shape, tried, channel), slack);
                }
            } else {
                let slack = slack(channel);
                for (wraps, &tried) in wraps.iter_mut().zip(walk.trace(block, channel)) {
                    *wraps += out(base_of(tried) + other, slack);
                }
            }
        }
    }
}

/// Adds `texture`'s votes, read through `presets` as they stand, and says whether a block of it
/// hangs on something not settled.
///
/// A pass that fills (`fill`) reads only the levels with such a block, and holds a level with
/// no plain level beside it against what the fit draws beside it.
pub(super) fn vote(texture: &TextureIn, presets: &Presets, votes: &mut Votes, fill: bool) -> bool {
    let mut open = false;
    for level in coded(texture, false, fill.then_some(presets)) {
        if fill && level.target.is_none() {
            continue;
        }
        let Ok(mut walk) = Walk::new(level.lanes, presets, true) else {
            continue;
        };
        for block in 0..walk.blocks {
            let Ok(of) = walk.step(block) else {
                break;
            };
            open |= walk.open(block, of.0);
            vote_range(&walk, block, of.0, texture.signed, votes);
            let wanted = (level.target.as_ref())
                .and_then(|target| wanted(target, level.width, level.at[block]));
            let Some(wanted) = wanted else {
                continue;
            };
            let settled = (0..6 * of.0.regions).all(|field| walk.settled(block, field) == 16);
            let slot = usize::from(walk.index_slot(block));
            // A pass that fills leaves the indices the plain levels voted as they are.
            if settled && slot != 0 && !(fill && presets.first_known[slot]) {
                vote_indices(&walk, block, of, &wanted, votes);
            }
            if walk.indices_known(block) && !settled {
                vote_ends(&walk, block, of, &wanted, votes);
            }
        }
    }
    open
}

/// Takes into `presets` what `votes` settle: a voted field's next 8 bits, and each voted index.
pub(super) fn solve(votes: &Votes, presets: &mut Presets) {
    for slot in 1..256 {
        let mut known = true;
        for (pixel, index) in presets.first[slot].iter_mut().enumerate() {
            let at = 16 * (16 * slot + pixel);
            let tally = &votes.first[at..at + 16];
            if tally.iter().sum::<u32>() < MIN_VOTES {
                known = false;
                continue;
            }
            let best = (0..16).fold(0, |best, x| if tally[x] > tally[best] { x } else { best });
            *index = best as u8;
        }
        presets.first_known[slot] |= known;
        for field in 0..FIELDS {
            let at = FIELDS * slot + field;
            let settled = presets.ends_bits[slot][field];
            if votes.ends_count[at] < 2 * MIN_VOTES || settled >= 16 {
                continue;
            }
            let costs = &votes.ends[CANDIDATES * at..CANDIDATES * (at + 1)];
            let wraps = &votes.wraps[CANDIDATES * at..CANDIDATES * (at + 1)];
            // An end out of range counts as a block that fits nothing. It counts no more: the
            // other of its base and its difference is itself a fitted value, so a few blocks
            // that only a far value keeps in range say less than the pixels of the rest.
            let cost = |x: usize| costs[x].saturating_add(wraps[x].saturating_mul(CAP >> 4));
            let least =
                (0..CANDIDATES).fold(0, |best, x| if cost(x) < cost(best) { x } else { best });
            presets.ends[slot][field] = presets.candidate(slot, field, least);
            presets.ends_bits[slot][field] = settled + 8;
        }
    }
}

/// Draws `texture`'s coded levels through `presets` and adds what they show to `summary`: its mismatch is in stops.
///
/// A level is held against the level above pixel by pixel, else its lights averaged 2 by 2
/// against the level below.
pub(super) fn score(texture: &TextureIn, presets: &Presets, summary: &mut Summary) {
    let levels = coded(texture, true, None);
    summary.textures += u64::from(!levels.is_empty());
    for level in levels {
        let Ok(mut walk) = Walk::new(level.lanes, presets, false) else {
            continue;
        };
        summary.bodies += 1;
        for block in 0..walk.blocks {
            let Ok((shape, partition)) = walk.step(block) else {
                break;
            };
            summary.blocks += 1;
            let known = walk.ends_known(block, shape) && walk.indices_known(block);
            summary.unknown += u64::from(!known);
            let wanted = (level.target.as_ref())
                .filter(|_| known)
                .and_then(|target| wanted(target, level.width, level.at[block]));
            let Some(wanted) = wanted else {
                continue;
            };
            let drawn = bc6h::draw(shape, walk.fields(block), partition, &walk.indices(block));
            let (off, compared) = off(level.quads, &drawn, &wanted);
            summary.mismatch += off;
            summary.compared += compared;
        }
    }
}

/// What the pixels a block draws are off its target by, in stops, and the values compared:
/// pixel by pixel, or where `quads` its lights averaged 2 by 2 against the level below.
fn off(quads: bool, drawn: &[[u32; 3]; 16], wanted: &BlockPixels) -> (f64, u64) {
    let (mut off, mut compared) = (0f64, 0u64);
    let mut compare = |drawn: f32, target: f32| {
        let apart = drawn.max(FLOOR) - target.max(FLOOR);
        off += f64::from(apart.abs()) / STOP;
        compared += 1;
    };
    for channel in 0..3 {
        if quads {
            for corner in [0, 2, 8, 10] {
                let lights = [0, 1, 4, 5].map(|more| light(drawn[corner + more][channel] as f32));
                let mean = lights.iter().sum::<f32>() / 4.0;
                compare(interpolated(mean), wanted[corner][channel]);
            }
        } else {
            for (drawn, target) in drawn.iter().zip(wanted) {
                compare(drawn[channel] as f32, target[channel]);
            }
        }
    }
    (off, compared)
}

/// Moves each preset value the coded levels of `texture` read a step either way, and adds to
/// `ends`, by slot and field, and to `first`, by slot and pixel, what its blocks are then off
/// the plain level beside them by.
///
/// A field's step is the least one of its preset field that the block answers to, and a preset
/// field several of a block's fields hang on moves them all.
///
/// `numbered` is the number of the chain's first level among those a reader takes.
pub(super) fn probe(
    texture: &TextureIn,
    presets: &Presets,
    numbered: u32,
    (ends, first): (&mut [Moved], &mut [Moved]),
) {
    for (level, numbered) in coded(texture, true, None).into_iter().zip(numbered..) {
        let (Some(target), Ok(mut walk)) = (&level.target, Walk::new(level.lanes, presets, true))
        else {
            continue;
        };
        for block in 0..walk.blocks {
            let Ok((shape, partition)) = walk.step(block) else {
                break;
            };
            let known = walk.ends_known(block, shape) && walk.indices_known(block);
            let Some(wanted) = wanted(target, level.width, level.at[block]).filter(|_| known)
            else {
                continue;
            };
            let (fields, indices) = (*walk.fields(block), walk.indices(block));
            let through = |fields: &[u16; FIELDS], indices: &[u8; 16]| {
                off(
                    level.quads,
                    &bc6h::draw(shape, fields, partition, indices),
                    &wanted,
                )
                .0
            };
            let held = through(&fields, &indices);
            let drawn = 6 * shape.regions;
            for field in 0..drawn {
                let from = walk.from(block, field);
                // Each preset field once, at the first of the block's fields that hang on it.
                if from == 0 || (0..field).any(|before| walk.from(block, before) == from) {
                    continue;
                }
                let hang = || (field..drawn).filter(|&other| walk.from(block, other) == from);
                let lowest = hang()
                    .map(|other| walk.reach(block, other).0)
                    .max()
                    .unwrap_or(0);
                let moved = |up: bool| {
                    let mut fields = fields;
                    for other in hang() {
                        // A step that leaves the field's 16 bits moves nothing.
                        let by = 1u32.checked_shl(lowest + walk.reach(block, other).1);
                        let by = by.unwrap_or(0) as u16;
                        fields[other] = match up {
                            true => fields[other].wrapping_add(by),
                            false => fields[other].wrapping_sub(by),
                        };
                    }
                    through(&fields, &indices)
                };
                ends[usize::from(from) - 1].add(numbered, held, moved(false), moved(true));
            }
            let slot = usize::from(walk.index_slot(block));
            if slot == 0 {
                continue;
            }
            let shift = 4 - shape.index_depth();
            for pixel in 0..16 {
                let trace = walk.first_trace(block, pixel);
                let through_index = |preset: u8| ((trace >> (4 * (preset & 15))) & 15) as u8;
                let at = presets.first[slot][pixel];
                let answers = |step: &u8| {
                    through_index(at.wrapping_add(*step)) != through_index(at)
                        || through_index(at.wrapping_sub(*step)) != through_index(at)
                };
                let Some(step) = [1u8, 2, 4, 8].into_iter().find(answers) else {
                    continue;
                };
                let moved = |preset: u8| {
                    let mut indices = indices;
                    indices[pixel] = through_index(preset) >> shift;
                    through(&fields, &indices)
                };
                let (lower, higher) = (moved(at.wrapping_sub(step)), moved(at.wrapping_add(step)));
                first[16 * slot + pixel].add(numbered, held, lower, higher);
            }
        }
    }
}

/// The preset slots `presets` knows a field of, and those it knows the indices of.
pub(super) fn solved(presets: &Presets) -> (u32, u32) {
    let ends = (presets.ends_bits.iter())
        .filter(|bits| bits.iter().any(|&bits| bits > 0))
        .count();
    let first = presets.first_known.iter().filter(|&&known| known).count();
    (ends as u32, first as u32)
}
