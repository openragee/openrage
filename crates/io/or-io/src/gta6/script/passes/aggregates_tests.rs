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
        value_types: Default::default(),
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
        bytes: 0,
        zeros: 0,
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
        single: false,
    };
    f.head.vars.insert(Var::Local(5), held);
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
}

/// `extract_bytes(value, offset, bytes)`.
fn extract(value: Expr, offset: u32, bytes: u32) -> Expr {
    Expr::ExtractBytes {
        value: Box::new(value),
        offset,
        bytes,
    }
}

/// `zero_bytes(&local_n, bytes);`.
fn clear(n: u32, bytes: u32) -> Stmt {
    Stmt::ZeroBytes {
        addr: address(n),
        bytes,
    }
}

fn leaves() -> Terminator {
    Terminator::Return(None)
}

#[test]
fn a_row_copied_cell_by_cell_is_one_copy_while_the_row_copied_from_is_one_value() {
    let body = |after: Option<Stmt>| {
        let mut stmts = vec![
            clear(4, 16),
            set(Var::Local(0), local(4)),
            set(Var::Local(1), local(5)),
            used(0x100, vec![address(0)]),
        ];
        stmts.extend(after);
        stmts
    };
    let made = grouped(function(1, body(None), leaves()), Project::empty(), on());
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [
            set(Var::Local(4), Expr::ZeroSlots(2)),
            set(Var::Local(0), local(4)),
            used(0x100, vec![address(0)]),
        ]
    );
    assert_eq!(
        shape(&made, Var::Local(0)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );
    assert_eq!(shape(&made, Var::Local(1)), None);

    // A cell of the row copied from assigned alone: that row is no value, and so neither is its copy.
    let f = function(1, body(Some(set(Var::Local(5), int(1)))), leaves());
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);

    // Two rows that share a cell are cells moved along, which is no copy of a value.
    let moved = vec![
        set(Var::Local(1), local(2)),
        set(Var::Local(2), local(3)),
        used(0x100, vec![address(1)]),
    ];
    let f = function(1, moved, leaves());
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
}

#[test]
fn a_run_assigned_cell_by_cell_in_order_from_plain_values_is_assigned_whole() {
    let project = knowing(0x200, takes(&[(2, Kind::Unknown)]));
    let body = |second: Expr| {
        vec![
            set(Var::Local(4), param(0)),
            set(Var::Local(5), second),
            used(0x200, vec![local(4), local(5)]),
        ]
    };
    let made = grouped(function(1, body(int(7)), leaves()), &project, on());
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [
            set(Var::Local(4), Expr::Cells(vec![param(0), int(7)])),
            used(0x200, vec![local(4)]),
        ]
    );

    // A call may read the run while one cell of it is assigned, and a cell of the run read for the next is no plain value.
    let f = function(1, body(call(0x300, vec![], Some(1))), leaves());
    assert_eq!(grouped(f.clone(), &project, on()), f);
    let f = function(1, body(local(4)), leaves());
    assert_eq!(grouped(f.clone(), &project, on()), f);
    // Out of order, the cells are assigned alone.
    let turned = vec![
        set(Var::Local(5), int(7)),
        set(Var::Local(4), param(0)),
        used(0x200, vec![local(4), local(5)]),
    ];
    let f = function(1, turned, leaves());
    assert_eq!(grouped(f.clone(), &project, on()), f);
}

