use std::collections::{BTreeMap, HashSet};

use super::*;
use crate::gta6::script::cfg::{Block, BlockId, Cfg, Terminator};
use crate::gta6::script::ir::{
    self, BinOp, Callee, Const, Head, Scalar, UnOp, Var, VarInfo, Width,
};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::sample;
use crate::gta6::script::tree::Case;

// ---- what the tests build with ------------------------------------------------------------

/// `local_n = n;`, a statement to tell apart from the others.
fn set(n: u32) -> ir::Stmt {
    ir::Stmt::Assign {
        target: Var::Local(n),
        value: Expr::Const(Const::i32(n as i32)),
    }
}

pub(super) fn s(n: u32) -> Stmt {
    Stmt::Simple(set(n))
}

/// `pN`, a condition no rewrite looks into.
pub(super) fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

pub(super) fn not(e: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::Not,
        e: Box::new(e),
    }
}

fn int(e: Expr) -> Expr {
    Expr::cast(Scalar::I32, e)
}

fn number(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

/// `(int)p0 == k`.
fn is(k: i32) -> Expr {
    Expr::binary(BinOp::Eq, int(p(0)), number(k))
}

fn block(stmts: &[u32], term: Terminator) -> Block {
    Block {
        at: None,
        stmts: stmts.iter().map(|n| set(*n)).collect(),
        term,
    }
}

fn jump(stmts: &[u32], to: BlockId) -> Block {
    block(stmts, Terminator::Jump(to))
}

fn branch(stmts: &[u32], cond: Expr, then: BlockId, els: BlockId) -> Block {
    block(stmts, Terminator::Branch { cond, then, els })
}

/// `return p9;`.
fn ret(stmts: &[u32]) -> Block {
    block(stmts, Terminator::Return(Some(p(9))))
}

pub(super) fn returns() -> Stmt {
    Stmt::Return(Some(p(9)))
}

pub(super) fn when(cond: Expr, then: Vec<Stmt>, els: Vec<Stmt>) -> Stmt {
    Stmt::If { cond, then, els }
}

pub(super) fn forever() -> Expr {
    Expr::Const(Const::bool(true))
}

fn head(returns: Returns) -> Head {
    Head {
        value_types: Default::default(),
        program: 1,
        at: 0x10,
        name: "func_000010".to_owned(),
        returns,
        params: 10,
        locals: 100,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

fn function(blocks: Vec<Block>, returns: Returns) -> cfg::Function {
    cfg::Function {
        head: head(returns),
        cfg: Cfg { blocks },
    }
}

/// The pass over blocks, with no check of the result.
fn rebuilt(blocks: Vec<Block>) -> Vec<Stmt> {
    let f = function(blocks, Returns::Cell);
    run(f, &Names::default(), &mut Stats::default()).body
}

/// The pass over blocks whose tests it may join but not rewrite: the result must be the same flow, every statement once.
fn structured(blocks: Vec<Block>) -> Vec<Stmt> {
    let before = Cfg {
        blocks: blocks.clone(),
    };
    let body = rebuilt(blocks);
    let after = cfg::lower_body(body.clone()).expect("the body lowers");
    if let Err(why) = same_flow(&before, &after) {
        panic!("the flow changed: {why}\n{body:#?}");
    }
    assert_eq!(tree::counts(&body).statements, before.statements());
    body
}

/// The pass over what a body lowers to.
fn restructured(body: Vec<Stmt>) -> Vec<Stmt> {
    structured(cfg::lower_body(body).expect("the body lowers").blocks)
}

fn shows(body: &[Stmt], what: &str) -> bool {
    format!("{body:?}").contains(what)
}

/// How deep the statements of a body nest.
fn depth(body: &[Stmt]) -> usize {
    let mut deepest = 0;
    let mut work: Vec<(&[Stmt], usize)> = vec![(body, 1)];
    while let Some((list, at)) = work.pop() {
        deepest = deepest.max(at);
        for stmt in list {
            match stmt {
                Stmt::If { then, els, .. } => {
                    work.push((then.as_slice(), at + 1));
                    work.push((els.as_slice(), at + 1));
                }
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    work.push((body.as_slice(), at + 1));
                }
                Stmt::Switch { cases, .. } => {
                    for case in cases {
                        work.push((case.body.as_slice(), at + 1));
                    }
                }
                _ => {}
            }
        }
    }
    deepest
}

// ---- whether two block graphs are the same flow -------------------------------------------

/// One step of a function: jumps and labels are edges between these.
#[derive(Debug)]
enum Node {
    Stmt(ir::Stmt, usize),
    /// A test nothing joins: where it leads when it holds, and when not.
    Test(Expr, usize, usize),
    Switch(Expr, Vec<(i64, usize)>, usize),
    Return(Option<Expr>),
    Unreachable,
    /// A block's entry: the node its first statement or its terminator is.
    Entry(usize),
}

/// The node a condition's outcome is decided from, with `&&`, `||`, `?:` and `!` taken apart again.
fn test(nodes: &mut Vec<Node>, cond: &Expr, yes: usize, no: usize) -> usize {
    match cond {
        Expr::Unary { op: UnOp::Not, e } => test(nodes, e, no, yes),
        Expr::Binary {
            op: BinOp::LogicalAnd,
            l,
            r,
            ..
        } => {
            let right = test(nodes, r, yes, no);
            test(nodes, l, right, no)
        }
        Expr::Binary {
            op: BinOp::LogicalOr,
            l,
            r,
            ..
        } => {
            let right = test(nodes, r, yes, no);
            test(nodes, l, yes, right)
        }
        Expr::Select { cond, then, els } => {
            let (a, b) = (test(nodes, then, yes, no), test(nodes, els, yes, no));
            test(nodes, cond, a, b)
        }
        // A comparison and its opposite are one test with its arms swapped.
        Expr::Binary {
            op: op @ (BinOp::Ne | BinOp::Ge | BinOp::Gt),
            at,
            l,
            r,
        } if *op == BinOp::Ne || at.is_integer() => {
            let turned = Expr::Binary {
                op: match op {
                    BinOp::Ne => BinOp::Eq,
                    BinOp::Ge => BinOp::Lt,
                    _ => BinOp::Le,
                },
                at: at.clone(),
                l: l.clone(),
                r: r.clone(),
            };
            nodes.push(Node::Test(turned, no, yes));
            nodes.len() - 1
        }
        atom => {
            nodes.push(Node::Test(atom.clone(), yes, no));
            nodes.len() - 1
        }
    }
}

/// A block graph as nodes; block `b` enters at node `b`.
fn nodes_of(cfg: &Cfg) -> Vec<Node> {
    let mut nodes: Vec<Node> = cfg.blocks.iter().map(|_| Node::Unreachable).collect();
    for (b, block) in cfg.blocks.iter().enumerate() {
        let mut next = match &block.term {
            Terminator::Jump(to) => *to,
            Terminator::Branch { cond, then, els } => test(&mut nodes, cond, *then, *els),
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                nodes.push(Node::Switch(value.clone(), cases.clone(), *default));
                nodes.len() - 1
            }
            Terminator::Return(value) => {
                nodes.push(Node::Return(value.clone()));
                nodes.len() - 1
            }
            Terminator::Unreachable => {
                nodes.push(Node::Unreachable);
                nodes.len() - 1
            }
        };
        for stmt in block.stmts.iter().rev() {
            nodes.push(Node::Stmt(stmt.clone(), next));
            next = nodes.len() - 1;
        }
        nodes[b] = Node::Entry(next);
    }
    nodes
}

/// The node control is at once it has passed the entries; `usize::MAX` for a ring of blocks that do nothing.
fn passed(nodes: &[Node], mut at: usize) -> usize {
    for _ in 0..=nodes.len() {
        match &nodes[at] {
            Node::Entry(next) => at = *next,
            _ => return at,
        }
    }
    usize::MAX
}

/// Whether two block graphs run the same statements, tests, switches and returns on every path from their entries.
pub(super) fn same_flow(a: &Cfg, b: &Cfg) -> Result<(), String> {
    let (left, right) = (nodes_of(a), nodes_of(b));
    let mut seen = HashSet::new();
    let mut work = vec![(0usize, 0usize)];
    while let Some((x, y)) = work.pop() {
        let (x, y) = (passed(&left, x), passed(&right, y));
        if !seen.insert((x, y)) {
            continue;
        }
        if x == usize::MAX || y == usize::MAX {
            if x != y {
                return Err("one side spins".to_owned());
            }
            continue;
        }
        match (&left[x], &right[y]) {
            (Node::Stmt(s, on), Node::Stmt(t, to)) if s == t => work.push((*on, *to)),
            (Node::Test(c, yes, no), Node::Test(d, so, not)) if c == d => {
                work.push((*yes, *so));
                work.push((*no, *not));
            }
            (Node::Switch(v, cases, default), Node::Switch(w, others, other)) if v == w => {
                for (label, to) in cases {
                    let there = others.iter().find(|case| case.0 == *label);
                    work.push((*to, there.map_or(*other, |case| case.1)));
                }
                for (label, to) in others {
                    if !cases.iter().any(|case| case.0 == *label) {
                        work.push((*default, *to));
                    }
                }
                work.push((*default, *other));
            }
            (Node::Return(v), Node::Return(w)) if v == w => {}
            (Node::Unreachable, Node::Unreachable) => {}
            (was, is) => return Err(format!("{was:?} became {is:?}")),
        }
    }
    Ok(())
}

#[test]
fn the_check_tells_two_flows_apart() {
    let straight = Cfg {
        blocks: vec![branch(&[0], p(0), 1, 2), jump(&[1], 2), ret(&[])],
    };
    let crossed = Cfg {
        blocks: vec![branch(&[0], p(0), 2, 1), jump(&[1], 2), ret(&[])],
    };
    let turned = Cfg {
        blocks: vec![branch(&[0], not(p(0)), 2, 1), jump(&[1], 2), ret(&[])],
    };
    assert!(same_flow(&straight, &straight).is_ok());
    assert!(same_flow(&straight, &crossed).is_err());
    assert!(
        same_flow(&straight, &turned).is_ok(),
        "a test turned and its arms swapped is the same test"
    );
}

// ---- if and else --------------------------------------------------------------------------

#[test]
fn an_if_with_an_else_comes_back_as_one() {
    let body = vec![s(0), when(p(0), vec![s(1)], vec![s(2)]), s(3), returns()];
    assert_eq!(restructured(body.clone()), body);
}

#[test]
fn an_empty_arm_in_front_is_the_test_turned() {
    let body = vec![when(p(0), Vec::new(), vec![s(1)]), returns()];
    assert_eq!(
        restructured(body),
        [when(not(p(0)), vec![s(1)], Vec::new()), returns()]
    );
}

#[test]
fn an_else_that_is_an_if_stays_a_chain() {
    let body = vec![
        when(p(0), vec![s(1)], vec![when(p(1), vec![s(2)], vec![s(3)])]),
        returns(),
    ];
    assert_eq!(restructured(body.clone()), body);
}

#[test]
fn an_arm_that_returns_takes_no_else() {
    // if (p0) { s1; return; } s2; return;  the second arm is what follows, not an else.
    let blocks = vec![
        branch(&[], p(0), 1, 2),
        block(&[1], Terminator::Return(None)),
        block(&[2], Terminator::Return(Some(p(9)))),
    ];
    assert_eq!(
        structured(blocks),
        [
            when(p(0), vec![s(1), Stmt::Return(None)], Vec::new()),
            s(2),
            returns()
        ]
    );
}

#[test]
fn an_else_is_only_what_the_then_arm_cannot_reach() {
    // 0: p0 ? 3 : 1    1: s5; p1 ? 3 : 2    2: s1 -> 4    3: s2 -> 4
    // Block 3 looks like the else of block 0, but block 1 leads into it too.
    let blocks = vec![
        branch(&[0], p(0), 3, 1),
        branch(&[5], p(1), 3, 2),
        jump(&[1], 4),
        jump(&[2], 4),
        ret(&[4]),
    ];
    let body = structured(blocks);
    assert_eq!(tree::counts(&body).gotos, 1, "{body:#?}");
    let Some(Stmt::If { els, .. }) = body.get(1) else {
        panic!("no if after the first statement: {body:#?}");
    };
    assert!(
        els.is_empty(),
        "the shared block was made an else: {body:#?}"
    );
}

// ---- conditions ---------------------------------------------------------------------------

#[test]
fn two_tests_that_skip_to_one_place_are_an_and() {
    // 0: s0; p0 ? 1 : 3    1: p1 ? 2 : 3    2: s1 -> 3    3: return
    let blocks = vec![
        branch(&[0], p(0), 1, 3),
        branch(&[], p(1), 2, 3),
        jump(&[1], 3),
        ret(&[]),
    ];
    let both = Expr::binary(BinOp::LogicalAnd, p(0), p(1));
    assert_eq!(
        structured(blocks),
        [s(0), when(both, vec![s(1)], Vec::new()), returns()]
    );
}

#[test]
fn two_tests_that_enter_one_place_are_an_or() {
    // 0: s0; p0 ? 2 : 1    1: p1 ? 2 : 3    2: s1 -> 3    3: return
    let blocks = vec![
        branch(&[0], p(0), 2, 1),
        branch(&[], p(1), 2, 3),
        jump(&[1], 3),
        ret(&[]),
    ];
    let either = Expr::binary(BinOp::LogicalOr, p(0), p(1));
    assert_eq!(
        structured(blocks),
        [s(0), when(either, vec![s(1)], Vec::new()), returns()]
    );
}

#[test]
fn a_second_test_with_a_statement_before_it_is_not_joined() {
    // As the `and`, but block 1 runs a statement before it tests: joined, the statement would
    // have nowhere to run.
    let blocks = vec![
        branch(&[0], p(0), 1, 3),
        branch(&[5], p(1), 2, 3),
        jump(&[1], 3),
        ret(&[]),
    ];
    assert_eq!(
        structured(blocks),
        [
            s(0),
            when(
                p(0),
                vec![s(5), when(p(1), vec![s(1)], Vec::new())],
                Vec::new()
            ),
            returns()
        ]
    );
}

#[test]
fn a_second_test_reached_from_elsewhere_is_not_joined() {
    // 0: p0 ? 1 : 2    1: p1 ? 3 : 4    2: s5 -> 1    3: s1 -> 4    4: return
    // Block 1 is reached past block 2 without `p0` holding.
    let blocks = vec![
        branch(&[0], p(0), 1, 2),
        branch(&[], p(1), 3, 4),
        jump(&[5], 1),
        jump(&[1], 4),
        ret(&[]),
    ];
    let body = structured(blocks);
    assert!(!shows(&body, "LogicalAnd"), "{body:#?}");
}

#[test]
fn a_test_of_one_of_two_tests_is_a_choice() {
    // 0: p0 ? 1 : 2    1: p1 ? 3 : 4    2: p2 ? 3 : 4    3: s1 -> 5    4: s2 -> 5
    let blocks = vec![
        branch(&[0], p(0), 1, 2),
        branch(&[], p(1), 3, 4),
        branch(&[], p(2), 3, 4),
        jump(&[1], 5),
        jump(&[2], 5),
        ret(&[]),
    ];
    let choice = Expr::Select {
        cond: Box::new(p(0)),
        then: Box::new(p(1)),
        els: Box::new(p(2)),
    };
    assert_eq!(
        structured(blocks),
        [s(0), when(choice, vec![s(1)], vec![s(2)]), returns()]
    );

    // One of the two runs a statement first: no expression states that.
    let blocks = vec![
        branch(&[0], p(0), 1, 2),
        branch(&[], p(1), 3, 4),
        branch(&[7], p(2), 3, 4),
        jump(&[1], 5),
        jump(&[2], 5),
        ret(&[]),
    ];
    let body = structured(blocks);
    assert!(!shows(&body, "Select"), "{body:#?}");
}

#[test]
fn a_condition_gathers_no_more_tests_than_its_bound() {
    // 200 tests in a row that all skip to the end.
    let n = 200;
    let mut blocks: Vec<Block> = (0..n)
        .map(|i| branch(&[], p(i as u32 % 8), i + 1, n + 1))
        .collect();
    blocks.push(jump(&[1], n + 1));
    blocks.push(ret(&[]));
    let body = structured(blocks);
    let mut work: Vec<&[Stmt]> = vec![body.as_slice()];
    let mut tests = 0;
    while let Some(list) = work.pop() {
        for stmt in list {
            if let Stmt::If { cond, then, els } = stmt {
                let mut joined = 0;
                cond.walk(&mut |e| {
                    joined += usize::from(matches!(e, Expr::Var(_)));
                });
                assert!(joined <= 64, "{joined} tests in one condition");
                tests += joined;
                work.push(then.as_slice());
                work.push(els.as_slice());
            }
        }
    }
    assert_eq!(tests, n);
}

#[test]
fn a_negation_goes_into_a_comparison_only_where_that_is_the_same_test() {
    let less = |ty: Scalar| Expr::binary(BinOp::Lt, Expr::cast(ty, p(0)), Expr::cast(ty, p(1)));
    let not_less = Expr::binary(BinOp::Ge, int(p(0)), int(p(1)));
    assert_eq!(cond::negate(less(Scalar::I32)), not_less);
    // ⚠ `!(a < b)` holds for a NaN and `a >= b` does not.
    assert_eq!(cond::negate(less(Scalar::F32)), not(less(Scalar::F32)));
    assert_eq!(cond::negate(not(p(0))), p(0));
    assert_eq!(
        cond::negate(Expr::binary(BinOp::LogicalAnd, p(0), not(p(1)))),
        Expr::binary(BinOp::LogicalOr, not(p(0)), p(1))
    );
}

#[test]
fn a_test_of_a_truth_value_against_zero_is_that_value() {
    let less = Expr::binary(BinOp::Lt, int(p(0)), int(p(1)));
    let zero = || number(0);
    let unsigned = Expr::cast(Scalar::U32, less.clone());
    assert_eq!(
        cond::simplify(Expr::binary(BinOp::Ne, unsigned.clone(), zero())),
        less
    );
    assert_eq!(
        cond::simplify(Expr::binary(BinOp::Eq, unsigned, zero())),
        Expr::binary(BinOp::Ge, int(p(0)), int(p(1)))
    );
    // A value that is not 0 or 1 is not its own test against zero under a cast.
    let plain = Expr::binary(BinOp::Ne, Expr::cast(Scalar::U32, p(0)), zero());
    assert_eq!(cond::simplify(plain.clone()), plain);
}

// ---- loops --------------------------------------------------------------------------------

#[test]
fn a_loop_tested_at_its_top_is_a_while() {
    // 0: s0 -> 1    1: p0 ? 2 : 3    2: s1 -> 1    3: return
    let blocks = vec![
        jump(&[0], 1),
        branch(&[], p(0), 2, 3),
        jump(&[1], 1),
        ret(&[]),
    ];
    let lp = Stmt::While {
        cond: p(0),
        body: vec![s(1)],
    };
    assert_eq!(structured(blocks), [s(0), lp, returns()]);
}

#[test]
fn a_header_that_runs_a_statement_before_its_test_keeps_the_test_inside() {
    // As the `while`, but the header runs `s0` on every turn before it tests.
    let blocks = vec![
        jump(&[9], 1),
        branch(&[0], p(0), 2, 3),
        jump(&[1], 1),
        ret(&[]),
    ];
    let lp = Stmt::While {
        cond: forever(),
        body: vec![s(0), when(not(p(0)), vec![Stmt::Break], Vec::new()), s(1)],
    };
    assert_eq!(structured(blocks), [s(9), lp, returns()]);
}

#[test]
fn a_loop_tested_at_its_bottom_is_a_do() {
    // 0: s0 -> 1    1: s1 -> 2    2: p0 ? 1 : 3    3: return
    let blocks = vec![
        jump(&[0], 1),
        jump(&[1], 2),
        branch(&[], p(0), 1, 3),
        ret(&[]),
    ];
    let lp = Stmt::DoWhile {
        body: vec![s(1)],
        cond: p(0),
    };
    assert_eq!(structured(blocks), [s(0), lp, returns()]);

    // The whole loop in one block, its test turned.
    let blocks = vec![jump(&[0], 1), branch(&[1], p(0), 2, 1), ret(&[])];
    let lp = Stmt::DoWhile {
        body: vec![s(1)],
        cond: not(p(0)),
    };
    assert_eq!(structured(blocks), [s(0), lp, returns()]);
}

#[test]
fn a_second_jump_back_to_the_top_makes_no_do() {
    // 0: s0 -> 1    1: s1; p1 ? 1 : 2    2: s2; p0 ? 1 : 3    3: return
    // The jump from block 1 goes to the top, where `continue` in a `do` would go to the test.
    let blocks = vec![
        jump(&[0], 1),
        branch(&[1], p(1), 1, 2),
        branch(&[2], p(0), 1, 3),
        ret(&[]),
    ];
    let lp = Stmt::While {
        cond: forever(),
        body: vec![
            s(1),
            when(p(1), vec![Stmt::Continue], Vec::new()),
            s(2),
            when(not(p(0)), vec![Stmt::Break], Vec::new()),
        ],
    };
    assert_eq!(structured(blocks), [s(0), lp, returns()]);
}

#[test]
fn a_do_whose_test_leaves_elsewhere_has_no_break() {
    // 1: s1; p1 ? 3 : 2    2: s2; p0 ? 1 : 4    3: s3 -> 5    4: s4 -> 5
    // The loop's test leaves to block 4 and block 1 to block 3, which follows the loop: as a
    // `do`, the `break` for block 1 would land on the jump to block 4.
    let blocks = vec![
        jump(&[0], 1),
        branch(&[1], p(1), 3, 2),
        branch(&[2], p(0), 1, 4),
        jump(&[3], 5),
        jump(&[4], 5),
        ret(&[]),
    ];
    let body = structured(blocks);
    assert!(!shows(&body, "DoWhile"), "{body:#?}");
    assert!(shows(&body, "Break"), "{body:#?}");
}

fn counter() -> (ir::Stmt, Expr, ir::Stmt) {
    let i = || Expr::Var(Var::Local(0));
    let init = ir::Stmt::Assign {
        target: Var::Local(0),
        value: number(0),
    };
    let below = Expr::binary(BinOp::Lt, int(i()), int(p(0)));
    let step = ir::Stmt::Assign {
        target: Var::Local(0),
        value: Expr::binary(BinOp::Add, int(i()), number(1)),
    };
    (init, below, step)
}

fn simple(stmt: &ir::Stmt, term: Terminator) -> Block {
    Block {
        at: None,
        stmts: vec![stmt.clone()],
        term,
    }
}

#[test]
fn a_counted_loop_is_a_for() {
    let (init, below, step) = counter();
    let counted = Stmt::For {
        init: Some(Box::new(init.clone())),
        cond: Some(below.clone()),
        step: Some(Box::new(step.clone())),
        body: vec![s(5)],
    };

    // The step in a block of its own, as a `for` lowers.
    let blocks = vec![
        simple(&init, Terminator::Jump(1)),
        branch(&[], below.clone(), 2, 4),
        jump(&[5], 3),
        simple(&step, Terminator::Jump(1)),
        ret(&[]),
    ];
    assert_eq!(structured(blocks), [counted.clone(), returns()]);

    // The step at the end of the body's one block.
    let mut body = jump(&[5], 1);
    body.stmts.push(step.clone());
    let blocks = vec![
        simple(&init, Terminator::Jump(1)),
        branch(&[], below.clone(), 2, 3),
        body,
        ret(&[]),
    ];
    assert_eq!(structured(blocks), [counted, returns()]);
}

#[test]
fn a_jump_back_past_the_step_makes_no_for() {
    // 2: s5; p1 ? 1 : 3    Block 2 goes back to the test without stepping: in a `for`, a
    // `continue` there would step.
    let (init, below, step) = counter();
    let blocks = vec![
        simple(&init, Terminator::Jump(1)),
        branch(&[], below.clone(), 2, 4),
        branch(&[5], p(1), 1, 3),
        simple(&step, Terminator::Jump(1)),
        ret(&[]),
    ];
    let lp = Stmt::While {
        cond: below,
        body: vec![
            s(5),
            when(p(1), vec![Stmt::Continue], Vec::new()),
            Stmt::Simple(step),
        ],
    };
    assert_eq!(structured(blocks), [Stmt::Simple(init), lp, returns()]);
}

#[test]
fn a_loop_with_no_counter_set_before_it_is_no_for() {
    // The body steps `local_0`, but what stands before the loop sets `local_7`.
    let (_, below, step) = counter();
    let mut body = jump(&[5], 1);
    body.stmts.push(step.clone());
    let blocks = vec![
        jump(&[7], 1),
        branch(&[], below.clone(), 2, 3),
        body,
        ret(&[]),
    ];
    let lp = Stmt::While {
        cond: below,
        body: vec![s(5), Stmt::Simple(step)],
    };
    assert_eq!(structured(blocks), [s(7), lp, returns()]);
}

// ---- switch -------------------------------------------------------------------------------

pub(super) fn case(labels: &[Option<i64>], body: Vec<Stmt>) -> Case {
    Case {
        labels: labels.to_vec(),
        body,
    }
}

#[test]
fn a_switch_comes_back_with_its_bodies() {
    let body = vec![
        Stmt::Switch {
            value: p(0),
            cases: vec![
                case(&[Some(0), Some(2)], vec![s(1), Stmt::Break]),
                // No `break`: this body runs into the next.
                case(&[Some(1)], vec![s(2)]),
                case(&[None], vec![s(3), Stmt::Break]),
            ],
        },
        s(4),
        returns(),
    ];
    assert_eq!(restructured(body.clone()), body);
}

#[test]
fn a_default_that_only_leaves_the_switch_is_not_written() {
    // 0: switch (p0) { 0 -> 1, 1 -> 2, default -> 3 }    1: s1 -> 3    2: s2 -> 3    3: return
    let blocks = vec![
        block(
            &[0],
            Terminator::Switch {
                value: p(0),
                cases: vec![(0, 1), (1, 2)],
                default: 3,
            },
        ),
        jump(&[1], 3),
        jump(&[2], 3),
        ret(&[]),
    ];
    let switch = Stmt::Switch {
        value: p(0),
        cases: vec![
            case(&[Some(0)], vec![s(1), Stmt::Break]),
            case(&[Some(1)], vec![s(2), Stmt::Break]),
        ],
    };
    assert_eq!(structured(blocks), [s(0), switch, returns()]);
}

#[test]
fn bodies_a_listing_hoisted_past_the_join_go_back_into_the_switch() {
    // The listing states the join, at 0x216, before the two bodies, at 0x16F and 0x17E.
    let at = |offset: u32, mut block: Block| {
        block.at = Some(offset);
        block
    };
    let blocks = vec![
        block(
            &[0],
            Terminator::Switch {
                value: p(0),
                cases: vec![(0, 2), (1, 3)],
                default: 1,
            },
        ),
        at(0x216, ret(&[9])),
        at(0x16F, jump(&[1], 1)),
        at(0x17E, jump(&[2], 1)),
    ];
    let switch = Stmt::Switch {
        value: p(0),
        cases: vec![
            case(&[Some(0)], vec![s(1), Stmt::Break]),
            case(&[Some(1)], vec![s(2), Stmt::Break]),
        ],
    };
    assert_eq!(structured(blocks), [s(0), switch, s(9), returns()]);
}

#[test]
fn a_switch_on_a_value_less_a_constant_is_a_switch_on_the_value() {
    let biased = |by: i32, labels: [i64; 2]| {
        vec![
            block(
                &[0],
                Terminator::Switch {
                    value: int(Expr::binary(BinOp::Sub, int(p(0)), number(by))),
                    cases: vec![(labels[0], 1), (labels[1], 2)],
                    default: 3,
                },
            ),
            jump(&[1], 3),
            jump(&[2], 3),
            ret(&[]),
        ]
    };
    let labels = |body: &[Stmt]| match body.get(1) {
        Some(Stmt::Switch { value, cases }) => (
            value.clone(),
            cases
                .iter()
                .flat_map(|c| c.labels.clone())
                .collect::<Vec<_>>(),
        ),
        _ => panic!("no switch: {body:#?}"),
    };
    let moved = rebuilt(biased(3, [0, 1]));
    assert_eq!(
        labels(moved.as_slice()),
        (int(p(0)), vec![Some(3), Some(4)])
    );

    // A label that would leave the range of an `int` keeps the subtraction.
    let top = i64::from(i32::MAX);
    let kept = int(Expr::binary(BinOp::Sub, int(p(0)), number(1)));
    let unmoved = rebuilt(biased(1, [0, top]));
    assert_eq!(labels(unmoved.as_slice()), (kept, vec![Some(0), Some(top)]));
}

/// The blocks of `if (p0 == 10) s1; else if (p0 == 20) s2; else if (p0 == 30) s3; else s4;`.
fn chain(second: &[u32]) -> Vec<Block> {
    vec![
        branch(&[0], is(10), 3, 1),
        branch(second, is(20), 4, 2),
        branch(&[], is(30), 5, 6),
        jump(&[1], 7),
        jump(&[2], 7),
        jump(&[3], 7),
        jump(&[4], 7),
        ret(&[]),
    ]
}

#[test]
fn a_chain_of_tests_of_one_value_is_a_switch() {
    let switch = Stmt::Switch {
        value: int(p(0)),
        cases: vec![
            case(&[Some(10)], vec![s(1), Stmt::Break]),
            case(&[Some(20)], vec![s(2), Stmt::Break]),
            case(&[Some(30)], vec![s(3), Stmt::Break]),
            case(&[None], vec![s(4), Stmt::Break]),
        ],
    };
    assert_eq!(rebuilt(chain(&[])), [s(0), switch, returns()]);
}

#[test]
fn tests_with_a_statement_between_them_are_no_switch() {
    // The second test runs `s9` first: read as a switch, `s9` would not run for 20 or 30.
    let body = structured(chain(&[9]));
    assert!(!shows(&body, "Switch"), "{body:#?}");
}

#[test]
fn two_cases_stay_an_if() {
    let blocks = vec![
        branch(&[0], is(10), 2, 1),
        branch(&[], is(20), 3, 4),
        jump(&[1], 5),
        jump(&[2], 5),
        jump(&[3], 5),
        ret(&[]),
    ];
    let body = structured(blocks);
    assert!(!shows(&body, "Switch"), "{body:#?}");
}

#[test]
fn a_tree_of_tests_is_read_by_running_it_on_each_constant() {
    // 0: p0 > 20 ? 3 : 1    1: p0 == 10 ? 5 : 2    2: p0 == 20 ? 6 : 9
    // 3: p0 == 30 ? 7 : 4   4: p0 == 40 ? 8 : 9
    let over = Expr::binary(BinOp::Gt, int(p(0)), number(20));
    let mut blocks = vec![
        branch(&[0], over, 3, 1),
        branch(&[], is(10), 5, 2),
        branch(&[], is(20), 6, 9),
        branch(&[], is(30), 7, 4),
        branch(&[], is(40), 8, 9),
    ];
    blocks.extend((1..=5).map(|n| ret(&[n])));
    let mut switched = blocks.clone();
    graph::switch_trees(&mut switched, false);
    // The four tests under the first are gone, so each block after them is four earlier.
    assert_eq!(
        switched[0].term,
        Terminator::Switch {
            value: int(p(0)),
            cases: vec![(10, 1), (20, 2), (30, 3), (40, 4)],
            default: 5,
        }
    );
    assert_eq!(switched.len(), 6);

    // The same tree with one way out, not by an equality, to a block of its own: a value over
    // 40 would have to reach that block, which no case can say.
    let last = blocks.len();
    blocks.push(ret(&[6]));
    blocks[4] = branch(&[], is(40), 8, last);
    let mut kept = blocks.clone();
    graph::switch_trees(&mut kept, false);
    assert_eq!(kept, blocks);
}

#[test]
fn a_test_compares_as_its_operator_reads_the_bits() {
    let signed = cond::Test::of(&Expr::binary(BinOp::Lt, int(p(0)), number(3)));
    let high = Expr::Const(Const {
        bits: 0x8000_0000,
        kind: ir::Kind::U32,
    });
    let unsigned = cond::Test::of(&Expr::binary(
        BinOp::Lt,
        Expr::cast(Scalar::U32, p(0)),
        high,
    ));
    let (Some(signed), Some(unsigned)) = (signed, unsigned) else {
        panic!("a comparison with a constant is a test");
    };
    let minus_five = -5i32 as u32;
    assert!(signed.holds(minus_five), "-5 < 3");
    assert!(
        !unsigned.holds(minus_five),
        "0xFFFFFFFB is not below 0x80000000"
    );
    assert!(unsigned.holds(7));
    // Nothing to read where neither side is a constant, or the comparison is of floats.
    assert_eq!(
        cond::Test::of(&Expr::binary(BinOp::Lt, int(p(0)), int(p(1)))),
        None
    );
    let float = Expr::binary(
        BinOp::Lt,
        Expr::cast(Scalar::F32, p(0)),
        Expr::Const(Const::f32(1.0)),
    );
    assert_eq!(cond::Test::of(&float), None);
}

// ---- a value a join only tests ------------------------------------------------------------

const CARRIED: Var = Var::Stack { at: 0x40, index: 0 };

/// `if (p0) { first; carried = p1 > 5; } else carried = 0;` then `if (carried != 0) guarded;`.
fn carried(first: Vec<ir::Stmt>, guarded: ir::Stmt) -> cfg::Function {
    let over = Expr::binary(BinOp::Gt, int(p(1)), number(5));
    let mut first_arm = first;
    first_arm.push(ir::Stmt::Assign {
        target: CARRIED,
        value: over,
    });
    let set_zero = ir::Stmt::Assign {
        target: CARRIED,
        value: number(0),
    };
    let tested = Expr::binary(
        BinOp::Ne,
        Expr::cast(Scalar::U32, Expr::Var(CARRIED)),
        number(0),
    );
    let blocks = vec![
        branch(&[0], p(0), 1, 2),
        Block {
            at: None,
            stmts: first_arm,
            term: Terminator::Jump(3),
        },
        simple(&set_zero, Terminator::Jump(3)),
        branch(&[], tested, 4, 5),
        simple(&guarded, Terminator::Jump(5)),
        ret(&[]),
    ];
    let mut f = function(blocks, Returns::Cell);
    f.head.vars.insert(CARRIED, VarInfo::new(Width::Cell));
    f
}

#[test]
fn a_value_a_join_only_tests_is_tested_where_it_is_made() {
    let out = run(
        carried(Vec::new(), set(1)),
        &Names::default(),
        &mut Stats::default(),
    );
    let over = Expr::binary(BinOp::Gt, int(p(1)), number(5));
    let both = Expr::binary(BinOp::LogicalAnd, p(0), over);
    assert_eq!(
        out.body,
        [s(0), when(both, vec![s(1)], Vec::new()), returns()]
    );
    assert!(
        !out.head.vars.contains_key(&CARRIED),
        "the variable is gone with its assignments"
    );
}

#[test]
fn a_value_read_again_stays_in_its_variable() {
    // The guarded statement reads the variable too: threaded, it would read nothing.
    let reads = ir::Stmt::Assign {
        target: Var::Local(7),
        value: Expr::Var(CARRIED),
    };
    let f = carried(Vec::new(), reads);
    let before = f.cfg.statements();
    let out = run(f, &Names::default(), &mut Stats::default());
    assert_eq!(tree::counts(&out.body).statements, before);
    assert!(out.head.vars.contains_key(&CARRIED));
}

#[test]
fn a_value_made_after_another_statement_stays_in_its_variable() {
    // The first arm runs `s8` before it makes the value: the arm would be left a test with a
    // statement in front, which reads no better than the variable.
    let f = carried(vec![set(8)], set(1));
    let before = f.cfg.statements();
    let out = run(f, &Names::default(), &mut Stats::default());
    assert_eq!(tree::counts(&out.body).statements, before);
    assert!(out.head.vars.contains_key(&CARRIED));
}

#[test]
fn a_constant_decides_a_test_only_where_the_test_can_be_worked_out() {
    let test = |op: BinOp| Expr::binary(op, Expr::cast(Scalar::U32, Expr::Var(CARRIED)), number(0));
    assert_eq!(cond::tested_temporary(&test(BinOp::Ne)), Some(CARRIED));
    assert_eq!(cond::holds_for(&test(BinOp::Ne), 0), Some(false));
    assert_eq!(cond::holds_for(&test(BinOp::Ne), 1), Some(true));
    assert_eq!(cond::holds_for(&test(BinOp::Eq), 0), Some(true));
    // Only the low 32 bits are read.
    assert_eq!(cond::holds_for(&test(BinOp::Ne), 1 << 32), Some(false));
    assert_eq!(cond::holds_for(&not(Expr::Var(CARRIED)), 5), Some(false));

    // A test that reads anything else, or a local, is no such test.
    let other = Expr::binary(BinOp::Ne, Expr::Var(CARRIED), p(1));
    assert_eq!(cond::tested_temporary(&other), None);
    let local = Expr::binary(BinOp::Ne, Expr::Var(Var::Local(3)), number(0));
    assert_eq!(cond::tested_temporary(&local), None);
}

// ---- return, goto and what is left over ---------------------------------------------------

/// A loop that leaves from its middle to the function's `return`, which is `last`.
fn leaves_from_a_loop(last: Block) -> Vec<Block> {
    // 0: s0 -> 1    1: p0 ? 2 : 4    2: s1; p1 ? 5 : 3    3: s2 -> 1    4: s3 -> 5    5: last
    vec![
        jump(&[0], 1),
        branch(&[], p(0), 2, 4),
        branch(&[1], p(1), 5, 3),
        jump(&[2], 1),
        jump(&[3], 5),
        last,
    ]
}

#[test]
fn a_jump_to_a_bare_return_is_that_return() {
    let lp = Stmt::While {
        cond: p(0),
        body: vec![s(1), when(p(1), vec![returns()], Vec::new()), s(2)],
    };
    assert_eq!(
        structured(leaves_from_a_loop(ret(&[]))),
        [s(0), lp, s(3), returns()]
    );
}

#[test]
fn a_jump_to_a_return_that_calls_stays_a_goto() {
    // Written at the jump too, the call would be in the source twice.
    let call = Expr::Call {
        callee: Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        },
        args: Vec::new(),
        returns: None,
    };
    let last = block(&[], Terminator::Return(Some(call.clone())));
    let end = Label::Block(5);
    let lp = Stmt::While {
        cond: p(0),
        body: vec![s(1), when(p(1), vec![Stmt::Goto(end)], Vec::new()), s(2)],
    };
    assert_eq!(
        structured(leaves_from_a_loop(last)),
        [s(0), lp, s(3), Stmt::Label(end), Stmt::Return(Some(call))]
    );
}

