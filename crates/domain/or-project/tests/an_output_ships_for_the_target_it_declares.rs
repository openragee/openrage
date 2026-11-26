//! An output that declares a target ships its drawables and texture dictionaries in that layout,
//! builds the same twice, and stops, leaving the last good build, at a file that cannot move.

mod common;

use common::{declare, enhanced_ytd, layout_of, legacy_ytd, planned, snapshot, unretargetable_ytd};
use or_io::Generation;
use or_project::{build, build_all, verify, BuildError};
use serde_json::{json, Value};

fn output(target: Option<&str>) -> Value {
    let mut output =
        json!({ "id": "fivem", "kind": "fivem-resource", "name": "pack", "take": ["files"] });
    if let Some(target) = target {
        output["target"] = json!(target);
    }
    output
}

#[test]
fn an_enhanced_output_ships_a_legacy_dictionary_as_an_enhanced_one_and_the_rest_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("stream/road.ytd", legacy_ytd("road")),
        ("stream/sky.ytd", enhanced_ytd("sky")),
        ("client.lua", b"print('hi')\n".to_vec()),
    ];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output(Some("enhanced")),
    );
    let plans = planned(&project);
    assert!(plans[0].is_buildable(), "{:?}", plans[0].problems);

    let root = dir.path().join("out");
    let (built, stopped) = build_all(&plans, &root);
    assert!(stopped.is_none(), "{stopped:?}");

    let written = snapshot(&built[0].folder);
    assert_eq!(
        layout_of("ytd", &written["stream/road.ytd"]),
        Generation::Gen9,
        "the Legacy dictionary was copied as it came"
    );
    assert_eq!(
        written["stream/sky.ytd"], files[1].1,
        "a dictionary already in the layout is copied exactly"
    );
    assert_eq!(written["client.lua"], files[2].1);
    assert_eq!(built[0].verification.problems, Vec::<String>::new());
    assert!(
        built[0]
            .verification
            .reloaded
            .iter()
            .any(|r| r.path == "stream/road.ytd" && r.reader.contains("Enhanced")),
        "{:?}",
        built[0].verification.reloaded
    );
    assert_eq!(
        verify(&plans[0], &built[0].folder),
        built[0].verification,
        "verifying the live folder says the same"
    );
}

#[test]
fn a_file_an_asset_escrow_locks_ships_as_it_came_whatever_the_target_and_is_named() {
    let dir = tempfile::tempdir().unwrap();
    // No layout can be read off a locked file, so there is none to move it from: what follows the marker is no dictionary.
    let locked = [
        or_project::resource::ESCROW_MARKER.as_slice(),
        b"what the escrow wrote",
    ]
    .concat();
    let files = [
        ("stream/road.ytd", legacy_ytd("road")),
        ("stream/sealed.ytd", locked.clone()),
        ("stream/sealed.ydr", locked.clone()),
    ];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output(Some("enhanced")),
    );
    let plans = planned(&project);
    assert!(plans[0].is_buildable(), "{:?}", plans[0].problems);

    let (built, stopped) = build_all(&plans, &dir.path().join("out"));
    assert!(stopped.is_none(), "{stopped:?}");

    let written = snapshot(&built[0].folder);
    assert_eq!(written["stream/sealed.ytd"], locked);
    assert_eq!(written["stream/sealed.ydr"], locked);
    assert_eq!(
        layout_of("ytd", &written["stream/road.ytd"]),
        Generation::Gen9,
        "the dictionary beside them still moves"
    );
    let found = &built[0].verification;
    assert_eq!(found.problems, Vec::<String>::new());
    assert_eq!(found.locked, ["stream/sealed.ydr", "stream/sealed.ytd"]);
    assert!(
        found.reloaded.iter().all(|r| !r.path.contains("sealed")),
        "no reader opened a locked file: {:?}",
        found.reloaded
    );
    assert_eq!(verify(&plans[0], &built[0].folder), *found);

    // A locked file that is not the one the plan names is a problem, as a copy that differs is.
    std::fs::write(
        built[0].folder.join("stream/sealed.ydr"),
        [&locked[..], b"!"].concat(),
    )
    .unwrap();
    let changed = verify(&plans[0], &built[0].folder);
    assert_eq!(changed.locked, ["stream/sealed.ytd"]);
    assert!(
        changed
            .problems
            .iter()
            .any(|p| p.contains("stream/sealed.ydr") && p.contains("differs")),
        "{:?}",
        changed.problems
    );
}

#[test]
fn a_legacy_output_ships_an_enhanced_dictionary_as_a_legacy_one() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("stream/sky.ytd", enhanced_ytd("sky"))];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output(Some("legacy")),
    );
    let plans = planned(&project);

    let (built, stopped) = build_all(&plans, &dir.path().join("out"));

    assert!(stopped.is_none(), "{stopped:?}");
    let written = snapshot(&built[0].folder);
    assert_eq!(
        layout_of("ytd", &written["stream/sky.ytd"]),
        Generation::Gen8
    );
}

#[test]
fn an_output_with_no_target_ships_every_file_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("stream/road.ytd", legacy_ytd("road")),
        ("stream/sky.ytd", enhanced_ytd("sky")),
    ];
    let project = declare(&dir.path().join("pack"), &files, Value::Null, output(None));

    let (built, stopped) = build_all(&planned(&project), &dir.path().join("out"));

    assert!(stopped.is_none(), "{stopped:?}");
    let written = snapshot(&built[0].folder);
    assert_eq!(written["stream/road.ytd"], files[0].1);
    assert_eq!(written["stream/sky.ytd"], files[1].1);
}

#[test]
fn two_builds_of_an_enhanced_output_are_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("stream/road.ytd", legacy_ytd("road")),
        ("client.lua", b"a".to_vec()),
    ];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output(Some("enhanced")),
    );
    let plans = planned(&project);

    let first = build(&plans[0], &dir.path().join("one")).expect("builds");
    let second = build(&plans[0], &dir.path().join("two")).expect("builds again");

    assert_eq!(snapshot(&first.folder), snapshot(&second.folder));
}

#[test]
fn a_file_that_cannot_move_stops_the_build_and_leaves_the_last_good_one() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("pack");
    let files = [("stream/lights.ytd", unretargetable_ytd("lights"))];
    let root = dir.path().join("out");

    // As authored the file is a valid Legacy dictionary, so the project builds.
    let project = declare(&folder, &files, Value::Null, output(None));
    let good = build(&planned(&project)[0], &root).expect("builds as it is");
    let before = snapshot(&good.folder);

    let project = declare(&folder, &files, Value::Null, output(Some("enhanced")));
    let plans = planned(&project);
    let (built, stopped) = build_all(&plans, &root);

    assert!(built.is_empty());
    let Some(BuildError::Retargeting { file, why }) = stopped else {
        panic!("a build that cannot move a file is refused by name: {stopped:?}")
    };
    assert_eq!(file, "stream/lights.ytd");
    assert!(why.contains("lights"), "{why}");
    assert_eq!(
        snapshot(&root.join("pack")),
        before,
        "the last good build is as it was"
    );
    assert!(
        !root.join(".pack.new").exists(),
        "the staging folder is gone"
    );
}
