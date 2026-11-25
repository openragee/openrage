//! A project of loose files builds a resource that reads back the same, builds the same
//! twice, and never damages the last good build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_project::{build, plan, verify, BuildError, Container, Output, Plan, Project};

/// Every byte value, so that no text decoding of a copy goes unnoticed, and big enough to span reads.
fn blob() -> Vec<u8> {
    (0..1_048_576u32)
        .map(|i| (i % 251) as u8 ^ (i >> 9) as u8)
        .collect()
}

fn write(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// Every file under `dir` by its `/` path, with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A project folder holding `files`, created through the container and read back.
fn project(dir: &Path, files: &[(&str, &[u8])]) -> Project {
    project_in(&dir.join("pack"), files)
}

/// The same project, in the folder `folder`.
fn project_in(folder: &Path, files: &[(&str, &[u8])]) -> Project {
    let members: Vec<PathBuf> = files
        .iter()
        .map(|(rel, body)| {
            write(&folder.join(rel), body);
            folder.join(rel)
        })
        .collect();
    let mut container = Container::default();
    container
        .outputs
        .push(Output::fivem_resource("fivem", "pack", &["files"]));
    let file = folder.join("pack.openrage-proj");
    Project::create(&file, &container, &members).expect("a new project");
    Project::load(&file).expect("it reads back")
}

fn planned(project: &Project) -> Plan {
    plan(
        &project.container,
        "fivem",
        &project.folder(),
        &project.sources(),
    )
    .expect("the output is declared")
}

fn loose_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("client.lua", b"print('hello')\r\n".to_vec()),
        (
            "data/vehicles.meta",
            b"<?xml version=\"1.0\"?>\n<CVehicleModelInfo__InitDataList/>\n".to_vec(),
        ),
        ("stream/big.bin", blob()),
        ("html/index.html", "<p>é</p>".as_bytes().to_vec()),
    ]
}

fn borrowed<'a>(files: &'a [(&'static str, Vec<u8>)]) -> Vec<(&'static str, &'a [u8])> {
    files
        .iter()
        .map(|(rel, body)| (*rel, body.as_slice()))
        .collect()
}

#[test]
fn a_project_of_loose_files_builds_a_resource_that_reads_back_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let files = loose_files();
    let project = project(dir.path(), &borrowed(&files));
    let plan = planned(&project);
    assert!(plan.is_buildable(), "{:?}", plan.problems);
    let root = dir.path().join("out");

    let built = build(&plan, &root).expect("the build succeeds");

    assert_eq!(built.folder, root.join("pack"));
    assert!(built.previous.is_none(), "there was no build to keep");
    let written = snapshot(&built.folder);
    let mut expected: Vec<&str> = files.iter().map(|(rel, _)| *rel).collect();
    expected.push("fxmanifest.lua");
    expected.sort_unstable();
    assert_eq!(
        written.keys().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
    for (rel, body) in &files {
        assert_eq!(&written[*rel], body, "{rel} is not what the project holds");
    }
    assert!(String::from_utf8_lossy(&written["fxmanifest.lua"]).contains("'client.lua',"));

    assert_eq!(built.verification.problems, Vec::<String>::new());
    assert_eq!(built.verification.checked, written.len());
    let again = verify(&plan, &built.folder);
    assert_eq!(
        again, built.verification,
        "verifying the live folder says the same"
    );
    assert!(
        !root.join(".pack.new").exists(),
        "the staging folder is gone"
    );
}

#[test]
fn two_builds_of_one_project_are_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let files = loose_files();
    let project = project(dir.path(), &borrowed(&files));
    let plan = planned(&project);

    let first = build(&plan, &dir.path().join("a")).unwrap();
    let elsewhere = build(&plan, &dir.path().join("b")).unwrap();
    assert_eq!(snapshot(&first.folder), snapshot(&elsewhere.folder));

    let wrote_first = snapshot(&first.folder);
    let rebuilt = build(&planned(&project), &dir.path().join("a")).unwrap();
    assert_eq!(
        snapshot(&rebuilt.folder),
        wrote_first,
        "a rebuild over the first one differs"
    );
}

#[test]
fn a_rebuild_keeps_the_build_it_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"one"), ("b.lua", b"b")]);
    let root = dir.path().join("out");
    let first = build(&planned(&project), &root).unwrap();
    let before = snapshot(&first.folder);

    write(&project.folder().join("a.lua"), b"two, longer");
    let second = build(&planned(&project), &root).unwrap();

    assert_eq!(snapshot(&second.folder)["a.lua"], b"two, longer");
    let kept = second.previous.expect("the replaced build is kept");
    assert_eq!(snapshot(&kept), before);
    assert_eq!(kept, root.join(".pack.previous"));

    let third = build(&planned(&project), &root).unwrap();
    assert_eq!(
        snapshot(&third.previous.unwrap())["a.lua"],
        b"two, longer",
        "one build is kept"
    );
}

