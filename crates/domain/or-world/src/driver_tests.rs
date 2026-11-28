use glam::Vec3;
use or_job::{Cancel, Progress};

use super::*;
use crate::contract::{EditPlace, EditTitle, Keys, Opening, WorldTitle};
use crate::fake::{self, FakeTitle, Fault};
use crate::kept;
use crate::model::{is_interior_key, Gate, MapRow};
use crate::streaming::MapBox;

struct Opened {
    title: FakeTitle,
    install: Arc<dyn WorldInstall>,
    index: Arc<Index>,
    _folder: tempfile::TempDir,
}

fn open() -> Opened {
    open_title(FakeTitle::new())
}

fn open_title(title: FakeTitle) -> Opened {
    let folder = tempfile::tempdir().expect("a scratch folder");
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
    fn driver(&self) -> Driver<'static> {
        Driver::new(Arc::clone(&self.install), Arc::clone(&self.index))
    }

    fn id(&self, name: u64) -> MapId {
        self.index.named(name).expect("a map of the fake world")
    }
}

fn view(driver: &Driver<'_>, eye: Vec3) -> View {
    (driver.policy()).view(eye, Vec3::ZERO, std::f32::consts::FRAC_PI_4)
}

/// [`view`], where a local of that name hides it.
fn view_of(driver: &Driver<'_>, eye: Vec3) -> View {
    view(driver, eye)
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
    sendable::<Driver<'static>>();
    sendable::<Selected>();
    sendable::<Streamed>();
}

#[test]
fn a_map_the_reader_refused_is_taken_from_an_edit_session_that_holds_it() {
    let opened = open();
    let near = opened.id(fake::NEAR);
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    driver.want(&view, &noon(), &none, None);
    driver.give(near, Err(WorldError::NoSuchMap(near)));
    assert_eq!(driver.settle(&view).unread.len(), 1);

    let edits = opened.title.edits();
    let place = EditPlace {
        install: &opened.install,
        index: &opened.index,
        folder: None,
    };
    let mut session = edits.open(&place).expect("the fake store opens");
    let op = fake::move_node(&*session, fake::NEAR, 0, fake::MOVED_TO);
    session.apply(&op).expect("the move applies");
    let streamed = driver.stream(&view, &noon(), &none, Some(&*session), usize::MAX);
    assert!(streamed.unread.is_empty(), "{:?}", streamed.unread);
    assert!(maps(&streamed).contains(&near));
}

#[test]
fn the_maps_to_read_go_out_nearest_first() {
    let opened = open();
    let (far, near, yard) = (
        opened.id(fake::FAR),
        opened.id(fake::NEAR),
        opened.id(fake::YARD),
    );
    let mut driver = opened.driver();
    driver.policy.streams_in = |_, _| true;
    let none = Switched::default();
    // The yard's box holds the eye, the far map's ends 4400 m from it and the near map's 4800 m.
    let view = view(&driver, fake::YARD_EYE);
    assert!(
        yard > near && near > far,
        "the nearest is the last by place"
    );
    assert_eq!(driver.want(&view, &noon(), &none, None), [yard, far, near]);
}

#[test]
fn a_map_the_view_left_is_taken_out_of_the_forest_and_stays_held() {
    let opened = open();
    let mut driver = opened.driver();
    let none = Switched::default();
    let near = view(&driver, fake::NEAR_EYE);
    driver.stream(&near, &noon(), &none, None, usize::MAX);
    assert_eq!(driver.len().0, 2);
    let nowhere = view(&driver, fake::NOWHERE);
    let streamed = driver.stream(&nowhere, &noon(), &none, None, usize::MAX);
    assert!(streamed.wanted.is_empty() && driver.is_empty());
    assert_eq!(driver.held_maps(), 2);
    let selected = driver.select(&nowhere, &noon(), &ready);
    assert!(selected.instances.is_empty() && selected.bodies.is_empty());
}

