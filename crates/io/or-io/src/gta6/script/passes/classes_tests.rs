use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Terminator};
use crate::gta6::script::ir::{StoreKind, VarInfo, Width};
use crate::gta6::script::{Names, Options};

const PROGRAM: u32 = 7;
/// A builtin type: the base of a class that derives from no script type.
const OBJECT: TypeKey = TypeKey(0x8000_0000);
const T1: TypeKey = TypeKey(0x11 << 40);
const T2: TypeKey = TypeKey(0x22 << 40);
const T3: TypeKey = TypeKey(0x33 << 40);
const T4: TypeKey = TypeKey(0x44 << 40);
const T5: TypeKey = TypeKey(0x55 << 40);

/// A type as a listing states it where an object of it is made.
fn vm_type(key: TypeKey, flags: u32) -> VmType {
    VmType {
        name: key.to_string(),
        byte_size: 24,
        id: 1,
        flags,
    }
}

fn key(at: u32) -> FunctionKey {
    FunctionKey::new(PROGRAM, at)
}

fn head(at: u32, params: u32, returns: Returns) -> Head {
    Head {
        program: PROGRAM,
        at,
        name: format!("func_{at:06X}"),
        returns,
        params,
        locals: 1,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

/// A function of two parameters whose one block holds `stmts`.
fn blocks(at: u32, returns: Returns, stmts: Vec<ir::Stmt>) -> cfg::Function {
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    cfg::Function {
        head: head(at, 2, returns),
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

fn function(at: u32, params: u32, body: Vec<Stmt>) -> tree::Function {
    tree::Function {
        head: head(at, params, Returns::Void),
        body,
    }
}

fn temp(n: u32) -> Var {
    Var::Value { at: n, index: 0 }
}

fn value(var: Var) -> Expr {
    Expr::Var(var)
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn make(target: Var, ty: &VmType) -> ir::Stmt {
    ir::Stmt::Assign {
        target,
        value: Expr::NewObject(ty.clone()),
    }
}

/// A call of the program's own function at `at`.
fn call(at: u32, args: Vec<Expr>) -> Expr {
    let callee = Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    };
    Expr::Call {
        callee,
        args,
        returns: None,
    }
}

/// A call of slot `slot` as the type `ty` declares it.
fn virtual_call(ty: TypeKey, slot: u32, args: Vec<Expr>) -> Expr {
    let callee = Callee::Virtual {
        class: ty.to_string(),
        method: format!("Method_{slot}"),
        ty: ty.to_string(),
        slot,
    };
    Expr::Call {
        callee,
        args,
        returns: None,
    }
}

fn eval(e: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(e))
}

fn method(owner: TypeKey, slot: Option<u32>) -> Member {
    Member {
        slot,
        ..provisional(owner, Role::Method)
    }
}

/// A type of `PROGRAM` with the functions at `slots` in its table.
fn facts(base: Option<TypeKey>, slots: &[(u32, u32)]) -> TypeFacts {
    TypeFacts {
        base,
        home: PROGRAM,
        slots: slots.iter().map(|&(slot, at)| (slot, key(at))).collect(),
        ..TypeFacts::default()
    }
}

/// What a project settles on for the function at `at` from the code of `functions` alone.
fn said_of(functions: &[cfg::Function], at: u32) -> Option<Member> {
    let mut project = Project::default();
    for f in functions {
        of_function(&mut project, f);
    }
    settle(&mut project);
    project.members.get(&key(at)).cloned()
}

/// `body` as `run` leaves it in a function of `PROGRAM`.
fn ran(body: &[Stmt], project: &Project, options: Options) -> Vec<Stmt> {
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    let mut f = function(0x100, 2, body.to_vec());
    run(&mut f, &cx, &mut Stats::default());
    f.body
}

fn unit_of(project: &Project, functions: Vec<tree::Function>, options: Options) -> Unit {
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    unit(PROGRAM, Vec::new(), functions, &cx)
}

#[test]
fn a_function_run_only_on_objects_just_made_of_one_type_is_its_constructor() {
    let ty = vm_type(T1, 0);
    // The function at `at` makes an object of `ty` and runs the function at 0x200 on it.
    let runs_on = |ty: &VmType, at: u32, also: Expr| {
        let run = ir::Stmt::Eval(call(0x200, vec![value(temp(1)), also]));
        blocks(at, Returns::Void, vec![make(temp(1), ty), run])
    };
    let with = |other: cfg::Function| said_of(&[runs_on(&ty, 0x100, p(1)), other], 0x200);
    let constructor = provisional(T1, Role::Constructor);

    assert_eq!(with(runs_on(&ty, 0x180, p(0))), Some(constructor.clone()));
    // Made in the call itself, with no variable between.
    let folded = ir::Stmt::Eval(call(0x200, vec![Expr::NewObject(ty.clone())]));
    let folded = blocks(0x180, Returns::Void, vec![folded]);
    assert_eq!(with(folded), Some(constructor.clone()));
    // Run after a statement that leaves the object alone, and after one that stores to it.
    let after = |between: ir::Stmt| {
        let run = ir::Stmt::Eval(call(0x200, vec![value(temp(1))]));
        blocks(0x180, Returns::Void, vec![make(temp(1), &ty), between, run])
    };
    let apart = ir::Stmt::Eval(call(0x300, vec![p(0)]));
    assert_eq!(with(after(apart)), Some(constructor));
    let stored = ir::Stmt::Store {
        kind: StoreKind::Ref,
        addr: value(temp(1)),
        offset: 8,
        value: p(0),
    };

    let called = ir::Stmt::Eval(call(0x200, vec![p(0), p(1)]));
    let referred = ir::Stmt::Assign {
        target: Var::Local(0),
        value: Expr::FunctionRef {
            program: PROGRAM,
            at: 0x200,
        },
    };
    let rebinds = ir::Stmt::Assign {
        target: Var::Param(0),
        value: p(1),
    };
    // Each of these alone leaves it a function like any other.
    for other in [
        // Called on what is no object just made, referred to, run on an object already used.
        blocks(0x180, Returns::Void, vec![called]),
        blocks(0x180, Returns::Void, vec![referred]),
        after(stored),
        // Run on an object of a second type, and handed the object twice.
        runs_on(&vm_type(T2, 0), 0x180, p(0)),
        runs_on(&ty, 0x180, value(temp(1))),
        // It returns a value; it writes its own first parameter.
        blocks(0x200, Returns::Cell, Vec::new()),
        blocks(0x200, Returns::Void, vec![rebinds]),
    ] {
        assert_eq!(with(other.clone()), None, "{other:?}");
    }
}

#[test]
fn a_function_bound_only_to_closure_objects_of_one_type_is_that_closures_lambda() {
    // A function that makes an object of `closure`, fills it and binds the function at 0x400 to it.
    let binds = |closure: &VmType, then: Vec<ir::Stmt>| {
        let delegate = vm_type(T5, 0);
        let ctor = Callee::Builtin {
            path: "Example.Bound.Ctor".to_owned(),
            ty: 2,
            method: 0,
        };
        let captured = ir::Stmt::Store {
            kind: StoreKind::Ref,
            addr: value(Var::Local(0)),
            offset: 8,
            value: p(0),
        };
        let bound = Expr::FunctionRef {
            program: PROGRAM,
            at: 0x400,
        };
        let target = Expr::cast(Scalar::U64, value(Var::Local(0)));
        let bind = Expr::Call {
            callee: ctor,
            args: vec![value(temp(2)), target, bound],
            returns: None,
        };
        let mut stmts = vec![
            make(Var::Local(0), closure),
            captured,
            make(temp(2), &delegate),
            ir::Stmt::Eval(bind),
        ];
        stmts.extend(then);
        blocks(0x100, Returns::Void, stmts)
    };
    let closure = vm_type(T1, GENERATED);
    let lambda = provisional(T1, Role::Lambda);

    // A lambda may return a value, which a constructor may not.
    let body = blocks(0x400, Returns::Cell, Vec::new());
    assert_eq!(
        said_of(&[binds(&closure, Vec::new()), body], 0x400),
        Some(lambda)
    );

    // A type the compiler did not make is no closure.
    let plain = vm_type(T1, 0);
    assert_eq!(said_of(&[binds(&plain, Vec::new())], 0x400), None);
    // The variable bound may hold something other than the object made.
    let reassigned = ir::Stmt::Assign {
        target: Var::Local(0),
        value: p(1),
    };
    assert_eq!(said_of(&[binds(&closure, vec![reassigned])], 0x400), None);
    // The function is also called like any other.
    let called = ir::Stmt::Eval(call(0x400, vec![p(0)]));
    assert_eq!(said_of(&[binds(&closure, vec![called])], 0x400), None);
    // It is bound to closure objects of two types.
    let second = vm_type(T2, GENERATED);
    let both = [binds(&closure, Vec::new()), binds(&second, Vec::new())];
    assert_eq!(said_of(&both, 0x400), None);
}

#[test]
fn parts_say_the_same_of_a_function_whatever_order_they_arrive_in() {
    // One program's part: an object of `ty` made and the function at 0x200 run on it.
    let part = |ty: TypeKey| {
        let run = ir::Stmt::Eval(call(0x200, vec![value(temp(1))]));
        let stmts = vec![make(temp(1), &vm_type(ty, 0)), run];
        let mut project = Project::default();
        of_function(&mut project, &blocks(0x100, Returns::Void, stmts));
        project
    };
    let merged = |types: &[TypeKey]| {
        let mut project = Project::default();
        for ty in types {
            absorb(&mut project, &mut part(*ty));
        }
        settle(&mut project);
        project.members.get(&key(0x200)).cloned()
    };

    assert_eq!(merged(&[T1, T1]), Some(provisional(T1, Role::Constructor)));
    // A second type in any one part is final, wherever that part comes.
    for order in [[T1, T1, T2], [T1, T2, T1], [T2, T1, T1]] {
        assert_eq!(merged(&order), None, "{order:?}");
    }
}

#[test]
fn the_one_type_whose_table_first_holds_a_function_declares_it() {
    let mut project = Project::default();
    project.types.insert(T1, facts(Some(OBJECT), &[(4, 0x100)]));
    // A derived type holds what it inherits, and what it declares in two of its slots.
    let derived = facts(Some(T1), &[(4, 0x100), (5, 0x200), (9, 0x200)]);
    project.types.insert(T2, derived);
    // Two types with no ancestor in common hold one function.
    project.types.insert(T3, facts(Some(OBJECT), &[(4, 0x300)]));
    project.types.insert(T4, facts(Some(OBJECT), &[(6, 0x300)]));
    // A type whose base no program read states: that base may hold the function too.
    let unseen = TypeKey(0x99 << 40);
    project.types.insert(T5, facts(Some(unseen), &[(4, 0x500)]));
    // What the code said of two functions a table holds and of one no table holds, and a note.
    let constructor = provisional(T3, Role::Constructor);
    for at in [0x100, 0x300, 0x700] {
        project.members.insert(key(at), constructor.clone());
    }
    project.members.insert(key(0x600), disputed());

    settle(&mut project);
    let expected = BTreeMap::from([
        (key(0x100), method(T1, Some(4))),
        (key(0x200), method(T2, Some(5))),
        (key(0x700), constructor),
    ]);
    assert_eq!(project.members, expected);
}

#[test]
fn a_call_proved_to_be_on_a_receiver_reads_as_a_method_call() {
    let mut project = Project::default();
    project.types.insert(T1, facts(Some(OBJECT), &[(4, 0x200)]));
    project.types.insert(T2, facts(None, &[]));
    let of_struct = TypeFacts {
        flags: STRUCT,
        ..facts(Some(OBJECT), &[])
    };
    project.types.insert(T3, of_struct);
    project.members.insert(key(0x200), method(T1, Some(4)));
    project.members.insert(key(0x300), method(T3, None));
    let of_static = Member {
        is_static: true,
        ..method(T1, None)
    };
    project.members.insert(key(0x400), of_static);
    let constructor = provisional(T1, Role::Constructor);
    project.members.insert(key(0x600), constructor);

    let on_class = virtual_call(T1, 4, vec![p(0), p(1)]);
    let direct = call(0x200, vec![p(0)]);
    // An interface's slot, a struct's method, a static function, a function no type claims, a constructor.
    let stay = [
        virtual_call(T2, 0, vec![p(0)]),
        call(0x300, vec![p(0)]),
        call(0x400, vec![p(0)]),
        call(0x500, vec![p(0)]),
        call(0x600, vec![p(0)]),
    ];
    let nested = |cond: Expr, then: Expr| {
        let mut body = vec![Stmt::If {
            cond,
            then: vec![eval(then)],
            els: Vec::new(),
        }];
        body.extend(stay.iter().cloned().map(eval));
        body
    };
    let body = nested(on_class.clone(), direct.clone());
    let as_method = |e: Expr| Expr::sugar(ExprSugar::Method, e);

    let done = ran(&body, &project, Options::default());
    assert_eq!(done, nested(as_method(on_class), as_method(direct)));
    // The reading comes off again, and none is made with its name off.
    assert_eq!(tree::desugar(done), body);
    let off = Options::default().without(Pass::Receivers);
    assert_eq!(ran(&body, &project, off), body);
}

#[test]
fn a_slot_of_a_type_nothing_states_reads_as_a_method_only_on_an_object_of_that_type() {
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let slot = virtual_call(T1, 0, vec![Expr::cast(Scalar::U64, p(1))]);
    // The call as `run` leaves it where the receiver is known to hold `kind`.
    let on = |kind: Kind| {
        let mut f = function(0x100, 2, vec![eval(slot.clone())]);
        let info = VarInfo {
            width: Width::Cell,
            kind,
            name: None,
        };
        f.head.vars.insert(Var::Param(1), info);
        run(&mut f, &cx, &mut Stats::default());
        f.body
    };

    let read = eval(Expr::sugar(ExprSugar::Method, slot.clone()));
    assert_eq!(on(Kind::Object(T1.to_string())), [read]);
    assert_eq!(on(Kind::Object(T2.to_string())), [eval(slot.clone())]);
    assert_eq!(on(Kind::Unknown), [eval(slot.clone())]);
}

#[test]
fn an_object_made_and_its_constructor_run_on_it_are_one_construction() {
    let mut project = Project::default();
    let constructor = provisional(T1, Role::Constructor);
    project.members.insert(key(0x200), constructor);
    project.members.insert(key(0x300), method(T1, None));
    let options = Options::default().without(Pass::Receivers);
    let made = make(temp(1), &vm_type(T1, 0));
    let run_on = |made: &ir::Stmt, at: u32, args: Vec<Expr>| {
        let call = ir::Stmt::Eval(call(at, args));
        vec![
            Stmt::Simple(made.clone()),
            Stmt::Simple(call),
            Stmt::Return(None),
        ]
    };

    let body = run_on(&made, 0x200, vec![value(temp(1)), p(0)]);
    let parts = vec![
        made.clone(),
        ir::Stmt::Eval(call(0x200, vec![value(temp(1)), p(0)])),
    ];
    let kind = Group::Construct;
    let group = Stmt::Sugar(Box::new(Sugar::Group { kind, parts }));
    let done = ran(&body, &project, options);
    assert_eq!(done, [group, Stmt::Return(None)]);
    // The two statements come back, and they are not grouped with the name off.
    assert_eq!(tree::desugar(done), body);
    let off = options.without(Pass::Constructors);
    assert_eq!(ran(&body, &project, off), body);

    let mut between = body.clone();
    between.insert(1, eval(call(0x500, vec![p(0)])));
    // ⚠ `x = new T(a)` assigns once the constructor has run: a global the constructor can read
    // while it runs, and a variable a pointer reaches, must hold the object before.
    let global = Var::Global {
        program: PROGRAM,
        offset: 0x10,
        statics: false,
    };
    let held = make(global, &vm_type(T1, 0));
    let mut pointed = body.clone();
    let address = Expr::AddrOf {
        var: temp(1),
        cell: 0,
    };
    pointed.insert(2, eval(call(0x500, vec![address])));
    for left in [
        run_on(&held, 0x200, vec![value(global), p(0)]),
        pointed,
        // An object of another type, and a function that is no constructor.
        run_on(&make(temp(1), &vm_type(T2, 0)), 0x200, vec![value(temp(1))]),
        run_on(&made, 0x300, vec![value(temp(1)), p(0)]),
        // The object handed in twice, another object than the one made, a statement between the two.
        run_on(&made, 0x200, vec![value(temp(1)), value(temp(1))]),
        run_on(&made, 0x200, vec![p(0), p(1)]),
        between,
    ] {
        assert_eq!(ran(&left, &project, options), left);
    }
}

#[test]
fn a_builtin_named_as_a_types_equality_reads_as_the_operator() {
    let builtin = |path: &str, args: Vec<Expr>| {
        let callee = Callee::Builtin {
            path: path.to_owned(),
            ty: 3,
            method: 1,
        };
        Expr::Call {
            callee,
            args,
            returns: Some(1),
        }
    };
    let empty = Project::empty();
    let equal = builtin("Example.Thing.op_Equality", vec![p(0), p(1)]);
    let body = [Stmt::Return(Some(equal.clone()))];
    let read = Expr::sugar(ExprSugar::Operator, equal);

    let done = ran(&body, empty, Options::default());
    assert_eq!(done, [Stmt::Return(Some(read))]);
    assert_eq!(tree::desugar(done), body);
    let off = Options::default().without(Pass::Operators);
    assert_eq!(ran(&body, empty, off), body);

    // Cells that are not two operands, and a builtin of another name.
    for left in [
        builtin("Example.Thing.op_Equality", vec![p(0), p(1), p(0), p(1)]),
        builtin("Example.Thing.Equals", vec![p(0), p(1)]),
    ] {
        let body = [Stmt::Return(Some(left))];
        assert_eq!(ran(&body, empty, Options::default()), body);
    }
}

/// A program of two classes, a closure, a struct and a free function, and what a project proves of them.
fn program() -> (Project, Vec<tree::Function>) {
    let mut project = Project::default();
    project.types.insert(T1, facts(Some(OBJECT), &[(4, 0x100)]));
    project.types.insert(T2, facts(Some(T1), &[(4, 0x200)]));
    let closure = TypeFacts {
        flags: GENERATED,
        ..facts(Some(OBJECT), &[])
    };
    project.types.insert(T3, closure);
    // A struct of the program that only states a field, and a type another program states.
    let of_struct = TypeFacts {
        flags: STRUCT,
        ..facts(None, &[])
    };
    project.types.insert(T4, of_struct);
    let elsewhere = TypeFacts {
        home: PROGRAM + 1,
        ..facts(None, &[])
    };
    project.types.insert(T5, elsewhere);
    let field = FieldFacts {
        offset: 8,
        bytes: 4,
        kind: Kind::I32,
    };
    for ty in [T1, T4, T5] {
        project.fields.insert(ty, vec![field.clone()]);
    }
    project.members.insert(key(0x100), method(T1, Some(4)));
    project.members.insert(key(0x110), method(T1, None));
    project.members.insert(key(0x120), method(T1, None));
    project.members.insert(key(0x200), method(T2, Some(4)));
    let lambda = provisional(T3, Role::Lambda);
    project.members.insert(key(0x400), lambda);
    // A note the gathering left, which no settling took away.
    project.members.insert(key(0x500), disputed());

    let makes = Stmt::Simple(make(temp(1), &vm_type(T3, GENERATED)));
    let load = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(p(0)),
        offset: 8,
    };
    let functions = vec![
        function(0x100, 1, vec![makes]),
        function(0x110, 1, vec![Stmt::Return(Some(load))]),
        function(0x120, 2, vec![stores(8)]),
        function(0x200, 1, Vec::new()),
        function(0x400, 1, Vec::new()),
        function(0x500, 1, Vec::new()),
    ];
    (project, functions)
}

/// A setter's one statement: the second parameter stored to four bytes at `offset` of the first.
fn stores(offset: u32) -> Stmt {
    Stmt::Simple(ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: p(0),
        offset,
        value: p(1),
    })
}

/// A class's key, kind, base and count of fields.
fn stated(class: &Class) -> (TypeKey, ClassKind, Option<TypeKey>, usize) {
    (class.key, class.kind, class.base, class.fields.len())
}

/// Each method of a class: its offset, its role and whether it overrides.
fn methods(class: &Class) -> Vec<(u32, Role, bool)> {
    let method = |m: &Method| (m.function.head.at, m.role, m.overrides);
    class.methods.iter().map(method).collect()
}

#[test]
fn a_programs_functions_go_under_the_types_they_are_proved_members_of() {
    let (project, functions) = program();
    let done = unit_of(&project, functions.clone(), Options::default());

    // The type another program states is not this program's to declare.
    let classes: Vec<_> = done.classes.iter().map(stated).collect();
    assert_eq!(
        classes,
        [
            (T1, ClassKind::Class, Some(OBJECT), 1),
            (T2, ClassKind::Class, Some(T1), 0),
            (T4, ClassKind::Struct, None, 1),
        ]
    );
    // ⚠ A function that only reads a field, and one that only stores it, stay methods: their calls name them as methods.
    let all = [
        (0x100, Role::Method, false),
        (0x110, Role::Method, false),
        (0x120, Role::Method, false),
    ];
    assert_eq!(methods(&done.classes[0]), all);
    // A method in a slot the base fills too overrides it.
    assert_eq!(methods(&done.classes[1]), [(0x200, Role::Method, true)]);
    // The closure class is in the class whose method makes its objects.
    let nested = &done.classes[0].nested;
    assert_eq!(nested.len(), 1);
    assert_eq!(
        stated(&nested[0]),
        (T3, ClassKind::Closure, Some(OBJECT), 0)
    );
    assert_eq!(methods(&nested[0]), [(0x400, Role::Lambda, false)]);
    // A function the gathering only left a note of belongs to no type.
    let free: Vec<u32> = done.functions.iter().map(|m| m.function.head.at).collect();
    assert_eq!(free, [0x500]);

    // With `classes` off every function is free.
    let off = Options::default().without(Pass::Classes);
    let flat = unit_of(&project, functions.clone(), off);
    assert_eq!(
        (flat.classes.len(), flat.functions.len()),
        (0, functions.len())
    );
}

#[test]
fn nesting_stops_at_what_is_not_proved() {
    let nested = |functions: Vec<tree::Function>| {
        let (project, _) = program();
        let done = unit_of(&project, functions, Options::default());
        done.classes[0].nested.len()
    };
    let (_, functions) = program();
    assert_eq!(nested(functions.clone()), 1);
    // A method of a second class makes the closure's objects too.
    let mut shared = functions.clone();
    shared[3].body = functions[0].body.clone();
    assert_eq!(nested(shared), 0);
}

#[test]
fn a_units_classes_members_and_readings_are_counted() {
    let (mut project, mut functions) = program();
    let constructor = provisional(T1, Role::Constructor);
    project.members.insert(key(0x600), constructor);
    functions.push(function(0x600, 1, Vec::new()));
    // The free function makes an object, constructs it and calls a method.
    let news = [
        Stmt::Simple(make(temp(1), &vm_type(T1, 0))),
        eval(call(0x600, vec![value(temp(1))])),
        eval(call(0x200, vec![p(0)])),
    ];
    functions[5].body = ran(&news, &project, Options::default());

    let done = unit_of(&project, functions, Options::default());
    let expected = Counts {
        classes: 4,
        methods: 6,
        free: 1,
        constructors: 1,
        lambdas: 1,
        accessors: 0,
        overrides: 1,
        method_calls: 1,
        constructions: 1,
        operators: 0,
    };
    assert_eq!(counts(&done), expected);
}

#[test]
fn a_types_flags_say_what_it_is_in_the_source() {
    let of = |flags: u32, base: Option<TypeKey>| {
        let facts = TypeFacts {
            flags,
            base,
            ..TypeFacts::default()
        };
        kind(Some(&facts))
    };
    assert_eq!(of(0, Some(OBJECT)), ClassKind::Class);
    assert_eq!(of(STRUCT, Some(OBJECT)), ClassKind::Struct);
    assert_eq!(of(0x48, Some(OBJECT)), ClassKind::Enum);
    assert_eq!(
        of(STRUCT | GENERATED, Some(OBJECT)),
        ClassKind::StateMachine
    );
    assert_eq!(of(ABSTRACT, None), ClassKind::Interface);
    // An abstract class states a base, and a type nothing states is taken for a class.
    assert_eq!(of(ABSTRACT, Some(OBJECT)), ClassKind::Class);
    assert_eq!(kind(None), ClassKind::Class);
}

#[test]
fn a_function_that_holds_a_wait_is_async_and_no_other_is() {
    let wait = Stmt::Sugar(Box::new(Sugar::Await {
        before: Vec::new(),
        ready: p(0),
        suspend: Vec::new(),
        resumed: tree::Label::Block(1),
        resume: Vec::new(),
        after: Vec::new(),
    }));
    // A wait under a test is a wait of the function too.
    let nested = Stmt::If {
        cond: p(0),
        then: vec![wait.clone()],
        els: Vec::new(),
    };
    let functions = vec![
        function(0x10, 1, vec![wait]),
        function(0x20, 1, vec![eval(p(0))]),
        function(0x30, 1, vec![nested]),
    ];
    // With the classes left off the functions are all free, and a wait is still a wait.
    for options in [
        Options::default(),
        Options::default().without(Pass::Classes),
    ] {
        let unit = unit_of(&Project::default(), functions.clone(), options);
        let said: Vec<(u32, bool)> = (unit.functions.iter())
            .map(|method| (method.function.head.at, method.is_async))
            .collect();
        assert_eq!(said, [(0x10, true), (0x20, false), (0x30, true)]);
    }
}
