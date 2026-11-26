//! A mount stops at a pack of the DLC list when asked to, and takes the map as story or as
//! multiplayer by the groups a pack's setup names its change sets under.
//!
//! Every install here is a scratch directory of unencrypted archives built by this file, so the
//! suite needs no keys and no game.

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{NoOpCryptoProvider, RpfCryptoProvider};
use or_io::container::rpf::RpfBuilder;
use or_title::dlc::{ContentChangeSet, ContentChangeSetGroup, MapChangeSet};
use or_title::vfs::{build_mounted_game, build_mounted_game_with, DlcCutoff};
use or_title::{
    ContentXml, DataFileEntry, DlcList, ExtraTitleUpdateData, GtaInstall, ListedPackState, MapMode,
    MountOptions, MountedGame, SetupData, TitleUpdateMount, VfsBuildError,
};

/// A file every pack holds, each with its own bytes.
const SHARED: &str = "shared.ydr";
/// A file only the last listed pack holds.
const ONLY_GAMMA: &str = "only_gamma.ydr";
/// A file only the title update's patch of the last listed pack holds.
const ONLY_GAMMA_PATCH: &str = "only_gamma_patch.ydr";

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = RpfBuilder::default();
    for (path, body) in entries {
        builder
            .insert_with(path, *body, false)
            .expect("add an entry");
    }
    builder.to_bytes().expect("save")
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, bytes).expect("write");
}

fn device(name: &str) -> String {
    format!("dlc_{name}")
}

/// A pack's `dlc.rpf` on disk, under the folder its name gives it: its two documents and `files`.
fn write_pack(root: &Path, setup: &SetupData, content: &ContentXml, files: &[(&str, &[u8])]) {
    let (setup_xml, content_xml) = (setup.to_xml(), content.to_xml());
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("setup2.xml", setup_xml.as_bytes()),
        ("content.xml", content_xml.as_bytes()),
    ];
    entries.extend_from_slice(files);
    let folder = root.join("x64/dlcpacks").join(&setup.name_hash);
    write(&folder.join("dlc.rpf"), &archive(&entries));
}

fn setup(name: &str, order: i32) -> SetupData {
    SetupData {
        name_hash: name.into(),
        device_name: device(name),
        dat_file: "content.xml".into(),
        order,
        ..Default::default()
    }
}

/// An install whose list holds `entries` in that order, with no pack on disk yet.
fn install_listing(name: &str, entries: &[&str]) -> (tempfile::TempDir, GtaInstall) {
    let scratch = tempfile::Builder::new()
        .prefix(&format!("or-title-level-{name}-"))
        .tempdir()
        .expect("scratch");
    let root = scratch.path().to_path_buf();
    let list = DlcList {
        paths: entries
            .iter()
            .map(|entry| entry.to_string().into())
            .collect(),
    };
    write(
        &root.join("common.rpf"),
        &archive(&[("data/dlclist.xml", list.to_xml().as_bytes())]),
    );
    (scratch, GtaInstall { root })
}

/// An install whose list names `alpha`, `beta` and `gamma` in that order, each mounting at the order given.
fn install(name: &str, orders: [i32; 3]) -> (tempfile::TempDir, GtaInstall) {
    let entries = ["alpha", "beta", "gamma"].map(|pack| format!("dlcpacks:/{pack}/"));
    let (scratch, install) = install_listing(name, &entries.each_ref().map(String::as_str));
    let root = &install.root;
    let [alpha, beta, gamma] = orders;
    let empty = ContentXml::default();
    write_pack(root, &setup("alpha", alpha), &empty, &[(SHARED, b"alpha")]);
    write_pack(root, &setup("beta", beta), &empty, &[(SHARED, b"beta")]);
    write_pack(
        root,
        &setup("gamma", gamma),
        &empty,
        &[(SHARED, b"gamma"), (ONLY_GAMMA, b"gamma")],
    );
    (scratch, install)
}

/// A title update whose table gives the device of each pack of `patches` the path beside it, holding `files`.
fn write_title_update_of(root: &Path, patches: &[(&str, &str)], files: &[(&str, &[u8])]) {
    let table = ExtraTitleUpdateData {
        mounts: patches
            .iter()
            .map(|(pack, path)| TitleUpdateMount {
                device_name: device(pack),
                path: path.to_string(),
            })
            .collect(),
    };
    let table = table.to_xml();
    let mut entries: Vec<(&str, &[u8])> =
        vec![("common/data/ExtraTitleUpdateData.meta", table.as_bytes())];
    entries.extend_from_slice(files);
    write(&root.join("update/update.rpf"), &archive(&entries));
}

