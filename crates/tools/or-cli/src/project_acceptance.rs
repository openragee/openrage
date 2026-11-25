//! `openrage-cli project acceptance`: the lines of the acceptance sheet a built folder concerns, and which nobody has checked in game.
//!
//! ⚠ It reads the sheet the binary was built with and the names of the files a build wrote. It runs nothing and writes nothing, and a
//! line it prints as loaded is a result a person wrote into the sheet.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use or_project::acceptance::{kinds_in, Concern, Sheet, Verdict};
use or_project::Target;

use crate::project::{parse, Failure};
use Failure::{Run, Usage};

/// The two editions a line is checked for, in the order they are printed.
const EDITIONS: [Target; 2] = [Target::Legacy, Target::Enhanced];

pub(crate) fn acceptance(args: &[String]) -> Result<ExitCode, Failure> {
    let sheet = Sheet::shipped().map_err(|e| Run(e.to_string()))?;
    acceptance_to(args, &sheet, &mut std::io::stdout().lock())
}

/// What the sheet holds of a line for one edition, as the end of a line that names the edition.
fn standing(concern: &Concern) -> String {
    let Some(seen) = concern.newest else {
        return "not checked in game".to_string();
    };
    let when = format!(
        "{} on {}, game build {}",
        seen.place.label(),
        seen.date,
        seen.build
    );
    match &seen.verdict {
        Verdict::Loaded => format!("loaded {when}"),
        Verdict::NotLoaded => format!("not loaded {when}"),
        Verdict::LoadedWithFault(fault) => format!("loaded with a fault {when}: {fault}"),
    }
}