#[test]
fn a_body_given_after_its_map_left_the_view_stands_in_no_forest() {
    let opened = open();
    let near = opened.id(fake::NEAR);
    let mut driver = opened.driver();
    let none = Switched::default();
    let at_near = view(&driver, fake::NEAR_EYE);
    driver.stream(&at_near, &noon(), &none, None, usize::MAX);
    assert_eq!(driver.len().0, 2);

    // A second read of the map, asked for while the first was under way, comes in after the view left.
    let nowhere = view(&driver, fake::NOWHERE);
    assert!(driver.want(&nowhere, &noon(), &none, None).is_empty());
    let mut worker = Worker::new();
    let late = opened.install.maps().read(&opened.index, near, &mut worker);
    driver.give(near, late);
    let streamed = driver.settle(&nowhere);
    assert!(streamed.wanted.is_empty() && streamed.read.is_empty());
    assert!(driver.is_empty(), "{:?} stand in the forest", driver.len());
    assert!(driver
        .select(&nowhere, &noon(), &ready)
        .instances
        .is_empty());
}

#[test]
fn a_map_wanted_again_is_not_the_one_let_go_of() {
    let opened = open();
    let mut driver = opened.driver();
    let none = Switched::default();
    let night = Moment {
        hour: 22.0,
        weather: fake::WEATHER,
    };
    let at_night = view(&driver, fake::NIGHT_EYE);
    let at_yard = view(&driver, fake::YARD_EYE);
    let at_near = view(&driver, fake::NEAR_EYE);
    driver.stream(&at_night, &night, &none, None, usize::MAX);
    let one_node = driver.held_bytes();
    driver.stream(&at_yard, &night, &none, None, usize::MAX);
    // Held, and wanted again after the yard was.
    driver.stream(&at_night, &night, &none, None, usize::MAX);
    assert_eq!(driver.held_bytes(), 3 * one_node, "one node, then two");

    // Three more nodes come in, and one map of those the view left must go: the yard, wanted before.
    driver.policy.budget = 5 * one_node;
    driver.stream(&at_near, &night, &none, None, usize::MAX);
    assert_eq!(driver.held_bytes(), 4 * one_node);
    driver.stream(&at_night, &night, &none, None, usize::MAX);
    let counts = opened.title.counts();
    assert_eq!(counts.reads_of(fake::NIGHT), 1, "it was held throughout");
    assert_eq!(counts.reads_of(fake::YARD), 1);
}

#[test]
fn an_interior_in_reach_is_read_once_and_one_that_gives_no_body_is_asked_for_once() {
    for (title, came_in) in [
        (FakeTitle::new(), 1),
        (FakeTitle::broken(Fault::InteriorUnread), 0),
    ] {
        let opened = open_title(title);
        let mut driver = opened.driver();
        let none = Switched::default();
        let view = view(&driver, fake::YARD_EYE);
        for _ in 0..3 {
            let streamed = driver.stream(&view, &noon(), &none, None, usize::MAX);
            assert_eq!(streamed.interiors.len(), came_in);
            assert_eq!(streamed.interiors_unread.len(), 1 - came_in);
        }
        assert_eq!(opened.title.counts().interior_reads(), 1);
    }
}

#[test]
fn an_interior_follows_its_instance_when_an_edit_moves_it() {
    let opened = open();
    let edits = opened.title.edits();
    let place = EditPlace {
        install: &opened.install,
        index: &opened.index,
        folder: None,
    };
    let mut session = edits.open(&place).expect("the fake store opens");
    // The instance of the interior is the yard's first node.
    let to = fake::YARD_EYE + Vec3::new(0.0, 20.0, 0.0);
    let op = fake::move_node(&*session, fake::YARD, 0, to);
    session.apply(&op).expect("the move applies");

    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::YARD_EYE);
    for _ in 0..3 {
        let streamed = driver.stream(&view, &noon(), &none, Some(&*session), usize::MAX);
        assert_eq!(streamed.interiors.len(), 1);
    }
    assert_eq!(
        opened.title.counts().interior_reads(),
        1,
        "a session's body that stands is taken once, and its interior read once"
    );
    let selected = driver.select(&view, &noon(), &ready);
    let of = &selected.instances;
    let mut inside: Vec<Vec3> = (0..of.len())
        .filter(|at| is_interior_key(of.node[*at].0))
        .filter_map(|at| of.position(at))
        .collect();
    inside.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_eq!(
        inside,
        [to - Vec3::new(3.0, 0.0, 0.0), to + Vec3::new(3.0, 0.0, 0.0)],
        "the interior's nodes stand about the instance where the edit put it"
    );
}

