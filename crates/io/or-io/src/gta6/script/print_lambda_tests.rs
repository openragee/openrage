use super::*;
use crate::gta6::script::ir::{Callee, Const, FunctionKey, Head, Returns, VmType};
use crate::gta6::script::project::{ImportInfo, ProgramInfo};
use crate::gta6::script::tree::{Arm, Catch};
use crate::gta6::script::{Context, Names, Options, Project};

const PROGRAM: u32 = 0x1122_3344;
/// Another program, which no list names.
const ELSEWHERE: u32 = 0x5566_7788;

fn head(at: u32, params: u32) -> Head {
    Head {
        value_types: Default::default(),
        program: PROGRAM,
        at,
        name: format!("func_{at:06X}"),
        returns: Returns::Void,
        params,
        locals: 4,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn zero() -> Expr {
    Expr::Const(Const::i32(0))
}

/// A class of the program's own, `type_00000000000000AB`.
fn object() -> VmType {
    VmType {
        name: "type_00000000000000AB".to_owned(),
        byte_size: 32,
        id: 7,
        flags: 0,
    }
}

fn delegate_type() -> VmType {
    VmType {
        name: "System.Delegate".to_owned(),
        byte_size: 32,
        id: 9,
        flags: 0,
    }
}

fn local_call(at: u32, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Local {
            at,
            name: format!("func_{at:06X}"),
        },
        args,
        returns: Some(1),
    }
}

/// Slot 21 of `type_00000000000000AB`, taken from `receiver`.
fn slot(receiver: Expr) -> Expr {
    Expr::VirtualFunctionRef {
        receiver: Box::new(receiver),
        ty: object(),
        slot: 21,
    }
}

/// What a method whose receiver is `p0` writes as the method group of a delegate of `function` bound to `bound`.
fn group_of(names: &Names, options: Options, bound: &Expr, function: &Expr) -> Option<String> {
    let cx = Context::new(names, Project::empty(), options);
    let head = head(0x40, 2);
    let mut out = String::new();
    let mut writer = Writer::in_context(&mut out, &cx, &head);
    writer.this = Some(Var::Param(0));
    writer.method_group(bound, function)
}

#[test]
fn a_delegate_of_a_slot_of_the_object_it_is_bound_to_is_that_objects_method_group() {
    let names = Names::default();
    let on = Options::default();
    let this = group_of(&names, on, &param(0), &slot(param(0)));
    assert_eq!(this.as_deref(), Some("this.Method_21"));
    let held = group_of(&names, on, &local(1), &slot(local(1)));
    assert_eq!(held.as_deref(), Some("local_1.Method_21"));
    // The name a list gives the slot.
    let mut named = Names::default();
    named
        .methods
        .insert((TypeKey(0xAB), 21), "zq_Refresh".to_owned());
    let listed = group_of(&named, on, &param(0), &slot(param(0)));
    assert_eq!(listed.as_deref(), Some("this.zq_Refresh"));

    // ⚠ The slot of another object, a receiver read through a call, which the code makes twice,
    // a receiver that is nothing, and the names off: no group.
    let call = local_call(0x80, Vec::new());
    let cases: Vec<(&str, Options, Expr, Expr)> = vec![
        ("another object", on, local(1), slot(local(2))),
        ("a call", on, call.clone(), slot(call)),
        ("nothing", on, zero(), slot(zero())),
        (
            "delegates off",
            on.without(Pass::Delegates),
            param(0),
            slot(param(0)),
        ),
        (
            "the four first passes",
            Options::baseline(),
            param(0),
            slot(param(0)),
        ),
    ];
    for (what, options, bound, function) in cases {
        assert_eq!(group_of(&names, options, &bound, &function), None, "{what}");
    }
}

#[test]
fn a_delegate_of_a_function_of_another_program_is_its_method_group() {
    let names = Names::default();
    let on = Options::default();
    let elsewhere = Expr::FunctionRef {
        program: ELSEWHERE,
        at: 0x200,
    };
    let alone = group_of(&names, on, &zero(), &elsewhere);
    assert_eq!(alone.as_deref(), Some("program_55667788.func_000200"));
    // Read as a lambda's body it is the same function.
    let read = Expr::sugar(ExprSugar::Lambda, elsewhere.clone());
    let unbound = group_of(&names, on, &zero(), &read);
    assert_eq!(unbound.as_deref(), Some("program_55667788.func_000200"));
    // ⚠ Bound to an object it is no member of the object: `local_1.func_000200` would name this
    // program's function of that offset.
    assert_eq!(group_of(&names, on, &local(1), &elsewhere), None);
    assert_eq!(group_of(&names, on, &param(0), &read), None);

    // The names a list gives the program and the function.
    let mut named = Names::default();
    named
        .programs
        .insert(u64::from(ELSEWHERE), "zq_town".to_owned());
    named
        .functions
        .insert(FunctionKey::new(ELSEWHERE, 0x200), "zq_Wake".to_owned());
    let alone = group_of(&named, on, &zero(), &elsewhere);
    assert_eq!(alone.as_deref(), Some("zq_town.zq_Wake"));
    assert_eq!(group_of(&named, on, &local(1), &elsewhere), None);

    // ⚠ A function of this program is the delegate's own form, and a number is no object.
    let own = Expr::FunctionRef {
        program: PROGRAM,
        at: 0x200,
    };
    assert_eq!(group_of(&names, on, &local(1), &own), None);
    let number = Expr::Const(Const::i32(5));
    assert_eq!(group_of(&names, on, &number, &elsewhere), None);
}

#[test]
fn a_reference_whose_top_byte_holds_flags_names_the_function_at_its_offset() {
    let flagged = Expr::FunctionRef {
        program: ELSEWHERE,
        at: 0x4000_0200,
    };
    let names = Names::default();
    let alone = group_of(&names, Options::default(), &zero(), &flagged);
    assert_eq!(alone.as_deref(), Some("program_55667788.func_000200"));
    // The user's name for the function is found under its offset.
    let mut named = Names::default();
    named
        .functions
        .insert(FunctionKey::new(ELSEWHERE, 0x200), "zq_Wake".to_owned());
    let alone = group_of(&named, Options::default(), &zero(), &flagged);
    assert_eq!(alone.as_deref(), Some("program_55667788.zq_Wake"));
}

/// `local_1 = new System.Delegate(); System.Delegate.Ctor(local_1, local_0, <the function at 0x200>)`, read as one group.
fn bound_here() -> tree::Stmt {
    let lambda = Expr::sugar(
        ExprSugar::Lambda,
        Expr::FunctionRef {
            program: PROGRAM,
            at: 0x200,
        },
    );
    let ctor = Expr::Call {
        callee: Callee::Builtin {
            path: "System.Delegate.Ctor".to_owned(),
            ty: 9,
            method: 0,
        },
        args: vec![local(1), local(0), lambda],
        returns: Some(0),
    };
    let parts = vec![
        Stmt::Assign {
            target: Var::Local(1),
            value: Expr::NewObject(delegate_type()),
        },
        Stmt::Eval(ctor),
    ];
    construct(tree::Sugar::Group {
        kind: tree::Group::Delegate,
        parts,
    })
}

fn construct(sugar: tree::Sugar) -> tree::Stmt {
    tree::Stmt::Sugar(Box::new(sugar))
}

/// A function that makes its closure object in `local_0`, then runs `then`.
fn binder(then: Vec<tree::Stmt>) -> tree::Function {
    let made = Stmt::Assign {
        target: Var::Local(0),
        value: Expr::NewObject(object()),
    };
    let mut body = vec![tree::Stmt::Simple(made)];
    body.extend(then);
    tree::Function {
        head: head(0x40, 2),
        body,
    }
}

/// The function at 0x200: it hands a field of its closure object and its parameter on.
fn lambda_body() -> tree::Function {
    let field = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(param(0)),
        offset: 8,
    };
    let call = local_call(0x80, vec![field, param(1)]);
    tree::Function {
        head: head(0x200, 2),
        body: vec![tree::Stmt::Simple(Stmt::Eval(call))],
    }
}

