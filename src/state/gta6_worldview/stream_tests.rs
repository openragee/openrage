use std::path::PathBuf;

use bytemuck::Zeroable;
use or_io::gta6::map::stream::{
    GrassInstance, StreamEntity, StreamGrassList, StreamLink, StreamMap, StreamPropList,
};
use or_io::gta6::map::Aabb;
use or_world::streaming::{Category, Extent, Kind, Level, ParentRef};

use super::super::index::tests_support::{index_of, index_with};
use super::super::index::{MapRow, ALL_HOURS};
use super::read::{Instances, List};
use super::*;

const TREE: u64 = 100;
const LAMP: u64 = 200;
const ROOM: u64 = 300;
const FERN: u64 = 400;
const STAND_IN: u64 = 500;
/// The hours from eight in the evening to six in the morning.
const NIGHT: u32 = 0x00F0_003F;

fn entity(word: u64, at: [f32; 3], lod_dist: f32) -> StreamEntity {
    StreamEntity {
        archetype_word: word,
        archetype_name: 1,
        position: at,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale_xy: 1.0,
        scale_z: 1.0,
        lod_dist,
        child_lod_dist: 0.0,
        num_children: 0,
        lod_level: 0,
        flags: 0,
        interior: false,
        parent: None,
    }
}

fn boxed(min: [f32; 3], max: [f32; 3]) -> Aabb {
    Aabb { min, max }
}

/// An instance that stands at `at` with its axes along the world's.
fn plain(at: [f32; 3]) -> [[f32; 4]; 3] {
    [
        [1.0, 0.0, 0.0, at[0]],
        [0.0, 1.0, 0.0, at[1]],
        [0.0, 0.0, 1.0, at[2]],
    ]
}

/// A list of lamps of level 1 that states no distance, with these instances.
fn lamps(instances: Vec<[[f32; 4]; 3]>) -> StreamPropList {
    StreamPropList {
        archetype_word: LAMP,
        archetype_name: 2,
        position: [15.0, 5.0, 5.0],
        lod_dist: 0.0,
        child_lod_dist: 0.0,
        instance_lod_dist: 50.0,
        lod_level: 1,
        flags: 0,
        num_children: 0,
        parent: None,
        batch_aabb: boxed([0.0; 3], [30.0, 10.0, 10.0]),
        tints: vec![[51, 102, 255, 255]; instances.len()],
        parent_instances: Vec::new(),
        instances,
    }
}

/// A map of two entities, an interior and a list of two lamps, whose parent map is named 77.
fn made_up() -> StreamMap {
    let mut child = entity(TREE, [10.0, 0.0, 0.0], -1.0);
    child.parent = Some(StreamLink {
        map: Some(0),
        index: 3,
        instance: Some(1),
    });
    let mut interior = entity(ROOM, [0.0, 50.0, 0.0], 80.0);
    interior.interior = true;
    // Turned a quarter about Z and moved to (5, 6, 7): its x axis is the world's y.
    let turned = [
        [0.0, 1.0, 0.0, 5.0],
        [-1.0, 0.0, 0.0, 6.0],
        [0.0, 0.0, 1.0, 7.0],
    ];
    StreamMap {
        name: 55,
        block: 9,
        parent: 0,
        flags: 0,
        content_flags: 1,
        streaming_extents: boxed([-100.0; 3], [100.0; 3]),
        entities_extents: boxed([-50.0; 3], [50.0; 3]),
        lod_parents: vec![77],
        entities: vec![entity(TREE, [1.0, 2.0, 3.0], 40.0), child, interior],
        prop_lists: vec![lamps(vec![turned, plain([20.0, 0.0, 0.0])])],
        grass_lists: 0,
        grass: Vec::new(),
    }
}

/// A grass instance `metres` along x from the corner of a box 655.35 m a side, of the least scale.
fn blade(metres: u16, rotation: u8, guid: u16) -> GrassInstance {
    GrassInstance {
        position: [metres * 100, 0, 0],
        normal: [128, 128],
        colour: [60, 115, 109],
        scale: 0,
        scale_z: 0,
        rotation,
        guid,
    }
}

/// A list of ferns that fade from 40 m to 50 m, with these instances.
fn ferns(instances: Vec<GrassInstance>) -> StreamGrassList {
    StreamGrassList {
        archetype_word: FERN,
        archetype_name: 4,
        position: [40.0, 0.0, 0.0],
        lod_dist: 200.0,
        lod_level: 5,
        num_children: 0,
        batch_aabb: boxed([0.0; 3], [655.35; 3]),
        scale_range: [2.0, 4.0],
        lod_fade_start_dist: 40.0,
        instance_lod_dist: 50.0,
        orient_to_terrain: 1.0,
        slod: false,
        archetypes: Vec::new(),
        instances,
    }
}

/// The made-up map with a list of three ferns 10, 45 and 80 m along x, and a list of far stand-ins.
fn grown() -> StreamMap {
    let mut map = made_up();
    let mut far = ferns(vec![blade(10, 0, 7)]);
    (far.archetype_word, far.slod, far.lod_level) = (STAND_IN, true, 2);
    far.archetypes = vec![(4, FERN)];
    map.grass = vec![
        ferns(vec![blade(10, 0, 0), blade(45, 0, 0), blade(80, 0, 0)]),
        far,
    ];
    map.grass_lists = 2;
    map
}

