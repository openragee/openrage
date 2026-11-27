use super::*;
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{
    Const, FunctionKey, Returns, StoreKind, TypeKey, VarInfo, VmType, Width,
};
use crate::gta6::script::project::{FieldFacts, Member};
use crate::gta6::script::{Level, Names, Options};

const OBJECT: &str = "type_0123456789ABCDEF";

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn int(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

fn binary(op: BinOp, at: Kind, l: Expr, r: Expr) -> Expr {
    Expr::Binary {
        op,
        at,
        l: Box::new(l),
        r: Box::new(r),
    }
}

fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
    }
}

fn store(kind: StoreKind, addr: Expr, offset: u32, value: Expr) -> ir::Stmt {
    ir::Stmt::Store {
        kind,
        addr,
        offset,
        value,
    }
}

fn assign(target: Var, value: Expr) -> ir::Stmt {
    ir::Stmt::Assign { target, value }
}

/// A call of the function at `at` of the program itself.
fn call(at: u32, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args,
        returns: Some(1),
    }
}

fn does(at: u32, args: Vec<Expr>) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(call(at, args)))
}

fn simples(parts: &[ir::Stmt]) -> Vec<Stmt> {
    parts.iter().cloned().map(Stmt::Simple).collect()
}

fn ty(name: &str) -> VmType {
    VmType {
        name: name.to_owned(),
        byte_size: 32,
        id: 5,
        flags: 0x22,
    }
}

fn head(vars: &[(Var, Kind)]) -> Head {
    let info = |kind: &Kind| VarInfo {
        width: Width::Cell,
        kind: kind.clone(),
        name: None,
        single: false,
    };
    Head {
        value_types: Default::default(),
        program: 1,
        at: 0x10,
        name: "func_000010".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 8,
        vars: vars.iter().map(|(var, kind)| (*var, info(kind))).collect(),
        incomplete: None,
    }
}

/// One of this file's names and no other pass.
fn only(pass: Pass) -> Options {
    Options::at(Level::Lifted).with(pass)
}

fn rewritten(
    body: Vec<Stmt>,
    vars: &[(Var, Kind)],
    project: &Project,
    options: Options,
) -> Vec<Stmt> {
    rewritten_in(head(vars), body, project, options)
}

fn rewritten_in(head: Head, body: Vec<Stmt>, project: &Project, options: Options) -> Vec<Stmt> {
    let mut f = tree::Function { head, body };
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    run(&mut f, &cx, &mut Stats::default());
    f.body
}

fn compounded_alone(s: &ir::Stmt, vars: &[(Var, Kind)]) -> Vec<Stmt> {
    let body = vec![Stmt::Simple(s.clone())];
    rewritten(body, vars, Project::empty(), only(Pass::Compound))
}

/// `p0->f_10`, four bytes.
fn place() -> Expr {
    load(Scalar::I32, p(0), 0x10)
}

/// `p0->f_30`, a whole cell.
fn cell() -> Expr {
    load(Scalar::U64, p(0), 0x30)
}

/// `p0->f_8`, an address that is loaded.
fn far() -> Expr {
    load(Scalar::U64, p(0), 0x8)
}

#[test]
fn a_store_of_a_whole_cell_that_works_on_its_own_place_is_a_compound_assignment() {
    let u64s = StoreKind::U64;
    let through = load(Scalar::U64, far(), 0x30);
    let fired = vec![
        (
            "a flag set",
            store(
                u64s,
                p(0),
                0x30,
                binary(BinOp::Or, Kind::U64, cell(), int(4)),
            ),
        ),
        (
            "at no stated kind",
            store(
                u64s,
                p(0),
                0x30,
                binary(BinOp::Add, Kind::Unknown, cell(), int(4)),
            ),
        ),
        (
            "an unsigned division of the cell as it lies",
            store(
                u64s,
                p(0),
                0x30,
                binary(BinOp::Div, Kind::U64, cell(), int(2)),
            ),
        ),
        (
            "through a loaded address",
            store(
                u64s,
                far(),
                0x30,
                binary(BinOp::Or, Kind::U64, through, local(1)),
            ),
        ),
    ];
    for (what, s) in fired {
        let done = compounded_alone(&s, &[]);
        assert_eq!(
            done,
            vec![group(Group::Compound, vec![s.clone()])],
            "{what}"
        );
        assert_eq!(tree::desugar(done), vec![Stmt::Simple(s)], "{what}");
    }
}

