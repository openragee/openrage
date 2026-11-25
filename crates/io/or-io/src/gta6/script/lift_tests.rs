use super::super::ir::{BinOp, Const, Scalar, StoreKind, VmType};
use super::*;

/// Where the functions of these tests start; an instruction is two bytes.
const AT: u32 = 0x100;

/// The offset of instruction `n`.
fn at(n: u32) -> u32 {
    AT + 2 * n
}

fn code(steps: Vec<Vec<Op>>) -> Vec<Insn> {
    let insns = steps.into_iter().zip(0u32..).map(|(ops, n)| Insn {
        at: at(n),
        len: 2,
        opcode: 0,
        ops,
    });
    insns.collect()
}

fn lifted(steps: Vec<Vec<Op>>) -> Result<cfg::Function, LiftError> {
    let insns = code(steps);
    let row = FunctionRow {
        at: AT,
        end: at(insns.len() as u32),
        params: 2,
        locals: 0,
        returns: 1,
    };
    super::steps(0x1122_3344, &row, &insns)
}

/// The block that starts at instruction `n`.
fn block(f: &cfg::Function, n: u32) -> &Block {
    let found = f.cfg.blocks.iter().find(|b| b.at == Some(at(n)));
    found.expect("a block starts at the instruction")
}

fn push(e: Expr) -> Op {
    Op::Push { e, cells: 1 }
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn int(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

/// Value `index` of the block, named at instruction `n`.
fn v(n: u32, index: u32) -> Var {
    Var::Value { at: at(n), index }
}

/// Cell `index` carried into the block at instruction `n`.
fn carry(n: u32, index: u32) -> Var {
    Var::Stack { at: at(n), index }
}

fn cell(var: Var, index: u32) -> Expr {
    let base = Box::new(Expr::Var(var));
    Expr::Cell { base, index }
}

fn callee() -> Callee {
    Callee::Local {
        at: 0x900,
        name: "func_000900".to_owned(),
    }
}

fn call(args: u32, returns: u32) -> Op {
    let callee = callee();
    Op::Call {
        callee,
        args,
        returns,
    }
}

fn called_with(args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: callee(),
        args,
        returns: None,
    }
}

fn operation(name: &str) -> Callee {
    Callee::Intrinsic {
        name: name.to_owned(),
        ty: None,
        wrapper: None,
    }
}

fn operate(name: &str, args: u32, returns: u32) -> Op {
    let callee = operation(name);
    Op::Call {
        callee,
        args,
        returns,
    }
}

fn operated(name: &str, args: Vec<Expr>, returns: u32) -> Expr {
    Expr::Call {
        callee: operation(name),
        args,
        returns: Some(returns),
    }
}

fn set(target: Var, value: Expr) -> Stmt {
    Stmt::Assign { target, value }
}

fn done() -> Vec<Op> {
    vec![Op::Return { cells: 0 }]
}

fn refused_at(result: Result<cfg::Function, LiftError>, n: u32) -> bool {
    matches!(result, Err(LiftError::Stack { at: by, .. }) if by == at(n))
}

#[test]
fn a_cell_stays_an_expression_until_something_takes_it() {
    let add = Op::Binary {
        op: BinOp::Add,
        at: Kind::I32,
    };
    let f = lifted(vec![
        vec![push(p(0))],
        vec![push(int(2)), add],
        vec![Op::Return { cells: 1 }],
    ])
    .expect("the function lifts");
    let sum = Expr::Binary {
        op: BinOp::Add,
        at: Kind::I32,
        l: Box::new(p(0)),
        r: Box::new(int(2)),
    };
    // A listing's first label leaves the entry empty, and the function's code is the block after.
    let blocks = vec![
        Block {
            at: None,
            stmts: Vec::new(),
            term: Terminator::Jump(1),
        },
        Block {
            at: Some(AT),
            stmts: Vec::new(),
            term: Terminator::Return(Some(sum)),
        },
    ];
    assert_eq!(f.cfg.blocks, blocks);
    assert!(f.head.vars.is_empty());
    assert_eq!(f.head.name, "func_000100");
    assert_eq!((f.head.returns, f.head.params), (Returns::Cell, 2));
}

#[test]
fn a_call_names_the_cells_left_pending_and_its_result() {
    let address = Expr::AddrOf {
        var: Var::Local(1),
        cell: 0,
    };
    let f = lifted(vec![
        vec![push(p(0)), push(int(7)), push(address), push(p(1))],
        vec![call(1, 1)],
        vec![Op::Drop { cells: 4 }],
        done(),
    ])
    .expect("the function lifts");
    // The parameter could change under the call; the constant and the address could not, and stay.
    let stmts = vec![set(v(1, 0), p(0)), set(v(1, 1), called_with(vec![p(1)]))];
    assert_eq!(block(&f, 0).stmts, stmts);
    assert_eq!(f.head.vars.len(), 2);
    assert_eq!(f.head.locals, 2);
}

#[test]
fn storing_a_copy_names_the_cell_that_stays() {
    let field = || Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(p(0)),
        offset: 0x10,
    };
    let load = Op::Load {
        ty: Scalar::I32,
        offset: 0x10,
    };
    let copy = Op::Copy { depth: 0, cells: 1 };
    let f = lifted(vec![
        vec![push(p(0)), load],
        vec![copy.clone(), Op::Set(Var::Local(0))],
        vec![Op::Drop { cells: 1 }],
        done(),
    ])
    .expect("the function lifts");
    let stmts = vec![set(v(1, 0), field()), set(Var::Local(0), field())];
    assert_eq!(block(&f, 0).stmts, stmts);

    // A value that has a name already is left as it is.
    let f = lifted(vec![
        vec![call(0, 1)],
        vec![copy, Op::Set(Var::Local(0))],
        vec![Op::Drop { cells: 1 }],
        done(),
    ])
    .expect("the function lifts");
    let stmts = vec![
        set(v(0, 0), called_with(Vec::new())),
        set(Var::Local(0), Expr::Var(v(0, 0))),
    ];
    assert_eq!(block(&f, 0).stmts, stmts);
}

