use super::*;
use crate::gta6::script::class::{Method, Role};
use crate::gta6::script::ir::{FunctionKey, Head, StoreKind, VarInfo, VmType, Width};
use crate::gta6::script::print::tests::{
    address, assign, at, constant_of, function_of, head, high, int, intrinsic, kinds, load, local,
    native, not, param, simple, written_in,
};
use crate::gta6::script::print::{bound_in_place, method, method_in, Lambdas};
use crate::gta6::script::project::{FieldFacts, Member};
use crate::gta6::script::tree::{Arm, Guard, Label, Sugar};
use crate::gta6::script::{Context, Level, Names, Options, Project};

fn group(kind: Group, parts: Vec<Stmt>) -> tree::Stmt {
    tree::Stmt::Sugar(Box::new(Sugar::Group { kind, parts }))
}

fn sugar(sugar: Sugar) -> tree::Stmt {
    tree::Stmt::Sugar(Box::new(sugar))
}

fn object() -> VmType {
    VmType {
        name: "type_00000000000000AB".to_owned(),
        byte_size: 32,
        id: 7,
        flags: 0,
    }
}

fn local_call(at: u32, args: Vec<Expr>) -> Expr {
    let name = format!("func_{at:06X}");
    Expr::Call {
        callee: Callee::Local { at, name },
        args,
        returns: Some(1),
    }
}

fn builtin(path: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 2,
            method: 0,
        },
        args,
        returns: Some(1),
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

fn used(args: Vec<Expr>) -> tree::Stmt {
    simple(Stmt::Eval(native("USE", args)))
}

/// The lines of a function of the head given between its braces, as the passes of `options` write `body`.
fn lines_in(head: Head, names: &Names, options: Options, body: Vec<tree::Stmt>) -> String {
    let cx = Context::new(names, Project::empty(), options);
    let text = written_in(&function_of(head, body), &cx);
    let open = text.find("{\n").map_or(0, |at| at + 2);
    let close = text.rfind('}').unwrap_or(text.len());
    text[open..close].to_owned()
}

/// The lines of `body` at `Source`, with the passes of `off` switched off.
fn lines(off: &[Pass], body: Vec<tree::Stmt>) -> String {
    let options = off
        .iter()
        .fold(Options::default(), |o, pass| o.without(*pass));
    lines_in(head(), &Names::default(), options, body)
}

/// `e` as the source level writes it with the passes of `off` switched off.
fn text(off: &[Pass], head: &Head, e: Expr) -> String {
    let body = vec![used(vec![e])];
    let options = off
        .iter()
        .fold(Options::default(), |o, pass| o.without(*pass));
    let written = lines_in(head.clone(), &Names::default(), options, body);
    let line = written.lines().last().unwrap_or_default().trim();
    let inner = line
        .strip_prefix("DEMO.USE(")
        .and_then(|l| l.strip_suffix(");"));
    inner.unwrap_or(line).to_owned()
}

/// An object made, constructed with `p0` and two of its fields stored, then used.
fn constructed() -> Vec<tree::Stmt> {
    let parts = vec![
        assign(Var::Local(0), Expr::NewObject(object())),
        Stmt::Eval(local_call(0x100, vec![local(0), param(0)])),
        store(StoreKind::Ref, local(0), 0x18, param(0)),
        store(StoreKind::I32, local(0), 0x10, int(-1)),
    ];
    vec![group(Group::Construct, parts), used(vec![local(0)])]
}

#[test]
fn an_object_made_constructed_and_filled_is_one_new() {
    let whole = "    Cell local_0 = new type_00000000000000AB(p0) { f_18 = p0, f_10 = -1 };
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[], constructed()), whole);

    // Without `initialisers` the fields are the stores they are, and without `constructors`
    // the group is every statement it holds: the way back.
    let stored = "    Cell local_0 = new type_00000000000000AB(p0);
    local_0->f_18 = p0;
    local_0->f_10 = -1;
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[Pass::Initialisers], constructed()), stored);
    let apart = "    Cell local_0 = new type_00000000000000AB();
    func_000100(local_0, p0);
    local_0->f_18 = p0;
    local_0->f_10 = -1;
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[Pass::Constructors], constructed()), apart);

    // An object no constructor runs on is still one `new` with its fields.
    let parts = vec![
        assign(Var::Local(0), Expr::NewObject(object())),
        store(StoreKind::Ref, local(0), 0x18, param(0)),
    ];
    let body = vec![group(Group::Construct, parts), used(vec![local(0)])];
    let filled = "    Cell local_0 = new type_00000000000000AB { f_18 = p0 };
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[], body), filled);
}

#[test]
fn a_constructor_run_on_an_object_made_in_the_call_is_that_object_made() {
    let call = |at: u32| Stmt::Eval(local_call(at, vec![Expr::NewObject(object()), int(7)]));
    let body = || vec![simple(call(0x300)), simple(call(0x400))];
    let mut project = Project::default();
    let constructor = Member {
        owner: TypeKey(0xAB),
        role: Role::Constructor,
        is_static: false,
        slot: None,
    };
    let key = FunctionKey::new(head().program, 0x300);
    project.members.insert(key, constructor);
    let names = Names::default();
    let cx = Context::new(&names, &project, Options::default());
    let text = written_in(&function_of(head(), body()), &cx);
    assert!(
        text.contains("    new type_00000000000000AB(7);\n"),
        "{text}"
    );
    // ⚠ A function nothing proves the constructor keeps its call, and so does the constructor with its pass off.
    let called = "    func_000400(new type_00000000000000AB(), 7);\n";
    assert!(text.contains(called), "{text}");
    let off = Options::default().without(Pass::Constructors);
    let off = Context::new(&names, &project, off);
    let text = written_in(&function_of(head(), body()), &off);
    let named = "    func_000300(new type_00000000000000AB(), 7);\n";
    assert!(text.contains(named), "{text}");
}

