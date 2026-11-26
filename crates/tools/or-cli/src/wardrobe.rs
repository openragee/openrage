//! `openrage-cli wardrobe`: validate a clothing pack, print the plan of its build as data, build it, say what moved since the last build, and make a pack of the files of an existing resource.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_project::{build_all_watched, since_last_build, ModuleKind, Project};
use or_wardrobe::import::{self, Options};
use or_wardrobe::{Depth, Finding, Severity};
use serde_json::{json, Value};

use crate::project::{parse, planned as planned_plans, root_of, Failure};
use Failure::{Run, Usage};

pub fn run(args: &[String]) -> ExitCode {
    let rest = args.get(1..).unwrap_or_default();
    let result = match args.first().map(String::as_str) {
        Some("validate") => validate(rest),
        Some("plan") => plan_of(rest),
        Some("build") => build_of(rest),
        Some("diff") => diff_of(rest),
        Some("import") => import_of(rest),
        Some("rules") => rules_of(rest),
        Some("dump") => crate::wardrobe_dump::dump_of(rest),
        Some("fix") => crate::wardrobe_fix::fix(rest),
        Some("freeze") => crate::wardrobe_places::freeze_of(rest),
        Some("dummies") => crate::wardrobe_places::dummies_of(rest),
        Some("textures") => crate::wardrobe_fix::textures(rest),
        Some("maps") => crate::wardrobe_maps::maps(rest),
        Some("budget") => crate::wardrobe_maps::budget(rest),
        Some("slot") => crate::wardrobe_maps::slot(rest),
        Some("mesh") => crate::wardrobe_mesh::mesh(rest),
        Some("reopen") => crate::wardrobe_exchange::reopen_of(rest),
        Some("package") => crate::wardrobe_exchange::package_of(rest),
        Some("add") => crate::wardrobe_exchange::add_of(rest),
        Some("query") => crate::wardrobe_power::query_of(rest),
        Some("bulk") => crate::wardrobe_power::bulk_of(rest),
        Some("review") => crate::wardrobe_power::review_of(rest),
        Some("credit") => crate::wardrobe_power::credit_of(rest),
        Some("sizes") => crate::wardrobe_power::sizes_of(rest),
        Some("pack") => crate::wardrobe_power::pack_of_a_project(rest),
        Some("unpack") => crate::wardrobe_power::unpack_of(rest),
        Some("verify") => crate::wardrobe_exchange::verify_of(rest),
        Some("-h" | "--help" | "help") => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        Some(other) => Err(Usage(format!("unknown wardrobe command: {other}"))),
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
         openrage-cli wardrobe validate <project.openrage-proj> [--module <id>] [--json]\n  \
         openrage-cli wardrobe plan     <project.openrage-proj> [--build <id>] [--out <file>]\n  \
         openrage-cli wardrobe build    <project.openrage-proj> [--build <id>] [--out <folder>] [--shader-defs <file>]\n  \
         openrage-cli wardrobe diff     <project.openrage-proj> [--build <id>] [--out <folder>] [--redirects <file>]\n  \
         openrage-cli wardrobe import   <folder> [--pack <name>]\n  \
         openrage-cli wardrobe rules    [--json] [--out <file>]\n  \
         openrage-cli wardrobe dump     <project.openrage-proj> [--module <id>] [--lua] [--out <folder>]\n  \
         openrage-cli wardrobe fix      <project.openrage-proj> [--module <id>] [--json]\n  \
         openrage-cli wardrobe freeze   <project.openrage-proj> [--module <id>] [--thaw]\n  \
         openrage-cli wardrobe dummies  (fill | sort) <project.openrage-proj> --ped <male|female> --slot <name> [--module <id>]\n  \
         openrage-cli wardrobe textures <project.openrage-proj> [--module <id>] [--garment <id>]\n                                 \
         [--ratio <0 to 1> | --side <pixels>] [--no-format] [--no-mips] [--no-power-of-two] [--json]\n  \
         openrage-cli wardrobe maps     <project.openrage-proj> [--module <id>] [--garment <id>] [--texture <name>,...]\n                                 \
         [--no-normal] [--no-specular] [--strength <n>] [--dry-run] [--json]\n  \
         openrage-cli wardrobe budget   <project.openrage-proj> --target <size> [--module <id>] [--garment <id>]\n                                 \
         [--smallest <pixels>] [--apply] [--json]\n  \
         openrage-cli wardrobe slot     <project.openrage-proj> (--list | --garment <id> --folder <dir> [--texture <n>]\n                                 \
         [--slot diffuse|specular|normal] [--unbind] [--now]) [--module <id>] [--json]\n  \
         openrage-cli wardrobe mesh     <project.openrage-proj> [--garment <id>] [--module <id>] [--skeleton <ped.yft>]\n                                 \
         (--check | --levels <medium,low> [--replace] | --rig | --weights | --mirror | --crop-uvs | --materials [--shader-defs <file>]) [--json]\n  \
         openrage-cli wardrobe reopen   <dlc.rpf | file.oiv | file.gctproject | folder> --out <folder> [--pack <name>]\n  \
         openrage-cli wardrobe package  <project.openrage-proj> --garment <id> --out <folder> [--module <id>]\n  \
         openrage-cli wardrobe add      <project.openrage-proj> (<file or folder>... | --package <folder>) [--module <id>]\n  \
         openrage-cli wardrobe verify   <project.openrage-proj> [--build <id>] [--out <root>] [--json]\n  \
         openrage-cli wardrobe query    <project.openrage-proj> \"<query>\" [--module <id>] [--json]\n  \
         openrage-cli wardrobe bulk     <project.openrage-proj> \"<query>\" (--add-tag <w> | --remove-tag <w> | --group <g> | --no-group | --remove)\n                                 \
         [--apply] [--module <id>] [--json]\n  \
         openrage-cli wardrobe review   <project.openrage-proj> (--list | --from <version> [--to <version>]) [--module <id>] [--json]\n  \
         openrage-cli wardrobe credit   <project.openrage-proj> --garment <id> [--author <t>] [--url <t>] [--licence <t>] [--module <id>]\n  \
         openrage-cli wardrobe sizes    <project.openrage-proj> [--module <id>] [--json]\n  \
         openrage-cli wardrobe pack     <project.openrage-proj> --out <file.openrage-pack> [--module <id>]\n  \
         openrage-cli wardrobe unpack   <file.openrage-pack> --out <folder>\n\n\
         validate reads every file of the clothing content, opens it through its reader, and lists\n\
         what the rules find, each with a code and a severity. It exits 1 when a finding that is\n\
         not waived is an error.\n\
         plan prints the plan of the build as JSON: the files, the budgets, the items with their\n\
         stable ids and the problems, for a script or a CI job to read (docs/reference/Plan_Document.md).\n\
         It writes nothing but the file --out names.\n\
         build writes the resource as `project build` does, then reads the clothing metadata back\n\
         against the pack and says what held.\n\
         diff says what was added, changed, moved and removed since the last build, by stable id,\n\
         and with --redirects writes the old name and the new name of each moved garment as JSON.\n\
         import makes a pack of the models, textures, cloth and metadata under <folder> and writes\n\
         wardrobe.json and a project file there. It moves and copies nothing and replaces nothing.\n\
         rules lists every rule the Wardrobe holds a pack to, with how serious it is, whether a\n\
         reason can set a finding of it aside and where its limit stands; --json writes it as a\n\
         document that a script reads, and --out writes it to a file. A project changes a rule in\n\
         the `rules` of its wardrobe.json.\n\
         dump lists every texture of a clothing content as a server's scripts read it, as JSON or\n\
         with --lua as a Lua table, and --out writes both files into a folder; a build carries the\n\
         same files in a FiveM resource when its clothes dump option asks for them.\n\
         fix runs Fix all safe over each clothing content: it repairs a model whose counts contradict\n\
         their runs where that loses nothing, gives a texture its mips, its power of two and a format\n\
         the game's clothing uses, and lets go the pins that clash. It says what each step changed,\n\
         what it refused by name and what it left, and run again it changes no byte.\n\
         freeze says how many drawables a clothing content has frozen and how many a rebuild would move. A build\n\
         that goes through freezes the drawables it ships, a garment that would move stops the next build until it is\n\
         pinned back or a reason is waived, and --thaw lets the pack renumber freely.\n\
         dummies fill makes a dummy of each hole below the last garment of a slot's collections; dummies sort puts the\n\
         dummies of a slot after its garments and pins each where it went.\n\
         textures optimises the textures of a content with those options and, by --ratio or --side,\n\
         their size, and the textures embedded in the models too. A source is never changed in place: the garments name a copy\n\
         that is lighter.\n\
         maps makes a normal map and a specular map of each diffuse texture (a name with the word diff, or the ones --texture\n\
         names) as new pictures of a copy of its source, which the garments name; they are an estimate from the brightness of\n\
         the diffuse, and a map the source has is left. --dry-run lists what would be made and writes nothing.\n\
         budget chooses a size and a format for each texture that fit --target (64MB, 512KB or bytes) with the least picture\n\
         lost, prints each choice with its PSNR, and writes nothing until --apply; it exits 1 when the target is below what the\n\
         textures can be brought to.\n\
         slot binds a folder an external editor saves into to the diffuse, specular or normal slot of a garment's texture, which must be a\n\
         textures source of the pack: the editor puts what is saved there into the source and the preview follows. --now puts the newest\n\
         picture of the folder into the slot at once; --list lists the bindings and --unbind lets one go.\n\
         mesh --materials states the constants a shader declares that a material of the garment's models lacks; with no\n\
         --garment it does so for every garment of the content, as one step. It needs the shader database (OPENRAGE_SHADER_DEFS).\n\n\
         reopen makes a project of a built pack: the garments of a singleplayer DLC, an .oiv or a grzyClothTool\n\
         project with everything their metadata states, each model and texture as a source, and every file it\n\
         could not place kept under files/ and named.\n\
         package writes a garment as one folder of sources and data, and add --package puts it in another pack\n\
         as an equal garment. add with files or folders does what dropping them on the Wardrobe does.\n\
         verify reads an existing build back, each resource of a split output by itself, and exits 1 when\n\
         one does not reload or its metadata does not agree with the pack.\n\
         query lists the garments a query selects (tag:wool slot:jbib polys>5000 lod:missing is:duplicate duplicate-of:<name>\n\
         problem:<code> is:problem, a - in front of a term turns it round, a word is a name searched for).\n\
         bulk makes one change to the garments a query selects and says first what it would change in how many; it is a\n\
         dry run until --apply, and one Undo of the app takes all of it back. review reads two versions of the pack (--list\n\
         numbers them, a --to left out is the pack as it is) and says the garments, textures and settings added, changed and\n\
         removed. credit states who made the sources of a garment, in the asset.json of each. sizes ranks the files the\n\
         pack's build writes by garment and by slot. pack writes the whole pack as one package of sources and settings, and\n\
         unpack makes a pack of one in a folder of its own.\n\n\
         A build checks the files. It does not test that GTA V loads them.\n"
    );
}

