//! A compiled GTA VI script printed as a listing: one line per instruction, function by function.
//!
//! ⚠ [`op`] names an opcode only where its interpreter handler was read (what it does to the operand
//! stack, the frame and the program counter); every other opcode prints as `op_XXX` with its decoded
//! operand bytes. A function opens at an `enter`, so a function that starts without one runs on
//! under the one before it.

use std::fmt::Write;

use super::{Compiled, CompiledError, Instruction};
use crate::gta6::script::native_addresses::NativeAddresses;

/// A listing and what it counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub text: String,
    pub functions: usize,
    pub instructions: usize,
    /// Instructions printed under a mnemonic of the table rather than `op_XXX`.
    pub named: usize,
}

/// What an operand field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An argument slot of the frame, by index.
    Arg,
    /// A local slot of the frame, by index.
    Local,
    /// A count of 8-byte stack slots.
    Slots,
    /// A count of bytes.
    Bytes,
    /// A byte offset past the op's header from the reference or pointer the instruction goes through.
    Offset,
    /// A constant the instruction pushes or selects.
    Imm,
    /// An index into the program's import table; 0 is the program itself.
    Import,
    /// A code offset in the program the import names.
    Code,
    /// A signed displacement counted from past the opcode word.
    Disp,
    /// A frame's arguments (high nibble) and its locals past the four every frame has (low nibble).
    Frame,
    /// The arguments a call passes or a frame takes.
    Args,
    /// The locals a frame has past the four every frame has.
    Locals,
    /// The results a native call leaves.
    Results,
    /// The log2 of the slots a frame's locals are aligned to.
    Align,
    /// A virtual method's index.
    Method,
    /// A builtin: its type in the high byte, its method in the low.
    Builtin,
    /// A native's address relative to the image base.
    Native,
    /// A type reference.
    Type,
    /// The cases of a switch.
    Count,
    /// A switch's jump table, as an offset in the program's data.
    Table,
    /// An offset in the program's data.
    Data,
    /// A builtin global, by its index.
    Global,
    /// Bytes the handler does not read as a value here.
    Raw,
}

/// One operand field: what it holds and its width in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub kind: Kind,
    pub width: u8,
}

/// What one opcode does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Op {
    pub mnemonic: &'static str,
    /// The operand fields in byte order after the opcode word.
    pub fields: &'static [Field],
    /// The operand stack before and after, top rightmost.
    pub stack: &'static str,
    /// What an `Offset` field adds: the bytes before the data of the reference it goes through.
    pub header: u8,
    /// The operand an opcode of a family carries in itself (a slot index, a constant).
    pub implied: Option<(Kind, i32)>,
}

const fn f(kind: Kind, width: u8) -> Field {
    Field { kind, width }
}

const fn row(
    mnemonic: &'static str,
    fields: &'static [Field],
    stack: &'static str,
    header: u8,
) -> Op {
    Op {
        mnemonic,
        fields,
        stack,
        header,
        implied: None,
    }
}

impl Op {
    /// The family member `opcode`, whose operand is its distance from the family's `base`.
    const fn implied(self, kind: Kind, opcode: u16, base: u16) -> Self {
        Self {
            implied: Some((kind, opcode as i32 - base as i32)),
            ..self
        }
    }
}

const ARG1: &[Field] = &[f(Kind::Arg, 1)];
const ARG1_SLOTS1: &[Field] = &[f(Kind::Arg, 1), f(Kind::Slots, 1)];
const ARGS1_LOCALS2_ALIGN1: &[Field] = &[f(Kind::Args, 1), f(Kind::Locals, 2), f(Kind::Align, 1)];
const ARGS1_METHOD1_RAW1: &[Field] = &[f(Kind::Args, 1), f(Kind::Method, 1), f(Kind::Raw, 1)];
const ARGS1_METHOD2_RAW1: &[Field] = &[f(Kind::Args, 1), f(Kind::Method, 2), f(Kind::Raw, 1)];
const ARGS1_METHOD4_RAW1: &[Field] = &[f(Kind::Args, 1), f(Kind::Method, 4), f(Kind::Raw, 1)];
const ARGS1_METHOD4_RAW2: &[Field] = &[f(Kind::Args, 1), f(Kind::Method, 4), f(Kind::Raw, 2)];
const BUILTIN2: &[Field] = &[f(Kind::Builtin, 2)];
const BYTES1: &[Field] = &[f(Kind::Bytes, 1)];
const BYTES2: &[Field] = &[f(Kind::Bytes, 2)];
const COUNT4_TABLE4: &[Field] = &[f(Kind::Count, 4), f(Kind::Table, 4)];
const DATA4: &[Field] = &[f(Kind::Data, 4)];
const DISP1: &[Field] = &[f(Kind::Disp, 1)];
const DISP4: &[Field] = &[f(Kind::Disp, 4)];
const FRAME1: &[Field] = &[f(Kind::Frame, 1)];
const FRAME1_ALIGN1: &[Field] = &[f(Kind::Frame, 1), f(Kind::Align, 1)];
const GLOBAL1: &[Field] = &[f(Kind::Global, 1)];
const IMM1: &[Field] = &[f(Kind::Imm, 1)];
const IMM1_IMM1: &[Field] = &[f(Kind::Imm, 1), f(Kind::Imm, 1)];
const IMM1_IMM1_IMM1_IMM1: &[Field] = &[
    f(Kind::Imm, 1),
    f(Kind::Imm, 1),
    f(Kind::Imm, 1),
    f(Kind::Imm, 1),
];
const IMM2_IMM2: &[Field] = &[f(Kind::Imm, 2), f(Kind::Imm, 2)];
const IMM4: &[Field] = &[f(Kind::Imm, 4)];
const IMM4_IMM4: &[Field] = &[f(Kind::Imm, 4), f(Kind::Imm, 4)];
const IMM4_IMM4_IMM4: &[Field] = &[f(Kind::Imm, 4), f(Kind::Imm, 4), f(Kind::Imm, 4)];
const IMM4_IMM4_IMM4_IMM4: &[Field] = &[
    f(Kind::Imm, 4),
    f(Kind::Imm, 4),
    f(Kind::Imm, 4),
    f(Kind::Imm, 4),
];
const IMM8: &[Field] = &[f(Kind::Imm, 8)];
const IMM8_IMM8: &[Field] = &[f(Kind::Imm, 8), f(Kind::Imm, 8)];
const IMPORT1_CODE2: &[Field] = &[f(Kind::Import, 1), f(Kind::Code, 2)];
const IMPORT1_CODE4: &[Field] = &[f(Kind::Import, 1), f(Kind::Code, 4)];
const IMPORT1_OFFSET1: &[Field] = &[f(Kind::Import, 1), f(Kind::Offset, 1)];
const IMPORT1_OFFSET1_BYTES1: &[Field] =
    &[f(Kind::Import, 1), f(Kind::Offset, 1), f(Kind::Bytes, 1)];