#[test]
fn a_store_stays_a_store_where_the_compound_form_would_say_something_else() {
    let (i32s, u64s) = (StoreKind::I32, StoreKind::U64);
    let or = |l: Expr, r: Expr| binary(BinOp::Or, Kind::U64, l, r);
    let made = || call(0x100, Vec::new());
    let byte = Expr::cast(Scalar::I32, load(Scalar::U8, p(0), 0x10));
    let float = Expr::cast(Scalar::F64, cell());
    let half = Expr::Const(Const::f32(0.5));
    let stays = vec![
        // ⚠ Through a pointer of no proved type `p0->f_10 |= 4` would read as the whole cell.
        (
            "four bytes of no declared type",
            store(
                i32s,
                p(0),
                0x10,
                binary(BinOp::Or, Kind::I32, place(), int(4)),
            ),
        ),
        (
            "a byte of no declared type",
            store(
                StoreKind::U8,
                p(0),
                0x10,
                binary(BinOp::Add, Kind::I32, byte, int(1)),
            ),
        ),
        (
            "a whole cell worked on as a float",
            store(u64s, p(0), 0x30, binary(BinOp::Add, Kind::F64, float, half)),
        ),
        (
            "a signed division of the cell as it lies",
            store(
                u64s,
                p(0),
                0x30,
                binary(BinOp::Div, Kind::I64, cell(), int(2)),
            ),
        ),
        (
            "the place on the right",
            store(u64s, p(0), 0x30, or(int(4), cell())),
        ),
        (
            "another offset",
            store(u64s, p(0), 0x38, or(cell(), int(4))),
        ),
        (
            "another object",
            store(u64s, p(1), 0x30, or(cell(), int(4))),
        ),
        (
            "a narrower load",
            store(u64s, p(0), 0x30, or(load(Scalar::U8, p(0), 0x30), int(4))),
        ),
        (
            "an operator of another width",
            store(
                u64s,
                p(0),
                0x30,
                binary(BinOp::Or, Kind::I32, cell(), int(4)),
            ),
        ),
        (
            "a comparison",
            store(
                u64s,
                p(0),
                0x30,
                binary(BinOp::Lt, Kind::U64, cell(), int(4)),
            ),
        ),
        (
            "a reading that cuts bits",
            store(u64s, p(0), 0x30, or(Expr::cast(Scalar::U8, cell()), int(4))),
        ),
        (
            "an address a call yields",
            store(
                u64s,
                made(),
                0x30,
                or(load(Scalar::U64, made(), 0x30), int(4)),
            ),
        ),
        (
            "a loaded address beside a call",
            store(
                u64s,
                far(),
                0x30,
                or(load(Scalar::U64, far(), 0x30), made()),
            ),
        ),
    ];
    for (why, s) in stays {
        assert_eq!(compounded_alone(&s, &[]), vec![Stmt::Simple(s)], "{why}");
    }
}

#[test]
fn a_variable_assigned_itself_and_an_operand_is_a_compound_assignment() {
    let x = Var::Local(1);
    let read = || Expr::cast(Scalar::I32, local(1));
    let or = assign(x, binary(BinOp::Or, Kind::I32, read(), int(4)));
    let compound_of = |s: &ir::Stmt| vec![group(Group::Compound, vec![s.clone()])];
    let plain = |s: &ir::Stmt| vec![Stmt::Simple(s.clone())];

    assert_eq!(compounded_alone(&or, &[(x, Kind::I32)]), compound_of(&or));
    assert_eq!(compounded_alone(&or, &[(x, Kind::U32)]), compound_of(&or));
    // ⚠ Declared wider than the operator works, or of no proved kind and so a whole cell:
    // `x |= 4` would keep the bits this cuts.
    assert_eq!(compounded_alone(&or, &[(x, Kind::U64)]), plain(&or));
    assert_eq!(compounded_alone(&or, &[]), plain(&or));
    assert_eq!(compounded_alone(&or, &[(x, Kind::F32)]), plain(&or));

    // A whole cell worked on whole is one too, declared or not.
    let wide = assign(x, binary(BinOp::Or, Kind::U64, local(1), int(4)));
    assert_eq!(compounded_alone(&wide, &[]), compound_of(&wide));
    assert_eq!(compounded_alone(&wide, &[(x, Kind::I32)]), plain(&wide));

    // One up in 32 bits is the printer's own `x++`.
    let step = assign(x, binary(BinOp::Add, Kind::I32, read(), int(1)));
    let ints = head(&[(x, Kind::I32)]);
    assert_eq!(
        compound(&step, &ints, Project::empty(), None).map(|(op, _)| op),
        Some(BinOp::Add)
    );
    assert_eq!(compounded_alone(&step, &[(x, Kind::I32)]), plain(&step));

    // A division works at the sign the variable is declared with.
    let unsigned = Expr::cast(Scalar::U32, local(1));
    let halved = assign(x, binary(BinOp::Div, Kind::U32, unsigned, int(2)));
    assert_eq!(
        compounded_alone(&halved, &[(x, Kind::U32)]),
        compound_of(&halved)
    );
    assert_eq!(compounded_alone(&halved, &[(x, Kind::I32)]), plain(&halved));
    assert_eq!(compounded_alone(&halved, &[]), plain(&halved));

    let reversed = assign(x, binary(BinOp::Sub, Kind::I32, int(4), read()));
    assert_eq!(
        compounded_alone(&reversed, &[(x, Kind::I32)]),
        plain(&reversed)
    );
    let other = assign(x, binary(BinOp::Or, Kind::I32, local(2), int(4)));
    assert_eq!(compounded_alone(&other, &[(x, Kind::I32)]), plain(&other));
}

/// A project in which the type `owner` declares the `bytes` bytes at 0x10 a field of `kind`.
fn declared(kind: Kind, bytes: u32) -> (Project, TypeKey) {
    let owner = TypeKey(7);
    let mut project = Project::default();
    let field = FieldFacts {
        offset: 0x10,
        bytes,
        kind,
    };
    project.fields.insert(owner, vec![field]);
    (project, owner)
}

/// What `compound` makes of a store to the field at 0x10 of an object of `owner`, the field read as `ty`.
fn field_compounded(
    project: &Project,
    owner: TypeKey,
    kind: StoreKind,
    ty: Scalar,
    value: impl Fn(Expr) -> Expr,
) -> (Vec<Stmt>, ir::Stmt) {
    let read = Expr::sugar(ExprSugar::Field { owner }, load(ty, p(0), 0x10));
    let set = store(kind, p(0), 0x10, value(read));
    let body = vec![group(Group::Field { owner }, vec![set.clone()])];
    (rewritten(body, &[], project, only(Pass::Compound)), set)
}

