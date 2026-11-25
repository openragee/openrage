//! A pack's side of fitting the BCPack coder's tables: which of its textures a calibration reads,
//! each as the mip chains it votes on, and the packs of a folder calibrated together.

use std::fs::File;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::{read_at, TexturePack, TtpfError, KIND_STORED, KIND_ZLIB};
use crate::codec::texture::DxgiFormat;
use crate::gta6::bcpack::calibrate::{self as fit, LevelData, LevelIn, Summary, TextureIn};
use crate::gta6::bcpack::{self, BcPackError, Tables};
use crate::gta6::tiling::{untile, Tile};

/// A BC1 body's first bytes, which say whether it reads a preset slot with no built-in value.
const HEAD: u64 = 4096;
/// The blocks of the largest coded level a calibration reads: a fit wants many bodies, not large
/// ones, and this bounds what one texture takes in memory.
const MAX_CODED_BLOCKS: usize = 1 << 16;
/// The blocks of the largest plain level it reads: the level above the largest coded one.
const MAX_PLAIN_BLOCKS: usize = 4 * MAX_CODED_BLOCKS;
/// The blocks of the largest coded level a pass that fills reads, of BC6H and BC7: such a pass
/// reads few textures, so it can hold larger ones.
const LARGE_CODED_BLOCKS: usize = 2 * MAX_CODED_BLOCKS;

/// What calibrating a set of packs gave.
pub struct Calibrated {
    pub tables: Tables,
    /// What was found for each mode.
    pub summaries: Vec<Summary>,
    /// The key of each pack read, which a calibration file lists.
    pub members: Vec<u64>,
}

/// Where each element of a level of `size`, stored in `len` bytes of tiles, lies in the level's
/// linear rows; `u32::MAX` for an element outside the level.
fn places(len: usize, element: usize, size: [usize; 3], tile: &Tile) -> Option<Vec<u32>> {
    let mut numbered = vec![0u8; len];
    for (number, bytes) in numbered.chunks_mut(element).enumerate() {
        bytes
            .get_mut(..4)?
            .copy_from_slice(&(number as u32).to_le_bytes());
    }
    let linear = untile(&numbered, 0, size, tile)?;
    let mut at = vec![u32::MAX; len / element];
    for (place, bytes) in linear.chunks(element).enumerate() {
        let number = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        *at.get_mut(number as usize)? = place as u32;
    }
    Some(at)
}

impl TexturePack {
    /// Draws the bodies that do not decode exactly through `tables` from now on, where it covers them; `None` refuses them again.
    pub fn set_calibration(&mut self, tables: Option<Arc<Tables>>) {
        self.calibration = tables;
    }

    /// Whether a calibration is set.
    pub fn is_calibrated(&self) -> bool {
        self.calibration.is_some()
    }

    /// The pack's key in a calibration file: its file's name and length.
    pub fn member_key(&self) -> u64 {
        fit::member_key(&self.path, self.on_disk)
    }

    /// The BC number of texture `index`'s blocks, where a calibration fits that mode's tables.
    fn fitted_kind(&self, index: usize) -> Option<u8> {
        use DxgiFormat as D;
        match self.entries.get(index)?.format()?.dxgi() {
            D::Bc1Unorm | D::Bc1UnormSrgb => Some(1),
            D::Bc3Unorm | D::Bc3UnormSrgb => Some(3),
            D::Bc4Unorm => Some(4),
            D::Bc5Unorm => Some(5),
            // No signed texture has a plain level beside a coded one: one votes only what its blocks say without a target.
            D::Bc6hUf16 | D::Bc6hSf16 => Some(6),
            D::Bc7Unorm | D::Bc7UnormSrgb => Some(7),
            _ => None,
        }
    }

    /// Whether the body of codec kind 1 in `row` reads a preset slot with no built-in value.
    fn reads_unknown_slot(&self, file: &mut File, row: usize) -> bool {
        let (Ok(len), Ok((this, next))) = (self.stored_len(row), self.file_span(row)) else {
            return false;
        };
        next.offset <= self.on_disk
            && read_at(file, &self.path, this.offset, len.min(HEAD))
                .is_ok_and(|head| bcpack::bc1_reads_unknown_slot(&head, self.size(row)))
    }

