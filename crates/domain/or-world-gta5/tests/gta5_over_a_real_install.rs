//! GTA V's world behind the contract over a real install: the adapter's selection through the
//! core's driver held to `WorldForest`'s own loop at fixed cameras, the rules of the kit that take
//! no built scene, a level of DLC with a kept file of its own, and how much of what a selection
//! draws with reads: its models by how their file holds them and for which target, their
//! textures, and the lights and occluders of its bodies.
//!
//! ⚠ Needs an install: without one each test records a skip and has checked nothing. Each reads
//! every map, type file and bound of the install where no index of it is kept under the build's
//! scratch folder, which is why the suite is in the slow tier.

mod compare;

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider, RpfCryptoProvider};
use or_job::{Cancel, Progress};
use or_title::vfs::{build_mounted_game_with, DlcCutoff, MountOptions};
use or_title::GtaInstall;
use or_world::contract::{GameId, Keys, Opening, WorldInstall, WorldSpec, WorldTitle};
use or_world::driver::Driver;
use or_world::kept::{self, Verdict};
use or_world::kit::{
    self, Fixture, Grounded, HeldBack, InteriorAt, Ladder, Lit, Rule, Scene, Timed,
};
use or_world::model::{
    AssetKey, Element, ElementFormat, Gate, Index, MapRow, Mesh, Moment, Semantic, Switched,
    TextureAsk, EVERY_HOUR,
};
use or_world::slots::OptionValue;
use or_world::streaming::NodeState;
use or_world_gta5::world_forest::read;
use or_world_gta5::world_index::refresh;
use or_world_gta5::{
    Body, Gta5Title, Gta5World, Holding, Linked, Materials, BOX, ENHANCED, LEGACY, LEVEL, TITLE,
};
use or_world_gta5::{MountedWorld, WorldCollision, WorldSource};

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
    let Some(game) = or_title::Game::detect_from_root(root) else {
        or_test_support::skip_install(what, "the folder holds no game");
        return None;
    };
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
    // The index `world_index` builds of the same mount by itself, kept in a file of its own.
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
    // What `WorldCollision` says by itself over a mount of its own: the highest ground of a fixed
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
    // What `WorldForest`'s own loop and the model's reader of a map say of each fixed camera: the
    // instances that stand in an interior, and the far lights and occluders the maps in the
    // forest state.
    let own = Gta5World::over(mounted(&root, &exe), root.clone());
    let policy = own.policy();
    let state = or_world_gta5::WorldState::at(12, "EXTRASUNNY");
    let seen: Vec<(Vec3, usize, usize, usize)> = (CAMERAS.iter())
        .map(|eye| {
            let view = policy.view(*eye, Vec3::ZERO, std::f32::consts::FRAC_PI_4);
            let seen = compare::by_the_forest(&own, &linked.world, &view, &state, false, 1);
            let inside = seen.selections[0].keys().filter(|place| place.1.is_some());
            let inside = match seen.interiors.is_empty() {
                true => 0,
                false => inside.count(),
            };
            let (mut lights, mut occluders) = (0, 0);
            for hash in seen.wanted.keys().filter(|map| !seen.unread.contains(map)) {
                let map = linked.world.map(*hash).expect("a map of the index");
                let bytes = source.open(map.archive, &map.logical).expect("its file");
                let root = or_world_gta5::world_index::parse(&bytes).expect("a map");
                let data = or_world_gta5::MapData::new(&root).expect("a map");
                occluders += data.box_occluder_count() + data.occlude_model_count();
                lights += data.lod_lights().map_or(0, |far| far.len());
            }
            (*eye, inside, lights, occluders)
        })
        .collect();
    let inside = seen.iter().find(|(_, inside, ..)| *inside > 0).copied();
    let lit = (seen.iter().copied())
        .max_by_key(|(_, _, lights, occluders)| (*lights.min(occluders), lights + occluders))
        .expect("a camera");
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
            eye: inside.map_or(near, |(eye, ..)| eye),
            instances: inside.map_or(0, |(_, inside, ..)| inside),
        },
        // No map of GTA V lays out a list, so the rule of the lists is not asked.
        list: None,
        lit: Lit {
            eye: lit.0,
            lights: lit.2,
            occluders: lit.3,
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
    // The rule of the models is asked through `the_models_and_textures_a_selection_draws_with_read`:
    // it holds only where every model and every texture reads, and that suite says which do not.
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
    rules.extend(inside.map(|_| Rule::Interior));
    let mut broken: Vec<String> = (rules.iter())
        .filter_map(|rule| {
            Some(format!(
                "{}: {}",
                rule.name(),
                kit::check(&fixture, *rule, &config()).err()?
            ))
        })
        .collect();
    // ⚠ The rule of the kept index starts from no kept file, so it builds the index of the
    // install afresh in a folder of its own, which is removed.
    let fresh = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a scratch folder");
    rules.push(Rule::KeptLinks);
    if let Err(why) = kit::check(&fixture, Rule::KeptLinks, fresh.path()) {
        broken.push(format!("{}: {why}", Rule::KeptLinks.name()));
    }
    drop(fresh);
    // The rule of the models holds only where every model the scene's places draw with reads
    // with every texture it asks for: asked, and its answer said, without the suite failing by it.
    let models = kit::check(&fixture, Rule::Models, &config());
    println!(
        "{}: the lights and occluders at {}: {} and {}; an interior's instances at {:?}; the rule of the models: {}",
        id.0,
        lit.0,
        lit.2,
        lit.3,
        inside.map(|(eye, inside, ..)| (eye, inside)),
        match &models {
            Ok(()) => "held".to_owned(),
            Err(why) => format!("not held: {why}"),
        }
    );
    assert!(
        lit.2 > 0 && lit.3 > 0,
        "no camera sees a far light and an occluder: {seen:?}"
    );
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

/// How many of some asks read, of how many.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Of {
    read: usize,
    asked: usize,
}

