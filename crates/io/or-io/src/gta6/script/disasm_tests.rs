use byteorder::{ByteOrder, LE};

use super::*;
use crate::gta6::script::compiled::{
    check_copy, length, HEADER_LEN, MAX_OPERANDS, NATIVE, OPCODE_MULTIPLIER, OPERAND_KEY,
    OPERAND_KEY_HIGH, OPERAND_MULTIPLIER,
};

const HASH: u32 = 0x0BAD_F00D;
/// What a body pointer adds to the body offset it names.
const POINTER_BASE: u64 = 0x5000_0000;
/// The opcodes the interpreter's table holds.
const TABLE: u16 = opcode::COUNT;

fn inverse16(m: u16) -> u16 {
    (1..=u16::MAX).find(|x| x.wrapping_mul(m) == 1).unwrap_or(0)
}

fn inverse8(m: u8) -> u8 {
    (1..=u8::MAX).find(|x| x.wrapping_mul(m) == 1).unwrap_or(0)
}

/// One instruction as a body stores it, its operand bytes whitened.
fn encode(op: u16, plain: &[u8]) -> Vec<u8> {
    let mut out = (op.wrapping_mul(inverse16(OPCODE_MULTIPLIER)))
        .to_le_bytes()
        .to_vec();
    let inv = inverse8(OPERAND_MULTIPLIER);
    let key = OPERAND_KEY.iter().chain(&OPERAND_KEY_HIGH);
    out.extend(plain.iter().zip(key).map(|(p, k)| p.wrapping_mul(inv) ^ k));
    out
}

fn body(code: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; HEADER_LEN];
    LE::write_u32(&mut b[0x10..], HASH);
    LE::write_u32(&mut b[0x24..], check_copy(HASH));
    LE::write_u32(&mut b[0x28..], 1);
    LE::write_u32(&mut b[0x2C..], 1);
    LE::write_u32(&mut b[0xB8..], 1);
    LE::write_u64(&mut b[0xE8..], POINTER_BASE + HEADER_LEN as u64);
    LE::write_u32(&mut b[0xF8..], code.len() as u32);
    b.extend_from_slice(code);
    b
}

/// An instruction at `offset` holding `plain` as its decoded operands.
fn at(offset: usize, opcode: u16, plain: &[u8]) -> Instruction {
    let mut operands = [0u8; MAX_OPERANDS];
    operands[..plain.len()].copy_from_slice(plain);
    Instruction {
        offset,
        opcode,
        len: length(opcode).unwrap_or(2),
        operands,
    }
}

/// Two functions: frame entries, locals by index, a field load, a branch, a native, a call, a function reference.
fn two_functions() -> Vec<u8> {
    let parts: [(u16, &[u8]); 12] = [
        (0x83, &[0x21]),
        (0x0C, &[3]),
        (0x0F, &[]),
        (0x3C, &[0x10]),
        (0xAD, &29i32.to_le_bytes()),
        (NATIVE, &[0x0D, 0x0C, 0x0B, 0x0A, 3, 0]),
        (0x85, &[0, 0, 0, 0, 0]),
        (0x7E, &[0, 0xBB, 0x96, 0, 0]),
        (0x9D, &[1]),
        (0x91, &[]),
        (0x18E, &[1, 4, 0, 2]),
        (0x90, &[]),
    ];
    let mut code = Vec::new();
    for (op, plain) in parts {
        assert_eq!(
            length(op),
            Some(2 + plain.len() as u8),
            "opcode {op:#X} has the length this body is built on"
        );
        code.extend(encode(op, plain));
    }
    code
}

fn line<'a>(text: &'a str, offset: &str) -> Vec<&'a str> {
    (text.lines())
        .find(|l| l.split_whitespace().next() == Some(offset))
        .map(|l| l.split_whitespace().collect())
        .unwrap_or_default()
}

