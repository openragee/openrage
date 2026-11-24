use byteorder::{ByteOrder, LE};

use super::*;
use crate::gta6::script::compiled::{
    check_copy, length, HEADER_LEN, OPCODE_MULTIPLIER, OPERAND_KEY, OPERAND_KEY_HIGH,
    OPERAND_MULTIPLIER,
};

const HASH: u32 = 0x0BAD_F00D;
/// What a body pointer adds to the body offset it names.
const POINTER_BASE: u64 = 0x5000_0000;

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

/// Two functions: an enter, a native call, a branch and a jump to the return, an unnamed opcode.
fn two_functions() -> Vec<u8> {
    let parts: [(u16, &[u8]); 8] = [
        (0x83, &[0x10]),
        (NATIVE, &[0x50, 0xF0, 0x4B, 0x04, 2, 1]),
        (0xA0, &[11]),
        (0x14, &[0x34, 0x12]),
        (0x9C, &4i32.to_le_bytes()),
        (0x90, &[]),
        (0x18E, &[1, 4, 0, 0]),
        (0x91, &[]),
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
    assert_eq!((out.functions, out.instructions, out.named), (2, 8, 7));
    assert!(out.text.contains("function 0x000000:"), "{}", out.text);
    assert!(out.text.contains("function 0x00001A:"), "{}", out.text);
    assert_eq!(
        line(&out.text, "000000"),
        ["000000", "083", "enter", "0x10"]
    );
    assert_eq!(
        line(&out.text, "000003"),
        [
            "000003",
            "08B",
            "native",
            "0x044BF050",
            "args",
            "2",
            "results",
            "1"
        ]
    );
    assert_eq!(
        line(&out.text, "00000B"),
        ["00000B", "0A0", "branch", "->", "0x000018"]
    );
    assert_eq!(
        line(&out.text, "00000E"),
        ["00000E", "014", "op_014", "0x1234"]
    );
    assert_eq!(
        line(&out.text, "000012"),
        ["000012", "09C", "jump", "->", "0x000018"]
    );
    assert_eq!(line(&out.text, "000018"), ["000018", "090", "return"]);
    assert_eq!(
        line(&out.text, "00001A"),
        ["00001A", "18E", "enter", "0x00000401"]
    );
    assert!(out
        .text
        .starts_with("; program 0x0BADF00D: 8 instructions in 2 functions"));
}

#[test]
fn a_backward_displacement_counts_from_past_the_opcode_word() {
    let at = |opcode, len, operands: &[u8]| {
        let mut o = [0u8; crate::gta6::script::compiled::MAX_OPERANDS];
        o[..operands.len()].copy_from_slice(operands);
        Instruction {
            offset: 0x40,
            opcode,
            len,
            operands: o,
        }
    };
    assert_eq!(target(&at(0x9D, 3, &[(-6i8) as u8])), Some(0x3C));
    assert_eq!(target(&at(0xA5, 6, &(-0x43i32).to_le_bytes())), None);
    assert_eq!(target(&at(0xA5, 6, &(-0x30i32).to_le_bytes())), Some(0x12));
    assert_eq!(target(&at(0x14, 4, &[1, 0])), None, "no jump, no target");
}

#[test]
fn every_jump_and_branch_has_a_displacement_width() {
    for op in 0..=u16::MAX {
        let Some(len) = length(op) else { break };
        if matches!(mnemonic(op), Some("jump" | "branch")) {
            assert!(len == 3 || len == 6, "opcode {op:#X} is {len} bytes long");
        }
    }
    assert_eq!(
        mnemonic(0xA8),
        None,
        "an opcode the comparison did not settle prints as op_"
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
