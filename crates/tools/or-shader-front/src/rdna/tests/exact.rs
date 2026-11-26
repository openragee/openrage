//! What the lowering no longer leaves to a device or to a guess: a group of several waves, the
//! first lane of a wave, a fused multiply-add of 16-bit floats and the kind of a loaded image.

use super::*;

/// A compute program of a group of `threads` that shares `shared_dwords` of memory.
fn group(threads: [u32; 3], shared_dwords: u32, words: Vec<u32>) -> Program {
    Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads,
            shared_dwords,
        }),
        ..program(ShaderKind::Compute, words)
    }
}

/// `v<dst>` = this thread's number in a group sixteen threads across.
fn thread_into(dst: u32) -> Vec<u32> {
    vec![
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), dst, I_ZERO + 4, 1),
        vop2(op(opcodes::VOP2, "v_add_u32"), dst, V, dst),
    ]
}

#[test]
fn the_waves_of_a_group_of_several_fold_branch_and_loop_each_as_its_own() {
    // v4 = the wave's number; v1 = (thread ^ 42) + 7, folded to its wave's least in s21.
    let mut words = thread_into(3);
    words.push(vop2(op(opcodes::VOP2, "v_lshrrev_b32"), 4, I_ZERO + 6, 3));
    words.push(vop2(op(opcodes::VOP2, "v_xor_b32"), 1, I_ZERO + 42, 3));
    words.push(vop2(op(opcodes::VOP2, "v_add_u32"), 1, I_ZERO + 7, 1));
    words.extend(wave_fold("v_min_u32"));
    // s22 = the wave's number, read off its first lane; s20 counts a loop the wave runs once
    // more than its number, so the two waves leave it at different rounds.
    words.extend([
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 22, V + 4),
        sopk(S_MOVK, 20, 0),
        sop2(S_ADD_U32, 20, 20, I_ONE),
        sopc(op(opcodes::SOPC, "s_cmp_le_u32"), 20, 22),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
        // Only the second wave enters: a scalar register the first never writes.
        sopc(S_CMP_EQ_U32, 22, I_ZERO),
        sopp(S_CBRANCH_SCC1, 1),
        sopk(S_MOVK, 27, 7),
        S_ENDPGM,
    ]);
    let out = translate(&group([16, 8, 1], 0, words)).unwrap();
    validate(&out.module);
    // No subgroup of the device is asked, and nothing is read as this lane alone.
    assert!(!out.subgroups);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let held = |thread: u32| (thread ^ 42) + 7;
    let ran = interp::run(&out.module, [16, 8, 1], &|_, _| None);
    assert_eq!(ran.len(), 128);
    for (thread, lane) in ran.iter().enumerate() {
        let wave = thread as u32 / 64;
        let least = (64 * wave..64 * wave + 64).map(held).min().unwrap();
        let read = [20, 21, 22, 27].map(|r| lane.reg(&format!("s{r}")));
        assert_eq!(read, [wave + 1, least, wave, 7 * wave], "thread {thread}");
    }
}

