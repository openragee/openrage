use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{BinOp, Callee, Const, Kind, Returns, Scalar, VarInfo};
use crate::gta6::script::passes::sugar_statements::{rewrite, Found};
use crate::gta6::script::project::reach::Reach;
use crate::gta6::script::project::{Member, TypeFacts};
use crate::gta6::script::{Context, Names, Options, Pass};

// Made up: the program, a value type's hash, and the offsets of the functions the loops call.
const PROGRAM: u32 = 1;
const ENUMERATOR: TypeKey = TypeKey(0x1357_9BDF_2468_ACE0);
const MAKE: u32 = 0x300;
const ADVANCE: u32 = 0x310;
const CURRENT: u32 = 0x320;
const OTHER_MAKE: u32 = 0x330;
const HANDS: u32 = 0x340;
/// A function the project says nothing of.
const UNKNOWN: u32 = 0x500;

const MADE: Var = Var::Value { at: 0x40, index: 0 };
const MADE_AGAIN: Var = Var::Value { at: 0x60, index: 0 };

fn key(at: u32) -> FunctionKey {
    FunctionKey::new(PROGRAM, at)
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn address(var: Var) -> Expr {
    Expr::AddrOf { var, cell: 0 }
}

fn call(at: u32, args: Vec<Expr>, returns: u32) -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args,
        returns: Some(returns),
    }
}

fn assign(target: Var, value: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Assign { target, value })
}

fn eval(e: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(e))
}

/// `v = default; make(&v, p0, 0); local_<e> = v;`.
fn setup(make: u32, made: Var, e: u32) -> Vec<Stmt> {
    let args = vec![address(made), Expr::Var(Var::Param(0)), int(0)];
    vec![
        assign(made, Expr::ZeroSlots(3)),
        eval(call(make, args, 0)),
        assign(Var::Local(e), Expr::Var(made)),
    ]
}

/// `(uint)advance(&local_<e>) != 0`.
fn advanced(e: u32) -> Expr {
    let advance = call(ADVANCE, vec![address(Var::Local(e))], 1);
    Expr::binary(BinOp::Ne, Expr::cast(Scalar::U32, advance), int(0))
}

/// `local_<x> = local_<e>.f_10;`.
fn field(e: u32, x: u32) -> Stmt {
    let load = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(address(Var::Local(e))),
        offset: 0x10,
    };
    assign(Var::Local(x), load)
}

/// A use of `local_<x>` by a function the project says nothing of.
fn touch(x: u32) -> Stmt {
    eval(call(UNKNOWN, vec![local(x)], 0))
}

fn looped(e: u32, item: Stmt, rest: Vec<Stmt>) -> Stmt {
    let mut body = vec![item];
    body.extend(rest);
    Stmt::While {
        cond: advanced(e),
        body,
    }
}

/// The whole expansion over `local_<e>`, its item `local_<x>` used once.
fn site(make: u32, made: Var, e: u32, x: u32) -> Vec<Stmt> {
    let mut stmts = setup(make, made, e);
    stmts.push(looped(e, field(e, x), vec![touch(x)]));
    stmts
}

fn reach(bytes: u32, mode: Mode) -> Reach {
    Reach {
        bytes,
        mode: Some(mode),
        dies: true,
        ..Reach::default()
    }
}

/// A project that proves the advance and the getter methods of a value type of three cells, and states the reach of each function.
fn project() -> Project {
    let mut project = Project::default();
    let value = TypeFacts {
        flags: 0x04,
        ..TypeFacts::default()
    };
    project.types.insert(ENUMERATOR, value);
    for at in [ADVANCE, CURRENT] {
        let member = Member {
            owner: ENUMERATOR,
            role: Role::Method,
            is_static: false,
            slot: Some(6),
        };
        project.members.insert(key(at), member);
    }
    let facts = [
        (MAKE, 0x18, Mode::Out),
        (OTHER_MAKE, 0x18, Mode::Out),
        (ADVANCE, 0x18, Mode::Ref),
        (CURRENT, 0x18, Mode::In),
        (HANDS, 0x08, Mode::Ref),
    ];
    for (at, bytes, mode) in facts {
        let held = BTreeMap::from([(0, reach(bytes, mode))]);
        project.reach.insert(key(at), held);
    }
    project
}

/// What the pass makes of `body` where `local_0` and `local_5` are values of three cells.
fn read_with(body: Vec<Stmt>, project: &Project, options: Options) -> (Vec<Stmt>, Found) {
    read_beside(body, project, options, &[])
}

