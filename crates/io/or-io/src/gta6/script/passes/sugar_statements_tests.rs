use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg;
use crate::gta6::script::exec;
use crate::gta6::script::ir::{Const, ExprSugar, Returns, StoreKind};
use crate::gta6::script::project::TypeFacts;
use crate::gta6::script::{Names, Options, Project};

// Made up: a script type's hash for the collection's interface and for the type whose slot
// yields the item, and two builtin indices for the enumerator's interface and the disposable's.
const ENUMERABLE: u64 = 0x1111_2222_3333_4444;
const ITEMS: u64 = 0x5555_6666_7777_8888;
const ENUMERATOR: u64 = 0x8000_0F01;
const DISPOSABLE: u64 = 0x8000_0F02;

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

fn set(n: u32, value: Expr) -> Stmt {
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

fn op(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: Some(name.to_owned()),
        },
        args,
        returns: Some(1),
    }
}

/// A call of slot 0 of the type `ty`.
fn slot(ty: u64, args: Vec<Expr>) -> Expr {
    let ty = TypeKey(ty).to_string();
    Expr::Call {
        callee: Callee::Virtual {
            class: ty.clone(),
            method: "Method_0".to_owned(),
            ty,
            slot: 0,
        },
        args,
        returns: Some(1),
    }
}

fn hidden(stmts: Vec<Stmt>) -> Stmt {
    Stmt::Sugar(Box::new(Sugar::Hidden(stmts)))
}

fn function(body: Vec<Stmt>) -> tree::Function {
    tree::Function {
        head: Head {
            value_types: Default::default(),
            program: 1,
            at: 0x10,
            name: "func_000010".to_owned(),
            returns: Returns::Cell,
            params: 1,
            locals: 8,
            vars: BTreeMap::new(),
            incomplete: None,
        },
        body,
    }
}

/// What the pass makes of `body` with every name on, and what it found.
fn read(body: Vec<Stmt>, names: &Names) -> (Vec<Stmt>, Found) {
    let mut f = function(body);
    let cx = Context::new(names, Project::empty(), Options::default());
    let found = rewrite(&mut f, &cx);
    (f.body, found)
}

/// [`read`] with the names of the blocks off: what is made of the loops alone.
// A loop that is no `foreach` may still be the body of a `using` of its enumerator.
fn read_loops(body: Vec<Stmt>, names: &Names) -> (Vec<Stmt>, Found) {
    let mut f = function(body);
    let loops = Options::default().without(Pass::Using).without(Pass::Lock);
    let found = rewrite(&mut f, &Context::new(names, Project::empty(), loops));
    (f.body, found)
}

/// [`read`] where `local_3` is known to be a value of three cells.
fn read_wide(body: Vec<Stmt>) -> (Vec<Stmt>, Found) {
    let mut f = function(body);
    let wide = VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(3),
        name: None,
        single: false,
    };
    f.head.vars.insert(Var::Local(3), wide);
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let found = rewrite(&mut f, &cx);
    (f.body, found)
}

/// `local_5 <op> array_length(local_<array>)`, both sides read as `int`.
fn length_test(op_: BinOp, array: u32) -> Expr {
    let length = Expr::cast(Scalar::I32, op("array_length", vec![local(array)]));
    Expr::binary(op_, Expr::cast(Scalar::I32, local(5)), length)
}

/// `local_6 = <reader>(local_<array>, local_5);`.
fn element(reader: &str, array: u32) -> Stmt {
    set(6, op(reader, vec![local(array), local(5)]))
}

/// `for (local_5 = 0; <cond>; local_5++) { <body> }`.
fn looped(cond: Expr, body: Vec<Stmt>) -> Stmt {
    let up = Expr::binary(BinOp::Add, Expr::cast(Scalar::I32, local(5)), int(1));
    Stmt::For {
        init: Some(Box::new(assign(5, int(0)))),
        cond: Some(cond),
        step: Some(Box::new(assign(5, up))),
        body,
    }
}

/// `for (local_5 = 0; local_5 < local_4.Length; local_5++) { local_6 = local_4[local_5]; <rest> }`.
fn counted(rest: Vec<Stmt>) -> Stmt {
    let mut body = vec![element("array_get_u64", 4)];
    body.extend(rest);
    looped(length_test(BinOp::Lt, 4), body)
}

