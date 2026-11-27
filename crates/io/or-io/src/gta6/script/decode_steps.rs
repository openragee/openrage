//! What each instruction does, as steps on the stack: the decoder's reading of an opcode's row,
//! with every token resolved against the program's tables and, past them, the project's.
//!
//! ⚠ The steps spell what a lifted listing spells for the same instruction (`as_T` around each
//! operand of a typed operator, a negated opposite for an ordered float test), so that both front
//! ends hand the passes one form. Four things are stated more exactly than a listing has them:
//! a scalar taken out of a value keeps its own width, an array store of eight bytes that are no
//! reference is `array_set_u64`, a cast says by its mode whether it must hold, and the token of a
//! type is that type.

use super::super::ir::{
    BinOp, Callee, Const, Expr, FunctionKey, Kind, Scalar, TypeKey, UnOp, Var, VmType,
};
use super::super::opcode::{self, Act, Frame, Op, Spec, Status, Type};
use super::super::program::{Program, SlotRow};
use super::super::project::Project;
use super::{DisasmError, Raw};

/// Where a token of the code is looked up: the program's own tables first, the project's for what another program defines.
pub(super) struct Tables<'a> {
    pub program: &'a Program,
    pub project: &'a Project,
}

impl Tables<'_> {
    /// The hash of the program an instruction names by its place in the import table, 0 being the program itself.
    fn import(&self, at: u32, index: u32) -> Result<u32, DisasmError> {
        self.program.imported(index).ok_or_else(|| {
            let what = format!("import {index} is past the program's import table");
            malformed(at, what)
        })
    }

    /// The type an instruction names: a builtin by its index, any other by its id in this program.
    fn ty(&self, at: u32, id: u32, builtin: bool) -> Result<VmType, DisasmError> {
        if builtin || id & 0x8000_0000 != 0 {
            return Ok(builtin_type(id & 0x7FFF_FFFF));
        }
        if let Some(row) = self.program.types.get(id as usize) {
            return Ok(VmType {
                name: TypeKey(row.name).to_string(),
                byte_size: row.byte_size,
                id,
                flags: row.flags,
            });
        }
        // Another program's type: its name and size are in that program, by the id it has there.
        let imported = || {
            let mut imports = self.program.imports.iter();
            let import = imports.find(|row| row.types.contains(&id))?;
            let home = self.project.programs.get(&import.program)?;
            let key = *home.types.get(&(id - import.types.start))?;
            Some((key, self.project.types.get(&key)?))
        };
        let Some((key, facts)) = imported() else {
            let what = format!("type {id} is another program's, and the project does not hold it");
            return Err(malformed(at, what));
        };
        Ok(VmType {
            name: key.to_string(),
            byte_size: facts.byte_size,
            id,
            flags: facts.flags,
        })
    }

    /// The cells a function takes and the cells it returns, where a table states them.
    fn signature(&self, program: u32, at: u32) -> Option<(u32, u32)> {
        let own = (program == self.program.program).then(|| self.program.function(at));
        if let Some(row) = own.flatten() {
            return Some((row.params, row.returns));
        }
        let known = self
            .project
            .signatures
            .get(&FunctionKey::new(program, at))?;
        let cells = known.params.iter().map(|param| param.cells);
        Some((cells.fold(0, u32::saturating_add), known.returns))
    }

    /// The cells a call of virtual slot `slot` of a script type returns, where a table states what fills the slot.
    // The one method record a type has for the slot states the cells itself, and is all there is
    // for an interface the engine implements. Else they are the cells of what the type's own slot
    // holds; for a type of another program, those of the function the project resolves the slot to.
    fn slot_returns(&self, ty: &VmType, slot: u32) -> Option<u32> {
        let returns = |program: u32, at: u32| self.signature(program, at).map(|cells| cells.1);
        if let Some(row) = self.program.types.get(ty.id as usize) {
            let own = row.methods.start as usize..row.methods.end as usize;
            let records = self.program.methods.get(own).unwrap_or(&[]);
            let mut stated = records.iter().filter(|method| method.slot == Some(slot));
            if let (Some(method), None) = (stated.next(), stated.next()) {
                return Some(method.returns);
            }
            match row.vtable.get(slot as usize) {
                Some(SlotRow::Function { import, at, .. }) => {
                    return returns(self.program.imported(*import)?, *at);
                }
                Some(SlotRow::Builtin { ty, method, .. }) => {
                    return opcode::builtin_method(*ty, *method).map(|cells| cells.1);
                }
                Some(SlotRow::Empty) | None => {}
            }
        }
        // Another program's type: what its home states of the slot is in the project.
        self.project.slot_returns(TypeKey::of(ty)?, slot)
    }
}

