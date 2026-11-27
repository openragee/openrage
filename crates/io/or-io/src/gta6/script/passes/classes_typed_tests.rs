use std::collections::BTreeMap;

use super::super::{absorb, disputed, of_function, provisional, settle, unseen};
use super::*;
use crate::gta6::script::cfg::{self, Block, Cfg, Terminator};
use crate::gta6::script::ir::{self, Returns, Scalar, StoreKind, VarInfo, VmType};
use crate::gta6::script::project::{Param, Signature, TypeFacts};

const PROGRAM: u32 = 0x51;
/// A builtin type: the base of a class that derives from no script type.
const OBJECT: TypeKey = TypeKey(0x8000_0000);
/// A class of 24 bytes; the function at 0x100 fills its slot 4.
const BASE: TypeKey = TypeKey(0x61 << 40);
/// A class made from `BASE` of 40 bytes; the functions at 0x110 and 0x120 fill its slots 5 and 6.
const DERIVED: TypeKey = TypeKey(0x62 << 40);
/// A class under no script type.
const OTHER: TypeKey = TypeKey(0x63 << 40);
/// An interface: it states no base.
const SHAPE: TypeKey = TypeKey(0x64 << 40);

fn key(at: u32) -> FunctionKey {
    FunctionKey::new(PROGRAM, at)
}

