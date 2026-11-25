//! `openrage-cli maps edit-game`: move or delete an object the game placed, from a mounted install, in a project's maps content
//! (`or_maps::first_edit`). It prints what the edit would import and do, does it with `--apply`, and takes a record back.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_depends::WorldIndex;
use or_maps::first_edit::{self, Change, Edit, Place, Record, Which};
use or_project::Target;
use or_title::MountedGame;
use serde_json::{json, Value};

use crate::maps::{deletion_lines, maps_of};
use crate::project::{parse, Failure};
use Failure::{Run, Usage};

/// A mounted install as a first edit asks it: the bytes from the mount, and which map file is which from the world index.
pub(crate) struct Mounted<'a> {
    pub game: &'a MountedGame,
    pub index: &'a WorldIndex,
    pub edition: Target,
}

impl first_edit::Game for Mounted<'_> {
    fn edition(&self) -> Target {
        self.edition
    }

    fn read(&self, logical: &str) -> Option<Vec<u8>> {
        self.game.vfs.open(logical)
    }

    fn lies_at(&self, logical: &str) -> Option<String> {
        self.game.vfs.locate(logical)
    }

    fn map_file(&self, name: u32) -> Option<String> {
        self.index.map(name).map(|map| map.logical.clone())
    }

    fn maps_under(&self, parent: u32) -> Vec<String> {
        let under = self.index.maps().iter().filter(|map| map.parent == parent);
        under.map(|map| map.logical.clone()).collect()
    }
}

/// The entity `text` names: an index, or `guid:` and a guid in decimal or after `0x` in hexadecimal.
fn which(text: &str) -> Result<Which, Failure> {
    let named = match text.strip_prefix("guid:") {
        Some(guid) => match guid.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => guid.parse().ok(),
        }
        .map(Which::Guid),
        None => text.parse().ok().map(Which::Index),
    };
    named.ok_or_else(|| {
        Usage(format!(
            "`{text}` names no entity: say its index, such as 12, or guid:<its guid>"
        ))
    })
}

/// The position `text` states as `x,y,z`.
fn position(text: &str) -> Result<[f32; 3], Failure> {
    let numbers: Vec<f32> = (text.split(','))
        .filter_map(|number| number.trim().parse().ok())
        .collect();
    match (numbers.as_slice(), text.split(',').count()) {
        (&[x, y, z], 3) => Ok([x, y, z]),
        _ => Err(Usage(format!(
            "--move takes a position as x,y,z, and `{text}` is none"
        ))),
    }
}

/// What the verb prints of an edit, and the same as data: the record, and for a deletion the lines of its plan.
///
/// With `apply` the edit is done; without, nothing is written.
pub(crate) fn report(
    place: &Place<'_>,
    game: &dyn first_edit::Game,
    edit: &Edit,
    apply: bool,
) -> Result<(Vec<String>, Value), Failure> {
    let planned = first_edit::plan(place, game, edit).map_err(|e| Run(e.to_string()))?;
    let record = match apply {
        true => first_edit::apply(place, game, edit).map_err(|e| Run(e.to_string()))?,
        false => planned.record,
    };
    // ⚠ Without its notes, which the record states: a line is said once.
    let plan: Vec<String> = (planned.deletion.iter())
        .flat_map(|deletion| deletion_lines(deletion, apply))
        .filter(|line| !line.starts_with("  note   "))
        .collect();
    let mut lines = record.lines();
    lines.extend(plan.iter().cloned());
    if !apply {
        lines.push("nothing was written: say --apply to do it".to_string());
    }
    let mut document = serde_json::to_value(&record).map_err(|e| Run(e.to_string()))?;
    document["plan"] = json!(plan);
    Ok((lines, document))
}

/// The one maps content of the project at `path` the flags name, with the project.
fn content(
    path: &String,
    flags: &std::collections::HashMap<String, String>,
) -> Result<(or_project::Project, String), Failure> {
    let (project, modules) = maps_of(std::slice::from_ref(path), flags)?;
    let [id] = modules.as_slice() else {
        return Err(Run(format!(
            "{path} holds {} maps contents ({}): say which with --module",
            modules.len(),
            modules.join(", ")
        )));
    };
    let id = id.clone();
    Ok((project, id))
}

