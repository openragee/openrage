//! `asset.json`: the file every source folder holds, and how a folder is recognised as a source.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::json::canonical;
use crate::Problem;

/// The `format` every `asset.json` states.
pub const FORMAT: &str = "openrage-source";
/// The layout of `asset.json` this build writes and the newest it reads.
pub const SCHEMA: u32 = 1;
/// The marker file inside every source folder.
pub const MANIFEST: &str = "asset.json";

/// The contents of `asset.json`; nothing in it changes when the payload does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "asset.json")]
pub struct AssetManifest {
    #[schemars(extend("const" = "openrage-source"))]
    pub format: String,
    pub schema: u32,
    /// A word with no platform letter that chooses the compiler, such as `textures`.
    pub kind: String,
    /// Made once and never changed, so a rename or a move breaks no reference to the asset.
    pub id: String,
    /// One plain file name: the folder is `<name>.<kind>.src` and the cook writes `<name>.ytd`.
    pub name: String,
    /// The slug of the RSC7 layout the asset compiles to, such as `gen8`.
    pub target: String,
    #[serde(default = "win64")]
    pub platform: String,
    pub origin: Origin,
    pub created_with: String,
    #[serde(default)]
    pub notes: String,
    /// Who made the asset and on what terms it may be used, which a build can list in a credits file; nothing compiles from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<Credit>,
    /// Keys a later build wrote, such as a model's `compile`: carried, never dropped.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Where an asset's content came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    pub how: How,
    /// True when the content is the game's, which a build leaves out while it is still the game's file and a repository does not keep by default.
    pub from_game: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_build: Option<String>,
    /// The SHA-256 of the compiled original, to tell that the game's file changed after an update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The SHA-256 of what the cook of this source made when the game's file was imported, which a build takes for the game's file as `sha256` is, because a cook writes OpenRage's form of a file and not the game's bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooked_sha256: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Who made an asset, where it was found and on what terms it may be used: what a credits file says of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Credit {
    /// Who made it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// Where it was found: a page, a shop, or the pack it was taken from.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// The terms it may be used on: the name of a licence such as `CC-BY-4.0`, or a sentence.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub licence: String,
}

impl Credit {
    /// Whether it says nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum How {
    Extracted,
    Authored,
    Imported,
}

fn win64() -> String {
    "win64".to_string()
}

impl AssetManifest {
    /// A manifest for a new asset, with an id of its own.
    pub fn new(kind: &str, name: &str, target: &str, origin: Origin) -> Result<Self, Problem> {
        if let Some(bad) = unsafe_file_name(name) {
            return Err(Problem::new(format!("name: {bad}")).in_file(MANIFEST));
        }
        Ok(Self {
            format: FORMAT.to_string(),
            schema: SCHEMA,
            kind: kind.to_string(),
            id: new_id()?,
            name: name.to_string(),
            target: target.to_string(),
            platform: win64(),
            origin,
            created_with: format!("openrage {}", env!("CARGO_PKG_VERSION")),
            notes: String::new(),
            credit: None,
            extra: BTreeMap::new(),
        })
    }

    /// The manifest `text` holds; refused when its header is not this format's or is newer than this build reads.
    pub fn parse(text: &str) -> Result<Self, String> {
        // The header is judged before the shape: a newer schema may have reshaped every other field.
        #[derive(Deserialize)]
        struct Header {
            format: Option<String>,
            schema: Option<u64>,
        }
        let header: Header = serde_json::from_str(text).map_err(|e| e.to_string())?;
        match header.format.as_deref() {
            Some(FORMAT) => {}
            Some(other) => return Err(format!("format is {other:?}, not {FORMAT:?}")),
            None => return Err(format!("format is missing, and a source states {FORMAT:?}")),
        }
        match header.schema {
            Some(schema) if (1..=u64::from(SCHEMA)).contains(&schema) => {}
            Some(schema) => {
                return Err(format!(
                    "schema {schema} is not one this build reads (1 to {SCHEMA})"
                ))
            }
            None => return Err("schema is missing".to_string()),
        }
        let manifest: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        // All three become path components of what a build or an import writes, and a source is a file anyone can edit or clone.
        for (field, value) in [("id", &manifest.id), ("name", &manifest.name)] {
            if let Some(bad) = unsafe_file_name(value) {
                return Err(format!("{field}: {bad}"));
            }
        }
        if !is_kind_word(&manifest.kind) {
            return Err(format!(
                "kind: {:?} is not a lowercase word, which is what the folder `<name>.<kind>.src` spells",
                manifest.kind
            ));
        }
        Ok(manifest)
    }

