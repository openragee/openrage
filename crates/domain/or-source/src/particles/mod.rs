//! The source of a `.ypt`: its effects, emitters and particles as one document of rules, its textures as exact DDS files, and the
//! models its particles draw as a list of their own.
//!
//! ```text
//! <name>.particles.src/   asset.json  particles.json
//!                         textures.json  textures/<texture>.dds     (when the list holds textures)
//!                         models.ypt                                (when its particles draw models)
//! ```
//!
//! `particles.json` states every member the declaration of a list shows, by its names (see [`records`]), and the textures are the
//! source of a `.ytd` ([`crate::textures`]), so a texture is edited as a DDS or an image beside it and compiled with the rules that
//! source states. The models a list draws are kept whole in `models.ypt`, a particle list that holds only them, because the
//! drawable source of [`crate::models`] is for the drawable of a `.ydr` and a `.ydd`, and not for the model of a particle.
//!
//! ⚠ **An extraction compiles what it wrote and compares it with the file.** A list the text cannot hold is an error that names
//! the first rule it differs in, and nothing is left behind: a file is never written as a source that loses part of it.

mod list;
mod records;
pub(crate) mod text;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use or_io::decl::{File, Gen8, Gen9};
use or_io::gta5::drawable::DrawableTarget;
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::gen9::GrcTextureG9;
use or_io::gta5::particle::PtxFxList;
use or_io::gta5::PgDictionary;
use or_io::Generation;
use sha2::{Digest, Sha256};

use crate::asset::{folder_name, sha256_hex, AssetManifest, How, Origin, Source, MANIFEST};
use crate::json::canonical;
use crate::textures::{self, Extraction};
use crate::Problem;

/// The `kind` of a particles source.
pub const KIND: &str = "particles";
/// The document of rules a particles source always holds.
pub const RULES: &str = "particles.json";
/// The list that holds the models the particles draw, when they draw any.
pub const MODELS: &str = "models.ypt";

/// A target of a list, with the two things that differ between them: the dictionary of textures a list holds.
pub(crate) trait Flavour: DrawableTarget + Sized + std::fmt::Debug + 'static
where
    PtxFxList<Self>: File + PartialEq + Clone,
{
    const GENERATION: Generation;

    /// The `.ytd` the list's texture dictionary is, none where it holds no texture.
    fn textures(list: &PtxFxList<Self>) -> Result<Option<Vec<u8>>, String>;

    /// A list that holds only the models of `list`.
    fn models_only(list: &PtxFxList<Self>) -> PtxFxList<Self>;

    /// The list with the textures of `ytd` for its dictionary.
    fn put_textures(list: &mut PtxFxList<Self>, ytd: &[u8]) -> Result<(), String>;
}

macro_rules! flavour {
    ($target:ty, $generation:expr, $texture:ty) => {
        impl Flavour for $target {
            const GENERATION: Generation = $generation;

            fn textures(list: &PtxFxList<Self>) -> Result<Option<Vec<u8>>, String> {
                match list.texture_dict.as_deref() {
                    Some(dict) if !dict.entries.is_empty() => {
                        dict.to_bytes().map(Some).map_err(|e| {
                            format!("its textures cannot be written as a texture dictionary: {e}")
                        })
                    }
                    _ => Ok(None),
                }
            }

            fn models_only(list: &PtxFxList<Self>) -> PtxFxList<Self> {
                PtxFxList {
                    model_dict: list.model_dict.clone(),
                    ..Default::default()
                }
            }

            fn put_textures(list: &mut PtxFxList<Self>, ytd: &[u8]) -> Result<(), String> {
                let dict = PgDictionary::<$texture>::from_bytes(ytd)
                    .map_err(|e| format!("the compiled textures do not read back: {e}"))?;
                list.texture_dict = Some(Box::new(dict));
                Ok(())
            }
        }
    };
}

flavour!(Gen8, Generation::Gen8, GrcTexture);
flavour!(Gen9, Generation::Gen9, GrcTextureG9);

/// The name of the `.ypt` a source compiles to.
pub fn output_name(manifest: &AssetManifest) -> String {
    format!("{}.ypt", manifest.name)
}

