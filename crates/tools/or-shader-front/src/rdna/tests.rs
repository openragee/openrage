//! Every input is assembled here from the encodings' field layouts; none comes from a game file.

use naga::valid::{Capabilities, ValidationFlags, Validator};
use or_io::gta5::shaders::xdat::{XdatProgram, XdatStage};

use super::*;

mod cache;
mod equal;
mod exact;
mod hand;
mod interp;
mod link;
mod pass_aa;
mod pass_ab;
mod pass_ac;
mod pass_rt;
mod pass_rt_body;
mod pass_rt_query;
mod pass_rt_walk;
mod pass_rt_wave;
mod pass_rt_wave_walk;
mod pass_w;
mod pass_y;

const S_ENDPGM: u32 = 0xBF81_0000;
const ONE: u32 = 0x3F80_0000;
/// Operand codes: VGPR `n` is 256 + n, and 255 says a literal word follows.
const V: u32 = 256;
const LITERAL: u32 = 255;
/// The inline constant 1.0.
const F_ONE: u32 = 242;

fn sopp(op: u32, imm: u16) -> u32 {
    0xBF80_0000 | op << 16 | u32::from(imm)
}

fn sop1(op: u32, dst: u32, src: u32) -> u32 {
    0xBE80_0000 | dst << 16 | op << 8 | src
}

fn sop2(op: u32, dst: u32, src0: u32, src1: u32) -> u32 {
    0x8000_0000 | op << 23 | dst << 16 | src1 << 8 | src0
}

fn sopk(op: u32, dst: u32, imm: u16) -> u32 {
    0xB000_0000 | op << 23 | dst << 16 | u32::from(imm)
}

fn vop1(op: u32, dst: u32, src: u32) -> u32 {
    0x7E00_0000 | dst << 17 | op << 9 | src
}

fn vop2(op: u32, dst: u32, src0: u32, src1: u32) -> u32 {
    op << 25 | dst << 17 | src1 << 9 | src0
}

fn vop3(op: u32, dst: u32, srcs: [u32; 3], abs: u32, neg: u32) -> [u32; 2] {
    [
        0xD400_0000 | op << 16 | abs << 8 | dst,
        srcs[0] | srcs[1] << 9 | srcs[2] << 18 | neg << 29,
    ]
}

fn vintrp(op: u32, dst: u32, attr: u32, chan: u32, src: u32) -> u32 {
    0xC800_0000 | dst << 18 | op << 16 | attr << 10 | chan << 8 | src
}

/// An export of four VGPRs to `target`, marked as the last.
fn exp(target: u32, srcs: [u8; 4]) -> [u32; 2] {
    [
        0xF800_0000 | 1 << 11 | target << 4 | 0xF,
        u32::from_le_bytes(srcs),
    ]
}

const V_MOV: u32 = 0x01;
const V_ADD_F32: u32 = 0x03;
const V_MUL_F32: u32 = 0x08;
const V_MAD_F32: u32 = 0x141;
const S_BRANCH: u32 = 2;
const S_CBRANCH_SCC1: u32 = 5;
const POS0: u32 = 12;

fn text(words: &[u32]) -> Vec<String> {
    (decode(words).unwrap().iter())
        .map(|(_, inst)| inst.to_string())
        .collect()
}

/// Validates `module`, then has every writer a device compiles a module with write it: one
/// that validates and that a backend cannot be given is no translation.
fn validate(module: &naga::Module) {
    use naga::back::{hlsl, msl, spv, wgsl};
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .unwrap();
    let options = spv::Options {
        lang_version: (1, 3),
        capabilities: None,
        ..Default::default()
    };
    spv::write_vec(module, &info, &options, None).unwrap();
    wgsl::write_string(module, &info, wgsl::WriterFlags::empty()).unwrap();
    let options = msl::Options {
        lang_version: (3, 0),
        fake_missing_bindings: true,
        ..Default::default()
    };
    msl::write_string(module, &info, &options, &msl::PipelineOptions::default()).unwrap();
    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V6_0,
        fake_missing_bindings: true,
        ..Default::default()
    };
    let mut text = String::new();
    hlsl::Writer::new(&mut text, &options, &hlsl::PipelineOptions::default())
        .write(module, &info, None)
        .unwrap();
}

#[test]
fn published_encodings_disassemble_to_their_assembly() {
    assert_eq!(
        text(&[0x7E00_0301, 0x0600_0501, 0xBE80_0301, 0xBE84_2104, S_ENDPGM]),
        [
            "v_mov_b32 v0, v1",
            "v_add_f32 v0, v1, v2",
            "s_mov_b32 s0, s1",
            "s_swappc_b64 s4, s4",
            "s_endpgm",
        ]
    );
}

#[test]
fn each_family_is_told_by_its_leading_bits() {
    let family = |word: u32| {
        let (inst, _) = decode_one(&[word, 0, 0, 0], 0).unwrap();
        inst.op().map(|op| op.family)
    };
    for (word, expected) in [
        (vop2(V_ADD_F32, 0, V, 0), Family::Vop2),
        (vop1(V_MOV, 0, V), Family::Vop1),
        (0x7C00_0000, Family::Vopc),
        (sop2(0, 0, 1, 2), Family::Sop2),
        (sopk(0, 0, 7), Family::Sopk),
        (sop1(3, 0, 1), Family::Sop1),
        (0xBF00_0000, Family::Sopc),
        (S_ENDPGM, Family::Sopp),
        (vintrp(0, 0, 0, 0, 0), Family::Vintrp),
        (0xCC00_0000, Family::Vop3p),
        (0xD400_0000, Family::Vop3),
        (0xD800_0000, Family::Ds),
        (0xDC00_0000, Family::Flat),
        (0xDC00_8000, Family::Global),
        (0xE000_0000, Family::Mubuf),
        (0xE800_0000, Family::Mtbuf),
        (0xF000_0000, Family::Mimg),
        (0xF400_0000, Family::Smem),
        (0xF800_0000, Family::Exp),
    ] {
        assert_eq!(family(word), Some(expected), "{word:#010x}");
    }
    assert_eq!(family(0xC000_0000), None);
}

#[test]
fn an_instruction_is_as_long_as_its_operands_make_it() {
    let len = |words: &[u32]| decode_one(words, 0).unwrap().1;
    assert_eq!(len(&[vop1(V_MOV, 0, V + 1)]), 1);
    assert_eq!(len(&[vop1(V_MOV, 0, LITERAL), ONE]), 2);
    assert_eq!(len(&[sop1(3, 0, LITERAL), 7]), 2);
    // `s_setreg_imm32_b32`, and `v_madak_f32`'s constant.
    assert_eq!(len(&[sopk(0x15, 0, 0), 7]), 2);
    assert_eq!(len(&[vop2(0x21, 0, V, 1), ONE]), 2);
    let mad = vop3(V_MAD_F32, 0, [V, V + 1, V + 2], 0, 0);
    assert_eq!(len(&mad), 2);
    let mad = vop3(V_MAD_F32, 0, [V, LITERAL, V + 2], 0, 0);
    assert_eq!(len(&[mad[0], mad[1], ONE]), 3);
    // An image instruction with two extra address words.
    assert_eq!(len(&[0xF000_0000 | 0x20 << 18 | 2 << 1, 0, 0, 0]), 4);

    let (inst, _) = decode_one(&[vop1(V_MOV, 3, LITERAL), ONE], 0).unwrap();
    assert_eq!(inst.to_string(), "v_mov_b32 v3, 0x3f800000");
}

#[test]
fn modifiers_and_exports_are_spelled_out() {
    let mad = vop3(V_MAD_F32, 4, [V + 1, V + 2, F_ONE], 0b010, 0b001);
    let export = exp(POS0, [0, 1, 2, 3]);
    assert_eq!(
        text(&[mad[0], mad[1], export[0], export[1]]),
        [
            "v_mad_f32 v4, -v1, |v2|, 1.0",
            "exp pos0, v0, v1, v2, v3 done"
        ]
    );
}

#[test]
fn an_instruction_cut_short_or_of_no_family_is_said() {
    assert_eq!(
        decode_one(&[vop1(V_MOV, 0, LITERAL)], 0),
        Err(DecodeError::Truncated {
            pc: 0,
            family: Family::Vop1,
            need: 2
        })
    );
    assert_eq!(
        decode_one(&[], 0),
        Err(DecodeError::OutOfRange { pc: 0, len: 0 })
    );
    assert_eq!(text(&[0xC000_0000]), [".long 0xc0000000"]);
    // A number its family does not define keeps its family and number.
    assert_eq!(text(&[sopp(0x7F, 0)]), ["sopp_0x7f"]);
}

#[test]
fn a_conditional_branch_cuts_three_blocks() {
    // 0: s_cbranch_scc1 +1 (to word 2)   1: v_mov v0, v1   2: s_endpgm
    let words = [sopp(S_CBRANCH_SCC1, 1), vop1(V_MOV, 0, V + 1), S_ENDPGM];
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(cfg.blocks.keys().copied().collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(
        cfg.blocks[&0].terminator,
        Terminator::Branch {
            taken: 2,
            fallthrough: 1
        }
    );
    assert_eq!(cfg.blocks[&1].terminator, Terminator::Fall(2));
    assert_eq!(cfg.blocks[&2].terminator, Terminator::End);
    assert_eq!(cfg.successors(0), [2, 1]);
    assert_eq!(cfg.successors(2), Vec::<u32>::new());
}

#[test]
fn a_jump_steps_over_data_and_a_loop_closes() {
    // 0: s_branch +1 (to 2)   1: a word of no family   2: s_branch -1 (to 2)
    let words = [sopp(S_BRANCH, 1), 0xC000_0000, sopp(S_BRANCH, 0xFFFF)];
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(cfg.blocks.keys().copied().collect::<Vec<_>>(), [0, 2]);
    assert_eq!(cfg.blocks[&2].terminator, Terminator::Jump(2));
}

#[test]
fn a_graph_that_leaves_the_code_is_refused() {
    assert_eq!(
        Cfg::build(&[sopp(S_BRANCH, 5), S_ENDPGM]),
        Err(CfgError::Target {
            pc: 0,
            target: 6,
            len: 2
        })
    );
    assert_eq!(
        Cfg::build(&[sopp(S_BRANCH, 0xFFFE)]),
        Err(CfgError::Target {
            pc: 0,
            target: -1,
            len: 1
        })
    );
    assert_eq!(
        Cfg::build(&[vop1(V_MOV, 0, V + 1)]),
        Err(CfgError::RunsOff { pc: 1 })
    );
    assert_eq!(
        Cfg::build(&[0xC000_0000]),
        Err(CfgError::Undecoded {
            pc: 0,
            word: 0xC000_0000
        })
    );
    // The branch lands on the literal of the two-word instruction at word 1.
    assert_eq!(
        Cfg::build(&[
            sopp(S_CBRANCH_SCC1, 1),
            vop1(V_MOV, 0, LITERAL),
            S_ENDPGM,
            S_ENDPGM
        ]),
        Err(CfgError::Overlap { pc: 1, inside: 2 })
    );
}

fn program(kind: ShaderKind, words: Vec<u32>) -> Program {
    Program {
        kind,
        slot: 0,
        words,
        wave32: false,
        // Attribute `n` is the vertex stage's parameter `n + 1`.
        inputs: (1..4).map(|slot| PixelInput { slot, flags: 2 }).collect(),
        vertex_inputs: vec![VertexInput {
            register: 0,
            mask: 0b0111,
            component_type: 3,
        }],
        // A pixel program reads the position's x and y, then the facing.
        position_read: 0b0011,
        front_face: true,
        launch: None,
    }
}

fn sopc(op: u32, src0: u32, src1: u32) -> u32 {
    0xBF00_0000 | op << 16 | src1 << 8 | src0
}

fn vopc(op: u32, src0: u32, src1: u32) -> u32 {
    0x7C00_0000 | op << 17 | src1 << 9 | src0
}

/// A scalar memory instruction with an immediate offset and no offset register.
fn smem(op: u32, data: u32, base: u32, offset: u32) -> [u32; 2] {
    [
        0xF400_0000 | op << 18 | data << 6 | (base / 2),
        offset | 0x7D << 25,
    ]
}

fn swizzle(dst: u32, src: u32, pattern: u32) -> [u32; 2] {
    [0xD800_0000 | DS_SWIZZLE << 18 | pattern, src | dst << 24]
}

/// `image_sample` of a two-dimensional image, all four components, address and data from `v0`.
fn sample(rsrc: u32, samp: u32) -> [u32; 2] {
    [
        0xF000_0000 | IMAGE_SAMPLE << 18 | 0xF << 8 | 1 << 3,
        (rsrc / 4) << 16 | (samp / 4) << 21,
    ]
}

const S_MOVK: u32 = 0x0;
const S_ADD_U32: u32 = 0x0;
const S_ADD_I32: u32 = 0x2;
const S_CMP_EQ_U32: u32 = 0x6;
const S_CMP_LG_U32: u32 = 0x7;
const S_CBRANCH_SCC0: u32 = 4;
const S_SWAPPC: u32 = 0x21;
const S_LOAD_X2: u32 = 0x1;
const S_LOAD_X4: u32 = 0x2;
const S_LOAD_X8: u32 = 0x3;
const S_BUFFER_LOAD: u32 = 0x8;
const V_SUB_F32: u32 = 0x04;
const V_CVT_PKRTZ: u32 = 0x2F;
const V_CMPX_GT_F32: u32 = 0x14;
const DS_SWIZZLE: u32 = 0x35;
const IMAGE_SAMPLE: u32 = 0x20;
const F_HALF: u32 = 240;
/// The inline integers 0, 1 and -1.
const I_ZERO: u32 = 128;
const I_ONE: u32 = 129;
const I_MINUS_ONE: u32 = 193;

fn lowered(kind: ShaderKind, words: Vec<u32>) -> Lowered {
    let lowered = translate(&program(kind, words)).unwrap();
    validate(&lowered.module);
    lowered
}

/// A pixel program lowered with its wave as its device's subgroup.
fn waved(words: Vec<u32>) -> Lowered {
    let options = Options {
        wave: true,
        ..Options::default()
    };
    let lowered = translate_with(&program(ShaderKind::Pixel, words), &options).unwrap();
    validate(&lowered.module);
    lowered
}

fn refusal(kind: ShaderKind, words: Vec<u32>) -> RdnaError {
    translate(&program(kind, words)).unwrap_err()
}

/// Whether any statement of `block`, nested ones included, satisfies `wanted`.
fn holds(block: &naga::Block, wanted: &dyn Fn(&naga::Statement) -> bool) -> bool {
    use naga::Statement as S;
    block.iter().any(|statement| {
        wanted(statement)
            || match statement {
                S::Block(inner) => holds(inner, wanted),
                S::If { accept, reject, .. } => holds(accept, wanted) || holds(reject, wanted),
                S::Loop {
                    body, continuing, ..
                } => holds(body, wanted) || holds(continuing, wanted),
                _ => false,
            }
    })
}

/// The function the entry point calls, which holds the program's own statements.
fn body(module: &naga::Module) -> &naga::Function {
    module.functions.iter().next().unwrap().1
}

#[test]
fn a_vertex_program_of_arithmetic_and_exports_is_a_valid_module() {
    let mad = vop3(V_MAD_F32, 2, [V, V + 1, F_ONE], 0, 0b001);
    let position = exp(POS0, [0, 1, 2, 2]);
    let param = exp(32 + 3, [2, 2, 0, 1]);
    let words = vec![
        vop1(V_MOV, 0, LITERAL),
        ONE,
        vop2(V_ADD_F32, 1, F_ONE, 0),
        mad[0],
        mad[1],
        position[0],
        position[1],
        param[0],
        param[1],
        S_ENDPGM,
    ];
    let module = lowered(ShaderKind::Vertex, words).module;
    let entry = &module.entry_points[0];
    assert_eq!(entry.stage, naga::ShaderStage::Vertex);
    let result = entry.function.result.as_ref().unwrap();
    let naga::TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("the result is not a struct");
    };
    let bindings: Vec<_> = members.iter().map(|m| m.binding.clone().unwrap()).collect();
    assert!(matches!(
        bindings.as_slice(),
        [
            naga::Binding::BuiltIn(naga::BuiltIn::Position { .. }),
            naga::Binding::Location { location: 3, .. }
        ]
    ));
}

