//! Closures: a delegate made and bound read as one statement, and the function it binds read as
//! a lambda's body where what it is bound to is an object of a class the compiler made.
//!
//! Works on the structured form and moves no statement: the allocation and the constructor call
//! become the two parts of a `Group::Delegate`, and the function reference an
//! `ExprSugar::Lambda`. [`sole`] says which bodies a program names once.
//!
//! ⚠ `Pass::LocalFunctions` and `Pass::Events` switch nothing yet. A local function that
//! captures takes its variables through an address, as any `ref` argument does, so none is
//! proved; an event's `+=` is a load and a loop, which no `tree::Sugar` holds.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::ir::{self, BinOp, Callee, Expr, ExprSugar, Scalar, UnOp, Var};
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[
    Pass::Lambdas,
    Pass::LocalFunctions,
    Pass::Delegates,
    Pass::Events,
];

/// The bit of a type's flag word that a type the compiler made carries: a closure class, a state machine.
const GENERATED: u32 = 0x0010_0000;
/// The bits of a type's flag word of which a value type carries one or more.
const VALUE: u32 = 0x1C;
/// The bit a builtin type's id carries above its index in the runtime's table.
const BUILTIN: u32 = 0x8000_0000;

/// What the rewrites made of one function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Delegates made and bound, each now one group.
    pub delegates: usize,
    /// Functions bound to a closure object the same function makes.
    pub lambdas: usize,
    /// Functions bound to a holder in a global and kept in another: lambdas that capture nothing.
    pub cached: usize,
}

/// A delegate made and bound: the variable it is held in, the object bound and the function bound.
struct Made<'a> {
    delegate: Var,
    target: &'a Expr,
    function: &'a Expr,
}

/// Reads each delegate made and bound as one statement, and the function bound to a closure as a lambda's body.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    rewrite(f, cx);
}

/// [`run`], with the count of what it made.
pub fn rewrite(f: &mut tree::Function, cx: &Context<'_>) -> Found {
    let group = cx.options.on(Pass::Delegates);
    let lambdas = cx.options.on(Pass::Lambdas);
    let mut found = Found::default();
    if !group && !lambdas {
        return found;
    }
    let program = f.head.program;
    let objects = match lambdas {
        true => closures(&f.body),
        false => BTreeSet::new(),
    };
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        if group {
            found.delegates += grouped(list);
        }
        if lambdas {
            for i in 0..list.len() {
                let closure = made(list, i).is_some_and(|(bound, _)| {
                    matches!(bare(bound.target), Expr::Var(held) if objects.contains(held))
                });
                if closure && read_as_lambda(list, i) {
                    found.lambdas += 1;
                }
            }
            for stmt in list.iter_mut() {
                let Stmt::If { cond, then, els } = stmt else {
                    continue;
                };
                let Some((cache, null)) = null_test(cond) else {
                    continue;
                };
                let (made_in, kept_in) = match null {
                    true => (then, els),
                    false => (els, then),
                };
                let at = cached(cache, made_in, kept_in, program);
                if at.is_some_and(|at| read_as_lambda(made_in, at)) {
                    found.cached += 1;
                }
            }
        }
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
    found
}

/// The functions of one program that are a lambda's body and that the program names once, by offset.
// ⚠ A function another program binds or calls is named there, and no table of the project
// says so: whoever writes a body where it is bound must know the program is read alone.
pub fn sole(functions: &[tree::Function]) -> BTreeSet<u32> {
    let mut named: BTreeMap<u32, u32> = BTreeMap::new();
    let mut bodies = BTreeSet::new();
    for f in functions {
        let program = f.head.program;
        tree::each_expr(&f.body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::FunctionRef { program: of, at } if *of == program => {
                    *named.entry(*at).or_default() += 1;
                }
                Expr::Call {
                    callee: Callee::Local { at, .. },
                    ..
                } => *named.entry(*at).or_default() += 1,
                Expr::Sugar {
                    kind: ExprSugar::Lambda,
                    e: inner,
                } => {
                    if let Expr::FunctionRef { program: of, at } = &**inner {
                        if *of == program {
                            bodies.insert(*at);
                        }
                    }
                }
                _ => {}
            });
        });
    }
    bodies.retain(|at| named.get(at) == Some(&1));
    bodies
}

