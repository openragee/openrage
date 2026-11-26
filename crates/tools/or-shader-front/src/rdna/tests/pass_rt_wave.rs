//! A program that tests nodes and is not a lane an invocation, lowered with its wave kept whole:
//! a lane mask is the wave's own number in every lane, a vector register is read and written at
//! a lane's number, and the lanes meet before an instruction of the memory their group shares.

use naga::Statement as S;

use super::exact::{entered_twice, switches, DISPATCHED};
use super::pass_rt::{test_words, MODES, WIDE};
use super::*;

/// A compute program of one wave of 32 lanes, a thread a lane, that tests a node last.
pub(super) fn wave_of_32(threads: u32, shared_dwords: u32, mut words: Vec<u32>) -> Program {
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    Program {
        wave32: true,
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [threads, 1, 1],
            shared_dwords,
        }),
        ..program(ShaderKind::Compute, words)
    }
}

pub(super) fn whole(program: &Program) -> Lowered {
    let options = Options {
        arena: true,
        ..Options::default()
    };
    let lowered = translate_with(program, &options).unwrap();
    validate(&lowered.module);
    assert!(!lowered.subgroups);
    assert!(
        lowered.approximations.is_empty(),
        "{:?}",
        lowered.approximations
    );
    lowered
}

/// `v<dst>` = this lane's number.
pub(super) fn lane_into(dst: u32) -> [u32; 2] {
    vop3(
        op3("v_mbcnt_lo_u32_b32"),
        dst,
        [I_MINUS_ONE, I_ZERO, 0],
        0,
        0,
    )
}

pub(super) fn s_mov(dst: u32, src: u32) -> u32 {
    sop1(op(opcodes::SOP1, "s_mov_b32"), dst, src)
}

pub(super) const EXEC_LO: u32 = 126;
const VCC_LO: u32 = 106;

#[test]
fn a_lane_mask_is_the_wave_s_own_number_in_every_lane() {
    // Read at the first lane: fifty more than a lane's number, so what the wave's first lane
    // holds is told from a dword nothing wrote.
    let first = |dst: u32| vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), dst, V + 19);
    let before = vop3(op3("v_mbcnt_lo_u32_b32"), 23, [20, I_ZERO, 0], 0, 0);
    let chosen = vop3(
        0x100 + op(opcodes::VOP2, "v_cndmask_b32"),
        31,
        [I_ONE, I_ZERO + 2, 20],
        0,
        0,
    );
    let mut words = lane_into(20).to_vec();
    words.extend([
        vop2(op(opcodes::VOP2, "v_add_u32"), 19, I_ZERO + 50, 20),
        // The lanes past the eighth, as a mask: kept, counted, anded with each lane's own bit
        // and moved into a vector register.
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO + 7, 20),
        s_mov(20, VCC_LO),
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b32"), 21, 20),
        vop1(V_MOV, 26, I_ONE),
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 27, V + 20, 26),
        vop2(op(opcodes::VOP2, "v_and_b32"), 22, 20, 27),
        vop1(V_MOV, 21, 20),
        before[0],
        before[1],
        // A choice by the mask is by the lane's own bit of it.
        chosen[0],
        chosen[1],
        // The first lane that runs once the first five are off, then with none left.
        vopc(op(opcodes::VOPC, "v_cmpx_lt_u32"), I_ZERO + 4, 20),
        first(23),
        s_mov(24, EXEC_LO),
        // A comparison that holds in lanes that do not run leaves none of them in its mask.
        vopc(op(opcodes::VOPC, "v_cmp_gt_u32"), I_ZERO + 10, 20),
        s_mov(29, VCC_LO),
        vopc(op(opcodes::VOPC, "v_cmpx_lt_u32"), I_ZERO + 40, 20),
        first(25),
        s_mov(28, EXEC_LO),
        s_mov(EXEC_LO, I_MINUS_ONE),
    ]);
    let out = whole(&wave_of_32(32, 0, words));
    let ran = interp::run(&out.module, [32, 1, 1], &|_, _| None);
    assert_eq!(ran.len(), 32);
    const PAST: u32 = 0xFFFF_FF00;
    for (thread, lane) in ran.iter().enumerate() {
        let thread = thread as u32;
        let scalars = ["s20", "s21", "s23", "s24", "s29", "s25", "s28"].map(|name| lane.reg(name));
        assert_eq!(
            scalars,
            [PAST, 24, 55, 0xFFFF_FFE0, 0x3E0, 50, 0],
            "lane {thread}"
        );
        let vectors = ["v21", "v22", "v23", "v31"].map(|name| lane.reg(name));
        let own = (1 << thread) & PAST;
        assert_eq!(
            vectors,
            [
                PAST,
                own,
                thread.saturating_sub(8),
                1 + u32::from(thread > 7)
            ],
            "lane {thread}"
        );
    }
}

