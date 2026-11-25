//! `openrage-cli world ground`: the ground under a position of a GTA V install, over the static
//! collision the world index names for its column.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glam::Vec3;
use or_depends::world_collision::{GroundAnswer, WorldCollision};
use or_depends::world_index::Bound;
use or_depends::MountedWorld;
use or_io::gta5::data::materials_palette::MaterialPalette;
use or_title::{DlcCutoff, MapMode, MountOptions, MountedGame};
use or_world::collision::GroundProbe;
use serde_json::json;

use super::world::flag_value;

/// The flags of this verb that take a value, so the value is not read as a coordinate.
const VALUED_FLAGS: &[&str] = &[
    "--game", "--keys", "--kept", "--level", "--mode", "--rise", "--slope",
];

/// The bytes of colliders one question may hold, which no column of a map comes near.
const BUDGET: u64 = 256 << 20;

/// How far above a position given with a height its ground may be, in metres, with no `--rise`.
const RISE: f32 = 1.0;

/// The steepest surface taken for ground, in degrees from level, with no `--slope`.
const SLOPE: f32 = 60.0;

const USAGE: &str = "world ground --game <dir> <x> <y> [<z>] [--level <pack>] \
                     [--mode story|multiplayer] [--rise <m>] [--slope <degrees>] [--kept <file>] \
                     [--keys <file>] [--fresh] [--json]";

/// The position a person asked about, and how the ground under it is looked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Asked {
    pub x: f32,
    pub y: f32,
    /// `None` for the highest ground of the column.
    pub z: Option<f32>,
    /// Metres.
    pub rise: f32,
    /// Degrees from level.
    pub slope: f32,
}

/// The flags of this verb that take no value.
const PLAIN_FLAGS: &[&str] = &["--json", "--fresh"];

/// A flag's value as a number no less than zero, and no more than `most` where there is a most.
fn measure(
    args: &[String],
    flag: &str,
    default: f32,
    most: Option<f32>,
    unit: &str,
) -> Result<f32, String> {
    let Some(word) = flag_value(args, flag) else {
        return Ok(default);
    };
    (word.parse::<f32>().ok())
        .filter(|value| *value >= 0.0 && *value <= most.unwrap_or(f32::MAX))
        .ok_or_else(|| match most {
            Some(most) => format!("{flag} is {unit} from 0 to {most}, not `{word}`"),
            None => format!("{flag} is {unit}, 0 or more, not `{word}`"),
        })
}

/// The position of `args`, two numbers or three, and its `--rise` and `--slope`.
pub(super) fn asked_of(args: &[String]) -> Result<Asked, String> {
    // ⚠ A flag this verb does not know is refused: passed over, its value would be read as a
    // coordinate, and a position of two numbers would become one of three.
    let known = |flag: &str| VALUED_FLAGS.contains(&flag) || PLAIN_FLAGS.contains(&flag);
    let flags = args.iter().filter(|arg| arg.starts_with("--"));
    if let Some(unknown) = flags
        .map(|arg| arg.split_once('=').map_or(arg.as_str(), |(flag, _)| flag))
        .find(|flag| !known(flag))
    {
        return Err(format!("`{unknown}` is no flag of this verb"));
    }
    // ⚠ And one left without its value is refused, where it would stand at its default unsaid.
    if let Some(flag) = (args.last()).filter(|arg| VALUED_FLAGS.contains(&arg.as_str())) {
        return Err(format!("`{flag}` takes a value"));
    }
    let numbers = super::world_index::numbers_of(args, VALUED_FLAGS)?;
    let (x, y, z) = match numbers[..] {
        [x, y] => (x, y, None),
        [x, y, z] => (x, y, Some(z)),
        _ => {
            return Err(format!(
                "a position is two numbers, x y, or three, x y z, and {} were given",
                numbers.len()
            ))
        }
    };
    Ok(Asked {
        x,
        y,
        z,
        rise: measure(args, "--rise", RISE, None, "metres")?,
        slope: measure(args, "--slope", SLOPE, Some(90.0), "degrees")?,
    })
}

/// Where the mount stops and which mode it takes, as `--level` and `--mode` say.
pub(super) fn mount_of(args: &[String]) -> Result<(DlcCutoff, MountOptions), String> {
    let (level, mode) = (flag_value(args, "--level"), flag_value(args, "--mode"));
    super::search::dlc_list_mount(None, level, mode)
}

