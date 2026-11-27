//! A dword of the group's memory compared and swapped, and the updates that stay refused.

use super::*;

/// A shared-memory instruction at the address in `addr`, with its two data and its destination.
fn ds_at(name: &str, addr: u32, data0: u32, data1: u32, dst: u32) -> [u32; 2] {
    let number = op(opcodes::DS, name);
    [
        0xD800_0000 | number << 18,
        addr | data0 << 8 | data1 << 16 | dst << 24,
    ]
}

#[test]
fn a_shared_dword_is_swapped_only_where_it_holds_the_compared_value() {
    // v6 = the thread's number in a group eight across, stored in its own dword (v7).
    let mut words = vec![
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 6, I_ZERO + 3, 1),
        vop2(op(opcodes::VOP2, "v_add_u32"), 6, V, 6),
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 7, I_ZERO + 2, 6),
        vop1(V_MOV, 8, I_ZERO + 5),
        vop1(V_MOV, 9, I_ZERO + 9),
        vop1(V_MOV, 20, I_MINUS_ONE),
    ];
    // Stored; 9 put where the dword is 5, twice; the greater of it and -1 as signed; read back.
    for inst in [
        ds_at("ds_write_b32", 7, 6, 0, 0),
        ds_at("ds_cmpst_rtn_b32", 7, 8, 9, 11),
        ds_at("ds_cmpst_rtn_b32", 7, 8, 9, 12),
        ds_at("ds_max_rtn_i32", 7, 20, 0, 21),
        ds_at("ds_read_b32", 7, 0, 0, 22),
    ] {
        words.extend(inst);
    }
    words.push(S_ENDPGM);
    let group = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [8, 8, 1],
            shared_dwords: 128,
        }),
        ..program(ShaderKind::Compute, words)
    };
    let out = translate(&group).unwrap();
    validate(&out.module);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    for (lane, thread) in interp::run(&out.module, [8, 8, 1], &|_, _| None)
        .iter()
        .enumerate()
    {
        let lane = lane as u32;
        let after = if lane == 5 { 9 } else { lane };
        let held = ["v11", "v12", "v21", "v22"].map(|name| thread.reg(name));
        assert_eq!(held, [lane, after, after, after], "lane {lane}");
    }
}

#[test]
fn a_texel_compared_and_swapped_is_refused_by_its_own_reason() {
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let swap = mimg("image_atomic_cmpswap", 1, 3, 0);
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([swap[0], swap[1], S_ENDPGM]);
    let refused = refusal(ShaderKind::Pixel, words);
    let wanted = format!("image_atomic_cmpswap: {}", to_naga::TEXEL_SWAP);
    assert!(
        matches!(&refused, RdnaError::NotTranslated(why) if why.name() == wanted),
        "{refused}"
    );
}