/// The offsets of the lambdas `binder` writes in place, with the one at 0x200 at hand.
fn sited(binder: &tree::Function) -> Vec<u32> {
    let body = lambda_body();
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let mut lambdas = Lambdas::new();
    lambdas.insert(0x200, &body);
    bound_in_place(binder, &lambdas, &cx).into_iter().collect()
}

#[test]
fn a_lambda_bound_in_a_body_every_form_of_its_construct_writes_is_written_in_place() {
    assert_eq!(sited(&binder(vec![bound_here()])), [0x200]);
    let tried = construct(tree::Sugar::Try {
        body: vec![bound_here()],
        catches: Vec::new(),
        finally: Vec::new(),
    });
    assert_eq!(sited(&binder(vec![tried])), [0x200]);
    let finally = construct(tree::Sugar::Try {
        body: Vec::new(),
        catches: Vec::new(),
        finally: vec![bound_here()],
    });
    assert_eq!(sited(&binder(vec![finally])), [0x200]);
    let arm = construct(tree::Sugar::SwitchOn {
        arms: vec![Arm {
            test: param(1),
            body: vec![bound_here()],
        }],
        default: Vec::new(),
    });
    assert_eq!(sited(&binder(vec![arm])), [0x200]);
    let default = construct(tree::Sugar::SwitchOn {
        arms: Vec::new(),
        default: vec![bound_here()],
    });
    assert_eq!(sited(&binder(vec![default])), [0x200]);

    // ⚠ Not where a form leaves the statement unwritten: a handler, which the statements a
    // `try` stands for drop, and what is hidden.
    let caught = construct(tree::Sugar::Try {
        body: Vec::new(),
        catches: vec![Catch {
            ty: None,
            var: None,
            body: vec![bound_here()],
        }],
        finally: Vec::new(),
    });
    assert!(sited(&binder(vec![caught])).is_empty());
    let hidden = construct(tree::Sugar::Hidden(vec![bound_here()]));
    assert!(sited(&binder(vec![hidden])).is_empty());
}