#[test]
fn the_index_of_a_name_restarts_with_a_block_and_not_inside_one() {
    let f = lifted(vec![
        vec![call(0, 1)],
        vec![Op::Set(Var::Local(0))],
        vec![call(0, 1)],
        vec![Op::Branch { to: at(5) }],
        vec![call(0, 1), Op::Drop { cells: 1 }],
        done(),
    ])
    .expect("the function lifts");
    // The stack is empty after the store, and the next name still counts on.
    let stmts = vec![
        set(v(0, 0), called_with(Vec::new())),
        set(Var::Local(0), Expr::Var(v(0, 0))),
        set(v(2, 1), called_with(Vec::new())),
    ];
    assert_eq!(block(&f, 0).stmts, stmts);
    assert_eq!(
        block(&f, 4).stmts,
        vec![set(v(4, 0), called_with(Vec::new()))]
    );
    // With no cell left on the stack an arm is the block itself.
    let branch = Terminator::Branch {
        cond: Expr::Var(v(2, 1)),
        then: 3,
        els: 2,
    };
    assert_eq!(block(&f, 0).term, branch);
}

#[test]
fn cells_that_cross_a_branch_are_handed_on_by_each_arm() {
    let copy = Op::Copy { depth: 0, cells: 1 };
    let f = lifted(vec![
        vec![push(p(0)), copy, Op::Branch { to: at(3) }],
        vec![Op::Drop { cells: 1 }],
        done(),
        vec![Op::Return { cells: 1 }],
    ])
    .expect("the function lifts");
    let arm = |to: Var, block: BlockId| Block {
        at: None,
        stmts: vec![set(to, p(0))],
        term: Terminator::Jump(block),
    };
    let blocks = vec![
        Block {
            at: None,
            stmts: Vec::new(),
            term: Terminator::Jump(1),
        },
        Block {
            at: Some(AT),
            stmts: Vec::new(),
            term: Terminator::Branch {
                cond: p(0),
                then: 2,
                els: 3,
            },
        },
        arm(carry(3, 0), 5),
        arm(carry(1, 0), 4),
        Block {
            at: Some(at(1)),
            stmts: Vec::new(),
            term: Terminator::Return(None),
        },
        Block {
            at: Some(at(3)),
            stmts: Vec::new(),
            term: Terminator::Return(Some(Expr::Var(carry(3, 0)))),
        },
    ];
    assert_eq!(f.cfg.blocks, blocks);
    assert_eq!(f.head.vars.len(), 2);
}