#[test]
fn a_store_that_is_no_field_of_the_object_leaves_the_stores_statements() {
    // ⚠ The second store goes through another variable: an initialiser would say it is the object's.
    let parts = vec![
        assign(Var::Local(0), Expr::NewObject(object())),
        Stmt::Eval(local_call(0x100, vec![local(0), param(0)])),
        store(StoreKind::Ref, local(0), 0x18, param(0)),
        store(StoreKind::Ref, local(1), 0x18, param(0)),
    ];
    let body = vec![group(Group::Construct, parts), used(vec![local(0)])];
    let expected = "    Cell local_1;
    Cell local_0 = new type_00000000000000AB(p0);
    local_0->f_18 = p0;
    local_1->f_18 = p0;
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[], body), expected);

    // A second statement that is no call on the object is no constructor: nothing is joined.
    let parts = vec![
        assign(Var::Local(0), Expr::NewObject(object())),
        Stmt::Eval(local_call(0x100, vec![param(0), local(0)])),
    ];
    let body = vec![group(Group::Construct, parts), used(vec![local(0)])];
    let apart = "    Cell local_0 = new type_00000000000000AB();
    func_000100(p0, local_0);
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[], body), apart);
}

#[test]
fn a_construction_names_its_type_and_its_fields_by_what_is_supplied() {
    let mut names = Names::default();
    names
        .fields
        .insert((TypeKey(0xAB), 0x18), "owner".to_owned());
    let made = Kind::Object("type_00000000000000AB".to_owned());
    let head = kinds(&[(Var::Local(0), made)]);
    let written = lines_in(head, &names, Options::default(), constructed());
    let expected = "    type_00000000000000AB local_0 = new type_00000000000000AB(p0) { owner = p0, f_10 = -1 };
    DEMO.USE(local_0);
";
    assert_eq!(written, expected);
}

#[test]
fn an_array_made_and_filled_in_order_is_one_new() {
    let int32 = || VmType {
        name: "System.Int32".to_owned(),
        byte_size: 4,
        id: 0x8000_0003,
        flags: 0,
    };
    let made = |len: i32| {
        let array = Expr::NewArray {
            ty: int32(),
            len: Box::new(int(len)),
            wrapper: "NewArray_3".to_owned(),
        };
        assign(Var::Local(0), array)
    };
    let set = |index: i32, value: Expr| {
        Stmt::Eval(intrinsic(
            "array_set_u32",
            vec![local(0), int(index), value],
        ))
    };
    let body = |parts: Vec<Stmt>| vec![group(Group::Collection, parts), used(vec![local(0)])];

    let filled = vec![made(2), set(0, int(5)), set(1, param(0))];
    let whole = "    Cell local_0 = new int[] { 5, (uint)p0 };
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[], body(filled.clone())), whole);
    let apart = "    Cell local_0 = new int[2];
    local_0[0] = 5;
    local_0[1] = (uint)p0;
    DEMO.USE(local_0);
";
    assert_eq!(lines(&[Pass::Initialisers], body(filled)), apart);

    // ⚠ Every element in order and no other: one left out is zero, which fewer items would not say.
    let short = vec![made(3), set(0, int(5)), set(1, param(0))];
    let written = lines(&[], body(short));
    assert!(
        written.starts_with("    Cell local_0 = new int[3];\n"),
        "{written}"
    );
    let swapped = vec![made(2), set(1, int(5)), set(0, param(0))];
    let written = lines(&[], body(swapped));
    assert!(
        written.starts_with("    Cell local_0 = new int[2];\n"),
        "{written}"
    );
    assert!(written.contains("    local_0[1] = 5;\n"), "{written}");
}

#[test]
fn a_delegate_made_and_bound_is_the_function_on_what_it_is_bound_to() {
    let delegate = || VmType {
        name: "System.Delegate".to_owned(),
        byte_size: 32,
        id: 0x8000_0002,
        flags: 0,
    };
    let own = |at: u32| Expr::FunctionRef {
        program: 0x1122_3344,
        at,
    };
    let body = |bound: Expr, function: Expr| {
        let bind = builtin("System.Delegate.Ctor", vec![local(0), bound, function]);
        let parts = vec![
            assign(Var::Local(0), Expr::NewObject(delegate())),
            Stmt::Eval(bind),
        ];
        vec![group(Group::Delegate, parts), used(vec![local(0)])]
    };
    let first = |written: String| written.lines().next().unwrap_or_default().to_owned();

    let bound = lines(&[], body(param(0), own(0xED00)));
    assert_eq!(first(bound), "    Cell local_0 = p0.func_00ED00;");
    let null = constant_of(0, Kind::Pointer);
    let free = lines(&[], body(null, own(0xED00)));
    assert_eq!(first(free), "    Cell local_0 = func_00ED00;");
    // A function read as a lambda's body is still named: no body is written in an expression yet.
    let lambda = Expr::sugar(ExprSugar::Lambda, own(0xED00));
    let closed = lines(&[], body(param(0), lambda));
    assert_eq!(first(closed), "    Cell local_0 = p0.func_00ED00;");

    // ⚠ Another program's function is no member of what it is bound to: the delegate stays a `new`.
    let other = Expr::FunctionRef {
        program: 0xCD,
        at: 0xED00,
    };
    let foreign = lines(&[], body(param(0), other));
    let expected = "    Cell local_0 = new System.Delegate(p0, &program_000000CD.func_00ED00);";
    assert_eq!(first(foreign), expected);

    let apart = "    Cell local_0 = new System.Delegate();
    System.Delegate.Ctor(local_0, p0, &func_00ED00);
    DEMO.USE(local_0);
";
    let off = lines(&[Pass::Delegates], body(param(0), own(0xED00)));
    assert_eq!(off, apart);
}

