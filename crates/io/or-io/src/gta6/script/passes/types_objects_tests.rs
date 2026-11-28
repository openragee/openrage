use std::collections::BTreeMap;

use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, FunctionKey, Head, Kind, Returns, Scalar, Stmt, TypeKey, UnOp, Var,
    VmType,
};
use crate::gta6::script::names::Names;
use crate::gta6::script::project::{FieldFacts, Member, Project, TypeFacts};
use crate::gta6::script::{Context, Options, Pass, Stats};

/// The program the tests' functions belong to.
const PROGRAM: u32 = 0x1122_3344;
/// The names of two script types: `THING` is `TypeKey(0xAA)`, `OTHER` is `TypeKey(0xBB)`.
const THING: &str = "type_00000000000000AA";
const OTHER: &str = "type_00000000000000BB";

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn set(n: u32, value: Expr) -> Stmt {
    Stmt::Assign {
        target: Var::Local(n),
        value,
    }
}

fn load(addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(addr),
        offset,
    }
}

fn object(name: &str) -> Kind {
    Kind::Object(name.to_owned())
}

/// A call of a function of the program of which nothing is known.
fn unknown() -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        },
        args: Vec::new(),
        returns: None,
    }
}

/// A call through slot `slot` of the type named `ty`.
fn virtual_call(ty: &str, slot: u32, args: Vec<Expr>, returns: u32) -> Expr {
    Expr::Call {
        callee: Callee::Virtual {
            class: ty.to_owned(),
            method: format!("Method_{slot}"),
            ty: ty.to_owned(),
            slot,
        },
        args,
        returns: Some(returns),
    }
}

/// `array_get_u64(array, index)`.
fn element(array: Expr, index: Expr) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: "array_get_u64".to_owned(),
            ty: None,
            wrapper: Some("array_get_u64".to_owned()),
        },
        args: vec![array, index],
        returns: Some(1),
    }
}

/// `new T[2]`, of objects where `flags` say so and of values held in place otherwise.
fn array(name: &str, flags: u32) -> Expr {
    Expr::NewArray {
        ty: VmType {
            name: name.to_owned(),
            byte_size: 8,
            id: 3,
            flags,
        },
        len: Box::new(int(2)),
        wrapper: String::new(),
    }
}

/// `new T()`.
fn made(name: &str) -> Expr {
    Expr::NewObject(VmType {
        name: name.to_owned(),
        byte_size: 32,
        id: 7,
        flags: 0,
    })
}

