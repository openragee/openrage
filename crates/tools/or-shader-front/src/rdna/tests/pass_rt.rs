//! A program that tests rays against the nodes of a structure it reads from memory, lowered
//! with the structure as one buffer of dwords and the test as one function of the module.

use naga::{Expression as E, Statement as S};

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

pub(super) fn traced(words: Vec<u32>) -> Result<Lowered, RdnaError> {
    let options = Options {
        arena: true,
        ..Options::default()
    };
    let lowered = translate_with(&compute(words), &options)?;
    validate(&lowered.module);
    Ok(lowered)
}

fn named<'a>(module: &'a naga::Module, name: &str) -> &'a naga::Function {
    let found = (module.functions.iter()).find(|(_, f)| f.name.as_deref() == Some(name));
    found.unwrap_or_else(|| panic!("no function {name}")).1
}

/// The calls of the node test among `function`'s statements.
fn tests_of_a_node(module: &naga::Module, function: &str) -> usize {
    let test = (module.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("bvh_intersect"))
        .map(|(handle, _)| handle);
    count(
        &named(module, function).body,
        &|s| matches!(s, S::Call { function, .. } if Some(*function) == test),
    )
}

/// The descriptor of a structure in `s8`..`s11`, its address word `first`, and the test of the
/// node in `v0` (and `v1` for the 64-bit form) by the ray in the registers after it.
pub(super) fn test_words(name: &str, first: u32, modes: u32) -> Vec<u32> {
    described(
        name,
        first,
        [
            sop1(op(opcodes::SOP1, "s_brev_b32"), 9, I_ONE),
            I_MINUS_ONE,
            modes,
        ],
    )
}

/// [`test_words`] of a descriptor whose second word is what `second` writes to `s9`, whose
/// third is the operand `third` and whose fourth the literal `modes`.
fn described(name: &str, first: u32, [second, third, modes]: [u32; 3]) -> Vec<u32> {
    let s_mov = op(opcodes::SOP1, "s_mov_b32");
    let test = mimg(name, 0, 0xF, 0);
    vec![
        sop1(s_mov, 8, first),
        second,
        sop1(s_mov, 10, third),
        sop1(s_mov, 11, LITERAL),
        modes,
        test[0],
        test[1],
    ]
}

pub(super) const MODES: u32 = 0x8900_03FF;
pub(super) const WIDE: &str = "image_bvh64_intersect_ray";