#[test]
fn an_interior_in_the_forest_is_read_again_from_the_body_an_apply_put_in_its_maps_place() {
    let opened = open();
    let edits = opened.title.edits();
    let place = EditPlace {
        install: &opened.install,
        index: &opened.index,
        folder: None,
    };
    let mut session = edits.open(&place).expect("the fake store opens");
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::YARD_EYE);
    let streamed = driver.stream(&view, &noon(), &none, Some(&*session), usize::MAX);
    assert_eq!(streamed.interiors.len(), 1);

    // The interior stands in the forest, and its instance, the yard's first node, is moved.
    let to = fake::YARD_EYE + Vec3::new(0.0, 20.0, 0.0);
    let op = fake::move_node(&*session, fake::YARD, 0, to);
    session.apply(&op).expect("the move applies");
    let streamed = driver.stream(&view, &noon(), &none, Some(&*session), usize::MAX);
    assert_eq!(streamed.interiors.len(), 1);
    assert_eq!(opened.title.counts().interior_reads(), 2);
    let selected = driver.select(&view, &noon(), &ready);
    let of = &selected.instances;
    let mut inside: Vec<Vec3> = (0..of.len())
        .filter(|at| is_interior_key(of.node[*at].0))
        .filter_map(|at| of.position(at))
        .collect();
    inside.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_eq!(
        inside,
        [to - Vec3::new(3.0, 0.0, 0.0), to + Vec3::new(3.0, 0.0, 0.0)]
    );
}

#[test]
fn a_model_two_nodes_wait_for_is_waited_for_once() {
    let opened = open();
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    driver.stream(&view, &noon(), &none, None, usize::MAX);
    let loading = |model: AssetKey| match model == fake::CUBE {
        true => NodeState::Loading,
        false => NodeState::Ready,
    };
    // Both children of the ladder draw with the cube.
    let selected = driver.select(&view, &noon(), &loading);
    assert_eq!(selected.waiting, [fake::CUBE]);
}

#[test]
fn a_map_that_stays_in_the_view_is_put_in_the_forest_once() {
    let opened = open();
    let mut driver = opened.driver();
    let none = Switched::default();
    // From afar the parent of the ladder draws alone: a root of a low level, which fades in by time.
    let view = view(&driver, fake::FAR_EYE);
    for views in 0..3 {
        driver.stream(&view, &noon(), &none, None, usize::MAX);
        let selected = driver.select(&view, &noon(), &ready);
        // The first selection is a cut, and what drew whole at one draws whole at the next: a
        // body put in the forest again would fade in from nought.
        assert!(!selected.fading, "view {views}");
        assert_eq!(selected.instances.fade, [1.0], "view {views}");
    }
}

#[test]
fn a_map_whose_box_is_stated_inside_out_is_asked_for_by_how_far_its_middle_stands() {
    let opened = open();
    let row = |name: u64, min: f32, max: f32| {
        let span = (Vec3::splat(min), Vec3::splat(max));
        MapRow {
            name,
            label: format!("{name}"),
            boxes: Some(MapBox {
                streaming: span,
                entities: span,
            }),
            parents: Vec::new(),
            gate: Gate::Always,
            archive: 0,
        }
    };
    // From the origin: a corner at 60, an inside-out box about 50, a corner at 30, a corner at 10.
    let rows = vec![
        row(1, 60.0, 70.0),
        row(2, 80.0, 20.0),
        row(3, 30.0, 35.0),
        row(4, 10.0, 20.0),
    ];
    let index = Arc::new(Index::new(rows).expect("four rows that link"));
    let mut driver = Driver::new(Arc::clone(&opened.install), index);
    driver.policy.streams_in = |_, _| true;
    let none = Switched::default();
    let view = view(&driver, Vec3::ZERO);
    assert_eq!(
        driver.want(&view, &noon(), &none, None),
        [MapId(3), MapId(2), MapId(1), MapId(0)]
    );
}

