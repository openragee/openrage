//! GTA V's world behind the contract over a real install: the adapter's selection through the
//! core's driver held to `or_depends`' own forest at fixed cameras, the rules of the kit that take
//! no built scene, and a level of DLC with a kept file of its own.
//!
//! ⚠ Needs an install: without one each test records a skip and has checked nothing. Each reads
//! every map, type file and bound of the install where no index of it is kept under the build's
//! scratch folder, which is why the suite is in the slow tier.

mod compare;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;
use or_depends::world_forest::read;
use or_depends::world_index::refresh;
use or_depends::{MountedWorld, WorldCollision};
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider, RpfCryptoProvider};
use or_job::{Cancel, Progress};
use or_title::vfs::{build_mounted_game_with, DlcCutoff, MountOptions};
use or_title::GtaInstall;
use or_world::contract::{GameId, Keys, Opening, WorldInstall, WorldSpec, WorldTitle};
use or_world::kept::{self, Verdict};
use or_world::kit::{
    self, Fixture, Grounded, HeldBack, InteriorAt, Ladder, Lit, Rule, Scene, Timed,
};
use or_world::model::{Gate, Index, MapRow, EVERY_HOUR};
use or_world::slots::OptionValue;
use or_world_gta5::{Gta5Title, Gta5World, Linked, ENHANCED, LEGACY, LEVEL, TITLE};

/// The cameras every install is asked about, each at noon and at midnight.
const CAMERAS: [Vec3; 8] = [
    Vec3::new(0.0, 0.0, 72.0),
    Vec3::new(-75.0, -820.0, 326.0),
    Vec3::new(-1040.0, -2740.0, 20.0),
    Vec3::new(1700.0, 3250.0, 41.0),
    Vec3::new(-1100.0, 4900.0, 220.0),
    Vec3::new(2500.0, -400.0, 95.0),
    Vec3::new(130.0, 6600.0, 32.0),
    Vec3::new(-3000.0, 30.0, 10.0),
];

