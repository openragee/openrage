//! `openrage-cli world find`: what a person typed, asked of the indexes of a GTA V install: a place with what stands there, a
//! hash with what it names, the models and the maps a name answers, and the placements of the model the text names.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glam::Vec3;
use or_depends::find::{
    find, models_asked, names_of, place_maps, placements_of, Asked, Budget, Detail, Found,
    GroundFound, Install, ModelFound, ModelPlacements, Named, Rank, SearchNames,
};
use or_depends::{MountedWorld, PlacementIndex, WorldCollision, WorldIndex};
use or_maps::place::{read_pasted, Pasted};
use or_maps::script_text::{ScriptForm, ScriptForms};
use or_title::{MapMode, MountedGame};
use serde_json::json;

use super::world::flag_value;

const USAGE: &str = "world find <install> <text> [--near <x,y,z>] [--limit <n>] \
                     [--copy <lua|json|csharp>] [--level <pack>] [--mode <story|multiplayer>] \
                     [--kept <file>] [--keys <file>] [--fresh] [--json]";

/// The flags of this verb that take a value, so the value is not read as the text.
const VALUED_FLAGS: &[&str] = &[
    "--near", "--limit", "--copy", "--level", "--mode", "--kept", "--keys",
];

/// The flags of this verb that take no value.
const PLAIN_FLAGS: &[&str] = &["--json", "--fresh"];

/// The most rows of a group, with no `--limit`.
const LIMIT: usize = 10;

/// The bytes of map files read to list the placements of one model.
const MAPS_BUDGET: u64 = 64 << 20;

/// The bytes of colliders the ground of one place may hold, as `world ground` holds them.
const COLLISION_BUDGET: u64 = 256 << 20;

/// The decimals a copy is written with: a millimetre.
const DECIMALS: usize = 3;

/// What a person asked, as the arguments give it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Question {
    pub install: String,
    pub text: String,
    pub near: Option<Vec3>,
    pub limit: usize,
    /// The id of the form a result is copied in.
    pub copy: Option<String>,
}

/// The install, the text and the options of `args`.
pub(super) fn question_of(args: &[String]) -> Result<Question, String> {
    let mut words: Vec<&str> = Vec::new();
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        if VALUED_FLAGS.contains(&arg) {
            // ⚠ A flag left without its value is refused, where it would stand at its default unsaid.
            if at + 1 >= args.len() {
                return Err(format!("`{arg}` takes a value"));
            }
            at += 1;
        } else if arg.starts_with("--") {
            // ⚠ A flag this verb does not know is refused: passed over, it would be searched for as text. A value may
            // stand behind `=`, as `flag_value` reads one.
            let valued = arg.split_once('=');
            let valued = valued.is_some_and(|(flag, _)| VALUED_FLAGS.contains(&flag));
            if !(valued || PLAIN_FLAGS.contains(&arg)) {
                return Err(format!("`{arg}` is no flag of this verb"));
            }
        } else {
            // A coordinate or a signed hash may be negative, so one dash does not make a flag.
            words.push(arg);
        }
        at += 1;
    }
    let Some((install, text)) = words.split_first() else {
        return Err("<install> names the install".to_string());
    };
    if text.is_empty() {
        return Err("<text> is what to find: a place, a hash or a name".to_string());
    }
    let near = match flag_value(args, "--near") {
        None => None,
        Some(view) => match read_pasted(view) {
            Ok(Pasted::Place(place)) => match place.z {
                Some(z) => Some(Vec3::new(place.x, place.y, z)),
                None => {
                    return Err(format!(
                        "--near is a view, x,y,z, and `{view}` has no height"
                    ))
                }
            },
            Err(refused) => return Err(refused.to_string()),
            Ok(_) => return Err(format!("--near is a view, x,y,z, not `{view}`")),
        },
    };
    let limit = match flag_value(args, "--limit") {
        None => LIMIT,
        Some(word) => (word.parse::<usize>())
            .map_err(|_| format!("--limit is a count of rows, not `{word}`"))?,
    };
    Ok(Question {
        install: install.to_string(),
        // A place pasted as it was copied reaches here as the several words a shell split it into.
        text: text.join(" "),
        near,
        limit,
        copy: flag_value(args, "--copy").map(str::to_string),
    })
}