#[test]
fn a_shape_no_construct_explains_keeps_its_jumps() {
    // A ring of two blocks entered at both: 0: p0 ? 1 : 2    1: s1; p1 ? 2 : 3    2: s2; p2 ? 1 : 3
    let blocks = vec![
        branch(&[0], p(0), 1, 2),
        branch(&[1], p(1), 2, 3),
        branch(&[2], p(2), 1, 3),
        ret(&[]),
    ];
    let body = structured(blocks);
    let counts = tree::counts(&body);
    assert!(counts.gotos >= 1, "{body:#?}");
    assert!(
        counts.labels <= counts.gotos,
        "a label nothing names: {body:#?}"
    );
}

#[test]
fn a_void_function_loses_only_its_last_return() {
    let blocks = vec![
        branch(&[], p(0), 1, 2),
        block(&[1], Terminator::Return(None)),
        block(&[2], Terminator::Return(None)),
    ];
    let early = when(p(0), vec![s(1), Stmt::Return(None)], Vec::new());
    let void = run(
        function(blocks.clone(), Returns::Void),
        &Names::default(),
        &mut Stats::default(),
    );
    assert_eq!(void.body, [early.clone(), s(2)]);
    // A function that returns a value states every return.
    assert_eq!(rebuilt(blocks), [early, s(2), Stmt::Return(None)]);
}