#[test]
fn a_pixel_program_reads_its_interpolants_at_their_parameter_slots() {
    // v_interp_p2_f32 v0, v1, attr2.y; v_mul_f32 v0, v0, v0; exp mrt0.
    let target = exp(0, [0, 0, 0, 0]);
    let words = vec![
        vintrp(1, 0, 2, 1, 1),
        vop2(V_MUL_F32, 0, V, 0),
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let module = lowered(ShaderKind::Pixel, words).module;
    let entry = &module.entry_points[0];
    assert_eq!(entry.stage, naga::ShaderStage::Fragment);
    // Attribute 2 is the third record of the input table, whose slot is 3.
    assert!(matches!(
        entry.function.arguments[0].binding,
        Some(naga::Binding::Location { location: 3, .. })
    ));
    // An attribute the table does not list is refused, not guessed.
    let words = vec![vintrp(1, 0, 7, 0, 1), target[0], target[1], S_ENDPGM];
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 0, .. })
    ));
}

#[test]
fn a_branch_on_the_scalar_condition_becomes_an_if() {
    let position = exp(POS0, [0, 0, 0, 0]);
    let words = vec![
        sopk(S_MOVK, 0, 1),
        sopc(S_CMP_EQ_U32, 0, I_ONE),
        sopp(S_CBRANCH_SCC0, 2),
        vop1(V_MOV, 0, F_ONE),
        sopp(S_BRANCH, 1),
        vop1(V_MOV, 0, F_HALF),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let module = lowered(ShaderKind::Vertex, words).module;
    let function = body(&module);
    assert!(holds(&function.body, &|s| matches!(
        s,
        naga::Statement::If { .. }
    )));
    assert!(!holds(&function.body, &|s| matches!(
        s,
        naga::Statement::Loop { .. }
    )));
}

#[test]
fn a_backward_branch_becomes_a_loop() {
    let position = exp(POS0, [0, 0, 0, 0]);
    let words = vec![
        sopk(S_MOVK, 0, 3),
        vop1(V_MOV, 0, F_ONE),
        sop2(S_ADD_I32, 0, 0, I_MINUS_ONE),
        sopc(S_CMP_LG_U32, 0, I_ZERO),
        // Back to the addition at word 2.
        sopp(S_CBRANCH_SCC1, 0xFFFD),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let module = lowered(ShaderKind::Vertex, words).module;
    assert!(holds(&body(&module).body, &|s| matches!(
        s,
        naga::Statement::Loop { .. }
    )));
}

#[test]
fn a_lane_switched_off_is_discarded_and_packed_halves_are_exported() {
    // v0 = attr0.x; the lane stays on where 1.0 > v0; v1 = (v0, v0) as halves; exp mrt0 compr.
    let words = vec![
        vintrp(1, 0, 0, 0, 1),
        vopc(V_CMPX_GT_F32, F_ONE, 0),
        vop2(V_CVT_PKRTZ, 1, V, 0),
        0xF800_0000 | 1 << 11 | 1 << 10 | 0xF,
        u32::from_le_bytes([1, 1, 0, 0]),
        S_ENDPGM,
    ];
    let masked = lowered(ShaderKind::Pixel, words);
    let function = body(&masked.module);
    assert!(holds(&function.body, &|s| matches!(
        s,
        naga::Statement::Kill
    )));
    // The halves are packed by integer arithmetic, toward zero as the instruction rounds.
    assert!(masked.approximations.is_empty());
    assert!(!packs_halves(&masked.module));
    // With no instruction that masks lanes, nothing is discarded.
    let target = exp(0, [0, 0, 0, 0]);
    let plain = vec![vintrp(1, 0, 0, 0, 1), target[0], target[1], S_ENDPGM];
    let plain = lowered(ShaderKind::Pixel, plain).module;
    assert!(!holds(&body(&plain).body, &|s| matches!(
        s,
        naga::Statement::Kill
    )));
}

#[test]
fn tables_lead_to_a_constant_buffer_a_texture_and_a_sampler() {
    // s2 points at a table: a buffer descriptor at 0x10, an image's at 0x40, a sampler's at 0x80.
    let buffer = smem(S_LOAD_X4, 4, 2, 0x10);
    let read = smem(S_BUFFER_LOAD, 20, 4, 0x24);
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let sampler = smem(S_LOAD_X4, 16, 2, 0x80);
    let texel = sample(8, 16);
    let target = exp(0, [0, 1, 2, 3]);
    let words = vec![
        buffer[0],
        buffer[1],
        read[0],
        read[1],
        vop1(V_MOV, 0, 20),
        vop1(V_MOV, 1, 20),
        image[0],
        image[1],
        sampler[0],
        sampler[1],
        texel[0],
        texel[1],
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    let at = |offset| {
        Some(Loc {
            root: 2,
            path: Vec::new(),
            offset,
            index: None,
        })
    };
    assert_eq!(
        out.resources,
        [
            Resource {
                // The read at byte 0x24 is the tenth dword, in the third row of four.
                kind: ResourceKind::ConstantBuffer { rows: 3 },
                at: at(0x10),
                words: None,
                group: 0,
                binding: 0,
                count: None,
                written: false,
            },
            Resource {
                kind: ResourceKind::Texture {
                    dim: 1,
                    compared: false
                },
                at: at(0x40),
                words: None,
                group: 0,
                binding: 1,
                count: None,
                written: false,
            },
            Resource {
                kind: ResourceKind::Sampler { comparison: false },
                at: at(0x80),
                words: None,
                group: 0,
                binding: 2,
                count: None,
                written: false,
            },
        ]
    );
}

#[test]
fn a_table_reached_through_another_keeps_its_path() {
    // s[4:5] = the pointer at s2's table + 8; s[8:11] = that table's words from 0x20.
    let pointer = smem(S_LOAD_X2, 4, 2, 8);
    let words = smem(S_LOAD_X4, 8, 4, 0x20);
    let code = [pointer[0], pointer[1], words[0], words[1], S_ENDPGM];
    let cfg = Cfg::build(&code).unwrap();
    let mut state = sym::entry();
    for (_, inst) in &cfg.blocks[&0].insts {
        sym::step(&mut state, inst);
    }
    assert_eq!(
        sym::descriptor(&state, 8, 4),
        Some(Loc {
            root: 2,
            path: vec![8],
            offset: 0x20,
            index: None,
        })
    );
    // Three words of it are no four-word descriptor from the second.
    assert_eq!(sym::descriptor(&state, 9, 4), None);
}

#[test]
fn two_lanes_of_a_quad_subtracted_are_a_derivative() {
    // v1 = v0 of the quad's second lane, v2 = v0 of its first, v3 = v1 - v2: the coarse x derivative.
    let (right, left) = (swizzle(1, 0, 0x8055), swizzle(2, 0, 0x8000));
    let target = exp(0, [3, 3, 3, 3]);
    let words = vec![
        vintrp(1, 0, 0, 0, 1),
        right[0],
        right[1],
        left[0],
        left[1],
        vop2(V_SUB_F32, 3, V + 1, 2),
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let module = lowered(ShaderKind::Pixel, words).module;
    assert!(body(&module).expressions.iter().any(|(_, e)| matches!(
        e,
        naga::Expression::Derivative {
            axis: naga::DerivativeAxis::X,
            ctrl: naga::DerivativeControl::Coarse,
            ..
        }
    )));
    // A neighbouring lane's value read any other way goes by the fine derivatives of its halves.
    let target = exp(0, [1, 1, 1, 1]);
    let words = vec![
        vintrp(1, 0, 0, 0, 1),
        right[0],
        right[1],
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    assert!(out
        .approximations
        .contains_key("a neighbouring lane's value read through the device's fine derivatives"));
}

/// An image instruction of dimension `dim` whose address and data start at `v0`; `flags` holds
/// the bits of 16-bit addresses (30) and data (31).
fn mimg(name: &str, dim: u32, dmask: u32, flags: u32) -> [u32; 2] {
    let number = op(opcodes::MIMG, name);
    [
        0xF000_0000 | (number & 0x7F) << 18 | number >> 7 | dmask << 8 | dim << 3,
        (8 / 4) << 16 | (16 / 4) << 21 | flags,
    ]
}

#[test]
fn every_image_form_is_a_valid_module() {
    for (name, dim, dmask, flags) in [
        ("image_gather4_lz", 1, 0b0010, 0),
        ("image_gather4_c_lz", 1, 0b0001, 0),
        ("image_get_lod", 1, 0b0011, 0),
        ("image_get_resinfo", 5, 0b1111, 0),
        ("image_sample_lz", 3, 0b1111, 0),
        ("image_sample_l", 0, 0b1111, 0),
        ("image_sample", 5, 0b0111, 0),
        ("image_sample_b", 1, 0b1111, 1 << 30),
        ("image_load", 0, 0b1111, 1 << 31),
        ("image_load_mip", 5, 0b1111, 1 << 30),
        // Gradients of a cube lie on its face; a least level stretches them.
        ("image_sample_d", 3, 0b0111, 0),
        ("image_sample_d_cl", 1, 0b1111, 0),
        ("image_sample_d_cl", 3, 0b0111, 0),
        ("image_sample_d_cl", 5, 0b0111, 0),
        // A gather at a stated level is its four texels; a texel offset is the first address.
        ("image_gather4_l", 1, 0b0001, 0),
        ("image_gather4_l_o", 1, 0b0001, 0),
        ("image_gather4_lz_o", 1, 0b1000, 0),
        ("image_sample_o", 1, 0b0111, 0),
        ("image_sample_o", 5, 0b0111, 0),
    ] {
        let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
        let read = mimg(name, dim, dmask, flags);
        let target = exp(0, [0, 1, 2, 3]);
        let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
        words.extend((0..10).map(|v| vop1(V_MOV, v, F_ONE)));
        words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
        let out = translate(&program(ShaderKind::Pixel, words));
        let out = out.unwrap_or_else(|e| panic!("{name} of dimension {dim}: {e}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&out.module)
            .unwrap_or_else(|e| panic!("{name} of dimension {dim}: {e:?}"));
        validate(&out.module);
    }
    // A gather of a three-dimensional image has no four texels.
    let read = mimg("image_gather4_lz", 2, 1, 0);
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let words = vec![image[0], image[1], read[0], read[1], S_ENDPGM];
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 2, .. })
    ));
}

#[test]
fn the_bit_and_byte_forms_are_a_valid_module() {
    let three = |name: &str, dst: u32, srcs: [u32; 3]| vop3(op3(name), dst, srcs, 0, 0);
    let regs = [V + 1, V + 2, V + 3];
    let position = exp(POS0, [4, 5, 6, 7]);
    let mut words: Vec<u32> = (1..4).map(|v| vop1(V_MOV, v, F_ONE)).collect();
    for inst in [
        three("v_perm_b32", 4, regs),
        three("v_cubeid_f32", 5, regs),
        three("v_cubema_f32", 5, regs),
        three("v_cubesc_f32", 6, regs),
        three("v_cubetc_f32", 6, regs),
        three("v_cvt_pk_u8_f32", 7, regs),
        three("v_add_lshl_u32", 8, regs),
        three("v_bcnt_u32_b32", 8, regs),
        three("v_alignbit_b32", 8, regs),
        three("v_alignbyte_b32", 8, regs),
        three("v_bfe_i32", 8, regs),
        three("v_mad_i32_i24", 8, regs),
        three("v_bfm_b32", 8, regs),
        three("v_add_co_u32", 8, regs),
        // A scalar kept in lane 3 of v9 and read back into s21.
        three("v_writelane_b32", 9, [I_ONE, I_ZERO + 3, 0]),
        three("v_readlane_b32", 21, [V + 9, I_ZERO + 3, 0]),
    ] {
        words.extend(inst);
    }
    words.extend([
        vop1(op(opcodes::VOP1, "v_ffbh_u32"), 8, V + 8),
        vop1(op(opcodes::VOP1, "v_ffbh_i32"), 8, V + 8),
        vop1(op(opcodes::VOP1, "v_swap_b32"), 8, V + 4),
        // The compare leaves the carry the next two read.
        vopc(op(opcodes::VOPC, "v_cmp_lt_u64"), V + 1, 2),
        vop2(op(opcodes::VOP2, "v_addc_co_u32"), 8, V + 1, 2),
        vop2(op(opcodes::VOP2, "v_subbrev_co_u32"), 8, V + 1, 2),
        sop1(op(opcodes::SOP1, "s_bitset1_b32"), 21, I_ZERO + 7),
        sop1(op(opcodes::SOP1, "s_bitset0_b32"), 21, I_ZERO + 7),
        vop1(V_MOV, 7, 21),
        position[0],
        position[1],
        S_ENDPGM,
    ]);
    let out = lowered(ShaderKind::Vertex, words);
    // The lane was one the program wrote, so nothing was read as this lane's own.
    assert!(!out.approximations.contains_key("a lane read as this lane"));
}

#[test]
fn a_vertex_program_finds_the_instance_in_its_fourth_register_and_exports_clip_distances() {
    let (position, clip) = (exp(POS0, [0, 0, 0, 3]), exp(13, [3, 3, 3, 3]));
    let words = vec![position[0], position[1], clip[0], clip[1], S_ENDPGM];
    let module = lowered(ShaderKind::Vertex, words).module;
    let entry = &module.entry_points[0];
    assert!((entry.function.arguments.iter())
        .any(|a| a.binding == Some(naga::Binding::BuiltIn(naga::BuiltIn::InstanceIndex))));
    let naga::TypeInner::Struct { members, .. } =
        &module.types[entry.function.result.as_ref().unwrap().ty].inner
    else {
        panic!("the outputs are no struct");
    };
    assert!((members.iter())
        .any(|m| m.binding == Some(naga::Binding::BuiltIn(naga::BuiltIn::ClipDistance))));
}

/// A shared-memory instruction at the address in `v0` plus `offset` bytes, with data and destination `v1`.
fn ds(name: &str, offset: u32) -> [u32; 2] {
    let number = op(opcodes::DS, name);
    [0xD800_0000 | number << 18 | offset, 1 << 8 | 1 << 24]
}

#[test]
fn a_compute_program_shares_memory_behind_a_barrier_and_stores_to_a_buffer() {
    // Two user registers, the group's number along x after them, 8 by 8 threads, 512 bytes shared.
    let launch = Launch {
        user_registers: 2,
        group_axes: 0b001,
        group_size: false,
        threads: [8, 8, 1],
        shared_dwords: 128,
    };
    let descriptor = smem(S_LOAD_X4, 4, 0, 0x20);
    let (write, read) = (ds("ds_write_b32", 4), ds("ds_read_b32", 4));
    let store = mubuf(op(opcodes::MUBUF, "buffer_store_dword"), 1, 0, 4, true);
    let words = vec![
        // v1 = the thread's row plus the group's number; v0 stays its column.
        vop2(op(opcodes::VOP2, "v_add_u32"), 1, 2, 1),
        write[0],
        write[1],
        sopp(op(opcodes::SOPP, "s_barrier"), 0),
        read[0],
        read[1],
        descriptor[0],
        descriptor[1],
        store[0],
        store[1],
        S_ENDPGM,
    ];
    let program = Program {
        launch: Some(launch),
        ..program(ShaderKind::Compute, words)
    };
    let out = translate(&program).unwrap();
    validate(&out.module);
    let entry = &out.module.entry_points[0];
    assert_eq!(entry.stage, naga::ShaderStage::Compute);
    assert_eq!(entry.workgroup_size, [8, 8, 1]);
    for wanted in [naga::BuiltIn::LocalInvocationId, naga::BuiltIn::WorkGroupId] {
        assert!((entry.function.arguments.iter())
            .any(|a| a.binding == Some(naga::Binding::BuiltIn(wanted))));
    }
    assert!((out.module.global_variables.iter())
        .any(|(_, global)| global.space == naga::AddressSpace::WorkGroup));
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        naga::Statement::ControlBarrier(_)
    )));
    assert!(out.resources.iter().any(|r| r.written));
    // The group's size in a register is a launch this front end does not make.
    let program = Program {
        launch: Some(Launch {
            group_size: true,
            ..launch
        }),
        ..program
    };
    assert_eq!(
        translate(&program).unwrap_err(),
        RdnaError::NotTranslated(NotTranslated::Stage("compute".to_owned()))
    );
}

