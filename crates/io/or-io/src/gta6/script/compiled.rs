//! A compiled GTA VI script program, read from its decrypted body: the container header, the
//! instruction stream under the interpreter's two whitening rules, and its native calls.
//!
//! ⚠ An instruction's length is the interpreter's own per opcode ([`length`]); where it is not
//! known the decode stops with [`CompiledError::Unsized`] rather than guess, and nothing turns the
//! stream into the decompiler's input yet.

use std::collections::BTreeSet;

use byteorder::{ByteOrder, LE};

/// The vector the interpreter XORs into the 8 operand bytes after an opcode word, at executable address 0x146178EE0.
pub const OPERAND_KEY: [u8; 8] = [0x85, 0x94, 0xA7, 0xB6, 0xC1, 0xD0, 0xE3, 0xF2];
/// The multiplier that turns a raw 16-bit code word into its dispatch index.
pub const OPCODE_MULTIPLIER: u16 = 0x3315;
/// The multiplier applied to an operand byte after the XOR, keeping the low byte.
pub const OPERAND_MULTIPLIER: u8 = 0x6D;
/// The decoded opcode of a native call: operand bytes 0..4 the native's RVA, 4 the argument count, 5 the result count.
pub const NATIVE: u16 = 0x8B;
/// The RVA a program's check copy masks its hash with; the game places natives at that RVA less the unmasked value.
pub const NATIVE_ANCHOR_RVA: u32 = 0x01D1_09B0;
/// The bytes of the smallest header, before the code region.
pub const HEADER_LEN: usize = 0x180;

/// What a pointer in the body adds to the body offset it names.
const POINTER_BASE: u64 = 0x5000_0000;

/// Each opcode's instruction length in bytes, the opcode word included; 0 where it is not known.
///
/// Read from the interpreter's handlers (the next fetch's distance from the opcode); 422 of them
/// tile every function of 689 programs exactly at the offsets their listings print. 0x1D3, 0x1D4 and
/// 0x1F9 are unsized: no handler read gives their length, and one program alone uses the first two.
static LENGTHS: [u8; 530] = [
    2, 3, 2, 2, 2, 2, 2, 4, 4, 3, 4, 3, 3, 2, 2, 2, // 0x000
    2, 2, 2, 2, 4, 4, 3, 3, 2, 2, 2, 2, 2, 2, 2, 4, // 0x010
    3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 6, 6, // 0x020
    10, 6, 18, 3, 6, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // 0x030
    4, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 5, // 0x040
    3, 3, 3, 3, 3, 3, 3, 3, 4, 3, 4, 4, 4, 4, 4, 4, // 0x050
    5, 4, 3, 3, 3, 3, 3, 3, 4, 4, 4, 3, 3, 3, 3, 3, // 0x060
    4, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 5, 7, 7, // 0x070
    2, 3, 3, 3, 4, 7, 5, 8, 6, 5, 4, 8, 6, 7, 4, 3, // 0x080
    2, 2, 3, 3, 2, 3, 6, 7, 2, 6, 2, 2, 6, 3, 10, 6, // 0x090
    3, 6, 6, 3, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, // 0x0A0
    6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, // 0x0B0
    6, 6, 6, 6, 6, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x0C0
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x0D0
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 4, 10, 18, 2, 2, 2, // 0x0E0
    2, 2, 2, 4, 2, 2, 2, 2, 2, 3, 2, 2, 2, 2, 2, 2, // 0x0F0
    2, 2, 2, 3, 3, 3, 3, 3, 3, 2, 3, 3, 3, 3, 3, 3, // 0x100
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x110
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x120
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x130
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x140
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // 0x150
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 4, // 0x160
    4, 4, 3, 6, 2, 4, 7, 6, 3, 4, 4, 6, 2, 2, 2, 2, // 0x170
    2, 2, 2, 2, 2, 2, 2, 2, 3, 4, 6, 4, 4, 6, 6, 9, // 0x180
    8, 4, 5, 4, 4, 4, 4, 4, 4, 6, 4, 6, 6, 6, 6, 6, // 0x190
    6, 8, 4, 4, 4, 4, 4, 4, 6, 4, 7, 7, 7, 7, 7, 7, // 0x1A0
    9, 7, 4, 4, 4, 4, 4, 6, 4, 4, 4, 4, 4, 6, 7, 7, // 0x1B0
    7, 7, 9, 7, 7, 7, 7, 7, 7, 9, 7, 9, 2, 6, 6, 6, // 0x1C0
    6, 4, 4, 0, 0, 2, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, // 0x1D0
    2, 2, 2, 2, 2, 2, 2, 2, 2, 14, 3, 3, 3, 3, 3, 3, // 0x1E0
    3, 3, 3, 3, 3, 7, 7, 4, 7, 0, 2, 7, 7, 8, 8, 4, // 0x1F0
    4, 4, 4, 2, 6, 8, 7, 7, 6, 2, 6, 6, 2, 2, 2, 6, // 0x200
    6, 7, // 0x210
];