#[test]
fn bytes_taken_out_of_a_group_of_locals_are_a_field_of_one_local() {
    let body = |addressed: Option<u32>| {
        let group = Expr::Cells(vec![local(4), local(5)]);
        let taken = Expr::cast(Scalar::I32, extract(group, 12, 4));
        let handed = addressed.map(|n| used(0x100, vec![address(n)]));
        let mut stmts: Vec<Stmt> = handed.into_iter().collect();
        stmts.push(set(Var::Local(0), taken));
        stmts
    };

    // The local's address is handed on already: the bytes are loaded through it, as the cast reads them.
    let made = grouped(function(1, body(Some(4)), leaves()), Project::empty(), on());
    let field = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(address(4)),
        offset: 12,
    };
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [used(0x100, vec![address(4)]), set(Var::Local(0), field)]
    );
    assert_eq!(
        shape(&made, Var::Local(4)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );
    assert_eq!(shape(&made, Var::Local(5)), None);

    // No address of it anywhere: it is one local, and none is written for the bytes.
    let made = grouped(function(1, body(None), leaves()), Project::empty(), on());
    let bytes = Expr::cast(Scalar::I32, extract(local(4), 12, 4));
    assert_eq!(made.cfg.blocks[0].stmts, [set(Var::Local(0), bytes)]);
    assert_eq!(
        shape(&made, Var::Local(4)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );

    // Its second cell has an address of its own: the two are two variables.
    let f = function(1, body(Some(5)), leaves());
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
    // With the pass switched off nothing is grouped.
    let f = function(1, body(Some(4)), leaves());
    let off = on().without(Pass::Aggregates);
    assert_eq!(grouped(f.clone(), Project::empty(), off), f);
}

#[test]
fn an_address_a_whole_write_takes_away_exposes_nothing() {
    // The one address of the local is under the bytes cleared, which become an assignment.
    let group = Expr::Cells(vec![local(4), local(5)]);
    let body = vec![clear(4, 16), set(Var::Local(0), extract(group, 8, 4))];
    let made = grouped(function(1, body, leaves()), Project::empty(), on());
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [
            set(Var::Local(4), Expr::ZeroSlots(2)),
            set(Var::Local(0), extract(local(4), 8, 4)),
        ]
    );
}

#[test]
fn bytes_of_some_cells_of_a_value_are_bytes_of_the_value() {
    let mut f = function(1, Vec::new(), leaves());
    let v = wide(&mut f);
    let cells = |range: std::ops::Range<u32>| {
        Expr::Cells(range.map(|index| cell(Expr::Var(v), index)).collect())
    };
    let body = |taken: Expr| {
        vec![
            set(v, call(0x100, vec![], Some(4))),
            used(0x200, vec![taken]),
        ]
    };

    f.cfg.blocks[0].stmts = body(extract(cells(2..4), 4, 4));
    let made = grouped(f.clone(), Project::empty(), on());
    let further = extract(Expr::Var(v), 20, 4);
    assert_eq!(made.cfg.blocks[0].stmts[1], used(0x200, vec![further]));

    // Every byte of the value is the value.
    f.cfg.blocks[0].stmts = body(extract(cells(0..4), 0, 32));
    let made = grouped(f.clone(), Project::empty(), on());
    assert_eq!(made.cfg.blocks[0].stmts[1], used(0x200, vec![Expr::Var(v)]));

    // Bytes past the cells gathered are none of the value's.
    f.cfg.blocks[0].stmts = body(extract(cells(2..4), 12, 8));
    assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
}

#[test]
fn bytes_of_a_parameter_of_several_cells_are_loaded_only_below_an_address_already_taken() {
    let project = knowing(AT, takes(&[(1, Kind::Unknown), (3, Kind::Unknown)]));
    let body = |before: Option<Stmt>| {
        let group = Expr::Cells(vec![param(1), param(2), param(3)]);
        let mut stmts: Vec<Stmt> = before.into_iter().collect();
        stmts.push(used(0x100, vec![extract(group, 17, 1)]));
        stmts
    };
    let of_param = |n: u32| Expr::AddrOf {
        var: Var::Param(n),
        cell: 0,
    };
    let handed = || used(0x300, vec![of_param(0)]);

    let made = grouped(function(4, body(Some(handed())), leaves()), &project, on());
    let field = Expr::Load {
        ty: Scalar::U8,
        addr: Box::new(of_param(1)),
        offset: 17,
    };
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [handed(), used(0x100, vec![field])]
    );

    let made = grouped(function(4, body(None), leaves()), &project, on());
    let bytes = extract(param(1), 17, 1);
    assert_eq!(made.cfg.blocks[0].stmts, [used(0x100, vec![bytes])]);
}

