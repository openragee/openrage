//! The `switch` a compiler laid out as tests of one value: a chain of equalities, the ordering
//! tests that halve a sparse set of cases, the small switches left under them, and a condition
//! whose ordering tests choose among such equalities.
//!
//! ⚠ The `Sugar::SwitchOn` made here holds each arm under the equality that leads to it and no
//! ordering test: `tree::desugar` gives a chain of `if`s that runs as the tests did, not the tests
//! themselves. It is made only where every constant is proved to reach its arm through the
//! ordering tests above it. Arms with no statements, which all run on to what follows the
//! `switch`, are one arm of all their constants, so `desugar` tests them in one `if`.

use std::collections::BTreeSet;

use crate::gta6::script::ir::{BinOp, Const, Expr, Kind, Scalar};
use crate::gta6::script::tree::{Arm, Case, Stmt, Sugar};

use super::{constant, level, same};

/// The constants one body of a switch is reached by, each with the test that named it where one did.
type Cases<'a> = Vec<(u64, Option<&'a Expr>)>;

/// The least arms that make tests a `switch`.
const ARMS: usize = 3;

/// The most tests one `switch` is read from.
const TESTS: usize = 1024;

/// The statements one reading looks at to learn where bodies leave; past them a body is taken not to.
const LOOKS: usize = 1 << 14;

/// The most constants arms with no statements are joined into one arm by: each joins one more `||` to its test.
const JOINED: usize = 256;

/// `x < k`, `x <= k`, `x > k` or `x >= k`, with how the two are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Order {
    pub op: BinOp,
    pub signed: bool,
    /// Whole cells are compared, not their low 32 bits.
    pub wide: bool,
    pub bound: u64,
}

impl Order {
    /// Whether the test holds of a value whose compared bits are `value`.
    fn holds(self, value: u64) -> bool {
        let ordering = match (self.wide, self.signed) {
            (true, true) => (value as i64).cmp(&(self.bound as i64)),
            (true, false) => value.cmp(&self.bound),
            (false, true) => (value as u32 as i32).cmp(&(self.bound as u32 as i32)),
            (false, false) => (value as u32).cmp(&(self.bound as u32)),
        };
        match self.op {
            BinOp::Lt => ordering.is_lt(),
            BinOp::Le => ordering.is_le(),
            BinOp::Gt => ordering.is_gt(),
            _ => ordering.is_ge(),
        }
    }
}