/// Why a body is not read as a compiled program, or its code stopped decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompiledError {
    #[error("the body is {len} bytes, shorter than a program header")]
    Short { len: usize },
    #[error("the body's first 8 bytes are not zero, so it is no compiled program")]
    NoZeroSlot,
    #[error("the check copy {check:#010X} does not match the program hash {hash:#010X}")]
    CheckCopy { hash: u32, check: u32 },
    #[error("the two type counts disagree: {first} and {second}")]
    TypeCounts { first: u32, second: u32 },
    #[error(
        "the code at pointer {pointer:#X}, {len:#X} bytes long, lies outside the {body}-byte body"
    )]
    CodeRange { pointer: u64, len: u32, body: usize },
    #[error("at code offset {offset:#X}: opcode {opcode:#X} is past the interpreter's table")]
    OutOfTable { offset: usize, opcode: u16 },
    #[error("at code offset {offset:#X}: the length of opcode {opcode:#X} is not known")]
    Unsized { offset: usize, opcode: u16 },
    #[error("at code offset {offset:#X}: a {len}-byte instruction runs past the code's end")]
    Truncated { offset: usize, len: u8 },
}

/// The decoded opcode of a raw 16-bit code word.
pub fn opcode(word: u16) -> u16 {
    word.wrapping_mul(OPCODE_MULTIPLIER)
}

/// The decoded operand bytes of the up to 8 raw bytes after an opcode word; bytes past 8 stay zero.
pub fn operands(raw: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for ((o, r), k) in out.iter_mut().zip(raw).zip(OPERAND_KEY) {
        *o = (r ^ k).wrapping_mul(OPERAND_MULTIPLIER);
    }
    out
}

/// An opcode's instruction length in bytes, or `None` where it is past the table or not known.
pub fn length(opcode: u16) -> Option<u8> {
    LENGTHS
        .get(usize::from(opcode))
        .copied()
        .filter(|l| *l != 0)
}

/// The check copy a program with `hash` carries at +0x24.
pub fn check_copy(hash: u32) -> u32 {
    (hash ^ NATIVE_ANCHOR_RVA).rotate_left(12)
}

/// What the program header states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// The joaat of the program's name, at +0x10.
    pub hash: u32,
    /// The types the program defines, at +0x28 and again at +0x2C.
    pub types: u32,
    /// The distinct natives the program calls, at +0xB8.
    pub natives: u32,
    /// The code region's body offset, from the pointer at +0xE8.
    pub code_start: usize,
    /// The code region's length in bytes, at +0xF8.
    pub code_len: usize,
}

impl Header {
    /// Read and check the header at the start of a decrypted body.
    pub fn read(body: &[u8]) -> Result<Self, CompiledError> {
        if body.len() < HEADER_LEN {
            return Err(CompiledError::Short { len: body.len() });
        }
        if LE::read_u64(body) != 0 {
            return Err(CompiledError::NoZeroSlot);
        }
        let hash = LE::read_u32(&body[0x10..]);
        let check = LE::read_u32(&body[0x24..]);
        if check != check_copy(hash) {
            return Err(CompiledError::CheckCopy { hash, check });
        }
        let (first, second) = (LE::read_u32(&body[0x28..]), LE::read_u32(&body[0x2C..]));
        if first != second {
            return Err(CompiledError::TypeCounts { first, second });
        }
        let pointer = LE::read_u64(&body[0xE8..]);
        let len = LE::read_u32(&body[0xF8..]);
        let range = (pointer.checked_sub(POINTER_BASE))
            .and_then(|start| usize::try_from(start).ok())
            .filter(|start| *start >= HEADER_LEN)
            .and_then(|start| Some((start, start.checked_add(usize::try_from(len).ok()?)?)))
            .filter(|(_, end)| *end <= body.len());
        let Some((code_start, code_end)) = range else {
            return Err(CompiledError::CodeRange {
                pointer,
                len,
                body: body.len(),
            });
        };
        Ok(Self {
            hash,
            types: first,
            natives: LE::read_u32(&body[0xB8..]),
            code_start,
            code_len: code_end - code_start,
        })
    }
}

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    /// The offset in the code region, the unit a listing's function offsets count in.
    pub offset: usize,
    pub opcode: u16,
    /// The length in bytes, the opcode word included.
    pub len: u8,
    /// The decoded operand bytes, as many as the instruction holds up to 8; the rest zero.
    pub operands: [u8; 8],
}