#[test]
fn a_statement_that_works_on_the_place_it_writes_is_a_compound_assignment() {
    let field = |offset| load(Scalar::U64, param(0), offset);
    let or = |read: Expr| Expr::binary(BinOp::Or, read, int(0x400));
    let compound = |part: Stmt| vec![group(Group::Compound, vec![part])];

    let flagged = store(StoreKind::U64, param(0), 0x30, or(field(0x30)));
    assert_eq!(
        lines(&[], compound(flagged.clone())),
        "    p0->f_30 |= 1024;\n"
    );
    assert_eq!(
        lines(&[Pass::Compound], compound(flagged)),
        "    p0->f_30 = p0->f_30 | 1024;\n"
    );

    let shifted = Expr::Binary {
        op: BinOp::Shl,
        at: Kind::I32,
        l: Box::new(load(Scalar::I32, param(0), 0x10)),
        r: Box::new(int(2)),
    };
    // ⚠ Four bytes through a pointer of no proved type: `<<=` would read as the whole cell.
    let narrow = store(StoreKind::I32, param(0), 0x10, shifted);
    assert_eq!(
        lines(&[], compound(narrow)),
        "    p0->f_10 = (int)p0->f_10 << 2;\n"
    );

    let stepped = |at: Expr| Expr::Binary {
        op: BinOp::Add,
        at: Kind::I64,
        l: Box::new(at),
        r: Box::new(int(1)),
    };
    let step = assign(Var::Local(0), stepped(Expr::cast(Scalar::I64, local(0))));
    assert_eq!(
        lines(&[], compound(step)),
        "    Cell local_0;\n    local_0++;\n"
    );
    // ⚠ `p0->f_30++` is the step of an `int` there: a whole cell stepped says `+= 1`.
    let counted = store(StoreKind::U64, param(0), 0x30, stepped(field(0x30)));
    assert_eq!(lines(&[], compound(counted)), "    p0->f_30 += 1;\n");

    // ⚠ Another place on the left of the operator is no compound assignment.
    let other = store(StoreKind::U64, param(0), 0x30, or(field(0x38)));
    assert_eq!(
        lines(&[], compound(other)),
        "    p0->f_30 = p0->f_38 | 1024;\n"
    );
    // ⚠ Nor is an address that calls: the one statement would call once where the two call twice.
    let got = || native("GET", vec![]);
    let read = load(Scalar::U64, got(), 0x30);
    let called = store(StoreKind::U64, got(), 0x30, or(read));
    assert_eq!(
        lines(&[], compound(called)),
        "    DEMO.GET()->f_30 = DEMO.GET()->f_30 | 1024;\n"
    );
}

#[test]
fn a_field_its_type_declares_is_written_with_a_dot_and_its_name() {
    let owner = TypeKey(0xAB);
    let mut head = head();
    head.params = 2;
    let body = || {
        let stored = store(StoreKind::I32, param(0), 0x8, param(1));
        let read = Expr::sugar(ExprSugar::Field { owner }, load(Scalar::I32, param(0), 0x8));
        let wide = Expr::LoadBytes {
            addr: Box::new(param(0)),
            offset: 0x10,
            bytes: 16,
        };
        let pair = Expr::sugar(ExprSugar::Field { owner }, wide);
        vec![
            group(Group::Field { owner }, vec![stored]),
            used(vec![read, pair]),
        ]
    };
    let names = Names::default();
    // The field's bytes read as a number are still said: nothing declares it an `int`.
    let read = "    p0.f_8 = (int)p1;
    DEMO.USE((int)p0.f_8, p0.f_10);
";
    assert_eq!(
        lines_in(head.clone(), &names, Options::default(), body()),
        read
    );
    let off = Options::default().without(Pass::Fields);
    let through = "    p0->f_8 = (int)p1;
    DEMO.USE((int)p0->f_8, (Cells2)p0->f_10);
";
    assert_eq!(lines_in(head.clone(), &names, off, body()), through);

    // A field declared an `int` is read bare, and one the user named goes by that name.
    let mut named = Names::default();
    named.fields.insert((owner, 0x8), "count".to_owned());
    let mut project = Project::default();
    let declared = FieldFacts {
        offset: 0x8,
        bytes: 4,
        kind: Kind::I32,
    };
    project.fields.insert(owner, vec![declared]);
    let cx = Context::new(&named, &project, Options::default());
    let text = written_in(&function_of(head, body()), &cx);
    assert!(text.contains("    p0.count = (int)p1;\n"), "{text}");
    assert!(
        text.contains("    DEMO.USE(p0.count, p0.f_10);\n"),
        "{text}"
    );
}