#[test]
fn nesting_stops_at_its_bound_and_the_rest_is_written_with_jumps() {
    // 300 ifs, each inside the one before, each with a statement so that none joins another.
    let n = 300;
    let mut blocks: Vec<Block> = (0..n)
        .map(|i| branch(&[i as u32], p(0), i + 1, n + 1))
        .collect();
    blocks.push(jump(&[n as u32], n + 1));
    blocks.push(ret(&[]));
    let body = structured(blocks);
    assert!(depth(&body) <= region::MAX_NEST + 1, "{}", depth(&body));
    assert!(depth(&body) >= region::MAX_NEST - 1, "{}", depth(&body));
}

#[test]
fn code_nothing_reaches_keeps_its_statements() {
    // Block 1 is reached by nothing and loops on itself.
    let blocks = vec![ret(&[0]), jump(&[1], 1)];
    let body = rebuilt(blocks);
    assert_eq!(tree::counts(&body).statements, 2, "{body:#?}");
    assert!(cfg::lower_body(body).is_ok());
}

// ---- the sample listing -------------------------------------------------------------------

#[test]
fn every_function_of_the_sample_keeps_its_statements_and_lowers_again() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    let mut seen = 0;
    for (f, statements) in listing.functions().zip(sample::STATEMENTS) {
        let f = f.expect("the function reads");
        let lowered = cfg::lower(f).expect("the function lowers");
        let out = run(lowered, &Names::default(), &mut Stats::default());
        let counts = tree::counts(&out.body);
        assert_eq!(counts.statements, statements, "{}", out.head.name);
        assert_eq!(counts.gotos, 0, "{}: {:#?}", out.head.name, out.body);
        assert_eq!(counts.labels, 0, "{}", out.head.name);
        assert!(
            cfg::lower_body(out.body).is_ok(),
            "what the pass wrote does not lower"
        );
        seen += 1;
    }
    assert_eq!(seen, sample::STATEMENTS.len());
}

