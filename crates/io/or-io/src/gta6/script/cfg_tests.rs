use super::*;
use crate::gta6::script::ir::{Const, Var};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::sample;

/// `local_n = n;`, a statement to tell apart from the others.
fn set(n: u32) -> tree::Stmt {
    tree::Stmt::Simple(mark(n))
}

fn mark(n: u32) -> Stmt {
    Stmt::Assign {
        target: Var::Local(n),
        value: Expr::Const(Const::i32(n as i32)),
    }
}

fn cond() -> Expr {
    Expr::Var(Var::Param(0))
}

fn lowered(body: Vec<tree::Stmt>) -> Cfg {
    lower_body(body).expect("the body lowers")
}

#[test]
fn lowering_keeps_every_statement_of_the_sample() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    let mut functions = 0;
    for (f, statements) in listing.functions().zip(sample::STATEMENTS) {
        let f = f.expect("the function reads");
        let cfg = lower(f).expect("the function lowers").cfg;
        assert_eq!(cfg.statements(), statements);
        assert!(
            cfg.reachable().iter().all(|r| *r),
            "the sample holds no dead code, and a block it does not reach is one lowering made up"
        );
        for (b, block) in cfg.blocks.iter().enumerate() {
            for target in block.term.successors() {
                assert!(
                    target < cfg.blocks.len(),
                    "block {b} jumps out of the function"
                );
            }
        }
        functions += 1;
    }
    assert_eq!(functions, sample::STATEMENTS.len());
}

#[test]
fn an_if_lowers_to_a_diamond() {
    let cfg = lowered(vec![
        set(0),
        tree::Stmt::If {
            cond: cond(),
            then: vec![set(1)],
            els: vec![set(2)],
        },
        set(3),
        tree::Stmt::Return(None),
    ]);
    let block = |n: u32, term: Terminator| Block {
        at: None,
        stmts: vec![mark(n)],
        term,
    };
    assert_eq!(
        cfg.blocks,
        [
            block(
                0,
                Terminator::Branch {
                    cond: cond(),
                    then: 1,
                    els: 2
                }
            ),
            block(1, Terminator::Jump(3)),
            block(2, Terminator::Jump(3)),
            block(3, Terminator::Return(None)),
        ]
    );
    let preds = cfg.predecessors();
    assert!(preds[0].is_empty());
    assert_eq!(preds[1], [0]);
    assert_eq!(preds[2], [0]);
    assert_eq!(preds[3], [1, 2]);
    assert_eq!(cfg.reverse_post_order(), [0, 2, 1, 3]);
    let idom = cfg.dominators();
    assert_eq!(idom, [Some(0), Some(0), Some(0), Some(0)]);
    assert!(dominates(&idom, 0, 3));
    assert!(!dominates(&idom, 1, 3));
    assert!(dominates(&idom, 3, 3));
}

#[test]
fn an_arm_with_nothing_in_it_is_no_block() {
    let cfg = lowered(vec![
        tree::Stmt::If {
            cond: cond(),
            then: Vec::new(),
            els: vec![set(1)],
        },
        tree::Stmt::Return(None),
    ]);
    assert_eq!(cfg.blocks.len(), 3);
    assert_eq!(
        cfg.blocks[0].term,
        Terminator::Branch {
            cond: cond(),
            then: 2,
            els: 1
        }
    );
    assert_eq!(cfg.blocks[1].term, Terminator::Jump(2));
}

#[test]
fn a_loop_that_always_runs_has_no_test() {
    let always = Expr::Truth(Box::new(Expr::Const(Const::bool(true))));
    let cfg = lowered(vec![
        tree::Stmt::While {
            cond: always,
            body: vec![
                set(0),
                tree::Stmt::If {
                    cond: cond(),
                    then: vec![tree::Stmt::Break],
                    els: vec![tree::Stmt::Continue],
                },
            ],
        },
        set(1),
        tree::Stmt::Return(None),
    ]);
    // The entry, the loop's head holding its body, and what follows the loop: `break` leaves to
    // the last and `continue` goes back to the head, with no block of their own.
    assert_eq!(cfg.blocks.len(), 3);
    assert_eq!(cfg.blocks[0].term, Terminator::Jump(1));
    assert_eq!(cfg.blocks[1].stmts, [mark(0)]);
    assert_eq!(
        cfg.blocks[1].term,
        Terminator::Branch {
            cond: cond(),
            then: 2,
            els: 1
        }
    );
    assert_eq!(cfg.blocks[2].stmts, [mark(1)]);
    assert_eq!(cfg.predecessors()[1], [0, 1]);
    assert_eq!(cfg.dominators(), [Some(0), Some(0), Some(1)]);
}

#[test]
fn a_loop_that_does_nothing_is_kept() {
    let always = Expr::Truth(Box::new(Expr::Const(Const::bool(true))));
    let cfg = lowered(vec![tree::Stmt::While {
        cond: always,
        body: Vec::new(),
    }]);
    assert_eq!(cfg.blocks.len(), 2);
    assert_eq!(cfg.blocks[0].term, Terminator::Jump(1));
    assert_eq!(cfg.blocks[1].term, Terminator::Jump(1));
}