#[test]
fn a_body_read_before_an_apply_does_not_take_the_place_of_the_sessions() {
    let opened = open();
    let (far, near) = (opened.id(fake::FAR), opened.id(fake::NEAR));
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    // The map is asked of the reader, and an apply moves a node of it while the read is under way.
    assert_eq!(driver.want(&view, &noon(), &none, None), [far, near]);
    let mut worker = Worker::new();
    let mut read = |map: MapId| opened.install.maps().read(&opened.index, map, &mut worker);
    let (far_read, near_read) = (read(far), read(near));
    let edits = opened.title.edits();
    let place = EditPlace {
        install: &opened.install,
        index: &opened.index,
        folder: None,
    };
    let mut session = edits.open(&place).expect("the fake store opens");
    let op = fake::move_node(&*session, fake::NEAR, 0, fake::MOVED_TO);
    session.apply(&op).expect("the move applies");

    assert_eq!(driver.want(&view, &noon(), &none, Some(&*session)), [far]);
    driver.give(far, far_read);
    driver.give(near, near_read);
    let streamed = driver.settle(&view);
    assert_eq!(maps(&streamed), [far, near]);
    assert_eq!(
        streamed.read,
        [far],
        "the session's body was read by no reader"
    );
    let selected = driver.select(&view, &noon(), &ready);
    let moved = (selected.instances.node.iter()).position(|node| *node == (near.key(), 0));
    assert_eq!(
        moved.and_then(|at| selected.instances.position(at)),
        Some(fake::MOVED_TO)
    );
}

#[test]
fn a_body_says_what_draws_its_nodes_once_a_moment_and_never_once_a_node() {
    let opened = open();
    let (far, near) = (opened.id(fake::FAR).key(), opened.id(fake::NEAR).key());
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    driver.stream(&view, &noon(), &none, None, usize::MAX);
    let counts = opened.title.counts();
    assert_eq!(
        counts.draws_of(near),
        0,
        "nothing is asked before a selection"
    );
    for _ in 0..3 {
        driver.select(&view, &noon(), &ready);
    }
    // Two nodes, three selections, one moment.
    assert_eq!((counts.draws_of(near), counts.draws_of(far)), (1, 1));
    let later = Moment {
        hour: 13.0,
        weather: fake::WEATHER,
    };
    driver.select(&view, &later, &ready);
    assert_eq!((counts.draws_of(near), counts.draws_of(far)), (2, 2));

    // A map that left the view and came back is asked again: its body may be another.
    let nowhere = view_of(&driver, fake::NOWHERE);
    driver.stream(&nowhere, &later, &none, None, usize::MAX);
    assert!(
        driver.told.is_empty(),
        "nothing is kept of a body that left the forest"
    );
    driver.stream(&view, &later, &none, None, usize::MAX);
    driver.select(&view, &later, &ready);
    assert_eq!(counts.draws_of(near), 3);
}

