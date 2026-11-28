//! Sugar for expressions and simple statements: tests of one value as a `switch`, the null
//! operators and the `is` of an `as` tested for null, compound assignment, string
//! concatenation, initialisers, one inside another, `ref` and `out`, and a box handed on whole
//! as the conversion C# writes in its place.
//!
//! Works on the structured form, last before `names`. An expression is read through an
//! `ExprSugar` and statements through a `Group` or a `Sugar::SwitchOn`, and `tree::desugar` gives
//! back what was there.
//!
//! ⚠ Three rewrites give back statements that run the same and are not the ones read: a
//! `switch` drops the ordering tests its cases were halved by, an initialiser puts the copies
//! and the assignments that stood between its stores around itself, and a choice between a
//! reference and what stands in for null is one assignment where it was an `if` of two.
//!
//! ⚠ An initialiser that holds another holds the inner object's statements among its own, in
//! the order they ran, with no variable of its own in the source: the printer writes them as
//! the field's value, `new A { F = new B { G = a } }`.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{
    self, BinOp, Callee, Expr, ExprSugar, FunctionKey, Head, Kind, Scalar, Var, Width,
};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::project::{reach, signatures};
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Project, Stats};

/// The names this file's rewrites are switched by.
// ⚠ `nullable` and `tuples` read nothing: no table states which type is a nullable or a tuple.
// `ref-out` reads an `out` only where the project's reach table proves the script function
// writes the whole variable before it reads it.
pub const PASSES: &[Pass] = &[
    Pass::SwitchOn,
    Pass::NullOps,
    Pass::Compound,
    Pass::Concat,
    Pass::Initialisers,
    Pass::RefOut,
    Pass::Nullable,
    Pass::Tuples,
    Pass::Boxing,
];

/// The most statements one initialiser is read from: each store is held against every one before it.
const FIELDS: usize = 1024;

/// The most variables one initialiser follows its object through.
const NAMES: usize = 8;

/// The most initialisers one is read inside, one in another.
const DEPTH: usize = 4;

/// The most statements an initialiser inside another is read from.
const INNER: usize = 256;

/// Reads each proved shape as the operator, the initialiser or the `switch` of the source.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let options = cx.options;
    if !options.any(PASSES) {
        return;
    }
    let tree::Function { head, body } = f;
    let head: &Head = head;
    let project = cx.project;
    let reached = match options.on(Pass::Initialisers) {
        true => Reached::of(body),
        false => Reached::default(),
    };
    let named = match options.on(Pass::Initialisers) {
        true => named(body),
        false => BTreeMap::new(),
    };
    let around = Around {
        reached: &reached,
        named: &named,
    };
    let kept = match options.on(Pass::RefOut) {
        true => kept_in(body, head, project),
        false => Reached::default(),
    };
    let nulls = options.on(Pass::NullOps);
    let joins = options.on(Pass::Concat);
    let refs = options.on(Pass::RefOut);
    // An explicit stack: a body nests as deep as its listing does. With each list, whether a
    // `switch` may take the statements after its first test into its arms.
    let mut work: Vec<(&mut Vec<Stmt>, bool)> = vec![(&mut *body, true)];
    while let Some((list, whole)) = work.pop() {
        if options.on(Pass::Initialisers) {
            initialisers(list, &around);
        }
        let mut at = 0;
        while at < list.len() {
            if options.on(Pass::SwitchOn) {
                switch::switched(list, at, whole);
            }
            let Some(stmt) = list.get_mut(at) else {
                break;
            };
            if nulls {
                chosen(stmt, head, project);
            }
            if nulls || joins || refs {
                tree::own_exprs_mut(stmt, &mut |top| {
                    top.walk_mut(&mut |e| {
                        if nulls {
                            null_op(e, head, project);
                            type_tested(e);
                        }
                        if joins {
                            joined(e, head, project);
                        }
                        if refs {
                            referred(e, head, project, &kept);
                        }
                    });
                });
            }
            if options.on(Pass::Compound) {
                compounded(stmt, head, project);
            }
            at += 1;
        }
        // A list a construct holds by its role keeps its statements as they are.
        if refs && whole {
            outs(list);
        }
        for stmt in list {
            // ⚠ A list a construct holds by its role keeps its last statements where they are:
            // only the lists it runs as bodies, which it holds first, may end in a `switch`.
            let bodies = match &*stmt {
                Stmt::Sugar(sugar) => match &**sugar {
                    Sugar::SwitchOn { .. } | Sugar::Try { .. } => usize::MAX,
                    Sugar::Foreach { .. } | Sugar::Guarded { .. } => 1,
                    _ => 0,
                },
                _ => usize::MAX,
            };
            let nested = tree::nested_mut(stmt).into_iter().enumerate();
            work.extend(nested.map(|(index, list)| (list, index < bodies)));
        }
    }
    // Last, so that no reading above meets a box as anything but the call it is.
    if options.on(Pass::Boxing) {
        boxes::run(body, head, project, cx.names);
    }
}

/// How many of each construct this file makes a body holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Made {
    /// `place op= value`.
    pub compound: usize,
    /// `new T { F = a }`: an object made and at least one field of it stored.
    pub objects: usize,
    /// `new T[] { a, b }`.
    pub arrays: usize,
    /// A chain of tests read as a `switch`.
    pub switches: usize,
    /// `a ?? b`.
    pub coalesce: usize,
    /// `a?.b`.
    pub null_conditional: usize,
    /// `x is T`.
    pub type_tests: usize,
}

/// Counts the constructs of the kinds this file makes in `body`, at every depth.
pub fn made(body: &[Stmt]) -> Made {
    let mut made = Made::default();
    let mut open = vec![body];
    while let Some(list) = open.pop() {
        for stmt in list {
            match stmt {
                Stmt::Sugar(sugar) => {
                    match &**sugar {
                        Sugar::Group { kind, parts } => match kind {
                            Group::Compound => made.compound += 1,
                            Group::Collection => made.arrays += 1,
                            Group::Construct => {
                                let stored = |s: &ir::Stmt| {
                                    matches!(
                                        s,
                                        ir::Stmt::Store { .. } | ir::Stmt::StoreBytes { .. }
                                    )
                                };
                                made.objects += usize::from(parts.iter().any(stored));
                            }
                            _ => {}
                        },
                        Sugar::SwitchOn { .. } => made.switches += 1,
                        _ => {}
                    }
                    open.extend(sugar.parts().bodies);
                }
                Stmt::If { then, els, .. } => open.extend([then.as_slice(), els.as_slice()]),
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    open.push(body);
                }
                Stmt::Switch { cases, .. } => {
                    open.extend(cases.iter().map(|case| case.body.as_slice()));
                }
                _ => {}
            }
        }
    }
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::Sugar {
                kind: ExprSugar::Coalesce,
                ..
            } => made.coalesce += 1,
            Expr::Sugar {
                kind: ExprSugar::NullConditional,
                ..
            } => made.null_conditional += 1,
            Expr::Sugar {
                kind: ExprSugar::Is,
                ..
            } => made.type_tests += 1,
            _ => {}
        });
    });
    made
}