/// A function at offset 0 of the tests' program, of one block that holds `stmts`.
fn function(stmts: Vec<Stmt>) -> Function {
    let head = Head {
        value_types: Default::default(),
        program: PROGRAM,
        at: 0,
        name: "func_000000".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 8,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

/// `stmts` typed under `options` with `project` at hand.
fn typed_with(stmts: Vec<Stmt>, project: &Project, options: Options) -> Function {
    let mut f = function(stmts);
    let names = Names::default();
    super::super::run_in(
        &mut f,
        &Context::new(&names, project, options),
        &mut Stats::default(),
    );
    f
}

/// `stmts` typed with `kinds` and `signs` on.
fn typed(stmts: Vec<Stmt>, project: &Project) -> Function {
    let options = Options::baseline().with(Pass::Kinds).with(Pass::Signs);
    typed_with(stmts, project, options)
}

fn kind_of(f: &Function, var: Var) -> Kind {
    (f.head.vars.get(&var)).map_or(Kind::Unknown, |info| info.kind.clone())
}

/// A project that holds `THING` as an object type whose field at 0x18 refers to an `OTHER`, and the tests' function as a member of `THING` under `member`.
fn with_member(flags: u32, member: Option<bool>) -> Project {
    let mut project = Project::default();
    let facts = TypeFacts {
        flags,
        byte_size: 32,
        ..TypeFacts::default()
    };
    project.types.insert(TypeKey(0xAA), facts);
    let field = FieldFacts {
        offset: 0x18,
        bytes: 8,
        kind: object(OTHER),
    };
    project.fields.insert(TypeKey(0xAA), vec![field]);
    if let Some(is_static) = member {
        let member = Member {
            owner: TypeKey(0xAA),
            role: Role::Method,
            is_static,
            slot: None,
        };
        project.members.insert(FunctionKey::new(PROGRAM, 0), member);
    }
    project
}

#[test]
fn the_receiver_of_a_member_of_an_object_type_is_one_and_what_its_fields_hold_is_typed() {
    let read = || vec![set(1, load(param(0), 0x18))];
    let f = typed(read(), &with_member(0, Some(false)));
    assert_eq!(kind_of(&f, Var::Param(0)), object(THING));
    assert_eq!(kind_of(&f, Var::Local(1)), object(OTHER));

    // No member: the receiver is some reference, and what is loaded through it nothing.
    let f = typed(read(), &with_member(0, None));
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::Pointer);
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);

    // A static member takes no receiver, and a value type's method a pointer to the value.
    for project in [with_member(0, Some(true)), with_member(0x04, Some(false))] {
        let f = typed(read(), &project);
        assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);
    }

    // A receiver the function assigns, or whose address it takes, is not what it was called on.
    let assigned = Stmt::Assign {
        target: Var::Param(0),
        value: unknown(),
    };
    let addressed = Stmt::Eval(Expr::Call {
        callee: Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        },
        args: vec![Expr::AddrOf {
            var: Var::Param(0),
            cell: 0,
        }],
        returns: Some(0),
    });
    for touched in [assigned, addressed] {
        let f = typed(
            vec![touched, read()[0].clone()],
            &with_member(0, Some(false)),
        );
        assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);
    }

    // With `kinds` off the project is not read.
    let off = typed_with(read(), &with_member(0, Some(false)), Options::baseline());
    assert_eq!(kind_of(&off, Var::Local(1)), Kind::Unknown);
}

#[test]
fn what_a_virtual_call_is_made_on_is_an_object_of_the_slots_type() {
    let stmts = || {
        vec![
            set(0, unknown()),
            Stmt::Eval(virtual_call(THING, 3, vec![local(0), int(1)], 0)),
            Stmt::Eval(virtual_call(THING, 4, vec![param(1)], 0)),
        ]
    };
    let f = typed(stmts(), &with_member(0, None));
    assert_eq!(kind_of(&f, Var::Local(0)), object(THING));
    assert_eq!(kind_of(&f, Var::Param(1)), object(THING));

    // A type the project does not hold, or a value type, gives the receiver no type: it is a
    // reference and no more, which a virtual call made on it says where nothing says more.
    for project in [Project::default(), with_member(0x04, None)] {
        let f = typed(stmts(), &project);
        assert_eq!(kind_of(&f, Var::Local(0)), Kind::Pointer);
        assert_eq!(kind_of(&f, Var::Param(1)), Kind::Pointer);
    }

    // What the variable is assigned says more than the call made on it.
    let mut assigned = stmts();
    assigned[0] = set(0, made(OTHER));
    let f = typed(assigned, &with_member(0, None));
    assert_eq!(kind_of(&f, Var::Local(0)), object(OTHER));

    // A local assigned twice, or a parameter assigned at all, may hold another value where no call
    // is made on it: of no type, a reference and no more.
    let mut twice = stmts();
    twice.insert(0, set(0, unknown()));
    twice.push(Stmt::Assign {
        target: Var::Param(1),
        value: unknown(),
    });
    let f = typed(twice, &with_member(0, None));
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Pointer);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::Pointer);

    // Calls made on it as two types leave it of neither: a reference and no more.
    let mut two = stmts();
    two.push(Stmt::Eval(virtual_call(OTHER, 3, vec![local(0)], 0)));
    let mut project = with_member(0, None);
    project.types.insert(TypeKey(0xBB), TypeFacts::default());
    let f = typed(two, &project);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Pointer);
}

