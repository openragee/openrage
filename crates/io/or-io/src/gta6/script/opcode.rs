//! The instruction set as the lifter takes it: each instruction of a compiled program is one or
//! several steps on a stack of cells, stated in terms of the intermediate form.
//!
//! A step says what it pops, what it builds and what it pushes. Which bytes spell which steps
//! is the decoder's table, kept in this file too; an instruction that does several things is
//! several steps, so the lifter knows no opcode.

use super::ir::{BinOp, Callee, Expr, Kind, Scalar, StoreKind, UnOp, Var, VmType};

#[path = "opcode_table.rs"]
mod table;

/// One step of an instruction on the evaluation stack; every count is in cells.
// ⚠ Operands are popped in the order each step states. An instruction that takes them in
// another order is decoded with a `Swap`, a `Sink`, a `Raise` or a `Copy` in front.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Pushes the `cells` cells of an expression that reads no stack cell: a constant, a variable, an address, a new object, zero cells.
    Push { e: Expr, cells: u32 },
    /// Pops one cell into a variable.
    Set(Var),
    /// Pops `cells` cells and drops them.
    Drop { cells: u32 },
    /// Pushes a copy of the `cells` cells that end `depth` cells below the top; `depth` 0 copies the top ones.
    Copy { depth: u32, cells: u32 },
    /// Exchanges the two top cells.
    Swap,
    /// Moves the top cell down to be cell `depth` from the top, the top being 1; the cells it passes each rise by one.
    Sink { depth: u32 },
    /// Moves cell `depth` from the top, the top being 1, up to the top; the cells it passes each sink by one.
    Raise { depth: u32 },
    /// Pushes the address of the value that the top `cells` cells hold, and leaves them.
    AddrOfTop { cells: u32 },
    /// Pops `x`, pushes `as_T(x)`.
    Cast(Scalar),
    /// Pops `x`, pushes whether any bit of it is set.
    Truth,
    /// Pops `x`, pushes `op x`.
    Unary(UnOp),
    /// Pops `r` and then `l`, pushes `l op r` worked at `at`.
    Binary { op: BinOp, at: Kind },
    /// Pops the value for no, the value for yes and the test, pushes the one the test chooses.
    Select,
    /// Pops an address, pushes `load_T(address, offset)`.
    Load { ty: Scalar, offset: u32 },
    /// Pops an address, pushes the cells of `load_bytes(address, offset, bytes)`.
    LoadBytes { offset: u32, bytes: u32 },
    /// Pops `cells` cells, pushes the cells of `extract_bytes(cells, offset, bytes)`.
    ExtractBytes { cells: u32, offset: u32, bytes: u32 },
    /// Pops a count of bytes and then an address, pushes `byte_offset(address, count)`.
    ByteOffset,
    /// Pops `z`, `y` and `x`, pushes the two cells of `vector3(x, y, z)`.
    Vector3,
    /// Pops a length, pushes a new array of that many elements.
    NewArray { ty: VmType, wrapper: String },
    /// Pops a receiver, pushes a reference to the function its virtual slot resolves to.
    VirtualFunctionRef { ty: VmType, slot: u32 },
    /// Pops `args` cells, the first argument deepest, calls, and pushes the `returns` cells of the result.
    Call {
        callee: Callee,
        args: u32,
        returns: u32,
    },
    /// A call some cells of which no table at hand states: `None` is a count the lifter has to find.
    // ⚠ The stack after it is unknown until both counts are; nothing may be lifted past it before.
    Unsized {
        callee: Callee,
        args: Option<u32>,
        returns: Option<u32>,
    },
    /// Pops a value and then an address: `store_T(address, offset, value)`.
    Store { kind: StoreKind, offset: u32 },
    /// Pops `cells` cells and then an address: `store_bytes(address, offset, bytes, cells)`.
    StoreBytes { offset: u32, bytes: u32, cells: u32 },
    /// Pops `cells` cells and then an address: `store_f32_slots(address, offset, cells)`.
    StoreF32Slots { offset: u32, cells: u32 },
    /// Pops an address: `zero_bytes(address, bytes)`.
    ZeroBytes { bytes: u32 },
    /// Pops an address: `increment_i32(address, offset)`, or `decrement_i32` when `down`.
    Step { offset: u32, down: bool },
    /// Goes on at the instruction at `to`.
    Jump { to: u32 },
    /// Pops a cell and goes on at `to` where a bit of it is set, at the next instruction otherwise.
    Branch { to: u32 },
    /// Pops a cell read as an `int` and goes on at the case it equals, at `default` when it equals none.
    Switch {
        cases: Vec<(i64, u32)>,
        default: u32,
    },
    /// Pops `cells` cells and returns them.
    Return { cells: u32 },
    /// Nothing follows: a fault is raised, or the bytecode ends.
    Unreachable,
    /// Changes nothing the lifter keeps: a frame set up, a barrier with no statement.
    Nop,
}

