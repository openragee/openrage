//! A GTA VI PSO document's own strings, deciphered with the key snapshot, naming the hash values it
//! carries; without the snapshot the document opens as it reads and says what the names need.

use std::collections::HashMap;

use or_io::container::rpf::GtaViKeys;
use or_io::meta::binary::pso::string_names;

/// What a document shows in place of the names its enciphered strings would give.
pub const STRING_NAMES_NEED_KEYS: &str =
    tr_key!("The names of this document's strings need the GTA VI key snapshot");

/// The names a document's string sections give its hash values.
#[derive(Debug, Clone, Default)]
pub struct DocumentStrings {
    names: HashMap<u32, String>,
    /// Whether the document's strings are enciphered and no key snapshot deciphers them.
    pub needs_keys: bool,
}

impl DocumentStrings {
    /// The names the PSO `bytes` give their own hash values; `keys` is loaded only to decipher them.
    pub fn of(bytes: &[u8], keys: impl FnOnce() -> Option<GtaViKeys>) -> Self {
        let keys = if string_names::has_enciphered_strings(bytes) {
            let Some(keys) = keys().filter(GtaViKeys::has_string_keys) else {
                return Self {
                    needs_keys: true,
                    ..Self::default()
                };
            };
            Some(keys)
        } else {
            None
        };
        match string_names::document_strings(bytes, keys.as_ref()) {
            Ok(strings) => Self::from_strings(strings),
            Err(e) => {
                tracing::warn!(error = %e, "the document's string sections do not read; its hash values keep no names from them");
                Self::default()
            }
        }
    }

    fn from_strings(strings: Vec<String>) -> Self {
        let names = (strings.into_iter())
            .map(|text| (or_io::string_hash(text.as_bytes()), text))
            .collect();
        Self {
            names,
            needs_keys: false,
        }
    }

    /// Whether any string names a hash value.
    pub fn has_names(&self) -> bool {
        !self.names.is_empty()
    }

    /// The document's own string `hash` names.
    pub fn name(&self, hash: u32) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }
}

/// The user's GTA VI key snapshot, where one is found and reads.
pub fn install_keys() -> Option<GtaViKeys> {
    match GtaViKeys::for_install(or_title::config::config_dir().as_deref()) {
        Ok(keys) => keys,
        Err(e) => {
            tracing::warn!(error = %e, "the GTA VI key snapshot does not read");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
        out.extend_from_slice(ident);
        out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(content);
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

    const NAME: &str = "zq_shown_name";

    /// A PSO whose `LSNS` holds one enciphered block, and keys under which it deciphers to [`NAME`]:
    /// CBC's first block is the block transform XOR the IV, so the IV is chosen from the transform.
    fn enciphered_document() -> (Vec<u8>, GtaViKeys) {
        let cipher = [0x5Au8; 16];
        let mut transformed = cipher;
        snapshot([0; 16])
            .string_decrypt(&mut transformed)
            .expect("the string key deciphers");
        let mut plain = [0u8; 16];
        plain[..NAME.len()].copy_from_slice(NAME.as_bytes());
        let iv: [u8; 16] = std::array::from_fn(|i| transformed[i] ^ plain[i]);
        let mut blob = 1u32.to_le_bytes().to_vec();
        blob.extend_from_slice(&0x102u32.to_le_bytes());
        blob.extend_from_slice(&16u32.to_le_bytes());
        blob.extend_from_slice(&cipher);
        let mut doc = Vec::new();
        frame(&mut doc, b"PSIN", &[0; 8]);
        frame(&mut doc, b"LSNS", &blob);
        (doc, snapshot(iv))
    }

    fn xml(hash: u32) -> String {
        format!("<Root>\n  <a>hash_{hash:08X}</a>\n</Root>\n")
    }

    #[test]
    fn with_the_key_snapshot_a_hash_the_document_carries_shows_its_deciphered_string() {
        let (doc, keys) = enciphered_document();
        let strings = DocumentStrings::of(&doc, || Some(keys));
        let hash = or_io::string_hash(NAME.as_bytes());
        assert_eq!(strings.name(hash), Some(NAME));
        assert!(!strings.needs_keys);
        let shown = super::super::gta6_meta_names::shown_xml(xml(hash), None, &strings);
        assert!(shown.contains(&format!("<!-- {NAME} -->")), "{shown}");
    }

    #[test]
    fn without_the_key_snapshot_the_document_shows_as_it_reads_and_says_what_names_need() {
        let (doc, _) = enciphered_document();
        let strings = DocumentStrings::of(&doc, || None);
        assert!(strings.needs_keys);
        assert!(!strings.has_names());
        let hash = or_io::string_hash(NAME.as_bytes());
        let shown = super::super::gta6_meta_names::shown_xml(xml(hash), None, &strings);
        assert_eq!(shown, xml(hash));
        // A document with nothing enciphered asks for no keys.
        assert!(!DocumentStrings::of(b"<Root/>", || None).needs_keys);
    }

    #[test]
    fn a_clear_document_names_its_hashes_without_loading_the_key_snapshot() {
        let clear_name = "zq_clear_name";
        let mut blob = 1u32.to_le_bytes().to_vec();
        blob.extend_from_slice(&0x101u32.to_le_bytes());
        blob.extend_from_slice(&16u32.to_le_bytes());
        blob.extend_from_slice(b"zq_clear_name\0\0\0");
        let mut doc = Vec::new();
        frame(&mut doc, b"PSIN", &[0; 8]);
        frame(&mut doc, b"LSNS", &blob);
        let strings = DocumentStrings::of(&doc, || -> Option<GtaViKeys> {
            panic!("a clear document loads no key snapshot")
        });
        assert!(!strings.needs_keys);
        assert_eq!(
            strings.name(or_io::string_hash(clear_name.as_bytes())),
            Some(clear_name)
        );
    }
}
