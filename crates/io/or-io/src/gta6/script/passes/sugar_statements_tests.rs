use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Const, Returns, VarInfo, Width};
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

/// [`read`] where `local_3` is known to be a value of three cells.
fn read_wide(body: Vec<Stmt>) -> (Vec<Stmt>, Found) {
    let mut f = function(body);
    let wide = VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(3),
        name: None,
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
        enumerators: 0,
        refused: 0,
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
            "the address of the array is taken",
            vec![address(4), counted(Vec::new())],
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
            arrays: 0,
            enumerators: 0,
            refused,
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
        arrays: 0,
        enumerators: 1,
        refused: 0,
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
        let (is, found) = read(was.clone(), &members());
        assert_eq!(is, was, "{why}");
        let none = Found {
            arrays: 0,
            enumerators: 0,
            refused: 1,
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
        arrays: 0,
        enumerators: 0,
        refused: 1,
    };
    assert_eq!(read(was.clone(), &unnamed), (was.clone(), refused));
    // ⚠ The item is taken by a call no list calls `Current`: read as a `foreach`, the call would not show.
    let mut unnamed = members();
    unnamed.methods.remove(&(TypeKey(ITEMS), 0));
    assert_eq!(read(was.clone(), &unnamed), (was, refused));
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
        arrays: 0,
        enumerators: 2,
        refused: 0,
    };
    assert_eq!(found, two);
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}
