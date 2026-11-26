//! A module as lowered and the same module after the optimiser, each run whole on the same
//! inputs: what the entry point returns, every texel stored, the memory outside the threads
//! and the discard must be the same, bit for bit. Registers are not compared: they are what a
//! pass removes.
//!
//! Nothing of a build is in the tree. `OPENRAGE_RDNA_SAMPLE` names a folder of modules kept as
//! the translation cache keeps one, each in a file named `.module`; without it the last test
//! checks nothing and says so. `OPENRAGE_RDNA_SEEDS` is how many seeds each is run on. With a
//! folder named the test takes minutes: run it under `--profile all`, since a default run of
//! nextest ends a test at 30 s.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::path::Path;

use naga::{Expression as E, Statement as S};

use super::*;
use crate::rdna::opt;

/// The most expressions a thread of a whole run computes before it is stopped.
const MOST: u64 = 3_000_000;

/// What one thread of a whole run leaves.
#[derive(Debug, PartialEq, Eq)]
struct Left {
    /// What the entry point returned, dword by dword.
    returned: Vec<u32>,
    discarded: bool,
    /// Each texel stored, in order: the texture's number, the place, the texel.
    stored: Vec<[Vec<u32>; 3]>,
}

/// What a whole run leaves that a pass must keep.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    threads: Vec<Left>,
    /// The memory the group shares and the buffers stored to, at the end.
    memory: Vec<(String, Vec<u32>)>,
}

/// A whole run of `module` on `seed`: its outcome, how many expressions it computed and which
/// of the module's it reached. It stops, by a panic, where the interpreter does.
fn whole(module: &naga::Module, seed: u64) -> (Outcome, u64, HashSet<(usize, usize)>) {
    let ran = interp::run_whole(module, seed, MOST);
    let mut reached = HashSet::new();
    let (mut computed, mut threads) = (0, Vec::new());
    for thread in ran.threads {
        let mut returned = Vec::new();
        if let Some(result) = &thread.result {
            result.words(&mut returned);
        }
        threads.push(Left {
            returned,
            discarded: thread.killed,
            stored: thread.stores,
        });
        computed += thread.computed;
        reached.extend(thread.reached);
    }
    let memory = ran.memory;
    (Outcome { threads, memory }, computed, reached)
}

/// How two modules compared over some seeds.
#[derive(Debug, Default)]
struct Compared {
    /// The seeds both ran to their end on, with the same outcome.
    equal: u32,
    /// Of those, the seeds a thread was discarded on.
    discarded: u32,
    /// The expressions each computed.
    computed: [u64; 2],
    /// The expressions of the module as lowered that a seed reached.
    reached: HashSet<(usize, usize)>,
}

/// Where two outcomes first differ.
fn difference(lowered: &Outcome, passed: &Outcome) -> String {
    for (thread, (a, b)) in lowered.threads.iter().zip(&passed.threads).enumerate() {
        let what = match () {
            () if a.discarded != b.discarded => {
                format!("discarded {}, not {}", b.discarded, a.discarded)
            }
            () if a.returned != b.returned => {
                format!("returned {:08X?}, not {:08X?}", b.returned, a.returned)
            }
            () if a.stored != b.stored => {
                format!("stored the texels {:08X?}, not {:08X?}", b.stored, a.stored)
            }
            () => continue,
        };
        return format!("thread {thread} {what}");
    }
    for (a, b) in lowered.memory.iter().zip(&passed.memory) {
        if let Some(at) = (0..a.1.len().max(b.1.len())).find(|&at| a.1.get(at) != b.1.get(at)) {
            let (was, is) = (a.1.get(at), b.1.get(at));
            return format!("dword {at} of {} is {is:08X?}, not {was:08X?}", a.0);
        }
    }
    "not the same threads or the same memory".to_owned()
}

/// Runs `lowered` and `passed` whole on each seed; `Err` is the first seed whose outcomes
/// differ. `between` is called with a seed once the first has run it to its end.
fn compare_noting(
    lowered: &naga::Module,
    passed: &naga::Module,
    seeds: Range<u64>,
    between: &mut dyn FnMut(u64),
) -> Result<Compared, String> {
    let mut compared = Compared::default();
    for seed in seeds {
        let (first, computed, reached) = whole(lowered, seed);
        between(seed);
        let (second, fewer, _) = whole(passed, seed);
        if first != second {
            return Err(format!("seed {seed}: {}", difference(&first, &second)));
        }
        compared.equal += 1;
        compared.discarded += u32::from(first.threads.iter().any(|thread| thread.discarded));
        compared.computed[0] += computed;
        compared.computed[1] += fewer;
        compared.reached.extend(reached);
    }
    Ok(compared)
}

