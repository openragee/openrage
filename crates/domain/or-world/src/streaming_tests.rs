use std::collections::HashMap;

use glam::Vec3;

use super::*;

/// An entity: of the high level where it has a parent, a level-of-detail model where it has none.
fn entity(at: [f32; 3], lod_dist: f32, child_lod_dist: f32, parent: Option<(u64, u32)>) -> Node {
    Node {
        position: Vec3::from(at),
        extent: Extent::Point,
        lod_dist,
        child_lod_dist,
        parent: parent.map(|(map, node)| ParentRef { map, node }),
        instance: None,
        level: match parent {
            Some(_) => Level::Hd,
            None => Level::Lod,
        },
        kind: Kind::Entity,
        category: Category::Other,
        radius: 0.0,
        num_children: 0,
        batch: false,
    }
}

/// `node` stating `count` children.
fn with_children(node: Node, count: u32) -> Node {
    Node {
        num_children: count,
        ..node
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
        level: Level::Lod,
        kind: Kind::Props,
        category: Category::Other,
        radius: (max - min).length() / 2.0,
        num_children: 0,
        batch: false,
    }
}

/// `node` of the level `level`.
fn leveled(mut node: Node, level: Level) -> Node {
    node.level = level;
    node
}

/// A forest of one node at the origin of the level `level` that draws to 100 m.
fn lone(level: Level) -> Forest {
    let mut forest = Forest::new();
    forest.insert(1, vec![leveled(entity([0.0; 3], 100.0, 0.0, None), level)]);
    forest
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

/// A camera at rest `x` metres along the first axis.
fn along(x: f32) -> View {
    View::at(Vec3::new(x, 0.0, 0.0), 1.0)
}

/// The entities a selection draws, in order.
fn ids(selection: &Selection) -> Vec<NodeId> {
    let mut ids: Vec<NodeId> = selection.drawn.iter().map(|d| d.node).collect();
    ids.sort_unstable();
    ids
}

/// How opaque a selection draws the entity or the list `id`: nought where it does not.
fn fade(selection: &Selection, id: NodeId) -> f32 {
    let entity = selection.drawn.iter().find(|d| d.node == id);
    let list = selection.lists.iter().find(|l| l.node == id);
    entity
        .map(|d| d.fade)
        .or(list.map(|l| l.fade))
        .unwrap_or(0.0)
}

/// Within a step of opacity, which goes by 255ths.
fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0 / 255.0
}

/// A parent at the origin drawn to 1,000 m whose two children, 300 m apart, draw within 100 m of it.
fn parent_and_two_children() -> Forest {
    let mut forest = Forest::new();
    forest.insert(
        1,
        vec![with_children(entity([0.0; 3], 1000.0, 100.0, None), 2)],
    );
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
    let far = forest.select(&along(240.0), &all_ready);
    assert_eq!(ids(&far), vec![(1, 0)]);
    // 70 m from the parent and 220 m from the second child: both children draw, the parent does not.
    let near = forest.select(&along(70.0), &all_ready);
    assert_eq!(ids(&near), vec![(2, 0), (2, 1)]);
}

#[test]
fn a_parent_fades_out_as_its_children_fade_in_over_their_band() {
    let mut forest = parent_and_two_children();
    // What draws at each distance from the parent: the parent, then each child; together whole.
    for (away, parent, child) in [
        (130.0, 1.0, 0.0),
        (110.0, 0.5, 0.5),
        (100.0, 0.0, 1.0),
        (80.0, 0.0, 1.0),
    ] {
        let drawn = forest.select(&along(away), &all_ready);
        assert!(close(fade(&drawn, (1, 0)), parent), "the parent at {away}");
        assert!(close(fade(&drawn, (2, 0)), child), "a child at {away}");
        assert!(close(fade(&drawn, (2, 1)), child), "its sibling at {away}");
        // What has faded out is not listed.
        assert!(drawn.drawn.iter().all(|d| d.fade > 0.0));
        assert!(!drawn.fading);
    }
}

#[test]
fn a_parent_is_held_whole_while_a_child_that_draws_is_not_ready_and_the_child_is_waited_for() {
    let mut forest = parent_and_two_children();
    let waiting = [(2, 1)];
    // The ready child draws beside its parent.
    let held = forest.select(&along(90.0), &loading(&waiting));
    assert_eq!(ids(&held), vec![(1, 0), (2, 0)]);
    assert!(close(fade(&held, (1, 0)), 1.0));
    assert_eq!(held.wanted, vec![(2, 1)]);
    // In the band before their distance the children are waited for already.
    let band = forest.select(&along(115.0), &loading(&waiting));
    assert_eq!(band.wanted, vec![(2, 1)]);
    // Out of it nothing of theirs is waited for.
    let far = forest.select(&along(500.0), &loading(&waiting));
    assert_eq!((ids(&far), far.wanted), (vec![(1, 0)], vec![]));
}