/// The form `--copy` names among the forms of the tree.
fn form_of(forms: &ScriptForms, id: &str) -> Result<ScriptForm, String> {
    forms.form(id).cloned().ok_or_else(|| {
        let ids: Vec<&str> = forms.forms.iter().map(|form| form.id.as_str()).collect();
        format!("--copy is one of {}, not `{id}`", ids.join(", "))
    })
}

/// What is said of the mount a question was asked of, a line each.
fn mount_notes(game: &MountedGame, mode: Option<MapMode>) -> Vec<String> {
    let mut notes = Vec::new();
    let left_out = super::search::packs_left_out(game);
    if !left_out.is_empty() {
        notes.push(format!(
            "the level leaves out {} of {} listed pack(s) and what they place: {}",
            left_out.len(),
            game.listed.len(),
            left_out.join(", ")
        ));
    }
    if let Some(mode) = mode {
        // ⚠ A mode takes or leaves a pack's change sets, and the index reads no change set: it reads every map and type file
        // of every archive mounted.
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
            "mounted as {}, which leaves out no map: the index reads every map and type file a mounted pack holds, \
             whatever change set would load it{left_out}",
            mode.label()
        ));
    }
    notes
}

fn hex(hash: u32) -> String {
    format!("0x{hash:08X}")
}

fn point(at: Vec3) -> String {
    format!("({}, {}, {})", at.x, at.y, at.z)
}

fn rank_word(rank: Rank) -> &'static str {
    match rank {
        Rank::Exact => "exact",
        Rank::Prefix => "prefix",
        Rank::PartPrefix => "part",
        Rank::Contained => "contained",
    }
}

/// The name of the map a placement names, else its hash.
fn map_named(index: &WorldIndex, hash: u32) -> String {
    match index.map(hash) {
        Some(map) => map.name.clone(),
        None => hex(hash),
    }
}

fn model_named(model: &ModelFound) -> String {
    model.name.clone().unwrap_or_else(|| hex(model.hash))
}

