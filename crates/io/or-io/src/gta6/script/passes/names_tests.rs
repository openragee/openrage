use super::*;
use crate::gta5::script::natives::NativeDb;
use crate::gta6::script::ir::{BinOp, Returns, Scalar, VmType};
use crate::gta6::script::project::{Member, TypeFacts};
use crate::gta6::script::tree::Function;
use crate::gta6::script::{Options, Project};

fn head(params: u32, locals: u32) -> Head {
    Head {
        value_types: Default::default(),
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
        single: false,
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
        // A list writes a name it guessed after `_`.
        ("_GET_ID", None),
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

/// The baseline's passes and `passes`, over an empty project.
fn with<'a>(names: &'a Names, passes: &[Pass]) -> Context<'a> {
    let options = passes
        .iter()
        .fold(Options::baseline(), |o, pass| o.with(*pass));
    Context::new(names, Project::empty(), options)
}

fn applied(head: Head, body: Vec<tree::Stmt>, cx: &Context<'_>) -> (Function, Did) {
    let mut f = Function { head, body };
    let did = apply(&mut f, cx);
    (f, did)
}

fn object(ty: &str) -> Expr {
    Expr::NewObject(VmType {
        name: ty.to_owned(),
        byte_size: 16,
        id: 3,
        flags: 0,
    })
}

/// A call handed the address of `local_n`: a write through it, for all its caller knows.
fn fill(local: u32) -> tree::Stmt {
    let address = Expr::AddrOf {
        var: Var::Local(local),
        cell: 0,
    };
    eval(native("FILL", 0xA4, vec![address]))
}

#[test]
fn a_variable_takes_the_role_of_the_named_type_whose_new_object_it_holds() {
    let mut names = Names::default();
    names.types.insert(0xAA, "Lantern".to_owned());
    names.types.insert(0xBB, "<Private>".to_owned());
    let safe = || object("type_00000000000000AA");
    let locals: Vec<Var> = (0..5).map(Var::Local).collect();
    let body = vec![
        assign(locals[0], safe()),
        assign(locals[1], object("Demo.Collections.Dictionary_2")),
        // The role of a delegate would be a word of C#.
        assign(locals[2], object("Demo.Delegate")),
        // No list names this type, and the next one's name is no identifier.
        assign(locals[3], object("type_00000000000000CC")),
        assign(locals[4], object("type_00000000000000BB")),
    ];
    let cx = with(&names, &[Pass::RoleNames]);
    let (f, did) = applied(head(0, 5), body, &cx);
    let roles: Vec<Option<&str>> = locals.iter().map(|v| name_of(&f, *v)).collect();
    assert_eq!(
        roles,
        [Some("lantern"), Some("dictionary"), None, None, None]
    );
    assert_eq!(did.roles, 2);

    let first = locals[0];
    // Another value, and a write through its address, are each another thing it holds.
    let twice = vec![assign(first, safe()), assign(first, int(0))];
    assert_eq!(name_of(&applied(head(0, 1), twice, &cx).0, first), None);
    let addressed = vec![assign(first, safe()), fill(0)];
    assert_eq!(name_of(&applied(head(0, 1), addressed, &cx).0, first), None);

    // With `role-names` off the variable keeps its slot name.
    let off = named(head(0, 1), vec![assign(first, safe())], &names);
    assert_eq!(name_of(&off, first), None);
}

#[test]
fn a_variable_takes_the_role_of_what_the_getter_it_holds_the_result_of_gets() {
    let builtin = |path: &str| Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 4,
            method: 2,
        },
        args: vec![var(Var::Param(0))],
        returns: Some(1),
    };
    let method = |ty: u64, slot: u32| Expr::Call {
        callee: Callee::Virtual {
            class: format!("type_{ty:016X}"),
            method: format!("Method_{slot}"),
            ty: format!("type_{ty:016X}"),
            slot,
        },
        args: vec![var(Var::Param(0))],
        returns: Some(1),
    };
    // 0xA1 derives from 0xA0, which declares the slots; 0xA2 is no relation of either.
    let mut names = Names::default();
    for (slot, name) in [(3, "GetOwner"), (4, "IsMoored"), (5, "Start")] {
        names.methods.insert((TypeKey(0xA0), slot), name.to_owned());
    }
    let mut project = Project::default();
    let derived = TypeFacts {
        base: Some(TypeKey(0xA0)),
        ..TypeFacts::default()
    };
    project.types.insert(TypeKey(0xA1), derived);
    project.types.insert(TypeKey(0xA2), TypeFacts::default());
    let options = Options::baseline().with(Pass::RoleNames);
    let cx = Context::new(&names, &project, options);

    let locals: Vec<Var> = (0..7).map(Var::Local).collect();
    let values = vec![
        builtin("Demo.List_1.get_Count"),
        Expr::cast(
            Scalar::I32,
            builtin("Demo.IEnumerable.GetEnumerator__7_1_1"),
        ),
        method(0xA1, 3),
        method(0xA0, 4),
        // Neither a getter nor a test.
        method(0xA0, 5),
        builtin("Demo.IEnumerator.MoveNext"),
        // The same slot of a type that is no relation.
        method(0xA2, 3),
    ];
    let body = || -> Vec<tree::Stmt> {
        let held = locals.iter().zip(&values);
        held.map(|(local, value)| assign(*local, value.clone()))
            .collect()
    };
    let (f, did) = applied(head(1, 7), body(), &cx);
    let roles: Vec<Option<&str>> = locals.iter().map(|v| name_of(&f, *v)).collect();
    let got = ["count", "enumerator", "owner", "isMoored"].map(Some);
    assert_eq!(roles[..4], got);
    assert_eq!(roles[4..], [None::<&str>; 3]);
    assert_eq!(did.roles, 4);

    // With `role-names` off none of them is named.
    let off = named(head(1, 7), body(), &names);
    assert!(locals.iter().all(|v| name_of(&off, *v).is_none()));
}