#[test]
fn a_register_a_program_keeps_a_table_in_is_a_vector_register_still() {
    let write = |value: u32, lane: u32| vop3(op3("v_writelane_b32"), 5, [value, lane, 0], 0, 0);
    let read = |dst: u32, lane: u32| vop3(op3("v_readlane_b32"), dst, [V + 5, lane, 0], 0, 0);
    let mut words = lane_into(20).to_vec();
    words.extend([
        // Every lane its own number, then 60 in the first sixteen alone.
        vop1(V_MOV, 5, V + 20),
        s_mov(24, EXEC_LO),
        vopc(op(opcodes::VOPC, "v_cmpx_gt_u32"), I_ZERO + 16, 20),
        vop1(V_MOV, 5, I_ZERO + 60),
    ]);
    // A lane that does not run is written by its number all the same.
    words.extend(write(I_ZERO + 55, I_ZERO + 20));
    words.extend([
        s_mov(EXEC_LO, 24),
        sopk(S_MOVK, 11, 9),
        sopk(S_MOVK, 12, 42),
        sopk(S_MOVK, 19, 41),
    ]);
    words.extend(write(12, 11));
    words.extend(write(I_ZERO + 33, I_ZERO + 3));
    for (dst, lane) in [
        (13, I_ZERO + 20),
        (14, I_ZERO + 17),
        (15, 11),
        (16, I_ZERO + 3),
        (17, I_ZERO + 5),
        // A lane's number is its low five bits.
        (18, 19),
    ] {
        words.extend(read(dst, lane));
    }
    words.push(vop1(V_MOV, 30, V + 5));
    let out = whole(&wave_of_32(32, 0, words));
    let ran = interp::run(&out.module, [32, 1, 1], &|_, _| None);
    for (thread, lane) in ran.iter().enumerate() {
        let scalars = ["s13", "s14", "s15", "s16", "s17", "s18"].map(|name| lane.reg(name));
        assert_eq!(scalars, [55, 17, 42, 33, 60, 42], "lane {thread}");
        let held = match thread as u32 {
            3 => 33,
            9 => 42,
            20 => 55,
            low if low < 16 => 60,
            high => high,
        };
        assert_eq!(lane.reg("v30"), held, "lane {thread}");
    }
    // A lane the group has no thread for is no lane to read: eight threads have no twentieth.
    let mut words = lane_into(20).to_vec();
    words.extend(read(13, I_ZERO + 20));
    let options = Options {
        arena: true,
        ..Options::default()
    };
    let Err(RdnaError::NotTranslated(why)) = translate_with(&wave_of_32(8, 0, words), &options)
    else {
        panic!("a lane no thread is was read");
    };
    assert_eq!(
        why.name(),
        "v_readlane_b32: a lane by its number that no thread of the group is"
    );
}

#[test]
fn a_count_of_a_mask_another_block_saved_is_the_wave_s_and_not_a_lane_s() {
    // The lanes that run, kept in a register pair; then, in a block of its own, the count of
    // its low half and the first lane of its high half.
    let mut words = vec![
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, EXEC_LO),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 0),
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b32"), 26, 24),
        sop1(op(opcodes::SOP1, "s_ff1_i32_b32"), 27, 25),
    ];
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    let out = pass_rt::traced(words).unwrap();
    let ran = interp::run(&out.module, [8, 8, 1], &|_, _| None);
    assert_eq!(ran.len(), 64);
    for (thread, lane) in ran.iter().enumerate() {
        assert_eq!([lane.reg("s26"), lane.reg("s27")], [32, 0], "lane {thread}");
    }
}

