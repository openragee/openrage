use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{Const, StoreKind, VarInfo, Width};
use crate::gta6::script::project::{Member, Param, Signature};

/// An enum of a script, of four bytes and no constant below zero.
const ENUM: &str = "type_1111222233334444";

/// An enum of a script, of one byte.
const SMALL: &str = "type_99990000AAAABBBB";

/// A struct of a script.
const STRUCT_TYPE: &str = "type_5555666677778888";

fn int(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn boxed_as(ty: &str, value: Expr) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: format!("box<{ty}>"),
            ty: None,
            wrapper: Some(format!("box_{ty}_")),
        },
        args: vec![value],
        returns: Some(1),
    }
}

/// The box read as the conversion it is.
fn converted(ty: &str, value: Expr) -> Expr {
    Expr::sugar(ExprSugar::Convert, boxed_as(ty, value))
}

fn called(callee: Callee, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee,
        args,
        returns: Some(1),
    }
}

fn own(at: u32) -> Callee {
    Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    }
}

fn builtin(path: &str) -> Callee {
    Callee::Builtin {
        path: path.to_owned(),
        ty: 0,
        method: 0,
    }
}

fn types() -> Project {
    let mut project = Project::default();
    let facts = |flags: u32, byte_size: u32| TypeFacts {
        flags,
        byte_size,
        constants: vec![1, 2],
        ..TypeFacts::default()
    };
    let key = |name: &str| TypeKey::of_name(name).unwrap_or(TypeKey(0));
    project.types.insert(key(ENUM), facts(0x08, 4));
    project.types.insert(key(SMALL), facts(0x08, 1));
    project.types.insert(key(STRUCT_TYPE), facts(0x04, 16));
    // The function the tests call takes two objects.
    let object = Param {
        kind: Kind::Object("System.Object".to_owned()),
        cells: 1,
    };
    let signature = Signature {
        params: vec![object; 2],
        ..Signature::default()
    };
    project
        .signatures
        .insert(FunctionKey::new(1, 0x200), signature);
    project
}

/// A function of one cell's result whose local 0 is a float, local 1 the struct and local 2 a truth.
fn frame(returns: Returns) -> Head {
    let info = |width: Width, kind: Kind| VarInfo {
        width,
        kind,
        name: None,
        single: false,
    };
    let kinds = [
        (Var::Local(0), info(Width::Cell, Kind::F32)),
        (
            Var::Local(1),
            info(
                Width::Cells,
                Kind::Value {
                    ty: STRUCT_TYPE.to_owned(),
                    cells: 2,
                },
            ),
        ),
        (Var::Local(2), info(Width::Cell, Kind::Bool)),
    ];
    Head {
        value_types: Default::default(),
        program: 1,
        at: 0x10,
        name: "func_000010".to_owned(),
        returns,
        params: 2,
        locals: 8,
        vars: kinds.into_iter().collect(),
        incomplete: None,
    }
}

/// A list that names two builtins, by indices no program states, as the runtime's `int` and `float`.
fn listed() -> Names {
    let mut names = Names::default();
    names.types.insert(0x8000_0ABC, "System.Int32".to_owned());
    names.types.insert(0x8000_0ABD, "System.Single".to_owned());
    names
}

fn read_in(head: &Head, body: Vec<Stmt>, project: &Project) -> Vec<Stmt> {
    read_listed(head, body, project, &listed())
}

fn read_listed(head: &Head, body: Vec<Stmt>, project: &Project, names: &Names) -> Vec<Stmt> {
    let mut body = body;
    run(&mut body, head, project, names);
    body
}

/// What the evaluation of `value` reads as, in a function that returns one cell.
fn read(value: Expr, project: &Project) -> Expr {
    let body = vec![Stmt::Simple(ir::Stmt::Eval(value))];
    match read_in(&frame(Returns::Cell), body, project).pop() {
        Some(Stmt::Simple(ir::Stmt::Eval(value))) => value,
        other => panic!("the statement stays an evaluation: {other:?}"),
    }
}

