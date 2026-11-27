//! The test of a node answered from the device's own ray query: one function of the signature
//! the arena's test has, which validates and is written by the writers that carry a query, and
//! which a walk answered through it commits what the CPU's model of the protocol commits, every
//! answer to the bit, with the device's candidates made up here in shuffled orders.

use std::collections::HashMap;

use naga::{Handle, Statement as S};
use or_io::gta6::accel::layout::QUERY_TOP;
use or_io::gta6::accel::{
    handed_over, shuffled, walk_lane, walk_lane_with, Arena, Form, Instance, Probe, Query, Ray,
    Shown, Structure, Triangle, Walk,
};

use super::interp::{answer_queries_with, Calls, Value};
use super::pass_rt::{test_words, MODES, WIDE};
use super::*;

/// A compute program of one wave whose pipeline loads `s0` and `s1`.
fn compute(words: Vec<u32>) -> Program {
    Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [8, 8, 1],
            shared_dwords: 128,
        }),
        ..program(ShaderKind::Compute, words)
    }
}

/// A program of one test of a node, with the lane mask written first where `masked`.
fn one_test(masked: bool) -> Vec<u32> {
    let mut words = Vec::new();
    if masked {
        words.push(sop1(op(opcodes::SOP1, "s_mov_b32"), 126, I_MINUS_ONE));
    }
    words.extend((0..12).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    words
}

fn lowered(words: Vec<u32>, query: bool) -> Lowered {
    let options = Options {
        arena: true,
        query,
        ..Options::default()
    };
    translate_with(&compute(words), &options).unwrap()
}

fn function<'a>(
    module: &'a naga::Module,
    name: &str,
) -> (Handle<naga::Function>, &'a naga::Function) {
    let found = (module.functions.iter()).find(|(_, f)| f.name.as_deref() == Some(name));
    found.unwrap_or_else(|| panic!("no function {name}"))
}

#[test]
fn a_test_of_a_node_that_asks_the_device_validates_and_is_written_where_a_writer_carries_a_query() {
    use naga::back::{hlsl, msl, spv};
    let module = lowered(one_test(false), true).module;
    // The device's structure is a type only a device with ray queries has.
    let without =
        Capabilities::all() - Capabilities::RAY_QUERY - Capabilities::RAY_TRACING_PIPELINE;
    assert!(Validator::new(ValidationFlags::all(), without)
        .validate(&module)
        .is_err());
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .unwrap();
    let (_, test) = function(&module, "bvh_intersect");
    let queries = count(&test.body, &|s| matches!(s, S::RayQuery { .. }));
    // Started and proceeded, and never confirmed: the walk keeps the hit in its own registers.
    assert_eq!(queries, 2);
    let confirms = |s: &S| {
        matches!(
            s,
            S::RayQuery {
                fun: naga::RayQueryFunction::ConfirmIntersection
                    | naga::RayQueryFunction::Terminate,
                ..
            }
        )
    };
    assert_eq!(count(&test.body, &confirms), 0);

    let options = spv::Options {
        lang_version: (1, 4),
        capabilities: None,
        ..Default::default()
    };
    let words = spv::write_vec(&module, &info, &options, None).unwrap();
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let named = b"SPV_KHR_ray_query";
    assert!(bytes.windows(named.len()).any(|window| window == named));

    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V6_5,
        fake_missing_bindings: true,
        ..Default::default()
    };
    let mut text = String::new();
    hlsl::Writer::new(&mut text, &options, &hlsl::PipelineOptions::default())
        .write(&module, &info, None)
        .unwrap();
    assert!(text.contains(".TraceRayInline(") && text.contains(".Proceed()"));
    assert!(!text.contains("CommitNonOpaqueTriangleHit"));

    // Metal's writer refuses a query below its language 2.4, and writes one at 3.0, whose
    // looped proceed naga 29 never ends: no device of Metal is given such a module.
    let metal = |lang_version| msl::Options {
        lang_version,
        fake_missing_bindings: true,
        ..Default::default()
    };
    let pipeline = msl::PipelineOptions::default();
    assert!(msl::write_string(&module, &info, &metal((2, 3)), &pipeline).is_err());
    assert!(msl::write_string(&module, &info, &metal((3, 0)), &pipeline).is_ok());
    // WGSL is not asked: naga 29's writer stops the process on a ray query, so such a module
    // reaches a device as naga's own.
}