#[test]
fn an_out_declares_the_variable_the_statement_before_only_made_zero() {
    let v = Var::Value { at: 0x10, index: 0 };
    let mut head = head();
    head.vars.insert(v, VarInfo::new(Width::Cell));
    let filled = |zero: Expr| {
        let out = Expr::sugar(ExprSugar::Out, address(v, 0));
        let parts = vec![assign(v, zero), Stmt::Eval(native("FILL", vec![out]))];
        group(Group::Out, parts)
    };
    let names = Names::default();
    let write =
        |options: Options, body: Vec<tree::Stmt>| lines_in(head.clone(), &names, options, body);
    let body = || vec![filled(int(0)), used(vec![Expr::Var(v)])];
    let whole = "    DEMO.FILL(out Cell v_000010_0);
    DEMO.USE(v_000010_0);
";
    assert_eq!(write(Options::default(), body()), whole);
    let apart = "    Cell v_000010_0 = 0;
    DEMO.FILL(&v_000010_0);
    DEMO.USE(v_000010_0);
";
    assert_eq!(
        write(Options::default().without(Pass::RefOut), body()),
        apart
    );

    // A variable named before the call is declared at the top, and the `out` only names it.
    let earlier = vec![used(vec![Expr::Var(v)]), filled(int(0))];
    let written = write(Options::default(), earlier);
    assert!(
        written.contains("    DEMO.FILL(out v_000010_0);\n"),
        "{written}"
    );
    // ⚠ A variable given a value first is no `out` of nothing: both statements stay.
    let valued = vec![filled(int(5)), used(vec![Expr::Var(v)])];
    let written = write(Options::default(), valued);
    assert!(
        written.starts_with("    Cell v_000010_0 = 5;\n"),
        "{written}"
    );
}

#[test]
fn a_value_of_several_cells_taken_apart_is_a_deconstruction() {
    let v = Var::Value { at: 0x20, index: 0 };
    let mut head = head();
    head.vars.insert(v, VarInfo::new(Width::Cells));
    let pair = Expr::Call {
        callee: Callee::Native {
            namespace: Some("DEMO".to_owned()),
            name: "PAIR".to_owned(),
            hash: 0xA2,
            rva: 0,
        },
        args: vec![],
        returns: Some(2),
    };
    let cell = |index| Expr::Cell {
        base: Box::new(Expr::Var(v)),
        index,
    };
    let body = |first: u32, second: u32| {
        let parts = vec![
            assign(v, pair.clone()),
            assign(Var::Local(0), cell(first)),
            assign(Var::Local(1), cell(second)),
        ];
        vec![
            group(Group::Deconstruct, parts),
            used(vec![local(0), local(1)]),
        ]
    };
    let names = Names::default();
    let whole = "    (Cell local_0, Cell local_1) = DEMO.PAIR();
    DEMO.USE(local_0, local_1);
";
    assert_eq!(
        lines_in(head.clone(), &names, Options::default(), body(0, 1)),
        whole
    );
    let apart = "    Cells v_000020_0 = DEMO.PAIR();
    Cell local_0 = v_000020_0[0];
    Cell local_1 = v_000020_0[1];
    DEMO.USE(local_0, local_1);
";
    let off = Options::default().without(Pass::Tuples);
    assert_eq!(lines_in(head.clone(), &names, off, body(0, 1)), apart);
    // ⚠ Cells taken out of order are no deconstruction of the value.
    let swapped = lines_in(head, &names, Options::default(), body(1, 0));
    assert!(
        swapped.contains("    Cell local_0 = v_000020_0[1];\n"),
        "{swapped}"
    );
}

#[test]
fn a_chain_of_tests_of_one_value_is_a_switch() {
    let test = |subject: u32, n: i32| {
        Expr::binary(BinOp::Eq, Expr::cast(Scalar::I32, param(subject)), int(n))
    };
    let either = |l: Expr, r: Expr| Expr::Binary {
        op: BinOp::LogicalOr,
        at: Kind::Bool,
        l: Box::new(l),
        r: Box::new(r),
    };
    let chain = |second: Expr| {
        let arms = vec![
            Arm {
                test: test(0, 1),
                body: vec![tree::Stmt::Return(None)],
            },
            Arm {
                test: second,
                body: vec![used(vec![])],
            },
            Arm {
                test: test(0, 4),
                body: Vec::new(),
            },
        ];
        let default = vec![used(vec![])];
        vec![sugar(Sugar::SwitchOn { arms, default })]
    };
    let mut head = head();
    head.params = 2;
    let names = Names::default();
    let write =
        |options: Options, body: Vec<tree::Stmt>| lines_in(head.clone(), &names, options, body);

    let switched = "    switch ((int)p0)
    {
        case 1:
            return;
        case 2:
        case 3:
            DEMO.USE();
            break;
        case 4:
            break;
        default:
            DEMO.USE();
            break;
    }
";
    let one = || chain(either(test(0, 2), test(0, 3)));
    assert_eq!(write(Options::default(), one()), switched);
    let tested = "    if ((int)p0 == 1)
    {
        return;
    }
    else if ((int)p0 == 2 || (int)p0 == 3)
    {
        DEMO.USE();
    }
    else if ((int)p0 != 4)
    {
        DEMO.USE();
    }
";
    let off = Options::default().without(Pass::SwitchOn);
    assert_eq!(write(off, one()), tested);

    // ⚠ An arm that tests another value is no case of the switch: the tests stay tests.
    let other = write(Options::default(), chain(test(1, 2)));
    assert!(!other.contains("switch"), "{other}");
    assert!(other.contains("    else if ((int)p1 == 2)\n"), "{other}");

    // Hashes are cases in 32 unsigned bits, as in a switch the program itself holds.
    let arm = |value: Expr| Arm {
        test: Expr::binary(BinOp::Eq, Expr::cast(Scalar::I32, param(0)), value),
        body: vec![tree::Stmt::Return(None)],
    };
    let arms = vec![arm(high()), arm(int(7))];
    let hashed = vec![sugar(Sugar::SwitchOn {
        arms,
        default: Vec::new(),
    })];
    let written = write(Options::default(), hashed);
    assert!(written.starts_with("    switch ((uint)p0)\n"), "{written}");
    assert!(written.contains("        case 0x9A3B5C7D:\n"), "{written}");
    assert!(written.contains("        case 0x00000007:\n"), "{written}");
}

