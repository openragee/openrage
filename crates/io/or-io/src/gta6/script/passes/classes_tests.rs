use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Terminator};
use crate::gta6::script::ir::{StoreKind, VarInfo, Width};
use crate::gta6::script::program::{MethodRow, TypeRow};
use crate::gta6::script::project::Signature;
use crate::gta6::script::{Names, Options};

const PROGRAM: u32 = 7;
/// A builtin type: the base of a class that derives from no script type.
const OBJECT: TypeKey = TypeKey(0x8000_0000);
const T1: TypeKey = TypeKey(0x11 << 40);
const T2: TypeKey = TypeKey(0x22 << 40);
const T3: TypeKey = TypeKey(0x33 << 40);
const T4: TypeKey = TypeKey(0x44 << 40);
const T5: TypeKey = TypeKey(0x55 << 40);
/// The index of the builtin type the delegates of these tests are objects of.
const DELEGATE: u32 = 2;

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

/// A builtin type as a program with no names list spells it where an object of it is made.
fn builtin_type(index: u32) -> VmType {
    let id = BUILTIN | index;
    VmType {
        name: TypeKey(u64::from(id)).to_string(),
        byte_size: 48,
        id,
        flags: 0,
    }
}

/// A delegate made into `held`, then bound to `target` and the function at `at` by a method of builtin type `of`.
// The call is spelled as a compiled program spells it: by the two numbers, with no name.
fn bind(held: Var, of: u32, target: Expr, at: u32) -> [ir::Stmt; 2] {
    let ctor = Callee::Builtin {
        path: format!("type_{:08X}.Method_0", BUILTIN | of),
        ty: of,
        method: 0,
    };
    let bound = Expr::FunctionRef {
        program: PROGRAM,
        at,
    };
    let bind = Expr::Call {
        callee: ctor,
        args: vec![value(held), target, bound],
        returns: None,
    };
    [make(held, &builtin_type(DELEGATE)), ir::Stmt::Eval(bind)]
}

/// A store of the second parameter to four bytes at `offset` of the first.
fn writes(offset: u32) -> ir::Stmt {
    ir::Stmt::Store {
        kind: StoreKind::I32,
        addr: p(0),
        offset,
        value: p(1),
    }
}

/// A function that only hands `first` and its second parameter to the function at `to`.
fn hands_on(at: u32, to: u32, first: Expr) -> cfg::Function {
    let handed = ir::Stmt::Eval(call(to, vec![first, p(1)]));
    blocks(at, Returns::Void, vec![handed])
}

/// A class of `PROGRAM` whose objects hold `byte_size` bytes after their header.
fn sized(base: TypeKey, byte_size: u32, slots: &[(u32, u32)]) -> TypeFacts {
    TypeFacts {
        byte_size,
        ..facts(Some(base), slots)
    }
}