#[test]
fn a_permutation_folded_into_a_sum_and_a_comparison_of_parts_are_lowered() {
    // v2 = v0 as the lane to the right computed it, plus v0; s20 = whether v0 equals v1, whole registers.
    let target = exp(0, [2, 2, 2, 2]);
    let words = vec![
        vintrp(1, 0, 0, 0, 1),
        vintrp(1, 1, 0, 1, 1),
        vop2(V_ADD_F32, 2, 250, 0),
        0x55 << 8 | 0xFF << 24,
        vopc(op(opcodes::VOPC, "v_cmp_eq_u32"), 249, 1),
        20 << 8 | 1 << 15 | 6 << 16 | 6 << 24,
        sop1(op(opcodes::SOP1, "s_bcnt1_i32_b64"), 22, 126),
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let cfg = Cfg::build(&words).unwrap();
    let mut state = sym::entry();
    for (pc, inst) in &cfg.blocks[&0].insts {
        sym::step_at(&mut state, *pc, inst, false);
    }
    // The comparison wrote the mask register its word names, and not the comparison's own.
    assert_eq!(state[20], sym::Sym::Unknown);
    assert_eq!(state[usize::from(sym::VCC)], sym::Sym::User(sym::VCC));
    let out = waved(words);
    let note = "a neighbouring lane's value read through the device's fine derivatives";
    assert!(out.approximations.contains_key(note));
    // The count is over the wave's own mask: every lane's bit, gathered by a vote of the subgroup.
    assert!(out.subgroups);
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        naga::Statement::SubgroupBallot { .. }
    )));
}

#[test]
fn a_part_selection_takes_the_float_modifiers() {
    // v1 = twice the low byte of v0 as a float, clamped: bits 13 and 14 of the word.
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        vop1(op(opcodes::VOP1, "v_cvt_f32_u32"), 1, 249),
        6 << 8 | 1 << 13 | 1 << 14,
        position[0],
        position[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Vertex, words);
    // An integer result takes none.
    let words = vec![
        vop1(op(opcodes::VOP1, "v_not_b32"), 1, 249),
        6 << 8 | 6 << 16 | 1 << 13,
        position[0],
        position[1],
        S_ENDPGM,
    ];
    assert!(matches!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 0, .. })
    ));
}

#[test]
fn the_fetch_call_becomes_vertex_inputs() {
    // The signature's one element, register 0, arrives in v4..v7.
    let position = exp(POS0, [4, 5, 6, 7]);
    let words = vec![sop1(S_SWAPPC, 4, 4), position[0], position[1], S_ENDPGM];
    let module = lowered(ShaderKind::Vertex, words).module;
    assert!(matches!(
        module.entry_points[0].function.arguments[0].binding,
        Some(naga::Binding::Location { location: 0, .. })
    ));
}

#[test]
fn what_is_not_translated_is_named() {
    // Registers that hold a sampler's words on one path and user data on the other name no sampler.
    let (image, table) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 4, 2, 0));
    let read = sample(8, 4);
    let words = vec![
        image[0],
        image[1],
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 2),
        table[0],
        table[1],
        read[0],
        read[1],
        S_ENDPGM,
    ];
    assert_eq!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor {
            pc: 6,
            what: "a sampler whose descriptor it did not load from a table"
        })
    );
    // A trace word, which no lowering reads.
    let refused = refusal(ShaderKind::Vertex, vec![sopp(0x28, 0), S_ENDPGM]);
    assert_eq!(
        refused,
        RdnaError::NotTranslated(NotTranslated::Opcode {
            pc: 0,
            name: "s_ttracedata_imm".to_owned()
        })
    );
    let RdnaError::NotTranslated(refused) = refused else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(refused.name(), "s_ttracedata_imm");
    // A buffer read through registers no table filled.
    let load = mubuf(op(opcodes::MUBUF, "buffer_load_dword"), 0, 0, 0, false);
    assert_eq!(
        refusal(ShaderKind::Vertex, vec![load[0], load[1], S_ENDPGM]),
        RdnaError::NotTranslated(NotTranslated::Descriptor {
            pc: 0,
            what: "a buffer whose descriptor it did not load from a table"
        })
    );
    let position = exp(POS0, [0, 0, 0, 0]);
    assert_eq!(
        refusal(
            ShaderKind::Vertex,
            vec![vop1(V_MOV, 0, V + 5), position[0], position[1], S_ENDPGM]
        ),
        RdnaError::NotTranslated(NotTranslated::LiveIn {
            pc: 0,
            register: "v5".to_owned()
        })
    );
    // A compute program whose package states no launch.
    assert_eq!(
        refusal(ShaderKind::Compute, vec![S_ENDPGM]),
        RdnaError::NotTranslated(NotTranslated::Stage("compute".to_owned()))
    );
    assert_eq!(
        refusal(ShaderKind::Vertex, vec![S_ENDPGM]),
        RdnaError::NotTranslated(NotTranslated::NoPosition)
    );
}

/// The number of a mnemonic in its family's table.
fn op(table: &[(u16, &str)], name: &str) -> u32 {
    u32::from(table.iter().find(|row| row.1 == name).unwrap().0)
}

/// The number of a mnemonic of the three-source family.
fn op3(name: &str) -> u32 {
    u32::from(opcodes::VOP3.iter().find(|row| row.1 == name).unwrap().0)
}

/// A scalar memory instruction whose offset register is `soffset`.
fn smem_at(op: u32, data: u32, base: u32, offset: u32, soffset: u32) -> [u32; 2] {
    [
        0xF400_0000 | op << 18 | data << 6 | (base / 2),
        offset | soffset << 25,
    ]
}

/// A buffer instruction with no immediate offset and no scalar one: the address register is an
/// index where `idxen`, and reads nothing otherwise.
fn mubuf(op: u32, data: u32, addr: u32, rsrc: u32, idxen: bool) -> [u32; 2] {
    [
        0xE000_0000 | (op & 0x7F) << 18 | (op >> 7) << 25 | u32::from(idxen) << 13,
        addr | data << 8 | (rsrc / 4) << 16 | I_ZERO << 24,
    ]
}

fn plain(root: u8, offset: u32) -> Loc {
    Loc {
        root,
        path: Vec::new(),
        offset,
        index: None,
    }
}

#[test]
fn a_graph_that_does_not_nest_runs_in_a_dispatch_loop() {
    // The entry branches into the middle of a loop that the block after it enters at its head.
    let colour = exp(0, [0, 0, 0, 0]);
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 1),
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_LG_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
        colour[0],
        colour[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    assert!(out
        .approximations
        .contains_key("a dispatch loop for a graph that does not nest"));
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        naga::Statement::Switch { .. }
    )));
}

#[test]
fn a_table_read_at_an_index_names_where_the_index_comes_from() {
    // s14 = s9 << 4; s[16:19] = the table at s10, at that index.
    let load = smem_at(S_LOAD_X4, 16, 10, 0, 14);
    let shift = sop2(op(opcodes::SOP2, "s_lshl_b32"), 14, 9, I_ZERO + 4);
    let code = [shift, load[0], load[1], S_ENDPGM];
    let cfg = Cfg::build(&code).unwrap();
    let mut state = sym::entry();
    for (_, inst) in &cfg.blocks[&0].insts {
        sym::step(&mut state, inst);
    }
    assert_eq!(
        sym::descriptor(&state, 16, 4),
        Some(Loc {
            index: Some(Index {
                source: IndexSource::User(9),
                shift: 4
            }),
            ..plain(10, 0)
        })
    );

    // s20 = a constant buffer's dword at 0x24, shifted by 5: the index of an image in the table at s12.
    let buffer = smem(S_LOAD_X4, 4, 2, 0x10);
    let read = smem(S_BUFFER_LOAD, 20, 4, 0x24);
    let shift = sop2(op(opcodes::SOP2, "s_lshl_b32"), 20, 20, I_ZERO + 5);
    let image = smem_at(S_LOAD_X8, 8, 12, 0, 20);
    let code = [
        buffer[0], buffer[1], read[0], read[1], shift, image[0], image[1], S_ENDPGM,
    ];
    let cfg = Cfg::build(&code).unwrap();
    let mut state = sym::entry();
    for (_, inst) in &cfg.blocks[&0].insts {
        sym::step(&mut state, inst);
    }
    assert_eq!(
        sym::descriptor(&state, 8, 8),
        Some(Loc {
            index: Some(Index {
                source: IndexSource::Data {
                    buffer: Box::new(plain(2, 0x10)),
                    offset: 0x24,
                    plus: None
                },
                shift: 5
            }),
            ..plain(12, 0)
        })
    );
    // An index the program computed any other way is a place only a run knows, told by its load.
    let add = sop2(S_ADD_U32, 14, 9, 9);
    let code = [add, load[0], load[1], S_ENDPGM];
    let cfg = Cfg::build(&code).unwrap();
    let mut state = sym::entry();
    for (pc, inst) in &cfg.blocks[&0].insts {
        sym::step_at(&mut state, *pc, inst, false);
    }
    assert_eq!(
        sym::descriptor(&state, 16, 4),
        Some(Loc {
            index: Some(Index {
                source: IndexSource::Runtime { site: 1 },
                shift: 0
            }),
            ..plain(10, 0)
        })
    );
    // A table whose pointer the program computed is still refused.
    let pointer = sop2(S_ADD_U32, 10, 9, 9);
    let load = smem(S_LOAD_X4, 16, 10, 0);
    let code = [pointer, load[0], load[1], S_ENDPGM];
    assert_eq!(
        refusal(ShaderKind::Vertex, code.to_vec()),
        RdnaError::NotTranslated(NotTranslated::Descriptor {
            pc: 1,
            what: "a table through a pointer or at an offset the program computed"
        })
    );
}

/// The whole table a user register points at, as a binding array is listed.
fn table(root: u8) -> Loc {
    Loc {
        index: Some(Index {
            source: IndexSource::Runtime { site: 0 },
            shift: 0,
        }),
        ..plain(root, 0)
    }
}