/// A closure class the compiler made, `type_00000000000000CD`.
const CLOSURE: TypeKey = TypeKey(0xCD);

fn closure_type() -> VmType {
    VmType {
        name: "type_00000000000000CD".to_owned(),
        byte_size: 24,
        id: 8,
        flags: 0x0010_0000,
    }
}

fn simple(s: Stmt) -> tree::Stmt {
    tree::Stmt::Simple(s)
}

fn eval(e: Expr) -> tree::Stmt {
    simple(Stmt::Eval(e))
}

/// The whole cell at `offset` of what `object` points at.
fn field(object: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(object),
        offset,
    }
}

/// `object.f_<offset> = value;`, a whole cell.
fn given(object: Var, offset: u32, value: Expr) -> tree::Stmt {
    simple(Stmt::Store {
        kind: StoreKind::U64,
        addr: Expr::Var(object),
        offset,
        value,
    })
}

/// The function at 0x40 of three parameters: a closure object made in `local_0` and given `p2` at byte 8, `between`, the lambda at 0x200 bound to it in `local_1`, the delegate handed on, then `after`.
fn maker(between: Vec<tree::Stmt>, after: Vec<tree::Stmt>) -> tree::Function {
    let made = Stmt::Assign {
        target: Var::Local(0),
        value: Expr::NewObject(closure_type()),
    };
    let mut body = vec![simple(made), given(Var::Local(0), 8, param(2))];
    body.extend(between);
    body.push(bound_here());
    body.push(eval(local_call(0x90, vec![local(1)])));
    body.extend(after);
    tree::Function {
        head: head(0x40, 3),
        body,
    }
}

/// The closure class whose one member is the lambda `body`.
fn closure_class(body: tree::Function) -> Class {
    Class {
        key: CLOSURE,
        kind: ClassKind::Closure,
        base: None,
        interfaces: Vec::new(),
        fields: Vec::new(),
        methods: vec![Method {
            role: Role::Lambda,
            is_static: false,
            ..Method::free(body)
        }],
        nested: Vec::new(),
    }
}

/// The program of the free functions `functions` and the closure class of the lambda `body`.
fn unit_of(functions: Vec<tree::Function>, body: tree::Function) -> Unit {
    let mut unit = Unit::flat(PROGRAM, Vec::new(), functions);
    unit.classes.push(closure_class(body));
    unit
}

/// An install that holds the program, and the programs `importers`, each of which imports it.
fn installed(importers: &[u32]) -> Project {
    let mut project = Project::default();
    let own = ProgramInfo {
        program: PROGRAM,
        ..ProgramInfo::default()
    };
    project.programs.insert(PROGRAM, own);
    for at in importers {
        let import = ImportInfo {
            program: PROGRAM,
            first: 0,
            end: 1,
        };
        let info = ProgramInfo {
            program: *at,
            imports: vec![import],
            ..ProgramInfo::default()
        };
        project.programs.insert(*at, info);
    }
    project
}

/// Every lambda of the closure classes of `unit`, as if each were written where it is bound.
fn every_lambda(unit: &Unit) -> Lambdas<'_> {
    let mut lambdas = Lambdas::new();
    for class in &unit.classes {
        for method in class.methods.iter().filter(|m| m.role == Role::Lambda) {
            lambdas.insert(method.function.head.at, &method.function);
        }
    }
    lambdas
}