/// A title update that patches `beta` and `gamma`, each from the folder its table names.
fn write_title_update(root: &Path) {
    write_title_update_of(
        root,
        &[
            ("beta", "update:/dlc_patch/beta/"),
            ("gamma", "update:/dlc_patch/gamma/"),
        ],
        &[
            ("dlc_patch/beta/shared.ydr", b"beta patch"),
            ("dlc_patch/gamma/shared.ydr", b"gamma patch"),
            ("dlc_patch/gamma/only_gamma_patch.ydr", b"gamma patch"),
        ],
    );
}

fn crypto() -> Arc<dyn RpfCryptoProvider> {
    Arc::new(NoOpCryptoProvider::new())
}

fn mount(install: &GtaInstall, cutoff: DlcCutoff) -> MountedGame {
    build_mounted_game(install, crypto(), cutoff).expect("the synthetic install mounts")
}

fn level(pack: &str) -> DlcCutoff {
    DlcCutoff::Level(pack.into())
}

/// The bytes a file name resolves to: the last copy the walk meets, as `Gta5` indexes an install.
fn resolve(game: &MountedGame, leaf: &str) -> Option<Vec<u8>> {
    let mut logical = None;
    game.for_each_entry(|facts| {
        if facts.leaf.eq_ignore_ascii_case(leaf) {
            logical = Some(facts.logical.clone());
        }
    });
    game.vfs.open(&logical?)
}

fn mounted(game: &MountedGame) -> Vec<&str> {
    game.packs.iter().map(|p| p.name_hash.as_str()).collect()
}

fn listed(game: &MountedGame) -> Vec<(&str, ListedPackState)> {
    (game.listed.iter())
        .map(|pack| (pack.name.as_str(), pack.state))
        .collect()
}

#[test]
fn a_level_set_to_the_second_of_three_packs_hides_the_third_and_keeps_the_seconds_copy() {
    let (_scratch, install) = install("second", [10, 20, 30]);
    let game = mount(&install, level("beta"));

    assert_eq!(mounted(&game), ["alpha", "beta"]);
    assert_eq!(
        resolve(&game, ONLY_GAMMA),
        None,
        "a file only the third pack holds does not resolve by name"
    );
    assert_eq!(
        game.vfs.open(&format!("{}:/{ONLY_GAMMA}", device("gamma"))),
        None,
        "nor by the third pack's device, which is not mounted"
    );
    assert_eq!(
        resolve(&game, SHARED).as_deref(),
        Some(&b"beta"[..]),
        "a file the second pack overrides resolves to the second pack's copy"
    );
}

#[test]
fn with_no_level_all_three_packs_mount_and_the_last_copy_wins() {
    let (_scratch, install) = install("none", [10, 20, 30]);
    let game = mount(&install, DlcCutoff::All);

    assert_eq!(mounted(&game), ["alpha", "beta", "gamma"]);
    assert_eq!(resolve(&game, ONLY_GAMMA).as_deref(), Some(&b"gamma"[..]));
    assert_eq!(resolve(&game, SHARED).as_deref(), Some(&b"gamma"[..]));
    assert_eq!(
        listed(&game),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::Mounted),
            ("gamma", ListedPackState::Mounted),
        ]
    );
    assert!(
        game.roots.iter().all(|root| root.left_out.is_empty()),
        "and no folder of any archive is kept from the walk"
    );
}

#[test]
fn the_mount_reports_every_listed_pack_with_the_ones_the_level_left_out() {
    let (_scratch, install) = install("report", [10, 20, 30]);

    assert_eq!(
        listed(&mount(&install, level("beta"))),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::Mounted),
            ("gamma", ListedPackState::LeftOut),
        ]
    );
    assert_eq!(
        listed(&mount(&install, level("ALPHA"))),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::LeftOut),
            ("gamma", ListedPackState::LeftOut),
        ],
        "a level is matched whatever its case, as a pack's name is"
    );
    assert!(
        mount(&install, DlcCutoff::BaseGame).listed.is_empty(),
        "a mount of the base game reads no list"
    );
}

