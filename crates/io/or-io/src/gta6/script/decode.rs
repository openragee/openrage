//! The disassembler: the bytes of a compiled program's functions decoded to instructions, each
//! with the steps it takes on the stack.
//!
//! An instruction is a 16-bit word that spells its opcode and the operand bytes the opcode fixes,
//! each stored under a key of its place. [`raw`] reads one against no table at all; [`decode_in`]
//! adds the steps, which name types, functions and globals through the program's tables.

use std::fmt::Write;

use super::ir::{Callee, Expr, FunctionKey};
use super::names::Names;
use super::native_addresses;
use super::opcode::{self, Op, Spec, Status, Type};
use super::program::{FunctionRow, Program};
use super::project::Project;

#[path = "decode_steps.rs"]
mod steps;

#[cfg(test)]
pub(super) use steps::CAST;
pub(super) use steps::TYPE_TOKEN;

use steps::Tables;

/// The steps one function may decode to.
// One instruction pushes up to 65,535 cells, a step each: unbounded, a few hundred bytes of
// such instructions decode to gigabytes.
const MAX_STEPS: usize = 1 << 20;

/// Why bytes were refused as instructions.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DisasmError {
    #[error("instruction {at:06X}: the code ends inside it")]
    Truncated { at: u32 },
    #[error("instruction {at:06X}: opcode {opcode} is no instruction")]
    Unknown { at: u32, opcode: u16 },
    #[error("instruction {at:06X}: {what}")]
    Malformed { at: u32, what: String },
}

/// One instruction as its bytes state it, read against no table of the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Raw {
    /// The offset of its first byte in the code.
    pub at: u32,
    /// Its length in bytes: the opcode word and the operands.
    pub len: u32,
    pub opcode: u16,
    /// The operands in the order of the opcode's fields, a signed one widened with its sign; the places past them are zero.
    pub fields: [u64; 4],
}

/// One instruction: where it is, how long it is and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insn {
    /// The offset of its first byte in the program's code.
    pub at: u32,
    /// Its length in bytes, never zero.
    pub len: u32,
    pub opcode: u16,
    /// The steps it takes, in order; a token of the code is resolved against the program's tables.
    pub ops: Vec<Op>,
}

fn read(code: &[u8], at: u32) -> Result<(Raw, &'static Spec), DisasmError> {
    let truncated = DisasmError::Truncated { at };
    let Some(rest) = code.get(at as usize..) else {
        return Err(truncated);
    };
    let Some(word) = rest.first_chunk::<2>() else {
        return Err(truncated);
    };
    let opcode = opcode::of_word(u16::from_le_bytes(*word));
    let Some(spec) = opcode::spec(opcode) else {
        return Err(DisasmError::Unknown { at, opcode });
    };
    let len = spec.bytes();
    let Some(stored) = rest.get(2..len as usize) else {
        return Err(truncated);
    };
    let plain = stored.iter().enumerate();
    let mut plain = plain.map(|(index, byte)| opcode::plain(*byte, index));
    let mut fields = [0u64; 4];
    for (field, slot) in spec.fields.iter().zip(&mut fields) {
        let mut value = 0u64;
        for byte in 0..field.1.bytes() {
            value |= u64::from(plain.next().unwrap_or(0)) << (8 * byte);
        }
        *slot = match field.1 {
            Type::S8 => value as u8 as i8 as u64,
            Type::S32 => value as u32 as i32 as u64,
            _ => value,
        };
    }
    let raw = Raw {
        at,
        len,
        opcode,
        fields,
    };
    Ok((raw, spec))
}

/// Reads the instruction at `at` of `code`: its opcode and its operands, with no table consulted.
pub fn raw(code: &[u8], at: u32) -> Result<Raw, DisasmError> {
    read(code, at).map(|(raw, _)| raw)
}

/// Decodes the instruction at `at` with the program's own tables alone.
pub fn decode(program: &Program, at: u32) -> Result<Insn, DisasmError> {
    decode_in(program, Project::empty(), at)
}

/// Decodes the instruction at `at`, taking from `project` what another program defines: its types and the cells of its functions.
pub fn decode_in(program: &Program, project: &Project, at: u32) -> Result<Insn, DisasmError> {
    let (raw, spec) = read(&program.code, at)?;
    let ops = steps::of(&Tables { program, project }, &raw, spec)?;
    Ok(Insn {
        at,
        len: raw.len,
        opcode: raw.opcode,
        ops,
    })
}

/// The first instruction of `code` whose opcode no program was seen to hold: its offset and its name.
// ⚠ Its length is its family's and no more: where every function after it starts rests on that.
pub fn first_unseen(code: &[u8]) -> Option<(u32, &'static str)> {
    let mut at = 0u32;
    while (at as usize) < code.len() {
        let (raw, spec) = read(code, at).ok()?;
        if spec.status == Status::Unseen {
            return Some((at, spec.name));
        }
        // An instruction is its opcode word at the least, so the walk ends.
        at = at.checked_add(raw.len.max(2))?;
    }
    None
}

/// The offset after an instruction of `len` bytes at `at`, refused where that is no place of the function.
// An instruction of no length would never end a walk, and one past the end is not the function's.
fn after(at: u32, len: u32, end: u32) -> Result<u32, DisasmError> {
    let next = at.checked_add(len).filter(|next| *next > at);
    match next {
        Some(next) if next <= end => Ok(next),
        _ => {
            let what = "it does not end inside its function".to_owned();
            Err(DisasmError::Malformed { at, what })
        }
    }
}

/// Every instruction of a function, in the order of their offsets, with the program's own tables alone.
pub fn function(program: &Program, row: &FunctionRow) -> Result<Vec<Insn>, DisasmError> {
    function_in(program, Project::empty(), row)
}

