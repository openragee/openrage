//! What a translated module computes against what the hardware does: 16-bit floats, helper
//! lanes, and a wave of 64 on a device of any width.

use super::*;

/// The number a one-source mnemonic takes in the three-source encoding.
fn op1_wide(name: &str) -> u32 {
    0x180 + op(opcodes::VOP1, name)
}

/// The count of expressions of the body that are the dword `word`.
fn literals(module: &naga::Module, word: u32) -> usize {
    let wanted = naga::Expression::Literal(naga::Literal::U32(word));
    let all = body(module).expressions.iter();
    all.filter(|(_, e)| **e == wanted).count()
}

#[test]
fn a_half_is_widened_scaled_and_written_beside_the_other_half() {
    let position = exp(POS0, [2, 2, 2, 2]);
    // v2 = the half in v1 as a float, over two: the scale is the second word's output modifier.
    let mut halved = vop3(op1_wide("v_cvt_f32_f16"), 2, [V + 1, 0, 0], 0, 0);
    halved[1] |= 3 << 27;
    let words = vec![
        vop1(V_MOV, 1, F_ONE),
        halved[0],
        halved[1],
        // v1's low half = v2 as a half: its high half stays what the move left there.
        vop1(op(opcodes::VOP1, "v_cvt_f16_f32"), 1, V + 2),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    assert!(!packs_halves(&out.module));
    assert_eq!(literals(&out.module, 0xFFFF_0000), 1);
}

#[test]
fn a_16_bit_result_that_may_differ_from_the_hardware_s_is_counted_by_its_kind() {
    let position = exp(POS0, [2, 2, 2, 2]);
    let fused = vop3(op3("v_fma_f16"), 3, [V + 1, V + 1, V + 2], 0, 0);
    let words = vec![
        vop1(V_MOV, 1, F_ONE),
        vop1(op(opcodes::VOP1, "v_rsq_f16"), 2, V + 1),
        fused[0],
        fused[1],
        vop2(op(opcodes::VOP2, "v_mul_f16"), 2, V + 3, 2),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let notes = lowered(ShaderKind::Vertex, words).approximations;
    let counted: Vec<_> = notes.iter().map(|(note, count)| (*note, *count)).collect();
    assert_eq!(
        counted,
        [
            (
                "a 16-bit float function as exact as the device's 32-bit one",
                1
            ),
            (
                "a 16-bit fused multiply-add rounded at 32 bits before 16",
                1
            ),
        ]
    );
}

/// A compute program of a group of `threads`, each thread's number in `v0`, `v1` and `v2`.
fn compute(threads: [u32; 3], words: Vec<u32>) -> Program {
    Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads,
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, words)
    }
}

/// `v2` = this thread's lane in a group eight threads across.
fn lane_into_v2() -> Vec<u32> {
    vec![
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 2, I_ZERO + 3, 1),
        vop2(op(opcodes::VOP2, "v_add_u32"), 2, V, 2),
    ]
}

/// Runs a compute module a thread a lane, and gives each lane's registers.
fn lanes(module: &naga::Module, threads: [u32; 3]) -> Vec<interp::Thread> {
    interp::run(module, threads, &|_, _| None)
}

/// What the steps [`wave_fold`] writes leave in the first lane, each step done on all 64 lanes
/// as its instruction is defined: the eight-lane patterns, the row's mirror, the other row
/// of the same half, then lane 32 joined into every lane.
fn folded(mut lanes: [f32; 64], join: fn(f32, f32) -> f32) -> f32 {
    for selectors in [PAIR_SWAP, 0o4466_0022, 0o4444] {
        let from = |lane: usize| (lane & !7) | ((selectors >> (3 * (lane & 7))) & 7) as usize;
        lanes = std::array::from_fn(|lane| join(lanes[from(lane)], lanes[lane]));
    }
    lanes = std::array::from_fn(|lane| join(lanes[(lane & !15) | (15 - (lane & 15))], lanes[lane]));
    lanes = std::array::from_fn(|lane| join(lanes[(lane & !31) | (!lane & 16)], lanes[lane]));
    let half = lanes[32];
    join(half, lanes[0])
}