#[test]
fn a_counted_loop_that_reads_each_element_first_is_a_foreach() {
    let was = vec![counted(vec![eval(call(vec![local(6)]))])];
    let (is, found) = read(was.clone(), &Names::default());
    let one = Found {
        arrays: 1,
        ..Found::default()
    };
    assert_eq!(found, one);
    let [Stmt::Sugar(made)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Foreach {
        setup,
        item,
        body,
        step,
        after,
        ..
    } = &**made
    else {
        panic!("{made:?}")
    };
    assert_eq!(setup, &vec![assign(5, int(0))]);
    assert_eq!(Stmt::Simple((**item).clone()), element("array_get_u64", 4));
    assert_eq!(body, &vec![eval(call(vec![local(6)]))]);
    assert!(step.is_some() && after.is_empty());
    // The way back: the blocks of the loop it was read from.
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}

#[test]
fn no_loop_is_read_while_its_name_is_off() {
    let was = vec![counted(Vec::new())];
    let names = Names::default();
    let mut f = function(was.clone());
    let off = Options::default().without(Pass::Foreach);
    let mut stats = Stats::default();
    run(
        &mut f,
        &Context::new(&names, Project::empty(), off),
        &mut stats,
    );
    assert_eq!(f.body, was);
    let on = Options::default();
    run(
        &mut f,
        &Context::new(&names, Project::empty(), on),
        &mut stats,
    );
    assert!(
        matches!(f.body.as_slice(), [Stmt::Sugar(_)]),
        "{:?}",
        f.body
    );
}

#[test]
fn a_loop_that_uses_its_index_its_array_or_its_item_otherwise_stays_a_for() {
    let touch = |n: u32| eval(call(vec![local(n)]));
    let address = |n: u32| {
        eval(call(vec![Expr::AddrOf {
            var: Var::Local(n),
            cell: 0,
        }]))
    };
    let inside = Stmt::Label(tree::Label::Block(1));
    let cases: Vec<(&str, Vec<Stmt>, usize)> = vec![
        (
            "the index is read in the body",
            vec![counted(vec![touch(5)])],
            1,
        ),
        (
            "the index is read after the loop",
            vec![counted(Vec::new()), touch(5)],
            1,
        ),
        (
            "a loop inside steps the same index",
            vec![counted(vec![counted(Vec::new())])],
            2,
        ),
        (
            "the array is assigned in the body",
            vec![counted(vec![set(4, local(0))])],
            1,
        ),
        (
            "the item is assigned again",
            vec![counted(vec![set(6, int(7))])],
            1,
        ),
        (
            "the address of the index is taken",
            vec![address(5), counted(Vec::new())],
            1,
        ),
        (
            "the address of the array is taken and the body makes a call",
            vec![address(4), counted(vec![touch(0)])],
            1,
        ),
        (
            "the address of the item is taken",
            vec![counted(vec![address(6)])],
            1,
        ),
        ("a label stands in the body", vec![counted(vec![inside])], 1),
    ];
    for (why, was, refused) in cases {
        let (is, found) = read(was.clone(), &Names::default());
        assert_eq!(is, was, "{why}");
        let none = Found {
            refused,
            ..Found::default()
        };
        assert_eq!(found, none, "{why}");
    }
}

#[test]
fn a_loop_of_another_shape_is_no_foreach() {
    let first = eval(call(vec![local(0)]));
    let cases: Vec<(&str, Stmt, usize)> = vec![
        (
            "the test lets the index reach the length",
            looped(length_test(BinOp::Le, 4), vec![element("array_get_u64", 4)]),
            0,
        ),
        (
            "the element is read from another array",
            looped(length_test(BinOp::Lt, 4), vec![element("array_get_u64", 3)]),
            1,
        ),
        (
            "the element's address is taken in place of its value",
            looped(
                length_test(BinOp::Lt, 4),
                vec![element("array_element_address", 4)],
            ),
            1,
        ),
        (
            "something runs before the element is read",
            looped(
                length_test(BinOp::Lt, 4),
                vec![first, element("array_get_u64", 4)],
            ),
            1,
        ),
    ];
    for (why, was, refused) in cases {
        let (is, found) = read(vec![was.clone()], &Names::default());
        assert_eq!(is, vec![was], "{why}");
        assert_eq!((found.arrays, found.refused), (0, refused), "{why}");
    }
}

#[test]
fn the_address_of_a_wider_local_below_reaches_the_index() {
    let address = eval(call(vec![Expr::AddrOf {
        var: Var::Local(3),
        cell: 0,
    }]));
    let was = vec![address, counted(Vec::new())];
    // One cell, as far as anything says: its address reaches no further than `local_3`.
    assert_eq!(read(was.clone(), &Names::default()).1.arrays, 1);
    // Three cells: `local_3`, the array and the index.
    let (is, found) = read_wide(was.clone());
    assert_eq!((is, found.arrays, found.refused), (was, 0, 1));
}

#[test]
fn an_item_whose_cells_hold_the_index_is_no_item() {
    let item = set(3, op("array_get_value", vec![local(4), local(5)]));
    let was = vec![looped(length_test(BinOp::Lt, 4), vec![item])];
    assert_eq!(read(was.clone(), &Names::default()).1.arrays, 1);
    let (is, found) = read_wide(was.clone());
    assert_eq!((is, found.arrays, found.refused), (was, 0, 1));
}

#[test]
fn loops_that_take_one_slot_for_their_index_in_turn_are_each_read() {
    let was = vec![counted(Vec::new()), counted(Vec::new())];
    let (is, found) = read(was.clone(), &Names::default());
    assert_eq!((found.arrays, found.refused), (2, 0));
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}

/// A list that names slot 0 of the three made-up types as the members a `foreach` is made of.
fn members() -> Names {
    let mut names = Names::default();
    let listed = [
        (ENUMERABLE, "GetEnumerator"),
        (ENUMERATOR, "MoveNext"),
        (ITEMS, "get_Current"),
        (DISPOSABLE, "Dispose"),
    ];
    for (ty, name) in listed {
        names.methods.insert((TypeKey(ty), 0), name.to_owned());
    }
    names
}

/// `local_e = local_c.GetEnumerator(); while ((uint)local_e.MoveNext() != 0) { local_x = local_e.Current; <rest> }`, then `after`.
fn enumerated_as(c: u32, e: u32, x: u32, rest: Vec<Stmt>, after: Vec<Stmt>) -> Vec<Stmt> {
    let mut body = vec![set(x, slot(ITEMS, vec![local(e)]))];
    body.extend(rest);
    let moved = Expr::cast(Scalar::U32, slot(ENUMERATOR, vec![local(e)]));
    let mut all = vec![
        set(e, slot(ENUMERABLE, vec![local(c)])),
        Stmt::While {
            cond: Expr::binary(BinOp::Ne, moved, int(0)),
            body,
        },
    ];
    all.extend(after);
    all
}

/// [`enumerated_as`] over `local_0`, with the enumerator in `local_1` and the item in `local_2`.
fn enumerated(rest: Vec<Stmt>, after: Vec<Stmt>) -> Vec<Stmt> {
    enumerated_as(0, 1, 2, rest, after)
}

/// `local_n.Dispose();`.
fn dispose(n: u32) -> Stmt {
    eval(slot(DISPOSABLE, vec![local(n)]))
}

/// `if (local_n != null) { local_n.Dispose(); }`.
fn dispose_unless_null(n: u32) -> Stmt {
    Stmt::If {
        cond: Expr::binary(BinOp::Ne, Expr::cast(Scalar::U64, local(n)), int(0)),
        then: vec![dispose(n)],
        els: Vec::new(),
    }
}

/// `if (local_2) { <before> return <value>; }`.
fn leave(before: Vec<Stmt>, value: Expr) -> Stmt {
    let mut then = before;
    then.push(Stmt::Return(Some(value)));
    Stmt::If {
        cond: local(2),
        then,
        els: Vec::new(),
    }
}

#[test]
fn an_enumerator_loop_disposed_at_every_way_out_is_a_foreach_that_hides_the_disposal() {
    let rest = vec![
        leave(vec![dispose_unless_null(1)], int(1)),
        eval(call(vec![local(2)])),
    ];
    let mut was = enumerated(rest, vec![dispose_unless_null(1)]);
    was.push(Stmt::Return(Some(int(0))));
    let (is, found) = read(was.clone(), &members());
    let one = Found {
        enumerators: 1,
        ..Found::default()
    };
    assert_eq!(found, one);
    let [Stmt::Sugar(made), Stmt::Return(_)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Foreach {
        setup,
        item,
        body,
        step,
        after,
        ..
    } = &**made
    else {
        panic!("{made:?}")
    };
    assert_eq!(setup, &vec![assign(1, slot(ENUMERABLE, vec![local(0)]))]);
    assert_eq!(**item, assign(2, slot(ITEMS, vec![local(1)])));
    assert!(step.is_none());
    // Each disposal is still there and written as nothing: after the loop, and before the `return` in it.
    assert_eq!(after, &vec![hidden(vec![dispose_unless_null(1)])]);
    let [Stmt::If { then, .. }, _] = body.as_slice() else {
        panic!("{body:?}")
    };
    let left = vec![
        hidden(vec![dispose_unless_null(1)]),
        Stmt::Return(Some(int(1))),
    ];
    assert_eq!(then, &left);
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));

    // The disposal alone, and no disposal at all, are the two other ways every way out may agree.
    let alone = enumerated(Vec::new(), vec![dispose(1)]);
    assert_eq!(read(alone, &members()).1, one);
    let never = enumerated(vec![leave(Vec::new(), int(1))], Vec::new());
    assert_eq!(read(never, &members()).1, one);
}