#[test]
fn a_build_that_fails_midway_leaves_the_previous_output_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(
        dir.path(),
        &[
            ("a.lua", b"a"),
            ("b.lua", b"b"),
            ("c.lua", b"c"),
            ("d.lua", b"d"),
        ],
    );
    let root = dir.path().join("out");
    let good = build(&planned(&project), &root).unwrap();
    let before = snapshot(&good.folder);

    // Edit the project, plan it, and lose a file before the build reaches it: `a.lua` and
    // `b.lua` are written by then, and `c.lua` is not there to copy.
    write(&project.folder().join("a.lua"), b"edited");
    let plan = planned(&project);
    std::fs::remove_file(project.folder().join("c.lua")).unwrap();
    let failed = build(&plan, &root).expect_err("a file went missing");

    assert!(matches!(failed, BuildError::Io { .. }), "{failed}");
    assert!(failed.to_string().contains("c.lua"), "{failed}");
    assert_eq!(
        snapshot(&root.join("pack")),
        before,
        "the last good build changed"
    );
    // The build, the hidden file that says OpenRage built it and the one that records its items,
    // and no staging folder.
    let left = names(&root);
    assert_eq!(
        left,
        [".pack.openrage-build", ".pack.openrage-items.json", "pack"],
        "the failed build left a folder behind: {left:?}"
    );
}

#[test]
fn a_source_that_changed_since_the_plan_stops_the_build() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"short")]);
    let plan = planned(&project);
    write(&project.folder().join("a.lua"), b"much longer than it was");

    let failed = build(&plan, &dir.path().join("out")).expect_err("the plan is stale");

    assert!(failed.to_string().contains("changed since"), "{failed}");
    assert!(!dir.path().join("out").join("pack").exists());
}

#[test]
fn a_file_that_does_not_reload_through_its_reader_stops_the_build() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(
        dir.path(),
        &[
            ("a.lua", b"fine"),
            ("stream/broken.ytd", b"RSC7 but not really"),
        ],
    );
    let root = dir.path().join("out");

    let failed =
        build(&planned(&project), &root).expect_err("a texture dictionary that is not one");

    let BuildError::Unverified(found) = failed else {
        panic!("expected the verification to refuse it: {failed}")
    };
    assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
    assert!(
        found.problems[0].contains("stream/broken.ytd"),
        "{:?}",
        found.problems
    );
    assert!(
        !root.join("pack").exists(),
        "an unverified build was swapped in"
    );
    assert!(
        !root.join(".pack.new").exists(),
        "the staging folder is gone"
    );
}

#[test]
fn a_file_an_asset_escrow_locks_ships_as_it_came_and_is_named_and_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    // What follows the marker is no texture dictionary: a reader handed it would stop the build, as the test above shows.
    let locked = [
        or_project::resource::ESCROW_MARKER.as_slice(),
        b"but not really",
    ]
    .concat();
    let project = project(
        dir.path(),
        &[("a.lua", b"fine"), ("stream/sealed.ytd", &locked)],
    );
    let root = dir.path().join("out");

    let built = build(&planned(&project), &root).expect("a locked file does not stop a build");

    assert_eq!(built.verification.problems, Vec::<String>::new());
    assert_eq!(built.verification.locked, ["stream/sealed.ytd"]);
    assert!(
        built
            .verification
            .reloaded
            .iter()
            .all(|r| r.path != "stream/sealed.ytd"),
        "no reader opened it"
    );
    assert_eq!(
        std::fs::read(root.join("pack/stream/sealed.ytd")).unwrap(),
        locked,
        "it ships byte for byte"
    );
    let read_back = verify(&planned(&project), &root.join("pack"));
    assert_eq!(read_back.locked, ["stream/sealed.ytd"]);
    assert!(read_back.problems.is_empty(), "{:?}", read_back.problems);
}

/// ⚠ What reads a built folder agrees with the build on a locked file: the check of a resource names it by the same marker and opens
/// it no more than the build did, and the acceptance sheet counts it under its kind though no reader saw what it holds.
#[test]
fn a_built_folder_with_a_locked_file_is_read_as_the_build_read_it() {
    let dir = tempfile::tempdir().unwrap();
    let locked = [
        or_project::resource::ESCROW_MARKER.as_slice(),
        b"but not really",
    ]
    .concat();
    let project = project(
        dir.path(),
        &[("a.lua", b"fine"), ("stream/sealed.ytd", &locked)],
    );
    let root = dir.path().join("out");
    let built = build(&planned(&project), &root).expect("a locked file does not stop a build");
    assert_eq!(built.verification.locked, ["stream/sealed.ytd"]);

    let report = or_project::resource::check(&built.folder, &Default::default());
    assert_eq!((report.streamed, report.opened, report.locked), (1, 0, 1));
    let about: Vec<&str> = (report.findings.iter())
        .filter(|f| f.file.as_deref() == Some("stream/sealed.ytd"))
        .map(|f| f.code)
        .collect();
    assert_eq!(about, ["file-locked"]);
    assert_eq!(report.count(or_project::content::rule::Severity::Error), 0);

    let kinds = or_project::acceptance::kinds_in(&built.folder).expect("a folder a build wrote");
    assert_eq!(kinds.get("ytd"), Some(&1), "{kinds:?}");
}

