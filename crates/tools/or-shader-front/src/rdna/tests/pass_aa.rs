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
fn a_loop_over_lanes_the_lowering_does_not_run_is_counted() {
    let out = vertex(lanes_in_turns());
    assert_eq!(out.approximations.get(LOOP_OTHER_LANES), Some(&1));
    assert_eq!(out.loops_not_shown_to_end(), 1);
    assert!(LOOP_OTHER_LANES.starts_with(LOOP_NOT_SHOWN));
    assert!(LOOP_UNCHANGING.starts_with(LOOP_NOT_SHOWN));
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
