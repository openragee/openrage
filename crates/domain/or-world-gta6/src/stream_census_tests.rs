use glam::Vec3;
use or_world::streaming::{Category, Extent, Kind, Level, Node};

use super::kind;

fn node(kind: Kind) -> Node {
    Node {
        position: Vec3::ZERO,
        extent: Extent::Point,
        lod_dist: 100.0,
        child_lod_dist: 0.0,
        parent: None,
        instance: None,
        level: Level::Hd,
        kind,
        category: Category::Other,
        radius: 0.0,
        num_children: 0,
        batch: false,
    }
}

#[test]
fn a_map_of_an_entity_a_prop_list_a_grass_list_and_a_far_list_counts_four_kinds() {
    // One entity, then one prop list, one plain grass list and one list of far stand-ins.
    let map = [Kind::Entity, Kind::Props, Kind::Grass, Kind::Entity];
    let kinds: Vec<usize> = (map.iter().enumerate())
        .map(|(at, &k)| kind(&node(k), at, 1))
        .collect();
    assert_eq!(kinds, [0, 1, 2, 3]);
}

#[test]
fn a_far_list_that_comes_straight_after_the_entities_is_a_far_list() {
    // One entity, then a list of far stand-ins, with no prop list and no plain grass list between.
    let kinds = [0, 1].map(|at| kind(&node(Kind::Entity), at, 1));
    assert_eq!(kinds, [0, 3]);
}