#[test]
fn a_child_that_holds_its_parent_is_waited_for_though_its_own_children_draw_in_its_place() {
    let mut forest = Forest::new();
    forest.insert(
        1,
        vec![leveled(
            with_children(entity([0.0; 3], 2000.0, 0.0, None), 1),
            Level::Slod2,
        )],
    );
    let child = with_children(entity([100.0, 0.0, 0.0], 1000.0, 0.0, Some((1, 0))), 1);
    forest.insert(2, vec![leveled(child, Level::Slod1)]);
    let grandchild = entity([100.0, 0.0, 0.0], 450.0, 0.0, Some((2, 0)));
    forest.insert(3, vec![leveled(grandchild, Level::Lod)]);
    // The child draws nothing beside its whole child, yet holds its parent until it is on the device.
    let held = forest.select(&along(100.0), &loading(&[(2, 0)]));
    assert_eq!(
        (ids(&held), held.wanted),
        (vec![(1, 0), (3, 0)], vec![(2, 0)])
    );
    let ready = forest.select(&along(100.0), &all_ready);
    assert_eq!(ids(&ready), vec![(3, 0)]);
}

#[test]
fn nothing_out_of_range_is_asked_for_however_fast_the_camera_closes_on_it() {
    let mut forest = parent_and_two_children();
    let waiting = [(1, 0), (2, 0)];
    // 1,021 m from the parent, a metre past its band, closing at 100 m a second: nothing yet.
    let mut view = along(1021.0);
    view.velocity = Vec3::new(-100.0, 0.0, 0.0);
    assert!(forest.select(&view, &loading(&waiting)).wanted.is_empty());
    // Inside the band it is asked for on the selection it is first in range at.
    view.eye.x = 1019.0;
    assert_eq!(
        forest.select(&view, &loading(&waiting)).wanted,
        vec![(1, 0)]
    );
}

#[test]
fn a_parent_that_fades_out_for_its_children_yields_and_they_do_not() {
    let mut forest = parent_and_two_children();
    let yields = |selection: &Selection, id: NodeId| {
        (selection.drawn.iter().find(|d| d.node == id)).map(|d| d.yields)
    };
    let band = forest.select(&along(110.0), &all_ready);
    assert_eq!(yields(&band, (1, 0)), Some(true));
    assert_eq!(yields(&band, (2, 0)), Some(false));
    // Held whole for a child on its way, it gives way to nothing.
    let held = forest.select(&along(110.0), &loading(&[(2, 1)]));
    assert_eq!(yields(&held, (1, 0)), Some(false));
}

#[test]
fn a_parent_draws_beside_its_children_until_as_many_as_it_states_are_loaded() {
    let mut forest = parent_and_two_children();
    // It states a third child, whose map is not loaded.
    forest.insert(
        1,
        vec![with_children(entity([0.0; 3], 1000.0, 100.0, None), 3)],
    );
    let short = forest.select(&along(70.0), &all_ready);
    assert_eq!(ids(&short), vec![(1, 0), (2, 0), (2, 1)]);
    assert!(close(fade(&short, (1, 0)), 1.0));
    forest.insert(3, vec![entity([0.0; 3], 40.0, 0.0, Some((1, 0)))]);
    let swapped = forest.select(&along(70.0), &all_ready);
    assert_eq!(ids(&swapped), vec![(2, 0), (2, 1), (3, 0)]);
    // What a parent states is raised to what is loaded: one that states fewer, or none, gives way.
    for stated in [2, 0] {
        forest.insert(
            1,
            vec![with_children(entity([0.0; 3], 1000.0, 100.0, None), stated)],
        );
        let raised = forest.select(&along(70.0), &all_ready);
        assert_eq!(
            ids(&raised),
            vec![(2, 0), (2, 1), (3, 0)],
            "stating {stated}"
        );
    }
}

