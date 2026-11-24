//! Bounded entry reads whose layout comes only from the RPF9 table.

use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use super::gta6_chunk_table::{self, Chunk, Table};
use super::{Gta6Archive, Gta6Cipher, Gta6Class, Gta6Entry, Gta6EntryExtra, GtaViKeys};
use crate::error::{IoError, Result};

/// An entry's validated stored extent and decoding instructions, detached from its table.
#[derive(Clone)]
pub struct Gta6EntryRead {
    extent: Range<u64>,
    real: usize,
    decoding: Decoding,
    cipher: Option<Gta6Cipher>,
}

#[derive(Clone)]
enum Decoding {
    Raw,
    Table(Vec<Chunk>),
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
        let table = if let Some(at) = gta6_chunk_table::cell_offset(archive, index as usize) {
            let heap = &archive.toc.bytes[at..];
            let mut cell = [0; 8];
            cell.copy_from_slice(&heap[..8]);
            Some(gta6_chunk_table::decode(cell, heap)?)
        } else {
            None
        };
        Self::plan(
            entry,
            extra,
            table,
            archive.toc.bytes.len() as u64,
            extent,
            limit,
        )
    }

    fn plan(
        entry: &Gta6Entry,
        extra: Gta6EntryExtra,
        table: Option<Table>,
        table_len: u64,
        parent: Range<u64>,
        limit: u64,
    ) -> Result<Self> {
        let start = entry.start.ok_or_else(|| bad("entry has no start"))?;
        let (stored, real, skip, decoding) = match (entry.stored, entry.real, extra.nested) {
            (None, None, Some(row)) if entry.class.is_none() => {
                let table =
                    table.ok_or_else(|| bad("a 20-byte resource row requires a chunk table"))?;
                let (body, real) = table_sizes(&table, 2, extra.chunk_count)?;
                let stored = body + 16;
                if stored != row.stored_size() {
                    return Err(bad(
                        "chunk sizes do not match the 20-byte row's stored size",
                    ));
                }
                (stored, real, 16, Decoding::Table(table.chunks))
            }
            (Some(stored), Some(real), None) => {
                let decoding = match (entry.kind(), table) {
                    (Some(Gta6Class::Media | Gta6Class::NestedArchive), None)
                        if extra.chunk_count.is_none()
                            && real.checked_add(15).map(|n| n & !15) == Some(stored) =>
                    {
                        Decoding::Raw
                    }
                    (Some(Gta6Class::Media | Gta6Class::CompressedResource), Some(table)) => {
                        if table_sizes(&table, 3, extra.chunk_count)? != (stored, real) {
                            return Err(bad("chunk sizes do not match the 24-byte row's sizes"));
                        }
                        Decoding::Table(table.chunks)
                    }
                    _ => {
                        return Err(bad(
                            "unsupported entry layout or inconsistent sizes/chunk count",
                        ))
                    }
                };
                (stored, real, 0, decoding)
            }
            _ => return Err(bad("unsupported or conflicting entry rows")),
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
        if let Some(cipher) = entry.cipher {
            if cipher.cipher_type != 17 {
                return Err(IoError::RpfEncryptionUnsupported {
                    key_id: cipher.key_id,
                });
            }
            if let Some(reason) =
                super::gta6_content::framing_reject_reason(&cipher.framing, stored - skip)
            {
                return Err(bad(reason));
            }
        }
        usize::try_from(stored).map_err(|_| bad("stored size exceeds addressable memory"))?;
        let real =
            usize::try_from(real).map_err(|_| bad("decoded size exceeds addressable memory"))?;
        Ok(Self {
            extent: begin + skip..end,
            real,
            decoding,
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
        match &self.decoding {
            Decoding::Table(chunks) => inflate_table(&body, chunks),
            Decoding::Raw => {
                body.truncate(self.real);
                Ok(body)
            }
        }
    }
}

fn table_sizes(table: &Table, mode: u8, count: Option<u32>) -> Result<(u64, u64)> {
    if table.mode != mode || table.chunks.is_empty() || count != Some(table.chunks.len() as u32) {
        return Err(bad(
            "chunk table mode or count does not match the entry row",
        ));
    }
    let mut stored = 0;
    let mut real = 0;
    for chunk in &table.chunks {
        if chunk.stored == 0
            || chunk.inflated == 0
            || (!chunk.zlib && chunk.stored != chunk.inflated)
        {
            return Err(bad("empty or inconsistent chunk sizes"));
        }
        stored += (u64::from(chunk.stored) + 15) & !15;
        real += u64::from(chunk.inflated);
    }
    Ok((stored, real))
}

fn inflate_table(body: &[u8], chunks: &[Chunk]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for chunk in chunks {
        let end = at + chunk.stored as usize;
        let bytes = body
            .get(at..end)
            .ok_or_else(|| bad("chunk extends past its entry"))?;
        let expected = out.len() + chunk.inflated as usize;
        if chunk.zlib {
            let consumed = inflate_zlib(bytes, &mut out, expected)?;
            if consumed != bytes.len() || out.len() != expected {
                return Err(bad("compressed chunk does not match its exact table sizes"));
            }
        } else {
            out.extend_from_slice(bytes);
        }
        at = end.next_multiple_of(16);
    }
    if at != body.len() {
        return Err(bad("chunk sizes do not cover the entry body"));
    }
    Ok(out)
}

fn inflate_zlib(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<usize> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut decoder = Decompress::new(true);
    let mut buffer = [0; 64 * 1024];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &input[before_in as usize..],
                &mut buffer,
                FlushDecompress::Finish,
            )
            .map_err(|e| {
                bad(format!(
                    "invalid zlib chunk: {e}; raw-DEFLATE chunks are not supported"
                ))
            })?;
        let made = (decoder.total_out() - before_out) as usize;
        if made > limit.saturating_sub(out.len()) {
            return Err(bad("chunks exceed the decoded size stated by the table"));
        }
        out.extend_from_slice(&buffer[..made]);
        if status == Status::StreamEnd {
            return Ok(decoder.total_in() as usize);
        }
        if made == 0 && decoder.total_in() == before_in {
            return Err(bad("truncated zlib chunk"));
        }
    }
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

    fn plan(row: &Gta6Entry, chunks: Option<&[Chunk]>) -> Result<Gta6EntryRead> {
        Gta6EntryRead::plan(
            row,
            Gta6EntryExtra {
                chunk_count: chunks.map(|c| c.len() as u32),
                ..Default::default()
            },
            chunks.map(|c| Table {
                mode: 3,
                chunks: c.to_vec(),
            }),
            64,
            128..1024,
            1024,
        )
    }

    fn zlib(bytes: &[u8]) -> (Vec<u8>, Chunk) {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        let mut stored = encoder.finish().unwrap();
        let chunk = Chunk {
            inflated: bytes.len() as u32,
            stored: stored.len() as u32,
            zlib: true,
        };
        stored.resize(stored.len().next_multiple_of(16), 0);
        (stored, chunk)
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
    fn table_zlib_streams_end_at_the_stated_size() {
        let (mut stored, first) = zlib(b"first chunk");
        let (second_body, second) = zlib(b"second chunk");
        stored.extend(second_body);
        let mut src = source(&stored);
        let row = entry(stored.len() as u64, 23, 11);
        assert_eq!(
            plan(&row, Some(&[first, second]))
                .unwrap()
                .read(&mut src, &keys())
                .unwrap(),
            b"first chunksecond chunk"
        );
    }

    #[test]
    fn malformed_compression_never_becomes_raw_bytes_or_a_later_stream() {
        let mut stored = vec![0; 16];
        stored.extend(zlib(b"decoy stream").0);
        let row = entry(stored.len() as u64, 12, 11);
        let chunk = Chunk {
            inflated: 12,
            stored: stored.len() as u32,
            zlib: true,
        };
        assert!(plan(&row, Some(&[chunk]))
            .unwrap()
            .read(&mut source(&stored), &keys())
            .is_err());
        let (mut stored, chunk) = zlib(b"broken checksum");
        stored[3] ^= 0x55;
        assert!(plan(&entry(stored.len() as u64, 15, 11), Some(&[chunk]))
            .unwrap()
            .read(&mut source(&stored), &keys())
            .is_err());
    }

    #[test]
    fn size_count_and_trailing_stream_mismatches_are_errors() {
        let (mut stored, first) = zlib(b"first");
        let (second_body, second) = zlib(b"second");
        stored.extend(second_body);
        for (chunks, real) in [
            (vec![first], 5),
            (vec![first, second], 10),
            (vec![first, second], 12),
            (vec![first, second, second], 11),
        ] {
            assert!(plan(&entry(stored.len() as u64, real, 11), Some(&chunks))
                .and_then(|read| read.read(&mut source(&stored), &keys()))
                .is_err());
        }
        let chunk = Chunk {
            inflated: first.inflated,
            stored: stored.len() as u32,
            zlib: true,
        };
        assert!(plan(
            &entry(stored.len() as u64, u64::from(first.inflated), 11),
            Some(&[chunk])
        )
        .unwrap()
        .read(&mut source(&stored), &keys())
        .is_err());
    }

    #[test]
    fn unsupported_rows_and_out_of_bounds_extents_are_refused() {
        assert!(plan(&entry(16, 16, 14), None).is_err());
        assert!(plan(&entry(16, 16, 11), None).is_err());
        assert!(plan(&entry(16, 16, 11), Some(&[])).is_err());
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
        let (mut body, chunk) = zlib(expected);
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
            plan(&row, Some(&[chunk]))
                .unwrap()
                .read(&mut source(&body), &keys)
                .unwrap(),
            expected
        );
        row.cipher.as_mut().unwrap().cipher_type = 19;
        assert!(plan(&row, Some(&[chunk])).is_err());
        row.cipher.as_mut().unwrap().cipher_type = 17;
        row.cipher.as_mut().unwrap().framing[4] = 5;
        assert!(plan(&row, Some(&[chunk])).is_err());
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
