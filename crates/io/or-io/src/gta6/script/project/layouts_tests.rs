use super::*;
use crate::gta6::script::cfg::{Block, Cfg, Function, Terminator};
use crate::gta6::script::ir::{Const, Returns, VmType, Width};
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
        value_types: Default::default(),
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

const UPPER: TypeKey = TypeKey(0x0A0A_0000_0000_0001);
const LOWER: TypeKey = TypeKey(0x0A0A_0000_0000_0002);
const APART: TypeKey = TypeKey(0x0A0A_0000_0000_0003);

/// `l op r`, the operator working at `at`.
fn worked(op: BinOp, at: Kind, l: Expr, r: Expr) -> Expr {
    Expr::Binary {
        op,
        at,
        l: Box::new(l),
        r: Box::new(r),
    }
}

/// A call of slot 4 of the type `ty` on `receiver`.
fn called(ty: TypeKey, receiver: Expr) -> Stmt {
    let callee = Callee::Virtual {
        class: ty.to_string(),
        method: "Method_4".to_owned(),
        ty: ty.to_string(),
        slot: 4,
    };
    Stmt::Eval(Expr::Call {
        callee,
        args: vec![receiver],
        returns: Some(0),
    })
}

#[test]
fn the_sign_every_operator_on_a_field_is_listed_at_is_the_fields() {
    let int_at = |offset: u32| load(Scalar::I32, local(0), offset);
    let uint_at = |offset: u32| load(Scalar::U32, local(0), offset);
    let reads = |value: Expr| set(Var::Local(1), value);
    let stepped = Stmt::StepI32 {
        addr: local(0),
        offset: 0x24,
        down: false,
    };
    let one = function(
        0x40,
        vec![
            set(Var::Local(0), made()),
            // Compared as an `int`.
            reads(worked(BinOp::Lt, Kind::I32, int_at(8), int(3))),
            // Masked as a `uint`, on the right of its operator.
            reads(worked(BinOp::And, Kind::U32, int(4), uint_at(0xC))),
            // Tested against zero, which either sign does alike, and handed on under no operator.
            reads(worked(BinOp::Ne, Kind::U32, uint_at(0x10), int(0))),
            reads(int_at(0x10)),
            // A count of bits to shift by.
            reads(worked(BinOp::Shl, Kind::I32, int(1), int_at(0x14))),
            // An `int` here and a `uint` in the other function.
            reads(worked(BinOp::Add, Kind::I32, int_at(0x18), int(1))),
            // A float here and an `int` in the other function.
            reads(load(Scalar::F32, local(0), 0x1C)),
            // One up is arithmetic.
            stepped,
        ],
    );
    let mut other = function(
        0x80,
        vec![
            set(Var::Local(0), made()),
            reads(worked(BinOp::Shr, Kind::U32, uint_at(0x18), int(2))),
            reads(worked(BinOp::Eq, Kind::I32, int_at(0x1C), int(0))),
        ],
    );
    // A switch reads its value as an `int`.
    other.cfg.blocks[0].term = Terminator::Switch {
        value: int_at(0x20),
        cases: Vec::new(),
        default: 0,
    };

    let project = settled(&[one, other]);
    let fields = vec![
        field(8, 4, Kind::I32),
        field(0xC, 4, Kind::U32),
        field(0x10, 4, Kind::Unknown),
        field(0x14, 4, Kind::Unknown),
        field(0x18, 4, Kind::Unknown),
        field(0x1C, 4, Kind::Unknown),
        field(0x20, 4, Kind::I32),
        field(0x24, 4, Kind::I32),
    ];
    assert_eq!(project.fields.get(&MADE), Some(&fields));
}