fn group(kind: Group, parts: Vec<ir::Stmt>) -> Stmt {
    Stmt::Sugar(Box::new(Sugar::Group { kind, parts }))
}

/// `e` under the source readings around it.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// `e` under the 64-bit readings around it, which change no bit of a cell.
fn whole(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// Whether `e` is a constant cell with no bit set.
fn is_null(e: &Expr) -> bool {
    matches!(whole(e), Expr::Const(c) if c.bits == 0)
}

/// Whether `a` and `b` are one expression, whatever source readings either carries.
fn same(a: &Expr, b: &Expr) -> bool {
    if a == b {
        return true;
    }
    let (mut a, mut b) = (a.clone(), b.clone());
    a.desugar();
    b.desugar();
    a == b
}

/// What evaluating `e` depends on: 0 variables and constants alone, 1 memory as well, 2 a call or an allocation.
fn level(e: &Expr) -> u8 {
    let mut level = 0;
    e.walk(&mut |node| {
        let own = match node {
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. } => 2,
            Expr::Load { .. }
            | Expr::LoadBytes { .. }
            | Expr::VirtualFunctionRef { .. }
            | Expr::BuiltinGlobal(_)
            | Expr::Var(Var::Global { .. }) => 1,
            _ => 0,
        };
        level = level.max(own);
    });
    level
}

/// Whether `e` reads `var` or takes its address.
fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(node, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

/// The bytes a value of a scalar kind takes.
fn bytes_of(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::I8 | Kind::U8 => Some(1),
        Kind::I16 | Kind::U16 => Some(2),
        Kind::I32 | Kind::U32 | Kind::F32 | Kind::Hash => Some(4),
        Kind::I64
        | Kind::U64
        | Kind::F64
        | Kind::Pointer
        | Kind::Object(_)
        | Kind::String
        | Kind::Array(_) => Some(8),
        Kind::Unknown
        | Kind::Bool
        | Kind::Vector
        | Kind::Handle
        | Kind::Aggregate(_)
        | Kind::Value { .. } => None,
    }
}

/// The bytes an integer of this kind is worked on at; a kind nothing proves is a whole cell.
fn integer_bytes(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::Unknown => Some(8),
        kind if kind.is_float() => None,
        kind => bytes_of(kind),
    }
}

/// The operator and the right operand of a statement that works on the place it writes: `place = place op r`.
// ⚠ Only where `place op= r` says the same in C#: the left operand is the place read whole, under
// no reading but the operator's own, and a division, a remainder or a right shift works at the
// place's own sign. The place is never looked for on the right: `x = e - x` is not `x -= e`.
// `owner` is the type that declares the field a store writes, where a pass proved one.
pub(crate) fn compound<'a>(
    s: &'a ir::Stmt,
    head: &Head,
    project: &Project,
    owner: Option<ir::TypeKey>,
) -> Option<(BinOp, &'a Expr)> {
    let (ir::Stmt::Assign { value, .. } | ir::Stmt::Store { value, .. }) = s else {
        return None;
    };
    let Expr::Binary { op, at, l, r } = value else {
        return None;
    };
    let signless = matches!(
        op,
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Shl
    );
    if !signless && !matches!(op, BinOp::Div | BinOp::Rem | BinOp::Shr) {
        return None;
    }
    let read = match bare(l) {
        Expr::Cast { to, e } if to.kind() == *at => bare(e),
        other => other,
    };
    let holds = match (s, read) {
        (ir::Stmt::Assign { target, .. }, Expr::Var(var)) if var == target => {
            let declared = (head.vars.get(target)).map_or(&Kind::Unknown, |info| &info.kind);
            // ⚠ Wider than the operator works, and a variable of no proved kind is a whole cell,
            // it would keep the bits the statement cuts.
            let same = integer_bytes(at).is_some() && integer_bytes(declared) == integer_bytes(at);
            declared == at || (signless && same)
        }
        (
            ir::Stmt::Store {
                kind, addr, offset, ..
            },
            Expr::Load {
                ty,
                addr: from,
                offset: read_at,
            },
        ) => {
            let field = owner.and_then(|owner| project.field(owner, *offset));
            let fits = match (field.map(|field| &field.kind), kind.bytes()) {
                // ⚠ Through a pointer of no proved type the place says no width: `op=` there
                // reads as the whole cell, and would drop the reading a narrower store states.
                (None, 8) => integer_bytes(at) == Some(8),
                (None, _) => false,
                (Some(declared), _) if *declared != ty.kind() => false,
                // A byte or two are widened to an `int` and the store cuts the result back.
                (Some(_), 1 | 2) => *at == Kind::I32,
                (Some(declared), _) => {
                    let same =
                        integer_bytes(at).is_some() && integer_bytes(declared) == integer_bytes(at);
                    declared == at || (signless && same)
                }
            };
            let exact = signless || at.is_float() || ty.kind() == *at || ty.bytes() < 4;
            // ⚠ The source evaluates the address once: twice, it must be the same address.
            let once = match level(addr) {
                0 => true,
                1 => level(r) < 2,
                _ => false,
            };
            let place = ty.bytes() == kind.bytes() && read_at == offset;
            place && fits && exact && once && same(from, addr)
        }
        _ => false,
    };
    holds.then_some((*op, &**r))
}

/// Whether the printer writes `s` as `x++` or `x--` already: one up or down in 32 bits.
fn steps(s: &ir::Stmt) -> bool {
    let ir::Stmt::Assign {
        value:
            Expr::Binary {
                op: BinOp::Add | BinOp::Sub,
                at: Kind::I32,
                r,
                ..
            },
        ..
    } = s
    else {
        return false;
    };
    matches!(&**r, Expr::Const(c) if c.bits == 1)
}

