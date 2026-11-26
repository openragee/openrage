//! Real names for GTA VI's `rage::par` structures, members and enums, loaded
//! from the recovered-name corpus at run time.
//!
//! GTA VI's PSO bodies store every structure, member and enum as a joaat hash
//! of its C++ name, so without names a `.ymt`-equivalent reads as `<0x16F53758>`
//! rather than its class name. Two files, derived offline from the engine's
//! compiled `par` tree and the recovered-name search, carry those names:
//!
//! - `schema-named.json`: every structure and enum by its name hash, with the
//!   clear name where the dump knows it (`_0x........` where it does not), its
//!   byte size, and its members by file order (name, offset, type, subtype).
//!   The structure/enum hash in the file is the engine's own, cross-checked
//!   against the exe `parStructure` registry by hash and size (e.g. hash
//!   `0x16F53758` carries size `0x340`), so a structure or enum name attaches
//!   to that hash with no recomputation.
//! - `resolved-names.tsv`: a flat `0xHHHHHHHH<TAB>name`, one recovered name per
//!   row, each already keyed by the on-disk hash the row resolved.
//!
//! Neither file is ours to redistribute: both are gitignored, a clean clone has
//! neither, and nothing is compiled in. [`installed`] finds them through
//! [`CORPUS_DIR_ENV`], then takes the 32-bit rows of the table `hashes mine`
//! keeps in the configuration folder a host named with [`use_config_dir`];
//! without either this resolves nothing and the hashes show as
//! `hash_XXXXXXXX`, exactly as a clean checkout does.
//!
//! ⚠ A member or enum-value clear name carries no numeric hash in the file, and
//! the engine hashes those either literally or case-folded (both occur). The
//! loader keeps the safety a hash gives: it files each clear name under *both*
//! its literal and its folded hash, so whichever the body used resolves, and a
//! name whose hash matches neither of a body's members simply never attaches.
//! Structure and enum names, which the file keys numerically, are filed under
//! that one hash and never guessed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

/// The directory holding `schema-named.json` and `resolved-names.tsv`.
pub const CORPUS_DIR_ENV: &str = "OPENRAGE_PARSER_NAMES";
const SCHEMA_NAMED_FILE: &str = "schema-named.json";
const RESOLVED_NAMES_FILE: &str = "resolved-names.tsv";

/// Why the recovered-name corpus could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

/// One `rage::par` structure's layout: its clear name where the dump knows it,
/// its byte size, and its members in file order.
#[derive(Debug, Clone)]
pub struct StructSchema {
    /// The engine's class name, or `None` when the dump has only the hash.
    pub name: Option<String>,
    /// The structure's byte size (`parStructure::size`).
    pub size: u32,
    pub fields: Vec<FieldSchema>,
}

/// One member of a structure.
#[derive(Debug, Clone)]
pub struct FieldSchema {
    /// The member's clear name, or `None` when the dump has only its hash.
    pub name: Option<String>,
    /// The member's own name hash, known only when the name is hashed.
    pub hash: Option<u32>,
    /// The member's byte offset in the structure.
    pub offset: u32,
    pub size: u32,
    /// The engine `parMemberType` as the dump spells it, e.g. `FLOAT`, `STRING`.
    pub ty: String,
    /// The member's subtype, e.g. `ATNONFINALHASHSTRING`.
    pub subtype: String,
}

/// The recovered names: every structure's layout by its hash, and a flat
/// hash → name index over structures, enums, members and the resolved rows.
#[derive(Debug, Default)]
pub struct ParserNames {
    structs: HashMap<u32, StructSchema>,
    names: HashMap<u32, String>,
}

impl ParserNames {
    /// Build the index from the two files' bytes, either absent.
    ///
    /// `schema_json` is the bytes of `schema-named.json`, `resolved_tsv` the
    /// text of `resolved-names.tsv`. Later sources never overwrite an earlier
    /// one's answer, so the recorded rows and the numerically-keyed structure
    /// and enum names outrank the member names filed under both hash flavours.
    pub fn parse(
        schema_json: Option<&[u8]>,
        resolved_tsv: Option<&str>,
    ) -> Result<Self, LoadError> {
        let mut out = ParserNames::default();

        if let Some(tsv) = resolved_tsv {
            out.load_resolved(tsv);
        }
        if let Some(bytes) = schema_json {
            let file: SchemaNamedFile =
                serde_json::from_slice(bytes).map_err(|source| LoadError::Json {
                    path: PathBuf::from(SCHEMA_NAMED_FILE),
                    source,
                })?;
            out.load_schema(file);
        }
        Ok(out)
    }

