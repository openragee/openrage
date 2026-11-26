//! A patched copy of a GTA VI archive: bodies rewritten chunk by chunk to the sizes the signed table
//! states, every header, table and signature byte left as the source holds it, so no check read in
//! the game's code rejects it (no run of the game was made). A body that cannot keep every chunk's
//! sizes is refused: only a rebuilt archive could hold it, and the game refuses one.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::path::Path;

use flate2::{
    Compress, Compression, Decompress, DecompressError, FlushCompress, FlushDecompress, Status,
};

use super::gta6_archive::{Gta6Class, Gta6Entry};
use super::gta6_chunk_table::{entry_chunks, Chunk};
use super::gta6_content::{decrypt_extent, framing_reject_reason};
use super::gta6_mount::{open_at, Gta6Mount};
use super::rpf9::{Rpf9Header, RPF9_HEADER_SIZE};
use super::rpf9_toc::GtaViKeys;
use crate::error::IoError;

/// The content cipher type the reader opens; a body under any other is not re-enciphered.
const CONTENT_CIPHER_TYPE: u16 = 17;
/// Rounds of the optimal parse over one part: about a second for 64 KiB in an optimised build,
/// several at the dev profile's `opt-level = 1`.
const OPTIMAL_ROUNDS: u64 = 15;
/// Rounds in a row that find no shorter stream, after which the optimal parse stops early.
const OPTIMAL_STALL: u64 = 3;
/// The bytes the optimal parse takes at a time, with the 32 KiB before them as its window: its time
/// grows linearly with a chunk, and a stop waits for one part at most.
const OPTIMAL_PART: usize = 64 * 1024;
/// The largest chunk the optimal parse is run over: a save waits minutes for one this long.
pub const OPTIMAL_CAP: usize = 8 << 20;

/// A new inflated body for row `entry` of the table of the archive whose header is at offset `base`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyPatch {
    pub base: u64,
    pub entry: u32,
    pub body: Vec<u8>,
}

/// One chunk of a planned patch: its table's sizes, its new stream's length, whether it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkPatch {
    pub inflated: u32,
    /// The stored size the table states; a changed chunk fills exactly this many bytes.
    pub ceiling: u32,
    /// The new stream's length before what fills it to the ceiling; the ceiling if unchanged or stored.
    pub new_stored: u32,
    pub zlib: bool,
    pub changed: bool,
    /// Whether the optimal parse wrote the new stream, no level of the tree's deflater fitting.
    pub optimal: bool,
}

/// What patching one row writes: its chunks, and the bytes laid at absolute offsets of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchPlan {
    pub base: u64,
    pub entry: u32,
    pub ciphered: bool,
    pub chunks: Vec<ChunkPatch>,
    writes: Vec<(u64, Vec<u8>)>,
}

impl PatchPlan {
    /// The bytes this plan overwrites in a copy.
    pub fn bytes_written(&self) -> u64 {
        self.writes
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum()
    }
}

/// A written copy: chunks changed, bytes written, plans, and the table spans proven identical to the source's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchReport {
    pub chunks_changed: usize,
    pub bytes_written: u64,
    pub table_spans: Vec<(u64, u64)>,
    pub plans: Vec<PatchPlan>,
}

