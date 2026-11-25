//! The lifter: a compiled function's instructions run on a stack of expressions, to the blocks the
//! passes take, and the compiled half of the facade.
//!
//! What it makes is what a lifted listing reads as: `v_OFFSET_n` for a value an instruction
//! names, `stack_OFFSET_n` for a cell that crosses into the block at an offset, a block at each
//! offset control can enter, after one empty entry block.
//!
//! ⚠ **A cell stays an expression until a statement is written.** A step that only computes
//! pushes an expression and writes nothing; a step that writes a statement first gives a name to
//! every cell still pending that the statement could change, the deepest first. Which steps
//! write, and which cells are plain enough to stay, was measured against the listings and is
//! stated at `simple`, `called` and `floor_of_object` below.

use std::collections::{BTreeMap, BTreeSet};

use super::cfg::{self, Block, BlockId, Cfg, Terminator};
use super::disasm::{self, DisasmError, Insn};
use super::ir::{
    Callee, Expr, FunctionKey, Head, Kind, Returns, Scalar, Stmt, Var, VarInfo, Width,
};
use super::listing::Import;
use super::opcode::Op;
use super::program::{FunctionRow, Program, ProgramError};
use super::project::Project;
use super::tree;
use super::{Context, DecompileError, Names, Output, Stats};

/// The deepest an expression may nest before its function is refused: every walk of one recurses.
const NESTING: u32 = 256;

/// The most cells a call's result is tried at where no table states them.
const WIDEST: u32 = 40;

/// The walks a function is given to find the cells of such calls.
const WALKS: u32 = 4000;

/// The cells the walk that holds a choice of element cells to the compiler's rules may keep at once; past them the rules are not asked.
const MARKS: usize = 1 << 22;

/// The expression nodes a function may copy on its stack and hand to the arms of its branches before it is refused.
// Each copy of a cell that is not named doubles it: 40 of them in 166 bytes of code are a million million nodes.
const MOVED: u64 = 1 << 20;

/// Why a function did not lift.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LiftError {
    #[error(transparent)]
    Disasm(#[from] DisasmError),
    #[error("func_{function:06X}, instruction {at:06X}: {what}")]
    Stack {
        function: u32,
        at: u32,
        what: String,
    },
}

/// One function lifted, and the regions of it a fault leaves for a handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifted {
    pub function: cfg::Function,
    /// Empty where the program's tables protect none of the function.
    pub regions: Vec<cfg::Region>,
}

/// Lifts one function of a program to blocks, with the program's own tables alone.
pub fn function(program: &Program, row: &FunctionRow) -> Result<Lifted, LiftError> {
    function_in(program, Project::empty(), row)
}

/// Lifts one function of a program to blocks, taking from `project` what another program defines.
pub fn function_in(
    program: &Program,
    project: &Project,
    row: &FunctionRow,
) -> Result<Lifted, LiftError> {
    // A handler is entered by no jump, so the walk from the entry would leave it out unseen.
    let guarded = |h: &super::program::HandlerRow| {
        (h.from < row.end && row.at < h.to) || (row.at..row.end).contains(&h.handler)
    };
    if program.handlers.iter().any(guarded) {
        return Err(LiftError::Stack {
            function: row.at,
            at: row.at,
            what: "its exception regions are not lifted".to_owned(),
        });
    }
    let insns = disasm::function_in(program, project, row)?;
    let mut function = steps(program.program, row, &insns)?;
    function.head.name = name(program, row);
    Ok(Lifted {
        function,
        regions: Vec::new(),
    })
}

/// The code of a function its entry does not reach, padding left out: where it starts and its bytes.
// ⚠ A function only another program calls stands there unseen when its program is read alone:
// the walk from the entry leaves it out, and nothing of the function says so.
pub fn unreached(program: &Program, project: &Project, row: &FunctionRow) -> Option<(u32, u32)> {
    let insns = disasm::function_in(program, project, row).ok()?;
    let (reached, _) = sized(row.at, &insns).ok()?;
    let padding = |insn: &Insn| insn.ops.iter().all(|op| matches!(op, Op::Nop));
    let mut left = insns.iter().zip(&reached.depth);
    let (first, _) = left.find(|(insn, depth)| depth.is_none() && !padding(insn))?;
    Some((first.at, row.end.saturating_sub(first.at)))
}

/// Reads a compiled program and writes every function of it, by the level and the passes asked.
pub fn decompile(bytes: &[u8], cx: &Context<'_>) -> Result<Output, DecompileError> {
    let program = read(bytes, cx.project)?;
    Ok(functions(&program, &program.functions, cx))
}

/// Reads a compiled program, starting a function at every offset `project` holds one of it at.
// A function only another program calls is shown by nothing of its own program: without its
// start it reads as code after the function before it, which that function never reaches.
pub fn read(bytes: &[u8], project: &Project) -> Result<Program, ProgramError> {
    let mut program = Program::read(bytes)?;
    let own = |at: u32| FunctionKey::new(program.program, at);
    let known = project.signatures.range(own(0)..=own(u32::MAX));
    let known: Vec<u32> = known.map(|(key, _)| key.at()).collect();
    for at in known {
        // An offset that starts no instruction is of another build of the program: the project says nothing of this one there.
        let _ = program.add_function(at);
    }
    Ok(program)
}

/// Writes the functions `rows` of a program read, by the level and the passes asked; one that does not lift is written with no body and the reason.
pub fn functions(program: &Program, rows: &[FunctionRow], cx: &Context<'_>) -> Output {
    let imports = program
        .imports
        .iter()
        .zip(1..)
        .map(|(import, index)| Import {
            index,
            program: import.program,
            types: import.types.clone(),
        });
    let imports: Vec<Import> = imports.collect();
    let mut stats = Stats::default();
    let mut done = Vec::with_capacity(rows.len());
    for row in rows {
        stats.functions += 1;
        let lifted = match function_in(program, cx.project, row) {
            Ok(mut lifted) => {
                hashed(&mut lifted.function, cx.names);
                lifted
            }
            // One function that does not lift leaves the rest of the program to be written.
            Err(why) => {
                stats.incomplete += 1;
                let mut head = head(program.program, row);
                head.name = name(program, row);
                head.incomplete = Some(format!("not lifted: {why}"));
                done.push(tree::Function {
                    head,
                    body: Vec::new(),
                });
                continue;
            }
        };
        let head = &lifted.function.head;
        stats.statements_in += lifted.function.cfg.statements();
        stats.temporaries_in += head.vars.keys().filter(|v| v.is_temporary()).count();
        // No pass understands a fault's way to its handler: such a function is written as its blocks.
        done.push(match lifted.regions.is_empty() {
            true => super::blocks_in(lifted.function, cx, &mut stats),
            false => cfg::flat(lifted.function),
        });
    }
    let mut unit = super::passes::classes::unit(program.program, imports, done, cx);
    unit.front = Some(super::print::PROGRAM);
    let files = super::emit::unit(&unit, cx, &stats);
    Output { files, stats }
}