    /// Load from `dir`, reading whichever of the two files is present. An absent
    /// file is skipped (the offline ingest may still be running); a present but
    /// unreadable or malformed one is an error.
    pub fn load_dir(dir: &Path) -> Result<Self, LoadError> {
        let schema = read_optional(&dir.join(SCHEMA_NAMED_FILE))?;
        let resolved = read_optional(&dir.join(RESOLVED_NAMES_FILE))?;
        let resolved = resolved
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned());
        Self::parse(schema.as_deref(), resolved.as_deref())
    }

    fn load_resolved(&mut self, tsv: &str) {
        for line in tsv.lines() {
            let mut cols = line.splitn(2, '\t');
            let (Some(hash), Some(name)) = (cols.next(), cols.next()) else {
                continue;
            };
            let name = name.trim_end_matches('\r');
            if let Some(hash) = parse_hash(hash.trim()) {
                if !name.is_empty() {
                    self.names.entry(hash).or_insert_with(|| name.to_string());
                }
            }
        }
    }

    fn load_schema(&mut self, file: SchemaNamedFile) {
        // Pass 1: structure and enum names, keyed by the file's own hash.
        for s in &file.structs {
            let Some(hash) = parse_hash(&s.hash) else {
                continue;
            };
            if let Some(name) = clear(&s.name) {
                self.names.entry(hash).or_insert_with(|| name.to_string());
            }
        }
        for e in &file.enums {
            if let Some(hash) = parse_hash(&e.hash) {
                if let Some(name) = clear(&e.name) {
                    self.names.entry(hash).or_insert_with(|| name.to_string());
                }
            }
        }
        // Pass 2: member and enum-value clear names, under both hash flavours.
        for s in &file.structs {
            for f in &s.fields {
                self.file_clear_name(&f.name);
            }
        }
        for e in &file.enums {
            for v in &e.values {
                self.file_clear_name(&v.name);
            }
        }
        // Pass 3: the structure layouts.
        for s in file.structs {
            let Some(hash) = parse_hash(&s.hash) else {
                continue;
            };
            let fields = s
                .fields
                .into_iter()
                .map(|f| {
                    let name = clear(&f.name).map(str::to_string);
                    let hash = if name.is_none() {
                        parse_hash(&f.name)
                    } else {
                        None
                    };
                    FieldSchema {
                        name,
                        hash,
                        offset: f.offset,
                        size: f.size,
                        ty: f.ty,
                        subtype: f.subtype,
                    }
                })
                .collect();
            self.structs.insert(
                hash,
                StructSchema {
                    name: clear(&s.name).map(str::to_string),
                    size: s.size,
                    fields,
                },
            );
        }
    }

    /// File a clear member/value name under both the literal and the folded
    /// joaat of its text, the two the engine hashes member names with.
    fn file_clear_name(&mut self, raw: &str) {
        let Some(name) = clear(raw) else {
            return;
        };
        for hash in [
            crate::literal_string_hash(name.as_bytes()),
            crate::string_hash(name.as_bytes()),
        ] {
            self.names.entry(hash).or_insert_with(|| name.to_string());
        }
    }

    /// The clear name a hash resolves to, over all four sources.
    pub fn name(&self, hash: u32) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }

    /// One structure's layout by its name hash.
    pub fn struct_schema(&self, hash: u32) -> Option<&StructSchema> {
        self.structs.get(&hash)
    }

    /// The clear name of the member of structure `struct_hash` at `offset`.
    pub fn field_name(&self, struct_hash: u32, offset: u32) -> Option<&str> {
        self.structs
            .get(&struct_hash)?
            .fields
            .iter()
            .find(|f| f.offset == offset)?
            .name
            .as_deref()
    }

    /// Every `(hash, name)` the flat index holds.
    pub fn pairs(&self) -> impl Iterator<Item = (u32, &str)> {
        self.names.iter().map(|(&h, n)| (h, n.as_str()))
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// The configuration folder [`installed`] reads the mined table of.
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Has [`installed`] read the class and member rows of [`mined_table`] under `config`; only the first call before the first read counts.
pub fn use_config_dir(config: &Path) {
    let _ = CONFIG_DIR.set(config.to_path_buf());
}

/// The table `hashes mine` merges its rows into under the configuration folder `config`, which the GTA VI names cache reads too.
pub fn mined_table(config: &Path) -> PathBuf {
    config.join("gta6_names").join(RESOLVED_NAMES_FILE)
}

/// The recovered names of [`CORPUS_DIR_ENV`] and of the configuration folder's
/// mined table, or `None` when neither names anything — the clean-checkout case.
///
/// ⚠ Resolved once: the hash index caches what this returns.
pub fn installed() -> Option<&'static ParserNames> {
    static INSTALLED: OnceLock<Option<ParserNames>> = OnceLock::new();
    INSTALLED.get_or_init(discover).as_ref()
}