/// [`read_with`] where each `(n, cells)` also makes `local_n` a value of that many cells.
fn read_beside(
    body: Vec<Stmt>,
    project: &Project,
    options: Options,
    beside: &[(u32, u32)],
) -> (Vec<Stmt>, Found) {
    let wide = |cells: u32| VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(cells),
        name: None,
        single: false,
    };
    let mut vars = BTreeMap::from([(Var::Local(0), wide(3)), (Var::Local(5), wide(3))]);
    vars.extend(
        beside
            .iter()
            .map(|(n, cells)| (Var::Local(*n), wide(*cells))),
    );
    let mut f = tree::Function {
        head: Head {
            value_types: Default::default(),
            program: PROGRAM,
            at: 0x10,
            name: "func_000010".to_owned(),
            returns: Returns::Cell,
            params: 1,
            locals: 16,
            vars,
            incomplete: None,
        },
        body,
    };
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    let found = rewrite(&mut f, &cx);
    (f.body, found)
}

fn read(body: Vec<Stmt>) -> (Vec<Stmt>, Found) {
    read_with(body, &project(), Options::default())
}

#[test]
fn a_value_made_copied_advanced_in_place_and_read_first_is_a_foreach() {
    let was = site(MAKE, MADE, 0, 3);
    let (is, found) = read(was.clone());
    let one = Found {
        held: 1,
        ..Found::default()
    };
    assert_eq!(found, one);
    let [Stmt::Sugar(made)] = is.as_slice() else {
        panic!("{is:?}")
    };
    let Sugar::Foreach {
        setup: held,
        cond,
        item,
        body,
        step,
        after,
    } = &**made
    else {
        panic!("{made:?}")
    };
    let simples: Vec<Stmt> = held.iter().cloned().map(Stmt::Simple).collect();
    assert_eq!(simples, setup(MAKE, MADE, 0));
    assert_eq!(cond, &advanced(0));
    assert_eq!(Stmt::Simple((**item).clone()), field(0, 3));
    assert_eq!(body, &vec![touch(3)]);
    assert!(step.is_none() && after.is_empty());
    // The way back: the blocks of the loop it was read from.
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
}

#[test]
fn no_loop_is_read_while_foreach_is_off() {
    let was = site(MAKE, MADE, 0, 3);
    let off = Options::default().without(Pass::Foreach);
    let (is, found) = read_with(was.clone(), &project(), off);
    assert_eq!((is, found), (was, Found::default()));
}

#[test]
fn a_pair_the_project_does_not_prove_stays_a_loop() {
    let unchanged = |project: &Project, why: &str| {
        let was = site(MAKE, MADE, 0, 3);
        let (is, found) = read_with(was.clone(), project, Options::default());
        assert_eq!(is, was, "{why}");
        assert_eq!((found.held, found.refused), (0, 1), "{why}");
    };
    unchanged(Project::empty(), "the project holds nothing");
    type Change = Box<dyn Fn(&mut Project)>;
    let changes: Vec<(&str, Change)> = vec![
        (
            "the advance takes no receiver",
            Box::new(|p: &mut Project| {
                if let Some(m) = p.members.get_mut(&key(ADVANCE)) {
                    m.is_static = true;
                }
            }),
        ),
        (
            "the advance fills no slot",
            Box::new(|p: &mut Project| {
                if let Some(m) = p.members.get_mut(&key(ADVANCE)) {
                    m.slot = None;
                }
            }),
        ),
        (
            "the type is no value type",
            Box::new(|p: &mut Project| {
                if let Some(t) = p.types.get_mut(&ENUMERATOR) {
                    t.flags = 0;
                }
            }),
        ),
        (
            "the advance may reach past the enumerator",
            Box::new(|p: &mut Project| {
                p.reach
                    .insert(key(ADVANCE), BTreeMap::from([(0, reach(0x20, Mode::Ref))]));
            }),
        ),
        (
            "the advance keeps the address",
            Box::new(|p: &mut Project| {
                let kept = Reach {
                    dies: false,
                    ..reach(0x18, Mode::Ref)
                };
                p.reach.insert(key(ADVANCE), BTreeMap::from([(0, kept)]));
            }),
        ),
        (
            "the advance only reads the enumerator",
            Box::new(|p: &mut Project| {
                p.reach
                    .insert(key(ADVANCE), BTreeMap::from([(0, reach(0x18, Mode::In))]));
            }),
        ),
        (
            "the make reads what it is handed",
            Box::new(|p: &mut Project| {
                p.reach
                    .insert(key(MAKE), BTreeMap::from([(0, reach(0x18, Mode::Ref))]));
            }),
        ),
        (
            "the type implements an interface, which may make it disposable",
            Box::new(|p: &mut Project| {
                if let Some(t) = p.types.get_mut(&ENUMERATOR) {
                    t.interfaces
                        .push(crate::gta6::script::ir::TypeKey(0x8000_0001));
                }
            }),
        ),
        (
            "the make's reach is not known",
            Box::new(|p: &mut Project| {
                p.reach.remove(&key(MAKE));
            }),
        ),
    ];
    for (why, change) in changes {
        let mut project = project();
        change(&mut project);
        unchanged(&project, why);
    }
}

