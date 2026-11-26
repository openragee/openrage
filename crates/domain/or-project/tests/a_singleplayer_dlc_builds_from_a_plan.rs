//! An output of the kind singleplayer-dlc writes the plan as a `dlc.rpf` that reads back as the
//! archive the plan described, registers each file with a type someone stated, and packs a path
//! through an archive's name as that nested archive.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{declare, enhanced_ytd, layout_of, legacy_ytd, planned, snapshot};
use or_io::container::rpf::RpfArchive;
use or_io::Generation;
use or_project::{
    build, build_all, plan_with, verify, ArchiveContents, Container, Contribution, ManifestEntry,
    Module, ModuleKind, Origin, Output, OutputKind, PlannedFile,
};
use or_title::dlc::{ContentXml, SetupData};
use serde_json::{json, Value};

const HANDLING: &[u8] = b"<?xml version=\"1.0\"?>\n<CHandlingDataMgr/>\n";

fn output(name: &str) -> Value {
    json!({ "id": "dlc", "kind": "singleplayer-dlc", "name": name, "take": ["files"] })
}

fn open(path: &Path) -> RpfArchive {
    RpfArchive::open(path).expect("the pack is an archive")
}

fn open_bytes(bytes: Vec<u8>) -> RpfArchive {
    RpfArchive::from_bytes(bytes).expect("the nested archive opens")
}

/// Every file of the archive by its `/` path, with the bytes it extracts to.
fn contents(archive: &RpfArchive) -> BTreeMap<String, Vec<u8>> {
    archive
        .nodes
        .iter()
        .filter(|node| !node.is_dir())
        .map(|node| {
            let bytes = archive.read_entry(node.entry_index());
            let path = node.path().replace('\\', "/");
            (
                path.trim_start_matches('/').to_string(),
                bytes.expect("it extracts"),
            )
        })
        .collect()
}

fn typed() -> Value {
    json!({ "data/handling.meta": "HANDLING_FILE" })
}

#[test]
fn a_dlc_output_writes_a_pack_that_reads_back_and_registers_each_file_with_its_type() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("data/handling.meta", HANDLING.to_vec()),
        ("data/props.ityp", vec![1, 2, 3, 4]),
    ];
    let project = declare(&dir.path().join("pack"), &files, typed(), output("mypack"));
    let plans = planned(&project);
    assert!(plans[0].is_buildable(), "{:?}", plans[0].problems);
    assert_eq!(plans[0].dlclist_entry.as_deref(), Some("dlcpacks:/mypack/"));
    let planned_paths: Vec<&str> = plans[0].files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        planned_paths,
        [
            "content.xml",
            "data/handling.meta",
            "data/props.ityp",
            "setup2.xml"
        ]
    );

    let root = dir.path().join("out");
    let (built, stopped) = build_all(&plans, &root);
    assert!(stopped.is_none(), "{stopped:?}");

    assert_eq!(
        snapshot(&built[0].folder).keys().collect::<Vec<_>>(),
        ["dlc.rpf"],
        "the folder holds the archive and nothing to copy into an install by mistake"
    );
    let archive = open(&built[0].folder.join("dlc.rpf"));
    let held = contents(&archive);
    assert_eq!(held["data/handling.meta"], HANDLING);
    assert_eq!(held["data/props.ityp"], [1, 2, 3, 4]);

    let setup = SetupData::parse(&held["setup2.xml"]).expect("setup2.xml parses");
    assert_eq!(setup.name_hash, "mypack");
    let content = ContentXml::parse(&held["content.xml"]).expect("content.xml parses");
    let registered: BTreeMap<String, String> = content
        .data_files
        .iter()
        .map(|d| (d.filename.clone(), d.file_type.clone()))
        .collect();
    let device = setup.content_device();
    assert_eq!(
        registered,
        BTreeMap::from([
            (
                format!("{device}:/data/handling.meta"),
                "HANDLING_FILE".to_string()
            ),
            (
                format!("{device}:/data/props.ityp"),
                "DLC_ITYP_REQUEST".to_string()
            ),
        ])
    );
    let enabled = content.files_to_enable();
    assert!(
        content
            .data_files
            .iter()
            .all(|d| enabled.iter().any(|e| e.value == d.filename)),
        "a registered file no change set enables never loads"
    );

    assert_eq!(built[0].verification.problems, Vec::<String>::new());
    assert_eq!(verify(&plans[0], &built[0].folder), built[0].verification);
}

