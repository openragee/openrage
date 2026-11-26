//! A walk of the user's own build that its wave takes together, lowered alone with the wave
//! kept whole and run here against the reference walk of a wave, lane for lane: whether it hit,
//! the distance's bits, the leaf, the turns the lane counted, and the instance its leaf and
//! distance name.
//!
//! The files are those `OPENRAGE_RDNA_WALKS` names that state `form = wave`; what each walk came
//! to is written beside its file, with `.out` after its name.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use or_io::gta6::accel::layout::{
    pointer, BOX_BOUNDS, BOX_BOUNDS_STRIDE, BOX_CHILDREN, FLAG_NO_CULL, HEADER_BYTES,
    HEADER_FIRST_NODE, HEADER_ROOT, INSTANCE_BOTTOM, INSTANCE_CONTRIBUTION_FLAGS,
    INSTANCE_IDENTITY, INSTANCE_ID_MASK, INSTANCE_INDEX, INSTANCE_OBJECT_TO_WORLD, INSTANCE_PARENT,
    INSTANCE_ROOT, INSTANCE_TRANSLATION, INVALID, TYPE_BOX32, TYPE_INSTANCE,
};
use or_io::gta6::accel::{
    candidates_as, walk_wave, word, Arena, Cull, Form, Instance, Ray, Triangle, Wave, WaveOutcome,
};

use super::interp;
use super::pass_rt_walk::{deep, number, placed, Changed, Spec};
use super::pass_rt_wave::{barriers, lane_into, s_mov, wave_of_32, whole, EXEC_LO};
use super::*;

/// The walk a file states, lowered alone.
struct Lowering {
    module: naga::Module,
    registers: Registers,
    threads: [u32; 3],
    shared: u32,
    wave: Wave,
}

fn lower(spec: &Spec) -> Lowering {
    let program = spec.program();
    let left = ["leaf", "t", "count"].map(|what| spec.text(what).to_owned());
    let options = Options {
        arena: true,
        region: Some(Region {
            first: spec.number("first"),
            exit: spec.number("exit"),
            left: left.to_vec(),
        }),
        ..Options::default()
    };
    let lowered = translate_with(&program, &options).expect("the walk lowers");
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&lowered.module)
        .expect("a module the validator takes");
    assert!(!lowered.subgroups, "the walk asks for a device's subgroup");
    assert!(
        (lowered.module.global_variables.iter())
            .any(|(_, global)| global.name.as_deref() == Some("wave_cells")),
        "the walk's wave is not kept whole"
    );
    assert!(
        lowered.approximations.is_empty(),
        "{:?}",
        lowered.approximations
    );
    let launch = program.launch.expect("a compute program");
    let threads = launch.threads.map(|n| n.max(1));
    Lowering {
        module: lowered.module,
        registers: lowered.region.expect("a walk's registers"),
        threads,
        shared: launch.shared_dwords,
        wave: Wave {
            lanes: threads.iter().product(),
            ring: (spec.number("ring") + 4) / spec.number("step"),
            stack: spec.number("pairs"),
            mask: spec.number("mask") as u8,
            cull: match spec.text("cull") {
                "none" => Cull::None,
                "back" => Cull::Back,
                "front" => Cull::Front,
                other => panic!("{other} is no culling"),
            },
            first_hit: spec.text("first_hit") == "true",
            bound: spec.number("bound"),
        },
    }
}

