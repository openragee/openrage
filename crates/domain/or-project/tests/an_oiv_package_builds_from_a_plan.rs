//! An output of the kind oiv-package writes the plan as a zip whose manifest names every file it
//! holds, that reads back as the plan, and that is the same twice.

mod common;

use std::io::BufReader;
use std::path::Path;

use common::{declare, enhanced_ytd, layout_of, legacy_ytd, planned, snapshot};
use or_io::Generation;
use or_project::{build, build_all, verify};
use or_title::oiv::{Add, PackageReader};
use serde_json::{json, Value};

fn output(name: &str) -> Value {
    json!({
        "id": "oiv", "kind": "oiv-package", "name": name, "take": ["files"],
        "author": "Someone", "version": "2.1", "description": "Two things"
    })
}

fn open(path: &Path) -> PackageReader<BufReader<std::fs::File>> {
    let file = std::fs::File::open(path).expect("the package is a file");
    PackageReader::new(BufReader::new(file)).expect("the package reads")
}

fn files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("readme.txt", b"read me".to_vec()),
        ("x64e.rpf/levels/gta5/a.ytd", legacy_ytd("road")),
        (
            "update/x64/dlcpacks/mypack/dlc.rpf",
            vec![0x52, 0x50, 0x46, 0x37, 1, 2, 3],
        ),
    ]
}

#[test]
fn an_oiv_output_writes_a_package_whose_manifest_names_every_file_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let files = files();
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output("mypack"),
    );
    let plans = planned(&project);
    assert!(plans[0].is_buildable(), "{:?}", plans[0].problems);

    let (built, stopped) = build_all(&plans, &dir.path().join("out"));
    assert!(stopped.is_none(), "{stopped:?}");

    assert_eq!(
        snapshot(&built[0].folder).keys().collect::<Vec<_>>(),
        ["mypack.oiv"]
    );
    let mut package = open(&built[0].folder.join("mypack.oiv"));
    assert_eq!(package.package.name, "mypack");
    assert_eq!(package.package.author, "Someone");
    assert_eq!(package.package.description, "Two things");
    assert_eq!((package.package.major, package.package.minor), (2, 1));
    let adds: Vec<&Add> = package.package.adds.iter().collect();
    assert_eq!(adds.len(), 3);
    let by_source = |source: &str| *adds.iter().find(|a| a.source == source).expect(source);
    assert_eq!(by_source("readme.txt").archives, Vec::<String>::new());
    let inside = by_source("x64e.rpf/levels/gta5/a.ytd");
    assert_eq!(inside.archives, ["x64e.rpf"]);
    assert_eq!(inside.destination, "levels/gta5/a.ytd");
    let whole = by_source("update/x64/dlcpacks/mypack/dlc.rpf");
    assert_eq!(whole.archives, Vec::<String>::new());
    assert_eq!(whole.destination, "update/x64/dlcpacks/mypack/dlc.rpf");

    for (path, body) in &files {
        assert_eq!(&package.read(path).unwrap(), body, "{path}");
    }
    assert_eq!(built[0].verification.problems, Vec::<String>::new());
    assert!(
        built[0]
            .verification
            .reloaded
            .iter()
            .any(|r| r.path == "x64e.rpf/levels/gta5/a.ytd"),
        "the dictionary in the package reloads: {:?}",
        built[0].verification.reloaded
    );
    assert_eq!(verify(&plans[0], &built[0].folder), built[0].verification);
}

#[test]
fn two_builds_of_an_oiv_output_are_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let project = declare(
        &dir.path().join("pack"),
        &files(),
        Value::Null,
        output("mypack"),
    );
    let plans = planned(&project);

    let one = build(&plans[0], &dir.path().join("one")).expect("builds");
    let two = build(&plans[0], &dir.path().join("two")).expect("builds again");

    assert_eq!(snapshot(&one.folder), snapshot(&two.folder));
}

