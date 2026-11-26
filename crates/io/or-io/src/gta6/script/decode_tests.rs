use std::collections::BTreeMap;

use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName};
use crate::gta6::script::ir::{BinOp, Const, Kind, Scalar, StoreKind, TypeKey, UnOp, Var, VmType};
use crate::gta6::script::program::{ImportRow, MethodRow, SlotRow, TypeRow};
use crate::gta6::script::project::{Param, ProgramInfo, Signature, TypeFacts};

const PROGRAM: u32 = 0xAABB_CCDD;
const OTHER: u32 = 0x11;
const OWN_TYPE: &str = "type_0123456789ABCDEF";

/// An instruction written by hand: the word of the opcode named, then each operand byte under the key of its place.
fn insn(name: &str, operands: &[u8]) -> Vec<u8> {
    let opcode = (0..512u16).find(|n| opcode::name(*n) == Some(name));
    let opcode = opcode.expect("an opcode of that name");
    let mut bytes = opcode.wrapping_mul(0x843D).to_le_bytes().to_vec();
    let stored = operands.iter().enumerate().map(|(place, plain)| {
        let key = 0x85 ^ 0x11u8.wrapping_mul(place as u8);
        plain.wrapping_mul(0x65) ^ key
    });
    bytes.extend(stored);
    bytes
}

fn code(insns: &[Vec<u8>]) -> Vec<u8> {
    insns.concat()
}

/// A program of one own type, one imported program with one type, and one function at 0x40.
// The type's slots: none, the function at 0x40, a builtin method of two cells in and one out, a
// function of the imported program. Its method records: one for slot 4, two for slot 5.
fn program(code: Vec<u8>) -> Program {
    let function = |import: u32, at: u32| SlotRow::Function {
        import,
        at,
        unboxes: false,
    };
    let builtin = SlotRow::Builtin {
        ty: 0,
        method: 5,
        unboxes: false,
    };
    let own = TypeRow {
        name: 0x0123_4567_89AB_CDEF,
        byte_size: 24,
        flags: 0x22,
        vtable: vec![
            SlotRow::Empty,
            function(0, 0x40),
            builtin,
            function(1, 0x20),
        ],
        methods: 0..3,
        ..TypeRow::default()
    };
    let record = |slot: u32, returns: u32| MethodRow {
        slot: Some(slot),
        returns,
        ..MethodRow::default()
    };
    let at_40 = FunctionRow {
        at: 0x40,
        end: 0x50,
        params: 3,
        locals: 4,
        returns: 2,
    };
    Program {
        program: PROGRAM,
        code,
        functions: vec![at_40],
        types: vec![own],
        imports: vec![ImportRow {
            program: OTHER,
            types: 1..2,
        }],
        methods: vec![record(4, 3), record(5, 1), record(5, 4)],
        ..Program::default()
    }
}

/// A project that holds the imported program's type and one of its functions.
fn project() -> Project {
    let key = TypeKey(0x1122_3344_5566_7788);
    let mut project = Project::default();
    let info = ProgramInfo {
        program: OTHER,
        types: BTreeMap::from([(0, key)]),
        ..ProgramInfo::default()
    };
    project.programs.insert(OTHER, info);
    let facts = TypeFacts {
        byte_size: 12,
        flags: 0x14,
        home: OTHER,
        ..TypeFacts::default()
    };
    project.types.insert(key, facts);
    let params = [2, 1].map(|cells| Param {
        kind: Kind::Unknown,
        cells,
    });
    let signature = Signature {
        params: params.to_vec(),
        returns: 1,
        result: Kind::Unknown,
    };
    let function = FunctionKey::new(OTHER, 0x20);
    project.signatures.insert(function, signature);
    project
}

fn steps_in(project: &Project, name: &str, operands: &[u8]) -> Result<Vec<Op>, DisasmError> {
    let program = program(insn(name, operands));
    decode_in(&program, project, 0).map(|insn| insn.ops)
}

fn steps(name: &str, operands: &[u8]) -> Vec<Op> {
    steps_in(Project::empty(), name, operands).expect("the instruction decodes")
}

fn refused(name: &str, operands: &[u8]) -> bool {
    let refusal = steps_in(Project::empty(), name, operands);
    matches!(refusal, Err(DisasmError::Malformed { at: 0, .. }))
}

fn int(value: i32) -> Op {
    let e = Expr::Const(Const::i32(value));
    Op::Push { e, cells: 1 }
}

fn var(var: Var) -> Op {
    let e = Expr::Var(var);
    Op::Push { e, cells: 1 }
}

fn operation(name: &str, args: u32, returns: u32) -> Op {
    let callee = Callee::Intrinsic {
        name: name.to_owned(),
        ty: None,
        wrapper: Some(name.to_owned()),
    };
    Op::Call {
        callee,
        args,
        returns,
    }
}

fn own_type() -> VmType {
    VmType {
        name: OWN_TYPE.to_owned(),
        byte_size: 24,
        id: 0,
        flags: 0x22,
    }
}

fn both(ty: Scalar) -> Vec<Op> {
    vec![Op::Swap, Op::Cast(ty), Op::Swap, Op::Cast(ty)]
}

#[test]
fn an_instruction_is_its_word_and_its_operands_under_the_keys_of_their_places() {
    // Worked by hand: opcode 1 is the word 0x843D, and 9 times 0x65 under the key 0x85 is 0x08.
    assert_eq!(insn("push_param", &[9]), [0x3D, 0x84, 0x08]);
    assert_eq!(insn("push_param_0", &[]), [0x7A, 0x08]);
    let held = Raw {
        at: 0,
        len: 3,
        opcode: 1,
        fields: [9, 0, 0, 0],
    };
    assert_eq!(raw(&insn("push_param", &[9]), 0), Ok(held));

    // Several operands, each of its own width, lowest byte first.
    let call = raw(&insn("call", &[1, 0x01, 0x02, 0x03, 0]), 0).expect("a call");
    assert_eq!((call.len, call.fields), (7, [1, 0x03_0201, 0, 0]));
    // A signed operand is widened with its sign, an unsigned one of the same bytes is not.
    let minus = raw(&insn("push_i32", &[0xFE, 0xFF, 0xFF, 0xFF]), 0).expect("a literal");
    assert_eq!(minus.fields[0], -2i64 as u64);
    let token = raw(&insn("push_type", &[0xFE, 0xFF, 0xFF, 0xFF]), 0).expect("a token");
    assert_eq!(token.fields[0], 0xFFFF_FFFE);
    let back = raw(&insn("jump_short", &[0xFD]), 0).expect("a jump");
    assert_eq!(back.fields[0] as i64, -3);
    // Sixteen operand bytes, the most an instruction has.
    let wide = [1u64 << 40, 7].map(u64::to_le_bytes).concat();
    let select = raw(&insn("select_i64", &wide), 0).expect("a select");
    assert_eq!((select.len, select.fields), (18, [1 << 40, 7, 0, 0]));
}