/// Whether `module` holds a variable of every thread or of its group named `name`.
pub(super) fn holds_global(module: &naga::Module, name: &str) -> bool {
    (module.global_variables.iter()).any(|(_, global)| global.name.as_deref() == Some(name))
}

#[test]
fn a_count_a_first_bit_and_a_last_bit_of_a_saved_mask_each_end_a_lane_an_invocation() {
    // The lanes that run, kept in a register pair, and one instruction over a half of it in a
    // block of its own. A lane alone holds 1 in the low half and nought in the high, which
    // these would make 1, no bit and 31.
    for (name, half, wave) in [
        ("s_bcnt1_i32_b32", 24, 32),
        ("s_ff1_i32_b32", 25, 0),
        ("s_flbit_i32_b32", 24, 0),
    ] {
        let mut words = vec![
            sop1(op(opcodes::SOP1, "s_mov_b64"), 24, EXEC_LO),
            sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
            sopp(S_CBRANCH_SCC1, 0),
            sop1(op(opcodes::SOP1, name), 26, half),
        ];
        words.extend(test_words(WIDE, I_ZERO, MODES));
        words.push(S_ENDPGM);
        let out = pass_rt::traced(words).unwrap();
        assert!(holds_global(&out.module, "wave_cells"), "{name}");
        let ran = interp::run(&out.module, [8, 8, 1], &|_, _| None);
        for (thread, lane) in ran.iter().enumerate() {
            assert_eq!(lane.reg("s26"), wave, "{name}, lane {thread}");
        }
    }
}

#[test]
fn a_group_that_does_not_fill_its_wave_starts_with_the_lanes_it_has_threads_for() {
    // Eight threads of a wave of 32: the lanes that run are those eight, and eight is their count.
    let words = vec![
        s_mov(24, EXEC_LO),
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b32"), 25, EXEC_LO),
    ];
    let out = whole(&wave_of_32(8, 0, words));
    assert!(holds_global(&out.module, "wave_cells"));
    let ran = interp::run(&out.module, [8, 1, 1], &|_, _| None);
    assert_eq!(ran.len(), 8);
    for (thread, lane) in ran.iter().enumerate() {
        let held = [lane.reg("s24"), lane.reg("s25")];
        assert_eq!(held, [0xFF, 8], "lane {thread}");
    }
}

#[test]
fn a_lane_the_group_has_no_thread_for_never_runs_whatever_a_program_moves_into_the_lane_mask() {
    // Eight threads of a wave of 32, each lane's number and fifty in `v19`. Every lane is
    // moved into the lane mask, then the third and those past the eighth, then those past the
    // eighth alone, then the complement of a number: the lanes that run are of the eight.
    let first = |dst: u32| vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), dst, V + 19);
    let mut words = lane_into(20).to_vec();
    words.extend([
        vop2(op(opcodes::VOP2, "v_add_u32"), 19, I_ZERO + 50, 20),
        s_mov(EXEC_LO, I_MINUS_ONE),
        s_mov(24, EXEC_LO),
        s_mov(EXEC_LO, LITERAL),
        0xFFFF_FF04,
        s_mov(25, EXEC_LO),
        first(26),
        // No lane that runs is left, and the first lane read is the wave's first: a lane past
        // the eighth is no thread's, and nothing would hold what it is read for.
        s_mov(EXEC_LO, LITERAL),
        0xFFFF_FF00,
        s_mov(27, EXEC_LO),
        first(28),
        sopk(S_MOVK, 29, 0xF0),
        sop1(op(opcodes::SOP1, "s_not_b32"), EXEC_LO, 29),
        s_mov(30, EXEC_LO),
        s_mov(EXEC_LO, 24),
    ]);
    let out = whole(&wave_of_32(8, 0, words));
    let ran = interp::run(&out.module, [8, 1, 1], &|_, _| None);
    assert_eq!(ran.len(), 8);
    for (thread, lane) in ran.iter().enumerate() {
        let held = ["s24", "s25", "s26", "s27", "s28", "s30"].map(|name| lane.reg(name));
        assert_eq!(held, [0xFF, 4, 52, 0, 50, 0x0F], "lane {thread}");
    }
}