/// Gives each native the hash a supplied list states for the handler at its address, under the name a listing gives a hash, and each method of the runtime the path a supplied list states for it.
// The code calls a native by its handler's address and a method of the runtime by two numbers: without a list a call keeps them.
// ⚠ A listing names the runtime's methods itself, and the passes know one by its path: an equality of two handles is an operator only under its name.
fn hashed(f: &mut cfg::Function, names: &Names) {
    if names.native_hashes.is_empty() && names.builtins.is_empty() {
        return;
    }
    let mut known = |e: &mut Expr| match e {
        Expr::Call {
            callee: Callee::Native {
                name, hash, rva, ..
            },
            ..
        } => {
            if let Some(stated) = names.native_at(*rva).filter(|_| *hash == 0) {
                *hash = stated;
                *name = format!("_0x{stated:016X}");
            }
        }
        Expr::Call {
            callee: Callee::Builtin { path, ty, method },
            ..
        } => {
            if let Some(stated) = names.builtin(*ty, *method) {
                stated.clone_into(path);
            }
        }
        _ => {}
    };
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut |top| top.walk_mut(&mut known));
        }
        if let Some(e) = block.term.expr_mut() {
            e.walk_mut(&mut known);
        }
    }
}

/// The name a listing gives a function: the two the header names by theirs, any other by its offset.
fn name(program: &Program, row: &FunctionRow) -> String {
    match Some(row.at) {
        at if at == program.main => "main".to_owned(),
        at if at == program.initialize => "initialize".to_owned(),
        _ => format!("func_{:06X}", row.at),
    }
}

/// A function's head as the function table states it, with no variable yet.
fn head(program: u32, row: &FunctionRow) -> Head {
    Head {
        program,
        at: row.at,
        name: format!("func_{:06X}", row.at),
        returns: match row.returns {
            0 => Returns::Void,
            1 => Returns::Cell,
            _ => Returns::Cells,
        },
        params: row.params,
        // A frame has four local cells unasked, which a listing counts only where the code names them.
        locals: row.locals.saturating_sub(4),
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

/// Lifts a function's decoded instructions, which are in the order of their offsets.
fn steps(program: u32, row: &FunctionRow, insns: &[Insn]) -> Result<cfg::Function, LiftError> {
    let (reached, sizes) = sized(row.at, insns)?;
    let mut lifter = Lifter {
        function: row.at,
        insns,
        reached,
        sizes,
        vars: BTreeMap::new(),
        locals: row.locals.saturating_sub(4),
        moved: 0,
    };
    let mut drafts = Vec::new();
    for first in 0..insns.len() {
        // Code the entry does not reach is in no listing either: it is left out.
        if lifter.reached.leader[first] && lifter.reached.depth[first].is_some() {
            drafts.push(lifter.block(first)?);
        }
    }
    let mut head = head(program, row);
    head.vars = lifter.vars;
    // A listing counts the locals the code names, where that is more than the entry asks for.
    head.locals = lifter.locals;
    Ok(cfg::Function {
        head,
        cfg: assemble(row.at, drafts)?,
    })
}

/// What the walk from the entry found of each instruction.
struct Reached {
    /// The cells on the stack before the instruction; `None` for one the entry does not reach.
    depth: Vec<Option<u32>>,
    /// Whether a block starts at the instruction.
    leader: Vec<bool>,
}

/// The cells a call takes and the cells it yields, by the offset of each instruction whose call states neither.
type Sizes = BTreeMap<u32, (u32, u32)>;

/// Why a walk stopped.
enum Halt {
    Refused(LiftError),
    /// The instruction of this index holds a call whose cells are not chosen yet.
    Unsized(usize),
}

/// The walk of a function under the one choice of result cells, for the calls that state none, that lets the cells balance.
// Measured on the functions a listing sizes: where one choice alone balances, it is the listing's.
fn sized(function: u32, insns: &[Insn]) -> Result<(Reached, Sizes), LiftError> {
    let refuse = |at: u32, what: &str| LiftError::Stack {
        function,
        at,
        what: what.to_owned(),
    };
    let mut found: Option<(Reached, Sizes)> = None;
    let mut tried = vec![Sizes::new()];
    let mut walks = 0;
    while let Some(sizes) = tried.pop() {
        walks += 1;
        if walks > WALKS {
            return Err(refuse(function, "too many ways to size its calls"));
        }
        match reach(function, insns, &sizes) {
            // ⚠ Only ever turns a choice down: one the compiler's rules for an array's elements rule out.
            Ok(reached) if !sizes.is_empty() && !elements_hold(insns, &sizes, &reached) => {}
            Ok(_) if found.is_some() => {
                let what = "more than one count of cells for a call's result balances the stack";
                return Err(refuse(function, what));
            }
            Ok(reached) => found = Some((reached, sizes)),
            // With nothing chosen the refusal is the function's own.
            Err(Halt::Refused(why)) if sizes.is_empty() => return Err(why),
            Err(Halt::Refused(_)) => {}
            Err(Halt::Unsized(i)) => {
                let insn = &insns[i];
                let open = insn.ops.iter().find_map(|op| match op {
                    Op::Unsized {
                        callee,
                        args: Some(args),
                        returns: None,
                        ..
                    } => Some((callee, *args)),
                    _ => None,
                });
                let Some((callee, args)) = open else {
                    return Err(refuse(
                        insn.at,
                        "a call whose argument cells no table states",
                    ));
                };
                // A value read out of an array is a cell at the least; a function may yield none.
                let least = u32::from(matches!(callee, Callee::Intrinsic { .. }));
                for returns in (least..=WIDEST).rev() {
                    let mut with = sizes.clone();
                    with.insert(insn.at, (args, returns));
                    tried.push(with);
                }
            }
        }
    }
    let none = "no count of cells for a call's result balances the stack";
    found.ok_or_else(|| refuse(function, none))
}

/// Walks the instructions from the first, counting cells: every way into an instruction must find the same number.
fn reach(function: u32, insns: &[Insn], sizes: &Sizes) -> Result<Reached, Halt> {
    let refuse = |at: u32, what: String| Halt::Refused(LiftError::Stack { function, at, what });
    let Some(entry) = insns.first() else {
        return Err(refuse(
            function,
            "the function holds no instruction".to_owned(),
        ));
    };
    if entry.at != function {
        let what = "the first instruction is not at the function's offset".to_owned();
        return Err(refuse(entry.at, what));
    }
    let index = |to: u32, from: u32| {
        let found = insns.binary_search_by_key(&to, |insn| insn.at);
        found.map_err(|_| {
            refuse(
                from,
                format!("a jump to {to:06X}, where no instruction starts"),
            )
        })
    };
    let mut depth = vec![None; insns.len()];
    let mut leader = vec![false; insns.len()];
    depth[0] = Some(0);
    leader[0] = true;
    // An instruction enters the work list once, when its depth is first known.
    let mut work = vec![0usize];
    while let Some(i) = work.pop() {
        let insn = &insns[i];
        let mut cells = depth[i].unwrap_or(0);
        // Where control goes on: the instruction, the cells it finds, whether a jump names it.
        let mut next: Vec<(usize, u32, bool)> = Vec::new();
        let mut falls = true;
        let mut splits = false;
        for op in &insn.ops {
            let (pops, pushes) = match op {
                Op::Unsized { args, returns, .. } => {
                    match (args.zip(*returns), sizes.get(&insn.at)) {
                        (Some(stated), _) => stated,
                        (None, Some(chosen)) => *chosen,
                        (None, None) => return Err(Halt::Unsized(i)),
                    }
                }
                _ => op.stack(),
            };
            let needs = match op {
                Op::Copy {
                    depth: under,
                    cells: count,
                    ..
                } => under.saturating_add(*count),
                Op::AddrOfTop { cells: count, .. } => *count,
                _ => pops,
            };
            if cells < needs {
                let what = format!("a step takes {needs} cell(s) of a stack of {cells}");
                return Err(refuse(insn.at, what));
            }
            let Some(after) = (cells - pops).checked_add(pushes) else {
                return Err(refuse(insn.at, "more cells than a stack holds".to_owned()));
            };
            cells = after;
            match op {
                Op::Return { .. } if cells != 0 => {
                    let what = format!("a return leaves {cells} cell(s) on the stack");
                    return Err(refuse(insn.at, what));
                }
                Op::Jump { to, .. } | Op::Branch { to, .. } => {
                    next.push((index(*to, insn.at)?, cells, true));
                }
                Op::Switch { cases, default, .. } => {
                    for (_, to) in cases {
                        next.push((index(*to, insn.at)?, cells, true));
                    }
                    next.push((index(*default, insn.at)?, cells, true));
                }
                _ => {}
            }
            splits |= matches!(op, Op::Branch { .. });
            if op.leaves() {
                falls = false;
                break;
            }
        }
        if falls {
            if i + 1 >= insns.len() {
                let what = "control runs off the end of the function".to_owned();
                return Err(refuse(insn.at, what));
            }
            next.push((i + 1, cells, splits));
        }
        for (to, cells, named) in next {
            leader[to] |= named;
            match depth[to] {
                None => {
                    depth[to] = Some(cells);
                    work.push(to);
                }
                Some(had) if had != cells => {
                    let what = format!("reached with {had} cell(s) on the stack and with {cells}");
                    return Err(refuse(insns[to].at, what));
                }
                Some(_) => {}
            }
        }
    }
    Ok(Reached { depth, leader })
}

/// What the walk of [`elements_hold`] keeps of one cell of the stack.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Mark {
    /// Which cell of an array element's value this is: 0 for its first, and for a cell of nothing such.
    part: u32,
    /// The array the cell holds, where it is one the walk follows.
    leaf: Option<u32>,
}

/// What a cell followed as an array is read from: a variable, or eight bytes at an offset through another such cell.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Leaf {
    Var(Var),
    Field(u32, u32),
}

