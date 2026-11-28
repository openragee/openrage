//! A world session with no window: which game's session an install opens, that two share no
//! state, that the selection handed to a frame is the core driver's, and that what does not read
//! is counted with its reason.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::Vec3;
use or_job::{Cancel, Progress};
use or_world::caps::Capability;
use or_world::contract::{Keys, Opening, WorldTitle};
use or_world::driver::Driver;
use or_world::fake::{self, FakeTitle, Fault};
use or_world::kept;
use or_world::model::{AssetKey, Moment, Switched};
use or_world::streaming::NodeState;

use super::driven::{DrivenSession, OwnedKeys};
use super::{gta6, Kind, Reason, Selection, Standing, WorldKey, WorldSession};
use crate::state::prefs::{Game, Prefs};
use crate::state::AppState;

/// How long a test waits for a session's threads before it fails.
const PATIENCE: Duration = Duration::from_secs(20);
/// How many polls in a row must find nothing new for a session to count as at rest.
const QUIET: u32 = 12;

/// A session over the fake title `title`, its index kept in a scratch folder that lives as long as the session is used.
fn over(title: FakeTitle, install: &str) -> (DrivenSession, tempfile::TempDir) {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let key = WorldKey {
        game: Game::GtaVLegacy,
        install: PathBuf::from(install),
        options: Vec::new(),
    };
    // A title of the registry lives as long as the process; so does this one.
    let title: &'static FakeTitle = Box::leak(Box::new(title));
    let config = scratch.path().to_path_buf();
    let session = DrivenSession::open(key, title, FakeTitle::spec(), OwnedKeys::None, config);
    (session, scratch)
}

/// Where each model the session handed over ended up, as a frame would tell it.
#[derive(Default)]
struct Device {
    there: Vec<AssetKey>,
    never: Vec<AssetKey>,
}

