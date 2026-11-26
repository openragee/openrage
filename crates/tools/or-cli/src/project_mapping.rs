//! `openrage-cli project adopt` for a mapping that comes in as a zip or as a folder: what it holds and what each file would become,
//! as text or as JSON, and its adoption into a new project.
//!
//! ⚠ `--dry-run` writes nothing. An archive is refused whole, by the entry, when it is not safe to unpack (`or_project::place`),
//! and an adoption that cannot finish removes what it made.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_maps::mapping::{self, file_line, Report, Request};
use or_project::container::is_name;
use or_project::place::{Limits, Place, Refusal, LIMITS};
use or_project::template::resource_name_for;
use serde_json::{json, Value};

use crate::project::{parse, Failure};
use crate::wardrobe_fix::{print_json, without};
use Failure::{Run, Usage};

const VALUED: [&str; 5] = ["--into", "--name", "--resource", "--root", "--limits"];
const BARE: [&str; 2] = ["--dry-run", "--json"];

/// Whether `project adopt <args>` asks for a mapping to be adopted into a new folder, and not for a folder to become a project in place.
pub(crate) fn is_asked(args: &[String]) -> bool {
    let flagged = (args.iter()).any(|arg| {
        arg != "--name"
            && arg != "--resource"
            && (VALUED.contains(&arg.as_str()) || BARE.contains(&arg.as_str()))
    });
    flagged || args.first().is_some_and(|first| Path::new(first).is_file())
}

/// The bounds `--limits` states, as `code=count` pairs over the defaults.
fn limits_of(stated: Option<&String>) -> Result<Limits, Failure> {
    let mut limits = Limits::default();
    for pair in stated.iter().flat_map(|l| l.split(',')) {
        let (code, count) = pair
            .split_once('=')
            .ok_or_else(|| Usage(format!("--limits takes code=count pairs, not {pair}")))?;
        let count = (count.trim().parse())
            .map_err(|_| Usage(format!("--limits takes a count, not {count}")))?;
        limits.set(code.trim(), count).map_err(Usage)?;
    }
    Ok(limits)
}

/// The limits as lines of the usage: each code, its default and why it has it.
pub(crate) fn limit_lines() -> String {
    (LIMITS.iter())
        .map(|row| format!("  {} (default {}): {}", row.code, row.default, row.reason))
        .collect::<Vec<_>>()
        .join("\n")
}

fn refusal_document(refusal: &Refusal) -> Value {
    json!({
        "code": refusal.code(),
        "entry": refusal.entry(),
        "message": refusal.to_string(),
    })
}

/// Why an adoption did not happen, or would not, as the document states a refusal.
fn not_adopted(why: &mapping::Error) -> Value {
    match why {
        mapping::Error::Refused(refusal) => refusal_document(refusal),
        other => json!({ "code": "not-adopted", "entry": null, "message": other.to_string() }),
    }
}

