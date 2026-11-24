use std::collections::HashMap;

use glam::Vec3;

use super::*;

fn entity(at: [f32; 3], lod_dist: f32, child_lod_dist: f32, parent: Option<(u64, u32)>) -> Node {
    Node {
        position: Vec3::from(at),
        extent: Extent::Point,
        lod_dist,
        child_lod_dist,
        parent: parent.map(|(map, node)| ParentRef { map, node }),
        instance: None,
    }
}

fn list(min: [f32; 3], max: [f32; 3], lod_dist: f32, parent: Option<(u64, u32)>) -> Node {
    let (min, max) = (Vec3::from(min), Vec3::from(max));
    Node {
        position: (min + max) / 2.0,
        extent: Extent::Box(min, max),
        lod_dist,
        child_lod_dist: 0.0,
        parent: parent.map(|(map, node)| ParentRef { map, node }),
        instance: None,
    }
}

/// Every node ready but those named, which are loading.
fn loading(nodes: &[NodeId]) -> impl Fn(NodeId) -> NodeState + '_ {
    move |id| match nodes.contains(&id) {
        true => NodeState::Loading,
        false => NodeState::Ready,
    }
}

fn all_ready(_: NodeId) -> NodeState {
    NodeState::Ready
}

/// A parent at the origin drawn to 1,000 m whose two children, 300 m apart, draw within 100 m of it.
fn parent_and_two_children() -> Forest {
    let mut forest = Forest::new();
    forest.insert(1, vec![entity([0.0; 3], 1000.0, 100.0, None)]);
    forest.insert(
        2,
        vec![
            entity([150.0, 0.0, 0.0], 40.0, 0.0, Some((1, 0))),
            entity([-150.0, 0.0, 0.0], 40.0, 0.0, Some((1, 0))),
        ],
    );
    forest
}

#[test]
fn the_children_of_one_parent_swap_together_by_the_distance_to_the_parent() {
    let mut forest = parent_and_two_children();
    // Beside the first child and 240 m from the parent: neither child draws, the parent does.
    let far = forest.select(Vec3::new(240.0, 0.0, 0.0), 1.0, &all_ready);
    assert_eq!(far.drawn, vec![(1, 0)]);
    // 90 m from the parent and 240 m from the second child: both children draw, the parent does not.
    let mut near = forest
        .select(Vec3::new(90.0, 0.0, 0.0), 1.0, &all_ready)
        .drawn;
    near.sort_unstable();
    assert_eq!(near, vec![(2, 0), (2, 1)]);
}

#[test]
fn a_parent_draws_until_every_child_is_ready_and_the_child_is_waited_for() {
    let mut forest = parent_and_two_children();
    let waiting = [(2, 1)];
    let held = forest.select(Vec3::new(90.0, 0.0, 0.0), 1.0, &loading(&waiting));
    assert_eq!(held.drawn, vec![(1, 0)]);
    assert_eq!(held.wanted, vec![(2, 1)]);
    // Out of the children's distance nothing of theirs is waited for.
    let far = forest.select(Vec3::new(500.0, 0.0, 0.0), 1.0, &loading(&waiting));
    assert_eq!((far.drawn, far.wanted), (vec![(1, 0)], vec![]));
}

#[test]
fn a_parent_draws_while_a_child_map_of_its_map_is_not_loaded() {
    let mut forest = parent_and_two_children();
    forest.set_complete(1, false);
    let held = forest.select(Vec3::new(90.0, 0.0, 0.0), 1.0, &all_ready);
    assert_eq!(held.drawn, vec![(1, 0)]);
    forest.set_complete(1, true);
    assert_eq!(
        forest
            .select(Vec3::new(90.0, 0.0, 0.0), 1.0, &all_ready)
            .drawn
            .len(),
        2
    );
}

