//! BC7's two presets and the partition order of its modes 0 and 2, fitted against the plain
//! levels beside coded ones, after the mode of each rank of lane 15, read from the lanes alone.
//!
//! A census takes, for each rank, the widest symbol its blocks carry in each lane: a mode's
//! fields have the widths the block format gives them, so the widths name the mode.
//!
//! A pass votes each index slot from the blocks whose endpoints are known, as the index nearest
//! each pixel's target, and each endpoint value from the blocks whose indices are known, as the
//! least squares of a subset's two ends over its pixels; a block free of the presets is known
//! from the start, so the first pass has blocks to vote from.
//!
//! A level with no plain level beside it is voted last, for the values still not known: against
//! the coded levels beside it as the fit draws them by then, and against the pixels across each
//! block's edges in its own level.

use super::super::bptc::{self, Presets, Sig, Walk, RANKS, UNPLACED, VALUES, WIDTHS};
use super::{bilinear_up, box_down, edges, LevelData, Pixels, Summary, TextureIn, MIN_VOTES};

/// What a vote counts a difference past it as, in 8-bit steps: a block that fits nothing does not outvote the rest.
const CAP: f32 = 24.0;
/// The span of a subset's ends, summed over its channels, under which its indices say nothing.
const FLAT: i32 = 8;
/// The blocks a rank needs before its widest symbols are taken to show its mode.
const CENSUS_MIN: u64 = 32;

/// For each rank of lane 15: the blocks that carry it, and the widest symbol they carry in each of lanes 2 to 14.
pub(super) struct Census {
    seen: Vec<u64>,
    widest: Vec<[u16; WIDTHS]>,
}

impl Census {
    pub(super) fn empty() -> Self {
        Self {
            seen: vec![0; RANKS],
            widest: vec![[0; WIDTHS]; RANKS],
        }
    }

    /// Adds what `other` counted.
    pub(super) fn add(&mut self, other: &Self) {
        for (seen, more) in self.seen.iter_mut().zip(&other.seen) {
            *seen += more;
        }
        for (widest, more) in self.widest.iter_mut().zip(&other.widest) {
            for (symbol, &more) in widest.iter_mut().zip(more) {
                *symbol = (*symbol).max(more);
            }
        }
    }

    /// Counts the blocks of `texture`'s coded levels.
    pub(super) fn take(&mut self, texture: &TextureIn) {
        for level in &texture.levels {
            let LevelData::Coded { lanes, .. } = &level.data else {
                continue;
            };
            let [_, _, fields @ .., signature] = lanes.as_slice() else {
                continue;
            };
            if fields.len() != WIDTHS / 2 {
                continue;
            }
            for (block, pair) in signature.as_chunks::<2>().0.iter().enumerate() {
                let Some(rank) = bptc::rank(pair[0], pair[1]) else {
                    continue;
                };
                self.seen[rank] += 1;
                let symbols = (fields.iter()).flat_map(|lane| lane.get(2 * block..2 * block + 2));
                for (widest, symbols) in self.widest[rank]
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .zip(symbols)
                {
                    for (widest, &symbol) in widest.iter_mut().zip(symbols) {
                        *widest = (*widest).max(symbol);
                    }
                }
            }
        }
    }