/// Reads a statement that works on the place it writes as a compound assignment; a store to a field keeps its field in the place its value reads.
fn compounded(stmt: &mut Stmt, head: &Head, project: &Project) {
    match stmt {
        Stmt::Simple(s) => {
            if compound(s, head, project, None).is_some() && !steps(s) {
                let held = std::mem::replace(s, ir::Stmt::Eval(Expr::GuidEmpty));
                *stmt = group(Group::Compound, vec![held]);
            }
        }
        Stmt::Sugar(sugar) => {
            if let Sugar::Group { kind, parts } = &mut **sugar {
                let Group::Field { owner } = *kind else {
                    return;
                };
                let works = |s: &ir::Stmt| compound(s, head, project, Some(owner)).is_some();
                if matches!(parts.as_slice(), [s] if works(s)) {
                    *kind = Group::Compound;
                }
            }
        }
        _ => {}
    }
}

/// What `cond` tests for not being null: `x` of `x != 0`, compared as a whole cell.
fn not_null(cond: &Expr) -> Option<&Expr> {
    match bare(cond) {
        Expr::Binary {
            op: BinOp::Ne,
            at: Kind::I64 | Kind::U64 | Kind::Unknown,
            l,
            r,
        } if is_null(r) => Some(whole(l)),
        _ => None,
    }
}