#[test]
fn a_child_takes_the_distance_its_parent_states_for_its_children() {
    // The child states 40 m and its parent 100 m for its children: at 90 m from the parent it draws.
    let mut forest = parent_and_two_children();
    assert_eq!(
        forest
            .select(Vec3::new(0.0, 90.0, 0.0), 1.0, &all_ready)
            .drawn
            .len(),
        2
    );
    // A parent that states none takes the farthest of its children's.
    let mut unstated = Forest::new();
    unstated.insert(1, vec![entity([0.0; 3], 1000.0, 0.0, None)]);
    unstated.insert(
        2,
        vec![
            entity([1.0, 0.0, 0.0], 40.0, 0.0, Some((1, 0))),
            entity([2.0, 0.0, 0.0], 70.0, 0.0, Some((1, 0))),
        ],
    );
    let at = |d: f32| Vec3::new(0.0, d, 0.0);
    assert_eq!(
        unstated.select(at(80.0), 1.0, &all_ready).drawn,
        vec![(1, 0)]
    );
    // Within 70 m the parent gives way, and the child of 40 m is itself out of its distance.
    assert_eq!(
        unstated.select(at(60.0), 1.0, &all_ready).drawn,
        vec![(2, 1)]
    );
    assert_eq!(unstated.select(at(30.0), 1.0, &all_ready).drawn.len(), 2);
}

#[test]
fn a_node_with_no_parent_draws_by_its_own_place_and_the_scale_stretches_every_distance() {
    let mut forest = Forest::new();
    forest.insert(7, vec![entity([10.0, 0.0, 0.0], 50.0, 0.0, None)]);
    let at = |x: f32| Vec3::new(x, 0.0, 0.0);
    assert_eq!(forest.select(at(59.0), 1.0, &all_ready).drawn, vec![(7, 0)]);
    assert!(forest.select(at(61.0), 1.0, &all_ready).drawn.is_empty());
    assert_eq!(
        forest.select(at(100.0), 2.0, &all_ready).drawn,
        vec![(7, 0)]
    );
    assert!(forest.select(at(59.0), 0.5, &all_ready).drawn.is_empty());
}

#[test]
fn a_grandparent_draws_until_the_whole_of_what_replaces_it_is_ready() {
    let mut forest = Forest::new();
    forest.insert(1, vec![entity([0.0; 3], 5000.0, 1000.0, None)]);
    forest.insert(
        2,
        vec![
            entity([0.0; 3], 0.0, 100.0, Some((1, 0))),
            entity([400.0, 0.0, 0.0], 0.0, 100.0, Some((1, 0))),
        ],
    );
    forest.insert(3, vec![entity([0.0; 3], 0.0, 0.0, Some((2, 0)))]);
    let eye = Vec3::new(0.0, 50.0, 0.0);
    // The first parent gives way to its child, the second draws itself: three levels at once.
    let mut drawn = forest.select(eye, 1.0, &all_ready).drawn;
    drawn.sort_unstable();
    assert_eq!(drawn, vec![(2, 1), (3, 0)]);
    // With the grandchild loading its parent draws, which is ready, so the grandparent still gives way.
    let waiting = [(3, 0)];
    let mut drawn = forest.select(eye, 1.0, &loading(&waiting)).drawn;
    drawn.sort_unstable();
    assert_eq!(drawn, vec![(2, 0), (2, 1)]);
    // With that parent loading as well nothing can take the grandparent's place.
    let waiting = [(3, 0), (2, 0)];
    let held = forest.select(eye, 1.0, &loading(&waiting));
    assert_eq!(held.drawn, vec![(1, 0)]);
    assert!(held.wanted.contains(&(2, 0)) && held.wanted.contains(&(3, 0)));
}

#[test]
fn a_child_whose_parent_map_is_not_loaded_draws_alone_until_it_is() {
    let mut forest = Forest::new();
    forest.insert(2, vec![entity([0.0; 3], 40.0, 0.0, Some((1, 0)))]);
    let eye = Vec3::new(0.0, 30.0, 0.0);
    assert_eq!(forest.select(eye, 1.0, &all_ready).drawn, vec![(2, 0)]);
    forest.insert(1, vec![entity([0.0, 500.0, 0.0], 2000.0, 100.0, None)]);
    // The parent stands 470 m away: its child, now judged by that, does not draw.
    assert_eq!(forest.select(eye, 1.0, &all_ready).drawn, vec![(1, 0)]);
    forest.remove(1);
    assert_eq!(forest.select(eye, 1.0, &all_ready).drawn, vec![(2, 0)]);
    assert_eq!(forest.len(), (1, 1));
}

