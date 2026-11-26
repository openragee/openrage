//! `world ground` over installs built by hand: what it says of a position, and what a level of
//! DLC and a mode leave out of the bounds it asks.

use crate::world_fixture;

use std::sync::Arc;

use or_io::container::rpf::crypto::{NoOpCryptoProvider, RpfCryptoProvider};
use or_io::gta5::data::materials_palette::{MaterialPaletteEntry, MaterialPaletteSource};
use or_io::gta5::PhBoundGeometry;
use or_title::dlc::{ContentChangeSet, ContentChangeSetGroup};
use or_title::{
    build_mounted_game_with, ContentXml, DataFileEntry, DlcList, GtaInstall, SetupData,
};

use super::*;

use world_fixture::{
    cube, level_square, manifest_bytes, map_bytes, mesh_bound, mesh_bound_bytes, mount,
    sphere_bytes, write_archive, MapSpec,
};

/// Material words whose low byte is a row of [`palette`], the first with a bit above it.
const FLOOR: u64 = 0x1_0000_0002;
const ROOF: u64 = 0x0003;
const HILL: u64 = 0x0004;
/// A type past the last row of the palette, which has no name.
const SHOP: u64 = 0x00c8;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// A table of five materials, named for the test alone.
fn palette() -> MaterialPalette {
    let entries = ["zero", "one", "two", "three", "four"].map(|name| MaterialPaletteEntry {
        name,
        fx_group: "none",
        rgb: [0; 3],
    });
    MaterialPalette::from_runtime(entries.to_vec(), MaterialPaletteSource::NotLoaded)
}