/// The kind a variable, a global or a field read as its type's is stated to hold.
fn stated_kind<'a>(e: &Expr, head: &'a Head, project: &'a Project) -> Option<&'a Kind> {
    match e {
        Expr::Var(Var::Global {
            program,
            offset,
            statics,
        }) => project
            .programs
            .get(program)
            .and_then(|info| match *statics {
                true => info.statics.get(offset),
                false => info.globals.get(offset),
            }),
        Expr::Var(var) => head.vars.get(var).map(|info| &info.kind),
        Expr::Sugar {
            kind: ExprSugar::Field { owner },
            e,
        } => match &**e {
            Expr::Load { offset, .. } => project.field(*owner, *offset).map(|field| &field.kind),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `e` is proved to hold a reference: a variable, a global or a field of such a kind.
fn reference(e: &Expr, head: &Head, project: &Project) -> bool {
    // A global the container marks and a field a reference was stored to hold references, though their kind says no more than an address.
    let stated = !matches!(
        e,
        Expr::Var(Var::Param(_) | Var::Local(_) | Var::Value { .. } | Var::Stack { .. })
    );
    match stated_kind(e, head, project) {
        Some(Kind::Object(_) | Kind::String | Kind::Array(_)) => true,
        Some(Kind::Pointer) => stated,
        _ => false,
    }
}

/// Whether `e` is a variable of one cell that nothing proves a number, a truth or a value of several cells: what it holds may be a reference.
fn open(e: &Expr, head: &Head, project: &Project) -> bool {
    let Expr::Var(var) = e else {
        return false;
    };
    let one = (head.vars.get(var)).is_none_or(|info| info.width == Width::Cell);
    let kind = stated_kind(e, head, project);
    one && matches!(kind, None | Some(Kind::Unknown | Kind::Pointer))
}

/// What a choice reads as: `x != null ? x : e` as `x ?? e`, and `x != null ? x.f : null` as `x?.f`.
// ⚠ Only over references: a number that is not zero is no `??`, and `x?.f` of a number would be
// a nullable in C#. `x` is written twice and evaluated twice, so it holds no call. A variable no
// kind is proved for is a reference where what stands in for it is one: a choice is one value
// of one type in C#, and neither a number nor a pointer becomes a reference without a box.
fn null_reading(e: &Expr, head: &Head, project: &Project) -> Option<ExprSugar> {
    let Expr::Select { cond, then, els } = e else {
        return None;
    };
    let tested = not_null(cond)?;
    let then = whole(then);
    let of_tested =
        |field: &Expr| matches!(bare(field), Expr::Load { addr, .. } if same(whole(addr), tested));
    let held = reference(tested, head, project)
        || (open(tested, head, project) && reference(whole(els), head, project));
    // `x ?? null` is `x`: a statement nobody writes, so such a choice keeps its plain form.
    let kind = if held && same(then, tested) && !is_null(els) {
        ExprSugar::Coalesce
    } else if is_null(els) && reference(then, head, project) && of_tested(then) {
        ExprSugar::NullConditional
    } else {
        return None;
    };
    (level(tested) <= 1).then_some(kind)
}

/// Reads a choice as the null operator it is.
fn null_op(e: &mut Expr, head: &Head, project: &Project) {
    if let Some(kind) = null_reading(e, head, project) {
        let held = std::mem::replace(e, Expr::GuidEmpty);
        *e = Expr::sugar(kind, held);
    }
}

/// The test `e` is of what `x as T` makes against null, compared whole: whether it holds where that is not null, the type cast to, and `x`.
// ⚠ An `as` yields its value where the value is a `T` and null where it is not, so its test for
// null is `is` exactly; a test of fewer bits than the cell would not be.
pub(crate) fn type_test(e: &Expr) -> Option<(bool, &str, &Expr)> {
    let Expr::Binary {
        op,
        at: Kind::I64 | Kind::U64 | Kind::Unknown,
        l,
        r,
    } = e
    else {
        return None;
    };
    let holds = match op {
        BinOp::Ne => true,
        BinOp::Eq => false,
        _ => return None,
    };
    let cast = match (is_null(l), is_null(r)) {
        (false, true) => l,
        (true, false) => r,
        _ => return None,
    };
    let Expr::Call {
        callee: Callee::Intrinsic { name, .. },
        args,
        ..
    } = whole(cast)
    else {
        return None;
    };
    let ty = name.strip_prefix("try_cast<")?.strip_suffix('>')?;
    match args.as_slice() {
        [value] if !ty.is_empty() => Some((holds, ty, value)),
        _ => None,
    }
}

/// Reads a test of what an `as` makes against null as the `is` it is.
fn type_tested(e: &mut Expr) {
    if type_test(e).is_some() {
        let held = std::mem::replace(e, Expr::GuidEmpty);
        *e = Expr::sugar(ExprSugar::Is, held);
    }
}

/// Makes `if (x != null) { s = x; } else { s = e; }` the one assignment `s = x != null ? x : e` where that reads as a null operator.
// ⚠ Any other `if` stays an `if`. The choice evaluates what the `if` did, in its order: the
// test, then the one value. An `if` on `x == null` chooses the other way round.
fn chosen(stmt: &mut Stmt, head: &Head, project: &Project) {
    let made = {
        let Stmt::If { cond, then, els } = &*stmt else {
            return;
        };
        let (
            [Stmt::Simple(ir::Stmt::Assign {
                target,
                value: held,
            })],
            [Stmt::Simple(ir::Stmt::Assign {
                target: other,
                value: unheld,
            })],
        ) = (then.as_slice(), els.as_slice())
        else {
            return;
        };
        if target != other {
            return;
        }
        let (test, kept, lost) = match cond {
            Expr::Binary { op: BinOp::Ne, .. } => (cond.clone(), held, unheld),
            Expr::Binary {
                op: BinOp::Eq,
                at,
                l,
                r,
            } => {
                let test = Expr::Binary {
                    op: BinOp::Ne,
                    at: at.clone(),
                    l: l.clone(),
                    r: r.clone(),
                };
                (test, unheld, held)
            }
            _ => return,
        };
        let choice = Expr::Select {
            cond: Box::new(test),
            then: Box::new(kept.clone()),
            els: Box::new(lost.clone()),
        };
        if null_reading(&choice, head, project).is_none() {
            return;
        }
        ir::Stmt::Assign {
            target: *target,
            value: choice,
        }
    };
    *stmt = Stmt::Simple(made);
}

/// Reads a call of the builtin the user's list names `String.Concat` as `a + b`.
// ⚠ By that name alone, and only where one of the first two pieces is proved a string: between
// two numbers `+` would add.
fn joined(e: &mut Expr, head: &Head, project: &Project) {
    let Expr::Call {
        callee: Callee::Builtin { path, .. },
        args,
        ..
    } = &*e
    else {
        return;
    };
    let mut name = path.rsplit('.');
    let named = name.next() == Some("Concat") && name.next() == Some("String");
    let text = |arg: &Expr| match whole(arg) {
        Expr::StringConstant { .. }
        | Expr::Sugar {
            kind: ExprSugar::Concat,
            ..
        } => true,
        other => stated_kind(other, head, project) == Some(&Kind::String),
    };
    if named && args.len() >= 2 && args.iter().take(2).any(text) {
        let held = std::mem::replace(e, Expr::GuidEmpty);
        *e = Expr::sugar(ExprSugar::Concat, held);
    }
}

/// Whether the address of `var` reaches no variable but `var`: a `ref` says as much of what it is handed.
// A temporary is storage of its own. A local or a parameter is a run of frame cells, and its
// address reaches the cells after it up to the next variable proved to start one: the local
// itself where it is proved one cell wide, else the first cell past it, or the frame's end.
fn bounded(var: Var, head: &Head) -> bool {
    let info = head.vars.get(&var);
    let cells = match info.map(|info| (&info.width, &info.kind)) {
        None | Some((Width::Cell, _)) => 1,
        Some((Width::Cells, Kind::Aggregate(cells) | Kind::Value { cells, .. })) => (*cells).max(1),
        Some((Width::Cells, Kind::Vector | Kind::Handle)) => 2,
        Some((Width::Cells, _)) => return false,
    };
    let proved = |n: u32| {
        let info = head.vars.get(&Var::Local(n));
        info.is_some_and(|info| info.single && info.width == Width::Cell)
    };
    match var {
        Var::Local(n) => {
            let past = n.saturating_add(cells);
            (cells == 1 && proved(n)) || past >= head.locals || proved(past)
        }
        Var::Param(n) => n.saturating_add(cells) >= head.params,
        Var::Global { .. } => false,
        Var::Value { .. } | Var::Stack { .. } => true,
    }
}

/// The bytes a variable is known to hold: its cells, where a kind states them.
fn held_bytes(var: Var, head: &Head) -> Option<u32> {
    if matches!(var, Var::Global { .. }) {
        return None;
    }
    let info = head.vars.get(&var);
    let cells = match info.map(|info| (&info.width, &info.kind)) {
        None | Some((Width::Cell, _)) => 1,
        Some((Width::Cells, Kind::Aggregate(cells) | Kind::Value { cells, .. })) => (*cells).max(1),
        Some((Width::Cells, Kind::Vector | Kind::Handle)) => 2,
        Some((Width::Cells, _)) => return None,
    };
    cells.checked_mul(8)
}

/// What a script function is proved to do through the address of a variable it is handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handed {
    /// It touches no byte past the variable and writes its cells before it reads any.
    Out,
    /// It touches no byte past the variable.
    Within,
    /// It may touch bytes past the variable.
    Past,
    /// The project does not say.
    Unknown,
}

/// What the function `key` does through the address of `var` handed to its parameter cell `cell`, by the project's reach table.
// ⚠ A reach is of what goes through the one cell, in the function's own code and what it hands
// the cell to, never of its callers; where the address may be kept, or the bytes reached are
// unbounded (handed to a native, a slot or a function not read), nothing is said.
fn handed(project: &Project, key: FunctionKey, cell: u32, var: Var, head: &Head) -> Handed {
    let (Some(bytes), Some(held)) = (project.reach(key, cell), held_bytes(var, head)) else {
        return Handed::Unknown;
    };
    if bytes == reach::UNBOUNDED || project.address_dies(key, cell) != Some(true) {
        return Handed::Unknown;
    }
    if bytes > held {
        return Handed::Past;
    }
    // Every byte of the variable written on every path, and none read before it is written.
    let out = project.param_mode(key, cell) == Some(reach::Mode::Out)
        && project
            .param_covered(key, cell)
            .is_some_and(|covered| covered >= held);
    match out {
        true => Handed::Out,
        false => Handed::Within,
    }
}

/// The parameter cell each argument of a call lands in, the cells of the arguments before it, and the cells of them all, where they are known.
fn landing(args: &[Expr], head: &Head) -> (Vec<Option<u32>>, Option<u32>) {
    let mut next = Some(0u32);
    let mut cells = Vec::with_capacity(args.len());
    for arg in args {
        cells.push(next);
        next = (next.zip(cells_of(arg, head))).and_then(|(a, b)| a.checked_add(b));
    }
    (cells, next)
}

/// What each argument of a call to the script function `key` is proved to be, where it is the address of a variable.
fn proved_args(
    key: Option<FunctionKey>,
    args: &[Expr],
    head: &Head,
    project: &Project,
) -> Vec<Option<(Var, Handed)>> {
    let (cells, all) = landing(args, head);
    // ⚠ The cells handed must be the cells the function takes, or an argument lands elsewhere.
    let takes = |key: &FunctionKey| {
        let signature = project.signatures.get(key);
        let params = signature.map(|held| held.params.iter().map(|p| p.cells).sum::<u32>());
        params.is_some() && params == all
    };
    let key = key.filter(takes);
    let each = args.iter().zip(cells).map(|(arg, at)| match arg {
        Expr::AddrOf { var, cell: 0 } => Some(match key.zip(at) {
            Some((key, cell)) => (*var, handed(project, key, cell, *var, head)),
            None => (*var, Handed::Unknown),
        }),
        _ => None,
    });
    each.collect()
}

/// Whether the cells of two variables of the frame may be the same cells.
fn overlaps(a: Var, b: Var, head: &Head) -> bool {
    let span = |var: Var| held_bytes(var, head).map(|bytes| bytes / 8);
    match (a, b) {
        (Var::Local(x), Var::Local(y)) | (Var::Param(x), Var::Param(y)) => {
            match (span(a), span(b)) {
                (Some(of_a), Some(of_b)) => {
                    x < y.saturating_add(of_b) && y < x.saturating_add(of_a)
                }
                _ => true,
            }
        }
        _ => a == b,
    }
}

/// Whether `e` takes the address of any variable.
fn addresses(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |node| found |= matches!(node, Expr::AddrOf { .. }));
    found
}

/// Whether no argument but the one at `index` may reach the cells of `var`: each takes no address, or is the address of another variable the callee touches nothing past.
fn apart(
    index: usize,
    var: Var,
    args: &[Expr],
    proved: &[Option<(Var, Handed)>],
    head: &Head,
) -> bool {
    args.iter().enumerate().all(|(other, arg)| {
        if other == index {
            return true;
        }
        match proved.get(other).copied().flatten() {
            Some((held, Handed::Out | Handed::Within)) => !overlaps(var, held, head),
            Some(_) => false,
            None => !addresses(arg),
        }
    })
}

/// What the first cell handed to the script function `key` is: the first argument that is no receiver, and whether that argument is the value a struct's method works on.
// A function of no type, or a static one, takes no receiver. A struct's method takes the address
// of its value, `ref this`, which a frame variable's address is; a class's object never is one.
// Any other member's first cell may be the receiver the classes pass writes before the dot.
fn receiver(project: &Project, key: FunctionKey) -> (usize, bool) {
    let Some(member) = project.members.get(&key) else {
        return (0, false);
    };
    let owner = project.types.get(&member.owner);
    let method = matches!(member.role, Role::Method);
    match member.is_static {
        true => (0, false),
        false if method && owner.is_some_and(|facts| facts.is_value()) => (0, true),
        false => (1, false),
    }
}

/// Reads the address of a variable handed to a call as `ref`, where it reaches that variable alone, and as `out` where the script function it is handed to writes it whole before it reads it.
// ⚠ Never the first cell of a call that may be made on it: a receiver is written before the dot.
// Only a native, and a script function proved to take no receiver or to be a struct's method, are
// handed no receiver; a struct's value is `ref this` and never an `out`. An `out` also needs that
// nothing else the callee can reach holds the variable's address: no other argument, and no
// address of it, or of a cell before it, taken elsewhere and kept.
fn referred(e: &mut Expr, head: &Head, project: &Project, kept: &Reached) {
    match e {
        Expr::Call { callee, args, .. } => {
            let (first, this) = match callee {
                Callee::Intrinsic { .. } => return,
                Callee::Native { .. } => (0, false),
                Callee::Local { at, .. } => receiver(project, FunctionKey::new(head.program, *at)),
                Callee::Import { program, at, .. } => {
                    receiver(project, FunctionKey::new(*program, *at))
                }
                Callee::Builtin { .. } | Callee::Virtual { .. } => (1, false),
            };
            let key = signatures::key_of(head.program, callee);
            let all: &[Expr] = args.as_slice();
            let proved = proved_args(key, all, head, project);
            let kinds: Vec<Option<ExprSugar>> = (proved.iter().enumerate())
                .map(|(index, found)| {
                    let (var, handed) = (*found)?;
                    if index < first {
                        return None;
                    }
                    let alone = || {
                        let own = !(this && index == 0);
                        own && !kept.has(var) && apart(index, var, all, &proved, head)
                    };
                    match handed {
                        Handed::Out if alone() => Some(ExprSugar::Out),
                        Handed::Out | Handed::Within => Some(ExprSugar::Ref),
                        Handed::Unknown if bounded(var, head) => Some(ExprSugar::Ref),
                        Handed::Past | Handed::Unknown => None,
                    }
                })
                .collect();
            for (arg, kind) in args.iter_mut().zip(kinds) {
                if let Some(kind) = kind {
                    let held = std::mem::replace(arg, Expr::GuidEmpty);
                    *arg = Expr::sugar(kind, held);
                }
            }
        }
        // A call read as a method or a property is written on its first argument, and one read
        // as an operator or a conversion with no call at all.
        Expr::Sugar { kind, e: inner } => {
            let Expr::Call { args, .. } = &mut **inner else {
                return;
            };
            let plain = match kind {
                ExprSugar::Method | ExprSugar::Property => 1,
                _ => args.len(),
            };
            for arg in args.iter_mut().take(plain) {
                if let Expr::Sugar {
                    kind: ExprSugar::Ref | ExprSugar::Out,
                    e: held,
                } = arg
                {
                    let held = std::mem::replace(&mut **held, Expr::GuidEmpty);
                    *arg = held;
                }
            }
        }
        _ => {}
    }
}

/// Whether `first` makes a variable zero and `next` is a call that hands it whole as an `out`, naming it nowhere else.
// ⚠ Only where the call is the statement's own value: one inside a choice may not be made, and
// then the zero is what stays. Another argument that read the variable would read the zero.
fn zeroed_out(first: &Stmt, next: &Stmt) -> bool {
    let (Stmt::Simple(ir::Stmt::Assign { target, value }), Stmt::Simple(call)) = (first, next)
    else {
        return false;
    };
    let zero = is_null(value) || matches!(bare(value), Expr::ZeroSlots(_));
    let top = match call {
        ir::Stmt::Eval(e) => e,
        ir::Stmt::Assign { target: to, value } if to != target => value,
        _ => return false,
    };
    let Expr::Call { args, .. } = bare(top) else {
        return false;
    };
    let handed = Expr::AddrOf {
        var: *target,
        cell: 0,
    };
    let out = args
        .iter()
        .any(|arg| matches!(arg, Expr::Sugar { kind: ExprSugar::Out, e } if **e == handed));
    zero && out && times(*target, [call]) == 1
}

/// Reads a variable made zero and then handed as an `out` by the next statement as the one statement `F(out var x)`.
// The zero is what a callee that writes the whole variable before it reads any never sees.
fn outs(list: &mut Vec<Stmt>) {
    if list.len() < 2 {
        return;
    }
    let mut out = Vec::with_capacity(list.len());
    let mut stmts = std::mem::take(list).into_iter().peekable();
    while let Some(stmt) = stmts.next() {
        let paired = stmts.peek().is_some_and(|next| zeroed_out(&stmt, next));
        match (paired, stmt) {
            (true, Stmt::Simple(zero)) => match stmts.next() {
                Some(Stmt::Simple(call)) => out.push(group(Group::Out, vec![zero, call])),
                Some(other) => {
                    out.push(Stmt::Simple(zero));
                    out.push(other);
                }
                None => out.push(Stmt::Simple(zero)),
            },
            (_, stmt) => out.push(stmt),
        }
    }
    *list = out;
}

/// The constant `e` is, as a comparison of whole cells sees it when `wide` and one of 32 bits otherwise.
fn constant(e: &Expr, wide: bool) -> Option<u64> {
    let mut e = e;
    loop {
        match e {
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => e = &**inner,
            // A 32-bit reading keeps the 32 bits a 32-bit comparison sees.
            Expr::Cast {
                to: Scalar::I32 | Scalar::U32,
                e: inner,
            } if !wide => e = &**inner,
            Expr::Const(c) if !c.kind.is_float() => {
                return Some(match wide {
                    true => c.bits,
                    false => u64::from(c.bits as u32),
                });
            }
            _ => return None,
        }
    }
}

/// The variables a pointer may reach.
#[derive(Default)]
pub(super) struct Reached {
    /// The temporaries and globals whose address is taken.
    named: BTreeSet<Var>,
    /// The lowest local whose address is taken: it reaches every local after it too.
    local: Option<u32>,
    /// The same of the parameters.
    param: Option<u32>,
}

impl Reached {
    pub(super) fn of(body: &[Stmt]) -> Reached {
        let mut reached = Reached::default();
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| {
                if let Expr::AddrOf { var, .. } = e {
                    reached.add(*var);
                }
            });
        });
        reached
    }

    /// Notes that a pointer may reach `var`, and so every local or parameter after it.
    fn add(&mut self, var: Var) {
        match var {
            Var::Local(n) => self.local = Some(self.local.map_or(n, |low| low.min(n))),
            Var::Param(n) => self.param = Some(self.param.map_or(n, |low| low.min(n))),
            other => {
                self.named.insert(other);
            }
        }
    }

    /// Whether code that is not handed `var` may still read it: through a pointer, or by its name where it is a global.
    pub(super) fn has(&self, var: Var) -> bool {
        match var {
            Var::Local(n) => self.local.is_some_and(|low| low <= n),
            Var::Param(n) => self.param.is_some_and(|low| low <= n),
            Var::Global { .. } => true,
            temporary => self.named.contains(&temporary),
        }
    }
}