    /// The textures a calibration reads: each with a body of codec kind 3, 4, 5, 6 or 7, or one of
    /// kind 1 that reads a preset slot with no built-in value.
    pub fn calibration_textures(&self, stop: &dyn Fn() -> bool) -> Vec<usize> {
        let Ok(mut file) = File::open(&self.path) else {
            return Vec::new();
        };
        let mut wanted = Vec::new();
        for index in 0..self.entries.len() {
            if stop() {
                break;
            }
            let Some(kind) = self.fitted_kind(index) else {
                continue;
            };
            let mut coded = (self.rows_of(index).into_iter().flatten())
                .filter(|&row| self.rows[row].kind == kind && self.replaced_row(row).is_none());
            let reads = match kind {
                1 => coded.any(|row| self.reads_unknown_slot(&mut file, row)),
                _ => coded.next().is_some(),
            };
            if reads {
                wanted.push(index);
            }
        }
        wanted
    }

    /// The mip chains texture `index` gives a calibration, one for each slice; 0 for a shape it does not read.
    pub fn calibration_slices(&self, index: usize) -> usize {
        match self.layout_of(index) {
            Ok(plan) if plan.layout.depth.is_none() => plan.layout.slices as usize,
            _ => 0,
        }
    }

    /// Slice `slice` of texture `index` as a calibration reads it: a level is plain where its
    /// rows decode exactly, and coded where one row holds it alone and does not.
    ///
    /// Only the levels of at most [`MAX_PLAIN_BLOCKS`] blocks are read, the coded ones of at most
    /// [`MAX_CODED_BLOCKS`], and only the rows that hold them; `large` reads the coded ones up to
    /// [`LARGE_CODED_BLOCKS`], which are otherwise said to be left out for their size.
    pub fn calibration_input(&self, index: usize, slice: usize, large: bool) -> Option<TextureIn> {
        let coded_blocks = if large {
            LARGE_CODED_BLOCKS
        } else {
            MAX_CODED_BLOCKS
        };
        let (kind, plan) = (self.fitted_kind(index)?, self.layout_of(index).ok()?);
        let layout = &plan.layout;
        let slice = slice as u64;
        if layout.depth.is_some() || slice >= layout.slices {
            return None;
        }
        let mut file = File::open(&self.path).ok()?;
        let element = layout.format.element_bytes();
        // Where each level of the slice lies in the texture's memory, for the levels small enough to read.
        let spans: Vec<Option<(usize, usize)>> = (layout.levels.iter())
            .map(|level| {
                let (start, len) = (level.from(), level.len);
                let base = if layout.piece_major {
                    layout.slices * start + slice * len
                } else {
                    slice * layout.unit + start
                };
                let small = level.width * level.height <= MAX_PLAIN_BLOCKS;
                small.then_some((base as usize, (base + len) as usize))
            })
            .collect();
        let low = spans.iter().flatten().map(|span| span.0).min()?;
        let high = spans.iter().flatten().map(|span| span.1).max()?;
        let mut memory = vec![0u8; high - low];
        // The ranges of the texture's memory read exactly, joined where they touch, and each coded row's lanes.
        let mut exact: Vec<(usize, usize)> = Vec::new();
        let mut coded: Vec<(usize, usize, Vec<Vec<u16>>)> = Vec::new();
        // The coded rows left out for their size, which a pass that fills reads.
        let mut left_out: Vec<(usize, usize)> = Vec::new();
        for row in plan.rows.clone() {
            let at = (self.rows[row].virtual_address - plan.start) as usize;
            let end = at + self.size(row) as usize;
            let wanted = |span: &(usize, usize)| span.0 < end && at < span.1;
            if end - at > 2 * element * MAX_PLAIN_BLOCKS || !spans.iter().flatten().any(wanted) {
                continue;
            }
            // No table reads a body of these kinds: its lanes are what a calibration reads, where
            // it is small enough.
            if (3..=7).contains(&kind)
                && self.rows[row].kind == kind
                && self.replaced_row(row).is_none()
            {
                if end - at <= element * coded_blocks {
                    let lanes = (self.stored(&mut file, row).ok())
                        .and_then(|stored| bcpack::lanes(kind, &stored, (end - at) as u64).ok());
                    if let Some(lanes) = lanes {
                        coded.push((at, end, lanes));
                    }
                } else if matches!(kind, 6 | 7) && end - at <= element * LARGE_CODED_BLOCKS {
                    left_out.push((at, end));
                }
                continue;
            }
            match self.body(&mut file, row) {
                Ok(body) if body.len() == end - at => {
                    let (from, to) = (at.max(low), end.min(high));
                    memory[from - low..to - low].copy_from_slice(&body[from - at..to - at]);
                    match exact.last_mut() {
                        Some(last) if last.1 == from => last.1 = to,
                        _ => exact.push((from, to)),
                    }
                }
                Err(
                    TtpfError::UnidentifiedCodec { .. }
                    | TtpfError::BcPack {
                        source:
                            BcPackError::NoBlockTables { .. } | BcPackError::PresetNotKnown { .. },
                        ..
                    },
                ) if self.rows[row].kind == kind && end - at <= element * coded_blocks => {
                    let lanes = (self.stored(&mut file, row).ok())
                        .and_then(|stored| bcpack::lanes(kind, &stored, (end - at) as u64).ok());
                    if let Some(lanes) = lanes {
                        coded.push((at, end, lanes));
                    }
                }
                _ => {}
            }
        }
        let levels = (layout.levels.iter().zip(&spans).enumerate())
            .map(|(i, (level, span))| {
                let size = [level.width, level.height, 1];
                let data = span.and_then(|(from, to)| {
                    let plain = (exact.iter().any(|&(a, b)| a <= from && to <= b))
                        .then(|| memory.get(from - low..to - low))
                        .flatten()
                        .and_then(|piece| {
                            untile(piece, level.at - level.from(), size, &layout.tile)
                        })
                        // A tail slot past the end of a chain whose levels are not stated is all zero.
                        .filter(|blocks| {
                            layout.stated || i == 0 || !level.tail || blocks.iter().any(|&b| b != 0)
                        });
                    if let Some(blocks) = plain {
                        return Some(LevelData::Plain(blocks));
                    }
                    if !level.tail && left_out.contains(&(from, to)) {
                        return Some(LevelData::Large);
                    }
                    let lanes = (coded.iter_mut())
                        .find(|row| !level.tail && (row.0, row.1) == (from, to))
                        .map(|row| std::mem::take(&mut row.2))?;
                    let at = places(to - from, element, size, &layout.tile)?;
                    Some(LevelData::Coded { lanes, at })
                });
                LevelIn {
                    width: level.width,
                    height: level.height,
                    data: data.unwrap_or(LevelData::Absent),
                }
            })
            .collect();
        let signed = layout.format.dxgi() == DxgiFormat::Bc6hSf16;
        Some(TextureIn {
            kind,
            signed,
            levels,
        })
    }
}

