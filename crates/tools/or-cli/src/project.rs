//! `openrage-cli project`: create a project, adopt a folder, link content, share a template, print the plan of a build, build it and verify what was built.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_project::adopt::analyse;
use or_project::container::{is_name, unique_id};
use or_project::links::{
    add_link, add_project_links, linked_sources, links, project_file_in, remove_link, Linked,
};
use or_project::project::{add_files, edit_container};
use or_project::template::{self, resource_name_for, Template};
use or_project::{
    build_all_watched, build_root, built_resources, check_reproducible, clean, find_sources,
    since_last_build, verify, BuildError, Built, Container, Cooks, Diff, Module, ModuleKind,
    Origin, Output, OutputKind, Plan, Project, Reproducible, SourceFolder, Target, Verification,
    PROJECT_EXTENSION,
};
use or_source::cook::{cook_source, describe};
use serde_json::Value;

/// A failed command: a usage error is the caller's, anything else the run's.
pub(crate) enum Failure {
    Usage(String),
    Run(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Run(message)
    }
}

use Failure::{Run, Usage};

pub fn run(args: &[String]) -> ExitCode {
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("create") => create(rest),
        Some("adopt") => adopt(rest),
        Some("add-content") => add_content(rest),
        Some("add-build") => add_build(rest),
        Some("remove-build") => remove_build(rest),
        Some("set-build") => set_build(rest),
        Some("set-file-type") => set_file_type(rest),
        Some("clean") => clean_of(rest),
        Some("redirects") => redirects_of(rest),
        Some("link") => link(rest),
        Some("unlink") => unlink(rest),
        Some("links") => links_of(rest),
        Some("template") => template_of(rest),
        Some("apply-template") => crate::project_checks::apply_template(rest),
        Some("check-sharing") => crate::project_checks::check_sharing(rest),
        Some("check-original") => crate::project_checks::check_original(rest),
        Some("check-resource") => crate::project_resource::check_resource(rest),
        Some("plan") => plan_of(rest),
        Some("build") => build_of(rest),
        Some("verify") => verify_of(rest),
        Some("acceptance") => crate::project_acceptance::acceptance(rest),
        Some("history") => crate::project_steps::history_of(rest),
        Some("rollback") => crate::project_steps::rollback_of(rest),
        Some("remove") => crate::project_steps::remove_of(rest),
        Some("recook") => crate::project_steps::recook_of(rest),
        Some("-h" | "--help" | "help") => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(Usage(format!("unknown project command: {other}"))),
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

const USAGE: &str = "Usage:\n  \
         openrage-cli project create <folder> [--name <project>] [--resource <name>]\n                              \
         [--kind fivem|dlc|oiv] [--target legacy|enhanced] [--game legacy|enhanced|both]\n                              \
         [--template <file>] [<file>...]\n  \
         openrage-cli project adopt  <folder> [--name <project>] [--resource <name>]\n  \
         openrage-cli project add-content  <project.openrage-proj> [--kind files|clothing] [<file>...]\n  \
         openrage-cli project add-build    <project.openrage-proj> [--name <resource>]\n                                    \
         [--kind fivem|dlc|oiv] [--target legacy|enhanced]\n  \
         openrage-cli project remove-build <project.openrage-proj> <id>\n  \
         openrage-cli project set-build <project.openrage-proj> <id> [--name <resource>]\n                                \
         [--kind fivem|dlc|oiv] [--target legacy|enhanced|authored] [--take <content>,...]\n                                \
         [--split on|off] [--max-files <n>|none] [--max-bytes <n>|none]\n                                \
         [--limits <figure>=<n>|none,...] [--ship-unchanged on|off]\n                                \
         [--author <text>|none] [--version <text>|none] [--description <text>|none]\n                                \
         [--clothing <option>=<value>,...]\n  \
         openrage-cli project set-file-type <project.openrage-proj> <path> <TYPE>|none\n  \
         openrage-cli project link   <project.openrage-proj> <folder> [--id <id>] [--only files]\n  \
         openrage-cli project unlink <project.openrage-proj> <id>\n  \
         openrage-cli project links  <project.openrage-proj>\n  \
         openrage-cli project template <project.openrage-proj> <file.openrage-template>\n                              \
         [--name <template>] [--description <text>]\n  \
         openrage-cli project apply-template <project.openrage-proj> <file.openrage-template>\n  \
         openrage-cli project check-sharing  <project.openrage-proj> [--json]\n  \
         openrage-cli project check-original <project.openrage-proj> --game <install root>\n                                      \
         [--keys <file>] [--source <source folder>] [--json]\n  \
         openrage-cli project check-resource <resource folder> [--game <install root>] [--keys <file>]\n                                      \
         [--limits file_memory=<bytes>] [--json]\n  \
         openrage-cli project plan   <project.openrage-proj> [--build <id>] [--out <folder>]\n  \
         openrage-cli project build  <project.openrage-proj> [--build <id>] [--out <folder>]\n                              \
         [--shader-defs <json>] [--reproducible]\n  \
         openrage-cli project verify <project.openrage-proj> [--build <id>] [--out <folder>]\n  \
         openrage-cli project acceptance <built folder>\n  \
         openrage-cli project clean  <project.openrage-proj> [--build <id>] [--out <folder>]\n  \
         openrage-cli project redirects <project.openrage-proj> [--build <id>] [--out <folder>]\n                                 \
         [--file <path>]\n  \
         openrage-cli project history  <project.openrage-proj> [--json]\n  \
         openrage-cli project rollback <project.openrage-proj> <step>\n  \
         openrage-cli project remove   <project.openrage-proj> <file>...\n  \
         openrage-cli project recook   <project.openrage-proj> <source folder>...\n\n\
         create writes <folder>/<project>.openrage-proj: one Files content holding the\n\
         files named, and one build named <resource>: a FiveM resource (fivem, the default),\n\
         a singleplayer DLC (dlc) or an OIV package (oiv). It makes the folder's sources/ and\n\
         records the game the project is for (legacy, enhanced or both, the default), and a\n\
         build ships for the edition of its game unless --target says another. With --target\n\
         every drawable and texture dictionary the build ships is moved to that layout.\n\
         Nothing is moved on disk. With --template the content and builds are the template's,\n\
         the first build is named for the resource and the others for the resource and their\n\
         own id, and a clothing or addon ped content gets its folder and an empty pack or model.\n\
         adopt makes a project of a folder that exists, and moves nothing: a clothing pack in it\n\
         becomes clothing content (a wardrobe.json is added beside its files), the peds it streams\n\
         become addon ped content (an addonped.json), what no content claims is the Files\n\
         content, and what could not be placed is printed and recorded in\n\
         the project file.\n\
         add-content adds the files named to the Files content, once each, and says why it\n\
         will not take one a build could not place. With --kind clothing it makes a clothing\n\
         folder with an empty pack instead, and every build takes it.\n\
         add-build declares a build that takes all the content, named for the project unless\n\
         --name says another (a second one is named for what it is, <project>_dlc or\n\
         <project>_enhanced, and <project>_v2 after that, never <project>_2, which a split\n\
         writes), for the edition of the project's game unless --target says another.\n\
         remove-build stops declaring a build and touches no file; clean removes what it left.\n\
         set-build changes a build as the Builds window does: its name, kind and edition, the\n\
         content it takes (--take lists every content it takes, so the rest are left out),\n\
         whether it divides when over a ceiling, its ceilings on files and size and on a named\n\
         figure such as clothing_drawables, and whether a source that is still the game's file\n\
         ships. none takes a ceiling away. --author, --version and --description are the words a\n\
         FiveM manifest or a package states of the build (a package takes major.minor for a\n\
         version), and --clothing sets how a clothing pack is written for the build:\n\
         subfolders=on|off, creature-metadata=on|off, alternates=on|off, collection-prefix=<text>|none\n\
         and clothes-dump=off|json|both. set-file-type states the type a singleplayer DLC\n\
         registers a file with, for a file of the Files content (none takes it away).\n\
         link makes the folder of a pack, which another project holds, content of this one: the\n\
         other project is its only writer, and every build takes it. The folder of a project\n\
         links everything that project holds of its own, its Files content and each pack, and\n\
         none of the links it holds itself, which a link does not follow; --only files links\n\
         just its Files content. unlink stops that and touches no file. links lists them, with\n\
         the project that holds each.\n\
         template writes the content, builds and layout of a project as a file another project\n\
         can be made from. It holds no file, no link and no path of this machine.\n\
         apply-template adds what a template file states and the project lacks, as Project >\n\
         Apply a template does: each content and build whose id the project does not hold, and\n\
         nothing it holds is replaced. check-sharing lists what the folder holds that is not yours\n\
         to send: paths of this machine in the project file, sources taken from the game, and the\n\
         private folders. It writes nothing and exits 1 for a path of this machine or a source\n\
         taken from the game, so a pre-push hook can run it; the private folders are told and\n\
         do not fail it, since Git never sees them. check-original reads the install --game names and says, for each source taken\n\
         from the game (or the one --source names), whether the game's file it was taken from is\n\
         still that file; it exits 1 where the file changed or is gone.\n\
         check-resource reads any resource folder as a server will read it, whoever made it and with\n\
         no project: its manifest (fxmanifest.lua, or the older __resource.lua), the files the manifest\n\
         lists and its data-file lines name, and every file under stream/. It lists what its rules find,\n\
         each with a code, a severity, the file and a sentence: no manifest; no version line; a file\n\
         named and not there; a path that leaves the resource, which is reported and not followed; one\n\
         file name twice under stream/; a file of a type a server does not stream; a streamed file that\n\
         does not open through OpenRage's reader of its type, with the reader's error, or that is a\n\
         map, a type file or a .ymt kept as text, which no game streams; a file over the\n\
         ceiling --limits file_memory=<bytes> states for the streaming memory of one file (no file is\n\
         weighed without it); maps and no this_is_a_map line; and the names nothing holds: the model\n\
         or the texture dictionary of an archetype, and an archetype a map places. With --game the\n\
         install is asked about a name the resource lacks, and without it those are notes. A file\n\
         locked by a server's asset escrow is counted, named and not opened, as a note. A line of the\n\
         manifest that is not a key followed by text or a table of text is a note, and the manifest\n\
         is never run. It writes nothing and exits 1 when a finding is an error; --json prints the\n\
         same findings as data. It cannot see whether the game loads a file, whether a server starts\n\
         the resource, or what another resource of the server ships.\n\
         plan prints what a build will write, the budgets, what changed since the last\n\
         build and why it cannot be built, without writing. A build that declares\n\
         split and is over a ceiling divides at content boundaries into several\n\
         resources, <resource>_1, <resource>_2 and so on.\n\
         build cooks the .src folders of the project first, each into the game file it\n\
         stands for, and leaves out a source of the game's own content that has not changed\n\
         (the build's ship_unchanged key ships it). It writes <folder>/build/<resource> (or\n\
         <out>/<resource>) to a fresh folder, reads every file back, swaps it in and keeps\n\
         the previous build. It refuses a\n\
         folder that is, holds or is inside the project (but for build/), and a folder\n\
         it did not build. Of a split build it checks every resource before it writes\n\
         the first, so a refusal stops them all; a resource that fails while it is\n\
         written leaves the ones before it built. A DLC is <resource>/dlc.rpf, and the\n\
         dlclist.xml line it needs is printed; a package is <resource>/<resource>.oiv.\n\
         A file that cannot move to the target stops the build. --shader-defs, or the\n\
         database OPENRAGE_SHADER_DEFS names, respells the parameters of a drawable that\n\
         moves; with none, they are dropped and named. With --reproducible it then builds\n\
         again into a scratch folder and says whether every byte is the same.\n\
         verify reads a built resource back against the plan the project has now.\n\
         acceptance reads the folder a build wrote (a resource, or the folder of a dlc.rpf or an\n\
         .oiv) and prints the lines of the acceptance sheet about the kinds of file in it: what to\n\
         do in the game, what to look for, and for Legacy and for Enhanced the newest result a\n\
         person wrote into the sheet, or that the line is not checked in game. It writes nothing,\n\
         and exits 0 whatever the sheet holds.\n\
         clean removes the resources OpenRage built under build/ (or <out>), found by their\n\
         hidden marker, or with --build only the resources of that build: its name, and\n\
         <name>_1, <name>_2 and so on when it splits and no build is named so.\n\
         redirects writes the redirect map of what moved since the last build recorded its\n\
         items, to --file or to the terminal: run it before the build that moves them.\n\
         history lists the steps the editor kept of the project (.openrage/history), oldest first, and marks the\n\
         one it stands at. rollback stands the project at a step, as the editor's History does: going back over an\n\
         import takes back the sources it made while each is as the import left it, and refuses, with nothing\n\
         moved, once one was changed. remove takes files out of the project's documents and keeps a step, so that\n\
         rollback brings them back; the files stay on disk. recook cooks the source folders named now into the\n\
         project's shadow folder, as a saved file in one does.\n\n\
         A build checks the files. It does not test that GTA V loads them, or that\n\
         OpenIV installs a package.\n";

fn print_usage() {
    eprintln!("{USAGE}");
}

/// The flag that names a build of the project.
pub(crate) const BUILD: &str = "--build";

/// The flags a command takes, each with a value, and its positional arguments.
pub(crate) fn parse(
    args: &[String],
    takes: &[&str],
) -> Result<(Vec<String>, HashMap<String, String>), Failure> {
    let mut positional = Vec::new();
    let mut flags = HashMap::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        // ⚠ `--output` is the older spelling of `--build`, kept so a script that uses it still runs; the usage does not name it.
        let name = match arg.as_str() {
            "--output" if takes.contains(&BUILD) => BUILD,
            other => other,
        };
        if !arg.starts_with("--") {
            positional.push(arg.clone());
        } else if takes.contains(&name) {
            let value = args
                .next()
                .ok_or_else(|| Usage(format!("{arg} needs a value")))?;
            flags.insert(name.to_string(), value.clone());
        } else {
            return Err(Usage(format!("unknown flag: {arg}")));
        }
    }
    Ok((positional, flags))
}