#[test]
fn a_descriptor_at_a_computed_place_indexes_its_whole_table() {
    // s30 = s9 + s9; an image and a sampler of the table at s2, at that offset.
    let image = smem_at(S_LOAD_X8, 8, 2, 0x40, 30);
    let sampler = smem_at(S_LOAD_X4, 16, 2, 0, 30);
    let texel = sample(8, 16);
    let target = exp(0, [0, 1, 2, 3]);
    let words = vec![
        sop2(S_ADD_U32, 30, 9, 9),
        image[0],
        image[1],
        sampler[0],
        sampler[1],
        vop1(V_MOV, 0, F_ONE),
        vop1(V_MOV, 1, F_ONE),
        texel[0],
        texel[1],
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    let listed = |kind: ResourceKind| {
        (out.resources.iter())
            .any(|r| r.kind == kind && r.at == Some(table(2)) && r.count.is_some())
    };
    assert!(listed(ResourceKind::Texture {
        dim: 1,
        compared: false
    }));
    assert!(listed(ResourceKind::Sampler { comparison: false }));
    let arrays = (out.module.types.iter())
        .filter(|(_, ty)| matches!(ty.inner, naga::TypeInner::BindingArray { .. }))
        .count();
    assert_eq!(arrays, 2);
}

#[test]
fn a_buffer_at_a_computed_place_and_a_constant_buffer_of_one_are_read() {
    // s14 = s9 + s9; a buffer of the table at s2 at that offset, read as a typed element whose
    // format its header states, then a constant buffer of the same table read as a number.
    let descriptor = smem_at(S_LOAD_X4, 4, 2, 0, 14);
    let typed = mubuf(op(opcodes::MUBUF, "buffer_load_format_x"), 4, 0, 4, true);
    let block = smem_at(S_LOAD_X4, 20, 2, 0x20, 14);
    let read = smem(S_BUFFER_LOAD, 24, 20, 0x8);
    let position = exp(POS0, [4, 5, 4, 4]);
    let words = vec![
        sop2(S_ADD_U32, 14, 9, 9),
        descriptor[0],
        descriptor[1],
        typed[0],
        typed[1],
        block[0],
        block[1],
        read[0],
        read[1],
        vop1(V_MOV, 5, 24),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    let listed = |kind: ResourceKind| {
        (out.resources.iter())
            .any(|r| r.kind == kind && r.at == Some(table(2)) && r.count.is_some())
    };
    assert!(listed(ResourceKind::Buffer { stride: None }));
    assert!(listed(ResourceKind::ConstantBuffer { rows: 4096 }));
}

#[test]
fn memory_the_code_describes_is_read_by_a_scalar_load_too() {
    let s_mov = op(opcodes::SOP1, "s_mov_b32");
    let read = smem(S_BUFFER_LOAD, 20, 0, 0x10);
    let position = exp(POS0, [4, 4, 4, 4]);
    // The stride bit is set the other way a descriptor takes it: one bit of the word.
    let words = vec![
        sop1(s_mov, 0, 9),
        sop1(s_mov, 1, 10),
        sop1(op(opcodes::SOP1, "s_bitset1_b32"), 1, I_ZERO + 16),
        sop1(s_mov, 2, I_MINUS_ONE),
        sop1(s_mov, 3, LITERAL),
        20 << 12 | 0xFAC,
        read[0],
        read[1],
        vop1(V_MOV, 4, 20),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!((out.resources.iter())
        .any(|r| r.kind == ResourceKind::Buffer { stride: Some(1) } && r.at == Some(plain(9, 0))));
}

#[test]
fn a_lane_mask_is_one_register_in_a_program_32_lanes_wide() {
    // A comparison into s7 leaves the table pointer in s8 alone where a mask is one register.
    let compare = vop3(op(opcodes::VOPC, "v_cmp_eq_u32"), 7, [V, V + 1, 0], 0, 0);
    let (inst, _) = decode_one(&compare, 0).unwrap();
    for (wave32, kept) in [(true, true), (false, false)] {
        let mut state = sym::entry();
        sym::step_at(&mut state, 0, &inst, wave32);
        assert_eq!(state[8] == sym::Sym::User(8), kept);
        assert_eq!(state[7], sym::Sym::Unknown);
    }
    // A constant the code reverses is still a constant: a sampler's word is written that way.
    let reverse = [sop1(op(opcodes::SOP1, "s_brev_b32"), 18, LITERAL), 1];
    let (inst, _) = decode_one(&reverse, 0).unwrap();
    let mut state = sym::entry();
    sym::step_at(&mut state, 0, &inst, false);
    assert_eq!(state[18], sym::Sym::Const(0x8000_0000));
}

#[test]
fn a_sampler_the_code_writes_is_bound_by_its_words() {
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let texel = sample(8, 16);
    let target = exp(0, [0, 1, 2, 3]);
    let words = vec![
        image[0],
        image[1],
        sopk(S_MOVK, 16, 0x92),
        sopk(S_MOVK, 17, 0),
        sopk(S_MOVK, 18, 0),
        sopk(S_MOVK, 19, 0),
        vop1(V_MOV, 0, F_ONE),
        vop1(V_MOV, 1, F_ONE),
        texel[0],
        texel[1],
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(
        out.resources[1],
        Resource {
            kind: ResourceKind::Sampler { comparison: false },
            at: None,
            words: Some([0x92, 0, 0, 0]),
            group: 0,
            binding: 1,
            count: None,
            written: false
        }
    );
}

#[test]
fn a_sampler_one_of_two_the_code_writes_is_both_and_a_run_chooses() {
    // s16 is 0x92 where s9 is 1 and 0 elsewhere; the other three words are written once.
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let texel = sample(8, 16);
    let target = exp(0, [0, 1, 2, 3]);
    let words = vec![
        image[0],
        image[1],
        sopc(S_CMP_EQ_U32, 9, I_ONE),
        sopp(S_CBRANCH_SCC0, 2),
        sopk(S_MOVK, 16, 0x92),
        sopp(S_BRANCH, 1),
        sopk(S_MOVK, 16, 0),
        sopk(S_MOVK, 17, 0),
        sopk(S_MOVK, 18, 0),
        sopk(S_MOVK, 19, 0),
        vop1(V_MOV, 0, F_ONE),
        vop1(V_MOV, 1, F_ONE),
        texel[0],
        texel[1],
        target[0],
        target[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    let written: Vec<_> = (out.resources.iter())
        .filter(|r| r.kind == ResourceKind::Sampler { comparison: false })
        .map(|r| r.words)
        .collect();
    assert_eq!(written, [Some([0, 0, 0, 0]), Some([0x92, 0, 0, 0])]);
}

#[test]
fn a_read_of_one_register_leaves_a_sampler_in_the_registers_beside_it() {
    // The sampler's words wait in v20..v23 while one component is read into v19.
    let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
    let first = op(opcodes::VOP1, "v_readfirstlane_b32");
    let one = [
        0xF000_0000 | IMAGE_SAMPLE << 18 | 1 << 8 | 1 << 3,
        19 << 8 | (8 / 4) << 16 | (16 / 4) << 21,
    ];
    let (read, target) = (sample(8, 24), exp(0, [0, 1, 2, 3]));
    let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend((0..4).map(|k| vop1(V_MOV, 20 + k, 16 + k)));
    words.extend(one);
    words.extend((0..4).map(|k| vop1(first, 24 + k, V + 20 + k)));
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words);
    assert!((out.resources.iter()).any(|r| {
        r.kind == ResourceKind::Sampler { comparison: false } && r.at == Some(plain(2, 0x20))
    }));
}

#[test]
fn a_pixel_program_finds_the_position_and_the_facing_after_the_barycentrics() {
    // The helper's signature reads x and y, then the facing: v2, v3 and v4.
    let colour = exp(0, [2, 3, 4, 4]);
    let module = lowered(ShaderKind::Pixel, vec![colour[0], colour[1], S_ENDPGM]).module;
    let arguments = &module.entry_points[0].function.arguments;
    let built_in = |wanted: naga::BuiltIn| {
        (arguments.iter()).any(|a| a.binding == Some(naga::Binding::BuiltIn(wanted)))
    };
    assert!(built_in(naga::BuiltIn::Position { invariant: false }));
    assert!(built_in(naga::BuiltIn::FrontFacing));
    // The register after them holds nothing the signature names.
    let colour = exp(0, [5, 5, 5, 5]);
    assert_eq!(
        refusal(ShaderKind::Pixel, vec![colour[0], colour[1], S_ENDPGM]),
        RdnaError::NotTranslated(NotTranslated::LiveIn {
            pc: 0,
            register: "v5".to_owned()
        })
    );
}

#[test]
fn a_buffer_is_read_by_address_with_its_stride_from_its_header_or_from_the_code() {
    // A table's buffer, four dwords at the vertex index times the stride its header states.
    let descriptor = smem(S_LOAD_X4, 4, 2, 0x20);
    let load = mubuf(op(opcodes::MUBUF, "buffer_load_dwordx4"), 4, 0, 4, true);
    let store = mubuf(op(opcodes::MUBUF, "buffer_store_dword"), 4, 0, 4, true);
    let position = exp(POS0, [4, 5, 6, 7]);
    let words = vec![
        descriptor[0],
        descriptor[1],
        load[0],
        load[1],
        store[0],
        store[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert_eq!(
        out.resources,
        [Resource {
            kind: ResourceKind::Buffer { stride: None },
            at: Some(plain(2, 0x20)),
            words: None,
            group: 0,
            binding: 0,
            count: None,
            written: false
        }]
    );
    assert!(out
        .approximations
        .contains_key("a buffer store before the pixel stage, dropped"));

    // A descriptor the code builds around the address in s9 and s10: a stride of one byte, 32-bit unsigned elements.
    let s_mov = op(opcodes::SOP1, "s_mov_b32");
    let typed = mubuf(op(opcodes::MUBUF, "buffer_load_format_x"), 4, 0, 0, true);
    let position = exp(POS0, [4, 4, 4, 4]);
    let words = vec![
        sop1(s_mov, 0, 9),
        sop2(op(opcodes::SOP2, "s_or_b32"), 1, 10, LITERAL),
        0x1_0000,
        sop1(s_mov, 2, I_MINUS_ONE),
        sop1(s_mov, 3, LITERAL),
        20 << 12 | 0xFAC,
        typed[0],
        typed[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!(out.resources.contains(&Resource {
        kind: ResourceKind::Buffer { stride: Some(1) },
        at: Some(plain(9, 0)),
        words: None,
        group: 0,
        binding: 0,
        count: None,
        written: false
    }));
}

#[test]
fn a_parameter_read_as_halves_crosses_the_stages_as_two_vectors() {
    // The pixel program interpolates the high half of attribute 0's x, which is parameter 1.
    let read = vop3(op3("v_interp_p2_f16"), 4, [V, V + 1, V], 0, 0);
    let colour = exp(0, [4, 4, 4, 4]);
    let out = lowered(
        ShaderKind::Pixel,
        vec![read[0], read[1], colour[0], colour[1], S_ENDPGM],
    );
    assert_eq!(out.half_slots, std::collections::BTreeSet::from([1]));
    assert!(matches!(
        out.module.entry_points[0].function.arguments[0].binding,
        Some(naga::Binding::Location { location: 33, .. })
    ));

    // Told so, the vertex program exports parameter 1 as its low halves and its high halves.
    let (position, parameter) = (exp(POS0, [0, 0, 0, 0]), exp(33, [0, 0, 0, 0]));
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        position[0],
        position[1],
        parameter[0],
        parameter[1],
        S_ENDPGM,
    ];
    let options = Options {
        half_slots: out.half_slots,
        ..Options::default()
    };
    let module = translate_with(&program(ShaderKind::Vertex, words), &options)
        .unwrap()
        .module;
    validate(&module);
    let result = module.entry_points[0].function.result.as_ref().unwrap();
    let naga::TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("the vertex stage returns no structure");
    };
    let locations: Vec<u32> = (members.iter())
        .filter_map(|member| match member.binding {
            Some(naga::Binding::Location { location, .. }) => Some(location),
            _ => None,
        })
        .collect();
    assert_eq!(locations, [1, 33]);
}

#[test]
fn the_high_dword_of_a_product_is_a_valid_module() {
    let product = vop3(op3("v_mul_hi_u32"), 1, [V, V, 0], 0, 0);
    let position = exp(POS0, [1, 1, 1, 1]);
    lowered(
        ShaderKind::Vertex,
        vec![product[0], product[1], position[0], position[1], S_ENDPGM],
    );
}

#[test]
fn a_mesh_program_runs_as_a_vertex_program_that_calls_its_body_twice() {
    // Each thread places a vertex at (1, 1, 1, 1) and a primitive made of its own number.
    let lane = vop3(op3("v_mbcnt_lo_u32_b32"), 0, [I_MINUS_ONE, I_ZERO, 0], 0, 0);
    let (position, primitive) = (exp(POS0, [1, 1, 1, 1]), exp(20, [0, 0, 0, 0]));
    let words = vec![
        lane[0],
        lane[1],
        vop1(V_MOV, 1, F_ONE),
        sopp(op(opcodes::SOPP, "s_sendmsg"), 9),
        sopp(op(opcodes::SOPP, "s_barrier"), 0),
        position[0],
        position[1],
        primitive[0],
        primitive[1],
        S_ENDPGM,
    ];
    let module = lowered(ShaderKind::Mesh, words.clone()).module;
    let entry = &module.entry_points[0];
    assert_eq!(entry.stage, naga::ShaderStage::Vertex);
    for wanted in [naga::BuiltIn::VertexIndex, naga::BuiltIn::InstanceIndex] {
        assert!((entry.function.arguments.iter())
            .any(|a| a.binding == Some(naga::Binding::BuiltIn(wanted))));
    }
    let calls = (entry.function.body.iter())
        .filter(|s| matches!(s, naga::Statement::Call { .. }))
        .count();
    assert_eq!(calls, 2);
    // No other stage knows which lane of its wave it is.
    assert!(matches!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 0, .. })
    ));
}

#[test]
fn a_part_of_a_register_is_read_where_a_word_selects_it() {
    // v1 = the high half of v0 as a float: the word names v0, its second 16 bits, and the whole destination.
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        vop1(op(opcodes::VOP1, "v_cvt_f32_u32"), 1, 249),
        5 << 16 | 6 << 8,
        position[0],
        position[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Vertex, words);
    // A selector past the whole register names nothing.
    let words = vec![
        vop1(op(opcodes::VOP1, "v_cvt_f32_u32"), 1, 249),
        7 << 16 | 6 << 8,
        position[0],
        position[1],
        S_ENDPGM,
    ];
    assert!(matches!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 0, .. })
    ));
}

#[test]
fn the_corrections_the_published_encodings_made_hold() {
    let len = |words: &[u32]| decode_one(words, 0).unwrap().1;
    // `v_fmaak_f32` carries its constant in a second word.
    assert_eq!(len(&[vop2(0x2D, 0, V, 1), ONE]), 2);
    // The `sdwa` code means nothing in a scalar field and takes no word.
    assert_eq!(len(&[sop1(3, 0, 249)]), 1);
    assert_eq!(text(&[sopp(0x28, 0)]), ["s_ttracedata_imm"]);
    // A buffer opcode's eighth bit is bit 25 of the first word.
    let (inst, _) = decode_one(&[0xE000_0000 | 1 << 25, 0], 0).unwrap();
    assert_eq!(inst.op().map(|op| op.number), Some(0x80));
    // A global offset is signed: twelve set bits are -1.
    let (inst, _) = decode_one(&[0xDC00_8FFF, 0], 0).unwrap();
    assert!(matches!(inst, Inst::Flat { offset: -1, .. }));
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// One block whose used slot is `slot` and whose code range is six descriptor words, `words`, then filler to whole 64-byte lines.
fn stage(slot: usize, stage_word: u32, words: &[u32]) -> XdatStage {
    const EMPTY: u64 = 0xDEAD_BEEF_DEAD_BEEF;
    const CODE_OFFSET: usize = 0x300;
    let mut code = vec![0xD580_0000, 0x03FE_0000, 0, 0xD580_0000, 0x03FF_0000, 0];
    code.extend_from_slice(words);
    code.push(S_CODE_END);
    while !code.len().is_multiple_of(16) {
        code.push(S_CODE_END);
    }
    let start = 0x10 + CODE_OFFSET;
    let mut m = vec![0u8; start + 4 * code.len() + 16];
    let len = m.len();
    m[..4].copy_from_slice(b"FSSM");
    put32(&mut m, 4, 0x101);
    put32(&mut m, 8, (len - 16) as u32);
    put32(&mut m, 0x10, 6);
    for s in 0..12 {
        put64(&mut m, 0x18 + 8 * s, if s == slot { 0xB0 } else { EMPTY });
    }
    put32(&mut m, 0x78, 0x3_0002);
    put64(&mut m, 0xC0, CODE_OFFSET as u64);
    put64(&mut m, 0xC8, 4 * code.len() as u64);
    put32(&mut m, 0xD8, 0xC);
    put64(&mut m, 0xE8, (CODE_OFFSET + 4 * code.len()) as u64);
    put32(&mut m, 0x100, stage_word);
    for (i, w) in code.iter().enumerate() {
        put32(&mut m, start + 4 * i, *w);
    }
    XdatStage {
        slot,
        stage_word,
        package: m,
        code: start..start + 4 * code.len(),
    }
}

/// A DXBC container whose one part is `dxil`.
fn container(dxil: &[u8]) -> Vec<u8> {
    let size = 36 + 8 + dxil.len();
    let mut out = b"DXBC".to_vec();
    out.extend_from_slice(&[0; 16]);
    for word in [1, size as u32, 1, 36] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(b"DXIL");
    out.extend_from_slice(&(dxil.len() as u32).to_le_bytes());
    out.extend_from_slice(dxil);
    out
}

#[test]
fn a_container_unwraps_to_every_block_of_its_directory() {
    let first = [vop1(V_MOV, 0, V + 1), S_ENDPGM];
    let second = [vop1(V_MOV, 2, V + 3), vop1(V_MOV, 4, V + 5), S_ENDPGM];
    let xdat = XdatProgram {
        program_version: 0x0001_0066,
        digest: [7; 16],
        stages: vec![stage(2, 0x2_0002, &first), stage(4, 0x2_0004, &second)],
        stub_words: 0x1A5,
    };
    let bytes = container(&xdat.write().unwrap());
    let programs = unwrap(&bytes).unwrap();
    assert_eq!(programs.len(), 2);
    assert_eq!(
        (programs[0].kind, programs[0].slot, &programs[0].words[..]),
        (ShaderKind::Vertex, 2, &first[..])
    );
    assert_eq!((programs[1].slot, &programs[1].words[..]), (4, &second[..]));
}

#[test]
fn a_container_that_is_not_one_is_refused() {
    assert_eq!(unwrap(b"FSSM"), Err(ContainerError::NotDxbc));
    assert_eq!(
        unwrap(&container(&[])[..36]),
        Err(ContainerError::Truncated {
            what: "the size the header states",
            at: 36
        })
    );
    let xdat = XdatProgram {
        program_version: 0x66,
        digest: [0; 16],
        stages: vec![stage(5, 0x2_0105, &[S_ENDPGM])],
        stub_words: 0,
    };
    let mut bytes = container(&xdat.write().unwrap());
    // The second directory entry's tag, 0x0D, becomes one no rule allows.
    let tag = 36 + 8 + 24 + 12 + 12;
    assert_eq!(bytes[tag], 0x0D);
    bytes[tag] = 0x0E;
    assert!(matches!(unwrap(&bytes), Err(ContainerError::Xdat(_))));
}

/// A two-source instruction whose first source is a trailing word's: a part selection, or
/// another lane's value.
fn vop2_with(name: &str, dst: u32, code: u32, src1: u32, word: u32) -> [u32; 2] {
    [vop2(op(opcodes::VOP2, name), dst, code, src1), word]
}

/// `global_load_dword` of `dst` at the address in `addr` plus `offset`, with no scalar address.
fn global_load(dst: u32, addr: u32, offset: u32) -> [u32; 2] {
    let number = op(opcodes::GLOBAL, "global_load_dword");
    [
        0xDC00_8000 | number << 18 | offset,
        addr | 0x7D << 16 | dst << 24,
    ]
}

#[test]
fn a_word_of_a_descriptor_read_as_a_number_comes_from_the_dwords_of_its_table() {
    // s[8:15] = the image at 0x20 of the table at s2; s1 = the top two bits of its second word.
    let image = smem(S_LOAD_X8, 8, 2, 0x20);
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        image[0],
        image[1],
        sop2(op(opcodes::SOP2, "s_lshr_b32"), 1, 9, I_ZERO + 30),
        vop1(V_MOV, 1, 1),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert_eq!(out.resources.len(), 1);
    assert_eq!(out.resources[0].kind, ResourceKind::TableWords);
    assert_eq!(out.resources[0].at, Some(plain(2, 0)));

    // At an index a run computes the register holds the place of the word, and the read finds it there.
    let image = smem_at(S_LOAD_X8, 8, 2, 0, 30);
    let words = vec![
        sop2(S_ADD_U32, 30, 9, 9),
        image[0],
        image[1],
        sop2(op(opcodes::SOP2, "s_lshr_b32"), 1, 10, I_ZERO + 14),
        vop1(V_MOV, 1, 1),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!((out.resources.iter())
        .any(|r| r.kind == ResourceKind::TableWords && r.at == Some(plain(2, 0))));
}

#[test]
fn a_register_only_some_paths_write_is_no_input() {
    // v1 is written unless the branch is taken, and exported either way.
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 1),
        vop1(V_MOV, 1, F_ONE),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Vertex, words);
}

#[test]
fn two_loads_of_one_table_at_computed_places_join_as_one_descriptor() {
    let load = smem_at(S_LOAD_X4, 4, 2, 0, 30);
    let typed = mubuf(op(opcodes::MUBUF, "buffer_load_format_x"), 4, 0, 4, true);
    let position = exp(POS0, [4, 4, 4, 4]);
    let words = vec![
        sop2(S_ADD_U32, 30, 9, 9),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 3),
        load[0],
        load[1],
        sopp(S_BRANCH, 2),
        load[0],
        load[1],
        typed[0],
        typed[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!((out.resources.iter())
        .any(|r| r.kind == ResourceKind::Buffer { stride: None } && r.count.is_some()));
}

#[test]
fn a_mode_write_leaves_the_user_registers_as_they_were() {
    let words = [
        sopk(op(opcodes::SOPK, "s_setreg_imm32_b32"), 0, 0xF814),
        0,
        S_ENDPGM,
    ];
    let cfg = Cfg::build(&words).unwrap();
    let mut state = sym::entry();
    for (pc, inst) in &cfg.blocks[&0].insts {
        sym::step_at(&mut state, *pc, inst, false);
    }
    assert_eq!(state[0], sym::Sym::User(0));
    // Nor does a wait for the stores under way write the register its word has room for.
    let wait = sopk(op(opcodes::SOPK, "s_waitcnt_vscnt"), 0, 0);
    let position = exp(POS0, [0, 0, 0, 0]);
    let words = vec![
        wait,
        vop1(V_MOV, 0, F_ONE),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let (inst, _) = decode_one(&words, 0).unwrap();
    sym::step_at(&mut state, 0, &inst, false);
    assert_eq!(state[0], sym::Sym::User(0));
    lowered(ShaderKind::Vertex, words);
}

#[test]
fn the_code_reads_the_words_after_its_last_instruction() {
    // s[4:5] = the address of word 1; v0 = it; v1 = the word 24 bytes on, which is word 7.
    let load = global_load(1, 0, 24);
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0),
        vop1(V_MOV, 0, 4),
        load[0],
        load[1],
        position[0],
        position[1],
        S_ENDPGM,
        ONE,
    ];
    let module = lowered(ShaderKind::Vertex, words.clone()).module;
    let (_, data) = (module.global_variables.iter())
        .find(|(_, global)| global.name.as_deref() == Some("code_data"))
        .unwrap();
    assert!(data.init.is_some());
    assert!(matches!(
        module.types[data.ty].inner,
        naga::TypeInner::Array {
            size: naga::ArraySize::Constant(count),
            ..
        } if count.get() == 1
    ));
    // s6 = the same word by a scalar load through the pointer: two words further on now.
    let scalar = smem(op(opcodes::SMEM, "s_load_dword"), 6, 4, 32);
    let words = vec![
        words[0],
        words[1],
        scalar[0],
        scalar[1],
        vop1(V_MOV, 1, 6),
        vop1(V_MOV, 2, 6),
        position[0],
        position[1],
        S_ENDPGM,
        ONE,
    ];
    lowered(ShaderKind::Vertex, words);
    // A program that never takes its own address has no such words to read.
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        load[0],
        load[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    assert!(matches!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 1, .. })
    ));
}

