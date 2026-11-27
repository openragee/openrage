//! An await read where its task is made: `t = f(a); await t;` is `await f(a);` where nothing in
//! sight reads `t` besides, and how an await reads at the source level.
//!
//! The runtime awaits a task itself: the awaiter the test asks is the variable the task was put
//! in, so the statement that puts it there is the one that takes the awaiter, the role `before`
//! holds. Taken into the await it runs where it ran, first of what the await lowers to.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::ir::{BinOp, Callee, Expr, Stmt, Var};
use crate::gta6::script::tree::{self, Sugar};

use super::{constant, framed, unwide};

/// How an await reads at the source level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading<'a> {
    /// The statements of `before` that stay in sight before the await.
    pub shown: &'a [Stmt],
    /// What is awaited: the value the last statement of `before` gives the awaiter, else the awaiter the test asks.
    pub awaited: &'a Expr,
    /// What becomes of the await's result.
    pub taken: Taken<'a>,
}

/// What becomes of an await's result, as the one statement of `after` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken<'a> {
    /// No statement asks for it: `after` is written as it is, after the await.
    Nothing,
    /// `x = await e;`.
    Assigned(Var),
    /// `await e;`: the result is asked for and left.
    Dropped,
    /// `place = await e;`: the store given the result, written with the await as its value.
    Stored(&'a Stmt),
}

/// How the await `sugar` is read; `None` where it is no await.
// ⚠ Where `before` ends in the statement that gives the awaiter its value, what is awaited is that
// whole value: the awaiter may be the task the call makes, and only that call says what runs.
pub fn reading(sugar: &Sugar) -> Option<Reading<'_>> {
    let Sugar::Await {
        before,
        ready,
        after,
        ..
    } = sugar
    else {
        return None;
    };
    let asked = awaiter(ready);
    let (shown, awaited) = match (before.split_last(), asked) {
        (Some((Stmt::Assign { target, value }, rest)), Some(asked))
            if *unwide(asked) == Expr::Var(*target) =>
        {
            (rest, value)
        }
        _ => (before.as_slice(), asked.unwrap_or(ready)),
    };
    let taken = match (after.as_slice(), asked) {
        ([one], Some(asked)) => taken(one, asked),
        _ => Taken::Nothing,
    };
    Some(Reading {
        shown,
        awaited,
        taken,
    })
}

/// The awaiter an await's test asks whether it is ready: the first argument of the call the test makes, under any reading or test of it.
fn awaiter(ready: &Expr) -> Option<&Expr> {
    let mut e = ready;
    loop {
        match e {
            Expr::Cast { e: inner, .. }
            | Expr::Sugar { e: inner, .. }
            | Expr::Truth(inner)
            | Expr::Unary { e: inner, .. } => e = &**inner,
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                l,
                r,
                ..
            } if constant(r).is_some_and(|c| c.bits == 0) => e = &**l,
            Expr::Call { args, .. } => return args.first(),
            _ => return None,
        }
    }
}

/// What a statement makes of the result of an await of `asked`: it asks for it where it calls a function given the awaiter alone, and the call is its whole value.
fn taken<'a>(stmt: &'a Stmt, asked: &Expr) -> Taken<'a> {
    let value = match stmt {
        Stmt::Assign { value, .. }
        | Stmt::Eval(value)
        | Stmt::Store { value, .. }
        | Stmt::StoreBytes { value, .. } => value,
        _ => return Taken::Nothing,
    };
    let asks = match unwide(value) {
        Expr::Call {
            callee: Callee::Intrinsic { .. },
            ..
        } => false,
        Expr::Call { args, .. } => {
            matches!(args.as_slice(), [only] if unwide(only) == unwide(asked))
        }
        _ => false,
    };
    match (asks, stmt) {
        (false, _) => Taken::Nothing,
        (true, Stmt::Assign { target, .. }) => Taken::Assigned(*target),
        (true, Stmt::Eval(_)) => Taken::Dropped,
        (true, _) => Taken::Stored(stmt),
    }
}

