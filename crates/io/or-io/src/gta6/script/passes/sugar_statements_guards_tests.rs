use std::collections::BTreeMap as Map;

use super::super::rewrite;
use super::*;
use crate::gta6::script::exec;
use crate::gta6::script::ir::{Const, Head, Returns, Scalar, StoreKind};
use crate::gta6::script::{Names, Options, Project};

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn assign(n: u32, value: Expr) -> ir::Stmt {
    ir::Stmt::Assign {
        target: Var::Local(n),
        value,
    }
}

fn set_to(n: u32, value: Expr) -> Stmt {
    Stmt::Simple(assign(n, value))
}

fn eval(e: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(e))
}

/// A call of a function of the program itself.
fn call(args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at: 0x200,
            name: "func_000200".to_owned(),
        },
        args,
        returns: Some(1),
    }
}

/// A call of the method of the runtime a list gives `path`.
fn runtime(path: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 0x0F03,
            method: 0,
        },
        args,
        returns: Some(0),
    }
}

fn head() -> Head {
    Head {
        program: 1,
        at: 0x10,
        name: "func_000010".to_owned(),
        returns: Returns::Cell,
        params: 1,
        locals: 8,
        vars: Map::new(),
        incomplete: None,
    }
}

fn function(body: Vec<Stmt>) -> tree::Function {
    tree::Function { head: head(), body }
}

/// What the pass makes of `body` under `options`, and what it found.
fn read_with(body: Vec<Stmt>, options: Options) -> (Vec<Stmt>, Found) {
    let mut f = function(body);
    let names = Names::default();
    let found = rewrite(&mut f, &Context::new(&names, Project::empty(), options));
    (f.body, found)
}

/// [`read_with`] every name on.
fn read(body: Vec<Stmt>) -> (Vec<Stmt>, Found) {
    read_with(body, Options::default())
}

/// Whether two bodies do the same on made-up inputs, as far as each run goes.
fn run_alike(was: Vec<Stmt>, is: Vec<Stmt>) -> Result<(), String> {
    let was = cfg::lower(function(was)).map_err(|e| e.to_string())?;
    let is = cfg::lower(function(is)).map_err(|e| e.to_string())?;
    for seed in 0..8 {
        exec::same(&exec::run(&was, seed), &exec::run(&is, seed))?;
    }
    Ok(())
}

/// The four bytes at offset 8 of the object `p0` refers to.
fn field() -> Expr {
    Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(Expr::Var(Var::Param(0))),
        offset: 8,
    }
}

/// `store_i32(p0, 8, 0);`.
fn field_start() -> ir::Stmt {
    ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: Expr::Var(Var::Param(0)),
        offset: 8,
        value: int(0),
    }
}

/// `increment_i32(p0, 8);`.
fn field_step() -> ir::Stmt {
    ir::Stmt::StepI32 {
        addr: Expr::Var(Var::Param(0)),
        offset: 8,
        down: false,
    }
}

/// `while (load_i32(p0, 8) < 4) { <body> }`.
fn while_field(body: Vec<Stmt>) -> Stmt {
    Stmt::While {
        cond: Expr::binary(BinOp::Lt, field(), int(4)),
        body,
    }
}

#[test]
fn a_while_whose_counter_is_set_just_before_and_stepped_last_is_a_for() {
    let work = eval(call(Vec::new()));
    let was = vec![
        Stmt::Simple(field_start()),
        while_field(vec![work.clone(), Stmt::Simple(field_step())]),
    ];
    let (is, found) = read(was.clone());
    assert_eq!(found.counted, 1);
    let made = Stmt::For {
        init: Some(Box::new(field_start())),
        cond: Some(Expr::binary(BinOp::Lt, field(), int(4))),
        step: Some(Box::new(field_step())),
        body: vec![work],
    };
    assert_eq!(is, vec![made]);
    assert_eq!(run_alike(was.clone(), is), Ok(()));
    // With the name off the loop stays a `while`.
    let off = Options::default().without(Pass::For);
    assert_eq!(read_with(was.clone(), off), (was, Found::default()));
}

