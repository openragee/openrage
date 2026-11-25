//! GTA VI's enciphered PSO string sections (`LSNS`, `STNS`, `STFN`), deciphered at read and grown by
//! a save that adds a text.
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

/// The descriptor id of the enciphered category-1 run, the one `StringHash` values register in.
pub const CATEGORY_1_RUN: u32 = 0x102;

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

/// `body` with `text` appended to its enciphered run `id`, the run enciphered again.
// Unchanged when the run already registers a string of the text's hash. Refused for a run the blob
// lacks, one under no string cipher, or one not laid out as its strings, one more NUL and zeros to a
// whole block (the layout every enciphered run of the build has).
pub fn append_string(body: &[u8], id: u32, text: &str, keys: &GtaViKeys) -> Result<Vec<u8>> {
    let runs = parse(body)?;
    let (i, run) = (runs.iter().enumerate())
        .find(|(_, r)| r.id == id)
        .ok_or_else(|| blob_bad(format!("it has no run {id:#X} to add \"{text}\" to")))?;
    let category = run.category.filter(|_| run.enciphered).ok_or_else(|| {
        blob_bad(format!(
            "run {id:#X} is not enciphered text, so no measured rule adds to it"
        ))
    })?;
    if text.is_empty() || text.as_bytes().contains(&0) {
        return Err(blob_bad(
            "a text added to a run is not empty and holds no NUL",
        ));
    }
    let hash = super::string_names::category_hash(category, text.as_bytes())
        .ok_or_else(|| blob_bad(format!("run {id:#X} registers no hash of \"{text}\"")))?;
    let len = run.len as usize;
    let mut plain = body[run.off..run.off + len].to_vec();
    keys.string_decrypt(&mut plain)
        .map_err(|e| blob_bad(e.to_string()))?;
    // A run is its strings, each with its NUL, then one more NUL and zeros to a whole block.
    let used = plain
        .iter()
        .rposition(|&b| b != 0)
        .map_or(0, |last| last + 2);
    if len != (used + 1).next_multiple_of(16) {
        return Err(blob_bad(format!(
            "run {id:#X} deciphers to {len} bytes, not its strings and one more NUL padded to 16"
        )));
    }
    let strings = plain[..used].split(|&b| b == 0).filter(|s| !s.is_empty());
    if strings
        .map(|s| super::string_names::category_hash(category, s))
        .any(|h| h == Some(hash))
    {
        return Ok(body.to_vec());
    }
    plain.truncate(used);
    plain.extend_from_slice(text.as_bytes());
    plain.push(0);
    plain.resize((plain.len() + 1).next_multiple_of(16), 0);
    let grown = u32::try_from(plain.len())
        .map_err(|_| blob_bad(format!("run {id:#X} outgrows the 4 GiB a length states")))?;
    keys.string_encrypt(&mut plain)
        .map_err(|e| blob_bad(e.to_string()))?;
    let mut out = [&body[..run.off], &plain[..], &body[run.off + len..]].concat();
    LE::write_u32(&mut out[8 + i * 8..], grown);
    Ok(out)
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

    /// A blob of a clear run then the category-1 run holding `plain`, enciphered with `keys`.
    fn enciphered_blob(plain: &[u8], keys: &GtaViKeys) -> Vec<u8> {
        let mut run = plain.to_vec();
        keys.string_encrypt(&mut run)
            .expect("the string key enciphers");
        let mut b = Vec::new();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0x101u32.to_le_bytes());
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&CATEGORY_1_RUN.to_le_bytes());
        b.extend_from_slice(&(run.len() as u32).to_le_bytes());
        b.extend_from_slice(b"zq_clear_run\0\0\0\0");
        b.extend_from_slice(&run);
        b
    }

    fn deciphered_strings(body: &[u8], keys: &GtaViKeys) -> Vec<Vec<String>> {
        let mut plain = body.to_vec();
        decipher_blob(&mut plain, keys).expect("it deciphers");
        let runs = parse(&plain).expect("it parses");
        runs.iter().map(|r| run_strings(&plain, r)).collect()
    }

    #[test]
    fn an_appended_string_reads_back_after_the_last_and_pads_by_the_rule() {
        let keys = crate::container::rpf::gta6_strings::tests::clean_keys();
        let body = enciphered_blob(b"zq_one\0zq_two\0\0\0", &keys);
        let grown = append_string(&body, CATEGORY_1_RUN, "zq_three", &keys).expect("it appends");
        let runs = parse(&grown).expect("the grown blob parses");
        // 14 bytes of strings, 9 of the new one and its NUL, one more NUL: 32 bytes.
        assert_eq!(runs[1].len, 32);
        assert_eq!(
            grown.len(),
            body.len() + 16,
            "the blob grows by the run alone"
        );
        assert_eq!(
            deciphered_strings(&grown, &keys),
            [
                vec!["zq_clear_run".to_string()],
                vec!["zq_one".into(), "zq_two".into(), "zq_three".into()]
            ]
        );
        assert_eq!(
            grown[..16],
            body[..16],
            "the count and the clear run's descriptor"
        );
        assert_eq!(grown[20..36], body[20..36], "the clear run is untouched");
        // A text the run registers already, by its category's folded hash, leaves the blob as it was.
        assert_eq!(
            append_string(&grown, CATEGORY_1_RUN, "ZQ_Two", &keys).unwrap(),
            grown
        );
    }

    #[test]
    fn a_run_the_rules_do_not_cover_is_refused() {
        let keys = crate::container::rpf::gta6_strings::tests::clean_keys();
        let body = enciphered_blob(b"zq_one\0\0\0\0\0\0\0\0\0\0", &keys);
        assert!(
            append_string(&body, 0x101, "zq_new", &keys).is_err(),
            "a clear run"
        );
        assert!(
            append_string(&body, 0x11F, "zq_new", &keys).is_err(),
            "a run the blob lacks"
        );
        assert!(
            append_string(&body, CATEGORY_1_RUN, "", &keys).is_err(),
            "an empty text"
        );
        assert!(
            append_string(&body, CATEGORY_1_RUN, "zq\0x", &keys).is_err(),
            "a NUL in the text"
        );
        // A run whose strings fill it with no extra NUL is not laid out by the rule.
        let full = enciphered_blob(b"zq_sixteen_byte\0", &keys);
        assert!(append_string(&full, CATEGORY_1_RUN, "zq_new", &keys).is_err());
        // A run padded past one block is not either.
        let long = enciphered_blob(&[b"zq_one\0".as_slice(), &[0; 25]].concat(), &keys);
        assert!(append_string(&long, CATEGORY_1_RUN, "zq_new", &keys).is_err());
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