/// How often a body assigns one variable, reads it and takes its address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    writes: u32,
    reads: u32,
    addressed: u32,
}

impl Tally {
    /// The tally of every variable a body names.
    fn all(body: &[tree::Stmt]) -> BTreeMap<Var, Tally> {
        let mut all: BTreeMap<Var, Tally> = BTreeMap::new();
        tree::each_simple(body, &mut |stmt| {
            for target in stmt.targets() {
                let held = all.entry(*target).or_default();
                held.writes = held.writes.saturating_add(1);
            }
        });
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::Var(var) => {
                    let held = all.entry(*var).or_default();
                    held.reads = held.reads.saturating_add(1);
                }
                Expr::AddrOf { var, .. } => {
                    let held = all.entry(*var).or_default();
                    held.addressed = held.addressed.saturating_add(1);
                }
                _ => {}
            });
        });
        all
    }

    /// The tally of `var` alone over simple statements.
    fn of_simples(stmts: &[Stmt], var: Var) -> Tally {
        let mut tally = Tally::default();
        for stmt in stmts {
            let writes = stmt
                .targets()
                .iter()
                .filter(|target| **target == var)
                .count();
            let writes = u32::try_from(writes).unwrap_or(u32::MAX);
            tally.writes = tally.writes.saturating_add(writes);
            stmt.each_expr(&mut |top| tally.expr(top, var));
        }
        tally
    }

    /// The tally of `var` alone over a body.
    fn of(body: &[tree::Stmt], var: Var) -> Tally {
        let mut tally = Tally::default();
        tree::each_simple(body, &mut |stmt| {
            let writes = stmt
                .targets()
                .iter()
                .filter(|target| **target == var)
                .count();
            let writes = u32::try_from(writes).unwrap_or(u32::MAX);
            tally.writes = tally.writes.saturating_add(writes);
        });
        tree::each_expr(body, &mut |top| tally.expr(top, var));
        tally
    }

    fn expr(&mut self, top: &Expr, var: Var) {
        top.walk(&mut |e| match e {
            Expr::Var(read) if *read == var => self.reads = self.reads.saturating_add(1),
            Expr::AddrOf { var: held, .. } if *held == var => {
                self.addressed = self.addressed.saturating_add(1);
            }
            _ => {}
        });
    }

    /// This tally and `more` together.
    fn plus(self, more: Tally) -> Tally {
        Tally {
            writes: self.writes.saturating_add(more.writes),
            reads: self.reads.saturating_add(more.reads),
            addressed: self.addressed.saturating_add(more.addressed),
        }
    }
}

/// An assignment that stands just before an await that takes no awaiter, of the variable the await's test asks.
struct Pair {
    task: Var,
    /// Whether nothing of the two in sight names the variable once the assignment is the await's own: the assignment reads it nowhere, and `after` only asks it for the result.
    // ⚠ In sight are the assignment and `after`: the test, the suspend path and the resume point
    // are the compiler's, and the variable may be stored, handed and taken back there.
    fits: bool,
    /// What the two do with the variable.
    tally: Tally,
}

/// The pair the statements `made` and `construct` are, one after the other.
fn pair(made: &tree::Stmt, construct: &tree::Stmt) -> Option<Pair> {
    let (tree::Stmt::Simple(made), tree::Stmt::Sugar(sugar)) = (made, construct) else {
        return None;
    };
    let Sugar::Await {
        before,
        ready,
        after,
        ..
    } = &**sugar
    else {
        return None;
    };
    let Stmt::Assign { target, .. } = made else {
        return None;
    };
    let task = *target;
    let asked = awaiter(ready)?;
    if !before.is_empty() || !framed(task) || *unwide(asked) != Expr::Var(task) {
        return None;
    }
    let own = Tally::of_simples(std::slice::from_ref(made), task);
    let shown = Tally::of_simples(after, task);
    // The one statement in sight that reads it asks it for the result, and is written as the await's own.
    let result = match shown.reads {
        0 => true,
        1 => matches!(
            (after.as_slice(), taken_of(after, asked)),
            ([_], Some(Taken::Assigned(_) | Taken::Dropped))
        ),
        _ => false,
    };
    let fits = own.reads == 0 && own.addressed == 0 && shown.addressed == 0 && result;
    let tally = own.plus(Tally::of(std::slice::from_ref(construct), task));
    Some(Pair { task, fits, tally })
}