#[test]
fn a_field_whose_object_is_called_is_of_the_most_derived_type_it_is_called_as() {
    let held = |offset: u32| load(Scalar::U64, local(0), offset);
    let body = vec![
        set(Var::Local(0), made()),
        // As a type and as the type it derives from.
        called(UPPER, held(8)),
        called(LOWER, held(8)),
        // As two types neither of which is the other's.
        called(UPPER, held(0x10)),
        called(APART, held(0x10)),
        // As one type, with a reference of no known type stored.
        called(UPPER, held(0x18)),
        store(StoreKind::Ref, local(0), 0x18, param(1)),
        // As a builtin, whose slot says nothing.
        called(TypeKey(0x8000_0012), held(0x20)),
        // As a type, though a number is stored there.
        called(LOWER, held(0x28)),
        store(StoreKind::U64, local(0), 0x28, int(7)),
    ];
    let mut project = Project::default();
    let lower = TypeFacts {
        base: Some(UPPER),
        ..TypeFacts::default()
    };
    project.types.insert(LOWER, lower);
    project.types.insert(UPPER, TypeFacts::default());
    project.types.insert(APART, TypeFacts::default());
    let mut part = Project::default();
    of_function(&mut part, &function(0x40, body));
    absorb(&mut project, &mut part);
    settle(&mut project);

    let fields = vec![
        field(8, 8, Kind::Object(LOWER.to_string())),
        field(0x10, 8, Kind::Pointer),
        field(0x18, 8, Kind::Object(UPPER.to_string())),
        field(0x20, 8, Kind::Pointer),
        field(0x28, 8, Kind::Unknown),
    ];
    assert_eq!(project.fields.get(&MADE), Some(&fields));
    // Settled again, the fields are the fields they were.
    settle(&mut project);
    assert_eq!(project.fields.get(&MADE), Some(&fields));
}

#[test]
fn a_byte_only_ever_stored_a_truth_and_tested_for_zero_is_a_bool() {
    let byte = |offset: u32, value: Expr| store(StoreKind::U8, local(0), offset, value);
    let tested = |offset: u32| {
        let read = load(Scalar::U8, local(0), offset);
        Expr::binary(BinOp::Ne, read, int(0))
    };
    let less = Expr::binary(BinOp::Lt, param(0), param(1));
    let stores = vec![
        set(Var::Local(0), made()),
        byte(0x10, int(1)),
        byte(0x10, less),
        // Stored a value of any byte, and read as a number.
        byte(0x11, param(1)),
        byte(0x12, int(0)),
    ];
    let reads = vec![
        set(Var::Local(0), made()),
        set(Var::Local(1), tested(0x10)),
        set(Var::Local(1), tested(0x11)),
        set(
            Var::Local(1),
            Expr::binary(BinOp::Add, load(Scalar::U8, local(0), 0x12), int(1)),
        ),
    ];
    let project = settled(&[function(0x10, stores), function(0x20, reads)]);
    let kind = |offset: u32| project.field(MADE, offset).map(|field| field.kind.clone());
    assert_eq!(kind(0x10), Some(Kind::Bool));
    assert_eq!(kind(0x11), Some(Kind::U8));
    assert_eq!(kind(0x12), Some(Kind::U8));
    assert!(project.truths.is_empty());
}

