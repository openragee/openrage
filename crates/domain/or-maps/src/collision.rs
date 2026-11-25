//! The collision of an area as a person changes it: polygons of a bounds source taken out, given another material or moved, each a
//! named operation that is data, with the operation that takes it back; and what `or-world` proposes as the collision of an object,
//! over the bounds sources a pack holds, turned into such operations.
//!
//! An [`Operation`] names a bounds source of the pack, a folder with `/`, a polygon soup of the bound it compiles to by the address
//! its document gives it (`bound`, `bound_child0`), and polygons by their place in that soup as the source compiles **now**: a soup
//! that is built again holds its polygons in another order, so a place is read from a proposal made after the last edit. An
//! operation a proposal makes states the digest of the source it was read from, and is refused once the source on disk is another, or
//! once an earlier operation of the same list has built its soup again.
//!
//! ⚠ **The versions of an area are written together.** The game reads an area's collision from up to three files, told apart by
//! their names as the dependency index reads a map's static bounds: `<area>`, `hi@<area>` and `ma@<area>` ([`VERSION_PREFIXES`]).
//! An operation names one of them, and every bounds source beside it that is a version of the same area takes the same change, on
//! the polygons with the same corners to within [`SAME_CORNER`]. A version that lacks one of the polygons, or the soup, makes the
//! operation refuse and is named, and so is a bound the source cannot build again; nothing is then written.
//!
//! ⚠ **The versions the game ships are not copies of one another.** Of the staged bounds, the versions of an area hold other soups
//! and almost none of each other's polygons, so an operation that asks them to be alike is refused on them. An operation that states
//! `alone` changes its one source, and that is what a proposal comes to: the box of an object is judged against each version, each
//! loses what was proposed in it, and the list is applied whole or not at all.
//!
//! ⚠ **An inverse is the files as they were.** A glb written again is not held to be the glb a tool wrote, so what takes an edit back
//! is [`Operation::RestoreSources`] with the document and the glb of every version it changed, byte for byte. It is as large as they
//! are.
//!
//! A moved polygon gets corners of its own: the polygons that stay state the corners they stated, so a corner shared with one of them
//! stays where it was.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Affine3A, Quat, Vec3, Vec3A};
use or_io::decl::File;
use or_io::gta5::PhBoundAny;
use or_source::bounds::edit::{leaves, write_files, Leaf, Polygon, Soup};
use or_source::bounds::{Inputs, KIND};
use or_world::collision::{
    associate, Association, AssociationOptions, BoundCollider, CollisionSet, OrientedBox,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::edit;
use crate::facts::{inspect, stays_inside, Depth};
use crate::model::{self, Maps};

pub mod sample;

/// The file a list of collision operations states it is.
pub const FORMAT: &str = "openrage-collision-operations";
/// The one schema of a list this build reads and writes.
pub const SCHEMA: u32 = 1;

/// What comes before the name of an area in the name of a version of its collision; the third version has the name alone.
pub const VERSION_PREFIXES: [&str; 2] = ["hi@", "ma@"];

/// How far apart two versions may hold one corner, in metres: each stores it in the quantum of its own bound, millimetres for an area, so a centimetre apart is one corner; unmeasured on game files.
pub const SAME_CORNER: f32 = 0.01;

/// How far the axes of a composite's matrix may be from one length and right angles, as a fraction, and still carry a rigid move.
const SKEW: f32 = 1e-3;

/// The area a bound's file is a version of the collision of: its name without `hi@` or `ma@`.
pub fn area_of(name: &str) -> &str {
    VERSION_PREFIXES
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name)
}

/// Why a list of operations was not read, or an operation not applied.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a list of collision operations: {1}")]
    Json(String, serde_json::Error),
    #[error("{0} is not a list of collision operations: its format is `{1}`, not `{FORMAT}`")]
    Format(String, String),
    #[error(
        "{0} is a list of collision operations of schema {1}, and this OpenRage reads schema {SCHEMA}"
    )]
    Schema(String, u64),
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("{name} of `{at}`: {why}")]
    Refused {
        /// What was asked, as an operation's JSON names it.
        name: &'static str,
        /// The source it names.
        at: String,
        why: String,
    },
    #[error("operation {number} of {of}, {refused}")]
    InList {
        /// Counted from one.
        number: usize,
        of: usize,
        refused: Box<Error>,
    },
    #[error("{0}")]
    Write(String),
}

/// Bytes as a list of operations holds them: lower-case hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hex(pub Vec<u8>);

