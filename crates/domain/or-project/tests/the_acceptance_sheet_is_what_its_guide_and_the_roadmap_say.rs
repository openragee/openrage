//! The acceptance sheet names rows the Roadmap has, its guide lists the same lines word for word, and a build is told which of
//! the lines about its files nobody has seen load.
//!
//! ⚠ No test here writes a result into the sheet the tree ships: each lays one over a copy of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_project::acceptance::{kinds_in, Place, Sheet, Verdict, SHEET};
use or_project::{
    build, plan_with, Container, Contribution, ManifestEntry, Module, Origin, Output, OutputKind,
    PlannedFile, Target,
};
use serde_json::{json, Value};

const GUIDE: &str = "docs/guides/In_Game_Acceptance.md";
const ROADMAP: &str = "docs/direction/Roadmap.md";
/// The head of the guide's table of lines, which is in the guide once.
const HEAD: &str = "| Line | Kind | Row | In the game | Look for |";

fn document(path: &str) -> String {
    let root: PathBuf = or_test_support::openrage_root().expect("the tests run inside the tree");
    std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The rows of a Roadmap that have a heading of their own, such as `R32` and `R70a`.
fn rows_of(roadmap: &str) -> BTreeSet<String> {
    (roadmap.lines())
        .filter_map(|line| {
            let heading = line.strip_prefix("##")?.trim_start_matches('#');
            let row = heading.strip_prefix(' ')?.split(' ').next()?;
            let number = row.strip_prefix('R')?;
            number
                .starts_with(|c: char| c.is_ascii_digit())
                .then(|| row.to_string())
        })
        .collect()
}

/// The lines a guide's table lists: the id, and then the kind, the row and the two sentences as the cells say them.
///
/// The id is in backticks and the row is the text of a link, which are how the guide writes them.
fn table_of(guide: &str) -> Vec<[String; 5]> {
    let heads: Vec<usize> = (guide.lines().enumerate())
        .filter(|(_, line)| line.trim() == HEAD)
        .map(|(at, _)| at)
        .collect();
    let [head] = heads.as_slice() else {
        panic!(
            "the guide has its table of lines once, headed `{HEAD}`: found {}",
            heads.len()
        );
    };
    (guide.lines().skip(head + 2))
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cells: Vec<&str> = (line.trim().trim_matches('|').split('|'))
                .map(str::trim)
                .collect();
            let [id, kind, row, action, look_for] = cells.as_slice() else {
                panic!("a line of the table has five cells: {line}");
            };
            let row = row
                .strip_prefix('[')
                .and_then(|rest| rest.split_once(']'))
                .map_or(*row, |(text, _)| text);
            [id.trim_matches('`'), kind, row, action, look_for].map(str::to_string)
        })
        .collect()
}

#[test]
fn every_line_names_a_row_of_the_roadmap() {
    let rows = rows_of(&document(ROADMAP));
    assert!(
        rows.contains("R70a") && rows.contains("R32"),
        "the Roadmap's rows are read from its headings: {} found",
        rows.len()
    );
    let sheet = Sheet::shipped().expect("the sheet the tree ships is a sheet");
    for line in &sheet.lines {
        assert!(
            rows.contains(&line.row),
            "line `{}` names the row {}, which has no heading in {ROADMAP}",
            line.id,
            line.row
        );
    }
}

#[test]
fn the_rows_of_a_roadmap_are_its_headings_and_nothing_else() {
    let sample = "## R1 Land it\n### R70a What the game loads\n#### R34b Audio\n\
                  R99 is prose\n### Risks\n- [ ] [R12 A box](#r12)\n### Rxx Not a row\n";
    let rows: Vec<String> = rows_of(sample).into_iter().collect();
    assert_eq!(rows, ["R1", "R34b", "R70a"]);
}

#[test]
fn the_guide_lists_the_lines_of_the_sheet() {
    let sheet = Sheet::shipped().expect("the sheet the tree ships is a sheet");
    let held: Vec<[String; 5]> = (sheet.lines.iter())
        .map(|line| {
            [
                &line.id,
                &line.kind,
                &line.row,
                &line.action,
                &line.look_for,
            ]
            .map(String::clone)
        })
        .collect();
    assert_eq!(
        table_of(&document(GUIDE)),
        held,
        "the table of {GUIDE} and the lines of acceptance.json are one set, in one order, word for word"
    );
}