#[test]
fn two_builds_of_a_dlc_output_are_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("data/handling.meta", HANDLING.to_vec())];
    let project = declare(&dir.path().join("pack"), &files, typed(), output("mypack"));
    let plans = planned(&project);

    let one = build(&plans[0], &dir.path().join("one")).expect("builds");
    let two = build(&plans[0], &dir.path().join("two")).expect("builds again");

    assert_eq!(snapshot(&one.folder), snapshot(&two.folder));
}

#[test]
fn a_file_whose_extension_settles_no_type_is_refused_at_planning_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("data/handling.meta", HANDLING.to_vec())];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output("mypack"),
    );

    let plans = planned(&project);

    assert!(!plans[0].is_buildable());
    assert!(
        plans[0]
            .problems
            .iter()
            .any(|p| p.contains("data/handling.meta") && p.contains("File types")),
        "{:?}",
        plans[0].problems
    );
}

#[test]
fn a_type_stated_for_a_file_the_project_does_not_hold_is_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("data/handling.meta", HANDLING.to_vec())];
    let types = json!({ "data/handling.meta": "HANDLING_FILE", "data/gone.meta": "HANDLING_FILE" });
    let project = declare(&dir.path().join("pack"), &files, types, output("mypack"));

    let plans = planned(&project);

    assert!(
        plans[0]
            .problems
            .iter()
            .any(|p| p.contains("data/gone.meta")),
        "{:?}",
        plans[0].problems
    );
}

#[test]
fn a_name_that_is_not_a_device_name_is_refused_at_planning() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("data/props.ityp", vec![0])];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output("my pack"),
    );

    let plans = planned(&project);

    assert!(
        plans[0].problems.iter().any(|p| p.contains("my pack")),
        "{:?}",
        plans[0].problems
    );
}

#[test]
fn a_script_cannot_ship_in_a_dlc() {
    let mut container = Container::default();
    container.outputs.push(Output::of(
        OutputKind::SingleplayerDlc,
        "dlc",
        "mypack",
        &["files"],
    ));
    let mut contribution = Contribution::of("files");
    contribution.manifest.push(ManifestEntry {
        key: "client_script".into(),
        values: vec!["client.lua".into()],
    });

    let plan = plan_with(
        &container,
        "dlc",
        Path::new("/projects/pack"),
        &[contribution],
    )
    .expect("the output is declared");

    assert!(
        plan.problems.iter().any(|p| p.contains("client_script")),
        "{:?}",
        plan.problems
    );
}

#[test]
fn a_path_through_an_archives_name_is_packed_as_that_nested_archive() {
    let dir = tempfile::tempdir().unwrap();
    let ytd = legacy_ytd("comp");
    let files = [("x64/models/cdimages/peds.rpf/comp.ytd", ytd.clone())];
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

    let held = contents(&open(&built[0].folder.join("dlc.rpf")));
    let content = ContentXml::parse(&held["content.xml"]).expect("content.xml parses");
    let registered: Vec<(&str, &str)> = content
        .data_files
        .iter()
        .map(|d| (d.filename.as_str(), d.file_type.as_str()))
        .collect();
    assert_eq!(
        registered.len(),
        1,
        "the archive is registered and not what is inside it"
    );
    assert!(
        registered[0].0.ends_with(":/x64/models/cdimages/peds.rpf"),
        "{registered:?}"
    );
    assert_eq!(registered[0].1, "RPF_FILE");

    let inner = open_bytes(held["x64/models/cdimages/peds.rpf"].clone());
    assert_eq!(contents(&inner)["comp.ytd"], ytd);
    assert!(
        built[0]
            .verification
            .reloaded
            .iter()
            .any(|r| r.path == "x64/models/cdimages/peds.rpf/comp.ytd"),
        "what is inside the nested archive reloads too: {:?}",
        built[0].verification.reloaded
    );
}

