//! The blocks a compiler expands, read back: a `while` with its counter as `for`, a resource and
//! its disposal as `using`, a monitor entered and left as `lock`, and the regions a front end
//! states beside a function as `try`.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::{
    closing, disposal_before, disposed, hide, is_const, under, used, var_of, Form, Found, Scope,
};
use crate::gta6::script::cfg::{self, Terminator};
use crate::gta6::script::ir::{self, BinOp, Callee, Expr, TypeKey, Var};
use crate::gta6::script::tree::{self, Catch, Guard, Label, Stmt, Sugar};
use crate::gta6::script::{Context, Pass};

/// The handlers of one protected run, in the order they catch: what each catches and the block it starts at.
type Handlers = Vec<(Option<TypeKey>, usize)>;

/// The statements back from an exit that its entry is looked for in.
const SPAN: usize = 1024;

/// The exits of one list found to close nothing before no more of that list are tried.
const TRIES: usize = 64;

/// The regions of one function that are read as `try`s; a function that states more is written as its blocks.
const REGIONS: usize = 4096;

/// Whether `body` holds a `break` (where `breaks`) or a `continue` (where `continues`) of no loop and no switch inside it: one that leaves `body` itself.
fn leaves(body: &[Stmt], breaks: bool, continues: bool) -> bool {
    let mut work = vec![(body, breaks, continues)];
    while let Some((list, breaks, continues)) = work.pop() {
        for stmt in list {
            match stmt {
                Stmt::Break if breaks => return true,
                Stmt::Continue if continues => return true,
                Stmt::If { then, els, .. } => {
                    work.push((then.as_slice(), breaks, continues));
                    work.push((els.as_slice(), breaks, continues));
                }
                Stmt::Switch { cases, .. } => {
                    let bodies = cases.iter().map(|case| case.body.as_slice());
                    work.extend(bodies.map(|body| (body, false, continues)));
                }
                Stmt::Sugar(sugar) => match &**sugar {
                    // The loop keeps its own; what runs once it is left does not.
                    Sugar::Foreach { after, .. } => {
                        work.push((after.as_slice(), breaks, continues));
                    }
                    other => {
                        let bodies = other.parts().bodies;
                        work.extend(bodies.into_iter().map(|body| (body, breaks, continues)));
                    }
                },
                // A loop keeps its own, and nothing else holds a statement.
                _ => {}
            }
        }
    }
    false
}

/// What a statement sets or steps: a variable, or what lies at an offset from an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place<'a> {
    Var(Var),
    At(&'a Expr, u32),
}

/// Whether `e` is itself a read of `place`.
fn reads(e: &Expr, place: Place<'_>) -> bool {
    match (e, place) {
        (Expr::Var(var), Place::Var(held)) => *var == held,
        (Expr::Load { addr, offset, .. }, Place::At(held, at)) => **addr == *held && *offset == at,
        _ => false,
    }
}

/// The place `stmt` sets.
fn set(stmt: &ir::Stmt) -> Option<Place<'_>> {
    match stmt {
        ir::Stmt::Assign { target, .. } => Some(Place::Var(*target)),
        ir::Stmt::Store { addr, offset, .. } => Some(Place::At(addr, *offset)),
        _ => None,
    }
}

/// The place `stmt` steps by one: a variable it sets to what the variable held and one more or less, or what a step through an address steps.
// ⚠ Any other step, and a store of what a place held and more, is none here: in a body it is
// read as the `+=` it is, and in the head of a `for` nothing reads it so.
fn stepped(stmt: &ir::Stmt) -> Option<Place<'_>> {
    match stmt {
        ir::Stmt::StepI32 { addr, offset, .. } => Some(Place::At(addr, *offset)),
        ir::Stmt::Assign { value, .. } => {
            let place = set(stmt)?;
            let own = match under(value, true) {
                Expr::Binary {
                    op: BinOp::Add | BinOp::Sub,
                    l,
                    r,
                    ..
                } => is_const(r, 1) && reads(under(l, true), place),
                _ => false,
            };
            own.then_some(place)
        }
        _ => None,
    }
}

