use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

use crate::meta::schema::{consistent_struct_short_name, SchemaLibrary};

#[derive(Debug, Default)]
pub struct HashIndex {
    map: HashMap<u32, String>,
}

impl HashIndex {
    pub fn from_library(library: &SchemaLibrary) -> Self {
        let mut index = Self::default();
        index.add_library(library);
        index
    }

    /// Add the names of `library` for the hashes nothing held names yet.
    pub fn add_library(&mut self, library: &SchemaLibrary) {
        let mut names: Vec<(&str, bool)> =
            Vec::with_capacity(library.structs.len() * 8 + library.enums.len() * 4);
        for s in library.structs.values() {
            names.push((s.name.as_str(), true));
            for f in &s.fields {
                names.push((f.name.as_str(), false));
            }
        }
        for e in library.enums.values() {
            names.push((e.name.as_str(), true));
            for v in &e.values {
                names.push((v.name.as_str(), false));
            }
        }
        names.sort_unstable();
        names.dedup();

        self.map.reserve(names.len());
        // A name a library spells as a hash is no name.
        for (name, is_type) in names.into_iter().filter(|(n, _)| spelled_hash(n).is_none()) {
            if is_type {
                insert_type(&mut self.map, name);
            } else {
                insert(&mut self.map, name);
            }
        }
    }

    /// Add the identifiers public sources name, for the hashes a library
    /// left unresolved.
    ///
    /// ⚠ Applied after the library so a staged snapshot's answer survives:
    /// a title's own schema outranks what we know without one, and [`insert`]
    /// is first-wins.
    pub fn with_public_names(mut self) -> Self {
        for name in crate::meta::binary::public_names::all() {
            insert(&mut self.map, name);
        }
        self
    }

    /// File each `(hash, name)` pair directly, by the hash as given rather than
    /// by hashing the name. First-wins, so an answer already in the index — a
    /// title's own schema, the public names — is never overwritten.
    pub fn with_pairs(mut self, pairs: impl IntoIterator<Item = (u32, String)>) -> Self {
        for (hash, name) in pairs {
            self.map.entry(hash).or_insert(name);
        }
        self
    }

    /// Add GTA VI's recovered `rage::par` names, for the hashes no schema or
    /// public name resolved. Inert when the host made no corpus available
    /// ([`crate::meta::binary::ragepar_names`]), so a clean checkout is unchanged.
    ///
    /// ⚠ Applied last, after the library and the public names, so its names
    /// never displace a title's own.
    pub fn with_parser_names(self) -> Self {
        match crate::meta::binary::ragepar_names::installed() {
            Some(names) => self.with_pairs(names.pairs().map(|(h, n)| (h, n.to_string()))),
            None => self,
        }
    }

    pub fn lookup(&self, hash: u32) -> Option<&str> {
        self.map.get(&hash).map(String::as_str)
    }

    /// The name `hash` has, or `hash_XXXXXXXX` when nothing names it.
    pub fn name_or_literal(&self, hash: u32) -> String {
        match self.lookup(hash) {
            Some(s) => s.to_string(),
            None => format!("hash_{hash:08X}"),
        }
    }

    /// Every name the index holds, once per hash it is filed under.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.values().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// The hash `text` spells: `hash_` and exactly eight hex digits, or the `0x`
/// form earlier builds wrote.
pub fn spelled_hash(text: &str) -> Option<u32> {
    let hex = text
        .strip_prefix("hash_")
        .or_else(|| text.strip_prefix("0x"))
        .or_else(|| text.strip_prefix("0X"))?;
    if hex.len() != 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

fn insert(map: &mut HashMap<u32, String>, name: &str) {
    let literal = crate::literal_string_hash(name.as_bytes());
    map.entry(literal).or_insert_with(|| name.to_string());
}

fn insert_type(map: &mut HashMap<u32, String>, friendly: &str) {
    insert(map, friendly);
    let underscored = consistent_struct_short_name(friendly);
    if underscored != friendly {
        let literal = crate::literal_string_hash(underscored.as_bytes());
        map.entry(literal).or_insert_with(|| friendly.to_string());
    }
}

std::thread_local! {
    static PUBLIC_ONLY: Cell<bool> = const { Cell::new(false) };
}

std::thread_local! {
    static TITLE_NAMES: Cell<bool> = const { Cell::new(false) };
}

/// While held, a hash read on this thread is named by the second title's library first.
pub struct TitleNames(bool);

impl Drop for TitleNames {
    fn drop(&mut self) {
        TITLE_NAMES.with(|on| on.set(self.0));
    }
}

/// Names the hashes of the second title's bodies, read and written on this thread until the guard drops.
///
/// ⚠ Two titles' hash spaces are unrelated: outside a guard no name of the second title's library is
/// given, so a document of the first title prints as it does with no such library installed.
pub fn title_names() -> TitleNames {
    TitleNames(TITLE_NAMES.with(|on| on.replace(true)))
}

/// Runs `f` naming every hash on this thread from the public table alone, whatever library the host installed, so what `f` reads and writes is what a clean checkout makes.
pub fn with_public_names_only<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            PUBLIC_ONLY.with(|only| only.set(self.0));
        }
    }
    let _restore = Restore(PUBLIC_ONLY.with(|only| only.replace(true)));
    f()
}

