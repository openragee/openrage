//! Reading an RPF9 entry's body and mounting a nested archive from the decrypted table.
//!
//! An entry row states where the entry's bytes begin, relative to its archive's own header, and how
//! large they are ([`Gta6Entry`]). Two-thirds of a build's top-level entries are themselves RPF9
//! archives, whose 64-byte header is in the clear at that position; their table decrypts with the key
//! material already held, so a nested archive is opened by decrypting its table at `parent_base +
//! start`, recursively, with no scan and no new key. Reads are ranged — a top-level archive runs to
//! tens of gigabytes — so this takes a `Read + Seek` source and reads only a header, a table, or one
//! body at a time. It is read-only: nothing here writes an archive.

use std::io::{Read, Seek, SeekFrom};

use byteorder::{ByteOrder, LE};

use super::gta6_archive::{Gta6Archive, Gta6Entry};
use super::rpf9::{Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC};
use super::rpf9_scan::read_rpf9_body;
use super::rpf9_toc::GtaViKeys;
use crate::error::{IoError, Result};

/// The content cipher type (type 17): an entry whose cipher record states it has a CTR-enciphered body
/// whose plaintext is zlib. Other cipher types are not opened here.
const CONTENT_CIPHER_TYPE: u16 = 17;

/// `[offset, offset + len)` of `src`.
fn read_at<R: Read + Seek>(src: &mut R, offset: u64, len: usize) -> Result<Vec<u8>> {
    src.seek(SeekFrom::Start(offset)).map_err(mount_io)?;
    let mut buf = vec![0u8; len];
    src.read_exact(&mut buf).map_err(mount_io)?;
    Ok(buf)
}

fn mount_io(source: std::io::Error) -> IoError {
    IoError::Invalid {
        what: "reading an RPF9 archive".into(),
        why: source.to_string(),
    }
}

fn no_entry_row(entry: &Gta6Entry) -> IoError {
    IoError::Invalid {
        what: "an RPF9 entry".into(),
        why: format!(
            "entry {} has no entry row, so its start is unknown",
            entry.index
        ),
    }
}

/// Open the RPF9 archive whose header is at absolute `offset` in `src`, decrypting its table with
/// `keys`. The header states the table's end; only the header and table are read, never the content.
pub fn open_at<R: Read + Seek>(src: &mut R, offset: u64, keys: &GtaViKeys) -> Result<Gta6Archive> {
    let head = read_at(src, offset, RPF9_HEADER_SIZE)?;
    let header = Rpf9Header::parse(&head)?;
    let toc = read_at(src, offset, header.toc_end as usize)?;
    Gta6Archive::open(&toc, keys)
}

/// An opened RPF9 archive together with the absolute offset of its header, so its entries' starts —
/// stated relative to that header — resolve to file positions.
#[derive(Debug, Clone)]
pub struct Gta6Mount {
    /// The absolute offset of this archive's header in the file; 0 for the top-level archive.
    pub base: u64,
    pub archive: Gta6Archive,
}

impl Gta6Mount {
    /// Open the top-level archive at the start of `src`.
    pub fn open_root<R: Read + Seek>(src: &mut R, keys: &GtaViKeys) -> Result<Self> {
        Ok(Self {
            base: 0,
            archive: open_at(src, 0, keys)?,
        })
    }

    /// The absolute file offset of `entry`'s bytes, or `None` when it has no entry row.
    pub fn absolute_start(&self, entry: &Gta6Entry) -> Option<u64> {
        entry.start.map(|start| self.base + start)
    }

    /// Whether `entry`'s body begins with the clear `9FPR` header, i.e. it is a nested archive. An
    /// entry with no row is not an archive.
    pub fn entry_begins_archive<R: Read + Seek>(
        &self,
        src: &mut R,
        entry: &Gta6Entry,
    ) -> Result<bool> {
        let Some(start) = self.absolute_start(entry) else {
            return Ok(false);
        };
        let head = read_at(src, start, 4)?;
        Ok(LE::read_u32(&head) == RPF9_MAGIC)
    }

