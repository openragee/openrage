//! Aggregates: a value of several cells is one variable, as a parameter too.
//!
//! Works on the blocks, after `interproc` and before `types`. A run of parameters or locals is
//! one variable where the project's signatures or the code's own whole writes say it holds one
//! value and no statement assigns one of its cells alone; its first cell's `VarInfo` then says
//! `Width::Cells` and a kind that states the cells, and no later cell of the run is named again.
//! A run of locals the code only ever uses as one value is one too, while nothing states
//! otherwise; a row of statements that assigns every cell of a run in order is one assignment,
//! and bytes taken out of a run are a load through its address where that address is taken already.
//! Bytes taken out of a group of one item, or out of bytes they lie within, are bytes of that value.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg;
use crate::gta6::script::ir::{
    Callee, Expr, FunctionKey, Head, Kind, Scalar, Stmt, Var, VarInfo, Width,
};
use crate::gta6::script::project::{Project, Signature};
use crate::gta6::script::{Context, Pass, Stats};

#[path = "aggregates_kinds.rs"]
mod kinds;
pub use kinds::copied_kinds;

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Aggregates];

/// Groups of cells nested deeper than this state no count of cells.
const DEPTH: u32 = 32;
/// The `byte_offset`s under one access that are followed to a frame cell's own address.
const NESTED: usize = 8;

/// A run of frame cells: `cells` of the parameters, or of the locals, from `first` on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    params: bool,
    first: u32,
    cells: u32,
}

impl Run {
    fn var(self) -> Var {
        frame_var(self.params, self.first)
    }

    fn key(self) -> (bool, u32) {
        (self.params, self.first)
    }

    /// One past the run's last cell.
    fn end(self) -> u64 {
        u64::from(self.first) + u64::from(self.cells)
    }
}

/// The runs made one variable each, by frame and first cell, with their cells; no two overlap.
type Runs = BTreeMap<(bool, u32), u32>;

fn frame_var(params: bool, n: u32) -> Var {
    match params {
        true => Var::Param(n),
        false => Var::Local(n),
    }
}

/// Whether a variable is a parameter, and its cell of the frame; `None` for a temporary and a global.
fn frame_cell(var: Var) -> Option<(bool, u32)> {
    match var {
        Var::Param(n) => Some((true, n)),
        Var::Local(n) => Some((false, n)),
        Var::Value { .. } | Var::Stack { .. } | Var::Global { .. } => None,
    }
}

/// The run `var` is a cell of, and which cell.
fn run_of(runs: &Runs, var: Var) -> Option<(Run, u32)> {
    let (params, n) = frame_cell(var)?;
    let (&(space, first), &cells) = runs.range(..=(params, n)).next_back()?;
    let run = Run {
        params,
        first,
        cells,
    };
    if space != params || u64::from(n) >= run.end() {
        return None;
    }
    Some((run, n - first))
}

/// The cells a kind states a value of it takes, where it states them.
fn kind_cells(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::Vector | Kind::Handle => Some(2),
        Kind::Aggregate(cells) | Kind::Value { cells, .. } => Some(*cells),
        _ => None,
    }
}

/// The cells an expression yields, where its form says; `None` where nothing states them.
fn cells_of(head: &Head, widths: &BTreeMap<Var, u32>, e: &Expr, depth: u32) -> Option<u32> {
    let deeper = depth.checked_sub(1)?;
    match e {
        Expr::ZeroSlots(cells) => Some(*cells),
        Expr::Call { returns, .. } => *returns,
        Expr::LoadBytes { bytes, .. } | Expr::ExtractBytes { bytes, .. } => Some(bytes.div_ceil(8)),
        Expr::Vector3(_) | Expr::GuidEmpty => Some(2),
        Expr::BuiltinGlobal(_) => None,
        Expr::Var(var) => match head.vars.get(var) {
            Some(info) if info.width == Width::Cells => match var.is_temporary() {
                // A temporary assigned in several arms states its cells by the kind declared of it.
                true => widths.get(var).copied().or(match info.kind {
                    Kind::Aggregate(cells) => Some(cells),
                    _ => None,
                }),
                false => kind_cells(&info.kind),
            },
            _ => Some(1),
        },
        Expr::Cells(items) => items.iter().try_fold(0u32, |sum, item| {
            sum.checked_add(cells_of(head, widths, item, deeper)?)
        }),
        Expr::Sugar { e: inner, .. } => cells_of(head, widths, inner, deeper),
        Expr::Select { then, els, .. } => {
            let cells = cells_of(head, widths, then, deeper)?;
            (cells_of(head, widths, els, deeper) == Some(cells)).then_some(cells)
        }
        _ => Some(1),
    }
}

/// The cells of each temporary of several cells: one assignment, by a value whose form states them.
fn widths(f: &cfg::Function) -> BTreeMap<Var, u32> {
    let stmts = || f.cfg.blocks.iter().flat_map(|block| &block.stmts);
    let mut defs: BTreeMap<Var, u32> = BTreeMap::new();
    for stmt in stmts() {
        for target in stmt.targets().iter().filter(|var| var.is_temporary()) {
            *defs.entry(*target).or_default() += 1;
        }
    }
    let mut widths = BTreeMap::new();
    for stmt in stmts() {
        let Stmt::Assign { target, value } = stmt else {
            continue;
        };
        let many = (f.head.vars.get(target)).is_some_and(|info| info.width == Width::Cells);
        if target.is_temporary() && many && defs.get(target) == Some(&1) {
            if let Some(cells) = cells_of(&f.head, &widths, value, DEPTH) {
                widths.insert(*target, cells);
            }
        }
    }
    widths
}

/// Calls `f` on every expression at the top of a statement or of a terminator.
fn each_top<'a>(cfg: &'a cfg::Cfg, f: &mut impl FnMut(&'a Expr)) {
    for block in &cfg.blocks {
        for stmt in &block.stmts {
            stmt.each_expr(f);
        }
        if let Some(e) = block.term.expr() {
            f(e);
        }
    }
}