#[test]
fn an_enumerator_loop_stays_a_loop_where_a_way_out_or_another_use_says_otherwise() {
    let guard = || dispose_unless_null(1);
    let touch = |n: u32| eval(call(vec![local(n)]));
    let field = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(local(0)),
        offset: 8,
    };
    let away = Stmt::Goto(tree::Label::Block(9));
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a return skips the disposal",
            enumerated(vec![leave(Vec::new(), int(1))], vec![guard()]),
        ),
        (
            "a return disposes in another shape",
            enumerated(vec![leave(vec![dispose(1)], int(1))], vec![guard()]),
        ),
        (
            "a return hands back what the disposal may change",
            enumerated(vec![leave(vec![guard()], field)], vec![guard()]),
        ),
        (
            "only a return disposes",
            enumerated(vec![leave(vec![guard()], int(1))], Vec::new()),
        ),
        (
            "the enumerator is used in the body",
            enumerated(vec![touch(1)], vec![guard()]),
        ),
        (
            "the enumerator is used after the loop",
            enumerated(Vec::new(), vec![guard(), touch(1)]),
        ),
        (
            "a jump leaves the body",
            enumerated(vec![away], vec![guard()]),
        ),
        (
            "the item is assigned again",
            enumerated(vec![set(2, int(0))], vec![guard()]),
        ),
    ];
    for (why, was) in cases {
        let (is, found) = read_loops(was.clone(), &members());
        assert_eq!(is, was, "{why}");
        let none = Found {
            refused: 1,
            ..Found::default()
        };
        assert_eq!(found, none, "{why}");
    }
}

#[test]
fn no_enumerator_loop_is_read_without_the_names_of_its_members() {
    let was = enumerated(Vec::new(), vec![dispose_unless_null(1)]);
    // No list: nothing says which slot advances an enumerator.
    let unread = (was.clone(), Found::default());
    assert_eq!(read(was.clone(), &Names::default()), unread);
    // The enumerator is taken by a call no list calls `GetEnumerator`.
    let mut unnamed = members();
    unnamed.methods.remove(&(TypeKey(ENUMERABLE), 0));
    let refused = Found {
        refused: 1,
        ..Found::default()
    };
    assert_eq!(read_loops(was.clone(), &unnamed), (was.clone(), refused));
    // ⚠ The item is taken by a call no list calls `Current`: read as a `foreach`, the call would not show.
    let mut unnamed = members();
    unnamed.methods.remove(&(TypeKey(ITEMS), 0));
    assert_eq!(read_loops(was.clone(), &unnamed), (was, refused));
}

