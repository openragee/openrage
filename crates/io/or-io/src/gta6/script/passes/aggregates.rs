//! Aggregates: a value of several cells is one variable, as a parameter too.
//!
//! Works on the blocks, after `interproc` and before `types`. A run of parameters or locals is
//! one variable where the project's signatures or the code's own whole writes say it holds one
//! value and no statement assigns one of its cells alone; its first cell's `VarInfo` then says
//! `Width::Cells` and a kind that states the cells, and no later cell of the run is named again.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Expr, FunctionKey, Head, Kind, Stmt, Var, VarInfo, Width};
use crate::gta6::script::project::{Project, Signature};
use crate::gta6::script::{Context, Pass, Stats};

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
                true => widths.get(var).copied(),
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

/// The runs wanted that hold, with their cells and kinds.
// A run holds where it lies in the frame, apart from every other and from each variable of
// several cells there already is, no statement assigns one of its cells alone and no load or
// store through an address reaches across its ends.
fn sound(
    f: &cfg::Function,
    widths: &BTreeMap<Var, u32>,
    wanted: Wanted,
) -> BTreeMap<(bool, u32), (u32, Kind)> {
    let Wanted {
        mut runs,
        refused: mut gone,
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
    for block in &f.cfg.blocks {
        let mut at = 0;
        while at < block.stmts.len() {
            let whole = spread(widths, &block.stmts[at..])
                .filter(|(run, _)| cells.get(&run.key()) == Some(&run.cells));
            if let Some((run, _)) = whole {
                at += run.cells as usize;
                continue;
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
                _ => None,
            };
            if let Some(span) = span {
                crossed(&cells, &mut gone, span);
            }
        });
    });
    runs.retain(|key, _| !gone.contains(key));
    runs
}

/// The assignment of a whole run that `stmts` starts with, and how many statements it stands for.
// A spread of a value over the run; else every byte of the run cleared, or stored from a value
// of as many cells, through the run's own address.
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

/// Rewrites one expression for the runs made: a cell alone is `run[n]`, an address counts its cells from the run's first, and every cell in order is the run.
// `Expr::walk_mut` reaches a node after those inside it, so a row of cells is found as `run[n]`s.
fn regrouped(top: &mut Expr, runs: &Runs, widths: &BTreeMap<Var, u32>) {
    let wide = |var: Var| match frame_cell(var) {
        Some(key) => runs.get(&key).copied(),
        None => widths.get(&var).copied(),
    };
    top.walk_mut(&mut |e| {
        let new = match &mut *e {
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

/// What one run did, each a count of runs made one variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Made {
    pub params: usize,
    pub locals: usize,
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
    let made = sound(f, &widths, wanted);
    if made.is_empty() && widths.is_empty() {
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
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut |top| regrouped(top, &runs, &widths));
        }
        if let Some(e) = block.term.expr_mut() {
            regrouped(e, &runs, &widths);
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
        };
        f.head.vars.insert(var, info);
    }
    done
}

#[cfg(test)]
#[path = "aggregates_tests.rs"]
mod tests;