#[test]
fn a_plan_with_problems_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"a")]);
    std::fs::remove_file(project.folder().join("a.lua")).unwrap();
    let plan = planned(&project);
    let root = dir.path().join("out");

    let refused = build(&plan, &root).expect_err("a.lua is missing");

    assert!(matches!(refused, BuildError::Refused(_)), "{refused}");
    assert!(!root.exists(), "a refused plan wrote something");
}

#[test]
fn verifying_a_folder_finds_what_a_build_left_stale() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"a"), ("b.lua", b"b")]);
    let root = dir.path().join("out");
    let built = build(&planned(&project), &root).unwrap();
    assert!(verify(&planned(&project), &built.folder)
        .problems
        .is_empty());

    // The project moves on, and the folder gains a file and loses one.
    write(&project.folder().join("a.lua"), b"changed after the build");
    std::fs::remove_file(built.folder.join("b.lua")).unwrap();
    write(&built.folder.join("stray.lua"), b"nobody planned this");

    let found = verify(&planned(&project), &built.folder)
        .problems
        .join("\n");
    assert!(
        found.contains("a.lua") && found.contains("differs"),
        "{found}"
    );
    assert!(
        found.contains("b.lua") && found.contains("missing"),
        "{found}"
    );
    assert!(
        found.contains("stray.lua") && found.contains("not in the plan"),
        "{found}"
    );
}

#[test]
fn a_resource_of_real_files_reloads_through_the_readers() {
    use or_test_support::{shared_corpus_root, Corpus};
    let root = shared_corpus_root(env!("CARGO_MANIFEST_DIR"));
    let mut real = Vec::new();
    for ext in ["ytd", "ydr", "ydd"] {
        let found = Corpus::with_root(&root, ext)
            .limit(1)
            .find_or_skip(&format!("or-project reloads a .{ext}"));
        if let Some(mut found) = found {
            real.push((ext, found.remove(0)));
        }
    }
    if real.is_empty() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<(String, Vec<u8>)> = real
        .iter()
        .map(|(ext, path)| (format!("stream/real.{ext}"), std::fs::read(path).unwrap()))
        .collect();
    let borrowed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let project = project(dir.path(), &borrowed);

    let built = build(&planned(&project), &dir.path().join("out")).expect("real files build");

    assert!(
        built.verification.problems.is_empty(),
        "{:?}",
        built.verification.problems
    );
    let reloaded: Vec<&str> = built
        .verification
        .reloaded
        .iter()
        .map(|r| r.path.as_str())
        .collect();
    for (name, _) in &files {
        assert!(
            reloaded.contains(&name.as_str()),
            "{name} was not read: {reloaded:?}"
        );
    }
    assert!(built
        .verification
        .reloaded
        .iter()
        .all(|r| !r.reader.is_empty()));
}

#[test]
fn a_plan_that_names_a_path_outside_its_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"a")]);
    let mut plan = planned(&project);
    plan.files.push(or_project::PlannedFile {
        path: "../escaped.txt".into(),
        origin: or_project::Origin::Document("out of the resource".into()),
    });
    let root = dir.path().join("out");

    let refused = build(&plan, &root).expect_err("a path that climbs out of the resource");

    assert!(matches!(refused, BuildError::Refused(_)), "{refused}");
    assert!(!root.join("escaped.txt").exists() && !dir.path().join("escaped.txt").exists());
    assert!(!root.exists(), "something was written before the refusal");
}

/// The names in `dir`, hidden ones too, in order.
fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// A project in `folder` holding one member, a file that is no member and a file in a folder of its own.
fn project_with_files_no_build_reads(folder: &Path) -> Project {
    let project = project_in(folder, &[("a.lua", b"member")]);
    write(&folder.join("notes.txt"), b"not a member");
    write(&folder.join("raw/source.psd"), b"the artist's file");
    project
}