/// Whether a choice of cells for the array elements a function reads keeps to the compiler's two rules.
// No instruction takes, copies or moves a part of one element's value, and every element read
// from or stored to one array is as many cells. An array is told by what it is read from: a
// variable, or a field loaded through one.
// ⚠ Held against the functions a listing sizes, the rules turned down none of the listing's
// choices. Where the walk would keep more than `MARKS` cells the rules are not asked.
fn elements_hold(insns: &[Insn], sizes: &Sizes, reached: &Reached) -> bool {
    let kept = |sum: usize, depth: &Option<u32>| sum.saturating_add(depth.unwrap_or(0) as usize);
    if reached.depth.iter().fold(0usize, kept) > MARKS {
        return true;
    }
    let index = |to: u32| insns.binary_search_by_key(&to, |insn| insn.at).ok();
    let mut leaves: BTreeMap<Leaf, u32> = BTreeMap::new();
    let mut leaf = |of: Leaf| {
        let next = leaves.len() as u32;
        *leaves.entry(of).or_insert(next)
    };
    // The cells of the elements of each array followed.
    let mut widths: BTreeMap<u32, u32> = BTreeMap::new();
    let mut agrees = |array: Option<u32>, cells: u32| match array {
        Some(array) => *widths.entry(array).or_insert(cells) == cells,
        None => true,
    };
    // An instruction is queued once, so the stacks waiting are no more cells than were counted.
    let mut seen = vec![false; insns.len()];
    if let Some(first) = seen.first_mut() {
        *first = true;
    }
    let mut work: Vec<(usize, Vec<Mark>)> = vec![(0, Vec::new())];
    while let Some((i, mut stack)) = work.pop() {
        let Some(insn) = insns.get(i) else {
            continue;
        };
        // The deepest cell the instruction takes: the first of a value, or it takes a part of one.
        let mut lowest = (stack.len(), Mark::default());
        let mut next: Vec<usize> = Vec::new();
        let mut falls = true;
        for op in &insn.ops {
            let len = stack.len();
            let whole =
                |at: Option<usize>| at.and_then(|at| stack.get(at)).is_none_or(|m| m.part == 0);
            let under = |cells: u32| len.checked_sub(cells as usize);
            match op {
                Op::Copy { depth, cells } => {
                    let end = under(*depth);
                    let start = end.and_then(|end| end.checked_sub(*cells as usize));
                    if !whole(start) || (*depth > 0 && !whole(end)) {
                        return false;
                    }
                }
                Op::Swap if !(whole(under(1)) && whole(under(2))) => return false,
                Op::Sink { depth } if !(whole(under(1)) && whole(under(*depth))) => return false,
                Op::Raise { depth } => {
                    let from = under(*depth);
                    if !whole(from) || !whole(from.map(|from| from + 1)) {
                        return false;
                    }
                }
                Op::AddrOfTop { cells } if !whole(under(*cells)) => return false,
                _ => {}
            }
            if matches!(
                op,
                Op::Copy { .. } | Op::Swap | Op::Sink { .. } | Op::Raise { .. }
            ) {
                if shuffle(&mut stack, op).is_err() {
                    return true;
                }
                continue;
            }
            let (pops, pushes) = match op {
                Op::Unsized { args, returns, .. } => {
                    match (args.zip(*returns), sizes.get(&insn.at)) {
                        (Some(stated), _) => stated,
                        (None, Some(chosen)) => *chosen,
                        (None, None) => return true,
                    }
                }
                _ => op.stack(),
            };
            let Some(floor) = under(pops) else {
                return true;
            };
            if let (true, Some(mark)) = (floor < lowest.0, stack.get(floor)) {
                lowest = (floor, *mark);
            }
            let mut made = vec![Mark::default(); pushes as usize];
            match op {
                Op::Unsized {
                    callee: Callee::Intrinsic { name, .. },
                    ..
                } if name == "array_get_value" => {
                    if !agrees(stack.get(floor).and_then(|array| array.leaf), pushes) {
                        return false;
                    }
                    for (part, mark) in made.iter_mut().enumerate() {
                        mark.part = part as u32;
                    }
                }
                Op::Call {
                    callee: Callee::Intrinsic { name, .. },
                    args,
                    ..
                } if name == "array_set_slots" && *args >= 2 => {
                    if !agrees(stack.get(floor).and_then(|array| array.leaf), args - 2) {
                        return false;
                    }
                }
                Op::Push {
                    e: Expr::Var(var),
                    cells: 1,
                } => made[0].leaf = Some(leaf(Leaf::Var(*var))),
                Op::Load { ty, offset } if ty.bytes() == 8 => {
                    let through = stack.last().and_then(|address| address.leaf);
                    made[0].leaf = through.map(|through| leaf(Leaf::Field(through, *offset)));
                }
                Op::Cast(Scalar::I64 | Scalar::U64) => {
                    made[0].leaf = stack.last().and_then(|m| m.leaf)
                }
                _ => {}
            }
            stack.truncate(floor);
            stack.append(&mut made);
            match op {
                Op::Jump { to } | Op::Branch { to } => next.extend(index(*to)),
                Op::Switch { cases, default } => {
                    next.extend(cases.iter().filter_map(|(_, to)| index(*to)));
                    next.extend(index(*default));
                }
                _ => {}
            }
            if op.leaves() {
                falls = false;
                break;
            }
        }
        if lowest.1.part != 0 {
            return false;
        }
        if falls && i + 1 < insns.len() {
            next.push(i + 1);
        }
        next.retain(|to| !std::mem::replace(&mut seen[*to], true));
        while let Some(to) = next.pop() {
            match next.is_empty() {
                true => work.push((to, std::mem::take(&mut stack))),
                false => work.push((to, stack.clone())),
            }
        }
    }
    true
}

