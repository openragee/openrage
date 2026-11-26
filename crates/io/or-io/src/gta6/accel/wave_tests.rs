use super::super::query::{handed_over, shuffled, Query};
use super::super::reference::{every_triangle, met, world, Met, Placed, Verdict};
use super::super::scenes::{chain, placed, ray, scene, stack, Scene, Sequence, IDENTITY, RAY_MASK};
use super::super::{Arena, Instance, Structure, Triangle};
use super::*;

/// The wave's walk answered from the arena's bytes, each lane's own flags `flags`, with
/// `candidate` as the walk's candidate code.
fn walked(
    words: &[u32],
    top: u32,
    rays: &[Option<Ray>],
    flags: &[u32],
    wave: &Wave,
    candidate: &mut dyn FnMut(usize, &Hit) -> bool,
) -> WaveOutcome {
    let mut code = Code {
        body: &mut |_, probe| probe.software(words),
        candidate,
    };
    walk_wave(words, top, rays, flags, wave, &mut code)
}

/// A walk of `wave` to each lane's first hit with no triangle opaque, so that its candidate code
/// is asked of every hit a lane could commit: the outcome, and what each lane was offered in the
/// order it walked, each taken.
fn first_offered(
    words: &[u32],
    top: u32,
    rays: &[Option<Ray>],
    wave: &Wave,
) -> (WaveOutcome, Vec<Vec<Met>>) {
    let wave = Wave {
        flags: wave.flags | RAY_FORCE_NON_OPAQUE,
        first_hit: true,
        ..*wave
    };
    let mut offered = vec![Vec::new(); rays.len()];
    let out = walked(words, top, rays, &[], &wave, &mut |lane, hit| {
        offered[lane].push(met(words, top, hit));
        true
    });
    (out, offered)
}

/// The rays of wave `number`, one a lane. An even wave's are the scene's own, each from anywhere
/// to anywhere; an odd wave's leave one point for places about one instance, each with its own
/// far end, as a tile of shadow rays does. Every eighth wave leaves a quarter of its lanes out.
fn rays_of(
    scene: &Scene,
    sequence: &mut Sequence,
    lanes: usize,
    number: usize,
) -> Vec<Option<Ray>> {
    let origin: [f32; 3] = std::array::from_fn(|_| sequence.within(60.0));
    let aimed = (sequence.next() * scene.instances.len() as f32) as usize;
    let about = scene.instances[aimed].object_to_world.map(|row| row[3]);
    (0..lanes)
        .map(|_| {
            if number % 8 == 7 && sequence.next() < 0.25 {
                return None;
            }
            if number.is_multiple_of(2) {
                return Some(ray(sequence));
            }
            let direction = std::array::from_fn(|k| {
                let to = about[k] + sequence.within(12.0) - origin[k];
                if to == 0.0 {
                    1e-9
                } else {
                    to
                }
            });
            Some(Ray {
                origin,
                direction,
                t_min: 0.0,
                t_max: 0.6 + sequence.next(),
            })
        })
        .collect()
}

/// What the lanes of some waves gave.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// The lanes that traced, and those of them every triangle says hit beyond doubt.
    pub rays: usize,
    pub hits: usize,
    /// For the closest hit and the first hit, against the test of every triangle that shares
    /// nothing with the walk: the lanes that are the same beyond doubt, and those an unsure
    /// triangle could answer where the walk gave a possible answer. A first hit is one only
    /// where it is the first the lane was offered in the order it walked.
    pub same: [usize; 2],
    pub possible: [usize; 2],
    /// The lanes whose first hit lies farther than their closest: where a walk that ends on its
    /// first hit shows itself.
    pub sooner: usize,
    /// The lanes whose closest hit under the query's answers is the arena's own to every bit of
    /// it, and those whose first hit is one where the arena's is.
    pub query: [usize; 2],
    /// The lanes a query hands a candidate at all, and those it hands more than one.
    pub offered: [usize; 2],
    /// The most pairs on a wave's stack, the pushes dropped, the lanes' climbs, and the query's
    /// tests of a box off its candidate's way: under the arena's answers, then under the query's.
    pub deepest: [u32; 2],
    pub dropped: [u32; 2],
    pub climbs: [u32; 2],
    pub astray: u32,
}

