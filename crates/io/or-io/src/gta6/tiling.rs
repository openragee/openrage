//! The 4 KiB and 64 KiB tiles GTA VI lays texture memory in: a slice's chain holds a tail tile of
//! the small levels first and level 0 last, each level's tiles row by row.

use super::ttpf::Format;
use crate::codec::texture::DxgiFormat;

/// Where tail level i starts in a 4 KiB tail tile of 1-, 4- or 8-byte elements; 16-byte elements stop at index 5.
const SLOTS_4K: &[u64] = &[0x800, 0x600, 0x500, 0x400, 0x300, 0x200, 0x100];
/// Where tail level i starts in a 64 KiB tail tile of 16-byte elements; 8-byte elements stop at index 6, 1-byte at index 5, 4-byte at index 4.
const SLOTS_64K: &[u64] = &[0x8000, 0x4000, 0x2000, 0x1000, 0x800, 0x600, 0x500, 0x400];
/// Where tail level i starts in a volume's 4 KiB tail tile, whatever its element size.
const SLOTS_3D: &[u64] = &[0x800, 0x300, 0x200, 0x100, 0];
/// A volume level is a tail level when its part of one z-slab fits a tile and is at most this many bytes.
const TAIL_3D_BYTES: usize = 0x800;

/// A tile: the coordinate bit each element-index bit takes, lowest first, and the tail slots.
pub(super) struct Tile {
    pattern: &'static str,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) depth: usize,
    element: usize,
    /// No slot is measured for 2-byte elements: no texture holds a 2-byte tail.
    slots: &'static [u64],
}

impl Tile {
    /// A 2D tile of `format`'s elements, of 64 KiB when `large`, else of 4 KiB.
    pub(super) fn of(format: Format, large: bool) -> Self {
        let element = format.element_bytes();
        let (pattern, slots): (&str, &[u64]) = match (large, element) {
            (false, 1) => ("x0 x1 x2 x3 y0 y1 y2 y3 y4 x4 y5 x5", SLOTS_4K),
            (true, 1) => (
                "x0 x1 x2 y1 y0 y2 x3 y3 y4 x4 y5 x5 y6 x6 y7 x7",
                &SLOTS_64K[..6],
            ),
            (false, 2) => ("x0 x1 x2 y0 y1 y2 x3 y3 x4 y4 x5", &[]),
            (true, 2) => ("x0 x1 x2 y0 y1 y2 x3 y3 x4 y4 x5 y5 x6 y6 x7", &[]),
            (false, 4) => ("x0 x1 y0 y1 y2 x2 y3 x3 y4 x4", SLOTS_4K),
            (true, 4) => ("x0 x1 y0 y1 x2 y2 y3 x3 y4 x4 y5 x5 y6 x6", &SLOTS_64K[..5]),
            (false, 8) => ("x0 y0 y1 x1 x2 y2 x3 y3 x4", SLOTS_4K),
            (true, 8) => ("x0 y0 x1 x2 y1 y2 x3 y3 x4 y4 x5 y5 x6", &SLOTS_64K[..7]),
            (false, _) => ("y0 y1 x0 x1 y2 x2 y3 x3", &SLOTS_4K[..6]),
            (true, _) => ("x0 y0 x1 y1 y2 x2 y3 x3 y4 x4 y5 x5", SLOTS_64K),
        };
        Self::new(pattern, element, slots)
    }

    /// [`Self::of`] on the PlayStation 5, whose 64 KiB tile is the 4 KiB tile's order, then the Xbox 64 KiB tile's other bits.
    pub(super) fn prospero(format: Format, large: bool) -> Self {
        let xbox = Self::of(format, large);
        let pattern = match (large, xbox.element) {
            (false, _) | (true, 2) => return xbox,
            (true, 1) => "x0 x1 x2 x3 y0 y1 y2 y3 y4 x4 y5 x5 y6 x6 y7 x7",
            (true, 4) => "x0 x1 y0 y1 y2 x2 y3 x3 y4 x4 y5 x5 y6 x6",
            (true, 8) => "x0 y0 y1 x1 x2 y2 x3 y3 x4 y4 x5 y5 x6",
            (true, _) => "y0 y1 x0 x1 y2 x2 y3 x3 y4 x4 y5 x5",
        };
        Self::new(pattern, xbox.element, xbox.slots)
    }