fn compare(
    lowered: &naga::Module,
    passed: &naga::Module,
    seeds: Range<u64>,
) -> Result<Compared, String> {
    compare_noting(lowered, passed, seeds, &mut |_| {})
}

/// The bare blocks under `block`: those that are no arm of an `if`, a `switch` or a loop.
fn bare_blocks(block: &naga::Block) -> usize {
    (block.iter())
        .map(|statement| match statement {
            S::Block(inner) => 1 + bare_blocks(inner),
            S::If { accept, reject, .. } => bare_blocks(accept) + bare_blocks(reject),
            S::Switch { cases, .. } => cases.iter().map(|case| bare_blocks(&case.body)).sum(),
            S::Loop {
                body, continuing, ..
            } => bare_blocks(body) + bare_blocks(continuing),
            _ => 0,
        })
        .sum()
}

fn functions(module: &naga::Module) -> impl Iterator<Item = &naga::Function> {
    (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function))
}

/// The bit casts of `module` that read a bit cast.
fn casts_of_casts(module: &naga::Module) -> usize {
    let bit_cast = |expression: &E| match *expression {
        E::As {
            expr,
            convert: None,
            ..
        } => Some(expr),
        _ => None,
    };
    functions(module)
        .map(|function| {
            (function.expressions.iter())
                .filter_map(|(_, expression)| bit_cast(expression))
                .filter(|&read| bit_cast(&function.expressions[read]).is_some())
                .count()
        })
        .sum()
}

/// A pixel program: `v0` and `v1` are the first attribute's first two components; three times
/// round, `v0 = v0 * v1 + v0`; the lane stays on where `limit` > `v0`; the target takes `v0`,
/// the last product and `v1`. `product` is the operation of the product.
fn looped(product: u32, limit: u32) -> naga::Module {
    let target = exp(0, [0, 2, 1, 1]);
    let words = vec![
        vintrp(1, 0, 0, 0, 1),
        vintrp(1, 1, 0, 1, 1),
        sopk(S_MOVK, 0, 3),
        vop2(product, 2, V, 1),
        vop2(V_ADD_F32, 0, V + 2, 0),
        sop2(S_ADD_I32, 0, 0, I_MINUS_ONE),
        sopc(S_CMP_LG_U32, 0, I_ZERO),
        // Back to the product at word 3.
        sopp(S_CBRANCH_SCC1, 0xFFFB),
        vopc(V_CMPX_GT_F32, limit, 0),
        target[0],
        target[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Pixel, words).module
}

const SEEDS: Range<u64> = 0..24;

#[test]
fn a_program_computes_the_same_with_its_bare_blocks_joined() {
    let module = looped(V_MUL_F32, F_ONE);
    assert!(functions(&module).any(|function| bare_blocks(&function.body) > 0));
    let mut passed = module.clone();
    let changed = opt::optimise(&mut passed);
    assert!(changed.blocks > 0, "{changed:?}");
    assert_eq!(
        functions(&passed)
            .map(|function| bare_blocks(&function.body))
            .sum::<usize>(),
        0
    );
    // The validator and the four writers take it.
    validate(&passed);
    let compared = compare(&module, &passed, SEEDS).unwrap();
    // Both ways out of the program were taken: the discard and the export.
    assert!(
        (1..compared.equal).contains(&compared.discarded),
        "{compared:?}"
    );
    // A second run finds nothing left to do.
    let again = passed.clone();
    assert_eq!(opt::optimise(&mut passed), opt::Changed::default());
    assert_eq!(format!("{passed:?}"), format!("{again:?}"));
}

/// What a module written by hand gets wrong, for the comparison to see.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// A conversion reads the signed value a chain of bit casts first read, as a fold of the
    /// chain that took a signed number for an unsigned one would leave it.
    CastOfAnotherType,
    CoordinatesSwapped,
    OtherTexture,
}