/// One cell of the evaluation stack: the expression that yields it, and how deep that nests.
#[derive(Clone)]
struct Cell {
    e: Expr,
    depth: u32,
}

/// A value read by its name, which nests nothing.
fn named(e: Expr) -> Cell {
    Cell { e, depth: 0 }
}

/// What a block holds while its instructions run.
struct Run {
    stack: Vec<Cell>,
    stmts: Vec<Stmt>,
    /// The index the next value named in the block takes.
    next: u32,
}

/// How control leaves a block, by the offsets of the blocks it goes to.
enum End {
    Jump(u32),
    /// Each arm with the statements that hand it the cells left on the stack.
    Branch {
        cond: Expr,
        then: (u32, Vec<Stmt>),
        els: (u32, Vec<Stmt>),
    },
    /// `left` is the cells left on the stack, which every case hands to its block as an arm of a branch does.
    Switch {
        value: Expr,
        cases: Vec<(i64, u32)>,
        default: u32,
        left: Vec<Cell>,
    },
    Return(Option<Expr>),
    Unreachable,
}

/// A block before block numbers exist.
struct Draft {
    at: u32,
    stmts: Vec<Stmt>,
    end: End,
}

struct Lifter<'a> {
    function: u32,
    insns: &'a [Insn],
    reached: Reached,
    sizes: Sizes,
    vars: BTreeMap<Var, VarInfo>,
    /// One past the highest local cell the code names, and no less than the entry asks for.
    locals: u32,
    /// The expression nodes copied and handed on so far, held to [`MOVED`].
    moved: u64,
}

