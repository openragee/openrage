//! `world new-map`: a `.ymap` made from nothing, with no donor file and no schema snapshot.

use std::path::Path;
use std::process::ExitCode;

use glam::Vec3;
use or_io::decl::File as _;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::CMapData;
use or_io::meta::{BinaryFormat, ErrorMode};
use or_world::archetypes::ArchetypeIndex;
use or_world::create::{self, Placed};
use or_world::map_types::MapTypes;

const USAGE: &str = "world new-map --name <map-name> [--entity <archetype> --at <x,y,z> \
                     --lod-dist <distance> [--turn <x,y,z degrees>] [--scale <xy>[,<z>]] \
                     [--flags <number>] --types <file.ytyp-or-dir>] [--as pso|rbm] <out.ymap>";

/// The container `--as` names, PSO when it is not given.
pub(crate) fn container(args: &[String]) -> Result<BinaryFormat, String> {
    match super::world::flag_value(args, "--as").map(str::to_ascii_lowercase) {
        None => Ok(BinaryFormat::Pso),
        Some(s) if s == "pso" => Ok(BinaryFormat::Pso),
        Some(s) if s == "rbm" => Ok(BinaryFormat::Rbm),
        Some(other) => Err(format!("--as takes pso or rbm, not '{other}'")),
    }
}

/// The numbers of `text`, which separates them with commas.
fn numbers(flag: &str, text: &str) -> Result<Vec<f32>, String> {
    text.split(',')
        .map(|part| {
            part.trim()
                .parse::<f32>()
                .map_err(|_| format!("{flag} takes numbers, and '{part}' is not one"))
        })
        .collect()
}

fn triple(flag: &str, text: &str) -> Result<Vec3, String> {
    match numbers(flag, text)?.as_slice() {
        [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
        other => Err(format!(
            "{flag} takes three numbers as x,y,z, and {} were given",
            other.len()
        )),
    }
}

/// A flag word as a decimal number or as `0x` and hexadecimal digits.
fn word(flag: &str, text: &str) -> Result<u32, String> {
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("{flag} takes a whole number, and '{text}' is not one"))
}

/// The entity the arguments ask for, none when they ask for an empty map.
fn entity(args: &[String]) -> Result<Option<Placed>, String> {
    let value = |flag| super::world::flag_value(args, flag);
    let Some(archetype) = value("--entity") else {
        // ⚠ An option of an entity with no entity would make an empty map and say nothing of what it dropped.
        let stray = [
            "--at",
            "--lod-dist",
            "--turn",
            "--scale",
            "--flags",
            "--types",
        ]
        .into_iter()
        .find(|flag| value(flag).is_some());
        return match stray {
            Some(flag) => Err(format!(
                "{flag} says something of an entity, and no --entity <archetype> names one"
            )),
            None => Ok(None),
        };
    };
    let at = value("--at").ok_or("--entity needs --at <x,y,z>, where it stands")?;
    let lod_dist =
        value("--lod-dist").ok_or("--entity needs --lod-dist <distance>, how far it is drawn")?;
    let lod_dist = lod_dist
        .parse::<f32>()
        .map_err(|_| format!("--lod-dist takes a number, and '{lod_dist}' is not one"))?;
    let mut placed = Placed::at(
        HashedString::from_xml_text(archetype),
        triple("--at", at)?,
        lod_dist,
    );
    if let Some(turn) = value("--turn") {
        let degrees = triple("--turn", turn)?;
        placed.rotation = or_maps::edit::Pose::rotation_of_degrees(degrees.to_array());
    }
    if let Some(scale) = value("--scale") {
        (placed.scale_xy, placed.scale_z) = match numbers("--scale", scale)?.as_slice() {
            [both] => (*both, *both),
            [xy, z] => (*xy, *z),
            other => {
                return Err(format!(
                    "--scale takes one number, or two as xy,z, and {} were given",
                    other.len()
                ))
            }
        };
    }
    if let Some(flags) = value("--flags") {
        placed.flags = word("--flags", flags)?;
    }
    Ok(Some(placed))
}

/// The boxes of the archetypes the type files under `root` declare.
fn boxes(root: &Path) -> Result<ArchetypeIndex, String> {
    let mut index = ArchetypeIndex::new();
    let files = super::world::gather(root, &["ytyp"]);
    if files.is_empty() {
        return Err(format!("{} holds no .ytyp", root.display()));
    }
    for path in files {
        let tree = super::world::read(&path, ErrorMode::Lenient)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let types =
            MapTypes::new(&tree).ok_or_else(|| format!("{} is not a CMapTypes", path.display()))?;
        index.add(types);
    }
    Ok(index)
}

fn made(args: &[String], name: &str) -> Result<Vec<u8>, String> {
    if name.trim().is_empty() {
        return Err("--name is empty, and the game finds a map by its name".to_string());
    }
    let format = container(args)?;
    // Said before anything is read: no type file changes which container can be written.
    create::writable(format).map_err(|e| e.to_string())?;
    let mut map = create::map(name);
    let mut index = ArchetypeIndex::new();
    if let Some(placed) = entity(args)? {
        let types = super::world::flag_value(args, "--types").ok_or(
            "--entity needs --types <file.ytyp-or-dir>: the extents of the map hold the \
             entity's box, which its archetype declares",
        )?;
        index = boxes(Path::new(types))?;
        create::place(&mut map, &placed).map_err(|e| e.to_string())?;
    }
    create::map_file(&map, &index, format).map_err(|e| e.to_string())
}

/// `world new-map`: an empty map with a name, or one that places an entity.
pub fn cmd_new_map(args: &[String]) -> ExitCode {
    let positional: Vec<&String> = {
        let mut skip = false;
        args.iter()
            .filter(|a| {
                if skip {
                    skip = false;
                    return false;
                }
                if a.starts_with("--") {
                    skip = !a.contains('=');
                    return false;
                }
                true
            })
            .collect()
    };
    let (Some(name), [out]) = (
        super::world::flag_value(args, "--name"),
        positional.as_slice(),
    ) else {
        eprintln!("Error: {USAGE}");
        return ExitCode::from(2);
    };
    let encoded = match made(args, name) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // ⚠ Read back before writing: a file its own reader does not return is one the game will not open either.
    let back = match CMapData::from_bytes(&encoded) {
        Ok(back) => back,
        Err(e) => {
            eprintln!("Error: the map made does not read back: {e}");
            return ExitCode::FAILURE;
        }
    };
    if back.name.hash != HashedString::from_xml_text(name).hash {
        eprintln!(
            "Error: the map made reads back with name {:#010X}",
            back.name.hash
        );
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(out, &encoded) {
        eprintln!("Error: {out}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "{out}: {} bytes, PSO, name {:#010X}, {} entity(ies), contentFlags {:#X}",
        encoded.len(),
        back.name.hash,
        back.entities.len(),
        back.content_flags
    );
    if !back.entities.is_empty() {
        let (lo, hi) = (back.entities_extents_min, back.entities_extents_max);
        println!(
            "  entities extents ({}, {}, {}) to ({}, {}, {})",
            lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
        );
    }
    println!(
        "  A map exists for the game once a `_manifest.ymf` states it; whether the game loads \
         a map written as PSO is not checked here."
    );
    ExitCode::SUCCESS
}