/// A compute module written by hand, of one thread. `out[0]` is `inp[0]` through a float and
/// back; `out[1]` is `inp[0]` halved as a signed number, cast to a float and on to an unsigned
/// number, and that number converted to a float; `out[2]` is the bits of a texel of the first
/// texture at `inp[1]`, cast to its own type first, and `inp[2]`.
fn by_hand(fault: Fault) -> naga::Module {
    use naga::{
        AddressSpace, GlobalVariable, ImageClass, ImageDimension, Literal, ResourceBinding, Scalar,
        ScalarKind as Kind, Span, StorageAccess, Type, TypeInner, VectorSize,
    };
    let mut module = naga::Module::default();
    let mut ty = |inner| {
        module
            .types
            .insert(Type { name: None, inner }, Span::UNDEFINED)
    };
    let uint = ty(TypeInner::Scalar(Scalar::U32));
    let pair = ty(TypeInner::Vector {
        size: VectorSize::Bi,
        scalar: Scalar::F32,
    });
    let four = ty(TypeInner::Array {
        base: uint,
        size: naga::ArraySize::Constant(std::num::NonZeroU32::new(4).unwrap()),
        stride: 4,
    });
    let texture = ty(TypeInner::Image {
        dim: ImageDimension::D2,
        arrayed: false,
        class: ImageClass::Sampled {
            kind: Kind::Float,
            multi: false,
        },
    });
    let sampler = ty(TypeInner::Sampler { comparison: false });
    let mut bound = |name: &str, space, ty| {
        let binding = module.global_variables.len() as u32;
        let variable = GlobalVariable {
            name: Some(name.to_owned()),
            space,
            binding: Some(ResourceBinding { group: 0, binding }),
            ty,
            init: None,
            memory_decorations: Default::default(),
        };
        module.global_variables.append(variable, Span::UNDEFINED)
    };
    let buffer = |access| AddressSpace::Storage { access };
    let inp = bound("inp", buffer(StorageAccess::LOAD), four);
    let out = bound(
        "out",
        buffer(StorageAccess::LOAD | StorageAccess::STORE),
        four,
    );
    let first = bound("first", AddressSpace::Handle, texture);
    let second = bound("second", AddressSpace::Handle, texture);
    let sampler = bound("sampler", AddressSpace::Handle, sampler);

    let mut function = naga::Function::default();
    let mut e = |expression| function.expressions.append(expression, Span::UNDEFINED);
    let bit_cast = |expr, kind| E::As {
        expr,
        kind,
        convert: None,
    };
    let shifted = |left, right| E::Binary {
        op: naga::BinaryOperator::ShiftRight,
        left,
        right,
    };
    // What no statement emits: the variables and the literals.
    let [inp, out] = [inp, out].map(|global| e(E::GlobalVariable(global)));
    let [first, second, sampler] =
        [first, second, sampler].map(|global| e(E::GlobalVariable(global)));
    let one = e(E::Literal(Literal::U32(1)));
    let level = e(E::Literal(Literal::F32(0.0)));
    // The seven above.
    let emitted_from = 7;
    let [a, b, c] = [0, 1, 2].map(|index| {
        let pointer = e(E::AccessIndex { base: inp, index });
        e(E::Load { pointer })
    });
    let there = e(bit_cast(a, Kind::Float));
    let back = e(bit_cast(there, Kind::Uint));
    let signed = e(bit_cast(a, Kind::Sint));
    let halved = e(shifted(signed, one));
    let as_float = e(bit_cast(halved, Kind::Float));
    let as_uint = e(bit_cast(as_float, Kind::Uint));
    let number = e(E::As {
        expr: match fault {
            Fault::CastOfAnotherType => halved,
            _ => as_uint,
        },
        kind: Kind::Float,
        convert: Some(4),
    });
    let number = e(bit_cast(number, Kind::Uint));
    let own = e(bit_cast(b, Kind::Uint));
    let [u, v] = [own, c].map(|word| e(bit_cast(word, Kind::Float)));
    let coordinate = e(E::Compose {
        ty: pair,
        components: match fault {
            Fault::CoordinatesSwapped => vec![v, u],
            _ => vec![u, v],
        },
    });
    let texel = e(E::ImageSample {
        image: match fault {
            Fault::OtherTexture => second,
            _ => first,
        },
        sampler,
        gather: None,
        coordinate,
        array_index: None,
        offset: None,
        level: naga::SampleLevel::Exact(level),
        depth_ref: None,
        clamp_to_edge: false,
    });
    let red = e(E::AccessIndex {
        base: texel,
        index: 0,
    });
    let red = e(bit_cast(red, Kind::Uint));
    let places = [0, 1, 2].map(|index| e(E::AccessIndex { base: out, index }));
    let emitted = function.expressions.range_from(emitted_from);
    function.body.push(S::Emit(emitted), Span::UNDEFINED);
    for (pointer, value) in places.into_iter().zip([back, number, red]) {
        function
            .body
            .push(S::Store { pointer, value }, Span::UNDEFINED);
    }
    module.entry_points.push(naga::EntryPoint {
        name: "main".to_owned(),
        stage: naga::ShaderStage::Compute,
        early_depth_test: None,
        workgroup_size: [1; 3],
        workgroup_size_overrides: None,
        function,
        mesh_info: None,
        task_payload: None,
        incoming_ray_payload: None,
    });
    check(&module).unwrap();
    module
}

