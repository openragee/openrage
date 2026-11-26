//! `openrage-cli world models` and `world vehicles`: the models and the vehicles of a GTA V install as a browser lists
//! them, a page at a time. The models are said from the world index alone; the vehicles from the vehicle data files the
//! mount would load.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use glam::Vec3;
use or_depends::find::Rank;
use or_depends::models::{Model, Page, Query, Sort};
use or_depends::vehicles::{VehiclePage, VehicleQuery};
use or_depends::{Kind, ModelCatalogue, VehicleCatalogue};
use or_title::{MapMode, MountedGame};
use or_world::ArchetypeKind;
use serde_json::json;

use super::world::flag_value;

const MODELS_USAGE: &str = "world models <install> [<text>] [--pack <device>] \
                            [--kind <base|time|interior>] [--min-size <m>] [--max-size <m>] \
                            [--placed | --unplaced] [--sort <name|size|placed>] [--desc] \
                            [--offset <n>] [--count <n>] [--level <pack>] \
                            [--mode <story|multiplayer>] [--kept <file>] [--keys <file>] \
                            [--fresh] [--json]";

const VEHICLES_USAGE: &str = "world vehicles <install> [<text>] [--class <VC_…>] \
                              [--type <VEHICLE_TYPE_…>] [--pack <device>] [--offset <n>] \
                              [--count <n>] [--level <pack>] [--mode <story|multiplayer>] \
                              [--keys <file>] [--json]";

/// The flags of `world models` that take a value, so the value is not read as the text.
const MODELS_VALUED: &[&str] = &[
    "--game",
    "--pack",
    "--kind",
    "--min-size",
    "--max-size",
    "--sort",
    "--offset",
    "--count",
    "--level",
    "--mode",
    "--kept",
    "--keys",
];

/// The flags of `world models` that take no value.
const MODELS_PLAIN: &[&str] = &["--placed", "--unplaced", "--desc", "--fresh", "--json"];

const VEHICLES_VALUED: &[&str] = &[
    "--game", "--class", "--type", "--pack", "--offset", "--count", "--level", "--mode", "--keys",
];

const VEHICLES_PLAIN: &[&str] = &["--json"];

/// The most rows of a page, with no `--count`.
const COUNT: usize = 50;

/// The install and the text of `args`, whose flags are `valued` and `plain`; a flag that is neither, or one left without its value, is refused.
fn words_of(
    args: &[String],
    valued: &[&str],
    plain: &[&str],
) -> Result<(String, Option<String>), String> {
    let mut words: Vec<&str> = Vec::new();
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        if valued.contains(&arg) {
            if at + 1 >= args.len() {
                return Err(format!("`{arg}` takes a value"));
            }
            at += 1;
        } else if arg.starts_with("--") {
            // ⚠ A flag this verb does not know is refused: passed over, it would be searched for as text.
            let joined = arg.split_once('=');
            let joined = joined.is_some_and(|(flag, _)| valued.contains(&flag));
            if !(joined || plain.contains(&arg)) {
                return Err(format!("`{arg}` is no flag of this verb"));
            }
        } else {
            // A signed hash may be negative, so one dash does not make a flag.
            words.push(arg);
        }
        at += 1;
    }
    // The install is `--game`'s value, else the first word, as every verb over an install takes it.
    let (install, text) = match (flag_value(args, "--game"), words.split_first()) {
        (Some(game), _) => (game, &words[..]),
        (None, Some((install, text))) => (*install, text),
        (None, None) => return Err("<install> names the install".to_string()),
    };
    let text = (!text.is_empty()).then(|| text.join(" "));
    Ok((install.to_string(), text))
}

/// The count `flag` gives, else `default`.
fn count_of(args: &[String], flag: &str, default: usize) -> Result<usize, String> {
    match flag_value(args, flag) {
        None => Ok(default),
        Some(word) => {
            (word.parse::<usize>()).map_err(|_| format!("{flag} is a count of rows, not `{word}`"))
        }
    }
}