#[test]
fn a_float_is_rounded_to_the_nearest_half() {
    for (value, half) in [
        (1.0f32, 0x3C00u16),
        (0.5, 0x3800),
        (-2.0, 0xC000),
        (4.0, 0x4400),
        (0.0, 0),
        // One part in two pi, the one inline constant no half holds exactly.
        (f32::from_bits(0x3E22_F983), 0x3118),
        (65536.0, 0x7C00),
        // The smallest denormal half, half of it (a tie, to the even zero), and the float after that.
        (f32::from_bits(0x3380_0000), 1),
        (f32::from_bits(0x3300_0000), 0),
        (f32::from_bits(0x3300_0001), 1),
        // The float before two rounds up into the next exponent.
        (f32::from_bits(0x3FFF_FFFF), 0x4000),
    ] {
        assert_eq!(to_naga::half_bits(value), half, "{value}");
    }
}

#[test]
fn the_16_bit_float_forms_are_a_valid_module() {
    let position = exp(POS0, [2, 3, 4, 5]);
    // The high half of v2 = v1 as a half, the low half kept: the word names v1 whole, and the
    // second 16 bits of the destination, the rest preserved.
    let narrowed = [
        vop1(op(opcodes::VOP1, "v_cvt_f16_f32"), 2, 249),
        1 | 5 << 8 | 2 << 11 | 6 << 16,
    ];
    // v3 = the low half of v2 and the half 1.0; v4 = v3 + v2 as halves; two masks = 0.5 < v4 as halves.
    let packed = vop3(op3("v_pack_b32_f16"), 3, [V + 2, F_ONE, 0], 0, 0);
    let compared = vop3(
        op(opcodes::VOPC, "v_cmp_lt_f16"),
        20,
        [F_HALF, V + 4, 0],
        0,
        0,
    );
    let words = vec![
        vop1(V_MOV, 1, F_ONE),
        vop1(V_MOV, 2, F_ONE),
        narrowed[0],
        narrowed[1],
        packed[0],
        packed[1],
        vop2(op(opcodes::VOP2, "v_add_f16"), 4, V + 3, 2),
        vopc(op(opcodes::VOPC, "v_cmp_lt_f16"), F_HALF, 4),
        compared[0],
        compared[1],
        vop1(op(opcodes::VOP1, "v_cvt_f32_f16"), 5, V + 4),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    // A sum of halves rounded at 32 bits and again at 16 is the sum rounded once.
    assert!(out.approximations.is_empty());
    assert!(!packs_halves(&out.module));
}

/// Whether a module leaves a 16-bit float's rounding or its denormals to the device.
fn packs_halves(module: &naga::Module) -> bool {
    let device = |e: &naga::Expression| {
        matches!(
            e,
            naga::Expression::Math {
                fun: naga::MathFunction::Pack2x16float | naga::MathFunction::Unpack2x16float,
                ..
            }
        )
    };
    let entries = module.entry_points.iter().map(|e| &e.function);
    (module.functions.iter().map(|(_, f)| f))
        .chain(entries)
        .any(|f| f.expressions.iter().any(|(_, e)| device(e)))
}

#[test]
fn a_fold_over_the_wave_and_a_lane_the_program_computes_are_of_this_lane() {
    // v2 = the least of v1 over a lane pattern of eight; s20 = v2 at the lane s30 names.
    let fold = vop2_with("v_min_u32", 2, 233, 1, 1 | (PAIR_SWAP << 8));
    let lane = vop3(op3("v_readlane_b32"), 20, [V + 2, 30, 0], 0, 0);
    let position = exp(POS0, [1, 2, 3, 3]);
    let words = vec![
        vop1(V_MOV, 1, F_ONE),
        fold[0],
        fold[1],
        lane[0],
        lane[1],
        vop1(V_MOV, 3, 20),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    for note in [
        "a fold over the wave's lanes read as this lane's",
        "a lane read as this lane",
    ] {
        assert!(out.approximations.contains_key(note), "{note}");
    }
    assert!(!out.subgroups);
    // A move from another lane is half a step of a fold: nothing else may read what it wrote.
    let moved = [vop1(V_MOV, 2, 233), 1 | (PAIR_SWAP << 8)];
    let words = vec![
        vop1(V_MOV, 1, F_ONE),
        moved[0],
        moved[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    assert_eq!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor {
            pc: 3,
            what: "another lane's value read outside a fold over the wave"
        })
    );
    // A pattern that opens no fold is refused, not guessed at.
    let stray = vop2_with("v_min_u32", 2, 233, 1, 1 | (1 << 8));
    let words = vec![
        vop1(V_MOV, 1, F_ONE),
        stray[0],
        stray[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    assert!(matches!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 1, .. })
    ));
}

/// The lane selectors of a fold's first step, and the controls of a row's mirror and of a
/// row moved right by one lane.
const PAIR_SWAP: u32 = 0o6745_2301;
const ROW_MIRROR: u32 = 0x140;
const ROW_SHR_1: u32 = 0x111;

/// How many statements of `block`, nested ones included, satisfy `wanted`.
fn count(block: &naga::Block, wanted: &dyn Fn(&naga::Statement) -> bool) -> usize {
    use naga::Statement as S;
    (block.iter())
        .map(|statement| {
            usize::from(wanted(statement))
                + match statement {
                    S::Block(inner) => count(inner, wanted),
                    S::If { accept, reject, .. } => count(accept, wanted) + count(reject, wanted),
                    S::Loop {
                        body, continuing, ..
                    } => count(body, wanted) + count(continuing, wanted),
                    _ => 0,
                }
        })
        .sum()
}

/// The steps a compiler writes for a fold of `v1` over a wave of 64 lanes with `name`, which
/// leave the result in `s21`: pairs, fours and eights, the row's mirror, the other row, the
/// other half of the wave, then the first lane.
fn wave_fold(name: &str) -> Vec<u32> {
    let join = op(opcodes::VOP2, name);
    let other_row = vop3(op3("v_permlanex16_b32"), 2, [V + 1, I_ZERO, I_ZERO], 0, 0);
    let half = vop3(op3("v_readlane_b32"), 20, [V + 1, I_ZERO + 32, 0], 0, 0);
    let first = vop3(op3("v_readlane_b32"), 21, [V + 1, I_ZERO, 0], 0, 0);
    let mut words = Vec::new();
    for selectors in [PAIR_SWAP, 0o4466_0022, 0o4444] {
        words.extend(vop2_with(name, 1, 233, 1, 1 | (selectors << 8)));
    }
    words.extend(vop2_with(
        name,
        1,
        250,
        1,
        1 | (ROW_MIRROR << 8) | (0xFF << 24),
    ));
    words.extend(other_row);
    words.push(vop2(join, 1, V + 2, 1));
    words.extend(half);
    words.push(vop2(join, 1, 20, 1));
    words.extend(first);
    words
}

#[test]
fn a_fold_over_the_wave_is_one_collective_of_the_subgroup() {
    use naga::Statement as S;
    let target = exp(0, [3, 3, 3, 3]);
    for (name, wanted) in [
        ("v_min_u32", naga::SubgroupOperation::Min),
        ("v_or_b32", naga::SubgroupOperation::Or),
        ("v_add_f32", naga::SubgroupOperation::Add),
    ] {
        let mut words = vec![vintrp(1, 1, 0, 0, 1)];
        words.extend(wave_fold(name));
        words.extend([vop1(V_MOV, 3, 21), target[0], target[1], S_ENDPGM]);
        // Unasked, a pixel is a wave of one lane, and its module one every device runs.
        let alone = lowered(ShaderKind::Pixel, words.clone());
        assert!(!alone.subgroups, "{name}");
        let out = waved(words);
        assert!(out.subgroups, "{name}");
        // Seven steps, one fold: a sum folded at every step would count each lane 64 times.
        let folds = count(&body(&out.module).body, &|s| {
            matches!(
                s,
                S::SubgroupCollectiveOperation {
                    op,
                    collective_op: naga::CollectiveOperation::Reduce,
                    ..
                } if *op == wanted
            )
        });
        assert_eq!(folds, 1, "{name}");
        let lone = "a fold over the wave's lanes read as this lane's";
        assert!(!out.approximations.contains_key(lone), "{name}");
        // A mesh program is one lane of no subgroup: the fold is its own value, and counted.
        let position = exp(POS0, [3, 3, 3, 3]);
        let mut words = vec![vop1(V_MOV, 1, F_ONE)];
        words.extend(wave_fold(name));
        words.extend([vop1(V_MOV, 3, 21), position[0], position[1], S_ENDPGM]);
        let out = lowered(ShaderKind::Vertex, words);
        assert!(!out.subgroups, "{name}");
        assert!(out
            .approximations
            .contains_key("a fold over the wave's lanes read as this lane's"));
    }
}

#[test]
fn a_running_sum_and_a_64_bit_sum_over_the_wave_are_collectives_too() {
    use naga::Statement as S;
    let target = exp(0, [3, 3, 3, 3]);
    // v3 = v1, then the sum of v3 over the lanes up to this one: four steps right along the
    // row, the row before, the half before; less v1, the lanes before this one.
    let add = op(opcodes::VOP2, "v_add_u32");
    let row_before = vop3(
        op3("v_permlanex16_b32"),
        4,
        [V + 3, I_MINUS_ONE, I_MINUS_ONE],
        0,
        0,
    );
    let half_before = vop3(op3("v_readlane_b32"), 20, [V + 3, I_ZERO + 31, 0], 0, 0);
    let mut words = vec![vintrp(1, 1, 0, 0, 1), vop1(V_MOV, 3, V + 1)];
    for step in 0..4 {
        let control = (ROW_SHR_1 & !0xF) | (1 << step);
        words.extend(vop2_with(
            "v_add_u32",
            3,
            250,
            3,
            3 | (control << 8) | (0xFF << 24),
        ));
    }
    words.extend(row_before);
    words.push(vop2(add, 3, V + 4, 3));
    words.extend(half_before);
    words.push(vop2(add, 3, 20, 3));
    words.push(vop2(op(opcodes::VOP2, "v_sub_u32"), 3, V + 3, 1));
    words.extend([target[0], target[1], S_ENDPGM]);
    let out = waved(words);
    let scans = count(&body(&out.module).body, &|s| {
        matches!(
            s,
            S::SubgroupCollectiveOperation {
                op: naga::SubgroupOperation::Add,
                collective_op: naga::CollectiveOperation::InclusiveScan,
                ..
            }
        )
    });
    assert_eq!(scans, 1);

    // A 64-bit sum of the pair v1, v2: each step moves the other lane's pair, then adds with carry.
    let add_low = |from: u32| vop3(op3("v_add_co_u32"), 1, [V + from, V + 1, 0], 0, 0);
    let add_high = |from: u32| vop2(op(opcodes::VOP2, "v_addc_co_u32"), 2, V + from, 2);
    let mut words = vec![vintrp(1, 1, 0, 0, 1), vintrp(1, 2, 0, 1, 1)];
    for selectors in [PAIR_SWAP, 0o4466_0022] {
        for (to, from) in [(3, 1), (4, 2)] {
            words.extend([vop1(V_MOV, to, 233), from | (selectors << 8)]);
        }
        words.extend(add_low(3));
        words.push(add_high(4));
    }
    words.extend([vop1(V_MOV, 3, V + 2), target[0], target[1], S_ENDPGM]);
    let out = waved(words);
    // The low dword folds as two 16-bit halves, the high one whole: three sums, whatever the steps.
    let sums = count(&body(&out.module).body, &|s| {
        matches!(s, S::SubgroupCollectiveOperation { .. })
    });
    assert_eq!(sums, 3);
}

#[test]
fn a_compute_wave_reads_its_lanes_through_the_subgroup() {
    use naga::Statement as S;
    let launch = Launch {
        user_registers: 2,
        group_axes: 0,
        group_size: false,
        threads: [8, 8, 1],
        shared_dwords: 0,
    };
    // v3 = v0 of the lane across each pair; v4 = v0 of the lane 16 away; s20 = the first lane
    // that runs; s21 = v0 there; v5 = how many lanes of the mask in s22 come before this one.
    let across = [vop1(V_MOV, 3, 250), (0xB1 << 8) | (0xFF << 24)];
    let far = swizzle(4, 0, 31 | (64 << 8));
    let lane = vop3(op3("v_readlane_b32"), 21, [V, 20, 0], 0, 0);
    let before = vop3(op3("v_mbcnt_lo_u32_b32"), 5, [22, I_ZERO, 0], 0, 0);
    let words = vec![
        across[0],
        across[1],
        far[0],
        far[1],
        sop1(op(opcodes::SOP1, "s_ff1_i32_b64"), 20, 126),
        lane[0],
        lane[1],
        sop1(op(opcodes::SOP1, "s_mov_b64"), 22, 126),
        before[0],
        before[1],
        S_ENDPGM,
    ];
    let compute = Program {
        launch: Some(launch),
        ..program(ShaderKind::Compute, words)
    };
    // Asked for its device's subgroup: a group of one wave keeps it in shared memory otherwise.
    let options = Options {
        wave: true,
        ..Options::default()
    };
    let out = translate_with(&compute, &options).unwrap();
    validate(&out.module);
    assert!(out.subgroups);
    let inner = &body(&out.module).body;
    let gathers = count(inner, &|s| {
        matches!(
            s,
            S::SubgroupGather {
                mode: naga::GatherMode::Shuffle(_),
                ..
            }
        )
    });
    assert_eq!(gathers, 3);
    assert!(holds(inner, &|s| matches!(s, S::SubgroupBallot { .. })));
    assert!(holds(inner, &|s| matches!(
        s,
        S::SubgroupCollectiveOperation {
            collective_op: naga::CollectiveOperation::ExclusiveScan,
            ..
        }
    )));
    // A group of two axes has no lane number of the stage's own: the entry point counts the
    // threads before this one.
    let entry = &out.module.entry_points[0];
    assert!(!(entry.function.arguments.iter())
        .any(|a| a.binding == Some(naga::Binding::BuiltIn(naga::BuiltIn::SubgroupInvocationId))));
    assert!(holds(&entry.function.body, &|s| matches!(
        s,
        S::SubgroupCollectiveOperation { .. }
    )));
}

#[test]
fn a_64_bit_shift_moves_a_register_pair() {
    let position = exp(POS0, [2, 3, 4, 5]);
    let left = vop3(op3("v_lshlrev_b64"), 2, [I_ZERO + 36, V, 0], 0, 0);
    let right = vop3(op3("v_ashrrev_i64"), 4, [V + 2, V, 0], 0, 0);
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        vop1(V_MOV, 1, F_ONE),
        left[0],
        left[1],
        right[0],
        right[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Vertex, words);
}

#[test]
fn a_compute_thread_knows_its_lane_of_its_wave() {
    let lane = vop3(op3("v_mbcnt_lo_u32_b32"), 3, [I_MINUS_ONE, I_ZERO, 0], 0, 0);
    let program = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [8, 8, 1],
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, vec![lane[0], lane[1], S_ENDPGM])
    };
    let out = translate(&program).unwrap();
    validate(&out.module);
}

