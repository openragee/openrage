use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::ir::{Const, Returns, Scalar};
use crate::gta6::script::project::Param;
use crate::gta6::script::{Level, Names, Options};

const PROGRAM: u32 = 0x1122_3344;
/// The function every test groups.
const AT: u32 = 0x40;

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn cell(of: Expr, index: u32) -> Expr {
    Expr::Cell {
        base: Box::new(of),
        index,
    }
}

fn set(target: Var, value: Expr) -> Stmt {
    Stmt::Assign { target, value }
}

/// `&local_n`.
fn address(n: u32) -> Expr {
    Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    }
}

/// A call of the program's function at `at`.
fn call(at: u32, args: Vec<Expr>, returns: Option<u32>) -> Expr {
    let callee = Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    };
    Expr::Call {
        callee,
        args,
        returns,
    }
}

fn used(at: u32, args: Vec<Expr>) -> Stmt {
    Stmt::Eval(call(at, args, Some(0)))
}

/// A function of one block, `params` parameter cells and eight locals.
fn function(params: u32, stmts: Vec<Stmt>, term: Terminator) -> Function {
    let head = Head {
        program: PROGRAM,
        at: AT,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params,
        locals: 8,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    let block = Block {
        at: None,
        stmts,
        term,
    };
    Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

/// A signature of parameters of these cells and kinds.
fn takes(params: &[(u32, Kind)]) -> Signature {
    let params = params.iter().map(|(cells, kind)| Param {
        kind: kind.clone(),
        cells: *cells,
    });
    Signature {
        params: params.collect(),
        ..Signature::default()
    }
}

/// A project that has `signature` for the function at `at`.
fn knowing(at: u32, signature: Signature) -> Project {
    let mut project = Project::default();
    project
        .signatures
        .insert(FunctionKey::new(PROGRAM, at), signature);
    project
}

fn on() -> Options {
    Options::at(Level::Structured)
}

fn grouped(mut f: Function, project: &Project, options: Options) -> Function {
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    run(&mut f, &cx, &mut Stats::default());
    f
}

fn shape(f: &Function, var: Var) -> Option<(Width, Kind)> {
    let info = f.head.vars.get(&var);
    info.map(|info| (info.width, info.kind.clone()))
}

/// A temporary of several cells, declared as a listing declares one.
fn wide(f: &mut Function) -> Var {
    let v = Var::Value { at: 0x10, index: 0 };
    f.head.vars.insert(v, VarInfo::new(Width::Cells));
    v
}

#[test]
fn a_parameter_the_signature_gives_two_cells_is_one_variable() {
    let project = knowing(AT, takes(&[(1, Kind::Unknown), (2, Kind::Vector)]));
    let body = || {
        vec![
            used(0x100, vec![param(0), param(1), param(2)]),
            set(Var::Local(0), Expr::cast(Scalar::F32, param(2))),
        ]
    };
    let both = || Terminator::Return(Some(Expr::Cells(vec![param(1), param(2)])));
    let done = grouped(function(3, body(), both()), &project, on());

    let block = &done.cfg.blocks[0];
    // Passed whole, a cell read alone, and returned whole.
    assert_eq!(block.stmts[0], used(0x100, vec![param(0), param(1)]));
    let alone = Expr::cast(Scalar::F32, cell(param(1), 1));
    assert_eq!(block.stmts[1], set(Var::Local(0), alone));
    assert_eq!(block.term, Terminator::Return(Some(param(1))));
    assert_eq!(
        shape(&done, Var::Param(1)),
        Some((Width::Cells, Kind::Vector))
    );
    assert_eq!(shape(&done, Var::Param(2)), None);
    let counted = Made {
        params: 1,
        locals: 0,
    };
    assert_eq!(apply(&mut function(3, body(), both()), &project), counted);

    // A signature whose cells are not the function's says nothing of it.
    let other = function(4, body(), both());
    assert_eq!(grouped(other.clone(), &project, on()), other);
    // With the pass switched off nothing is grouped.
    let off = on().without(Pass::Aggregates);
    let same = function(3, body(), both());
    assert_eq!(grouped(same.clone(), &project, off), same);
}

#[test]
fn a_run_with_a_cell_assigned_alone_or_a_load_across_its_end_stays_cells() {
    let project = knowing(
        AT,
        takes(&[(1, Kind::Unknown), (2, Kind::Vector), (1, Kind::Unknown)]),
    );
    let passed = || used(0x100, vec![param(1), param(2)]);

    let assigned = vec![passed(), set(Var::Param(2), int(0))];
    let f = function(4, assigned, Terminator::Return(None));
    assert_eq!(grouped(f.clone(), &project, on()), f);

    // Sixteen bytes from the run's second cell reach the parameter after it.
    let across = Expr::LoadBytes {
        addr: Box::new(Expr::AddrOf {
            var: Var::Param(2),
            cell: 0,
        }),
        offset: 0,
        bytes: 16,
    };
    let crossed = vec![passed(), set(Var::Local(0), across)];
    let f = function(4, crossed, Terminator::Return(None));
    assert_eq!(grouped(f.clone(), &project, on()), f);

    // Neither there, the run is made.
    let f = function(4, vec![passed()], Terminator::Return(None));
    let done = grouped(f, &project, on());
    assert_eq!(done.cfg.blocks[0].stmts, [used(0x100, vec![param(1)])]);
}

#[test]
fn a_result_spread_over_locals_is_one_local_and_a_cell_of_it_read_alone() {
    let body = |v: Var| {
        vec![
            set(v, call(0x100, vec![], Some(2))),
            set(Var::Local(2), cell(Expr::Var(v), 0)),
            set(Var::Local(3), cell(Expr::Var(v), 1)),
            used(0x200, vec![local(2), local(3)]),
            set(Var::Local(5), local(3)),
        ]
    };
    let mut f = function(1, Vec::new(), Terminator::Return(None));
    let v = wide(&mut f);
    f.cfg.blocks[0].stmts = body(v);
    let done = grouped(f.clone(), Project::empty(), on());
    assert_eq!(
        done.cfg.blocks[0].stmts,
        [
            set(v, call(0x100, vec![], Some(2))),
            set(Var::Local(2), Expr::Var(v)),
            used(0x200, vec![local(2)]),
            set(Var::Local(5), cell(local(2), 1)),
        ]
    );
    assert_eq!(
        shape(&done, Var::Local(2)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );

    // One of the two locals assigned alone: the two stay two.
    f.cfg.blocks[0].stmts.push(set(Var::Local(3), int(1)));
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
}

#[test]
fn the_cells_of_a_result_passed_in_order_are_the_result() {
    let body = |v: Var, returns: Option<u32>, order: [u32; 2]| {
        let cells = order.map(|index| cell(Expr::Var(v), index));
        vec![
            set(v, call(0x100, vec![], returns)),
            used(0x200, cells.to_vec()),
        ]
    };
    let mut f = function(1, Vec::new(), Terminator::Return(None));
    let v = wide(&mut f);

    f.cfg.blocks[0].stmts = body(v, Some(2), [0, 1]);
    let done = grouped(f.clone(), Project::empty(), on());
    assert_eq!(done.cfg.blocks[0].stmts[1], used(0x200, vec![Expr::Var(v)]));

    // Out of order they are two cells, and so they are where nothing states how many the call yields.
    f.cfg.blocks[0].stmts = body(v, Some(2), [1, 0]);
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
    f.cfg.blocks[0].stmts = body(v, None, [0, 1]);
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
}

#[test]
fn locals_passed_where_a_callee_takes_one_value_are_one_local() {
    let project = knowing(0x200, takes(&[(1, Kind::Unknown), (2, Kind::Handle)]));
    let body = |passed: Vec<Expr>| vec![used(0x100, vec![address(4)]), used(0x200, passed)];
    let whole = || vec![param(0), local(4), local(5)];
    let f = function(1, body(whole()), Terminator::Return(None));

    let done = grouped(f.clone(), &project, on());
    let made = body(vec![param(0), local(4)]);
    assert_eq!(done.cfg.blocks[0].stmts, made);
    assert_eq!(
        shape(&done, Var::Local(4)),
        Some((Width::Cells, Kind::Handle))
    );

    // No signature of the callee, the locals the other way round, or a call of other cells than the callee takes.
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
    let turned = body(vec![param(0), local(5), local(4)]);
    let f = function(1, turned, Terminator::Return(None));
    assert_eq!(grouped(f.clone(), &project, on()), f);
    let more = body(vec![param(0), local(4), local(5), local(6)]);
    let f = function(1, more, Terminator::Return(None));
    assert_eq!(grouped(f.clone(), &project, on()), f);
}

#[test]
fn a_run_inside_another_is_a_part_of_it_and_two_that_cross_are_neither() {
    let mut project = knowing(0x200, takes(&[(3, Kind::Unknown)]));
    let pair = FunctionKey::new(PROGRAM, 0x300);
    project
        .signatures
        .insert(pair, takes(&[(2, Kind::Unknown)]));
    let three = || used(0x200, vec![local(4), local(5), local(6)]);

    let inside = vec![three(), used(0x300, vec![local(5), local(6)])];
    let done = grouped(
        function(1, inside, Terminator::Return(None)),
        &project,
        on(),
    );
    let parts = vec![cell(local(4), 1), cell(local(4), 2)];
    assert_eq!(
        done.cfg.blocks[0].stmts,
        [used(0x200, vec![local(4)]), used(0x300, parts)]
    );

    let crossing = vec![three(), used(0x300, vec![local(6), local(7)])];
    let f = function(1, crossing, Terminator::Return(None));
    assert_eq!(grouped(f.clone(), &project, on()), f);
}

#[test]
fn every_byte_of_a_run_cleared_through_its_address_is_its_assignment() {
    let clear = || Stmt::ZeroBytes {
        addr: address(4),
        bytes: 16,
    };
    let body = vec![clear(), used(0x100, vec![local(4), local(5)])];
    let done = grouped(
        function(1, body, Terminator::Return(None)),
        Project::empty(),
        on(),
    );
    assert_eq!(
        done.cfg.blocks[0].stmts,
        [
            set(Var::Local(4), Expr::ZeroSlots(2)),
            used(0x100, vec![local(4)])
        ]
    );

    // A variable of several cells that is there already takes no part in a new one.
    let body = vec![clear(), used(0x100, vec![local(4), local(5)])];
    let mut f = function(1, body, Terminator::Return(None));
    let held = VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(2),
        name: None,
    };
    f.head.vars.insert(Var::Local(5), held);
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
}