#[test]
fn a_bit_cast_of_a_bit_cast_reads_what_was_first_cast() {
    let module = by_hand(Fault::None);
    assert_eq!(casts_of_casts(&module), 3);
    let mut passed = module.clone();
    let changed = opt::optimise(&mut passed);
    validate(&passed);
    let compared = compare(&module, &passed, SEEDS).unwrap();
    assert_eq!(compared.equal, 24);
    assert!(compared.computed[1] < compared.computed[0], "{compared:?}");
    // The cast to its own type changes nothing and is read past with the constants. The way
    // back from a float is what it reads; the cast that reads the float between two integers
    // and the one that read the cast to its own type read past them.
    assert_eq!((changed.idle, changed.casts), (1, 3), "{changed:?}");
    assert_eq!(casts_of_casts(&passed), 0);
    // Gone: the float and its way back, the float between the integers, the cast to its own
    // type. The signed cast stays, for the shift that reads it as signed.
    assert_eq!(opt::emitted(&module) - opt::emitted(&passed), 4);
    assert_eq!(opt::optimise(&mut passed), opt::Changed::default());
}

#[test]
fn the_comparison_sees_a_module_that_computes_something_else() {
    // A bound buffer: a module the validator takes, whose conversion is of a signed number.
    let right = by_hand(Fault::None);
    let wrong = by_hand(Fault::CastOfAnotherType);
    let said = compare(&right, &wrong, SEEDS).unwrap_err();
    assert!(said.contains("dword 1 of 1 out"), "{said}");
    // A texel is the texture's own, at the place sampled.
    for fault in [Fault::CoordinatesSwapped, Fault::OtherTexture] {
        let said = compare(&right, &by_hand(fault), SEEDS).unwrap_err();
        assert!(said.contains("dword 2 of 1 out"), "{said}");
    }
    // What the entry point returns, and the discard.
    let right = looped(V_MUL_F32, F_ONE);
    let said = compare(&right, &looped(V_SUB_F32, F_ONE), SEEDS).unwrap_err();
    assert!(said.contains("returned"), "{said}");
    let said = compare(&right, &looped(V_MUL_F32, I_ZERO), SEEDS).unwrap_err();
    assert!(said.contains("discarded"), "{said}");
    // And a module is the same as itself.
    assert_eq!(compare(&right, &right, SEEDS).unwrap().equal, 24);
}