/// `waves` waves of `lanes` lanes over the scene, back faces left out: each lane against every
/// triangle, and the walk answered from a query against the walk answered from the arena.
pub fn tally(lanes: u32, waves: usize) -> Tally {
    let scene = scene();
    let world = scene.world();
    let top = scene.top.base;
    let words = scene.arena.words();
    let mut asking = scene.arena.clone();
    assert!(asking.state_query_root(&scene.top, true));
    let asked = asking.words();
    let mut sequence = Sequence(4242);
    let mut tally = Tally::default();
    for number in 0..waves {
        let rays = rays_of(&scene, &mut sequence, lanes as usize, number);
        // What a device hands each lane's query, in an order of its own.
        let handed: Vec<_> = (rays.iter().zip(0u32..))
            .map(|(ray, lane)| {
                let of = |ray: &Ray| handed_over(asked, top, ray, Form::Wave);
                shuffled(
                    ray.as_ref().map_or(Vec::new(), of),
                    97 * number as u32 + lane,
                )
            })
            .collect();
        tally.offered[0] += handed.iter().filter(|list| !list.is_empty()).count();
        tally.offered[1] += handed.iter().filter(|list| list.len() > 1).count();
        let mut closest: Vec<Option<Hit>> = Vec::new();
        for (slot, first_hit) in [false, true].into_iter().enumerate() {
            let wave = Wave {
                lanes,
                mask: RAY_MASK,
                flags: RAY_CULL_BACK,
                first_hit,
                ..Wave::default()
            };
            let (software, offered) = match first_hit {
                false => (
                    walked(words, top, &rays, &[], &wave, &mut |_, _| true),
                    Vec::new(),
                ),
                true => first_offered(words, top, &rays, &wave),
            };
            let mut queries: Vec<Query> = handed.iter().cloned().map(Query::new).collect();
            let mut code = Code {
                body: &mut |lane, probe| queries[lane].answer(asked, top, probe),
                candidate: &mut |_, _| true,
            };
            let queried = walk_wave(asked, top, &rays, &[], &wave, &mut code);
            tally.astray += queries.iter().map(|query| query.astray).sum::<u32>();
            for (body, out) in [&software, &queried].into_iter().enumerate() {
                tally.deepest[body] = tally.deepest[body].max(out.deepest);
                tally.dropped[body] += out.dropped;
                tally.climbs[body] += out.climbs;
            }
            for (lane, ray) in rays.iter().enumerate() {
                let Some(ray) = ray else { continue };
                let answer = every_triangle(&world, ray, RAY_MASK, Cull::Back);
                let found = software.hits[lane].map(|hit| met(words, top, &hit));
                let verdict = match first_hit {
                    false => answer.closest(found),
                    true => answer.first(found, &offered[lane]),
                };
                tally.same[slot] += usize::from(verdict == Verdict::Same);
                tally.possible[slot] += usize::from(verdict == Verdict::Possible);
                let equal = match first_hit {
                    false => queried.hits[lane] == software.hits[lane],
                    true => queried.hits[lane].is_some() == software.hits[lane].is_some(),
                };
                tally.query[slot] += usize::from(equal);
                if !first_hit {
                    tally.rays += 1;
                    tally.hits += usize::from(!answer.sure.is_empty());
                } else if let (Some(first), Some(Some(closest))) =
                    (software.hits[lane], closest.get(lane))
                {
                    tally.sooner += usize::from(first.t > closest.t);
                }
            }
            if !first_hit {
                closest = software.hits;
            }
        }
    }
    println!("{lanes} lanes, {waves} waves: {tally:?}");
    tally
}

/// Whether no lane differs from every triangle, no more than one in twenty is one an unsure
/// triangle could answer, the query's answers gave what the arena's did, and no push was lost.
pub fn holds(tally: &Tally) -> bool {
    let rays = tally.rays;
    let apart = |slot: usize| tally.same[slot] + tally.possible[slot] == rays;
    (0..2).all(|slot| apart(slot) && 20 * tally.possible[slot] <= rays)
        && tally.query == [rays; 2]
        && tally.dropped == [0; 2]
        && tally.astray == 0
        && 20 * tally.hits > rays
        && tally.offered[1] > 0
}

#[test]
fn every_lane_of_a_wave_finds_what_every_triangle_finds_and_the_same_from_a_query() {
    let narrow = tally(32, 2);
    assert!(narrow.rays > 60 && holds(&narrow), "{narrow:?}");
    let wide = tally(64, 1);
    assert!(wide.rays > 60 && holds(&wide), "{wide:?}");
    // The query's answers name one child a box, so no lane's own stack holds anything.
    assert_eq!((narrow.climbs[1], wide.climbs[1]), (0, 0));
}

