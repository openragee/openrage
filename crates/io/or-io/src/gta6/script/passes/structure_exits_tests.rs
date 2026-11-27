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
    // A jump past it that sets nothing: `t` keeps what it held before.
    let unset = vec![
        Stmt::While {
            cond: p(1),
            body: vec![
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
            body: vec![give(T, p(6)), only(p(2), vec![give(T, p(3)), goes(L)])],
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
    // A `break` of the loop around the `if` leaves past the constant to where it is not set.
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
    assert_eq!(moved(left.clone(), &head), left);
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
