use super::super::ir::{BinOp, Const, Scalar, StoreKind, TypeKey, VmType};
use super::super::project::{Signature, TypeFacts};
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
    lifted_in(steps, Project::empty())
}

/// The function of `steps` lifted with what `project` states of the types its calls name.
fn lifted_in(steps: Vec<Vec<Op>>, project: &Project) -> Result<cfg::Function, LiftError> {
    let insns = code(steps);
    let row = FunctionRow {
        at: AT,
        end: at(insns.len() as u32),
        params: 2,
        locals: 0,
        returns: 1,
    };
    super::steps(0x1122_3344, &row, &insns, &Slots::new(project))
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

/// A call of slot 3 of the script type numbered `ty` that takes its receiver and states no result.
fn slot_call(ty: u64) -> Op {
    let ty = TypeKey(ty).to_string();
    Op::Unsized {
        callee: Callee::Virtual {
            class: ty.clone(),
            method: "Method_3".to_owned(),
            ty,
            slot: 3,
        },
        args: Some(1),
        returns: None,
    }
}

/// The cells the first statement of the block at instruction 0 says its call returns.
fn first_returns(f: &cfg::Function) -> Option<u32> {
    match block(f, 0).stmts.first() {
        Some(Stmt::Assign {
            value: Expr::Call { returns, .. },
            ..
        }) => *returns,
        _ => None,
    }
}

#[test]
fn a_slot_its_type_leaves_open_returns_what_the_types_below_state() {
    let (top, one, other) = (TypeKey(0xA1), TypeKey(0xA2), TypeKey(0xA3));
    let filled = FunctionKey::new(0x1122_3344, 0x900);
    let under = |base: TypeKey, slots: &[(u32, FunctionKey)], returns: &[(u32, u32)]| TypeFacts {
        base: Some(base),
        slots: slots.iter().copied().collect(),
        returns: returns.iter().copied().collect(),
        ..TypeFacts::default()
    };
    let mut project = Project::default();
    project.types.insert(top, TypeFacts::default());
    // One type below fills the slot with a function, one below that has a record for it.
    project.types.insert(one, under(top, &[(3, filled)], &[]));
    project.types.insert(other, under(one, &[], &[(3, 2)]));
    let stated = Signature {
        returns: 2,
        ..Signature::default()
    };
    project.signatures.insert(filled, stated);
    // Any count balances before code that ends nowhere: alone, nothing sizes the call.
    let steps = || vec![vec![push(p(0)), slot_call(top.0)], vec![Op::Unreachable]];
    assert!(lifted(steps()).is_err());
    let f = lifted_in(steps(), &project).expect("the function lifts");
    assert_eq!(first_returns(&f), Some(2));

    // ⚠ Two types below that state different counts say nothing of the slot.
    project
        .types
        .insert(TypeKey(0xA4), under(top, &[], &[(3, 1)]));
    assert!(lifted_in(steps(), &project).is_err());
}

#[test]
fn the_calls_of_one_slot_take_one_count() {
    // Two results handed to a call of four cells.
    let two = |first: u64, second: u64| {
        lifted(vec![
            vec![push(p(0)), slot_call(first)],
            vec![push(p(1)), slot_call(second)],
            vec![call(4, 0)],
            done(),
        ])
    };
    let f = two(0xB1, 0xB1).expect("the function lifts");
    assert_eq!(first_returns(&f), Some(2));
    // Two slots say nothing of each other: every split of the four cells balances.
    assert!(two(0xB1, 0xB2).is_err());
}

#[test]
fn a_run_of_array_reads_is_sized_one_read_at_a_time() {
    let read = || Op::Unsized {
        callee: operation("array_get_value"),
        args: Some(2),
        returns: None,
    };
    // Thirty elements of two cells, each stored whole. ⚠ The counts balance in many ways,
    // three cells of one read and one of the next among them: only the rule that an element
    // is taken whole leaves one, and it is asked at each read, not of each way.
    let mut steps = Vec::new();
    for n in 0..30 {
        steps.push(vec![push(int(7)), push(int(n)), read()]);
        steps.push(vec![Op::Set(Var::Local(1)), Op::Set(Var::Local(0))]);
    }
    steps.push(done());
    let f = lifted(steps).expect("the function lifts");
    let two_cells = |stmt: &&Stmt| {
        matches!(
            stmt,
            Stmt::Assign {
                value: Expr::Call {
                    returns: Some(2),
                    ..
                },
                ..
            }
        )
    };
    assert_eq!(block(&f, 0).stmts.iter().filter(two_cells).count(), 30);
}

#[test]
fn the_walk_of_a_choice_says_where_it_stopped() {
    let read = || Op::Unsized {
        callee: operation("array_get_value"),
        args: Some(2),
        returns: None,
    };
    let insns = code(vec![
        vec![push(p(0)), push(p(1)), read()],
        vec![Op::Set(Var::Local(1)), Op::Set(Var::Local(0))],
        vec![push(p(0)), push(p(1)), read()],
        done(),
    ]);
    let mut spent = 0;
    let mut sizes = Sizes::new();
    assert_eq!(
        elements(&insns, &sizes, true, &mut spent),
        Elements::Open(0)
    );
    // Three cells of which two are stored: a part of the element is taken, whatever the next read yields.
    sizes.insert(at(0), (2, 3));
    assert_eq!(elements(&insns, &sizes, true, &mut spent), Elements::Broken);
    sizes.insert(at(0), (2, 1));
    assert_eq!(elements(&insns, &sizes, true, &mut spent), Elements::Short);
    sizes.insert(at(0), (2, 2));
    assert_eq!(
        elements(&insns, &sizes, true, &mut spent),
        Elements::Open(2)
    );
    // Asked of a choice that passed for whole, the walk stops at nothing.
    assert_eq!(elements(&insns, &sizes, false, &mut spent), Elements::Hold);
    assert!(spent > 0);
}

#[test]
fn a_function_starts_wherever_the_project_states_one_or_knows_one_called() {
    let (here, there) = (0x1122_3344, 0x5566_7788);
    let mut project = Project::default();
    let mut state = |program: u32, at: u32| {
        let function = FunctionKey::new(program, at);
        project.signatures.insert(function, Signature::default());
    };
    state(here, 0x10);
    state(there, 0x20);
    // A function of each program that only the other's code is known to call.
    let called = vec![FunctionKey::new(here, 0x40), FunctionKey::new(there, 0x50)];
    project.calls.insert(FunctionKey::new(there, 0x20), called);
    assert_eq!(starts(here, &project), BTreeSet::from([0x10, 0x40]));
    assert_eq!(starts(there, &project), BTreeSet::from([0x20, 0x50]));
}

/// The steps lifted with the value type each instruction numbered in `typed` names.
fn lifted_typed(steps: Vec<Vec<Op>>, typed: &[(u32, TypeKey)]) -> Result<cfg::Function, LiftError> {
    let insns = code(steps);
    let row = FunctionRow {
        at: AT,
        end: at(insns.len() as u32),
        params: 2,
        locals: 0,
        returns: 0,
    };
    let types: BTreeMap<u32, TypeKey> = typed.iter().map(|(n, ty)| (at(*n), *ty)).collect();
    let slots = Slots::new(Project::empty());
    super::steps_typed(0x1122_3344, &row, &insns, &slots, &types).map(|(f, _)| f)
}

fn address(var: Var) -> Op {
    push(Expr::AddrOf { var, cell: 0 })
}

fn store_bytes(offset: u32, bytes: u32) -> Op {
    let cells = bytes.div_ceil(8);
    Op::StoreBytes {
        offset,
        bytes,
        cells,
    }
}

#[test]
fn a_value_stored_whole_states_its_type_where_it_goes_and_where_it_comes_from() {
    let thing = TypeKey(0xD1);
    let steps = |offset: u32| {
        vec![
            vec![address(Var::Local(2)), push(local(6)), push(local(7))],
            vec![store_bytes(offset, 16)],
            done(),
        ]
    };
    let f = lifted_typed(steps(0), &[(1, thing)]).expect("the function lifts");
    let stated = BTreeMap::from([
        (Var::Local(2), Some((thing, 2))),
        (Var::Local(6), Some((thing, 2))),
    ]);
    assert_eq!(f.head.value_types, stated);
    // The same store with no type named, and one into a part of what the address reaches.
    let untyped = lifted_typed(steps(0), &[]).expect("the function lifts");
    assert!(untyped.head.value_types.is_empty());
    let inside = lifted_typed(steps(8), &[(1, thing)]).expect("the function lifts");
    assert!(inside.head.value_types.is_empty());
    // The statement is the same store of bytes whatever the type.
    assert_eq!(block(&f, 0).stmts, block(&untyped, 0).stmts);
}

#[test]
fn a_value_taken_out_of_a_box_states_its_type_and_so_do_the_cells_it_is_copied_to_in_order() {
    let thing = TypeKey(0xD1);
    let unboxed = vec![
        push(p(0)),
        Op::LoadBytes {
            offset: 8,
            bytes: 16,
        },
    ];
    let f = lifted_typed(
        vec![
            unboxed.clone(),
            vec![Op::Set(Var::Local(5)), Op::Set(Var::Local(4))],
            done(),
        ],
        &[(0, thing)],
    )
    .expect("the function lifts");
    let stated = BTreeMap::from([
        (v(0, 0), Some((thing, 2))),
        (Var::Local(4), Some((thing, 2))),
    ]);
    assert_eq!(f.head.value_types, stated);
    // Its cells stored crossed over are no value of the type at either place.
    let crossed = lifted_typed(
        vec![
            unboxed,
            vec![Op::Set(Var::Local(4)), Op::Set(Var::Local(5))],
            done(),
        ],
        &[(0, thing)],
    )
    .expect("the function lifts");
    let only = BTreeMap::from([(v(0, 0), Some((thing, 2)))]);
    assert_eq!(crossed.head.value_types, only);
}

#[test]
fn a_builtin_value_made_in_place_states_its_type_and_two_types_named_state_none() {
    let made = TypeKey(0x8000_0015);
    let zeroed = Op::Push {
        e: Expr::ZeroSlots(2),
        cells: 2,
    };
    let steps = || {
        vec![
            vec![
                zeroed.clone(),
                Op::AddrOfTop { cells: 2 },
                operate("ctor", 1, 0),
            ],
            vec![Op::Drop { cells: 2 }],
            done(),
        ]
    };
    let f = lifted_typed(steps(), &[(0, made)]).expect("the function lifts");
    assert_eq!(
        f.head.value_types,
        BTreeMap::from([(v(0, 0), Some((made, 2)))])
    );
    // Zero cells made with no type named, as `alloc_value` makes them.
    let alloc = lifted_typed(steps(), &[]).expect("the function lifts");
    assert!(alloc.head.value_types.is_empty());

    let (one, other) = (TypeKey(0xD1), TypeKey(0xD2));
    let twice = lifted_typed(
        vec![
            vec![address(Var::Local(2)), push(local(6))],
            vec![store_bytes(0, 8)],
            vec![address(Var::Local(2)), push(local(6))],
            vec![store_bytes(0, 8)],
            done(),
        ],
        &[(1, one), (3, other)],
    )
    .expect("the function lifts");
    let neither = BTreeMap::from([(Var::Local(2), None), (Var::Local(6), None)]);
    assert_eq!(twice.head.value_types, neither);
}

/// A function that hands the results of slot 3 of two script types to a call of four cells.
fn two_slots(first: u64, second: u64) -> Vec<Vec<Op>> {
    vec![
        vec![push(p(0)), slot_call(first)],
        vec![push(p(1)), slot_call(second)],
        vec![call(4, 0)],
        done(),
    ]
}

/// The steps lifted with `slots`, and the counts their slot calls were sized with.
fn sized_with(
    steps: Vec<Vec<Op>>,
    slots: &Slots<'_>,
) -> Result<(cfg::Function, Counts), LiftError> {
    let insns = code(steps);
    let row = FunctionRow {
        at: AT,
        end: at(insns.len() as u32),
        params: 2,
        locals: 0,
        returns: 0,
    };
    super::steps_typed(0x1122_3344, &row, &insns, slots, &BTreeMap::new())
}

#[test]
fn a_slot_another_function_sized_alone_settles_a_function_its_calls_left_open() {
    let (known, open) = (TypeKey(0xC1), TypeKey(0xC2));
    let alone = Slots::new(Project::empty());
    // One slot handed to a call of two cells takes two.
    let teacher = vec![
        vec![push(p(0)), slot_call(known.0)],
        vec![call(2, 0)],
        done(),
    ];
    let (_, found) = sized_with(teacher, &alone).expect("the function lifts");
    assert_eq!(found, vec![((known, 3), 2)]);
    // Two slots handed to four cells balance in every split of them.
    let refused = sized_with(two_slots(known.0, open.0), &alone);
    assert!(matches!(&refused, Err(why) if settled_later(why)));

    let mut learned = Learned::new();
    learn(&mut learned, &found);
    let slots = Slots::new(Project::empty());
    assert!(slots.bearing(&code(two_slots(known.0, open.0))).is_empty());
    assert!(slots.install(&learned));
    assert!(!slots.install(&learned));
    let bearing = slots.bearing(&code(two_slots(known.0, open.0)));
    assert_eq!(bearing, vec![((known, 3), 2)]);
    let settled = sized_with(two_slots(known.0, open.0), &slots);
    let (f, settled) = settled.expect("the function lifts");
    assert_eq!(first_returns(&f), Some(2));
    assert_eq!(settled, vec![((known, 3), 2), ((open, 3), 2)]);
    // A count learned of another slot says nothing here.
    let elsewhere = Slots::new(Project::empty());
    elsewhere.install(&BTreeMap::from([((TypeKey(0xC3), 3), Some(2))]));
    assert!(elsewhere
        .bearing(&code(two_slots(known.0, open.0)))
        .is_empty());
    assert!(sized_with(two_slots(known.0, open.0), &elsewhere).is_err());
}

#[test]
fn a_slot_two_functions_or_two_types_below_disagree_on_is_learned_as_nothing() {
    let (known, open) = (TypeKey(0xC1), TypeKey(0xC2));
    let mut learned = Learned::new();
    learn(&mut learned, &[((known, 3), 2)]);
    learn(&mut learned, &[((known, 3), 1)]);
    assert_eq!(learned.get(&(known, 3)), Some(&None));
    let slots = Slots::new(Project::empty());
    slots.install(&learned);
    assert!(slots.bearing(&code(two_slots(known.0, open.0))).is_empty());
    assert!(sized_with(two_slots(known.0, open.0), &slots).is_err());

    // ⚠ Two types below that state different counts make the slot no one signature: what a
    // function took of it is not held either.
    let under = |returns: u32| TypeFacts {
        base: Some(known),
        returns: BTreeMap::from([(3, returns)]),
        ..TypeFacts::default()
    };
    let mut project = Project::default();
    project.types.insert(known, TypeFacts::default());
    project.types.insert(TypeKey(0xC4), under(1));
    project.types.insert(TypeKey(0xC5), under(3));
    let disputed = Slots::new(&project);
    disputed.install(&BTreeMap::from([((known, 3), Some(2))]));
    assert!(disputed
        .bearing(&code(two_slots(known.0, open.0)))
        .is_empty());
    assert!(sized_with(two_slots(known.0, open.0), &disputed).is_err());
}

#[test]
fn only_a_function_refused_for_want_of_a_count_is_lifted_again() {
    let stack = |what: &str| LiftError::Stack {
        function: AT,
        at: AT,
        what: what.to_owned(),
    };
    assert!(settled_later(&stack(AMBIGUOUS)));
    assert!(settled_later(&stack(TOO_MANY)));
    assert!(!settled_later(&stack(
        "no count of cells for a call's result balances the stack"
    )));
    assert!(!settled_later(&LiftError::Disasm(DisasmError::Truncated {
        at: AT
    })));
}

/// An instruction written by hand: the word of the opcode named, then each operand byte under the key of its place.
fn encoded(name: &str, operands: &[u8]) -> Vec<u8> {
    let opcode = (0..512u16).find(|n| super::super::opcode::name(*n) == Some(name));
    let opcode = opcode.expect("an opcode of that name");
    let mut bytes = opcode.wrapping_mul(0x843D).to_le_bytes().to_vec();
    let stored = operands.iter().enumerate().map(|(place, plain)| {
        let key = 0x85 ^ 0x11u8.wrapping_mul(place as u8);
        plain.wrapping_mul(0x65) ^ key
    });
    bytes.extend(stored);
    bytes
}

#[test]
fn a_program_lifts_a_function_its_calls_left_open_with_what_its_other_functions_took() {
    let ty = |name: u64| super::super::program::TypeRow {
        name,
        ..Default::default()
    };
    // Slot 3 of the program's type `n`, handed its receiver; no table states what it returns.
    let slot = |n: u8| encoded("call_virtual", &[1, n, 0, 3]);
    let call = |at: u8| encoded("call_short", &[0, at, 0]);
    let leave = || encoded("return", &[]);
    let code = [
        // 0x00 takes two cells and 0x02 four.
        leave(),
        leave(),
        // 0x04: one slot's result handed to the function of two cells.
        encoded("push_param_0", &[]),
        slot(0),
        call(0x00),
        leave(),
        // 0x13: the results of two slots handed to the function of four.
        encoded("push_param_0", &[]),
        slot(0),
        encoded("push_param_1", &[]),
        slot(1),
        call(0x02),
        leave(),
    ]
    .concat();
    let row = |at: u32, end: u32, params: u32| FunctionRow {
        at,
        end,
        params,
        locals: 0,
        returns: 0,
    };
    let program = Program {
        program: 0x1122_3344,
        code,
        functions: vec![
            row(0x00, 0x02, 2),
            row(0x02, 0x04, 4),
            row(0x04, 0x13, 1),
            row(0x13, 0x2A, 2),
        ],
        types: vec![ty(0xC1), ty(0xC2)],
        ..Program::default()
    };
    let learner = program.functions[3];
    let alone = function_in(&program, Project::empty(), &learner);
    assert!(matches!(&alone, Err(why) if settled_later(why)));

    let all = program_in(&program, Project::empty());
    assert_eq!(all.len(), 4);
    let Some(Ok(lifted)) = all.get(3) else {
        panic!("the function lifts with what the one before it took: {all:?}");
    };
    let start = lifted
        .function
        .cfg
        .blocks
        .iter()
        .find(|b| b.at == Some(0x13));
    let start = start.expect("a block starts at the function");
    let results: Vec<Option<u32>> = (start.stmts.iter())
        .filter_map(|stmt| match stmt {
            Stmt::Assign {
                value: Expr::Call { returns, .. },
                ..
            } => Some(*returns),
            _ => None,
        })
        .collect();
    assert_eq!(results, vec![Some(2), Some(2)]);
}

/// A value of one zero cell made under the two cells on top, as the decoder writes `alloc_value`, and its address above it.
fn made_under_two() -> Vec<Op> {
    let zeroed = Op::Push {
        e: Expr::ZeroSlots(1),
        cells: 1,
    };
    let raise = Op::Raise { depth: 4 };
    vec![zeroed, Op::AddrOfTop { cells: 1 }, raise.clone(), raise]
}

#[test]
fn a_run_of_raises_lifts_an_element_whole_over_a_value_made_under_it() {
    let read = || Op::Unsized {
        callee: operation("array_get_value"),
        args: Some(2),
        returns: None,
    };
    // An element read, a value made under it, and the address with the element handed to a call.
    let steps = || {
        vec![
            vec![push(p(0)), push(p(1)), read()],
            made_under_two(),
            vec![call(3, 0)],
            vec![Op::Drop { cells: 1 }],
            done(),
        ]
    };
    let f = lifted(steps()).expect("the function lifts");
    let element = operated("array_get_value", vec![p(0), p(1)], 2);
    assert_eq!(block(&f, 0).stmts.first(), Some(&set(v(0, 0), element)));

    // ⚠ The two raises move the element's two cells as one block: of three, they move two and leave one.
    let insns = code(steps());
    let mut spent = 0;
    let mut sizes = Sizes::new();
    sizes.insert(at(0), (2, 2));
    assert_eq!(elements(&insns, &sizes, true, &mut spent), Elements::Hold);
    sizes.insert(at(0), (2, 3));
    assert_eq!(elements(&insns, &sizes, true, &mut spent), Elements::Broken);
}

#[test]
fn a_count_that_leaves_cells_at_a_return_is_turned_down_with_every_count_above_it() {
    // Four slots of four types, none of whose results anything takes: every count but none
    // leaves cells at the return, and the products of the counts are past what the search walks.
    let steps = vec![
        vec![push(p(0)), slot_call(0xD1)],
        vec![push(p(0)), slot_call(0xD2)],
        vec![push(p(0)), slot_call(0xD3)],
        vec![push(p(0)), slot_call(0xD4)],
        done(),
    ];
    let f = lifted(steps).expect("the function lifts");
    let yields_none = |stmt: &&Stmt| {
        matches!(
            stmt,
            Stmt::Eval(Expr::Call {
                returns: Some(0),
                ..
            })
        )
    };
    assert_eq!(block(&f, 0).stmts.iter().filter(yields_none).count(), 4);

    // The walk itself: a count above none leaves a cell at the return, by either way to it.
    let mut spent = 0;
    let alone = code(vec![vec![push(p(0)), slot_call(0xD1)], done()]);
    assert!(!too_deep(&alone, &Sizes::new(), &mut spent));
    assert!(too_deep(
        &alone,
        &Sizes::from([(at(0), (1, 1))]),
        &mut spent
    ));
    let met = code(vec![
        vec![push(p(0)), Op::Branch { to: at(2) }],
        vec![push(p(0)), slot_call(0xD1), Op::Jump { to: at(2) }],
        done(),
    ]);
    assert!(!too_deep(&met, &Sizes::new(), &mut spent));
    assert!(too_deep(&met, &Sizes::from([(at(1), (1, 1))]), &mut spent));
    // A jump the walk cannot follow proves nothing.
    let lost = code(vec![
        vec![push(p(0)), slot_call(0xD1)],
        vec![Op::Jump { to: 0x9999 }],
    ]);
    assert!(!too_deep(
        &lost,
        &Sizes::from([(at(0), (1, 1))]),
        &mut spent
    ));
    assert!(spent > 0);
}

#[test]
fn a_function_that_runs_past_the_last_byte_of_the_code_falls_off_the_bytecode() {
    let nop = || encoded("nop", &[]);
    let row = FunctionRow {
        at: 0,
        end: 4,
        params: 0,
        locals: 4,
        returns: 0,
    };
    let last = Program {
        program: 0x1122_3344,
        code: [nop(), nop()].concat(),
        functions: vec![row],
        ..Program::default()
    };
    let ran = function(&last, &row).expect("the function lifts");
    let blocks = &ran.function.cfg.blocks;
    let fallen = blocks.iter().filter(|b| b.term == Terminator::Unreachable);
    assert_eq!(fallen.count(), 1);

    // ⚠ Code after it would run next: running on into it is no end of the bytecode.
    let followed = Program {
        code: [nop(), nop(), encoded("return", &[])].concat(),
        ..last.clone()
    };
    let refused = function(&followed, &row);
    let off = "control runs off the end of the function";
    assert!(
        matches!(&refused, Err(LiftError::Stack { what, .. }) if what.as_str() == off),
        "{refused:?}"
    );
}
