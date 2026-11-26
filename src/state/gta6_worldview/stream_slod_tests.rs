use std::path::PathBuf;

use or_io::gta6::map::stream::GrassInstance;
use or_io::gta6::map::Aabb;

use super::super::super::index::tests_support::index_of;
use super::*;

const OAK: u64 = 100;
const PALM: u64 = 200;
/// A word no archetype of the index bears.
const LOST: u64 = 999;

/// An index whose oak has a model and whose palm has none, with the far stand-in ids 1 to 3 solved
/// to the oak, the palm and an archetype it does not hold.
fn index() -> WorldIndex {
    let mut index = index_of(
        vec![PathBuf::from("a.rpf")],
        &[(OAK, 500.0, Some(0x4000)), (PALM, 500.0, None)],
    );
    index.slod = vec![0, OAK, PALM, LOST];
    index
}

fn stand_in(position: [u16; 3], guid: u16) -> GrassInstance {
    GrassInstance {
        position,
        normal: [128, 128],
        colour: [10, 20, 30],
        scale: 255,
        scale_z: 128,
        rotation: 36,
        guid,
    }
}

fn far() -> StreamGrassList {
    StreamGrassList {
        archetype_word: 1,
        archetype_name: 1,
        position: [0.0; 3],
        lod_dist: 1000.0,
        lod_level: 5,
        num_children: 0,
        batch_aabb: Aabb {
            min: [10.0, 20.0, 30.0],
            max: [110.0, 220.0, 80.0],
        },
        scale_range: [0.5, 2.0],
        lod_fade_start_dist: 899.0,
        instance_lod_dist: 900.0,
        orient_to_terrain: 1.0,
        slod: true,
        archetypes: vec![(1, OAK), (2, PALM)],
        instances: vec![
            stand_in([0, 0, 0], 1),
            stand_in([65535, 0, 7], 2),
            stand_in([5, 6, 7], 1),
        ],
    }
}

#[test]
fn each_id_names_its_archetypes_sphere_and_one_left_unsolved_names_nothing() {
    let index = index();
    let species = species(&index);
    let oak = Species {
        archetype: OAK as u32,
        centre: [0.0; 3],
        radius: 1.0,
    };
    let palm = Species {
        archetype: PALM as u32,
        ..oak
    };
    assert_eq!(
        species[..],
        [Species::default(), oak, palm, Species::default()]
    );
    assert_eq!(
        model_of(&index, 1),
        Some(ModelRef {
            archive: 0,
            offset: 0x4000,
            entry: None
        })
    );
    for id in [0, 2, 3, 4, 70_000] {
        assert_eq!(model_of(&index, id), None, "{id}");
    }
}

#[test]
fn a_card_list_keeps_the_box_the_scales_and_each_record_as_stored() {
    let list = far();
    let cards = CardList::of(&list, &index());
    assert_eq!(cards.min, [10.0, 20.0, 30.0]);
    assert_eq!(cards.size, [100.0, 200.0, 50.0]);
    assert_eq!(cards.centre, [60.0, 120.0, 55.0]);
    // The box's half diagonal, and the corner of a radius-1 model's card at the most scale past it.
    let half = Vec3::new(100.0, 200.0, 50.0).length() / 2.0;
    let corner = 2.0 * std::f32::consts::SQRT_2;
    assert!(
        (cards.radius - (half + corner)).abs() < 1e-4,
        "{}",
        cards.radius
    );
    assert_eq!(cards.scale_range, [0.5, 2.0]);
    assert_eq!((cards.distance, cards.fade), (900.0, 1.0));
    let stored: Vec<[u8; 16]> = list.instances.iter().map(|i| i.record()).collect();
    assert_eq!(cards.records[..], stored[..]);
    assert_eq!(
        u16::from_le_bytes([cards.records[1][14], cards.records[1][15]]),
        2
    );
    // Each species once, for the atlas to wait for.
    assert_eq!(cards.ids[..], [1, 2]);
    // Seen through a view, it scales its distance and takes the list's fade, sharing the records.
    let seen = cards.seen(1.5, 0.25, &[]);
    assert_eq!((seen.distance, seen.fade), (1350.0, 0.25));
    assert!(Arc::ptr_eq(&seen.records, &cards.records));
    assert_eq!((seen.centre, seen.radius), (cards.centre, cards.radius));
}