    /// The 4 KiB tile of a volume of `format`'s elements: the bits that fill 16 bytes, then z y x z y x z y.
    pub(super) fn volume(format: Format) -> Option<Self> {
        let element = format.element_bytes();
        let pattern = match element {
            1 => "x0 x1 z0 y0 z1 y1 x2 z2 y2 x3 z3 y3",
            2 => "x0 z0 y0 z1 y1 x1 z2 y2 x2 z3 y3",
            4 => "x0 y0 z0 y1 x1 z1 y2 x2 z2 y3",
            8 => "x0 z0 y0 x1 z1 y1 x2 z2 y2",
            16 => "z0 y0 x0 z1 y1 x1 z2 y2",
            _ => return None,
        };
        Some(Self::new(pattern, element, SLOTS_3D))
    }

    fn new(pattern: &'static str, element: usize, slots: &'static [u64]) -> Self {
        let side = |axis| 1 << pattern.matches(axis).count();
        Self {
            pattern,
            width: side('x'),
            height: side('y'),
            depth: side('z'),
            element,
            slots,
        }
    }

    pub(super) fn bytes(&self) -> u64 {
        (self.width * self.height * self.depth * self.element) as u64
    }

    /// Each element-index bit: its axis (`b'x'`, `b'y'` or `b'z'`), and which bit of the coordinate it is.
    pub(super) fn bits(&self) -> impl Iterator<Item = (u8, usize)> + '_ {
        self.pattern
            .as_bytes()
            .chunks(3)
            .map(|token| (token[0], usize::from(token[1] - b'0')))
    }

    /// Whether a level of this many elements lies in the tail tile with every smaller level.
    fn holds_in_tail(&self, width: usize, height: usize) -> bool {
        width <= self.width / 2 && height <= self.height
    }

    fn tiles(&self, width: usize, height: usize) -> u64 {
        (width.div_ceil(self.width) * height.div_ceil(self.height)) as u64 * self.bytes()
    }
}

/// One mip level inside a slice's chain.
#[derive(Debug, Clone, Copy)]
pub(super) struct Level {
    /// Where its first tile, or for a tail level its slot, starts in the chain.
    pub(super) at: u64,
    /// Its size in elements.
    pub(super) width: usize,
    pub(super) height: usize,
    /// Whether it lies in the tail tile at the chain's start.
    pub(super) tail: bool,
    /// The bytes of its tiles, or of the tail tile.
    pub(super) len: u64,
}

impl Level {
    /// The first byte of the chain it needs: its tiles, or the tail tile.
    pub(super) fn from(&self) -> u64 {
        if self.tail {
            0
        } else {
            self.at
        }
    }
}

/// Level `level`'s size in elements.
fn size(width: u32, height: u32, format: Format, level: u32) -> (usize, usize) {
    let side = |pixels: u32| (pixels >> level).max(1).div_ceil(format.element_side()) as usize;
    (side(width), side(height))
}

fn level_count(width: u32, height: u32) -> u32 {
    u32::BITS - width.max(height).leading_zeros()
}

/// The levels a slice's chain of `range` bytes holds, top first: every explicit level, then each
/// tail level a slot is measured for; `None` when no chain of the texture's size is `range` bytes.
/// A `lone` level that fits one tile is that tile, at offset 0.
pub(super) fn chain(
    width: u32,
    height: u32,
    format: Format,
    tile: &Tile,
    lone: bool,
    range: u64,
) -> Option<Vec<Level>> {
    let count = level_count(width, height);
    let mut levels = Vec::new();
    let mut end = range;
    for level in 0..count {
        let (w, h) = size(width, height, format, level);
        if tile.holds_in_tail(w, h) {
            // This level and every smaller one share one tile, at the chain's start.
            if end != tile.bytes() {
                return None;
            }
            let lone = level == 0 && lone;
            let slots: &[u64] = if lone { &[0] } else { tile.slots };
            for (&at, level) in slots.iter().zip(level..count) {
                let (width, height) = size(width, height, format, level);
                levels.push(Level {
                    at,
                    width,
                    height,
                    tail: true,
                    len: tile.bytes(),
                });
            }
            return Some(levels);
        }
        let len = tile.tiles(w, h);
        end = end.checked_sub(len)?;
        levels.push(Level {
            at: end,
            width: w,
            height: h,
            tail: false,
            len,
        });
        if end == 0 {
            return Some(levels);
        }
    }
    None
}