/// The variables a pointer kept past the statement that took it may reach: every address taken but those handed to a script function that touches nothing past the variable and keeps nothing.
// ⚠ Of the body as the pass finds it; nothing it rewrites takes an address or drops one.
fn kept_in(body: &[Stmt], head: &Head, project: &Project) -> Reached {
    let mut taken: BTreeMap<Var, usize> = BTreeMap::new();
    let mut dying: BTreeMap<Var, usize> = BTreeMap::new();
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::AddrOf { var, .. } => *taken.entry(*var).or_default() += 1,
            Expr::Call { callee, args, .. } => {
                let key = signatures::key_of(head.program, callee);
                let proved = proved_args(key, args, head, project);
                for (var, handed) in proved.into_iter().flatten() {
                    if matches!(handed, Handed::Out | Handed::Within) {
                        *dying.entry(var).or_default() += 1;
                    }
                }
            }
            _ => {}
        });
    });
    let mut kept = Reached::default();
    for (var, times) in taken {
        if dying.get(&var).copied().unwrap_or(0) < times {
            kept.add(var);
        }
    }
    kept
}

/// Every variable a body assigns, reads or takes the address of, with the times it does.
fn named(body: &[Stmt]) -> BTreeMap<Var, usize> {
    let mut named: BTreeMap<Var, usize> = BTreeMap::new();
    tree::each_simple(body, &mut |s| {
        for target in s.targets() {
            *named.entry(*target).or_default() += 1;
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                *named.entry(*var).or_default() += 1;
            }
        });
    });
    named
}