/// Every instruction of a function, in the order of their offsets.
pub fn function_in(
    program: &Program,
    project: &Project,
    row: &FunctionRow,
) -> Result<Vec<Insn>, DisasmError> {
    let mut insns = Vec::new();
    let (mut at, mut taken) = (row.at, 0usize);
    while at < row.end {
        let insn = decode_in(program, project, at)?;
        // Each case of a switch is held like a step.
        let cases = |op: &Op| match op {
            Op::Switch { cases, .. } => cases.len(),
            _ => 0,
        };
        taken += insn.ops.len() + insn.ops.iter().map(cases).sum::<usize>();
        if taken > MAX_STEPS {
            let what = "its function takes more steps than a decoder holds".to_owned();
            return Err(DisasmError::Malformed { at, what });
        }
        at = after(at, insn.len, row.end)?;
        insns.push(insn);
    }
    Ok(insns)
}

/// What a call reaches, as the disassembly names it: the user's name for a script function or a native, else the slot name.
fn called(callee: &Callee, program: u32, names: &Names) -> Option<String> {
    let function = match callee {
        Callee::Intrinsic { .. } => return None,
        // The code states a handler's address: a supplied list gives the native's hash, and its name.
        Callee::Native { rva, .. } => match native_addresses::listed(names, *rva) {
            Some(listed) => return Some(format!("Native.{listed}")),
            None => None,
        },
        Callee::Local { at, .. } => Some(FunctionKey::new(program, *at)),
        Callee::Import { program, at, .. } => Some(FunctionKey::new(*program, *at)),
        _ => None,
    };
    let named = function.and_then(|function| names.function(function));
    match (named, callee) {
        (Some(name), _) => Some(name.to_owned()),
        (None, Callee::Local { name, .. }) => Some(name.clone()),
        (None, other) => other.path(),
    }
}

/// What the disassembly says of one step beyond the instruction's operands.
fn note(op: &Op, program: u32, names: &Names) -> Option<String> {
    match op {
        Op::Jump { to } | Op::Branch { to } => Some(format!("-> {to:06X}")),
        Op::Switch { cases, default } => {
            let mut targets = String::from("->");
            for (_, to) in cases {
                let _ = write!(targets, " {to:06X}");
            }
            let _ = write!(targets, ", else {default:06X}");
            Some(targets)
        }
        Op::Call { callee, .. } => called(callee, program, names),
        Op::Unsized { callee, .. } => Some(match called(callee, program, names) {
            Some(name) => format!("{name}, cells not stated"),
            None => "cells not stated".to_owned(),
        }),
        // The top byte of the offset holds the reference's flags.
        Op::Push {
            e: Expr::FunctionRef { program: of, at },
            ..
        } => {
            let at = at & 0x00FF_FFFF;
            let named = names.function(FunctionKey::new(*of, at));
            Some(match (named, of == &program) {
                (Some(name), _) => name.to_owned(),
                (None, true) => format!("func_{at:06X}"),
                (None, false) => format!("program_{of:08X}.func_{at:06X}"),
            })
        }
        _ => None,
    }
}

/// One line of the disassembly: the offset, the mnemonic, each operand by name, then what the operands resolve to.
fn line(out: &mut String, tables: &Tables<'_>, names: &Names, raw: &Raw, spec: &Spec) {
    let _ = write!(out, "    {:06X}  {}", raw.at, spec.name);
    for (field, value) in spec.fields.iter().zip(raw.fields) {
        let name = field.0;
        let _ = match field.1 {
            Type::S8 | Type::S32 => write!(out, " {name}={}", value as i64),
            Type::F32 => write!(out, " {name}={:?}", f32::from_bits(value as u32)),
            _ if value < 10 => write!(out, " {name}={value}"),
            _ => write!(out, " {name}=0x{value:X}"),
        };
    }
    let mut notes = Vec::new();
    match (spec.status, steps::of(tables, raw, spec)) {
        (Status::Unseen, _) => notes.push("unknown meaning".to_owned()),
        (status, Ok(ops)) => {
            let program = tables.program.program;
            notes.extend(ops.iter().filter_map(|op| note(op, program, names)));
            if status == Status::Inferred {
                notes.push("meaning inferred".to_owned());
            }
        }
        (_, Err(DisasmError::Malformed { what, .. })) => notes.push(what),
        (_, Err(refused)) => notes.push(refused.to_string()),
    }
    if !notes.is_empty() {
        let _ = write!(out, "  ; {}", notes.join(", "));
    }
    out.push('\n');
}

/// Writes the disassembly of one function: a line that heads it, then a line an instruction.
// A line whose steps a table refuses still prints, with the refusal after it: only bytes that
// are no instruction stop the text.
pub fn write_function(
    out: &mut String,
    program: &Program,
    row: &FunctionRow,
    names: &Names,
) -> Result<(), DisasmError> {
    let project = Project::empty();
    let tables = Tables { program, project };
    let name = names.function(FunctionKey::new(program.program, row.at));
    let name = name.map_or_else(|| format!("func_{:06X}", row.at), str::to_owned);
    let _ = writeln!(
        out,
        "{name}: {} parameter cell(s), {} local cell(s), {} returned",
        row.params, row.locals, row.returns
    );
    let mut at = row.at;
    while at < row.end {
        let (raw, spec) = read(&program.code, at)?;
        line(out, &tables, names, &raw, spec);
        at = after(at, raw.len, row.end)?;
    }
    Ok(())
}

/// The disassembly of a whole program as text, each function as [`write_function`] writes it.
pub fn text(program: &Program, names: &Names) -> Result<String, DisasmError> {
    let mut out = String::new();
    for row in &program.functions {
        write_function(&mut out, program, row, names)?;
    }
    Ok(out)
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