#[test]
fn a_variable_takes_the_role_of_the_named_field_it_is_read_from() {
    let owner = TypeKey(0xA0);
    let load = |offset: u32| Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(var(Var::Param(0))),
        offset,
    };
    let field = |offset: u32| Expr::sugar(ExprSugar::Field { owner }, load(offset));
    let mut names = Names::default();
    names.fields.insert((owner, 0x10), "_tint".to_owned());
    let (a, b, c) = (Var::Local(0), Var::Local(1), Var::Local(2));
    let body = vec![
        assign(a, field(0x10)),
        // A field no list names, and a load no pass proved a field of that type.
        assign(b, field(0x18)),
        assign(c, load(0x10)),
    ];
    let (f, did) = applied(head(1, 3), body, &with(&names, &[Pass::RoleNames]));
    assert_eq!(
        (name_of(&f, a), name_of(&f, b), name_of(&f, c)),
        (Some("tint"), None, None)
    );
    assert_eq!(did.roles, 1);
}

#[test]
fn a_variable_a_native_fills_through_a_named_pointer_takes_that_parameters_name() {
    let names = list(vec![
        (
            0xC1,
            listed("GET_DEMO_HEIGHT", &[("int", "index"), ("float*", "height")]),
        ),
        (0xC2, listed("GET_DEMO_NAME", &[("char*", "buffer")])),
        (0xC3, listed("GET_DEMO_PLACE", &[("Vector3*", "place")])),
    ]);
    let cx = with(&names, &[Pass::NativeParams]);
    let address = || Expr::AddrOf {
        var: Var::Local(0),
        cell: 0,
    };
    let height = || eval(native("GET_DEMO_HEIGHT", 0xC1, vec![int(0), address()]));
    let read = || eval(native("USE", 0xA3, vec![var(Var::Local(0))]));
    let filled = Var::Local(0);

    let (f, did) = applied(head(0, 1), vec![height(), read()], &cx);
    assert_eq!(name_of(&f, filled), Some("height"));
    assert_eq!(did.roles, 1);

    // An assignment, and a second use of its address, are each another value.
    let assigned = vec![assign(filled, int(1)), height()];
    assert_eq!(name_of(&applied(head(0, 1), assigned, &cx).0, filled), None);
    let twice = vec![height(), height()];
    assert_eq!(name_of(&applied(head(0, 1), twice, &cx).0, filled), None);

    // A pointer to text or to a vector fills more than the one cell.
    for (name, hash) in [("GET_DEMO_NAME", 0xC2), ("GET_DEMO_PLACE", 0xC3)] {
        let other = vec![eval(native(name, hash, vec![address()]))];
        let f = applied(head(0, 1), other, &cx).0;
        assert_eq!(name_of(&f, filled), None, "{name}");
    }

    // A variable of several cells is not the one the pointer fills.
    let mut wide = head(0, 1);
    wide.vars.insert(filled, handle());
    assert_eq!(name_of(&applied(wide, vec![height()], &cx).0, filled), None);

    // With `native-params` off the variable keeps its slot name.
    let off = named(head(0, 1), vec![height(), read()], &names);
    assert_eq!(name_of(&off, filled), None);
}