/// ⚠ The setup's `order` is where a pack mounts, not when it was listed: a pack listed last can
/// mount first, and stopping by mount order would keep it.
#[test]
fn a_level_is_a_place_in_the_list_whatever_order_the_packs_mount_in() {
    let (_scratch, install) = install("order", [10, 20, 5]);

    let all = mount(&install, DlcCutoff::All);
    assert_eq!(
        mounted(&all),
        ["gamma", "alpha", "beta"],
        "the pack listed last mounts first"
    );

    let game = mount(&install, level("beta"));
    assert_eq!(mounted(&game), ["alpha", "beta"]);
    assert_eq!(resolve(&game, ONLY_GAMMA), None);

    let by_name_hash = mount(&install, DlcCutoff::NameHash("beta".into()));
    assert_eq!(
        mounted(&by_name_hash),
        ["gamma", "alpha", "beta"],
        "the cutoff by name hash stops in mount order, as it did"
    );
    let by_name_hash = mount(&install, DlcCutoff::NameHash("alpha".into()));
    assert_eq!(
        listed(&by_name_hash),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::LeftOut),
            ("gamma", ListedPackState::Mounted),
        ],
        "and what it drops is reported as left out"
    );
}

#[test]
fn a_level_naming_a_pack_the_list_lacks_is_refused_with_its_name() {
    let (_scratch, install) = install("lacks", [10, 20, 30]);
    let refused = build_mounted_game(&install, crypto(), level("delta"));
    let Err(error) = refused else {
        panic!("a level the list lacks mounted")
    };
    assert!(
        matches!(&error, VfsBuildError::LevelNotListed { level, listed } if level == "delta" && *listed == 3),
        "{error:?}"
    );
    assert!(error.to_string().contains("`delta`"), "{error}");
}

#[test]
fn a_level_is_refused_on_an_install_that_has_no_list() {
    let (_scratch, install) = install("no-list", [10, 20, 30]);
    write(
        &install.root.join("common.rpf"),
        &archive(&[("data/other.dat", b"no list here")]),
    );
    assert!(
        matches!(
            build_mounted_game(&install, crypto(), level("alpha")),
            Err(VfsBuildError::LevelNotListed { listed: 0, .. })
        ),
        "an install with no list has no pack to stop at"
    );
    assert!(
        mount(&install, DlcCutoff::All).packs.is_empty(),
        "and with no level it mounts with no pack, as it did"
    );
}

fn walked(game: &MountedGame) -> Vec<String> {
    let mut out = Vec::new();
    game.for_each_entry(|facts| out.push(facts.logical.to_ascii_lowercase()));
    out
}

#[test]
fn the_title_updates_patch_of_a_pack_past_the_level_is_not_applied() {
    let (_scratch, install) = install("patch", [10, 20, 30]);
    write_title_update(&install.root);
    let gamma_patch = "update:/dlc_patch/gamma/";

    let all = mount(&install, DlcCutoff::All);
    assert_eq!(
        (all.vfs
            .open(&format!("{}:/{ONLY_GAMMA_PATCH}", device("gamma"))))
        .as_deref(),
        Some(&b"gamma patch"[..]),
        "with no level the patch lies over its pack: {:?}",
        all.diagnostics.messages()
    );
    assert!(walked(&all).iter().any(|l| l.starts_with(gamma_patch)));

    let game = mount(&install, level("beta"));
    assert_eq!(
        (game.vfs.open(&format!("{}:/{SHARED}", device("beta")))).as_deref(),
        Some(&b"beta patch"[..]),
        "the patch of a pack the level keeps still lies over it"
    );
    assert_eq!(
        game.vfs.open(&format!("{}:/{SHARED}", device("gamma"))),
        None
    );
    assert_eq!(
        resolve(&game, ONLY_GAMMA_PATCH),
        None,
        "a file only the left-out pack's patch holds does not resolve by name"
    );
    let walked = walked(&game);
    assert!(
        !walked.iter().any(|l| l.starts_with(gamma_patch)),
        "the walk skips the folder the table gives the left-out pack: {walked:?}"
    );
    assert!(
        walked
            .iter()
            .any(|l| l.starts_with("update:/dlc_patch/beta/")),
        "and no other folder"
    );
}