/// What is said of the mount a question was asked of, a line each.
pub(super) fn mount_notes(game: &MountedGame, mode: Option<MapMode>) -> Vec<String> {
    let mut notes = Vec::new();
    let left_out = super::search::packs_left_out(game);
    if !left_out.is_empty() {
        notes.push(format!(
            "the level leaves out {} of {} listed pack(s) and their bounds: {}",
            left_out.len(),
            game.listed.len(),
            left_out.join(", ")
        ));
    }
    if let Some(mode) = mode {
        // ⚠ A mode takes or leaves a pack's change sets, and the index reads no change set: it
        // reads every bounds file of every archive mounted.
        let sets = game
            .packs
            .iter()
            .flat_map(|pack| &pack.change_sets_left_out);
        let sets: Vec<&str> = sets.map(String::as_str).collect();
        let left_out = match sets.len() {
            0 => String::new(),
            count => format!(
                "; {count} change set(s) stated for the other mode: {}",
                sets.join(", ")
            ),
        };
        notes.push(format!(
            "mounted as {}, which leaves out no bounds file: the index reads every one a \
             mounted pack holds, whatever change set would load it{left_out}",
            mode.label()
        ));
    }
    notes
}

/// The ground `asked` about, over what `world` reads for its column.
pub(super) fn ground_of<'a>(world: &mut WorldCollision<'a>, asked: &Asked) -> GroundAnswer<'a> {
    let max_slope = asked.slope.to_radians();
    match asked.z {
        Some(z) => world.ground(
            Vec3::new(asked.x, asked.y, z),
            &GroundProbe {
                max_rise: asked.rise,
                max_drop: f32::INFINITY,
                max_slope,
            },
        ),
        None => world.highest_ground(asked.x, asked.y, max_slope),
    }
}

/// The position asked about and how far it was looked from, in words.
fn asked_in_words(asked: &Asked) -> String {
    match asked.z {
        Some(z) => format!(
            "({}, {}, {z}), from {} m above it down",
            asked.x, asked.y, asked.rise
        ),
        None => format!("({}, {}), the whole of its column", asked.x, asked.y),
    }
}

/// A `phMaterialMgr::Id` word, the material type in its low byte, and the name the install gives that type.
fn material_parts(id: u64, palette: &MaterialPalette) -> (String, u8, Option<&str>) {
    let kind = id as u8;
    (format!("{id:#018x}"), kind, palette.name_for(kind))
}

fn file_line(label: &str, bound: &Bound) -> String {
    format!("    {label:<9} {}  {}", bound.name, bound.logical)
}

/// What `world ground` prints of an answer.
pub(super) fn ground_lines(
    asked: &Asked,
    answer: &GroundAnswer<'_>,
    palette: &MaterialPalette,
    notes: &[String],
) -> Vec<String> {
    let mut lines = Vec::new();
    match &answer.ground {
        Some(ground) => {
            let (at, normal) = (ground.hit.position, ground.hit.normal);
            lines.push(format!(
                "Ground under {}: {:.3}",
                asked_in_words(asked),
                at.z
            ));
            lines.push(format!(
                "  at ({:.3}, {:.3}, {:.3}), normal ({:.3}, {:.3}, {:.3})",
                at.x, at.y, at.z, normal.x, normal.y, normal.z
            ));
            lines.push(match ground.hit.material {
                Some(id) => {
                    let (id, kind, name) = material_parts(id, palette);
                    let name = name.map(|name| format!(" ({name})")).unwrap_or_default();
                    format!("  material {id}, type {kind}{name}")
                }
                None => "  material none: the polygon's bound lists no material".to_string(),
            });
            lines.push(match ground.bound {
                Some(bound) => format!("  from {}  {}", bound.name, bound.logical),
                None => "  from a collider its caller placed".to_string(),
            });
        }
        None => lines.push(format!("No ground under {}", asked_in_words(asked))),
    }
    let streamed = &answer.streamed;
    lines.push(format!(
        "  looked in {} bounds file(s); {} did not read; {} not loaded, which an interior places",
        streamed.held.len(),
        streamed.unread.len(),
        streamed.unplaced.len()
    ));
    lines.extend(streamed.held.iter().map(|bound| file_line("asked", bound)));
    for (bound, why) in &streamed.unread {
        lines.push(format!("{}: {why}", file_line("unread", bound)));
    }
    let unplaced = streamed.unplaced.iter();
    lines.extend(unplaced.map(|bound| file_line("unplaced", bound)));
    lines.extend(notes.iter().map(|note| format!("  note: {note}")));
    lines
}

