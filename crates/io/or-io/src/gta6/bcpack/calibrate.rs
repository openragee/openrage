//! The fixed tables of BC1, BC3, BC4, BC5, BC6H and BC7 fitted on this machine from the user's own packs.
//!
//! No stream carries a mode's preset or its index tables, and no texture is stored both coded and
//! plain, so a table is fitted against the plain levels beside a coded one: the level above gives
//! each pixel a target, the level below each 2 by 2 pixels. A pass votes each index symbol's
//! pattern from the blocks whose endpoints are known and each preset slot's fields from the
//! blocks whose indices are known; later passes meet a level with no plain level beside it
//! through the levels and the neighbouring blocks already solved. A fitted table is an estimate.
//!
//! A pass keeps tallies of a fixed size and one texture at a time for each worker, so a fit's
//! memory does not grow with the packs it reads.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use super::bc6h;
use super::bptc::{Presets, PRESETS_BYTES};
use super::planes::{
    self, Chains, Half, Mode, ModeTables, Plane, FIELDS, QUADRANTS, UNKNOWN, UNKNOWN_PAIR,
};
use crate::gta6::hash::name_hash64;

#[path = "bc6h_fit.rs"]
mod bc6h_fit;
#[path = "bptc_fit.rs"]
mod bptc_fit;

/// The modes of two halves whose tables are fitted; BC6H's and BC7's presets are fitted beside them.
const KINDS: [u8; 4] = [1, 3, 4, 5];
const MAGIC: [u8; 4] = *b"ORBP";
const VERSION: u32 = 4;
const MODE_BYTES: usize = 2 * 256 * FIELDS + 64 + 2 * 256;
const TABLES_BYTES: usize = 8 + KINDS.len() * MODE_BYTES + PRESETS_BYTES + bc6h::PRESETS_BYTES;
/// Votes a preset field or an index symbol needs before it is taken as known.
const MIN_VOTES: u32 = 3;
/// Passes against the plain levels, then passes through what is already solved.
const FIRST_PASSES: usize = 4;
const SECOND_PASSES: usize = 2;
/// The passes that vote BC6H's presets: a field is settled 8 bits to a pass, its base before its
/// other ends, and each waits for the indices that the ends in turn let a pass vote.
const HDR_PASSES: usize = 8;
/// The passes after them that vote what those left open: they read only the levels with a
/// block that hangs on it, and hold one with no plain level beside it against what the fit draws.
const HDR_FILLS: usize = 5;
/// The most workers a pass reads textures on, each holding one texture and its own tallies.
const MAX_WORKERS: usize = 16;

/// The tables fitted for every mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tables {
    modes: Vec<ModeTables>,
    bptc: Presets,
    hdr: bc6h::Presets,
}

impl Tables {
    pub(super) fn empty() -> Self {
        Self {
            modes: KINDS.iter().map(|_| ModeTables::empty()).collect(),
            bptc: Presets::empty(),
            hdr: bc6h::Presets::empty(),
        }
    }

    fn at(kind: u8) -> Option<usize> {
        KINDS.iter().position(|&fitted| fitted == kind)
    }

    pub(super) fn of(&self, kind: u8) -> Option<&ModeTables> {
        Self::at(kind).map(|at| &self.modes[at])
    }

    /// BC7's presets.
    pub(super) fn bptc(&self) -> &Presets {
        &self.bptc
    }

    /// BC6H's presets.
    pub(super) fn hdr(&self) -> &bc6h::Presets {
        &self.hdr
    }

    /// Whether a fitted value serves bodies of codec kind `kind`.
    pub fn covers(&self, kind: u8) -> bool {
        match kind {
            6 => return self.hdr.any(),
            7 => return self.bptc.any(),
            _ => {}
        }
        self.of(kind).is_some_and(|tables| match kind {
            1 => (tables.preset.iter().flatten()).any(|&value| value != UNKNOWN),
            _ => tables.pairs.iter().any(|&pair| pair != UNKNOWN_PAIR),
        })
    }

    /// The tables as a calibration file holds them.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        for tables in &self.modes {
            out.extend((tables.preset.iter().flatten()).flat_map(|value| value.to_le_bytes()));
            out.extend(tables.pairs);
            out.extend((tables.quadrants.iter()).flat_map(|value| value.to_le_bytes()));
        }
        out.extend(self.bptc.to_bytes());
        out.extend(self.hdr.to_bytes());
        out
    }

    /// The tables [`Self::to_bytes`] wrote; `None` for any other bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let head: [u8; 8] = bytes.get(..8)?.try_into().ok()?;
        if bytes.len() != TABLES_BYTES || head[..4] != MAGIC || head[4..] != VERSION.to_le_bytes() {
            return None;
        }
        let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let modes = (0..KINDS.len())
            .map(|mode| {
                let from = 8 + mode * MODE_BYTES;
                let (pairs_at, quadrants_at) =
                    (from + 2 * 256 * FIELDS, from + 2 * 256 * FIELDS + 64);
                let mut tables = ModeTables::empty();
                for (slot, fields) in tables.preset.iter_mut().enumerate() {
                    for (field, value) in fields.iter_mut().enumerate() {
                        *value = word(from + 2 * (slot * FIELDS + field));
                    }
                }
                tables.pairs.copy_from_slice(&bytes[pairs_at..quadrants_at]);
                for (symbol, value) in tables.quadrants.iter_mut().enumerate() {
                    *value = word(quadrants_at + 2 * symbol);
                }
                tables
            })
            .collect();
        let (bptc, hdr) = bytes[8 + KINDS.len() * MODE_BYTES..].split_at(PRESETS_BYTES);
        Some(Self {
            modes,
            bptc: Presets::from_bytes(bptc)?,
            hdr: bc6h::Presets::from_bytes(hdr)?,
        })
    }
}

/// A pack's identity among those a calibration read: its file's name and length.
pub fn member_key(path: &Path, len: u64) -> u64 {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut text = name.as_bytes().to_vec();
    text.extend(len.to_le_bytes());
    name_hash64(&text)
}

/// The packs `members` key, each once and in order, as a calibration file lists them.
fn listed(members: &[u64]) -> Vec<u8> {
    let mut members = members.to_vec();
    members.sort_unstable();
    members.dedup();
    members.iter().flat_map(|key| key.to_le_bytes()).collect()
}

/// The file under `dir` that holds the calibration of the packs `members` key: the packs name it, wherever they lie.
pub fn cache_path(dir: &Path, members: &[u64]) -> PathBuf {
    dir.join(format!("bcpack-{:016x}.cal", name_hash64(&listed(members))))
}

/// Writes `bytes` at `path` whole or not at all: a run cut short leaves the file that was there.
pub fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let beside = path.with_extension("tmp");
    std::fs::write(&beside, bytes)?;
    std::fs::rename(&beside, path)
}