fn is_false(stated: &bool) -> bool {
    !*stated
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Serialize for Hex {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex(&self.0))
    }
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let digits = text.as_bytes();
        if digits.len() % 2 != 0 {
            return Err(serde::de::Error::custom("an odd number of hex digits"));
        }
        let nibble = |digit: u8| match digit {
            b'0'..=b'9' => Ok(digit - b'0'),
            b'a'..=b'f' => Ok(digit - b'a' + 10),
            _ => Err(serde::de::Error::custom(
                "a digit that is not lower-case hex",
            )),
        };
        digits
            .chunks(2)
            .map(|pair| Ok(nibble(pair[0])? << 4 | nibble(pair[1])?))
            .collect::<Result<Vec<u8>, D::Error>>()
            .map(Self)
    }
}

/// The document and the glb of one bounds source, as an inverse puts them back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restored {
    pub source: String,
    /// The text of `bounds.json`.
    pub document: String,
    /// The bytes of `bounds.glb`; none for a source that holds no glb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Hex>,
}

/// One edit of the collision of an area, by the name its JSON states as `op`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Take polygons out of a soup, in every version of the area.
    RemovePolygons {
        source: String,
        /// The address of the soup in the source's document.
        soup: String,
        /// The polygons, by their place in the soup as `source` compiles now.
        polygons: Vec<u32>,
        /// The digest of `source` as the polygons were chosen in it; stated, the operation is refused once the source on disk is another.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        /// Change `source` alone, whatever versions of its area stand beside it.
        #[serde(default, skip_serializing_if = "is_false")]
        alone: bool,
    },
    /// Give polygons of a soup a material, in every version of the area; a material the soup does not list is added to its list.
    SetMaterial {
        source: String,
        soup: String,
        polygons: Vec<u32>,
        /// The game's value of the material, written `0x…`.
        material: String,
        /// The digest of `source` as the polygons were chosen in it; stated, the operation is refused once the source on disk is another.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        /// Change `source` alone, whatever versions of its area stand beside it.
        #[serde(default, skip_serializing_if = "is_false")]
        alone: bool,
    },
    /// Move polygons of a soup by a turn and then a shift, in the space of the file, in every version of the area; the corners they share with polygons that stay are left where they are.
    MovePolygons {
        source: String,
        soup: String,
        polygons: Vec<u32>,
        /// The turn, as `x, y, z, w`; stored at length one.
        rotation: [f32; 4],
        /// The shift after the turn, in metres.
        translation: [f32; 3],
        /// The digest of `source` as the polygons were chosen in it; stated, the operation is refused once the source on disk is another.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expects: Option<String>,
        /// Change `source` alone, whatever versions of its area stand beside it.
        #[serde(default, skip_serializing_if = "is_false")]
        alone: bool,
    },
    /// Hold the document and the glb of each source as given: what takes an edit back.
    RestoreSources { sources: Vec<Restored> },
}

impl Operation {
    /// The name the JSON form states as `op`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::RemovePolygons { .. } => "remove_polygons",
            Self::SetMaterial { .. } => "set_material",
            Self::MovePolygons { .. } => "move_polygons",
            Self::RestoreSources { .. } => "restore_sources",
        }
    }

    /// The source it names, the first of them for one that names several.
    pub fn source(&self) -> &str {
        match self {
            Self::RemovePolygons { source, .. }
            | Self::SetMaterial { source, .. }
            | Self::MovePolygons { source, .. } => source,
            Self::RestoreSources { sources } => sources.first().map_or("", |s| s.source.as_str()),
        }
    }
}

/// A list of operations as a file: what `openrage-cli maps collision --operations` takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operations {
    pub format: String,
    pub schema: u32,
    pub operations: Vec<Operation>,
}

impl Operations {
    pub fn new(operations: Vec<Operation>) -> Self {
        Self {
            format: FORMAT.to_string(),
            schema: SCHEMA,
            operations,
        }
    }

    /// The list as JSON with a two-space indent and LF.
    pub fn to_json(&self) -> Result<String, Error> {
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Json("the list".to_string(), e))?;
        text.push('\n');
        Ok(text)
    }

    /// The list `bytes` hold; `shown` names them in an error. A format that is not [`FORMAT`] and a schema that is not [`SCHEMA`] are refused before an operation is read.
    pub fn from_json(bytes: &[u8], shown: &str) -> Result<Self, Error> {
        let json = |e| Error::Json(shown.to_string(), e);
        let top: serde_json::Value = serde_json::from_slice(bytes).map_err(json)?;
        let format = top.get("format").and_then(|f| f.as_str()).unwrap_or("");
        if format != FORMAT {
            return Err(Error::Format(shown.to_string(), format.to_string()));
        }
        let schema = top.get("schema").and_then(serde_json::Value::as_u64);
        if schema != Some(u64::from(SCHEMA)) {
            return Err(Error::Schema(shown.to_string(), schema.unwrap_or(0)));
        }
        serde_json::from_value(top).map_err(json)
    }
}

