//! `openrage-cli maps`: list what a project's map sets place and declare, validate them and say what the rules find, read a build back
//! against them, and apply a list of operations to their sources.
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
        Some("list") => list(rest),
        Some("validate") => validate(rest),
        Some("verify") => verify(rest),
        Some("apply") => apply(rest),
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
         openrage-cli maps list     <project.openrage-proj> [--module <id>]\n  \
         openrage-cli maps validate <project.openrage-proj> [--module <id>] [--json]\n  \
         openrage-cli maps verify   <project.openrage-proj> [--build <id>] [--out <folder>]\n  \
         openrage-cli maps apply    <project.openrage-proj> <operations.json> [--module <id>] [--dry-run]\n\n\
         changes compares each map file and type file of the content with the base its source keeps,\n\
         the file it was imported from, and says by object what the source changed: the entities and\n\
         the archetypes added, deleted and changed, each with its index in the base and its index now,\n\
         and the ones no guid and no place tells from a twin, which are named ambiguous and never\n\
         moved. What a file states of itself differently, such as its flags and its extents, is a\n\
         line of its own, and the entities of an interior are under the interior. With --json it\n\
         prints the same changes as data. It exits 1 when a source cannot be compared.\n\
         list names the map sets of the content and, for each source, what it places (the entities of\n\
         a map file) or declares (the archetypes of a type file, and the rooms and portals of its\n\
         interiors).\n\
         validate reads every source of the maps content, opens the map files and type files through\n\
         the typed world family, and lists what the rules find, each with a code and a severity. It\n\
         exits 1 when a finding that is not waived is an error.\n\
         verify reads the resource a build wrote against the map sets it was made of: that each source\n\
         reached the place the plan gave it, that every map file and type file reloads with the\n\
         entities and archetypes its source states, and that the manifest registers each type file.\n\
         It exits 1 when a check fails. The plan and the build are `openrage-cli project plan` and\n\
         `build`; maps go through them as any content does.\n\
         apply does the operations of a list, in order, to the sources of the maps content: an entity\n\
         placed, added as a copy or taken out with its LOD links renumbered, an archetype added or taken\n\
         out, the extents grown or stated, a record put back. The list is JSON whose format is\n\
         `openrage-map-operations` and whose schema is 1, each operation an object whose `op` names it\n\
         and whose `source` is a source folder of the content. It writes all of them or none: an\n\
         operation that is refused is named by its place in the list, and no source changes. With\n\
         --dry-run it says what each operation would change and writes nothing. It exits 1 when an\n\
         operation is refused.\n\n\
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
                        map.problems.len()
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