#[test]
fn the_table_of_a_guide_is_read_cell_by_cell() {
    let sample = format!(
        "# A guide\n\n| Other | Table |\n|---|---|\n| `not-this` | one |\n\n{HEAD}\n|---|---|---|---|---|\n\
         | `a-line` | ymap | [R32](../direction/Roadmap.md#r32) | Go there. | It is there. |\n\
         | `b-line` | ytyp | R70a | Go. | Look. |\n\nProse after it.\n| `c-line` | x | R1 | a | b |\n"
    );
    assert_eq!(
        table_of(&sample),
        [
            ["a-line", "ymap", "R32", "Go there.", "It is there."].map(str::to_string),
            ["b-line", "ytyp", "R70a", "Go.", "Look."].map(str::to_string),
        ]
    );
}

#[test]
fn the_guide_writes_a_result_the_way_the_sheet_reads_one() {
    let guide = document(GUIDE);
    let (_, guide) = guide
        .split_once("\n## Write down a result\n")
        .expect("the guide says how a result is written, under a heading of its own");
    let examples: Vec<&str> = (guide.split("```json\n").skip(1))
        .filter_map(|rest| rest.split("\n```").next())
        .collect();
    let [example] = examples.as_slice() else {
        panic!(
            "the guide shows the results of a line once, as JSON: found {}",
            examples.len()
        );
    };
    let member: Value =
        serde_json::from_str(&format!("{{{example}}}")).expect("the example is a member of a line");
    // The test's own copy of the sheet, with the guide's example under its first line.
    let mut own: Value = serde_json::from_str(SHEET).unwrap();
    own["lines"][0]["results"] = member["results"].clone();
    let own =
        Sheet::parse(&own.to_string()).expect("the guide's example is a result the sheet reads");
    assert_eq!(own.lines[0].results.len(), 1, "the example is one run");

    let editions = [Target::Legacy, Target::Enhanced].map(Target::word);
    let places = Place::ALL.map(Place::word);
    for word in (editions.iter()).chain(&places).chain(&Verdict::WORDS) {
        assert!(
            guide.contains(&format!("`{word}`")),
            "the guide names `{word}`, a word a result is written with"
        );
    }
}

/// A folder as a build of a FiveM resource leaves it; the names of the files are all that the kinds are read from.
fn a_built_resource(dir: &Path, files: &[&str]) {
    for file in files {
        let at = dir.join(file);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, b"").unwrap();
    }
}

/// The ids of the lines about `kinds`, each with whether the sheet says the game loaded it for `edition`.
fn standing(sheet: &Sheet, kinds: &BTreeSet<String>, edition: Target) -> Vec<(String, bool)> {
    (sheet.about(kinds, edition).iter())
        .map(|concern| (concern.line.id.clone(), concern.loaded()))
        .collect()
}

#[test]
fn a_build_that_wrote_a_map_and_a_type_file_is_told_their_lines_and_that_nobody_checked_them() {
    let dir = tempfile::tempdir().unwrap();
    a_built_resource(
        dir.path(),
        &[
            "stream/harbour.ymap",
            "stream/harbour_types.ytyp",
            "fxmanifest.lua",
        ],
    );
    let counted = kinds_in(dir.path()).expect("the folder holds files");
    assert_eq!(
        counted,
        BTreeMap::from([
            ("lua".to_string(), 1),
            ("ymap".to_string(), 1),
            ("ytyp".to_string(), 1)
        ])
    );
    let kinds: BTreeSet<String> = counted.into_keys().collect();

    let sheet = Sheet::shipped().unwrap();
    let unchecked = vec![
        ("map-object-moved".to_string(), false),
        ("map-object-added".to_string(), false),
        ("type-file-read".to_string(), false),
        ("map-made-from-nothing".to_string(), false),
    ];
    for edition in [Target::Legacy, Target::Enhanced] {
        assert_eq!(standing(&sheet, &kinds, edition), unchecked, "{edition:?}");
    }
    assert_eq!(sheet.without_a_line(&kinds), ["lua"]);

    // The test's own copy of the sheet, with one run written under one line.
    let mut own: Value = serde_json::from_str(SHEET).unwrap();
    let line = (own["lines"].as_array_mut().unwrap().iter_mut())
        .find(|line| line["id"] == "map-object-moved")
        .unwrap();
    line["results"] = json!([{
        "edition": "legacy", "build": "1.0.1", "where": "fivem-server",
        "date": "2026-11-25", "verdict": "loaded",
    }]);
    let own = Sheet::parse(&own.to_string()).expect("a sheet with a result is a sheet");
    assert_eq!(
        standing(&own, &kinds, Target::Legacy),
        [
            ("map-object-moved".to_string(), true),
            ("map-object-added".to_string(), false),
            ("type-file-read".to_string(), false),
            ("map-made-from-nothing".to_string(), false),
        ],
        "the line a result says loaded is checked for the result's edition, and no other line is"
    );
    assert_eq!(
        standing(&own, &kinds, Target::Enhanced),
        unchecked,
        "a result for Legacy says nothing of Enhanced"
    );
    let seen = own.about(&kinds, Target::Legacy)[0]
        .newest
        .expect("the result is the newest");
    assert_eq!(seen.verdict, Verdict::Loaded);
    assert_eq!(
        Sheet::shipped().unwrap(),
        sheet,
        "the sheet the tree ships is as it was"
    );
}

