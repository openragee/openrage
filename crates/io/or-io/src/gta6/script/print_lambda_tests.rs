use super::*;
use crate::gta6::script::ir::{Callee, Const, FunctionKey, Head, Returns, VmType};
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
