//! `world index` and `world at` over an install built by hand.

use crate::world_fixture;

use super::*;

use world_fixture::{
    bound_bytes, cube, manifest_bytes, map_bytes, mount, types_bytes, write_archive, MapSpec,
};

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Where the map whose parent the install lacks stands, far from the two others.
const LOST: [f32; 3] = [-3000.0, -3000.0, 0.0];

/// An install of three maps: one the parent of another and named a bound by a manifest, and one
/// whose parent the install lacks.
fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let town = cube([100.0, 100.0, 0.0], 100.0);
    let shop = MapSpec {
        parent: "m_town",
        content_flags: 9,
        ..MapSpec::around(cube([120.0, 120.0, 0.0], 20.0))
    };
    let lost = MapSpec {
        parent: "m_gone",
        ..MapSpec::around(cube(LOST, 20.0))
    };
    let manifest = manifest_bytes(&[("m_town", &["m_town_0"])], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    write_archive(
        &root,
        "x64a.rpf",
        &[
            (
                "levels/m_town.ymap",
                map_bytes("m_town", &MapSpec::around(town)),
            ),
            ("levels/m_shop.ymap", map_bytes("m_shop", &shop)),
            ("levels/m_lost.ymap", map_bytes("m_lost", &lost)),
            (
                "levels/m_town_0.ybn",
                bound_bytes(cube([110.0, 110.0, 0.0], 50.0)),
            ),
            (
                "levels/m_shop_0.ybn",
                bound_bytes(cube([900.0, 900.0, 0.0], 5.0)),
            ),
            (
                "types/props.ytyp",
                types_bytes("props", &[("prop_a", cube([-1.0; 3], 2.0), 80.0)]),
            ),
        ],
    );
    (scratch, root)
}

#[test]
fn the_index_verb_says_what_it_holds_and_what_it_read_again() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");

    let game = mount(&root);
    let (index, report) = indexed(&game, &root, &kept, false);
    let first = index_lines(&index, &report, &root, &kept);
    let said = |lines: &[String], words: &str| lines.iter().any(|line| line.contains(words));
    assert!(
        said(&first, "none was read, so every archive was"),
        "{first:#?}"
    );
    assert!(
        said(&first, "2 archive(s), 2 read, 7 file(s) opened"),
        "{first:#?}"
    );
    assert!(said(&first, "    read common.rpf"), "{first:#?}");
    assert!(said(&first, "    read x64a.rpf"), "{first:#?}");
    assert!(
        said(&first, "3 map(s), 0 that did not read as one; 1 name a parent the install holds, 1 one it lacks"),
        "{first:#?}"
    );
    assert!(
        said(
            &first,
            "1 type file(s), 0 that did not read as one; 1 archetype(s)"
        ),
        "{first:#?}"
    );
    assert!(
        said(&first, "bounds named for 1 map group(s) and 0 interior(s)"),
        "{first:#?}"
    );
    assert!(
        said(&first, "2 bound(s), 0 with no box read; 2 carried"),
        "{first:#?}"
    );

    let game = mount(&root);
    let (index, report) = indexed(&game, &root, &kept, false);
    let second = index_lines(&index, &report, &root, &kept);
    assert!(
        said(&second, "of the install as it stands, so nothing was read"),
        "{second:#?}"
    );
    assert!(
        said(&second, "2 archive(s), 0 read, 0 file(s) opened"),
        "{second:#?}"
    );
    assert!(!said(&second, "    read "), "{second:#?}");
    assert_eq!(
        second[3..],
        first[5..],
        "what the index holds is said the same from a kept one"
    );

    let json = index_json(&index, &report, &root, &kept);
    assert_eq!(json["kept"], "current");
    assert_eq!(json["maps"], 3);
    assert_eq!(json["read"], json!([]));

    // `--fresh` reads every archive whatever is kept, and keeps what it read.
    let whole = std::fs::read(&kept).expect("kept");
    std::fs::write(&kept, b"not what was kept").expect("written");
    let (_, report) = indexed(&mount(&root), &root, &kept, true);
    assert_eq!(report.read, ["common.rpf", "x64a.rpf"]);
    assert!(
        matches!(report.kept, KeptState::Absent),
        "{:?}",
        report.kept
    );
    assert_eq!(std::fs::read(&kept).expect("kept anew"), whole);
    let (_, report) = indexed(&mount(&root), &root, &kept, false);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );
}

/// The lines and the JSON `world index` gives of a build of `root` from `kept`.
fn said_of(root: &Path, kept: &Path) -> (Vec<String>, serde_json::Value) {
    let game = mount(root);
    let (index, report) = indexed(&game, root, kept, false);
    (
        index_lines(&index, &report, root, kept),
        index_json(&index, &report, root, kept),
    )
}

