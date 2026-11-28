//! Calls of the runtime's library in C#'s spelling: a method of an object on its receiver, a
//! property of a type on the type, and an object made and constructed as the one `new` it was.
//!
//! ⚠ What a member is comes from the table of the library's public members, by the path a user's
//! list gives the call; a call the table does not hold, or holds in no shape its arguments take,
//! stays as it is. Each reading is an `Expr::Sugar` or a `Group`, so `tree::desugar` gives back
//! the call it was made of.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::ir::{self, Callee, Expr, ExprSugar, Kind, Scalar, Var, VarInfo, Width};
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

use super::sugar_expressions::Reached;

#[path = "sugar_runtime_table.rs"]
mod table;

use table::Reading;

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Runtime];

/// The bit a builtin type's id carries above its index in the runtime's table.
const BUILTIN: u32 = 0x8000_0000;

/// Whether a call of the runtime's member `path` with `args` arguments is of an overload its library declares, every public overload of that member returning `bool`.
// ⚠ What the declaration returns: the cell such a call yields is a truth only where the user's
// list names the call rightly, as every reading of this file takes it to.
pub fn truth(path: &str, args: usize) -> bool {
    table::truth(path, args)
}

/// The kind a call of the runtime's member `path` with `args` arguments yields, where its library declares every public overload of that member to return it: a truth, a number or a string.
// ⚠ As `truth`: only where the user's list names the call rightly. A member whose overloads
// return objects, values of several cells or enums states none.
pub fn result(path: &str, args: usize) -> Option<Kind> {
    table::result(path, args)
}

/// Reads each call of the runtime's library the table holds as the C# it is.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    if !cx.options.on(Pass::Runtime) {
        return;
    }
    let body = &mut f.body;
    let frame = Frame::of(body, &f.head.vars);
    let made = made(body, &frame, &f.head.vars);
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        construct(list, &frame);
        for stmt in list {
            tops(stmt, &mut |top, taken| read_all(top, taken, &made));
            work.extend(tree::nested_mut(stmt));
        }
    }
}

/// Calls `f` on each expression at the top of `stmt`, a construct's parts and a `for` head included, with whether its value is taken for sure: not where a call is the whole of a statement.
fn tops(stmt: &mut Stmt, f: &mut impl FnMut(&mut Expr, bool)) {
    match stmt {
        Stmt::Simple(simple) => simple_tops(simple, f),
        Stmt::For {
            init, cond, step, ..
        } => {
            if let Some(init) = init {
                simple_tops(init, f);
            }
            if let Some(cond) = cond {
                f(cond, true);
            }
            if let Some(step) = step {
                simple_tops(step, f);
            }
        }
        // ⚠ The expressions a construct holds apart from its statements are tests, whose value is
        // what they are for.
        Stmt::Sugar(sugar) => {
            let parts = sugar.parts_mut();
            for simple in parts.simples {
                simple_tops(simple, f);
            }
            for test in parts.exprs {
                f(test, true);
            }
        }
        other => tree::own_exprs_mut(other, &mut |top| f(top, true)),
    }
}

/// Calls `f` on each expression of one simple statement, with whether its value is taken: an `Eval`'s is not.
fn simple_tops(stmt: &mut ir::Stmt, f: &mut impl FnMut(&mut Expr, bool)) {
    let taken = !matches!(stmt, ir::Stmt::Eval(_));
    stmt.each_expr_mut(&mut |top| f(top, taken));
}

/// Reads every call inside `top` the table says what it is, but a call another reading already holds.
fn read_all(top: &mut Expr, taken: bool, made: &BTreeMap<Var, u32>) {
    let mut held: BTreeSet<*const Expr> = BTreeSet::new();
    top.walk(&mut |e| {
        if let Expr::Sugar { e: inner, .. } = e {
            held.insert(std::ptr::from_ref(&**inner));
        }
    });
    let whole = std::ptr::from_ref(&*top);
    // The walk reaches each expression after those inside it, and the whole one last, at the
    // place it was handed: a reading made inside moves nothing `held` points at.
    top.walk_mut(&mut |e| {
        let at = std::ptr::from_ref(&*e);
        if held.contains(&at) {
            return;
        }
        read(e, taken || at != whole, made);
    });
}

/// Reads one call as the method call or the use of a type's property it is; `taken` says whether its value is.
fn read(e: &mut Expr, taken: bool, made: &BTreeMap<Var, u32>) {
    let Expr::Call {
        callee: Callee::Builtin { path, ty, .. },
        args,
        returns,
    } = &*e
    else {
        return;
    };
    let kind = match table::reading(path, args.len()) {
        Some(Reading::Method) if args.first().is_some_and(object) => ExprSugar::Method,
        // ⚠ `x.M()` of a method an override may replace runs the override: the call reads so only
        // on an object of exactly the type that declares it.
        Some(Reading::Virtual)
            if args
                .first()
                .is_some_and(|receiver| exact(receiver, made) == Some(*ty & !BUILTIN)) =>
        {
            ExprSugar::Method
        }
        // ⚠ `T.P;` alone is no statement C# has.
        Some(Reading::StaticGetter) if taken && returns.is_some_and(|n| n > 0) => {
            ExprSugar::Property
        }
        _ => return,
    };
    *e = Expr::sugar(kind, std::mem::replace(e, Expr::GuidEmpty));
}