/// The statement lists nested in `stmt`: those `tree::nested_mut` gives.
fn lists(stmt: &tree::Stmt) -> Vec<&[tree::Stmt]> {
    match stmt {
        tree::Stmt::If { then, els, .. } => vec![then.as_slice(), els.as_slice()],
        tree::Stmt::While { body, .. }
        | tree::Stmt::DoWhile { body, .. }
        | tree::Stmt::For { body, .. } => vec![body.as_slice()],
        tree::Stmt::Switch { cases, .. } => cases.iter().map(|case| case.body.as_slice()).collect(),
        tree::Stmt::Sugar(sugar) => sugar.parts().bodies,
        _ => Vec::new(),
    }
}

/// The variables whose every mention in `body` is in a pair that fits: each the awaiter of awaits alone, given its task just before each.
// ⚠ One variable may be the awaiter of several awaits: each is given its task in its own pair, or
// a mention left in sight would read a value given out of sight.
fn awaiters(body: &[tree::Stmt]) -> BTreeSet<Var> {
    let whole = Tally::all(body);
    let mut paired: BTreeMap<Var, (Tally, bool)> = BTreeMap::new();
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&[tree::Stmt]> = vec![body];
    while let Some(list) = work.pop() {
        for two in list.windows(2) {
            if let Some(found) = pair(&two[0], &two[1]) {
                let held = paired.entry(found.task).or_insert((Tally::default(), true));
                held.0 = held.0.plus(found.tally);
                held.1 &= found.fits;
            }
        }
        for stmt in list {
            work.extend(lists(stmt));
        }
    }
    paired
        .into_iter()
        .filter(|(task, (tally, fits))| *fits && whole.get(task) == Some(tally))
        .map(|(task, _)| task)
        .collect()
}

/// What the one statement of `after` makes of the result.
fn taken_of<'a>(after: &'a [Stmt], asked: &Expr) -> Option<Taken<'a>> {
    match after {
        [one] => Some(taken(one, asked)),
        _ => None,
    }
}

/// Takes into each await that takes no awaiter the assignment just before it that gives the awaiter the task, where nothing in sight but such awaits names that variable: `await f(a)`.
pub(super) fn tasks(body: &mut Vec<tree::Stmt>) {
    let taken = awaiters(body);
    if taken.is_empty() {
        return;
    }
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        let mut at = 1;
        while at < list.len() {
            let found = pair(&list[at - 1], &list[at]);
            let made = match (found, &list[at - 1]) {
                (Some(found), tree::Stmt::Simple(made)) if taken.contains(&found.task) => {
                    made.clone()
                }
                _ => {
                    at += 1;
                    continue;
                }
            };
            let moved = match &mut list[at] {
                tree::Stmt::Sugar(sugar) => match &mut **sugar {
                    Sugar::Await { before, .. } => {
                        before.push(made);
                        true
                    }
                    _ => false,
                },
                _ => false,
            };
            // The await now stands where the assignment stood, and the next pair starts with it.
            match moved {
                true => {
                    list.remove(at - 1);
                }
                false => at += 1,
            }
        }
        for stmt in list.iter_mut() {
            work.extend(tree::nested_mut(stmt));
        }
    }
}

#[cfg(test)]
#[path = "async_await_place_tests.rs"]
mod tests;
