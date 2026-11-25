use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::ir::{Const, Returns, VmType};
use crate::gta6::script::program::{Area, FieldRow, FunctionRow};

const PROGRAM: u32 = 0x1122_3344;
const MADE: TypeKey = TypeKey(0x0123_4567_89AB_CDEF);
const BASE: TypeKey = TypeKey(0x0FED_CBA9_8765_4321);

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn set(target: Var, value: Expr) -> Stmt {
    Stmt::Assign { target, value }
}

fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
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

/// An object of the type `MADE`, of 32 bytes after its header.
fn made() -> Expr {
    Expr::NewObject(VmType {
        name: MADE.to_string(),
        byte_size: 32,
        id: 7,
        flags: 0x22,
    })
}

/// A function of one block at `at`, of two parameters and four locals.
fn function(at: u32, stmts: Vec<Stmt>) -> Function {
    let head = Head {
        program: PROGRAM,
        at,
        name: format!("func_{at:06X}"),
        returns: Returns::Void,
        params: 2,
        locals: 4,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

fn field(offset: u32, bytes: u32, kind: Kind) -> FieldFacts {
    FieldFacts {
        offset,
        bytes,
        kind,
    }
}

/// The settled fields of what `functions` show, each function read as a part of its own.
fn settled(functions: &[Function]) -> Project {
    let mut project = Project::default();
    for f in functions {
        let mut part = Project::default();
        of_function(&mut part, f);
        absorb(&mut project, &mut part);
    }
    settle(&mut project);
    project
}

#[test]
fn the_accesses_of_an_object_a_function_makes_are_its_types_fields() {
    let receiver = Callee::Virtual {
        class: "type_00000000000000AA".to_owned(),
        method: "Method_4".to_owned(),
        ty: "type_00000000000000AA".to_owned(),
        slot: 4,
    };
    let through = Expr::Call {
        callee: receiver,
        args: vec![load(Scalar::U64, local(0), 0x20)],
        returns: Some(0),
    };
    let body = || {
        vec![
            set(Var::Local(0), made()),
            store(StoreKind::I32, local(0), 8, int(3)),
            store(StoreKind::Ref, local(0), 0x10, param(1)),
            set(Var::Local(1), load(Scalar::F32, local(0), 0x18)),
            Stmt::Eval(through.clone()),
            // The header is no field.
            set(Var::Local(2), load(Scalar::U64, local(0), 0)),
        ]
    };
    let project = settled(&[function(0x40, body())]);
    let fields = vec![
        field(8, 4, Kind::Unknown),
        field(0x10, 8, Kind::Pointer),
        field(0x18, 4, Kind::F32),
        field(0x20, 8, Kind::Pointer),
    ];
    assert_eq!(project.fields, BTreeMap::from([(MADE, fields)]));

    // A variable that is also given something else holds no object of a known type.
    let mut also = body();
    also.push(set(Var::Local(0), param(0)));
    assert!(settled(&[function(0x40, also)]).fields.is_empty());
    // Nor does a local at or after one whose address is taken.
    let mut reached = body();
    let lower = Expr::AddrOf {
        var: Var::Local(0),
        cell: 0,
    };
    reached.push(set(Var::Local(3), lower));
    assert!(settled(&[function(0x40, reached)]).fields.is_empty());
}

#[test]
fn an_offset_seen_at_two_widths_has_no_field_and_two_kinds_leave_the_width() {
    let one = function(
        0x40,
        vec![
            set(Var::Local(0), made()),
            set(Var::Local(1), load(Scalar::I32, local(0), 0x10)),
            set(Var::Local(1), load(Scalar::U8, local(0), 0x18)),
            set(Var::Local(1), load(Scalar::F32, local(0), 0x1C)),
            store(StoreKind::Ref, local(0), 0x20, param(1)),
        ],
    );
    let other = function(
        0x80,
        vec![
            set(Var::Local(0), made()),
            set(Var::Local(1), load(Scalar::U64, local(0), 0x10)),
            store(StoreKind::U8, local(0), 0x18, int(1)),
            store(StoreKind::I32, local(0), 0x1C, int(7)),
            store(StoreKind::U64, local(0), 0x20, int(7)),
        ],
    );
    let project = settled(&[one, other]);
    let fields = vec![
        field(0x18, 1, Kind::U8),
        field(0x1C, 4, Kind::F32),
        // A reference in one function and a number in the other.
        field(0x20, 8, Kind::Unknown),
    ];
    assert_eq!(project.fields.get(&MADE), Some(&fields));
}

#[test]
fn a_field_that_lies_in_a_base_type_is_the_base_types() {
    let mut project = Project::default();
    let derived = TypeFacts {
        byte_size: 24,
        base: Some(BASE),
        ..TypeFacts::default()
    };
    let base = TypeFacts {
        byte_size: 8,
        ..TypeFacts::default()
    };
    project.types.insert(MADE, derived);
    project.types.insert(BASE, base);
    let seen = vec![
        field(8, 8, Kind::Pointer),
        field(0x10, 4, Kind::Unknown),
        // Past the 24 bytes the type has: no field of it.
        field(0x20, 4, Kind::Unknown),
    ];
    project.fields.insert(MADE, seen);
    // A receiver nothing came to read is no type.
    let waited = waiting(0x40).expect("an offset this low is a key");
    project
        .fields
        .insert(waited, vec![field(0, 0, Kind::Pointer)]);

    settle(&mut project);
    let own = vec![field(0x10, 4, Kind::Unknown)];
    let inherited = vec![field(8, 8, Kind::Pointer)];
    assert_eq!(
        project.fields,
        BTreeMap::from([(MADE, own), (BASE, inherited)])
    );
    assert_eq!(project.field(MADE, 8).map(|field| field.bytes), Some(8));
}

#[test]
fn the_receiver_of_a_function_one_types_slots_first_hold_is_that_type() {
    let ty = |name: TypeKey, base: Option<u32>, flags: u32, slots: &[u32]| TypeRow {
        name: name.0,
        byte_size: 24,
        flags,
        base,
        slots: slots.iter().copied().map(Some).collect(),
        ..TypeRow::default()
    };
    let (two, three, value) = (TypeKey(2 << 40), TypeKey(3 << 40), TypeKey(4 << 40));
    let mut derived = ty(MADE, Some(0), 0, &[0x40, 0x80]);
    derived.fields = vec![FieldRow {
        offset: 0x18,
        bytes: 8,
        ty: Some(0),
        ..FieldRow::default()
    }];
    let program = Program {
        program: PROGRAM,
        // Of four global cells, the second holds a reference.
        globals: Area {
            cells: 4,
            ..Area::default()
        },
        global_references: vec![0b0010],
        types: vec![
            ty(BASE, None, 0, &[0x40]),
            derived,
            // Two types that are no base of each other hold the function at 0xC0.
            ty(two, None, 0, &[0xC0]),
            ty(three, None, 0, &[0xC0]),
            ty(value, None, 0x04, &[0x100]),
        ],
        functions: [0x40, 0x80, 0xC0, 0x100]
            .map(|at| FunctionRow {
                at,
                ..FunctionRow::default()
            })
            .to_vec(),
        ..Program::default()
    };
    let reads = |at: u32, offset: u32| {
        let read = set(Var::Local(0), load(Scalar::I32, param(0), offset));
        function(at, vec![read])
    };

    let mut project = Project::default();
    project.programs.insert(PROGRAM, program.info());
    of_program(&mut project, &program);
    let globals = project.programs.get(&PROGRAM).map(|info| &info.globals);
    assert_eq!(globals, Some(&BTreeMap::from([(8, Kind::Pointer)])));
    of_function(&mut project, &reads(0x40, 8));
    let mut second = reads(0x80, 0x10);
    let stored = store(StoreKind::Ref, param(0), 0x18, param(1));
    second.cfg.blocks[0].stmts.push(stored);
    of_function(&mut project, &second);
    of_function(&mut project, &reads(0xC0, 8));
    of_function(&mut project, &reads(0x100, 8));
    assert!(project.fields.keys().all(|key| !waits(*key)));
    settle(&mut project);
    // The function the base type holds is the base's, though the derived type's slots hold it too.
    let own = vec![
        field(0x10, 4, Kind::Unknown),
        field(0x18, 8, Kind::Object(BASE.to_string())),
    ];
    let inherited = vec![field(8, 4, Kind::Unknown)];
    assert_eq!(
        project.fields,
        BTreeMap::from([(MADE, own), (BASE, inherited)])
    );

    // A receiver the function assigns is no longer the object it was passed.
    let mut project = Project::default();
    of_program(&mut project, &program);
    let mut assigned = reads(0x40, 8);
    let again = set(Var::Param(0), param(1));
    assigned.cfg.blocks[0].stmts.push(again);
    of_function(&mut project, &assigned);
    assert_eq!(project.fields.get(&BASE), None);
}