/// ⚠ A pack inside the title update's archive is reached twice by a walk, through the archive
/// that holds it and through its own device, so leaving its device out is half of leaving it out.
#[test]
fn a_pack_past_the_level_that_the_title_update_holds_is_not_walked_through_it() {
    let (_scratch, install) = install("nested", [10, 20, 30]);
    let on_disk = install.root.join("x64/dlcpacks/gamma/dlc.rpf");
    let gamma = std::fs::read(&on_disk).expect("gamma's archive");
    std::fs::remove_file(&on_disk).expect("gamma leaves the disk");
    write(
        &install.root.join("update/update.rpf"),
        &archive(&[("dlcpacks/gamma/dlc.rpf", &gamma)]),
    );
    let inside = "update:/dlcpacks/gamma/";

    let all = mount(&install, DlcCutoff::All);
    assert_eq!(mounted(&all), ["alpha", "beta", "gamma"]);
    assert!(walked(&all).iter().any(|l| l.starts_with(inside)));

    let game = mount(&install, level("beta"));
    assert_eq!(resolve(&game, ONLY_GAMMA), None);
    let walked = walked(&game);
    assert!(
        !walked.iter().any(|l| l.starts_with(inside)),
        "the walk skips the folder that holds the left-out pack: {walked:?}"
    );
}

/// ⚠ The older cutoff, by a pack's name hash, reports the packs past it as left out too, so it keeps the same folders from the walk.
#[test]
fn a_pack_past_a_name_hash_is_left_out_of_the_walk_as_one_past_a_level_is() {
    let by_hash = || DlcCutoff::NameHash(setup("beta", 0).name_hash);

    let (_scratch, patched) = install("hash-patch", [10, 20, 30]);
    write_title_update(&patched.root);
    let game = mount(&patched, by_hash());
    assert_eq!(mounted(&game), ["alpha", "beta"]);
    assert_eq!(
        listed(&game).last(),
        Some(&("gamma", ListedPackState::LeftOut))
    );
    assert_eq!(
        (game.vfs.open(&format!("{}:/{SHARED}", device("beta")))).as_deref(),
        Some(&b"beta patch"[..]),
        "the patch of a pack that is kept still lies over it"
    );
    assert_eq!(
        resolve(&game, ONLY_GAMMA_PATCH),
        None,
        "a file only the left-out pack's patch holds does not resolve by name"
    );
    let walk = walked(&game);
    assert!(
        !walk
            .iter()
            .any(|l| l.starts_with("update:/dlc_patch/gamma/")),
        "the walk skips the folder the table gives the left-out pack: {walk:?}"
    );
    assert!(walk
        .iter()
        .any(|l| l.starts_with("update:/dlc_patch/beta/")));

    // The left-out pack held by the title update's archive is not walked through it either.
    let (_scratch, nested) = install("hash-nested", [10, 20, 30]);
    let install = nested;
    let on_disk = install.root.join("x64/dlcpacks/gamma/dlc.rpf");
    let gamma = std::fs::read(&on_disk).expect("gamma's archive");
    std::fs::remove_file(&on_disk).expect("gamma leaves the disk");
    write(
        &install.root.join("update/update.rpf"),
        &archive(&[("dlcpacks/gamma/dlc.rpf", &gamma)]),
    );
    let game = mount(&install, by_hash());
    assert_eq!(mounted(&game), ["alpha", "beta"]);
    assert_eq!(resolve(&game, ONLY_GAMMA), None);
    let walk = walked(&game);
    assert!(
        !walk
            .iter()
            .any(|l| l.starts_with("update:/dlcpacks/gamma/")),
        "{walk:?}"
    );
}

