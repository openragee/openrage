//! A depth held to one side of the rasterizer's, and a count of lanes summed as 64-bit floats.

use super::*;

/// A signature of elements that hold these system values.
fn signature(system_values: &[u32]) -> Vec<u8> {
    let mut bytes = vec![0u8; 8 + ELEMENT * system_values.len()];
    bytes[..4].copy_from_slice(&(system_values.len() as u32).to_le_bytes());
    bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
    for (element, value) in system_values.iter().enumerate() {
        let at = 8 + ELEMENT * element + ELEMENT_SYSTEM_VALUE;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn a_depth_held_to_one_side_hands_the_program_the_position_s_depth() {
    // A target and a depth are no reason; a depth that may only grow, or only shrink, is.
    assert!(!one_sided_depth(&signature(&[64, 65])));
    assert!(one_sided_depth(&signature(&[64, 67])));
    assert!(one_sided_depth(&signature(&[68])));
    assert!(!one_sided_depth(&[]));
    // v4 = the register after the position's x and y: the depth where the program is handed
    // it, and nothing the model knows where it is not.
    let target = exp(0, [4, 4, 4, 4]);
    let words = vec![vop2(V_ADD_F32, 4, V + 4, 4), target[0], target[1], S_ENDPGM];
    let handed = |position_read| Program {
        position_read,
        front_face: false,
        ..program(ShaderKind::Pixel, words.clone())
    };
    let out = translate(&handed(0b0111)).unwrap();
    validate(&out.module);
    let refused = translate(&handed(0b0011)).unwrap_err();
    assert!(
        matches!(&refused, RdnaError::NotTranslated(why) if why.name() == "live-in v"),
        "{refused}"
    );
}

/// A two-source mnemonic's number in the three-source encoding.
fn op2_wide(name: &str) -> u32 {
    0x100 + op(opcodes::VOP2, name)
}

#[test]
fn a_count_of_lanes_summed_as_doubles_over_a_wave_is_the_count_as_a_double() {
    let one = 1f64.to_bits();
    // v6 = this lane's number in a group eight threads across; the mask is the lanes whose
    // number has bit 0 or bit 2.
    let mut words = vec![
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 6, I_ZERO + 3, 1),
        vop2(op(opcodes::VOP2, "v_add_u32"), 6, V, 6),
        vop2(op(opcodes::VOP2, "v_and_b32"), 7, I_ZERO + 5, 6),
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO, 7),
        // v19 and v20 = the double one; v2 and v3 = it in the mask's lanes, nought in the rest.
        vop1(V_MOV, 19, I_ZERO),
        vop1(V_MOV, 20, LITERAL),
        (one >> 32) as u32,
        vop1(V_MOV, 2, I_ZERO),
        vop1(V_MOV, 3, I_ZERO),
    ];
    words.extend(vop3(
        op2_wide("v_cndmask_b32"),
        2,
        [V + 2, V + 19, 106],
        0,
        0,
    ));
    words.extend(vop3(
        op2_wide("v_cndmask_b32"),
        3,
        [V + 3, V + 20, 106],
        0,
        0,
    ));
    let join = |from: u32| vop3(op3("v_add_f64"), 2, [from, V + 2, 0], 0, 0);
    // Each step moves another lane's pair beside this lane's and adds the two.
    let patterns = [PAIR_SWAP, 0o4466_0022, 0o4444].map(|selectors| (233, selectors << 8));
    for (operand, pattern) in (patterns.into_iter()).chain([(250, ROW_MIRROR << 8 | 0xFF << 24)]) {
        words.extend([vop1(V_MOV, 4, operand), 2 | pattern]);
        words.extend([vop1(V_MOV, 5, operand), 3 | pattern]);
        words.extend(join(V + 4));
    }
    words.extend(vop3(
        op3("v_permlanex16_b32"),
        4,
        [V + 2, I_ZERO, I_ZERO],
        0,
        0,
    ));
    words.extend(vop3(
        op3("v_permlanex16_b32"),
        5,
        [V + 3, I_ZERO, I_ZERO],
        0,
        0,
    ));
    words.extend(join(V + 4));
    // The other half of the wave's sum, through two scalar registers and a move of them.
    words.extend(vop3(
        op3("v_readlane_b32"),
        20,
        [V + 2, I_ZERO + 32, 0],
        0,
        0,
    ));
    words.extend(vop3(
        op3("v_readlane_b32"),
        21,
        [V + 3, I_ZERO + 32, 0],
        0,
        0,
    ));
    words.push(sop1(op(opcodes::SOP1, "s_mov_b32"), 22, 20));
    words.push(sop1(op(opcodes::SOP1, "s_mov_b32"), 23, 21));
    words.extend(join(22));
    words.extend(vop3(op3("v_readlane_b32"), 24, [V + 2, I_ZERO, 0], 0, 0));
    words.extend(vop3(op3("v_readlane_b32"), 25, [V + 3, I_ZERO, 0], 0, 0));
    words.push(vop1(op(opcodes::VOP1, "v_cvt_f32_f64"), 8, 24));
    words.push(S_ENDPGM);
    let group = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [8, 8, 1],
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, words.clone())
    };
    let out = translate(&group).unwrap();
    validate(&out.module);
    assert!(!out.subgroups);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let count = (0..64u32).filter(|lane| lane & 5 != 0).count() as u32;
    let sum = f64::from(count).to_bits();
    for (lane, thread) in interp::run(&out.module, [8, 8, 1], &|_, _| None)
        .iter()
        .enumerate()
    {
        let held = [thread.reg("s24"), thread.reg("s25"), thread.reg("v8")];
        let want = [sum as u32, (sum >> 32) as u32, (count as f32).to_bits()];
        assert_eq!(held, want, "lane {lane}");
    }
    // Floats the code does not show to be nought or one are not summed.
    let other = words.iter().position(|word| *word == (one >> 32) as u32);
    let mut words = words.clone();
    words[other.unwrap()] = (2f64.to_bits() >> 32) as u32;
    let refused = translate(&Program { words, ..group }).unwrap_err();
    assert!(
        matches!(&refused, RdnaError::NotTranslated(why) if why.name().starts_with("v_add_f64")),
        "{refused}"
    );
}