/// Writes `tables` under `dir` as the calibration of the packs `members` key, and says where.
pub fn save_cache(dir: &Path, tables: &Tables, members: &[u64]) -> std::io::Result<PathBuf> {
    let path = cache_path(dir, members);
    let mut bytes = tables.to_bytes();
    bytes.extend(listed(members));
    write_whole(&path, &bytes)?;
    Ok(path)
}

/// The newest calibration under `dir` that read the pack `member` keys.
pub fn load_cache(dir: &Path, member: u64) -> Option<Tables> {
    let mut found: Option<(std::time::SystemTime, Tables)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "cal") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Some((head, listed)) = bytes.split_at_checked(TABLES_BYTES) else {
            continue;
        };
        let Some(tables) = Tables::from_bytes(head) else {
            continue;
        };
        let (keys, _) = listed.as_chunks::<8>();
        if !keys.iter().any(|key| u64::from_le_bytes(*key) == member) {
            continue;
        }
        let written =
            (entry.metadata().and_then(|m| m.modified())).unwrap_or(std::time::UNIX_EPOCH);
        if found.as_ref().is_none_or(|(newest, _)| written > *newest) {
            found = Some((written, tables));
        }
    }
    found.map(|(_, tables)| tables)
}

/// One level of a texture as a calibration reads it.
pub enum LevelData {
    /// Its blocks in linear rows, read through no fitted table.
    Plain(Vec<u8>),
    /// A coded body's lanes, and where each of its blocks lies in the level's linear rows: `u32::MAX` for one outside it.
    Coded { lanes: Vec<Vec<u16>>, at: Vec<u32> },
    /// A coded body left out for its size, which a pass that fills reads.
    Large,
    /// Not read.
    Absent,
}

/// A level: its size in blocks, and what was read of it.
pub struct LevelIn {
    pub width: usize,
    pub height: usize,
    pub data: LevelData,
}

/// One mip chain of codec kind `kind`'s blocks, the top level first.
pub struct TextureIn {
    pub kind: u8,
    /// Whether its BC6H blocks are of the signed format.
    pub signed: bool,
    pub levels: Vec<LevelIn>,
}

/// What a calibration found for one mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    pub kind: u8,
    /// Mip chains read that hold a coded body.
    pub textures: u64,
    pub bodies: u64,
    pub blocks: u64,
    /// Blocks drawn through a preset slot or an index symbol the fit did not solve.
    pub unknown: u64,
    /// Values compared against a plain level beside a coded one, and the sum of their differences in 8-bit steps; in stops for BC6H.
    pub compared: u64,
    pub mismatch: f64,
    /// The same for BC7's alpha alone, its colour being compared only where the plain level's alpha is 16 or more.
    pub alpha_compared: u64,
    pub alpha_mismatch: f64,
    /// Preset slots, pair symbols and quadrant symbols the fit gave a value.
    pub slots: u32,
    pub pairs: u32,
    pub quadrants: u32,
}

impl Summary {
    /// Adds what `other` counted of the same mode.
    fn add(&mut self, other: &Self) {
        self.textures += other.textures;
        self.bodies += other.bodies;
        self.blocks += other.blocks;
        self.unknown += other.unknown;
        self.compared += other.compared;
        self.mismatch += other.mismatch;
        self.alpha_compared += other.alpha_compared;
        self.alpha_mismatch += other.alpha_mismatch;
    }

    /// The mean difference from the plain levels, in 8-bit steps, or in stops for BC6H; `None` where no coded body has one beside it.
    pub fn mean_mismatch(&self) -> Option<f64> {
        (self.compared > 0).then(|| self.mismatch / self.compared as f64)
    }

    /// The mean difference of BC7's alpha from the plain levels; `None` for another mode or where nothing was compared.
    pub fn mean_alpha_mismatch(&self) -> Option<f64> {
        (self.alpha_compared > 0).then(|| self.alpha_mismatch / self.alpha_compared as f64)
    }
}

/// A level's pixels, a plane of `width * height` values for each channel; NaN where a block is not known.
struct Pixels {
    width: usize,
    height: usize,
    channels: Vec<Vec<f32>>,
}

struct Coded<'a> {
    lanes: &'a [Vec<u16>],
    at: &'a [u32],
    chains: Chains,
}

struct Level<'a> {
    /// Its size in blocks.
    width: usize,
    height: usize,
    plain: Option<Pixels>,
    coded: Option<Coded<'a>>,
    /// A coded level drawn through the tables as they stand.
    drawn: Option<Pixels>,
}

/// The eight values of an alpha half.
fn alpha_palette(a0: u16, a1: u16) -> [f32; 8] {
    let (a, b) = (f32::from(a0), f32::from(a1));
    std::array::from_fn(|j| match j {
        0 => a,
        1 => b,
        _ if a0 > a1 => ((8 - j) as f32 * a + (j - 1) as f32 * b) / 7.0,
        6 => 0.0,
        7 => 255.0,
        _ => ((6 - j) as f32 * a + (j - 1) as f32 * b) / 5.0,
    })
}

/// A 5- or 6-bit colour field as 8 bits.
fn expand(value: u16, bits: u32) -> f32 {
    let value = u32::from(value);
    let wide = match bits {
        5 => value << 3 | value >> 2,
        6 => value << 2 | value >> 4,
        _ => value,
    };
    wide as f32
}

/// The four colours of a colour half; `four` where its blocks are never the three-colour kind.
fn colour_palette(fields: &[u16], four: bool) -> [[f32; 3]; 4] {
    let colour = |at: usize| -> [f32; 3] {
        [
            expand(fields[at], 5),
            expand(fields[at + 1], 6),
            expand(fields[at + 2], 5),
        ]
    };
    let word = |at: usize| {
        u32::from(fields[at]) << 11 | u32::from(fields[at + 1]) << 5 | u32::from(fields[at + 2])
    };
    let (a, b) = (colour(0), colour(3));
    let mix =
        |of_a: f32, of_b: f32| -> [f32; 3] { std::array::from_fn(|c| of_a * a[c] + of_b * b[c]) };
    if four || word(0) > word(3) {
        [a, b, mix(2.0 / 3.0, 1.0 / 3.0), mix(1.0 / 3.0, 2.0 / 3.0)]
    } else {
        [a, b, mix(0.5, 0.5), [0.0; 3]]
    }
}