impl Of {
    fn count(&mut self, read: bool) {
        self.asked += 1;
        self.read += usize::from(read);
    }
}

impl std::fmt::Display for Of {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} of {}", self.read, self.asked)
    }
}

/// What the cameras of one install came to, summed.
#[derive(Default)]
struct Read {
    /// By how the file holds the model and the target it is laid out for; `None` where no file was found or its version names no target.
    models: BTreeMap<(Option<Holding>, Option<bool>), Of>,
    unread_models: BTreeMap<&'static str, usize>,
    textures: Of,
    /// The textures answered whose file says their colours are sRGB.
    srgb: usize,
    unread_textures: BTreeMap<String, usize>,
    /// Meshes, those whose colour is the texture their parameter for it binds, those that took the first binding, those with no normal among their elements, and those a position of which is no place of a model, by whether the target is the Enhanced game's.
    meshes: BTreeMap<bool, [usize; 5]>,
    lights: [usize; 2],
    boxes: usize,
    occlude_models: [usize; 2],
    lists: usize,
}

const READS: &str = "the_models_and_textures_a_selection_draws_with_read_at_fixed_cameras";

/// Whether every position of `mesh` stored as three floats is a number within 10 km of its model's origin, which bytes read from the wrong place seldom are.
fn stands_in_its_model(mesh: &Mesh) -> bool {
    let position = |element: &&Element| element.semantic == Semantic::Position;
    let Some(element) = mesh.layout.elements.iter().find(position) else {
        return false;
    };
    if element.format != ElementFormat::F32x3 {
        return true;
    }
    let mut vertices = mesh.vertices.chunks_exact(mesh.layout.stride as usize);
    vertices.all(|vertex| {
        let numbers = vertex[element.offset as usize..][..12].as_chunks::<4>().0;
        (numbers.iter()).all(|four| f32::from_le_bytes(*four).abs() < 1.0e4)
    })
}