/// Whether a path inside a particles source, relative to its folder, is a file the compile reads.
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
        [file] => [MANIFEST, RULES, textures::RULES, MODELS].contains(file),
        [dir, file] => *dir == textures::DIR && textures::is_texture_file(file),
        _ => false,
    }
}

/// Writes `ypt` into `parent` as `<name>.particles.src` and returns the folder; nothing is left behind on a refusal.
pub fn extract(
    ypt: &[u8],
    name: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>> {
    let target = crate::models::target_of(ypt, "ypt")?;
    match target {
        Generation::Gen9 => extract_as::<Gen9>(ypt, name, parent, how),
        _ => extract_as::<Gen8>(ypt, name, parent, how),
    }
}

fn extract_as<T: Flavour>(
    ypt: &[u8],
    name: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>>
where
    PtxFxList<T>: File + PartialEq + Clone,
{
    let mut list = PtxFxList::<T>::from_bytes(ypt)
        .map_err(|e| vec![Problem::new(format!("cannot read the .ypt: {e}"))])?;
    let refusals = list::refusals(&list);
    if !refusals.is_empty() {
        return Err(refusals.into_iter().map(Problem::new).collect());
    }
    or_particle::list::canonical(&mut list);

    let origin = Origin {
        how: How::Extracted,
        from_game: how.from_game,
        virtual_path: how.virtual_path.map(str::to_string),
        game_build: None,
        sha256: Some(sha256_hex(ypt)),
        cooked_sha256: None,
        extra: BTreeMap::new(),
    };
    let manifest =
        AssetManifest::new(KIND, name, T::GENERATION.slug(), origin).map_err(|p| vec![p])?;
    let dir = parent.join(folder_name(name, KIND));
    if dir.exists() {
        return Err(vec![Problem::new(format!(
            "{} already exists",
            dir.display()
        ))]);
    }
    let made = write_source::<T>(&list, &manifest, name, &dir, how, parent)
        .and_then(|()| same_after_compile::<T>(&dir, &list));
    if let Err(problems) = made {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(problems);
    }
    Ok(dir)
}

/// The files of the source of `list`, written into the new folder `dir`.
fn write_source<T: Flavour>(
    list: &PtxFxList<T>,
    manifest: &AssetManifest,
    name: &str,
    dir: &Path,
    how: &Extraction<'_>,
    parent: &Path,
) -> Result<(), Vec<Problem>>
where
    PtxFxList<T>: File + PartialEq + Clone,
{
    let cannot = |what: &Path, e: std::io::Error| {
        vec![Problem::new(format!(
            "cannot write {}: {e}",
            what.display()
        ))]
    };
    std::fs::create_dir_all(dir).map_err(|e| cannot(dir, e))?;
    let rules = canonical(&list::write(list))
        .map_err(|e| vec![Problem::new(e.to_string()).in_file(RULES)])?;
    for (file, text) in [(MANIFEST, manifest.to_text()), (RULES, rules)] {
        std::fs::write(dir.join(file), text).map_err(|e| cannot(&dir.join(file), e))?;
    }

    if let Some(ytd) = T::textures(list).map_err(|why| vec![Problem::new(why)])? {
        // The textures source of the dictionary is made in a folder of its own and its files moved in, so the rules are the ones
        // a `.ytd` states.
        let scratch = parent.join(format!(".{name}.textures.new"));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).map_err(|e| cannot(&scratch, e))?;
        let moved = textures::extract(&ytd, name, &scratch, how).and_then(|made| {
            std::fs::rename(made.join(textures::DIR), dir.join(textures::DIR))
                .and_then(|()| {
                    std::fs::rename(made.join(textures::RULES), dir.join(textures::RULES))
                })
                .map_err(|e| cannot(&made, e))
        });
        let _ = std::fs::remove_dir_all(&scratch);
        moved?;
    }

    if list
        .model_dict
        .as_deref()
        .is_some_and(|d| !d.entries.is_empty())
    {
        let bytes = T::models_only(list).to_bytes().map_err(|e| {
            vec![Problem::new(format!("its models cannot be written: {e}")).in_file(MODELS)]
        })?;
        std::fs::write(dir.join(MODELS), bytes).map_err(|e| cannot(&dir.join(MODELS), e))?;
    }
    Ok(())
}

/// Compiles the source in `dir` and checks the list it compiles to is `list`, rule for rule.
fn same_after_compile<T: Flavour>(dir: &Path, list: &PtxFxList<T>) -> Result<(), Vec<Problem>>
where
    PtxFxList<T>: File + PartialEq + Clone,
{
    let source = Source::open(dir).map_err(|p| vec![p])?;
    let compiled = Inputs::read(&source)?.compile()?;
    let back = PtxFxList::<T>::from_bytes(&compiled.bytes).map_err(|e| {
        vec![Problem::new(format!(
            "the compiled list does not read: {e}"
        ))]
    })?;
    let mut back = back;
    or_particle::list::canonical(&mut back);
    match first_difference(list, &back) {
        None => Ok(()),
        Some(what) => Err(vec![Problem::new(format!(
            "cannot be written as text that compiles back to the same list: {what}"
        ))]),
    }
}

/// What the first difference of two lists is, none where they are the same.
///
/// The rules are compared as the documents they are written as, so a member the declaration holds only as a runtime word is not
/// compared; the textures as the DDS and header words a source holds; the models as the file that keeps them.
fn first_difference<T: Flavour>(a: &PtxFxList<T>, b: &PtxFxList<T>) -> Option<String>
where
    PtxFxList<T>: File + PartialEq + Clone,
{
    let (x, y) = (list::write(a), list::write(b));
    if let Some(at) = json_difference(&x, &y, "") {
        return Some(at);
    }
    let (x, y) = (T::textures(a), T::textures(b));
    match (&x, &y) {
        (Ok(x), Ok(y)) => {
            let parts = |ytd: &Option<Vec<u8>>| ytd.as_deref().map(textures::parts_of).transpose();
            if parts(x).ok() != parts(y).ok() {
                return Some("its textures compile to others".to_string());
            }
        }
        _ => return Some("its textures cannot be written as a texture dictionary".to_string()),
    }
    let models = |list: &PtxFxList<T>| {
        list.model_dict
            .as_deref()
            .filter(|d| !d.entries.is_empty())
            .map(|_| T::models_only(list).to_bytes().ok())
    };
    (models(a) != models(b)).then(|| "its models compile to others".to_string())
}

/// Where two documents first differ, as the place in them and what each says there; the rules are named, not numbered.
fn json_difference(a: &serde_json::Value, b: &serde_json::Value, at: &str) -> Option<String> {
    use serde_json::Value;
    let said = |v: &Value| {
        let text = v.to_string();
        match text.char_indices().nth(60) {
            Some((i, _)) => format!("{}…", &text[..i]),
            None => text,
        }
    };
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys()) {
                let place = if at.is_empty() {
                    key.clone()
                } else {
                    format!("{at}.{key}")
                };
                match (x.get(key), y.get(key)) {
                    (Some(p), Some(q)) => {
                        if let Some(found) = json_difference(p, q, &place) {
                            return Some(found);
                        }
                    }
                    _ => return Some(format!("{place} is stated by one and not the other")),
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!(
                    "{at} lists {} and compiles to {}",
                    x.len(),
                    y.len()
                ));
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                // A rule is named by its name, which a position in a list says less of.
                let name = p.get("name").and_then(Value::as_str);
                let place = match name {
                    Some(name) => format!("{at}[`{name}`]"),
                    None => format!("{at}[{i}]"),
                };
                if let Some(found) = json_difference(p, q, &place) {
                    return Some(found);
                }
            }
            None
        }
        _ if a == b => None,
        _ => Some(format!("{at} says {} and compiles to {}", said(a), said(b))),
    }
}
/// Everything a compile reads, so a recook can tell nothing changed before it compiles anything.
pub struct Inputs {
    manifest: AssetManifest,
    rules: String,
    textures: Option<textures::Inputs>,
    models: Option<Vec<u8>>,
}