fn discover() -> Option<ParserNames> {
    let corpus = std::env::var_os(CORPUS_DIR_ENV).map(PathBuf::from);
    match load_from(corpus.as_deref(), CONFIG_DIR.get().map(PathBuf::as_path)) {
        Ok(names) if !names.is_empty() => Some(names),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!("the recovered parser names are unusable: {error}");
            None
        }
    }
}

/// The names of the corpus folder `corpus`, then the 32-bit rows of [`mined_table`] under `config` for the hashes it leaves unnamed.
pub fn load_from(corpus: Option<&Path>, config: Option<&Path>) -> Result<ParserNames, LoadError> {
    let mut names = match corpus {
        Some(dir) => ParserNames::load_dir(dir)?,
        None => ParserNames::default(),
    };
    // Its 64-bit rows are entry names, which `parse_hash` reads as no hash; a 32-bit row is taken
    // only where its name hashes to it, so a stale or foreign row names nothing.
    if let Some(config) = config {
        if let Some(bytes) = read_optional(&mined_table(config))? {
            let text = String::from_utf8_lossy(&bytes);
            let hashed = |line: &&str| {
                let mut cols = line.splitn(2, '\t');
                let hash = cols.next().and_then(|h| parse_hash(h.trim()));
                let name = cols.next().unwrap_or_default().trim_end_matches('\r');
                hash.is_some_and(|h| {
                    crate::hash::string_hash(name.as_bytes()) == h
                        || crate::hash::literal_string_hash(name.as_bytes()) == h
                })
            };
            let rows: Vec<&str> = text.lines().filter(hashed).collect();
            names.load_resolved(&rows.join("\n"));
        }
    }
    Ok(names)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, LoadError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LoadError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// `name` when it is a clear name, `None` when the dump spells it as its hash.
fn clear(name: &str) -> Option<&str> {
    parse_hash(name).is_none().then_some(name)
}

/// The `u32` a `_0x........`, `0x........` or `0X........` string spells, else
/// `None`. Eight hex digits exactly, so a clear name is never read as a hash.
fn parse_hash(text: &str) -> Option<u32> {
    let body = text.strip_prefix('_').unwrap_or(text);
    let hex = body
        .strip_prefix("0x")
        .or_else(|| body.strip_prefix("0X"))?;
    if hex.len() != 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

#[derive(Deserialize)]
struct SchemaNamedFile {
    #[serde(default)]
    structs: Vec<RawStruct>,
    #[serde(default)]
    enums: Vec<RawEnum>,
}

#[derive(Deserialize)]
struct RawStruct {
    hash: String,
    name: String,
    #[serde(default)]
    size: u32,
    #[serde(default)]
    fields: Vec<RawField>,
}

#[derive(Deserialize)]
struct RawField {
    name: String,
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    size: u32,
    #[serde(default, rename = "type")]
    ty: String,
    #[serde(default)]
    subtype: String,
}

#[derive(Deserialize)]
struct RawEnum {
    hash: String,
    name: String,
    #[serde(default)]
    values: Vec<RawEnumValue>,
}

#[derive(Deserialize)]
struct RawEnumValue {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny synthetic corpus in the documented shape: one clear structure
    /// with a clear and a hashed member, one hashed structure, one enum, and
    /// two resolved rows. The hashes and names are invented, not recovered.
    const SCHEMA: &[u8] = br#"{
        "meta": {"note": "fixture"},
        "structs": [
            {"hash":"_0x11112222","name":"WidgetMeta","size":16,"align":4,"parent":null,
             "fields":[
                {"name":"intensity","offset":0,"size":4,"type":"FLOAT","subtype":"NONE"},
                {"name":"_0xAABBCCDD","offset":4,"size":1,"type":"UCHAR","subtype":"NONE"}
             ]},
            {"hash":"_0x33334444","name":"_0x33334444","size":4,"fields":[
                {"name":"Flags","offset":0,"size":4,"type":"BITSET","subtype":"_32BIT"}
             ]}
        ],
        "enums": [
            {"hash":"_0x55556666","name":"eWidgetKind","values":[
                {"name":"WK_First","value":0},{"name":"_0x77778888","value":3}
            ]}
        ]
    }"#;
    const RESOLVED: &str = "0x1A2B3C4D\twidget_param\n0x5E6F7A8B\tfolder/thing.dat\n";

    fn fixture() -> ParserNames {
        ParserNames::parse(Some(SCHEMA), Some(RESOLVED)).expect("the fixture parses")
    }

    #[test]
    fn a_structure_name_attaches_to_the_file_s_own_hash() {
        let names = fixture();
        assert_eq!(names.name(0x1111_2222), Some("WidgetMeta"));
        // The hashed structure has no clear name to give.
        assert_eq!(names.name(0x3333_4444), None);
        assert_eq!(names.name(0x5555_6666), Some("eWidgetKind"));
    }

    #[test]
    fn a_clear_member_resolves_under_both_literal_and_folded_hashes() {
        let names = fixture();
        assert_eq!(
            names.name(crate::literal_string_hash(b"intensity")),
            Some("intensity")
        );
        assert_eq!(
            names.name(crate::string_hash(b"intensity")),
            Some("intensity")
        );
        // Likewise an enum value's clear name.
        assert_eq!(
            names.name(crate::literal_string_hash(b"WK_First")),
            Some("WK_First")
        );
        assert_eq!(
            names.name(crate::string_hash(b"WK_First")),
            Some("WK_First")
        );
    }

    #[test]
    fn a_resolved_row_resolves_by_its_recorded_hash() {
        let names = fixture();
        assert_eq!(names.name(0x1A2B_3C4D), Some("widget_param"));
        assert_eq!(names.name(0x5E6F_7A8B), Some("folder/thing.dat"));
    }

    #[test]
    fn the_structure_layout_carries_size_and_members() {
        let names = fixture();
        let s = names
            .struct_schema(0x1111_2222)
            .expect("the structure is kept");
        assert_eq!(s.name.as_deref(), Some("WidgetMeta"));
        assert_eq!(s.size, 16);
        assert_eq!(s.fields.len(), 2);
        assert_eq!(names.field_name(0x1111_2222, 0), Some("intensity"));
        // A hashed member keeps its hash and yields no name.
        assert_eq!(names.field_name(0x1111_2222, 4), None);
        assert_eq!(s.fields[1].hash, Some(0xAABB_CCDD));
    }

    /// The fingerprint safety: a wrong name never attaches to a body's hash.
    #[test]
    fn an_unmatched_name_attaches_to_nothing() {
        let names = fixture();
        assert_eq!(names.name(0xDEAD_BEEF), None);
        // `WidgetMeta` is filed under 0x11112222 only, so a lookup by the folded
        // joaat of its own string (a different hash) does not return it.
        let other = crate::string_hash(b"WidgetMeta");
        if other != 0x1111_2222 {
            assert_ne!(names.name(other), Some("WidgetMeta"));
        }
    }

    #[test]
    fn either_file_may_be_absent() {
        let only_schema = ParserNames::parse(Some(SCHEMA), None).expect("schema alone");
        // The schema's own structure name resolves; a resolved-only hash does not.
        assert_eq!(only_schema.name(0x1111_2222), Some("WidgetMeta"));
        assert_eq!(only_schema.name(0x1A2B_3C4D), None);

        let only_tsv = ParserNames::parse(None, Some(RESOLVED)).expect("tsv alone");
        // A resolved row resolves; the schema's structure layout is absent.
        assert_eq!(only_tsv.name(0x1A2B_3C4D), Some("widget_param"));
        assert!(only_tsv.struct_schema(0x1111_2222).is_none());

        assert!(ParserNames::parse(None, None).expect("neither").is_empty());
    }

    /// The configuration folder's mined table: its eight-digit rows are read after the corpus's,
    /// which win, and its sixteen-digit entry rows are not.
    #[test]
    fn the_mined_table_of_the_configuration_folder_names_what_the_corpus_leaves_unnamed() {
        let corpus = tempfile::tempdir().expect("a corpus folder");
        let config = tempfile::tempdir().expect("a configuration folder");
        std::fs::write(corpus.path().join(RESOLVED_NAMES_FILE), RESOLVED).expect("written");
        let table = mined_table(config.path());
        std::fs::create_dir_all(table.parent().expect("a folder")).expect("made");
        let class = crate::hash::literal_string_hash(b"ZqWidgetMeta");
        let member = crate::hash::string_hash(b"zq_member");
        let mined = format!(
            "0x00000000000000AB\tzq/entry/name\n0x1a2b3c4d\tmined_other\n0x0000ABCD\tmined_member\n\
             0x{class:08x}\tZqWidgetMeta\n0x{member:08x}\tzq_member\n"
        );
        std::fs::write(&table, mined).expect("written");

        let names = load_from(Some(corpus.path()), Some(config.path())).expect("both load");
        assert_eq!(names.name(class), Some("ZqWidgetMeta"));
        assert_eq!(names.name(member), Some("zq_member"));
        assert_eq!(
            names.name(0x0000_ABCD),
            None,
            "a row its name does not hash to"
        );
        assert_eq!(
            names.name(0x1A2B_3C4D),
            Some("widget_param"),
            "the corpus wins"
        );
        assert_eq!(names.len(), 4, "an entry row is no 32-bit name");
        let alone = load_from(None, Some(config.path())).expect("the table alone");
        assert_eq!(alone.name(class), Some("ZqWidgetMeta"));
        assert_eq!(alone.len(), 2);
        let none = load_from(None, Some(&config.path().join("absent"))).expect("no table");
        assert!(none.is_empty());
    }

    #[test]
    fn hash_spellings_parse_and_clear_names_do_not() {
        assert_eq!(parse_hash("_0x16F53758"), Some(0x16F5_3758));
        assert_eq!(parse_hash("0x002B79A4"), Some(0x002B_79A4));
        assert_eq!(parse_hash("0X0019CF52"), Some(0x0019_CF52));
        for clear_name in ["WidgetMeta", "Flags", "intensity", "0x1234", "_0xZZZZZZZZ"] {
            assert_eq!(parse_hash(clear_name), None, "{clear_name}");
            assert_eq!(clear(clear_name), Some(clear_name), "{clear_name}");
        }
        assert_eq!(clear("_0x11112222"), None);
    }

    /// Install-gated: the real recovered-name corpus, when the environment
    /// points at it. Skips cleanly otherwise, so a clean checkout passes.
    #[test]
    fn the_real_corpus_names_the_cross_checked_structure() {
        let Some(dir) = real_corpus_dir() else {
            or_test_support::skip_install(
                "GTA VI rage::par recovered names",
                format_args!(
                    "set {CORPUS_DIR_ENV}, or stage docs/extra/gta6/intake/ragepar/derived"
                ),
            );
            return;
        };
        let names = ParserNames::load_dir(&dir).expect("the real corpus loads");
        assert!(
            names.len() > 1000,
            "the real corpus resolves many names, saw {}",
            names.len()
        );
        // The exe `parStructure` cross-check: hash 0x16F53758 is size 0x340.
        let cross = names
            .struct_schema(0x16F5_3758)
            .expect("the structure is present");
        assert_eq!(cross.size, 0x340, "the size the exe registry records");
        assert!(cross.name.is_some(), "and it carries a clear name");
        assert!(names.name(0x16F5_3758).is_some());
    }

    /// The real corpus directory: [`CORPUS_DIR_ENV`], else `docs/extra/.../derived`
    /// found by walking up from the crate, which exists only in a full checkout.
    fn real_corpus_dir() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os(CORPUS_DIR_ENV) {
            let dir = PathBuf::from(dir);
            return dir.join(SCHEMA_NAMED_FILE).is_file().then_some(dir);
        }
        let mut cur: Option<&Path> = Some(Path::new(env!("CARGO_MANIFEST_DIR")));
        while let Some(dir) = cur {
            let derived = dir.join("docs/extra/gta6/intake/ragepar/derived");
            if derived.join(SCHEMA_NAMED_FILE).is_file() {
                return Some(derived);
            }
            cur = dir.parent();
        }
        None
    }
}
