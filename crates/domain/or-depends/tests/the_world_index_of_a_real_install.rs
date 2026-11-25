//! The world index of a real install, held to everything its mount lists, and the ground under the
//! vehicle nodes of its own path regions.
//!
//! ⚠ Needs an install: without one each test records a skip and has checked nothing. Each reads
//! every map, type file and bound of the install once, which is why it is in the slow tier.

mod world_fixture;

use std::path::PathBuf;
use std::sync::Arc;

use or_depends::{MountedWorld, WorldIndex};
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::GtaInstall;
use world_fixture::{ground_at_the_road_nodes, held_to_its_mount, NODE_SCALES, STEP};

/// The first install of the machine and the keys of its executable; without either, `what` is
/// recorded as skipped.
fn install(what: &str) -> Option<(PathBuf, Arc<dyn RpfCryptoProvider>)> {
    let roots = or_test_support::game_roots();
    let Some(root) = roots.first() else {
        or_test_support::skip_install(
            what,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return None;
    };
    let exe = or_title::Game::detect_from_root(root).find_executable(root);
    let Some(exe) = exe else {
        or_test_support::skip_install(what, "no executable, so no keys");
        return None;
    };
    let Ok(keys) = or_io::container::rpf::crypto::extract_keys(&exe, &|_| {}) else {
        or_test_support::skip_install(what, "key extraction failed");
        return None;
    };
    if let Some(path) = or_test_support::schema_snapshot() {
        if let Err(e) = or_io::meta::install_snapshot(&path) {
            panic!(
                "the staged schema snapshot {} is unusable: {e}",
                path.display()
            );
        }
    }
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(or_io::container::rpf::crypto::Gta5RetailCryptoProvider::new(Arc::new(keys)));
    Some((root.clone(), crypto))
}

const WHAT: &str = "the_world_index_names_everything_a_real_install_lists";

#[test]
fn the_world_index_names_everything_a_real_install_lists() {
    let Some((root, crypto)) = install(WHAT) else {
        return;
    };
    let mount = || {
        build_mounted_game(
            &GtaInstall { root: root.clone() },
            Arc::clone(&crypto),
            DlcCutoff::All,
        )
        .expect("the install mounts")
    };
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let held = held_to_its_mount(&mount, &root, &scratch.path().join("world.index"));
    assert!(held.maps > 0 && held.type_files > 0, "{held:?}");
}

const ROADS: &str = "the_ground_under_a_real_installs_road_nodes_is_within_a_step_of_them";

/// The least share of the vehicle nodes asked whose ground must be within a step of them.
///
/// ⚠ A floor set before any install was asked, not a measurement. It is under one because a
/// vehicle node may run over water, and over a bridge that is a placed prop, whose collision is
/// not static and is not loaded.
const SHARE: f64 = 0.75;

/// ⚠ Needs an install. It also holds the reading of a node's stored integers to the ground: the
/// scale asked to hold must put more nodes on the ground than any rival does.
#[test]
fn the_ground_under_a_real_installs_road_nodes_is_within_a_step_of_them() {
    let Some((root, crypto)) = install(ROADS) else {
        return;
    };
    let install = GtaInstall { root: root.clone() };
    let game = build_mounted_game(&install, crypto, DlcCutoff::All).expect("the install mounts");
    let source = MountedWorld::new(&game, &root);
    let (index, _) = WorldIndex::build(&source, None);

    // Every sixteenth vehicle node of every region at the scale asked to hold, and a quarter
    // of those at each rival.
    let held = ground_at_the_road_nodes(&game, &index, &source, NODE_SCALES[0], 16);
    eprintln!(
        "\n=== at {:?} units a metre: {held:?}, {STEP} m a step ===",
        NODE_SCALES[0]
    );
    assert!(held.regions > 0 && held.asked > 1000, "{held:?}");
    let share = |held: &world_fixture::NodesHeld| held.within_a_step as f64 / held.asked as f64;
    assert!(
        share(&held) >= SHARE,
        "{:.3} of the nodes asked have ground within a step: {held:?}",
        share(&held)
    );
    for rival in &NODE_SCALES[1..] {
        let other = ground_at_the_road_nodes(&game, &index, &source, *rival, 64);
        eprintln!("=== at {rival:?} units a metre: {other:?} ===");
        assert!(
            share(&other) < share(&held) * 0.5,
            "the rival {rival:?} puts {:.3} of its nodes on the ground",
            share(&other)
        );
    }
}