#[test]
fn a_constant_keeps_the_kind_hash_only_in_a_cell_a_table_says_holds_hashes() {
    const NAMED: u32 = 0x1A2B_3C4D;
    const NEGATIVE: u32 = 0x8A2B_3C4D;
    let mut names = Names::default();
    names.texts.insert(NAMED, "demo_name".to_owned());
    names.texts.insert(NEGATIVE, "demo_other".to_owned());
    names.texts.insert(0xCAFE, "small".to_owned());
    names.hash_slots.insert((0xD1, 2));
    let cx = with(&names, &[Pass::HashText]);

    let constant = |bits: u64, kind: Kind| Expr::Const(Const { bits, kind });
    let hash = |kind: Kind| constant(u64::from(NAMED), kind);
    // The handle in front takes cells 0 and 1, so the value is cell 2.
    let set = |value: Expr| eval(native("SET_THING", 0xD1, vec![var(Var::Param(0)), value]));
    let other = |value: Expr| eval(native("SET_OTHER", 0xD2, vec![var(Var::Param(0)), value]));
    let product = Expr::binary(BinOp::Mul, var(Var::Local(0)), hash(Kind::Hash));
    let body = vec![
        set(hash(Kind::I32)),
        set(Expr::cast(Scalar::U32, hash(Kind::Hash))),
        set(constant(
            0xFFFF_FFFF_0000_0000 | u64::from(NEGATIVE),
            Kind::I32,
        )),
        // A float literal is its bits where the vocabulary knows them, and a float where it does not.
        set(constant(u64::from(NAMED), Kind::F32)),
        set(Expr::Const(Const::f32(10.0))),
        // A hash the vocabulary has no text for is still a hash.
        set(constant(0x5566_7788, Kind::U32)),
        // Too small to be a hash: an enum's value passed where a hash goes.
        set(constant(0xCAFE, Kind::I32)),
        // A parameter typed as a hash that no table vouches for, and an operand of arithmetic.
        other(hash(Kind::Hash)),
        assign(Var::Local(0), product),
    ];
    let kinds = |f: &Function| {
        let mut kinds = Vec::new();
        tree::each_expr(&f.body, &mut |top| {
            top.walk(&mut |e| {
                if let Expr::Const(c) = e {
                    kinds.push(c.kind.clone());
                }
            });
        });
        kinds
    };
    let wide = || {
        let mut wide = head(1, 1);
        wide.vars.insert(Var::Param(0), handle());
        wide
    };

    let (f, did) = applied(wide(), body.clone(), &cx);
    let expected = [
        Kind::Hash,
        Kind::Hash,
        Kind::Hash,
        Kind::Hash,
        Kind::F32,
        Kind::Hash,
        Kind::I32,
        Kind::U32,
        Kind::U32,
    ];
    assert_eq!(kinds(&f), expected);
    assert_eq!(did.texts, 4);

    // Where the handle is one cell the value is cell 1, which no table names.
    let (narrow, did) = applied(head(1, 1), vec![set(hash(Kind::Hash))], &cx);
    assert_eq!(kinds(&narrow), [Kind::U32]);
    assert_eq!(did.texts, 0);

    // With `hash-text` off every constant is as it was.
    let off = named(wide(), body.clone(), &names);
    assert_eq!(off.body, body);
}

#[test]
fn what_a_handler_catches_is_an_assignment_too() {
    let caught = Var::Local(0);
    let get = || assign(caught, native("GET_DEMO_THING", 0xA1, vec![]));
    let handler = tree::Catch {
        ty: None,
        var: Some(caught),
        body: Vec::new(),
    };
    let guarded = tree::Stmt::Sugar(Box::new(tree::Sugar::Try {
        body: vec![get()],
        catches: vec![handler],
        finally: Vec::new(),
    }));
    let none = Names::default();
    assert_eq!(
        name_of(&named(head(0, 1), vec![guarded], &none), caught),
        None
    );
    let alone = named(head(0, 1), vec![get()], &none);
    assert_eq!(name_of(&alone, caught), Some("demoThing"));
}

#[test]
fn no_variable_of_a_setter_is_named_value() {
    let names = list(vec![(0xE1, listed("SET_DEMO", &[("int", "value")]))]);
    let body = || vec![eval(native("SET_DEMO", 0xE1, vec![var(Var::Local(0))]))];
    let free = named(head(0, 1), body(), &names);
    assert_eq!(name_of(&free, Var::Local(0)), Some("value"));

    let setter = Member {
        owner: TypeKey(0xA0),
        role: Role::Setter,
        is_static: false,
        slot: None,
    };
    let mut project = Project::default();
    let key = FunctionKey::new(0x1122_3344, 0x40);
    project.members.insert(key, setter);
    let cx = Context::new(&names, &project, Options::baseline());
    let (f, _) = applied(head(0, 1), body(), &cx);
    assert_eq!(name_of(&f, Var::Local(0)), Some("value_2"));
}

#[test]
fn a_types_name_a_methods_and_a_fields_become_a_role_by_one_rule() {
    for (name, role) in [
        ("Lantern", Some("lantern")),
        ("AnimScene", Some("animScene")),
        ("UIState", Some("uiState")),
        ("IEnumerator", Some("iEnumerator")),
        ("EVENT_DONE", Some("eventDone")),
        ("ID", Some("id")),
        ("_tint", Some("tint")),
        ("Delegate", None),
        ("P12", None),
        ("<>c", None),
        ("", None),
    ] {
        assert_eq!(camel(name).as_deref(), role, "{name}");
    }
    for (name, role) in [
        ("Lantern", Some("lantern")),
        ("Demo.Collections.Dictionary_2", Some("dictionary")),
        ("Vector3", Some("vector3")),
        ("Demo.String", None),
        ("type_0123456789ABCDEF", None),
        ("Demo.Pail<Demo.Lantern>", Some("pail")),
        ("Pail<Gourd<Lantern>, Wicket>", Some("pail")),
        ("Demo.Map<Lantern, Wicket>.Keys", Some("keys")),
        ("Pail<Lantern>>", None),
    ] {
        assert_eq!(type_role(name).as_deref(), role, "{name}");
    }
    for (name, role) in [
        ("GetKite", Some("kite")),
        ("get_Count", Some("count")),
        ("get_IsCompleted", Some("isCompleted")),
        ("get_Item__5_2_1", Some("item")),
        ("CreateVehicle", Some("vehicle")),
        ("FindIndex", Some("index")),
        ("IsMoored", Some("isMoored")),
        ("HasEntity", Some("hasEntity")),
        ("Getter", None),
        ("Get", None),
        ("Island", None),
        ("Start", None),
        ("MoveNext", None),
        ("get_Default", None),
    ] {
        assert_eq!(method_role(name).as_deref(), role, "{name}");
    }
}