#[test]
fn a_child_takes_the_distance_its_parent_states_for_its_children() {
    // The child states 40 m and its parent 100 m for its children: at 90 m from the parent it draws.
    let mut forest = parent_and_two_children();
    let at = |d: f32| View::at(Vec3::new(0.0, d, 0.0), 1.0);
    let drawn = forest.select(&at(90.0), &all_ready);
    assert!(close(fade(&drawn, (2, 0)), 1.0) && close(fade(&drawn, (2, 1)), 1.0));
    // A parent that states none takes the farthest of its children's.
    let mut unstated = Forest::new();
    unstated.insert(
        1,
        vec![with_children(entity([0.0; 3], 1000.0, 0.0, None), 2)],
    );
    unstated.insert(
        2,
        vec![
            entity([1.0, 0.0, 0.0], 40.0, 0.0, Some((1, 0))),
            entity([2.0, 0.0, 0.0], 70.0, 0.0, Some((1, 0))),
        ],
    );
    assert_eq!(ids(&unstated.select(&at(95.0), &all_ready)), vec![(1, 0)]);
    // Within 70 m the parent is gone, and the child of 40 m is itself out of its distance.
    let part = unstated.select(&at(60.0), &all_ready);
    assert_eq!(ids(&part), vec![(2, 1)]);
    assert!(close(fade(&part, (2, 1)), 1.0));
    // Half into the band of the child of 40 m it draws as whole as its sibling.
    let shared = unstated.select(&at(50.0), &all_ready);
    assert!(close(fade(&shared, (2, 0)), 1.0) && close(fade(&shared, (2, 1)), 1.0));
    assert_eq!(
        ids(&unstated.select(&at(30.0), &all_ready)),
        vec![(2, 0), (2, 1)]
    );
}

#[test]
fn a_node_with_no_parent_draws_by_its_own_place_and_fades_out_past_its_distance() {
    let mut forest = Forest::new();
    forest.insert(7, vec![entity([10.0, 0.0, 0.0], 50.0, 0.0, None)]);
    assert!(close(
        fade(&forest.select(&along(59.0), &all_ready), (7, 0)),
        1.0
    ));
    assert!(close(
        fade(&forest.select(&along(70.0), &all_ready), (7, 0)),
        0.5
    ));
    assert!(forest.select(&along(81.0), &all_ready).drawn.is_empty());
    // The scale stretches the distance and leaves the band as it is.
    let scaled = |x: f32, scale: f32| View::at(Vec3::new(x, 0.0, 0.0), scale);
    assert!(close(
        fade(&forest.select(&scaled(100.0, 2.0), &all_ready), (7, 0)),
        1.0
    ));
    assert!(forest
        .select(&scaled(59.0, 0.5), &all_ready)
        .drawn
        .is_empty());
}

#[test]
fn each_level_takes_its_own_scale() {
    let mut forest = parent_and_two_children();
    // At 150 m the children, which draw within 100 m, are out of their distance.
    assert_eq!(ids(&forest.select(&along(150.0), &all_ready)), vec![(1, 0)]);
    // Twice the distance for the high level alone brings them in and takes the parent out.
    let mut view = along(150.0);
    view.scales[Level::Hd as usize] = 2.0;
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(2, 0), (2, 1)]);
    // A scale of another level leaves them where they were.
    let mut view = along(150.0);
    view.scales[Level::Slod1 as usize] = 2.0;
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(1, 0)]);
    // And the parent's own level stretches the parent's own distance.
    let mut view = along(1500.0);
    assert!(forest.select(&view, &all_ready).drawn.is_empty());
    view.scales[Level::Lod as usize] = 2.0;
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(1, 0)]);
}

#[test]
fn an_orphan_that_draws_no_farther_than_the_band_fades_over_a_short_one() {
    let orphan = |lod_dist: f32, level: Level| {
        let mut forest = Forest::new();
        let mut node = entity([0.0; 3], lod_dist, 0.0, None);
        node.level = level;
        forest.insert(1, vec![node]);
        forest
    };
    let mut small = orphan(15.0, Level::OrphanHd);
    assert!(close(
        fade(&small.select(&along(17.0), &all_ready), (1, 0)),
        0.6
    ));
    assert!(small.select(&along(21.0), &all_ready).drawn.is_empty());
    // One of another level with the same distance takes the whole band.
    let mut other = orphan(15.0, Level::Lod);
    assert!(close(
        fade(&other.select(&along(21.0), &all_ready), (1, 0)),
        0.7
    ));
    // And so does an orphan that draws farther than the band.
    let mut large = orphan(50.0, Level::OrphanHd);
    assert!(close(
        fade(&large.select(&along(60.0), &all_ready), (1, 0)),
        0.5
    ));
    // The level a map's number names: one past the last is an orphan.
    assert_eq!(Level::from_raw(5), Level::OrphanHd);
    assert_eq!(Level::from_raw(6), Level::Slod4);
    assert_eq!(Level::from_raw(9), Level::OrphanHd);
    assert_eq!(Level::from_raw(0), Level::Hd);
}