/// What the lowered walk left a lane, for one wave of rays over the top level at `top` of the
/// arena `words`: the leaf, the distance's bits and the count. With them the barriers a thread
/// came to.
fn run(
    spec: &Spec,
    lowering: &Lowering,
    words: &[u32],
    top: u32,
    rays: &[Option<Ray>],
) -> (Vec<[u32; 3]>, u64) {
    let lanes = lowering.wave.lanes as usize;
    let given = &lowering.registers.given;
    let slots = given.len();
    let stated: BTreeMap<&str, &str> = spec.pairs("given").into_iter().collect();
    let tracing = (rays.iter().enumerate())
        .filter(|(_, ray)| ray.is_some())
        .fold(0u32, |set, (lane, _)| set | 1 << lane);
    let mut rows = vec![0u32; lanes * slots];
    for lane in 0..lanes {
        let ray = rays.get(lane).copied().flatten();
        let part = |of: fn(&Ray) -> f32| ray.as_ref().map_or(0, |ray| of(ray).to_bits());
        for (slot, register) in given.iter().enumerate() {
            let what = stated.get(register.as_str()).copied().unwrap_or("zero");
            rows[lane * slots + slot] = match what {
                "zero" => 0,
                "root" => word(words, top, HEADER_ROOT),
                "top" => top >> 8,
                "lanes" => tracing,
                "lane" => lane as u32,
                "far" => part(|ray| ray.t_max),
                "ox" => part(|ray| ray.origin[0]),
                "oy" => part(|ray| ray.origin[1]),
                "oz" => part(|ray| ray.origin[2]),
                "dx" => part(|ray| ray.direction[0]),
                "dy" => part(|ray| ray.direction[1]),
                "dz" => part(|ray| ray.direction[2]),
                "ix" => part(|ray| 1.0 / ray.direction[0]),
                "iy" => part(|ray| 1.0 / ray.direction[1]),
                "iz" => part(|ray| 1.0 / ray.direction[2]),
                other => number(other).unwrap_or_else(|| panic!("{other} is no value")),
            };
        }
    }
    let shared = vec![0u32; lowering.shared as usize];
    let left = vec![0u32; lanes * 3];
    let buffers = [
        ("arena", words),
        ("registers_given", &rows[..]),
        ("registers_left", &left[..]),
        ("shared", &shared[..]),
    ];
    let (threads, after) =
        interp::run_bound(&lowering.module, lowering.threads, &|_, _| None, &buffers);
    let got = (0..lanes)
        .map(|lane| std::array::from_fn(|k| after[2][lane * 3 + k]))
        .collect();
    (got, threads[0].barriers)
}

/// What the reference walk of a wave says each lane is left with.
fn expected(want: &WaveOutcome, rays: &[Option<Ray>]) -> Vec<[u32; 3]> {
    (rays.iter().enumerate())
        .map(|(lane, ray)| match (ray, want.hits[lane]) {
            (Some(_), Some(hit)) => [hit.leaf, hit.t.to_bits(), want.counts[lane]],
            (Some(ray), None) => [0, ray.t_max.to_bits(), want.counts[lane]],
            // A lane that traces nothing still runs: it counts the wave's turns in the top
            // level, which every lane that runs counts, and no other.
            (None, _) => [0, 0, want.turns],
        })
        .collect()
}

/// The instances whose triangle at `leaf` the line of `ray` meets at the distance of bits
/// `t`, of every instance of the structure whatever its mask. A walk a wave takes together
/// keeps the instance a lane hit in no register, so this is what its leaf and its distance say
/// of it: one instance where the two name it, and more where they cannot.
fn instances_of((words, top): (&[u32], u32), ray: &Ray, leaf: u32, t: u32) -> Vec<u32> {
    (candidates_as(words, top, ray, 0xFF, Form::Wave).iter())
        .filter(|found| {
            // The walk's own quotient: a reciprocal, then a product.
            let [numerator, denominator, ..] = found.result.map(f32::from_bits);
            found.leaf == leaf && ((1.0 / denominator) * numerator).to_bits() == t
        })
        .map(|found| found.instance)
        .collect()
}

/// What a walk's lanes came to over some waves.
#[derive(Debug, Default)]
struct Tally {
    lanes: u32,
    hits: u32,
    /// The lanes whose every field is the reference's and whose leaf and distance name the
    /// instance the reference hit and no other; those whose leaf and distance are the
    /// reference's; and those that name its instance.
    equal: u32,
    same_hit: u32,
    same_instance: u32,
    /// The wave's turns in the top level, each lane's own turns summed, the greatest count of
    /// a lane summed over the waves, and the barriers one thread came to.
    turns: u64,
    counted: u64,
    greatest: u64,
    barriers: u64,
    deepest: u32,
    dropped: u32,
    climbs: u32,
    differing: Vec<(usize, [u32; 3], [u32; 3])>,
}