#[test]
fn a_structure_is_memory_and_the_test_of_a_node_one_function() {
    // s4.. = the structure's descriptor; s12 = its address cut to 256 bytes, at its root's
    // word; s14 = that word by a scalar load, v1 = it by a vector one; then the test of v0.
    let descriptor = smem(S_LOAD_X4, 4, 0, 0x20);
    let through = smem(op(opcodes::SMEM, "s_load_dword"), 14, 12, 0);
    let by_lane = global_load(1, 0, 4);
    let mut words = vec![
        descriptor[0],
        descriptor[1],
        sop2(op(opcodes::SOP2, "s_and_b32"), 12, 4, LITERAL),
        0xFFFF_FF00,
        sop2(op(opcodes::SOP2, "s_or_b32"), 12, 12, I_ZERO + 48),
        through[0],
        through[1],
        vop1(V_MOV, 0, 12),
        by_lane[0],
        by_lane[1],
    ];
    words.extend((2..12).map(|v| vop1(V_MOV, v, F_ONE)));
    let at = words.len() as u32 + 5;
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    let cfg = Cfg::build(&words).unwrap();
    let out = traced(words).unwrap();

    let arenas: Vec<_> = (out.resources.iter())
        .filter(|r| r.kind == ResourceKind::Arena)
        .collect();
    assert_eq!(arenas.len(), 1, "{:?}", out.resources);
    assert_eq!((arenas[0].at.as_ref(), arenas[0].written), (None, false));
    // The descriptor's words are numbers of its table, as any descriptor's are.
    assert!((out.resources.iter())
        .any(|r| r.kind == ResourceKind::TableWords && r.at == Some(plain(0, 0))));
    let (arena, global) = (out.module.global_variables.iter())
        .find(|(_, global)| global.name.as_deref() == Some("arena"))
        .unwrap();
    assert_eq!(
        global.space,
        naga::AddressSpace::Storage {
            access: naga::StorageAccess::LOAD
        }
    );
    // Both loads read the buffer at their address over four, and nothing else does.
    let main = named(&out.module, "main_body");
    let reads = (main.expressions.iter()).filter(|(_, e)| {
        let E::Access { base, index } = **e else {
            return false;
        };
        let by_four = matches!(
            main.expressions[index],
            E::Binary { op: naga::BinaryOperator::ShiftRight, right, .. }
                if main.expressions[right] == E::Literal(naga::Literal::U32(2))
        );
        main.expressions[base] == E::GlobalVariable(arena) && by_four
    });
    assert_eq!(reads.count(), 2);

    let test = named(&out.module, "bvh_intersect");
    let arguments: Vec<_> = (test.arguments.iter())
        .map(|a| (a.name.as_deref().unwrap(), &out.module.types[a.ty].inner))
        .collect();
    let word = naga::TypeInner::Scalar(naga::Scalar::U32);
    let float = naga::TypeInner::Scalar(naga::Scalar::F32);
    let three = naga::TypeInner::Vector {
        size: naga::VectorSize::Tri,
        scalar: naga::Scalar::F32,
    };
    assert_eq!(
        arguments,
        [
            ("base", &word),
            ("node", &word),
            ("far", &float),
            ("origin", &three),
            ("direction", &three),
            ("inverse", &three),
        ]
    );
    assert_eq!(
        out.module.types[test.result.as_ref().unwrap().ty].inner,
        naga::TypeInner::Vector {
            size: naga::VectorSize::Quad,
            scalar: naga::Scalar::U32,
        }
    );
    assert_eq!(tests_of_a_node(&out.module, "main_body"), 1);
    assert_eq!(
        out.intersections,
        [Intersection {
            word: at,
            wide: true,
            function: "main_body".to_owned(),
            depth: 0,
            loops: Vec::new(),
        }]
    );
    assert_eq!(out.intersections, intersections(&cfg));
    // The test of a node is the module's own function, and counted as no departure.
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
}

#[test]
fn without_the_option_nothing_is_read_as_a_structure() {
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    let Err(RdnaError::NotTranslated(why)) = translate(&compute(words.clone())) else {
        panic!("a program that tests nodes was lowered unasked");
    };
    assert_eq!(why.name(), "a program that traces rays");
    assert!(traced(words).is_ok());
    // A pointer the program computed is an address of a structure only with the option.
    let load = smem(S_LOAD_X4, 16, 10, 0);
    let code = vec![sop2(S_ADD_U32, 10, 1, 1), load[0], load[1], S_ENDPGM];
    let lowered = traced(code).unwrap();
    assert!(lowered.intersections.is_empty());
    assert!((lowered.resources.iter()).any(|r| r.kind == ResourceKind::Arena));
    // And one that is a table's on one path and computed on the other is no address either way.
    let table = smem(S_LOAD_X2, 10, 0, 0);
    let code = vec![
        sop2(S_ADD_U32, 10, 1, 1),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 2),
        table[0],
        table[1],
        load[0],
        load[1],
        S_ENDPGM,
    ];
    let Err(RdnaError::NotTranslated(why)) = traced(code) else {
        panic!("a pointer that may be a table's was read");
    };
    assert_eq!(
        why.name(),
        "a table through a pointer or at an offset the program computed"
    );
}

#[test]
fn the_32_bit_form_takes_the_structure_s_address_from_its_descriptor() {
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    // The descriptor's first word is the structure's address in units of 256 bytes.
    words.extend(test_words("image_bvh_intersect_ray", 1, MODES));
    words.push(S_ENDPGM);
    let out = traced(words).unwrap();
    assert_eq!(out.intersections.len(), 1);
    assert!(!out.intersections[0].wide);
    // The far distance is the register after the node here, and two after it in the 64-bit form.
    let main = named(&out.module, "main_body");
    let far = |function: &naga::Function| {
        let mut tests = flat(&function.body).into_iter().filter_map(|s| match s {
            S::Call { arguments, .. } if arguments.len() == 6 => Some(arguments[2]),
            _ => None,
        });
        tests.next().unwrap()
    };
    let register = |function: &naga::Function, value: naga::Handle<E>| {
        let E::As { expr, .. } = function.expressions[value] else {
            panic!("the far distance is no cast");
        };
        let E::Load { pointer } = function.expressions[expr] else {
            panic!("the far distance is no register");
        };
        let E::LocalVariable(local) = function.expressions[pointer] else {
            panic!("the far distance is no register");
        };
        function.local_variables[local].name.clone().unwrap()
    };
    assert_eq!(register(main, far(main)), "v1");
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    let out = traced(words).unwrap();
    let main = named(&out.module, "main_body");
    assert_eq!(register(main, far(main)), "v2");
}