#[test]
fn an_enhanced_dlc_packs_the_converted_file() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("x64/a.rpf/road.ytd", legacy_ytd("road")),
        ("x64/a.rpf/sky.ytd", enhanced_ytd("sky")),
    ];
    let mut declared = output("mypack");
    declared["target"] = json!("enhanced");
    let project = declare(&dir.path().join("pack"), &files, Value::Null, declared);

    let (built, stopped) = build_all(&planned(&project), &dir.path().join("out"));

    assert!(stopped.is_none(), "{stopped:?}");
    let held = contents(&open(&built[0].folder.join("dlc.rpf")));
    let inner = contents(&open_bytes(held["x64/a.rpf"].clone()));
    assert_eq!(layout_of("ytd", &inner["road.ytd"]), Generation::Gen9);
    assert_eq!(inner["sky.ytd"], files[1].1);
}

#[test]
fn verification_notices_a_pack_that_is_not_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("data/props.ityp", vec![1, 2, 3])];
    let project = declare(
        &dir.path().join("pack"),
        &files,
        Value::Null,
        output("mypack"),
    );
    let plans = planned(&project);
    let built = build(&plans[0], &dir.path().join("out")).expect("builds");
    let other = declare(
        &dir.path().join("other"),
        &[("data/props.ityp", vec![9, 9, 9])],
        Value::Null,
        output("mypack"),
    );
    let swapped = build(&planned(&other)[0], &dir.path().join("swapped")).expect("builds");

    std::fs::copy(swapped.folder.join("dlc.rpf"), built.folder.join("dlc.rpf")).unwrap();
    let found = verify(&plans[0], &built.folder);

    assert!(
        found.problems.iter().any(|p| p.contains("data/props.ityp")),
        "{:?}",
        found.problems
    );
    std::fs::write(built.folder.join("dlc.rpf"), b"not an archive").unwrap();
    assert!(!verify(&plans[0], &built.folder).problems.is_empty());
}

#[test]
fn the_types_a_project_states_are_lines_of_a_fivem_manifest_too() {
    let dir = tempfile::tempdir().unwrap();
    let files = [("data/handling.meta", HANDLING.to_vec())];
    let fivem =
        json!({ "id": "fivem", "kind": "fivem-resource", "name": "pack", "take": ["files"] });
    let project = declare(&dir.path().join("pack"), &files, typed(), fivem);

    let built = build(&planned(&project)[0], &dir.path().join("out")).expect("builds");

    let manifest = snapshot(&built.folder).remove("fxmanifest.lua").unwrap();
    let manifest = String::from_utf8(manifest).unwrap();
    assert!(
        manifest.contains("data_file 'HANDLING_FILE' 'data/handling.meta'\n"),
        "{manifest}"
    );
}

#[test]
fn a_document_a_content_writes_ships_in_the_pack_beside_the_files_it_copies() {
    let dir = tempfile::tempdir().unwrap();
    let mut container = Container::default();
    container.outputs.push(Output::of(
        OutputKind::SingleplayerDlc,
        "dlc",
        "mypack",
        &["files"],
    ));
    let mut contribution = Contribution::of("files");
    for path in [
        "data/generated.meta",
        "x64/models/cdimages/peds.rpf/ped.meta",
    ] {
        contribution.files.push(PlannedFile {
            path: path.to_string(),
            origin: Origin::Document(String::from_utf8(HANDLING.to_vec()).unwrap()),
        });
    }
    contribution.manifest.push(ManifestEntry {
        key: "data_file".into(),
        values: vec!["HANDLING_FILE".into(), "data/generated.meta".into()],
    });
    let plan = plan_with(
        &container,
        "dlc",
        Path::new("/projects/pack"),
        &[contribution],
    )
    .expect("the output is declared");
    assert!(plan.is_buildable(), "{:?}", plan.problems);

    let built = build(&plan, &dir.path().join("out")).expect("a document ships like a file");

    assert_eq!(built.verification.problems, Vec::<String>::new());
    let held = contents(&open(&built.folder.join("dlc.rpf")));
    assert_eq!(held["data/generated.meta"], HANDLING);
    let inner = open_bytes(held["x64/models/cdimages/peds.rpf"].clone());
    assert_eq!(contents(&inner)["ped.meta"], HANDLING);
    assert!(
        held.contains_key("setup2.xml") && held.contains_key("content.xml"),
        "the pack still writes its own two documents"
    );
}