/// Runs the lowered walk over `rays`, a wave of them at a time, and holds each lane to what
/// the reference walk of a wave of `wave` comes to.
fn compare(
    spec: &Spec,
    lowering: &Lowering,
    (words, top): (&[u32], u32),
    rays: &[Option<Ray>],
    wave: &Wave,
) -> Tally {
    let mut tally = Tally::default();
    for rays in rays.chunks(lowering.wave.lanes as usize) {
        let want = walk_wave(words, top, rays, wave, &mut |_, probe| {
            probe.software(words)
        });
        let wanted = expected(&want, rays);
        let (got, barriers) = run(spec, lowering, words, top, rays);
        for (lane, wanted) in wanted.iter().enumerate() {
            // A leaf of nought is no hit, as the program itself reads it.
            let named = match (rays[lane], got[lane]) {
                (Some(ray), [leaf, t, _]) if leaf != 0 => instances_of((words, top), &ray, leaf, t),
                _ => Vec::new(),
            };
            let instance: Vec<u32> = want.hits[lane].iter().map(|hit| hit.instance).collect();
            tally.lanes += 1;
            tally.hits += u32::from(want.hits[lane].is_some());
            tally.equal += u32::from(got[lane] == *wanted && named == instance);
            tally.same_hit += u32::from(got[lane][..2] == wanted[..2]);
            tally.same_instance += u32::from(named == instance);
            if got[lane] != *wanted && tally.differing.len() < 6 {
                tally.differing.push((lane, got[lane], *wanted));
            }
        }
        tally.turns += u64::from(want.turns);
        tally.counted += want.counts.iter().map(|&n| u64::from(n)).sum::<u64>();
        tally.greatest += want.counts.iter().map(|&n| u64::from(n)).max().unwrap_or(0);
        tally.barriers += barriers;
        tally.deepest = tally.deepest.max(want.deepest);
        tally.dropped += want.dropped;
        tally.climbs += want.climbs;
    }
    tally
}