#[test]
fn an_enhanced_package_holds_the_converted_file() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("x64e.rpf/road.ytd", legacy_ytd("road")),
        ("x64e.rpf/sky.ytd", enhanced_ytd("sky")),
    ];
    let mut declared = output("mypack");
    declared["target"] = json!("enhanced");
    let project = declare(&dir.path().join("pack"), &files, Value::Null, declared);

    let (built, stopped) = build_all(&planned(&project), &dir.path().join("out"));

    assert!(stopped.is_none(), "{stopped:?}");
    let mut package = open(&built[0].folder.join("mypack.oiv"));
    assert_eq!(
        layout_of("ytd", &package.read("x64e.rpf/road.ytd").unwrap()),
        Generation::Gen9
    );
    assert_eq!(package.read("x64e.rpf/sky.ytd").unwrap(), files[1].1);
}

#[test]
fn a_file_named_for_the_manifest_and_a_version_that_is_not_one_are_refused_at_planning() {
    let dir = tempfile::tempdir().unwrap();
    let project = declare(
        &dir.path().join("pack"),
        &[("Assembly.xml", b"<x/>".to_vec())],
        Value::Null,
        output("mypack"),
    );
    let plans = planned(&project);
    assert!(
        plans[0].problems.iter().any(|p| p.contains("assembly.xml")),
        "{:?}",
        plans[0].problems
    );

    let mut declared = output("mypack");
    declared["version"] = json!("two");
    let project = declare(
        &dir.path().join("other"),
        &[("a.txt", b"a".to_vec())],
        Value::Null,
        declared,
    );
    let plans = planned(&project);
    assert!(
        plans[0].problems.iter().any(|p| p.contains("version")),
        "{:?}",
        plans[0].problems
    );
}

#[test]
fn verification_notices_a_package_that_is_not_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let project = declare(
        &dir.path().join("pack"),
        &[("a.txt", b"first".to_vec())],
        Value::Null,
        output("mypack"),
    );
    let plans = planned(&project);
    let built = build(&plans[0], &dir.path().join("out")).expect("builds");
    let other = declare(
        &dir.path().join("other"),
        &[("a.txt", b"second".to_vec())],
        Value::Null,
        output("mypack"),
    );
    let swapped = build(&planned(&other)[0], &dir.path().join("swapped")).expect("builds");

    std::fs::copy(
        swapped.folder.join("mypack.oiv"),
        built.folder.join("mypack.oiv"),
    )
    .unwrap();
    let found = verify(&plans[0], &built.folder);

    assert!(
        found.problems.iter().any(|p| p.contains("a.txt")),
        "{:?}",
        found.problems
    );
    std::fs::write(built.folder.join("mypack.oiv"), b"not a zip").unwrap();
    assert!(!verify(&plans[0], &built.folder).problems.is_empty());
}

/// ⚠ Only a FiveM server of its keyholder opens a file an asset escrow locks, so a package that installs one is stopped by name.
#[test]
fn a_file_an_asset_escrow_locks_stops_a_package_which_the_game_could_not_read_it_from() {
    let dir = tempfile::tempdir().unwrap();
    let locked = [
        or_project::resource::ESCROW_MARKER.as_slice(),
        b"what only its keyholder opens",
    ]
    .concat();
    let files = vec![
        ("readme.txt", b"read me".to_vec()),
        ("x64e.rpf/levels/gta5/sealed.ydr", locked),
    ];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output("mypack"),
    );
    let plans = planned(&project);
    assert!(plans[0].is_buildable(), "{:?}", plans[0].problems);

    let (built, stopped) = build_all(&plans, &dir.path().join("out"));
    assert!(built.is_empty(), "the package is not swapped in");
    let why = format!("{stopped:?}");
    assert!(
        why.contains("x64e.rpf/levels/gta5/sealed.ydr")
            && why.contains("locked by a server's asset escrow"),
        "{why}"
    );
}
