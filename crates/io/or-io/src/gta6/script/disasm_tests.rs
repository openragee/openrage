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
const TABLE: u16 = 530;

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

/// Two functions: frame entries, locals by index, a field load, a branch, a native, a call, an unnamed opcode.
fn two_functions() -> Vec<u8> {
    let parts: [(u16, &[u8]); 12] = [
        (0x83, &[0x21]),
        (0x0C, &[3]),
        (0x0F, &[]),
        (0x3C, &[0x10]),
        (0xAD, &29i32.to_le_bytes()),
        (NATIVE, &[0x50, 0xF0, 0x4B, 0x04, 2, 1]),
        (0x85, &[0, 0, 0, 0, 0]),
        (0x7E, &[1, 2, 3, 4, 5]),
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
    let out = listing(&program).expect("the code decodes");
    assert_eq!((out.functions, out.instructions, out.named), (2, 12, 11));
    assert!(out.text.contains("function 0x000000:"), "{}", out.text);
    assert!(out.text.contains("function 0x00002C:"), "{}", out.text);
    let want: [(&str, &str); 12] = [
        ("000000", "083 enter args 2 locals 5"),
        ("000003", "00C load_local local3"),
        ("000006", "00F load_local local2"),
        ("000008", "03C load_i32 +0x18"),
        ("00000B", "0AD jgt_i32 if a > b -> 0x00002A"),
        ("000011", "08B native 0x044BF050 args 2 results 1"),
        ("000019", "085 call import 0 -> func 0x000000"),
        ("000020", "07E op_07E 01 02 03 04 05"),
        ("000027", "09D jump -> 0x00002A"),
        ("00002A", "091 return_value"),
        ("00002C", "18E enter args 1 locals 8 align 4"),
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
    let cases: [(u16, &[u8], &str); 16] = [
        (0x02, &[], "arg0"),
        (0x2B, &[], "-1"),
        (0x2D, &[0x7F], "0x7F"),
        (0x2E, &[0x78, 0x56, 0x34, 0x12], "0x12345678"),
        (0x40, &[0, 16], "+0x8 bytes 16"),
        (0x42, &[4], "+0x1C"),
        (0x5E, &[2, 0x40], "import 2 +0x40"),
        (0x84, &[0x10, 2], "args 1 locals 4 align 4"),
        (0x88, &[2, 0x34, 0x12, 1], "args 2 method 4660 01"),
        (0x8A, &[5, 3], "type 3 method 5"),
        (0x93, &[7], "type 0x7"),
        (0x9E, &[3, 0, 0, 0, 0x20, 0, 0, 0], "cases 3 table 0x20"),
        (0x9F, &[4, 0, 0, 0], "if a == 0 -> 0x000006"),
        (0x170, &[2, 1], "slots 2 slots 1"),
        (0x1AE, &[0, 0, 1, 0, 0], "import 0 +0x100"),
        (
            0x1F9,
            &[1, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0],
            "01 00 00 00 00 00 00 00 -> func 0x000040",
        ),
    ];
    for (opcode, plain, want) in cases {
        let row = op(opcode).expect("a named opcode");
        let width: u8 = row.fields.iter().map(|f| f.width).sum();
        assert_eq!(usize::from(width), plain.len(), "opcode {opcode:#X}");
        assert_eq!(operands(&at(0, opcode, plain)), want, "opcode {opcode:#X}");
    }
}

#[test]
fn every_named_opcode_fills_its_instruction_length() {
    let mut named = 0;
    for opcode in 0..TABLE {
        let Some(row) = op(opcode) else { continue };
        named += 1;
        let width: u8 = row.fields.iter().map(|f| f.width).sum();
        assert_eq!(Some(2 + width), length(opcode), "opcode {opcode:#X}");
        if row.implied.is_some() {
            assert!(
                row.fields.is_empty(),
                "opcode {opcode:#X} carries its operand"
            );
        }
    }
    assert_eq!(named, 492);
    assert_eq!(op(TABLE), None, "nothing is named past the table");
}

#[test]
fn a_family_member_carries_its_distance_from_the_base() {
    assert_eq!(op(0x04).and_then(|o| o.implied), Some((Kind::Arg, 2)));
    assert_eq!(op(0x13).and_then(|o| o.implied), Some((Kind::Local, 6)));
    assert_eq!(op(0x1E).and_then(|o| o.implied), Some((Kind::Local, 6)));
    assert_eq!(op(0x2A).and_then(|o| o.implied), Some((Kind::Imm, 9)));
    assert_eq!(operands(&at(0, 0x1B, &[])), "local3");
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
        let disp: Vec<_> = (row.fields.iter())
            .filter(|f| f.kind == Kind::Disp)
            .collect();
        if disp.is_empty() {
            assert_eq!(condition(row.mnemonic), None, "opcode {opcode:#X}");
            continue;
        }
        branches += 1;
        assert!(disp.len() == 1 && matches!(disp[0].width, 1 | 4));
        assert!(matches!(length(opcode), Some(3 | 6)), "opcode {opcode:#X}");
        assert_eq!(
            condition(row.mnemonic).is_none(),
            row.mnemonic == "jump",
            "opcode {opcode:#X} {}",
            row.mnemonic
        );
    }
    assert_eq!(branches, 40, "two jumps and 38 conditional branches");
}

#[test]
fn an_unnamed_opcode_prints_its_bytes() {
    assert_eq!(mnemonic(0x7E), None);
    assert_eq!(
        operands(&at(0, 0x7E, &[0xAB, 1, 2, 3, 4])),
        "AB 01 02 03 04"
    );
}

#[test]
fn a_decode_that_stops_is_the_error() {
    let b = body(&encode(NATIVE, &[0, 0]));
    let program = Compiled::open(&b).expect("the header reads");
    assert!(matches!(
        listing(&program),
        Err(CompiledError::Truncated { len: 8, .. })
    ));
}