    /// The mode each rank names: the one whose fields hold the rank's widest symbols most tightly.
    ///
    /// A rank with too few blocks to show its widths takes the mode of the nearest rank below
    /// that shows them; before the first such rank and past the last rank seen, a rank names none.
    pub(super) fn modes(&self) -> [u8; RANKS] {
        // Modes 4 and 5 move their wider field with the rotation, which their place among their ranks gives.
        let shapes: Vec<(u8, [u8; WIDTHS])> = (0..8usize)
            .flat_map(|mode| {
                let places = match mode {
                    4 => vec![0, 2, 4, 6],
                    5 => vec![0, 1, 2, 3],
                    _ => vec![0],
                };
                (places.into_iter())
                    .map(move |position| (mode as u8, Sig { mode, position }.widths()))
            })
            .collect();
        let mut modes = [UNPLACED; RANKS];
        for (rank, mode) in modes.iter_mut().enumerate() {
            if self.seen[rank] < CENSUS_MIN {
                continue;
            }
            let bits = self.widest[rank].map(|symbol| (16 - symbol.leading_zeros()) as u8);
            let slack = |widths: &[u8; WIDTHS]| -> Option<u32> {
                (widths.iter().zip(&bits)).try_fold(0, |slack, (&room, &used)| {
                    (used <= room).then(|| slack + u32::from(room - used))
                })
            };
            let tightest = (shapes.iter())
                .filter_map(|(mode, widths)| Some((slack(widths)?, *mode)))
                .min();
            if let Some((_, tightest)) = tightest {
                *mode = tightest;
            }
        }
        let last = (0..RANKS).rev().find(|&rank| self.seen[rank] > 0);
        let shown = modes;
        for rank in 0..=last.unwrap_or(0) {
            if shown[rank] == UNPLACED {
                let below = shown[..rank].iter().rev().find(|&&mode| mode != UNPLACED);
                modes[rank] = below.copied().unwrap_or(UNPLACED);
            }
        }
        modes
    }
}

/// The costs kept for one preset endpoint value: 256 where no bit is cut, 128 for one, down to 1.
const ENDS_TALLY: usize = 512;

/// Where the costs of the preset values that differ above their `cut` low bits start among a value's.
fn ends_tally_at(cut: u32) -> usize {
    ENDS_TALLY - (ENDS_TALLY >> cut)
}

/// The 16 pixels of a block, each as red, green, blue and alpha.
type BlockPixels = [[f32; 4]; 16];

/// One pass's votes.
pub(super) struct Votes {
    /// For each slot, pixel and value of a first index, the blocks it suits; the same for the second.
    first: Vec<u32>,
    second: Vec<u32>,
    /// For each preset endpoint value, how far each value it could take leaves the blocks hanging
    /// on it, in half steps, and how many voted. A block's value does not depend on the low bits its
    /// chain cut from the preset value, so a cost is kept once for the values that differ only
    /// there: [`ENDS_TALLY`] costs for a preset value, those of each number of bits cut in turn.
    ends: Vec<u32>,
    ends_count: Vec<u32>,
    /// For each position of modes 0 and 2 and each partition, how far that leaves its blocks, and how many voted.
    orders: [Vec<u64>; 2],
    orders_count: [Vec<u32>; 2],
}

impl Votes {
    pub(super) fn empty() -> Self {
        Self {
            first: vec![0; 256 * 16 * 16],
            second: vec![0; 256 * 16 * 4],
            ends: vec![0; 256 * VALUES * ENDS_TALLY],
            ends_count: vec![0; 256 * VALUES],
            orders: [vec![0; 16 * 16], vec![0; 64 * 64]],
            orders_count: [vec![0; 16], vec![0; 64]],
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
        sum(&mut self.second, &other.second);
        sum(&mut self.ends, &other.ends);
        sum(&mut self.ends_count, &other.ends_count);
        for which in 0..2 {
            sum(&mut self.orders_count[which], &other.orders_count[which]);
            for (into, more) in self.orders[which].iter_mut().zip(&other.orders[which]) {
                *into += more;
            }
        }
    }
}

/// A plain level's blocks as pixels.
fn plain(blocks: &[u8], width: usize, height: usize) -> Option<Pixels> {
    if width == 0 || blocks.len() != 16 * width * height {
        return None;
    }
    let (w, h) = (4 * width, 4 * height);
    let mut channels = vec![vec![0f32; w * h]; 4];
    let mut rgba = [0u8; 64];
    let (cells, _) = blocks.as_chunks::<16>();
    for (i, block) in cells.iter().enumerate() {
        bcdec_rs::bc7(block, &mut rgba, 16);
        let origin = 4 * (i / width) * w + 4 * (i % width);
        for (pixel, colour) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let at = origin + (pixel / 4) * w + pixel % 4;
            for (channel, &value) in channels.iter_mut().zip(colour) {
                channel[at] = f32::from(value);
            }
        }
    }
    Some(Pixels {
        width: w,
        height: h,
        channels,
    })
}

