//! The `gotos` rules that read a `goto` of the finished statements as the way out it is: the
//! end of an arm, a `break`, a `continue`, the test in front of the code it passes over, a
//! guard that returns, the one exit of a loop.
//!
//! Each rule holds only where the label a `goto` names stands at the place control reaches by
//! what is written instead, so what runs is what ran. ⚠ One rule writes a statement twice: a
//! `return` of a small value with no call, a `break` or a `continue`, in front of the arm it
//! followed. Two write a statement that says where running off already went: a `continue` at the
//! end of an arm that ran off the end of a loop's body, and a `break` at the end of a switch's
//! last body. No simple statement is copied and none is dropped. ⚠ One rule moves one: the
//! assignment of a constant to a variable no address reaches, from behind a construct whose
//! jumps past it each assign that variable first, to in front of the construct.
//!
//! Two rules run once over the finished statements, apart from the rounds: [`fallen`] writes the
//! body a `switch` body runs into at its end where that body is a few assignments with no call
//! and a statement that leaves, so ⚠ those are written twice; [`steps_last`] puts the arm of a
//! loop's last `if` that leaves first, so that the body ends in the step of its counter.

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg;
use crate::gta6::script::ir::{self, Callee, Const, Expr, Head, Kind, Var, Width};
// `PIECE`: the intrinsic `async_await::untangle` leaves a call of where it took a piece out.
use crate::gta6::script::passes::async_await::PIECE;
use crate::gta6::script::tree::{self, Label, Stmt};

use super::super::values::effects::Facts;
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

/// [`labels_at`], with where a `break`, a `continue` or a `goto` standing at `list[at]` goes on to.
// Such a statement runs nothing, so control that reaches it is control where it leads.
fn landing(list: &[Stmt], at: usize, ways: &Ways) -> Vec<Label> {
    let mut found = labels_at(list, at, &ways.after);
    let past = list.get(at..).map_or(0, |rest| {
        rest.iter()
            .take_while(|stmt| matches!(stmt, Stmt::Label(_)))
            .count()
    });
    match list.get(at + past) {
        Some(Stmt::Break) => found.extend(ways.brk.iter().flatten().copied()),
        Some(Stmt::Continue) => found.extend(ways.cont.iter().flatten().copied()),
        Some(Stmt::Goto(label)) => found.push(*label),
        _ => {}
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

/// The locals of `f` that only an assignment naming them reads or writes: no load, store or call reaches them through an address.
// ⚠ Read off the blocks before they are structured: the rules copy and move statements but
// never take an address no block took.
pub(super) fn alone(f: &cfg::Function) -> HashSet<Var> {
    let facts = Facts::of(f);
    f.head
        .vars
        .keys()
        .filter(|var| matches!(**var, Var::Local(_)) && facts.named_only(**var))
        .copied()
        .collect()
}

/// The variables of a function whose assignment may move in front of a construct: one cell each, and no address reaches them.
pub(super) struct Private {
    vars: HashSet<Var>,
}

impl Private {
    /// What `head` and `body` prove, with `alone` the locals [`alone`] proves; nothing where a piece of a state machine is out of sight.
    // Such a piece enters the body at labels this pass cannot see named, and may read anything.
    pub(super) fn of(head: &Head, body: &[Stmt], alone: &HashSet<Var>) -> Private {
        let mut addressed: HashSet<Var> = HashSet::new();
        let mut piece = false;
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::AddrOf { var, .. } => {
                    addressed.insert(*var);
                }
                Expr::Call {
                    callee: Callee::Intrinsic { name, .. },
                    ..
                } => piece |= name.as_str() == PIECE,
                _ => {}
            });
        });
        // The address of a frame cell, moved, may reach a local other than its own, unless the
        // blocks show that no address reaches that local.
        let frame = addressed
            .iter()
            .any(|var| matches!(var, Var::Local(_) | Var::Param(_)));
        let vars = match piece {
            true => HashSet::new(),
            false => head
                .vars
                .iter()
                .filter(|(var, info)| {
                    let plain = info.width == Width::Cell && !addressed.contains(*var);
                    let reached = frame && !alone.contains(*var);
                    let local = matches!(**var, Var::Local(_)) && info.single && !reached;
                    plain && (var.is_temporary() || local)
                })
                .map(|(var, _)| *var)
                .collect(),
        };
        Private { vars }
    }
}

