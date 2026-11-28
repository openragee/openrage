//! `openrage-cli world select`: what the streamed world of a GTA V install takes for a view at a
//! place, with no screen: the maps wanted, the instances the level-of-detail rules select and the
//! files those need.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glam::Vec3;
use or_title::{MapMode, MountedGame};
use or_world::streaming::{Level, NodeState, View};
use or_world_gta5::world_forest::{Instance, Model, Selected, Streamed, Why};
use or_world_gta5::WorldState;
use or_world_gta5::{Catalogue, Kind, MountedWorld, WorldForest, WorldIndex, WorldSource};
use serde_json::json;

use super::world::flag_value;

/// The flags of this verb that take a value, so the value is not read as the install or a coordinate.
const VALUED_FLAGS: &[&str] = &[
    "--game",
    "--keys",
    "--kept",
    "--level",
    "--mode",
    "--distance",
    "--hour",
    "--weather",
];

/// The flags of this verb that take no value.
const PLAIN_FLAGS: &[&str] = &["--json", "--fresh"];

/// The bytes of maps one view may hold let go of: the verb asks one view, so none is.
const BUDGET: u64 = 0;

/// The hour of the day a view is taken at with no `--hour`.
const HOUR: u8 = 12;

/// The weather a view is taken in with no `--weather`.
const WEATHER: &str = "EXTRASUNNY";

const USAGE: &str = "world select <install> <x,y,z> [--distance <m>] [--hour <0-23>] \
                     [--weather <name>] [--level <pack>] [--mode <story|multiplayer>] \
                     [--kept <file>] [--keys <file>] [--fresh] [--json]";

/// What each level of detail is called, by `Level as usize`.
pub(super) const LEVELS: [&str; Level::COUNT] =
    ["HD", "LOD", "SLOD1", "SLOD2", "SLOD3", "ORPHAN_HD", "SLOD4"];

/// The view a person asked about.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Asked {
    pub at: Vec3,
    /// Past this many metres from the place an instance is counted and not listed; `None` lists all.
    pub distance: Option<f32>,
    /// 0 to 23.
    pub hour: u8,
    pub weather: String,
}

/// The install `args` name and the view they ask about: the install is `--game`'s value, else the first argument that is no flag.
pub(super) fn asked_of(args: &[String]) -> Result<(String, Asked), String> {
    // ⚠ A flag this verb does not know is refused: passed over, its value would be read as the
    // install or as a coordinate.
    let known = |flag: &str| VALUED_FLAGS.contains(&flag) || PLAIN_FLAGS.contains(&flag);
    // A coordinate may be negative, so a leading dash does not make an argument a flag.
    let flags = args.iter().filter(|arg| arg.starts_with("--"));
    if let Some(unknown) = flags
        .map(|arg| arg.split_once('=').map_or(arg.as_str(), |(flag, _)| flag))
        .find(|flag| !known(flag))
    {
        return Err(format!("`{unknown}` is no flag of this verb"));
    }
    if let Some(flag) = (args.last()).filter(|arg| VALUED_FLAGS.contains(&arg.as_str())) {
        return Err(format!("`{flag}` takes a value"));
    }
    let (install, rest) = super::world_index::install_of(args, VALUED_FLAGS)?;
    let numbers = super::world_index::numbers_of(&rest, VALUED_FLAGS)?;
    let [x, y, z] = numbers[..] else {
        return Err(format!(
            "a place is three numbers, x,y,z, and {} were given",
            numbers.len()
        ));
    };
    let distance = match flag_value(args, "--distance") {
        Some(word) => Some(
            (word.parse::<f32>().ok())
                .filter(|metres| *metres >= 0.0)
                .ok_or_else(|| format!("--distance is metres, 0 or more, not `{word}`"))?,
        ),
        None => None,
    };
    let hour = match flag_value(args, "--hour") {
        Some(word) => (word.parse::<u8>().ok())
            .filter(|hour| *hour < 24)
            .ok_or_else(|| format!("--hour is an hour of the day, 0 to 23, not `{word}`"))?,
        None => HOUR,
    };
    let weather = flag_value(args, "--weather").unwrap_or(WEATHER);
    let asked = Asked {
        at: Vec3::new(x, y, z),
        distance,
        hour,
        weather: weather.to_ascii_uppercase(),
    };
    Ok((install, asked))
}