/// A forest whose parent drew alone a selection ago, its children loading, 70 m from it.
fn children_about_to_arrive() -> Forest {
    let mut forest = parent_and_two_children();
    let waiting = [(2, 0), (2, 1)];
    let held = forest.select(&along(70.0), &loading(&waiting));
    assert_eq!(ids(&held), vec![(1, 0)]);
    forest
}

#[test]
fn children_that_come_ready_draw_whole_at_once_and_their_parent_gives_way_at_once() {
    let mut forest = children_about_to_arrive();
    // The camera has not moved: a node with a parent still does not fade in by time.
    let drawn = forest.select(&along(70.0), &all_ready);
    assert_eq!(ids(&drawn), vec![(2, 0), (2, 1)]);
    assert!(close(fade(&drawn, (2, 0)), 1.0) && close(fade(&drawn, (2, 1)), 1.0));
    assert!(!drawn.fading);
}

#[test]
fn a_far_node_with_no_parent_fades_in_a_step_a_selection_and_one_of_the_high_level_at_once() {
    let waiting = [(1, 0)];
    let mut far = lone(Level::Lod);
    assert!(far
        .select(&along(50.0), &loading(&waiting))
        .drawn
        .is_empty());
    for k in 1..=4 {
        let drawn = far.select(&along(50.0), &all_ready);
        assert!(
            close(fade(&drawn, (1, 0)), 51.0 / 255.0 * k as f32),
            "step {k}"
        );
        assert!(drawn.fading, "step {k}");
    }
    let whole = far.select(&along(50.0), &all_ready);
    assert!(close(fade(&whole, (1, 0)), 1.0) && !whole.fading);
    // Every far level does; the high level and an orphan never do.
    for (level, first) in [
        (Level::Slod1, 0.2),
        (Level::Slod4, 0.2),
        (Level::Hd, 1.0),
        (Level::OrphanHd, 1.0),
    ] {
        let mut forest = lone(level);
        forest.select(&along(50.0), &loading(&waiting));
        let drawn = forest.select(&along(50.0), &all_ready);
        assert!(close(fade(&drawn, (1, 0)), first), "{level:?}");
    }
}

/// How opaque a far node with no parent draws at its first selection for `view` after one at
/// 50 m along the first axis that waited for it.
fn first_step(view: View) -> f32 {
    let mut forest = lone(Level::Lod);
    forest.select(&along(50.0), &loading(&[(1, 0)]));
    fade(&forest.select(&view, &all_ready), (1, 0))
}

#[test]
fn a_jump_a_fast_camera_or_a_change_of_scale_ends_every_fade_in_time() {
    assert!(close(first_step(along(55.0)), 0.2), "5 m is no jump");
    assert!(close(first_step(along(56.0)), 1.0), "6 m is");
    let moving = |speed: f32| View {
        velocity: Vec3::new(0.0, speed, 0.0),
        ..along(50.0)
    };
    assert!(close(first_step(moving(FADE_MAX_SPEED)), 0.2));
    assert!(close(first_step(moving(FADE_MAX_SPEED + 1.0)), 1.0));
    let scaled = |scale: f32| View::at(Vec3::new(50.0, 0.0, 0.0), scale);
    assert!(close(first_step(scaled(1.0005)), 0.2));
    assert!(close(first_step(scaled(1.01)), 1.0));
    // A first selection is a cut too.
    assert!(close(
        fade(&lone(Level::Lod).select(&along(50.0), &all_ready), (1, 0)),
        1.0
    ));
}

