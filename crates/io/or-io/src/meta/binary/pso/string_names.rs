//! The strings a GTA VI PSO document's string sections hold, enciphered runs deciphered with the key
//! snapshot, so the hash values the document carries can be shown by the text they hash.
//!
//! `STDN` carries the `STNS` blob again byte for byte wherever it was measured, so it is read by the
//! same descriptor layout and the same per-run id gate; the executable's loader leaves it as it is.

use super::sections::{split_sections, PsoSection, RawPsoSection};
use super::string_blob::{self, Run};
use crate::container::rpf::GtaViKeys;
use crate::meta::error::{MetaError, Result};

/// The sections laid out as a descriptor blob, `{u32 count; count × (u32 id, u32 len); data}`.
const BLOBS: [&[u8; 4]; 3] = [b"LSNS", b"STNS", b"STDN"];
/// The section that is one buffer, enciphered wholesale as category 1.
const STFN: &[u8; 4] = b"STFN";

fn is(section: &RawPsoSection, tag: &[u8; 4]) -> bool {
    section.ident == PsoSection::from_ident(u32::from_be_bytes(*tag))
}

/// A section's body, after its 8-byte frame (which `split_sections` guarantees).
fn body(section: &RawPsoSection) -> &[u8] {
    &section.bytes[8..]
}

/// `STFN`'s whole buffer as one category-1 run.
fn stfn_run(body: &[u8]) -> Run {
    const ID: u32 = 0x102;
    Run {
        id: ID,
        len: body.len() as u32,
        off: 0,
        category: string_blob::category(ID),
        enciphered: true,
    }
}

/// Whether the PSO `bytes` hold a string run whose whole blocks are enciphered, so its text needs the
/// key snapshot; `false` for bytes that are not PSO sections.
pub fn has_enciphered_strings(bytes: &[u8]) -> bool {
    let Ok(sections) = split_sections(bytes) else {
        return false;
    };
    sections.iter().any(|s| {
        if is(s, STFN) {
            return body(s).len() >= 16;
        }
        BLOBS.iter().any(|tag| is(s, tag)) && enciphered_blob(body(s))
    })
}

/// Whether a blob body parses and holds a run with a whole enciphered block.
fn enciphered_blob(body: &[u8]) -> bool {
    let runs = string_blob::parse(body).unwrap_or_default();
    runs.iter().any(|r| r.enciphered && r.len >= 16)
}