#[test]
fn a_local_cleared_past_its_first_cell_is_as_many_cells_as_the_bytes_reach() {
    let body = vec![clear(4, 12), set(Var::Local(0), local(5))];
    let made = grouped(function(1, body, leaves()), Project::empty(), on());
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [clear(4, 12), set(Var::Local(0), cell(local(4), 1))]
    );
    assert_eq!(
        shape(&made, Var::Local(4)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );
}

#[test]
fn what_the_code_suggests_gives_way_to_what_something_states() {
    // Sixteen bytes cleared state two cells; a group of three from the same local only suggests three.
    let three = || Expr::Cells(vec![local(4), local(5), local(6)]);
    let body = vec![clear(4, 16), used(0x100, vec![three()])];
    let made = grouped(function(1, body, leaves()), Project::empty(), on());
    let parts = Expr::Cells(vec![local(4), local(6)]);
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [
            set(Var::Local(4), Expr::ZeroSlots(2)),
            used(0x100, vec![parts])
        ]
    );

    // Of two groups from one local the longer is the value, and the shorter is cells of it.
    let two = Expr::Cells(vec![local(4), local(5)]);
    let body = vec![used(0x100, vec![two]), used(0x100, vec![three()])];
    let made = grouped(function(1, body, leaves()), Project::empty(), on());
    let first_two = Expr::Cells(vec![cell(local(4), 0), cell(local(4), 1)]);
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [used(0x100, vec![first_two]), used(0x100, vec![local(4)])]
    );
    assert_eq!(
        shape(&made, Var::Local(4)),
        Some((Width::Cells, Kind::Aggregate(3)))
    );
}

#[test]
fn bytes_taken_out_of_a_group_of_one_cell_are_bytes_of_that_cell() {
    let body = |value: Expr| {
        let taken = Expr::cast(Scalar::I32, extract(value, 4, 4));
        vec![set(Var::Local(0), taken)]
    };
    let f = function(2, body(Expr::Cells(vec![param(1)])), leaves());
    let made = grouped(f.clone(), Project::empty(), on());
    assert_eq!(made.cfg.blocks[0].stmts, body(param(1)));
    let counted = Made {
        bytes: 1,
        ..Made::default()
    };
    assert_eq!(apply(&mut f.clone(), Project::empty()), counted);

    // With the pass switched off nothing changes, and a group of two is no one cell.
    let off = on().without(Pass::Aggregates);
    assert_eq!(grouped(f.clone(), Project::empty(), off), f);
    let two = function(2, body(Expr::Cells(vec![param(0), param(1)])), leaves());
    assert_eq!(grouped(two.clone(), Project::empty(), on()), two);
}

#[test]
fn bytes_out_of_bytes_they_lie_within_are_bytes_of_the_value_under_both() {
    let group = || Expr::Cells(vec![param(0), param(1), param(2)]);
    let body = |taken: Expr| vec![used(0x100, vec![taken])];
    let f = function(3, body(extract(extract(group(), 4, 16), 8, 4)), leaves());
    let made = grouped(f, Project::empty(), on());
    assert_eq!(made.cfg.blocks[0].stmts, body(extract(group(), 12, 4)));

    // Bytes past those taken first, or bytes first taken past their value's cells, stay as they are.
    let past = function(3, body(extract(extract(group(), 4, 8), 4, 8)), leaves());
    assert_eq!(grouped(past.clone(), Project::empty(), on()), past);
    let beyond = function(3, body(extract(extract(group(), 20, 8), 0, 4)), leaves());
    assert_eq!(grouped(beyond.clone(), Project::empty(), on()), beyond);
}