/// The register a dword of `function` is read from: a local's load, reread as a float or not.
fn register_of(function: &naga::Function, mut value: naga::Handle<E>) -> String {
    if let E::As { expr, .. } = function.expressions[value] {
        value = expr;
    }
    let E::Load { pointer } = function.expressions[value] else {
        panic!("{:?} is no register", function.expressions[value]);
    };
    let E::LocalVariable(local) = function.expressions[pointer] else {
        panic!("{:?} is no register", function.expressions[pointer]);
    };
    function.local_variables[local].name.clone().unwrap()
}

#[test]
fn the_call_takes_the_descriptor_s_word_the_node_and_the_ray_in_the_instruction_s_order() {
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    // The descriptor's first word is the user register `s1`.
    words.extend(test_words("image_bvh_intersect_ray", 1, MODES));
    words.push(S_ENDPGM);
    let out = traced(words).unwrap();
    let main = named(&out.module, "main_body");
    let statements = flat(&main.body);
    let (arguments, result) = (statements.iter())
        .find_map(|s| match s {
            S::Call {
                arguments,
                result: Some(result),
                ..
            } if arguments.len() == 6 => Some((arguments, *result)),
            _ => None,
        })
        .expect("no test of a node");
    // The structure's address word, the node, the far distance.
    let scalars: Vec<String> = (arguments[..3].iter())
        .map(|&argument| register_of(main, argument))
        .collect();
    assert_eq!(scalars, ["s8", "v0", "v1"]);
    // Then the origin, the direction and the direction's inverse, three registers each.
    let vectors: Vec<Vec<String>> = (arguments[3..].iter())
        .map(|&argument| {
            let E::Compose { ref components, .. } = main.expressions[argument] else {
                panic!("{:?} is no vector", main.expressions[argument]);
            };
            (components.iter())
                .map(|&part| register_of(main, part))
                .collect()
        })
        .collect();
    assert_eq!(
        vectors,
        [["v2", "v3", "v4"], ["v5", "v6", "v7"], ["v8", "v9", "v10"]]
    );
    // The four dwords answered go to the four registers from the instruction's destination.
    for k in 0..4u32 {
        let stored = statements.iter().any(|s| {
            let S::Store { pointer, value } = **s else {
                return false;
            };
            let E::LocalVariable(local) = main.expressions[pointer] else {
                return false;
            };
            main.local_variables[local].name.as_deref() == Some(&format!("v{k}"))
                && main.expressions[value]
                    == E::AccessIndex {
                        base: result,
                        index: k,
                    }
        });
        assert!(stored, "dword {k} of the answer is not in v{k}");
    }
}

#[test]
fn each_word_of_the_descriptor_is_held_to_the_contract_alone() {
    let sorted = sop1(op(opcodes::SOP1, "s_brev_b32"), 9, I_ONE);
    let unsorted = sop1(op(opcodes::SOP1, "s_mov_b32"), 9, I_ZERO);
    for (descriptor, lowers) in [
        ([sorted, I_MINUS_ONE, MODES], true),
        // The children as the node holds them, not the nearest first.
        ([unsorted, I_MINUS_ONE, MODES], false),
        // An address range that is not the whole of it.
        ([sorted, I_ONE, MODES], false),
        ([sorted, I_MINUS_ONE, MODES ^ 1], false),
    ] {
        let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
        words.extend(described(WIDE, I_ZERO, descriptor));
        words.push(S_ENDPGM);
        match traced(words) {
            Ok(_) => assert!(lowers, "{descriptor:x?} was lowered"),
            Err(RdnaError::NotTranslated(why)) => {
                assert!(!lowers, "{descriptor:x?}: {why}");
                assert!(why.name().ends_with("and barycentrics"), "{}", why.name());
            }
            Err(other) => panic!("{other}"),
        }
    }
}