#[test]
fn cells_that_cross_a_switch_are_handed_on_by_each_target() {
    // Two cases go to one block, a third to another, and no case to the next instruction.
    let function = |left: Vec<Op>, taken: u32| {
        let mut first = left;
        let cases = vec![(0, at(2)), (1, at(2)), (2, at(3))];
        let default = at(1);
        first.extend([push(p(1)), Op::Switch { cases, default }]);
        let back = || vec![Op::Return { cells: taken }];
        lifted(vec![first, back(), back(), back()]).expect("the function lifts")
    };
    let f = function(vec![push(p(0))], 1);
    let arm = |to: u32, block: BlockId| Block {
        at: None,
        stmts: vec![set(carry(to, 0), p(0))],
        term: Terminator::Jump(block),
    };
    let handed = |n: u32| Block {
        at: Some(at(n)),
        stmts: Vec::new(),
        term: Terminator::Return(Some(Expr::Var(carry(n, 0)))),
    };
    let switch = |cases: Vec<(i64, BlockId)>, default: BlockId| Block {
        at: Some(AT),
        stmts: Vec::new(),
        term: Terminator::Switch {
            value: p(1),
            cases,
            default,
        },
    };
    let entry = Block {
        at: None,
        stmts: Vec::new(),
        term: Terminator::Jump(1),
    };
    // One arm a target, in the order the cases name them, and the two cases of one target share theirs.
    let blocks = vec![
        entry.clone(),
        switch(vec![(0, 2), (1, 2), (2, 3)], 4),
        arm(2, 6),
        arm(3, 7),
        arm(1, 5),
        handed(1),
        handed(2),
        handed(3),
    ];
    assert_eq!(f.cfg.blocks, blocks);

    // With no cell left a case goes straight to its block.
    let f = function(Vec::new(), 0);
    let plain = |n: u32| Block {
        at: Some(at(n)),
        stmts: Vec::new(),
        term: Terminator::Return(None),
    };
    let blocks = vec![
        entry,
        switch(vec![(0, 3), (1, 3), (2, 4)], 2),
        plain(1),
        plain(2),
        plain(3),
    ];
    assert_eq!(f.cfg.blocks, blocks);
}

#[test]
fn two_cells_carried_are_one_assignment() {
    let f = lifted(vec![
        vec![push(p(0)), push(p(1)), Op::Jump { to: at(1) }],
        vec![Op::Return { cells: 2 }],
    ])
    .expect("the function lifts");
    let handed = Stmt::AssignMany {
        targets: vec![carry(1, 0), carry(1, 1)],
        values: vec![p(0), p(1)],
    };
    assert_eq!(block(&f, 0).stmts, vec![handed]);
    let both = Expr::Cells(vec![Expr::Var(carry(1, 0)), Expr::Var(carry(1, 1))]);
    assert_eq!(block(&f, 1).term, Terminator::Return(Some(both)));
}

#[test]
fn two_ways_in_that_leave_different_depths_are_refused() {
    let function = |extra: Vec<Op>| {
        lifted(vec![
            vec![push(p(0)), Op::Branch { to: at(2) }],
            extra,
            done(),
        ])
    };
    assert!(refused_at(function(vec![push(int(1))]), 2));
    assert!(function(vec![Op::Nop]).is_ok());
}

#[test]
fn a_step_that_takes_more_than_the_stack_holds_is_refused() {
    let add = Op::Binary {
        op: BinOp::Add,
        at: Kind::I32,
    };
    assert!(refused_at(lifted(vec![vec![push(p(0)), add], done()]), 0));
    let below = Op::Copy { depth: 1, cells: 1 };
    assert!(refused_at(lifted(vec![vec![push(p(0)), below], done()]), 0));
    assert!(refused_at(lifted(vec![vec![push(p(0))]]), 0));
    let away = Op::Jump { to: at(1) + 1 };
    assert!(refused_at(lifted(vec![vec![away], done()]), 0));
}