#[test]
fn a_field_store_that_works_on_its_own_field_takes_the_compound_reading() {
    let or = |read: Expr| binary(BinOp::Or, Kind::I32, read, int(4));
    let (project, owner) = declared(Kind::I32, 4);
    let (done, set) = field_compounded(&project, owner, StoreKind::I32, Scalar::I32, or);
    assert_eq!(done, vec![group(Group::Compound, vec![set])]);

    // A byte the type declares is widened to an `int` and cut back, as C# does.
    let (bytes, owner) = declared(Kind::U8, 1);
    let up = |read: Expr| binary(BinOp::Add, Kind::I32, Expr::cast(Scalar::I32, read), int(1));
    let (done, set) = field_compounded(&bytes, owner, StoreKind::U8, Scalar::U8, up);
    assert_eq!(done, vec![group(Group::Compound, vec![set])]);

    // A signed division of a field declared signed is the field's own.
    let halve = |read: Expr| binary(BinOp::Div, Kind::I32, read, int(2));
    let (done, set) = field_compounded(&project, owner, StoreKind::I32, Scalar::I32, halve);
    assert_eq!(done, vec![group(Group::Compound, vec![set])]);

    let plain = vec![group(
        Group::Field { owner },
        vec![store(StoreKind::I32, p(0), 0x10, int(4))],
    )];
    let done = rewritten(plain.clone(), &[], &project, only(Pass::Compound));
    assert_eq!(done, plain);
}

#[test]
fn a_field_store_keeps_the_reading_its_field_does_not_declare() {
    let field = |set: ir::Stmt| vec![group(Group::Field { owner: TypeKey(7) }, vec![set])];
    let or = |read: Expr| binary(BinOp::Or, Kind::I32, read, int(4));

    // ⚠ No layout says what the four bytes are: the plain form's `(int)` is all that does.
    let (done, set) = field_compounded(
        Project::empty(),
        TypeKey(7),
        StoreKind::I32,
        Scalar::I32,
        or,
    );
    assert_eq!(done, field(set));

    // ⚠ Declared a `uint` and divided as an `int`: `/=` would divide unsigned.
    let (unsigned, owner) = declared(Kind::U32, 4);
    let halve = |read: Expr| binary(BinOp::Div, Kind::I32, read, int(2));
    let (done, set) = field_compounded(&unsigned, owner, StoreKind::I32, Scalar::I32, halve);
    assert_eq!(done, field(set));

    // ⚠ Declared a float and worked on as an `int`.
    let (float, owner) = declared(Kind::F32, 4);
    let (done, set) = field_compounded(&float, owner, StoreKind::I32, Scalar::I32, or);
    assert_eq!(done, field(set));
}

fn temporary() -> Var {
    Var::Value {
        at: 0x100,
        index: 0,
    }
}

fn made_object(target: Var) -> ir::Stmt {
    assign(target, Expr::NewObject(ty(OBJECT)))
}

fn field(object: Var, kind: StoreKind, offset: u32, value: Expr) -> ir::Stmt {
    store(kind, Expr::Var(object), offset, value)
}

fn initialised(body: Vec<Stmt>) -> Vec<Stmt> {
    rewritten(body, &[], Project::empty(), only(Pass::Initialisers))
}

