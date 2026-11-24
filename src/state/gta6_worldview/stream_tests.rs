use std::path::PathBuf;

use or_io::gta6::map::stream::{StreamEntity, StreamPropList};
use or_io::gta6::map::Aabb;

use super::super::index::tests_support::{index_of, index_with};
use super::super::index::MapRow;
use super::*;

const TREE: u64 = 100;
const LAMP: u64 = 200;
const ROOM: u64 = 300;

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
    let plain = [
        [1.0, 0.0, 0.0, 20.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
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
        prop_lists: vec![StreamPropList {
            archetype_word: LAMP,
            archetype_name: 2,
            lod_dist: 0.0,
            child_lod_dist: 0.0,
            lod_level: 1,
            parent: None,
            batch_aabb: boxed([0.0; 3], [30.0, 10.0, 10.0]),
            instances: vec![turned, plain],
        }],
        grass_lists: 0,
    }
}

/// An index whose tree draws to 120 m and lamp to 300 m, each with a model, and a room with none.
fn made_up_index() -> WorldIndex {
    index_of(
        vec![PathBuf::from("made-up.rpf")],
        &[
            (TREE, 120.0, Some(0x1000)),
            (LAMP, 300.0, Some(0x2000)),
            (ROOM, 80.0, None),
        ],
    )
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
    let point = Vec3::new(1.0, 2.0, 3.0);
    for (stated, rows) in map.prop_lists[0]
        .instances
        .iter()
        .zip(&read.loaded.lists[0])
    {
        let by_the_reader = or_io::gta6::map::stream::place(stated, point.to_array());
        assert_eq!(place(rows, point).to_array(), by_the_reader);
    }
    // The turned instance puts the model's x axis along the world's y.
    assert_eq!(
        place(&read.loaded.lists[0][0], Vec3::X),
        Vec3::new(5.0, 7.0, 7.0)
    );
    // An entity's rows place its own origin at its position.
    assert_eq!(
        place(&read.loaded.entities[0], Vec3::ZERO),
        Vec3::new(1.0, 2.0, 3.0)
    );
    assert_eq!(stretch(&read.loaded.lists[0][0]), 1.0);
}

#[test]
fn a_selection_draws_each_entity_and_each_instance_with_its_model_and_waits_for_the_rest() {
    let index = Arc::new(made_up_index());
    let mut streamer = Streamer::new(Arc::clone(&index));
    let mut map = made_up();
    map.entities[1].parent = None;
    streamer.wanted.insert(55, 0.0);
    assert!(streamer.insert(55, read_map(&index, &map)));

    let eye = Vec3::new(0.0, 0.0, 0.0);
    // Nothing is on the device yet: each model is waited for once, and nothing draws.
    let none = streamer.select(eye, 1.0, &|_| Some(false));
    assert!(none.instances.is_empty() && none.chunks.is_empty());
    let mut models: Vec<u64> = none.wanted.iter().map(|(_, m)| m.offset).collect();
    models.sort_unstable();
    assert_eq!(models, vec![0x1000, 0x2000]);

    // With every model there: two trees in one chunk, and both lamps of the list in another.
    let all = streamer.select(eye, 1.0, &|_| Some(true));
    assert!(all.wanted.is_empty());
    let of = |offset: u64| {
        (all.instances.iter())
            .filter(|i| all.models[i.model as usize].offset == offset)
            .count()
    };
    assert_eq!((of(0x1000), of(0x2000)), (2, 2));
    assert_eq!(all.streaming.drawn, 4);
    assert_eq!(all.chunks.len(), 2);
    for chunk in &all.chunks {
        // A chunk's sphere holds the sphere of each instance in it.
        for i in &all.instances[chunk.from as usize..chunk.to as usize] {
            assert!(chunk.centre.distance(i.centre) + i.radius <= chunk.radius + 1e-3);
        }
    }
    // A model that will never come draws nothing and is not waited for.
    let never = streamer.select(eye, 1.0, &|_| None);
    assert!(never.wanted.is_empty() && never.instances.is_empty());
    // A map let go before its body arrived is not kept.
    assert!(!streamer.insert(56, read_map(&index, &map)));
    assert!(!streamer.loaded.contains_key(&56));
}

fn map_row(name: u32, held: u32, from: [f32; 3], to: [f32; 3], parents: (u32, u32)) -> MapRow {
    let mut boxes = [0.0; 12];
    for (slot, corner) in boxes.chunks_mut(3).zip([from, to, from, to]) {
        slot.copy_from_slice(&corner);
    }
    MapRow {
        archive: 0,
        name,
        block: 0,
        flags: 0,
        content_flags: 0,
        held,
        parents_from: parents.0,
        parents_len: parents.1,
        offset: 0,
        boxes,
    }
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
    streamer.rewant(Vec3::ZERO);
    let mut wanted: Vec<u32> = streamer.wanted.keys().copied().collect();
    wanted.sort_unstable();
    assert_eq!(
        wanted,
        vec![1, 3, 5],
        "the map, its parent and that one's, less what a script manages"
    );
    // 55 m from the box's side it streams in, and 70 m from it it does not.
    streamer.rewant(Vec3::new(55.0, 0.0, 0.0));
    assert!(streamer.wanted.contains_key(&1));
    streamer.rewant(Vec3::new(70.0, 0.0, 0.0));
    assert!(streamer.wanted.is_empty());
}