#[test]
fn a_wave_at_a_barrier_waits_for_the_group_s_other_waves() {
    // Each thread leaves three times its number at its own dword, the group meets, and each
    // reads the dword of the thread that mirrors it, which is in the other wave. The first
    // wave comes to the store by a block past the program's end, so later than the second.
    let (write, read) = (ds("ds_write_b32", 0), ds("ds_read_b32", 0));
    let mut words = thread_into(3);
    words.extend([
        vop2(op(opcodes::VOP2, "v_lshrrev_b32"), 4, I_ZERO + 6, 3),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 22, V + 4),
        sopc(S_CMP_EQ_U32, 22, I_ZERO),
        sopp(S_CBRANCH_SCC1, 12),
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 0, I_ZERO + 2, 3),
        vop2(op(opcodes::VOP2, "v_add_u32"), 1, V + 3, 3),
        vop2(op(opcodes::VOP2, "v_add_u32"), 1, V + 1, 3),
        write[0],
        write[1],
        sopp(op(opcodes::SOPP, "s_barrier"), 0),
        vop2(op(opcodes::VOP2, "v_xor_b32"), 5, LITERAL, 3),
        127,
        vop2(op(opcodes::VOP2, "v_lshlrev_b32"), 0, I_ZERO + 2, 5),
        read[0],
        read[1],
        S_ENDPGM,
        sopk(S_MOVK, 27, 7),
        sopp(S_BRANCH, 0xFFF2),
    ]);
    let out = translate(&group([16, 8, 1], 128, words)).unwrap();
    validate(&out.module);
    assert!(!out.subgroups);
    // A block of the waves' turns stands under an `if`, and its barriers in no `switch`.
    let switches = |s: &naga::Statement| matches!(s, naga::Statement::Switch { .. });
    let (_, main) = (out.module.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("main_body"))
        .unwrap();
    assert_eq!(count(&main.body, &switches), 0);
    let ran = interp::run(&out.module, [16, 8, 1], &|_, _| None);
    for (thread, lane) in ran.iter().enumerate() {
        assert_eq!(lane.reg("v1"), 3 * (127 - thread as u32), "thread {thread}");
    }
}

#[test]
fn a_group_whose_last_wave_is_not_full_keeps_the_device_s_subgroup() {
    let mut words = thread_into(2);
    words.extend(wave_fold("v_min_u32"));
    words.push(S_ENDPGM);
    // 80 threads are a wave of 64 and one of 16, which no thread stands for the rest of.
    assert!(
        translate(&group([16, 5, 1], 0, words.clone()))
            .unwrap()
            .subgroups
    );
    assert!(!translate(&group([16, 8, 1], 0, words)).unwrap().subgroups);
}

#[test]
fn a_fused_multiply_add_of_halves_is_rounded_once_in_the_module() {
    // 7/4 * 293/512 is the middle of two 16-bit floats; less the least 16-bit float it is
    // the lower of them, where the sum rounded to a float first comes back to the middle
    // and then to the even one above. A factor of zero leaves the addend.
    let fused = vop3(op3("v_fma_f16"), 3, [V + 1, V + 2, V + 4], 0, 0);
    let plain = vop3(op3("v_fma_f16"), 6, [V + 5, V + 2, V + 4], 0, 0);
    let position = exp(POS0, [3, 3, 3, 3]);
    let mut words = Vec::new();
    for (register, half) in [(1, 0x3F00), (2, 0x3894), (4, 0x8001), (5, 0)] {
        words.extend([vop1(V_MOV, register, LITERAL), half]);
    }
    words.extend([fused[0], fused[1], plain[0], plain[1]]);
    words.extend([position[0], position[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Vertex, words);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!([ran.reg("v3"), ran.reg("v6")], [0x3C01, 0x8001]);
}

/// The kind of texels a module's one texture is declared to hold.
fn texel_kind(module: &naga::Module) -> naga::ScalarKind {
    let kinds = (module.types.iter()).filter_map(|(_, ty)| match ty.inner {
        naga::TypeInner::Image {
            class: naga::ImageClass::Sampled { kind, .. },
            ..
        } => Some(kind),
        _ => None,
    });
    kinds.into_iter().next_back().expect("a texture")
}

#[test]
fn an_image_is_loaded_as_the_kind_the_binder_states() {
    // The texels of a two-dimensional image at v0 and v1, loaded into v0 to v3.
    let (image, read) = (
        smem(S_LOAD_X8, 8, 2, 0x40),
        mimg("image_load", 1, 0b1111, 0),
    );
    let target = exp(0, [0, 1, 2, 3]);
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, I_ONE)));
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let pixel = program(ShaderKind::Pixel, words);
    // Not stated: floats, and counted, since a texture of integers would not bind.
    let out = translate(&pixel).unwrap();
    assert_eq!(
        out.approximations.get("an image loaded as floats"),
        Some(&1)
    );
    assert_eq!(texel_kind(&out.module), naga::ScalarKind::Float);
    let loads = out.image_loads();
    let [at] = loads.iter().collect::<Vec<_>>()[..] else {
        panic!("{loads:?}");
    };
    assert_eq!(Some(at), out.resources.iter().find_map(|r| r.at.as_ref()));
    for kind in [
        naga::ScalarKind::Uint,
        naga::ScalarKind::Sint,
        naga::ScalarKind::Float,
    ] {
        let options = Options {
            image_kinds: [(at.clone(), kind)].into(),
            ..Options::default()
        };
        let out = translate_with(&pixel, &options).unwrap();
        validate(&out.module);
        assert!(out.approximations.is_empty(), "{:?}", out.approximations);
        assert_eq!(texel_kind(&out.module), kind);
        assert_eq!(out.image_loads(), loads);
    }
}