#[test]
fn an_object_and_the_stores_straight_after_it_are_one_initialiser() {
    let v = temporary();
    let parts = vec![
        made_object(v),
        field(v, StoreKind::Ref, 0x8, p(0)),
        field(v, StoreKind::I32, 0x10, call(0x200, vec![local(2)])),
    ];
    let used = does(0x300, vec![Expr::Var(v)]);
    let mut body = simples(&parts);
    body.push(used.clone());

    let done = initialised(body.clone());
    assert_eq!(done, vec![group(Group::Construct, parts), used]);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn an_object_a_pass_before_constructed_takes_its_field_stores_too() {
    let v = temporary();
    let constructed = vec![
        made_object(v),
        ir::Stmt::Eval(call(0x200, vec![Expr::Var(v)])),
    ];
    let owner = TypeKey(7);
    let named = field(v, StoreKind::Ref, 0x8, p(0));
    let body = vec![
        group(Group::Construct, constructed.clone()),
        group(Group::Field { owner }, vec![named.clone()]),
    ];
    let mut parts = constructed;
    parts.push(named);
    assert_eq!(initialised(body), vec![group(Group::Construct, parts)]);
}

#[test]
fn an_initialiser_ends_before_a_store_the_source_form_would_run_otherwise() {
    let v = temporary();
    let first = field(v, StoreKind::Ref, 0x8, p(0));
    let moved = Expr::ByteOffset {
        addr: Box::new(Expr::Var(v)),
        offset: Box::new(int(8)),
    };
    let ends = vec![
        (
            "a value that reads the object",
            field(v, StoreKind::Ref, 0x18, Expr::Var(v)),
        ),
        ("a field stored twice", field(v, StoreKind::U8, 0xA, int(1))),
        (
            "a store to another object",
            store(StoreKind::Ref, p(1), 0x18, p(0)),
        ),
        (
            "a store over the object's type",
            field(v, StoreKind::U64, 0x0, int(1)),
        ),
        (
            "a store through a moved address",
            store(StoreKind::Ref, moved, 0x18, p(0)),
        ),
    ];
    for (why, last) in ends {
        let body = simples(&[made_object(v), first.clone(), last.clone()]);
        let held = group(Group::Construct, vec![made_object(v), first.clone()]);
        assert_eq!(initialised(body), vec![held, Stmt::Simple(last)], "{why}");
    }

    // Nothing joins across a statement that is no store.
    let apart = vec![
        Stmt::Simple(made_object(v)),
        does(0x300, Vec::new()),
        Stmt::Simple(first),
    ];
    assert_eq!(initialised(apart.clone()), apart);
}

#[test]
fn a_variable_a_pointer_reaches_takes_no_value_that_could_read_it() {
    let v = Var::Local(3);
    let loaded = field(v, StoreKind::Ref, 0x8, far());
    let body = |pointed: u32| {
        let pointer = Expr::AddrOf {
            var: Var::Local(pointed),
            cell: 0,
        };
        vec![
            does(0x300, vec![pointer]),
            Stmt::Simple(made_object(v)),
            Stmt::Simple(loaded.clone()),
        ]
    };
    // The address of a local reaches the locals after it.
    assert_eq!(initialised(body(2)), body(2));
    assert_eq!(made(&initialised(body(4))).objects, 1);

    // A value that reads no memory cannot tell when the variable was assigned.
    let constant = field(v, StoreKind::I32, 0x8, int(1));
    let mut reached = body(2);
    reached.pop();
    reached.push(Stmt::Simple(constant));
    assert_eq!(made(&initialised(reached)).objects, 1);
}

#[test]
fn an_initialiser_takes_the_stores_among_copies_and_assignments_that_cannot_tell() {
    let v = temporary();
    let (copy, kept) = (Var::Local(5), Var::Local(6));
    let keeps = assign(kept, p(1));
    let copies = assign(copy, Expr::Var(v));
    let used = does(0x300, vec![Expr::Var(copy)]);
    let mut body = simples(&[
        made_object(v),
        keeps.clone(),
        copies.clone(),
        field(v, StoreKind::I32, 0x8, Expr::Var(kept)),
        field(copy, StoreKind::Ref, 0x10, p(0)),
    ]);
    body.push(used.clone());

    // The assignment goes before the object is made, the copy after its last store, and the
    // store through the copy is a store to the object.
    let parts = vec![
        made_object(v),
        field(v, StoreKind::I32, 0x8, Expr::Var(kept)),
        field(v, StoreKind::Ref, 0x10, p(0)),
    ];
    let read = vec![
        Stmt::Simple(keeps),
        group(Group::Construct, parts),
        Stmt::Simple(copies),
        used,
    ];
    assert_eq!(initialised(body), read);

    // A call goes before the making where nothing stands between them.
    let called = assign(Var::Local(7), call(0x200, Vec::new()));
    let takes = field(v, StoreKind::Ref, 0x8, local(7));
    let body = simples(&[made_object(v), called.clone(), takes.clone()]);
    let read = vec![
        Stmt::Simple(called),
        group(Group::Construct, vec![made_object(v), takes]),
    ];
    assert_eq!(initialised(body), read);

    // What stands after the last store stays where it is.
    let stored = field(v, StoreKind::I32, 0x8, int(1));
    let body = simples(&[made_object(v), stored.clone(), copies_of(v), keeps_of()]);
    let read = vec![
        group(Group::Construct, vec![made_object(v), stored]),
        Stmt::Simple(copies_of(v)),
        Stmt::Simple(keeps_of()),
    ];
    assert_eq!(initialised(body), read);
}

/// `local_5 = object`.
fn copies_of(object: Var) -> ir::Stmt {
    assign(Var::Local(5), Expr::Var(object))
}

/// `local_6 = p1`.
fn keeps_of() -> ir::Stmt {
    assign(Var::Local(6), p(1))
}

#[test]
fn an_initialiser_moves_no_statement_that_could_tell() {
    let v = temporary();
    let stored = field(v, StoreKind::I32, 0x8, local(6));
    let last = field(v, StoreKind::I32, 0x10, int(1));
    let global = Var::Global {
        program: 1,
        offset: 8,
        statics: false,
    };
    let between = vec![
        (
            "an assignment a store before it reads",
            assign(Var::Local(6), int(2)),
        ),
        (
            "a call after a store",
            assign(Var::Local(7), call(0x200, Vec::new())),
        ),
        (
            "an assignment that reads the object",
            assign(Var::Local(7), load(Scalar::I32, Expr::Var(v), 0x8)),
        ),
        ("an assignment to a global", assign(global, int(2))),
        (
            "another object made",
            assign(Var::Local(7), Expr::NewObject(ty(OBJECT))),
        ),
    ];
    for (why, moved) in between {
        let body = simples(&[made_object(v), stored.clone(), moved.clone(), last.clone()]);
        let held = group(Group::Construct, vec![made_object(v), stored.clone()]);
        let stays = vec![held, Stmt::Simple(moved), Stmt::Simple(last.clone())];
        assert_eq!(initialised(body), stays, "{why}");
    }

    // No object is made before another that was made first.
    let other = assign(Var::Local(7), Expr::NewObject(ty(OBJECT)));
    let apart = simples(&[made_object(v), other, stored.clone()]);
    assert_eq!(initialised(apart.clone()), apart);

    // A copy assigned again is no name of the object any more.
    let again = simples(&[
        made_object(v),
        copies_of(v),
        assign(Var::Local(5), p(1)),
        field(Var::Local(5), StoreKind::I32, 0x8, int(1)),
    ]);
    assert_eq!(initialised(again.clone()), again);

    // ⚠ Past a constructor's call nothing moves that the call could read through a pointer.
    let constructed = vec![
        made_object(v),
        ir::Stmt::Eval(call(0x200, vec![Expr::Var(v)])),
    ];
    let body = |pointed: u32| {
        let pointer = Expr::AddrOf {
            var: Var::Local(pointed),
            cell: 0,
        };
        vec![
            does(0x300, vec![pointer]),
            group(Group::Construct, constructed.clone()),
            Stmt::Simple(keeps_of()),
            Stmt::Simple(stored.clone()),
        ]
    };
    assert_eq!(initialised(body(2)), body(2));
    let moved = initialised(body(7));
    assert_eq!(moved.get(1), Some(&Stmt::Simple(keeps_of())));
    assert_eq!(made(&moved).objects, 1);
}

fn made_array(target: Var, len: Expr) -> ir::Stmt {
    let made = Expr::NewArray {
        ty: ty("type_0"),
        len: Box::new(len),
        wrapper: "NewArray_type_0".to_owned(),
    };
    assign(target, made)
}

fn set(array: Var, index: i32, value: Expr) -> ir::Stmt {
    ir::Stmt::Eval(Expr::Call {
        callee: Callee::Intrinsic {
            name: "array_set_u32".to_owned(),
            ty: None,
            wrapper: Some("array_set_u32".to_owned()),
        },
        args: vec![Expr::Var(array), int(index), value],
        returns: Some(0),
    })
}

#[test]
fn an_array_and_every_element_set_in_order_are_one_initialiser() {
    let a = temporary();
    let parts = vec![
        made_array(a, int(2)),
        set(a, 0, local(1)),
        set(a, 1, int(7)),
    ];
    let used = does(0x300, vec![Expr::Var(a)]);
    let mut body = simples(&parts);
    body.push(used.clone());

    let done = initialised(body.clone());
    assert_eq!(done, vec![group(Group::Collection, parts), used]);
    assert_eq!(made(&done).arrays, 1);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn an_array_filled_in_part_or_out_of_order_stays_statements() {
    let a = temporary();
    let stays = vec![
        (
            "an element left out",
            vec![made_array(a, int(3)), set(a, 0, int(1)), set(a, 1, int(2))],
        ),
        (
            "out of order",
            vec![made_array(a, int(2)), set(a, 1, int(1)), set(a, 0, int(2))],
        ),
        (
            "an element that reads the array",
            vec![
                made_array(a, int(2)),
                set(a, 0, int(1)),
                set(a, 1, Expr::Var(a)),
            ],
        ),
        (
            "a length that is no constant",
            vec![made_array(a, local(1)), set(a, 0, int(1))],
        ),
        (
            "another array's element",
            vec![made_array(a, int(1)), set(Var::Local(5), 0, int(1))],
        ),
    ];
    for (why, parts) in stays {
        let body = simples(&parts);
        assert_eq!(initialised(body.clone()), body, "{why}");
    }
}

fn not_zero(at: Kind, e: Expr) -> Expr {
    binary(BinOp::Ne, at, e, int(0))
}

fn select(cond: Expr, then: Expr, els: Expr) -> Expr {
    Expr::Select {
        cond: Box::new(cond),
        then: Box::new(then),
        els: Box::new(els),
    }
}

/// What the null operators make of `value`, assigned to a local.
fn nulled(value: &Expr, vars: &[(Var, Kind)], project: &Project) -> Expr {
    let body = vec![Stmt::Simple(assign(Var::Local(1), value.clone()))];
    match rewritten(body, vars, project, only(Pass::NullOps)).pop() {
        Some(Stmt::Simple(ir::Stmt::Assign { value, .. })) => value,
        other => panic!("the statement stays an assignment: {other:?}"),
    }
}

/// A project in which the type `owner` has a field of `kind` at byte 0x18.
fn layout(kind: Kind) -> (Project, TypeKey) {
    let owner = TypeKey(0x0123_4567_89AB_CDEF);
    let mut project = Project::default();
    let field = FieldFacts {
        offset: 0x18,
        bytes: 8,
        kind,
    };
    project.fields.insert(owner, vec![field]);
    (project, owner)
}

#[test]
fn a_reference_or_what_stands_in_when_it_is_null_is_a_coalescing() {
    let x = Var::Local(2);
    let reference = Kind::Object(OBJECT.to_owned());
    let object = [(x, reference.clone())];
    let none = Project::empty();
    let other = || call(0x200, Vec::new());

    let read = select(not_zero(Kind::U64, local(2)), local(2), other());
    let coalesced = Expr::sugar(ExprSugar::Coalesce, read.clone());
    assert_eq!(nulled(&read, &object, none), coalesced);
    let mut back = coalesced;
    back.desugar();
    assert_eq!(back, read);

    // ⚠ A number that is not zero is no `??`, and an address a local holds is no reference.
    assert_eq!(nulled(&read, &[], none), read);
    assert_eq!(nulled(&read, &[(x, Kind::I64)], none), read);
    assert_eq!(nulled(&read, &[(x, Kind::Pointer)], none), read);

    let low = select(not_zero(Kind::U32, local(2)), local(2), other());
    assert_eq!(nulled(&low, &object, none), low, "32 bits tested");
    let both = [(x, reference.clone()), (Var::Local(3), reference)];
    let apart = select(not_zero(Kind::U64, local(2)), local(3), other());
    assert_eq!(nulled(&apart, &both, none), apart, "another value yielded");
    let is_null = binary(BinOp::Eq, Kind::U64, local(2), int(0));
    let swapped = select(is_null, local(2), other());
    assert_eq!(nulled(&swapped, &object, none), swapped, "tested for null");
}

#[test]
fn a_reference_field_of_what_was_tested_for_null_is_a_null_conditional() {
    let (project, owner) = layout(Kind::Object(OBJECT.to_owned()));
    let field = |base: Expr| Expr::sugar(ExprSugar::Field { owner }, load(Scalar::U64, base, 0x18));
    let tested = || not_zero(Kind::U64, local(2));

    let read = select(tested(), field(local(2)), int(0));
    let conditional = Expr::sugar(ExprSugar::NullConditional, read.clone());
    assert_eq!(nulled(&read, &[], &project), conditional);

    // A field a reference was stored to is one, though its kind says no more than an address.
    let (stored, _) = layout(Kind::Pointer);
    assert_eq!(nulled(&read, &[], &stored), conditional);

    // ⚠ A number read that way would be a nullable in C#.
    let (numbers, _) = layout(Kind::I64);
    assert_eq!(nulled(&read, &[], &numbers), read);
    let apart = select(tested(), field(local(3)), int(0));
    assert_eq!(
        nulled(&apart, &[], &project),
        apart,
        "another object's field"
    );
    let filled = select(tested(), field(local(2)), int(1));
    assert_eq!(nulled(&filled, &[], &project), filled, "not null otherwise");

    // A field is a reference by its layout, and is read twice: no call may be in it.
    let twice = |base: Expr| {
        select(
            not_zero(Kind::U64, field(base.clone())),
            field(base),
            int(1),
        )
    };
    let kept = twice(p(0));
    assert_eq!(
        nulled(&kept, &[], &project),
        Expr::sugar(ExprSugar::Coalesce, kept.clone())
    );
    let called = twice(call(0x200, Vec::new()));
    assert_eq!(nulled(&called, &[], &project), called);
}

#[test]
fn a_reference_or_its_stand_in_assigned_by_an_if_is_one_assignment() {
    let vars = [(Var::Local(2), Kind::Object(OBJECT.to_owned()))];
    let s = Var::Stack { at: 0x40, index: 0 };
    let kept = assign(s, local(2));
    let stand_in = assign(s, p(1));
    let either = |cond: Expr, then: &ir::Stmt, els: &ir::Stmt| {
        vec![Stmt::If {
            cond,
            then: simples(std::slice::from_ref(then)),
            els: simples(std::slice::from_ref(els)),
        }]
    };
    let read = |body: Vec<Stmt>, vars: &[(Var, Kind)]| {
        rewritten(body, vars, Project::empty(), only(Pass::NullOps))
    };
    let tested = || not_zero(Kind::U64, local(2));
    let choice = select(tested(), local(2), p(1));
    let one = vec![Stmt::Simple(assign(
        s,
        Expr::sugar(ExprSugar::Coalesce, choice.clone()),
    ))];
    assert_eq!(read(either(tested(), &kept, &stand_in), &vars), one);
    // The same `if` on the reference being null, its arms the other way round.
    let is_null = binary(BinOp::Eq, Kind::U64, local(2), int(0));
    assert_eq!(read(either(is_null, &stand_in, &kept), &vars), one);
    assert_eq!(made(&one).coalesce, 1);
    assert_eq!(
        tree::desugar(one),
        vec![Stmt::Simple(assign(s, choice))],
        "the choice without its reading"
    );

    // ⚠ Any other `if` stays an `if`: a number, two variables assigned, another value kept.
    let number = either(tested(), &kept, &stand_in);
    assert_eq!(read(number.clone(), &[]), number);
    let apart = either(tested(), &kept, &assign(Var::Local(3), p(1)));
    assert_eq!(read(apart.clone(), &vars), apart);
    let other = either(tested(), &assign(s, p(0)), &stand_in);
    assert_eq!(read(other.clone(), &vars), other);
    // ⚠ Nor is null a stand-in: `s = x ?? null` is a statement nobody writes.
    let nothing = either(tested(), &kept, &assign(s, int(0)));
    assert_eq!(read(nothing.clone(), &vars), nothing);
}

fn builtin(path: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 6,
            method: 1,
        },
        args,
        returns: Some(1),
    }
}