fn create(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(
        args,
        &[
            "--name",
            "--resource",
            "--kind",
            "--target",
            "--template",
            "--game",
        ],
    )?;
    let game = game_of(flags.get("--game").map(String::as_str))?;
    let Some((folder, files)) = positional.split_first() else {
        return Err(Usage("expected a <folder> for the project".into()));
    };
    let folder = PathBuf::from(folder);
    let name = match flags.get("--name") {
        Some(name) => name.clone(),
        None => std::path::absolute(&folder)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default(),
    };
    if !is_name(&name) {
        return Err(Usage(format!(
            "{name:?} is not a project name: it becomes a file name, so say one with --name"
        )));
    }
    let resource = flags.get("--resource").unwrap_or(&name);
    if !is_name(resource) {
        return Err(Usage(format!(
            "{resource:?} is not a folder name for the resource"
        )));
    }
    for file in files {
        if !Path::new(file).is_file() {
            return Err(Run(format!("{file} is not a file")));
        }
    }
    if let Some(template) = flags.get("--template") {
        if flags.contains_key("--kind") || flags.contains_key("--target") {
            return Err(Usage(
                "a template states its builds, so --kind and --target do not go with --template"
                    .into(),
            ));
        }
        let said = flags.get("--game").map(|_| game);
        return create_from(&folder, &name, resource, files, Path::new(template), said);
    }

    let (id, kind) = build_kind_of(flags.get("--kind").map(String::as_str))?;
    let target = match flags.get("--target").map(String::as_str) {
        None => Target::from_word(game),
        Some(word) => Some(
            Target::from_word(word)
                .ok_or_else(|| Usage(format!("unknown --target {word}: legacy or enhanced")))?,
        ),
    };

    let mut output = Output::of(kind, id, resource, &["files"]);
    output.target = target;
    let label = output.kind.label();
    let container = Container {
        modules: vec![Module::files("files")],
        outputs: vec![output],
    };
    let path = folder.join(format!("{name}.{PROJECT_EXTENSION}"));
    let members: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
    refuse_outside(&container, &path, &members)?;
    let mut extra = serde_json::Map::new();
    extra.insert(GAME_KEY.into(), Value::String(game.into()));
    Project::create_with(&path, &container, &members, extra)
        .map_err(|e| Run(format!("{}: {e}", path.display())))?;
    make_sources(&folder)?;

    println!("created {}", path.display());
    println!("  game:    {game}");
    println!("  content: files, holding {} file(s)", members.len());
    match target {
        Some(target) => println!(
            "  build:   {id}, a {label} named {resource}, for {}",
            target.generation().edition_label()
        ),
        None => println!("  build:   {id}, a {label} named {resource}"),
    }
    Ok(ExitCode::SUCCESS)
}