/// What an operation did.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The operation that puts every source back byte for byte.
    pub inverse: Operation,
    /// The sources it changed a file of, as the pack names them, in order.
    pub changed: Vec<String>,
    /// What it did to each version and what the compile of each said, a sentence each.
    pub report: Vec<String>,
}

fn problems(found: Vec<or_source::Problem>) -> String {
    found
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The bound `inputs` compile to.
fn compiled(inputs: &Inputs) -> Result<PhBoundAny, String> {
    let bytes = inputs.compile().map_err(problems)?.bytes;
    PhBoundAny::from_bytes(&bytes).map_err(|e| e.to_string())
}

/// The leaf at `address` of what `inputs`, the bounds source `source`, compile to, which is a soup.
fn soup_at(inputs: &Inputs, source: &str, address: &str) -> Result<(Leaf, Soup), String> {
    let bound = compiled(inputs)
        .map_err(|why| format!("the bounds source `{source}` does not compile: {why}"))?;
    let leaf = leaves(&bound)?
        .into_iter()
        .find(|leaf| leaf.address == address)
        .ok_or_else(|| format!("the version `{source}` holds no bound at `{address}`"))?;
    match leaf.soup.clone() {
        Some(soup) => Ok((leaf, soup)),
        None => Err(format!(
            "`{address}` of `{source}` is a {}, one shape and no polygon soup: a bounds source takes no child out of a composite and edits no shape's polygons",
            leaf.kind
        )),
    }
}

/// Whether two versions hold one polygon: the same kind on the same points, in any order, and the same radius.
fn alike(a: &Polygon, b: &Polygon) -> bool {
    if a.kind != b.kind
        || a.points.len() != b.points.len()
        || (a.radius - b.radius).abs() > SAME_CORNER
    {
        return false;
    }
    let mut spare: Vec<Vec3> = b.points.iter().map(|p| Vec3::from_array(*p)).collect();
    a.points.iter().all(|p| {
        let p = Vec3::from_array(*p);
        match spare.iter().position(|q| p.distance(*q) <= SAME_CORNER) {
            Some(at) => {
                spare.swap_remove(at);
                true
            }
            None => false,
        }
    })
}

/// A polygon said so that a person finds it: what it is and where its first point is.
fn said(polygon: &Polygon) -> String {
    match polygon.points.first() {
        Some([x, y, z]) => format!("a {} at {x}, {y}, {z}", polygon.kind.word()),
        None => format!("a {}", polygon.kind.word()),
    }
}

/// What an edit does to the polygons it names.
enum Change {
    Remove,
    Material(u64),
    /// A rigid move in the space of the file.
    Move(Affine3A),
}

/// The matrices of the composites a leaf is under, as the one transform from its space to the file's.
fn to_file(leaf: &Leaf) -> Affine3A {
    leaf.placed_by
        .iter()
        .fold(Affine3A::IDENTITY, |whole, [x, y, z, at]| {
            whole
                * Affine3A::from_cols(
                    Vec3A::from(*x),
                    Vec3A::from(*y),
                    Vec3A::from(*z),
                    Vec3A::from(*at),
                )
        })
}

/// [`to_file`], refused where a matrix on the way is not a rotation, one scale and a translation: only such a one carries a rigid move.
fn rigid_to_file(leaf: &Leaf) -> Result<Affine3A, String> {
    for [x, y, z, at] in &leaf.placed_by {
        let (x, y, z) = (Vec3::from(*x), Vec3::from(*y), Vec3::from(*z));
        let scale = x.length();
        let slack = SKEW * scale;
        let even = (y.length() - scale).abs() <= slack && (z.length() - scale).abs() <= slack;
        let square = [x.dot(y), x.dot(z), y.dot(z)]
            .iter()
            .all(|d| d.abs() <= slack * scale);
        if !(scale > 0.0 && scale.is_finite() && even && square && Vec3::from(*at).is_finite()) {
            return Err(format!(
                "`{}` is placed by a matrix that is not a rotation, one scale and a translation, so a move of the file is no rigid move of its polygons",
                leaf.address
            ));
        }
    }
    Ok(to_file(leaf))
}

/// `polygon` with its points where `place` puts them.
fn placed(polygon: &Polygon, place: &Affine3A) -> Polygon {
    Polygon {
        points: (polygon.points.iter())
            .map(|p| place.transform_point3(Vec3::from_array(*p)).to_array())
            .collect(),
        ..polygon.clone()
    }
}

/// `soup` with `change` made to the polygons `chosen`, which are places in it.
fn changed_soup(
    leaf: &Leaf,
    soup: &Soup,
    chosen: &BTreeSet<usize>,
    change: &Change,
) -> Result<Soup, String> {
    let mut out = soup.clone();
    match change {
        Change::Remove => {
            let mut at = 0;
            out.polygons.retain(|_| {
                at += 1;
                !chosen.contains(&(at - 1))
            });
        }
        Change::Material(id) => {
            let slot = match out.materials.iter().position(|held| held == id) {
                Some(slot) => slot,
                None => {
                    out.materials.push(*id);
                    out.materials.len() - 1
                }
            };
            let slot = u8::try_from(slot).map_err(|_| {
                format!(
                    "`{}` lists {} materials, and a polygon names its material in eight bits",
                    leaf.address,
                    out.materials.len()
                )
            })?;
            for at in chosen {
                out.polygons[*at].material = Some(slot);
            }
        }
        Change::Move(by) => {
            let place = rigid_to_file(leaf)?;
            let in_soup = place.inverse() * *by * place;
            for at in chosen {
                out.polygons[*at] = placed(&out.polygons[*at], &in_soup);
            }
        }
    }
    Ok(out)
}

/// A bounds source the work has opened: its inputs as the operations so far leave them, and as the disk holds them.
struct Held {
    dir: PathBuf,
    /// The name of the file it compiles to, without its extension.
    name: String,
    inputs: Inputs,
    on_disk: Inputs,
}

impl Held {
    fn differs(&self) -> bool {
        self.inputs.document() != self.on_disk.document()
            || self.inputs.model() != self.on_disk.model()
    }

    fn restored(&self, source: &str) -> Result<Restored, String> {
        Ok(Restored {
            source: source.to_string(),
            document: String::from_utf8(self.inputs.document().to_vec()).map_err(|_| {
                format!("the document of `{source}` is not UTF-8, so the operation could not be taken back")
            })?,
            model: self.inputs.model().map(|glb| Hex(glb.to_vec())),
        })
    }
}

/// The bounds source in `dir`; an error for a folder that is another kind of source or none.
fn open(dir: &Path) -> Result<(String, Inputs), String> {
    let opened = or_source::asset::Source::open(dir).map_err(|p| p.to_string())?;
    if opened.manifest.kind != KIND {
        return Err(format!(
            "it is a {} source, and collision is edited in a bounds source",
            opened.manifest.kind
        ));
    }
    let inputs = Inputs::read(&opened).map_err(problems)?;
    Ok((opened.manifest.name, inputs))
}

/// The bounds sources of a pack while operations are applied: nothing is written until [`Work::commit`].
struct Work<'a> {
    folder: &'a Path,
    held: BTreeMap<String, Held>,
}