/// Fits the coder's tables from `packs` together; `None` once `stop` says to stop.
///
/// `progress` is told how far the whole is: the search for the textures to read is its first
/// third, the passes over them the rest.
pub fn calibrate_packs(
    packs: &[TexturePack],
    progress: &mut dyn FnMut(u64, u64),
    stop: &(dyn Fn() -> bool + Sync),
) -> Option<Calibrated> {
    calibrate_packs_from(packs, None, progress, stop)
}

/// [`calibrate_packs`], keeping under `keep` where the fit stands after each pass: a run that is
/// stopped or cut short is gone on from, by the next run of the same packs, at the pass it was in.
pub fn calibrate_packs_from(
    packs: &[TexturePack],
    keep: Option<&std::path::Path>,
    progress: &mut dyn FnMut(u64, u64),
    stop: &(dyn Fn() -> bool + Sync),
) -> Option<Calibrated> {
    // A pack, a texture of it and a slice of the texture.
    let mut wanted: Vec<(usize, usize, usize)> = Vec::new();
    for (p, pack) in packs.iter().enumerate() {
        if stop() {
            return None;
        }
        progress(p as u64, 3 * packs.len() as u64);
        for index in pack.calibration_textures(stop) {
            let slices = pack.calibration_slices(index);
            wanted.extend((0..slices).map(|slice| (p, index, slice)));
        }
    }
    if stop() {
        return None;
    }
    let read = |at: usize, large: bool| {
        let (p, index, slice) = wanted[at];
        Vec::from_iter(packs[p].calibration_input(index, slice, large))
    };
    let mut rest = |done: u64, total: u64| progress(total / 2 + done, total + total / 2);
    // Where a run of these packs that did not end stood, and where this one stands after each pass.
    let members: Vec<u64> = packs.iter().map(TexturePack::member_key).collect();
    let part = keep.map(|dir| fit::cache_path(dir, &members).with_extension("part"));
    let from = part.as_ref().and_then(|part| std::fs::read(part).ok());
    let mut kept = |stood: &[u8]| {
        if let Some(part) = &part {
            // A pass that is not kept is run again by the next run.
            let _ = fit::write_whole(part, stood);
        }
    };
    let (tables, summaries) = fit::calibrate_from(
        wanted.len(),
        &read,
        &mut rest,
        stop,
        from.as_deref(),
        &mut kept,
    )?;
    if let Some(part) = &part {
        let _ = std::fs::remove_file(part);
    }
    Some(Calibrated {
        tables,
        summaries,
        members: packs.iter().map(TexturePack::member_key).collect(),
    })
}

