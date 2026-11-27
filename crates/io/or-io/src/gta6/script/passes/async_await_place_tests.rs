use super::*;
use crate::gta6::script::ir::{Scalar, StoreKind};
use crate::gta6::script::tree::Label;

// ---- what the tests build with ------------------------------------------------------------

/// A call of the function of the program at `at`.
fn call(at: u32, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args,
        returns: Some(1),
    }
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn simple(stmt: Stmt) -> tree::Stmt {
    tree::Stmt::Simple(stmt)
}

fn assign(n: u32, value: Expr) -> Stmt {
    Stmt::Assign {
        target: Var::Local(n),
        value,
    }
}

/// The task `local_3` is given: a call of a function given `p0`.
fn making() -> Stmt {
    assign(3, call(0x100, vec![Expr::Var(Var::Param(0))]))
}

fn made() -> tree::Stmt {
    simple(making())
}

/// An await of `local_3` that takes no awaiter: the test asks it, the suspend path keeps it in the machine and leaves, the resume point takes it back; `after` follows.
fn awaiting(after: Vec<Stmt>) -> tree::Stmt {
    let machine = || Expr::Var(Var::Param(1));
    tree::Stmt::Sugar(Box::new(Sugar::Await {
        before: Vec::new(),
        ready: Expr::Truth(Box::new(call(0x104, vec![local(3)]))),
        suspend: vec![
            simple(Stmt::Store {
                kind: StoreKind::U64,
                addr: machine(),
                offset: 0x20,
                value: local(3),
            }),
            simple(Stmt::Eval(call(
                0x300,
                vec![Expr::AddrOf {
                    var: Var::Local(3),
                    cell: 0,
                }],
            ))),
            tree::Stmt::Return(None),
        ],
        resumed: Label::Block(0),
        resume: vec![simple(assign(
            3,
            Expr::Load {
                ty: Scalar::U64,
                addr: Box::new(machine()),
                offset: 0x20,
            },
        ))],
        after,
    }))
}

/// The statement that asks `local_3` for its result: a call given it alone.
fn result() -> Expr {
    call(0x108, vec![local(3)])
}

/// A statement of the method that reads `local_N`.
fn used(n: u32) -> tree::Stmt {
    simple(Stmt::Eval(call(0x200, vec![local(n)])))
}