#[test]
fn the_samples_loops_come_out_as_loops() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    let functions: Vec<tree::Function> = listing
        .functions()
        .map(|f| {
            let lowered = cfg::lower(f.expect("the function reads")).expect("it lowers");
            run(lowered, &Names::default(), &mut Stats::default())
        })
        .collect();
    // `label: ...; if (!(a < b)) goto out; else goto label;` is a loop tested at its bottom.
    let second = &functions[1].body;
    assert!(
        matches!(
            second.as_slice(),
            [Stmt::DoWhile { .. }, Stmt::Return(Some(_))]
        ),
        "{second:#?}"
    );
    // `while (true) { if (!(local_0 < 10)) break; ... }` is tested at its top.
    let third = &functions[2].body;
    assert!(
        matches!(
            third.as_slice(),
            [Stmt::Simple(_), Stmt::While { .. }, Stmt::Return(Some(_))]
        ),
        "{third:#?}"
    );
    assert!(
        !shows(third, "Truth"),
        "the loop kept `while (true)`: {third:#?}"
    );
    // The last function is `return;` and nothing else.
    assert!(functions[4].body.is_empty());
}

// ---- the `gotos` rules --------------------------------------------------------------------

/// The pass with the `gotos` rules on: the result must be the same flow, with no statement lost.
fn untangled(blocks: Vec<Block>) -> Vec<Stmt> {
    let before = Cfg {
        blocks: blocks.clone(),
    };
    let names = Names::default();
    let options = crate::gta6::script::Options::baseline().with(Pass::Gotos);
    let cx = Context::new(&names, crate::gta6::script::Project::empty(), options);
    let f = function(blocks, Returns::Cell);
    let body = run_in(f, &cx, &mut Stats::default()).body;
    let after = cfg::lower_body(body.clone()).expect("the body lowers");
    if let Err(why) = same_flow(&before, &after) {
        panic!("the flow changed: {why}\n{body:#?}");
    }
    assert!(tree::counts(&body).statements >= before.statements());
    body
}

