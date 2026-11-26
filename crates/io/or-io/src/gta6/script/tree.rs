//! The structured form: statements nested in ifs, loops and switches, as a listing states a
//! function and as the printer takes one.

use std::collections::BTreeSet;
use std::fmt;

use super::ir::{self, Expr, Head, TypeKey, Var};

/// What a `goto` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Label {
    /// `label_OFFSET`: a bytecode offset the listing labels.
    At(u32),
    /// `block_N`: a block no listing label names, numbered by whoever made the label.
    Block(u32),
}

impl Label {
    /// The label a name spells: `label_0001B9` or `block_12`.
    pub fn from_name(name: &str) -> Option<Label> {
        if let Some(at) = name.strip_prefix("label_") {
            let hex = !at.is_empty() && at.bytes().all(|b| b.is_ascii_hexdigit());
            return hex
                .then(|| u32::from_str_radix(at, 16).ok())
                .flatten()
                .map(Label::At);
        }
        let n = name.strip_prefix("block_")?;
        let digits = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| n.parse().ok()).flatten().map(Label::Block)
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Label::At(at) => write!(f, "label_{at:06X}"),
            Label::Block(n) => write!(f, "block_{n}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stmt {
    Simple(ir::Stmt),
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    DoWhile {
        body: Vec<Stmt>,
        cond: Expr,
    },
    /// `None` leaves that part of the head empty.
    For {
        init: Option<Box<ir::Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<ir::Stmt>>,
        body: Vec<Stmt>,
    },
    Switch {
        value: Expr,
        cases: Vec<Case>,
    },
    Break,
    Continue,
    Return(Option<Expr>),
    Goto(Label),
    Label(Label),
    /// The listing's `throw` for control that falls off the bytecode: nothing may reach it.
    Unreachable,
    /// A source construct a pass proved, in place of the statements it holds; [`Sugar::desugar`] writes them back.
    Sugar(Box<Sugar>),
}

/// A source construct, made of the statements and tests it was proved from.
// ⚠ It holds each of them once, by the role it plays, and nothing computed from them: the
// printer shows the construct by reading the roles, and a rewrite inside a part shows in both
// forms. A pass makes one only where the whole pattern holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sugar {
    /// Statements the compiler added and no source wrote: a state machine's dispatch, a closure object made.
    Hidden(Vec<Stmt>),
    /// Simple statements that are one statement of the source, read as `kind` says.
    Group { kind: Group, parts: Vec<ir::Stmt> },
    /// `await`: `before` takes the awaiter, `ready` tests it, `suspend` leaves until it is, `resume` runs at `resumed` once it is, `after` takes the result.
    Await {
        before: Vec<ir::Stmt>,
        ready: Expr,
        /// Ends in the `return` that hands control back.
        suspend: Vec<Stmt>,
        /// The label the state machine's dispatch jumps to.
        resumed: Label,
        resume: Vec<Stmt>,
        after: Vec<ir::Stmt>,
    },
    /// `yield return`: `value` stores what is yielded, `suspend` leaves, `resume` runs at `resumed` on the next call.
    Yield {
        value: Vec<ir::Stmt>,
        suspend: Vec<Stmt>,
        resumed: Label,
        resume: Vec<Stmt>,
    },
    /// `foreach`: `setup` runs once, `cond` before each round, `item` first in it, `step` last, `after` once the loop is left.
    Foreach {
        setup: Vec<ir::Stmt>,
        cond: Expr,
        item: Box<ir::Stmt>,
        body: Vec<Stmt>,
        /// An index stepped; `None` where an enumerator is advanced by `cond`.
        step: Option<Box<ir::Stmt>>,
        after: Vec<Stmt>,
    },
    /// `using` or `lock`: `enter` runs before `body` and `exit` after it.
    Guarded {
        kind: Guard,
        enter: Vec<ir::Stmt>,
        body: Vec<Stmt>,
        exit: Vec<Stmt>,
    },
    /// `try`, with its handlers and its `finally`.
    // ⚠ `desugar` drops the handlers: no fault is raised by the check that runs through it.
    Try {
        body: Vec<Stmt>,
        catches: Vec<Catch>,
        finally: Vec<Stmt>,
    },
    /// `throw`: the statement that raises, after which nothing runs.
    Throw(Box<ir::Stmt>),
    /// `switch` over tests that are not the cases of one integer: each arm runs where its test is the first to hold.
    // ⚠ A `break` in an arm would leave this switch in the source and an enclosing loop in the
    // statements: a pass makes this only where no arm holds one.
    SwitchOn { arms: Vec<Arm>, default: Vec<Stmt> },
    /// `place += handler` or `place -= handler` on an event: `seed` reads the delegate held, each round of `round` joins the handler to it and exchanges the result for what was read, and `again` holds while another writer came between.
    Event {
        seed: Box<ir::Stmt>,
        round: Vec<ir::Stmt>,
        again: Expr,
    },
    /// `cache ??= delegate`: `test` asks a global for the delegate kept there, `made` makes and keeps it where the global is null, `kept` takes it where it is not.
    Cached {
        test: Expr,
        /// Whether `test` holds where the global is null, so that `made` is the arm that runs then.
        null: bool,
        made: Vec<Stmt>,
        kept: Vec<Stmt>,
    },
}