/// Whether `e` may be a reference a method can be called on: no address, no constant, no load narrower than a cell and no address of an element.
// ⚠ A value type's method is handed the value's address, which `x.M()` would have to write as
// the place, not the address: the table reads no member of a value type on its receiver.
fn object(e: &Expr) -> bool {
    let mut e = e;
    loop {
        match e {
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::U64 | Scalar::I64,
                e: inner,
            } => e = &**inner,
            Expr::Load { ty, .. } => return matches!(ty, Scalar::U64 | Scalar::I64),
            Expr::Call { callee, .. } => {
                return !matches!(
                    callee,
                    Callee::Intrinsic { name, .. } if name == "array_element_address"
                );
            }
            Expr::Var(_)
            | Expr::Cell { .. }
            | Expr::Select { .. }
            | Expr::NewObject(_)
            | Expr::StringConstant { .. } => return true,
            _ => return false,
        }
    }
}

/// The index of the builtin type `e` holds an object of exactly: where it is a variable `made` proves one of.
fn exact(e: &Expr, made: &BTreeMap<Var, u32>) -> Option<u32> {
    let mut e = e;
    loop {
        match e {
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::U64 | Scalar::I64,
                e: inner,
            } => e = &**inner,
            Expr::Var(var) => return made.get(var).copied(),
            _ => return None,
        }
    }
}

/// The variables of a body that hold an object of one builtin type and nothing else, by the type's index: every assignment of each makes such an object, and no pointer or wider variable may reach it.
// ⚠ A parameter is handed whatever its callers pass, and a global whatever any program stores:
// neither is one. Every statement that may assign a variable is a simple one, a construct's
// included; one through an address is ruled out by `frame`, and one of a value of several
// cells into a lower local, which runs across the locals after it, by `vars`, but of a local
// `frame` holds alone, which no other variable runs across.
fn made(body: &[Stmt], frame: &Frame, vars: &BTreeMap<Var, VarInfo>) -> BTreeMap<Var, u32> {
    let mut seen: BTreeMap<Var, Option<u32>> = BTreeMap::new();
    tree::each_simple(body, &mut |stmt| {
        let of = match stmt {
            ir::Stmt::Assign {
                value: Expr::NewObject(ty),
                ..
            } if ty.id & BUILTIN != 0 => Some(ty.id & !BUILTIN),
            _ => None,
        };
        for target in stmt.targets() {
            let held = seen.entry(*target).or_insert(of);
            if *held != of {
                *held = None;
            }
        }
    });
    let wide = |var: &Var| vars.get(var).is_some_and(|info| info.width == Width::Cells);
    let lowest_wide = vars
        .keys()
        .filter(|var| wide(var))
        .filter_map(|var| match var {
            Var::Local(n) => Some(*n),
            _ => None,
        })
        .min();
    let own = |var: &Var| {
        let kind = match var {
            Var::Local(n) => frame.alone.contains(n) || lowest_wide.is_none_or(|low| low > *n),
            Var::Value { .. } | Var::Stack { .. } => !wide(var),
            Var::Param(_) | Var::Global { .. } => false,
        };
        kind && !frame.has(*var)
    };
    seen.into_iter()
        .filter(|(var, _)| own(var))
        .filter_map(|(var, of)| Some((var, of?)))
        .collect()
}

/// What a pointer the body takes may reach of its variables.
// ⚠ A local `values::widths` proves one cell wide is one no other variable runs across: the
// address of a local below it, handed to a function or gone through, stops short of it. Only an
// address that names its cell, or runs onto it by a stated count, reaches it. One moved by a
// count nothing states, worked on as a number, handed to an intrinsic or kept anywhere may come
// out anywhere: then no local is held alone.
struct Frame {
    reached: Reached,
    /// The locals proved one cell wide that no address the body takes reaches.
    alone: BTreeSet<u32>,
}

impl Frame {
    fn of(body: &[Stmt], vars: &BTreeMap<Var, VarInfo>) -> Frame {
        Frame {
            reached: Reached::of(body),
            alone: alone(body, vars),
        }
    }

    /// Whether code that is not handed `var` may still read or write it.
    fn has(&self, var: Var) -> bool {
        match var {
            Var::Local(n) if self.alone.contains(&n) => false,
            other => self.reached.has(other),
        }
    }
}