#[test]
fn each_level_fades_out_over_its_own_band() {
    // 5 m past a distance of 100 m: half of a band of 10 m, three quarters of one of 20 m and
    // seven eighths of one of 40 m.
    let bands = [10.0, 40.0, 20.0, 40.0, 10.0, 20.0, 40.0];
    let view = |x: f32| View {
        fade: bands,
        ..along(x)
    };
    for (level, at_105) in [
        (Level::Hd, 0.5),
        (Level::Lod, 0.875),
        (Level::Slod1, 0.75),
        (Level::Slod2, 0.875),
        (Level::Slod3, 0.5),
        (Level::OrphanHd, 0.75),
        (Level::Slod4, 0.875),
    ] {
        let mut forest = lone(level);
        let drawn = forest.select(&view(105.0), &all_ready);
        assert!(close(fade(&drawn, (1, 0)), at_105), "{level:?}");
        assert!(close(
            fade(&lone(level).select(&view(100.0), &all_ready), (1, 0)),
            1.0
        ));
    }
    // Where nothing sets them, every level fades out over 20 m.
    let mut forest = lone(Level::Slod4);
    assert!(close(
        fade(&forest.select(&along(110.0), &all_ready), (1, 0)),
        0.5
    ));
}

#[test]
fn a_parent_takes_what_its_child_leaves_over_the_child_s_own_band() {
    // A child of a band of 30 m: it draws from 130 m and whole from 100 m, and the parent takes the rest.
    let mut bands = [FADE; Level::COUNT];
    bands[Level::Slod2 as usize] = 30.0;
    let mut forest = Forest::new();
    let parent = leveled(entity([0.0; 3], 5000.0, 100.0, None), Level::Slod3);
    forest.insert(1, vec![with_children(parent, 1)]);
    let child = entity([1.0, 0.0, 0.0], 40.0, 0.0, Some((1, 0)));
    forest.insert(2, vec![leveled(child, Level::Slod2)]);
    for (away, parent, child) in [
        (127.0, 0.9, 0.1),
        (115.0, 0.5, 0.5),
        (100.0, 0.0, 1.0),
        (70.0, 0.0, 1.0),
    ] {
        let view = View {
            fade: bands,
            ..along(away)
        };
        let drawn = forest.select(&view, &all_ready);
        assert!(close(fade(&drawn, (1, 0)), parent), "the parent at {away}");
        assert!(close(fade(&drawn, (2, 0)), child), "the child at {away}");
    }
}

#[test]
fn an_archetype_s_category_takes_the_place_of_its_scale_and_keeps_its_radius_whole() {
    let node = |category: Category, level: Level| Node {
        category,
        radius: 10.0,
        ..leveled(entity([0.0; 3], 110.0, 0.0, None), level)
    };
    let at_rest = View::at(Vec3::ZERO, 1.0);
    let reach = |node: Node, view: &View| node.scaled(110.0, view);
    assert!(close(
        reach(node(Category::Other, Level::Hd), &at_rest),
        110.0
    ));
    // Half of what lies past the radius of 10 m.
    assert!(close(
        reach(node(Category::Terrain, Level::Hd), &at_rest),
        60.0
    ));
    assert!(close(
        reach(node(Category::Terrain, Level::OrphanHd), &at_rest),
        100.0
    ));
    assert!(close(
        reach(node(Category::Environment, Level::Lod), &at_rest),
        70.0
    ));
    assert!(close(
        reach(node(Category::Vegetation, Level::Hd), &at_rest),
        110.0
    ));
    // The level's scale multiplies the category's; from 1 on the whole distance stretches.
    let mut twice = at_rest;
    twice.scales[Level::Hd as usize] = 2.0;
    assert!(close(
        reach(node(Category::Terrain, Level::Hd), &twice),
        110.0
    ));
    twice.scales[Level::Hd as usize] = 4.0;
    assert!(close(
        reach(node(Category::Terrain, Level::Hd), &twice),
        220.0
    ));
    // And a selection draws by it: the terrain node is half gone 10 m past its 60 m.
    let mut forest = Forest::new();
    forest.insert(1, vec![node(Category::Terrain, Level::Hd)]);
    assert!(close(
        fade(&forest.select(&along(70.0), &all_ready), (1, 0)),
        0.5
    ));
}

#[test]
fn a_grass_list_draws_short_of_what_it_states_and_a_prop_list_never_does() {
    let of = |kind: Kind, category: Category| Node {
        kind,
        category,
        radius: 10.0,
        ..list([0.0; 3], [1.0; 3], 110.0, None)
    };
    let at_rest = View::at(Vec3::ZERO, 1.0);
    let grass = of(Kind::Grass, Category::Other);
    assert!(close(grass.scale(&at_rest), 0.8));
    assert!(close(grass.scaled(110.0, &at_rest), 90.0));
    // A category of its archetype takes the grass scale's place.
    assert!(close(
        of(Kind::Grass, Category::Vegetation).scaled(110.0, &at_rest),
        110.0
    ));
    // The level's scale shortens an entity and leaves a prop list as it states.
    let half = View::at(Vec3::ZERO, 0.5);
    assert!(close(
        of(Kind::Props, Category::Other).scaled(110.0, &half),
        110.0
    ));
    assert!(close(
        of(Kind::Entity, Category::Other).scaled(110.0, &half),
        60.0
    ));
    assert!(close(
        of(Kind::Props, Category::Other).scaled(110.0, &View::at(Vec3::ZERO, 2.0)),
        220.0
    ));
}