#[test]
fn a_variable_typed_by_the_project_and_a_members_receiver_add_their_accesses_once_every_program_is_in(
) {
    use crate::gta6::script::class::Role;
    use crate::gta6::script::ir::VarInfo;
    use crate::gta6::script::project::Member;
    // Made in one function: the type's field at 8 is an `int` there.
    let made_here = function(
        0x10,
        vec![
            set(Var::Local(0), made()),
            Stmt::StepI32 {
                addr: local(0),
                offset: 8,
                down: false,
            },
        ],
    );
    let mut project = settled(&[made_here]);
    assert_eq!(
        project.field(MADE, 8).map(|f| f.kind.clone()),
        Some(Kind::I32)
    );
    assert_eq!(project.field(MADE, 0x10), None);
    project.types.insert(
        MADE,
        TypeFacts {
            byte_size: 32,
            ..TypeFacts::default()
        },
    );
    // A local the kinds pass types as a `MADE`, and the receiver of a member of `MADE`.
    let mut typed = function(
        0x20,
        vec![set(Var::Local(2), load(Scalar::F32, local(1), 0x10))],
    );
    let mut info = VarInfo::new(crate::gta6::script::ir::Width::Cell);
    info.kind = Kind::Object(MADE.to_string());
    typed.head.vars.insert(Var::Local(1), info);
    let member = function(0x30, vec![store(StoreKind::U8, param(0), 0x18, int(1))]);
    let owner = Member {
        owner: MADE,
        role: Role::Method,
        is_static: false,
        slot: None,
    };
    project
        .members
        .insert(FunctionKey::new(PROGRAM, 0x30), owner);
    let mut part = Project::default();
    of_typed_function(&mut part, &typed, &project);
    of_typed_function(&mut part, &member, &project);
    refine(&mut project, vec![part]);
    let kind = |offset: u32| {
        project
            .field(MADE, offset)
            .map(|f| (f.bytes, f.kind.clone()))
    };
    assert_eq!(kind(8), Some((4, Kind::I32)));
    assert_eq!(kind(0x10), Some((4, Kind::F32)));
    assert_eq!(kind(0x18), Some((1, Kind::Bool)));
}

const VALUE: TypeKey = TypeKey(0x0A0A_0000_0000_0005);
const BUILTIN: TypeKey = TypeKey(0x0A0A_0000_0000_0006);

#[test]
fn the_receiver_of_a_value_types_member_points_at_the_value_whose_bytes_are_its_own_fields() {
    use crate::gta6::script::class::Role;
    use crate::gta6::script::project::Member;
    let at = |offset: u32| load(Scalar::I32, param(0), offset);
    let body = vec![
        // Masked as a `uint`, compared as an `int`.
        set(
            Var::Local(0),
            worked(BinOp::Or, Kind::U32, load(Scalar::U32, param(0), 0), int(4)),
        ),
        set(Var::Local(0), worked(BinOp::Lt, Kind::I32, at(4), int(3))),
        // A byte stored a truth and tested for zero.
        store(StoreKind::U8, param(0), 8, int(1)),
        set(
            Var::Local(1),
            Expr::binary(BinOp::Ne, load(Scalar::U8, param(0), 8), int(0)),
        ),
        set(Var::Local(2), at(0xC)),
    ];
    let member = function(0x30, body);
    // Of twelve bytes, held in place, on a type that would hold the first 72 bytes of an object.
    let typed = |flags: u32| {
        let mut project = Project::default();
        let own = TypeFacts {
            byte_size: 12,
            flags,
            base: Some(BUILTIN),
            ..TypeFacts::default()
        };
        let above = TypeFacts {
            byte_size: 64,
            ..TypeFacts::default()
        };
        project.types.insert(VALUE, own);
        project.types.insert(BUILTIN, above);
        let owner = Member {
            owner: VALUE,
            role: Role::Method,
            is_static: false,
            slot: None,
        };
        project
            .members
            .insert(FunctionKey::new(PROGRAM, 0x30), owner);
        project
    };
    let refined = |mut project: Project, f: &Function| {
        let mut part = Project::default();
        of_typed_function(&mut part, f, &project);
        refine(&mut project, vec![part]);
        project.fields
    };

    // The value's bytes from the first are its fields, and none past its twelve is.
    let fields = vec![
        field(0, 4, Kind::U32),
        field(4, 4, Kind::I32),
        field(8, 1, Kind::Bool),
    ];
    assert_eq!(
        refined(typed(0x04), &member),
        BTreeMap::from([(VALUE, fields)])
    );
    // An object's first eight bytes are its header: what lies past them within its base type's
    // bytes is the base type's.
    let inherited = vec![field(8, 1, Kind::Bool), field(0xC, 4, Kind::Unknown)];
    assert_eq!(
        refined(typed(0), &member),
        BTreeMap::from([(BUILTIN, inherited)])
    );
    // A function that takes no receiver says nothing of the type.
    let mut project = typed(0x04);
    if let Some(owner) = project.members.get_mut(&FunctionKey::new(PROGRAM, 0x30)) {
        owner.is_static = true;
    }
    assert!(refined(project, &member).is_empty());
}

