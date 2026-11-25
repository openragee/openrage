//! The level-of-detail forest of a mount built by hand: which maps a view streams in, what the
//! rules select of their entities and of an interior's, and what is read of the mount to say so.
//!
//! ⚠ Every map here is written as the bytes the game ships one in and read back through a mount,
//! so a test that draws nothing has read nothing: each says what it expects to draw.

mod forest_fixture;
mod world_fixture;

use std::f32::consts::FRAC_PI_2;

use forest_fixture::{
    arch, ent, entity_value, install, manifest_bytes, map_around, map_bytes_of, map_file,
    selected_at_the_centres, types_file, Arch, Ent, MapOf, LOD, NO_LEVEL, ORPHAN, ROOMY,
};
use glam::{Quat, Vec3};
use or_depends::world_forest::{
    interior_key, map_box, model_of, request_of, Model, ModelFiles, NoModel, Request, Selected,
    Unread, Why,
};
use or_depends::world_index::{refresh, KeptState};
use or_depends::{MountedWorld, WorldForest, WorldIndex, WorldSource};
use or_world::state::{flags as group_flags, Gate, NOT_BY_DEFAULT};
use or_world::streaming::{Level, NodeState, View, FADE, PRESTREAM};
use or_world::{Placement, WorldState};
use world_fixture::{cube, hash, mount, Counting};

fn all_ready(_: u32) -> NodeState {
    NodeState::Ready
}

fn at(x: f32, y: f32, z: f32) -> View {
    View::at(Vec3::new(x, y, z), 1.0)
}

fn noon() -> WorldState {
    WorldState::at(12, "CLEAR")
}

/// The archetypes a selection draws, lowest hash first.
fn drawn(selected: &Selected) -> Vec<u32> {
    let mut names: Vec<u32> = selected.instances.iter().map(|i| i.archetype).collect();
    names.sort_unstable();
    names
}

/// The hashes of `names`, lowest first.
fn hashes(names: &[&str]) -> Vec<u32> {
    let mut hashes: Vec<u32> = names.iter().map(|name| hash(name)).collect();
    hashes.sort_unstable();
    hashes
}

/// The three archetypes of a parent and its two children, each a drawable of its own name.
fn family_types() -> (String, Vec<u8>) {
    types_file(
        "family",
        &[
            arch("lod_p", 500.0),
            arch("hd_a", 500.0),
            arch("hd_b", 500.0),
        ],
    )
}

/// A parent at the origin drawn to 1,000 m that states two children and 100 m for them.
fn parent() -> Ent {
    Ent {
        lod_dist: 1000.0,
        child_lod_dist: 100.0,
        level: LOD,
        num_children: 2,
        ..ent("lod_p", [0.0; 3])
    }
}

/// The two children, 300 m apart, each naming entity `index` as its parent.
fn children(index: i32, in_parent_map: bool) -> Vec<Ent> {
    let child = |archetype, x| Ent {
        lod_dist: 40.0,
        parent: Some((index, in_parent_map)),
        ..ent(archetype, [x, 0.0, 0.0])
    };
    vec![child("hd_a", 150.0), child("hd_b", -150.0)]
}

/// Select for `view` at noon, every model ready, after streaming for it.
fn seen(forest: &mut WorldForest<'_>, view: &View) -> Selected {
    forest.stream(view, &noon());
    forest.select(view, &all_ready)
}

