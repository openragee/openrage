use std::collections::BTreeMap;

use super::*;
use crate::gta6::script::cfg::{self, Block, Cfg, Terminator};
use crate::gta6::script::ir::{self, Callee, Head, Returns, Scalar, StoreKind, VmType};
use crate::gta6::script::project::FieldFacts;

const PROGRAM: u32 = 0x73;
/// A builtin type: the base of a class that derives from no script type.
const OBJECT: TypeKey = TypeKey(0x8000_0000);
/// A class whose slot 4 the function at 0x100 fills; its field at 0x10 holds a reference.
const HOLDER: TypeKey = TypeKey(0x71 << 40);
/// A class of 24 bytes, whose field at 0x10 holds a reference too.
const PART: TypeKey = TypeKey(0x72 << 40);
/// A class of 24 bytes.
const PIECE: TypeKey = TypeKey(0x74 << 40);

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

/// The eight bytes at `offset` of the first parameter.
fn field(offset: u32) -> Expr {
    Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(p(0)),
        offset,
    }
}

/// A call of the program's own function at `at`.
fn call(at: u32, args: Vec<Expr>) -> ir::Stmt {
    let callee = Callee::Local {
        at,
        name: format!("func_{at:06X}"),
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

/// A function that hands the function at `to` its receiver's field at `offset`, and its second parameter.
fn hands_field(at: u32, to: u32, offset: u32) -> cfg::Function {
    function(at, vec![call(to, vec![field(offset), p(1)])])
}

/// A class of `PROGRAM` of 24 bytes under the builtin base.
fn class(slots: &[(u32, u32)]) -> TypeFacts {
    TypeFacts {
        byte_size: 24,
        base: Some(OBJECT),
        home: PROGRAM,
        slots: slots.iter().map(|&(slot, at)| (slot, key(at))).collect(),
        ..TypeFacts::default()
    }
}

/// A project that states the three classes and the reference fields `fields`, of their owner, offset and kind.
fn stated(fields: &[(TypeKey, u32, Kind)]) -> Project {
    let mut project = Project::default();
    project.types.insert(OBJECT, TypeFacts::default());
    project.types.insert(HOLDER, class(&[(4, 0x100)]));
    project.types.insert(PART, class(&[]));
    project.types.insert(PIECE, class(&[]));
    for (owner, offset, kind) in fields {
        let field = FieldFacts {
            offset: *offset,
            bytes: 8,
            kind: kind.clone(),
        };
        project.fields.entry(*owner).or_default().push(field);
    }
    project
}

/// What `project` settles on for each function at `ats` from the code of `functions`.
fn said(mut project: Project, functions: &[cfg::Function], ats: &[u32]) -> Vec<Option<Member>> {
    for f in functions {
        super::super::of_function(&mut project, f);
    }
    super::super::settle(&mut project);
    assert!(project
        .programs
        .values()
        .all(|row| row.carried == Carried::default()));
    ats.iter()
        .map(|at| project.members.get(&key(*at)).cloned())
        .collect()
}

fn method(owner: TypeKey) -> Member {
    provisional(owner, Role::Method)
}

#[test]
fn a_field_of_the_receiver_whole_is_what_a_call_hands_first() {
    let receiver = Some(Var::Param(0));
    assert_eq!(handed(&[field(0x10), p(1)], receiver), Some(0x10));
    let widened = Expr::Cast {
        to: Scalar::U64,
        e: Box::new(field(0x18)),
    };
    assert_eq!(handed(&[widened], receiver), Some(0x18));
    let narrow = Expr::Load {
        ty: Scalar::I32,
        addr: Box::new(p(0)),
        offset: 0x10,
    };
    let elsewhere = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(p(1)),
        offset: 0x10,
    };
    for first in [narrow, elsewhere, p(0)] {
        assert_eq!(
            handed(std::slice::from_ref(&first), receiver),
            None,
            "{first:?}"
        );
    }
    assert_eq!(handed(&[field(0x10)], None), None);
    assert_eq!(handed(&[], receiver), None);
}

/// The fields the project types, and the functions of a case.
type Case = (Vec<(TypeKey, u32, Kind)>, Vec<cfg::Function>);

#[test]
fn a_function_every_call_hands_a_typed_field_of_a_members_receiver_is_a_method_of_its_type() {
    let holds = |ty: TypeKey| Kind::Object(ty.to_string());
    let typed = [(HOLDER, 0x10, holds(PART))];
    // A method of HOLDER hands the function at 0x200 its field at 0x10, which holds a PART.
    let direct = [
        hands_field(0x100, 0x200, 0x10),
        function(0x200, vec![writes(8)]),
    ];
    let settled = said(stated(&typed), &direct, &[0x200]);
    assert_eq!(settled, [Some(method(PART))]);

    // That function hands its own field on: the function it hands it to waits for it, and is PIECE's.
    let both = [(HOLDER, 0x10, holds(PART)), (PART, 0x10, holds(PIECE))];
    let onward = || call(0x210, vec![field(0x10), p(1)]);
    let on = [
        hands_field(0x100, 0x200, 0x10),
        function(0x200, vec![writes(8), onward()]),
        function(0x210, vec![writes(8)]),
    ];
    let settled = said(stated(&both), &on, &[0x200, 0x210]);
    assert_eq!(settled, [Some(method(PART)), Some(method(PIECE))]);

    let referred = ir::Stmt::Assign {
        target: Var::Local(0),
        value: Expr::FunctionRef {
            program: PROGRAM,
            at: 0x200,
        },
    };
    let fresh = Var::Value { at: 1, index: 0 };
    let part = VmType {
        name: PART.to_string(),
        byte_size: 24,
        id: 1,
        flags: 0,
    };
    let made = ir::Stmt::Assign {
        target: fresh,
        value: Expr::NewObject(part),
    };
    let runs = call(0x200, vec![Expr::Var(fresh), p(1)]);
    // Each of these leaves it a function like any other.
    let left: [Case; 7] = [
        // The project types the field as nothing, or as a builtin type.
        (vec![(HOLDER, 0x10, Kind::Unknown)], direct.to_vec()),
        (vec![(HOLDER, 0x10, holds(OBJECT))], direct.to_vec()),
        // A function no type claims hands it its field too.
        (
            typed.to_vec(),
            vec![
                hands_field(0x100, 0x200, 0x10),
                hands_field(0x300, 0x200, 0x10),
                function(0x200, vec![writes(8)]),
            ],
        ),
        // It is handed something else, or referred to.
        (
            typed.to_vec(),
            vec![
                hands_field(0x100, 0x200, 0x10),
                function(0x110, vec![call(0x200, vec![p(1), p(1)])]),
                function(0x200, vec![writes(8)]),
            ],
        ),
        (
            typed.to_vec(),
            vec![
                hands_field(0x100, 0x200, 0x10),
                function(0x110, vec![referred]),
                function(0x200, vec![writes(8)]),
            ],
        ),
        // It is run on an object just made too: no constructor either.
        (
            typed.to_vec(),
            vec![
                hands_field(0x100, 0x200, 0x10),
                function(0x110, vec![made, runs]),
                function(0x200, vec![writes(8)]),
            ],
        ),
        // Two functions only ever hand each other their fields.
        (
            both.to_vec(),
            vec![
                function(0x200, vec![writes(8), onward()]),
                function(0x210, vec![call(0x200, vec![field(0x10), p(1)])]),
            ],
        ),
    ];
    for (fields, functions) in &left {
        let settled = said(stated(fields), functions, &[0x200]);
        assert_eq!(settled, [None], "{fields:?} {functions:?}");
    }

    // A field whose type another program states holds no function of this one.
    let mut elsewhere = stated(&typed);
    if let Some(facts) = elsewhere.types.get_mut(&PART) {
        facts.home = PROGRAM + 1;
    }
    assert_eq!(said(elsewhere, &direct, &[0x200]), [None]);
}