/// An index whose tree draws to 120 m, lamp to 300 m and fern to 150 m, each with a model, a far
/// stand-in with one too, and a room with none.
fn made_up_index() -> WorldIndex {
    index_of(
        vec![PathBuf::from("made-up.rpf")],
        &[
            (TREE, 120.0, Some(0x1000)),
            (LAMP, 300.0, Some(0x2000)),
            (ROOM, 80.0, None),
            (FERN, 150.0, Some(0x3000)),
            (STAND_IN, 900.0, Some(0x4000)),
        ],
    )
}

fn placed(list: &List) -> &[Rows] {
    match &list.instances {
        Instances::Placed(rows) => rows,
        Instances::Grass(_) | Instances::Cards(_) => panic!("a list of placed instances"),
    }
}

/// Each instance of `list` that draws for a camera at the origin, with how opaque it is.
fn drawn_from(list: &List, scale: f32, skipped: &[u32]) -> Vec<(Rows, f32)> {
    let mut out = Vec::new();
    list.each(Vec3::ZERO, scale, skipped, |rows, fade, _| {
        out.push((rows, fade))
    });
    out
}

#[test]
fn a_grass_instance_is_tinted_by_its_colour_squared_and_a_prop_by_its_tint() {
    let read = read_map(&made_up_index(), &grown());
    let mut tints = Vec::new();
    read.loaded.lists[1].each(Vec3::ZERO, 1.0, &[], |_, _, tint| tints.push(tint));
    let squared = [60.0f32, 115.0, 109.0].map(|c| (c / 255.0) * (c / 255.0));
    assert_eq!(tints, vec![squared; 2]);
    // A prop's `Tint` of 51, 102, 255: its first three bytes over 255, the alpha left out.
    let mut props = Vec::new();
    read.loaded.lists[0].each(Vec3::ZERO, 1.0, &[], |_, _, tint| props.push(tint));
    assert_eq!(props, vec![[0.2, 0.4, 1.0]; 2]);
}

fn streamer_with(map: &StreamMap) -> Streamer {
    let index = Arc::new(made_up_index());
    let mut streamer = Streamer::new(Arc::clone(&index));
    streamer.wanted.insert(map.name, 0.0);
    assert!(streamer.insert(map.name, read_map(&index, map)));
    streamer
}

/// How many instances of `selected` draw with the model at `offset`.
fn count(selected: &Selected, offset: u64) -> usize {
    (selected.instances.iter())
        .filter(|i| selected.models[i.model as usize].offset == offset)
        .count()
}

#[test]
fn a_map_s_nodes_take_their_archetype_s_distance_and_name_their_parent_by_its_map() {
    let read = read_map(&made_up_index(), &made_up());
    assert_eq!(read.nodes.len(), 4, "three entities, then one list");
    assert_eq!(read.nodes[0].lod_dist, 40.0);
    // An entity that states no distance takes its archetype's.
    assert_eq!(read.nodes[1].lod_dist, 120.0);
    assert_eq!(
        read.nodes[1].parent,
        Some(ParentRef { map: 77, node: 3 }),
        "the first map of its parent list, and the fourth of that map's nodes"
    );
    assert_eq!(read.nodes[1].instance, Some(1));
    assert_eq!(read.nodes[0].parent, None);
    assert_eq!(
        (read.nodes[0].level, read.nodes[3].level),
        (Level::Hd, Level::Lod)
    );
    // The list is measured to its box and takes its archetype's distance too.
    assert!(matches!(read.nodes[3].extent, Extent::Box(..)));
    assert_eq!(read.nodes[3].lod_dist, 300.0);
    assert_eq!(read.nodes[3].position, Vec3::new(15.0, 5.0, 5.0));
    // An interior's own model is not drawn: its contents stand in a space nothing here places.
    assert!(read.loaded.drawn[0].model.is_some());
    assert!(read.loaded.drawn[2].model.is_none());
}

#[test]
fn an_instance_s_axes_become_the_rows_a_point_is_placed_by() {
    let map = made_up();
    let read = read_map(&made_up_index(), &map);
    let rows = placed(&read.loaded.lists[0]);
    let point = Vec3::new(1.0, 2.0, 3.0);
    for (stated, rows) in map.prop_lists[0].instances.iter().zip(rows) {
        let by_the_reader = or_io::gta6::map::stream::place(stated, point.to_array());
        assert_eq!(place(rows, point).to_array(), by_the_reader);
    }
    // The turned instance puts the model's x axis along the world's y.
    assert_eq!(place(&rows[0], Vec3::X), Vec3::new(5.0, 7.0, 7.0));
    // An entity's rows place its own origin at its position.
    assert_eq!(
        place(&read.loaded.entities[0], Vec3::ZERO),
        Vec3::new(1.0, 2.0, 3.0)
    );
    assert_eq!(stretch(&rows[0]), 1.0);
}