#[test]
fn a_box_returned_or_stored_whole_is_the_conversion_to_the_reference_it_becomes() {
    let project = types();
    let returned = vec![Stmt::Return(Some(boxed_as(ENUM, int(3))))];
    let done = read_in(&frame(Returns::Cell), returned.clone(), &project);
    assert_eq!(done, vec![Stmt::Return(Some(converted(ENUM, int(3))))]);
    assert_eq!(tree::desugar(done), returned);

    // ⚠ A result the project proves a number is no reference, and none is handed back by `return;`.
    let mut numbered = project.clone();
    let signature = Signature {
        returns: 1,
        result: Kind::I32,
        ..Signature::default()
    };
    numbered
        .signatures
        .insert(FunctionKey::new(1, 0x10), signature);
    let head = frame(Returns::Cell);
    assert_eq!(read_in(&head, returned.clone(), &numbered), returned);
    assert_eq!(
        read_in(&frame(Returns::Void), returned.clone(), &project),
        returned
    );

    // A store of eight bytes takes the reference whole; one of four takes no box.
    let store = |kind: StoreKind| {
        Stmt::Simple(ir::Stmt::Store {
            kind,
            addr: Expr::Var(Var::Param(0)),
            offset: 0x10,
            value: boxed_as(ENUM, local(3)),
        })
    };
    let done = read_in(&head, vec![store(StoreKind::Ref)], &project);
    let Some(Stmt::Simple(ir::Stmt::Store { value, .. })) = done.first() else {
        panic!("a store: {done:?}");
    };
    assert_eq!(*value, converted(ENUM, local(3)));
    let narrow = vec![store(StoreKind::I32)];
    assert_eq!(read_in(&head, narrow.clone(), &project), narrow);

    // ⚠ A box compared is a reference compared, never its value: it stays.
    let compared = vec![Stmt::If {
        cond: Expr::Binary {
            op: BinOp::Eq,
            at: Kind::U64,
            l: Box::new(boxed_as(ENUM, local(3))),
            r: Box::new(Expr::Const(Const {
                bits: 0,
                kind: Kind::U64,
            })),
        },
        then: vec![Stmt::Return(None)],
        els: Vec::new(),
    }];
    assert_eq!(read_in(&head, compared.clone(), &project), compared);
}

#[test]
fn a_box_handed_to_a_parameter_that_takes_a_reference_is_its_conversion() {
    let mut unsigned = types();
    unsigned.signatures.clear();
    let mut project = unsigned.clone();
    let objects = |n: usize| {
        let object = Param {
            kind: Kind::Object("System.Object".to_owned()),
            cells: 1,
        };
        vec![object; n]
    };
    let signature = Signature {
        params: objects(2),
        ..Signature::default()
    };
    project
        .signatures
        .insert(FunctionKey::new(1, 0x200), signature);
    let line = FunctionKey::new(1, 0x300);
    if let Some(facts) = project
        .types
        .get_mut(&TypeKey::of_name(ENUM).unwrap_or(TypeKey(0)))
    {
        facts.slots.insert(4, line);
    }
    project.slot_params.insert(line, objects(2));
    let ours = |args: Vec<Expr>| called(own(0x200), args);
    let int32 = "System.Int32";

    // A function of no type takes each box as a reference; a method's first is its receiver.
    let both = ours(vec![boxed_as(ENUM, int(3)), boxed_as(int32, local(3))]);
    assert_eq!(
        read(both.clone(), &project),
        ours(vec![converted(ENUM, int(3)), converted(int32, local(3))])
    );
    let mut methods = project.clone();
    let member = Member {
        owner: TypeKey(7),
        role: Role::Method,
        is_static: false,
        slot: None,
    };
    methods.members.insert(FunctionKey::new(1, 0x200), member);
    assert_eq!(
        read(both.clone(), &methods),
        ours(vec![boxed_as(ENUM, int(3)), converted(int32, local(3))])
    );
    let slot = Callee::Virtual {
        class: ENUM.to_owned(),
        method: "Method_4".to_owned(),
        ty: ENUM.to_owned(),
        slot: 4,
    };
    let on = called(
        slot.clone(),
        vec![boxed_as(ENUM, int(1)), boxed_as(ENUM, int(2))],
    );
    assert_eq!(
        read(on.clone(), &project),
        called(slot, vec![boxed_as(ENUM, int(1)), converted(ENUM, int(2))])
    );
    // ⚠ A parameter the callee's head does not declare an object is written a number: the box stays.
    assert_eq!(read(both.clone(), &unsigned), both);
    assert_eq!(read(on.clone(), &unsigned), on);
    let mut numbers = project.clone();
    let cells = Param {
        kind: Kind::Unknown,
        cells: 1,
    };
    let signature = Signature {
        params: vec![cells.clone(), cells],
        ..Signature::default()
    };
    numbers
        .signatures
        .insert(FunctionKey::new(1, 0x200), signature);
    assert_eq!(read(both.clone(), &numbers), both);

    // ⚠ A method of the runtime may take the number itself by another overload: only a script's
    // own type is read there, and never at a reading the printer makes of the box itself.
    let ink = called(
        builtin("Quill.Ink"),
        vec![boxed_as(ENUM, int(3)), boxed_as(int32, local(3))],
    );
    assert_eq!(
        read(ink, &project),
        called(
            builtin("Quill.Ink"),
            vec![converted(ENUM, int(3)), boxed_as(int32, local(3))]
        )
    );
    let to_int = |ty: &str| called(builtin(CONVERT_TO_INT32), vec![boxed_as(ty, local(3))]);
    assert_eq!(
        read(to_int(ENUM), &project),
        called(builtin(CONVERT_TO_INT32), vec![converted(ENUM, local(3))])
    );
    let stays = [
        ("a box the printer reads as `(int)x`", to_int(SMALL)),
        (
            "two boxes the printer reads as `a == b`",
            called(
                builtin(OBJECT_EQUALS),
                vec![boxed_as(ENUM, int(1)), boxed_as(ENUM, int(2))],
            ),
        ),
        (
            "an operator",
            called(
                builtin("Quill.op_Equality"),
                vec![boxed_as(ENUM, int(1)), boxed_as(ENUM, int(2))],
            ),
        ),
        (
            "a constructor",
            called(
                builtin("Quill.Ctor"),
                vec![local(4), boxed_as(ENUM, int(2))],
            ),
        ),
        (
            "a native",
            called(
                Callee::Native {
                    namespace: None,
                    name: "_0x0000000000000001".to_owned(),
                    hash: 1,
                    rva: 0x10,
                },
                vec![boxed_as(ENUM, int(1))],
            ),
        ),
        (
            "an array's store of eight bytes that are no reference",
            called(
                Callee::Intrinsic {
                    name: "array_set_u64".to_owned(),
                    ty: None,
                    wrapper: None,
                },
                vec![local(4), int(0), boxed_as(ENUM, int(1))],
            ),
        ),
    ];
    for (why, value) in stays {
        assert_eq!(read(value.clone(), &project), value, "{why}");
    }
    let set = |value: Expr| {
        let callee = Callee::Intrinsic {
            name: "array_set_ref".to_owned(),
            ty: None,
            wrapper: None,
        };
        called(callee, vec![local(4), int(0), value])
    };
    assert_eq!(
        read(set(boxed_as(ENUM, int(1))), &project),
        set(converted(ENUM, int(1)))
    );
}

