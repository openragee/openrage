//! The `gotos` rules that read a `goto` of the finished statements as the way out it is: the
//! end of an arm, a `break`, a `continue`, the test in front of the code it passes over, a
//! guard that returns, the one exit of a loop.
//!
//! Each rule holds only where the label a `goto` names stands at the place control reaches by
//! what is written instead, so what runs is what ran. ⚠ One rule writes a statement twice: a
//! `return` of a small value with no call, a `break` or a `continue`, in front of the arm it
//! followed. No simple statement is copied and none is dropped.

use std::collections::HashMap;

use crate::gta6::script::ir::{Const, Expr, Kind};
use crate::gta6::script::tree::{Label, Stmt};

use super::cond;
use super::graph::SCAN;
use super::leaves;
use super::region::MAX_NEST;

/// The labels that stand where control goes from a list of statements.
#[derive(Default)]
struct Ways {
    /// Where control is once the list has run off its end.
    after: Vec<Label>,
    /// Where a `break` lands; `None` outside a loop and a switch.
    brk: Option<Vec<Label>>,
    /// Where a `continue` lands; `None` outside a loop.
    cont: Option<Vec<Label>>,
}

/// The labels that stand at `list[at]`: the run of labels there and, where it reaches the end of the list, `after`.
fn labels_at(list: &[Stmt], at: usize, after: &[Label]) -> Vec<Label> {
    let mut found = Vec::new();
    let mut at = at;
    while let Some(Stmt::Label(label)) = list.get(at) {
        found.push(*label);
        at += 1;
    }
    if at >= list.len() {
        found.extend_from_slice(after);
    }
    found
}

/// The labels that stand right before `list[at]`.
fn labels_before(list: &[Stmt], at: usize) -> Vec<Label> {
    let mut found = Vec::new();
    for stmt in list[..at].iter().rev() {
        match stmt {
            Stmt::Label(label) => found.push(*label),
            _ => break,
        }
    }
    found
}

/// Where the first label at or after `from` stands in `list`, or the end of `list`; `None` past [`SCAN`] statements.
fn first_label(list: &[Stmt], from: usize) -> Option<usize> {
    let far = list.len().min(from.saturating_add(SCAN));
    (from..far)
        .find(|at| matches!(list[*at], Stmt::Label(_)))
        .or((far == list.len()).then_some(far))
}

fn forever(cond: &Expr) -> bool {
    matches!(cond, Expr::Const(c) if c.kind == Kind::Bool && c.bits != 0)
}

/// Whether a statement that leaves may be written a second time: it holds no effect to run twice.
fn copies(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break | Stmt::Continue => true,
        Stmt::Return(value) => cond::cheap(value.as_ref()),
        _ => false,
    }
}

/// Whether control may run off the end of `list`: `false` only where its last statement leaves, or is an `if` neither arm of which runs off.
fn runs_off(list: &[Stmt]) -> bool {
    let mut work = vec![list];
    let mut seen = 0usize;
    while let Some(list) = work.pop() {
        seen += 1;
        match list.last() {
            Some(stmt) if leaves(stmt) => {}
            Some(Stmt::If { then, els, .. }) if seen < MAX_NEST => {
                work.push(then.as_slice());
                work.push(els.as_slice());
            }
            _ => return true,
        }
    }
    false
}

/// How deep the lists under `list` nest: 0 where it holds no `if`, loop or `switch`.
fn deepest(list: &[Stmt]) -> usize {
    let mut most = 0;
    let mut work: Vec<(&[Stmt], usize)> = vec![(list, 0)];
    while let Some((list, at)) = work.pop() {
        most = most.max(at);
        for stmt in list {
            match stmt {
                Stmt::If { then, els, .. } => {
                    work.push((then.as_slice(), at + 1));
                    work.push((els.as_slice(), at + 1));
                }
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    work.push((body.as_slice(), at + 1));
                }
                Stmt::Switch { cases, .. } => {
                    work.extend(cases.iter().map(|case| (case.body.as_slice(), at + 1)));
                }
                _ => {}
            }
        }
    }
    most
}

/// What of a list would leave a loop the list stood in.
#[derive(Default)]
struct Free {
    /// Each `break` no `switch` and no loop of the list takes.
    breaks: u32,
    /// Each `continue` no loop of the list takes.
    continues: u32,
    /// Each `goto` to one of the labels asked about, where a `break` would leave that loop.
    jumps: u32,
    /// A construct this cannot see into stands in the way.
    opaque: bool,
}

