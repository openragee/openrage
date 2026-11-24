//! Bounded entry reads whose layout comes only from the RPF9 table.

use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use super::{Gta6Archive, Gta6Cipher, Gta6Class, Gta6Entry, Gta6EntryExtra, GtaViKeys};
use crate::error::{IoError, Result};

/// An entry's validated stored extent and decoding instructions, detached from its table.
#[derive(Clone)]
pub struct Gta6EntryRead {
    extent: Range<u64>,
    real: usize,
    chunks: Option<u32>,
    cipher: Option<Gta6Cipher>,
}

fn bad(why: impl Into<String>) -> IoError {
    IoError::Invalid {
        what: "an RPF9 table entry".into(),
        why: why.into(),
    }
}

impl Gta6EntryRead {
    /// Validate an entry's layout and sizes before reading or allocating its body.
    pub fn from_table(
        archive: &Gta6Archive,
        extent: Range<u64>,
        index: u32,
        limit: u64,
    ) -> Result<Self> {
        let entry = archive
            .entries
            .get(index as usize)
            .ok_or_else(|| bad("entry index is out of range"))?;
        let extra = archive
            .entry_extra(index as usize)
            .ok_or_else(|| bad("entry columns are unreadable"))?;
        Self::plan(entry, extra, archive.toc.bytes.len() as u64, extent, limit)
    }

    fn plan(
        entry: &Gta6Entry,
        extra: Gta6EntryExtra,
        table_len: u64,
        parent: Range<u64>,
        limit: u64,
    ) -> Result<Self> {
        let (Some(start), Some(stored), Some(real)) = (entry.start, entry.stored, entry.real)
        else {
            return Err(bad("this row does not state a supported stored and decoded size; 20-byte resource rows are not decoded"));
        };
        let begin = parent
            .start
            .checked_add(start)
            .ok_or_else(|| bad("entry offset overflow"))?;
        let end = begin
            .checked_add(stored)
            .ok_or_else(|| bad("entry size overflow"))?;
        if parent.start > parent.end
            || start < table_len
            || end > parent.end
            || stored > limit
            || real > limit
            || stored % 16 != 0
        {
            return Err(bad("entry exceeds its parent archive or the size limit, overlaps its table, or is not aligned"));
        }
        let chunks = match (entry.kind(), extra.chunk_count) {
            (Some(Gta6Class::Media | Gta6Class::NestedArchive), None)
                if real.checked_add(15).map(|n| n & !15) == Some(stored) =>
            {
                None
            }
            (Some(Gta6Class::CompressedResource), Some(count))
                if count > 0 && u64::from(count) <= stored / 16 =>
            {
                Some(count)
            }
            _ => {
                return Err(bad(
                    "unsupported entry layout or inconsistent sizes/chunk count",
                ))
            }
        };
        if let Some(cipher) = entry.cipher {
            if cipher.cipher_type != 17 {
                return Err(IoError::RpfEncryptionUnsupported {
                    key_id: cipher.key_id,
                });
            }
            if let Some(reason) =
                super::gta6_content::framing_reject_reason(&cipher.framing, stored)
            {
                return Err(bad(reason));
            }
        }
        usize::try_from(stored).map_err(|_| bad("stored size exceeds addressable memory"))?;
        let real =
            usize::try_from(real).map_err(|_| bad("decoded size exceeds addressable memory"))?;
        Ok(Self {
            extent: begin..end,
            real,
            chunks,
            cipher: entry.cipher,
        })
    }

    /// Read exactly the table's extent and decode it without searching for signatures or chunk boundaries.
    pub fn read<R: Read + Seek>(&self, source: &mut R, keys: &GtaViKeys) -> Result<Vec<u8>> {
        if let Some(cipher) = self.cipher {
            keys.content_decrypt(cipher.key_id, &cipher.iv, &mut [])?;
        }
        let io = |e: std::io::Error| bad(e.to_string());
        if source.seek(SeekFrom::End(0)).map_err(io)? < self.extent.end {
            return Err(bad("entry extends past the source file"));
        }
        source
            .seek(SeekFrom::Start(self.extent.start))
            .map_err(io)?;
        let mut body = vec![0; (self.extent.end - self.extent.start) as usize];
        source.read_exact(&mut body).map_err(io)?;
        if let Some(cipher) = self.cipher {
            let mut result = Ok(());
            super::gta6_content::decrypt_extent(
                &mut body,
                &cipher.framing,
                &cipher.iv,
                |slice, counter| {
                    if result.is_ok() {
                        result = keys.content_decrypt(cipher.key_id, counter, slice);
                    }
                },
            );
            result?;
        }
        match self.chunks {
            Some(count) => inflate(&body, count, self.real),
            None => {
                body.truncate(self.real);
                Ok(body)
            }
        }
    }
}