/// The project at `path` and the ids of its clothing modules, or the one `--module` names.
pub(crate) fn clothing_of(
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
    let all: Vec<String> = project
        .container
        .modules
        .iter()
        .filter(|m| m.kind == ModuleKind::Clothing)
        .map(|m| m.id.clone())
        .collect();
    let chosen = match flags.get("--module") {
        Some(id) if all.contains(id) => vec![id.clone()],
        Some(id) => return Err(Run(format!("{path} has no clothing content `{id}`"))),
        None => all,
    };
    if chosen.is_empty() {
        return Err(Run(format!("{path} holds no clothing content")));
    }
    Ok((project, chosen))
}

/// `wardrobe rules`: the rule list, as lines a person reads or as the document a script reads, on the screen or in a file.
fn rules_of(args: &[String]) -> Result<ExitCode, Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let (positional, flags) = parse(&rest, &["--out"])?;
    if !positional.is_empty() {
        return Err(Usage(
            "rules takes no project, only --json and --out".into(),
        ));
    }
    let text = match json_out {
        true => {
            let mut text = serde_json::to_string_pretty(&or_wardrobe::rules::export())
                .map_err(|e| Run(e.to_string()))?;
            text.push('\n');
            text
        }
        false => or_wardrobe::rules::export_text(),
    };
    match flags.get("--out") {
        Some(out) => {
            std::fs::write(out, text).map_err(|e| Run(format!("{out}: {e}")))?;
            println!("wrote the rules to {out}");
        }
        None => print!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn validate(args: &[String]) -> Result<ExitCode, Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let (positional, flags) = parse(&rest, &["--module"])?;
    let (project, modules) = clothing_of(&positional, &flags)?;
    let mut blocked = false;
    let mut reports = Vec::new();
    for id in modules {
        let module = project
            .container
            .module(&id)
            .expect("chosen from the container");
        let planned = or_wardrobe::planned(&project.folder(), module, Depth::Open)
            .map_err(|e| Run(format!("content `{id}`: {e}")))?;
        blocked |= planned.findings.iter().any(Finding::blocks);
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
            "content {id}: {} garment(s) placed, {} error(s), {} warning(s), {} note(s), {} waived",
            planned.allocation.placed.len(),
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
                    println!("  waived  {:<20} {} (waived: {reason})", f.code, f.message)
                }
                None => println!("  {word:<7} {:<20} {}", f.code, f.message),
            }
        }
    }
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Array(reports)).expect("JSON serialises")
        );
    }
    Ok(if blocked {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn plan_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--build", "--out"])?;
    let (_, plans) = planned_plans(&positional, &flags)?;
    let mut buildable = true;
    let documents: Vec<Value> = plans
        .iter()
        .map(|plan| {
            buildable &= plan.is_buildable();
            plan.document()
        })
        .collect();
    let text = serde_json::to_string_pretty(&Value::Array(documents)).expect("JSON serialises");
    match flags.get("--out") {
        Some(out) => {
            std::fs::write(out, format!("{text}\n")).map_err(|e| Run(format!("{out}: {e}")))?;
            println!("wrote the plan to {out}");
        }
        None => println!("{text}"),
    }
    Ok(if buildable {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn build_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--build", "--out", "--shader-defs"])?;
    let (project, plans) = planned_plans(&positional, &flags)?;
    let root = root_of(&positional[0], &flags);
    // The database is asked for only by an output that moves files to a target, as `project build` does.
    let remap = match plans.iter().any(|plan| plan.target.is_some()) {
        true => {
            crate::retarget::remap_of(flags.get("--shader-defs").map(Path::new)).map_err(Run)?
        }
        false => Default::default(),
    };
    let watch = crate::jobs::BuildWatch::start();
    let (built, stopped) = build_all_watched(
        &plans,
        &root,
        &remap,
        Some(&or_source::cook::cook_source),
        &|_| {},
        &watch.watch,
    );
    watch.finish();
    for (plan, one) in plans.iter().zip(&built) {
        println!("wrote {}", one.folder.display());
        println!(
            "  {} file(s), {} bytes",
            plan.totals.files, plan.totals.bytes
        );
        crate::project::report(&one.verification);
        crate::project_acceptance::report_seen(&one.folder, plan.target);
        if let Some(found) = &one.since {
            crate::project::since(found);
        }
    }
    let mut clean = stopped.is_none();
    if let Some(error) = stopped {
        match error {
            or_project::BuildError::Refused(why) => {
                why.iter().for_each(|p| println!("problem: {p}"))
            }
            or_project::BuildError::Unverified(found) => {
                println!("the build was not swapped in: what it wrote does not read back");
                crate::project::problems(&found);
            }
            or_project::BuildError::Stopped => {
                println!("stopped: the folder it was writing is removed and the last good build is as it was");
            }
            other => return Err(Run(other.to_string())),
        }
    } else {
        // The metadata is read back against the pack, resource by resource and content by content.
        for (plan, one) in plans.iter().zip(&built) {
            for (module, found) in
                crate::wardrobe_exchange::clothing_checks(&project, plan, &one.folder)
                    .map_err(Run)?
            {
                println!("content {module} in {}:", plan.resource);
                for check in &found.checks {
                    let mark = if check.passed { "ok  " } else { "FAIL" };
                    println!("  {mark} {}: {}", check.name, check.detail);
                }
                clean &= found.is_clean();
            }
        }
    }
    if clean {
        crate::wardrobe_places::frozen_after(&project, &plans);
        println!("This checks the files. It does not test that GTA V loads them.");
    }
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn diff_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--build", "--out", "--redirects"])?;
    let (_, plans) = planned_plans(&positional, &flags)?;
    let root = root_of(&positional[0], &flags);
    let mut moved = Vec::new();
    for plan in &plans {
        let Some(found) = since_last_build(plan, &root) else {
            println!(
                "{}: no earlier build recorded its items, so there is nothing to compare",
                plan.resource
            );
            continue;
        };
        println!("{}:", plan.resource);
        crate::project::since(&found);
        for item in &found.added {
            println!("  added {}: {}", item.id, item.name);
        }
        for item in &found.changed {
            println!("  changed {}: {}", item.id, item.name);
        }
        for item in &found.removed {
            println!("  removed {}: {}", item.id, item.name);
        }
        moved.extend(found.moved.clone());
    }
    if let Some(path) = flags.get("--redirects") {
        let map = or_project::Diff {
            moved,
            ..Default::default()
        }
        .redirect_map();
        std::fs::write(path, map).map_err(|e| Run(format!("{path}: {e}")))?;
        println!("wrote the redirect map to {path}");
    }
    Ok(ExitCode::SUCCESS)
}

fn import_of(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, flags) = parse(args, &["--pack"])?;
    let [folder] = positional.as_slice() else {
        return Err(Usage(
            "expected one <folder> holding the files of a pack".into(),
        ));
    };
    let folder = PathBuf::from(folder);
    let options = Options {
        pack: flags.get("--pack").cloned(),
    };
    let imported = import::import(&folder, &options).map_err(|e| Run(e.to_string()))?;
    let w = &imported.wardrobe;
    if !or_wardrobe::names::is_identifier(&w.pack) {
        return Err(Run(format!(
            "the pack is called {:?}, which is not a lower case name: say one with --pack",
            w.pack
        )));
    }
    import::write(&folder, &imported).map_err(|e| Run(e.to_string()))?;
    let has_project = std::fs::read_dir(&folder)
        .map_err(|e| Run(format!("{}: {e}", folder.display())))?
        .flatten()
        .any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == or_project::PROJECT_EXTENSION)
        });
    println!(
        "wrote {} for pack `{}`: {} model(s), {} cloth file(s), {} texture(s), {} place(s) held",
        folder.join(or_wardrobe::FILE).display(),
        w.pack,
        imported.report.models,
        imported.report.cloth,
        imported.report.textures,
        imported.report.reserved
    );
    for left in &imported.report.left_out {
        println!("  left out {}: {}", left.path, left.why);
    }
    for missing in &imported.report.cloth_missing {
        println!("  cloth missing {}: {}", missing.path, missing.why);
    }
    let shop = &imported.report.shop;
    if !shop.is_empty() {
        println!(
            "  shop metadata: {} file(s), {} entr(ies), {} resolved to a garment's texture",
            shop.files.len(),
            shop.entries,
            shop.resolved
        );
        for line in shop.lines() {
            println!("  {line}");
        }
    }
    if has_project {
        println!("The folder already holds a project; add clothing content with path \".\" to it.");
    } else {
        let container = or_project::Container {
            modules: vec![or_project::Module::clothing("wardrobe", ".")],
            outputs: vec![or_project::Output::fivem_resource(
                "fivem",
                &w.pack,
                &["wardrobe"],
            )],
        };
        let file = folder.join(format!("{}.{}", w.pack, or_project::PROJECT_EXTENSION));
        or_project::Project::create(&file, &container, &[])
            .map_err(|e| Run(format!("{}: {e}", file.display())))?;
        println!(
            "wrote {}: one clothing content and one FiveM resource build",
            file.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}
