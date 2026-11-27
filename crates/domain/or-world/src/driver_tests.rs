use glam::Vec3;
use or_job::{Cancel, Progress};

use super::*;
use crate::contract::{EditPlace, EditTitle, Keys, Opening, WorldTitle};
use crate::fake::{self, FakeTitle};
use crate::kept;
use crate::model::is_interior_key;

struct Opened {
    title: FakeTitle,
    install: Arc<dyn WorldInstall>,
    index: Arc<Index>,
    _folder: tempfile::TempDir,
}

fn open() -> Opened {
    let folder = tempfile::tempdir().expect("a scratch folder");
    let title = FakeTitle::new();
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config: folder.path(),
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    let world = FakeTitle::spec();
    let install = title.open(&world, &opening).expect("the fake world opens");
    let file = kept::file_for(folder.path(), title.id(), &world);
    let (index, _) = kept::refresh(title.id(), &world, &*install, &file).expect("the index builds");
    Opened {
        title,
        install,
        index: Arc::new(index),
        _folder: folder,
    }
}

impl Opened {
    fn driver(&self) -> Driver {
        Driver::new(Arc::clone(&self.install), Arc::clone(&self.index))
    }

    fn id(&self, name: u64) -> MapId {
        self.index.named(name).expect("a map of the fake world")
    }
}

fn view(driver: &Driver, eye: Vec3) -> View {
    (driver.policy()).view(eye, Vec3::ZERO, std::f32::consts::FRAC_PI_4)
}

fn noon() -> Moment {
    Moment::noon(fake::WEATHER)
}

fn ready(_: AssetKey) -> NodeState {
    NodeState::Ready
}

fn maps(streamed: &Streamed) -> Vec<MapId> {
    streamed.wanted.iter().map(|(map, _)| *map).collect()
}

#[test]
fn a_parent_map_is_wanted_with_its_child_whatever_its_own_box() {
    let opened = open();
    let (far, near) = (opened.id(fake::FAR), opened.id(fake::NEAR));
    let mut driver = opened.driver();
    let none = Switched::default();
    let streamed = driver.stream(
        &view(&driver, fake::NEAR_EYE),
        &noon(),
        &none,
        None,
        usize::MAX,
    );
    assert_eq!(streamed.wanted, [(far, Why::Own), (near, Why::Own)]);
    assert_eq!(streamed.read, [far, near]);

    // Where the child's box alone is wanted, the child brings its parent.
    let mut alone = opened.driver();
    alone.policy.streams_in = |boxes, _| boxes.entities.1.x == 200.0;
    let none = Switched::default();
    let streamed = alone.stream(
        &view(&alone, fake::NOWHERE),
        &noon(),
        &none,
        None,
        usize::MAX,
    );
    assert_eq!(
        streamed.wanted,
        [(far, Why::ParentOf(near)), (near, Why::Own)]
    );
}

#[test]
fn a_held_map_is_read_once() {
    let opened = open();
    let mut driver = opened.driver();
    let none = Switched::default();
    for eye in [
        fake::NEAR_EYE,
        fake::FAR_EYE,
        fake::NEAR_EYE,
        fake::YARD_EYE,
        fake::NEAR_EYE,
    ] {
        let view = view(&driver, eye);
        driver.stream(&view, &noon(), &none, None, usize::MAX);
        driver.select(&view, &noon(), &ready);
    }
    let counts = opened.title.counts();
    for name in [fake::FAR, fake::NEAR, fake::YARD] {
        assert_eq!(counts.reads_of(name), 1, "the map {name:#x}");
    }
    assert_eq!(counts.reads_of(fake::NIGHT), 0);
    assert_eq!(driver.held_maps(), 3);
}

#[test]
fn a_body_is_laid_out_once_a_selection_and_only_where_it_draws() {
    let opened = open();
    let (far, near) = (opened.id(fake::FAR).key(), opened.id(fake::NEAR).key());
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    driver.stream(&view, &noon(), &none, None, usize::MAX);
    let counts = opened.title.counts();
    for selections in 1..=3 {
        let selected = driver.select(&view, &noon(), &ready);
        assert_eq!(selected.instances.len(), 2);
        assert_eq!(counts.lays_of(near), selections);
        // The parent gave way to its children: nothing of its body draws, and it is not laid out.
        assert_eq!(counts.lays_of(far), 0);
    }
}

#[test]
fn the_wanted_maps_go_out_and_their_bodies_come_in() {
    let opened = open();
    let (far, near) = (opened.id(fake::FAR), opened.id(fake::NEAR));
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    let ask = driver.want(&view, &noon(), &none, None);
    assert_eq!(ask, [far, near], "the nearest first, then by place");
    let waiting = driver.settle(&view);
    assert_eq!(waiting.waiting, [far, near]);
    assert!(waiting.wanted.is_empty() && driver.is_empty());

    let mut worker = Worker::new();
    let body = opened.install.maps().read(&opened.index, near, &mut worker);
    assert_eq!(driver.want(&view, &noon(), &none, None), [far, near]);
    driver.give(near, body);
    let streamed = driver.settle(&view);
    assert_eq!(
        (maps(&streamed), streamed.read, streamed.waiting),
        (vec![near], vec![near], vec![far])
    );
    // Its parent not held yet, each child stands alone by its own distance.
    let selected = driver.select(&view, &noon(), &ready);
    assert_eq!(selected.instances.len(), 2);
    assert_eq!(driver.want(&view, &noon(), &none, None), [far]);
}