/// What a coded level is held against: its lanes, where each block lies, its width in blocks, and a target for each pixel if a plain level is beside it.
struct Coded<'a> {
    lanes: &'a [Vec<u16>],
    at: &'a [u32],
    width: usize,
    target: Option<Vec<Vec<f32>>>,
    /// Whether a plain level beside it gave the target.
    plain: bool,
    /// For a level with no plain level beside it: whether a block of it hangs on something the plain levels left unknown.
    open: bool,
}

/// Whether the block last stepped hangs on an endpoint value, and on an index slot, that `plain` does not know.
fn open(walk: &Walk<'_>, block: usize, sig: Sig, plain: &Presets) -> (bool, bool) {
    let ends = (0..VALUES).any(|value| {
        let used = sig.depth(value / 8, (value / 2) % 4) != 0;
        let from = usize::from(walk.end_from(block, value));
        let at = from.saturating_sub(1);
        used && from != 0 && !plain.ends_known[at / VALUES][at % VALUES]
    });
    let (first, second) = walk.index_slots(block);
    let indices = (first != 0 && !plain.first_known[usize::from(first)])
        || (second != 0 && !plain.second_known[usize::from(second)]);
    (ends, indices)
}

/// A coded level through `presets` as pixels, NaN in a block that hangs on a value that is not
/// known, and whether a block of it hangs on something `plain` does not know.
fn drawn(
    lanes: &[Vec<u16>],
    at: &[u32],
    (width, height): (usize, usize),
    (presets, plain): (&Presets, &Presets),
) -> Option<(Pixels, bool)> {
    let mut walk = Walk::new(lanes, presets, false).ok()?;
    let (w, h) = (4 * width, 4 * height);
    let mut channels = vec![vec![f32::NAN; w * h]; 4];
    let mut any_open = false;
    for (block, &place) in at.iter().enumerate().take(walk.blocks) {
        let sig = walk.step(block).ok()?;
        let place = place as usize;
        any_open |= open(&walk, block, sig, plain) != (false, false);
        let known = walk.ends_known(block, sig) && walk.indices_known(block);
        if place >= width * height || !known {
            continue;
        }
        let of = Block::of(&walk, block, sig);
        let origin = 4 * (place / width) * w + 4 * (place % width);
        for pixel in 0..16 {
            let subset = bptc::subset_of(sig.subsets(), of.partition, pixel);
            for (channel, plane) in channels.iter_mut().enumerate() {
                plane[origin + (pixel / 4) * w + pixel % 4] = (of.spans[subset][channel])
                    .map_or(255.0, |span| bptc::mix(span, of.weights[pixel][channel]));
            }
        }
    }
    let pixels = Pixels {
        width: w,
        height: h,
        channels,
    };
    Some((pixels, any_open))
}