/// What a condition asks of the value it tests.
pub(super) enum What<'a> {
    /// `x == a`, or `x == a || x == b`: each constant as it is compared, and as it is written.
    Equal(Vec<(u64, &'a Expr)>),
    /// `x != a`, or `x != a && x != b`: each constant as it is compared, and as it is written where it is compared at the type of the first.
    Unequal(Vec<(u64, Option<&'a Expr>)>),
    Ordered(Order),
    /// `x > k ? x == a || x == b : x == c`: ordering tests that choose among tests of one value, which come to whether the value is one of the constants (`held`) or none of them.
    Among {
        constants: Vec<(u64, Option<&'a Expr>)>,
        held: bool,
    },
}

/// A condition that compares one value with constants.
pub(super) struct Test<'a> {
    /// The value as the condition writes it.
    pub value: &'a Expr,
    /// The type it is compared at.
    pub at: &'a Kind,
    /// Whole cells are compared, not their low 32 bits.
    pub wide: bool,
    pub what: What<'a>,
}

/// Whether a comparison at `at` is of whole cells, and whether it reads a sign.
fn width(at: &Kind) -> Option<(bool, bool)> {
    match at {
        Kind::I32 => Some((false, true)),
        Kind::U32 => Some((false, false)),
        Kind::I64 => Some((true, true)),
        Kind::U64 => Some((true, false)),
        _ => None,
    }
}

/// The test a condition is, where it compares one value with constants.
pub(super) fn test(cond: &Expr) -> Option<Test<'_>> {
    if let Expr::Select { .. } = cond {
        return among(cond);
    }
    if let Expr::Binary { op, at, l, r } = cond {
        let ordered = matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge);
        if ordered || *op == BinOp::Ne {
            let (wide, signed) = width(at)?;
            let bound = constant(r, wide)?;
            let what = match ordered {
                true => What::Ordered(Order {
                    op: *op,
                    signed,
                    wide,
                    bound,
                }),
                false => What::Unequal(vec![(bound, Some(&**r))]),
            };
            return Some(Test {
                value: l,
                at,
                wide,
                what,
            });
        }
    }
    if let Expr::Binary {
        op: BinOp::LogicalAnd,
        ..
    } = cond
    {
        return unequals(cond);
    }
    let mut tested: Option<(&Expr, &Kind, bool)> = None;
    let mut constants = Vec::new();
    let mut open = vec![cond];
    while let Some(e) = open.pop() {
        match e {
            Expr::Binary {
                op: BinOp::LogicalOr,
                l,
                r,
                ..
            } => {
                open.push(&**r);
                open.push(&**l);
            }
            Expr::Binary {
                op: BinOp::Eq,
                at,
                l,
                r,
            } => {
                let (wide, _) = width(at)?;
                let one = tested.is_none_or(|(value, held, _)| held == at && same(value, l));
                if !one {
                    return None;
                }
                constants.push((constant(r, wide)?, &**r));
                tested = Some((&**l, at, wide));
            }
            _ => return None,
        }
    }
    let (value, at, wide) = tested?;
    Some(Test {
        value,
        at,
        wide,
        what: What::Equal(constants),
    })
}

/// `x != a && x != b ..`: the test that one value is none of the constants, each compared at one width.
// 32 bits are unequal whichever sign they are read with, so the types may differ in sign alone.
fn unequals(cond: &Expr) -> Option<Test<'_>> {
    let mut first: Option<(&Expr, &Kind, bool)> = None;
    let mut constants = Vec::new();
    let mut open = vec![cond];
    while let Some(e) = open.pop() {
        if constants.len() >= TESTS {
            return None;
        }
        match e {
            Expr::Binary {
                op: BinOp::LogicalAnd,
                l,
                r,
                ..
            } => {
                open.push(&**r);
                open.push(&**l);
            }
            Expr::Binary {
                op: BinOp::Ne,
                at,
                l,
                r,
            } => {
                let (wide, _) = width(at)?;
                let own = match first {
                    None => {
                        first = Some((&**l, at, wide));
                        true
                    }
                    Some((value, held, as_wide)) => {
                        if as_wide != wide || !same(under(value, wide), under(l, wide)) {
                            return None;
                        }
                        held == at
                    }
                };
                constants.push((constant(r, wide)?, own.then_some(&**r)));
            }
            _ => return None,
        }
    }
    let (value, at, wide) = first?;
    Some(Test {
        value,
        at,
        wide,
        what: What::Unequal(constants),
    })
}

/// `e` under the readings that change no bit a comparison of this width sees.
fn under(e: &Expr, wide: bool) -> &Expr {
    let mut e = e;
    loop {
        match e {
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => e = &**inner,
            Expr::Cast {
                to: Scalar::I32 | Scalar::U32,
                e: inner,
            } if !wide => e = &**inner,
            _ => return e,
        }
    }
}

/// Whether `value`, read at the width `wide`, is the one value `subject` holds; the first is, unless reading it again could see another value.
fn one_value<'a>(subject: &mut Option<(&'a Expr, bool)>, value: &'a Expr, wide: bool) -> bool {
    let read = under(value, wide);
    match *subject {
        Some((held, as_wide)) => as_wide == wide && same(held, read),
        None if level(read) > 1 => false,
        None => {
            *subject = Some((read, wide));
            true
        }
    }
}

