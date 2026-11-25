//! `openrage-cli project acceptance`: the lines of the acceptance sheet a built folder concerns, and which nobody has checked in game.
//!
//! ⚠ It reads the sheet the binary was built with and the names of the files a build wrote. It runs nothing and writes nothing, and a
//! line it prints as loaded is a result a person wrote into the sheet.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use or_project::acceptance::{kinds_in, Concern, Place, Seen, Sheet, Verdict};
use or_project::Target;

use crate::project::{parse, Failure};
use Failure::{Run, Usage};

/// The two editions a line is checked for, in the order they are printed.
const EDITIONS: [Target; 2] = [Target::Legacy, Target::Enhanced];

pub(crate) fn acceptance(args: &[String]) -> Result<ExitCode, Failure> {
    let sheet = Sheet::shipped().map_err(|e| Run(e.to_string()))?;
    acceptance_to(args, &sheet, &mut std::io::stdout().lock())
}

/// What a build's report says of `sheet` for the folder it wrote: what was seen in game of its kinds of file, and where the steps to check are.
///
/// ⚠ The sentences are `Sheet::seen`'s, which the Build tab says too, so the command and the screen read one sheet one way: by the
/// results from where the build loads, story mode for a pack or a package and a server for a resource folder.
pub(crate) fn seen_lines(sheet: &Sheet, built: &Path, stated: Option<Target>) -> Vec<String> {
    let kinds: BTreeSet<String> = match kinds_in(built) {
        Ok(counted) => counted.into_keys().collect(),
        Err(why) => return vec![format!("not read against the acceptance sheet: {why}")],
    };
    let seen = sheet.seen(&kinds, stated, Place::of_built(built));
    let mut lines: Vec<String> = (seen.iter())
        .map(|(edition, seen)| seen.sentence(*edition))
        .collect();
    if !seen.is_empty() {
        lines.extend(sheet.silent_sentence(&kinds));
    }
    if seen.iter().any(|(_, seen)| matches!(seen, Seen::Not(_))) {
        // ⚠ Between quotes, since a folder under a person's own name often holds a space, and the line is there to be copied.
        lines.push(format!(
            "to check: openrage-cli project acceptance \"{}\" lists what to look for, and docs/guides/In_Game_Acceptance.md says how",
            built.display()
        ));
    }
    lines
}