fn free(list: &[Stmt], labels: &[Label]) -> Free {
    let mut found = Free::default();
    // A list, and whether a `switch` around it takes its `break`s.
    let mut work: Vec<(&[Stmt], bool)> = vec![(list, false)];
    while let Some((list, switched)) = work.pop() {
        for stmt in list {
            match stmt {
                Stmt::Break if !switched => found.breaks += 1,
                Stmt::Continue => found.continues += 1,
                Stmt::Goto(label) if !switched && labels.contains(label) => found.jumps += 1,
                Stmt::If { then, els, .. } => {
                    work.push((then.as_slice(), switched));
                    work.push((els.as_slice(), switched));
                }
                Stmt::Switch { cases, .. } => {
                    work.extend(cases.iter().map(|case| (case.body.as_slice(), true)));
                }
                Stmt::Sugar(_) => found.opaque = true,
                _ => {}
            }
        }
    }
    found
}

/// Whether `list`, written in front of the labels `end`, holds a `goto` to one of them that then goes.
// The jump ends the list, or ends the arm of an `if` of it that no label follows, or the list
// ends in an `if` with no else whose arm is such a list in turn.
fn leaves_to(list: &[Stmt], end: &[Label]) -> bool {
    let jumps =
        |stmt: Option<&Stmt>| matches!(stmt, Some(Stmt::Goto(label)) if end.contains(label));
    let mut list = list;
    for _ in 0..MAX_NEST {
        if jumps(list.last()) {
            return true;
        }
        let from = list
            .iter()
            .rposition(|stmt| matches!(stmt, Stmt::Label(_)))
            .map_or(0, |at| at + 1);
        let nests = list[from..].iter().any(|stmt| match stmt {
            Stmt::If { then, els, .. } => jumps(then.last()) && !els.last().is_some_and(leaves),
            _ => false,
        });
        if nests {
            return true;
        }
        match list.last() {
            Some(Stmt::If { then, els, .. }) if els.is_empty() => list = then.as_slice(),
            _ => return false,
        }
    }
    false
}

/// Writes `tail` in front of the one `break` of `body` that leaves its loop, found through `if`s; gives `tail` back where there is none.
fn place(body: &mut Vec<Stmt>, tail: Vec<Stmt>) -> Option<Vec<Stmt>> {
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        if let Some(at) = list.iter().position(|stmt| matches!(stmt, Stmt::Break)) {
            // `rest` starts with the `break`, which nothing reaches behind a tail that leaves.
            let rest = list.split_off(at);
            let unreached = usize::from(!runs_off(&tail));
            list.extend(tail);
            list.extend(rest.into_iter().skip(unreached));
            return None;
        }
        for stmt in list {
            if let Stmt::If { then, els, .. } = stmt {
                work.push(then);
                work.push(els);
            }
        }
    }
    Some(tail)
}

struct Exits<'a> {
    /// The `goto`s that name each label.
    named: &'a mut HashMap<Label, u32>,
    changed: bool,
}