#[test]
fn a_field_objects_are_stored_in_and_no_call_is_made_on_is_a_reference_and_no_more() {
    let body = vec![
        set(Var::Local(0), made()),
        set(Var::Local(1), made()),
        // An object made in place, and one a variable holds.
        store(StoreKind::Ref, local(0), 8, made()),
        store(StoreKind::Ref, local(0), 0x10, local(1)),
        // An object stored beside a reference of no known type.
        store(StoreKind::Ref, local(0), 0x18, local(1)),
        store(StoreKind::Ref, local(0), 0x18, param(1)),
        // Objects of two types neither of which is the other's.
        store(StoreKind::Ref, local(0), 0x20, local(1)),
        called(APART, load(Scalar::U64, local(0), 0x20)),
    ];
    let mut project = Project::default();
    project.types.insert(APART, TypeFacts::default());
    let mut part = Project::default();
    of_function(&mut part, &function(0x40, body));
    absorb(&mut project, &mut part);
    settle(&mut project);
    // ⚠ A store through a base of no known type is never seen: what the stores seen hold names
    // no type the field is.
    let fields = vec![
        field(8, 8, Kind::Pointer),
        field(0x10, 8, Kind::Pointer),
        field(0x18, 8, Kind::Pointer),
        field(0x20, 8, Kind::Pointer),
    ];
    assert_eq!(project.fields.get(&MADE), Some(&fields));
}

/// An object of the type `key`.
fn object(key: TypeKey) -> Expr {
    Expr::NewObject(VmType {
        name: key.to_string(),
        byte_size: 24,
        id: 9,
        flags: 0x22,
    })
}

#[test]
fn a_reference_field_is_of_the_most_derived_type_its_calls_are_made_as_that_every_object_stored_in_it_is(
) {
    // `LOWER` and `APART` both derive from `UPPER`.
    let below = |base: TypeKey| TypeFacts {
        base: Some(base),
        ..TypeFacts::default()
    };
    let mut project = Project::default();
    project.types.insert(UPPER, TypeFacts::default());
    project.types.insert(LOWER, below(UPPER));
    project.types.insert(APART, below(UPPER));
    let body = vec![
        set(Var::Local(0), made()),
        // An object of a type and one of the type it derives from.
        store(StoreKind::Ref, local(0), 8, object(LOWER)),
        store(StoreKind::Ref, local(0), 8, object(UPPER)),
        // Objects of two types that derive from one.
        store(StoreKind::Ref, local(0), 0x10, object(LOWER)),
        store(StoreKind::Ref, local(0), 0x10, object(APART)),
        // An object of a type, and a call made as the type below it.
        store(StoreKind::Ref, local(0), 0x18, object(UPPER)),
        called(LOWER, load(Scalar::U64, local(0), 0x18)),
        // An object of a type, and a call made as the type above it.
        store(StoreKind::Ref, local(0), 0x20, object(LOWER)),
        called(UPPER, load(Scalar::U64, local(0), 0x20)),
    ];
    let mut part = Project::default();
    of_function(&mut part, &function(0x40, body));
    absorb(&mut project, &mut part);
    settle(&mut project);
    let of = |key: TypeKey| Kind::Object(key.to_string());
    // Objects stored and no call: no type. A call made as the type below the object's: no type.
    let fields = vec![
        field(8, 8, Kind::Pointer),
        field(0x10, 8, Kind::Pointer),
        field(0x18, 8, Kind::Pointer),
        field(0x20, 8, of(UPPER)),
    ];
    assert_eq!(project.fields.get(&MADE), Some(&fields));
    // Settled again from the rows it was settled from, with nothing more.
    let rows = project.rows.clone();
    refine(&mut project, Vec::new());
    assert_eq!(project.fields.get(&MADE), Some(&fields));
    assert_eq!(project.rows, rows);
}