#[test]
fn a_hand_built_body_lists_its_functions_and_operands() {
    let b = body(&two_functions());
    let program = Compiled::open(&b).expect("the header reads");
    let out = listing(&program, None).expect("the code decodes");
    assert_eq!((out.functions, out.instructions, out.named), (2, 12, 12));
    assert!(out.text.contains("function 0x000000:"), "{}", out.text);
    assert!(out.text.contains("function 0x00002C:"), "{}", out.text);
    let want: [(&str, &str); 12] = [
        ("000000", "083 enter args 2 locals 5"),
        ("000003", "00C push_local local3"),
        ("000006", "00F push_local_2 local2"),
        ("000008", "03C load_i32_o8 +0x18"),
        ("00000B", "0AD bgt_i32 if a > b -> 0x00002A"),
        ("000011", "08B call_native 0x0A0B0C0D args 3 results 0"),
        ("000019", "085 call import 0 -> func 0x000000"),
        (
            "000020",
            "07E push_function import 0 -> func 0x0096BB flags 0",
        ),
        ("000027", "09D jump_short -> 0x00002A"),
        ("00002A", "091 return_1"),
        ("00002C", "18E enter_w args 1 locals 8 align 4"),
        ("000032", "090 return"),
    ];
    for (offset, rest) in want {
        let mut expected = vec![offset];
        expected.extend(rest.split_whitespace());
        assert_eq!(line(&out.text, offset), expected, "{}", out.text);
    }
    assert!(out
        .text
        .starts_with("; program 0x0BADF00D: 12 instructions in 2 functions"));
}

#[test]
fn each_operand_layout_prints_its_fields() {
    let cases: [(u16, &[u8], &str); 21] = [
        (0x02, &[], "arg0"),
        (0x2B, &[], "-1"),
        (0x2D, &[0x7F], "0x7F"),
        (0x2E, &[0x78, 0x56, 0x34, 0x12], "0x12345678"),
        (0x40, &[0, 16], "+0x8 bytes 16"),
        (0x42, &[4], "+0x1C"),
        (0x5E, &[2, 0x40], "import 2 +0x40"),
        (0x84, &[0x10, 2], "args 1 locals 4 align 4"),
        (0x88, &[2, 0x34, 0x12, 1], "args 2 type 0x1234 slot 1"),
        (0x8A, &[5, 3], "method 5 type 0x3"),
        (0x93, &[7], "type 0x7"),
        (0x9E, &[3, 0, 0, 0, 0x20, 0, 0, 0], "cases 3 table 0x20"),
        (0x9F, &[4, 0, 0, 0], "if a == 0 -> 0x000006"),
        // An ordered test of floats is the negation of its opposite.
        (0xB1, &[4, 0, 0, 0], "if !(a <= b) -> 0x000006"),
        // The branch that sits among the tests for equal jumps where its operands differ.
        (0xA8, &[4, 0, 0, 0], "if a != b -> 0x000006"),
        (0xAC, &[4, 0, 0, 0], "if a != b -> 0x000006"),
        (0xA7, &[4, 0, 0, 0], "if a == b -> 0x000006"),
        (0x170, &[2, 1], "slots 2 depth 1"),
        (0x1AE, &[0, 0, 1, 0, 0], "import 0 +0x100"),
        // The byte after a call's offset prints only where it is not zero.
        (
            0x85,
            &[1, 0x40, 0, 0, 7],
            "import 1 -> func 0x000040 zero 7",
        ),
        (
            0x1F9,
            &[1, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0],
            "01 00 00 00 00 00 00 00 -> func 0x000040",
        ),
    ];
    for (opcode, plain, want) in cases {
        let row = op(opcode).expect("a named opcode");
        assert_eq!(row.bytes() as usize, 2 + plain.len(), "opcode {opcode:#X}");
        assert_eq!(
            operands(&at(0, opcode, plain), None),
            want,
            "opcode {opcode:#X}"
        );
    }
}

#[test]
fn every_opcode_of_the_table_has_a_row_and_all_but_five_a_mnemonic() {
    let mut named = 0;
    for opcode in 0..TABLE {
        let row = opcode::spec(opcode).expect("a row for every opcode of the table");
        assert_eq!(u8::try_from(row.bytes()).ok(), length(opcode));
        if implied(row.act).is_some() {
            assert!(
                row.fields.is_empty(),
                "opcode {opcode:#X} carries its operand"
            );
        }
        named += usize::from(op(opcode).is_some());
    }
    // Of 0x1D1, 0x1D2, 0x1D5, 0x1FA and 0x210 only the length is known.
    assert_eq!(named, 525);
    assert!(op(TABLE).is_none(), "nothing is named past the table");
}