#[test]
fn the_wave_s_stack_holds_128_pairs_and_a_push_past_them_is_lost() {
    // 43 boxes leave three pairs waiting each but the last, which leaves two: 128 pairs, the
    // stack's whole size, before the first instance is walked.
    let chain = chain(43);
    let (words, top, instances) = (&chain.words[..], chain.top, &chain.instances);
    let models: Vec<_> = (instances.iter())
        .map(|i| (&chain.triangles[..], i))
        .collect();
    let world = world(&models);
    // The second instance of the last box is the last pair pushed. A lane aims at it, and the
    // other lanes at instances all along the chain.
    let last = instances.len() - 2;
    let aims: Vec<usize> = (0..64)
        .map(|lane| (last + 2 * lane) % instances.len())
        .collect();
    let rays: Vec<Option<Ray>> = (aims.iter())
        .map(|aim| {
            let at = instances[*aim].object_to_world.map(|row| row[3]);
            Some(Ray {
                origin: [at[0] + 0.125, at[1] + 0.125, -5.0],
                direction: [0.0, 0.0, 450.0],
                t_min: 0.0,
                t_max: 1.0,
            })
        })
        .collect();
    // The lanes that find what every triangle finds, by their numbers.
    let found = |stack: u32| {
        let wave = Wave {
            lanes: 64,
            stack,
            ..Wave::default()
        };
        let out = walked(words, top, &rays, &[], &wave, &mut |_, _| true);
        let right = |lane: &usize| {
            let ray = rays[*lane].as_ref().expect("a ray a lane");
            let answer = every_triangle(&world, ray, 0xFF, Cull::None);
            let named = out.hits[*lane].map(|hit| met(words, top, &hit));
            assert_eq!(
                answer.sure.len(),
                1,
                "a lane's ray meets its own instance alone"
            );
            assert_eq!(answer.sure[0].instance as usize, aims[*lane]);
            answer.closest(named) == Verdict::Same
        };
        let lanes: Vec<usize> = (0..64).filter(right).collect();
        (lanes, out.deepest, out.dropped)
    };
    assert_eq!(Wave::default().stack, 256);
    assert_eq!(found(256), ((0..64).collect(), 128, 0));
    // One pair fewer, and the lane whose instance was the last pair pushed finds nothing.
    assert_eq!(found(254), ((1..64).collect(), 127, 1));
}

/// `triangles` placed once with the instance's `flags` and `contribution`, and their top level.
fn once(triangles: &[Triangle], flags: u32, contribution: u32) -> (Arena, Structure) {
    let mut arena = Arena::new();
    let bottom = arena.bottom_level(triangles).expect("a bottom level");
    let instance = Instance {
        flags: flags as u8,
        contribution,
        ..placed(bottom, IDENTITY, [0.0; 3], 0xFF)
    };
    let top = arena.top_level(&[instance]).expect("a top level");
    (arena, top)
}

/// A ray along z from `from` to `to`, where every triangle of a [`stack`] lies across it.
fn along(from: f32, to: f32) -> Ray {
    Ray {
        origin: [0.0, 0.0, from],
        direction: [0.0, 0.0, to - from],
        t_min: 0.0,
        t_max: 1.0,
    }
}

