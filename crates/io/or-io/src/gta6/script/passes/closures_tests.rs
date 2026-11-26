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
            cached: 0,
            ..Found::default()
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
            cached: 0,
            ..Found::default()
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
            cached: 0,
            ..Found::default()
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
            cached: 1,
            ..Found::default()
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
            cached: 1,
            ..Found::default()
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
            cached: 0,
            ..Found::default()
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
            cached: 0,
            ..Found::default()
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

const P0: Var = Var::Param(0);
const P1: Var = Var::Param(1);
const CUR: Var = Var::Local(0);
const OLD: Var = Var::Local(1);
/// The names a list gives three methods of the made-up runtime of these tests.
const COMBINE: &str = "Demo.Delegate.Combine";
const REMOVE: &str = "Demo.Delegate.Remove";
const EXCHANGE: &str = "Demo.Cells.CompareExchange";

/// A call of a method of the made-up runtime, under the name given.
fn builtin_call(path: &str, method: u32, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 9,
            method,
        },
        args,
        returns: Some(1),
    }
}

/// The reference a field of an object holds.
fn field(object: Var, offset: u32) -> Expr {
    Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(var(object)),
        offset,
    }
}

fn field_address(object: Var, offset: i32) -> Expr {
    Expr::ByteOffset {
        addr: Box::new(var(object)),
        offset: Box::new(Expr::Const(Const::i32(offset))),
    }
}

fn differ(a: Var, b: Var) -> Expr {
    Expr::binary(
        BinOp::Ne,
        Expr::cast(Scalar::I64, var(a)),
        Expr::cast(Scalar::I64, var(b)),
    )
}

/// The pieces of an event's loop over `CUR` and `OLD`, each of which a test may change.
struct Subscribed {
    read: Expr,
    place: Expr,
    held: Expr,
    handler: Expr,
    comparand: Expr,
    join: &'static str,
    exchange: &'static str,
    test: Expr,
}

/// `CUR = p0.f; do { OLD = CUR; CUR = Exchange(&p0.f, Combine(OLD, p1), OLD); } while (CUR != OLD);`
fn subscribed() -> Subscribed {
    Subscribed {
        read: field(P0, 0x30),
        place: field_address(P0, 0x30),
        held: var(OLD),
        handler: var(P1),
        comparand: var(OLD),
        join: COMBINE,
        exchange: EXCHANGE,
        test: differ(CUR, OLD),
    }
}

impl Subscribed {
    fn body(self) -> Vec<Stmt> {
        let joined = builtin_call(self.join, 5, vec![self.held, self.handler]);
        let swap = builtin_call(self.exchange, 1, vec![self.place, joined, self.comparand]);
        vec![
            assign(CUR, self.read),
            Stmt::DoWhile {
                body: vec![assign(OLD, var(CUR)), assign(CUR, swap)],
                cond: self.test,
            },
        ]
    }
}

/// The one construct a body is, where it is one.
fn construct(body: &[Stmt]) -> &Sugar {
    match body {
        [Stmt::Sugar(sugar)] => sugar,
        other => panic!("no one construct: {other:?}"),
    }
}

#[test]
fn an_events_loop_is_one_construct_that_lowers_to_the_read_and_the_loop() {
    for (join, remove) in [(COMBINE, false), (REMOVE, true)] {
        let body = Subscribed {
            join,
            ..subscribed()
        }
        .body();
        let (done, found) = rewritten(body.clone(), &[Pass::Events]);
        assert_eq!(
            found,
            Found {
                events: 1,
                ..Found::default()
            }
        );
        let Sugar::Event { seed, round, again } = construct(&done) else {
            panic!("{done:?}");
        };
        let read = subscription(seed, round, again).expect("an event");
        assert_eq!(read.place, &field(P0, 0x30));
        assert_eq!(read.handler, &var(P1));
        assert_eq!(read.remove, remove);
        assert_eq!(tree::desugar(done), body);
    }
}