/// The length in metres `flag` gives, where it is given.
fn metres_of(args: &[String], flag: &str) -> Result<Option<f32>, String> {
    let Some(word) = flag_value(args, flag) else {
        return Ok(None);
    };
    let metres = word.parse::<f32>().ok();
    let metres = metres.filter(|metres| metres.is_finite() && *metres >= 0.0);
    metres
        .map(Some)
        .ok_or_else(|| format!("{flag} is a length in metres, not `{word}`"))
}

/// What a person asked of the models, as the arguments give it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ModelsAsked {
    pub install: String,
    pub text: Option<String>,
    pub pack: Option<String>,
    pub kind: Option<ArchetypeKind>,
    pub size: (Option<f32>, Option<f32>),
    pub placed: Option<bool>,
    pub sort: Option<Sort>,
    pub descending: bool,
    pub offset: usize,
    pub count: usize,
}

impl ModelsAsked {
    fn query(&self) -> Query<'_> {
        Query {
            text: self.text.as_deref(),
            pack: self.pack.as_deref(),
            kind: self.kind,
            size: self.size,
            placed: self.placed,
            sort: self.sort,
            descending: self.descending,
            offset: self.offset,
            count: self.count,
        }
    }
}

fn kind_word(kind: ArchetypeKind) -> &'static str {
    match kind {
        ArchetypeKind::Base => "base",
        ArchetypeKind::Time => "time",
        ArchetypeKind::Mlo => "interior",
    }
}

/// The install, the text and the filters of `args`.
pub(super) fn models_asked(args: &[String]) -> Result<ModelsAsked, String> {
    let (install, text) = words_of(args, MODELS_VALUED, MODELS_PLAIN)?;
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let kind = match flag_value(args, "--kind") {
        None => None,
        Some("base") => Some(ArchetypeKind::Base),
        Some("time") => Some(ArchetypeKind::Time),
        Some("interior") => Some(ArchetypeKind::Mlo),
        Some(word) => return Err(format!("--kind is base, time or interior, not `{word}`")),
    };
    let sort = match flag_value(args, "--sort") {
        None => None,
        Some("name") => Some(Sort::Name),
        Some("size") => Some(Sort::Size),
        Some("placed") => Some(Sort::Placed),
        Some(word) => return Err(format!("--sort is name, size or placed, not `{word}`")),
    };
    let placed = match (has("--placed"), has("--unplaced")) {
        (true, true) => return Err("--placed and --unplaced leave no model; give one".into()),
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    let size = (
        metres_of(args, "--min-size")?,
        metres_of(args, "--max-size")?,
    );
    if let (Some(least), Some(most)) = size {
        if least > most {
            return Err(format!("--min-size {least} is past --max-size {most}"));
        }
    }
    Ok(ModelsAsked {
        install,
        text,
        pack: flag_value(args, "--pack").map(str::to_string),
        kind,
        size,
        placed,
        sort,
        descending: has("--desc"),
        offset: count_of(args, "--offset", 0)?,
        count: count_of(args, "--count", COUNT)?,
    })
}

/// What a level and a mode leave out of what `world models` reads, in its words.
const MODELS_LEFT_OUT: super::world_ground::LeftOutWords = super::world_ground::LeftOutWords {
    with_a_pack: "the models they declare",
    none_of: "type file",
    every: "map and type file",
};

fn hex(hash: u32) -> String {
    format!("0x{hash:08X}")
}

fn rank_word(rank: Rank) -> &'static str {
    match rank {
        Rank::Exact => "exact",
        Rank::Prefix => "prefix",
        Rank::PartPrefix => "part",
        Rank::Contained => "contained",
    }
}

fn model_named(model: &Model) -> String {
    model.name.clone().unwrap_or_else(|| hex(model.hash))
}

/// A dictionary as its name where a file of the install has it, else its hash; `None` where the model states none.
fn dictionary(catalogue: &ModelCatalogue, hash: u32) -> Option<String> {
    (hash != 0).then(|| match catalogue.dictionary_name(hash) {
        Some(name) => name.to_string(),
        None => hex(hash),
    })
}