#[test]
fn bytes_that_are_no_instruction_are_refused_with_their_offset() {
    let mut code = insn("nop", &[]);
    // The first number the interpreter does not dispatch on.
    let past = opcode::COUNT;
    code.extend(past.wrapping_mul(0x843D).to_le_bytes());
    assert_eq!(raw(&code, 0).map(|raw| raw.len), Ok(2));
    let refusal = DisasmError::Unknown {
        at: 2,
        opcode: past,
    };
    assert_eq!(raw(&code, 2), Err(refusal));
    let far = opcode::of_word(0xFFFF);
    let unknown = DisasmError::Unknown { at: 0, opcode: far };
    assert_eq!(raw(&[0xFF, 0xFF], 0), Err(unknown));

    let whole = insn("push_i32", &[1, 0, 0, 0]);
    assert!(raw(&whole, 0).is_ok());
    let cut = DisasmError::Truncated { at: 0 };
    assert_eq!(raw(&whole[..5], 0), Err(cut.clone()));
    assert_eq!(raw(&whole[..1], 0), Err(cut));
    assert_eq!(raw(&whole, 6), Err(DisasmError::Truncated { at: 6 }));
    assert_eq!(raw(&whole, 99), Err(DisasmError::Truncated { at: 99 }));
}

#[test]
fn a_function_is_walked_to_its_end_and_no_further() {
    let code = code(&[
        insn("enter", &[0x10]),
        insn("push_param_0", &[]),
        insn("return_1", &[]),
        insn("push_1", &[]),
    ]);
    let program = program(code);
    let mut row = FunctionRow {
        at: 0,
        end: 7,
        ..FunctionRow::default()
    };
    let insns = function(&program, &row).expect("the function decodes");
    let places: Vec<(u32, u32)> = insns.iter().map(|insn| (insn.at, insn.len)).collect();
    assert_eq!(places, [(0, 3), (3, 2), (5, 2)]);
    assert_eq!(insns[1].ops, [var(Var::Param(0))]);
    assert_eq!(insns[2].ops, [Op::Return { cells: 1 }]);
    // An end inside the last instruction is refused, not read past.
    row.end = 6;
    let refusal = function(&program, &row);
    assert!(matches!(refusal, Err(DisasmError::Malformed { at: 5, .. })));
}

#[test]
fn a_run_of_hostile_counts_is_held_to_a_bound() {
    let wide = insn("push_locals_w", &[0, 0, 0xFF, 0xFF]);
    let two = program(code(&[wide.clone(), wide.clone()]));
    let row = FunctionRow {
        at: 0,
        end: 12,
        ..FunctionRow::default()
    };
    let insns = function(&two, &row).expect("two pushes decode");
    assert_eq!(insns[1].ops.len(), 0xFFFF);
    assert_eq!(insns[1].ops.last(), Some(&var(Var::Local(0xFFFE))));
    let many = program(vec![wide; 17].concat());
    let row = FunctionRow {
        at: 0,
        end: 17 * 6,
        ..FunctionRow::default()
    };
    let refusal = function(&many, &row);
    assert!(matches!(refusal, Err(DisasmError::Malformed { .. })));

    // The cases of a switch count as steps do: 17 switches of 65,536 cases over one table.
    let cases = 1u32 << 16;
    let operands = [cases.to_le_bytes(), 0u32.to_le_bytes()].concat();
    let switches = |count: usize| {
        let mut program = program(vec![insn("switch", &operands); count].concat());
        program.strings = vec![0; 4 * cases as usize];
        let row = FunctionRow {
            at: 0,
            end: 10 * count as u32,
            ..FunctionRow::default()
        };
        function(&program, &row)
    };
    assert!(switches(2).is_ok());
    assert!(matches!(switches(17), Err(DisasmError::Malformed { .. })));
}

#[test]
fn frame_cells_are_pushed_lowest_first_and_stored_highest_first() {
    assert_eq!(steps("push_local_3", &[]), [var(Var::Local(3))]);
    assert_eq!(steps("push_param", &[9]), [var(Var::Param(9))]);
    assert_eq!(
        steps("push_params_short", &[2, 3]),
        [2, 3, 4].map(|n| var(Var::Param(n)))
    );
    assert_eq!(
        steps("store_locals", &[3, 2]),
        [Op::Set(Var::Local(4)), Op::Set(Var::Local(3))]
    );
    assert_eq!(
        steps("store_param2", &[5]),
        [Op::Set(Var::Param(6)), Op::Set(Var::Param(5))]
    );
    let copy = Op::Copy { depth: 0, cells: 1 };
    assert_eq!(steps("tee_local", &[7]), [copy, Op::Set(Var::Local(7))]);
    let address = Expr::AddrOf {
        var: Var::Local(0x0102),
        cell: 0,
    };
    let address = Op::Push {
        e: address,
        cells: 1,
    };
    assert_eq!(steps("local_address_w", &[0x02, 0x01]), [address]);
    let stepped = [
        var(Var::Local(2)),
        Op::Cast(Scalar::I32),
        int(1),
        Op::Binary {
            op: BinOp::Sub,
            at: Kind::I32,
        },
        Op::Set(Var::Local(2)),
    ];
    assert_eq!(steps("dec_local", &[2]), stepped);
    // The value left on the stack is the parameter before the step.
    let after = steps("postinc_param", &[1]);
    assert_eq!(after.len(), 6);
    assert_eq!(
        (&after[0], &after[5]),
        (&var(Var::Param(1)), &Op::Set(Var::Param(1)))
    );
}