/// `e` under its source readings and its casts to a whole cell, which change no bit of a reference.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. }
    | Expr::Cast {
        to: Scalar::U64 | Scalar::I64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |e| {
        found |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

/// What `alloc` and `ctor` make where they are a delegate made and bound.
// No name is asked: an object of a builtin type, handed with a target and a function reference
// to a builtin of that same type, is what only the runtime's delegate is made by.
fn delegate<'a>(alloc: &ir::Stmt, ctor: &'a ir::Stmt) -> Option<Made<'a>> {
    let ir::Stmt::Assign {
        target: held,
        value,
    } = alloc
    else {
        return None;
    };
    let Expr::NewObject(ty) = bare(value) else {
        return None;
    };
    let ir::Stmt::Eval(call) = ctor else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Builtin { ty: builtin, .. },
        args,
        ..
    } = bare(call)
    else {
        return None;
    };
    let [object, target, function] = args.as_slice() else {
        return None;
    };
    let own = ty.id & BUILTIN != 0 && ty.id & !BUILTIN == *builtin;
    let reference = matches!(
        bare(function),
        Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::VirtualFunctionRef { .. }
    );
    // A target that reads the delegate would read it half made in the one-statement form.
    let again = mentions(target, *held) || mentions(function, *held);
    let whole = own && reference && !again && *bare(object) == Expr::Var(*held);
    whole.then_some(Made {
        delegate: *held,
        target,
        function,
    })
}

