//! The source of a `.ydr` or a `.ydd`: a folder holding the geometry as a glb, the rules that put it back, and the file it replaces.
//!
//! This is edit mode: the glb replaces the geometry of a base, and everything else a drawable
//! states (shaders, textures, skeleton, bounds, LOD distances) stays the base's. The base is
//! the extracted file itself, kept beside the glb as `base.ydr` or `base.ydd`, so a folder
//! compiles with no game install and the hash in `asset.json` says it is the one the glb left.
//!
//! ```text
//! <name>.drawable.src/    asset.json  base.ydr  drawable.json  model.glb
//! <name>.drawables.src/   asset.json  base.ydd  dictionary.json
//!                         drawables/<entry>/    drawable.json  model.glb
//! ```

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use or_interchange::extract::bones_of;
use or_interchange::gltf::{self, Container};
use or_interchange::import::{describe_channels, Imported as ImportTarget};
use or_interchange::{
    replace_geometry_named, Bone, ExtractOptions, ImportOptions, LodSelection, Object, Scene,
    SceneBuilder,
};
use or_io::container::ResourceEnvelope;
use or_io::decl::{File, Gen8, Gen9};
use or_io::gta5::{DrawableTarget, GtaDrawable, GtaFragType, PgDictionary, RmcDrawable};
use or_io::Generation;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::asset::{
    folder_name, sha256_hex, unsafe_file_name, AssetManifest, How, Origin, Source, MANIFEST,
};
use crate::drawable_values::{
    apply, describe_into, names_of, BoneRule, Edited, Extent, Material, ModelRule, Real, Sphere,
};
use crate::json::canonical;
use crate::Problem;

/// The `kind` of a drawable's source.
pub const DRAWABLE: &str = "drawable";
/// The `kind` of a drawable dictionary's source.
pub const DRAWABLES: &str = "drawables";
/// A drawable's rules, which a drawable source always holds.
pub const RULES: &str = "drawable.json";
/// The entries of a dictionary, which a dictionary source always holds.
pub const DICTIONARY: &str = "dictionary.json";
/// A drawable's geometry.
pub const MODEL: &str = "model.glb";
/// The folder of a dictionary's drawables.
pub const DIR: &str = "drawables";
/// The only compile mode this build has code for.
pub(crate) const EDIT: &str = "edit";
/// What a protected asset, a file an asset escrow locks, opens with where an RSC7 resource opens with `RSC7`.
// ⚠ `or_project::resource::ESCROW_MARKER` is the same bytes for the crates above this one, and or-wardrobe holds the two equal.
pub const PROTECTED: &[u8] = b"FXAP";

/// What an extraction records about where its file came from, and what it carries.
pub struct Extraction<'a> {
    pub from_game: bool,
    pub virtual_path: Option<&'a str>,
    /// Decode the drawable's own textures into the glb, for a person to look at.
    pub textures: bool,
    /// The skeleton a skinned drawable with none of its own is embedded against.
    pub skeleton: Option<&'a [Bone]>,
}

/// A compiled `.ydr` or `.ydd`, named for the platform's extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub file_name: String,
    pub bytes: Vec<u8>,
    /// What the import did, as lines a panel can list.
    pub report: Vec<String>,
}

/// `drawable.json`: what the base holds, for a person, and the rules a compile puts the glb back by.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "drawable.json")]
pub struct Rules {
    /// Written at extraction and never read by a compile, so editing it changes nothing.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub base: Value,
    #[serde(default)]
    pub import: ImportRules,
    /// The bones the drawable binds to, by name, in the order its palette indexes them. A glb that
    /// lists its skin's bones in another order, as a modelling tool may, is bound by these names.
    /// Left out where the base has no skeleton to name, or two of its bones share a name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skeleton: Vec<String>,
    /// Every shader of the base with its draw bucket, flags and parameters, in the base's order. A value
    /// that differs from the base's is set in the compiled file, and the shader itself is refused if changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    /// Each model of the base, by level, with the name its mesh goes by in the glb and its words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelRule>,
    /// The distances the four levels of detail switch at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lod_distances: Option<[Real; 4]>,
    /// The render mask of each level (`bucket_masks` of `rmcLodGroup`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_masks: Option<[u32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_sphere: Option<Sphere>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_box: Option<Extent>,
    /// The tag of each bone of the drawable's own skeleton, in the base's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<BoneRule>,
}

/// How a glb becomes the geometry of a base, under the names the importer gives its options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ImportRules {
    /// A channel the base declares and the glb omits, such as a colour, takes a stated default
    /// and the report names it. Off: the compile is refused.
    pub fill_missing: bool,
    /// A skinned drawable whose weights the glb cannot state, as a ped component's cannot
    /// without its skeleton, keeps the ones the base holds, and is refused if the vertex count moved.
    pub keep_skinning: bool,
}

impl Default for ImportRules {
    fn default() -> Self {
        Self {
            fill_missing: false,
            keep_skinning: true,
        }
    }
}

impl ImportRules {
    fn options(&self) -> ImportOptions {
        ImportOptions {
            fill_missing: self.fill_missing,
            keep_skinning: self.keep_skinning,
        }
    }
}

