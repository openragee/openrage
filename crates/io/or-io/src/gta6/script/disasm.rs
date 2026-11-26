//! A compiled GTA VI script printed as a listing: one line per instruction, function by function.
//!
//! ⚠ The listing holds no table of its own. A mnemonic, an operand's width and what an operand
//! holds are read off the row [`opcode::spec`] gives; an opcode of which only the length is known
//! prints as `op_XXX` with its decoded operand bytes. A function opens at an entry instruction,
//! so a function that starts without one runs on under the one before it.

use std::fmt::Write;

use super::{Compiled, CompiledError, Instruction};
use crate::gta6::script::ir::BinOp;
use crate::gta6::script::native_addresses::NativeAddresses;
use crate::gta6::script::opcode::{self, Act, Field, Frame, Spec, Status, Type};

/// A listing and what it counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub text: String,
    pub functions: usize,
    pub instructions: usize,
    /// Instructions printed under a mnemonic of the table rather than `op_XXX`.
    pub named: usize,
}

/// The row of an opcode the table says more of than its length.
pub fn op(opcode: u16) -> Option<&'static Spec> {
    opcode::spec(opcode).filter(|row| row.act != Act::Unknown)
}

/// The mnemonic of an opcode the table names.
pub fn mnemonic(opcode: u16) -> Option<&'static str> {
    op(opcode).map(|row| row.name)
}

/// Whether a row is an instruction a function begins with.
fn enters(row: &Spec) -> bool {
    matches!(row.name, "enter" | "enter_flagged" | "enter_w")
}

/// The operand fields of an instruction, each with its little-endian value and its bytes.
fn values<'a>(
    row: &'static Spec,
    ins: &'a Instruction,
) -> impl Iterator<Item = (&'static Field, u64, &'a [u8])> + 'a {
    let mut at = 0;
    row.fields.iter().map(move |field| {
        let width = field.1.bytes() as usize;
        let bytes = ins.operands.get(at..at + width).unwrap_or(&[]);
        at += width;
        let value = (bytes.iter().rev()).fold(0u64, |v, b| (v << 8) | u64::from(*b));
        (field, value, bytes)
    })
}

/// The code offset a jump or branch goes to: its signed displacement past the opcode word.
pub fn target(ins: &Instruction) -> Option<usize> {
    let row = op(ins.opcode)?;
    let (field, value, _) = values(row, ins).find(|(field, _, _)| field.0 == "rel")?;
    let disp = match field.1 {
        Type::S8 => i64::from(value as u8 as i8),
        Type::S32 => i64::from(value as u32 as i32),
        _ => return None,
    };
    usize::try_from(i64::try_from(ins.offset).ok()? + 2 + disp).ok()
}

/// What a conditional branch tests: `a` is the deeper operand, `b` the top.
// ⚠ An ordered test of floats is the negation of its opposite, which differs from the test itself where an operand is no number.
fn condition(act: Act) -> Option<&'static str> {
    let (op, not) = match act {
        Act::BranchZero(_, BinOp::Eq) => return Some("a == 0"),
        Act::BranchZero(_, BinOp::Ne) => return Some("a != 0"),
        Act::BranchCompare(op, _, not) => (op, not),
        _ => return None,
    };
    Some(match (op, not) {
        (BinOp::Eq, false) => "a == b",
        (BinOp::Ne, false) => "a != b",
        (BinOp::Gt, false) => "a > b",
        (BinOp::Ge, false) => "a >= b",
        (BinOp::Lt, false) => "a < b",
        (BinOp::Le, false) => "a <= b",
        (BinOp::Gt, true) => "!(a > b)",
        (BinOp::Ge, true) => "!(a >= b)",
        (BinOp::Lt, true) => "!(a < b)",
        (BinOp::Le, true) => "!(a <= b)",
        _ => return None,
    })
}

/// The frame an instruction's `index` or `first` operand counts in.
fn frame(act: Act) -> Option<Frame> {
    match act {
        Act::PushVar(frame, _) | Act::SetVar(frame, _) | Act::StepVar(frame, _) => Some(frame),
        Act::PushVars(frame) | Act::PushVar2(frame) => Some(frame),
        Act::SetVars(frame) | Act::SetVar2(frame) | Act::AddrOfVar(frame) => Some(frame),
        Act::TeeVar | Act::TeeVar2 => Some(Frame::Local),
        Act::PostStepParam(_) => Some(Frame::Param),
        _ => None,
    }
}

/// The bytes an instruction adds to its `offset` operand: those before the data of the reference it goes through.
fn base(act: Act) -> u64 {
    let bytes = match act {
        Act::Load(_, base) | Act::LoadP0(_, base) | Act::Store(_, base) => base,
        Act::LoadBytes(base) | Act::LoadVector(base) | Act::Offset(base) => base,
        Act::StoreBytes(base) | Act::StoreF32s(base) | Act::StoreU64s(base) => base,
        Act::StoreVector(base) | Act::StepField(base, _) => base,
        _ => 0,
    };
    u64::from(bytes)
}

/// The local slots every frame has before its entry instruction adds its own.
const BASE_LOCALS: u64 = 4;

fn slot(frame: Frame, index: u64) -> String {
    match frame {
        Frame::Param => format!("arg{index}"),
        Frame::Local => format!("local{index}"),
    }
}