#[test]
fn a_variable_set_before_a_while_and_stepped_last_in_it_is_the_counter_of_a_for() {
    let up = Expr::binary(BinOp::Add, Expr::cast(Scalar::I32, local(1)), int(1));
    let test = Expr::binary(BinOp::Lt, Expr::cast(Scalar::I32, local(1)), int(9));
    // A `continue` of a loop inside goes round nothing of this one.
    let inner = Stmt::While {
        cond: local(2),
        body: vec![Stmt::Continue],
    };
    let was = vec![
        set_to(1, int(0)),
        Stmt::While {
            cond: test,
            body: vec![inner, eval(call(vec![local(1)])), set_to(1, up)],
        },
    ];
    let (is, found) = read(was.clone());
    assert_eq!(found.counted, 1);
    assert!(
        matches!(is.as_slice(), [Stmt::For { init: Some(_), step: Some(_), body, .. }] if body.len() == 2),
        "{is:?}"
    );
    assert_eq!(run_alike(was, is), Ok(()));
}

#[test]
fn a_while_of_another_shape_is_no_for() {
    let step = || Stmt::Simple(field_step());
    let start = || Stmt::Simple(field_start());
    let work = || eval(call(Vec::new()));
    let skip = Stmt::If {
        cond: local(2),
        then: vec![Stmt::Continue],
        els: Vec::new(),
    };
    let elsewhere = ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: Expr::Var(Var::Param(0)),
        offset: 12,
        value: int(0),
    };
    let untested = Stmt::While {
        cond: local(2),
        body: vec![work(), step()],
    };
    let by_two = Stmt::While {
        cond: Expr::binary(BinOp::Lt, local(1), int(9)),
        body: vec![set_to(1, Expr::binary(BinOp::Add, local(1), int(2)))],
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "the step is of more than one",
            vec![set_to(1, int(0)), by_two],
        ),
        (
            "a `continue` would go round the step",
            vec![start(), while_field(vec![skip, step()])],
        ),
        (
            "the statement before sets another place",
            vec![Stmt::Simple(elsewhere), while_field(vec![step()])],
        ),
        (
            "the body does not end in a step",
            vec![start(), while_field(vec![step(), work()])],
        ),
        ("the test does not read the place", vec![start(), untested]),
        (
            "nothing stands before the loop",
            vec![while_field(vec![step()])],
        ),
        (
            "something stands between",
            vec![start(), work(), while_field(vec![step()])],
        ),
    ];
    for (why, was) in cases {
        assert_eq!(read(was.clone()), (was, Found::default()), "{why}");
    }
}

/// `local_n = func();`: the taking of what is disposed.
fn take(n: u32) -> Stmt {
    set_to(n, call(Vec::new()))
}

/// `local_n.Dispose();`, through the runtime.
fn dispose(n: u32) -> Stmt {
    eval(runtime("Sample.Resource.Dispose", vec![local(n)]))
}

/// `if (local_n != null) { local_n.Dispose(); }`.
fn dispose_unless_null(n: u32) -> Stmt {
    Stmt::If {
        cond: Expr::binary(BinOp::Ne, Expr::cast(Scalar::U64, local(n)), int(0)),
        then: vec![dispose(n)],
        els: Vec::new(),
    }
}

/// `if (local_7) { <before> return 1; }`.
fn leave(before: Vec<Stmt>) -> Stmt {
    let mut then = before;
    then.push(Stmt::Return(Some(int(1))));
    Stmt::If {
        cond: local(7),
        then,
        els: Vec::new(),
    }
}

fn hidden(stmts: Vec<Stmt>) -> Stmt {
    Stmt::Sugar(Box::new(Sugar::Hidden(stmts)))
}