/// The runs something says hold one value each, and those two sayings disagree on.
#[derive(Default)]
struct Wanted {
    runs: BTreeMap<(bool, u32), (u32, Kind)>,
    refused: BTreeSet<(bool, u32)>,
    /// The runs only the code's own use of the cells suggests.
    weak: BTreeSet<(bool, u32)>,
    /// Those of them only a copy of another run suggests: one stands while the run copied does.
    tied: BTreeSet<(bool, u32)>,
}

impl Wanted {
    /// Notes that `cells` cells from `first` on hold one value, of kind `kind` where that states as many cells.
    fn want(&mut self, params: bool, first: u32, cells: u32, kind: &Kind) {
        if cells < 2 {
            return;
        }
        let kind = match kind_cells(kind) == Some(cells) {
            true => kind.clone(),
            false => Kind::Aggregate(cells),
        };
        match self.runs.get_mut(&(params, first)) {
            Some((known, _)) if *known != cells => {
                self.refused.insert((params, first));
            }
            // Of two kinds for one run, the one that names what the cells are stays.
            Some((_, held)) => {
                if matches!(held, Kind::Aggregate(_)) {
                    *held = kind;
                }
            }
            None => {
                self.runs.insert((params, first), (cells, kind));
            }
        }
    }
}

/// The run and the source of a spread `stmts` starts with: `x_k = v[0]; x_k+1 = v[1]; ..` over every cell of a value `v` whose cells are known.
fn spread<'a>(widths: &BTreeMap<Var, u32>, stmts: &'a [Stmt]) -> Option<(Run, &'a Expr)> {
    let Some(Stmt::Assign {
        target,
        value: Expr::Cell { base, index: 0 },
    }) = stmts.first()
    else {
        return None;
    };
    let (params, first) = frame_cell(*target)?;
    let cells = match &**base {
        Expr::Var(var) if var.is_temporary() => widths.get(var).copied()?,
        Expr::GuidEmpty => 2,
        _ => return None,
    };
    if cells < 2 {
        return None;
    }
    let rest = stmts.get(1..cells as usize)?;
    for (stmt, i) in rest.iter().zip(1u32..) {
        let Stmt::Assign {
            target,
            value: Expr::Cell { base: of, index },
        } = stmt
        else {
            return None;
        };
        let wanted = first.checked_add(i).map(|n| (params, n));
        if wanted.is_none() || frame_cell(*target) != wanted || *index != i || of != base {
            return None;
        }
    }
    let run = Run {
        params,
        first,
        cells,
    };
    Some((run, &**base))
}

/// Notes the function's own parameters of several cells, as the project's signature of it has them.
// ⚠ A signature whose cells are not the function's own says nothing of this function.
fn own_parameters(f: &cfg::Function, project: &Project, wanted: &mut Wanted) {
    let key = FunctionKey::new(f.head.program, f.head.at);
    let Some(signature) = project.signatures.get(&key) else {
        return;
    };
    let total = signature
        .params
        .iter()
        .try_fold(0u32, |sum, param| sum.checked_add(param.cells));
    if total != Some(f.head.params) {
        return;
    }
    let mut first = 0u32;
    for param in &signature.params {
        wanted.want(true, first, param.cells, &param.kind);
        first = first.saturating_add(param.cells);
    }
}

/// Notes each run of locals that a statement, or a row of them, writes whole: a value of several cells spread over it, or its bytes cleared.
fn written_whole(f: &cfg::Function, widths: &BTreeMap<Var, u32>, wanted: &mut Wanted) {
    for block in &f.cfg.blocks {
        for at in 0..block.stmts.len() {
            if let Some((run, _)) = spread(widths, &block.stmts[at..]) {
                if !run.params {
                    wanted.want(false, run.first, run.cells, &Kind::Unknown);
                }
            }
            if let Stmt::ZeroBytes {
                addr:
                    Expr::AddrOf {
                        var: Var::Local(first),
                        cell: 0,
                    },
                bytes,
            } = &block.stmts[at]
            {
                if bytes.is_multiple_of(8) {
                    wanted.want(false, *first, bytes / 8, &Kind::Unknown);
                }
            }
        }
    }
}

/// Notes each run of locals a call passes where the callee's signature takes one value of as many cells.
// ⚠ A call that does not pass the cells its callee's signature takes says nothing.
fn passed(
    head: &Head,
    widths: &BTreeMap<Var, u32>,
    signature: &Signature,
    args: &[Expr],
    wanted: &mut Wanted,
) {
    // The cell each argument starts at.
    let mut starts = Vec::with_capacity(args.len());
    let mut cell = 0u32;
    for arg in args {
        starts.push(cell);
        let next = cells_of(head, widths, arg, DEPTH).and_then(|cells| cell.checked_add(cells));
        match next {
            Some(next) => cell = next,
            None => return,
        }
    }
    let taken = signature
        .params
        .iter()
        .try_fold(0u32, |sum, param| sum.checked_add(param.cells));
    if taken != Some(cell) {
        return;
    }
    let mut first = 0u32;
    for param in &signature.params {
        let at = starts.partition_point(|start| *start < first);
        if let (true, Some(Expr::Var(Var::Local(local)))) = (param.cells >= 2, args.get(at)) {
            let run = args.get(at..at.saturating_add(param.cells as usize));
            let in_order = run.is_some_and(|run| {
                run.iter().zip(0u32..).all(|(arg, i)| {
                    matches!(arg, Expr::Var(Var::Local(n)) if local.checked_add(i) == Some(*n))
                })
            });
            if in_order && starts.get(at) == Some(&first) {
                wanted.want(false, *local, param.cells, &param.kind);
            }
        }
        first = first.saturating_add(param.cells);
    }
}

