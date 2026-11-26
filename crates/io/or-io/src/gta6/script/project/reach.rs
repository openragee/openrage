//! Reach: how many bytes from an address handed to a function the function may touch, for each
//! parameter cell, over every program of the project.
//!
//! A function's own code says what its loads and stores through the cell reach, and to which
//! parameter cells of which functions it hands the cell on, moved by how many bytes; `settle`
//! carries the reach of those along the calls until nothing changes. A cell the function does
//! anything else with (keeps, compares, works on, hands to a native, a builtin or a slot) reaches
//! without bound, and so does one handed to a function the project holds no reach of.
//!
//! ⚠ What a function reaches is a fact of its code alone: nothing here is taken from its callers.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::signatures::key_of;
use super::Project;
use crate::gta6::script::cfg;
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

/// A cell handed on: to parameter cell `cell` of `to`, moved `by` bytes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Handed {
    pub to: FunctionKey,
    pub cell: u32,
    pub by: u32,
}

/// What one parameter cell of a function reaches as an address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reach {
    /// The bytes from the address the function may touch through the cell, [`UNBOUNDED`] where nothing bounds them.
    pub bytes: u32,
    /// Where the function's own code hands the cell on, until [`settle`] has carried what those reach into `bytes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handed: Vec<Handed>,
}

/// What one function shows of one cell while its code is read.
#[derive(Default)]
struct Shown {
    bytes: u32,
    handed: BTreeSet<Handed>,
    /// The places the cell, or a copy of it, stands that the reading accounts for.
    accounted: u64,
}

/// One reading of a function: the cells of its parameters, by the variables that hold them, and what each does.
struct Reading<'a> {
    head: &'a Head,
    /// The variables that hold a parameter cell's address, moved by a count of bytes.
    holds: HashMap<Var, (u32, u32)>,
    shown: BTreeMap<u32, Shown>,
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
    fn access(&mut self, addr: &Expr, offset: u32, bytes: u32) {
        if let Some((cell, by)) = self.held(addr) {
            let end = by.checked_add(offset).and_then(|at| at.checked_add(bytes));
            let shown = self.shown.entry(cell).or_default();
            shown.bytes = shown.bytes.max(end.unwrap_or(UNBOUNDED));
            shown.accounted += 1;
        }
    }

    fn expr(&mut self, top: &Expr) {
        top.walk(&mut |e| match e {
            Expr::Load { ty, addr, offset } => self.access(addr, *offset, ty.bytes()),
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.access(addr, *offset, *bytes),
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
                // The cell an argument lands in: the cells of the arguments before it.
                let mut at = Some(0u32);
                for arg in args {
                    if let (Some(cell), Some((held, by))) = (at, self.held(arg)) {
                        let shown = self.shown.entry(held).or_default();
                        shown.handed.insert(Handed { to, cell, by });
                        shown.accounted += 1;
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
            } => self.access(addr, *offset, kind.bytes()),
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                ..
            } => self.access(addr, *offset, *bytes),
            Stmt::ZeroBytes { addr, bytes } => self.access(addr, 0, *bytes),
            Stmt::StepI32 { addr, offset, .. } => self.access(addr, *offset, 4),
            Stmt::Assign { target, value } if self.holds.contains_key(target) => {
                // The copy that made a holder: the cell stands there to be held, no more.
                if let Some((cell, _)) = self.held(value) {
                    self.shown.entry(cell).or_default().accounted += 1;
                }
            }
            _ => {}
        }
        stmt.each_expr(&mut |top| self.expr(top));
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
    }
    let mut holds: HashMap<Var, (u32, u32)> = HashMap::new();
    for cell in 0..params.min(MAX_PARAMS) {
        let var = Var::Param(cell);
        // A parameter the function writes holds the caller's address no longer.
        if !sets.contains_key(&var) && !addressed.contains(&var) {
            holds.insert(var, (cell, 0));
        }
    }
    for _ in 0..COPIES {
        let reading = Reading {
            head: &f.head,
            holds: holds.clone(),
            shown: BTreeMap::new(),
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
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            reading.stmt(stmt);
            stmt.each_expr(&mut |top| count(top, &reading.holds));
        }
        if let Some(e) = block.term.expr() {
            reading.expr(e);
            count(e, &reading.holds);
        }
    }
    let mut cells = BTreeMap::new();
    for cell in 0..params {
        let reach = match reading.holds.get(&Var::Param(cell)) {
            // Written or addressed, the parameter may be anything at its uses.
            None => Reach {
                bytes: UNBOUNDED,
                handed: Vec::new(),
            },
            Some(_) => {
                let shown = reading.shown.remove(&cell).unwrap_or_default();
                let all = stands.get(&cell).copied().unwrap_or(0) <= shown.accounted;
                Reach {
                    bytes: if all { shown.bytes } else { UNBOUNDED },
                    handed: shown.handed.into_iter().collect(),
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
            held.handed.clear();
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
}

#[cfg(test)]
#[path = "reach_tests.rs"]
mod tests;