#[test]
fn a_sum_of_floats_over_a_wave_in_shared_memory_is_rounded_as_the_steps_round_it() {
    const SPREAD: u32 = 2_654_435_761;
    // v1 = this lane's number spread over a dword, as a float: sums of these round by their order.
    let spread = vop3(op3("v_mul_lo_u32"), 3, [V + 2, LITERAL, 0], 0, 0);
    let mut words = lane_into_v2();
    words.extend([spread[0], spread[1], SPREAD]);
    words.push(vop1(op(opcodes::VOP1, "v_cvt_f32_u32"), 1, V + 3));
    words.extend(wave_fold("v_add_f32"));
    words.push(S_ENDPGM);
    let out = translate(&compute([8, 8, 1], words)).unwrap();
    validate(&out.module);
    // No subgroup of the device is asked, and nothing is read as this lane alone.
    assert!(!out.subgroups);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let held: [f32; 64] = std::array::from_fn(|lane| (lane as u32).wrapping_mul(SPREAD) as f32);
    let sum = folded(held, |a, b| a + b);
    // The order matters here: the lanes added one after another come to another float.
    assert_ne!(sum, held.iter().sum::<f32>());
    for (lane, thread) in lanes(&out.module, [8, 8, 1]).iter().enumerate() {
        assert_eq!(thread.reg("s21"), sum.to_bits(), "lane {lane}");
    }
}

#[test]
fn a_wave_s_least_and_greatest_come_from_every_lane_of_shared_memory() {
    for (name, join) in [
        ("v_min_u32", u32::min as fn(u32, u32) -> u32),
        ("v_max_u32", u32::max),
        ("v_or_b32", |a, b| a | b),
        ("v_add_u32", u32::wrapping_add),
    ] {
        // v1 = (lane ^ 42) + 7: its least and greatest are in the wave's two halves.
        let mut words = lane_into_v2();
        words.push(vop2(op(opcodes::VOP2, "v_xor_b32"), 1, I_ZERO + 42, 2));
        words.push(vop2(op(opcodes::VOP2, "v_add_u32"), 1, I_ZERO + 7, 1));
        words.extend(wave_fold(name));
        words.push(S_ENDPGM);
        let out = translate(&compute([8, 8, 1], words)).unwrap();
        validate(&out.module);
        assert!(!out.subgroups);
        let want = (0..64u32).map(|lane| (lane ^ 42) + 7).reduce(join).unwrap();
        for (lane, thread) in lanes(&out.module, [8, 8, 1]).iter().enumerate() {
            assert_eq!(thread.reg("s21"), want, "{name}, lane {lane}");
        }
    }
}

