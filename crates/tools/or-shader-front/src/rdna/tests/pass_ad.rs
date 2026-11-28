//! The leftovers: a mesh group's memory filled by running its threads, a texel compared and
//! swapped without the exchange, a pixel program's ordered section run in no order, and the
//! least and the greatest of floats where an operand is NaN.

use super::*;

const SPREAD: u32 = 2_654_435_761;

/// A shared-memory instruction at the address in `addr`, its offsets, data and destination.
fn ds_at(name: &str, addr: u32, offsets: [u32; 2], data: [u32; 2], dst: u32) -> [u32; 2] {
    let number = op(opcodes::DS, name);
    [
        0xD800_0000 | number << 18 | offsets[1] << 8 | offsets[0],
        addr | data[0] << 8 | data[1] << 16 | dst << 24,
    ]
}

/// A mesh group of 64 threads that share 128 dwords: each thread's number in `v0`, its value
/// `v9`, then `inside`, then a vertex and a primitive of its number.
fn mesh_sharing(inside: &[u32]) -> Program {
    let low = vop3(op3("v_mbcnt_lo_u32_b32"), 0, [I_MINUS_ONE, I_ZERO, 0], 0, 0);
    let lane = vop3(op3("v_mbcnt_hi_u32_b32"), 0, [I_MINUS_ONE, V, 0], 0, 0);
    let spread = vop3(op3("v_mul_lo_u32"), 9, [V, LITERAL, 0], 0, 0);
    let (position, primitive) = (exp(POS0, [2, 2, 2, 2]), exp(20, [0, 0, 0, 0]));
    let mut words = vec![low[0], low[1], lane[0], lane[1]];
    words.extend([spread[0], spread[1], SPREAD]);
    words.extend_from_slice(inside);
    words.extend([vop1(V_MOV, 2, F_ONE), position[0], position[1]]);
    words.extend([primitive[0], primitive[1], S_ENDPGM]);
    Program {
        launch: Some(Launch {
            user_registers: 0,
            group_axes: 0,
            group_size: false,
            threads: [64, 1, 1],
            shared_dwords: 128,
        }),
        ..program(ShaderKind::Mesh, words)
    }
}

/// The run of one vertex of a mesh module: the first corner of primitive `thread`, the vertex
/// that thread placed.
fn mesh_vertex(module: &naga::Module, thread: u32) -> interp::Thread {
    let given = move |_: u32, binding: &naga::Binding| match binding {
        naga::Binding::BuiltIn(naga::BuiltIn::VertexIndex) => Some(interp::Value::U(3 * thread)),
        _ => None,
    };
    interp::run(module, [1, 1, 1], &given).remove(0)
}

#[test]
fn a_mesh_thread_reads_what_the_other_threads_of_its_group_stored_before_the_barrier() {
    // Thread t stores its value at dword 63 - t; after the barrier it reads dwords t and t + 1,
    // which threads 63 - t and 62 - t stored.
    let shift = op(opcodes::VOP2, "v_lshlrev_b32");
    let mut inside = vec![
        vop2(op(opcodes::VOP2, "v_sub_u32"), 7, I_ZERO + 63, 0),
        vop2(shift, 7, I_ZERO + 2, 7),
        vop2(shift, 8, I_ZERO + 2, 0),
    ];
    inside.extend(ds_at("ds_write_b32", 7, [0, 0], [9, 0], 0));
    inside.push(sopp(op(opcodes::SOPP, "s_barrier"), 0));
    inside.extend(ds_at("ds_read2_b32", 8, [0, 1], [0, 0], 11));
    let out = translate(&mesh_sharing(&inside)).unwrap();
    validate(&out.module);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    // Every thread of the group runs before the vertex's own two: a loop of calls.
    let entry = &out.module.entry_points[0].function.body;
    assert!(holds(entry, &|s| matches!(s, naga::Statement::Loop { .. })));
    let held = |thread: u32| thread.wrapping_mul(SPREAD);
    for thread in [0u32, 1, 5, 31, 32, 62, 63] {
        let ran = mesh_vertex(&out.module, thread);
        assert_eq!(ran.reg("v9"), held(thread), "thread {thread}");
        assert_eq!(ran.reg("v11"), held(63 - thread), "thread {thread}");
        if thread < 63 {
            assert_eq!(ran.reg("v12"), held(62 - thread), "thread {thread}");
        }
    }
}