/// A plain level's blocks as pixels.
fn plain_pixels(
    kind: u8,
    mode: &Mode,
    blocks: &[u8],
    width: usize,
    height: usize,
) -> Option<Pixels> {
    let size = mode.block_bytes();
    if width == 0 || blocks.len() != width * height * size {
        return None;
    }
    let (w, h) = (4 * width, 4 * height);
    let mut channels = vec![vec![0f32; w * h]; mode.channels()];
    for (i, block) in blocks.chunks(size).enumerate() {
        let origin = 4 * (i / width) * w + 4 * (i % width);
        let mut channel = 0;
        for plane in mode.planes {
            let half = &block[plane.at..plane.at + 8];
            match plane.half {
                Half::Alpha => {
                    let palette = alpha_palette(half[0].into(), half[1].into());
                    let word = (half[2..].iter().rev())
                        .fold(0u64, |word, &byte| word << 8 | u64::from(byte));
                    for pixel in 0..16 {
                        let at = origin + (pixel / 4) * w + pixel % 4;
                        channels[channel][at] = palette[((word >> (3 * pixel)) & 7) as usize];
                    }
                }
                Half::Colour => {
                    let colours = [
                        u16::from_le_bytes([half[0], half[1]]),
                        u16::from_le_bytes([half[2], half[3]]),
                    ];
                    let fields: [u16; 6] = std::array::from_fn(|k| match k % 3 {
                        0 => colours[k / 3] >> 11,
                        1 => (colours[k / 3] >> 5) & 63,
                        _ => colours[k / 3] & 31,
                    });
                    let palette = colour_palette(&fields, kind != 1);
                    let word = u32::from_le_bytes([half[4], half[5], half[6], half[7]]);
                    for pixel in 0..16 {
                        let at = origin + (pixel / 4) * w + pixel % 4;
                        let colour = palette[((word >> (2 * pixel)) & 3) as usize];
                        for (c, value) in colour.into_iter().enumerate() {
                            channels[channel + c][at] = value;
                        }
                    }
                }
            }
            channel += plane.channels();
        }
    }
    Some(Pixels {
        width: w,
        height: h,
        channels,
    })
}

/// `texture`'s levels: the plain ones as pixels, the coded ones as their chains.
fn prepare<'a>(texture: &'a TextureIn, mode: &Mode) -> Vec<Level<'a>> {
    (texture.levels.iter())
        .map(|level| {
            let (width, height) = (level.width, level.height);
            let mut out = Level {
                width,
                height,
                plain: None,
                coded: None,
                drawn: None,
            };
            match &level.data {
                LevelData::Plain(blocks) => {
                    out.plain = plain_pixels(texture.kind, mode, blocks, width, height);
                }
                LevelData::Coded { lanes, at } if lanes.len() >= 12 => {
                    out.coded = (planes::chains(mode, lanes).ok())
                        .filter(|chains| chains.blocks == at.len())
                        .map(|chains| Coded { lanes, at, chains });
                }
                _ => {}
            }
            out
        })
        .collect()
}

/// The fields of `plane` in block `block`; `None` where its preset slot's are not known.
fn plane_fields(
    mode: &Mode,
    plane: &Plane,
    chains: &Chains,
    block: usize,
    tables: &ModeTables,
) -> Option<[u16; 6]> {
    let mut out = [0u16; 6];
    for (k, value) in out.iter_mut().enumerate().take(plane.fields.len()) {
        *value = chains.field(mode, block, plane.base + k, plane.bits(k), &tables.preset)?;
    }
    Some(out)
}

/// The values a half's indices name, each as its channels, and how many there are.
fn palette(kind: u8, plane: &Plane, fields: &[u16; 6]) -> ([[f32; 3]; 8], usize) {
    match plane.half {
        Half::Alpha => {
            let values = alpha_palette(fields[0], fields[1]);
            (std::array::from_fn(|j| [values[j], 0.0, 0.0]), 8)
        }
        Half::Colour => {
            let colours = colour_palette(fields, kind != 1);
            (
                std::array::from_fn(|j| colours.get(j).copied().unwrap_or_default()),
                4,
            )
        }
    }
}

/// A coded level's pixels through `tables`, its blocks inside the level, and how many of them are not known.
fn draw(
    kind: u8,
    mode: &Mode,
    level: &Level<'_>,
    tables: &ModeTables,
) -> Option<(Pixels, u64, u64)> {
    let coded = level.coded.as_ref()?;
    let (w, h) = (4 * level.width, 4 * level.height);
    let mut channels = vec![vec![f32::NAN; w * h]; mode.channels()];
    let (mut blocks, mut unknown) = (0, 0);
    for block in 0..coded.chains.blocks {
        let place = coded.at[block] as usize;
        if place >= level.width * level.height {
            continue;
        }
        let origin = 4 * (place / level.width) * w + 4 * (place % level.width);
        blocks += 1;
        let (mut known, mut channel) = (true, 0);
        for plane in mode.planes {
            let fields = plane_fields(mode, plane, &coded.chains, block, tables);
            let indices = planes::indices(mode, plane, coded.lanes, block, tables);
            match fields {
                Some(fields) if indices.iter().all(Option::is_some) => {
                    let (palette, _) = palette(kind, plane, &fields);
                    for (pixel, index) in indices.iter().enumerate() {
                        let at = origin + (pixel / 4) * w + pixel % 4;
                        let entry = palette[usize::from(index.unwrap_or(0))];
                        for (c, value) in entry.into_iter().enumerate().take(plane.channels()) {
                            channels[channel + c][at] = value;
                        }
                    }
                }
                _ => known = false,
            }
            channel += plane.channels();
        }
        unknown += u64::from(!known);
    }
    let pixels = Pixels {
        width: w,
        height: h,
        channels,
    };
    Some((pixels, blocks, unknown))
}

/// `source`, twice `width` by twice `height`, with each 2 by 2 pixels averaged.
fn box_down(source: &[f32], width: usize, height: usize) -> Vec<f32> {
    let wide = 2 * width;
    (0..width * height)
        .map(|i| {
            let at = 2 * (i / width) * wide + 2 * (i % width);
            (source[at] + source[at + 1] + source[at + wide] + source[at + wide + 1]) / 4.0
        })
        .collect()
}

/// `source`, half `width` by half `height`, with each pixel repeated 2 by 2.
fn nearest_up(source: &[f32], width: usize, height: usize) -> Vec<f32> {
    let narrow = width / 2;
    (0..width * height)
        .map(|i| source[(i / width / 2) * narrow + (i % width) / 2])
        .collect()
}

/// `source`, half `width` by half `height`, smoothed up to each pixel.
fn bilinear_up(source: &[f32], width: usize, height: usize) -> Vec<f32> {
    let (w, h) = ((width / 2) as isize, (height / 2) as isize);
    let get = |x: isize, y: isize| source[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize];
    (0..width * height)
        .map(|i| {
            let fx = ((i % width) as f32 + 0.5) / 2.0 - 0.5;
            let fy = ((i / width) as f32 + 0.5) / 2.0 - 0.5;
            let (ax, ay) = (fx - fx.floor(), fy - fy.floor());
            let (x, y) = (fx.floor() as isize, fy.floor() as isize);
            (1.0 - ay) * ((1.0 - ax) * get(x, y) + ax * get(x + 1, y))
                + ay * ((1.0 - ax) * get(x, y + 1) + ax * get(x + 1, y + 1))
        })
        .collect()
}