#[test]
fn a_virtual_call_yields_what_every_function_of_its_slots_line_proves() {
    let slot = FunctionKey::new(PROGRAM, 0x200);
    let call = || virtual_call(THING, 3, vec![param(1)], 1);
    let tested = || {
        let test = Expr::binary(BinOp::Eq, Expr::cast(Scalar::U32, call()), int(0));
        vec![set(0, test)]
    };
    let mut project = with_member(0, None);
    if let Some(facts) = project.types.get_mut(&TypeKey(0xAA)) {
        facts.slots.insert(3, slot);
    }
    let unproved = project.clone();
    project.slot_results.insert(slot, Kind::Bool);

    let f = typed(tested(), &project);
    let negated = Expr::Unary {
        op: UnOp::Not,
        e: Box::new(call()),
    };
    assert_eq!(f.cfg.blocks[0].stmts, [set(0, negated)]);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Bool);

    // A line the project proves no kind of yields none: the cast stays.
    let f = typed(tested(), &unproved);
    assert_eq!(f.cfg.blocks[0].stmts, tested());
}

/// An intrinsic of one argument that yields one cell.
fn intrinsic(name: &str, arg: Expr) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: None,
        },
        args: vec![arg],
        returns: Some(1),
    }
}

/// A call of the function at `at` of the tests' program.
fn direct(at: u32, args: Vec<Expr>) -> Stmt {
    Stmt::Eval(Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args,
        returns: Some(0),
    })
}

/// `project` with the function at `at` of the tests' program a member of the type `owner`.
fn member_at(mut project: Project, at: u32, owner: TypeKey, is_static: bool) -> Project {
    let member = Member {
        owner,
        role: Role::Method,
        is_static,
        slot: None,
    };
    project
        .members
        .insert(FunctionKey::new(PROGRAM, at), member);
    project
}

#[test]
fn what_a_cast_to_an_object_type_yields_is_an_object_of_it() {
    let stmts = |stem: &str| {
        vec![
            // A null first, as a variable declared before the cast is given.
            set(0, int(0)),
            set(0, intrinsic(&format!("{stem}<{THING}>"), param(1))),
        ]
    };
    for stem in ["try_cast", "cast"] {
        let f = typed(stmts(stem), &with_member(0, None));
        assert_eq!(kind_of(&f, Var::Local(0)), object(THING), "{stem}");
    }

    // A cast of another mode, to a type the project does not hold or to a value type says nothing.
    let f = typed(stmts("cast_mode_2"), &with_member(0, None));
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    for project in [Project::default(), with_member(0x04, None)] {
        let f = typed(stmts("try_cast"), &project);
        assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    }

    // With `kinds` off the project is not read.
    let off = typed_with(
        stmts("try_cast"),
        &with_member(0, None),
        Options::baseline(),
    );
    assert_eq!(kind_of(&off, Var::Local(0)), Kind::Unknown);
}

#[test]
fn what_an_instance_member_is_called_on_is_an_object_of_its_owner() {
    let stmts = || vec![set(0, unknown()), direct(0x300, vec![local(0), int(1)])];
    let project = member_at(with_member(0, None), 0x300, TypeKey(0xAA), false);
    let f = typed(stmts(), &project);
    assert_eq!(kind_of(&f, Var::Local(0)), object(THING));

    // A static member takes no receiver, a value type's member a pointer, and a function that is
    // no member says nothing.
    let unproved = [
        member_at(with_member(0, None), 0x300, TypeKey(0xAA), true),
        member_at(with_member(0x04, None), 0x300, TypeKey(0xAA), false),
        member_at(with_member(0, None), 0x400, TypeKey(0xAA), false),
    ];
    for project in unproved {
        let f = typed(stmts(), &project);
        assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    }

    // A local assigned twice may hold another value where the member is not called on it.
    let mut twice = stmts();
    twice.push(set(0, unknown()));
    let f = typed(twice, &project);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
}