#[test]
fn the_models_and_textures_a_selection_draws_with_read_at_fixed_cameras() {
    let Some((root, id, exe)) = install(READS) else {
        return;
    };
    let world = Arc::new(Gta5World::over(mounted(&root, &exe), root.clone()));
    let spec = WorldSpec::of(id, &root);
    let file = kept::file_for(&config(), TITLE, &spec);
    let install: &dyn WorldInstall = &*world;
    let (index, report) = kept::refresh(TITLE, &spec, install, &file).expect("the index builds");
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    let index = Arc::new(index);
    let at = Moment::noon(u64::from(or_io::string_hash(b"EXTRASUNNY")));
    let mut all = Read::default();
    let colour = |enhanced: bool| {
        or_io::string_hash(match enhanced {
            true => b"DiffuseTex".as_slice(),
            false => b"DiffuseSampler".as_slice(),
        })
    };
    for eye in CAMERAS {
        let as_install: Arc<dyn WorldInstall> = world.clone();
        let mut driver = Driver::new(as_install, Arc::clone(&index));
        let view = (driver.policy()).view(eye, Vec3::ZERO, std::f32::consts::FRAC_PI_4);
        let streamed = driver.stream(&view, &at, &Switched::default(), None, usize::MAX);
        let selected = driver.select(&view, &at, &|_: AssetKey| NodeState::Ready);
        for (_, body) in &selected.bodies {
            let own: &dyn Any = &**body;
            let Some(own) = own.downcast_ref::<Body>() else {
                panic!("a body of another title");
            };
            all.lights[0] += body.lights().len();
            all.lights[1] += own.beside.lights_unread as usize;
            all.boxes += body.occluders().boxes.len();
            all.occlude_models[0] += body.occluders().meshes.len();
            all.occlude_models[1] += own.beside.occluders_unread as usize;
            all.lists += own.beside.lists as usize;
        }
        let asks: Vec<AssetKey> = (selected.instances.models.iter().copied())
            .filter(|ask| *ask != BOX)
            .collect();
        let (started, mut outcomes) = (Instant::now(), Vec::new());
        let models = world.read_models(&index, &asks, &mut outcomes);
        let models_took = started.elapsed();
        assert_eq!(outcomes.len(), asks.len());
        let (mut wanted, mut held) = (BTreeSet::new(), 0usize);
        for outcome in &outcomes {
            let enhanced = outcome.target.map(|target| target.is_gen9());
            let of = all.models.entry((outcome.holding, enhanced)).or_default();
            of.count(outcome.model.is_ok());
            let model = match &outcome.model {
                Ok(model) => model,
                Err(why) => {
                    *all.unread_models.entry(why.kind()).or_default() += 1;
                    continue;
                }
            };
            let enhanced = enhanced.expect("a model that read has a target");
            let title = model.title.as_ref().expect("the materials of a model");
            let materials = title.downcast_ref::<Materials>().expect("GTA V's record");
            let counts = all.meshes.entry(enhanced).or_default();
            for (lod, drawn_by) in model.lods.iter().zip(&materials.of) {
                for (mesh, shader) in lod.meshes.iter().zip(drawn_by) {
                    held += mesh.vertices.len() + 4 * mesh.indices.len();
                    wanted.extend(mesh.surface.colour);
                    wanted.extend(mesh.surface.normal);
                    let material = shader.map(|at| &materials.materials[usize::from(at)]);
                    let named = material.is_some_and(|material| {
                        let bound = material.textures.iter();
                        bound
                            .into_iter()
                            .any(|bound| bound.parameter == colour(enhanced))
                    });
                    let normal = |element: &Element| element.semantic == Semantic::Normal;
                    counts[0] += 1;
                    counts[1] += usize::from(named);
                    counts[2] += usize::from(!named && mesh.surface.colour.is_some());
                    counts[3] += usize::from(!mesh.layout.elements.iter().any(normal));
                    counts[4] += usize::from(!stands_in_its_model(mesh));
                }
            }
        }
        drop(outcomes);
        let wanted: Vec<TextureAsk> = wanted.into_iter().collect();
        let (started, mut textures) = (Instant::now(), Vec::new());
        let dictionaries = world.read_textures(&index, &wanted, &mut textures);
        let textures_took = started.elapsed();
        assert_eq!(textures.len(), wanted.len());
        let mut answered = 0;
        for (_, texture) in &textures {
            all.textures.count(texture.is_ok());
            match texture {
                Ok(texture) => {
                    answered += 1;
                    all.srgb += usize::from(texture.srgb);
                    held += texture.levels.iter().map(Vec::len).sum::<usize>();
                }
                Err(why) => {
                    let reason = match why {
                        or_world_gta5::NoTexture::Format(format) => {
                            format!("{}: {format}", why.kind())
                        }
                        or_world_gta5::NoTexture::NotInChain { walked, .. } => {
                            format!("{}, of a chain of {walked}", why.kind())
                        }
                        why => why.kind().to_owned(),
                    };
                    *all.unread_textures.entry(reason).or_default() += 1;
                }
            }
        }
        println!(
            "{} at ({}, {}, {}): {} maps in the forest ({} unread), {} instances; {} models asked, {} files of {} bytes in {:.1} s; {} textures asked, {} answered, {} files of {} bytes in {:.1} s; {} bytes held",
            id.0,
            eye.x,
            eye.y,
            eye.z,
            streamed.wanted.len(),
            streamed.unread.len(),
            selected.instances.len(),
            asks.len(),
            models.files,
            models.bytes,
            models_took.as_secs_f64(),
            wanted.len(),
            answered,
            dictionaries.files,
            dictionaries.bytes,
            textures_took.as_secs_f64(),
            held,
        );
    }
    let total = all.models.values().fold(Of::default(), |sum, of| Of {
        read: sum.read + of.read,
        asked: sum.asked + of.asked,
    });
    println!("{}: models read, {total}:", id.0);
    for ((holding, enhanced), of) in &all.models {
        println!("  {holding:?}, Enhanced layout {enhanced:?}: {of}");
    }
    println!("  not read, by reason: {:?}", all.unread_models);
    println!(
        "{}: textures answered, {}, of which {} state sRGB; not answered, by reason: {:?}",
        id.0, all.textures, all.srgb, all.unread_textures
    );
    for (enhanced, [meshes, named, first, no_normal, misplaced]) in &all.meshes {
        println!(
            "  Enhanced layout {enhanced}: {meshes} meshes, the colour of {named} by its parameter's name and of {first} by the first binding, {no_normal} with no normal among their elements, {misplaced} with a position that is no place of a model"
        );
        assert_eq!(
            *misplaced, 0,
            "meshes whose positions are no numbers of a model"
        );
    }
    println!(
        "{}: far lights stated {} and not stated {}; box occluders {}; occlude models {} and unread {}; instance lists not laid out {}",
        id.0,
        all.lights[0],
        all.lights[1],
        all.boxes,
        all.occlude_models[0],
        all.occlude_models[1],
        all.lists
    );
    // A model of each holding reads, and a texture: what does not is said above, with why.
    for holding in Holding::ALL {
        let read: usize = (all.models.iter())
            .filter(|((held, _), _)| *held == Some(holding))
            .map(|(_, of)| of.read)
            .sum();
        assert!(read > 0, "no model held as {holding:?} read");
    }
    // ⚠ A share, not a count: a fault in how a file binds its textures answered two in three of
    // them on the Enhanced game while every model still read.
    assert!(
        total.read * 100 >= total.asked * 99,
        "{total} models read: {:?}",
        all.unread_models
    );
    assert!(
        all.textures.read * 20 >= all.textures.asked * 19,
        "{} textures answered: {:?}",
        all.textures,
        all.unread_textures
    );
    assert!(all.lights[0] > 0 && all.boxes > 0 && all.occlude_models[0] > 0);
    assert_eq!(
        (all.lights[1], all.occlude_models[1]),
        (0, 0),
        "far lights and occlude models a body left unread"
    );
}