#[test]
fn constants_are_pushed_as_a_listing_writes_them() {
    assert_eq!(steps("push_m1", &[]), [int(-1)]);
    assert_eq!(steps("push_u8", &[200]), [int(200)]);
    assert_eq!(steps("push_i32", &[0xFE, 0xFF, 0xFF, 0xFF]), [int(-2)]);
    assert_eq!(steps("push_u8x4", &[1, 2, 3, 4]), [1, 2, 3, 4].map(int));
    let float = |bits: u64| {
        let kind = Kind::F32;
        Expr::Const(Const { bits, kind })
    };
    let one = Expr::Const(Const::f32(1.0));
    assert_eq!(steps("push_f1", &[]), [Op::Push { e: one, cells: 1 }]);
    // The token of a type of the runtime is that type, which every program holds.
    let token = steps("push_type", &[0x0B, 0, 0, 0x80]);
    let named = |op: &Op| match op {
        Op::Call {
            callee: Callee::Intrinsic { name, ty, .. },
            args: 0,
            returns: 1,
        } => name == "typeof" && ty.as_ref().is_some_and(|ty| ty.name == "type_8000000B"),
        _ => false,
    };
    assert!(matches!(token.as_slice(), [op] if named(op)), "{token:?}");
    let long = Const {
        bits: 0x0807_0605_0403_0201,
        kind: Kind::U64,
    };
    let long = Op::Push {
        e: Expr::Const(long),
        cells: 1,
    };
    assert_eq!(steps("push_i64", &[1, 2, 3, 4, 5, 6, 7, 8]), [long]);
    let bit = Expr::Binary {
        op: BinOp::Shl,
        at: Kind::U64,
        l: Box::new(Expr::Const(Const {
            bits: 1,
            kind: Kind::U64,
        })),
        r: Box::new(Expr::cast(Scalar::I32, Expr::Const(Const::i32(5)))),
    };
    assert_eq!(steps("push_bit", &[5]), [Op::Push { e: bit, cells: 1 }]);
    let parts = [1.5f32, -2.0, 0.25].map(|part| float(u64::from(part.to_bits())));
    let vector = Op::Push {
        e: Expr::Vector3(Box::new(parts)),
        cells: 2,
    };
    let bytes = [1.5f32, -2.0, 0.25].map(f32::to_le_bytes).concat();
    assert_eq!(steps("push_vector3", &bytes), [vector]);
    let string = Expr::StringConstant {
        program: PROGRAM,
        offset: 16,
    };
    let string = Op::Push {
        e: string,
        cells: 1,
    };
    assert_eq!(steps("push_string", &[16, 0, 0, 0]), [string]);
}

#[test]
fn a_shuffle_moves_cells_and_a_depth_of_zero_names_none() {
    assert_eq!(steps("dup_n", &[3]), [Op::Copy { depth: 0, cells: 3 }]);
    assert_eq!(steps("pick", &[4]), [Op::Copy { depth: 3, cells: 1 }]);
    assert_eq!(steps("drop_n", &[2]), [Op::Drop { cells: 2 }]);
    assert_eq!(steps("sink", &[1]), [Op::Nop]);
    assert_eq!(steps("sink", &[2]), [Op::Swap]);
    assert_eq!(steps("sink", &[3]), [Op::Sink { depth: 3 }]);
    let load = Op::Load {
        ty: Scalar::U64,
        offset: 0,
    };
    assert_eq!(steps("deref_at", &[1]), std::slice::from_ref(&load));
    assert_eq!(
        steps("deref_at", &[3]),
        [Op::Raise { depth: 3 }, load, Op::Sink { depth: 3 }]
    );
    assert_eq!(
        steps("unbox_address_at", &[2]),
        [Op::Swap, int(8), Op::ByteOffset, Op::Swap]
    );
    for name in ["pick", "sink", "deref_at", "unbox_address_at"] {
        assert!(refused(name, &[0]), "{name}");
    }
}

#[test]
fn a_load_and_a_store_add_the_base_of_their_form_to_the_offset() {
    let load = |ty: Scalar, offset: u32| Op::Load { ty, offset };
    assert_eq!(steps("load_i32", &[4]), [load(Scalar::I32, 4)]);
    assert_eq!(steps("load_i32_o8", &[4]), [load(Scalar::I32, 12)]);
    assert_eq!(steps("load_u8_o24", &[16]), [load(Scalar::U8, 40)]);
    assert_eq!(steps("load_u64_o8_w", &[0, 1]), [load(Scalar::U64, 264)]);
    assert_eq!(steps("deref_i16", &[]), [load(Scalar::I16, 0)]);
    // A field of `this`: the instruction pops no address.
    assert_eq!(
        steps("load_u64_p0_o8", &[8]),
        [var(Var::Param(0)), load(Scalar::U64, 16)]
    );
    let store = |kind: StoreKind, offset: u32| Op::Store { kind, offset };
    assert_eq!(steps("store_ref_o8", &[8]), [store(StoreKind::Ref, 16)]);
    assert_eq!(steps("store_u8", &[3]), [store(StoreKind::U8, 3)]);
    assert_eq!(steps("put_u64", &[]), [store(StoreKind::U64, 0)]);
    let bytes = |offset: u32, bytes: u32, cells: u32| Op::StoreBytes {
        offset,
        bytes,
        cells,
    };
    assert_eq!(steps("store_bytes_o8", &[4, 12]), [bytes(12, 12, 2)]);
    assert_eq!(steps("store_u64s", &[8, 3]), [bytes(8, 24, 3)]);
    assert_eq!(steps("store_vector_o8", &[0]), [bytes(8, 16, 2)]);
    let floats = Op::StoreF32Slots {
        offset: 12,
        cells: 3,
    };
    assert_eq!(steps("store_f32s_o8", &[4, 3]), [floats]);
    let loaded = Op::LoadBytes {
        offset: 28,
        bytes: 20,
    };
    assert_eq!(steps("load_bytes_o24", &[4, 20]), [loaded]);
    let step = Op::Step {
        offset: 12,
        down: true,
    };
    assert_eq!(steps("dec_field_o8", &[4]), [step]);
    assert_eq!(
        steps("zero_bytes", &[0x10, 0]),
        [Op::ZeroBytes { bytes: 16 }]
    );
    // An address moved by nothing is the address.
    assert_eq!(steps("offset", &[0]), [Op::Nop]);
    assert_eq!(steps("offset", &[5]), [int(5), Op::ByteOffset]);
    assert_eq!(steps("offset_o8", &[0]), [int(8), Op::ByteOffset]);
}

#[test]
fn a_scalar_taken_out_of_a_value_keeps_its_width_and_its_sign() {
    let taken = |cells: u32, offset: u32, bytes: u32| Op::ExtractBytes {
        cells,
        offset,
        bytes,
    };
    assert_eq!(
        steps("extract_i32", &[2, 4]),
        [taken(2, 4, 4), Op::Cast(Scalar::I32)]
    );
    assert_eq!(
        steps("extract_i8", &[3, 17]),
        [taken(3, 17, 1), Op::Cast(Scalar::I8)]
    );
    // An unsigned field is widened with zeros, as the bytes come out.
    assert_eq!(steps("extract_u8", &[3, 17]), [taken(3, 17, 1)]);
    assert_eq!(steps("extract_u64", &[2, 8]), [taken(2, 8, 8)]);
    assert_eq!(steps("extract_bytes", &[4, 8, 12]), [taken(4, 8, 12)]);
    assert_eq!(steps("extract_vector", &[4, 16]), [taken(4, 16, 16)]);
}

