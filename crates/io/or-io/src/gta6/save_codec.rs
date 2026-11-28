//! The story save's payload codec: the LZ4 block stream the ASAV payload holds between its
//! PKCS#7-style pad and its 32-byte MAC, measured on the Xbox build and on real saves.
//!
//! A story save's payload — after the [`SaveCipher`](crate::gta6::save_crypto::SaveCipher)
//! XOR — is `[LZ4 stream][pad][MAC]`: the LZ4 block format (tokens of a literal count and a
//! match length, little-endian 16-bit match offsets, `0xFF` length extensions), a pad of the
//! pad's own length 1..=16 bytes, and the 32-byte tag the security singleton verifies. The
//! decompressed stream is the `JSAV` container [`Jsav`](crate::gta6::save::Jsav) reads.
//!
//! The codec was measured two ways: the game's own decompressor at `0x142220A50` in
//! `game_scarlett_master.exe`, driven on the decrypted payload of a real save, returns the same
//! bytes this module does, byte for byte; and a stream this module compresses, written back
//! through the whole payload pipeline, decodes to the same `JSAV` through that same game code.
//! The game's own compressor at `0x142220330` declines inputs of 64KiB and more, so the real
//! saves' streams were made by a sibling encoder; any valid LZ4 block stream loads, because the
//! decompressor is the standard one.

use crate::gta6::save::Jsav;

/// Why a payload is not decoded, or not encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error("the LZ4 stream is malformed: {why} at {at}")]
    Malformed { why: &'static str, at: usize },
    #[error("the LZ4 stream decodes to {decoded} bytes, which is past the {cap} byte limit")]
    TooLarge { decoded: usize, cap: usize },
    #[error("the pad at the payload's end is {found}, which is not 1..=16, or its bytes are not the pad")]
    BadPad { found: u8 },
    #[error("the payload's body ends at {len} bytes, before a pad and a 32-byte MAC could end there")]
    TooShort { len: usize },
    #[error("the decompressed stream is not a JSAV: {0}")]
    NotJsav(#[from] crate::gta6::save::SaveError),
}

/// The 32-byte MAC the security singleton verifies at the payload's end.
pub const MAC: usize = 32;

/// The size a decompressed story save is refused past: the game's container buffers are sized
/// in megabytes, and every real save decodes to under one and a half.
pub const DECOMPRESS_CAP: usize = 1 << 24;

/// Decode the LZ4 block format as the game's `LZ4_decompress_safe` at `0x142220A50` does, with
/// the one deviation that build allows: a match offset of zero fills the match with the zeroed
/// bytes the destination was cleared to before the decode.
pub fn lz4_decompress(src: &[u8], cap: usize) -> Result<Vec<u8>, CodecError> {
    let malformed = |why: &'static str, at: usize| CodecError::Malformed { why, at };
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let token = src[i];
        i += 1;
        let mut literals = token as usize >> 4;
        if literals >= 15 {
            loop {
                let more = *src.get(i).ok_or(malformed("a literal length runs past the stream", i))?;
                i += 1;
                literals += more as usize;
                if more != 0xFF {
                    break;
                }
            }
        }
        let end = i
            .checked_add(literals)
            .filter(|end| *end <= src.len())
            .ok_or(malformed("the literals run past the stream", i))?;
        if out.len() + literals > cap {
            return Err(CodecError::TooLarge { decoded: out.len() + literals, cap });
        }
        out.extend_from_slice(&src[i..end]);
        i = end;
        if i == src.len() {
            return Ok(out); // the last sequence: literals alone, no match
        }
        let offset = u16::from_le_bytes(std::array::from_fn(|k| {
            *src.get(i + k).ok_or(malformed("a match offset runs past the stream", i + k)).unwrap()
        })) as usize;
        i += 2;
        let mut matchlen = (token & 0xF) as usize + 4;
        if token & 0xF == 15 {
            loop {
                let more = *src.get(i).ok_or(malformed("a match length runs past the stream", i))?;
                i += 1;
                matchlen += more as usize;
                if more != 0xFF {
                    break;
                }
            }
        }
        if offset > out.len() {
            return Err(malformed("a match offset reaches before the output's start", i - 2));
        }
        let out_end = out.len() + matchlen;
        if out_end > cap {
            return Err(CodecError::TooLarge { decoded: out_end, cap });
        }
        if offset == 0 {
            out.resize(out_end, 0);
        } else {
            let start = out_end - matchlen;
            out.resize(out_end, 0);
            for k in 0..matchlen {
                let byte = out[start + k - offset];
                out[start + k] = byte;
            }
        }
    }
    Ok(out)
}

