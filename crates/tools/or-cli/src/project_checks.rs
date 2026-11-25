//! `openrage-cli project apply-template`, `check-sharing` and `check-original`: what the Project workspace's Apply a template…, Check before sharing… and
//! Check against the game do, with no window.
//!
//! ⚠ `check-sharing` and `check-original` write nothing and answer with their exit code, so a pre-push hook or a CI step can run them: 0 where there is
//! nothing to take out or the originals are the game's now, 1 where there is. The private folders `check-sharing` lists are told and never fail it: Git
//! never sees them, and an edited pack always has one.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_content::sharing::{concerns, game_sources, Kind};
use or_project::project::edit_container;
use or_project::{find_sources, project_folder, relative_inside, ModuleKind, Project};
use or_source::asset::Source;
use or_source::origin_check::{check, Installed, Verdict};
use serde_json::json;

use crate::project::{parse, Failure};
use crate::wardrobe_fix::without;
use Failure::{Run, Usage};

/// The names the clothing packs of the project already have, which a template's pack must not take.
fn pack_names(project: &Project, folder: &Path) -> Vec<String> {
    (project.container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Clothing)
        .filter_map(|m| or_wardrobe::module_folder(folder, m).ok())
        .filter_map(|at| or_wardrobe::Wardrobe::read(&at).ok())
        .map(|w| w.pack)
        .collect()
}

/// `project apply-template`: add the content and builds a template file states that the project lacks.
pub fn apply_template(args: &[String]) -> Result<ExitCode, Failure> {
    let (positional, _) = parse(args, &[])?;
    let [project, file] = positional.as_slice() else {
        return Err(Usage(
            "expected a <project.openrage-proj> and the <file.openrage-template> to apply".into(),
        ));
    };
    let path = Path::new(project);
    let loaded = Project::load(path).map_err(|e| Run(format!("{project}: {e}")))?;
    let folder = project_folder(path);
    let name = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let bytes = std::fs::read(file).map_err(|e| Run(format!("{file}: {e}")))?;
    let packs = pack_names(&loaded, &folder);
    let applied = edit_container(path, |container| {
        or_content::template::apply_template(container, &name, &folder, &bytes, &packs)
    })
    .map_err(Run)?;
    println!("{}", applied.sentence());
    Ok(ExitCode::SUCCESS)
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::MachinePath => "machine_path",
        Kind::GameContent => "game_content",
        Kind::PrivateFolder => "private_folder",
        Kind::Unsaved => "unsaved",
        Kind::Bounded => "bounded",
    }
}

/// `project check-sharing`: what the project's folder holds that is not yours to hand to another machine or a repository.
pub fn check_sharing(args: &[String]) -> Result<ExitCode, Failure> {
    let (rest, bare) = without(args, &["--json"]);
    let (positional, _) = parse(&rest, &[])?;
    let [project] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let path = Path::new(project);
    let loaded = Project::load(path).map_err(|e| Run(format!("{project}: {e}")))?;
    let folder = project_folder(path);
    let (games, bounded) = game_sources(path);
    // A pack inside the folder keeps its earlier versions in a private folder of its own; a linked one is another project's.
    let packs: Vec<PathBuf> = (loaded.container.modules.iter())
        .filter(|m| m.kind == ModuleKind::Clothing)
        .filter_map(|m| or_wardrobe::module_folder(&folder, m).ok())
        .filter(|at| relative_inside(at, &folder).is_some())
        .collect();
    let found = concerns(path, &games, &packs, bounded);
    if bare[0] {
        let list: Vec<_> = (found.iter())
            .map(|c| json!({ "kind": kind_word(c.kind), "text": c.text }))
            .collect();
        crate::wardrobe_fix::print_json(vec![json!({ "project": project, "concerns": list })]);
    } else if found.is_empty() {
        println!("nothing to take out: {project} holds no path of this machine, no source taken from the game and no manifest of the game");
    } else {
        for concern in &found {
            println!("{}: {}", kind_word(concern.kind), concern.text);
        }
    }
    let blocks = found
        .iter()
        .any(|c| matches!(c.kind, Kind::MachinePath | Kind::GameContent));
    Ok(if blocks {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `project check-original`: whether the game's file each source taken from it was made from is still the file the install holds.
pub fn check_original(args: &[String]) -> Result<ExitCode, Failure> {
    let (rest, bare) = without(args, &["--json"]);
    let (positional, flags) = parse(&rest, &["--game", "--keys", "--source"])?;
    let [project] = positional.as_slice() else {
        return Err(Usage("expected one <project.openrage-proj>".into()));
    };
    let root = flags.get("--game").map(PathBuf::from).ok_or_else(|| {
        Usage(
            "--game <install root> says which install holds the game's files to compare with"
                .into(),
        )
    })?;
    let path = Path::new(project);
    Project::load(path).map_err(|e| Run(format!("{project}: {e}")))?;
    let sources: Vec<PathBuf> = match flags.get("--source") {
        Some(one) => vec![PathBuf::from(one)],
        None => find_sources(&project_folder(path), &[]).0,
    };
    let keys = flags.get("--keys").map(PathBuf::from);
    let resolved = crate::keys::resolve_crypto(&root, keys.as_deref());
    for note in &resolved.notes {
        eprintln!("note: {note}");
    }
    let mut answers = Vec::new();
    let mut failed = false;
    for dir in &sources {
        let manifest = Source::open(dir).map_err(|p| Run(format!("{}: {p}", dir.display())))?;
        if !manifest.manifest.origin.from_game {
            continue;
        }
        let verdict =
            check(
                &manifest.manifest,
                |virtual_path| match or_title::install::read_virtual_file(
                    &root,
                    virtual_path,
                    &resolved.provider,
                ) {
                    Ok(Some(bytes)) => Installed::Found(bytes),
                    Ok(None) => Installed::Missing,
                    Err(why) => Installed::Unreadable(why),
                },
            );
        failed |= matches!(verdict, Verdict::Changed { .. } | Verdict::Missing { .. });
        let leaf = dir
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        answers.push((leaf, verdict.sentence()));
    }
    if bare[0] {
        let list: Vec<_> = (answers.iter())
            .map(|(source, said)| json!({ "source": source, "verdict": said }))
            .collect();
        crate::wardrobe_fix::print_json(vec![json!({ "project": project, "sources": list })]);
    } else if answers.is_empty() {
        println!(
            "no source of {project} was taken from the game, so there is no original to compare"
        );
    } else {
        for (source, said) in &answers {
            println!("{source}: {said}");
        }
    }
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}