#[test]
fn a_loop_with_the_join_and_the_result_in_variables_of_their_own_is_an_event_too() {
    let object = Var::Value { at: 0x10, index: 0 };
    let (copy, cur, old, joined) = (Var::Local(0), Var::Local(1), Var::Local(2), Var::Local(3));
    let result = Var::Value { at: 0x20, index: 0 };
    let exchange = vec![field_address(copy, 0x20), var(joined), var(old)];
    let body = vec![
        assign(object, var(P0)),
        // The exchange goes through a copy of the object the read goes through.
        assign(copy, var(object)),
        assign(cur, field(object, 0x20)),
        Stmt::DoWhile {
            body: vec![
                assign(old, var(cur)),
                assign(joined, builtin_call(REMOVE, 6, vec![var(cur), var(P1)])),
                assign(result, builtin_call(EXCHANGE, 1, exchange)),
                assign(cur, var(result)),
            ],
            cond: differ(result, old),
        },
        eval(call(0x100, vec![var(copy)])),
    ];
    let (done, found) = rewritten(body.clone(), &[Pass::Events]);
    assert_eq!(found.events, 1);
    assert_eq!(done.len(), 4, "{done:?}");
    let Some(Stmt::Sugar(sugar)) = done.get(2) else {
        panic!("{done:?}");
    };
    let Sugar::Event { seed, round, again } = &**sugar else {
        panic!("{sugar:?}");
    };
    let read = subscription(seed, round, again).expect("an event");
    assert_eq!(read.handler, &var(P1));
    assert!(read.remove);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn loops_that_share_their_variables_are_events_while_nothing_else_names_them() {
    let two = || {
        let mut body = subscribed().body();
        body.extend(
            Subscribed {
                join: REMOVE,
                ..subscribed()
            }
            .body(),
        );
        body
    };
    let (done, found) = rewritten(two(), &[Pass::Events]);
    assert_eq!(found.events, 2);
    assert_eq!(done.len(), 2, "{done:?}");
    assert_eq!(tree::desugar(done), two());

    // One read of a variable of the loops after them: what a loop left there is asked for.
    let mut read_after = two();
    read_after.push(eval(call(0x100, vec![var(OLD)])));
    let (done, found) = rewritten(read_after.clone(), &[Pass::Events]);
    assert_eq!(found, Found::default());
    assert_eq!(done, read_after);
}

#[test]
fn a_loop_that_is_not_an_events_whole_form_stays_a_loop() {
    let handler = Var::Local(2);
    let exposed = {
        let mut body = Subscribed {
            handler: var(handler),
            ..subscribed()
        }
        .body();
        let address = Expr::AddrOf {
            var: handler,
            cell: 0,
        };
        body.insert(0, eval(call(0x100, vec![address])));
        body
    };
    let between = {
        let mut body = subscribed().body();
        if let Some(Stmt::DoWhile { body: round, .. }) = body.get_mut(1) {
            round.insert(1, eval(call(0x100, vec![])));
        }
        body
    };
    let stale = {
        // The next round would start from what the first read, whatever the exchange returned.
        let result = Var::Local(3);
        let joined = builtin_call(COMBINE, 5, vec![var(OLD), var(P1)]);
        let swap = builtin_call(EXCHANGE, 1, vec![field_address(P0, 0x30), joined, var(OLD)]);
        vec![
            assign(CUR, field(P0, 0x30)),
            Stmt::DoWhile {
                body: vec![assign(OLD, var(CUR)), assign(result, swap)],
                cond: differ(result, OLD),
            },
        ]
    };
    let same = Expr::binary(
        BinOp::Eq,
        Expr::cast(Scalar::I64, var(CUR)),
        Expr::cast(Scalar::I64, var(OLD)),
    );
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "another field read than exchanged",
            Subscribed {
                read: field(P0, 0x38),
                ..subscribed()
            }
            .body(),
        ),
        (
            "another object read than exchanged",
            Subscribed {
                read: field(Var::Local(2), 0x30),
                ..subscribed()
            }
            .body(),
        ),
        (
            "a handler that is a call, which the loop makes each round",
            Subscribed {
                handler: call(0x100, vec![]),
                ..subscribed()
            }
            .body(),
        ),
        ("a handler whose address is handed out", exposed),
        (
            "the handler joined to something else than what was read",
            Subscribed {
                held: var(P1),
                ..subscribed()
            }
            .body(),
        ),
        (
            "exchanged against something else than what was read",
            Subscribed {
                comparand: var(P1),
                ..subscribed()
            }
            .body(),
        ),
        (
            "a test that is no difference",
            Subscribed {
                test: same,
                ..subscribed()
            }
            .body(),
        ),
        ("a call between the copy and the exchange", between),
        ("a round that does not start from the exchange", stale),
        (
            "a join no list names",
            Subscribed {
                join: "type_80000009.Method_5",
                ..subscribed()
            }
            .body(),
        ),
        (
            "an exchange no list names",
            Subscribed {
                exchange: "type_80000009.Method_1",
                ..subscribed()
            }
            .body(),
        ),
        (
            "a join of another type than the delegate's",
            Subscribed {
                join: "Demo.Cells.Combine",
                ..subscribed()
            }
            .body(),
        ),
    ];
    for (what, body) in cases {
        let (done, found) = rewritten(body.clone(), &[Pass::Events]);
        assert_eq!(found, Found::default(), "{what}");
        assert_eq!(done, body, "{what}");
    }
    // And the whole form stays a loop where the name is off.
    let body = subscribed().body();
    let (done, found) = rewritten(body.clone(), &[Pass::Lambdas, Pass::Delegates]);
    assert_eq!(found, Found::default());
    assert_eq!(done, body);
}