#[test]
fn a_selection_draws_each_entity_and_each_instance_with_its_model_and_waits_for_the_rest() {
    let index = Arc::new(made_up_index());
    let mut map = made_up();
    map.entities[1].parent = None;
    let mut streamer = streamer_with(&map);

    let view = View::at(Vec3::ZERO, 1.0);
    // Nothing is on the device yet: each model is waited for once, and nothing draws.
    let none = streamer.select(&view, &|_| Some(false));
    assert!(none.instances.is_empty() && none.chunks.is_empty());
    let mut models: Vec<u64> = none.wanted.iter().map(|(_, m)| m.offset).collect();
    models.sort_unstable();
    assert_eq!(models, vec![0x1000, 0x2000]);

    // What is waited for is handed out before the selection is whole, each model at its band.
    let mut asked = Vec::new();
    let again = streamer.select_asking(&view, &|_| Some(false), &mut |wanted, bands| {
        asked = wanted.keys().map(|m| (m.offset, bands[m])).collect();
    });
    asked.sort_by_key(|(offset, _)| *offset);
    assert_eq!(asked.len(), again.wanted.len());
    assert!(asked.iter().all(|(_, band)| *band >= BAND_OTHER));

    // With every model there: two trees in one chunk, and both lamps of the list in another.
    let all = streamer.select(&view, &|_| Some(true));
    assert!(all.wanted.is_empty());
    assert_eq!((count(&all, 0x1000), count(&all, 0x2000)), (2, 2));
    assert_eq!(all.streaming.drawn, 4);
    assert_eq!(all.chunks.len(), 2);
    for chunk in &all.chunks {
        // A chunk's sphere holds the sphere of each instance in it.
        for i in &all.instances[chunk.from as usize..chunk.to as usize] {
            assert!(chunk.centre.distance(i.centre) + i.radius <= chunk.radius + 1e-3);
        }
    }
    // Each instance bears the fade its selection gave it, well inside every distance: the trees,
    // of the high level, whole at once; the lamps, a list of level 1 with no parent that was
    // waited for a selection ago, one step in.
    for i in &all.instances {
        let lamp = all.models[i.model as usize].offset == 0x2000;
        let expected = if lamp { 51.0 / 255.0 } else { 1.0 };
        assert!((i.fade - expected).abs() < 1e-4, "{}", i.fade);
    }
    // A model that will never come draws nothing and is not waited for.
    let never = streamer.select(&view, &|_| None);
    assert!(never.wanted.is_empty() && never.instances.is_empty());
    // A map let go before its body arrived is not kept.
    assert!(!streamer.insert(56, read_map(&index, &map)));
    assert!(!streamer.loaded.contains_key(&56));
}

#[test]
fn a_grass_list_is_a_node_after_the_prop_lists_and_a_list_of_far_stand_ins_draws_nothing() {
    let read = read_map(&made_up_index(), &grown());
    assert_eq!(
        read.nodes.len(),
        6,
        "three entities, a prop list, two grass lists"
    );
    let (ferns, far) = (&read.nodes[4], &read.nodes[5]);
    // A grass list is a list in a box, by the distance it states, and names no parent.
    assert!(matches!(ferns.extent, Extent::Box(..)));
    assert_eq!((ferns.lod_dist, ferns.parent), (200.0, None));
    assert_eq!((ferns.level, far.level), (Level::OrphanHd, Level::Slod1));
    // Its instances fade from where the list says to where it says they are gone.
    assert_eq!(read.loaded.lists[1].reach, Some((40.0, 50.0)));
    assert!(read.loaded.drawn[4].model.is_some());
    // A far stand-in's model is named by an id no map states: its list holds its instances as cards.
    assert!(read.loaded.drawn[5].model.is_none());
    let cards = &read.loaded.lists[2].instances;
    assert!(matches!(cards, Instances::Cards(c) if c.records.len() == 1));
}

#[test]
fn a_list_of_far_stand_ins_fades_out_whole_as_the_list_under_it_fades_in() {
    let mut index = made_up_index();
    index.slod = vec![0; 8];
    index.slod[7] = FERN;
    let index = Arc::new(index);
    // The lamps stand in for the far list's one instance, the sixth node of their own map.
    let mut map = grown();
    map.prop_lists[0].parent = Some(StreamLink {
        map: None,
        index: 5,
        instance: None,
    });
    map.prop_lists[0].parent_instances = vec![0, 0];
    let select = |map: &StreamMap| {
        let mut streamer = Streamer::new(Arc::clone(&index));
        streamer.wanted.insert(55, 0.0);
        assert!(streamer.insert(55, read_map(&index, map)));
        // Beside the lamps, which draw whole, and 567 m from the far list's centre, inside its distance.
        streamer.select(&View::at(Vec3::ZERO, 1.0), &|_| Some(true))
    };
    let id_of = |selected: &Selected| {
        let [cards] = &selected.cards[..] else {
            panic!("one list of cards, not {}", selected.cards.len());
        };
        u16::from_le_bytes([cards.records[0][14], cards.records[0][15]])
    };
    // Its instance stands where the tree 10 m along x draws: it yields to the tree.
    assert_eq!(id_of(&select(&map)), super::slod::HIDDEN);
    // A metre off, a far list that states no child draws on beside them, every card kept.
    map.grass[1].instances[0] = blade(11, 0, 7);
    let alone = select(&map);
    assert!(count(&alone, 0x2000) > 0, "the lamps draw");
    assert_ne!(id_of(&alone), super::slod::HIDDEN);
    // One that states the lamps as its child gives way to them whole.
    map.grass[1].num_children = 1;
    let given_way = select(&map);
    assert!(count(&given_way, 0x2000) > 0 && given_way.cards.is_empty());
}

