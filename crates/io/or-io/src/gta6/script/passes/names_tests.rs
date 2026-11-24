use super::*;
use crate::gta5::script::natives::NativeDb;
use crate::gta6::script::ir::{Const, Returns, Scalar};
use crate::gta6::script::tree::Function;

fn head(params: u32, locals: u32) -> Head {
    Head {
        program: 0x1122_3344,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params,
        locals,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

/// A call of a native the listing names `DEMO.<name>`.
fn native(name: &str, hash: u64, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Native {
            namespace: Some("DEMO".to_owned()),
            name: name.to_owned(),
            hash,
            rva: 0,
        },
        args,
        returns: Some(1),
    }
}

/// A native as a user's list has it, with `(type, name)` parameters.
fn listed(name: &str, params: &[(&str, &str)]) -> NativeName {
    NativeName {
        namespace: Some("DEMO".to_owned()),
        name: name.to_owned(),
        params: params
            .iter()
            .map(|(ty, name)| NativeParam {
                ty: (*ty).to_owned(),
                name: (*name).to_owned(),
            })
            .collect(),
        returns: None,
    }
}

fn list(natives: Vec<(u64, NativeName)>) -> Names {
    Names {
        natives: Some(NativeDb::from_names("a test's", natives)),
        ..Names::default()
    }
}

fn var(v: Var) -> Expr {
    Expr::Var(v)
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn assign(target: Var, value: Expr) -> tree::Stmt {
    tree::Stmt::Simple(Stmt::Assign { target, value })
}

fn eval(e: Expr) -> tree::Stmt {
    tree::Stmt::Simple(Stmt::Eval(e))
}

fn named(head: Head, body: Vec<tree::Stmt>, names: &Names) -> Function {
    let mut f = Function { head, body };
    run(&mut f, names, &mut Stats::default());
    f
}

fn name_of(f: &Function, v: Var) -> Option<&str> {
    f.head.vars.get(&v).and_then(|info| info.name.as_deref())
}

fn handle() -> VarInfo {
    VarInfo {
        width: Width::Cells,
        kind: Kind::Handle,
        name: None,
    }
}

#[test]
fn a_variable_takes_the_role_of_the_named_native_whose_only_result_it_holds() {
    let (thing, ped) = (Var::Value { at: 0x10, index: 0 }, Var::Local(0));
    let body = vec![
        assign(
            thing,
            native("GET_DEMO_THING", 0xA1, vec![var(Var::Param(0))]),
        ),
        assign(
            ped,
            Expr::cast(Scalar::I32, native("DEMO_PED_ID", 0xA2, vec![])),
        ),
        eval(native("USE", 0xA3, vec![var(thing), var(ped)])),
    ];
    let f = named(head(1, 1), body, &Names::default());
    assert_eq!(name_of(&f, thing), Some("demoThing"));
    assert_eq!(
        name_of(&f, ped),
        Some("demoPed"),
        "the low bits of the result are still the result"
    );
    assert_eq!(
        name_of(&f, Var::Param(0)),
        None,
        "no list names the parameter it is passed as"
    );
}

#[test]
fn the_users_list_names_a_native_the_listing_knows_only_by_its_hash() {
    let held = Var::Local(0);
    let body = || vec![assign(held, native("_0x00000000000000A1", 0xA1, vec![]))];

    let unnamed = named(head(0, 1), body(), &Names::default());
    assert_eq!(name_of(&unnamed, held), None, "a hash is no role");

    let names = list(vec![(0xA1, listed("GET_LISTED_THING", &[]))]);
    let f = named(head(0, 1), body(), &names);
    assert_eq!(name_of(&f, held), Some("listedThing"));
}

#[test]
fn a_variable_with_another_value_or_an_address_keeps_its_slot_name() {
    let none = Names::default();
    let get = || native("GET_DEMO_THING", 0xA1, vec![]);
    let fill = |local: u32| {
        let address = Expr::AddrOf {
            var: Var::Local(local),
            cell: 0,
        };
        eval(native("FILL", 0xA4, vec![address]))
    };
    let (first, second) = (Var::Local(0), Var::Local(1));

    let twice = vec![assign(first, get()), assign(first, int(0))];
    assert_eq!(name_of(&named(head(0, 1), twice, &none), first), None);

    let tuple = Stmt::AssignMany {
        targets: vec![first, second],
        values: vec![int(1), int(2)],
    };
    let many = vec![assign(first, get()), tree::Stmt::Simple(tuple)];
    assert_eq!(name_of(&named(head(0, 2), many, &none), first), None);

    // A write through its address is another value.
    let addressed = vec![assign(first, get()), fill(0)];
    assert_eq!(name_of(&named(head(0, 1), addressed, &none), first), None);

    // A parameter holds what its caller passed too.
    let param = vec![assign(Var::Param(0), get())];
    let f = named(head(1, 0), param, &none);
    assert_eq!(name_of(&f, Var::Param(0)), None);

    // The address of an earlier local of two cells is the address of this one too.
    let after = || vec![assign(second, get()), fill(0)];
    let mut wide = head(0, 2);
    wide.vars.insert(first, handle());
    assert_eq!(name_of(&named(wide, after(), &none), second), None);
    // The same address on a local of one cell reaches nothing after it.
    let apart = named(head(0, 2), after(), &none);
    assert_eq!(name_of(&apart, second), Some("demoThing"));
}

#[test]
fn a_variable_always_passed_as_one_named_parameter_takes_that_name() {
    let names = list(vec![
        (
            0xB1,
            listed("SET_DEMO_VALUE", &[("int", "index"), ("float", "amount")]),
        ),
        (0xB2, listed("CLEAR_DEMO_VALUE", &[("int", "index")])),
    ]);
    let (index, amount) = (Var::Local(0), Var::Param(0));
    let set = vec![var(index), Expr::cast(Scalar::F32, var(amount))];
    let body = vec![
        assign(index, int(3)),
        eval(native("SET_DEMO_VALUE", 0xB1, set)),
        assign(index, int(4)),
        eval(native("CLEAR_DEMO_VALUE", 0xB2, vec![var(index)])),
    ];
    let f = named(head(1, 1), body, &names);
    assert_eq!(
        name_of(&f, index),
        Some("index"),
        "however often it is assigned, every read passes it as `index`"
    );
    assert_eq!(name_of(&f, amount), Some("amount"));
}

#[test]
fn a_variable_used_any_other_way_takes_no_parameters_name() {
    let names = list(vec![
        (
            0xB1,
            listed("SET_DEMO_VALUE", &[("int", "index"), ("float", "amount")]),
        ),
        (0xB3, listed("COUNT_DEMO", &[("int", "count")])),
        (
            0xB4,
            listed("HOLD_DEMO", &[("int", "p0"), ("Any", "unk_1")]),
        ),
        (
            0xB5,
            listed("HANDLE_DEMO", &[("Ped", "ped"), ("int", "flags")]),
        ),
    ]);
    let (a, b, c) = (Var::Local(0), Var::Local(1), Var::Local(2));
    let set = || eval(native("SET_DEMO_VALUE", 0xB1, vec![var(a), int(0)]));

    // One read that is no argument.
    let read = named(head(0, 2), vec![set(), assign(b, var(a))], &names);
    assert_eq!(name_of(&read, a), None);

    // Two parameters of different names.
    let count = eval(native("COUNT_DEMO", 0xB3, vec![var(a)]));
    let two = named(head(0, 1), vec![set(), count], &names);
    assert_eq!(name_of(&two, a), None);

    // A placeholder is no name.
    let hold = eval(native("HOLD_DEMO", 0xB4, vec![var(a), var(b)]));
    let held = named(head(0, 2), vec![hold], &names);
    assert_eq!((name_of(&held, a), name_of(&held, b)), (None, None));

    // An argument more than the list's parameters cover: the list is of another signature.
    let more = eval(native("COUNT_DEMO", 0xB3, vec![var(a), int(0)]));
    let uncovered = named(head(0, 1), vec![more], &names);
    assert_eq!(name_of(&uncovered, a), None);

    // One cell of a handle is not the handle; the parameter after it still lines up.
    let cells = eval(native("HANDLE_DEMO", 0xB5, vec![var(a), var(b), var(c)]));
    let spread = named(head(0, 3), vec![cells], &names);
    assert_eq!(
        (
            name_of(&spread, a),
            name_of(&spread, b),
            name_of(&spread, c)
        ),
        (None, None, Some("flags"))
    );

    // The handle in one variable is the whole parameter.
    let mut whole = head(0, 2);
    whole.vars.insert(a, handle());
    let one = eval(native("HANDLE_DEMO", 0xB5, vec![var(a), var(b)]));
    let merged = named(whole, vec![one], &names);
    assert_eq!(
        (name_of(&merged, a), name_of(&merged, b)),
        (Some("ped"), Some("flags"))
    );
}

#[test]
fn a_role_is_one_variables_and_never_a_name_the_printer_writes() {
    let get = || native("GET_DEMO_THING", 0xA1, vec![]);
    let import = Expr::Call {
        callee: Callee::Import {
            module: "shared".to_owned(),
            name: "func_0000A0".to_owned(),
            program: 0xAB,
            at: 0xA0,
        },
        args: vec![],
        returns: Some(0),
    };
    let body = vec![
        assign(Var::Local(0), get()),
        assign(Var::Local(1), get()),
        // `shared` is what the call after it prints under.
        assign(Var::Local(2), native("GET_SHARED", 0xA5, vec![])),
        eval(import),
        assign(Var::Local(3), native("GET_OBJECT", 0xA6, vec![])),
    ];
    let f = named(head(0, 4), body, &Names::default());
    assert_eq!(name_of(&f, Var::Local(0)), Some("demoThing"));
    assert_eq!(name_of(&f, Var::Local(1)), Some("demoThing_2"));
    assert_eq!(name_of(&f, Var::Local(2)), Some("shared_2"));
    assert_eq!(name_of(&f, Var::Local(3)), Some("@object"));

    // A name another pass gave stays, and is taken.
    let mut given = head(0, 2);
    let mut info = VarInfo::new(Width::Cell);
    info.name = Some("demoThing".to_owned());
    given.vars.insert(Var::Local(0), info);
    let body = vec![assign(Var::Local(0), int(0)), assign(Var::Local(1), get())];
    let f = named(given, body, &Names::default());
    assert_eq!(name_of(&f, Var::Local(0)), Some("demoThing"));
    assert_eq!(name_of(&f, Var::Local(1)), Some("demoThing_2"));
}

#[test]
fn a_function_with_no_body_is_left_as_it_is() {
    let mut stub = head(1, 0);
    stub.incomplete = Some("a reason".to_owned());
    let f = named(stub.clone(), Vec::new(), &Names::default());
    assert_eq!(f.head, stub);
}

#[test]
fn a_natives_name_and_a_parameters_become_a_role_by_one_rule() {
    for (native, role) in [
        ("PLAYER_PED_ID", Some("playerPed")),
        ("PLAYER_ID", Some("player")),
        ("GET_ENTITY_COORDS", Some("entityCoords")),
        ("CREATE_VEHICLE", Some("vehicle")),
        ("IS_PED_INJURED", Some("isPedInjured")),
        ("_GET_ID", Some("id")),
        ("GET_OBJECT", Some("@object")),
        ("_0x00000000000000A1", None),
        ("GET_", None),
        ("P12", None),
        ("WITH-DASH", None),
    ] {
        assert_eq!(role_of_native(native).as_deref(), role, "{native}");
    }
    for (parameter, role) in [
        ("entity", Some("entity")),
        ("vehicleHandle", Some("vehicleHandle")),
        ("object", Some("@object")),
        ("p0", None),
        ("P12", None),
        ("unk_1", None),
        ("param2", None),
        ("", None),
        ("two words", None),
        ("func_10", None),
    ] {
        assert_eq!(parameter_role(parameter).as_deref(), role, "{parameter}");
    }
}