#[test]
fn a_lane_a_program_computes_is_refused_in_a_group_that_does_not_fill_its_wave() {
    let options = Options {
        arena: true,
        ..Options::default()
    };
    let read = vop3(op3("v_readlane_b32"), 13, [V + 5, 11, 0], 0, 0);
    let write = vop3(op3("v_writelane_b32"), 5, [I_ZERO + 33, 11, 0], 0, 0);
    for (name, at) in [("v_readlane_b32", read), ("v_writelane_b32", write)] {
        let mut words = lane_into(20).to_vec();
        words.extend([vop1(V_MOV, 5, V + 20), sopk(S_MOVK, 11, 3)]);
        words.extend(at);
        // Eight threads of 32 lanes: a number a run computes may be of a lane no thread is.
        let partial = wave_of_32(8, 0, words.clone());
        let Err(RdnaError::NotTranslated(why)) = translate_with(&partial, &options) else {
            panic!("{name} at a lane a run computes, in a group of eight threads");
        };
        let refused = format!("{name}: a lane by its number that no thread of the group is");
        assert_eq!(why.name(), refused);
        // A group that fills its wave has a thread for whatever lane the number names.
        whole(&wave_of_32(32, 0, words));
    }
}

#[test]
fn a_wave_kept_whole_folds_and_permutes_through_the_exchange_between_its_meetings() {
    // Two lanes. A mask gathered, a sum over the wave, a lane read by its number, a
    // permutation, a mask again and a first-lane read, each straight after the other: the sum
    // and the permutation go through the wave's exchange, the rest through the wave's own
    // dwords, and every one of them counts its meeting on the one counter that picks a half.
    let fold = vop2_with("v_add_u32", 1, 233, 1, 1 | (PAIR_SWAP << 8));
    let there = vop3(op3("v_readlane_b32"), 13, [V + 3, I_ONE, 0], 0, 0);
    let mut words = lane_into(20).to_vec();
    words.extend([
        vop2(op(opcodes::VOP2, "v_add_u32"), 1, I_ZERO + 5, 20),
        // The lanes past the first: the second alone.
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO, 20),
        s_mov(20, VCC_LO),
    ]);
    // v1 = 5 + 6, in both lanes.
    words.extend(fold);
    words.extend([
        vop1(V_MOV, 22, V + 1),
        vop2(op(opcodes::VOP2, "v_add_u32"), 3, I_ZERO + 60, 20),
    ]);
    words.extend(there);
    words.extend([
        // Each lane takes the other's v1: the address it gives is four bytes a lane.
        vop2(op(opcodes::VOP2, "v_xor_b32"), 0, I_ONE, 20),
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 0, I_ZERO + 2, 0),
        vop1(V_MOV, 1, V + 3),
    ]);
    words.extend(ds("ds_bpermute_b32", 0));
    words.extend([
        vop1(V_MOV, 23, V + 1),
        // The lanes before the second: the first alone.
        vopc(op(opcodes::VOPC, "v_cmp_gt_u32"), I_ONE, 20),
        s_mov(21, VCC_LO),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 14, V + 23),
    ]);
    let out = whole(&wave_of_32(2, 0, words));
    for name in ["wave_cells", "wave_values"] {
        assert!(holds_global(&out.module, name), "{name}");
    }
    // A lane at a time from one meeting to the next, in both orders.
    for order in [[0, 1], [1, 0]] {
        let (ran, _) =
            interp::run_in_order(&out.module, [2, 1, 1], &|_, _| None, &[], Some(&order));
        for (thread, lane) in ran.iter().enumerate() {
            let at = format!("lane {thread}, order {order:?}");
            let scalars = ["s20", "s13", "s21", "s14"].map(|name| lane.reg(name));
            assert_eq!(scalars, [2, 61, 1, 61], "{at}");
            let vectors = [lane.reg("v22"), lane.reg("v23")];
            assert_eq!(vectors, [11, 61 - thread as u32], "{at}");
        }
    }
}