/// A content of a kind this crate has no code for, handing a pack one type file inside a nested archive.
fn a_type_file_in_an_archive() -> (Container, Contribution) {
    let container = Container {
        modules: vec![Module {
            id: "props".to_string(),
            kind: ModuleKind::Unknown("widgets".into()),
            extra: Default::default(),
        }],
        outputs: vec![Output::of(
            OutputKind::SingleplayerDlc,
            "dlc",
            "mypack",
            &["props"],
        )],
    };
    let mut contribution = Contribution::of("props");
    contribution.files.push(PlannedFile {
        path: "x64/levels/props.rpf/a.ytyp".to_string(),
        origin: Origin::Generated(vec![1, 2, 3]),
    });
    (container, contribution)
}

fn asking(contribution: &mut Contribution, path: &str) {
    contribution.manifest.push(ManifestEntry {
        key: "data_file".to_string(),
        values: vec!["DLC_ITYP_REQUEST".to_string(), path.to_string()],
    });
}

/// ⚠ What only the content knows of a file, such as what a type file declares, reaches the cell and the plan as one sentence: a
/// cell that said yes over a plan that is refused would offer a build that cannot be made.
#[test]
fn what_a_content_says_locks_the_build_it_is_laid_out_for_locks_the_cell_and_stops_the_plan() {
    let (container, mut contribution) = a_type_file_in_an_archive();
    let dlc = OutputKind::SingleplayerDlc;
    assert_eq!(or_project::cannot_ship(&contribution, &dlc), None);
    let clear = plan_with(&container, "dlc", Path::new("/p"), &[contribution.clone()]).unwrap();
    assert_eq!(clear.problems, Vec::<String>::new());

    let lock = "The type file a.ytyp cannot ship in a singleplayer DLC.".to_string();
    contribution.locks.push(lock.clone());
    assert_eq!(
        or_project::cannot_ship(&contribution, &dlc).as_deref(),
        Some(lock.as_str())
    );
    let plan = plan_with(&container, "dlc", Path::new("/p"), &[contribution]).unwrap();
    assert_eq!(plan.problems, [lock]);
}

#[test]
fn a_type_file_in_a_nested_archive_is_asked_for_by_the_name_of_its_folder_and_by_no_other() {
    let (container, mut contribution) = a_type_file_in_an_archive();
    asking(&mut contribution, "x64/levels/a.ityp");
    // As the packs of the game state it: props of the name the type file is asked for by, and nothing of its archive.
    contribution.archives.push(ArchiveContents {
        path: "x64/levels/a.ityp".to_string(),
        contents: "CONTENTS_PROPS".to_string(),
    });
    let plan = plan_with(&container, "dlc", Path::new("/p"), &[contribution]).unwrap();
    assert_eq!(
        plan.problems,
        Vec::<String>::new(),
        "a type file asked for from an archive ships in a singleplayer DLC"
    );
    let registered: Vec<(&str, &str, &str, bool)> = (plan.registrations.iter())
        .map(|r| {
            (
                r.path.as_str(),
                r.file_type.as_str(),
                r.contents.as_str(),
                r.asked,
            )
        })
        .collect();
    assert_eq!(
        registered,
        [
            (
                "x64/levels/a.ityp",
                "DLC_ITYP_REQUEST",
                "CONTENTS_PROPS",
                true
            ),
            ("x64/levels/props.rpf", "RPF_FILE", "", false),
        ]
    );
    assert!(
        plan.files.iter().all(|f| f.path != "x64/levels/a.ityp"),
        "the name is registered and is no file of the pack"
    );
    assert!(
        !plan.problems.iter().any(|p| p.contains("does not hold")),
        "{:?}",
        plan.problems
    );

    // A name with no type file behind it, in that folder or in another, is a type stated for a file the build lacks.
    for stray in ["x64/levels/b.ityp", "x64/a.ityp"] {
        let (container, mut contribution) = a_type_file_in_an_archive();
        asking(&mut contribution, stray);
        let plan = plan_with(&container, "dlc", Path::new("/p"), &[contribution]).unwrap();
        assert!(
            plan.problems
                .iter()
                .any(|p| p.contains(stray) && p.contains("does not hold")),
            "{stray}: {:?}",
            plan.problems
        );
        assert!(plan.registrations.iter().all(|r| !r.asked));
    }
}