fn intrinsic(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: None,
        },
        args,
        returns: Some(1),
    }
}

/// `for (index = 0; index < below; index = index + 1) { body }`.
fn counted(index: Var, below: Expr, body: Vec<tree::Stmt>) -> tree::Stmt {
    let start = Stmt::Assign {
        target: index,
        value: int(0),
    };
    let step = Stmt::Assign {
        target: index,
        value: Expr::binary(BinOp::Add, var(index), int(1)),
    };
    tree::Stmt::For {
        init: Some(Box::new(start)),
        cond: Some(Expr::binary(BinOp::Lt, var(index), below)),
        step: Some(Box::new(step)),
        body,
    }
}

/// The element of the array `p0` at `index`, read and dropped.
fn element(index: Var) -> tree::Stmt {
    let read = vec![var(Var::Param(0)), var(index)];
    eval(intrinsic("array_get_u64", read))
}

/// A head whose first `locals` locals are each proved an `int`.
fn counting(locals: u32) -> Head {
    let mut head = head(2, locals);
    for n in 0..locals {
        let info = VarInfo {
            kind: Kind::I32,
            ..VarInfo::new(Width::Cell)
        };
        head.vars.insert(Var::Local(n), info);
    }
    head
}

#[test]
fn the_index_a_for_counts_and_indexes_with_is_i_and_the_one_of_a_loop_inside_it_j() {
    let locals: Vec<Var> = (0..4).map(Var::Local).collect();
    // Four loops, each in the one before and each reading the element at its own index.
    let nest = || {
        let mut inside: Vec<tree::Stmt> = Vec::new();
        for index in locals.iter().rev() {
            let mut body = vec![element(*index)];
            body.append(&mut inside);
            inside = vec![counted(*index, int(8), body)];
        }
        inside
    };
    let names = Names::default();
    let cx = with(&names, &[Pass::RoleNames]);
    let (f, did) = applied(counting(4), nest(), &cx);
    let roles: Vec<Option<&str>> = locals.iter().map(|v| name_of(&f, *v)).collect();
    assert_eq!(
        roles,
        [Some("i"), Some("j"), Some("k"), None],
        "the letters go three loops deep"
    );
    assert_eq!(did.roles, 3);

    // With `role-names` off every index keeps its slot name.
    let off = named(counting(4), nest(), &names);
    assert!(locals.iter().all(|v| name_of(&off, *v).is_none()));

    // An element written at the index, its address, and a list's indexer are each an index.
    let (array, index) = (var(Var::Param(0)), locals[0]);
    let list = Expr::Call {
        callee: Callee::Builtin {
            path: "Demo.List_1.get_Item__5_2_1".to_owned(),
            ty: 4,
            method: 2,
        },
        args: vec![array.clone(), var(index)],
        returns: Some(1),
    };
    let stored = vec![array.clone(), Expr::cast(Scalar::I32, var(index)), int(7)];
    for indexed in [
        intrinsic("array_set_u32", stored),
        intrinsic("array_element_address", vec![array.clone(), var(index)]),
        list,
    ] {
        let body = vec![counted(index, int(8), vec![eval(indexed.clone())])];
        let (f, _) = applied(counting(1), body, &cx);
        assert_eq!(name_of(&f, index), Some("i"), "{indexed:?}");
    }
}

#[test]
fn an_index_that_is_anything_more_than_a_count_keeps_its_slot_name() {
    let names = Names::default();
    let cx = with(&names, &[Pass::RoleNames]);
    let index = Var::Local(0);
    let kept = |head: Head, body: Vec<tree::Stmt>| {
        let (f, did) = applied(head, body, &cx);
        name_of(&f, index).is_none() && did.roles == 0
    };
    let plain = || counted(index, int(8), vec![element(index)]);
    assert!(
        !kept(counting(1), vec![plain()]),
        "the whole pattern is named"
    );

    // Not proved an integer.
    assert!(kept(head(2, 1), vec![plain()]));
    // Written by something other than the head of a `for`.
    let skipped = vec![element(index), assign(index, int(5))];
    assert!(kept(counting(1), vec![counted(index, int(8), skipped)]));
    assert!(kept(counting(1), vec![assign(index, int(2)), plain()]));
    // Its address taken.
    assert!(kept(counting(1), vec![plain(), fill(0)]));

    // A head that is not the whole of a count: no start, a step by what a variable holds, a
    // step of another variable, a test that does not read the index.
    let part = |init: bool, by: Expr, stepped: Var, tested: Var| {
        let start = Stmt::Assign {
            target: index,
            value: int(0),
        };
        let step = Stmt::Assign {
            target: stepped,
            value: Expr::binary(BinOp::Add, var(index), by),
        };
        tree::Stmt::For {
            init: init.then(|| Box::new(start)),
            cond: Some(Expr::binary(BinOp::Lt, var(tested), int(8))),
            step: Some(Box::new(step)),
            body: vec![element(index)],
        }
    };
    let other = Var::Local(1);
    assert!(!kept(counting(2), vec![part(true, int(2), index, index)]));
    assert!(kept(counting(2), vec![part(false, int(1), index, index)]));
    assert!(kept(
        counting(2),
        vec![part(true, var(other), index, index)]
    ));
    assert!(kept(counting(2), vec![part(true, int(1), other, index)]));
    assert!(kept(counting(2), vec![part(true, int(1), index, other)]));

    // A parameter holds what its caller passed before it counts.
    let param = Var::Param(1);
    let mut head = counting(0);
    let info = VarInfo {
        kind: Kind::I32,
        ..VarInfo::new(Width::Cell)
    };
    head.vars.insert(param, info);
    let (f, _) = applied(
        head,
        vec![counted(param, int(8), vec![element(param)])],
        &cx,
    );
    assert_eq!(name_of(&f, param), None);
}