#[test]
fn a_typed_operator_reads_each_operand_as_its_type() {
    let binary = |op: BinOp, at: Kind| Op::Binary { op, at };
    let mut sub = both(Scalar::I32);
    sub.push(binary(BinOp::Sub, Kind::I32));
    assert_eq!(steps("sub_i32", &[]), sub);
    // The bitwise operators work at the unsigned type of their width.
    let mut and = both(Scalar::U32);
    and.push(binary(BinOp::And, Kind::U32));
    assert_eq!(steps("and_i32", &[]), and);
    let shift = [
        Op::Cast(Scalar::U64),
        Op::Cast(Scalar::I32),
        Op::Swap,
        Op::Cast(Scalar::U64),
        Op::Swap,
        binary(BinOp::Shr, Kind::U64),
    ];
    assert_eq!(steps("shr_u64", &[]), shift);
    assert_eq!(
        steps("not_i32", &[]),
        [Op::Cast(Scalar::U32), Op::Unary(UnOp::BitNot)]
    );
    assert_eq!(
        steps("neg_f32", &[]),
        [Op::Cast(Scalar::F32), Op::Unary(UnOp::Neg)]
    );
    assert_eq!(
        steps("inc_i32", &[]),
        [Op::Cast(Scalar::I32), int(1), binary(BinOp::Add, Kind::I32)]
    );
    let copy = Op::Copy { depth: 0, cells: 1 };
    assert_eq!(
        steps("square_f32", &[]),
        [Op::Cast(Scalar::F32), copy, binary(BinOp::Mul, Kind::F32)]
    );
    // ⚠ A conversion a listing prints nothing for is one the interpreter does nothing for.
    assert_eq!(steps("i64_to_i32", &[]), [Op::Nop]);
    assert_eq!(steps("conv_u8", &[]), [Op::Cast(Scalar::U8)]);
    assert_eq!(steps("i32_to_f32", &[]), [operation("i32_to_f32", 1, 1)]);
    assert_eq!(steps("clamp_f32", &[]), [operation("clamp_f32", 3, 1)]);
    assert_eq!(
        steps("vector_dot3", &[]),
        [operation("packed_vector_dot3", 4, 1)]
    );
}

#[test]
fn a_comparison_is_the_one_a_listing_prints_and_an_ordered_float_test_is_negated() {
    let binary = |op: BinOp, at: Kind| Op::Binary { op, at };
    let mut equal = both(Scalar::I64);
    equal.push(binary(BinOp::Eq, Kind::I64));
    assert_eq!(steps("ceq_i64", &[]), equal);
    // Greater, or unordered: not less or equal.
    let mut greater = both(Scalar::F32);
    greater.extend([binary(BinOp::Le, Kind::F32), Op::Unary(UnOp::Not)]);
    assert_eq!(steps("cgt_f32", &[]), greater);
    // An integer has no unordered case, and its test stands as it is.
    let mut whole = both(Scalar::U32);
    whole.push(binary(BinOp::Gt, Kind::U32));
    assert_eq!(steps("cgt_u32", &[]), whole);
    assert_eq!(
        steps("is_nonzero", &[]),
        [Op::Cast(Scalar::U32), int(0), binary(BinOp::Ne, Kind::U32)]
    );
    assert_eq!(
        steps("is_zero64", &[]),
        [Op::Cast(Scalar::U64), int(0), binary(BinOp::Eq, Kind::U64)]
    );
    // The value for yes is on top in the instruction and under the value for no in the step.
    assert_eq!(steps("select", &[]), [Op::Swap, Op::Select]);
    assert_eq!(steps("select_u8", &[1, 2]), [int(2), int(1), Op::Select]);
}

#[test]
fn a_jump_goes_where_its_distance_from_the_operand_says() {
    let binary = |op: BinOp, at: Kind| Op::Binary { op, at };
    let nop = insn("nop", &[]);
    // At 4: the operand is at 6, and six bytes back from it is the start.
    let back = program(code(&[
        nop.clone(),
        nop.clone(),
        insn("brz_short", &[0xFA]),
    ]));
    let zero = [
        Op::Cast(Scalar::U32),
        int(0),
        binary(BinOp::Eq, Kind::U32),
        Op::Branch { to: 0 },
    ];
    assert_eq!(decode(&back, 4).map(|insn| insn.ops), Ok(zero.to_vec()));
    let ahead = program(code(&[insn("jump", &[4, 0, 0, 0]), nop.clone()]));
    let jump = decode(&ahead, 0).map(|insn| insn.ops);
    assert_eq!(jump, Ok(vec![Op::Jump { to: 6 }]));
    // Two bytes further is the end of the code, where no instruction is.
    let out = program(code(&[insn("jump", &[6, 0, 0, 0]), nop.clone()]));
    assert!(matches!(
        decode(&out, 0),
        Err(DisasmError::Malformed { at: 0, .. })
    ));
    assert!(refused("jump_short", &[0xF0]));

    let wide = program(code(&[insn("brnz64", &[0, 0, 0, 0]), nop.clone()]));
    let set = [
        Op::Cast(Scalar::U64),
        int(0),
        binary(BinOp::Ne, Kind::U64),
        Op::Branch { to: 2 },
    ];
    assert_eq!(decode(&wide, 0).map(|insn| insn.ops), Ok(set.to_vec()));
    let branch = |name: &str| {
        let program = program(code(&[insn(name, &[0, 0, 0, 0]), nop.clone()]));
        decode(&program, 0).expect("a branch").ops
    };
    let mut greater = both(Scalar::F64);
    greater.extend([
        binary(BinOp::Le, Kind::F64),
        Op::Unary(UnOp::Not),
        Op::Branch { to: 2 },
    ]);
    assert_eq!(branch("bgt_f64"), greater);
    let mut signed = both(Scalar::I32);
    signed.extend([binary(BinOp::Gt, Kind::I32), Op::Branch { to: 2 }]);
    assert_eq!(branch("bgt_i32"), signed);
    // ⚠ The one branch whose place in its family says the opposite of what the interpreter
    // runs: it sits among the tests for equal and jumps where its operands differ.
    let mut differs = both(Scalar::F64);
    differs.extend([binary(BinOp::Ne, Kind::F64), Op::Branch { to: 2 }]);
    assert_eq!(branch("bne_f64_alt"), differs);
    assert_eq!(branch("bne_f64"), differs);
    assert_eq!(steps("return", &[]), [Op::Return { cells: 0 }]);
    assert_eq!(steps("return_n", &[5]), [Op::Return { cells: 5 }]);
}

