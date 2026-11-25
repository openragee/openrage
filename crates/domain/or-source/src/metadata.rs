//! The source of a `.ymt`, `.ymap`, `.ytyp` or `.meta`: OpenRage's own canonical XML, and the file it came from.
//!
//! The XML is `or_io::meta::canonical`'s: it carries no types, a name the build cannot resolve is
//! spelled `hash_XXXXXXXX`, and it is not CodeWalker's. A compile imports it back through the type
//! table of the file it was exported from, kept beside it as `base.<ext>`, so a folder compiles with
//! no schema snapshot and no install, and the same bytes come out wherever it is cooked.
//!
//! ⚠ The compile writes the canonical form, which is not the original's bytes: only a file whose
//! first export and import reaches a fixed point, with no value lost, becomes a source, and the
//! others are refused here for the import to keep as compiled files.
//!
//! ⚠ `game_import::KINDS` has no row for `.meta`: a FiveM data file is registered by its own path and
//! a source ships at `stream/`, so the import of a project keeps a `.meta` under `files/`. A content that
//! states the folder its data sources ship at (the Weapons content: `or_weapons::DATA_FOLDER`) makes
//! the source of a `.meta` itself with [`extract`], and a build of that content cooks it to that path. An edit of its
//! XML that is no XML document is refused.
//!
//! ```text
//! <name>.metadata.src/    asset.json  base.ymap  metadata.xml
//! ```

use std::path::{Component, Path, PathBuf};

use or_io::meta::binary::hash_index::with_public_names_only;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::canonical;
use or_io::meta::{detect_binary_format, Diagnostics, MetaDocument, SchemaLibrary, SourceFormat};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::asset::{folder_name, sha256_hex, AssetManifest, How, Origin, Source, MANIFEST};
use crate::models::{problem, Compiled, Extraction};
use crate::Problem;

/// The `kind` of a metadata file's source.
pub const KIND: &str = "metadata";
/// The canonical XML, which a person edits.
pub const XML: &str = "metadata.xml";
/// The game extensions that become this kind of source.
pub const EXTENSIONS: [&str; 4] = ["ymt", "ymap", "ytyp", "meta"];

/// The `compile` block of `asset.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Compile {
    /// The game extension the cooked file has.
    extension: String,
    /// The SHA-256 of the file kept as the base, which the XML was exported from.
    base: String,
}

/// Everything a compile reads, so a recook can tell nothing changed before it imports anything.
#[derive(Clone)]
pub struct Inputs {
    manifest: AssetManifest,
    compile: Compile,
    base: Vec<u8>,
    xml: Vec<u8>,
}

/// The file a source of `extension` keeps its original in.
fn base_file(extension: &str) -> String {
    format!("base.{extension}")
}

/// Why `extension` is not one a metadata source holds.
fn unknown_extension(extension: &str) -> Problem {
    problem(
        format!(
            "compile.extension is {extension:?}, and a metadata source holds one of {}",
            EXTENSIONS.join(", ")
        ),
        MANIFEST,
    )
}