    /// The canonical text of the manifest.
    pub fn to_text(&self) -> String {
        // A struct of strings, numbers and JSON values always serialises.
        canonical(self).unwrap_or_default()
    }

    /// The text a recook digest hashes: the manifest as it is without its notes and its credit, which no compile reads.
    pub fn digest_text(&self) -> String {
        Self {
            notes: String::new(),
            credit: None,
            ..self.clone()
        }
        .to_text()
    }
}

/// A source folder: the directory and the manifest its header decided it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub dir: PathBuf,
    pub manifest: AssetManifest,
}

impl Source {
    /// The source in `dir`, recognised by its marker file and its header; the folder's own name does not decide.
    pub fn open(dir: &Path) -> Result<Self, Problem> {
        let marker = dir.join(MANIFEST);
        let text = std::fs::read_to_string(&marker).map_err(|e| {
            Problem::new(format!("{} is not an OpenRage source: {e}", dir.display()))
                .in_file(MANIFEST)
        })?;
        let manifest = AssetManifest::parse(&text).map_err(|why| {
            Problem::new(format!(
                "{} is not an OpenRage source: {why}",
                dir.display()
            ))
            .in_file(MANIFEST)
        })?;
        if let Some(fixed) = fixed_file(&manifest.kind) {
            if !dir.join(fixed).is_file() {
                return Err(Problem::new(format!(
                    "a {} source holds a {fixed}, and {} has none",
                    manifest.kind,
                    dir.display()
                ))
                .in_file(fixed));
            }
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    /// Writes `asset.json` into the folder.
    pub fn write_manifest(&self) -> Result<(), Problem> {
        std::fs::write(self.dir.join(MANIFEST), self.manifest.to_text())
            .map_err(|e| Problem::new(format!("cannot write: {e}")).in_file(MANIFEST))
    }

    /// Writes `asset.json` and rewrites every other JSON file of the folder canonically, touching only a file whose text changes.
    ///
    /// ⚠ Every file is read and parsed before any is written, so a file that is no JSON stops the
    /// save with nothing changed, and a source nobody edited is saved without changing a byte.
    pub fn save(&self) -> Result<(), Problem> {
        let mut texts = vec![(self.dir.join(MANIFEST), self.manifest.to_text())];
        let mut folders = vec![self.dir.clone()];
        while let Some(folder) = folders.pop() {
            let cannot =
                |e: std::io::Error| Problem::new(format!("cannot read {}: {e}", folder.display()));
            for entry in std::fs::read_dir(&folder).map_err(cannot)?.flatten() {
                let path = entry.path();
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() && !hidden => folders.push(path),
                    Ok(kind) if kind.is_file() && is_json(&path) && path != texts[0].0 => {
                        let shown = path.strip_prefix(&self.dir).map_or_else(
                            |_| path.display().to_string(),
                            |p| p.to_string_lossy().replace('\\', "/"),
                        );
                        let read = std::fs::read_to_string(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|text| {
                                serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())
                            })
                            .and_then(|value| canonical(&value).map_err(|e| e.to_string()));
                        texts.push((path, read.map_err(|why| Problem::new(why).in_file(shown))?));
                    }
                    _ => {}
                }
            }
        }
        for (path, text) in texts {
            if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                std::fs::write(&path, text).map_err(|e| {
                    Problem::new(format!("cannot write: {e}")).in_file(path.display().to_string())
                })?;
            }
        }
        Ok(())
    }
}