impl Op {
    /// The cells the step pops and the cells it pushes.
    // ⚠ A count an `Unsized` call does not state is given as 0: a sum over steps holds only where none is unsized.
    pub fn stack(&self) -> (u32, u32) {
        match self {
            Op::Push { cells, .. } | Op::Copy { cells, .. } => (0, *cells),
            Op::Set(_) | Op::ZeroBytes { .. } | Op::Step { .. } => (1, 0),
            Op::Branch { .. } | Op::Switch { .. } => (1, 0),
            Op::Drop { cells } | Op::Return { cells } => (*cells, 0),
            Op::Swap => (2, 2),
            Op::Sink { depth } | Op::Raise { depth } => (*depth, *depth),
            Op::AddrOfTop { .. } => (0, 1),
            Op::Cast(_) | Op::Truth | Op::Unary(_) | Op::Load { .. } => (1, 1),
            Op::NewArray { .. } | Op::VirtualFunctionRef { .. } => (1, 1),
            Op::Binary { .. } | Op::ByteOffset => (2, 1),
            Op::Select => (3, 1),
            Op::LoadBytes { bytes, .. } => (1, bytes.div_ceil(8)),
            Op::ExtractBytes { cells, bytes, .. } => (*cells, bytes.div_ceil(8)),
            Op::Vector3 => (3, 2),
            Op::Call { args, returns, .. } => (*args, *returns),
            Op::Unsized { args, returns, .. } => (args.unwrap_or(0), returns.unwrap_or(0)),
            Op::Store { .. } => (2, 0),
            Op::StoreBytes { cells, .. } | Op::StoreF32Slots { cells, .. } => {
                (cells.saturating_add(1), 0)
            }
            Op::Jump { .. } | Op::Unreachable | Op::Nop => (0, 0),
        }
    }

    /// Whether control does not go on to the next instruction after this step.
    pub fn leaves(&self) -> bool {
        matches!(
            self,
            Op::Jump { .. } | Op::Switch { .. } | Op::Return { .. } | Op::Unreachable
        )
    }
}

/// How an operand is stored after the opcode word: its bytes, lowest first, and how they are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    U8,
    /// A byte widened with its sign: a short jump's distance.
    S8,
    U16,
    U24,
    U32,
    /// Four bytes widened with their sign.
    S32,
    U64,
    /// The bits of a `float`.
    F32,
}

impl Type {
    pub fn bytes(self) -> u32 {
        match self {
            Type::U8 | Type::S8 => 1,
            Type::U16 => 2,
            Type::U24 => 3,
            Type::U32 | Type::S32 | Type::F32 => 4,
            Type::U64 => 8,
        }
    }
}

/// One operand of an instruction: its name in the disassembly and how it is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field(pub &'static str, pub Type);

/// How far an opcode's meaning is established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// What it does was held against lifted listings of the programs it occurs in.
    Matched,
    /// It occurs and its cells balance in every function, and what it does is read from its place in its family.
    Inferred,
    /// No program was seen to hold it: only its length is taken as known, from its family.
    Unseen,
}

/// The parameters of a function, or its locals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Param,
    Local,
}