/// A floor a hundred metres square at 10, and a roof over part of it at 14.
fn town() -> PhBoundGeometry {
    mesh_bound(
        &[
            [100.0, 100.0, 10.0],
            [200.0, 100.0, 10.0],
            [200.0, 200.0, 10.0],
            [100.0, 200.0, 10.0],
            [120.0, 120.0, 14.0],
            [140.0, 120.0, 14.0],
            [140.0, 140.0, 14.0],
            [120.0, 140.0, 14.0],
        ],
        &[[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        &[FLOOR, ROOF],
        &[0, 0, 1, 1],
    )
}

fn hill() -> PhBoundGeometry {
    level_square([150.0, 150.0, 12.0], 30.0, HILL)
}

/// A ramp ten metres square that rises a metre for each metre along X, apart from the town.
fn ramp() -> PhBoundGeometry {
    mesh_bound(
        &[
            [300.0, 100.0, 0.0],
            [310.0, 100.0, 10.0],
            [310.0, 110.0, 10.0],
            [300.0, 110.0, 0.0],
        ],
        &[[0, 1, 2], [0, 2, 3]],
        &[HILL],
        &[0, 0],
    )
}

fn stored_z(g: &PhBoundGeometry, vertex: usize) -> f32 {
    g.vertex_position(vertex).expect("a vertex")[2]
}

/// Files of an archive, each by its path in it.
type Files = Vec<(&'static str, Vec<u8>)>;

/// The files of the town: its two maps, its floor, a hill over a corner of it, a file over
/// another corner that reads as no collider, and an interior's floor about its own origin,
/// which a third map, a mall apart from the town, places.
fn town_files() -> Files {
    let town_map = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let hill_map = MapSpec::around(cube([150.0, 150.0, 0.0], 30.0));
    let mall_map = MapSpec {
        interiors: vec!["v_int_shop"],
        ..MapSpec::around(cube([600.0, 600.0, 0.0], 50.0))
    };
    vec![
        ("levels/m_town.ymap", map_bytes("m_town", &town_map)),
        ("levels/m_hill.ymap", map_bytes("m_hill", &hill_map)),
        ("levels/m_mall.ymap", map_bytes("m_mall", &mall_map)),
        ("levels/m_town_0.ybn", mesh_bound_bytes(town())),
        (
            "levels/m_town_2.ybn",
            sphere_bytes(cube([180.0, 180.0, 50.0], 15.0), f32::NAN),
        ),
        ("levels/m_hill_0.ybn", mesh_bound_bytes(hill())),
        ("levels/m_town_3.ybn", mesh_bound_bytes(ramp())),
        (
            "levels/v_int_shop_0.ybn",
            mesh_bound_bytes(level_square([-10.0, -10.0, 0.0], 20.0, SHOP)),
        ),
    ]
}

fn town_manifest() -> Vec<u8> {
    manifest_bytes(
        &[("m_town", &["m_town_0", "m_town_2", "m_town_3"])],
        &[("v_int_shop", &["v_int_shop_0"])],
    )
}

fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let manifest = town_manifest();
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    write_archive(&root, "x64a.rpf", &town_files());
    (scratch, root)
}

/// What `world ground <args>` says of `game`: its lines and its JSON.
fn say(
    game: &MountedGame,
    root: &Path,
    kept: &Path,
    args: &[&str],
) -> (Vec<String>, serde_json::Value) {
    let args = strings(args);
    let asked = asked_of(&args).expect("a position");
    let (_, options) = mount_of(&args).expect("a mount");
    let (lines, json, not_kept) = said(game, root, kept, false, &asked, options.mode, &palette());
    assert!(not_kept.is_none(), "{not_kept:?}");
    (lines, json)
}

/// The path the index names a file of `x64a.rpf` by.
fn logical(leaf: &str) -> String {
    format!("platform:/levels/{leaf}")
}

const LOOKED: &str = "bounds file(s)";

#[test]
fn the_ground_is_said_with_its_height_its_material_and_the_file_it_is_in() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let (floor, roof) = (stored_z(&town(), 0), stored_z(&town(), 4));
    let top = stored_z(&hill(), 0);

    let (lines, json) = say(&game, &root, &kept, &["190", "110"]);
    let town_file = logical("m_town_0.ybn");
    assert_eq!(
        lines,
        [
            format!("Ground under (190, 110), the whole of its column: {floor:.3}"),
            format!("  at (190.000, 110.000, {floor:.3}), normal (0.000, 0.000, 1.000)"),
            format!("  material {FLOOR:#018x}, type 2 (two)"),
            format!("  from m_town_0  {town_file}"),
            format!(
                "  looked in 1 {LOOKED}; 0 did not read; 0 not loaded, which an interior places"
            ),
            format!("    asked     m_town_0  {town_file}"),
        ]
    );
    assert_eq!(json["at"], json!([190.0, 110.0]));
    assert_eq!(json["from"], json!(null));
    assert_eq!(json["rise"], json!(null));
    assert_eq!(json["slope"], json!(SLOPE));
    assert_eq!(json["ground"]["height"], json!(floor));
    assert_eq!(json["ground"]["position"], json!([190.0, 110.0, floor]));
    assert_eq!(
        json["ground"]["material"],
        json!({ "id": format!("{FLOOR:#018x}"), "type": 2, "name": "two" })
    );
    assert_eq!(
        json["ground"]["bound"],
        json!({ "name": "m_town_0", "logical": town_file })
    );
    assert_eq!(json["lookedIn"], json!([json["ground"]["bound"]]));
    assert_eq!(json["unread"], json!([]));
    assert_eq!(json["notLoaded"], json!([]));
    assert_eq!(json["notes"], json!([]));

    // Under the roof with no height, the roof; from between the two, the floor; and with a rise
    // that reaches the roof, the roof again.
    let height = |args: &[&str]| say(&game, &root, &kept, args).1["ground"]["height"].clone();
    assert_eq!(height(&["130", "130"]), json!(roof));
    assert_eq!(height(&["130", "130", "12"]), json!(floor));
    assert_eq!(height(&["130", "130", "12", "--rise", "3"]), json!(roof));
    let (lines, json) = say(&game, &root, &kept, &["--json", "130", "130", "12"]);
    assert_eq!(
        lines[0],
        format!("Ground under (130, 130, 12), from 1 m above it down: {floor:.3}")
    );
    assert_eq!(json["from"], json!(12.0));
    assert_eq!(json["rise"], json!(RISE));

    // Where the hill's file overlaps the town's, the higher surface and its file.
    let (lines, json) = say(&game, &root, &kept, &["160", "160"]);
    assert_eq!(json["ground"]["height"], json!(top));
    assert_eq!(json["ground"]["bound"]["name"], "m_hill_0");
    assert_eq!(json["ground"]["material"]["name"], "four");
    assert_eq!(json["lookedIn"].as_array().map(Vec::len), Some(2));
    assert!(lines.contains(&format!("  from m_hill_0  {}", logical("m_hill_0.ybn"))));
    assert!(lines.contains(&format!("    asked     m_town_0  {town_file}")));
    let under_the_hill = say(&game, &root, &kept, &["160", "160", "11", "--rise", "0.5"]).1;
    assert_eq!(under_the_hill["ground"]["bound"]["name"], "m_town_0");

    // The steepest ground asked for may be nearly level, and level ground is ground still.
    assert_eq!(height(&["190", "110", "--slope", "0.5"]), json!(floor));
    // A ramp at 45 degrees is ground as far as 60 and none as far as 30: half way up it, the
    // height between the two the file holds for its ends.
    let half_way = f64::from(stored_z(&ramp(), 0) + stored_z(&ramp(), 1)) * 0.5;
    let (lines, json) = say(&game, &root, &kept, &["305", "105"]);
    let on_the_ramp = json["ground"]["height"].as_f64().expect("the ramp");
    assert!((on_the_ramp - half_way).abs() < 1e-3, "{on_the_ramp}");
    assert!(
        lines[1].ends_with("normal (-0.707, 0.000, 0.707)"),
        "{}",
        lines[1]
    );
    assert_eq!(json["slope"], json!(60.0));
    let (lines, json) = say(&game, &root, &kept, &["305", "105", "--slope", "30"]);
    assert_eq!(json["ground"], json!(null));
    assert_eq!(json["slope"], json!(30.0));
    assert_eq!(
        lines[0],
        "No ground under (305, 105), the whole of its column"
    );
    assert_eq!(json["lookedIn"][0]["name"], "m_town_3");
}

#[test]
fn no_ground_is_said_with_the_files_looked_in_and_a_file_that_does_not_read_is_named() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let floor = stored_z(&town(), 0);
    let (town_file, broken) = (logical("m_town_0.ybn"), logical("m_town_2.ybn"));

    // A file over this column does not read: it is named, and the floor under it answers.
    let (lines, json) = say(&game, &root, &kept, &["185", "185"]);
    assert_eq!(
        lines[0],
        format!("Ground under (185, 185), the whole of its column: {floor:.3}")
    );
    assert_eq!(
        lines[4..],
        [
            format!(
                "  looked in 1 {LOOKED}; 1 did not read; 0 not loaded, which an interior places"
            ),
            format!("    asked     m_town_0  {town_file}"),
            format!(
                "    unread    m_town_2  {broken}: it is no collider: bound 0: the bound is not a \
                 finite size or place"
            ),
        ]
    );
    assert_eq!(json["unread"][0]["name"], "m_town_2");
    assert_eq!(json["unread"][0]["logical"], json!(broken));
    let why = json["unread"][0]["why"].as_str().expect("a reason");
    assert!(why.starts_with("it is no collider"), "{why}");

    // Under the floor there is none, and the files looked in are said all the same.
    let (lines, json) = say(&game, &root, &kept, &["185", "185", "5"]);
    assert_eq!(
        lines[0],
        "No ground under (185, 185, 5), from 1 m above it down"
    );
    assert_eq!(lines.len(), 4, "{lines:#?}");
    assert_eq!(lines[2], format!("    asked     m_town_0  {town_file}"));
    assert_eq!(json["ground"], json!(null));
    assert_eq!(json["lookedIn"][0]["name"], "m_town_0");

    // An interior's collision is named where the map that places it streams, and not loaded.
    let (lines, json) = say(&game, &root, &kept, &["620", "620"]);
    assert_eq!(
        lines,
        [
            "No ground under (620, 620), the whole of its column".to_string(),
            format!(
                "  looked in 0 {LOOKED}; 0 did not read; 1 not loaded, which an interior places"
            ),
            format!(
                "    unplaced  v_int_shop_0  {}",
                logical("v_int_shop_0.ybn")
            ),
        ]
    );
    assert_eq!(json["notLoaded"][0]["name"], "v_int_shop_0");
    assert_eq!(json["lookedIn"], json!([]));
    // Its file states its box about the interior's own origin, which is no place of the world:
    // at the world's origin nothing is said of it.
    let (lines, json) = say(&game, &root, &kept, &["0", "0"]);
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert_eq!(json["notLoaded"], json!([]));

    // And where no file's box reaches, nothing was looked in.
    let (lines, json) = say(&game, &root, &kept, &["5000", "-5000"]);
    assert_eq!(
        lines,
        [
            "No ground under (5000, -5000), the whole of its column".to_string(),
            format!(
                "  looked in 0 {LOOKED}; 0 did not read; 0 not loaded, which an interior places"
            ),
        ]
    );
    assert_eq!(json["ground"], json!(null));
}

