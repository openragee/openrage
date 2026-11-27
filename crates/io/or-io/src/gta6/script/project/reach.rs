//! Reach: how many bytes from an address handed to a function the function may touch, for each
//! parameter cell, over every program of the project, and how it uses them.
//!
//! A function's own code says what its loads and stores through the cell reach, and to which
//! parameter cells of which functions it hands the cell on, moved by how many bytes; `settle`
//! carries the reach of those along the calls until nothing changes. A cell the function does
//! anything else with (keeps, compares, works on, hands to a native, a builtin or a slot) reaches
//! without bound, and so does one handed to a function the project holds no reach of.
//!
//! Of a cell whose every use is accounted for, the reading also keeps the uses in the order
//! control may take them, and `settle` runs them along the calls: whether the function reads the
//! bytes as the caller left them, writes them, writes them on every path to a return, and whether
//! the address outlives the call.
//!
//! ⚠ What a function reaches is a fact of its code alone: nothing here is taken from its callers.
//! ⚠ Each fact is of what goes through the one cell: another argument, or an address kept before
//! the call, may reach the same bytes.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::signatures::key_of;
use super::Project;
use crate::gta6::script::cfg::{self, BlockId};
use crate::gta6::script::ir::{BinOp, Expr, FunctionKey, Head, Stmt, Var};
use crate::gta6::script::print::cells_of;

/// The reach of a cell that may reach anything.
pub const UNBOUNDED: u32 = u32::MAX;
/// The bytes a reach is carried to before it is taken to be unbounded: a ring of calls that each move the address grows it for good.
const MOST: u32 = 1 << 20;
/// The rounds `settle` is given; a reach still growing after them is unbounded.
const ROUNDS: usize = 4096;
/// The times one reach may grow before it is taken to grow for good.
const WIDEN: u32 = 16;
/// The parameter cells of a function looked at: past them a cell's reach is unknown.
const MAX_PARAMS: u32 = 1024;
/// The temporaries in a row a cell may be copied through and still be followed.
const COPIES: usize = 8;
/// The ranges a set of bytes is kept to: past them one that may be larger is taken whole, and one that must be smaller loses its last ranges.
const RANGES: usize = 32;
/// The blocks the reading of one function may step through to order the uses of its cells.
const VISITS: usize = 1 << 22;
/// The runs of each block, or of each cell, the uses are given to settle in; uses still changing after them are told nothing.
const PASSES: usize = 64;

/// A cell handed on: to parameter cell `cell` of `to`, moved `by` bytes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Handed {
    pub to: FunctionKey,
    pub cell: u32,
    pub by: u32,
}

/// How a function uses the bytes the address in one of its parameter cells points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Neither read nor written.
    Untouched,
    /// Read and never written.
    In,
    /// Written before any read of them on every path, and every byte written is written on every path to a return.
    Out,
    /// Written, and read as the caller left them or written on some paths only.
    Ref,
}

/// What one statement does through a cell: every read is taken to come before every write.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// The bytes read, each run `(start, end)` from the address.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<(u32, u32)>,
    /// The bytes written, each run `(start, end)` from the address.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<(u32, u32)>,
    /// The calls the cell is handed to, each with whether every path through the statement makes it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<(Handed, bool)>,
}

/// A block that uses a cell: its steps in order, and where control may go after them with no use between.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub steps: Vec<Step>,
    /// The nodes control may come to next.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<u32>,
    /// Whether control may come to a return next.
    #[serde(default)]
    pub returns: bool,
}

/// The uses of one cell in the order control may take them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub nodes: Vec<Node>,
    /// The nodes control may come to first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starts: Vec<u32>,
    /// Whether control may come to a return with no use on the way.
    #[serde(default)]
    pub returns: bool,
}

/// What the uses of one cell come to, each set of bytes as runs `(start, end)` from the address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uses {
    /// The bytes that may be read as the caller left them.
    pub reads: Vec<(u32, u32)>,
    /// The bytes that may be written.
    pub writes: Vec<(u32, u32)>,
    /// The bytes every path to a return writes.
    pub written: Vec<(u32, u32)>,
}