#[test]
fn an_instance_a_drawn_prop_stands_in_for_is_hidden_and_the_rest_are_kept() {
    let cards = CardList::of(&far(), &index());
    // The second instance's prop is drawn; an instance past the list's end changes nothing.
    let seen = cards.seen(1.0, 1.0, &[1, 9]);
    let id =
        |records: &[[u8; 16]], at: usize| u16::from_le_bytes([records[at][14], records[at][15]]);
    assert_eq!(id(&seen.records, 1), HIDDEN);
    assert!(usize::from(HIDDEN) >= species(&index()).len());
    assert_eq!(seen.records[1][..14], cards.records[1][..14]);
    for at in [0, 2] {
        assert_eq!(seen.records[at], cards.records[at]);
    }
    // The list itself keeps its own records.
    assert_eq!(id(&cards.records, 1), 2);
}

#[test]
fn a_card_list_of_ids_no_list_settles_holds_its_box_alone() {
    let mut list = far();
    for instance in &mut list.instances {
        instance.guid = 3;
    }
    let cards = CardList::of(&list, &index());
    let half = Vec3::new(100.0, 200.0, 50.0).length() / 2.0;
    assert!((cards.radius - half).abs() < 1e-4, "{}", cards.radius);
}

#[test]
fn a_far_stand_in_yields_to_a_model_drawn_where_it_stands_and_to_nothing_else() {
    let cards = CardList::of(&far(), &index());
    // The first and third instances stand at the box's least corner, the second at its far east side
    let first = Vec3::new(10.0, 20.0, 30.0);
    let second = Vec3::new(110.0, 20.0, 30.0 + 7.0 / 65535.0 * 50.0);
    assert_eq!(cards.handed_off(&Standing::of([first])), [0, 2]);
    // A model a few centimetres off, as a box quantises a place, is still its tree
    let off = Vec3::new(0.05, -0.04, 0.3);
    assert_eq!(
        cards.handed_off(&Standing::of([first + off, second - off])),
        [0, 1, 2]
    );
    // Nothing drawn, or something a metre away, keeps every card
    assert!(cards.handed_off(&Standing::of([])).is_empty());
    assert!(cards
        .handed_off(&Standing::of([first + Vec3::X, second + Vec3::Y * 2.0]))
        .is_empty());
    // A place across a cell's edge is found from the cell beside it
    let edge = Vec3::new(16.0, 24.0, 0.0);
    assert!(Standing::of([edge - Vec3::new(0.01, 0.0, 0.0)])
        .stands_at(edge + Vec3::new(0.05, 0.0, 0.0)));
}

#[test]
fn the_cells_bits_let_go_of_no_place_that_stands() {
    // Places on a ragged grid across negative and positive cells, some on a cell's edge.
    let places: Vec<Vec3> = (0..40)
        .map(|k| {
            Vec3::new(
                k as f32 * 7.9 - 150.0,
                (k * k % 23) as f32 * 8.0 - 90.0,
                1.0,
            )
        })
        .collect();
    let standing = Standing::of(places.iter().copied());
    assert!(!standing.taken.is_empty());
    let looked_up = |at: Vec3| {
        (places.iter()).any(|p| {
            (p.x - at.x).hypot(p.y - at.y) <= SAME_TREE && (p.z - at.z).abs() <= SAME_TREE_HEIGHT
        })
    };
    for p in &places {
        for off in [
            Vec3::ZERO,
            Vec3::new(0.1, -0.1, 0.5),
            Vec3::new(0.3, 0.0, 0.0),
            Vec3::new(-40.0, 3.0, 0.0),
        ] {
            assert_eq!(
                standing.stands_at(*p + off),
                looked_up(*p + off),
                "at {}",
                *p + off
            );
        }
    }
    // Outside the box every way.
    assert!(!standing.stands_at(Vec3::new(-1000.0, 0.0, 1.0)));
    assert!(!standing.stands_at(Vec3::new(0.0, 5000.0, 1.0)));
}