#[test]
fn a_switch_is_refused_over_a_value_that_calls() {
    // ⚠ A switch reads its value once, and the tests it stands for read it once each.
    let got = || Expr::cast(Scalar::I32, native("GET", vec![]));
    let arm = |n: i32| Arm {
        test: Expr::binary(BinOp::Eq, got(), int(n)),
        body: vec![tree::Stmt::Return(None)],
    };
    let arms = vec![arm(1), arm(2), arm(3)];
    let body = vec![sugar(Sugar::SwitchOn {
        arms,
        default: Vec::new(),
    })];
    let written = lines(&[], body);
    assert!(!written.contains("switch"), "{written}");
    assert!(
        written.starts_with("    if ((int)DEMO.GET() == 1)\n"),
        "{written}"
    );
}

#[test]
fn an_await_and_a_yield_are_written_as_what_they_take_and_give() {
    let awaited = || Sugar::Await {
        before: vec![assign(Var::Local(1), local_call(0x100, vec![param(0)]))],
        ready: local_call(0x104, vec![local(1)]),
        suspend: vec![tree::Stmt::Return(None)],
        resumed: Label::At(0x200),
        resume: Vec::new(),
        after: vec![assign(Var::Local(2), local_call(0x108, vec![local(1)]))],
    };
    let body = || vec![sugar(awaited()), used(vec![local(2)])];
    let whole = "    Cell local_1, local_2;
    local_2 = await p0;
    DEMO.USE(local_2);
";
    assert_eq!(lines(&[], body()), whole);
    let lowered = lines(&[Pass::Await], body());
    assert!(!lowered.contains("await"), "{lowered}");
    // The arm taken while ready writes nothing, so the test is written the other way round.
    let taken = "    local_1 = func_000100(p0);\n    if (!func_000104(local_1))\n";
    assert!(lowered.contains(taken), "{lowered}");
    assert!(lowered.contains("        label_000200: ;\n"), "{lowered}");
    assert!(
        lowered.contains("    local_2 = func_000108(local_1);\n"),
        "{lowered}"
    );

    let yielded = || Sugar::Yield {
        value: vec![assign(Var::Local(0), int(5))],
        suspend: vec![tree::Stmt::Return(None)],
        resumed: Label::At(0x300),
        resume: Vec::new(),
    };
    let written = lines(&[], vec![sugar(yielded())]);
    assert!(written.ends_with("    yield return 5;\n"), "{written}");
    let lowered = lines(&[Pass::Iterators], vec![sugar(yielded())]);
    assert!(
        lowered.contains("    local_0 = 5;\n    return;\n"),
        "{lowered}"
    );
}

#[test]
fn what_the_compiler_added_shows_only_below_the_source_level() {
    let body = || vec![sugar(Sugar::Hidden(vec![used(vec![])]))];
    assert_eq!(lines(&[], body()), "");
    let structured = Options::at(Level::Structured);
    let shown = lines_in(head(), &Names::default(), structured, body());
    assert_eq!(shown, "    DEMO.USE();\n");
}

/// An enumerator taken from `p0` and gone through, each item handed to a function.
fn enumerated() -> Sugar {
    Sugar::Foreach {
        setup: vec![assign(Var::Local(1), local_call(0x100, vec![param(0)]))],
        cond: local_call(0x104, vec![local(1)]),
        item: Box::new(assign(Var::Local(2), local_call(0x108, vec![local(1)]))),
        body: vec![simple(Stmt::Eval(local_call(0x200, vec![local(2)])))],
        step: None,
        after: Vec::new(),
    }
}

#[test]
fn a_foreach_declares_its_item_and_its_loop_is_written_from_the_same_parts() {
    let whole = "    Cell local_1;
    foreach (Cell local_2 in p0)
    {
        func_000200(local_2);
    }
";
    assert_eq!(lines(&[], vec![sugar(enumerated())]), whole);
    let looped = "    Cell local_1;
    local_1 = func_000100(p0);
    while (func_000104(local_1))
    {
        Cell local_2 = func_000108(local_1);
        func_000200(local_2);
    }
";
    assert_eq!(lines(&[Pass::Foreach], vec![sugar(enumerated())]), looped);

    // ⚠ An item read after the loop is no variable the `foreach` declares: the loop stays.
    let body = vec![sugar(enumerated()), used(vec![local(2)])];
    let written = lines(&[], body);
    assert!(!written.contains("foreach"), "{written}");
    assert!(
        written.starts_with("    Cell local_1, local_2;\n"),
        "{written}"
    );
    assert!(
        written.contains("        local_2 = func_000108(local_1);\n"),
        "{written}"
    );
}

#[test]
fn an_index_stepped_over_an_array_is_a_foreach_over_the_array() {
    let step = Expr::binary(BinOp::Add, Expr::cast(Scalar::I32, local(0)), int(1));
    let length = intrinsic("array_length", vec![param(0)]);
    let below = Expr::binary(
        BinOp::Lt,
        Expr::cast(Scalar::I32, local(0)),
        Expr::cast(Scalar::I32, length),
    );
    let over = |start: i32, at: Expr| Sugar::Foreach {
        setup: vec![assign(Var::Local(0), int(start))],
        cond: below.clone(),
        item: Box::new(assign(
            Var::Local(1),
            intrinsic("array_get_u64", vec![param(0), at]),
        )),
        body: vec![used(vec![local(1)])],
        step: Some(Box::new(assign(Var::Local(0), step.clone()))),
        after: Vec::new(),
    };
    let whole = "    Cell local_0;
    foreach (Cell local_1 in p0)
    {
        DEMO.USE(local_1);
    }
";
    assert_eq!(lines(&[], vec![sugar(over(0, local(0)))]), whole);
    let looped = lines(&[Pass::Foreach], vec![sugar(over(0, local(0)))]);
    let opens = "    Cell local_0;\n    local_0 = 0;\n    for (; ";
    assert!(looped.starts_with(opens), "{looped}");
    let stepped = "; local_0 = (int)local_0 + 1)\n    {\n";
    assert!(looped.contains(stepped), "{looped}");
    let item = "        Cell local_1 = p0[local_0];\n";
    assert!(looped.contains(item), "{looped}");

    // ⚠ An index that starts past the first element, and an item read at another index, go
    // through less than the array.
    for refused in [over(1, local(0)), over(0, param(0))] {
        let written = lines(&[], vec![sugar(refused)]);
        assert!(!written.contains("foreach"), "{written}");
        assert!(written.contains("    for (; "), "{written}");
    }
}