#[test]
fn indices_side_by_side_share_a_letter_and_one_that_counts_at_two_depths_takes_none() {
    let names = Names::default();
    let cx = with(&names, &[Pass::RoleNames]);
    let (a, b, c) = (Var::Local(0), Var::Local(1), Var::Local(2));
    let over = |index: Var| counted(index, int(8), vec![element(index)]);
    fn names_of(f: &Function) -> (Option<&str>, Option<&str>) {
        (name_of(f, Var::Local(0)), name_of(f, Var::Local(1)))
    }

    // One loop after another, each declaring its own index: both are `i`, and one index of both is one.
    let (f, did) = applied(counting(2), vec![over(a), over(b)], &cx);
    assert_eq!((names_of(&f), did.roles), ((Some("i"), Some("i")), 2));
    let (f, did) = applied(counting(2), vec![over(a), over(a)], &cx);
    assert_eq!((name_of(&f, a), did.roles), (Some("i"), 1));
    // A third beside them shares the name too, and one of another letter keeps its own.
    let inside = counted(c, int(8), vec![element(c), over(a)]);
    let (f, _) = applied(counting(3), vec![inside, over(b)], &cx);
    assert_eq!(
        (name_of(&f, c), name_of(&f, a), name_of(&f, b)),
        (Some("i"), Some("j"), Some("i"))
    );

    // An index read after its loop is declared at the top, where a second `i` would meet it.
    let read = eval(native("USE", 0xA3, vec![var(b)]));
    let (f, _) = applied(counting(2), vec![over(a), over(b), read], &cx);
    assert_eq!(names_of(&f), (Some("i"), Some("i_2")));
    // A loop a construct holds is declared at the top in the form of old.
    let guarded = tree::Stmt::Sugar(Box::new(tree::Sugar::Guarded {
        kind: tree::Guard::Using,
        enter: Vec::new(),
        body: vec![over(b)],
        exit: Vec::new(),
    }));
    let (f, _) = applied(counting(2), vec![over(a), guarded], &cx);
    assert_eq!(names_of(&f), (Some("i"), Some("i_2")));

    // A name the function has already is not taken again.
    let mut head = counting(2);
    if let Some(info) = head.vars.get_mut(&b) {
        info.name = Some("i".to_owned());
    }
    let (f, _) = applied(head, vec![over(a)], &cx);
    assert_eq!(names_of(&f), (Some("i_2"), Some("i")));
    let mut head = counting(3);
    if let Some(info) = head.vars.get_mut(&c) {
        info.name = Some("i".to_owned());
    }
    let (f, _) = applied(head, vec![over(a), over(b)], &cx);
    assert_eq!(names_of(&f), (Some("i_2"), Some("i_2")));

    // `b` counts inside the loop of `a` and again outside it: neither letter is its own.
    let nested = counted(a, int(8), vec![element(a), over(b)]);
    let (f, _) = applied(counting(2), vec![nested, over(b)], &cx);
    assert_eq!(names_of(&f), (Some("i"), None));

    // A loop that counts with no integer does not deepen the loops in it.
    let mut untyped = counting(2);
    untyped.vars.remove(&a);
    let (f, _) = applied(untyped, vec![counted(a, int(8), vec![over(b)])], &cx);
    assert_eq!(names_of(&f), (None, Some("i")));
}

#[test]
fn an_index_that_only_counts_is_i_however_the_count_is_read() {
    let names = Names::default();
    let cx = with(&names, &[Pass::RoleNames]);
    let index = Var::Local(0);
    let passed = eval(native("USE", 0xA3, vec![var(index)]));
    let next = Expr::binary(BinOp::Add, var(index), int(1));
    let beside = eval(intrinsic("array_get_u64", vec![var(Var::Param(0)), next]));
    for body in [vec![passed], vec![beside], Vec::new()] {
        let (f, did) = applied(counting(1), vec![counted(index, int(8), body)], &cx);
        assert_eq!((name_of(&f, index), did.roles), (Some("i"), 1));
    }
    // A loop that counts nothing a letter names still deepens the one in it.
    let (outer, inner) = (Var::Local(0), Var::Local(1));
    let nest = counted(outer, int(8), vec![counted(inner, int(4), Vec::new())]);
    let (f, _) = applied(counting(2), vec![nest], &cx);
    assert_eq!(
        (name_of(&f, outer), name_of(&f, inner)),
        (Some("i"), Some("j"))
    );
}