/// Each pixel on a block's edge as the mean of the known pixels across that edge; NaN elsewhere.
fn edges(image: &[f32], width: usize, height: usize) -> Vec<f32> {
    (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            let across = [
                (x.is_multiple_of(4) && x > 0).then(|| i - 1),
                (x % 4 == 3 && x + 1 < width).then(|| i + 1),
                (y.is_multiple_of(4) && y > 0).then(|| i - width),
                (y % 4 == 3 && y + 1 < height).then(|| i + width),
            ];
            let (sum, count) = (across.into_iter().flatten())
                .map(|at| image[at])
                .filter(|value| !value.is_nan())
                .fold((0f32, 0u32), |(sum, count), value| (sum + value, count + 1));
            if count == 0 {
                f32::NAN
            } else {
                sum / count as f32
            }
        })
        .collect()
}

/// What a coded level's pixels are held against, a plane for each channel.
struct Target {
    /// A value for each pixel; else one for each 2 by 2 pixels.
    per_pixel: bool,
    values: Vec<Vec<f32>>,
    /// The level below smoothed up, which stands for each pixel where only 2 by 2 values are known.
    soft: Vec<Vec<f32>>,
    weight: u32,
    /// The pixels across a block's edges, drawn from the level itself.
    edge: bool,
}

/// The pixels `level` shows: a plain level's, or in a second pass a coded level's as drawn.
fn seen<'a>(level: &'a Level<'_>, second: bool) -> Option<&'a Pixels> {
    (level.plain.as_ref()).or(level.drawn.as_ref().filter(|_| second))
}