/// Puts the lists `stmt` holds on `work`, each with whether a loop of the construct being read takes its `continue` and whether a loop or a switch of it takes its `break`.
fn push_nested<'a>(
    work: &mut Vec<(&'a [Stmt], bool, bool)>,
    stmt: &'a Stmt,
    looped: bool,
    broken: bool,
) {
    match stmt {
        Stmt::If { then, els, .. } => {
            work.push((then.as_slice(), looped, broken));
            work.push((els.as_slice(), looped, broken));
        }
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
            work.push((body.as_slice(), true, true));
        }
        Stmt::Switch { cases, .. } => {
            work.extend(
                cases
                    .iter()
                    .map(|case| (case.body.as_slice(), looped, true)),
            );
        }
        _ => {}
    }
}

/// Whether an assignment of `var` that follows `stmt`, with the labels `end` after it, may stand in front of `stmt` instead.
// Every `goto` of `stmt` to `end` comes right after an assignment of `var`, every assignment of
// `var` in it right before such a `goto`, and nothing of it reads `var`: on a path that runs off
// its end it neither reads nor assigns `var`, and on one that jumps it assigns `var` the value
// `var` then has at `end`. Nothing enters `stmt` but at its head and nothing leaves it but by
// running off, by `return` or by those jumps: any other way out would meet the moved value.
fn moves_over(named: &HashMap<Label, u32>, stmt: &Stmt, var: Var, end: &[Label]) -> bool {
    let whole = std::slice::from_ref(stmt);
    let mut read = false;
    tree::each_expr(whole, &mut |top| read |= cond::mentions(top, var));
    if read {
        return false;
    }
    let mut assigned = 0usize;
    tree::each_simple(whole, &mut |simple| {
        assigned += simple
            .targets()
            .iter()
            .filter(|target| **target == var)
            .count();
    });
    let assigns = |at: Option<&Stmt>| matches!(at, Some(Stmt::Simple(ir::Stmt::Assign { target, .. })) if *target == var);
    let jumps_out =
        |at: Option<&Stmt>| matches!(at, Some(Stmt::Goto(label)) if end.contains(label));
    let mut pairs = 0usize;
    let mut inside: HashMap<Label, u32> = HashMap::new();
    let mut defined: HashSet<Label> = HashSet::new();
    let mut elsewhere: Vec<Label> = Vec::new();
    let mut work: Vec<(&[Stmt], bool, bool)> = Vec::new();
    push_nested(&mut work, stmt, false, false);
    while let Some((list, looped, broken)) = work.pop() {
        for (k, at) in list.iter().enumerate() {
            match at {
                Stmt::Goto(label) => {
                    *inside.entry(*label).or_insert(0) += 1;
                    if !end.contains(label) {
                        elsewhere.push(*label);
                    } else if !assigns(k.checked_sub(1).and_then(|before| list.get(before))) {
                        return false;
                    }
                }
                Stmt::Simple(ir::Stmt::Assign { target, .. }) if *target == var => {
                    if !jumps_out(list.get(k + 1)) {
                        return false;
                    }
                    pairs += 1;
                }
                Stmt::Label(label) => {
                    defined.insert(*label);
                }
                Stmt::Break if !broken => return false,
                Stmt::Continue if !looped => return false,
                Stmt::Sugar(_) => return false,
                _ => push_nested(&mut work, at, looped, broken),
            }
        }
    }
    let stays = elsewhere.iter().all(|label| defined.contains(label));
    let entered = defined.iter().any(|label| {
        let all = named.get(label).copied().unwrap_or(0);
        all > inside.get(label).copied().unwrap_or(0)
    });
    pairs > 0 && pairs == assigned && stays && !entered
}

