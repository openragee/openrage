//! `openrage-cli maps`: list what a project's map sets place and declare, validate them and say what the rules find, read a build back
//! against them, apply a list of operations to their sources, merge two copies of one map, propose and take out the collision of an
//! object, plan or apply the deletion of entities with their chain of far models, keep a manifest of the game for the manifest of the
//! maps to be written from, list their doors, list and edit the path regions a folder holds as sources, and move or delete an object
//! the game placed, which imports its map (`maps_edit_game`).
//!
//! The plan and the build of maps are `openrage-cli project plan` and `build`:
//! a maps content is content like any other, and goes through the one plan.

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use or_maps::{planned, Depth, Severity};
use or_project::{ModuleKind, Project};
use serde_json::json;

use crate::project::{parse, planned as planned_plans, root_of, Failure};
use Failure::{Run, Usage};

pub fn run(args: &[String]) -> ExitCode {
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("changes") => crate::maps_changes::run(rest),
        Some("collision") => crate::maps_collision::run(rest),
        Some("list") => list(rest),
        Some("merge") => crate::maps_merge::run(rest),
        Some("validate") => validate(rest),
        Some("verify") => verify(rest),
        Some("apply") => apply(rest),
        Some("keep-manifest") => keep_manifest(rest),
        Some("delete") => delete(rest),
        Some("put-back") => crate::maps_put_back::run(rest),
        Some("edit-game") => crate::maps_edit_game::run(rest),
        Some("doors") => doors(rest),
        Some("roads") => crate::maps_roads::run(rest),
        Some("-h" | "--help" | "help") => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(Usage(format!("unknown maps command: {other}"))),
        None => {
            print_usage();
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => code,
        Err(Usage(message)) => crate::usage_error(&message),
        Err(Run(message)) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         openrage-cli maps changes  <project.openrage-proj> [--module <id>] [--json]\n  \
         openrage-cli maps collision <project.openrage-proj> --object <map source>:<entity> [--box <x,y,z,x,y,z>]\n  \
         \x20                          [--take-ground] [--apply | --move <x,y,z> [--turn <degrees>]] [--inverse <file>]\n  \
         \x20                          [--dry-run] [--module <id>] [--json]\n  \
         openrage-cli maps collision <project.openrage-proj> --operations <operations.json> [--inverse <file>]\n  \
         \x20                          [--dry-run] [--module <id>] [--json]\n  \
         openrage-cli maps list     <project.openrage-proj> [--module <id>]\n  \
         openrage-cli maps merge    --base <file> --ours <file> --theirs <file> --out <file> [--take ours|theirs] [--json]\n  \
         openrage-cli maps validate <project.openrage-proj> [--module <id>] [--json]\n  \
         openrage-cli maps verify   <project.openrage-proj> [--build <id>] [--out <folder>]\n  \
         openrage-cli maps apply    <project.openrage-proj> <operations.json> [--module <id>] [--dry-run]\n  \
         openrage-cli maps keep-manifest <project.openrage-proj> <_manifest.ymf>... [--module <id>]\n  \
         openrage-cli maps keep-manifest <project.openrage-proj> --game <install root> [--keys <file>] [--module <id>]\n  \
         openrage-cli maps delete   <project.openrage-proj> <source>:<entity>... [--module <id>] [--apply]\n                             \
         [--script <form>] [--radius <metres>]\n  \
         openrage-cli maps put-back <project.openrage-proj> <source>:<entity> | <source>:base:<entity> [--module <id>]\n                             \
         [--apply] [--inverse <file>] [--json]\n  \
         openrage-cli maps edit-game <project.openrage-proj> --game <install root> --map <logical path>\n                             \
         --entity <index|guid:guid> (--move x,y,z | --delete) [--apply] [--record <file>] [--json]\n                             \
         [--module <id>] [--level <pack>] [--mode <story|multiplayer>] [--keys <file>] [--kept <file>]\n                             \
         [--fresh]\n  \
         openrage-cli maps edit-game <project.openrage-proj> --take-back <record.json> [--module <id>]\n  \
         openrage-cli maps doors    <project.openrage-proj> [--module <id>] [--names <file>] [--json]\n  \
         openrage-cli maps doors    --game <install root> --map <logical path> [--names <file>] [--json]\n                             \
         [--keys <file>] [--kept <file>] [--fresh]\n  \
         openrage-cli maps roads    <folder> [--json]\n  \
         openrage-cli maps roads    <folder> --apply <operations.json> [--dry-run] [--inverse <file>]\n\n\
         changes compares each map file and type file of the content with the base its source keeps,\n\
         the file it was imported from, and says by object what the source changed: the entities and\n\
         the archetypes added, deleted and changed, each with its index in the base and its index now,\n\
         and the ones no guid and no place tells from a twin, which are named ambiguous and never\n\
         moved. What a file states of itself differently, such as its flags and its extents, is a\n\
         line of its own, and the entities of an interior are under the interior. With --json it\n\
         prints the same changes as data. It exits 1 when a source cannot be compared.\n\
         collision says which polygons of the bounds sources the content holds are proposed as the\n\
         collision of an object, the entity of a map source: the polygons its box holds whole, with the\n\
         ground left, each polygon the box cuts listed apart with the share of it in the box, and the\n\
         reason for each one left. The box is the one a type file of the content declares for the\n\
         archetype, or the one --box states in the archetype's own space; an object under two metres\n\
         is not judged. --take-ground proposes the ground too. It writes nothing unless asked: --apply\n\
         takes the proposal out, and --move and --turn move it by an offset in metres and a turn in\n\
         degrees about where the object stands, leaving the corners shared with what stays. The\n\
         versions of an area, the files `<area>`, `hi@<area>` and `ma@<area>`, hold polygons of their\n\
         own, so each is changed by what is proposed in it, and all are written or none. An operation\n\
         of a list that does not state `alone` is done to every version alike, or refused naming the\n\
         version that lacks a polygon; a bound its source cannot build again is refused by name.\n\
         --json prints the proposal and its operations as data, a list whose format is\n\
         `openrage-collision-operations` that --operations applies after a person has edited it, and\n\
         --inverse writes the list that puts every source back byte for byte. A list names polygons\n\
         by their place in a source as it was when the list was made, and is refused once that\n\
         source has changed. It moves no entity: `maps apply` does. It exits 1 when an operation is\n\
         refused.\n\
         list names the map sets of the content and, for each source, what it places (the entities of\n\
         a map file) or declares (the archetypes of a type file, and the rooms and portals of its\n\
         interiors).\n\
         merge merges two copies of one map file or type file, ours and theirs, against the file both\n\
         were made from, the base, which for a map of the game is the game's file. It takes each\n\
         change `changes` would list of a side: an entity only one side moved, changed or deleted, the\n\
         entities each added, each with a guid of its own that the base does not state, and the\n\
         archetypes of a type file by name. What both changed to one value is one change. The rest is\n\
         a conflict, named with the entity as each of the three files indexes it: one member both\n\
         changed to different values, an entity one side changed and the other deleted, an entity of\n\
         one archetype both added at one position that does not state the same on both sides, and an\n\
         entity either side cannot tell from a twin, which is never paired by guess. With no --take\n\
         a conflict stops the merge: nothing is written and it exits 1. --take ours or --take theirs\n\
         decides every conflict for that side. The merged file is written to --out with its LOD links,\n\
         its counts of children and its extents right, and a merge that cannot be made valid is\n\
         refused with the reason. The report says what was applied, each conflict with what was taken,\n\
         and what the merge does not look into; with --json it is the same report as data.\n\
         validate reads every source of the maps content, opens the map files and type files through\n\
         the typed world family, and lists what the rules find, each with a code and a severity. It\n\
         exits 1 when a finding that is not waived is an error.\n\
         verify reads the resource a build wrote against the map sets it was made of: that each source\n\
         reached the place the plan gave it, that every map file and type file reloads with the\n\
         entities and archetypes its source states, that the manifest registers each type file and\n\
         declares the resource a map, and that the manifest of the maps is there and states what the\n\
         sources come to. It exits 1 when a check fails. The plan and the build are\n\
         `openrage-cli project plan` and `build`; maps go through them as any content does.\n\
         apply does the operations of a list, in order, to the sources of the maps content: an entity\n\
         placed, added as a copy or taken out with its LOD links renumbered, an archetype added or taken\n\
         out, the extents grown or stated, a record put back. The list is JSON whose format is\n\
         `openrage-map-operations` and whose schema is 1, each operation an object whose `op` names it\n\
         and whose `source` is a source folder of the content. An operation that states `expects`,\n\
         the SHA-256 of the XML its source is to hold, is refused when the source holds another. It\n\
         writes all of them or none: an operation that is refused is named by its place in the list,\n\
         and no source changes. With --dry-run it says what each operation would change and writes\n\
         nothing. It exits 1 when an operation is refused. A name an operation states, the archetype\n\
         of an entity added or the name and the asset of an archetype added, is remembered in the\n\
         content, since a file keeps its hash alone.\n\
         keep-manifest keeps a manifest of the game in the maps content, as the game ships it. A build\n\
         writes the manifest of its own maps from the ones kept: they say how that document is laid\n\
         out and what flags the entry of a map takes, which nothing else in a project says. Name the\n\
         files, or name an install with --game to keep every manifest of it that states a map of the\n\
         content. With none kept a build writes no manifest and its plan says so. A kept manifest\n\
         is the game's own file, whole: `project check-sharing` names it and the project's .gitignore\n\
         leaves it out.\n\
         delete plans the removal of entities of the map files of the content, each named by its source\n\
         folder and its index there (`city.metadata.src:12`), and prints the plan: what goes with them (what\n\
         stands under each in the level-of-detail tree, and each far model left standing for nothing, in\n\
         this file and in the map it names as parent), each far model that stays because it stands for\n\
         another entity too, with its count of children lowered, each entity left because another of its\n\
         archetype stands at its pose, and every file the deletion would write with what changes in it.\n\
         It also prints the script line that hides each model at its place and changes no file, in the\n\
         form --script names (lua, json or csharp; lua when none) within --radius metres (1 when none).\n\
         It reads the map files of the content and no other: a map that is not among them keeps its links.\n\
         With --apply it writes the deletion, every file or none. It exits 1 when an entity named is not\n\
         there or the deletion is refused.\n\
         put-back plans what makes one entity of a map file of the content state what the base of its\n\
         source states again, the file the source was imported from, and prints the plan. The entity\n\
         is named as `changes` lists it: one the map file holds by its source folder and its index there\n\
         (`city.metadata.src:12`), one the map file lacks by its index in the base\n\
         (`city.metadata.src:base:7`). An entity the file holds is put back by one record in its place,\n\
         with the far model the base names for it. One it lacks comes back where the order of the base\n\
         puts it, with what a deletion took: each far model above it that the content lacks too, in\n\
         its file or in the map it names as parent, and each entity the base places under it. A far\n\
         model counts the children that stand under it afterwards, and the file states the extents of\n\
         its base, grown to hold what still stands otherwise. It writes nothing unless asked: --apply\n\
         writes it, every file or none, and --inverse keeps the list that takes it back, which\n\
         `maps apply` does, before a source is written. The last line says how many changes of its base\n\
         the source holds, which is what a discard of the map would lose. --json prints the plan as\n\
         data, with its operations as a list `maps apply` takes. It is refused with the reason, and\n\
         exits 1, for an entity that cannot be told from a twin, which is never guessed, for one the\n\
         base does not hold, which the source added and `delete` takes out, for one that states what\n\
         the base states, and where a base does not cook. A plan is of the sources as they were read:\n\
         each source it writes is held to the XML it was planned from.\n\
         edit-game moves or deletes an object the game placed, in the maps content. The object is an\n\
         entity of a map file of the install --game names, said by the logical path of that file as\n\
         `world find` and `world select` print it and by its index there, or by `guid:` and its guid\n\
         when one entity of the file states it. The install is mounted as --level and --mode say, as\n\
         those verbs mount it, and a path that is not the copy of its map that mount loads is refused\n\
         with the path of the copy it loads. It prints what the edit would import and do: each map file of\n\
         the game that would become a source of the content with the game's file as its base (for a\n\
         move the one map, for a deletion every map file its chain of far models changes), each\n\
         operation, each note, what goes and stays for a deletion, and what the person is told, with\n\
         what a source taken from the game is to a repository of the project. A map file that is a\n\
         source of the content already is edited there and nothing is imported: an index given with\n\
         the path of the game's file is a place in the game's file and is carried to the source by\n\
         what the two hold, refused where that entity is deleted there or cannot be told from a twin.\n\
         --map may name such a source as the content does (`city.metadata.src`), and an index is then\n\
         a place in the source. With --apply it does it, all or\n\
         nothing: a map file that cannot become a source, one that does not read, and one of an install\n\
         of the other edition than a build of the content targets refuse the whole edit by name, and\n\
         nothing stays written. --record keeps the record of what was done in a file, and --take-back\n\
         takes such a record back: the operations by their inverses, then the sources it imported and\n\
         the files it rewrote, refused while a later edit of one of its sources stands. --json prints\n\
         the record as data, with the lines of a deletion's plan. The world index is read from the\n\
         file `world index` keeps (--kept, --fresh). It exits 1 when the edit is refused.\n\
         doors lists the doors of each map set: an entity that states the door extension or whose\n\
         archetype does, and an entity whose model's name a door pattern matches, with how many\n\
         entities have a model with no name, since a door among them is missed. A model has a name\n\
         when the content remembers it\n\
         (one typed on the Map screen or stated by an operation of `maps apply`) or the file --names\n\
         gives lists it, a name a line: a map file a project imported keeps hashes alone. For each\n\
         door it says what its box reads as (a leaf that swings, slides or lifts, its hinge and its\n\
         width), the other leaf of its double door, how far it opens each way before the box of\n\
         another entity is in its arc, and what the door rules find. All of it is read from the\n\
         model's box: the game's door tuning and the collision of the map are not read. --json prints\n\
         the same doors as JSON. With --game it lists the doors of one map file of that install,\n\
         named by its logical path as `world index`, `world at` and `world find` print it, or by\n\
         a path that answers the same file, as a pack's own does once the title update patches it;\n\
         another copy of the map than the one the install loads is refused. It\n\
         reads the type files the world index says declare what the map places and what its\n\
         interiors hold. A model has the name of a file the install lists, or one --names gives.\n\
         It writes nothing to the install; the world index is read from the file `world index`\n\
         keeps (--kept, --fresh).\n\
         roads lists the path regions a folder holds as sources, each `<name>.paths.src` directly\n\
         inside it, such as the `sources` folder of a project: the region each states, its nodes, links\n\
         and junctions, and what the rules find over the set, each with a code and a severity. A rule\n\
         that was not decided is listed as not checked, with the reason: the two that measure a node\n\
         against the ground, and the lanes of a link, which the declaration of a link does not name.\n\
         With --json it prints the same as data. It exits 1 when a finding is an error. With --apply\n\
         it does the operations of a list, in order, to those sources: a node's values stated, a link's\n\
         word stated, a link added or taken out with the link back, a node added or taken out with\n\
         every link of the set that names a later node of its region renumbered. The list is JSON\n\
         whose format is `openrage-road-operations` and whose schema is 1, each operation an object\n\
         whose `op` names it, a node named `region:index`. It writes all of them or none, and with\n\
         --dry-run nothing. --inverse writes the list that puts every source back, newest first,\n\
         before a source is written, and `maps roads --apply` applies it while the set is as the\n\
         list left it. An operation that states `expects`, a digest of every document of the\n\
         set, is refused when the set holds other documents, as an operation of `maps apply` is for\n\
         its source: a link names a node of another region by its index, so the whole set is what\n\
         an operation is held to. A node another region of the set names is not taken out, and a\n\
         request to disable a node is refused, since the file states no such thing.\n\n\
         A build checks the files. It does not test that GTA V loads them.\n"
    );
}

