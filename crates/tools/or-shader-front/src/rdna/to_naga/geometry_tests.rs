//! A made-up geometry pair on hand-assembled words: a vertex half that leaves two fetched
//! components and its vertex's number in shared memory, and a geometry half that emits a
//! triangle of what it reads of the third corner.

use std::sync::Arc;

use naga::valid::{Capabilities, ValidationFlags, Validator};

use super::{Emitted, Pair};
use crate::dxil::ShaderKind;
use crate::rdna::{
    opcodes, translate, translate_with, Lowered, Options, Program, RdnaError, ResourceKind,
    VertexInput,
};

const S_ENDPGM: u32 = 0xBF81_0000;
const LITERAL: u32 = 255;
const EXEC: u32 = 126;

/// The operand of a small whole number.
fn int(n: u32) -> u32 {
    128 + n
}

fn op(table: &[(u16, &str)], name: &str) -> u32 {
    u32::from(table.iter().find(|row| row.1 == name).unwrap().0)
}

fn sopp(name: &str, imm: u32) -> u32 {
    0xBF80_0000 | op(opcodes::SOPP, name) << 16 | imm
}

fn sop1(name: &str, dst: u32, src: u32) -> u32 {
    0xBE80_0000 | dst << 16 | op(opcodes::SOP1, name) << 8 | src
}

fn sop2(name: &str, dst: u32, src0: u32, src1: u32) -> u32 {
    0x8000_0000 | op(opcodes::SOP2, name) << 23 | dst << 16 | src1 << 8 | src0
}

fn vop1(name: &str, dst: u32, src: u32) -> u32 {
    0x7E00_0000 | dst << 17 | op(opcodes::VOP1, name) << 9 | src
}

fn vop2(name: &str, dst: u32, src0: u32, src1: u32) -> u32 {
    op(opcodes::VOP2, name) << 25 | dst << 17 | src1 << 9 | src0
}

/// A shared-memory instruction: the address register, two data registers, the first register
/// read into, and the two offsets.
fn ds(name: &str, addr: u32, data: [u32; 2], dst: u32, offsets: [u32; 2]) -> [u32; 2] {
    [
        0xD800_0000 | op(opcodes::DS, name) << 18 | offsets[1] << 8 | offsets[0],
        addr | data[0] << 8 | data[1] << 16 | dst << 24,
    ]
}

/// The lanes of the wave the count in `s3` at `field` names, as every half opens.
fn lanes(count: u32, field: u32) -> Vec<u32> {
    vec![
        sop2("s_bfe_i32", count, 3, LITERAL),
        field,
        sop2("s_bfm_b64", EXEC, count, int(0)),
    ]
}

/// Leaves the fetched `v9` and `v10` and the vertex's number at its wave's place, and returns
/// through `back`.
fn vertex_half(back: u32) -> Program {
    let mut words = lanes(23, 0x0008_0000);
    words.extend([
        sop2("s_bfe_u32", 10, 3, LITERAL),
        0x0004_0018,
        sop2("s_lshl_b32", 10, 10, int(11)),
        sop1("s_swappc_b64", 12, 12),
        vop1("v_mov_b32", 20, 10),
    ]);
    words.extend(ds("ds_write2_b32", 20, [9, 10], 0, [0, 1]));
    words.extend(ds("ds_write_b32", 20, [5, 0], 0, [8, 0]));
    words.extend([sopp("s_barrier", 0), sop1("s_setpc_b64", 0, back), S_ENDPGM]);
    Program {
        kind: ShaderKind::Vertex,
        slot: 2,
        words,
        wave32: false,
        inputs: Vec::new(),
        vertex_inputs: vec![VertexInput {
            register: 0,
            mask: 0b0011,
            component_type: 3,
        }],
        position_read: 0,
        front_face: false,
        launch: None,
    }
}

/// Reads the third corner's three dwords and emits `vertices` vertices of two registers: the
/// first writes `first` dwords of its own.
fn geometry_half(vertices: usize, first: [u32; 2]) -> Program {
    let mut words = lanes(0, 0x0008_0008);
    words.push(vop2("v_lshlrev_b32", 2, int(2), 1));
    words.extend(ds("ds_read2_b32", 2, [0, 0], 4, [0, 1]));
    words.extend(ds("ds_read_b32", 2, [0, 0], 6, [8, 0]));
    // Where the emitted vertices start: the pipeline's word, in units of 256 bytes.
    words.extend([
        0xF400_0000 | op(opcodes::SMEM, "s_load_dword") << 18 | (8 / 2),
        0xB8 | 0x7D << 25,
        sop2("s_lshl_b32", 0, 0, int(8)),
        vop1("v_mov_b32", 7, 0),
    ]);
    let emits = [
        vec![ds("ds_write2_b32", 7, [4, 5], 0, first)],
        vec![ds("ds_write2_b32", 7, [5, 4], 0, [8, 9])],
        vec![
            ds("ds_write2_b32", 7, [6, 6], 0, [16, 17]),
            ds("ds_write2_b32", 7, [4, 6], 0, [20, 21]),
        ],
    ];
    for writes in &emits[..vertices] {
        words.extend(writes.iter().flatten());
        words.push(sopp("s_sendmsg", 0x22));
    }
    words.extend([sopp("s_sendmsg", 0x12), sopp("s_sendmsg", 0x03), S_ENDPGM]);
    Program {
        kind: ShaderKind::Geometry,
        slot: 3,
        vertex_inputs: Vec::new(),
        words,
        ..vertex_half(6)
    }
}