/// The statements that take each await's awaiter, at the top of a body.
fn takings(body: &[tree::Stmt]) -> Vec<Vec<Stmt>> {
    body.iter()
        .filter_map(|stmt| match stmt {
            tree::Stmt::Sugar(sugar) => match &**sugar {
                Sugar::Await { before, .. } => Some(before.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Asserts that `tasks` leaves `body` as it is.
fn left(body: Vec<tree::Stmt>) {
    let mut done = body.clone();
    tasks(&mut done);
    assert_eq!(done, body);
}

// ---- the task taken into its await ------------------------------------------------------------

#[test]
fn the_statement_that_makes_the_task_an_await_takes_is_the_awaits_own() {
    for after in [
        Vec::new(),
        vec![Stmt::Eval(result())],
        vec![assign(5, result())],
    ] {
        let body = vec![used(1), made(), awaiting(after), used(1)];
        let mut done = body.clone();
        tasks(&mut done);
        assert_eq!(done.len(), 3, "{done:#?}");
        assert_eq!(takings(&done), vec![vec![making()]], "{done:#?}");
        // It runs where it ran: first of what the await lowers to, right after what came before.
        assert_eq!(tree::desugar(done), tree::desugar(body));
    }
}

#[test]
fn a_task_named_where_it_can_be_seen_stays_where_it_is_made() {
    // Read after the await by the method.
    left(vec![made(), awaiting(Vec::new()), used(3)]);
    // Read before the await, in the statement that makes it.
    let again = simple(assign(3, call(0x100, vec![local(3)])));
    left(vec![again, awaiting(Vec::new())]);
    // Given another value elsewhere.
    left(vec![made(), awaiting(Vec::new()), made()]);
    // Its address handed on by the method.
    let handed = simple(Stmt::Eval(call(
        0x200,
        vec![Expr::AddrOf {
            var: Var::Local(3),
            cell: 0,
        }],
    )));
    left(vec![handed, made(), awaiting(Vec::new())]);
    // ⚠ A store of the result keeps the task in sight: written as it is, the store names it.
    let stored = Stmt::Store {
        kind: StoreKind::U64,
        addr: Expr::Var(Var::Param(1)),
        offset: 0x30,
        value: result(),
    };
    left(vec![made(), awaiting(vec![stored])]);
    // A statement of `after` that reads it and asks for no result.
    let read = Stmt::Eval(call(0x200, vec![local(3), local(1)]));
    left(vec![made(), awaiting(vec![read])]);
    // A statement between.
    left(vec![made(), used(1), awaiting(Vec::new())]);
    // The variable assigned is not the awaiter the test asks.
    left(vec![
        simple(assign(4, call(0x100, Vec::new()))),
        awaiting(Vec::new()),
    ]);
}

#[test]
fn an_await_that_takes_its_awaiter_already_takes_nothing_more() {
    let mut body = vec![made(), awaiting(Vec::new())];
    if let tree::Stmt::Sugar(sugar) = &mut body[1] {
        if let Sugar::Await { before, .. } = &mut **sugar {
            before.push(assign(9, local(1)));
        }
    }
    left(body);
}

#[test]
fn an_await_written_twice_takes_the_task_into_neither() {
    // What `structure` does to a short tail: the same await at two places reads the task twice.
    left(vec![made(), awaiting(Vec::new()), awaiting(Vec::new())]);
}

#[test]
fn one_awaiter_of_several_awaits_takes_each_its_task_or_none() {
    let body = vec![
        made(),
        awaiting(Vec::new()),
        used(1),
        made(),
        awaiting(vec![Stmt::Eval(result())]),
    ];
    let mut done = body.clone();
    tasks(&mut done);
    assert_eq!(done.len(), 3, "{done:#?}");
    assert_eq!(takings(&done), vec![vec![making()], vec![making()]]);
    assert_eq!(tree::desugar(done), tree::desugar(body));
    // ⚠ One of them left in sight would read, after the other, a value given out of sight.
    let stored = Stmt::Store {
        kind: StoreKind::U64,
        addr: Expr::Var(Var::Param(1)),
        offset: 0x30,
        value: result(),
    };
    left(vec![
        made(),
        awaiting(Vec::new()),
        used(1),
        made(),
        awaiting(vec![stored]),
    ]);
}

#[test]
fn an_await_inside_a_construct_takes_its_task_too() {
    let body = vec![tree::Stmt::If {
        cond: local(1),
        then: vec![made(), awaiting(Vec::new())],
        els: Vec::new(),
    }];
    let mut done = body.clone();
    tasks(&mut done);
    let tree::Stmt::If { then, .. } = &done[0] else {
        panic!("the `if` stays: {done:#?}");
    };
    assert_eq!(then.len(), 1, "{done:#?}");
    assert_eq!(tree::desugar(done), tree::desugar(body));
}

// ---- how an await reads ----------------------------------------------------------------------------

#[test]
fn an_await_reads_as_the_task_it_takes_and_what_it_makes_of_the_result() {
    let task = call(0x100, vec![Expr::Var(Var::Param(0))]);
    for (after, taken) in [
        (Vec::new(), Taken::Nothing),
        (vec![Stmt::Eval(result())], Taken::Dropped),
        (vec![assign(5, result())], Taken::Assigned(Var::Local(5))),
    ] {
        let mut body = vec![made(), awaiting(after)];
        tasks(&mut body);
        let tree::Stmt::Sugar(sugar) = &body[0] else {
            panic!("no await: {body:#?}");
        };
        let read = reading(sugar).expect("an await");
        assert_eq!(read.awaited, &task);
        assert!(read.shown.is_empty());
        assert_eq!(read.taken, taken);
    }
    // One that takes no awaiter awaits the awaiter its test asks.
    let stored = Stmt::Store {
        kind: StoreKind::U64,
        addr: Expr::Var(Var::Param(1)),
        offset: 0x30,
        value: result(),
    };
    let alone = awaiting(vec![stored.clone()]);
    let tree::Stmt::Sugar(sugar) = &alone else {
        panic!("no await");
    };
    let read = reading(sugar).expect("an await");
    assert_eq!(read.awaited, &local(3));
    assert_eq!(read.taken, Taken::Stored(&stored));
    // ⚠ A statement given the awaiter and more asks for no result.
    let more = awaiting(vec![Stmt::Eval(call(0x108, vec![local(3), local(1)]))]);
    let tree::Stmt::Sugar(sugar) = &more else {
        panic!("no await");
    };
    assert_eq!(reading(sugar).map(|read| read.taken), Some(Taken::Nothing));
    // What is no await has no reading.
    assert_eq!(reading(&Sugar::Hidden(Vec::new())), None);
}

#[test]
fn a_statement_before_the_one_that_takes_the_awaiter_stays_in_sight() {
    let first = assign(1, call(0x10C, Vec::new()));
    let taking = assign(3, call(0x100, vec![local(1)]));
    let mut held = awaiting(Vec::new());
    if let tree::Stmt::Sugar(sugar) = &mut held {
        if let Sugar::Await { before, .. } = &mut **sugar {
            before.extend([first.clone(), taking]);
        }
    }
    let tree::Stmt::Sugar(sugar) = &held else {
        panic!("no await");
    };
    let read = reading(sugar).expect("an await");
    assert_eq!(read.shown, std::slice::from_ref(&first));
    assert_eq!(read.awaited, &call(0x100, vec![local(1)]));
}