/// `plan` built twice into `root`, which is how a source was lost: each time refused, the project as it was.
fn refused_and_the_project_stands(plan: &Plan, root: &Path, project: &Project) {
    let before = snapshot(&project.folder());
    for run in 1..=2 {
        let refused = build(plan, root).expect_err(&format!("build {run} was allowed"));
        assert!(matches!(refused, BuildError::Refused(_)), "{refused}");
        assert_eq!(
            snapshot(&project.folder()),
            before,
            "build {run} changed the project"
        );
    }
}

#[test]
fn a_build_never_replaces_the_folder_of_its_own_project() {
    let dir = tempfile::tempdir().unwrap();
    let project = project_with_files_no_build_reads(&dir.path().join("pack"));
    let plan = planned(&project);
    // `project create` names the resource for the project folder, and `--out` names the folder above it.
    assert_eq!(plan.resource, "pack");

    refused_and_the_project_stands(&plan, dir.path(), &project);

    assert_eq!(
        names(dir.path()),
        ["pack"],
        "a refused build left a folder beside the project"
    );
}

#[test]
fn a_build_never_replaces_a_folder_inside_its_project() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = project_with_files_no_build_reads(&dir.path().join("pack"));
    project.container.outputs[0].name = "raw".into();
    let plan = planned(&project);
    assert_eq!(plan.resource, "raw");

    refused_and_the_project_stands(&plan, &project.folder(), &project);

    assert_eq!(
        names(&project.folder()),
        ["a.lua", "notes.txt", "pack.openrage-proj", "raw"],
        "a refused build left a folder in the project"
    );
}

#[test]
fn a_build_never_keeps_its_work_in_a_folder_that_is_the_project() {
    for hidden in [".pack.new", ".pack.previous"] {
        let dir = tempfile::tempdir().unwrap();
        let project = project_with_files_no_build_reads(&dir.path().join(hidden));
        let plan = planned(&project);
        assert_eq!(plan.resource, "pack");

        refused_and_the_project_stands(&plan, dir.path(), &project);

        assert_eq!(names(dir.path()), [hidden], "{hidden}: a folder was left");
    }
}

#[test]
fn a_build_never_replaces_a_folder_that_holds_a_file_it_copies() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"a")]);
    let shared = dir.path().join("shared");
    write(&shared.join("s.lua"), b"lives outside the project");
    // Marked as a build, so that only the file it holds says no.
    write(&dir.path().join(".shared.openrage-build"), b"");
    let mut plan = planned(&project);
    plan.resource = "shared".into();
    plan.files.push(or_project::PlannedFile {
        path: "s.lua".into(),
        origin: or_project::Origin::Copy {
            from: shared.join("s.lua"),
            bytes: 25,
        },
    });
    let before = snapshot(&shared);

    for run in 1..=2 {
        let refused = build(&plan, dir.path()).expect_err(&format!("build {run} was allowed"));
        assert!(matches!(refused, BuildError::Refused(_)), "{refused}");
        assert_eq!(snapshot(&shared), before, "build {run} changed the source");
    }
}

#[test]
fn a_folder_that_openrage_did_not_build_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path(), &[("a.lua", b"a")]);
    let root = dir.path().join("out");
    write(&root.join("pack/mine.txt"), b"a resource written by hand");
    let before = snapshot(&root.join("pack"));

    for run in 1..=2 {
        let refused =
            build(&planned(&project), &root).expect_err(&format!("build {run} was allowed"));
        assert!(matches!(refused, BuildError::Refused(_)), "{refused}");
        assert!(
            refused.to_string().contains("did not build"),
            "it does not say why: {refused}"
        );
        assert_eq!(snapshot(&root.join("pack")), before, "build {run}");
    }
    assert_eq!(
        names(&root),
        ["pack"],
        "a refused build left a folder beside it"
    );
}

#[test]
fn a_build_folder_inside_the_project_is_where_a_build_may_go() {
    let dir = tempfile::tempdir().unwrap();
    let project = project_with_files_no_build_reads(&dir.path().join("pack"));
    let before = snapshot(&project.folder());
    let root = project.folder().join("build");

    let first = build(&planned(&project), &root).expect("the default folder is allowed");
    let second = build(&planned(&project), &root).expect("and again over its own build");

    assert_eq!(
        second.previous.as_deref(),
        Some(root.join(".pack.previous").as_path())
    );
    assert_eq!(snapshot(&second.folder), snapshot(&first.folder));
    let mut after = snapshot(&project.folder());
    after.retain(|path, _| !path.starts_with("build/"));
    assert_eq!(after, before, "a build changed what is not its own");
}

#[test]
fn a_build_refuses_the_project_however_its_folder_is_spelled() {
    let dir = tempfile::tempdir().unwrap();
    let project = project_with_files_no_build_reads(&dir.path().join("pack"));
    let plan = planned(&project);

    // `pack/..` is the folder above the project, so the resource `pack` in it is the project again.
    refused_and_the_project_stands(&plan, &project.folder().join(".."), &project);
}
