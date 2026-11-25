use super::*;
use crate::gta6::script::ir::{Const, Head, Returns, StoreKind, VmType};
use crate::gta6::script::{Names, Options, Project};

const PROGRAM: u32 = 0x1122_3344;
/// The index the made-up runtime of these tests gives its delegate type.
const DELEGATE: u32 = 2;
/// A class with fields that holds references, as a listing flags one.
const CLASS: u32 = 0x22;

fn head() -> Head {
    Head {
        program: PROGRAM,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 4,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

fn delegate_type() -> VmType {
    VmType {
        name: "Delegate".to_owned(),
        byte_size: 48,
        id: BUILTIN | DELEGATE,
        flags: 0x2000,
    }
}

/// A class of the program's own with the flag word given.
fn class(flags: u32) -> VmType {
    VmType {
        name: "type_00000000000000C1".to_owned(),
        byte_size: 24,
        id: 7,
        flags,
    }
}

fn var(v: Var) -> Expr {
    Expr::Var(v)
}

fn global(offset: u32) -> Var {
    Var::Global {
        program: PROGRAM,
        offset,
        statics: false,
    }
}

fn function(at: u32) -> Expr {
    Expr::FunctionRef {
        program: PROGRAM,
        at,
    }
}

fn assign(target: Var, value: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Assign { target, value })
}

fn eval(e: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(e))
}

/// A call of the function at `at` of the program itself.
fn call(at: u32, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args,
        returns: Some(0),
    }
}

/// The constructor call of the builtin type `ty`, given the object, a target and a function.
fn bind(ty: u32, object: Var, target: Expr, function: Expr) -> ir::Stmt {
    ir::Stmt::Eval(Expr::Call {
        callee: Callee::Builtin {
            path: "Delegate.Ctor".to_owned(),
            ty,
            method: 0,
        },
        args: vec![var(object), target, function],
        returns: Some(0),
    })
}

fn made_delegate(held: Var) -> ir::Stmt {
    ir::Stmt::Assign {
        target: held,
        value: Expr::NewObject(delegate_type()),
    }
}

/// `held = new Delegate(); Ctor(held, target, function);`
fn pair(held: Var, target: Expr, function: Expr) -> Vec<Stmt> {
    vec![
        Stmt::Simple(made_delegate(held)),
        Stmt::Simple(bind(DELEGATE, held, target, function)),
    ]
}

/// A closure object of a class with the flag word given, made in `held` and bound to the function at 0x300.
fn closure(held: Var, flags: u32) -> Vec<Stmt> {
    let mut body = vec![
        assign(held, Expr::NewObject(class(flags))),
        Stmt::Simple(ir::Stmt::Store {
            kind: StoreKind::I32,
            addr: var(held),
            offset: 0x10,
            value: var(Var::Param(1)),
        }),
    ];
    body.extend(pair(Var::Local(3), var(held), function(0x300)));
    body
}

/// `if (cache != 0) { join = cache; } else { made }`, as a value made on first use is kept.
fn lazily(cache: Var, kept: Vec<Stmt>, made: Vec<Stmt>) -> Vec<Stmt> {
    let tested = Expr::cast(Scalar::U64, var(cache));
    vec![Stmt::If {
        cond: Expr::binary(BinOp::Ne, tested, Expr::Const(Const::i32(0))),
        then: kept,
        els: made,
    }]
}

const JOIN: Var = Var::Stack { at: 0x50, index: 0 };
const HOLDER: u32 = 0x60;
const CACHE: u32 = 0x68;

/// The arm that makes a delegate bound to `target` and `function`, keeps it in the global at `kept` and copies it on.
fn made_arm(target: Expr, function: Expr, kept: u32) -> Vec<Stmt> {
    let d = Var::Value { at: 0x44, index: 0 };
    let dead = Var::Stack { at: 0x40, index: 0 };
    let mut arm = vec![assign(dead, var(global(CACHE)))];
    arm.extend(pair(d, target, function));
    arm.push(assign(global(kept), Expr::cast(Scalar::U64, var(d))));
    arm.push(assign(JOIN, var(d)));
    arm
}

fn kept_arm() -> Vec<Stmt> {
    vec![assign(JOIN, var(global(CACHE)))]
}

fn holder() -> Expr {
    Expr::cast(Scalar::U64, var(global(HOLDER)))
}

fn rewritten(body: Vec<Stmt>, passes: &[Pass]) -> (Vec<Stmt>, Found) {
    let names = Names::default();
    let options = passes
        .iter()
        .fold(Options::baseline(), |options, pass| options.with(*pass));
    let mut f = tree::Function { head: head(), body };
    let found = rewrite(&mut f, &Context::new(&names, Project::empty(), options));
    (f.body, found)
}