#[test]
fn group_memory_that_is_no_stack_takes_the_wave_whole_in_a_program_that_tests_nodes() {
    let with = |memory: &[&str], test: bool| {
        let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
        words.extend(memory.iter().flat_map(|name| ds(name, 4)));
        if test {
            words.extend(test_words(WIDE, I_ZERO, MODES));
        }
        words.push(S_ENDPGM);
        let out = traced(words).unwrap();
        let barrier = |s: &S| matches!(s, S::ControlBarrier(_));
        count(&named(&out.module, "main_body").body, &barrier)
    };
    // A lane reads what another stored: the lanes meet before the store and before the load,
    // which the program's own compiler left out for a group that is one wave in step.
    assert_eq!(with(&["ds_write_b32", "ds_read_b32"], true), 2);
    // What a walk keeps its stack with is each lane's own, where nothing else is the wave's.
    let stack = [
        "ds_write_b32",
        "ds_wrxchg_rtn_b32",
        "ds_write2_b32",
        "ds_read2_b32",
    ];
    assert_eq!(with(&stack, true), 0);
    // And a program that tests no node keeps the memory as it did.
    assert_eq!(with(&["ds_write_b32", "ds_read_b32"], false), 0);
}

/// Every statement of `block`, those of the blocks inside it too.
fn flat(block: &naga::Block) -> Vec<&S> {
    let mut all = Vec::new();
    for statement in block {
        all.push(statement);
        match statement {
            S::Block(inner) => all.extend(flat(inner)),
            S::If { accept, reject, .. } => {
                all.extend(flat(accept));
                all.extend(flat(reject));
            }
            S::Loop {
                body, continuing, ..
            } => {
                all.extend(flat(body));
                all.extend(flat(continuing));
            }
            _ => {}
        }
    }
    all
}

#[test]
fn a_descriptor_of_other_modes_is_refused_by_name() {
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    // The return of barycentrics is not asked for.
    words.extend(test_words(WIDE, I_ZERO, MODES & !(1 << 24)));
    words.push(S_ENDPGM);
    let Err(RdnaError::NotTranslated(why)) = traced(words) else {
        panic!("a test of other modes was lowered");
    };
    assert!(
        why.name()
            .ends_with("the whole address range and barycentrics"),
        "{}",
        why.name()
    );
}

#[test]
fn a_test_is_placed_by_its_nest_its_loops_and_the_function_it_lands_in() {
    // A loop from word 12, and in it 17 ifs one in another around the test: deeper than one
    // function is given, so the innermost statements are in a function of their own.
    const IFS: u32 = 17;
    let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    let head = words.len() as u32;
    let test = test_words(WIDE, I_ZERO, MODES);
    for k in 0..IFS {
        // Past the ifs still to open, the test and the ends of the ifs inside this one.
        let past = 2 * (IFS - 1 - k) + test.len() as u32 + (IFS - 1 - k);
        words.push(sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO));
        words.push(sopp(S_CBRANCH_SCC0, past as u16));
    }
    let at = words.len() as u32 + 5;
    words.extend(test);
    words.extend((0..IFS).map(|_| sopp(0, 0)));
    words.push(sopc(S_CMP_LG_U32, I_ZERO, I_ZERO));
    let back = head as i32 - (words.len() as i32 + 1);
    words.push(sopp(S_CBRANCH_SCC1, back as i16 as u16));
    words.push(S_ENDPGM);
    let cfg = Cfg::build(&words).unwrap();
    let out = traced(words).unwrap();
    let [site] = out.intersections.as_slice() else {
        panic!("{:?}", out.intersections);
    };
    assert_eq!((site.word, site.depth), (at, IFS + 1));
    assert_eq!(site.loops, [head]);
    assert!(site.function.starts_with("part"), "{}", site.function);
    // The function named is the one that holds the call, and no other does.
    assert_eq!(tests_of_a_node(&out.module, &site.function), 1);
    assert_eq!(tests_of_a_node(&out.module, "main_body"), 0);
    // The same follows from the graph alone, for a program something else stops.
    assert_eq!(out.intersections, intersections(&cfg));
}