#[test]
fn a_member_is_named_on_the_generic_type_its_type_is_made_from() {
    const DEFINITION: u64 = 0x9999_AAAA_BBBB_CCCC;
    let mut names = members();
    let name = names.methods.remove(&(TypeKey(ENUMERABLE), 0));
    names
        .methods
        .extend(name.map(|name| ((TypeKey(DEFINITION), 0), name)));
    let made = TypeFacts {
        generic: Some((TypeKey(DEFINITION), Vec::new())),
        ..TypeFacts::default()
    };
    let project = Project {
        types: BTreeMap::from([(TypeKey(ENUMERABLE), made)]),
        ..Project::default()
    };
    let mut f = function(enumerated(Vec::new(), vec![dispose(1)]));
    let found = rewrite(&mut f, &Context::new(&names, &project, Options::default()));
    assert_eq!((found.enumerators, found.refused), (1, 0));
}

#[test]
fn a_disposal_through_a_cast_of_the_enumerator_is_hidden_with_its_cast() {
    let cast = set(3, op("try_cast<type_80000F02>", vec![local(1)]));
    let was = enumerated(Vec::new(), vec![cast, dispose_unless_null(3)]);
    let (is, found) = read(was.clone(), &members());
    assert_eq!((found.enumerators, found.refused), (1, 0));
    let [Stmt::Sugar(made)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Foreach { after, .. } = &**made else {
        panic!("{made:?}")
    };
    assert_eq!(after, &vec![hidden(was[2..].to_vec())]);
    // The cast is the disposal's own: read anywhere else, it is a variable of the function's.
    let mut read_again = was;
    read_again.push(eval(call(vec![local(3)])));
    assert_eq!(read(read_again.clone(), &members()).0, read_again);
}

#[test]
fn a_loop_inside_another_is_read_past_the_disposal_hidden_for_the_one_around_it() {
    // Leaving both loops runs the disposal of the one inside, then that of the one around it.
    let out = Stmt::If {
        cond: local(5),
        then: vec![dispose(4), dispose(1), Stmt::Return(Some(int(1)))],
        els: Vec::new(),
    };
    let inner = enumerated_as(2, 4, 5, vec![out], vec![dispose(4)]);
    let mut was = enumerated(inner, vec![dispose(1)]);
    was.push(Stmt::Return(Some(int(0))));
    let (is, found) = read(was.clone(), &members());
    let two = Found {
        enumerators: 2,
        ..Found::default()
    };
    assert_eq!(found, two);
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}

/// `local_4[local_5]` read as an `int`, where the loop of [`looped`] reads it in place.
fn in_place() -> Expr {
    op("array_get_i32", vec![local(4), local(5)])
}

/// `local_4[local_5]` read as a value of the cells `returns` states.
fn wide(returns: Option<u32>) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: "array_get_value".to_owned(),
            ty: None,
            wrapper: Some("array_get_value".to_owned()),
        },
        args: vec![local(4), local(5)],
        returns,
    }
}

/// `local_4 = p0; for (local_5 = 0; local_5 < local_4.Length; local_5++) { <body> }`.
fn over_parameter(body: Vec<Stmt>) -> Vec<Stmt> {
    vec![
        set(4, Expr::Var(Var::Param(0))),
        looped(length_test(BinOp::Lt, 4), body),
    ]
}

/// The item the first loop of [`function`] that kept none is given.
const ITEM: Var = Var::Value { at: 0x10, index: 0 };

/// Whether two bodies of [`function`] do the same on made-up inputs, as far as each run goes.
fn run_alike(was: tree::Function, is: tree::Function) -> Result<(), String> {
    let was = cfg::lower(was).map_err(|e| e.to_string())?;
    let is = cfg::lower(is).map_err(|e| e.to_string())?;
    for seed in 0..8 {
        exec::same(&exec::run(&was, seed), &exec::run(&is, seed))?;
    }
    Ok(())
}

#[test]
fn a_loop_that_reads_its_element_once_in_place_is_a_foreach_over_an_item_made_for_it() {
    // `if (func(local_4[local_5]) != 0) { return 1; }`
    let test = |element: Expr| Stmt::If {
        cond: Expr::binary(BinOp::Ne, call(vec![element]), int(0)),
        then: vec![Stmt::Return(Some(int(1)))],
        els: Vec::new(),
    };
    let was = over_parameter(vec![test(in_place())]);
    let mut f = function(was.clone());
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let found = rewrite(&mut f, &cx);
    assert_eq!((found.arrays, found.folded, found.refused), (1, 1, 0));
    let [Stmt::Simple(_), Stmt::Sugar(made)] = f.body.as_slice() else {
        panic!("{:?}", f.body)
    };
    let Sugar::Foreach {
        setup,
        item,
        body,
        step,
        after,
        ..
    } = &**made
    else {
        panic!("{made:?}")
    };
    assert_eq!(setup, &vec![assign(5, int(0))]);
    // The read is the taking of the item, and the item stands where the read stood.
    let taken = ir::Stmt::Assign {
        target: ITEM,
        value: in_place(),
    };
    assert_eq!(**item, taken);
    assert_eq!(body, &vec![test(Expr::Var(ITEM))]);
    assert!(step.is_some() && after.is_empty());
    // The item is a variable of the function's: one cell, of the type the element is read at,
    // under a name of its own, which a temporary's slot name would not be.
    let info = VarInfo {
        width: Width::Cell,
        kind: Kind::I32,
        name: Some("item".to_owned()),
        single: false,
    };
    assert_eq!(f.head.vars.get(&ITEM), Some(&info));
    assert_eq!(run_alike(function(was.clone()), f), Ok(()));

    // With the name off the loop stays as it was, and no variable is made.
    let mut f = function(was.clone());
    let off = Options::default().without(Pass::Foreach);
    rewrite(&mut f, &Context::new(&names, Project::empty(), off));
    assert_eq!((f.body, f.head.vars.len()), (was, 0));
}