/// What the pass `pass` alone makes of `value`, evaluated as a statement.
fn read_alone(value: &Expr, vars: &[(Var, Kind)], project: &Project, pass: Pass) -> Expr {
    let body = vec![Stmt::Simple(ir::Stmt::Eval(value.clone()))];
    match rewritten(body, vars, project, only(pass)).pop() {
        Some(Stmt::Simple(ir::Stmt::Eval(value))) => value,
        other => panic!("the statement stays an evaluation: {other:?}"),
    }
}

#[test]
fn a_call_the_list_names_string_concat_over_a_proved_string_is_a_sum() {
    const CONCAT: &str = "System.String.Concat";
    let vars = [(Var::Local(2), Kind::String)];
    let read = |value: &Expr| read_alone(value, &vars, Project::empty(), Pass::Concat);
    let sum = |call: Expr| Expr::sugar(ExprSugar::Concat, call);

    let two = builtin(CONCAT, vec![p(1), local(2)]);
    assert_eq!(read(&two), sum(two.clone()));

    // A sum is a string to the sum it is a piece of, and so is a constant of the string heap.
    let text = Expr::StringConstant {
        program: 1,
        offset: 0x40,
    };
    let inner = builtin(CONCAT, vec![text, p(0)]);
    let outer = builtin(CONCAT, vec![inner.clone(), p(1), p(0)]);
    let nested = builtin(CONCAT, vec![sum(inner), p(1), p(0)]);
    assert_eq!(read(&outer), sum(nested));

    // ⚠ Between two numbers `+` would add, and no other name is that call.
    let stays = vec![
        (
            "no piece proved a string",
            builtin(CONCAT, vec![p(0), p(1)]),
        ),
        (
            "a string that is not one of the first two",
            builtin(CONCAT, vec![p(0), p(1), local(2)]),
        ),
        ("one piece", builtin(CONCAT, vec![local(2)])),
        (
            "another method",
            builtin("System.String.Glue", vec![local(2), p(1)]),
        ),
        (
            "another type's method of that name",
            builtin("Rope.Concat", vec![local(2), p(1)]),
        ),
        (
            "a function of the program",
            call(0x200, vec![local(2), p(1)]),
        ),
    ];
    for (why, value) in stays {
        assert_eq!(read(&value), value, "{why}");
    }
}