/// The names the default library and [`crate::meta::binary::public_names`] give.
///
/// ⚠ Keyed on whether a library is installed: an index built before the host
/// installed one would otherwise name nothing outside the public names for
/// the rest of the process.
pub fn default_hash_index() -> &'static HashIndex {
    static INSTALLED_INDEX: OnceLock<HashIndex> = OnceLock::new();
    static PUBLIC_INDEX: OnceLock<HashIndex> = OnceLock::new();
    static CLEAN_INDEX: OnceLock<HashIndex> = OnceLock::new();
    static TITLE_INDEX: OnceLock<HashIndex> = OnceLock::new();
    // ⚠ The override is the clean-checkout simulation, which has no recovered
    // name corpus either, so it excludes the parser names a dev machine's
    // `OPENRAGE_PARSER_NAMES` would otherwise fold in.
    if PUBLIC_ONLY.with(Cell::get) {
        return CLEAN_INDEX.get_or_init(|| {
            HashIndex::from_library(&SchemaLibrary::default()).with_public_names()
        });
    }
    let installed = crate::meta::snapshot::installed();
    let title = crate::meta::snapshot::names_library().filter(|_| TITLE_NAMES.with(Cell::get));
    if let Some(title) = title {
        return TITLE_INDEX.get_or_init(|| {
            let mut index = HashIndex::from_library(title);
            installed.into_iter().for_each(|l| index.add_library(l));
            index.with_public_names().with_parser_names()
        });
    }
    match installed {
        Some(library) => INSTALLED_INDEX.get_or_init(|| {
            HashIndex::from_library(library)
                .with_public_names()
                .with_parser_names()
        }),
        None => PUBLIC_INDEX.get_or_init(|| {
            HashIndex::from_library(&SchemaLibrary::default())
                .with_public_names()
                .with_parser_names()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::schema::{
        EnumDef, EnumValue, Field, FieldHint, FieldType, Literal, StructDef,
    };

    fn make_library() -> SchemaLibrary {
        let mut lib = SchemaLibrary::new();
        lib.insert_struct(StructDef {
            name: "CFooBar".into(),
            base: None,
            abstract_: false,
            simple: true,
            fields: vec![Field {
                name: "m_Probability".into(),
                ty: FieldType::Float,
                init: Some(Literal::Float(0.5)),
                min: None,
                max: None,
                step: None,
                ui_key: false,
                hide_widgets: false,
                description: None,
                hint: FieldHint::None,
            }],
            source_path: "test.psc".into(),
        });
        lib.insert_enum(EnumDef {
            name: "EFooKind".into(),
            values: vec![EnumValue {
                name: "EFK_Default".into(),
                description: None,
                value: Some(0),
            }],
            generate_bitset: false,
            source_path: "test.psc".into(),
        });
        lib
    }

    #[test]
    fn structs_and_fields_are_indexed() {
        let lib = make_library();
        let idx = HashIndex::from_library(&lib);
        let h_struct = crate::literal_string_hash(b"CFooBar");
        let h_field = crate::literal_string_hash(b"m_Probability");
        let h_enum = crate::literal_string_hash(b"EFooKind");
        let h_value = crate::literal_string_hash(b"EFK_Default");
        assert_eq!(idx.lookup(h_struct), Some("CFooBar"));
        assert_eq!(idx.lookup(h_field), Some("m_Probability"));
        assert_eq!(idx.lookup(h_enum), Some("EFooKind"));
        assert_eq!(idx.lookup(h_value), Some("EFK_Default"));
    }

    #[test]
    fn a_second_library_names_only_what_the_first_left_unnamed() {
        let mut second = SchemaLibrary::new();
        for name in ["CFooBar", "cfoobar_other", "hash_0000BEEF"] {
            second.insert_struct(StructDef {
                name: name.into(),
                base: None,
                abstract_: false,
                simple: true,
                fields: vec![],
                source_path: String::new(),
            });
        }
        let mut idx = HashIndex::from_library(&make_library());
        let before = idx.len();
        idx.add_library(&second);
        assert_eq!(
            idx.len(),
            before + 1,
            "one new name; a spelled hash is none"
        );
        let other = crate::literal_string_hash(b"cfoobar_other");
        assert_eq!(idx.lookup(other), Some("cfoobar_other"));
        assert_eq!(idx.lookup(0xBEEF), None);
    }

    #[test]
    fn scoped_struct_double_underscore_spelling_resolves() {
        let mut lib = SchemaLibrary::new();
        lib.insert_struct(StructDef {
            name: "CDataFileMgr::DataFile".into(),
            base: None,
            abstract_: false,
            simple: true,
            fields: vec![],
            source_path: "test.psc".into(),
        });
        let idx = HashIndex::from_library(&lib);
        let under = crate::literal_string_hash(b"CDataFileMgr__DataFile");
        assert_eq!(idx.lookup(under), Some("CDataFileMgr::DataFile"));
    }

    #[test]
    fn an_unknown_name_is_spelled_as_its_hash_and_read_back() {
        let lib = make_library();
        let idx = HashIndex::from_library(&lib);
        assert_eq!(idx.name_or_literal(0xDEADBEEF), "hash_DEADBEEF");
        assert_eq!(
            crate::meta::binary::synth::name_hash("hash_DEADBEEF"),
            0xDEAD_BEEF
        );
        for name in [
            "hash_table",
            "hash_DEADBEE",
            "hash_DEADBEEF0",
            "hash_+EADBEEF",
        ] {
            assert_eq!(
                crate::meta::binary::synth::name_hash(name),
                crate::literal_string_hash(name.as_bytes()),
                "{name} is a name, not a hash"
            );
        }
    }

    /// Without a snapshot the world metadata reads as numbers, and this is
    /// what gives it names. The hashes are ones a retail install ships.
    #[test]
    fn public_names_resolve_what_an_empty_library_cannot() {
        let bare = HashIndex::from_library(&SchemaLibrary::new());
        assert_eq!(bare.lookup(0xD359_3FA6), None);
        let named = bare.with_public_names();
        assert_eq!(named.lookup(0xD359_3FA6), Some("CMapData"));
        assert_eq!(named.lookup(0xCCAB_9307), Some("entities"));
        assert_eq!(named.lookup(0x0116_6164), Some("position"));
    }

    /// ⚠ Seeding order is load-bearing: the library indexes
    /// `rage::fwInstancedMapData` under its double-underscore hash too, the
    /// slot the public table claims, so seeding the table first renames it.
    #[test]
    fn the_library_outranks_the_public_table() {
        let mut lib = SchemaLibrary::new();
        lib.insert_struct(StructDef {
            name: "rage::fwInstancedMapData".into(),
            base: None,
            abstract_: false,
            simple: true,
            fields: vec![],
            source_path: "test.psc".into(),
        });
        let under = crate::literal_string_hash(b"rage__fwInstancedMapData");
        let idx = HashIndex::from_library(&lib).with_public_names();
        assert_eq!(idx.lookup(under), Some("rage::fwInstancedMapData"));
    }

    /// Seeding is additive: it never changes an answer the library already
    /// gave, so staging a snapshot cannot be made worse by this table.
    #[test]
    fn the_override_is_scoped_to_its_closure_and_its_thread() {
        assert!(!PUBLIC_ONLY.with(Cell::get));
        with_public_names_only(|| {
            assert!(PUBLIC_ONLY.with(Cell::get));
            with_public_names_only(|| {});
            assert!(
                PUBLIC_ONLY.with(Cell::get),
                "the inner call restores what the outer set"
            );
            std::thread::spawn(|| assert!(!PUBLIC_ONLY.with(Cell::get)))
                .join()
                .expect("another thread is not under the override");
        });
        assert!(!PUBLIC_ONLY.with(Cell::get));
    }

    #[test]
    fn seeding_only_adds() {
        let lib = make_library();
        let before = HashIndex::from_library(&lib);
        let h_field = crate::literal_string_hash(b"m_Probability");
        let after = HashIndex::from_library(&lib).with_public_names();
        assert_eq!(after.lookup(h_field), before.lookup(h_field));
        assert!(after.len() > before.len());
    }

    /// Parser-name pairs are filed by the hash as given — a GTA VI hash the
    /// name does not literally hash to — and never displace what is there.
    #[test]
    fn parser_pairs_are_filed_by_hash_and_do_not_overwrite() {
        let lib = make_library();
        let h_field = crate::literal_string_hash(b"m_Probability");
        let idx = HashIndex::from_library(&lib).with_pairs([
            (0x16F5_3758, "camMetadata".to_string()),
            (h_field, "wrong".to_string()),
        ]);
        // A fresh hash gets its name though it is not that name's literal hash.
        assert_eq!(idx.lookup(0x16F5_3758), Some("camMetadata"));
        // The library's answer is kept; the pair does not overwrite it.
        assert_eq!(idx.lookup(h_field), Some("m_Probability"));
    }
}