#[test]
fn a_drawn_list_of_the_batch_lod_level_leaves_out_the_instances_of_its_batch_parent_it_names() {
    let index = Arc::new(made_up_index());
    let mut far = lamps(vec![
        plain([5.0, 5.0, 5.0]),
        plain([25.0, 5.0, 5.0]),
        plain([15.0, 5.0, 5.0]),
    ]);
    (far.flags, far.num_children, far.lod_dist) = (1 << 12, 1, 2000.0);
    let parent_map = StreamMap {
        name: 77,
        lod_parents: Vec::new(),
        entities: Vec::new(),
        prop_lists: vec![far],
        ..made_up()
    };
    // The near list stands in for the far list's third lamp.
    let mut near = lamps(vec![plain([15.0, 5.0, 6.0])]);
    near.parent = Some(StreamLink {
        map: Some(0),
        index: 0,
        instance: None,
    });
    near.parent_instances = vec![2];
    let lamps_drawn = |near: &StreamPropList| {
        let child_map = StreamMap {
            entities: Vec::new(),
            prop_lists: vec![near.clone()],
            ..made_up()
        };
        let mut streamer = Streamer::new(Arc::clone(&index));
        for map in [&parent_map, &child_map] {
            streamer.wanted.insert(map.name, 0.0);
            assert!(streamer.insert(map.name, read_map(&index, map)));
        }
        let selected = streamer.select(&View::at(Vec3::new(15.0, 5.0, 5.0), 1.0), &|_| Some(true));
        let mut at: Vec<[i32; 3]> = (selected.instances.iter())
            .filter(|i| selected.models[i.model as usize].offset == 0x2000)
            .map(|i| [i.rows[0][3], i.rows[1][3], i.rows[2][3]].map(|v| v as i32))
            .collect();
        at.sort_unstable();
        at
    };
    // Of the batch LOD level, it draws in the third lamp's place.
    near.flags = 1 << 11;
    assert_eq!(lamps_drawn(&near), vec![[5, 5, 5], [15, 5, 6], [25, 5, 5]]);
    // Of no batch level, it leaves every lamp of its parent drawn.
    near.flags = 0;
    assert_eq!(
        lamps_drawn(&near),
        vec![[5, 5, 5], [15, 5, 5], [15, 5, 6], [25, 5, 5]]
    );
    // One that names none leaves out all of them.
    (near.flags, near.parent_instances) = (1 << 11, Vec::new());
    assert_eq!(lamps_drawn(&near), vec![[15, 5, 6]]);
}

#[test]
fn a_list_of_far_stand_ins_draws_as_cards_and_waits_for_no_model() {
    let mut index = made_up_index();
    index.slod = vec![0; 8];
    index.slod[7] = FERN;
    let index = Arc::new(index);
    let mut streamer = Streamer::new(Arc::clone(&index));
    streamer.wanted.insert(55, 0.0);
    assert!(streamer.insert(55, read_map(&index, &grown())));
    // No model is on the device, and the far stand-ins still draw, as one list of cards.
    let view = View::at(Vec3::ZERO, 1.0);
    let none = streamer.select(&view, &|_| Some(false));
    let [cards] = &none.cards[..] else {
        panic!("one list of cards, not {}", none.cards.len());
    };
    assert_eq!((cards.records.len(), cards.fade), (1, 1.0));
    assert_eq!(cards.distance, 50.0 * view.scales[Level::Slod1 as usize]);
    assert_eq!(none.species[7].archetype, FERN as u32);
    assert!(none
        .instances
        .iter()
        .all(|i| none.models[i.model as usize].offset != 0x4000));
    // Past its distance the list is gone.
    let away = streamer.select(&View::at(Vec3::splat(5000.0), 1.0), &|_| Some(true));
    assert!(away.cards.is_empty());
}