/// Writes `bytes`, a game file with `extension`, into `parent` as `<name>.metadata.src` and returns the folder.
///
/// A file whose XML does not come back to a fixed point, loses a value, or is no metadata document is
/// refused with the reason, and nothing is left behind. The XML is the same on every host.
///
/// ⚠ Every name is spelled from the public table alone (`with_public_names_only`), whatever schema
/// snapshot the host installed, so the XML a snapshot would name member by member is `hash_XXXXXXXX`
/// here, and a project compiles the same bytes on a machine with no snapshot.
pub fn extract(
    bytes: &[u8],
    name: &str,
    extension: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>> {
    if !EXTENSIONS.contains(&extension) {
        return Err(vec![unknown_extension(extension)]);
    }
    let xml = with_public_names_only(|| canonical_xml(bytes, extension))?;
    publish(bytes, name, extension, parent, how, xml)
}

/// The XML of `bytes`, when it compiles back to the canonical form with no value lost.
fn canonical_xml(bytes: &[u8], extension: &str) -> Result<String, Vec<Problem>> {
    let empty = SchemaLibrary::default();
    let refused = |why: String| vec![Problem::new(why)];
    if detect_binary_format(bytes).is_none() {
        // Only a `.meta` is the game's own XML text; the others are binary files.
        if extension != "meta" {
            return Err(refused(format!(
                "is not a binary metadata file, and a .{extension} is one"
            )));
        }
        // A text document is its own XML, kept exactly as the game ships it.
        text_document(bytes).map_err(refused)?;
        return String::from_utf8(bytes.to_vec())
            .map_err(|e| refused(format!("is XML that is not UTF-8: {e}")));
    }
    let converged = canonical::converge(&empty, bytes).map_err(|why| refused(why.to_string()))?;
    if converged.values_moved {
        return Err(refused(
            "re-importing its XML loses a value: the second export differs from the first"
                .to_string(),
        ));
    }
    let mut diagnostics = Diagnostics::lenient();
    let (instance, layout) = parse_meta_binary_full(&empty, bytes, &mut diagnostics)
        .map_err(|e| refused(e.to_string()))?;
    let xml = canonical::export(&instance, &layout).map_err(|e| refused(e.to_string()))?;
    // The XML that is kept is the XML that has to compile to the canonical form.
    if compile_bytes(bytes, xml.as_bytes())? != converged.bytes {
        return Err(refused(
            "its XML does not compile to the canonical form of the file".to_string(),
        ));
    }
    Ok(xml)
}

/// The file `xml` compiles to, laid out as `base` is and typed by the type table it holds.
fn compile_bytes(base: &[u8], xml: &[u8]) -> Result<Vec<u8>, Vec<Problem>> {
    with_public_names_only(|| compile_named(base, xml))
}

/// Why `bytes` are not one XML metadata document.
fn text_document(bytes: &[u8]) -> Result<(), String> {
    // ⚠ Read with the weapon family's declaration, which a weapon file's references and malformed numbers need: a document of another root is read without one, as it was.
    let document = MetaDocument::parse(
        &or_io::meta::typed::weapon_types::library(or_io::meta::default_library()),
        bytes.to_vec(),
    )
    .map_err(|e| format!("is no metadata document OpenRage reads: {e}"))?;
    if document.source_format == SourceFormat::Xml {
        Ok(())
    } else {
        Err(format!(
            "is a {} document, which has no XML source yet",
            document.source_format.label()
        ))
    }
}

fn compile_named(base: &[u8], xml: &[u8]) -> Result<Vec<u8>, Vec<Problem>> {
    let refused = |why: String| vec![Problem::new(why)];
    let Some(format) = detect_binary_format(base) else {
        // ⚠ A text document is its XML, and an edit that is no document would ship as the file.
        text_document(xml).map_err(refused)?;
        return Ok(xml.to_vec());
    };
    let empty = SchemaLibrary::default();
    let mut diagnostics = Diagnostics::lenient();
    let (_, layout) = parse_meta_binary_full(&empty, base, &mut diagnostics)
        .map_err(|e| refused(format!("the base does not read: {e}")))?;
    let typed = canonical::typed_by(&empty, Some(&layout));
    canonical::import(&typed, xml, format, None).map_err(|e| refused(e.to_string()))
}

/// Writes the folder, after checking that what it holds compiles.
fn publish(
    base: &[u8],
    name: &str,
    extension: &str,
    parent: &Path,
    how: &Extraction<'_>,
    xml: String,
) -> Result<PathBuf, Vec<Problem>> {
    let origin = Origin {
        how: How::Extracted,
        from_game: how.from_game,
        virtual_path: how.virtual_path.map(str::to_string),
        game_build: None,
        sha256: Some(sha256_hex(base)),
        cooked_sha256: None,
        extra: Default::default(),
    };
    // A metadata file has no RSC7 layout of its own to name, so the default one stands.
    let mut manifest = AssetManifest::new(KIND, name, "gen8", origin).map_err(|p| vec![p])?;
    let compile = Compile {
        extension: extension.to_string(),
        base: sha256_hex(base),
    };
    // Built from a struct of two strings, which always serialises.
    manifest.extra.insert(
        "compile".to_string(),
        serde_json::to_value(&compile).unwrap_or_default(),
    );
    let inputs = Inputs {
        manifest,
        compile,
        base: base.to_vec(),
        xml: xml.into_bytes(),
    };
    let dir = parent.join(folder_name(name, KIND));
    if dir.exists() {
        return Err(vec![Problem::new(format!(
            "{} already exists",
            dir.display()
        ))]);
    }
    // ⚠ A source the compile would refuse is not written.
    inputs.compile()?;
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(MANIFEST), inputs.manifest.to_text()))
        .and_then(|()| std::fs::write(dir.join(base_file(extension)), &inputs.base))
        .and_then(|()| std::fs::write(dir.join(XML), &inputs.xml));
    match written {
        Ok(()) => Ok(dir),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(vec![Problem::new(format!(
                "cannot write {}: {e}",
                dir.display()
            ))])
        }
    }
}

