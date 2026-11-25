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
use std::ops::Range;

use byteorder::{ByteOrder, LE};

use super::gta6_archive::{Gta6Archive, Gta6Entry};
use super::gta6_entry::Gta6EntryRead;
use super::rpf9::{Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC};
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
    let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
    open_table(src, offset..end, keys, u64::MAX)
}

/// Open only the header and table of an archive within its containing entry's bounds.
pub fn open_at_bounded<R: Read + Seek>(
    src: &mut R,
    extent: Range<u64>,
    keys: &GtaViKeys,
) -> Result<Gta6Archive> {
    open_table(src, extent, keys, 64 << 20)
}

fn open_table<R: Read + Seek>(
    src: &mut R,
    extent: Range<u64>,
    keys: &GtaViKeys,
    budget: u64,
) -> Result<Gta6Archive> {
    let bad = |why: &str| IoError::Invalid {
        what: "an RPF9 table".into(),
        why: why.into(),
    };
    let len = extent
        .end
        .checked_sub(extent.start)
        .filter(|len| *len >= RPF9_HEADER_SIZE as u64)
        .ok_or_else(|| bad("the archive extent is shorter than its header"))?;
    let head = read_at(src, extent.start, RPF9_HEADER_SIZE)?;
    let header = Rpf9Header::parse(&head)?;
    // Bound both the table bytes and the expanded entry rows before allocating either.
    let listing_bytes = u64::from(header.entry_count) * std::mem::size_of::<Gta6Entry>() as u64;
    if header.toc_end < RPF9_HEADER_SIZE as u32
        || u64::from(header.toc_end) > len
        || u64::from(header.toc_end) + listing_bytes > budget
    {
        return Err(bad(
            "the table exceeds its archive extent or the table size limit",
        ));
    }
    let mut toc = head;
    toc.resize(header.toc_end as usize, 0);
    src.read_exact(&mut toc[RPF9_HEADER_SIZE..])
        .map_err(mount_io)?;
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
        entry.start.and_then(|start| self.base.checked_add(start))
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
        let stored = entry.stored.ok_or_else(|| no_entry_row(entry))?;
        let end = start
            .checked_add(stored)
            .ok_or_else(|| no_entry_row(entry))?;
        let source_end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        if entry
            .start
            .is_none_or(|at| at < self.archive.toc.bytes.len() as u64)
            || end > source_end
        {
            return Err(no_entry_row(entry));
        }
        Ok(Self {
            base: start,
            archive: open_table(src, start..end, keys, u64::MAX)?,
        })
    }

    /// Validate an entry's detached read plan against its containing archive and size limit.
    pub fn read_entry_plan(
        &self,
        index: u32,
        parent_end: u64,
        limit: u64,
    ) -> Result<Gta6EntryRead> {
        Gta6EntryRead::from_table(&self.archive, self.base..parent_end, index, limit)
    }

    /// Read a clear entry using only its table's decoding instructions.
    pub fn read_entry<R: Read + Seek>(&self, mut src: R, entry: &Gta6Entry) -> Result<Vec<u8>> {
        let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        self.read_entry_plan(entry.index, end, u64::MAX)?
            .read_clear(&mut src)
    }

    /// Read an entry through its table, deciphering its stored body when its row requires it.
    pub fn read_entry_decrypted<R: Read + Seek>(
        &self,
        src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Vec<u8>> {
        self.read_entry_decrypted_report(src, entry, keys)
            .map(|report| report.data)
    }

    /// Read through the table and report the decoded chunks and exact stored bytes consumed.
    pub fn read_entry_decrypted_report<R: Read + Seek>(
        &self,
        mut src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Gta6DecryptedBody> {
        let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        let plan = self.read_entry_plan(entry.index, end, u64::MAX)?;
        let data = plan.read(&mut src, keys)?;
        Ok(Gta6DecryptedBody {
            data,
            chunk_count: plan.chunk_count(),
            stored_consumed: plan.stored_len(),
        })
    }

    /// Where a ciphered entry's body sits and how it is sized: a 24-byte row carries the body start,
    /// the stored size and the real (inflated) size; a 20-byte row carries only a proven start, so its
    /// body (16 bytes past the slot, `w8 * 16 - 16` long, real size unreliable) is resolved from the
    /// archive's nested descriptor. Returns `(absolute body start, stored length, real size)`.
    fn body_geometry(&self, entry: &Gta6Entry) -> Result<(u64, u64, Option<u64>)> {
        let start = self
            .absolute_start(entry)
            .filter(|_| {
                entry
                    .start
                    .is_some_and(|at| at >= self.archive.toc.bytes.len() as u64)
            })
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
        let body_start = start.checked_add(16).ok_or_else(|| no_entry_row(entry))?;
        let stored = nested
            .stored_size()
            .checked_sub(16)
            .ok_or_else(|| no_entry_row(entry))?;
        Ok((body_start, stored, None))
    }
}

impl Gta6Mount {
    /// Read and decipher the stored body without inflating it or searching its contents.
    pub fn read_entry_deciphered_stored<R: Read + Seek>(
        &self,
        mut src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Vec<u8>> {
        let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        self.read_entry_deciphered_stored_bounded(src, entry, keys, end)
    }

    pub(super) fn read_entry_deciphered_stored_bounded<R: Read + Seek>(
        &self,
        mut src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
        parent_end: u64,
    ) -> Result<Vec<u8>> {
        let body_bad = |why: String| IoError::Invalid {
            what: "a content-ciphered RPF9 entry".into(),
            why,
        };
        let Some(cipher) = entry.cipher else {
            return Err(body_bad(format!(
                "entry {} is not content-ciphered, so it has no enciphered stored body",
                entry.index
            )));
        };
        if cipher.cipher_type != CONTENT_CIPHER_TYPE {
            return Err(IoError::RpfEncryptionUnsupported {
                key_id: cipher.key_id,
            });
        }
        let (body_start, stored, _real) = self.body_geometry(entry)?;
        if stored == 0 {
            return Err(body_bad(format!("entry {} has an empty body", entry.index)));
        }
        // A malformed framing is refused, not guessed — before the body is read.
        if let Some(reason) = super::gta6_content::framing_reject_reason(&cipher.framing, stored) {
            return Err(body_bad(format!("entry {} {reason}", entry.index)));
        }
        // Check the stored extent against the source before allocating, so a malformed row cannot
        // force a huge allocation.
        let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        if parent_end < self.base
            || body_start
                .checked_add(stored)
                .is_none_or(|s| s > parent_end.min(end))
            || stored % 16 != 0
        {
            return Err(body_bad(format!(
                "entry {}'s stored extent runs past the source",
                entry.index
            )));
        }
        keys.content_decrypt(cipher.key_id, &cipher.iv, &mut [])?;
        let stored = usize::try_from(stored)
            .map_err(|_| body_bad("stored size exceeds addressable memory".into()))?;
        let mut body = read_at(&mut src, body_start, stored)?;
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
        Ok(body)
    }

    /// Inflate already deciphered bytes using the entry's table, without applying its cipher again.
    pub fn inflate_deciphered_stored(&self, stored: &[u8], entry: &Gta6Entry) -> Result<Vec<u8>> {
        self.read_entry_plan(entry.index, u64::MAX, u64::MAX)?
            .read_deciphered(stored)
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

#[cfg(test)]
mod tests {
    use super::super::gta6_chunk_table::Chunk;
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

    fn synthetic_mount(chunks: &[Chunk]) -> (Gta6Mount, Gta6Entry) {
        use super::super::{Rpf9Builder, Rpf9Toc};
        let keys = GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap();
        let descriptors: Vec<_> = chunks
            .iter()
            .map(|c| (c.inflated, c.stored, c.zlib))
            .collect();
        let plain = or_test_support::rpf9::chunked_table(11, &descriptors);
        let bytes = Rpf9Builder::serialize(
            &Rpf9Toc {
                bytes: plain,
                key_id: 0,
                registry_id: 0,
                entry_count: 0,
                groups: Vec::new(),
            },
            &keys,
        )
        .unwrap();
        let mount = Gta6Mount {
            base: 0,
            archive: Gta6Archive::open(&bytes, &keys).unwrap(),
        };
        let entry = mount.archive.entries[0].clone();
        (mount, entry)
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

        let start = 0x1000u64;
        let mut file = vec![0u8; start as usize];
        file.extend_from_slice(&stored);
        let src = Cursor::new(file);

        let entry = Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(start),
            stored: Some(stored.len() as u64),
            real: Some(plain.len() as u64),
            class: Some(11),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id,
                cipher_type: 17,
                iv,
                // F0=0, F1=0x400, F2=0: the whole body is enciphered, which is how this test built it.
                framing: [0, 0x400, 0, 0, 4],
            }),
        };
        let (mut mount, _) = synthetic_mount(&[Chunk {
            inflated: plain.len() as u32,
            stored: zlib.len() as u32,
            zlib: true,
        }]);
        mount.archive.entries[0] = entry.clone();
        let got = mount
            .read_entry_decrypted(src, &entry, &keys)
            .expect("a content entry decrypts and inflates");
        assert_eq!(got, plain, "the body round-trips to its plaintext");
    }

    /// The deciphered STORED bytes of a content-ciphered entry are its padded zlib with the enciphered
    /// range turned back to plaintext — the form the table-free resource reader classifies — and those
    /// bytes inflate through [`Gta6Mount::inflate_deciphered_stored`] with no second decipher. A clear
    /// entry has no enciphered stored body and is refused.
    #[test]
    fn deciphered_stored_bytes_are_the_padded_plaintext_zlib() {
        let key_id = 0x1000_0029u32;
        let iv = [0x11u8; 16];
        let keys =
            GtaViKeys::parse(&content_snapshot(key_id)).expect("snapshot with content parses");

        // The stored (clear) bytes an archive would hold: the plaintext's zlib, 16-padded. Enciphering
        // the whole extent (the `[0, 0x400, 0, 0, 4]` framing covers it) gives the bytes on disk.
        let plain: Vec<u8> = (0..5000u32).map(|x| (x ^ (x >> 3)) as u8).collect();
        let mut stored = zlib_stream(&plain);
        stored.resize(stored.len().next_multiple_of(16), 0);
        let mut enciphered = stored.clone();
        keys.content_decrypt(key_id, &iv, &mut enciphered)
            .expect("encipher via the symmetric CTR");

        let start = 0x1000u64;
        let mut file = vec![0u8; start as usize];
        file.extend_from_slice(&enciphered);
        let src = Cursor::new(file);

        let entry = Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(start),
            stored: Some(enciphered.len() as u64),
            real: Some(plain.len() as u64),
            class: Some(11),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id,
                cipher_type: 17,
                iv,
                framing: [0, 0x400, 0, 0, 4],
            }),
        };
        let (mut mount, _) = synthetic_mount(&[Chunk {
            inflated: plain.len() as u32,
            stored: zlib_stream(&plain).len() as u32,
            zlib: true,
        }]);
        mount.archive.entries[0] = entry.clone();
        let got = mount
            .read_entry_deciphered_stored(src, &entry, &keys)
            .expect("the deciphered stored bytes");
        assert_eq!(
            got, stored,
            "the enciphered range is turned back to the stored plaintext zlib"
        );
        // The same bytes inflate without re-deciphering, to the plaintext the row's real size states.
        let inflated = mount
            .inflate_deciphered_stored(&got, &entry)
            .expect("the stored bytes inflate");
        assert_eq!(
            inflated, plain,
            "inflating the deciphered stored bytes gives the plaintext"
        );

        // A clear entry (no cipher record) has no enciphered stored body.
        let clear = Gta6Entry {
            cipher: None,
            ..entry.clone()
        };
        let clear_src = Cursor::new(vec![0u8; 0x100]);
        assert!(
            mount
                .read_entry_deciphered_stored(clear_src, &clear, &keys)
                .is_err(),
            "a clear entry has no enciphered stored body"
        );
    }

    /// A content entry of an unsupported cipher type, and one with no content keys, are refused rather
    /// than mis-read.
    #[test]
    fn an_unsupported_content_entry_is_refused() {
        let keys = GtaViKeys::parse(&content_snapshot(0x1000_0029)).unwrap();
        let (mut mount, _) = synthetic_mount(&[Chunk {
            inflated: 64,
            stored: 64,
            zlib: false,
        }]);
        let entry = Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(0x1000),
            stored: Some(0x40),
            real: Some(0x40),
            class: Some(11),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id: 0x1000_0029,
                cipher_type: 99, // not type 17
                iv: [0; 16],
                framing: [0, 0x400, 0, 0, 4],
            }),
        };
        mount.archive.entries[0] = entry.clone();
        let src = Cursor::new(vec![0u8; 0x1040]);
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

        let start = 0x1000u64;
        let src = Cursor::new(vec![0u8; 0x1080]);
        let entry = Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(start),
            stored: Some(0x80),
            real: Some(0x80),
            class: Some(11),
            cipher: Some(super::super::gta6_archive::Gta6Cipher {
                key_id,
                cipher_type: 17,
                iv: [0x11; 16],
                // Not one of the five known tuples.
                framing: [0x123, 0, 0, 0, 4],
            }),
        };
        let (mut mount, _) = synthetic_mount(&[Chunk {
            inflated: 128,
            stored: 128,
            zlib: false,
        }]);
        mount.archive.entries[0] = entry.clone();
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

    fn pad16(v: &mut Vec<u8>) {
        v.resize(v.len().next_multiple_of(16), 0);
    }

    /// zlib, then a 64-byte stored chunk whose bytes from offset 16 happen to be a whole zlib stream,
    /// then zlib: the body, its chunk table and the plaintext.
    fn body_with_a_zlib_lookalike() -> (Vec<u8>, Vec<Chunk>, Vec<u8>) {
        let a: Vec<u8> = (0..120u32).map(|x| x as u8).collect();
        let mut stored = vec![0x55u8; 16];
        stored.extend_from_slice(&zlib_stream(b"looks like a chunk"));
        stored.resize(64, 0x55);
        let c: Vec<u8> = (0..90u32).map(|x| (x * 3) as u8).collect();
        let (za, zc) = (zlib_stream(&a), zlib_stream(&c));

        let mut body = za.clone();
        pad16(&mut body);
        body.extend_from_slice(&stored);
        body.extend_from_slice(&zc);
        pad16(&mut body);
        let chunk = |inflated: usize, stored: usize, zlib| Chunk {
            inflated: inflated as u32,
            stored: stored as u32,
            zlib,
        };
        let table = vec![
            chunk(a.len(), za.len(), true),
            chunk(64, 64, false),
            chunk(c.len(), zc.len(), true),
        ];
        (body, table, [a, stored, c].concat())
    }

    #[test]
    fn a_mount_walks_an_entry_with_a_chunk_row_by_its_table() {
        let (body, chunks, plain) = body_with_a_zlib_lookalike();
        let (mut mount, _) = synthetic_mount(&chunks);
        let entry = mount.archive.entries[0].clone();
        assert_eq!(
            mount.inflate_deciphered_stored(&body, &entry).unwrap(),
            plain
        );
        let mut file = vec![0; 0x1000];
        file.extend(&body);
        assert_eq!(mount.read_entry(Cursor::new(&file), &entry).unwrap(), plain);
        let keys = GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap();
        let report = mount
            .read_entry_decrypted_report(Cursor::new(&file), &entry, &keys)
            .unwrap();
        assert_eq!(report.data, plain);
        assert_eq!(report.chunk_count, chunks.len());
        assert_eq!(report.stored_consumed, body.len() as u64);
        mount
            .archive
            .toc
            .groups
            .retain(|g| g.name_hash != super::super::gta6_archive::GROUP_CHUNK_RECORD);
        assert!(mount.inflate_deciphered_stored(&body, &entry).is_err());
    }
}