impl Inputs {
    pub fn read(source: &Source) -> Result<Self, Vec<Problem>> {
        let rules = std::fs::read_to_string(source.dir.join(RULES))
            .map_err(|e| vec![Problem::new(format!("cannot read: {e}")).in_file(RULES)])?;
        let textures = source
            .dir
            .join(textures::RULES)
            .is_file()
            .then(|| textures::Inputs::read(source))
            .transpose()?;
        let models = match std::fs::read(source.dir.join(MODELS)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(vec![
                    Problem::new(format!("cannot read: {e}")).in_file(MODELS)
                ])
            }
        };
        Ok(Self {
            manifest: source.manifest.clone(),
            rules,
            textures,
            models,
        })
    }

    pub fn output_name(&self) -> String {
        output_name(&self.manifest)
    }

    /// A digest of every input, whatever its formatting or its place.
    pub fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(self.manifest.digest_text());
        // The rules as a value, so a reformatted document is the same input.
        let rules = serde_json::from_str::<serde_json::Value>(&self.rules).unwrap_or_default();
        hash.update(canonical(&rules).unwrap_or_default());
        hash.update(
            self.textures
                .as_ref()
                .map_or([0; 32], textures::Inputs::digest),
        );
        if let Some(models) = &self.models {
            hash.update((models.len() as u64).to_le_bytes());
            hash.update(models);
        }
        hash.finalize().into()
    }

    /// The `.ypt` the inputs compile to, reloaded through the reader before it is returned.
    pub fn compile(&self) -> Result<textures::Compiled, Vec<Problem>> {
        if self.manifest.platform != "win64" {
            return Err(vec![Problem::new(format!(
                "platform {:?} is not one this build compiles for; only win64 is",
                self.manifest.platform
            ))
            .in_file(MANIFEST)]);
        }
        let bytes = match self.manifest.target.as_str() {
            t if t == Generation::Gen8.slug() => self.compile_as::<Gen8>()?,
            t if t == Generation::Gen9.slug() => self.compile_as::<Gen9>()?,
            other => {
                return Err(vec![Problem::new(format!(
                    "target {other:?} is neither Legacy nor Enhanced"
                ))
                .in_file(MANIFEST)])
            }
        };
        Ok(textures::Compiled {
            file_name: self.output_name(),
            bytes,
        })
    }

    fn compile_as<T: Flavour>(&self) -> Result<Vec<u8>, Vec<Problem>>
    where
        PtxFxList<T>: File + PartialEq + Clone,
    {
        let document: serde_json::Value = serde_json::from_str(&self.rules)
            .map_err(|e| vec![Problem::new(e.to_string()).in_file(RULES)])?;
        let mut list = list::read::<T>(&document)
            .map_err(|fault| vec![Problem::new(fault.to_string()).in_file(RULES)])?;
        if let Some(textures) = &self.textures {
            let compiled = textures.compile()?;
            T::put_textures(&mut list, &compiled.bytes)
                .map_err(|why| vec![Problem::new(why).in_file(textures::RULES)])?;
        }
        if let Some(models) = &self.models {
            let held = PtxFxList::<T>::from_bytes(models).map_err(|e| {
                vec![
                    Problem::new(format!("is not a particle list of this target: {e}"))
                        .in_file(MODELS),
                ]
            })?;
            list.model_dict = held.model_dict;
        }
        let bytes = list
            .to_bytes()
            .map_err(|e| vec![Problem::new(format!("the list cannot be written: {e}"))])?;
        // ⚠ What is written is read again before it is handed on, as every compile does.
        PtxFxList::<T>::from_bytes(&bytes).map_err(|e| {
            vec![Problem::new(format!(
                "the compiled list does not read: {e}"
            ))]
        })?;
        Ok(bytes)
    }
}