#[test]
fn a_texture_is_stored_to_and_updated_in_one_step() {
    use naga::{Statement as S, StorageFormat as Format};
    // A texel of four floats stored at the coordinates in v0 and v1, from v0 to v3.
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let store = mimg("image_store", 1, 0b1111, 0);
    let mut words = vec![image[0], image[1]];
    words.extend((0..4).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([store[0], store[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words.clone());
    assert_eq!(
        out.resources,
        [Resource {
            kind: ResourceKind::StorageTexture {
                dim: 1,
                format: Format::Rgba16Float
            },
            at: Some(plain(2, 0x40)),
            words: None,
            group: 0,
            binding: 0,
            count: None,
            written: true
        }]
    );
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        S::ImageStore { .. }
    )));
    // The binder states the texture's own format, which the stored value is read by.
    let mut options = Options::default();
    (options.storage_formats).insert(plain(2, 0x40), Format::Rgba8Uint);
    let out = translate_with(&program(ShaderKind::Pixel, words), &options).unwrap();
    validate(&out.module);
    assert_eq!(
        out.resources[0].kind,
        ResourceKind::StorageTexture {
            dim: 1,
            format: Format::Rgba8Uint
        }
    );

    // An unsigned dword, and a 64-bit pair of registers: the greatest of the texel and them.
    for (dmask, format) in [(1, Format::R32Uint), (3, Format::R64Uint)] {
        let update = mimg("image_atomic_umax", 1, dmask, 0);
        let mut words = vec![image[0], image[1]];
        words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
        words.extend([update[0], update[1], S_ENDPGM]);
        let out = lowered(ShaderKind::Pixel, words);
        assert_eq!(
            out.resources[0].kind,
            ResourceKind::StorageTexture { dim: 1, format }
        );
        assert!(holds(&body(&out.module).body, &|s| matches!(
            s,
            S::ImageAtomic {
                fun: naga::AtomicFunction::Max,
                ..
            }
        )));
    }
    // A 64-bit texel takes the least and the greatest only.
    let update = mimg("image_atomic_or", 1, 3, 0);
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([update[0], update[1], S_ENDPGM]);
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Form { pc: 4, .. })
    ));

    // A texel loaded and stored back: one binding, which both go through.
    let load = mimg("image_load", 1, 0b1111, 0);
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([load[0], load[1], store[0], store[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(out.resources.len(), 1);
    assert!(out.resources[0].written);
    let (_, target) = out.module.global_variables.iter().next().unwrap();
    assert!(matches!(
        out.module.types[target.ty].inner,
        naga::TypeInner::Image {
            class: naga::ImageClass::Storage { access, .. },
            ..
        } if access == naga::StorageAccess::LOAD | naga::StorageAccess::STORE
    ));
}

#[test]
fn a_buffer_is_updated_in_one_step_and_stored_to_in_its_format() {
    use naga::Statement as S;
    // The dword at index v0 of a table's buffer: v4 added, then swapped for v4 where it is v5,
    // then four components stored in the format the buffer's header states.
    let descriptor = smem(S_LOAD_X4, 4, 2, 0x20);
    let add = mubuf(op(opcodes::MUBUF, "buffer_atomic_add"), 4, 0, 4, true);
    let swap = mubuf(op(opcodes::MUBUF, "buffer_atomic_cmpswap"), 4, 0, 4, true);
    let typed = mubuf(
        op(opcodes::MUBUF, "buffer_store_format_xyzw"),
        4,
        0,
        4,
        true,
    );
    let load = mubuf(op(opcodes::MUBUF, "buffer_load_dword"), 6, 0, 4, true);
    let mut words = vec![descriptor[0], descriptor[1], vop1(V_MOV, 0, F_ONE)];
    words.extend((4..8).map(|v| vop1(V_MOV, v, F_ONE)));
    for inst in [add, swap, typed, load] {
        words.extend(inst);
    }
    words.push(S_ENDPGM);
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(out.resources.len(), 1);
    assert!(out.resources[0].written);
    let inner = &body(&out.module).body;
    assert_eq!(count(inner, &|s| matches!(s, S::Atomic { .. })), 2);
    // A store of four components is four, each kept to what an element of the buffer's stride holds.
    assert_eq!(count(inner, &|s| matches!(s, S::If { .. })), 4);
    // The buffer's dwords are atomic ones, which its plain reads and stores go through too.
    let (_, buffer) = out.module.global_variables.iter().next().unwrap();
    let naga::TypeInner::Array { base, .. } = out.module.types[buffer.ty].inner else {
        panic!("the buffer is no array");
    };
    assert_eq!(
        out.module.types[base].inner,
        naga::TypeInner::Atomic(naga::Scalar::U32)
    );

    // Memory the code describes as pairs of bytes read as fractions: two components, one dword.
    let s_mov = op(opcodes::SOP1, "s_mov_b32");
    let stated = mubuf(
        op(opcodes::MUBUF, "buffer_store_format_xyzw"),
        4,
        0,
        0,
        true,
    );
    let mut words = vec![
        sop1(s_mov, 0, 9),
        sop2(op(opcodes::SOP2, "s_or_b32"), 1, 10, LITERAL),
        0x2_0000,
        sop1(s_mov, 2, I_MINUS_ONE),
        sop1(s_mov, 3, LITERAL),
        (14 << 12) | 0xFAC,
        vop1(V_MOV, 0, F_ONE),
    ];
    words.extend((4..8).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([stated[0], stated[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words);
    let stores = count(&body(&out.module).body, &|s| matches!(s, S::Store { .. }));
    // Twelve registers and the lane mask's two halves are written before; the store adds two.
    assert!(out.resources[0].written);
    assert!(stores >= 2);

    // A signed greatest is no one update of an unsigned dword: it is tried until it holds.
    let signed = mubuf(op(opcodes::MUBUF, "buffer_atomic_smax"), 4, 0, 4, true);
    let words = vec![
        descriptor[0],
        descriptor[1],
        vop1(V_MOV, 0, F_ONE),
        vop1(V_MOV, 4, F_ONE),
        signed[0],
        signed[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        naga::Statement::Loop { .. }
    )));
}

#[test]
fn shared_memory_is_updated_in_one_step() {
    let launch = Launch {
        user_registers: 2,
        group_axes: 0,
        group_size: false,
        threads: [8, 8, 1],
        shared_dwords: 128,
    };
    // v1 = what the dword at v0 plus four held before v1 was added to it; then a plain read.
    let (add, read) = (ds("ds_add_rtn_u32", 4), ds("ds_read_b32", 4));
    let compute = Program {
        launch: Some(launch),
        ..program(
            ShaderKind::Compute,
            vec![add[0], add[1], read[0], read[1], S_ENDPGM],
        )
    };
    let out = translate(&compute).unwrap();
    validate(&out.module);
    let (_, shared) = (out.module.global_variables.iter())
        .find(|(_, global)| global.space == naga::AddressSpace::WorkGroup)
        .unwrap();
    let naga::TypeInner::Array { base, .. } = out.module.types[shared.ty].inner else {
        panic!("the shared memory is no array");
    };
    assert_eq!(
        out.module.types[base].inner,
        naga::TypeInner::Atomic(naga::Scalar::U32)
    );
}

#[test]
fn a_register_only_some_paths_gave_a_table_word_is_still_a_number() {
    // s4 is the first word of the table at s2 on one path and the user's on the other; s0 is twice it.
    let table = smem(S_LOAD_X4, 4, 2, 0);
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 2),
        table[0],
        table[1],
        sop2(S_ADD_U32, 0, 4, 4),
        vop1(V_MOV, 1, 0),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    // The load read its words as numbers from the table's own dwords, which the sum then finds
    // in its register whichever path ran.
    assert!((out.resources.iter())
        .any(|r| r.kind == ResourceKind::TableWords && r.at == Some(plain(2, 0))));
    assert!(out
        .resources
        .iter()
        .any(|r| r.kind == ResourceKind::UserData));
}

#[test]
fn two_entries_of_one_table_join_as_a_place_a_run_computes() {
    // The buffer at 0x40 of the table on one path, and the one at the offset in s30 on the other.
    let (fixed, indexed) = (smem(S_LOAD_X4, 4, 2, 0x40), smem_at(S_LOAD_X4, 4, 2, 0, 30));
    let typed = mubuf(op(opcodes::MUBUF, "buffer_load_format_x"), 4, 0, 4, true);
    let position = exp(POS0, [4, 4, 4, 4]);
    let words = vec![
        sop2(S_ADD_U32, 30, 9, 9),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 3),
        fixed[0],
        fixed[1],
        sopp(S_BRANCH, 2),
        indexed[0],
        indexed[1],
        typed[0],
        typed[1],
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words.clone());
    assert!((out.resources.iter())
        .any(|r| r.kind == ResourceKind::Buffer { stride: None } && r.count.is_some()));
    // Each path's registers hold a word of an entry: the same word, 32 bytes an entry.
    let cfg = Cfg::build(&words).unwrap();
    let states = sym::solve(&cfg);
    let joined = &states[&8];
    for k in 0..4u32 {
        let sym::Sym::Word(at) = &joined[4 + k as usize] else {
            panic!("s{} holds no table word after the join", 4 + k);
        };
        assert_eq!((at.root, at.offset), (2, 4 * k));
        assert!(sym::computed(at));
    }
    // A word of another table, or another word of an entry, joins with nothing.
    let other = smem(S_LOAD_X4, 4, 2, 0x44);
    let mut words = words;
    (words[6], words[7]) = (other[0], other[1]);
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(sym::solve(&cfg)[&8][4], sym::Sym::Mixed);
}

#[test]
fn a_sampler_moved_through_vector_registers_is_still_its_table_entry() {
    // The sampler's four words go to v20..v23, come back to s24..s27 as the first lane's, and
    // name the sampler an image at s8 is read with.
    let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
    let first = op(opcodes::VOP1, "v_readfirstlane_b32");
    let read = [
        0xF000_0000 | (IMAGE_SAMPLE << 18) | (0xF << 8) | (1 << 3),
        ((8 / 4) << 16) | ((24 / 4) << 21),
    ];
    let target = exp(0, [0, 1, 2, 3]);
    let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
    words.extend((0..4).map(|k| vop1(V_MOV, 20 + k, 16 + k)));
    words.extend((0..4).map(|k| vop1(first, 24 + k, V + 20 + k)));
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words.clone());
    assert!((out.resources.iter()).any(|r| {
        r.kind == ResourceKind::Sampler { comparison: false } && r.at == Some(plain(2, 0x20))
    }));
    // Anything else written to one of the vector registers forgets what it carried.
    words.insert(8, vop2(V_ADD_F32, 21, V + 21, 21));
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor { pc: 15, .. })
    ));
}

