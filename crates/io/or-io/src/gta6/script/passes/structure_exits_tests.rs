use super::super::tests::{case, forever, not, p, returns, s, same_flow, when};
use super::super::tidy;
use super::*;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::Callee;

const L: Label = Label::Block(7);
const M: Label = Label::Block(8);

fn goes(label: Label) -> Stmt {
    Stmt::Goto(label)
}

/// `if (cond) { then }`.
fn only(cond: Expr, then: Vec<Stmt>) -> Stmt {
    when(cond, then, Vec::new())
}

/// The `goto`s that name each label of `body`.
fn counted(body: &[Stmt]) -> HashMap<Label, u32> {
    let mut named = HashMap::new();
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            match stmt {
                Stmt::Goto(label) => *named.entry(*label).or_insert(0) += 1,
                Stmt::If { then, els, .. } => {
                    work.push(then.as_slice());
                    work.push(els.as_slice());
                }
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    work.push(body.as_slice());
                }
                Stmt::Switch { cases, .. } => {
                    work.extend(cases.iter().map(|case| case.body.as_slice()));
                }
                _ => {}
            }
        }
    }
    named
}

/// The rules over `body`, with the tidy-up between rounds as the pass runs them: the result must be the same flow, and the count of each label's `goto`s must be what is left of them.
fn swept(body: Vec<Stmt>) -> Vec<Stmt> {
    let before = cfg::lower_body(body.clone()).expect("the body lowers");
    let mut named = counted(&body);
    let mut out = body;
    for _ in 0..4 {
        if !rewritten(&mut out, &mut named) {
            break;
        }
        tidy(&mut out, &mut named);
    }
    let after = cfg::lower_body(out.clone()).expect("the result lowers");
    if let Err(why) = same_flow(&before, &after) {
        panic!("the flow changed: {why}\n{out:#?}");
    }
    let left = counted(&out);
    for (label, count) in &named {
        assert_eq!(left.get(label).copied().unwrap_or(0), *count, "{label}");
    }
    out
}

fn gotos(body: &[Stmt]) -> u32 {
    counted(body).values().sum()
}

// ---- a jump over statements ---------------------------------------------------------------

#[test]
fn a_jump_over_the_statements_before_its_label_is_the_test_in_front_of_them() {
    // `if (p0) goto L; s1; s2; L: s3; return;`
    let body = vec![
        only(p(0), vec![goes(L)]),
        s(1),
        s(2),
        Stmt::Label(L),
        s(3),
        returns(),
    ];
    assert_eq!(
        swept(body),
        [only(not(p(0)), vec![s(1), s(2)]), s(3), returns()]
    );
}

#[test]
fn an_arm_that_ends_in_the_jump_takes_what_it_passed_over_as_its_else() {
    // `if (p0) { s1; goto L; } s2; L: s3; return;`
    let body = vec![
        only(p(0), vec![s(1), goes(L)]),
        s(2),
        Stmt::Label(L),
        s(3),
        returns(),
    ];
    assert_eq!(
        swept(body),
        [when(p(0), vec![s(1)], vec![s(2)]), s(3), returns()]
    );
}

#[test]
fn a_label_after_the_enclosing_if_is_where_its_arm_runs_on_to() {
    // `if (p0) { if (p1) goto L; s1; s2; } L: s3; return;`
    let arm = vec![only(p(1), vec![goes(L)]), s(1), s(2)];
    let body = vec![only(p(0), arm), Stmt::Label(L), s(3), returns()];
    let nested = only(not(p(1)), vec![s(1), s(2)]);
    assert_eq!(swept(body), [only(p(0), vec![nested]), s(3), returns()]);
}

#[test]
fn a_jump_is_not_nested_past_another_label_or_beside_an_else_that_leaves() {
    // `M` stands between the jump and `L`: as an else, `s1` would take `M` with it or stop short of `L`.
    let crossed = vec![
        only(p(0), vec![goes(L)]),
        s(1),
        Stmt::Label(M),
        s(2),
        only(p(1), vec![goes(M)]),
        Stmt::Label(L),
        s(3),
        returns(),
    ];
    assert_eq!(swept(crossed.clone()), crossed);
    // The else returns, so nothing ran on into `s1`.
    let left = vec![
        when(p(0), vec![goes(L)], vec![returns()]),
        s(1),
        Stmt::Label(L),
        s(2),
        returns(),
    ];
    assert_eq!(swept(left.clone()), left);
}

#[test]
fn a_jump_to_where_its_arm_runs_on_to_says_nothing() {
    // `if (p0) { s1; if (p1) { s2; goto L; } } L: s3; return;`
    let inner = only(p(1), vec![s(2), goes(L)]);
    let body = vec![
        only(p(0), vec![s(1), inner]),
        Stmt::Label(L),
        s(3),
        returns(),
    ];
    let bare = only(p(1), vec![s(2)]);
    assert_eq!(swept(body), [only(p(0), vec![s(1), bare]), s(3), returns()]);
}