#[test]
fn a_variable_taken_and_disposed_at_every_way_out_is_a_using() {
    let work = eval(call(vec![local(1)]));
    let was = vec![
        take(1),
        work.clone(),
        leave(vec![dispose_unless_null(1)]),
        dispose_unless_null(1),
        Stmt::Return(Some(int(0))),
    ];
    let (is, found) = read(was.clone());
    let one = Found {
        usings: 1,
        ..Found::default()
    };
    assert_eq!(found, one);
    let [Stmt::Sugar(made), Stmt::Return(_)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Guarded {
        kind,
        enter,
        body,
        exit,
    } = &**made
    else {
        panic!("{made:?}")
    };
    assert_eq!(*kind, Guard::Using);
    assert_eq!(enter, &vec![assign(1, call(Vec::new()))]);
    // The disposal before the `return` is still there, and written as nothing.
    let left = leave(vec![hidden(vec![dispose_unless_null(1)])]);
    assert_eq!(body, &vec![work, left]);
    assert_eq!(exit, &vec![dispose_unless_null(1)]);
    // The way back: the blocks of the statements it was read from.
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was.clone()));
    // With the name off they stay as they were.
    let off = Options::default().without(Pass::Using);
    assert_eq!(read_with(was.clone(), off), (was, Found::default()));
}

#[test]
fn a_using_inside_another_is_read_past_the_disposal_hidden_for_the_one_around_it() {
    let both = vec![dispose_unless_null(2), dispose_unless_null(1)];
    let was = vec![
        take(1),
        take(2),
        eval(call(vec![local(1), local(2)])),
        leave(both),
        dispose_unless_null(2),
        dispose_unless_null(1),
        Stmt::Return(Some(int(0))),
    ];
    let (is, found) = read(was.clone());
    assert_eq!(found.usings, 2);
    let [Stmt::Sugar(outer), Stmt::Return(_)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Guarded { body, .. } = &**outer else {
        panic!("{outer:?}")
    };
    assert!(
        matches!(body.as_slice(), [Stmt::Sugar(inner)] if matches!(**inner, Sugar::Guarded { .. })),
        "{body:?}"
    );
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}

#[test]
fn statements_that_do_not_dispose_at_every_way_out_are_no_using() {
    let guard = || dispose_unless_null(1);
    let touch = |n: u32| eval(call(vec![local(n)]));
    let address = eval(call(vec![Expr::AddrOf {
        var: Var::Local(1),
        cell: 0,
    }]));
    let out = Stmt::If {
        cond: local(7),
        then: vec![Stmt::Break],
        els: Vec::new(),
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a return skips the disposal",
            vec![take(1), leave(Vec::new()), guard()],
        ),
        (
            "the disposal has no test for null",
            vec![take(1), touch(1), dispose(1)],
        ),
        (
            "a return disposes with no test for null",
            vec![take(1), leave(vec![dispose(1)]), guard()],
        ),
        (
            "the variable is read after the disposal",
            vec![take(1), guard(), touch(1)],
        ),
        (
            "the variable is taken again before the disposal",
            vec![take(1), touch(1), take(1), touch(1), guard()],
        ),
        (
            "the address of the variable is taken",
            vec![take(1), address, guard()],
        ),
        (
            "a jump leaves the statements",
            vec![take(1), Stmt::Goto(tree::Label::Block(9)), guard()],
        ),
        (
            "a `break` leaves the statements for the loop around them",
            vec![Stmt::While {
                cond: local(6),
                body: vec![take(1), out, guard()],
            }],
        ),
        ("nothing takes the variable", vec![touch(1), guard()]),
    ];
    for (why, was) in cases {
        let (is, found) = read(was.clone());
        assert_eq!((is, found.usings), (was, 0), "{why}");
    }
}

/// `Monitor.Enter(local_n);` or `Monitor.Exit(local_n);`, through the runtime.
fn monitor_call(member: &str, n: u32) -> Stmt {
    let path = format!("System.Threading.Monitor.{member}");
    eval(runtime(&path, vec![local(n)]))
}

