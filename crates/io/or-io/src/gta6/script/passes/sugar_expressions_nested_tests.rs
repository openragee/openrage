use super::*;
use crate::gta6::script::project::{Param, Signature};

fn address(n: u32) -> Expr {
    Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    }
}

fn native(args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Native {
            namespace: None,
            name: "_0x0000000000000001".to_owned(),
            hash: 1,
            rva: 0x10,
        },
        args,
        returns: Some(1),
    }
}

/// `func_000200(p0, handed, ..)`: what is handed lands in parameter cell 1, and the rest after it.
fn fill(handed: Expr, rest: Vec<Expr>) -> Expr {
    let mut args = vec![p(0), handed];
    args.extend(rest);
    call(0x200, args)
}

fn evaluated(value: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(value))
}

/// A project in which `func_000200` takes `cells` parameter cells, and whose reach table says of cell 1 only `held`.
fn reaching(held: reach::Reach, cells: u32) -> Project {
    let mut project = Project::default();
    let key = FunctionKey::new(1, 0x200);
    let one = Param {
        kind: Kind::Unknown,
        cells: 1,
    };
    let signature = Signature {
        params: vec![one; cells as usize],
        ..Signature::default()
    };
    project.signatures.insert(key, signature);
    project.reach.insert(key, [(1, held)].into_iter().collect());
    project
}

/// What a function does through a cell: the bytes it touches, how, the bytes every path writes, and whether the address dies with the call.
fn uses(bytes: u32, mode: reach::Mode, covered: u32, dies: bool) -> reach::Reach {
    reach::Reach {
        bytes,
        mode: Some(mode),
        covered,
        dies,
        ..reach::Reach::default()
    }
}

fn referred_in(head: Head, body: Vec<Stmt>, project: &Project) -> Vec<Stmt> {
    rewritten_in(head, body, project, only(Pass::RefOut))
}

/// The value the statement at `at` of `done` evaluates.
fn value_at(done: &[Stmt], at: usize) -> Expr {
    match done.get(at) {
        Some(Stmt::Simple(ir::Stmt::Eval(value))) => value.clone(),
        other => panic!("an evaluation: {other:?}"),
    }
}

#[test]
fn an_address_a_script_function_touches_nothing_past_is_a_ref_and_one_it_writes_first_is_an_out() {
    let by = |kind: ExprSugar| fill(Expr::sugar(kind, address(4)), Vec::new());
    let plain = fill(address(4), Vec::new());
    let read = |project: &Project| {
        let done = referred_in(head(&[]), vec![evaluated(plain.clone())], project);
        value_at(&done, 0)
    };

    // Local 4 is proved nothing, so with nothing known of the callee its address stays.
    assert_eq!(read(Project::empty()), plain);
    // The callee touches the one cell and nothing keeps the address: a `ref`.
    let within = reaching(uses(8, reach::Mode::Ref, 0, true), 2);
    assert_eq!(read(&within), by(ExprSugar::Ref));
    let only_read = reaching(uses(8, reach::Mode::In, 0, true), 2);
    assert_eq!(read(&only_read), by(ExprSugar::Ref));
    // It writes the whole cell on every path before any read: an `out`.
    let out = reaching(uses(8, reach::Mode::Out, 8, true), 2);
    let done = referred_in(head(&[]), vec![evaluated(plain.clone())], &out);
    assert_eq!(value_at(&done, 0), by(ExprSugar::Out));
    assert_eq!(tree::desugar(done), vec![evaluated(plain.clone())]);
    // Written in part is no `out`; past the cell, or kept past the call, it is not even a `ref`.
    let part = reaching(uses(8, reach::Mode::Out, 4, true), 2);
    assert_eq!(read(&part), by(ExprSugar::Ref));
    let past = reaching(uses(16, reach::Mode::Ref, 0, true), 2);
    assert_eq!(read(&past), plain);
    let kept = reaching(uses(8, reach::Mode::Out, 8, false), 2);
    assert_eq!(read(&kept), plain);
    let unbounded = reaching(uses(reach::UNBOUNDED, reach::Mode::Ref, 0, true), 2);
    assert_eq!(read(&unbounded), plain);
    // ⚠ A call that hands other cells than the function takes says nothing of where each lands.
    let otherwise = reaching(uses(8, reach::Mode::Ref, 0, true), 3);
    assert_eq!(read(&otherwise), plain);

    // ⚠ A local proved one cell is a `ref` with nothing known, and not where the callee reaches past it.
    let mut single = head(&[]);
    let mut info = VarInfo::new(Width::Cell);
    info.single = true;
    single.vars.insert(Var::Local(4), info);
    let in_single = |project: &Project| {
        let done = referred_in(single.clone(), vec![evaluated(plain.clone())], project);
        value_at(&done, 0)
    };
    assert_eq!(in_single(Project::empty()), by(ExprSugar::Ref));
    assert_eq!(in_single(&past), plain);
}