impl Lifter<'_> {
    fn refuse(&self, at: u32, what: &str) -> LiftError {
        LiftError::Stack {
            function: self.function,
            at,
            what: what.to_owned(),
        }
    }

    /// Counts the nodes of `cells`, each written `times` over, towards what the function may copy.
    fn charge(&mut self, at: u32, cells: &[Cell], times: u64) -> Result<(), LiftError> {
        let mut nodes = 0u64;
        for cell in cells {
            cell.e.walk(&mut |_| nodes += 1);
        }
        self.moved = self.moved.saturating_add(nodes.saturating_mul(times));
        match self.moved > MOVED {
            true => Err(self.refuse(at, "it copies more of its stack than the lifter follows")),
            false => Ok(()),
        }
    }

    /// Runs the instructions from the one that starts a block to the one that ends it.
    fn block(&mut self, first: usize) -> Result<Draft, LiftError> {
        let insns = self.insns;
        let at = insns[first].at;
        let carried_in = self.reached.depth[first].unwrap_or(0);
        let mut run = Run {
            stack: Vec::new(),
            stmts: Vec::new(),
            next: 0,
        };
        for index in 0..carried_in {
            let var = Var::Stack { at, index };
            self.vars.insert(var, VarInfo::new(Width::Cell));
            run.stack.push(named(Expr::Var(var)));
        }
        for (i, insn) in insns.iter().enumerate().skip(first) {
            let next = insns.get(i + 1);
            if let Some(end) = self.insn(insn, next.map(|insn| insn.at), &mut run)? {
                let stmts = run.stmts;
                return Ok(Draft { at, stmts, end });
            }
            // The instruction runs into the next one, which another way in makes a block of its own.
            match next {
                Some(next) if self.reached.leader[i + 1] => {
                    self.charge(next.at, &run.stack, 1)?;
                    run.stmts.extend(carried(&run.stack, next.at));
                    return Ok(Draft {
                        at,
                        stmts: run.stmts,
                        end: End::Jump(next.at),
                    });
                }
                Some(_) => {}
                None => break,
            }
        }
        Err(self.refuse(at, "control runs off the end of the function"))
    }

    /// The name the next value of the block takes, declared.
    fn name(&mut self, run: &mut Run, at: u32, width: Width) -> Var {
        let var = Var::Value {
            at,
            index: run.next,
        };
        run.next += 1;
        self.vars.insert(var, VarInfo::new(width));
        var
    }

    /// Writes `value` to a new name of the block.
    fn assign(&mut self, run: &mut Run, at: u32, width: Width, value: Expr) -> Var {
        let target = self.name(run, at, width);
        run.stmts.push(Stmt::Assign { target, value });
        target
    }

    /// Names every cell below `floor` that is not plain, the deepest first: a statement is about to be written.
    fn spill_below(&mut self, run: &mut Run, at: u32, floor: usize) {
        for i in 0..floor.min(run.stack.len()) {
            if simple(&run.stack[i].e) {
                continue;
            }
            let target = self.name(run, at, Width::Cell);
            let value = std::mem::replace(&mut run.stack[i], named(Expr::Var(target))).e;
            run.stmts.push(Stmt::Assign { target, value });
        }
    }

    fn spill(&mut self, run: &mut Run, at: u32) {
        let all = run.stack.len();
        self.spill_below(run, at, all);
    }

    fn pop(&self, run: &mut Run, at: u32) -> Result<Cell, LiftError> {
        let cell = run.stack.pop();
        cell.ok_or_else(|| self.refuse(at, "a step takes a cell of an empty stack"))
    }

    /// The `count` top cells, the deepest first.
    fn pop_many(&self, run: &mut Run, at: u32, count: u32) -> Result<Vec<Cell>, LiftError> {
        let from = run.stack.len().checked_sub(count as usize);
        let from =
            from.ok_or_else(|| self.refuse(at, "a step takes more cells than the stack holds"))?;
        Ok(run.stack.split_off(from))
    }

    /// Pushes what a step computed from cells that nested `under` deep.
    fn push(&self, run: &mut Run, at: u32, e: Expr, under: u32) -> Result<(), LiftError> {
        if under >= NESTING {
            return Err(self.refuse(at, "an expression nests deeper than the lifter follows"));
        }
        run.stack.push(Cell {
            e,
            depth: under + 1,
        });
        Ok(())
    }

    /// Pushes the `cells` cells of a value the block has named.
    fn push_cells(&self, run: &mut Run, var: Var, cells: u32) {
        for index in 0..cells {
            let base = Box::new(Expr::Var(var));
            run.stack.push(named(Expr::Cell { base, index }));
        }
    }

    /// Counts the locals an expression of a step names.
    fn count(&mut self, e: &Expr) {
        let mut locals = self.locals;
        e.walk(&mut |node| {
            if let Expr::Var(Var::Local(n))
            | Expr::AddrOf {
                var: Var::Local(n), ..
            } = node
            {
                locals = locals.max(n.saturating_add(1));
            }
        });
        self.locals = locals;
    }

    /// Writes a call that takes `args` cells and yields `returns`.
    fn call(
        &mut self,
        run: &mut Run,
        at: u32,
        callee: &Callee,
        args: u32,
        returns: u32,
    ) -> Result<(), LiftError> {
        let taken = self.pop_many(run, at, args)?;
        let under = taken.iter().map(|cell| cell.depth).max().unwrap_or(0);
        let (names, folds) = called(callee, returns);
        if names {
            self.spill(run, at);
        }
        let call = Expr::Call {
            callee: callee.clone(),
            args: taken.into_iter().map(|cell| cell.e).collect(),
            // A listing states the cells of a call into its own program at the callee's head only.
            returns: (!matches!(callee, Callee::Local { .. })).then_some(returns),
        };
        match returns {
            0 => run.stmts.push(Stmt::Eval(call)),
            1 if folds => self.push(run, at, call, under)?,
            1 => {
                let var = self.assign(run, at, Width::Cell, call);
                run.stack.push(named(Expr::Var(var)));
            }
            cells => {
                let var = self.assign(run, at, Width::Cells, call);
                self.push_cells(run, var, cells);
            }
        }
        Ok(())
    }

    /// Runs one instruction's steps; `Some` when control leaves the block in it.
    // The last arm is there for a step the decoder gains before the lifter does.
    #[allow(unreachable_patterns)]
    fn insn(
        &mut self,
        insn: &Insn,
        next: Option<u32>,
        run: &mut Run,
    ) -> Result<Option<End>, LiftError> {
        let at = insn.at;
        let ops = &insn.ops;
        let mut k = 0;
        while let Some(op) = ops.get(k) {
            k += 1;
            match op {
                Op::Nop => {}
                Op::Push { e, cells, .. } => {
                    let e = e.clone();
                    self.count(&e);
                    match e {
                        Expr::NewObject(_) if *cells == 1 => {
                            if let Some(floor) = floor_of_object(run.stack.len(), &ops[k..]) {
                                self.spill_below(run, at, floor);
                            }
                            let var = self.assign(run, at, Width::Cell, e);
                            run.stack.push(named(Expr::Var(var)));
                        }
                        Expr::ZeroSlots(_)
                        | Expr::Vector3(_)
                        | Expr::LoadBytes { .. }
                        | Expr::ExtractBytes { .. }
                        | Expr::Cells(_) => {
                            let var = self.assign(run, at, Width::Cells, e);
                            self.push_cells(run, var, *cells);
                        }
                        Expr::GuidEmpty | Expr::BuiltinGlobal(_) => {
                            for index in 0..*cells {
                                let base = Box::new(e.clone());
                                self.push(run, at, Expr::Cell { base, index }, 0)?;
                            }
                        }
                        e if *cells == 1 => self.push(run, at, e, 0)?,
                        _ => return Err(self.refuse(at, "a push of several cells of one")),
                    }
                }
                Op::Set(_) => {
                    // The stores of one instruction: the cell on top goes to the first one named.
                    let mut stores: Vec<(Var, Expr)> = Vec::new();
                    k -= 1;
                    while let Some(Op::Set(var)) = ops.get(k) {
                        k += 1;
                        if let Var::Local(n) = *var {
                            self.locals = self.locals.max(n.saturating_add(1));
                        }
                        stores.push((*var, self.pop(run, at)?.e));
                    }
                    self.spill(run, at);
                    stores.reverse();
                    run.stmts.extend(stored(stores));
                }
                Op::Drop { .. }
                | Op::Copy { .. }
                | Op::Swap
                | Op::Sink { .. }
                | Op::Raise { .. } => {
                    let held = run.stack.len();
                    let moved = shuffle(&mut run.stack, op);
                    moved.map_err(|()| self.refuse(at, "a step reaches below the stack"))?;
                    // What a copy added lies above what was held.
                    self.charge(at, run.stack.get(held..).unwrap_or_default(), 1)?;
                }
                Op::Cast(to) => {
                    let x = self.pop(run, at)?;
                    self.push(run, at, Expr::cast(*to, x.e), x.depth)?;
                }
                Op::Truth => {
                    let x = self.pop(run, at)?;
                    self.push(run, at, Expr::Truth(Box::new(x.e)), x.depth)?;
                }
                Op::Unary(op) => {
                    let x = self.pop(run, at)?;
                    let e = Box::new(x.e);
                    self.push(run, at, Expr::Unary { op: *op, e }, x.depth)?;
                }
                Op::Binary { op, at: kind, .. } => {
                    let r = self.pop(run, at)?;
                    let l = self.pop(run, at)?;
                    let e = Expr::Binary {
                        op: *op,
                        at: kind.clone(),
                        l: Box::new(l.e),
                        r: Box::new(r.e),
                    };
                    self.push(run, at, e, l.depth.max(r.depth))?;
                }
                Op::Select => {
                    let els = self.pop(run, at)?;
                    let then = self.pop(run, at)?;
                    let cond = self.pop(run, at)?;
                    let under = cond.depth.max(then.depth).max(els.depth);
                    let e = Expr::Select {
                        cond: Box::new(cond.e),
                        then: Box::new(then.e),
                        els: Box::new(els.e),
                    };
                    self.push(run, at, e, under)?;
                }
                Op::Load { ty, offset, .. } => {
                    let addr = self.pop(run, at)?;
                    let e = Expr::Load {
                        ty: *ty,
                        addr: Box::new(addr.e),
                        offset: *offset,
                    };
                    self.push(run, at, e, addr.depth)?;
                }
                Op::LoadBytes { offset, bytes, .. } => {
                    let addr = Box::new(self.pop(run, at)?.e);
                    let (offset, bytes) = (*offset, *bytes);
                    let e = Expr::LoadBytes {
                        addr,
                        offset,
                        bytes,
                    };
                    let var = self.assign(run, at, Width::Cells, e);
                    self.push_cells(run, var, bytes.div_ceil(8));
                }
                // A whole cell taken out of a value is that cell, which a listing reads by itself and names nothing for.
                Op::ExtractBytes {
                    cells,
                    offset,
                    bytes: 8,
                } if offset % 8 == 0 && offset / 8 < *cells => {
                    let mut taken = self.pop_many(run, at, *cells)?;
                    run.stack.push(taken.swap_remove((offset / 8) as usize));
                }
                Op::ExtractBytes {
                    cells,
                    offset,
                    bytes,
                    ..
                } => {
                    let taken = self.pop_many(run, at, *cells)?;
                    let value = Expr::Cells(taken.into_iter().map(|cell| cell.e).collect());
                    let e = Expr::ExtractBytes {
                        value: Box::new(value),
                        offset: *offset,
                        bytes: *bytes,
                    };
                    let var = self.assign(run, at, Width::Cells, e);
                    // One cell taken out is read through the cast that follows, with no index.
                    match bytes.div_ceil(8) {
                        1 => run.stack.push(named(Expr::Var(var))),
                        cells => self.push_cells(run, var, cells),
                    }
                }
                Op::ByteOffset => {
                    let offset = self.pop(run, at)?;
                    let addr = self.pop(run, at)?;
                    let e = Expr::ByteOffset {
                        addr: Box::new(addr.e),
                        offset: Box::new(offset.e),
                    };
                    self.push(run, at, e, addr.depth.max(offset.depth))?;
                }
                Op::Vector3 => {
                    let taken = self.pop_many(run, at, 3)?;
                    let parts: Vec<Expr> = taken.into_iter().map(|cell| cell.e).collect();
                    let Ok(parts) = <[Expr; 3]>::try_from(parts) else {
                        return Err(self.refuse(at, "a vector of other than three values"));
                    };
                    let e = Expr::Vector3(Box::new(parts));
                    let var = self.assign(run, at, Width::Cells, e);
                    self.push_cells(run, var, 2);
                }
                Op::NewArray { ty, wrapper, .. } => {
                    let len = Box::new(self.pop(run, at)?.e);
                    self.spill(run, at);
                    let e = Expr::NewArray {
                        ty: ty.clone(),
                        len,
                        wrapper: wrapper.clone(),
                    };
                    let var = self.assign(run, at, Width::Cell, e);
                    run.stack.push(named(Expr::Var(var)));
                }
                Op::VirtualFunctionRef { ty, slot, .. } => {
                    let receiver = self.pop(run, at)?;
                    let e = Expr::VirtualFunctionRef {
                        receiver: Box::new(receiver.e),
                        ty: ty.clone(),
                        slot: *slot,
                    };
                    self.push(run, at, e, receiver.depth)?;
                }
                Op::Call {
                    callee,
                    args,
                    returns,
                    ..
                } => self.call(run, at, callee, *args, *returns)?,
                Op::Unsized {
                    callee,
                    args,
                    returns,
                    ..
                } => {
                    let stated = args.zip(*returns);
                    let Some((args, returns)) = stated.or(self.sizes.get(&at).copied()) else {
                        return Err(self.refuse(at, "a call whose cells were not found"));
                    };
                    self.call(run, at, callee, args, returns)?;
                }
                Op::AddrOfTop { cells, .. } => {
                    let from = run.stack.len().checked_sub(*cells as usize);
                    let Some(var) = from.and_then(|from| value_of(&run.stack[from..])) else {
                        return Err(self.refuse(at, "the address of cells that are no one value's"));
                    };
                    run.stack.push(named(Expr::AddrOf { var, cell: 0 }));
                }
                Op::Store { kind, offset, .. } => {
                    let value = self.pop(run, at)?.e;
                    let addr = self.pop(run, at)?.e;
                    self.spill(run, at);
                    run.stmts.push(Stmt::Store {
                        kind: *kind,
                        addr,
                        offset: *offset,
                        value,
                    });
                }
                Op::StoreBytes {
                    offset,
                    bytes,
                    cells,
                    ..
                } => {
                    let taken = self.pop_many(run, at, *cells)?;
                    let value = Expr::Cells(taken.into_iter().map(|cell| cell.e).collect());
                    let addr = self.pop(run, at)?.e;
                    self.spill(run, at);
                    run.stmts.push(Stmt::StoreBytes {
                        addr,
                        offset: *offset,
                        bytes: *bytes,
                        value,
                    });
                }
                Op::StoreF32Slots { offset, cells, .. } => {
                    let taken = self.pop_many(run, at, *cells)?;
                    let value = Expr::Cells(taken.into_iter().map(|cell| cell.e).collect());
                    let addr = self.pop(run, at)?.e;
                    self.spill(run, at);
                    run.stmts.push(Stmt::StoreF32Slots {
                        addr,
                        offset: *offset,
                        value,
                    });
                }
                Op::ZeroBytes { bytes, .. } => {
                    let addr = self.pop(run, at)?.e;
                    self.spill(run, at);
                    run.stmts.push(Stmt::ZeroBytes {
                        addr,
                        bytes: *bytes,
                    });
                }
                Op::Step { offset, down, .. } => {
                    let addr = self.pop(run, at)?.e;
                    self.spill(run, at);
                    run.stmts.push(Stmt::StepI32 {
                        addr,
                        offset: *offset,
                        down: *down,
                    });
                }
                Op::Jump { to, .. } => {
                    self.charge(at, &run.stack, 1)?;
                    run.stmts.extend(carried(&run.stack, *to));
                    return Ok(Some(End::Jump(*to)));
                }
                Op::Branch { to, .. } => {
                    let cond = self.pop(run, at)?.e;
                    // The walk gave the cells of the instruction's end to the next one.
                    let (true, Some(next)) = (k == ops.len(), next) else {
                        return Err(self.refuse(at, "a branch that does not end its instruction"));
                    };
                    self.charge(at, &run.stack, 2)?;
                    let arm = |to: u32| (to, carried(&run.stack, to).into_iter().collect());
                    return Ok(Some(End::Branch {
                        cond,
                        then: arm(*to),
                        els: arm(next),
                    }));
                }
                Op::Switch { cases, default, .. } => {
                    let value = self.pop(run, at)?.e;
                    // Each target is handed the cells left, and no two cases need go to one.
                    self.charge(at, &run.stack, cases.len() as u64 + 1)?;
                    return Ok(Some(End::Switch {
                        value,
                        cases: cases.clone(),
                        default: *default,
                        left: run.stack.clone(),
                    }));
                }
                Op::Return { cells, .. } => {
                    let value = match *cells {
                        0 => None,
                        1 => Some(self.pop(run, at)?.e),
                        cells => {
                            let taken = self.pop_many(run, at, cells)?;
                            Some(Expr::Cells(taken.into_iter().map(|cell| cell.e).collect()))
                        }
                    };
                    return Ok(Some(End::Return(value)));
                }
                Op::Unreachable => return Ok(Some(End::Unreachable)),
                _ => return Err(self.refuse(at, "a step the lifter does not know")),
            }
        }
        Ok(None)
    }
}