/// What a view came to: the maps streamed for it, what was selected of them within the distance asked, and how many instances lay past it.
pub(super) struct Answer<'a> {
    pub streamed: Streamed<'a>,
    pub selected: Selected,
    pub past_distance: usize,
    /// Entities of the streamed maps that state no level, and those that name a parent in a map the mount lacks.
    pub misfits: (u64, u64),
}

/// The view `asked` about, streamed and selected over `forest` with every model taken as loaded.
pub(super) fn answer_of<'a>(forest: &mut WorldForest<'a>, asked: &Asked) -> Answer<'a> {
    let view = View::at(asked.at, 1.0);
    let state = WorldState::at(asked.hour, &asked.weather);
    let streamed = forest.stream(&view, &state);
    let mut selected = forest.select(&view, &|_| NodeState::Ready);
    let all = selected.instances.len();
    if let Some(distance) = asked.distance {
        let away = |instance: &Instance| asked.at.distance(instance.to_world.translation.into());
        selected.instances.retain(|i| away(i) <= distance);
    }
    Answer {
        streamed,
        past_distance: all - selected.instances.len(),
        selected,
        misfits: forest.misfits(),
    }
}

/// A hash as a script writes one.
fn hex(hash: u32) -> String {
    format!("{hash:#010X}")
}

/// The name of the map a hash names, else the hash.
fn map_name(index: &WorldIndex, hash: u32) -> String {
    index
        .map(hash)
        .map_or_else(|| hex(hash), |m| m.name.clone())
}

/// The logical path of the map file whose stem hashes to `hash`, which is how `world find` and `maps edit-game` name a map file.
fn map_logical(index: &WorldIndex, hash: u32) -> Option<&str> {
    index.map(hash).map(|map| map.logical.as_str())
}

fn why_in_words(index: &WorldIndex, why: Why) -> String {
    match why {
        Why::Own => "the view is within its box".to_string(),
        Why::ParentOf(child) => format!("the parent of {}", map_name(index, child)),
    }
}

/// One file a selection needs: what it is, the name it is asked by, and where the mount holds it.
struct Needs<'a> {
    kind: &'static str,
    hash: u32,
    found: Option<&'a or_world_gta5::Located>,
}

impl Needs<'_> {
    /// The file's stem where the mount holds it, else its hash.
    fn name(&self) -> String {
        (self.found).map_or_else(|| hex(self.hash), |found| found.stem.clone())
    }

    fn logical(&self) -> Option<&str> {
        self.found.map(|found| found.logical.as_str())
    }
}

/// The files `selected` needs, each kind in the order of its names' hashes.
fn needs<'a>(selected: &Selected, catalogue: &'a Catalogue) -> Vec<Needs<'a>> {
    let files = selected.files();
    // ⚠ A stem may be a drawable and a fragment both; the archetype's asset type says which is
    // meant, and the index does not keep it, so the drawable is named first.
    let loose = files.drawables.iter().map(|&hash| {
        let found = catalogue.find_any(hash, &[Kind::Drawable, Kind::Fragment]);
        let kind = match found.map(|found| found.kind) {
            Some(Kind::Fragment) => "fragment",
            _ => "drawable",
        };
        Needs { kind, hash, found }
    });
    let of = |kind: &'static str, what: Kind, hash: u32| Needs {
        kind,
        hash,
        found: catalogue.find(hash, what),
    };
    let dictionaries = (files.drawable_dictionaries.iter())
        .map(|&hash| of("drawable dictionary", Kind::DrawableDict, hash));
    let textures = (files.texture_dictionaries.iter())
        .map(|&hash| of("texture dictionary", Kind::TextureDict, hash));
    loose.chain(dictionaries).chain(textures).collect()
}

/// The instances of `selected` that stand as a box.
fn boxes(selected: &Selected) -> impl Iterator<Item = (&Instance, String)> {
    selected.instances.iter().filter_map(|i| match i.model {
        Model::Box(why) => Some((i, why.to_string())),
        Model::Files(_) => None,
    })
}

