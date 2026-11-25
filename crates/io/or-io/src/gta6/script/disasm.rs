//! A compiled GTA VI script printed as a listing: one line per instruction, function by function.
//!
//! ⚠ A mnemonic is printed only where the interpreter's handler or the lifted listings of the same
//! programs give the opcode one meaning on every occurrence; every other opcode prints
//! as `op_XXX`. A function opens at an `enter`, so a function that starts without one runs on under
//! the one before it.

use std::fmt::Write;

use super::{Compiled, CompiledError, Instruction, NATIVE};

/// A listing and what it counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub text: String,
    pub functions: usize,
    pub instructions: usize,
    /// Instructions printed under an established mnemonic rather than `op_XXX`.
    pub named: usize,
}

/// The mnemonic of an opcode whose meaning the listing comparison established, in the listing vocabulary's words.
pub fn mnemonic(opcode: u16) -> Option<&'static str> {
    Some(match opcode {
        // Only ever the first instruction of a function.
        0x83 | 0x84 | 0x18E => "enter",
        // Ends its block, carries no displacement, and agrees with the function's result count.
        0x90..=0x92 => "return",
        // A displacement to an instruction of its function; a jump's successor is never reached by falling through.
        0x9C | 0x9D => "jump",
        0x9F..=0xA7 | 0xA9..=0xAB | 0xAD..=0xB5 | 0xB7..=0xBB | 0xBD..=0xC4 => "branch",
        NATIVE => "native",
        // In the listing vocabulary's words, each the one word the listings give every occurrence.
        0x49 | 0x4B | 0x1A1 => "extract_bytes",
        0x60 | 0x172 | 0x1A8 | 0x1C9 => "load_bytes",
        0x175 => "typed_intrinsic",
        0x1D6 => "intrinsic",
        0x1E0 => "packed_vector_lerp",
        0x1E4 => "packed_vector_normalize4",
        _ => return None,
    })
}

/// The code offset a jump or branch goes to: its signed operand (1 byte in a 3-byte instruction, 4 in a 6-byte one) past the opcode word.
pub fn target(ins: &Instruction) -> Option<usize> {
    if !matches!(mnemonic(ins.opcode), Some("jump" | "branch")) {
        return None;
    }
    let o = ins.operands;
    let disp = match ins.len {
        3 => i64::from(o[0] as i8),
        6 => i64::from(i32::from_le_bytes([o[0], o[1], o[2], o[3]])),
        _ => return None,
    };
    usize::try_from(i64::try_from(ins.offset).ok()? + 2 + disp).ok()
}

/// The whole code region as a listing; a decode that stops is the error.
pub fn listing(program: &Compiled) -> Result<Listing, CompiledError> {
    let mut body = String::new();
    let (mut functions, mut instructions, mut named) = (0, 0, 0);
    for ins in program.instructions() {
        let ins = ins?;
        instructions += 1;
        let name = mnemonic(ins.opcode);
        if name == Some("enter") {
            functions += 1;
            let _ = writeln!(body, "\nfunction {:#08X}:", ins.offset);
        }
        named += usize::from(name.is_some());
        let shown = name.map_or_else(|| format!("op_{:03X}", ins.opcode), str::to_owned);
        let line = format!(
            "  {:06X}  {:03X}  {shown:<24} {}",
            ins.offset,
            ins.opcode,
            operands(&ins)
        );
        let _ = writeln!(body, "{}", line.trim_end());
    }
    let mut text = String::new();
    let _ = writeln!(
        text,
        "; program {:#010X}: {instructions} instructions in {functions} functions, {named} under an established mnemonic",
        program.header.hash
    );
    text.push_str(
        "; a native prints by its address, an op_XXX opcode by its decoded operand bytes\n",
    );
    text.push_str(body.trim_end());
    text.push('\n');
    Ok(Listing {
        text,
        functions,
        instructions,
        named,
    })
}

/// An instruction's operands as text: a native's address and counts, a jump's target, else the decoded bytes.
fn operands(ins: &Instruction) -> String {
    let o = ins.operands;
    if ins.opcode == NATIVE {
        let rva = u32::from_le_bytes([o[0], o[1], o[2], o[3]]);
        return format!("{rva:#010X} args {} results {}", o[4], o[5]);
    }
    if let Some(to) = target(ins) {
        return format!("-> {to:#08X}");
    }
    let n = usize::from(ins.len).saturating_sub(2).min(o.len());
    let bytes = &o[..n];
    match n {
        0 => String::new(),
        1 | 2 | 4 | 8 => {
            let mut wide = [0u8; 8];
            wide[..n].copy_from_slice(bytes);
            format!("{:#0w$X}", u64::from_le_bytes(wide), w = 2 + 2 * n)
        }
        _ => bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[cfg(test)]
#[path = "disasm_tests.rs"]
mod tests;