/// The lengths a chain of the texture's top levels can have, shortest first: level 0 alone, then
/// with each next level, and last the whole chain down to its tail tile.
pub(super) fn chain_lengths(width: u32, height: u32, format: Format, tile: &Tile) -> Vec<u64> {
    let mut lengths = Vec::new();
    let mut sum = 0;
    for level in 0..level_count(width, height) {
        let (w, h) = size(width, height, format, level);
        if tile.holds_in_tail(w, h) {
            lengths.push(sum + tile.bytes());
            break;
        }
        sum += tile.tiles(w, h);
        lengths.push(sum);
    }
    lengths
}

/// A volume's levels, top first, and the bytes of each of its z-slabs, which each hold a tail tile
/// of the small levels, then the other levels smallest first and level 0 last; `None` when no
/// volume of its size is `range` bytes. A tail level past the measured slots is left out.
pub(super) fn volume_chain(
    sides: [u32; 3],
    format: Format,
    tile: &Tile,
    mips: u32,
    range: u64,
) -> Option<(Vec<Level>, u64)> {
    let (levels, slab) = volume_levels(sides, format, tile, mips);
    let slabs = u64::from(sides[2]).div_ceil(tile.depth as u64);
    (slab.checked_mul(slabs)? == range).then_some((levels, slab))
}

/// [`volume_chain`]'s levels and z-slab for a volume of any range.
pub(super) fn volume_levels(
    sides: [u32; 3],
    format: Format,
    tile: &Tile,
    mips: u32,
) -> (Vec<Level>, u64) {
    let [width, height, depth] = sides;
    let count = mips.clamp(1, u32::BITS - width.max(height).max(depth).leading_zeros());
    let (mut levels, mut tails, mut footprint) = (Vec::new(), 0, 0);
    for level in 0..count {
        let (w, h) = size(width, height, format, level);
        let d = (depth >> level).max(1) as usize;
        let small = w <= tile.width
            && h <= tile.height
            && w * h * d.min(tile.depth) * tile.element <= TAIL_3D_BYTES;
        if count > 1 && (tails > 0 || small) {
            if let Some(&at) = tile.slots.get(tails) {
                levels.push(Level {
                    at,
                    width: w,
                    height: h,
                    tail: true,
                    len: tile.bytes(),
                });
            }
            tails += 1;
            continue;
        }
        let len = tile.tiles(w, h);
        footprint += len;
        levels.push(Level {
            at: footprint,
            width: w,
            height: h,
            tail: false,
            len,
        });
    }
    let slab = footprint + if tails > 0 { tile.bytes() } else { 0 };
    // Each level's tiles end where the next larger level's start, level 0's at the slab's end.
    for level in levels.iter_mut().filter(|l| !l.tail) {
        level.at = slab - level.at;
    }
    (levels, slab)
}

/// Where a texture's slices lie in its memory.
pub(super) struct Layout {
    pub(super) format: Format,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) tile: Tile,
    pub(super) levels: Vec<Level>,
    pub(super) slices: u64,
    /// Each slice's chain length.
    pub(super) unit: u64,
    /// Piece by piece (the tail tiles of every slice, then each level of every slice), not slice by slice.
    pub(super) piece_major: bool,
    /// Whether the levels are stated, so that a tail level of zero bytes is one, not a slot past the chain.
    pub(super) stated: bool,
    /// A volume's depth, its z-slabs `unit` bytes apart; `None` for any other shape.
    pub(super) depth: Option<u32>,
}