/// ⚠ A folder is left out whole and by its own name: `dlc_patch/beta/` is not the front of
/// `dlc_patch/beta2/`, the table may spell it in another case than the archive, and another
/// archive's folder of the same path is not the patch.
#[test]
fn only_the_folder_the_table_gives_the_pack_left_out_is_kept_from_the_walk() {
    let entries = ["dlcpacks:/beta2/", "dlcpacks:/beta/"];
    let (_scratch, install) = install_listing("folder", &entries);
    let empty = ContentXml::default();
    write_pack(&install.root, &setup("beta2", 10), &empty, &[]);
    write_pack(&install.root, &setup("beta", 20), &empty, &[]);
    write_title_update_of(
        &install.root,
        &[
            ("beta2", "update:/dlc_patch/beta2/"),
            ("beta", "update:/DLC_PATCH/Beta/"),
        ],
        &[
            ("dlc_patch/beta2/kept.ydr", b"kept"),
            ("dlc_patch/beta/left_out.ydr", b"left out"),
        ],
    );
    write(
        &install.root.join("x64a.rpf"),
        &archive(&[("dlc_patch/beta/elsewhere.ydr", b"not a patch")]),
    );
    let left_out = "update:/dlc_patch/beta/left_out.ydr".to_string();

    let all = walked(&mount(&install, DlcCutoff::All));
    assert!(all.contains(&left_out), "{all:?}");

    let game = mount(&install, level("beta2"));
    assert_eq!(mounted(&game), ["beta2"]);
    let walked = walked(&game);
    assert!(
        !walked.contains(&left_out),
        "the patch of the pack left out is skipped: {walked:?}"
    );
    for kept in [
        "update:/dlc_patch/beta2/kept.ydr",
        "platform:/dlc_patch/beta/elsewhere.ydr",
    ] {
        assert!(walked.iter().any(|l| l == kept), "{kept} in {walked:?}");
    }
}

/// ⚠ A list edited by hand can name a pack twice. The second naming is past the level, and the
/// pack it names is the one the level keeps, so its folders are not the ones to skip.
#[test]
fn a_pack_the_list_names_again_past_the_level_keeps_its_patch_in_the_walk() {
    let (_scratch, install) = install("twice", [10, 20, 30]);
    let entries = ["alpha", "beta", "gamma", "beta"].map(|pack| format!("dlcpacks:/{pack}/"));
    let list = DlcList {
        paths: entries.iter().map(|entry| entry.clone().into()).collect(),
    };
    write(
        &install.root.join("common.rpf"),
        &archive(&[("data/dlclist.xml", list.to_xml().as_bytes())]),
    );
    write_title_update(&install.root);

    let game = mount(&install, level("beta"));
    assert_eq!(mounted(&game), ["alpha", "beta"]);
    assert_eq!(
        listed(&game),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::Mounted),
            ("gamma", ListedPackState::LeftOut),
            ("beta", ListedPackState::LeftOut),
        ]
    );
    let walked = walked(&game);
    assert!(
        walked
            .iter()
            .any(|l| l.starts_with("update:/dlc_patch/beta/")),
        "the patch of the pack the level keeps is walked: {walked:?}"
    );
    assert!(!walked
        .iter()
        .any(|l| l.starts_with("update:/dlc_patch/gamma/")));
}

#[test]
fn a_listed_pack_that_does_not_mount_is_reported_as_skipped_and_can_be_the_level() {
    let (_scratch, install) = install("skipped", [10, 20, 30]);
    std::fs::remove_file(install.root.join("x64/dlcpacks/beta/dlc.rpf")).expect("beta leaves");

    assert_eq!(
        listed(&mount(&install, DlcCutoff::All)),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::Skipped),
            ("gamma", ListedPackState::Mounted),
        ]
    );
    let game = mount(&install, level("beta"));
    assert_eq!(mounted(&game), ["alpha"]);
    assert_eq!(
        listed(&game),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::Skipped),
            ("gamma", ListedPackState::LeftOut),
        ],
        "the level is a place in the list, whether or not its pack mounts"
    );
}

#[test]
fn a_level_is_trimmed_and_an_entry_that_names_no_pack_is_neither_a_level_nor_counted() {
    let entries = ["dlcpacks:/alpha/", "dlcpacks:/", "dlcpacks:/beta/"];
    let (_scratch, install) = install_listing("unnamed", &entries);
    let empty = ContentXml::default();
    write_pack(&install.root, &setup("alpha", 10), &empty, &[]);
    write_pack(&install.root, &setup("beta", 20), &empty, &[]);

    assert_eq!(
        listed(&mount(&install, level(" alpha "))),
        [
            ("alpha", ListedPackState::Mounted),
            ("beta", ListedPackState::LeftOut),
        ]
    );
    let refused = build_mounted_game(&install, crypto(), level(""));
    assert!(
        matches!(
            &refused,
            Err(VfsBuildError::LevelNotListed { level, listed: 2 }) if level.is_empty()
        ),
        "{:?}",
        refused.err()
    );
}

