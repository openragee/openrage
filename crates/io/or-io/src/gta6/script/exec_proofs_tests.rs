//! The proofs of a project held to the code of the functions they are about: each is caught
//! where a function's own code denies it, and left alone where the code agrees.

use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{Const, Head, StoreKind, VmType};
use crate::gta6::script::project::reach::Reach;
use crate::gta6::script::project::{FieldFacts, Member, Param, TypeFacts};

/// The program and offset of the function each test runs.
const PROGRAM: u32 = 0x0000_0001;
const AT: u32 = 0x10;
/// A function it calls.
const CALLED: u32 = 0x40;

/// Types the tests make objects of: one a field or a parameter is declared as, one of no relation to it, one that holds the field.
const WANT: &str = "type_00000000000000C1";
const OTHER: &str = "type_00000000000000C2";
const HOLDER: &str = "type_00000000000000C3";

fn key(name: &str) -> TypeKey {
    TypeKey::of_name(name).expect("a type's name")
}

fn own() -> FunctionKey {
    FunctionKey::new(PROGRAM, AT)
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn native(name: &str, hash: u64, args: Vec<Expr>, returns: u32) -> Expr {
    Expr::Call {
        callee: Callee::Native {
            namespace: Some("DEMO".to_owned()),
            name: name.to_owned(),
            hash,
            rva: 0,
        },
        args,
        returns: Some(returns),
    }
}

/// The native whose result a project may prove only ever tested for zero.
fn gauge() -> Expr {
    native("GAUGE", 0xB1, vec![p(0)], 1)
}

/// A native that takes one cell and yields none.
fn tally(arg: Expr) -> Stmt {
    Stmt::Eval(native("TALLY", 0xB2, vec![arg], 0))
}

fn called(args: Vec<Expr>) -> Stmt {
    Stmt::Eval(Expr::Call {
        callee: Callee::Local {
            at: CALLED,
            name: format!("func_{CALLED:06X}"),
        },
        args,
        returns: Some(0),
    })
}

fn made(name: &str) -> Expr {
    Expr::NewObject(VmType {
        name: name.to_owned(),
        byte_size: 32,
        id: 7,
        flags: 0,
    })
}

fn set(n: u32, value: Expr) -> Stmt {
    Stmt::Assign {
        target: Var::Local(n),
        value,
    }
}

fn store(kind: StoreKind, addr: Expr, offset: u32, value: Expr) -> Stmt {
    Stmt::Store {
        kind,
        addr,
        offset,
        value,
    }
}

fn head(params: u32, locals: u32) -> Head {
    Head {
        value_types: Default::default(),
        program: PROGRAM,
        at: AT,
        name: format!("func_{AT:06X}"),
        returns: Returns::Cell,
        params,
        locals,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

/// A function of `params` parameters and four locals: `blocks`, each its statements and how it ends.
fn function(params: u32, blocks: Vec<(Vec<Stmt>, Terminator)>) -> cfg::Function {
    let blocks = blocks.into_iter().map(|(stmts, term)| cfg::Block {
        at: None,
        stmts,
        term,
    });
    cfg::Function {
        head: head(params, 4),
        cfg: cfg::Cfg {
            blocks: blocks.collect(),
        },
    }
}

/// A function of one block that ends returning `value`.
fn straight(params: u32, stmts: Vec<Stmt>, value: Expr) -> cfg::Function {
    function(params, vec![(stmts, Terminator::Return(Some(value)))])
}

/// The seeds the tests run on: enough that a made-up cell is other than zero or one on some.
const RUNS: u64 = 16;

/// What [`proofs_held`] finds of `f` under `project`, over [`RUNS`] seeds: the kind of the first proof denied.
fn denied(f: &cfg::Function, project: &Project) -> Option<String> {
    let singles = BTreeSet::new();
    let was: Vec<(Input, Run)> = (0..RUNS)
        .map(|seed| {
            let input = Input::of(seed);
            let run = run_on(f, &input, &[], Limits::DEFAULT, project, &singles);
            (input, run)
        })
        .collect();
    let found = proofs_held(f, &was, Limits::DEFAULT, project, &singles)?;
    Some(found.split(',').next().unwrap_or_default().to_owned())
}

fn types(relations: &[(&str, Option<&str>)]) -> BTreeMap<TypeKey, TypeFacts> {
    let facts = |base: &Option<&str>| TypeFacts {
        base: base.map(key),
        ..TypeFacts::default()
    };
    (relations.iter())
        .map(|(name, base)| (key(name), facts(base)))
        .collect()
}

#[test]
fn a_native_proved_a_truth_that_is_read_for_more_is_caught() {
    let mut project = Project::default();
    project.native_truths.insert(0xB1);
    // Its result handed on whole, three times over.
    let handed = straight(
        1,
        vec![tally(gauge()), tally(gauge()), tally(gauge())],
        int(0),
    );
    assert_eq!(denied(&handed, &project).as_deref(), Some("native truth"));
    // Tested, the result says no more than whether it is zero.
    let tested = |n: usize| {
        let test = Terminator::Branch {
            cond: Expr::Truth(Box::new(gauge())),
            then: n + 1,
            els: n + 2,
        };
        vec![
            (vec![], test),
            (vec![tally(int(1))], Terminator::Jump(n + 2)),
        ]
    };
    let mut blocks = tested(0);
    blocks.extend(tested(2));
    blocks.push((vec![], Terminator::Return(Some(int(0)))));
    assert_eq!(denied(&function(1, blocks), &project), None);
    // Returned, it is held as a truth: whether the function's callers only test it is the project's.
    assert_eq!(denied(&straight(1, vec![], gauge()), &project), None);
    // With no such proof, nothing is held.
    assert_eq!(denied(&handed, &Project::default()), None);
}

#[test]
fn a_native_proved_a_truth_is_held_by_the_before_and_after_check() {
    let mut project = Project::default();
    project.native_truths.insert(0xB1);
    let handed = straight(
        1,
        vec![tally(gauge()), tally(gauge()), tally(gauge())],
        int(0),
    );
    let names = Names::default();
    let cx = Context::new(&names, &project, Options::baseline());
    let why = check_blocks(&handed, &cx, Limits::DEFAULT).expect_err("the proof is denied");
    assert!(why.starts_with("native truth, "), "{why}");
}

#[test]
fn a_slot_line_said_to_return_a_truth_is_held_to_what_one_of_it_returns() {
    let mut project = Project::default();
    project.slot_results.insert(own(), Kind::Bool);
    assert_eq!(
        denied(&straight(1, vec![], p(0)), &project).as_deref(),
        Some("slot result")
    );
    let truth = Expr::Truth(Box::new(p(0)));
    assert_eq!(denied(&straight(1, vec![], truth), &project), None);
}

#[test]
fn an_object_of_another_type_handed_to_a_typed_parameter_is_caught() {
    let signature = Signature {
        params: vec![Param {
            kind: Kind::Object(WANT.to_owned()),
            cells: 1,
        }],
        returns: 0,
        result: Kind::Unknown,
    };
    let f = straight(0, vec![set(0, made(OTHER)), called(vec![local(0)])], int(0));
    let mut project = Project::default();
    project
        .signatures
        .insert(FunctionKey::new(PROGRAM, CALLED), signature);
    project.types = types(&[(WANT, None), (OTHER, None)]);
    assert_eq!(denied(&f, &project).as_deref(), Some("typed parameter"));
    // An object of a type derived from the one declared stands for it.
    project.types = types(&[(WANT, None), (OTHER, Some(WANT))]);
    assert_eq!(denied(&f, &project), None);
    // Of a type the project holds no facts of, nothing is said.
    project.types = types(&[(WANT, None)]);
    assert_eq!(denied(&f, &project), None);
}

#[test]
fn an_object_of_another_type_stored_to_a_typed_field_is_caught() {
    let f = straight(
        0,
        vec![
            set(0, made(OTHER)),
            set(1, made(HOLDER)),
            store(StoreKind::Ref, local(1), 8, local(0)),
        ],
        int(0),
    );
    let field = FieldFacts {
        offset: 8,
        bytes: 8,
        kind: Kind::Object(WANT.to_owned()),
    };
    let mut project = Project::default();
    project.fields.insert(key(HOLDER), vec![field]);
    project.types = types(&[(WANT, None), (OTHER, None), (HOLDER, None)]);
    assert_eq!(denied(&f, &project).as_deref(), Some("typed field"));
    project.types = types(&[(WANT, None), (OTHER, Some(WANT)), (HOLDER, None)]);
    assert_eq!(denied(&f, &project), None);
}

#[test]
fn an_object_of_another_type_returned_where_a_type_is_said_is_caught() {
    let f = straight(0, vec![set(0, made(OTHER))], local(0));
    let mut project = Project::default();
    let signature = Signature {
        params: Vec::new(),
        returns: 1,
        result: Kind::Object(WANT.to_owned()),
    };
    project.signatures.insert(own(), signature);
    project.types = types(&[(WANT, None), (OTHER, None)]);
    assert_eq!(denied(&f, &project).as_deref(), Some("typed result"));
    project.types = types(&[(WANT, None), (OTHER, Some(WANT))]);
    assert_eq!(denied(&f, &project), None);
}

/// A project in which the function is a method of [`HOLDER`], whose byte 0x10 is a `bool`.
fn with_a_bool_field() -> Project {
    let mut project = Project::default();
    let member = Member {
        owner: key(HOLDER),
        role: Role::Method,
        is_static: false,
        slot: None,
    };
    project.members.insert(own(), member);
    let field = FieldFacts {
        offset: 0x10,
        bytes: 1,
        kind: Kind::Bool,
    };
    project.fields.insert(key(HOLDER), vec![field]);
    project.types = types(&[(HOLDER, None)]);
    project
}

#[test]
fn a_bool_field_written_through_the_receiver_with_another_byte_is_caught() {
    let project = with_a_bool_field();
    let two = straight(1, vec![store(StoreKind::U8, p(0), 0x10, int(2))], int(0));
    assert_eq!(denied(&two, &project).as_deref(), Some("bool field"));
    let one = straight(1, vec![store(StoreKind::U8, p(0), 0x10, int(1))], int(0));
    assert_eq!(denied(&one, &project), None);
    // A store as wide as four bytes over it writes its byte too.
    let wide = straight(
        1,
        vec![store(StoreKind::I32, p(0), 0xF, int(0x300))],
        int(0),
    );
    assert_eq!(denied(&wide, &project).as_deref(), Some("bool field"));
    // A static function's first parameter is no receiver.
    let mut project = project;
    if let Some(member) = project.members.get_mut(&own()) {
        member.is_static = true;
    }
    assert_eq!(denied(&two, &project), None);
}

/// A project that says parameter cell 0 of the function reaches `bytes` bytes, and cell 0 of the function it calls `theirs`.
fn reaching(bytes: u32, theirs: u32) -> Project {
    let reach = |bytes: u32| {
        let cell = Reach {
            bytes,
            ..Reach::default()
        };
        BTreeMap::from([(0, cell)])
    };
    let mut project = Project::default();
    project.reach.insert(own(), reach(bytes));
    project
        .reach
        .insert(FunctionKey::new(PROGRAM, CALLED), reach(theirs));
    project
}

#[test]
fn an_access_past_the_reach_the_project_gives_a_parameter_is_caught() {
    let project = reaching(8, 8);
    let within = straight(1, vec![store(StoreKind::I32, p(0), 4, int(1))], int(0));
    assert_eq!(denied(&within, &project), None);
    let past = straight(1, vec![store(StoreKind::I32, p(0), 8, int(1))], int(0));
    assert_eq!(denied(&past, &project).as_deref(), Some("reach"));
    let load = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(p(0)),
        offset: 8,
    };
    assert_eq!(
        denied(&straight(1, vec![tally(load)], int(0)), &project).as_deref(),
        Some("reach")
    );
    // Unbounded, the reach says nothing to hold.
    let unbounded = reaching(UNBOUNDED, 8);
    assert_eq!(denied(&past, &unbounded), None);
}

#[test]
fn an_address_said_to_reach_a_bound_that_goes_further_is_caught() {
    // Handed on to a function that reaches no further, it stays in its bound.
    let handed = straight(1, vec![called(vec![p(0)])], int(0));
    assert_eq!(denied(&handed, &reaching(8, 8)), None);
    assert_eq!(denied(&handed, &reaching(8, 16)).as_deref(), Some("reach"));
    // Handed to a native, returned or stored, it goes where no reading follows.
    let native = straight(1, vec![tally(p(0))], int(0));
    assert_eq!(denied(&native, &reaching(8, 8)).as_deref(), Some("reach"));
    let returned = straight(1, vec![], p(0));
    assert_eq!(denied(&returned, &reaching(8, 8)).as_deref(), Some("reach"));
    let global = Var::Global {
        program: PROGRAM,
        offset: 0x20,
        statics: false,
    };
    let kept = Stmt::Assign {
        target: global,
        value: p(0),
    };
    let stored = straight(1, vec![kept], int(0));
    assert_eq!(denied(&stored, &reaching(8, 8)).as_deref(), Some("reach"));
}