/// The offsets of the functions a body reads as a lambda's body, in the order they stand.
fn lambdas(body: &[Stmt]) -> Vec<u32> {
    let mut found = Vec::new();
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Sugar {
                kind: ExprSugar::Lambda,
                e: inner,
            } = e
            {
                if let Expr::FunctionRef { at, .. } = &**inner {
                    found.push(*at);
                }
            }
        });
    });
    found
}

fn groups(body: &[Stmt]) -> Vec<Group> {
    body.iter()
        .filter_map(|stmt| match stmt {
            Stmt::Sugar(sugar) => match &**sugar {
                Sugar::Group { kind, .. } => Some(*kind),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_delegate_made_and_bound_is_one_group_that_lowers_to_the_two_statements() {
    let d = Var::Local(0);
    let mut body = pair(d, var(Var::Param(0)), function(0x300));
    body.push(assign(Var::Local(1), var(d)));
    let (done, found) = rewritten(body.clone(), &[Pass::Delegates]);
    assert_eq!(
        found,
        Found {
            delegates: 1,
            lambdas: 0,
            cached: 0
        }
    );
    assert_eq!(done.len(), 2, "{done:?}");
    assert_eq!(groups(&done), vec![Group::Delegate]);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn a_delegate_in_a_nested_body_is_grouped_too() {
    let inner = pair(Var::Local(0), var(Var::Param(0)), function(0x300));
    let body = vec![Stmt::While {
        cond: var(Var::Param(1)),
        body: vec![Stmt::If {
            cond: var(Var::Param(1)),
            then: inner,
            els: Vec::new(),
        }],
    }];
    let (done, found) = rewritten(body.clone(), &[Pass::Delegates]);
    assert_eq!(found.delegates, 1);
    assert_ne!(done, body);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn statements_that_are_not_a_delegate_made_and_bound_stay_as_they_are() {
    let (d, other) = (Var::Local(0), Var::Local(1));
    let target = || var(Var::Param(0));
    let two = |alloc: ir::Stmt, ctor: ir::Stmt| vec![Stmt::Simple(alloc), Stmt::Simple(ctor)];
    let apart = {
        let mut body = pair(d, target(), function(0x300));
        body.insert(1, eval(call(0x100, vec![])));
        body
    };
    let object = ir::Stmt::Assign {
        target: d,
        value: Expr::NewObject(class(CLASS)),
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a builtin of another type than the object's",
            two(
                made_delegate(d),
                bind(DELEGATE + 1, d, target(), function(0x300)),
            ),
        ),
        (
            "an object of a type that is no builtin",
            two(object, bind(7, d, target(), function(0x300))),
        ),
        (
            "no function bound",
            two(
                made_delegate(d),
                bind(DELEGATE, d, target(), var(Var::Param(1))),
            ),
        ),
        (
            "a target that reads the delegate",
            two(made_delegate(d), bind(DELEGATE, d, var(d), function(0x300))),
        ),
        (
            "another object than the one made",
            two(
                made_delegate(d),
                bind(DELEGATE, other, target(), function(0x300)),
            ),
        ),
        ("a statement between the two", apart),
    ];
    for (what, body) in cases {
        let (done, found) = rewritten(body.clone(), &[Pass::Delegates, Pass::Lambdas]);
        assert_eq!(found, Found::default(), "{what}");
        assert_eq!(done, body, "{what}");
    }
}

#[test]
fn a_function_bound_to_a_closure_object_made_here_reads_as_a_lambda() {
    let body = closure(Var::Local(0), GENERATED | CLASS);
    let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
    assert_eq!(
        found,
        Found {
            delegates: 0,
            lambdas: 1,
            cached: 0
        }
    );
    assert_eq!(lambdas(&done), vec![0x300]);
    assert!(groups(&done).is_empty(), "{done:?}");
    assert_eq!(tree::desugar(done), body);

    // With both names on, the reading stands inside the group.
    let (done, found) = rewritten(body.clone(), &[Pass::Lambdas, Pass::Delegates]);
    assert_eq!(
        found,
        Found {
            delegates: 1,
            lambdas: 1,
            cached: 0
        }
    );
    assert_eq!(lambdas(&done), vec![0x300]);
    assert_eq!(groups(&done), vec![Group::Delegate]);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn a_function_bound_to_anything_but_a_proved_closure_object_stays_a_reference() {
    let c = Var::Local(0);
    let twice = {
        let mut body = closure(c, GENERATED | CLASS);
        body.insert(1, assign(c, var(Var::Param(0))));
        body
    };
    let exposed = {
        let mut body = closure(c, GENERATED | CLASS);
        body.insert(1, eval(call(0x100, vec![Expr::AddrOf { var: c, cell: 0 }])));
        body
    };
    let late = {
        let slot = Expr::VirtualFunctionRef {
            receiver: Box::new(var(c)),
            ty: class(CLASS),
            slot: 3,
        };
        let mut body = vec![assign(c, Expr::NewObject(class(GENERATED | CLASS)))];
        body.extend(pair(Var::Local(3), var(c), slot));
        body
    };
    let elsewhere = {
        let mut body = vec![assign(c, Expr::NewObject(class(GENERATED | CLASS)))];
        body.extend(pair(Var::Local(3), var(Var::Local(1)), function(0x300)));
        body
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        ("a class someone wrote", closure(c, CLASS)),
        ("a value type", closure(c, GENERATED | 0x04)),
        (
            "a parameter, which held a value before",
            closure(Var::Param(0), GENERATED | CLASS),
        ),
        ("a variable assigned twice", twice),
        ("a variable whose address is taken", exposed),
        ("a virtual slot, which is no function of a program", late),
        ("another object than the closure", elsewhere),
    ];
    for (what, body) in cases {
        let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
        assert_eq!(found, Found::default(), "{what}");
        assert_eq!(done, body, "{what}");
    }
}

#[test]
fn a_function_bound_to_a_holder_and_kept_in_a_global_reads_as_a_lambda() {
    let cache = global(CACHE);
    let made = made_arm(holder(), function(0x400), CACHE);
    let body = lazily(cache, kept_arm(), made.clone());
    let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
    assert_eq!(
        found,
        Found {
            delegates: 0,
            lambdas: 0,
            cached: 1
        }
    );
    assert_eq!(lambdas(&done), vec![0x400]);
    assert_eq!(tree::desugar(done), body);

    // Each arm may carry another value to where they meet, as the stack does across a branch.
    let carried = assign(Var::Stack { at: 0x50, index: 1 }, var(Var::Param(1)));
    let with = |mut arm: Vec<Stmt>| {
        arm.insert(0, carried.clone());
        arm
    };
    let body = lazily(cache, with(kept_arm()), with(made.clone()));
    let (done, found) = rewritten(body, &[Pass::Lambdas]);
    assert_eq!(found.cached, 1);
    assert_eq!(lambdas(&done), vec![0x400]);

    // The same with the test turned round and the arms with it, and with nothing kept apart.
    let tested = Expr::cast(Scalar::U64, var(cache));
    let turned = vec![Stmt::If {
        cond: Expr::Unary {
            op: UnOp::Not,
            e: Box::new(Expr::binary(BinOp::Ne, tested, Expr::Const(Const::i32(0)))),
        },
        then: made,
        els: Vec::new(),
    }];
    let (done, found) = rewritten(turned.clone(), &[Pass::Lambdas, Pass::Delegates]);
    assert_eq!(
        found,
        Found {
            delegates: 1,
            lambdas: 0,
            cached: 1
        }
    );
    assert_eq!(lambdas(&done), vec![0x400]);
    assert_eq!(tree::desugar(done), turned);
}

#[test]
fn a_cache_that_is_not_the_compilers_own_form_leaves_the_function_a_reference() {
    let cache = global(CACHE);
    let plain = || made_arm(holder(), function(0x400), CACHE);
    let more = |mut arm: Vec<Stmt>| {
        arm.push(eval(call(0x100, vec![])));
        arm
    };
    let foreign = Expr::FunctionRef {
        program: PROGRAM + 1,
        at: 0x400,
    };
    let above = vec![Stmt::If {
        cond: Expr::binary(BinOp::Gt, var(cache), Expr::Const(Const::i32(0))),
        then: kept_arm(),
        els: plain(),
    }];
    let calls = |mut arm: Vec<Stmt>| {
        arm.insert(0, assign(Var::Local(2), call(0x100, vec![])));
        arm
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "another statement where it is made",
            lazily(cache, kept_arm(), more(plain())),
        ),
        (
            "a value carried that is a call's",
            lazily(cache, calls(kept_arm()), plain()),
        ),
        (
            "another statement where it is kept",
            lazily(cache, more(kept_arm()), plain()),
        ),
        (
            "kept in another global than the one tested",
            lazily(cache, kept_arm(), made_arm(holder(), function(0x400), 0x70)),
        ),
        (
            "bound to no global",
            lazily(
                cache,
                kept_arm(),
                made_arm(var(Var::Param(0)), function(0x400), CACHE),
            ),
        ),
        (
            "bound to the cache itself",
            lazily(
                cache,
                kept_arm(),
                made_arm(var(cache), function(0x400), CACHE),
            ),
        ),
        (
            "a function of another program",
            lazily(cache, kept_arm(), made_arm(holder(), foreign, CACHE)),
        ),
        (
            "made where the global is not null",
            lazily(cache, plain(), kept_arm()),
        ),
        ("a test that is no test against null", above),
    ];
    for (what, body) in cases {
        let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
        assert_eq!(found, Found::default(), "{what}");
        assert_eq!(done, body, "{what}");
    }
}

#[test]
fn a_pair_the_constructors_pass_gathered_is_read_as_the_delegate_it_is() {
    let gathered = |parts: Vec<ir::Stmt>| {
        Stmt::Sugar(Box::new(Sugar::Group {
            kind: Group::Construct,
            parts,
        }))
    };
    let c = Var::Local(0);
    let d = Var::Local(3);
    let object = ir::Stmt::Assign {
        target: d,
        value: Expr::NewObject(class(CLASS)),
    };
    let body = vec![
        assign(c, Expr::NewObject(class(GENERATED | CLASS))),
        gathered(vec![
            made_delegate(d),
            bind(DELEGATE, d, var(c), function(0x300)),
        ]),
        // An object of the program's own and its constructor: no delegate.
        gathered(vec![object, ir::Stmt::Eval(call(0x200, vec![var(d)]))]),
    ];
    let (done, found) = rewritten(body.clone(), &[Pass::Delegates, Pass::Lambdas]);
    assert_eq!(
        found,
        Found {
            delegates: 1,
            lambdas: 1,
            cached: 0
        }
    );
    assert_eq!(groups(&done), vec![Group::Delegate, Group::Construct]);
    assert_eq!(lambdas(&done), vec![0x300]);
    assert_eq!(tree::desugar(done), tree::desugar(body.clone()));

    // With `delegates` off the group keeps the reading it came with.
    let (done, found) = rewritten(body, &[Pass::Lambdas]);
    assert_eq!(found.delegates, 0);
    assert_eq!(groups(&done), vec![Group::Construct, Group::Construct]);
    assert_eq!(lambdas(&done), vec![0x300]);
}

#[test]
fn nothing_is_rewritten_where_the_names_are_off() {
    let mut body = closure(Var::Local(0), GENERATED | CLASS);
    body.extend(lazily(
        global(CACHE),
        kept_arm(),
        made_arm(holder(), function(0x400), CACHE),
    ));
    let off: [&[Pass]; 2] = [&[], &[Pass::LocalFunctions, Pass::Events]];
    for passes in off {
        let (done, found) = rewritten(body.clone(), passes);
        assert_eq!(found, Found::default(), "{passes:?}");
        assert_eq!(done, body, "{passes:?}");
    }
    // `delegates` alone groups and reads no function as a lambda.
    let (done, found) = rewritten(body.clone(), &[Pass::Delegates]);
    assert_eq!(
        found,
        Found {
            delegates: 2,
            lambdas: 0,
            cached: 0
        }
    );
    assert!(lambdas(&done).is_empty(), "{done:?}");
}

#[test]
fn a_body_is_sole_only_where_its_program_names_it_once() {
    let lambda = |at: u32| Expr::sugar(ExprSugar::Lambda, function(at));
    let bound = |n: u32, function: Expr| {
        Stmt::Simple(bind(DELEGATE, Var::Local(n), var(Var::Param(0)), function))
    };
    let elsewhere = Expr::FunctionRef {
        program: PROGRAM + 1,
        at: 0x300,
    };
    let first = tree::Function {
        head: head(),
        body: vec![
            bound(0, lambda(0x300)),
            bound(1, lambda(0x400)),
            bound(2, lambda(0x500)),
            // Another program's function at the same offset is another function.
            bound(3, elsewhere),
            // A reference no pass read as a lambda is no body to write in place.
            bound(4, function(0x600)),
        ],
    };
    let second = tree::Function {
        head: head(),
        body: vec![Stmt::While {
            cond: var(Var::Param(1)),
            body: vec![eval(call(0x400, vec![])), bound(0, lambda(0x500))],
        }],
    };
    assert_eq!(sole(&[first, second]), BTreeSet::from([0x300]));
}