/// The test a choice is where its ordering tests choose among tests of one value against constants: `x > k ? x == a || x == b : x == c`.
// ⚠ Only where every constant stands on the way its ordering tests send it: a value equal to one
// then takes the way to that constant's own test, and a value equal to none finds none on
// whichever way it takes, so the choice holds where the value is one of them all. Every way ends
// in tests of one sense, all `==` or all `!=`, at one width.
fn among(cond: &Expr) -> Option<Test<'_>> {
    // Each ordering test met twice, the way on where it holds and then where it does not, with
    // the one it is reached through.
    let mut path: Vec<(Order, bool, Option<usize>)> = Vec::new();
    let mut open: Vec<(&Expr, Option<usize>)> = vec![(cond, None)];
    let mut subject: Option<(&Expr, bool)> = None;
    // The first equality's value and type, which the arm is written with.
    let mut first: Option<(&Expr, &Kind)> = None;
    let mut held: Option<bool> = None;
    let mut constants: Vec<(u64, Option<&Expr>)> = Vec::new();
    while let Some((e, above)) = open.pop() {
        if path.len() + constants.len() >= TESTS {
            return None;
        }
        if let Expr::Select {
            cond: order,
            then,
            els,
        } = e
        {
            let ordered = matches!(
                &**order,
                Expr::Binary {
                    op: BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
                    ..
                }
            );
            if !ordered {
                return None;
            }
            let asked = test(order)?;
            let (value, wide) = (asked.value, asked.wide);
            let What::Ordered(order) = asked.what else {
                return None;
            };
            if !one_value(&mut subject, value, wide) {
                return None;
            }
            let at = path.len();
            path.push((order, true, above));
            path.push((order, false, above));
            open.push((&**els, Some(at + 1)));
            open.push((&**then, Some(at)));
            continue;
        }
        // A way's end: a test of the value, which is no choice and so reads no further.
        if !matches!(e, Expr::Binary { .. }) {
            return None;
        }
        let asked = test(e)?;
        let (value, at, wide) = (asked.value, asked.at, asked.wide);
        let (found, equal): (Vec<(u64, Option<&Expr>)>, bool) = match asked.what {
            What::Equal(found) => (found.into_iter().map(|(c, w)| (c, Some(w))).collect(), true),
            What::Unequal(found) => (found, false),
            _ => return None,
        };
        if *held.get_or_insert(equal) != equal || !one_value(&mut subject, value, wide) {
            return None;
        }
        let (_, kind) = *first.get_or_insert((value, at));
        for (constant, written) in found {
            let mut up = above;
            while let Some(index) = up {
                let (order, way, over) = *path.get(index)?;
                if order.holds(constant) != way {
                    return None;
                }
                up = over.filter(|over| *over < index);
            }
            constants.push((constant, written.filter(|_| kind == at)));
        }
    }
    let ((value, at), held, (_, wide)) = (first?, held?, subject?);
    Some(Test {
        value,
        at,
        wide,
        what: What::Among { constants, held },
    })
}

/// The constant a case is written as in a test at `at`.
fn label(constant: u64, at: &Kind) -> Expr {
    let bits = match at {
        Kind::I32 => constant as u32 as i32 as i64 as u64,
        _ => constant,
    };
    Expr::Const(Const {
        bits,
        kind: at.clone(),
    })
}