#[test]
fn a_parent_and_its_two_children_in_one_file_swap_by_the_distance_to_the_parent() {
    let mut entities = vec![parent()];
    entities.extend(children(0, false));
    let one = map_around(cube([-200.0, -200.0, -10.0], 400.0), entities);
    let dir = install(&[("x64a.rpf", vec![map_file("m_one", &one), family_types()])]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    // 240 m from the parent and beside the first child: past the children's 100 m, the parent draws.
    let far = seen(&mut forest, &at(240.0, 0.0, 0.0));
    assert_eq!(drawn(&far), hashes(&["lod_p"]));
    assert_eq!(far.by_level()[Level::Lod as usize], 1);
    // 70 m from the parent and 220 m from the second child: both children draw, the parent does not.
    let near = seen(&mut forest, &at(70.0, 0.0, 0.0));
    assert_eq!(drawn(&near), hashes(&["hd_a", "hd_b"]));
    assert_eq!(near.by_level()[Level::Hd as usize], 2);
    assert_eq!(near.instances.iter().map(|i| i.entity).sum::<u32>(), 1 + 2);
    // Half into the band past the children's distance each draws by half, the parent by the rest.
    let band = seen(&mut forest, &at(100.0 + FADE / 2.0, 0.0, 0.0));
    assert_eq!(drawn(&band), hashes(&["hd_a", "hd_b", "lod_p"]));
    for instance in &band.instances {
        assert!((instance.fade - 0.5).abs() < 1.0 / 255.0, "{instance:?}");
        assert_eq!(instance.yields, instance.archetype == hash("lod_p"));
    }
}

/// A parent in `m_lod`, which streams nowhere near and which nothing asks for, and its two children in `m_hd`, whose parent it is.
fn two_files() -> (MapOf, MapOf) {
    let lod = MapOf {
        flags: NOT_BY_DEFAULT,
        streaming: cube([5000.0, 5000.0, 0.0], 10.0),
        extents: cube([5001.0, 5001.0, 1.0], 1.0),
        ..map_around(cube([0.0; 3], 1.0), vec![parent()])
    };
    // Its entities' extents are centred on the parent's place.
    let hd = MapOf {
        parent: "m_lod",
        ..map_around(cube([-200.0; 3], 400.0), children(0, true))
    };
    (lod, hd)
}

#[test]
fn a_parent_in_the_parent_map_swaps_with_its_children_as_one_in_their_own_file_does() {
    let (lod, hd) = two_files();
    let files = vec![
        map_file("m_lod", &lod),
        map_file("m_hd", &hd),
        family_types(),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    // The parent map streams nowhere near the view and is held back until a script asks: it is
    // wanted as the parent of the map that streams here.
    let lod_map = index.map(hash("m_lod")).expect("the map is indexed");
    assert_eq!(request_of(&index, lod_map), Request::Script);
    let streamed = forest.stream(&at(240.0, 0.0, 0.0), &noon());
    let why: Vec<(&str, Why)> = (streamed.wanted.iter())
        .map(|w| (w.map.name.as_str(), w.why))
        .collect();
    assert!(why.contains(&("m_hd", Why::Own)), "{why:?}");
    assert!(
        why.contains(&("m_lod", Why::ParentOf(hash("m_hd")))),
        "{why:?}"
    );
    assert_eq!(why.len(), 2);

    let far = seen(&mut forest, &at(240.0, 0.0, 0.0));
    assert_eq!(drawn(&far), hashes(&["lod_p"]));
    assert_eq!(far.instances[0].map, hash("m_lod"));
    let near = seen(&mut forest, &at(70.0, 0.0, 0.0));
    assert_eq!(drawn(&near), hashes(&["hd_a", "hd_b"]));
    assert!(near.instances.iter().all(|i| i.map == hash("m_hd")));
    // No child draws beside its parent at any distance along the way.
    for away in (0..30).map(|k| k as f32 * 10.0) {
        let both = seen(&mut forest, &at(away, 0.0, 0.0));
        let whole = |name: &str| {
            let of = both.instances.iter().find(|i| i.archetype == hash(name));
            of.is_some_and(|i| i.fade >= 1.0)
        };
        assert!(
            !(whole("lod_p") && (whole("hd_a") || whole("hd_b"))),
            "{away} m: {:?}",
            drawn(&both)
        );
    }
}

#[test]
fn a_child_whose_parent_map_is_not_read_yet_stands_alone_until_it_is() {
    // One child at the origin that draws to 40 m; its parent stands 500 m off and states 100 m for it.
    let far_parent = Ent {
        lod_dist: 2000.0,
        child_lod_dist: 100.0,
        level: LOD,
        num_children: 1,
        ..ent("lod_p", [0.0, 500.0, 0.0])
    };
    let child = Ent {
        lod_dist: 40.0,
        parent: Some((0, true)),
        ..ent("hd_a", [0.0; 3])
    };
    let (lod, hd) = two_files();
    let lod = MapOf {
        entities: vec![far_parent],
        ..lod
    };
    let hd = MapOf {
        entities: vec![child],
        ..hd
    };
    // The child's map is in the earlier archive: of two maps as near, the earlier is read first.
    let dir = install(&[
        ("x64a.rpf", vec![map_file("m_hd", &hd), family_types()]),
        ("x64b.rpf", vec![map_file("m_lod", &lod)]),
    ]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);
    let view = at(0.0, 30.0, 0.0);

    let first = forest.stream_some(&view, &noon(), 1);
    let names = |maps: &[&or_depends::world_index::Map]| -> Vec<String> {
        maps.iter().map(|map| map.name.clone()).collect()
    };
    assert_eq!(
        (names(&first.read), names(&first.waiting)),
        (vec!["m_hd".to_string()], vec!["m_lod".to_string()])
    );
    // 30 m from the child, inside its own 40 m: it draws alone, as an orphan does.
    assert_eq!(drawn(&forest.select(&view, &all_ready)), hashes(&["hd_a"]));

    let second = forest.stream_some(&view, &noon(), 1);
    assert_eq!(names(&second.read), vec!["m_lod".to_string()]);
    assert!(second.waiting.is_empty());
    // The parent stands 470 m away: its child, now judged by that, gives way to it.
    assert_eq!(drawn(&forest.select(&view, &all_ready)), hashes(&["lod_p"]));
}

#[test]
fn a_parent_that_states_more_children_than_name_it_draws_whole_beside_them() {
    // ⚠ The rules as they stand, held here so a change to them shows: a parent gives way once as
    // many children as it states are in the forest, and no map holds a third child of this one.
    let (lod, hd) = two_files();
    let overstating = MapOf {
        entities: vec![Ent {
            num_children: 3,
            ..parent()
        }],
        ..lod.clone()
    };
    let centre_of = |lod: &MapOf| {
        let files = vec![
            map_file("m_lod", lod),
            map_file("m_hd", &hd),
            family_types(),
        ];
        let dir = install(&[("x64a.rpf", files)]);
        let game = mount(dir.path());
        let source = MountedWorld::new(&game, dir.path());
        let (index, _) = WorldIndex::build(&source, None);
        let mut forest = WorldForest::new(&index, &source, ROOMY);
        // The centre of the children's map is the parent's place: both children draw whole.
        let map = index.map(hash("m_hd")).expect("the map is indexed");
        selected_at_the_centres(&index, &mut forest, &[map])
    };
    let stated_right = centre_of(&lod);
    assert_eq!((stated_right.instances, stated_right.beside), (2, 0));
    let stated_more = centre_of(&overstating);
    assert_eq!((stated_more.instances, stated_more.beside), (3, 2));
    assert_eq!(stated_more.boxes + stated_more.interiors, 0);
    // Both children are beside a parent that states more than the forest holds of it, which is what the install suite counts apart.
    assert_eq!((stated_more.states_more, stated_more.states_fewer), (2, 0));
    assert_eq!(stated_more.unaccounted, Vec::<String>::new());

    // A parent that states one child where two name it never finds as many as it states either.
    let understating = MapOf {
        entities: vec![Ent {
            num_children: 1,
            ..parent()
        }],
        ..lod.clone()
    };
    let stated_fewer = centre_of(&understating);
    assert_eq!((stated_fewer.instances, stated_fewer.beside), (3, 2));
    assert_eq!(
        (stated_fewer.states_more, stated_fewer.states_fewer),
        (0, 2)
    );
    assert_eq!(stated_fewer.unaccounted, Vec::<String>::new());
}

#[test]
fn a_parent_whose_children_lie_in_two_maps_gives_way_once_both_are_in_the_forest() {
    // ⚠ The rules as they stand, held here so a change to them shows: until every map that holds
    // a child of it streams, the parent draws whole beside the children the forest has.
    let (lod, hd) = two_files();
    let child = |archetype, x| Ent {
        lod_dist: 40.0,
        parent: Some((0, true)),
        ..ent(archetype, [x, 0.0, 0.0])
    };
    let first = MapOf {
        entities: vec![child("hd_a", 30.0)],
        ..hd.clone()
    };
    // The second child's map streams from a box that begins 40 m past the parent.
    let second = MapOf {
        streaming: cube([40.0, -500.0, -500.0], 1000.0),
        extents: cube([60.0, -10.0, -10.0], 20.0),
        entities: vec![child("hd_b", 70.0)],
        ..hd
    };
    let files = vec![
        map_file("m_lod", &lod),
        map_file("m_hd_a", &first),
        map_file("m_hd_b", &second),
        family_types(),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);
    let whole = |selected: &Selected| -> Vec<u32> {
        let mut names: Vec<u32> = (selected.instances.iter())
            .filter(|i| i.fade >= 1.0)
            .map(|i| i.archetype)
            .collect();
        names.sort_unstable();
        names
    };

    // 60 m from the parent, inside the 100 m it states for its children, and 110 m short of the
    // second child's map: one child of the two it states is in the forest.
    let short = at(-60.0, 0.0, 0.0);
    let streamed = forest.stream(&short, &noon());
    let mut wanted: Vec<&str> = (streamed.wanted.iter())
        .map(|w| w.map.name.as_str())
        .collect();
    wanted.sort_unstable();
    assert_eq!(wanted, ["m_hd_a", "m_lod"]);
    let beside = forest.select(&short, &all_ready);
    assert_eq!(whole(&beside), hashes(&["hd_a", "lod_p"]));
    assert_eq!(beside.instances.len(), 2);

    // 20 m from the parent the second map streams too: both children, and the parent gives way.
    let both = seen(&mut forest, &at(20.0, 0.0, 0.0));
    assert_eq!(whole(&both), hashes(&["hd_a", "hd_b"]));
    assert_eq!(both.instances.len(), 2);
}

/// A shop's interior placed at (1000, 0, 0) a quarter turn about the upright axis, drawn to 60 m, and its counter 10 m along the interior's first axis, drawn to 30 m.
fn shop() -> (MapOf, Vec<Arch>, Quat, Quat) {
    let turned = Quat::from_rotation_z(FRAC_PI_2);
    let leaning = Quat::from_rotation_x(0.5);
    let instance = Ent {
        rotation: Placement::InteriorInstance.store(turned),
        lod_dist: 60.0,
        level: ORPHAN,
        interior: true,
        ..ent("v_shop", [1000.0, 0.0, 0.0])
    };
    let counter = Ent {
        rotation: Placement::Entity.store(leaning),
        lod_dist: 30.0,
        ..ent("shop_counter", [10.0, 0.0, 0.0])
    };
    let street = map_around(cube([900.0, -100.0, -10.0], 200.0), vec![instance]);
    let types = vec![
        Arch {
            interior: Some(vec![counter]),
            ..arch("v_shop", 0.0)
        },
        arch("shop_counter", 100.0),
    ];
    (street, types, turned, leaning)
}

#[test]
fn an_interior_within_its_distance_brings_its_entities_placed_through_its_instance() {
    let (street, types, turned, leaning) = shop();
    let files = vec![map_file("m_street", &street), types_file("shop", &types)];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let counting = Counting::new(&source);
    let mut forest = WorldForest::new(&index, &counting, ROOMY);
    let opened = || counting.counts().1.iter().sum::<u32>();

    // The counter stands where the instance's turn puts its 10 m: along the second axis.
    let instance_at = Vec3::new(1000.0, 0.0, 0.0);
    let counter_at = instance_at + turned * Vec3::new(10.0, 0.0, 0.0);
    assert!(counter_at.abs_diff_eq(Vec3::new(1000.0, 10.0, 0.0), 1e-3));

    // 200 m from the instance, past its 60 m and its band: the map is read, the interior is not.
    let far = at(1000.0, 200.0, 0.0);
    let streamed = forest.stream(&far, &noon());
    assert_eq!((streamed.wanted.len(), streamed.interiors.len()), (1, 0));
    assert_eq!(opened(), 1);
    assert!(forest.select(&far, &all_ready).instances.is_empty());

    // 5 m from the counter and 15 m from the instance: the type file is read, and the counter draws.
    let near = at(1000.0, 15.0, 0.0);
    let streamed = forest.stream(&near, &noon());
    assert_eq!(streamed.interiors.len(), 1);
    assert_eq!(streamed.interiors[0].archetype, hash("v_shop"));
    assert_eq!(opened(), 2);
    let selected = forest.select(&near, &all_ready);
    assert_eq!(drawn(&selected), hashes(&["shop_counter"]));
    let counter = selected.instances[0];
    assert_eq!((counter.map, counter.entity), (hash("m_street"), 0));
    assert_eq!(counter.interior, Some(0));
    assert_eq!(counter.node.0, interior_key(hash("m_street"), 0));
    assert!(
        Vec3::from(counter.to_world.translation).abs_diff_eq(counter_at, 1e-3),
        "{:?}",
        counter.to_world
    );
    // An entity's stored turn is the conjugate of its turn and an instance's is its turn itself.
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let placed = counter.to_world.transform_vector3(axis);
        assert!(
            placed.abs_diff_eq(turned * (leaning * axis), 1e-3),
            "{axis}: {placed}"
        );
    }

    // 65 m from the instance, inside its band, and 55 m from the counter, past the counter's own:
    // the interior's entities are in the forest and the counter does not draw.
    let between = at(1000.0, 65.0, 0.0);
    let streamed = forest.stream(&between, &noon());
    assert_eq!(streamed.interiors.len(), 1);
    assert!(forest.select(&between, &all_ready).instances.is_empty());
    // Still held: no file was opened for it again.
    assert_eq!(opened(), 2);

    // Out of the instance's distance its entities leave the forest.
    let nodes_with = forest.len().1;
    forest.stream(&far, &noon());
    assert_eq!(forest.len(), (1, nodes_with - 1));
}

/// Bits 22 and 23 and 0 to 5: the hours of the night.
const NIGHT: u32 = (0b11 << 22) | 0b11_1111;

#[test]
fn a_map_the_view_leaves_is_dropped_and_a_night_group_s_map_is_wanted_at_night_alone() {
    let a = map_around(cube([0.0; 3], 100.0), vec![ent("hd_a", [50.0, 50.0, 0.0])]);
    let b = map_around(
        cube([5000.0, 0.0, 0.0], 100.0),
        vec![ent("hd_b", [5050.0, 50.0, 0.0])],
    );
    let held_back = |entities| MapOf {
        flags: NOT_BY_DEFAULT,
        ..map_around(cube([0.0; 3], 100.0), entities)
    };
    let night = held_back(vec![ent("lod_p", [40.0, 40.0, 0.0])]);
    let scripted = held_back(vec![ent("hd_b", [60.0, 60.0, 0.0])]);
    let files = vec![
        map_file("m_a", &a),
        map_file("m_b", &b),
        map_file("m_night", &night),
        map_file("m_scripted", &scripted),
        family_types(),
        (
            "levels/_manifest.ymf".to_string(),
            manifest_bytes(&[("m_night", group_flags::TIMED, NIGHT, &[])]),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    let map = |name: &str| index.map(hash(name)).expect("the map is indexed");
    assert_eq!(request_of(&index, map("m_a")), Request::Always);
    assert_eq!(
        request_of(&index, map("m_night")),
        Request::Group(&Gate::Timed { hours: NIGHT })
    );
    assert_eq!(request_of(&index, map("m_scripted")), Request::Script);
    let boxes = map_box(map("m_a")).expect("it states its boxes");
    assert_eq!(boxes.entities.1, Vec3::splat(100.0));
    assert_eq!(boxes.streaming.0, Vec3::splat(-500.0));

    let wanted = |forest: &mut WorldForest<'_>, view: &View, state: &WorldState| {
        let streamed = forest.stream(view, state);
        let names = streamed.wanted.iter().map(|w| w.map.name.clone());
        names.collect::<Vec<String>>()
    };
    let (here, there) = (at(50.0, 50.0, 0.0), at(5050.0, 50.0, 0.0));
    assert_eq!(wanted(&mut forest, &here, &noon()), ["m_a"]);
    assert_eq!(drawn(&forest.select(&here, &all_ready)), hashes(&["hd_a"]));
    // The view leaves the first map's box for the second's: the first leaves the forest.
    assert_eq!(wanted(&mut forest, &there, &noon()), ["m_b"]);
    assert_eq!(forest.len(), (1, 1));
    assert_eq!(drawn(&forest.select(&there, &all_ready)), hashes(&["hd_b"]));
    // 50 m past the box it streams by a map is still wanted, and past that it is not.
    let edge = 600.0 + PRESTREAM;
    assert_eq!(
        wanted(&mut forest, &at(edge - 1.0, 50.0, 0.0), &noon()),
        ["m_a"]
    );
    assert!(wanted(&mut forest, &at(edge + 1.0, 50.0, 0.0), &noon()).is_empty());
    assert!(forest.is_empty());

    // At an hour of the night the group's map streams; at noon it does not, whatever the weather.
    for (hour, with_night) in [(23, true), (3, true), (12, false), (6, false), (21, false)] {
        let state = WorldState::at(hour, "RAIN");
        let names = wanted(&mut forest, &here, &state);
        assert_eq!(
            names.contains(&"m_night".to_string()),
            with_night,
            "{hour} h"
        );
        assert!(names.contains(&"m_a".to_string()));
        assert!(!names.contains(&"m_scripted".to_string()));
    }
    // A map nothing but a script asks for streams once one has.
    let asked = noon().requesting(hash("m_scripted"));
    assert_eq!(wanted(&mut forest, &here, &asked), ["m_a", "m_scripted"]);
}

#[test]
fn the_budget_drops_the_map_wanted_longest_ago_and_it_is_read_again_when_wanted() {
    let place = |x: f32| {
        let entities = (0..4).map(|k| ent("hd_a", [x + k as f32, 0.0, 0.0]));
        map_around(cube([x, 0.0, 0.0], 100.0), entities.collect())
    };
    let files = vec![
        map_file("m_a", &place(0.0)),
        map_file("m_b", &place(5000.0)),
        map_file("m_c", &place(10000.0)),
        family_types(),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let (a, b, c) = (
        at(50.0, 50.0, 0.0),
        at(5050.0, 50.0, 0.0),
        at(10050.0, 50.0, 0.0),
    );

    // What one map of these weighs, by a forest that holds one.
    let one = {
        let mut forest = WorldForest::new(&index, &source, ROOMY);
        forest.stream(&a, &noon());
        assert_eq!(forest.held_maps(), 1);
        forest.held_bytes()
    };
    assert!(one > 0);

    let counting = Counting::new(&source);
    let opened = || counting.counts().1.iter().sum::<u32>();
    // Room for two maps and not for three.
    let mut forest = WorldForest::new(&index, &counting, 2 * one);
    let mut read = Vec::new();
    for view in [&a, &b, &a, &c, &a, &b] {
        forest.stream(view, &noon());
        read.push(opened());
        assert_eq!(forest.len().0, 1, "one map is in the forest at a time");
        assert!(forest.held_bytes() <= 2 * one);
    }
    // The first, the second and the third are read. Back at the first nothing is: it is held.
    // The third pushes out the second, wanted longer ago than the first, which is read again.
    assert_eq!(read, [1, 2, 2, 3, 3, 4]);
    assert_eq!(forest.held_maps(), 2);

    // With no room for a map let go of, each view reads its map again.
    let counting = Counting::new(&source);
    let mut forest = WorldForest::new(&index, &counting, 0);
    for view in [&a, &b, &a] {
        forest.stream(view, &noon());
        assert_eq!(drawn(&forest.select(view, &all_ready)).len(), 4);
    }
    assert_eq!(counting.counts().1.iter().sum::<u32>(), 3);
}

#[test]
fn an_entity_of_an_archetype_no_type_file_declares_is_returned_as_a_box_with_its_reason() {
    let entities = vec![
        Ent {
            lod_dist: 80.0,
            ..ent("hd_a", [10.0, 0.0, 0.0])
        },
        Ent {
            lod_dist: 80.0,
            ..ent("prop_nowhere", [20.0, 0.0, 0.0])
        },
        // No distance of its own and no archetype to take one from.
        ent("prop_nowhere_either", [30.0, 0.0, 0.0]),
        ent("", [40.0, 0.0, 0.0]),
    ];
    let map = map_around(cube([0.0; 3], 100.0), entities);
    let dir = install(&[("x64a.rpf", vec![map_file("m_one", &map), family_types()])]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    let view = at(0.0, 0.0, 0.0);
    forest.stream(&view, &noon());
    // Every model is loading: a box waits for none, and is asked of no one.
    let loading = |archetype: u32| {
        assert_eq!(
            archetype,
            hash("hd_a"),
            "only a declared archetype is asked for"
        );
        NodeState::Loading
    };
    let selected = forest.select(&view, &loading);
    let boxes: Vec<(u32, Model)> = (selected.instances.iter())
        .map(|i| (i.entity, i.model))
        .collect();
    assert_eq!(
        boxes,
        [
            (1, Model::Box(NoModel::Undeclared)),
            (2, Model::Box(NoModel::Undeclared)),
            (3, Model::Box(NoModel::Unnamed)),
        ]
    );
    assert_eq!(selected.instances[0].archetype, hash("prop_nowhere"));
    assert_eq!(selected.waiting, [hash("hd_a")]);
    assert_eq!(
        NoModel::Undeclared.to_string(),
        "no type file of the mount declares its archetype"
    );
    assert_eq!(
        model_of(&index, hash("prop_nowhere")),
        Some(Model::Box(NoModel::Undeclared))
    );
    assert!(selected.files().drawables.is_empty());

    // Anywhere its map streams from, the one that states no distance still stands.
    let corner = at(590.0, 590.0, 0.0);
    forest.stream(&corner, &noon());
    let far = forest.select(&corner, &all_ready);
    let entities: Vec<u32> = far.instances.iter().map(|i| i.entity).collect();
    assert_eq!(entities, [2, 3]);

    // What an install suite holds a selection to counts the three, and holds each to the index.
    let map = index.map(hash("m_one")).expect("the map is indexed");
    assert_eq!(
        selected_at_the_centres(&index, &mut forest, &[map]).boxes,
        3
    );
}

#[test]
fn an_instance_names_the_files_its_archetype_states_and_a_kept_index_names_them_too() {
    let types = vec![
        Arch {
            files: ("loose_model", "", "loose_txd"),
            ..arch("loose", 80.0)
        },
        Arch {
            files: ("in_dictionary", "props_dict", "props_txd"),
            ..arch("in_dictionary", 80.0)
        },
        // It names no asset: nothing draws it, and it is no box.
        Arch {
            files: ("", "", ""),
            ..arch("assetless", 80.0)
        },
    ];
    let entities = vec![
        ent("loose", [1.0, 0.0, 0.0]),
        ent("in_dictionary", [2.0, 0.0, 0.0]),
        ent("assetless", [3.0, 0.0, 0.0]),
    ];
    let map = MapOf {
        flags: NOT_BY_DEFAULT,
        ..map_around(cube([0.0; 3], 100.0), entities)
    };
    let files = vec![
        map_file("m_one", &map),
        types_file("props", &types),
        (
            "levels/_manifest.ymf".to_string(),
            manifest_bytes(&[(
                "m_one",
                group_flags::TIMED | group_flags::WEATHER,
                NIGHT,
                &["RAIN", "THUNDER"],
            )]),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let kept = dir.path().join("kept.index");
    let (built, report) = refresh(&source, &kept);
    assert!(
        matches!(report.kept, KeptState::Absent),
        "{:?}",
        report.kept
    );
    let (index, report) = refresh(&source, &kept);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );
    assert_eq!((report.read.len(), report.opened), (0, 0));

    let loose = ModelFiles {
        drawable: hash("loose_model"),
        dictionary: None,
        textures: Some(hash("loose_txd")),
    };
    let in_dictionary = ModelFiles {
        drawable: hash("in_dictionary"),
        dictionary: Some(hash("props_dict")),
        textures: Some(hash("props_txd")),
    };
    let gate = Gate::TimedWeather {
        hours: NIGHT,
        types: vec![hash("RAIN"), hash("THUNDER")],
    };
    // What the index read and what it kept say the same of every archetype and of the group.
    for index in [&built, &index] {
        assert_eq!(model_of(index, hash("loose")), Some(Model::Files(loose)));
        assert_eq!(
            model_of(index, hash("in_dictionary")),
            Some(Model::Files(in_dictionary))
        );
        assert_eq!(model_of(index, hash("assetless")), None);
        let map = index.map(hash("m_one")).expect("the map is indexed");
        assert_eq!(request_of(index, map), Request::Group(&gate));
    }

    let mut forest = WorldForest::new(&index, &source, ROOMY);
    let view = at(0.0, 0.0, 0.0);
    // A rainy night lets the map in; a clear one and a rainy noon do not.
    for (hour, weather, streams) in [(2, "RAIN", true), (2, "CLEAR", false), (12, "RAIN", false)] {
        let streamed = forest.stream(&view, &WorldState::at(hour, weather));
        assert_eq!(
            streamed.wanted.len(),
            usize::from(streams),
            "{hour} {weather}"
        );
    }
    forest.stream(&view, &WorldState::at(2, "RAIN"));
    let selected = forest.select(&view, &all_ready);
    let models: Vec<Model> = selected.instances.iter().map(|i| i.model).collect();
    assert_eq!(models, [Model::Files(loose), Model::Files(in_dictionary)]);
    let needed = selected.files();
    let set = |names: &[&str]| hashes(names).into_iter().collect();
    assert_eq!(needed.drawables, set(&["loose_model"]));
    assert_eq!(needed.drawable_dictionaries, set(&["props_dict"]));
    assert_eq!(
        needed.texture_dictionaries,
        set(&["loose_txd", "props_txd"])
    );
}

#[test]
fn a_map_streams_by_its_entities_extents_where_its_streaming_ones_do_not_hold_them() {
    // Its streaming extents are a small box far off, which a tree of them answers nowhere near here.
    let loose = MapOf {
        streaming: cube([9000.0, 9000.0, 0.0], 1.0),
        ..map_around(cube([0.0; 3], 100.0), vec![ent("hd_a", [50.0, 50.0, 0.0])])
    };
    // Its streaming extents end 49 m short of the view on each of two axes: 69 m off as a circle measures.
    let cornered = MapOf {
        streaming: cube([-1000.0, -1000.0, -500.0], 1000.0 - PRESTREAM + 1.0),
        extents: cube([-600.0, -600.0, 0.0], 10.0),
        ..map_around(
            cube([0.0; 3], 1.0),
            vec![ent("hd_b", [-600.0, -600.0, 0.0])],
        )
    };
    let files = vec![
        map_file("m_loose", &loose),
        map_file("m_cornered", &cornered),
        family_types(),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    let view = at(0.0, 0.0, 0.0);
    // The rules want both: each is within 50 m of its box along every axis.
    for name in ["m_loose", "m_cornered"] {
        let map = index.map(hash(name)).expect("the map is indexed");
        let boxes = map_box(map).expect("it states its boxes");
        assert!(boxes.wanted(&view, map.content_flags), "{name}");
    }
    assert!(index.maps_in_column(0.0, 0.0).is_empty());
    assert!(index.maps_near_column(0.0, 0.0, PRESTREAM).is_empty());
    let streamed = forest.stream(&view, &noon());
    let mut names: Vec<&str> = (streamed.wanted.iter())
        .map(|w| w.map.name.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["m_cornered", "m_loose"]);
}

#[test]
fn an_entity_that_states_no_level_stands_as_an_orphan_and_a_link_into_a_map_the_mount_lacks_is_counted(
) {
    let entities = vec![
        Ent {
            lod_dist: 15.0,
            level: NO_LEVEL,
            ..ent("hd_a", [0.0; 3])
        },
        // It names entity 0 of a parent map, and its map's parent is in no archive.
        Ent {
            lod_dist: 40.0,
            parent: Some((0, true)),
            ..ent("hd_b", [0.0, 100.0, 0.0])
        },
    ];
    let map = MapOf {
        parent: "m_gone",
        ..map_around(cube([0.0; 3], 100.0), entities)
    };
    let dir = install(&[("x64a.rpf", vec![map_file("m_one", &map), family_types()])]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    // 17 m off: an orphan that draws to 15 m is three fifths there over its band of 5 m.
    let selected = seen(&mut forest, &at(17.0, 0.0, 0.0));
    assert_eq!(drawn(&selected), hashes(&["hd_a"]));
    assert_eq!(selected.instances[0].level, Level::OrphanHd);
    assert!((selected.instances[0].fade - 0.6).abs() < 1.0 / 255.0);
    assert_eq!(forest.misfits(), (1, 1));
    // The entity whose parent map is lacking draws by its own place and distance.
    let other = seen(&mut forest, &at(0.0, 130.0, 0.0));
    assert_eq!(drawn(&other), hashes(&["hd_b"]));
    assert_eq!(other.instances[0].level, Level::Hd);
}

#[test]
fn an_entity_that_states_no_distance_takes_its_archetype_s_and_a_scale_below_one_keeps_its_radius()
{
    // Forty times the size its archetype states, and no distance of its own: the archetype's 500 m.
    let big = Ent {
        scale: 40.0,
        ..ent("hd_a", [0.0; 3])
    };
    let map = map_around(cube([-10.0; 3], 20.0), vec![big]);
    let dir = install(&[("x64a.rpf", vec![map_file("m_one", &map), family_types()])]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);
    let fade_of = |forest: &mut WorldForest<'_>, view: &View| {
        let selected = seen(forest, view);
        selected.instances.first().map_or(0.0, |i| i.fade)
    };

    // Half into the band past 500 m it is half there, and past the band it is gone.
    let stated = index.archetype(hash("hd_a")).expect("declared").lod_dist;
    assert_eq!(stated, 500.0);
    let half = fade_of(&mut forest, &at(stated + FADE / 2.0, 0.0, 0.0));
    assert!((half - 0.5).abs() < 1.0 / 255.0, "{half}");
    assert_eq!(
        fade_of(&mut forest, &at(stated + FADE + 1.0, 0.0, 0.0)),
        0.0
    );

    // Half the scale shortens only what lies past its radius: half the diagonal of the archetype's
    // box of 2 m a side, forty times over.
    let radius = Vec3::splat(2.0).length() / 2.0 * 40.0;
    let reach = radius + 0.5 * (stated - radius);
    let away = reach + FADE / 4.0;
    let halved = View::at(Vec3::new(away, 0.0, 0.0), 0.5);
    let selected = seen(&mut forest, &halved);
    assert_eq!(drawn(&selected), hashes(&["hd_a"]));
    assert!(
        (selected.instances[0].fade - 0.75).abs() < 1.0 / 255.0,
        "{}",
        selected.instances[0].fade
    );
    // And its matrix carries its scale.
    let placed = selected.instances[0].to_world;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        assert!((placed.transform_vector3(axis).length() - 40.0).abs() < 1e-3);
    }
}

#[test]
fn of_the_maps_a_view_wants_the_nearest_is_read_first() {
    // In the mount's order the farthest comes first and the nearest second.
    let around = |x: f32, archetype| {
        map_around(
            cube([x, 0.0, 0.0], 10.0),
            vec![ent(archetype, [x + 5.0, 5.0, 5.0])],
        )
    };
    let files = vec![
        map_file("m_a", &around(400.0, "hd_a")),
        map_file("m_b", &around(0.0, "hd_b")),
        map_file("m_c", &around(200.0, "lod_p")),
        family_types(),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let in_mount: Vec<&str> = index.maps().iter().map(|m| m.name.as_str()).collect();
    assert_eq!(in_mount, ["m_a", "m_b", "m_c"]);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    let view = at(5.0, 5.0, 5.0);
    let mut read = Vec::new();
    for left in [2, 1, 0] {
        let streamed = forest.stream_some(&view, &noon(), 1);
        read.extend(streamed.read.iter().map(|map| map.name.clone()));
        assert_eq!(streamed.waiting.len(), left);
    }
    assert_eq!(read, ["m_b", "m_c", "m_a"]);
}

#[test]
fn a_model_the_view_s_velocity_brings_into_range_is_named_ahead_of_it() {
    let near = Ent {
        lod_dist: 40.0,
        ..ent("hd_a", [0.0; 3])
    };
    let map = map_around(cube([-10.0; 3], 20.0), vec![near]);
    let dir = install(&[("x64a.rpf", vec![map_file("m_one", &map), family_types()])]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);
    let loading = |_: u32| NodeState::Loading;

    // 100 m off, past its 40 m and its band: at rest nothing is waited for or asked ahead.
    let mut view = at(100.0, 0.0, 0.0);
    forest.stream(&view, &noon());
    let at_rest = forest.select(&view, &loading);
    assert!(at_rest.waiting.is_empty() && at_rest.ahead.is_empty());
    // Closing at 60 m a second, a second on it is 40 m off, in range.
    view.velocity = Vec3::new(-60.0, 0.0, 0.0);
    let closing = forest.select(&view, &loading);
    assert_eq!(closing.ahead, [hash("hd_a")]);
    assert!(closing.waiting.is_empty() && closing.instances.is_empty());
    // In range it is waited for, and no longer ahead.
    let within = at(30.0, 0.0, 0.0);
    let waited = forest.select(&within, &loading);
    assert_eq!(waited.waiting, [hash("hd_a")]);
    assert!(waited.ahead.is_empty());
}

#[test]
fn two_instances_of_one_interior_each_bring_its_entities_and_a_link_inside_it_holds() {
    // A shelf 5 m into the shop, and the far model that stands for it past 50 m.
    let shelf_far = Ent {
        lod_dist: 200.0,
        child_lod_dist: 50.0,
        level: LOD,
        num_children: 1,
        ..ent("shelf_lod", [5.0, 0.0, 0.0])
    };
    let shelf = Ent {
        lod_dist: 20.0,
        parent: Some((0, false)),
        ..ent("shelf", [5.0, 0.0, 0.0])
    };
    let shop = |y: f32| Ent {
        lod_dist: 200.0,
        level: ORPHAN,
        interior: true,
        ..ent("v_shop", [1000.0, y, 0.0])
    };
    let street = map_around(
        cube([900.0, -100.0, -10.0], 200.0),
        vec![shop(0.0), ent("shelf", [1000.0, -90.0, 0.0]), shop(60.0)],
    );
    let types = vec![
        Arch {
            interior: Some(vec![shelf_far, shelf]),
            ..arch("v_shop", 0.0)
        },
        arch("shelf_lod", 300.0),
        arch("shelf", 30.0),
    ];
    let files = vec![map_file("m_street", &street), types_file("shop", &types)];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    // Between the two shops, some 30 m from each shelf: within the 50 m its far model states.
    let between = at(1000.0, 30.0, 0.0);
    let streamed = forest.stream(&between, &noon());
    let brought: Vec<u32> = streamed.interiors.iter().map(|i| i.entity).collect();
    assert_eq!(brought, [0, 2]);
    let selected = forest.select(&between, &all_ready);
    let shelves: Vec<(Option<u32>, u32, Vec3)> = (selected.instances.iter())
        .map(|i| (i.interior, i.archetype, i.to_world.translation.into()))
        .collect();
    assert_eq!(
        shelves,
        [
            (Some(0), hash("shelf"), Vec3::new(1005.0, 0.0, 0.0)),
            (Some(2), hash("shelf"), Vec3::new(1005.0, 60.0, 0.0)),
        ]
    );
    let keys: Vec<u64> = selected.instances.iter().map(|i| i.node.0).collect();
    assert_eq!(
        keys,
        [
            interior_key(hash("m_street"), 0),
            interior_key(hash("m_street"), 2)
        ]
    );
    assert!(selected.instances.iter().all(|i| i.entity == 1));

    // 90 m up, past the 50 m and its band: each shelf gives way to its far model.
    let above = at(1000.0, 30.0, 90.0);
    let far = seen(&mut forest, &above);
    assert_eq!(drawn(&far), [hash("shelf_lod"); 2]);
    assert!(far.instances.iter().all(|i| i.entity == 0));
}

/// A source that gives no bytes for the files whose path ends as told.
struct Hiding<'a> {
    source: &'a dyn WorldSource,
    ends: &'static str,
}

impl WorldSource for Hiding<'_> {
    fn identity(&self) -> String {
        self.source.identity()
    }

    fn archives(&self) -> Vec<or_depends::world_index::Archive> {
        self.source.archives()
    }

    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&or_title::vfs::EntryFacts)) {
        self.source.entries(archive, visit);
    }

    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
        (!logical.ends_with(self.ends))
            .then(|| self.source.open(archive, logical))
            .flatten()
    }

    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
        (!logical.ends_with(self.ends))
            .then(|| self.source.open_head(archive, logical, limit))
            .flatten()
    }
}