#[test]
fn a_monitor_entered_and_left_at_every_way_out_is_a_lock() {
    let work = eval(call(vec![local(3)]));
    let was = vec![
        monitor_call("Enter", 3),
        work.clone(),
        leave(vec![monitor_call("Exit", 3)]),
        monitor_call("Exit", 3),
        Stmt::Return(Some(int(0))),
    ];
    let (is, found) = read(was.clone());
    let one = Found {
        locks: 1,
        ..Found::default()
    };
    assert_eq!(found, one);
    let [Stmt::Sugar(made), Stmt::Return(_)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Guarded {
        kind,
        enter,
        body,
        exit,
    } = &**made
    else {
        panic!("{made:?}")
    };
    assert_eq!(*kind, Guard::Lock);
    let entered = ir::Stmt::Eval(runtime("System.Threading.Monitor.Enter", vec![local(3)]));
    assert_eq!(enter, &vec![entered]);
    let left = leave(vec![hidden(vec![monitor_call("Exit", 3)])]);
    assert_eq!(body, &vec![work, left]);
    assert_eq!(exit, &vec![monitor_call("Exit", 3)]);
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was.clone()));
    let off = Options::default().without(Pass::Lock);
    assert_eq!(read_with(was.clone(), off), (was, Found::default()));
}

#[test]
fn a_monitor_inside_another_of_the_same_object_closes_its_own_entry() {
    let was = vec![
        monitor_call("Enter", 3),
        monitor_call("Enter", 3),
        eval(call(Vec::new())),
        monitor_call("Exit", 3),
        monitor_call("Exit", 3),
    ];
    let (is, found) = read(was.clone());
    assert_eq!(found.locks, 2);
    let [Stmt::Sugar(outer)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Guarded { body, .. } = &**outer else {
        panic!("{outer:?}")
    };
    assert!(
        matches!(body.as_slice(), [Stmt::Sugar(inner)] if matches!(**inner, Sugar::Guarded { .. })),
        "{body:?}"
    );
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}

#[test]
fn statements_that_do_not_leave_a_monitor_at_every_way_out_are_no_lock() {
    let two = eval(runtime(
        "System.Threading.Monitor.Enter",
        vec![local(3), local(4)],
    ));
    let other = eval(runtime("Sample.Resource.Enter", vec![local(3)]));
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a return does not leave the monitor",
            vec![
                monitor_call("Enter", 3),
                leave(Vec::new()),
                monitor_call("Exit", 3),
            ],
        ),
        (
            "the variable is set between",
            vec![
                monitor_call("Enter", 3),
                set_to(3, int(0)),
                monitor_call("Exit", 3),
            ],
        ),
        (
            "another object is left",
            vec![monitor_call("Enter", 3), monitor_call("Exit", 4)],
        ),
        (
            "the entry takes a second argument",
            vec![two, monitor_call("Exit", 3)],
        ),
        (
            "the entry is of another type",
            vec![other, monitor_call("Exit", 3)],
        ),
        ("nothing enters", vec![monitor_call("Exit", 3)]),
    ];
    for (why, was) in cases {
        assert_eq!(read(was.clone()), (was, Found::default()), "{why}");
    }
}

// Made up: the hash of a script type a handler takes.
const FAULT: u64 = 0x1234_5678_9ABC_DEF0;

/// `func(n);`.
fn marked(n: i32) -> ir::Stmt {
    ir::Stmt::Eval(call(vec![int(n)]))
}

fn block(n: i32, term: Terminator) -> cfg::Block {
    cfg::Block {
        at: None,
        stmts: vec![marked(n)],
        term,
    }
}

/// Four blocks: the first runs into the second, the second and the third jump to the fourth, and the entry does not reach the third.
fn guarded_function() -> cfg::Function {
    let blocks = vec![
        block(0, Terminator::Jump(1)),
        block(1, Terminator::Jump(3)),
        block(2, Terminator::Jump(3)),
        block(3, Terminator::Return(None)),
    ];
    cfg::Function {
        head: head(),
        cfg: cfg::Cfg { blocks },
    }
}

fn catching(ty: Option<u64>, blocks: Vec<usize>, handler: usize) -> cfg::Region {
    cfg::Region {
        kind: cfg::RegionKind::Catch(ty.map(TypeKey)),
        blocks,
        handler,
    }
}

fn all_on(names: &Names) -> Context<'_> {
    Context::new(names, Project::empty(), Options::default())
}