fn hidden_in(unit: &Unit, project: &Project, options: Options) -> Hidden {
    let names = Names::default();
    let cx = Context::new(&names, project, options);
    Hidden::of(unit, &every_lambda(unit), &cx)
}

/// The method of `unit` at `at`, free or of a class.
fn method_at(unit: &Unit, at: u32) -> &Method {
    let mut all = unit.functions.iter();
    let free = all.find(|m| m.function.head.at == at);
    let mut members = unit.classes.iter().flat_map(|class| class.methods.iter());
    free.or_else(|| members.find(|m| m.function.head.at == at))
        .expect("a method at that offset")
}

/// The method at `at` as the source level writes it: with the closure classes hidden where `hide`, as of old where not.
fn written(
    unit: &Unit,
    project: &Project,
    at: u32,
    class: Option<(TypeKey, &str)>,
    hide: bool,
) -> String {
    let names = Names::default();
    let cx = Context::new(&names, project, Options::default());
    let lambdas = every_lambda(unit);
    let hidden = match hide {
        true => Hidden::of(unit, &lambdas, &cx),
        false => Hidden::default(),
    };
    let mut out = String::new();
    method_hiding(
        &mut out,
        method_at(unit, at),
        class,
        0,
        &cx,
        &lambdas,
        &hidden,
    );
    out
}

#[test]
fn a_closure_class_whose_lambdas_are_all_written_in_place_is_written_nowhere() {
    let unit = unit_of(vec![maker(Vec::new(), Vec::new())], lambda_body());
    let project = installed(&[]);
    assert!(hidden_in(&unit, &project, Options::default()).hides(CLOSURE));
    let text = written(&unit, &project, 0x40, None, true);
    assert!(
        text.contains("Cell local_1 = (Cell p1) => func_000080(p2, p1);"),
        "{text}"
    );
    assert!(!text.contains("local_0"), "{text}");
    assert!(!text.contains("type_00000000000000CD"), "{text}");

    // The way back: with nothing hidden the object is made, given its field and read.
    let before = written(&unit, &project, 0x40, None, false);
    assert!(before.contains("local_0.f_8"), "{before}");
    assert!(before.contains("type_00000000000000CD"), "{before}");
    let names = Names::default();
    let cx = Context::new(&names, &project, Options::default());
    let mut old = String::new();
    method_in(
        &mut old,
        method_at(&unit, 0x40),
        None,
        0,
        &cx,
        &every_lambda(&unit),
    );
    assert_eq!(before, old);
}

#[test]
fn a_closure_class_stays_where_a_field_may_hold_other_than_its_variable() {
    let project = installed(&[]);
    let on = Options::default();
    let hides = |unit: &Unit| hidden_in(unit, &project, on).hides(CLOSURE);
    let lambda = lambda_body;
    assert!(hides(&unit_of(
        vec![maker(Vec::new(), Vec::new())],
        lambda()
    )));

    // ⚠ The variable given assigned again, or its address handed on: the field the lambda reads
    // keeps what it was given.
    let again = simple(Stmt::Assign {
        target: Var::Param(2),
        value: zero(),
    });
    assert!(!hides(&unit_of(
        vec![maker(Vec::new(), vec![again])],
        lambda()
    )));
    let below = Expr::AddrOf {
        var: Var::Param(1),
        cell: 0,
    };
    let handed = eval(local_call(0x91, vec![below]));
    assert!(!hides(&unit_of(
        vec![maker(Vec::new(), vec![handed])],
        lambda()
    )));
    // ⚠ The field given again after the lambda is bound.
    let stored = given(Var::Local(0), 8, param(1));
    assert!(!hides(&unit_of(
        vec![maker(Vec::new(), vec![stored])],
        lambda()
    )));
    // ⚠ The field read before it is given anything.
    let mut early = maker(Vec::new(), Vec::new());
    early
        .body
        .insert(1, eval(local_call(0x91, vec![field(local(0), 8)])));
    assert!(!hides(&unit_of(vec![early], lambda())));
    // ⚠ A field given what no variable holds.
    let mut computed = maker(Vec::new(), Vec::new());
    computed.body[1] = given(Var::Local(0), 8, local_call(0x92, Vec::new()));
    assert!(!hides(&unit_of(vec![computed], lambda())));

    // ⚠ A lambda that writes its object, or reads a field it was never given.
    let mut writes = lambda();
    writes.body.insert(0, given(Var::Param(0), 8, param(1)));
    assert!(!hides(&unit_of(
        vec![maker(Vec::new(), Vec::new())],
        writes
    )));
    let mut unknown = lambda();
    unknown.body = vec![eval(local_call(
        0x80,
        vec![field(param(0), 0x10), param(1)],
    ))];
    assert!(!hides(&unit_of(
        vec![maker(Vec::new(), Vec::new())],
        unknown
    )));
    // ⚠ A variable of the lambda's own that goes by the name of the one given.
    let mut named = maker(Vec::new(), Vec::new());
    named.body[1] = given(Var::Local(0), 8, param(1));
    assert!(!hides(&unit_of(vec![named], lambda())));

    // ⚠ Another function that names the class.
    let mut typed = tree::Function {
        head: head(0x60, 1),
        body: vec![simple(Stmt::Assign {
            target: Var::Local(2),
            value: param(0),
        })],
    };
    let info = VarInfo {
        kind: Kind::Object("type_00000000000000CD".to_owned()),
        ..VarInfo::new(Width::Cell)
    };
    typed.head.vars.insert(Var::Local(2), info);
    let unit = unit_of(vec![maker(Vec::new(), Vec::new()), typed], lambda());
    assert!(!hides(&unit));

    // ⚠ A program that imports this one may name it, and so may one the project does not hold;
    // and `lambdas` off writes the class.
    let unit = unit_of(vec![maker(Vec::new(), Vec::new())], lambda());
    assert!(!hidden_in(&unit, &installed(&[ELSEWHERE]), on).hides(CLOSURE));
    assert!(!hidden_in(&unit, Project::empty(), on).hides(CLOSURE));
    let off = on.without(Pass::Lambdas);
    assert!(!hidden_in(&unit, &project, off).hides(CLOSURE));
}

