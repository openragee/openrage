//! `openrage-cli world index` and `world at`: the world index of a GTA V install, built or
//! refreshed from the file it is kept in, and asked what lies at a position.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glam::Vec3;
use or_depends::world_index::{refresh, KeptState, Report};
use or_depends::{MountedWorld, Via, WorldIndex};
use or_title::MountedGame;
use serde_json::json;

use super::world::flag_value;

/// The flags of these two verbs that take a value, so the value is not read as a coordinate.
const VALUED_FLAGS: &[&str] = &["--game", "--keys", "--kept"];

/// The folder under the configuration directory that holds an index an install.
const KEPT_FOLDER: &str = "gta5_world_index";

/// Where the index of the install at `root` is kept, with no `--kept`: a file named for the
/// install's path; `None` with no configuration directory.
fn default_kept(root: &Path) -> Option<PathBuf> {
    let name = or_io::gta6::hash::name_hash64(root.to_string_lossy().as_bytes());
    let folder = or_title::config::config_dir()?.join(KEPT_FOLDER);
    Some(folder.join(format!("{name:016x}.index")))
}

/// The index of `game`, mounted from `root`: from `kept` where the install stands as it did, and
/// read whole, ignoring what is kept, when `fresh` says so.
fn indexed(game: &MountedGame, root: &Path, kept: &Path, fresh: bool) -> (WorldIndex, Report) {
    let source = MountedWorld::new(game, root);
    if !fresh {
        return refresh(&source, kept);
    }
    let (index, mut report) = WorldIndex::build(&source, None);
    report.not_kept = index.keep(kept).err();
    (index, report)
}

/// Mount the install `--game` names and index it, as `--kept`, `--keys` and `--fresh` say.
fn open(args: &[String]) -> Result<(WorldIndex, Report, PathBuf, PathBuf), String> {
    let game = flag_value(args, "--game").ok_or("--game <dir> names the install")?;
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = std::fs::canonicalize(game).map_err(|e| format!("{game}: {e}"))?;
    let kept = match flag_value(args, "--kept") {
        Some(file) => PathBuf::from(file),
        None => default_kept(&root).ok_or(
            "no configuration directory to keep the index in: name a file with --kept <file>",
        )?,
    };
    let mount = super::keys::mount_all(&root, flag_value(args, "--keys").map(Path::new))?;
    let fresh = args.iter().any(|a| a == "--fresh");
    let (index, report) = indexed(&mount, &root, &kept, fresh);
    Ok((index, report, root, kept))
}

/// What became of the kept index, in words.
fn kept_in_words(kept: &KeptState) -> String {
    match kept {
        KeptState::Absent => "none was read, so every archive was".to_string(),
        KeptState::Current => "of the install as it stands, so nothing was read".to_string(),
        KeptState::Changed => {
            "of the install as it was, so each archive that changed was read again".to_string()
        }
        KeptState::Foreign => "of another install, so every archive was read".to_string(),
        KeptState::Refused(why) => format!("refused ({why}), so every archive was read"),
    }
}

/// How many of `all` are not `read`.
fn unread<T>(all: &[T], read: impl Fn(&T) -> bool) -> usize {
    all.iter().filter(|one| !read(one)).count()
}