/// Whether a pending cell is left as it is when a statement is written before its use.
// Measured: a listing names every other cell, a parameter or a local read included, and leaves a
// constant, an address, and a value or a carried cell read by name.
fn simple(e: &Expr) -> bool {
    match e {
        // A float no literal spells is written `float_bits(..)`, which a listing names like a call.
        Expr::Const(c) => c.kind != Kind::F32 || c.as_f32().is_finite(),
        Expr::Var(var) => var.is_temporary(),
        Expr::Cell { base, .. } => matches!(**base, Expr::Var(var) if var.is_temporary()),
        Expr::AddrOf { .. }
        | Expr::FunctionRef { .. }
        | Expr::BuiltinFunctionRef { .. }
        | Expr::StringConstant { .. } => true,
        _ => false,
    }
}

/// Whether a call names the cells left pending before it runs, and whether its one result stays inside the expression that uses it.
// Measured per kind of call. A function, a native, a builtin and a virtual call name them and
// name their result. Of the runtime's own operations only a statement, a box, a delegate's call
// and an array's value read name them; the barrier before a replicated store names nothing.
fn called(callee: &Callee, returns: u32) -> (bool, bool) {
    let Callee::Intrinsic { name, .. } = callee else {
        return (true, false);
    };
    if name.starts_with("replication_barrier") {
        return (false, false);
    }
    let heap = name.starts_with("box<") || name == "invoke_delegate" || name == "array_get_value";
    let kept = heap || name == "box_at" || name == "stack_alloc_zeroed";
    (returns == 0 || heap, returns == 1 && !kept)
}