/// A pixel program that reads `v1`'s first lane into `s20` after `before`, and goes on with `after`.
fn first_lane(before: &[u32], after: &[u32]) -> Lowered {
    let target = exp(0, [1, 1, 1, 1]);
    let mut words = before.to_vec();
    words.push(vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V + 1));
    words.extend_from_slice(after);
    words.extend([target[0], target[1], S_ENDPGM]);
    lowered(ShaderKind::Pixel, words)
}

#[test]
fn the_first_lane_is_this_lane_where_the_lanes_agree_or_the_program_takes_them_in_turn() {
    const NOTE: &str = "the first lane read as this lane";
    let counted = |out: &Lowered| out.approximations.get(NOTE).copied().unwrap_or(0);
    let add = |dst, a, b| vop2(op(opcodes::VOP2, "v_add_u32"), dst, a, b);
    // A value moved from a scalar register and computed with, in every lane that runs.
    let agreed = [vop1(V_MOV, 2, 4), add(1, I_ONE, 2)];
    assert_eq!(counted(&first_lane(&agreed, &[])), 0);
    // An interpolated input is each lane's own, and so is anything computed from one.
    let own = [vintrp(1, 0, 0, 0, 2), add(1, I_ONE, 2)];
    assert_eq!(counted(&first_lane(&own, &[])), 1);
    // A value written while fewer lanes ran, read once others run again, is not theirs.
    let narrowed = [
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        vopc(V_CMPX_GT_F32, F_HALF, 0),
        vop1(V_MOV, 1, 4),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 24),
    ];
    assert_eq!(counted(&first_lane(&narrowed, &[])), 1);
    // Fewer lanes still agree on what they agreed on.
    let kept = [vop1(V_MOV, 1, 4), vopc(V_CMPX_GT_F32, F_HALF, 0)];
    assert_eq!(counted(&first_lane(&kept, &[])), 0);
    // What the lanes of a saved mask agreed on and nothing wrote since, they agree on when
    // the mask is put back, or the lanes of it that did not run are taken in their turn.
    for back in [
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 24),
        sop2(op(opcodes::SOP2, "s_andn2_b64"), 126, 24, 126),
    ] {
        let through = [
            vop1(V_MOV, 1, 4),
            sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
            vopc(V_CMPX_GT_F32, F_HALF, 0),
            vop1(V_MOV, 3, 4),
            back,
        ];
        assert_eq!(counted(&first_lane(&through, &[])), 0);
        // The register the mask was saved in holds another by then.
        let mut lost = through;
        lost[3] = sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 6);
        assert_eq!(counted(&first_lane(&lost, &[])), 1);
    }
    // The lanes a branch left waiting hold what they agreed on, whatever the lanes that ran
    // wrote; lanes that were taken away after the write hold what it wrote.
    let narrow = vopc(V_CMPX_GT_F32, F_HALF, 0);
    let turn = sop2(op(opcodes::SOP2, "s_andn2_b64"), 126, 24, 126);
    let waited = [
        vop1(V_MOV, 1, 4),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        narrow,
        vintrp(1, 0, 0, 0, 2),
        add(1, I_ONE, 2),
        vop1(op(opcodes::VOP1, "v_nop"), 0, 0),
        turn,
    ];
    assert_eq!(counted(&first_lane(&waited, &[])), 0);
    let mut twice = waited;
    twice[5] = narrow;
    assert_eq!(counted(&first_lane(&twice, &[])), 1);
    // A mask made of a saved mask's lanes is put back as those lanes; a scalar register
    // written beside the saved one, by an instruction of 32 bits, leaves it be.
    let within = [
        vop1(V_MOV, 1, 4),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 24, 126),
        sop2(op(opcodes::SOP2, "s_and_b64"), 28, 24, 24),
        sop1(op(opcodes::SOP1, "s_mov_b32"), 27, 6),
        narrow,
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, 28),
    ];
    assert_eq!(counted(&first_lane(&within, &[])), 0);
    let mut beside = within;
    beside[3] = sop1(op(opcodes::SOP1, "s_mov_b32"), 29, 6);
    assert_eq!(counted(&first_lane(&beside, &[])), 1);
    // The loop over a wave's distinct values: the lanes that hold the first lane's value
    // run and the others wait their turn, which comes with their own value read.
    let turn = [
        vopc(op(opcodes::VOPC, "v_cmp_eq_u32"), 20, 1),
        sop1(op(opcodes::SOP1, "s_and_saveexec_b64"), 24, 106),
    ];
    assert_eq!(counted(&first_lane(&own, &turn)), 0);
    let kept = [vopc(op(opcodes::VOPC, "v_cmpx_eq_u32"), 20, 1)];
    assert_eq!(counted(&first_lane(&own, &kept)), 0);
    // A comparison with something else keeps the wave to other lanes.
    let other = [
        vopc(op(opcodes::VOPC, "v_cmp_eq_u32"), 21, 1),
        sop1(op(opcodes::SOP1, "s_and_saveexec_b64"), 24, 106),
    ];
    assert_eq!(counted(&first_lane(&own, &other)), 1);
    // A value of 64 bits is read a half at a time and compared whole: both reads are the
    // turn's own. Compared a register on, only the half that is the comparison's low one is.
    let wide = |scalar, vector| {
        [
            vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 21, V + 2),
            vopc(op(opcodes::VOPC, "v_cmp_eq_u64"), scalar, vector),
            sop1(op(opcodes::SOP1, "s_and_saveexec_b64"), 24, 106),
        ]
    };
    assert_eq!(counted(&first_lane(&own, &wide(20, 1))), 0);
    assert_eq!(counted(&first_lane(&own, &wide(21, 2))), 1);
    // A 16-bit float widened fills its register: from a scalar register it is one value.
    let widened = [vop1(op(opcodes::VOP1, "v_cvt_f32_f16"), 1, 4)];
    assert_eq!(counted(&first_lane(&widened, &[])), 0);
}