#[test]
fn the_receiver_given_to_a_closure_object_reads_as_this_in_the_binder_and_in_the_lambda() {
    let read = eval(local_call(0x91, vec![field(local(0), 8)]));
    let mut binder = maker(Vec::new(), vec![read]);
    binder.body[1] = given(Var::Local(0), 8, param(0));
    let owner = Class {
        key: TypeKey(0xEE),
        kind: ClassKind::Class,
        base: None,
        interfaces: Vec::new(),
        fields: Vec::new(),
        methods: vec![Method {
            role: Role::Method,
            is_static: false,
            ..Method::free(binder)
        }],
        nested: Vec::new(),
    };
    let mut unit = Unit::flat(PROGRAM, Vec::new(), Vec::new());
    unit.classes.push(owner);
    unit.classes.push(closure_class(lambda_body()));
    let project = installed(&[]);
    assert!(hidden_in(&unit, &project, Options::default()).hides(CLOSURE));
    let class = Some((TypeKey(0xEE), "zq_Owner"));
    let text = written(&unit, &project, 0x40, class, true);
    assert!(text.contains("=> func_000080(this, p1);"), "{text}");
    assert!(text.contains("func_000091(this)"), "{text}");
    assert!(!text.contains("local_0"), "{text}");
}

#[test]
fn a_local_given_to_a_closure_object_is_read_for_its_field_where_assigned_once_before() {
    let project = installed(&[]);
    let hides = |binder: &tree::Function| {
        let unit = unit_of(vec![binder.clone()], lambda_body());
        hidden_in(&unit, &project, Options::default()).hides(CLOSURE)
    };
    let set = simple(Stmt::Assign {
        target: Var::Local(3),
        value: local_call(0x70, Vec::new()),
    });
    let mut binder = maker(Vec::new(), Vec::new());
    binder.body[1] = given(Var::Local(0), 8, local(3));
    binder.body.insert(0, set.clone());
    assert!(hides(&binder));
    let unit = unit_of(vec![binder.clone()], lambda_body());
    let text = written(&unit, &project, 0x40, None, true);
    assert!(text.contains("=> func_000080(local_3, p1);"), "{text}");

    // ⚠ Assigned again, or only once the object holds it.
    let mut again = binder.clone();
    again.body.push(simple(Stmt::Assign {
        target: Var::Local(3),
        value: zero(),
    }));
    assert!(!hides(&again));
    let mut late = binder.clone();
    late.body.remove(0);
    late.body.insert(2, set);
    assert!(!hides(&late));
    // ⚠ The address of a local at or below it reaches it; one above it does not.
    let address = |n: u32| {
        let at = Expr::AddrOf {
            var: Var::Local(n),
            cell: 0,
        };
        eval(local_call(0x71, vec![at]))
    };
    let mut reached = binder.clone();
    reached.body.push(address(2));
    assert!(!hides(&reached));
    let mut above = binder.clone();
    above.body.push(address(5));
    assert!(hides(&above));
}