#[test]
fn an_interior_in_reach_waits_for_its_body_and_a_settle_reads_none() {
    let opened = open();
    let yard = opened.id(fake::YARD);
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::YARD_EYE);
    let mut worker = Worker::new();
    let reader = opened.install.maps();
    assert_eq!(driver.want(&view, &noon(), &none, None), [yard]);
    driver.give(yard, reader.read(&opened.index, yard, &mut worker));
    let streamed = driver.settle(&view);
    assert_eq!(
        (streamed.interiors.len(), streamed.interiors_waiting.len()),
        (0, 1)
    );
    let counts = opened.title.counts();
    assert_eq!(counts.interior_reads(), 0, "a settle reads nothing");

    let at = streamed.interiors_waiting[0];
    let placing = Arc::clone(driver.placing(yard).expect("the yard is held"));
    let mut interior = |placing: &Arc<dyn MapBody>| {
        reader.interior(&opened.index, yard, &**placing, &at.placed, &mut worker)
    };
    // Read from another body of the map than the one that stands, it is let go of.
    let other = (reader.read(&opened.index, yard, &mut Worker::new())).expect("the yard reads");
    driver.give_interior(at, &other, interior(&other));
    let streamed = driver.settle(&view);
    assert_eq!(
        (streamed.interiors.len(), streamed.interiors_waiting.len()),
        (0, 1)
    );

    // Given for a view that left its reach before the next settle, it is let go of too.
    driver.give_interior(at, &placing, interior(&placing));
    let away = view_of(&driver, fake::YARD_EYE + Vec3::new(0.0, 280.0, 0.0));
    assert!(driver.settle(&away).interiors_waiting.is_empty());
    assert_eq!(driver.settle(&view).interiors_waiting, [at]);

    driver.give_interior(at, &placing, interior(&placing));
    let streamed = driver.settle(&view);
    assert_eq!(
        (streamed.interiors, streamed.interiors_waiting),
        (vec![at], vec![])
    );
    assert_eq!(driver.len().0, 2, "the yard and its interior");

    // An interior that gives no body is asked for no more.
    let mut refusing = opened.driver();
    refusing.want(&view, &noon(), &none, None);
    refusing.give(yard, reader.read(&opened.index, yard, &mut Worker::new()));
    let waiting = refusing.settle(&view).interiors_waiting;
    assert_eq!(waiting, [at]);
    let held = Arc::clone(refusing.placing(yard).expect("the yard is held"));
    refusing.give_interior(at, &held, Err(WorldError::Lacks("such interior")));
    let streamed = refusing.settle(&view);
    assert_eq!(streamed.interiors_unread.len(), 1);
    assert!(streamed.interiors_waiting.is_empty() && streamed.interiors.is_empty());
}

#[test]
fn an_interior_refused_from_a_body_that_stands_no_more_is_asked_for_again() {
    let opened = open();
    let yard = opened.id(fake::YARD);
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::YARD_EYE);
    let reader = opened.install.maps();
    driver.want(&view, &noon(), &none, None);
    driver.give(yard, reader.read(&opened.index, yard, &mut Worker::new()));
    let waiting = driver.settle(&view).interiors_waiting;
    assert_eq!(waiting.len(), 1);

    // A read from a body an apply has since replaced says nothing of the interior as it stands.
    let other = (reader.read(&opened.index, yard, &mut Worker::new())).expect("the yard reads");
    driver.give_interior(waiting[0], &other, Err(WorldError::Lacks("such instance")));
    let streamed = driver.settle(&view);
    assert_eq!(streamed.interiors_waiting, waiting);
    assert!(streamed.interiors_unread.is_empty());
}

#[test]
fn a_refusal_given_after_a_body_leaves_the_body_in_the_forest() {
    let opened = open();
    let near = opened.id(fake::NEAR);
    let mut driver = opened.driver();
    let none = Switched::default();
    let view = view(&driver, fake::NEAR_EYE);
    // The map was asked for twice before either read came back, and the second read failed.
    driver.want(&view, &noon(), &none, None);
    let body = (opened.install.maps()).read(&opened.index, near, &mut Worker::new());
    driver.give(near, body);
    driver.give(near, Err(WorldError::NoSuchMap(near)));
    let streamed = driver.settle(&view);
    assert!(streamed.unread.is_empty());
    assert_eq!(maps(&streamed), [near]);
}