/// What `world models` prints of a page.
fn models_lines(
    root: &Path,
    catalogue: &ModelCatalogue,
    asked: &ModelsAsked,
    page: &Page<'_>,
    notes: &[String],
) -> Vec<String> {
    let kinds = catalogue
        .kinds()
        .map(|(kind, count)| format!("{count} {}", kind_word(kind)));
    let mut lines = vec![
        format!("Models of {}", root.display()),
        format!(
            "  {} model(s): {}; {} named by no file and listed by their hash",
            catalogue.len(),
            kinds.join(", "),
            catalogue.unnamed()
        ),
        format!(
            "  {} answer, {} listed from {}",
            page.total,
            page.rows.len(),
            asked.offset
        ),
    ];
    for row in &page.rows {
        let model = row.model;
        let declared = catalogue.declared(model);
        let size = match model.size {
            Some(size) => format!("{size:.3} m"),
            None => "no box".to_string(),
        };
        let held = match catalogue.held(model) {
            Some(file) => format!("held by {}", file.logical),
            None => "held by no file".to_string(),
        };
        let placed = match (model.placed, model.inside) {
            (0, 0) => "never placed".to_string(),
            (placed, 0) => format!("placed {placed} time(s)"),
            (placed, inside) => {
                format!("placed {placed} time(s), {inside} inside an interior")
            }
        };
        let replaced = catalogue.replaced(model);
        let over = match replaced.len() {
            0 => String::new(),
            _ => {
                let packs: Vec<&str> = replaced.iter().map(|declared| declared.pack).collect();
                format!(", over {}", packs.join(", "))
            }
        };
        let rank = row
            .rank
            .map(|rank| format!("  {}", rank_word(rank)))
            .unwrap_or_default();
        let drawables = dictionary(catalogue, model.drawable_dictionary)
            .map(|name| format!("  drawables {name}"))
            .unwrap_or_default();
        let textures = dictionary(catalogue, model.texture_dictionary)
            .map(|name| format!("  textures {name}"))
            .unwrap_or_default();
        lines.push(format!(
            "  {}  {}  {}  {size}  lod {}  {placed}  {} in {}{over}  {held}{drawables}{textures}{rank}",
            model_named(model),
            hex(model.hash),
            kind_word(model.kind),
            model.lod_dist,
            declared.type_file.name,
            declared.pack,
        ));
    }
    lines.extend(notes.iter().map(|note| format!("  note: {note}")));
    lines
}

fn models_json(
    root: &Path,
    catalogue: &ModelCatalogue,
    asked: &ModelsAsked,
    page: &Page<'_>,
    notes: &[String],
) -> serde_json::Value {
    let xyz = |at: Vec3| json!([at.x, at.y, at.z]);
    let named = |hash: u32| {
        (hash != 0).then(|| json!({ "hash": hex(hash), "name": catalogue.dictionary_name(hash) }))
    };
    let declaration = |declared: &or_depends::models::Declared<'_>| {
        json!({
            "typeFile": declared.type_file.name,
            "logical": declared.type_file.logical,
            "pack": declared.pack,
        })
    };
    let rows: Vec<_> = (page.rows.iter())
        .map(|row| {
            let model = row.model;
            let held = catalogue.held(model).map(|file| {
                let entry = file.kind == Kind::DrawableDict;
                json!({ "extension": file.kind.extension(), "logical": file.logical, "entry": entry })
            });
            let replaced = catalogue.replaced(model);
            json!({
                "name": model.name,
                "hash": hex(model.hash),
                "kind": kind_word(model.kind),
                "declared": declaration(&catalogue.declared(model)),
                "replaced": replaced.iter().map(declaration).collect::<Vec<_>>(),
                "box": model.span.map(|(min, max)| json!({ "min": xyz(min), "max": xyz(max) })),
                "sphere": model.sphere.map(|(centre, radius)| {
                    json!({ "centre": xyz(centre), "radius": radius })
                }),
                "size": model.size,
                "lodDist": model.lod_dist,
                "drawableDictionary": named(model.drawable_dictionary),
                "textureDictionary": named(model.texture_dictionary),
                "held": held,
                "placed": model.placed,
                "inside": model.inside,
                "isPlaced": model.is_placed(),
                "rank": row.rank.map(rank_word),
            })
        })
        .collect();
    let kinds: serde_json::Map<String, serde_json::Value> = (catalogue.kinds().into_iter())
        .map(|(kind, count)| (kind_word(kind).to_string(), json!(count)))
        .collect();
    let packs: Vec<_> = (catalogue.packs().into_iter())
        .map(|(pack, models)| json!({ "pack": pack, "models": models }))
        .collect();
    json!({
        "install": root.display().to_string(),
        "models": catalogue.len(),
        "unnamed": catalogue.unnamed(),
        "kinds": kinds,
        "packs": packs,
        "text": asked.text,
        "total": page.total,
        "offset": asked.offset,
        "count": asked.count,
        "rows": rows,
        "notes": notes,
    })
}