#[test]
fn a_tested_loop_and_a_do_loop_and_a_for_loop_lower() {
    let cfg = lowered(vec![
        tree::Stmt::While {
            cond: cond(),
            body: vec![set(0)],
        },
        tree::Stmt::DoWhile {
            body: vec![set(1)],
            cond: cond(),
        },
        tree::Stmt::For {
            init: Some(Box::new(mark(2))),
            cond: Some(cond()),
            step: Some(Box::new(mark(3))),
            body: vec![set(4), tree::Stmt::Continue],
        },
        tree::Stmt::Return(None),
    ]);
    assert_eq!(cfg.statements(), 5);
    let reachable = cfg.reachable();
    assert!(reachable.iter().all(|r| *r), "{cfg:?}");
    let back_edges = cfg
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(b, block)| block.term.successors().into_iter().map(move |s| (b, s)))
        .filter(|(b, s)| s <= b)
        .count();
    assert_eq!(back_edges, 3, "one back edge for each loop");
}

#[test]
fn a_switch_lowers_to_one_block_for_each_case() {
    let cfg = lowered(vec![
        tree::Stmt::Switch {
            value: cond(),
            cases: vec![
                tree::Case {
                    labels: vec![Some(0), Some(2)],
                    body: vec![set(0), tree::Stmt::Break],
                },
                tree::Case {
                    labels: vec![Some(1)],
                    body: vec![set(1)],
                },
                tree::Case {
                    labels: vec![None],
                    body: vec![set(2), tree::Stmt::Break],
                },
            ],
        },
        tree::Stmt::Return(None),
    ]);
    assert_eq!(
        cfg.blocks[0].term,
        Terminator::Switch {
            value: cond(),
            cases: vec![(0, 1), (2, 1), (1, 2)],
            default: 3
        }
    );
    assert_eq!(cfg.blocks[1].term, Terminator::Jump(4));
    assert_eq!(
        cfg.blocks[2].term,
        Terminator::Jump(3),
        "a case that does not leave falls into the next"
    );
    assert_eq!(cfg.blocks[3].term, Terminator::Jump(4));
    assert_eq!(cfg.blocks[4].term, Terminator::Return(None));
    assert_eq!(cfg.successors(0), [1, 2, 3]);
}

#[test]
fn labels_and_gotos_lower_to_jumps_between_labelled_blocks() {
    let (top, end) = (Label::At(0x10), Label::At(0x40));
    let cfg = lowered(vec![
        tree::Stmt::Label(top),
        set(0),
        tree::Stmt::If {
            cond: cond(),
            then: vec![tree::Stmt::Goto(end)],
            els: vec![tree::Stmt::Goto(top)],
        },
        tree::Stmt::Label(end),
        tree::Stmt::Return(Some(cond())),
        tree::Stmt::Unreachable,
    ]);
    let ats: Vec<Option<u32>> = cfg.blocks.iter().map(|b| b.at).collect();
    assert_eq!(ats, [None, Some(0x10), Some(0x40)]);
    assert_eq!(cfg.blocks[0].term, Terminator::Jump(1));
    assert_eq!(
        cfg.blocks[1].term,
        Terminator::Branch {
            cond: cond(),
            then: 2,
            els: 1
        },
        "an arm that is one goto is the branch's own target"
    );
    assert_eq!(cfg.blocks[2].term, Terminator::Return(Some(cond())));
    assert_eq!(cfg.predecessors()[1], [0, 1]);
}

#[test]
fn a_label_with_nothing_under_it_keeps_its_block() {
    let (first, second) = (Label::At(0x10), Label::At(0x18));
    let cfg = lowered(vec![
        tree::Stmt::If {
            cond: cond(),
            then: vec![tree::Stmt::Goto(first)],
            els: Vec::new(),
        },
        tree::Stmt::Label(first),
        tree::Stmt::Label(second),
        tree::Stmt::Return(None),
    ]);
    let ats: Vec<Option<u32>> = cfg.blocks.iter().map(|b| b.at).collect();
    assert_eq!(
        ats,
        [None, Some(0x10), Some(0x18)],
        "an offset the listing states is not lost"
    );
    assert_eq!(cfg.blocks[1].term, Terminator::Jump(2));
}

#[test]
fn dead_code_is_kept_and_leaves_no_edge_into_what_is_reached() {
    let label = Label::At(0x20);
    let cfg = lowered(vec![
        tree::Stmt::Return(None),
        tree::Stmt::Label(label),
        set(0),
        tree::Stmt::Goto(label),
    ]);
    assert_eq!(cfg.blocks.len(), 2);
    assert_eq!(cfg.reachable(), [true, false]);
    assert_eq!(cfg.blocks[1].stmts, [mark(0)]);
    assert_eq!(cfg.blocks[1].term, Terminator::Jump(1));
    assert!(
        cfg.predecessors().iter().all(Vec::is_empty),
        "the dead block's jump to itself is no edge the passes see"
    );
    assert_eq!(cfg.reverse_post_order(), [0]);
    assert_eq!(cfg.dominators(), [Some(0), None]);
}

#[test]
fn a_body_that_cannot_be_cut_is_refused() {
    let label = Label::At(0x20);
    assert_eq!(
        lower_body(vec![tree::Stmt::Label(label), tree::Stmt::Label(label)]),
        Err(LowerError::DuplicateLabel(label))
    );
    assert_eq!(
        lower_body(vec![tree::Stmt::Goto(label)]),
        Err(LowerError::UnknownLabel(label))
    );
    assert_eq!(
        lower_body(vec![tree::Stmt::Break]),
        Err(LowerError::StrayBreak)
    );
    let in_switch = tree::Stmt::Switch {
        value: cond(),
        cases: vec![tree::Case {
            labels: vec![None],
            body: vec![tree::Stmt::Continue],
        }],
    };
    assert_eq!(lower_body(vec![in_switch]), Err(LowerError::StrayContinue));
}