/// The pixels a coded level with no plain level beside it is held against: those of the coded
/// level above as `draws` has it, averaged down, else of the one below, smoothed up, else the
/// pixels across each block's edges in level `i` itself; a pixel takes the first that knows it.
fn beside(draws: &[Option<Pixels>], i: usize) -> Option<Vec<Vec<f32>>> {
    let own = draws[i].as_ref()?;
    let (w, h) = (own.width, own.height);
    let above = (i.checked_sub(1))
        .and_then(|above| draws[above].as_ref())
        .filter(|above| above.width == 2 * w && above.height == 2 * h);
    let below = (draws.get(i + 1))
        .and_then(Option::as_ref)
        .filter(|below| 2 * below.width == w && 2 * below.height == h);
    let planes = (0..4).map(|c| {
        let mut plane = edges(&own.channels[c], w, h);
        let others = [
            below.map(|below| bilinear_up(&below.channels[c], w, h)),
            above.map(|above| box_down(&above.channels[c], w, h)),
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

/// The coded levels of `texture`: a level's target is the plain level above averaged down, else the one below smoothed up.
///
/// With `through`, the presets as they stand and as the plain levels left them, a level with
/// neither is held against the coded levels beside it as the first draw them ([`beside`]).
fn coded<'a>(texture: &'a TextureIn, through: Option<(&Presets, &Presets)>) -> Vec<Coded<'a>> {
    // A pass that takes only the levels with no plain level beside them needs no plain level's pixels.
    let plains: Vec<Option<Pixels>> = (texture.levels.iter())
        .map(|level| match &level.data {
            LevelData::Plain(blocks) if through.is_none() => {
                plain(blocks, level.width, level.height)
            }
            _ => None,
        })
        .collect();
    // Whether the level `to` levels from level `i` is a plain one of `scale` halves its size.
    let plain_beside = |i: usize, to: isize, scale: usize| {
        let (Some(level), Some(other)) = (
            texture.levels.get(i),
            i.checked_add_signed(to)
                .and_then(|at| texture.levels.get(at)),
        ) else {
            return false;
        };
        let whole = 16 * other.width * other.height;
        matches!(&other.data, LevelData::Plain(blocks) if blocks.len() == whole && other.width > 0)
            && 2 * other.width == scale * level.width
            && 2 * other.height == scale * level.height
    };
    let mut out = Vec::new();
    for (i, level) in texture.levels.iter().enumerate() {
        let LevelData::Coded { lanes, at } = &level.data else {
            continue;
        };
        if lanes.len() != 16 || 2 * at.len() != lanes[0].len() {
            continue;
        }
        let (w, h) = (4 * level.width, 4 * level.height);
        let above = (i.checked_sub(1))
            .and_then(|above| plains[above].as_ref())
            .filter(|above| above.width == 2 * w && above.height == 2 * h)
            .map(|above| (above.channels.iter().map(|c| box_down(c, w, h))).collect());
        let below = || {
            (plains.get(i + 1))
                .and_then(Option::as_ref)
                .filter(|below| 2 * below.width == w && 2 * below.height == h)
                .map(|below| (below.channels.iter().map(|c| bilinear_up(c, w, h))).collect())
        };
        let target: Option<Vec<Vec<f32>>> = above.or_else(below);
        let coded = Coded {
            lanes,
            at,
            width: level.width,
            plain: plain_beside(i, -1, 4) || plain_beside(i, 1, 1),
            open: false,
            target,
        };
        out.push((i, coded));
    }
    if let Some(presets) = through.filter(|_| out.iter().any(|(_, level)| !level.plain)) {
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
        // A level none of whose blocks hangs on something unknown has nothing to vote.
        for (i, level) in out.iter_mut().filter(|(_, level)| !level.plain) {
            level.open = opens[*i];
            level.target = (level.open).then(|| beside(&draws, *i)).flatten();
        }
    }
    out.into_iter().map(|(_, level)| level).collect()
}

/// The target of the block that lies at `place` of a level `width` blocks wide; `None` for a block outside the level.
fn wanted(target: &[Vec<f32>], width: usize, place: u32) -> Option<BlockPixels> {
    let place = usize::try_from(place).ok().filter(|_| place != u32::MAX)?;
    let origin = 16 * (place / width) * width + 4 * (place % width);
    let mut out = [[0f32; 4]; 16];
    for (pixel, colour) in out.iter_mut().enumerate() {
        let at = origin + (pixel / 4) * 4 * width + pixel % 4;
        for (value, channel) in colour.iter_mut().zip(target) {
            *value = *channel.get(at)?;
        }
    }
    Some(out)
}

/// What a pass reads of the block last stepped, taken once for all its votes.
struct Block {
    sig: Sig,
    partition: usize,
    /// The two ends of each subset's output channels as 8 bits; `None` where the mode has no such field.
    spans: [[Option<[u8; 2]>; 4]; 3],
    /// The weight, of 64, each pixel gives each output channel.
    weights: [[u8; 4]; 16],
}

impl Block {
    fn of(walk: &Walk<'_>, block: usize, sig: Sig) -> Self {
        Self {
            sig,
            partition: walk.partition(sig),
            spans: std::array::from_fn(|subset| {
                std::array::from_fn(|channel| walk.span(block, sig, subset, channel))
            }),
            weights: std::array::from_fn(|pixel| {
                std::array::from_fn(|channel| walk.weight(block, sig, pixel, channel))
            }),
        }
    }

    /// Each pixel's first three channels as subset `subset`'s ends and the pixel's index draw them.
    fn mixed(&self, subset: usize) -> [[Option<f32>; 3]; 16] {
        std::array::from_fn(|pixel| {
            std::array::from_fn(|channel| {
                self.spans[subset][channel]
                    .map(|span| bptc::mix(span, self.weights[pixel][channel]))
            })
        })
    }
}

/// Votes the preset indices block `block` hangs on, its endpoints being known.
fn vote_indices(
    walk: &Walk<'_>,
    block: usize,
    of: &Block,
    wanted: &BlockPixels,
    votes: &mut Votes,
) {
    let sig = of.sig;
    let slots = walk.index_slots(block);
    for (first, from, depth) in [(true, slots.0, sig.first_depth()), (false, slots.1, 2)] {
        if from == 0 {
            continue;
        }
        let weights = bptc::weights(depth);
        // For each subset whose ends are apart, what each index draws in the channels this set weighs.
        let draws: [Option<[[Option<f32>; 4]; 16]>; 3] = std::array::from_fn(|subset| {
            let spans: [Option<[u8; 2]>; 4] = std::array::from_fn(|channel| {
                of.spans[subset][channel].filter(|_| sig.first_weighs(channel) == first)
            });
            let spread: i32 = (spans.iter().flatten())
                .map(|span| (i32::from(span[1]) - i32::from(span[0])).abs())
                .sum();
            (spread >= FLAT).then(|| {
                std::array::from_fn(|index| {
                    let weight = weights.get(index).copied().unwrap_or(0);
                    spans.map(|span| span.map(|span| bptc::mix(span, weight)))
                })
            })
        });
        for (pixel, target) in wanted.iter().enumerate() {
            let Some(draws) = &draws[bptc::subset_of(sig.subsets(), of.partition, pixel)] else {
                continue;
            };
            if target.iter().any(|value| value.is_nan()) {
                continue;
            }
            let off = |index: usize| -> f32 {
                let mut off = 0f32;
                for (drawn, wanted) in draws[index].iter().zip(target) {
                    if let Some(drawn) = drawn {
                        off += (drawn - wanted).abs();
                    }
                }
                off
            };
            let mut best = (0, off(0));
            for index in 1..weights.len() {
                let off = off(index);
                if off < best.1 {
                    best = (index, off);
                }
            }
            let best = best.0;
            let slot = 16 * usize::from(from) + pixel;
            if first {
                let trace = walk.first_trace(block, pixel);
                let tally = &mut votes.first[16 * slot..16 * (slot + 1)];
                for (x, vote) in tally.iter_mut().enumerate() {
                    let through = ((trace >> (4 * x)) & 0xF) as usize;
                    *vote += u32::from(through >> (4 - depth) == best);
                }
            } else {
                let adds = usize::from(walk.second_add(block, pixel));
                votes.second[4 * slot + ((best + 4 - adds) & 3)] += 1;
            }
        }
    }
}

/// Votes the preset endpoint values block `block` hangs on, its indices being known.
fn vote_ends(walk: &Walk<'_>, block: usize, of: &Block, wanted: &BlockPixels, votes: &mut Votes) {
    let sig = of.sig;
    for subset in 0..sig.subsets() {
        for channel in 0..4 {
            let depth = sig.depth(subset, channel);
            let value = 2 * (4 * subset + channel);
            let from = [walk.end_from(block, value), walk.end_from(block, value + 1)];
            if depth == 0 || from == [0, 0] {
                continue;
            }
            // The two ends that leave the subset's pixels nearest their targets, by least squares.
            let (mut aa, mut ab, mut bb, mut at, mut bt, mut count) =
                (0f32, 0f32, 0f32, 0f32, 0f32, 0f32);
            for (pixel, target) in wanted.iter().enumerate() {
                let outside = bptc::subset_of(sig.subsets(), of.partition, pixel) != subset;
                if outside || target[channel].is_nan() {
                    continue;
                }
                let b = f32::from(of.weights[pixel][channel]) / 64.0;
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
            for (end, (found, from)) in found.into_iter().zip(from).enumerate() {
                let (Some(found), Some(at)) = (found, usize::from(from).checked_sub(1)) else {
                    continue;
                };
                let found = found.clamp(0.0, 255.0);
                votes.ends_count[at] += 1;
                let widening = walk.widening(block, sig, (subset, channel, end));
                let cut = walk.end_cut(block, value + end);
                let trace = walk.end_trace(block, value + end);
                let costs = &mut votes.ends[ENDS_TALLY * at + ends_tally_at(cut)..];
                for (class, cost) in costs.iter_mut().enumerate().take(256 >> cut) {
                    let x = class << cut;
                    let through = (trace[x / 8] >> (8 * (x % 8))) as u8;
                    let off = (f32::from(bptc::widen(widening, through)) - found)
                        .abs()
                        .min(CAP);
                    // Rounded to the nearest, a half upward: the sum is exact in the wider type.
                    *cost = cost.saturating_add((f64::from(2.0 * off) + 0.5) as u32);
                }
            }
        }
    }
}

/// Votes the partition of a block of mode 0 or 2, its endpoints and indices being known.
fn vote_order(of: &Block, wanted: &BlockPixels, votes: &mut Votes) {
    let sig = of.sig;
    let (which, partitions) = match sig.mode {
        0 => (0, 16),
        2 => (1, 64),
        _ => return,
    };
    if wanted.iter().flatten().any(|value| value.is_nan()) {
        return;
    }
    votes.orders_count[which][sig.position] += 1;
    let mixed: [_; 3] = std::array::from_fn(|subset| of.mixed(subset));
    let costs = &mut votes.orders[which][partitions * sig.position..][..partitions];
    for (partition, cost) in costs.iter_mut().enumerate() {
        let mut off = 0f32;
        for (pixel, target) in wanted.iter().enumerate() {
            let drawn = &mixed[bptc::subset_of(sig.subsets(), partition, pixel)][pixel];
            for (drawn, &target) in drawn.iter().zip(target) {
                if let Some(drawn) = drawn {
                    off += (drawn - target).abs();
                }
            }
        }
        *cost += off.round() as u64;
    }
}

/// Whether a block of `lanes` may hang on something `presets` does not know.
///
/// A reference reaches 255 blocks back, so only a body's first 255 blocks read a preset, and
/// every later block hangs on what they read: the body is walked no further. A value one of
/// them carries and no later block draws with still counts, so this says yes too often, never no.
fn hangs_open(lanes: &[Vec<u16>], presets: &Presets) -> bool {
    let Ok(mut walk) = Walk::new(lanes, presets, false) else {
        return false;
    };
    (0..walk.blocks.min(255)).any(|block| {
        if walk.step(block).is_err() {
            return false;
        }
        let ends = (0..VALUES).any(|value| {
            let from = usize::from(walk.end_from(block, value));
            let at = from.saturating_sub(1);
            from != 0 && !presets.ends_known[at / VALUES][at % VALUES]
        });
        let (first, second) = walk.index_slots(block);
        ends || (first != 0 && !presets.first_known[usize::from(first)])
            || (second != 0 && !presets.second_known[usize::from(second)])
    })
}

/// Adds `texture`'s votes, read through `presets` as they stand.
///
/// With `mark`, it says whether a coded level with no plain level beside it has a block that
/// hangs on something not known, which is what a later pass has to fill. `beside`, the presets
/// as the plain levels left them, makes it that pass: it takes those levels alone, a block votes
/// only what those presets do not know, and it says whether a level had such a block.
pub(super) fn vote(
    texture: &TextureIn,
    presets: &Presets,
    votes: &mut Votes,
    beside: Option<&Presets>,
    mark: bool,
) -> bool {
    let levels = coded(texture, beside.map(|plain| (presets, plain)));
    let lone = (levels.iter().filter(|level| !level.plain)).any(|level| match beside {
        Some(_) => level.open,
        None => mark && hangs_open(level.lanes, presets),
    });
    for level in levels {
        if beside.is_some() && !level.open {
            continue;
        }
        let (Some(target), Ok(mut walk)) = (&level.target, Walk::new(level.lanes, presets, true))
        else {
            continue;
        };
        for block in 0..walk.blocks {
            let Ok(sig) = walk.step(block) else {
                break;
            };
            let Some(wanted) = wanted(target, level.width, level.at[block]) else {
                continue;
            };
            let (ends, indices) = (walk.ends_known(block, sig), walk.indices_known(block));
            // Whether the block hangs on an endpoint value, and on an index slot, still to be voted.
            let open = beside.map_or((true, true), |plain| open(&walk, block, sig, plain));
            if open == (false, false) {
                continue;
            }
            let of = Block::of(&walk, block, sig);
            if ends && open.1 {
                vote_indices(&walk, block, &of, &wanted, votes);
            }
            if indices && open.0 {
                vote_ends(&walk, block, &of, &wanted, votes);
            }
            if ends && indices && beside.is_none() {
                vote_order(&of, &wanted, votes);
            }
        }
    }
    lone
}

/// The most voted of `tally`, where it holds [`MIN_VOTES`] votes.
fn most(tally: &[u32]) -> Option<u8> {
    let best = (0..tally.len()).fold(0, |best, x| if tally[x] > tally[best] { x } else { best });
    (tally.iter().sum::<u32>() >= MIN_VOTES).then_some(best as u8)
}

/// The preset value that costs least by `tally`, one preset value's costs.
fn least(tally: &[u32]) -> usize {
    // A tally stops at the most it can hold, and so does the sum, as when every vote was added to every value.
    let cost = |x: usize| -> u32 {
        let whole: u64 = (0..=8)
            .map(|cut| u64::from(tally[ends_tally_at(cut) + (x >> cut)]))
            .sum();
        whole.min(u64::from(u32::MAX)) as u32
    };
    let costs: Vec<u32> = (0..256).map(cost).collect();
    (0..256).fold(0, |best, x| if costs[x] < costs[best] { x } else { best })
}

/// Takes into `presets` what `votes` settle; a value with too few votes keeps what it had.
///
/// With `fill`, the presets as the plain levels left them, only what those did not know is
/// taken, and the partition orders stay.
pub(super) fn solve(votes: &Votes, presets: &mut Presets, fill: Option<&Presets>) {
    for slot in 1..256 {
        let (mut first, mut second) = (true, true);
        let open = (
            !fill.is_some_and(|plain| plain.first_known[slot]),
            !fill.is_some_and(|plain| plain.second_known[slot]),
        );
        for pixel in 0..16 {
            let at = 16 * slot + pixel;
            match most(&votes.first[16 * at..16 * (at + 1)]) {
                Some(index) if open.0 => presets.first[slot][pixel] = index,
                Some(_) => {}
                None => first = false,
            }
            match most(&votes.second[4 * at..4 * (at + 1)]) {
                Some(index) if open.1 => presets.second[slot][pixel] = index,
                Some(_) => {}
                None => second = false,
            }
        }
        presets.first_known[slot] |= first;
        presets.second_known[slot] |= second;
        for value in 0..VALUES {
            let at = VALUES * slot + value;
            let open = !fill.is_some_and(|plain| plain.ends_known[slot][value]);
            if votes.ends_count[at] >= 2 * MIN_VOTES && open {
                presets.ends[slot][value] =
                    least(&votes.ends[ENDS_TALLY * at..ENDS_TALLY * (at + 1)]) as u8;
                presets.ends_known[slot][value] = true;
            }
        }
    }
    if fill.is_some() {
        return;
    }
    place(
        &votes.orders[0],
        &votes.orders_count[0],
        &mut presets.order0,
    );
    place(
        &votes.orders[1],
        &votes.orders_count[1],
        &mut presets.order2,
    );
}

/// Gives each voted position of `order` the partition that costs its blocks least, the most voted position first and no partition twice.
fn place(costs: &[u64], counts: &[u32], order: &mut [u8]) {
    let partitions = order.len();
    let mut positions: Vec<usize> = (0..partitions)
        .filter(|&position| counts[position] >= MIN_VOTES)
        .collect();
    positions.sort_by_key(|&position| std::cmp::Reverse(counts[position]));
    order.fill(UNPLACED);
    for position in positions {
        let costs = &costs[partitions * position..partitions * (position + 1)];
        let free = (0..partitions).filter(|&partition| !order.contains(&(partition as u8)));
        let best = free.reduce(|best, partition| {
            if costs[partition] < costs[best] {
                partition
            } else {
                best
            }
        });
        if let Some(best) = best {
            order[position] = best as u8;
        }
    }
}

/// Draws `texture`'s coded levels through `presets` and adds what they show to `summary`.
pub(super) fn score(texture: &TextureIn, presets: &Presets, summary: &mut Summary) {
    let levels = coded(texture, None);
    summary.textures += u64::from(!levels.is_empty());
    for level in levels {
        let Ok(mut walk) = Walk::new(level.lanes, presets, false) else {
            continue;
        };
        summary.bodies += 1;
        for block in 0..walk.blocks {
            let Ok(sig) = walk.step(block) else {
                break;
            };
            summary.blocks += 1;
            let known = walk.ends_known(block, sig) && walk.indices_known(block);
            summary.unknown += u64::from(!known);
            let wanted = (level.target.as_ref())
                .filter(|_| known)
                .and_then(|target| wanted(target, level.width, level.at[block]));
            let Some(wanted) = wanted else {
                continue;
            };
            let of = Block::of(&walk, block, sig);
            for (pixel, target) in wanted.iter().enumerate() {
                // A colour under a transparent pixel is whatever the encoder left there.
                let seen = target[3] >= 16.0;
                let subset = bptc::subset_of(sig.subsets(), of.partition, pixel);
                for (channel, &target) in target.iter().enumerate() {
                    // A mode with no alpha draws 255.
                    let drawn = (of.spans[subset][channel])
                        .map_or(255.0, |span| bptc::mix(span, of.weights[pixel][channel]));
                    let off = f64::from((drawn - target).abs());
                    if channel == 3 {
                        summary.alpha_mismatch += off;
                        summary.alpha_compared += 1;
                    } else if seen {
                        summary.mismatch += off;
                        summary.compared += 1;
                    }
                }
            }
        }
    }
}

/// The preset slots `presets` knows an endpoint value of, and those it knows the first indices of.
pub(super) fn solved(presets: &Presets) -> (u32, u32) {
    let ends = (presets.ends_known.iter())
        .filter(|known| known.contains(&true))
        .count();
    let first = presets.first_known.iter().filter(|&&known| known).count();
    (ends as u32, first as u32)
}