#[test]
fn a_guard_a_try_and_a_throw_are_written_under_their_passes() {
    let locked = || Sugar::Guarded {
        kind: Guard::Lock,
        enter: vec![Stmt::Eval(local_call(0x400, vec![param(0)]))],
        body: vec![used(vec![])],
        exit: vec![simple(Stmt::Eval(local_call(0x404, vec![param(0)])))],
    };
    let whole = "    lock (p0)
    {
        DEMO.USE();
    }
";
    assert_eq!(lines(&[], vec![sugar(locked())]), whole);
    let apart = "    func_000400(p0);
    DEMO.USE();
    func_000404(p0);
";
    assert_eq!(lines(&[Pass::Lock], vec![sugar(locked())]), apart);

    let raised = || Sugar::Throw(Box::new(Stmt::Eval(local_call(0x300, vec![param(0)]))));
    assert_eq!(lines(&[], vec![sugar(raised())]), "    throw p0;\n");
    let lowered = "    func_000300(p0);
    throw new System.Diagnostics.UnreachableException();
";
    assert_eq!(lines(&[Pass::Try], vec![sugar(raised())]), lowered);
}

#[test]
fn an_expression_reads_as_its_construct_only_under_its_pass() {
    let class = "type_00000000000000AA";
    let update = Expr::Call {
        callee: Callee::Virtual {
            class: class.to_owned(),
            method: "Method_3".to_owned(),
            ty: class.to_owned(),
            slot: 3,
        },
        args: vec![param(0), local(1)],
        returns: Some(1),
    };
    let equal = builtin(
        "System.Guid.op_Equality",
        vec![local(0), Expr::ZeroSlots(2)],
    );
    let set = Expr::binary(BinOp::Ne, local(0), int(0));
    let select = |then: Expr, els: Expr| Expr::Select {
        cond: Box::new(set.clone()),
        then: Box::new(then),
        els: Box::new(els),
    };
    let null = || constant_of(0, Kind::Pointer);
    let own = Expr::FunctionRef {
        program: 0x1122_3344,
        at: 0xED00,
    };
    let joined = builtin("System.String.Concat", vec![local(0), local(1)]);
    let field = load(Scalar::U64, local(0), 0x10);
    let head = head();
    let cases = [
        (
            Expr::sugar(ExprSugar::Method, update.clone()),
            Pass::Receivers,
            "p0.Method_3(local_1)",
            "type_00000000000000AA.Method_3(p0, local_1)",
        ),
        (
            Expr::sugar(ExprSugar::Operator, equal.clone()),
            Pass::Operators,
            "local_0 == default",
            "System.Guid.op_Equality(local_0, default(Cells2))",
        ),
        (
            Expr::sugar(ExprSugar::Coalesce, select(local(0), local(1))),
            Pass::NullOps,
            "local_0 ?? local_1",
            "local_0 != 0 ? local_0 : local_1",
        ),
        (
            Expr::sugar(ExprSugar::NullConditional, select(field, null())),
            Pass::NullOps,
            "local_0?.f_10",
            "local_0 != 0 ? local_0->f_10 : null",
        ),
        (
            Expr::sugar(ExprSugar::Lambda, own),
            Pass::Lambdas,
            "func_00ED00",
            "&func_00ED00",
        ),
        (
            Expr::sugar(ExprSugar::Concat, joined),
            Pass::Concat,
            "local_0 + local_1",
            "string.Concat(local_0, local_1)",
        ),
        (
            Expr::sugar(ExprSugar::Tuple, Expr::Cells(vec![local(0), local(1)])),
            Pass::Tuples,
            "(local_0, local_1)",
            "(local_0, local_1)",
        ),
    ];
    for (e, pass, on, off) in cases {
        assert_eq!(text(&[], &head, e.clone()), on);
        assert_eq!(text(&[pass], &head, e), off);
    }

    // ⚠ The printer reads no call as a construct itself: without the reading the call is the call.
    let bare = "System.Guid.op_Equality(local_0, default(Cells2))";
    assert_eq!(text(&[], &head, equal.clone()), bare);
    let typed = kinds(&[(Var::Param(0), Kind::Object(class.to_owned()))]);
    let called = "type_00000000000000AA.Method_3(p0, local_1)";
    assert_eq!(text(&[], &typed, update), called);

    // The negation of a type's own `==` is its `!=`.
    let unequal = not(Expr::sugar(ExprSugar::Operator, equal));
    assert_eq!(text(&[], &head, unequal), "local_0 != default");
    // ⚠ A narrower reading of nothing would not be null: only a field read whole is `?.`.
    let narrow = load(Scalar::I32, local(0), 0x10);
    let read = Expr::sugar(ExprSugar::NullConditional, select(narrow, null()));
    let chosen = "local_0 != 0 ? (int)local_0->f_10 : null";
    assert_eq!(text(&[], &head, read), chosen);
    // A conversion is the cast it is in C#.
    let unboxed = intrinsic("unbox<System.Int32>", vec![local(0)]);
    let converted = Expr::sugar(ExprSugar::Convert, unboxed);
    assert_eq!(text(&[], &head, converted), "(int)local_0");
}