pub(super) fn ground_json(
    asked: &Asked,
    answer: &GroundAnswer<'_>,
    palette: &MaterialPalette,
    notes: &[String],
) -> serde_json::Value {
    let file = |bound: &Bound| json!({ "name": bound.name, "logical": bound.logical });
    let ground = answer.ground.as_ref().map(|ground| {
        let (at, normal) = (ground.hit.position, ground.hit.normal);
        let material = ground.hit.material.map(|id| {
            let (id, kind, name) = material_parts(id, palette);
            // A string, because the word is 64 bits and a JSON number holds 53.
            json!({ "id": id, "type": kind, "name": name })
        });
        json!({
            "height": at.z,
            "position": [at.x, at.y, at.z],
            "normal": [normal.x, normal.y, normal.z],
            "material": material,
            "bound": ground.bound.map(file),
        })
    });
    let streamed = &answer.streamed;
    let unread: Vec<_> = (streamed.unread.iter())
        .map(|(bound, why)| {
            json!({ "name": bound.name, "logical": bound.logical, "why": why.to_string() })
        })
        .collect();
    json!({
        "at": [asked.x, asked.y],
        "from": asked.z,
        "rise": asked.z.map(|_| asked.rise),
        "slope": asked.slope,
        "ground": ground,
        "lookedIn": streamed.held.iter().map(|bound| file(bound)).collect::<Vec<_>>(),
        "unread": unread,
        "notLoaded": streamed.unplaced.iter().map(|bound| file(bound)).collect::<Vec<_>>(),
        "notes": notes,
    })
}

/// What `world ground` says of `asked` over `game`, mounted from `root`, with its index kept in `kept`: its lines, its JSON, and why the index was not kept where it was not.
pub(super) fn said(
    game: &MountedGame,
    root: &Path,
    kept: &Path,
    fresh: bool,
    asked: &Asked,
    mode: Option<MapMode>,
    palette: &MaterialPalette,
) -> (Vec<String>, serde_json::Value, Option<std::io::Error>) {
    let (index, report) = super::world_index::indexed(game, root, kept, fresh);
    let source = MountedWorld::new(game, root);
    let mut world = WorldCollision::new(&index, &source, BUDGET);
    let answer = ground_of(&mut world, asked);
    let notes = mount_notes(game, mode);
    (
        ground_lines(asked, &answer, palette, &notes),
        ground_json(asked, &answer, palette, &notes),
        report.not_kept,
    )
}

pub(crate) fn cmd_ground(args: &[String]) -> ExitCode {
    let usage = |e: &str| {
        eprintln!("Error: {USAGE}: {e}");
        ExitCode::from(2)
    };
    let asked = match asked_of(args) {
        Ok(asked) => asked,
        Err(e) => return usage(&e),
    };
    let (cutoff, options) = match mount_of(args) {
        Ok(mount) => mount,
        Err(e) => return usage(&e),
    };
    let Some(game) = flag_value(args, "--game") else {
        return usage("--game <dir> names the install");
    };
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = match std::fs::canonicalize(game) {
        Ok(root) => root,
        Err(e) => return usage(&format!("{game}: {e}")),
    };
    let kept = match flag_value(args, "--kept") {
        Some(file) => PathBuf::from(file),
        None => match super::world_index::default_kept(&root, flag_value(args, "--level")) {
            Some(file) => file,
            None => return usage(
                "no configuration directory to keep the index in: name a file with --kept <file>",
            ),
        },
    };
    let keys = flag_value(args, "--keys").map(Path::new);
    let (mount, crypto) = match super::keys::mount_at(&root, keys, cutoff, options) {
        Ok(mounted) => mounted,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // A material's name is the install's own, so with no table that reads it is left unsaid.
    let install = or_title::GtaInstall { root: root.clone() };
    let palette = match or_title::load_material_palette_from_gta_install(&install, crypto) {
        Ok(palette) => palette,
        Err(e) => {
            eprintln!("note: materials are not named: {e}");
            MaterialPalette::empty().clone()
        }
    };
    let fresh = args.iter().any(|a| a == "--fresh");
    let (lines, json, not_kept) = said(&mount, &root, &kept, fresh, &asked, options.mode, &palette);
    if let Some(error) = not_kept {
        eprintln!("warning: the index was not kept: {error}");
    }
    if args.iter().any(|a| a == "--json") {
        println!("{json}");
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
#[path = "world_ground_tests.rs"]
mod tests;