/// What simple statements that are one statement of the source stand for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// `x = new T(a) { F = b }`: an object made, constructed, and its fields stored.
    Construct,
    /// `x = new T[] { a, b }` or `new List { a, b }`: a collection made and its elements stored.
    Collection,
    /// `d = receiver.Method` or `d = (a) => ..`: a delegate made and bound.
    Delegate,
    /// `place op= value`, `place++`: one statement that works on the place it writes.
    Compound,
    /// `base.Name = value`: one store to a field of the type `owner`.
    Field { owner: TypeKey },
    /// `F(out var x)`: cells made zero, then filled by the call that follows.
    Out,
    /// `(a, b) = F()`: a value of several cells taken apart.
    Deconstruct,
    /// `s = a + b` over strings: the pieces gathered and the call that joins them.
    Concat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Guard {
    Using,
    Lock,
}

/// One handler of a `try`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catch {
    /// The type caught, `None` for any.
    pub ty: Option<TypeKey>,
    /// The variable the fault is bound to.
    pub var: Option<Var>,
    pub body: Vec<Stmt>,
}

/// One arm of a [`Sugar::SwitchOn`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub test: Expr,
    pub body: Vec<Stmt>,
}

/// What a source construct holds itself, by kind of part.
pub struct Parts<'a> {
    /// Its simple statements, in the order they ran.
    pub simples: Vec<&'a ir::Stmt>,
    /// Its tests.
    pub exprs: Vec<&'a Expr>,
    /// The statement lists nested in it.
    pub bodies: Vec<&'a [Stmt]>,
}

/// [`Parts`], to rewrite.
pub struct PartsMut<'a> {
    pub simples: Vec<&'a mut ir::Stmt>,
    pub exprs: Vec<&'a mut Expr>,
    pub bodies: Vec<&'a mut Vec<Stmt>>,
}

fn simple(parts: Vec<ir::Stmt>) -> impl Iterator<Item = Stmt> {
    parts.into_iter().map(Stmt::Simple)
}

impl Sugar {
    /// The statements this stands for, in the order they ran; a construct nested in one of them stays.
    pub fn desugar(self) -> Vec<Stmt> {
        match self {
            Sugar::Hidden(stmts) => stmts,
            Sugar::Group { parts, .. } => simple(parts).collect(),
            Sugar::Await {
                before,
                ready,
                suspend,
                resumed,
                resume,
                after,
            } => {
                let mut waited = suspend;
                waited.push(Stmt::Label(resumed));
                waited.extend(resume);
                let mut out: Vec<Stmt> = simple(before).collect();
                out.push(Stmt::If {
                    cond: ready,
                    then: Vec::new(),
                    els: waited,
                });
                out.extend(simple(after));
                out
            }
            Sugar::Yield {
                value,
                suspend,
                resumed,
                resume,
            } => {
                let mut out: Vec<Stmt> = simple(value).collect();
                out.extend(suspend);
                out.push(Stmt::Label(resumed));
                out.extend(resume);
                out
            }
            Sugar::Foreach {
                setup,
                cond,
                item,
                body,
                step,
                after,
            } => {
                let mut round = vec![Stmt::Simple(*item)];
                round.extend(body);
                let mut out: Vec<Stmt> = simple(setup).collect();
                out.push(match step {
                    Some(step) => Stmt::For {
                        init: None,
                        cond: Some(cond),
                        step: Some(step),
                        body: round,
                    },
                    None => Stmt::While { cond, body: round },
                });
                out.extend(after);
                out
            }
            Sugar::Guarded {
                enter, body, exit, ..
            } => {
                let mut out: Vec<Stmt> = simple(enter).collect();
                out.extend(body);
                out.extend(exit);
                out
            }
            Sugar::Try { body, finally, .. } => {
                let mut out = body;
                out.extend(finally);
                out
            }
            Sugar::Throw(raise) => vec![Stmt::Simple(*raise), Stmt::Unreachable],
            Sugar::Event { seed, round, again } => vec![
                Stmt::Simple(*seed),
                Stmt::DoWhile {
                    body: simple(round).collect(),
                    cond: again,
                },
            ],
            Sugar::Cached {
                test,
                null,
                made,
                kept,
            } => {
                let (then, els) = match null {
                    true => (made, kept),
                    false => (kept, made),
                };
                vec![Stmt::If {
                    cond: test,
                    then,
                    els,
                }]
            }
            Sugar::SwitchOn { arms, default } => {
                let mut els = default;
                for arm in arms.into_iter().rev() {
                    els = vec![Stmt::If {
                        cond: arm.test,
                        then: arm.body,
                        els,
                    }];
                }
                els
            }
        }
    }