/// What a verb of this file says.
pub(super) struct Said {
    pub lines: Vec<String>,
    pub json: serde_json::Value,
    /// Why the index was not kept, where it was not.
    pub not_kept: Option<std::io::Error>,
}

/// Answer `asked` over `game`, mounted from `root`, with its index kept in `kept`; an error where the text has the shape of a hash and is none.
pub(super) fn models_said(
    game: &MountedGame,
    root: &Path,
    kept: &Path,
    fresh: bool,
    asked: &ModelsAsked,
    mode: Option<MapMode>,
) -> Result<Said, String> {
    let (index, report) = super::world_index::indexed(game, root, kept, fresh);
    let built = Instant::now();
    let catalogue = ModelCatalogue::new(&index);
    let built = built.elapsed();
    let answered = Instant::now();
    let page = catalogue.page(&asked.query()).map_err(|e| e.to_string())?;
    let answered = answered.elapsed();
    let mut notes = super::world_ground::mount_notes(game, mode, &MODELS_LEFT_OUT);
    if let Some(pack) = &asked.pack {
        let packs = catalogue.packs();
        if !(packs.iter()).any(|(device, _)| device.eq_ignore_ascii_case(pack)) {
            let devices: Vec<&str> = packs.iter().map(|(device, _)| *device).collect();
            notes.push(format!(
                "the mount has no pack `{pack}`: its packs are {}",
                devices.join(", ")
            ));
        }
    }
    let mut lines = models_lines(root, &catalogue, asked, &page, &notes);
    let mut json = models_json(root, &catalogue, asked, &page, &notes);
    // How long the list took to put together from the index, and the question to answer: a measurement, said apart from the answer.
    let micros = |took: std::time::Duration| took.as_micros() as u64;
    lines.insert(
        3,
        format!(
            "  listed in {} µs from the index ({} file(s) opened for it), asked in {} µs",
            micros(built),
            report.opened,
            micros(answered)
        ),
    );
    json["timing"] = json!({
        "catalogueMicros": micros(built),
        "queryMicros": micros(answered),
        "filesOpened": report.opened,
    });
    Ok(Said {
        lines,
        json,
        not_kept: report.not_kept,
    })
}

/// Why a verb of this file did nothing: the arguments were wrong, or the install did not mount.
enum Stopped {
    Usage(String),
    Mount(String),
}

/// The install `args` name, mounted where they say it stops and in the mode they say, with its folder.
fn mounted(
    args: &[String],
    install: &str,
) -> Result<(MountedGame, PathBuf, Option<MapMode>), Stopped> {
    let (cutoff, options) = super::world_ground::mount_of(args).map_err(Stopped::Usage)?;
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = std::fs::canonicalize(install);
    let root = root.map_err(|e| Stopped::Usage(format!("{install}: {e}")))?;
    let keys = flag_value(args, "--keys").map(Path::new);
    let (game, _) = super::keys::mount_at(&root, keys, cutoff, options).map_err(Stopped::Mount)?;
    Ok((game, root, options.mode))
}

fn print(said: &Said, args: &[String]) {
    if let Some(error) = &said.not_kept {
        eprintln!("warning: the index was not kept: {error}");
    }
    if args.iter().any(|arg| arg == "--json") {
        println!("{}", said.json);
    } else {
        for line in &said.lines {
            println!("{line}");
        }
    }
}