/// Notes the runs of locals passed to the functions the project has a signature of.
fn passed_whole(
    f: &cfg::Function,
    project: &Project,
    widths: &BTreeMap<Var, u32>,
    wanted: &mut Wanted,
) {
    if project.signatures.is_empty() {
        return;
    }
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| {
            let Expr::Call { callee, args, .. } = e else {
                return;
            };
            let key = match callee {
                Callee::Local { at, .. } => FunctionKey::new(f.head.program, *at),
                Callee::Import { program, at, .. } => FunctionKey::new(*program, *at),
                _ => return,
            };
            if let Some(signature) = project.signatures.get(&key) {
                passed(&f.head, widths, signature, args, wanted);
            }
        });
    });
}

/// A run of locals the code's own use of the cells suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hint {
    cells: u32,
    /// Whether nothing but a copy of another run into it suggests it.
    tied: bool,
}

/// The runs suggested, by first local; of two from one local the longer stays.
type Hints = BTreeMap<u32, Hint>;

/// Notes that the code uses `cells` locals from `first` on as one value.
fn hint(hints: &mut Hints, first: u32, cells: u32, tied: bool) {
    if cells < 2 {
        return;
    }
    let tied = match hints.get(&first) {
        Some(held) if held.cells > cells => return,
        Some(held) if held.cells == cells => held.tied && tied,
        _ => tied,
    };
    hints.insert(first, Hint { cells, tied });
}

/// The locals a list of expressions is, where it is two or more of them in a row and nothing else: the first and how many.
fn local_row(items: &[Expr]) -> Option<(u32, u32)> {
    let Some(Expr::Var(Var::Local(first))) = items.first() else {
        return None;
    };
    let cells = u32::try_from(items.len())
        .ok()
        .filter(|cells| *cells >= 2)?;
    let in_order = (0..cells).zip(items).all(
        |(at, item)| matches!(item, Expr::Var(Var::Local(n)) if first.checked_add(at) == Some(*n)),
    );
    in_order.then_some((*first, cells))
}

/// The frame cell a statement assigns and the frame cell it copies there, where it is `x = y` over two of them.
fn copy(stmt: &Stmt) -> Option<((bool, u32), (bool, u32))> {
    let Stmt::Assign {
        target,
        value: Expr::Var(from),
    } = stmt
    else {
        return None;
    };
    Some((frame_cell(*target)?, frame_cell(*from)?))
}

/// A frame cell: whether it is a parameter's, and its number.
type FrameCell = (bool, u32);

/// The copy `stmts` starts with of one row of frame cells to another, cell by cell in order: where each row starts, and how many cells.
fn copy_row(stmts: &[Stmt]) -> Option<(FrameCell, FrameCell, u32)> {
    let (to, from) = copy(stmts.first()?)?;
    let mut cells = 1u32;
    for (stmt, i) in stmts[1..].iter().zip(1u32..) {
        let next = |key: (bool, u32)| key.1.checked_add(i).map(|n| (key.0, n));
        match (copy(stmt), next(to), next(from)) {
            (Some((a, b)), Some(x), Some(y)) if a == x && b == y => cells = i + 1,
            _ => break,
        }
    }
    Some((to, from, cells))
}

/// Whether two rows of `cells` frame cells share no cell.
fn apart(a: (bool, u32), b: (bool, u32), cells: u32) -> bool {
    a.0 != b.0 || a.1.abs_diff(b.1) >= cells
}

/// The runs of locals the code uses as one value: gathered into a group whole, cleared through the first one's address, or copied cell by cell to or from another row.
// ⚠ A copy says no more of the row copied into than the row copied from holds: such a run is tied.
fn suggested(f: &cfg::Function, stated: &Wanted) -> Hints {
    let mut hints = Hints::new();
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Cells(items) = e {
                if let Some((first, cells)) = local_row(items) {
                    hint(&mut hints, first, cells, false);
                }
            }
        });
    });
    let known = |key: (bool, u32)| stated.runs.get(&key).map(|(cells, _)| *cells);
    for block in &f.cfg.blocks {
        let mut at = 0;
        while at < block.stmts.len() {
            if let Stmt::ZeroBytes {
                addr:
                    Expr::AddrOf {
                        var: Var::Local(first),
                        cell: 0,
                    },
                bytes,
            } = &block.stmts[at]
            {
                // Bytes that end inside a cell are a value's own size, and the value holds that cell whole.
                if *bytes > 8 && !bytes.is_multiple_of(8) {
                    hint(&mut hints, *first, bytes.div_ceil(8), false);
                }
            }
            let Some((to, from, row)) = copy_row(&block.stmts[at..]) else {
                at += 1;
                continue;
            };
            // `copy_row` found each cell of both rows below `row` within `u32`.
            let mut done = 0u32;
            while done < row {
                let (to, from) = ((to.0, to.1 + done), (from.0, from.1 + done));
                let left = row - done;
                let said = known(to).or_else(|| known(from));
                let cells = said.unwrap_or(left);
                if cells < 2 || cells > left || !apart(to, from, cells) {
                    // A row nothing states the cells of is one copy or none.
                    done = match said {
                        Some(_) => done + 1,
                        None => row,
                    };
                    continue;
                }
                if !to.0 && (!from.0 || known(from).is_some()) {
                    hint(&mut hints, to.1, cells, true);
                }
                if !from.0 {
                    hint(&mut hints, from.1, cells, false);
                }
                done += cells;
            }
            at += row as usize;
        }
    }
    hints
}