#[test]
fn a_lane_below_an_instance_walks_triangles_first_then_boxes_each_nearest_first() {
    // Five triangles four apart, none opaque: the root holds the lowest three and a box of the
    // upper two. A ray from above meets the box first, and a lane still takes the root's
    // triangles before it, the nearest first.
    let (arena, top) = once(&stack(5, 4.0), 0, 0);
    let (words, top, lane) = (arena.words(), top.base, 3usize);
    let mut rays = vec![None; 32];
    rays[lane] = Some(along(20.0, -20.0));
    let wave = Wave::default();
    let step = wave.lanes as i32 * 4;
    let mut order = Vec::new();
    let out = walked(words, top, &rays, &[], &wave, &mut |at, hit| {
        order.push((at, met(words, top, hit).primitive));
        false
    });
    assert_eq!(order, [2, 1, 0, 4, 3].map(|primitive| (lane, primitive)));
    // The instance in the top level; the root, its three triangles, the box and its two below
    // it. The emptied stack is one entry under the lane's own address.
    let empty = lane as i32 * 4 - step;
    assert_eq!(
        (
            out.hits[lane],
            out.counts[lane],
            out.stacks[lane],
            out.turns
        ),
        (None, 8, empty, 1)
    );
    // A lane that does not trace counts nothing and keeps the word of a lane that walked nothing.
    assert_eq!((out.counts[0], out.stacks[0]), (0, -step));
    // The first hit a lane is done on is the first of that order: the instance, the root and
    // one triangle. The root's other two triangles and its box were pushed, and the turn's own
    // pop took one of them back.
    let first = Wave {
        first_hit: true,
        ..wave
    };
    let out = walked(words, top, &rays, &[], &first, &mut |_, _| true);
    let named = out.hits[lane].map(|hit| met(words, top, &hit).primitive);
    assert_eq!(
        (named, out.counts[lane], out.stacks[lane]),
        (Some(2), 3, lane as i32 * 4 + step)
    );
    // And the closest is the last of it but one: the nearest of the box's two.
    let out = walked(words, top, &rays, &[], &wave, &mut |_, _| true);
    let named = out.hits[lane].map(|hit| met(words, top, &hit).primitive);
    assert_eq!(
        (named, out.counts[lane], out.stacks[lane]),
        (Some(4), 8, empty)
    );
}