/// Opcodes of every operand shape, each with its mnemonic, its operands and what they print.
const SHAPES: [(u16, &str, &[u8], &str); 29] = [
    (0x34, "push_string", &[8, 0, 0, 0], "data 0x8"),
    (0x62, "builtin_global", &[3], "global 3"),
    (0x69, "store_f32s_o8", &[0, 2], "+0x8 slots 2"),
    (0x6A, "store_u64s_o8", &[8, 2], "+0x10 slots 2"),
    (0x77, "store_f32s", &[4, 3], "+0x4 slots 3"),
    (0x78, "store_u64s", &[0, 2], "+0x0 slots 2"),
    (
        0x7E,
        "push_function",
        &[1, 0x40, 0, 0, 0x40],
        "import 1 -> func 0x000040 flags 64",
    ),
    (
        0x7F,
        "push_virtual_function",
        &[0xFE, 0x15, 0, 0, 0x39],
        "type 0x15FE slot 57",
    ),
    (0x8D, "cast", &[0xA7, 2, 0, 0, 2], "type 0x2A7 mode 2"),
    (0x8E, "cast_builtin", &[5, 1], "type 0x5 mode 1"),
    (0x8F, "unbox_address_at", &[2], "depth 2"),
    (0x94, "array_set_ref", &[], ""),
    (0xF9, "deref_at", &[1], "depth 1"),
    (0x171, "new_builtin_value", &[0, 0x21], "method 0 type 0x21"),
    (0x173, "unbox", &[0x22, 0x79, 0, 0], "type 0x7922"),
    (0x174, "unbox_address", &[], ""),
    (0x176, "box_at", &[0x65, 0, 0, 0, 1], "type 0x65 depth 1"),
    (0x177, "store_value", &[0xC5, 0x11, 0, 0], "type 0x11C5"),
    (0x178, "store_value_builtin", &[0x15], "type 0x15"),
    (0x17C, "array_init_from_constant", &[], ""),
    (0x187, "array_set_u64", &[], ""),
    (0x188, "array_set_value", &[2], "slots 2"),
    (
        0x190,
        "push_virtual_function_w",
        &[1, 0, 0, 0, 0x2C, 1],
        "type 0x1 slot 300",
    ),
    (0x1CD, "barrier_461", &[0, 0, 1, 0], "+0x0 bytes 1"),
    (0x1CE, "barrier_462", &[0, 0, 4, 0], "+0x0 bytes 4"),
    (0x1D4, "barrier_468", &[2], "slots 2"),
    (0x1D6, "stack_alloc", &[4], "align 16"),
    (0x1D7, "element_address", &[0x10], "bytes 16"),
    (0x1D8, "stack_address", &[1], "depth 1"),
];

#[test]
fn each_operand_shape_decodes_to_its_length_and_prints_its_operands() {
    let mut code = Vec::new();
    for (opcode, _, plain, _) in SHAPES {
        code.extend(encode(opcode, plain));
    }
    let b = body(&code);
    let program = Compiled::open(&b).expect("the header reads");
    let decoded: Vec<_> = (program.instructions())
        .collect::<Result<_, _>>()
        .expect("the code decodes");
    assert_eq!(decoded.len(), SHAPES.len(), "one instruction each");
    for (ins, (opcode, name, plain, want)) in decoded.iter().zip(SHAPES) {
        assert_eq!(ins.opcode, opcode);
        assert_eq!(usize::from(ins.len), 2 + plain.len(), "opcode {opcode:#X}");
        assert_eq!(mnemonic(opcode), Some(name), "opcode {opcode:#X}");
        assert_eq!(operands(ins, None), want, "opcode {opcode:#X}");
    }
}

#[test]
fn a_family_member_carries_its_operand_in_its_opcode() {
    assert_eq!(operands(&at(0, 0x04, &[]), None), "arg2");
    assert_eq!(operands(&at(0, 0x13, &[]), None), "local6");
    assert_eq!(operands(&at(0, 0x1E, &[]), None), "local6");
    assert_eq!(operands(&at(0, 0x2A, &[]), None), "9");
    assert_eq!(operands(&at(0, 0x1B, &[]), None), "local3");
    // The member that takes its index as an operand carries none.
    assert_eq!(implied(Act::PushVar(Frame::Local, -1)), None);
}

#[test]
fn a_backward_displacement_counts_from_past_the_opcode_word() {
    assert_eq!(target(&at(0x40, 0x9D, &[(-6i8) as u8])), Some(0x3C));
    assert_eq!(target(&at(0x40, 0xA5, &(-0x43i32).to_le_bytes())), None);
    assert_eq!(
        target(&at(0x40, 0xA5, &(-0x30i32).to_le_bytes())),
        Some(0x12)
    );
    assert_eq!(target(&at(0x40, 0x14, &[1, 0])), None, "no jump, no target");
}