fn gotos(body: &[Stmt]) -> usize {
    tree::counts(body).gotos
}

/// A then arm whose last block returns, and whose blocks 1 and 2 leave to `first` and `second`, past the code after the `if`.
fn arm_that_leaves_to(first: BlockId, second: BlockId, last: &[u32]) -> Vec<Block> {
    // 0: s0; p0 ? 1 : 4    1: s1; p1 ? first : 2    2: s2; p2 ? 3 : second    3: s3; return
    // 4: s4 -> 5    5: s5 -> 6    6: last; return
    vec![
        branch(&[0], p(0), 1, 4),
        branch(&[1], p(1), first, 2),
        branch(&[2], p(2), 3, second),
        ret(&[3]),
        jump(&[4], 5),
        jump(&[5], 6),
        ret(last),
    ]
}

#[test]
fn the_one_place_a_then_arm_leaves_to_makes_what_is_in_between_its_else() {
    // `if (p0) { s1; if (p1) goto L; s2; if (!p2) goto L; s3; return; } s4; s5; L: s6; return;`
    let blocks = arm_that_leaves_to(6, 6, &[6]);
    assert_eq!(gotos(&rebuilt(blocks.clone())), 2, "the rules ran unasked");
    let inner = when(p(2), vec![s(3), returns()], Vec::new());
    let then = vec![s(1), when(not(p(1)), vec![s(2), inner], Vec::new())];
    assert_eq!(
        untangled(blocks),
        [s(0), when(p(0), then, vec![s(4), s(5)]), s(6), returns()]
    );
}