#[test]
fn the_element_is_taken_first_where_only_what_changes_nothing_comes_before_its_read() {
    let used = |element: Expr| eval(call(vec![element]));
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a variable is set before the read",
            vec![set(7, int(3)), used(in_place())],
        ),
        (
            "the read is under a test of a variable, and the other way calls",
            vec![Stmt::If {
                cond: local(0),
                then: vec![used(in_place())],
                els: vec![used(local(1))],
            }],
        ),
        (
            "the read is in a case of a switch on a variable",
            vec![Stmt::Switch {
                value: local(0),
                cases: vec![
                    tree::Case {
                        labels: vec![Some(1)],
                        body: vec![Stmt::Break],
                    },
                    tree::Case {
                        labels: vec![None],
                        body: vec![used(in_place()), Stmt::Break],
                    },
                ],
            }],
        ),
        (
            "the read is the value a store writes",
            vec![Stmt::Simple(ir::Stmt::Store {
                kind: StoreKind::I32,
                addr: local(0),
                offset: 8,
                value: in_place(),
            })],
        ),
    ];
    for (why, body) in cases {
        let was = over_parameter(body);
        let mut f = function(was.clone());
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), Options::default());
        let found = rewrite(&mut f, &cx);
        assert_eq!((found.folded, found.refused), (1, 0), "{why}");
        assert!(f.head.vars.contains_key(&ITEM), "{why}");
        assert_eq!(run_alike(function(was), f), Ok(()), "{why}");
    }
}

#[test]
fn an_element_read_in_place_is_no_item_where_it_may_have_changed_or_is_read_otherwise() {
    let used = |element: Expr| eval(call(vec![element]));
    let store = Stmt::Simple(ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: local(0),
        offset: 8,
        value: int(1),
    });
    let unstated = wide(None);
    let at_two_types = op("array_get_u32", vec![local(4), local(5)]);
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a call is made before the read, in the same statement",
            vec![eval(call(vec![call(vec![local(0)]), in_place()]))],
        ),
        (
            "a call is made before the read, in the statement before",
            vec![used(local(0)), used(in_place())],
        ),
        (
            "a store is made before the read",
            vec![store, used(in_place())],
        ),
        (
            "the read is under a test that calls",
            vec![Stmt::If {
                cond: call(vec![local(0)]),
                then: vec![used(in_place())],
                els: Vec::new(),
            }],
        ),
        (
            "the way before the read calls under a test",
            vec![
                Stmt::If {
                    cond: local(0),
                    then: vec![used(local(1))],
                    els: Vec::new(),
                },
                used(in_place()),
            ],
        ),
        (
            "the read is in a loop of the body",
            vec![Stmt::While {
                cond: local(0),
                body: vec![used(in_place())],
            }],
        ),
        (
            "the element is read again after a call",
            vec![used(in_place()), used(in_place())],
        ),
        (
            "the element is read again after a store",
            vec![
                Stmt::Simple(ir::Stmt::Store {
                    kind: StoreKind::I32,
                    addr: local(0),
                    offset: 8,
                    value: in_place(),
                }),
                set(7, in_place()),
            ],
        ),
        (
            "the element is read at two types",
            vec![eval(call(vec![in_place(), at_two_types]))],
        ),
        (
            "the element is read again in a loop that calls",
            vec![Stmt::If {
                cond: Expr::binary(BinOp::Gt, in_place(), int(0)),
                then: vec![Stmt::While {
                    cond: local(0),
                    body: vec![used(in_place())],
                }],
                els: Vec::new(),
            }],
        ),
        (
            "the cells of the element are not stated",
            vec![used(unstated)],
        ),
        (
            "the index is read as itself",
            vec![used(Expr::cast(Scalar::I32, local(5)))],
        ),
    ];
    for (why, body) in cases {
        let was = over_parameter(body);
        let mut f = function(was.clone());
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), Options::default());
        let found = rewrite(&mut f, &cx);
        assert_eq!((found.arrays, found.refused), (0, 1), "{why}");
        assert_eq!((f.body, f.head.vars.len()), (was, 0), "{why}");
    }
}