#[test]
fn a_variable_takes_the_role_of_what_a_function_a_list_names_as_a_getter_gets() {
    let mut names = Names::default();
    let own = FunctionKey::new(0x1122_3344, 0x80);
    names.functions.insert(own, "GetOwner".to_owned());
    let other = FunctionKey::new(0x55, 0x10);
    names.functions.insert(other, "FindKite".to_owned());
    names
        .functions
        .insert(FunctionKey::new(0x1122_3344, 0xA0), "Start".to_owned());
    let local = |at: u32| Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args: Vec::new(),
        returns: Some(1),
    };
    let imported = Expr::Call {
        callee: Callee::Import {
            module: "module_0x00000000000000CC".to_owned(),
            name: "func_000010".to_owned(),
            program: 0x55,
            at: 0x10,
        },
        args: Vec::new(),
        returns: Some(1),
    };
    let locals: Vec<Var> = (0..4).map(Var::Local).collect();
    let body = || {
        vec![
            assign(locals[0], local(0x80)),
            assign(locals[1], imported.clone()),
            // No getter by its name, and a function no list names.
            assign(locals[2], local(0xA0)),
            assign(locals[3], local(0x90)),
        ]
    };
    let cx = with(&names, &[Pass::RoleNames]);
    let (f, did) = applied(head(0, 4), body(), &cx);
    let roles: Vec<Option<&str>> = locals.iter().map(|v| name_of(&f, *v)).collect();
    assert_eq!(roles, [Some("owner"), Some("kite"), None, None]);
    assert_eq!(did.roles, 2);

    let off = named(head(0, 4), body(), &names);
    assert!(locals.iter().all(|v| name_of(&off, *v).is_none()));
}

#[test]
fn a_variable_handed_to_a_cell_that_holds_hashes_holds_hashes() {
    let mut names = Names::default();
    names.hash_slots.insert((0xD1, 0));
    let cx = with(&names, &[Pass::HashText]);
    let cell = |kind: Kind| {
        let mut info = VarInfo::new(Width::Cell);
        info.kind = kind;
        info
    };
    let mut held = head(2, 1);
    held.vars.insert(Var::Param(0), cell(Kind::Unknown));
    held.vars.insert(Var::Param(1), cell(Kind::F32));
    held.vars.insert(Var::Local(0), cell(Kind::I32));
    let set = |value: Expr| eval(native("SET_THING", 0xD1, vec![value]));
    let body = vec![
        set(var(Var::Param(0))),
        set(Expr::cast(Scalar::I32, var(Var::Local(0)))),
        // A float's kind is what its reads proved.
        set(var(Var::Param(1))),
    ];
    let mut f = Function { head: held, body };
    apply(&mut f, &cx);
    let kind = |v: Var| f.head.vars[&v].kind.clone();
    assert_eq!(kind(Var::Param(0)), Kind::Hash);
    assert_eq!(kind(Var::Local(0)), Kind::Hash);
    assert_eq!(kind(Var::Param(1)), Kind::F32);
    // Switched off by its name, no variable is a hash.
    let mut f = Function {
        head: head(1, 0),
        body: vec![set(var(Var::Param(0)))],
    };
    f.head.vars.insert(Var::Param(0), cell(Kind::Unknown));
    apply(&mut f, &with(&names, &[]));
    assert_eq!(f.head.vars[&Var::Param(0)].kind, Kind::Unknown);
}

#[test]
fn a_value_only_ever_stored_whole_to_one_named_field_takes_that_fields_role() {
    let owner = TypeKey(0xA0);
    let store = |offset: u32, value: Var| {
        let stored = Stmt::Store {
            kind: crate::gta6::script::ir::StoreKind::I32,
            addr: var(Var::Param(0)),
            offset,
            value: var(value),
        };
        tree::Stmt::Sugar(Box::new(tree::Sugar::Group {
            kind: tree::Group::Field { owner },
            parts: vec![stored],
        }))
    };
    let mut names = Names::default();
    names.fields.insert((owner, 0x10), "_tint".to_owned());
    names.fields.insert((owner, 0x14), "Shade".to_owned());
    let (a, b) = (Var::Local(0), Var::Local(1));
    let body = vec![
        // A parameter stored to a named field, and a local given once and stored so.
        store(0x10, Var::Param(1)),
        assign(a, int(3)),
        store(0x14, a),
        // A local also read otherwise, and a parameter stored to two named fields.
        assign(b, int(4)),
        store(0x10, b),
        eval(native("USE", 0xD5, vec![var(b)])),
        store(0x10, Var::Param(2)),
        store(0x14, Var::Param(2)),
    ];
    let (f, _) = applied(head(3, 2), body, &with(&names, &[Pass::RoleNames]));
    let named = |v: Var| name_of(&f, v);
    assert_eq!(
        (
            named(Var::Param(1)),
            named(a),
            named(b),
            named(Var::Param(2))
        ),
        (Some("tint"), Some("shade"), None, None)
    );
}

/// The name the names pass gives `v` of `body`.
fn slot_name(head: Head, body: Vec<tree::Stmt>, cx: &Context<'_>, v: Var) -> Option<String> {
    let (f, _) = applied(head, body, cx);
    name_of(&f, v).map(str::to_owned)
}