#[test]
fn a_sum_that_leaves_its_carry_in_the_register_it_added_still_makes_an_address_of_the_code() {
    // s4 = the address of word 1; v2 = it moved on by v0, the carry left in s4; v1 = the word there.
    let sum = vop3(op3("v_add_co_u32"), 2, [V, 4, 0], 4, 0);
    let load = global_load(1, 2, 0);
    let words = vec![
        sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0),
        sum[0],
        sum[1],
        load[0],
        load[1],
        S_ENDPGM,
        ONE,
        ONE,
    ];
    let out = traced(words).unwrap();
    assert!((out.module.global_variables.iter())
        .any(|(_, global)| global.name.as_deref() == Some("code_data")));
    assert!(!(out.resources.iter()).any(|r| r.kind == ResourceKind::Arena));
}

/// A program that tests a node and keeps a dword at its lane's own address of the group's
/// memory: `v20` is the lane's number, `v22` what the lane finds where it stored that number
/// and 60, `v24` whether it took the branch the lanes of the wave's high half take. `more` is
/// put before the test, which is left out without `test`.
fn a_lane_s_own(more: &[u32], test: bool) -> Vec<u32> {
    // The lanes before this one in the low half of the wave, then those in the high half.
    let low = vop3(
        op3("v_mbcnt_lo_u32_b32"),
        20,
        [I_MINUS_ONE, I_ZERO, 0],
        0,
        0,
    );
    let lane = vop3(
        op3("v_mbcnt_hi_u32_b32"),
        20,
        [I_MINUS_ONE, V + 20, 0],
        0,
        0,
    );
    let (write, take) = (ds("ds_write_b32", 0), ds("ds_wrxchg_rtn_b32", 0));
    let mut words: Vec<u32> = (2..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    words.extend([
        low[0],
        low[1],
        lane[0],
        lane[1],
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 0, I_ZERO + 2, 20),
        vop2(op(opcodes::VOP2, "v_add_u32"), 1, I_ZERO + 60, 20),
        write[0],
        write[1],
        vop1(V_MOV, 1, I_ZERO + 7),
        take[0],
        take[1],
        vop1(V_MOV, 22, V + 1),
        // The lanes past the thirty-second go on into a block the others jump past.
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        vopc(op(opcodes::VOPC, "v_cmpx_lt_u32"), I_ZERO + 31, 20),
        sopp(op(opcodes::SOPP, "s_cbranch_execz"), 1),
        vop1(V_MOV, 24, I_ONE),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 24),
    ]);
    words.extend(more);
    if test {
        words.extend(test_words(WIDE, I_ZERO, MODES));
    }
    words.push(S_ENDPGM);
    words
}

/// Whether `module` holds a statement or a variable of a wave: a barrier, an operation of the
/// device's subgroup, or the memory the lanes' values are exchanged in.
fn holds_a_wave(module: &naga::Module) -> bool {
    let of_a_wave = |s: &S| {
        matches!(
            s,
            S::ControlBarrier(_)
                | S::SubgroupBallot { .. }
                | S::SubgroupCollectiveOperation { .. }
                | S::SubgroupGather { .. }
        )
    };
    let functions = (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function));
    let statements = functions.map(|function| count(&function.body, &of_a_wave));
    statements.sum::<usize>() > 0
        || (module.global_variables.iter())
            .any(|(_, global)| global.name.as_deref() == Some("wave_values"))
}

#[test]
fn a_program_that_tests_nodes_and_holds_nothing_of_its_wave_runs_a_lane_an_invocation() {
    let out = traced(a_lane_s_own(&[], true)).unwrap();
    assert!(!out.subgroups && !holds_a_wave(&out.module));
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    // Every thread of the group at once: each is the lane its place in the group makes it,
    // and its dword of the group's memory is its own.
    let ran = interp::run(&out.module, [8, 8, 1], &|_, _| None);
    assert_eq!(ran.len(), 64);
    for (thread, lane) in ran.iter().enumerate() {
        let thread = thread as u32;
        assert_eq!(
            [lane.reg("v20"), lane.reg("v22"), lane.reg("v24")],
            [thread, thread + 60, u32::from(thread > 31)]
        );
    }
}

#[test]
fn one_operation_over_the_wave_keeps_the_wave_exchanged_in_the_group_s_memory() {
    // The wave's first lane's number, which no lane but the first holds.
    let first = vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V + 20);
    let out = traced(a_lane_s_own(&[first, vop1(V_MOV, 23, 20)], true)).unwrap();
    assert!(holds_a_wave(&out.module));
    let ran = interp::run(&out.module, [8, 8, 1], &|_, _| None);
    for (thread, lane) in ran.iter().enumerate() {
        let thread = thread as u32;
        assert_eq!(
            [lane.reg("v20"), lane.reg("v23"), lane.reg("v24")],
            [thread, 0, u32::from(thread > 31)]
        );
    }
    // And a program that tests no node is lowered as it was, whatever the option: its
    // branch is voted by the wave.
    let out = traced(a_lane_s_own(&[], false)).unwrap();
    assert!(holds_a_wave(&out.module));
}