    /// The simple statements, the tests and the statement lists this holds itself.
    pub fn parts(&self) -> Parts<'_> {
        let mut parts = Parts {
            simples: Vec::new(),
            exprs: Vec::new(),
            bodies: Vec::new(),
        };
        match self {
            Sugar::Hidden(stmts) => parts.bodies.push(stmts),
            Sugar::Group { parts: held, .. } => parts.simples.extend(held),
            Sugar::Await {
                before,
                ready,
                suspend,
                resume,
                after,
                ..
            } => {
                parts.simples.extend(before.iter().chain(after));
                parts.exprs.push(ready);
                parts.bodies.extend([suspend.as_slice(), resume.as_slice()]);
            }
            Sugar::Yield {
                value,
                suspend,
                resume,
                ..
            } => {
                parts.simples.extend(value);
                parts.bodies.extend([suspend.as_slice(), resume.as_slice()]);
            }
            Sugar::Foreach {
                setup,
                cond,
                item,
                body,
                step,
                after,
            } => {
                parts.simples.extend(setup);
                parts.simples.push(item);
                parts.simples.extend(step.as_deref());
                parts.exprs.push(cond);
                parts.bodies.extend([body.as_slice(), after.as_slice()]);
            }
            Sugar::Guarded {
                enter, body, exit, ..
            } => {
                parts.simples.extend(enter);
                parts.bodies.extend([body.as_slice(), exit.as_slice()]);
            }
            Sugar::Try {
                body,
                catches,
                finally,
            } => {
                parts.bodies.push(body);
                parts
                    .bodies
                    .extend(catches.iter().map(|c| c.body.as_slice()));
                parts.bodies.push(finally);
            }
            Sugar::Event { seed, round, again } => {
                parts.simples.push(seed);
                parts.simples.extend(round);
                parts.exprs.push(again);
            }
            Sugar::Cached {
                test, made, kept, ..
            } => {
                parts.exprs.push(test);
                parts.bodies.extend([made.as_slice(), kept.as_slice()]);
            }
            Sugar::Throw(raise) => parts.simples.push(raise),
            Sugar::SwitchOn { arms, default } => {
                for arm in arms {
                    parts.exprs.push(&arm.test);
                    parts.bodies.push(&arm.body);
                }
                parts.bodies.push(default);
            }
        }
        parts
    }

    /// [`Sugar::parts`], to rewrite.
    pub fn parts_mut(&mut self) -> PartsMut<'_> {
        let mut parts = PartsMut {
            simples: Vec::new(),
            exprs: Vec::new(),
            bodies: Vec::new(),
        };
        match self {
            Sugar::Hidden(stmts) => parts.bodies.push(stmts),
            Sugar::Group { parts: held, .. } => parts.simples.extend(held),
            Sugar::Await {
                before,
                ready,
                suspend,
                resume,
                after,
                ..
            } => {
                parts.simples.extend(before.iter_mut().chain(after));
                parts.exprs.push(ready);
                parts.bodies.extend([suspend, resume]);
            }
            Sugar::Yield {
                value,
                suspend,
                resume,
                ..
            } => {
                parts.simples.extend(value);
                parts.bodies.extend([suspend, resume]);
            }
            Sugar::Foreach {
                setup,
                cond,
                item,
                body,
                step,
                after,
            } => {
                parts.simples.extend(setup);
                parts.simples.push(item);
                parts.simples.extend(step.as_deref_mut());
                parts.exprs.push(cond);
                parts.bodies.extend([body, after]);
            }
            Sugar::Guarded {
                enter, body, exit, ..
            } => {
                parts.simples.extend(enter);
                parts.bodies.extend([body, exit]);
            }
            Sugar::Try {
                body,
                catches,
                finally,
            } => {
                parts.bodies.push(body);
                parts.bodies.extend(catches.iter_mut().map(|c| &mut c.body));
                parts.bodies.push(finally);
            }
            Sugar::Event { seed, round, again } => {
                parts.simples.push(seed);
                parts.simples.extend(round);
                parts.exprs.push(again);
            }
            Sugar::Cached {
                test, made, kept, ..
            } => {
                parts.exprs.push(test);
                parts.bodies.extend([made, kept]);
            }
            Sugar::Throw(raise) => parts.simples.push(raise),
            Sugar::SwitchOn { arms, default } => {
                for arm in arms {
                    parts.exprs.push(&mut arm.test);
                    parts.bodies.push(&mut arm.body);
                }
                parts.bodies.push(default);
            }
        }
        parts
    }
}