/// Takes into `wanted` each run suggested that shares no cell with a run something states.
fn take_hints(wanted: &mut Wanted, hints: Hints) {
    // The stated runs of locals by first cell, each with the furthest end of those up to it.
    let mut reach = 0u64;
    let locals = wanted.runs.range((false, 0u32)..(true, 0u32));
    let stated: Vec<(u32, u64)> = locals
        .map(|((_, first), (cells, _))| {
            reach = reach.max(u64::from(*first) + u64::from(*cells));
            (*first, reach)
        })
        .collect();
    for (first, hint) in hints {
        let end = u64::from(first) + u64::from(hint.cells);
        let before = stated.partition_point(|(start, _)| u64::from(*start) < end);
        let last = before.checked_sub(1).and_then(|at| stated.get(at));
        if last.is_some_and(|(_, far)| *far > u64::from(first)) {
            continue;
        }
        let key = (false, first);
        let held = (hint.cells, Kind::Aggregate(hint.cells));
        wanted.runs.insert(key, held);
        wanted.weak.insert(key);
        if hint.tied {
            wanted.tied.insert(key);
        }
    }
}

/// The address a statement stores through, the offset it writes at and the bytes it writes, where it states them.
fn stored(stmt: &Stmt) -> Option<(&Expr, u32, Option<u32>)> {
    match stmt {
        Stmt::Store {
            kind, addr, offset, ..
        } => Some((addr, *offset, Some(kind.bytes()))),
        Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            ..
        } => Some((addr, *offset, Some(*bytes))),
        Stmt::StoreF32Slots { addr, offset, .. } => Some((addr, *offset, None)),
        Stmt::ZeroBytes { addr, bytes } => Some((addr, 0, Some(*bytes))),
        Stmt::StepI32 { addr, offset, .. } => Some((addr, *offset, Some(4))),
        Stmt::Assign { .. } | Stmt::AssignMany { .. } | Stmt::Eval(_) => None,
    }
}

/// The frame cells `lo..=hi` an access of `bytes` bytes at `offset` through `addr` reaches, where `addr` is a frame cell's own address moved by offsets.
// ⚠ An offset that is no constant, and a count of bytes nothing states, reach from the cell to the frame's end.
fn reached(addr: &Expr, offset: u32, bytes: Option<u32>) -> Option<(bool, u32, u32)> {
    let (mut addr, mut offset, mut bytes) = (addr, u64::from(offset), bytes);
    for _ in 0..NESTED {
        let Expr::ByteOffset {
            addr: inner,
            offset: by,
        } = addr
        else {
            break;
        };
        match &**by {
            Expr::Const(c) if c.bits <= u64::from(u32::MAX) => offset += c.bits,
            _ => (offset, bytes) = (0, None),
        }
        addr = &**inner;
    }
    let Expr::AddrOf { var, cell } = addr else {
        return None;
    };
    let (params, n) = frame_cell(*var)?;
    let start = u64::from(n) + u64::from(*cell);
    let cell_at = |bytes: u64| u32::try_from(start + bytes / 8).unwrap_or(u32::MAX);
    let hi = match bytes {
        Some(bytes) => cell_at(offset + u64::from(bytes.max(1)) - 1),
        None => u32::MAX,
    };
    Some((params, cell_at(offset), hi))
}

/// Refuses each run that an access of the frame cells `lo..=hi` reaches without lying inside it.
fn crossed(runs: &Runs, gone: &mut BTreeSet<(bool, u32)>, (params, lo, hi): (bool, u32, u32)) {
    if lo > hi {
        return;
    }
    let low = run_of(runs, frame_var(params, lo)).map(|(run, _)| run);
    if low.is_some_and(|run| u64::from(hi) < run.end()) {
        return;
    }
    gone.extend(low.map(Run::key));
    gone.extend(runs.range((params, lo)..=(params, hi)).map(|(key, _)| *key));
}

/// Whether `e` is one cell that reading has no effect on and that no cell of `run` is read for: a constant, a variable or a cell of one, or a cast of those.
// ⚠ Not a load and not a call: either may read the run through an address while only some of its cells are assigned.
fn simple(head: &Head, widths: &BTreeMap<Var, u32>, e: &Expr, run: Run) -> bool {
    let read = match e {
        Expr::Cast { e: inner, .. } => &**inner,
        other => other,
    };
    let var = match read {
        Expr::Const(_) => return true,
        Expr::Var(var) => *var,
        Expr::Cell { base, .. } => match **base {
            Expr::Var(var) => var,
            _ => return false,
        },
        _ => return false,
    };
    let outside = frame_cell(var).is_none_or(|(params, n)| {
        params != run.params || n < run.first || u64::from(n) >= run.end()
    });
    outside && cells_of(head, widths, read, DEPTH) == Some(1)
}

/// The run of `runs` whose every cell the statements `stmts` starts with assign in order, each a simple value, and where the row of frame cells they copy starts if they copy one.
fn built(
    head: &Head,
    runs: &Runs,
    widths: &BTreeMap<Var, u32>,
    stmts: &[Stmt],
) -> Option<(Run, Option<(bool, u32)>)> {
    let Some(Stmt::Assign { target, value }) = stmts.first() else {
        return None;
    };
    let (params, first) = frame_cell(*target)?;
    let cells = *runs.get(&(params, first))?;
    let run = Run {
        params,
        first,
        cells,
    };
    let row = stmts.get(..cells as usize)?;
    let mut from = match value {
        Expr::Var(var) => frame_cell(*var),
        _ => None,
    };
    for (stmt, i) in row.iter().zip(0u32..) {
        let Stmt::Assign { target, value } = stmt else {
            return None;
        };
        let wanted = first.checked_add(i).map(|n| (params, n));
        if wanted.is_none() || frame_cell(*target) != wanted {
            return None;
        }
        if !simple(head, widths, value, run) {
            return None;
        }
        let copied = match value {
            Expr::Var(var) => frame_cell(*var),
            _ => None,
        };
        let next = from.and_then(|(space, n)| Some((space, n.checked_add(i)?)));
        if copied.is_none() || copied != next {
            from = None;
        }
    }
    Some((run, from))
}