/// What the decoder does with an instruction: its family, with what tells the family's members apart.
// An operand is named by its place among the row's fields. A `u8` beside a load or a store is the
// bytes the instruction adds to its offset operand (8 past an object's header); a `bool` named
// `builtin` says the type operand is a builtin type's index, one named `statics` that the offset
// is in the statics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Nothing on the stack: a no-op, or the entry instruction that sizes a function's frame.
    Nop,
    /// Pushes a frame cell: the one numbered, or the one operand 0 names where the number is negative.
    PushVar(Frame, i8),
    /// Pushes operand 1 cells of the frame from cell operand 0 on, the lowest deepest.
    PushVars(Frame),
    /// Pushes the two cells from cell operand 0 on.
    PushVar2(Frame),
    /// Pops into a frame cell, numbered as `PushVar` has it.
    SetVar(Frame, i8),
    /// Pops operand 1 cells into the frame from cell operand 0 on, the top into the highest.
    SetVars(Frame),
    SetVar2(Frame),
    AddrOfVar(Frame),
    /// Stores the top cell in a local and leaves it.
    TeeVar,
    TeeVar2,
    /// Adds one to a frame cell's low 32 bits, or takes one off when the flag is set.
    StepVar(Frame, bool),
    /// Pushes a parameter and then steps it.
    PostStepParam(bool),
    PushInt(i8),
    PushF1,
    /// Pushes operand 0 as an `int`.
    PushImm,
    /// Pushes each operand as an `int`, the first deepest.
    PushImms,
    PushType,
    PushI64,
    PushBit,
    PushString,
    PushVector3,
    Dup,
    DupN,
    Pick,
    DropOne,
    DropN,
    Sink,
    Load(Scalar, u8),
    /// A load through parameter 0, which the instruction does not pop.
    LoadP0(Scalar, u8),
    LoadBytes(u8),
    LoadVector(u8),
    /// Moves an address by its operand.
    Offset(u8),
    /// Takes a scalar out of a value of several cells.
    Extract(Scalar),
    ExtractBytes,
    ExtractVector,
    LoadGlobal(Scalar, bool),
    LoadGlobalBytes(bool),
    LoadGlobalVector(bool),
    GlobalAddress(bool),
    BuiltinGlobal,
    StoreGlobal(Scalar),
    StoreGlobalBytes(bool),
    StoreGlobalVector,
    Store(StoreKind, u8),
    StoreBytes(u8),
    StoreF32s(u8),
    StoreU64s(u8),
    StoreVector(u8),
    StoreValue(bool),
    Deref(Scalar),
    DerefBytes,
    /// Loads through the address a number of cells down, in its place.
    DerefAt,
    Put(StoreKind),
    StepField(u8, bool),
    ZeroBytes,
    PushFunction,
    PushVirtualFunction,
    PushBuiltinFunction,
    Call,
    CallVirtual(bool),
    CallBuiltin,
    CallNative,
    CallDelegate,
    TypeCast(bool),
    UnboxAddressAt,
    UnboxAddress,
    /// Returns the cells numbered, or operand 0 cells where the number is negative.
    Return(i8),
    NewArray(bool),
    BoxValue(bool),
    UnboxValue(bool),
    BoxAt(bool),
    NewObject,
    NewObjectPlain,
    NewWeakReference,
    NewBuiltin,
    NewBuiltinValue,
    AllocValue,
    ArrayGetValue,
    ArraySetValue,
    BytesEqual,
    Jump,
    Switch,
    /// Jumps where the cell read as the type compares so with zero.
    BranchZero(Scalar, BinOp),
    /// Jumps where the two cells read as the type compare so; the flag negates the comparison.
    BranchCompare(BinOp, Scalar, bool),
    Zero(Scalar, BinOp),
    Compare(BinOp, Scalar, bool),
    Select,
    /// Chooses between operand 1 and operand 0.
    SelectImm,
    Binary(BinOp, Scalar),
    Unary(UnOp, Scalar),
    Shift(BinOp, Scalar),
    /// Adds one to the top cell's low 32 bits, or takes one off when the flag is set.
    StepCell(bool),
    Square,
    Convert(Scalar),
    /// A call of the runtime's operation named, with the cells it takes and yields.
    Intrinsic(&'static str, u8, u8),
    Barrier,
    StackAlloc,
    ElementAddress,
    StackAddress,
}

/// One instruction of the set: its number, its name, its operands and what it does.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub opcode: u16,
    pub name: &'static str,
    pub fields: &'static [Field],
    pub status: Status,
    pub act: Act,
}

impl Spec {
    /// The bytes the instruction takes: its word and its operands.
    pub fn bytes(&self) -> u32 {
        2 + self.fields.iter().map(|field| field.1.bytes()).sum::<u32>()
    }
}

/// The opcode a code word spells.
// The word is the opcode times 0x843D, of which 0x3315 is the inverse modulo 65536.
pub fn of_word(word: u16) -> u16 {
    word.wrapping_mul(0x3315)
}

/// The operand byte stored as `stored` at place `index` after the opcode word.
// Stored is plain times 0x65, of which 0x6D is the inverse modulo 256, under a key that steps by 0x11 a place.
pub fn plain(stored: u8, index: usize) -> u8 {
    let key = 0x85 ^ 0x11u8.wrapping_mul(index as u8);
    (stored ^ key).wrapping_mul(0x6D)
}

/// The row of an opcode, `None` for a number that is no instruction.
pub fn spec(opcode: u16) -> Option<&'static Spec> {
    let found = table::TABLE.binary_search_by_key(&opcode, |row| row.opcode);
    found.ok().and_then(|n| table::TABLE.get(n))
}

/// The mnemonic of an opcode, `None` for a number that is no instruction.
pub fn name(opcode: u16) -> Option<&'static str> {
    spec(opcode).map(|row| row.name)
}

/// The cells method `method` of builtin type `ty` takes and the cells it returns.
pub fn builtin_method(ty: u32, method: u32) -> Option<(u32, u32)> {
    let rows = table::BUILTIN_METHODS.iter();
    let row = rows
        .copied()
        .find(|row| (u32::from(row.0), u32::from(row.1)) == (ty, method));
    row.map(|row| (u32::from(row.2), u32::from(row.3)))
}

/// The bytes a value of builtin type `index` takes and the type's flags, 0 for flags no program states.
pub fn builtin_type(index: u32) -> Option<(u32, u32)> {
    let mut rows = table::BUILTIN_TYPES.iter().copied();
    let row = rows.find(|row| u32::from(row.0) == index);
    row.map(|row| (u32::from(row.1), row.2))
}

/// The cells a call of virtual slot `slot` of builtin type `ty` returns.
pub fn builtin_slot(ty: u32, slot: u32) -> Option<u32> {
    let mut rows = table::BUILTIN_SLOTS.iter().copied();
    let row = rows.find(|row| (u32::from(row.0), u32::from(row.1)) == (ty, slot));
    row.map(|row| u32::from(row.2))
}

#[cfg(test)]
#[path = "opcode_tests.rs"]
mod tests;