/// The statement lists nested in `stmt`, a source construct's included.
pub fn nested_mut(stmt: &mut Stmt) -> Vec<&mut Vec<Stmt>> {
    match stmt {
        Stmt::If { then, els, .. } => vec![then, els],
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
            vec![body]
        }
        Stmt::Switch { cases, .. } => cases.iter_mut().map(|case| &mut case.body).collect(),
        Stmt::Sugar(sugar) => sugar.parts_mut().bodies,
        Stmt::Simple(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Return(_)
        | Stmt::Goto(_)
        | Stmt::Label(_)
        | Stmt::Unreachable => Vec::new(),
    }
}

/// Calls `f` on each expression `stmt` evaluates itself, those of a source construct's own parts included and those of nested lists not.
pub fn own_exprs_mut(stmt: &mut Stmt, f: &mut impl FnMut(&mut Expr)) {
    match stmt {
        Stmt::Simple(s) => s.each_expr_mut(f),
        Stmt::If { cond, .. }
        | Stmt::While { cond, .. }
        | Stmt::DoWhile { cond, .. }
        | Stmt::Switch { value: cond, .. }
        | Stmt::Return(Some(cond)) => f(cond),
        Stmt::For {
            init, cond, step, ..
        } => {
            if let Some(s) = init {
                s.each_expr_mut(f);
            }
            if let Some(cond) = cond {
                f(cond);
            }
            if let Some(s) = step {
                s.each_expr_mut(f);
            }
        }
        Stmt::Sugar(sugar) => {
            let parts = sugar.parts_mut();
            for s in parts.simples {
                s.each_expr_mut(f);
            }
            for e in parts.exprs {
                f(e);
            }
        }
        Stmt::Return(None)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Goto(_)
        | Stmt::Label(_)
        | Stmt::Unreachable => {}
    }
}

/// `body` with every source construct written back as the statements and the expressions it stands for.
// An explicit stack: a body nests as deep as its listing does.
pub fn desugar(body: Vec<Stmt>) -> Vec<Stmt> {
    let mut body = body;
    let mut work: Vec<&mut Vec<Stmt>> = vec![&mut body];
    while let Some(list) = work.pop() {
        // A construct may stand for another: a list is written back until it holds none.
        while list.iter().any(|stmt| matches!(stmt, Stmt::Sugar(_))) {
            let mut lowered = Vec::with_capacity(list.len());
            for stmt in list.drain(..) {
                match stmt {
                    Stmt::Sugar(sugar) => lowered.extend(sugar.desugar()),
                    other => lowered.push(other),
                }
            }
            *list = lowered;
        }
        for stmt in list {
            own_exprs_mut(stmt, &mut |e| e.desugar());
            work.extend(nested_mut(stmt));
        }
    }
    body
}

/// One body of a switch and the labels that lead to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    /// `None` is `default`.
    pub labels: Vec<Option<i64>>,
    /// A body that does not leave falls into the next case's.
    pub body: Vec<Stmt>,
}

/// A function in the structured form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub head: Head,
    pub body: Vec<Stmt>,
}

/// How much of each thing a body holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Simple statements, a loop head's included.
    pub statements: usize,
    pub gotos: usize,
    pub labels: usize,
}

pub fn counts(body: &[Stmt]) -> Counts {
    let mut counts = Counts::default();
    count(body, true, &mut counts);
    counts
}

/// [`counts`] of what the source level writes: what a `Hidden` holds is no statement and no jump there.
pub fn shown(body: &[Stmt]) -> Counts {
    let mut counts = Counts::default();
    count(body, false, &mut counts);
    counts
}