#[test]
fn an_out_needs_that_nothing_else_the_callee_can_reach_holds_the_address() {
    let out = reaching(uses(8, reach::Mode::Out, 8, true), 2);
    let out_of_three = reaching(uses(8, reach::Mode::Out, 8, true), 3);
    let by = |kind: ExprSugar, rest: Vec<Expr>| fill(Expr::sugar(kind, address(4)), rest);

    // Another argument that is an address the callee may follow anywhere.
    let other = fill(address(4), vec![address(6)]);
    let done = referred_in(head(&[]), vec![evaluated(other)], &out_of_three);
    assert_eq!(value_at(&done, 0), by(ExprSugar::Ref, vec![address(6)]));

    // An address of a local at or before it, taken elsewhere and kept, may reach it; one after it may not.
    let plain = fill(address(4), Vec::new());
    for (taken, kind) in [
        (2, ExprSugar::Ref),
        (4, ExprSugar::Ref),
        (6, ExprSugar::Out),
    ] {
        let body = vec![
            evaluated(plain.clone()),
            evaluated(native(vec![address(taken)])),
        ];
        let done = referred_in(head(&[]), body, &out);
        assert_eq!(value_at(&done, 0), by(kind, Vec::new()), "local {taken}");
    }

    // Its address handed to a function that keeps nothing dies there.
    let body = vec![evaluated(plain.clone()), evaluated(plain.clone())];
    let done = referred_in(head(&[]), body, &out);
    assert_eq!(value_at(&done, 0), by(ExprSugar::Out, Vec::new()));
    assert_eq!(value_at(&done, 1), by(ExprSugar::Out, Vec::new()));
}

#[test]
fn a_variable_made_zero_and_then_handed_as_an_out_is_one_statement() {
    let out = reaching(uses(8, reach::Mode::Out, 8, true), 2);
    let zero = assign(Var::Local(4), int(0));
    let handed = Expr::sugar(ExprSugar::Out, address(4));
    let body = vec![
        Stmt::Simple(zero.clone()),
        evaluated(fill(address(4), Vec::new())),
    ];
    let done = referred_in(head(&[]), body.clone(), &out);
    let call = ir::Stmt::Eval(fill(handed.clone(), Vec::new()));
    assert_eq!(done, vec![group(Group::Out, vec![zero.clone(), call])]);
    assert_eq!(tree::desugar(done), body);

    // Read again by another argument, the zero is what that argument reads.
    let body = vec![
        Stmt::Simple(zero.clone()),
        evaluated(fill(address(4), vec![local(4)])),
    ];
    let out_of_three = reaching(uses(8, reach::Mode::Out, 8, true), 3);
    let done = referred_in(head(&[]), body, &out_of_three);
    let read = vec![
        Stmt::Simple(zero.clone()),
        evaluated(fill(handed, vec![local(4)])),
    ];
    assert_eq!(done, read);

    // A `ref` is no `out`: the zero stays a statement of its own.
    let within = reaching(uses(8, reach::Mode::Ref, 0, true), 2);
    let body = vec![
        Stmt::Simple(zero.clone()),
        evaluated(fill(address(4), Vec::new())),
    ];
    let done = referred_in(head(&[]), body, &within);
    assert_eq!(done.first(), Some(&Stmt::Simple(zero)));
}

fn inner() -> Var {
    Var::Value {
        at: 0x200,
        index: 0,
    }
}

fn parts_at(done: &[Stmt], at: usize) -> Vec<ir::Stmt> {
    match done.get(at) {
        Some(Stmt::Sugar(sugar)) => match &**sugar {
            Sugar::Group {
                kind: Group::Construct,
                parts,
            } => parts.clone(),
            other => panic!("an initialiser: {other:?}"),
        },
        other => panic!("an initialiser: {other:?}"),
    }
}

#[test]
fn an_object_made_and_filled_straight_before_a_field_takes_it_is_inside_the_initialiser() {
    let (v, w) = (temporary(), inner());
    let outer = field(v, StoreKind::Ref, 0x8, p(0));
    let filled = field(w, StoreKind::I32, 0x8, int(1));
    let taken = field(v, StoreKind::Ref, 0x10, Expr::Var(w));
    let after = field(v, StoreKind::I32, 0x18, int(2));
    let parts = vec![
        made_object(v),
        outer.clone(),
        made_object(w),
        filled.clone(),
        taken.clone(),
        after,
    ];
    let used = does(0x300, vec![Expr::Var(v)]);
    let mut body = simples(&parts);
    body.push(used.clone());

    let done = initialised(body.clone());
    assert_eq!(done, vec![group(Group::Construct, parts), used]);
    assert_eq!(made(&done).objects, 1);
    assert_eq!(tree::desugar(done), body);

    // One made with no store of its own, straight before the store that takes it.
    let body = simples(&[made_object(v), made_object(w), taken.clone()]);
    let done = initialised(body);
    let held = vec![made_object(v), made_object(w), taken.clone()];
    assert_eq!(parts_at(&done, 0), held);

    // ⚠ Each stays apart where the source would run otherwise or name what it has no name for.
    let head_only = vec![made_object(v), outer.clone()];
    let apart = vec![
        (
            "the inner object is named again",
            vec![
                filled.clone(),
                taken.clone(),
                ir::Stmt::Eval(call(0x400, vec![Expr::Var(w)])),
            ],
        ),
        (
            "an inner value reads the outer object",
            vec![field(w, StoreKind::Ref, 0x8, Expr::Var(v)), taken.clone()],
        ),
        (
            "a statement stands before the store that takes it",
            vec![filled.clone(), assign(Var::Local(5), int(3)), taken.clone()],
        ),
        (
            "a field takes something else",
            vec![filled.clone(), field(v, StoreKind::Ref, 0x10, p(1))],
        ),
    ];
    for (why, rest) in apart {
        let mut stmts = head_only.clone();
        stmts.push(made_object(w));
        stmts.extend(rest);
        let done = initialised(simples(&stmts));
        assert_eq!(parts_at(&done, 0), head_only, "{why}");
    }
}