#[test]
fn masks_counts_votes_and_lane_reads_of_a_wave_in_shared_memory_are_the_wave_s() {
    // The mask is the lanes whose number has bit 0 or bit 2: it leaves out the first lane.
    let chosen = |lane: u32| lane & 5 != 0;
    let before = vop3(op3("v_mbcnt_lo_u32_b32"), 5, [106, I_ZERO, 0], 0, 0);
    let above = vop3(op3("v_mbcnt_hi_u32_b32"), 5, [107, V + 5, 0], 0, 0);
    let far = vop3(op3("v_readlane_b32"), 28, [V + 2, I_ZERO + 37, 0], 0, 0);
    let mut words = lane_into_v2();
    words.extend([
        vop2(op(opcodes::VOP2, "v_and_b32"), 4, I_ZERO + 5, 2),
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO, 4),
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b64"), 20, 106),
        sop1(op(opcodes::SOP1, "s_ff1_i32_b64"), 22, 106),
        before[0],
        before[1],
        above[0],
        above[1],
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        // Only the mask's lanes run: the first of them is read, and a block is entered that
        // the wave enters where any lane runs, so every lane's scalar register is written.
        sop2(op(opcodes::SOP2, "s_and_b64"), 126, 126, 106),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 26, V + 2),
        sopp(op(opcodes::SOPP, "s_cbranch_execz"), 1),
        sop1(op(opcodes::SOP1, "s_mov_b32"), 27, I_ZERO + 7),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 24),
        far[0],
        far[1],
        S_ENDPGM,
    ]);
    for threads in [[8, 8, 1], [5, 3, 1]] {
        let out = translate(&compute(threads, words.clone())).unwrap();
        validate(&out.module);
        assert!(!out.subgroups);
        assert!(out.approximations.is_empty(), "{:?}", out.approximations);
        // A lane's number as the program computes it, eight threads to a row whatever the group's width.
        let count = threads[0] * threads[1];
        let number = |thread: u32| thread % threads[0] + 8 * (thread / threads[0]);
        let mask: Vec<bool> = (0..count).map(|thread| chosen(number(thread))).collect();
        let first = mask.iter().position(|&set| set).unwrap() as u32;
        for (thread, lane) in lanes(&out.module, threads).iter().enumerate() {
            let at = format!("thread {thread} of {count}");
            let set = |before: &[bool]| before.iter().filter(|&&set| set).count() as u32;
            assert_eq!(lane.reg("s20"), set(&mask), "{at}");
            assert_eq!(lane.reg("s22"), first, "{at}");
            assert_eq!(lane.reg("v5"), set(&mask[..thread]), "{at}");
            assert_eq!(lane.reg("s26"), number(first), "{at}");
            assert_eq!(lane.reg("s27"), 7, "{at}");
            let there = if count > 37 { number(37) } else { 0 };
            assert_eq!(lane.reg("s28"), there, "{at}");
        }
    }
}

#[test]
fn a_group_of_several_waves_and_a_caller_that_asks_keep_the_device_s_subgroup() {
    let mut words = lane_into_v2();
    words.extend(wave_fold("v_min_u32"));
    words.push(S_ENDPGM);
    // 128 threads are two waves of 64, which no barrier of the group keeps in step.
    let two = compute([16, 8, 1], words.clone());
    assert!(translate(&two).unwrap().subgroups);
    let asked = Options {
        wave: true,
        ..Options::default()
    };
    let one = compute([8, 8, 1], words.clone());
    assert!(translate_with(&one, &asked).unwrap().subgroups);
    assert!(!translate(&one).unwrap().subgroups);
    // A wave of 32 lanes is filled by a group of 32 threads: 64 are two of them.
    let narrow = Program {
        wave32: true,
        ..compute([8, 8, 1], words)
    };
    assert!(translate(&narrow).is_ok_and(|out| out.subgroups));
}

/// What a pixel program that kills the lanes whose input is under a half leaves in each lane
/// of a quad, by the instructions' definitions over the quad's four lanes: whether the lane
/// is killed, the register written while the killed lanes help, and the one written after.
fn quad(inputs: [f32; 4]) -> [(bool, f32, f32); 4] {
    let kill = inputs.map(|x| 0.5 > x);
    let live = kill.map(|killed| !killed);
    // Whole-quad mode: every lane of a quad that holds a live one.
    let helping = live.iter().any(|&l| l);
    std::array::from_fn(|lane| {
        let helped = if helping { inputs[lane] + 1.0 } else { 0.0 };
        (kill[lane], helped, if live[lane] { 1.0 } else { 0.0 })
    })
}

/// One pixel of the module run with `x` in every component of every input.
fn pixel(module: &naga::Module, x: f32) -> interp::Thread {
    let input = interp::Value::List(vec![interp::Value::F(x); 4]);
    let given = move |_: u32, binding: &naga::Binding| {
        matches!(binding, naga::Binding::Location { .. }).then(|| input.clone())
    };
    interp::run(module, [1, 1, 1], &given).remove(0)
}