/// A top level written node by node over a bottom level of one triangle: a chain of `boxes`
/// boxes along z, each of three instances and the next box, the last of three instances alone.
/// A box states each child over the whole footprint, so every ray along z meets every child,
/// the next box before the instances, and three children wait at every box. Each instance's
/// triangle lies in a cell of its own, and no face of it is culled. With the arena's dwords:
/// the top level's address, and where each instance is.
fn chain(boxes: usize) -> (Vec<u32>, u32, Vec<[f32; 3]>) {
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners: [[0.0; 3], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let mut words = arena.words().to_vec();
    let top = words.len() as u32 * 4;
    // Two units a node after the header's two; box `k` first, then its three instances.
    let unit = |k: usize, child: usize| (2 + 8 * k + 2 * child) as u32;
    let mut tree = vec![0u32; 16 * unit(boxes, 0) as usize];
    let mut put =
        |unit: u32, field: u32, value: u32| tree[(unit * 16 + field / 4) as usize] = value;
    // Child `i` of the box at `unit`: `to`, stated over the whole footprint between two heights.
    let child = |put: &mut dyn FnMut(u32, u32, u32), unit: u32, i: u32, to: u32, z: [f32; 2]| {
        put(unit, BOX_CHILDREN + 4 * i, to);
        let bounds = [-100.0, -100.0, z[0], 100.0, 100.0, z[1]];
        for (k, x) in bounds.into_iter().enumerate() {
            let field = BOX_BOUNDS + BOX_BOUNDS_STRIDE * i + 4 * k as u32;
            put(unit, field, x.to_bits());
        }
    };
    put(0, HEADER_ROOT, pointer(unit(0, 0), TYPE_BOX32));
    put(0, HEADER_FIRST_NODE, HEADER_BYTES);
    let mut places = Vec::new();
    let end = 10.0 * boxes as f32;
    for k in 0..boxes {
        let (me, level) = (unit(k, 0), 10.0 * k as f32);
        match k + 1 < boxes {
            true => {
                let next = pointer(unit(k + 1, 0), TYPE_BOX32);
                child(&mut put, me, 3, next, [level + 1.0, end]);
            }
            false => child(&mut put, me, 3, INVALID, [f32::NAN; 2]),
        }
        for i in 0..3 {
            let number = places.len();
            let at = [
                2.0 * (number % 12) as f32,
                2.0 * (number / 12) as f32,
                level + 3.0 + 2.0 * i as f32,
            ];
            let node = unit(k, 1 + i);
            let slab = [at[2] - 0.5, at[2] + 0.5];
            child(&mut put, me, i as u32, pointer(node, TYPE_INSTANCE), slab);
            // The instance as it was made, and the world as the instance sees it: both moves.
            for row in 0..3u32 {
                let one = INSTANCE_OBJECT_TO_WORLD + 16 * row + 4 * row;
                put(node, one, 1f32.to_bits());
                let moved = INSTANCE_OBJECT_TO_WORLD + 16 * row + 12;
                put(node, moved, at[row as usize].to_bits());
                let back = INSTANCE_TRANSLATION + 4 * row;
                put(node, back, (-at[row as usize]).to_bits());
            }
            put(node, INSTANCE_ID_MASK, 0xFF << 24);
            put(node, INSTANCE_CONTRIBUTION_FLAGS, FLAG_NO_CULL << 24);
            put(node, INSTANCE_ROOT, bottom.root | INSTANCE_IDENTITY);
            put(node, INSTANCE_BOTTOM, bottom.base >> 8);
            put(node, INSTANCE_INDEX, number as u32);
            put(node, INSTANCE_PARENT, pointer(me, TYPE_BOX32));
            places.push(at);
        }
    }
    words.extend(tree);
    (words, top, places)
}

/// The two chains of [`chain`] that hold a wave's stack of `pairs` pairs to its bound, each with
/// the pairs it leaves waiting: the longest whose pairs all fit, and the one a box longer, whose
/// last pushes are dropped. A chain leaves three pairs waiting at each box but its last, where
/// one instance is walked at once and two wait.
fn chains_at_the_bound(pairs: u32) -> [(usize, u32); 2] {
    let fits = (pairs + 1) / 3;
    [fits, fits + 1].map(|boxes| (boxes as usize, 3 * boxes - 1))
}

/// A wave of rays up the z axis, lane `k` at the instance `aims[k]` of a chain alone.
fn aimed(places: &[[f32; 3]], aims: &[usize], reach: f32) -> Vec<Option<Ray>> {
    (aims.iter())
        .map(|aim| {
            let at = places[*aim];
            Some(Ray {
                origin: [at[0] + 0.125, at[1] + 0.125, -5.0],
                direction: [0.0, 0.0, reach],
                t_min: 0.0,
                t_max: 1.0,
            })
        })
        .collect()
}

#[test]
fn a_walk_a_wave_takes_together_lowered_alone_equals_the_reference_walk_lane_for_lane() {
    let Some(files) = std::env::var_os("OPENRAGE_RDNA_WALKS") else {
        or_test_support::skip_install(
            "a wave's walk of the build against the reference walk",
            "OPENRAGE_RDNA_WALKS names no file of a walk",
        );
        return;
    };
    for file in std::env::split_paths(&files) {
        let spec = Spec::read(&file);
        if !spec.of_a_wave() {
            continue;
        }
        let lowering = lower(&spec);
        let lanes = lowering.wave.lanes as usize;
        let mut report = String::new();
        let scenes = [
            placed(
                "instances a quarter turn apart",
                12345,
                true,
                Changed::Nothing,
            ),
            placed(
                "instances turned by any angle",
                777,
                false,
                Changed::Nothing,
            ),
            deep(),
        ];
        for scene in &scenes {
            // Every fourth wave leaves a quarter of its lanes out of the trace.
            let rays: Vec<Option<Ray>> = (scene.rays.iter().enumerate())
                .map(|(k, ray)| ((k / lanes) % 4 != 3 || k % 4 != 1).then_some(*ray))
                .collect();
            let world = (scene.arena.words(), scene.top.base);
            let tally = compare(&spec, &lowering, world, &rays, &lowering.wave);
            let _ = writeln!(report, "{}: {tally:?}", scene.name);
            assert!(tally.hits > 0 && tally.hits < tally.lanes, "{tally:?}");
            assert_eq!(tally.equal, tally.lanes, "{}: {tally:?}", scene.name);
        }

        // The wave's stack at its bound, by the two chains its own size gives: the longest
        // that leaves no more pairs waiting than the stack holds, and one a box longer, whose
        // last pushes are dropped and their instances never walked. The last lanes aim at the
        // last instances, the dropped ones among them.
        let pairs = lowering.wave.stack / 2;
        for (boxes, waiting) in chains_at_the_bound(pairs) {
            let (deepest, dropped) = (waiting.min(pairs), waiting.saturating_sub(pairs));
            let (words, top, places) = chain(boxes);
            let aims: Vec<usize> = (places.len().saturating_sub(lanes)..places.len()).collect();
            let rays = aimed(&places, &aims, 10.0 * boxes as f32 + 10.0);
            let tally = compare(&spec, &lowering, (&words, top), &rays, &lowering.wave);
            let _ = writeln!(report, "a chain of {boxes} boxes: {tally:?}");
            assert_eq!(
                (tally.deepest, tally.dropped),
                (deepest, dropped),
                "{tally:?}"
            );
            assert_eq!(tally.hits, tally.lanes - dropped, "{tally:?}");
            assert_eq!(tally.equal, tally.lanes, "{boxes} boxes: {tally:?}");
            // The reference with a stack of one pair fewer than the chain fills loses one
            // instance more: one lane finds another hit than the walk's, and no lane counts
            // the walk's turns.
            let fewer = Wave {
                stack: 2 * (deepest - 1),
                ..lowering.wave
            };
            let short = compare(&spec, &lowering, (&words, top), &rays, &fewer);
            let _ = writeln!(report, "a chain of {boxes} boxes, a pair fewer: {short:?}");
            assert_eq!(
                (short.dropped, short.same_hit, short.equal),
                (dropped + 1, short.lanes - 1, 0),
                "{short:?}"
            );
        }

        // Controls of the reference that must differ: another instance mask, no face culled,
        // and a lane's own stack of fewer entries on the scene that overruns it.
        let world = (scenes[0].arena.words(), scenes[0].top.base);
        let all: Vec<Option<Ray>> = scenes[0].rays.iter().copied().map(Some).collect();
        let deep: Vec<Option<Ray>> = scenes[2].rays.iter().copied().map(Some).collect();
        let deep_world = (scenes[2].arena.words(), scenes[2].top.base);
        for (what, world, rays, wave) in [
            (
                "every instance entered",
                world,
                &all,
                Wave {
                    mask: 0xFF,
                    ..lowering.wave
                },
            ),
            (
                "no face culled",
                world,
                &all,
                Wave {
                    cull: Cull::None,
                    ..lowering.wave
                },
            ),
            (
                "a lane's stack of half the entries",
                deep_world,
                &deep,
                Wave {
                    ring: lowering.wave.ring / 2,
                    ..lowering.wave
                },
            ),
        ] {
            let differs = compare(&spec, &lowering, world, rays, &wave);
            let _ = writeln!(report, "{what}: {differs:?}");
            assert!(differs.equal < differs.lanes, "{what}: {differs:?}");
        }
        let mut out = file.clone().into_os_string();
        out.push(".out");
        std::fs::write(&out, report).expect("the walk's figures written");
    }
}

#[test]
fn a_chain_holds_a_stack_of_any_size_to_its_bound_and_a_box_more_overruns_it() {
    // The reference walk alone, over the chains a stack's size gives: what waits is the count
    // the chain was chosen for, what does not fit is dropped, and a lane misses the instance
    // of a dropped push alone.
    for pairs in [4, 9, 32, 128] {
        for (boxes, waiting) in chains_at_the_bound(pairs) {
            let (words, top, places) = chain(boxes);
            let aims: Vec<usize> = (places.len().saturating_sub(32)..places.len()).collect();
            let rays = aimed(&places, &aims, 10.0 * boxes as f32 + 10.0);
            let wave = Wave {
                stack: 2 * pairs,
                ..Wave::default()
            };
            let out = walk_wave(&words, top, &rays, &wave, &mut |_, probe| {
                probe.software(&words)
            });
            let at = format!("{pairs} pairs, {boxes} boxes");
            let held = (waiting.min(pairs), waiting.saturating_sub(pairs));
            assert_eq!((out.deepest, out.dropped), held, "{at}");
            let hits = out.hits.iter().flatten().count();
            assert_eq!(hits, rays.len() - out.dropped as usize, "{at}");
        }
    }
    // Of the sizes above two are filled to the pair, and a box more overruns by one, two and three.
    assert_eq!(chains_at_the_bound(128), [(43, 128), (44, 131)]);
    assert_eq!(chains_at_the_bound(9), [(3, 8), (4, 11)]);
}

/// One wave of `rays` by the reference walk, with every instance entered and no face culled.
fn reference(world: (&[u32], u32), rays: &[Ray]) -> WaveOutcome {
    let rays: Vec<Option<Ray>> = rays.iter().copied().map(Some).collect();
    walk_wave(
        world.0,
        world.1,
        &rays,
        &Wave::default(),
        &mut |_, probe| probe.software(world.0),
    )
}

#[test]
fn a_hit_s_leaf_and_distance_name_its_instance_in_the_scenes_a_wave_s_walk_is_held_on() {
    // The reference walk alone. Six instances of each placed scene share a model, so a leaf
    // alone names none of them; with the distance it names the one that was hit.
    let scenes = [
        placed("a quarter turn apart", 12345, true, Changed::Nothing),
        placed("turned by any angle", 777, false, Changed::Nothing),
        deep(),
    ];
    for scene in &scenes {
        let world = (scene.arena.words(), scene.top.base);
        let mut hits = 0;
        for rays in scene.rays.chunks(32) {
            let out = reference(world, rays);
            for (ray, hit) in rays.iter().zip(&out.hits) {
                let Some(hit) = hit else { continue };
                let named = instances_of(world, ray, hit.leaf, hit.t.to_bits());
                assert_eq!(named, [hit.instance], "{}", scene.name);
                hits += 1;
            }
        }
        assert!(hits > 0, "{}", scene.name);
    }
    // Two instances of one model in one place share every leaf and every distance: the two
    // cannot say which was hit, and no single instance is named.
    let mut arena = Arena::new();
    let triangle = Triangle {
        corners: [[0.0; 3], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let bottom = arena.bottom_level(&[triangle]).expect("a bottom level");
    let twice: Vec<Instance> = (0..2)
        .map(|id| Instance {
            bottom,
            object_to_world: std::array::from_fn(|row| {
                std::array::from_fn(|column| f32::from(u8::from(row == column)))
            }),
            id,
            mask: 0xFF,
            flags: 0,
            contribution: 0,
        })
        .collect();
    let top = arena.top_level(&twice).expect("a top level");
    let world = (arena.words(), top.base);
    let ray = Ray {
        origin: [0.125, 0.125, -5.0],
        direction: [0.0, 0.0, 10.0],
        t_min: 0.0,
        t_max: 1.0,
    };
    let hit = reference(world, &[ray]).hits[0].expect("the ray meets the triangle");
    assert_eq!(hit.t, 0.5);
    assert_eq!(
        instances_of(world, &ray, hit.leaf, hit.t.to_bits()).len(),
        2
    );
}

/// Runs a module of two lanes one lane at a time, each from a meeting of the group to the next,
/// in both orders: what each lane left in `v22`.
fn two_lanes_in_turn(module: &naga::Module) -> Vec<[u32; 2]> {
    let orders: [[u32; 2]; 2] = [[0, 1], [1, 0]];
    (orders.iter())
        .map(|order| {
            let (ran, _) = interp::run_in_order(module, [2, 1, 1], &|_, _| None, &[], Some(order));
            [ran[0].reg("v22"), ran[1].reg("v22")]
        })
        .collect()
}

#[test]
fn a_lane_loads_what_another_stored_in_memory_of_the_group_that_is_no_stack() {
    // Two lanes. The first alone stores 60 at byte 8 of the group's memory, and both load it:
    // nothing of the program stands between the store and the other lane's load.
    let mut words = lane_into(20).to_vec();
    words.extend([
        vop1(V_MOV, 0, I_ZERO + 8),
        vop1(V_MOV, 1, I_ZERO + 60),
        s_mov(24, EXEC_LO),
        vopc(op(opcodes::VOPC, "v_cmpx_eq_u32"), I_ZERO, 20),
    ]);
    words.extend(ds("ds_write_b32", 0));
    words.push(s_mov(EXEC_LO, 24));
    words.extend(ds("ds_read_b32", 0));
    words.push(vop1(V_MOV, 22, V + 1));
    let out = whole(&wave_of_32(2, 16, words));
    assert_eq!(barriers(&out.module), 1);
    // Whichever lane runs first between two meetings, the second lane loads the first's store.
    assert_eq!(two_lanes_in_turn(&out.module), [[60, 60], [60, 60]]);
}

#[test]
fn a_store_an_update_and_a_load_of_one_dword_are_each_every_lane_s_before_the_next() {
    // Both lanes store 40 at one dword, add their own number and five to it, and load it:
    // 40 + 5 + 6, whichever lane is first at each of the three.
    let mut words = lane_into(20).to_vec();
    words.extend([vop1(V_MOV, 0, I_ZERO + 8), vop1(V_MOV, 1, I_ZERO + 40)]);
    words.extend(ds("ds_write_b32", 0));
    words.push(vop2(op(opcodes::VOP2, "v_add_u32"), 1, I_ZERO + 5, 20));
    words.extend(ds("ds_add_u32", 0));
    words.extend(ds("ds_read_b32", 0));
    words.push(vop1(V_MOV, 22, V + 1));
    let out = whole(&wave_of_32(2, 16, words));
    assert_eq!(barriers(&out.module), 3);
    assert_eq!(two_lanes_in_turn(&out.module), [[51, 51], [51, 51]]);
}