#[test]
fn a_field_stored_arrays_or_strings_is_a_reference_and_no_more() {
    let array = |key: TypeKey, flags: u32| Expr::NewArray {
        ty: VmType {
            name: key.to_string(),
            byte_size: 12,
            id: 3,
            flags,
        },
        len: Box::new(int(4)),
        wrapper: "NewArray".to_owned(),
    };
    let text = || Expr::StringConstant {
        program: PROGRAM,
        offset: 0x10,
    };
    let body = vec![
        set(Var::Local(0), made()),
        store(StoreKind::Ref, local(0), 8, array(APART, 0)),
        store(StoreKind::Ref, local(0), 8, array(APART, 0)),
        // Of values held in place.
        store(StoreKind::Ref, local(0), 0x10, array(APART, 0x04)),
        // Arrays of two element types, and an array beside a string.
        store(StoreKind::Ref, local(0), 0x18, array(APART, 0)),
        store(StoreKind::Ref, local(0), 0x18, array(UPPER, 0)),
        store(StoreKind::Ref, local(0), 0x20, array(APART, 0)),
        store(StoreKind::Ref, local(0), 0x20, text()),
        store(StoreKind::Ref, local(0), 0x28, text()),
        // An array beside a reference of no known kind, and one beside a null.
        store(StoreKind::Ref, local(0), 0x30, array(APART, 0)),
        store(StoreKind::Ref, local(0), 0x30, param(1)),
        store(StoreKind::Ref, local(0), 0x38, array(APART, 0)),
        store(StoreKind::Ref, local(0), 0x38, int(0)),
    ];
    let project = settled(&[function(0x40, body)]);
    // ⚠ A store through a base of no known type is never seen: the arrays and the strings stored
    // that are seen name nothing a field holds.
    let offsets = [8, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38];
    let fields: Vec<FieldFacts> = (offsets.iter())
        .map(|offset| field(*offset, 8, Kind::Pointer))
        .collect();
    assert_eq!(project.fields.get(&MADE), Some(&fields));
}

#[test]
fn a_byte_stored_a_truth_and_kept_whole_in_a_bool_is_a_bool() {
    use crate::gta6::script::ir::VarInfo;
    let stores = function(
        0x10,
        vec![
            set(Var::Local(0), made()),
            store(StoreKind::U8, local(0), 0x10, int(1)),
            store(StoreKind::U8, local(0), 0x11, int(0)),
        ],
    );
    let mut reads = function(
        0x20,
        vec![
            set(Var::Local(0), made()),
            set(Var::Local(1), load(Scalar::U8, local(0), 0x10)),
            // Kept in a variable of no proved kind.
            set(Var::Local(2), load(Scalar::U8, local(0), 0x11)),
        ],
    );
    let mut truth = VarInfo::new(Width::Cell);
    truth.kind = Kind::Bool;
    reads.head.vars.insert(Var::Local(1), truth);
    let project = settled(&[stores, reads]);
    let kind = |offset: u32| project.field(MADE, offset).map(|field| field.kind.clone());
    assert_eq!(kind(0x10), Some(Kind::Bool));
    assert_eq!(kind(0x11), Some(Kind::U8));
}

#[test]
fn a_four_byte_field_stored_an_integer_variable_takes_its_sign_unless_another_access_says_the_other(
) {
    use crate::gta6::script::ir::VarInfo;
    let mut one = function(
        0x10,
        vec![
            set(Var::Local(0), made()),
            store(StoreKind::I32, local(0), 8, local(1)),
            store(StoreKind::I32, local(0), 0xC, local(2)),
            store(StoreKind::I32, local(0), 0x10, local(1)),
            // A variable of no proved kind, and a sum, whose cast to the other sign stores the same bits.
            store(StoreKind::I32, local(0), 0x14, local(3)),
            store(
                StoreKind::I32,
                local(0),
                0x18,
                worked(BinOp::Add, Kind::I32, local(1), int(1)),
            ),
        ],
    );
    for (n, kind) in [(1, Kind::I32), (2, Kind::U32)] {
        let mut info = VarInfo::new(Width::Cell);
        info.kind = kind;
        one.head.vars.insert(Var::Local(n), info);
    }
    let other = function(
        0x20,
        vec![
            set(Var::Local(0), made()),
            // Shifted as a `uint`.
            set(
                Var::Local(1),
                worked(
                    BinOp::Shr,
                    Kind::U32,
                    load(Scalar::U32, local(0), 0x10),
                    int(2),
                ),
            ),
        ],
    );
    let project = settled(&[one, other]);
    let fields = vec![
        field(8, 4, Kind::I32),
        field(0xC, 4, Kind::U32),
        field(0x10, 4, Kind::Unknown),
        field(0x14, 4, Kind::Unknown),
        field(0x18, 4, Kind::Unknown),
    ];
    assert_eq!(project.fields.get(&MADE), Some(&fields));
}

