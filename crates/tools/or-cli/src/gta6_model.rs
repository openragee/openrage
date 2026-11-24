//! `model replace`: a loose GTA VI model's geometries replaced from a glTF, each kept in its
//! drawable's vertex layout, skeleton and material slot, and written to a new loose resource.

use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::ExitCode;

use or_interchange::gltf::read::lod_and_model_in_name;
use or_interchange::{LodSlot, Mesh, Primitive, Scene};
use or_io::container::rpf::{
    read_rpf9_body, write_rpf9_resource, Rpf9BodyKind, Rpf9Role, Rpf9Sections,
};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{
    read_dictionary, read_drawable, read_drawable_with_bound, read_skeleton_of, Drawable, Entry,
    Geometry, SkeletonOwner,
};

const USAGE: &str =
    "usage: openrage-cli model replace <resource> <mesh.glb|mesh.gltf> --out <new file> \
[--entry <code>] [--lods left|replaced] [--fill-missing] [--limit <bytes>]";

/// What every write of this verb says of the file it wrote.
const STRUCTURAL: &str = "The resource is a structural result written to a new file: an archive \
that carries it has a table only the game's maker can sign, so the game refuses that archive.";

/// What a replace does with the lower levels of detail the glTF names no geometry of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelLods {
    /// Each is left as the drawable holds it.
    #[default]
    Left,
    /// Each geometry takes the level-0 mesh at its model and place where both draw with one shader.
    Replaced,
}

/// How [`replace_gta6_model`] replaces a drawable's meshes.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModelReplace {
    /// The dictionary entry whose drawable is replaced; `None` for the body's own or its fragment's.
    pub entry: Option<u32>,
    /// A channel the mesh omits at another vertex count takes its stated default.
    pub fill_missing: bool,
    pub lods: ModelLods,
}

/// `sections` with the drawable's geometries replaced from `scene` as `how` says, and a line each.
// Refused by name: a skeleton the drawable's own does not hold, and whatever the geometry writer
// refuses (a vertex layout it does not encode, a joint its palette cannot take).
pub fn replace_gta6_model(
    sections: &Rpf9Sections,
    role: Rpf9Role,
    scene: &Scene,
    how: &ModelReplace,
) -> Result<(Rpf9Sections, Vec<String>), String> {
    let drawable = drawable_of(sections, role, how.entry)?;
    same_skeleton(sections, role, how.entry, scene)?;
    let (expanded, mut notes) = match how.lods {
        ModelLods::Left => (None, left_levels(&drawable, scene)),
        ModelLods::Replaced => {
            let (expanded, notes) = lower_levels(&drawable, scene);
            (Some(expanded), notes)
        }
    };
    let scene = expanded.as_ref().unwrap_or(scene);
    notes.extend(slots(&drawable, scene));
    let (out, mut lines) = super::gta6_import::replace_gta6_mesh_with(
        sections,
        role,
        how.entry,
        scene,
        how.fill_missing,
    )?;
    lines.extend(notes);
    Ok((out, lines))
}

/// The drawable a body of `role` holds: its own, a fragment's, or the dictionary entry `entry`.
fn drawable_of(s: &Rpf9Sections, role: Rpf9Role, entry: Option<u32>) -> Result<Drawable, String> {
    let (v, p) = (&s.virtual_section, Some(s.physical.as_slice()));
    match (role, entry) {
        (Rpf9Role::Drawable, None) => read_drawable(v, p).map_err(|e| e.to_string()),
        (Rpf9Role::DrawableWithBound, None) => read_drawable_with_bound(v, p)
            .map(|r| r.drawable)
            .map_err(|e| e.to_string()),
        (Rpf9Role::Dictionary, Some(code)) => {
            let entries = read_dictionary(v, p).map_err(|e| e.to_string())?.entries;
            (entries.into_iter())
                .find_map(|(c, e)| match e {
                    Ok(Entry::Drawable(d)) if c == code => Some(*d),
                    _ => None,
                })
                .ok_or(format!(
                    "the dictionary holds no drawable under {code:#010X}"
                ))
        }
        (Rpf9Role::Dictionary, None) => {
            Err("this is a dictionary: --entry <code> names the drawable".into())
        }
        (_, Some(_)) => Err("--entry names a dictionary's drawable; this is none".into()),
        _ => Err("this body is no drawable, fragment or dictionary".into()),
    }
}