/// The variable the delegate that binds the function at `bound` is made in.
fn delegate_of(bound: u32) -> Var {
    Var::Value {
        at: bound,
        index: 0,
    }
}

/// `d = new Delegate(); Ctor(d, holder, function); cache = d;`, then the copy to where the arms meet.
fn making(cache: u32, bound: u32, d: Var, join: Option<Var>) -> Vec<Stmt> {
    let mut arm = pair(d, holder(), function(bound));
    arm.push(assign(global(cache), var(d)));
    arm.extend(join.map(|join| assign(join, var(d))));
    arm
}

/// `if (cache != 0) { JOIN = cache; } else { d = holder.function; cache = d; JOIN = d; }`
fn kept_form(cache: u32, bound: u32) -> Vec<Stmt> {
    let taken = vec![assign(JOIN, var(global(cache)))];
    let made = making(cache, bound, delegate_of(bound), Some(JOIN));
    lazily(global(cache), taken, made)
}

#[test]
fn a_delegate_kept_in_a_global_is_one_construct_that_lowers_to_the_test_and_its_arms() {
    let body = kept_form(CACHE, 0x400);
    let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
    assert_eq!(
        found,
        Found {
            cached: 1,
            kept: 1,
            ..Found::default()
        }
    );
    let Sugar::Cached {
        test,
        null,
        made: making_arm,
        kept,
    } = construct(&done)
    else {
        panic!("{done:?}");
    };
    assert!(!*null);
    let read = kept_delegate(test, making_arm, kept).expect("a kept delegate");
    assert_eq!(read.cache, global(CACHE));
    assert_eq!(read.target, &holder());
    assert_eq!(bare(read.function), &function(0x400));
    assert_eq!(read.take, Some(0));
    assert_eq!(lambdas(&done), vec![0x400]);
    assert_eq!(tree::desugar(done), body);

    // With `lambdas` off the test and its arms stay as they are.
    let (done, found) = rewritten(body.clone(), &[Pass::Delegates, Pass::Events]);
    assert_eq!(found.kept, 0);
    assert!(matches!(done.as_slice(), [Stmt::If { .. }]), "{done:?}");
}