#[test]
fn a_killed_pixel_lane_computes_on_for_its_quad_and_is_discarded_at_the_end() {
    let (exec, live, saved, helpers) = (126, 4, 8, 6);
    let last = exp(0, [6, 6, 6, 6]);
    let none = exp(9, [0, 0, 0, 0]);
    let words = vec![
        sop1(op(opcodes::SOP1, "s_mov_b64"), live, exec),
        sop1(op(opcodes::SOP1, "s_wqm_b64"), exec, exec),
        vintrp(1, 0, 0, 0, 1),
        sop1(op(opcodes::SOP1, "s_mov_b64"), saved, exec),
        // The lanes to kill run; they leave the live mask; where none is left the wave ends.
        vopc(V_CMPX_GT_F32, F_HALF, 0),
        sop2(op(opcodes::SOP2, "s_andn2_b64"), live, live, exec),
        sopp(S_CBRANCH_SCC0, 9),
        sop1(op(opcodes::SOP1, "s_wqm_b64"), helpers, live),
        sop2(op(opcodes::SOP2, "s_and_b64"), exec, saved, helpers),
        vop2(V_ADD_F32, 5, F_ONE, 0),
        sop2(op(opcodes::SOP2, "s_and_b64"), exec, exec, live),
        vop1(V_MOV, 6, F_ONE),
        last[0],
        last[1],
        S_ENDPGM,
        sop2(op(opcodes::SOP2, "s_and_b64"), exec, exec, live),
        none[0],
        none[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    for inputs in [
        [0.75, 0.25, 0.875, 0.125],
        [0.25, 0.25, 0.25, 0.75],
        [1.0; 4],
    ] {
        for (lane, (killed, helped, after)) in quad(inputs).into_iter().enumerate() {
            let ran = pixel(&out.module, inputs[lane]);
            assert_eq!(ran.killed, killed, "{inputs:?} lane {lane}");
            assert_eq!(ran.reg("v5"), helped.to_bits(), "{inputs:?} lane {lane}");
            assert_eq!(ran.reg("v6"), after.to_bits(), "{inputs:?} lane {lane}");
        }
    }
    // A quad with no live lane: the hardware computes nothing there and nothing reads what
    // a lane here computes, which is discarded like the rest of it.
    assert!(pixel(&out.module, 0.25).killed);
}

/// A mesh program whose threads each hold a value of their own in `v9`, read one another's
/// by fours as a subdividing program does, and place a vertex and a primitive of their number.
fn mesh_of_fours(inside: &[u32]) -> Vec<u32> {
    const SPREAD: u32 = 2_654_435_761;
    let low = vop3(op3("v_mbcnt_lo_u32_b32"), 0, [I_MINUS_ONE, I_ZERO, 0], 0, 0);
    let lane = vop3(op3("v_mbcnt_hi_u32_b32"), 0, [I_MINUS_ONE, V, 0], 0, 0);
    let spread = vop3(op3("v_mul_lo_u32"), 9, [V, LITERAL, 0], 0, 0);
    let (position, primitive) = (exp(POS0, [2, 2, 2, 2]), exp(20, [0, 0, 0, 0]));
    let mut words = vec![low[0], low[1], lane[0], lane[1]];
    words.extend([spread[0], spread[1], SPREAD]);
    words.extend_from_slice(inside);
    words.extend([vop1(V_MOV, 2, F_ONE), position[0], position[1]]);
    words.extend([primitive[0], primitive[1], S_ENDPGM]);
    words
}

#[test]
fn a_mesh_thread_reads_the_threads_of_its_four_by_running_the_body_for_them() {
    const SPREAD: u32 = 2_654_435_761;
    // v12, v7 and v8 = v9 as the first three threads of this four hold it; v10 their sum;
    // v11 = that sum as the second thread holds it, which hangs on the three reads before.
    let read = |dst, src, lane: u32| swizzle(dst, src, 28 | (lane << 5));
    let mut inside = Vec::new();
    for (dst, lane) in [(12, 0), (7, 1), (8, 2)] {
        inside.extend(read(dst, 9, lane));
    }
    inside.push(vop2(op(opcodes::VOP2, "v_add_u32"), 10, V + 12, 7));
    inside.push(vop2(op(opcodes::VOP2, "v_add_u32"), 10, V + 10, 8));
    inside.extend(read(11, 10, 1));
    let out = lowered(ShaderKind::Mesh, mesh_of_fours(&inside));
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    // The body runs for each of the three threads read, twice over since a value hangs on
    // two rounds of reads, then for the thread itself; and all of that for the primitive too.
    let entry = &out.module.entry_points[0].function.body;
    let calls = count(entry, &|s| matches!(s, naga::Statement::Call { .. }));
    assert_eq!(calls, 2 * (2 * 3 + 1));
    for thread in [0u32, 1, 2, 3, 6, 37, 63] {
        let ran = mesh_thread(&out.module, thread);
        let held = |lane: u32| ((thread & !3) | lane).wrapping_mul(SPREAD);
        let sum = held(0).wrapping_add(held(1)).wrapping_add(held(2));
        assert_eq!(
            ran.reg("v9"),
            thread.wrapping_mul(SPREAD),
            "thread {thread}"
        );
        let read = [ran.reg("v12"), ran.reg("v7"), ran.reg("v8")];
        assert_eq!(read, [held(0), held(1), held(2)], "thread {thread}");
        assert_eq!(ran.reg("v11"), sum, "thread {thread}");
    }
}

/// One vertex of a mesh module: the first corner of primitive `thread`, which is the vertex
/// that thread placed.
fn mesh_thread(module: &naga::Module, thread: u32) -> interp::Thread {
    let given = move |_: u32, binding: &naga::Binding| match binding {
        naga::Binding::BuiltIn(naga::BuiltIn::VertexIndex) => Some(interp::Value::U(3 * thread)),
        _ => None,
    };
    interp::run(module, [1, 1, 1], &given).remove(0)
}

#[test]
fn a_mesh_thread_reads_a_thread_that_jumped_past_the_read() {
    const SPREAD: u32 = 2_654_435_761;
    // The threads whose value read as a float is under a half run on into a block the
    // others jump past; there each reads the first thread of its four, which may have jumped.
    let read = swizzle(12, 9, 28);
    let inside = [
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        vopc(V_CMPX_GT_F32, F_HALF, 9),
        sopp(op(opcodes::SOPP, "s_cbranch_execz"), 2),
        read[0],
        read[1],
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 24),
    ];
    let out = lowered(ShaderKind::Mesh, mesh_of_fours(&inside));
    let held = |thread: u32| thread.wrapping_mul(SPREAD);
    let enters = |thread: u32| 0.5 > f32::from_bits(held(thread));
    // Some thread reads a first thread that jumped, and some thread jumps itself.
    assert!((0..64).any(|thread| enters(thread) && !enters(thread & !3)));
    for thread in 0..64 {
        let ran = mesh_thread(&out.module, thread);
        let want = if enters(thread) { held(thread & !3) } else { 0 };
        assert_eq!(ran.reg("v12"), want, "thread {thread}");
    }
}

#[test]
fn a_read_of_another_mesh_thread_in_a_loop_is_refused_by_name() {
    // The read, then back to it while a count is not spent.
    let read = swizzle(12, 9, 28);
    let inside = [
        sopk(S_MOVK, 30, 3),
        read[0],
        read[1],
        sop2(S_ADD_I32, 30, 30, I_MINUS_ONE),
        sopc(S_CMP_LG_U32, 30, I_ZERO),
        sopp(S_CBRANCH_SCC1, 0xFFFB),
    ];
    let refused = refusal(ShaderKind::Mesh, mesh_of_fours(&inside));
    let RdnaError::NotTranslated(NotTranslated::Pipeline(what)) = refused else {
        panic!("{refused:?}");
    };
    assert!(what.starts_with("another thread's value at a place the program comes to more"));
}

#[test]
fn a_wave_of_32_lanes_finds_its_first_lane_and_counts_its_mask_in_one_dword() {
    // The mask leaves out the first lane; s21 = its first lane, s22 = v2 there, s23 = its count.
    let chosen = |lane: u32| lane & 5 != 0;
    let there = vop3(op3("v_readlane_b32"), 22, [V + 2, 21, 0], 0, 0);
    let mut words = lane_into_v2();
    words.extend([
        vop2(op(opcodes::VOP2, "v_and_b32"), 4, I_ZERO + 5, 2),
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO, 4),
        sop2(op(opcodes::SOP2, "s_and_b32"), 20, 106, 126),
        sop1(op(opcodes::SOP1, "s_ff1_i32_b32"), 21, 20),
        there[0],
        there[1],
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b32"), 23, 20),
        // A dword that is no mask is counted as the number it is.
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b32"), 24, I_ZERO + 7),
        S_ENDPGM,
    ]);
    let narrow = Program {
        wave32: true,
        ..compute([8, 4, 1], words)
    };
    let out = translate(&narrow).unwrap();
    validate(&out.module);
    assert!(!out.subgroups);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let first = (0..32).find(|&lane| chosen(lane)).unwrap();
    let count = (0..32).filter(|&lane| chosen(lane)).count() as u32;
    for (lane, thread) in lanes(&out.module, [8, 4, 1]).iter().enumerate() {
        let read = [21, 22, 23, 24].map(|r| thread.reg(&format!("s{r}")));
        assert_eq!(read, [first, first, count, 3], "lane {lane}");
    }
}

