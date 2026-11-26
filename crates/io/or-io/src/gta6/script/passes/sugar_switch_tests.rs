use super::*;
use crate::gta6::script::ir::{self, Callee, Var};
use crate::gta6::script::tree;

/// The value every test of these is of.
fn x() -> Expr {
    Expr::Var(Var::Param(1))
}

/// `x` read as 32 bits with the sign of `at`.
fn read_at(at: &Kind) -> Expr {
    let to = match at {
        Kind::I32 => Scalar::I32,
        _ => Scalar::U32,
    };
    Expr::cast(to, x())
}

/// `x op constant`, compared at `at`.
fn compare(op: BinOp, at: Kind, constant: i64) -> Expr {
    let r = Expr::Const(Const {
        bits: constant as u64,
        kind: at.clone(),
    });
    Expr::Binary {
        op,
        l: Box::new(read_at(&at)),
        at,
        r: Box::new(r),
    }
}

fn equal(constant: i64) -> Expr {
    compare(BinOp::Eq, Kind::U32, constant)
}

fn unequal(constant: i64) -> Expr {
    compare(BinOp::Ne, Kind::U32, constant)
}

/// A call of the function at `at`, which is how these tests tell one body from another.
fn does(at: u32) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args: Vec::new(),
        returns: Some(0),
    }))
}

fn when(cond: Expr, then: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
    Stmt::If { cond, then, els }
}

fn leave() -> Stmt {
    Stmt::Return(None)
}

fn case(labels: &[Option<i64>], body: Vec<Stmt>) -> Case {
    Case {
        labels: labels.to_vec(),
        body,
    }
}

/// `if (x == a) { func_a(); } else if (x == b) { func_b(); } ..`, ending in `default`.
fn chain(constants: &[i64], default: Vec<Stmt>) -> Vec<Stmt> {
    let mut els = default;
    for constant in constants.iter().rev() {
        els = vec![when(equal(*constant), vec![does(*constant as u32)], els)];
    }
    els
}

/// How a list of statements was left.
#[derive(PartialEq)]
enum Left {
    Fell,
    Returned,
    Broke,
}

/// What a test's expression comes to where `x` holds `x`.
fn value(e: &Expr, x: u64) -> u64 {
    match e {
        Expr::Var(_) => x,
        Expr::Const(c) => c.bits,
        Expr::Cast { to: Scalar::I32, e } => value(e, x) as u32 as i32 as i64 as u64,
        Expr::Cast { to: Scalar::U32, e } => u64::from(value(e, x) as u32),
        Expr::Binary {
            op: BinOp::LogicalOr,
            l,
            r,
            ..
        } => u64::from(value(l, x) != 0 || value(r, x) != 0),
        Expr::Binary { op, at, l, r } => {
            let (a, b) = (value(l, x) as u32, value(r, x) as u32);
            let ordering = match at {
                Kind::I32 => (a as i32).cmp(&(b as i32)),
                _ => a.cmp(&b),
            };
            u64::from(match op {
                BinOp::Eq => ordering.is_eq(),
                BinOp::Ne => ordering.is_ne(),
                BinOp::Lt => ordering.is_lt(),
                BinOp::Le => ordering.is_le(),
                BinOp::Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            })
        }
        other => panic!("no expression of these tests: {other:?}"),
    }
}