#[test]
fn a_protected_run_and_its_handler_are_a_try_where_the_run_stood() {
    let names = Names::default();
    let regions = [catching(Some(FAULT), vec![1], 2)];
    let is = protected(guarded_function(), &regions, &all_on(&names));
    let after = tree::Label::Block(3);
    let tried = Sugar::Try {
        body: vec![Stmt::Simple(marked(1)), Stmt::Goto(after)],
        catches: vec![Catch {
            ty: Some(TypeKey(FAULT)),
            var: None,
            // What runs off the end of a handler is told where it goes.
            body: vec![Stmt::Simple(marked(2)), Stmt::Goto(after)],
        }],
        finally: Vec::new(),
    };
    let expected = vec![
        Stmt::Simple(marked(0)),
        Stmt::Sugar(Box::new(tried)),
        Stmt::Label(after),
        Stmt::Simple(marked(3)),
        Stmt::Return(None),
    ];
    assert_eq!(is.body, expected);
    // The way back holds what runs with no fault: every block but the handler's.
    let lowered = cfg::lower_body(is.body).map(|cfg| cfg.statements());
    assert_eq!(lowered, Ok(3));
}

#[test]
fn two_handlers_of_one_run_are_the_catches_of_one_try_in_the_order_stated() {
    let blocks = vec![
        block(0, Terminator::Jump(1)),
        block(1, Terminator::Jump(4)),
        block(2, Terminator::Jump(4)),
        block(3, Terminator::Return(None)),
        block(4, Terminator::Return(None)),
    ];
    let f = cfg::Function {
        head: head(),
        cfg: cfg::Cfg { blocks },
    };
    let names = Names::default();
    let regions = [
        catching(Some(FAULT), vec![1], 2),
        catching(None, vec![1], 3),
    ];
    let is = protected(f, &regions, &all_on(&names));
    let [_, Stmt::Sugar(made), ..] = is.body.as_slice() else {
        panic!("{:?}", is.body)
    };
    let Sugar::Try { catches, .. } = &**made else {
        panic!("{made:?}")
    };
    let taken: Vec<(Option<TypeKey>, usize)> = (catches.iter())
        .map(|catch| (catch.ty, catch.body.len()))
        .collect();
    assert_eq!(taken, vec![(Some(TypeKey(FAULT)), 2), (None, 2)]);
}

#[test]
fn regions_that_are_no_such_runs_leave_the_function_as_its_blocks() {
    let names = Names::default();
    let flat = cfg::flat(guarded_function());
    let finally = cfg::Region {
        kind: cfg::RegionKind::Finally,
        blocks: vec![1],
        handler: 2,
    };
    let cases: Vec<(&str, Vec<cfg::Region>)> = vec![
        ("no region is stated", Vec::new()),
        ("a region runs however it is left", vec![finally]),
        (
            "the entry reaches the handler",
            vec![catching(None, vec![1], 3)],
        ),
        ("the blocks are no run", vec![catching(None, vec![0, 3], 2)]),
        (
            "the handler is in the run",
            vec![catching(None, vec![1, 2], 2)],
        ),
        (
            "a block is past the function's",
            vec![catching(None, vec![4], 2)],
        ),
    ];
    for (why, regions) in cases {
        let is = protected(guarded_function(), &regions, &all_on(&names));
        assert_eq!(is, flat, "{why}");
    }
    // With the name off a function with a `try` to read is written as its blocks too.
    let regions = [catching(Some(FAULT), vec![1], 2)];
    let off = Options::default().without(Pass::Try);
    let cx = Context::new(&names, Project::empty(), off);
    assert_eq!(protected(guarded_function(), &regions, &cx), flat);
}

#[test]
fn a_jump_into_a_handler_from_outside_it_leaves_the_function_as_its_blocks() {
    // The third block is reached by no jump, and jumps into the handler after it.
    let blocks = vec![
        block(0, Terminator::Jump(1)),
        block(1, Terminator::Jump(4)),
        block(2, Terminator::Jump(3)),
        block(3, Terminator::Jump(4)),
        block(4, Terminator::Return(None)),
    ];
    let f = cfg::Function {
        head: head(),
        cfg: cfg::Cfg { blocks },
    };
    let names = Names::default();
    let regions = [catching(None, vec![1], 3), catching(None, vec![0], 2)];
    let flat = cfg::flat(f.clone());
    assert_eq!(protected(f, &regions, &all_on(&names)), flat);
}