struct Exits<'a> {
    /// The `goto`s that name each label.
    named: &'a mut HashMap<Label, u32>,
    /// The variables whose assignment may move; `None` moves none.
    private: Option<&'a Private>,
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
            if self.hoist(list, i, ways) {
                // The assignment stands at `i`; the construct it went in front of is looked at next.
                i += 1;
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
            let next = landing(list, i + 1, ways);
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
                    self.lifted(body);
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
                    self.lifted(body);
                    let inner = Ways {
                        after: Vec::new(),
                        brk: Some(next),
                        cont: Some(labels_before(body.as_slice(), body.len())),
                    };
                    self.list(body, &inner, depth + 1);
                }
                // ⚠ A body that runs off its end runs into the next one: a jump out of it is a
                // `break` and never nothing. The last runs off the switch to what follows it, and
                // [`closed_cases`] writes its `break` back where a rule took its jump out.
                Stmt::Switch { cases, .. } => {
                    let last = cases.len().saturating_sub(1);
                    for (at, case) in cases.iter_mut().enumerate() {
                        let inner = Ways {
                            after: match at == last {
                                true => next.clone(),
                                false => Vec::new(),
                            },
                            brk: Some(next.clone()),
                            cont: ways.cont.clone(),
                        };
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
        if landing(list, i + 1, ways).contains(&label) {
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

    /// `C; v = k; L:` is `v = k; C; L:` where [`moves_over`] holds for the construct `C`; `true` when it was written so.
    // `for (..) { if (found) { v = i; goto L; } } v = -1; L:`: with the constant set first, each
    // jump of the loop is to where the loop runs on to, which the rules here then read as such.
    fn hoist(&mut self, list: &mut Vec<Stmt>, i: usize, ways: &Ways) -> bool {
        let Some(private) = self.private else {
            return false;
        };
        let var = match (list.get(i), list.get(i + 1)) {
            (
                Some(
                    Stmt::If { .. }
                    | Stmt::While { .. }
                    | Stmt::DoWhile { .. }
                    | Stmt::For { .. }
                    | Stmt::Switch { .. },
                ),
                Some(Stmt::Simple(ir::Stmt::Assign {
                    target,
                    value: Expr::Const(_),
                })),
            ) if private.vars.contains(target) => *target,
            _ => return false,
        };
        let end = landing(list, i + 2, ways);
        if end.is_empty() || !moves_over(&*self.named, &list[i], var, &end) {
            return false;
        }
        let assign = list.remove(i + 1);
        list.insert(i, assign);
        self.changed = true;
        true
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

    /// `if (c) { T } else { E; L: R }` that ends a loop's `body` is `if (c) { T; continue; } else { E } L: R` where `T` jumps to `L`: that jump then runs off the `if`.
    // Off the end of a loop's body is where `continue` goes in `while`, `do` and `for` alike, so
    // `T` run off its end still goes there, and `R` runs off the body as it ran off the else. A
    // label taken out of an arm is still in reach of every `goto` that reached it.
    fn lifted(&mut self, body: &mut Vec<Stmt>) {
        let Some(Stmt::If { then, els, .. }) = body.last_mut() else {
            return;
        };
        let Some(at) = els.iter().position(|stmt| matches!(stmt, Stmt::Label(_))) else {
            return;
        };
        let inside = free(then.as_slice(), &labels_at(els.as_slice(), at, &[]));
        if inside.jumps == 0 || inside.opaque {
            return;
        }
        let rest = els.split_off(at);
        if runs_off(then.as_slice()) {
            then.push(Stmt::Continue);
        }
        body.extend(rest);
        self.changed = true;
    }
}

/// Ends the last body of each `switch` that runs off its end with the `break` that leaves the switch there.
// The rules read the end of a last body as what follows the switch, and so may take out the
// jump that ended it. A body that ends in a loop is left: a loop no `break` leaves never runs off.
fn closed_cases(body: &mut Vec<Stmt>) {
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            if let Stmt::Switch { cases, .. } = &mut *stmt {
                if let Some(case) = cases.last_mut() {
                    let looped = matches!(
                        case.body.last(),
                        Some(Stmt::While { .. } | Stmt::DoWhile { .. } | Stmt::For { .. })
                    );
                    if !looped && runs_off(&case.body) {
                        case.body.push(Stmt::Break);
                    }
                }
            }
            work.extend(super::nested(stmt));
        }
    }
}

/// The assignments a body that another runs into may hold in front of its leaving statement to be written there too.
const FALLEN: usize = 2;

/// What `next`, a body another runs into, is written as at the end of that one: past its labels, at most [`FALLEN`] assignments of a value with no call, then one statement [`copies`] allows, and nothing else.
fn fallen_tail(next: &[Stmt]) -> Option<Vec<Stmt>> {
    let start = next
        .iter()
        .take_while(|stmt| matches!(stmt, Stmt::Label(_)))
        .count();
    let (last, simples) = next[start..].split_last()?;
    let plain = simples.iter().all(|stmt| {
        matches!(stmt, Stmt::Simple(ir::Stmt::Assign { value, .. }) if cond::cheap(Some(value)))
    });
    (simples.len() <= FALLEN && plain && copies(last)).then(|| next[start..].to_vec())
}

/// Whether control may run off the end of a switch's `body` into the next: it holds a statement, and does not end in a loop.
// A body of labels alone stands above the next one's labels; a loop no `break` leaves never runs
// off, and a copy behind it would be reached by nothing.
fn runs_into_next(body: &[Stmt]) -> bool {
    let held = body.iter().any(|stmt| !matches!(stmt, Stmt::Label(_)));
    let looped = matches!(
        body.last(),
        Some(Stmt::While { .. } | Stmt::DoWhile { .. } | Stmt::For { .. })
    );
    held && !looped && runs_off(body)
}

/// `case 1: s1; goto case 2; case 2: x = 0; return;` is `case 1: s1; x = 0; return; case 2: ..`: each body that runs into one [`fallen_tail`] reads ends in a copy of it; whether one was written.
// Running off the end of a body is entering the next, and a `break`, a `continue` or a `return`
// leaves to one place from either, so each path still runs one copy and leaves as it did. The
// last bodies are read first, so a copy that ends a body may be copied on into the one before.
pub(super) fn fallen(body: &mut Vec<Stmt>) -> bool {
    let mut changed = false;
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            if let Stmt::Switch { cases, .. } = &mut *stmt {
                for at in (1..cases.len()).rev() {
                    let Some(tail) = fallen_tail(&cases[at].body) else {
                        continue;
                    };
                    let before = &mut cases[at - 1].body;
                    if runs_into_next(before) {
                        before.extend(tail);
                        changed = true;
                    }
                }
            }
            work.extend(super::nested(stmt));
        }
    }
    changed
}