#[test]
fn a_parent_several_wanted_maps_name_is_wanted_as_the_parent_of_the_first_in_the_indexs_order() {
    let opened = open();
    let span = (Vec3::splat(-10.0), Vec3::splat(10.0));
    let row = |name: u64, boxes: Option<MapBox>, parents: &[u32]| MapRow {
        name,
        label: format!("{name}"),
        boxes,
        parents: parents.iter().map(|at| MapId(*at)).collect(),
        gate: Gate::Always,
        archive: 0,
    };
    let own = Some(MapBox {
        streaming: span,
        entities: span,
    });
    // Rows 1, 2 and 3 name row 0, which streams by no box of its own; row 4 names row 3 and
    // then row 5, and row 5 names row 6; row 7 names rows 8 and 9, which both name row 10.
    let rows = vec![
        row(10, None, &[]),
        row(11, own, &[0]),
        row(12, own, &[0]),
        row(13, own, &[0]),
        row(14, own, &[3, 5]),
        row(15, None, &[6]),
        row(16, None, &[]),
        row(17, own, &[8, 9]),
        row(18, None, &[10]),
        row(19, None, &[10]),
        row(20, None, &[]),
    ];
    let index = Arc::new(Index::new(rows).expect("rows that link"));
    let mut driver = Driver::new(Arc::clone(&opened.install), index);
    driver.policy.streams_in = |_, _| true;
    let view = view(&driver, Vec3::ZERO);
    driver.want(&view, &noon(), &Switched::default(), None);
    let whys: Vec<(MapId, Why)> = (driver.wanted.iter())
        .map(|(map, (why, _))| (*map, *why))
        .collect();
    assert_eq!(
        whys,
        [
            (MapId(0), Why::ParentOf(MapId(1))),
            (MapId(1), Why::Own),
            (MapId(2), Why::Own),
            (MapId(3), Why::Own),
            (MapId(4), Why::Own),
            (MapId(5), Why::ParentOf(MapId(4))),
            (MapId(6), Why::ParentOf(MapId(5))),
            (MapId(7), Why::Own),
            (MapId(8), Why::ParentOf(MapId(7))),
            (MapId(9), Why::ParentOf(MapId(7))),
            // Of a map's parents the first is followed first.
            (MapId(10), Why::ParentOf(MapId(8))),
        ]
    );
}

#[test]
fn a_reader_that_words_no_reason_answers_each_ask_once_and_refuses_what_did_not_read() {
    use crate::fake::{Fault, CUBE, SLAB};
    use crate::model::TextureAsk;
    let whole = open();
    let broken = open_title(FakeTitle::broken(Fault::ModelUnread));
    let mut worker = Worker::new();
    // The cube twice: one answer an ask, and the second ask of a model is refused.
    let asks = [SLAB, CUBE, CUBE];
    let read = |opened: &Opened, worker: &mut Worker| {
        let mut out = Vec::new();
        (opened.install.models()).models_or_why(&opened.index, &asks, worker, &mut out);
        let whys: Vec<Option<&'static str>> = (out.iter())
            .map(|(_, model)| model.as_ref().err().map(|refused| refused.why.key()))
            .collect();
        let asked: Vec<_> = out.iter().map(|(ask, _)| *ask).collect();
        assert_eq!(asked, asks);
        whys
    };
    let unsaid = Some("The title read no model and gave no reason");
    assert_eq!(read(&whole, &mut worker), [None, None, unsaid]);
    assert_eq!(read(&broken, &mut worker), [unsaid, None, unsaid]);

    let torn = open_title(FakeTitle::broken(Fault::TextureUnread));
    let flat = TextureAsk {
        scope: CUBE,
        name: 1,
        pixels: 4,
    };
    let nowhere = TextureAsk { name: 2, ..flat };
    let textures = |opened: &Opened, worker: &mut Worker| {
        let mut out = Vec::new();
        (opened.install.models()).textures_or_why(
            &opened.index,
            &[flat, nowhere],
            worker,
            &mut out,
        );
        let whys: Vec<Option<&'static str>> = (out.iter())
            .map(|(_, texture)| texture.as_ref().err().map(|refused| refused.why.key()))
            .collect();
        whys
    };
    let unsaid = Some("The title read no texture and gave no reason");
    assert_eq!(textures(&whole, &mut worker), [None, unsaid]);
    assert_eq!(textures(&torn, &mut worker), [unsaid, unsaid]);
}