#[test]
fn both_tests_of_a_node_are_one_function_of_one_signature() {
    let shape = |lowered: &Lowered| {
        let (_, test) = function(&lowered.module, "bvh_intersect");
        let types = &lowered.module.types;
        let arguments: Vec<_> = (test.arguments.iter())
            .map(|a| (a.name.clone(), types[a.ty].inner.clone()))
            .collect();
        let result = test.result.as_ref().map(|r| types[r.ty].inner.clone());
        (arguments, result)
    };
    let (arena, asked) = (
        lowered(one_test(false), false),
        lowered(one_test(false), true),
    );
    assert_eq!(shape(&arena), shape(&asked));
    let kinds = |lowered: &Lowered| -> Vec<_> {
        let kinds = lowered.resources.iter().map(|r| r.kind);
        kinds
            .filter(|kind| matches!(kind, ResourceKind::Arena | ResourceKind::DeviceStructure))
            .collect()
    };
    assert_eq!(kinds(&arena), [ResourceKind::Arena]);
    let mut both = kinds(&asked);
    both.sort_by_key(|kind| matches!(kind, ResourceKind::DeviceStructure));
    assert_eq!(both, [ResourceKind::Arena, ResourceKind::DeviceStructure]);
    // The arena's test departs from nothing; the device's distances and order are counted.
    assert!(
        arena.approximations.is_empty(),
        "{:?}",
        arena.approximations
    );
    for name in [
        "a distance from the device",
        "candidates shown nearest first",
        "a hit at zero distance",
    ] {
        assert_eq!(asked.approximations.get(name), Some(&1), "{name}");
    }
}

#[test]
fn a_lane_the_program_masks_off_asks_the_device_nothing() {
    for (masked, guarded) in [(false, 0), (true, 1)] {
        let module = lowered(one_test(masked), true).module;
        let (test, _) = function(&module, "bvh_intersect");
        let (_, main) = function(&module, "main_body");
        let call = |s: &S| matches!(s, S::Call { function, .. } if *function == test);
        assert_eq!(count(&main.body, &call), 1);
        let under_an_if = |s: &S| matches!(s, S::If { accept, .. } if accept.iter().any(call));
        assert_eq!(count(&main.body, &under_an_if), guarded, "masked {masked}");
    }
    // The arena's test is a function of its operands alone and is called by every lane.
    let module = lowered(one_test(true), false).module;
    let (test, _) = function(&module, "bvh_intersect");
    let (_, main) = function(&module, "main_body");
    let under_an_if = |s: &S| {
        let call = |s: &S| matches!(s, S::Call { function, .. } if *function == test);
        matches!(s, S::If { accept, .. } if accept.iter().any(call))
    };
    assert_eq!(count(&main.body, &under_an_if), 0);
}

/// A fixed sequence of floats in `[-size / 2, size / 2)`.
struct Sequence(u64);

impl Sequence {
    fn within(&mut self, size: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * size
    }
}

/// A sheet of `n` by `n` squares from -1 to 1, two triangles a square that share its diagonal.
fn sheet(n: usize) -> Vec<Triangle> {
    let at = |i: usize| -1.0 + 2.0 * i as f32 / n as f32;
    let mut triangles = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let corner = |dx: usize, dy: usize| [at(x + dx), at(y + dy), 0.0];
            let (a, b, c, d) = (corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1));
            for corners in [[a, b, c], [a, c, d]] {
                triangles.push(Triangle {
                    corners,
                    geometry: 0,
                    primitive: triangles.len() as u32,
                    opaque: true,
                });
            }
        }
    }
    triangles
}

