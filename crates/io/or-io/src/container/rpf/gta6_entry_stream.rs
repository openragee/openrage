//! Buffered table-driven entry decoding, with the cipher counter anchored to the whole body.

use super::super::gta6_content::advance_counter_be;
use super::{bad, Decoding, Gta6Cipher, Gta6EntryRead, GtaViKeys, Result};
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};

// Only the current read buffer's intersections are retained, even for a very large periodic body.
fn enciphered_window(
    framing: &[u64; 5],
    len: u64,
    window: std::ops::Range<u64>,
) -> Vec<(u64, u64)> {
    let [head, width, gap, tail, _] = *framing;
    let mut ranges = Vec::new();
    let mut add = |start: u64, end: u64| {
        let start = start.max(window.start);
        let end = end.min(len).min(window.end);
        if start < end {
            ranges.push((start, end));
        }
    };
    add(0, head);
    if width > 0 {
        if gap == 0 {
            add(head, len);
        } else {
            let period = width.saturating_add(gap);
            let first = window.start.saturating_sub(head) / period;
            let mut start = head.saturating_add(first.saturating_mul(period));
            while start < window.end && start < len {
                add(start, start.saturating_add(width));
                start = start.saturating_add(period);
            }
        }
    }
    add(len.saturating_sub(tail), len);
    ranges.sort_unstable();
    let mut kept = 0;
    for index in 0..ranges.len() {
        let (start, end) = ranges[index];
        if kept > 0 && start <= ranges[kept - 1].1 {
            ranges[kept - 1].1 = ranges[kept - 1].1.max(end);
        } else {
            ranges[kept] = (start, end);
            kept += 1;
        }
    }
    ranges.truncate(kept);
    ranges
}

struct Body<'a, R> {
    source: &'a mut R,
    keys: &'a GtaViKeys,
    cipher: Option<Gta6Cipher>,
    len: u64,
    loaded: u64,
    buffer: [u8; 65536],
    begin: usize,
    end: usize,
}
impl<R: Read> BufRead for Body<'_, R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.begin == self.end && self.loaded < self.len {
            let count = (self.len - self.loaded).min(self.buffer.len() as u64) as usize;
            self.source.read_exact(&mut self.buffer[..count])?;
            if let Some(cipher) = self.cipher {
                let end = self.loaded + count as u64;
                for (a, b) in enciphered_window(&cipher.framing, self.len, self.loaded..end) {
                    let counter = advance_counter_be(&cipher.iv, a / 16);
                    self.keys
                        .content_decrypt(
                            cipher.key_id,
                            &counter,
                            &mut self.buffer
                                [(a - self.loaded) as usize..(b - self.loaded) as usize],
                        )
                        .map_err(|e| io::Error::other(e.to_string()))?;
                }
            }
            self.loaded += count as u64;
            self.begin = 0;
            self.end = count;
        }
        Ok(&self.buffer[self.begin..self.end])
    }
    fn consume(&mut self, count: usize) {
        self.begin = (self.begin + count).min(self.end);
    }
}
impl<R: Read> Read for Body<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let bytes = self.fill_buf()?;
        let count = bytes.len().min(out.len());
        out[..count].copy_from_slice(&bytes[..count]);
        self.consume(count);
        Ok(count)
    }
}
fn copy<R: Read, W: Write + ?Sized>(source: &mut R, output: &mut W, len: u64) -> Result<()> {
    let mut remaining = len;
    let mut buffer = [0; 65536];
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        source
            .read_exact(&mut buffer[..count])
            .map_err(|e| bad(e.to_string()))?;
        output
            .write_all(&buffer[..count])
            .map_err(|e| bad(e.to_string()))?;
        remaining -= count as u64;
    }
    Ok(())
}
fn inflate<R: BufRead, W: Write + ?Sized>(
    input: &mut R,
    output: &mut W,
    real: u64,
    zlib: bool,
) -> Result<u64> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut decoder = Decompress::new(zlib);
    let mut buffer = [0; 65536];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                input.fill_buf().map_err(|e| bad(e.to_string()))?,
                &mut buffer,
                FlushDecompress::None,
            )
            .map_err(|e| bad(format!("invalid compressed chunk: {e}")))?;
        input.consume((decoder.total_in() - before_in) as usize);
        let made = (decoder.total_out() - before_out) as usize;
        if decoder.total_out() > real {
            return Err(bad("chunk exceeds its declared decoded size"));
        }
        output
            .write_all(&buffer[..made])
            .map_err(|e| bad(e.to_string()))?;
        if status == Status::StreamEnd {
            if decoder.total_out() != real {
                return Err(bad("chunk does not match its declared decoded size"));
            }
            return Ok(decoder.total_in());
        }
        if decoder.total_in() == before_in && made == 0 {
            return Err(bad("truncated compressed chunk"));
        }
    }
}
pub(super) fn extract<R: Read + Seek, W: Write + ?Sized>(
    plan: &Gta6EntryRead,
    source: &mut R,
    keys: &GtaViKeys,
    output: &mut W,
) -> Result<u64> {
    if let Some(cipher) = plan.cipher {
        keys.content_decrypt(cipher.key_id, &cipher.iv, &mut [])?;
    }
    if source
        .seek(SeekFrom::End(0))
        .map_err(|e| bad(e.to_string()))?
        < plan.extent.end
    {
        return Err(bad("entry extends past the source file"));
    }
    source
        .seek(SeekFrom::Start(plan.extent.start))
        .map_err(|e| bad(e.to_string()))?;
    let len = plan.extent.end - plan.extent.start;
    let mut body = Body {
        source,
        keys,
        cipher: plan.cipher,
        len,
        loaded: 0,
        buffer: [0; 65536],
        begin: 0,
        end: 0,
    };
    match &plan.decoding {
        Decoding::Raw => {
            copy(&mut body, output, plan.real)?;
            copy(&mut body, &mut io::sink(), len - plan.real)?;
        }
        Decoding::Deflate => {
            let consumed = inflate(&mut body, output, plan.real, false)?;
            if consumed.checked_add(15).map(|n| n & !15) != Some(len) {
                return Err(bad("DEFLATE stream does not match the row's stored size"));
            }
            copy(&mut body, &mut io::sink(), len - consumed)?;
        }
        Decoding::Table(chunks) => {
            for chunk in chunks {
                let stored = u64::from(chunk.stored);
                if chunk.zlib {
                    let mut input = (&mut body).take(stored);
                    let consumed = inflate(&mut input, output, u64::from(chunk.inflated), true)?;
                    if consumed != stored {
                        return Err(bad("compressed chunk does not match its exact stored size"));
                    }
                } else {
                    copy(&mut body, output, stored)?;
                }
                copy(&mut body, &mut io::sink(), (16 - stored % 16) % 16)?;
            }
        }
    }
    if body.loaded - (body.end - body.begin) as u64 != len {
        return Err(bad("entry body was not consumed exactly"));
    }
    Ok(plan.real)
}