pub(crate) fn maps_of(
    positional: &[String],
    flags: &HashMap<String, String>,
) -> Result<(Project, Vec<String>), Failure> {
    let [path] = positional else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let project = Project::load(Path::new(path)).map_err(|e| Run(format!("{path}: {e}")))?;
    for finding in &project.findings {
        eprintln!("warning: {finding}");
    }
    let all: Vec<String> = (project.container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Maps)
        .map(|m| m.id.clone())
        .collect();
    let chosen = match flags.get("--module") {
        Some(id) if all.contains(id) => vec![id.clone()],
        Some(id) => return Err(Run(format!("{path} has no maps content `{id}`"))),
        None => all,
    };
    if chosen.is_empty() {
        return Err(Run(format!("{path} holds no maps content")));
    }
    Ok((project, chosen))
}

/// What every listing of doors ends on: where what it says of a door was read from.
pub(crate) const DOORS_READ_FROM: &str = "A door's hinge, motion and arc are read from its model's box. The game's door tuning and the collision of the map are not read.";

/// List the doors of each map set of the content, or of one map file of an install, with their pairing and what the door rules say.
fn doors(args: &[String]) -> Result<ExitCode, Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let fresh = args.iter().any(|a| a == "--fresh");
    let rest: Vec<String> = (args.iter())
        .filter(|a| *a != "--json" && *a != "--fresh")
        .cloned()
        .collect();
    let (positional, flags) = parse(
        &rest,
        &["--module", "--names", "--game", "--map", "--keys", "--kept"],
    )?;
    if flags.contains_key("--game") {
        if !positional.is_empty() || flags.contains_key("--module") {
            return Err(Usage(
                "--game lists the doors of a map file of an install, and a project and --module name a content: say one".into(),
            ));
        }
        return crate::maps_doors::run(&flags, fresh, json_out);
    }
    let of_an_install = crate::maps_doors::OF_AN_INSTALL.iter();
    if let Some(flag) = of_an_install
        .copied()
        .find(|flag| flags.contains_key(*flag))
        .or(fresh.then_some("--fresh"))
    {
        return Err(Usage(format!(
            "{flag} says something of an install, and no --game <install root> names one"
        )));
    }
    let (project, modules) = maps_of(&positional, &flags)?;
    let patterns = or_maps::doors::built_in_patterns().map_err(|e| Run(e.to_string()))?;
    let listed = crate::maps_doors::names_listed(flags.get("--names"))?;
    let named = |hash: u32| listed.get(&hash).cloned();
    let mut reports = Vec::new();
    for id in modules {
        let module = (project.container.module(&id))
            .ok_or_else(|| Run(format!("the project has no maps content `{id}`")))?;
        let folder = or_maps::module_folder(&project.folder(), module)
            .map_err(|e| Run(format!("content `{id}`: {e}")))?;
        let model =
            or_maps::Maps::read(&folder).map_err(|e| Run(format!("content `{id}`: {e}")))?;
        let sets = or_maps::doors::doors_of(&folder, &model, Some(&named), &patterns);
        if json_out {
            reports.push(json!({ "module": id, "sets": sets }));
            continue;
        }
        println!("content {id}: {} map set(s)", sets.len());
        for line in sets.iter().flat_map(or_maps::doors::SetDoors::lines) {
            println!("  {line}");
        }
    }
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports).map_err(|e| Run(e.to_string()))?
        );
    } else {
        println!("{DOORS_READ_FROM}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Do the operations of a list to the sources of one maps content, all of them or none.
fn apply(args: &[String]) -> Result<ExitCode, Failure> {
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--dry-run").cloned().collect();
    let (positional, flags) = parse(&rest, &["--module"])?;
    let [path, list] = positional.as_slice() else {
        return Err(Usage(
            "expected <project.openrage-proj> <operations.json>".into(),
        ));
    };
    let (project, modules) = maps_of(std::slice::from_ref(path), &flags)?;
    let [id] = modules.as_slice() else {
        return Err(Run(format!(
            "{path} holds {} maps contents ({}): say which with --module",
            modules.len(),
            modules.join(", ")
        )));
    };
    let module = (project.container.module(id))
        .ok_or_else(|| Run(format!("{path} has no maps content `{id}`")))?;
    // ⚠ A dry run writes nothing, so it may read a linked content, which only the project that holds it may write.
    let folder = if dry_run {
        or_maps::module_folder(&project.folder(), module)
    } else {
        or_maps::module_folder_to_edit(Path::new(path), &project.container, module)
    }
    .map_err(|e| Run(format!("content `{id}`: {e}")))?;
    let bytes = std::fs::read(list).map_err(|e| Run(format!("{list}: {e}")))?;
    let operations = or_maps::operation::Operations::from_json(&bytes, list)
        .map_err(|e| Run(e.to_string()))?
        .operations;
    let applied =
        or_maps::operation::apply_all(&folder, &operations, dry_run).map_err(|e| match e {
            // The one failure after which a source may hold what the list wrote: it says which.
            or_maps::operation::Error::Write(_) => Run(e.to_string()),
            refused => Run(format!("{refused}; no source was changed")),
        })?;

    // ⚠ A file keeps the hash of a name alone, so the names the list states are remembered in the pack, as the Map screen remembers
    // the ones a person types: a door is found by its model's name.
    let stated: Vec<&str> = operations.iter().flat_map(|op| op.names()).collect();
    if !dry_run && !stated.is_empty() {
        let remembered =
            or_maps::Maps::read(&folder).and_then(|mut model| match model.remember(&stated) {
                true => model.write(&folder),
                false => Ok(()),
            });
        if let Err(e) = remembered {
            eprintln!("note: the names the operations state are not remembered: {e}");
        }
    }

    // Each file with the XML it held before the list and the XML it holds after: an operation taken back later in the list changes nothing.
    let mut files: Vec<(String, &[u8], &[u8])> = Vec::new();
    for (at, (operation, done)) in operations.iter().zip(&applied).enumerate() {
        let said = match (done.edited.changed(), done.added) {
            (false, _) => ": no change".to_string(),
            (true, Some(index)) => format!(": added at {index}"),
            (true, None) => String::new(),
        };
        println!(
            "  {}. {} of {}{said}",
            at + 1,
            operation.name(),
            operation.source()
        );
        for note in &done.notes {
            println!("     note: {note}");
        }
        let file = (done.edited.file.strip_prefix(&folder))
            .unwrap_or(&done.edited.file)
            .to_string_lossy()
            .replace('\\', "/");
        match files.iter_mut().find(|(held, ..)| *held == file) {
            Some((.., after)) => *after = &done.edited.after,
            None => files.push((file, &done.edited.before, &done.edited.after)),
        }
    }
    let changed: Vec<&str> = (files.iter())
        .filter(|(_, before, after)| before != after)
        .map(|(file, ..)| file.as_str())
        .collect();
    let verb = if dry_run { "would change" } else { "changed" };
    for file in &changed {
        println!("{verb} {file}");
    }
    println!(
        "content {id}: {} operation(s) {verb} {} source(s){}",
        operations.len(),
        changed.len(),
        if dry_run { "; nothing was written" } else { "" }
    );
    Ok(ExitCode::SUCCESS)
}

/// Keep manifests of the game in a maps content: the files named, or those of an install that state a map of the content.
fn keep_manifest(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--module", "--game", "--keys"])?;
    let Some((path, files)) = positional.split_first() else {
        return Err(Usage("expected a <project.openrage-proj>".into()));
    };
    let (project, modules) = maps_of(std::slice::from_ref(path), &flags)?;
    let [id] = modules.as_slice() else {
        return Err(Usage(format!(
            "{path} holds several maps contents ({}): name one with --module",
            modules.join(", ")
        )));
    };
    let module = project
        .container
        .module(id)
        .expect("chosen from the container");
    let folder = or_maps::module_folder_to_edit(Path::new(path), &project.container, module)
        .map_err(|e| Run(format!("content `{id}`: {e}")))?;
    let incoming: Vec<(String, Vec<u8>)> = match (flags.get("--game"), files) {
        (Some(_), [_, ..]) => {
            return Err(Usage(
                "name the manifests to keep, or an install with --game, and not both".into(),
            ))
        }
        (None, []) => {
            return Err(Usage(
                "expected the <_manifest.ymf> files to keep, or --game <install root>".into(),
            ))
        }
        (None, files) => files
            .iter()
            .map(|file| {
                std::fs::read(file)
                    .map(|bytes| (file.clone(), bytes))
                    .map_err(|e| Run(format!("{file}: {e}")))
            })
            .collect::<Result<_, _>>()?,
        (Some(root), []) => {
            let (_, facts, _) = planned(&project.folder(), module, Depth::Open, None)
                .map_err(|e| Run(format!("content `{id}`: {e}")))?;
            let wanted: Vec<u32> = (or_maps::facts::named_paths(
                &or_maps::Maps::read(&folder).map_err(|e| Run(e.to_string()))?,
            ))
            .filter_map(|source| facts.source(source)?.map().map(|map| map.name.hash))
            .collect();
            stating(Path::new(root), flags.get("--keys").map(Path::new), &wanted)?
        }
    };
    for (shown, bytes) in &incoming {
        let stated =
            or_world::manifest_build::stated_in(bytes).map_err(|e| Run(format!("{shown}: {e}")))?;
        let kept =
            or_maps::manifest::keep(&folder, bytes).map_err(|e| Run(format!("{shown}: {e}")))?;
        println!(
            "kept {shown} as {}: it states {} map(s)",
            kept.display(),
            stated.len()
        );
    }
    // ⚠ A kept manifest is the game's file: the project's ignore rules keep it out of a repository, as they do a source taken from the game.
    if let Err(e) = or_source::git::update(&project.folder()) {
        eprintln!("note: the ignore rules of the project are not updated: {e}");
    }
    println!(
        "A kept manifest is the game's own file: `project check-sharing` names it, and it is not yours to send or commit."
    );
    Ok(ExitCode::SUCCESS)
}

/// The manifests of the install at `root` that state one of the maps `wanted`, each with its path in the install.
fn stating(
    root: &Path,
    keys: Option<&Path>,
    wanted: &[u32],
) -> Result<Vec<(String, Vec<u8>)>, Failure> {
    let game = crate::keys::mount_all(root, keys).map_err(Run)?;
    let mut paths: Vec<String> = Vec::new();
    game.for_each_file(|logical, _container, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ymf") {
            paths.push(logical.to_string());
        }
    });
    paths.sort();
    let found: Vec<(String, Vec<u8>)> = paths
        .into_iter()
        .filter_map(|path| {
            let bytes = game.vfs.open(&path)?;
            // ⚠ A `.ymf` that is no manifest a build can write from is passed by: the install holds a few in another container.
            let stated = or_world::manifest_build::stated_in(&bytes).ok()?;
            wanted
                .iter()
                .any(|map| stated.contains_key(map))
                .then(|| (path, bytes.to_vec()))
        })
        .collect();
    if found.is_empty() {
        return Err(Run(format!(
            "no manifest of the install at {} states a map of the content, so there is none to keep",
            root.display()
        )));
    }
    Ok(found)
}