#[test]
fn each_loop_is_given_an_item_no_variable_of_the_function_has() {
    let round = || {
        looped(
            length_test(BinOp::Lt, 4),
            vec![eval(call(vec![in_place()]))],
        )
    };
    let mut f = function(vec![round(), round()]);
    // A value the function already has at its own offset.
    f.head.vars.insert(ITEM, VarInfo::new(Width::Cell));
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let found = rewrite(&mut f, &cx);
    assert_eq!((found.arrays, found.folded, found.refused), (2, 2, 0));
    let items: Vec<Var> = f.head.vars.keys().copied().collect();
    let value = |index: u32| Var::Value { at: 0x10, index };
    assert_eq!(items, vec![value(0), value(1), value(2)]);
    let taken: Vec<Var> = (f.body.iter())
        .filter_map(|stmt| match stmt {
            Stmt::Sugar(sugar) => match &**sugar {
                Sugar::Foreach { item, .. } => item.targets().first().copied(),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(taken, vec![value(1), value(2)]);
}

#[test]
fn an_item_made_for_an_element_takes_the_kind_the_array_states() {
    let mut f = function(over_parameter(vec![eval(call(vec![in_place()]))]));
    let floats = VarInfo {
        width: Width::Cell,
        kind: Kind::Array(Box::new(Kind::F32)),
        name: None,
        single: false,
    };
    f.head.vars.insert(Var::Local(4), floats);
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    assert_eq!(rewrite(&mut f, &cx).folded, 1);
    assert_eq!(
        f.head.vars.get(&ITEM).map(|info| &info.kind),
        Some(&Kind::F32)
    );
}

/// `body` with every `read` in it made a read of [`ITEM`].
fn read_as_item(mut body: Vec<Stmt>, read: &Expr) -> Vec<Stmt> {
    let mut work: Vec<&mut Vec<Stmt>> = vec![&mut body];
    while let Some(list) = work.pop() {
        for stmt in list {
            tree::own_exprs_mut(stmt, &mut |top| {
                top.walk_mut(&mut |e| {
                    if *e == *read {
                        *e = Expr::Var(ITEM);
                    }
                });
            });
            work.extend(tree::nested_mut(stmt));
        }
    }
    body
}

#[test]
fn a_loop_that_reads_its_element_again_before_anything_may_change_it_takes_it_once() {
    // `if (local_4[local_5] > local_7) { local_7 = local_4[local_5]; }`
    let most = Stmt::If {
        cond: Expr::binary(BinOp::Gt, in_place(), local(7)),
        then: vec![set(7, in_place())],
        els: Vec::new(),
    };
    // `func(local_4[local_5], local_4[local_5]);`: both reads are made before the call.
    let both = eval(call(vec![in_place(), in_place()]));
    // A loop of the body that changes nothing reads what was read before it.
    let calm = Stmt::DoWhile {
        body: vec![set(7, Expr::binary(BinOp::Add, local(7), in_place()))],
        cond: int(0),
    };
    let bodies = vec![
        vec![most.clone()],
        vec![both.clone()],
        vec![most, calm, both],
    ];
    for body in bodies {
        let was = over_parameter(body.clone());
        let mut f = function(was.clone());
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), Options::default());
        let found = rewrite(&mut f, &cx);
        assert_eq!(
            (found.arrays, found.folded, found.refused),
            (1, 1, 0),
            "{body:?}"
        );
        let [Stmt::Simple(_), Stmt::Sugar(made)] = f.body.as_slice() else {
            panic!("{:?}", f.body)
        };
        let Sugar::Foreach {
            item, body: made, ..
        } = &**made
        else {
            panic!("{made:?}")
        };
        let taken = ir::Stmt::Assign {
            target: ITEM,
            value: in_place(),
        };
        assert_eq!(**item, taken);
        assert_eq!(made, &read_as_item(body.clone(), &in_place()), "{body:?}");
        assert_eq!(run_alike(function(was), f), Ok(()), "{body:?}");
    }
}

#[test]
fn an_element_of_the_cells_its_read_states_is_an_item_of_those_cells() {
    let was = over_parameter(vec![eval(call(vec![wide(Some(2))]))]);
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let mut f = function(was.clone());
    let found = rewrite(&mut f, &cx);
    assert_eq!((found.arrays, found.folded, found.refused), (1, 1, 0));
    let info = VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(2),
        name: Some("item".to_owned()),
        single: false,
    };
    assert_eq!(f.head.vars.get(&ITEM), Some(&info));
    assert_eq!(run_alike(function(was.clone()), f), Ok(()));

    // An array whose elements are handles makes a handle of the item; one of single cells says nothing of two.
    for (of, kind) in [
        (Kind::Handle, Kind::Handle),
        (Kind::I32, Kind::Aggregate(2)),
    ] {
        let mut f = function(was.clone());
        let array = VarInfo {
            width: Width::Cell,
            kind: Kind::Array(Box::new(of)),
            name: None,
            single: false,
        };
        f.head.vars.insert(Var::Local(4), array);
        assert_eq!(rewrite(&mut f, &cx).folded, 1);
        assert_eq!(f.head.vars.get(&ITEM).map(|info| &info.kind), Some(&kind));
    }
}

// Made up: the type whose field holds the array.
const OWNER: u64 = 0x1357_9BDF_2468_ACE0;

/// `p0->f_20`, the field the array is held in.
fn field() -> Expr {
    Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(Expr::Var(Var::Param(0))),
        offset: 0x20,
    }
}

/// `p0->f_20[local_5]` read as an `int`.
fn of_field() -> Expr {
    op("array_get_i32", vec![field(), local(5)])
}

/// `for (local_5 = 0; local_5 < p0->f_20.Length; local_5++) { <body> }`, the field read as one in the test alone.
fn over_field(body: Vec<Stmt>) -> Vec<Stmt> {
    let read = Expr::sugar(
        ExprSugar::Field {
            owner: TypeKey(OWNER),
        },
        field(),
    );
    let length = Expr::cast(Scalar::I32, op("array_length", vec![read]));
    let cond = Expr::binary(BinOp::Lt, Expr::cast(Scalar::I32, local(5)), length);
    vec![looped(cond, body)]
}