/// The runs wanted that hold, with their cells and kinds.
// A run holds where it lies in the frame, apart from every other and from each variable of
// several cells there already is, no statement assigns one of its cells alone and no load or
// store through an address reaches across its ends. One the code only suggests holds, besides,
// only while no cell after its first has an address of its own.
fn sound(
    f: &cfg::Function,
    widths: &BTreeMap<Var, u32>,
    wanted: Wanted,
) -> BTreeMap<(bool, u32), (u32, Kind)> {
    let Wanted {
        mut runs,
        refused: mut gone,
        weak,
        tied,
    } = wanted;
    let run_at = |key: &(bool, u32), cells: u32| Run {
        params: key.0,
        first: key.1,
        cells,
    };
    runs.retain(|key, (cells, _)| {
        let frame = match key.0 {
            true => f.head.params,
            false => f.head.locals,
        };
        !gone.contains(key) && run_at(key, *cells).end() <= u64::from(frame)
    });

    // One of no stated width may run to the frame's end.
    let made = f.head.vars.iter().filter_map(|(var, info)| {
        let (params, first) = frame_cell(*var).filter(|_| info.width == Width::Cells)?;
        let cells = kind_cells(&info.kind).unwrap_or(u32::MAX);
        Some(Run {
            params,
            first,
            cells,
        })
    });
    let made: Vec<Run> = made.collect();
    // The run that reaches furthest of those before this one.
    let mut far: Option<Run> = None;
    for (key, (cells, _)) in &runs {
        let run = run_at(key, *cells);
        let before = far.filter(|far| far.params == run.params && u64::from(run.first) < far.end());
        if let Some(before) = before {
            // A run inside another is a part of that one; two that cross are neither.
            gone.insert(run.key());
            if run.end() > before.end() {
                gone.insert(before.key());
            }
        }
        let taken = made.iter().any(|other| {
            other.params == run.params
                && u64::from(run.first) < other.end()
                && u64::from(other.first) < run.end()
        });
        if taken {
            gone.insert(run.key());
        }
        if far.is_none_or(|far| far.params != run.params || run.end() > far.end()) {
            far = Some(run);
        }
    }
    runs.retain(|key, _| !gone.contains(key));
    if runs.is_empty() {
        return runs;
    }

    let cells: Runs = runs.iter().map(|(key, (n, _))| (*key, *n)).collect();
    // The tied runs each run is copied whole into.
    let mut copies: BTreeMap<(bool, u32), Vec<(bool, u32)>> = BTreeMap::new();
    for block in &f.cfg.blocks {
        let mut at = 0;
        while at < block.stmts.len() {
            let whole = spread(widths, &block.stmts[at..])
                .filter(|(run, _)| cells.get(&run.key()) == Some(&run.cells));
            if let Some((run, _)) = whole {
                at += run.cells as usize;
                continue;
            }
            if let Some((run, from)) = built(&f.head, &cells, widths, &block.stmts[at..]) {
                // A tied run is assigned whole only by a copy of a run as long.
                let from = from.filter(|from| cells.get(from) == Some(&run.cells));
                let is_tied = tied.contains(&run.key());
                if !is_tied || from.is_some() {
                    if let (true, Some(from)) = (is_tied, from) {
                        copies.entry(from).or_default().push(run.key());
                    }
                    at += run.cells as usize;
                    continue;
                }
            }
            let stmt = &block.stmts[at];
            for target in stmt.targets() {
                gone.extend(run_of(&cells, *target).map(|(run, _)| run.key()));
            }
            let through =
                stored(stmt).and_then(|(addr, offset, bytes)| reached(addr, offset, bytes));
            if let Some(span) = through {
                crossed(&cells, &mut gone, span);
            }
            at += 1;
        }
    }
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| {
            let span = match e {
                Expr::Load { ty, addr, offset } => reached(addr, *offset, Some(ty.bytes())),
                Expr::LoadBytes {
                    addr,
                    offset,
                    bytes,
                } => reached(addr, *offset, Some(*bytes)),
                // A cell after the first of a run the code only suggests, with an address of its own, is a variable of its own.
                Expr::AddrOf { var, cell } => {
                    let inside = frame_cell(*var).and_then(|(params, n)| {
                        run_of(&cells, frame_var(params, n.checked_add(*cell)?))
                    });
                    if let Some((run, at)) = inside {
                        if at != 0 && weak.contains(&run.key()) {
                            gone.insert(run.key());
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some(span) = span {
                crossed(&cells, &mut gone, span);
            }
        });
    });
    // A tied run falls with the run it is a copy of.
    let mut fallen: Vec<(bool, u32)> = gone.iter().copied().collect();
    while let Some(key) = fallen.pop() {
        for copy in copies.remove(&key).unwrap_or_default() {
            if gone.insert(copy) {
                fallen.push(copy);
            }
        }
    }
    runs.retain(|key, _| !gone.contains(key));
    runs
}

/// The assignment of a whole run that `stmts` starts with, and how many statements it stands for.
// A spread of a value over the run; else a row that assigns each cell in order, read whole before
// any is written, which changes nothing while every value is simple; else every byte of the run
// cleared, or stored from a value of as many cells, through the run's own address.
fn whole_write(
    head: &Head,
    runs: &Runs,
    widths: &BTreeMap<Var, u32>,
    stmts: &[Stmt],
) -> Option<(usize, Stmt)> {
    let spread = spread(widths, stmts).filter(|(run, _)| runs.get(&run.key()) == Some(&run.cells));
    if let Some((run, value)) = spread {
        let assign = Stmt::Assign {
            target: run.var(),
            value: value.clone(),
        };
        return Some((run.cells as usize, assign));
    }
    if let Some((run, _)) = built(head, runs, widths, stmts) {
        let values = stmts.iter().take(run.cells as usize);
        let values = values.filter_map(|stmt| match stmt {
            Stmt::Assign { value, .. } => Some(value.clone()),
            _ => None,
        });
        let assign = Stmt::Assign {
            target: run.var(),
            value: Expr::Cells(values.collect()),
        };
        return Some((run.cells as usize, assign));
    }
    let (addr, bytes, stored) = match stmts.first()? {
        Stmt::ZeroBytes { addr, bytes } => (addr, *bytes, None),
        Stmt::StoreBytes {
            addr,
            offset: 0,
            bytes,
            value,
        } => (addr, *bytes, Some(value)),
        _ => return None,
    };
    let Expr::AddrOf { var, cell: 0 } = addr else {
        return None;
    };
    let cells = *runs.get(&frame_cell(*var)?)?;
    if u64::from(bytes) != 8 * u64::from(cells) {
        return None;
    }
    let value = match stored {
        None => Expr::ZeroSlots(cells),
        Some(value) if cells_of(head, widths, value, DEPTH) == Some(cells) => value.clone(),
        Some(_) => return None,
    };
    let assign = Stmt::Assign {
        target: *var,
        value,
    };
    Some((1, assign))
}

/// Whether `e` is cell `index` of the variable `var`.
fn is_cell(e: &Expr, var: Var, index: u32) -> bool {
    matches!(e, Expr::Cell { base, index: at } if *at == index && **base == Expr::Var(var))
}

/// The cells of a frame variable of several cells a pass before this one made, as its kind states them.
fn held_cells(head: &Head, var: Var) -> Option<u32> {
    let info = head
        .vars
        .get(&var)
        .filter(|info| info.width == Width::Cells)?;
    kind_cells(&info.kind)
}

/// Takes off what `extract_bytes` wraps its value in and says nothing by, and is the value itself where it takes every byte of it.
// A group of one item is that item. Bytes out of an extract they lie within are bytes of that
// extract's value, where that extract lies within its own value's stated cells, so no reach past
// a value's end goes with it. Every byte of a value of two cells or more is the value; a value
// of one cell is a cell, which bytes of it as whole cells are not.
fn plainer(head: &Head, widths: &BTreeMap<Var, u32>, e: &mut Expr) -> bool {
    let mut changed = false;
    let whole = {
        let Expr::ExtractBytes {
            value,
            offset,
            bytes,
        } = &mut *e
        else {
            return false;
        };
        for _ in 0..NESTED {
            let taken = match &mut **value {
                Expr::Cells(items) if items.len() == 1 => items.pop().map(|item| (item, *offset)),
                Expr::ExtractBytes {
                    value: under,
                    offset: at,
                    bytes: of,
                } => {
                    let inside = u64::from(*offset) + u64::from(*bytes) <= u64::from(*of);
                    let stated = cells_of(head, widths, under, DEPTH).is_some_and(|cells| {
                        u64::from(*at) + u64::from(*of) <= 8 * u64::from(cells)
                    });
                    match at.checked_add(*offset) {
                        Some(moved) if inside && stated => {
                            Some((std::mem::replace(&mut **under, Expr::GuidEmpty), moved))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let Some((inner, moved)) = taken else {
                break;
            };
            **value = inner;
            *offset = moved;
            changed = true;
        }
        let cells = cells_of(head, widths, value, DEPTH);
        *offset == 0
            && cells.is_some_and(|cells| cells >= 2 && u64::from(*bytes) == 8 * u64::from(cells))
    };
    if whole {
        if let Expr::ExtractBytes { value, .. } = std::mem::replace(e, Expr::GuidEmpty) {
            *e = *value;
        }
    }
    changed || whole
}

/// Makes every `extract_bytes` of the function plainer, with the count of those changed.
fn plainer_all(f: &mut cfg::Function, widths: &BTreeMap<Var, u32>) -> usize {
    let mut count = 0;
    let head = &f.head;
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut |top| {
                top.walk_mut(&mut |e| count += usize::from(plainer(head, widths, e)));
            });
        }
        if let Some(e) = block.term.expr_mut() {
            e.walk_mut(&mut |e| count += usize::from(plainer(head, widths, e)));
        }
    }
    count
}

/// Replaces each row `x[0], x[1], ..` that names every cell of a variable `x` of several cells, in order, by `x`.
fn joined(wide: &impl Fn(Var) -> Option<u32>, items: &mut Vec<Expr>) {
    let mut at = 0;
    while at < items.len() {
        let whole = match &items[at] {
            Expr::Cell { base, index: 0 } => match **base {
                Expr::Var(var) => wide(var).map(|cells| (var, cells as usize)),
                _ => None,
            },
            _ => None,
        };
        if let Some((var, cells)) = whole {
            let row = items.get(at..at.saturating_add(cells));
            let in_order = row
                .is_some_and(|row| (row.iter().zip(0u32..)).all(|(item, i)| is_cell(item, var, i)));
            if cells >= 2 && in_order {
                items.drain(at + 1..at + cells);
                items[at] = Expr::Var(var);
            }
        }
        at += 1;
    }
}

/// The frame variables whose address the code already takes for more than an access of their own first cell.
// ⚠ A pass after this one takes the lowest local and the lowest parameter with an address taken
// to reach every one after it, so an address written where none was costs those their kinds and
// their source constructs. A run is loaded through its address only where that adds no such address.
#[derive(Debug, Default, PartialEq, Eq)]
struct Exposed {
    locals: BTreeSet<u32>,
    /// The lowest such parameter.
    param: Option<u32>,
}

impl Exposed {
    /// Whether a load through the address of the run at `key` takes no address the code does not take already.
    fn has(&self, (params, first): (bool, u32)) -> bool {
        match params {
            true => self.param.is_some_and(|low| low <= first),
            false => self.locals.contains(&first),
        }
    }
}

/// The variable whose own address an access of `bytes` bytes at `offset` goes through, where the access ends within the variable's one cell.
fn first_cell(addr: &Expr, offset: u32, bytes: Option<u32>) -> Option<Var> {
    let Expr::AddrOf { var, cell: 0 } = addr else {
        return None;
    };
    let inside = bytes.is_some_and(|bytes| u64::from(offset) + u64::from(bytes) <= 8);
    inside.then_some(*var)
}

/// The frame variables exposed, each cell of a run counted as the run.
fn exposure(f: &cfg::Function, runs: &Runs) -> Exposed {
    // The run a variable is a cell of, else the variable, and whether the variable is its first cell.
    let home = |var: Var| -> Option<((bool, u32), bool)> {
        let key = frame_cell(var)?;
        Some(match run_of(runs, var) {
            Some((run, at)) => (run.key(), at == 0),
            None => (key, true),
        })
    };
    // The mentions of each one's address, and those of them an access of its first cell alone goes through.
    let mut all: BTreeMap<(bool, u32), u64> = BTreeMap::new();
    let mut plain: BTreeMap<(bool, u32), u64> = BTreeMap::new();
    let mut within = |var: Option<Var>| {
        if let Some((key, true)) = var.and_then(home) {
            *plain.entry(key).or_insert(0) += 1;
        }
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            if let Some((addr, offset, bytes)) = stored(stmt) {
                within(first_cell(addr, offset, bytes));
            }
        }
    }
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::AddrOf { var, .. } => {
                if let Some((key, _)) = home(*var) {
                    *all.entry(key).or_insert(0) += 1;
                }
            }
            Expr::Load { ty, addr, offset } => {
                within(first_cell(addr, *offset, Some(ty.bytes())));
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => {
                // Every byte of a run loaded through its address becomes the run, and the address goes.
                let whole = *offset == 0
                    && own_cells(runs, addr)
                        .is_some_and(|cells| u64::from(*bytes) == 8 * u64::from(cells));
                let bytes = if whole { 0 } else { *bytes };
                within(first_cell(addr, *offset, Some(bytes)));
            }
            _ => {}
        });
    });
    let mut found = Exposed::default();
    for (key, mentions) in all {
        if mentions <= plain.get(&key).copied().unwrap_or(0) {
            continue;
        }
        match key {
            (true, n) => found.param = Some(found.param.map_or(n, |low| low.min(n))),
            (false, n) => {
                found.locals.insert(n);
            }
        }
    }
    found
}