#[test]
fn the_forms_a_kept_delegate_takes_are_all_read() {
    let cache = global(CACHE);
    let tested = Expr::cast(Scalar::U64, var(cache));
    let turned = vec![Stmt::If {
        cond: Expr::binary(BinOp::Eq, tested, Expr::Const(Const::i32(0))),
        then: making(CACHE, 0x400, delegate_of(0x400), Some(JOIN)),
        els: vec![assign(JOIN, var(cache))],
    }];
    let own = Var::Local(2);
    let carried = Var::Stack { at: 0x50, index: 1 };
    let with = |at: usize, value: Expr, mut arm: Vec<Stmt>| {
        arm.insert(at, assign(carried, value));
        arm
    };
    let taken = || vec![assign(JOIN, var(cache))];
    let made_arm = || making(CACHE, 0x400, delegate_of(0x400), Some(JOIN));
    let cases: Vec<(&str, Vec<Stmt>, Option<usize>)> = vec![
        ("made where the test holds", turned, Some(0)),
        (
            "taken by the delegate's own variable",
            lazily(
                cache,
                vec![assign(own, var(cache))],
                making(CACHE, 0x400, own, None),
            ),
            Some(0),
        ),
        (
            "taken by nothing",
            lazily(
                cache,
                Vec::new(),
                making(CACHE, 0x400, delegate_of(0x400), None),
            ),
            None,
        ),
        (
            "a field carried on the same side of the making in both arms",
            lazily(
                cache,
                with(0, field(P0, 8), taken()),
                with(0, field(P0, 8), made_arm()),
            ),
            Some(1),
        ),
        (
            "a variable carried on the other side of the making",
            lazily(
                cache,
                with(0, var(P1), taken()),
                with(3, var(P1), made_arm()),
            ),
            Some(1),
        ),
    ];
    for (what, body, take) in cases {
        let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
        assert_eq!(found.kept, 1, "{what}");
        let Sugar::Cached {
            test,
            made: making_arm,
            kept,
            ..
        } = construct(&done)
        else {
            panic!("{what}: {done:?}");
        };
        let read = kept_delegate(test, making_arm, kept);
        assert_eq!(read.map(|read| read.take), Some(take), "{what}");
        assert_eq!(tree::desugar(done), body, "{what}");
    }
}

#[test]
fn arms_that_differ_by_more_than_the_delegate_stay_a_test() {
    let cache = global(CACHE);
    let carried = Var::Stack { at: 0x50, index: 1 };
    let with = |at: usize, value: Expr, mut arm: Vec<Stmt>| {
        arm.insert(at, assign(carried, value));
        arm
    };
    let taken = || vec![assign(JOIN, var(cache))];
    let made_arm = || making(CACHE, 0x400, delegate_of(0x400), Some(JOIN));
    let read_after = {
        let mut body = kept_form(CACHE, 0x400);
        body.push(eval(call(0x100, vec![var(delegate_of(0x400))])));
        body
    };
    let twice = {
        let mut arm = taken();
        arm.push(assign(carried, var(cache)));
        arm
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "a field carried on the other side of the making",
            lazily(
                cache,
                with(0, field(P0, 8), taken()),
                with(3, field(P0, 8), made_arm()),
            ),
        ),
        (
            "another value carried in one arm than in the other",
            lazily(
                cache,
                with(0, var(P0), taken()),
                with(0, var(P1), made_arm()),
            ),
        ),
        (
            "a value carried in one arm alone",
            lazily(cache, taken(), with(0, var(P1), made_arm())),
        ),
        (
            "a carried value that reads what takes the delegate",
            lazily(
                cache,
                with(1, var(JOIN), taken()),
                with(4, var(JOIN), made_arm()),
            ),
        ),
        ("the delegate's variable read after the arms", read_after),
        ("the cache taken twice", lazily(cache, twice, made_arm())),
    ];
    for (what, body) in cases {
        let (done, found) = rewritten(body.clone(), &[Pass::Lambdas]);
        assert_eq!(found.kept, 0, "{what}");
        assert!(matches!(done.first(), Some(Stmt::If { .. })), "{what}");
        assert_eq!(tree::desugar(done), body, "{what}");
    }
}