#[test]
fn the_index_verb_says_why_a_kept_index_did_not_serve() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let said = |lines: &[String], words: &str| lines.iter().any(|line| line.contains(words));
    let (_, json) = said_of(&root, &kept);
    assert_eq!(json["kept"], "absent");
    assert_eq!(json["refused"], serde_json::Value::Null);
    assert_eq!(json["notKept"], serde_json::Value::Null);

    // An archive written again, ten seconds on.
    let common = root.join("common.rpf");
    let later = std::fs::metadata(&common)
        .and_then(|meta| meta.modified())
        .expect("a modification time")
        + std::time::Duration::from_secs(10);
    let file = std::fs::File::options()
        .write(true)
        .open(&common)
        .expect("opened");
    file.set_modified(later).expect("timed");
    drop(file);
    let (lines, json) = said_of(&root, &kept);
    assert!(
        said(
            &lines,
            "of the install as it was, so each archive that changed was read again"
        ),
        "{lines:#?}"
    );
    assert!(
        said(&lines, "2 archive(s), 1 read, 1 file(s) opened"),
        "{lines:#?}"
    );
    assert!(said(&lines, "    read common.rpf"), "{lines:#?}");
    assert!(!said(&lines, "    read x64a.rpf"), "{lines:#?}");
    assert_eq!(json["kept"], "changed");
    assert_eq!(json["read"], json!(["common.rpf"]));

    // The same file, asked for another install.
    let (_other_scratch, other) = install();
    let (lines, json) = said_of(&other, &kept);
    assert!(
        said(&lines, "of another install, so every archive was read"),
        "{lines:#?}"
    );
    assert_eq!(json["kept"], "foreign");

    // A file cut short.
    let whole = std::fs::read(&kept).expect("kept");
    std::fs::write(&kept, &whole[..whole.len() / 2]).expect("written");
    let (lines, json) = said_of(&other, &kept);
    let why = or_depends::world_index::KeptError::Damaged.to_string();
    assert!(
        said(
            &lines,
            &format!("refused ({why}), so every archive was read")
        ),
        "{lines:#?}"
    );
    assert_eq!(json["kept"], "refused");
    assert_eq!(json["refused"], why);
    assert_eq!(std::fs::read(&kept).expect("kept anew"), whole);

    // A place no file can be kept in: the index answers all the same, and says it was not kept.
    let blocked = scratch.path().join("a-file");
    std::fs::write(&blocked, b"in the way").expect("written");
    let nowhere = blocked.join("world.index");
    let (lines, json) = said_of(&other, &nowhere);
    assert!(
        said(&lines, "  warning: the index was not kept: "),
        "{lines:#?}"
    );
    assert!(said(&lines, "3 map(s)"), "{lines:#?}");
    assert!(json["notKept"].is_string(), "{json}");
    assert_eq!(json["maps"], 3);
}

#[test]
fn an_install_has_one_kept_index_of_its_own_under_the_configuration_folder() {
    let (a, b) = (Path::new("/games/one"), Path::new("/games/two"));
    let Some(of_a) = default_kept(a, None) else {
        // No configuration directory on this machine: `open` asks for `--kept`.
        return;
    };
    assert_eq!(default_kept(a, None), Some(of_a.clone()));
    assert_ne!(default_kept(b, None), Some(of_a.clone()));
    assert_eq!(of_a.extension().and_then(|e| e.to_str()), Some("index"));
    let folder = of_a.parent().expect("a folder");
    assert_eq!(
        folder.file_name().and_then(|n| n.to_str()),
        Some(KEPT_FOLDER)
    );
    assert_eq!(folder.parent(), or_title::config::config_dir().as_deref());
}

#[test]
fn the_at_verb_answers_a_position_with_its_maps_and_its_bounds() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let (index, _) = indexed(&game, &root, &kept, false);

    let inside = at_lines(&index, Vec3::new(130.0, 130.0, 10.0));
    assert_eq!(
        inside,
        [
            "At (130, 130, 10): 2 map(s) whose streaming extents hold it, 1 bound(s) whose box does",
            "  map    m_shop  platform:/levels/m_shop.ymap  parent m_town  flags 0x0  contentFlags 0x9",
            "  map    m_town  platform:/levels/m_town.ymap  flags 0x0  contentFlags 0x0",
            "  bound  m_town_0  platform:/levels/m_town_0.ybn",
            "           carried by m_town, by a manifest's map group",
        ]
    );
    let far = at_lines(&index, Vec3::new(902.0, 902.0, 2.0));
    assert_eq!(
        far,
        [
            "At (902, 902, 2): 0 map(s) whose streaming extents hold it, 1 bound(s) whose box does",
            "  bound  m_shop_0  platform:/levels/m_shop_0.ybn",
            "           carried by m_shop, by its filename, and it lies outside that map's streaming extents",
        ]
    );
    // A parent the install lacks is said by its number, having no name to say.
    let lost = at_lines(&index, Vec3::from(LOST) + Vec3::splat(10.0));
    let gone = or_io::string_hash(b"m_gone");
    assert_eq!(
        lost[1..],
        [format!(
            "  map    m_lost  platform:/levels/m_lost.ymap  parent {gone:#010X}  flags 0x0  contentFlags 0x0"
        )]
    );
    let nowhere = at_lines(&index, Vec3::new(-4000.0, 0.0, 0.0));
    assert_eq!(nowhere.len(), 1, "{nowhere:#?}");
    assert!(nowhere[0].contains("0 map(s)") && nowhere[0].contains("0 bound(s)"));

    let json = at_json(&index, Vec3::new(130.0, 130.0, 10.0));
    assert_eq!(json["maps"][0]["parent"], "m_town");
    assert_eq!(json["maps"][1]["parent"], serde_json::Value::Null);
    assert_eq!(json["bounds"][0]["carriedBy"][0]["name"], "m_town");
}