#[test]
fn every_jump_and_branch_has_a_displacement_width() {
    let mut branches = 0;
    for opcode in 0..TABLE {
        let Some(row) = op(opcode) else { continue };
        let disp: Vec<_> = (row.fields.iter()).filter(|f| f.0 == "rel").collect();
        if disp.is_empty() {
            assert_eq!(condition(row.act), None, "opcode {opcode:#X}");
            continue;
        }
        branches += 1;
        assert!(disp.len() == 1 && matches!(disp[0].1, Type::S8 | Type::S32));
        assert!(matches!(length(opcode), Some(3 | 6)), "opcode {opcode:#X}");
        assert_eq!(
            condition(row.act).is_none(),
            row.act == Act::Jump,
            "opcode {opcode:#X} {}",
            row.name
        );
    }
    assert_eq!(branches, 40, "two jumps and 38 conditional branches");
}

#[test]
fn an_unnamed_opcode_prints_its_bytes() {
    assert_eq!(mnemonic(0x210), None);
    assert_eq!(
        operands(&at(0, 0x210, &[0xAB, 1, 2, 3]), None),
        "AB 01 02 03"
    );
}

#[test]
fn a_line_says_how_far_its_opcode_is_established() {
    let parts: [(u16, &[u8]); 4] = [
        (0x35, &[]),
        (0x87, &[1, 2, 0, 0, 0, 3]),
        (0x1FA, &[]),
        (0x90, &[]),
    ];
    let mut code = Vec::new();
    for (op, plain) in parts {
        code.extend(encode(op, plain));
    }
    let b = body(&code);
    let program = Compiled::open(&b).expect("the header reads");
    let out = listing(&program, None).expect("the code decodes");
    assert_eq!((out.instructions, out.named), (4, 3), "{}", out.text);
    let lines: Vec<&str> = out.text.lines().skip(3).collect();
    let want = [
        "  000000  035  dup  ; meaning inferred",
        "  000002  087  call_virtual_w           args 1 type 0x2 slot 3  ; met in no program",
        "  00000A  1FA  op_1FA",
        "  00000C  090  return",
    ];
    assert_eq!(lines, want, "{}", out.text);
}

/// A lifted listing declaring `SET_THING` at executable address 0x0A0B0C0D.
const DECLARING: &str = "[VmProgram(0x00000001U)]\npublic static class Program_00000001\n{\n    \
     public static class Native\n    {\n        public static class PLAYER\n        {\n            \
     public static void SET_THING(VmValue p0) => Vm.Native(0xAAUL, 0xA0B0C0DU, 0, p0);\n        \
     }\n    }\n}\n";

#[test]
fn a_native_prints_its_name_where_a_table_maps_its_address() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.ysc.cs");
    std::fs::write(&path, DECLARING).expect("write a listing");
    let table = NativeAddresses::read(&[path]).expect("the listing reads");
    let mapped = at(0, NATIVE, &[0x0D, 0x0C, 0x0B, 0x0A, 3, 0]);
    assert_eq!(
        operands(&mapped, Some(&table)),
        "SET_THING args 3 results 0"
    );
    assert_eq!(operands(&mapped, None), "0x0A0B0C0D args 3 results 0");
    let unmapped = at(0, NATIVE, &[0x0E, 0x0C, 0x0B, 0x0A, 0, 0]);
    assert_eq!(
        operands(&unmapped, Some(&table)),
        "0x0A0B0C0E args 0 results 0"
    );

    let b = body(&two_functions());
    let program = Compiled::open(&b).expect("the header reads");
    let named = listing(&program, Some(&table)).expect("the code decodes");
    let want = [
        "000011",
        "08B",
        "call_native",
        "SET_THING",
        "args",
        "3",
        "results",
        "0",
    ];
    assert_eq!(line(&named.text, "000011"), want, "{}", named.text);
    assert!(named.text.contains("the 1 natives"), "{}", named.text);
    let bare = listing(&program, None).expect("the code decodes");
    assert!(
        bare.text.contains("no table of native names"),
        "{}",
        bare.text
    );
}

#[test]
fn a_decode_that_stops_is_the_error() {
    let b = body(&encode(NATIVE, &[0, 0]));
    let program = Compiled::open(&b).expect("the header reads");
    assert!(matches!(
        listing(&program, None),
        Err(CompiledError::Truncated { len: 8, .. })
    ));
}