/// What one parameter cell of a function reaches as an address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reach {
    /// The bytes from the address the function may touch through the cell, [`UNBOUNDED`] where nothing bounds them.
    pub bytes: u32,
    /// Where the function's own code hands the cell on, until [`settle`] has carried what those reach into `bytes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handed: Vec<Handed>,
    /// How the function uses the bytes, `None` where it cannot be told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// Where `mode` is [`Mode::Out`]: the bytes from the address on that every path to a return writes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub covered: u32,
    /// Whether nothing the function, or a function it hands the cell to, does keeps the address past the call; before [`settle`], of the function's own code alone.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dies: bool,
    /// The uses of the cell where it is handed on, until [`settle`] has run them into `mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Flow>,
    /// What the uses come to where the cell is handed to no function, until [`settle`] has made `mode` of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<Uses>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Byte runs from an address: sorted, apart, none empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Span(Vec<(u32, u32)>);

impl Span {
    /// Every byte.
    fn all() -> Span {
        Span(vec![(0, u32::MAX)])
    }

    /// The bytes of `runs`, each `(start, end)`, in any order.
    fn of(runs: impl IntoIterator<Item = (u32, u32)>) -> Span {
        let mut runs: Vec<(u32, u32)> = runs.into_iter().filter(|(a, b)| a < b).collect();
        runs.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(runs.len());
        for (a, b) in runs {
            if let Some(last) = out.last_mut() {
                if a <= last.1 {
                    last.1 = last.1.max(b);
                    continue;
                }
            }
            out.push((a, b));
        }
        Span(out)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn union(&self, other: &Span) -> Span {
        Span::of(self.0.iter().chain(&other.0).copied())
    }

    fn meet(&self, other: &Span) -> Span {
        let (mut i, mut j) = (0, 0);
        let mut out = Vec::new();
        while let (Some(&(a, b)), Some(&(c, d))) = (self.0.get(i), other.0.get(j)) {
            let (lo, hi) = (a.max(c), b.min(d));
            if lo < hi {
                out.push((lo, hi));
            }
            match b <= d {
                true => i += 1,
                false => j += 1,
            }
        }
        Span::of(out)
    }

    /// The bytes of this span that `other` does not hold.
    fn minus(&self, other: &Span) -> Span {
        let mut out = Vec::new();
        for &(a, b) in &self.0 {
            let mut at = a;
            for &(c, d) in &other.0 {
                if d <= at {
                    continue;
                }
                if c >= b {
                    break;
                }
                if c > at {
                    out.push((at, c));
                }
                at = at.max(d);
                if at >= b {
                    break;
                }
            }
            if at < b {
                out.push((at, b));
            }
        }
        Span::of(out)
    }

    /// The span moved `by` bytes on; what passes the last byte is lost.
    fn shift(&self, by: u32) -> Span {
        let moved = self
            .0
            .iter()
            .map(|(a, b)| (a.saturating_add(by), b.saturating_add(by)));
        Span::of(moved)
    }

    fn covers(&self, other: &Span) -> bool {
        other.minus(self).is_empty()
    }

    /// The bytes from the address on that the span holds with no gap.
    fn prefix(&self) -> u32 {
        match self.0.first() {
            Some(&(0, end)) => end,
            _ => 0,
        }
    }

    /// The span or more, in at most [`RANGES`] runs.
    fn widened(self) -> Span {
        let hull = match (self.0.first(), self.0.last()) {
            (Some(first), Some(last)) if self.0.len() > RANGES => Some((first.0, last.1)),
            _ => None,
        };
        match hull {
            Some(run) => Span(vec![run]),
            None => self,
        }
    }

    /// The span or less, in at most [`RANGES`] runs.
    fn narrowed(mut self) -> Span {
        self.0.truncate(RANGES);
        self
    }
}

/// What one function shows of one cell while its code is read.
#[derive(Default)]
struct Shown {
    bytes: u32,
    handed: BTreeSet<Handed>,
    /// The places the cell, or a copy of it, stands that the reading accounts for.
    accounted: u64,
}

/// How an access goes through a cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Touch {
    Read,
    Write,
    Both,
}

/// One reading of a function: the cells of its parameters, by the variables that hold them, and what each does.
struct Reading<'a> {
    head: &'a Head,
    /// The variables that hold a parameter cell's address, moved by a count of bytes.
    holds: HashMap<Var, (u32, u32)>,
    shown: BTreeMap<u32, Shown>,
    /// What the statement being read does through each cell.
    step: BTreeMap<u32, Step>,
}