impl Layout {
    /// Every slice's levels as the texture's pixels, top first, slice after slice, and how many
    /// levels each slice keeps; levels needing bytes before `read_from` are not read. `None` when
    /// not even the top level lies in `memory`.
    pub(super) fn read(&self, memory: &[u8], read_from: u64) -> Option<(u32, Vec<u8>)> {
        if let Some(depth) = self.depth {
            return self.read_volume(memory, depth);
        }
        let mut kept = self.levels.len();
        let mut each = Vec::new();
        for slice in 0..self.slices {
            let mut levels = Vec::new();
            for (i, level) in self.levels.iter().enumerate().take(kept) {
                let (start, len) = (level.from(), level.len);
                let base = if self.piece_major {
                    self.slices * start + slice * len
                } else {
                    slice * self.unit + start
                };
                let elements = usize::try_from(base)
                    .ok()
                    .filter(|_| start >= read_from)
                    .and_then(|base| memory.get(base..base.checked_add(len as usize)?))
                    .and_then(|piece| {
                        untile(
                            piece,
                            level.at - start,
                            [level.width, level.height, 1],
                            &self.tile,
                        )
                    })
                    // A tail slot past the end of a chain whose levels are not stated is all zero.
                    .filter(|e| self.stated || i == 0 || !level.tail || e.iter().any(|&b| b != 0));
                let Some(elements) = elements else {
                    kept = i;
                    break;
                };
                levels.push(pixels(self.format, elements, self.width, self.height, i));
            }
            each.push(levels);
        }
        (kept > 0).then(|| {
            let pixels = each
                .into_iter()
                .flat_map(|levels| levels.into_iter().take(kept).flatten())
                .collect();
            (kept as u32, pixels)
        })
    }

    /// A volume read whole as `depth` slices, slice s holding slice `s >> m` of each level m.
    fn read_volume(&self, memory: &[u8], depth: u32) -> Option<(u32, Vec<u8>)> {
        let levels = (self.levels.iter().enumerate())
            .map(|(m, level)| {
                let d = (depth >> m).max(1) as usize;
                let size = [level.width, level.height, d];
                let elements = untile_slabs(memory, level.at, size, &self.tile, self.unit)?;
                Some((elements, d))
            })
            .collect::<Option<Vec<_>>>()?;
        let mut out = Vec::new();
        for slice in 0..depth as usize {
            for (m, (elements, d)) in levels.iter().enumerate() {
                let plane = elements.len() / d;
                let at = (slice >> m).min(d - 1) * plane;
                let elements = elements[at..at + plane].to_vec();
                out.extend(pixels(self.format, elements, self.width, self.height, m));
            }
        }
        Some((levels.len() as u32, out))
    }
}

/// `size` (width, height, depth) elements, from tiles laid out row by row, then plane by plane,
/// from `at`, to linear rows; `None` if one lies past `memory`.
pub(super) fn untile(memory: &[u8], at: u64, size: [usize; 3], tile: &Tile) -> Option<Vec<u8>> {
    let plane = size[0].div_ceil(tile.width) * size[1].div_ceil(tile.height);
    untile_slabs(
        memory,
        at,
        size,
        tile,
        (plane as u64).checked_mul(tile.bytes())?,
    )
}