/// The times the statements name `var`: assigned, read or its address taken.
fn times<'a>(var: Var, stmts: impl IntoIterator<Item = &'a ir::Stmt>) -> usize {
    let mut times = 0;
    for s in stmts {
        times += s.targets().iter().filter(|target| **target == var).count();
        s.each_expr(&mut |top| {
            top.walk(&mut |e| {
                let named = matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
                times += usize::from(named);
            });
        });
    }
    times
}

/// What an initialiser is read against.
struct Around<'a> {
    reached: &'a Reached,
    /// Every variable the function names, with the times it does.
    named: &'a BTreeMap<Var, usize>,
}

/// What a statement makes that the statements after it may fill.
enum Fresh {
    Object,
    /// An array of this many elements.
    Array(u64),
}

/// The one simple statement `stmt` is, a store read as a field's included.
fn simple(stmt: &Stmt) -> Option<&ir::Stmt> {
    match stmt {
        Stmt::Simple(s) => Some(s),
        Stmt::Sugar(sugar) => match &**sugar {
            Sugar::Group {
                kind: Group::Field { .. },
                parts,
            } => match parts.as_slice() {
                [s] => Some(s),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// The variable a statement gives a new object or array to, what it makes, and the statements it is: the assignment, or the group a pass before put it in with its constructor.
fn opening(stmt: &Stmt) -> Option<(Var, Fresh, &[ir::Stmt])> {
    let parts = match stmt {
        Stmt::Simple(s) => std::slice::from_ref(s),
        Stmt::Sugar(sugar) => match &**sugar {
            Sugar::Group {
                kind: Group::Construct,
                parts,
            } => parts.as_slice(),
            _ => return None,
        },
        _ => return None,
    };
    let ir::Stmt::Assign { target, value } = parts.first()? else {
        return None;
    };
    let fresh = match bare(value) {
        Expr::NewObject(_) => Fresh::Object,
        Expr::NewArray { len, .. } if parts.len() == 1 => match &**len {
            Expr::Const(c) if c.bits >= 1 => Fresh::Array(c.bits),
            _ => return None,
        },
        _ => return None,
    };
    Some((*target, fresh, parts))
}

/// The bytes of the object a statement stores to through one of `names`, first and one past the last, what it stores, and the name it stores through.
// An object's first eight bytes are its type: a store there is no field's.
fn field_store<'a>(s: &'a ir::Stmt, names: &[Var]) -> Option<((u32, u32), &'a Expr, Var)> {
    let (addr, offset, bytes, value) = match s {
        ir::Stmt::Store {
            kind,
            addr,
            offset,
            value,
        } => (addr, *offset, kind.bytes(), value),
        ir::Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            value,
        } => (addr, *offset, *bytes, value),
        _ => return None,
    };
    let end = offset.checked_add(bytes)?;
    let Expr::Var(through) = bare(whole(addr)) else {
        return None;
    };
    (names.contains(through) && offset >= 8).then_some(((offset, end), value, *through))
}

/// The cells a statement sets element `index` of the array in `array` to.
fn element_store(s: &ir::Stmt, array: Var, index: u64) -> Option<&[Expr]> {
    let ir::Stmt::Eval(set) = s else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Intrinsic { name, .. },
        args,
        ..
    } = bare(set)
    else {
        return None;
    };
    let [to, at, cells @ ..] = args.as_slice() else {
        return None;
    };
    let own = matches!(bare(whole(to)), Expr::Var(var) if *var == array);
    let here = matches!(whole(at), Expr::Const(c) if c.bits == index);
    (name.starts_with("array_set_") && own && here && !cells.is_empty()).then_some(cells)
}

/// What expressions read, and the most any of them depends on.
#[derive(Default)]
struct Reads {
    vars: BTreeSet<Var>,
    level: u8,
}

impl Reads {
    fn add(&mut self, e: &Expr) {
        self.level = self.level.max(level(e));
        e.walk(&mut |node| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = node {
                self.vars.insert(*var);
            }
        });
    }
}