impl<'a> Work<'a> {
    fn dir(&self, source: &str) -> PathBuf {
        self.folder
            .join(source.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    /// The source `source`, opened once; one that leaves the folder of the pack is refused.
    fn hold(&mut self, source: &str) -> Result<&mut Held, String> {
        if !self.held.contains_key(source) {
            // ⚠ A list of operations is a file anyone can write, and a name in it with `..` would be written beside the pack.
            if !stays_inside(source) {
                return Err(
                    "the source leaves the folder of the pack, and an operation edits what is inside it"
                        .to_string(),
                );
            }
            let dir = self.dir(source);
            let (name, inputs) = open(&dir)?;
            // ⚠ A disk that does not tell `S` from `s` opens one folder under two names, and a source held twice is written twice.
            let listed = (self.folder.canonicalize())
                .map(|folder| source.split('/').fold(folder, |dir, part| dir.join(part)));
            if !matches!((dir.canonicalize(), listed), (Ok(is), Ok(named)) if is == named) {
                return Err(
                    "the pack holds no source by that name, letter for letter, in a folder of its own"
                        .to_string(),
                );
            }
            let held = Held {
                dir,
                name,
                on_disk: inputs.clone(),
                inputs,
            };
            self.held.insert(source.to_string(), held);
        }
        self.held
            .get_mut(source)
            .ok_or_else(|| "the source was not opened".to_string())
    }

    /// `source` and the bounds sources beside it that are versions of the same area, in order.
    fn versions(&mut self, source: &str) -> Result<Vec<String>, String> {
        let area = area_of(&self.hold(source)?.name).to_string();
        let parent = source.rsplit_once('/').map(|(parent, _)| parent);
        let beside = match parent {
            Some(parent) => self.dir(parent),
            None => self.folder.to_path_buf(),
        };
        let mut found = BTreeSet::from([source.to_string()]);
        let entries = std::fs::read_dir(&beside)
            .map_err(|e| format!("cannot list {}: {e}", beside.display()))?;
        for entry in entries.flatten() {
            let Ok(file) = entry.file_name().into_string() else {
                continue;
            };
            let path = match parent {
                Some(parent) => format!("{parent}/{file}"),
                None => file,
            };
            if found.contains(&path) || !entry.path().is_dir() {
                continue;
            }
            let is_version = or_source::asset::Source::open(&entry.path())
                .is_ok_and(|s| s.manifest.kind == KIND && area_of(&s.manifest.name) == area);
            if is_version {
                found.insert(path);
            }
        }
        Ok(found.into_iter().collect())
    }

    /// The leaf at `address` of what `source` compiles to now, which is a soup.
    fn soup(&mut self, source: &str, address: &str) -> Result<(Leaf, Soup), String> {
        soup_at(&self.hold(source)?.inputs, source, address)
    }

    /// Make `change` to the polygons `polygons` of the soup `address` of `source`, and to the same polygons of each of `versions`; what was done to each.
    fn edit(
        &mut self,
        versions: &[String],
        source: &str,
        address: &str,
        polygons: &[u32],
        change: &Change,
    ) -> Result<Vec<String>, String> {
        let (named_leaf, named) = self.soup(source, address)?;
        // The chosen polygons where the file has them: versions are alike in the file's space, whatever places their soups.
        let in_file: Vec<Polygon> = (named.polygons.iter())
            .map(|polygon| placed(polygon, &to_file(&named_leaf)))
            .collect();
        let chosen: BTreeSet<usize> = polygons.iter().map(|p| *p as usize).collect();
        if let Some(past) = chosen.iter().find(|at| **at >= named.polygons.len()) {
            return Err(format!(
                "polygon {past} is past the {} that `{address}` of `{source}` holds",
                named.polygons.len()
            ));
        }
        // Every version is edited before one is kept, so a refusal leaves the work as it was.
        let mut edited: Vec<(String, Inputs, String)> = Vec::new();
        for version in versions {
            let (leaf, soup) = self.soup(version, address)?;
            let places: BTreeSet<usize> = if version == source {
                chosen.clone()
            } else {
                let place = to_file(&leaf);
                let held: Vec<Polygon> = (soup.polygons.iter())
                    .map(|polygon| placed(polygon, &place))
                    .collect();
                let mut taken = vec![false; held.len()];
                let mut places = BTreeSet::new();
                for at in &chosen {
                    let wanted = &in_file[*at];
                    let found = (0..held.len())
                        .find(|k| !taken[*k] && alike(&held[*k], wanted))
                        .ok_or_else(|| {
                            format!(
                                "the version `{version}` holds no polygon like polygon {at} of `{address}` in `{source}` ({}), so the versions of the area would no longer be alike",
                                said(wanted)
                            )
                        })?;
                    taken[found] = true;
                    places.insert(found);
                }
                places
            };
            let restated = changed_soup(&leaf, &soup, &places, change)
                .and_then(|soup| {
                    let held = self.hold(version)?;
                    held.inputs.restated(address, &soup).map_err(problems)
                })
                .map_err(|why| format!("the version `{version}`: {why}"))?;
            let built = restated.compile().map_err(|why| {
                format!(
                    "the bounds source `{version}` cannot be built again with the change to `{address}`: {}",
                    problems(why)
                )
            })?;
            let did = match change {
                Change::Remove => "taken out of",
                Change::Material(_) => "given the material in",
                Change::Move(_) => "moved in",
            };
            let mut line = format!("{version}: {} polygon(s) {did} `{address}`", places.len());
            for note in built.report {
                line.push_str(&format!("; {note}"));
            }
            edited.push((version.clone(), restated, line));
        }
        let mut report = Vec::new();
        for (version, restated, line) in edited {
            self.hold(&version)?.inputs = restated;
            report.push(line);
        }
        Ok(report)
    }

    /// Do `operation` to the inputs the work holds.
    fn apply(&mut self, operation: &Operation) -> Result<Applied, String> {
        match operation {
            Operation::RestoreSources { sources } => self.restore(sources),
            Operation::RemovePolygons {
                source,
                soup,
                polygons,
                expects,
                alone,
            } => self.change(source, soup, polygons, expects, *alone, &Change::Remove),
            Operation::SetMaterial {
                source,
                soup,
                polygons,
                material,
                expects,
                alone,
            } => {
                let id = material
                    .strip_prefix("0x")
                    .and_then(|digits| u64::from_str_radix(digits, 16).ok())
                    .ok_or_else(|| {
                        format!("`{material}` is not a material, which is written `0x…`")
                    })?;
                self.change(
                    source,
                    soup,
                    polygons,
                    expects,
                    *alone,
                    &Change::Material(id),
                )
            }
            Operation::MovePolygons {
                source,
                soup,
                polygons,
                rotation,
                translation,
                expects,
                alone,
            } => {
                let turn = Quat::from_array(*rotation);
                let shift = Vec3::from_array(*translation);
                if !(turn.is_finite() && shift.is_finite()) {
                    return Err(
                        "a turn or a shift that is not a finite number moves nothing".to_string(),
                    );
                }
                if turn.length_squared() <= f32::EPSILON {
                    return Err("a rotation of no length turns nothing".to_string());
                }
                let by = Affine3A::from_rotation_translation(turn.normalize(), shift);
                self.change(source, soup, polygons, expects, *alone, &Change::Move(by))
            }
        }
    }

    fn change(
        &mut self,
        source: &str,
        soup: &str,
        polygons: &[u32],
        expects: &Option<String>,
        alone: bool,
        change: &Change,
    ) -> Result<Applied, String> {
        // ⚠ A place in a soup is a place in one compile of it: a list made for another state of the source names other polygons.
        if let Some(expected) = expects {
            let held = self.hold(source)?;
            if *expected != hex(&held.on_disk.digest()) {
                return Err(format!(
                    "`{source}` is not the source the polygons were chosen in: it changed since, and a soup built again holds its polygons in another order, so propose again"
                ));
            }
            // ⚠ The digest is of the source on disk, which an earlier operation of the list leaves as it was while it builds the soup again.
            if held.differs()
                && soup_at(&held.on_disk, source, soup)?.1 != soup_at(&held.inputs, source, soup)?.1
            {
                return Err(format!(
                    "an earlier operation of the list changed `{soup}` of `{source}`, and the polygons were chosen in the soup as it was: ask what is asked of one soup in one operation"
                ));
            }
        }
        let versions = match alone {
            true => vec![source.to_string()],
            false => self.versions(source)?,
        };
        let before = self.before(&versions)?;
        let report = self.edit(&versions, source, soup, polygons, change)?;
        self.done(before, &versions, report)
    }

    fn restore(&mut self, sources: &[Restored]) -> Result<Applied, String> {
        let mut put_back = Vec::new();
        for restored in sources {
            let held = self.hold(&restored.source)?;
            let candidate = held.inputs.with_files(
                restored.document.clone().into_bytes(),
                restored.model.as_ref().map(|glb| glb.0.clone()),
            );
            // ⚠ Files that do not compile would be a source nothing cooks.
            candidate.compile().map_err(|why| {
                format!(
                    "what is to be put back in `{}` does not compile: {}",
                    restored.source,
                    problems(why)
                )
            })?;
            put_back.push((restored.source.clone(), candidate));
        }
        let touched: Vec<String> = put_back.iter().map(|(s, _)| s.clone()).collect();
        let before = self.before(&touched)?;
        for (source, candidate) in put_back {
            self.hold(&source)?.inputs = candidate;
        }
        self.done(before, &touched, Vec::new())
    }

    /// What puts `sources` back as the work holds them now.
    fn before(&mut self, sources: &[String]) -> Result<Vec<Restored>, String> {
        sources
            .iter()
            .map(|source| self.hold(source)?.restored(source))
            .collect()
    }

    fn done(
        &mut self,
        before: Vec<Restored>,
        touched: &[String],
        report: Vec<String>,
    ) -> Result<Applied, String> {
        let mut changed = Vec::new();
        for (source, was) in touched.iter().zip(&before) {
            if self.hold(source)?.restored(source)? != *was {
                changed.push(source.clone());
            }
        }
        Ok(Applied {
            inverse: Operation::RestoreSources { sources: before },
            changed,
            report,
        })
    }

    /// Write the files of every source the operations changed; when one cannot be written, the ones written before it are put back.
    fn commit(self) -> Result<(), Error> {
        let mut written: Vec<&Held> = Vec::new();
        for held in self.held.values() {
            if !held.differs() {
                continue;
            }
            if let Err(why) = write_files(&held.dir, &held.inputs) {
                let mut said = problems(why);
                for earlier in written {
                    if let Err(left) = write_files(&earlier.dir, &earlier.on_disk) {
                        said.push_str(&format!(
                            "; and {} could not be put back as it was: {}",
                            earlier.dir.display(),
                            problems(left)
                        ));
                    }
                }
                return Err(Error::Write(said));
            }
            written.push(held);
        }
        Ok(())
    }
}

/// Apply `operation` to the bounds sources of the pack in `folder`, and say what takes it back; a refusal leaves every source as it was.
pub fn apply(folder: &Path, operation: &Operation) -> Result<Applied, Error> {
    apply_all(folder, std::slice::from_ref(operation), false).and_then(|mut applied| {
        applied
            .pop()
            .ok_or_else(|| Error::Write("the operation was not applied".to_string()))
    })
}

/// Apply `operations` in order to the bounds sources of the pack in `folder`, all of them or none, and say what each did; with `dry_run` nothing is written.
pub fn apply_all(
    folder: &Path,
    operations: &[Operation],
    dry_run: bool,
) -> Result<Vec<Applied>, Error> {
    let mut work = Work {
        folder,
        held: BTreeMap::new(),
    };
    let mut applied = Vec::with_capacity(operations.len());
    for (index, operation) in operations.iter().enumerate() {
        let refused = |why| Error::Refused {
            name: operation.name(),
            at: operation.source().to_string(),
            why,
        };
        let done = work.apply(operation).map_err(|why| match operations {
            [_] => refused(why),
            _ => Error::InList {
                number: index + 1,
                of: operations.len(),
                refused: Box::new(refused(why)),
            },
        })?;
        applied.push(done);
    }
    if !dry_run {
        work.commit()?;
    }
    Ok(applied)
}

/// The turn and the shift that carry what stood with an object at `from` to where it stands at `to`, each a position and the rotation that places it.
pub fn carried(from: (Vec3, Quat), to: (Vec3, Quat)) -> ([f32; 4], [f32; 3]) {
    let turn = (to.1 * from.1.inverse()).normalize();
    (turn.to_array(), (to.0 - turn * from.0).to_array())
}

/// A bound of a bounds source, as a proposal names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeafName {
    /// The address the source's document gives it.
    pub address: String,
    /// The layout it is, by the name of its declaration.
    pub kind: &'static str,
    /// Whether it is a polygon soup, which an operation edits.
    pub soup: bool,
}

/// What is proposed as the collision of an object, over the bounds sources of a pack.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The pieces proposed and left, by bounds source as the pack names it.
    pub association: Association<String>,
    /// The leaves of each bounds source the pack holds, by the number the association gives them.
    pub leaves: BTreeMap<String, Vec<LeafName>>,
    /// The name of the file each bounds source compiles to, without its extension.
    pub names: BTreeMap<String, String>,
    /// The digest of each bounds source as the proposal read it, which its operations state as what they expect.
    pub digests: BTreeMap<String, String>,
}