#[test]
fn a_mesh_program_that_stores_after_its_first_barrier_is_refused_by_name() {
    let mut inside = vec![sopp(op(opcodes::SOPP, "s_barrier"), 0)];
    inside.extend(ds_at("ds_write_b32", 0, [0, 0], [9, 0], 0));
    let refused = translate(&mesh_sharing(&inside)).unwrap_err();
    let RdnaError::NotTranslated(refused) = refused else {
        panic!("not a refusal of the lowering");
    };
    let name = refused.name();
    assert!(
        name.starts_with("ds_write_b32: the memory a group's threads share, stored after"),
        "{name}"
    );
    // And one that reads before it.
    let mut inside = ds_at("ds_read_b32", 0, [0, 0], [0, 0], 11).to_vec();
    inside.push(sopp(op(opcodes::SOPP, "s_barrier"), 0));
    let refused = translate(&mesh_sharing(&inside)).unwrap_err();
    assert!(refused
        .to_string()
        .contains("stored after the group's first barrier or read before it"));
}

#[test]
fn a_mesh_package_states_its_group_s_threads_and_shared_memory() {
    let mut package = vec![0u8; 0x600];
    package[0x10C..0x110].copy_from_slice(&10_368u32.to_le_bytes());
    package[0x55C..0x560].copy_from_slice(&0x0080_0080u32.to_le_bytes());
    let group = crate::rdna::mesh_group(&package);
    assert_eq!(group.threads, [128, 1, 1]);
    assert_eq!(group.shared_dwords, 2_592);
    // A size that ends inside a dword still holds that dword.
    package[0x10C..0x110].copy_from_slice(&10u32.to_le_bytes());
    assert_eq!(crate::rdna::mesh_group(&package).shared_dwords, 3);
}

/// `image_atomic_cmpswap` of a 32-bit texel at the place in `addr`: the new dword in `data`,
/// the compared one after it, the texel found given back.
fn swap(data: u32, addr: u32) -> [u32; 2] {
    let [first, second] = mimg("image_atomic_cmpswap", 1, 3, 0);
    [first | 1 << 13, second | data << 8 | addr]
}

#[test]
fn a_texel_is_compared_and_swapped_as_a_load_and_a_store_and_counted() {
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    // Nought is compared first, which no texel here holds; then what the texel held.
    words.extend([vop1(V_MOV, 4, I_ZERO + 9), vop1(V_MOV, 5, I_ZERO)]);
    words.extend(swap(4, 0));
    words.extend([vop1(V_MOV, 7, V + 4), vop1(V_MOV, 6, I_ZERO + 9)]);
    words.extend(swap(6, 0));
    words.push(S_ENDPGM);
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(out.approximations.get(to_naga::TEXEL_UNLOCKED), Some(&2));
    assert_eq!(
        out.resources[0].kind,
        ResourceKind::StorageTexture {
            dim: 1,
            format: naga::StorageFormat::R32Uint
        }
    );
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    let found = ran.reg("v4");
    assert_ne!(found, 0, "a texel that held the first compared dword");
    // The second swap found the same texel, which it held, and stored over it.
    assert_eq!(ran.reg("v6"), found);
    assert_eq!(ran.stores.len(), 1, "{:?}", ran.stores);
    assert_eq!(ran.stores[0][2], [9, 0, 0, 0]);
}

/// `s_setreg_b32` of `s1` into the three low bits of register 25, which turns the order on.
fn order_on() -> u32 {
    sopk(op(opcodes::SOPK, "s_setreg_b32"), 1, 0x1019)
}