#[test]
fn a_switch_reads_its_cases_from_the_program_data_or_is_refused() {
    let switch = |count: u32, table: u32, tail: usize, data: &[u8]| {
        let operands = [count.to_le_bytes(), table.to_le_bytes()].concat();
        let mut code = insn("switch", &operands);
        code.extend(vec![insn("nop", &[]); tail].concat());
        let mut program = program(code);
        program.strings = data.to_vec();
        decode(&program, 0).map(|insn| insn.ops)
    };
    let mut data = vec![0xEE; 4];
    data.extend([10i32, 12].map(i32::to_le_bytes).concat());
    // The cases count from the operand, at 2; a selector past them goes on after the instruction.
    let cases = vec![(0, 12), (1, 14)];
    let steps = vec![Op::Cast(Scalar::I32), Op::Switch { cases, default: 10 }];
    assert_eq!(switch(2, 4, 3, &data), Ok(steps));
    let refused = |steps: Result<Vec<Op>, DisasmError>| {
        matches!(steps, Err(DisasmError::Malformed { at: 0, .. }))
    };
    // A table that ends past the data, a count no data holds, and a case out of the code.
    assert!(refused(switch(2, 8, 3, &data)));
    assert!(refused(switch(u32::MAX, 4, 3, &data)));
    assert!(refused(switch(2, 4, 2, &data)));
    assert!(refused(switch(1, 0, 3, &[])));
    // ⚠ Every case is held as a target of its own: more than a decoder holds are refused.
    let most = 1u32 << 16;
    let table = vec![0u8; 4 * (most as usize + 1)];
    assert!(switch(most, 0, 3, &table).is_ok());
    assert!(refused(switch(most + 1, 0, 3, &table)));
}

#[test]
fn a_global_is_of_the_program_its_import_names() {
    let global = |program: u32, offset: u32, statics: bool| Var::Global {
        program,
        offset,
        statics,
    };
    assert_eq!(
        steps("load_global_u64", &[0, 0x10]),
        [var(global(PROGRAM, 0x10, false)), Op::Cast(Scalar::U64)]
    );
    assert_eq!(
        steps("load_static_i32", &[1, 0, 0x20, 0, 0]),
        [var(global(OTHER, 0x2000, true)), Op::Cast(Scalar::I32)]
    );
    assert_eq!(
        steps("store_global_u8", &[1, 4]),
        [Op::Cast(Scalar::U8), Op::Set(global(OTHER, 4, false))]
    );
    // The program imports one other: a second is past its table.
    assert!(refused("load_global_u64", &[2, 0x10]));
    let address = |var: Var| Op::Push {
        e: Expr::AddrOf { var, cell: 0 },
        cells: 1,
    };
    assert_eq!(
        steps("static_address", &[0, 8, 0, 0, 0]),
        [address(global(PROGRAM, 8, true))]
    );
    let loaded = Op::LoadBytes {
        offset: 0,
        bytes: 24,
    };
    assert_eq!(
        steps("load_global_bytes", &[0, 8, 24]),
        [address(global(PROGRAM, 8, false)), loaded]
    );
    // The address goes under the cells the instruction pops, where a store looks for it.
    let stored = Op::StoreBytes {
        offset: 0,
        bytes: 16,
        cells: 2,
    };
    assert_eq!(
        steps("store_global_bytes", &[0, 8, 16]),
        [
            address(global(PROGRAM, 8, false)),
            Op::Sink { depth: 3 },
            stored
        ]
    );
    let small = Op::StoreBytes {
        offset: 0,
        bytes: 5,
        cells: 1,
    };
    assert_eq!(
        steps("store_global_bytes", &[0, 8, 5]),
        [address(global(PROGRAM, 8, false)), Op::Swap, small]
    );
    let guid = Op::Push {
        e: Expr::GuidEmpty,
        cells: 2,
    };
    assert_eq!(steps("builtin_global", &[3]), [guid]);
    let cell = Expr::Cell {
        base: Box::new(Expr::BuiltinGlobal(4)),
        index: 0,
    };
    assert_eq!(
        steps("builtin_global", &[4]),
        [Op::Push { e: cell, cells: 1 }]
    );
    // No program was seen to read builtin global 2: its cells are not known.
    assert!(refused("builtin_global", &[2]));
}

#[test]
fn a_call_states_its_cells_where_a_table_holds_them_and_says_so_where_none_does() {
    let local = |at: u32| Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    };
    let known = Op::Call {
        callee: local(0x40),
        args: 3,
        returns: 2,
    };
    assert_eq!(
        steps("call", &[0, 0x40, 0, 0, 0]),
        std::slice::from_ref(&known)
    );
    assert_eq!(steps("call_short", &[0, 0x40, 0]), [known]);
    // The program's function table holds no function at 0x41.
    let unknown = Op::Unsized {
        callee: local(0x41),
        args: None,
        returns: None,
    };
    assert_eq!(steps("call_short", &[0, 0x41, 0]), [unknown]);

    let import = Callee::Import {
        module: "program_00000011".to_owned(),
        name: "func_000020".to_owned(),
        program: OTHER,
        at: 0x20,
    };
    let alone = Op::Unsized {
        callee: import.clone(),
        args: None,
        returns: None,
    };
    assert_eq!(steps("call_short", &[1, 0x20, 0]), [alone]);
    let indexed = Op::Call {
        callee: import,
        args: 3,
        returns: 1,
    };
    let with = steps_in(&project(), "call_short", &[1, 0x20, 0]);
    assert_eq!(with, Ok(vec![indexed]));

    let native = Callee::Native {
        namespace: None,
        name: "rva_01020304".to_owned(),
        hash: 0,
        rva: 0x0102_0304,
    };
    let native = Op::Call {
        callee: native,
        args: 5,
        returns: 1,
    };
    assert_eq!(steps("call_native", &[4, 3, 2, 1, 5, 1]), [native]);
    assert_eq!(
        steps("call_delegate", &[3, 0, 1, 0]),
        [operation("invoke_delegate", 3, 1)]
    );
    let builtin = |method: u32| Callee::Builtin {
        path: format!("type_80000000.Method_{method}"),
        ty: 0,
        method,
    };
    let sized = Op::Call {
        callee: builtin(5),
        args: 2,
        returns: 1,
    };
    assert_eq!(steps("call_builtin", &[5, 0]), [sized]);
    let open = Op::Unsized {
        callee: builtin(1),
        args: None,
        returns: None,
    };
    assert_eq!(steps("call_builtin", &[1, 0]), [open]);
}