#[test]
fn a_list_draws_by_its_box_less_each_instance_whose_entity_draws() {
    let mut forest = Forest::new();
    forest.insert(1, vec![list([0.0; 3], [100.0, 100.0, 10.0], 300.0, None)]);
    let mut stand_in = entity([10.0, 10.0, 0.0], 50.0, 0.0, Some((1, 0)));
    stand_in.instance = Some(4);
    let mut other = entity([90.0, 90.0, 0.0], 50.0, 0.0, Some((1, 0)));
    other.instance = Some(9);
    forest.insert(2, vec![stand_in, other]);
    // Inside the box, beside the first entity: it draws and its instance is left out.
    let near = forest.select(Vec3::new(12.0, 12.0, 0.0), 1.0, &all_ready);
    assert_eq!(near.drawn, vec![(2, 0)]);
    assert_eq!(near.lists.len(), 1);
    assert_eq!(&near.skipped[near.lists[0].skipped.clone()], &[4]);
    // 250 m off the box's side the list draws whole; 350 m off it does not.
    let off = forest.select(Vec3::new(350.0, 50.0, 0.0), 1.0, &all_ready);
    assert_eq!(
        (off.drawn.len(), off.lists.len(), off.skipped.len()),
        (0, 1, 0)
    );
    let gone = forest.select(Vec3::new(450.0, 50.0, 0.0), 1.0, &all_ready);
    assert!(gone.lists.is_empty());
    // An entity still loading leaves its instance drawn and is waited for.
    let waiting = [(2, 0)];
    let held = forest.select(Vec3::new(12.0, 12.0, 0.0), 1.0, &loading(&waiting));
    assert_eq!((held.drawn.len(), held.skipped.len()), (0, 0));
    assert_eq!(held.wanted, vec![(2, 0)]);
}

#[test]
fn a_list_gives_way_to_the_lists_under_it() {
    let mut forest = Forest::new();
    let mut far = list([0.0; 3], [100.0, 100.0, 10.0], 2000.0, None);
    far.child_lod_dist = 200.0;
    forest.insert(1, vec![far]);
    forest.insert(
        2,
        vec![list([0.0; 3], [50.0, 50.0, 10.0], 0.0, Some((1, 0)))],
    );
    let ids = |s: Selection| s.lists.into_iter().map(|l| l.node).collect::<Vec<_>>();
    assert_eq!(
        ids(forest.select(Vec3::new(150.0, 50.0, 0.0), 1.0, &all_ready)),
        vec![(2, 0)]
    );
    assert_eq!(
        ids(forest.select(Vec3::new(900.0, 50.0, 0.0), 1.0, &all_ready)),
        vec![(1, 0)]
    );
}

#[test]
fn a_node_with_no_model_draws_nothing_and_holds_nothing_back() {
    let mut forest = parent_and_two_children();
    let absent: HashMap<NodeId, NodeState> = HashMap::from([((2, 1), NodeState::Absent)]);
    let state = |id: NodeId| *absent.get(&id).unwrap_or(&NodeState::Ready);
    let near = forest.select(Vec3::new(90.0, 0.0, 0.0), 1.0, &state);
    assert_eq!((near.drawn, near.wanted), (vec![(2, 0)], vec![]));
}

#[test]
fn a_map_streams_in_within_fifty_metres_of_the_box_it_streams_by() {
    let streamed = MapBox {
        streaming: (Vec3::splat(-100.0), Vec3::splat(100.0)),
        entities: (Vec3::splat(-10.0), Vec3::splat(10.0)),
    };
    assert!(streamed.wants(Vec3::new(149.0, 0.0, 0.0)));
    assert!(!streamed.wants(Vec3::new(151.0, 0.0, 0.0)));
    assert!(!streamed.wants(Vec3::new(0.0, 0.0, 151.0)));
    // A streaming box that does not hold the entities' box is not the one streamed by.
    let narrow = MapBox {
        streaming: (Vec3::splat(-1.0), Vec3::splat(1.0)),
        entities: (Vec3::splat(-200.0), Vec3::splat(200.0)),
    };
    assert!(narrow.wants(Vec3::new(240.0, 0.0, 0.0)));
    assert!(!narrow.wants(Vec3::new(260.0, 0.0, 0.0)));
}