/// The stack position below which the cells are named before a new object is: under its deepest copy, once the shuffles that follow it in its instruction have run.
// `None` where nothing copies or moves the object: one pushed alone names no cell. Where it is
// put under its constructor's arguments, those stay pending and the cells under it are named.
fn floor_of_object(len: usize, rest: &[Op]) -> Option<usize> {
    let mut marks: Vec<Option<usize>> = (0..len).map(Some).collect();
    marks.push(None);
    let mut moved = false;
    for op in rest {
        match shuffle(&mut marks, op) {
            Ok(true) => moved = true,
            _ => break,
        }
    }
    let floor = marks.iter().position(Option::is_none)?;
    let mut below = marks.iter().take(floor).enumerate();
    let untouched = below.all(|(i, mark)| *mark == Some(i));
    (moved && untouched).then_some(floor)
}

/// The value a block named whose cells these are, in order from its first.
fn value_of(cells: &[Cell]) -> Option<Var> {
    let owner = |(n, cell): (usize, &Cell)| match &cell.e {
        Expr::Cell { base, index } if *index as usize == n => match **base {
            Expr::Var(var @ Var::Value { .. }) => Some(var),
            _ => None,
        },
        _ => None,
    };
    let mut owners = cells.iter().enumerate().map(owner);
    let first = owners.next()??;
    owners.all(|other| other == Some(first)).then_some(first)
}