/// What a statement that stands before the last store of an initialiser is to it.
enum Between {
    /// A store to the object: a field of the initialiser.
    Field,
    /// A copy of the reference to another variable: it goes after the initialiser.
    Copy,
    /// An assignment that cannot tell where it stands: it goes before the object is made.
    Apart,
    /// The first statement of an object or array made inside this one: the statements it is, in order.
    Inner(Vec<ir::Stmt>),
    /// A later statement of one made inside this one.
    Held,
}

/// The initialiser the statements at the head of a list read as.
struct Initialised {
    /// The assignments that stood between its statements and run before it.
    before: Vec<Stmt>,
    made: Sugar,
    /// The copies of the reference that stood between its statements and run after it.
    after: Vec<Stmt>,
    /// The statements of the list all of this stands for.
    held: usize,
}

/// Whether `e` makes an object or an array.
fn makes(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(node, Expr::NewObject(_) | Expr::NewArray { .. });
    });
    found
}

/// The initialiser of the object `opened` makes in `target`: every store to it among the statements of `rest` that come before its first other use.
// ⚠ The source assigns the variable after the last store and evaluates each value in turn. So
// no value may be able to tell, by a name of the object or through a pointer to one; a copy of
// the reference is moved to after the last store, and a store through the copy is a store to
// the object; any other assignment is moved to before the object is made, and only where
// neither it nor a statement it passes can tell. A call is never moved past a store or a
// call, and no making of an object past another.
fn object(
    target: Var,
    opened: &[ir::Stmt],
    rest: &[Stmt],
    around: &Around<'_>,
    depth: usize,
) -> Option<Initialised> {
    let reached = around.reached;
    let mut names = vec![target];
    let mut filled: Vec<(u32, u32)> = (opened.iter())
        .filter_map(|s| field_store(s, &names))
        .map(|(bytes, _, _)| bytes)
        .collect();
    // What the statements an assignment would be moved past read: the constructor's and the stores'.
    let mut passed = Reads::default();
    for s in opened.iter().skip(1) {
        s.each_expr(&mut |e| passed.add(e));
    }
    let blind = |value: &Expr, names: &[Var]| blind_to(value, names, reached);
    let exposed = |value: &Expr| {
        let mut found = false;
        value.walk(&mut |node| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = node {
                found |= reached.has(*var);
            }
        });
        found
    };
    let mut roles = Vec::new();
    let mut last = None;
    // The statements of one made inside this one still to pass.
    let mut skip = 0usize;
    for (index, stmt) in rest.iter().take(FIELDS).enumerate() {
        if skip > 0 {
            skip -= 1;
            roles.push(Between::Held);
            continue;
        }
        let tail = rest.get(index..).unwrap_or_default();
        if let Some(inner) = nested(tail, &names, &filled, around, depth) {
            for s in &inner.parts {
                s.each_expr(&mut |e| passed.add(e));
            }
            skip = inner.held.saturating_sub(1);
            roles.push(Between::Inner(inner.parts));
            continue;
        }
        let Some(s) = simple(stmt) else {
            break;
        };
        if let Some((bytes, value, _)) = field_store(s, &names) {
            // A field stored twice is no initialiser's: the run ends before the second store.
            let twice = (filled.iter()).any(|held| held.0 < bytes.1 && bytes.0 < held.1);
            if twice || !blind(value, &names) {
                break;
            }
            filled.push(bytes);
            passed.add(value);
            roles.push(Between::Field);
            last = Some(index);
            continue;
        }
        let Stmt::Simple(ir::Stmt::Assign { target: to, value }) = stmt else {
            break;
        };
        // A global is memory a callee reads, and a name of the object assigned again is a use.
        if matches!(to, Var::Global { .. }) || names.contains(to) {
            break;
        }
        if matches!(bare(whole(value)), Expr::Var(from) if names.contains(from)) {
            if names.len() >= NAMES {
                break;
            }
            names.push(*to);
            roles.push(Between::Copy);
            continue;
        }
        let own = level(value);
        let unseen = !passed.vars.contains(to) && !(passed.level > 0 && reached.has(*to));
        let unmoved = match (own, passed.level) {
            // A call goes before the making alone, which nothing sees.
            (2, _) => opened.len() == 1 && filled.is_empty() && !makes(value),
            // What a call may write is not what this reads.
            (_, 2) => own == 0 && !exposed(value),
            _ => true,
        };
        if !(blind(value, &names) && unseen && unmoved) {
            break;
        }
        roles.push(Between::Apart);
    }
    let last = last?;
    let mut parts = opened.to_vec();
    let (mut before, mut after) = (Vec::new(), Vec::new());
    for (stmt, role) in rest.iter().zip(roles).take(last + 1) {
        match role {
            Between::Field => {
                let s = simple(stmt)?;
                let (_, _, through) = field_store(s, &names)?;
                let mut s = s.clone();
                if through != target {
                    if let ir::Stmt::Store { addr, .. } | ir::Stmt::StoreBytes { addr, .. } = &mut s
                    {
                        *addr = Expr::Var(target);
                    }
                }
                parts.push(s);
            }
            Between::Copy => after.push(stmt.clone()),
            Between::Apart => before.push(stmt.clone()),
            Between::Inner(made) => parts.extend(made),
            Between::Held => {}
        }
    }
    let kind = Group::Construct;
    Some(Initialised {
        before,
        made: Sugar::Group { kind, parts },
        after,
        held: last + 2,
    })
}