fn head(at: u32) -> Head {
    Head {
        value_types: Default::default(),
        program: PROGRAM,
        at,
        name: format!("func_{at:06X}"),
        returns: Returns::Void,
        params: 2,
        locals: 1,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

/// A function of two parameters whose one block holds `stmts`.
fn function(at: u32, stmts: Vec<ir::Stmt>) -> cfg::Function {
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    cfg::Function {
        head: head(at),
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

/// A call of the program's own function at `at` that yields `returns` cells.
fn called(at: u32, args: Vec<Expr>, returns: Option<u32>) -> Expr {
    let callee = Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    };
    Expr::Call {
        callee,
        args,
        returns,
    }
}

fn call(at: u32, args: Vec<Expr>) -> ir::Stmt {
    ir::Stmt::Eval(called(at, args, None))
}

/// A call of slot `slot` as the type `ty` declares it.
fn slot(ty: TypeKey, slot: u32, args: Vec<Expr>) -> ir::Stmt {
    let callee = Callee::Virtual {
        class: ty.to_string(),
        method: format!("Method_{slot}"),
        ty: ty.to_string(),
        slot,
    };
    ir::Stmt::Eval(Expr::Call {
        callee,
        args,
        returns: None,
    })
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

/// A class of `PROGRAM` under `base` whose objects hold `byte_size` bytes past their header.
fn class(base: TypeKey, byte_size: u32, slots: &[(u32, u32)]) -> TypeFacts {
    TypeFacts {
        byte_size,
        base: Some(base),
        home: PROGRAM,
        slots: slots.iter().map(|&(slot, at)| (slot, key(at))).collect(),
        ..TypeFacts::default()
    }
}

/// A settled signature of two parameters of one cell, the first of kind `first`.
fn taking(first: Kind) -> Signature {
    let param = |kind: Kind| Param { kind, cells: 1 };
    Signature {
        params: vec![param(first), param(Kind::Unknown)],
        returns: 0,
        result: Kind::Unknown,
    }
}

fn of(ty: TypeKey) -> Kind {
    Kind::Object(ty.to_string())
}

/// The types of these tests, and the signature of the function at 0x200 where one is given.
fn project(first: Option<Kind>) -> Project {
    let mut project = Project::default();
    project.types.insert(OBJECT, TypeFacts::default());
    project.types.insert(BASE, class(OBJECT, 24, &[(4, 0x100)]));
    let derived = class(BASE, 40, &[(5, 0x110), (6, 0x120)]);
    project.types.insert(DERIVED, derived);
    project.types.insert(OTHER, class(OBJECT, 24, &[]));
    let shape = TypeFacts {
        home: PROGRAM,
        ..TypeFacts::default()
    };
    project.types.insert(SHAPE, shape);
    if let Some(first) = first {
        project.signatures.insert(key(0x200), taking(first));
    }
    project
}

/// What the project settles on for the function at 0x200 from the code of `functions`.
fn member(project: Project, functions: &[cfg::Function]) -> Option<Member> {
    let mut project = project;
    for f in functions {
        of_function(&mut project, f);
    }
    settle(&mut project);
    project.members.get(&key(0x200)).cloned()
}

fn method(owner: TypeKey) -> Option<Member> {
    Some(provisional(owner, Role::Method))
}

/// A function that hands the function at 0x200 `first` and its own second parameter.
fn hands_on(at: u32, first: Expr) -> cfg::Function {
    function(at, vec![call(0x200, vec![first, p(1)])])
}

#[test]
fn a_function_every_call_hands_an_object_its_signature_states_is_a_method_of_that_type() {
    let works = function(0x200, vec![writes(8)]);
    let typed = Some(of(BASE));
    let by_local = [hands_on(0x300, local(0)), works.clone()];
    assert_eq!(member(project(typed.clone()), &by_local), method(BASE));
    // The value handed is what one call yields, under a cast that keeps its cell.
    let yielded = Expr::cast(Scalar::U64, called(0x400, Vec::new(), Some(1)));
    let by_result = [hands_on(0x300, yielded), works.clone()];
    assert_eq!(member(project(typed.clone()), &by_result), method(BASE));

    // A signature that states no object of a script type says nothing of what is handed.
    let builtin = Kind::Object(OBJECT.to_string());
    for first in [
        None,
        Some(Kind::Pointer),
        Some(Kind::Unknown),
        Some(builtin),
    ] {
        assert_eq!(member(project(first.clone()), &by_local), None, "{first:?}");
    }

    // Its own code types its first parameter, or reaches nothing through it: the signature's kind is that code's.
    let mut own = works.clone();
    let info = VarInfo {
        width: Width::Cell,
        kind: of(BASE),
        name: None,
        single: false,
    };
    own.head.vars.insert(Var::Param(0), info);
    let idle = function(0x200, Vec::new());
    for f in [own, idle] {
        let functions = [hands_on(0x300, local(0)), f];
        assert_eq!(member(project(typed.clone()), &functions), None);
    }

    // A caller hands it a parameter, a global, a choice, a value of several cells, what a call yields in cells it does not state.
    let global = Expr::Var(Var::Global {
        program: PROGRAM,
        offset: 8,
        statics: false,
    });
    let choice = Expr::Select {
        cond: Box::new(p(1)),
        then: Box::new(local(0)),
        els: Box::new(p(1)),
    };
    let unstated = called(0x400, Vec::new(), None);
    let mut wide = hands_on(0x310, local(0));
    let cells = VarInfo {
        width: Width::Cells,
        kind: Kind::Unknown,
        name: None,
        single: false,
    };
    wide.head.vars.insert(Var::Local(0), cells);
    let mut odd: Vec<cfg::Function> = [p(1), global, choice, unstated]
        .into_iter()
        .map(|first| hands_on(0x310, first))
        .collect();
    odd.push(wide);
    for other in odd {
        let functions = [hands_on(0x300, local(0)), other.clone(), works.clone()];
        assert_eq!(
            member(project(typed.clone()), &functions),
            None,
            "{other:?}"
        );
    }

    // It is referred to; another function runs it, first, on an object just made.
    let referred = ir::Stmt::Assign {
        target: Var::Local(0),
        value: Expr::FunctionRef {
            program: PROGRAM,
            at: 0x200,
        },
    };
    let made = ir::Stmt::Assign {
        target: Var::Value { at: 1, index: 0 },
        value: Expr::NewObject(VmType {
            name: BASE.to_string(),
            byte_size: 24,
            id: 1,
            flags: 0,
        }),
    };
    let run = call(0x200, vec![Expr::Var(Var::Value { at: 1, index: 0 })]);
    for other in [
        function(0x310, vec![referred]),
        function(0x310, vec![made, run]),
    ] {
        let functions = [hands_on(0x300, local(0)), other.clone(), works.clone()];
        assert_eq!(
            member(project(typed.clone()), &functions),
            None,
            "{other:?}"
        );
    }
}

#[test]
fn the_objects_callers_hand_and_the_receivers_members_hand_are_of_one_type_together() {
    // A method of DERIVED hands it its receiver, a function its own object; it stores past BASE.
    let works = function(0x200, vec![writes(32)]);
    let functions = [
        hands_on(0x110, p(0)),
        hands_on(0x300, local(0)),
        works.clone(),
    ];
    assert_eq!(
        member(project(Some(of(DERIVED))), &functions),
        method(DERIVED)
    );
    // Objects of a type no script type both are under, and of a base the field is past.
    for first in [of(OTHER), of(BASE)] {
        assert_eq!(member(project(Some(first)), &functions), None);
    }
}

#[test]
fn a_class_whose_slot_a_function_calls_on_its_receiver_is_one_the_type_declaring_it_is_made_from() {
    // A method of DERIVED hands it its receiver; it stores to a field BASE holds, so either may declare it.
    let with = |also: Vec<ir::Stmt>| {
        let mut stmts = vec![writes(8)];
        stmts.extend(also);
        let functions = [hands_on(0x110, p(0)), function(0x200, stmts)];
        member(project(None), &functions)
    };
    assert_eq!(with(Vec::new()), None);
    // A slot only DERIVED has, called on the receiver; a function DERIVED's table holds, handed it.
    assert_eq!(with(vec![slot(DERIVED, 5, vec![p(0)])]), method(DERIVED));
    assert_eq!(with(vec![call(0x120, vec![p(0), p(1)])]), method(DERIVED));
    // A slot of BASE, which both are made from; a slot of DERIVED called on something else; a slot of an interface.
    for also in [
        slot(BASE, 4, vec![p(0)]),
        slot(DERIVED, 5, vec![p(1)]),
        slot(SHAPE, 0, vec![p(0)]),
    ] {
        assert_eq!(with(vec![also.clone()]), None, "{also:?}");
    }
    // A class the receivers are not made from leaves no type to declare it.
    assert_eq!(with(vec![slot(OTHER, 2, vec![p(0)])]), None);
    let project = project(None);
    let needs = [DERIVED];
    let declares = |arrived: TypeKey, needs: &[TypeKey]| {
        super::super::declarer(&project, arrived, key(0x200), Some(12), needs)
    };
    assert_eq!(declares(DERIVED, &needs), Some(DERIVED));
    assert_eq!(declares(BASE, &needs), None);
    assert_eq!(declares(DERIVED, &[]), None);
}

#[test]
fn what_two_parts_gathered_of_the_values_callers_hand_is_settled_together() {
    let merged = |works: cfg::Function| {
        let mut project = project(Some(of(BASE)));
        for f in [works, hands_on(0x300, local(0))] {
            let mut part = Project::default();
            of_function(&mut part, &f);
            absorb(&mut project, &mut part);
        }
        settle(&mut project);
        let left = project.programs.values();
        assert!(left
            .into_iter()
            .all(|row| row.carried == Carried::default()));
        project.members.get(&key(0x200)).cloned()
    };
    let works = function(0x200, vec![writes(8)]);
    assert_eq!(merged(works.clone()), method(BASE));
    // One part says its own code types its first parameter.
    let mut own = works;
    let info = VarInfo {
        width: Width::Cell,
        kind: of(BASE),
        name: None,
        single: false,
    };
    own.head.vars.insert(Var::Param(0), info);
    assert_eq!(merged(own), None);
}

#[test]
fn a_function_of_a_program_whose_code_was_not_all_read_is_handed_anything() {
    let mut carried = Carried::default();
    carried
        .typed
        .insert(key(0x200), BTreeSet::from([key(0x300)]));
    carried.blind.insert(PROGRAM);
    let mut gathered = BTreeMap::new();
    unseen(&Project::default(), &carried, &mut gathered);
    assert_eq!(gathered.get(&key(0x200)), Some(&disputed()));
}

#[test]
fn a_value_of_the_callers_own_is_a_local_a_result_or_an_object_made() {
    let mut head = head(0x300);
    let cells = VarInfo {
        width: Width::Cells,
        kind: Kind::Unknown,
        name: None,
        single: false,
    };
    head.vars.insert(Var::Local(1), cells);
    let made = Expr::NewObject(VmType {
        name: BASE.to_string(),
        byte_size: 24,
        id: 1,
        flags: 0,
    });
    let first = |e: Expr| handed_value(&[e, p(1)], &head);
    assert!(first(local(0)));
    assert!(first(Expr::cast(Scalar::U64, local(0))));
    assert!(first(Expr::Var(Var::Stack { at: 4, index: 0 })));
    assert!(first(called(0x400, Vec::new(), Some(1))));
    assert!(first(made));
    for other in [
        p(0),
        local(1),
        Expr::cast(Scalar::I32, local(0)),
        called(0x400, Vec::new(), Some(2)),
    ] {
        assert!(!first(other.clone()), "{other:?}");
    }
    assert!(!handed_value(&[], &head));
}