#[test]
fn a_cache_is_private_only_where_its_program_names_it_in_its_constructs_alone() {
    let read = |body: Vec<Stmt>| {
        let (body, _) = rewritten(body, &[Pass::Lambdas]);
        tree::Function { head: head(), body }
    };
    let alone = read(kept_form(0x68, 0x400));
    let written = {
        let mut body = kept_form(0x70, 0x500);
        body.push(assign(global(0x70), Expr::Const(Const::i32(0))));
        read(body)
    };
    // Two delegates kept in one global, each of another function.
    let rebound = {
        let mut body = kept_form(0x78, 0x600);
        body.extend(kept_form(0x78, 0x700));
        read(body)
    };
    let kept_in = |f: &tree::Function| {
        let mut count = 0;
        tree::each_sugar(&f.body, &mut |sugar| {
            count += usize::from(matches!(sugar, Sugar::Cached { .. }));
        });
        count
    };
    assert_eq!(
        [&alone, &written, &rebound].map(kept_in),
        [1, 1, 2],
        "each is a construct: what tells them apart is who else names the global"
    );
    let functions = [alone.clone(), written, rebound];
    assert_eq!(private_caches(&functions), BTreeSet::from([global(0x68)]));

    // A holder assigned twice is not the one object every delegate was bound to.
    let holder_made = || assign(global(HOLDER), Expr::NewObject(class(GENERATED | CLASS)));
    let remade = tree::Function {
        head: head(),
        body: vec![holder_made(), holder_made()],
    };
    assert_eq!(private_caches(&[alone.clone(), remade]), BTreeSet::new());
    let once = tree::Function {
        head: head(),
        body: vec![holder_made()],
    };
    assert_eq!(
        private_caches(&[alone, once]),
        BTreeSet::from([global(0x68)])
    );
}

/// A function of two parameters, the first its closure object, with the body given.
fn lambda_body(returns: Returns, body: Vec<Stmt>) -> tree::Function {
    let head = Head { returns, ..head() };
    tree::Function { head, body }
}

#[test]
fn a_body_of_one_value_or_a_few_statements_is_written_where_it_is_bound() {
    let captured = || field(P0, 0x10);
    let value = call(0x100, vec![captured(), var(P1)]);
    let returned = lambda_body(Returns::Cell, vec![Stmt::Return(Some(value.clone()))]);
    assert_eq!(inline(&returned), Some(Inline::Value(&value)));

    let called = lambda_body(Returns::Void, vec![eval(value.clone()), Stmt::Return(None)]);
    assert_eq!(inline(&called), Some(Inline::Value(&value)));

    let few = vec![
        assign(Var::Local(0), captured()),
        eval(call(0x100, vec![var(Var::Local(0))])),
        eval(call(0x200, vec![var(P1)])),
    ];
    let mut ended = few.clone();
    ended.push(Stmt::Return(None));
    let block = lambda_body(Returns::Void, ended);
    assert_eq!(inline(&block), Some(Inline::Block(&few)));

    // ⚠ A value that names a variable of the body's own frame keeps the block that declares it.
    let scratch = Expr::AddrOf {
        var: Var::Local(0),
        cell: 0,
    };
    let filled = call(0x100, vec![var(P1), scratch]);
    let one = vec![Stmt::Return(Some(filled.clone()))];
    let returned = lambda_body(Returns::Cell, one.clone());
    assert_eq!(inline(&returned), Some(Inline::Block(&one)));
    let one = vec![eval(filled)];
    let called = lambda_body(Returns::Void, one.clone());
    assert_eq!(inline(&called), Some(Inline::Block(&one)));
}

#[test]
fn a_body_that_is_long_or_writes_its_closure_object_is_not_written_in_place() {
    let step = || eval(call(0x100, vec![var(P1)]));
    let long: Vec<Stmt> = (0..=BLOCK).map(|_| step()).collect();
    let rebound = vec![assign(P0, var(P1)), step()];
    let address = Expr::AddrOf { var: P0, cell: 0 };
    let exposed = vec![eval(call(0x100, vec![address]))];
    let jumps = vec![
        Stmt::Label(tree::Label::At(0x90)),
        step(),
        step(),
        Stmt::Goto(tree::Label::At(0x90)),
    ];
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        ("more statements than a block holds", long),
        ("the closure object assigned", rebound),
        ("the closure object's address handed out", exposed),
        ("a jump", jumps),
    ];
    for (what, body) in cases {
        assert_eq!(inline(&lambda_body(Returns::Void, body)), None, "{what}");
    }
    let mut free = lambda_body(Returns::Void, vec![step()]);
    free.head.params = 0;
    assert_eq!(inline(&free), None, "no parameter for a closure object");
    let mut unread = lambda_body(Returns::Void, vec![step()]);
    unread.head.incomplete = Some("not read".to_owned());
    assert_eq!(inline(&unread), None, "a body the listing does not hold");
}