#[test]
fn a_virtual_call_returns_what_its_slot_resolves_to() {
    let method = |class: &str, slot: u32| Callee::Virtual {
        class: class.to_owned(),
        method: format!("Method_{slot}"),
        ty: class.to_owned(),
        slot,
    };
    let sized = |slot: u32, returns: u32| Op::Call {
        callee: method(OWN_TYPE, slot),
        args: 3,
        returns,
    };
    let open = |slot: u32| Op::Unsized {
        callee: method(OWN_TYPE, slot),
        args: Some(3),
        returns: None,
    };
    let call = |slot: u8| steps("call_virtual", &[3, 0, 0, slot]);
    // Slot 1 of the own type is the function at 0x40, which returns two cells.
    assert_eq!(call(1), [sized(1, 2)]);
    // Slot 0 is left to a base the program does not hold.
    assert_eq!(call(0), [open(0)]);
    // Slot 2 is a builtin method the table gives one cell.
    assert_eq!(call(2), [sized(2, 1)]);
    // Slot 3 is a function of the imported program: only the project knows its cells.
    assert_eq!(call(3), [open(3)]);
    let with = steps_in(&project(), "call_virtual", &[3, 0, 0, 3]);
    assert_eq!(with, Ok(vec![sized(3, 1)]));
    // The one method record for slot 4 states three cells; two records for slot 5 state nothing.
    assert_eq!(call(4), [sized(4, 3)]);
    assert_eq!(call(5), [open(5)]);
    let object = Op::Call {
        callee: method("type_80000000", 3),
        args: 1,
        returns: 1,
    };
    assert_eq!(steps("call_virtual_builtin", &[1, 0, 3]), [object]);
    let unlisted = Op::Unsized {
        callee: method("type_80000000", 9),
        args: Some(1),
        returns: None,
    };
    assert_eq!(steps("call_virtual_builtin", &[1, 0, 9]), [unlisted]);
    let reference = Op::VirtualFunctionRef {
        ty: own_type(),
        slot: 1,
    };
    assert_eq!(
        steps("push_virtual_function", &[0, 0, 0, 0, 1]),
        [reference]
    );
    let function = |program: u32, at: u32| Op::Push {
        e: Expr::FunctionRef { program, at },
        cells: 1,
    };
    assert_eq!(
        steps("push_function", &[0, 0x40, 0, 0, 0]),
        [function(PROGRAM, 0x40)]
    );
    // The flags ride in the top byte of the offset, as a listing prints them.
    assert_eq!(
        steps("push_function", &[1, 0x40, 0, 0, 0x40]),
        [function(OTHER, 0x4000_0040)]
    );
}

#[test]
fn a_type_of_another_program_is_named_by_the_project_or_refused() {
    let cast = |name: &str| Callee::Intrinsic {
        name: format!("try_cast<{name}>"),
        ty: None,
        wrapper: Some(format!("try_cast_{name}_")),
    };
    let own = Op::Call {
        callee: cast(OWN_TYPE),
        args: 1,
        returns: 1,
    };
    assert_eq!(steps("cast", &[0, 0, 0, 0, 0]), [own]);
    // Type 1 is the imported program's first: this program holds neither its name nor its size.
    assert!(refused("cast", &[1, 0, 0, 0, 0]));
    assert!(refused("box", &[1, 0, 0, 0]));
    let named = Op::Call {
        callee: cast("type_1122334455667788"),
        args: 1,
        returns: 1,
    };
    let with = steps_in(&project(), "cast", &[1, 0, 0, 0, 0]);
    assert_eq!(with, Ok(vec![named]));
    let unboxed = steps_in(&project(), "unbox", &[1, 0, 0, 0]);
    let twelve = Op::LoadBytes {
        offset: 8,
        bytes: 12,
    };
    assert_eq!(unboxed, Ok(vec![twelve]));
    // A type id past every import's range is no type of the program.
    assert!(steps_in(&project(), "cast", &[2, 0, 0, 0, 0]).is_err());
    let builtin = Op::Call {
        callee: cast("type_8000000B"),
        args: 1,
        returns: 1,
    };
    assert_eq!(steps("cast_builtin", &[11, 0]), [builtin]);
}

#[test]
fn a_value_is_moved_by_the_cells_of_its_type() {
    let boxed = |name: &str| Callee::Intrinsic {
        name: format!("box<{name}>"),
        ty: None,
        wrapper: Some(format!("box_{name}_")),
    };
    let own = Op::Call {
        callee: boxed(OWN_TYPE),
        args: 3,
        returns: 1,
    };
    assert_eq!(steps("box", &[0, 0, 0, 0]), [own]);
    let int32 = Op::Call {
        callee: boxed("type_8000000B"),
        args: 1,
        returns: 1,
    };
    assert_eq!(steps("box_builtin", &[11]), [int32]);
    // Builtin type 1 has no size in the table: its cells are not known.
    assert!(refused("box_builtin", &[1]));
    assert!(refused("unbox_builtin", &[1]));
    let loaded = |bytes: u32| Op::LoadBytes { offset: 8, bytes };
    assert_eq!(steps("unbox", &[0, 0, 0, 0]), [loaded(24)]);
    // A value of four bytes is read as a whole cell.
    assert_eq!(steps("unbox_builtin", &[11]), [loaded(8)]);
    let stored = |bytes: u32, cells: u32| Op::StoreBytes {
        offset: 0,
        bytes,
        cells,
    };
    assert_eq!(steps("store_value", &[0, 0, 0, 0]), [stored(24, 3)]);
    assert_eq!(steps("store_value_builtin", &[11]), [stored(4, 1)]);
    // A value of no bytes still takes a cell.
    assert_eq!(steps("store_value_builtin", &[0]), [stored(0, 1)]);
    assert_eq!(steps("unbox_address", &[]), [int(8), Op::ByteOffset]);
    let at = Callee::Intrinsic {
        name: "box_at".to_owned(),
        ty: Some(own_type()),
        wrapper: None,
    };
    let at = Op::Call {
        callee: at,
        args: 1,
        returns: 1,
    };
    assert_eq!(
        steps("box_at", &[0, 0, 0, 0, 3]),
        [Op::Raise { depth: 3 }, at, Op::Sink { depth: 3 }]
    );
}