/// Refuses a glTF that weights a bone the drawable's own skeleton does not hold, by its name.
// A drawable with no skeleton of its own is worn on another's, which a joint names by hash.
fn same_skeleton(
    s: &Rpf9Sections,
    role: Rpf9Role,
    entry: Option<u32>,
    scene: &Scene,
) -> Result<(), String> {
    let owner = match (role, entry) {
        (_, Some(code)) => SkeletonOwner::Entry(code),
        (Rpf9Role::DrawableWithBound, None) => SkeletonOwner::Fragment,
        _ => SkeletonOwner::Drawable,
    };
    let own = match read_skeleton_of(&s.virtual_section, owner) {
        Ok(Some(own)) => own,
        Ok(None) => return Ok(()),
        Err(e) => return Err(format!("the drawable's skeleton does not read: {e}")),
    };
    for object in &scene.objects {
        let weighted: BTreeSet<u16> = (object.meshes.iter().flat_map(|m| &m.primitives))
            .flat_map(|p| p.joints.iter().zip(&p.weights))
            .flat_map(|(j, w)| j.iter().zip(w).filter(|(_, w)| **w > 0.0).map(|(j, _)| *j))
            .collect();
        for joint in weighted {
            let bone = object.bones.get(usize::from(joint)).ok_or_else(|| {
                format!("skeleton mismatch: joint {joint} names no bone of the glTF's skeleton")
            })?;
            let hash = name_hash32(bone.name.as_bytes());
            if !own
                .bones
                .iter()
                .any(|b| b.name == bone.name || b.hash == hash)
            {
                return Err(format!(
                    "skeleton mismatch: the glTF weights {}, a bone the drawable's own skeleton \
                     of {} bones does not hold",
                    bone.name,
                    own.bones.len()
                ));
            }
        }
    }
    Ok(())
}

/// A geometry's level of detail, model and place in it.
type Place = (usize, usize, usize);

/// Where primitive `slot` of `mesh` goes; `None` where it says nowhere.
fn place(mesh: &Mesh, slot: usize, p: &Primitive) -> Option<Place> {
    let LodSlot::Level(level) = p.lod else {
        return None;
    };
    let (model, geometry) = match p.stated_address {
        Some(stated) => stated,
        None => (lod_and_model_in_name(&mesh.name)?.1, slot),
    };
    Some((usize::from(level), model, geometry))
}

fn geometry_at(d: &Drawable, (level, model, geometry): Place) -> Option<&Geometry> {
    let models = d.levels.get(level)?.as_ref()?;
    models.get(model)?.geometries.get(geometry)?.as_ref().ok()
}

fn label((level, model, geometry): Place) -> String {
    format!("lod{level}/model{model}/geometry{geometry}")
}

/// Every place a primitive of `scene` names.
fn named(scene: &Scene) -> BTreeSet<Place> {
    let meshes = scene.objects.iter().flat_map(|o| &o.meshes);
    meshes
        .flat_map(|m| {
            (m.primitives.iter().enumerate()).filter_map(move |(slot, p)| place(m, slot, p))
        })
        .collect()
}

/// The line a replace that leaves the lower levels says of those the glTF names no geometry of.
fn left_levels(d: &Drawable, scene: &Scene) -> Vec<String> {
    let named = named(scene);
    let left: Vec<String> = (1..d.levels.len())
        .filter(|l| d.levels[*l].is_some() && !named.iter().any(|(k, _, _)| k == l))
        .map(|l| format!("lod{l}"))
        .collect();
    match left.is_empty() {
        true => Vec::new(),
        false => vec![format!(
            "{} left as the drawable holds them: the glTF names none of their geometries",
            left.join(", ")
        )],
    }
}

