use super::*;
use crate::gta6::script::cfg::Terminator;
use crate::gta6::script::ir::Callee;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::project::signatures::tests::{function, listing};
use crate::gta6::script::project::Signature;
use crate::gta6::script::{Names, Options};

/// The function at 0x10 of program 1 that runs `body`, as the reader makes it.
fn lowered(ty: &str, body: &str) -> cfg::Function {
    let text = listing(1, &[function(ty, 0x10, 1, body)]);
    let listing = Listing::open(&text).expect("the listing opens");
    let read = listing.functions().next().expect("a function");
    cfg::lower(read.expect("the function reads")).expect("the function lowers")
}

/// A project in which the function at 0x20 of program 1 takes `params` and returns `returns` cells that are `result`.
fn project(params: Vec<Param>, returns: u32, result: Kind) -> Project {
    let signature = Signature {
        params,
        returns,
        result,
    };
    let mut project = Project::default();
    project
        .signatures
        .insert(FunctionKey::new(1, 0x20), signature);
    project
}

fn truth() -> Project {
    project(Vec::new(), 1, Kind::Bool)
}

/// The cells each call of a function of the program states, in the order the calls stand.
fn counts(f: &cfg::Function) -> Vec<Option<u32>> {
    let mut counts = Vec::new();
    let mut note = |top: &Expr| {
        top.walk(&mut |e| {
            if let Expr::Call {
                callee: Callee::Local { .. },
                returns,
                ..
            } = e
            {
                counts.push(*returns);
            }
        });
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            stmt.each_expr(&mut note);
        }
        if let Some(e) = block.term.expr() {
            note(e);
        }
    }
    counts
}

fn is_call(e: &Expr) -> bool {
    matches!(e, Expr::Call { .. })
}

fn is_cast(e: &Expr) -> bool {
    matches!(e, Expr::Cast { .. })
}

const TWICE: &str = "v_000002_1 = func_000020(p0); func_000030(v_000002_1[0]); return;";

#[test]
fn a_call_states_the_cells_its_callee_returns() {
    let mut f = lowered("void", TWICE);
    let done = apply(&mut f, &project(Vec::new(), 2, Kind::Unknown));
    // The project holds no function at 0x30: its call keeps saying nothing.
    assert_eq!(counts(&f), [Some(2), None]);
    assert_eq!(done.sized, 1);
}

#[test]
fn a_signature_that_still_waits_states_nothing() {
    let waiting = Param {
        kind: Kind::Unknown,
        cells: 0,
    };
    let mut f = lowered("void", TWICE);
    let was = f.clone();
    let done = apply(&mut f, &project(vec![waiting], 2, Kind::Bool));
    assert_eq!((done, &f), (Applied::default(), &was));
}

#[test]
fn a_truth_read_as_an_integer_and_held_against_zero_is_the_truth() {
    let test =
        |op: &str| format!("if (((as_u32(func_000020(p0)) {op} 0))) {{ return 1; }} return 0;");
    let cond = |f: &cfg::Function| match &f.cfg.blocks[0].term {
        Terminator::Branch { cond, .. } => cond.clone(),
        other => panic!("no test: {other:?}"),
    };

    let mut f = lowered("VmValue", &test("!="));
    let done = apply(&mut f, &truth());
    assert!(is_call(&cond(&f)), "{:?}", cond(&f));
    assert_eq!((done.casts, done.tests), (1, 1));

    let mut f = lowered("VmValue", &test("=="));
    apply(&mut f, &truth());
    assert!(
        matches!(cond(&f), Expr::Unary { op: UnOp::Not, e } if is_call(&e)),
        "{:?}",
        cond(&f)
    );

    // An `int` read as a `uint` is another cell where it is negative, and no truth: both stay.
    let mut f = lowered("VmValue", &test("!="));
    let done = apply(&mut f, &project(Vec::new(), 1, Kind::I32));
    assert!(
        matches!(cond(&f), Expr::Binary { op: BinOp::Ne, l, .. } if is_cast(&l)),
        "{:?}",
        cond(&f)
    );
    assert_eq!((done.casts, done.tests), (0, 0));
}

#[test]
fn the_value_a_variable_is_assigned_keeps_its_reading_unless_the_variable_is_proved() {
    let set = "local_0 = as_u32(func_000020(p0));";
    let value = |f: &cfg::Function| match &f.cfg.blocks[0].stmts[0] {
        Stmt::Assign { value, .. } => value.clone(),
        other => panic!("no assignment: {other:?}"),
    };
    let kind = |f: &cfg::Function| {
        f.head
            .vars
            .get(&Var::Local(0))
            .map(|info| info.kind.clone())
    };

    let mut f = lowered("VmValue", &format!("{set} return local_0;"));
    let done = apply(&mut f, &truth());
    assert!(is_call(&value(&f)), "{:?}", value(&f));
    assert_eq!((kind(&f), done.kinds), (Some(Kind::Bool), 1));

    // Assigned twice, the local is typed by what it is assigned: the cast is that.
    let mut f = lowered("VmValue", &format!("{set} local_0 = 5; return local_0;"));
    apply(&mut f, &truth());
    assert!(is_cast(&value(&f)), "{:?}", value(&f));
    assert_eq!(kind(&f), None);

    // A callee given its address may write it.
    let reached = format!("{set} Native.DEMO.USE(Vm.Address(locals, 0)); return local_0;");
    let mut f = lowered("VmValue", &reached);
    apply(&mut f, &truth());
    assert!(is_cast(&value(&f)), "{:?}", value(&f));
    assert_eq!(kind(&f), None);
}

#[test]
fn a_parameter_only_passed_on_takes_the_kind_its_signature_states() {
    let float = Param {
        kind: Kind::F32,
        cells: 1,
    };
    let mut own = project(Vec::new(), 0, Kind::Unknown);
    let signature = Signature {
        params: vec![float],
        returns: 0,
        result: Kind::Unknown,
    };
    own.signatures.insert(FunctionKey::new(1, 0x10), signature);
    let kind = |f: &cfg::Function| {
        f.head
            .vars
            .get(&Var::Param(0))
            .map(|info| info.kind.clone())
    };

    let mut f = lowered("void", "func_000020(p0); return;");
    apply(&mut f, &own);
    assert_eq!(kind(&f), Some(Kind::F32));

    // Read by a native too, the parameter is typed by this code's own readings of it.
    let mut f = lowered("void", "func_000020(p0); Native.DEMO.USE(p0); return;");
    apply(&mut f, &own);
    assert_eq!(kind(&f), None);
}

#[test]
fn with_the_name_off_nothing_is_touched() {
    let names = Names::default();
    let project = project(Vec::new(), 2, Kind::Unknown);
    let mut stats = Stats::default();
    let mut f = lowered("void", TWICE);
    let was = f.clone();

    let off = Context::new(&names, &project, Options::baseline());
    run(&mut f, &off, &mut stats);
    assert_eq!(f, was);

    let on = Context::new(&names, &project, Options::baseline().with(Pass::Interproc));
    run(&mut f, &on, &mut stats);
    assert_eq!(counts(&f), [Some(2), None]);
}