pub(crate) fn cmd_models(args: &[String]) -> ExitCode {
    let usage = |e: &str| {
        eprintln!("Error: {MODELS_USAGE}: {e}");
        ExitCode::from(2)
    };
    let asked = match models_asked(args) {
        Ok(asked) => asked,
        Err(e) => return usage(&e),
    };
    let (game, root, mode) = match mounted(args, &asked.install) {
        Ok(mounted) => mounted,
        Err(Stopped::Usage(e)) => return usage(&e),
        Err(Stopped::Mount(e)) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
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
    let fresh = args.iter().any(|arg| arg == "--fresh");
    match models_said(&game, &root, &kept, fresh, &asked, mode) {
        Ok(said) => print(&said, args),
        Err(e) => return usage(&e),
    }
    ExitCode::SUCCESS
}

/// What a person asked of the vehicles, as the arguments give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VehiclesAsked {
    pub install: String,
    pub text: Option<String>,
    pub class: Option<String>,
    pub vehicle_type: Option<String>,
    pub pack: Option<String>,
    pub offset: usize,
    pub count: usize,
}

/// The install, the text and the filters of `args`.
pub(super) fn vehicles_asked(args: &[String]) -> Result<VehiclesAsked, String> {
    let (install, text) = words_of(args, VEHICLES_VALUED, VEHICLES_PLAIN)?;
    let flag = |flag: &str| flag_value(args, flag).map(str::to_string);
    Ok(VehiclesAsked {
        install,
        text,
        class: flag("--class"),
        vehicle_type: flag("--type"),
        pack: flag("--pack"),
        offset: count_of(args, "--offset", 0)?,
        count: count_of(args, "--count", COUNT)?,
    })
}

/// Each text of `tally` with its count, as `class: n`.
fn tallied(tally: &[(&str, usize)]) -> String {
    let said = tally.iter().map(|(text, count)| match text.is_empty() {
        true => format!("none stated: {count}"),
        false => format!("{text}: {count}"),
    });
    said.collect::<Vec<_>>().join(", ")
}

fn vehicles_lines(
    root: &Path,
    catalogue: &VehicleCatalogue,
    asked: &VehiclesAsked,
    page: &VehiclePage<'_>,
    notes: &[String],
) -> Vec<String> {
    let list = catalogue.list();
    let read = list.files.iter().filter(|file| file.read.is_ok()).count();
    let mut lines = vec![
        format!("Vehicles of {}", root.display()),
        format!(
            "  {} vehicle(s) from {} data file(s), {} that gave none",
            list.vehicles.len(),
            list.files.len(),
            list.files.len() - read
        ),
        format!("  classes: {}", tallied(&catalogue.classes())),
        format!("  types: {}", tallied(&catalogue.types())),
        format!(
            "  {} answer, {} listed from {}",
            page.total,
            page.rows.len(),
            asked.offset
        ),
    ];
    for row in &page.rows {
        let vehicle = &row.vehicle.vehicle;
        let over = match row.vehicle.replaced.len() {
            0 => String::new(),
            _ => {
                let packs = row.vehicle.replaced.iter();
                let packs: Vec<&str> = packs.map(|file| list.files[*file].pack.as_str()).collect();
                format!(", over {}", packs.join(", "))
            }
        };
        let rank = row
            .rank
            .map(|rank| format!("  {}", rank_word(rank)))
            .unwrap_or_default();
        let stated = |text: &str| match text.is_empty() {
            true => "none stated".to_string(),
            false => text.to_string(),
        };
        lines.push(format!(
            "  {}  {}  {}  {}  in {}{over}  {}{rank}",
            vehicle.model_name,
            stated(&vehicle.game_name),
            stated(&vehicle.class),
            stated(&vehicle.vehicle_type),
            row.file.pack,
            row.file.path,
        ));
    }
    for file in &list.files {
        if let Err(why) = &file.read {
            lines.push(format!(
                "  note: {} of {} gave no vehicle: {why}",
                file.path, file.pack
            ));
        }
    }
    lines.extend(notes.iter().map(|note| format!("  note: {note}")));
    lines
}