const IMPORT1_OFFSET4: &[Field] = &[f(Kind::Import, 1), f(Kind::Offset, 4)];
const IMPORT1_OFFSET4_BYTES2: &[Field] =
    &[f(Kind::Import, 1), f(Kind::Offset, 4), f(Kind::Bytes, 2)];
const LOCAL1: &[Field] = &[f(Kind::Local, 1)];
const LOCAL1_SLOTS1: &[Field] = &[f(Kind::Local, 1), f(Kind::Slots, 1)];
const LOCAL2: &[Field] = &[f(Kind::Local, 2)];
const LOCAL2_SLOTS2: &[Field] = &[f(Kind::Local, 2), f(Kind::Slots, 2)];
const NATIVE4_ARGS1_RESULTS1: &[Field] =
    &[f(Kind::Native, 4), f(Kind::Args, 1), f(Kind::Results, 1)];
const NONE: &[Field] = &[];
const OFFSET1: &[Field] = &[f(Kind::Offset, 1)];
const OFFSET1_BYTES1: &[Field] = &[f(Kind::Offset, 1), f(Kind::Bytes, 1)];
const OFFSET1_SLOTS1: &[Field] = &[f(Kind::Offset, 1), f(Kind::Slots, 1)];
const OFFSET2: &[Field] = &[f(Kind::Offset, 2)];
const OFFSET2_BYTES2: &[Field] = &[f(Kind::Offset, 2), f(Kind::Bytes, 2)];
const RAW1: &[Field] = &[f(Kind::Raw, 1)];
const RAW2: &[Field] = &[f(Kind::Raw, 2)];
const RAW4: &[Field] = &[f(Kind::Raw, 4)];
const RAW5: &[Field] = &[f(Kind::Raw, 5)];
const RAW6: &[Field] = &[f(Kind::Raw, 6)];
const RAW8_CODE4: &[Field] = &[f(Kind::Raw, 8), f(Kind::Code, 4)];
const SLOTS1: &[Field] = &[f(Kind::Slots, 1)];
const SLOTS1_OFFSET1: &[Field] = &[f(Kind::Slots, 1), f(Kind::Offset, 1)];
const SLOTS1_OFFSET1_BYTES1: &[Field] = &[f(Kind::Slots, 1), f(Kind::Offset, 1), f(Kind::Bytes, 1)];
const SLOTS1_SLOTS1: &[Field] = &[f(Kind::Slots, 1), f(Kind::Slots, 1)];
const SLOTS2_OFFSET2: &[Field] = &[f(Kind::Slots, 2), f(Kind::Offset, 2)];
const SLOTS2_OFFSET2_BYTES2: &[Field] = &[f(Kind::Slots, 2), f(Kind::Offset, 2), f(Kind::Bytes, 2)];
const SLOTS2_SLOTS1: &[Field] = &[f(Kind::Slots, 2), f(Kind::Slots, 1)];
const TYPE1: &[Field] = &[f(Kind::Type, 1)];
const TYPE1_RAW1: &[Field] = &[f(Kind::Type, 1), f(Kind::Raw, 1)];
const TYPE1_SLOTS1: &[Field] = &[f(Kind::Type, 1), f(Kind::Slots, 1)];
const TYPE4: &[Field] = &[f(Kind::Type, 4)];
const TYPE4_METHOD1: &[Field] = &[f(Kind::Type, 4), f(Kind::Method, 1)];
const TYPE4_METHOD2: &[Field] = &[f(Kind::Type, 4), f(Kind::Method, 2)];
const TYPE4_RAW1: &[Field] = &[f(Kind::Type, 4), f(Kind::Raw, 1)];
const TYPE4_SLOTS1: &[Field] = &[f(Kind::Type, 4), f(Kind::Slots, 1)];