#[test]
fn a_grass_instance_draws_by_its_own_distance_and_fades_before_it_is_gone() {
    let read = read_map(&made_up_index(), &grown());
    let ferns = &read.loaded.lists[1];
    // 10 m is nearer than the fade's start, 45 m half way through it, 80 m past its end.
    let near = drawn_from(ferns, 1.0, &[]);
    let fades: Vec<f32> = near.iter().map(|(_, fade)| *fade).collect();
    assert_eq!(fades.len(), 2);
    assert!(
        fades[0] == 1.0 && (fades[1] - 0.5).abs() < 1e-3,
        "{fades:?}"
    );
    // Twice the distances: the fade runs from 80 m to 100 m, and all three draw.
    let scaled = drawn_from(ferns, 2.0, &[]);
    assert_eq!(scaled.len(), 3);
    assert!((scaled[2].1 - 1.0).abs() < 1e-3);
    // An instance at the very end of the fade is gone.
    assert_eq!(ferns.fade(50.0, 1.0), None);
    assert_eq!(ferns.fade(49.0, 1.0).map(|f| (f * 10.0).round()), Some(1.0));
    // A list whose fade starts past its end has no band.
    let abrupt = List {
        instances: Instances::Placed(Vec::new()),
        reach: Some((8191.0, 100.0)),
        stands_in_for: Vec::new(),
        batch_lod: false,
        tints: Vec::new(),
    };
    assert_eq!(
        (abrupt.fade(99.0, 1.0), abrupt.fade(101.0, 1.0)),
        (Some(1.0), None)
    );

    // In a selection the grass scale of 0.8 takes the fade from 32 m to 40 m: the fern at 45 m
    // is gone. The list is measured to its box's centre, 567 m off: it states enough to draw.
    let mut map = grown();
    map.grass[0].lod_dist = 700.0;
    let mut streamer = streamer_with(&map);
    let at_rest = streamer.select(&View::at(Vec3::ZERO, 1.0), &|_| Some(true));
    assert_eq!(count(&at_rest, 0x3000), 1);
    // A camera that draws a quarter farther gives the ferns the distances they state: a chunk for
    // the list, each instance with the fade of its own distance.
    let mut streamer = streamer_with(&map);
    let all = streamer.select(&View::at(Vec3::ZERO, 1.25), &|_| Some(true));
    assert_eq!(count(&all, 0x3000), 2);
    assert_eq!(count(&all, 0x4000), 0, "the far stand-ins");
    let ferns: Vec<&Instance> = (all.instances.iter())
        .filter(|i| all.models[i.model as usize].offset == 0x3000)
        .collect();
    assert!(ferns[0].fade == 1.0 && (ferns[1].fade - 0.5).abs() < 1e-3);
    assert_eq!(all.chunks.len(), 3, "the entities, the lamps, the ferns");
}

#[test]
fn a_map_s_nodes_say_what_scales_them_in_whole_metres() {
    let mut map = grown();
    map.entities[0].lod_dist = 40.7;
    map.entities[0].child_lod_dist = 12.9;
    let read = read_map(&made_up_index(), &map);
    let nodes = &read.nodes;
    assert_eq!((nodes[0].lod_dist, nodes[0].child_lod_dist), (40.0, 12.0));
    // An entity keeps its archetype's sphere; a list keeps its box's half diagonal.
    assert_eq!((nodes[0].kind, nodes[0].radius), (Kind::Entity, 1.0));
    let lamps = Vec3::new(30.0, 10.0, 10.0).length() / 2.0;
    assert_eq!((nodes[3].kind, nodes[3].radius), (Kind::Props, lamps));
    assert_eq!(nodes[4].kind, Kind::Grass);
    // An archetype of no load flags is of no category.
    assert!(nodes.iter().all(|n| n.category == Category::Other));
    // A plain grass list keeps the 200 m it states; one of far stand-ins draws as far as its
    // farthest instance, and scales as an entity.
    assert_eq!(nodes[4].lod_dist, 200.0);
    let farthest = (Vec3::splat(655.35).length() / 2.0 + 50.0).trunc();
    assert!(farthest > 200.0);
    assert_eq!((nodes[5].lod_dist, nodes[5].kind), (farthest, Kind::Entity));
}