#[test]
fn a_material_the_install_does_not_name_is_said_by_its_word_and_type_alone() {
    let (scratch, root) = install();
    let kept = scratch.path().join("world.index");
    let game = mount(&root);
    let asked = asked_of(&strings(&["190", "110"])).expect("a position");
    let none = MaterialPalette::empty();
    let (lines, json, _) = said(&game, &root, &kept, false, &asked, None, none);
    assert_eq!(lines[2], format!("  material {FLOOR:#018x}, type 2"));
    assert_eq!(json["ground"]["material"]["name"], json!(null));
    assert_eq!(json["ground"]["material"]["type"], 2);
}

#[test]
fn a_position_is_two_numbers_or_three_and_a_flags_value_is_none_of_them() {
    let asked = |args: &[&str]| asked_of(&strings(args));
    let plain = Asked {
        x: -12.5,
        y: 40.0,
        z: None,
        rise: RISE,
        slope: SLOPE,
    };
    assert_eq!(asked(&["--game", "7", "-12.5", "40"]), Ok(plain));
    assert_eq!(
        asked(&["--json", "--fresh", "--game=7", "-12.5", "40"]),
        Ok(plain)
    );
    let every_flag = [
        "-12.5", "40", "3", "--rise", "2", "--slope", "30", "--level", "9", "--mode", "1",
        "--kept", "5", "--keys", "6",
    ];
    assert_eq!(
        asked(&every_flag),
        Ok(Asked {
            z: Some(3.0),
            rise: 2.0,
            slope: 30.0,
            ..plain
        })
    );
    // A place as `world at` takes one: pasted in one argument, or as the words a shell split it into, with a height or none.
    for (pasted, z) in [
        (&["-12.5,40"][..], None),
        (&["-12.5,", "40"], None),
        (&["(-12.5, 40, 3)"], Some(3.0)),
        (&["X:", "-12.5,", "Y:", "40,", "Z:", "3"], Some(3.0)),
        (
            &["--game", "7", "vector3(-12.5,", "40,", "3)", "--json"],
            Some(3.0),
        ),
    ] {
        assert_eq!(asked(pasted), Ok(Asked { z, ..plain }), "{pasted:?}");
    }
    for (args, why) in [
        (&["40"][..], "and 1 were given"),
        (&["1", "2", "3", "4"], "and 4 were given"),
        (&["1", "nan"], "`nan` is not a number"),
        (&["1", "2", "--rise", "-1"], "--rise is metres, 0 or more"),
        (&["1", "2", "--rise", "nan"], "--rise is metres, 0 or more"),
        (&["1", "2", "--rise", "inf"], "--rise is metres, 0 or more"),
        (
            &["1", "2", "--slope", "91"],
            "--slope is degrees from 0 to 90",
        ),
        (&["1", "2", "--slope", "steep"], "not `steep`"),
        // A flag misspelt would give its value to the position as a height.
        (
            &["1", "2", "--rize", "3"],
            "`--rize` is no flag of this verb",
        ),
        (
            &["1", "2", "--dlc=mpbeta"],
            "`--dlc` is no flag of this verb",
        ),
        // A flag left without its value would stand at its default, or mount every pack, unsaid.
        (&["1", "2", "--rise"], "`--rise` takes a value"),
        (&["1", "2", "--level"], "`--level` takes a value"),
    ] {
        let refused = asked(args).expect_err("refused");
        assert!(refused.contains(why), "{args:?}: {refused}");
    }

    let (cutoff, options) = mount_of(&strings(&["1", "2"])).expect("no flag");
    assert_eq!((cutoff, options.mode), (DlcCutoff::All, None));
    let level = strings(&["1", "2", "--level", "mpbeta", "--mode", "Story"]);
    let (cutoff, options) = mount_of(&level).expect("both flags");
    assert_eq!(cutoff, DlcCutoff::Level("mpbeta".into()));
    assert_eq!(options.mode, Some(MapMode::Story));
    let refused = mount_of(&strings(&["1", "2", "--mode", "online"])).expect_err("no such mode");
    assert!(refused.contains("`online`"), "{refused}");
}

