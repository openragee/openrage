//! What the values pass knows of a function before it moves anything: how each variable is used,
//! which frame cells a pointer may reach, and what evaluating an expression reads and writes.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use crate::gta6::script::cfg::{self, BlockId, Cfg};
use crate::gta6::script::ir::{BinOp, Callee, Expr, Head, Kind, Scalar, Stmt, Var, VarInfo, Width};

/// How many statements past its assignment a new object is followed before it is taken to be in other hands.
const HELD: usize = 256;

/// How one variable is used over a whole function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::gta6::script::passes) struct Uses {
    /// Assignments that name it.
    pub defs: u32,
    /// Times its value is read by name.
    pub reads: u32,
    /// Times its address is taken.
    pub addrs: u32,
    /// Of those, the times a load or a store goes through the address and it goes nowhere else.
    pub direct: u32,
}

/// A variable, or a run of frame cells, that an evaluation reads or writes without a pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::gta6::script::passes) enum Place {
    /// Cells `lo..=hi` of the locals, or of the parameters.
    Frame {
        params: bool,
        lo: u32,
        hi: u32,
    },
    Temp(Var),
}

impl Place {
    fn overlaps(self, other: Place) -> bool {
        match (self, other) {
            (
                Place::Frame {
                    params: a,
                    lo: a_lo,
                    hi: a_hi,
                },
                Place::Frame {
                    params: b,
                    lo: b_lo,
                    hi: b_hi,
                },
            ) => a == b && a_lo <= b_hi && b_lo <= a_hi,
            (Place::Temp(a), Place::Temp(b)) => a == b,
            _ => false,
        }
    }
}

/// The statements of one block through which a new object is in no hands but its variable's: those after `from` and before `until`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    block: BlockId,
    from: usize,
    until: usize,
}

impl Window {
    fn holds(self, at: Option<(BlockId, usize)>) -> bool {
        at.is_some_and(|(block, k)| block == self.block && self.from < k && k < self.until)
    }
}

/// A load or a store through a variable that holds an address, and the bytes it covers from there.
#[derive(Clone, Copy, Debug)]
struct Based {
    base: Var,
    lo: u32,
    /// `None` where nothing states how far the access runs.
    hi: Option<u32>,
    /// Where the object the variable holds is in no other hands, if the function made it.
    window: Option<Window>,
}

impl Based {
    fn apart(&self, other: &Based) -> bool {
        self.hi.is_some_and(|hi| hi < other.lo) || other.hi.is_some_and(|hi| hi < self.lo)
    }
}

/// The reads of `var` in `top` that are not the address a load goes through.
fn loose(top: &Expr, var: Var) -> i64 {
    let mut count = 0;
    top.walk(&mut |e| match e {
        Expr::Var(read) if *read == var => count += 1,
        Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. }
            if matches!(&**addr, Expr::Var(base) if *base == var) =>
        {
            count -= 1;
        }
        _ => {}
    });
    count
}

/// The reads of `var` in a statement that are not the address a load or a store goes through.
fn loose_in(stmt: &Stmt, var: Var) -> i64 {
    let mut count = 0;
    stmt.each_expr(&mut |top| count += loose(top, var));
    let through = matches!(stored(stmt), Some((Expr::Var(base), ..)) if *base == var);
    count - i64::from(through)
}

/// The cells `bytes` bytes at `offset` from cell `first` cover; a count nothing states runs to the frame's end.
fn span(first: u32, offset: u32, bytes: Option<u32>) -> (u32, u32) {
    let lo = first.saturating_add(offset / 8);
    let hi = match bytes {
        Some(bytes) => first.saturating_add(offset.saturating_add(bytes.max(1) - 1) / 8),
        None => u32::MAX,
    };
    (lo, hi)
}

/// The address a store goes through, the offset it writes at and the bytes it writes, where the statement says.
pub(in crate::gta6::script::passes) fn stored(stmt: &Stmt) -> Option<(&Expr, u32, Option<u32>)> {
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

/// Calls `f` on every expression at the top of a statement or of a terminator.
pub(in crate::gta6::script::passes) fn each_top<'a>(cfg: &'a Cfg, f: &mut impl FnMut(&'a Expr)) {
    for block in &cfg.blocks {
        for stmt in &block.stmts {
            stmt.each_expr(f);
        }
        if let Some(e) = block.term.expr() {
            f(e);
        }
    }
}

pub(in crate::gta6::script::passes) fn each_top_mut(cfg: &mut Cfg, f: &mut impl FnMut(&mut Expr)) {
    for block in &mut cfg.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(f);
        }
        if let Some(e) = block.term.expr_mut() {
            f(e);
        }
    }
}

/// Calls `f` on each expression directly inside `e`, in the order they are evaluated.
fn operands<'a>(e: &'a Expr, f: &mut impl FnMut(&'a Expr)) {
    match e {
        Expr::Const(_)
        | Expr::Var(_)
        | Expr::GuidEmpty
        | Expr::AddrOf { .. }
        | Expr::ZeroSlots(_)
        | Expr::NewObject(_)
        | Expr::FunctionRef { .. }
        | Expr::BuiltinFunctionRef { .. }
        | Expr::StringConstant { .. }
        | Expr::BuiltinGlobal(_) => {}
        Expr::Cell { base: inner, .. }
        | Expr::Sugar { e: inner, .. }
        | Expr::Cast { e: inner, .. }
        | Expr::Truth(inner)
        | Expr::Unary { e: inner, .. }
        | Expr::Load { addr: inner, .. }
        | Expr::LoadBytes { addr: inner, .. }
        | Expr::ExtractBytes { value: inner, .. }
        | Expr::NewArray { len: inner, .. }
        | Expr::VirtualFunctionRef {
            receiver: inner, ..
        } => f(inner),
        Expr::Binary { l, r, .. } => {
            f(l);
            f(r);
        }
        Expr::ByteOffset { addr, offset } => {
            f(addr);
            f(offset);
        }
        Expr::Select { cond, then, els } => {
            f(cond);
            f(then);
            f(els);
        }
        Expr::Cells(items) | Expr::Call { args: items, .. } => {
            for item in items {
                f(item);
            }
        }
        Expr::Vector3(items) => {
            for item in items.iter() {
                f(item);
            }
        }
    }
}