#[test]
fn a_position_is_three_numbers_and_a_flags_value_is_not_one_of_them() {
    let at = position_of(&strings(&[
        "--game", "5", "-1200.5", "30", "--kept", "7", "71.25",
    ]));
    assert_eq!(at, Ok(Vec3::new(-1200.5, 30.0, 71.25)));
    assert!(position_of(&strings(&["--game", "g", "1", "2"])).is_err());
    assert!(position_of(&strings(&["1", "2", "3", "4"])).is_err());
    for word in ["two", "NaN", "inf", "-infinity"] {
        assert_eq!(
            position_of(&strings(&["1", word, "3"])),
            Err(format!("`{word}` is not a number"))
        );
    }
    assert_eq!(
        position_of(&strings(&["--json", "1", "2", "--fresh", "3"])),
        Ok(Vec3::new(1.0, 2.0, 3.0)),
        "a flag with no value is passed over"
    );
    assert_eq!(
        cmd_at(&strings(&["--game", "g", "1", "2"])),
        ExitCode::from(2)
    );
    assert_eq!(cmd_index(&strings(&[])), ExitCode::from(2));

    // One argument is a position as a script or `world new-map --at` writes one, read by the reader of a pasted place.
    for written in [
        "100,200,30",
        "vector3(100.0, 200.0, 30.0)",
        "{ x = 100, y = 200, z = 30 }",
    ] {
        assert_eq!(
            position_of(&strings(&["--game", "g", written, "--json"])),
            Ok(Vec3::new(100.0, 200.0, 30.0)),
            "{written}"
        );
    }
    assert!(position_of(&strings(&["100,200"])).is_err(), "no height");
    assert!(position_of(&strings(&["prop_bench_01a"])).is_err());

    // A place pasted with no quotes reaches the verb as the words a shell split it into, a flag among them or not.
    for pasted in [
        vec!["-75.0,", "-818.2,", "326.1"],
        vec!["--game", "g", "-75.0,", "-818.2,", "--json", "326.1"],
        vec!["X:", "-75.0,", "Y:", "-818.2,", "Z:", "326.1"],
        vec!["vector3(-75.0,", "-818.2,", "326.1)"],
    ] {
        assert_eq!(
            position_of(&strings(&pasted)),
            Ok(Vec3::new(-75.0, -818.2, 326.1)),
            "{pasted:?}"
        );
    }
    assert_eq!(
        position_of(&strings(&["-75.0,", "there", "326.1"])),
        Err("`-75.0,` is not a number".to_string())
    );
}

#[test]
fn the_verbs_run_from_their_arguments_and_keep_the_index_where_told() {
    let (scratch, root) = install();
    let kept = scratch.path().join("kept").join("world.index");
    let (root, kept_arg) = (root.to_string_lossy(), kept.to_string_lossy());
    let index = strings(&["index", "--game", &root, "--kept", &kept_arg]);
    assert_eq!(super::super::world::run(&index), ExitCode::SUCCESS);
    assert!(kept.is_file(), "the index is kept where `--kept` says");
    // A file written again is a new file with a new time, whatever its bytes.
    let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let file = std::fs::File::options()
        .write(true)
        .open(&kept)
        .expect("opened");
    file.set_modified(earlier).expect("timed");
    drop(file);
    let modified = || {
        std::fs::metadata(&kept)
            .and_then(|meta| meta.modified())
            .expect("kept")
    };
    let stood = modified();

    let at = strings(&[
        "at", "--game", &root, "--kept", &kept_arg, "130", "130", "10", "--json",
    ]);
    assert_eq!(super::super::world::run(&at), ExitCode::SUCCESS);
    assert_eq!(
        modified(),
        stood,
        "asking a position of an install that stands writes nothing"
    );

    // The same install spelled another way is the same install to the kept index.
    let spelled = format!("{root}/../install");
    let index = strings(&["--game", &spelled, "--kept", &kept_arg]);
    assert_eq!(cmd_index(&index), ExitCode::SUCCESS);
    assert_eq!(
        modified(),
        stood,
        "an install named by another path was read as another install"
    );

    let nowhere = strings(&["--game", "no/such/install", "--kept", &kept_arg]);
    assert_eq!(cmd_index(&nowhere), ExitCode::from(2));
}