#[test]
fn an_archetype_s_load_flags_give_its_category_the_last_named_bit_winning() {
    for (flags, category) in [
        (0, Category::Other),
        (1 << 1, Category::Other),
        (1 << 16, Category::Vegetation),
        (1, Category::Terrain),
        (1 << 7, Category::Environment),
        (1 | 1 << 16, Category::Terrain),
        (1 | 1 << 7, Category::Environment),
        (1 << 7 | 1 << 16, Category::Environment),
    ] {
        assert_eq!(Category::of(flags), category, "{flags:#x}");
    }
}

#[test]
fn a_camera_narrower_than_forty_five_degrees_draws_farther() {
    assert!(close(camera_scale(60f32.to_radians()), 1.0));
    assert!(close(camera_scale(45f32.to_radians()), 1.0));
    assert!(close(camera_scale(90f32.to_radians()), 1.0));
    // tan 22.5° / tan 15°.
    let expected = (22.5f32).to_radians().tan() / (15f32).to_radians().tan();
    assert!(close(camera_scale(30f32.to_radians()), expected));
    assert!(expected > 1.5);
}

#[test]
fn a_grandparent_draws_while_a_child_that_draws_is_not_ready() {
    let mut forest = Forest::new();
    forest.insert(
        1,
        vec![with_children(entity([0.0; 3], 5000.0, 1000.0, None), 2)],
    );
    forest.insert(
        2,
        vec![
            with_children(entity([0.0; 3], 0.0, 100.0, Some((1, 0))), 1),
            entity([400.0, 0.0, 0.0], 0.0, 100.0, Some((1, 0))),
        ],
    );
    forest.insert(3, vec![entity([0.0; 3], 0.0, 0.0, Some((2, 0)))]);
    let view = View::at(Vec3::new(0.0, 50.0, 0.0), 1.0);
    // The first parent gives way to its child, the second draws itself: three levels at once.
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(2, 1), (3, 0)]);
    // With the grandchild loading its parent draws, which is ready, so the grandparent still gives way.
    let waiting = [(3, 0)];
    assert_eq!(
        ids(&forest.select(&view, &loading(&waiting))),
        vec![(2, 0), (2, 1)]
    );
    // With that parent loading as well the grandparent is held, beside the child that is ready.
    let waiting = [(3, 0), (2, 0)];
    let held = forest.select(&view, &loading(&waiting));
    assert_eq!(ids(&held), vec![(1, 0), (2, 1)]);
    assert!(held.wanted.contains(&(2, 0)) && held.wanted.contains(&(3, 0)));
}

#[test]
fn a_child_whose_parent_map_is_not_loaded_draws_alone_until_it_is() {
    let mut forest = Forest::new();
    forest.insert(2, vec![entity([0.0; 3], 40.0, 0.0, Some((1, 0)))]);
    let view = View::at(Vec3::new(0.0, 30.0, 0.0), 1.0);
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(2, 0)]);
    forest.insert(1, vec![entity([0.0, 500.0, 0.0], 2000.0, 100.0, None)]);
    // The parent stands 470 m away: its child, now judged by that, does not draw.
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(1, 0)]);
    forest.remove(1);
    assert_eq!(ids(&forest.select(&view, &all_ready)), vec![(2, 0)]);
    assert_eq!(forest.len(), (1, 1));
}