const EMITTED: [Emitted; 2] = [
    Emitted {
        register: 0,
        position: true,
    },
    Emitted {
        register: 1,
        position: false,
    },
];

fn with(other: Program, emitted: &[Emitted]) -> Options {
    Options {
        pair: Some(Arc::new(Pair {
            other,
            emitted: emitted.to_vec(),
        })),
        ..Options::default()
    }
}

fn paired(vertex: &Program, geometry: &Program) -> Result<Lowered, RdnaError> {
    translate_with(geometry, &with(vertex.clone(), &EMITTED))
}

fn refusal(result: Result<Lowered, RdnaError>) -> String {
    match result {
        Err(RdnaError::NotTranslated(why)) => why.name(),
        other => panic!(
            "not a refusal: {:?}",
            other.map(|lowered| lowered.resources)
        ),
    }
}

/// The module as WGSL, once every writer a device compiles a module with has written it.
fn written(module: &naga::Module) -> String {
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
    wgsl::write_string(module, &info, wgsl::WriterFlags::empty()).unwrap()
}

#[test]
fn either_half_given_the_other_is_the_pairs_one_vertex_program() {
    let (vertex, geometry) = (vertex_half(6), geometry_half(3, [0, 1]));
    let lowered = paired(&vertex, &geometry).unwrap();
    let text = written(&lowered.module);
    let from_vertex = translate_with(&vertex, &with(geometry, &EMITTED)).unwrap();
    assert_eq!(text, written(&from_vertex.module));

    // The index buffer, then the vertex buffer of one input: four dwords a vertex.
    let kinds: Vec<_> = lowered.resources.iter().map(|r| r.kind).collect();
    let buffer = |stride| ResourceKind::Buffer {
        stride: Some(stride),
    };
    assert_eq!(kinds[..2], [buffer(4), buffer(16)]);
    assert!(lowered.resources[..2].iter().all(|r| r.at.is_none()));

    // The vertex half at each corner, then the geometry half.
    assert_eq!(text.matches("main_body();").count(), 4, "{text}");
    for run in 0..4 {
        assert!(text.contains(&format!("pair_run = {run}u;")), "{text}");
    }
    assert!(text.contains("pull_indices["), "{text}");
    assert!(text.contains("pull_vertices["), "{text}");
    // Three waves of corners, then three emitted vertices of two registers.
    assert!(text.contains("pair_shared: array<u32, 1560>"), "{text}");
    assert!(text.contains("@builtin(position) position: vec4<f32>"));
    // Register 1 leaves as parameter 1, which the pixel program after a pair reads it as.
    assert!(text.contains("@location(1) param1_: vec4<f32>"));
    assert!(!text.contains("@location(0)"), "{text}");
}

#[test]
fn a_half_alone_stays_refused() {
    assert_eq!(
        refusal(translate(&vertex_half(6))),
        "the vertex half of a geometry pair"
    );
    assert_eq!(
        refusal(translate(&geometry_half(3, [0, 1]))),
        "a geometry program"
    );
}

#[test]
fn a_pair_this_lowering_would_get_wrong_is_refused_by_what_it_does() {
    let (vertex, geometry) = (vertex_half(6), geometry_half(3, [0, 1]));
    // A jump through another register than the pipeline's return address.
    assert_eq!(
        refusal(paired(&vertex_half(4), &geometry)),
        "a geometry pair whose vertex half does not end by returning"
    );
    assert_eq!(
        refusal(paired(&vertex, &geometry_half(2, [0, 1]))),
        "a geometry program that does not emit one triangle"
    );
    // The first vertex written past its two registers.
    assert_eq!(
        refusal(paired(&vertex, &geometry_half(3, [0, 12]))),
        "a geometry program whose emitted vertices do not fill its output signature"
    );
    let no_position = [Emitted {
        position: false,
        ..EMITTED[0]
    }];
    assert_eq!(
        refusal(translate_with(&geometry, &with(vertex, &no_position))),
        "a geometry program whose emitted vertices do not fill its output signature"
    );
}