/// The arena of a made-up scene: two sheets one behind another, one beside them, and two lone
/// triangles between and before them. Its words as built, its words for a query (the first box
/// stated as the root and the top level's address at [`QUERY_TOP`]), and the top level.
fn scene() -> (Vec<u32>, Vec<u32>, u32) {
    let mut arena = Arena::new();
    let sheet = arena.bottom_level(&sheet(4)).unwrap();
    let lone = Triangle {
        corners: [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let lone = arena.bottom_level(&[lone]).unwrap();
    let placed = |bottom: Structure, [x, y, z]: [f32; 3]| Instance {
        bottom,
        object_to_world: [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z]],
        id: 0,
        mask: 0xFF,
        flags: 0,
        contribution: 0,
    };
    let instances = [
        placed(sheet, [0.0, 0.0, 0.0]),
        placed(sheet, [0.5, 0.25, 2.0]),
        placed(lone, [0.25, 0.0, 1.0]),
        placed(sheet, [3.0, 0.0, 1.0]),
        placed(lone, [-0.5, 0.5, -1.0]),
    ];
    let top = arena.top_level(&instances).unwrap();
    let plain = arena.words().to_vec();
    assert!(arena.state_query_root(&top, true));
    let mut asked = arena.words().to_vec();
    asked[(QUERY_TOP / 4) as usize] = top.base;
    (plain, asked, top.base)
}

/// A candidate as naga's `RayIntersection`: kind, distance, the instance's value and number,
/// the hit-group offset, the geometry and the triangle, the weights, the facing and two
/// matrices no test reads.
fn intersection(shown: &Shown) -> Value {
    let [u, v] = shown.weights;
    Value::List(vec![
        Value::U(1),
        Value::F(shown.t),
        Value::U(0),
        Value::U(shown.instance),
        Value::U(0),
        Value::U(0),
        Value::U(shown.primitive),
        Value::List(vec![Value::F(u), Value::F(v)]),
        Value::B(shown.front),
        Value::U(0),
        Value::U(0),
    ])
}

fn floats(value: &Value) -> [f32; 3] {
    let Value::List(parts) = value else {
        panic!("{value:?} is no three floats");
    };
    std::array::from_fn(|k| f32::from_bits(parts[k].word()))
}

/// The arguments of the test of the node `probe` names, in the 32-bit form (the structure's
/// address and the node's pointer) or the 64-bit one (the node's address in eighths).
fn arguments(probe: &Probe, wide: bool) -> Vec<Value> {
    let list = |part: [f32; 3]| Value::List(part.map(Value::F).to_vec());
    let (base, node) = match wide {
        false => (probe.base >> 8, probe.node),
        true => (0, (probe.base >> 3) + probe.node),
    };
    vec![
        Value::U(base),
        Value::U(node),
        Value::F(probe.t_max),
        list(probe.origin),
        list(probe.direction),
        list(probe.inverse),
    ]
}

#[test]
fn a_walk_answered_through_the_device_s_query_is_answered_as_the_model_answers_it() {
    const RAYS: u32 = 48;
    let (plain, asked, top) = scene();
    let module = lowered(one_test(false), true).module;
    let calls = Calls::new(&module, &[("arena", &asked)]);
    // The device hands over every triangle its ray meets inside its extent, in an order of the
    // ray's own: what a device may do.
    let seed = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let (words, order) = (asked.clone(), seed.clone());
    answer_queries_with(move |ray| {
        let Value::List(parts) = ray else {
            panic!("{ray:?} is no ray");
        };
        let ray = Ray {
            origin: floats(&parts[4]),
            direction: floats(&parts[5]),
            t_min: 0.0,
            t_max: f32::from_bits(parts[3].word()),
        };
        let handed = handed_over(&words, top, &ray, Form::Lane);
        order.set(order.get().wrapping_add(1));
        (shuffled(handed, order.get()).iter())
            .map(intersection)
            .collect()
    });
    let mut sequence = Sequence(0x2545_F491_4F6C_DD1D);
    let (mut hits, mut several, mut confirmed, mut answers) = (0, 0, 0, 0);
    // One invocation's privates over every walk, in either form: a walk's start resets them.
    let (mut narrow, mut wide) = (HashMap::new(), HashMap::new());
    for number in 0..RAYS {
        let ray = Ray {
            origin: [sequence.within(3.0), sequence.within(3.0), -5.0],
            direction: [sequence.within(0.25), sequence.within(0.25), 1.0],
            t_min: 0.0,
            t_max: 20.0,
        };
        seed.set(number.wrapping_mul(7919));
        let mut handed = handed_over(&asked, top, &ray, Form::Lane);
        several += usize::from(handed.len() > 1);
        // The model is handed the same candidates in the order the module shows them: the
        // last is handed over first.
        handed.sort_by(|a, b| {
            (b.t.total_cmp(&a.t))
                .then(b.instance.cmp(&a.instance))
                .then(b.primitive.cmp(&a.primitive))
        });
        for first_hit in [false, true] {
            let walk = Walk {
                first_hit,
                ..Walk::default()
            };
            let mut model = Query::new(handed.clone());
            let mut body = |probe: &Probe| {
                let want = model.answer(&asked, top, probe);
                for (kept, form) in [(&mut narrow, false), (&mut wide, true)] {
                    let got = calls.call_kept(kept, "bvh_intersect", arguments(probe, form));
                    let mut got_words = Vec::new();
                    got.words(&mut got_words);
                    assert_eq!(
                        got_words, want,
                        "ray {number}, {probe:?}, 64-bit form {form}"
                    );
                }
                answers += 1;
                want
            };
            let queried = walk_lane_with(&asked, top, &ray, &walk, &mut body, &mut |_| true);
            let software = walk_lane(&plain, top, &ray, &walk, &mut |_| true);
            confirmed += model.counts[2];
            match first_hit {
                // The candidates carry the arena's own distances, so the closest hit is the
                // arena's to every bit.
                false => {
                    assert_eq!(queried.hit, software.hit, "ray {number}");
                    hits += usize::from(software.hit.is_some());
                }
                true => assert_eq!(
                    queried.hit.is_some(),
                    software.hit.is_some(),
                    "ray {number}"
                ),
            }
        }
    }
    assert!(hits * 4 > RAYS as usize, "{hits} hits of {RAYS}");
    assert!(
        several > 0 && confirmed > 0,
        "{several} rays of several, {confirmed} confirmed"
    );
    assert!(answers > 4 * RAYS, "{answers} answers");
}
