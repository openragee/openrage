use super::super::reference::met;
use super::super::scenes::{placed, ray, scene, stack, stacked, Sequence, IDENTITY, RAY_MASK};
use super::super::walk::{walk_lane, walk_lane_with, Cull, Hit, Walk};
use super::super::{Arena, Structure, Triangle};
use super::*;

/// What `rays` rays of the scene gave under the per-lane walk, answered from the arena and from
/// a query.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// The rays the walk answered from the arena hits.
    pub hits: usize,
    /// The rays whose closest hit is the same to every bit of it, and those whose first hit is
    /// one where the arena's is and is a triangle the query handed over, in two orders of the
    /// candidates each.
    pub closest: usize,
    pub first: usize,
    /// The turns of every walk answered from the arena, and of the two a ray answered from the
    /// query.
    pub turns: [u32; 2],
    /// Under the query: the walks' climbs, their deepest stack, the candidates handed over and
    /// confirmed, and the tests of a box off the candidate's way.
    pub climbs: u32,
    pub deepest: u32,
    pub handed: [u32; 2],
    pub astray: u32,
    /// The rays handed a candidate at all, and those handed more than one: only on the last can
    /// two orders of the candidates differ.
    pub offered: [usize; 2],
}

/// The per-lane walk over the scene, back faces left out, at a ring of 16 and of 4.
///
/// A ring of two is not held here: the walk from the arena loses a child at it, and which rays
/// that costs a hit is the tree's affair, not the query's.
pub fn tally(rays: usize) -> [Tally; 2] {
    let scene = scene();
    let top = scene.top.base;
    let words = scene.arena.words();
    let mut asking = scene.arena.clone();
    assert!(asking.state_query_root(&scene.top, true));
    let asked = asking.words();
    assert_eq!(stated_boxes(words, top), None);
    let nodes = instance_nodes(asked, top);
    let mut sequence = Sequence(777);
    let mut tallies = [Tally::default(), Tally::default()];
    for number in 0..rays as u32 {
        let ray = ray(&mut sequence);
        let handed = handed_over(asked, top, &ray, Form::Lane);
        for tally in &mut tallies {
            tally.offered[0] += usize::from(!handed.is_empty());
            tally.offered[1] += usize::from(handed.len() > 1);
        }
        let cases = [(16, false), (16, true), (4, false), (4, true)];
        for (slot, (ring, first_hit)) in cases.into_iter().enumerate() {
            let tally = &mut tallies[slot / 2];
            let walk = Walk {
                ring,
                mask: RAY_MASK,
                cull: Cull::Back,
                first_hit,
                ..Walk::default()
            };
            let software = walk_lane(words, top, &ray, &walk, &mut |_| true);
            tally.hits += usize::from(!first_hit && software.hit.is_some());
            tally.turns[0] += software.turns;
            // Two orders of the same candidates give the same walk's answer.
            let mut equal = true;
            for seed in [number, !number] {
                let mut query = Query::new(shuffled(handed.clone(), seed));
                let mut body = |probe: &Probe| query.answer(asked, top, probe);
                let queried = walk_lane_with(asked, top, &ray, &walk, &mut body, &mut |_| true);
                tally.turns[1] += queried.turns;
                tally.climbs += queried.climbs;
                tally.deepest = tally.deepest.max(queried.deepest);
                tally.handed[0] += query.counts[1];
                tally.handed[1] += query.counts[2];
                tally.astray += query.astray;
                equal &= match (first_hit, queried.hit) {
                    (false, hit) => hit == software.hit,
                    (true, None) => software.hit.is_none(),
                    (true, Some(hit)) => {
                        let it = |s: &Shown| {
                            let shown = (s.leaf(), nodes[s.instance as usize], s.t);
                            shown == (hit.leaf, hit.instance, hit.t)
                        };
                        software.hit.is_some() && handed.iter().any(it)
                    }
                };
            }
            match first_hit {
                false => tally.closest += usize::from(equal),
                true => tally.first += usize::from(equal),
            }
        }
    }
    println!("{rays} rays at a ring of 16 and of 4: {tallies:?}");
    tallies
}

/// Whether every ray's answer is the same from the query, and the walk never climbed, never
/// held more than the second protocol box, and never asked off a candidate's way.
pub fn holds(tally: &Tally, rays: usize) -> bool {
    (tally.closest, tally.first) == (rays, rays)
        && (tally.climbs, tally.astray) == (0, 0)
        && tally.deepest <= 1
        && 20 * tally.hits > rays
        && tally.handed[1] > 0
        && tally.offered[1] > 0
        && tally.turns[1] < 2 * tally.turns[0]
}