impl Reading<'_> {
    /// The parameter cell and the bytes `e` is moved from it, where `e` is one held, moved by constants.
    fn held(&self, e: &Expr) -> Option<(u32, u32)> {
        let mut e = e;
        let mut by = 0u32;
        loop {
            match e {
                Expr::Var(var) => {
                    let (cell, at) = *self.holds.get(var)?;
                    return Some((cell, at.checked_add(by)?));
                }
                Expr::ByteOffset { addr, offset } => {
                    let Expr::Const(c) = &**offset else {
                        return None;
                    };
                    by = by.checked_add(u32::try_from(c.bits).ok()?)?;
                    e = &**addr;
                }
                _ => return None,
            }
        }
    }

    /// Notes an access of `bytes` bytes at `offset` from `addr`, where `addr` holds a parameter cell.
    fn access(&mut self, addr: &Expr, offset: u32, bytes: u32, how: Touch) {
        if let Some((cell, by)) = self.held(addr) {
            let start = by.checked_add(offset);
            let end = start.and_then(|at| at.checked_add(bytes));
            let shown = self.shown.entry(cell).or_default();
            shown.bytes = shown.bytes.max(end.unwrap_or(UNBOUNDED));
            shown.accounted += 1;
            if let (Some(start), Some(end)) = (start, end) {
                let step = self.step.entry(cell).or_default();
                if how != Touch::Write {
                    step.reads.push((start, end));
                }
                if how != Touch::Read {
                    step.writes.push((start, end));
                }
            }
        }
    }

    /// Reads `top`; `whole` is the call in it that every path through the statement makes, if any.
    fn expr(&mut self, top: &Expr, whole: Option<&Expr>) {
        top.walk(&mut |e| match e {
            Expr::Load { ty, addr, offset } => self.access(addr, *offset, ty.bytes(), Touch::Read),
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.access(addr, *offset, *bytes, Touch::Read),
            // Held against zero, the address touches nothing.
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                l,
                r,
                ..
            } => {
                for (held, other) in [(l, r), (r, l)] {
                    let zero = matches!(&**other, Expr::Const(c) if c.bits == 0);
                    let mut held = &**held;
                    while let Expr::Cast { e, .. } = held {
                        held = &**e;
                    }
                    if let (true, Some((cell, _))) = (zero, self.held(held)) {
                        self.shown.entry(cell).or_default().accounted += 1;
                    }
                }
            }
            Expr::Call { callee, args, .. } => {
                let Some(to) = key_of(self.head.program, callee) else {
                    return;
                };
                let made = whole.is_some_and(|whole| std::ptr::eq(whole, e));
                // The cell an argument lands in: the cells of the arguments before it.
                let mut at = Some(0u32);
                for arg in args {
                    if let (Some(cell), Some((held, by))) = (at, self.held(arg)) {
                        let handed = Handed { to, cell, by };
                        let shown = self.shown.entry(held).or_default();
                        shown.handed.insert(handed);
                        shown.accounted += 1;
                        self.step
                            .entry(held)
                            .or_default()
                            .calls
                            .push((handed, made));
                    }
                    at = at
                        .zip(cells_of(arg, self.head))
                        .and_then(|(a, b)| a.checked_add(b));
                }
            }
            _ => {}
        });
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Store {
                kind, addr, offset, ..
            } => self.access(addr, *offset, kind.bytes(), Touch::Write),
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                ..
            } => self.access(addr, *offset, *bytes, Touch::Write),
            Stmt::ZeroBytes { addr, bytes } => self.access(addr, 0, *bytes, Touch::Write),
            Stmt::StepI32 { addr, offset, .. } => self.access(addr, *offset, 4, Touch::Both),
            Stmt::Assign { target, value } if self.holds.contains_key(target) => {
                // The copy that made a holder: the cell stands there to be held, no more.
                if let Some((cell, _)) = self.held(value) {
                    self.shown.entry(cell).or_default().accounted += 1;
                }
            }
            _ => {}
        }
        // A call the statement is: every path through the statement makes it.
        let whole = match stmt {
            Stmt::Eval(e) | Stmt::Assign { value: e, .. } if matches!(e, Expr::Call { .. }) => {
                Some(e)
            }
            _ => None,
        };
        stmt.each_expr(&mut |top| self.expr(top, whole));
    }

    /// Ends the statement being read in `block`: what it did through each cell is the next step of the block.
    // ⚠ Steps with no call in a row are one: each read the earlier ones wrote first is no read of
    // what the caller left, and the rest are read before any of their writes.
    fn close(&mut self, block: BlockId, uses: &mut BTreeMap<u32, BTreeMap<BlockId, Vec<Step>>>) {
        for (cell, step) in std::mem::take(&mut self.step) {
            let steps = uses.entry(cell).or_default().entry(block).or_default();
            let alone = step.calls.is_empty();
            if let Some(last) = steps
                .last_mut()
                .filter(|last| alone && last.calls.is_empty())
            {
                let before = Span::of(last.writes.iter().copied());
                let reads = Span::of(step.reads.iter().copied()).minus(&before);
                let reads = Span::of(last.reads.iter().copied()).union(&reads);
                let writes = before.union(&Span::of(step.writes.iter().copied()));
                last.reads = reads.0;
                last.writes = writes.0;
                continue;
            }
            steps.push(step);
        }
    }
}

