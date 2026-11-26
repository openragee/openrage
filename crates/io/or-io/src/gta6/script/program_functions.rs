//! The functions of a program's code, which no table of the container lists: found by the entry
//! instruction most begin with, by the calls of the program's own code and by the offsets its
//! tables name.
//!
//! ⚠ A function with no parameter and at most four locals has no entry instruction. One that
//! only another program calls is then found by nothing here and stays inside the function before
//! it, as code that function never reaches, until [`split`] is told of it.

use super::super::opcode;
use super::{FunctionRow, ProgramError};

const PUSH_STRING: u16 = 52;
const PUSH_FUNCTION: u16 = 126;
const ENTER: u16 = 131;
const ENTER_FLAGGED: u16 = 132;
const CALL: u16 = 133;
const CALL_SHORT: u16 = 134;
const RETURN: u16 = 144;
const RETURN_1: u16 = 145;
const RETURN_N: u16 = 146;
const ENTER_W: u16 = 398;

/// The local cells every frame has before an entry instruction asks for more.
const FRAME_LOCALS: u32 = 4;

/// One instruction: its opcode number, its length and its operand bytes as the code keeps them.
struct Insn<'a> {
    opcode: u16,
    len: usize,
    operands: &'a [u8],
}

impl<'a> Insn<'a> {
    /// The instruction at `at`, refused where the word is no opcode or the code ends inside it.
    fn at(code: &'a [u8], at: usize) -> Result<Insn<'a>, ProgramError> {
        let refused = |what: &str| ProgramError::Code {
            at: at as u32,
            what: what.to_owned(),
        };
        let word = code.get(at..at + 2);
        let word = word.ok_or_else(|| refused("the code ends inside an opcode"))?;
        let opcode = opcode::of_word(u16::from_le_bytes([word[0], word[1]]));
        // The opcode table states the length of every instruction, of one never seen too.
        let Some(spec) = opcode::spec(opcode) else {
            return Err(refused("the word here is no instruction"));
        };
        let len = spec.bytes() as usize;
        let operands = code.get(at + 2..at + len);
        let operands = operands.ok_or_else(|| refused("the code ends inside an instruction"))?;
        Ok(Insn {
            opcode,
            len,
            operands,
        })
    }

    /// Operand byte `n` in the clear.
    // ⚠ `n` is inside the instruction for every opcode read below: the opcode table gives each the operand bytes asked of it.
    fn byte(&self, n: usize) -> u32 {
        u32::from(opcode::plain(self.operands[n], n))
    }

    /// The `width` operand bytes from byte `from` as one number, the lowest byte first.
    fn number(&self, from: usize, width: usize) -> u32 {
        (from..from + width)
            .rev()
            .fold(0, |number, n| (number << 8) | self.byte(n))
    }
}

/// The functions of `code` by rising offset; `roots` are the offsets the program's tables name as functions.
pub(super) fn discover(code: &[u8], roots: &[u32]) -> Result<Vec<FunctionRow>, ProgramError> {
    let mut starts = roots.to_vec();
    if !code.is_empty() {
        starts.push(0);
    }
    // ponytail: a byte an offset of the code, ten megabytes for the largest program; a bit an offset if that ever shows.
    let mut instruction = vec![false; code.len()];
    let mut at = 0;
    while at < code.len() {
        let insn = Insn::at(code, at)?;
        instruction[at] = true;
        match insn.opcode {
            ENTER | ENTER_FLAGGED | ENTER_W => starts.push(at as u32),
            // Import 0 is the program itself; a call into another program says nothing of this code.
            CALL | PUSH_FUNCTION if insn.byte(0) == 0 => starts.push(insn.number(1, 3)),
            CALL_SHORT if insn.byte(0) == 0 => starts.push(insn.number(1, 2)),
            _ => {}
        }
        at += insn.len;
    }
    starts.sort_unstable();
    starts.dedup();
    let stray = |at: &&u32| instruction.get(**at as usize) != Some(&true);
    if let Some(at) = starts.iter().find(stray) {
        return Err(ProgramError::Code {
            at: *at,
            what: "a function is said to start here, where no instruction does".to_owned(),
        });
    }
    let ends = starts.iter().skip(1).copied().chain([code.len() as u32]);
    let rows = starts.iter().zip(ends).map(|(at, end)| row(code, *at, end));
    rows.collect()
}

/// Each function of another program the code calls or refers to: the place of that program among the imports, and the function's offset.
pub(super) fn foreign(code: &[u8]) -> Result<Vec<(u32, u32)>, ProgramError> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < code.len() {
        let insn = Insn::at(code, at)?;
        match insn.opcode {
            CALL | PUSH_FUNCTION if insn.byte(0) != 0 => {
                found.push((insn.byte(0), insn.number(1, 3)));
            }
            CALL_SHORT if insn.byte(0) != 0 => found.push((insn.byte(0), insn.number(1, 2))),
            _ => {}
        }
        at += insn.len;
    }
    Ok(found)
}