/// The table: what each opcode the interpreter's handlers were read for does.
pub fn op(opcode: u16) -> Option<Op> {
    Some(match opcode {
        0x00 | 0x15D | 0x160 | 0x203 | 0x209 | 0x20D | 0x20E => row("nop", NONE, "--", 0),
        0x01 => row("load_arg", ARG1, "-- v", 0),
        0x02..=0x06 => row("load_arg", NONE, "-- v", 0).implied(Kind::Arg, opcode, 0x02),
        0x07 | 0x08 => row("load_args", ARG1_SLOTS1, "-- v..", 0),
        0x09 => row("store_arg", ARG1, "v --", 0),
        0x0A => row("store_args", ARG1_SLOTS1, "v.. --", 0),
        0x0B => row("arg_ref", ARG1, "-- ptr", 0),
        0x0C => row("load_local", LOCAL1, "-- v", 0),
        0x0D..=0x13 => row("load_local", NONE, "-- v", 0).implied(Kind::Local, opcode, 0x0D),
        0x14 | 0x15 => row("load_locals", LOCAL1_SLOTS1, "-- v..", 0),
        0x16 => row("local_ref", LOCAL1, "-- ptr", 0),
        0x17 => row("store_local", LOCAL1, "v --", 0),
        0x18..=0x1E => row("store_local", NONE, "v --", 0).implied(Kind::Local, opcode, 0x18),
        0x1F => row("store_locals", LOCAL1_SLOTS1, "v.. --", 0),
        0x20 => row("store_local_keep", LOCAL1, "v -- v", 0),
        0x21..=0x2A => row("push", NONE, "-- n", 0).implied(Kind::Imm, opcode, 0x21),
        0x2B => row("push", NONE, "-- n", 0).implied(Kind::Imm, opcode, 0x2C),
        0x2C => row("push_one_f32", NONE, "-- 1.0", 0),
        0x2D => row("push", IMM1, "-- n", 0),
        0x2E | 0x2F => row("push", IMM4, "-- n", 0),
        0x30 => row("push", IMM8, "-- n", 0),
        0x31 => row("push_u8x4", IMM1_IMM1_IMM1_IMM1, "-- a b c d", 0),
        0x32 => row("push_u32x4", IMM4_IMM4_IMM4_IMM4, "-- a b c d", 0),
        0x33 => row("push_bit", IMM1, "-- 1<<n", 0),
        0x34 => row("string_constant", DATA4, "-- ref", 0),
        0x35 => row("dup", NONE, "v -- v v", 0),
        0x36 => row("dup_n", SLOTS1, "v.. -- v.. v..", 0),
        0x37 => row("pick", SLOTS1, "-- v", 0),
        0x38 => row("load_i8", OFFSET1, "ref -- v", 0x08),
        0x39 => row("load_u8", OFFSET1, "ref -- v", 0x08),
        0x3A => row("load_i16", OFFSET1, "ref -- v", 0x08),
        0x3B => row("load_u16", OFFSET1, "ref -- v", 0x08),
        0x3C => row("load_i32", OFFSET1, "ref -- v", 0x08),
        0x3D => row("load_arg0_i32", OFFSET1, "-- v", 0x08),
        0x3E => row("load_u64", OFFSET1, "ref -- v", 0x08),
        0x3F => row("load_arg0_u64", OFFSET1, "-- v", 0x08),
        0x40 => row("load_bytes", OFFSET1_BYTES1, "ref -- v..", 0x08),
        0x41 => row("byte_offset", OFFSET1, "ref -- ptr", 0x08),
        0x42 => row("load_i8", OFFSET1, "ref -- v", 0x18),
        0x43 => row("load_u8", OFFSET1, "ref -- v", 0x18),
        0x44 => row("load_i16", OFFSET1, "ref -- v", 0x18),
        0x45 => row("load_u16", OFFSET1, "ref -- v", 0x18),
        0x46 => row("load_i32", OFFSET1, "ref -- v", 0x18),
        0x47 => row("load_u64", OFFSET1, "ref -- v", 0x18),
        0x48 => row("load_bytes", OFFSET1_BYTES1, "ref -- v..", 0x18),
        0x49 => row("extract_i8", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x4A => row("extract_u8", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x4B => row("extract_i16", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x4C => row("extract_u16", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x4D => row("extract_i32", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x4E => row("extract_u64", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x4F => row("extract_bytes", SLOTS1_OFFSET1_BYTES1, "v.. -- x..", 0),
        0x50 => row("load_i8", OFFSET1, "ptr -- v", 0),
        0x51 => row("load_u8", OFFSET1, "ptr -- v", 0),
        0x52 => row("load_i16", OFFSET1, "ptr -- v", 0),
        0x53 => row("load_u16", OFFSET1, "ptr -- v", 0),
        0x54 => row("load_i32", OFFSET1, "ptr -- v", 0),
        0x55 => row("load_arg0_i32", OFFSET1, "-- v", 0),
        0x56 => row("load_u64", OFFSET1, "ptr -- v", 0),
        0x57 => row("load_arg0_u64", OFFSET1, "-- v", 0),
        0x58 => row("load_bytes", OFFSET1_BYTES1, "ptr -- v..", 0),
        0x59 => row("byte_offset", OFFSET1, "ptr -- ptr", 0),
        0x5A => row("load_static_i8", IMPORT1_OFFSET1, "-- v", 0),
        0x5B => row("load_static_u8", IMPORT1_OFFSET1, "-- v", 0),
        0x5C => row("load_static_i16", IMPORT1_OFFSET1, "-- v", 0),
        0x5D => row("load_static_u16", IMPORT1_OFFSET1, "-- v", 0),
        0x5E => row("load_static_i32", IMPORT1_OFFSET1, "-- v", 0),
        0x5F => row("load_static_u64", IMPORT1_OFFSET1, "-- v", 0),
        0x60 => row("load_static_bytes", IMPORT1_OFFSET1_BYTES1, "-- v..", 0),
        0x61 => row("static_ref", IMPORT1_OFFSET1, "-- ptr", 0),
        0x62 => row("load_builtin_global", GLOBAL1, "-- v..", 0),
        0x63 => row("store_u8", OFFSET1, "ref v --", 0x08),
        0x64 => row("store_u16", OFFSET1, "ref v --", 0x08),
        0x65 => row("store_i32", OFFSET1, "ref v --", 0x08),
        0x66 => row("store_u64", OFFSET1, "ref v --", 0x08),
        0x67 => row("store_ref", OFFSET1, "ref v --", 0x08),
        0x68 => row("store_bytes", OFFSET1_BYTES1, "ref v.. --", 0x08),
        0x69 => row("store_i32_slots", OFFSET1_SLOTS1, "ref v.. --", 0x08),
        0x6A => row("store_bytes", OFFSET1_SLOTS1, "ref v.. --", 0x08),
        0x6B => row("store_u8", OFFSET1, "ref v --", 0x18),
        0x6C => row("store_u16", OFFSET1, "ref v --", 0x18),
        0x6D => row("store_i32", OFFSET1, "ref v --", 0x18),
        0x6E => row("store_u64", OFFSET1, "ref v --", 0x18),
        0x6F => row("store_ref", OFFSET1, "ref v --", 0x18),
        0x70 => row("store_bytes", OFFSET1_BYTES1, "ref v.. --", 0x18),
        0x71 => row("store_u8", OFFSET1, "ptr v --", 0),
        0x72 => row("store_u16", OFFSET1, "ptr v --", 0),
        0x73 => row("store_i32", OFFSET1, "ptr v --", 0),
        0x74 => row("store_u64", OFFSET1, "ptr v --", 0),
        0x75 => row("store_ref", OFFSET1, "ptr v --", 0),
        0x76 => row("store_bytes", OFFSET1_BYTES1, "ptr v.. --", 0),
        0x77 => row("store_i32_slots", OFFSET1_SLOTS1, "ptr v.. --", 0),
        0x78 => row("store_bytes", OFFSET1_SLOTS1, "ptr v.. --", 0),
        0x79 => row("store_static_u8", IMPORT1_OFFSET1, "v --", 0),
        0x7A => row("store_static_u16", IMPORT1_OFFSET1, "v --", 0),
        0x7B => row("store_static_i32", IMPORT1_OFFSET1, "v --", 0),
        0x7C => row("store_static_u64", IMPORT1_OFFSET1, "v --", 0),
        0x7D => row("store_static_bytes", IMPORT1_OFFSET1_BYTES1, "v.. --", 0),
        0x7E => row("function_ref", IMPORT1_CODE4, "-- fn", 0),
        0x7F => row("virtual_function_ref", TYPE4_METHOD1, "ref -- fn", 0),
        0x80 | 0x20C => row("drop", NONE, "v --", 0),
        0x81 => row("drop_n", SLOTS1, "v.. --", 0),
        0x82 => row("bury", SLOTS1, "a.. t -- t a..", 0),
        0x83 => row("enter", FRAME1, "--", 0),
        0x84 => row("enter", FRAME1_ALIGN1, "--", 0),
        0x85 => row("call", IMPORT1_CODE4, "args -- results", 0),
        0x86 => row("call", IMPORT1_CODE2, "args -- results", 0),
        0x87 => row("call_virtual", ARGS1_METHOD4_RAW1, "ref args -- results", 0),
        0x88 => row("call_virtual", ARGS1_METHOD2_RAW1, "ref args -- results", 0),
        0x89 => row("call_virtual", ARGS1_METHOD1_RAW1, "ref args -- results", 0),
        0x8A => row("call_builtin", BUILTIN2, "args -- results", 0),
        0x8B => row("native", NATIVE4_ARGS1_RESULTS1, "args -- results", 0),
        0x8C => row("invoke_delegate", IMM2_IMM2, "delegate args -- results", 0),
        0x8D => row("try_cast", TYPE4_RAW1, "ref -- ref", 0),
        0x8E => row("try_cast", TYPE1_RAW1, "ref -- ref", 0),
        0x8F => row("byte_offset_at", SLOTS1, "ref v.. -- ptr v..", 0),
        0x90 => row("return", NONE, "--", 0),
        0x91 => row("return_value", NONE, "v --", 0),
        0x92 => row("return_n", SLOTS1, "v.. --", 0),
        0x93 => row("new_array", TYPE1, "n -- arr", 0),
        0x94 | 0x187 => row("array_set_value", NONE, "arr i v --", 0),
        0x95 => row("box", TYPE1, "v -- ref", 0),
        0x96 => row("box", TYPE4, "v.. -- ref", 0),
        0x97 => row("new_object", TYPE4_SLOTS1, "args -- ref args", 0),
        0x98 => row("new_object", NONE, "-- ref", 0),
        0x99 => row("new_object", TYPE4, "-- ref", 0),
        0x9A => row("array_get_u64", NONE, "arr i -- v", 0),
        0x9B => row("array_element_address", NONE, "arr i -- ptr", 0),
        0x9C => row("jump", DISP4, "--", 0),
        0x9D => row("jump", DISP1, "--", 0),
        0x9E => row("switch", COUNT4_TABLE4, "i --", 0),
        0x9F => row("jz_i32", DISP4, "a --", 0),
        0xA0 => row("jz_i32", DISP1, "a --", 0),
        0xA1 => row("jz_i64", DISP4, "a --", 0),
        0xA2 => row("jnz_i32", DISP4, "a --", 0),
        0xA3 => row("jnz_i32", DISP1, "a --", 0),
        0xA4 => row("jnz_i64", DISP4, "a --", 0),
        0xA5 => row("jeq_i32", DISP4, "a b --", 0),
        0xA6 => row("jeq_i64", DISP4, "a b --", 0),
        0xA7 => row("jeq_f32", DISP4, "a b --", 0),
        0xA8 | 0xAC => row("jne_f64", DISP4, "a b --", 0),
        0xA9 => row("jne_i32", DISP4, "a b --", 0),
        0xAA => row("jne_i64", DISP4, "a b --", 0),
        0xAB => row("jne_f32", DISP4, "a b --", 0),
        0xAD => row("jgt_i32", DISP4, "a b --", 0),
        0xAE => row("jgt_u32", DISP4, "a b --", 0),
        0xAF => row("jgt_i64", DISP4, "a b --", 0),
        0xB0 => row("jgt_u64", DISP4, "a b --", 0),
        0xB1 => row("jgt_f32", DISP4, "a b --", 0),
        0xB2 => row("jgt_f64", DISP4, "a b --", 0),
        0xB3 => row("jge_i32", DISP4, "a b --", 0),
        0xB4 => row("jge_u32", DISP4, "a b --", 0),
        0xB5 => row("jge_i64", DISP4, "a b --", 0),
        0xB6 => row("jge_u64", DISP4, "a b --", 0),
        0xB7 => row("jge_f32", DISP4, "a b --", 0),
        0xB8 => row("jge_f64", DISP4, "a b --", 0),
        0xB9 => row("jlt_i32", DISP4, "a b --", 0),
        0xBA => row("jlt_u32", DISP4, "a b --", 0),
        0xBB => row("jlt_i64", DISP4, "a b --", 0),
        0xBC => row("jlt_u64", DISP4, "a b --", 0),
        0xBD => row("jlt_f32", DISP4, "a b --", 0),
        0xBE => row("jlt_f64", DISP4, "a b --", 0),
        0xBF => row("jle_i32", DISP4, "a b --", 0),
        0xC0 => row("jle_u32", DISP4, "a b --", 0),
        0xC1 => row("jle_i64", DISP4, "a b --", 0),
        0xC2 => row("jle_u64", DISP4, "a b --", 0),
        0xC3 => row("jle_f32", DISP4, "a b --", 0),
        0xC4 => row("jle_f64", DISP4, "a b --", 0),
        0xC5 => row("eqz_i32", NONE, "a -- a==0", 0),
        0xC6 => row("eqz_i64", NONE, "a -- a==0", 0),
        0xC7 => row("nez_i32", NONE, "a -- a!=0", 0),
        0xC8 => row("nez_i64", NONE, "a -- a!=0", 0),
        0xC9 => row("eq_i32", NONE, "a b -- bool", 0),
        0xCA => row("eq_i64", NONE, "a b -- bool", 0),
        0xCB => row("eq_f32", NONE, "a b -- bool", 0),
        0xCC => row("eq_f64", NONE, "a b -- bool", 0),
        0xCD => row("gt_i32", NONE, "a b -- bool", 0),
        0xCE => row("gt_u32", NONE, "a b -- bool", 0),
        0xCF => row("gt_i64", NONE, "a b -- bool", 0),
        0xD0 => row("gt_u64", NONE, "a b -- bool", 0),
        0xD1 => row("gt_f32", NONE, "a b -- bool", 0),
        0xD2 => row("gt_f64", NONE, "a b -- bool", 0),
        0xD3 => row("lt_i32", NONE, "a b -- bool", 0),
        0xD4 => row("lt_u32", NONE, "a b -- bool", 0),
        0xD5 => row("lt_i64", NONE, "a b -- bool", 0),
        0xD6 => row("lt_u64", NONE, "a b -- bool", 0),
        0xD7 => row("lt_f32", NONE, "a b -- bool", 0),
        0xD8 => row("lt_f64", NONE, "a b -- bool", 0),
        0xD9 => row("ne_i32", NONE, "a b -- bool", 0),
        0xDA => row("ne_i64", NONE, "a b -- bool", 0),
        0xDB => row("ne_f32", NONE, "a b -- bool", 0),
        0xDC => row("ne_f64", NONE, "a b -- bool", 0),
        0xDD => row("ge_i32", NONE, "a b -- bool", 0),
        0xDE => row("ge_u32", NONE, "a b -- bool", 0),
        0xDF => row("ge_i64", NONE, "a b -- bool", 0),
        0xE0 => row("ge_u64", NONE, "a b -- bool", 0),
        0xE1 => row("ge_f32", NONE, "a b -- bool", 0),
        0xE2 => row("ge_f64", NONE, "a b -- bool", 0),
        0xE3 => row("le_i32", NONE, "a b -- bool", 0),
        0xE4 => row("le_u32", NONE, "a b -- bool", 0),
        0xE5 => row("le_i64", NONE, "a b -- bool", 0),
        0xE6 => row("le_u64", NONE, "a b -- bool", 0),
        0xE7 => row("le_f32", NONE, "a b -- bool", 0),
        0xE8 => row("le_f64", NONE, "a b -- bool", 0),
        0xE9 => row("select", NONE, "c x y -- c?y:x", 0),
        0xEA => row("select", IMM1_IMM1, "c -- c?b:a", 0),
        0xEB => row("select", IMM4_IMM4, "c -- c?b:a", 0),
        0xEC => row("select", IMM8_IMM8, "c -- c?b:a", 0),
        0xED => row("load_i8", NONE, "ptr -- v", 0),
        0xEE => row("load_u8", NONE, "ptr -- v", 0),
        0xEF => row("load_i16", NONE, "ptr -- v", 0),
        0xF0 => row("load_u16", NONE, "ptr -- v", 0),
        0xF1 => row("load_i32", NONE, "ptr -- v", 0),
        0xF2 => row("load_u64", NONE, "ptr -- v", 0),
        0xF3 => row("load_bytes", BYTES2, "ptr -- v..", 0),
        0xF4 => row("store_u8", NONE, "ptr v --", 0),
        0xF5 => row("store_u16", NONE, "ptr v --", 0),
        0xF6 => row("store_i32", NONE, "ptr v --", 0),
        0xF7 => row("store_u64", NONE, "ptr v --", 0),
        0xF8 => row("store_ref", NONE, "ptr v --", 0),
        0xF9 => row("load_u64_at", SLOTS1, "ptr v.. -- w v..", 0),
        0xFA => row("add_i32", NONE, "a b -- c", 0),
        0xFB => row("add_i64", NONE, "a b -- c", 0),
        0xFC => row("add_f32", NONE, "a b -- c", 0),
        0xFD => row("add_f64", NONE, "a b -- c", 0),
        0xFE => row("sub_i32", NONE, "a b -- c", 0),
        0xFF => row("sub_i64", NONE, "a b -- c", 0),
        0x100 => row("sub_f32", NONE, "a b -- c", 0),
        0x101 => row("sub_f64", NONE, "a b -- c", 0),
        0x102 => row("inc_i32", NONE, "a -- a+1", 0),
        0x103 => row("inc_arg", ARG1, "--", 0),
        0x104 => row("inc_local", LOCAL1, "--", 0),
        0x105 => row("inc_at_i32", OFFSET1, "ref --", 0x08),
        0x106 => row("inc_at_i32", OFFSET1, "ptr --", 0),
        0x107 => row("post_inc_arg", ARG1, "-- old", 0),
        0x108 => row("post_inc_local", LOCAL1, "-- old", 0),
        0x109 => row("dec_i32", NONE, "a -- a-1", 0),
        0x10A => row("dec_arg", ARG1, "--", 0),
        0x10B => row("dec_local", LOCAL1, "--", 0),
        0x10C => row("dec_at_i32", OFFSET1, "ref --", 0x08),
        0x10D => row("dec_at_i32", OFFSET1, "ptr --", 0),
        0x10E => row("post_dec_arg", ARG1, "-- old", 0),
        0x10F => row("post_dec_local", LOCAL1, "-- old", 0),
        0x110 => row("mul_i32", NONE, "a b -- c", 0),
        0x111 => row("mul_i64", NONE, "a b -- c", 0),
        0x112 => row("mul_f32", NONE, "a b -- c", 0),
        0x113 => row("mul_f64", NONE, "a b -- c", 0),
        0x114 => row("div_i32", NONE, "a b -- c", 0),
        0x115 => row("div_u32", NONE, "a b -- c", 0),
        0x116 => row("div_i64", NONE, "a b -- c", 0),
        0x117 => row("div_u64", NONE, "a b -- c", 0),
        0x118 => row("div_f32", NONE, "a b -- c", 0),
        0x119 => row("div_f64", NONE, "a b -- c", 0),
        0x11A => row("rem_i32", NONE, "a b -- c", 0),
        0x11B => row("rem_u32", NONE, "a b -- c", 0),
        0x11C => row("rem_i64", NONE, "a b -- c", 0),
        0x11D => row("rem_u64", NONE, "a b -- c", 0),
        0x11E => row("rem_f32", NONE, "a b -- c", 0),
        0x11F => row("rem_f64", NONE, "a b -- c", 0),
        0x120 => row("and_i32", NONE, "a b -- c", 0),
        0x121 => row("and_i64", NONE, "a b -- c", 0),
        0x122 => row("or_i32", NONE, "a b -- c", 0),
        0x123 => row("or_i64", NONE, "a b -- c", 0),
        0x124 => row("xor_i32", NONE, "a b -- c", 0),
        0x125 => row("xor_i64", NONE, "a b -- c", 0),
        0x126 => row("not_i32", NONE, "a -- ~a", 0),
        0x127 => row("not_i64", NONE, "a -- ~a", 0),
        0x128 => row("shl_i32", NONE, "a b -- c", 0),
        0x129 => row("shl_i64", NONE, "a b -- c", 0),
        0x12A => row("shr_i32", NONE, "a b -- c", 0),
        0x12B => row("shr_u32", NONE, "a b -- c", 0),
        0x12C => row("shr_i64", NONE, "a b -- c", 0),
        0x12D => row("shr_u64", NONE, "a b -- c", 0),
        0x12E => row("neg_i32", NONE, "a -- b", 0),
        0x12F => row("neg_i64", NONE, "a -- b", 0),
        0x130 => row("neg_f32", NONE, "a -- b", 0),
        0x131 => row("neg_f64", NONE, "a -- b", 0),
        0x132 => row("abs_i32", NONE, "a -- b", 0),
        0x133 => row("abs_i64", NONE, "a -- b", 0),
        0x134 => row("abs_f32", NONE, "a -- b", 0),
        0x135 => row("sqrt_f32", NONE, "a -- b", 0),
        0x136 => row("square_f32", NONE, "a -- b", 0),
        0x137 => row("pow_i32", NONE, "a b -- c", 0),
        0x138 => row("pow_i64", NONE, "a b -- c", 0),
        0x139 => row("pow_f32", NONE, "a b -- c", 0),
        0x13A => row("sin_f32", NONE, "a -- b", 0),
        0x13B => row("asin_f32", NONE, "a -- b", 0),
        0x13C => row("cos_f32", NONE, "a -- b", 0),
        0x13D => row("acos_f32", NONE, "a -- b", 0),
        0x13E => row("tan_f32", NONE, "a -- b", 0),
        0x13F => row("atan_f32", NONE, "a -- b", 0),
        0x140 => row("atan2_f32", NONE, "a b -- c", 0),
        0x141 => row("max_i32", NONE, "a b -- c", 0),
        0x142 => row("max_u32", NONE, "a b -- c", 0),
        0x143 => row("max_i64", NONE, "a b -- c", 0),
        0x144 => row("max_u64", NONE, "a b -- c", 0),
        0x145 => row("max_f32", NONE, "a b -- c", 0),
        0x146 => row("min_i32", NONE, "a b -- c", 0),
        0x147 => row("min_u32", NONE, "a b -- c", 0),
        0x148 => row("min_i64", NONE, "a b -- c", 0),
        0x149 => row("min_u64", NONE, "a b -- c", 0),
        0x14A => row("min_f32", NONE, "a b -- c", 0),
        0x14B => row("round_f32", NONE, "a -- b", 0),
        0x14C => row("floor_f32", NONE, "a -- b", 0),
        0x14D => row("ceil_f32", NONE, "a -- b", 0),
        0x14E => row("clamp_i32", NONE, "x lo hi -- y", 0),
        0x14F => row("clamp_u32", NONE, "x lo hi -- y", 0),
        0x150 => row("clamp_f32", NONE, "x lo hi -- y", 0),
        0x151 => row("lerp_f32", NONE, "a b t -- c", 0),
        0x152 => row("sign_i32", NONE, "a -- b", 0),
        0x153 => row("sign_i64", NONE, "a -- b", 0),
        0x154 => row("sign_f32", NONE, "a -- b", 0),
        0x155 | 0x156 => row("i8_to_i32", NONE, "a -- b", 0),
        0x157 | 0x158 => row("u8_to_i32", NONE, "a -- b", 0),
        0x159 | 0x15A => row("i16_to_i32", NONE, "a -- b", 0),
        0x15B | 0x15C => row("u16_to_i32", NONE, "a -- b", 0),
        0x15E => row("f32_to_i32", NONE, "a -- b", 0),
        0x15F => row("f64_to_i32", NONE, "a -- b", 0),
        0x161 => row("i32_to_i64", NONE, "a -- b", 0),
        0x162 => row("f32_to_i64", NONE, "a -- b", 0),
        0x163 => row("f64_to_i64", NONE, "a -- b", 0),
        0x164 => row("u32_to_u64", NONE, "a -- b", 0),
        0x165 => row("i32_to_f32", NONE, "a -- b", 0),
        0x166 => row("u32_to_f32", NONE, "a -- b", 0),
        0x167 => row("i64_to_f32", NONE, "a -- b", 0),
        0x168 => row("u64_to_f32", NONE, "a -- b", 0),
        0x169 => row("f64_to_f32", NONE, "a -- b", 0),
        0x16A => row("i32_to_f64", NONE, "a -- b", 0),
        0x16B => row("u32_to_f64", NONE, "a -- b", 0),
        0x16C => row("i64_to_f64", NONE, "a -- b", 0),
        0x16D => row("u64_to_f64", NONE, "a -- b", 0),
        0x16E => row("f32_to_f64", NONE, "a -- b", 0),
        0x16F => row("new_object_builtin", BUILTIN2, "args -- ref", 0),
        0x170 => row("zero_slots", SLOTS1_SLOTS1, "v.. -- 0.. v..", 0),
        0x171 => row("new_value_builtin", BUILTIN2, "args -- v..", 0),
        0x172 => row("load_bytes", TYPE1, "ref -- v..", 0),
        0x173 => row("unbox", TYPE4, "ref -- v..", 0),
        0x174 => Op {
            implied: Some((Kind::Offset, 8)),
            ..row("byte_offset", NONE, "ref -- ptr", 0)
        },
        0x175 => row("typed_intrinsic", TYPE1_SLOTS1, "v.. -- ref", 0),
        0x176 => row("box_at", TYPE4_SLOTS1, "ptr v.. -- ref v..", 0),
        0x177 => row("store_bytes", TYPE4, "ptr v.. --", 0),
        0x178 => row("store_bytes", TYPE1, "ptr v.. --", 0),
        0x179 => row("zero_bytes", BYTES2, "ptr --", 0),
        0x17A => row("bytes_equal", BYTES2, "p q -- bool", 0),
        0x17B => row("new_array", TYPE4, "n -- arr", 0),
        0x17C => row("array_init_from_constant", NONE, "arr i --", 0),
        0x17D => row("array_length", NONE, "arr -- n", 0),
        0x17E => row("array_get_i8", NONE, "arr i -- v", 0),
        0x17F => row("array_get_u8", NONE, "arr i -- v", 0),
        0x180 => row("array_get_i16", NONE, "arr i -- v", 0),
        0x181 => row("array_get_u16", NONE, "arr i -- v", 0),
        0x182 => row("array_get_i32", NONE, "arr i -- v", 0),
        0x183 => row("array_get_value", NONE, "arr i -- v..", 0),
        0x184 => row("array_set_u8", NONE, "arr i v --", 0),
        0x185 => row("array_set_u16", NONE, "arr i v --", 0),
        0x186 => row("array_set_u32", NONE, "arr i v --", 0),
        0x188 => row("array_set_value", SLOTS1, "arr i v.. --", 0),
        0x189 => row("load_local", LOCAL2, "-- v", 0),
        0x18A => row("load_locals", LOCAL2_SLOTS2, "-- v..", 0),
        0x18B => row("local_ref", LOCAL2, "-- ptr", 0),
        0x18C => row("store_local", LOCAL2, "v --", 0),
        0x18D => row("store_locals", LOCAL2_SLOTS2, "v.. --", 0),
        0x18E => row("enter", ARGS1_LOCALS2_ALIGN1, "--", 0),
        0x18F => row("call_virtual", ARGS1_METHOD4_RAW2, "ref args -- results", 0),
        0x190 => row("virtual_function_ref", TYPE4_METHOD2, "ref -- fn", 0),
        0x192 => row("zero_slots", SLOTS2_SLOTS1, "v.. -- 0.. v..", 0),
        0x193 => row("load_i8", OFFSET2, "ref -- v", 0x08),
        0x194 => row("load_u8", OFFSET2, "ref -- v", 0x08),
        0x195 => row("load_i16", OFFSET2, "ref -- v", 0x08),
        0x196 => row("load_u16", OFFSET2, "ref -- v", 0x08),
        0x197 => row("load_i32", OFFSET2, "ref -- v", 0x08),
        0x198 => row("load_u64", OFFSET2, "ref -- v", 0x08),
        0x199 => row("load_bytes", OFFSET2_BYTES2, "ref -- v..", 0x08),
        0x19A => row("byte_offset", OFFSET2, "ref -- ptr", 0x08),
        0x19B => row("extract_i8", SLOTS2_OFFSET2, "v.. -- x", 0),
        0x19C => row("extract_u8", SLOTS2_OFFSET2, "v.. -- x", 0),
        0x19D => row("extract_i16", SLOTS2_OFFSET2, "v.. -- x", 0),
        0x19E => row("extract_u16", SLOTS2_OFFSET2, "v.. -- x", 0),
        0x19F => row("extract_i32", SLOTS2_OFFSET2, "v.. -- x", 0),
        0x1A0 => row("extract_u64", SLOTS2_OFFSET2, "v.. -- x", 0),
        0x1A1 => row("extract_bytes", SLOTS2_OFFSET2_BYTES2, "v.. -- x..", 0),
        0x1A2 => row("load_i8", OFFSET2, "ptr -- v", 0),
        0x1A3 => row("load_u8", OFFSET2, "ptr -- v", 0),
        0x1A4 => row("load_i16", OFFSET2, "ptr -- v", 0),
        0x1A5 => row("load_u16", OFFSET2, "ptr -- v", 0),
        0x1A6 => row("load_i32", OFFSET2, "ptr -- v", 0),
        0x1A7 => row("load_u64", OFFSET2, "ptr -- v", 0),
        0x1A8 => row("load_bytes", OFFSET2_BYTES2, "ptr -- v..", 0),
        0x1A9 => row("byte_offset", OFFSET2, "ptr -- ptr", 0),
        0x1AA => row("load_static_i8", IMPORT1_OFFSET4, "-- v", 0),
        0x1AB => row("load_static_u8", IMPORT1_OFFSET4, "-- v", 0),
        0x1AC => row("load_static_i16", IMPORT1_OFFSET4, "-- v", 0),
        0x1AD => row("load_static_u16", IMPORT1_OFFSET4, "-- v", 0),
        0x1AE => row("load_static_i32", IMPORT1_OFFSET4, "-- v", 0),
        0x1AF => row("load_static_u64", IMPORT1_OFFSET4, "-- v", 0),
        0x1B0 => row("load_static_bytes", IMPORT1_OFFSET4_BYTES2, "-- v..", 0),
        0x1B1 => row("static_ref", IMPORT1_OFFSET4, "-- ptr", 0),
        0x1B2 => row("store_u8", OFFSET2, "ref v --", 0x08),
        0x1B3 => row("store_u16", OFFSET2, "ref v --", 0x08),
        0x1B4 => row("store_i32", OFFSET2, "ref v --", 0x08),
        0x1B5 => row("store_u64", OFFSET2, "ref v --", 0x08),
        0x1B6 => row("store_ref", OFFSET2, "ref v --", 0x08),
        0x1B7 => row("store_bytes", OFFSET2_BYTES2, "ref v.. --", 0x08),
        0x1B8 => row("store_u8", OFFSET2, "ptr v --", 0),
        0x1B9 => row("store_u16", OFFSET2, "ptr v --", 0),
        0x1BA => row("store_i32", OFFSET2, "ptr v --", 0),
        0x1BB => row("store_u64", OFFSET2, "ptr v --", 0),
        0x1BC => row("store_ref", OFFSET2, "ptr v --", 0),
        0x1BD => row("store_bytes", OFFSET2_BYTES2, "ptr v.. --", 0),
        0x1BE => row("store_static_u8", IMPORT1_OFFSET4, "v --", 0),
        0x1BF => row("store_static_u16", IMPORT1_OFFSET4, "v --", 0),
        0x1C0 => row("store_static_i32", IMPORT1_OFFSET4, "v --", 0),
        0x1C1 => row("store_static_u64", IMPORT1_OFFSET4, "v --", 0),
        0x1C2 => row("store_static_bytes", IMPORT1_OFFSET4_BYTES2, "v.. --", 0),
        0x1C3 => row("load_global_i8", IMPORT1_OFFSET4, "-- v", 0),
        0x1C4 => row("load_global_u8", IMPORT1_OFFSET4, "-- v", 0),
        0x1C5 => row("load_global_i16", IMPORT1_OFFSET4, "-- v", 0),
        0x1C6 => row("load_global_u16", IMPORT1_OFFSET4, "-- v", 0),
        0x1C7 => row("load_global_i32", IMPORT1_OFFSET4, "-- v", 0),
        0x1C8 => row("load_global_u64", IMPORT1_OFFSET4, "-- v", 0),
        0x1C9 => row("load_global_bytes", IMPORT1_OFFSET4_BYTES2, "-- v..", 0),
        0x1CA => row("global_ref", IMPORT1_OFFSET4, "-- ptr", 0),
        0x1CB => row("store_global_bytes", IMPORT1_OFFSET4_BYTES2, "v.. --", 0),
        0x1CD => row(
            "replication_barrier",
            OFFSET2_BYTES2,
            "ref v.. -- ref v..",
            0x08,
        ),
        0x1CE => row(
            "replication_barrier",
            OFFSET2_BYTES2,
            "ptr v.. -- ptr v..",
            0,
        ),
        0x1CF => row("replication_barrier", OFFSET2_BYTES2, "ref -- ref", 0x08),
        0x1D0 => row("replication_barrier", OFFSET2_BYTES2, "ptr -- ptr", 0),
        0x1D6 => row("intrinsic", RAW1, "n -- ptr", 0),
        0x1D7 => row("element_address", BYTES1, "ref i -- ptr", 0),
        0x1D8 => row("stack_address", SLOTS1, "v.. -- v.. ptr", 0),
        0x1D9 => row("packed_vector_add", NONE, "a b -- c", 0),
        0x1DA => row("packed_vector_subtract", NONE, "a b -- c", 0),
        0x1DB => row("packed_vector_multiply", NONE, "a b -- c", 0),
        0x1DC => row("packed_vector_divide", NONE, "a b -- c", 0),
        0x1DD => row("packed_vector_min", NONE, "a b -- c", 0),
        0x1DE => row("packed_vector_max", NONE, "a b -- c", 0),
        0x1DF => row("packed_vector_clamp", NONE, "x lo hi -- y", 0),
        0x1E0 => row("packed_vector_lerp", NONE, "a b t -- c", 0),
        0x1E1 => row("packed_vector_scale", NONE, "s v -- w", 0),
        0x1E2 => row("packed_vector_fma", NONE, "c a b -- c+a*b", 0),
        0x1E3 => row("packed_vector_dot4", NONE, "a b -- dot", 0),
        0x1E4 => row("packed_vector_normalize4", NONE, "v -- n", 0),
        0x1E5 => row("packed_vector_splat", NONE, "s -- v", 0),
        0x1E6 => row("packed_vector_dot3", NONE, "a b -- dot", 0),
        0x1E7 => row("packed_vector_normalize3", NONE, "v -- n", 0),
        0x1E8 => row("packed_vector_from_xyz", NONE, "x y z -- v", 0),
        0x1E9 => row("vector3", IMM4_IMM4_IMM4, "-- v", 0),
        0x1EA => row("load_arg_vec", ARG1, "-- v", 0),
        0x1EB => row("store_arg_vec", ARG1, "v --", 0),
        0x1EC => row("load_local_vec", LOCAL1, "-- v", 0),
        0x1ED => row("store_local_vec", LOCAL1, "v --", 0),
        0x1EE => row("store_local_vec_keep", LOCAL1, "v -- v", 0),
        0x1EF => row("load_vec", OFFSET1, "ref -- v", 0x08),
        0x1F0 => row("store_vec", OFFSET1, "ref v --", 0x08),
        0x1F1 => row("load_vec", OFFSET1, "ref -- v", 0x18),
        0x1F2 => row("store_vec", OFFSET1, "ref v --", 0x18),
        0x1F3 => row("load_vec", OFFSET1, "ptr -- v", 0),
        0x1F4 => row("store_vec", OFFSET1, "ptr v --", 0),
        0x1F5 => row("load_static_vec", IMPORT1_OFFSET4, "-- v", 0),
        0x1F6 => row("store_static_vec", IMPORT1_OFFSET4, "v --", 0),
        0x1F7 => row("extract_vec", SLOTS1_OFFSET1, "v.. -- x", 0),
        0x1F8 => row("load_global_vec", IMPORT1_OFFSET4, "-- v", 0),
        0x1F9 => row("tail_call", RAW8_CODE4, "--", 0),
        0x1FB | 0x1FC | 0x206 | 0x207 | 0x211 => row("nop", RAW5, "--", 0),
        0x1FD | 0x1FE | 0x205 => row("nop", RAW6, "--", 0),
        0x1FF..=0x202 => row("nop", RAW2, "--", 0),
        0x204 | 0x208 | 0x20A | 0x20F => row("nop", RAW4, "--", 0),
        0x20B => row("drop", RAW4, "v --", 0),
        _ => return None,
    })
}

/// The mnemonic of an opcode the table names.
pub fn mnemonic(opcode: u16) -> Option<&'static str> {
    op(opcode).map(|o| o.mnemonic)
}

/// The operand fields of an instruction, each with its little-endian value and its bytes.
fn values(op: Op, ins: &Instruction) -> impl Iterator<Item = (Field, u64, &[u8])> + '_ {
    let mut at = 0;
    op.fields.iter().map(move |field| {
        let width = usize::from(field.width);
        let bytes = ins.operands.get(at..at + width).unwrap_or(&[]);
        at += width;
        let value = (bytes.iter().rev()).fold(0u64, |v, b| (v << 8) | u64::from(*b));
        (*field, value, bytes)
    })
}

/// The code offset a jump or branch goes to: its signed displacement past the opcode word.
pub fn target(ins: &Instruction) -> Option<usize> {
    let op = op(ins.opcode)?;
    let (field, value, _) = values(op, ins).find(|(field, _, _)| field.kind == Kind::Disp)?;
    let disp = match field.width {
        1 => i64::from(value as u8 as i8),
        4 => i64::from(value as u32 as i32),
        _ => return None,
    };
    usize::try_from(i64::try_from(ins.offset).ok()? + 2 + disp).ok()
}

/// What a conditional branch tests, from its mnemonic: `a` is the deeper operand, `b` the top.
fn condition(mnemonic: &str) -> Option<&'static str> {
    Some(match mnemonic.split('_').next()? {
        "jz" => "a == 0",
        "jnz" => "a != 0",
        "jeq" => "a == b",
        "jne" => "a != b",
        "jgt" => "a > b",
        "jge" => "a >= b",
        "jlt" => "a < b",
        "jle" => "a <= b",
        _ => return None,
    })
}

/// The local slots every frame has before its enter adds its own.
const BASE_LOCALS: u64 = 4;

/// An instruction's operands as text by the table's fields, or its decoded bytes where it has none; a native by the name `natives` gives its address.
fn operands(ins: &Instruction, natives: Option<&NativeAddresses>) -> String {
    let Some(op) = op(ins.opcode) else {
        let n = usize::from(ins.len)
            .saturating_sub(2)
            .min(ins.operands.len());
        return hex_bytes(&ins.operands[..n]);
    };
    let mut parts = Vec::new();
    match op.implied {
        Some((Kind::Arg, n)) => parts.push(format!("arg{n}")),
        Some((Kind::Local, n)) => parts.push(format!("local{n}")),
        Some((Kind::Offset, n)) => parts.push(format!("+{n:#X}")),
        Some((_, n)) => parts.push(n.to_string()),
        None => {}
    }
    for (field, v, bytes) in values(op, ins) {
        parts.push(match field.kind {
            Kind::Arg => format!("arg{v}"),
            Kind::Local => format!("local{v}"),
            Kind::Slots => format!("slots {v}"),
            Kind::Bytes => format!("bytes {v}"),
            Kind::Offset => format!("+{:#X}", v + u64::from(op.header)),
            Kind::Imm => format!("{v:#0w$X}", w = 2 + 2 * bytes.len()),
            Kind::Import => format!("import {v}"),
            Kind::Code => format!("-> func {v:#08X}"),
            Kind::Disp => {
                let to = target(ins).map_or_else(|| "?".to_owned(), |t| format!("{t:#08X}"));
                match condition(op.mnemonic) {
                    Some(test) => format!("if {test} -> {to}"),
                    None => format!("-> {to}"),
                }
            }
            Kind::Frame => format!("args {} locals {}", v >> 4, BASE_LOCALS + (v & 0xF)),
            Kind::Args => format!("args {v}"),
            Kind::Locals => format!("locals {}", BASE_LOCALS + v),
            Kind::Results => format!("results {v}"),
            Kind::Align => format!("align {}", 1u64.checked_shl(v as u32).unwrap_or(0)),
            Kind::Method => format!("method {v}"),
            Kind::Builtin => format!("type {} method {}", v >> 8, v & 0xFF),
            Kind::Native => match natives.and_then(|t| t.name_of(v as u32)) {
                Some(name) => name.to_owned(),
                None => format!("{v:#010X}"),
            },
            Kind::Type => format!("type {v:#X}"),
            Kind::Count => format!("cases {v}"),
            Kind::Table => format!("table {v:#X}"),
            Kind::Data => format!("data {v:#X}"),
            Kind::Global => format!("global {v}"),
            Kind::Raw => hex_bytes(bytes),
        });
    }
    parts.join(" ")
}

fn hex_bytes(bytes: &[u8]) -> String {
    (bytes.iter())
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
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
            operands(&ins, natives)
        );
        let _ = writeln!(body, "{}", line.trim_end());
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
