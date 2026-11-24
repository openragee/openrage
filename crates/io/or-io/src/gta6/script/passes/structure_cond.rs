//! Conditions: negating one, joining two, and reading what a test compares or a constant makes of it.

use crate::gta6::script::ir::{BinOp, Const, Expr, Kind, Scalar, UnOp, Var};

/// How deep a rewrite follows a condition; past it the condition is wrapped or left as it is.
const DEPTH: u32 = 96;

fn wrapped(cond: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::Not,
        e: Box::new(cond),
    }
}

/// `!cond` as a condition, the negation pushed into `&&`, `||`, `?:` and each comparison it is exact for.
pub(super) fn negate(cond: Expr) -> Expr {
    not(cond, DEPTH)
}

fn not(cond: Expr, depth: u32) -> Expr {
    if depth == 0 {
        return wrapped(cond);
    }
    let d = depth - 1;
    match cond {
        Expr::Unary { op: UnOp::Not, e } => *e,
        Expr::Binary {
            op: BinOp::LogicalAnd,
            l,
            r,
            ..
        } => or(not(*l, d), not(*r, d)),
        Expr::Binary {
            op: BinOp::LogicalOr,
            l,
            r,
            ..
        } => and(not(*l, d), not(*r, d)),
        Expr::Select { cond, then, els } => Expr::Select {
            cond,
            then: Box::new(not(*then, d)),
            els: Box::new(not(*els, d)),
        },
        Expr::Binary { op, at, l, r } if flips(op, &at) => Expr::Binary {
            op: flipped(op),
            at,
            l,
            r,
        },
        Expr::Const(c) if c.kind == Kind::Bool => Expr::Const(Const::bool(c.bits == 0)),
        other => wrapped(other),
    }
}

/// Whether `!(a op b)` is `a` under the opposite operator `b`.
// ⚠ Not for an ordering of floats: `!(a < b)` holds for a NaN and `a >= b` does not.
fn flips(op: BinOp, at: &Kind) -> bool {
    match op {
        BinOp::Eq | BinOp::Ne => true,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => at.is_integer(),
        _ => false,
    }
}

fn flipped(op: BinOp) -> BinOp {
    match op {
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        BinOp::Lt => BinOp::Ge,
        BinOp::Ge => BinOp::Lt,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        other => other,
    }
}

pub(super) fn and(l: Expr, r: Expr) -> Expr {
    Expr::binary(BinOp::LogicalAnd, l, r)
}

pub(super) fn or(l: Expr, r: Expr) -> Expr {
    Expr::binary(BinOp::LogicalOr, l, r)
}

/// `cond ? then : els`, the test of one of two tests.
pub(super) fn select(cond: Expr, then: Expr, els: Expr) -> Expr {
    Expr::Select {
        cond: Box::new(cond),
        then: Box::new(then),
        els: Box::new(els),
    }
}

/// Whether an expression yields 0 or 1 and nothing else.
fn is_truth(e: &Expr) -> bool {
    match e {
        Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
        Expr::Binary { op, .. } => op.is_logical(),
        Expr::Const(c) => c.kind == Kind::Bool,
        _ => false,
    }
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Const(c) if c.bits == 0 && !c.kind.is_float())
}

/// The expression under the casts that read its low bits as an integer.
fn under_integer_casts(e: &Expr) -> &Expr {
    let mut e = e;
    loop {
        match e {
            Expr::Cast { to, e: inner } if !matches!(to, Scalar::F32 | Scalar::F64) => {
                let under: &Expr = inner;
                e = under;
            }
            _ => return e,
        }
    }
}

