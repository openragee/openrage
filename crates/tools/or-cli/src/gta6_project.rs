//! `gta6-project build|check|revert`: a GTA VI mod as a project of archive edits.
//!
//!   `gta6-project build <project.json> --install <dir> --out <new dir> [--keys <snapshot>] [--exe <game.exe>]`
//!   `gta6-project check <project.json>...`   list every entry two edits touch
//!   `gta6-project revert <out dir>`          delete a build's output folder
//!
//! A build writes copies of the archives it touches into a new folder with a manifest, and never
//! writes the install. `--exe` names the game executable whose keys the build reads with.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_project::gta6::{build, conflicts, revert, Gta6Project, GTA6_MANIFEST, STRUCTURAL};

const USAGE: &str = "\
usage:
  openrage-cli gta6-project build <project.json> --install <dir> --out <new dir> [--keys <snapshot>] [--exe <game.exe>]
  openrage-cli gta6-project check <project.json>...
  openrage-cli gta6-project revert <out dir>";

fn usage(msg: &str) -> ExitCode {
    eprintln!("Error: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let Some((verb, rest)) = args.split_first() else {
        return usage("give a verb");
    };
    let mut positionals = Vec::new();
    let (mut install, mut out, mut keys, mut exe) = (None, None, None, None);
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let slot = match arg.as_str() {
            "--install" => &mut install,
            "--out" => &mut out,
            "--keys" => &mut keys,
            "--exe" => &mut exe,
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => {
                positionals.push(PathBuf::from(other));
                continue;
            }
        };
        match it.next() {
            Some(value) => *slot = Some(PathBuf::from(value)),
            None => return usage(&format!("{arg} needs a value")),
        }
    }
    let done = match verb.as_str() {
        "build" => match (positionals.as_slice(), install, out) {
            ([project], Some(install), Some(out)) => {
                run_build(project, &install, &out, keys.as_deref(), exe.as_deref())
            }
            _ => return usage("build takes one project, --install and --out"),
        },
        "check" if positionals.is_empty() => return usage("check takes one or more projects"),
        "check" => check(&positionals),
        "revert" => match positionals.as_slice() {
            [out] => revert(out)
                .map(|()| format!("{}: deleted; the install was never written.", out.display()))
                .map_err(|e| e.to_string()),
            _ => return usage("revert takes the output folder of one build"),
        },
        _ => return usage(&format!("unknown verb {verb}")),
    };
    match done {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_build(
    project: &Path,
    install: &Path,
    out: &Path,
    keys: Option<&Path>,
    exe: Option<&Path>,
) -> Result<String, String> {
    let project = Gta6Project::load(project).map_err(|e| e.to_string())?;
    let keys = crate::gta6_keys::load(keys, exe)?;
    let manifest = build(&project, install, out, &keys).map_err(|e| e.to_string())?;
    let mut report = String::new();
    for change in &manifest.changes {
        report.push_str(&format!(
            "{} {} in {}\n",
            change.op,
            change.entry,
            change.archive.display()
        ));
    }
    report.push_str(&format!(
        "{} edit(s) written to {}, listed in {GTA6_MANIFEST}. {STRUCTURAL}",
        manifest.changes.len(),
        out.display()
    ));
    Ok(report)
}

fn check(paths: &[PathBuf]) -> Result<String, String> {
    let projects = paths
        .iter()
        .map(|p| Gta6Project::load(p).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let found = conflicts(&projects);
    if found.is_empty() {
        return Ok("No entry is edited twice.".to_owned());
    }
    let lines: Vec<String> = found.iter().map(ToString::to_string).collect();
    Err(lines.join("\n"))
}