/// Whether `init` and the loop after it are a `for`: the loop tests a place `init` sets, and its body ends by stepping that place with no `continue` that would go round the step.
fn counted(init: &Stmt, looped: &Stmt) -> bool {
    let (Stmt::Simple(init), Stmt::While { cond, body }) = (init, looped) else {
        return false;
    };
    let Some((Stmt::Simple(step), rest)) = body.split_last() else {
        return false;
    };
    let Some(place) = stepped(step) else {
        return false;
    };
    let mut tested = false;
    cond.walk(&mut |e| tested |= reads(e, place));
    tested && set(init) == Some(place) && !leaves(rest, false, true)
}

/// Reads as a `for` each `while` of `body`, at every depth, that a counter is set just before and stepped at the end of; returns how many.
// The same statements in the same order: a `for` runs its step where the body ended, and only
// a `continue` would tell the two apart.
pub(super) fn counted_loops(body: &mut Vec<Stmt>) -> usize {
    let mut made = 0;
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        // Each statement moves once, however many loops the list holds.
        let mut out: Vec<Stmt> = Vec::with_capacity(list.len());
        for stmt in std::mem::take(list) {
            let is_for = out.last().is_some_and(|init| counted(init, &stmt));
            match (is_for, stmt) {
                (true, Stmt::While { cond, mut body }) => match (out.pop(), body.pop()) {
                    (Some(Stmt::Simple(init)), Some(Stmt::Simple(step))) => {
                        out.push(Stmt::For {
                            init: Some(Box::new(init)),
                            cond: Some(cond),
                            step: Some(Box::new(step)),
                            body,
                        });
                        made += 1;
                    }
                    // `counted` saw both: anything else goes back where it stood.
                    (init, step) => {
                        out.extend(init);
                        body.extend(step);
                        out.push(Stmt::While { cond, body });
                    }
                },
                (_, stmt) => out.push(stmt),
            }
        }
        *list = out;
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
    made
}

/// The `using` that `list` ends in, and where in `list` it starts: the taking of a variable, the statements after it and its disposal where it is not null, where every way out of them disposes so and nothing else names the variable.
// ⚠ A `using` disposes when a fault leaves it too, and these statements do not: the two read
// the same only where no fault is caught, which holds of a function no region is stated of.
// A disposal with no test for null is none of a `using`: that faults where this does nothing.
fn using(list: &[Stmt], scope: &Scope<'_>) -> Option<(Stmt, usize)> {
    let cx = &scope.cx;
    let (held, len, (form, _)) = closing(list, cx)?;
    if form != Form::Guarded {
        return None;
    }
    let end = list.len().checked_sub(len)?;
    let first = (end.saturating_sub(SPAN)..end).rev().find(
        |at| matches!(&list[*at], Stmt::Simple(ir::Stmt::Assign { target, .. }) if *target == held),
    )?;
    let Stmt::Simple(enter) = &list[first] else {
        return None;
    };
    // The function names the variable nowhere else, and sets it once: where it is taken.
    let inside = used(&list[first..]);
    let own = scope.uses.get(&held) == inside.get(&held) && !scope.exposed(held);
    let once = inside.get(&held).is_some_and(|uses| uses.assigned == 1);
    let body = &list[first + 1..end];
    if !own || !once || leaves(body, true, true) {
        return None;
    }
    let mut body = body.to_vec();
    // What each disposal reads of the variable is within the statements counted above.
    let mut read = BTreeMap::new();
    let mut leaving = |before: &[Stmt]| disposal_before(before, held, form, cx, &mut read);
    hide(&mut body, scope, true, &mut leaving)?;
    let made = Sugar::Guarded {
        kind: Guard::Using,
        enter: vec![enter.clone()],
        body,
        exit: list[end..].to_vec(),
    };
    Some((Stmt::Sugar(Box::new(made)), first))
}

