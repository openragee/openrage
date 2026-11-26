//! Recooking the one asset a saved file belongs to, into a shadow folder, as a function a watcher calls.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::asset::{sha256_hex, AssetManifest, Source, MANIFEST};
use crate::bounds;
use crate::fragment;
use crate::metadata;
use crate::models;
use crate::particles;
use crate::paths;
use crate::textures::{self, Inputs};
use crate::Problem;

/// Cooks sources into `<shadow>/<asset id>/<name>.<ext>`, a folder of its own for each asset.
pub struct Cooker {
    shadow: PathBuf,
    /// The digest of the inputs each source was last cooked from.
    cooked: HashMap<PathBuf, [u8; 32]>,
    /// The source whose output holds each id's slot of the shadow, so that a copied folder cannot overwrite it.
    owners: HashMap<String, PathBuf>,
}

/// Why a changed path started no cook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// It is in the shadow folder, which only the cook writes.
    OwnWrite,
    /// No folder above it is a source.
    OutsideSource,
    /// It is in a source and is not a file the cook reads.
    NotAnInput,
}

/// What a changed path did.
#[derive(Debug, Clone, PartialEq)]
pub enum Recook {
    Ignored(Ignored),
    /// The source's inputs are those it was last cooked from, and its output is there.
    Unchanged {
        source: PathBuf,
    },
    Cooked {
        source: PathBuf,
        output: PathBuf,
    },
    /// The previous output, if there was one, is as it was.
    Failed {
        source: PathBuf,
        problems: Vec<Problem>,
    },
}

impl Cooker {
    pub fn new(shadow: impl Into<PathBuf>) -> Self {
        Self {
            shadow: shadow.into(),
            cooked: HashMap::new(),
            owners: HashMap::new(),
        }
    }

    /// Cooks the source `changed` is inside, and no other. Paths are compared as given.
    pub fn recook(&mut self, changed: &Path) -> Recook {
        // The shadow folder is the cook's own wherever it sits under a watched folder.
        if changed.starts_with(&self.shadow) {
            return Recook::Ignored(Ignored::OwnWrite);
        }
        let Some(dir) = changed.ancestors().find(|d| d.join(MANIFEST).is_file()) else {
            return Recook::Ignored(Ignored::OutsideSource);
        };
        if !changed.strip_prefix(dir).is_ok_and(is_input) {
            return Recook::Ignored(Ignored::NotAnInput);
        }

        let failed = |problems| Recook::Failed {
            source: dir.to_path_buf(),
            problems,
        };
        let source = match Source::open(dir) {
            Ok(source) => source,
            Err(problem) => return failed(vec![problem]),
        };
        // A folder copied in a file manager keeps its id, and its cook would land in the original's slot.
        let id = &source.manifest.id;
        if let Some(owner) = self.owners.get(id) {
            if owner != dir && holds_id(owner, id) {
                return failed(vec![Problem::new(format!(
                    "has the id {id}, which {} has too, and a cook would overwrite that source's output; \
                     give one of them a new id in {MANIFEST}",
                    owner.display()
                ))
                .in_file(MANIFEST)]);
            }
        }
        let inputs = match Read::of(&source) {
            Ok(inputs) => inputs,
            Err(problems) => return failed(problems),
        };
        let digest = inputs.digest();
        let output = match output_in_shadow(
            &self.shadow,
            &source.manifest.id,
            &inputs.output_name(&source.manifest),
        ) {
            Ok(output) => output,
            Err(problem) => return failed(vec![problem]),
        };
        if self.cooked.get(dir) == Some(&digest) && output.is_file() {
            return Recook::Unchanged {
                source: dir.to_path_buf(),
            };
        }
        let bytes = match inputs.compile() {
            Ok(bytes) => bytes,
            Err(problems) => return failed(problems),
        };
        if let Err(e) = publish(&output, &bytes) {
            return failed(vec![Problem::new(format!(
                "cannot write {}: {e}",
                output.display()
            ))]);
        }
        self.cooked.insert(dir.to_path_buf(), digest);
        self.owners
            .insert(source.manifest.id.clone(), dir.to_path_buf());
        Recook::Cooked {
            source: dir.to_path_buf(),
            output,
        }
    }
}

/// What a build needs to know of a source folder before it cooks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Described {
    /// The word that chooses the compiler, such as `textures`.
    pub kind: String,
    /// The asset's own name, which names the cooked file.
    pub name: String,
    /// The game extension of the cooked file, such as `ytd`.
    pub extension: String,
    /// The size of every file the cook reads, which is what the cooked file is made from.
    pub bytes: u64,
    /// The SHA-256 of the game's own file this source was taken from and of what its import cooked it to, either of which a build takes for the game's file; empty for any other source.
    pub game: Vec<String>,
}