#[test]
fn a_slot_every_assignment_of_which_gives_one_role_takes_it() {
    let mut names = Names::default();
    names.types.insert(0xAA, "Lantern".to_owned());
    let cx = with(&names, &[Pass::RoleNames]);
    let get = || native("GET_DEMO_THING", 0xA1, vec![]);
    let other = || native("GET_DEMO_OTHER", 0xA2, vec![]);
    let unnamed = || native("_0x00000000000000A3", 0xA3, vec![]);
    let lantern = || object("type_00000000000000AA");
    let slot = Var::Local(0);
    fn name(body: Vec<tree::Stmt>, cx: &Context<'_>) -> Option<String> {
        slot_name(head(0, 1), body, cx, Var::Local(0))
    }

    let gets = || vec![assign(slot, get()), assign(slot, get())];
    assert_eq!(name(gets(), &cx).as_deref(), Some("demoThing"));
    let lanterns = vec![assign(slot, lantern()), assign(slot, lantern())];
    assert_eq!(name(lanterns, &cx).as_deref(), Some("lantern"));

    // Two roles, a value of no role, and a value of one kind beside one of another name nothing.
    for body in [
        vec![assign(slot, get()), assign(slot, other())],
        vec![assign(slot, get()), assign(slot, unnamed())],
        vec![assign(slot, get()), assign(slot, int(0))],
        vec![assign(slot, get()), assign(slot, lantern())],
    ] {
        assert_eq!(name(body.clone(), &cx), None, "{body:?}");
    }

    // A parameter holds what its caller passed too.
    let param = Var::Param(0);
    let given = vec![assign(param, get()), assign(param, get())];
    assert_eq!(slot_name(head(1, 0), given, &cx, param), None);

    // With `role-names` off only a slot given once takes a native's role.
    let off = with(&names, &[]);
    assert_eq!(name(gets(), &off), None);
    let once = vec![assign(slot, get())];
    assert_eq!(name(once, &off).as_deref(), Some("demoThing"));
}

#[test]
fn a_variable_of_one_value_takes_the_name_of_the_parameter_it_is_passed_as() {
    let names = list(vec![
        (
            0xB5,
            listed("HANDLE_DEMO", &[("Ped", "ped"), ("int", "flags")]),
        ),
        (0xB3, listed("COUNT_DEMO", &[("int", "count")])),
    ]);
    let cx = with(&names, &[Pass::RoleNames]);
    let (given, flags, twice) = (Var::Local(0), Var::Param(0), Var::Local(1));
    let mut wide = head(1, 2);
    wide.vars.insert(given, handle());
    let body = vec![
        assign(given, native("_0x00000000000000A3", 0xA3, vec![])),
        eval(native("HANDLE_DEMO", 0xB5, vec![var(given), var(flags)])),
        // Each also handed to a call no list names.
        eval(native("USE", 0xA4, vec![var(given), var(flags)])),
        // Given two values, one of them passed as `count`.
        assign(twice, int(1)),
        eval(native("COUNT_DEMO", 0xB3, vec![var(twice)])),
        assign(twice, int(2)),
        eval(native("USE", 0xA4, vec![var(twice)])),
    ];
    let (f, did) = applied(wide.clone(), body.clone(), &cx);
    assert_eq!(
        (name_of(&f, given), name_of(&f, flags), name_of(&f, twice)),
        (Some("ped"), Some("flags"), None)
    );
    assert_eq!(did.roles, 2);

    // Passed as two parameters of different names: neither.
    let apart = vec![
        assign(twice, int(1)),
        eval(native("COUNT_DEMO", 0xB3, vec![var(twice)])),
        eval(native("HANDLE_DEMO", 0xB5, vec![var(given), var(twice)])),
        eval(native("USE", 0xA4, vec![var(twice)])),
    ];
    assert_eq!(slot_name(wide.clone(), apart, &cx, twice), None);

    // With `role-names` off only a variable every read of which passes it is named.
    let off = named(wide, body, &names);
    assert_eq!((name_of(&off, given), name_of(&off, flags)), (None, None));
}

#[test]
fn a_variable_takes_the_role_of_the_named_field_whose_bytes_it_copies() {
    let owner = TypeKey(0xA0);
    let copy = |offset: u32| {
        let bytes = Expr::LoadBytes {
            addr: Box::new(var(Var::Param(0))),
            offset,
            bytes: 24,
        };
        Expr::sugar(ExprSugar::Field { owner }, bytes)
    };
    let mut names = Names::default();
    names.fields.insert((owner, 0x20), "Quibble".to_owned());
    let (a, b) = (Var::Local(0), Var::Local(1));
    let body = || vec![assign(a, copy(0x20)), assign(b, copy(0x40))];
    let (f, did) = applied(head(1, 2), body(), &with(&names, &[Pass::RoleNames]));
    assert_eq!((name_of(&f, a), name_of(&f, b)), (Some("quibble"), None));
    assert_eq!(did.roles, 1);

    let off = named(head(1, 2), body(), &names);
    assert_eq!(name_of(&off, a), None);
}

