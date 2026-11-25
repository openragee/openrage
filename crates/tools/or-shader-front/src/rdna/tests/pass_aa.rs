//! The loops of a program whose end the lowered code does not show, and the mask of running
//! lanes kept to the lane a module runs.

use super::*;

const S_MOV_B64: &str = "s_mov_b64";
const EXEC: u32 = 126;
const VCC: u32 = 106;

fn s1(name: &str, dst: u32, src: u32) -> u32 {
    sop1(op(opcodes::SOP1, name), dst, src)
}

fn s2(name: &str, dst: u32, src0: u32, src1: u32) -> u32 {
    sop2(op(opcodes::SOP2, name), dst, src0, src1)
}

/// A vertex program of `words`, then its position.
fn vertex(mut words: Vec<u32>) -> Lowered {
    let position = exp(POS0, [0, 0, 0, 0]);
    words.extend([position[0], position[1], S_ENDPGM]);
    lowered(ShaderKind::Vertex, words)
}

/// A program that sets every lane of its wave running, then takes the lanes in turns: those
/// that hold the first running lane's `v1` run a turn, which counts in `v2`, and leave the
/// lanes still to take; it goes round while any is left. A program picks a texture by a
/// number that differs a lane with so.
fn lanes_in_turns() -> Vec<u32> {
    vec![
        s1(S_MOV_B64, EXEC, I_MINUS_ONE),
        vop1(V_MOV, 2, I_ZERO),
        vop1(V_MOV, 1, F_ONE),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V + 1),
        vopc(op(opcodes::VOPC, "v_cmp_eq_u32"), 20, 1),
        s1("s_and_saveexec_b64", 24, VCC),
        vop2(op(opcodes::VOP2, "v_add_u32"), 2, I_ONE, 2),
        s2("s_xor_b64", EXEC, EXEC, 24),
        // Back to the read of the first lane at word 3.
        sopp(op(opcodes::SOPP, "s_cbranch_execnz"), 0xFFFA),
    ]
}

#[test]
fn a_loop_over_a_wave_s_lanes_takes_this_lane_s_turn_and_ends() {
    let out = vertex(lanes_in_turns());
    // The machine stops a loop at 100,000 turns: with the 63 other lanes left in the mask, which
    // nothing a module runs takes out, this one went round for ever.
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!(ran.reg("v2"), 1);
    assert_eq!([ran.reg("exec_lo"), ran.reg("exec_hi")], [0, 0]);
    assert_eq!(out.loops_not_shown_to_end(), 0);
    assert!(LOOP_OTHER_LANES.starts_with(LOOP_NOT_SHOWN));
    assert!(LOOP_UNCHANGING.starts_with(LOOP_NOT_SHOWN));
}

#[test]
fn the_mask_of_running_lanes_holds_this_lane_whatever_is_stored_there() {
    // Every lane set running, the lanes that ran taken out, and the rest put back.
    let words = vec![
        s1(S_MOV_B64, EXEC, I_MINUS_ONE),
        s1(S_MOV_B64, 24, EXEC),
        vop1(V_MOV, 0, F_ONE),
        s2("s_andn2_b64", 26, 24, EXEC),
        s1("s_not_b64", EXEC, EXEC),
        s1(S_MOV_B64, 28, EXEC),
        s1(S_MOV_B64, EXEC, 24),
    ];
    let ran = interp::run(&vertex(words).module, [1, 1, 1], &|_, _| None).remove(0);
    let pair = |r: u32| [ran.reg(&format!("s{r}")), ran.reg(&format!("s{}", r + 1))];
    assert_eq!([pair(24), pair(26), pair(28)], [[1, 0], [0, 0], [0, 0]]);
    assert_eq!([ran.reg("exec_lo"), ran.reg("exec_hi")], [1, 0]);
}