/// The locals proved one cell wide that no address `body` takes reaches: none where an address of a local is used other than in place.
fn alone(body: &[Stmt], vars: &BTreeMap<Var, VarInfo>) -> BTreeSet<u32> {
    let mut alone: BTreeSet<u32> = vars
        .iter()
        .filter_map(|(var, info)| match var {
            Var::Local(n) if info.single && info.width == Width::Cell => Some(*n),
            _ => None,
        })
        .collect();
    if alone.is_empty() {
        return alone;
    }
    let mut taken = 0usize;
    // The first cell and the last of each address of a local used in place.
    let mut placed: Vec<(u32, u32)> = Vec::new();
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::AddrOf {
                var: Var::Local(_), ..
            } => taken += 1,
            Expr::Call { callee, args, .. } if !matches!(callee, Callee::Intrinsic { .. }) => {
                placed.extend(args.iter().filter_map(|arg| span(arg, 0, Some(1))));
            }
            Expr::Load { ty, addr, offset } => {
                placed.extend(span(addr, *offset, Some(ty.bytes())));
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => placed.extend(span(addr, *offset, Some(*bytes))),
            _ => {}
        });
    });
    tree::each_simple(body, &mut |stmt| {
        let at = match stmt {
            ir::Stmt::Store {
                kind, addr, offset, ..
            } => span(addr, *offset, Some(kind.bytes())),
            ir::Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                ..
            } => span(addr, *offset, Some(*bytes)),
            ir::Stmt::StoreF32Slots { addr, offset, .. } => span(addr, *offset, None),
            ir::Stmt::ZeroBytes { addr, bytes } => span(addr, 0, Some(*bytes)),
            ir::Stmt::StepI32 { addr, offset, .. } => span(addr, *offset, Some(4)),
            // An address a statement assigns or keeps is no access in place.
            _ => None,
        };
        placed.extend(at);
    });
    if placed.len() != taken {
        return BTreeSet::new();
    }
    for (first, last) in placed {
        let reached: Vec<u32> = alone.range(first..=last).copied().collect();
        for n in reached {
            alone.remove(&n);
        }
    }
    alone
}

/// The first local cell and the last that `bytes` bytes at `offset` past `addr` run over, where `addr` is a local's address moved by stated counts; with no `bytes`, to the frame's end.
fn span(addr: &Expr, offset: u32, bytes: Option<u32>) -> Option<(u32, u32)> {
    let (first, moved) = in_place(addr)?;
    let from = moved.checked_add(u64::from(offset))?;
    let past = match bytes {
        Some(bytes) => from.saturating_add(u64::from(bytes.max(1)) - 1) / 8,
        None => u64::from(u32::MAX),
    };
    let last = u64::from(first).saturating_add(past);
    Some((first, u32::try_from(last).unwrap_or(u32::MAX)))
}

/// The cell a local's address names and the bytes it is then moved by, where every move is by a count stated as a constant.
fn in_place(addr: &Expr) -> Option<(u32, u64)> {
    let mut e = addr;
    let mut moved: u64 = 0;
    loop {
        match e {
            Expr::AddrOf {
                var: Var::Local(n),
                cell,
            } => return Some((n.saturating_add(*cell), moved)),
            Expr::Sugar {
                kind: ExprSugar::Ref | ExprSugar::Out,
                e: inner,
            } => e = &**inner,
            // ⚠ A count with its sign bit set may move the address down the frame.
            Expr::ByteOffset { addr, offset } => match &**offset {
                Expr::Const(c) if c.bits < 0x8000_0000 => {
                    moved = moved.checked_add(c.bits)?;
                    e = &**addr;
                }
                _ => return None,
            },
            _ => return None,
        }
    }
}

/// Groups each object made and the constructor of its own type the next statement runs on it into the one `new` they were.
fn construct(list: &mut Vec<Stmt>, frame: &Frame) {
    let mut out: Vec<Stmt> = Vec::with_capacity(list.len());
    for stmt in list.drain(..) {
        match (out.pop(), stmt) {
            (Some(Stmt::Simple(made)), Stmt::Simple(call)) if constructs(&made, &call, frame) => {
                let parts = vec![made, call];
                let kind = Group::Construct;
                out.push(Stmt::Sugar(Box::new(Sugar::Group { kind, parts })));
            }
            (before, stmt) => {
                out.extend(before);
                out.push(stmt);
            }
        }
    }
    *list = out;
}

/// Whether `made` makes an object of a type of the runtime and `call` runs that type's constructor on it, handed nothing that could see the object before it is made.
// ⚠ `x = new T(a)` evaluates `a` before `x` holds the object: no argument may name the
// variable, and no pointer or other function may reach it.
fn constructs(made: &ir::Stmt, call: &ir::Stmt, frame: &Frame) -> bool {
    let (
        ir::Stmt::Assign {
            target,
            value: Expr::NewObject(ty),
        },
        ir::Stmt::Eval(Expr::Call {
            callee: Callee::Builtin { path, ty: of, .. },
            args,
            returns,
        }),
    ) = (made, call)
    else {
        return false;
    };
    let Some((Expr::Var(receiver), passed)) = args.split_first() else {
        return false;
    };
    let own = ty.id & BUILTIN != 0 && ty.id & !BUILTIN == *of;
    let constructor = table::reading(path, args.len()) == Some(Reading::Constructor);
    let void = returns.is_none_or(|n| n == 0);
    let unseen = !frame.has(*target) && !passed.iter().any(|arg| mentions(arg, *target));
    own && constructor && void && receiver == target && unseen
}

/// Whether `e` reads `var` or takes its address.
fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(node, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

#[cfg(test)]
#[path = "sugar_runtime_tests.rs"]
mod tests;