/// Whether a load as `ty` leaves every bit above the bytes it reads zero.
fn unsigned(ty: Scalar) -> bool {
    matches!(ty, Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64)
}

/// The cells of the run made that `addr` is the own address of.
fn own_cells(runs: &Runs, addr: &Expr) -> Option<u32> {
    match addr {
        Expr::AddrOf { var, cell: 0 } => frame_cell(*var).and_then(|key| runs.get(&key)).copied(),
        _ => None,
    }
}

/// What `extract_bytes(value, offset, bytes)` is where `value` is cells of one variable of several, in order, and the bytes lie within them.
// Every byte of the variable is the variable. Bytes of a run whose address is exposed are a load
// through that address, as the code's own accesses of the run are. Bytes of some cells are the
// bytes that much further into the variable whole.
fn fielded(
    wide: &impl Fn(Var) -> Option<u32>,
    runs: &Runs,
    exposed: &Exposed,
    value: &Expr,
    offset: u32,
    bytes: u32,
) -> Option<Expr> {
    // The variable, the cell of it the value starts at, and the cells the value is.
    let (var, at, cells) = match value {
        Expr::Var(var) => (*var, 0, wide(*var)?),
        Expr::Cell { base, index } => match **base {
            Expr::Var(var) => (var, *index, 1),
            _ => return None,
        },
        Expr::Cells(items) => {
            let Some(Expr::Cell { base, index }) = items.first() else {
                return None;
            };
            let var = match **base {
                Expr::Var(var) => var,
                _ => return None,
            };
            let cells = u32::try_from(items.len()).ok()?;
            let in_order = (items.iter().zip(0u32..))
                .all(|(item, i)| index.checked_add(i).is_some_and(|n| is_cell(item, var, n)));
            if !in_order {
                return None;
            }
            (var, *index, cells)
        }
        _ => return None,
    };
    let held = wide(var)?;
    let lies_within = u64::from(at) + u64::from(cells) <= u64::from(held)
        && u64::from(offset) + u64::from(bytes) <= 8 * u64::from(cells);
    if bytes == 0 || !lies_within {
        return None;
    }
    let offset = at.checked_mul(8)?.checked_add(offset)?;
    if offset == 0 && u64::from(bytes) == 8 * u64::from(held) {
        return Some(Expr::Var(var));
    }
    let loaded = frame_cell(var).is_some_and(|key| runs.contains_key(&key) && exposed.has(key));
    let addr = Box::new(Expr::AddrOf { var, cell: 0 });
    let ty = match bytes {
        1 => Some(Scalar::U8),
        2 => Some(Scalar::U16),
        4 => Some(Scalar::U32),
        8 => Some(Scalar::U64),
        _ => None,
    };
    match (loaded, ty) {
        (true, Some(ty)) => Some(Expr::Load { ty, addr, offset }),
        (true, None) if bytes.is_multiple_of(8) => Some(Expr::LoadBytes {
            addr,
            offset,
            bytes,
        }),
        // Already the bytes of the variable whole.
        _ if at == 0 && matches!(value, Expr::Var(_)) => None,
        _ => Some(Expr::ExtractBytes {
            value: Box::new(Expr::Var(var)),
            offset,
            bytes,
        }),
    }
}