fn inflate(body: &[u8], count: u32, real: usize) -> Result<Vec<u8>> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut buffer = [0; 64 * 1024];
    for _ in 0..count {
        let mut decoder = Decompress::new(true);
        loop {
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            let input = body
                .get(at + before_in as usize..)
                .ok_or_else(|| bad("chunk extends past its entry"))?;
            let status = decoder
                .decompress(input, &mut buffer, FlushDecompress::Finish)
                .map_err(|e| {
                    bad(format!(
                        "invalid zlib chunk: {e}; stored or raw-DEFLATE chunks are not supported"
                    ))
                })?;
            let made = (decoder.total_out() - before_out) as usize;
            if made > real.saturating_sub(out.len()) {
                return Err(bad("chunks exceed the decoded size stated by the table"));
            }
            out.extend_from_slice(&buffer[..made]);
            if status == Status::StreamEnd {
                break;
            }
            if made == 0 && decoder.total_in() == before_in {
                return Err(bad("truncated zlib chunk"));
            }
        }
        at = at
            .checked_add(decoder.total_in() as usize)
            .and_then(|n| n.checked_add(15))
            .map(|n| n & !15)
            .ok_or_else(|| bad("chunk size overflow"))?;
    }
    if at != body.len() || out.len() != real {
        return Err(bad(
            "chunk count or decoded/stored size does not match the table",
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor, Write};

    struct Bounded {
        source: Cursor<Vec<u8>>,
        allowed: Range<u64>,
        reads: usize,
    }

    impl Read for Bounded {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let at = self.source.position();
            assert!(
                at >= self.allowed.start && at + buf.len() as u64 <= self.allowed.end,
                "read outside entry"
            );
            self.reads += 1;
            self.source.read(buf)
        }
    }

    impl Seek for Bounded {
        fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
            self.source.seek(at)
        }
    }

    fn entry(stored: u64, real: u64, class: u8) -> Gta6Entry {
        Gta6Entry {
            index: 0,
            name64: 0,
            name32: None,
            name: None,
            start: Some(128),
            stored: Some(stored),
            real: Some(real),
            class: Some(class),
            cipher: None,
        }
    }

    fn keys() -> GtaViKeys {
        GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap()
    }

    fn source(bytes: &[u8]) -> Bounded {
        let mut data = vec![0x78; 1024];
        data[256..256 + bytes.len()].copy_from_slice(bytes);
        Bounded {
            source: Cursor::new(data),
            allowed: 256..256 + bytes.len() as u64,
            reads: 0,
        }
    }

    fn plan(row: &Gta6Entry, chunks: Option<u32>) -> Result<Gta6EntryRead> {
        Gta6EntryRead::plan(
            row,
            Gta6EntryExtra {
                chunk_count: chunks,
                ..Default::default()
            },
            64,
            128..1024,
            1024,
        )
    }

    fn zlib(bytes: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        let mut stored = encoder.finish().unwrap();
        stored.resize(stored.len().next_multiple_of(16), 0);
        stored
    }

    #[test]
    fn raw_bytes_use_the_table_start_and_real_size_without_detection() {
        let mut src = source(b"arbitrary bytesP");
        assert_eq!(
            plan(&entry(16, 15, 255), None)
                .unwrap()
                .read(&mut src, &keys())
                .unwrap(),
            b"arbitrary bytes"
        );
        assert_eq!(src.reads, 1);
    }

    #[test]
    fn counted_zlib_streams_end_at_the_stated_size() {
        let mut stored = zlib(b"first chunk");
        stored.extend(zlib(b"second chunk"));
        let mut src = source(&stored);
        let row = entry(stored.len() as u64, 23, 11);
        assert_eq!(
            plan(&row, Some(2))
                .unwrap()
                .read(&mut src, &keys())
                .unwrap(),
            b"first chunksecond chunk"
        );
    }

    #[test]
    fn malformed_compression_never_becomes_raw_bytes_or_a_later_stream() {
        let mut stored = vec![0; 16];
        stored.extend(zlib(b"decoy stream"));
        let row = entry(stored.len() as u64, 12, 11);
        assert!(plan(&row, Some(1))
            .unwrap()
            .read(&mut source(&stored), &keys())
            .is_err());
        let mut stored = zlib(b"broken checksum");
        stored[3] ^= 0x55;
        assert!(plan(&entry(stored.len() as u64, 15, 11), Some(1))
            .unwrap()
            .read(&mut source(&stored), &keys())
            .is_err());
    }

    #[test]
    fn size_count_and_trailing_stream_mismatches_are_errors() {
        let mut stored = zlib(b"first");
        stored.extend(zlib(b"second"));
        for (count, real) in [(1, 5), (2, 10), (2, 12), (3, 11)] {
            assert!(plan(&entry(stored.len() as u64, real, 11), Some(count))
                .and_then(|read| read.read(&mut source(&stored), &keys()))
                .is_err());
        }
    }

    #[test]
    fn unsupported_rows_and_out_of_bounds_extents_are_refused() {
        assert!(plan(&entry(16, 16, 14), None).is_err());
        assert!(plan(&entry(16, 16, 11), None).is_err());
        assert!(plan(&entry(16, 16, 11), Some(0)).is_err());
        assert!(plan(&entry(16, 17, 255), None).is_err());
        assert!(plan(&entry(32, 1, 255), None).is_err());
        assert!(plan(&entry(2048, 2048, 255), None).is_err());
        let mut row = entry(16, 16, 255);
        row.start = Some(32);
        assert!(plan(&row, None).is_err());
        row.start = Some(u64::MAX - 127);
        assert!(plan(&row, None).is_err());
        row.stored = None;
        assert!(plan(&row, None).is_err());
    }

    #[test]
    fn a_missing_content_key_refuses_before_reading_any_body() {
        let mut row = entry(16, 16, 255);
        row.cipher = Some(Gta6Cipher {
            key_id: 1,
            cipher_type: 17,
            iv: [0; 16],
            framing: [0, 0x400, 0, 0, 4],
        });
        let mut src = source(&[0; 16]);
        assert!(plan(&row, None).unwrap().read(&mut src, &keys()).is_err());
        assert_eq!(src.reads, 0);
    }

    #[test]
    fn encrypted_zlib_uses_the_content_key_before_decoding() {
        let mut snapshot = or_test_support::rpf9::keys();
        snapshot.extend(0x5243u32.to_le_bytes());
        for size in [4096u32, 16384] {
            snapshot.extend(size.to_le_bytes());
            snapshot.extend(vec![0; size as usize]);
        }
        snapshot.extend(1u32.to_le_bytes());
        snapshot.extend(0x0000_0001_0200_0000u64.to_le_bytes());
        snapshot.extend((4u32 + 60 * 4 + 16 * 256).to_le_bytes());
        snapshot.extend([0x5d, 0xce, 0x18, 0x27]);
        snapshot.extend([0; 60 * 4]);
        for lane in 0..16u8 {
            snapshot.extend([0x80 + lane; 256]);
        }
        let keys = GtaViKeys::parse(&snapshot).unwrap();
        let expected = b"synthetic encrypted content";
        let mut body = zlib(expected);
        // Constant output maps make the expected keystream independent of the cipher implementation.
        for (i, byte) in body.iter_mut().enumerate() {
            *byte ^= 0x80 + (i % 16) as u8;
        }
        let mut row = entry(body.len() as u64, expected.len() as u64, 11);
        row.cipher = Some(Gta6Cipher {
            key_id: 1,
            cipher_type: 17,
            iv: [0; 16],
            framing: [0, 0x400, 0, 0, 4],
        });
        assert_eq!(
            plan(&row, Some(1))
                .unwrap()
                .read(&mut source(&body), &keys)
                .unwrap(),
            expected
        );
        row.cipher.as_mut().unwrap().cipher_type = 19;
        assert!(plan(&row, Some(1)).is_err());
        row.cipher.as_mut().unwrap().cipher_type = 17;
        row.cipher.as_mut().unwrap().framing[4] = 5;
        assert!(plan(&row, Some(1)).is_err());
    }

    #[test]
    fn a_truncated_source_is_rejected_before_allocating_or_reading() {
        let mut src = source(&[0; 16]);
        src.source.get_mut().truncate(256);
        assert!(plan(&entry(16, 16, 255), None)
            .unwrap()
            .read(&mut src, &keys())
            .is_err());
        assert_eq!(src.reads, 0);
    }
}