/// Two instances of one opaque triangle across z, the second behind the first and half a unit
/// aside, with the rays of three lanes: one through both, one through the second alone and one
/// that meets neither; and the two triangles in the world.
fn two_behind() -> (Arena, Structure, Vec<Option<Ray>>, Vec<Placed>) {
    let mut arena = Arena::new();
    let triangle = Triangle {
        opaque: true,
        ..stack(1, 1.0)[0]
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let instances = [
        placed(bottom, IDENTITY, [0.0, 0.0, 5.0], 0xFF),
        placed(bottom, IDENTITY, [0.5, 0.5, 10.0], 0xFF),
    ];
    let top = arena.top_level(&instances).expect("a top level");
    let from = |x: f32, y: f32| {
        Some(Ray {
            origin: [x, y, 0.0],
            direction: [0.0, 0.0, 20.0],
            t_min: 0.0,
            t_max: 1.0,
        })
    };
    let rays = vec![from(0.0, 0.0), from(2.3, -0.4), from(50.0, 50.0)];
    let model = [triangle];
    let models = instances.each_ref().map(|instance| (&model[..], instance));
    (arena, top, rays, world(&models))
}

/// Five lanes over [`two_behind`]: one through both triangles, one through the near one alone and
/// three through the far one alone. The far instance is named first by three lanes and the near
/// one by two, so the wave goes to the far one first, and the pair of the near one waits for the
/// lane through both and for the lane through it alone.
fn five_lanes() -> Vec<Option<Ray>> {
    let from = |(x, y): (f32, f32)| {
        Some(Ray {
            origin: [x, y, 0.0],
            direction: [0.0, 0.0, 20.0],
            t_min: 0.0,
            t_max: 1.0,
        })
    };
    [
        (0.0, 0.0),
        (-0.8, -0.8),
        (2.3, -0.4),
        (2.2, -0.4),
        (2.1, -0.4),
    ]
    .map(from)
    .to_vec()
}

#[test]
fn a_wave_goes_first_where_most_of_its_lanes_want_to_go_and_a_first_hit_ends_a_lane() {
    let (arena, top, rays, _) = two_behind();
    let (words, top) = (arena.words(), top.base);
    let run = |first_hit: bool, bound: u32| {
        let wave = Wave {
            first_hit,
            bound,
            ..Wave::default()
        };
        let out = walked(words, top, &rays, &[], &wave, &mut |_, _| true);
        let named = |lane: usize| out.hits[lane].map(|hit| met(words, top, &hit).instance);
        ([0, 1, 2].map(named), out.counts.clone(), out.turns)
    };
    // The near instance is named first by one lane; the far one first by one and second by one,
    // so the wave goes to the far one first. A lane that is done on its first hit there has
    // left the wave when the near instance's turn comes, and the pair that waits for it is
    // passed over: the root and one instance are all the wave's turns.
    assert_eq!(
        run(true, 0xFFFF),
        ([Some(1), Some(1), None], vec![3, 3, 2], 2)
    );
    // A lane that wants the closest goes on to the near instance. Every lane with a ray counts
    // each turn of the top level, whether the turn's node is its own or not, and its own turns
    // below an instance.
    assert_eq!(
        run(false, 0xFFFF),
        ([Some(0), Some(1), None], vec![5, 4, 3], 3)
    );
    // A lane is done at the turn below an instance that brings its count to the bound: the
    // lane through both never comes to the near instance.
    assert_eq!(Wave::default().bound, 0xFFFF);
    assert_eq!(run(false, 3), ([Some(1), Some(1), None], vec![3, 3, 2], 2));
}

#[test]
fn a_lane_that_has_left_the_wave_still_counts_the_top_level_s_turns() {
    // One lane through the near instance's triangle alone; another through that instance's box
    // beside its triangle, and through the far instance's triangle.
    let (arena, top, ..) = two_behind();
    let (words, top) = (arena.words(), top.base);
    let from = |x: f32, y: f32| {
        Some(Ray {
            origin: [x, y, 0.0],
            direction: [0.0, 0.0, 20.0],
            t_min: 0.0,
            t_max: 1.0,
        })
    };
    let rays = [from(-0.8, -0.8), from(1.9, -0.2)];
    let wave = Wave {
        first_hit: true,
        ..Wave::default()
    };
    let out = walked(words, top, &rays, &[], &wave, &mut |_, _| true);
    let named = |lane: usize| out.hits[lane].map(|hit| met(words, top, &hit).instance);
    assert_eq!(([0, 1].map(named), out.turns), ([Some(0), Some(1)], 3));
    // The root, the near instance and the far one for both; a turn each below the near
    // instance, where the first is done; and one for the second below the far one.
    assert_eq!(out.counts, [4, 5]);
}

#[test]
fn a_pair_that_waits_is_walked_by_the_lanes_still_in_the_wave_alone() {
    let (arena, top, ..) = two_behind();
    let (words, top) = (arena.words(), top.base);
    let rays = five_lanes();
    let wave = Wave {
        first_hit: true,
        ..Wave::default()
    };
    let out = walked(words, top, &rays, &[], &wave, &mut |_, _| true);
    let named = |hit: &Option<Hit>| hit.map(|hit| met(words, top, &hit).instance);
    // The lane through both is done in the far instance and has left the wave when the near
    // one's pair is taken for the lane through it alone: it keeps the far triangle, though the
    // near one lies before it.
    assert_eq!(
        out.hits.iter().map(named).collect::<Vec<_>>(),
        [Some(1), Some(0), Some(1), Some(1), Some(1)]
    );
    // The root and both instances are the wave's turns, which every lane counts, and each lane
    // takes one turn of its own below the one instance it walks.
    assert_eq!((out.turns, out.counts), (3, vec![4; 5]));
}

#[test]
fn a_first_hit_is_the_first_a_lane_was_offered_in_the_order_it_walked() {
    let (arena, top, _, world) = two_behind();
    let (words, top) = (arena.words(), top.base);
    let rays = five_lanes();
    let (out, offered) = first_offered(words, top, &rays, &Wave::default());
    let answers: Vec<_> = (rays.iter().flatten())
        .map(|ray| every_triangle(&world, ray, 0xFF, Cull::None))
        .collect();
    for (lane, answer) in answers.iter().enumerate() {
        let found = out.hits[lane].map(|hit| met(words, top, &hit));
        assert_eq!(
            answer.first(found, &offered[lane]),
            Verdict::Same,
            "lane {lane} found {found:?}, offered {:?}",
            offered[lane]
        );
    }
    // The check itself, on the lane through both: the near triangle is as sure a hit as the far
    // one, and is no first hit of a walk that was offered the far one before it.
    let [near, far] = answers[0].sure[..] else {
        panic!("two triangles: {:?}", answers[0].sure)
    };
    assert_eq!((near.instance, far.instance), (0, 1));
    assert_eq!(answers[0].first(Some(near), &[far, near]), Verdict::Differs);
    assert_eq!(answers[0].first(Some(far), &[far, near]), Verdict::Same);
    assert_eq!(answers[0].first(None, &[]), Verdict::Differs);
}

#[test]
fn the_key_of_a_box_s_child_is_its_lanes_counts_in_base_32_cut_to_24_bits() {
    assert_eq!(vote([1, 2, 3, 4]), ((32 + 2) * 32 + 3) * 32 + 4);
    // A count of 32 carries into the place before it.
    assert_eq!(vote([0, 32, 0, 0]), vote([1, 0, 0, 0]));
    // What passes 24 bits is lost before the next digit is put on.
    assert_eq!(vote([1 << 19, 0, 0, 0]), 0);
    assert_eq!(vote([(1 << 19) + 1, 0, 0, 5]), 32 * 32 * 32 + 5);
}

#[test]
fn a_lane_s_own_stack_is_eight_entries_and_a_pop_past_them_climbs() {
    // Five levels of boxes whose four children a ray along z all meets leave three children
    // waiting a level: more than a lane's eight entries and fewer than sixteen.
    let count = 1024;
    let (arena, top) = once(&stack(count, 0.25), 0, 0);
    let (words, top) = (arena.words(), top.base);
    let rays = [Some(along(-1.0, 300.0)), Some(along(300.0, -1.0))];
    let run = |ring: u32| {
        let wave = Wave {
            ring,
            ..Wave::default()
        };
        let mut seen = [Vec::new(), Vec::new()];
        let out = walked(words, top, &rays, &[], &wave, &mut |lane, hit| {
            seen[lane].push(met(words, top, hit).primitive);
            false
        });
        for list in &mut seen {
            list.sort_unstable();
        }
        (seen, out.ring_deepest, out.climbs)
    };
    assert_eq!(Wave::default().ring, 8);
    let every: Vec<u32> = (0..count as u32).collect();
    let (seen, deepest, climbs) = run(16);
    assert_eq!(seen, [every.clone(), every.clone()]);
    assert!(
        deepest > 8 && deepest <= 16 && climbs == 0,
        "{deepest} entries, {climbs} climbs"
    );
    // At the real size the entries a lane lost are found again by the parent table, each once.
    let (seen, _, climbs) = run(8);
    assert_eq!(seen, [every.clone(), every]);
    assert!(climbs > 0);
}

#[test]
fn the_near_end_is_nought_or_the_ray_s_own() {
    // Two triangles at a quarter and at half of the ray, and a ray that states a near end
    // between them.
    let (arena, top) = once(&stack(2, 4.0), 0, 0);
    let (words, top) = (arena.words(), top.base);
    let ray = Ray {
        t_min: 0.4,
        ..along(-4.0, 12.0)
    };
    let hit = |near: Near| {
        let wave = Wave {
            near,
            ..Wave::default()
        };
        let out = walked(words, top, &[Some(ray)], &[], &wave, &mut |_, _| true);
        let hit = out.hits[0].expect("a hit");
        (met(words, top, &hit).primitive, (hit.t * 1024.0).round())
    };
    assert_eq!(Wave::default().near, Near::Nought);
    assert_eq!(hit(Near::Nought), (0, 256.0));
    assert_eq!(hit(Near::Ray), (1, 512.0));
}

#[test]
fn ray_flags_decide_what_a_triangle_s_opacity_and_face_are_worth() {
    // A quad of two triangles in one node, facing +z, and a ray down onto its front. What a
    // walk gives of it: whether it hit, how often its candidate code was asked, and the lane's
    // turns below the instance, which are two where the node's second triangle is visited.
    let quad = |opaque: bool| {
        let corners = [
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0f32],
        ];
        [[0, 1, 2], [1, 3, 2]].map(|at: [usize; 3]| Triangle {
            corners: at.map(|corner| corners[corner]),
            geometry: 0,
            primitive: 0,
            opaque,
        })
    };
    let ray = Ray {
        origin: [0.25, 0.25, 1.0],
        direction: [0.0, 0.0, -2.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let given = |instance: u32, opaque: bool, flags: u32, commits: bool| {
        let (arena, top) = once(&quad(opaque), instance, 0);
        let (words, top) = (arena.words(), top.base);
        // Compiled into the walk, and as the lane's own at run time: the same either way.
        let both = [(flags, 0), (0, flags)].map(|(compiled, own)| {
            let wave = Wave {
                flags: compiled,
                ..Wave::default()
            };
            let mut asked = 0;
            let out = walked(words, top, &[Some(ray)], &[own], &wave, &mut |_, _| {
                asked += 1;
                commits
            });
            (out.hits[0].is_some(), asked, out.counts[0] - out.turns)
        });
        assert_eq!(
            both[0], both[1],
            "flags {flags:#x} on instance {instance:#x}"
        );
        both[0]
    };
    let (no_cull, flip) = (FLAG_NO_CULL, FLAG_FLIP_WINDING);
    let (opaque, none) = (FLAG_FORCE_OPAQUE, FLAG_FORCE_NON_OPAQUE);
    // The geometry's bit, then the instance's force flags over it, then the ray's over those.
    assert_eq!(given(0, true, 0, false), (true, 0, 2));
    assert_eq!(given(0, false, 0, false), (false, 1, 2));
    assert_eq!(given(0, false, 0, true), (true, 1, 2));
    assert_eq!(given(opaque, false, 0, false), (true, 0, 2));
    assert_eq!(given(none, true, 0, false), (false, 1, 2));
    let unforced = RAY_FORCE_NON_OPAQUE;
    assert_eq!(given(opaque, false, unforced, false), (false, 1, 2));
    assert_eq!(given(none, true, RAY_FORCE_OPAQUE, false), (true, 0, 2));
    // Both of the ray's force bits make nothing opaque.
    let both = RAY_FORCE_OPAQUE | RAY_FORCE_NON_OPAQUE;
    assert_eq!(given(0, true, both, true), (true, 1, 2));
    // What the flags pass over is not looked at, nor is the second triangle of its node.
    assert_eq!(given(0, true, RAY_SKIP_OPAQUE, true), (false, 0, 1));
    assert_eq!(given(0, false, RAY_SKIP_NON_OPAQUE, true), (false, 0, 1));
    assert_eq!(given(0, false, RAY_SKIP_OPAQUE, true), (true, 1, 2));
    assert_eq!(given(0, true, RAY_SKIP_NON_OPAQUE, true), (true, 0, 2));
    assert_eq!(given(opaque, false, RAY_SKIP_OPAQUE, true), (false, 0, 1));
    let forced = RAY_FORCE_NON_OPAQUE | RAY_SKIP_NON_OPAQUE;
    assert_eq!(given(0, true, forced, true), (false, 0, 1));
    // The ray meets the front. A face is left out before the candidate code is asked.
    assert_eq!(given(0, true, RAY_CULL_BACK, true), (true, 0, 2));
    assert_eq!(given(0, true, RAY_CULL_FRONT, true), (false, 0, 2));
    assert_eq!(given(0, false, RAY_CULL_FRONT, true), (false, 0, 2));
    let either = RAY_CULL_BACK | RAY_CULL_FRONT;
    assert_eq!(given(0, true, either, true), (false, 0, 2));
    assert_eq!(given(no_cull, true, RAY_CULL_FRONT, true), (true, 0, 2));
    assert_eq!(given(flip, true, RAY_CULL_FRONT, true), (true, 0, 2));
    assert_eq!(given(flip, true, RAY_CULL_BACK, true), (false, 0, 2));
}

#[test]
fn candidate_code_that_reads_a_table_commits_by_the_record_of_a_hit_s_geometry() {
    // Six triangles a unit apart, none opaque, each a geometry of its own, under an instance
    // that adds 5 to a geometry's number: a record a geometry, and a triangle is committed
    // where its record's first dword has none of the bits the code refuses.
    let triangles: Vec<Triangle> = (stack(6, 1.0).into_iter().zip(0..))
        .map(|(triangle, geometry)| Triangle {
            geometry,
            ..triangle
        })
        .collect();
    let (arena, top) = once(&triangles, 0, 5);
    let (words, top) = (arena.words(), top.base);
    let refused = 0x3_DA00;
    let mut table = [0xFFFF_FFFFu32; 16];
    // The records of geometries 0 to 5: the two nearest and the farthest are refused.
    table[5..11].copy_from_slice(&[0x200, 0x4000, 0x25FF, 0x1, 0, 0x800]);
    let ray = along(-1.0, 7.0);
    let mut records = Vec::new();
    let mut commits = |hit: &Hit| {
        let record = record_of(words, top, hit, 24);
        records.push(record);
        table[record as usize] & refused == 0
    };
    let mut lanes = |first_hit: bool| {
        let wave = Wave {
            first_hit,
            ..Wave::default()
        };
        let out = walked(words, top, &[Some(ray)], &[], &wave, &mut |_, hit| {
            commits(hit)
        });
        let walk = Walk {
            first_hit,
            ..Walk::default()
        };
        let alone = walk_lane(words, top, &ray, &walk, &mut commits);
        let named = |hit: Option<Hit>| hit.map(|hit| met(words, top, &hit).geometry);
        (named(out.hits[0]), named(alone.hit), alone.hit)
    };
    // The nearest triangle whose record refuses nothing, from either walk.
    let (wave, alone, hit) = lanes(false);
    assert_eq!((wave, alone), (Some(2), Some(2)));
    let first = lanes(true);
    assert!(
        first.0 == first.1 && [2, 3, 4].map(Some).contains(&first.0),
        "{first:?}"
    );
    records.sort_unstable();
    records.dedup();
    assert!(
        records.len() > 3 && records.iter().all(|record| (5..11).contains(record)),
        "{records:?}"
    );
    // A contribution is as many bits as the code keeps of it.
    let hit = hit.expect("a hit");
    assert_eq!(record_of(words, top, &hit, 24), 5 + 2);
    assert_eq!(record_of(words, top, &hit, 2), 1 + 2);
}

#[test]
fn a_distance_is_nought_with_nought_but_in_the_walk_that_fuses_nothing() {
    // An instruction that answers a triangle with a numerator of nought below nought.
    let opaque = Triangle {
        opaque: true,
        ..stack(1, 1.0)[0]
    };
    let (arena, top) = once(&[opaque], 0, 0);
    let (words, top) = (arena.words(), top.base);
    let rays = [Some(along(-1.0, 1.0))];
    let answer = |_: &Probe| [-0.0f32, 1.0, 0.25, 0.25].map(f32::to_bits);
    let bits = |form: Form| {
        let wave = Wave {
            form,
            ..Wave::default()
        };
        let mut code = Code {
            body: &mut |_, probe| answer(probe),
            candidate: &mut |_, _| true,
        };
        let out = walk_wave(words, top, &rays, &[], &wave, &mut code);
        out.hits[0].map(|hit| hit.t.to_bits())
    };
    assert_eq!(bits(Form::Wave), Some(0));
    assert_eq!(bits(Form::Plain), Some((-0.0f32).to_bits()));
    // And the per-lane walk's is nought with nought too.
    let (ray, walk) = (along(-1.0, 1.0), Walk::default());
    let mut body = answer;
    let out = walk_lane_with(words, top, &ray, &walk, &mut body, &mut |_| true);
    assert_eq!(out.hit.map(|hit| hit.t.to_bits()), Some(0));
}

#[test]
fn the_walk_that_fuses_nothing_finds_what_the_fused_one_finds_but_for_a_distance_s_last_bits() {
    let scene = scene();
    let (words, top) = (scene.arena.words(), scene.top.base);
    let mut sequence = Sequence(909);
    let (mut lanes, mut hits, mut apart, mut worst) = (0, 0, 0, 0.0f32);
    for number in 0..2 {
        let rays = rays_of(&scene, &mut sequence, 32, 2 * number);
        let run = |form: Form| {
            let wave = Wave {
                mask: RAY_MASK,
                form,
                ..Wave::default()
            };
            walked(words, top, &rays, &[], &wave, &mut |_, _| true)
        };
        let (fused, plain) = (run(Form::Wave), run(Form::Plain));
        for (fused, plain) in fused.hits.iter().zip(&plain.hits) {
            lanes += 1;
            assert_eq!(fused.is_some(), plain.is_some());
            let (Some(fused), Some(plain)) = (fused, plain) else {
                continue;
            };
            hits += 1;
            assert_eq!((fused.leaf, fused.instance), (plain.leaf, plain.instance));
            apart += usize::from(fused.t != plain.t);
            worst = worst.max((fused.t - plain.t).abs() / fused.t);
        }
    }
    println!(
        "{lanes} lanes, {hits} hit, {apart} of them at another distance, by {worst:e} at most"
    );
    assert!(
        hits > 10 && apart > 0 && worst < 1e-5,
        "{hits} hits, {apart} apart, {worst:e}"
    );
}

/// The counts U7's prototype ran, which a default run has no time for.
pub mod slow {
    #[test]
    fn sixty_four_waves_of_sixty_four_lanes_find_what_every_triangle_finds() {
        let tally = super::tally(64, 64);
        assert!(tally.rays > 3900 && super::holds(&tally), "{tally:?}");
        assert!(tally.sooner > 0, "no first hit lies behind a closest one");
    }

    #[test]
    fn a_hundred_and_twenty_eight_waves_of_thirty_two_lanes_find_what_every_triangle_finds() {
        let tally = super::tally(32, 128);
        assert!(tally.rays > 3900 && super::holds(&tally), "{tally:?}");
        assert!(tally.sooner > 0, "no first hit lies behind a closest one");
    }
}