// ---- break and continue -------------------------------------------------------------------

#[test]
fn a_jump_to_the_statement_after_its_loop_is_a_break() {
    // `while (p0) { s1; if (p1) { s2; goto L; } s3; } L: s4; return;`
    let lp = |way: Stmt| Stmt::While {
        cond: p(0),
        body: vec![s(1), only(p(1), vec![s(2), way]), s(3)],
    };
    let body = vec![lp(goes(L)), Stmt::Label(L), s(4), returns()];
    assert_eq!(swept(body), [lp(Stmt::Break), s(4), returns()]);
}

#[test]
fn a_jump_to_the_end_of_its_loops_body_is_a_continue() {
    // `while (p0) { if (p1) { s1; if (p2) goto M; s2; } s3; M: } s4; return;`
    let lp = |way: Stmt, end: Vec<Stmt>| {
        let arm = vec![s(1), only(p(2), vec![way]), s(2)];
        let mut body = vec![only(p(1), arm), s(3)];
        body.extend(end);
        Stmt::While { cond: p(0), body }
    };
    let body = vec![lp(goes(M), vec![Stmt::Label(M)]), s(4), returns()];
    assert_eq!(
        swept(body),
        [lp(Stmt::Continue, Vec::new()), s(4), returns()]
    );
}

#[test]
fn a_jump_out_of_a_case_to_what_follows_the_switch_is_a_break() {
    // `switch (p0) { case 0: s1; if (p1) { s5; goto L; } s2; break; case 1: s3; break; } L: s4; return;`
    let switch = |way: Stmt| Stmt::Switch {
        value: p(0),
        cases: vec![
            case(
                &[Some(0)],
                vec![s(1), only(p(1), vec![s(5), way]), s(2), Stmt::Break],
            ),
            case(&[Some(1)], vec![s(3), Stmt::Break]),
        ],
    };
    let body = vec![switch(goes(L)), Stmt::Label(L), s(4), returns()];
    assert_eq!(swept(body), [switch(Stmt::Break), s(4), returns()]);
}

// ---- a guard that returns -----------------------------------------------------------------

#[test]
fn a_return_an_arm_runs_on_to_is_written_in_front_of_the_arm() {
    // `if (p0) { s1; if (p1) goto L; } return; L: s2; return;`
    let arm = vec![s(1), only(p(1), vec![goes(L)])];
    let body = vec![only(p(0), arm), returns(), Stmt::Label(L), s(2), returns()];
    let guard = |n: u32| only(not(p(n)), vec![returns()]);
    assert_eq!(swept(body), [guard(0), s(1), guard(1), s(2), returns()]);
}

#[test]
fn guards_are_written_for_each_arm_the_jump_leaves() {
    // `if (p0) { s1; if (p1) { s2; if (p2) goto L; } } return; L: s3; return;`
    let inner = vec![s(2), only(p(2), vec![goes(L)])];
    let arm = vec![s(1), only(p(1), inner)];
    let body = vec![only(p(0), arm), returns(), Stmt::Label(L), s(3), returns()];
    let guard = |n: u32| only(not(p(n)), vec![returns()]);
    assert_eq!(
        swept(body),
        [guard(0), s(1), guard(1), s(2), guard(2), s(3), returns()]
    );
}

#[test]
fn the_return_behind_an_arm_that_never_runs_off_goes() {
    // `if (p0) { if (p1) goto L; s1; return; } return p9; L: s2; return p9;`
    let arm = vec![only(p(1), vec![goes(L)]), s(1), Stmt::Return(None)];
    let body = vec![only(p(0), arm), returns(), Stmt::Label(L), s(2), returns()];
    assert_eq!(
        swept(body),
        [
            only(not(p(0)), vec![returns()]),
            only(not(p(1)), vec![s(1), Stmt::Return(None)]),
            s(2),
            returns()
        ]
    );
}

#[test]
fn a_return_that_calls_is_not_written_twice() {
    let call = Expr::Call {
        callee: Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        },
        args: Vec::new(),
        returns: None,
    };
    let arm = vec![s(1), only(p(1), vec![goes(L)])];
    let body = vec![
        only(p(0), arm),
        Stmt::Return(Some(call)),
        Stmt::Label(L),
        s(2),
        returns(),
    ];
    assert_eq!(swept(body.clone()), body);
}

