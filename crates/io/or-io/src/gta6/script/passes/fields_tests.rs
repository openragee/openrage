use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{Const, Head, Kind, Returns, Scalar, StoreKind, VmType};
use crate::gta6::script::project::{FieldFacts, Member, TypeFacts};
use crate::gta6::script::{Names, Options};

const PROGRAM: u32 = 0x1122_3344;
const AT: u32 = 0x40;
const MADE: TypeKey = TypeKey(0x0123_4567_89AB_CDEF);
const BASE: TypeKey = TypeKey(0x0FED_CBA9_8765_4321);

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn set(target: Var, value: Expr) -> tree::Stmt {
    tree::Stmt::Simple(ir::Stmt::Assign { target, value })
}

fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
    }
}

/// `store_i32(addr, offset, 3)`.
fn store(addr: Expr, offset: u32) -> ir::Stmt {
    ir::Stmt::Store {
        kind: StoreKind::I32,
        addr,
        offset,
        value: Expr::Const(Const::i32(3)),
    }
}

/// An object of the type `MADE`.
fn made() -> Expr {
    Expr::NewObject(VmType {
        name: MADE.to_string(),
        byte_size: 24,
        id: 7,
        flags: 0x22,
    })
}

fn function(body: Vec<tree::Stmt>) -> tree::Function {
    let head = Head {
        program: PROGRAM,
        at: AT,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 4,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    tree::Function { head, body }
}

fn field(offset: u32, bytes: u32) -> FieldFacts {
    FieldFacts {
        offset,
        bytes,
        kind: Kind::Unknown,
    }
}

/// `MADE`, of four bytes at 0x10, on `BASE`, of eight bytes at 8.
fn project() -> Project {
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
    project.fields.insert(MADE, vec![field(0x10, 4)]);
    project.fields.insert(BASE, vec![field(8, 8)]);
    project
}

fn read(mut f: tree::Function, project: &Project, options: Options) -> tree::Function {
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    run(&mut f, &cx, &mut Stats::default());
    f
}

fn of(owner: TypeKey, load: Expr) -> Expr {
    Expr::sugar(ExprSugar::Field { owner }, load)
}

#[test]
fn an_access_of_an_object_the_function_makes_is_its_types_field() {
    let body = || {
        vec![
            set(Var::Local(0), made()),
            tree::Stmt::Simple(store(local(0), 0x10)),
            tree::Stmt::If {
                cond: load(Scalar::U64, local(0), 8),
                then: vec![set(Var::Local(1), load(Scalar::I32, local(0), 0x10))],
                els: Vec::new(),
            },
            // As wide as no field at its offset, and at an offset of no field.
            set(Var::Local(1), load(Scalar::U64, local(0), 0x10)),
            set(Var::Local(1), load(Scalar::I32, local(0), 0x14)),
            tree::Stmt::Simple(store(local(0), 8)),
        ]
    };
    let project = project();
    let done = read(function(body()), &project, Options::default());

    let kind = Group::Field { owner: MADE };
    let parts = vec![store(local(0), 0x10)];
    let stored = tree::Stmt::Sugar(Box::new(Sugar::Group { kind, parts }));
    // The field of the base type is read as the base type's.
    let tested = tree::Stmt::If {
        cond: of(BASE, load(Scalar::U64, local(0), 8)),
        then: vec![set(
            Var::Local(1),
            of(MADE, load(Scalar::I32, local(0), 0x10)),
        )],
        els: Vec::new(),
    };
    let mut wanted = body();
    wanted[1] = stored;
    wanted[2] = tested;
    assert_eq!(done.body, wanted);
    // The way back is the body as it was.
    assert_eq!(tree::desugar(done.body), body());

    // Switched off, the pass reads nothing.
    let off = Options::default().without(Pass::Fields);
    assert_eq!(read(function(body()), &project, off).body, body());

    let counted = Found {
        loads: 2,
        stores: 1,
    };
    assert_eq!(apply(&mut function(body()), &project), counted);
}

#[test]
fn a_variable_that_may_hold_anything_else_is_no_base() {
    let project = project();
    let reads = || {
        vec![
            set(Var::Local(1), made()),
            set(Var::Local(2), load(Scalar::I32, local(1), 0x10)),
        ]
    };
    let done = read(function(reads()), &project, Options::default());
    assert_ne!(done.body, reads());

    // Given a parameter as well.
    let mut also = reads();
    also.push(set(Var::Local(1), param(1)));
    let done = read(function(also.clone()), &project, Options::default());
    assert_eq!(done.body, also);
    // A local at or after one whose address is taken.
    let mut reached = reads();
    let lower = Expr::AddrOf {
        var: Var::Local(0),
        cell: 0,
    };
    reached.push(set(Var::Local(3), lower));
    let done = read(function(reached.clone()), &project, Options::default());
    assert_eq!(done.body, reached);
    // Two widths seen at the offset, in a project nothing has settled.
    let mut unsettled = project.clone();
    unsettled
        .fields
        .insert(MADE, vec![field(0x10, 4), field(0x10, 8)]);
    let done = read(function(reads()), &unsettled, Options::default());
    assert_eq!(done.body, reads());
}

#[test]
fn the_receiver_of_a_member_is_an_object_of_the_members_type() {
    let member = |is_static: bool| Member {
        owner: MADE,
        role: Role::Method,
        is_static,
        slot: None,
    };
    let key = FunctionKey::new(PROGRAM, AT);
    let reads = || vec![set(Var::Local(0), load(Scalar::I32, param(0), 0x10))];
    let mut project = project();
    // Of no type, the receiver is any pointer.
    let done = read(function(reads()), &project, Options::default());
    assert_eq!(done.body, reads());

    project.members.insert(key, member(false));
    let done = read(function(reads()), &project, Options::default());
    let wanted = set(Var::Local(0), of(MADE, load(Scalar::I32, param(0), 0x10)));
    assert_eq!(done.body, [wanted]);

    // A receiver the function assigns is no longer the object it was passed.
    let mut assigned = reads();
    assigned.push(set(Var::Param(0), param(1)));
    let done = read(function(assigned.clone()), &project, Options::default());
    assert_eq!(done.body, assigned);
    // A function that takes no receiver, and a type held in place, whose method takes a pointer to the value.
    project.members.insert(key, member(true));
    let done = read(function(reads()), &project, Options::default());
    assert_eq!(done.body, reads());
    project.members.insert(key, member(false));
    if let Some(ty) = project.types.get_mut(&MADE) {
        ty.flags = 0x04;
    }
    let done = read(function(reads()), &project, Options::default());
    assert_eq!(done.body, reads());
}
