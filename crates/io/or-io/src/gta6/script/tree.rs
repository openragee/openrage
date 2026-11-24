//! The structured form: statements nested in ifs, loops and switches, as a listing states a
//! function and as the printer takes one.

use std::collections::BTreeSet;
use std::fmt;

use super::ir::{self, Expr, Head, Var};

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
    count(body, &mut counts);
    counts
}

fn count(body: &[Stmt], counts: &mut Counts) {
    for stmt in body {
        match stmt {
            Stmt::Simple(_) => counts.statements += 1,
            Stmt::If { then, els, .. } => {
                count(then, counts);
                count(els, counts);
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => count(body, counts),
            Stmt::For {
                init, step, body, ..
            } => {
                counts.statements += usize::from(init.is_some()) + usize::from(step.is_some());
                count(body, counts);
            }
            Stmt::Switch { cases, .. } => {
                for case in cases {
                    count(&case.body, counts);
                }
            }
            Stmt::Goto(_) => counts.gotos += 1,
            Stmt::Label(_) => counts.labels += 1,
            Stmt::Break | Stmt::Continue | Stmt::Return(_) | Stmt::Unreachable => {}
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