/// Whether evaluating `value` where the source would cannot tell the object `names` name from what the variable held before: it names none of them, and reads no memory a pointer to one may be in.
fn blind_to(value: &Expr, names: &[Var], reached: &Reached) -> bool {
    let alone = level(value) == 0;
    (names.iter()).all(|name| !mentions(value, *name) && (alone || !reached.has(*name)))
}

/// An object or array made inside another: the statements it is, and the statements of the list it stands for before the store that takes it.
struct Inner {
    parts: Vec<ir::Stmt>,
    held: usize,
}

/// The object or array made, constructed and filled by the statements `list` starts with, where the statement after them stores it whole to a field of the object `names` name: `new A { F = new B { G = a } }`.
// ⚠ The source makes the inner one where the field takes it, which is where these statements
// make it: so only where its own statements stand straight before that store, none of them can
// tell the outer object from what its variable held before, and nothing else names the inner
// variable, which the source has no name for.
fn nested(
    list: &[Stmt],
    names: &[Var],
    filled: &[(u32, u32)],
    around: &Around<'_>,
    depth: usize,
) -> Option<Inner> {
    if depth >= DEPTH {
        return None;
    }
    let (inner, fresh, opened) = opening(list.first()?)?;
    let global = matches!(inner, Var::Global { .. });
    if global || names.contains(&inner) || around.reached.has(inner) {
        return None;
    }
    let window = list.get(..list.len().min(INNER)).unwrap_or_default();
    let (parts, held) = match initialiser(window, around, depth + 1) {
        Some(read) if read.before.is_empty() && read.after.is_empty() => match read.made {
            Sugar::Group { parts, .. } => (parts, read.held),
            _ => return None,
        },
        Some(_) => return None,
        None => match fresh {
            Fresh::Object => (opened.to_vec(), 1),
            Fresh::Array(_) => return None,
        },
    };
    let store = simple(list.get(held)?)?;
    let (bytes, value, _) = field_store(store, names)?;
    let twice = (filled.iter()).any(|held| held.0 < bytes.1 && bytes.0 < held.1);
    let taken = matches!(bare(whole(value)), Expr::Var(var) if *var == inner);
    let mut unseen = true;
    for s in &parts {
        s.each_expr(&mut |e| unseen &= blind_to(e, names, around.reached));
    }
    let alone = around.named.get(&inner) == Some(&times(inner, parts.iter().chain([store])));
    (!twice && taken && unseen && alone).then_some(Inner { parts, held })
}

/// The initialiser the statements at the head of `list` are, read inside `depth` others.
fn initialiser(list: &[Stmt], around: &Around<'_>, depth: usize) -> Option<Initialised> {
    let (first, rest) = list.split_first()?;
    let (target, fresh, opened) = opening(first)?;
    let len = match fresh {
        Fresh::Object => return object(target, opened, rest, around, depth),
        Fresh::Array(len) => len,
    };
    let reached = around.reached;
    // ⚠ The source assigns the variable after the last element: no value may be able to tell,
    // by the variable's name or through a pointer to it.
    let blind =
        |value: &Expr| !mentions(value, target) && (level(value) == 0 || !reached.has(target));
    // ⚠ Every element in order and nothing else: an element left out is zero, which an
    // initialiser of fewer elements would not say.
    let mut joined: Vec<&ir::Stmt> = Vec::new();
    for (index, stmt) in rest.iter().enumerate() {
        let index = index as u64;
        if index == len {
            break;
        }
        let set = simple(stmt).and_then(|s| Some((s, element_store(s, target, index)?)));
        let Some((s, cells)) = set else {
            break;
        };
        if cells.iter().any(|cell| !blind(cell)) {
            break;
        }
        joined.push(s);
    }
    if joined.len() as u64 != len {
        return None;
    }
    let held = 1 + joined.len();
    let mut parts = opened.to_vec();
    parts.extend(joined.into_iter().cloned());
    let kind = Group::Collection;
    Some(Initialised {
        before: Vec::new(),
        made: Sugar::Group { kind, parts },
        after: Vec::new(),
        held,
    })
}

/// Reads each object or array made and filled by the statements after it as one statement.
fn initialisers(list: &mut Vec<Stmt>, around: &Around<'_>) {
    if !list.iter().any(|stmt| opening(stmt).is_some()) {
        return;
    }
    let mut out = Vec::with_capacity(list.len());
    let mut at = 0;
    while at < list.len() {
        match initialiser(&list[at..], around, 0) {
            Some(read) => {
                out.extend(read.before);
                out.push(Stmt::Sugar(Box::new(read.made)));
                out.extend(read.after);
                at += read.held;
            }
            None => {
                out.push(std::mem::replace(&mut list[at], Stmt::Break));
                at += 1;
            }
        }
    }
    *list = out;
}

#[path = "sugar_switch.rs"]
mod switch;

#[path = "sugar_boxes.rs"]
mod boxes;

#[cfg(test)]
#[path = "sugar_expressions_tests.rs"]
mod tests;