#[test]
fn a_map_that_gives_no_body_is_asked_for_once() {
    let opened = open();
    let near = opened.id(fake::NEAR);
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    driver.want(&view, &noon(), &none, None);
    driver.give(near, Err(WorldError::NoSuchMap(near)));
    let streamed = driver.settle(&view);
    assert_eq!(streamed.unread.len(), 1);
    assert_eq!(streamed.unread[0].0, near);
    assert!(!driver.want(&view, &noon(), &none, None).contains(&near));
}

#[test]
fn an_interior_comes_and_goes_with_its_instance_distance() {
    let opened = open();
    let yard = opened.id(fake::YARD);
    let mut driver = opened.driver();
    let none = Switched::default();
    let near = view(&driver, fake::YARD_EYE);
    let streamed = driver.stream(&near, &noon(), &none, None, usize::MAX);
    assert_eq!(streamed.interiors.len(), 1);
    assert_eq!(streamed.interiors[0].map, yard);
    let selected = driver.select(&near, &noon(), &ready);
    let inside = selected
        .instances
        .node
        .iter()
        .filter(|node| is_interior_key(node.0));
    assert_eq!(inside.count(), fake::INTERIOR_NODES);
    assert_eq!(
        selected.bodies.len(),
        2,
        "the map's body and the interior's"
    );

    // Still inside the map's sphere, past the instance's distance and its band.
    let away = view(&driver, fake::YARD_EYE + Vec3::new(0.0, 280.0, 0.0));
    let streamed = driver.stream(&away, &noon(), &none, None, usize::MAX);
    assert_eq!(maps(&streamed), [yard]);
    assert!(streamed.interiors.is_empty());
    assert_eq!(driver.len().0, 1);
}

#[test]
fn a_budget_drops_the_map_wanted_longest_ago_and_never_one_in_the_forest() {
    let opened = open();
    let mut driver = opened.driver();
    driver.policy.budget = 1;
    let none = Switched::default();
    let view_near = view(&driver, fake::NEAR_EYE);
    driver.stream(&view_near, &noon(), &none, None, usize::MAX);
    assert_eq!(driver.held_maps(), 2, "a view's own maps pass the budget");
    let view_yard = view(&driver, fake::YARD_EYE);
    driver.stream(&view_yard, &noon(), &none, None, usize::MAX);
    assert_eq!(driver.held_maps(), 1);
    driver.stream(&view_near, &noon(), &none, None, usize::MAX);
    assert_eq!(
        opened.title.counts().reads_of(fake::NEAR),
        2,
        "a map let go of is read again"
    );
}

#[test]
fn an_edit_session_is_asked_before_the_reader_and_its_body_taken_once() {
    let opened = open();
    let near = opened.id(fake::NEAR);
    let edits = opened.title.edits();
    let place = EditPlace {
        install: &opened.install,
        index: &opened.index,
        folder: None,
    };
    let mut session = edits.open(&place).expect("the fake store opens");
    let to = Vec3::new(-10.0, 30.0, 0.0);
    let op = fake::move_node(&*session, fake::NEAR, 0, to);
    session.apply(&op).expect("the move applies");

    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    let counts = opened.title.counts();
    for _ in 0..3 {
        let streamed = driver.stream(&view, &noon(), &none, Some(&*session), usize::MAX);
        assert!(maps(&streamed).contains(&near));
    }
    assert_eq!(
        counts.reads_of(fake::NEAR),
        0,
        "the session's body is never read"
    );
    let selected = driver.select(&view, &noon(), &ready);
    let moved = selected
        .instances
        .node
        .iter()
        .position(|node| *node == (near.key(), 0));
    assert_eq!(
        moved.and_then(|at| selected.instances.position(at)),
        Some(to)
    );
    assert_eq!(counts.flushes(), 0);

    // Flushed, the session holds the map no more, and the reader's body stands again, moved.
    session.flush().expect("the fake store flushes");
    driver.stream(&view, &noon(), &none, Some(&*session), usize::MAX);
    assert_eq!(counts.reads_of(fake::NEAR), 1);
    let selected = driver.select(&view, &noon(), &ready);
    let moved = selected
        .instances
        .node
        .iter()
        .position(|node| *node == (near.key(), 0));
    assert_eq!(
        moved.and_then(|at| selected.instances.position(at)),
        Some(to)
    );
}

#[test]
fn a_driver_can_be_handed_to_a_thread_of_its_own() {
    fn sendable<T: Send>() {}
    sendable::<Driver>();
    sendable::<Selected>();
    sendable::<Streamed>();
}
