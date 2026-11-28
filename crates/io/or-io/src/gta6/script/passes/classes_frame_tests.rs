use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::ir::{Callee, FunctionKey, Kind, Returns, Scalar, StoreKind, VarInfo};
use crate::gta6::script::project::{Param, Signature};

fn head() -> Head {
    Head {
        value_types: BTreeMap::new(),
        program: 3,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 8,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

fn address(var: Var) -> Expr {
    Expr::AddrOf { var, cell: 0 }
}

fn local(n: u32) -> Var {
    Var::Local(n)
}

/// A statement that hands `args` to the program's function at 0x80.
fn hands(args: Vec<Expr>) -> Stmt {
    let callee = Callee::Local {
        at: 0x80,
        name: "func_000080".to_owned(),
    };
    Stmt::Simple(ir::Stmt::Eval(Expr::Call {
        callee,
        args,
        returns: None,
    }))
}

/// A load of `ty` at `offset` through `addr`.
fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
    }
}

/// Which of locals 0 to 4 `body` leaves private in a project that says nothing.
fn private(body: &[Stmt], head: &Head) -> Vec<u32> {
    private_in(body, head, &Project::default())
}

fn private_in(body: &[Stmt], head: &Head, project: &Project) -> Vec<u32> {
    let frame = Frame::of(body, head, project);
    (0..5).filter(|n| frame.private(local(*n), head)).collect()
}

/// A project in which the function at 0x80 takes `cells` cells and touches `bytes` bytes through its first, keeping the address where `kept`.
fn reaching(cells: u32, bytes: u32, kept: bool) -> Project {
    let mut project = Project::default();
    let key = FunctionKey::new(3, 0x80);
    let param = |cells: u32| Param {
        kind: Kind::Unknown,
        cells,
    };
    let signature = Signature {
        params: vec![param(cells)],
        returns: 0,
        result: Kind::Unknown,
    };
    project.signatures.insert(key, signature);
    let first = reach::Reach {
        bytes,
        dies: !kept,
        ..reach::Reach::default()
    };
    project.reach.insert(key, BTreeMap::from([(0, first)]));
    project
}

#[test]
fn an_address_used_where_it_is_taken_reaches_the_bytes_it_names_alone() {
    let head = head();
    assert_eq!(private(&[], &head), [0, 1, 2, 3, 4]);
    // Four bytes at byte 8 from local 0 are in local 1; a cast that keeps the cell changes nothing.
    let read = hands(vec![load(Scalar::I32, address(local(0)), 8)]);
    assert_eq!(private(&[read], &head), [0, 2, 3, 4]);
    let cast = Expr::cast(Scalar::U64, address(local(0)));
    let read = hands(vec![load(Scalar::I32, cast, 8)]);
    assert_eq!(private(&[read], &head), [0, 2, 3, 4]);
    // A store of eight bytes at byte 16 of cell 1 from local 0 is local 3; a clear of 16 bytes from local 1 is 1 and 2.
    let stored = Stmt::Simple(ir::Stmt::Store {
        kind: StoreKind::U64,
        addr: Expr::AddrOf {
            var: local(0),
            cell: 1,
        },
        offset: 16,
        value: Expr::Var(Var::Param(1)),
    });
    assert_eq!(private(&[stored], &head), [0, 1, 2, 4]);
    let cleared = Stmt::Simple(ir::Stmt::ZeroBytes {
        addr: address(local(1)),
        bytes: 16,
    });
    assert_eq!(private(&[cleared], &head), [0, 3, 4]);
    // A use in a test and one in a statement of its branch.
    let nested = Stmt::If {
        cond: load(Scalar::U8, address(local(2)), 0),
        then: vec![hands(vec![load(Scalar::I64, address(local(0)), 32)])],
        els: Vec::new(),
    };
    assert_eq!(private(&[nested], &head), [0, 1, 3]);
}