#[test]
fn a_loop_lowered_alone_is_given_its_registers_and_leaves_those_named() {
    // Before the loop v20 = 5, after it v21 = 0: neither is the loop's. A turn adds v22 to
    // v21, tests a node and counts in s20, and the loop is left once s20 is 3 or more.
    let mut words: Vec<u32> = vec![vop1(V_MOV, 20, I_ZERO + 5)];
    let head = words.len() as u32;
    words.push(vop2(op(opcodes::VOP2, "v_add_u32"), 21, V + 22, 21));
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(sop2(S_ADD_U32, 20, 20, I_ONE));
    words.push(sopc(op(opcodes::SOPC, "s_cmp_lt_u32"), 20, I_ZERO + 3));
    let back = head as i32 - (words.len() as i32 + 1);
    words.push(sopp(S_CBRANCH_SCC1, back as i16 as u16));
    let exit = words.len() as u32;
    words.extend([vop1(V_MOV, 21, I_ZERO), S_ENDPGM]);
    let cfg = Cfg::build(&words).unwrap();
    let [site] = intersections(&cfg).try_into().unwrap();
    assert_eq!(site.loops, [head]);
    let alone = |first: u32, left: &[&str]| {
        let options = Options {
            arena: true,
            region: Some(Region {
                first,
                exit,
                left: left.iter().map(|name| (*name).to_owned()).collect(),
            }),
            ..Options::default()
        };
        translate_with(&compute(words.clone()), &options)
    };
    let out = alone(head, &["v21", "s20", "v20"]).unwrap();
    validate(&out.module);
    assert!(!holds_a_wave(&out.module));
    let registers = out.region.clone().unwrap();
    assert_eq!(registers.left, ["v21", "s20", "v20"]);
    // The node, whose high dword is not read, the far distance and the ray; the sum and what
    // is added; the count. The descriptor is built in the loop, and v20 is given only to be
    // left as it was.
    let vectors = (0..12).filter(|v| *v != 1).chain(20..23);
    let mut given: Vec<String> = vectors.map(|v| format!("v{v}")).collect();
    given.push("s20".to_owned());
    assert_eq!(registers.given, given);
    let kinds: Vec<_> = (out.resources.iter())
        .filter(|r| r.kind == ResourceKind::Registers)
        .map(|r| r.written)
        .collect();
    assert!(
        kinds == [true, false] || kinds == [false, true],
        "{kinds:?}"
    );

    // Every thread of a group at once, each with its own row of the two buffers.
    let slots = registers.given.len();
    let slot = |name: &str| {
        registers
            .given
            .iter()
            .position(|given| given == name)
            .unwrap()
    };
    let mut rows = vec![0u32; 64 * slots];
    for thread in 0..64 {
        let row = &mut rows[thread * slots..][..slots];
        row[slot("v20")] = 900 + thread as u32;
        row[slot("v21")] = 1000 * thread as u32;
        row[slot("v22")] = 7 + thread as u32;
        row[slot("s20")] = thread as u32 % 5;
    }
    let buffers = [
        ("registers_given", &rows[..]),
        ("registers_left", &[0; 64 * 3][..]),
    ];
    let (_, left) = interp::run_bound(&out.module, [8, 8, 1], &|_, _| None, &buffers);
    for thread in 0..64u32 {
        let turns = 3u32.saturating_sub(thread % 5).max(1);
        assert_eq!(
            left[1][thread as usize * 3..][..3],
            [
                1000 * thread + turns * (7 + thread),
                thread % 5 + turns,
                900 + thread
            ],
            "thread {thread}"
        );
    }

    // A word no block starts at, and a register that is none, are refused by name.
    for (first, left) in [(head + 1, "v21"), (head, "v300")] {
        let Err(RdnaError::NotTranslated(why)) = alone(first, &[left]) else {
            panic!("a walk that is none was lowered");
        };
        assert_eq!(why.name(), "a walk lowered alone");
    }
}