/// The barriers among the statements of the module's body, those of its blocks too.
pub(super) fn barriers(module: &naga::Module) -> usize {
    let (_, body) = (module.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("main_body"))
        .unwrap();
    count(&body.body, &|s| matches!(s, S::ControlBarrier(_)))
}

#[test]
fn the_lanes_meet_before_an_instruction_of_the_group_s_memory_that_takes_an_order() {
    // The first lane's value read first: the lanes hold something of their wave, and the
    // group has met where the first instruction of its memory stands.
    let first = vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 30, V + 20);
    let start = || {
        let mut words: Vec<u32> = (0..12).map(|v| vop1(V_MOV, v, F_ONE)).collect();
        words.push(first);
        words
    };
    let with = |memory: &[&str]| {
        let mut words = start();
        words.extend(memory.iter().flat_map(|name| ds(name, 4)));
        barriers(&whole(&wave_of_32(32, 64, words)).module)
    };
    // A store and the load after it; the four instructions a walk keeps its stack with.
    assert_eq!(with(&["ds_write_b32", "ds_read_b32"]), 1);
    let stack = [
        "ds_write_b32",
        "ds_wrxchg_rtn_b32",
        "ds_write2_b32",
        "ds_read2_b32",
    ];
    assert_eq!(with(&stack), 3);
    // Two loads take no order, nor two updates by one function; two by different ones do.
    assert_eq!(with(&["ds_read_b32", "ds_read_b32"]), 0);
    assert_eq!(with(&["ds_add_u32", "ds_add_u32"]), 0);
    assert_eq!(with(&["ds_add_u32", "ds_min_u32"]), 1);
    assert_eq!(with(&["ds_write_b32", "ds_write_b32"]), 1);
    // Nor do two exchanges: each gives back what the one before it left.
    assert_eq!(with(&["ds_wrxchg_rtn_b32", "ds_wrxchg_rtn_b32"]), 1);
    // A meeting the program makes itself between two of them is the one they need.
    let mut words = start();
    words.extend(ds("ds_write_b32", 4));
    words.push(vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO + 7, 20));
    words.extend(ds("ds_read_b32", 4));
    assert_eq!(barriers(&whole(&wave_of_32(32, 64, words)).module), 0);
    // And where a block starts nothing says the group has met.
    let mut words = start();
    words.extend([sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO), sopp(S_CBRANCH_SCC1, 0)]);
    words.extend(ds("ds_read_b32", 4));
    assert_eq!(barriers(&whole(&wave_of_32(32, 64, words)).module), 1);
}

#[test]
fn a_meeting_does_not_disturb_a_lane_still_reading_the_one_before_it() {
    // Two lane reads, then two masks, each straight after the other: the lane that is first
    // out of a meeting writes the next before the others have read the last.
    let read = |dst: u32, lane: u32| vop3(op3("v_readlane_b32"), dst, [V + 5, lane, 0], 0, 0);
    let mut words = lane_into(20).to_vec();
    words.push(vop2(op(opcodes::VOP2, "v_add_u32"), 5, I_ZERO + 50, 20));
    words.extend(read(13, I_ZERO + 3));
    words.extend(read(14, I_ZERO));
    words.extend([
        vopc(op(opcodes::VOPC, "v_cmp_gt_u32"), I_ZERO + 8, 20),
        s_mov(20, VCC_LO),
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO + 7, 20),
        s_mov(21, VCC_LO),
    ]);
    let out = whole(&wave_of_32(32, 0, words));
    let forward: Vec<u32> = (0..32).collect();
    let backward: Vec<u32> = (0..32).rev().collect();
    for order in [forward, backward] {
        let (ran, _) =
            interp::run_in_order(&out.module, [32, 1, 1], &|_, _| None, &[], Some(&order));
        for (thread, lane) in ran.iter().enumerate() {
            let scalars = ["s13", "s14", "s20", "s21"].map(|name| lane.reg(name));
            assert_eq!(scalars, [53, 50, 0xFF, 0xFFFF_FF00], "lane {thread}");
        }
    }
}