impl Exits<'_> {
    /// Records that a `goto` to `label` went.
    fn gone(&mut self, label: Label) {
        if let Some(count) = self.named.get_mut(&label) {
            *count = count.saturating_sub(1);
        }
        self.changed = true;
    }

    /// The rules over `list`, which stands `depth` constructs deep, and then over the lists nested in it.
    fn list(&mut self, list: &mut Vec<Stmt>, ways: &Ways, depth: usize) {
        if depth > MAX_NEST {
            return;
        }
        let mut i = 0;
        while i < list.len() {
            if self.jump(list, i, ways) {
                continue;
            }
            self.nest(list, i, ways, depth);
            if self.guard(list, i, ways) {
                // The guard stands at `i`; what it guarded follows and is looked at next.
                i += 1;
                continue;
            }
            self.tail(list, i, ways, depth);
            self.tested(&mut list[i]);
            i += 1;
        }
        for i in 0..list.len() {
            // A `while` goes round to its test, which is where a label right before it leads.
            let before = match &list[i] {
                Stmt::While { .. } => labels_before(list, i),
                _ => Vec::new(),
            };
            let next = labels_at(list, i + 1, &ways.after);
            match &mut list[i] {
                Stmt::If { then, els, .. } => {
                    let inner = Ways {
                        after: next,
                        brk: ways.brk.clone(),
                        cont: ways.cont.clone(),
                    };
                    self.list(then, &inner, depth + 1);
                    self.list(els, &inner, depth + 1);
                }
                Stmt::While { body, .. } => {
                    let mut cont = before.clone();
                    cont.extend(labels_before(body.as_slice(), body.len()));
                    let inner = Ways {
                        after: before,
                        brk: Some(next),
                        cont: Some(cont),
                    };
                    self.list(body, &inner, depth + 1);
                }
                // Off the end of the body is the test of a `do` and the step of a `for`: no
                // label stands there but one the body ends in.
                Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    let inner = Ways {
                        after: Vec::new(),
                        brk: Some(next),
                        cont: Some(labels_before(body.as_slice(), body.len())),
                    };
                    self.list(body, &inner, depth + 1);
                }
                // ⚠ A body that runs off its end runs into the next one: a jump out of it is a
                // `break` and never nothing.
                Stmt::Switch { cases, .. } => {
                    let inner = Ways {
                        after: Vec::new(),
                        brk: Some(next),
                        cont: ways.cont.clone(),
                    };
                    for case in cases.iter_mut() {
                        self.list(&mut case.body, &inner, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }

    /// A `goto` to where control goes anyway says nothing, and one to where a `break` or a `continue` lands is that; `true` when `list[i]` went.
    fn jump(&mut self, list: &mut Vec<Stmt>, i: usize, ways: &Ways) -> bool {
        let label = match &list[i] {
            Stmt::Goto(label) => *label,
            _ => return false,
        };
        if labels_at(list, i + 1, &ways.after).contains(&label) {
            list.remove(i);
            self.gone(label);
            return true;
        }
        let lands = |at: &Option<Vec<Label>>| at.as_ref().is_some_and(|at| at.contains(&label));
        if lands(&ways.brk) {
            list[i] = Stmt::Break;
            self.gone(label);
        } else if lands(&ways.cont) {
            list[i] = Stmt::Continue;
            self.gone(label);
        }
        false
    }

    /// `if (c) { ..; goto L; } rest; L:` is `if (c) { .. } else { rest }`: the jump passed over `rest` to where the `if` now runs on to.
    // `rest` ends at the first label, so no jump from elsewhere lands inside the arm it becomes.
    fn nest(&mut self, list: &mut Vec<Stmt>, i: usize, ways: &Ways, depth: usize) {
        let label = match &list[i] {
            Stmt::If { then, els, .. } => match then.last() {
                // An else that leaves never ran on into `rest`.
                Some(Stmt::Goto(label)) if !els.last().is_some_and(leaves) => *label,
                _ => return,
            },
            _ => return,
        };
        let Some(end) = first_label(list, i + 1) else {
            return;
        };
        if end == i + 1 || !labels_at(list, end, &ways.after).contains(&label) {
            return;
        }
        if depth + 2 + deepest(&list[i + 1..end]) > MAX_NEST {
            return;
        }
        let rest: Vec<Stmt> = list.drain(i + 1..end).collect();
        if let Stmt::If { cond, then, els } = &mut list[i] {
            then.pop();
            els.extend(rest);
            if then.is_empty() {
                let test = std::mem::replace(cond, Expr::GuidEmpty);
                *cond = cond::negate(test);
                std::mem::swap(then, els);
            }
        }
        self.gone(label);
    }

    /// `if (c) { arm } return x; L:` is `if (!c) return x; arm; return x; L:` where a `goto L` of the arm then goes; `true` when it was written so.
    // The arm's way on to the `return` is by running off its end, so the `return` after it
    // stays; behind an arm that never runs off, nothing reaches it and it goes.
    fn guard(&mut self, list: &mut Vec<Stmt>, i: usize, ways: &Ways) -> bool {
        let fits = match (&list[i], list.get(i + 1)) {
            (Stmt::If { then, els, .. }, Some(way)) if els.is_empty() && copies(way) => {
                let stands = matches!(list.get(i + 2), None | Some(Stmt::Label(_)));
                stands && leaves_to(then, &labels_at(list, i + 2, &ways.after))
            }
            _ => false,
        };
        if !fits {
            return false;
        }
        let (cond, then) = match std::mem::replace(&mut list[i], Stmt::Unreachable) {
            Stmt::If { cond, then, .. } => (cond, then),
            other => {
                list[i] = other;
                return false;
            }
        };
        let way = list[i + 1].clone();
        if !runs_off(&then) {
            list.remove(i + 1);
        }
        let rest = list.split_off(i + 1);
        list[i] = Stmt::If {
            cond: cond::negate(cond),
            then: vec![way],
            els: Vec::new(),
        };
        list.extend(then);
        list.extend(rest);
        self.changed = true;
        true
    }

    /// A loop left by one `break`, or by its test at the bottom, takes in the statements after it, where its `goto`s past them are then its `break`s.
    // `while (true) { if (a) goto L; if (b) break; .. } tail; L:` is
    // `while (true) { if (a) break; if (b) { tail; break; } .. }`, and
    // `do { .. } while (c); tail; L:` is `while (true) { ..; if (!c) { tail; break; } }`.
    // ⚠ Not a `while (c)`: its test leaves to the tail too. Not a tail that holds a `break` or a
    // `continue`: inside the loop it would leave this loop. Not a `do` that holds a `continue`,
    // which goes to the test, nor one that holds a `break`, which lands before the tail.
    fn tail(&mut self, list: &mut Vec<Stmt>, i: usize, ways: &Ways, depth: usize) {
        let made = match &list[i] {
            Stmt::While { cond, .. } if forever(cond) => true,
            Stmt::DoWhile { .. } => false,
            _ => return,
        };
        let Some(end) = first_label(list, i + 1) else {
            return;
        };
        if end == i + 1 {
            return;
        }
        let labels = labels_at(list, end, &ways.after);
        if labels.is_empty() {
            return;
        }
        let (Stmt::While { body, .. } | Stmt::DoWhile { body, .. }) = &list[i] else {
            return;
        };
        let inside = free(body, &labels);
        let outside = free(&list[i + 1..end], &labels);
        let one_way = match made {
            true => inside.breaks == 1,
            false => inside.breaks == 0 && inside.continues == 0,
        };
        let moves = outside.breaks == 0 && outside.continues == 0 && !outside.opaque;
        if !one_way || !moves || inside.opaque || inside.jumps == 0 {
            return;
        }
        if depth + 3 + deepest(body) + deepest(&list[i + 1..end]) > MAX_NEST {
            return;
        }
        let taken = std::mem::replace(&mut list[i], Stmt::Unreachable);
        let tail: Vec<Stmt> = list.drain(i + 1..end).collect();
        let (lp, left) = match taken {
            Stmt::While { cond, mut body } => {
                // The tail goes in first: the one `break` it follows is told from those made next.
                let left = place(&mut body, tail);
                if left.is_none() {
                    self.breaks_for(&mut body, &labels);
                }
                (Stmt::While { cond, body }, left)
            }
            Stmt::DoWhile { mut body, cond } => {
                let mut out = tail;
                if runs_off(&out) {
                    out.push(Stmt::Break);
                }
                body.push(Stmt::If {
                    cond: cond::negate(cond),
                    then: out,
                    els: Vec::new(),
                });
                self.breaks_for(&mut body, &labels);
                let cond = Expr::Const(Const::bool(true));
                (Stmt::While { cond, body }, None)
            }
            other => (other, Some(tail)),
        };
        list[i] = lp;
        match left {
            Some(back) => {
                let rest = list.split_off(i + 1);
                list.extend(back);
                list.extend(rest);
            }
            None => self.changed = true,
        }
    }

    /// Writes each `goto` of a loop's `body` to one of `labels`, where a `break` leaves that loop, as the `break`.
    fn breaks_for(&mut self, body: &mut Vec<Stmt>, labels: &[Label]) {
        let mut work: Vec<&mut Vec<Stmt>> = vec![body];
        while let Some(list) = work.pop() {
            for stmt in list {
                let jump = match &*stmt {
                    Stmt::Goto(label) if labels.contains(label) => Some(*label),
                    _ => None,
                };
                if let Some(label) = jump {
                    *stmt = Stmt::Break;
                    self.gone(label);
                } else if let Stmt::If { then, els, .. } = stmt {
                    work.push(then);
                    work.push(els);
                }
            }
        }
    }

    /// `while (true) { if (c) break; .. }` is `while (!c) { .. }`.
    fn tested(&mut self, stmt: &mut Stmt) {
        let Stmt::While { cond, body } = stmt else {
            return;
        };
        let leads = matches!(
            body.first(),
            Some(Stmt::If { then, els, .. })
                if els.is_empty() && matches!(then.as_slice(), [Stmt::Break])
        );
        if !forever(cond) || !leads {
            return;
        }
        if let Stmt::If { cond: test, .. } = body.remove(0) {
            *cond = cond::negate(test);
            self.changed = true;
        }
    }
}

/// Writes each `goto` a rule explains as the way out it is, once over `body`; whether anything was rewritten.
// ⚠ `named` must count every `goto` of `body`: a label is dropped once nothing names it.
pub(super) fn rewritten(body: &mut Vec<Stmt>, named: &mut HashMap<Label, u32>) -> bool {
    let mut exits = Exits {
        named,
        changed: false,
    };
    exits.list(body, &Ways::default(), 0);
    exits.changed
}

#[cfg(test)]
#[path = "structure_exits_tests.rs"]
mod tests;