#[test]
fn every_byte_of_a_value_whose_cells_a_call_states_is_the_value() {
    let body = |taken: Expr| vec![used(0x200, vec![taken])];
    let result = |cells: u32| call(0x100, vec![], Some(cells));
    let f = function(1, body(extract(result(2), 0, 16)), leaves());
    let made = grouped(f, Project::empty(), on());
    assert_eq!(made.cfg.blocks[0].stmts, body(result(2)));

    // Fewer bytes than the value holds, and every byte of one cell, stay bytes.
    let fewer = function(1, body(extract(result(3), 0, 16)), leaves());
    assert_eq!(grouped(fewer.clone(), Project::empty(), on()), fewer);
    let one = function(1, body(extract(result(1), 0, 8)), leaves());
    assert_eq!(grouped(one.clone(), Project::empty(), on()), one);
}

/// `f` with local 4 a variable of two cells, as a pass before this one makes one.
fn with_pair(mut f: Function) -> Function {
    let held = VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(2),
        name: None,
        single: false,
    };
    f.head.vars.insert(Var::Local(4), held);
    f
}

#[test]
fn the_cells_of_a_local_made_of_two_before_are_that_local_in_order() {
    let body = |row: Vec<Expr>| {
        let taken = extract(cell(local(4), 1), 0, 4);
        vec![used(0x100, row), set(Var::Local(0), taken)]
    };
    let f = with_pair(function(
        1,
        body(vec![cell(local(4), 0), cell(local(4), 1)]),
        leaves(),
    ));
    let made = grouped(f, Project::empty(), on());
    let whole = [
        used(0x100, vec![local(4)]),
        set(Var::Local(0), extract(local(4), 8, 4)),
    ];
    assert_eq!(made.cfg.blocks[0].stmts, whole);
    assert_eq!(
        shape(&made, Var::Local(4)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );

    // Out of order the cells stay cells.
    let turned = vec![cell(local(4), 1), cell(local(4), 0)];
    let f = with_pair(function(1, body(turned.clone()), leaves()));
    let made = grouped(f, Project::empty(), on());
    assert_eq!(made.cfg.blocks[0].stmts[0], used(0x100, turned));
}

#[test]
fn a_parameter_given_each_cell_of_a_local_made_before_in_order_is_given_the_local() {
    let project = knowing(AT, takes(&[(1, Kind::Unknown), (2, Kind::Unknown)]));
    let body = |second: Expr| {
        vec![
            set(Var::Param(1), cell(local(4), 0)),
            set(Var::Param(2), second),
            used(0x100, vec![param(1), param(2)]),
        ]
    };
    let f = with_pair(function(3, body(cell(local(4), 1)), leaves()));
    let made = grouped(f, &project, on());
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [set(Var::Param(1), local(4)), used(0x100, vec![param(1)])]
    );

    // A cell of the local twice is no copy of it: the parameter takes the two cells as they are.
    let f = with_pair(function(3, body(cell(local(4), 0)), leaves()));
    let made = grouped(f, &project, on());
    let cells = Expr::Cells(vec![cell(local(4), 0), cell(local(4), 0)]);
    assert_eq!(made.cfg.blocks[0].stmts[0], set(Var::Param(1), cells));
}

/// `load_bytes(&local_n, offset, bytes)`.
fn loaded(n: u32, offset: u32, bytes: u32) -> Expr {
    Expr::LoadBytes {
        addr: Box::new(address(n)),
        offset,
        bytes,
    }
}