fn is_json(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// States who made the source in `dir` and on what terms it may be used; an empty credit takes the statement away.
///
/// Nothing else of the folder is touched, and nothing compiles from it, so a recook has nothing to do.
pub fn set_credit(dir: &Path, credit: Credit) -> Result<(), Problem> {
    let mut source = Source::open(dir)?;
    source.manifest.credit = (!credit.is_empty()).then_some(credit);
    source.write_manifest()
}

/// Gives the source in `dir`, which is a copy of another, an id of its own, and returns it.
pub fn reid(dir: &Path) -> Result<String, Problem> {
    let mut source = Source::open(dir)?;
    source.manifest.id = new_id()?;
    source.write_manifest()?;
    Ok(source.manifest.id)
}

/// The file besides `asset.json` that a source of `kind` always holds, for the kinds this build knows.
fn fixed_file(kind: &str) -> Option<&'static str> {
    (kind == crate::textures::KIND)
        .then_some(crate::textures::RULES)
        .or_else(|| (kind == crate::metadata::KIND).then_some(crate::metadata::XML))
        .or_else(|| (kind == crate::particles::KIND).then_some(crate::particles::RULES))
        .or_else(|| (kind == crate::bounds::KIND).then_some(crate::bounds::RULES))
        .or_else(|| (kind == crate::paths::KIND).then_some(crate::paths::RULES))
        .or_else(|| crate::models::fixed_file(kind))
}

/// Why `name` cannot be one file's or folder's name: a separator, `.` or `..`, a control or reserved character, a trailing dot or space, or a Windows device name.
pub fn unsafe_file_name(name: &str) -> Option<String> {
    let bad = name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c));
    if bad {
        return Some(format!(
            "{name:?} cannot be the name of a file in a source folder"
        ));
    }
    is_device_name(name).then(|| {
        format!("{name:?} is a device name on Windows, so it cannot be the name of a file in a source folder")
    })
}

/// Whether Windows takes `name` for a device and not a file: `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9` and `LPT1` to `LPT9`, whatever follows a dot.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let stem = stem.to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
}

/// The folder a `kind` asset called `name` lives in.
pub fn folder_name(name: &str, kind: &str) -> String {
    format!("{name}.{kind}.src")
}

/// The name and the kind a folder called `folder` states, when it is spelled `<name>.<kind>.src`.
pub fn parse_folder_name(folder: &str) -> Option<(&str, &str)> {
    let (name, kind) = folder.strip_suffix(".src")?.rsplit_once('.')?;
    (is_kind_word(kind) && !name.is_empty()).then_some((name, kind))
}

/// Whether `kind` is a word of lowercase letters, which is the only thing a folder name can spell as a kind.
fn is_kind_word(kind: &str) -> bool {
    !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_lowercase())
}