#[test]
fn a_typed_local_no_address_handed_on_reaches_is_a_base_and_a_parameter_only_its_signature_types_is_not(
) {
    use crate::gta6::script::ir::VarInfo;
    use crate::gta6::script::project::reach::Reach;
    use crate::gta6::script::project::{Param, Signature};
    let mut project = Project::default();
    let facts = TypeFacts {
        byte_size: 32,
        ..TypeFacts::default()
    };
    project.types.insert(MADE, facts);
    let seen = Some(vec![field(0x10, 4, Kind::F32)]);
    let read = |project: &Project, f: &Function| {
        let mut part = Project::default();
        of_typed_function(&mut part, f, project);
        part.fields.get(&MADE).cloned()
    };

    // The address of `local_1`, handed to the function at 0x80, then a read through `local_3`.
    let callee = FunctionKey::new(PROGRAM, 0x80);
    let handed = Stmt::Eval(Expr::Call {
        callee: Callee::Local {
            at: 0x80,
            name: "func_000080".to_owned(),
        },
        args: vec![Expr::AddrOf {
            var: Var::Local(1),
            cell: 0,
        }],
        returns: Some(0),
    });
    let read_through = set(Var::Local(2), load(Scalar::F32, local(3), 0x10));
    let mut typed = function(0x20, vec![handed, read_through]);
    let mut info = VarInfo::new(Width::Cell);
    info.kind = Kind::Object(MADE.to_string());
    typed.head.vars.insert(Var::Local(3), info);
    let reaching = |bytes: u32| {
        let mut held = project.clone();
        let cells = BTreeMap::from([(
            0,
            Reach {
                bytes,
                dies: true,
                ..Reach::default()
            },
        )]);
        held.reach.insert(callee, cells);
        held
    };
    // With no reach known, the address may reach every local after it.
    assert_eq!(read(&project, &typed), None);
    // One cell from `local_1`, and two, miss `local_3`; four reach it, and so does an address nothing bounds.
    assert_eq!(read(&reaching(8), &typed), seen);
    assert_eq!(read(&reaching(16), &typed), seen);
    assert_eq!(read(&reaching(32), &typed), None);
    assert_eq!(read(&reaching(reach::UNBOUNDED), &typed), None);

    // ⚠ A parameter only the function's signature gives an object of `MADE` is none: the kinds
    // pass weighs what the calls pass with the function's own code, and its kind is the proof.
    let own = FunctionKey::new(PROGRAM, 0x30);
    let one = |kind: Kind| Param { kind, cells: 1 };
    let signature = Signature {
        params: vec![one(Kind::Object(MADE.to_string())), one(Kind::Unknown)],
        returns: 0,
        result: Kind::Unknown,
    };
    project.signatures.insert(own, signature);
    let through = function(
        0x30,
        vec![set(Var::Local(0), load(Scalar::F32, param(0), 0x10))],
    );
    assert_eq!(read(&project, &through), None);
    let mut kinded = through.clone();
    let mut info = VarInfo::new(Width::Cell);
    info.kind = Kind::Pointer;
    kinded.head.vars.insert(Var::Param(0), info);
    assert_eq!(read(&project, &kinded), None);
}