#[test]
fn a_box_is_read_as_a_conversion_only_where_the_conversion_keeps_what_the_box_copies() {
    let project = types();
    let ours = |value: Expr| called(own(0x200), vec![value]);
    let reads = |ty: &str, value: Expr| {
        let found = read(ours(boxed_as(ty, value.clone())), &project);
        found == ours(converted(ty, value))
    };
    assert!(reads(ENUM, local(3)));
    assert!(reads("System.Boolean", local(2)));
    assert!(reads(STRUCT_TYPE, local(1)));
    let stays = [
        ("a float made an enum by its value", ENUM, local(0)),
        ("a float of the runtime", "System.Single", local(3)),
        ("a constant written with its sign", ENUM, int(-1)),
        ("a number made a truth", "System.Boolean", int(1)),
        (
            "a struct of a variable of no such type",
            STRUCT_TYPE,
            local(3),
        ),
        (
            "a global, whose kind the function does not state",
            ENUM,
            Expr::Var(Var::Global {
                program: 1,
                offset: 8,
                statics: false,
            }),
        ),
        (
            "a type the project does not hold",
            "type_0F0F0F0F0F0F0F0F",
            local(3),
        ),
    ];
    for (why, ty, value) in stays {
        assert!(!reads(ty, value), "{why}");
    }
}

#[test]
fn a_box_of_a_builtin_is_of_the_runtime_type_the_users_list_names_its_index() {
    let project = types();
    let head = frame(Returns::Cell);
    let int32 = "type_80000ABC";
    let body = |value: Expr| {
        let call = called(own(0x200), vec![value]);
        vec![Stmt::Simple(ir::Stmt::Eval(call))]
    };
    let boxed = body(boxed_as(int32, local(3)));
    let done = read_in(&head, boxed.clone(), &project);
    assert_eq!(done, body(converted(int32, local(3))));
    assert_eq!(tree::desugar(done), boxed);

    // ⚠ With no list the index says no type, and a float the list names keeps its box.
    let unlisted = read_listed(&head, boxed.clone(), &project, &Names::default());
    assert_eq!(unlisted, boxed);
    let float = body(boxed_as("type_80000ABD", local(3)));
    assert_eq!(read_in(&head, float.clone(), &project), float);
    // A float of the function boxed as an `int` would be converted by its value.
    let made = body(boxed_as(int32, local(0)));
    assert_eq!(read_in(&head, made.clone(), &project), made);
}
