//! The world index of a real install, held to everything its mount lists, the ground under the
//! vehicle nodes of its own path regions, its own models found by name with each placement
//! held to the map file it names, and what the level-of-detail forest selects at places of its
//! own maps.
//!
//! ⚠ Needs an install: without one each test records a skip and has checked nothing. Each reads
//! every map, type file and bound of the install once, which is why it is in the slow tier.

mod forest_fixture;
mod world_fixture;

use std::path::PathBuf;
use std::sync::Arc;

use or_depends::world_forest::{request_of, Request};
use or_depends::{MountedWorld, WorldForest, WorldIndex};
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::GtaInstall;
use world_fixture::{
    ground_at_the_road_nodes, held_to_its_mount, models_held_to_their_maps, stated_by_the_files,
    NODE_SCALES, STEP,
};

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
    let exe = or_title::Game::detect_from_root(root).and_then(|game| game.find_executable(root));
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

const STATED: &str = "what_the_files_of_a_real_install_state_where_the_index_takes_one_reading";

/// ⚠ Needs an install. A measure of three readings the index takes of an install's files: a map
/// is named by its file's stem, the last type file to declare an archetype is believed, and the
/// box of an interior's bound is about the interior.
#[test]
fn what_the_files_of_a_real_install_state_where_the_index_takes_one_reading() {
    let Some((root, crypto)) = install(STATED) else {
        return;
    };
    let install = GtaInstall { root: root.clone() };
    let game = build_mounted_game(&install, crypto, DlcCutoff::All).expect("the install mounts");
    let (index, _) = WorldIndex::build(&MountedWorld::new(&game, &root), None);
    let stated = stated_by_the_files(&game, &index);
    eprintln!(
        "\n=== {stated:?}; {} redefinition(s) in mount order ===",
        index.redefined()
    );
    assert!(stated.maps > 0 && stated.interior_bounds > 0, "{stated:?}");
    // A parent is found by the stem of its file, and an interior's bound is no place of the world.
    assert!(
        stated.parent_not_held * 100 <= stated.with_parent,
        "{stated:?}"
    );
    assert!(
        stated.nearer_a_placement * 10 <= stated.interior_bounds,
        "{stated:?}"
    );
    assert_eq!(stated.answered_at_a_position, 0, "{stated:?}");
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

const FIND: &str =
    "a_real_installs_own_models_are_found_by_name_and_each_placement_stands_in_the_map_it_names";

/// How many models are asked for, spread over the names the install states.
const HANDFUL: usize = 8;

/// The least share of the placements read that must lie inside the streaming extents of their map.
///
/// ⚠ A floor set before any install was asked, not a measurement: a map is expected to stream from
/// a box that holds what it places, and nothing here has yet counted the maps that do not.
const INSIDE: f64 = 0.9;

/// ⚠ Needs an install. The names are the install's own: archetypes its type files declare, that a
/// file of it names and that a map of it places. Each is found by its name, and each placement
/// listed is held to the entity at that index of the map file it names, read again.
#[test]
fn a_real_installs_own_models_are_found_by_name_and_each_placement_stands_in_the_map_it_names() {
    let Some((root, crypto)) = install(FIND) else {
        return;
    };
    let install = GtaInstall { root: root.clone() };
    let game = build_mounted_game(&install, crypto, DlcCutoff::All).expect("the install mounts");
    let source = MountedWorld::new(&game, &root);
    let held = models_held_to_their_maps(&source, HANDFUL);
    eprintln!("\n=== {held:?} ===");
    assert_eq!(held.asked, HANDFUL, "{held:?}");
    assert!(held.placements >= HANDFUL, "{held:?}");
    let share = held.inside as f64 / held.placements as f64;
    assert!(
        share >= INSIDE,
        "{share:.3} of the placements read lie inside their map's streaming extents: {held:?}"
    );
}

const FOREST: &str = "the_forest_of_a_real_install_selects_what_its_index_holds_and_no_child_whole_beside_a_parent_that_holds_the_children_it_states";

/// How many places are asked: the centres of that many maps, spread evenly over the index.
const PLACES: usize = 24;

/// The bytes of maps the forest may hold let go of, so a place reads again part of what the last one did.
const FOREST_BUDGET: u64 = 64 << 20;

/// ⚠ Needs an install. A view stands at the centre of the entities' extents of maps spread over the
/// index, half of them maps that place an interior, at noon in clear weather with every model
/// taken as loaded.
///
/// ⚠ A parent gives way once as many children as it states are in the forest, so one that states
/// another count than name it draws whole beside them. An install ships such parents, and each
/// child beside one is counted; a child whole beside a parent that holds the children it states
/// is what the rules do not give, and is the failure.
#[test]
fn the_forest_of_a_real_install_selects_what_its_index_holds_and_no_child_whole_beside_a_parent_that_holds_the_children_it_states(
) {
    let Some((root, crypto)) = install(FOREST) else {
        return;
    };
    let install = GtaInstall { root: root.clone() };
    let game = build_mounted_game(&install, crypto, DlcCutoff::All).expect("the install mounts");
    let source = MountedWorld::new(&game, &root);
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, FOREST_BUDGET);

    // Maps that stream wherever their box is wanted, so the view at their centre wants them.
    let index = &index;
    let always = |with_interior: bool| {
        let maps = index.maps().iter().filter(move |map| {
            map.read
                && map.entities.is_some()
                && map.interiors.is_empty() != with_interior
                && request_of(index, map) == Request::Always
        });
        maps.collect::<Vec<_>>()
    };
    let mut places = Vec::new();
    for with_interior in [false, true] {
        let maps = always(with_interior);
        let step = (maps.len() / (PLACES / 2)).max(1);
        places.extend(maps.into_iter().step_by(step).take(PLACES / 2));
    }
    assert!(places.len() > PLACES / 2, "{} place(s)", places.len());

    let held = forest_fixture::selected_at_the_centres(index, &mut forest, &places);
    let (unleveled, lost) = forest.misfits();
    eprintln!(
        "\n=== {held:?}; at the last place {unleveled} entit(ies) state no level and {lost} name \
         a parent in a map the mount lacks ==="
    );
    assert!(held.instances > 0, "nothing was selected at any place");
    assert_eq!(
        held.unaccounted,
        Vec::<String>::new(),
        "a child draws whole beside a whole parent that holds the children it states: {held:?}"
    );
    assert_eq!(
        held.beside,
        held.states_more + held.states_fewer,
        "{held:?}"
    );
}