#[test]
fn a_16_bit_result_is_rounded_as_the_float_mode_the_program_wrote_says() {
    // 1 + 3/2048 lies midway between two 16-bit floats: toward zero it is the lower, to the
    // nearest the even one above. v1 = v0 narrowed after the mode's bits 2 and 3 are set,
    // v2 after they are cleared, v3 = the reciprocal root of the half in v4 while they are
    // set again.
    let tie = (1.0f32 + 3.0 / 2048.0).to_bits();
    let mode = |value: u32| {
        [
            sopk(op(opcodes::SOPK, "s_setreg_imm32_b32"), 0, 2177),
            value,
        ]
    };
    let narrowed = |dst| vop1(op(opcodes::VOP1, "v_cvt_f16_f32"), dst, V);
    let position = exp(POS0, [1, 1, 1, 1]);
    let mut words = vec![vop1(V_MOV, 0, LITERAL), tie];
    words.extend(mode(3));
    words.push(narrowed(1));
    words.extend(mode(0));
    words.push(narrowed(2));
    words.extend(mode(3));
    words.extend([vop1(V_MOV, 4, LITERAL), 0x3C1B]);
    words.push(vop1(op(opcodes::VOP1, "v_rsq_f16"), 3, V + 4));
    words.extend([position[0], position[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Vertex, words);
    // Each write of the rounding bits is followed, so none is left unmodelled.
    assert!(!(out.approximations.keys()).any(|note| note.contains("float mode")));
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!([ran.reg("v1"), ran.reg("v2")], [0x3C01, 0x3C02]);
    // A function's result is rounded to the nearest whatever the mode: this one lies past
    // the middle of two 16-bit floats, so toward zero it would be the lower.
    let root = 1.0 / (1.0f32 + 27.0 / 1024.0).sqrt();
    let nearest = u32::from(to_naga::half_bits(root));
    let worth = |half: u32| (1024 + (half & 0x3FF)) as f32 * 2f32.powi((half >> 10) as i32 - 25);
    assert!(worth(nearest) > root);
    assert_eq!(ran.reg("v3"), nearest);
}