/// Prints the lines of `sheet` about the build the arguments name to `out`.
fn acceptance_to(
    args: &[String],
    sheet: &Sheet,
    out: &mut impl Write,
) -> Result<ExitCode, Failure> {
    let (positional, _) = parse(args, &[])?;
    let [built] = positional.as_slice() else {
        return Err(Usage("expected one <built folder>".into()));
    };
    let counted = kinds_in(Path::new(built)).map_err(Run)?;
    let kinds: BTreeSet<String> = counted.keys().cloned().collect();
    let mut text = format!(
        "{built}: {} file(s) of {} kind(s)\n",
        counted.values().sum::<usize>(),
        counted.len()
    );
    let by_edition = EDITIONS.map(|edition| sheet.about(&kinds, edition));
    let [first, _] = &by_edition;
    for (index, concern) in first.iter().enumerate() {
        let line = concern.line;
        text.push_str(&format!(
            "  {} (.{}, {} file(s); {})\n    do:       {}\n    look for: {}\n",
            line.id,
            line.kind,
            counted.get(&line.kind).copied().unwrap_or_default(),
            line.row,
            line.action,
            line.look_for
        ));
        for (edition, concerns) in EDITIONS.iter().zip(&by_edition) {
            let label = format!("{}:", edition.generation().edition_label());
            text.push_str(&format!("    {label:<9} {}\n", standing(&concerns[index])));
        }
    }
    let silent: Vec<String> = (sheet.without_a_line(&kinds).iter())
        .map(|kind| format!(".{kind} ({} file(s))", counted[*kind]))
        .collect();
    if !silent.is_empty() {
        text.push_str(&format!(
            "  the sheet has no line about: {}\n",
            silent.join(", ")
        ));
    }
    if !first.is_empty() {
        let unseen: Vec<String> = (EDITIONS.iter().zip(&by_edition))
            .map(|(edition, concerns)| {
                format!(
                    "{} of {} line(s) for {}",
                    concerns.iter().filter(|c| !c.loaded()).count(),
                    concerns.len(),
                    edition.generation().edition_label()
                )
            })
            .collect();
        text.push_str(&format!(
            "not seen to load in game: {}\n",
            unseen.join(", ")
        ));
    }
    text.push_str(
        "A build checks the files. Only a result a person wrote into the sheet says GTA V loaded them:\n\
         docs/guides/In_Game_Acceptance.md says how.\n",
    );
    out.write_all(text.as_bytes())
        .map_err(|e| Run(e.to_string()))?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use or_project::acceptance::SHEET;
    use serde_json::{json, Value};

    use super::*;

    /// A folder as a build of a FiveM resource leaves it, with two maps, a type file and the manifest; the names are all the command reads.
    fn a_built_resource(dir: &Path) -> String {
        std::fs::create_dir_all(dir.join("stream")).unwrap();
        for file in [
            "stream/harbour.ymap",
            "stream/quay.ymap",
            "stream/harbour_types.ytyp",
            "fxmanifest.lua",
        ] {
            std::fs::write(dir.join(file), b"").unwrap();
        }
        dir.to_str().expect("a scratch path is UTF-8").to_string()
    }

    fn printed(args: &[String], sheet: &Sheet) -> String {
        let mut out = Vec::new();
        let code = acceptance_to(args, sheet, &mut out);
        assert!(matches!(code, Ok(code) if code == ExitCode::SUCCESS));
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_command_names_the_lines_the_sheet_holds_about_what_a_build_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let built = a_built_resource(dir.path());
        let sheet = Sheet::shipped().unwrap();
        let text = printed(std::slice::from_ref(&built), &sheet);
        assert!(
            text.starts_with(&format!("{built}: 4 file(s) of 3 kind(s)\n")),
            "{text}"
        );

        let kinds = BTreeSet::from(["ymap".to_string(), "ytyp".to_string(), "lua".to_string()]);
        let about = sheet.about(&kinds, Target::Legacy);
        assert!(
            about.len() >= 2,
            "the sheet holds a line about a map and one about a type file"
        );
        // The ids at the head of a line of the output, in its order, are the ones the sheet answers with.
        let named: Vec<&str> = (text.lines())
            .filter_map(|line| line.strip_prefix("  ")?.split_once(" (.").map(|(id, _)| id))
            .filter(|id| !id.starts_with(' '))
            .collect();
        let held: Vec<&str> = about.iter().map(|c| c.line.id.as_str()).collect();
        assert_eq!(named, held);
        for concern in &about {
            assert!(text.contains(&concern.line.action) && text.contains(&concern.line.look_for));
            let files = if concern.line.kind == "ymap" { 2 } else { 1 };
            let head = format!(
                "  {} (.{}, {files} file(s); {})\n",
                concern.line.id, concern.line.kind, concern.line.row
            );
            assert!(text.contains(&head), "{head}{text}");
        }
        for line in sheet
            .lines
            .iter()
            .filter(|line| !kinds.contains(&line.kind))
        {
            assert!(
                !text.contains(&line.id),
                "{} is about no file of the build",
                line.id
            );
        }
        assert_eq!(
            text.matches("not checked in game").count(),
            2 * about.len(),
            "nobody ran the game, for either edition:\n{text}"
        );
        assert!(
            text.contains("the sheet has no line about: .lua (1 file(s))"),
            "{text}"
        );
        let all = format!("{0} of {0} line(s)", about.len());
        assert!(
            text.contains(&format!(
                "not seen to load in game: {all} for Legacy, {all} for Enhanced"
            )),
            "{text}"
        );
    }

    #[test]
    fn a_result_in_the_sheet_is_printed_for_its_edition_and_the_other_stays_unchecked() {
        let dir = tempfile::tempdir().unwrap();
        let built = a_built_resource(dir.path());
        // The test's own copy of the sheet: no result is written into the one the tree ships.
        let mut own: Value = serde_json::from_str(SHEET).unwrap();
        let line = own["lines"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|line| line["kind"] == "ytyp")
            .expect("a line about a type file");
        let id = line["id"].as_str().unwrap().to_string();
        line["results"] = json!([{
            "edition": "enhanced", "build": "1.0.1", "where": "fivem-server",
            "date": "2026-11-25", "verdict": "loaded-with-fault", "fault": "The object flickers.",
        }]);
        let sheet = Sheet::parse(&own.to_string()).unwrap();
        let text = printed(&[built], &sheet);

        let (_, after) = text
            .split_once(&format!("  {id} ("))
            .expect("the line is printed");
        let block: Vec<&str> = after.lines().skip(1).take(4).collect();
        assert_eq!(block[2].trim(), "Legacy:   not checked in game", "{text}");
        assert_eq!(
            block[3].trim(),
            "Enhanced: loaded with a fault on a FiveM server on 2026-11-25, game build 1.0.1: The object flickers.",
            "{text}"
        );
    }

    #[test]
    fn a_line_seen_to_load_is_printed_as_loaded_and_leaves_the_count_of_those_not_seen() {
        let dir = tempfile::tempdir().unwrap();
        let built = a_built_resource(dir.path());
        // The test's own copy of the sheet: no result is written into the one the tree ships.
        let mut own: Value = serde_json::from_str(SHEET).unwrap();
        let run = |edition: &str, place: &str, verdict: &str| {
            json!([{
                "edition": edition, "build": "3258", "where": place,
                "date": "2026-11-25", "verdict": verdict,
            }])
        };
        let lines = own["lines"].as_array_mut().unwrap();
        let ids: Vec<String> = (lines.iter())
            .filter(|line| line["kind"] == "ymap")
            .map(|line| line["id"].as_str().unwrap().to_string())
            .collect();
        let [loaded, refused, ..] = ids.as_slice() else {
            panic!("the sheet holds two lines about a map or more: {ids:?}");
        };
        for line in lines.iter_mut() {
            if line["id"] == loaded.as_str() {
                line["results"] = run("legacy", "fivem-server", "loaded");
            } else if line["id"] == refused.as_str() {
                line["results"] = run("legacy", "story-mode", "not-loaded");
            }
        }
        let sheet = Sheet::parse(&own.to_string()).unwrap();
        let text = printed(&[built], &sheet);

        // The two lines under the head of the line `id` that name an edition.
        let editions = |id: &str| -> Vec<String> {
            let (_, after) = text
                .split_once(&format!("  {id} ("))
                .expect("the line is printed");
            (after.lines().skip(3).take(2))
                .map(|line| line.trim().to_string())
                .collect()
        };
        assert_eq!(
            editions(loaded),
            [
                "Legacy:   loaded on a FiveM server on 2026-11-25, game build 3258",
                "Enhanced: not checked in game",
            ],
            "{text}"
        );
        assert_eq!(
            editions(refused),
            [
                "Legacy:   not loaded in story mode on 2026-11-25, game build 3258",
                "Enhanced: not checked in game",
            ],
            "{text}"
        );
        let lines = sheet.about(&["ymap", "ytyp"].map(str::to_string).into(), Target::Legacy);
        let unseen = lines.len() - 1;
        assert!(
            text.contains(&format!(
                "not seen to load in game: {unseen} of {0} line(s) for Legacy, {0} of {0} line(s) for Enhanced",
                lines.len()
            )),
            "{text}"
        );
    }

    #[test]
    fn a_folder_that_is_no_build_is_refused_and_a_flag_is_a_usage_error() {
        let sheet = Sheet::shipped().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone").to_string_lossy().into_owned();
        let empty = dir.path().to_string_lossy().into_owned();
        let mut out = Vec::new();
        assert!(matches!(
            acceptance_to(&[gone], &sheet, &mut out),
            Err(Run(_))
        ));
        assert!(matches!(
            acceptance_to(std::slice::from_ref(&empty), &sheet, &mut out),
            Err(Run(_))
        ));
        assert!(matches!(
            acceptance_to(&[], &sheet, &mut out),
            Err(Usage(_))
        ));
        let built = a_built_resource(&dir.path().join("built"));
        for flag in [
            vec!["--json"],
            vec!["--json", "lines.json"],
            vec!["--build", "or_acceptance"],
        ] {
            let mut args = vec![built.clone()];
            args.extend(flag.iter().map(ToString::to_string));
            assert!(
                matches!(acceptance_to(&args, &sheet, &mut out), Err(Usage(_))),
                "the command takes no flag: {flag:?}"
            );
        }
        assert!(
            matches!(
                acceptance_to(&[built, empty], &sheet, &mut out),
                Err(Usage(_))
            ),
            "one build is read at a time"
        );
        assert_eq!(
            out,
            Vec::<u8>::new(),
            "a refusal prints no line of the sheet"
        );
    }
}
