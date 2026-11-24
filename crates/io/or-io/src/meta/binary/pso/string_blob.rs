//! GTA VI's enciphered PSO string sections (`LSNS`, `STNS`, `STFN`), deciphered at read.
//!
//! An `LSNS`/`STNS` section body is a blob of `{u32 count; count × (u32 id, u32 len); data}`, header
//! `count * 8 + 4` bytes, the runs concatenated after it. A run's `id` selects its category and whether
//! it is enciphered: `id >= 0x101` with `category = id - 0x101`, and a category of `1` or `0x1e` is
//! enciphered with the PSO string cipher ([`crate::container::rpf::gta6_strings`]) — the whole 16-byte
//! blocks of the run (`len & !0xF`), a trailing `< 16` bytes left clear, the cipher's IV restarted for
//! each run. `STFN` is a single buffer deciphered wholesale as category 1, with no descriptor header.
//!
//! These are layout and extent rules (committable); the key material is game-derived and lives only in
//! the key snapshot [`GtaViKeys`] carries, so a run deciphers only when the snapshot has the string
//! section and is otherwise refused.

use byteorder::{ByteOrder, LE};

use crate::container::rpf::GtaViKeys;
use crate::meta::error::{MetaError, Result};

/// One run of an `LSNS`/`STNS` string blob: its descriptor and where its bytes lie in the blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The descriptor id, which selects the category and whether the run is enciphered.
    pub id: u32,
    /// The run's byte length, as the descriptor states it (the enciphered extent is `len & !0xF`).
    pub len: u32,
    /// The run's data offset within the blob body.
    pub off: usize,
    /// The category (`id - 0x101`) when the id names one, else `None`.
    pub category: Option<u32>,
    /// Whether this run is enciphered (category `1` or `0x1e`).
    pub enciphered: bool,
}

fn blob_bad(why: impl Into<String>) -> MetaError {
    MetaError::Other(format!("GTA VI PSO string blob: {}", why.into()))
}

/// The category a descriptor id names, or `None` when the id is below `0x101` (clear) or its category
/// would not fit a byte. Categories `1` and `0x1e` are the enciphered ones.
pub fn category(id: u32) -> Option<u32> {
    id.checked_sub(0x101).filter(|&c| c < 0x100)
}

/// Whether a run with this descriptor id is enciphered (category `1` or `0x1e`).
pub fn is_enciphered(id: u32) -> bool {
    matches!(category(id), Some(1) | Some(0x1e))
}

/// Parse an `LSNS`/`STNS` blob body's descriptor table into its runs; a count or a run that overruns
/// the body is refused rather than read past the end.
pub fn parse(body: &[u8]) -> Result<Vec<Run>> {
    if body.len() < 4 {
        return Err(blob_bad("shorter than its 4-byte count"));
    }
    let count = LE::read_u32(&body[0..4]) as usize;
    let header = count
        .checked_mul(8)
        .and_then(|h| h.checked_add(4))
        .filter(|&h| h <= body.len())
        .ok_or_else(|| blob_bad("its descriptors run past the body"))?;
    let mut off = header;
    let mut runs = Vec::with_capacity(count);
    for i in 0..count {
        let id = LE::read_u32(&body[4 + i * 8..]);
        let len = LE::read_u32(&body[8 + i * 8..]);
        let end = off
            .checked_add(len as usize)
            .filter(|&e| e <= body.len())
            .ok_or_else(|| blob_bad(format!("run {i} runs past the body")))?;
        runs.push(Run {
            id,
            len,
            off,
            category: category(id),
            enciphered: is_enciphered(id),
        });
        off = end;
    }
    Ok(runs)
}

/// Decipher every enciphered run of an `LSNS`/`STNS` blob in place, leaving clear runs and every run's
/// trailing `< 16` bytes untouched. Refused, with nothing changed, when `keys` carries no string
/// section (`has_string_keys`), so a build without the key cannot be half-deciphered.
pub fn decipher_blob(body: &mut [u8], keys: &GtaViKeys) -> Result<()> {
    let runs = parse(body)?;
    if runs.iter().any(|r| r.enciphered) && !keys.has_string_keys() {
        return Err(blob_bad("the key snapshot carries no string section"));
    }
    for run in runs {
        if !run.enciphered {
            continue;
        }
        let whole = (run.len as usize) & !0xF;
        let slice = &mut body[run.off..run.off + whole];
        keys.string_decrypt(slice)
            .map_err(|e| blob_bad(e.to_string()))?;
    }
    Ok(())
}