/// `scene` with each lower-level geometry it names none of given the level-0 primitive at the
/// same model and place, where both draw with one shader; and what that took and left.
fn lower_levels(d: &Drawable, scene: &Scene) -> (Scene, Vec<String>) {
    let named = named(scene);
    let mut out = scene.clone();
    let (mut taken, mut lines, mut given) = (0usize, Vec::new(), BTreeSet::new());
    for (o, object) in scene.objects.iter().enumerate() {
        let mut added: Vec<Mesh> = Vec::new();
        for mesh in &object.meshes {
            for (slot, p) in mesh.primitives.iter().enumerate() {
                let Some((0, model, geometry)) = place(mesh, slot, p) else {
                    continue;
                };
                let Some(top) = geometry_at(d, (0, model, geometry)) else {
                    continue;
                };
                for level in 1..d.levels.len() {
                    let at = (level, model, geometry);
                    let Some(lower) = geometry_at(d, at) else {
                        continue;
                    };
                    if named.contains(&at) || !given.insert(at) {
                        continue;
                    }
                    if lower.shader != top.shader {
                        lines.push(format!(
                            "{} left as held: it draws with shader {}, the level-0 geometry with \
                             shader {}",
                            label(at),
                            lower.shader,
                            top.shader
                        ));
                        continue;
                    }
                    let lod = LodSlot::Level(level as u8);
                    let copy = Primitive {
                        lod,
                        stated_address: Some((model, geometry)),
                        ..p.clone()
                    };
                    let name = format!("{}_{}_model{model}", object.name, lod.label());
                    match added.iter_mut().find(|m| m.name == name) {
                        Some(m) => m.primitives.push(copy),
                        None => added.push(Mesh {
                            name,
                            lod,
                            primitives: vec![copy],
                        }),
                    }
                    taken += 1;
                }
            }
        }
        out.objects[o].meshes.extend(added);
    }
    let mut rest = 0;
    for (level, models) in d.levels.iter().enumerate().skip(1) {
        let Some(models) = models else { continue };
        for (m, model) in models.iter().enumerate() {
            rest += (0..model.geometries.len())
                .filter(|g| !named.contains(&(level, m, *g)) && !given.contains(&(level, m, *g)))
                .count();
        }
    }
    lines.insert(
        0,
        format!("lower levels: {taken} geometries take the level-0 mesh at their model and place"),
    );
    if rest > 0 {
        lines.push(format!(
            "lower levels: {rest} geometries left as held, the glTF giving no level-0 mesh at \
             their model and place"
        ));
    }
    (out, lines)
}