/// The entity `at` as a line names it.
fn entity_text(entity: &or_maps::delete::Entity) -> String {
    let [x, y, z] = entity.position;
    format!(
        "{} entity {}, {} at {x}, {y}, {z}",
        entity.at.source,
        entity.at.index,
        entity.archetype.shown()
    )
}

/// `count` of a thing, with the plural its noun takes.
pub(crate) fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The lines that say a plan of a deletion: what goes, what stays, what is left, what the sources do not reach, and each file with what changes in it.
pub(crate) fn deletion_lines(planned: &or_maps::delete::Deletion, applied: bool) -> Vec<String> {
    use or_maps::delete::Why;
    let mut lines = Vec::new();
    for going in &planned.goes {
        let why = match &going.why {
            Why::Chosen => "chosen".to_string(),
            Why::Under(parent) => {
                format!("under entity {} of {}", parent.index, parent.source)
            }
            Why::LeftWithNoChild => "left standing for nothing".to_string(),
        };
        lines.push(format!("  goes   {}: {why}", entity_text(&going.entity)));
    }
    for staying in &planned.stays {
        let mut stands = Vec::new();
        if staying.children_left > 0 {
            stands.push(format!(
                "{} still under it",
                counted(staying.children_left, "entity is", "entities are")
            ));
        }
        if staying.children_elsewhere > 0 {
            stands.push(format!(
                "it counts {} no source holds",
                counted(staying.children_elsewhere as usize, "child", "children")
            ));
        }
        lines.push(format!(
            "  stays  {}: {}; its count of children goes from {} to {}",
            entity_text(&staying.entity),
            stands.join(" and "),
            staying.counted,
            staying.counts
        ));
    }
    for left in &planned.ambiguous {
        let twins: Vec<String> = (left.twins.iter())
            .map(|twin| format!("entity {} of {}", twin.index, twin.source))
            .collect();
        lines.push(format!(
            "  left   {}: ambiguous, it cannot be told from {}, of the same archetype at the same pose; name them all to take them all",
            entity_text(&left.entity),
            twins.join(" and ")
        ));
    }
    for note in &planned.notes {
        lines.push(format!("  note   {note}"));
    }
    let verb = if applied { "changed" } else { "would change" };
    for file in &planned.files {
        let mut what = Vec::new();
        if !file.removed.is_empty() {
            let indexes: Vec<String> = file.removed.iter().map(usize::to_string).collect();
            what.push(format!(
                "{} taken out ({})",
                counted(file.removed.len(), "entity", "entities"),
                indexes.join(", ")
            ));
        }
        for lowered in &file.lowered {
            what.push(format!(
                "the count of children of entity {} lowered from {} to {}",
                lowered.entity, lowered.from, lowered.to
            ));
        }
        if file.renumbered > 0 {
            what.push(format!(
                "{} renumbered",
                counted(file.renumbered, "LOD link", "LOD links")
            ));
        }
        if file.renumbered_into_parent > 0 {
            what.push(format!(
                "{} into the parent map renumbered",
                counted(file.renumbered_into_parent, "LOD link", "LOD links")
            ));
        }
        lines.push(format!("{verb} {}: {}", file.file, what.join(", ")));
    }
    if !planned.goes.is_empty() {
        lines.push(
            "  stays  the collision and the occluders of what goes: a deletion does not reach them"
                .to_string(),
        );
    }
    lines
}