#[test]
fn the_address_of_a_variable_handed_to_a_call_is_a_ref() {
    let address = |n: u32| Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    };
    let by_ref = |n: u32| Expr::sugar(ExprSugar::Ref, address(n));
    let native = |args: Vec<Expr>| Expr::Call {
        callee: Callee::Native {
            namespace: None,
            name: "_0x0000000000000001".to_owned(),
            hash: 1,
            rva: 0x10,
        },
        args,
        returns: Some(1),
    };
    let none = Project::empty();
    // Locals 1 to 3 are each proved a variable of one cell; 4 to 7 are proved nothing.
    let frame = || {
        let mut head = head(&[]);
        for n in 1..4 {
            let mut info = VarInfo::new(Width::Cell);
            info.single = true;
            head.vars.insert(Var::Local(n), info);
        }
        head
    };
    let read_in = |head: Head, value: &Expr, project: &Project| {
        let body = vec![Stmt::Simple(ir::Stmt::Eval(value.clone()))];
        match rewritten_in(head, body, project, only(Pass::RefOut)).pop() {
            Some(Stmt::Simple(ir::Stmt::Eval(value))) => value,
            other => panic!("the statement stays an evaluation: {other:?}"),
        }
    };
    let read = |value: &Expr, project: &Project| read_in(frame(), value, project);

    // A native takes no receiver: every address that reaches its variable alone is a `ref`.
    let both = native(vec![address(1), p(0), address(2)]);
    assert_eq!(read(&both, none), native(vec![by_ref(1), p(0), by_ref(2)]));

    // ⚠ An address reaches the locals after its own up to the next proved to start a variable:
    // of a local proved nothing with such locals after it, it stays an address.
    let open = native(vec![address(4), address(5)]);
    assert_eq!(read(&open, none), open);
    assert_eq!(read_in(head(&[]), &both, none), both);
    // The last local of the frame has none after it, and a temporary is storage of its own.
    let temporary = Var::Value { at: 0x10, index: 0 };
    let of_temporary = Expr::AddrOf {
        var: temporary,
        cell: 0,
    };
    let ends = native(vec![address(7), of_temporary.clone()]);
    let by_ref_of = |address: Expr| Expr::sugar(ExprSugar::Ref, address);
    assert_eq!(
        read(&ends, none),
        native(vec![by_ref(7), by_ref_of(of_temporary)])
    );
    // A value of several cells ends where a proved local starts, and the last parameter at the frame's end.
    let mut wide = frame();
    let value = VarInfo {
        width: Width::Cells,
        kind: Kind::Aggregate(2),
        ..VarInfo::new(Width::Cells)
    };
    wide.vars.insert(Var::Local(4), value.clone());
    wide.vars.remove(&Var::Local(1));
    wide.vars.insert(Var::Local(0), value);
    let of_param = |n: u32| Expr::AddrOf {
        var: Var::Param(n),
        cell: 0,
    };
    let values = native(vec![address(0), address(4), of_param(0), of_param(1)]);
    assert_eq!(
        read_in(wide, &values, none),
        native(vec![
            by_ref(0),
            address(4),
            of_param(0),
            by_ref_of(of_param(1))
        ])
    );

    // ⚠ The first cell handed to a method not proved to take no receiver may be its receiver; a
    // function of no type takes none.
    let own = call(0x200, vec![address(1), address(2)]);
    let after_first = call(0x200, vec![address(1), by_ref(2)]);
    assert_eq!(read(&own, none), call(0x200, vec![by_ref(1), by_ref(2)]));
    let mut project = Project::default();
    let member = |is_static: bool| Member {
        owner: TypeKey(7),
        role: Role::Method,
        is_static,
        slot: None,
    };
    project
        .members
        .insert(FunctionKey::new(1, 0x200), member(false));
    assert_eq!(read(&own, &project), after_first);
    project
        .members
        .insert(FunctionKey::new(1, 0x200), member(true));
    assert_eq!(
        read(&own, &project),
        call(0x200, vec![by_ref(1), by_ref(2)])
    );
    assert_eq!(
        read(&builtin("Rope.Fill", vec![address(1), address(2)]), none),
        builtin("Rope.Fill", vec![address(1), by_ref(2)])
    );

    // A call read as a method is written on its first argument, and an operator with no call.
    let method = Expr::sugar(ExprSugar::Method, both.clone());
    let on_first = native(vec![address(1), p(0), by_ref(2)]);
    assert_eq!(
        read(&method, none),
        Expr::sugar(ExprSugar::Method, on_first)
    );
    let operator = Expr::sugar(ExprSugar::Operator, native(vec![address(1), address(2)]));
    assert_eq!(read(&operator, none), operator);

    // No `ref` of a global, of one cell of a variable, or to an operation of the machine.
    let global = Expr::AddrOf {
        var: Var::Global {
            program: 1,
            offset: 8,
            statics: false,
        },
        cell: 0,
    };
    let inner = Expr::AddrOf {
        var: Var::Local(1),
        cell: 1,
    };
    let apart = native(vec![global, inner]);
    assert_eq!(read(&apart, none), apart);
    let operation = Expr::Call {
        callee: Callee::Intrinsic {
            name: "array_set_u32".to_owned(),
            ty: None,
            wrapper: Some("array_set_u32".to_owned()),
        },
        args: vec![address(1), int(0), address(2)],
        returns: Some(0),
    };
    assert_eq!(read(&operation, none), operation);
    let mut back = read(&both, none);
    back.desugar();
    assert_eq!(back, both);
}