fn targets(levels: &[Level<'_>], at: usize, second: bool) -> Vec<Target> {
    let level = &levels[at];
    let (w, h) = (4 * level.width, 4 * level.height);
    let mut out = Vec::new();
    let above = (at.checked_sub(1))
        .and_then(|above| seen(&levels[above], second))
        .filter(|above| above.width == 2 * w && above.height == 2 * h);
    if let Some(above) = above {
        out.push(Target {
            per_pixel: true,
            values: (above.channels.iter().map(|c| box_down(c, w, h))).collect(),
            soft: Vec::new(),
            weight: 4,
            edge: false,
        });
    }
    let below = (levels.get(at + 1))
        .and_then(|below| seen(below, second))
        .filter(|below| 2 * below.width == w && 2 * below.height == h);
    if let Some(below) = below {
        out.push(Target {
            per_pixel: false,
            values: (below.channels.iter().map(|c| nearest_up(c, w, h))).collect(),
            soft: (below.channels.iter().map(|c| bilinear_up(c, w, h))).collect(),
            weight: 1,
            edge: false,
        });
    }
    if let Some(own) = level.drawn.as_ref().filter(|_| second) {
        out.push(Target {
            per_pixel: true,
            values: (own.channels.iter().map(|c| edges(c, w, h))).collect(),
            soft: Vec::new(),
            weight: 1,
            edge: true,
        });
    }
    out
}

/// One pass's votes for one mode.
struct Votes {
    /// For each slot, field and value: blocks whose pixels of a pure index say the field has that value.
    aware: Vec<u32>,
    /// The same from a block's brightest and darkest pixels, where its indices are not known yet.
    boot: Vec<u32>,
    /// For each pair symbol and pattern.
    pairs: Vec<u32>,
    /// For each quadrant symbol and pattern.
    quadrants: Vec<u32>,
}

impl Votes {
    fn new() -> Self {
        Self {
            aware: vec![0; 256 * FIELDS * 256],
            boot: vec![0; 256 * FIELDS * 256],
            pairs: vec![0; 64 * 64],
            quadrants: vec![0; 256 * 256],
        }
    }

    /// Adds `other`'s votes.
    fn add(&mut self, other: &Self) {
        let sum = |into: &mut [u32], more: &[u32]| {
            for (into, more) in into.iter_mut().zip(more) {
                *into += more;
            }
        };
        sum(&mut self.aware, &other.aware);
        sum(&mut self.boot, &other.boot);
        sum(&mut self.pairs, &other.pairs);
        sum(&mut self.quadrants, &other.quadrants);
    }
}

/// The votes of one mip chain's coded levels.
fn vote(
    kind: u8,
    mode: &Mode,
    levels: &[Level<'_>],
    tables: &ModeTables,
    votes: &mut Votes,
    second: bool,
) {
    for (li, level) in levels.iter().enumerate() {
        let Some(coded) = &level.coded else {
            continue;
        };
        let targets = targets(levels, li, second);
        if targets.is_empty() {
            continue;
        }
        let (w, cells) = (4 * level.width, level.width * level.height);
        let mut channel = 0;
        for plane in mode.planes {
            let nc = plane.channels();
            for block in 0..coded.chains.blocks {
                let place = coded.at[block] as usize;
                if place >= cells {
                    continue;
                }
                let origin = 4 * (place / level.width) * w + 4 * (place % level.width);
                let at = |pixel: usize| origin + (pixel / 4) * w + pixel % 4;
                let root = coded.chains.roots[block].map(usize::from);
                let fields = plane_fields(mode, plane, &coded.chains, block, tables);
                // BC1's quadrant table is built in.
                let fitted_indices = !(kind == 1 && plane.half == Half::Colour);
                if let Some(fields) = fields.filter(|_| fitted_indices) {
                    let (palette, entries) = palette(kind, plane, &fields);
                    let spread = (0..nc)
                        .map(|c| (palette[0][c] - palette[1][c]).abs())
                        .fold(0.0, f32::max);
                    // The pixels across a block's edges are too few to name a pattern.
                    for target in (targets.iter()).filter(|target| spread >= 16.0 && !target.edge) {
                        let source = if target.per_pixel {
                            &target.values
                        } else {
                            &target.soft
                        };
                        // A level drawn through the tables has no pixel where a block is not known.
                        let seen = |pixel: usize| {
                            (0..nc).all(|c| !source[channel + c][at(pixel)].is_nan())
                        };
                        // The palette entry nearest a pixel's target.
                        let near = |pixel: usize| -> u32 {
                            let mut best = (f32::INFINITY, 0);
                            for (j, entry) in palette.iter().enumerate().take(entries) {
                                let far: f32 = (0..nc)
                                    .map(|c| (entry[c] - source[channel + c][at(pixel)]).abs())
                                    .sum();
                                if far < best.0 {
                                    best = (far, j as u32);
                                }
                            }
                            best.1
                        };
                        let weight = target.weight * if root.is_none() { 2 } else { 1 };
                        for (k, &(lane, sub)) in plane.indices.iter().enumerate() {
                            let symbol =
                                usize::from(coded.lanes[lane][mode.elements * block + sub]);
                            let whole = match plane.half {
                                Half::Alpha => seen(2 * k) && seen(2 * k + 1),
                                Half::Colour => {
                                    let (left, top) = QUADRANTS[k];
                                    let q = 4 * top + left;
                                    [q, q + 1, q + 4, q + 5].into_iter().all(seen)
                                }
                            };
                            if !whole {
                                continue;
                            }
                            match plane.half {
                                Half::Alpha => {
                                    let pattern = near(2 * k) | near(2 * k + 1) << 3;
                                    votes.pairs[(symbol & 63) * 64 + pattern as usize] += weight;
                                }
                                Half::Colour => {
                                    let (left, top) = QUADRANTS[k];
                                    let q = 4 * top + left;
                                    let pattern = near(q)
                                        | near(q + 1) << 2
                                        | near(q + 4) << 4
                                        | near(q + 5) << 6;
                                    votes.quadrants[(symbol & 255) * 256 + pattern as usize] +=
                                        weight;
                                }
                            }
                        }
                    }
                }
                let Some(slot) = root.filter(|&slot| !planes::built_in(kind, slot)) else {
                    continue;
                };
                let indices = planes::indices(mode, plane, coded.lanes, block, tables);
                let aware = indices.iter().all(Option::is_some);
                for target in &targets {
                    let value = |c: usize, pixel: usize| target.values[channel + c][at(pixel)];
                    let mut estimates: [Option<[f32; 3]>; 2] = [None, None];
                    if aware {
                        for (e, estimate) in estimates.iter_mut().enumerate() {
                            let pure = |pixel: usize| indices[pixel] == Some(e as u8);
                            // A 2 by 2 value speaks for an endpoint only where its four pixels all take it.
                            let speaks = |pixel: usize| {
                                let whole = target.per_pixel
                                    || (pure(pixel + 1) && pure(pixel + 4) && pure(pixel + 5));
                                pure(pixel) && whole && !value(0, pixel).is_nan()
                            };
                            let pixels: &[usize] = if target.per_pixel {
                                &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
                            } else {
                                &[0, 2, 8, 10]
                            };
                            let (mut sum, mut count) = ([0f32; 3], 0u32);
                            for &pixel in pixels.iter().filter(|&&pixel| speaks(pixel)) {
                                for (c, total) in sum.iter_mut().enumerate().take(nc) {
                                    *total += value(c, pixel);
                                }
                                count += 1;
                            }
                            if count > 0 {
                                *estimate = Some(sum.map(|sum| sum / count as f32));
                            }
                        }
                    } else if !target.edge {
                        // The indices are not known yet: the block's brightest and darkest pixels stand for its endpoints.
                        let light = |pixel: usize| (0..nc).map(|c| value(c, pixel)).sum::<f32>();
                        if (0..16).all(|pixel| !light(pixel).is_nan()) {
                            let pick = |brighter: bool| {
                                let best = (0..16).fold(0, |best, pixel| {
                                    let beats = if brighter {
                                        light(pixel) > light(best)
                                    } else {
                                        light(pixel) < light(best)
                                    };
                                    if beats {
                                        pixel
                                    } else {
                                        best
                                    }
                                });
                                std::array::from_fn(|c| if c < nc { value(c, best) } else { 0.0 })
                            };
                            estimates = [Some(pick(true)), Some(pick(false))];
                        }
                    }
                    let into = if aware {
                        &mut votes.aware
                    } else {
                        &mut votes.boot
                    };
                    for (e, estimate) in estimates.iter().enumerate() {
                        let Some(estimate) = estimate else {
                            continue;
                        };
                        for (c, &guess) in estimate.iter().enumerate().take(nc) {
                            let k = e * nc + c;
                            let field = plane.base + k;
                            let most = (1u32 << plane.bits(k)) - 1;
                            let scaled = (guess * most as f32 / 255.0).round();
                            let scaled = scaled.clamp(0.0, most as f32) as u32;
                            let offset =
                                u32::from(coded.chains.offsets[block * mode.fields() + field]);
                            let voted = (scaled + most + 1 - offset) & most;
                            into[(slot * FIELDS + field) * 256 + voted as usize] += 1;
                        }
                    }
                }
            }
            channel += nc;
        }
    }
}

/// The votes cast and the most voted value, a value's neighbours on either side counting for it.
fn peak(votes: &[u32]) -> (u32, usize) {
    let n = votes.len();
    let smooth = |v: usize| votes[(v + n - 1) % n] + 2 * votes[v] + votes[(v + 1) % n];
    let best = (0..n).fold(0, |best, v| if smooth(v) > smooth(best) { v } else { best });
    (votes.iter().sum(), best)
}

/// Each symbol's pattern, one symbol to a pattern, the most voted first; a symbol `known` already keeps its pattern.
fn assign(votes: &[u32], patterns: usize, known: Vec<Option<usize>>) -> Vec<Option<usize>> {
    let symbols = known.len();
    let mut candidates: Vec<(u32, usize, usize)> = (0..symbols * patterns)
        .map(|at| (votes[at], at / patterns, at % patterns))
        .filter(|candidate| candidate.0 >= MIN_VOTES)
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let (mut table, mut taken) = (known, vec![false; patterns]);
    for pattern in table.iter().flatten() {
        taken[*pattern] = true;
    }
    for (_, symbol, pattern) in candidates {
        if table[symbol].is_none() && !taken[pattern] {
            table[symbol] = Some(pattern);
            taken[pattern] = true;
        }
    }
    table
}

/// Takes a pass's votes into `tables`; a second pass fills only the preset fields still not known.
fn solve(kind: u8, mode: &Mode, votes: &Votes, tables: &mut ModeTables, second: bool) {
    for slot in (0..256).filter(|&slot| !planes::built_in(kind, slot)) {
        for plane in mode.planes {
            for k in 0..plane.fields.len() {
                let field = plane.base + k;
                let known = tables.preset[slot][field] != UNKNOWN;
                if second && known {
                    continue;
                }
                let at = (slot * FIELDS + field) * 256;
                let bins = 1usize << plane.bits(k);
                let (cast, best) = peak(&votes.aware[at..at + bins]);
                if cast >= MIN_VOTES {
                    tables.preset[slot][field] = best as u16;
                } else if !known {
                    let (cast, best) = peak(&votes.boot[at..at + bins]);
                    if cast >= MIN_VOTES {
                        tables.preset[slot][field] = best as u16;
                    }
                }
            }
        }
    }
    // A first pass names every symbol anew; a second pass only those no first pass named.
    let holds = |half: Half| mode.planes.iter().any(|plane| plane.half == half);
    if holds(Half::Alpha) {
        let known = (tables.pairs.iter())
            .map(|&pair| (second && pair != UNKNOWN_PAIR).then_some(usize::from(pair)))
            .collect();
        for (symbol, pattern) in assign(&votes.pairs, 64, known).into_iter().enumerate() {
            tables.pairs[symbol] = pattern.map_or(UNKNOWN_PAIR, |pattern| pattern as u8);
        }
    }
    if kind != 1 && holds(Half::Colour) {
        let known = (tables.quadrants.iter())
            .map(|&quadrant| (second && quadrant != UNKNOWN).then_some(usize::from(quadrant)))
            .collect();
        for (symbol, pattern) in assign(&votes.quadrants, 256, known).into_iter().enumerate() {
            tables.quadrants[symbol] = pattern.map_or(UNKNOWN, |pattern| pattern as u16);
        }
    }
}

/// Adds to `summary` how one mip chain's coded levels, drawn through `tables`, meet the plain levels beside them.
fn score(kind: u8, mode: &Mode, levels: &[Level<'_>], tables: &ModeTables, summary: &mut Summary) {
    summary.textures += 1;
    for (li, level) in levels.iter().enumerate() {
        let Some((drawn, blocks, unknown)) = draw(kind, mode, level, tables) else {
            continue;
        };
        summary.bodies += 1;
        summary.blocks += blocks;
        summary.unknown += unknown;
        let (w, h) = (drawn.width, drawn.height);
        let below = (levels.get(li + 1))
            .and_then(|below| below.plain.as_ref())
            .filter(|below| 2 * below.width == w && 2 * below.height == h);
        let above = (li.checked_sub(1))
            .and_then(|above| levels[above].plain.as_ref())
            .filter(|above| above.width == 2 * w && above.height == 2 * h);
        for (c, own) in drawn.channels.iter().enumerate() {
            let (mine, theirs) = match (below, above) {
                (Some(below), _) => (box_down(own, w / 2, h / 2), below.channels[c].clone()),
                (None, Some(above)) => (own.clone(), box_down(&above.channels[c], w, h)),
                (None, None) => continue,
            };
            for (mine, theirs) in mine.iter().zip(&theirs).filter(|(mine, _)| !mine.is_nan()) {
                summary.mismatch += f64::from((mine - theirs).abs());
                summary.compared += 1;
            }
        }
    }
}

/// What one worker gathers in one pass: tallies of a fixed size, whatever it reads.
struct Part {
    votes: Vec<Option<Votes>>,
    bptc_votes: Option<bptc_fit::Votes>,
    census: Option<bptc_fit::Census>,
    hdr_votes: Option<bc6h_fit::Votes>,
    summaries: Vec<Summary>,
    bptc: Summary,
    hdr: Summary,
}

/// What one pass does with each mode's textures.
#[derive(Clone, Copy)]
struct Doing {
    /// What the modes of two halves do: nothing, else a first pass, a second pass or their score.
    halves: bool,
    second: bool,
    last: bool,
    bptc: BptcPass,
    hdr: BptcPass,
}

impl Doing {
    fn of(pass: usize, passes: usize) -> Self {
        let own = FIRST_PASSES + SECOND_PASSES + 1;
        let last = pass + 1 == own;
        Self {
            halves: pass < own,
            second: pass >= FIRST_PASSES,
            last,
            // BC7's votes need each rank's mode, which the first pass counts.
            bptc: match pass {
                0 => BptcPass::Census,
                _ if pass <= FIRST_PASSES => BptcPass::Vote,
                _ if pass <= FIRST_PASSES + SECOND_PASSES => BptcPass::Fill,
                _ if pass == own => BptcPass::Score,
                _ => BptcPass::Rest,
            },
            hdr: match pass {
                _ if pass + 1 == passes => BptcPass::Score,
                _ if pass < HDR_PASSES => BptcPass::Vote,
                _ if pass < HDR_PASSES + HDR_FILLS => BptcPass::Fill,
                _ => BptcPass::Rest,
            },
        }
    }

    /// Whether the pass does anything with a texture that holds chains of the codec kinds whose
    /// bits `kinds` has; bit 0 says a BC7 chain of it has something a pass that fills votes from,
    /// and bit 2 the same of a BC6H chain.
    fn reads(&self, kinds: u8) -> bool {
        let holds = |kind: u8| kinds >> kind & 1 != 0;
        let reads = |pass: BptcPass, marked: u8| match pass {
            BptcPass::Rest => false,
            BptcPass::Fill => holds(marked),
            _ => true,
        };
        let (bptc, hdr) = (reads(self.bptc, 0), reads(self.hdr, 2));
        (holds(6) && hdr)
            || (holds(7) && bptc)
            || ([1, 3, 4, 5].into_iter().any(holds) && self.halves)
    }
}

/// What a pass does with BC7's textures, or BC6H's: the census of lane 15's ranks first, for BC7,
/// then its votes, then the votes that fill what is not known from the levels with no plain
/// level beside them, its score last.
#[derive(Clone, Copy, PartialEq)]
enum BptcPass {
    Census,
    Vote,
    Fill,
    Rest,
    Score,
}

impl Part {
    fn new() -> Self {
        Self {
            votes: KINDS.iter().map(|_| None).collect(),
            bptc_votes: None,
            census: None,
            hdr_votes: None,
            summaries: vec![Summary::default(); KINDS.len()],
            bptc: Summary::default(),
            hdr: Summary::default(),
        }
    }

    /// Takes one mip chain into the pass's tallies, read through `tables` as they stand, and says
    /// whether it is a chain a pass that fills has something to vote from: for BC7 one with a
    /// coded level that has no plain level beside it and a block that hangs on something not
    /// known, for BC6H one with a block that hangs on something not settled.
    ///
    /// `plain` is BC7's presets as the plain levels left them, in a pass that fills what they did not know.
    fn take(
        &mut self,
        input: &TextureIn,
        tables: &Tables,
        doing: Doing,
        plain: Option<&Presets>,
    ) -> bool {
        let Doing { second, last, .. } = doing;
        if input.kind == 6 {
            match doing.hdr {
                BptcPass::Vote | BptcPass::Fill => {
                    let votes = self.hdr_votes.get_or_insert_with(bc6h_fit::Votes::empty);
                    let fill = doing.hdr == BptcPass::Fill;
                    return bc6h_fit::vote(input, &tables.hdr, votes, fill);
                }
                BptcPass::Score => bc6h_fit::score(input, &tables.hdr, &mut self.hdr),
                BptcPass::Census | BptcPass::Rest => {}
            }
            return false;
        }
        if input.kind == 7 {
            match doing.bptc {
                BptcPass::Census => {
                    let census = self.census.get_or_insert_with(bptc_fit::Census::empty);
                    census.take(input);
                }
                BptcPass::Vote | BptcPass::Fill => {
                    let votes = self.bptc_votes.get_or_insert_with(bptc_fit::Votes::empty);
                    // The last pass against the plain levels marks the textures the next ones fill from.
                    return bptc_fit::vote(input, &tables.bptc, votes, plain, second);
                }
                BptcPass::Score => bptc_fit::score(input, &tables.bptc, &mut self.bptc),
                BptcPass::Rest => {}
            }
            return false;
        }
        let (Some(at), Some(mode)) = (Tables::at(input.kind), planes::mode(input.kind)) else {
            return false;
        };
        if !doing.halves {
            return false;
        }
        let resolved = tables.modes[at].with_built_in(input.kind);
        let mut levels = prepare(input, mode);
        if !levels.iter().any(|level| level.coded.is_some()) {
            return false;
        }
        if last {
            score(
                input.kind,
                mode,
                &levels,
                &resolved,
                &mut self.summaries[at],
            );
            return false;
        }
        if second {
            for level in &mut levels {
                let drawn = draw(input.kind, mode, level, &resolved);
                level.drawn = drawn.map(|(pixels, _, _)| pixels);
            }
        }
        let votes = self.votes[at].get_or_insert_with(Votes::new);
        vote(input.kind, mode, &levels, &resolved, votes, second);
        false
    }

    /// Adds what another worker gathered in the same pass.
    fn add(&mut self, other: Self) {
        for (mine, theirs) in self.votes.iter_mut().zip(other.votes) {
            match (mine.as_mut(), theirs) {
                (Some(mine), Some(theirs)) => mine.add(&theirs),
                (None, theirs) => *mine = theirs,
                (Some(_), None) => {}
            }
        }
        match (self.bptc_votes.as_mut(), other.bptc_votes) {
            (Some(mine), Some(theirs)) => mine.add(&theirs),
            (None, theirs) => self.bptc_votes = theirs,
            (Some(_), None) => {}
        }
        match (self.census.as_mut(), other.census) {
            (Some(mine), Some(theirs)) => mine.add(&theirs),
            (None, theirs) => self.census = theirs,
            (Some(_), None) => {}
        }
        match (self.hdr_votes.as_mut(), other.hdr_votes) {
            (Some(mine), Some(theirs)) => mine.add(&theirs),
            (None, theirs) => self.hdr_votes = theirs,
            (Some(_), None) => {}
        }
        for (mine, theirs) in self.summaries.iter_mut().zip(&other.summaries) {
            mine.add(theirs);
        }
        self.bptc.add(&other.bptc);
        self.hdr.add(&other.hdr);
    }
}

/// Where a fit stands after a pass, as the bytes a later run goes on from.
struct Stood {
    /// The pass to run next.
    pass: usize,
    tables: Tables,
    plain_bptc: Option<Presets>,
    held: Vec<u8>,
    /// What the modes of two halves, BC7 and BC6H counted so far, in that order.
    sums: Vec<Summary>,
}

const STOOD_MAGIC: [u8; 4] = *b"ORBS";

impl Stood {
    fn to_bytes(&self, count: usize, passes: usize) -> Vec<u8> {
        let mut out = STOOD_MAGIC.to_vec();
        for number in [
            u64::from(VERSION),
            passes as u64,
            count as u64,
            self.pass as u64,
        ] {
            out.extend(number.to_le_bytes());
        }
        out.extend(self.tables.to_bytes());
        out.push(u8::from(self.plain_bptc.is_some()));
        match &self.plain_bptc {
            Some(plain) => out.extend(plain.to_bytes()),
            None => out.resize(out.len() + PRESETS_BYTES, 0),
        }
        out.extend(&self.held);
        for sum in &self.sums {
            let numbers = [
                sum.textures,
                sum.bodies,
                sum.blocks,
                sum.unknown,
                sum.compared,
                sum.mismatch.to_bits(),
                sum.alpha_compared,
                sum.alpha_mismatch.to_bits(),
            ];
            out.extend(numbers.iter().flat_map(|number| number.to_le_bytes()));
        }
        out
    }

    /// What [`Self::to_bytes`] wrote for a fit of `count` textures in `passes` passes; `None` for anything else.
    fn from_bytes(bytes: &[u8], count: usize, passes: usize) -> Option<Self> {
        let rest = bytes.strip_prefix(&STOOD_MAGIC)?;
        let (head, rest) = rest.split_at_checked(32)?;
        let head: Vec<u64> = (head.as_chunks::<8>().0.iter())
            .map(|number| u64::from_le_bytes(*number))
            .collect();
        let pass = usize::try_from(head[3])
            .ok()
            .filter(|&pass| pass <= passes)?;
        if head[..3] != [u64::from(VERSION), passes as u64, count as u64] {
            return None;
        }
        let (tables, rest) = rest.split_at_checked(TABLES_BYTES)?;
        let (&has_plain, rest) = rest.split_first()?;
        let (plain, rest) = rest.split_at_checked(PRESETS_BYTES)?;
        let (held, rest) = rest.split_at_checked(count)?;
        let (sums, []) = rest.as_chunks::<64>() else {
            return None;
        };
        let sums = (sums.iter())
            .map(|sum| {
                let number =
                    |at: usize| u64::from_le_bytes(std::array::from_fn(|byte| sum[8 * at + byte]));
                Summary {
                    textures: number(0),
                    bodies: number(1),
                    blocks: number(2),
                    unknown: number(3),
                    compared: number(4),
                    mismatch: f64::from_bits(number(5)),
                    alpha_compared: number(6),
                    alpha_mismatch: f64::from_bits(number(7)),
                    ..Summary::default()
                }
            })
            .collect::<Vec<_>>();
        (sums.len() == KINDS.len() + 2).then_some(())?;
        Some(Self {
            pass,
            tables: Tables::from_bytes(tables)?,
            plain_bptc: match has_plain {
                0 => None,
                _ => Some(Presets::from_bytes(plain)?),
            },
            held: held.to_vec(),
            sums,
        })
    }
}

/// Fits every mode's tables from `count` textures, each read again through `texture`, which
/// gives a texture's mip chains, in every pass that does something with one of its modes;
/// `None` once `stop` says to stop.
///
/// A pass reads its textures on several workers, each of which holds one texture at a time.
pub fn calibrate(
    count: usize,
    texture: &(dyn Fn(usize, bool) -> Vec<TextureIn> + Sync),
    progress: &mut dyn FnMut(u64, u64),
    stop: &(dyn Fn() -> bool + Sync),
) -> Option<(Tables, Vec<Summary>)> {
    calibrate_from(count, texture, progress, stop, None, &mut |_| {})
}

/// [`calibrate`], going on from where an earlier run of the same textures stood (`from`, what
/// that run gave `kept` last), and giving `kept` where it stands after each pass.
pub fn calibrate_from(
    count: usize,
    texture: &(dyn Fn(usize, bool) -> Vec<TextureIn> + Sync),
    progress: &mut dyn FnMut(u64, u64),
    stop: &(dyn Fn() -> bool + Sync),
    from: Option<&[u8]>,
    kept: &mut dyn FnMut(&[u8]),
) -> Option<(Tables, Vec<Summary>)> {
    let mut tables = Tables::empty();
    let mut summaries: Vec<Summary> = (KINDS.iter())
        .map(|&kind| Summary {
            kind,
            ..Summary::default()
        })
        .collect();
    let mut bptc = Summary {
        kind: 7,
        ..Summary::default()
    };
    let mut hdr = Summary {
        kind: 6,
        ..Summary::default()
    };
    let passes = (FIRST_PASSES + SECOND_PASSES).max(HDR_PASSES + HDR_FILLS) + 1;
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .clamp(1, MAX_WORKERS)
        .min(count.max(1));
    let mut told = None;
    // BC7's presets as the last pass against the plain levels left them.
    let mut plain_bptc: Option<Presets> = None;
    // The modes each texture gave chains of, a bit for each codec kind, as the first pass found
    // them: a later pass does not read a texture it does nothing with.
    let held: Vec<AtomicU8> = (0..count).map(|_| AtomicU8::new(0)).collect();
    let mut first = 0;
    if let Some(stood) = from.and_then(|bytes| Stood::from_bytes(bytes, count, passes)) {
        first = stood.pass;
        tables = stood.tables;
        plain_bptc = stood.plain_bptc;
        for (held, was) in held.iter().zip(stood.held) {
            held.store(was, Ordering::Relaxed);
        }
        let (halves, rest) = stood.sums.split_at(KINDS.len());
        for (summary, was) in summaries.iter_mut().zip(halves) {
            summary.add(was);
        }
        bptc.add(&rest[0]);
        hdr.add(&rest[1]);
    }
    for pass in first..passes {
        let doing = Doing::of(pass, passes);
        let second = doing.second;
        if doing.bptc == BptcPass::Fill && plain_bptc.is_none() {
            plain_bptc = Some(tables.bptc.clone());
        }
        let plain = (plain_bptc.as_ref()).filter(|_| doing.bptc == BptcPass::Fill);
        let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let standing = &tables;
        let gathered = std::thread::scope(|scope| {
            let running: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        let mut part = Part::new();
                        loop {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            if index >= count || stop() {
                                break;
                            }
                            let kinds = held[index].load(Ordering::Relaxed);
                            if pass > 0 && !doing.reads(kinds) {
                                done.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            // A pass that fills reads the larger coded levels too.
                            let large = pass > 0
                                && ((kinds >> 6 & 1 != 0 && doing.hdr == BptcPass::Fill)
                                    || (kinds >> 7 & 1 != 0 && doing.bptc == BptcPass::Fill));
                            let inputs = texture(index, large);
                            let kinds =
                                (inputs.iter()).fold(0, |all, input| all | 1 << (input.kind & 7));
                            let left_out = |input: &TextureIn| {
                                let large =
                                    |level: &LevelIn| matches!(level.data, LevelData::Large);
                                input.levels.iter().any(large)
                            };
                            // A level left out for its size may hang on something not known.
                            let mut lone = inputs.iter().any(left_out);
                            if pass == 0 {
                                held[index].store(kinds | u8::from(lone), Ordering::Relaxed);
                            }
                            for input in inputs {
                                lone |= part.take(&input, standing, doing, plain);
                            }
                            // The mark of a BC7 chain, or of a BC6H one. A pass that fills leaves
                            // it only where a level still had something to vote, and so does
                            // each of BC6H's passes, which all look.
                            let (mark, looked) = match kinds >> 6 & 1 {
                                0 => (1, doing.bptc == BptcPass::Fill),
                                _ => (4, true),
                            };
                            match (lone, looked) {
                                (true, _) => held[index].fetch_or(mark, Ordering::Relaxed),
                                (false, true) => held[index].fetch_and(!mark, Ordering::Relaxed),
                                (false, false) => 0,
                            };
                            done.fetch_add(1, Ordering::Relaxed);
                        }
                        part
                    })
                })
                .collect();
            loop {
                let over = running.iter().all(|worker| worker.is_finished());
                let read = done.load(Ordering::Relaxed).min(count.saturating_sub(1));
                let at = (pass * count + read) as u64;
                if told != Some(at) && !stop() {
                    told = Some(at);
                    progress(at, (passes * count) as u64);
                }
                if over {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let mut gathered = Part::new();
            for worker in running {
                match worker.join() {
                    Ok(part) => gathered.add(part),
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            }
            gathered
        });
        if stop() {
            return None;
        }
        for (at, votes) in gathered.votes.iter().enumerate() {
            if let (Some(votes), Some(mode)) = (votes, planes::mode(KINDS[at])) {
                solve(KINDS[at], mode, votes, &mut tables.modes[at], second);
            }
        }
        if let Some(census) = &gathered.census {
            tables.bptc.modes = census.modes();
        }
        if let Some(votes) = &gathered.bptc_votes {
            bptc_fit::solve(votes, &mut tables.bptc, plain);
        }
        if let Some(votes) = &gathered.hdr_votes {
            bc6h_fit::solve(votes, &mut tables.hdr);
        }
        for (summary, more) in summaries.iter_mut().zip(&gathered.summaries) {
            summary.add(more);
        }
        bptc.add(&gathered.bptc);
        hdr.add(&gathered.hdr);
        if pass + 1 < passes {
            let stood = Stood {
                pass: pass + 1,
                tables: tables.clone(),
                plain_bptc: plain_bptc.clone(),
                held: (held.iter().map(|held| held.load(Ordering::Relaxed))).collect(),
                sums: (summaries.iter().copied()).chain([bptc, hdr]).collect(),
            };
            kept(&stood.to_bytes(count, passes));
        }
    }
    for (summary, fitted) in summaries.iter_mut().zip(&tables.modes) {
        let solved = |fields: &&[u16; FIELDS]| fields.iter().any(|&value| value != UNKNOWN);
        summary.slots = fitted.preset.iter().filter(solved).count() as u32;
        summary.pairs = (fitted.pairs.iter().filter(|&&pair| pair != UNKNOWN_PAIR)).count() as u32;
        summary.quadrants = (fitted.quadrants.iter().filter(|&&q| q != UNKNOWN)).count() as u32;
    }
    (hdr.slots, hdr.pairs) = bc6h_fit::solved(&tables.hdr);
    (bptc.slots, bptc.pairs) = bptc_fit::solved(&tables.bptc);
    summaries.push(hdr);
    summaries.push(bptc);
    Some((tables, summaries))
}

#[cfg(test)]
#[path = "calibrate_tests.rs"]
pub(in crate::gta6) mod tests;