fn malformed(at: u32, what: impl Into<String>) -> DisasmError {
    let what = what.into();
    DisasmError::Malformed { at, what }
}

/// A builtin type as a program with no names list spells it.
fn builtin_type(index: u32) -> VmType {
    let id = 0x8000_0000 | index;
    let (byte_size, flags) = opcode::builtin_type(index).unwrap_or((0, 0));
    VmType {
        name: TypeKey(u64::from(id)).to_string(),
        byte_size,
        id,
        flags,
    }
}

/// The bytes a value of a type takes, refused for a builtin type no table sizes.
fn sized(at: u32, ty: &VmType) -> Result<u32, DisasmError> {
    if ty.id & 0x8000_0000 == 0 {
        return Ok(ty.byte_size);
    }
    let index = ty.id & 0x7FFF_FFFF;
    let known = opcode::builtin_type(index).map(|(bytes, _)| bytes);
    known.ok_or_else(|| malformed(at, format!("the size of builtin type {index} is not known")))
}

/// The cells a builtin constructor takes, the object or the value's address among them.
fn constructor(at: u32, ty: u32, method: u32) -> Result<u32, DisasmError> {
    match opcode::builtin_method(ty, method) {
        Some((args, 0)) if args > 0 => Ok(args),
        Some(_) => Err(malformed(
            at,
            format!("builtin method {ty}.{method} is no constructor: it takes no object, or returns cells"),
        )),
        None => Err(malformed(
            at,
            format!("the cells of builtin method {ty}.{method} are not known"),
        )),
    }
}

fn builtin_callee(ty: u32, method: u32) -> Callee {
    let path = format!("type_{:08X}.Method_{method}", 0x8000_0000 | ty);
    Callee::Builtin { path, ty, method }
}

/// One of the runtime's operations, called through the wrapper a listing declares under the same name.
fn operation(name: &str) -> Callee {
    Callee::Intrinsic {
        name: name.to_owned(),
        ty: None,
        wrapper: Some(name.to_owned()),
    }
}

/// The operation that yields the token of its type: `typeof(T)`.
pub(in crate::gta6::script) const TYPE_TOKEN: &str = "typeof";

/// The operation that casts to its type and faults where the object is none of it: `(T)x`.
pub(in crate::gta6::script) const CAST: &str = "cast";

/// An intrinsic a listing writes in place, with no wrapper.
fn inline(name: &str, ty: Option<VmType>) -> Callee {
    let name = name.to_owned();
    let wrapper = None;
    Callee::Intrinsic { name, ty, wrapper }
}

/// The operation `name` over a type, as a listing declares one for each type it is used with.
fn typed(name: &str, ty: &VmType) -> Callee {
    Callee::Intrinsic {
        name: format!("{name}<{}>", ty.name),
        ty: None,
        wrapper: Some(format!("{name}_{}_", ty.name)),
    }
}

fn call(callee: Callee, args: u32, returns: u32) -> Op {
    Op::Call {
        callee,
        args,
        returns,
    }
}

fn push(e: Expr) -> Op {
    Op::Push { e, cells: 1 }
}

fn int(value: i32) -> Op {
    push(Expr::Const(Const::i32(value)))
}

fn cell(frame: Frame, index: u32) -> Var {
    match frame {
        Frame::Param => Var::Param(index),
        Frame::Local => Var::Local(index),
    }
}

/// The number a row fixes, or `operand` where the row leaves it to the instruction.
fn numbered(fixed: i8, operand: u32) -> u32 {
    u32::try_from(fixed).unwrap_or(operand)
}

fn step(down: bool) -> BinOp {
    match down {
        true => BinOp::Sub,
        false => BinOp::Add,
    }
}

// `depth` is 2 at the least where these are called: a cell that is the top already moves nowhere.
fn sink(depth: u32) -> Op {
    match depth {
        2 => Op::Swap,
        _ => Op::Sink { depth },
    }
}

fn raise(depth: u32) -> Op {
    match depth {
        2 => Op::Swap,
        _ => Op::Raise { depth },
    }
}