impl Inputs {
    pub fn read(source: &Source) -> Result<Self, Vec<Problem>> {
        let manifest = source.manifest.clone();
        if manifest.kind != KIND {
            return Err(vec![problem(
                format!("{:?} is not a metadata source", manifest.kind),
                MANIFEST,
            )]);
        }
        let compile = compile_of(&manifest).map_err(|p| vec![p])?;
        let base_name = base_file(&compile.extension);
        let base = std::fs::read(source.dir.join(&base_name))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), base_name)])?;
        let xml = std::fs::read(source.dir.join(XML))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), XML)])?;
        Ok(Self {
            manifest,
            compile,
            base,
            xml,
        })
    }

    /// A digest of every input, whatever its formatting.
    pub fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        let mut put = |bytes: &[u8]| {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        };
        put(self.manifest.digest_text().as_bytes());
        put(&self.base);
        put(&self.xml);
        hash.finalize().into()
    }

    /// The file the inputs compile to, read back through the readers before it is returned.
    pub fn compile(&self) -> Result<Compiled, Vec<Problem>> {
        let held = sha256_hex(&self.base);
        if held != self.compile.base {
            return Err(vec![problem(
                format!(
                    "the base holds {held} and compile.base says {}; the base is not the file the XML was exported from",
                    self.compile.base
                ),
                base_file(&self.compile.extension),
            )]);
        }
        let bytes = compile_bytes(&self.base, &self.xml).map_err(|mut found| {
            for p in &mut found {
                p.file = Some(XML.to_string());
            }
            found
        })?;
        // ⚠ A file the readers refuse would be shipped as a file the game misreads.
        if detect_binary_format(&bytes).is_some() {
            let mut diagnostics = Diagnostics::lenient();
            parse_meta_binary_full(&SchemaLibrary::default(), &bytes, &mut diagnostics).map_err(
                |e| {
                    vec![problem(
                        format!("the compiled file does not read back: {e}"),
                        XML,
                    )]
                },
            )?;
        }
        Ok(Compiled {
            file_name: output_name(&self.manifest, &self.compile.extension),
            bytes,
            report: Vec::new(),
        })
    }

    /// The name of the file the inputs compile to.
    pub fn output_name(&self) -> String {
        output_name(&self.manifest, &self.compile.extension)
    }

    /// The game extension of the file the inputs compile to.
    pub fn extension(&self) -> &str {
        &self.compile.extension
    }

    /// The XML the inputs hold.
    pub fn xml(&self) -> &[u8] {
        &self.xml
    }

    /// Hold `xml` as the XML, which nothing has checked.
    pub fn set_xml(&mut self, xml: Vec<u8>) {
        self.xml = xml;
    }

    /// The XML that makes the inputs cook to `bytes`, a file of the kind they hold, refused as [`replace`] refuses it; nothing is written.
    pub fn xml_of(&self, bytes: &[u8]) -> Result<Vec<u8>, Vec<Problem>> {
        let xml = with_public_names_only(|| canonical_xml(bytes, &self.compile.extension))?;
        if xml.as_bytes() == self.xml.as_slice() {
            return Ok(xml.into_bytes());
        }
        let candidate = Self {
            xml: xml.clone().into_bytes(),
            ..self.clone()
        };
        let compiled = candidate.compile()?;
        let again =
            with_public_names_only(|| canonical_xml(&compiled.bytes, &self.compile.extension))?;
        if again != xml {
            return Err(vec![Problem::new(
                "the edit changes the shape of the file: its XML does not compile back to itself through the type table of the base",
            )]);
        }
        Ok(xml.into_bytes())
    }

    /// Whether the XML held is the XML that `bytes`, a file of the kind the inputs hold, exports as: XML a person typed is often not.
    pub fn is_the_export_of(&self, bytes: &[u8]) -> bool {
        if detect_binary_format(bytes).is_none() {
            return bytes == self.xml.as_slice();
        }
        let exported = with_public_names_only(|| {
            let mut diagnostics = Diagnostics::lenient();
            let (instance, layout) =
                parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut diagnostics).ok()?;
            canonical::export(&instance, &layout).ok()
        });
        exported.is_some_and(|xml| xml.as_bytes() == self.xml.as_slice())
    }
}

