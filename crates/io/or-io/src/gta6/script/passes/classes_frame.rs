//! What the addresses a function takes of its own locals may reach, read finer than `Reached`:
//! an address loaded or stored through where it is taken touches the bytes it names and no more,
//! and one handed to a script function touches what the project's reach table bounds, where the
//! address dies with the call. In a body that runs each statement once, in order, an address
//! taken after a construction reaches nothing while its constructor runs.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::ir::{self, Expr, Head, Var, Width};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::project::signatures::key_of;
use crate::gta6::script::project::{reach, Project};
use crate::gta6::script::tree::{self, Stmt, Sugar};

use super::{constructed, whole, Reached};

/// The most constructions of one body whose variable is looked for again over what runs before them alone.
// Each look reads the statements before the construction once more.
const MAX_EARLY: usize = 64;

/// The places in the outermost list of `body` where an object is made and constructed and an address taken later may reach its variable, but none taken before or by the constructor's call.
// ⚠ Only where each statement runs once and after those written before it: the outermost list
// is in no loop, and a body with no jump, no label and no point it resumes at runs nothing out of
// the order it is written in. An address taken later points nowhere while the constructor runs;
// one kept from an earlier run of the function points into a frame that has ended.
pub(super) fn early(
    body: &[Stmt],
    head: &Head,
    project: &Project,
    reached: &Reached,
    frame: &Frame,
) -> BTreeSet<usize> {
    let mut found = BTreeSet::new();
    if jumps(body) {
        return found;
    }
    let mut looks = 0usize;
    for (at, pair) in body.windows(2).enumerate() {
        let [Stmt::Simple(made), Stmt::Simple(call)] = pair else {
            continue;
        };
        let Some(target) = constructed(made, call, head, project) else {
            continue;
        };
        // What the whole body says is said already.
        if !reached.has(target) || frame.private(target, head) {
            continue;
        }
        if looks == MAX_EARLY {
            break;
        }
        looks += 1;
        let before = &body[..at + 2];
        let none = !Reached::of(before).has(target);
        if none || Frame::of(before, head, project).private(target, head) {
            found.insert(at);
        }
    }
    found
}

/// Whether a body holds a jump, a label or a point a state machine resumes at: what may run a statement again, or before one written ahead of it.
fn jumps(body: &[Stmt]) -> bool {
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            match stmt {
                Stmt::Goto(_) | Stmt::Label(_) => return true,
                Stmt::If { then, els, .. } => work.extend([then.as_slice(), els.as_slice()]),
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    work.push(body);
                }
                Stmt::Switch { cases, .. } => work.extend(cases.iter().map(|c| c.body.as_slice())),
                Stmt::Sugar(sugar) => {
                    if matches!(&**sugar, Sugar::Await { .. } | Sugar::Yield { .. }) {
                        return true;
                    }
                    work.extend(sugar.parts().bodies);
                }
                Stmt::Simple(_)
                | Stmt::Break
                | Stmt::Continue
                | Stmt::Return(_)
                | Stmt::Unreachable => {}
            }
        }
    }
    false
}

/// The bytes of a cell.
const CELL: u64 = 8;

/// What the addresses a body takes may reach of its locals.
// ⚠ An address of a local is taken to reach the locals after it and none before, as `Reached`
// takes it; an address of a parameter or a temporary handed on or kept, or touching past its own
// cell, may reach anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Frame {
    /// The bytes each address of a local used in place touches, counted from the first local's first byte.
    touched: Vec<(u64, u64)>,
    /// The lowest local whose address is handed on or kept.
    kept: Option<u32>,
    /// Whether some address may reach what is not said.
    blind: bool,
}