/// `dictionary.json`: which folder holds which drawable of the base dictionary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "dictionary.json")]
pub struct Dictionary {
    pub entries: Vec<EntryRef>,
}

/// One drawable of a dictionary, by the hash the file keys it with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntryRef {
    /// `0x` and eight hex digits.
    pub code: String,
    /// A folder of `drawables/`, which is a label and not the key.
    pub folder: String,
}

/// The `compile` block of `asset.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Compile {
    pub(crate) mode: String,
    /// The SHA-256 of the base the glb was made from.
    pub(crate) base: String,
}

/// One drawable's geometry and the rules that put it back.
#[derive(Debug, Clone)]
pub(crate) struct Model {
    pub(crate) rules: Rules,
    pub(crate) glb: Vec<u8>,
}

#[derive(Clone)]
enum Parts {
    One(Box<Model>),
    Many(Vec<(EntryRef, Model)>),
}

/// Everything a compile reads, so a recook can tell nothing changed before it imports anything.
#[derive(Clone)]
pub struct Inputs {
    manifest: AssetManifest,
    compile: Compile,
    base: Vec<u8>,
    parts: Parts,
}

pub(crate) fn problem(why: impl Into<String>, file: impl Into<String>) -> Problem {
    Problem::new(why).in_file(file)
}

/// The file a source of `kind` keeps its base in, or `None` for a kind that is not a model's.
fn base_file(kind: &str) -> Option<&'static str> {
    match kind {
        DRAWABLE => Some("base.ydr"),
        DRAWABLES => Some("base.ydd"),
        _ => None,
    }
}

/// The file a source of `kind` always holds besides `asset.json`; a fragment is a model's source too, because it holds a drawable's glb.
pub fn fixed_file(kind: &str) -> Option<&'static str> {
    match kind {
        DRAWABLE => Some(RULES),
        DRAWABLES => Some(DICTIONARY),
        crate::fragment::KIND => Some(crate::fragment::FIXED),
        _ => None,
    }
}

pub(crate) fn target_of(bytes: &[u8], extension: &str) -> Result<Generation, Vec<Problem>> {
    let refused = |why: String| {
        vec![Problem::new(format!(
            "not a .{extension} this build reads: {why}"
        ))]
    };
    // A protected asset is no RSC7 and would fail as one, on a header word that says nothing.
    if bytes.starts_with(PROTECTED) {
        return Err(vec![Problem::new(format!(
            "this .{extension} is a protected asset: it opens with FXAP and not with an RSC7 header, \
             and OpenRage does not read protected assets"
        ))]);
    }
    let version = or_io::container::resource::Rsc7::peek(bytes)
        .map_err(|e| refused(e.to_string()))?
        .version();
    or_io::gta5::formats::format(extension)
        .ok_or_else(|| refused(format!("no .{extension} format is declared")))?
        .target_of_version(version)
        .map_err(|e| refused(e.to_string()))
}

/// The bones of the skeleton in a ped's `.yft` or in a `.ydr`, which a ped component's palette indexes.
pub fn reference_skeleton(extension: &str, bytes: &[u8]) -> Result<Vec<Bone>, Problem> {
    let first = |mut problems: Vec<Problem>| problems.remove(0);
    let target = target_of(bytes, extension).map_err(first)?;
    if target == Generation::Gen9 {
        skeleton_in::<Gen9>(extension, bytes)
    } else {
        skeleton_in::<Gen8>(extension, bytes)
    }
}

fn skeleton_in<T: DrawableTarget>(extension: &str, bytes: &[u8]) -> Result<Vec<Bone>, Problem>
where
    GtaDrawable<T>: File,
    GtaFragType<T>: File,
{
    let unreadable = |e: or_io::Error| Problem::new(format!("cannot read the .{extension}: {e}"));
    let bones = match extension {
        "yft" => {
            let fragment = GtaFragType::<T>::from_bytes(bytes).map_err(unreadable)?;
            fragment
                .drawables()
                .into_iter()
                .find_map(|d| d.base.skeleton.as_deref().map(bones_of))
        }
        "ydr" => GtaDrawable::<T>::from_bytes(bytes)
            .map_err(unreadable)?
            .base
            .skeleton
            .as_deref()
            .map(bones_of),
        other => {
            return Err(Problem::new(format!(
                "a reference skeleton is read from a .yft or a .ydr, and this is a .{other}"
            )))
        }
    };
    bones
        .filter(|b| !b.is_empty())
        .ok_or_else(|| Problem::new(format!("the .{extension} holds no skeleton")))
}