/// What the source in `dir` cooks to, or the one sentence that says why this build cannot cook it.
pub fn describe(dir: &Path) -> Result<Described, String> {
    let source = Source::open(dir).map_err(|p| p.message)?;
    let manifest = &source.manifest;
    let output = match manifest.kind.as_str() {
        textures::KIND => textures::output_name(manifest),
        fragment::KIND => fragment::output_name(manifest),
        metadata::KIND => metadata::cooked_name(manifest).map_err(|p| p.message)?,
        particles::KIND => particles::output_name(manifest),
        bounds::KIND => bounds::output_name(manifest),
        paths::KIND => paths::output_name(manifest),
        models::DRAWABLE | models::DRAWABLES => models::output_name(manifest),
        kind => return Err(refused_kind(kind)),
    };
    if manifest.platform != "win64" {
        return Err(format!(
            "its platform is {}, and this build cooks win64 only",
            manifest.platform
        ));
    }
    let extension = Path::new(&output)
        .extension()
        .map_or_else(String::new, |e| e.to_string_lossy().into_owned());
    let mut game: Vec<String> = Vec::new();
    if manifest.origin.from_game {
        let origin = &manifest.origin;
        game.extend(origin.sha256.iter().chain(&origin.cooked_sha256).cloned());
        game.dedup();
    }
    Ok(Described {
        kind: manifest.kind.clone(),
        name: manifest.name.clone(),
        extension,
        bytes: inputs_size(dir, Path::new("")),
        game,
    })
}

/// Cooks the source in `dir` into `<scratch>/<id>/<name>.<extension>`, as a recook does, and says where the file is.
///
/// ⚠ Each call is a cook of its own: nothing is remembered between two, so a source is always cooked from what is in the folder now.
pub fn cook_source(dir: &Path, scratch: &Path) -> Result<PathBuf, Vec<String>> {
    match Cooker::new(scratch).recook(&dir.join(MANIFEST)) {
        Recook::Cooked { output, .. } => Ok(output),
        Recook::Failed { problems, .. } => Err(problems.iter().map(ToString::to_string).collect()),
        Recook::Unchanged { .. } | Recook::Ignored(_) => {
            Err(vec!["the cook made no file".to_string()])
        }
    }
}

/// The SHA-256 of what the cook of the source in `dir` makes, or the sentences that say why it makes nothing.
pub fn cooked_sha256(dir: &Path) -> Result<String, Vec<String>> {
    let strings = |problems: Vec<Problem>| -> Vec<String> {
        problems.iter().map(ToString::to_string).collect()
    };
    let source = Source::open(dir).map_err(|p| vec![p.to_string()])?;
    let bytes = Read::of(&source)
        .and_then(|inputs| inputs.compile())
        .map_err(strings)?;
    Ok(sha256_hex(&bytes))
}

/// The bytes of every file under `dir/inside` that the cook reads.
fn inputs_size(dir: &Path, inside: &Path) -> u64 {
    std::fs::read_dir(dir.join(inside))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            let at = inside.join(entry.file_name());
            match entry.metadata() {
                Ok(meta) if meta.is_dir() && at.components().count() < 3 => inputs_size(dir, &at),
                Ok(meta) if meta.is_file() && is_input(&at) => meta.len(),
                _ => 0,
            }
        })
        .sum()
}

/// Whether `dir` is still a source that states `id`: a folder that was deleted or given a new id holds no slot.
fn holds_id(dir: &Path, id: &str) -> bool {
    Source::open(dir).is_ok_and(|source| source.manifest.id == id)
}

fn refused_kind(kind: &str) -> String {
    format!(
        "this build cooks {}, {}, {}, {}, {}, {}, {} and {} sources, and this is a {kind} source",
        textures::KIND,
        models::DRAWABLE,
        models::DRAWABLES,
        fragment::KIND,
        metadata::KIND,
        particles::KIND,
        bounds::KIND,
        paths::KIND
    )
}

/// What a source of a kind this build cooks reads.
enum Read {
    Textures(Inputs),
    Models(Box<models::Inputs>),
    Fragment(Box<fragment::Inputs>),
    Metadata(metadata::Inputs),
    Particles(Box<particles::Inputs>),
    Bounds(Box<bounds::Inputs>),
    Paths(Box<paths::Inputs>),
}