fn vehicles_json(
    root: &Path,
    catalogue: &VehicleCatalogue,
    asked: &VehiclesAsked,
    page: &VehiclePage<'_>,
    notes: &[String],
) -> serde_json::Value {
    let list = catalogue.list();
    let tally = |all: Vec<(&str, usize)>, key: &str| -> Vec<serde_json::Value> {
        let all = all.into_iter();
        all.map(|(text, count)| json!({ key: text, "vehicles": count }))
            .collect()
    };
    let files: Vec<_> = (list.files.iter())
        .map(|file| {
            json!({
                "pack": file.pack,
                "path": file.path,
                "vehicles": file.read.as_ref().ok(),
                "notRead": file.read.as_ref().err(),
            })
        })
        .collect();
    let rows: Vec<_> = (page.rows.iter())
        .map(|row| {
            let vehicle = &row.vehicle.vehicle;
            let replaced = row.vehicle.replaced.iter();
            let replaced: Vec<&str> = replaced
                .map(|file| list.files[*file].pack.as_str())
                .collect();
            json!({
                "modelName": vehicle.model_name,
                "gameName": vehicle.game_name,
                "class": vehicle.class,
                "type": vehicle.vehicle_type,
                "pack": row.file.pack,
                "file": row.file.path,
                "replaced": replaced,
                "rank": row.rank.map(rank_word),
            })
        })
        .collect();
    json!({
        "install": root.display().to_string(),
        "vehicles": list.vehicles.len(),
        "files": files,
        "classes": tally(catalogue.classes(), "class"),
        "types": tally(catalogue.types(), "type"),
        "packs": tally(catalogue.packs(), "pack"),
        "text": asked.text,
        "total": page.total,
        "offset": asked.offset,
        "count": asked.count,
        "rows": rows,
        "notes": notes,
    })
}

/// What a level and a mode leave out of what `world vehicles` reads, in its words.
fn vehicle_notes(game: &MountedGame, mode: Option<MapMode>) -> Vec<String> {
    let mut notes = Vec::new();
    let left_out = super::search::packs_left_out(game);
    if !left_out.is_empty() {
        notes.push(format!(
            "the level leaves out {} of {} listed pack(s) and the vehicles they state: {}",
            left_out.len(),
            game.listed.len(),
            left_out.join(", ")
        ));
    }
    if let Some(mode) = mode {
        // ⚠ A mode takes or leaves a pack's change sets, and a change set enables or leaves disabled a vehicle data file.
        notes.push(format!(
            "mounted as {}: a vehicle data file a change set of the other mode alone enables is not read",
            mode.label()
        ));
    }
    notes
}

/// Answer `asked` over `game`.
pub(super) fn vehicles_said(
    game: &MountedGame,
    root: &Path,
    asked: &VehiclesAsked,
    mode: Option<MapMode>,
) -> Said {
    let catalogue = VehicleCatalogue::new(or_title::install::vehicles::vehicles(game));
    let page = catalogue.page(&VehicleQuery {
        text: asked.text.as_deref(),
        class: asked.class.as_deref(),
        vehicle_type: asked.vehicle_type.as_deref(),
        pack: asked.pack.as_deref(),
        offset: asked.offset,
        count: asked.count,
    });
    let notes = vehicle_notes(game, mode);
    Said {
        lines: vehicles_lines(root, &catalogue, asked, &page, &notes),
        json: vehicles_json(root, &catalogue, asked, &page, &notes),
        not_kept: None,
    }
}

pub(crate) fn cmd_vehicles(args: &[String]) -> ExitCode {
    let usage = |e: &str| {
        eprintln!("Error: {VEHICLES_USAGE}: {e}");
        ExitCode::from(2)
    };
    let asked = match vehicles_asked(args) {
        Ok(asked) => asked,
        Err(e) => return usage(&e),
    };
    let (game, root, mode) = match mounted(args, &asked.install) {
        Ok(mounted) => mounted,
        Err(Stopped::Usage(e)) => return usage(&e),
        Err(Stopped::Mount(e)) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };
    print(&vehicles_said(&game, &root, &asked, mode), args);
    ExitCode::SUCCESS
}

#[cfg(test)]
#[path = "world_models_tests.rs"]
mod tests;