/// A random version 4 UUID in its 36-character lowercase spelling.
pub fn new_id() -> Result<String, Problem> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| Problem::new(format!("no randomness for a new id: {e}")))?;
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let hex = hex(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// The SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design's example, with its keys in the order a person would write them.
    const AS_WRITTEN: &str = r#"{
    "format": "openrage-source",
    "schema": 1,
    "kind": "drawables",
    "id": "7c1e2b0a-5d1f-4e7a-9b3c-2a6f0d8e4c11",
    "name": "jbib_012_u",
    "target": "gen8",
    "platform": "win64",
    "origin": {
      "how": "extracted",
      "from_game": true,
      "virtual_path": "x64v.rpf/…/jbib_012_u.ydd",
      "game_build": "…",
      "sha256": "9f2c…"
    },
    "compile": { "mode": "edit", "base": "9f2c…" },
    "created_with": "openrage 0.1.0",
    "notes": ""
}"#;

    /// The same file as the writer must spell it: keys sorted at every level, two-space indent, LF, one final newline.
    const CANONICAL: &str = "{
  \"compile\": {
    \"base\": \"9f2c…\",
    \"mode\": \"edit\"
  },
  \"created_with\": \"openrage 0.1.0\",
  \"format\": \"openrage-source\",
  \"id\": \"7c1e2b0a-5d1f-4e7a-9b3c-2a6f0d8e4c11\",
  \"kind\": \"drawables\",
  \"name\": \"jbib_012_u\",
  \"notes\": \"\",
  \"origin\": {
    \"from_game\": true,
    \"game_build\": \"…\",
    \"how\": \"extracted\",
    \"sha256\": \"9f2c…\",
    \"virtual_path\": \"x64v.rpf/…/jbib_012_u.ydd\"
  },
  \"platform\": \"win64\",
  \"schema\": 1,
  \"target\": \"gen8\"
}
";

    #[test]
    fn a_file_written_in_another_order_is_rewritten_canonically() {
        let manifest = AssetManifest::parse(AS_WRITTEN).unwrap();
        assert_eq!(manifest.to_text(), CANONICAL);
    }

    #[test]
    fn canonical_text_is_a_fixed_point() {
        let once = AssetManifest::parse(CANONICAL).unwrap().to_text();
        assert_eq!(once, CANONICAL);
        assert_eq!(AssetManifest::parse(&once).unwrap().to_text(), once);
        assert!(!once.contains('\r') && once.ends_with("}\n") && !once.ends_with("\n\n"));
    }

    #[test]
    fn a_key_this_build_does_not_know_is_carried_and_a_default_is_written() {
        let text = r#"{"format":"openrage-source","schema":1,"kind":"textures","id":"a","name":"n",
            "target":"gen9","origin":{"how":"authored","from_game":false,"future":[1,2]},
            "created_with":"x","tomorrow":{"b":1,"a":2}}"#;
        let written = AssetManifest::parse(text).unwrap().to_text();
        assert!(written.contains("\"tomorrow\": {\n    \"a\": 2,\n    \"b\": 1\n  }"));
        assert!(written.contains("\"future\": [\n      1,\n      2\n    ]"));
        assert!(written.contains("\"platform\": \"win64\""));
        assert!(written.contains("\"notes\": \"\""));
    }

    #[test]
    fn a_header_this_build_does_not_read_is_refused_and_says_why() {
        let with = |edit: &dyn Fn(&str) -> String| edit(CANONICAL);
        let wrong_format = with(&|t| t.replace("openrage-source", "something-else"));
        let newer = with(&|t| t.replace("\"schema\": 1", "\"schema\": 2"));
        let no_id = with(&|t| t.replace("7c1e2b0a-5d1f-4e7a-9b3c-2a6f0d8e4c11", ""));
        assert!(AssetManifest::parse(&wrong_format)
            .unwrap_err()
            .contains("something-else"));
        assert!(AssetManifest::parse(&newer)
            .unwrap_err()
            .contains("schema 2"));
        assert!(AssetManifest::parse(&no_id).unwrap_err().contains("id"));
        assert!(AssetManifest::parse("{}").is_err());
    }

    #[test]
    fn a_header_is_judged_before_the_shape_it_would_have_had() {
        // A newer schema may rename every field: it must be refused as newer, not as missing an `id`.
        let newer = r#"{"format":"openrage-source","schema":99,"kind":["a","list"]}"#;
        let why = AssetManifest::parse(newer).unwrap_err();
        assert!(why.contains("schema 99"), "{why}");
        let other = r#"{"format":"something-else","schema":1,"origin":5}"#;
        let why = AssetManifest::parse(other).unwrap_err();
        assert!(why.contains("something-else"), "{why}");
        let none = r#"{"kind":"textures"}"#;
        let why = AssetManifest::parse(none).unwrap_err();
        assert!(why.contains("format"), "{why}");
        let text = r#"{"format":"openrage-source","schema":0}"#;
        assert!(AssetManifest::parse(text).unwrap_err().contains("schema 0"));
        // With a good header the shape is judged, and says what is missing.
        let shapeless = r#"{"format":"openrage-source","schema":1}"#;
        assert!(AssetManifest::parse(shapeless)
            .unwrap_err()
            .contains("missing field"));
    }

    #[test]
    fn a_windows_device_name_is_not_a_file_name_with_or_without_an_extension() {
        let devices = ["CON", "PRN", "AUX", "NUL"]
            .into_iter()
            .map(str::to_string)
            .chain((1..=9).map(|n| format!("COM{n}")))
            .chain((1..=9).map(|n| format!("LPT{n}")));
        for device in devices {
            for spelling in [
                device.clone(),
                device.to_lowercase(),
                format!("{device}.dds"),
                format!("{}.tar.gz", device.to_lowercase()),
                format!("{device} .txt"),
            ] {
                let why = unsafe_file_name(&spelling)
                    .unwrap_or_else(|| panic!("{spelling:?} was taken for a file name"));
                assert!(why.contains("device"), "{spelling:?}: {why}");
            }
        }
        for plain in [
            "console", "com10", "COM0", "LPT", "aux1", "nul_", "a.con", "prn2.dds",
        ] {
            assert_eq!(unsafe_file_name(plain), None, "{plain:?}");
        }
        let origin = Origin {
            how: How::Authored,
            from_game: false,
            virtual_path: None,
            game_build: None,
            sha256: None,
            cooked_sha256: None,
            extra: BTreeMap::new(),
        };
        let problem = AssetManifest::new("textures", "NUL", "gen8", origin).unwrap_err();
        assert!(problem.message.contains("name: "), "{problem}");
        let mut manifest: Value = serde_json::from_str(CANONICAL).unwrap();
        manifest["id"] = "aux.json".into();
        let why = AssetManifest::parse(&manifest.to_string()).unwrap_err();
        assert!(why.contains("id: "), "{why}");
    }

    #[test]
    fn a_kind_that_is_not_a_lowercase_word_is_refused_by_the_parser() {
        for hostile in [
            "x/../../../escaped",
            r"..\x",
            "..",
            "",
            "Textures",
            "gen9textures",
            "a.b",
            "tex tures",
        ] {
            let mut manifest: Value = serde_json::from_str(CANONICAL).unwrap();
            manifest["kind"] = hostile.into();
            let why = AssetManifest::parse(&manifest.to_string()).unwrap_err();
            assert!(why.starts_with("kind: "), "{hostile:?}: {why}");
        }
        assert!(AssetManifest::parse(CANONICAL).is_ok());
    }

    #[test]
    fn an_id_is_a_version_4_uuid_and_never_repeats() {
        let a = new_id().unwrap();
        let b = new_id().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4') && "89ab".contains(&parts[3][..1]));
        assert!(a
            .bytes()
            .all(|c| c == b'-' || c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    }

    #[test]
    fn sha256_is_the_published_digest_of_abc() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_folder_name_states_its_name_and_kind() {
        assert_eq!(
            folder_name("jbib_diff", "textures"),
            "jbib_diff.textures.src"
        );
        assert_eq!(
            parse_folder_name("jbib_diff.textures.src"),
            Some(("jbib_diff", "textures"))
        );
        assert_eq!(
            parse_folder_name("a.b.drawables.src"),
            Some(("a.b", "drawables"))
        );
        for no in [
            "jbib.ytd",
            "jbib.src",
            ".textures.src",
            "jbib.Textures.src",
            "textures.src",
        ] {
            assert_eq!(parse_folder_name(no), None, "{no}");
        }
    }

    #[test]
    fn the_header_decides_and_the_folders_own_name_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let renamed = dir.path().join("copy of my pack");
        std::fs::create_dir(&renamed).unwrap();
        let mut manifest = AssetManifest::parse(CANONICAL).unwrap();
        manifest.kind = "textures".to_string();
        std::fs::write(renamed.join(MANIFEST), manifest.to_text()).unwrap();

        let lacking = Source::open(&renamed).unwrap_err();
        assert_eq!(lacking.file.as_deref(), Some("textures.json"));
        std::fs::write(renamed.join("textures.json"), "{}").unwrap();
        let source = Source::open(&renamed).unwrap();
        assert_eq!(source.manifest.id, manifest.id);

        let named_like_one = dir.path().join("x.textures.src");
        std::fs::create_dir(&named_like_one).unwrap();
        assert!(
            Source::open(&named_like_one).is_err(),
            "a suffix alone is not a source"
        );
        std::fs::write(named_like_one.join(MANIFEST), "{\"format\":\"other\"}").unwrap();
        assert!(Source::open(&named_like_one).is_err());
    }

    #[test]
    fn an_id_or_a_name_that_is_a_path_is_refused_by_the_parser_and_by_new() {
        let hostile = [
            r"..\..\x", "../x", "a/b", r"C:\x", "/etc", "..", ".", "", "x ",
        ];
        for key in ["id", "name"] {
            for value in hostile {
                let mut manifest: Value = serde_json::from_str(CANONICAL).unwrap();
                manifest[key] = value.into();
                let why = AssetManifest::parse(&manifest.to_string()).unwrap_err();
                assert!(why.contains(&format!("{key}: ")), "{key} {value:?}: {why}");
            }
        }
        let origin = Origin {
            how: How::Authored,
            from_game: false,
            virtual_path: None,
            game_build: None,
            sha256: None,
            cooked_sha256: None,
            extra: BTreeMap::new(),
        };
        for name in hostile {
            let problem = AssetManifest::new("textures", name, "gen8", origin.clone()).unwrap_err();
            assert!(problem.message.contains("name: "), "{name:?}: {problem}");
        }
        assert!(AssetManifest::new("textures", "jbib_012_u.v2", "gen8", origin).is_ok());
    }
}