/// A native call: where it is and what it states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCall {
    pub offset: usize,
    /// The native's address relative to the image base.
    pub rva: u32,
    pub args: u8,
    pub returns: u8,
}

/// What a whole program decodes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub header: Header,
    pub instructions: usize,
    pub native_calls: usize,
    /// The distinct natives the calls name, which the header's count states.
    pub natives_found: usize,
}

/// A decrypted body read as a compiled program.
#[derive(Debug, Clone, Copy)]
pub struct Compiled<'a> {
    pub header: Header,
    code: &'a [u8],
}

impl<'a> Compiled<'a> {
    /// Read the header of a decrypted body and hold its code region.
    pub fn open(body: &'a [u8]) -> Result<Self, CompiledError> {
        let header = Header::read(body)?;
        let code = &body[header.code_start..header.code_start + header.code_len];
        Ok(Self { header, code })
    }

    /// The code region's instructions from offset 0, stopping after the first error.
    pub fn instructions(&self) -> Instructions<'a> {
        Instructions {
            code: self.code,
            at: 0,
            failed: false,
        }
    }

    /// Every native call, in code order.
    pub fn natives(&self) -> Result<Vec<NativeCall>, CompiledError> {
        let mut out = Vec::new();
        for ins in self.instructions() {
            let ins = ins?;
            if ins.opcode == NATIVE {
                let o = ins.operands;
                out.push(NativeCall {
                    offset: ins.offset,
                    rva: u32::from_le_bytes([o[0], o[1], o[2], o[3]]),
                    args: o[4],
                    returns: o[5],
                });
            }
        }
        Ok(out)
    }

    /// Decode the whole code region and count what it holds.
    pub fn facts(&self) -> Result<Facts, CompiledError> {
        let instructions = self.instructions().try_fold(0, |n, i| i.map(|_| n + 1))?;
        let natives = self.natives()?;
        let found: BTreeSet<u32> = natives.iter().map(|n| n.rva).collect();
        Ok(Facts {
            header: self.header,
            instructions,
            native_calls: natives.len(),
            natives_found: found.len(),
        })
    }
}

/// The instructions of a code region, see [`Compiled::instructions`].
#[derive(Debug, Clone)]
pub struct Instructions<'a> {
    code: &'a [u8],
    at: usize,
    failed: bool,
}

impl Instructions<'_> {
    fn next_instruction(&mut self) -> Result<Instruction, CompiledError> {
        let offset = self.at;
        let truncated = |len| CompiledError::Truncated { offset, len };
        let word = (self.code.get(offset..offset + 2)).ok_or_else(|| truncated(2))?;
        let opcode = opcode(LE::read_u16(word));
        if usize::from(opcode) >= LENGTHS.len() {
            return Err(CompiledError::OutOfTable { offset, opcode });
        }
        let len = length(opcode).ok_or(CompiledError::Unsized { offset, opcode })?;
        let end = offset + usize::from(len);
        let bytes = (self.code.get(offset..end)).ok_or_else(|| truncated(len))?;
        self.at += usize::from(len);
        Ok(Instruction {
            offset,
            opcode,
            len,
            operands: operands(&bytes[2..]),
        })
    }
}