/// Say, for each of `files`, why it is not a member of the project at `path` when it is not.
fn refuse_outside(container: &Container, path: &Path, files: &[PathBuf]) -> Result<(), Failure> {
    for file in files {
        if let Some(why) = or_project::links::refuse_member(container, path, file) {
            return Err(Run(why));
        }
    }
    Ok(())
}

/// A project made from the template in `file`, in `folder`.
fn create_from(
    folder: &Path,
    name: &str,
    resource: &str,
    files: &[String],
    file: &Path,
    game: Option<&str>,
) -> Result<ExitCode, Failure> {
    let shown = file.display();
    let bytes = std::fs::read(file).map_err(|e| Run(format!("{shown}: {e}")))?;
    let (template, findings) =
        Template::from_json(&bytes).map_err(|why| Run(format!("{shown}: {why}")))?;
    for finding in &findings {
        eprintln!("warning: {finding}");
    }
    let mut made = template.instantiate(resource).map_err(Run)?;
    // The game is the one said, else the template's, else both; a build that states no edition ships for the game's own.
    let mut keys = template.project_keys();
    let stated = keys
        .get(GAME_KEY)
        .and_then(Value::as_str)
        .map(str::to_string);
    let game = match (game, stated) {
        (Some(said), _) => said.to_string(),
        (None, Some(stated)) => game_of(Some(&stated))?.to_string(),
        (None, None) => "both".to_string(),
    };
    keys.insert(GAME_KEY.into(), Value::String(game.clone()));
    for output in &mut made.container.outputs {
        output.target = output.target.or(Target::from_word(&game));
    }
    let path = folder.join(format!("{name}.{PROJECT_EXTENSION}"));
    if path.exists() {
        return Err(Run(format!("{} already exists", path.display())));
    }
    let members: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
    refuse_outside(&made.container, &path, &members)?;
    let pack = resource_name_for(resource);
    for needed in &made.folders {
        if let Some(module) = made.container.module(&needed.module) {
            or_content::start_module(folder, module, &pack).map_err(Run)?;
        }
    }
    Project::create_with(&path, &made.container, &members, keys)
        .map_err(|e| Run(format!("{}: {e}", path.display())))?;
    make_sources(folder)?;
    println!(
        "created {} from the template `{}`",
        path.display(),
        template.name
    );
    for module in &made.container.modules {
        println!("  content: {}", module.id);
    }
    for output in &made.container.outputs {
        println!(
            "  build:   {}, a {} named {}",
            output.id,
            output.kind.label(),
            output.name
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// The key of the project file that says which game the project is for.
const GAME_KEY: &str = "game";

/// The game a project is for as its file spells it: `legacy`, `enhanced` or `both`, which is the default.
fn game_of(word: Option<&str>) -> Result<&'static str, Failure> {
    match word {
        None | Some("both") => Ok("both"),
        Some("legacy") => Ok("legacy"),
        Some("enhanced") => Ok("enhanced"),
        Some(other) => Err(Usage(format!(
            "unknown --game {other}: legacy, enhanced or both"
        ))),
    }
}

/// The folder a project keeps its sources in, made beside its file.
fn make_sources(folder: &Path) -> Result<(), Failure> {
    let sources = folder.join("sources");
    std::fs::create_dir_all(&sources).map_err(|e| Run(format!("{}: {e}", sources.display())))
}

fn adopt(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--name", "--resource"])?;
    let [folder] = positional.as_slice() else {
        return Err(Usage("expected one <folder> to adopt".into()));
    };
    let folder = PathBuf::from(folder);
    if !folder.is_dir() {
        return Err(Run(format!("{} is not a folder", folder.display())));
    }
    let held = std::fs::read_dir(&folder)
        .map_err(|e| Run(format!("{}: {e}", folder.display())))?
        .flatten()
        .map(|entry| entry.path())
        .find(|p| p.extension().is_some_and(|x| x == PROJECT_EXTENSION));
    if let Some(held) = held {
        return Err(Run(format!(
            "{} already holds the project {}; open it instead",
            folder.display(),
            held.file_name().unwrap_or_default().to_string_lossy()
        )));
    }
    let name = match flags.get("--name") {
        Some(name) => name.clone(),
        None => std::path::absolute(&folder)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default(),
    };
    if !is_name(&name) {
        return Err(Usage(format!(
            "{name:?} is not a project name: it becomes a file name, so say one with --name"
        )));
    }
    let resource = flags
        .get("--resource")
        .cloned()
        .unwrap_or_else(|| resource_name_for(&name));
    if !is_name(&resource) {
        return Err(Usage(format!(
            "{resource:?} is not a folder name for the resource"
        )));
    }

    let adoption = analyse(&folder, &or_content::recognisers());
    for found in &adoption.found {
        println!("content {}: {}", found.module.id, found.summary);
    }
    if !adoption.files.is_empty() || !adoption.recognised() {
        println!("content files: {} file(s)", adoption.files.len());
    }
    for note in &adoption.notes {
        println!("note: {note}");
    }
    for left in &adoption.unplaced {
        println!("not placed {}: {}", left.path, left.why);
    }
    let written = adoption
        .write()
        .map_err(|e| Run(format!("{}: {e}", folder.display())))?;
    let file = folder.join(format!("{name}.{PROJECT_EXTENSION}"));
    let mut extra = serde_json::Map::new();
    if let Some(record) = adoption.record() {
        extra.insert(or_project::adopt::ADOPTED_KEY.into(), record);
    }
    let container = adoption.container(&resource);
    if let Err(e) = Project::create_with(&file, &container, &adoption.files, extra) {
        written.undo();
        return Err(Run(format!("{}: {e}", file.display())));
    }
    println!(
        "created {}; nothing in the folder was moved or rewritten",
        file.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Link a pack by its folder, a project by its folder (every content it holds of its own), or with `--only files` just the Files content of a project.
fn link(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--id", "--only"])?;
    let [project, target] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj> and the <folder> to link".into(),
        ));
    };
    let (project, target) = (Path::new(project), PathBuf::from(target));
    let only_files = match flags.get("--only").map(String::as_str) {
        None => false,
        Some("files") => true,
        Some(other) => return Err(Usage(format!("unknown --only {other}: files"))),
    };
    let whole = !only_files && project_file_in(&target).is_some();
    if whole && flags.contains_key("--id") {
        return Err(Usage(
            "--id names one link, and a project is linked as several: link its Files content with --only files to name it".into(),
        ));
    }
    let linked = or_project::project::edit_container(project, |container| {
        if whole {
            return add_project_links(container, project, &target);
        }
        let kind = if only_files {
            ModuleKind::Files
        } else {
            or_content::recognisers()
                .iter()
                .find(|r| r.holds(&target))
                .map(|r| r.kind())
                .ok_or_else(|| {
                    format!(
                        "{} holds no content this build knows: a clothing folder holds a wardrobe.json, and a project holds its project file",
                        target.display()
                    )
                })?
        };
        let id = add_link(
            container,
            project,
            kind,
            &target,
            flags.get("--id").map(String::as_str),
        )?;
        Ok(Linked {
            ids: vec![id],
            ..Linked::default()
        })
    })
    .map_err(Run)?;
    let loaded = Project::load(project).map_err(|e| Run(format!("{}: {e}", project.display())))?;
    for id in &linked.ids {
        println!("linked {id}");
        if let Some(link) = links(&loaded.container, project)
            .iter()
            .find(|l| &l.module == id)
        {
            println!("  {}", link.banner());
        }
        let taking: Vec<&str> = (loaded.container.outputs.iter())
            .filter(|o| o.take.contains(id))
            .map(|o| o.id.as_str())
            .collect();
        println!("  taken by the build(s): {}", taking.join(", "));
    }
    if !linked.not_followed.is_empty() {
        println!(
            "not followed: that project links {} itself, and a link is not followed; link {} here to ship them",
            linked.not_followed.join(", "),
            if linked.not_followed.len() == 1 { "it" } else { "each" }
        );
    }
    for why in &linked.left {
        println!("not linked: {why}");
    }
    Ok(ExitCode::SUCCESS)
}

fn unlink(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, _) = parse(args, &[])?;
    let [project, id] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj> and the <id> of the link".into(),
        ));
    };
    or_project::project::edit_container(Path::new(project), |container| remove_link(container, id))
        .map_err(Run)?;
    println!("unlinked {id}; no file was touched");
    Ok(ExitCode::SUCCESS)
}