#[test]
fn a_compute_wave_s_first_lane_is_read_in_shared_memory_where_the_lanes_may_differ() {
    // s20 = the first lane's thread number, which every lane of the wave then holds.
    let mut words = thread_into(3);
    words.push(vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V + 3));
    words.push(S_ENDPGM);
    let out = translate(&group([8, 8, 1], 0, words)).unwrap();
    validate(&out.module);
    assert!(!out.subgroups);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    for (thread, lane) in interp::run(&out.module, [8, 8, 1], &|_, _| None)
        .iter()
        .enumerate()
    {
        assert_eq!(lane.reg("s20"), 0, "thread {thread}");
    }
    // A caller that asks for the device's subgroup keeps this lane's reading, counted.
    let asked = Options {
        wave: true,
        ..Options::default()
    };
    let mut words = thread_into(3);
    words.push(vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V + 3));
    words.push(S_ENDPGM);
    let out = translate_with(&group([8, 8, 1], 0, words), &asked).unwrap();
    assert_eq!(out.approximations.len(), 1);
}

#[test]
fn a_part_s_float_modifiers_and_clamp_are_a_16_bit_float_s() {
    // v1's low half is -2 and its high half 1; v2's low half is 3/2. v3's low half = |-2| - 3/2,
    // which is a half: the modifiers turn bit 15 of the part, where a float's sign is bit 31.
    // v4's low half = the same sum without the magnitude, kept to zero and one: zero.
    let part = |dst: u32, modifiers: u32| {
        let word = 1 | 4 << 8 | 2 << 11 | 4 << 16 | 4 << 24 | modifiers;
        [vop2(op(opcodes::VOP2, "v_add_f16"), dst, 249, 2), word]
    };
    let (sum, kept) = (part(3, 1 << 21 | 1 << 28), part(4, 1 << 13 | 1 << 28));
    let position = exp(POS0, [3, 3, 3, 3]);
    let words = vec![
        vop1(V_MOV, 1, LITERAL),
        0x3C00_C000,
        vop1(V_MOV, 2, LITERAL),
        0x3E00,
        vop1(V_MOV, 4, LITERAL),
        0xABCD_1234,
        sum[0],
        sum[1],
        kept[0],
        kept[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    // The destination's other half is kept as the word says.
    assert_eq!([ran.reg("v3"), ran.reg("v4")], [0x3800, 0xABCD_0000]);
}

#[test]
fn a_value_chosen_by_a_condition_every_lane_shares_is_one_value() {
    const NOTE: &str = "the first lane read as this lane";
    let counted = |out: &Lowered| out.approximations.get(NOTE).copied().unwrap_or(0);
    let chosen = 0x100 + op(opcodes::VOP2, "v_cndmask_b32");
    // A scalar condition kept as a mask of every lane that runs or of none, and a choice by it.
    let pick = vop3(chosen, 1, [I_ZERO, I_ONE, 6], 0, 0);
    let shared = [
        sopc(S_CMP_EQ_U32, 4, I_ZERO),
        sop2(op(opcodes::SOP2, "s_cselect_b64"), 6, 126, I_ZERO),
        sop2(op(opcodes::SOP2, "s_or_b64"), 6, 6, 6),
        pick[0],
        pick[1],
    ];
    assert_eq!(counted(&first_lane(&shared, &[])), 0);
    // The mask is a pair of registers: its second written, it is that mask no longer.
    let clobbered = [
        sopc(S_CMP_EQ_U32, 4, I_ZERO),
        sop2(op(opcodes::SOP2, "s_cselect_b64"), 6, 126, I_ZERO),
        sop1(op(opcodes::SOP1, "s_mov_b32"), 7, 4),
        pick[0],
        pick[1],
    ];
    assert_eq!(counted(&first_lane(&clobbered, &[])), 1);
    // A comparison of each lane's own input is each lane's own bit.
    let own = [
        vintrp(1, 0, 0, 0, 2),
        vopc(op(opcodes::VOPC, "v_cmp_lt_u32"), I_ZERO, 2),
        sop1(op(opcodes::SOP1, "s_mov_b64"), 6, 106),
        pick[0],
        pick[1],
    ];
    assert_eq!(counted(&first_lane(&own, &[])), 1);
}

#[test]
fn a_mesh_thread_reads_its_wave_s_first_launch_number_at_the_entry() {
    // s20 = the launch number in the first lane while every lane runs: the wave's first thread's.
    let (position, primitive) = (exp(POS0, [1, 1, 1, 1]), exp(20, [5, 5, 5, 5]));
    let words = vec![
        sop1(op(opcodes::SOP1, "s_mov_b64"), 126, I_MINUS_ONE),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V + 5),
        vop1(V_MOV, 1, F_ONE),
        position[0],
        position[1],
        primitive[0],
        primitive[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Mesh, words);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    for thread in [0u32, 37, 64, 70, 127] {
        let given = move |_: u32, binding: &naga::Binding| match binding {
            naga::Binding::BuiltIn(naga::BuiltIn::VertexIndex) => {
                Some(interp::Value::U(3 * thread))
            }
            _ => None,
        };
        let ran = interp::run(&out.module, [1, 1, 1], &given).remove(0);
        assert_eq!(ran.reg("s20"), thread & !63, "thread {thread}");
    }
}

#[test]
fn a_three_source_form_over_halves_reads_and_writes_the_halves_its_word_names() {
    // v1's high half is 1 and its low half 2; v3's high half = 1 * 3 + 1/2 and its low half
    // stays: bits 11 and 14 of the first word name the first source's and the result's.
    let mut fused = vop3(op3("v_fma_f16"), 3, [V + 1, V + 2, V + 4], 0, 0);
    fused[0] |= 0b1001 << 11;
    let position = exp(POS0, [3, 3, 3, 3]);
    let mut words = Vec::new();
    for (register, held) in [(1, 0x3C00_4000), (2, 0x4200), (4, 0x3800), (3, 0x1234)] {
        words.extend([vop1(V_MOV, register, LITERAL), held]);
    }
    words.extend([fused[0], fused[1], position[0], position[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Vertex, words);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let ran = interp::run(&out.module, [1, 1, 1], &|_, _| None).remove(0);
    assert_eq!(ran.reg("v3"), 0x4300_1234);
}

#[test]
fn a_compute_thread_reads_its_wave_s_first_thread_at_the_entry() {
    // s20 and s21 = the thread's column and row in the first lane: those of its wave's
    // first thread, which no memory is shared to find.
    let words = vec![
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 20, V),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 21, V + 1),
        S_ENDPGM,
    ];
    let out = translate(&group([16, 8, 1], 0, words)).unwrap();
    validate(&out.module);
    assert!(!out.subgroups);
    assert!(out.approximations.is_empty(), "{:?}", out.approximations);
    let shared = (out.module.global_variables.iter())
        .any(|(_, global)| global.space == naga::AddressSpace::WorkGroup);
    assert!(!shared);
    let ran = interp::run(&out.module, [16, 8, 1], &|_, _| None);
    for (thread, lane) in ran.iter().enumerate() {
        let read = [lane.reg("s20"), lane.reg("s21")];
        assert_eq!(read, [0, 4 * (thread as u32 / 64)], "thread {thread}");
    }
}

/// A graph that does not nest, after the words `before`: the entry branches into the middle of
/// a loop that the block after it enters at its head. `inside` opens the loop's second block,
/// and `v4` is left what `v3` held on the way a run takes, which never comes to the loop's head.
pub(super) fn entered_twice(before: &[u32], inside: &[u32]) -> Vec<u32> {
    let back = 0x1_0000 - (inside.len() as u32 + 3);
    let mut words = before.to_vec();
    words.extend([
        vop1(V_MOV, 3, F_ONE),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 1),
        vop1(V_MOV, 3, F_HALF),
    ]);
    words.extend_from_slice(inside);
    words.extend([
        sopc(S_CMP_LG_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, back as u16),
        vop1(V_MOV, 4, V + 3),
    ]);
    words
}

/// The `switch` statements among those of the module's body.
pub(super) fn switches(module: &naga::Module) -> usize {
    let (_, main) = (module.functions.iter())
        .find(|(_, f)| f.name.as_deref() == Some("main_body"))
        .unwrap();
    count(&main.body, &|s| matches!(s, naga::Statement::Switch { .. }))
}

pub(super) const DISPATCHED: &str = "a dispatch loop for a graph that does not nest";

#[test]
fn a_block_that_meets_stands_under_an_if_where_the_graph_does_not_nest() {
    let lowered = |inside: u32| {
        let mut words = entered_twice(&[], &[inside]);
        words.push(S_ENDPGM);
        let out = translate(&group([8, 8, 1], 0, words)).unwrap();
        validate(&out.module);
        assert!(out.approximations.contains_key(DISPATCHED));
        let ran = interp::run(&out.module, [8, 8, 1], &|_, _| None);
        assert_eq!(ran.len(), 64);
        for (thread, lane) in ran.iter().enumerate() {
            assert_eq!(lane.reg("v4"), ONE, "thread {thread}");
        }
        (switches(&out.module), ran[0].barriers)
    };
    // The program's own barrier in a block of the loop: every block under an `if`, and the
    // one barrier come to once.
    assert_eq!(lowered(sopp(op(opcodes::SOPP, "s_barrier"), 0)), (0, 1));
    // The same graph with no barrier anywhere keeps its switch.
    assert_eq!(lowered(vop1(V_MOV, 5, F_ONE)), (1, 0));
}