/// Where an instance is placed, in words.
fn placed_in_words(index: &WorldIndex, instance: &Instance) -> String {
    let map = map_name(index, instance.map);
    match instance.interior {
        Some(interior) => format!(
            "entity {} of the interior that entity {interior} of {map} places",
            instance.entity
        ),
        None => format!("entity {} of {map}", instance.entity),
    }
}

/// What `world select` prints of an answer.
pub(super) fn select_lines(
    asked: &Asked,
    answer: &Answer<'_>,
    index: &WorldIndex,
    catalogue: &Catalogue,
    notes: &[String],
) -> Vec<String> {
    let (streamed, selected) = (&answer.streamed, &answer.selected);
    let mut lines = vec![format!(
        "View at ({}, {}, {}), {} h, {}: {} map(s) wanted, {} that gave no entities; {} interior(s) brought in, {} that gave none",
        asked.at.x,
        asked.at.y,
        asked.at.z,
        asked.hour,
        asked.weather,
        streamed.wanted.len(),
        streamed.unread.len(),
        streamed.interiors.len(),
        streamed.interiors_unread.len()
    )];
    for wanted in &streamed.wanted {
        lines.push(format!(
            "  map       {}  {}  {}",
            wanted.map.name,
            wanted.map.logical,
            why_in_words(index, wanted.why)
        ));
    }
    for (map, why) in &streamed.unread {
        lines.push(format!("  unread    {}  {}: {why}", map.name, map.logical));
    }
    for at in &streamed.interiors {
        lines.push(format!(
            "  interior  {}  entity {} of {}",
            hex(at.archetype),
            at.entity,
            map_name(index, at.map)
        ));
    }
    for (at, why) in &streamed.interiors_unread {
        lines.push(format!(
            "  interior  {}  entity {} of {}: {why}",
            hex(at.archetype),
            at.entity,
            map_name(index, at.map)
        ));
    }
    let past = match asked.distance {
        Some(distance) => format!(
            " within {distance} m, {} more past it",
            answer.past_distance
        ),
        None => String::new(),
    };
    lines.push(format!(
        "  {} instance(s) selected{past}, every model taken as loaded",
        selected.instances.len()
    ));
    let by_level = selected.by_level();
    let counts: Vec<String> = (LEVELS.iter().zip(by_level))
        .map(|(level, count)| format!("{level} {count}"))
        .collect();
    lines.push(format!("    {}", counts.join(", ")));
    lines.push(format!(
        "  {} of them stand(s) as a box",
        boxes(selected).count()
    ));
    for (instance, why) in boxes(selected) {
        lines.push(format!(
            "    box       {}  {}: {why}",
            hex(instance.archetype),
            placed_in_words(index, instance)
        ));
    }
    let needed = needs(selected, catalogue);
    lines.push(format!(
        "  {} file(s) needed, {} that the mount lacks",
        needed.len(),
        needed.iter().filter(|file| file.found.is_none()).count()
    ));
    for file in &needed {
        lines.push(format!(
            "    {:<19}  {}  {}",
            file.kind,
            file.name(),
            file.logical().unwrap_or("not in the mount")
        ));
    }
    let (unleveled, lost) = answer.misfits;
    if unleveled + lost > 0 {
        lines.push(format!(
            "  of the maps' entities, {unleveled} state(s) no level and stand(s) as an orphan; {lost} name(s) a parent in a map the mount lacks and stand(s) alone"
        ));
    }
    lines.extend(notes.iter().map(|note| format!("  note: {note}")));
    lines
}