/// Plan the deletion of entities of one maps content with their chain of far models, print the plan, and write it with `--apply`.
fn delete(args: &[String]) -> Result<ExitCode, Failure> {
    let apply = args.iter().any(|a| a == "--apply");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--apply").cloned().collect();
    let (positional, flags) = parse(&rest, &["--module", "--script", "--radius"])?;
    let [path, named @ ..] = positional.as_slice() else {
        return Err(Usage(
            "expected <project.openrage-proj> <source>:<entity>...".into(),
        ));
    };
    if named.is_empty() {
        return Err(Usage(
            "name an entity to delete as <source>:<entity>".into(),
        ));
    }
    let chosen = (named.iter())
        .map(|text| {
            let parsed = text.rsplit_once(':').and_then(|(source, index)| {
                Some(or_maps::operation::At {
                    source: source.to_string(),
                    index: index.parse().ok()?,
                })
            });
            parsed.filter(|at| !at.source.is_empty()).ok_or_else(|| {
                Usage(format!(
                    "`{text}` names no entity: say <source>:<entity>, such as city.metadata.src:12"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let radius = match flags.get("--radius") {
        Some(text) => (text.parse::<f32>().ok())
            .filter(|metres| metres.is_finite() && *metres > 0.0)
            .ok_or_else(|| {
                Usage(format!(
                    "--radius takes metres above nought, and `{text}` is none"
                ))
            })?,
        None => or_maps::script_text::HIDE_RADIUS,
    };
    let forms = or_maps::script_text::ScriptForms::built_in().map_err(|e| Run(e.to_string()))?;
    let form_id = flags.get("--script").map_or("lua", String::as_str);
    let form = forms.form(form_id).ok_or_else(|| {
        let known: Vec<&str> = forms.forms.iter().map(|form| form.id.as_str()).collect();
        Usage(format!(
            "no script form is called `{form_id}`: the forms are {}",
            known.join(", ")
        ))
    })?;

    let (project, modules) = maps_of(std::slice::from_ref(path), &flags)?;
    let [id] = modules.as_slice() else {
        return Err(Run(format!(
            "{path} holds {} maps contents ({}): say which with --module",
            modules.len(),
            modules.join(", ")
        )));
    };
    let module = (project.container.module(id))
        .ok_or_else(|| Run(format!("{path} has no maps content `{id}`")))?;
    // ⚠ A plan writes nothing, so it may read a linked content, which only the project that holds it may write.
    let folder = if apply {
        or_maps::module_folder_to_edit(Path::new(path), &project.container, module)
    } else {
        or_maps::module_folder(&project.folder(), module)
    }
    .map_err(|e| Run(format!("content `{id}`: {e}")))?;

    let planned = or_maps::delete::plan(&folder, &chosen)
        .map_err(|e| Run(format!("content `{id}`: {e}; no source was changed")))?;
    let script = planned
        .script(form, radius, HIDE_DECIMALS)
        .map_err(|e| Run(e.to_string()))?;
    if apply {
        or_maps::delete::apply(&folder, &planned).map_err(|e| match e {
            // The one failure after which a source may hold what the deletion wrote: it says which.
            or_maps::operation::Error::Write(_) => Run(e.to_string()),
            refused => Run(format!("{refused}; no source was changed")),
        })?;
    }
    println!(
        "content {id}: {} named, {} to take out",
        counted(chosen.len(), "entity", "entities"),
        counted(planned.goes.len(), "entity", "entities"),
    );
    for line in deletion_lines(&planned, apply) {
        println!("{line}");
    }
    if !script.is_empty() {
        println!(
            "or hide by script, which changes no file and takes no collision away ({}, within {radius} m):",
            form.name
        );
        for line in &script {
            println!("  {line}");
        }
    }
    println!(
        "content {id}: {} {} {}{}",
        counted(planned.operations.len(), "operation", "operations"),
        if apply { "changed" } else { "would change" },
        counted(planned.files.len(), "source", "sources"),
        if apply {
            ""
        } else {
            "; nothing was written (write it with --apply)"
        }
    );
    Ok(ExitCode::SUCCESS)
}

/// The decimals a script line states a place with: a millimetre.
const HIDE_DECIMALS: usize = 3;

/// Read each resource the build of an output wrote against the maps content it holds.
fn verify(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--build", "--out"])?;
    let (project, plans) = planned_plans(&positional, &flags)?;
    let root = root_of(&positional[0], &flags);
    let mut clean = true;
    let mut seen = false;
    for plan in &plans {
        let Some(output) = project.container.output(&plan.output) else {
            continue;
        };
        let folder = root.join(&plan.resource);
        for module in (project.container.modules.iter())
            .filter(|m| m.kind == ModuleKind::Maps && output.take.contains(&m.id))
        {
            // ⚠ Open, not Stat: what a map file places is read at that depth.
            let Ok((model, facts, planned)) = planned(&project.folder(), module, Depth::Open, None)
            else {
                continue;
            };
            // ⚠ A split divides between contents and never cuts one, so a module is read in the resource that holds its files.
            let held = (planned.contribution.files.iter())
                .any(|f| plan.files.iter().any(|p| p.path == f.path));
            if !held && plans.len() > 1 {
                continue;
            }
            seen = true;
            let found = or_maps::verify::verify(&model, &facts, &folder);
            println!("content {} in {}:", module.id, folder.display());
            for check in &found.checks {
                let mark = if check.passed { "ok  " } else { "FAIL" };
                println!("  {mark} {}: {}", check.name, check.detail);
            }
            clean &= found.is_clean();
        }
    }
    if !seen {
        return Err(Run(format!(
            "{} holds no maps content that a build takes",
            positional[0]
        )));
    }
    if clean {
        println!("This checks the files. It does not test that GTA V loads them.");
    }
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn validate(args: &[String]) -> Result<ExitCode, Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let (positional, flags) = parse(&rest, &["--module"])?;
    let (project, modules) = maps_of(&positional, &flags)?;
    let mut blocked = false;
    let mut reports = Vec::new();
    for id in modules {
        let module = project
            .container
            .module(&id)
            .expect("chosen from the container");
        let (model, _, planned) = planned(&project.folder(), module, Depth::Open, None)
            .map_err(|e| Run(format!("content `{id}`: {e}")))?;
        blocked |= planned.findings.iter().any(or_maps::Finding::blocks);
        if json_out {
            reports.push(json!({ "module": id, "findings": planned.findings }));
            continue;
        }
        let count = |severity| {
            planned
                .findings
                .iter()
                .filter(|f| f.severity == severity && f.waived.is_none())
                .count()
        };
        println!(
            "content {id}: {} map set(s), {} error(s), {} warning(s), {} note(s), {} waived",
            model.maps.len(),
            count(Severity::Error),
            count(Severity::Warning),
            count(Severity::Info),
            planned
                .findings
                .iter()
                .filter(|f| f.waived.is_some())
                .count()
        );
        for f in &planned.findings {
            let word = match f.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "note",
            };
            match &f.waived {
                Some(reason) => {
                    println!("  waived  {:<24} {} (waived: {reason})", f.code, f.message)
                }
                None => println!("  {word:<7} {:<24} {}", f.code, f.message),
            }
        }
    }
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&reports).map_err(|e| Run(e.to_string()))?
        );
    }
    Ok(if blocked {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Name what each source of the content places and declares.
fn list(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--module"])?;
    let (project, modules) = maps_of(&positional, &flags)?;
    for id in modules {
        let module = project
            .container
            .module(&id)
            .expect("chosen from the container");
        let (model, facts, _) = planned(&project.folder(), module, Depth::Open, None)
            .map_err(|e| Run(format!("content `{id}`: {e}")))?;
        println!("content {id}: {} map set(s)", model.maps.len());
        for set in &model.maps {
            println!("  set {}: {} source(s)", set.name, set.sources.len());
            for path in &set.sources {
                let Some(source) = facts.source(path) else {
                    println!("    {path}: not readable");
                    continue;
                };
                match (&source.inside, source.map(), source.types()) {
                    (_, Some(map), _) => println!(
                        "    {path}: {} entity(ies), {} problem(s)",
                        map.entities.len(),
                        map.problems.len() + map.record_problems.len()
                    ),
                    (_, _, Some(types)) => {
                        let interiors = (types.archetypes.iter())
                            .filter_map(|a| a.interior.as_ref())
                            .collect::<Vec<_>>();
                        println!(
                            "    {path}: {} archetype(s), {} interior(s) with {} room(s) and {} portal(s)",
                            types.archetypes.len(),
                            interiors.len(),
                            interiors.iter().map(|i| i.rooms.len()).sum::<usize>(),
                            interiors.iter().map(|i| i.portals.len()).sum::<usize>(),
                        );
                    }
                    (Some(Err(why)), ..) => println!("    {path}: {why}"),
                    _ => println!("    {path}: a {} source", source.kind),
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::deletion_lines;
    use or_maps::delete::{
        Ambiguous, Deletion, Entity, FileChange, Going, Lowered, Note, Staying, Why,
    };
    use or_maps::facts::Name;
    use or_maps::operation::At;

    fn at(source: &str, index: usize) -> At {
        At {
            source: source.to_string(),
            index,
        }
    }

    fn entity(source: &str, index: usize, archetype: &str) -> Entity {
        Entity {
            at: at(source, index),
            archetype: Name::from_text(archetype),
            position: [1.5, -2.0, 30.25],
        }
    }

    #[test]
    fn the_door_survey_takes_the_colliders_a_mount_streams_and_names_one() {
        use or_depends::world_collision::Collider;
        let colliders: or_world::collision::CollisionSet<Collider> = Default::default();
        let patterns = or_maps::doors::built_in_patterns().unwrap();
        let (model, set) = (or_maps::Maps::new(), or_maps::MapSet::new("shop"));
        let surveyed =
            or_maps::doors::survey(&model, &set, &[], &[], &|_| None, &patterns, &colliders);
        assert!(surveyed.doors.is_empty());
        assert_eq!(Collider::File(3).to_string(), "bounds file 3 of the mount");
        assert_eq!(Collider::Placed(7).to_string(), "placed collider 7");
    }

    #[test]
    fn every_part_of_a_plan_has_its_line_and_a_part_that_is_empty_has_none() {
        let planned = Deletion {
            goes: vec![
                Going {
                    entity: entity("far", 4, "block_far"),
                    why: Why::Chosen,
                },
                Going {
                    entity: entity("near", 7, "house"),
                    why: Why::Under(at("far", 4)),
                },
                Going {
                    entity: entity("farther", 0, "block_farther"),
                    why: Why::LeftWithNoChild,
                },
            ],
            stays: vec![
                Staying {
                    entity: entity("farthest", 2, "district"),
                    children_left: 0,
                    children_elsewhere: 1,
                    counted: 2,
                    counts: 1,
                },
                Staying {
                    entity: entity("farthest", 3, "district"),
                    children_left: 2,
                    children_elsewhere: 3,
                    counted: 6,
                    counts: 5,
                },
            ],
            ambiguous: vec![Ambiguous {
                entity: entity("near", 1, "lamp"),
                twins: vec![at("near", 2), at("other", 5)],
            }],
            notes: vec![
                Note::ParentMapNotAmongSources {
                    entity: at("near", 7),
                    parent: Name::from_text("far_away"),
                },
                Note::NoParentMap {
                    entity: at("near", 8),
                },
                Note::ParentMapSplit {
                    entity: at("near", 9),
                    sources: vec!["far_a".to_string(), "far_b".to_string()],
                },
                Note::ParentPastTheEnd {
                    entity: at("near", 10),
                    index: 12,
                    len: 11,
                },
                Note::ChildrenNotAmongSources {
                    entity: at("far", 4),
                    counted: 3,
                    found: 1,
                },
            ],
            files: vec![
                FileChange {
                    source: "far".to_string(),
                    file: "far/metadata.xml".to_string(),
                    removed: vec![4],
                    lowered: vec![Lowered {
                        entity: 6,
                        from: 2,
                        to: 1,
                    }],
                    renumbered: 1,
                    renumbered_into_parent: 3,
                },
                FileChange {
                    source: "near".to_string(),
                    file: "near/metadata.xml".to_string(),
                    removed: Vec::new(),
                    lowered: Vec::new(),
                    renumbered: 0,
                    renumbered_into_parent: 1,
                },
            ],
            operations: Vec::new(),
            hidden: Vec::new(),
        };
        let place = "at 1.5, -2, 30.25";
        assert_eq!(
            deletion_lines(&planned, false),
            [
                format!("  goes   far entity 4, block_far {place}: chosen"),
                format!("  goes   near entity 7, house {place}: under entity 4 of far"),
                format!("  goes   farther entity 0, block_farther {place}: left standing for nothing"),
                format!("  stays  farthest entity 2, district {place}: it counts 1 child no source holds; its count of children goes from 2 to 1"),
                format!("  stays  farthest entity 3, district {place}: 2 entities are still under it and it counts 3 children no source holds; its count of children goes from 6 to 5"),
                format!("  left   near entity 1, lamp {place}: ambiguous, it cannot be told from entity 2 of near and entity 5 of other, of the same archetype at the same pose; name them all to take them all"),
                "  note   near entity 7 names its far model in the map far_away, which is not among the sources: the far model stays, and counts a child it lost".to_string(),
                "  note   near entity 8 names a far model in a parent map, and its map names none".to_string(),
                "  note   near entity 9 names its far model in a map that far_a and far_b each hold entities of, so its link says none of them".to_string(),
                "  note   near entity 10 names entity 12 of its parent map, which holds 11".to_string(),
                "  note   far entity 4 counts 3 children and the sources hold 1: the others are in a map that is not among the sources, and lose their parent".to_string(),
                "would change far/metadata.xml: 1 entity taken out (4), the count of children of entity 6 lowered from 2 to 1, 1 LOD link renumbered, 3 LOD links into the parent map renumbered".to_string(),
                "would change near/metadata.xml: 1 LOD link into the parent map renumbered".to_string(),
                "  stays  the collision and the occluders of what goes: a deletion does not reach them".to_string(),
            ]
        );

        let applied = deletion_lines(&planned, true);
        assert!(
            applied[11].starts_with("changed far/metadata.xml: "),
            "{applied:?}"
        );
        assert!(
            applied[12].starts_with("changed near/metadata.xml: "),
            "{applied:?}"
        );

        let nothing = Deletion {
            goes: Vec::new(),
            stays: Vec::new(),
            ambiguous: Vec::new(),
            notes: Vec::new(),
            files: Vec::new(),
            ..planned
        };
        assert_eq!(deletion_lines(&nothing, false), Vec::<String>::new());
    }
}