#[test]
fn the_walk_answered_from_a_query_commits_what_it_commits_from_the_arena() {
    for tally in tally(40) {
        assert!(holds(&tally, 40), "{tally:?}");
    }
}

#[test]
fn a_candidate_the_walk_s_own_candidate_code_refuses_is_followed_by_the_next() {
    // Sixty-four triangles one behind another, none opaque, and candidate code that refuses
    // four in five of them by their node alone, so it says the same in any order.
    let (arena, top) = stacked(64, 0.25);
    let mut asking = arena.clone();
    assert!(asking.state_query_root(&top, true));
    let (words, asked, top) = (arena.words(), asking.words(), top.base);
    let mut sequence = Sequence(63);
    let (walks, mut equal, mut refused, mut hits) = (800, 0, 0, 0);
    for number in 0..200u32 {
        let ray = Ray {
            origin: [sequence.within(3.0), sequence.within(3.0), -1.0],
            direction: [sequence.within(0.5), sequence.within(0.5), 18.0],
            t_min: 0.0,
            t_max: 1.0,
        };
        let handed = handed_over(asked, top, &ray, Form::Lane);
        for first_hit in [false, true] {
            let walk = Walk {
                first_hit,
                ..Walk::default()
            };
            let mut commits = |hit: &Hit| {
                let commits = (hit.leaf >> 3).is_multiple_of(5);
                refused += u32::from(!commits);
                commits
            };
            let software = walk_lane(words, top, &ray, &walk, &mut commits);
            hits += u32::from(software.hit.is_some());
            for seed in [number, !number] {
                let mut query = Query::new(shuffled(handed.clone(), seed));
                let mut body = |probe: &Probe| query.answer(asked, top, probe);
                let queried = walk_lane_with(asked, top, &ray, &walk, &mut body, &mut commits);
                // A first hit is whichever committed candidate the order shows first.
                equal += u32::from(match first_hit {
                    false => queried.hit == software.hit,
                    true => {
                        let committed = |hit: Hit| (hit.leaf >> 3).is_multiple_of(5);
                        queried.hit.is_some() == software.hit.is_some()
                            && queried.hit.is_none_or(committed)
                    }
                });
            }
        }
    }
    assert_eq!(equal, walks, "{refused} refusals, {hits} hits");
    assert!(
        refused > walks && hits > 100,
        "{refused} refusals, {hits} hits"
    );
}

#[test]
fn a_top_level_of_one_instance_of_one_triangle_or_of_one_pair_is_led_through_too() {
    // No box in either level: the first protocol box's child is the instance, and the instance's
    // root is the triangle's node. Of a pair, the candidate may be the node's second triangle.
    let corners = [
        [0.0; 3],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [1.0, 1.0, 0.0f32],
    ];
    let triangle = |a: usize, b: usize, c: usize, primitive: u32| Triangle {
        corners: [corners[a], corners[b], corners[c]],
        geometry: 0,
        primitive,
        opaque: true,
    };
    for triangles in [
        vec![triangle(0, 1, 2, 0)],
        vec![triangle(0, 1, 2, 0), triangle(1, 3, 2, 1)],
    ] {
        let mut arena = Arena::new();
        let bottom = arena.bottom_level(&triangles).expect("a bottom level");
        let top = (arena.top_level(&[placed(bottom, IDENTITY, [10.0, 0.0, 0.0], 0xFF)]))
            .expect("a top level");
        let mut asking = arena.clone();
        assert!(asking.state_query_root(&top, true));
        for (x, y) in [(10.25, 0.25), (10.75, 0.75), (12.0, 0.25)] {
            let ray = Ray {
                origin: [x, y, 1.0],
                direction: [0.0, 0.0, -1.0],
                t_min: 0.0,
                t_max: 4.0,
            };
            let walk = Walk::default();
            let software = walk_lane(arena.words(), top.base, &ray, &walk, &mut |_| true);
            let handed = handed_over(asking.words(), top.base, &ray, Form::Lane);
            let mut query = Query::new(handed);
            let mut body = |probe: &Probe| query.answer(asking.words(), top.base, probe);
            let queried = walk_lane_with(
                asking.words(),
                top.base,
                &ray,
                &walk,
                &mut body,
                &mut |_| true,
            );
            assert_eq!(queried.hit, software.hit, "a ray down at {x}, {y}");
        }
    }
}