#[test]
fn code_the_entry_does_not_reach_is_left_out() {
    let f = lifted(vec![done(), vec![call(0, 0)], done()]).expect("the function lifts");
    assert_eq!(f.cfg.blocks.len(), 2);
    assert_eq!(f.cfg.statements(), 0);
}

#[test]
fn an_operation_of_the_runtime_folds_unless_it_reaches_the_heap() {
    let f = lifted(vec![
        vec![push(p(0)), operate("sinf", 1, 1)],
        vec![Op::Return { cells: 1 }],
    ])
    .expect("the function lifts");
    assert!(block(&f, 0).stmts.is_empty());
    let sine = operated("sinf", vec![p(0)], 1);
    assert_eq!(block(&f, 0).term, Terminator::Return(Some(sine)));

    let boxed = "box<type_0000000000000001>";
    let f = lifted(vec![
        vec![push(p(0)), push(p(1)), operate(boxed, 1, 1)],
        vec![Op::Drop { cells: 2 }],
        done(),
    ])
    .expect("the function lifts");
    let stmts = vec![
        set(v(0, 0), p(0)),
        set(v(0, 1), operated(boxed, vec![p(1)], 1)),
    ];
    assert_eq!(block(&f, 0).stmts, stmts);

    // Two cells out are named, and nothing else is; the barrier before a store names nothing.
    let barrier = "replication_barrier_462";
    let f = lifted(vec![
        vec![push(p(0)), push(p(1)), operate("packed_vector_splat", 1, 2)],
        vec![operate(barrier, 0, 0), Op::Drop { cells: 3 }],
        done(),
    ])
    .expect("the function lifts");
    let stmts = vec![
        set(v(0, 0), operated("packed_vector_splat", vec![p(1)], 2)),
        Stmt::Eval(operated(barrier, Vec::new(), 0)),
    ];
    assert_eq!(block(&f, 0).stmts, stmts);
    assert_eq!(
        f.head.vars.get(&v(0, 0)).map(|info| info.width),
        Some(Width::Cells)
    );
}

#[test]
fn a_new_object_put_under_its_arguments_names_what_lies_below_it() {
    let ty = VmType {
        name: "type_0000000000000001".to_owned(),
        byte_size: 24,
        id: 3,
        flags: 0,
    };
    let under = vec![
        push(Expr::NewObject(ty.clone())),
        Op::Copy { depth: 0, cells: 1 },
        Op::Sink { depth: 3 },
        Op::Sink { depth: 3 },
    ];
    let f = lifted(vec![
        vec![push(p(0)), push(p(1))],
        under,
        vec![Op::Drop { cells: 4 }],
        done(),
    ])
    .expect("the function lifts");
    // The argument above the object stays pending; the cell under it is named first.
    let stmts = vec![
        set(v(1, 0), p(0)),
        set(v(1, 1), Expr::NewObject(ty.clone())),
    ];
    assert_eq!(block(&f, 0).stmts, stmts);

    let f = lifted(vec![
        vec![push(p(0))],
        vec![push(Expr::NewObject(ty.clone()))],
        vec![Op::Drop { cells: 2 }],
        done(),
    ])
    .expect("the function lifts");
    assert_eq!(block(&f, 0).stmts, vec![set(v(1, 0), Expr::NewObject(ty))]);
}

#[test]
fn the_stores_of_one_instruction_are_written_deepest_first_while_none_sees_another() {
    let stores = || vec![Op::Set(Var::Local(5)), Op::Set(Var::Local(4))];
    let f = lifted(vec![vec![call(0, 2)], stores(), done()]).expect("the function lifts");
    let stmts = vec![
        set(v(0, 0), called_with(Vec::new())),
        set(Var::Local(4), cell(v(0, 0), 0)),
        set(Var::Local(5), cell(v(0, 0), 1)),
    ];
    assert_eq!(block(&f, 0).stmts, stmts);
    assert_eq!(f.head.locals, 6);

    // An exchange read after the first store would see it: the two stay one statement.
    let f = lifted(vec![vec![push(local(5)), push(local(4))], stores(), done()])
        .expect("the function lifts");
    let exchange = Stmt::AssignMany {
        targets: vec![Var::Local(4), Var::Local(5)],
        values: vec![local(5), local(4)],
    };
    assert_eq!(block(&f, 0).stmts, vec![exchange]);
}