/// What `world find` prints of an answer: the groups, the placements of the model the text names, and the notes.
fn find_lines(
    index: &WorldIndex,
    text: &str,
    found: &Found,
    named: Option<&(String, ModelPlacements)>,
    notes: &[String],
) -> Vec<String> {
    let mut lines = vec![format!("Find `{text}`")];
    if let Some(place) = &found.place {
        let typed = match place.z {
            Some(z) => point(Vec3::new(place.x, place.y, z)),
            None => format!("({}, {}), with no height", place.x, place.y),
        };
        let heading = place
            .heading
            .map(|heading| format!(", heading {heading}"))
            .unwrap_or_default();
        lines.push(format!("Place {typed}{heading}"));
        lines.push(match &place.ground {
            GroundFound::At { height, bound } => {
                let from = bound
                    .as_ref()
                    .map(|bound| format!(", from {bound}"))
                    .unwrap_or_default();
                format!("  ground {height:.3}{from}")
            }
            GroundFound::Nothing => "  ground none: no bounds file holds ground there".to_string(),
            GroundFound::Unasked => "  ground not asked".to_string(),
        });
        lines.push(match place.position() {
            Some(at) => format!("  go to {}", point(at)),
            None => "  go to nowhere: it has no height and no ground".to_string(),
        });
        lines.push(format!(
            "  {} map(s) whose streaming extents hold it, {} named",
            place.maps_held,
            place.maps.len()
        ));
        let maps = place.maps.iter();
        lines.extend(maps.map(|map| format!("    map    {}  {}", map.name, map.logical)));
        lines.push(format!(
            "  {} bound(s) whose box holds it, {} named",
            place.bounds_held,
            place.bounds.len()
        ));
        let bounds = place.bounds.iter();
        lines.extend(bounds.map(|bound| format!("    bound  {}  {}", bound.name, bound.logical)));
    }
    if let Some(hash) = &found.hash {
        let spelled = &hash.spellings;
        let name = match &spelled.name {
            Some(name) => format!("name {name}"),
            None => "no name stated by the install".to_string(),
        };
        lines.push(format!(
            "Hash {}  unsigned {}  signed {}  {name}",
            spelled.hexadecimal, spelled.unsigned, spelled.signed
        ));
        if hash.names.is_empty() {
            lines.push("  names nothing in the install".to_string());
        }
        lines.extend(hash.names.iter().map(|named| match named {
            Named::Archetype {
                type_file: Some(file),
            } => format!(
                "  an archetype, declared in {}  {}",
                file.name, file.logical
            ),
            Named::Archetype { type_file: None } => "  an archetype".to_string(),
            Named::Map { logical } => format!("  a map  {logical}"),
            Named::File { kind, logical } => {
                format!("  a file ({})  {logical}", kind.extension())
            }
        }));
    }
    if found.place.is_none() {
        let more = |more: bool| if more { ", and more" } else { "" };
        lines.push(format!(
            "Models: {}{}",
            found.models.len(),
            more(found.more_models)
        ));
        for model in &found.models {
            let declared = match &model.type_file {
                Some(file) => format!("declared in {}  {}", file.name, file.logical),
                None => "declared in no type file read".to_string(),
            };
            let placed = match (model.placements, &model.nearest) {
                (0, _) => "0 placement(s): no map places it, so it opens as a model".to_string(),
                (count, Some(nearest)) => format!(
                    "{count} placement(s), the nearest {:.3} m away at {} in {}",
                    nearest.distance,
                    point(nearest.position),
                    map_named(index, nearest.map)
                ),
                (count, None) => format!("{count} placement(s)"),
            };
            lines.push(format!(
                "  {}  {}  {}  {declared}  {placed}",
                model_named(model),
                hex(model.hash),
                rank_word(model.rank)
            ));
        }
        if let Some((name, placements)) = named {
            lines.push(format!(
                "Placements of {name}: {} known, {} listed, {} map file(s) read",
                placements.known,
                placements.listed.len(),
                placements.maps_read
            ));
            for placement in &placements.listed {
                let map = (placement.map_name.clone()).unwrap_or_else(|| hex(placement.map));
                let file = placement.logical.as_deref().unwrap_or("no file");
                let at = match placement.position {
                    Some(at) => point(at),
                    None => "no position".to_string(),
                };
                let detail = match placement.detail {
                    Detail::Entity {
                        index,
                        guid,
                        heading,
                    } => format!("entity {index}  guid {guid}  heading {heading:.3}"),
                    Detail::Batch => "a grass batch".to_string(),
                    Detail::Unread(why) => format!("not read: {why}"),
                };
                let away = placement
                    .distance
                    .map(|distance| format!("  {distance:.3} m"))
                    .unwrap_or_default();
                lines.push(format!("    {map}  {file}  {at}  {detail}{away}"));
            }
        }
        lines.push(format!(
            "Maps: {}{}",
            found.maps.len(),
            more(found.more_maps)
        ));
        for map in &found.maps {
            let centre = match map.centre {
                Some(centre) => format!("centre {}", point(centre)),
                None => "no extents".to_string(),
            };
            lines.push(format!(
                "  {}  {}  {}  {centre}  {}",
                map.name,
                hex(map.hash),
                rank_word(map.rank),
                map.logical
            ));
        }
    }
    lines.extend(notes.iter().map(|note| format!("  note: {note}")));
    lines
}