#[test]
fn an_enumerator_or_a_value_named_anywhere_else_stays_a_loop() {
    let cases: Vec<(&str, Vec<Stmt>, Vec<Stmt>)> = vec![
        (
            "the enumerator is read after the loop",
            Vec::new(),
            vec![touch(0)],
        ),
        (
            "the enumerator's address is handed on after the loop",
            Vec::new(),
            vec![eval(call(UNKNOWN, vec![address(Var::Local(0))], 0))],
        ),
        (
            "a later cell of the enumerator is assigned",
            Vec::new(),
            vec![assign(Var::Local(1), int(2))],
        ),
        (
            "the value made is read again",
            Vec::new(),
            vec![eval(call(UNKNOWN, vec![Expr::Var(MADE)], 0))],
        ),
        (
            "an address below the enumerator is handed to what reaches without bound",
            vec![eval(call(UNKNOWN, vec![address(Var::Param(0))], 0))],
            Vec::new(),
        ),
    ];
    for (why, before, after) in cases {
        let mut was = before;
        was.extend(site(MAKE, MADE, 0, 3));
        was.extend(after);
        let (is, found) = read(was.clone());
        assert_eq!(is, was, "{why}");
        assert_eq!((found.held, found.refused), (0, 1), "{why}");
    }
}

#[test]
fn the_item_is_kept_as_the_round_took_it_and_the_enumerator_is_hidden_from_the_body() {
    let cases: Vec<(&str, Vec<Stmt>)> = vec![
        (
            "the item is assigned again",
            vec![assign(Var::Local(3), int(1))],
        ),
        (
            "the item's address is handed on",
            vec![eval(call(UNKNOWN, vec![address(Var::Local(3))], 0))],
        ),
        (
            "the body advances the enumerator itself",
            vec![eval(call(ADVANCE, vec![address(Var::Local(0))], 1))],
        ),
        (
            "a label stands in the body",
            vec![Stmt::Label(tree::Label::Block(1))],
        ),
    ];
    for (why, rest) in cases {
        let mut was = setup(MAKE, MADE, 0);
        was.push(looped(0, field(0, 3), rest));
        let (is, found) = read(was.clone());
        assert_eq!(is, was, "{why}");
        assert_eq!((found.held, found.refused), (0, 1), "{why}");
    }
    // The item read through its address alone is still the item as it was taken.
    let read_through = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(address(Var::Local(3))),
        offset: 4,
    };
    let mut was = setup(MAKE, MADE, 0);
    was.push(looped(
        0,
        field(0, 3),
        vec![eval(call(UNKNOWN, vec![read_through], 0))],
    ));
    let (_, found) = read(was);
    assert_eq!((found.held, found.refused), (1, 0));
}

#[test]
fn an_address_below_the_enumerator_is_harmless_where_it_is_read_through_or_its_reach_stops_short() {
    let below = |stmt: Stmt| {
        let mut was = vec![stmt];
        was.extend(site(MAKE, MADE, 5, 9));
        read(was).1
    };
    let loaded = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(address(Var::Local(4))),
        offset: 8,
    };
    let short = below(eval(call(HANDS, vec![address(Var::Local(4))], 0)));
    let read_through = below(eval(call(UNKNOWN, vec![loaded], 0)));
    assert_eq!((short.held, read_through.held), (1, 1));
    // `HANDS` reaches eight bytes: from two cells below, into nothing; from the cell below, no further.
    let further = below(eval(call(HANDS, vec![address(Var::Local(3))], 0)));
    assert_eq!(further.held, 1);
    let unbounded = below(eval(call(UNKNOWN, vec![address(Var::Local(4))], 0)));
    let kept = below(assign(Var::Local(12), address(Var::Local(4))));
    assert_eq!((unbounded.held, unbounded.refused), (0, 1));
    assert_eq!((kept.held, kept.refused), (0, 1));
}