#[test]
fn a_loop_over_an_array_in_a_field_is_a_foreach_where_nothing_in_it_stores_or_calls() {
    let sum = |element: Expr| set(7, Expr::binary(BinOp::Add, local(7), element));
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());

    // Read in place: an item is made for it.
    let was = over_field(vec![sum(of_field())]);
    let mut f = function(was.clone());
    let found = rewrite(&mut f, &cx);
    assert_eq!((found.arrays, found.folded, found.refused), (1, 1, 0));
    let [Stmt::Sugar(made)] = f.body.as_slice() else {
        panic!("{:?}", f.body)
    };
    let Sugar::Foreach { item, body, .. } = &**made else {
        panic!("{made:?}")
    };
    let taken = ir::Stmt::Assign {
        target: ITEM,
        value: of_field(),
    };
    assert_eq!(**item, taken);
    let summed = vec![sum(of_field())];
    assert_eq!(body, &read_as_item(summed, &of_field()));
    assert_eq!(run_alike(function(was), f), Ok(()));

    // Taken first into a variable of the function's.
    let was = over_field(vec![set(6, of_field()), sum(local(6))]);
    let (is, found) = read(was.clone(), &names);
    assert_eq!((found.arrays, found.folded, found.refused), (1, 0, 0));
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));

    // ⚠ Each round's test reads the field again: anything that may store to it keeps the loop.
    let store = Stmt::Simple(ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: local(0),
        offset: 8,
        value: int(1),
    });
    let again = Stmt::Simple(ir::Stmt::Assign {
        target: Var::Param(0),
        value: local(1),
    });
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a call is made after the read",
            vec![sum(of_field()), eval(call(Vec::new()))],
        ),
        (
            "the item is handed to a call",
            vec![set(6, of_field()), eval(call(vec![local(6)]))],
        ),
        (
            "a store is made after the read",
            vec![sum(of_field()), store],
        ),
        (
            "what the field is read from is assigned",
            vec![set(6, of_field()), again],
        ),
    ];
    for (why, body) in cases {
        let was = over_field(body);
        let (is, found) = read(was.clone(), &names);
        assert_eq!(is, was, "{why}");
        assert_eq!((found.arrays, found.refused), (0, 1), "{why}");
    }
}

#[test]
fn an_item_only_ever_read_through_its_address_is_still_the_item_a_foreach_takes() {
    let item = Expr::AddrOf {
        var: Var::Local(6),
        cell: 0,
    };
    let through = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(item.clone()),
        offset: 4,
    };
    let was = vec![counted(vec![eval(call(vec![through]))])];
    let (is, found) = read(was.clone(), &Names::default());
    assert_eq!((found.arrays, found.refused), (1, 0));
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));

    // Handed on, the address may be written through: the item is no longer the element.
    let was = vec![counted(vec![eval(call(vec![item]))])];
    let (is, found) = read(was.clone(), &Names::default());
    assert_eq!(is, was);
    assert_eq!((found.arrays, found.refused), (0, 1));
}

/// A global cell of the program's statics, which a call may change behind its name.
fn global(offset: u32) -> Var {
    Var::Global {
        program: 1,
        offset,
        statics: true,
    }
}

/// `for (local_5 = 0; local_5 < <array>.Length; local_5++) { local_6 = <array>[local_5]; <rest> }`.
fn counted_over(array: Expr, rest: Vec<Stmt>) -> Stmt {
    let length = Expr::cast(Scalar::I32, op("array_length", vec![array.clone()]));
    let cond = Expr::binary(BinOp::Lt, Expr::cast(Scalar::I32, local(5)), length);
    let mut body = vec![set(6, op("array_get_u64", vec![array, local(5)]))];
    body.extend(rest);
    looped(cond, body)
}

/// `local_7 = local_7 + local_6;`: what writes no memory and calls nothing.
fn summed() -> Stmt {
    set(7, Expr::binary(BinOp::Add, local(7), local(6)))
}

#[test]
fn an_array_something_else_may_change_is_gone_through_where_the_loop_runs_nothing_that_may() {
    let names = Names::default();
    let over_global = |rest: Vec<Stmt>| vec![counted_over(Expr::Var(global(0x40)), rest)];

    // A global array, and a local one whose address a call was handed before the loop.
    let handed = eval(call(vec![Expr::AddrOf {
        var: Var::Local(4),
        cell: 0,
    }]));
    let quiet: Vec<Vec<Stmt>> = vec![
        over_global(vec![summed()]),
        vec![handed.clone(), counted(vec![summed()])],
    ];
    for was in quiet {
        let (is, found) = read(was.clone(), &names);
        assert_eq!((found.arrays, found.refused), (1, 0), "{was:?}");
        assert!(
            is.iter().any(|stmt| matches!(stmt, Stmt::Sugar(_))),
            "{is:?}"
        );
        assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
    }

    // ⚠ The test reads the array again each round: a call, a store or a write to another global
    // there may change it, and the loop stays.
    let store = Stmt::Simple(ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: local(0),
        offset: 8,
        value: int(1),
    });
    let other = Stmt::Simple(ir::Stmt::Assign {
        target: global(0x48),
        value: local(6),
    });
    let loud: Vec<(&str, Vec<Stmt>)> = vec![
        ("a call", over_global(vec![eval(call(Vec::new()))])),
        ("a store", over_global(vec![store])),
        ("another global assigned", over_global(vec![other])),
        (
            "a call after the address was handed",
            vec![handed, counted(vec![eval(call(Vec::new()))])],
        ),
    ];
    for (why, was) in loud {
        let (is, found) = read(was.clone(), &names);
        assert_eq!(is, was, "{why}");
        assert_eq!((found.arrays, found.refused), (0, 1), "{why}");
    }
}