fn walk(body: &[Stmt], x: u64, calls: &mut Vec<u32>) -> Left {
    for stmt in body {
        match stmt {
            Stmt::Simple(ir::Stmt::Eval(Expr::Call {
                callee: Callee::Local { at, .. },
                ..
            })) => calls.push(*at),
            Stmt::If { cond, then, els } => {
                let taken = if value(cond, x) != 0 { then } else { els };
                match walk(taken, x, calls) {
                    Left::Fell => {}
                    left => return left,
                }
            }
            Stmt::Switch { value: on, cases } => {
                let on = i64::from(value(on, x) as u32 as i32);
                let named = cases
                    .iter()
                    .position(|case| case.labels.contains(&Some(on)));
                let default = || cases.iter().position(|case| case.labels.contains(&None));
                let start = named.or_else(default).unwrap_or(cases.len());
                for case in cases.iter().skip(start) {
                    match walk(&case.body, x, calls) {
                        Left::Fell => {}
                        Left::Broke => break,
                        Left::Returned => return Left::Returned,
                    }
                }
            }
            Stmt::Return(_) => return Left::Returned,
            Stmt::Break => return Left::Broke,
            other => panic!("no statement of these tests: {other:?}"),
        }
    }
    Left::Fell
}

/// The functions `body` calls where `x` holds `x`, in order, and whether it then returns.
fn calls(body: &[Stmt], x: u64) -> (Vec<u32>, bool) {
    let mut calls = Vec::new();
    let left = walk(&tree::desugar(body.to_vec()), x, &mut calls);
    (calls, left == Left::Returned)
}

/// `body` with the tests at its head read as a `switch`, held to run as `body` does for each of `values`.
fn read_whole(body: &[Stmt], values: &[u64]) -> Vec<Stmt> {
    let mut list = body.to_vec();
    switched(&mut list, 0, true);
    for x in values {
        assert_eq!(
            calls(&list, *x),
            calls(body, *x),
            "where the value is {x:#X}"
        );
    }
    list
}

/// The constants each arm of the `switch` `stmt` is tests, and its default.
fn arms_of(stmt: &Stmt) -> (Vec<Vec<u64>>, &[Stmt]) {
    let Stmt::Sugar(sugar) = stmt else {
        panic!("a construct: {stmt:?}");
    };
    let Sugar::SwitchOn { arms, default } = &**sugar else {
        panic!("a switch: {sugar:?}");
    };
    let constants = (arms.iter())
        .map(|arm| match test(&arm.test) {
            Some(Test {
                what: What::Equal(found),
                ..
            }) => found.iter().map(|(constant, _)| *constant).collect(),
            _ => Vec::new(),
        })
        .collect();
    (constants, default.as_slice())
}

/// Four cases as a compiler halves them: an ordering test read with its sign, and on each side two tests that leave where they do not hold.
fn halved(order: Kind) -> Vec<Stmt> {
    let low = vec![
        when(
            unequal(0x9000_0000),
            vec![
                when(compare(BinOp::Ne, Kind::I32, 20), vec![leave()], Vec::new()),
                does(0x20),
                leave(),
            ],
            Vec::new(),
        ),
        does(0x10),
        leave(),
    ];
    vec![
        when(compare(BinOp::Le, order, 20), low, Vec::new()),
        when(
            unequal(30),
            vec![
                when(unequal(40), vec![leave()], Vec::new()),
                does(0x40),
                leave(),
            ],
            Vec::new(),
        ),
        does(0x30),
    ]
}

/// Values on each side of every constant these tests use.
const VALUES: [u64; 14] = [
    0,
    1,
    2,
    3,
    10,
    19,
    20,
    21,
    30,
    40,
    41,
    50,
    0x9000_0000,
    0xFFFF_FFFB,
];

#[test]
fn cases_halved_by_an_ordering_test_are_one_switch() {
    let body = halved(Kind::I32);
    let done = read_whole(&body, &VALUES);
    let [only] = done.as_slice() else {
        panic!("one statement: {done:?}");
    };
    let (constants, default) = arms_of(only);
    let each: Vec<Vec<u64>> = vec![vec![0x9000_0000], vec![20], vec![30], vec![40]];
    assert_eq!(constants, each);
    assert_eq!(default, [leave()].as_slice());

    // Every arm is written as a test of the one value at the one type, whichever sign its own test read.
    let Stmt::Sugar(sugar) = only else {
        panic!("a construct: {only:?}");
    };
    let Sugar::SwitchOn { arms, .. } = &**sugar else {
        panic!("a switch: {sugar:?}");
    };
    for arm in arms {
        let spelt = test(&arm.test).map(|test| (test.value.clone(), test.at.clone()));
        assert_eq!(spelt, Some((read_at(&Kind::U32), Kind::U32)));
    }
    assert_eq!(arms[2].body, vec![does(0x30)]);
    assert_eq!(arms[3].body, vec![does(0x40), leave()]);
}