/// Decipher an `STFN` section wholesale as a single category-1 run: the whole 16-byte blocks of `data`
/// (`len & !0xF`), the trailing `< 16` bytes left clear. Refused when `keys` has no string section.
pub fn decipher_stfn(data: &mut [u8], keys: &GtaViKeys) -> Result<()> {
    if !keys.has_string_keys() {
        return Err(blob_bad("the key snapshot carries no string section"));
    }
    let whole = data.len() & !0xF;
    keys.string_decrypt(&mut data[..whole])
        .map_err(|e| blob_bad(e.to_string()))
}

/// The NUL-terminated strings a deciphered run holds, in order, dropping the zero padding at its end.
/// Lossy UTF-8, so a byte sequence that is not valid UTF-8 still returns a (replacement-marked) string
/// rather than being skipped, which keeps a malformed decode visible to the caller.
pub fn run_strings(body: &[u8], run: &Run) -> Vec<String> {
    let Some(data) = run
        .off
        .checked_add(run.len as usize)
        .and_then(|end| body.get(run.off..end))
    else {
        return Vec::new(); // a run that does not lie within this body yields nothing, never a panic
    };
    let mut out = Vec::new();
    for part in data.split(|&b| b == 0) {
        if !part.is_empty() {
            out.push(String::from_utf8_lossy(part).into_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_category_and_cipher_gate_match_the_id_ranges() {
        assert_eq!(category(0x100), None, "below 0x101 is clear");
        assert_eq!(category(0x101), Some(0), "id 0x101 is category 0 (clear)");
        assert_eq!(category(0x102), Some(1));
        assert_eq!(category(0x11f), Some(0x1e));
        assert!(!is_enciphered(0x101), "category 0 is not enciphered");
        assert!(is_enciphered(0x102), "category 1 is enciphered");
        assert!(is_enciphered(0x11f), "category 0x1e is enciphered");
        assert!(!is_enciphered(0x103), "category 2 is not enciphered");
    }

    /// `{count=2; (0x101, 16), (0x102, 32); data}` — a clear run then an enciphered one.
    fn blob() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0x101u32.to_le_bytes());
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&0x102u32.to_le_bytes());
        b.extend_from_slice(&32u32.to_le_bytes());
        b.extend(0..48u8); // 16 + 32 data bytes
        b
    }

    #[test]
    fn parse_reads_the_runs_and_flags_the_enciphered_one() {
        let runs = parse(&blob()).expect("the blob parses");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, 0x101);
        assert!(!runs[0].enciphered);
        assert_eq!(runs[0].off, 4 + 2 * 8);
        assert_eq!(runs[1].id, 0x102);
        assert!(runs[1].enciphered);
        assert_eq!(runs[1].off, 4 + 2 * 8 + 16);
    }

    #[test]
    fn a_run_that_overruns_the_body_is_refused() {
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&0x102u32.to_le_bytes());
        b.extend_from_slice(&0x1000u32.to_le_bytes()); // len far past the body
        b.extend_from_slice(&[0u8; 16]);
        assert!(parse(&b).is_err());
    }

    #[test]
    fn run_strings_splits_on_nul_and_drops_padding() {
        // One run holding "ab\0cd\0\0" (padding at the end).
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&0x101u32.to_le_bytes());
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(b"ab\0cd\0\0\0");
        let runs = parse(&b).unwrap();
        assert_eq!(
            run_strings(&b, &runs[0]),
            vec!["ab".to_string(), "cd".to_string()]
        );
        // A run that does not lie within the body yields nothing rather than panicking.
        let stray = Run {
            id: 0x101,
            len: 8,
            off: b.len() + 4,
            category: Some(0),
            enciphered: false,
        };
        assert!(run_strings(&b, &stray).is_empty());
    }
}