#[test]
fn a_body_may_be_written_in_place_where_it_is_bound_once_outside_every_loop() {
    let bound = |body: Vec<Stmt>| {
        let (body, _) = rewritten(body, &[Pass::Lambdas, Pass::Delegates]);
        inlinable(&tree::Function { head: head(), body })
    };
    let c = Var::Local(0);
    assert_eq!(
        bound(closure(c, GENERATED | CLASS)),
        BTreeSet::from([0x300])
    );
    // A lambda that captures nothing is bound to a holder, which no loop makes anew.
    assert_eq!(bound(kept_form(CACHE, 0x400)), BTreeSet::from([0x400]));

    let twice = {
        let mut body = closure(c, GENERATED | CLASS);
        body.extend(pair(Var::Local(2), var(c), function(0x300)));
        body
    };
    let looped = vec![Stmt::While {
        cond: var(P1),
        body: closure(c, GENERATED | CLASS),
    }];
    let labelled = {
        let mut body = closure(c, GENERATED | CLASS);
        body.push(Stmt::Label(tree::Label::At(0x90)));
        body
    };
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        ("bound at two places", twice),
        ("a closure object made each round of a loop", looped),
        ("a label, which may close a loop", labelled),
    ];
    for (what, body) in cases {
        assert_eq!(bound(body), BTreeSet::new(), "{what}");
    }
}

/// What the source level writes of `body` once this pass has read it, with the passes of `off` switched off.
fn written(body: Vec<Stmt>, off: &[Pass]) -> String {
    let names = Names::default();
    let options = off
        .iter()
        .fold(Options::default(), |options, pass| options.without(*pass));
    let cx = Context::new(&names, Project::empty(), options);
    let mut f = tree::Function { head: head(), body };
    rewrite(&mut f, &cx);
    let mut out = String::new();
    crate::gta6::script::print::function_in(&mut out, &f, &cx);
    out
}

#[test]
fn an_event_is_written_as_the_handler_added_to_its_place_or_taken_from_it() {
    for (join, op) in [(COMBINE, "+="), (REMOVE, "-=")] {
        let body = || {
            Subscribed {
                join,
                ..subscribed()
            }
            .body()
        };
        let text = written(body(), &[]);
        assert!(text.contains(&format!("    p0->f_30 {op} p1;\n")), "{text}");
        assert!(!text.contains("CompareExchange"), "{text}");
        // With the name off the read and the loop are written as they stand.
        let lowered = written(body(), &[Pass::Events]);
        assert!(lowered.contains("CompareExchange"), "{lowered}");
        assert!(lowered.contains("    do\n"), "{lowered}");
    }
}

#[test]
fn a_kept_delegate_is_written_as_the_cache_given_it_where_it_holds_none() {
    let text = written(kept_form(CACHE, 0x400), &[]);
    let taken = text.lines().find(|line| line.contains("??="));
    let taken = taken.unwrap_or_else(|| panic!("{text}"));
    assert!(taken.trim_start().starts_with("stack_"), "{text}");
    assert!(
        taken.contains(" ??= ") && taken.ends_with("func_000400;"),
        "{text}"
    );
    assert!(!text.contains("if ("), "{text}");
    // With `lambdas` off the test and its two arms are written.
    let lowered = written(kept_form(CACHE, 0x400), &[Pass::Lambdas]);
    assert!(lowered.contains("if ("), "{lowered}");
    assert!(!lowered.contains("??="), "{lowered}");
}