#[test]
fn an_address_handed_on_or_kept_reaches_its_local_and_every_one_after_it() {
    let head = head();
    let handed = hands(vec![address(local(2))]);
    assert_eq!(private(&[handed], &head), [0, 1]);
    let kept = Stmt::Simple(ir::Stmt::Assign {
        target: local(4),
        value: address(local(1)),
    });
    assert_eq!(private(&[kept], &head), [0]);
    // An address moved by a count of bytes is handed on, wherever it then points.
    let moved = Expr::ByteOffset {
        addr: Box::new(address(local(3))),
        offset: Box::new(Expr::Var(Var::Param(1))),
    };
    let read = hands(vec![load(Scalar::I32, moved, 0)]);
    assert_eq!(private(&[read], &head), [0, 1, 2]);
    // A global's address reaches no local.
    let global = Var::Global {
        program: 3,
        offset: 0x10,
        statics: false,
    };
    assert_eq!(
        private(&[hands(vec![address(global)])], &head),
        [0, 1, 2, 3, 4]
    );
}

#[test]
fn an_address_handed_to_a_function_that_keeps_nothing_reaches_what_the_project_bounds() {
    let head = head();
    let handed = [hands(vec![address(local(1))])];
    assert_eq!(
        private_in(&handed, &head, &reaching(1, 8, false)),
        [0, 2, 3, 4]
    );
    assert_eq!(
        private_in(&handed, &head, &reaching(1, 16, false)),
        [0, 3, 4]
    );
    // The address kept, nothing bounding the bytes, a function that takes other cells than the call hands.
    for project in [
        reaching(1, 8, true),
        reaching(1, reach::UNBOUNDED, false),
        reaching(2, 8, false),
    ] {
        assert_eq!(private_in(&handed, &head, &project), [0]);
    }
    // An address of a later cell of the local is no address the table says anything of.
    let later = [hands(vec![Expr::AddrOf {
        var: local(1),
        cell: 1,
    }])];
    assert_eq!(private_in(&later, &head, &reaching(1, 8, false)), [0]);
}

#[test]
fn an_address_of_a_parameter_or_a_temporary_past_its_own_cell_may_reach_any_local() {
    let mut head = head();
    let temporary = Var::Value { at: 0x10, index: 0 };
    head.vars.insert(temporary, VarInfo::new(Width::Cell));
    // Read in place within its one cell, a temporary's address reaches nothing else.
    let within = hands(vec![load(Scalar::I32, address(temporary), 4)]);
    assert_eq!(private(&[within], &head), [0, 1, 2, 3, 4]);
    let past = hands(vec![load(Scalar::I32, address(temporary), 8)]);
    let handed = hands(vec![address(temporary)]);
    // A parameter the head says nothing of has no cell an address of it is known to stay in.
    let parameter = hands(vec![load(Scalar::I32, address(Var::Param(1)), 0)]);
    for body in [past, handed, parameter] {
        assert_eq!(
            private(std::slice::from_ref(&body), &head),
            Vec::<u32>::new(),
            "{body:?}"
        );
    }
}

#[test]
fn only_a_local_of_one_cell_is_private() {
    let mut head = head();
    head.vars.insert(local(1), VarInfo::new(Width::Cells));
    assert_eq!(private(&[], &head), [0, 2, 3, 4]);
    let frame = Frame::of(&[], &head, &Project::default());
    assert!(!frame.private(Var::Param(0), &head));
    assert!(!frame.private(Var::Value { at: 0x10, index: 0 }, &head));
}

#[test]
fn a_jump_or_a_label_anywhere_in_a_body_runs_it_out_of_the_order_it_is_written_in() {
    let label = tree::Label::Block(1);
    let cond = Expr::Var(Var::Param(1));
    let nested = |inner: Stmt| Stmt::If {
        cond: cond.clone(),
        then: Vec::new(),
        els: vec![Stmt::While {
            cond: cond.clone(),
            body: vec![inner],
        }],
    };
    assert!(!jumps(&[]));
    assert!(!jumps(&[
        hands(vec![address(local(1))]),
        nested(Stmt::Break),
        Stmt::Return(None),
    ]));
    for inner in [Stmt::Goto(label), Stmt::Label(label)] {
        let around = [hands(Vec::new()), nested(inner.clone())];
        assert!(jumps(&around), "{inner:?}");
        assert!(jumps(&[inner]));
    }
    // A construct's own statements are looked in too.
    let hidden = Stmt::Sugar(Box::new(Sugar::Hidden(vec![Stmt::Goto(label)])));
    assert!(jumps(&[hidden]));
}