/// What a project that states `types`, and the builtin type no script type is under, settles on from the code of `functions`.
fn settled(types: &[(TypeKey, TypeFacts)], functions: &[cfg::Function]) -> Project {
    let mut project = Project::default();
    project.types.insert(OBJECT, TypeFacts::default());
    project.types.extend(types.iter().cloned());
    for f in functions {
        of_function(&mut project, f);
    }
    settle(&mut project);
    project
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
    // A function that makes an object of `closure`, fills it and binds the function at 0x400 to
    // it by a method of builtin type `of`.
    let binds_by = |closure: &VmType, of: u32, then: Vec<ir::Stmt>| {
        let captured = ir::Stmt::Store {
            kind: StoreKind::Ref,
            addr: value(Var::Local(0)),
            offset: 8,
            value: p(0),
        };
        let target = Expr::cast(Scalar::U64, value(Var::Local(0)));
        let mut stmts = vec![make(Var::Local(0), closure), captured];
        stmts.extend(bind(temp(2), of, target, 0x400));
        stmts.extend(then);
        blocks(0x100, Returns::Void, stmts)
    };
    let binds = |closure: &VmType, then: Vec<ir::Stmt>| binds_by(closure, DELEGATE, then);
    let closure = vm_type(T1, GENERATED);
    let lambda = provisional(T1, Role::Lambda);

    // A lambda may return a value, which a constructor may not.
    let body = blocks(0x400, Returns::Cell, Vec::new());
    assert_eq!(
        said_of(&[binds(&closure, Vec::new()), body], 0x400),
        Some(lambda)
    );

    // ⚠ No name says what binds: a method of another builtin type than the object made binds nothing.
    let other = binds_by(&closure, DELEGATE + 1, Vec::new());
    assert_eq!(said_of(&[other], 0x400), None);
    // A type the compiler did not make is no closure.
    let plain = vm_type(T1, 0);
    assert_eq!(said_of(&[binds(&plain, Vec::new())], 0x400), None);
    // The variable bound may hold something other than the object made.
    let reassigned = ir::Stmt::Assign {
        target: Var::Local(0),
        value: p(1),
    };
    assert_eq!(said_of(&[binds(&closure, vec![reassigned])], 0x400), None);
    // The function is also called like any other, and handed the first parameter of a function no type claims.
    for first in [p(1), p(0)] {
        let called = ir::Stmt::Eval(call(0x400, vec![first]));
        assert_eq!(said_of(&[binds(&closure, vec![called])], 0x400), None);
    }
    // It is bound to closure objects of two types.
    let second = vm_type(T2, GENERATED);
    let both = [binds(&closure, Vec::new()), binds(&second, Vec::new())];
    assert_eq!(said_of(&both, 0x400), None);
}