#[test]
fn a_list_draws_by_its_box_s_centre_less_each_instance_whose_entity_draws() {
    let mut forest = Forest::new();
    forest.insert(1, vec![list([0.0; 3], [100.0, 100.0, 10.0], 300.0, None)]);
    let mut stand_in = entity([10.0, 10.0, 5.0], 50.0, 0.0, Some((1, 0)));
    stand_in.instance = Some(4);
    let mut other = entity([90.0, 90.0, 5.0], 50.0, 0.0, Some((1, 0)));
    other.instance = Some(9);
    forest.insert(2, vec![stand_in, other]);
    let at = |x: f32, y: f32| View::at(Vec3::new(x, y, 5.0), 1.0);
    // Inside the box, beside the first entity: it draws and its instance is left out.
    let near = forest.select(&at(12.0, 12.0), &all_ready);
    assert_eq!(ids(&near), vec![(2, 0)]);
    assert_eq!(near.lists.len(), 1);
    assert_eq!(&near.skipped[near.lists[0].skipped.clone()], &[4]);
    // 55 m from it, 5 m into its band: it draws at three quarters and its instance is still left out.
    let band = forest.select(&at(10.0, 65.0), &all_ready);
    assert!(close(fade(&band, (2, 0)), 0.75));
    assert_eq!(&band.skipped[band.lists[0].skipped.clone()], &[4]);
    // 290 m from the box's centre the list draws whole, 310 m from it by half, 330 m from it not at all.
    let off = forest.select(&at(340.0, 50.0), &all_ready);
    assert_eq!(
        (off.drawn.len(), off.lists.len(), off.skipped.len()),
        (0, 1, 0)
    );
    assert!(close(fade(&off, (1, 0)), 1.0));
    let fading = forest.select(&at(360.0, 50.0), &all_ready);
    assert!(close(fade(&fading, (1, 0)), 0.5));
    let gone = forest.select(&at(380.0, 50.0), &all_ready);
    assert!(gone.lists.is_empty());
    // An entity still loading leaves its instance drawn and is waited for.
    let waiting = [(2, 0)];
    let held = forest.select(&at(12.0, 12.0), &loading(&waiting));
    assert_eq!((held.drawn.len(), held.skipped.len()), (0, 0));
    assert_eq!(held.wanted, vec![(2, 0)]);
}

#[test]
fn a_parent_gives_way_by_each_child_s_own_scale_on_the_parent_s_radius_and_the_most_opaque_of_them()
{
    let mut forest = Forest::new();
    let parent = Node {
        radius: 50.0,
        ..with_children(entity([0.0; 3], 1000.0, 100.0, None), 2)
    };
    forest.insert(1, vec![parent]);
    // The same stated 100 m: a plain child draws whole to 100 m, an environment one of the second
    // level to 50 + 0.6 × 50 = 80 m on its parent's sphere of 50 m; its own 5 m would leave 62 m.
    let mut shorter = leveled(entity([0.0; 3], 0.0, 0.0, Some((1, 0))), Level::Lod);
    (shorter.category, shorter.radius) = (Category::Environment, 5.0);
    let plain = leveled(entity([0.0; 3], 0.0, 0.0, Some((1, 0))), Level::Lod);
    forest.insert(2, vec![plain, shorter]);
    // 115 m off, inside the plain child's band and past the other's: the plain child draws at a
    // quarter and the parent at the rest.
    let drawn = forest.select(&along(115.0), &all_ready);
    assert!(close(fade(&drawn, (2, 0)), 0.25));
    assert!(close(fade(&drawn, (2, 1)), 0.0));
    assert!(close(fade(&drawn, (1, 0)), 0.75));
    // At 90 m the parent is gone, and the other child, half into its own band, is as whole as the plain one.
    let near = forest.select(&along(90.0), &all_ready);
    assert!(close(fade(&near, (1, 0)), 0.0) && close(fade(&near, (2, 1)), 1.0));
}

#[test]
fn a_list_gives_way_to_the_lists_under_it_each_judged_by_its_own_box() {
    let mut forest = Forest::new();
    let mut far = list([0.0; 3], [1000.0, 100.0, 10.0], 2000.0, None);
    far.child_lod_dist = 200.0;
    forest.insert(1, vec![with_children(far, 1)]);
    // The lower list's centre stands 400 m from the upper's.
    forest.insert(
        2,
        vec![list([0.0; 3], [200.0, 100.0, 10.0], 0.0, Some((1, 0)))],
    );
    let at = |x: f32| View::at(Vec3::new(x, 50.0, 5.0), 1.0);
    let lists = |s: &Selection| s.lists.iter().map(|l| l.node).collect::<Vec<_>>();
    // Beside the lower list and 400 m from the upper's centre: the lower draws in its place.
    assert_eq!(lists(&forest.select(&at(100.0), &all_ready)), vec![(2, 0)]);
    assert_eq!(lists(&forest.select(&at(1500.0), &all_ready)), vec![(1, 0)]);
    // 210 m from the lower list's centre: it draws by half, and the upper by the other half.
    let both = forest.select(&at(310.0), &all_ready);
    assert!(close(fade(&both, (2, 0)), 0.5) && close(fade(&both, (1, 0)), 0.5));
}