/// Prints [`seen_lines`] for the sheet the binary was built with, under the report of the build that wrote `built`.
pub(crate) fn report_seen(built: &Path, stated: Option<Target>) {
    let lines = match Sheet::shipped() {
        Ok(sheet) => seen_lines(&sheet, built, stated),
        Err(e) => vec![e.to_string()],
    };
    for line in lines {
        println!("  {line}");
    }
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
        // ⚠ Counted by the runs from where this build loads: a line seen only in the other place is not seen for it.
        let place = Place::of_built(Path::new(built));
        let unseen: Vec<String> = (EDITIONS.iter().zip(&by_edition))
            .map(|(edition, concerns)| {
                let not_seen = |c: &&Concern| !c.line.loaded_in(*edition, place);
                format!(
                    "{} of {} line(s) for {}",
                    concerns.iter().filter(not_seen).count(),
                    concerns.len(),
                    edition.generation().edition_label()
                )
            })
            .collect();
        text.push_str(&format!(
            "not seen to load {}: {}\n",
            place.label(),
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

    /// The sheet the tree ships, as JSON with every result taken out: what a test lays its own results over, so that none depends on what a person has written into the sheet.
    fn the_sheet_with_no_result() -> Value {
        let mut own: Value = serde_json::from_str(SHEET).unwrap();
        for line in own["lines"].as_array_mut().unwrap() {
            line["results"] = json!([]);
        }
        own
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
        let sheet = Sheet::parse(&the_sheet_with_no_result().to_string()).unwrap();
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
                "not seen to load on a FiveM server: {all} for Legacy, {all} for Enhanced"
            )),
            "{text}"
        );
    }

    #[test]
    fn a_result_in_the_sheet_is_printed_for_its_edition_and_the_other_stays_unchecked() {
        let dir = tempfile::tempdir().unwrap();
        let built = a_built_resource(dir.path());
        // The test's own copy of the sheet: no result is written into the one the tree ships.
        let mut own = the_sheet_with_no_result();
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
        let mut own = the_sheet_with_no_result();
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
                "not seen to load on a FiveM server: {unseen} of {0} line(s) for Legacy, {0} of {0} line(s) for Enhanced",
                lines.len()
            )),
            "{text}"
        );
    }

    #[test]
    fn a_builds_report_says_what_the_sheet_says_of_its_kinds_and_where_the_steps_are() {
        let dir = tempfile::tempdir().unwrap();
        let built = a_built_resource(dir.path());
        let folder = Path::new(&built);
        let kinds: BTreeSet<String> = ["ymap", "ytyp", "lua"].map(str::to_string).into();
        let to_check = format!(
            "to check: openrage-cli project acceptance \"{built}\" lists what to look for, and docs/guides/In_Game_Acceptance.md says how"
        );

        let no_line =
            "The acceptance sheet has no line about what else this build wrote: .lua files";
        let shipped = Sheet::parse(&the_sheet_with_no_result().to_string()).unwrap();
        let said = |sheet: &Sheet, stated: Option<Target>| -> Vec<String> {
            (sheet.seen(&kinds, stated, Place::FivemServer).iter())
                .map(|(edition, seen)| seen.sentence(*edition))
                .collect()
        };
        let lines = seen_lines(&shipped, folder, Some(Target::Legacy));
        assert_eq!(
            lines[..1],
            said(&shipped, Some(Target::Legacy)),
            "{lines:?}"
        );
        assert_eq!(
            lines,
            [
                "2 kinds of file this build wrote were not seen loading in GTA V Legacy: maps, type files",
                no_line,
                to_check.as_str(),
            ]
        );

        // The test's own copy of the sheet, with every line about a map or a type file seen to load in Legacy.
        let mut own: Value = serde_json::from_str(SHEET).unwrap();
        for line in own["lines"].as_array_mut().unwrap() {
            if line["kind"] == "ymap" || line["kind"] == "ytyp" {
                line["results"] = json!([{
                    "edition": "legacy", "build": "3258", "where": "fivem-server",
                    "date": "2026-12-01", "verdict": "loaded",
                }]);
            }
        }
        let own = Sheet::parse(&own.to_string()).unwrap();
        assert_eq!(
            seen_lines(&own, folder, Some(Target::Legacy)),
            [
                "Every kind of file this build wrote that the acceptance sheet has a line about was seen loading in GTA V Legacy; the newest result is dated 2026-12-01",
                no_line,
            ],
            "a build whose kinds were all seen is sent to no steps"
        );
        let unstated = seen_lines(&own, folder, None);
        assert_eq!(unstated[..2], said(&own, None), "{unstated:?}");
        assert_eq!(
            unstated[1..],
            [
                "2 kinds of file this build wrote were not seen loading in GTA V Enhanced: maps, type files",
                no_line,
                to_check.as_str(),
            ],
            "a build that states no edition is read for both: {unstated:?}"
        );

        // A build of scripts alone wrote nothing a line is about, and its report gains nothing.
        let scripts = dir.path().join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        for file in ["client.lua", "fxmanifest.lua"] {
            std::fs::write(scripts.join(file), b"").unwrap();
        }
        assert_eq!(seen_lines(&shipped, &scripts, None), [] as [&str; 0]);

        // The same files in a pack are read by the runs in story mode: the runs on a server, which saw the resource, say nothing of it.
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("dlc.rpf"), b"").unwrap();
        assert_eq!(Place::of_built(&pack), Place::StoryMode);
        assert_eq!(Place::of_built(folder), Place::FivemServer);
        let kinds_of_a_pack: BTreeSet<String> = ["ymap", "ytyp", "rpf"].map(str::to_string).into();
        assert!(matches!(
            own.seen(&kinds_of_a_pack, Some(Target::Legacy), Place::StoryMode).as_slice(),
            [(Target::Legacy, Seen::Not(unseen))] if unseen.contains(&"ymap".to_string())
        ));

        let gone = seen_lines(&shipped, &dir.path().join("gone"), None);
        assert!(
            matches!(gone.as_slice(), [one] if one.starts_with("not read against the acceptance sheet: ")),
            "{gone:?}"
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