fn links_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, _) = parse(args, &[])?;
    let [path] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let project = Project::load(Path::new(path)).map_err(|e| Run(format!("{path}: {e}")))?;
    let found = links(&project.container, Path::new(path));
    if found.is_empty() {
        println!("{path} links no content");
    }
    let mut sound = true;
    for link in &found {
        println!(
            "link {}: {} -> {}",
            link.module,
            link.stored,
            link.folder.display()
        );
        println!("  {}", link.banner());
        if let Some(problem) = &link.problem {
            sound = false;
            println!("  problem: {problem}");
        }
    }
    Ok(if sound {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `--kind files` (the default) adds the files named to the Files content; `--kind clothing` makes a clothing folder and an empty pack.
fn add_content(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--kind"])?;
    let Some((project, files)) = positional.split_first() else {
        return Err(Usage(
            "expected a <project.openrage-proj> to add content to".into(),
        ));
    };
    let project = Path::new(project);
    match flags.get("--kind").map(String::as_str) {
        None | Some("files") => add_the_files(project, files),
        Some("clothing") if files.is_empty() => add_clothing(project),
        Some("clothing") => Err(Usage("--kind clothing takes no file".into())),
        Some(other) => Err(Usage(format!("unknown --kind {other}: files or clothing"))),
    }
}

fn add_the_files(project: &Path, files: &[String]) -> Result<ExitCode, Failure> {
    if files.is_empty() {
        return Err(Usage("expected the files to add after the project".into()));
    }
    let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
    if let Some(missing) = files.iter().find(|file| !file.is_file()) {
        return Err(Run(format!("{} is not a file", missing.display())));
    }
    let done = add_files(project, &files).map_err(Run)?;
    println!(
        "added {} file(s) to the Files content; {} held already",
        done.added, done.held
    );
    for why in &done.refused {
        println!("not added: {why}");
    }
    Ok(if done.refused.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// A `clothing` folder holding an empty pack, a Clothing content for it, and a place for it in every build.
fn add_clothing(path: &Path) -> Result<ExitCode, Failure> {
    let project = Project::load(path).map_err(|e| Run(format!("{}: {e}", path.display())))?;
    let folder = project.folder();
    let taken = project.container.modules.iter().map(|m| m.id.as_str());
    let id = unique_id("clothing", taken);
    // ⚠ Two packs of one name claim the same collections, so a second pack is named for the project and a number.
    let wanted = or_wardrobe::names::identifier_for(
        &path
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned()),
    );
    let packs: Vec<String> = (project.container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Clothing)
        .filter_map(|m| or_wardrobe::module_folder(&folder, m).ok())
        .filter_map(|at| or_wardrobe::Wardrobe::read(&at).ok())
        .map(|w| w.pack)
        .collect();
    let pack = (1..)
        .map(|n| match n {
            1 => wanted.clone(),
            n => format!("{wanted}_{n}"),
        })
        .find(|name| !packs.contains(name))
        .unwrap_or_else(|| wanted.clone());
    let at = folder.join(&id);
    or_project::links::refuse_write(&project.container, path, &at.join(or_wardrobe::FILE))
        .map_err(Run)?;
    or_wardrobe::edit::create(&at, &pack).map_err(|e| Run(e.to_string()))?;
    edit_container(path, |container| {
        container.modules.push(Module::clothing(&id, &id));
        for build in &mut container.outputs {
            build.take.push(id.clone());
        }
        Ok(())
    })
    .map_err(Run)?;
    println!(
        "added the clothing content {id}: the pack `{pack}` in {}",
        at.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// The kind of build a `--kind` word names: a FiveM resource, a singleplayer DLC or an OIV package.
fn build_kind_of(word: Option<&str>) -> Result<(&'static str, OutputKind), Failure> {
    match word {
        None | Some("fivem") => Ok(("fivem", OutputKind::FivemResource)),
        Some("dlc") => Ok(("dlc", OutputKind::SingleplayerDlc)),
        Some("oiv") => Ok(("oiv", OutputKind::OivPackage)),
        Some(other) => Err(Usage(format!("unknown --kind {other}: fivem, dlc or oiv"))),
    }
}

/// A build that takes all the content, named for the project unless `--name` says another, for the edition of the project's game unless `--target` does.
///
/// ⚠ A second build is named for what it is (`<project>_dlc`, `<project>_enhanced`) and numbered `_v2` after that, never `_2`: a split writes its second resource as `<name>_2`.
fn add_build(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--name", "--kind", "--target"])?;
    let [path] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let path = Path::new(path);
    let (_, kind) = build_kind_of(flags.get("--kind").map(String::as_str))?;
    let loaded = Project::load(path).map_err(|e| Run(format!("{}: {e}", path.display())))?;
    let target = match flags.get("--target").map(String::as_str) {
        Some(word) => Some(
            Target::from_word(word)
                .ok_or_else(|| Usage(format!("unknown --target {word}: legacy or enhanced")))?,
        ),
        None => game_edition(path),
    };
    let name = match flags.get("--name") {
        Some(name) => name.clone(),
        None => {
            let base = path
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            loaded.container.free_build_name(&base, &kind, target)
        }
    };
    let label = kind.label();
    let id =
        edit_container(path, |container| container.add_build(&name, kind, target)).map_err(Run)?;
    println!("added the build {id}: a {label} named {name}");
    Ok(ExitCode::SUCCESS)
}

/// The edition of the game the project's file records, none for both or for a file that records none.
fn game_edition(path: &Path) -> Option<Target> {
    let top: serde_json::Map<String, Value> =
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Target::from_word(top.get(GAME_KEY)?.as_str()?)
}

fn remove_build(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, _) = parse(args, &[])?;
    let [project, id] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj> and the <id> of the build".into(),
        ));
    };
    edit_container(Path::new(project), |container| {
        if container.remove_build(id) {
            Ok(())
        } else {
            Err(format!("{project} declares no build `{id}`"))
        }
    })
    .map_err(Run)?;
    println!("removed the build {id}; what it wrote stays until clean removes it");
    Ok(ExitCode::SUCCESS)
}

/// What `set-build` changes about a build, each as the flags state it; a flag that is not given changes nothing.
#[derive(Default)]
struct BuildChanges {
    name: Option<String>,
    kind: Option<OutputKind>,
    /// `Some(None)` ships each file as authored.
    target: Option<Option<Target>>,
    take: Option<Vec<String>>,
    split: Option<bool>,
    /// `Some(None)` takes the ceiling away.
    max_files: Option<Option<u64>>,
    max_bytes: Option<Option<u64>>,
    limits: Vec<(String, Option<u64>)>,
    ship_unchanged: Option<bool>,
    /// The author, version and description the build states; `None` takes the statement away.
    words: Vec<(&'static str, Option<String>)>,
    /// The clothing options, each as `key=value`.
    clothing: Vec<(String, String)>,
}

impl BuildChanges {
    fn from_flags(flags: &HashMap<String, String>) -> Result<Self, Failure> {
        let on_off = |flag: &str| -> Result<Option<bool>, Failure> {
            match flags.get(flag).map(String::as_str) {
                None => Ok(None),
                Some("on") => Ok(Some(true)),
                Some("off") => Ok(Some(false)),
                Some(other) => Err(Usage(format!("{flag} is on or off, not {other}"))),
            }
        };
        let ceiling = |flag: &str| -> Result<Option<Option<u64>>, Failure> {
            flags
                .get(flag)
                .map(|text| count_or_none(flag, text))
                .transpose()
        };
        let mut limits = Vec::new();
        for pair in flags.get("--limits").iter().flat_map(|l| l.split(',')) {
            let (figure, text) = pair
                .split_once('=')
                .ok_or_else(|| Usage(format!("--limits takes figure=count pairs, not {pair}")))?;
            limits.push((figure.trim().to_string(), count_or_none("--limits", text)?));
        }
        let mut words = Vec::new();
        for (flag, key) in [
            ("--author", "author"),
            ("--version", "version"),
            ("--description", "description"),
        ] {
            if let Some(text) = flags.get(flag) {
                words.push((key, (text != "none").then(|| text.clone())));
            }
        }
        let mut clothing = Vec::new();
        for pair in flags.get("--clothing").iter().flat_map(|l| l.split(',')) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| Usage(format!("--clothing takes option=value pairs, not {pair}")))?;
            let (key, value) = (key.trim(), value.trim());
            or_wardrobe::options::Options::default()
                .set(key, value)
                .map_err(Usage)?;
            clothing.push((key.to_string(), value.to_string()));
        }
        Ok(Self {
            name: flags.get("--name").cloned(),
            kind: flags
                .get("--kind")
                .map(|word| build_kind_of(Some(word)).map(|(_, kind)| kind))
                .transpose()?,
            target: flags
                .get("--target")
                .map(|word| match word.as_str() {
                    "authored" => Ok(None),
                    word => Target::from_word(word).map(Some).ok_or_else(|| {
                        Usage(format!(
                            "unknown --target {word}: legacy, enhanced or authored"
                        ))
                    }),
                })
                .transpose()?,
            take: flags.get("--take").map(|list| {
                list.split(',')
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .collect()
            }),
            split: on_off("--split")?,
            max_files: ceiling("--max-files")?,
            max_bytes: ceiling("--max-bytes")?,
            limits,
            ship_unchanged: on_off("--ship-unchanged")?,
            words,
            clothing,
        })
    }

    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.kind.is_none()
            && self.target.is_none()
            && self.take.is_none()
            && self.split.is_none()
            && self.max_files.is_none()
            && self.max_bytes.is_none()
            && self.limits.is_empty()
            && self.ship_unchanged.is_none()
            && self.words.is_empty()
            && self.clothing.is_empty()
    }

    fn apply(&self, output: &mut Output) {
        if let Some(name) = &self.name {
            output.name.clone_from(name);
        }
        if let Some(kind) = &self.kind {
            output.kind = kind.clone();
        }
        if let Some(target) = self.target {
            output.target = target;
        }
        if let Some(take) = &self.take {
            output.take.clone_from(take);
        }
        if let Some(split) = self.split {
            output.split = split;
        }
        if let Some(max) = self.max_files {
            output.max_files = max;
        }
        if let Some(max) = self.max_bytes {
            output.max_bytes = max;
        }
        for (figure, ceiling) in &self.limits {
            match ceiling {
                Some(ceiling) => output.limits.insert(figure.clone(), *ceiling),
                None => output.limits.remove(figure),
            };
        }
        if let Some(ship) = self.ship_unchanged {
            output.ship_unchanged = ship;
        }
        for (key, text) in &self.words {
            match text {
                Some(text) => {
                    output
                        .extra
                        .insert((*key).to_string(), Value::String(text.clone()));
                }
                None => {
                    output.extra.remove(*key);
                }
            }
        }
        if !self.clothing.is_empty() {
            let (mut options, _) = or_wardrobe::options::Options::of(output);
            for (key, value) in &self.clothing {
                // Checked when the flags were read.
                let _ = options.set(key, value);
            }
            options.store(output);
        }
    }
}

