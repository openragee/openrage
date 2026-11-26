use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Const, ExprSugar, Returns};
use crate::gta6::script::print;
use crate::gta6::script::Names;

/// `local_n = n;`, a statement to tell apart from the others.
fn mark(n: u32) -> ir::Stmt {
    ir::Stmt::Assign {
        target: Var::Local(n),
        value: Expr::Const(Const::i32(n as i32)),
    }
}

fn set(n: u32) -> Stmt {
    Stmt::Simple(mark(n))
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

/// A call of the function at `at` of the program itself.
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

fn boxed(sugar: Sugar) -> Stmt {
    Stmt::Sugar(Box::new(sugar))
}

/// The locals a body's marks assign, in the order the statements stand.
fn marks(body: &[Stmt]) -> Vec<u32> {
    let mut found = Vec::new();
    each_simple(body, &mut |s| {
        if let ir::Stmt::Assign {
            target: Var::Local(n),
            ..
        } = s
        {
            found.push(*n);
        }
    });
    found
}

/// The source constructs a body holds, at any depth.
fn constructs(body: &mut Vec<Stmt>) -> usize {
    let mut found = 0;
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            found += usize::from(matches!(stmt, Stmt::Sugar(_)));
            work.extend(nested_mut(stmt));
        }
    }
    found
}

fn awaited() -> Sugar {
    Sugar::Await {
        before: vec![mark(1)],
        ready: call(0x100, vec![local(1)]),
        suspend: vec![set(2), Stmt::Return(None)],
        resumed: Label::Block(7),
        resume: vec![set(3)],
        after: vec![mark(4)],
    }
}

fn foreach(step: Option<Box<ir::Stmt>>) -> Sugar {
    Sugar::Foreach {
        setup: vec![mark(1)],
        cond: p(0),
        item: Box::new(mark(2)),
        body: vec![set(3)],
        step,
        after: vec![set(5)],
    }
}