/// Poll `session` with its camera at `eye`, answering for each model as a device that takes every one, until nothing is waited for and nothing changes.
fn at_rest(session: &mut DrivenSession, eye: Vec3, device: &mut Device) -> Arc<Selection> {
    let mut camera = session.camera();
    camera.position = eye;
    session.set_camera(camera);
    let started = Instant::now();
    let (mut quiet, mut was) = (0, 0);
    loop {
        session.poll();
        assert!(
            !matches!(session.standing(), Standing::Failed(_)),
            "{:?}",
            session.standing()
        );
        let mut arrived = false;
        if let Some(feed) = session.feed() {
            while let Some((model, read)) = feed.take_model() {
                arrived = true;
                match read {
                    Some(_) => {
                        device.there.push(model);
                        feed.tell(model, NodeState::Ready);
                    }
                    None => device.never.push(model),
                }
            }
            while feed.take_texture().is_some() {}
        }
        let selection = session.feed().and_then(|feed| feed.selection());
        let version = selection.as_ref().map_or(0, |selection| selection.version);
        let still = selection
            .as_ref()
            .is_some_and(|selection| !selection.fading);
        quiet = match still && !arrived && version == was && !session.waiting().any() {
            true => quiet + 1,
            false => 0,
        };
        was = version;
        if let (true, Some(selection)) = (quiet >= QUIET, selection) {
            return selection;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "the session was not at rest: {:?}, waiting for {:?}",
            session.standing(),
            session.waiting()
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn the_selection_a_session_hands_a_frame_is_the_drivers() {
    let (mut session, scratch) = over(FakeTitle::new(), "fake-install");
    // One device for the whole flight: a model put there at one eye is there at the next.
    let mut device = Device::default();
    for eye in [fake::NEAR_EYE, fake::FAR_EYE, fake::YARD_EYE] {
        let handed = at_rest(&mut session, eye, &mut device);
        assert!(!handed.instances.is_empty(), "nothing draws at {eye}");

        // The core's driver, by itself, over a world of its own opened the same way.
        let title = FakeTitle::new();
        let (cancel, progress) = (Cancel::new(), Progress::new());
        let opening = Opening {
            config: scratch.path(),
            keys: Keys::None,
            cancel: &cancel,
            progress: &progress,
        };
        let spec = FakeTitle::spec();
        let install = title.open(&spec, &opening).expect("the fake opens");
        let file = scratch.path().join("the-drivers-own.index");
        let (index, _) = kept::refresh(title.id(), &spec, &*install, &file).expect("it links");
        let mut driver = Driver::new(Arc::clone(&install), Arc::new(index));
        let view = install
            .policy()
            .view(eye, Vec3::ZERO, session.camera().fov_y);
        let moment = Moment::noon(0);
        let stands = |model: AssetKey| match device.there.contains(&model) {
            true => NodeState::Ready,
            false => NodeState::Loading,
        };
        let started = Instant::now();
        let own = loop {
            driver.stream(&view, &moment, &Switched::default(), None, usize::MAX);
            let selected = driver.select(&view, &moment, &stands);
            if !selected.fading && selected.waiting.is_empty() {
                break selected;
            }
            assert!(started.elapsed() < PATIENCE, "the driver was not at rest");
            std::thread::sleep(Duration::from_millis(15));
        };
        assert_eq!(handed.instances, own.instances, "at {eye}");
    }
}

#[test]
fn two_sessions_share_no_state() {
    let (mut near, _a) = over(FakeTitle::new(), "one-install");
    let (mut nowhere, _b) = over(FakeTitle::new(), "another-install");
    {
        use super::WorldFeed;
        assert_ne!(WorldFeed::id(&near), WorldFeed::id(&nowhere));
    }
    near.set_hour(3.0);
    let (mut there, mut nothing) = (Device::default(), Device::default());
    let drawn = at_rest(&mut near, fake::NEAR_EYE, &mut there);
    let empty = at_rest(&mut nowhere, fake::NOWHERE, &mut nothing);
    assert!(!drawn.instances.is_empty() && !there.there.is_empty());
    // What one session streamed, was handed and was told is nothing of the other's.
    assert!(empty.instances.is_empty(), "{}", empty.instances.len());
    assert!(nothing.there.is_empty() && nothing.never.is_empty());
    assert_eq!((near.hour(), nowhere.hour()), (3.0, 12.0));
    assert_eq!(near.camera().position, fake::NEAR_EYE);
    assert_eq!(nowhere.camera().position, fake::NOWHERE);
    // The second, moved to where the first stands, is handed its own models: none came from the first's device.
    let second = at_rest(&mut nowhere, fake::NEAR_EYE, &mut nothing);
    assert_eq!(second.instances, drawn.instances);
    let mut handed = (nothing.there.clone(), there.there.clone());
    handed.0.sort();
    handed.1.sort();
    assert_eq!(handed.0, handed.1);
}

#[test]
fn a_model_that_does_not_read_is_counted_with_its_reason() {
    let (mut session, _scratch) = over(FakeTitle::broken(Fault::ModelUnread), "fake-install");
    let mut device = Device::default();
    for eye in [fake::NEAR_EYE, fake::FAR_EYE] {
        at_rest(&mut session, eye, &mut device);
    }
    assert_eq!(
        device.never,
        [fake::SLAB],
        "the slab is the model that fault leaves unread"
    );
    let unread = session.unread();
    let models: Vec<_> = (unread.iter())
        .filter(|unread| unread.kind == Kind::Model)
        .collect();
    assert_eq!(models.len(), 1, "{unread:?}");
    assert_eq!(models[0].count, 1);
    let Reason::Said(why) = &models[0].why else {
        panic!("a model's reason is a sentence: {:?}", models[0].why);
    };
    assert_eq!(why.key(), "The title read no model and gave no reason");
    assert_eq!(models[0].why.shown(), crate::i18n::tr(why.key()));
    // A whole world has nothing unread.
    let (mut whole, _scratch) = over(FakeTitle::new(), "fake-install");
    at_rest(&mut whole, fake::NEAR_EYE, &mut Device::default());
    assert_eq!(whole.unread(), []);
}

#[test]
fn a_session_states_the_capabilities_of_the_world_its_title_opened() {
    let (mut session, scratch) = over(FakeTitle::new(), "fake-install");
    assert_eq!(
        session.caps(),
        or_world::caps::WorldCaps::none(),
        "none until it is open"
    );
    at_rest(&mut session, fake::NOWHERE, &mut Device::default());
    let title = FakeTitle::new();
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config: scratch.path(),
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    let own = title
        .open(&FakeTitle::spec(), &opening)
        .expect("the fake opens");
    assert_eq!(session.caps(), own.caps());
    assert_eq!(session.option_kinds(), title.options());
    // GTA VI's shell states GTA VI's, which are not these, and hands the bridge no feed.
    let mut gta6 = gta6::Gta6Session::new(WorldKey {
        game: Game::GtaVI,
        install: PathBuf::from("a-build"),
        options: Vec::new(),
    });
    assert_eq!(gta6.caps(), gta6::CAPS);
    assert!(gta6.caps().has(Capability::Ground) && !gta6.caps().has(Capability::Edits));
    assert_ne!(gta6.caps(), session.caps());
    assert!(gta6.feed().is_none() && gta6.option_kinds().is_empty());
}

#[test]
fn the_active_install_decides_whose_world_is_opened_and_another_drops_the_one_held() {
    let folders = tempfile::tempdir().expect("a scratch folder");
    let (gta5, gta6) = (folders.path().join("five"), folders.path().join("six"));
    for folder in [&gta5, &gta6] {
        std::fs::create_dir_all(folder).expect("a folder is made");
    }
    let mut state = AppState::from_prefs(Prefs::default());
    assert!(state.world_session().is_none(), "no install, no session");

    // A GTA VI folder alone: its world, as before any other title had one.
    state.prefs.game_paths.set(Game::GtaVI, Some(gta6.clone()));
    let key = state.world_session().map(|session| session.key().clone());
    assert_eq!(
        key.map(|key| (key.game, key.install)),
        Some((Game::GtaVI, gta6.clone()))
    );
    let caps = state.world_session().map(|session| session.caps());
    assert_eq!(caps, Some(gta6::CAPS));
    assert_eq!(state.world.opened, 1);
    state.world_session();
    assert_eq!(
        state.world.opened, 1,
        "the session held is the one asked for again"
    );

    // GTA V made the active install: its session in place of GTA VI's, whose title is the registry's.
    state
        .prefs
        .game_paths
        .set(Game::GtaVLegacy, Some(gta5.clone()));
    state.prefs.active_game = Some(Game::GtaVLegacy);
    let key = state.world_session().map(|session| session.key().clone());
    assert_eq!(
        key.map(|key| (key.game, key.install)),
        Some((Game::GtaVLegacy, gta5.clone()))
    );
    assert_eq!(state.world.opened, 2);
    let kinds = state.world_session().map(|session| session.option_kinds());
    let title = or_worlds::of_game(Game::GtaVLegacy)
        .expect("GTA V has a title")
        .0
        .title;
    assert_eq!(kinds, Some(title.options()));
    assert!(crate::ui::panels::world_viewer::fed(&state));

    // An option chosen opens the world again, as another world.
    let level = (
        title.options()[0].id,
        or_world::slots::OptionValue::Named("a-pack".into()),
    );
    state.world.choose(Game::GtaVLegacy, vec![level.clone()]);
    let key = state.world_session().map(|session| session.key().clone());
    assert_eq!(key.map(|key| key.options), Some(vec![level]));
    assert_eq!(state.world.opened, 3);

    // A folder that is no install does not open, and says so.
    let started = Instant::now();
    loop {
        let session = state.world_session().expect("the session is held");
        session.poll();
        if let Standing::Failed(why) = session.standing() {
            assert!(!why.is_empty());
            break;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "an empty folder was still opening"
        );
        std::thread::sleep(Duration::from_millis(15));
    }

    // Back to no active install: GTA VI's world again.
    state.prefs.active_game = None;
    let game = state.world_session().map(|session| session.key().game);
    assert_eq!(game, Some(Game::GtaVI));
    assert_eq!(state.world.opened, 4);
    assert!(!crate::ui::panels::world_viewer::fed(&state));
}

#[test]
fn a_gta_v_install_opens_a_session_of_gta_vs_capabilities_whose_selection_is_the_drivers() {
    use or_world::contract::WorldSpec;
    let install = crate::forest_fixture::install(&[("x64a.rpf", crate::hand_built::files())]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (entry, game) = or_worlds::of_game(Game::GtaVLegacy).expect("GTA V has a title");
    let key = WorldKey {
        game: Game::GtaVLegacy,
        install: install.path().to_path_buf(),
        options: Vec::new(),
    };
    let spec = WorldSpec::of(game, install.path());
    let config = scratch.path().join("the-sessions");
    let mut session = DrivenSession::open(key, entry.title, spec.clone(), OwnedKeys::None, config);

    // The same world opened by hand, for the driver's own selection.
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let own_config = scratch.path().join("the-drivers");
    let opening = Opening {
        config: &own_config,
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    let own = entry
        .title
        .open(&spec, &opening)
        .expect("the install built by hand opens");
    let file = kept::file_for(&own_config, entry.title.id(), &spec);
    let (index, _) = kept::refresh(entry.title.id(), &spec, &*own, &file).expect("it links");
    let index = Arc::new(index);

    let mut device = Device::default();
    for eye in [crate::hand_built::NEAR_EYE, crate::hand_built::FAR_EYE] {
        let handed = at_rest(&mut session, eye, &mut device);
        assert!(!handed.instances.is_empty(), "nothing draws at {eye}");
        assert!(
            !device.there.is_empty(),
            "no model of the scene was handed over at {eye}"
        );
        let mut driver = Driver::new(Arc::clone(&own), Arc::clone(&index));
        let view = own.policy().view(eye, Vec3::ZERO, session.camera().fov_y);
        let moment = Moment::noon(0);
        let stands = |model: AssetKey| match () {
            () if device.there.contains(&model) => NodeState::Ready,
            () if device.never.contains(&model) => NodeState::Absent,
            () => NodeState::Loading,
        };
        let started = Instant::now();
        let selected = loop {
            driver.stream(&view, &moment, &Switched::default(), None, usize::MAX);
            let selected = driver.select(&view, &moment, &stands);
            if !selected.fading {
                break selected;
            }
            assert!(started.elapsed() < PATIENCE, "the driver was not at rest");
            std::thread::sleep(Duration::from_millis(15));
        };
        assert_eq!(handed.instances, selected.instances, "at {eye}");
    }
    // What the session states is what GTA V's opened world states, and its options are the title's.
    assert_eq!(session.caps(), own.caps());
    for capability in [
        Capability::Ground,
        Capability::Collision,
        Capability::Placements,
        Capability::WeatherGate,
    ] {
        assert!(session.caps().has(capability), "{capability:?}");
    }
    assert!(!session.caps().has(Capability::Edits));
    assert_ne!(session.caps(), gta6::CAPS);
    assert_eq!(session.option_kinds(), entry.title.options());
}