#[test]
fn a_field_stored_through_the_receiver_goes_by_the_users_name_too() {
    let owner = TypeKey(0xAB);
    let mut head = head();
    head.params = 2;
    let stored = store(StoreKind::U64, param(0), 0x18, param(1));
    let body = vec![group(Group::Field { owner }, vec![stored])];
    let held = Method {
        function: function_of(head, body),
        role: Role::Method,
        is_static: false,
        is_async: false,
        slot: None,
        overrides: false,
    };
    let write = |names: &Names| {
        let cx = Context::new(names, Project::empty(), Options::default());
        let mut out = String::new();
        method(&mut out, &held, Some((owner, "CThing")), 0, &cx);
        out
    };
    // The store is written `this.f_18 = p1`, with a dot: the name takes the place of the offset.
    let unnamed = write(&Names::default());
    assert!(unnamed.contains("\n    this.f_18 = p1;\n"), "{unnamed}");
    let mut names = Names::default();
    names.fields.insert((owner, 0x18), "zq_held".to_owned());
    let named = write(&names);
    assert!(named.contains("\n    this.zq_held = p1;\n"), "{named}");
}

#[test]
fn a_field_stored_through_a_field_read_as_one_is_written_with_a_dot_as_its_load_is() {
    let (outer, inner) = (TypeKey(0xAB), TypeKey(0xAC));
    // `p0.f_8`, a reference the fields pass read as a field of `outer`.
    let held = || {
        Expr::sugar(
            ExprSugar::Field { owner: outer },
            load(Scalar::U64, param(0), 8),
        )
    };
    let stored = store(StoreKind::U64, held(), 0x18, param(1));
    let body = vec![group(Group::Field { owner: inner }, vec![stored])];
    let mut head = head();
    head.params = 2;
    let mut names = Names::default();
    let written = lines_in(head.clone(), &names, Options::default(), body.clone());
    assert_eq!(written, "    p0.f_8.f_18 = p1;\n");
    names.fields.insert((inner, 0x18), "zq_held".to_owned());
    let named = lines_in(head.clone(), &names, Options::default(), body);
    assert_eq!(named, "    p0.f_8.zq_held = p1;\n");
    // ⚠ Through a pointer no pass read as a field the store keeps its arrow.
    let raw = store(
        StoreKind::U64,
        load(Scalar::U64, param(0), 8),
        0x18,
        param(1),
    );
    let body = vec![group(Group::Field { owner: inner }, vec![raw])];
    let written = lines_in(head, &Names::default(), Options::default(), body);
    assert_eq!(written, "    p0->f_8->f_18 = p1;\n");
}

/// A function that makes a closure object in `local_0`, keeps `p0` in it, and binds the function at 0x200 of its program to it in `local_1`.
fn binder() -> tree::Function {
    let delegate = VmType {
        name: "System.Delegate".to_owned(),
        byte_size: 32,
        id: 9,
        flags: 0,
    };
    let reference = Expr::FunctionRef {
        program: 0x1122_3344,
        at: 0x200,
    };
    let bound = Expr::sugar(ExprSugar::Lambda, reference);
    let parts = vec![
        assign(Var::Local(1), Expr::NewObject(delegate)),
        Stmt::Eval(builtin(
            "System.Delegate.Ctor",
            vec![local(1), local(0), bound],
        )),
    ];
    let body = vec![
        simple(assign(Var::Local(0), Expr::NewObject(object()))),
        simple(store(StoreKind::U64, local(0), 0x8, param(0))),
        group(Group::Delegate, parts),
        used(vec![local(1)]),
    ];
    function_of(head(), body)
}

/// The function at 0x200 that `binder` binds: its closure object first, then one parameter.
fn bound_body(body: Vec<tree::Stmt>) -> tree::Function {
    let mut head = head();
    head.at = 0x200;
    head.name = "func_000200".to_owned();
    head.params = 2;
    function_of(head, body)
}

/// `binder` as the source level writes it with `body` the one lambda written where it is bound.
fn bound_in(binder: &tree::Function, body: &tree::Function) -> String {
    let mut lambdas = Lambdas::new();
    lambdas.insert(0x200, body);
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let held = Method::free(binder.clone());
    let mut out = String::new();
    method_in(&mut out, &held, None, 0, &cx, &lambdas);
    out
}

#[test]
fn a_lambda_is_written_where_it_is_bound_with_the_object_it_reads_named_as_the_binder_holds_it() {
    let field = || load(Scalar::U64, param(0), 0x8);
    let one = bound_body(vec![used(vec![field(), param(1)])]);
    let expected = "\
void func_000040(Cell p0)
{
    Cell local_0 = new type_00000000000000AB();
    local_0->f_8 = p0;
    Cell local_1 = (Cell p1) => DEMO.USE(local_0->f_8, p1);
    DEMO.USE(local_1);
}
";
    assert_eq!(bound_in(&binder(), &one), expected);

    // A body of a few statements is a block under the statement that binds it.
    let kept = simple(store(StoreKind::U64, param(0), 0x8, param(1)));
    let few = bound_body(vec![used(vec![param(1)]), kept]);
    let block = "    Cell local_1 = (Cell p1) =>
    {
        DEMO.USE(p1);
        local_0->f_8 = p1;
    };
    DEMO.USE(local_1);
";
    let text = bound_in(&binder(), &few);
    assert!(text.ends_with(&format!("{block}}}\n")), "{text}");

    // ⚠ A value that names a variable of the body's own is a block too: the block declares it.
    let scratch = bound_body(vec![used(vec![param(1), address(Var::Local(3), 0)])]);
    let block = "    Cell local_1 = (Cell p1) =>
    {
        Cell local_3;
        DEMO.USE(p1, &local_3);
    };
    DEMO.USE(local_1);
";
    let text = bound_in(&binder(), &scratch);
    assert!(text.ends_with(&format!("{block}}}\n")), "{text}");

    // With no body at hand, and with `lambdas` off, the delegate names the function.
    let names = Names::default();
    let named = "\n    Cell local_1 = local_0.func_000200;\n";
    let cx = Context::new(&names, Project::empty(), Options::default());
    let alone = written_in(&binder(), &cx);
    assert!(alone.contains(named), "{alone}");
    let off = Options::default().without(Pass::Lambdas);
    let cx = Context::new(&names, Project::empty(), off);
    let mut lambdas = Lambdas::new();
    lambdas.insert(0x200, &one);
    assert!(bound_in_place(&binder(), &lambdas, &cx).is_empty());
}