#[test]
fn each_loop_a_variable_is_the_enumerator_of_is_read_where_the_loops_over_its_type_agree() {
    let mut was = site(MAKE, MADE, 0, 3);
    was.extend(site(MAKE, MADE_AGAIN, 0, 3));
    let (is, found) = read(was.clone());
    assert_eq!(found.held, 2);
    assert!(
        matches!(is.as_slice(), [Stmt::Sugar(_), Stmt::Sugar(_)]),
        "{is:?}"
    );
    assert_eq!(cfg::lower_body(is), cfg::lower_body(was));

    // Two makes over the one type: neither loop is read.
    let mut was = site(MAKE, MADE, 0, 3);
    was.extend(site(OTHER_MAKE, MADE_AGAIN, 5, 9));
    let (is, found) = read(was.clone());
    assert_eq!(is, was);
    assert_eq!((found.held, found.refused), (0, 2));
}

#[test]
fn an_item_a_getter_of_the_enumerators_type_reads_is_taken() {
    let got = |project: &Project| {
        let getter = call(CURRENT, vec![address(Var::Local(0))], 1);
        let mut was = setup(MAKE, MADE, 0);
        was.push(looped(0, assign(Var::Local(3), getter), vec![touch(3)]));
        let (is, found) = read_with(was.clone(), project, Options::default());
        assert_eq!(cfg::lower_body(is), cfg::lower_body(was));
        found
    };
    assert_eq!(got(&project()).held, 1);
    let mut free = project();
    free.members.remove(&key(CURRENT));
    assert_eq!((got(&free).held, got(&free).refused), (0, 1));
    let mut writes = project();
    writes
        .reach
        .insert(key(CURRENT), BTreeMap::from([(0, reach(0x18, Mode::Ref))]));
    assert_eq!(got(&writes).held, 0);
}

#[test]
fn the_collection_is_what_the_enumerator_is_made_from() {
    let simples = |stmts: Vec<Stmt>| -> Vec<ir::Stmt> {
        let each = stmts.into_iter().filter_map(|stmt| match stmt {
            Stmt::Simple(s) => Some(s),
            _ => None,
        });
        each.collect()
    };
    let mut held = simples(vec![eval(call(UNKNOWN, Vec::new(), 0))]);
    held.extend(simples(setup(MAKE, MADE, 0)));
    let (rest, source) = enumerated(&held, &advanced(0)).unwrap_or_else(|| panic!("{held:?}"));
    assert_eq!((rest, source), (&held[..1], &Expr::Var(Var::Param(0))));
    // A test that advances another variable says nothing of this one.
    assert_eq!(enumerated(&held, &advanced(5)), None);
    assert_eq!(enumerated(&held[..3], &advanced(0)), None);
}

#[test]
fn a_foreach_over_a_struct_enumerator_is_written_over_what_the_enumerator_is_made_from() {
    let (is, _) = read(site(MAKE, MADE, 0, 3));
    let wide = |cells: u32| VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(cells),
        name: None,
        single: false,
    };
    let f = tree::Function {
        head: Head {
            value_types: Default::default(),
            program: PROGRAM,
            at: 0x10,
            name: "func_000010".to_owned(),
            returns: Returns::Void,
            params: 1,
            locals: 16,
            vars: BTreeMap::from([(Var::Local(0), wide(3))]),
            incomplete: None,
        },
        body: is,
    };
    let names = Names::default();
    let project = project();
    let cx = Context::new(&names, &project, Options::default());
    let mut text = String::new();
    crate::gta6::script::print::function_in(&mut text, &f, &cx);
    assert!(text.contains(" in p0)"), "{text}");
    assert!(!text.contains("in &"), "{text}");
}

#[test]
fn an_enumerator_or_an_item_a_wider_variable_names_by_its_cells_stays_a_loop() {
    // `local_5`, of three cells, holds the item's cell where it is read after the loop.
    let (_, found) = read(site(MAKE, MADE, 0, 6));
    assert_eq!((found.held, found.refused), (1, 0));
    let mut was = site(MAKE, MADE, 0, 6);
    was.push(touch(5));
    let (is, found) = read(was.clone());
    assert_eq!((is, found.held, found.refused), (was, 0, 1));

    // `local_4`, read after the loop, holds the first cell of the enumerator in `local_5` where it is of two cells.
    let mut was = site(MAKE, MADE, 5, 3);
    was.push(touch(4));
    let (is, found) = read(was.clone());
    assert_eq!((found.held, found.refused), (1, 0));
    assert_ne!(is, was);
    let (is, found) = read_beside(was.clone(), &project(), Options::default(), &[(4, 2)]);
    assert_eq!((is, found.held, found.refused), (was, 0, 1));
}