/// A pixel program whose lane mask a comparison narrows where `narrows`: the lane stays on
/// where the first attribute's first component is under a half. `v2` is written under that
/// mask, `v3` past a branch that leaves where no lane runs, `v4` where the two paths meet and
/// `v5` once the mask is what it was. The target takes the four.
fn narrowed(narrows: bool) -> naga::Module {
    let keep = op(opcodes::SOP1, "s_mov_b64");
    let target = exp(0, [2, 3, 4, 5]);
    let words = vec![
        vintrp(1, 0, 0, 0, 1),
        vintrp(1, 1, 0, 1, 1),
        sop1(keep, 24, 126),
        match narrows {
            true => vopc(V_CMPX_GT_F32, F_HALF, 0),
            // In its place a move that changes nothing.
            false => sop1(keep, 28, 126),
        },
        vop2(V_ADD_F32, 2, V, 1),
        sopp(op(opcodes::SOPP, "s_cbranch_execz"), 1),
        vop2(V_MUL_F32, 3, V, 1),
        vop2(V_SUB_F32, 4, V, 1),
        sop1(keep, 126, 24),
        vop2(V_ADD_F32, 5, V + 2, 3),
        target[0],
        target[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Pixel, words).module
}

#[test]
fn a_write_keeps_its_guard_unless_a_branch_or_a_constant_decides_the_mask() {
    let module = narrowed(true);
    // The two attributes, then `v2` to `v5`.
    assert_eq!(opt::guarded(&module), 6);
    let mut passed = module.clone();
    let changed = opt::optimise(&mut passed);
    validate(&passed);
    let compared = compare(&module, &passed, SEEDS).unwrap();
    assert_eq!(compared.equal, 24);
    assert!(compared.computed[1] < compared.computed[0], "{compared:?}");
    // Decided: the attributes and `v5`, where the mask is the constant it started as, and
    // `v3`, inside the branch on it. `v2` and `v4` are written by a lane that may not run.
    assert_eq!(
        (changed.guards, opt::guarded(&passed)),
        (4, 2),
        "{changed:?}"
    );
    // Some seed's lane does not run: with the mask left whole the target holds other numbers.
    let said = compare(&module, &narrowed(false), SEEDS).unwrap_err();
    assert!(said.contains("returned"), "{said}");
}

#[test]
fn a_load_reads_what_the_last_store_left_and_an_expression_is_computed_once() {
    // Three turns of a loop that reads `v0` and `v1`, writes `v2` and `v0`, and counts down.
    let module = looped(V_MUL_F32, F_ONE);
    let mut passed = module.clone();
    let changed = opt::optimise(&mut passed);
    validate(&passed);
    assert!(changed.forwarded > 0, "{changed:?}");
    let compared = compare(&module, &passed, SEEDS).unwrap();
    assert_eq!(compared.equal, 24);
    assert!(compared.computed[1] < compared.computed[0], "{compared:?}");
}

/// A compute program of a group of 128 threads, two waves of 64, that shares `shared_dwords`.
fn two_waves(shared_dwords: u32, words: Vec<u32>) -> naga::Module {
    let program = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [16, 8, 1],
            shared_dwords,
        }),
        ..program(ShaderKind::Compute, words)
    };
    let lowered = translate(&program).unwrap();
    validate(&lowered.module);
    lowered.module
}

/// `v3` = the thread's number; the lanes whose number is odd stay on, the mask kept in `s24`,
/// and `s26` = the number of the wave's first lane left. Past a branch that leaves where no
/// lane of the wave runs, `s27` = 7 and `v5` = twice the number; the mask is put back.
/// `between` stands before the branch.
fn odd_lanes(between: &[u32]) -> Vec<u32> {
    let mut words = vec![
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 3, I_ZERO + 4, 1),
        vop2(op(opcodes::VOP2, "v_add_u32"), 3, V, 3),
        vop2(op(opcodes::VOP2, "v_and_b32"), 4, I_ONE, 3),
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO, 4),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        sop2(op(opcodes::SOP2, "s_and_b64"), 126, 126, 106),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 26, V + 3),
    ];
    words.extend(between);
    words.extend([
        sopp(op(opcodes::SOPP, "s_cbranch_execz"), 2),
        sop1(op(opcodes::SOP1, "s_mov_b32"), 27, I_ZERO + 7),
        vop2(op(opcodes::VOP2, "v_add_u32"), 5, V + 3, 3),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 24),
        S_ENDPGM,
    ]);
    words
}

/// The registers each thread of `module`'s two waves leaves, its threads run at once.
fn left_by_two_waves(module: &naga::Module) -> Vec<BTreeMap<String, u32>> {
    (interp::run(module, [16, 8, 1], &|_, _| None).into_iter())
        .map(|thread| thread.registers.into_iter().collect())
        .collect()
}