#[test]
fn a_triangle_a_query_shows_answers_once() {
    // One instance of one triangle: the first protocol box answers the instance, whose root is
    // the triangle's node. No walk asks a triangle twice of one candidate, so the instruction is
    // asked here as a walk would ask it, and then once more.
    let (arena, top) = stacked(1, 1.0);
    let asking = asking(&arena, &top);
    let (asked, top) = (asking.words(), top.base);
    let ray = Ray {
        origin: [0.0, 0.0, -1.0],
        direction: [0.0, 0.0, 2.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let mut query = Query::new(handed_over(asked, top, &ray, Form::Lane));
    let probe = |base: u32, node: u32| Probe {
        base,
        node,
        t_max: ray.t_max,
        origin: ray.origin,
        direction: ray.direction,
        inverse: ray.direction.map(|x| 1.0 / x),
    };
    let [first, again] = stated_boxes(asked, top).expect("the protocol's boxes");
    let [instance, back, ..] = query.answer(asked, top, &probe(top, first));
    assert_eq!((instance & 7, back), (TYPE_INSTANCE, again));
    let at = top + offset(instance);
    let bottom = word(asked, at, INSTANCE_BOTTOM) << 8;
    let leaf = word(asked, at, INSTANCE_ROOT) & INSTANCE_NO_ROOT;
    let shown = query
        .answer(asked, top, &probe(bottom, leaf))
        .map(f32::from_bits);
    assert!((shown[0] / shown[1] - 0.5).abs() < 1e-6 && shown[1].abs() == 1.0);
    assert_eq!(query.answer(asked, top, &probe(bottom, leaf)), MISS);
    // And a triangle that is not the candidate's does not answer at all.
    assert_eq!(query.answer(asked, top, &probe(bottom, leaf | 1)), MISS);
}

#[test]
fn a_query_hands_over_every_triangle_before_the_far_end_whatever_its_mask_or_its_face() {
    let scene = scene();
    let (words, top) = (scene.arena.words(), scene.top.base);
    let nodes = instance_nodes(words, top);
    let mut sequence = Sequence(31);
    let (mut handed, mut masked, mut back, mut moved) = (0, 0, 0, 0);
    for seed in 0..120 {
        let ray = Ray {
            t_max: 0.75,
            ..ray(&mut sequence)
        };
        let shown = handed_over(words, top, &ray, Form::Lane);
        assert!(shown.iter().all(|s| s.t > 0.0 && s.t < 0.75));
        // Another order, the same triangles.
        let again = shuffled(shown.clone(), seed);
        let key = |s: &Shown| (s.instance, s.primitive, s.t.to_bits());
        let sorted = |mut list: Vec<Shown>| {
            list.sort_by_key(key);
            list
        };
        moved += usize::from(again != shown);
        assert_eq!(sorted(again), sorted(shown.clone()));
        handed += shown.len();
        let out = |s: &&Shown| {
            let at = top + offset(nodes[s.instance as usize]);
            (word(words, at, INSTANCE_ID_MASK) >> 24) & u32::from(RAY_MASK) == 0
        };
        masked += shown.iter().filter(out).count();
        back += shown.iter().filter(|s| !s.front).count();
    }
    assert!(
        handed > 10 && masked > 0 && back > 0 && moved > 0,
        "{handed} handed over: {masked} of a mask the rays lack, {back} back faces, {moved} \
         lists the shuffle moved"
    );
}

/// A walk of `top` answered from a query that hands over `handed`, with what the query counted.
fn asked_of(
    asked: &[u32],
    top: u32,
    ray: &Ray,
    walk: &Walk,
    handed: Vec<Shown>,
    confirms: bool,
) -> (Option<Hit>, [u32; 3]) {
    let mut query = Query::new(handed);
    query.confirms = confirms;
    let mut body = |probe: &Probe| query.answer(asked, top, probe);
    let out = walk_lane_with(asked, top, ray, walk, &mut body, &mut |_| true);
    assert_eq!(query.astray, 0);
    (out.hit, query.counts)
}

/// `arena` with the header of `top` stating the protocol's first box.
fn asking(arena: &Arena, top: &Structure) -> Arena {
    let mut asking = arena.clone();
    assert!(asking.state_query_root(top, true));
    asking
}

#[test]
fn of_two_triangles_at_one_distance_the_walk_keeps_the_one_it_is_shown_first() {
    // One opaque triangle placed twice where it is: two candidates at one distance, to the bit.
    let mut arena = Arena::new();
    let triangle = Triangle {
        opaque: true,
        ..stack(1, 1.0)[0]
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let twice = [0, 1].map(|_| placed(bottom, IDENTITY, [0.0, 0.0, 3.0], 0xFF));
    let top = arena.top_level(&twice).expect("a top level");
    let asking = asking(&arena, &top);
    let (words, asked, top) = (arena.words(), asking.words(), top.base);
    let ray = Ray {
        origin: [0.0; 3],
        direction: [0.0, 0.0, 8.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let handed = handed_over(asked, top, &ray, Form::Lane);
    assert_eq!(handed.len(), 2);
    assert_eq!(handed[0].t.to_bits(), handed[1].t.to_bits());
    assert_ne!(handed[0].instance, handed[1].instance);
    for first_hit in [false, true] {
        let walk = Walk {
            first_hit,
            ..Walk::default()
        };
        let software = walk_lane(words, top, &ray, &walk, &mut |_| true);
        let taken = met(words, top, &software.hit.expect("a hit")).instance;
        for confirms in [true, false] {
            let mut verdicts = Vec::new();
            for order in [handed.clone(), vec![handed[1], handed[0]]] {
                // The last of the list is shown first.
                let shown_first = order[1].instance;
                let (hit, counts) = asked_of(asked, top, &ray, &walk, order, confirms);
                let named = met(asked, top, &hit.expect("a hit")).instance;
                assert_eq!(named, instance_index(asked, top, shown_first));
                verdicts.push(agreed(software.hit.as_ref(), hit.as_ref()));
                // A device told of the commit withholds the other; one that is not shows it,
                // and the walk's own compare refuses it. A walk that ends on its first hit
                // never comes back to be shown it.
                let shown = if confirms || first_hit { 1 } else { 2 };
                assert_eq!(counts, [1, shown, u32::from(confirms && !first_hit)]);
            }
            verdicts.sort_by_key(|verdict| *verdict == Agreed::Tied);
            assert_eq!(
                verdicts,
                [Agreed::Same, Agreed::Tied],
                "the arena took {taken}"
            );
        }
    }
}

/// The place in the list its top level was built from of the instance numbered `number`.
fn instance_index(words: &[u32], top: u32, number: u32) -> u32 {
    let node = instance_nodes(words, top)[number as usize];
    word(words, top + offset(node), INSTANCE_INDEX) & 0xFFFF
}

#[test]
fn a_hit_at_nought_is_the_arena_s_and_no_device_shows_it() {
    // Two triangles, and a ray that starts on the first.
    let (arena, top) = stacked(2, 4.0);
    let asking = asking(&arena, &top);
    let (words, asked, top) = (arena.words(), asking.words(), top.base);
    let ray = Ray {
        origin: [0.0; 3],
        direction: [0.0, 0.0, 8.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let walk = Walk::default();
    let software = walk_lane(words, top, &ray, &walk, &mut |_| true);
    assert_eq!(software.hit.map(|hit| hit.t), Some(0.0));
    let handed = handed_over(asked, top, &ray, Form::Lane);
    assert_eq!(handed.len(), 1);
    // Nor does one show a triangle at the far end of a ray.
    let short = Ray {
        t_max: handed[0].t,
        ..ray
    };
    assert!(handed_over(asked, top, &short, Form::Lane).is_empty());
    let (hit, _) = asked_of(asked, top, &ray, &walk, handed, true);
    assert_eq!(hit.map(|hit| met(asked, top, &hit).primitive), Some(1));
    assert_eq!(
        agreed(software.hit.as_ref(), hit.as_ref()),
        Agreed::AtNought
    );
    // The verdicts' own order: the same first, then nought, then a tie.
    let other = hit.map(|hit| Hit { leaf: 1, ..hit });
    assert_eq!(agreed(hit.as_ref(), hit.as_ref()), Agreed::Same);
    assert_eq!(agreed(None, None), Agreed::Same);
    assert_eq!(agreed(hit.as_ref(), other.as_ref()), Agreed::Tied);
    assert_eq!(agreed(hit.as_ref(), None), Agreed::Apart);
    assert_eq!(agreed(None, hit.as_ref()), Agreed::Apart);
    assert_eq!(agreed(hit.as_ref(), software.hit.as_ref()), Agreed::Apart);
}

/// What `rays` rays of the scene gave under a walk led by distances that are not the arena's.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Moved {
    /// The rays handed a candidate, and the candidates handed over.
    pub offered: usize,
    pub candidates: usize,
    /// The rays whose walk committed the nearest candidate as the device stated it, to the
    /// distance's last bit.
    pub kept: usize,
    /// The rays whose walk committed that candidate's own triangle, and those where another
    /// candidate lies at the same distance to the bit, of which the walk may keep either.
    pub same_triangle: usize,
    pub tied: usize,
    /// The rays whose committed distance lies one place from the arena's own hit, on the
    /// arena's own triangle.
    pub one_place: usize,
}

/// The per-lane walk over the scene, no face left out and every instance admitted, answered
/// from candidates whose every distance is the float after or before the arena's.
pub fn moved(rays: usize) -> Moved {
    let scene = scene();
    let asking = asking(&scene.arena, &scene.top);
    let (words, asked, top) = (scene.arena.words(), asking.words(), scene.top.base);
    let nodes = instance_nodes(asked, top);
    let mut sequence = Sequence(2026);
    let mut moved = Moved::default();
    for number in 0..rays as u32 {
        let ray = ray(&mut sequence);
        let walk = Walk::default();
        let software = walk_lane(words, top, &ray, &walk, &mut |_| true);
        let mut handed = handed_over(asked, top, &ray, Form::Lane);
        if handed.is_empty() {
            assert_eq!(software.hit, None);
            continue;
        }
        for (place, shown) in handed.iter_mut().enumerate() {
            let bits = shown.t.to_bits();
            shown.t = f32::from_bits(if place % 2 == 0 { bits + 1 } else { bits - 1 });
        }
        // A candidate the device puts at or past the far end is not one it shows.
        handed.retain(|shown| shown.t < ray.t_max);
        let Some(nearest) = handed.iter().copied().min_by(|a, b| a.t.total_cmp(&b.t)) else {
            continue;
        };
        moved.offered += 1;
        moved.candidates += handed.len();
        let as_near = |shown: &&Shown| shown.t.to_bits() == nearest.t.to_bits();
        moved.tied += usize::from(handed.iter().filter(as_near).count() > 1);
        let (hit, _) = asked_of(asked, top, &ray, &walk, shuffled(handed, number), true);
        let hit = hit.expect("a walk shown a candidate commits one");
        let arena = software
            .hit
            .expect("the arena's walk hits where a candidate is");
        let it = (nearest.leaf(), nodes[nearest.instance as usize]);
        moved.kept += usize::from(hit.t.to_bits() == nearest.t.to_bits());
        moved.same_triangle += usize::from((hit.leaf, hit.instance) == it);
        let apart = hit.t.to_bits().abs_diff(arena.t.to_bits());
        moved.one_place += usize::from(apart == 1 && (arena.leaf, arena.instance) == it);
    }
    println!("{rays} rays led by a device's own distances: {moved:?}");
    moved
}

/// Whether every walk kept the nearest candidate at the device's own distance, and all but a
/// few of them the arena's own triangle one place from the arena's distance.
pub fn follows(moved: &Moved) -> bool {
    moved.offered > 5
        && moved.candidates > moved.offered
        && moved.kept == moved.offered
        && moved.same_triangle + moved.tied >= moved.offered
        && 10 * moved.one_place >= 9 * moved.offered
}

#[test]
fn a_walk_led_by_a_device_s_own_distances_commits_them_to_the_bit() {
    let moved = moved(60);
    assert!(follows(&moved), "{moved:?}");
}

#[test]
fn a_device_s_triangles_are_the_models_own_in_the_order_of_their_nodes() {
    let scene = scene();
    let words = scene.arena.words();
    for (bottom, triangles) in scene.bottoms.iter().zip(&scene.triangles) {
        let device = device_triangles(words, bottom.base);
        assert_eq!(
            device.len() as u32,
            2 * word(words, bottom.base, HEADER_PARENTS) / UNIT
        );
        let mut found = 0;
        for (slot, corners) in device.iter().enumerate() {
            let at = bottom.base + (slot as u32 / 2) * UNIT;
            let model = triangles
                .iter()
                .find(|triangle| triangle.corners == *corners);
            let Some(model) = model else {
                assert_eq!(
                    *corners, [[0.0; 3]; 3],
                    "slot {slot} holds no model's triangle"
                );
                continue;
            };
            // The slot is the node's unit and the triangle's place in it.
            let primitive = word(words, at, TRIANGLE_PRIMITIVE + 4 * (slot as u32 % 2));
            assert_eq!(primitive, model.primitive);
            found += 1;
        }
        assert_eq!(found, triangles.len());
    }
}

/// The count the plan's prototype ran, which a default run has no time for.
pub mod slow {
    #[test]
    fn four_thousand_walks_led_by_a_device_s_own_distances_commit_them_to_the_bit() {
        let moved = super::moved(4000);
        assert!(super::follows(&moved), "{moved:?}");
    }

    #[test]
    fn four_thousand_rays_answered_from_a_query_commit_what_they_commit_from_the_arena() {
        for tally in super::tally(4000) {
            assert!(super::holds(&tally, 4000), "{tally:?}");
        }
    }
}