/// The bounds sources that are folders of the pack in `folder`, as the pack names them, in order.
pub fn bounds_sources(folder: &Path) -> Result<Vec<String>, Error> {
    let listing = std::fs::read_dir(folder)
        .map_err(|e| Error::Write(format!("cannot list {}: {e}", folder.display())))?;
    let mut found: Vec<String> = listing
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            or_source::asset::Source::open(&entry.path()).is_ok_and(|s| s.manifest.kind == KIND)
        })
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    found.sort();
    Ok(found)
}

/// What the bounds sources of the pack in `folder` hold of the collision of an object whose box is `object`, by the rule of `or_world::collision`, each bound read where its file states it.
pub fn propose(
    folder: &Path,
    object: &OrientedBox,
    options: &AssociationOptions,
) -> Result<Proposal, Error> {
    // ⚠ No placement is asked for: the static bounds of a map state their polygons in the world.
    let mut set = CollisionSet::new();
    let mut named = BTreeMap::new();
    let mut names = BTreeMap::new();
    let mut digests = BTreeMap::new();
    for source in bounds_sources(folder)? {
        let refused = |why: String| Error::Refused {
            name: "propose",
            at: source.clone(),
            why,
        };
        let dir = folder.join(&source);
        let (name, inputs) = open(&dir).map_err(refused)?;
        let bound = compiled(&inputs).map_err(refused)?;
        let collider = BoundCollider::new(&bound).map_err(|e| refused(e.to_string()))?;
        set.add(source.clone(), Arc::new(collider), Affine3A::IDENTITY)
            .map_err(|e| refused(e.to_string()))?;
        let held = leaves(&bound).map_err(refused)?;
        named.insert(
            source.clone(),
            held.into_iter()
                .map(|leaf| LeafName {
                    address: leaf.address,
                    kind: leaf.kind,
                    soup: leaf.soup.is_some(),
                })
                .collect(),
        );
        digests.insert(source.clone(), hex(&inputs.digest()));
        names.insert(source, name);
    }
    Ok(Proposal {
        association: associate(&set, object, options),
        leaves: named,
        names,
        digests,
    })
}