#[test]
fn a_mask_another_lane_s_vote_or_the_wave_s_turn_decides_stays_guarded() {
    // A wave enters the block where any of its lanes runs: both waves here, and with each its
    // lanes of even number, which write `s27` and must leave `v5` as it was. With a barrier
    // the waves take turns, and a write is the turn's to decide as well.
    let barrier = sopp(op(opcodes::SOPP, "s_barrier"), 0);
    for (between, waits) in [(&[][..], false), (&[barrier][..], true)] {
        let module = two_waves(0, odd_lanes(between));
        let mut passed = module.clone();
        let changed = opt::optimise(&mut passed);
        validate(&passed);
        let (before, after) = (opt::guarded(&module), opt::guarded(&passed));
        let (lowered, optimised) = (left_by_two_waves(&module), left_by_two_waves(&passed));
        assert_eq!(lowered.len(), 128);
        for (thread, (was, is)) in lowered.iter().zip(&optimised).enumerate() {
            // A register a run never touched held nought.
            let held =
                |left: &BTreeMap<String, u32>, name: &str| left.get(name).copied().unwrap_or(0);
            let (thread, odd) = (thread as u32, thread as u32 & 1);
            let read = ["s26", "s27", "v5"].map(|name| held(was, name));
            assert_eq!(read, [thread & !63 | 1, 7, 2 * thread * odd], "{thread}");
            for name in was.keys().chain(is.keys()) {
                assert_eq!(held(was, name), held(is, name), "{name} of thread {thread}");
            }
        }
        // No write lost its guard to a condition: the four gone wrote nought over the nought
        // that `exec_hi` (twice), `vcc_hi` and `s25` always hold. The one `select` decided is
        // on a count of waiting waves that a group with no barrier of its own never sets.
        let decided = (changed.guards, changed.branches, changed.selects);
        assert_eq!(decided, (0, 0, usize::from(!waits)), "{changed:?}");
        assert_eq!(before - after, 4, "{changed:?}");
    }
}

/// One kept module, as a process of its own runs it: the panic that stops a whole run ends the
/// process, and a test of this crate does not catch one. A line is printed at each step, for
/// the process that started this one to read: that the validator takes the module as lowered,
/// what it is once it takes it after the passes too, then each seed as it starts, once it has
/// run as lowered, and as it is found equal or not.
fn one_kept_module(file: &Path, seeds: Range<u64>) {
    let name = file.file_stem().unwrap().to_string_lossy();
    // The harness has not ended the line of the test's name.
    println!();
    let kept = crate::rdna_cache::decode(&std::fs::read(file).unwrap());
    let Some(crate::rdna_cache::Cached::Translated(kept)) = kept else {
        panic!("{name}: not a module this build kept");
    };
    let lowered = kept.module;
    check(&lowered).unwrap_or_else(|why| panic!("{name} as lowered: {why}"));
    println!("valid");
    let mut passed = lowered.clone();
    let started = std::time::Instant::now();
    let changed = opt::optimise(&mut passed);
    let took = started.elapsed();
    check(&passed).unwrap_or_else(|why| panic!("{name} after the passes: {why}"));
    let entry = &lowered.entry_points[0];
    println!(
        "module {:?} of {:?} threads, {} expressions, {} after the passes in {took:?}, \
         {} of {} guarded writes left, {changed:?}",
        entry.stage,
        entry.workgroup_size,
        opt::emitted(&lowered),
        opt::emitted(&passed),
        opt::guarded(&passed),
        opt::guarded(&lowered)
    );
    let mut reached = HashSet::new();
    for seed in seeds {
        println!("seed {seed}");
        let mut as_lowered = |seed| println!("as-lowered {seed}");
        match compare_noting(&lowered, &passed, seed..seed + 1, &mut as_lowered) {
            Err(why) => println!("differs {why}"),
            Ok(compared) => {
                reached.extend(compared.reached);
                println!(
                    "equal {seed} {} {} {} {}",
                    compared.discarded,
                    compared.computed[0],
                    compared.computed[1],
                    reached.len()
                );
            }
        }
    }
}

/// What the processes that ran one kept module said of it.
#[derive(Debug, Default)]
struct Kept {
    /// The first process's line of what the module is.
    module: String,
    /// Seeds equal, and of those the seeds with a discard.
    equal: [u32; 2],
    /// The expressions computed as lowered and after the passes, over the equal seeds.
    computed: [u64; 2],
    /// The most expressions of the module that one process reached over its seeds.
    reached: u64,
    /// What stopped the module as lowered, each with its seeds.
    stopped: BTreeMap<String, Vec<u64>>,
    /// What must not be: a seed that differs, or that only the module after the passes stops on.
    wrong: Vec<String>,
}