#[test]
fn a_sampler_moved_where_lanes_run_survives_the_branch_taken_when_none_does() {
    // v20..v23 hold a sum, then the sampler's words unless no lane runs; the first lane's come
    // back to s24..s27 either way. A sum written to v19 after the moves touches none of them.
    let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
    let first = op(opcodes::VOP1, "v_readfirstlane_b32");
    let read = sample(8, 24);
    let target = exp(0, [0, 1, 2, 3]);
    let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.push(vop2(V_ADD_F32, 21, V, 0));
    words.push(sopc(S_CMP_EQ_U32, 0, I_ONE));
    words.push(sopp(op(opcodes::SOPP, "s_cbranch_execz"), 5));
    words.extend((0..4).map(|k| vop1(V_MOV, 20 + k, 16 + k)));
    words.push(vop2(V_ADD_F32, 19, V, 0));
    words.extend((0..4).map(|k| vop1(first, 24 + k, V + 20 + k)));
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words.clone());
    assert!((out.resources.iter()).any(|r| {
        r.kind == ResourceKind::Sampler { comparison: false } && r.at == Some(plain(2, 0x20))
    }));
    // Lanes run along both edges of a branch on the scalar condition: on one the register holds the sum.
    words[8] = sopp(S_CBRANCH_SCC1, 5);
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor { .. })
    ));
}

#[test]
fn a_descriptor_kept_in_the_lanes_of_a_vector_register_comes_back_as_itself() {
    // The sampler's four words go to lanes 2..5 of v30 and come back to s24..s27.
    let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
    let (read, target) = (sample(8, 24), exp(0, [0, 1, 2, 3]));
    let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
    for k in 0..4 {
        words.extend(vop3(
            op3("v_writelane_b32"),
            30,
            [16 + k, I_ZERO + 2 + k, 0],
            0,
            0,
        ));
    }
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    for k in 0..4 {
        words.extend(vop3(
            op3("v_readlane_b32"),
            24 + k,
            [V + 30, I_ZERO + 2 + k, 0],
            0,
            0,
        ));
    }
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words.clone());
    assert!((out.resources.iter()).any(|r| {
        r.kind == ResourceKind::Sampler { comparison: false } && r.at == Some(plain(2, 0x20))
    }));
    // Anything else written to the register forgets its lanes.
    words.insert(12, vop2(V_ADD_F32, 30, V, 0));
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor { .. })
    ));
}

#[test]
fn an_implicit_level_outside_a_pixel_program_is_the_first_level() {
    let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
    let (read, position) = (sample(8, 16), exp(POS0, [0, 1, 2, 3]));
    let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([read[0], read[1], position[0], position[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Vertex, words.clone());
    let note = "an implicit level outside a pixel program read as the first level";
    assert!(out.approximations.contains_key(note));
    let level_of = |module: &naga::Module| {
        (body(module).expressions.iter()).find_map(|(_, e)| match e {
            naga::Expression::ImageSample { level, .. } => Some(*level),
            _ => None,
        })
    };
    assert!(matches!(
        level_of(&out.module),
        Some(naga::SampleLevel::Exact(_))
    ));
    // A pixel has its neighbours, and the level is its own.
    let target = exp(0, [0, 1, 2, 3]);
    let at = words.len() - 3;
    (words[at], words[at + 1]) = (target[0], target[1]);
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(level_of(&out.module), Some(naga::SampleLevel::Auto));
    assert!(!out.approximations.contains_key(note));
}

/// A packed instruction over the halves of its sources, each result reading its own half of each.
fn vop3p(name: &str, dst: u32, srcs: [u32; 3]) -> [u32; 2] {
    let number = u32::from(opcodes::VOP3P.iter().find(|row| row.1 == name).unwrap().0);
    [
        0xCC00_0000 | (number << 16) | (1 << 14) | dst,
        srcs[0] | (srcs[1] << 9) | (srcs[2] << 18) | (3 << 27),
    ]
}

#[test]
fn the_tail_of_scalar_and_packed_forms_is_a_valid_module() {
    // s[4:5] = s[9:10] moved left by two; s6 = the first set bit of s9 from the top; v2 = the low
    // half of v1 moved right by three; v3 = the halves of v1 and v2 added pair by pair.
    let wide = sop2(op(opcodes::SOP2, "s_lshl_b64"), 4, 9, I_ZERO + 2);
    let narrow = vop3(op3("v_lshrrev_b16"), 2, [I_ZERO + 3, V + 1, 0], 0, 0);
    let packed = vop3p("v_pk_add_f16", 3, [V + 1, V + 2, 0]);
    let fused = vop3p("v_pk_fma_f16", 4, [V + 1, V + 2, V + 3]);
    let position = exp(POS0, [1, 2, 3, 4]);
    let words = vec![
        wide,
        sop1(op(opcodes::SOP1, "s_flbit_i32_b32"), 6, 9),
        vop1(V_MOV, 1, 5),
        narrow[0],
        narrow[1],
        packed[0],
        packed[1],
        fused[0],
        fused[1],
        vop1(V_MOV, 5, 6),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    lowered(ShaderKind::Vertex, words);
}

#[test]
fn an_array_kept_in_registers_is_read_and_written_at_the_index_in_m0() {
    // m0 = s9; the register m0 after v10 = v1; v2 = the register m0 after v10.
    let position = exp(POS0, [2, 2, 2, 2]);
    let words = vec![
        sop1(op(opcodes::SOP1, "s_mov_b32"), 124, 9),
        vop1(V_MOV, 1, F_ONE),
        vop1(op(opcodes::VOP1, "v_movreld_b32"), 10, V + 1),
        vop1(op(opcodes::VOP1, "v_movrels_b32"), 2, V + 10),
        position[0],
        position[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Vertex, words);
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        naga::Statement::Switch { .. }
    )));
    // The read is a choice among 64 registers, written a few choices a line: a writer
    // recurses a frame for every operator one line nests.
    assert!(written_depth(&out.module) <= 16);
}

/// The deepest nest of parentheses on a line of `module` written as WGSL.
fn written_depth(module: &naga::Module) -> usize {
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .unwrap();
    let text =
        naga::back::wgsl::write_string(module, &info, naga::back::wgsl::WriterFlags::empty())
            .unwrap();
    let mut deepest = 0usize;
    for line in text.lines() {
        let mut open = 0usize;
        for c in line.chars() {
            match c {
                '(' => open += 1,
                ')' => open = open.saturating_sub(1),
                _ => {}
            }
            deepest = deepest.max(open);
        }
    }
    deepest
}

#[test]
fn a_table_the_code_holds_is_read_at_an_entry_a_run_computes() {
    // s[4:5] = the address of word 1, moved on by s9; s6 = the word 16 bytes past it.
    let scalar = smem(op(opcodes::SMEM, "s_load_dword"), 6, 4, 16);
    let position = exp(POS0, [1, 1, 1, 1]);
    let words = vec![
        sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0),
        sop2(S_ADD_U32, 4, 4, 9),
        scalar[0],
        scalar[1],
        vop1(V_MOV, 1, 6),
        position[0],
        position[1],
        S_ENDPGM,
        ONE,
        ONE,
    ];
    let module = lowered(ShaderKind::Vertex, words.clone()).module;
    assert!((module.global_variables.iter())
        .any(|(_, global)| global.name.as_deref() == Some("code_data")));
    // A number added to anything else is no address of the code's.
    let mut words = words;
    words[0] = sop1(op(opcodes::SOP1, "s_mov_b32"), 4, 9);
    assert!(matches!(
        refusal(ShaderKind::Vertex, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor { pc: 2, .. })
    ));
}

#[test]
fn a_mesh_program_that_shares_memory_is_refused_by_name() {
    let write = ds("ds_write_b32", 4);
    let refused = translate(&program(
        ShaderKind::Mesh,
        vec![write[0], write[1], S_ENDPGM],
    ));
    let RdnaError::NotTranslated(refused) = refused.unwrap_err() else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(
        refused.name(),
        format!("ds_write_b32: {}", to_naga::MESH_SHARED)
    );
}

#[test]
fn a_far_branch_is_followed_to_the_word_it_lands_on() {
    // s[4:5] = the address of word 1; 20 bytes on is word 6; the word between is no instruction.
    let take = sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0);
    let carry = sop2(op(opcodes::SOP2, "s_addc_u32"), 5, 5, I_ZERO);
    let jump = sop1(op(opcodes::SOP1, "s_setpc_b64"), 0, 4);
    let words = [
        take,
        sop2(S_ADD_U32, 4, 4, LITERAL),
        20,
        carry,
        jump,
        0xC000_0000,
        S_ENDPGM,
    ];
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(cfg.blocks.keys().copied().collect::<Vec<_>>(), [0, 6]);
    assert_eq!(cfg.blocks[&0].terminator, Terminator::Jump(6));
    translate(&program(ShaderKind::Pixel, words.to_vec())).unwrap();
    // A jump through a register filled any other way leads nowhere this front end knows.
    let cfg = Cfg::build(&[sop1(op(opcodes::SOP1, "s_mov_b64"), 4, 6), jump]).unwrap();
    assert_eq!(cfg.blocks[&0].terminator, Terminator::Indirect);
    // And one past the code's last word is no branch.
    let mut outside = words;
    outside[2] = 400;
    let cfg = Cfg::build(&outside).unwrap();
    assert_eq!(cfg.blocks[&0].terminator, Terminator::Indirect);
}

#[test]
fn a_geometry_pair_is_refused_as_what_it_is() {
    // The vertex half ends on its return, a jump through the register the pipeline left it.
    let back = sop1(op(opcodes::SOP1, "s_setpc_b64"), 0, 6);
    let RdnaError::NotTranslated(half) = refusal(ShaderKind::Vertex, vec![back]) else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(half.name(), "the vertex half of a geometry pair");
    let RdnaError::NotTranslated(whole) = refusal(ShaderKind::Geometry, vec![S_ENDPGM]) else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(whole.name(), "a geometry program");
    assert!(whole.to_string().contains("a stage wgpu does not have"));
    // A compute program's jump through a register is still only that.
    let compute = Program {
        launch: Some(Launch {
            user_registers: 2,
            group_axes: 0,
            group_size: false,
            threads: [8, 8, 1],
            shared_dwords: 0,
        }),
        ..program(ShaderKind::Compute, vec![back])
    };
    assert_eq!(
        translate(&compute).unwrap_err(),
        RdnaError::NotTranslated(NotTranslated::Indirect { pc: 0 })
    );
}

/// What `work` gives on a thread with the stack a process's first thread has where it is smallest.
fn on_a_small_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    (std::thread::Builder::new().stack_size(1 << 20))
        .spawn(work)
        .unwrap()
        .join()
        .unwrap()
}

/// `depth` compares, each of which leaves for the export or goes on to the next: `depth - 1` nested ifs.
fn nested_ifs(depth: u16) -> Vec<u32> {
    let mut words = vec![vop1(V_MOV, 0, F_ONE)];
    for i in 0..depth {
        words.push(sopc(S_CMP_EQ_U32, 0, I_ONE));
        words.push(sopp(S_CBRANCH_SCC1, 2 * (depth - i) - 2));
    }
    words.extend(exp(0, [0, 0, 0, 0]));
    words.push(S_ENDPGM);
    words
}

#[test]
fn a_program_longer_than_any_shipped_is_refused_before_it_is_lowered() {
    let mut words = vec![sopp(op(opcodes::SOPP, "s_nop"), 0); 1 << 18];
    words.push(S_ENDPGM);
    let RdnaError::NotTranslated(long) = refusal(ShaderKind::Pixel, words) else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(long.name(), "a program of more than 262,144 words");
}

/// How deep the ifs, loops and switches of `block` nest.
fn deepest(block: &naga::Block) -> usize {
    use naga::Statement as S;
    (block.iter())
        .map(|statement| match statement {
            S::Block(inner) => deepest(inner),
            S::If { accept, reject, .. } => 1 + deepest(accept).max(deepest(reject)),
            S::Loop { body, .. } => 1 + deepest(body),
            S::Switch { cases, .. } => {
                1 + cases
                    .iter()
                    .map(|case| deepest(&case.body))
                    .max()
                    .unwrap_or(0)
            }
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn ifs_nested_to_any_depth_translate_on_a_small_stack() {
    for depth in [8, 17, 18, 60, 90, 120, 4000] {
        let (notes, functions, nest) = on_a_small_stack(move || {
            let out = translate(&program(ShaderKind::Pixel, nested_ifs(depth))).unwrap();
            validate(&out.module);
            let nest = (out.module.functions.iter()).map(|(_, function)| deepest(&function.body));
            (
                out.approximations,
                out.module.functions.len(),
                nest.max().unwrap_or(0),
            )
        });
        // A deep nest is spread over functions, none of which nests deeper than a reader of
        // the module is sure to follow; past what the walk follows the blocks run in a
        // dispatch loop, whose statements nest no deeper either.
        let dispatched = notes.keys().any(|note| note.starts_with("a dispatch loop"));
        assert_eq!(dispatched, depth > 90, "{depth}");
        assert_eq!(functions > 1, (18..=90).contains(&depth), "{depth}");
        assert!(nest <= 16, "{depth} nests {nest} deep");
    }
}

#[test]
fn a_branch_past_the_next_join_keeps_its_structure() {
    // Word 4 leaves for the export at word 9, past the join at word 8 that words 6 and 7 reach.
    let colour = exp(0, [2, 2, 2, 2]);
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_EQ_U32, 9, I_ONE),
        sopp(S_CBRANCH_SCC1, 4),
        sopc(S_CMP_EQ_U32, 10, I_ONE),
        sopp(S_CBRANCH_SCC1, 4),
        vop1(V_MOV, 0, F_HALF),
        sopp(S_BRANCH, 1),
        vop1(V_MOV, 1, F_ONE),
        vop1(V_MOV, 2, V),
        colour[0],
        colour[1],
        S_ENDPGM,
    ];
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(
        cfg.blocks.keys().copied().collect::<Vec<_>>(),
        [0, 3, 5, 7, 8, 9]
    );
    let out = lowered(ShaderKind::Pixel, words);
    assert!(out.approximations.is_empty());
    let function = body(&out.module);
    assert!(!holds(&function.body, &|s| matches!(
        s,
        naga::Statement::Switch { .. }
    )));
    // What says where control is on its way is a register like any other.
    assert!((function.local_variables.iter())
        .any(|(_, local)| local.name.as_deref() == Some("leaving")));
}

#[test]
fn a_branch_out_of_two_loops_and_one_to_the_outer_loop_keep_their_structure() {
    // Words 3 and 4 open two loops. Word 6 leaves both for the export, which word 2 also
    // reaches; word 8 runs the inner loop again and word 10 the outer one.
    let step = |register: u32| sop2(S_ADD_I32, register, register, I_ONE);
    let colour = exp(0, [0, 0, 0, 0]);
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_EQ_U32, 12, I_ONE),
        sopp(S_CBRANCH_SCC1, 8),
        step(9),
        step(10),
        sopc(S_CMP_EQ_U32, 9, I_ONE),
        sopp(S_CBRANCH_SCC1, 4),
        sopc(S_CMP_EQ_U32, 10, I_ONE),
        sopp(S_CBRANCH_SCC1, 0xFFFB),
        sopc(S_CMP_EQ_U32, 11, I_ONE),
        sopp(S_CBRANCH_SCC1, 0xFFF8),
        colour[0],
        colour[1],
        S_ENDPGM,
    ];
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(
        cfg.blocks.keys().copied().collect::<Vec<_>>(),
        [0, 3, 4, 7, 9, 11]
    );
    let out = lowered(ShaderKind::Pixel, words);
    assert!(out.approximations.is_empty());
    let function = body(&out.module);
    let loops = count(&function.body, &|s| {
        matches!(s, naga::Statement::Loop { .. })
    });
    assert_eq!(loops, 2);
    let named = |name: &str| {
        (function.local_variables.iter()).any(|(_, local)| local.name.as_deref() == Some(name))
    };
    assert!(named("loops_left") && named("again"));
}