#[test]
fn a_new_object_goes_under_the_arguments_its_constructor_was_given() {
    let object = Op::Push {
        e: Expr::NewObject(own_type()),
        cells: 1,
    };
    let copy = Op::Copy { depth: 0, cells: 1 };
    assert_eq!(
        steps("new_object", &[0, 0, 0, 0, 1]),
        [object.clone(), copy.clone()]
    );
    let under = Op::Sink { depth: 4 };
    assert_eq!(
        steps("new_object", &[0, 0, 0, 0, 3]),
        [object.clone(), copy.clone(), under.clone(), under]
    );
    assert!(refused("new_object", &[0, 0, 0, 0, 0]));
    assert_eq!(steps("new_object_plain", &[0, 0, 0, 0]), [object]);
    let weak = VmType {
        name: "weak_reference".to_owned(),
        byte_size: 0,
        id: 0,
        flags: 0,
    };
    let weak = Op::Push {
        e: Expr::NewObject(weak),
        cells: 1,
    };
    assert_eq!(steps("new_weak_reference", &[]), [weak]);

    // Builtin type 20, method 0: a constructor of five cells, the object among them.
    let array = VmType {
        name: "type_80000014".to_owned(),
        byte_size: 40,
        id: 0x8000_0014,
        flags: 0x000D_2000,
    };
    let constructor = |ty: u32, method: u32, args: u32| Op::Call {
        callee: Callee::Builtin {
            path: format!("type_{:08X}.Method_{method}", 0x8000_0000 | ty),
            ty,
            method,
        },
        args,
        returns: 0,
    };
    let made = [
        Op::Push {
            e: Expr::NewObject(array.clone()),
            cells: 1,
        },
        copy,
        Op::Sink { depth: 6 },
        Op::Sink { depth: 6 },
        constructor(20, 0, 5),
    ];
    assert_eq!(steps("new_builtin", &[0, 20]), made);
    // A method the table does not hold, and one that returns a cell and so constructs nothing.
    assert!(refused("new_builtin", &[9, 20]));
    assert!(refused("new_builtin", &[3, 0]));

    // Builtin type 21 is sixteen bytes; its method 1 takes the value's address and two cells.
    let value = [
        Op::Push {
            e: Expr::ZeroSlots(2),
            cells: 2,
        },
        Op::AddrOfTop { cells: 2 },
        Op::Raise { depth: 5 },
        Op::Raise { depth: 5 },
        constructor(21, 1, 3),
    ];
    assert_eq!(steps("new_builtin_value", &[1, 21]), value);
    let alloc = [
        Op::Push {
            e: Expr::ZeroSlots(2),
            cells: 2,
        },
        Op::AddrOfTop { cells: 2 },
        Op::Raise { depth: 4 },
    ];
    assert_eq!(steps("alloc_value", &[2, 2]), alloc);
    assert_eq!(steps("alloc_value", &[2, 1]), alloc[..2]);
    assert!(refused("alloc_value", &[2, 0]));
    let elements = Op::NewArray {
        ty: array,
        wrapper: "NewArray_type_80000014".to_owned(),
    };
    assert_eq!(steps("new_array_builtin", &[20]), [elements]);
    let own = Op::NewArray {
        ty: own_type(),
        wrapper: format!("NewArray_{OWN_TYPE}"),
    };
    assert_eq!(steps("new_array", &[0, 0, 0, 0]), [own]);
}

#[test]
fn an_array_is_reached_through_the_operations_a_listing_calls() {
    assert_eq!(
        steps("array_get_u64", &[]),
        [operation("array_get_u64", 2, 1)]
    );
    assert_eq!(
        steps("array_length_alt", &[]),
        [operation("array_length", 1, 1)]
    );
    assert_eq!(
        steps("array_set_ref", &[]),
        [operation("array_set_ref", 3, 0)]
    );
    // Eight bytes that are no reference: a listing calls this `array_set_ref` too.
    assert_eq!(
        steps("array_set_u64", &[]),
        [operation("array_set_u64", 3, 0)]
    );
    assert_eq!(
        steps("array_init_from_constant", &[]),
        [operation("initialize_from_constant", 2, 0)]
    );
    assert_eq!(
        steps("array_set_value", &[3]),
        [operation("array_set_slots", 5, 0)]
    );
    // The cells of an element are in no instruction.
    let element = Op::Unsized {
        callee: Callee::Intrinsic {
            name: "array_get_value".to_owned(),
            ty: None,
            wrapper: Some("array_get_value".to_owned()),
        },
        args: Some(2),
        returns: None,
    };
    assert_eq!(steps("array_get_value", &[]), [element]);
    assert_eq!(
        steps("bytes_equal", &[12, 0]),
        [operation("bytes_equal_12", 2, 1)]
    );
    let scaled = [
        Op::Cast(Scalar::U32),
        int(12),
        Op::Binary {
            op: BinOp::Mul,
            at: Kind::U32,
        },
        Op::Swap,
        Op::Load {
            ty: Scalar::U64,
            offset: 0,
        },
        Op::Swap,
        Op::ByteOffset,
    ];
    assert_eq!(steps("element_address", &[12]), scaled);
}

#[test]
fn an_intrinsic_a_listing_writes_in_place_takes_its_operands_as_constants() {
    let inline = |name: &str, args: u32, returns: u32| Op::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: None,
        },
        args,
        returns,
    };
    // Each operand byte is an argument, lowest first.
    let barrier = [
        int(2),
        int(1),
        int(4),
        int(3),
        inline("replication_barrier_461", 4, 0),
    ];
    assert_eq!(steps("barrier_461", &[2, 1, 4, 3]), barrier);
    assert_eq!(
        steps("barrier_467", &[]),
        [inline("replication_barrier_467", 0, 0)]
    );
    // The operand is the logarithm of the alignment a listing prints.
    assert_eq!(
        steps("stack_alloc", &[4]),
        [int(16), inline("stack_alloc_zeroed", 2, 1)]
    );
    assert!(refused("stack_alloc", &[31]));
    assert_eq!(
        steps("stack_address", &[2]),
        [int(2), inline("stack_address", 1, 1)]
    );
}

#[test]
fn a_cast_is_the_one_its_mode_says() {
    let named = |name: &str| Op::Call {
        callee: Callee::Intrinsic {
            name: format!("{name}<{OWN_TYPE}>"),
            ty: None,
            wrapper: Some(format!("{name}_{OWN_TYPE}_")),
        },
        args: 1,
        returns: 1,
    };
    assert_eq!(steps("cast", &[0, 0, 0, 0, 0]), [named("try_cast")]);
    // ⚠ Mode 1 must hold where mode 0 yields null: it is never written as mode 0 is.
    assert_eq!(steps("cast", &[0, 0, 0, 0, 1]), [named("cast")]);
    // A mode that is not understood keeps its number.
    assert_eq!(steps("cast", &[0, 0, 0, 0, 2]), [named("cast_mode_2")]);
    assert_eq!(steps("cast", &[0, 0, 0, 0, 3]), [named("cast_mode_3")]);
}

#[test]
fn a_type_token_is_the_type_it_names_where_the_tables_hold_it() {
    let token = Op::Call {
        callee: Callee::Intrinsic {
            name: "typeof".to_owned(),
            ty: Some(own_type()),
            wrapper: None,
        },
        args: 0,
        returns: 1,
    };
    assert_eq!(steps("push_type", &[0, 0, 0, 0]), [token]);
    // ⚠ Another program's type the project does not hold stays the number, as a listing prints it.
    let (bits, kind) = (1, Kind::F32);
    let number = Op::Push {
        e: Expr::Const(Const { bits, kind }),
        cells: 1,
    };
    assert_eq!(steps("push_type", &[1, 0, 0, 0]), [number]);
}