/// `inner` worked on the cell `depth` from the top, the top being 1, and its result left in that place.
fn in_place(at: u32, depth: u32, inner: Vec<Op>) -> Result<Vec<Op>, DisasmError> {
    match depth {
        0 => Err(malformed(at, "a depth of zero names no cell")),
        1 => Ok(inner),
        _ => {
            let mut ops = vec![raise(depth)];
            ops.extend(inner);
            ops.push(sink(depth));
            Ok(ops)
        }
    }
}

/// The two top cells each read as `ty`, in their places.
fn both(ty: Scalar) -> Vec<Op> {
    vec![Op::Swap, Op::Cast(ty), Op::Swap, Op::Cast(ty)]
}

fn compare(op: BinOp, ty: Scalar, not: bool) -> Vec<Op> {
    let mut ops = both(ty);
    ops.push(Op::Binary { op, at: ty.kind() });
    if not {
        ops.push(Op::Unary(UnOp::Not));
    }
    ops
}

fn against_zero(op: BinOp, ty: Scalar) -> Vec<Op> {
    let at = ty.kind();
    vec![Op::Cast(ty), int(0), Op::Binary { op, at }]
}

/// `var++` or `var--` as a listing's reader has it: the low 32 bits stepped and stored back.
fn stepped(var: Var, down: bool) -> Vec<Op> {
    let (op, at) = (step(down), Kind::I32);
    vec![
        push(Expr::Var(var)),
        Op::Cast(Scalar::I32),
        int(1),
        Op::Binary { op, at },
        Op::Set(var),
    ]
}

/// A store of `bytes` bytes through `address`, which the instruction does not hold under the cells it pops.
fn stored_under(address: Expr, bytes: u32) -> Vec<Op> {
    let cells = bytes.div_ceil(8);
    let mut ops = vec![push(address)];
    if cells > 0 {
        ops.push(sink(cells + 1));
    }
    let offset = 0;
    ops.push(Op::StoreBytes {
        offset,
        bytes,
        cells,
    });
    ops
}

/// A value of `cells` zero cells and above it its address, both put under the `above` cells on top.
fn value_under(cells: u32, above: u32) -> Vec<Op> {
    let e = Expr::ZeroSlots(cells);
    let mut ops = vec![Op::Push { e, cells }, Op::AddrOfTop { cells }];
    ops.extend((0..above).map(|_| raise(above + cells + 1)));
    ops
}

/// Where a jump of `rel` bytes, counted from the instruction's operand, goes; refused outside the code.
fn target(code: usize, raw: &Raw, rel: i64) -> Result<u32, DisasmError> {
    let to = u32::try_from(i64::from(raw.at) + 2 + rel).ok();
    let inside = to.filter(|to| (*to as usize) < code);
    inside.ok_or_else(|| malformed(raw.at, "a jump out of the code"))
}

/// The most cases one switch is decoded with: each is held as a target of its own.
const MAX_CASES: usize = 1 << 16;

/// The case targets of a switch, from its table in `blob`, case `n` at place `n`.
fn switch(code: usize, blob: &[u8], raw: &Raw) -> Result<Vec<(i64, u32)>, DisasmError> {
    let (count, table) = (raw.fields[0] as usize, raw.fields[1] as usize);
    if count > MAX_CASES {
        return Err(malformed(
            raw.at,
            "a switch of more cases than a decoder holds",
        ));
    }
    let bad = || malformed(raw.at, "a switch table that is not in the program's data");
    let bytes = count.checked_mul(4).ok_or_else(bad)?;
    let end = table.checked_add(bytes).ok_or_else(bad)?;
    let entries = blob.get(table..end).ok_or_else(bad)?;
    let mut cases = Vec::with_capacity(count);
    for (case, entry) in entries.as_chunks::<4>().0.iter().enumerate() {
        let to = target(code, raw, i64::from(i32::from_le_bytes(*entry)))?;
        cases.push((case as i64, to));
    }
    Ok(cases)
}