#[test]
fn a_then_arm_that_leaves_to_two_places_takes_no_else() {
    // Blocks 1 and 2 leave to 5 and to 6: an else that ends at either leaves the other a jump into an arm.
    let blocks = arm_that_leaves_to(5, 6, &[6, 7, 8]);
    let plain = structured(blocks.clone());
    assert_eq!(gotos(&plain), 2);
    assert_eq!(untangled(blocks), plain);
}

/// `s1; switch (p1) { 0 -> 2, 1 -> 3, default -> 4 }`, as block 1.
fn inner_switch() -> Block {
    block(
        &[1],
        Terminator::Switch {
            value: p(1),
            cases: vec![(0, 2), (1, 3)],
            default: 4,
        },
    )
}

#[test]
fn bodies_that_leave_past_their_case_are_the_inner_switchs_breaks() {
    // Each inner body leaves to what follows the outer switch: `break` out of the inner one, then out of the outer.
    // 0: s0; switch (p0) { 0 -> 1, 1 -> 5, default -> 6 }    1: the inner switch
    // 2: s2 -> 6    3: s3 -> 6    4: s4 -> 6    5: s5 -> 6    6: s6; s7; s8; return
    let blocks = vec![
        block(
            &[0],
            Terminator::Switch {
                value: p(0),
                cases: vec![(0, 1), (1, 5)],
                default: 6,
            },
        ),
        inner_switch(),
        jump(&[2], 6),
        jump(&[3], 6),
        jump(&[4], 6),
        jump(&[5], 6),
        ret(&[6, 7, 8]),
    ];
    assert_eq!(gotos(&rebuilt(blocks.clone())), 2, "the rules ran unasked");
    let inner = Stmt::Switch {
        value: p(1),
        cases: vec![
            case(&[Some(0)], vec![s(2), Stmt::Break]),
            case(&[Some(1)], vec![s(3), Stmt::Break]),
            case(&[None], vec![s(4), Stmt::Break]),
        ],
    };
    let outer = Stmt::Switch {
        value: p(0),
        cases: vec![
            case(&[Some(0)], vec![s(1), inner, Stmt::Break]),
            case(&[Some(1)], vec![s(5), Stmt::Break]),
        ],
    };
    assert_eq!(
        untangled(blocks),
        [s(0), outer, s(6), s(7), s(8), returns()]
    );
}