#[test]
fn a_prop_list_of_the_batch_levels_draws_on_beside_the_lists_under_it() {
    let mut forest = Forest::new();
    let far = list([0.0; 3], [100.0, 100.0, 10.0], 2000.0, None);
    forest.insert(
        1,
        vec![Node {
            batch: true,
            ..with_children(far, 1)
        }],
    );
    forest.insert(
        2,
        vec![list([0.0; 3], [50.0, 50.0, 10.0], 300.0, Some((1, 0)))],
    );
    let drawn = forest.select(&View::at(Vec3::new(25.0, 25.0, 5.0), 1.0), &all_ready);
    let mut lists: Vec<(NodeId, f32)> = drawn.lists.iter().map(|l| (l.node, l.fade)).collect();
    lists.sort_by_key(|l| l.0);
    assert_eq!(lists, vec![((1, 0), 1.0), ((2, 0), 1.0)]);
}

#[test]
fn a_node_with_no_model_draws_nothing_and_holds_nothing_back() {
    let mut forest = parent_and_two_children();
    let absent: HashMap<NodeId, NodeState> = HashMap::from([((2, 1), NodeState::Absent)]);
    let state = |id: NodeId| *absent.get(&id).unwrap_or(&NodeState::Ready);
    let near = forest.select(&along(70.0), &state);
    assert_eq!((ids(&near), near.wanted), (vec![(2, 0)], vec![]));
}

#[test]
fn a_map_streams_in_within_its_streaming_box_s_sphere() {
    // A box 200 m wide and 40 m high: its sphere is the 20 m half height plus 50, about its middle.
    let streamed = MapBox {
        streaming: (
            Vec3::new(-100.0, -100.0, -20.0),
            Vec3::new(100.0, 100.0, 20.0),
        ),
        entities: (Vec3::splat(-10.0), Vec3::splat(10.0)),
    };
    assert!(streamed.wants(Vec3::new(69.0, 0.0, 0.0)));
    assert!(!streamed.wants(Vec3::new(71.0, 0.0, 0.0)));
    // Inside the box, but past the sphere.
    assert!(!streamed.wants(Vec3::new(90.0, 0.0, 0.0)));
    assert!(!streamed.wants(Vec3::new(50.0, 50.0, 0.0)));
    // A box under a metre of half extent on one side streams in nowhere.
    let thin = MapBox {
        streaming: (
            Vec3::new(-100.0, -100.0, -0.5),
            Vec3::new(100.0, 100.0, 0.5),
        ),
        ..streamed
    };
    assert!(!thin.wants(Vec3::ZERO));
}

#[test]
fn a_chain_of_parents_far_deeper_than_the_levels_is_selected_and_never_overflows_the_stack() {
    // A map's own bytes name each node's parent: here each node names the one before it.
    const DEEP: u32 = 200_000;
    let nodes = (0..DEEP)
        // ⚠ Not 1e9: past about 1e6 m a band of 20 m is lost in the distance's own rounding, and the fade is nought.
        .map(|k| entity([0.0; 3], 1e5, 0.0, k.checked_sub(1).map(|p| (1, p))))
        .collect();
    let mut forest = Forest::new();
    forest.insert(1, nodes);
    let chosen = std::thread::spawn(move || {
        forest
            .select(&View::at(Vec3::ZERO, 1.0), &|_| NodeState::Ready)
            .drawn
            .len()
    })
    .join()
    .expect("the selection ends");
    assert!(chosen > 0);
}

#[test]
fn parents_that_name_each_other_round_a_loop_of_maps_are_drawn_from_the_farthest_of_them() {
    // Map 1's node names map 2's as its parent and map 2's names map 1's; a child hangs under map 2's.
    let mut forest = Forest::new();
    forest.insert(1, vec![entity([0.0; 3], 500.0, 100.0, Some((2, 0)))]);
    forest.insert(
        2,
        vec![
            entity([0.0; 3], 900.0, 100.0, Some((1, 0))),
            entity([0.0; 3], 100.0, 0.0, Some((2, 0))),
        ],
    );
    let far = forest.select(&along(300.0), &all_ready);
    assert_eq!(
        ids(&far),
        vec![(2, 0)],
        "the farthest of the loop stands as its root"
    );
    let near = forest.select(&along(0.0), &all_ready);
    let near = ids(&near);
    assert!(
        near.contains(&(1, 0)) && near.contains(&(2, 1)),
        "what hangs under it draws: {near:?}"
    );
}