/// ⚠ The folder that patches a pack is found through the pack's setup, so a pack past the level
/// whose setup does not read keeps its patch in the walk, and the mount says so.
#[test]
fn a_pack_past_the_level_whose_setup_does_not_read_is_said_to_keep_its_patch_in_the_walk() {
    let (_scratch, readable) = install("read", [10, 20, 30]);
    write_title_update(&readable.root);
    let unread = archive(&[("setup2.xml", b"<SSetupData><deviceName>")]);
    let (_scratch, unpatched) = install("unpatched", [10, 20, 30]);
    write(&unpatched.root.join("x64/dlcpacks/gamma/dlc.rpf"), &unread);
    let (_scratch, install) = install("unread", [10, 20, 30]);
    write_title_update(&install.root);
    write(&install.root.join("x64/dlcpacks/gamma/dlc.rpf"), &unread);
    let said = |game: &MountedGame| {
        (game.diagnostics.messages().iter())
            .filter(|d| d.key == or_title::vfs::keys::PATCH_OF_A_PACK_LEFT_OUT_UNKNOWN)
            .filter_map(|d| d.location.clone())
            .collect::<Vec<_>>()
    };

    let game = mount(&install, level("beta"));
    assert_eq!(said(&game), ["gamma"]);
    assert!(
        walked(&game)
            .iter()
            .any(|l| l.starts_with("update:/dlc_patch/gamma/")),
        "the patch nothing names the pack of is still walked"
    );
    assert!(
        said(&mount(&install, DlcCutoff::All)).is_empty(),
        "with no level the pack is skipped as it was, and nothing is said of a patch"
    );
    assert!(said(&mount(&readable, level("beta"))).is_empty());
    assert!(
        said(&mount(&unpatched, level("beta"))).is_empty(),
        "an install with no title update has no patch to speak of"
    );
}

/// A change set that enables `file`, at the top level or inside a map's slice.
fn change_set(name: &str, file: &str, map_scoped: bool) -> ContentChangeSet {
    let enable = vec![file.to_string().into()];
    let mut set = ContentChangeSet {
        name: name.into(),
        ..Default::default()
    };
    if map_scoped {
        set.map_change_set_data = vec![MapChangeSet {
            associated_map: "gta5".into(),
            files_to_enable: enable,
            ..Default::default()
        }];
    } else {
        set.files_to_enable = enable;
    }
    set
}

fn group(name: &str, change_sets: &[&str]) -> ContentChangeSetGroup {
    ContentChangeSetGroup {
        name_hash: name.into(),
        content_change_sets: change_sets.iter().map(|c| c.to_string().into()).collect(),
    }
}

const ALWAYS: &str = "dlc_delta:/always.rpf";
const MULTIPLAYER_ONLY: &str = "dlc_delta:/mp.rpf";
const STORY_ONLY: &str = "dlc_delta:/sp.rpf";
const ON_DEMAND: &str = "dlc_delta:/on_demand.rpf";
const IN_NO_GROUP: &str = "dlc_delta:/in_no_group.rpf";

/// An install of one pack whose setup states a startup group, a map group for each mode and a
/// group of another name, and whose content holds one change set more than the groups name.
fn install_with_modes() -> (tempfile::TempDir, GtaInstall) {
    let scratch = tempfile::Builder::new()
        .prefix("or-title-mode-")
        .tempdir()
        .expect("scratch");
    let root = scratch.path().to_path_buf();
    let list = DlcList {
        paths: vec!["dlcpacks:/delta/".to_string().into()],
    };
    write(
        &root.join("common.rpf"),
        &archive(&[("data/dlclist.xml", list.to_xml().as_bytes())]),
    );
    let setup = SetupData {
        content_change_set_groups: vec![
            group("GROUP_STARTUP", &["DELTA_AUTOGEN"]),
            group("GROUP_MAP", &["DELTA_MAP"]),
            group("GROUP_MAP_SP", &["DELTA_MAP_SP"]),
            group("GROUP_ON_DEMAND", &["DELTA_ON_DEMAND"]),
        ],
        ..setup("delta", 10)
    };
    let files = [ALWAYS, MULTIPLAYER_ONLY, STORY_ONLY, ON_DEMAND, IN_NO_GROUP];
    let content = ContentXml {
        data_files: (files.iter())
            .map(|file| DataFileEntry {
                filename: file.to_string(),
                file_type: "RPF_FILE".into(),
                disabled: true,
                ..Default::default()
            })
            .collect(),
        content_change_sets: vec![
            change_set("DELTA_AUTOGEN", ALWAYS, false),
            change_set("DELTA_MAP", MULTIPLAYER_ONLY, true),
            change_set("DELTA_MAP_SP", STORY_ONLY, true),
            change_set("DELTA_ON_DEMAND", ON_DEMAND, false),
            change_set("DELTA_IN_NO_GROUP", IN_NO_GROUP, false),
        ],
        ..Default::default()
    };
    write_pack(&root, &setup, &content, &[]);
    (scratch, GtaInstall { root })
}