/// The operand an opcode of a family carries in itself: a slot index, a constant.
fn implied(act: Act) -> Option<String> {
    match act {
        Act::PushVar(frame, index) | Act::SetVar(frame, index) => {
            let index = u64::try_from(index).ok()?;
            Some(slot(frame, index))
        }
        Act::PushInt(value) => Some(value.to_string()),
        _ => None,
    }
}

/// One operand as text, by what its field holds; `None` for one that says nothing.
fn operand(
    row: &Spec,
    ins: &Instruction,
    (field, v, bytes): (&Field, u64, &[u8]),
    natives: Option<&NativeAddresses>,
) -> Option<String> {
    let name = field.0;
    Some(match name {
        "index" | "first" => match (frame(row.act), row.act) {
            (Some(frame), _) => slot(frame, v),
            (None, Act::BuiltinGlobal) => format!("global {v}"),
            (None, _) => format!("{name} {v}"),
        },
        "count" if row.act == Act::Switch => format!("cases {v}"),
        "count" | "cells" => format!("slots {v}"),
        "size" => format!("bytes {v}"),
        "offset" => match row.act {
            Act::Call | Act::PushFunction | Act::TailCall => format!("-> func {v:#08X}"),
            Act::PushString => format!("data {v:#X}"),
            act => format!("+{:#X}", v + base(act)),
        },
        "import" => format!("import {v}"),
        "rel" => {
            let to = target(ins).map_or_else(|| "?".to_owned(), |t| format!("{t:#08X}"));
            match condition(row.act) {
                Some(test) => format!("if {test} -> {to}"),
                None => format!("-> {to}"),
            }
        }
        "frame" => format!("args {} locals {}", v >> 4, BASE_LOCALS + (v & 0xF)),
        "args" | "params" => format!("args {v}"),
        "locals" => format!("locals {}", BASE_LOCALS + v),
        "returns" => format!("results {v}"),
        "align_log2" => format!("align {}", 1u64.checked_shl(v as u32).unwrap_or(0)),
        "rva" => match natives.and_then(|t| t.name_of(v as u32)) {
            Some(name) => name.to_owned(),
            None => format!("{v:#010X}"),
        },
        "type" => format!("type {v:#X}"),
        "table" => format!("table {v:#X}"),
        // A byte that is zero wherever the instruction occurs.
        "zero" if v == 0 => return None,
        "raw" | "raw_4" | "program" => hex_bytes(bytes),
        "slot" | "method" | "depth" | "mode" | "flags" | "zero" => format!("{name} {v}"),
        // A constant the instruction pushes or selects.
        _ => format!("{v:#0w$X}", w = 2 + 2 * bytes.len()),
    })
}

/// An instruction's operands as text by the table's fields, or its decoded bytes where the table names none; a native by the name `natives` gives its address.
fn operands(ins: &Instruction, natives: Option<&NativeAddresses>) -> String {
    let Some(row) = op(ins.opcode) else {
        let n = usize::from(ins.len)
            .saturating_sub(2)
            .min(ins.operands.len());
        return hex_bytes(&ins.operands[..n]);
    };
    let mut parts: Vec<String> = implied(row.act).into_iter().collect();
    for field in values(row, ins) {
        parts.extend(operand(row, ins, field, natives));
    }
    parts.join(" ")
}

fn hex_bytes(bytes: &[u8]) -> String {
    (bytes.iter())
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a line says after its operands of how far the instruction's meaning is established.
fn standing(row: &Spec) -> &'static str {
    match row.status {
        Status::Matched => "",
        Status::Inferred => "  ; meaning inferred",
        Status::Unseen => "  ; met in no program",
    }
}

/// The whole code region as a listing, each native named where `natives` maps its address; a decode that stops is the error.
pub fn listing(
    program: &Compiled,
    natives: Option<&NativeAddresses>,
) -> Result<Listing, CompiledError> {
    let mut body = String::new();
    let (mut functions, mut instructions, mut named) = (0, 0, 0);
    for ins in program.instructions() {
        let ins = ins?;
        instructions += 1;
        let row = op(ins.opcode);
        if row.is_some_and(enters) {
            functions += 1;
            let _ = writeln!(body, "\nfunction {:#08X}:", ins.offset);
        }
        named += usize::from(row.is_some());
        let shown = match row {
            Some(row) => row.name.to_owned(),
            None => format!("op_{:03X}", ins.opcode),
        };
        let line = format!(
            "  {:06X}  {:03X}  {shown:<24} {}",
            ins.offset,
            ins.opcode,
            operands(&ins, natives)
        );
        let _ = writeln!(body, "{}{}", line.trim_end(), row.map_or("", standing));
    }
    let mut text = String::new();
    let _ = writeln!(
        text,
        "; program {:#010X}: {instructions} instructions in {functions} functions, {named} under a mnemonic",
        program.header.hash
    );
    text.push_str("; an op_XXX opcode prints its decoded operand bytes; a and b are the deeper and the top operand\n");
    let _ = match natives {
        Some(table) => writeln!(
            text,
            "; a native prints its name where the {} natives of the lifted listings map its address, else its address",
            table.len()
        ),
        None => writeln!(
            text,
            "; a native prints its address: no table of native names was given"
        ),
    };
    text.push_str(body.trim_end());
    text.push('\n');
    Ok(Listing {
        text,
        functions,
        instructions,
        named,
    })
}

#[cfg(test)]
#[path = "disasm_tests.rs"]
mod tests;