#[test]
fn a_wave_of_64_lanes_keeps_its_mask_in_two_dwords() {
    let low = lane_into(20);
    let lane = vop3(
        op3("v_mbcnt_hi_u32_b32"),
        20,
        [I_MINUS_ONE, V + 20, 0],
        0,
        0,
    );
    let before = vop3(op3("v_mbcnt_hi_u32_b32"), 25, [27, I_ZERO, 0], 0, 0);
    let chosen = vop3(
        0x100 + op(opcodes::VOP2, "v_cndmask_b32"),
        31,
        [I_ONE, I_ZERO + 2, 26],
        0,
        0,
    );
    let s_mov_b64 = |dst: u32, src: u32| sop1(op(opcodes::SOP1, "s_mov_b64"), dst, src);
    let mut words = vec![low[0], low[1], lane[0], lane[1]];
    words.extend([
        // The lanes past the forty-first: none of the low half.
        s_mov_b64(24, EXEC_LO),
        vopc(op(opcodes::VOPC, "v_cmpx_lt_u32"), I_ZERO + 40, 20),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 22, V + 20),
        s_mov_b64(26, EXEC_LO),
        vop1(V_MOV, 23, I_ONE),
        s_mov_b64(EXEC_LO, 24),
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b64"), 28, 26),
        before[0],
        before[1],
        chosen[0],
        chosen[1],
    ]);
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    let out = pass_rt::traced(words).unwrap();
    let ran = interp::run(&out.module, [8, 8, 1], &|_, _| None);
    assert_eq!(ran.len(), 64);
    for (thread, lane) in ran.iter().enumerate() {
        let thread = thread as u32;
        let scalars = ["s22", "s26", "s27", "s28"].map(|name| lane.reg(name));
        assert_eq!(scalars, [41, 0, 0xFFFF_FE00, 23], "lane {thread}");
        let vectors = [lane.reg("v23"), lane.reg("v25"), lane.reg("v31")];
        let past = u32::from(thread > 40);
        assert_eq!(
            vectors,
            [past, thread.saturating_sub(41), 1 + past],
            "lane {thread}"
        );
    }
}

#[test]
fn a_program_that_tests_nodes_in_a_group_of_several_waves_is_refused_by_name() {
    // Two waves of 64 lanes: each would keep a lane mask as its lanes' own bits.
    let mut words = vec![vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 22, V + 20)];
    words.extend(test_words(WIDE, I_ZERO, MODES));
    words.push(S_ENDPGM);
    let program = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [16, 8, 1],
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, words)
    };
    let options = Options {
        arena: true,
        ..Options::default()
    };
    let Err(RdnaError::NotTranslated(why)) = translate_with(&program, &options) else {
        panic!("a program that tests nodes was lowered for two waves");
    };
    assert_eq!(
        why.name(),
        "a program that tests nodes in a group that is not one wave"
    );
}

#[test]
fn a_wave_kept_whole_meets_under_an_if_where_its_graph_does_not_nest() {
    // The lanes past the eighth, gathered and counted in a block of a loop entered at two
    // blocks: the meeting is in a function the block calls.
    let inside = [
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO + 7, 20),
        s_mov(21, VCC_LO),
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b32"), 22, 21),
    ];
    let words = entered_twice(&lane_into(20), &inside);
    let options = Options {
        arena: true,
        ..Options::default()
    };
    let out = translate_with(&wave_of_32(32, 0, words), &options).unwrap();
    validate(&out.module);
    assert!(holds_global(&out.module, "wave_cells"));
    assert!(out.approximations.contains_key(DISPATCHED));
    assert_eq!(switches(&out.module), 0);
    let ran = interp::run(&out.module, [32, 1, 1], &|_, _| None);
    for (thread, lane) in ran.iter().enumerate() {
        let held = [lane.reg("s21"), lane.reg("s22"), lane.reg("v4")];
        assert_eq!(held, [0xFFFF_FF00, 24, ONE], "lane {thread}");
    }
}
