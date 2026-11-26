//! Sugar for expressions and simple statements: tests of one value as a `switch`, the null
//! operators, compound assignment, string concatenation, initialisers and `ref`.
//!
//! Works on the structured form, last before `names`. An expression is read through an
//! `ExprSugar` and statements through a `Group` or a `Sugar::SwitchOn`, and `tree::desugar` gives
//! back what was there.
//!
//! ⚠ Three rewrites give back statements that run the same and are not the ones read: a
//! `switch` drops the ordering tests its cases were halved by, an initialiser puts the copies
//! and the assignments that stood between its stores around itself, and a choice between a
//! reference and what stands in for null is one assignment where it was an `if` of two.

use std::collections::BTreeSet;

use crate::gta6::script::ir::{
    self, BinOp, Callee, Expr, ExprSugar, Head, Kind, Scalar, Var, Width,
};
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Project, Stats};

/// The names this file's rewrites are switched by.
// ⚠ `nullable` and `tuples` read nothing: no table states which type is a nullable or a tuple.
// `ref-out` never reads an `out`, which needs a callee proved to write before it reads.
pub const PASSES: &[Pass] = &[
    Pass::SwitchOn,
    Pass::NullOps,
    Pass::Compound,
    Pass::Concat,
    Pass::Initialisers,
    Pass::RefOut,
    Pass::Nullable,
    Pass::Tuples,
];

/// The most statements one initialiser is read from: each store is held against every one before it.
const FIELDS: usize = 1024;

/// The most variables one initialiser follows its object through.
const NAMES: usize = 8;

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
    let nulls = options.on(Pass::NullOps);
    let joins = options.on(Pass::Concat);
    let refs = options.on(Pass::RefOut);
    // An explicit stack: a body nests as deep as its listing does. With each list, whether a
    // `switch` may take the statements after its first test into its arms.
    let mut work: Vec<(&mut Vec<Stmt>, bool)> = vec![(body, true)];
    while let Some((list, whole)) = work.pop() {
        if options.on(Pass::Initialisers) {
            initialisers(list, &reached);
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
                        }
                        if joins {
                            joined(e, head, project);
                        }
                        if refs {
                            referred(e, head, project);
                        }
                    });
                });
            }
            if options.on(Pass::Compound) {
                compounded(stmt, head, project);
            }
            at += 1;
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

/// What a choice reads as: `x != null ? x : e` as `x ?? e`, and `x != null ? x.f : null` as `x?.f`.
// ⚠ Only over references: a number that is not zero is no `??`, and `x?.f` of a number would be
// a nullable in C#. `x` is written twice and evaluated twice, so it holds no call.
fn null_reading(e: &Expr, head: &Head, project: &Project) -> Option<ExprSugar> {
    let Expr::Select { cond, then, els } = e else {
        return None;
    };
    let tested = not_null(cond)?;
    let then = whole(then);
    let of_tested =
        |field: &Expr| matches!(bare(field), Expr::Load { addr, .. } if same(whole(addr), tested));
    // `x ?? null` is `x`: a statement nobody writes, so such a choice keeps its plain form.
    let kind = if reference(tested, head, project) && same(then, tested) && !is_null(els) {
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

/// Reads the address of a variable handed to a call as `ref`, where it reaches that variable alone.
// ⚠ Never the first cell of a call that may be made on it: a receiver is written before the dot.
// Only a native, and a function proved to take no receiver, are handed no receiver.
fn referred(e: &mut Expr, head: &Head, project: &Project) {
    match e {
        Expr::Call { callee, args, .. } => {
            let free = |program: u32, at: u32| {
                let member = project.members.get(&ir::FunctionKey::new(program, at));
                member.is_some_and(|member| member.is_static)
            };
            let first = match callee {
                Callee::Intrinsic { .. } => return,
                Callee::Native { .. } => 0,
                Callee::Local { at, .. } => usize::from(!free(head.program, *at)),
                Callee::Import { program, at, .. } => usize::from(!free(*program, *at)),
                Callee::Builtin { .. } | Callee::Virtual { .. } => 1,
            };
            for arg in args.iter_mut().skip(first) {
                let own = matches!(&*arg, Expr::AddrOf { var, cell: 0 } if bounded(*var, head));
                if own {
                    let held = std::mem::replace(arg, Expr::GuidEmpty);
                    *arg = Expr::sugar(ExprSugar::Ref, held);
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
                    kind: ExprSugar::Ref,
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
                    match *var {
                        Var::Local(n) => {
                            reached.local = Some(reached.local.map_or(n, |low| low.min(n)))
                        }
                        Var::Param(n) => {
                            reached.param = Some(reached.param.map_or(n, |low| low.min(n)))
                        }
                        other => {
                            reached.named.insert(other);
                        }
                    }
                }
            });
        });
        reached
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
    reached: &Reached,
) -> Option<Initialised> {
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
    let blind = |value: &Expr, names: &[Var]| {
        let alone = level(value) == 0;
        (names.iter()).all(|name| !mentions(value, *name) && (alone || !reached.has(*name)))
    };
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
    for (index, stmt) in rest.iter().take(FIELDS).enumerate() {
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

/// The initialiser the statements at the head of `list` are.
fn initialiser(list: &[Stmt], reached: &Reached) -> Option<Initialised> {
    let (first, rest) = list.split_first()?;
    let (target, fresh, opened) = opening(first)?;
    let len = match fresh {
        Fresh::Object => return object(target, opened, rest, reached),
        Fresh::Array(len) => len,
    };
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
fn initialisers(list: &mut Vec<Stmt>, reached: &Reached) {
    if !list.iter().any(|stmt| opening(stmt).is_some()) {
        return;
    }
    let mut out = Vec::with_capacity(list.len());
    let mut at = 0;
    while at < list.len() {
        match initialiser(&list[at..], reached) {
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

#[cfg(test)]
#[path = "sugar_expressions_tests.rs"]
mod tests;