/// Rewrites one expression for the runs made: a cell alone is `run[n]`, an address counts its cells from the run's first, every cell in order is the run, and bytes taken out of cells of it are bytes of it.
// `Expr::walk_mut` reaches a node after those inside it, so a row of cells is found as `run[n]`s.
// A frame variable of several cells made before this pass is as one: no run takes a cell of it.
fn regrouped(
    top: &mut Expr,
    head: &Head,
    runs: &Runs,
    widths: &BTreeMap<Var, u32>,
    exposed: &Exposed,
) {
    let wide = |var: Var| match frame_cell(var) {
        Some(key) => (runs.get(&key).copied()).or_else(|| held_cells(head, var)),
        None => widths.get(&var).copied(),
    };
    top.walk_mut(&mut |e| {
        let new = match &mut *e {
            Expr::ExtractBytes {
                value,
                offset,
                bytes,
            } => fielded(&wide, runs, exposed, value, *offset, *bytes),
            // A cast as wide as the unsigned load of a run's bytes under it is the load read as the cast reads.
            Expr::Cast { to, e: inner } => match &**inner {
                Expr::Load { ty, addr, offset }
                    if ty.bytes() == to.bytes()
                        && unsigned(*ty)
                        && own_cells(runs, addr).is_some() =>
                {
                    Some(Expr::Load {
                        ty: *to,
                        addr: addr.clone(),
                        offset: *offset,
                    })
                }
                _ => None,
            },
            Expr::Var(var) => run_of(runs, *var).map(|(run, cell)| Expr::Cell {
                base: Box::new(Expr::Var(run.var())),
                index: cell,
            }),
            Expr::AddrOf { var, cell } => {
                if let Some((run, at)) = run_of(runs, *var) {
                    if let Some(moved) = cell.checked_add(at) {
                        (*var, *cell) = (run.var(), moved);
                    }
                }
                None
            }
            Expr::Call { args: items, .. } => {
                joined(&wide, items);
                None
            }
            Expr::Cells(items) => {
                joined(&wide, items);
                // A group made of one whole value is that value.
                match items.as_slice() {
                    [Expr::Var(var)] if wide(*var).is_some() => Some(Expr::Var(*var)),
                    _ => None,
                }
            }
            // Every byte of a run, loaded through its own address, is the run.
            Expr::LoadBytes {
                addr,
                offset: 0,
                bytes,
            } => match **addr {
                Expr::AddrOf { var, cell: 0 } => frame_cell(var)
                    .and_then(|key| runs.get(&key))
                    .filter(|cells| u64::from(*bytes) == 8 * u64::from(**cells))
                    .map(|_| Expr::Var(var)),
                _ => None,
            },
            _ => None,
        };
        if let Some(new) = new {
            *e = new;
        }
    });
}