/// Whether the `while (cond)` loop with this `body`, behind an assignment of `counter`, ends in an `if` to be turned: its then arm ends by stepping `counter`, which `cond` tests, and its else never runs off.
// ⚠ No `continue` of the loop: it goes round past what the then arm ends in, and a `for` would
// step there. A construct this cannot see into may hold one.
fn turns_to_step(counter: Var, cond: &Expr, body: &[Stmt]) -> bool {
    let Some(Stmt::If { then, els, .. }) = body.last() else {
        return false;
    };
    let steps = matches!(
        then.last(),
        Some(Stmt::Simple(ir::Stmt::Assign { target, value }))
            if *target == counter && cond::mentions(value, counter)
    );
    let held = free(body, &[]);
    let open = held.continues == 0 && !held.opaque;
    steps && open && cond::mentions(cond, counter) && !els.is_empty() && !runs_off(els)
}

/// `i = 0; while (c) { ..; if (t) { A; i = i + 1; } else { B } }` is `i = 0; while (c) { ..; if (!t) { B } A; i = i + 1; }` where `B` never runs off; whether one was turned.
// The test runs once either way, `B` runs where `t` fails and leaves, and `A` runs where it
// holds and runs on to the end of the body as it ran off the then arm. A label of `A` still
// leads to what follows it there. The body then ends in the step a `for` takes as its own.
pub(super) fn steps_last(body: &mut Vec<Stmt>) -> bool {
    let mut changed = false;
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for i in 1..list.len() {
            let counter = match &list[i - 1] {
                Stmt::Simple(ir::Stmt::Assign { target, .. }) => *target,
                _ => continue,
            };
            let Stmt::While { cond, body } = &mut list[i] else {
                continue;
            };
            if !turns_to_step(counter, cond, body) {
                continue;
            }
            let moved = match body.last_mut() {
                Some(Stmt::If { cond, then, els }) => {
                    let test = std::mem::replace(cond, Expr::GuidEmpty);
                    *cond = cond::negate(test);
                    std::mem::swap(then, els);
                    std::mem::take(els)
                }
                _ => continue,
            };
            body.extend(moved);
            changed = true;
        }
        for stmt in list {
            work.extend(super::nested(stmt));
        }
    }
    changed
}

/// Writes each `goto` a rule explains as the way out it is, once over `body`; whether anything was rewritten. `private` lets assignments of its variables move.
// ⚠ `named` must count every `goto` of `body`: a label is dropped once nothing names it, and an
// entry into a construct from outside it is told by it.
pub(super) fn rewritten(
    body: &mut Vec<Stmt>,
    named: &mut HashMap<Label, u32>,
    private: Option<&Private>,
) -> bool {
    let mut exits = Exits {
        named,
        private,
        changed: false,
    };
    exits.list(body, &Ways::default(), 0);
    closed_cases(body);
    exits.changed
}

#[cfg(test)]
#[path = "structure_exits_tests.rs"]
mod tests;