/// Why a body cannot be written in place, under a table left byte for byte as it is.
#[derive(Debug, thiserror::Error)]
pub enum PatchRefusal {
    #[error("the archive could not be read: {0}")]
    Archive(#[from] IoError),
    #[error("the archive at {base:#x} has no row {entry} that states where its body is")]
    NoRow { base: u64, entry: u32 },
    #[error(
        "entry {entry} is a nested archive: patch the rows inside it, addressed by its own base"
    )]
    NestedArchive { entry: u32 },
    #[error("entry {entry} is under cipher type {cipher_type}, which OpenRage does not encipher")]
    CipherUnsupported { entry: u32, cipher_type: u16 },
    #[error("entry {entry} {why}")]
    Malformed { entry: u32, why: String },
    #[error(
        "entry {entry} has no chunk table, and its body is neither one stream nor one stored body \
         that OpenRage reads back the same"
    )]
    NoChunkTable { entry: u32 },
    #[error(
        "the new body of entry {entry} is {got} bytes, not the {want} its table states; a patch in \
         place keeps every length"
    )]
    Length { entry: u32, got: u64, want: u64 },
    #[error(
        "chunk {chunk} of entry {entry} deflates at best to {deflated} bytes by the tree's deflater \
         {}: a stream may not run past it",
        optimal_said(*.optimal, *.ceiling, *.cap)
    )]
    ChunkDoesNotFit {
        entry: u32,
        chunk: usize,
        /// The shortest stream of the tree's deflater at any level.
        deflated: usize,
        /// The optimal parse's stream; `usize::MAX` when the chunk is past `cap` and it was not tried.
        optimal: usize,
        ceiling: u32,
        /// The largest chunk the plan ran the optimal parse over.
        cap: usize,
    },
    #[error("planning entry {entry} was stopped before it ended, and nothing was written")]
    Stopped { entry: u32 },
}

fn optimal_said(optimal: usize, ceiling: u32, cap: usize) -> String {
    if optimal == usize::MAX {
        let cap = if cap.is_multiple_of(1 << 20) {
            format!("{} MiB", cap >> 20)
        } else {
            format!("{} KiB", cap >> 10)
        };
        format!(
            "and the optimal parse is not tried on a chunk past {cap}, so nothing fits its \
             {ceiling}-byte stored size"
        )
    } else {
        format!(
            "and to {optimal} by an optimal parse, and neither fits its {ceiling}-byte stored size"
        )
    }
}

/// Plan `patch.body` over its row's chunks, refusing by name a body that cannot keep their sizes.
pub fn plan_patch<R: Read + Seek>(
    source: &mut R,
    keys: &GtaViKeys,
    patch: &BodyPatch,
) -> Result<PatchPlan, PatchRefusal> {
    plan_patch_with(source, keys, patch, OPTIMAL_CAP, &mut |_, _| true)
}