/// Appends the nodes of `top`, each after the nodes it works on: the order evaluation finishes them in.
// An arm of a ternary is listed whether or not it would run, which only adds to what came before.
pub(in crate::gta6::script::passes) fn evaluated<'a>(top: &'a Expr, out: &mut Vec<&'a Expr>) {
    let mut stack = vec![(top, false)];
    while let Some((e, opened)) = stack.pop() {
        if opened {
            out.push(e);
            continue;
        }
        stack.push((e, true));
        let first = stack.len();
        operands(e, &mut |operand| stack.push((operand, false)));
        stack[first..].reverse();
    }
}

/// Calls `f` on the address of every load that evaluating `top` is sure to make: an arm that may not run is not looked into.
pub(in crate::gta6::script::passes) fn sure_loads<'a>(top: &'a Expr, f: &mut impl FnMut(&'a Expr)) {
    let mut stack = vec![top];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Select { cond, .. } => stack.push(cond),
            Expr::Binary {
                op: BinOp::LogicalAnd | BinOp::LogicalOr,
                l,
                ..
            } => stack.push(l),
            other => {
                if let Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } = other {
                    f(addr);
                }
                operands(other, &mut |operand| stack.push(operand));
            }
        }
    }
}

/// How many expressions deep `top` nests.
pub(in crate::gta6::script::passes) fn depth(top: &Expr) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(top, 1usize)];
    while let Some((e, at)) = stack.pop() {
        deepest = deepest.max(at);
        operands(e, &mut |operand| stack.push((operand, at + 1)));
    }
    deepest
}

/// Whether evaluating `e` reads nothing but variables and changes nothing, so that not evaluating it loses nothing.
pub(in crate::gta6::script::passes) fn inert(e: &Expr) -> bool {
    let mut inert = true;
    e.walk(&mut |node| {
        inert &= match node {
            // Through a variable's own address a load reads the variable, which is always there to read.
            Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } => {
                matches!(**addr, Expr::AddrOf { .. })
            }
            Expr::Call { .. }
            | Expr::NewObject(_)
            | Expr::NewArray { .. }
            | Expr::VirtualFunctionRef { .. }
            | Expr::BuiltinGlobal(_) => false,
            _ => true,
        };
    });
    inert
}

/// Whether an expression is one cell wide by its form.
pub(in crate::gta6::script::passes) fn one_cell(head: &Head, e: &Expr) -> bool {
    match e {
        Expr::Var(var) => head
            .vars
            .get(var)
            .is_none_or(|info| info.width != Width::Cells),
        Expr::Call { returns, .. } => *returns == Some(1),
        Expr::Cells(_)
        | Expr::GuidEmpty
        | Expr::LoadBytes { .. }
        | Expr::ExtractBytes { .. }
        | Expr::ZeroSlots(_)
        | Expr::Vector3(_)
        | Expr::BuiltinGlobal(_) => false,
        _ => true,
    }
}

/// The cells a many-cell expression yields, where its own form says.
pub(in crate::gta6::script::passes) fn cells_of(
    e: &Expr,
    widths: &HashMap<Var, u32>,
) -> Option<u32> {
    match e {
        Expr::ZeroSlots(cells) => Some(*cells),
        Expr::Call {
            returns: Some(cells),
            ..
        } => Some(*cells),
        Expr::LoadBytes { bytes, .. } | Expr::ExtractBytes { bytes, .. } => Some(bytes.div_ceil(8)),
        Expr::Vector3(_) | Expr::GuidEmpty => Some(2),
        Expr::Var(var) => widths.get(var).copied(),
        _ => None,
    }
}