/// `project adopt <mapping.zip|folder>`: the report of a dry run, or the adoption into `--into`.
pub(crate) fn adopt_mapping(args: &[String]) -> Result<ExitCode, Failure> {
    let (rest, bare) = without(args, &BARE);
    let (dry_run, as_json) = (bare[0], bare[1]);
    let (positional, flags) = parse(&rest, &VALUED)?;
    let [from] = positional.as_slice() else {
        return Err(Usage(
            "expected one <mapping.zip> or <folder> to adopt from".into(),
        ));
    };
    let from = PathBuf::from(from);
    let limits = limits_of(flags.get("--limits"))?;
    let into = flags.get("--into").map(PathBuf::from);
    if !dry_run && into.is_none() {
        return Err(Usage(
            "a mapping is adopted into a new folder: name it with --into, or ask for --dry-run"
                .into(),
        ));
    }
    let name = match (flags.get("--name"), &into) {
        (Some(name), _) => name.clone(),
        (None, Some(into)) => std::path::absolute(into)
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default(),
        (None, None) => String::new(),
    };
    // A dry run that names the folder is asked about that adoption, so it reads its command line as the adoption does.
    let adopting = into.is_some();
    if adopting && !is_name(&name) {
        return Err(Usage(format!(
            "{name:?} is not a project name: it becomes a file name, so say one with --name"
        )));
    }
    let resource = (flags.get("--resource").cloned()).unwrap_or_else(|| resource_name_for(&name));
    if adopting && !is_name(&resource) {
        return Err(Usage(format!(
            "{resource:?} is not a folder name for the resource"
        )));
    }

    let mut place = match Place::open(&from, &limits) {
        Ok(place) => place,
        Err(refusal) => {
            if as_json {
                print_json(vec![json!({
                    "format": mapping::FORMAT,
                    "schema": mapping::SCHEMA,
                    "from": from.display().to_string(),
                    "refused": refusal_document(&refusal),
                })]);
            }
            return Err(Run(format!(
                "{} is refused, and nothing is written: {refusal}",
                from.display()
            )));
        }
    };
    let report = mapping::analyse(&mut place);
    let root = flags.get("--root").map(String::as_str);
    if dry_run {
        if !report.adoptable() {
            say(&report, as_json, None);
            return Ok(ExitCode::from(1));
        }
        // What would stop the adoption asked for: the folder it names and the root it names are judged, and nothing is written.
        let stops = match &into {
            Some(into) => {
                let request = Request {
                    into,
                    name: &name,
                    resource: &resource,
                    root,
                };
                mapping::check(&place, &report, &request).err()
            }
            None => root.and_then(|root| mapping::chosen(&report, Some(root)).err()),
        };
        let Some(why) = stops else {
            say(&report, as_json, None);
            return Ok(ExitCode::SUCCESS);
        };
        say(
            &report,
            as_json,
            Some(&json!({ "refused": not_adopted(&why) })),
        );
        return Err(Run(format!(
            "{} would not be adopted, and nothing is written: {why}",
            from.display()
        )));
    }
    let into = into.unwrap_or_default();
    let request = Request {
        into: &into,
        name: &name,
        resource: &resource,
        root,
    };
    match mapping::adopt(&mut place, &report, &request) {
        Ok(adopted) => {
            let made = json!({
                "project": adopted.project.display().to_string(),
                "root": adopted.root.path,
                "sources": adopted.sources.len(),
                "kept": adopted.kept.len(),
                "files": adopted.root.files,
            });
            say(&report, as_json, Some(&made));
            if !as_json {
                // What the adoption made of a file is said again only where it is not what the report said.
                let before = mapping::chosen(&report, request.root).ok();
                for (after, before) in adopted
                    .root
                    .files
                    .iter()
                    .zip(before.iter().flat_map(|root| &root.files))
                {
                    if after != before {
                        println!("adopted as {}", file_line(after));
                    }
                }
                println!(
                    "created {}: {} source(s) made, {} file(s) kept whole; the place adopted from is as it was",
                    adopted.project.display(),
                    adopted.sources.len(),
                    adopted.kept.len()
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(why) => {
            say(
                &report,
                as_json,
                Some(&json!({ "refused": not_adopted(&why) })),
            );
            Err(Run(format!(
                "{} is not adopted, and nothing is written: {why}",
                from.display()
            )))
        }
    }
}

/// Print the report as its lines, or as one JSON document that also holds `adopted` when an adoption was asked for.
fn say(report: &Report, as_json: bool, adopted: Option<&Value>) {
    if as_json {
        let mut document = report.document();
        if let (Value::Object(top), Some(adopted)) = (&mut document, adopted) {
            top.insert("adopted".into(), adopted.clone());
        }
        print_json(vec![document]);
    } else {
        for line in report.lines() {
            println!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn a_folder_with_no_flag_of_a_mapping_is_adopted_in_place_as_it_always_was() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_string_lossy().into_owned();
        assert!(!is_asked(&args(&[&folder])));
        assert!(!is_asked(&args(&[
            &folder,
            "--name",
            "pack",
            "--resource",
            "pack"
        ])));
        for flag in ["--into", "--dry-run", "--json", "--root", "--limits"] {
            assert!(is_asked(&args(&[&folder, flag])), "{flag}");
        }
        let archive = dir.path().join("mapping.zip");
        std::fs::write(&archive, b"x").unwrap();
        assert!(
            is_asked(&args(&[&archive.to_string_lossy()])),
            "a file is a mapping to adopt from"
        );
    }

    #[test]
    fn the_limits_are_pairs_of_a_code_and_a_count_over_the_defaults() {
        let stated = " entries = 12 ,bytes=4096".to_string();
        let limits = limits_of(Some(&stated)).ok().expect("two pairs");
        let expected = Limits {
            entries: 12,
            bytes: 4096,
            ..Limits::default()
        };
        assert_eq!(limits, expected);
        assert!(limits_of(None)
            .ok()
            .is_some_and(|none| none == Limits::default()));
        for refused in ["entries", "entries=many", "entries=-1", "files=3"] {
            assert!(
                matches!(limits_of(Some(&refused.to_string())), Err(Usage(_))),
                "{refused}"
            );
        }
        let lines = limit_lines();
        for row in LIMITS {
            assert!(
                lines.contains(row.code) && lines.contains(row.reason),
                "{}",
                row.code
            );
        }
    }
}