/// The expression under the casts that read 32 bits of it or more: its low 32 bits are theirs.
// ⚠ A narrower cast cuts bits a 32-bit comparison sees, so it stays part of what is compared.
pub(super) fn under_wide_casts(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// A condition read through its tests of a truth value: `as_u32(a < b) == 0` is `a >= b`.
// A truth value is 0 or 1 under any integer cast, so testing it against zero tests it.
pub(super) fn simplify(cond: Expr) -> Expr {
    simpler(cond, DEPTH)
}

fn simpler(cond: Expr, depth: u32) -> Expr {
    if depth == 0 {
        return cond;
    }
    let d = depth - 1;
    match cond {
        Expr::Binary { op, at, l, r }
            if matches!(op, BinOp::Eq | BinOp::Ne)
                && !at.is_float()
                && is_zero(&r)
                && is_truth(under_integer_casts(&l)) =>
        {
            let mut tested = *l;
            loop {
                match tested {
                    Expr::Cast { e, .. } => tested = *e,
                    bare => {
                        tested = bare;
                        break;
                    }
                }
            }
            let tested = simpler(tested, d);
            match op {
                BinOp::Eq => negate(tested),
                _ => tested,
            }
        }
        Expr::Unary { op: UnOp::Not, e } => negate(simpler(*e, d)),
        Expr::Truth(e) if is_truth(&e) => simpler(*e, d),
        other => other,
    }
}

/// Whether `e` reads `var`.
pub(super) fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |inner| {
        found |= matches!(inner, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

/// The one variable a condition tests against constants and nothing else, where it is a temporary.
// The variable is then the first thing the condition evaluates, so a value put in its place is
// evaluated where its assignment was.
pub(super) fn tested_temporary(cond: &Expr) -> Option<Var> {
    let mut vars = Vec::new();
    let mut plain = true;
    let mut nodes = 0u32;
    cond.walk(&mut |e| {
        nodes += 1;
        match e {
            Expr::Var(var) => vars.push(*var),
            Expr::Const(_) | Expr::Truth(_) => {}
            Expr::Cast { to, .. } => plain &= !matches!(to, Scalar::F32 | Scalar::F64),
            Expr::Unary { op, .. } => plain &= *op == UnOp::Not,
            Expr::Binary { op, at, .. } => {
                plain &= matches!(op, BinOp::Eq | BinOp::Ne) && !at.is_float();
            }
            _ => plain = false,
        }
    });
    match vars.as_slice() {
        [var] if plain && nodes <= DEPTH && var.is_temporary() => Some(*var),
        _ => None,
    }
}

/// `cond` with `value` where it reads `var`.
pub(super) fn with_value(cond: &Expr, var: Var, value: Expr) -> Expr {
    let mut out = cond.clone();
    let mut value = Some(value);
    out.walk_mut(&mut |e| {
        if matches!(e, Expr::Var(v) if *v == var) {
            if let Some(value) = value.take() {
                *e = value;
            }
        }
    });
    out
}

/// What a cell holds while a condition is worked out: bits, or the truth of a test.
#[derive(Clone, Copy)]
enum Held {
    Bits(u64),
    Truth(bool),
}

impl Held {
    fn bits(self) -> u64 {
        match self {
            Held::Bits(bits) => bits,
            Held::Truth(truth) => u64::from(truth),
        }
    }

    fn holds(self) -> bool {
        self.bits() != 0
    }
}

/// Whether a condition [`tested_temporary`] accepts holds when its variable is `bits`, `None` where this cannot say.
pub(super) fn holds_for(cond: &Expr, bits: u64) -> Option<bool> {
    held(cond, bits, DEPTH).map(Held::holds)
}

fn held(e: &Expr, var: u64, depth: u32) -> Option<Held> {
    let d = depth.checked_sub(1)?;
    Some(match e {
        Expr::Var(_) => Held::Bits(var),
        Expr::Const(c) => Held::Bits(c.bits),
        Expr::Truth(e) => Held::Truth(held(e, var, d)?.holds()),
        Expr::Unary { op: UnOp::Not, e } => Held::Truth(!held(e, var, d)?.holds()),
        Expr::Cast { to, e } => {
            let bits = held(e, var, d)?.bits();
            Held::Bits(match to {
                Scalar::I8 => bits as i8 as i64 as u64,
                Scalar::U8 => u64::from(bits as u8),
                Scalar::I16 => bits as i16 as i64 as u64,
                Scalar::U16 => u64::from(bits as u16),
                Scalar::I32 => bits as i32 as i64 as u64,
                Scalar::U32 => u64::from(bits as u32),
                Scalar::I64 | Scalar::U64 => bits,
                Scalar::F32 | Scalar::F64 => return None,
            })
        }
        Expr::Binary { op, at, l, r } if matches!(op, BinOp::Eq | BinOp::Ne) => {
            let (l, r) = (held(l, var, d)?, held(r, var, d)?);
            let equal = match at {
                Kind::Bool => l.holds() == r.holds(),
                Kind::I32 | Kind::U32 => l.bits() as u32 == r.bits() as u32,
                Kind::I64 | Kind::U64 | Kind::Unknown => l.bits() == r.bits(),
                _ => return None,
            };
            Held::Truth(equal == (*op == BinOp::Eq))
        }
        _ => return None,
    })
}

/// A branch condition that compares one variable with a 32-bit constant.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Test {
    /// The variable compared, under whatever casts the comparison reads it through.
    pub tested: Expr,
    /// The comparison as `tested op constant`.
    pub op: BinOp,
    pub signed: bool,
    pub constant: u32,
}

impl Test {
    /// The test a condition states, where it states one.
    pub fn of(cond: &Expr) -> Option<Test> {
        let Expr::Binary { op, at, l, r } = cond else {
            return None;
        };
        let signed = match at {
            Kind::I32 => true,
            Kind::U32 => false,
            _ => return None,
        };
        if !op.is_comparison() {
            return None;
        }
        let constant = |e: &Expr| match under_wide_casts(e) {
            Expr::Const(c) if !c.kind.is_float() => Some(c.bits as u32),
            _ => None,
        };
        let variable = |e: &Expr| matches!(under_integer_casts(e), Expr::Var(_));
        let left: &Expr = l;
        let right: &Expr = r;
        // A constant on the left is the same test read from the other side.
        let sides = (
            variable(left),
            constant(right),
            constant(left),
            variable(right),
        );
        let (tested, op, constant) = match sides {
            (true, Some(k), _, _) => (left, *op, k),
            (_, _, Some(k), true) => (right, mirrored(*op), k),
            _ => return None,
        };
        Some(Test {
            tested: tested.clone(),
            op,
            signed,
            constant,
        })
    }

    pub fn is_equality(&self) -> bool {
        matches!(self.op, BinOp::Eq | BinOp::Ne)
    }

    /// Whether the test holds when the variable's low 32 bits are `value`.
    pub fn holds(&self, value: u32) -> bool {
        let ordering = match self.signed {
            true => (value as i32).cmp(&(self.constant as i32)),
            false => value.cmp(&self.constant),
        };
        match self.op {
            BinOp::Eq => ordering.is_eq(),
            BinOp::Ne => ordering.is_ne(),
            BinOp::Lt => ordering.is_lt(),
            BinOp::Le => ordering.is_le(),
            BinOp::Gt => ordering.is_gt(),
            _ => ordering.is_ge(),
        }
    }
}

/// `b op' a` for `a op b`.
fn mirrored(op: BinOp) -> BinOp {
    match op {
        BinOp::Lt => BinOp::Gt,
        BinOp::Gt => BinOp::Lt,
        BinOp::Le => BinOp::Ge,
        BinOp::Ge => BinOp::Le,
        other => other,
    }
}

/// A switch on `x - k` or `x + k` as a switch on `x`, with `k` moved into the case labels.
// Exact while no label leaves the range of an `int`: the switch compares 32 bits either way.
pub(super) fn unbiased(value: &Expr, labels: &[i64]) -> Option<(Expr, Vec<i64>)> {
    let Expr::Binary { op, at, l, r } = under_wide_casts(value) else {
        return None;
    };
    let sign = match op {
        BinOp::Sub => 1,
        BinOp::Add => -1,
        _ => return None,
    };
    let bias = match under_wide_casts(r) {
        Expr::Const(c) if at.is_integer() && !c.kind.is_float() => i64::from(c.bits as u32 as i32),
        _ => return None,
    };
    let int = i64::from(i32::MIN)..=i64::from(i32::MAX);
    let moved: Vec<i64> = labels.iter().map(|label| label + sign * bias).collect();
    let fits = moved.iter().all(|label| int.contains(label));
    fits.then(|| ((**l).clone(), moved))
}

/// Whether a returned value is small enough, and free enough of calls, to write at each jump to its `return`.
pub(super) fn cheap(value: Option<&Expr>) -> bool {
    let Some(value) = value else {
        return true;
    };
    let mut nodes = 0u32;
    let mut plain = true;
    value.walk(&mut |e| {
        nodes += 1;
        plain &= !matches!(
            e,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    plain && nodes <= 12
}