    /// Mount the nested archive `entry` points at, decrypting its table at `base + entry.start`. The
    /// returned mount carries that absolute base, so a deeper nesting resolves the same way.
    pub fn open_nested<R: Read + Seek>(
        &self,
        src: &mut R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Self> {
        let start = self
            .absolute_start(entry)
            .ok_or_else(|| no_entry_row(entry))?;
        Ok(Self {
            base: start,
            archive: open_at(src, start, keys)?,
        })
    }

    /// Read `entry`'s body, inflated. [`read_rpf9_body`]'s detector reads the body it recognises at the
    /// row's start, capped by the row's real (inflated) size where it states one; a body the detector
    /// does not recognise at that start is an error. For a leaf body (a resource, a PSO, a movie); a
    /// nested archive is opened with [`Self::open_nested`], and an enciphered body
    /// (`entry.cipher.is_some()`) is not opened by this.
    pub fn read_entry<R: Read + Seek>(&self, src: R, entry: &Gta6Entry) -> Result<Vec<u8>> {
        let start = self
            .absolute_start(entry)
            .ok_or_else(|| no_entry_row(entry))?;
        // The inflated size, where the row states it, caps the inflated output; a 20-byte row states
        // none, so the detector's own measure of the body stands.
        let limit = entry.real.unwrap_or(u64::MAX);
        read_rpf9_body(src, start, limit)
            .map(|(_, bytes)| bytes)
            .map_err(|source| IoError::Invalid {
                what: "an RPF9 entry body".into(),
                why: source.to_string(),
            })
    }

    /// Read `entry`'s body, decrypting it first when it is content-ciphered (type 17): the inflated,
    /// concatenated bytes. A clear entry is read by [`Self::read_entry`]. For the full account of the
    /// read — the chunk count and the stored bytes consumed — use [`Self::read_entry_decrypted_report`].
    pub fn read_entry_decrypted<R: Read + Seek>(
        &self,
        src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Vec<u8>> {
        self.read_entry_decrypted_report(src, entry, keys)
            .map(|body| body.data)
    }

    /// Read a content-ciphered (type 17) entry's body in full: decipher its enciphered extent, then
    /// walk and inflate its chunks, reporting the inflated bytes, the chunk count and the stored bytes
    /// consumed. A clear entry (no cipher record) is read by [`Self::read_entry`] and reported with no
    /// chunk count.
    ///
    /// The extent is the rule of [`super::gta6_content::enciphered_ranges`]: the record's five framing
    /// words decide which byte ranges are enciphered, and each range is deciphered at its absolute
    /// counter (the IV advanced by the range's block offset, never restarted). The deciphered body is
    /// then a run of 16-aligned chunks — zlib, stored (a failed inflate) or, for a nested archive's
    /// first entry, raw DEFLATE — which are inflated and concatenated. A 24-byte row's real size bounds
    /// and ends the walk; a 20-byte row (a script body, whose stored real size is unreliable) walks to
    /// the end of its stored extent, a stored chunk ending at the next 16-aligned zlib head.
    pub fn read_entry_decrypted_report<R: Read + Seek>(
        &self,
        mut src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Gta6DecryptedBody> {
        let Some(cipher) = entry.cipher else {
            return self.read_entry(src, entry).map(|data| Gta6DecryptedBody {
                data,
                chunk_count: 0,
                stored_consumed: 0,
            });
        };
        if cipher.cipher_type != CONTENT_CIPHER_TYPE {
            return Err(IoError::RpfEncryptionUnsupported {
                key_id: cipher.key_id,
            });
        }
        let body_bad = |why: String| IoError::Invalid {
            what: "a content-ciphered RPF9 entry".into(),
            why,
        };
        let (body_start, stored, real) = self.body_geometry(entry)?;
        if stored == 0 {
            return Err(body_bad(format!("entry {} has an empty body", entry.index)));
        }
        // A malformed framing or extent is refused, not guessed.
        if let Some(reason) = super::gta6_content::framing_reject_reason(&cipher.framing, stored) {
            return Err(body_bad(format!("entry {} {reason}", entry.index)));
        }
        // Check the stored extent against the source before allocating, so a malformed row cannot
        // force a huge allocation.
        let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        if body_start.checked_add(stored).is_none_or(|s| s > end) {
            return Err(body_bad(format!(
                "entry {}'s stored extent runs past the source",
                entry.index
            )));
        }
        let mut body = read_at(&mut src, body_start, stored as usize)?;

        // Decipher only the enciphered ranges, each at its absolute counter. The CTR pass itself holds
        // the key material (`content_decrypt`), so the first failure (a missing key) stops it.
        let mut cipher_err: Option<IoError> = None;
        super::gta6_content::decrypt_extent(
            &mut body,
            &cipher.framing,
            &cipher.iv,
            |slice, ctr| {
                if cipher_err.is_none() {
                    if let Err(e) = keys.content_decrypt(cipher.key_id, ctr, slice) {
                        cipher_err = Some(e);
                    }
                }
            },
        );
        if let Some(e) = cipher_err {
            return Err(e);
        }

        let (data, chunk_count, consumed) = walk_chunks(&body, real)
            .map_err(|why| body_bad(format!("entry {} {why}", entry.index)))?;
        // A 24-byte row states the inflated size; the chunks must sum to exactly it.
        if let Some(real) = real {
            if data.len() as u64 != real {
                return Err(body_bad(format!(
                    "entry {}'s chunks inflated to {} bytes, not the {real} its row states",
                    entry.index,
                    data.len()
                )));
            }
        }
        Ok(Gta6DecryptedBody {
            data,
            chunk_count,
            stored_consumed: consumed,
        })
    }

    /// Where a ciphered entry's body sits and how it is sized: a 24-byte row carries the body start,
    /// the stored size and the real (inflated) size; a 20-byte row carries only a proven start, so its
    /// body (16 bytes past the slot, `w8 * 16 - 16` long, real size unreliable) is resolved from the
    /// archive's nested descriptor. Returns `(absolute body start, stored length, real size)`.
    fn body_geometry(&self, entry: &Gta6Entry) -> Result<(u64, u64, Option<u64>)> {
        let start = self
            .absolute_start(entry)
            .ok_or_else(|| no_entry_row(entry))?;
        if let (Some(stored), Some(real)) = (entry.stored, entry.real) {
            return Ok((start, stored, Some(real)));
        }
        // A 20-byte row: the body is 16 bytes past the slot, its stored length the paged size less the
        // slot. The real size is `w0`, which is not the sum of the chunks for a chunked script body, so
        // it is not used to bound the walk.
        let nested = self
            .archive
            .entry_extra(entry.index as usize)
            .and_then(|extra| extra.nested)
            .ok_or_else(|| IoError::Invalid {
                what: "a content-ciphered RPF9 entry".into(),
                why: format!(
                    "entry {} has no 24-byte size and no 20-byte descriptor, so its body cannot be \
                     located",
                    entry.index
                ),
            })?;
        let (body_start, stored) = nested.ciphered_body_extent(start);
        Ok((body_start, stored, None))
    }
}

/// The result of reading a content-ciphered entry: the inflated body, how many chunks it held, and how
/// many stored bytes the walk consumed (which a 24-byte row's walk ends exactly at its stored size).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gta6DecryptedBody {
    pub data: Vec<u8>,
    pub chunk_count: usize,
    pub stored_consumed: u64,
}

/// A per-chunk inflate is capped here to bound a malformed or hostile stream; real chunks inflate to
/// at most a few 64 KiB pages.
const MAX_CHUNK_INFLATE: u64 = 32 * 1024 * 1024;

/// Round `x` up to the next 16-byte boundary (chunks start 16-aligned).
fn round16(x: usize) -> usize {
    (x + 15) & !15
}

/// Walk the deciphered `body`'s 16-aligned chunks, inflating each and concatenating. `real` is the
/// stated inflated size for a 24-byte row, which bounds and ends the walk; `None` (a 20-byte row) walks
/// to the body's end, taking a stored chunk's length from the next 16-aligned zlib head. Returns the
/// inflated bytes, the chunk count and the stored offset reached.
fn walk_chunks(
    body: &[u8],
    real: Option<u64>,
) -> std::result::Result<(Vec<u8>, usize, u64), String> {
    let mut out: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    let mut chunks = 0usize;
    // The overall output ceiling: the stated real size, or, with none, a generous multiple of the
    // stored bytes (zlib cannot plausibly exceed it) to stop a runaway.
    // ponytail: 64x stored ceiling for 20-byte bodies, tighten if a real body is ever denser.
    let out_cap = real.unwrap_or((body.len() as u64).saturating_mul(64));
    loop {
        if pos >= body.len() {
            break;
        }
        match real {
            Some(real) if out.len() as u64 >= real => break,
            // With no real size, trailing zero padding ends the body.
            None if body[pos..].iter().all(|&b| b == 0) => break,
            _ => {}
        }
        let inflate_cap = out_cap.saturating_sub(out.len() as u64).saturating_add(1);
        if let Some((consumed, inflated)) = inflate_one(&body[pos..], false, inflate_cap) {
            out.extend_from_slice(&inflated);
            pos += consumed;
        } else if pos == 0 {
            // A nested archive's first entry can be raw DEFLATE, with no zlib header.
            if let Some((consumed, inflated)) = inflate_one(&body[pos..], true, inflate_cap) {
                out.extend_from_slice(&inflated);
                pos += consumed;
            } else {
                let len = stored_chunk_len(body, pos, real, out.len() as u64);
                out.extend_from_slice(&body[pos..pos + len]);
                pos += len;
            }
        } else {
            // A failed inflate marks a stored chunk; its length comes from the real size (24-byte) or
            // the next 16-aligned zlib head (20-byte).
            let len = stored_chunk_len(body, pos, real, out.len() as u64);
            if len == 0 {
                break;
            }
            out.extend_from_slice(&body[pos..pos + len]);
            pos += len;
        }
        chunks += 1;
        pos = round16(pos).min(body.len());
        if out.len() as u64 > out_cap {
            return Err("inflated past its stored body; the stream is malformed".to_string());
        }
    }
    Ok((out, chunks, pos as u64))
}

/// Inflate one stream at the start of `data` — zlib, or raw DEFLATE when `raw` — returning the bytes
/// consumed and the inflated output, or `None` when it is not such a stream (a stored chunk). The
/// output is capped at `cap` (and [`MAX_CHUNK_INFLATE`]) so a bomb cannot run away.
///
/// A streaming `read_to_end` returns `Ok` at an early EOF, so a truncated stream would pass as a
/// valid chunk; instead the decoder is driven to `FlushDecompress::Finish` and a chunk is counted
/// only on `Status::StreamEnd`. A stream that does not finish within `data` (or runs past the cap)
/// yields `None`, i.e. it is treated as a stored chunk, so a reader never accepts bad bytes.
fn inflate_one(data: &[u8], raw: bool, cap: u64) -> Option<(usize, Vec<u8>)> {
    use flate2::{Decompress, FlushDecompress, Status};
    let taken = cap.min(MAX_CHUNK_INFLATE);
    let mut decoder = Decompress::new(!raw); // a zlib header, unless raw DEFLATE
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read_in = decoder.total_in() as usize;
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(&data[read_in..], &mut buf, FlushDecompress::Finish)
            .ok()?;
        let produced = (decoder.total_out() - before_out) as usize;
        out.extend_from_slice(&buf[..produced]);
        if out.len() as u64 > taken {
            return None; // past the cap: not a plausible chunk
        }
        match status {
            Status::StreamEnd => return Some((decoder.total_in() as usize, out)),
            // No end and no progress: the stream needs input `data` does not hold, so it is stored.
            _ if produced == 0 => return None,
            _ => {}
        }
    }
}

/// The length of a stored chunk at `pos`. With a real size, a middle chunk is 64 KiB (or the remaining
/// real bytes for the last); with none, the chunk runs to the next 16-aligned zlib head, or to the
/// body's end when there is none.
fn stored_chunk_len(body: &[u8], pos: usize, real: Option<u64>, produced: u64) -> usize {
    match real {
        Some(real) => {
            let remaining = real.saturating_sub(produced);
            (65536u64.min(remaining) as usize).min(body.len() - pos)
        }
        None => {
            let mut p = round16(pos + 16);
            while p + 1 < body.len() {
                if body[p] == 0x78
                    && matches!(body[p + 1], 0xDA | 0x9C | 0x01 | 0x5E)
                    && inflate_one(&body[p..], false, MAX_CHUNK_INFLATE).is_some()
                {
                    return p - pos;
                }
                p += 16;
            }
            body.len() - pos
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A nested archive's header must be found and parsed at `base + start` from the entry row. This
    /// builds a tiny two-level file in memory: a byte run, then a minimal valid RPF9 header, and
    /// checks that an entry whose start points at the header is seen as an archive at the right place.
    #[test]
    fn an_entry_start_locates_a_nested_header() {
        let mut file = vec![0u8; 0x200];
        // A plausible RPF9 magic at 0x100; `entry_begins_archive` only reads the 4-byte magic.
        file[0x100..0x104].copy_from_slice(&RPF9_MAGIC.to_le_bytes());
        let mut src = Cursor::new(file);

        let entry = Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(0x100),
            stored: Some(0x40),
            real: Some(0x40),
            class: None,
            cipher: None,
        };
        let mount = Gta6Mount {
            base: 0,
            archive: Gta6Archive {
                toc: minimal_toc(),
                entries: vec![],
            },
        };
        assert_eq!(mount.absolute_start(&entry), Some(0x100));
        assert!(mount.entry_begins_archive(&mut src, &entry).unwrap());

        // With a non-zero base the start is taken relative to the nested header.
        let nested = Gta6Mount {
            base: 0x40,
            ..mount.clone()
        };
        let child = Gta6Entry {
            start: Some(0xC0),
            ..entry.clone()
        };
        assert_eq!(nested.absolute_start(&child), Some(0x100));
        assert!(nested.entry_begins_archive(&mut src, &child).unwrap());
    }

    #[test]
    fn an_entry_without_a_row_has_no_start_and_is_not_an_archive() {
        let mut src = Cursor::new(vec![0u8; 0x80]);
        let mount = Gta6Mount {
            base: 0,
            archive: Gta6Archive {
                toc: minimal_toc(),
                entries: vec![],
            },
        };
        let rowless = Gta6Entry {
            index: 7,
            name64: 0,
            name32: None,
            name: None,
            start: None,
            stored: None,
            real: None,
            class: None,
            cipher: None,
        };
        // With no entry row, nothing locates the entry: `open_nested` and `read_entry` would both
        // refuse here (they call `absolute_start` first), so this is their error gate.
        assert_eq!(mount.absolute_start(&rowless), None);
        assert!(!mount.entry_begins_archive(&mut src, &rowless).unwrap());
    }

    fn minimal_toc() -> super::super::rpf9_toc::Rpf9Toc {
        super::super::rpf9_toc::Rpf9Toc {
            bytes: vec![0u8; 0x40],
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: vec![],
        }
    }

    /// A parseable `ORGTAVIK` snapshot with no index keys and one content key, so [`GtaViKeys`] can be
    /// built with content material from synthetic (not game-derived) tables. The layout mirrors
    /// [`super::super::rpf9_toc`]'s snapshot format exactly.
    fn content_snapshot(key_id: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"ORGTAVIK");
        out.extend_from_slice(&1u32.to_le_bytes()); // version
        out.extend_from_slice(&0x4342u32.to_le_bytes()); // CBC mode tag
        out.extend_from_slice(&[0u8; 32]); // exe sha256
        out.extend_from_slice(&[0u8; 16]); // bootstrap iv
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend((0..4096).map(|i| i as u8)); // index input tables (unused here)
        out.extend_from_slice(&16384u32.to_le_bytes());
        out.extend((0..16384).map(|i| i as u8)); // index round tables (unused here)
        out.extend_from_slice(&0u32.to_le_bytes()); // zero index key blobs

        // The trailing content section: CTR mode, two shared tables, one content key blob.
        out.extend_from_slice(&0x5243u32.to_le_bytes()); // CTR mode tag
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend((0..4096).map(|i| (i * 3 + 1) as u8)); // content input tables
        out.extend_from_slice(&16384u32.to_le_bytes());
        out.extend((0..16384).map(|i| (i * 5 + 7) as u8)); // content round tables
        out.extend_from_slice(&1u32.to_le_bytes()); // one content key
        let registry_id = (u64::from(key_id) << 32) | 0x0200_0000;
        out.extend_from_slice(&registry_id.to_le_bytes());
        out.extend_from_slice(&4340u32.to_le_bytes());
        out.extend_from_slice(&[0x5d, 0xce, 0x18, 0x27]); // content blob magic
        out.extend((0..60u32).flat_map(|w| w.wrapping_mul(0x12345).to_le_bytes())); // 60 words
        out.extend((0..4096).map(|i| (i * 7 + 3) as u8)); // 16 output maps
        out
    }

    /// A content-ciphered entry reads back to its plaintext: CTR is symmetric, so enciphering with
    /// `content_decrypt` and reading with `read_entry_decrypted` must round-trip through the zlib body.
    #[test]
    fn a_content_ciphered_entry_decrypts_and_inflates() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let key_id = 0x1000_0029u32;
        let iv = [0x11u8; 16];
        let keys =
            GtaViKeys::parse(&content_snapshot(key_id)).expect("snapshot with content parses");
        assert!(keys.has_content_keys());

        // A plaintext body, its zlib stream, then enciphered (CTR being its own inverse) to the stored
        // bytes the archive would hold.
        let plain: Vec<u8> = (0..5000u32).map(|x| (x ^ (x >> 3)) as u8).collect();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&plain).unwrap();
        let zlib = encoder.finish().unwrap();
        let mut stored = zlib.clone();
        stored.resize(stored.len().next_multiple_of(16), 0); // pad as the archive stores it
        keys.content_decrypt(key_id, &iv, &mut stored)
            .expect("encipher via the symmetric CTR");