/// Whether `stmt` enters a monitor, else leaves one, and the object it is given, where it is the call of one of the two alone.
// The path is a list's or a listing's own, as `member_of` reads a disposal's.
fn monitor(stmt: &Stmt) -> Option<(bool, &Expr)> {
    let Stmt::Simple(ir::Stmt::Eval(call)) = stmt else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Builtin { path, .. },
        args,
        ..
    } = under(call, false)
    else {
        return None;
    };
    let [object] = args.as_slice() else {
        return None;
    };
    let (owner, member) = path.rsplit_once('.')?;
    let enters = match member {
        "Enter" => true,
        "Exit" => false,
        _ => return None,
    };
    (owner == "Monitor" || owner.ends_with(".Monitor")).then_some((enters, object))
}

/// The `lock` that `list` ends in, and where in `list` it starts: a monitor entered on a variable, the statements after it and the leaving of it, where every way out of them leaves it and nothing changes the variable between.
// ⚠ As a `using`: a `lock` leaves the monitor when a fault leaves it too.
fn lock(list: &[Stmt], scope: &Scope<'_>) -> Option<(Stmt, usize)> {
    let (last, before) = list.split_last()?;
    let (false, object) = monitor(last)? else {
        return None;
    };
    let held = var_of(object, false).filter(|var| !scope.exposed(*var))?;
    let of = |stmt: &Stmt| {
        let (enters, object) = monitor(stmt)?;
        (var_of(object, false) == Some(held)).then_some(enters)
    };
    // An exit met on the way back closes an entry of its own, further back.
    let mut open = 0usize;
    let mut first = None;
    for at in (before.len().saturating_sub(SPAN)..before.len()).rev() {
        match of(&before[at]) {
            Some(true) if open == 0 => {
                first = Some(at);
                break;
            }
            Some(true) => open -= 1,
            Some(false) => open += 1,
            None => {}
        }
    }
    let first = first?;
    let Stmt::Simple(enter) = &before[first] else {
        return None;
    };
    let body = &before[first + 1..];
    let set_again = used(body).get(&held).is_some_and(|uses| uses.assigned > 0);
    if set_again || leaves(body, true, true) {
        return None;
    }
    let mut body = body.to_vec();
    let mut leaving = |seq: &[Stmt]| (of(seq.last()?) == Some(false)).then_some(1usize);
    hide(&mut body, scope, true, &mut leaving)?;
    let made = Sugar::Guarded {
        kind: Guard::Lock,
        enter: vec![enter.clone()],
        body,
        exit: vec![last.clone()],
    };
    Some((Stmt::Sugar(Box::new(made)), first))
}

/// Reads the runs of statements of `list` itself that are a `using` or a `lock`.
pub(super) fn guards(list: &mut Vec<Stmt>, scope: &Scope<'_>, found: &mut Found) {
    let cx = &scope.cx;
    let (usings, locks) = (cx.options.on(Pass::Using), cx.options.on(Pass::Lock));
    let closes = |stmt: &Stmt| {
        let disposes = usings && disposed(stmt, cx).is_some();
        disposes || (locks && matches!(monitor(stmt), Some((false, _))))
    };
    if !list.iter().any(&closes) {
        return;
    }
    // From the end, so that a block around another is read first and hides what it runs on the
    // way out before the one inside looks for its own; each statement moves once.
    let mut done: Vec<Stmt> = Vec::with_capacity(list.len());
    let mut tries = 0;
    while let Some(last) = list.last() {
        let mut made = None;
        if tries < TRIES && closes(last) {
            if usings {
                made = using(list, scope).map(|(stmt, first)| (stmt, first, true));
            }
            if made.is_none() && locks {
                made = lock(list, scope).map(|(stmt, first)| (stmt, first, false));
            }
            if made.is_none() {
                tries += 1;
            }
        }
        match made {
            Some((stmt, first, disposes)) => {
                list.truncate(first);
                done.push(stmt);
                match disposes {
                    true => found.usings += 1,
                    false => found.locks += 1,
                }
            }
            None => done.extend(list.pop()),
        }
    }
    done.reverse();
    *list = done;
}

/// Where a block is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Home {
    /// 0 in the order of the blocks, outside every `try`; else the number of the body or the handler the block is of.
    piece: usize,
    handler: bool,
}

/// Outside every `try`.
const MAIN: Home = Home {
    piece: 0,
    handler: false,
};