impl Frame {
    pub(super) fn of(body: &[Stmt], head: &Head, project: &Project) -> Frame {
        // Each address that is the place of a load or a store, or that a call hands a function
        // that touches bounded bytes through it and keeps nothing, with the bytes it touches from
        // the variable's first.
        let mut used: BTreeMap<*const Expr, (u64, u64)> = BTreeMap::new();
        tree::each_simple(body, &mut |stmt| {
            if let Some((addr, offset, bytes)) = place(stmt) {
                at(addr, offset, bytes, &mut used);
            }
        });
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::Load { ty, addr, offset } => at(addr, *offset, ty.bytes(), &mut used),
                Expr::LoadBytes {
                    addr,
                    offset,
                    bytes,
                } => at(addr, *offset, *bytes, &mut used),
                Expr::Call { .. } => handed(e, head, project, &mut used),
                _ => {}
            });
        });
        let mut frame = Frame::default();
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| {
                if let Expr::AddrOf { var, .. } = e {
                    let touched = used.get(&std::ptr::from_ref(e)).copied();
                    frame.note(*var, touched, head);
                }
            });
        });
        // Disjoint and in order, so that a local is looked up and not compared with each.
        frame.touched.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(frame.touched.len());
        for (from, to) in frame.touched.drain(..) {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        frame.touched = merged;
        frame
    }

    /// Notes one address of `var`: used in place over the bytes `touched` names from the variable's first, or handed on.
    fn note(&mut self, var: Var, touched: Option<(u64, u64)>, head: &Head) {
        match (var, touched) {
            (Var::Global { .. }, _) => {}
            (Var::Local(n), Some((from, to))) => {
                let first = u64::from(n) * CELL;
                self.touched.push((first + from, first + to));
            }
            (Var::Local(n), None) => self.kept = Some(self.kept.map_or(n, |low| low.min(n))),
            (other, Some((_, to))) if to <= own(other, head) => {}
            _ => self.blind = true,
        }
    }

    /// Whether no address the body takes may reach the local `var` of one cell: none of it or of a local before it is handed on or kept, and none used in place touches its bytes.
    pub(super) fn private(&self, var: Var, head: &Head) -> bool {
        let Var::Local(n) = var else {
            return false;
        };
        let wide = head
            .vars
            .get(&var)
            .is_some_and(|info| info.width == Width::Cells);
        let from = u64::from(n) * CELL;
        let to = from + CELL;
        // The first range that ends past the local's first byte is the only one that may touch it.
        let next = self.touched.partition_point(|&(_, end)| end <= from);
        let touches = self.touched.get(next).is_some_and(|&(start, _)| start < to);
        !self.blind && !wide && !touches && self.kept.is_none_or(|low| low > n)
    }
}

/// The address a statement stores through, with the offset and the bytes it writes.
// ⚠ A store of floats fills four bytes of each cell of its value, a count the statement does not state.
fn place(stmt: &ir::Stmt) -> Option<(&Expr, u32, u32)> {
    match stmt {
        ir::Stmt::Store {
            kind, addr, offset, ..
        } => Some((addr, *offset, kind.bytes())),
        ir::Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            ..
        } => Some((addr, *offset, *bytes)),
        ir::Stmt::ZeroBytes { addr, bytes } => Some((addr, 0, *bytes)),
        ir::Stmt::StepI32 { addr, offset, .. } => Some((addr, *offset, 4)),
        _ => None,
    }
}

/// Notes `addr` as used in place over `bytes` bytes at `offset`, where it is the address of a variable itself.
fn at(addr: &Expr, offset: u32, bytes: u32, used: &mut BTreeMap<*const Expr, (u64, u64)>) {
    let addr = whole(addr);
    if let Expr::AddrOf { cell, .. } = addr {
        let from = u64::from(*cell) * CELL + u64::from(offset);
        used.insert(std::ptr::from_ref(addr), (from, from + u64::from(bytes)));
    }
}

/// Notes each address of a variable that `call` hands a script function, where the project bounds what the function touches through that parameter cell and says the address dies with the call, as used over those bytes.
// ⚠ The cells handed must be the cells the function takes, or an argument lands elsewhere. A
// reach is of the one cell alone: another address handed, or one kept before, is noted apart.
fn handed(
    call: &Expr,
    head: &Head,
    project: &Project,
    used: &mut BTreeMap<*const Expr, (u64, u64)>,
) {
    let Expr::Call { callee, args, .. } = call else {
        return;
    };
    let Some(key) = key_of(head.program, callee) else {
        return;
    };
    let signature = project.signatures.get(&key);
    let takes = signature.map(|held| held.params.iter().map(|param| param.cells).sum::<u32>());
    // The parameter cell each argument lands in.
    let mut next = Some(0u32);
    let mut lands = Vec::with_capacity(args.len());
    for arg in args {
        lands.push(next);
        next = next
            .zip(cells_of(arg, head))
            .and_then(|(from, cells)| from.checked_add(cells));
    }
    if takes.is_none() || takes != next {
        return;
    }
    for (arg, landed) in args.iter().zip(lands) {
        let (Expr::AddrOf { cell: 0, .. }, Some(param)) = (arg, landed) else {
            continue;
        };
        let bytes = project
            .reach(key, param)
            .filter(|bytes| *bytes != reach::UNBOUNDED);
        let dies = project.address_dies(key, param) == Some(true);
        if let (Some(bytes), true) = (bytes, dies) {
            used.insert(std::ptr::from_ref(arg), (0, u64::from(bytes)));
        }
    }
}

/// The bytes of a parameter or a temporary an address of it may touch in place: its one cell, or none where it holds several or is not known.
fn own(var: Var, head: &Head) -> u64 {
    match head.vars.get(&var) {
        Some(info) if info.width == Width::Cell => CELL,
        _ => 0,
    }
}

#[cfg(test)]
#[path = "classes_frame_tests.rs"]
mod tests;