/// A count, or `none` for no ceiling.
fn count_or_none(flag: &str, text: &str) -> Result<Option<u64>, Failure> {
    match text.trim() {
        "none" => Ok(None),
        count => count
            .parse()
            .map(Some)
            .map_err(|_| Usage(format!("{flag} takes a count or none, not {count}"))),
    }
}

/// Change a build the way the Builds window does: its name, kind, edition, the content it takes, whether it splits, and its ceilings.
fn set_build(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(
        args,
        &[
            "--name",
            "--kind",
            "--target",
            "--take",
            "--split",
            "--max-files",
            "--max-bytes",
            "--limits",
            "--ship-unchanged",
            "--author",
            "--version",
            "--description",
            "--clothing",
        ],
    )?;
    let [project, id] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj> and the <id> of the build".into(),
        ));
    };
    let changes = BuildChanges::from_flags(&flags)?;
    if changes.is_empty() {
        return Err(Usage("name what to change: --name, --kind, --target, --take, --split, --max-files, --max-bytes, --limits, --ship-unchanged, --author, --version, --description or --clothing".into()));
    }
    edit_container(Path::new(project), |container| {
        container.edit_build(id, |output| changes.apply(output))
    })
    .map_err(Run)?;
    let loaded = Project::load(Path::new(project)).map_err(|e| Run(format!("{project}: {e}")))?;
    if let Some(build) = loaded.container.output(id) {
        println!(
            "changed the build {id}: a {} named {}, taking {}{}",
            build.kind.label(),
            build.name,
            if build.take.is_empty() {
                "no content".to_string()
            } else {
                build.take.join(", ")
            },
            if build.split {
                ", splitting over a ceiling"
            } else {
                ""
            }
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// State the type the game is told for a file of the Files content, or take the statement away with `none`.
fn set_file_type(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, _) = parse(args, &[])?;
    let [project, path, file_type] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj>, the <path> of a file inside the project and its <TYPE> or none".into(),
        ));
    };
    let stated = (file_type != "none").then_some(file_type.as_str());
    edit_container(Path::new(project), |container| {
        let files = (container.modules.iter_mut())
            .find(|m| m.kind == ModuleKind::Files && !m.is_link())
            .ok_or_else(|| format!("{project} holds no Files content of its own"))?;
        files.set_file_type(path, stated)
    })
    .map_err(Run)?;
    match stated {
        Some(file_type) => println!("{path} is registered as {file_type}"),
        None => println!("{path} states no type"),
    }
    Ok(ExitCode::SUCCESS)
}