#[test]
fn a_lambda_its_closures_other_lambdas_hand_their_receiver_stays_that_closures() {
    // The function at 0x100 makes the closure and binds the functions at 0x400 and 0x410 to it.
    let closure = vm_type(T1, GENERATED);
    let mut stmts = vec![make(Var::Local(0), &closure)];
    stmts.extend(bind(temp(2), DELEGATE, value(Var::Local(0)), 0x400));
    stmts.extend(bind(temp(3), DELEGATE, value(Var::Local(0)), 0x410));
    let maker = blocks(0x100, Returns::Void, stmts);
    let lambda = Some(provisional(T1, Role::Lambda));

    // One lambda of the closure runs the other on the closure it was given.
    let functions = [maker.clone(), hands_on(0x400, 0x410, p(0))];
    assert_eq!(said_of(&functions, 0x410), lambda);
    assert_eq!(said_of(&functions, 0x400), lambda);
    // A function no type claims hands it its first parameter too.
    let functions = [
        maker,
        hands_on(0x400, 0x410, p(0)),
        hands_on(0x500, 0x410, p(0)),
    ];
    assert_eq!(said_of(&functions, 0x410), None);
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
            single: false,
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

#[test]
fn a_function_handed_only_the_receiver_of_members_is_a_method_of_their_type() {
    // T1 holds 24 bytes and the function at 0x100 in a slot; T2 is made from T1, adds 16 bytes
    // and holds the function at 0x110; T3 is under neither and holds the function at 0x120.
    let types = [
        (T1, sized(OBJECT, 24, &[(4, 0x100)])),
        (T2, sized(T1, 40, &[(5, 0x110)])),
        (T3, sized(OBJECT, 24, &[(4, 0x120)])),
    ];
    // The function at 0x200 stores to its first parameter at `offset`.
    let works = |offset: u32| blocks(0x200, Returns::Void, vec![writes(offset)]);
    let said = |functions: &[cfg::Function], at: u32| {
        let project = settled(&types, functions);
        project.members.get(&key(at)).cloned()
    };

    // Handed the receiver of a method of T1, it stores to a field T1 adds to what its base holds.
    let direct = [hands_on(0x100, 0x200, p(0)), works(8)];
    assert_eq!(said(&direct, 0x200), Some(method(T1, None)));
    // The receiver comes through a function that is no member itself: it works on no field.
    let through = [
        hands_on(0x100, 0x300, p(0)),
        hands_on(0x300, 0x200, p(0)),
        works(8),
    ];
    assert_eq!(said(&through, 0x200), Some(method(T1, None)));
    assert_eq!(said(&through, 0x300), None);
    // Handed the receivers of T1 and of a type made from it, it is the method of T1.
    let both = |offset: u32| {
        [
            hands_on(0x100, 0x200, p(0)),
            hands_on(0x110, 0x200, p(0)),
            works(offset),
        ]
    };
    assert_eq!(said(&both(8), 0x200), Some(method(T1, None)));
    // Handed only the receiver of T2, it is T2's where the field is one T2 adds.
    let derived = |offset: u32| [hands_on(0x110, 0x200, p(0)), works(offset)];
    assert_eq!(said(&derived(32), 0x200), Some(method(T2, None)));

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
    let handed = ir::Stmt::Eval(call(0x200, vec![p(0), p(1)]));
    // Each of these leaves it a function like any other.
    let left: [Vec<cfg::Function>; 9] = [
        // A field its base holds may be the base's to work on; a field past the type is no field of it.
        derived(8).to_vec(),
        both(32).to_vec(),
        // It works on no field of its receiver.
        vec![
            hands_on(0x100, 0x200, p(0)),
            blocks(0x200, Returns::Void, Vec::new()),
        ],
        // A function no type claims hands it its first parameter; a member hands it something else.
        vec![
            hands_on(0x100, 0x200, p(0)),
            hands_on(0x300, 0x200, p(0)),
            works(8),
        ],
        vec![
            hands_on(0x100, 0x200, p(0)),
            hands_on(0x110, 0x200, p(1)),
            works(8),
        ],
        // It is referred to; the member writes its receiver before it hands it on.
        vec![
            hands_on(0x100, 0x200, p(0)),
            blocks(0x110, Returns::Void, vec![referred]),
            works(8),
        ],
        vec![
            blocks(0x100, Returns::Void, vec![rebinds.clone(), handed]),
            works(8),
        ],
        // It writes its own first parameter; it is handed receivers no one script type is under.
        vec![
            hands_on(0x100, 0x200, p(0)),
            blocks(0x200, Returns::Void, vec![rebinds, writes(8)]),
        ],
        vec![
            hands_on(0x100, 0x200, p(0)),
            hands_on(0x120, 0x200, p(0)),
            works(8),
        ],
    ];
    for functions in &left {
        assert_eq!(said(functions, 0x200), None, "{functions:?}");
    }

    // A type another program states holds no function of this one.
    let mut elsewhere = types.clone();
    elsewhere[0].1.home = PROGRAM + 1;
    let project = settled(&elsewhere, &direct);
    assert_eq!(project.members.get(&key(0x200)), None);
    // What the gathering held is gone once a project is settled.
    assert!(project
        .programs
        .values()
        .all(|row| row.carried == Carried::default()));
}

#[test]
fn a_function_bound_to_the_receiver_as_a_delegate_is_handed_it() {
    let types = [(T1, sized(OBJECT, 24, &[(4, 0x100)]))];
    let works = blocks(0x200, Returns::Void, vec![writes(8)]);
    // The method at 0x100 binds the function at 0x200 to its own receiver by a method of builtin type `of`.
    let said = |of: u32, target: Expr| {
        let binds = bind(temp(2), of, target, 0x200).to_vec();
        let functions = [blocks(0x100, Returns::Void, binds), works.clone()];
        let project = settled(&types, &functions);
        project.members.get(&key(0x200)).cloned()
    };

    assert_eq!(said(DELEGATE, p(0)), Some(method(T1, None)));
    let cast = Expr::cast(Scalar::U64, p(0));
    assert_eq!(said(DELEGATE, cast), Some(method(T1, None)));
    // Bound to something else, and by a method of another builtin type than the object made.
    assert_eq!(said(DELEGATE, p(1)), None);
    assert_eq!(said(DELEGATE + 1, p(0)), None);
}

#[test]
fn a_constructor_handed_the_receiver_of_a_derived_types_constructor_stays_one() {
    let types = [
        (T1, sized(OBJECT, 24, &[])),
        (T2, sized(T1, 40, &[(4, 0x410)])),
        (T3, sized(OBJECT, 24, &[])),
    ];
    // A function that makes an object of `ty` and runs the function at `to` on it.
    let makes = |at: u32, ty: TypeKey, to: u32| {
        let run = ir::Stmt::Eval(call(to, vec![value(temp(1))]));
        blocks(at, Returns::Void, vec![make(temp(1), &vm_type(ty, 0)), run])
    };
    // The function at 0x200 is run on new objects of T1, the one at 0x400 on new objects of T2.
    let made = [makes(0x500, T1, 0x200), makes(0x510, T2, 0x400)];
    let said = |others: Vec<cfg::Function>, at: u32| {
        let mut functions = made.to_vec();
        functions.extend(others);
        let project = settled(&types, &functions);
        project.members.get(&key(at)).cloned()
    };
    let constructor = |ty: TypeKey| Some(provisional(ty, Role::Constructor));

    // The constructor of T2 runs the constructor of its base on its own receiver.
    let chained = vec![hands_on(0x400, 0x200, p(0))];
    assert_eq!(said(chained.clone(), 0x200), constructor(T1));
    assert_eq!(said(chained, 0x400), constructor(T2));

    // A method of T2 hands it its receiver too: it is no constructor, and a method of T1 where it works on a field of T1.
    let works = blocks(0x200, Returns::Void, vec![writes(8)]);
    let by_method = vec![hands_on(0x400, 0x200, p(0)), hands_on(0x410, 0x200, p(0))];
    assert_eq!(said(by_method.clone(), 0x200), None);
    let mut working = by_method;
    working.push(works);
    assert_eq!(said(working, 0x200), Some(method(T1, None)));
    // The constructor of a type that is not made from T1 hands it its receiver.
    let unrelated = vec![makes(0x520, T3, 0x420), hands_on(0x420, 0x200, p(0))];
    assert_eq!(said(unrelated, 0x200), None);
}

#[test]
fn a_method_record_under_a_constructors_name_states_the_constructor_of_its_type() {
    let ty = |key: TypeKey| TypeRow {
        name: key.0,
        ..TypeRow::default()
    };
    // A record of the second type, with code at `at`, that takes `params` cells and returns none.
    let record = |name: u32, at: u32, params: u32| MethodRow {
        name,
        at: Some(at),
        params,
        owner: 1,
        ..MethodRow::default()
    };
    let program = Program {
        program: PROGRAM,
        types: vec![ty(T1), ty(T2)],
        methods: vec![
            record(CONSTRUCTOR, 0x200, 2),
            record(STATIC_CONSTRUCTOR, 0x300, 0),
            // Another name; no code in this program; a slot filled; cells returned; no receiver taken.
            record(CONSTRUCTOR ^ 1, 0x400, 1),
            MethodRow {
                at: None,
                ..record(CONSTRUCTOR, 0, 1)
            },
            MethodRow {
                slot: Some(3),
                ..record(CONSTRUCTOR, 0x500, 1)
            },
            MethodRow {
                returns: 1,
                ..record(CONSTRUCTOR, 0x600, 1)
            },
            record(CONSTRUCTOR, 0x700, 0),
            // A record of a type the program states no name for.
            MethodRow {
                owner: 9,
                ..record(CONSTRUCTOR, 0x800, 1)
            },
        ],
        ..Program::default()
    };
    let mut project = Project::default();
    of_program(&mut project, &program);
    settle(&mut project);

    let of_static = Member {
        is_static: true,
        ..provisional(T2, Role::StaticConstructor)
    };
    let expected = BTreeMap::from([
        (key(0x200), provisional(T2, Role::Constructor)),
        (key(0x300), of_static),
    ]);
    assert_eq!(project.members, expected);
}

#[test]
fn a_constructor_a_record_states_stands_where_only_derived_constructors_run_it() {
    // No object of T1 is ever made: only the record says the function at 0x200 constructs one.
    let program = Program {
        program: PROGRAM,
        types: vec![TypeRow {
            name: T1.0,
            ..TypeRow::default()
        }],
        methods: vec![MethodRow {
            name: CONSTRUCTOR,
            at: Some(0x200),
            params: 1,
            ..MethodRow::default()
        }],
        ..Program::default()
    };
    let run = ir::Stmt::Eval(call(0x400, vec![value(temp(1))]));
    let makes = blocks(
        0x500,
        Returns::Void,
        vec![make(temp(1), &vm_type(T2, 0)), run],
    );
    let mut project = Project::default();
    project.types.insert(OBJECT, TypeFacts::default());
    project.types.insert(T1, sized(OBJECT, 24, &[]));
    project.types.insert(T2, sized(T1, 40, &[]));
    of_program(&mut project, &program);
    for f in [makes, hands_on(0x400, 0x200, p(0))] {
        of_function(&mut project, &f);
    }
    settle(&mut project);

    let constructor = |ty: TypeKey| Some(provisional(ty, Role::Constructor));
    assert_eq!(project.members.get(&key(0x200)).cloned(), constructor(T1));
    assert_eq!(project.members.get(&key(0x400)).cloned(), constructor(T2));
}

#[test]
fn what_two_parts_gathered_of_receivers_is_settled_together() {
    let types = [(T1, sized(OBJECT, 24, &[(4, 0x100), (5, 0x110)]))];
    // One part holds the function and one member that hands it its receiver, the other a second member.
    let part = |functions: &[cfg::Function]| {
        let mut project = Project::default();
        for f in functions {
            of_function(&mut project, f);
        }
        project
    };
    let works = blocks(0x200, Returns::Void, vec![writes(8)]);
    let merged = |second: cfg::Function| {
        let mut project = Project::default();
        project.types.insert(OBJECT, TypeFacts::default());
        project.types.extend(types.iter().cloned());
        let first = [hands_on(0x100, 0x200, p(0)), works.clone()];
        for mut part in [part(&first), part(&[second])] {
            absorb(&mut project, &mut part);
        }
        settle(&mut project);
        project.members.get(&key(0x200)).cloned()
    };

    assert_eq!(merged(hands_on(0x110, 0x200, p(0))), Some(method(T1, None)));
    // What the second part saw refuses it, whichever part holds the function itself.
    assert_eq!(merged(hands_on(0x300, 0x200, p(0))), None);
    assert_eq!(merged(hands_on(0x110, 0x200, p(1))), None);
}

#[test]
fn the_runtimes_own_type_under_a_class_is_sized_by_the_runtimes_table() {
    // The project states no row for the builtin type the class is made from.
    let mut project = Project::default();
    project.types.insert(T1, sized(OBJECT, 24, &[]));
    assert!(declares(&project, T1, key(0x200), Some(12)));
    // Nothing reached, the header alone, a byte past the object, a function of another program.
    assert!(!declares(&project, T1, key(0x200), None));
    assert!(!declares(&project, T1, key(0x200), Some(HEADER)));
    assert!(!declares(&project, T1, key(0x200), Some(HEADER + 25)));
    assert!(!declares(
        &project,
        T1,
        FunctionKey::new(PROGRAM + 1, 0x200),
        Some(12)
    ));
    // A value type's method takes a pointer, and a type that states no base is no class.
    let of_struct = TypeFacts {
        flags: STRUCT,
        ..sized(OBJECT, 24, &[])
    };
    project.types.insert(T2, of_struct);
    assert!(!declares(&project, T2, key(0x200), Some(12)));
    let rootless = TypeFacts {
        byte_size: 24,
        ..facts(None, &[])
    };
    project.types.insert(T3, rootless);
    assert!(!declares(&project, T3, key(0x200), Some(12)));
}

#[test]
fn a_slot_of_an_interface_reads_as_a_method_on_a_receiver_declared_to_hold_it() {
    let mut project = Project::default();
    // T2 is an interface: it states no base. T1 declares a reference to one at byte 0x10.
    project.types.insert(T1, facts(Some(OBJECT), &[]));
    project.types.insert(T2, facts(None, &[]));
    let field = |offset: u32, kind: Kind| FieldFacts {
        offset,
        bytes: 8,
        kind,
    };
    let fields = vec![
        field(0x10, Kind::Object(T2.to_string())),
        field(0x18, Kind::Object(T3.to_string())),
        field(0x20, Kind::Unknown),
    ];
    project.fields.insert(T1, fields);
    let returns = |kind: Kind| Signature {
        returns: 1,
        result: kind,
        ..Signature::default()
    };
    let of_interface = returns(Kind::Object(T2.to_string()));
    project.signatures.insert(key(0x300), of_interface);
    project
        .signatures
        .insert(key(0x310), returns(Kind::Unknown));

    let load = |offset: u32| Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(p(0)),
        offset,
    };
    // A load of a field of a T1, as the fields pass reads it.
    let field_of = |offset: u32| Expr::sugar(ExprSugar::Field { owner: T1 }, load(offset));
    let on = |receiver: Expr| virtual_call(T2, 3, vec![receiver, p(1)]);
    let as_method = |e: Expr| Expr::sugar(ExprSugar::Method, e);

    for receiver in [field_of(0x10), call(0x300, vec![p(1)])] {
        let body = [eval(on(receiver.clone()))];
        let done = ran(&body, &project, Options::default());
        assert_eq!(done, [eval(as_method(on(receiver)))]);
        // With the name off the call stays as it was.
        let off = Options::default().without(Pass::Receivers);
        assert_eq!(ran(&body, &project, off), body);
    }
    // The reading comes off again, with the field's.
    let done = ran(&[eval(on(field_of(0x10)))], &project, Options::default());
    assert_eq!(tree::desugar(done), [eval(on(load(0x10)))]);

    // A field declared to hold another type or none, a load no type declares, what a function of no declared result returns.
    for receiver in [
        field_of(0x18),
        field_of(0x20),
        load(0x10),
        call(0x310, vec![p(1)]),
    ] {
        let body = [eval(on(receiver))];
        assert_eq!(ran(&body, &project, Options::default()), body);
    }
}

#[test]
fn a_slot_a_names_list_names_reads_as_a_method_on_its_receiver() {
    let mut project = Project::default();
    // T2 is an interface, and T3 one made from it that the list names no slot of.
    project.types.insert(T2, facts(None, &[]));
    project.types.insert(T3, facts(Some(T2), &[]));
    let mut names = Names::default();
    names.methods.insert((T2, 3), "Wobble".to_owned());
    let cx = Context::new(&names, &project, Options::default());
    let read = |e: Expr| {
        let mut f = function(0x100, 2, vec![eval(e)]);
        run(&mut f, &cx, &mut Stats::default());
        f.body
    };

    let listed = virtual_call(T2, 3, vec![p(1)]);
    let as_method = eval(Expr::sugar(ExprSugar::Method, listed.clone()));
    assert_eq!(read(listed), [as_method]);
    // The slot beside it has a number alone, which names nothing on a receiver of no declared type.
    let numbered = virtual_call(T2, 4, vec![p(1)]);
    assert_eq!(read(numbered.clone()), [eval(numbered)]);
    // A name is looked for on the bases of the type a call names, as the printer looks for it.
    assert!(named(&cx, T3, 3));
    assert!(!named(&cx, T3, 4));
}