/// The archive of `beta` that holds its map, which its one change set enables.
const BETA_LEVELS: &str = "dlc_beta:/levels.rpf";
const BETA_CHANGE_SET: &str = "BETA_MAP";

/// A pack's setup; with `map_group`, one whose map group of multiplayer alone names the pack's change set.
fn setup(name: &str, order: i32, map_group: bool) -> SetupData {
    let group = ContentChangeSetGroup {
        name_hash: "GROUP_MAP".into(),
        content_change_sets: vec![BETA_CHANGE_SET.to_string().into()],
    };
    SetupData {
        name_hash: name.into(),
        device_name: format!("dlc_{name}"),
        dat_file: "content.xml".into(),
        order,
        content_change_set_groups: map_group.then_some(group).into_iter().collect(),
        ..Default::default()
    }
}

/// The town, and two packs the DLC list names after it: `alpha`, which holds no map, and `beta`,
/// whose floor stands ten metres over the town's in an archive that a change set of multiplayer
/// alone enables.
fn install_with_packs() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let list = DlcList {
        paths: ["alpha", "beta"]
            .iter()
            .map(|pack| format!("dlcpacks:/{pack}/").into())
            .collect(),
    };
    write_archive(
        &root,
        "common.rpf",
        &[
            ("data/dlclist.xml", list.to_xml().into_bytes()),
            ("data/_manifest.ymf", town_manifest()),
        ],
    );
    write_archive(&root, "x64a.rpf", &town_files());
    let nothing = ContentXml::default();
    let levels = ContentXml {
        data_files: vec![DataFileEntry {
            filename: BETA_LEVELS.into(),
            file_type: "RPF_FILE".into(),
            disabled: true,
            ..Default::default()
        }],
        content_change_sets: vec![ContentChangeSet {
            name: BETA_CHANGE_SET.into(),
            files_to_enable: vec![BETA_LEVELS.to_string().into()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let beta_map = MapSpec::around(cube([100.0, 100.0, 0.0], 100.0));
    let packs: [(&str, i32, ContentXml, Files); 2] = [
        ("alpha", 10, nothing, Vec::new()),
        (
            "beta",
            20,
            levels,
            vec![
                ("levels.rpf/m_beta.ymap", map_bytes("m_beta", &beta_map)),
                (
                    "levels.rpf/m_beta_0.ybn",
                    mesh_bound_bytes(level_square([100.0, 100.0, 20.0], 100.0, ROOF)),
                ),
            ],
        ),
    ];
    for (name, order, content, files) in packs {
        let folder = format!("x64/dlcpacks/{name}");
        std::fs::create_dir_all(root.join(&folder)).expect("mkdir");
        let setup = setup(name, order, name == "beta");
        let mut files = files;
        files.push(("setup2.xml", setup.to_xml().into_bytes()));
        files.push(("content.xml", content.to_xml().into_bytes()));
        write_archive(&root, &format!("{folder}/dlc.rpf"), &files);
    }
    (scratch, root)
}

/// The install at `root`, mounted as the flags of `args` say.
fn mounted_as(root: &Path, args: &[&str]) -> MountedGame {
    let (cutoff, options) = mount_of(&strings(args)).expect("the flags read");
    let install = GtaInstall {
        root: root.to_path_buf(),
    };
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
    build_mounted_game_with(&install, crypto, cutoff, options).expect("the install mounts")
}

#[test]
fn a_level_leaves_out_the_bounds_of_the_packs_after_it_and_a_mode_leaves_out_none() {
    let (scratch, root) = install_with_packs();
    // One kept file for every level, so what one mount read is offered to the next.
    let kept = scratch.path().join("world.index");
    let floor = stored_z(&town(), 0);
    let raised = stored_z(&level_square([100.0, 100.0, 20.0], 100.0, ROOF), 0);
    assert!(raised > floor + 5.0);
    let ask = |args: &[&str]| say(&mounted_as(&root, args), &root, &kept, args);

    // With every pack, the ground of the column is the later pack's floor.
    let (lines, whole) = ask(&["190", "110"]);
    assert_eq!(whole["ground"]["height"], json!(raised));
    assert_eq!(whole["ground"]["bound"]["name"], "m_beta_0");
    assert_eq!(whole["lookedIn"].as_array().map(Vec::len), Some(2));
    assert_eq!(whole["notes"], json!([]));
    assert!(
        lines.iter().all(|line| !line.contains("note:")),
        "{lines:#?}"
    );

    // Stopped at the pack before it, the town's own floor, and the pack left out is named.
    let at_alpha = ["190", "110", "--level", "alpha"];
    let (lines, level) = ask(&at_alpha);
    assert_eq!(level["ground"]["height"], json!(floor));
    assert_eq!(level["ground"]["bound"]["name"], "m_town_0");
    assert_eq!(level["lookedIn"].as_array().map(Vec::len), Some(1));
    let note = "the level leaves out 1 of 2 listed pack(s) and their bounds: beta";
    assert_eq!(level["notes"], json!([note]));
    assert_eq!(lines.last(), Some(&format!("  note: {note}")));
    // The last pack of the list is every pack, and what was kept for a level serves it rightly.
    assert_eq!(ask(&["190", "110", "--level", "beta"]).1, whole);
    assert_eq!(ask(&at_alpha).1, level);

    // Story leaves out the change set that enables the later pack's archive, and multiplayer
    // takes it.
    let left_out = |mode: &str| -> Vec<String> {
        let game = mounted_as(&root, &["--mode", mode]);
        let packs = game.packs.iter();
        packs
            .flat_map(|pack| pack.change_sets_left_out.clone())
            .collect()
    };
    assert_eq!(left_out("story"), [BETA_CHANGE_SET]);
    assert_eq!(left_out("multiplayer"), Vec::<String>::new());
    // ⚠ Either way the bound in that archive answers: a mode leaves out no bounds file, and
    // the verb says so.
    for mode in ["story", "multiplayer"] {
        let (lines, in_mode) = ask(&["190", "110", "--mode", mode]);
        assert_eq!(in_mode["ground"], whole["ground"]);
        assert_eq!(in_mode["lookedIn"], whole["lookedIn"]);
        let note = in_mode["notes"][0].as_str().expect("a note");
        assert!(
            note.starts_with(&format!(
                "mounted as {mode}, which leaves out no bounds file"
            )),
            "{note}"
        );
        assert_eq!(lines.last(), Some(&format!("  note: {note}")));
        // What the mode did leave out is said: the change set stated for the other one.
        let stated = format!("; 1 change set(s) stated for the other mode: {BETA_CHANGE_SET}");
        assert_eq!(note.ends_with(&stated), mode == "story", "{note}");
        assert_eq!(note.contains("change set(s)"), mode == "story", "{note}");
    }
}

#[test]
fn ground_that_lists_no_material_and_ground_a_caller_placed_are_said_as_such() {
    use or_depends::world_collision::{Collider, Ground, Streamed};
    use or_world::collision::{Hit, Part};

    let hit = Hit {
        distance: 4.0,
        position: Vec3::new(1.0, 2.0, 3.5),
        normal: Vec3::Z,
        material: None,
        part: Part {
            leaf: 0,
            polygon: Some(0),
        },
    };
    let answer = GroundAnswer {
        ground: Some(Ground {
            hit,
            collider: Collider::Placed(3),
            bound: None,
        }),
        streamed: Streamed::default(),
    };
    let asked = asked_of(&strings(&["1", "2"])).expect("a position");
    let none = MaterialPalette::empty();
    let lines = ground_lines(&asked, &answer, none, &[]);
    assert_eq!(
        lines[..4],
        [
            "Ground under (1, 2), the whole of its column: 3.500",
            "  at (1.000, 2.000, 3.500), normal (0.000, 0.000, 1.000)",
            "  material none: the polygon's bound lists no material",
            "  from a collider its caller placed",
        ]
    );
    let json = ground_json(&asked, &answer, none, &[]);
    assert_eq!(json["ground"]["height"], json!(3.5));
    assert_eq!(json["ground"]["material"], json!(null));
    assert_eq!(json["ground"]["bound"], json!(null));
}

#[test]
fn an_index_that_could_not_be_kept_is_said_and_the_ground_is_answered_all_the_same() {
    let (scratch, root) = install();
    let game = mount(&root);
    let asked = asked_of(&strings(&["190", "110"])).expect("a position");
    // A folder is no file an index can be written to.
    let (lines, json, not_kept) = said(
        &game,
        &root,
        scratch.path(),
        false,
        &asked,
        None,
        &palette(),
    );
    assert!(not_kept.is_some(), "the folder was written as a file");
    assert_eq!(json["ground"]["height"], json!(stored_z(&town(), 0)));
    assert_eq!(lines.len(), 6, "{lines:#?}");
}

#[test]
fn an_index_at_a_level_is_kept_in_a_file_of_its_own() {
    let root = Path::new("/an/install");
    let kept = |level| super::super::world_index::kept_name(root, level);
    let (whole, level) = (kept(None), kept(Some("mpbeta")));
    assert_ne!(level, whole);
    assert_eq!(
        kept(Some(" MPBeta ")),
        level,
        "a level is its name in any case"
    );
    assert_ne!(kept(Some("mpalpha")), level);
    // An install whose path ends as a level is named does not share that level's file.
    assert_ne!(
        super::super::world_index::kept_name(Path::new("/an/installmpbeta"), None),
        level
    );
}