/// The LZ4 block format's shortest match, and the longest offset a 16-bit field can state.
const MIN_MATCH: usize = 4;
const MAX_OFFSET: usize = 0xFFFF;

/// Compress `bytes` to the LZ4 block format the game's decompressor reads: a greedy hash match
/// over a 64KiB window, the shape the reference encoder produces. The output is a valid stream
/// for the game, though not the byte-for-byte stream the game's own encoder would have made;
/// any valid stream decodes.
pub fn lz4_compress(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() / 2 + 16);
    // The hash table holds the position of the last 4 bytes that hashed to each slot.
    let mut table = vec![usize::MAX; 1 << 12];
    let hash = |at: usize| {
        let word = u32::from_le_bytes(std::array::from_fn(|k| {
            bytes.get(at + k).copied().unwrap_or(0)
        }));
        (word.wrapping_mul(2654435761) >> 20) as usize & (1 << 12) - 1
    };
    let mut literals_start = 0usize;
    let mut at = 0usize;
    while at + MIN_MATCH <= bytes.len() {
        let h = hash(at);
        let candidate = table[h];
        table[h] = at;
        let matches = candidate != usize::MAX
            && at - candidate <= MAX_OFFSET
            && at + MIN_MATCH <= bytes.len()
            && bytes[candidate..candidate + MIN_MATCH] == bytes[at..at + MIN_MATCH];
        if !matches {
            at += 1;
            continue;
        }
        // The pending literals, then this match.
        let literals = at - literals_start;
        let token_at = out.len();
        out.push(((literals.min(15)) as u8) << 4);
        let mut rest = literals.saturating_sub(15);
        while rest > 0 {
            let take = rest.min(255);
            out.push(take as u8);
            rest -= take;
            if take == 255 && rest == 0 {
                // an extension byte of 255 must be followed by another, even a zero
                out.push(0);
            }
        }
        out.extend_from_slice(&bytes[literals_start..at]);
        let mut matchlen = MIN_MATCH;
        while at + matchlen < bytes.len()
            && bytes[candidate + matchlen] == bytes[at + matchlen]
            && matchlen < 15 + 255 * 3
        {
            matchlen += 1;
        }
        out.extend_from_slice(&((at - candidate) as u16).to_le_bytes());
        let coded = matchlen - MIN_MATCH;
        out[token_at] |= coded.min(15) as u8;
        let mut rest = coded.saturating_sub(15);
        while rest > 0 {
            let take = rest.min(255);
            out.push(take as u8);
            rest -= take;
        }
        at += matchlen;
        literals_start = at;
    }
    // The last literals, with no match after them.
    let literals = bytes.len() - literals_start;
    if literals > 0 {
        let token_at = out.len();
        out.push(((literals.min(15)) as u8) << 4);
        let mut rest = literals.saturating_sub(15);
        while rest > 0 {
            let take = rest.min(255);
            out.push(take as u8);
            rest -= take;
        }
        out.extend_from_slice(&bytes[literals_start..]);
    }
    out
}

/// The payload of a story save as the cipher leaves it: `body` is the payload without its
/// 32-byte MAC, holding `[LZ4 stream][pad]`. Returns the decompressed `JSAV`.
pub fn decode(body: &[u8]) -> Result<Jsav, CodecError> {
    let pad = match body.len().checked_sub(1).filter(|at| *at >= MAC) {
        Some(at) => body[at],
        None => return Err(CodecError::TooShort { len: body.len() + MAC }),
    };
    if !(1..=16).contains(&pad) || body[body.len() - pad as usize..] != vec![pad; pad as usize] {
        return Err(CodecError::BadPad { found: pad });
    }
    let stream = &body[..body.len() - pad as usize];
    let jsav = lz4_decompress(stream, DECOMPRESS_CAP)?;
    Jsav::open(&jsav).map_err(CodecError::NotJsav)
}

/// The payload as the cipher takes it back: `jsav` compressed, padded to the cipher's 16-byte
/// block with the pad's own length, and `mac` — the 32 bytes the save carried, or zeros — after.
pub fn encode(jsav: &[u8], mac: [u8; MAC]) -> Vec<u8> {
    let stream = lz4_compress(jsav);
    let pad = 16 - stream.len() % 16;
    let mut payload = stream;
    payload.extend(std::iter::repeat(pad as u8).take(pad));
    payload.extend_from_slice(&mac);
    payload
}

#[cfg(test)]
#[path = "save_codec_tests.rs"]
mod tests;