/// Whether `resource` is what `build` writes: its name, or, when it splits, its name and the number of a part.
///
/// ⚠ A name another declared build has is that build's, because a build may be named `<project>_2` by hand.
fn is_resource_of(resource: &str, build: &Output, outputs: &[Output]) -> bool {
    resource == build.name
        || (build.split
            && resource
                .strip_prefix(&build.name)
                .and_then(|rest| rest.strip_prefix('_'))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            && !outputs.iter().any(|other| other.name == resource))
}

/// Remove what builds left under `build/`, found by their hidden marker, or only what one build left.
fn clean_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &[BUILD, "--out"])?;
    let [path] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let project = Project::load(Path::new(path)).map_err(|e| Run(format!("{path}: {e}")))?;
    let root = root_of(path, &flags);
    let mut resources = built_resources(&root).map_err(|e| Run(e.to_string()))?;
    if let Some(id) = flags.get(BUILD) {
        let build = (project.container.output(id))
            .ok_or_else(|| Run(format!("{path} declares no build `{id}`")))?;
        resources.retain(|resource| is_resource_of(resource, build, &project.container.outputs));
    }
    let (mut removed, mut sound) = (0, true);
    for resource in &resources {
        match clean(&root, resource) {
            Ok(cleaned) => {
                for place in &cleaned.removed {
                    println!("removed {}", place.display());
                }
                removed += cleaned.removed.len();
            }
            Err(e) => {
                sound = false;
                println!("problem: {e}");
            }
        }
    }
    println!(
        "{removed} folder(s) and marker(s) removed from {} resource(s)",
        resources.len()
    );
    Ok(if sound {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The redirect map of the moves `plans` make against the items their last builds recorded, and the resources that recorded none.
pub(crate) fn redirect_map(plans: &[Plan], root: &Path) -> (String, Vec<String>) {
    let (mut moved, mut unrecorded) = (Vec::new(), Vec::new());
    for plan in plans {
        match since_last_build(plan, root) {
            Some(found) => moved.extend(found.moved),
            None => unrecorded.push(plan.resource.clone()),
        }
    }
    moved.sort_by(|a, b| a.id.cmp(&b.id));
    let map = Diff {
        moved,
        ..Diff::default()
    };
    (map.redirect_map(), unrecorded)
}

/// Write the redirect map of what will move at the next build, or print it.
fn redirects_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &[BUILD, "--out", "--file"])?;
    let (_, plans) = planned(&positional, &flags)?;
    let (map, unrecorded) = redirect_map(&plans, &root_of(&positional[0], &flags));
    for resource in unrecorded {
        eprintln!(
            "{resource}: no earlier build recorded its items, so no item is known to have moved"
        );
    }
    match flags.get("--file") {
        Some(file) => {
            or_project::replace_file(Path::new(file), map.as_bytes())
                .map_err(|e| Run(format!("{file}: {e}")))?;
            println!("wrote the redirect map to {file}");
        }
        None => print!("{map}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn template_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--name", "--description"])?;
    let [project, file] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj> and the <file> to write".into(),
        ));
    };
    let loaded = Project::load(Path::new(project)).map_err(|e| Run(format!("{project}: {e}")))?;
    let name = flags.get("--name").cloned().unwrap_or_else(|| {
        Path::new(project)
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned())
    });
    // The layout is the editor's keys of the file, which this crate does not read as a project.
    let top: serde_json::Map<String, Value> = std::fs::read(project)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let word = |key: &str| top.get(key).and_then(Value::as_str).map(str::to_string);
    let shown: Option<Vec<String>> =
        top.get("shown_workspaces")
            .and_then(Value::as_array)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|n| n.as_str().map(str::to_string))
                    .collect()
            });
    let (template, left_out) = Template::from_project(
        &name,
        flags.get("--description").map_or("", String::as_str),
        &loaded.container,
        word("workspace").as_deref(),
        shown.as_deref(),
        word("game").as_deref(),
    );
    if Path::new(file).exists() {
        return Err(Run(format!(
            "{file} is there already, and a template is never written over one"
        )));
    }
    or_project::replace_file(Path::new(file), template.to_json().as_bytes())
        .map_err(|e| Run(format!("{file}: {e}")))?;
    println!(
        "wrote {file}: the template `{}` ({})",
        template.name,
        template::EXTENSION
    );
    for left in &left_out {
        println!("  left out: {left}");
    }
    Ok(ExitCode::SUCCESS)
}