#[test]
fn the_address_of_cells_is_the_address_of_the_value_the_block_named() {
    let zeroed = Op::Push {
        e: Expr::ZeroSlots(1),
        cells: 1,
    };
    let store = Op::Store {
        kind: StoreKind::I32,
        offset: 0,
    };
    // The decoder's steps for a value made under one argument, which then is stored into it.
    let f = lifted(vec![
        vec![push(p(0)), push(int(5))],
        vec![zeroed, Op::AddrOfTop { cells: 1 }, Op::Raise { depth: 3 }],
        vec![store],
        vec![Op::Drop { cells: 2 }],
        done(),
    ])
    .expect("the function lifts");
    let stmts = vec![
        set(v(1, 0), Expr::ZeroSlots(1)),
        set(v(2, 1), p(0)),
        Stmt::Store {
            kind: StoreKind::I32,
            addr: Expr::AddrOf {
                var: v(1, 0),
                cell: 0,
            },
            offset: 0,
            value: int(5),
        },
    ];
    assert_eq!(block(&f, 0).stmts, stmts);

    // A cell that is no named value's has no address to give.
    let loose = vec![push(p(0)), Op::AddrOfTop { cells: 1 }];
    assert!(refused_at(
        lifted(vec![loose, vec![Op::Drop { cells: 2 }], done()]),
        0
    ));
}

#[test]
fn a_result_no_table_sizes_takes_the_one_count_that_balances() {
    let read = || Op::Unsized {
        callee: operation("array_get_value"),
        args: Some(2),
        returns: None,
    };
    let f = lifted(vec![
        vec![push(p(0)), push(p(1)), read()],
        vec![Op::Return { cells: 3 }],
    ])
    .expect("the function lifts");
    let element = operated("array_get_value", vec![p(0), p(1)], 3);
    assert_eq!(block(&f, 0).stmts, vec![set(v(0, 0), element)]);

    // Where the code after it takes any number of cells, none is chosen.
    let open = lifted(vec![
        vec![push(p(0)), push(p(1)), read()],
        vec![Op::Unreachable],
    ]);
    assert!(refused_at(open, 0));
    let unknown = Op::Unsized {
        callee: callee(),
        args: None,
        returns: None,
    };
    assert!(refused_at(lifted(vec![vec![unknown], done()]), 0));
}

#[test]
fn the_elements_of_one_array_are_as_many_cells_wherever_it_is_read_or_stored() {
    let read = || Op::Unsized {
        callee: operation("array_get_value"),
        args: Some(2),
        returns: None,
    };
    // Two elements handed to a call of four cells: one and three, two and two, three and one.
    let two = |other: u32, stored: bool| {
        let mut steps = vec![
            vec![push(p(0)), push(int(0)), read()],
            vec![push(p(other)), push(int(1)), read()],
            vec![call(4, 0)],
        ];
        if stored {
            let cells = vec![push(p(0)), push(int(2)), push(int(5)), push(int(6))];
            steps.push(cells);
            steps.push(vec![operate("array_set_slots", 4, 0)]);
        }
        steps.push(done());
        lifted(steps)
    };
    // One array read twice, and stored to two cells at a time: each element is two cells.
    let f = two(0, true).expect("the function lifts");
    let element = operated("array_get_value", vec![p(0), int(0)], 2);
    assert_eq!(block(&f, 0).stmts.first(), Some(&set(v(0, 0), element)));
    assert!(two(0, false).is_ok());
    // ⚠ Two arrays say nothing of each other: every split of the four cells is still open,
    // until a store says what one of them holds.
    assert!(two(1, false).is_err());
    assert!(two(1, true).is_ok());
}