fn count(body: &[Stmt], hidden: bool, counts: &mut Counts) {
    for stmt in body {
        match stmt {
            Stmt::Simple(_) => counts.statements += 1,
            Stmt::If { then, els, .. } => {
                count(then, hidden, counts);
                count(els, hidden, counts);
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => count(body, hidden, counts),
            Stmt::For {
                init, step, body, ..
            } => {
                counts.statements += usize::from(init.is_some()) + usize::from(step.is_some());
                count(body, hidden, counts);
            }
            Stmt::Switch { cases, .. } => {
                for case in cases {
                    count(&case.body, hidden, counts);
                }
            }
            Stmt::Goto(_) => counts.gotos += 1,
            Stmt::Label(_) => counts.labels += 1,
            Stmt::Break | Stmt::Continue | Stmt::Return(_) | Stmt::Unreachable => {}
            Stmt::Sugar(sugar) if !hidden && matches!(**sugar, Sugar::Hidden(_)) => {}
            Stmt::Sugar(sugar) => {
                let parts = sugar.parts();
                counts.statements += parts.simples.len();
                for body in parts.bodies {
                    count(body, hidden, counts);
                }
            }
        }
    }
}

/// Calls `f` on every source construct of a body, one inside another after it.
pub fn each_sugar<'a>(body: &'a [Stmt], f: &mut impl FnMut(&'a Sugar)) {
    for stmt in body {
        match stmt {
            Stmt::If { then, els, .. } => {
                each_sugar(then, f);
                each_sugar(els, f);
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                each_sugar(body, f);
            }
            Stmt::Switch { cases, .. } => {
                for case in cases {
                    each_sugar(&case.body, f);
                }
            }
            Stmt::Sugar(sugar) => {
                f(sugar);
                for body in sugar.parts().bodies {
                    each_sugar(body, f);
                }
            }
            Stmt::Simple(_)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Return(_)
            | Stmt::Goto(_)
            | Stmt::Label(_)
            | Stmt::Unreachable => {}
        }
    }
}

/// Calls `f` on every simple statement of a body, in source order.
pub fn each_simple<'a>(body: &'a [Stmt], f: &mut impl FnMut(&'a ir::Stmt)) {
    for stmt in body {
        match stmt {
            Stmt::Simple(s) => f(s),
            Stmt::If { then, els, .. } => {
                each_simple(then, f);
                each_simple(els, f);
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => each_simple(body, f),
            Stmt::For {
                init, step, body, ..
            } => {
                if let Some(s) = init.as_deref() {
                    f(s);
                }
                each_simple(body, f);
                if let Some(s) = step.as_deref() {
                    f(s);
                }
            }
            Stmt::Switch { cases, .. } => {
                for case in cases {
                    each_simple(&case.body, f);
                }
            }
            Stmt::Break
            | Stmt::Continue
            | Stmt::Return(_)
            | Stmt::Goto(_)
            | Stmt::Label(_)
            | Stmt::Unreachable => {}
            Stmt::Sugar(sugar) => {
                let parts = sugar.parts();
                for s in parts.simples {
                    f(s);
                }
                for body in parts.bodies {
                    each_simple(body, f);
                }
            }
        }
    }
}

/// Calls `f` on every expression a body evaluates: those of its statements, its conditions, its
/// switch values and what it returns. `f` is not called on the expressions inside them.
pub fn each_expr<'a>(body: &'a [Stmt], f: &mut impl FnMut(&'a Expr)) {
    for stmt in body {
        match stmt {
            Stmt::Simple(s) => s.each_expr(f),
            Stmt::If { cond, then, els } => {
                f(cond);
                each_expr(then, f);
                each_expr(els, f);
            }
            Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
                f(cond);
                each_expr(body, f);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(s) = init {
                    s.each_expr(f);
                }
                if let Some(cond) = cond {
                    f(cond);
                }
                each_expr(body, f);
                if let Some(s) = step {
                    s.each_expr(f);
                }
            }
            Stmt::Switch { value, cases } => {
                f(value);
                for case in cases {
                    each_expr(&case.body, f);
                }
            }
            Stmt::Return(Some(e)) => f(e),
            Stmt::Return(None)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Goto(_)
            | Stmt::Label(_)
            | Stmt::Unreachable => {}
            Stmt::Sugar(sugar) => {
                let parts = sugar.parts();
                for s in parts.simples {
                    s.each_expr(f);
                }
                for e in parts.exprs {
                    f(e);
                }
                for body in parts.bodies {
                    each_expr(body, f);
                }
            }
        }
    }
}

/// The `v_` and `stack_` variables a body still reads, writes or takes the address of.
pub fn temporaries(body: &[Stmt]) -> BTreeSet<Var> {
    let mut found = BTreeSet::new();
    each_simple(body, &mut |s| {
        found.extend(s.targets().iter().copied().filter(|v| v.is_temporary()));
    });
    each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(v) | Expr::AddrOf { var: v, .. } = e {
                if v.is_temporary() {
                    found.insert(*v);
                }
            }
        });
    });
    found
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