/// A pack of one shop file, which a build of `kind` writes for real.
fn a_built_pack(kind: OutputKind, root: &Path) -> PathBuf {
    let container = Container {
        modules: vec![Module::clothing("wardrobe", "wardrobe")],
        outputs: vec![Output::of(kind, "out", "uniforms", &["wardrobe"])],
    };
    let mut pack = Contribution::of("wardrobe");
    pack.pack = true;
    pack.files = vec![PlannedFile {
        path: "common/data/u_shop.meta".to_string(),
        origin: Origin::Document("<shop/>".to_string()),
    }];
    pack.manifest = vec![ManifestEntry {
        key: "data_file".into(),
        values: vec![
            "SHOP_PED_APPAREL_META_FILE".into(),
            "common/data/u_shop.meta".into(),
        ],
    }];
    let plan = plan_with(&container, "out", Path::new("/projects/uniforms"), &[pack])
        .expect("the output is declared");
    assert!(plan.is_buildable(), "{:?}", plan.problems);
    build(&plan, root).expect("it builds").folder
}

#[test]
fn a_built_pack_is_told_the_line_about_the_archive_it_is() {
    let sheet = Sheet::shipped().unwrap();
    for kind in [OutputKind::SingleplayerDlc, OutputKind::OivPackage] {
        let dir = tempfile::tempdir().unwrap();
        let folder = a_built_pack(kind.clone(), dir.path());
        let counted = kinds_in(&folder).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        assert_eq!(
            counted.get("rpf"),
            Some(&1),
            "{kind:?}: the pack is one file: {counted:?}"
        );
        assert_eq!(counted.get("meta"), Some(&1), "{kind:?}: {counted:?}");
        let package = kind == OutputKind::OivPackage;
        assert_eq!(
            counted.get("oiv"),
            package.then_some(&1),
            "{kind:?}: the package that installs a pack is one file more: {counted:?}"
        );
        let wanted = if package { "oiv" } else { "rpf" };
        let file = (std::fs::read_dir(&folder).unwrap().flatten())
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|x| x == wanted))
            .unwrap_or_else(|| panic!("{kind:?}: the build wrote a .{wanted}"));
        assert_eq!(
            kinds_in(&file).as_ref(),
            Ok(&counted),
            "{kind:?}: the file itself is read as its folder is"
        );
        let kinds: BTreeSet<String> = counted.into_keys().collect();
        assert_eq!(
            standing(&sheet, &kinds, Target::Legacy),
            [("story-pack-mounts".to_string(), false)],
            "{kind:?}"
        );
    }
}

#[test]
fn a_folder_no_build_wrote_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let why = |path: &Path| kinds_in(path).expect_err("it is no build");
    let empty = why(dir.path());
    assert!(
        empty.contains("is not a folder"),
        "an empty folder: {empty}"
    );
    let gone = why(&dir.path().join("gone"));
    assert!(gone.contains("is not a folder"), "no folder: {gone}");
    let loose = dir.path().join("notes.txt");
    std::fs::write(&loose, b"").unwrap();
    let loose = why(&loose);
    assert!(
        loose.contains("is not a folder"),
        "a file that is no pack: {loose}"
    );

    // ⚠ The folder a project builds into holds its resources and their markers, and a project holds its sources: neither is a build, and counted as one each would name kinds no build wrote.
    let project = dir.path().join("project");
    a_built_resource(
        &project,
        &[
            "sources/harbour.metadata.src/base.ymap",
            "build/.or_harbour.openrage-build",
            "build/or_harbour/stream/harbour.ymap",
            "build/or_harbour/fxmanifest.lua",
        ],
    );
    for no_build in [project.clone(), project.join("build")] {
        let said = why(&no_build);
        assert!(said.contains("is not a folder"), "{said}");
    }
    assert_eq!(
        kinds_in(&project.join("build/or_harbour")),
        Ok(BTreeMap::from([
            ("lua".to_string(), 1),
            ("ymap".to_string(), 1)
        ]))
    );
}