/// `e` under the casts that keep every bit of it.
fn whole(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// The local `e` is the address of, and the cell, read through casts that keep every bit of it.
fn local_address(e: &Expr) -> Option<(Var, u32)> {
    match whole(e) {
        Expr::AddrOf {
            var: var @ Var::Local(_),
            cell,
        } => Some((*var, *cell)),
        _ => None,
    }
}

/// Runs of cells `lo..=hi` in rising order, those that meet or touch made one.
fn merged(mut runs: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    runs.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(runs.len());
    for (lo, hi) in runs {
        match out.last_mut() {
            Some(last) if lo <= last.1.saturating_add(1) => last.1 = last.1.max(hi),
            _ => out.push((lo, hi)),
        }
    }
    out
}

/// Whether one of `runs`, as [`merged`] leaves them, holds a cell of `lo..=hi`.
fn within(runs: &[(u32, u32)], lo: u32, hi: u32) -> bool {
    let at = runs.partition_point(|(_, to)| *to < lo);
    runs.get(at).is_some_and(|(from, _)| *from <= hi)
}

#[derive(Default)]
struct Scan {
    uses: HashMap<Var, Uses>,
    direct_locals: Vec<(u32, u32)>,
    /// Each `h = &x[cell];`: the variable given an address, the variable it is of, and the cell.
    holds: Vec<(Var, Var, u32)>,
    /// Each load or store that goes through a variable's value: the variable, the offset and the bytes.
    through: Vec<(Var, u32, Option<u32>)>,
    /// How far a local's variable runs by an access through its own address, or by its address moved a stated count of bytes: its first cell and the last cell reached.
    extents: Vec<(u32, u32)>,
    /// The locals whose address is moved by a count nothing states, or gone through for a length nothing states.
    adrift: HashSet<Var>,
    /// Each variable's value moved by a count of bytes: the whole cells where a constant states them.
    moved: Vec<(Var, Option<u32>)>,
    /// How many reads of each variable hand its value on as it is: an argument of a call, or the value a store writes.
    handed: HashMap<Var, u32>,
}

impl Scan {
    /// Counts `addr` as used in place where it is a variable's own address.
    fn direct(&mut self, addr: &Expr, offset: u32, bytes: Option<u32>) {
        if let Expr::Var(holder) = addr {
            self.through.push((*holder, offset, bytes));
        }
        let Expr::AddrOf { var, cell } = addr else {
            return;
        };
        self.uses.entry(*var).or_default().direct += 1;
        if let Var::Local(n) = var {
            let reached = span(n.saturating_add(*cell), offset, bytes);
            self.direct_locals.push(reached);
            match bytes {
                Some(_) => self.extents.push((*n, reached.1)),
                None => {
                    self.adrift.insert(*var);
                }
            }
        }
    }

    /// Notes what a statement assigns, stores through and evaluates.
    fn stmt(&mut self, stmt: &Stmt) {
        for target in stmt.targets() {
            self.uses.entry(*target).or_default().defs += 1;
        }
        if let Some((addr, offset, bytes)) = stored(stmt) {
            self.direct(addr, offset, bytes);
        }
        match stmt {
            Stmt::Assign {
                target: holder @ (Var::Local(_) | Var::Value { .. } | Var::Stack { .. }),
                value: Expr::AddrOf { var, cell },
            } => self.holds.push((*holder, *var, *cell)),
            Stmt::Store {
                value: Expr::Var(var),
                ..
            } => *self.handed.entry(*var).or_default() += 1,
            _ => {}
        }
        stmt.each_expr(&mut |e| self.expr(e));
    }

    /// Carries what is done with a variable that holds a local's address over to the local: the address runs as far through its holder as it does in place.
    // ⚠ A read of a holder that is no address a load or a store goes through, no value handed
    // on as it is and no move by a stated count may take the address anywhere: a copy, a sum,
    // a test.
    fn settle(&mut self) {
        let mut furthest: HashMap<Var, Option<u32>> = HashMap::new();
        let mut explained: HashMap<Var, u32> = self.handed.clone();
        for (holder, offset, bytes) in &self.through {
            let end = bytes.map(|bytes| offset.saturating_add(bytes.max(1) - 1));
            let known = furthest.entry(*holder).or_insert(Some(0));
            *known = (*known).zip(end).map(|(far, end)| far.max(end));
            *explained.entry(*holder).or_default() += 1;
        }
        let mut moves: HashMap<Var, Vec<Option<u32>>> = HashMap::new();
        for (holder, cells) in &self.moved {
            *explained.entry(*holder).or_default() += 1;
            moves.entry(*holder).or_default().push(*cells);
        }
        for (holder, var, cell) in &self.holds {
            let Var::Local(n) = var else {
                continue;
            };
            let first = n.saturating_add(*cell);
            match furthest.get(holder) {
                Some(Some(end)) => self.extents.push((*n, first.saturating_add(end / 8))),
                Some(None) => {
                    self.adrift.insert(*var);
                }
                None => {}
            }
            for cells in moves.get(holder).into_iter().flatten() {
                match cells {
                    Some(cells) => self.extents.push((*n, first.saturating_add(*cells))),
                    None => {
                        self.adrift.insert(*var);
                    }
                }
            }
            let reads = self.uses.get(holder).map_or(0, |uses| uses.reads);
            if reads > explained.get(holder).copied().unwrap_or(0) {
                self.adrift.insert(*var);
            }
        }
    }

    /// Notes the address of a local moved by `offset` bytes: by a constant it stays in the variable, which then runs that far.
    fn offset(&mut self, addr: &Expr, offset: &Expr) {
        let cells = match offset {
            Expr::Const(c) => Some(u32::try_from(c.bits / 8).unwrap_or(u32::MAX)),
            _ => None,
        };
        if let Expr::Var(holder) = whole(addr) {
            self.moved.push((*holder, cells));
        }
        let Some((var, cell)) = local_address(addr) else {
            return;
        };
        match (var, cells) {
            (Var::Local(n), Some(cells)) => {
                let last = n.saturating_add(cell).saturating_add(cells);
                self.extents.push((n, last));
            }
            _ => {
                self.adrift.insert(var);
            }
        }
    }

    fn expr(&mut self, top: &Expr) {
        top.walk(&mut |e| match e {
            Expr::Var(var) => self.uses.entry(*var).or_default().reads += 1,
            Expr::AddrOf { var, cell } => {
                self.uses.entry(*var).or_default().addrs += 1;
                // The address of a later cell of a variable says the variable runs that far.
                if let (Var::Local(n), true) = (var, *cell > 0) {
                    self.extents.push((*n, n.saturating_add(*cell)));
                }
            }
            Expr::Load { ty, addr, offset } => self.direct(addr, *offset, Some(ty.bytes())),
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.direct(addr, *offset, Some(*bytes)),
            Expr::ByteOffset { addr, offset } => self.offset(addr, offset),
            Expr::Call { args, .. } => {
                for arg in args {
                    if let Expr::Var(var) = arg {
                        *self.handed.entry(*var).or_default() += 1;
                    }
                }
            }
            // An address worked on as a number may come out anywhere.
            Expr::Binary { l, r, .. } => {
                for side in [l, r] {
                    if let Some((var, _)) = local_address(side) {
                        self.adrift.insert(var);
                    }
                }
            }
            _ => {}
        });
    }
}

/// The locals a statement of its own assigns with no neighbour assigned beside it: each is one cell wide, and no variable runs across it.
// A value of several cells is written by one instruction, which a front end writes as its cells
// assigned in a row, the lowest first, or as one tuple; one cell of such a value is never assigned
// by a statement that stands alone. A cell that is, and that no row assigns anywhere, is a
// variable of its own: a frame cell may hold a scalar in one place and a cell of a value in another.
// ⚠ True of the form a front end made and of no later one: a pass that removes one assignment
// of a row leaves its neighbour standing alone.
// ⚠ An access through a local's address that runs over such a cell contradicts the proof: then
// nothing is proved of the function.
pub(in crate::gta6::script::passes) fn single_cells(f: &cfg::Function) -> BTreeSet<u32> {
    let assigned = |stmt: Option<&Stmt>| match stmt {
        Some(Stmt::Assign {
            target: Var::Local(n),
            ..
        }) => Some(*n),
        _ => None,
    };
    let mut alone: BTreeSet<u32> = BTreeSet::new();
    // The cells assigned as one of several: a cell assigned both ways is proved nothing.
    let mut tupled: Vec<u32> = Vec::new();
    let mut scan = Scan::default();
    for block in &f.cfg.blocks {
        for (i, stmt) in block.stmts.iter().enumerate() {
            match stmt {
                Stmt::Assign {
                    target: Var::Local(n),
                    ..
                } => {
                    let before = i.checked_sub(1).and_then(|j| assigned(block.stmts.get(j)));
                    let after = assigned(block.stmts.get(i + 1));
                    let in_row = before.is_some_and(|below| below.checked_add(1) == Some(*n))
                        || after.is_some_and(|above| n.checked_add(1) == Some(above));
                    match in_row {
                        true => tupled.push(*n),
                        false => {
                            alone.insert(*n);
                        }
                    }
                }
                Stmt::AssignMany { targets, .. } => {
                    tupled.extend(targets.iter().filter_map(|target| match target {
                        Var::Local(n) => Some(*n),
                        _ => None,
                    }));
                }
                _ => {}
            }
            scan.stmt(stmt);
        }
        if let Some(e) = block.term.expr() {
            scan.expr(e);
        }
    }
    scan.settle();
    for n in tupled {
        alone.remove(&n);
    }
    match refuted(&alone, &scan.extents) {
        true => BTreeSet::new(),
        false => alone,
    }
}

/// Whether a local is proved one cell wide, which `values::widths` alone says of one.
// ⚠ A local another pass declares a cell wide, with a kind or a name, is no proof of its width.
pub(in crate::gta6::script::passes) fn declared_single(info: &VarInfo) -> bool {
    info.single && info.width == Width::Cell
}

/// Whether a variable that `extents` shows to run over several cells holds a cell of `singles`.
fn refuted(singles: &BTreeSet<u32>, extents: &[(u32, u32)]) -> bool {
    extents
        .iter()
        .any(|(first, last)| last > first && singles.range(*first..=*last).next().is_some())
}

/// What holds of a function's variables wherever they are used.
pub(in crate::gta6::script::passes) struct Facts {
    pub uses: HashMap<Var, Uses>,
    /// The cells of each many-cell variable whose width something states.
    pub widths: HashMap<Var, u32>,
    /// The frame cells loads and stores reach through a local's own address.
    pub direct_locals: Vec<(u32, u32)>,
    /// The variables that only hold an address to load and store through, and what each may hold the address of.
    // `local_9 = &local_2; store_i32(local_9, 0, x);` hands the address to nothing: the store is
    // one of `local_2`'s own cell.
    holders: HashMap<Var, Vec<(Var, u32)>>,
    /// How many times each variable's address is given to such a holder.
    held: HashMap<Var, u32>,
    /// The lowest local, then the lowest parameter, whose address is taken at all.
    addressed: [Option<u32>; 2],
    /// The lowest parameter whose address goes where a later call or store may use it.
    escaped: Option<u32>,
    /// The local cells an address that goes there reaches, as runs `lo..=hi` in rising order that do not meet.
    // An address reaches the variable it is the address of and no other. Where a local is
    // declared one cell wide the variable ends there, or before the next such local above it;
    // where nothing bounds it the run ends at `u32::MAX`, every local from its own on.
    reach: Vec<(u32, u32)>,
    /// Whether two accesses are told apart by the variables their addresses are in (`escape`).
    fine: bool,
    /// The variables given an object the function makes, and where each object is still in no other hands.
    fresh: HashMap<Var, Window>,
}

std::thread_local! {
    /// The function this thread read facts of last, as it was, and what they were.
    // ⚠ Held whole and compared whole: the passes change a function in place and say so only by
    // what they return, and a round that changes nothing reads the same function a dozen times.
    static LAST: RefCell<Option<(cfg::Function, Rc<Facts>)>> = const { RefCell::new(None) };
}

impl Facts {
    /// What holds of `f`'s variables: the facts this thread read last where `f` is the same function as then, else read now.
    pub fn of(f: &cfg::Function) -> Rc<Facts> {
        let held = LAST.with(|last| {
            let last = last.borrow();
            let (seen, facts) = last.as_ref()?;
            (seen == f).then(|| Rc::clone(facts))
        });
        if let Some(facts) = held {
            return facts;
        }
        let facts = Rc::new(Self::read(f));
        LAST.with(|last| *last.borrow_mut() = Some((f.clone(), Rc::clone(&facts))));
        facts
    }

    fn read(f: &cfg::Function) -> Facts {
        let mut scan = Scan::default();
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                scan.stmt(stmt);
            }
            if let Some(e) = block.term.expr() {
                scan.expr(e);
            }
        }
        // What is done with a variable that holds an address counts for the variable the
        // address is of, whether or not the holder is taken at its word below.
        scan.settle();
        let Scan {
            uses,
            mut direct_locals,
            holds,
            through,
            extents,
            adrift,
            ..
        } = scan;
        // The locals declared one cell wide: what `values::widths` proved of the form a front
        // end made. One that a variable of several cells runs over is no proof.
        let every_local = Var::Local(0)..=Var::Local(u32::MAX);
        let mut singles: BTreeSet<u32> = (f.head.vars.range(every_local))
            .filter_map(|(var, info)| match var {
                Var::Local(n) if declared_single(info) => Some(*n),
                _ => None,
            })
            .collect();
        if refuted(&singles, &extents) {
            singles.clear();
        }
        // The last cell the address of the local `n` reaches, once it is in other hands.
        let last_reached = |n: u32| {
            let var = Var::Local(n);
            let cells = match f.head.vars.get(&var).map(|info| &info.kind) {
                Some(Kind::Aggregate(cells)) => (*cells).max(1),
                _ => 1,
            };
            if adrift.contains(&var) {
                return u32::MAX;
            }
            if cells == 1 && singles.contains(&n) {
                return n;
            }
            let above = n.saturating_add(cells - 1).checked_add(1);
            let next = above.and_then(|above| singles.range(above..).next());
            next.map_or(u32::MAX, |next| next.saturating_sub(1))
        };

        // A variable holds an address and nothing else where every assignment to it is of an
        // address, every read of it is the address a load or a store goes through, and nothing
        // takes its own address or reaches its cell.
        let mut given: HashMap<Var, u32> = HashMap::new();
        for (holder, ..) in &holds {
            *given.entry(*holder).or_default() += 1;
        }
        let mut gone_through: HashMap<Var, u32> = HashMap::new();
        for (holder, ..) in &through {
            *gone_through.entry(*holder).or_default() += 1;
        }
        let narrow =
            |var: &Var| (f.head.vars.get(var)).is_none_or(|info| info.width != Width::Cells);
        let mut sound: HashSet<Var> = given
            .iter()
            .filter(|(holder, assigned)| {
                let used = uses.get(holder).copied().unwrap_or_default();
                let derefs = gone_through.get(holder).copied().unwrap_or(0);
                let covered = match holder {
                    Var::Local(n) => direct_locals.iter().any(|(lo, hi)| lo <= n && n <= hi),
                    _ => false,
                };
                let only = used.defs == **assigned && used.reads == derefs && used.addrs == 0;
                only && !covered && narrow(holder)
            })
            .map(|(holder, _)| *holder)
            .collect();
        // ⚠ A holder a callee can reach hands its address on: an address that escapes reaches
        // every cell of its variable, and what those hold escapes with them.
        let mut held: HashMap<Var, u32>;
        let mut reach: Vec<(u32, u32)>;
        // The runs of local cells the addresses that escape reach, given the addresses holders keep.
        let reached = |held: &HashMap<Var, u32>| {
            let runs = uses.iter().filter_map(|(var, uses)| {
                let Var::Local(n) = var else {
                    return None;
                };
                let kept = held.get(var).copied().unwrap_or(0);
                (uses.addrs > uses.direct + kept).then(|| (*n, last_reached(*n)))
            });
            merged(runs.collect())
        };
        let lowest = |held: &HashMap<Var, u32>, params: bool, escaping: bool| {
            let cells = uses.iter().filter_map(|(var, uses)| {
                let n = match (var, params) {
                    (Var::Local(n), false) | (Var::Param(n), true) => *n,
                    _ => return None,
                };
                let kept = held.get(var).copied().unwrap_or(0);
                let counted = match escaping {
                    true => uses.addrs > uses.direct + kept,
                    false => uses.addrs > 0,
                };
                counted.then_some(n)
            });
            cells.min()
        };
        loop {
            held = HashMap::new();
            for (holder, var, _) in &holds {
                if sound.contains(holder) {
                    *held.entry(*var).or_default() += 1;
                }
            }
            reach = reached(&held);
            let before = sound.len();
            sound.retain(|holder| match holder {
                Var::Local(n) => !within(&reach, *n, *n),
                _ => true,
            });
            if sound.len() == before {
                break;
            }
        }
        let mut holders: HashMap<Var, Vec<(Var, u32)>> = HashMap::new();
        for (holder, var, cell) in &holds {
            if sound.contains(holder) {
                holders.entry(*holder).or_default().push((*var, *cell));
            }
        }
        // What goes through a holder is a load or a store of the cells its address is of.
        let mut spans: Vec<(u32, u32)> = Vec::new();
        for (holder, offset, bytes) in &through {
            for (var, cell) in holders.get(holder).into_iter().flatten() {
                if let Var::Local(n) = var {
                    spans.push(span(n.saturating_add(*cell), *offset, *bytes));
                }
            }
        }
        // ⚠ A store through one holder that covers another may change the address that one
        // holds: then no holder is taken at its word, and every address given to one escapes.
        let poisoned = holders.keys().any(|holder| match holder {
            Var::Local(n) => spans.iter().any(|(lo, hi)| lo <= n && n <= hi),
            _ => false,
        });
        if poisoned {
            holders.clear();
            held.clear();
            reach = reached(&held);
        } else {
            direct_locals.extend(spans);
        }
        let addressed = [lowest(&held, false, false), lowest(&held, true, false)];
        let escaped = lowest(&held, true, true);

        let mut widths: HashMap<Var, u32> = HashMap::new();
        for (var, info) in &f.head.vars {
            if let (false, Kind::Aggregate(cells)) = (var.is_temporary(), &info.kind) {
                widths.insert(*var, *cells);
            }
        }
        // A temporary assigned more than once: the assignments seen, and the cells all of them state alike.
        let mut several: HashMap<Var, (u32, Option<u32>)> = HashMap::new();
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                let Stmt::Assign { target, value } = stmt else {
                    continue;
                };
                let many = f
                    .head
                    .vars
                    .get(target)
                    .is_some_and(|info| info.width == Width::Cells);
                let defs = uses.get(target).map_or(0, |uses| uses.defs);
                if target.is_temporary() && many && defs >= 1 {
                    let cells = match value {
                        // A group of cells is as wide as it has cells.
                        Expr::Cells(items) if items.iter().all(|item| one_cell(&f.head, item)) => {
                            u32::try_from(items.len()).ok()
                        }
                        other => cells_of(other, &widths),
                    };
                    match (defs, cells) {
                        (1, Some(cells)) => {
                            widths.insert(*target, cells);
                        }
                        (1, None) => {}
                        _ => {
                            let seen = several.entry(*target).or_insert((0, cells));
                            seen.0 += 1;
                            if seen.1 != cells {
                                seen.1 = None;
                            }
                        }
                    }
                }
            }
        }
        // ⚠ Only where every assignment is one of those counted: a tuple's also assigns it.
        for (var, (assigned, cells)) in several {
            let defs = uses.get(&var).map_or(0, |uses| uses.defs);
            if let (true, Some(cells)) = (assigned == defs, cells) {
                widths.insert(var, cells);
            }
        }
        Facts {
            uses,
            widths,
            direct_locals,
            holders,
            held,
            addressed,
            escaped,
            reach,
            fine: false,
            fresh: HashMap::new(),
        }
    }

    /// [`Facts::of`] with the finer model of what two accesses may share: by the variable each address is in, and by the objects the function has just made.
    // ⚠ An object is in no other hands only until the first statement that mentions its variable
    // other than as the address of a load or a store: from there a copy of the reference may be
    // anywhere.
    pub fn finer(f: &cfg::Function) -> Rc<Facts> {
        let mut facts = Facts::read(f);
        facts.fine = true;
        for (b, block) in f.cfg.blocks.iter().enumerate() {
            for (i, stmt) in block.stmts.iter().enumerate() {
                let Stmt::Assign {
                    target,
                    value: Expr::NewObject(_) | Expr::NewArray { .. },
                } = stmt
                else {
                    continue;
                };
                let var = *target;
                let once = facts.uses.get(&var).is_some_and(|uses| uses.defs == 1);
                let narrow = (f.head.vars.get(&var)).is_none_or(|info| info.width != Width::Cells);
                if !(once && narrow && facts.named_only(var)) {
                    continue;
                }
                let end = block.stmts.len().min(i + 1 + HELD);
                let handed = (i + 1..end).find(|k| loose_in(&block.stmts[*k], var) > 0);
                let until = match handed {
                    Some(k) => k,
                    None if end < block.stmts.len() => end,
                    None => match block.term.expr().is_some_and(|e| loose(e, var) > 0) {
                        true => end,
                        false => end + 1,
                    },
                };
                let window = Window {
                    block: b,
                    from: i,
                    until,
                };
                facts.fresh.insert(var, window);
            }
        }
        Rc::new(facts)
    }

    /// Whether `var` holds an object the function made that is in no other hands at statement `at.1` of block `at.0`.
    pub fn fresh_at(&self, var: Var, at: (BlockId, usize)) -> bool {
        (self.fresh.get(&var)).is_some_and(|window| window.holds(Some(at)))
    }

    /// Whether `var` only holds the address of a variable, to load and store through.
    pub fn holds_address(&self, var: Var) -> bool {
        self.holders.contains_key(&var)
    }

    /// The access a load or a store of `bytes` bytes at `offset` from `addr` is, where the address is a variable only an assignment changes.
    fn based(&self, addr: &Expr, offset: u32, bytes: Option<u32>) -> Option<Based> {
        let Expr::Var(base) = addr else {
            return None;
        };
        (self.fine && self.named_only(*base)).then(|| Based {
            base: *base,
            lo: offset,
            hi: bytes.map(|bytes| offset.saturating_add(bytes.max(1) - 1)),
            window: self.fresh.get(base).copied(),
        })
    }

    /// The place a variable's own name reads or writes; a global has none.
    pub fn place(&self, var: Var) -> Option<Place> {
        let more = self.widths.get(&var).copied().unwrap_or(1).max(1) - 1;
        match var {
            Var::Local(n) => Some(Place::Frame {
                params: false,
                lo: n,
                hi: n.saturating_add(more),
            }),
            Var::Param(n) => Some(Place::Frame {
                params: true,
                lo: n,
                hi: n.saturating_add(more),
            }),
            Var::Global { .. } => None,
            temporary => Some(Place::Temp(temporary)),
        }
    }

    /// The place a load or a store of `bytes` bytes at `offset` from the address of cell `cell` of `var` reaches.
    fn place_at(var: Var, cell: u32, offset: u32, bytes: Option<u32>) -> Option<Place> {
        let frame = |params: bool, n: u32| {
            let (lo, hi) = span(n.saturating_add(cell), offset, bytes);
            Place::Frame { params, lo, hi }
        };
        match var {
            Var::Local(n) => Some(frame(false, n)),
            Var::Param(n) => Some(frame(true, n)),
            Var::Global { .. } => None,
            temporary => Some(Place::Temp(temporary)),
        }
    }

    /// Calls `f` on each place a load or a store of `bytes` bytes at `offset` from `addr` reaches; `false` where it may reach memory no place names.
    // Through a variable's own address it reaches that variable; through a holder, whichever
    // variable the holder has the address of, and it is still taken for a pointer's access.
    fn reached(
        &self,
        addr: &Expr,
        offset: u32,
        bytes: Option<u32>,
        f: &mut impl FnMut(Place),
    ) -> bool {
        match addr {
            Expr::AddrOf { var, cell } => match Facts::place_at(*var, *cell, offset, bytes) {
                Some(place) => {
                    f(place);
                    true
                }
                None => false,
            },
            Expr::Var(holder) => {
                for (var, cell) in self.holders.get(holder).into_iter().flatten() {
                    if let Some(place) = Facts::place_at(*var, *cell, offset, bytes) {
                        f(place);
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Whether a call or a store through a pointer may read or write `place`.
    pub fn exposed(&self, place: Place) -> bool {
        match place {
            Place::Frame {
                params: false,
                lo,
                hi,
            } => within(&self.reach, lo, hi),
            Place::Frame { hi, .. } => self.escaped.is_some_and(|low| hi >= low),
            Place::Temp(var) => {
                let kept = self.held.get(&var).copied().unwrap_or(0);
                (self.uses.get(&var)).is_some_and(|uses| uses.addrs > uses.direct + kept)
            }
        }
    }

    /// Whether nothing but an assignment that names `var` can change it: no load or store goes through an address that reaches it.
    // A local's address that only loads and stores go through reaches the cells those cover and
    // no other; one that goes elsewhere reaches every cell of its variable.
    pub fn named_only(&self, var: Var) -> bool {
        match self.place(var) {
            Some(Place::Frame {
                params: false,
                lo,
                hi,
            }) => {
                let covered =
                    (self.direct_locals.iter()).any(|(from, to)| *from <= hi && lo <= *to);
                !within(&self.reach, lo, hi) && !covered
            }
            Some(Place::Frame { params, hi, .. }) => {
                self.addressed[usize::from(params)].is_none_or(|low| hi < low)
            }
            Some(Place::Temp(var)) => self.uses.get(&var).is_none_or(|uses| uses.addrs == 0),
            None => false,
        }
    }

    /// Whether nothing reads `var`, by its name or through an address, so that assigning it does nothing.
    pub fn unread(&self, var: Var) -> bool {
        let read = self.uses.get(&var).is_some_and(|uses| uses.reads > 0);
        let kept = matches!(var, Var::Param(_) | Var::Global { .. });
        !read && !kept && self.named_only(var)
    }
}

/// What an intrinsic of the runtime does.
enum Class {
    /// Works on its operands alone.
    Pure,
    /// Reads memory and writes none.
    Reads,
    /// Makes an object that holds a copy of a value.
    Boxes,
    Calls,
}

/// What an intrinsic does, by its name; one this does not know is taken for a call.
fn class(name: &str) -> Class {
    // A typed intrinsic's name ends in its type: `try_cast<type_..>`.
    let base = name.split('<').next().unwrap_or(name);
    let converts = base.split_once("_to_").is_some_and(|(from, to)| {
        Scalar::from_name(from).is_some() && Scalar::from_name(to).is_some()
    });
    let family = ["abs_", "min_", "max_", "clamp_", "packed_vector_"]
        .iter()
        .any(|prefix| base.starts_with(*prefix));
    let maths = [
        "lerp_f32",
        "sign_f32",
        "round_half_up_f32",
        "sinf",
        "cosf",
        "tanf",
        "asinf",
        "acosf",
        "atanf",
        "atan2f",
        "sqrtf",
        "powf",
        "floorf",
        "ceilf",
    ]
    .contains(&base);
    if converts || family || maths {
        Class::Pure
    } else if base.starts_with("array_get_")
        || base.starts_with("bytes_equal_")
        || matches!(base, "array_length" | "array_element_address" | "try_cast")
    {
        Class::Reads
    } else if matches!(base, "box" | "box_at") {
        Class::Boxes
    } else {
        Class::Calls
    }
}

/// Whether evaluating `e` again, with nothing run since it was evaluated, gives what it gave and does nothing it did not.
// A load reads what it read, for nothing between wrote memory; a call to anything but an
// intrinsic that only works on or reads what it is given may not, and an allocation makes another object.
pub(in crate::gta6::script::passes) fn repeatable(e: &Expr) -> bool {
    let mut again = true;
    e.walk(&mut |node| {
        again &= match node {
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                ..
            } => matches!(class(name), Class::Pure | Class::Reads),
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. } => false,
            _ => true,
        };
    });
    again
}

/// What evaluating something does that bears on moving it past something else.
#[derive(Clone, Debug, Default)]
pub(in crate::gta6::script::passes) struct Effects {
    /// A call, which may read and write whatever a pointer reaches.
    pub call: bool,
    /// A write through a pointer or to a global.
    pub store: bool,
    /// A new object or array.
    pub alloc: bool,
    /// A read through a pointer or of a global.
    pub load: bool,
    /// A read, by name, of a variable a pointer reaches.
    reads_exposed: bool,
    /// A write, by name, of a variable a pointer reaches.
    writes_exposed: bool,
    reads: Vec<Place>,
    writes: Vec<Place>,
    /// The finer model of [`Facts::finer`] is the one asked for.
    fine: bool,
    /// A load, and a store, that the finer model knows no variable for: through a computed address, or of a global.
    loose_load: bool,
    loose_store: bool,
    /// The loads and the stores through a variable that holds an address.
    loads: Vec<Based>,
    stores: Vec<Based>,
    /// The statement these are the effects of, where one moving value is weighed against it.
    site: Option<(BlockId, usize)>,
}

impl Effects {
    pub fn of_expr(facts: &Facts, e: &Expr) -> Effects {
        let mut effects = Effects {
            fine: facts.fine,
            ..Effects::default()
        };
        e.walk(&mut |node| effects.node(facts, node));
        effects
    }

    pub fn of_stmt(facts: &Facts, stmt: &Stmt) -> Effects {
        let mut effects = Effects {
            fine: facts.fine,
            ..Effects::default()
        };
        stmt.each_expr(&mut |e| e.walk(&mut |node| effects.node(facts, node)));
        match stmt {
            Stmt::Assign { target, .. } => effects.write(facts, facts.place(*target)),
            Stmt::AssignMany { targets, .. } => {
                for target in targets {
                    effects.write(facts, facts.place(*target));
                }
            }
            Stmt::StepI32 { addr, offset, .. } => {
                effects.read_at(facts, addr, *offset, Some(4));
                effects.write_at(facts, addr, *offset, Some(4));
            }
            Stmt::Eval(_) => {}
            store => {
                if let Some((addr, offset, bytes)) = stored(store) {
                    effects.write_at(facts, addr, offset, bytes);
                }
            }
        }
        effects
    }

    /// Adds what evaluating `e` does itself, apart from what evaluating its operands does.
    pub fn node(&mut self, facts: &Facts, e: &Expr) {
        match e {
            Expr::Var(var) => self.read(facts, facts.place(*var)),
            Expr::Load { ty, addr, offset } => self.read_at(facts, addr, *offset, Some(ty.bytes())),
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.read_at(facts, addr, *offset, Some(*bytes)),
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                ..
            } => match class(name) {
                Class::Pure => {}
                Class::Reads => self.read(facts, None),
                Class::Boxes => {
                    self.alloc = true;
                    if name == "box_at" {
                        self.read(facts, None);
                    }
                }
                Class::Calls => self.call = true,
            },
            Expr::Call { .. } => self.call = true,
            Expr::NewObject(_) | Expr::NewArray { .. } => self.alloc = true,
            Expr::VirtualFunctionRef { .. } | Expr::BuiltinGlobal(_) => self.read(facts, None),
            _ => {}
        }
    }

    /// A load of `bytes` bytes at `offset` from `addr`.
    fn read_at(&mut self, facts: &Facts, addr: &Expr, offset: u32, bytes: Option<u32>) {
        let mut places = Vec::new();
        let known = facts.reached(addr, offset, bytes, &mut |place| places.push(place));
        for place in places {
            self.read(facts, Some(place));
        }
        match (known, facts.based(addr, offset, bytes)) {
            (true, _) => {}
            (false, Some(access)) => {
                self.load = true;
                self.loads.push(access);
            }
            (false, None) => self.read(facts, None),
        }
    }

    /// A store of `bytes` bytes at `offset` from `addr`.
    fn write_at(&mut self, facts: &Facts, addr: &Expr, offset: u32, bytes: Option<u32>) {
        let mut places = Vec::new();
        let known = facts.reached(addr, offset, bytes, &mut |place| places.push(place));
        for place in places {
            self.write(facts, Some(place));
        }
        match (known, facts.based(addr, offset, bytes)) {
            (true, _) => {}
            (false, Some(access)) => {
                self.store = true;
                self.stores.push(access);
            }
            (false, None) => self.write(facts, None),
        }
    }

    fn read(&mut self, facts: &Facts, place: Option<Place>) {
        match place {
            Some(place) => {
                self.reads_exposed |= facts.exposed(place);
                self.reads.push(place);
            }
            None => {
                self.load = true;
                self.loose_load = true;
            }
        }
    }

    fn write(&mut self, facts: &Facts, place: Option<Place>) {
        match place {
            Some(place) => {
                self.writes_exposed |= facts.exposed(place);
                self.writes.push(place);
            }
            None => {
                self.store = true;
                self.loose_store = true;
            }
        }
    }

    /// These, as the effects of the statement at `site`.
    pub fn placed(mut self, site: (BlockId, usize)) -> Effects {
        self.site = Some(site);
        self
    }

    /// Whether this may change what a load through a pointer reads.
    pub fn writes_memory(&self) -> bool {
        self.call || self.store || self.writes_exposed
    }

    /// Whether no store of `crossed` can write what a load of this reads, as far as the finer model tells two addresses apart.
    // Through one variable two accesses meet only where their bytes do. Through two, they may
    // meet unless one of the variables holds an object that is in no other hands where
    // `crossed` stands.
    fn clear_of(&self, crossed: &Effects) -> bool {
        if !self.fine || crossed.loose_store {
            return false;
        }
        crossed.stores.iter().all(|store| {
            let fresh = store
                .window
                .is_some_and(|window| window.holds(crossed.site));
            let others = !self.loose_load || fresh;
            others
                && self.loads.iter().all(|load| match load.base == store.base {
                    true => load.apart(store),
                    false => fresh || load.window.is_some_and(|window| window.holds(crossed.site)),
                })
        })
    }

    /// Whether evaluating this only reads variables and works on them: it changes nothing and cannot fail.
    pub fn free(&self) -> bool {
        !(self.call || self.store || self.alloc || self.load)
    }

    /// Whether this writes `place` by name or through its own address.
    pub fn writes(&self, place: Place) -> bool {
        self.writes.iter().any(|written| written.overlaps(place))
    }

    /// Whether this reads or writes `place` by name or through its own address.
    pub fn touches(&self, place: Place) -> bool {
        let named = self.reads.iter().chain(&self.writes);
        named.into_iter().any(|touched| touched.overlaps(place))
    }

    /// Whether an expression with these effects, evaluated before `crossed`, may be evaluated after it instead.
    pub fn may_cross(&self, crossed: &Effects) -> bool {
        let through_pointer = crossed.call || crossed.store;
        // A call keeps its place among calls, stores, loads and allocations, and sees every variable a pointer reaches.
        let seen = crossed.reads_exposed || crossed.writes_exposed;
        // An allocation reads nothing a call or a store writes and writes nothing either reads:
        // the finer model lets the two change places.
        let coarse = !self.fine;
        if self.call && (through_pointer || (crossed.alloc && coarse) || crossed.load || seen) {
            return false;
        }
        if (self.reads_exposed || (self.alloc && coarse)) && through_pointer {
            return false;
        }
        let stored = crossed.store && !self.clear_of(crossed);
        if self.load && (crossed.call || stored || crossed.writes_exposed) {
            return false;
        }
        !self
            .reads
            .iter()
            .any(|read| crossed.writes.iter().any(|written| read.overlaps(*written)))
    }
}