#[test]
fn a_variable_takes_the_role_of_the_named_script_type_it_is_cast_to() {
    let mut names = Names::default();
    names.types.insert(0xAA, "Lantern".to_owned());
    names.types.insert(0x8000_0051, "Demo.Flagon".to_owned());
    let cast = |name: &str| intrinsic(name, vec![var(Var::Param(0))]);
    let locals: Vec<Var> = (0..5).map(Var::Local).collect();
    let body = || {
        vec![
            assign(locals[0], cast("try_cast<type_00000000000000AA>")),
            assign(
                locals[1],
                Expr::cast(Scalar::I32, cast("cast<type_00000000000000AA>")),
            ),
            // A cast to a runtime type is as often an unboxing, and no list names this one.
            assign(locals[2], cast("try_cast<type_80000051>")),
            assign(locals[3], cast("cast<type_00000000000000CC>")),
            // A mode the decoder does not read is no cast.
            assign(locals[4], cast("cast_mode_2<type_00000000000000AA>")),
        ]
    };
    let (f, did) = applied(head(1, 5), body(), &with(&names, &[Pass::RoleNames]));
    let roles: Vec<Option<&str>> = locals.iter().map(|v| name_of(&f, *v)).collect();
    assert_eq!(
        roles,
        [Some("lantern"), Some("lantern_2"), None, None, None]
    );
    assert_eq!(did.roles, 2);

    let off = named(head(1, 5), body(), &names);
    assert_eq!(name_of(&off, locals[0]), None, "with `role-names` off");
}

#[test]
fn a_variable_fetched_once_takes_the_role_of_the_named_script_type_it_is_proved_to_hold() {
    let mut names = Names::default();
    names.types.insert(0xAA, "Lantern".to_owned());
    names
        .types
        .insert(0xAB, "Demo.Pail<Demo.Lantern>".to_owned());
    names.types.insert(0x8000_0051, "Demo.Flagon".to_owned());
    let of = |kind: Kind| VarInfo {
        kind,
        ..VarInfo::new(Width::Cell)
    };
    let lantern = || Kind::Object("type_00000000000000AA".to_owned());
    let kinds = [
        lantern(),
        Kind::Value {
            ty: "type_00000000000000AB".to_owned(),
            cells: 1,
        },
        lantern(),
        // A runtime type, and a type no list names.
        Kind::Object("type_80000051".to_owned()),
        Kind::Object("type_00000000000000CC".to_owned()),
        // Given a constant, given twice, and an integer.
        lantern(),
        lantern(),
        Kind::I32,
    ];
    let mut typed = head(1, kinds.len() as u32);
    let locals: Vec<Var> = (0..kinds.len() as u32).map(Var::Local).collect();
    for (local, kind) in locals.iter().zip(kinds) {
        typed.vars.insert(*local, of(kind));
    }
    let call = || native("_0x00000000000000A3", 0xA3, vec![var(Var::Param(0))]);
    let load = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(var(Var::Param(0))),
        offset: 0x10,
    };
    let body = || {
        vec![
            assign(locals[0], call()),
            assign(locals[1], load.clone()),
            assign(locals[2], Expr::cast(Scalar::I32, call())),
            assign(locals[3], call()),
            assign(locals[4], call()),
            assign(locals[5], int(0)),
            assign(locals[6], call()),
            assign(locals[6], call()),
            assign(locals[7], call()),
        ]
    };
    let cx = with(&names, &[Pass::RoleNames]);
    let (f, did) = applied(typed.clone(), body(), &cx);
    let roles: Vec<Option<&str>> = locals.iter().map(|v| name_of(&f, *v)).collect();
    let mut expected = vec![Some("lantern"), Some("pail"), Some("lantern_2")];
    expected.extend([None; 5]);
    assert_eq!(roles, expected);
    assert_eq!(did.roles, 3);

    // A write through its address is another value.
    let addressed = vec![assign(locals[0], call()), fill(0)];
    let (f, _) = applied(typed.clone(), addressed, &cx);
    assert_eq!(name_of(&f, locals[0]), None);

    // With `role-names` off none of them is named.
    let off = named(typed, body(), &names);
    assert!(locals.iter().all(|v| name_of(&off, *v).is_none()));
}

#[test]
fn a_native_a_list_names_with_a_guess_gives_no_role_nor_do_its_parameters() {
    let names = list(vec![
        (0xF1, listed("_GET_DEMO_GUESS", &[])),
        (0xF2, listed("_SET_DEMO_GUESS", &[("int", "index")])),
        (0xF3, listed("GET_DEMO_SURE", &[])),
        (0xF4, listed("SET_DEMO_SURE", &[("int", "index")])),
    ]);
    let (a, b) = (Var::Local(0), Var::Local(1));
    let body = |get: u64, set: u64| {
        vec![
            assign(a, native("GET", get, vec![])),
            assign(b, int(1)),
            eval(native("SET", set, vec![var(b)])),
        ]
    };
    let cx = with(&names, &[Pass::RoleNames]);
    let (f, did) = applied(head(0, 2), body(0xF1, 0xF2), &cx);
    assert_eq!((name_of(&f, a), name_of(&f, b), did.roles), (None, None, 0));
    let (f, _) = applied(head(0, 2), body(0xF3, 0xF4), &cx);
    assert_eq!(
        (name_of(&f, a), name_of(&f, b)),
        (Some("demoSure"), Some("index"))
    );
}

#[test]
fn a_label_a_list_of_guesses_gives_a_native_names_no_variable() {
    let mut names = Names::default();
    let guesses = "0x00000000000000A1 _GET_DEMO_FLASK\n";
    assert_eq!(names.read_labels("guesses", guesses), Ok(1));
    let held = Var::Local(0);
    let body = vec![assign(held, native("_0x00000000000000A1", 0xA1, vec![]))];
    let (f, did) = applied(head(0, 1), body, &with(&names, &[Pass::RoleNames]));
    assert_eq!(name_of(&f, held), None);
    assert_eq!(did.roles, 0);
}