/// The strings of the PSO `bytes`' `LSNS`, `STNS`, `STDN` and `STFN` sections in order, each
/// enciphered run deciphered with `keys`; a malformed blob, or an enciphered run with no `keys` or
/// that `keys` cannot decipher, is refused.
pub fn document_strings(bytes: &[u8], keys: Option<&GtaViKeys>) -> Result<Vec<String>> {
    let needs_keys = |what: &str| {
        MetaError::Other(format!(
            "GTA VI PSO string blob: {what} is enciphered and no key snapshot was given"
        ))
    };
    let mut out = Vec::new();
    for section in split_sections(bytes)? {
        let mut data = body(&section).to_vec();
        if is(&section, STFN) {
            match keys {
                Some(keys) => string_blob::decipher_stfn(&mut data, keys)?,
                None if data.len() >= 16 => return Err(needs_keys("STFN")),
                None => {}
            }
            out.extend(string_blob::run_strings(&data, &stfn_run(&data)));
            continue;
        }
        if !BLOBS.iter().any(|tag| is(&section, tag)) {
            continue;
        }
        match keys {
            Some(keys) => string_blob::decipher_blob(&mut data, keys)?,
            None if enciphered_blob(&data) => return Err(needs_keys("a run")),
            None => {}
        }
        for run in string_blob::parse(&data)? {
            out.extend(string_blob::run_strings(&data, &run));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
        out.extend_from_slice(ident);
        out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(content);
    }

    /// A blob of one run `(id, data)`.
    fn blob(id: u32, data: &[u8]) -> Vec<u8> {
        let mut b = 1u32.to_le_bytes().to_vec();
        b.extend_from_slice(&id.to_le_bytes());
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    /// An `ORGTAVIK` snapshot with no index or content keys and a string section of synthetic
    /// tables (never game-derived) and the fixed IV `iv`, laid out as the snapshot format states.
    fn snapshot(iv: [u8; 16]) -> GtaViKeys {
        let mut out = b"ORGTAVIK".to_vec();
        out.extend_from_slice(&1u32.to_le_bytes()); // version
        out.extend_from_slice(&0x4342u32.to_le_bytes()); // CBC mode tag
        out.extend_from_slice(&[0u8; 32 + 16]); // exe sha256, bootstrap iv
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend((0..4096).map(|i| i as u8));
        out.extend_from_slice(&16384u32.to_le_bytes());
        out.extend((0..16384).map(|i| i as u8));
        out.extend_from_slice(&0u32.to_le_bytes()); // no index keys
        out.extend_from_slice(&0x5243u32.to_le_bytes()); // CTR mode tag
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend((0..4096).map(|i| (i * 3 + 1) as u8));
        out.extend_from_slice(&16384u32.to_le_bytes());
        out.extend((0..16384).map(|i| (i * 5 + 7) as u8));
        out.extend_from_slice(&0u32.to_le_bytes()); // no content keys
        out.extend_from_slice(&0x5453u32.to_le_bytes()); // string mode tag
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend((0..4096).map(|i| (i * 3 + 7) as u8));
        out.extend_from_slice(&16384u32.to_le_bytes());
        out.extend((0..16384).map(|i| (i * 9) as u8));
        out.extend_from_slice(&iv);
        out.extend_from_slice(&4340u32.to_le_bytes());
        out.extend_from_slice(&[0x50, 0x4f, 0x86, 0x5e]); // string blob magic
        out.extend((0..60u32).flat_map(|w| w.wrapping_mul(13).to_le_bytes()));
        out.extend((0..4096).map(|i| (i * 2 + 1) as u8));
        GtaViKeys::parse(&out).expect("the synthetic snapshot parses")
    }

    /// One cipher block, and keys under which it deciphers to `plain`: CBC's first block is the
    /// block transform XOR the IV, so the IV is chosen from what the transform gives under a zero IV.
    fn enciphered(plain: &[u8; 16]) -> ([u8; 16], GtaViKeys) {
        let cipher = [0x5Au8; 16];
        let mut transformed = cipher;
        snapshot([0; 16])
            .string_decrypt(&mut transformed)
            .expect("the string key deciphers");
        let iv: [u8; 16] = std::array::from_fn(|i| transformed[i] ^ plain[i]);
        (cipher, snapshot(iv))
    }

    const PLAIN: &[u8; 16] = b"zq_lsns_name\0\0\0\0";

    #[test]
    fn an_enciphered_run_reads_as_the_string_it_deciphers_to() {
        let (cipher, keys) = enciphered(PLAIN);
        let mut doc = Vec::new();
        frame(&mut doc, b"PSIN", &[0; 8]);
        frame(&mut doc, b"LSNS", &blob(0x102, &cipher));
        assert!(has_enciphered_strings(&doc));
        assert_eq!(
            document_strings(&doc, Some(&keys)).expect("it deciphers"),
            vec!["zq_lsns_name".to_string()]
        );
        // The raw run is not the string: without the decipher it would not read so.
        assert_ne!(&cipher, PLAIN);
    }

    #[test]
    fn stfn_deciphers_wholesale_and_stdn_reads_by_the_blob_layout() {
        let (cipher, keys) = enciphered(PLAIN);
        let mut doc = Vec::new();
        frame(&mut doc, b"STFN", &cipher);
        frame(&mut doc, b"STDN", &blob(0x101, b"zq_stdn_clear\0\0\0"));
        assert_eq!(
            document_strings(&doc, Some(&keys)).expect("it reads"),
            vec!["zq_lsns_name".to_string(), "zq_stdn_clear".to_string()]
        );
    }

    #[test]
    fn a_clear_document_needs_no_keys_and_a_malformed_blob_is_refused() {
        let mut clear = Vec::new();
        frame(&mut clear, b"LSNS", &blob(0x101, b"zq_clear_name\0\0\0"));
        assert!(!has_enciphered_strings(&clear));
        assert!(!has_enciphered_strings(b"not a pso"));
        assert_eq!(
            document_strings(&clear, None).expect("a clear blob reads without keys"),
            vec!["zq_clear_name".to_string()]
        );
        let (cipher, _) = enciphered(PLAIN);
        let mut enciphered_doc = Vec::new();
        frame(&mut enciphered_doc, b"LSNS", &blob(0x102, &cipher));
        assert!(document_strings(&enciphered_doc, None).is_err());

        let mut bad = Vec::new();
        frame(&mut bad, b"LSNS", &[9, 0, 0, 0]); // nine descriptors in a 4-byte body
        assert!(document_strings(&bad, Some(&snapshot([0; 16]))).is_err());
    }
}