/// The operations a proposal comes to, and what it holds that no operation takes.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub operations: Vec<Operation>,
    /// A sentence for each thing proposed that the operations leave.
    pub notes: Vec<String>,
}

impl Proposal {
    /// The address of leaf `leaf` of `source`.
    pub fn leaf(&self, source: &str, leaf: u32) -> Option<&LeafName> {
        self.leaves.get(source)?.get(leaf as usize)
    }

    /// One operation for each soup of each bounds source a polygon is proposed in, made by `operation` from the source, the address, the polygons and the digest of the source.
    ///
    /// ⚠ Each version of an area is changed by what is proposed in it, alone: the versions the game ships hold polygons of their
    /// own, so the polygons proposed in one are not in another. The list is applied whole or not at all, which is how the versions
    /// are written together.
    fn planned(
        &self,
        operation: impl Fn(String, String, Vec<u32>, Option<String>) -> Operation,
    ) -> Planned {
        let mut notes = Vec::new();
        let mut operations = Vec::new();
        for bound in &self.association.bounds {
            for leaf in &bound.leaves {
                let (address, kind, soup) = self
                    .leaf(&bound.key, leaf.leaf)
                    .map_or(("?", "bound", false), |l| {
                        (l.address.as_str(), l.kind, l.soup)
                    });
                if leaf.proposed.iter().any(|p| p.polygon.is_none()) {
                    notes.push(format!(
                        "`{address}` of `{}` is a {kind} wholly in the box, and a bounds source takes no child out of a composite: it stays",
                        bound.key
                    ));
                }
                let polygons: Vec<u32> = leaf.proposed.iter().filter_map(|p| p.polygon).collect();
                if soup && !polygons.is_empty() {
                    operations.push(operation(
                        bound.key.clone(),
                        address.to_string(),
                        polygons,
                        self.digests.get(&bound.key).cloned(),
                    ));
                }
            }
        }
        Planned { operations, notes }
    }

