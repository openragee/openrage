//! `openrage-cli maps merge`: two copies of one map file or type file merged by object against the file both were made from
//! (`or_maps::merge`), with the report as text or as data.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use or_maps::changes::SourceChanges;
use or_maps::merge::{
    merge_files, Applied, By, Conflict, Did, Disagreement, Ref, Report, Side, Take,
};
use serde_json::json;

use crate::project::{parse, Failure};
use Failure::{Run, Usage};

pub(crate) fn run(args: &[String]) -> Result<ExitCode, Failure> {
    let (said, merged) = report(args)?;
    print!("{said}");
    Ok(if merged {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// What `maps merge` prints for `args`, and whether a merged file was written.
pub(crate) fn report(args: &[String]) -> Result<(String, bool), Failure> {
    let json_out = args.iter().any(|a| a == "--json");
    let rest: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let (positional, flags) = parse(&rest, &["--base", "--ours", "--theirs", "--out", "--take"])?;
    if let Some(extra) = positional.first() {
        return Err(Usage(format!(
            "maps merge takes its files by flag, and `{extra}` is none"
        )));
    }
    let named = |flag: &str| -> Result<&String, Failure> {
        (flags.get(flag)).ok_or_else(|| Usage(format!("maps merge needs {flag} <file>")))
    };
    let (base, ours, theirs, out) = (
        named("--base")?,
        named("--ours")?,
        named("--theirs")?,
        named("--out")?,
    );
    let take = match flags.get("--take").map(String::as_str) {
        None => Take::Stop,
        Some("ours") => Take::Ours,
        Some("theirs") => Take::Theirs,
        Some(other) => {
            return Err(Usage(format!(
                "--take is `ours` or `theirs`, and `{other}` is neither"
            )))
        }
    };
    // The base says what kind of file the three are: the copies of a resource are often named otherwise.
    let extension = Path::new(base)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let read = |path: &String| std::fs::read(path).map_err(|e| Run(format!("{path}: {e}")));
    let merge = merge_files(
        &extension,
        &read(base)?,
        &read(ours)?,
        &read(theirs)?,
        &mut |_| take,
    )
    .map_err(|e| Run(e.to_string()))?;
    if let Some(bytes) = &merge.merged {
        std::fs::write(out, bytes).map_err(|e| Run(format!("{out}: {e}")))?;
    }
    let written = merge.merged.is_some();
    let said = if json_out {
        let mut data = serde_json::to_value(&merge.report).map_err(|e| Run(e.to_string()))?;
        let out = written.then_some(out);
        data["files"] = json!({ "base": base, "ours": ours, "theirs": theirs, "out": out });
        let mut text = serde_json::to_string_pretty(&data).map_err(|e| Run(e.to_string()))?;
        text.push('\n');
        text
    } else {
        text(&merge.report, out, take)
    };
    Ok((said, written))
}

fn counts(changes: &SourceChanges) -> String {
    let (added, deleted, changed, ambiguous, unchanged) = match changes {
        SourceChanges::Map(c) => (
            c.added.len(),
            c.deleted.len(),
            c.changed.len(),
            c.ambiguous.len(),
            c.unchanged,
        ),
        SourceChanges::Types(c) => (
            c.added.len(),
            c.deleted.len(),
            c.changed.len(),
            c.ambiguous.len(),
            c.unchanged,
        ),
    };
    format!("{added} added, {deleted} deleted, {changed} changed, {ambiguous} ambiguous, {unchanged} unchanged")
}

fn by(side: By) -> &'static str {
    match side {
        By::Ours => "ours",
        By::Theirs => "theirs",
        By::Both => "both",
    }
}

fn indexes(refs: &[Ref]) -> String {
    match refs {
        [] => "-".to_string(),
        refs => {
            let said: Vec<String> = refs.iter().map(|r| r.index().to_string()).collect();
            said.join(", ")
        }
    }
}

fn applied(change: &Applied) -> String {
    let index = |named: &Option<Ref>| {
        named
            .as_ref()
            .map_or("-".to_string(), |r| r.index().to_string())
    };
    let what = [&change.base, &change.ours, &change.theirs]
        .into_iter()
        .flatten()
        .next()
        .map_or(String::new(), Ref::described);
    let places = format!(
        "[base {}; ours {}; theirs {}]",
        index(&change.base),
        index(&change.ours),
        index(&change.theirs)
    );
    let merged = change
        .merged
        .map_or(String::new(), |at| format!(", {at} in the merged file"));
    match change.did {
        Did::Changed => {
            let members: Vec<String> = (change.members.iter())
                .map(|m| format!("{} by {}", m.member, by(m.by)))
                .collect();
            format!("changed {places} {what}{merged}: {}", members.join("; "))
        }
        Did::Deleted => format!("deleted {places} {what}: by {}", by(change.by)),
        Did::Added => {
            let guid = (change.guid.filter(|_| change.merged.is_some()))
                .map_or(String::new(), |g| format!(" with the guid {g}"));
            format!(
                "added   {places} {what}{merged}{guid}: by {}",
                by(change.by)
            )
        }
    }
}

/// The word the JSON of a report names a kind of conflict by.
fn kind(what: &Disagreement) -> &'static str {
    match what {
        Disagreement::Member { .. } => "member",
        Disagreement::Record { .. } => "record",
        Disagreement::ChangedAndDeleted { .. } => "changed-and-deleted",
        Disagreement::Double { .. } => "double",
        Disagreement::Ambiguous => "ambiguous",
        Disagreement::File { .. } => "file",
    }
}

/// The names of members as a sentence lists them, and a word where a side changed none that has a name.
fn listed(members: &[String]) -> String {
    match members {
        [] => "nothing that has a name".to_string(),
        members => members.join(", "),
    }
}

/// One conflict: a line that says its kind, the indexes it names in each file, what each side did and what was taken, then a line
/// for each record it names, as its file indexes it.
fn conflict(conflict: &Conflict) -> String {
    let what = (conflict.base.iter())
        .chain(&conflict.ours)
        .chain(&conflict.theirs)
        .next()
        .map_or(String::new(), Ref::described);
    let values = |ours: &str, theirs: &str| match ours == theirs {
        // What no typed member names has no value to show.
        true => "each to something else".to_string(),
        false => format!("ours to {ours} and theirs to {theirs}"),
    };
    let said = match &conflict.what {
        Disagreement::Member { member, ours, theirs } => {
            format!("both changed {member} of {what}, {}", values(ours, theirs))
        }
        Disagreement::Record { ours, theirs } => format!(
            "{what} is not merged member by member, a side having given it another archetype or another type; ours changed {} and theirs changed {}",
            listed(ours),
            listed(theirs)
        ),
        Disagreement::ChangedAndDeleted { deleted_by, changed } => {
            let changed_by = match deleted_by {
                Side::Ours => Side::Theirs,
                Side::Theirs => Side::Ours,
            };
            format!(
                "{} deleted {what}, of which {} changed {}",
                deleted_by.word(),
                changed_by.word(),
                listed(changed)
            )
        }
        Disagreement::Double { differ } => format!(
            "both added {what}, and the two differ in {}; a likely double",
            listed(differ)
        ),
        Disagreement::Ambiguous => {
            format!("{what} cannot be told from a twin, so nothing says which is which")
        }
        Disagreement::File { member, ours, theirs } => {
            format!("both changed {member} of the file, {}", values(ours, theirs))
        }
    };
    let side = match conflict.taken {
        Take::Ours => Some(Side::Ours),
        Take::Theirs => Some(Side::Theirs),
        Take::Stop => None,
    };
    let taken = match (side, &conflict.what) {
        (None, _) => "stopped".to_string(),
        (Some(side), Disagreement::ChangedAndDeleted { deleted_by, .. }) if side == *deleted_by => {
            format!("took {}, so it is deleted", side.word())
        }
        (Some(side), Disagreement::ChangedAndDeleted { .. }) => {
            format!(
                "took {}, so it is kept as {} has it",
                side.word(),
                side.word()
            )
        }
        (Some(side), Disagreement::Ambiguous) => format!(
            "took {}, so every twin is as {} has it and none of the other side's is merged",
            side.word(),
            side.word()
        ),
        (Some(side), Disagreement::Double { .. }) => {
            format!(
                "took {}, so it is in the merged file once, as {} added it",
                side.word(),
                side.word()
            )
        }
        (Some(side), _) => format!("took {}", side.word()),
    };
    let mut said = format!(
        "{} [base {}; ours {}; theirs {}] {said}: {taken}",
        kind(&conflict.what),
        indexes(&conflict.base),
        indexes(&conflict.ours),
        indexes(&conflict.theirs)
    );
    for (file, refs) in [
        ("base", &conflict.base),
        ("ours", &conflict.ours),
        ("theirs", &conflict.theirs),
    ] {
        for named in refs {
            let _ = write!(
                said,
                "\n       {file} {}: {}",
                named.index(),
                named.described()
            );
        }
    }
    said
}

fn text(report: &Report, out: &str, take: Take) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "ours:   {}", counts(&report.ours));
    let _ = writeln!(text, "theirs: {}", counts(&report.theirs));
    let _ = writeln!(text, "applied: {}", report.applied.len());
    for change in &report.applied {
        let _ = writeln!(text, "  {}", applied(change));
    }
    if !report.file.is_empty() {
        let said: Vec<String> = (report.file.iter())
            .map(|m| format!("{} by {}", m.member, by(m.by)))
            .collect();
        let _ = writeln!(text, "the file: {}", said.join("; "));
    }
    let _ = writeln!(text, "conflicts: {}", report.conflicts.len());
    for (at, found) in report.conflicts.iter().enumerate() {
        let _ = writeln!(text, "  {}. {}", at + 1, conflict(found));
    }
    let _ = writeln!(text, "not compared:");
    for said in &report.not_compared {
        let _ = writeln!(text, "  {said}");
    }
    let findings: Vec<String> = (report.findings.iter())
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    for (title, lines) in [("notes", &report.notes), ("findings", &findings)] {
        if !lines.is_empty() {
            let _ = writeln!(text, "{title}:");
            for said in lines {
                let _ = writeln!(text, "  {said}");
            }
        }
    }
    if report.merged {
        let decided = match take {
            Take::Stop => String::new(),
            _ if report.conflicts.is_empty() => String::new(),
            Take::Ours => format!(", {} conflict(s) decided for ours", report.conflicts.len()),
            Take::Theirs => format!(
                ", {} conflict(s) decided for theirs",
                report.conflicts.len()
            ),
        };
        let _ = writeln!(text, "merged into {out}{decided}");
    } else {
        let _ = writeln!(
            text,
            "not merged: {} conflict(s) stopped it, and {out} was not written; --take ours or --take theirs decides them",
            report.stopped()
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use glam::Vec3;
    use or_io::decl::File;
    use or_io::meta::typed::CMapData;
    use or_maps::edit::{add_entity, place_entity, pose_of, remove_entity, Pose};
    use or_maps::facts::Name;
    use or_maps::sample::{map_file, Entity};
    use serde_json::Value;

    use super::*;

    fn moved(map: &mut CMapData, index: usize, to: [f32; 3]) {
        let pose = pose_of(&map.entities[index]);
        let position = Vec3::from_array(to);
        place_entity(map, index, Pose { position, ..pose }).unwrap();
    }

    /// A base of two crates, a bench and twin trees, with ours and theirs beside it; the paths of the three, and a path to write to.
    fn three_files(dir: &Path) -> [PathBuf; 4] {
        let base = CMapData::from_bytes(&map_file(
            "harbour",
            &[
                Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
                Entity::at("prop_crate_b", [14.0, 20.0, 0.5], 2),
                Entity::at("prop_bench", [18.0, 20.0, 0.5], 3),
                Entity::at("prop_tree", [30.0, 30.0, 0.0], 7),
                Entity::at("prop_tree", [34.0, 30.0, 0.0], 7),
                Entity::at("prop_crate_a", [50.0, 20.0, 0.5], 9),
            ],
            ([0.0; 3], [100.0; 3]),
        ))
        .unwrap();
        // Ours moves the first crate and a twin tree, takes the bench out and adds a crate; theirs moves the first crate elsewhere,
        // moves the bench and the last crate, and adds a crate where ours added one, drawn to another distance.
        let (mut ours, mut theirs) = (base.clone(), base.clone());
        moved(&mut ours, 0, [11.0, 21.0, 0.5]);
        moved(&mut ours, 4, [34.0, 33.0, 0.0]);
        moved(&mut theirs, 0, [9.0, 19.0, 0.5]);
        moved(&mut theirs, 2, [18.0, 22.0, 0.5]);
        moved(&mut theirs, 5, [52.0, 20.0, 0.5]);
        for (side, distance) in [(&mut ours, 100.0), (&mut theirs, 250.0)] {
            let donor = side.entities[1].clone();
            let pose = Pose {
                position: Vec3::new(70.0, 70.0, 0.5),
                ..pose_of(&donor)
            };
            let at = add_entity(side, &donor, Name::from_text("prop_crate_b"), pose).unwrap();
            side.entities[at].lod_dist = distance;
        }
        remove_entity(&mut ours, 2).unwrap();
        let paths = ["base.ymap", "ours.ymap", "theirs.ymap", "merged.ymap"].map(|n| dir.join(n));
        for (path, map) in paths.iter().zip([&base, &ours, &theirs]) {
            std::fs::write(path, map.to_bytes().unwrap()).unwrap();
        }
        paths
    }

    fn args(paths: &[PathBuf; 4], more: &[&str]) -> Vec<String> {
        let mut args = Vec::new();
        for (flag, path) in ["--base", "--ours", "--theirs", "--out"].iter().zip(paths) {
            args.push(flag.to_string());
            args.push(path.to_str().unwrap().to_string());
        }
        args.extend(more.iter().map(ToString::to_string));
        args
    }

    fn said(args: &[String]) -> (String, bool) {
        match report(args) {
            Ok(said) => said,
            Err(Usage(why) | Run(why)) => panic!("{why}"),
        }
    }

    /// A conflict as a report names it: its kind, the indexes it names in the base, in ours and in theirs, and what was taken.
    type Named = (String, Vec<usize>, Vec<usize>, Vec<usize>, String);

    /// The conflicts the lines of the text say, each `N. kind [base a, b; ours c; theirs -] ...: taken`.
    fn in_text(text: &str) -> Vec<Named> {
        // A line for each conflict, and under it a line for each record it names, which starts further in.
        let lines = text
            .lines()
            .skip_while(|l| !l.starts_with("conflicts:"))
            .skip(1)
            .take_while(|l| l.starts_with("  "))
            .filter(|l| !l.starts_with("   "));
        lines
            .map(|line| {
                let (_, rest) = line.trim_start().split_once(". ").expect("a number");
                let (kind, rest) = rest.split_once(" [").expect("the indexes");
                let (places, rest) = rest.split_once("] ").expect("the indexes close");
                let of = |word: &str| -> Vec<usize> {
                    let list = (places.split("; "))
                        .find_map(|part| part.strip_prefix(word))
                        .expect("a file");
                    (list.split(", ").filter(|i| *i != "-"))
                        .map(|i| i.parse().expect("an index"))
                        .collect()
                };
                // What was taken, and after a comma what that leaves.
                let taken = rest.rsplit_once(": ").expect("what was taken").1;
                let taken = match taken.split_once(", ").map_or(taken, |(taken, _)| taken) {
                    "took ours" => "ours",
                    "took theirs" => "theirs",
                    "stopped" => "stop",
                    other => panic!("{other}"),
                };
                (
                    kind.to_string(),
                    of("base "),
                    of("ours "),
                    of("theirs "),
                    taken.to_string(),
                )
            })
            .collect()
    }

    /// The conflicts the JSON of a report lists.
    fn in_json(report: &Value) -> Vec<Named> {
        let indexes = |refs: &Value| -> Vec<usize> {
            (refs.as_array().into_iter().flatten())
                .map(|r| r["index"].as_u64().expect("an index") as usize)
                .collect()
        };
        (report["conflicts"].as_array().into_iter().flatten())
            .map(|c| {
                (
                    c["what"]["kind"].as_str().expect("a kind").to_string(),
                    indexes(&c["base"]),
                    indexes(&c["ours"]),
                    indexes(&c["theirs"]),
                    c["taken"].as_str().expect("what was taken").to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn the_json_of_a_merge_lists_the_conflicts_its_text_lists() {
        let dir = tempfile::tempdir().unwrap();
        let paths = three_files(dir.path());

        let (text, written) = said(&args(&paths, &[]));
        assert!(!written, "a conflict stopped it");
        assert!(!paths[3].exists(), "and nothing was written");
        let (data, written_json) = said(&args(&paths, &["--json"]));
        assert!(!written_json);
        let data: Value = serde_json::from_str(&data).unwrap();
        let named = |kind: &str, base: &[usize], ours: &[usize], theirs: &[usize]| -> Named {
            (
                kind.to_string(),
                base.to_vec(),
                ours.to_vec(),
                theirs.to_vec(),
                "stop".to_string(),
            )
        };
        let expected = [
            named("ambiguous", &[4], &[3], &[4]),
            named("member", &[0], &[0], &[0]),
            named("changed-and-deleted", &[2], &[], &[2]),
            named("double", &[], &[5], &[6]),
        ];
        assert_eq!(in_json(&data), expected);
        assert_eq!(in_text(&text), expected, "{text}");
        assert_eq!(data["merged"], false);
        // The two values of the member both changed are in both.
        assert_eq!(
            data["conflicts"][1]["what"],
            json!({ "kind": "member", "member": "position", "ours": "(11, 21, 0.5)", "theirs": "(9, 19, 0.5)" })
        );
        assert!(
            text.contains("ours to (11, 21, 0.5) and theirs to (9, 19, 0.5)"),
            "{text}"
        );
        // What each side did is in both: the members one changed of what the other deleted, and what a double differs in.
        assert_eq!(
            data["conflicts"][2]["what"],
            json!({ "kind": "changed-and-deleted", "deleted_by": "ours", "changed": ["position"] })
        );
        assert_eq!(
            data["conflicts"][3]["what"],
            json!({ "kind": "double", "differ": ["lodDist"] })
        );
        // Each conflict is a line, and under it each record it names as its file indexes it, with what it places and where.
        let (tree, bench, crate_b) = (
            Name::of(Name::from_text("prop_tree").hash).shown(),
            Name::of(Name::from_text("prop_bench").hash).shown(),
            Name::of(Name::from_text("prop_crate_b").hash).shown(),
        );
        let lines: Vec<&str> = (text.lines())
            .skip_while(|l| !l.starts_with("conflicts:"))
            .skip(1)
            .take_while(|l| l.starts_with("  "))
            .collect();
        let expected_lines = [
            format!("  1. ambiguous [base 4; ours 3; theirs 4] {tree} at (34, 30, 0), guid 7 cannot be told from a twin, so nothing says which is which: stopped"),
            format!("       base 4: {tree} at (34, 30, 0), guid 7"),
            format!("       ours 3: {tree} at (34, 33, 0), guid 7"),
            format!("       theirs 4: {tree} at (34, 30, 0), guid 7"),
            format!("  3. changed-and-deleted [base 2; ours -; theirs 2] ours deleted {bench} at (18, 20, 0.5), guid 3, of which theirs changed position: stopped"),
            format!("       base 2: {bench} at (18, 20, 0.5), guid 3"),
            format!("       theirs 2: {bench} at (18, 22, 0.5), guid 3"),
            format!("  4. double [base -; ours 5; theirs 6] both added {crate_b} at (70, 70, 0.5), guid 10, and the two differ in lodDist; a likely double: stopped"),
            format!("       ours 5: {crate_b} at (70, 70, 0.5), guid 10"),
            format!("       theirs 6: {crate_b} at (70, 70, 0.5), guid 10"),
        ];
        for line in &expected_lines {
            assert!(lines.contains(&line.as_str()), "{line}\nis not in\n{text}");
        }
        assert_eq!(data["files"]["out"], Value::Null);
        assert!(
            text.ends_with(&format!(
                "not merged: 4 conflict(s) stopped it, and {} was not written; --take ours or --take theirs decides them\n",
                paths[3].display()
            )),
            "{text}"
        );
        // What was applied beside the conflicts is in both: the crate only theirs moved. A file states the hash of a name alone.
        let crate_a = Name::of(Name::from_text("prop_crate_a").hash).shown();
        assert_eq!(
            text.lines().find(|l| l.starts_with("applied")),
            Some("applied: 1"),
            "{text}"
        );
        assert!(
            text.contains(&format!("changed [base 5; ours 4; theirs 5] {crate_a} at (50, 20, 0.5), guid 9: position by theirs")),
            "{text}"
        );
        assert_eq!(data["applied"][0]["did"], "changed");
        assert_eq!(data["applied"][0]["base"]["index"], 5);
        for said in data["not_compared"].as_array().unwrap() {
            assert!(text.contains(said.as_str().unwrap()), "{text}");
        }

        // With a side taken the file is written, and the two reports say what was taken of each conflict.
        for (take, place) in [("ours", [11.0, 21.0]), ("theirs", [9.0, 19.0])] {
            let (text, written) = said(&args(&paths, &["--take", take]));
            assert!(written);
            let (data, _) = said(&args(&paths, &["--take", take, "--json"]));
            let data: Value = serde_json::from_str(&data).unwrap();
            let taken: Vec<Named> = (expected.iter().cloned())
                .map(|(kind, base, ours, theirs, _)| (kind, base, ours, theirs, take.to_string()))
                .collect();
            assert_eq!(in_json(&data), taken);
            assert_eq!(in_text(&text), taken, "{text}");
            assert_eq!(data["files"]["out"], paths[3].to_str().unwrap());
            let at = if take == "ours" { 4 } else { 5 };
            assert_eq!(
                data["applied"][0]["merged"], at,
                "the bench ours deleted is before it"
            );
            assert!(
                text.contains(&format!(
                    "guid 9, {at} in the merged file: position by theirs"
                )),
                "{text}"
            );
            let merged = CMapData::from_bytes(&std::fs::read(&paths[3]).unwrap()).unwrap();
            assert_eq!(
                [merged.entities[0].position.x, merged.entities[0].position.y],
                place
            );
            assert!(
                text.ends_with(&format!(
                    "merged into {}, 4 conflict(s) decided for {take}\n",
                    paths[3].display()
                )),
                "{text}"
            );
            // What was taken is said with what it leaves of the entity.
            let left = match take {
                "ours" => "took ours, so it is deleted",
                _ => "took theirs, so it is kept as theirs has it",
            };
            let line = text.lines().find(|l| l.contains("changed-and-deleted"));
            assert!(line.is_some_and(|l| l.ends_with(left)), "{text}");
            assert!(
                text.contains(&format!(": took {take}, so every twin is as {take} has it and none of the other side's is merged")),
                "{text}"
            );
        }
    }

    #[test]
    fn a_merge_is_refused_with_its_usage_when_a_file_or_a_side_is_not_named() {
        let dir = tempfile::tempdir().unwrap();
        let paths = three_files(dir.path());
        let usage = |args: &[String]| match report(args) {
            Err(Usage(why)) => why,
            other => panic!("{:?}", other.map_err(|_| "a failure that is no usage")),
        };
        let all = args(&paths, &[]);
        assert_eq!(usage(&all[..6]), "maps merge needs --out <file>");
        assert_eq!(
            usage(&args(&paths, &["--take", "mine"])),
            "--take is `ours` or `theirs`, and `mine` is neither"
        );
        assert!(usage(&args(&paths, &["extra.ymap"])).contains("extra.ymap"));

        // A file of another kind, and a file that is not there, fail the command with the reason.
        let mut other = paths.clone();
        other[0] = dir.path().join("base.ybn");
        std::fs::copy(&paths[0], &other[0]).unwrap();
        let Err(Run(why)) = report(&args(&other, &[])) else {
            panic!("a bound is no map");
        };
        assert!(why.contains("neither a map file nor a type file"), "{why}");
        other[0] = dir.path().join("absent.ymap");
        let Err(Run(why)) = report(&args(&other, &[])) else {
            panic!("the base is not there");
        };
        assert!(why.contains("absent.ymap"), "{why}");
    }
}