fn find_json(
    index: &WorldIndex,
    question: &Question,
    found: &Found,
    named: Option<&(String, ModelPlacements)>,
    notes: &[String],
) -> serde_json::Value {
    let xyz = |at: Vec3| json!([at.x, at.y, at.z]);
    let file =
        |file: &or_depends::find::FileNamed| json!({ "name": file.name, "logical": file.logical });
    let place = found.place.as_ref().map(|place| {
        let ground = match &place.ground {
            GroundFound::At { height, bound } => json!({ "height": height, "bound": bound }),
            _ => json!(null),
        };
        json!({
            "x": place.x,
            "y": place.y,
            "z": place.z,
            "heading": place.heading,
            "ground": ground,
            "goTo": place.position().map(xyz),
            "mapsHeld": place.maps_held,
            "maps": place.maps.iter().map(file).collect::<Vec<_>>(),
            "boundsHeld": place.bounds_held,
            "bounds": place.bounds.iter().map(file).collect::<Vec<_>>(),
        })
    });
    let hash = found.hash.as_ref().map(|hash| {
        let names: Vec<_> = (hash.names.iter())
            .map(|named| match named {
                Named::Archetype { type_file } => {
                    json!({ "is": "archetype", "typeFile": type_file.as_ref().map(file) })
                }
                Named::Map { logical } => json!({ "is": "map", "logical": logical }),
                Named::File { kind, logical } => {
                    json!({ "is": "file", "extension": kind.extension(), "logical": logical })
                }
            })
            .collect();
        json!({
            "hexadecimal": hash.spellings.hexadecimal,
            "unsigned": hash.spellings.unsigned,
            "signed": hash.spellings.signed,
            "name": hash.spellings.name,
            "names": names,
        })
    });
    let models: Vec<_> = (found.models.iter())
        .map(|model| {
            let nearest = model.nearest.map(|nearest| {
                json!({
                    "position": xyz(nearest.position),
                    "map": map_named(index, nearest.map),
                    "distance": nearest.distance,
                })
            });
            json!({
                "name": model.name,
                "hash": hex(model.hash),
                "rank": rank_word(model.rank),
                "typeFile": model.type_file.as_ref().map(file),
                "placements": model.placements,
                "nearest": nearest,
            })
        })
        .collect();
    let placements = named.map(|(name, placements)| {
        let listed: Vec<_> = (placements.listed.iter())
            .map(|placement| {
                let (entity, guid, heading, not_read) = match placement.detail {
                    Detail::Entity {
                        index,
                        guid,
                        heading,
                    } => (Some(index), Some(guid), Some(heading), None),
                    Detail::Batch => (None, None, None, None),
                    Detail::Unread(why) => (None, None, None, Some(why.to_string())),
                };
                json!({
                    "map": (placement.map_name.clone()).unwrap_or_else(|| hex(placement.map)),
                    "logical": placement.logical,
                    "grassBatch": matches!(placement.detail, Detail::Batch),
                    "position": placement.position.map(xyz),
                    "entity": entity,
                    "guid": guid,
                    "heading": heading,
                    "notRead": not_read,
                    "distance": placement.distance,
                })
            })
            .collect();
        json!({
            "model": name,
            "known": placements.known,
            "mapsRead": placements.maps_read,
            "listed": listed,
        })
    });
    let maps: Vec<_> = (found.maps.iter())
        .map(|map| {
            json!({
                "name": map.name,
                "hash": hex(map.hash),
                "rank": rank_word(map.rank),
                "logical": map.logical,
                "centre": map.centre.map(xyz),
                "extents": map.extents.map(|(min, max)| json!({ "min": xyz(min), "max": xyz(max) })),
            })
        })
        .collect();
    json!({
        "text": question.text,
        "near": question.near.map(xyz),
        "limit": question.limit,
        "place": place,
        "hash": hash,
        "models": models,
        "moreModels": found.more_models,
        "placements": placements,
        "maps": maps,
        "moreMaps": found.more_maps,
        "notes": notes,
    })
}

/// What `world find` says of a question.
pub(super) struct Said {
    pub lines: Vec<String>,
    pub json: serde_json::Value,
    /// With a form to copy in: the result as that form writes it, else why there is nothing to copy.
    pub copy: Option<Result<String, String>>,
    /// Why the index was not kept, where it was not.
    pub not_kept: Option<std::io::Error>,
}