fn equals(value: &Expr, constant: i32) -> Expr {
    binary(BinOp::Eq, Kind::I32, value.clone(), int(constant))
}

/// `if (value == a) { func_a(); } else if (value == b) { func_b(); } .. else { default }`.
fn chain_of(value: &Expr, constants: &[i32], default: Vec<Stmt>) -> Vec<Stmt> {
    let mut els = default;
    for constant in constants.iter().rev() {
        els = vec![Stmt::If {
            cond: equals(value, *constant),
            then: vec![does(0x1000 + constant.unsigned_abs(), Vec::new())],
            els,
        }];
    }
    els
}

fn switched_alone(body: Vec<Stmt>) -> Vec<Stmt> {
    rewritten(body, &[], Project::empty(), only(Pass::SwitchOn))
}

/// The constants each arm of the one `switch` `done` is tests, and its default.
fn arms_of(done: &[Stmt]) -> (Vec<Vec<u64>>, &[Stmt]) {
    let [Stmt::Sugar(sugar)] = done else {
        panic!("one construct: {done:?}");
    };
    let Sugar::SwitchOn { arms, default } = &**sugar else {
        panic!("a switch: {sugar:?}");
    };
    let constants = (arms.iter())
        .map(|arm| match switch::test(&arm.test) {
            Some(switch::Test {
                what: switch::What::Equal(found),
                ..
            }) => found.iter().map(|(constant, _)| *constant).collect(),
            _ => Vec::new(),
        })
        .collect();
    (constants, default.as_slice())
}