/// Runs this test again as a process of its own for each stretch of seeds a kept module runs
/// without stopping, and gathers what they print.
fn kept_module(file: &Path, seeds: u64) -> Kept {
    let mut kept = Kept::default();
    let mut from = 0;
    while from < seeds {
        let this = std::env::current_exe().unwrap();
        let test = "rdna::tests::equal::the_kept_sample_computes_the_same_after_the_passes";
        let ran = (std::process::Command::new(this))
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env("OPENRAGE_RDNA_ONE", file)
            .env("OPENRAGE_RDNA_FROM", from.to_string())
            .output()
            .unwrap();
        let (said, panic) = (
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr),
        );
        // The seed a process stopped on is the one after the last it found equal or not.
        let (mut next, mut as_lowered) = (from, None);
        let (mut valid, mut passes_valid) = (false, false);
        for line in said.lines() {
            let mut words = line.split(' ');
            let mut number = || words.next().and_then(|word| word.parse::<u64>().ok());
            match line.split(' ').next() {
                Some("valid") => valid = true,
                Some("module") => {
                    passes_valid = true;
                    if kept.module.is_empty() {
                        line.clone_into(&mut kept.module);
                    }
                }
                Some("as-lowered") => as_lowered = line[11..].parse::<u64>().ok(),
                Some("differs") => {
                    kept.wrong.push(line.to_owned());
                    next += 1;
                }
                Some("equal") => {
                    let read = [(); 6].map(|()| number().unwrap_or(0));
                    kept.equal[0] += 1;
                    kept.equal[1] += read[2] as u32;
                    kept.computed[0] += read[3];
                    kept.computed[1] += read[4];
                    kept.reached = kept.reached.max(read[5]);
                    next = read[1] + 1;
                }
                _ => {}
            }
        }
        if !ran.status.success() {
            let mut lines = panic
                .lines()
                .skip_while(|line| !line.contains("panicked at"));
            // The place of an expression differs from seed to seed; what it is does not.
            let why: String = lines
                .nth(1)
                .unwrap_or("no panic")
                .chars()
                .take(72)
                .collect();
            match as_lowered {
                // Not a module to compare anything with, on any seed.
                _ if !valid => {
                    kept.stopped.entry(why).or_default().extend(next..seeds);
                    next = seeds;
                }
                _ if !passes_valid => {
                    kept.wrong.push(format!("after the passes: {why}"));
                    next = seeds;
                }
                Some(seed) if seed == next => {
                    let wrong = format!("seed {seed}: as lowered it ran; after the passes: {why}");
                    kept.wrong.push(wrong);
                }
                _ => kept.stopped.entry(why).or_default().push(next),
            }
            next += 1;
        }
        from = next.max(from + 1);
    }
    kept
}

#[test]
fn the_kept_sample_computes_the_same_after_the_passes() {
    let seeds = std::env::var("OPENRAGE_RDNA_SEEDS").map_or(12, |seeds| seeds.parse().unwrap());
    if let Some(file) = std::env::var_os("OPENRAGE_RDNA_ONE") {
        let from = std::env::var("OPENRAGE_RDNA_FROM")
            .unwrap()
            .parse()
            .unwrap();
        // A module of a large program nests deeper than a test's own stack holds.
        let run = move || one_kept_module(Path::new(&file), from..seeds);
        let ran = (std::thread::Builder::new().stack_size(1 << 30)).spawn(run);
        assert!(ran.unwrap().join().is_ok());
        return;
    }
    let Some(folder) = std::env::var_os("OPENRAGE_RDNA_SAMPLE") else {
        eprintln!("skipped: OPENRAGE_RDNA_SAMPLE names no folder of kept modules");
        return;
    };
    let mut files: Vec<_> = (std::fs::read_dir(&folder).expect("the folder of kept modules"))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "module")
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no kept module in {folder:?}");
    let (mut ran, mut wrong) = (0, Vec::new());
    for file in &files {
        let name = file.file_stem().unwrap().to_string_lossy();
        let kept = kept_module(file, seeds);
        ran += usize::from(kept.equal[0] > 0);
        println!(
            "{name}: {}; equal on {} of {seeds} seeds ({} with a discard), computing {} for {}, \
             at least {} expressions reached; stopped by {:?}",
            kept.module,
            kept.equal[0],
            kept.equal[1],
            kept.computed[1],
            kept.computed[0],
            kept.reached,
            kept.stopped
        );
        wrong.extend(kept.wrong.iter().map(|wrong| format!("{name}: {wrong}")));
    }
    println!(
        "{ran} of {} modules ran to their end on a seed",
        files.len()
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
}