#[test]
fn a_grass_instance_is_scaled_turned_and_tilted_to_its_ground_then_moved() {
    let mut list = ferns(vec![blade(10, 18, 0)]);
    // A normal of two middle bytes leans a step of the byte off straight up: 2 cm in 2 m.
    let at = |p: Vec3| p.abs_diff_eq(Vec3::ZERO, 2e-2);
    let origin = Vec3::from(list.position_of(&list.instances[0]));
    assert!((origin - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-2);
    // A quarter turn about its up puts the model's x along the world's y, at the least scale.
    let rows = grass::rows(&list, &list.instances[0], origin);
    assert!(at(place(&rows, Vec3::X)
        - origin
        - Vec3::new(0.0, 2.0, 0.0)));
    assert!(at(place(&rows, Vec3::Z)
        - origin
        - Vec3::new(0.0, 0.0, 2.0)));
    assert!((stretch(&rows) - 2.0).abs() < 1e-3);
    // On ground whose normal lies along x the model's up follows it.
    list.instances[0].normal = [255, 128];
    list.instances[0].rotation = 0;
    let tilted = grass::rows(&list, &list.instances[0], origin);
    let up = (place(&tilted, Vec3::Z) - origin) / 2.0;
    assert!(up.x > 0.99 && up.z.abs() < 0.01, "{up}");
    // A list that takes none of the ground's tilt stands upright on it.
    list.orient_to_terrain = 0.0;
    let upright = grass::rows(&list, &list.instances[0], origin);
    assert!(at(place(&upright, Vec3::Z)
        - origin
        - Vec3::new(0.0, 0.0, 2.0)));
}

#[test]
fn a_prop_list_with_no_parent_and_no_children_draws_each_instance_by_its_own_distance() {
    let mut map = made_up();
    let mut alone = lamps(vec![plain([20.0, 0.0, 0.0]), plain([80.0, 0.0, 0.0])]);
    alone.lod_level = 5;
    let mut under = alone.clone();
    under.parent = Some(StreamLink {
        map: Some(0),
        index: 0,
        instance: None,
    });
    map.prop_lists = vec![alone, under, lamps(vec![plain([80.0, 0.0, 0.0])])];
    let read = read_map(&made_up_index(), &map);
    let [alone, under, level_one] = &read.loaded.lists[..] else {
        panic!("three lists");
    };
    assert_eq!(alone.reach, Some((50.0, 50.0)));
    // 20 m is inside its 50 m and 80 m is not; at twice the distances both are.
    assert_eq!(drawn_from(alone, 1.0, &[]).len(), 1);
    assert_eq!(drawn_from(alone, 2.0, &[]).len(), 2);
    // An instance an entity stands in for is left out.
    assert!(drawn_from(alone, 1.0, &[0]).is_empty());
    // A list something else stands in for, or that stands in for another, draws whole.
    assert_eq!((under.reach, level_one.reach), (None, None));
    assert_eq!(drawn_from(under, 1.0, &[]).len(), 2);
    assert_eq!(drawn_from(under, 1.0, &[1]).len(), 1);
}

#[test]
fn a_time_archetype_draws_inside_its_hours_alone_and_is_not_waited_for_outside_them() {
    let index = Arc::new(made_up_index());
    let mut map = made_up();
    map.entities[1].parent = None;
    let mut read = read_map(&index, &map);
    // Both trees are of an archetype of the night; the lamps are of every hour.
    read.loaded.drawn[0].hours = NIGHT;
    read.loaded.drawn[1].hours = NIGHT;
    assert_eq!(read.loaded.drawn[3].hours, ALL_HOURS);
    let mut streamer = Streamer::new(Arc::clone(&index));
    streamer.wanted.insert(55, 0.0);
    assert!(streamer.insert(55, read));
    let view = View::at(Vec3::ZERO, 1.0);

    // At noon, where a world starts: no tree draws, and none is waited for.
    let noon = streamer.select(&view, &|_| Some(true));
    assert_eq!((count(&noon, 0x1000), count(&noon, 0x2000)), (0, 2));
    let waiting = streamer.select(&view, &|_| Some(false));
    let waited: Vec<u64> = waiting.wanted.iter().map(|(_, m)| m.offset).collect();
    assert_eq!(waited, vec![0x2000]);

    // At half past ten in the evening both do; the hour is the same until eleven.
    assert!(streamer.set_hour(22.5));
    assert!(!streamer.set_hour(22.9));
    let night = streamer.select(&view, &|_| Some(true));
    assert_eq!((count(&night, 0x1000), count(&night, 0x2000)), (2, 2));
    // Five in the morning is night still, six is not.
    assert!(streamer.set_hour(5.99));
    assert_eq!(count(&streamer.select(&view, &|_| Some(true)), 0x1000), 2);
    assert!(streamer.set_hour(6.0));
    assert_eq!(count(&streamer.select(&view, &|_| Some(true)), 0x1000), 0);
}

#[test]
fn an_hour_of_the_day_is_one_of_twenty_four() {
    for (hour, of) in [
        (0.0, 0),
        (0.99, 0),
        (12.0, 12),
        (23.99, 23),
        (24.0, 0),
        (-1.0, 23),
    ] {
        assert_eq!(hour_of(hour), of, "{hour}");
    }
    assert_eq!(hour_of(f32::NAN), 0);
}

/// A map of the base game named `name`, held back for the reason `held`, in the box `from` to
/// `to`, with the run `parents` of the index's parent names.
fn map_row(name: u32, held: u32, from: [f32; 3], to: [f32; 3], parents: (u32, u32)) -> MapRow {
    let mut boxes = [0.0; 12];
    for (slot, corner) in boxes.chunks_mut(3).zip([from, to, from, to]) {
        slot.copy_from_slice(&corner);
    }
    MapRow {
        name,
        held,
        parents_from: parents.0,
        parents_len: parents.1,
        boxes,
        modes: 1,
        ..MapRow::zeroed()
    }
}

fn wanted_by(streamer: &Streamer) -> Vec<u32> {
    let mut wanted: Vec<u32> = streamer.wanted.keys().copied().collect();
    wanted.sort_unstable();
    wanted
}

#[test]
fn the_maps_wanted_are_those_at_rest_around_the_camera_and_their_parents() {
    let near = ([-10.0; 3], [10.0; 3]);
    let far = ([5000.0; 3], [5010.0; 3]);
    let maps = vec![
        // Around the camera, with the parent 3; around it but script managed; far off.
        map_row(1, 0, near.0, near.1, (0, 1)),
        map_row(2, 2, near.0, near.1, (0, 0)),
        map_row(4, 0, far.0, far.1, (0, 0)),
        // A parent far off, itself with the parent 5, which holds nothing; a parent held by script.
        map_row(3, 0, far.0, far.1, (1, 2)),
        map_row(5, 3, far.0, far.1, (0, 0)),
        map_row(6, 2, far.0, far.1, (0, 0)),
    ];
    let index = index_with(vec![PathBuf::from("made-up.rpf")], &[], maps, vec![3, 5, 6]);
    let mut streamer = Streamer::new(Arc::new(index));
    streamer.rewant(&View::at(Vec3::ZERO, 1.0));
    assert_eq!(
        wanted_by(&streamer),
        vec![1, 3, 5],
        "the map, its parent and that one's, less what a script manages"
    );
    // 55 m from the box's side it streams in, and 70 m from it it does not.
    streamer.rewant(&View::at(Vec3::new(55.0, 0.0, 0.0), 1.0));
    assert!(streamer.wanted.contains_key(&1));
    streamer.rewant(&View::at(Vec3::new(70.0, 0.0, 0.0), 1.0));
    assert!(streamer.wanted.is_empty());
}

#[test]
fn a_map_is_wanted_only_while_what_is_enabled_allows_it() {
    let near = ([-10.0; 3], [10.0; 3]);
    let mut managed = map_row(2, 2, near.0, near.1, (0, 0));
    managed.group = 7;
    let mut story = map_row(3, 0, near.0, near.1, (0, 0));
    story.modes = 2;
    let maps = vec![map_row(1, 0, near.0, near.1, (0, 0)), managed, story];
    let index = index_with(vec![PathBuf::from("made-up.rpf")], &[], maps, Vec::new());
    let mut streamer = Streamer::new(Arc::new(index));
    let view = View::at(Vec3::ZERO, 1.0);

    // The base game alone, no group: what streams at rest.
    streamer.stream(&view);
    assert_eq!(wanted_by(&streamer), vec![1]);
    // The same again is no change, and the maps are not worked out again for it.
    assert!(!streamer.set_enabled(Enabled::default()));
    assert!(streamer.streamed_at.is_some());

    // With the group 7 switched on its map streams too, though the camera stands.
    let with_group = Enabled {
        modes: 1,
        groups: vec![7],
    };
    assert!(streamer.set_enabled(with_group));
    streamer.stream(&view);
    assert_eq!(wanted_by(&streamer), vec![1, 2]);

    // The story with the base game, and no group.
    let with_story = Enabled {
        modes: 3,
        groups: Vec::new(),
    };
    assert!(streamer.set_enabled(with_story));
    streamer.stream(&view);
    assert_eq!(wanted_by(&streamer), vec![1, 3]);
}

#[test]
fn a_map_that_reads_with_nothing_to_draw_is_not_wanted_again_and_still_leads_to_its_parents() {
    let near = ([-10.0; 3], [10.0; 3]);
    let far = ([5000.0; 3], [5010.0; 3]);
    // A script-managed map with its group on and the parent 3, which holds nothing until read.
    let mut managed = map_row(2, 2, near.0, near.1, (0, 1));
    managed.group = 7;
    let maps = vec![managed, map_row(3, 0, far.0, far.1, (0, 0))];
    let index = Arc::new(index_with(
        vec![PathBuf::from("made-up.rpf")],
        &[],
        maps,
        vec![3],
    ));
    let mut streamer = Streamer::new(Arc::clone(&index));
    streamer.set_enabled(Enabled {
        modes: 1,
        groups: vec![7],
    });
    let view = View::at(Vec3::ZERO, 1.0);
    streamer.stream(&view);
    assert_eq!(wanted_by(&streamer), vec![2, 3]);
    let empty = StreamMap {
        name: 2,
        entities: Vec::new(),
        prop_lists: Vec::new(),
        ..made_up()
    };
    assert!(!streamer.insert(2, read_map(&index, &empty)));
    assert_eq!(wanted_by(&streamer), vec![3]);
    streamer.rewant(&view);
    assert_eq!(
        wanted_by(&streamer),
        vec![3],
        "worked out again, its parent alone"
    );
    // A parent that holds nothing is still loaded, as the parent of a map that is wanted.
    let parent = StreamMap { name: 3, ..empty };
    assert!(streamer.insert(3, read_map(&index, &parent)));
    assert_eq!(wanted_by(&streamer), vec![3]);
}

#[test]
fn a_script_managed_map_that_reads_empty_is_kept_while_it_is_the_parent_of_a_map_wanted() {
    let near = ([-10.0; 3], [10.0; 3]);
    // Around the camera: the script-managed 2, and 1, whose parent is 2.
    let mut managed = map_row(2, 2, near.0, near.1, (0, 0));
    managed.group = 7;
    let maps = vec![map_row(1, 0, near.0, near.1, (0, 1)), managed];
    let index = Arc::new(index_with(
        vec![PathBuf::from("made-up.rpf")],
        &[],
        maps,
        vec![2],
    ));
    let mut streamer = Streamer::new(Arc::clone(&index));
    streamer.set_enabled(Enabled {
        modes: 1,
        groups: vec![7],
    });
    streamer.stream(&View::at(Vec3::ZERO, 1.0));
    let empty = StreamMap {
        name: 2,
        entities: Vec::new(),
        prop_lists: Vec::new(),
        ..made_up()
    };
    assert!(streamer.insert(2, read_map(&index, &empty)));
    assert_eq!(wanted_by(&streamer), vec![1, 2]);
}

#[test]
fn a_fast_camera_still_wants_the_high_tier() {
    let near = ([-10.0; 3], [10.0; 3]);
    // Content flags 0: the high tier.
    let maps = vec![map_row(1, 0, near.0, near.1, (0, 0))];
    let index = index_with(vec![PathBuf::from("made-up.rpf")], &[], maps, Vec::new());
    let mut streamer = Streamer::new(Arc::new(index));
    let mut view = View::at(Vec3::ZERO, 1.0);
    view.velocity = Vec3::new(300.0, 0.0, 0.0);
    streamer.stream(&view);
    assert_eq!(wanted_by(&streamer), vec![1]);
}

#[test]
fn each_level_s_fade_band_is_read_from_the_visual_settings() {
    let settings = VisualSettings::parse("lod.fadedist.lod 12\nlod.fadedist.slod4 34\n");
    let mut want = [FADE; Level::COUNT];
    want[Level::Lod as usize] = 12.0;
    want[Level::Slod4 as usize] = 34.0;
    assert_eq!(fade_of(&settings), want);
    assert_eq!(fade_of(&VisualSettings::parse("")), [FADE; Level::COUNT]);
}

#[test]
fn a_camera_keeps_its_speed_between_frames_and_is_still_once_it_stops() {
    use std::time::Duration;
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let mut moved = None;
    assert_eq!(
        velocity_at(&mut moved, Vec3::ZERO, Vec3::ZERO, at(0)),
        Vec3::ZERO
    );
    // A frame 100 ms on, 12 m along: 120 m/s.
    let v = velocity_at(&mut moved, Vec3::ZERO, Vec3::new(12.0, 0.0, 0.0), at(100));
    assert!((v.x - 120.0).abs() < 1e-3, "{v}");
    // Passes before the next frame find it where it was, and keep its speed.
    let kept = velocity_at(&mut moved, v, Vec3::new(12.0, 0.0, 0.0), at(600));
    assert_eq!(kept, v);
    // A second with no new place: it has stopped.
    let still = velocity_at(&mut moved, kept, Vec3::new(12.0, 0.0, 0.0), at(1100));
    assert_eq!(still, Vec3::ZERO);
    // Moving off again 20 ms after a pass found it at rest is measured over those 20 ms: 6 m is 300 m/s.
    let off = velocity_at(&mut moved, still, Vec3::new(18.0, 0.0, 0.0), at(1120));
    assert!((off.x - 300.0).abs() < 1e-2, "{off}");
}

#[test]
fn a_world_thread_that_dies_is_started_again_until_it_has_died_too_often_and_the_panel_is_told() {
    use std::time::Duration;
    let index = Arc::new(made_up_index());
    let mut viewer = super::super::Viewer {
        world: Some(World::new(
            Arc::clone(&index),
            Vec3::ZERO,
            1.0,
            HashSet::new(),
        )),
        index: Some(index),
        ..Default::default()
    };
    for death in 1..=super::super::MOST_RESTARTS + 1 {
        let world = viewer.world.as_ref().expect("a world");
        world.end();
        let until = Instant::now() + Duration::from_secs(20);
        while !world.died() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        viewer.selected(&mut Vec::new());
        let restarts = death.min(super::super::MOST_RESTARTS);
        assert_eq!(viewer.world_restarts, restarts, "after death {death}");
        let given_up = death > super::super::MOST_RESTARTS;
        assert_eq!(viewer.world_given_up(), given_up, "after death {death}");
        assert_eq!(viewer.world.as_ref().is_some_and(World::died), given_up);
    }
}

#[test]
fn a_world_thread_that_dies_on_a_map_it_just_took_in_is_started_again_without_it_and_never_given_up(
) {
    use std::time::Duration;
    let index = Arc::new(made_up_index());
    let mut viewer = super::super::Viewer {
        world: Some(World::new(
            Arc::clone(&index),
            Vec3::ZERO,
            1.0,
            HashSet::new(),
        )),
        index: Some(index),
        ..Default::default()
    };
    // More deaths than give the thread up, each on a map of its own.
    let deaths = super::super::MOST_RESTARTS + 2;
    for death in 1..=deaths {
        let world = viewer.world.as_ref().expect("a world");
        world.took(0x100 + death);
        world.end();
        let until = Instant::now() + Duration::from_secs(20);
        while !world.died() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        viewer.selected(&mut Vec::new());
        assert_eq!(viewer.world_restarts, death, "after death {death}");
        assert!(
            viewer.skipped.contains(&(0x100 + death)),
            "after death {death}"
        );
        assert!(!viewer.world_given_up(), "after death {death}");
        assert!(!viewer.world.as_ref().is_some_and(World::died));
    }
    assert_eq!(viewer.skipped.len() as u32, deaths);
}

#[test]
fn what_was_told_the_world_s_thread_is_unselected_until_a_pass_has_taken_it() {
    use std::time::Duration;
    let world = World::new(Arc::new(made_up_index()), Vec3::ZERO, 1.0, HashSet::new());
    let settled = |world: &World| {
        let until = Instant::now() + Duration::from_secs(20);
        while world.unselected() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        !world.unselected()
    };
    assert!(
        settled(&world),
        "the first pass takes what it was made with"
    );
    // A pass that selects cannot publish while the selection is held, so it is not done meanwhile.
    let latest = world
        .shared
        .latest
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    world.look(Vec3::new(500.0, 0.0, 0.0), 1.0);
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        world.unselected(),
        "told, and the pass that took it has not selected"
    );
    drop(latest);
    assert!(settled(&world), "a pass took it and selected after it");
}
