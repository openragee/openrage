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

    /// Read `entry`'s body, decrypting it first when it is content-ciphered (type 17). A clear entry is
    /// read by [`Self::read_entry`]; a ciphered entry's stored bytes are read at `base + start`,
    /// CTR-decrypted with `keys`, and the resulting zlib stream inflated to exactly the row's real size.
    ///
    /// Only the fully-enciphered class is read: the whole stored body is one zlib stream from the IV,
    /// inflating to the row's stated real size. A body whose encrypted extent is a prefix or a paged
    /// RSC8 window leaves the continuous-CTR pass with garbage past the enciphered extent, so its
    /// plaintext does not inflate to the real size and the read is refused rather than returned wrong;
    /// decoding that framing is follow-up work with the entry reader, where the page table is parsed.
    pub fn read_entry_decrypted<R: Read + Seek>(
        &self,
        mut src: R,
        entry: &Gta6Entry,
        keys: &GtaViKeys,
    ) -> Result<Vec<u8>> {
        let Some(cipher) = entry.cipher else {
            return self.read_entry(src, entry);
        };
        if cipher.cipher_type != CONTENT_CIPHER_TYPE {
            return Err(IoError::RpfEncryptionUnsupported {
                key_id: cipher.key_id,
            });
        }
        let start = self
            .absolute_start(entry)
            .ok_or_else(|| no_entry_row(entry))?;
        let body_bad = |why: String| IoError::Invalid {
            what: "a content-ciphered RPF9 entry".into(),
            why,
        };
        let stored = entry.stored.ok_or_else(|| {
            body_bad(format!(
                "entry {} has no stored size, so its enciphered extent is unknown",
                entry.index
            ))
        })?;
        // The stated inflated size bounds the output: the fully-enciphered class states one, and
        // without it a malformed stream could not be told from a short one.
        let real = entry.real.ok_or_else(|| {
            body_bad(format!(
                "entry {} states no inflated size, so its body cannot be bounded",
                entry.index
            ))
        })?;
        // Check the stored extent against the source before allocating, so a malformed row cannot
        // force a huge allocation that `read_at` would attempt.
        let end = src.seek(SeekFrom::End(0)).map_err(mount_io)?;
        if start.checked_add(stored).is_none_or(|s| s > end) {
            return Err(body_bad(format!(
                "entry {}'s stored extent runs past the source",
                entry.index
            )));
        }
        let mut body = read_at(&mut src, start, stored as usize)?;
        keys.content_decrypt(cipher.key_id, &cipher.iv, &mut body)?;
        let out = inflate_zlib(&body, real).map_err(|source| {
            body_bad(format!(
                "its plaintext is not a whole zlib stream: {source}"
            ))
        })?;
        if out.len() as u64 != real {
            return Err(body_bad(format!(
                "its plaintext inflated to {} bytes, not the {real} the row states (the body is \
                 likely only partly enciphered, which this reader does not decode)",
                out.len()
            )));
        }
        Ok(out)
    }
}

/// Inflate the zlib stream at the start of `bytes`, capped one byte past `real`: a correct body
/// inflates to exactly `real`, the extra byte lets the caller tell an over-long (bomb) stream from an
/// exact one, and trailing padding after the stream's end is ignored.
fn inflate_zlib(bytes: &[u8], real: u64) -> std::io::Result<Vec<u8>> {
    let cap = real.saturating_add(1);
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(bytes)
        .take(cap)
        .read_to_end(&mut out)?;
    Ok(out)
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
            }),
        };
        let src = Cursor::new(vec![0u8; 0x100]);
        assert!(matches!(
            mount.read_entry_decrypted(src, &entry, &keys),
            Err(IoError::RpfEncryptionUnsupported { .. })
        ));
    }

    /// A body whose enciphered extent is only a prefix (the rest stored clear) must be refused, not
    /// returned partly garbage: the continuous-CTR pass turns the clear tail into noise, so the
    /// plaintext does not inflate to the stated real size.
    #[test]
    fn a_partly_enciphered_body_is_refused() {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;

        let key_id = 0x1000_0029u32;
        let iv = [0x11u8; 16];
        let keys = GtaViKeys::parse(&content_snapshot(key_id)).unwrap();

        let plain: Vec<u8> = (0..5000u32).map(|x| (x ^ (x >> 3)) as u8).collect();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&plain).unwrap();
        let zlib = encoder.finish().unwrap();
        assert!(zlib.len() > 16, "the body must span more than one block");

        // The on-disk body: only the first 16-byte block is enciphered, the rest is stored clear.
        let mut stored = zlib.clone();
        stored.resize(stored.len().next_multiple_of(16), 0);
        let mut head: Vec<u8> = stored[..16].to_vec();
        keys.content_decrypt(key_id, &iv, &mut head).unwrap();
        stored[..16].copy_from_slice(&head);

        let start = 0x40u64;
        let mut file = vec![0u8; start as usize];
        file.extend_from_slice(&stored);
        let src = Cursor::new(file);

        let entry = Gta6Entry {
            index: 4,
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
            "a prefix-enciphered body must not read back as a whole body"
        );
    }
}