/// What `world index` prints of an index and of the build that made it.
fn index_lines(index: &WorldIndex, report: &Report, root: &Path, kept: &Path) -> Vec<String> {
    let maps = index.maps();
    let parents = maps.iter().filter(|map| map.parent != 0);
    let (held, lacked): (Vec<_>, Vec<_>) = parents.partition(|map| index.parent_of(map).is_some());
    let (manifests, manifests_unread) = index.manifests();
    let tables = index.tables();
    let mut lines = vec![
        format!("World index of {}", root.display()),
        format!(
            "  kept in {}: {}",
            kept.display(),
            kept_in_words(&report.kept)
        ),
        format!(
            "  {} archive(s), {} read, {} file(s) opened",
            report.archives,
            report.read.len(),
            report.opened
        ),
    ];
    lines.extend(
        report
            .read
            .iter()
            .map(|container| format!("    read {container}")),
    );
    if let Some(error) = &report.not_kept {
        lines.push(format!("  warning: the index was not kept: {error}"));
    }
    lines.extend([
        format!(
            "  {} map(s), {} that did not read as one; {} name a parent the install holds, {} one it lacks",
            maps.len(),
            unread(maps, |map| map.read),
            held.len(),
            lacked.len()
        ),
        format!(
            "  {} type file(s), {} that did not read as one; {} archetype(s), {} declared again by a later file",
            index.type_files().len(),
            unread(index.type_files(), |file| file.read),
            index.archetype_count(),
            index.redefined()
        ),
        format!(
            "  {manifests} manifest(s), {manifests_unread} that did not read as one; bounds named for {} map group(s) and {} interior(s)",
            tables.static_bound_groups(),
            tables.bounded_interiors().count()
        ),
        format!(
            "  {} bound(s), {} with no box read; {} carried by a map group, a map's name or an interior",
            index.bounds().len(),
            unread(index.bounds(), |bound| bound.span.is_some()),
            index.carried_bounds()
        ),
        format!("  {} cop(ies) shadowed by a later archive's", index.shadowed()),
    ]);
    lines
}

fn index_json(index: &WorldIndex, report: &Report, root: &Path, kept: &Path) -> serde_json::Value {
    let (manifests, manifests_unread) = index.manifests();
    let state = match &report.kept {
        KeptState::Absent => "absent",
        KeptState::Current => "current",
        KeptState::Changed => "changed",
        KeptState::Foreign => "foreign",
        KeptState::Refused(_) => "refused",
    };
    let refused = match &report.kept {
        KeptState::Refused(why) => Some(why.to_string()),
        _ => None,
    };
    json!({
        "install": root.display().to_string(),
        "keptIn": kept.display().to_string(),
        "kept": state,
        "refused": refused,
        "notKept": report.not_kept.as_ref().map(ToString::to_string),
        "archives": report.archives,
        "read": report.read,
        "opened": report.opened,
        "maps": index.maps().len(),
        "mapsUnread": unread(index.maps(), |map| map.read),
        "typeFiles": index.type_files().len(),
        "typeFilesUnread": unread(index.type_files(), |file| file.read),
        "archetypes": index.archetype_count(),
        "archetypesRedefined": index.redefined(),
        "manifests": manifests,
        "manifestsUnread": manifests_unread,
        "mapGroupsWithBounds": index.tables().static_bound_groups(),
        "interiorsWithBounds": index.tables().bounded_interiors().count(),
        "bounds": index.bounds().len(),
        "boundsUnread": unread(index.bounds(), |bound| bound.span.is_some()),
        "boundsCarried": index.carried_bounds(),
        "shadowed": index.shadowed(),
    })
}

pub(crate) fn cmd_index(args: &[String]) -> ExitCode {
    let (index, report, root, kept) = match open(args) {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("Error: world index --game <dir> [--kept <file>] [--fresh]: {e}");
            return ExitCode::from(2);
        }
    };
    if args.iter().any(|a| a == "--json") {
        println!("{}", index_json(&index, &report, &root, &kept));
    } else {
        for line in index_lines(&index, &report, &root, &kept) {
            println!("{line}");
        }
    }
    ExitCode::SUCCESS
}

/// The position, in metres, that the arguments of `args` that are neither a flag nor a flag's value give: three numbers, or one argument that writes a place as a script does.
fn position_of(args: &[String]) -> Result<Vec3, String> {
    let mut numbers = Vec::new();
    let mut skip = false;
    let mut words = Vec::new();
    for arg in args {
        match skip || VALUED_FLAGS.contains(&arg.as_str()) {
            true => skip = !skip,
            false if !arg.starts_with("--") => words.push(arg.as_str()),
            false => {}
        }
    }
    // One argument that is no lone number is a place as `world new-map --at` and a script write one, read as a pasted place is.
    if let [written] = words[..] {
        use or_maps::place::{read_pasted, Pasted};
        match read_pasted(written) {
            Ok(Pasted::Place(place)) => {
                return match place.z {
                    Some(z) => Ok(Vec3::new(place.x, place.y, z)),
                    None => Err(format!(
                        "`{written}` gives no height, and a position is x, y and z"
                    )),
                }
            }
            Err(refused) => return Err(refused.to_string()),
            Ok(_) => {}
        }
    }
    skip = false;
    for arg in args {
        if skip {
            skip = false;
        } else if VALUED_FLAGS.contains(&arg.as_str()) {
            skip = true;
        } else if let Some(number) = arg.parse::<f32>().ok().filter(|n| n.is_finite()) {
            // A coordinate may be negative, so a leading dash does not make an argument a flag.
            numbers.push(number);
        } else if !arg.starts_with("--") {
            // ⚠ `nan` and `inf` parse as numbers and are no place.
            return Err(format!("`{arg}` is not a number"));
        }
    }
    match numbers[..] {
        [x, y, z] => Ok(Vec3::new(x, y, z)),
        _ => Err(format!(
            "a position is three numbers, x y z, and {} were given",
            numbers.len()
        )),
    }
}

