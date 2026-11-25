//! Sugar for expressions and simple statements: a chain of tests as a `switch`, the null
//! operators, compound assignment, string concatenation, initialisers, `ref` and `out`, nullable
//! values and tuples.
//!
//! Works on the structured form, last before `names`. An expression is read through an
//! `ExprSugar` and statements through a `Group` or a `Sugar::SwitchOn`; none is changed, so
//! `tree::desugar` gives back exactly what was there.

use std::collections::BTreeSet;

use crate::gta6::script::ir::{self, BinOp, Callee, Expr, ExprSugar, Head, Kind, Scalar, Var};
use crate::gta6::script::tree::{self, Arm, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Project, Stats};

/// The names this file's rewrites are switched by.
// ⚠ `concat`, `ref-out`, `nullable` and `tuples` read nothing yet. Each needs a fact nothing
// supplies: which builtin joins strings, which way a parameter passes its value, which type is
// a nullable or a tuple. Without it the code stays as it is.
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

/// The least arms that make a chain of tests a `switch`.
const ARMS: usize = 3;

/// The most field stores one initialiser takes: each is held against every one before it.
const FIELDS: usize = 1024;

/// Reads each proved shape as the operator, the initialiser or the `switch` of the source.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let options = cx.options;
    if !options.any(PASSES) {
        return;
    }
    let tree::Function { head, body } = f;
    let head: &Head = head;
    let reached = match options.on(Pass::Initialisers) {
        true => Reached::of(body),
        false => Reached::default(),
    };
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        if options.on(Pass::Initialisers) {
            initialisers(list, &reached);
        }
        for stmt in list {
            if options.on(Pass::SwitchOn) {
                switched(stmt);
            }
            if options.on(Pass::NullOps) {
                tree::own_exprs_mut(stmt, &mut |top| {
                    top.walk_mut(&mut |e| null_op(e, head, cx.project));
                });
            }
            if options.on(Pass::Compound) {
                compounded(stmt, head, cx.project);
            }
            work.extend(tree::nested_mut(stmt));
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

/// Whether `e` is proved to hold a reference: a variable, a global or a field of such a kind.
fn reference(e: &Expr, head: &Head, project: &Project) -> bool {
    // A global the container marks and a field a reference was stored to hold references, though their kind says no more than an address.
    let stated = !matches!(
        e,
        Expr::Var(Var::Param(_) | Var::Local(_) | Var::Value { .. } | Var::Stack { .. })
    );
    let kind = match e {
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
    };
    match kind {
        Some(Kind::Object(_) | Kind::String | Kind::Array(_)) => true,
        Some(Kind::Pointer) => stated,
        _ => false,
    }
}

/// Reads `x != null ? x : e` as `x ?? e` and `x != null ? x.f : null` as `x?.f`.
// ⚠ Only over references: a number that is not zero is no `??`, and `x?.f` of a number would be
// a nullable in C#. `x` is written twice and evaluated twice, so it holds no call.
fn null_op(e: &mut Expr, head: &Head, project: &Project) {
    let Expr::Select { cond, then, els } = &*e else {
        return;
    };
    let Some(tested) = not_null(cond) else {
        return;
    };
    let then = whole(then);
    let of_tested =
        |field: &Expr| matches!(bare(field), Expr::Load { addr, .. } if same(whole(addr), tested));
    let kind = if reference(tested, head, project) && same(then, tested) {
        ExprSugar::Coalesce
    } else if is_null(els) && reference(then, head, project) && of_tested(then) {
        ExprSugar::NullConditional
    } else {
        return;
    };
    if level(tested) > 1 {
        return;
    }
    let held = std::mem::replace(e, Expr::GuidEmpty);
    *e = Expr::sugar(kind, held);
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

/// What a test compares with constants, the type it compares at, and the constants: `x == a`, or `x == a || x == b`.
pub(crate) fn cases(test: &Expr) -> Option<(&Expr, &Kind, Vec<u64>)> {
    let mut tested: Option<(&Expr, &Kind)> = None;
    let mut constants = Vec::new();
    let mut open = vec![test];
    while let Some(e) = open.pop() {
        match e {
            Expr::Binary {
                op: BinOp::LogicalOr,
                l,
                r,
                ..
            } => {
                open.push(r);
                open.push(l);
            }
            Expr::Binary {
                op: BinOp::Eq,
                at,
                l,
                r,
            } => {
                let wide = match at {
                    Kind::I32 | Kind::U32 => false,
                    Kind::I64 | Kind::U64 => true,
                    _ => return None,
                };
                let one = tested.is_none_or(|(value, held)| held == at && same(value, l));
                if !one {
                    return None;
                }
                constants.push(constant(r, wide)?);
                tested = Some((&**l, at));
            }
            _ => return None,
        }
    }
    tested.map(|(value, at)| (value, at, constants))
}

/// Whether a `break` in `body` leaves a loop or a `switch` around `body`.
fn breaks(body: &[Stmt]) -> bool {
    let mut open = vec![body];
    while let Some(list) = open.pop() {
        for stmt in list {
            match stmt {
                Stmt::Break => return true,
                Stmt::If { then, els, .. } => open.extend([then.as_slice(), els.as_slice()]),
                // A construct counts as no loop of its own: a `break` in a `foreach` refuses too.
                Stmt::Sugar(sugar) => open.extend(sugar.parts().bodies),
                _ => {}
            }
        }
    }
    false
}

/// How many arms of the chain of `if`s at `stmt` are the cases of one `switch`; `None` below three.
// ⚠ Each arm tests the one value against constants no arm before it tests, the value is read
// with no call, so every test sees what the first saw, and no arm leaves by a `break`, which
// in a `switch` would leave the switch.
fn chain(stmt: &Stmt) -> Option<usize> {
    let mut seen = BTreeSet::new();
    let mut tested: Option<(&Expr, &Kind)> = None;
    let mut bodies = Vec::new();
    let mut next = stmt;
    let rest = loop {
        let stays = std::slice::from_ref(next);
        let Stmt::If { cond, then, els } = next else {
            break stays;
        };
        let Some((value, at, constants)) = cases(cond) else {
            break stays;
        };
        let one = tested.is_none_or(|(held, kind)| kind == at && same(held, value));
        if !one || level(value) > 1 || !constants.into_iter().all(|c| seen.insert(c)) {
            break stays;
        }
        tested = Some((value, at));
        bodies.push(then.as_slice());
        match els.as_slice() {
            [inner] => next = inner,
            other => break other,
        }
    };
    let arms = bodies.len();
    bodies.push(rest);
    (arms >= ARMS && !bodies.into_iter().any(breaks)).then_some(arms)
}

/// Reads an `if` whose `else` is an `if`, and so on, as a `switch` where the chain is one.
fn switched(stmt: &mut Stmt) {
    let Some(arms) = chain(stmt) else {
        return;
    };
    let mut taken = Vec::with_capacity(arms);
    let mut next = std::mem::replace(stmt, Stmt::Break);
    // Whatever is not taken as an arm stays, as the default.
    let default = loop {
        match next {
            Stmt::If {
                cond,
                then,
                mut els,
            } if taken.len() < arms => {
                taken.push(Arm {
                    test: cond,
                    body: then,
                });
                match els.pop() {
                    Some(inner) if els.is_empty() && taken.len() < arms => next = inner,
                    Some(inner) => {
                        els.push(inner);
                        break els;
                    }
                    None => break els,
                }
            }
            other => break vec![other],
        }
    };
    *stmt = Stmt::Sugar(Box::new(Sugar::SwitchOn {
        arms: taken,
        default,
    }));
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

/// The bytes of the object in `object` a statement stores to, first and one past the last, and what it stores.
// An object's first eight bytes are its type: a store there is no field's.
fn field_store(s: &ir::Stmt, object: Var) -> Option<((u32, u32), &Expr)> {
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
    let own = matches!(bare(whole(addr)), Expr::Var(var) if *var == object);
    (own && offset >= 8).then_some(((offset, end), value))
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

/// The initialiser the statements at the head of `list` are, and how many of them it stands for.
fn initialiser(list: &[Stmt], reached: &Reached) -> Option<(Sugar, usize)> {
    let (first, rest) = list.split_first()?;
    let (target, fresh, opened) = opening(first)?;
    // ⚠ The source assigns the variable after the last store and the statements before the
    // first: no value may be able to tell, by the variable's name or through a pointer to it.
    let blind =
        |value: &Expr| !mentions(value, target) && (level(value) == 0 || !reached.has(target));
    let mut joined: Vec<&ir::Stmt> = Vec::new();
    let kind = match fresh {
        Fresh::Object => {
            let mut filled: Vec<(u32, u32)> = (opened.iter())
                .filter_map(|s| field_store(s, target))
                .map(|(bytes, _)| bytes)
                .collect();
            for stmt in rest.iter().take(FIELDS) {
                let stored = simple(stmt).and_then(|s| Some((s, field_store(s, target)?)));
                let Some((s, (bytes, value))) = stored else {
                    break;
                };
                // A field stored twice is no initialiser's: the run ends before the second store.
                let twice = (filled.iter()).any(|held| held.0 < bytes.1 && bytes.0 < held.1);
                if twice || !blind(value) {
                    break;
                }
                filled.push(bytes);
                joined.push(s);
            }
            if joined.is_empty() {
                return None;
            }
            Group::Construct
        }
        // ⚠ Every element in order and nothing else: an element left out is zero, which an
        // initialiser of fewer elements would not say.
        Fresh::Array(len) => {
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
            Group::Collection
        }
    };
    let held = 1 + joined.len();
    let mut parts = opened.to_vec();
    parts.extend(joined.into_iter().cloned());
    Some((Sugar::Group { kind, parts }, held))
}

/// Reads each object or array made and filled by the statements straight after it as one statement.
fn initialisers(list: &mut Vec<Stmt>, reached: &Reached) {
    if !list.iter().any(|stmt| opening(stmt).is_some()) {
        return;
    }
    let mut out = Vec::with_capacity(list.len());
    let mut at = 0;
    while at < list.len() {
        match initialiser(&list[at..], reached) {
            Some((made, held)) => {
                out.push(Stmt::Sugar(Box::new(made)));
                at += held;
            }
            None => {
                out.push(std::mem::replace(&mut list[at], Stmt::Break));
                at += 1;
            }
        }
    }
    *list = out;
}

#[cfg(test)]
#[path = "sugar_expressions_tests.rs"]
mod tests;