#[test]
fn a_switch_under_an_ordering_test_gives_its_cases_to_the_one_switch() {
    let small = Stmt::Switch {
        value: read_at(&Kind::U32),
        cases: vec![
            case(&[None], vec![leave()]),
            case(&[Some(10)], vec![does(0x10), leave()]),
            case(&[Some(-5), Some(20)], vec![does(0x20), leave()]),
        ],
    };
    let body = vec![
        when(compare(BinOp::Le, Kind::I32, 20), vec![small], Vec::new()),
        when(equal(30), vec![does(0x30), leave()], Vec::new()),
        when(unequal(40), vec![leave()], Vec::new()),
        does(0x40),
    ];
    let done = read_whole(&body, &VALUES);
    let [only] = done.as_slice() else {
        panic!("one statement: {done:?}");
    };
    let (constants, default) = arms_of(only);
    let each: Vec<Vec<u64>> = vec![vec![10], vec![0xFFFF_FFFB, 20], vec![30], vec![40]];
    assert_eq!(constants, each);
    assert_eq!(default, [leave()].as_slice());

    // A case that ends in a `break`, with nothing after its switch, is an arm that runs on.
    let broken = Stmt::Switch {
        value: read_at(&Kind::U32),
        cases: vec![
            case(&[Some(1)], vec![does(1), Stmt::Break]),
            case(&[Some(2)], vec![does(2), leave()]),
            case(&[None], vec![does(9)]),
        ],
    };
    let body = vec![when(equal(3), vec![does(3)], vec![broken]), does(0x99)];
    let done = read_whole(&body, &VALUES);
    let [only, after] = done.as_slice() else {
        panic!("two statements: {done:?}");
    };
    let (constants, default) = arms_of(only);
    let each: Vec<Vec<u64>> = vec![vec![3], vec![1], vec![2]];
    assert_eq!(constants, each);
    assert_eq!(default, [does(9)].as_slice());
    assert_eq!(*after, does(0x99));
}

#[test]
fn what_follows_tests_that_all_leave_stays_after_the_switch() {
    let guard = |constant: i64| {
        let body = vec![does(constant as u32), leave()];
        when(equal(constant), body, Vec::new())
    };
    let body = vec![guard(1), guard(2), guard(3), does(0x99), does(0x98)];
    let done = read_whole(&body, &VALUES);
    let [only, rest @ ..] = done.as_slice() else {
        panic!("a statement: {done:?}");
    };
    let (constants, default) = arms_of(only);
    let each: Vec<Vec<u64>> = vec![vec![1], vec![2], vec![3]];
    assert_eq!(constants, each);
    assert!(default.is_empty());
    assert_eq!(rest, [does(0x99), does(0x98)].as_slice());

    // Asked for the one `if` alone, the statements after it are no part of a switch.
    let mut alone = body.clone();
    switched(&mut alone, 0, false);
    assert_eq!(alone, body);

    // An arm reached where a test does not hold takes what follows as its body.
    let body = vec![
        guard(1),
        guard(2),
        when(unequal(3), vec![leave()], Vec::new()),
        does(3),
    ];
    let done = read_whole(&body, &VALUES);
    let [only] = done.as_slice() else {
        panic!("one statement: {done:?}");
    };
    let (constants, default) = arms_of(only);
    assert_eq!(constants, each);
    assert_eq!(default, [leave()].as_slice());
}