#[test]
fn a_switch_one_body_of_which_leaves_to_what_follows_it_keeps_its_end() {
    // The second body leaves to block 4, the code after the switch, which runs on to the join
    // at 5: were the switch to end past its arm for the first body's sake, that code would be
    // its last body and need a `goto` out of it.
    // 0: s0; p0 ? 1 : 5    1: the switch    2: s2 -> 6    3: s3 -> 4    4: s4 -> 5    5: s5 -> 6
    // 6: s6; s7; s8; return
    let blocks = vec![
        branch(&[0], p(0), 1, 5),
        inner_switch(),
        jump(&[2], 6),
        jump(&[3], 4),
        jump(&[4], 5),
        jump(&[5], 6),
        ret(&[6, 7, 8]),
    ];
    let plain = structured(blocks.clone());
    assert_eq!(gotos(&plain), 1);
    assert_eq!(untangled(blocks), plain);
}

#[test]
fn a_loop_takes_in_the_tail_its_last_test_leaves_to() {
    // `do { s1; if (p1) goto L; s2; } while (p2); s3; return; L: s4; s5; s6; return;`
    // 0: s0 -> 1    1: s1; p1 ? 4 : 2    2: s2; p2 ? 1 : 3    3: s3; return    4: s4; s5; s6; return
    let blocks = vec![
        jump(&[0], 1),
        branch(&[1], p(1), 4, 2),
        branch(&[2], p(2), 1, 3),
        ret(&[3]),
        ret(&[4, 5, 6]),
    ];
    assert_eq!(gotos(&rebuilt(blocks.clone())), 1, "the rules ran unasked");
    let lp = Stmt::While {
        cond: forever(),
        body: vec![
            s(1),
            when(p(1), vec![Stmt::Break], Vec::new()),
            s(2),
            when(not(p(2)), vec![s(3), returns()], Vec::new()),
        ],
    };
    assert_eq!(untangled(blocks), [s(0), lp, s(4), s(5), s(6), returns()]);
}

#[test]
fn a_tail_that_runs_on_to_where_the_loop_leaves_ends_in_a_break() {
    // `while (true) { s1; if (p1) goto L; s2; if (p2) break; s3; } s4; L: s5; s6; s7; return;`
    // 0: s0 -> 1    1: s1; p1 ? 5 : 2    2: s2; p2 ? 4 : 3    3: s3 -> 1    4: s4 -> 5    5: s5; s6; s7; return
    let blocks = vec![
        jump(&[0], 1),
        branch(&[1], p(1), 5, 2),
        branch(&[2], p(2), 4, 3),
        jump(&[3], 1),
        jump(&[4], 5),
        ret(&[5, 6, 7]),
    ];
    assert_eq!(gotos(&rebuilt(blocks.clone())), 1, "the rules ran unasked");
    let lp = Stmt::While {
        cond: forever(),
        body: vec![
            s(1),
            when(p(1), vec![Stmt::Break], Vec::new()),
            s(2),
            when(not(p(2)), vec![s(3)], vec![s(4), Stmt::Break]),
        ],
    };
    assert_eq!(untangled(blocks), [s(0), lp, s(5), s(6), s(7), returns()]);
}

#[test]
fn a_tail_two_blocks_of_a_loop_lead_to_stays_after_it() {
    // Blocks 2 and 3 both leave to the tail at 4: inside the loop, the first of them would be a `goto`.
    // 0: s0 -> 1    1: s1; p1 ? 5 : 2    2: s2; p2 ? 4 : 3    3: s3; p3 ? 1 : 4    4: s4; return    5: s5; s6; s7; return
    let blocks = vec![
        jump(&[0], 1),
        branch(&[1], p(1), 5, 2),
        branch(&[2], p(2), 4, 3),
        branch(&[3], p(3), 1, 4),
        ret(&[4]),
        ret(&[5, 6, 7]),
    ];
    let plain = structured(blocks.clone());
    assert_eq!(gotos(&plain), 1);
    assert_eq!(untangled(blocks), plain);
}

#[test]
fn a_goto_to_the_statement_after_it_goes() {
    // The loop's test leaves to block 4, and the `return` at 3, written at the jump to it, stands in between.
    // 0: s0 -> 1    1: p1 ? 2 : 4    2: s2; p2 ? 3 : 1    3: return    4: s4; return
    let blocks = vec![
        jump(&[0], 1),
        branch(&[], p(1), 2, 4),
        branch(&[2], p(2), 3, 1),
        ret(&[]),
        ret(&[4]),
    ];
    assert_eq!(gotos(&rebuilt(blocks.clone())), 1, "the rules ran unasked");
    let lp = Stmt::While {
        cond: p(1),
        body: vec![s(2), when(p(2), vec![returns()], Vec::new())],
    };
    assert_eq!(untangled(blocks), [s(0), lp, s(4), returns()]);
}

fn named(label: Label, gotos: u32) -> HashMap<Label, u32> {
    HashMap::from([(label, gotos)])
}

#[test]
fn a_goto_with_a_statement_before_its_label_stays() {
    let end = Label::Block(7);
    let body = vec![Stmt::Goto(end), s(1), Stmt::Label(end), s(2)];
    let mut left = body.clone();
    assert!(!tails::next_gotos(&mut left, &mut named(end, 1)));
    assert_eq!(left, body);
    // Only labels between the jump and where it lands: it says nothing.
    let mut bare = vec![
        Stmt::Goto(end),
        Stmt::Label(Label::Block(8)),
        Stmt::Label(end),
        s(2),
    ];
    let mut names = named(end, 1);
    assert!(tails::next_gotos(&mut bare, &mut names));
    assert_eq!(bare, [Stmt::Label(Label::Block(8)), Stmt::Label(end), s(2)]);
    assert_eq!(names[&end], 0);
}

