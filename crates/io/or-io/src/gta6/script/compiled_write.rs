//! A compiled GTA VI script program written back: each instruction encoded again from its decoded
//! opcode and operands, every other byte of the body as read, and the edits that keep its length.
//!
//! ⚠ An edit changes only an instruction's operand bytes, so the body keeps its length and can go
//! into a patched copy of its archive. No run of the game was made with an edited program.

use super::compiled::disasm;
use super::compiled::{
    Compiled, CompiledError, Instruction, MAX_OPERANDS, OPCODE_MULTIPLIER, OPERAND_KEY,
    OPERAND_KEY_HIGH, OPERAND_MULTIPLIER,
};
use super::opcode::Act;

/// The inverse of an odd `m` modulo 2^16, by Newton's step (each doubles the correct low bits).
const fn inverse16(m: u16) -> u16 {
    let mut x = m;
    let mut i = 0;
    while i < 4 {
        x = x.wrapping_mul(2u16.wrapping_sub(m.wrapping_mul(x)));
        i += 1;
    }
    x
}

const OPCODE_INVERSE: u16 = inverse16(OPCODE_MULTIPLIER);
const OPERAND_INVERSE: u8 = inverse16(OPERAND_MULTIPLIER as u16) as u8;

/// Why an edit was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    #[error(transparent)]
    Read(#[from] CompiledError),
    #[error("no instruction starts at code offset {offset:#X}")]
    NoInstruction { offset: usize },
    #[error(
        "the instruction at code offset {offset:#X} is a branch ({mnemonic}): its operand is a code position, which an edit does not change"
    )]
    Branch {
        offset: usize,
        mnemonic: &'static str,
    },
    #[error(
        "the instruction at code offset {offset:#X} (opcode {opcode:#X}) is not a push of one immediate value"
    )]
    NotAPush { offset: usize, opcode: u16 },
    #[error("{value} does not fit the {width}-byte immediate at code offset {offset:#X}")]
    DoesNotFit {
        offset: usize,
        value: i128,
        width: u8,
    },
    #[error(
        "the push at code offset {offset:#X} holds {width} bytes; a float goes only into a 4-byte push"
    )]
    NotFloatWidth { offset: usize, width: u8 },
}

/// An instruction as the game stores it: the opcode word and its operand bytes whitened.
pub fn encode(ins: &Instruction) -> Vec<u8> {
    let mut out = ins
        .opcode
        .wrapping_mul(OPCODE_INVERSE)
        .to_le_bytes()
        .to_vec();
    let n = usize::from(ins.len).saturating_sub(2).min(MAX_OPERANDS);
    let key = OPERAND_KEY.iter().chain(&OPERAND_KEY_HIGH);
    out.extend(
        (ins.operands[..n].iter())
            .zip(key)
            .map(|(p, k)| p.wrapping_mul(OPERAND_INVERSE) ^ k),
    );
    out
}

/// The body with every instruction its code decodes to encoded again; unchanged, it equals the input.
pub fn write_back(body: &[u8]) -> Result<Vec<u8>, CompiledError> {
    let program = Compiled::open(body)?;
    let start = program.header.code_start;
    let mut out = body.to_vec();
    for ins in program.instructions() {
        let ins = ins?;
        let at = start + ins.offset;
        out[at..at + usize::from(ins.len)].copy_from_slice(&encode(&ins));
    }
    Ok(out)
}

/// The push at `offset`, the width of its immediate and its body offset.
fn push_at(body: &[u8], offset: usize) -> Result<(Instruction, u8, usize), EditError> {
    let program = Compiled::open(body)?;
    let found = (program.instructions())
        .find(|i| i.as_ref().map_or(true, |i| i.offset >= offset))
        .transpose()?
        .filter(|i| i.offset == offset);
    let ins = found.ok_or(EditError::NoInstruction { offset })?;
    let not_a_push = EditError::NotAPush {
        offset,
        opcode: ins.opcode,
    };
    let Some(op) = disasm::op(ins.opcode) else {
        return Err(not_a_push);
    };
    // An operand that is a place in the code: a jump's distance, a switch's table, a function's offset.
    let code = matches!(
        op.act,
        Act::Jump
            | Act::Switch
            | Act::BranchZero(..)
            | Act::BranchCompare(..)
            | Act::Call
            | Act::TailCall
            | Act::PushFunction
    );
    if code {
        return Err(EditError::Branch {
            offset,
            mnemonic: op.name,
        });
    }
    match (op.act, op.fields) {
        (Act::PushImm | Act::PushI64 | Act::PushType, [field]) => {
            // A field is at most eight bytes wide.
            let width = u8::try_from(field.1.bytes()).unwrap_or(u8::MAX);
            Ok((ins, width, program.header.code_start + offset))
        }
        _ => Err(not_a_push),
    }
}