#[test]
fn locals_loaded_whole_through_the_first_ones_address_are_one_local() {
    let body = |last: Option<Stmt>, load: Expr| {
        let mut stmts = vec![
            set(Var::Local(4), param(0)),
            set(Var::Local(5), param(1)),
            used(0x100, vec![load]),
        ];
        stmts.extend(last);
        stmts
    };
    let made = grouped(
        function(2, body(None, loaded(4, 0, 16)), leaves()),
        Project::empty(),
        on(),
    );
    assert_eq!(
        made.cfg.blocks[0].stmts,
        [
            set(Var::Local(4), Expr::Cells(vec![param(0), param(1)])),
            used(0x100, vec![local(4)]),
        ]
    );
    assert_eq!(
        shape(&made, Var::Local(4)),
        Some((Width::Cells, Kind::Aggregate(2)))
    );
    assert_eq!(shape(&made, Var::Local(5)), None);

    let untouched = |f: Function| assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
    // A cell assigned alone is a cell of its own.
    let alone = body(Some(set(Var::Local(5), int(1))), loaded(4, 0, 16));
    untouched(function(2, alone, leaves()));
    // Bytes that end inside a cell, or start past the first byte, are no value of whole cells.
    untouched(function(2, body(None, loaded(4, 0, 12)), leaves()));
    untouched(function(2, body(None, loaded(4, 8, 16)), leaves()));
    // With the pass switched off nothing is grouped.
    let f = function(2, body(None, loaded(4, 0, 16)), leaves());
    let off = on().without(Pass::Aggregates);
    assert_eq!(grouped(f.clone(), Project::empty(), off), f);
}

/// `zero_bytes(byte_offset(&v, offset), bytes);`.
fn field_clear(v: Var, offset: i32, bytes: u32) -> Stmt {
    let own = Expr::AddrOf { var: v, cell: 0 };
    Stmt::ZeroBytes {
        addr: Expr::ByteOffset {
            addr: Box::new(own),
            offset: Box::new(int(offset)),
        },
        bytes,
    }
}

#[test]
fn a_clear_of_a_field_of_a_value_of_several_cells_is_that_field_made_zero() {
    let body = |v: Var, clear: Stmt, after: Option<Stmt>| {
        let mut stmts = vec![set(v, Expr::ZeroSlots(4)), clear];
        stmts.extend(after);
        stmts.push(used(0x100, vec![Expr::Var(v)]));
        stmts
    };
    let cleared = |clear: Stmt, after: Option<Stmt>| {
        let mut f = function(1, Vec::new(), leaves());
        let v = wide(&mut f);
        f.cfg.blocks[0].stmts = body(v, clear, after);
        (f, v)
    };
    let (f, v) = cleared(field_clear(Var::Value { at: 0x10, index: 0 }, 16, 16), None);
    let mut f2 = f.clone();
    let made = grouped(f, Project::empty(), on());
    let zero = Stmt::StoreBytes {
        addr: Expr::AddrOf { var: v, cell: 0 },
        offset: 16,
        bytes: 16,
        value: Expr::ZeroSlots(2),
    };
    assert_eq!(made.cfg.blocks[0].stmts, body(v, zero, None));
    assert_eq!(apply(&mut f2, Project::empty()).zeros, 1);

    let untouched = |clear: Stmt, after: Option<Stmt>| {
        let (f, _) = cleared(clear, after);
        assert_eq!(grouped(f.clone(), Project::empty(), on()), f);
    };
    let v = Var::Value { at: 0x10, index: 0 };
    // Bytes that end inside a cell, a clear past the value's end, and one from its first byte stay clears.
    untouched(field_clear(v, 16, 12), None);
    untouched(field_clear(v, 24, 16), None);
    untouched(field_clear(v, 0, 16), None);
    // An address of the value handed on may be read for what the clear leaves.
    let handed = used(0x200, vec![Expr::AddrOf { var: v, cell: 0 }]);
    untouched(field_clear(v, 16, 16), Some(handed));
    // With the pass switched off the clear stays.
    let (f, _) = cleared(field_clear(v, 16, 16), None);
    let off = on().without(Pass::Aggregates);
    assert_eq!(grouped(f.clone(), Project::empty(), off), f);
}