#[test]
fn a_descriptor_made_where_lanes_run_holds_past_the_branch_taken_when_none_does() {
    // The sampler is loaded only where a lane runs, and read after the branch joins.
    let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
    let (read, target) = (sample(8, 16), exp(0, [0, 1, 2, 3]));
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.push(sopp(op(opcodes::SOPP, "s_cbranch_execz"), 2));
    words.extend(sampler);
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words.clone());
    assert!((out.resources.iter()).any(|r| {
        r.kind == ResourceKind::Sampler { comparison: false } && r.at == Some(plain(2, 0x20))
    }));
    // Lanes run along both edges of a branch on the scalar condition.
    words[4] = sopp(S_CBRANCH_SCC1, 2);
    assert!(matches!(
        refusal(ShaderKind::Pixel, words),
        RdnaError::NotTranslated(NotTranslated::Descriptor { .. })
    ));
}

#[test]
fn a_computed_place_and_a_user_pointer_come_back_from_the_lanes_they_were_kept_in() {
    // An image at a place a run computes waits in lanes 2..9 of v30, and the table's pointer
    // in lanes 0 and 1 of v31; the sampler is loaded through the pointer read back.
    let image = smem_at(S_LOAD_X8, 8, 2, 0x40, 30);
    let sampler = smem(S_LOAD_X4, 16, 50, 0x20);
    let (read, target) = (sample(40, 16), exp(0, [0, 1, 2, 3]));
    let lane = |name: &str, dst: u32, src: u32, lane: u32| {
        vop3(op3(name), dst, [src, I_ZERO + lane, 0], 0, 0)
    };
    let mut words = vec![sop2(S_ADD_U32, 30, 9, 9), image[0], image[1]];
    for k in 0..8 {
        words.extend(lane("v_writelane_b32", 30, 8 + k, 2 + k));
    }
    for k in 0..2 {
        words.extend(lane("v_writelane_b32", 31, 2 + k, k));
    }
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    for k in 0..8 {
        words.extend(lane("v_readlane_b32", 40 + k, V + 30, 2 + k));
    }
    for k in 0..2 {
        words.extend(lane("v_readlane_b32", 50 + k, V + 31, k));
    }
    words.extend(sampler);
    words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Pixel, words);
    assert!((out.resources.iter()).any(|r| {
        matches!(r.kind, ResourceKind::Texture { .. })
            && r.at == Some(table(2))
            && r.count.is_some()
    }));
    assert!((out.resources.iter()).any(|r| {
        r.kind == ResourceKind::Sampler { comparison: false } && r.at == Some(plain(2, 0x20))
    }));
}

#[test]
fn a_table_the_code_holds_is_reached_at_the_entry_a_lane_chooses() {
    // v2 = the address of word 1 moved on by v0; s6 = the first lane's; s8 = the word there.
    let scalar = smem(op(opcodes::SMEM, "s_load_dword"), 8, 6, 16);
    let position = exp(POS0, [1, 1, 1, 1]);
    let mut words = vec![
        sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0),
        vop2(op(opcodes::VOP2, "v_add_u32"), 2, 4, 0),
        vop1(op(opcodes::VOP1, "v_readfirstlane_b32"), 6, V + 2),
    ];
    words.extend(scalar);
    words.extend([
        vop1(V_MOV, 1, 8),
        position[0],
        position[1],
        S_ENDPGM,
        ONE,
        ONE,
    ]);
    let module = lowered(ShaderKind::Vertex, words).module;
    assert!((module.global_variables.iter())
        .any(|(_, global)| global.name.as_deref() == Some("code_data")));
}

#[test]
fn a_program_that_traces_rays_is_refused_as_what_it_is() {
    let test = mimg("image_bvh_intersect_ray", 1, 0xF, 0);
    let RdnaError::NotTranslated(why) =
        refusal(ShaderKind::Pixel, vec![test[0], test[1], S_ENDPGM])
    else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(why.name(), "a program that traces rays");
}

#[test]
fn a_pixel_package_lists_its_inputs_under_either_tag() {
    let mut package = vec![0u8; INPUT_TABLE + 3 * INPUT_RECORD];
    package[INPUT_TABLE..INPUT_TABLE + 8].copy_from_slice(&[7, 0, 0xF, 0, 0, 0, 0, 2]);
    package[INPUT_TABLE + 8..INPUT_TABLE + 16].copy_from_slice(&[6, 2, 3, 1, 0, 5, 2, 0x1A]);
    let slots: Vec<u8> = (pixel_inputs(&package).iter().map(|i| i.slot)).collect();
    assert_eq!(slots, [0, 2]);
}

#[test]
fn an_image_stored_to_whole_and_updated_is_one_binding_of_updates() {
    use naga::Statement as S;
    // A whole store, then the greatest of the texel and v0, then the texel read back.
    let image = smem(S_LOAD_X8, 8, 2, 0x40);
    let (store, update) = (
        mimg("image_store", 1, 0b1111, 0),
        mimg("image_atomic_umax", 1, 1, 0),
    );
    let load = mimg("image_load", 1, 1, 0);
    let mut words = vec![image[0], image[1]];
    words.extend((0..4).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([store[0], store[1], update[0], update[1], load[0], load[1]]);
    words.push(S_ENDPGM);
    let out = lowered(ShaderKind::Pixel, words);
    assert_eq!(out.resources.len(), 1);
    // The store is the texel cleared and set, each an update.
    assert_eq!(
        count(&body(&out.module).body, &|s| matches!(
            s,
            S::ImageAtomic { .. }
        )),
        3
    );
    // A 64-bit texel is read as its two dwords.
    let (update, load) = (
        mimg("image_atomic_umax", 1, 3, 0),
        mimg("image_load", 1, 3, 0),
    );
    let mut words = vec![image[0], image[1]];
    words.extend((0..2).map(|v| vop1(V_MOV, v, F_ONE)));
    words.extend([update[0], update[1], load[0], load[1], S_ENDPGM]);
    lowered(ShaderKind::Pixel, words);
}

#[test]
fn the_16_bit_integer_forms_and_a_word_of_the_code_are_a_valid_module() {
    // s[4:5] = the address of word 1; v1 = that address; v2 = the 16-bit word it leads to.
    let number = op(opcodes::GLOBAL, "global_load_ushort");
    let word = [0xDC00_8000 | number << 18 | 2, 1 | 0x7D << 16 | 2 << 24];
    let shift = vop3p("v_pk_lshlrev_b16", 3, [I_ZERO + 8, V + 2, 0]);
    let position = exp(POS0, [3, 3, 3, 3]);
    let mut words = vec![
        sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0),
        vop1(V_MOV, 1, 4),
    ];
    words.extend(word);
    words.extend(shift);
    words.extend(vop3p("v_pk_lshrrev_b16", 3, [I_ZERO + 8, V + 3, 0]));
    words.push(vopc(op(opcodes::VOPC, "v_cmp_eq_u16"), V + 3, 2));
    words.extend(vop3(op3("v_add_u16"), 3, [V + 3, V + 2, 0], 0, 0));
    words.extend([position[0], position[1], S_ENDPGM, ONE]);
    lowered(ShaderKind::Vertex, words);
}

#[test]
fn a_far_branch_back_lands_on_an_earlier_word() {
    // s[4:5] = the address of word 3; eight bytes back is word 1, which ends the program.
    let take = sop1(op(opcodes::SOP1, "s_getpc_b64"), 4, 0);
    let borrow = sop2(op(opcodes::SOP2, "s_subb_u32"), 5, 5, I_ZERO);
    let jump = sop1(op(opcodes::SOP1, "s_setpc_b64"), 0, 4);
    let words = [
        sopp(S_BRANCH, 1),
        S_ENDPGM,
        take,
        sop2(op(opcodes::SOP2, "s_sub_u32"), 4, 4, LITERAL),
        8,
        borrow,
        jump,
    ];
    let cfg = Cfg::build(&words).unwrap();
    assert_eq!(cfg.blocks[&2].terminator, Terminator::Jump(1));
}

#[test]
fn shared_memory_is_read_and_stored_a_16_bit_part_at_a_time() {
    let launch = Launch {
        user_registers: 2,
        group_axes: 0,
        group_size: false,
        threads: [8, 8, 1],
        shared_dwords: 128,
    };
    let mut words = Vec::new();
    for name in [
        "ds_write_b16",
        "ds_write_b16_d16_hi",
        "ds_read_u16",
        "ds_read_u16_d16_hi",
        "ds_max_i32",
    ] {
        words.extend(ds(name, 6));
    }
    // A four-component store names v4 to v7, which nothing wrote.
    let descriptor = smem(S_LOAD_X4, 8, 2, 0x20);
    let store = mubuf(
        op(opcodes::MUBUF, "buffer_store_format_xyzw"),
        4,
        0,
        8,
        true,
    );
    words.extend(descriptor);
    words.extend(store);
    words.push(S_ENDPGM);
    let compute = Program {
        launch: Some(launch),
        ..program(ShaderKind::Compute, words)
    };
    let out = translate(&compute).unwrap();
    validate(&out.module);
    // The signed greatest is tried until it holds.
    assert!(holds(&body(&out.module).body, &|s| matches!(
        s,
        naga::Statement::Loop { .. }
    )));
}

#[test]
fn a_pixel_program_that_computes_with_its_interpolation_pair_reads_its_weights() {
    let colour = exp(0, [0, 1, 1, 0]);
    let out = lowered(ShaderKind::Pixel, vec![colour[0], colour[1], S_ENDPGM]);
    let arguments = &out.module.entry_points[0].function.arguments;
    assert!((arguments.iter()).any(|a| {
        a.binding
            == Some(naga::Binding::BuiltIn(naga::BuiltIn::Barycentric {
                perspective: true,
            }))
    }));
}

#[test]
fn a_pixel_program_shaded_in_primitive_order_is_refused_as_what_it_is() {
    // Register 25, its first three bits, from s1.
    let turn = sopk(op(opcodes::SOPK, "s_setreg_b32"), 1, 0x1019);
    let RdnaError::NotTranslated(why) = refusal(ShaderKind::Pixel, vec![turn, S_ENDPGM]) else {
        panic!("not a refusal of the lowering");
    };
    assert_eq!(why.name(), "a pixel program shaded in primitive order");
}

/// The most statements any one block of `block` holds, nested ones included.
fn longest(block: &naga::Block) -> usize {
    use naga::Statement as S;
    let inner = block.iter().map(|statement| match statement {
        S::Block(inner) => longest(inner),
        S::If { accept, reject, .. } => longest(accept).max(longest(reject)),
        S::Loop { body, .. } => longest(body),
        S::Switch { cases, .. } => cases
            .iter()
            .map(|case| longest(&case.body))
            .max()
            .unwrap_or(0),
        _ => 0,
    });
    inner.fold(block.len(), usize::max)
}

#[test]
fn a_long_run_and_a_long_list_are_written_as_blocks_of_bounded_length() {
    // Two thousand sums in one block of the program, then two thousand branches over a move:
    // a writer that forgets a block's values one by one pays the square of what a block names.
    let position = exp(POS0, [1, 1, 1, 1]);
    let mut words = vec![vop1(V_MOV, 1, F_ONE), vop1(V_MOV, 2, F_ONE)];
    words.extend((0..2000).map(|_| vop2(V_ADD_F32, 1, V + 1, 2)));
    for _ in 0..2000 {
        words.push(sopc(S_CMP_EQ_U32, 9, I_ONE));
        words.push(sopp(S_CBRANCH_SCC1, 1));
        words.push(vop1(V_MOV, 1, F_ONE));
    }
    words.extend([position[0], position[1], S_ENDPGM]);
    let out = lowered(ShaderKind::Vertex, words);
    let most = (out.module.functions.iter()).map(|(_, function)| longest(&function.body));
    assert!(most.max().unwrap_or(0) <= 256);
}

/// `module` as the HLSL Direct3D 12 is given.
fn hlsl_of(module: &naga::Module) -> String {
    use naga::back::hlsl;
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .unwrap();
    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V6_0,
        fake_missing_bindings: true,
        ..Default::default()
    };
    let mut text = String::new();
    hlsl::Writer::new(&mut text, &options, &hlsl::PipelineOptions::default())
        .write(module, &info, None)
        .unwrap();
    text
}

#[test]
fn a_sample_at_the_first_level_is_written_as_direct3d_compiles_it() {
    // Direct3D has `SampleLevelZero` on no texture but as `SampleCmpLevelZero`.
    for (name, compared) in [("image_sample_lz", false), ("image_sample_c_lz", true)] {
        let (image, sampler) = (smem(S_LOAD_X8, 8, 2, 0x40), smem(S_LOAD_X4, 16, 2, 0x20));
        let (read, target) = (mimg(name, 1, 0b0001, 0), exp(0, [0, 0, 0, 0]));
        let mut words = vec![image[0], image[1], sampler[0], sampler[1]];
        words.extend((0..4).map(|v| vop1(V_MOV, v, F_ONE)));
        words.extend([read[0], read[1], target[0], target[1], S_ENDPGM]);
        let text = hlsl_of(&lowered(ShaderKind::Pixel, words).module);
        assert!(!text.contains(".SampleLevelZero("), "{name}");
        assert_eq!(text.contains("CmpLevelZero("), compared, "{name}");
    }
}

#[test]
fn a_dispatch_loop_assigns_the_next_block_by_two_stores() {
    // The graph of `a_graph_that_does_not_nest_runs_in_a_dispatch_loop`.
    let colour = exp(0, [0, 0, 0, 0]);
    let words = vec![
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_EQ_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 1),
        vop1(V_MOV, 0, F_ONE),
        sopc(S_CMP_LG_U32, I_ZERO, I_ZERO),
        sopp(S_CBRANCH_SCC1, 0xFFFD),
        colour[0],
        colour[1],
        S_ENDPGM,
    ];
    let out = lowered(ShaderKind::Pixel, words);
    // The variable is a scalar local of the function, and no line gives it a choice of two.
    let function = body(&out.module);
    let (_, next) = (function.local_variables.iter())
        .find(|(_, local)| local.name.as_deref() == Some("next"))
        .unwrap();
    assert_eq!(
        out.module.types[next.ty].inner,
        naga::TypeInner::Scalar(naga::Scalar::U32)
    );
    let text = hlsl_of(&out.module);
    let assigned: Vec<&str> = (text.lines())
        .filter(|line| line.trim_start().starts_with("next = "))
        .collect();
    assert!(assigned.len() >= 3);
    assert!(
        assigned.iter().all(|line| !line.contains('?')),
        "{assigned:?}"
    );
}