impl Read {
    fn of(source: &Source) -> Result<Self, Vec<Problem>> {
        match source.manifest.kind.as_str() {
            textures::KIND => Inputs::read(source).map(Self::Textures),
            fragment::KIND => fragment::Inputs::read(source).map(|i| Self::Fragment(Box::new(i))),
            metadata::KIND => metadata::Inputs::read(source).map(Self::Metadata),
            particles::KIND => {
                particles::Inputs::read(source).map(|inputs| Self::Particles(Box::new(inputs)))
            }
            bounds::KIND => bounds::Inputs::read(source).map(|i| Self::Bounds(Box::new(i))),
            paths::KIND => paths::Inputs::read(source).map(|i| Self::Paths(Box::new(i))),
            models::DRAWABLE | models::DRAWABLES => {
                models::Inputs::read(source).map(|i| Self::Models(Box::new(i)))
            }
            kind => Err(vec![Problem::new(refused_kind(kind)).in_file(MANIFEST)]),
        }
    }

    fn digest(&self) -> [u8; 32] {
        match self {
            Self::Textures(inputs) => inputs.digest(),
            Self::Models(inputs) => inputs.digest(),
            Self::Fragment(inputs) => inputs.digest(),
            Self::Metadata(inputs) => inputs.digest(),
            Self::Particles(inputs) => inputs.digest(),
            Self::Bounds(inputs) => inputs.digest(),
            Self::Paths(inputs) => inputs.digest(),
        }
    }

    fn output_name(&self, manifest: &AssetManifest) -> String {
        match self {
            Self::Textures(_) => textures::output_name(manifest),
            Self::Models(_) => models::output_name(manifest),
            Self::Fragment(_) => fragment::output_name(manifest),
            Self::Metadata(inputs) => inputs.output_name(),
            Self::Particles(inputs) => inputs.output_name(),
            Self::Bounds(inputs) => inputs.output_name(),
            Self::Paths(inputs) => inputs.output_name(),
        }
    }

    fn compile(&self) -> Result<Vec<u8>, Vec<Problem>> {
        match self {
            Self::Textures(inputs) => inputs.compile().map(|c| c.bytes),
            Self::Models(inputs) => inputs.compile().map(|c| c.bytes),
            Self::Fragment(inputs) => inputs.compile().map(|c| c.bytes),
            Self::Metadata(inputs) => inputs.compile().map(|c| c.bytes),
            Self::Particles(inputs) => inputs.compile().map(|c| c.bytes),
            Self::Bounds(inputs) => inputs.compile().map(|c| c.bytes),
            Self::Paths(inputs) => inputs.compile().map(|c| c.bytes),
        }
    }
}

/// Whether a path inside a source, relative to its folder, is a file a cook reads.
fn is_input(inside: &Path) -> bool {
    textures_input(inside)
        || models::is_input(inside)
        || fragment::is_input(inside)
        || metadata::is_input(inside)
        || particles::is_input(inside)
        || bounds::is_input(inside)
        || paths::is_input(inside)
}

/// Whether a path inside a textures source, relative to its folder, is a file the cook reads.
fn textures_input(inside: &Path) -> bool {
    let parts: Vec<&str> = inside
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [] => true,
        [file] => *file == MANIFEST || *file == textures::RULES,
        [dir, file] => *dir == textures::DIR && textures::is_texture_file(file),
        _ => false,
    }
}

/// Puts `bytes` at `output` in one step, so a reader sees the old file or the new one and a failure leaves the old one.
fn publish(output: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temporary = output.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let written =
        std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, output));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// `<shadow>/<id>/<file>`, refused when `id` or `file` is not one plain name, so no manifest chooses where the cook writes.
fn output_in_shadow(shadow: &Path, id: &str, file: &str) -> Result<PathBuf, Problem> {
    let one_name = |part: &str| {
        matches!(
            Path::new(part).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
    };
    if one_name(id) && one_name(file) {
        return Ok(shadow.join(id).join(file));
    }
    Err(Problem::new(format!(
        "the shadow folder holds <id>/<file>, and {id:?}/{file:?} is not one folder and one file in it"
    ))
    .in_file(MANIFEST))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_is_one_file_in_one_folder_of_the_shadow() {
        let shadow = Path::new("shadow");
        let ok = output_in_shadow(shadow, "7c1e2b0a", "a.ytd").unwrap();
        assert_eq!(ok, shadow.join("7c1e2b0a").join("a.ytd"));

        let held = tempfile::tempdir().unwrap();
        let absolute = held.path().to_string_lossy().into_owned();
        for (id, file) in [
            ("..", "a.ytd"),
            (".", "a.ytd"),
            ("", "a.ytd"),
            ("a/b", "a.ytd"),
            ("../../x", "a.ytd"),
            (absolute.as_str(), "a.ytd"),
            ("id", ".."),
            ("id", ""),
            ("id", "../a.ytd"),
            ("id", "a/b.ytd"),
            ("id", absolute.as_str()),
        ] {
            let problem = output_in_shadow(shadow, id, file).unwrap_err();
            assert!(
                problem.message.contains("shadow"),
                "{id:?} {file:?}: {problem}"
            );
        }
    }
}