#[test]
fn calls_made_on_a_value_as_a_type_and_as_its_base_make_it_an_object_of_the_derived_one() {
    let stmts = || {
        vec![
            set(0, unknown()),
            Stmt::Eval(virtual_call(THING, 3, vec![local(0)], 0)),
            direct(0x300, vec![local(0)]),
        ]
    };
    let mut project = member_at(with_member(0, None), 0x300, TypeKey(0xBB), false);
    let derived = TypeFacts {
        base: Some(TypeKey(0xAA)),
        ..TypeFacts::default()
    };
    project.types.insert(TypeKey(0xBB), derived);
    let f = typed(stmts(), &project);
    assert_eq!(kind_of(&f, Var::Local(0)), object(OTHER));

    // Of two types neither of which derives from the other, the value is of neither: a reference
    // and no more, as the virtual call made on it says.
    project.types.insert(TypeKey(0xBB), TypeFacts::default());
    let f = typed(stmts(), &project);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Pointer);
}

#[test]
fn an_array_an_element_of_which_is_called_on_as_an_object_type_is_an_array_of_it() {
    let stmts = || {
        vec![
            set(0, load(param(1), 0x10)),
            set(1, element(local(0), int(0))),
            Stmt::Eval(virtual_call(THING, 3, vec![local(1)], 0)),
        ]
    };
    let project = with_member(0, None);
    let f = typed(stmts(), &project);
    assert_eq!(
        kind_of(&f, Var::Local(0)),
        Kind::Array(Box::new(object(THING)))
    );
    assert_eq!(kind_of(&f, Var::Local(1)), object(THING));

    // An array assigned twice, an element assigned twice, an element read under a cast: no array.
    let mut array_twice = stmts();
    array_twice.insert(0, set(0, unknown()));
    let mut element_twice = stmts();
    element_twice.push(set(1, unknown()));
    let mut cast = stmts();
    cast[1] = set(
        1,
        intrinsic(&format!("cast<{THING}>"), element(local(0), int(0))),
    );
    for stmts in [array_twice, element_twice, cast] {
        let f = typed(stmts, &project);
        assert!(
            !matches!(kind_of(&f, Var::Local(0)), Kind::Array(_)),
            "{:?}",
            f.cfg.blocks[0].stmts
        );
    }

    // Elements called on as two unrelated types: an array of neither.
    let mut two = stmts();
    two.push(set(2, element(local(0), int(1))));
    two.push(Stmt::Eval(virtual_call(OTHER, 3, vec![local(2)], 0)));
    let mut unrelated = project.clone();
    unrelated.types.insert(TypeKey(0xBB), TypeFacts::default());
    let f = typed(two, &unrelated);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);

    // With `kinds` off nothing is read.
    let off = typed_with(stmts(), &project, Options::baseline());
    assert_eq!(kind_of(&off, Var::Local(0)), Kind::Unknown);
}

#[test]
fn an_element_read_from_an_array_of_references_refers_to_what_the_array_holds() {
    let mut project = with_member(0, None);
    let held = FieldFacts {
        offset: 0x20,
        bytes: 8,
        kind: Kind::Array(Box::new(object(OTHER))),
    };
    if let Some(fields) = project.fields.get_mut(&TypeKey(0xAA)) {
        fields.push(held);
    }
    let stmts = vec![
        set(0, array(THING, 0)),
        set(1, element(local(0), int(0))),
        // Arrays of two types: an element of neither.
        set(2, array(THING, 0)),
        set(2, array(OTHER, 0)),
        set(3, element(local(2), int(0))),
        // Values held in place are no references.
        set(4, array(OTHER, 0x08)),
        set(5, element(local(4), int(0))),
        // An array a field of a typed object holds.
        set(6, made(THING)),
        set(7, element(load(local(6), 0x20), int(1))),
    ];
    let f = typed(stmts.clone(), &project);
    assert_eq!(kind_of(&f, Var::Local(1)), object(THING));
    assert_eq!(kind_of(&f, Var::Local(3)), Kind::Unknown);
    assert_eq!(kind_of(&f, Var::Local(5)), Kind::Unknown);
    assert_eq!(kind_of(&f, Var::Local(7)), object(OTHER));

    // With `kinds` off an array is a pointer, and its element nothing.
    let off = typed_with(stmts, &project, Options::baseline());
    assert_eq!(kind_of(&off, Var::Local(1)), Kind::Unknown);
    assert_eq!(kind_of(&off, Var::Local(7)), Kind::Unknown);
}