/// Answer `question` over `game`, mounted from `root`, with its index kept in `kept`; an error where the text has the shape of a place or a hash and is none.
pub(super) fn said(
    game: &MountedGame,
    root: &Path,
    kept: &Path,
    fresh: bool,
    question: &Question,
    mode: Option<MapMode>,
    form: Option<&ScriptForm>,
) -> Result<Said, String> {
    let (index, report) = super::world_index::indexed(game, root, kept, fresh);
    let source = MountedWorld::new(game, root);
    let (catalogue, names) = names_of(&source);
    let search = SearchNames::new(&index, &names);
    let (text, limit) = (question.text.as_str(), question.limit);
    let mut notes = mount_notes(game, mode);

    // ⚠ Nothing keeps which maps place a model, so the maps are walked for the models the text lists and for no other; a text
    // that lists no model reads no map.
    let watched = models_asked(text, &index, &search, limit).map_err(|e| e.to_string())?;
    let mut placements = PlacementIndex::watching(watched.iter().copied());
    if !watched.is_empty() {
        let unread = place_maps(&index, &source, &mut placements);
        if unread > 0 {
            notes.push(format!(
                "{unread} map(s) the index read gave no map now, so what they place is not counted"
            ));
        }
    }
    let install = Install {
        world: &index,
        catalogue: &catalogue,
        names: &names,
        search: &search,
        placements: &placements,
    };
    let mut collision = WorldCollision::new(&index, &source, COLLISION_BUDGET);
    let asked = Asked {
        near: question.near,
        limit,
    };
    let found = find(text, &install, &asked, Some(&mut collision)).map_err(|e| e.to_string())?;
    // The placements of the model the text names, which is the one a person would go to.
    let named = (found.models.iter())
        .find(|model| model.rank == Rank::Exact)
        .map(|model| {
            let budget = Budget {
                placements: limit,
                bytes: MAPS_BUDGET,
            };
            let placements = placements_of(model.hash, &install, &source, question.near, &budget);
            (model_named(model), placements)
        });
    if search.unnamed() > 0 && found.place.is_none() && found.hash.is_none() {
        notes.push(format!(
            "{} archetype(s) are named by no file of the install: their hash and their whole name find them, a part of it does not",
            search.unnamed()
        ));
    }
    let copy = form.map(|form| {
        let written = match (&found.place, &named) {
            (Some(place), _) => place.script(form, DECIMALS).map(|text| {
                text.ok_or_else(|| "the place has no height and no ground under it".to_string())
            }),
            (None, Some((name, placements))) if !placements.listed.is_empty() => {
                placements.script(name, form, DECIMALS).map(Ok)
            }
            (None, Some((name, _))) => Ok(Err(format!("no map places {name}"))),
            (None, None) => Ok(Err(
                "the text is no place and names no model in full".to_string()
            )),
        };
        written.unwrap_or_else(|e| Err(e.to_string()))
    });
    let mut json = find_json(&index, question, &found, named.as_ref(), &notes);
    if let (Some(form), Some(Ok(text))) = (form, &copy) {
        json["copy"] = json!({ "form": form.id, "text": text });
    }
    Ok(Said {
        lines: find_lines(&index, text, &found, named.as_ref(), &notes),
        json,
        copy,
        not_kept: report.not_kept,
    })
}

pub(crate) fn cmd_find(args: &[String]) -> ExitCode {
    let usage = |e: &str| {
        eprintln!("Error: {USAGE}: {e}");
        ExitCode::from(2)
    };
    let question = match question_of(args) {
        Ok(question) => question,
        Err(e) => return usage(&e),
    };
    let form = match &question.copy {
        None => None,
        Some(id) => {
            let form = ScriptForms::built_in()
                .map_err(|e| e.to_string())
                .and_then(|forms| form_of(&forms, id));
            match form {
                Ok(form) => Some(form),
                Err(e) => return usage(&e),
            }
        }
    };
    let (cutoff, options) = match super::world_ground::mount_of(args) {
        Ok(mount) => mount,
        Err(e) => return usage(&e),
    };
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = match std::fs::canonicalize(&question.install) {
        Ok(root) => root,
        Err(e) => return usage(&format!("{}: {e}", question.install)),
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
    let said = match said(
        &mount,
        &root,
        &kept,
        fresh,
        &question,
        options.mode,
        form.as_ref(),
    ) {
        Ok(said) => said,
        Err(e) => return usage(&e),
    };
    if let Some(error) = &said.not_kept {
        eprintln!("warning: the index was not kept: {error}");
    }
    let json = args.iter().any(|a| a == "--json");
    match (&said.copy, json) {
        (Some(Err(why)), _) => {
            eprintln!("Error: nothing to copy: {why}");
            return ExitCode::FAILURE;
        }
        // A copy alone is printed, so it goes to a clipboard or a file as it is.
        (Some(Ok(text)), false) => println!("{text}"),
        (_, true) => println!("{}", said.json),
        (None, false) => {
            for line in &said.lines {
                println!("{line}");
            }
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
#[path = "world_find_tests.rs"]
mod tests;