/// The record `bytes` hold, as `--record` writes it or as `--json` prints it with the lines of a plan beside it.
pub(crate) fn record_of(bytes: &[u8]) -> Result<Record, String> {
    let Ok(Value::Object(mut document)) = serde_json::from_slice::<Value>(bytes) else {
        return Record::from_json(bytes).map_err(|e| e.to_string());
    };
    document.remove("plan");
    let record = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    Record::from_json(&record).map_err(|e| e.to_string())
}

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let bare = ["--apply", "--json", "--delete", "--fresh"];
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let rest: Vec<String> = (args.iter())
        .filter(|arg| !bare.contains(&arg.as_str()))
        .cloned()
        .collect();
    let valued = [
        "--module",
        "--game",
        "--keys",
        "--kept",
        "--map",
        "--entity",
        "--move",
        "--record",
        "--take-back",
    ];
    let (positional, flags) = parse(&rest, &valued)?;
    let [path] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let (project, id) = content(path, &flags)?;
    let module = (project.container.module(&id))
        .ok_or_else(|| Run(format!("{path} has no maps content `{id}`")))?;
    let place = Place {
        project: Path::new(path),
        container: &project.container,
        module,
    };

    if let Some(file) = flags.get("--take-back") {
        let bytes = std::fs::read(file).map_err(|e| Run(format!("{file}: {e}")))?;
        let record = record_of(&bytes).map_err(|e| Run(format!("{file}: {e}")))?;
        let taken = first_edit::take_back(&place, &record).map_err(|e| Run(e.to_string()))?;
        for line in taken.lines() {
            println!("{line}");
        }
        return Ok(ExitCode::SUCCESS);
    }

    let map = (flags.get("--map")).ok_or_else(|| {
        Usage("--map <logical path> names the map file of the install that holds the object".into())
    })?;
    let entity = (flags.get("--entity")).ok_or_else(|| {
        Usage("--entity <index> or --entity guid:<guid> names the object in its map file".into())
    })?;
    let change = match (flags.get("--move"), has("--delete")) {
        (Some(text), false) => Change::Move {
            position: position(text)?,
        },
        (None, true) => Change::Delete,
        _ => return Err(Usage("say --move x,y,z or --delete, and not both".into())),
    };
    let edit = Edit {
        map: map.clone(),
        entity: which(entity)?,
        change,
    };
    if flags.contains_key("--record") && !has("--apply") {
        return Err(Usage(
            "--record keeps the record of an edit that was done: say --apply with it".into(),
        ));
    }

    let game = (flags.get("--game")).ok_or_else(|| {
        Usage("--game <install root> names the install the object is placed in".into())
    })?;
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = std::fs::canonicalize(game).map_err(|e| Run(format!("{game}: {e}")))?;
    let mount = crate::keys::mount_all(&root, flags.get("--keys").map(Path::new)).map_err(Run)?;
    let kept = match flags.get("--kept") {
        Some(file) => PathBuf::from(file),
        None => crate::world_index::default_kept(&root, None).ok_or_else(|| {
            Run("no configuration directory to keep the world index in: name a file with --kept <file>".into())
        })?,
    };
    let (index, _) = crate::world_index::indexed(&mount, &root, &kept, has("--fresh"));
    let edition = match or_title::game::Game::detect_from_root(&root)
        .generation()
        .is_gen9()
    {
        true => Target::Enhanced,
        false => Target::Legacy,
    };
    let mounted = Mounted {
        game: &mount,
        index: &index,
        edition,
    };

    let apply = has("--apply");
    let (lines, document) = report(&place, &mounted, &edit, apply)?;
    if let Some(file) = flags.get("--record") {
        let mut text = serde_json::to_string_pretty(&document).map_err(|e| Run(e.to_string()))?;
        text.push('\n');
        // ⚠ The edit is done by now, and its record is the one thing that takes it back.
        if let Err(e) = std::fs::write(file, &text) {
            print!("{text}");
            return Err(Run(format!(
                "{file}: {e}; the edit was done, and its record is printed in the place of the file"
            )));
        }
    }
    if has("--json") {
        let text = serde_json::to_string_pretty(&document).map_err(|e| Run(e.to_string()))?;
        println!("{text}");
    } else {
        println!("content {id}:");
        for line in &lines {
            println!("{line}");
        }
    }
    Ok(ExitCode::SUCCESS)
}