/// The data files of the one pack that the mount left enabled, and the pack's two lists of change sets.
fn in_mode(install: &GtaInstall, mode: Option<MapMode>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let options = MountOptions {
        mode,
        ..MountOptions::default()
    };
    let game = build_mounted_game_with(install, crypto(), DlcCutoff::All, options)
        .expect("the synthetic install mounts");
    let [pack] = game.packs.as_slice() else {
        panic!("one pack mounts: {:?}", game.diagnostics.messages())
    };
    let enabled = (pack.data_files.iter())
        .filter(|file| !file.disabled)
        .map(|file| file.filename.clone())
        .collect();
    (
        enabled,
        pack.change_sets_left_out.clone(),
        pack.change_sets_unstated.clone(),
    )
}

#[test]
fn a_change_set_stated_for_multiplayer_applies_there_and_not_in_story_and_the_reverse() {
    let (_scratch, install) = install_with_modes();

    let (enabled, left_out, unstated) = in_mode(&install, Some(MapMode::Multiplayer));
    assert_eq!(enabled, [ALWAYS, MULTIPLAYER_ONLY, ON_DEMAND, IN_NO_GROUP]);
    assert_eq!(left_out, ["DELTA_MAP_SP"]);
    assert_eq!(unstated, ["DELTA_ON_DEMAND", "DELTA_IN_NO_GROUP"]);

    let (enabled, left_out, unstated) = in_mode(&install, Some(MapMode::Story));
    assert_eq!(enabled, [ALWAYS, STORY_ONLY, ON_DEMAND, IN_NO_GROUP]);
    assert_eq!(left_out, ["DELTA_MAP"]);
    assert_eq!(
        unstated,
        ["DELTA_ON_DEMAND", "DELTA_IN_NO_GROUP"],
        "a change set no group states a mode for applies in both and is named"
    );
}

/// ⚠ The mount reads the title update's content document and not its setup, so it knows no group
/// of the title update's change sets.
#[test]
fn the_title_updates_change_sets_apply_in_either_mode_and_are_named_as_unstated() {
    let (_scratch, install) = install("title-update-mode", [10, 20, 30]);
    let file = "update:/x64/patch.rpf";
    let content = ContentXml {
        data_files: vec![DataFileEntry {
            filename: file.into(),
            file_type: "RPF_FILE".into(),
            disabled: true,
            ..Default::default()
        }],
        content_change_sets: vec![change_set("TITLE_UPDATE_SET", file, false)],
        ..Default::default()
    };
    write(
        &install.root.join("update/update.rpf"),
        &archive(&[("content.xml", content.to_xml().as_bytes())]),
    );
    for mode in [MapMode::Story, MapMode::Multiplayer] {
        let options = MountOptions {
            mode: Some(mode),
            ..MountOptions::default()
        };
        let game = build_mounted_game_with(&install, crypto(), DlcCutoff::All, options)
            .expect("the synthetic install mounts");
        let update = (game.packs.iter())
            .find(|pack| pack.device_name == "update")
            .expect("the title update registers");
        assert!(update.data_files.iter().all(|f| !f.disabled), "{mode:?}");
        assert_eq!(update.change_sets_unstated, ["TITLE_UPDATE_SET"]);
        assert!(update.change_sets_left_out.is_empty());
    }
}

#[test]
fn with_no_mode_every_change_set_of_a_pack_applies() {
    let (_scratch, install) = install_with_modes();
    assert_eq!(MountOptions::default().mode, None);

    let (enabled, left_out, unstated) = in_mode(&install, None);
    assert_eq!(
        enabled,
        [ALWAYS, MULTIPLAYER_ONLY, STORY_ONLY, ON_DEMAND, IN_NO_GROUP]
    );
    assert!(left_out.is_empty() && unstated.is_empty());
}