pub(super) fn select_json(
    asked: &Asked,
    answer: &Answer<'_>,
    index: &WorldIndex,
    catalogue: &Catalogue,
    notes: &[String],
) -> serde_json::Value {
    let (streamed, selected) = (&answer.streamed, &answer.selected);
    let maps: Vec<_> = (streamed.wanted.iter())
        .map(|wanted| {
            json!({
                "name": wanted.map.name,
                "logical": wanted.map.logical,
                "why": why_in_words(index, wanted.why),
            })
        })
        .collect();
    let unread: Vec<_> = (streamed.unread.iter())
        .map(|(map, why)| json!({ "name": map.name, "logical": map.logical, "why": why.to_string() }))
        .collect();
    let interior = |at: &or_world_gta5::world_forest::InteriorAt, why: Option<String>| {
        json!({
            "archetype": hex(at.archetype),
            "map": map_name(index, at.map),
            "logical": map_logical(index, at.map),
            "entity": at.entity,
            "why": why,
        })
    };
    let interiors: Vec<_> = (streamed.interiors.iter())
        .map(|at| interior(at, None))
        .collect();
    let interiors_unread: Vec<_> = (streamed.interiors_unread.iter())
        .map(|(at, why)| interior(at, Some(why.to_string())))
        .collect();
    let by_level: serde_json::Map<String, serde_json::Value> = (LEVELS.iter())
        .zip(selected.by_level())
        .map(|(level, count)| (level.to_string(), json!(count)))
        .collect();
    let boxes: Vec<_> = boxes(selected)
        .map(|(instance, why)| {
            json!({
                "archetype": hex(instance.archetype),
                "map": map_name(index, instance.map),
                "logical": map_logical(index, instance.map),
                "entity": instance.entity,
                "interior": instance.interior,
                "placed": placed_in_words(index, instance),
                "why": why,
            })
        })
        .collect();
    let files: Vec<_> = needs(selected, catalogue)
        .iter()
        .map(|file| json!({ "kind": file.kind, "name": file.name(), "logical": file.logical() }))
        .collect();
    json!({
        "at": [asked.at.x, asked.at.y, asked.at.z],
        "hour": asked.hour,
        "weather": asked.weather,
        "distance": asked.distance,
        "maps": maps,
        "unread": unread,
        "interiors": interiors,
        "interiorsUnread": interiors_unread,
        "instances": {
            "selected": selected.instances.len(),
            "pastDistance": asked.distance.map(|_| answer.past_distance),
            "byLevel": by_level,
            "boxes": boxes,
        },
        "files": files,
        "noLevel": answer.misfits.0,
        "parentMapLacking": answer.misfits.1,
        "notes": notes,
    })
}

/// What a level and a mode leave out of what this verb reads, in its words.
const MAPS_LEFT_OUT: super::world_ground::LeftOutWords = super::world_ground::LeftOutWords {
    with_a_pack: "their maps",
    none_of: "map",
    every: "one",
};

/// What `world select` says of `asked` over `game`, mounted from `root`, with its index kept in `kept`: its lines, its JSON, and why the index was not kept where it was not.
pub(super) fn said(
    game: &MountedGame,
    root: &Path,
    kept: &Path,
    fresh: bool,
    asked: &Asked,
    mode: Option<MapMode>,
) -> (Vec<String>, serde_json::Value, Option<std::io::Error>) {
    let (index, report) = super::world_index::indexed(game, root, kept, fresh);
    let source = MountedWorld::new(game, root);
    let mut catalogue = Catalogue::new();
    for archive in 0..source.archives().len() {
        source.entries(archive, &mut |facts| catalogue.index(facts));
    }
    let mut forest = WorldForest::new(&index, &source, BUDGET);
    let answer = answer_of(&mut forest, asked);
    let notes = super::world_ground::mount_notes(game, mode, &MAPS_LEFT_OUT);
    (
        select_lines(asked, &answer, &index, &catalogue, &notes),
        select_json(asked, &answer, &index, &catalogue, &notes),
        report.not_kept,
    )
}

pub(crate) fn cmd_select(args: &[String]) -> ExitCode {
    let usage = |e: &str| {
        eprintln!("Error: {USAGE}: {e}");
        ExitCode::from(2)
    };
    let (game, asked) = match asked_of(args) {
        Ok(asked) => asked,
        Err(e) => return usage(&e),
    };
    let (cutoff, options) = match super::world_ground::mount_of(args) {
        Ok(mount) => mount,
        Err(e) => return usage(&e),
    };
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = match std::fs::canonicalize(&game) {
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
    let (mount, _) = match super::keys::mount_at(&root, keys, cutoff, options) {
        Ok(mounted) => mounted,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let fresh = args.iter().any(|a| a == "--fresh");
    let (lines, json, not_kept) = said(&mount, &root, &kept, fresh, &asked, options.mode);
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
#[path = "world_select_tests.rs"]
mod tests;