/// Applies a step that only moves cells; `Ok(false)` for any other step, `Err` where the stack is too short.
fn shuffle<T: Clone>(stack: &mut Vec<T>, op: &Op) -> Result<bool, ()> {
    let len = stack.len();
    match op {
        Op::Drop { cells, .. } => stack.truncate(len.checked_sub(*cells as usize).ok_or(())?),
        Op::Copy { depth, cells, .. } => {
            let end = len.checked_sub(*depth as usize).ok_or(())?;
            let start = end.checked_sub(*cells as usize).ok_or(())?;
            stack.extend_from_within(start..end);
        }
        Op::Sink { depth, .. } => {
            let to = len.checked_sub(*depth as usize).filter(|_| *depth > 0);
            let top = stack.pop().ok_or(())?;
            stack.insert(to.ok_or(())?, top);
        }
        Op::Raise { depth, .. } => {
            let from = len.checked_sub(*depth as usize).filter(|_| *depth > 0);
            let cell = stack.remove(from.ok_or(())?);
            stack.push(cell);
        }
        Op::Swap if len >= 2 => stack.swap(len - 1, len - 2),
        Op::Swap => return Err(()),
        _ => return Ok(false),
    }
    Ok(true)
}

/// The statements of one instruction's stores, the deepest cell's first as a listing has them.
// ⚠ The machine read every value before it wrote any. One after another is the same only while
// no later value could see an earlier store: each is then a constant, a name, or a variable no
// earlier store wrote. Otherwise the stores stay one statement that reads first.
fn stored(stores: Vec<(Var, Expr)>) -> Vec<Stmt> {
    let apart = stores.iter().enumerate().skip(1).all(|(i, (_, value))| {
        let earlier = |var: &Var| stores.iter().take(i).any(|(target, _)| target == var);
        match value {
            Expr::Var(var) => !earlier(var),
            other => simple(other),
        }
    });
    if apart {
        let one = |(target, value): (Var, Expr)| Stmt::Assign { target, value };
        return stores.into_iter().map(one).collect();
    }
    let (targets, values) = stores.into_iter().unzip();
    vec![Stmt::AssignMany { targets, values }]
}

/// The statement that hands the cells left on the stack to the block at `to`, where any are left.
fn carried(stack: &[Cell], to: u32) -> Option<Stmt> {
    let mut values: Vec<Expr> = stack.iter().map(|cell| cell.e.clone()).collect();
    let targets = (0u32..).map(|index| Var::Stack { at: to, index });
    let targets: Vec<Var> = targets.take(values.len()).collect();
    match (targets.first().copied(), values.len()) {
        (None, _) => None,
        (Some(target), 1) => values.pop().map(|value| Stmt::Assign { target, value }),
        (Some(_), _) => Some(Stmt::AssignMany { targets, values }),
    }
}

/// Numbers the blocks as lowering a flat listing does: an empty entry, each block at its offset, and an arm of a branch or of a switch that hands cells on right after the block that branches.
fn assemble(function: u32, drafts: Vec<Draft>) -> Result<Cfg, LiftError> {
    let mut ids: BTreeMap<u32, BlockId> = BTreeMap::new();
    let mut next: BlockId = 1;
    for draft in &drafts {
        ids.insert(draft.at, next);
        next += 1;
        match &draft.end {
            End::Branch { then, els, .. } => {
                next += usize::from(!then.1.is_empty()) + usize::from(!els.1.is_empty());
            }
            // A block a target that is handed cells.
            End::Switch {
                cases,
                default,
                left,
                ..
            } if !left.is_empty() => {
                let targets = cases.iter().map(|(_, to)| *to).chain([*default]);
                next += targets.collect::<BTreeSet<u32>>().len();
            }
            _ => {}
        }
    }
    let id = |at: u32| {
        ids.get(&at).copied().ok_or_else(|| LiftError::Stack {
            function,
            at,
            what: "a jump to a block that was not lifted".to_owned(),
        })
    };
    let mut blocks = vec![Block {
        at: None,
        stmts: Vec::new(),
        term: Terminator::Jump(1),
    }];
    for draft in drafts {
        let own = blocks.len();
        let mut arms: Vec<Block> = Vec::new();
        let term = match draft.end {
            End::Jump(to) => Terminator::Jump(id(to)?),
            End::Branch { cond, then, els } => {
                let mut ends = [0; 2];
                for (end, (to, stmts)) in ends.iter_mut().zip([then, els]) {
                    let to = id(to)?;
                    *end = match stmts.is_empty() {
                        true => to,
                        false => {
                            arms.push(Block {
                                at: None,
                                stmts,
                                term: Terminator::Jump(to),
                            });
                            own + arms.len()
                        }
                    };
                }
                let [then, els] = ends;
                Terminator::Branch { cond, then, els }
            }
            End::Switch {
                value,
                cases,
                default,
                left,
            } => {
                // The block each target is reached through: its own, or one that first hands it the cells left.
                let mut through: BTreeMap<u32, BlockId> = BTreeMap::new();
                let mut reach = |to: u32, arms: &mut Vec<Block>| -> Result<BlockId, LiftError> {
                    if let Some(known) = through.get(&to) {
                        return Ok(*known);
                    }
                    let block = id(to)?;
                    let via = match carried(&left, to) {
                        None => block,
                        Some(hand) => {
                            arms.push(Block {
                                at: None,
                                stmts: vec![hand],
                                term: Terminator::Jump(block),
                            });
                            own + arms.len()
                        }
                    };
                    through.insert(to, via);
                    Ok(via)
                };
                let mut table = Vec::with_capacity(cases.len());
                for (case, to) in cases {
                    table.push((case, reach(to, &mut arms)?));
                }
                let default = reach(default, &mut arms)?;
                Terminator::Switch {
                    value,
                    cases: table,
                    default,
                }
            }
            End::Return(value) => Terminator::Return(value),
            End::Unreachable => Terminator::Unreachable,
        };
        blocks.push(Block {
            at: Some(draft.at),
            stmts: draft.stmts,
            term,
        });
        blocks.append(&mut arms);
    }
    Ok(Cfg { blocks })
}

#[cfg(test)]
#[path = "lift_tests.rs"]
mod tests;