#[test]
fn a_goto_to_a_short_tail_that_leaves_is_that_tail() {
    let end = Label::Block(7);
    let mut body = vec![
        when(p(0), vec![Stmt::Goto(end)], Vec::new()),
        s(1),
        returns(),
        Stmt::Label(end),
        s(2),
        returns(),
    ];
    let mut names = named(end, 1);
    assert!(tails::copied(&mut body, &mut names));
    // Nothing runs into the tail any more, so only its copy is left.
    assert_eq!(
        body,
        [
            when(p(0), vec![s(2), returns()], Vec::new()),
            s(1),
            returns()
        ]
    );
    assert_eq!(names[&end], 0);
}

#[test]
fn a_tail_of_more_statements_or_one_that_runs_on_is_not_copied() {
    let end = Label::Block(7);
    let jump_to = when(p(0), vec![Stmt::Goto(end)], Vec::new());
    for tail in [
        vec![s(2), s(3), s(4), returns()],
        vec![s(2), s(3)],
        vec![s(2), when(p(1), vec![s(3)], Vec::new()), returns()],
    ] {
        let mut body = vec![jump_to.clone(), s(1), returns(), Stmt::Label(end)];
        body.extend(tail);
        let was = body.clone();
        assert!(!tails::copied(&mut body, &mut named(end, 1)));
        assert_eq!(body, was);
    }
}

#[test]
fn a_tail_that_breaks_is_copied_only_inside_what_it_leaves() {
    let end = Label::Block(7);
    let jump_to = when(p(0), vec![Stmt::Goto(end)], Vec::new());
    let mut inside = vec![Stmt::While {
        cond: p(1),
        body: vec![
            jump_to.clone(),
            s(1),
            Stmt::Continue,
            Stmt::Label(end),
            s(2),
            Stmt::Break,
        ],
    }];
    assert!(tails::copied(&mut inside, &mut named(end, 1)));
    assert_eq!(
        inside,
        [Stmt::While {
            cond: p(1),
            body: vec![
                when(p(0), vec![s(2), Stmt::Break], Vec::new()),
                s(1),
                Stmt::Continue
            ],
        }]
    );
    // From outside the loop a `break` would leave something else, or nothing.
    let mut outside = vec![
        jump_to,
        Stmt::While {
            cond: p(1),
            body: vec![s(1), Stmt::Label(end), s(2), Stmt::Break],
        },
    ];
    let was = outside.clone();
    assert!(!tails::copied(&mut outside, &mut named(end, 1)));
    assert_eq!(outside, was);
}

#[test]
fn a_tail_that_code_runs_into_stays_where_it_is() {
    // Blocks 1 and 2 leave to 5 and to 6; `s6; return;` at 6 is short, and `s5` runs into it.
    let body = untangled(arm_that_leaves_to(5, 6, &[6]));
    let five = Label::Block(5);
    let then = vec![
        s(1),
        when(p(1), vec![Stmt::Goto(five)], Vec::new()),
        s(2),
        when(not(p(2)), vec![s(6), returns()], Vec::new()),
        s(3),
        returns(),
    ];
    assert_eq!(
        body,
        [
            s(0),
            when(p(0), then, Vec::new()),
            s(4),
            Stmt::Label(five),
            s(5),
            s(6),
            returns()
        ]
    );
}

// ---- the `gotos` rules: a switch cut in two, and the ways out ------------------------------

/// The pass with the `gotos` rules on, with no check of the result: a tree of tests read as a switch is not the flow the check compares.
fn unknotted(blocks: Vec<Block>) -> Vec<Stmt> {
    let names = Names::default();
    let options = crate::gta6::script::Options::baseline().with(Pass::Gotos);
    let cx = Context::new(&names, crate::gta6::script::Project::empty(), options);
    run_in(function(blocks, Returns::Cell), &cx, &mut Stats::default()).body
}

/// A tree of tests of `p0` whose two halves each leave to a block of their own, 9 and `last`.
fn halves(last: Block) -> Vec<Block> {
    // 0: p0 > 20 ? 3 : 1    1: p0 == 10 ? 5 : 2    2: p0 == 20 ? 6 : 9
    // 3: p0 == 30 ? 7 : 4   4: p0 == 40 ? 8 : 10   5..8: sN; return    9: return    10: last
    let over = Expr::binary(BinOp::Gt, int(p(0)), number(20));
    let mut blocks = vec![
        branch(&[0], over, 3, 1),
        branch(&[], is(10), 5, 2),
        branch(&[], is(20), 6, 9),
        branch(&[], is(30), 7, 4),
        branch(&[], is(40), 8, 10),
    ];
    blocks.extend((1..=4).map(|n| ret(&[n])));
    blocks.push(ret(&[]));
    blocks.push(last);
    blocks
}

#[test]
fn ways_out_to_bare_returns_of_one_value_are_one_default() {
    let mut switched = halves(ret(&[]));
    graph::switch_trees(&mut switched, true);
    // The four tests under the first are gone, and so is the return nothing leads to any more.
    assert_eq!(
        switched[0].term,
        Terminator::Switch {
            value: int(p(0)),
            cases: vec![(10, 1), (20, 2), (30, 3), (40, 4)],
            default: 5,
        }
    );
    assert_eq!(switched.len(), 6);
    assert_eq!(switched[5], ret(&[]));

    // Unasked, two blocks are two ways out.
    let mut plain = halves(ret(&[]));
    graph::switch_trees(&mut plain, false);
    assert_eq!(plain, halves(ret(&[])));

    // A return of another value, and a return behind a statement, are other ways out.
    for last in [block(&[], Terminator::Return(Some(p(8)))), ret(&[6])] {
        let mut kept = halves(last.clone());
        graph::switch_trees(&mut kept, true);
        assert_eq!(kept, halves(last));
    }
}

#[test]
fn a_switch_cut_in_two_with_a_return_in_each_half_is_one_switch() {
    let plain = rebuilt(halves(ret(&[])));
    assert!(!shows(&plain, "Switch"), "the rule ran unasked: {plain:#?}");
    let switch = Stmt::Switch {
        value: int(p(0)),
        cases: vec![
            case(&[Some(10)], vec![s(1), returns()]),
            case(&[Some(20)], vec![s(2), returns()]),
            case(&[Some(30)], vec![s(3), returns()]),
            case(&[Some(40)], vec![s(4), returns()]),
        ],
    };
    assert_eq!(unknotted(halves(ret(&[]))), [s(0), switch, returns()]);
}

#[test]
fn a_return_an_arm_runs_on_to_is_a_guard_in_front_of_the_arm() {
    // `if (p0) { s1; if (p1) { s2; if (p2) goto L; } return; } L: s4; s5; s6; return;`
    // 0: s0; p0 ? 1 : 4    1: s1; p1 ? 2 : 3    2: s2; p2 ? 4 : 3    3: return    4: s4; s5; s6; return
    let blocks = vec![
        branch(&[0], p(0), 1, 4),
        branch(&[1], p(1), 2, 3),
        branch(&[2], p(2), 4, 3),
        ret(&[]),
        ret(&[4, 5, 6]),
    ];
    assert_eq!(gotos(&rebuilt(blocks.clone())), 1, "the rules ran unasked");
    let guard = |n: u32| when(not(p(n)), vec![returns()], Vec::new());
    let then = vec![s(1), guard(1), s(2), guard(2)];
    assert_eq!(
        untangled(blocks),
        [
            s(0),
            when(p(0), then, Vec::new()),
            s(4),
            s(5),
            s(6),
            returns()
        ]
    );
}