#[test]
fn a_step_that_takes_a_part_of_an_element_rules_its_count_out() {
    let read = Op::Unsized {
        callee: operation("array_get_value"),
        args: Some(2),
        returns: None,
    };
    // Three cells balance: one stored to a local and two handed on. ⚠ No instruction the
    // compiler writes takes one cell of an element and leaves the rest.
    let split = lifted(vec![
        vec![push(p(0)), push(p(1)), read.clone()],
        vec![Op::Set(Var::Local(0))],
        vec![call(2, 0)],
        done(),
    ]);
    assert!(split.is_err());
    // The whole element stored, cell by cell from the top, is one instruction's.
    let whole = lifted(vec![
        vec![push(p(0)), push(p(1)), read],
        vec![Op::Set(Var::Local(1)), Op::Set(Var::Local(0))],
        done(),
    ]);
    assert!(whole.is_ok());
}

#[test]
fn cells_read_out_of_memory_or_out_of_a_value_are_named_and_name_nothing_else() {
    let take = Op::ExtractBytes {
        cells: 2,
        offset: 4,
        bytes: 8,
    };
    let f = lifted(vec![
        vec![push(p(0)), push(p(0)), push(p(1))],
        vec![take, Op::Cast(Scalar::I32)],
        vec![Op::Swap, Op::Drop { cells: 1 }],
        vec![Op::Return { cells: 1 }],
    ])
    .expect("the function lifts");
    let taken = Expr::ExtractBytes {
        value: Box::new(Expr::Cells(vec![p(0), p(1)])),
        offset: 4,
        bytes: 8,
    };
    assert_eq!(block(&f, 0).stmts, vec![set(v(1, 0), taken)]);
    let read = Expr::cast(Scalar::I32, Expr::Var(v(1, 0)));
    assert_eq!(block(&f, 0).term, Terminator::Return(Some(read)));

    let load = Op::LoadBytes {
        offset: 8,
        bytes: 4,
    };
    let f = lifted(vec![vec![push(p(0)), load], vec![Op::Return { cells: 1 }]])
        .expect("the function lifts");
    let loaded = Expr::LoadBytes {
        addr: Box::new(p(0)),
        offset: 8,
        bytes: 4,
    };
    assert_eq!(block(&f, 0).stmts, vec![set(v(0, 0), loaded)]);
    assert_eq!(
        block(&f, 0).term,
        Terminator::Return(Some(cell(v(0, 0), 0)))
    );
}

#[test]
fn an_expression_nested_too_deep_is_refused() {
    let nested = |casts: u32| {
        let mut ops = vec![push(p(0))];
        ops.extend((0..casts).map(|_| Op::Cast(Scalar::I32)));
        lifted(vec![ops, vec![Op::Return { cells: 1 }]])
    };
    assert!(nested(3).is_ok());
    assert!(refused_at(nested(NESTING + 1), 0));
}

#[test]
fn a_cell_copied_until_it_is_no_longer_small_is_refused() {
    // `dup; add` doubles what the top cell holds: 30 rounds are a thousand million nodes.
    let doubled = |rounds: u32| {
        let mut ops = vec![push(p(0))];
        for _ in 0..rounds {
            ops.push(Op::Copy { depth: 0, cells: 1 });
            let (op, at) = (BinOp::Add, Kind::I32);
            ops.push(Op::Binary { op, at });
        }
        lifted(vec![ops, vec![Op::Return { cells: 1 }]])
    };
    assert!(doubled(8).is_ok());
    assert!(refused_at(doubled(30), 0));
}

#[test]
fn cells_handed_to_the_arms_of_many_branches_are_held_to_a_bound() {
    // 600 cells left on the stack at each of `tests` tests in a row: every arm is handed them all.
    let tests_of = |tests: u32| {
        let mut steps = vec![vec![push(p(0)); 600]];
        for n in 0..tests {
            steps.push(vec![push(p(1)), Op::Branch { to: at(n + 2) }]);
        }
        steps.push(vec![
            Op::Drop { cells: 600 },
            push(int(0)),
            Op::Return { cells: 1 },
        ]);
        lifted(steps)
    };
    assert!(tests_of(4).is_ok());
    assert!(tests_of(1000).is_err());
}

#[test]
fn a_function_the_tables_guard_is_refused_whole() {
    let mut program = Program::default();
    let row = FunctionRow {
        at: AT,
        end: at(1),
        ..FunctionRow::default()
    };
    // The program holds no code, so an unguarded function gets as far as the decoder's refusal.
    assert!(matches!(
        function(&program, &row),
        Err(LiftError::Disasm(_))
    ));
    program.handlers.push(super::super::program::HandlerRow {
        from: AT,
        to: at(1),
        handler: at(4),
        catches: None,
        finally: false,
    });
    assert!(matches!(
        function(&program, &row),
        Err(LiftError::Stack { .. })
    ));
}