        let start = 0x40u64;
        let mut file = vec![0u8; start as usize];
        file.extend_from_slice(&stored);
        let src = Cursor::new(file);

        let entry = Gta6Entry {
            index: 3,
            name64: 0,
            name32: None,
            name: None,
            start: Some(start),
            stored: Some(stored.len() as u64),
            real: Some(plain.len() as u64),
            class: Some(0),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id,
                cipher_type: 17,
                iv,
                // F0=0, F1=0x400, F2=0: the whole body is enciphered, which is how this test built it.
                framing: [0, 0x400, 0, 0, 4],
            }),
        };
        let mount = Gta6Mount {
            base: 0,
            archive: Gta6Archive {
                toc: minimal_toc(),
                entries: vec![],
            },
        };
        let got = mount
            .read_entry_decrypted(src, &entry, &keys)
            .expect("a content entry decrypts and inflates");
        assert_eq!(got, plain, "the body round-trips to its plaintext");
    }

    /// A content entry of an unsupported cipher type, and one with no content keys, are refused rather
    /// than mis-read.
    #[test]
    fn an_unsupported_content_entry_is_refused() {
        let keys = GtaViKeys::parse(&content_snapshot(0x1000_0029)).unwrap();
        let mount = Gta6Mount {
            base: 0,
            archive: Gta6Archive {
                toc: minimal_toc(),
                entries: vec![],
            },
        };
        let entry = Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(0x40),
            stored: Some(0x40),
            real: Some(0x40),
            class: Some(0),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id: 0x1000_0029,
                cipher_type: 99, // not type 17
                iv: [0; 16],
                framing: [0, 0x400, 0, 0, 4],
            }),
        };
        let src = Cursor::new(vec![0u8; 0x100]);
        assert!(matches!(
            mount.read_entry_decrypted(src, &entry, &keys),
            Err(IoError::RpfEncryptionUnsupported { .. })
        ));
    }

    /// A cipher record whose framing tuple is none of the five the install uses is refused, not
    /// decoded with a guessed extent.
    #[test]
    fn an_unknown_framing_is_refused() {
        let key_id = 0x1000_0029u32;
        let keys = GtaViKeys::parse(&content_snapshot(key_id)).unwrap();

        let start = 0x40u64;
        let src = Cursor::new(vec![0u8; 0x200]);
        let entry = Gta6Entry {
            index: 5,
            name64: 0,
            name32: None,
            name: None,
            start: Some(start),
            stored: Some(0x80),
            real: Some(0x80),
            class: Some(0),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id,
                cipher_type: 17,
                iv: [0x11; 16],
                // Not one of the five known tuples.
                framing: [0x123, 0, 0, 0, 4],
            }),
        };
        let mount = Gta6Mount {
            base: 0,
            archive: Gta6Archive {
                toc: minimal_toc(),
                entries: vec![],
            },
        };
        assert!(
            mount.read_entry_decrypted(src, &entry, &keys).is_err(),
            "an unrecognised framing tuple must be refused"
        );
    }

    fn zlib_stream(data: &[u8]) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn pad16(v: &mut Vec<u8>) {
        while !v.len().is_multiple_of(16) {
            v.push(0);
        }
    }

    #[test]
    fn a_truncated_zlib_stream_is_not_inflated() {
        let data: Vec<u8> = (0..4000u32).map(|x| (x ^ (x >> 3)) as u8).collect();
        let stream = zlib_stream(&data);
        // The whole stream inflates and reports exactly the bytes it consumed.
        let (consumed, out) = inflate_one(&stream, false, u64::MAX).expect("a whole zlib stream");
        assert_eq!(out, data);
        assert_eq!(consumed, stream.len());
        // A stream cut short is not a stream: it is refused (so the walk treats it as a stored
        // chunk), not accepted with a short `Ok` the way a streaming `read_to_end` would.
        let truncated = &stream[..stream.len() * 2 / 3];
        assert!(inflate_one(truncated, false, u64::MAX).is_none());
    }

    #[test]
    fn the_chunk_walk_round_trips_a_24_byte_body_of_three_chunks() {
        // zlib, a 64 KiB stored chunk (a failed inflate), then zlib — the shape a compressed
        // 24-byte entry is stored in; its real size bounds and ends the walk.
        let a: Vec<u8> = (0..200u32).map(|x| x as u8).collect();
        let b = vec![0x55u8; 65536]; // not a zlib stream, so it walks as a stored chunk
        let c: Vec<u8> = (0..300u32).map(|x| (x * 7) as u8).collect();
        let real = (a.len() + b.len() + c.len()) as u64;

        let mut body = zlib_stream(&a);
        pad16(&mut body);
        body.extend_from_slice(&b); // a 16-aligned 64 KiB stored chunk
        body.extend_from_slice(&zlib_stream(&c));
        pad16(&mut body);

        let (out, chunks, consumed) = walk_chunks(&body, Some(real)).expect("the walk");
        assert_eq!(
            out,
            [a, b, c].concat(),
            "the three chunks concatenate to the plaintext"
        );
        assert_eq!(chunks, 3);
        assert_eq!(consumed, body.len() as u64);
    }

    #[test]
    fn the_chunk_walk_takes_a_raw_deflate_first_chunk() {
        // A nested archive's first entry can be raw DEFLATE, with no zlib header.
        let a: Vec<u8> = (0..1000u32).map(|x| (x ^ 0x5A) as u8).collect();
        let mut body = raw_deflate(&a);
        pad16(&mut body);
        let (out, chunks, _) = walk_chunks(&body, Some(a.len() as u64)).expect("the walk");
        assert_eq!(out, a);
        assert_eq!(chunks, 1);
    }

    #[test]
    fn the_chunk_walk_of_a_20_byte_body_ends_a_stored_chunk_at_the_next_zlib_head() {
        // A 20-byte (script) body states no real size: a stored chunk runs to the next 16-aligned
        // zlib head, and trailing zero padding ends the body.
        let a: Vec<u8> = (0..120u32).map(|x| x as u8).collect();
        let stored = vec![0x55u8; 32]; // non-zlib, 16-aligned length
        let c: Vec<u8> = (0..90u32).map(|x| (x * 3) as u8).collect();

        let mut body = zlib_stream(&a);
        pad16(&mut body);
        body.extend_from_slice(&stored);
        body.extend_from_slice(&zlib_stream(&c));
        pad16(&mut body);
        body.extend_from_slice(&[0u8; 32]); // trailing zero padding ends the body

        let (out, chunks, _) = walk_chunks(&body, None).expect("the walk");
        assert_eq!(
            out,
            [a, stored, c].concat(),
            "zlib, stored-to-next-head, zlib"
        );
        assert_eq!(chunks, 3);
    }
}
