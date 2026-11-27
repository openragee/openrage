use std::collections::BTreeMap;

use super::super::tests::{not, p, s, when};
use super::*;
use crate::gta6::script::cfg::{self, Block, Cfg, Terminator};
use crate::gta6::script::ir::{Head, Returns};
use crate::gta6::script::names::Names;
use crate::gta6::script::options::Pass;
use crate::gta6::script::{Context, Options, Project, Stats};

/// A `stack_` variable the arms give a value to.
const X: Var = Var::Stack { at: 0x50, index: 0 };

fn give(value: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Assign { target: X, value })
}

fn truth(on: bool) -> Expr {
    Expr::Const(Const::bool(on))
}

fn int(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

/// `pN == k`: a test, which yields 0 or 1.
fn equal(n: u32, k: i32) -> Expr {
    Expr::binary(BinOp::Eq, p(n), int(k))
}

fn rewritten(stmt: Stmt) -> Vec<Stmt> {
    let mut body = vec![stmt];
    truths(&mut body);
    body
}

#[test]
fn a_test_that_only_sets_a_flag_true_or_false_is_the_flag_given_the_test() {
    let set = when(
        equal(0, 1),
        vec![give(truth(true))],
        vec![give(truth(false))],
    );
    assert_eq!(rewritten(set), [give(equal(0, 1))]);
    // The other way round it is the test turned.
    let unset = when(
        equal(0, 1),
        vec![give(truth(false))],
        vec![give(truth(true))],
    );
    assert_eq!(
        rewritten(unset),
        [give(Expr::binary(BinOp::Ne, p(0), int(1)))]
    );
    // A test of a cell that is no truth is written as one: its bits are not the flag's.
    let bare = when(p(0), vec![give(truth(true))], vec![give(truth(false))]);
    assert_eq!(rewritten(bare), [give(Expr::Truth(Box::new(p(0))))]);
    let turned = when(p(0), vec![give(truth(false))], vec![give(truth(true))]);
    assert_eq!(rewritten(turned), [give(not(p(0)))]);
    // `!p0` turned is `p0`, which is no truth either.
    let negated = when(not(p(0)), vec![give(truth(false))], vec![give(truth(true))]);
    assert_eq!(rewritten(negated), [give(Expr::Truth(Box::new(p(0))))]);
}

#[test]
fn a_flag_set_on_one_arm_and_tested_on_the_other_is_an_or_or_an_and() {
    let either = when(p(0), vec![give(truth(true))], vec![give(equal(1, 2))]);
    assert_eq!(rewritten(either), [give(or(p(0), equal(1, 2)))]);
    let both = when(p(0), vec![give(equal(1, 2))], vec![give(truth(false))]);
    assert_eq!(rewritten(both), [give(and(p(0), equal(1, 2)))]);
    let neither = when(p(0), vec![give(truth(false))], vec![give(equal(1, 2))]);
    assert_eq!(rewritten(neither), [give(and(not(p(0)), equal(1, 2)))]);
    let or_not = when(p(0), vec![give(equal(1, 2))], vec![give(truth(true))]);
    assert_eq!(rewritten(or_not), [give(or(not(p(0)), equal(1, 2)))]);
}

#[test]
fn one_and_zero_of_an_integer_are_a_choice_and_not_a_test() {
    let set = when(p(0), vec![give(int(1))], vec![give(int(0))]);
    assert_eq!(rewritten(set), [give(select(p(0), int(1), int(0)))]);
    let unset = when(p(0), vec![give(int(0))], vec![give(int(1))]);
    assert_eq!(rewritten(unset), [give(select(p(0), int(0), int(1)))]);
}

#[test]
fn an_if_that_gives_no_truth_or_two_places_or_more_stays() {
    let kept = [
        // `p1` may hold any bits.
        when(p(0), vec![give(truth(true))], vec![give(p(1))]),
        // Both arms give the same: the test still runs, and is no value.
        when(p(0), vec![give(truth(true))], vec![give(truth(true))]),
        // A `bool` and an `int` are no pair.
        when(p(0), vec![give(int(1))], vec![give(truth(false))]),
        // 1 next to a test is no `bool`.
        when(p(0), vec![give(int(1))], vec![give(equal(1, 2))]),
        // Floats are no truth.
        when(
            p(0),
            vec![give(Expr::Const(Const {
                bits: 1,
                kind: Kind::F32,
            }))],
            vec![give(Expr::Const(Const {
                bits: 0,
                kind: Kind::F32,
            }))],
        ),
        // Two variables.
        when(
            p(0),
            vec![give(truth(true))],
            vec![Stmt::Simple(ir::Stmt::Assign {
                target: Var::Stack { at: 0x54, index: 0 },
                value: truth(false),
            })],
        ),
        // An arm that does more.
        when(
            p(0),
            vec![s(1), give(truth(true))],
            vec![give(truth(false))],
        ),
    ];
    for stmt in kept {
        assert_eq!(rewritten(stmt.clone()), [stmt]);
    }
}

#[test]
fn an_if_that_holds_only_an_if_is_one_test_of_both() {
    let nested = when(p(0), vec![when(p(1), vec![s(1)], Vec::new())], Vec::new());
    assert_eq!(
        rewritten(nested),
        [when(and(p(0), p(1)), vec![s(1)], Vec::new())]
    );
    // An else on either is run on a test of one of them alone.
    let kept = [
        when(p(0), vec![when(p(1), vec![s(1)], vec![s(2)])], Vec::new()),
        when(p(0), vec![when(p(1), vec![s(1)], Vec::new())], vec![s(2)]),
        when(
            p(0),
            vec![s(2), when(p(1), vec![s(1)], Vec::new())],
            Vec::new(),
        ),
    ];
    for stmt in kept {
        assert_eq!(rewritten(stmt.clone()), [stmt]);
    }
}

#[test]
fn the_innermost_test_is_written_first() {
    // `if (p0) x = true; else if (p1) x = true; else x = p2 == 3;`
    let inner = when(p(1), vec![give(truth(true))], vec![give(equal(2, 3))]);
    let chain = when(p(0), vec![give(truth(true))], vec![inner]);
    assert_eq!(rewritten(chain), [give(or(p(0), or(p(1), equal(2, 3))))]);
}

#[test]
fn the_pass_by_its_name_writes_the_flag_and_without_it_the_if() {
    // 0: p0 == 1 ? 1 : 2    1: x = true -> 3    2: x = false -> 3    3: return x
    let assign = |on: bool| ir::Stmt::Assign {
        target: X,
        value: truth(on),
    };
    let blocks = vec![
        Block {
            at: None,
            stmts: Vec::new(),
            term: Terminator::Branch {
                cond: equal(0, 1),
                then: 1,
                els: 2,
            },
        },
        Block {
            at: None,
            stmts: vec![assign(true)],
            term: Terminator::Jump(3),
        },
        Block {
            at: None,
            stmts: vec![assign(false)],
            term: Terminator::Jump(3),
        },
        Block {
            at: None,
            stmts: Vec::new(),
            term: Terminator::Return(Some(Expr::Var(X))),
        },
    ];
    let names = Names::default();
    let run = |options: Options| {
        let f = cfg::Function {
            head: Head {
                value_types: Default::default(),
                program: 1,
                at: 0x10,
                name: "func_000010".to_owned(),
                returns: Returns::Cell,
                params: 1,
                locals: 0,
                vars: BTreeMap::new(),
                incomplete: None,
            },
            cfg: Cfg {
                blocks: blocks.clone(),
            },
        };
        let cx = Context::new(&names, Project::empty(), options);
        super::super::run_in(f, &cx, &mut Stats::default()).body
    };
    let on = Options::baseline().with(Pass::Conditions);
    let back = Stmt::Return(Some(Expr::Var(X)));
    assert_eq!(run(on), [give(equal(0, 1)), back.clone()]);
    let arms = when(
        equal(0, 1),
        vec![give(truth(true))],
        vec![give(truth(false))],
    );
    assert_eq!(run(Options::baseline()), [arms, back]);
}