/// Writes `ydr` into `parent` as `<name>.drawable.src` and returns the folder; nothing is left behind on a refusal.
pub fn extract_drawable(
    ydr: &[u8],
    name: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>> {
    let target = target_of(ydr, "ydr")?;
    let model = if target == Generation::Gen9 {
        drawable_model::<Gen9>(ydr, name, how)
    } else {
        drawable_model::<Gen8>(ydr, name, how)
    }?;
    publish(
        ydr,
        name,
        DRAWABLE,
        target,
        parent,
        how,
        Parts::One(Box::new(model)),
    )
}

/// Writes `ydd` into `parent` as `<name>.drawables.src` and returns the folder; nothing is left behind on a refusal.
pub fn extract_drawables(
    ydd: &[u8],
    name: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>> {
    let target = target_of(ydd, "ydd")?;
    let entries = if target == Generation::Gen9 {
        dictionary_models::<Gen9>(ydd, how)
    } else {
        dictionary_models::<Gen8>(ydd, how)
    }?;
    publish(
        ydd,
        name,
        DRAWABLES,
        target,
        parent,
        how,
        Parts::Many(entries),
    )
}

fn drawable_model<T: Edited>(
    bytes: &[u8],
    name: &str,
    how: &Extraction<'_>,
) -> Result<Model, Vec<Problem>>
where
    GtaDrawable<T>: File,
{
    let document = GtaDrawable::<T>::from_bytes(bytes)
        .map_err(|e| vec![Problem::new(format!("cannot read the .ydr: {e}"))])?;
    model_of::<T>(name, &document.base, how)?.ok_or_else(|| {
        vec![Problem::new(
            "holds no geometry, so there is nothing to edit",
        )]
    })
}

fn dictionary_models<T: Edited>(
    bytes: &[u8],
    how: &Extraction<'_>,
) -> Result<Vec<(EntryRef, Model)>, Vec<Problem>>
where
    PgDictionary<GtaDrawable<T>>: File,
{
    let dictionary = PgDictionary::<GtaDrawable<T>>::from_bytes(bytes)
        .map_err(|e| vec![Problem::new(format!("cannot read the .ydd: {e}"))])?;
    let held: Vec<(u32, &GtaDrawable<T>)> = dictionary
        .codes
        .iter()
        .copied()
        .zip(&dictionary.entries)
        .filter_map(|(code, entry)| Some((code, entry.as_deref()?)))
        .collect();

    // A drawable's name is the file's own word and not a key: two of one dictionary can state
    // the same one, and a name can be no file name, so a folder falls back to the code.
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let folder_of = |code: u32, stated: &str| {
        let by_code = format!("entry_{code:08x}");
        let alone = held
            .iter()
            .filter(|(_, d)| d.base.name.eq_ignore_ascii_case(stated))
            .count()
            == 1;
        if !stated.is_empty() && alone && unsafe_file_name(stated).is_none() {
            stated.to_string()
        } else {
            by_code
        }
    };

    let mut out = Vec::new();
    let mut problems = Vec::new();
    for (code, entry) in &held {
        let folder = folder_of(*code, &entry.base.name);
        if !taken.insert(folder.to_lowercase()) {
            problems.push(Problem::new(format!(
                "{folder:?} names two drawables, which a folder cannot hold both of"
            )));
            continue;
        }
        match model_of::<T>(&folder, &entry.base, how) {
            Ok(Some(model)) => out.push((
                EntryRef {
                    code: format!("0x{code:08x}"),
                    folder,
                },
                model,
            )),
            // A drawable with no geometry has nothing to edit and stays as the base has it.
            Ok(None) => {}
            Err(mut found) => {
                for p in &mut found {
                    p.message = format!("{folder}: {}", p.message);
                }
                problems.extend(found);
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    if out.is_empty() {
        return Err(vec![Problem::new(
            "holds no geometry, so there is nothing to edit",
        )]);
    }
    Ok(out)
}

/// One drawable's glb and rules, or `None` where it holds no geometry.
pub(crate) fn model_of<T: Edited>(
    name: &str,
    drawable: &RmcDrawable<T>,
    how: &Extraction<'_>,
) -> Result<Option<Model>, Vec<Problem>> {
    let mut builder = SceneBuilder::new(name);
    if let Some(bones) = how.skeleton {
        builder = builder.with_reference_skeleton(bones.to_vec());
    }
    builder.push(
        name,
        drawable,
        &ExtractOptions {
            lods: LodSelection::All,
            textures: how.textures,
        },
    );
    let scene = builder.finish();
    if scene.objects.iter().all(|o| o.meshes.is_empty()) {
        return Ok(None);
    }
    let own = drawable.skeleton.is_some();
    if let Err(why) = bones_hold(&scene, own) {
        return Err(vec![Problem::new(why)]);
    }

    let names: Vec<String> = scene
        .objects
        .first()
        .map(|o| o.bones.iter().map(|b| b.name.clone()).collect())
        .unwrap_or_default();
    let distinct = names.iter().collect::<BTreeSet<_>>().len() == names.len();
    let mut rules = Rules {
        base: describe(drawable, &scene, own),
        import: ImportRules::default(),
        skeleton: if distinct { names } else { Vec::new() },
        ..Rules::default()
    };
    describe_into(&mut rules, name, drawable);
    let glb = gltf::write(&scene, name, Container::Glb).primary;
    Ok(Some(Model { rules, glb }))
}

/// Refuses a weight on a bone the skeleton does not have, which a glb cannot say.
fn bones_hold(scene: &Scene, own: bool) -> Result<(), String> {
    for object in scene.objects.iter().filter(|o| !o.bones.is_empty()) {
        let primitives = object.meshes.iter().flat_map(|m| m.primitives.iter());
        let worst = primitives
            .flat_map(|p| p.joints.iter().zip(&p.weights))
            .flat_map(|(joints, weights)| {
                (0..4)
                    .filter(|lane| weights[*lane] > 0.0)
                    .map(|lane| usize::from(joints[lane]))
            })
            .max();
        if let Some(worst) = worst.filter(|w| *w >= object.bones.len()) {
            return Err(format!(
                "a vertex is weighted to bone {worst}, and the {} skeleton has {} bones{}",
                if own { "drawable's own" } else { "reference" },
                object.bones.len(),
                if own {
                    ""
                } else {
                    "; it is the skeleton of another ped"
                }
            ));
        }
    }
    Ok(())
}

/// What the base holds, in the words a person editing the glb needs: the geometries in the order the
/// glb must bring them, the channels each must supply, and where the skin's skeleton came from.
fn describe<T: DrawableTarget>(drawable: &RmcDrawable<T>, scene: &Scene, own: bool) -> Value {
    let geometries: Vec<Value> = scene
        .primitives()
        .map(|p| {
            json!({
                "at": format!("{}/model{}/geometry{}", p.lod.label(), p.model, p.geometry),
                "material": p.material.and_then(|m| scene.materials.get(m)).map(|m| m.name.as_str()),
                "triangles": p.triangle_count(),
                "vertices": p.vertex_count(),
            })
        })
        .collect();
    let bones = scene.objects.first().map_or(0, |o| o.bones.len());
    let skeleton = match (own, bones) {
        (true, _) => "own",
        (false, 0) => "none",
        (false, _) => "reference",
    };
    json!({
        "channels": describe_channels(drawable),
        "geometries": geometries,
        "skeleton": { "bones": bones, "from": skeleton },
    })
}

/// Checks that what an extraction made compiles, then writes it as the folder of `kind`.
fn publish(
    base: &[u8],
    name: &str,
    kind: &str,
    target: Generation,
    parent: &Path,
    how: &Extraction<'_>,
    parts: Parts,
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
    let mut manifest =
        AssetManifest::new(kind, name, target.slug(), origin).map_err(|p| vec![p])?;
    let compile = Compile {
        mode: EDIT.to_string(),
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
        parts,
    };
    let dir = parent.join(folder_name(name, kind));
    if dir.exists() {
        return Err(vec![Problem::new(format!(
            "{} already exists",
            dir.display()
        ))]);
    }
    // ⚠ A source the compile would refuse is not written: the person finds out now, with the
    // drawable in front of them, and not at their first build.
    inputs.compile()?;
    match write_folder(&dir, &inputs) {
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

fn write_folder(dir: &Path, inputs: &Inputs) -> std::io::Result<()> {
    let write_model = |at: &Path, model: &Model| -> std::io::Result<()> {
        std::fs::create_dir_all(at)?;
        // Plain data of strings and flags: serialising it cannot fail.
        std::fs::write(at.join(RULES), canonical(&model.rules).unwrap_or_default())?;
        std::fs::write(at.join(MODEL), &model.glb)
    };
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(MANIFEST), inputs.manifest.to_text())?;
    if let Some(file) = base_file(&inputs.manifest.kind) {
        std::fs::write(dir.join(file), &inputs.base)?;
    }
    match &inputs.parts {
        Parts::One(model) => write_model(dir, model),
        Parts::Many(entries) => {
            let list = Dictionary {
                entries: entries.iter().map(|(entry, _)| entry.clone()).collect(),
            };
            std::fs::write(dir.join(DICTIONARY), canonical(&list).unwrap_or_default())?;
            for (entry, model) in entries {
                write_model(&dir.join(DIR).join(&entry.folder), model)?;
            }
            Ok(())
        }
    }
}

/// `0x` and eight hex digits as the code a dictionary keys a drawable by.
fn parse_code(code: &str) -> Option<u32> {
    let digits = code.strip_prefix("0x")?;
    (digits.len() == 8).then(|| u32::from_str_radix(digits, 16).ok())?
}

/// The compile block of `manifest`, refused when it names a mode this build has no code for.
pub(crate) fn compile_of(manifest: &AssetManifest) -> Result<Compile, Problem> {
    let stated = manifest.extra.get("compile").ok_or_else(|| {
        problem(
            "a model source states how it compiles, and there is no `compile` block",
            MANIFEST,
        )
    })?;
    let compile: Compile = serde_json::from_value(stated.clone())
        .map_err(|e| problem(format!("compile: {e}"), MANIFEST))?;
    if compile.mode != EDIT {
        return Err(problem(
            format!(
                "compile mode {:?} is not built, and {EDIT:?} is the only mode: a glb replaces the geometry of a base",
                compile.mode
            ),
            MANIFEST,
        ));
    }
    Ok(compile)
}

pub(crate) fn read_model(dir: &Path, inside: &str) -> Result<Model, Vec<Problem>> {
    let at = |file: &str| {
        if inside.is_empty() {
            file.to_string()
        } else {
            format!("{inside}/{file}")
        }
    };
    let text = std::fs::read_to_string(dir.join(RULES))
        .map_err(|e| vec![problem(format!("cannot read: {e}"), at(RULES))])?;
    let rules = serde_json::from_str::<Rules>(&text)
        .map_err(|e| vec![problem(e.to_string(), at(RULES))])?;
    let glb = std::fs::read(dir.join(MODEL))
        .map_err(|e| vec![problem(format!("cannot read: {e}"), at(MODEL))])?;
    Ok(Model { rules, glb })
}

impl Inputs {
    pub fn read(source: &Source) -> Result<Self, Vec<Problem>> {
        let manifest = source.manifest.clone();
        let compile = compile_of(&manifest).map_err(|p| vec![p])?;
        let Some(base_name) = base_file(&manifest.kind) else {
            return Err(vec![problem(
                format!("{:?} is not a model source", manifest.kind),
                MANIFEST,
            )]);
        };
        let base = std::fs::read(source.dir.join(base_name))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), base_name)])?;

        let parts = if manifest.kind == DRAWABLE {
            Parts::One(Box::new(read_model(&source.dir, "")?))
        } else {
            Parts::Many(read_entries(&source.dir)?)
        };
        Ok(Self {
            manifest,
            compile,
            base,
            parts,
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
        let mut model = |folder: &str, model: &Model| {
            put(folder.as_bytes());
            put(canonical(&model.rules).unwrap_or_default().as_bytes());
            put(&model.glb);
        };
        match &self.parts {
            Parts::One(m) => model("", m),
            Parts::Many(entries) => {
                for (entry, m) in entries {
                    model(&format!("{}={}", entry.code, entry.folder), m);
                }
            }
        }
        hash.finalize().into()
    }

    /// The layout the source compiles to, or why it compiles to none this build writes.
    fn target(&self) -> Result<Generation, Vec<Problem>> {
        compile_target(&self.manifest)
    }

    /// The `.ydr` or `.ydd` the inputs compile to, reloaded through the readers before it is returned.
    pub fn compile(&self) -> Result<Compiled, Vec<Problem>> {
        let manifest = &self.manifest;
        let target = self.target()?;
        let held = sha256_hex(&self.base);
        if held != self.compile.base {
            return Err(vec![problem(
                format!(
                    "the base holds {held} and compile.base says {}; the base is not the file the glb was made from",
                    self.compile.base
                ),
                base_file(&manifest.kind).unwrap_or(MANIFEST),
            )]);
        }
        let (bytes, report) = if target == Generation::Gen9 {
            self.replace::<Gen9>()
        } else {
            self.replace::<Gen8>()
        }?;
        Ok(Compiled {
            file_name: output_name(manifest),
            bytes,
            report,
        })
    }

    fn replace<T: Edited>(&self) -> Result<(Vec<u8>, Vec<String>), Vec<Problem>>
    where
        GtaDrawable<T>: File,
        PgDictionary<GtaDrawable<T>>: File,
    {
        let unreadable = |e: or_io::Error| {
            vec![problem(
                format!("the base does not read: {e}"),
                base_file(&self.manifest.kind).unwrap_or(MANIFEST),
            )]
        };
        let unsaved = |e: or_io::Error| {
            vec![Problem::new(format!(
                "the drawable would not save, and the game would misread it: {e}"
            ))]
        };
        match &self.parts {
            Parts::One(model) => {
                let mut document = GtaDrawable::<T>::from_bytes(&self.base).map_err(unreadable)?;
                let (base, report) = replaced(&document.base, model, MODEL)?;
                document.base = base;
                let bytes = document.to_bytes().map_err(unsaved)?;
                let back = GtaDrawable::<T>::from_bytes(&bytes).map_err(|e| {
                    vec![Problem::new(format!(
                        "the compiled .ydr does not read back: {e}"
                    ))]
                })?;
                same_geometries(&back.base, &document.base, "the .ydr")?;
                Ok((bytes, report))
            }
            Parts::Many(entries) => {
                let mut document =
                    PgDictionary::<GtaDrawable<T>>::from_bytes(&self.base).map_err(unreadable)?;
                let mut lines = Vec::new();
                let mut problems = Vec::new();
                for (entry, model) in entries {
                    let inside = format!("{DIR}/{}/{MODEL}", entry.folder);
                    let Some(code) = parse_code(&entry.code) else {
                        problems.push(problem(
                            format!("{:?} is not 0x and eight hex digits", entry.code),
                            DICTIONARY,
                        ));
                        continue;
                    };
                    let held = document
                        .codes
                        .iter()
                        .position(|c| *c == code)
                        .and_then(|at| document.entries.get_mut(at))
                        .and_then(|e| e.as_deref_mut());
                    let Some(held) = held else {
                        problems.push(problem(
                            format!("the base holds no drawable keyed {}", entry.code),
                            DICTIONARY,
                        ));
                        continue;
                    };
                    match replaced(&held.base, model, &inside) {
                        Ok((base, report)) => {
                            held.base = base;
                            lines.extend(
                                report.into_iter().map(|l| format!("{}: {l}", entry.folder)),
                            );
                        }
                        Err(found) => problems.extend(found),
                    }
                }
                if !problems.is_empty() {
                    return Err(problems);
                }
                let bytes = document.to_bytes().map_err(unsaved)?;
                let back = PgDictionary::<GtaDrawable<T>>::from_bytes(&bytes).map_err(|e| {
                    vec![Problem::new(format!(
                        "the compiled .ydd does not read back: {e}"
                    ))]
                })?;
                if back.codes != document.codes {
                    return Err(vec![Problem::new(
                        "the compiled .ydd reads back under other hashes than it was written under",
                    )]);
                }
                for (was, is) in document.entries.iter().zip(&back.entries) {
                    if let (Some(was), Some(is)) = (was, is) {
                        same_geometries(&is.base, &was.base, "the .ydd")?;
                    }
                }
                Ok((bytes, lines))
            }
        }
    }
}

/// `drawable` with the geometry of `model`'s glb and the values its `drawable.json` states, or why they cannot replace it.
///
/// The report is what the import did, then each value that was set where the base stated it otherwise.
pub(crate) fn replaced<T: Edited>(
    drawable: &RmcDrawable<T>,
    model: &Model,
    file: &str,
) -> Result<(RmcDrawable<T>, Vec<String>), Vec<Problem>> {
    let mut scene = gltf::read::load(Path::new(MODEL), &model.glb)
        .map_err(|e| vec![problem(e.to_string(), file)])?;
    bind_by_name(
        &mut scene,
        &model.rules.skeleton,
        drawable.skeleton.is_some(),
    )
    .map_err(|why| vec![problem(why, file)])?;
    let (mut base, report) = replace_geometry_named(
        drawable,
        &scene,
        &model.rules.import.options(),
        &names_of(&model.rules),
    )
    .map_err(|e| vec![problem(e.to_string(), file)])?;
    // The values go on after the geometry, which leaves the drawable's box and levels as they were.
    let rules = std::path::Path::new(file).with_file_name(RULES);
    let mut lines = report.lines();
    lines.extend(apply(
        &mut base,
        &model.rules,
        &rules.to_string_lossy().replace('\\', "/"),
    )?);
    Ok((base, lines))
}

/// Renumbers the joints of `scene` to the order of `skeleton`, by the name of each bone.
///
/// ⚠ A glb's joint is a position in its own skin, and the importer takes it as a bone of the
/// skeleton. A tool that writes the bones in another order than the file it was given would bind
/// every weight to the wrong bone, and nothing but the names says so.
///
/// ⚠ With no names listed and no skeleton of the drawable's own, a skin has nothing to be bound
/// by, so it is refused: taking its joints by position is the mis-skinning this exists to stop.
fn bind_by_name(scene: &mut Scene, skeleton: &[String], own: bool) -> Result<(), String> {
    if skeleton.is_empty() {
        if !own && scene.objects.iter().any(Object::is_skinned) {
            return Err(
                "the glb states a skin, and this drawable has no skeleton of its own and \
                 drawable.json lists no bone names, so nothing says which bone each joint is and \
                 its weights would land by position. Extract the source again with the ped's \
                 skeleton (reference_skeleton), or export the glb without its skin and keep \
                 import.keep_skinning on, which keeps the weights the drawable holds"
                    .to_string(),
            );
        }
        return Ok(());
    }
    let Some(object) = scene.objects.first_mut().filter(|o| !o.bones.is_empty()) else {
        return Ok(());
    };
    let to: Vec<Option<u16>> = object
        .bones
        .iter()
        .map(|b| skeleton.iter().position(|n| *n == b.name).map(|i| i as u16))
        .collect();
    if to.iter().enumerate().all(|(i, t)| *t == Some(i as u16)) {
        return Ok(());
    }
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for primitive in object
        .meshes
        .iter_mut()
        .flat_map(|m| m.primitives.iter_mut())
    {
        for (joints, weights) in primitive.joints.iter_mut().zip(&primitive.weights) {
            for lane in 0..4 {
                if weights[lane] <= 0.0 {
                    continue;
                }
                match to.get(usize::from(joints[lane])) {
                    Some(Some(bone)) => joints[lane] = *bone,
                    _ => {
                        let name = object.bones.get(usize::from(joints[lane]));
                        missing.insert(
                            name.map_or_else(
                                || format!("joint {}", joints[lane]),
                                |b| b.name.clone(),
                            ),
                        );
                    }
                }
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let listed: Vec<String> = missing.iter().take(5).map(|n| format!("{n:?}")).collect();
    Err(format!(
        "a vertex is weighted to {}, and the drawable's skeleton has no bone of that name (it has {} bones, from {:?} to {:?})",
        listed.join(", "),
        skeleton.len(),
        skeleton.first().map_or("", String::as_str),
        skeleton.last().map_or("", String::as_str),
    ))
}

fn geometry_count<T: DrawableTarget>(drawable: &RmcDrawable<T>) -> usize {
    drawable
        .lod_group
        .lods
        .iter()
        .flat_map(|lod| lod.models.iter())
        .map(|model| model.geometries.len())
        .sum()
}

/// Whether `back`, as the readers took it, holds the geometries `written` was made with.
pub(crate) fn same_geometries<T: DrawableTarget>(
    back: &RmcDrawable<T>,
    written: &RmcDrawable<T>,
    what: &str,
) -> Result<(), Vec<Problem>> {
    let (is, was) = (geometry_count(back), geometry_count(written));
    if is == was {
        return Ok(());
    }
    Err(vec![Problem::new(format!(
        "{what} read back with {is} geometries, not the {was} it was written from"
    ))])
}

/// The layout a source compiles to, or why it compiles to none this build writes.
pub(crate) fn compile_target(manifest: &AssetManifest) -> Result<Generation, Vec<Problem>> {
    if manifest.platform != "win64" {
        return Err(vec![problem(
            format!(
                "platform {:?} is not one this build compiles for; only win64 is",
                manifest.platform
            ),
            MANIFEST,
        )]);
    }
    [Generation::Gen8, Generation::Gen9]
        .into_iter()
        .find(|g| g.slug() == manifest.target)
        .ok_or_else(|| {
            vec![problem(
                format!(
                    "target {:?} is neither Legacy nor Enhanced",
                    manifest.target
                ),
                MANIFEST,
            )]
        })
}

fn read_entries(dir: &Path) -> Result<Vec<(EntryRef, Model)>, Vec<Problem>> {
    let text = std::fs::read_to_string(dir.join(DICTIONARY))
        .map_err(|e| vec![problem(format!("cannot read: {e}"), DICTIONARY)])?;
    let list = serde_json::from_str::<Dictionary>(&text)
        .map_err(|e| vec![problem(e.to_string(), DICTIONARY)])?;

    let mut problems = Vec::new();
    let mut codes = BTreeSet::new();
    let mut folders = BTreeSet::new();
    let mut out = Vec::new();
    for entry in list.entries {
        // The folder is a path component of what is read, and a source is a file anyone can edit.
        if let Some(bad) = unsafe_file_name(&entry.folder) {
            problems.push(problem(format!("folder: {bad}"), DICTIONARY));
            continue;
        }
        if parse_code(&entry.code).is_none() {
            problems.push(problem(
                format!("{:?} is not 0x and eight hex digits", entry.code),
                DICTIONARY,
            ));
            continue;
        }
        if !codes.insert(entry.code.clone()) || !folders.insert(entry.folder.to_lowercase()) {
            problems.push(problem(
                format!(
                    "{} in {:?} is listed twice, once by its code or once by its folder",
                    entry.code, entry.folder
                ),
                DICTIONARY,
            ));
            continue;
        }
        let inside = format!("{DIR}/{}", entry.folder);
        match read_model(&dir.join(DIR).join(&entry.folder), &inside) {
            Ok(model) => out.push((entry, model)),
            Err(found) => problems.extend(found),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

/// The name of the file a source compiles to: a `.ydd` for a dictionary, a `.yft` for a fragment and a `.ydr` for the rest.
pub fn output_name(manifest: &AssetManifest) -> String {
    let extension = match manifest.kind.as_str() {
        DRAWABLES => "ydd",
        crate::fragment::KIND => "yft",
        _ => "ydr",
    };
    format!("{}.{extension}", manifest.name)
}

/// Whether a path inside a model source, relative to its folder, is a file the compile reads.
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
        [file] => [MANIFEST, RULES, DICTIONARY, MODEL, "base.ydr", "base.ydd"].contains(file),
        [dir, _, file] => *dir == DIR && (*file == RULES || *file == MODEL),
        _ => false,
    }
}

/// What the folder `source` names compiles to.
pub fn compile(source: &Source) -> Result<Compiled, Vec<Problem>> {
    Inputs::read(source)?.compile()
}

/// What an import took out of a file, and what the compile then did with the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// What the file held that a drawable has no place for.
    pub dropped: Vec<or_interchange::Dropped>,
    pub report: Vec<String>,
}

impl Imported {
    /// What the compile did and what the drawable had no place for, as lines a command or a window can list.
    pub fn lines(&self) -> Vec<String> {
        let carried = self
            .dropped
            .iter()
            .map(|held| format!("not carried: {held}"));
        self.report.iter().cloned().chain(carried).collect()
    }
}

/// The folders an import into `source` can name: a dictionary's drawables, and none for a drawable.
pub fn entries(source: &Source) -> Result<Vec<String>, Problem> {
    if source.manifest.kind != DRAWABLES {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(source.dir.join(DICTIONARY))
        .map_err(|e| problem(format!("cannot read: {e}"), DICTIONARY))?;
    let list = serde_json::from_str::<Dictionary>(&text)
        .map_err(|e| problem(e.to_string(), DICTIONARY))?;
    Ok(list.entries.into_iter().map(|e| e.folder).collect())
}

impl Inputs {
    /// Where the model an import names is: its place among the models, and its glb's path in the folder.
    fn slot_of(&self, entry: Option<&str>) -> Result<(usize, String), Vec<Problem>> {
        match (&self.parts, entry) {
            (Parts::One(_), None) => Ok((0, MODEL.to_string())),
            (Parts::One(_), Some(entry)) => Err(vec![Problem::new(format!(
                "a drawable source has no entries, and {entry:?} was named"
            ))]),
            (Parts::Many(entries), wanted) => {
                let folders: Vec<&str> = entries.iter().map(|(e, _)| e.folder.as_str()).collect();
                let listed = folders.join(", ");
                let Some(wanted) = wanted else {
                    return Err(vec![Problem::new(format!(
                        "a dictionary source holds many drawables: name the one the model goes to ({listed})"
                    ))]);
                };
                match folders.iter().position(|f| *f == wanted) {
                    Some(at) => Ok((at, format!("{DIR}/{wanted}/{MODEL}"))),
                    None => Err(vec![Problem::new(format!(
                        "the dictionary has no entry {wanted:?}; it has {listed}"
                    ))]),
                }
            }
        }
    }

    /// These inputs with the glb of the model at `slot` replaced.
    fn with_glb(&self, slot: usize, glb: Vec<u8>) -> Self {
        let mut out = self.clone();
        match &mut out.parts {
            Parts::One(model) => model.glb = glb,
            Parts::Many(entries) => entries[slot].1.glb = glb,
        }
        out
    }

    /// Names on each primitive of `scene` the geometry of the base it would pair with.
    fn address(&self, slot: usize, scene: &mut Scene) -> Result<(), Vec<Problem>> {
        if self.target()? == Generation::Gen9 {
            self.address_in::<Gen9>(slot, scene)
        } else {
            self.address_in::<Gen8>(slot, scene)
        }
    }

    fn address_in<T: ImportTarget>(
        &self,
        slot: usize,
        scene: &mut Scene,
    ) -> Result<(), Vec<Problem>>
    where
        GtaDrawable<T>: File,
        PgDictionary<GtaDrawable<T>>: File,
    {
        let unreadable = |e: or_io::Error| {
            vec![problem(
                format!("the base does not read: {e}"),
                base_file(&self.manifest.kind).unwrap_or(MANIFEST),
            )]
        };
        match &self.parts {
            Parts::One(_) => {
                let document = GtaDrawable::<T>::from_bytes(&self.base).map_err(unreadable)?;
                or_interchange::address(&document.base, scene);
            }
            Parts::Many(entries) => {
                let document =
                    PgDictionary::<GtaDrawable<T>>::from_bytes(&self.base).map_err(unreadable)?;
                let held = parse_code(&entries[slot].0.code)
                    .and_then(|code| document.codes.iter().position(|c| *c == code))
                    .and_then(|at| document.entries.get(at))
                    .and_then(|e| e.as_deref());
                let Some(held) = held else {
                    return Err(vec![problem(
                        format!("the base holds no drawable keyed {}", entries[slot].0.code),
                        DICTIONARY,
                    )]);
                };
                or_interchange::address(&held.base, scene);
            }
        }
        Ok(())
    }
}

/// Puts the model in `file`, a glb, a glTF or an FBX, into `source` as the glb it compiles from.
///
/// A glb is stored exactly as given, so the artist's own file stays the source; a glTF or an FBX
/// is converted to one, and the report says what the drawable had no place for. Nothing is written
/// unless the source compiles with the model in it, so a refusal leaves the folder as it was.
pub fn import(source: &Source, entry: Option<&str>, file: &Path) -> Result<Imported, Vec<Problem>> {
    let inputs = Inputs::read(source)?;
    let (slot, inside) = inputs.slot_of(entry)?;
    let shown = file.display().to_string();
    let bytes =
        std::fs::read(file).map_err(|e| vec![Problem::new(format!("cannot read {shown}: {e}"))])?;
    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let read = match extension.as_str() {
        "glb" | "gltf" => gltf::read::load(file, &bytes),
        "fbx" => or_interchange::fbx::load(&bytes),
        other => {
            return Err(vec![Problem::new(format!(
                "{shown} is a .{other}, and a model is imported from a .glb, a .gltf or an .fbx"
            ))])
        }
    };
    let mut scene = read.map_err(|e| vec![Problem::new(format!("{shown}: {e}"))])?;
    let dropped = std::mem::take(&mut scene.dropped);

    let glb = if extension == "glb" {
        bytes
    } else {
        inputs.address(slot, &mut scene)?;
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
        gltf::write(&scene, stem, Container::Glb).primary
    };
    // ⚠ Compiled before it is kept: a model the compile refuses must not replace the one that works.
    let compiled = inputs.with_glb(slot, glb.clone()).compile()?;

    let path = source.dir.join(&inside);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let written =
        std::fs::write(&temporary, &glb).and_then(|()| std::fs::rename(&temporary, &path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(vec![problem(format!("cannot write: {e}"), inside)]);
    }
    Ok(Imported {
        dropped,
        report: compiled.report,
    })
}