/// One `try`: the blocks it protects, and each handler's with the type it takes.
struct Protected {
    body: Range<usize>,
    catches: Vec<(Option<TypeKey>, Range<usize>)>,
}

/// The `try`s of a function, by the first block of each rising, and where each of its blocks is written.
struct Plan {
    tries: Vec<Protected>,
    home: Vec<Home>,
}

/// The `try`s `regions` state of `f`; `None` where they are not runs of blocks apart from one another, each with handlers that take a type and that only a fault enters.
// A handler runs from its first block to the next block the entry reaches, or the next run or
// handler: no jump leads to it, so what the entry does not reach after it is its own.
fn plan(f: &cfg::Function, regions: &[cfg::Region]) -> Option<Plan> {
    let blocks = f.cfg.blocks.len();
    if regions.is_empty() || regions.len() > REGIONS {
        return None;
    }
    let reached = f.cfg.reachable();
    let mut starts: BTreeSet<usize> = BTreeSet::new();
    let mut runs: BTreeMap<(usize, usize), Handlers> = BTreeMap::new();
    for region in regions {
        // ⚠ A `finally` runs on the way out with no fault too, and nothing states where it ends.
        let cfg::RegionKind::Catch(ty) = region.kind else {
            return None;
        };
        let (first, last) = (*region.blocks.first()?, *region.blocks.last()?);
        let mut run = region.blocks.iter().enumerate();
        let run = run.all(|(n, block)| first.checked_add(n) == Some(*block));
        let handler = region.handler;
        if !run || last >= blocks || handler >= blocks || reached[handler] {
            return None;
        }
        starts.insert(first);
        starts.insert(handler);
        runs.entry((first, last + 1))
            .or_default()
            .push((ty, handler));
    }

    let mut tries = Vec::with_capacity(runs.len());
    let mut taken: Vec<Range<usize>> = Vec::new();
    for ((first, end), handlers) in runs {
        let mut catches = Vec::with_capacity(handlers.len());
        for (ty, handler) in handlers {
            let mut to = handler + 1;
            while to < blocks && !reached[to] && !starts.contains(&to) {
                to += 1;
            }
            taken.push(handler..to);
            catches.push((ty, handler..to));
        }
        taken.push(first..end);
        tries.push(Protected {
            body: first..end,
            catches,
        });
    }
    taken.sort_by_key(|range| range.start);
    if !taken.windows(2).all(|pair| pair[0].end <= pair[1].start) {
        return None;
    }

    let mut home = vec![MAIN; blocks];
    let mut piece = 0;
    for protected in &tries {
        let body = (&protected.body, false);
        let handlers = protected.catches.iter().map(|(_, range)| (range, true));
        for (range, handler) in std::iter::once(body).chain(handlers) {
            piece += 1;
            for b in range.clone() {
                home[b] = Home { piece, handler };
            }
        }
    }
    // ⚠ Written back, a `try` loses its handlers: a jump into one from outside it would name a
    // label that is then nowhere.
    let entered = f.cfg.blocks.iter().enumerate().any(|(b, block)| {
        let mut to = block.term.successors().into_iter();
        to.any(|to| home.get(to).is_some_and(|at| at.handler) && home.get(to) != home.get(b))
    });
    (!entered).then_some(Plan { tries, home })
}

/// Whether control that runs off the end of block `b` is at block `to`, the next, where both are written: side by side in one body or one handler, or in the order of the blocks outside every handler.
// ⚠ A handler is written with its `try`, away from the blocks around it: what runs off its
// end is at the statement after the `try`, which need not be the next block.
fn falls(home: &[Home], b: usize, to: usize) -> bool {
    let handler = |at: usize| home.get(at).is_some_and(|at| at.handler);
    home.get(b) == home.get(to) || (!handler(b) && !handler(to))
}