#[test]
fn tests_stay_where_one_switch_would_run_otherwise() {
    let inner_break = Stmt::Switch {
        value: read_at(&Kind::U32),
        cases: vec![
            case(&[None], vec![leave()]),
            case(
                &[Some(10)],
                vec![when(x(), vec![Stmt::Break], Vec::new()), leave()],
            ),
            case(&[Some(20)], vec![does(0x20), leave()]),
        ],
    };
    let far_label = Stmt::Switch {
        value: read_at(&Kind::U32),
        cases: vec![
            case(&[None], vec![leave()]),
            case(&[Some(0x1_0000_000A)], vec![does(0x10), leave()]),
            case(&[Some(20)], vec![does(0x20), leave()]),
        ],
    };
    let under = |small: Stmt| {
        vec![
            when(compare(BinOp::Le, Kind::I32, 20), vec![small], Vec::new()),
            when(equal(30), vec![does(0x30), leave()], Vec::new()),
            when(unequal(40), vec![leave()], Vec::new()),
            does(0x40),
        ]
    };
    let split =
        |low: Vec<Stmt>, high: Vec<Stmt>| vec![when(compare(BinOp::Le, Kind::I32, 20), low, high)];
    let stays = vec![
        (
            "an ordering test with cases on one side",
            vec![when(
                compare(BinOp::Lt, Kind::I32, 50),
                chain(&[1, 2, 3], Vec::new()),
                Vec::new(),
            )],
        ),
        (
            "a case no value that reaches its test takes",
            halved(Kind::U32),
        ),
        (
            "two bodies where no test holds",
            split(
                chain(&[10, 20], vec![does(0x98)]),
                chain(&[30, 40], vec![does(0x99)]),
            ),
        ),
        (
            "a case on the wrong side of the ordering test",
            split(chain(&[10, 30], Vec::new()), chain(&[40, 50], Vec::new())),
        ),
        (
            "a case that leaves its switch from inside",
            under(inner_break),
        ),
        ("a label no 32-bit value takes", under(far_label)),
        (
            "what follows is reached from both sides",
            vec![
                when(equal(1), vec![does(1)], Vec::new()),
                when(equal(2), vec![does(2)], Vec::new()),
                when(equal(3), vec![does(3)], Vec::new()),
            ],
        ),
    ];
    for (why, body) in stays {
        let mut list = body.clone();
        switched(&mut list, 0, true);
        assert_eq!(list, body, "{why}");
    }

    // Both sides of an ordering test with the cases of one value, and nothing where none holds.
    let body = split(chain(&[10, 20], Vec::new()), chain(&[30, 40], Vec::new()));
    let done = read_whole(&body, &VALUES);
    let (constants, default) = arms_of(&done[0]);
    let each: Vec<Vec<u64>> = vec![vec![10], vec![20], vec![30], vec![40]];
    assert_eq!((constants, default.len()), (each, 0));
}

#[test]
fn an_ordering_test_holds_of_a_constant_as_its_width_and_sign_say() {
    let order = |op: BinOp, signed: bool, wide: bool, bound: u64| Order {
        op,
        signed,
        wide,
        bound,
    };
    let minus_one = u64::from(u32::MAX);
    assert!(order(BinOp::Le, true, false, 20).holds(minus_one));
    assert!(!order(BinOp::Le, false, false, 20).holds(minus_one));
    assert!(order(BinOp::Gt, false, false, 20).holds(minus_one));
    assert!(order(BinOp::Lt, true, false, 20).holds(19));
    assert!(!order(BinOp::Lt, true, false, 20).holds(20));
    assert!(order(BinOp::Ge, true, false, 20).holds(20));
    // Whole cells: what is minus one in 32 bits is a large number in 64.
    assert!(!order(BinOp::Le, true, true, 20).holds(minus_one));
    assert!(order(BinOp::Le, true, true, 20).holds(u64::MAX));
    assert!(!order(BinOp::Le, false, true, 20).holds(u64::MAX));
}