#[test]
fn a_native_takes_the_hash_a_list_states_for_its_handlers_address() {
    let native = |rva: u32| Callee::Native {
        namespace: None,
        name: format!("rva_{rva:08X}"),
        hash: 0,
        rva,
    };
    let called = |rva: u32| Op::Call {
        callee: native(rva),
        args: 0,
        returns: 0,
    };
    let mut f =
        lifted(vec![vec![called(0x10)], vec![called(0x20)], done()]).expect("the function lifts");
    let read = |f: &cfg::Function| -> Vec<(u64, String)> {
        let calls = block(f, 0).stmts.iter().filter_map(|stmt| match stmt {
            Stmt::Eval(Expr::Call {
                callee: Callee::Native { name, hash, .. },
                ..
            }) => Some((*hash, name.clone())),
            _ => None,
        });
        calls.collect()
    };
    let unnamed = [
        (0, "rva_00000010".to_owned()),
        (0, "rva_00000020".to_owned()),
    ];

    // With no list a call keeps the address it was made by.
    let mut names = Names::default();
    hashed(&mut f, &names);
    assert_eq!(read(&f), unnamed);

    names.native_hashes.insert(0x10, 0xAA);
    hashed(&mut f, &names);
    let named = [
        (0xAA, "_0x00000000000000AA".to_owned()),
        (0, "rva_00000020".to_owned()),
    ];
    assert_eq!(read(&f), named);
}

#[test]
fn a_method_of_the_runtime_takes_the_path_a_list_states_for_its_numbers() {
    let builtin = |method: u32| Callee::Builtin {
        path: format!("type_80000015.Method_{method}"),
        ty: 0x15,
        method,
    };
    let called = |method: u32| Op::Call {
        callee: builtin(method),
        args: 0,
        returns: 0,
    };
    let mut f = lifted(vec![vec![called(4)], vec![called(5)], done()]).expect("the function lifts");
    let read = |f: &cfg::Function| -> Vec<String> {
        let calls = block(f, 0).stmts.iter().filter_map(|stmt| match stmt {
            Stmt::Eval(Expr::Call {
                callee: Callee::Builtin { path, .. },
                ..
            }) => Some(path.clone()),
            _ => None,
        });
        calls.collect()
    };
    let unnamed = ["type_80000015.Method_4", "type_80000015.Method_5"];

    // With no list a call keeps the numbers it was made by.
    let mut names = Names::default();
    hashed(&mut f, &names);
    assert_eq!(read(&f), unnamed);

    // A list names the one method it states, and no other of the type.
    let path = "Demo.Handle.op_Equality".to_owned();
    names.builtins.insert((0x15, 4), path);
    hashed(&mut f, &names);
    assert_eq!(
        read(&f),
        ["Demo.Handle.op_Equality", "type_80000015.Method_5"]
    );
}

#[test]
fn eight_bytes_taken_out_at_a_cell_boundary_are_that_cell() {
    let take = |offset: u32| Op::ExtractBytes {
        cells: 2,
        offset,
        bytes: 8,
    };
    let taken = |offset: u32| {
        let steps = vec![
            vec![push(p(0)), push(p(1)), take(offset)],
            vec![Op::Return { cells: 1 }],
        ];
        lifted(steps).expect("the function lifts")
    };
    // The second cell is the value, and nothing is named for it.
    let f = taken(8);
    assert_eq!(block(&f, 0).term, Terminator::Return(Some(p(1))));
    assert!(block(&f, 0).stmts.is_empty() && f.head.vars.is_empty());

    // ⚠ Eight bytes across two cells are neither of them.
    let f = taken(4);
    let across = matches!(
        block(&f, 0).stmts.as_slice(),
        [Stmt::Assign {
            value: Expr::ExtractBytes {
                offset: 4,
                bytes: 8,
                ..
            },
            ..
        }]
    );
    assert!(across, "{:?}", block(&f, 0).stmts);
}