/// [`plan_patch`] with the optimal parse run over chunks of at most `cap` bytes, telling `tick` the bytes planned of the body's length; `false` from it stops the plan.
pub fn plan_patch_with<R: Read + Seek>(
    source: &mut R,
    keys: &GtaViKeys,
    patch: &BodyPatch,
    cap: usize,
    tick: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<PatchPlan, PatchRefusal> {
    let mount = Gta6Mount {
        base: patch.base,
        archive: open_at(source, patch.base, keys)?,
    };
    plan_in(&mount, source, keys, patch, cap, tick)
}

/// Copy `source` to the new file `out` with `patches` written in place and its tables proven unchanged.
pub fn write_patched_copy(
    source: &Path,
    out: &Path,
    keys: &GtaViKeys,
    patches: &[BodyPatch],
) -> Result<PatchReport, IoError> {
    if out.exists() {
        return Err(bad(format!(
            "{} exists, and a patched copy is written only to a new file",
            out.display()
        )));
    }
    let mut src = File::open(source)?;
    let mut plans: Vec<PatchPlan> = Vec::with_capacity(patches.len());
    for patch in patches {
        let plan = plan_patch(&mut src, keys, patch).map_err(|refusal| {
            bad(format!(
                "entry {} of the archive at {:#x} cannot be patched in place: {refusal}",
                patch.entry, patch.base
            ))
        })?;
        plans.push(plan);
    }
    write_planned_copy(source, out, keys, plans)
}

/// [`write_patched_copy`] from plans [`plan_patch`] already made over `source`.
pub fn write_planned_copy(
    source: &Path,
    out: &Path,
    keys: &GtaViKeys,
    plans: Vec<PatchPlan>,
) -> Result<PatchReport, IoError> {
    if out.exists() {
        return Err(bad(format!(
            "{} exists, and a patched copy is written only to a new file",
            out.display()
        )));
    }
    let mut src = File::open(source)?;
    let mut spans: Vec<(u64, u64)> = Vec::new();
    for (i, plan) in plans.iter().enumerate() {
        if plans[..i]
            .iter()
            .any(|p| p.base == plan.base && p.entry == plan.entry)
        {
            return Err(bad(format!(
                "entry {} of the archive at {:#x} is patched twice",
                plan.entry, plan.base
            )));
        }
        for span in protected_spans(&mut src, keys, plan.base)? {
            if !spans.contains(&span) {
                spans.push(span);
            }
        }
    }

    let mut writes: Vec<(u64, &[u8])> = plans
        .iter()
        .flat_map(|p| p.writes.iter().map(|(at, bytes)| (*at, &bytes[..])))
        .collect();
    writes.sort_by_key(|w| w.0);
    if writes
        .windows(2)
        .any(|w| w[0].0 + w[0].1.len() as u64 > w[1].0)
    {
        return Err(bad("two patches write the same bytes".into()));
    }
    for &(at, bytes) in &writes {
        let end = at + bytes.len() as u64;
        if let Some(&(a, b)) = spans.iter().find(|&&(a, b)| at < b && a < end) {
            return Err(bad(format!(
                "a chunk at {at:#x}..{end:#x} lies in the table span {a:#x}..{b:#x}"
            )));
        }
    }

    // Written beside `out` and renamed once proven, so no partial copy is ever left under its name.
    let mut partial = out.as_os_str().to_owned();
    partial.push(".partial");
    let partial = std::path::PathBuf::from(partial);
    let written = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(IoError::from)
        .and_then(|mut copy| {
            src.seek(SeekFrom::Start(0))?;
            std::io::copy(&mut src, &mut copy)?;
            patch_and_prove(&mut src, &mut copy, &writes, &spans)?;
            copy.sync_all()?;
            drop(copy);
            std::fs::rename(&partial, out)?;
            Ok(())
        });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    Ok(PatchReport {
        chunks_changed: plans
            .iter()
            .map(|p| p.chunks.iter().filter(|c| c.changed).count())
            .sum(),
        bytes_written: plans.iter().map(PatchPlan::bytes_written).sum(),
        table_spans: spans,
        plans,
    })
}

fn bad(why: String) -> IoError {
    IoError::Invalid {
        what: "a patched copy of a GTA VI archive".into(),
        why,
    }
}

fn read_at<R: Read + Seek>(src: &mut R, at: u64, len: u64) -> Result<Vec<u8>, IoError> {
    src.seek(SeekFrom::Start(at))?;
    let mut buf = vec![0u8; len as usize];
    src.read_exact(&mut buf)?;
    Ok(buf)
}

/// Write `writes` into `copy`, then compare every span of it with `src`'s.
fn patch_and_prove(
    src: &mut File,
    copy: &mut File,
    writes: &[(u64, &[u8])],
    spans: &[(u64, u64)],
) -> Result<(), IoError> {
    for &(at, bytes) in writes {
        copy.seek(SeekFrom::Start(at))?;
        copy.write_all(bytes)?;
    }
    copy.flush()?;
    for &(a, b) in spans {
        if read_at(src, a, b - a)? != read_at(copy, a, b - a)? {
            return Err(bad(format!(
                "the copy's table span {a:#x}..{b:#x} differs from the source's"
            )));
        }
    }
    Ok(())
}

/// The spans no patch may write, of every archive from the file's root down to the one whose header
/// is at `base`: its header and table, its names blob, and every nested archive's header and table.
fn protected_spans<R: Read + Seek>(
    src: &mut R,
    keys: &GtaViKeys,
    base: u64,
) -> Result<Vec<(u64, u64)>, IoError> {
    let file_end = src.seek(SeekFrom::End(0))?;
    let mut mount = Gta6Mount::open_root(src, keys)?;
    let mut spans = Vec::new();
    mount_spans(src, &mount, file_end, &mut spans)?;
    while mount.base != base {
        let inner = mount
            .archive
            .entries
            .iter()
            .find(|e| {
                e.kind() == Some(Gta6Class::NestedArchive)
                    && matches!((mount.absolute_start(e), e.stored),
                        (Some(s), Some(n)) if s > mount.base && s <= base && base < s + n)
            })
            .ok_or_else(|| {
                bad(format!(
                    "no archive of the file has its header at {base:#x}"
                ))
            })?;
        let end = mount.absolute_start(inner).unwrap_or(0) + inner.stored.unwrap_or(0);
        let next = mount.open_nested(src, inner, keys)?;
        mount = next;
        mount_spans(src, &mount, end, &mut spans)?;
    }
    Ok(spans)
}

/// `mount`'s header-and-table and names spans (it ends at `end`), and each nested archive's table span.
fn mount_spans<R: Read + Seek>(
    src: &mut R,
    mount: &Gta6Mount,
    end: u64,
    spans: &mut Vec<(u64, u64)>,
) -> Result<(), IoError> {
    let header = table_header(src, mount.base)?;
    // Nothing past the file's end can be written, so a span is cut at it and a header there skipped.
    let file_end = src.seek(SeekFrom::End(0))?;
    let mut push = |(a, b): (u64, u64)| {
        let span = (a, b.min(file_end));
        if span.0 < span.1 && !spans.contains(&span) {
            spans.push(span);
        }
    };
    push((mount.base, mount.base + u64::from(header.toc_end)));
    // The names blob ends the archive, after every row; where a row's stored bytes reach into it,
    // `end` is not the archive's (a file cut short), and the blob is not in the file to be written over.
    let names = end.saturating_sub(u64::from(header.names_size));
    let entries = mount.archive.entries.iter();
    if !entries
        .filter_map(|e| Some(mount.absolute_start(e)? + e.stored.unwrap_or(0)))
        .any(|row_end| row_end > names)
    {
        push((names, end));
    }
    for e in &mount.archive.entries {
        if e.kind() != Some(Gta6Class::NestedArchive) {
            continue;
        }
        if let Some(start) =
            (mount.absolute_start(e)).filter(|s| s + RPF9_HEADER_SIZE as u64 <= file_end)
        {
            let inner = table_header(src, start)?;
            push((start, start + u64::from(inner.toc_end)));
        }
    }
    Ok(())
}

fn table_header<R: Read + Seek>(src: &mut R, base: u64) -> Result<Rpf9Header, IoError> {
    Rpf9Header::parse(&read_at(src, base, RPF9_HEADER_SIZE as u64)?)
}

/// [`plan_patch_with`] over an archive already opened at `patch.base`.
fn plan_in<R: Read + Seek>(
    mount: &Gta6Mount,
    source: &mut R,
    keys: &GtaViKeys,
    patch: &BodyPatch,
    cap: usize,
    tick: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<PatchPlan, PatchRefusal> {
    let e = patch.entry;
    let malformed = |why: String| PatchRefusal::Malformed { entry: e, why };
    let RowChunks {
        at,
        raw,
        plain,
        chunks,
        want,
        ciphered,
    } = row_chunks(mount, source, keys, e)?;
    if patch.body.len() as u64 != want {
        return Err(PatchRefusal::Length {
            entry: e,
            got: patch.body.len() as u64,
            want,
        });
    }

    // Latched: once `tick` says stop, it is not asked again.
    let stopped = std::cell::Cell::new(false);
    let mut reached = |done: u64| {
        if !stopped.get() && !tick(done, want) {
            stopped.set(true);
        }
        !stopped.get()
    };
    let mut planned = Vec::with_capacity(chunks.len());
    let mut writes = Vec::new();
    let (mut pos, mut cursor) = (0usize, 0usize);
    for (i, chunk) in chunks.iter().enumerate() {
        if !reached(cursor as u64) {
            return Err(PatchRefusal::Stopped { entry: e });
        }
        let (stored, inflated) = (chunk.stored as usize, chunk.inflated as usize);
        let old = plain
            .get(pos..pos + stored)
            .ok_or_else(|| malformed(format!("has chunk {i} running past its stored extent")))?;
        let piece = &patch.body[cursor..cursor + inflated];
        let (framed, content) = chunk_content(old, chunk).ok_or_else(|| {
            malformed(format!(
                "has chunk {i}, which does not inflate from {stored} bytes to the {inflated} its \
                 table states"
            ))
        })?;
        let changed = content != piece;
        let mut new_stored = chunk.stored;
        let mut by_optimal = false;
        if changed {
            let new = if chunk.zlib {
                let fitted = deflate_to(piece, framed, stored, cap, &mut |n| {
                    reached((cursor + n) as u64)
                });
                if stopped.get() {
                    return Err(PatchRefusal::Stopped { entry: e });
                }
                let (stream, deflated, optimal) =
                    fitted.map_err(|(deflated, optimal)| PatchRefusal::ChunkDoesNotFit {
                        entry: e,
                        chunk: i,
                        deflated,
                        optimal,
                        ceiling: chunk.stored,
                        cap,
                    })?;
                new_stored = deflated as u32;
                by_optimal = optimal;
                stream
            } else {
                piece.to_vec()
            };
            // CTR keys a byte by its offset alone, so the new ciphertext is the old one xor the change
            // in plaintext: enciphered windows are re-enciphered, clear ones stay clear.
            let bytes: Vec<u8> = new
                .iter()
                .zip(old)
                .zip(&raw[pos..pos + stored])
                .map(|((n, p), r)| n ^ p ^ r)
                .collect();
            writes.push((at + pos as u64, bytes));
        }
        planned.push(ChunkPatch {
            inflated: chunk.inflated,
            ceiling: chunk.stored,
            new_stored,
            zlib: chunk.zlib,
            changed,
            optimal: by_optimal,
        });
        pos = (pos + stored + 15) & !15;
        cursor += inflated;
    }
    reached(want);
    Ok(PatchPlan {
        base: patch.base,
        entry: e,
        ciphered,
        chunks: planned,
        writes,
    })
}

/// Every span no patch may write, of the root archive and each nested one at any depth.
pub fn every_table_span<R: Read + Seek>(
    src: &mut R,
    keys: &GtaViKeys,
) -> Result<Vec<(u64, u64)>, IoError> {
    let file_end = src.seek(SeekFrom::End(0))?;
    let mut spans = Vec::new();
    let mut todo = vec![(Gta6Mount::open_root(src, keys)?, file_end)];
    while let Some((mount, end)) = todo.pop() {
        mount_spans(src, &mount, end, &mut spans)?;
        for e in &mount.archive.entries {
            let (Some(start), Some(n)) = (mount.absolute_start(e), e.stored) else {
                continue;
            };
            // A header past the file's end is not in it; one at or before its parent's would loop.
            if e.kind() == Some(Gta6Class::NestedArchive)
                && start > mount.base
                && start + RPF9_HEADER_SIZE as u64 <= file_end
            {
                todo.push((mount.open_nested(src, e, keys)?, start + n));
            }
        }
    }
    spans.sort_unstable();
    Ok(spans)
}

/// Row `entry` of `mount` read as a patch reads it: deciphered, each chunk inflated to its stated size.
pub fn read_row_by_chunks<R: Read + Seek>(
    mount: &Gta6Mount,
    source: &mut R,
    keys: &GtaViKeys,
    entry: u32,
) -> Result<Vec<u8>, PatchRefusal> {
    let row = row_chunks(mount, source, keys, entry)?;
    // Not sized from the table: a malformed one may state any total.
    let mut out = Vec::new();
    let mut pos = 0usize;
    for (i, chunk) in row.chunks.iter().enumerate() {
        let stored = chunk.stored as usize;
        let (_, content) = (row.plain.get(pos..pos + stored))
            .and_then(|old| chunk_content(old, chunk))
            .ok_or_else(|| PatchRefusal::Malformed {
                entry,
                why: format!(
                    "has chunk {i}, which does not inflate from {stored} bytes to the {} its table \
                     states",
                    chunk.inflated
                ),
            })?;
        out.extend_from_slice(&content);
        pos = (pos + stored + 15) & !15;
    }
    Ok(out)
}

/// A row's stored bytes, where they sit, as stored and deciphered, and its chunks with their total.
struct RowChunks {
    at: u64,
    raw: Vec<u8>,
    plain: Vec<u8>,
    chunks: Vec<Chunk>,
    want: u64,
    ciphered: bool,
}

/// Row `e` of `mount` located, deciphered and split into the chunks its table or lone stream states.
fn row_chunks<R: Read + Seek>(
    mount: &Gta6Mount,
    source: &mut R,
    keys: &GtaViKeys,
    e: u32,
) -> Result<RowChunks, PatchRefusal> {
    let malformed = |why: String| PatchRefusal::Malformed { entry: e, why };
    let no_row = PatchRefusal::NoRow {
        base: mount.base,
        entry: e,
    };
    let Some(entry) = mount.archive.entries.get(e as usize) else {
        return Err(no_row);
    };
    if entry.kind() == Some(Gta6Class::NestedArchive) {
        return Err(PatchRefusal::NestedArchive { entry: e });
    }
    let Some((at, len, real)) = extent(mount, entry) else {
        return Err(no_row);
    };
    let end = source.seek(SeekFrom::End(0)).map_err(IoError::from)?;
    if at.checked_add(len).is_none_or(|x| x > end) {
        return Err(malformed(
            "has a stored extent that runs past the file".into(),
        ));
    }
    let raw = read_at(source, at, len)?;
    let mut plain = raw.clone();
    if let Some(cipher) = entry.cipher {
        if cipher.cipher_type != CONTENT_CIPHER_TYPE {
            return Err(PatchRefusal::CipherUnsupported {
                entry: e,
                cipher_type: cipher.cipher_type,
            });
        }
        if let Some(why) = framing_reject_reason(&cipher.framing, len) {
            return Err(malformed(why));
        }
        let mut failed = None;
        decrypt_extent(&mut plain, &cipher.framing, &cipher.iv, |slice, ctr| {
            if failed.is_none() {
                failed = keys.content_decrypt(cipher.key_id, ctr, slice).err();
            }
        });
        if let Some(err) = failed {
            return Err(err.into());
        }
    }

    let chunks = match entry_chunks(&mount.archive, e as usize) {
        Err(err) => return Err(malformed(format!("has a malformed chunk table ({err})"))),
        Ok(Some(chunks)) => chunks,
        Ok(None) => {
            let chunk = lone_chunk(&plain, real).ok_or(PatchRefusal::NoChunkTable { entry: e })?;
            // A ciphered body with no table is read by a walk that can split it otherwise; the patch
            // goes ahead only where that walk reads the body this chunk holds.
            if entry.cipher.is_some() {
                let held = chunk_content(&plain[..chunk.stored as usize], &chunk).map(|(_, c)| c);
                if held.is_none()
                    || mount.read_entry_decrypted(&mut *source, entry, keys).ok() != held
                {
                    return Err(PatchRefusal::NoChunkTable { entry: e });
                }
            }
            vec![chunk]
        }
    };
    let want: u64 = chunks.iter().map(|c| u64::from(c.inflated)).sum();
    if let Some(real) = real.filter(|&r| r != want) {
        return Err(malformed(format!(
            "has chunks that inflate to {want} bytes, not the {real} its row states"
        )));
    }
    Ok(RowChunks {
        at,
        raw,
        plain,
        chunks,
        want,
        ciphered: entry.cipher.is_some(),
    })
}

/// Where `entry`'s stored bytes sit in the file, how many there are, and the real size a 24-byte row
/// states; a 20-byte row's body begins past its 16-byte zero slot.
fn extent(mount: &Gta6Mount, entry: &Gta6Entry) -> Option<(u64, u64, Option<u64>)> {
    let start = mount.absolute_start(entry)?;
    if let (Some(stored), Some(real)) = (entry.stored, entry.real) {
        return Some((start, stored, Some(real)));
    }
    let nested = mount.archive.entry_extra(entry.index as usize)?.nested?;
    let (at, len) = nested.ciphered_body_extent(start);
    Some((at, len, None))
}

/// The chunk a 24-byte row with no chunk table is stored as: one stream inflating to exactly its real
/// size, or, where its head is no stream at all, the real size stored as it is.
fn lone_chunk(plain: &[u8], real: Option<u64>) -> Option<Chunk> {
    let inflated = u32::try_from(real?).ok()?;
    let n = inflated as usize;
    let zlib = inflate_head(plain, true, n);
    let raw = inflate_head(plain, false, n);
    for head in [&zlib, &raw] {
        if let Ok(Some((used, out))) = head {
            let stored = u32::try_from(*used).ok()?;
            return (out.len() == n).then_some(Chunk {
                inflated,
                stored,
                zlib: true,
            });
        }
    }
    (zlib.is_err() && raw.is_err() && n <= plain.len()).then_some(Chunk {
        inflated,
        stored: inflated,
        zlib: false,
    })
}

/// A chunk's content from its stored bytes, and whether its stream is zlib-framed (raw DEFLATE
/// otherwise); a deflated chunk's stream must end within its stored bytes and inflate to exactly its
/// inflated size, and the bytes after it are ignored, as the reader and the game ignore them.
fn chunk_content(stored: &[u8], chunk: &Chunk) -> Option<(bool, Vec<u8>)> {
    if !chunk.zlib {
        return Some((false, stored.to_vec()));
    }
    let n = chunk.inflated as usize;
    [true, false]
        .into_iter()
        .find_map(|framed| match inflate_head(stored, framed, n) {
            Ok(Some((used, out))) if used <= stored.len() && out.len() == n => Some((framed, out)),
            _ => None,
        })
}

/// Inflate the stream at the head of `bytes` into at most `cap` bytes: the bytes consumed and the
/// output when it ends within them, `None` when it does not, an error when the bytes are no stream.
fn inflate_head(
    bytes: &[u8],
    zlib: bool,
    cap: usize,
) -> Result<Option<(usize, Vec<u8>)>, DecompressError> {
    let mut d = Decompress::new(zlib);
    let mut out = Vec::with_capacity(cap.saturating_add(1));
    let status = d.decompress_vec(bytes, &mut out, FlushDecompress::Finish)?;
    Ok((status == Status::StreamEnd).then(|| (d.total_in() as usize, out)))
}

/// `content` deflated (zlib-framed when `zlib`) to a stream filled to exactly `len` bytes: the
/// shortest fitting level of the tree's deflater, else the optimal parse's stream on a chunk of at most
/// `cap` bytes.
/// `Ok` holds the filled stream, its own length and whether the optimal parse made it; `Err` holds
/// the shortest length of the tree's deflater and the optimal parse's. `go` is asked between levels
/// and parts with the bytes parsed; once it says stop, the result is to be discarded.
// The optimal parse is far slower than any level, so it runs only for a chunk no level fits.
fn deflate_to(
    content: &[u8],
    zlib: bool,
    len: usize,
    cap: usize,
    go: &mut dyn FnMut(usize) -> bool,
) -> Result<(Vec<u8>, usize, bool), (usize, usize)> {
    // A stream fills `len` exactly when it is no longer, so only the shortest level is kept.
    let mut best: Option<Vec<u8>> = None;
    for level in 0..=9 {
        if !go(0) {
            return Err((usize::MAX, usize::MAX));
        }
        if let Some(s) = deflate(content, level, zlib) {
            if best.as_ref().is_none_or(|b| s.len() < b.len()) {
                best = Some(s);
            }
        }
    }
    let shortest = best.as_ref().map_or(usize::MAX, Vec::len);
    if let Some(out) = best.as_ref().and_then(|s| fill(s, zlib, len)) {
        return Ok((out, shortest, false));
    }
    if content.len() > cap {
        return Err((shortest, usize::MAX));
    }
    let optimal = optimal_parse(content, zlib, go);
    let optimal_len = optimal.as_ref().map_or(usize::MAX, Vec::len);
    match optimal.and_then(|s| fill(&s, zlib, len)) {
        Some(out) => Ok((out, optimal_len, true)),
        None => Err((shortest, optimal_len)),
    }
}

/// `stream` filled to exactly `len` bytes: led by empty blocks, past its zlib head when `zlib`, where
/// they fill the slack, else followed by zeros, which the reader and the game ignore past a stream's end.
fn fill(stream: &[u8], zlib: bool, len: usize) -> Option<Vec<u8>> {
    let slack = len.checked_sub(stream.len())?;
    let Some(pad) = padding(slack) else {
        return Some([stream, &vec![0u8; slack][..]].concat());
    };
    let (head, rest) = stream.split_at_checked(if zlib { 2 } else { 0 })?;
    Some([head, &pad[..], rest].concat())
}

/// `content` deflated by an optimal parse (zopfli), zlib-framed when `zlib`, raw DEFLATE otherwise.
pub fn optimal_deflate(content: &[u8], zlib: bool) -> Option<Vec<u8>> {
    optimal_parse(content, zlib, &mut |_| true)
}

fn optimal_options() -> Option<zopfli::Options> {
    Some(zopfli::Options {
        iteration_count: NonZeroU64::new(OPTIMAL_ROUNDS)?,
        iterations_without_improvement: NonZeroU64::new(OPTIMAL_STALL)?,
        ..zopfli::Options::default()
    })
}

/// [`optimal_deflate`] one [`OPTIMAL_PART`] at a time, each part's window the bytes before it, so the
/// parts join as one stream; `None` once `go`, asked after each part with the bytes parsed, says stop.
fn optimal_parse(content: &[u8], zlib: bool, go: &mut dyn FnMut(usize) -> bool) -> Option<Vec<u8>> {
    let options = optimal_options()?;
    let dynamic = zopfli::BlockType::Dynamic;
    let mut out = Vec::with_capacity(content.len() / 2 + 64);
    if zlib {
        let encoder = zopfli::ZlibEncoder::new(options, dynamic, &mut out).ok()?;
        in_parts(encoder, content, go)?.finish().ok()?;
    } else {
        let encoder = zopfli::DeflateEncoder::new(options, dynamic, &mut out);
        in_parts(encoder, content, go)?.finish().ok()?;
    }
    Some(out)
}

/// `content` written to `encoder` a part at a time; an encoder parses a part on the write after it.
fn in_parts<W: Write>(
    mut encoder: W,
    content: &[u8],
    go: &mut dyn FnMut(usize) -> bool,
) -> Option<W> {
    for (i, part) in content.chunks(OPTIMAL_PART).enumerate() {
        encoder.write_all(part).ok()?;
        if !go(i * OPTIMAL_PART) {
            // Dropping an encoder parses the part it holds, so a stopped one is forgotten: it leaks
            // at most one part and its window.
            std::mem::forget(encoder);
            return None;
        }
    }
    Some(encoder)
}

fn deflate(content: &[u8], level: u32, zlib: bool) -> Option<Vec<u8>> {
    let mut c = Compress::new(Compression::new(level), zlib);
    let mut out = Vec::with_capacity(content.len() + content.len() / 8 + 64);
    match c.compress_vec(content, &mut out, FlushCompress::Finish) {
        Ok(Status::StreamEnd) => Some(out),
        _ => None,
    }
}

/// Empty DEFLATE blocks that begin and end on a byte and fill exactly `n` bytes, to lead a stream; `None`
/// for 1 to 4 and 8, which no run of them fills.
fn padding(n: usize) -> Option<Vec<u8>> {
    // Groups of k empty fixed blocks closed by an empty stored block: 5, 6, 7 and 9 bytes for k = 0..=3.
    let (groups, used): (&[usize], usize) = match n % 5 {
        0 => (&[], 0),
        1 => (&[1], 6),
        2 => (&[2], 7),
        4 => (&[3], 9),
        _ => (&[1, 2], 13),
    };
    let rest = n.checked_sub(used)?;
    let mut out = Vec::with_capacity(n);
    for &k in groups.iter().chain(std::iter::repeat_n(&0, rest / 5)) {
        // An empty fixed block is 10 bits: BFINAL 0, BTYPE 01, the 7-bit end code 0; the stored
        // block's 3 header bits are 0, then it aligns and states LEN 0, NLEN 0xFFFF.
        let mut head = vec![0u8; (10 * k + 3).div_ceil(8)];
        for i in 0..k {
            let bit = 10 * i + 1;
            head[bit / 8] |= 1 << (bit % 8);
        }
        out.extend_from_slice(&head);
        out.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
    }
    Some(out)
}

#[cfg(test)]
#[path = "gta6_patch_tests.rs"]
mod tests;