/// The project at `path`, and the plans of the output the flags name: one, or one for each resource it splits into.
pub(crate) fn planned(
    positional: &[String],
    flags: &HashMap<String, String>,
) -> Result<(Project, Vec<Plan>), Failure> {
    let [path] = positional else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let project = Project::load(Path::new(path)).map_err(|e| Run(format!("{path}: {e}")))?;
    for finding in &project.findings {
        eprintln!("warning: {finding}");
    }
    let build = match (flags.get(BUILD), project.container.outputs.as_slice()) {
        (Some(id), _) => id.clone(),
        (None, [only]) => only.id.clone(),
        (None, []) => return Err(Run(format!("{path} declares no build"))),
        (None, many) => {
            let ids: Vec<&str> = many.iter().map(|o| o.id.as_str()).collect();
            return Err(Usage(format!(
                "{path} declares several builds ({}): name one with --build",
                ids.join(", ")
            )));
        }
    };
    // The Files modules and every other kind of content hand their contributions to one plan.
    let folders = cookable(&project)?;
    let plans = or_content::plans(&project, &build, &folders).map_err(|e| Run(e.to_string()))?;
    Ok((project, plans))
}

/// The `.src` folders of the project and of the projects whose Files content it links, found as the editor finds them, each with what it cooks to.
fn cookable(project: &Project) -> Result<Vec<SourceFolder>, Failure> {
    let members: Vec<PathBuf> = project.sources().into_iter().map(|s| s.path).collect();
    let (mut found, mut cut) = find_sources(&project.folder(), &members);
    let (linked, linked_cut) = linked_sources(&project.container, &project.folder());
    found.extend(linked);
    cut |= linked_cut;
    if cut {
        return Err(Run(format!(
            "{} holds more folders than a search for sources reads, so a build cannot tell which sources it has: build from a smaller project folder",
            project.folder().display()
        )));
    }
    Ok(found
        .into_iter()
        .map(|folder| SourceFolder {
            cooks: describe(&folder).map(|said| Cooks {
                kind: said.kind,
                name: said.name,
                extension: said.extension,
                bytes: said.bytes,
                game: said.game,
            }),
            folder,
        })
        .collect())
}