/// The temporaries that hold a parameter cell's address and nothing else: assigned once, from a parameter or another such temporary, moved by constants, and never addressed.
fn holders(f: &cfg::Function, params: u32) -> HashMap<Var, (u32, u32)> {
    let mut sets: HashMap<Var, u32> = HashMap::new();
    let mut addressed: BTreeSet<Var> = BTreeSet::new();
    let mut copies: Vec<(Var, &Expr)> = Vec::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            for target in stmt.targets() {
                *sets.entry(*target).or_default() += 1;
            }
            if let Stmt::Assign { target, value } = stmt {
                if target.is_temporary() {
                    copies.push((*target, value));
                }
            }
            stmt.each_expr(&mut |top| {
                top.walk(&mut |e| {
                    if let Expr::AddrOf { var, .. } = e {
                        addressed.insert(*var);
                    }
                })
            });
        }
        // A test, a switch's value or a value returned may take an address too.
        if let Some(e) = block.term.expr() {
            e.walk(&mut |e| {
                if let Expr::AddrOf { var, .. } = e {
                    addressed.insert(*var);
                }
            });
        }
    }
    let mut holds: HashMap<Var, (u32, u32)> = HashMap::new();
    // ⚠ An address of a parameter reaches every cell from it on, as signatures and layouts take
    // it: whatever it is handed to may overwrite the cells past it.
    let lowest = addressed
        .iter()
        .filter_map(|var| match var {
            Var::Param(n) => Some(*n),
            _ => None,
        })
        .min()
        .unwrap_or(u32::MAX);
    for cell in 0..params.min(MAX_PARAMS).min(lowest) {
        let var = Var::Param(cell);
        // A parameter the function writes holds the caller's address no longer.
        if !sets.contains_key(&var) {
            holds.insert(var, (cell, 0));
        }
    }
    for _ in 0..COPIES {
        let reading = Reading {
            head: &f.head,
            holds: holds.clone(),
            shown: BTreeMap::new(),
            step: BTreeMap::new(),
        };
        let mut grew = false;
        for (target, value) in &copies {
            let once = sets.get(target) == Some(&1) && !addressed.contains(target);
            if let (true, false, Some(held)) =
                (once, holds.contains_key(target), reading.held(value))
            {
                holds.insert(*target, held);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    holds
}

/// The blocks one walk has stepped through, by the walk that last did.
struct Seen {
    marks: Vec<usize>,
    walk: usize,
}

/// Where control may come after leaving for `from` with no use on the way: the nodes, and whether a return; `None` past `budget` blocks.
fn ahead(
    blocks: &[cfg::Block],
    index: &BTreeMap<BlockId, u32>,
    from: Vec<BlockId>,
    seen: &mut Seen,
    budget: &mut usize,
) -> Option<(Vec<u32>, bool)> {
    seen.walk = seen.walk.checked_add(1)?;
    let mut stack = from;
    let mut next = Vec::new();
    let mut returns = false;
    while let Some(b) = stack.pop() {
        let mark = seen.marks.get_mut(b)?;
        if *mark == seen.walk {
            continue;
        }
        *mark = seen.walk;
        *budget = budget.checked_sub(1)?;
        if let Some(node) = index.get(&b) {
            next.push(*node);
            continue;
        }
        let block = blocks.get(b)?;
        returns |= matches!(block.term, cfg::Terminator::Return(_));
        stack.extend(block.term.successors());
    }
    next.sort_unstable();
    Some((next, returns))
}

/// The uses of one cell, by the blocks that hold them, in the order control may take them.
// ⚠ A block no path from the entry reaches is never run: a function that has regions a fault
// leaves for a handler is not lifted, so no block is entered by a way the terminators do not show.
fn flow(
    f: &cfg::Function,
    uses: BTreeMap<BlockId, Vec<Step>>,
    seen: &mut Seen,
    budget: &mut usize,
) -> Option<Flow> {
    let blocks = &f.cfg.blocks;
    let mut index = BTreeMap::new();
    for (n, b) in uses.keys().enumerate() {
        index.insert(*b, u32::try_from(n).ok()?);
    }
    let (starts, returns) = ahead(blocks, &index, vec![cfg::ENTRY], seen, budget)?;
    let mut nodes = Vec::with_capacity(uses.len());
    for (b, steps) in uses {
        let block = blocks.get(b)?;
        let (next, onward) = ahead(blocks, &index, block.term.successors(), seen, budget)?;
        let returns = onward || matches!(block.term, cfg::Terminator::Return(_));
        nodes.push(Node {
            steps,
            next,
            returns,
        });
    }
    Some(Flow {
        nodes,
        starts,
        returns,
    })
}

/// Adds what one function's code shows of what each of its parameter cells reaches.
pub fn of_function(project: &mut Project, f: &cfg::Function) {
    let params = f.head.params;
    if params == 0 || params > MAX_PARAMS || f.head.incomplete.is_some() {
        return;
    }
    let holds = holders(f, params);
    let mut reading = Reading {
        head: &f.head,
        holds,
        shown: BTreeMap::new(),
        step: BTreeMap::new(),
    };
    // Every place a holder stands: the places the reading does not account for reach anything.
    let mut stands: BTreeMap<u32, u64> = BTreeMap::new();
    let mut count = |top: &Expr, holds: &HashMap<Var, (u32, u32)>| {
        top.walk(&mut |e| match e {
            Expr::Var(var) | Expr::AddrOf { var, .. } => {
                if let Some((cell, _)) = holds.get(var) {
                    *stands.entry(*cell).or_default() += 1;
                }
            }
            _ => {}
        });
    };
    let mut uses: BTreeMap<u32, BTreeMap<BlockId, Vec<Step>>> = BTreeMap::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for stmt in &block.stmts {
            reading.stmt(stmt);
            stmt.each_expr(&mut |top| count(top, &reading.holds));
            reading.close(b, &mut uses);
        }
        if let Some(e) = block.term.expr() {
            // A condition, a switch's value or a returned value is evaluated on every way out of the block.
            let whole = matches!(e, Expr::Call { .. }).then_some(e);
            reading.expr(e, whole);
            count(e, &reading.holds);
            reading.close(b, &mut uses);
        }
    }
    let mut seen = Seen {
        marks: vec![0; f.cfg.blocks.len()],
        walk: 0,
    };
    let mut budget = VISITS;
    let mut cells = BTreeMap::new();
    for cell in 0..params {
        let reach = match reading.holds.get(&Var::Param(cell)) {
            // Written or addressed, the parameter may be anything at its uses.
            None => Reach {
                bytes: UNBOUNDED,
                ..Reach::default()
            },
            Some(_) => {
                let shown = reading.shown.remove(&cell).unwrap_or_default();
                let all = stands.get(&cell).copied().unwrap_or(0) <= shown.accounted;
                let bytes = if all { shown.bytes } else { UNBOUNDED };
                let used = uses.remove(&cell).unwrap_or_default();
                let order = match bytes == UNBOUNDED {
                    true => None,
                    false => flow(f, used, &mut seen, &mut budget),
                };
                // Handed to no function, what the uses come to is known now.
                let alone = order.as_ref().is_some_and(|order| {
                    let mut steps = order.nodes.iter().flat_map(|node| &node.steps);
                    steps.all(|step| step.calls.is_empty())
                });
                let (order, come_to) = match (alone, order) {
                    (true, Some(order)) => {
                        let come_to = evaluate(&order, &BTreeMap::new()).map(Uses::from);
                        (None, come_to)
                    }
                    (_, order) => (order, None),
                };
                Reach {
                    bytes,
                    handed: shown.handed.into_iter().collect(),
                    dies: all,
                    flow: order,
                    uses: come_to,
                    ..Reach::default()
                }
            }
        };
        cells.insert(cell, reach);
    }
    project
        .reach
        .insert(FunctionKey::new(f.head.program, f.head.at), cells);
}

/// Moves the reaches of `from` into `into`: a function both hold keeps the first.
pub fn absorb(into: &mut Project, from: &mut Project) {
    for (key, cells) in std::mem::take(&mut from.reach) {
        into.reach.entry(key).or_insert(cells);
    }
}

/// Once every part is in: carries what each handed-on cell reaches into the function that hands it on, until nothing changes.
// ⚠ A reach that has grown `WIDEN` times is taken to grow for good: a ring of calls that move
// the address would otherwise grow a little every round.
pub fn settle(project: &mut Project) {
    let reach = &mut project.reach;
    let mut grown: HashMap<(FunctionKey, u32), u32> = HashMap::new();
    let mut known: HashMap<(FunctionKey, u32), u32> = reach
        .iter()
        .flat_map(|(key, cells)| cells.iter().map(|(cell, r)| ((*key, *cell), r.bytes)))
        .collect();
    for _ in 0..ROUNDS {
        let mut grew = false;
        for (key, cells) in reach.iter_mut() {
            for (cell, held) in cells.iter_mut() {
                if held.bytes == UNBOUNDED {
                    continue;
                }
                let mut most = held.bytes;
                for handed in &held.handed {
                    let theirs = known.get(&(handed.to, handed.cell)).copied();
                    let end = theirs
                        .filter(|bytes| *bytes != UNBOUNDED)
                        .and_then(|bytes| bytes.checked_add(handed.by))
                        .filter(|end| *end <= MOST)
                        .unwrap_or(UNBOUNDED);
                    most = most.max(end);
                }
                if most > held.bytes {
                    let times = grown.entry((*key, *cell)).or_default();
                    *times += 1;
                    held.bytes = if *times >= WIDEN { UNBOUNDED } else { most };
                    known.insert((*key, *cell), held.bytes);
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    // Still growing past every round: what has not settled is bounded by nothing.
    for cells in reach.values_mut() {
        for held in cells.values_mut() {
            let settled = held.handed.iter().all(|handed| {
                let theirs = known.get(&(handed.to, handed.cell)).copied();
                let end = theirs.and_then(|bytes| bytes.checked_add(handed.by));
                end.is_some_and(|end| end <= held.bytes)
            });
            if !settled {
                held.bytes = UNBOUNDED;
            }
        }
    }
    settle_uses(reach);
    for cells in reach.values_mut() {
        for held in cells.values_mut() {
            held.handed.clear();
            held.flow = None;
            held.uses = None;
        }
    }
}

/// A parameter cell of a function.
type CellKey = (FunctionKey, u32);

/// What a cell's uses come to, as they stand while [`settle_uses`] runs them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Summary {
    /// The bytes that may be read as the caller left them.
    reads: Span,
    /// The bytes that may be written.
    writes: Span,
    /// The bytes every path to a return writes.
    written: Span,
}

impl Summary {
    /// What is taken of a cell before its uses are run: nothing read or written, and every byte written on the way to a return.
    fn first() -> Summary {
        Summary {
            reads: Span::default(),
            writes: Span::default(),
            written: Span::all(),
        }
    }

    fn of(uses: &Uses) -> Summary {
        Summary {
            reads: Span::of(uses.reads.iter().copied()),
            writes: Span::of(uses.writes.iter().copied()),
            written: Span::of(uses.written.iter().copied()),
        }
    }

    fn mode(&self) -> Mode {
        match (self.writes.is_empty(), self.reads.is_empty()) {
            (true, true) => Mode::Untouched,
            (true, false) => Mode::In,
            (false, true) if self.written.covers(&self.writes) => Mode::Out,
            (false, _) => Mode::Ref,
        }
    }
}

impl From<Summary> for Uses {
    fn from(summary: Summary) -> Uses {
        Uses {
            reads: summary.reads.0,
            writes: summary.writes.0,
            written: summary.written.0,
        }
    }
}

/// Runs the steps of `node` from `state`, the bytes written on every path to it, to the bytes written on every path through it; with `seen`, adds to it what may be read as the caller left it and what may be written. `None` where a call goes to a cell `summaries` does not hold.
fn run_node(
    node: &Node,
    mut state: Span,
    summaries: &BTreeMap<CellKey, Summary>,
    mut seen: Option<&mut (Span, Span)>,
) -> Option<Span> {
    for step in &node.steps {
        let mut read = Span::of(step.reads.iter().copied());
        let mut wrote = Span::of(step.writes.iter().copied());
        // What every path through the step writes: a call made on some paths only writes nothing sure.
        let mut made = wrote.clone();
        for (handed, whole) in &step.calls {
            let theirs = summaries.get(&(handed.to, handed.cell))?;
            read = read.union(&theirs.reads.shift(handed.by));
            wrote = wrote.union(&theirs.writes.shift(handed.by));
            if *whole {
                made = made.union(&theirs.written.shift(handed.by));
            }
        }
        if let Some((reads, writes)) = seen.as_deref_mut() {
            *reads = reads.union(&read.minus(&state)).widened();
            *writes = writes.union(&wrote).widened();
        }
        state = state.union(&made).narrowed();
    }
    Some(state)
}

/// What the uses of one cell come to with what `summaries` holds of the cells it is handed to; `None` where they do not settle or a call goes to a cell `summaries` does not hold.
fn evaluate(flow: &Flow, summaries: &BTreeMap<CellKey, Summary>) -> Option<Summary> {
    let count = flow.nodes.len();
    // The bytes written on every path to each node, `None` until a path to it is seen.
    let mut ins: Vec<Option<Span>> = vec![None; count];
    let mut queued = vec![false; count];
    let mut work: VecDeque<usize> = VecDeque::new();
    for start in &flow.starts {
        let at = usize::try_from(*start).ok()?;
        *ins.get_mut(at)? = Some(Span::default());
        if !std::mem::replace(queued.get_mut(at)?, true) {
            work.push_back(at);
        }
    }
    let mut budget = count.saturating_add(1).saturating_mul(PASSES);
    while let Some(at) = work.pop_front() {
        budget = budget.checked_sub(1)?;
        *queued.get_mut(at)? = false;
        let node = flow.nodes.get(at)?;
        let Some(state) = ins.get(at)?.clone() else {
            continue;
        };
        let out = run_node(node, state, summaries, None)?;
        for next in &node.next {
            let to = usize::try_from(*next).ok()?;
            let slot = ins.get_mut(to)?;
            let merged = match slot.as_ref() {
                Some(held) => held.meet(&out),
                None => out.clone(),
            };
            if slot.as_ref() != Some(&merged) {
                *slot = Some(merged);
                if !std::mem::replace(queued.get_mut(to)?, true) {
                    work.push_back(to);
                }
            }
        }
    }
    let mut seen = (Span::default(), Span::default());
    let mut written: Option<Span> = flow.returns.then(Span::default);
    for (node, state) in flow.nodes.iter().zip(&ins) {
        let Some(state) = state.clone() else {
            continue;
        };
        let out = run_node(node, state, summaries, Some(&mut seen))?;
        if node.returns {
            written = Some(match written {
                Some(held) => held.meet(&out),
                None => out,
            });
        }
    }
    Some(Summary {
        reads: seen.0,
        writes: seen.1,
        written: written.unwrap_or_else(Span::all),
    })
}

/// The reach of one cell, where the project holds it.
fn get(reach: &BTreeMap<FunctionKey, BTreeMap<u32, Reach>>, at: CellKey) -> Option<&Reach> {
    reach.get(&at.0)?.get(&at.1)
}

/// Drops what is held of `at` and of every cell that hands on to it, however far.
fn forget(
    at: CellKey,
    summaries: &mut BTreeMap<CellKey, Summary>,
    callers: &BTreeMap<CellKey, Vec<CellKey>>,
) {
    let mut work = vec![at];
    while let Some(at) = work.pop() {
        if summaries.remove(&at).is_some() {
            work.extend(callers.get(&at).into_iter().flatten().copied());
        }
    }
}

/// Once every reach is settled: whether each address dies with the call, and how each cell's bytes are used, carried along the calls until nothing changes.
// ⚠ What every path writes starts at every byte and only shrinks, what may be read or written
// starts at none and only grows: a ring of calls settles on what its paths that return show.
fn settle_uses(reach: &mut BTreeMap<FunctionKey, BTreeMap<u32, Reach>>) {
    // The cells that hand on to each cell.
    let mut callers: BTreeMap<CellKey, Vec<CellKey>> = BTreeMap::new();
    for (key, cells) in reach.iter() {
        for (cell, held) in cells {
            for handed in &held.handed {
                let to = callers.entry((handed.to, handed.cell)).or_default();
                to.push((*key, *cell));
            }
        }
    }
    for list in callers.values_mut() {
        list.sort_unstable();
        list.dedup();
    }

    // A cell its own code may keep, or that it hands to one the project holds no reach of, is kept, and so is every cell handed on to it.
    let mut kept: BTreeSet<CellKey> = BTreeSet::new();
    let mut work: Vec<CellKey> = Vec::new();
    for (key, cells) in reach.iter() {
        for (cell, held) in cells {
            let lost = (held.handed.iter()).any(|h| get(reach, (h.to, h.cell)).is_none());
            if !held.dies || lost {
                kept.insert((*key, *cell));
                work.push((*key, *cell));
            }
        }
    }
    while let Some(at) = work.pop() {
        for caller in callers.get(&at).into_iter().flatten() {
            if kept.insert(*caller) {
                work.push(*caller);
            }
        }
    }

    // The cells whose uses can be told: bounded, read in order, and handed only to such cells.
    let mut known: BTreeSet<CellKey> = BTreeSet::new();
    for (key, cells) in reach.iter() {
        for (cell, held) in cells {
            let read = held.flow.is_some() || held.uses.is_some();
            if read && held.bytes != UNBOUNDED {
                known.insert((*key, *cell));
            }
        }
    }
    let mut work: Vec<CellKey> = (callers.keys())
        .filter(|at| !known.contains(*at))
        .copied()
        .collect();
    while let Some(at) = work.pop() {
        for caller in callers.get(&at).into_iter().flatten() {
            if known.remove(caller) {
                work.push(*caller);
            }
        }
    }

    let mut summaries: BTreeMap<CellKey, Summary> = BTreeMap::new();
    let mut work: VecDeque<CellKey> = VecDeque::new();
    for at in &known {
        let uses = get(reach, *at).and_then(|held| held.uses.as_ref());
        match uses {
            Some(uses) => {
                summaries.insert(*at, Summary::of(uses));
            }
            None => {
                summaries.insert(*at, Summary::first());
                work.push_back(*at);
            }
        }
    }
    let mut queued: BTreeSet<CellKey> = work.iter().copied().collect();
    let mut budget = work.len().saturating_mul(PASSES);
    let mut lost: Vec<CellKey> = Vec::new();
    while let Some(at) = work.pop_front() {
        queued.remove(&at);
        if !summaries.contains_key(&at) {
            continue;
        }
        let Some(left) = budget.checked_sub(1) else {
            // Not settled in time: what is still to run, and every cell handed on to it, is told nothing.
            lost.push(at);
            lost.extend(work.drain(..));
            break;
        };
        budget = left;
        let flow = get(reach, at).and_then(|held| held.flow.as_ref());
        let Some(new) = flow.and_then(|flow| evaluate(flow, &summaries)) else {
            forget(at, &mut summaries, &callers);
            continue;
        };
        let Some(old) = summaries.get(&at) else {
            continue;
        };
        let merged = Summary {
            reads: old.reads.union(&new.reads).widened(),
            writes: old.writes.union(&new.writes).widened(),
            written: old.written.meet(&new.written).narrowed(),
        };
        if merged != *old {
            summaries.insert(at, merged);
            for caller in callers.get(&at).into_iter().flatten() {
                if summaries.contains_key(caller) && queued.insert(*caller) {
                    work.push_back(*caller);
                }
            }
        }
    }
    for at in lost {
        forget(at, &mut summaries, &callers);
    }

    for (key, cells) in reach.iter_mut() {
        for (cell, held) in cells.iter_mut() {
            let at = (*key, *cell);
            held.dies = !kept.contains(&at);
            let summary = summaries.get(&at);
            held.mode = summary.map(Summary::mode);
            held.covered = match (summary, held.mode) {
                (Some(summary), Some(Mode::Out)) => summary.written.prefix().min(held.bytes),
                _ => 0,
            };
        }
    }
}

impl Project {
    /// The bytes from the address in parameter cell `cell` of `function` the function may touch; `None` where the project holds no reach of it.
    pub fn reach(&self, function: FunctionKey, cell: u32) -> Option<u32> {
        self.reach
            .get(&function)?
            .get(&cell)
            .map(|reach| reach.bytes)
    }

    /// The cells from the address in parameter cell `cell` of `function` the function may touch; `None` where nothing bounds them or the project holds no reach of it.
    pub fn reach_cells(&self, function: FunctionKey, cell: u32) -> Option<u32> {
        let bytes = self.reach(function, cell)?;
        (bytes != UNBOUNDED).then(|| bytes.div_ceil(8))
    }

    /// How `function` uses the bytes the address in its parameter cell `cell` points at; `None` where that cannot be told.
    pub fn param_mode(&self, function: FunctionKey, cell: u32) -> Option<Mode> {
        self.reach.get(&function)?.get(&cell)?.mode
    }

    /// Where `function` uses the address in its parameter cell `cell` as [`Mode::Out`]: the bytes from the address on that every path to a return writes.
    pub fn param_covered(&self, function: FunctionKey, cell: u32) -> Option<u32> {
        let held = self.reach.get(&function)?.get(&cell)?;
        (held.mode == Some(Mode::Out)).then_some(held.covered)
    }

    /// Whether nothing keeps the address in parameter cell `cell` of `function` past the call; `None` where the project holds no reach of it.
    pub fn address_dies(&self, function: FunctionKey, cell: u32) -> Option<bool> {
        self.reach.get(&function)?.get(&cell).map(|held| held.dies)
    }
}

#[cfg(test)]
#[path = "reach_tests.rs"]
mod tests;