/// Whether a `break` in `body` leaves a loop or a `switch` around `body`; taken to where finding out takes more looks than are left.
fn breaks(body: &[Stmt], looks: &mut usize) -> bool {
    let mut open = vec![body];
    while let Some(list) = open.pop() {
        for stmt in list {
            if *looks == 0 {
                return true;
            }
            *looks -= 1;
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

/// Whether control never runs off the end of `body`, nor leaves it by a `break`; `false` where nothing says.
fn leaves(body: &[Stmt], looks: &mut usize) -> bool {
    let mut open = vec![body];
    while let Some(list) = open.pop() {
        if *looks == 0 {
            return false;
        }
        *looks -= 1;
        match list.last() {
            Some(Stmt::Return(_) | Stmt::Continue | Stmt::Goto(_) | Stmt::Unreachable) => {}
            Some(Stmt::Sugar(sugar)) if matches!(**sugar, Sugar::Throw(_)) => {}
            Some(Stmt::If { then, els, .. }) if !then.is_empty() && !els.is_empty() => {
                open.push(then);
                open.push(els);
            }
            // Some case holds whatever the value is, none leaves the switch by a `break`, and
            // the last, which the others run into, leaves.
            Some(Stmt::Switch { cases, .. }) => {
                let always = cases.iter().any(|case| case.labels.contains(&None));
                let out = cases.iter().any(|case| breaks(&case.body, looks));
                match cases.last() {
                    Some(last) if always && !out => open.push(&last.body),
                    _ => return false,
                }
            }
            _ => return false,
        }
    }
    true
}

/// One way on from an ordering test.
#[derive(Clone, Copy)]
struct Side {
    order: Order,
    /// Whether this is the way on where the test holds.
    held: bool,
    /// The ordering test this one is reached through.
    above: Option<usize>,
    /// Whether an arm is reached this way.
    leads: bool,
}

/// A way on from a test, still to read.
struct Open<'a> {
    list: &'a [Stmt],
    /// The ordering test it is reached through, in `Reading::orders`.
    above: Option<usize>,
    /// Where it starts in the list the reading began on, when it is the rest of that list.
    rest: Option<usize>,
}

/// Tests of one value, read as the arms of one `switch`.
struct Reading<'a> {
    /// The value under no reading of its width, and whether whole cells of it are compared.
    subject: Option<(&'a Expr, bool)>,
    /// The value and the type the arms' tests are written with: the first equality's.
    spelt: Option<(&'a Expr, Kind)>,
    /// Every constant an arm is reached by.
    seen: BTreeSet<u64>,
    /// Each ordering test twice: the way on where it holds, then the way on where it does not.
    orders: Vec<Side>,
    arms: Vec<(Expr, &'a [Stmt])>,
    /// What runs where no arm's test holds, once for each way there.
    defaults: Vec<(&'a [Stmt], Option<usize>)>,
    tests: usize,
    looks: usize,
    /// Whether an arm was read from a choice its own ordering tests halve.
    halved: bool,
}

impl<'a> Reading<'a> {
    /// Whether `value` is the value switched on, compared whole when `wide`; the first value asked of is.
    fn of(&mut self, value: &'a Expr, wide: bool) -> bool {
        let read = under(value, wide);
        match self.subject {
            Some((held, as_wide)) => as_wide == wide && same(held, read),
            // ⚠ Every test reads it again: with a call in it the tests after the first would
            // see another value.
            None if level(read) > 1 => false,
            None => {
                self.subject = Some((read, wide));
                true
            }
        }
    }

    /// Whether the value can be `constant` where every ordering test from `above` up went the way it did.
    fn reaches(&self, constant: u64, above: Option<usize>) -> bool {
        let mut at = above;
        while let Some(index) = at {
            let Some(side) = self.orders.get(index).copied() else {
                return false;
            };
            if side.order.holds(constant) != side.held {
                return false;
            }
            at = side.above.filter(|up| *up < index);
        }
        true
    }

    /// Takes `constants` as the cases of one arm; `false`, with none taken, where one is a case already or no value that reaches the arm is it.
    fn takes(&mut self, constants: &[u64], above: Option<usize>) -> bool {
        let mut new = BTreeSet::new();
        for constant in constants {
            let fresh = !self.seen.contains(constant) && new.insert(*constant);
            if !fresh || !self.reaches(*constant, above) {
                return false;
            }
        }
        if new.is_empty() {
            return false;
        }
        self.seen.extend(new);
        let mut at = above;
        while let Some(index) = at {
            let Some(side) = self.orders.get_mut(index) else {
                break;
            };
            side.leads = true;
            at = side.above.filter(|up| *up < index);
        }
        true
    }

    /// The test of the arm reached where the value is one of `constants`: `stated` where that is the test as the arms are written, else an equality with each constant.
    // ⚠ The arms are written with the first equality's value and type, so that they read as the
    // cases of one value: 32 bits are equal whichever sign they are read with.
    fn written(
        &mut self,
        value: &'a Expr,
        at: &Kind,
        constants: &[(u64, Option<&'a Expr>)],
        stated: Option<&'a Expr>,
    ) -> Expr {
        let first = self.spelt.get_or_insert_with(|| (value, at.clone()));
        let (spelt, kind) = (first.0, first.1.clone());
        if let Some(stated) = stated {
            if spelt == value && kind == *at {
                return stated.clone();
            }
        }
        let mut tests = constants.iter().map(|(constant, written)| {
            let r = match written {
                Some(written) if kind == *at => (*written).clone(),
                _ => label(*constant, &kind),
            };
            Expr::Binary {
                op: BinOp::Eq,
                at: kind.clone(),
                l: Box::new(spelt.clone()),
                r: Box::new(r),
            }
        });
        let Some(mut arm) = tests.next() else {
            return Expr::Const(Const::bool(false));
        };
        for next in tests {
            arm = Expr::Binary {
                op: BinOp::LogicalOr,
                at: Kind::Bool,
                l: Box::new(arm),
                r: Box::new(next),
            };
        }
        arm
    }

    /// Reads the test `open` starts with, and says where each way on from it goes; `false`, with nothing read, where it starts with none this `switch` can hold.
    fn split(&mut self, open: &Open<'a>, more: &mut Vec<Open<'a>>) -> bool {
        let list: &'a [Stmt] = open.list;
        if self.tests >= TESTS {
            return false;
        }
        let read = match list.first() {
            Some(Stmt::If { cond, then, els }) => self.tested(cond, then, els, open, more),
            Some(Stmt::Switch { value, cases }) => self.cases(value, cases, open, more),
            _ => false,
        };
        self.tests += usize::from(read);
        read
    }

    /// [`Reading::split`] of an `if`.
    fn tested(
        &mut self,
        cond: &'a Expr,
        then: &'a [Stmt],
        els: &'a [Stmt],
        open: &Open<'a>,
        more: &mut Vec<Open<'a>>,
    ) -> bool {
        let list: &'a [Stmt] = open.list;
        let rest: &'a [Stmt] = list.get(1..).unwrap_or_default();
        let Some(asked) = test(cond) else {
            return false;
        };
        if !self.of(asked.value, asked.wide) {
            return false;
        }
        // Where the test does not hold: its `else`, or, after an `if` that leaves, what follows it.
        // ⚠ After any other `if`, what follows is reached both ways and is no arm of its own.
        let (other, after) = if rest.is_empty() {
            (els, None)
        } else if els.is_empty() && leaves(then, &mut self.looks) {
            (rest, open.rest.map(|at| at + 1))
        } else {
            return false;
        };
        let above = open.above;
        match &asked.what {
            What::Ordered(order) => {
                let held = self.orders.len();
                for holds in [true, false] {
                    self.orders.push(Side {
                        order: *order,
                        held: holds,
                        above,
                        leads: false,
                    });
                }
                more.push(Open {
                    list: other,
                    above: Some(held + 1),
                    rest: after,
                });
                more.push(Open {
                    list: then,
                    above: Some(held),
                    rest: None,
                });
            }
            What::Equal(constants) => {
                let values: Vec<u64> = constants.iter().map(|(constant, _)| *constant).collect();
                if !self.takes(&values, above) {
                    return false;
                }
                let each: Vec<(u64, Option<&'a Expr>)> = (constants.iter())
                    .map(|(constant, written)| (*constant, Some(*written)))
                    .collect();
                let arm = self.written(asked.value, asked.at, &each, Some(cond));
                self.arms.push((arm, then));
                more.push(Open {
                    list: other,
                    above,
                    rest: after,
                });
            }
            What::Unequal(constants) => {
                let values: Vec<u64> = constants.iter().map(|(constant, _)| *constant).collect();
                if !self.takes(&values, above) {
                    return false;
                }
                let arm = self.written(asked.value, asked.at, constants, None);
                self.arms.push((arm, other));
                more.push(Open {
                    list: then,
                    above,
                    rest: None,
                });
            }
            // The arm is written as the equalities the choice comes to, never as the choice.
            What::Among { constants, held } => {
                let values: Vec<u64> = constants.iter().map(|(constant, _)| *constant).collect();
                if !self.takes(&values, above) {
                    return false;
                }
                self.halved = true;
                let arm = self.written(asked.value, asked.at, constants, None);
                let (body, on, rest) = match *held {
                    true => (then, other, after),
                    false => (other, then, None),
                };
                self.arms.push((arm, body));
                more.push(Open {
                    list: on,
                    above,
                    rest,
                });
            }
        }
        true
    }

    /// [`Reading::split`] of a `switch` on the same value: each case an arm.
    fn cases(
        &mut self,
        value: &'a Expr,
        cases: &'a [Case],
        open: &Open<'a>,
        more: &mut Vec<Open<'a>>,
    ) -> bool {
        let list: &'a [Stmt] = open.list;
        let rest: &'a [Stmt] = list.get(1..).unwrap_or_default();
        if !self.of(value, false) {
            return false;
        }
        let mut found: Vec<(Cases<'a>, &'a [Stmt])> = Vec::new();
        let mut constants = Vec::new();
        let mut default: Option<&'a [Stmt]> = None;
        for (index, case) in cases.iter().enumerate() {
            let mut body: &'a [Stmt] = case.body.as_slice();
            if !leaves(body, &mut self.looks) {
                // ⚠ A case that does not leave runs into the next one or out of the switch. Only
                // the last, or one that ends in a `break`, runs on as an arm does, and only
                // where nothing follows the switch for the others to skip.
                let ends = match body.split_last() {
                    Some((Stmt::Break, before)) => {
                        body = before;
                        true
                    }
                    _ => index + 1 == cases.len(),
                };
                if !ends || !rest.is_empty() {
                    return false;
                }
            }
            if breaks(body, &mut self.looks) {
                return false;
            }
            if matches!(case.labels.as_slice(), [None]) {
                if default.replace(body).is_some() {
                    return false;
                }
                continue;
            }
            let mut labels = Vec::with_capacity(case.labels.len());
            for label in &case.labels {
                // ⚠ The switch compares 32 bits read with their sign: a label no such value
                // takes is never reached, and an equality with its low half would be.
                let Some(label) = (*label).filter(|label| i64::from(*label as i32) == *label)
                else {
                    return false;
                };
                labels.push((u64::from(label as u32), None));
            }
            if labels.is_empty() {
                return false;
            }
            constants.extend(labels.iter().map(|(constant, _)| *constant));
            found.push((labels, body));
        }
        // Every case leaves here, the default too: what follows the switch is reached by nothing.
        if default.is_some() && !rest.is_empty() {
            return false;
        }
        if !self.takes(&constants, open.above) {
            return false;
        }
        let narrow = Kind::I32;
        for (labels, body) in found {
            let arm = self.written(value, &narrow, &labels, None);
            self.arms.push((arm, body));
        }
        // Where no case holds: the default's body, else whatever follows the switch.
        more.push(match default {
            Some(body) => Open {
                list: body,
                above: open.above,
                rest: None,
            },
            None => Open {
                list: rest,
                above: open.above,
                rest: open.rest.map(|at| at + 1),
            },
        });
        true
    }
}

/// What tests read as one `switch` come to.
struct Read {
    arms: Vec<Arm>,
    default: Vec<Stmt>,
    /// The statements of the list read that the `switch` stands for.
    held: usize,
}

/// The `switch` the tests `top` starts with are, where they are one.
// ⚠ Each arm must be reached by exactly the values its test names, and whatever runs where no
// test holds must be one body: the same statements at the end of every such way.
fn read(top: &[Stmt]) -> Option<Read> {
    let mut reading = Reading {
        subject: None,
        spelt: None,
        seen: BTreeSet::new(),
        orders: Vec::new(),
        arms: Vec::new(),
        defaults: Vec::new(),
        tests: 0,
        looks: LOOKS,
        halved: false,
    };
    let mut open = vec![Open {
        list: top,
        above: None,
        rest: Some(0),
    }];
    while let Some(next) = open.pop() {
        if !reading.split(&next, &mut open) {
            reading.defaults.push((next.list, next.rest));
        }
    }
    // An ordering test halves the cases of a `switch` and no test a source writes, so under one
    // the constants count, however few arms they lead to.
    let ordered = !reading.orders.is_empty() || reading.halved;
    let halved = ordered && reading.seen.len() >= ARMS;
    if reading.arms.len() < ARMS && !halved {
        return None;
    }
    // ⚠ An ordering test with arms on one side only is not the halving of a set of cases: a
    // test the source may have written stays.
    if reading.orders.iter().any(|side| !side.leads) {
        return None;
    }
    let (first, others) = reading.defaults.split_first()?;
    if others.iter().any(|other| other.0 != first.0) {
        return None;
    }
    let mut looks = LOOKS;
    let all_leave = (reading.arms.iter()).all(|(_, body)| leaves(body, &mut looks));
    let none: &[Stmt] = &[];
    // The one way on where no test holds, when it is the rest of the list and no arm runs into
    // it, stays where it is, after the switch.
    let (default, held) = match (others.is_empty(), first.1) {
        (true, Some(at)) if all_leave => (none, at),
        _ => (first.0, top.len()),
    };
    // ⚠ A `break` would leave the `switch` in the source and a loop around it in the tests.
    let mut every = usize::MAX;
    let bodies = reading.arms.iter().map(|(_, body)| *body).chain([default]);
    for body in bodies {
        if breaks(body, &mut every) {
            return None;
        }
    }
    let arms = (joined(reading.arms).into_iter())
        .map(|(test, body)| Arm {
            test,
            body: body.to_vec(),
        })
        .collect();
    Some(Read {
        arms,
        default: default.to_vec(),
        held,
    })
}

/// The constants an arm's test names: one for each equality its `||` joins.
fn constants_of(test: &Expr) -> usize {
    let mut count = 0;
    let mut open = vec![test];
    while let Some(e) = open.pop() {
        match e {
            Expr::Binary {
                op: BinOp::LogicalOr,
                l,
                r,
                ..
            } => {
                open.push(&**r);
                open.push(&**l);
            }
            _ => count += 1,
        }
    }
    count
}

/// The arms with no statements joined into the first of them, up to [`JOINED`] constants an arm: each runs on to what follows the `switch`.
// ⚠ The tests of one reading are equalities of one value, read with nothing between them, and no
// two arms name one constant: which arm holds does not hang on the order they are asked in.
fn joined<'a>(arms: Vec<(Expr, &'a [Stmt])>) -> Vec<(Expr, &'a [Stmt])> {
    let mut out: Vec<(Expr, &'a [Stmt])> = Vec::with_capacity(arms.len());
    // The arm the next empty one joins, and the constants it holds so far.
    let mut open: Option<(usize, usize)> = None;
    for (test, body) in arms {
        if !body.is_empty() {
            out.push((test, body));
            continue;
        }
        let count = constants_of(&test);
        match open {
            Some((at, held)) if held + count <= JOINED && at < out.len() => {
                if let Some((first, _)) = out.get_mut(at) {
                    let l = std::mem::replace(first, Expr::GuidEmpty);
                    *first = Expr::Binary {
                        op: BinOp::LogicalOr,
                        at: Kind::Bool,
                        l: Box::new(l),
                        r: Box::new(test),
                    };
                }
                open = Some((at, held + count));
            }
            _ => {
                open = Some((out.len(), count));
                out.push((test, body));
            }
        }
    }
    out
}

/// [`read`] of the statements of `list` from `at`, where an `if` on a test starts them; of that `if` alone unless `whole`.
fn found(list: &[Stmt], at: usize, whole: bool) -> Option<Read> {
    let from = list.get(at..)?;
    let Some(Stmt::If { cond, then, els }) = from.first() else {
        return None;
    };
    test(cond)?;
    // What follows an `if` that leaves is where its test does not hold; what follows any
    // other is not the switch's.
    let mut looks = LOOKS;
    let alone = !(whole && els.is_empty() && leaves(then, &mut looks));
    read(match alone {
        true => from.get(..1)?,
        false => from,
    })
}

/// Reads the tests that start at statement `at` of `list` as a `switch` where they are one, in place of the statements it then stands for: the one `if`, or, where `whole`, as much of the list after it as the tests go through.
pub(super) fn switched(list: &mut Vec<Stmt>, at: usize, whole: bool) {
    let Some(read) = found(list, at, whole) else {
        return;
    };
    let end = (at + read.held).min(list.len());
    let after = list.split_off(end);
    list.truncate(at);
    list.push(Stmt::Sugar(Box::new(Sugar::SwitchOn {
        arms: read.arms,
        default: read.default,
    })));
    list.extend(after);
}

#[cfg(test)]
#[path = "sugar_switch_tests.rs"]
mod tests;