#[test]
fn an_instruction_the_interpreter_does_nothing_for_is_no_step() {
    // ⚠ The two sit among the conversions, and the interpreter leaves the cell as it is, all 64 bits.
    assert_eq!(steps("i64_to_i32", &[]), [Op::Nop]);
    assert_eq!(steps("u64_to_u32", &[]), [Op::Nop]);
    // A conversion the interpreter works is a step.
    assert_eq!(steps("i32_to_i64", &[]), [operation("i32_to_i64", 1, 1)]);
    // The branch among the tests for equal jumps as the test for unequal does.
    let unequal = steps("bne_f64", &[0, 0, 0, 0]);
    assert_eq!(steps("bne_f64_alt", &[0, 0, 0, 0]), unequal);
    let (op, at) = (BinOp::Ne, Kind::F64);
    assert!(unequal.contains(&Op::Binary { op, at }), "{unequal:?}");
    assert_eq!(unequal.last(), Some(&Op::Branch { to: 2 }));
    let equal = steps("beq_f32", &[0, 0, 0, 0]);
    let (op, at) = (BinOp::Eq, Kind::F32);
    assert!(equal.contains(&Op::Binary { op, at }), "{equal:?}");
}

#[test]
fn an_opcode_no_program_holds_has_a_length_and_no_steps() {
    let program = program(insn("ceq_f64", &[]));
    assert_eq!(raw(&program.code, 0).map(|raw| raw.len), Ok(2));
    let refusal = decode(&program, 0);
    assert!(matches!(refusal, Err(DisasmError::Malformed { at: 0, .. })));
    // Its sibling over floats occurs, and decodes.
    assert_eq!(steps("ceq_f32", &[]).len(), 5);
}

#[test]
fn the_disassembly_is_a_line_an_instruction_with_what_its_operands_resolve_to() {
    let body = code(&[
        insn("enter", &[0x10]),
        insn("push_param_0", &[]),
        insn("brz_short", &[6]),
        insn("call_short", &[0, 0x40, 0]),
        insn("ceq_f64", &[]),
        insn("dup", &[]),
        insn("cast", &[1, 0, 0, 0, 1]),
        insn("push_f1", &[]),
        insn("jump_short", &[0xFC]),
        insn("return_1", &[]),
    ]);
    let mut program = program(body);
    let end = program.code.len() as u32;
    program.functions.insert(
        0,
        FunctionRow {
            at: 0,
            end,
            params: 1,
            locals: 4,
            returns: 1,
        },
    );
    program.functions.truncate(1);
    let mut names = Names::default();
    let listed = text(&program, &names).expect("the program disassembles");
    let lines = [
        "func_000000: 1 parameter cell(s), 4 local cell(s), 1 returned",
        "    000000  enter frame=0x10",
        "    000003  push_param_0",
        "    000005  brz_short rel=6  ; -> 00000D",
        "    000008  call_short import=0 offset=0x40  ; func_000040, cells not stated",
        "    00000D  ceq_f64  ; unknown meaning",
        "    00000F  dup  ; meaning inferred",
        "    000011  cast type=1 mode=1  ; type 1 is another program's, and the project does not hold it",
        "    000018  push_f1",
        "    00001A  jump_short rel=-4  ; -> 000018",
        "    00001D  return_1",
        "",
    ];
    assert_eq!(listed, lines.join("\n"));
    // A name the user supplies stands for the slot name, at the head and at a call.
    let helper = FunctionKey::new(PROGRAM, 0x40);
    names.functions.insert(helper, "Helper".to_owned());
    let entry = FunctionKey::new(PROGRAM, 0);
    names.functions.insert(entry, "Entry".to_owned());
    let named = text(&program, &names).expect("the program disassembles");
    assert!(named.starts_with("Entry: 1 parameter cell(s)"));
    assert!(named.contains("offset=0x40  ; Helper, cells not stated\n"));
    // Bytes that are no instruction stop the text: the table ends before this number.
    let past = opcode::COUNT;
    program.code[13..15].copy_from_slice(&past.wrapping_mul(0x843D).to_le_bytes());
    let stopped = DisasmError::Unknown {
        at: 13,
        opcode: past,
    };
    assert_eq!(text(&program, &names), Err(stopped));
}

#[test]
fn an_opcode_only_the_interpreter_places_has_a_length_and_no_steps() {
    // A virtual call that names its type in four bytes: eight bytes in all.
    let wide = insn("call_virtual_w", &[1, 2, 0, 0, 0, 3]);
    let held = Raw {
        at: 0,
        len: 8,
        opcode: 135,
        fields: [1, 2, 3, 0],
    };
    assert_eq!(raw(&wide, 0), Ok(held));
    // The tail call the engine writes over code in memory: a pointer and an offset.
    let tail = insn("tail_call", &[0; 12]);
    assert_eq!(raw(&tail, 0).map(|raw| raw.len), Ok(14));
    let unwritten: [(&str, &[u8]); 4] = [
        ("call_virtual_w", &[1, 2, 0, 0, 0, 3]),
        ("tail_call", &[0; 12]),
        ("op_465", &[0, 0]),
        ("postinc_local", &[1]),
    ];
    for (name, operands) in unwritten {
        assert!(refused(name, operands), "{name}");
    }
    assert_eq!(first_unseen(&wide), Some((0, "call_virtual_w")));
}

#[test]
fn a_native_is_named_in_the_disassembly_where_a_list_states_its_handler() {
    let body = code(&[
        insn("call_native", &[4, 3, 2, 1, 0, 0]),
        insn("return", &[]),
    ]);
    let mut program = program(body);
    let end = program.code.len() as u32;
    program.functions = vec![FunctionRow {
        at: 0,
        end,
        params: 0,
        locals: 4,
        returns: 0,
    }];
    let mut names = Names::default();
    let bare = text(&program, &names).expect("the program disassembles");
    let line = "call_native rva=0x1020304 args=0 returns=0  ; Native.";
    assert!(bare.contains(&format!("{line}rva_01020304\n")), "{bare}");
    // The handler's hash alone, where no natives list names it.
    names.native_hashes.insert(0x0102_0304, 0xAA);
    let hashed = text(&program, &names).expect("the program disassembles");
    let by_hash = format!("{line}_0x00000000000000AA\n");
    assert!(hashed.contains(&by_hash), "{hashed}");
    let listed = NativeName {
        namespace: Some("DEMO".to_owned()),
        name: "DO_THING".to_owned(),
        ..NativeName::default()
    };
    names.natives = Some(NativeDb::from_names("test", [(0xAA, listed)]));
    let named = text(&program, &names).expect("the program disassembles");
    assert!(named.contains(&format!("{line}DEMO.DO_THING\n")), "{named}");
    // Another handler stays its address.
    names.native_hashes.clear();
    names.native_hashes.insert(0x0102_0305, 0xAA);
    assert_eq!(text(&program, &names).as_ref(), Ok(&bare));
}