/// What the folder `source` names compiles to.
pub fn compile(source: &Source) -> Result<textures::Compiled, Vec<Problem>> {
    Inputs::read(source)?.compile()
}

/// Puts `ypt`, a list opened from what the source in `dir` compiles to and changed since, back into the source.
///
/// The rules are written again; the textures and the models only when the edited list's differ from the ones the source compiles to,
/// so a texture a person put beside its DDS keeps its rules. `asset.json` stays as it is, with its id and its origin. Nothing is
/// written when the list is what the source already compiles to.
pub fn write_back(dir: &Path, ypt: &[u8]) -> Result<(), Vec<Problem>> {
    let source = Source::open(dir).map_err(|p| vec![p])?;
    if source.manifest.kind != KIND {
        return Err(vec![Problem::new(format!(
            "{} is not a particles source",
            dir.display()
        ))]);
    }
    let target = crate::models::target_of(ypt, "ypt")?;
    if target.slug() != source.manifest.target {
        let compiles_to = Generation::from_slug(&source.manifest.target).map_or_else(
            || source.manifest.target.clone(),
            |g| g.edition_label().to_string(),
        );
        return Err(vec![Problem::new(format!(
            "the list is laid out for {} and the source compiles to {compiles_to}",
            target.edition_label()
        ))]);
    }
    match target {
        Generation::Gen9 => write_back_as::<Gen9>(&source, ypt),
        _ => write_back_as::<Gen8>(&source, ypt),
    }
}