/// How the textures of a set of packs with a body stored through the coder read, for one codec kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    pub kind: u8,
    /// Textures whose coded bodies all decode exactly, those with one rebuilt through a fitted table, and those with one refused.
    pub exact: u64,
    pub calibrated: u64,
    pub refused: u64,
    /// The blocks of the coded bodies that read, those of them rebuilt through a fitted table, and
    /// those of these drawn through a slot or symbol the fit did not solve.
    pub blocks: u64,
    pub fitted: u64,
    pub unknown: u64,
}

impl TexturePack {
    /// Adds how texture `index` reads to `coverage`, if a body of it is stored through the coder.
    fn cover(&self, index: usize, coverage: &mut [Coverage; 8]) {
        let coded = |row: &usize| {
            let kind = self.rows[*row].kind;
            kind != KIND_STORED && kind != KIND_ZLIB && self.replaced_row(*row).is_none()
        };
        let rows: Vec<usize> = (self.rows_of(index).into_iter().flatten())
            .filter(coded)
            .collect();
        let (Some(&first), Ok(mut file)) = (rows.first(), File::open(&self.path)) else {
            return;
        };
        let kind = self.rows[first].kind;
        let Some(of_kind) = coverage.get_mut(usize::from(kind)) else {
            return;
        };
        let element = self.entries[index]
            .format()
            .map_or(16, |format| format.element_bytes() as u64);
        let (mut how, mut blocks) = (bcpack::Decode::default(), 0);
        for row in rows {
            if self.body_fitted(&mut file, row, &mut how).is_err() {
                of_kind.refused += 1;
                return;
            }
            blocks += self.size(row) / element;
        }
        of_kind.blocks += blocks;
        of_kind.fitted += how.calibrated;
        of_kind.unknown += how.unknown;
        if how.is_exact() {
            of_kind.exact += 1;
        } else {
            of_kind.calibrated += 1;
        }
    }
}

/// How every texture of `packs` with a body stored through the coder reads through the calibration
/// each pack has set, by codec kind; `None` once `stop` says to stop.
///
/// Every coded body is decoded, on several workers that each hold one body at a time.
pub fn coverage(packs: &[TexturePack], stop: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Coverage>> {
    let units: Vec<(usize, usize)> = (packs.iter().enumerate())
        .flat_map(|(p, pack)| (0..pack.entries.len()).map(move |index| (p, index)))
        .collect();
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let mut total = [Coverage::default(); 8];
    std::thread::scope(|scope| {
        let running: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut part = [Coverage::default(); 8];
                    while let Some(&(p, index)) = units.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if stop() {
                            break;
                        }
                        packs[p].cover(index, &mut part);
                    }
                    part
                })
            })
            .collect();
        for worker in running {
            match worker.join() {
                Ok(part) => {
                    for (total, part) in total.iter_mut().zip(part) {
                        total.exact += part.exact;
                        total.calibrated += part.calibrated;
                        total.refused += part.refused;
                        total.blocks += part.blocks;
                        total.fitted += part.fitted;
                        total.unknown += part.unknown;
                    }
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    });
    if stop() {
        return None;
    }
    let seen = |(kind, of_kind): (usize, Coverage)| {
        (of_kind.exact + of_kind.calibrated + of_kind.refused > 0).then_some(Coverage {
            kind: kind as u8,
            ..of_kind
        })
    };
    Some(total.into_iter().enumerate().filter_map(seen).collect())
}