fn store(body: &mut [u8], mut ins: Instruction, at: usize, bytes: &[u8]) {
    ins.operands[..bytes.len()].copy_from_slice(bytes);
    body[at..at + usize::from(ins.len)].copy_from_slice(&encode(&ins));
}

/// Set the integer a push at code `offset` pushes; it must fit the push's immediate, signed or unsigned.
pub fn set_int(body: &mut [u8], offset: usize, value: i128) -> Result<(), EditError> {
    let (ins, width, at) = push_at(body, offset)?;
    let bits = u32::from(width.min(8)) * 8;
    let fits = value >= -(1i128 << (bits - 1)) && value < (1i128 << bits);
    if !fits {
        return Err(EditError::DoesNotFit {
            offset,
            value,
            width,
        });
    }
    store(body, ins, at, &value.to_le_bytes()[..usize::from(width)]);
    Ok(())
}

/// Set the 32-bit float a 4-byte push at code `offset` pushes.
pub fn set_float(body: &mut [u8], offset: usize, value: f32) -> Result<(), EditError> {
    let (ins, width, at) = push_at(body, offset)?;
    if width != 4 {
        return Err(EditError::NotFloatWidth { offset, width });
    }
    store(body, ins, at, &value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use byteorder::{ByteOrder, LE};

    use super::*;
    use crate::gta6::script::compiled::{check_copy, length, opcode, operands, HEADER_LEN};

    fn ins(op: u16, plain: &[u8]) -> Instruction {
        let mut operands = [0u8; MAX_OPERANDS];
        operands[..plain.len()].copy_from_slice(plain);
        Instruction {
            offset: 0,
            opcode: op,
            len: length(op).expect("in the table"),
            operands,
        }
    }

    /// A body around `code`, with `tail` bytes standing for the tables after the code.
    fn body(code: &[Instruction], tail: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; HEADER_LEN];
        LE::write_u32(&mut b[0x10..], 0x1234_5678);
        LE::write_u32(&mut b[0x24..], check_copy(0x1234_5678));
        LE::write_u64(&mut b[0xE8..], 0x5000_0000 + HEADER_LEN as u64);
        let bytes: Vec<u8> = code.iter().flat_map(encode).collect();
        LE::write_u32(&mut b[0xF8..], bytes.len() as u32);
        b.extend(bytes);
        b.extend_from_slice(tail);
        b
    }

    /// push 1-byte, push 4-byte, jump, native, push 8-byte, return; offsets 0, 3, 9, 15, 23, 33.
    fn sample() -> Vec<Instruction> {
        vec![
            ins(0x2D, &[7]),
            ins(0x2E, &0x3F00_0000u32.to_le_bytes()),
            ins(0x9C, &[2, 0, 0, 0]),
            ins(0x8B, &[0x56, 0x34, 0x12, 0x00, 2, 1]),
            ins(0x30, &[1, 2, 3, 4, 5, 6, 7, 8]),
            ins(0x90, &[]),
        ]
    }

    fn pushed(body: &[u8], offset: usize) -> Instruction {
        let program = Compiled::open(body).expect("the header reads");
        (program.instructions().flatten())
            .find(|i| i.offset == offset)
            .expect("the instruction")
    }

    #[test]
    fn the_inverses_undo_the_multipliers() {
        assert_eq!(OPCODE_MULTIPLIER.wrapping_mul(OPCODE_INVERSE), 1);
        assert_eq!(OPERAND_MULTIPLIER.wrapping_mul(OPERAND_INVERSE), 1);
    }

    #[test]
    fn every_opcode_word_and_operand_byte_encodes_back() {
        for word in 0..=u16::MAX {
            let op = opcode(word);
            let len = match length(op) {
                Some(len) => len,
                None => continue,
            };
            let raw: Vec<u8> = (0..usize::from(len) - 2)
                .map(|i| (usize::from(word) + 37 * i) as u8)
                .collect();
            let mut stored = word.to_le_bytes().to_vec();
            stored.extend(&raw);
            let decoded = Instruction {
                offset: 0,
                opcode: op,
                len,
                operands: operands(&raw),
            };
            assert_eq!(encode(&decoded), stored, "word {word:#06X}");
        }
    }

    #[test]
    fn a_program_written_back_is_byte_identical() {
        let tail: Vec<u8> = (0..300u32).map(|i| (i * 89 + 5) as u8).collect();
        let b = body(&sample(), &tail);
        assert_eq!(write_back(&b), Ok(b.clone()));

        // Past a patched tail call the stale bytes are kept as read.
        let mut code = vec![ins(0x1F9, &[0; 12])];
        code.extend(sample());
        let b = body(&code, &tail);
        assert_eq!(write_back(&b), Ok(b));
    }

    #[test]
    fn an_integer_edit_changes_only_its_operand_bytes() {
        let source = body(&sample(), &[0xAA; 32]);
        let mut b = source.clone();
        set_int(&mut b, 3, -2).expect("a 4-byte push");
        set_int(&mut b, 0, 200).expect("a 1-byte push");
        set_int(&mut b, 23, i64::MIN.into()).expect("an 8-byte push");
        assert_eq!(b.len(), source.len());
        assert_eq!(pushed(&b, 3).operands[..4], (-2i32).to_le_bytes());
        assert_eq!(pushed(&b, 0).operands[0], 200);
        assert_eq!(pushed(&b, 23).operands[..8], i64::MIN.to_le_bytes());
        let changed: Vec<usize> = (0..b.len()).filter(|i| b[*i] != source[*i]).collect();
        let operand_bytes = |at: usize, n: usize| HEADER_LEN + at + 2..HEADER_LEN + at + 2 + n;
        let edited = [
            operand_bytes(0, 1),
            operand_bytes(3, 4),
            operand_bytes(23, 8),
        ];
        assert!(changed.iter().all(|i| edited.iter().any(|r| r.contains(i))));
        assert!(!changed.is_empty());
        assert_eq!(write_back(&b), Ok(b.clone()));
    }

    #[test]
    fn a_float_edit_writes_its_bits() {
        let mut b = body(&sample(), &[]);
        set_float(&mut b, 3, 2.5).expect("a 4-byte push");
        let got = pushed(&b, 3).operands;
        assert_eq!(f32::from_le_bytes([got[0], got[1], got[2], got[3]]), 2.5);
    }

    #[test]
    fn what_cannot_keep_its_length_is_refused_by_name() {
        let source = body(&sample(), &[]);
        let mut b = source.clone();
        let branch = EditError::Branch {
            offset: 9,
            mnemonic: "jump",
        };
        assert_eq!(set_int(&mut b, 9, 1), Err(branch));
        let not_a_push = EditError::NotAPush {
            offset: 15,
            opcode: 0x8B,
        };
        assert_eq!(set_int(&mut b, 15, 1), Err(not_a_push));
        assert_eq!(
            set_int(&mut b, 4, 1),
            Err(EditError::NoInstruction { offset: 4 })
        );
        assert_eq!(
            set_int(&mut b, 999, 1),
            Err(EditError::NoInstruction { offset: 999 })
        );
        for value in [256, -129, 0xFFFF_FFFF_FFFF_FF80, -0xFFFF_FFFF_FFFF_FFFF] {
            let refused = EditError::DoesNotFit {
                offset: 0,
                value,
                width: 1,
            };
            assert_eq!(set_int(&mut b, 0, value), Err(refused));
        }
        let narrow = EditError::NotFloatWidth {
            offset: 0,
            width: 1,
        };
        assert_eq!(set_float(&mut b, 0, 1.0), Err(narrow));
        assert_eq!(b, source, "a refused edit writes nothing");
    }
}