/// The delegate made at `list[i]` and the statements of the list it takes: two side by side, or one group that holds both.
fn made(list: &[Stmt], i: usize) -> Option<(Made<'_>, usize)> {
    match (list.get(i)?, list.get(i + 1)) {
        (Stmt::Simple(alloc), Some(Stmt::Simple(ctor))) => delegate(alloc, ctor).map(|m| (m, 2)),
        (Stmt::Sugar(sugar), _) => match &**sugar {
            Sugar::Group {
                kind: Group::Construct | Group::Delegate,
                parts,
            } => match parts.as_slice() {
                [alloc, ctor] => delegate(alloc, ctor).map(|m| (m, 1)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Puts each delegate made by one statement and bound by the next into one group, and counts them.
// One pass that rebuilds the list: a function holds thousands of them.
fn grouped(list: &mut Vec<Stmt>) -> usize {
    let mut count = 0;
    let old = std::mem::take(list);
    list.reserve(old.len());
    let mut rest = old.into_iter().peekable();
    while let Some(stmt) = rest.next() {
        let pair = match (&stmt, rest.peek()) {
            (Stmt::Simple(alloc), Some(Stmt::Simple(ctor))) => delegate(alloc, ctor).is_some(),
            _ => false,
        };
        let bound = match pair {
            true => rest.next(),
            false => None,
        };
        match (stmt, bound) {
            (Stmt::Simple(alloc), Some(Stmt::Simple(ctor))) => {
                count += 1;
                list.push(Stmt::Sugar(Box::new(Sugar::Group {
                    kind: Group::Delegate,
                    parts: vec![alloc, ctor],
                })));
            }
            (mut stmt, bound) => {
                count += usize::from(regrouped(&mut stmt));
                list.push(stmt);
                list.extend(bound);
            }
        }
    }
    count
}

/// Reads a pair the constructors pass gathered as the delegate it is; `false` where `stmt` is no such pair.
fn regrouped(stmt: &mut Stmt) -> bool {
    let Stmt::Sugar(sugar) = stmt else {
        return false;
    };
    let Sugar::Group { kind, parts } = &mut **sugar else {
        return false;
    };
    let pair = match parts.as_slice() {
        [alloc, ctor] => *kind == Group::Construct && delegate(alloc, ctor).is_some(),
        _ => false,
    };
    if pair {
        *kind = Group::Delegate;
    }
    pair
}

/// The variables that hold a closure object: assigned once in the body, an object of a class the compiler made, the address never taken.
fn closures(body: &[Stmt]) -> BTreeSet<Var> {
    let mut assigned: BTreeMap<Var, u32> = BTreeMap::new();
    tree::each_simple(body, &mut |s| {
        let ir::Stmt::Assign { target, value } = s else {
            return;
        };
        let class = matches!(
            bare(value),
            Expr::NewObject(ty) if ty.flags & GENERATED != 0 && ty.flags & VALUE == 0
        );
        // A parameter and a global hold a value before any statement runs.
        if class && !matches!(target, Var::Param(_) | Var::Global { .. }) {
            assigned.insert(*target, 0);
        }
    });
    if assigned.is_empty() {
        return BTreeSet::new();
    }
    tree::each_simple(body, &mut |s| {
        for target in s.targets() {
            if let Some(times) = assigned.get_mut(target) {
                *times += 1;
            }
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::AddrOf { var, .. } = e {
                assigned.remove(var);
            }
        });
    });
    assigned
        .into_iter()
        .filter(|(_, times)| *times == 1)
        .map(|(var, _)| var)
        .collect()
}

/// Reads the function bound by the delegate made at `list[i]` as a lambda's body; `false` where it is read so already or is no function of a program.
fn read_as_lambda(list: &mut [Stmt], i: usize) -> bool {
    // The constructor call is the next statement, or the second part of the group at `i`.
    let at = match list.get(i) {
        Some(Stmt::Sugar(_)) => i,
        _ => i + 1,
    };
    let ctor = match list.get_mut(at) {
        Some(Stmt::Simple(ctor)) => ctor,
        Some(Stmt::Sugar(sugar)) => match &mut **sugar {
            Sugar::Group { parts, .. } if parts.len() == 2 => &mut parts[1],
            _ => return false,
        },
        _ => return false,
    };
    let ir::Stmt::Eval(call) = ctor else {
        return false;
    };
    let mut call = call;
    while let Expr::Sugar { e: inner, .. } = call {
        call = &mut **inner;
    }
    let Expr::Call { args, .. } = call else {
        return false;
    };
    let Some(function) = args.get_mut(2) else {
        return false;
    };
    if !matches!(function, Expr::FunctionRef { .. }) {
        return false;
    }
    let bound = std::mem::replace(function, Expr::GuidEmpty);
    *function = Expr::sugar(ExprSugar::Lambda, bound);
    true
}

/// The global a condition tests against null, and whether the condition holds where the global is null.
fn null_test(cond: &Expr) -> Option<(Var, bool)> {
    let (mut e, mut negated) = (bare(cond), false);
    while let Expr::Unary {
        op: UnOp::Not,
        e: inner,
    } = e
    {
        e = bare(inner);
        negated = !negated;
    }
    let Expr::Binary { op, l, r, .. } = e else {
        return None;
    };
    let null = match op {
        BinOp::Eq => true,
        BinOp::Ne => false,
        _ => return None,
    };
    match (bare(l), bare(r)) {
        (Expr::Var(global @ Var::Global { .. }), Expr::Const(zero)) if zero.bits == 0 => {
            Some((*global, null != negated))
        }
        _ => None,
    }
}

/// Whether a statement only carries a value to where the arms of an `if` meet: it assigns a variable that is no global a value that calls and makes nothing.
fn carried(stmt: &Stmt) -> bool {
    let Stmt::Simple(ir::Stmt::Assign { target, value }) = stmt else {
        return false;
    };
    let mut quiet = !matches!(target, Var::Global { .. });
    value.walk(&mut |e| {
        quiet &= !matches!(
            e,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    quiet
}

/// Where the arm that runs while the global `cache` is null makes the delegate it keeps there, when that is all the two arms do.
// The compiler's own form for a lambda that captures nothing: bound to a holder kept in a
// global, made once and kept in `cache`. The arms may carry values to where they meet, as the
// stack does across a branch; anything else leaves the function a method someone wrote a
// cache for.
fn cached(cache: Var, made_in: &[Stmt], kept_in: &[Stmt], program: u32) -> Option<usize> {
    let at = (0..made_in.len()).find(|i| made(made_in, *i).is_some())?;
    let (bound, taken) = made(made_in, at)?;
    let holder = matches!(
        bare(bound.target),
        Expr::Var(held @ Var::Global { .. }) if *held != cache
    );
    let own = matches!(
        bare(bound.function),
        Expr::FunctionRef { program: of, .. } if *of == program
    );
    let (kept, after) = made_in.get(at + taken..)?.split_first()?;
    let keeps = matches!(
        kept,
        Stmt::Simple(ir::Stmt::Assign { target, value })
            if *target == cache && *bare(value) == Expr::Var(bound.delegate)
    );
    let only = made_in[..at]
        .iter()
        .chain(after)
        .chain(kept_in)
        .all(carried);
    (holder && own && keeps && only).then_some(at)
}

#[cfg(test)]
#[path = "closures_tests.rs"]
mod tests;