/// The first install of the machine, its game and its executable; without them, `what` is recorded as skipped.
fn install(what: &str) -> Option<(PathBuf, GameId, PathBuf)> {
    let roots = or_test_support::game_roots();
    let Some(root) = roots.first() else {
        or_test_support::skip_install(
            what,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return None;
    };
    let game = or_title::Game::detect_from_root(root);
    let Some(exe) = game.find_executable(root) else {
        or_test_support::skip_install(what, "no executable, so no keys");
        return None;
    };
    let id = [LEGACY, ENHANCED]
        .into_iter()
        .find(|id| id.0 == game.profile().id());
    let Some(id) = id else {
        or_test_support::skip_install(what, "the install is no GTA V");
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
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = std::fs::canonicalize(root).expect("the install's folder");
    Some((root, id, exe))
}

/// Where the indexes of the install suites are kept from one run to the next: under the build's own scratch folder.
fn config() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("gta5-over-a-real-install")
}

/// `world` opened through the title with the keys of `exe`, its index from the kept file under [`config`].
fn opened(world: &WorldSpec, exe: &Path) -> (Arc<dyn WorldInstall>, Index, kept::Report, PathBuf) {
    let (cancel, progress, config) = (Cancel::new(), Progress::new(), config());
    let opening = Opening {
        config: &config,
        keys: Keys::Executable(exe),
        cancel: &cancel,
        progress: &progress,
    };
    let install = Gta5Title.open(world, &opening).expect("the world opens");
    let file = kept::file_for(&config, TITLE, world);
    let (index, report) = kept::refresh(TITLE, world, &*install, &file).expect("the index builds");
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    (install, index, report, file)
}

const SELECTS: &str = "the_adapter_selects_as_the_forest_does_at_fixed_cameras_of_a_real_install";

#[test]
fn the_adapter_selects_as_the_forest_does_at_fixed_cameras_of_a_real_install() {
    let Some((root, id, exe)) = install(SELECTS) else {
        return;
    };
    let keys = extract_keys(&exe, &|_| {}).expect("the keys of the executable");
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    let mount = || {
        let install = GtaInstall { root: root.clone() };
        let all = DlcCutoff::All;
        build_mounted_game_with(
            &install,
            Arc::clone(&crypto),
            all,
            MountOptions::install_only(),
        )
        .expect("the install mounts")
    };
    let world = Arc::new(Gta5World::over(mount(), root.clone()));
    let spec = WorldSpec::of(id, &root);
    let file = kept::file_for(&config(), TITLE, &spec);
    let install: &dyn WorldInstall = &*world;
    let (index, report) = kept::refresh(TITLE, &spec, install, &file).expect("the index builds");
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    let index = Arc::new(index);
    // The index `or_depends` builds of the same mount by itself, kept in a file of its own.
    let source = MountedWorld::new(world.game(), &root);
    let (reference, _) = refresh(&source, &config().join(format!("{}.reference", id.0)));
    let linked = index.title::<Linked>().expect("a GTA V link");
    assert_eq!(
        (
            linked.world.maps().len(),
            linked.world.archetype_count(),
            linked.world.bounds().len()
        ),
        (
            reference.maps().len(),
            reference.archetype_count(),
            reference.bounds().len()
        ),
    );
    assert!(
        linked.world.seal().ok() == reference.seal().ok(),
        "the rows joined from the kept sections are not those `or_depends` builds of the mount"
    );
    let (mut drawn, mut differing) = (Vec::new(), Vec::new());
    for eye in CAMERAS {
        for hour in [12, 0] {
            match compare::compared(&world, &index, &reference, eye, (hour, "EXTRASUNNY")) {
                Ok(instances) => drawn.push(instances),
                Err(differs) => differing.push(differs),
            }
        }
    }
    println!(
        "{}: {} maps; instances drawn at the {} cameras, noon then midnight: {drawn:?}",
        id.0,
        index.maps().len(),
        CAMERAS.len()
    );
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    assert!(
        drawn.iter().filter(|count| **count > 0).count() >= CAMERAS.len(),
        "{drawn:?}"
    );
}

/// The centre of the box `row` streams by.
fn centre(row: &MapRow) -> Option<Vec3> {
    let (min, max) = row.boxes?.streaming;
    Some((min + max) / 2.0)
}

const RULES: &str = "the_rules_of_the_kit_that_take_no_built_scene_hold_over_a_real_install";

#[test]
fn the_rules_of_the_kit_that_take_no_built_scene_hold_over_a_real_install() {
    let Some((root, id, exe)) = install(RULES) else {
        return;
    };
    let spec = WorldSpec::of(id, &root);
    let (_, index, _, _) = opened(&spec, &exe);
    let maps = index.maps();
    // A child map nothing holds back, with the map it names as its parent.
    let child = (maps.iter())
        .find(|row| !row.parents.is_empty() && row.gate == Gate::Always && centre(row).is_some())
        .expect("a child map that streams by its box");
    let parent = index.map(child.parents[0]).expect("its parent's row");
    let near = centre(child).expect("its box");
    let timed = maps.iter().find_map(|row| match &row.gate {
        Gate::When { hours, weathers } if weathers.is_empty() && *hours != EVERY_HOUR => {
            let on = (0..24).find(|hour| hours & (1 << hour) != 0)?;
            let off = (0..24).find(|hour| hours & (1 << hour) == 0)?;
            Some((row, on as f32, off as f32))
        }
        _ => None,
    });
    let held = maps.iter().find_map(|row| match row.gate {
        Gate::Held {
            switch: Some(switch),
        } => Some((row, centre(row)?, switch)),
        _ => None,
    });
    let (held_row, held_eye, switch) = held.expect("a map only a request lets in");
    // What `or_depends` says by itself over a mount of its own: the highest ground of a fixed
    // camera's column, and how many entities the child map places.
    let game = mounted(&root, &exe);
    let source = MountedWorld::new(&game, &root);
    let linked = index.title::<Linked>().expect("a GTA V link");
    let column = CAMERAS[2];
    let slope = std::f32::consts::FRAC_PI_4;
    let top = WorldCollision::new(&linked.world, &source, 0)
        .highest_ground(column.x, column.y, slope)
        .ground
        .map(|ground| ground.hit.position.z)
        .expect("ground under the camera");
    let places = u32::try_from(child.name)
        .ok()
        .and_then(|hash| linked.world.map(hash))
        .and_then(|map| read::map(&linked.world, &source, map).ok())
        .map(|read| read.placed.len())
        .expect("the child map reads");
    let scene = Scene {
        weather: u64::from(or_io::string_hash(b"EXTRASUNNY")),
        nowhere: Vec3::new(1.0e6, 1.0e6, 0.0),
        ladder: Ladder {
            parent_map: parent.name,
            child_map: child.name,
            far: near,
            near,
        },
        timed: Timed {
            map: timed.map_or(child.name, |(row, ..)| row.name),
            eye: timed.and_then(|(row, ..)| centre(row)).unwrap_or(near),
            on: timed.map_or(0.0, |(_, on, _)| on),
            off: timed.map_or(0.0, |(.., off)| off),
        },
        held: HeldBack {
            map: held_row.name,
            eye: held_eye,
            switch,
        },
        interior: InteriorAt {
            eye: near,
            instances: 0,
        },
        list: None,
        lit: Lit {
            eye: near,
            lights: 0,
            occluders: 0,
        },
        placed: (child.name, places),
        ground: Grounded {
            at: Vec3::new(column.x, column.y, top + 5.0),
            height: top,
        },
    };
    let fixture = Fixture {
        title: Box::new(Gta5Title),
        world: spec,
        scene,
        edit: None,
        keeps: None,
        executable: Some(exe),
    };
    let mut rules = vec![
        Rule::Policy,
        Rule::Frame,
        Rule::Slots,
        Rule::Streams,
        Rule::Draws,
        Rule::Columns,
        Rule::Lit,
        Rule::ChildNamesParent,
        Rule::HeldMap,
        Rule::Caps,
    ];
    rules.extend(timed.map(|_| Rule::TimedMap));
    let broken: Vec<String> = (rules.iter())
        .filter_map(|rule| {
            Some(format!(
                "{}: {}",
                rule.name(),
                kit::check(&fixture, *rule, &config()).err()?
            ))
        })
        .collect();
    println!(
        "{}: {} rules of the kit asked, a map by the clock among them: {}; the ground under ({}, {}) stands at {top}, and the child map places {places}",
        id.0,
        rules.len(),
        timed.is_some(),
        column.x,
        column.y
    );
    assert!(broken.is_empty(), "{}", broken.join("\n"));
}

const LEVELS: &str =
    "a_level_of_a_real_install_keeps_an_index_of_its_own_and_a_second_start_reads_no_archive";

#[test]
fn a_level_of_a_real_install_keeps_an_index_of_its_own_and_a_second_start_reads_no_archive() {
    let Some((root, id, exe)) = install(LEVELS) else {
        return;
    };
    let whole = WorldSpec::of(id, &root);
    let (install, every, _, every_file) = opened(&whole, &exe);
    // The pack the list names in its middle, by the name the list gives it.
    let world = (every.title::<Linked>()).expect("a GTA V link");
    let _ = world;
    let listed = packs_listed(&root, &exe);
    let pack = listed.get(listed.len() / 2).expect("a DLC list with packs");
    let level = WorldSpec {
        options: vec![(LEVEL, OptionValue::Named(pack.clone()))],
        ..WorldSpec::of(id, &root)
    };
    let (at_level, fewer, _, level_file) = opened(&level, &exe);
    assert_ne!(every_file, level_file);
    assert!(every_file.is_file() && level_file.is_file());
    assert!(
        fewer.maps().len() < every.maps().len(),
        "{} maps at the level and {} with every pack",
        fewer.maps().len(),
        every.maps().len()
    );
    for (world, archives) in [
        (&whole, install.inputs().archives),
        (&level, at_level.inputs().archives),
    ] {
        assert!(archives.iter().all(|archive| archive.stamp.is_some()));
        let (_, _, again, _) = opened(world, &exe);
        assert!(matches!(again.verdict, Verdict::Current), "{again:?}");
        assert_eq!((again.read.len(), again.derived), (0, false));
    }
    println!(
        "{}: {} maps with every pack, {} at the pack in the middle of the list",
        id.0,
        every.maps().len(),
        fewer.maps().len()
    );
}

/// The install at `root` mounted with every pack, with the keys of `exe`.
fn mounted(root: &Path, exe: &Path) -> or_title::MountedGame {
    let keys = extract_keys(exe, &|_| {}).expect("the keys of the executable");
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    let install = GtaInstall {
        root: root.to_path_buf(),
    };
    build_mounted_game_with(
        &install,
        crypto,
        DlcCutoff::All,
        MountOptions::install_only(),
    )
    .expect("the install mounts")
}

/// The packs the DLC list of the install at `root` names, in its order.
fn packs_listed(root: &Path, exe: &Path) -> Vec<String> {
    let game = mounted(root, exe);
    game.listed.iter().map(|pack| pack.name.clone()).collect()
}