#[test]
fn an_archive_said_to_hold_something_is_one_the_plan_puts_a_file_in() {
    let (container, mut contribution) = a_type_file_in_an_archive();
    contribution.archives.push(ArchiveContents {
        path: "x64/levels/elsewhere.rpf".to_string(),
        contents: "CONTENTS_PROPS".to_string(),
    });
    let plan = plan_with(&container, "dlc", Path::new("/p"), &[contribution]).unwrap();
    assert!(
        plan.problems
            .iter()
            .any(|p| p.contains("x64/levels/elsewhere.rpf") && p.contains("no file")),
        "{:?}",
        plan.problems
    );
    assert!(plan.registrations.iter().all(|r| r.contents.is_empty()));
}

#[test]
fn two_contents_that_say_one_archive_holds_different_things_are_named() {
    let said = |first: &str, second: &str| {
        let (mut container, props) = a_type_file_in_an_archive();
        container.modules.push(Module {
            id: "more".to_string(),
            kind: ModuleKind::Unknown("widgets".into()),
            extra: Default::default(),
        });
        container.outputs[0].take.push("more".to_string());
        let mut more = Contribution::of("more");
        more.files.push(PlannedFile {
            path: "x64/levels/props.rpf/b.ytyp".to_string(),
            origin: Origin::Generated(vec![4, 5, 6]),
        });
        let mut handed = [props, more];
        for (contribution, contents) in handed.iter_mut().zip([first, second]) {
            contribution.archives.push(ArchiveContents {
                path: "x64/levels/props.rpf".to_string(),
                contents: contents.to_string(),
            });
        }
        let plan = plan_with(&container, "dlc", Path::new("/p"), &handed).unwrap();
        let contents: Vec<String> = (plan.registrations.iter())
            .filter(|r| r.path == "x64/levels/props.rpf")
            .map(|r| r.contents.clone())
            .collect();
        let named: Vec<String> = (plan.problems.iter())
            .filter(|p| p.contains("`props`") && p.contains("`more`"))
            .cloned()
            .collect();
        (contents, named)
    };

    let (contents, named) = said("CONTENTS_PROPS", "CONTENTS_PROPS");
    assert_eq!(contents, ["CONTENTS_PROPS"]);
    assert_eq!(named, Vec::<String>::new(), "they agree");

    let (contents, named) = said("CONTENTS_PROPS", "CONTENTS_MAP");
    assert_eq!(contents, ["CONTENTS_PROPS"]);
    let [problem] = named.as_slice() else {
        panic!("one problem names both contents: {named:?}")
    };
    assert!(
        problem.contains("CONTENTS_PROPS") && problem.contains("CONTENTS_MAP"),
        "{problem}"
    );
}

#[test]
fn a_map_file_that_would_sit_loose_is_refused_once_and_the_line_that_declares_a_map_is_passed_by() {
    let (container, mut contribution) = a_type_file_in_an_archive();
    contribution.files = vec![PlannedFile {
        path: "stream/harbour.ymap".to_string(),
        origin: Origin::Generated(vec![1, 2, 3]),
    }];
    // What a content laid out for a FiveM resource hands over.
    contribution.manifest.push(ManifestEntry {
        key: "this_is_a_map".to_string(),
        values: vec!["yes".to_string()],
    });
    let plan = plan_with(&container, "dlc", Path::new("/p"), &[contribution]).unwrap();
    let of_the_map: Vec<&String> = (plan.problems.iter())
        .filter(|p| p.contains("harbour.ymap"))
        .collect();
    let [problem] = of_the_map.as_slice() else {
        panic!("one sentence names the map file: {:?}", plan.problems)
    };
    assert!(problem.contains("sit loose"), "{problem}");
    assert!(
        !plan.problems.iter().any(|p| p.contains("this_is_a_map")),
        "{:?}",
        plan.problems
    );
}