/// What one run did: counts of runs made one variable, and of `extract_bytes` made plainer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Made {
    pub params: usize,
    pub locals: usize,
    pub bytes: usize,
}

/// Makes one variable of each run of cells that holds one value, and passes a value of several cells whole.
// `Stats` has no count of runs: what was made is traced.
pub fn run(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) {
    if !cx.options.on(Pass::Aggregates) || f.head.incomplete.is_some() {
        return;
    }
    let done = apply(f, cx.project);
    tracing::trace!(
        function = %f.head.name,
        params = done.params,
        locals = done.locals,
        bytes = done.bytes,
        "aggregates"
    );
}

/// [`run`] whatever the options say, with the count of what it did.
pub(crate) fn apply(f: &mut cfg::Function, project: &Project) -> Made {
    let mut done = Made::default();
    let widths = widths(f);
    let mut wanted = Wanted::default();
    own_parameters(f, project, &mut wanted);
    written_whole(f, &widths, &mut wanted);
    passed_whole(f, project, &widths, &mut wanted);
    let hints = suggested(f, &wanted);
    take_hints(&mut wanted, hints);
    let made = sound(f, &widths, wanted);
    done.bytes = plainer_all(f, &widths);
    let premade = (f.head.vars.iter())
        .any(|(var, info)| frame_cell(*var).is_some() && info.width == Width::Cells);
    if made.is_empty() && widths.is_empty() && !premade {
        return done;
    }
    let runs: Runs = made
        .iter()
        .map(|(key, (cells, _))| (*key, *cells))
        .collect();

    let head = &f.head;
    for block in &mut f.cfg.blocks {
        let mut merged: Vec<(usize, usize, Stmt)> = Vec::new();
        let mut at = 0;
        while at < block.stmts.len() {
            match whole_write(head, &runs, &widths, &block.stmts[at..]) {
                Some((len, assign)) => {
                    merged.push((at, len, assign));
                    at += len;
                }
                None => at += 1,
            }
        }
        // From the last back, so that the places before stay where they were found.
        for (at, len, assign) in merged.into_iter().rev() {
            block.stmts.drain(at + 1..at + len);
            block.stmts[at] = assign;
        }
    }
    // After the whole writes: one that was a store through the run's address takes that address away.
    let exposed = exposure(f, &runs);
    let head = &f.head;
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut |top| regrouped(top, head, &runs, &widths, &exposed));
        }
        if let Some(e) = block.term.expr_mut() {
            regrouped(e, head, &runs, &widths, &exposed);
        }
    }

    for ((params, first), (cells, kind)) in made {
        match params {
            true => done.params += 1,
            false => done.locals += 1,
        }
        let var = frame_var(params, first);
        let name = f.head.vars.get(&var).and_then(|info| info.name.clone());
        // The run's later cells are never named again; `first + cells` is within the frame.
        let later = frame_var(params, first + 1)..=frame_var(params, first + (cells - 1));
        let named: Vec<Var> = f.head.vars.range(later).map(|(var, _)| *var).collect();
        for var in named {
            f.head.vars.remove(&var);
        }
        let info = VarInfo {
            width: Width::Cells,
            kind,
            name,
            single: false,
        };
        f.head.vars.insert(var, info);
    }
    done
}

#[cfg(test)]
#[path = "aggregates_tests.rs"]
mod tests;