#[test]
fn a_chain_of_tests_of_one_value_against_constants_is_a_switch() {
    let value = Expr::cast(Scalar::I32, place());
    let default = vec![does(0x200, Vec::new())];
    let body = chain_of(&value, &[1, 2, 3], default.clone());

    let done = switched_alone(body.clone());
    let (constants, held) = arms_of(&done);
    let each: Vec<Vec<u64>> = vec![vec![1], vec![2], vec![3]];
    assert_eq!(constants, each);
    assert_eq!(held, default.as_slice());
    assert_eq!(made(&done).switches, 1);
    assert_eq!(tree::desugar(done), body);

    // An arm of two constants, and a `break` that leaves a loop of the arm's own.
    let either = binary(
        BinOp::LogicalOr,
        Kind::Bool,
        equals(&value, 1),
        equals(&value, 4),
    );
    let looped = Stmt::While {
        cond: p(1),
        body: vec![Stmt::Break],
    };
    let body = vec![Stmt::If {
        cond: either,
        then: vec![looped],
        els: chain_of(&value, &[2, 3], Vec::new()),
    }];
    let done = switched_alone(body.clone());
    let each: Vec<Vec<u64>> = vec![vec![1, 4], vec![2], vec![3]];
    assert_eq!(arms_of(&done).0, each);
    assert_eq!(tree::desugar(done), body);

    // The chain ends where a test is of something else, which stays in the default.
    let tail = chain_of(&local(1), &[9], Vec::new());
    let done = switched_alone(chain_of(&value, &[1, 2, 3], tail.clone()));
    let (constants, held) = arms_of(&done);
    assert_eq!((constants.len(), held), (3, tail.as_slice()));
}

#[test]
fn a_chain_stays_ifs_where_a_switch_would_run_otherwise() {
    let value = place();
    let leaving = vec![Stmt::If {
        cond: equals(&value, 1),
        then: vec![Stmt::Break],
        els: chain_of(&value, &[2, 3], Vec::new()),
    }];
    let stays = vec![
        ("two arms", chain_of(&value, &[1, 2], Vec::new())),
        (
            "a constant tested twice",
            chain_of(&value, &[1, 2, 1], Vec::new()),
        ),
        (
            "a value a call yields",
            chain_of(&call(0x300, vec![p(0)]), &[1, 2, 3], Vec::new()),
        ),
        (
            "a third test of another value",
            chain_of(&value, &[1, 2], chain_of(&local(1), &[3], Vec::new())),
        ),
        ("an arm that leaves by a break", leaving),
        (
            "a default that leaves by a break",
            chain_of(&value, &[1, 2, 3], vec![Stmt::Break]),
        ),
    ];
    for (why, body) in stays {
        assert_eq!(switched_alone(body.clone()), body, "{why}");
    }

    // A float has no case labels to be one of.
    let float = |constant: i32| binary(BinOp::Eq, Kind::F32, value.clone(), int(constant));
    let mut body = Vec::new();
    for constant in [3, 2, 1] {
        body = vec![Stmt::If {
            cond: float(constant),
            then: Vec::new(),
            els: body,
        }];
    }
    assert_eq!(switched_alone(body.clone()), body);
}

#[test]
fn a_rewrite_runs_only_where_its_name_is_on() {
    let v = temporary();
    let vars = [(Var::Local(2), Kind::Object(OBJECT.to_owned()))];
    let body = || {
        let flags = binary(BinOp::Or, Kind::U64, cell(), int(4));
        let or_else = select(not_zero(Kind::U64, local(2)), local(2), int(1));
        let mut body = simples(&[
            made_object(v),
            field(v, StoreKind::Ref, 0x8, p(0)),
            store(StoreKind::U64, p(0), 0x30, flags),
            assign(Var::Local(1), or_else),
        ]);
        body.extend(chain_of(
            &load(Scalar::I32, p(0), 0x14),
            &[1, 2, 3],
            Vec::new(),
        ));
        body
    };
    let found = |options: Options| made(&rewritten(body(), &vars, Project::empty(), options));
    let all = Made {
        compound: 1,
        objects: 1,
        arrays: 0,
        switches: 1,
        coalesce: 1,
        null_conditional: 0,
    };
    let every = Options::at(Level::Source);
    assert_eq!(found(every), all);
    assert_eq!(
        found(every.without(Pass::Compound)),
        Made { compound: 0, ..all }
    );
    assert_eq!(
        found(every.without(Pass::Initialisers)),
        Made { objects: 0, ..all }
    );
    assert_eq!(
        found(every.without(Pass::SwitchOn)),
        Made { switches: 0, ..all }
    );
    assert_eq!(
        found(every.without(Pass::NullOps)),
        Made { coalesce: 0, ..all }
    );

    // With none of this file's names the body is what it was, and every reading comes off again.
    let baseline = rewritten(body(), &vars, Project::empty(), Options::baseline());
    assert_eq!(baseline, body());
    let read = rewritten(body(), &vars, Project::empty(), every);
    assert_eq!(tree::desugar(read), body());
}

#[path = "sugar_expressions_nested_tests.rs"]
mod inside;