#[test]
fn a_file_that_gives_the_forest_nothing_is_said_with_why_and_opened_once() {
    // A child of a parent map whose file is no map, beside an instance of an archetype that is no
    // interior, one of an archetype no type file declares and one of an interior whose entities
    // begin with a pointer to nothing.
    let child = Ent {
        lod_dist: 40.0,
        parent: Some((0, true)),
        ..ent("hd_a", [0.0; 3])
    };
    let placing = |archetype, x| Ent {
        lod_dist: 60.0,
        level: ORPHAN,
        interior: true,
        ..ent(archetype, [x, 0.0, 0.0])
    };
    let yard = MapOf {
        parent: "m_bad",
        ..map_around(
            cube([-50.0; 3], 100.0),
            vec![
                child,
                placing("hd_b", 5.0),
                placing("v_nowhere", 10.0),
                placing("v_holed", 15.0),
            ],
        )
    };
    let holed = Arch {
        interior: Some(vec![ent("hd_a", [1.0, 0.0, 0.0])]),
        holed: true,
        ..arch("v_holed", 0.0)
    };
    let alley = map_around(cube([-50.0; 3], 100.0), vec![ent("hd_b", [0.0, 20.0, 0.0])]);
    let files = vec![
        map_file("m_yard", &yard),
        map_file("m_alley", &alley),
        ("levels/m_bad.ymap".to_string(), b"no map at all".to_vec()),
        family_types(),
        types_file("holed", &[holed]),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    assert!(!index.map(hash("m_bad")).expect("it is listed").read);
    // The alley's file is listed and gives no bytes when asked.
    let hiding = Hiding {
        source: &source,
        ends: "m_alley.ymap",
    };
    let counting = Counting::new(&hiding);
    let mut forest = WorldForest::new(&index, &counting, ROOMY);
    let opened = || counting.counts().1.iter().sum::<u32>();

    let view = at(0.0, 0.0, 0.0);
    for _ in 0..3 {
        let streamed = forest.stream(&view, &noon());
        let wanted: Vec<&str> = (streamed.wanted.iter())
            .map(|w| w.map.name.as_str())
            .collect();
        assert_eq!(wanted, ["m_yard"]);
        assert!(streamed.waiting.is_empty());
        let mut unread: Vec<(&str, Unread)> = (streamed.unread.iter())
            .map(|(map, why)| (map.name.as_str(), *why))
            .collect();
        unread.sort_by_key(|(name, _)| *name);
        assert_eq!(
            unread,
            [("m_alley", Unread::Absent), ("m_bad", Unread::NotAMap)]
        );
        assert!(streamed.interiors.is_empty());
        let interiors: Vec<(u32, u32, Unread)> = (streamed.interiors_unread.iter())
            .map(|(at, why)| (at.entity, at.archetype, *why))
            .collect();
        assert_eq!(
            interiors,
            [
                (1, hash("hd_b"), Unread::NotAnInterior),
                (2, hash("v_nowhere"), Unread::Undeclared),
                (3, hash("v_holed"), Unread::EntityUnread),
            ]
        );
        // The yard, the alley, the parent that is no map, and the type files of the archetype that
        // is no interior and of the interior that did not read whole: each asked for once, however
        // often the view asks.
        assert_eq!(opened(), 5);
    }
    assert_eq!(Unread::NotAMap.to_string(), "it does not read as a map");
    // The child stands alone by its own distance, the instance of an archetype no type file
    // declares stands as a box, and the instance of one that is no interior draws nothing.
    let selected = forest.select(&view, &all_ready);
    let models: Vec<(u32, Model)> = (selected.instances.iter())
        .map(|i| (i.entity, i.model))
        .collect();
    let child_files = ModelFiles {
        drawable: hash("hd_a"),
        dictionary: None,
        textures: None,
    };
    assert_eq!(
        models,
        [
            (0, Model::Files(child_files)),
            (2, Model::Box(NoModel::Undeclared)),
        ]
    );
}

#[test]
fn a_map_one_of_whose_entities_did_not_read_is_refused_for_no_link_into_it_holds() {
    // The first of its entities is a pointer to nothing: read past, the parent would stand at
    // place 0 and the two children would name place 1, the first of themselves.
    let mut entities = vec![parent()];
    entities.extend(children(1, false));
    let holed = map_around(cube([-200.0, -200.0, -10.0], 400.0), entities);
    let mut stated = vec![or_io::meta::Value::Pointer(None)];
    stated.extend(holed.entities.iter().map(entity_value));
    let whole = map_around(
        cube([-200.0, -200.0, -10.0], 400.0),
        vec![ent("hd_b", [70.0, 0.0, 0.0])],
    );
    let files = vec![
        (
            "levels/m_holed.ymap".to_string(),
            map_bytes_of("m_holed", &holed, stated),
        ),
        map_file("m_whole", &whole),
        family_types(),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let game = mount(dir.path());
    let source = MountedWorld::new(&game, dir.path());
    let (index, _) = WorldIndex::build(&source, None);
    assert!(index.map(hash("m_holed")).expect("it is listed").read);
    let mut forest = WorldForest::new(&index, &source, ROOMY);

    let view = at(70.0, 0.0, 0.0);
    let streamed = forest.stream(&view, &noon());
    let unread: Vec<(&str, Unread)> = (streamed.unread.iter())
        .map(|(map, why)| (map.name.as_str(), *why))
        .collect();
    assert_eq!(unread, [("m_holed", Unread::EntityUnread)]);
    let wanted: Vec<&str> = (streamed.wanted.iter())
        .map(|w| w.map.name.as_str())
        .collect();
    assert_eq!(wanted, ["m_whole"]);
    assert_eq!(drawn(&forest.select(&view, &all_ready)), hashes(&["hd_b"]));
}