/// The `compile` block of `manifest`, refused when it names an extension a metadata source does not hold.
fn compile_of(manifest: &AssetManifest) -> Result<Compile, Problem> {
    let stated = manifest.extra.get("compile").ok_or_else(|| {
        problem(
            "a metadata source states what it compiles, and there is no `compile` block",
            MANIFEST,
        )
    })?;
    let compile: Compile = serde_json::from_value(stated.clone())
        .map_err(|e| problem(format!("compile: {e}"), MANIFEST))?;
    // The extension is a part of a file name that is read, and a source is a file anyone can edit.
    if EXTENSIONS.contains(&compile.extension.as_str()) {
        Ok(compile)
    } else {
        Err(unknown_extension(&compile.extension))
    }
}

/// The name of the file a metadata source compiles to.
fn output_name(manifest: &AssetManifest, extension: &str) -> String {
    format!("{}.{extension}", manifest.name)
}

/// The name of the file the source `manifest` describes compiles to.
pub fn cooked_name(manifest: &AssetManifest) -> Result<String, Problem> {
    compile_of(manifest).map(|compile| output_name(manifest, &compile.extension))
}

/// Whether the file the source in `dir` compiles to is not the original's bytes, which is what the first export of a game file does.
pub fn differs_from_the_original(dir: &Path) -> Option<bool> {
    let source = Source::open(dir).ok()?;
    let original = source.manifest.origin.sha256.clone()?;
    let compiled = Inputs::read(&source).ok()?.compile().ok()?;
    Some(sha256_hex(&compiled.bytes) != original)
}

/// What the folder `source` names compiles to.
pub fn compile(source: &Source) -> Result<Compiled, Vec<Problem>> {
    Inputs::read(source)?.compile()
}

/// What replacing the XML of a metadata source changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    /// The XML file of the source.
    pub file: PathBuf,
    /// The XML it held, and the XML it holds now; equal when the file given was the one the source already cooked to.
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

impl Replaced {
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
}

/// Make the source in `dir` cook to `bytes`, a file of the kind it holds, by writing the canonical XML of `bytes` as its XML.
///
/// ⚠ Nothing is written unless the XML compiles through the type table of the source's base to a file that exports to that same XML:
/// an edit that changes the shape of the file (a class its base does not name) is refused with the reason, and the source stays as it
/// was. The XML is rewritten whole, so a name a person typed into it that the index of names lacks is spelled `hash_XXXXXXXX` after,
/// which compiles to the same hash.
pub fn replace(dir: &Path, bytes: &[u8]) -> Result<Replaced, Vec<Problem>> {
    let source = Source::open(dir).map_err(|p| vec![p])?;
    let inputs = Inputs::read(&source)?;
    let xml = inputs.xml_of(bytes)?;
    if xml != inputs.xml {
        write_xml(dir, &xml)?;
    }
    Ok(Replaced {
        file: dir.join(XML),
        before: inputs.xml,
        after: xml,
    })
}

/// Write `xml` as the XML of the metadata source in `dir`, to a part file first, so a failure leaves the XML that was there.
pub fn write_xml(dir: &Path, xml: &[u8]) -> Result<(), Vec<Problem>> {
    let file = dir.join(XML);
    let held = dir.join(format!("{XML}.part"));
    std::fs::write(&held, xml)
        .and_then(|()| std::fs::rename(&held, &file))
        .map_err(|e| {
            let _ = std::fs::remove_file(&held);
            vec![Problem::new(format!(
                "cannot write {}: {e}",
                file.display()
            ))]
        })
}

/// Whether a path inside a metadata source, relative to its folder, is a file the compile reads.
pub fn is_input(inside: &Path) -> bool {
    let parts: Vec<&str> = inside
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [] => true,
        [file] => {
            *file == MANIFEST || *file == XML || EXTENSIONS.iter().any(|e| *file == base_file(e))
        }
        _ => false,
    }
}