/// The value type an instruction names for the cells it stores, makes in place or takes out of a box; `None` for any other instruction and for a type the tables do not hold.
// ⚠ Its steps say none of it: `store_value` is a store of bytes, `unbox` a load of them and
// `new_builtin_value` zero cells under the constructor's arguments. `alloc_value` names no type.
pub(super) fn value_type(tables: &Tables<'_>, raw: &Raw, spec: &Spec) -> Option<TypeKey> {
    if spec.status == Status::Unseen {
        return None;
    }
    let n = |i: usize| raw.fields[i] as u32;
    let ty = match spec.act {
        Act::StoreValue(builtin) | Act::UnboxValue(builtin) => {
            tables.ty(raw.at, n(0), builtin).ok()?
        }
        Act::NewBuiltinValue => builtin_type(n(1)),
        _ => return None,
    };
    TypeKey::of(&ty)
}

/// The steps of one instruction.
pub(super) fn of(tables: &Tables<'_>, raw: &Raw, spec: &Spec) -> Result<Vec<Op>, DisasmError> {
    let at = raw.at;
    if spec.status == Status::Unseen {
        let name = spec.name;
        let what = format!("`{name}` was met in no program: what it does is not established");
        return Err(malformed(at, what));
    }
    // An operand that is no 64-bit literal fits 32 bits.
    let n = |i: usize| raw.fields[i] as u32;
    let code = tables.program.code.len();
    let jump = || target(code, raw, raw.fields[0] as i64);
    let global = |statics: bool| {
        let program = tables.import(at, n(0))?;
        let offset = n(1);
        Ok::<Var, DisasmError>(Var::Global {
            program,
            offset,
            statics,
        })
    };
    let address = |statics: bool| global(statics).map(|var| Expr::AddrOf { var, cell: 0 });
    let based = |base: u8| u32::from(base) + n(0);
    let ops = match spec.act {
        Act::Nop => vec![Op::Nop],
        Act::PushVar(frame, fixed) => vec![push(Expr::Var(cell(frame, numbered(fixed, n(0)))))],
        Act::PushVars(frame) => {
            let cells = (0..n(1)).map(|k| push(Expr::Var(cell(frame, n(0) + k))));
            cells.collect()
        }
        Act::PushVar2(frame) => vec![
            push(Expr::Var(cell(frame, n(0)))),
            push(Expr::Var(cell(frame, n(0) + 1))),
        ],
        Act::SetVar(frame, fixed) => vec![Op::Set(cell(frame, numbered(fixed, n(0))))],
        // The top of the stack goes to the highest cell.
        Act::SetVars(frame) => {
            let cells = (0..n(1)).rev().map(|k| Op::Set(cell(frame, n(0) + k)));
            cells.collect()
        }
        Act::SetVar2(frame) => vec![Op::Set(cell(frame, n(0) + 1)), Op::Set(cell(frame, n(0)))],
        Act::AddrOfVar(frame) => {
            let var = cell(frame, n(0));
            vec![push(Expr::AddrOf { var, cell: 0 })]
        }
        Act::TeeVar => vec![Op::Copy { depth: 0, cells: 1 }, Op::Set(Var::Local(n(0)))],
        Act::TeeVar2 => vec![
            Op::Copy { depth: 0, cells: 2 },
            Op::Set(Var::Local(n(0) + 1)),
            Op::Set(Var::Local(n(0))),
        ],
        Act::StepVar(frame, down) => stepped(cell(frame, n(0)), down),
        // ⚠ The value pushed is the parameter before the step: a lifter keeps it apart from the cell.
        Act::PostStepParam(down) => {
            let var = Var::Param(n(0));
            let mut ops = vec![push(Expr::Var(var))];
            ops.extend(stepped(var, down));
            ops
        }
        Act::PushInt(value) => vec![int(i32::from(value))],
        Act::PushF1 => vec![push(Expr::Const(Const::f32(1.0)))],
        Act::PushImm => vec![int(n(0) as i32)],
        Act::PushImms => {
            let values = raw.fields.iter().take(spec.fields.len());
            values.map(|value| int(*value as u32 as i32)).collect()
        }
        // The token of a type the tables hold is that type, named. ⚠ Any other stays the number
        // a listing prints as the `float` of the same bits: the number is all the program says.
        Act::PushType => match tables.ty(at, n(0), false) {
            Ok(ty) => vec![call(inline(TYPE_TOKEN, Some(ty)), 0, 1)],
            Err(_) => {
                let (bits, kind) = (u64::from(n(0)), Kind::F32);
                vec![push(Expr::Const(Const { bits, kind }))]
            }
        },
        Act::PushI64 => {
            let (bits, kind) = (raw.fields[0], Kind::U64);
            vec![push(Expr::Const(Const { bits, kind }))]
        }
        Act::PushBit => {
            let (bits, kind) = (1, Kind::U64);
            let one = Expr::Const(Const { bits, kind });
            let bit = Expr::cast(Scalar::I32, Expr::Const(Const::i32(n(0) as i32)));
            vec![push(Expr::binary(BinOp::Shl, one, bit))]
        }
        Act::PushString => {
            let (program, offset) = (tables.program.program, n(0));
            vec![push(Expr::StringConstant { program, offset })]
        }
        Act::PushVector3 => {
            let part = |i: usize| {
                let (bits, kind) = (u64::from(n(i)), Kind::F32);
                Expr::Const(Const { bits, kind })
            };
            let e = Expr::Vector3(Box::new([part(0), part(1), part(2)]));
            vec![Op::Push { e, cells: 2 }]
        }
        Act::Dup => vec![Op::Copy { depth: 0, cells: 1 }],
        Act::DupN => {
            let (depth, cells) = (0, n(0));
            vec![Op::Copy { depth, cells }]
        }
        Act::Pick => match n(0) {
            0 => return Err(malformed(at, "a depth of zero names no cell")),
            depth => vec![Op::Copy {
                depth: depth - 1,
                cells: 1,
            }],
        },
        Act::DropOne => vec![Op::Drop { cells: 1 }],
        Act::DropN => vec![Op::Drop { cells: n(0) }],
        Act::Sink => match n(0) {
            0 => return Err(malformed(at, "a depth of zero names no cell")),
            1 => vec![Op::Nop],
            depth => vec![sink(depth)],
        },
        Act::Load(ty, base) => {
            let offset = based(base);
            vec![Op::Load { ty, offset }]
        }
        Act::LoadP0(ty, base) => {
            let offset = based(base);
            vec![push(Expr::Var(Var::Param(0))), Op::Load { ty, offset }]
        }
        Act::LoadBytes(base) => {
            let (offset, bytes) = (based(base), n(1));
            vec![Op::LoadBytes { offset, bytes }]
        }
        Act::LoadVector(base) => {
            let (offset, bytes) = (based(base), 16);
            vec![Op::LoadBytes { offset, bytes }]
        }
        // An address moved by nothing is the address: a listing prints it alone.
        Act::Offset(base) => match based(base) {
            0 => vec![Op::Nop],
            offset => vec![int(offset as i32), Op::ByteOffset],
        },
        Act::Extract(ty) => {
            let (cells, offset, bytes) = (n(0), n(1), ty.bytes());
            let mut ops = vec![Op::ExtractBytes {
                cells,
                offset,
                bytes,
            }];
            // The bytes come out widened with zeros: a signed field is read again to take its sign.
            if matches!(ty, Scalar::I8 | Scalar::I16 | Scalar::I32) {
                ops.push(Op::Cast(ty));
            }
            ops
        }
        Act::ExtractBytes => {
            let (cells, offset, bytes) = (n(0), n(1), n(2));
            vec![Op::ExtractBytes {
                cells,
                offset,
                bytes,
            }]
        }
        Act::ExtractVector => {
            let (cells, offset, bytes) = (n(0), n(1), 16);
            vec![Op::ExtractBytes {
                cells,
                offset,
                bytes,
            }]
        }
        Act::LoadGlobal(ty, statics) => vec![push(Expr::Var(global(statics)?)), Op::Cast(ty)],
        Act::LoadGlobalBytes(statics) => {
            let (offset, bytes) = (0, n(2));
            vec![push(address(statics)?), Op::LoadBytes { offset, bytes }]
        }
        Act::LoadGlobalVector(statics) => {
            let (offset, bytes) = (0, 16);
            vec![push(address(statics)?), Op::LoadBytes { offset, bytes }]
        }
        Act::GlobalAddress(statics) => vec![push(address(statics)?)],
        Act::BuiltinGlobal => match n(0) {
            3 => {
                let e = Expr::GuidEmpty;
                vec![Op::Push { e, cells: 2 }]
            }
            index @ (0 | 1 | 4) => {
                let base = Box::new(Expr::BuiltinGlobal(index));
                vec![push(Expr::Cell { base, index: 0 })]
            }
            index => {
                let what = format!("the cells of builtin global {index} are not known");
                return Err(malformed(at, what));
            }
        },
        Act::StoreGlobal(ty) => vec![Op::Cast(ty), Op::Set(global(false)?)],
        Act::StoreGlobalBytes(statics) => stored_under(address(statics)?, n(2)),
        Act::StoreGlobalVector => stored_under(address(false)?, 16),
        Act::Store(kind, base) => {
            let offset = based(base);
            vec![Op::Store { kind, offset }]
        }
        Act::StoreBytes(base) => {
            let (offset, bytes) = (based(base), n(1));
            let cells = bytes.div_ceil(8);
            vec![Op::StoreBytes {
                offset,
                bytes,
                cells,
            }]
        }
        Act::StoreF32s(base) => {
            let (offset, cells) = (based(base), n(1));
            vec![Op::StoreF32Slots { offset, cells }]
        }
        Act::StoreU64s(base) => {
            let (offset, cells) = (based(base), n(1));
            let bytes = 8 * cells;
            vec![Op::StoreBytes {
                offset,
                bytes,
                cells,
            }]
        }
        Act::StoreVector(base) => {
            let (offset, bytes, cells) = (based(base), 16, 2);
            vec![Op::StoreBytes {
                offset,
                bytes,
                cells,
            }]
        }
        // A value takes a cell at the least, even one of no bytes.
        Act::StoreValue(builtin) => {
            let bytes = sized(at, &tables.ty(at, n(0), builtin)?)?;
            let (offset, cells) = (0, bytes.div_ceil(8).max(1));
            vec![Op::StoreBytes {
                offset,
                bytes,
                cells,
            }]
        }
        Act::Deref(ty) => vec![Op::Load { ty, offset: 0 }],
        Act::DerefBytes => {
            let (offset, bytes) = (0, n(0));
            vec![Op::LoadBytes { offset, bytes }]
        }
        Act::DerefAt => {
            let (ty, offset) = (Scalar::U64, 0);
            in_place(at, n(0), vec![Op::Load { ty, offset }])?
        }
        Act::Put(kind) => vec![Op::Store { kind, offset: 0 }],
        Act::StepField(base, down) => {
            let offset = based(base);
            vec![Op::Step { offset, down }]
        }
        Act::ZeroBytes => vec![Op::ZeroBytes { bytes: n(0) }],
        // A listing prints the offset and the flags as one number, and so does this.
        Act::PushFunction => {
            let program = tables.import(at, n(0))?;
            let at = n(1) | (n(2) << 24);
            vec![push(Expr::FunctionRef { program, at })]
        }
        Act::PushVirtualFunction => {
            let (ty, slot) = (tables.ty(at, n(0), false)?, n(1));
            vec![Op::VirtualFunctionRef { ty, slot }]
        }
        Act::PushBuiltinFunction => {
            let (method, ty) = (n(0), n(1));
            vec![push(Expr::BuiltinFunctionRef { ty, method })]
        }
        Act::Call => {
            let (program, to) = (tables.import(at, n(0))?, n(1));
            let name = format!("func_{to:06X}");
            let callee = match n(0) {
                0 => Callee::Local { at: to, name },
                _ => Callee::Import {
                    module: format!("program_{program:08X}"),
                    name,
                    program,
                    at: to,
                },
            };
            match tables.signature(program, to) {
                Some((args, returns)) => vec![call(callee, args, returns)],
                None => vec![Op::Unsized {
                    callee,
                    args: None,
                    returns: None,
                }],
            }
        }
        Act::CallVirtual(builtin) => {
            let (args, slot) = (n(0), n(2));
            let ty = tables.ty(at, n(1), builtin)?;
            let returns = match builtin {
                true => opcode::builtin_slot(n(1), slot),
                false => tables.slot_returns(&ty, slot),
            };
            let callee = Callee::Virtual {
                class: ty.name.clone(),
                method: format!("Method_{slot}"),
                ty: ty.name,
                slot,
            };
            match returns {
                Some(returns) => vec![call(callee, args, returns)],
                None => vec![Op::Unsized {
                    callee,
                    args: Some(args),
                    returns: None,
                }],
            }
        }
        Act::CallBuiltin => {
            let (method, ty) = (n(0), n(1));
            let callee = builtin_callee(ty, method);
            match opcode::builtin_method(ty, method) {
                Some((args, returns)) => vec![call(callee, args, returns)],
                None => vec![Op::Unsized {
                    callee,
                    args: None,
                    returns: None,
                }],
            }
        }
        // The code names a native by its handler's address alone: no table of the program gives its hash.
        Act::CallNative => {
            let rva = n(0);
            let callee = Callee::Native {
                namespace: None,
                name: format!("rva_{rva:08X}"),
                hash: 0,
                rva,
            };
            vec![call(callee, n(1), n(2))]
        }
        Act::CallDelegate => vec![call(operation("invoke_delegate"), n(0), n(1))],
        // ⚠ Mode 0 yields null where the object is no such type and mode 1 must hold: `x as T`
        // and `(T)x`. Any other mode is not understood, and keeps its number in its name.
        Act::TypeCast(builtin) => {
            let ty = tables.ty(at, n(0), builtin)?;
            let name = match n(1) {
                0 => "try_cast".to_owned(),
                1 => CAST.to_owned(),
                mode => format!("cast_mode_{mode}"),
            };
            vec![call(typed(&name, &ty), 1, 1)]
        }
        Act::UnboxAddressAt => in_place(at, n(0), vec![int(8), Op::ByteOffset])?,
        Act::UnboxAddress => vec![int(8), Op::ByteOffset],
        Act::Return(fixed) => vec![Op::Return {
            cells: numbered(fixed, n(0)),
        }],
        Act::NewArray(builtin) => {
            let ty = tables.ty(at, n(0), builtin)?;
            let wrapper = format!("NewArray_{}", ty.name);
            vec![Op::NewArray { ty, wrapper }]
        }
        Act::BoxValue(builtin) => {
            let ty = tables.ty(at, n(0), builtin)?;
            let cells = sized(at, &ty)?.div_ceil(8).max(1);
            vec![call(typed("box", &ty), cells, 1)]
        }
        // The value is read past the object's header, a whole cell at the least.
        Act::UnboxValue(builtin) => {
            let bytes = sized(at, &tables.ty(at, n(0), builtin)?)?.max(8);
            vec![Op::LoadBytes { offset: 8, bytes }]
        }
        Act::BoxAt(builtin) => {
            let ty = tables.ty(at, n(0), builtin)?;
            in_place(at, n(1), vec![call(inline("box_at", Some(ty)), 1, 1)])?
        }
        // Two references go under the constructor's arguments: its receiver, and the result under that.
        Act::NewObject => {
            let (ty, depth) = (tables.ty(at, n(0), false)?, n(1));
            let mut ops = vec![push(Expr::NewObject(ty)), Op::Copy { depth: 0, cells: 1 }];
            match depth {
                0 => return Err(malformed(at, "a depth of zero names no cell")),
                1 => {}
                _ => ops.extend([sink(depth + 1), sink(depth + 1)]),
            }
            ops
        }
        Act::NewObjectPlain => vec![push(Expr::NewObject(tables.ty(at, n(0), false)?))],
        Act::NewWeakReference => {
            let name = "weak_reference".to_owned();
            let (byte_size, id, flags) = (0, 0, 0);
            vec![push(Expr::NewObject(VmType {
                name,
                byte_size,
                id,
                flags,
            }))]
        }
        Act::NewBuiltin => {
            let (method, ty) = (n(0), n(1));
            let args = constructor(at, ty, method)?;
            let object = Expr::NewObject(builtin_type(ty));
            let mut ops = vec![push(object), Op::Copy { depth: 0, cells: 1 }];
            if args > 1 {
                ops.extend([sink(args + 1), sink(args + 1)]);
            }
            ops.push(call(builtin_callee(ty, method), args, 0));
            ops
        }
        Act::NewBuiltinValue => {
            let (method, ty) = (n(0), n(1));
            let args = constructor(at, ty, method)?;
            let cells = sized(at, &builtin_type(ty))?.div_ceil(8).max(1);
            let mut ops = value_under(cells, args - 1);
            ops.push(call(builtin_callee(ty, method), args, 0));
            ops
        }
        Act::AllocValue => match n(1) {
            0 => return Err(malformed(at, "a depth of zero names no cell")),
            depth => value_under(n(0), depth - 1),
        },
        // The cells of an element are in no instruction and no table: the lifter finds them.
        Act::ArrayGetValue => vec![Op::Unsized {
            callee: operation("array_get_value"),
            args: Some(2),
            returns: None,
        }],
        Act::ArraySetValue => vec![call(operation("array_set_slots"), 2 + n(0), 0)],
        Act::BytesEqual => {
            let name = format!("bytes_equal_{}", n(0));
            vec![call(operation(&name), 2, 1)]
        }
        Act::Jump => vec![Op::Jump { to: jump()? }],
        // A selector no case takes goes on at the next instruction.
        Act::Switch => {
            let cases = switch(code, &tables.program.strings, raw)?;
            let default = at.saturating_add(raw.len);
            vec![Op::Cast(Scalar::I32), Op::Switch { cases, default }]
        }
        Act::BranchZero(ty, op) => {
            let mut ops = against_zero(op, ty);
            ops.push(Op::Branch { to: jump()? });
            ops
        }
        Act::BranchCompare(op, ty, not) => {
            let mut ops = compare(op, ty, not);
            ops.push(Op::Branch { to: jump()? });
            ops
        }
        Act::Zero(ty, op) => against_zero(op, ty),
        Act::Compare(op, ty, not) => compare(op, ty, not),
        // The instruction holds the value for yes on top, the step the value for no.
        Act::Select => vec![Op::Swap, Op::Select],
        Act::SelectImm => {
            let wide = spec
                .fields
                .first()
                .is_some_and(|field| field.1 == Type::U64);
            let value = |bits: u64| match wide {
                true => {
                    let kind = Kind::U64;
                    Expr::Const(Const { bits, kind })
                }
                false => Expr::Const(Const::i32(bits as u32 as i32)),
            };
            vec![
                push(value(raw.fields[1])),
                push(value(raw.fields[0])),
                Op::Select,
            ]
        }
        Act::Binary(op, ty) => {
            let mut ops = both(ty);
            ops.push(Op::Binary { op, at: ty.kind() });
            ops
        }
        Act::Unary(op, ty) => vec![Op::Cast(ty), Op::Unary(op)],
        // A listing reads the count as the type and then as an `int`.
        Act::Shift(op, ty) => vec![
            Op::Cast(ty),
            Op::Cast(Scalar::I32),
            Op::Swap,
            Op::Cast(ty),
            Op::Swap,
            Op::Binary { op, at: ty.kind() },
        ],
        Act::StepCell(down) => {
            let (op, at) = (step(down), Kind::I32);
            vec![Op::Cast(Scalar::I32), int(1), Op::Binary { op, at }]
        }
        Act::Square => {
            let (op, at) = (BinOp::Mul, Kind::F32);
            vec![
                Op::Cast(Scalar::F32),
                Op::Copy { depth: 0, cells: 1 },
                Op::Binary { op, at },
            ]
        }
        Act::Convert(ty) => vec![Op::Cast(ty)],
        Act::Intrinsic(name, args, returns) => {
            vec![call(operation(name), u32::from(args), u32::from(returns))]
        }
        // A listing passes a barrier each of its operand bytes.
        Act::Barrier => {
            let mut ops = Vec::new();
            for (field, value) in spec.fields.iter().zip(raw.fields) {
                let bytes = (0..field.1.bytes()).map(|byte| (value >> (8 * byte)) as u8);
                ops.extend(bytes.map(|byte| int(i32::from(byte))));
            }
            let args = ops.len() as u32;
            let name = format!("replication_barrier_{}", raw.opcode);
            ops.push(call(inline(&name, None), args, 0));
            ops
        }
        // The operand is the base-two logarithm of the block's alignment, which a listing prints whole.
        Act::StackAlloc => match n(0) {
            log2 @ 0..=30 => vec![
                int(1 << log2),
                call(inline("stack_alloc_zeroed", None), 2, 1),
            ],
            _ => return Err(malformed(at, "an alignment no 32-bit count holds")),
        },
        // The address is of a cell that holds the pointer the elements start at.
        Act::ElementAddress => {
            let (op, at) = (BinOp::Mul, Kind::U32);
            vec![
                Op::Cast(Scalar::U32),
                int(n(0) as i32),
                Op::Binary { op, at },
                Op::Swap,
                Op::Load {
                    ty: Scalar::U64,
                    offset: 0,
                },
                Op::Swap,
                Op::ByteOffset,
            ]
        }
        Act::StackAddress => vec![int(n(0) as i32), call(inline("stack_address", None), 1, 1)],
        // No program holds one, so nothing was held against the steps such a row would give.
        Act::TailCall | Act::Unwritten | Act::Unknown => {
            let what = format!("no steps are written for `{}`", spec.name);
            return Err(malformed(at, what));
        }
    };
    Ok(ops)
}