    /// The operations that take the proposed polygons out, each version of an area by what is proposed in it.
    pub fn removal(&self) -> Planned {
        self.planned(
            |source, soup, polygons, expects| Operation::RemovePolygons {
                source,
                soup,
                polygons,
                expects,
                alone: true,
            },
        )
    }

    /// The operations that move the proposed polygons by a turn and then a shift in the world, each version of an area by what is proposed in it.
    pub fn moving(&self, rotation: [f32; 4], translation: [f32; 3]) -> Planned {
        self.planned(|source, soup, polygons, expects| Operation::MovePolygons {
            source,
            soup,
            polygons,
            rotation,
            translation,
            expects,
            alone: true,
        })
    }
}

/// The box of the entity `index` of the map source `source` of the pack in `folder`: the box of its archetype, `stated` or else the one a type file of the pack declares, under its placement.
pub fn object_box(
    folder: &Path,
    source: &str,
    index: usize,
    stated: Option<(Vec3, Vec3)>,
) -> Result<OrientedBox, Error> {
    let refused = |why: String| Error::Refused {
        name: "object",
        at: source.to_string(),
        why,
    };
    if !stays_inside(source) {
        return Err(refused(
            "the source leaves the folder of the pack".to_string(),
        ));
    }
    let dir = folder.join(source.replace('/', std::path::MAIN_SEPARATOR_STR));
    let entity = edit::read_entity(&dir, index).map_err(refused)?;
    let pose = edit::pose_of(&entity);
    let (min, max) = match stated {
        Some(stated) => stated,
        None => {
            let model = Maps::read(folder)?;
            let facts = inspect(folder, &model, Depth::Open);
            let wanted = entity.archetype_name.hash;
            let declared = crate::facts::named_paths(&model)
                .filter_map(|path| facts.source(path)?.types())
                .flat_map(|types| &types.archetypes)
                .find(|archetype| archetype.name.hash == wanted)
                .map(|archetype| (Vec3::from(archetype.bb_min), Vec3::from(archetype.bb_max)));
            declared.ok_or_else(|| {
                refused(format!(
                    "no type file of the pack declares `{}`, the archetype entity {index} places, so its box is not known: state the box",
                    entity.archetype_name.to_xml_text()
                ))
            })?
        }
    };
    if !(min.is_finite() && max.is_finite()) {
        return Err(refused(
            "a box that is not finite numbers holds nothing".to_string(),
        ));
    }
    Ok(OrientedBox::placed(
        min,
        max,
        pose.position,
        pose.rotation,
        or_world::transform::scale_of(pose.scale_xy, pose.scale_z),
    ))
}