/// Each offset of the pooled data the code pushes as a string object, rising and once each.
pub(super) fn strings(code: &[u8]) -> Result<Vec<u32>, ProgramError> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < code.len() {
        let insn = Insn::at(code, at)?;
        if insn.opcode == PUSH_STRING {
            found.push(insn.number(0, 4));
        }
        at += insn.len;
    }
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

/// The row of the function that takes up `at..end`: its entry instruction, where it has one, and its first return.
fn row(code: &[u8], at: u32, end: u32) -> Result<FunctionRow, ProgramError> {
    let mut row = FunctionRow {
        at,
        end,
        locals: FRAME_LOCALS,
        ..FunctionRow::default()
    };
    let mut next = at as usize;
    let entry = Insn::at(code, next)?;
    match entry.opcode {
        // One byte: the parameter cells above the local cells asked beyond the frame's own.
        ENTER | ENTER_FLAGGED => {
            row.params = entry.byte(0) >> 4;
            row.locals += entry.byte(0) & 15;
        }
        ENTER_W => {
            row.params = entry.byte(0);
            row.locals += entry.number(1, 2);
        }
        _ => {}
    }
    // ⚠ The range may end in functions nothing of this program names. They follow the function's
    // own code, so the first return by offset is its own, and theirs are not read.
    while next < end as usize {
        let insn = Insn::at(code, next)?;
        match insn.opcode {
            RETURN => break,
            RETURN_1 => {
                row.returns = 1;
                break;
            }
            RETURN_N => {
                row.returns = insn.byte(0);
                break;
            }
            _ => next += insn.len,
        }
    }
    Ok(row)
}

/// Starts a function at `at`, inside the function that holds it; `at` has to be the start of an instruction.
pub(super) fn split(code: &[u8], rows: &mut Vec<FunctionRow>, at: u32) -> Result<(), ProgramError> {
    let refused = |what: &str| ProgramError::Code {
        at,
        what: what.to_owned(),
    };
    let after = rows.partition_point(|row| row.at <= at);
    let host = after.checked_sub(1).and_then(|n| rows.get(n)).copied();
    let Some(host) = host.filter(|host| at < host.end) else {
        return Err(refused(
            "a function is said to start here, outside the code",
        ));
    };
    if host.at == at {
        return Ok(());
    }
    let mut next = host.at as usize;
    while next < at as usize {
        next += Insn::at(code, next)?.len;
    }
    if next != at as usize {
        return Err(refused(
            "a function is said to start here, inside an instruction",
        ));
    }
    // Both rows are read before either is kept, so a refusal leaves the table as it was.
    let (before, started) = (row(code, host.at, at)?, row(code, at, host.end)?);
    rows[after - 1] = before;
    rows.insert(after, started);
    Ok(())
}