fn plan_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &[BUILD, "--out"])?;
    let (_, plans) = planned(&positional, &flags)?;
    let root = root_of(&positional[0], &flags);
    if let [first, _, ..] = plans.as_slice() {
        println!(
            "build {} is over a ceiling and divides into {} resources",
            first.output,
            plans.len()
        );
    }
    let mut buildable = true;
    for plan in &plans {
        println!(
            "build {}: a {} named {}{}",
            plan.output,
            plan.kind.label(),
            plan.resource,
            target_words(plan)
        );
        for file in &plan.files {
            let (what, shown) = match &file.origin {
                Origin::Copy { from, .. } => ("copy", from.display().to_string()),
                Origin::Retarget { from, .. } => ("retarget", from.display().to_string()),
                Origin::Cook { source, game, .. } if !game.is_empty() && !plan.ship_unchanged => (
                    "cook",
                    format!(
                        "{} (left out if it is still the game's file)",
                        source.display()
                    ),
                ),
                Origin::Cook { source, .. } => ("cook", source.display().to_string()),
                Origin::Document(_) | Origin::Generated(_) => {
                    ("document", "written by the build".to_string())
                }
            };
            println!(
                "  {what:<8} {:<40} {:>10} bytes  {shown}",
                file.path,
                file.bytes()
            );
        }
        println!("{} file(s), {} bytes", plan.totals.files, plan.totals.bytes);
        if plan.target.is_some() {
            println!("a retarget file counts as its source's size, since the moved file's is not known before it is made");
        }
        if let Some(entry) = &plan.dlclist_entry {
            println!("dlclist.xml needs <Item>{entry}</Item> inside <Paths>");
        }
        for budget in &plan.budgets {
            match budget.ceiling {
                Some(max) => println!("budget {}: {} of {max}", budget.name, budget.used),
                None => println!("budget {}: {}", budget.name, budget.used),
            }
        }
        if let Some(found) = since_last_build(plan, &root) {
            since(&found);
        }
        for problem in &plan.problems {
            println!("problem: {problem}");
        }
        for note in &plan.notes {
            println!("note: {note}");
        }
        buildable &= plan.is_buildable();
    }
    Ok(if buildable {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What a build changed from the one it replaces, by stable id.
pub(crate) fn since(found: &Diff) {
    println!(
        "since the last build: {} added, {} changed, {} moved, {} removed",
        found.added.len(),
        found.changed.len(),
        found.moved.len(),
        found.removed.len()
    );
    for moved in &found.moved {
        println!("  moved {}: {} -> {}", moved.id, moved.from, moved.to);
    }
}

/// Where the builds of the project go: `--out`, else `build/` beside the project file.
pub(crate) fn root_of(path: &str, flags: &HashMap<String, String>) -> PathBuf {
    flags
        .get("--out")
        .map_or_else(|| build_root(Path::new(path)), PathBuf::from)
}

/// The flags `project build` takes a value for.
const BUILD_FLAGS: [&str; 3] = [BUILD, "--out", "--shader-defs"];

/// Whether `args` are ones `project build` takes, read as it reads them and without building.
pub(crate) fn accepts_build(args: &[String]) -> Result<(), String> {
    let args: Vec<String> = (args.iter())
        .filter(|a| *a != "--reproducible")
        .cloned()
        .collect();
    match parse(&args, &BUILD_FLAGS) {
        Ok((positional, _)) if positional.len() == 1 => Ok(()),
        Ok(_) => Err("expected one <project.openrage-proj>".into()),
        Err(Usage(why) | Run(why)) => Err(why),
    }
}

fn build_of(args: &[String]) -> Result<ExitCode, Failure> {
    // `--reproducible` takes no value, so it is taken out before the flags that do are read.
    let reproducible = args.iter().any(|a| a == "--reproducible");
    let args: Vec<String> = (args.iter())
        .filter(|a| *a != "--reproducible")
        .cloned()
        .collect();
    let (positional, flags) = parse(&args, &BUILD_FLAGS)?;
    let (project, plans) = planned(&positional, &flags)?;
    let root = root_of(&positional[0], &flags);
    // ⚠ The database is asked for only by an output that moves files to a target, since without
    // one every parameter of a drawable is dropped and the command says so.
    let remap = match plans.iter().any(|plan| plan.target.is_some()) {
        true => {
            crate::retarget::remap_of(flags.get("--shader-defs").map(Path::new)).map_err(Run)?
        }
        false => Default::default(),
    };
    // Every resource of a split output is checked before the first is written; one that fails
    // while it is being written leaves the resources before it built.
    let watch = crate::jobs::BuildWatch::start();
    let (mut built, stopped) = build_all_watched(
        &plans,
        &root,
        &remap,
        Some(&cook_source),
        &|_| {},
        &watch.watch,
    );
    watch.finish();
    let mut identical = true;
    if reproducible {
        for (plan, built) in plans.iter().zip(&mut built) {
            check_reproducible(plan, built, &root, &remap, Some(&cook_source));
            identical &= !matches!(built.verification.reproducible, Reproducible::No { .. });
        }
    }
    for (plan, built) in plans.iter().zip(&built) {
        println!("wrote {}", built.folder.display());
        println!(
            "  {} file(s), {} bytes",
            built.written.files, built.written.bytes
        );
        cooked(built);
        report(&built.verification);
        losses(built);
        if let Some(found) = &built.since {
            since(found);
        }
        if let Some(previous) = &built.previous {
            println!("  the build it replaced is kept at {}", previous.display());
        }
        if let Some(entry) = &plan.dlclist_entry {
            println!("  dlclist.xml needs <Item>{entry}</Item> inside <Paths>");
        }
    }
    let Some(error) = stopped else {
        crate::wardrobe_places::frozen_after(&project, &plans);
        let package = plans.iter().any(|plan| plan.kind == OutputKind::OivPackage);
        println!("This checks the files. It does not test that GTA V loads them.");
        if package {
            println!("It does not test that OpenIV installs a package either.");
        }
        return Ok(if identical {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    };
    if let (Some(plan), false) = (plans.get(built.len()), built.is_empty()) {
        println!(
            "stopped at {}: the {} resource(s) before it stay built",
            plan.resource,
            built.len()
        );
    }
    match error {
        BuildError::Stopped => {
            println!("stopped: the folder it was writing is removed and the last good build is as it was");
            Ok(ExitCode::FAILURE)
        }
        BuildError::Unverified(found) => {
            println!("the build was not swapped in: what it wrote does not read back");
            problems(&found);
            Ok(ExitCode::FAILURE)
        }
        BuildError::Refused(why) | BuildError::Cooking(why) => {
            for problem in why {
                println!("problem: {problem}");
            }
            Ok(ExitCode::FAILURE)
        }
        e => Err(Run(e.to_string())),
    }
}

fn verify_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &[BUILD, "--out"])?;
    let (_, plans) = planned(&positional, &flags)?;
    let mut clean = true;
    for plan in &plans {
        let folder = root_of(&positional[0], &flags).join(&plan.resource);
        let found = verify(plan, &folder);
        println!("{}", folder.display());
        report(&found);
        problems(&found);
        clean &= found.problems.is_empty();
    }
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub(crate) fn report(found: &Verification) {
    println!("  {}", found.reload_line());
    for reloaded in &found.reloaded {
        println!("    {:<40} {}", reloaded.path, reloaded.reader);
    }
    for count in found.metadata() {
        println!(
            "  metadata: {} document(s) read as {}, holding {} entr(ies)",
            count.documents, count.reader, count.entries
        );
    }
    println!("  names: {}", found.names.line());
    let memory = &found.memory;
    if memory.files > 0 || memory.ceiling.is_some() {
        let ceiling = memory
            .ceiling
            .map_or_else(String::new, |max| format!(", of {max} allowed"));
        println!(
            "  memory: an estimated {} bytes of streaming memory in {} RSC7 file(s){ceiling}",
            memory.bytes, memory.files
        );
    }
    if found.reproducible != Reproducible::NotRun {
        println!("  reproducible: {}", found.reproducible.label());
        if let Reproducible::No { differences } = &found.reproducible {
            for difference in differences {
                println!("    {difference}");
            }
        }
    }
}

pub(crate) fn problems(found: &Verification) {
    for problem in &found.problems {
        println!("problem: {problem}");
    }
}

/// The sources a build cooked, and the ones it left out because they are still the game's file.
fn cooked(built: &Built) {
    if !built.cooked.is_empty() {
        println!("  {} source(s) cooked", built.cooked.len());
    }
    for left in &built.unchanged {
        println!(
            "  {} is unchanged from the game, so it is not in the build",
            left.path
        );
    }
}

/// The values a build dropped on the way to its target, one file at a time.
fn losses(built: &Built) {
    for loss in &built.losses {
        println!("  {} dropped {} value(s):", loss.path, loss.dropped.len());
        for dropped in &loss.dropped {
            println!("    {dropped}");
        }
    }
}

/// `, for Enhanced` where the plan moves its files to a target, else nothing.
fn target_words(plan: &Plan) -> String {
    match plan.target {
        Some(target) => format!(", for {}", target.generation().edition_label()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_project::{build, plan_with, Contribution, Item, PlannedFile};

    fn plan_with_item(folder: &Path, name: &str) -> Plan {
        let mut held = Contribution::of("uniforms");
        held.files.push(PlannedFile {
            path: "stream/a.txt".into(),
            origin: Origin::Document("a".into()),
        });
        held.items.push(Item {
            id: "a1".into(),
            name: name.into(),
            digest: "same".into(),
        });
        let container = Container {
            modules: vec![Module {
                id: "uniforms".into(),
                kind: ModuleKind::Unknown("widgets".into()),
                extra: Default::default(),
            }],
            outputs: vec![Output::fivem_resource("fivem", "pack", &["uniforms"])],
        };
        plan_with(&container, "fivem", folder, &[held]).expect("the build is declared")
    }

    #[test]
    fn the_usage_names_the_flag_of_a_build_and_not_its_older_spelling() {
        assert!(USAGE.contains("[--build <id>]"));
        assert!(!USAGE.contains("--output"), "the older spelling is hidden");
    }

    #[test]
    fn a_moved_item_is_in_the_redirect_map_by_its_id_and_a_build_that_recorded_nothing_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let (project, root) = (dir.path().join("project"), dir.path().join("build"));
        build(&plan_with_item(&project, "old"), &root).expect("the first build");

        let (map, unrecorded) = redirect_map(&[plan_with_item(&project, "new")], &root);

        assert!(unrecorded.is_empty());
        let map: Value = serde_json::from_str(&map).unwrap();
        assert_eq!(map["format"], "openrage-redirects");
        assert_eq!(
            map["redirects"],
            serde_json::json!([{ "id": "a1", "from": "old", "to": "new" }])
        );

        let (map, unrecorded) = redirect_map(
            &[plan_with_item(&project, "new")],
            &dir.path().join("elsewhere"),
        );
        assert_eq!(unrecorded, ["pack"]);
        assert_eq!(
            serde_json::from_str::<Value>(&map).unwrap()["redirects"],
            serde_json::json!([])
        );
    }
}