impl Iterator for Instructions<'_> {
    type Item = Result<Instruction, CompiledError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.at >= self.code.len() {
            return None;
        }
        let item = self.next_instruction();
        self.failed = item.is_err();
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: u32 = 0x1234_5678;

    /// The multiplicative inverse of an odd `m` modulo 2^16.
    fn inverse16(m: u16) -> u16 {
        (1..=u16::MAX).find(|x| x.wrapping_mul(m) == 1).unwrap_or(0)
    }

    fn inverse8(m: u8) -> u8 {
        (1..=u8::MAX).find(|x| x.wrapping_mul(m) == 1).unwrap_or(0)
    }

    /// One instruction as the game stores it: the opcode word and its operand bytes whitened.
    fn encode(op: u16, plain: &[u8]) -> Vec<u8> {
        let mut out = op
            .wrapping_mul(inverse16(OPCODE_MULTIPLIER))
            .to_le_bytes()
            .to_vec();
        let inv = inverse8(OPERAND_MULTIPLIER);
        out.extend(
            plain
                .iter()
                .zip(OPERAND_KEY)
                .map(|(p, k)| p.wrapping_mul(inv) ^ k),
        );
        out
    }

    /// A body with a native call and a return in its code.
    fn body(code: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; HEADER_LEN];
        LE::write_u32(&mut b[0x10..], HASH);
        LE::write_u32(&mut b[0x24..], check_copy(HASH));
        LE::write_u32(&mut b[0x28..], 3);
        LE::write_u32(&mut b[0x2C..], 3);
        LE::write_u32(&mut b[0xB8..], 1);
        LE::write_u64(&mut b[0xE8..], POINTER_BASE + HEADER_LEN as u64);
        LE::write_u32(&mut b[0xF8..], code.len() as u32);
        b.extend_from_slice(code);
        b
    }

    fn native_then_return() -> Vec<u8> {
        let mut code = encode(NATIVE, &[0x56, 0x34, 0x12, 0x00, 2, 1]);
        code.extend(encode(0x90, &[]));
        code
    }

    #[test]
    fn a_hand_built_program_decodes_its_native_call() {
        let b = body(&native_then_return());
        let program = Compiled::open(&b).expect("the header reads");
        assert_eq!(program.header.hash, HASH);
        assert_eq!(program.header.code_start, HEADER_LEN);
        let natives = program.natives().expect("the code decodes");
        let want = NativeCall {
            offset: 0,
            rva: 0x0012_3456,
            args: 2,
            returns: 1,
        };
        assert_eq!(natives, vec![want]);
        let facts = program.facts().expect("the code decodes");
        assert_eq!((facts.instructions, facts.native_calls), (2, 1));
        assert_eq!(facts.natives_found as u32, facts.header.natives);
    }

    #[test]
    fn a_wrong_multiplier_does_not_decode() {
        let code = native_then_return();
        let word = LE::read_u16(&code);
        assert_eq!(opcode(word), NATIVE);
        assert_ne!(word.wrapping_mul(OPCODE_MULTIPLIER + 2), NATIVE);
        let right = operands(&code[2..8]);
        let mut wrong = [0u8; 6];
        for ((w, r), k) in wrong.iter_mut().zip(&code[2..8]).zip(OPERAND_KEY) {
            *w = (r ^ k).wrapping_mul(OPERAND_MULTIPLIER + 2);
        }
        assert_eq!(right[..6], [0x56, 0x34, 0x12, 0x00, 2, 1]);
        assert_ne!(wrong, right[..6]);
    }

    #[test]
    fn a_header_whose_fields_disagree_is_refused() {
        let good = body(&native_then_return());
        let with = |at: usize, v: u32| {
            let mut b = good.clone();
            LE::write_u32(&mut b[at..], v);
            Header::read(&b)
        };
        assert_eq!(with(0, 1), Err(CompiledError::NoZeroSlot));
        assert!(matches!(
            with(0x24, HASH.rotate_left(12)),
            Err(CompiledError::CheckCopy { .. })
        ));
        assert!(matches!(
            with(0x2C, 4),
            Err(CompiledError::TypeCounts { .. })
        ));
        assert!(matches!(
            with(0xF8, 0x1000),
            Err(CompiledError::CodeRange { .. })
        ));
        assert!(matches!(
            with(0xE8, 0x100),
            Err(CompiledError::CodeRange { .. })
        ));
        assert_eq!(
            Header::read(&good[..0x100]),
            Err(CompiledError::Short { len: 0x100 })
        );
    }

    #[test]
    fn an_opcode_of_no_known_length_stops_the_decode() {
        let mut code = encode(0x90, &[]);
        code.extend(encode(0x1F9, &[0; 8]));
        let b = body(&code);
        let program = Compiled::open(&b).expect("the header reads");
        let read: Vec<_> = program.instructions().collect();
        assert_eq!(read.len(), 2);
        assert_eq!(
            read[1],
            Err(CompiledError::Unsized {
                offset: 2,
                opcode: 0x1F9
            })
        );

        let b = body(&encode(LENGTHS.len() as u16, &[]));
        assert!(matches!(
            Compiled::open(&b).and_then(|p| p.facts()),
            Err(CompiledError::OutOfTable { .. })
        ));
        let b = body(&encode(NATIVE, &[0, 0]));
        assert!(matches!(
            Compiled::open(&b).and_then(|p| p.facts()),
            Err(CompiledError::Truncated { len: 8, .. })
        ));
    }
}