/// `for (local_5 = <from>; local_5 < 10; local_5++) { <body> }`: a loop over no array.
fn upto(from: Expr, body: Vec<Stmt>) -> Stmt {
    let up = Expr::binary(BinOp::Add, Expr::cast(Scalar::I32, local(5)), int(1));
    let cond = Expr::binary(BinOp::Lt, Expr::cast(Scalar::I32, local(5)), int(10));
    Stmt::For {
        init: Some(Box::new(assign(5, from))),
        cond: Some(cond),
        step: Some(Box::new(assign(5, up))),
        body,
    }
}

#[test]
fn an_index_another_loop_sets_in_its_own_head_is_still_this_loops_alone() {
    let names = Names::default();
    let touch = eval(call(vec![local(5)]));
    let shared: Vec<Vec<Stmt>> = vec![
        vec![counted(Vec::new()), upto(int(0), vec![touch.clone()])],
        vec![upto(int(0), vec![touch.clone()]), counted(Vec::new())],
    ];
    for was in shared {
        let (is, found) = read(was.clone(), &names);
        assert_eq!((found.arrays, found.refused), (1, 0), "{was:?}");
        assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
    }

    // ⚠ Each of these reads what the loop left in the index, or may.
    let inside = Stmt::Label(tree::Label::Block(1));
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "the other loop's head reads the index",
            vec![counted(Vec::new()), upto(local(5), vec![touch.clone()])],
        ),
        (
            "the other loop holds a label",
            vec![counted(Vec::new()), upto(int(0), vec![inside])],
        ),
        (
            "the index is read after the other loop",
            vec![counted(Vec::new()), upto(int(0), Vec::new()), touch.clone()],
        ),
        (
            "the other loop holds this one, and steps the index it left",
            vec![upto(int(0), vec![counted(Vec::new())])],
        ),
    ];
    for (why, was) in cases {
        let (is, found) = read(was.clone(), &names);
        assert_eq!(is, was, "{why}");
        assert_eq!((found.arrays, found.refused), (0, 1), "{why}");
    }
}

#[test]
fn an_index_a_wider_variable_names_by_its_cells_is_no_loops_alone() {
    // `local_3`, of three cells, holds the index's: reading it reads what the loop left there.
    let was = vec![counted(Vec::new()), eval(call(vec![local(3)]))];
    let (is, found) = read(was.clone(), &Names::default());
    assert_eq!((found.arrays, found.refused), (1, 0));
    assert_ne!(is, was);
    let (is, found) = read_wide(was.clone());
    assert_eq!((is, found.arrays, found.refused), (was, 0, 1));
}

/// [`read`] where each `(n, cells)` makes `local_n` a value of that many cells.
fn read_cells(body: Vec<Stmt>, wide: &[(u32, u32)], names: &Names) -> (Vec<Stmt>, Found) {
    let mut f = function(body);
    for (n, cells) in wide {
        let info = VarInfo {
            width: Width::Cells,
            kind: Kind::Aggregate(*cells),
            name: None,
            single: false,
        };
        f.head.vars.insert(Var::Local(*n), info);
    }
    let cx = Context::new(names, Project::empty(), Options::default());
    let found = rewrite(&mut f, &cx);
    (f.body, found)
}

#[test]
fn an_array_or_an_item_a_wider_variable_names_by_its_cells_is_no_foreach() {
    let names = Names::default();
    // Read after the loop: `local_3`, of two cells, holds the array's; `local_6`, of two, the item's.
    let over_array = vec![counted(Vec::new()), eval(call(vec![local(3)]))];
    let item = set(7, op("array_get_u64", vec![local(4), local(5)]));
    let used = eval(call(vec![local(7)]));
    let over_item = vec![
        looped(length_test(BinOp::Lt, 4), vec![item, used]),
        eval(call(vec![local(6)])),
    ];
    for (was, wide) in [(over_array, (3, 2)), (over_item, (6, 2))] {
        let (is, found) = read(was.clone(), &names);
        assert_eq!((found.arrays, found.refused), (1, 0), "{was:?}");
        assert_ne!(is, was);
        let (is, found) = read_cells(was.clone(), &[wide], &names);
        assert_eq!((is, found.arrays, found.refused), (was, 0, 1));
    }
}

#[test]
fn an_enumerator_or_an_item_a_wider_variable_names_by_its_cells_is_no_foreach() {
    // The enumerator, the item, and a variable of two cells read after the loop that holds the
    // item's cell (`local_2`) or the enumerator's (`local_4`).
    for (e, x, wide) in [(1, 3, 2), (5, 2, 4)] {
        let was = enumerated_as(0, e, x, Vec::new(), vec![eval(call(vec![local(wide)]))]);
        let (is, found) = read(was.clone(), &members());
        assert_eq!((found.enumerators, found.refused), (1, 0), "{was:?}");
        assert_ne!(is, was);
        let (is, found) = read_cells(was.clone(), &[(wide, 2)], &members());
        assert_eq!((is, found.enumerators, found.refused), (was, 0, 1));
    }
}