#[test]
fn an_arm_is_hoisted_only_where_a_jump_of_it_then_goes() {
    assert!(leaves_to(&[s(1), goes(L)], &[L]));
    assert!(leaves_to(&[only(p(0), vec![goes(L)]), s(1)], &[L]));
    assert!(leaves_to(&[s(1), only(p(0), vec![s(2), goes(L)])], &[M, L]));
    assert!(!leaves_to(&[s(1), goes(L)], &[M]));
    assert!(!leaves_to(&[s(1)], &[L]));
    assert!(!leaves_to(&[goes(L)], &[]));
    // A label after the `if`: what follows it cannot become its else.
    let labelled = [only(p(0), vec![goes(L)]), Stmt::Label(M), s(1)];
    assert!(!leaves_to(&labelled, &[L]));
    // Beside an else that returns, nothing ran on to the `return` being copied.
    let beside = [when(p(0), vec![goes(L)], vec![returns()])];
    assert!(!leaves_to(&beside, &[L]));
    // No `goto` in the arm at all: the body stays as it reads.
    let body = vec![
        only(p(0), vec![s(1), only(p(1), vec![s(2)])]),
        returns(),
        Stmt::Label(L),
        s(3),
        only(p(2), vec![goes(L)]),
        returns(),
    ];
    assert_eq!(swept(body.clone()), body);
}

// ---- a loop and what follows it -----------------------------------------------------------

#[test]
fn a_loop_left_by_one_break_takes_in_what_follows_it() {
    // `s0; while (true) { if (p0) goto L; s1; if (p1) break; s2; } s3; L: s4; return;`
    let body = vec![
        s(0),
        Stmt::While {
            cond: forever(),
            body: vec![
                only(p(0), vec![goes(L)]),
                s(1),
                only(p(1), vec![Stmt::Break]),
                s(2),
            ],
        },
        s(3),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    // The jump is the loop's `break`, which as its first statement is its test.
    let lp = Stmt::While {
        cond: not(p(0)),
        body: vec![s(1), only(p(1), vec![s(3), Stmt::Break]), s(2)],
    };
    assert_eq!(swept(body), [s(0), lp, s(4), returns()]);
}

#[test]
fn a_loop_keeps_out_a_tail_that_would_leave_it_or_that_two_breaks_lead_to() {
    let lp = |ways: Vec<Stmt>| {
        let mut body = vec![only(p(0), vec![goes(L)]), s(1)];
        body.extend(ways);
        body.push(s(2));
        Stmt::While {
            cond: forever(),
            body,
        }
    };
    let one = only(p(1), vec![Stmt::Break]);
    // Two ways to the tail: at either, the other would need the tail too.
    let twice = vec![
        lp(vec![one.clone(), only(p(2), vec![Stmt::Break])]),
        s(3),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    assert_eq!(swept(twice.clone()), twice);
    // The tail's `break` leaves the outer loop; inside the inner one it would leave that.
    let outer = vec![
        Stmt::While {
            cond: p(3),
            body: vec![lp(vec![one]), s(3), Stmt::Break, Stmt::Label(L), s(4)],
        },
        returns(),
    ];
    assert_eq!(swept(outer.clone()), outer);
}

#[test]
fn a_loop_tested_at_its_bottom_takes_in_what_follows_its_test() {
    // `do { s1; if (p0) goto L; s2; } while (p1); s3; L: s4; return;`
    let body = vec![
        Stmt::DoWhile {
            body: vec![s(1), only(p(0), vec![goes(L)]), s(2)],
            cond: p(1),
        },
        s(3),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    let lp = Stmt::While {
        cond: forever(),
        body: vec![
            s(1),
            only(p(0), vec![Stmt::Break]),
            s(2),
            only(not(p(1)), vec![s(3), Stmt::Break]),
        ],
    };
    let out = swept(body);
    assert_eq!(out, [lp, s(4), returns()]);
    assert_eq!(gotos(&out), 0);
}

#[test]
fn a_do_that_continues_or_breaks_keeps_its_test() {
    for way in [Stmt::Continue, Stmt::Break] {
        // A `continue` goes to the test and a `break` lands before `s3`: neither does in `while (true)`.
        let body = vec![
            Stmt::DoWhile {
                body: vec![s(1), only(p(0), vec![goes(L)]), only(p(2), vec![way]), s(2)],
                cond: p(1),
            },
            s(3),
            Stmt::Label(L),
            s(4),
            returns(),
        ];
        assert_eq!(swept(body.clone()), body);
    }
}

#[test]
fn what_a_list_holds_free_is_told_from_what_a_loop_or_a_switch_of_it_takes() {
    let inner = Stmt::While {
        cond: p(0),
        body: vec![Stmt::Break, Stmt::Continue, goes(L)],
    };
    let switch = Stmt::Switch {
        value: p(1),
        cases: vec![case(&[Some(0)], vec![Stmt::Break, Stmt::Continue, goes(L)])],
    };
    let list = [
        only(p(2), vec![Stmt::Break, goes(L), goes(M)]),
        inner,
        switch,
        Stmt::Continue,
    ];
    let found = free(&list, &[L]);
    assert_eq!(
        (found.breaks, found.continues, found.jumps, found.opaque),
        (1, 2, 1, false)
    );
    assert_eq!(deepest(&list), 1);
    assert_eq!(deepest(&[s(1)]), 0);
}
