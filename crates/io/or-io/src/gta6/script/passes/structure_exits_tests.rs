use super::super::tests::{case, forever, not, p, returns, s, same_flow, when};
use super::super::tidy;
use super::*;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Returns, VarInfo};

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
        if !rewritten(&mut out, &mut named, None) {
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
fn a_jump_to_where_the_break_after_its_switch_lands_is_the_switchs_break() {
    // `while (p0) { switch (p1) { case 0: s1; goto L; } break; } L: s2; return;`
    let lp = |way: Stmt| Stmt::While {
        cond: p(0),
        body: vec![
            Stmt::Switch {
                value: p(1),
                cases: vec![case(&[Some(0)], vec![s(1), way])],
            },
            Stmt::Break,
        ],
    };
    let body = vec![lp(goes(L)), Stmt::Label(L), s(2), returns()];
    let out = swept(body);
    assert_eq!(out, [lp(Stmt::Break), s(2), returns()]);
    assert_eq!(gotos(&out), 0);
}

#[test]
fn an_arm_that_runs_on_to_a_break_landing_at_its_label_needs_no_jump() {
    // `while (p0) { if (p1) { s1; goto L; } break; } L: s2; return;`
    let lp = |arm: Vec<Stmt>| Stmt::While {
        cond: p(0),
        body: vec![only(p(1), arm), Stmt::Break],
    };
    let body = vec![lp(vec![s(1), goes(L)]), Stmt::Label(L), s(2), returns()];
    assert_eq!(swept(body), [lp(vec![s(1)]), s(2), returns()]);
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

// ---- a constant set after a construct whose jumps past it set the variable first ----------

/// A `stack_` variable, one cell.
const T: Var = Var::Stack { at: 0x40, index: 0 };

fn give(var: Var, value: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Assign { target: var, value })
}

fn k(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

/// A head that declares each variable one cell wide, proved alone in its cells or not.
fn head_of(vars: &[(Var, Width, bool)]) -> Head {
    Head {
        value_types: Default::default(),
        program: 1,
        at: 0x10,
        name: "func_000010".to_owned(),
        returns: Returns::Cell,
        params: 10,
        locals: 100,
        vars: vars
            .iter()
            .map(|(var, width, single)| {
                let info = VarInfo {
                    single: *single,
                    ..VarInfo::new(*width)
                };
                (*var, info)
            })
            .collect(),
        incomplete: None,
    }
}

/// `f(&var);`: a call handed the address of `var`.
fn handing(var: Var) -> ir::Stmt {
    let call = Expr::Call {
        callee: Callee::Local {
            at: 0x200,
            name: "func_000200".to_owned(),
        },
        args: vec![Expr::AddrOf { var, cell: 0 }],
        returns: None,
    };
    ir::Stmt::Eval(call)
}

fn handed(var: Var) -> Stmt {
    Stmt::Simple(handing(var))
}

/// The rules as the pass runs them, `head` saying which assignments may move: the count of each label's `goto`s must be what is left of them.
fn moved(body: Vec<Stmt>, head: &Head) -> Vec<Stmt> {
    moved_beside(body, head, &HashSet::new())
}

/// [`moved`], with `alone` the locals the blocks show no address reaches.
fn moved_beside(body: Vec<Stmt>, head: &Head, alone: &HashSet<Var>) -> Vec<Stmt> {
    let mut named = counted(&body);
    let mut out = body;
    for _ in 0..4 {
        let private = Private::of(head, &out, alone);
        if !rewritten(&mut out, &mut named, Some(&private)) {
            break;
        }
        tidy(&mut out, &mut named);
    }
    let left = counted(&out);
    for (label, count) in &named {
        assert_eq!(left.get(label).copied().unwrap_or(0), *count, "{label}");
    }
    out
}

/// `while (p1) { s1; if (p2) { var = p3; <way> } }`.
fn search(var: Var, way: Stmt) -> Stmt {
    Stmt::While {
        cond: p(1),
        body: vec![s(1), only(p(2), vec![give(var, p(3)), way])],
    }
}

#[test]
fn a_constant_set_after_a_loop_whose_jumps_past_it_set_it_first_is_set_before_the_loop() {
    // `while (p1) { s1; if (p2) { t = p3; goto L; } } t = -1; L: s4; return;`
    let body = vec![
        search(T, goes(L)),
        give(T, k(-1)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    let head = head_of(&[(T, Width::Cell, false)]);
    let out = moved(body, &head);
    assert_eq!(
        out,
        [give(T, k(-1)), search(T, Stmt::Break), s(4), returns()]
    );
    assert_eq!(gotos(&out), 0);
}

#[test]
fn a_constant_set_after_an_if_is_set_before_it_and_the_jumps_run_off_its_arms() {
    // `if (p0) { s1; if (p2) { t = p3; goto L; } } else { t = p5; goto L; } t = 0; L: s4; return;`
    let then = vec![s(1), only(p(2), vec![give(T, p(3)), goes(L)])];
    let els = vec![give(T, p(5)), goes(L)];
    let body = vec![
        when(p(0), then, els),
        give(T, k(0)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    let head = head_of(&[(T, Width::Cell, false)]);
    let out = moved(body, &head);
    let then = vec![s(1), only(p(2), vec![give(T, p(3))])];
    assert_eq!(
        out,
        [
            give(T, k(0)),
            when(p(0), then, vec![give(T, p(5))]),
            s(4),
            returns()
        ]
    );
}

#[test]
fn a_constant_stays_behind_a_construct_a_way_out_of_which_does_not_set_it() {
    let head = head_of(&[(T, Width::Cell, false)]);
    // A jump past it that sets nothing: `t` keeps what it held before. (`s1` keeps the loop's
    // test from being tested again after it.)
    let unset = vec![
        Stmt::While {
            cond: p(1),
            body: vec![
                s(1),
                only(p(2), vec![give(T, p(3)), goes(L)]),
                only(p(5), vec![goes(L)]),
            ],
        },
        give(T, k(-1)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    assert_eq!(moved(unset.clone(), &head), unset);
    // An assignment that runs on: the constant must still come after it.
    let runs_on = vec![
        Stmt::While {
            cond: p(1),
            body: vec![
                s(1),
                give(T, p(6)),
                only(p(2), vec![give(T, p(3)), goes(L)]),
            ],
        },
        give(T, k(-1)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    assert_eq!(moved(runs_on.clone(), &head), runs_on);
    // A test of the loop reads it.
    let read = vec![
        Stmt::While {
            cond: Expr::Var(T),
            body: vec![s(1), only(p(2), vec![give(T, p(3)), goes(L)])],
        },
        give(T, k(-1)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    assert_eq!(moved(read.clone(), &head), read);
}

#[test]
fn a_constant_stays_behind_a_construct_entered_or_left_another_way() {
    let head = head_of(&[(T, Width::Cell, false)]);
    // A `break` of the loop around the `if` leaves past the constant to where it is not set: the
    // constant is not set in front of the `if`, but written at the end of each of its arms.
    let arm = vec![
        only(p(2), vec![give(T, p(3)), goes(L)]),
        only(p(5), vec![Stmt::Break]),
    ];
    let left = vec![
        Stmt::While {
            cond: p(0),
            body: vec![only(p(1), arm), give(T, k(-1)), Stmt::Label(L), s(4)],
        },
        returns(),
    ];
    let unset = vec![only(p(5), vec![Stmt::Break]), give(T, k(-1))];
    let arm = vec![when(p(2), vec![give(T, p(3))], unset)];
    let kept = vec![
        Stmt::While {
            cond: p(0),
            body: vec![when(p(1), arm, vec![give(T, k(-1))]), s(4)],
        },
        returns(),
    ];
    assert_eq!(moved(left, &head), kept);
    // A jump from before the loop enters it at `M`, and may then run off it to the constant.
    let entered = vec![
        only(p(2), vec![goes(M)]),
        s(1),
        Stmt::While {
            cond: p(1),
            body: vec![
                Stmt::Label(M),
                s(5),
                only(p(6), vec![give(T, p(3)), goes(L)]),
            ],
        },
        give(T, k(-1)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    assert_eq!(moved(entered.clone(), &head), entered);
}

#[test]
fn a_constant_of_a_variable_an_address_may_reach_stays_where_it_is() {
    let body = |var: Var, extra: Vec<Stmt>| {
        let mut body = vec![search(var, goes(L)), give(var, k(-1)), Stmt::Label(L), s(4)];
        body.extend(extra);
        body.push(returns());
        body
    };
    // The temporary's own address is handed out.
    let head = head_of(&[(T, Width::Cell, false)]);
    let handed_out = body(T, vec![handed(T)]);
    assert_eq!(moved(handed_out.clone(), &head), handed_out);
    // A local of one cell moves only while it is proved alone and no frame address is taken.
    let local = Var::Local(30);
    let alone = head_of(&[(local, Width::Cell, true)]);
    let out = moved(body(local, Vec::new()), &alone);
    assert_eq!(out[0], give(local, k(-1)));
    let unproved = head_of(&[(local, Width::Cell, false)]);
    assert_eq!(
        moved(body(local, Vec::new()), &unproved),
        body(local, Vec::new())
    );
    let framed = body(local, vec![handed(Var::Local(29))]);
    assert_eq!(moved(framed.clone(), &alone), framed);
}

#[test]
fn only_a_variable_of_one_cell_no_address_reaches_is_private() {
    let (u, wide, local, loose) = (
        Var::Stack { at: 0x44, index: 0 },
        Var::Stack { at: 0x48, index: 0 },
        Var::Local(20),
        Var::Local(21),
    );
    let head = head_of(&[
        (T, Width::Cell, false),
        (u, Width::Cell, false),
        (wide, Width::Cells, false),
        (local, Width::Cell, true),
        (loose, Width::Cell, false),
    ]);
    let none = HashSet::new();
    let private = Private::of(&head, &[handed(u)], &none);
    let mut vars: Vec<Var> = private.vars.iter().copied().collect();
    vars.sort_unstable();
    let mut expected = vec![T, local];
    expected.sort_unstable();
    assert_eq!(vars, expected);
    // The address of any frame cell may reach the local, unless the blocks show it reaches nothing of it.
    let framed = Private::of(&head, &[handed(Var::Param(1))], &none);
    assert!(framed.vars.contains(&T) && !framed.vars.contains(&local));
    let shown = HashSet::from([local, loose]);
    let apart = Private::of(&head, &[handed(Var::Param(1))], &shown);
    assert!(apart.vars.contains(&local));
    // Shown alone in what it reaches, a local still needs to be proved alone in its cells.
    assert!(!apart.vars.contains(&loose));
    // A piece of a state machine out of sight may enter anywhere: nothing moves.
    let piece = Expr::Call {
        callee: Callee::Intrinsic {
            name: PIECE.to_owned(),
            ty: None,
            wrapper: None,
        },
        args: vec![k(0)],
        returns: None,
    };
    let held = Private::of(&head, &[Stmt::Simple(ir::Stmt::Eval(piece))], &shown);
    assert!(held.vars.is_empty());
}

#[test]
fn a_local_the_blocks_show_no_address_reaches_is_set_before_the_loop_beside_a_frame_address() {
    let local = Var::Local(30);
    let head = head_of(&[(local, Width::Cell, true)]);
    // `while (p1) { s1; if (p2) { local_30 = p3; goto L; } } local_30 = -1; L: s4; f(&local_29); return;`
    let body = vec![
        search(local, goes(L)),
        give(local, k(-1)),
        Stmt::Label(L),
        s(4),
        handed(Var::Local(29)),
        returns(),
    ];
    let out = moved_beside(body.clone(), &head, &HashSet::from([local]));
    assert_eq!(
        out,
        [
            give(local, k(-1)),
            search(local, Stmt::Break),
            s(4),
            handed(Var::Local(29)),
            returns()
        ]
    );
    assert_eq!(moved_beside(body.clone(), &head, &HashSet::new()), body);
}

#[test]
fn a_local_is_alone_only_where_no_address_handed_out_reaches_it() {
    let (low, held, high) = (Var::Local(28), Var::Local(29), Var::Local(30));
    let blocks = vec![cfg::Block {
        at: None,
        stmts: vec![handing(held)],
        term: cfg::Terminator::Return(None),
    }];
    let of = |vars: &[(Var, Width, bool)]| cfg::Function {
        head: head_of(vars),
        cfg: cfg::Cfg {
            blocks: blocks.clone(),
        },
    };
    // One cell wide, the local handed out ends where it starts.
    let bounded = of(&[
        (low, Width::Cell, true),
        (held, Width::Cell, true),
        (high, Width::Cell, true),
    ]);
    assert_eq!(alone(&bounded), HashSet::from([low, high]));
    // Nothing bounds it: every local from its own on may be reached.
    let open = of(&[
        (low, Width::Cell, true),
        (held, Width::Cell, false),
        (high, Width::Cell, false),
    ]);
    assert_eq!(alone(&open), HashSet::from([low]));
}

// ---- the else a loop's body ends in, and a switch's last body -------------------------------

/// `while (p0) { s1; if (p1) { <then> } else { <els> } <rest> }`.
fn looped(then: Vec<Stmt>, els: Vec<Stmt>, rest: Vec<Stmt>) -> Stmt {
    let mut body = vec![s(1), when(p(1), then, els)];
    body.extend(rest);
    Stmt::While { cond: p(0), body }
}

#[test]
fn a_label_in_the_else_that_ends_a_loops_body_comes_out_after_the_if() {
    // `while (p0) { s1; if (p1) { if (p2) { s2; goto L; } s3; } else { s4; L: s5; } } return;`
    let then = vec![only(p(2), vec![s(2), goes(L)]), s(3)];
    let body = vec![
        looped(then, vec![s(4), Stmt::Label(L), s(5)], Vec::new()),
        returns(),
    ];
    let then = vec![when(p(2), vec![s(2)], vec![s(3), Stmt::Continue])];
    let out = swept(body);
    assert_eq!(out, [looped(then, vec![s(4)], vec![s(5)]), returns()]);
    assert_eq!(gotos(&out), 0);
}

#[test]
fn a_label_in_an_else_stays_where_its_if_does_not_end_a_loops_body_or_nothing_in_the_then_arm_jumps_to_it(
) {
    // Something follows the `if` in the body: what the then arm ran off to is not the loop's end.
    let then = vec![only(p(2), vec![s(2), goes(L)]), s(3)];
    let followed = vec![
        looped(then.clone(), vec![s(4), Stmt::Label(L), s(5)], vec![s(6)]),
        returns(),
    ];
    assert_eq!(swept(followed.clone()), followed);
    // The jump to the label comes from before the loop.
    let elsewhere = vec![
        only(p(3), vec![goes(L)]),
        looped(vec![s(3)], vec![s(4), Stmt::Label(L), s(5)], Vec::new()),
        returns(),
    ];
    assert_eq!(swept(elsewhere.clone()), elsewhere);
    // Not in a loop: off the end of the list is not where a `continue` goes.
    let bare = vec![when(p(1), then, vec![s(4), Stmt::Label(L), s(5)])];
    assert_eq!(swept(bare.clone()), bare);
}

#[test]
fn a_jump_out_of_a_last_body_to_what_follows_the_switch_runs_off_it_and_its_break_is_written() {
    // `switch (p0) { case 0: s1; break; case 1: switch (p1) { case 0: s2; goto L; case 1: s3; goto L; } } L: s4; return;`
    let inner = |first: Vec<Stmt>, last: Vec<Stmt>| Stmt::Switch {
        value: p(1),
        cases: vec![case(&[Some(0)], first), case(&[Some(1)], last)],
    };
    let outer = |last: Vec<Stmt>| Stmt::Switch {
        value: p(0),
        cases: vec![
            case(&[Some(0)], vec![s(1), Stmt::Break]),
            case(&[Some(1)], last),
        ],
    };
    let body = vec![
        outer(vec![inner(vec![s(2), goes(L)], vec![s(3), goes(L)])]),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    let done = inner(vec![s(2), Stmt::Break], vec![s(3), Stmt::Break]);
    let out = swept(body);
    assert_eq!(out, [outer(vec![done, Stmt::Break]), s(4), returns()]);
    assert_eq!(gotos(&out), 0);
}

#[test]
fn a_jump_out_of_a_body_before_the_last_still_needs_its_break() {
    // `switch (p0) { case 0: switch (p1) { case 0: s2; goto L; } s3; case 1: s1; break; } L: s4; return;`:
    // the first body runs off into the second, so `goto L` there is no `break` of the inner switch.
    let inner = Stmt::Switch {
        value: p(1),
        cases: vec![
            case(&[Some(0)], vec![s(2), goes(L)]),
            case(&[Some(1)], vec![Stmt::Break]),
        ],
    };
    let body = vec![
        Stmt::Switch {
            value: p(0),
            cases: vec![
                case(&[Some(0)], vec![inner, s(3)]),
                case(&[Some(1)], vec![s(1), Stmt::Break]),
            ],
        },
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    assert_eq!(swept(body.clone()), body);
}

#[test]
fn a_last_body_that_runs_off_is_ended_by_its_break_and_one_that_leaves_or_loops_is_not() {
    let switch = |last: Vec<Stmt>| Stmt::Switch {
        value: p(0),
        cases: vec![case(&[Some(0)], vec![s(1)]), case(&[None], last)],
    };
    let lp = Stmt::While {
        cond: p(2),
        body: vec![s(3)],
    };
    let mut body = vec![
        only(p(1), vec![switch(vec![s(2)])]),
        switch(Vec::new()),
        switch(vec![s(2), returns()]),
        switch(vec![lp.clone()]),
    ];
    closed_cases(&mut body);
    assert_eq!(
        body,
        [
            only(p(1), vec![switch(vec![s(2), Stmt::Break])]),
            switch(vec![Stmt::Break]),
            switch(vec![s(2), returns()]),
            switch(vec![lp]),
        ]
    );
}

// ---- a body that runs into the next, and a loop's counter stepped last ----------------------

/// `rule` over `body`, which must leave the flow as it was; the result, and whether the rule said it changed anything.
fn kept_flow(body: Vec<Stmt>, rule: fn(&mut Vec<Stmt>) -> bool) -> (Vec<Stmt>, bool) {
    let before = cfg::lower_body(body.clone()).expect("the body lowers");
    let mut out = body;
    let changed = rule(&mut out);
    let after = cfg::lower_body(out.clone()).expect("the result lowers");
    if let Err(why) = same_flow(&before, &after) {
        panic!("the flow changed: {why}\n{out:#?}");
    }
    (out, changed)
}

/// `func_000300()`, a value that calls.
fn called() -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at: 0x300,
            name: "func_000300".to_owned(),
        },
        args: Vec::new(),
        returns: None,
    }
}

/// `switch (p0) { case 0: <first> case 1: <next> }`.
fn two_cases(first: Vec<Stmt>, next: Vec<Stmt>) -> Vec<Stmt> {
    vec![Stmt::Switch {
        value: p(0),
        cases: vec![case(&[Some(0)], first), case(&[Some(1)], next)],
    }]
}

#[test]
fn a_body_that_runs_into_one_that_only_leaves_ends_in_a_copy_of_it() {
    // `case 0: s1; goto case 1; case 1: return p9; default: s2; s3; break;`
    let switch = |first: Vec<Stmt>| Stmt::Switch {
        value: p(0),
        cases: vec![
            case(&[Some(0)], first),
            case(&[Some(1)], vec![returns()]),
            case(&[None], vec![s(2), s(3), Stmt::Break]),
        ],
    };
    let (out, changed) = kept_flow(vec![switch(vec![s(1)]), returns()], fallen);
    assert!(changed);
    assert_eq!(out, [switch(vec![s(1), returns()]), returns()]);
    // The labels the next body starts with stay there, for the jumps that name them.
    let (out, _) = kept_flow(
        two_cases(vec![s(1)], vec![Stmt::Label(L), Stmt::Break]),
        fallen,
    );
    assert_eq!(
        out,
        two_cases(vec![s(1), Stmt::Break], vec![Stmt::Label(L), Stmt::Break])
    );
}

#[test]
fn assignments_with_no_call_go_along_and_a_copy_is_copied_on_into_the_body_before() {
    // `case 0: s1; case 1: s4; case 2: break;`: case 1 ends in `s4; break;`, which case 0 then takes too.
    let body = vec![Stmt::Switch {
        value: p(0),
        cases: vec![
            case(&[Some(0)], vec![s(1)]),
            case(&[Some(1)], vec![s(4)]),
            case(&[Some(2)], vec![Stmt::Break]),
        ],
    }];
    let (out, changed) = kept_flow(body, fallen);
    assert!(changed);
    assert_eq!(
        out,
        [Stmt::Switch {
            value: p(0),
            cases: vec![
                case(&[Some(0)], vec![s(1), s(4), Stmt::Break]),
                case(&[Some(1)], vec![s(4), Stmt::Break]),
                case(&[Some(2)], vec![Stmt::Break]),
            ],
        }]
    );
    // Two assignments go along; a third does not.
    let (out, _) = kept_flow(two_cases(vec![s(1)], vec![s(5), s(6), returns()]), fallen);
    assert_eq!(
        out,
        two_cases(
            vec![s(1), s(5), s(6), returns()],
            vec![s(5), s(6), returns()]
        )
    );
    let long = two_cases(vec![s(1)], vec![s(4), s(5), s(6), returns()]);
    assert_eq!(kept_flow(long.clone(), fallen), (long, false));
}

#[test]
fn a_body_run_into_is_not_copied_where_it_calls_or_runs_on_nor_behind_labels_or_a_loop() {
    let unchanged = |body: Vec<Stmt>| assert_eq!(kept_flow(body.clone(), fallen), (body, false));
    // The next body calls: in an assignment, in what it hands back, or alone.
    unchanged(two_cases(vec![s(1)], vec![give(T, called()), Stmt::Break]));
    unchanged(two_cases(vec![s(1)], vec![Stmt::Return(Some(called()))]));
    unchanged(two_cases(vec![s(1)], vec![handed(T), Stmt::Break]));
    // The next body runs on itself.
    unchanged(two_cases(vec![s(1)], vec![s(4)]));
    // A body of labels alone stands above the next one, and one that ends in a loop never runs off.
    unchanged(two_cases(vec![Stmt::Label(M)], vec![Stmt::Break]));
    let lp = Stmt::While {
        cond: p(2),
        body: vec![s(3)],
    };
    unchanged(two_cases(vec![s(1), lp], vec![Stmt::Break]));
    // A body that leaves runs into nothing.
    unchanged(two_cases(vec![s(1), returns()], vec![Stmt::Break]));
}

/// `local_5`, a counter.
const I: Var = Var::Local(5);

/// `local_5 < p0`.
fn below() -> Expr {
    Expr::binary(ir::BinOp::Lt, Expr::Var(I), p(0))
}

/// `local_5 = local_5 + 1;`.
fn step() -> Stmt {
    give(I, Expr::binary(ir::BinOp::Add, Expr::Var(I), k(1)))
}

/// `local_5 = 0; while (<cond>) { s1; <last> } return p9;`.
fn counting(cond: Expr, last: Vec<Stmt>) -> Vec<Stmt> {
    let mut body = vec![s(1)];
    body.extend(last);
    vec![give(I, k(0)), Stmt::While { cond, body }, returns()]
}

#[test]
fn a_loops_last_if_puts_the_arm_that_leaves_first_where_the_other_steps_the_counter() {
    // `if (p1) { s2; i++; } else { s3; break; }` is `if (!p1) { s3; break; } s2; i++;`.
    let turned = when(p(1), vec![s(2), step()], vec![s(3), Stmt::Break]);
    let (out, changed) = kept_flow(counting(below(), vec![turned]), steps_last);
    assert!(changed);
    let leads = only(not(p(1)), vec![s(3), Stmt::Break]);
    assert_eq!(out, counting(below(), vec![leads, s(2), step()]));
    // An else that ends in an `if` both arms of which leave never runs off either; it goes inside
    // a construct too.
    let both = when(p(2), vec![Stmt::Break], vec![returns()]);
    let nested = only(
        p(4),
        counting(below(), vec![when(p(1), vec![step()], vec![both.clone()])]),
    );
    let (out, _) = kept_flow(vec![nested, returns()], steps_last);
    let first = only(not(p(1)), vec![both]);
    assert_eq!(
        out,
        [
            only(p(4), counting(below(), vec![first, step()])),
            returns()
        ]
    );
}

#[test]
fn a_loops_last_if_stays_where_its_else_runs_on_or_nothing_makes_a_for_of_the_loop() {
    let unchanged =
        |body: Vec<Stmt>| assert_eq!(kept_flow(body.clone(), steps_last), (body, false));
    let leaving = || vec![s(3), Stmt::Break];
    // The else runs on to the end of the body.
    unchanged(counting(
        below(),
        vec![when(p(1), vec![step()], vec![s(3)])],
    ));
    // The loop does not test the counter.
    unchanged(counting(p(0), vec![when(p(1), vec![step()], leaving())]));
    // The then arm steps another variable, or does not end in the step.
    let other = give(
        Var::Local(6),
        Expr::binary(ir::BinOp::Add, Expr::Var(Var::Local(6)), k(1)),
    );
    unchanged(counting(below(), vec![when(p(1), vec![other], leaving())]));
    unchanged(counting(
        below(),
        vec![when(p(1), vec![step(), s(2)], leaving())],
    ));
    // A `continue` of the loop would go round past the step a `for` runs.
    let mut looped = counting(below(), vec![when(p(1), vec![step()], leaving())]);
    if let Stmt::While { body, .. } = &mut looped[1] {
        body.insert(0, only(p(2), vec![Stmt::Continue]));
    }
    unchanged(looped);
    // The counter is not set just before the loop.
    let mut apart = counting(below(), vec![when(p(1), vec![step()], leaving())]);
    apart.insert(1, Stmt::Label(L));
    unchanged(apart);
}

// ---- a run of constants, a loop's test again, and what an if runs on into ---------------------

/// A second `stack_` variable, one cell.
const W: Var = Var::Stack { at: 0x4C, index: 0 };

#[test]
fn a_run_of_constants_after_a_loop_whose_jumps_set_each_first_is_set_before_the_loop() {
    // `while (p1) { s1; if (p2) { t = p3; w = p4; goto L; } } t = -1; w = 0; L: s4; return;`
    let lp = |way: Stmt| Stmt::While {
        cond: p(1),
        body: vec![s(1), only(p(2), vec![give(T, p(3)), give(W, p(4)), way])],
    };
    let head = head_of(&[(T, Width::Cell, false), (W, Width::Cell, false)]);
    let body = vec![
        lp(goes(L)),
        give(T, k(-1)),
        give(W, k(0)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    let out = moved(body, &head);
    assert_eq!(
        out,
        [
            give(T, k(-1)),
            give(W, k(0)),
            lp(Stmt::Break),
            s(4),
            returns()
        ]
    );
    assert_eq!(gotos(&out), 0);
    // A jump that sets `t` alone: `t = -1` still goes in front, `w = 0` stays where it was. The
    // loop's test calls, so it is not tested again after the loop either.
    let partly = |way: Stmt| Stmt::While {
        cond: called(),
        body: vec![only(p(2), vec![give(T, p(3)), way])],
    };
    let body = vec![
        partly(goes(L)),
        give(T, k(-1)),
        give(W, k(0)),
        Stmt::Label(L),
        s(4),
        returns(),
    ];
    let out = moved(body, &head);
    assert_eq!(
        out,
        [
            give(T, k(-1)),
            partly(goes(L)),
            give(W, k(0)),
            Stmt::Label(L),
            s(4),
            returns()
        ]
    );
}

/// `*(int*)p1`, a test that reads memory.
fn loaded() -> Expr {
    Expr::Load {
        ty: ir::Scalar::I32,
        addr: Box::new(p(1)),
        offset: 0,
    }
}

/// `while (<cond>) { <body> } s3; L: s5; return;`.
fn searching(cond: Expr, body: Vec<Stmt>) -> Vec<Stmt> {
    vec![
        Stmt::While { cond, body },
        s(3),
        Stmt::Label(L),
        s(5),
        returns(),
    ]
}

#[test]
fn a_loop_whose_jumps_pass_over_what_follows_it_breaks_and_tests_itself_again_after() {
    let head = head_of(&[(T, Width::Cell, false)]);
    // `while (p1) { t = 1; if (p2) goto L; s2; } s3; L:` is
    // `while (p1) { t = 1; if (p2) break; s2; } if (!p1) { s3 }`.
    let body = |way: Stmt| vec![give(T, k(1)), only(p(2), vec![way]), s(2)];
    let out = moved(searching(p(1), body(goes(L))), &head);
    let ran_out = only(not(p(1)), vec![s(3)]);
    let lp = Stmt::While {
        cond: p(1),
        body: body(Stmt::Break),
    };
    assert_eq!(out, [lp, ran_out.clone(), s(5), returns()]);
    assert_eq!(gotos(&out), 0);
    // What the other arm of an `if` does is not on the way to the jump.
    let arms = |way: Stmt| {
        let set = give(Var::Param(1), k(0));
        vec![when(p(4), vec![set], vec![only(p(2), vec![way])])]
    };
    let out = moved(searching(p(1), arms(goes(L))), &head);
    let lp = Stmt::While {
        cond: p(1),
        body: arms(Stmt::Break),
    };
    assert_eq!(out, [lp, ran_out, s(5), returns()]);
    // A test that reads memory, with a call only after the jump.
    let late = |way: Stmt| {
        vec![
            only(p(2), vec![way]),
            Stmt::Simple(ir::Stmt::Eval(called())),
        ]
    };
    let out = moved(searching(loaded(), late(goes(L))), &head);
    let lp = Stmt::While {
        cond: loaded(),
        body: late(Stmt::Break),
    };
    assert_eq!(out, [lp, only(not(loaded()), vec![s(3)]), s(5), returns()]);
}

#[test]
fn a_loop_is_not_tested_again_where_its_test_may_read_otherwise_or_a_way_in_passes_it() {
    let head = head_of(&[(T, Width::Cell, false)]);
    let unchanged = |body: Vec<Stmt>| assert_eq!(moved(body.clone(), &head), body);
    let jump = || only(p(2), vec![goes(L)]);
    // The way to the jump sets what the test reads.
    unchanged(searching(p(1), vec![give(Var::Param(1), k(0)), jump()]));
    // A local no proof keeps from the test's variable.
    unchanged(searching(p(1), vec![s(6), jump()]));
    // The test calls.
    unchanged(searching(called(), vec![jump()]));
    // The test reads memory and a call comes before the jump.
    let call = Stmt::Simple(ir::Stmt::Eval(called()));
    unchanged(searching(loaded(), vec![call, jump()]));
    // A `break` leaves the loop to what follows it too.
    unchanged(searching(p(1), vec![only(p(4), vec![Stmt::Break]), jump()]));
    // A label in the loop lets a way in pass its test by.
    let mut entered = searching(p(1), vec![Stmt::Label(M), jump()]);
    entered.insert(0, only(p(6), vec![goes(M)]));
    unchanged(entered);
}

#[test]
fn a_loop_left_at_its_top_by_a_jump_and_by_breaks_further_on_tests_itself_again_after() {
    let head = head_of(&[(T, Width::Cell, false)]);
    // `while (true) { if (p1) goto L; t = 1; if (p2) break; if (p3) break; s2; } s3; L:` is
    // `while (!p1) { t = 1; if (p2) break; if (p3) break; s2; } if (!p1) { s3 }`: with two
    // `break`s the loop cannot take `s3` in.
    let rest = vec![
        give(T, k(1)),
        only(p(2), vec![Stmt::Break]),
        only(p(3), vec![Stmt::Break]),
        s(2),
    ];
    let mut body = vec![only(p(1), vec![goes(L)])];
    body.extend(rest.clone());
    let out = moved(searching(forever(), body), &head);
    let lp = Stmt::While {
        cond: not(p(1)),
        body: rest.clone(),
    };
    assert_eq!(out, [lp, only(not(p(1)), vec![s(3)]), s(5), returns()]);
    assert_eq!(gotos(&out), 0);
    // The way to a `break` sets what the test reads.
    let mut set = vec![only(p(1), vec![goes(L)]), give(Var::Param(1), k(0))];
    set.extend(rest);
    let body = searching(forever(), set);
    assert_eq!(moved(body.clone(), &head), body);
}

#[test]
fn what_an_if_runs_on_into_is_written_at_the_end_of_its_arms_where_their_jumps_then_go() {
    // `if (p0) { if (p1) { t = 1; goto L; } s2; if (p3) goto L; } w = 0; L: s5; return;`
    let arm = vec![
        only(p(1), vec![give(T, k(1)), goes(L)]),
        s(2),
        only(p(3), vec![goes(L)]),
    ];
    let body = vec![
        only(p(0), arm),
        give(W, k(0)),
        Stmt::Label(L),
        s(5),
        returns(),
    ];
    let out = swept(body);
    let inner = when(
        p(1),
        vec![give(T, k(1))],
        vec![s(2), only(not(p(3)), vec![give(W, k(0))])],
    );
    assert_eq!(
        out,
        [
            when(p(0), vec![inner], vec![give(W, k(0))]),
            s(5),
            returns()
        ]
    );
    assert_eq!(gotos(&out), 0);
    // An arm that never runs off takes nothing, so what follows may call: it goes once, to the else.
    let arm = vec![only(p(1), vec![goes(L)]), s(2), returns()];
    let body = vec![
        only(p(0), arm),
        give(W, called()),
        s(6),
        Stmt::Label(L),
        s(5),
        returns(),
    ];
    let out = swept(body);
    let then = vec![only(not(p(1)), vec![s(2), returns()])];
    assert_eq!(
        out,
        [
            when(p(0), then, vec![give(W, called()), s(6)]),
            s(5),
            returns()
        ]
    );
}

#[test]
fn what_an_if_runs_on_into_stays_where_it_calls_is_long_or_a_jump_would_stay() {
    let unchanged = |body: Vec<Stmt>| assert_eq!(swept(body.clone()), body);
    let with = |arm: Vec<Stmt>, passed: Vec<Stmt>| {
        let mut body = vec![only(p(0), arm)];
        body.extend(passed);
        body.extend([Stmt::Label(L), s(5), returns()]);
        body
    };
    let arm = || vec![only(p(1), vec![goes(L)]), s(2)];
    // Both arms run off, so it would be written twice: not where it calls, nor past two.
    unchanged(with(arm(), vec![give(W, called())]));
    unchanged(with(arm(), vec![give(W, k(0)), s(6), s(7)]));
    // A jump nested deeper than the rules that follow read.
    let deep = vec![only(p(1), vec![only(p(4), vec![goes(L)])]), s(2)];
    unchanged(with(deep, vec![give(W, k(0))]));
}