fn write_back_as<T: Flavour>(source: &Source, ypt: &[u8]) -> Result<(), Vec<Problem>>
where
    PtxFxList<T>: File + PartialEq + Clone,
{
    let before = PtxFxList::<T>::from_bytes(&Inputs::read(source)?.compile()?.bytes)
        .map_err(|e| vec![Problem::new(format!("the source does not compile: {e}"))])?;
    let mut after = PtxFxList::<T>::from_bytes(ypt)
        .map_err(|e| vec![Problem::new(format!("cannot read the list: {e}"))])?;
    or_particle::list::canonical(&mut after);
    let mut before = before;
    or_particle::list::canonical(&mut before);
    if first_difference(&before, &after).is_none() {
        return Ok(());
    }
    let kept = |a: Option<Vec<u8>>, b: Option<Vec<u8>>| -> bool {
        let parts = |ytd: &Option<Vec<u8>>| ytd.as_deref().map(textures::parts_of).transpose().ok();
        parts(&a) == parts(&b)
    };
    let same_textures = match (T::textures(&before), T::textures(&after)) {
        (Ok(a), Ok(b)) => kept(a, b),
        _ => false,
    };
    let models = |list: &PtxFxList<T>| {
        list.model_dict
            .as_deref()
            .filter(|d| !d.entries.is_empty())
            .and_then(|_| T::models_only(list).to_bytes().ok())
    };
    let same_models = models(&before) == models(&after);

    let parent = source.dir.parent().unwrap_or(Path::new("."));
    let scratch = parent.join(format!(".{}.write-back", source.manifest.id));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| {
        vec![Problem::new(format!(
            "cannot write {}: {e}",
            scratch.display()
        ))]
    })?;
    let origin = &source.manifest.origin;
    let made = extract(
        ypt,
        &source.manifest.name,
        &scratch,
        &Extraction {
            from_game: origin.from_game,
            virtual_path: origin.virtual_path.as_deref(),
        },
    );
    let adopted = made.and_then(|fresh| {
        let cannot = |what: &Path, e: std::io::Error| {
            vec![Problem::new(format!(
                "cannot write {}: {e}",
                what.display()
            ))]
        };
        let mut moved: Vec<&str> = vec![RULES];
        if !same_textures {
            moved.extend([textures::RULES, textures::DIR]);
        }
        if !same_models {
            moved.push(MODELS);
        }
        for name in moved {
            let (from, to) = (fresh.join(name), source.dir.join(name));
            if to.is_dir() {
                std::fs::remove_dir_all(&to).map_err(|e| cannot(&to, e))?;
            }
            if from.exists() {
                std::fs::rename(&from, &to).map_err(|e| cannot(&to, e))?;
            } else if to.is_file() {
                // The edited list has none of what the source held.
                std::fs::remove_file(&to).map_err(|e| cannot(&to, e))?;
            }
        }
        Ok(())
    });
    let _ = std::fs::remove_dir_all(&scratch);
    adopted
}