#[test]
fn a_lambda_keeps_its_name_where_its_body_in_place_would_read_another_object() {
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let field = || load(Scalar::U64, param(0), 0x8);
    let sited = |binder: &tree::Function, body: &tree::Function| {
        let mut lambdas = Lambdas::new();
        lambdas.insert(0x200, body);
        bound_in_place(binder, &lambdas, &cx)
    };
    let reads = bound_body(vec![used(vec![field(), param(1)])]);
    assert_eq!(
        sited(&binder(), &reads).into_iter().collect::<Vec<_>>(),
        [0x200]
    );

    // ⚠ A variable of the body that goes by the name the binder's object goes by would hide it.
    let own = simple(assign(Var::Local(0), param(1)));
    let hides = bound_body(vec![own, used(vec![local(0), field()])]);
    assert!(sited(&binder(), &hides).is_empty());
    let named = "\n    Cell local_1 = local_0.func_000200;\n";
    assert!(bound_in(&binder(), &hides).contains(named));

    // ⚠ A variable assigned twice holds two objects, and the body was bound to one of them.
    let mut twice = binder();
    let again = simple(assign(Var::Local(0), Expr::NewObject(object())));
    twice.body.push(again);
    assert!(sited(&twice, &reads).is_empty());
    // ⚠ So may one whose address is handed on.
    let mut reached = binder();
    reached.body.push(used(vec![address(Var::Local(0), 0)]));
    assert!(sited(&reached, &reads).is_empty());

    // ⚠ And a variable of a body that never reads its object would still read as the binder's.
    let scratch = bound_body(vec![used(vec![param(1), address(Var::Local(0), 0)])]);
    assert!(sited(&binder(), &scratch).is_empty());

    // A body that never reads its object is written in place whatever it was bound to.
    let free = bound_body(vec![used(vec![param(1)])]);
    assert_eq!(
        sited(&twice, &free).into_iter().collect::<Vec<_>>(),
        [0x200]
    );
    // No body is written in place for a function the map does not hold.
    assert!(bound_in_place(&binder(), &Lambdas::new(), &cx).is_empty());
}

#[test]
fn a_constant_stored_whole_to_a_field_of_a_byte_or_a_truth_is_that_kinds_literal() {
    let owner = TypeKey(0xAB);
    let mut project = Project::default();
    let field = |offset: u32, bytes: u32, kind: Kind| FieldFacts {
        offset,
        bytes,
        kind,
    };
    let fields = vec![
        field(0x10, 1, Kind::U8),
        field(0x11, 1, Kind::Bool),
        field(0x12, 2, Kind::I16),
        field(0x14, 4, Kind::Unknown),
    ];
    project.fields.insert(owner, fields);
    let byte = |offset: u32, value: i32| {
        let stored = store(StoreKind::U8, param(0), offset, int(value));
        group(Group::Field { owner }, vec![stored])
    };
    let body = vec![
        byte(0x10, 255),
        byte(0x11, 1),
        byte(0x11, 0),
        group(
            Group::Field { owner },
            vec![store(StoreKind::U16, param(0), 0x12, int(-2))],
        ),
        // A field of no proved kind, and a store narrower than its field, keep the width said.
        byte(0x14, 0),
    ];
    let names = Names::default();
    let cx = Context::new(&names, &project, Options::default());
    let text = written_in(&function_of(head(), body), &cx);
    for line in [
        "p0.f_10 = 255;",
        "p0.f_11 = true;",
        "p0.f_11 = false;",
        "p0.f_12 = -2;",
        "p0.f_14 = (byte)0;",
    ] {
        assert!(text.contains(line), "no `{line}` in:\n{text}");
    }
}

#[test]
fn a_field_the_project_proves_a_bool_is_read_and_tested_as_one() {
    let owner = TypeKey(0xAB);
    let mut project = Project::default();
    let truth = FieldFacts {
        offset: 0x10,
        bytes: 1,
        kind: Kind::Bool,
    };
    project.fields.insert(owner, vec![truth]);
    let read = || Expr::sugar(ExprSugar::Field { owner }, load(Scalar::U8, param(0), 0x10));
    let test = Expr::binary(BinOp::Ne, read(), int(0));
    let body = vec![tree::Stmt::If {
        cond: test,
        then: vec![used(vec![read()])],
        els: Vec::new(),
    }];
    let names = Names::default();
    let cx = Context::new(&names, &project, Options::default());
    let text = written_in(&function_of(head(), body.clone()), &cx);
    assert!(text.contains("if (p0.f_10)"), "{text}");
    assert!(text.contains("USE(p0.f_10)"), "{text}");
    // A byte of no proved kind is read as the byte it is.
    let text = written_in(&function_of(head(), body), &at(&names, Level::Source));
    assert!(text.contains("(byte)p0.f_10 != 0"), "{text}");
}