/// [`untile`] with each plane of tiles `slab` bytes after the one before it.
fn untile_slabs(
    memory: &[u8],
    at: u64,
    size: [usize; 3],
    tile: &Tile,
    slab: u64,
) -> Option<Vec<u8>> {
    let [width, height, depth] = size;
    let sides = [tile.width, tile.height, tile.depth];
    let mut along = sides.map(|side| vec![0usize; side]);
    for (bit, (axis, from)) in tile.bits().enumerate() {
        let spread = &mut along[usize::from(axis - b'x')];
        for (coordinate, index) in spread.iter_mut().enumerate() {
            *index |= ((coordinate >> from) & 1) << bit;
        }
    }
    let (at, slab) = (usize::try_from(at).ok()?, usize::try_from(slab).ok()?);
    let (element, tile_bytes) = (tile.element, tile.bytes() as usize);
    let (across, down) = (width.div_ceil(tile.width), height.div_ceil(tile.height));
    // Tiles cover the elements, so tiles that fit `memory` bound what is allocated.
    let tiles = across * down * depth.div_ceil(tile.depth);
    if tiles.checked_mul(tile_bytes)? > memory.len() {
        return None;
    }
    let mut out = vec![0; width * height * depth * element];
    for (i, block) in out.chunks_exact_mut(element).enumerate() {
        let (x, y, z) = (i % width, i / width % height, i / width / height);
        let tile_at = (y / tile.height) * across + x / tile.width;
        let index = along[0][x % tile.width] | along[1][y % tile.height] | along[2][z % tile.depth];
        let from = at + (z / tile.depth) * slab + tile_at * tile_bytes + index * element;
        block.copy_from_slice(memory.get(from..from + element)?);
    }
    Some(out)
}

/// `elements`, the z-planes `planes` of a level of `size` (width, height) elements as linear rows,
/// written into `memory` where [`untile_slabs`] reads them; `false` if one lies past `memory`.
pub(super) fn place_planes(
    memory: &mut [u8],
    at: u64,
    size: [usize; 2],
    planes: std::ops::Range<usize>,
    elements: &[u8],
    tile: &Tile,
    slab: u64,
) -> bool {
    let [width, height] = size;
    let sides = [tile.width, tile.height, tile.depth];
    let mut along = sides.map(|side| vec![0usize; side]);
    for (bit, (axis, from)) in tile.bits().enumerate() {
        let Some(spread) = along.get_mut(usize::from(axis - b'x')) else {
            return false;
        };
        for (coordinate, index) in spread.iter_mut().enumerate() {
            *index |= ((coordinate >> from) & 1) << bit;
        }
    }
    let (Ok(at), Ok(slab)) = (usize::try_from(at), usize::try_from(slab)) else {
        return false;
    };
    let (element, tile_bytes) = (tile.element, tile.bytes() as usize);
    let (across, plane) = (width.div_ceil(tile.width), width * height);
    if elements.len() != plane * planes.len() * element {
        return false;
    }
    for (i, block) in elements.chunks_exact(element).enumerate() {
        let (x, y, z) = (i % width, i / width % height, planes.start + i / plane);
        let tile_at = (y / tile.height) * across + x / tile.width;
        let index = along[0][x % tile.width] | along[1][y % tile.height] | along[2][z % tile.depth];
        let to = at + (z / tile.depth) * slab + tile_at * tile_bytes + index * element;
        let Some(slot) = memory.get_mut(to..to + element) else {
            return false;
        };
        slot.copy_from_slice(block);
    }
    true
}

/// Level `level`'s untiled elements as the texture's pixels: a signed BC6H level's blocks decode
/// to the RGBA floats the tree reads, since the tree reads BC6H as unsigned.
fn pixels(format: Format, elements: Vec<u8>, width: u32, height: u32, level: usize) -> Vec<u8> {
    if format.dxgi() != DxgiFormat::Bc6hSf16 {
        return elements;
    }
    let shift = |pixels: u32| (pixels >> level).max(1) as usize;
    let (width, height) = (shift(width), shift(height));
    let across = width.div_ceil(4);
    let mut out = vec![0u8; width * height * 16];
    let mut block = [0f32; 48];
    for (i, bytes) in elements.as_chunks::<16>().0.iter().enumerate() {
        bcdec_rs::bc6h_float(bytes, &mut block, 12, true);
        for (k, rgb) in block.as_chunks::<3>().0.iter().enumerate() {
            let (x, y) = (i % across * 4 + k % 4, i / across * 4 + k / 4);
            if x < width && y < height {
                let at = (y * width + x) * 16;
                for (c, value) in rgb.iter().chain([&1.0]).enumerate() {
                    out[at + 4 * c..at + 4 * c + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
    out
}