#[test]
fn a_loop_is_shown_to_end_by_an_exit_a_turn_changes() {
    // A count down to nought: the exit hangs on a register a turn makes of itself.
    let counted = vec![
        sopk(S_MOVK, 0, 3),
        vop1(V_MOV, 0, F_ONE),
        sop2(S_ADD_I32, 0, 0, I_MINUS_ONE),
        sopc(S_CMP_LG_U32, 0, I_ZERO),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
    ];
    assert_eq!(vertex(counted).loops_not_shown_to_end(), 0);
    // Two registers no turn writes, compared: a turn that goes round once goes round for ever.
    let same = vec![
        sopk(S_MOVK, 0, 3),
        sopk(S_MOVK, 1, 3),
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_EQ_U32, 0, 1),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
    ];
    let out = vertex(same);
    assert_eq!(out.approximations.get(LOOP_UNCHANGING), Some(&1));
    // The same with a register a turn only sets to what it was set to before, in a block
    // before the one that compares it.
    let set = vec![
        sopk(S_MOVK, 1, 3),
        sopk(S_MOVK, 0, 3),
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_EQ_U32, 1, 1),
        sopp(S_CBRANCH_SCC1, 1),
        sopk(S_MOVK, 5, 1),
        sopc(S_CMP_EQ_U32, 0, 1),
        sopp(S_CBRANCH_SCC1, 0xFFF9),
    ];
    assert_eq!(vertex(set).approximations.get(LOOP_UNCHANGING), Some(&1));
    // A mask of lanes the program states, tested whole: no lane the lowering runs empties it.
    let stated = vec![
        s1(S_MOV_B64, 24, I_MINUS_ONE),
        vop1(V_MOV, 0, F_ONE),
        s2("s_andn2_b64", 24, 24, EXEC),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
    ];
    assert_eq!(
        vertex(stated).approximations.get(LOOP_OTHER_LANES),
        Some(&1)
    );
    // The lanes that ran taken from a copy of the mask of running lanes: this lane's own bit.
    let own = vec![
        s1(S_MOV_B64, 24, EXEC),
        vop1(V_MOV, 0, F_ONE),
        s2("s_andn2_b64", 24, 24, EXEC),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
    ];
    assert_eq!(vertex(own).loops_not_shown_to_end(), 0);
}

#[test]
fn a_pixel_loop_that_takes_its_lanes_out_as_they_finish_ends() {
    // `s24` holds the lanes still going round; a lane is done at its second turn, counted in
    // `v2`, and the loop goes round while a lane is left. A kill takes lanes out of the live
    // ones by the same instruction, and is told from this by where its branch goes.
    let target = exp(0, [2, 2, 2, 2]);
    let words = vec![
        s1(S_MOV_B64, 24, EXEC),
        vop1(V_MOV, 2, I_ZERO),
        vop2(op(opcodes::VOP2, "v_add_u32"), 2, I_ONE, 2),
        vopc(op(opcodes::VOPC, "v_cmp_le_u32"), I_ZERO + 2, 2),
        s2("s_and_b64", EXEC, EXEC, VCC),
        s2("s_andn2_b64", 24, 24, EXEC),
        s1(S_MOV_B64, EXEC, 24),
        sopp(S_CBRANCH_SCC1, 0xFFFA),
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(out.loops_not_shown_to_end(), 0);
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!(ran.reg("v2"), 2);
    // A kill: with no lane left the program goes to an end that computes nothing. The lane
    // killed runs on, as the helper of its quad it may be, and writes `v3`.
    let end = exp(0, [3, 3, 3, 3]);
    let kill = vec![
        s1(S_MOV_B64, 24, EXEC),
        s2("s_andn2_b64", 24, 24, EXEC),
        sopp(S_CBRANCH_SCC0, 4),
        vop1(V_MOV, 3, F_ONE),
        end[0],
        end[1],
        S_ENDPGM,
        end[0],
        end[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, kill);
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!([ran.reg("s24"), ran.reg("v3")], [0, ONE]);
}

#[test]
fn a_store_walked_by_the_waves_out_of_turn_is_the_store_of_the_wave_in_turn() {
    // Two waves take turns, so every store keeps what a thread out of turn held. `s24` holds
    // a mask of lanes, then a number halved until it is nought: no vote over lanes.
    let mut words = wave_fold("v_min_u32");
    words.extend([
        s1(S_MOV_B64, 24, EXEC),
        sopk(S_MOVK, 24, 8),
        s2("s_lshr_b32", 24, 24, I_ONE),
        sopp(S_CBRANCH_SCC1, 0xFFFE),
        S_ENDPGM,
    ]);
    let group = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [16, 8, 1],
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, words)
    };
    let out = translate(&group).unwrap();
    assert!(!out.subgroups);
    assert_eq!(out.loops_not_shown_to_end(), 0);
}

#[test]
fn a_lane_that_runs_by_a_vote_is_still_asked_for_its_own_bit() {
    // A wave kept in shared memory votes on every branch, and each thread says whether it
    // runs. The mask of running lanes is narrowed here by what a vote over a stated mask
    // said; the loop after it counts down, and its exit is the count's.
    let mut words = wave_fold("v_min_u32");
    words.extend([
        s1(S_MOV_B64, 24, I_MINUS_ONE),
        s2("s_andn2_b64", 24, 24, EXEC),
        s2("s_cselect_b64", 26, I_ONE, I_ZERO),
        s2("s_and_b64", EXEC, EXEC, 26),
        sopk(S_MOVK, 0, 3),
        sop2(S_ADD_I32, 0, 0, I_MINUS_ONE),
        sopc(S_CMP_LG_U32, 0, I_ZERO),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
        S_ENDPGM,
    ]);
    let group = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [16, 4, 1],
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, words)
    };
    let out = translate(&group).unwrap();
    assert!(!out.subgroups);
    assert_eq!(out.loops_not_shown_to_end(), 0);
}
