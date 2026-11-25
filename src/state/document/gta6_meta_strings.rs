//! A GTA VI PSO document's own strings, deciphered with the key snapshot, naming the hash values it
//! carries; without the snapshot the document opens as it reads and says what the names need.

use std::collections::HashMap;
use std::sync::Arc;

use or_io::container::rpf::GtaViKeys;
use or_io::meta::binary::pso::string_names::{self, CategoryString};

/// What a document shows in place of the names its enciphered strings would give.
pub const STRING_NAMES_NEED_KEYS: &str =
    tr_key!("The names of this document's strings need the GTA VI key snapshot");

/// The names a document's string sections give its hash values.
#[derive(Debug, Clone, Default)]
pub struct DocumentStrings {
    names: HashMap<u32, String>,
    wide: HashMap<u64, String>,
    /// Whether the document's strings are enciphered and no key snapshot deciphers them.
    pub needs_keys: bool,
}

impl DocumentStrings {
    /// The names the PSO `bytes` give their own hash values; `keys` is loaded only to decipher them.
    pub fn of(bytes: &[u8], keys: impl FnOnce() -> Option<Arc<GtaViKeys>>) -> Self {
        let keys = if string_names::has_enciphered_strings(bytes) {
            let Some(keys) = keys().filter(|k| k.has_string_keys()) else {
                return Self {
                    needs_keys: true,
                    ..Self::default()
                };
            };
            Some(keys)
        } else {
            None
        };
        match string_names::registered_strings(bytes, keys.as_deref()) {
            Ok(strings) => Self::from_strings(strings),
            Err(e) => {
                tracing::warn!(error = %e, "the document's string sections do not read; its hash values keep no names from them");
                Self::default()
            }
        }
    }

    /// Each string under its category's 32-bit hash and the 64-bit string hash; the first wins.
    fn from_strings(strings: Vec<CategoryString>) -> Self {
        let mut out = Self::default();
        for CategoryString { category, text } in strings {
            let bytes = text.as_bytes();
            let wide = or_io::gta6::hash::name_hash64(bytes);
            out.wide.entry(wide).or_insert_with(|| text.clone());
            if let Some(hash) = string_names::category_hash(category, bytes) {
                out.names.entry(hash).or_insert(text);
            }
        }
        out
    }

    /// Whether any string names a hash value.
    pub fn has_names(&self) -> bool {
        !self.names.is_empty() || !self.wide.is_empty()
    }

    /// The document's own string `hash` names.
    pub fn name(&self, hash: u32) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }

    /// The document's own string the 64-bit string hash `hash` names.
    pub fn name64(&self, hash: u64) -> Option<&str> {
        self.wide.get(&hash).map(String::as_str)
    }
}

/// GTA VI's keys as every app path loads them, where they are found.
pub fn install_keys() -> Option<Arc<GtaViKeys>> {
    match crate::state::rpf9_scan::rpf9_table::keys() {
        Ok(keys) => Some(keys),
        Err(e) => {
            tracing::debug!(error = %e, "no GTA VI keys");
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
        let strings = DocumentStrings::of(&doc, || Some(Arc::new(keys)));
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
        let doc = clear_document(0x01, b"zq_clear_name\0\0\0");
        let strings = DocumentStrings::of(&doc, || -> Option<Arc<GtaViKeys>> {
            panic!("a clear document loads no key snapshot")
        });
        assert!(!strings.needs_keys);
        assert_eq!(
            strings.name(or_io::string_hash(clear_name.as_bytes())),
            Some(clear_name)
        );
    }

    /// A PSO whose `LSNS` holds the one clear run `(id, data)`.
    fn clear_document(id: u32, data: &[u8]) -> Vec<u8> {
        let mut blob = 1u32.to_le_bytes().to_vec();
        blob.extend_from_slice(&id.to_le_bytes());
        blob.extend_from_slice(&(data.len() as u32).to_le_bytes());
        blob.extend_from_slice(data);
        let mut doc = Vec::new();
        frame(&mut doc, b"PSIN", &[0; 8]);
        frame(&mut doc, b"LSNS", &blob);
        doc
    }

    #[test]
    fn each_category_names_by_its_own_hasher_and_a_category_nothing_registers_names_nothing() {
        let text = "Zq_Literal_Name";
        let literal = or_io::literal_string_hash(text.as_bytes());
        let folded = or_io::string_hash(text.as_bytes());
        let strings = DocumentStrings::of(&clear_document(0x1E, b"Zq_Literal_Name\0"), || None);
        assert_eq!(strings.name(literal), Some(text));
        assert_eq!(
            strings.name(folded),
            None,
            "category 0x1E does not fold case"
        );
        let folding = DocumentStrings::of(&clear_document(0x01, b"Zq_Literal_Name\0"), || None);
        assert_eq!(folding.name(folded), Some(text));
        assert_eq!(folding.name(literal), None, "category 1 folds case");
        // Category 0 is not text the game registers, so its bytes name nothing.
        let category_zero = clear_document(0x101, b"Zq_Literal_Name\0");
        assert!(!DocumentStrings::of(&category_zero, || None).has_names());
    }

    #[test]
    fn a_64_bit_string_hash_the_document_carries_shows_its_string() {
        let text = "zq_wide_name";
        let wide = or_io::gta6::hash::name_hash64(text.as_bytes());
        let strings = DocumentStrings::of(&clear_document(0x01, b"zq_wide_name\0"), || None);
        assert_eq!(strings.name64(wide), Some(text));
        assert_eq!(strings.name64(wide ^ 1), None);
        let xml = format!("<Root>\n  <key value=\"{wide}\"/>\n  <n value=\"7\"/>\n</Root>\n");
        let shown = super::super::gta6_meta_names::shown_xml(xml.clone(), None, &strings);
        let named = format!("<key value=\"{wide}\"/><!-- {text} -->\n");
        assert!(shown.contains(&named), "{shown}");
        assert!(shown.contains("<n value=\"7\"/>\n"), "{shown}");
        assert_eq!(
            super::super::gta6_meta_names::without_text_names(&shown),
            xml
        );
    }
}