#[test]
fn a_pixel_program_that_turns_the_order_on_and_stores_nothing_counts_only_its_wait() {
    // The order on, then the wave that last left read and added to five.
    let words = vec![order_on(), sop2(S_ADD_I32, 20, 239, I_ZERO + 5), S_ENDPGM];
    let out = lowered(ShaderKind::Pixel, words.clone());
    // The setting changes nothing where nothing is stored; the read of the wave is counted.
    assert_eq!(out.approximations.get(to_naga::UNORDERED), Some(&1));
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!(ran.reg("s20"), 5);
    // Outside such a program the wave that last left is no operand this front end has.
    let refused = refusal(ShaderKind::Pixel, words[1..].to_vec());
    assert!(
        refused.to_string().contains("src_pops_exiting_wave_id"),
        "{refused}"
    );
    let out = lowered(ShaderKind::Pixel, vec![order_on(), S_ENDPGM]);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
}

#[test]
fn a_pixel_program_that_waits_its_turn_and_stores_a_texel_counts_the_order_it_drops() {
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let store = mimg("image_store", 1, 0b1111, 0);
    let mut words = vec![image[0], image[1], order_on()];
    words.extend((0..4).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([store[0], store[1]]);
    words.extend([sopp(op(opcodes::SOPP, "s_sendmsg"), 7), S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(out.approximations.get(to_naga::UNORDERED), Some(&1));
    assert_eq!(
        to_naga::refused_for_drawing(&out.approximations),
        Some(to_naga::UNORDERED)
    );
}

#[test]
fn a_least_and_a_greatest_with_a_nan_give_the_other_operand_and_of_two_the_second() {
    const FIRST: u32 = 0x7FC0_0001;
    const SECOND: u32 = 0x7FC0_0002;
    let (min, max) = (
        op(opcodes::VOP2, "v_min_f32"),
        op(opcodes::VOP2, "v_max_f32"),
    );
    let mut words = vec![
        vop1(V_MOV, 5, LITERAL),
        FIRST,
        vop1(V_MOV, 6, LITERAL),
        SECOND,
        vop1(V_MOV, 7, F_ONE),
        vop2(min, 20, V + 5, 7),
        vop2(min, 21, V + 7, 5),
        vop2(max, 22, V + 5, 6),
        vop2(max, 23, V + 6, 5),
        vop2(max, 24, V + 7, 5),
    ];
    words.extend(vop3(op3("v_min3_f32"), 25, [V + 5, V + 6, V + 7], 0, 0));
    words.push(S_ENDPGM);
    let out = lowered(ShaderKind::Pixel, words);
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    let one = 1.0f32.to_bits();
    assert_eq!(ran.reg("v20"), one);
    assert_eq!(ran.reg("v21"), one);
    assert_eq!(ran.reg("v22"), SECOND);
    assert_eq!(ran.reg("v23"), FIRST);
    assert_eq!(ran.reg("v24"), one);
    assert_eq!(ran.reg("v25"), one);
}

#[test]
fn an_operand_that_cannot_be_a_nan_is_not_tested_for_one() {
    // The count of NaN tests in the body: two operands of registers, then a constant and one.
    let tests = |src0: &[u32]| {
        let mut words = vec![vop1(V_MOV, 5, LITERAL), 0x7FC0_0001, vop1(V_MOV, 6, F_ONE)];
        words.push(vop2(op(opcodes::VOP2, "v_max_f32"), 20, src0[0], 5));
        words.extend(&src0[1..]);
        words.push(S_ENDPGM);
        let out = lowered(ShaderKind::Pixel, words);
        let infinity = |e: &naga::Expression| {
            matches!(
                e,
                naga::Expression::Literal(naga::Literal::U32(0x7F80_0000))
            )
        };
        (body(&out.module).expressions.iter())
            .filter(|(_, e)| infinity(e))
            .count()
    };
    let (registers, constant) = (tests(&[V + 6]), tests(&[I_ZERO]));
    assert_eq!(registers, constant + 1);
    assert_eq!(tests(&[LITERAL, 0x4000_0000]), constant);
}