fn head() -> Head {
    Head {
        program: 1,
        at: 0x10,
        name: "func_000010".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 8,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

#[test]
fn a_construct_is_written_back_as_the_statements_it_holds_in_the_order_they_ran() {
    let yielded = Sugar::Yield {
        value: vec![mark(1)],
        suspend: vec![set(2), Stmt::Return(None)],
        resumed: Label::Block(7),
        resume: vec![set(3)],
    };
    let guarded = Sugar::Guarded {
        kind: Guard::Using,
        enter: vec![mark(1)],
        body: vec![set(2)],
        exit: vec![set(3)],
    };
    let tried = Sugar::Try {
        body: vec![set(1)],
        catches: Vec::new(),
        finally: vec![set(2)],
    };
    let chain = Sugar::SwitchOn {
        arms: vec![
            Arm {
                test: p(0),
                body: vec![set(1)],
            },
            Arm {
                test: p(1),
                body: vec![set(2)],
            },
        ],
        default: vec![set(3)],
    };
    let group = Sugar::Group {
        kind: Group::Construct,
        parts: vec![mark(1), mark(2), mark(3)],
    };
    let cases: Vec<(Sugar, Vec<u32>)> = vec![
        (Sugar::Hidden(vec![set(1), set(2)]), vec![1, 2]),
        (group, vec![1, 2, 3]),
        (awaited(), vec![1, 2, 3, 4]),
        (yielded, vec![1, 2, 3]),
        (foreach(None), vec![1, 2, 3, 5]),
        (foreach(Some(Box::new(mark(4)))), vec![1, 2, 3, 4, 5]),
        (guarded, vec![1, 2, 3]),
        (tried, vec![1, 2]),
        (Sugar::Throw(Box::new(mark(1))), vec![1]),
        (chain, vec![1, 2, 3]),
    ];
    for (sugar, order) in cases {
        let held = counts(&[boxed(sugar.clone())]);
        let lowered = sugar.clone().desugar();
        assert_eq!(constructs(&mut lowered.clone()), 0, "{sugar:?}");
        assert_eq!(marks(&lowered), order, "{sugar:?}");
        // Nothing is made and nothing is lost: a construct counts as the statements it holds.
        assert_eq!(counts(&lowered).statements, held.statements, "{sugar:?}");
        assert_eq!(held.statements, order.len(), "{sugar:?}");
    }
}

#[test]
fn a_throw_ends_what_runs_and_a_handler_is_not_written_back() {
    let raised = Sugar::Throw(Box::new(mark(1))).desugar();
    assert_eq!(raised, vec![set(1), Stmt::Unreachable]);

    // ⚠ The check that runs through `desugar` raises no fault, so it runs no handler.
    let caught = Catch {
        ty: None,
        var: None,
        body: vec![set(9)],
    };
    let tried = Sugar::Try {
        body: vec![set(1)],
        catches: vec![caught],
        finally: vec![set(2)],
    };
    assert_eq!(marks(&tried.desugar()), vec![1, 2]);
}

#[test]
fn a_source_reading_comes_off_an_expression_and_leaves_it_as_it_was() {
    let called = call(0x200, vec![p(0), p(1)]);
    let method = Expr::sugar(ExprSugar::Method, called.clone());
    let mut read = Expr::sugar(ExprSugar::Tuple, Expr::Cells(vec![method, p(2)]));
    read.desugar();
    assert_eq!(read, Expr::Cells(vec![called, p(2)]));
}

#[test]
fn a_body_is_written_back_at_every_depth_and_lowers_as_what_it_stands_for() {
    // A construct in a construct in an `if`, and a source reading in a statement and in a test.
    let read = Expr::sugar(ExprSugar::Method, call(0x200, vec![p(0), p(1)]));
    let inner = Sugar::Group {
        kind: Group::Compound,
        parts: vec![ir::Stmt::Eval(read)],
    };
    let outer = Sugar::Guarded {
        kind: Guard::Lock,
        enter: vec![mark(5)],
        body: vec![boxed(inner), boxed(awaited())],
        exit: vec![set(6)],
    };
    let body = vec![
        Stmt::If {
            cond: Expr::sugar(ExprSugar::Coalesce, p(2)),
            then: vec![boxed(outer)],
            els: Vec::new(),
        },
        Stmt::Return(None),
    ];
    assert_eq!(constructs(&mut body.clone()), 3);

    let lowered = desugar(body.clone());
    assert_eq!(constructs(&mut lowered.clone()), 0);
    let mut readings = 0;
    each_expr(&lowered, &mut |top| {
        top.walk(&mut |e| readings += usize::from(matches!(e, Expr::Sugar { .. })));
    });
    assert_eq!(readings, 0);
    assert_eq!(counts(&lowered).statements, counts(&body).statements);

    // Lowering writes a construct back itself: the blocks are those of the statements it holds.
    let held = vec![boxed(awaited()), Stmt::Return(None)];
    let blocks = cfg::lower_body(held.clone());
    assert!(blocks.is_ok(), "{blocks:?}");
    assert_eq!(blocks, cfg::lower_body(desugar(held)));
}

#[test]
fn a_construct_prints_its_keyword_and_a_listing_prints_what_it_stands_for() {
    let enumerator = ir::Stmt::Assign {
        target: Var::Local(1),
        value: call(0x100, vec![p(0)]),
    };
    let item = ir::Stmt::Assign {
        target: Var::Local(2),
        value: call(0x108, vec![local(1)]),
    };
    let looped = Sugar::Foreach {
        setup: vec![enumerator],
        cond: call(0x104, vec![local(1)]),
        item: Box::new(item),
        body: vec![Stmt::Simple(ir::Stmt::Eval(call(0x200, vec![local(2)])))],
        step: None,
        after: Vec::new(),
    };
    let raised = Sugar::Throw(Box::new(ir::Stmt::Eval(call(0x300, vec![p(1)]))));
    let body = vec![boxed(looped), boxed(raised)];
    let f = Function {
        head: head(),
        body: body.clone(),
    };

    let mut readable = String::new();
    print::function(&mut readable, &f, &Names::default());
    assert!(
        readable.contains("foreach (var local_2 in p0)"),
        "{readable}"
    );
    assert!(readable.contains("func_000200(local_2);"), "{readable}");
    assert!(readable.contains("throw p1;"), "{readable}");
    // Taking the enumerator and asking it for the next item are the construct's own.
    assert!(!readable.contains("func_000104"), "{readable}");

    // The listing's syntax has no `foreach`: it reads as the loop the construct stands for.
    let mut plain = print::ListingWriter::new(1, &[]);
    plain.function(&f);
    let mut lowered = print::ListingWriter::new(1, &[]);
    lowered.function(&Function {
        head: head(),
        body: desugar(body),
    });
    let plain = plain.finish();
    assert!(plain.contains("while (func_000104(local_1))"), "{plain}");
    assert_eq!(plain, lowered.finish());
}