/// The name of the map a hash names, else the hash.
fn named(index: &WorldIndex, hash: u32) -> String {
    match index.map(hash) {
        Some(map) => map.name.clone(),
        None => format!("{hash:#010X}"),
    }
}

/// Why a bound is said to be carried, in words.
fn via_in_words(via: Via) -> &'static str {
    match via {
        Via::ManifestMapDataGroup => "a manifest's map group",
        Via::ManifestInterior => "a manifest's interior",
        Via::MapStemConvention {
            inside_streaming_box: Some(false),
        } => "its filename, and it lies outside that map's streaming extents",
        _ => "its filename",
    }
}

/// What `world at` prints of a position.
fn at_lines(index: &WorldIndex, at: Vec3) -> Vec<String> {
    let maps = index.maps_at(at);
    let bounds = index.bounds_at(at);
    let mut lines = vec![format!(
        "At ({}, {}, {}): {} map(s) whose streaming extents hold it, {} bound(s) whose box does",
        at.x,
        at.y,
        at.z,
        maps.len(),
        bounds.len()
    )];
    for map in maps {
        let parent = match map.parent {
            0 => String::new(),
            parent => format!("  parent {}", named(index, parent)),
        };
        lines.push(format!(
            "  map    {}  {}{parent}  flags {:#x}  contentFlags {:#x}",
            map.name, map.logical, map.flags, map.content_flags
        ));
    }
    for found in bounds {
        lines.push(format!(
            "  bound  {}  {}",
            found.bound.name, found.bound.logical
        ));
        for carrier in found.carriers {
            lines.push(format!(
                "           carried by {}, by {}",
                named(index, carrier.name),
                via_in_words(carrier.via)
            ));
        }
    }
    lines
}

fn at_json(index: &WorldIndex, at: Vec3) -> serde_json::Value {
    let maps: Vec<_> = (index.maps_at(at).iter())
        .map(|map| {
            json!({
                "name": map.name,
                "logical": map.logical,
                "parent": (map.parent != 0).then(|| named(index, map.parent)),
                "flags": map.flags,
                "contentFlags": map.content_flags,
            })
        })
        .collect();
    let bounds: Vec<_> = (index.bounds_at(at).iter())
        .map(|found| {
            let carriers: Vec<_> = (found.carriers.iter())
                .map(|c| json!({ "name": named(index, c.name), "by": via_in_words(c.via) }))
                .collect();
            json!({
                "name": found.bound.name,
                "logical": found.bound.logical,
                "carriedBy": carriers,
            })
        })
        .collect();
    json!({ "at": [at.x, at.y, at.z], "maps": maps, "bounds": bounds })
}

pub(crate) fn cmd_at(args: &[String]) -> ExitCode {
    let usage = "world at --game <dir> <x> <y> <z> [--kept <file>]";
    let at = match position_of(args) {
        Ok(at) => at,
        Err(e) => {
            eprintln!("Error: {usage}: {e}");
            return ExitCode::from(2);
        }
    };
    let (index, report, ..) = match open(args) {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("Error: {usage}: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(error) = &report.not_kept {
        eprintln!("warning: the index was not kept: {error}");
    }
    if args.iter().any(|a| a == "--json") {
        println!("{}", at_json(&index, at));
    } else {
        for line in at_lines(&index, at) {
            println!("{line}");
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
#[path = "world_index_tests.rs"]
mod tests;