/// The blocks of `range` as labels and jumps, as `cfg::flat` writes a function's; a block is taken out of `blocks` as it is written.
fn flat_run(
    blocks: &mut [Option<cfg::Block>],
    labels: &[Label],
    named: &[bool],
    home: &[Home],
    range: Range<usize>,
) -> Vec<Stmt> {
    let go = |to: usize| {
        let label = labels.get(to).copied().unwrap_or(Label::Block(to as u32));
        vec![Stmt::Goto(label)]
    };
    let mut out = Vec::new();
    for b in range {
        let Some(block) = blocks.get_mut(b).and_then(Option::take) else {
            continue;
        };
        let label = labels.get(b).filter(|_| named.get(b) == Some(&true));
        out.extend(label.map(|label| Stmt::Label(*label)));
        out.extend(block.stmts.into_iter().map(Stmt::Simple));
        match block.term {
            Terminator::Jump(to) if to == b + 1 && falls(home, b, to) => {}
            Terminator::Jump(to) => out.extend(go(to)),
            Terminator::Branch { cond, then, els } => out.push(Stmt::If {
                cond,
                then: go(then),
                els: go(els),
            }),
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let mut all: Vec<tree::Case> = cases
                    .into_iter()
                    .map(|(at, to)| tree::Case {
                        labels: vec![Some(at)],
                        body: go(to),
                    })
                    .collect();
                all.push(tree::Case {
                    labels: vec![None],
                    body: go(default),
                });
                out.push(Stmt::Switch { value, cases: all });
            }
            Terminator::Return(value) => out.push(Stmt::Return(value)),
            Terminator::Unreachable => out.push(Stmt::Unreachable),
        }
    }
    out
}

/// Writes `f` with each run of `plan` and its handlers as one `try`, where the run stood.
fn written(f: cfg::Function, plan: &Plan) -> tree::Function {
    let Plan { tries, home } = plan;
    let count = f.cfg.blocks.len();
    let labels: Vec<Label> = (f.cfg.blocks.iter().enumerate())
        .map(|(b, block)| match block.at {
            Some(at) => Label::At(at),
            None => Label::Block(b as u32),
        })
        .collect();
    // A block is labelled where something jumps to it other than the block before running into it.
    let mut named = vec![false; count];
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for to in block.term.successors() {
            let runs_in =
                matches!(block.term, Terminator::Jump(_)) && to == b + 1 && falls(home, b, to);
            if let (false, Some(flag)) = (runs_in, named.get_mut(to)) {
                *flag = true;
            }
        }
    }
    let mut blocks: Vec<Option<cfg::Block>> = f.cfg.blocks.into_iter().map(Some).collect();
    let mut body = Vec::new();
    let mut b = 0;
    while b < count {
        // A handler is written with its `try`, wherever that stands.
        if home.get(b).is_some_and(|at| at.handler) {
            b += 1;
            continue;
        }
        let starts = tries.binary_search_by_key(&b, |protected| protected.body.start);
        match starts.ok().and_then(|k| tries.get(k)) {
            Some(protected) => {
                let catches = (protected.catches.iter())
                    .map(|(ty, range)| Catch {
                        ty: *ty,
                        // ⚠ Nothing states where a handler finds the fault it takes.
                        var: None,
                        body: flat_run(&mut blocks, &labels, &named, home, range.clone()),
                    })
                    .collect();
                let run = protected.body.clone();
                body.push(Stmt::Sugar(Box::new(Sugar::Try {
                    body: flat_run(&mut blocks, &labels, &named, home, run),
                    catches,
                    finally: Vec::new(),
                })));
                b = protected.body.end.max(b + 1);
            }
            None => {
                body.extend(flat_run(&mut blocks, &labels, &named, home, b..b + 1));
                b += 1;
            }
        }
    }
    tree::Function { head: f.head, body }
}

/// Writes a function a front end states regions of as its blocks in their order, each protected run with its handlers as one `try`; as `cfg::flat` writes it where `try` is off or the regions are not such runs.
pub fn protected(f: cfg::Function, regions: &[cfg::Region], cx: &Context<'_>) -> tree::Function {
    let read = match cx.options.on(Pass::Try) {
        true => plan(&f, regions),
        false => None,
    };
    match read {
        Some(read) => written(f, &read),
        None => cfg::flat(f),
    }
}

#[cfg(test)]
#[path = "sugar_statements_guards_tests.rs"]
mod tests;