#[cfg(test)]
mod tests {
    use super::super::super::gta6_content::{enciphered_ranges, KNOWN_FRAMINGS};
    use super::enciphered_window;

    #[test]
    fn buffered_cipher_windows_match_the_whole_body_cover() {
        for framing in KNOWN_FRAMINGS {
            for len in [0, 16, 256, 4096, 8192, 65536, 65552, 131088, 200000] {
                let whole = enciphered_ranges(&framing, len);
                for start in [0, 16, 65520, 65536, 131072, len.saturating_sub(16)] {
                    let start = start.min(len);
                    let end = (start + 65536).min(len);
                    let expected: Vec<_> = whole
                        .iter()
                        .filter_map(|&(a, b)| {
                            let a = a.max(start);
                            let b = b.min(end);
                            (a < b).then_some((a, b))
                        })
                        .collect();
                    assert_eq!(
                        enciphered_window(&framing, len, start..end),
                        expected,
                        "framing {framing:?}, body {len}, window {start}..{end}"
                    );
                }
            }
        }
    }

    #[test]
    fn enormous_cipher_bodies_only_materialise_the_current_buffer() {
        let len = u64::MAX - 15;
        for framing in KNOWN_FRAMINGS {
            let [head, width, gap, tail, _] = framing;
            for start in [0, 1 << 40, len - 65536] {
                let end = start + 65536;
                let ranges = enciphered_window(&framing, len, start..end);
                assert!(ranges.len() <= 10);
                assert!(ranges.iter().all(|&(a, b)| a >= start
                    && b <= end
                    && a < b
                    && a % 16 == 0
                    && b % 16 == 0));
                for at in (start..end).step_by(16) {
                    let expected = at < head
                        || (at >= head
                            && width > 0
                            && (gap == 0 || (at - head) % (width + gap) < width))
                        || at >= len.saturating_sub(tail);
                    let actual = ranges.iter().any(|&(a, b)| at >= a && at < b);
                    assert_eq!(actual, expected, "framing {framing:?}, offset {at}");
                }
            }
        }
    }
}
