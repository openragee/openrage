//! The `gotos` rules over the finished statements: a `goto` that says nothing goes, and a `goto`
//! to a short tail that leaves is that tail.
//!
//! ⚠ The second writes statements twice, which nothing else in the pass does: a tail of at most
//! [`TAIL`] simple statements and the statement that leaves, copied to each `goto` that names it.
//! Each path still runs one copy and then leaves as the tail did, so what runs is what ran.

use std::collections::HashMap;

use crate::gta6::script::tree::{Label, Stmt};

use super::leaves;

/// The simple statements a tail may hold in front of its leaving statement to be copied.
const TAIL: usize = 2;

/// A tail to copy, and the constructs its last statement leaves where the tail stands.
struct Tail {
    stmts: Vec<Stmt>,
    /// The loop or switch a `break` leaves there and the loop a `continue` goes round, by [`each_list`]'s numbers.
    brk: u32,
    cont: u32,
}

impl Tail {
    /// Whether the tail's last statement, in a list with these numbers, leaves to where it leaves from its own.
    fn leaves_alike(&self, brk: u32, cont: u32) -> bool {
        match self.stmts.last() {
            Some(Stmt::Break) => self.brk == brk,
            Some(Stmt::Continue) => self.cont == cont,
            _ => true,
        }
    }
}

/// Calls `f` on every list of `body` with the number of the loop or switch a `break` in it leaves and of the loop a `continue` in it goes round; 0 is none.
// ⚠ A walk gives a construct the number the walk before gave it only while no loop and no switch
// was added, dropped or moved in between.
fn each_list(body: &mut Vec<Stmt>, mut f: impl FnMut(&mut Vec<Stmt>, u32, u32)) {
    let mut next = 1u32;
    let mut work: Vec<(&mut Vec<Stmt>, u32, u32)> = vec![(body, 0, 0)];
    while let Some((list, brk, cont)) = work.pop() {
        f(&mut *list, brk, cont);
        for stmt in list {
            match stmt {
                Stmt::If { then, els, .. } => {
                    work.push((then, brk, cont));
                    work.push((els, brk, cont));
                }
                Stmt::Switch { cases, .. } => {
                    let id = next;
                    next += 1;
                    work.extend(cases.iter_mut().map(|case| (&mut case.body, id, cont)));
                }
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    let id = next;
                    next += 1;
                    work.push((body, id, id));
                }
                _ => {}
            }
        }
    }
}

/// Drops the entries of `list` that `gone` marks.
fn without(list: &mut Vec<Stmt>, gone: &[bool]) {
    let mut at = 0;
    list.retain(|_| {
        at += 1;
        !gone[at - 1]
    });
}

/// Drops each `goto` to the label that stands right after it, with only labels between; whether one went.
pub(super) fn next_gotos(body: &mut Vec<Stmt>, named: &mut HashMap<Label, u32>) -> bool {
    let mut changed = false;
    each_list(body, |list, _, _| {
        if !list.iter().any(|stmt| matches!(stmt, Stmt::Goto(_))) {
            return;
        }
        // From the end: the labels that stand where control is once `list[i]` has run on.
        let mut here: Vec<Label> = Vec::new();
        let mut gone = vec![false; list.len()];
        for (i, stmt) in list.iter().enumerate().rev() {
            match stmt {
                Stmt::Label(label) => here.push(*label),
                Stmt::Goto(label) if here.contains(label) => {
                    gone[i] = true;
                    if let Some(count) = named.get_mut(label) {
                        *count = count.saturating_sub(1);
                    }
                }
                _ => here.clear(),
            }
        }
        if gone.contains(&true) {
            without(list, &gone);
            changed = true;
        }
    });
    changed
}

/// Where the tail under the label `list[i]` starts and ends: past the labels there, simple statements and the statement that leaves.
fn tail_at(list: &[Stmt], i: usize) -> Option<(usize, usize)> {
    let labels = list[i..]
        .iter()
        .take_while(|stmt| matches!(stmt, Stmt::Label(_)))
        .count();
    let start = i + labels;
    let simples = list[start..]
        .iter()
        .take_while(|stmt| matches!(stmt, Stmt::Simple(_)))
        .take(TAIL + 1)
        .count();
    let end = start + simples;
    let leaving = matches!(
        list.get(end),
        Some(Stmt::Return(_) | Stmt::Unreachable | Stmt::Break | Stmt::Continue)
    );
    (simples <= TAIL && leaving).then_some((start, end + 1))
}

/// The tail that `stmt`, standing in a list with these numbers, is written as: it is a `goto` to one that leaves alike from there.
fn copy_for<'a>(
    tails: &'a HashMap<Label, Tail>,
    stmt: &Stmt,
    brk: u32,
    cont: u32,
) -> Option<(Label, &'a Tail)> {
    match stmt {
        Stmt::Goto(label) => tails
            .get(label)
            .filter(|tail| tail.leaves_alike(brk, cont))
            .map(|tail| (*label, tail)),
        _ => None,
    }
}

/// Writes each `goto` to a short tail that leaves as that tail, and drops the tail where nothing runs into it any more; whether one was written.
// A `return` leaves alike from anywhere; a `break` or a `continue` only from inside the construct
// it leaves where the tail stands.
pub(super) fn copied(body: &mut Vec<Stmt>, named: &mut HashMap<Label, u32>) -> bool {
    let mut tails: HashMap<Label, Tail> = HashMap::new();
    each_list(body, |list, brk, cont| {
        let list: &[Stmt] = &*list;
        for (i, stmt) in list.iter().enumerate() {
            if let Stmt::Label(label) = stmt {
                if let Some((start, end)) = tail_at(list, i) {
                    let stmts = list[start..end].to_vec();
                    tails.insert(*label, Tail { stmts, brk, cont });
                }
            }
        }
    });
    if tails.is_empty() {
        return false;
    }

    let mut changed = false;
    each_list(body, |list, brk, cont| {
        let copies = |stmt: &Stmt| copy_for(&tails, stmt, brk, cont).is_some();
        if !list.iter().any(copies) {
            return;
        }
        let mut out: Vec<Stmt> = Vec::with_capacity(list.len() + TAIL);
        for stmt in list.drain(..) {
            match copy_for(&tails, &stmt, brk, cont) {
                Some((label, tail)) => {
                    out.extend(tail.stmts.iter().cloned());
                    if let Some(count) = named.get_mut(&label) {
                        *count = count.saturating_sub(1);
                    }
                }
                None => out.push(stmt),
            }
        }
        *list = out;
        changed = true;
    });
    if !changed {
        return false;
    }

    // A tail no `goto` names any more, behind a statement that leaves, is reached by nothing:
    // every jump to it took its copy.
    each_list(body, |list, _, _| {
        let mut gone: Vec<bool> = Vec::new();
        let stmts: &[Stmt] = &*list;
        let mut i = 1;
        while i < stmts.len() {
            let dead = match &stmts[i] {
                Stmt::Label(label) if tails.contains_key(label) && leaves(&stmts[i - 1]) => {
                    tail_at(stmts, i).filter(|(start, _)| {
                        stmts[i..*start].iter().all(|stmt| match stmt {
                            Stmt::Label(label) => named.get(label).copied().unwrap_or(0) == 0,
                            _ => false,
                        })
                    })
                }
                _ => None,
            };
            match dead {
                Some((_, end)) => {
                    gone.resize(stmts.len(), false);
                    gone[i..end].fill(true);
                    i = end;
                }
                None => i += 1,
            }
        }
        if !gone.is_empty() {
            without(list, &gone);
        }
    });
    true
}