/// A line for each geometry whose glTF material names another shader than its slot, which it keeps.
fn slots(d: &Drawable, scene: &Scene) -> Vec<String> {
    let mut lines = Vec::new();
    for mesh in scene.objects.iter().flat_map(|o| &o.meshes) {
        for (slot, p) in mesh.primitives.iter().enumerate() {
            let (Some(at), Some(name)) = (place(mesh, slot, p), p.material_name.as_deref()) else {
                continue;
            };
            let Some(held) = geometry_at(d, at) else {
                continue;
            };
            let stated = name
                .strip_prefix("shader_")
                .and_then(|n| n.parse::<u16>().ok());
            if stated != Some(held.shader) {
                lines.push(format!(
                    "{}: keeps its material slot, shader {}; the glTF's material {name} is not \
                     applied",
                    label(at),
                    held.shader
                ));
            }
        }
    }
    lines
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.split_first() {
        Some((verb, rest)) if verb == "replace" => {
            let options = match options(rest) {
                Ok(options) => options,
                Err(why) => {
                    eprintln!("Error: {why}");
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            };
            match replace(&options) {
                Ok(lines) => {
                    for line in lines {
                        println!("{line}");
                    }
                    ExitCode::SUCCESS
                }
                Err(why) => {
                    eprintln!("Error: {why}");
                    ExitCode::from(1)
                }
            }
        }
        Some((word, _)) if matches!(word.as_str(), "-h" | "--help" | "help") => {
            eprintln!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// A `model replace` command line, read.
struct Options {
    resource: PathBuf,
    mesh: PathBuf,
    out: PathBuf,
    how: ModelReplace,
    limit: u64,
}

fn options(args: &[String]) -> Result<Options, String> {
    let fill_missing = args.iter().any(|a| a == "--fill-missing");
    let args: Vec<String> = (args.iter().filter(|a| *a != "--fill-missing"))
        .cloned()
        .collect();
    let (limit, args) = super::rpf::split_limit(&args)?;
    let (entry, args) = super::rpf::split_value(&args, "--entry")?;
    let (lods, args) = super::rpf::split_value(&args, "--lods")?;
    let (out, args) = super::rpf::split_value(&args, "--out")?;
    let [resource, mesh] = args.as_slice() else {
        return Err("name one resource and one glTF".into());
    };
    if let Some(flag) = [resource, mesh].into_iter().find(|a| a.starts_with("--")) {
        return Err(format!("{flag} is no option of model replace"));
    }
    let out = out.ok_or("--out names the new file the resource is written to")?;
    let entry = entry
        .map(|word| {
            let code = super::rpf::parse_offset(&word)?;
            u32::try_from(code).map_err(|_| format!("{word} is not a 32-bit entry code"))
        })
        .transpose()?;
    let lods = match lods.as_deref() {
        None | Some("left") => ModelLods::Left,
        Some("replaced") => ModelLods::Replaced,
        Some(other) => return Err(format!("--lods takes left or replaced, not {other}")),
    };
    Ok(Options {
        resource: PathBuf::from(resource),
        mesh: PathBuf::from(mesh),
        out: PathBuf::from(out),
        how: ModelReplace {
            entry,
            fill_missing,
            lods,
        },
        limit,
    })
}

/// The resource `o` names with its model replaced from the glTF, written to `o.out`; its lines.
fn replace(o: &Options) -> Result<Vec<String>, String> {
    let (resource, out) = (o.resource.to_string_lossy(), o.out.to_string_lossy());
    if super::navmesh_edit::same_file(&resource, &out) {
        return Err(format!(
            "the output is the input {resource}: name a new file"
        ));
    }
    let file = fs::File::open(&o.resource).map_err(|e| format!("open({resource}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, 0, o.limit).map_err(|e| e.to_string())?;
    let sections =
        Rpf9Sections::of(&find, &bytes).ok_or(format!("{resource} is not a loose resource"))?;
    let role = match find.kind {
        Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
        _ => None,
    };
    let role = role.ok_or(format!("{resource} is no headerless GTA VI resource"))?;
    let mesh = o.mesh.to_string_lossy();
    let source = fs::read(&o.mesh).map_err(|e| format!("read({mesh}): {e}"))?;
    let scene = or_interchange::gltf::read::load(o.mesh.as_path(), &source)
        .map_err(|e| format!("{mesh} is not a mesh this can read: {e}"))?;
    let (written, mut lines) = replace_gta6_model(&sections, role, &scene, &o.how)?;
    let saved = write_rpf9_resource(&written).map_err(|e| e.to_string())?;
    // The file is written only once it reads back as the drawable it was made from.
    let (find, bytes) = read_rpf9_body(Cursor::new(&saved), 0, u64::MAX)
        .map_err(|e| format!("the written resource does not read back: {e}"))?;
    let back = Rpf9Sections::of(&find, &bytes).ok_or("the written resource does not read back")?;
    drawable_of(&back, role, o.how.entry)?;
    fs::write(&o.out, &saved).map_err(|e| format!("write({out}): {e}"))?;
    lines.insert(
        0,
        format!("{resource} + {mesh} -> {out} ({} bytes)", saved.len()),
    );
    lines.extend(
        scene
            .dropped
            .iter()
            .map(|held| format!("not carried: {held}")),
    );
    lines.push("This checks the resource's bytes. It does not test that GTA VI loads them.".into());
    lines.push(STRUCTURAL.into());
    Ok(lines)
}

#[cfg(test)]
#[path = "gta6_model_tests.rs"]
mod tests;
