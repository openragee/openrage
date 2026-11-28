use std::collections::BTreeMap;

use super::table::{reading, Form, Reading, TYPES};
use super::*;
use crate::gta6::script::ir::{Const, Head, Returns, VmType};
use crate::gta6::script::project::Project;
use crate::gta6::script::tree::Arm;
use crate::gta6::script::{Names, Options};

/// The index in the runtime's table of the builtin type these tests call.
const INDEX: u32 = 0x3E;

fn head() -> Head {
    Head {
        value_types: BTreeMap::new(),
        program: 7,
        at: 0x100,
        name: "func_000100".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 2,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

fn ran(body: &[Stmt], options: Options) -> Vec<Stmt> {
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), options);
    let mut f = tree::Function {
        head: head(),
        body: body.to_vec(),
    };
    run(&mut f, &cx, &mut Stats::default());
    f.body
}

fn builtin(path: &str, args: Vec<Expr>, returns: u32) -> Expr {
    Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: INDEX,
            method: 1,
        },
        args,
        returns: Some(returns),
    }
}

fn p(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn address(n: u32) -> Expr {
    Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    }
}

fn int(n: i32) -> Expr {
    Expr::Const(Const::i32(n))
}

fn eval(e: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Eval(e))
}

fn assign(target: Var, value: Expr) -> Stmt {
    Stmt::Simple(ir::Stmt::Assign { target, value })
}

fn as_method(e: Expr) -> Expr {
    Expr::sugar(ExprSugar::Method, e)
}

/// A builtin type as a program states it where an object of it is made.
fn made_type(index: u32) -> VmType {
    VmType {
        name: "System.Text.StringBuilder".to_owned(),
        byte_size: 16,
        id: BUILTIN | index,
        flags: 0,
    }
}

/// Whether some call of `path` in `body` is read as a method of its receiver.
fn on_receiver(body: &[Stmt], path: &str) -> bool {
    let mut found = false;
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Sugar {
                kind: ExprSugar::Method,
                e: inner,
            } = e
            {
                found |= matches!(
                    &**inner,
                    Expr::Call { callee: Callee::Builtin { path: p, .. }, .. } if p == path
                );
            }
        });
    });
    found
}

#[test]
fn the_table_reads_a_member_by_the_overloads_that_take_its_arguments() {
    let queue = "System.Collections.Generic.Queue_1";
    let enqueue = format!("{queue}.Enqueue");
    assert_eq!(reading(&enqueue, 2), Some(Reading::Method));
    assert_eq!(reading(&enqueue, 1), None);
    assert_eq!(reading(&enqueue, 3), None);
    assert_eq!(
        reading("System.Text.StringBuilder.Ctor", 1),
        Some(Reading::Constructor)
    );
    assert_eq!(reading("System.Text.StringBuilder.Ctor", 4), None);
    assert_eq!(
        reading("System.DateTime.get_UtcNow", 0),
        Some(Reading::StaticGetter)
    );
    assert_eq!(reading("System.DateTime.get_UtcNow", 1), None);
    // An overload on an object and one on the type that take as many arguments.
    assert_eq!(reading("System.Type.GetType", 1), None);
    // A method of the type stays the call it already reads as.
    assert_eq!(reading("System.Type.GetType", 2), None);
    assert_eq!(reading("System.String.Compare", 2), None);
    // A method a script's type may override reads only on an object of exactly its type.
    assert_eq!(
        reading("System.Exception.GetBaseException", 1),
        Some(Reading::Virtual)
    );
    assert_eq!(
        reading("System.Random.NextBytes", 2),
        Some(Reading::Virtual)
    );
    // A value type's own method, an object's accessor.
    assert_eq!(reading("System.DateTime.AddTicks", 2), None);
    assert_eq!(reading(&format!("{queue}.get_Count"), 1), None);
    // A type or a member the table does not hold, and a path of no type.
    assert_eq!(reading("type_8000003E.Method_4", 1), None);
    assert_eq!(reading("System.Text.StringBuilder.Method_7", 1), None);
    assert_eq!(reading("Clear", 1), None);
}

#[test]
fn the_table_holds_each_type_once_and_a_constructor_under_its_one_name() {
    let mut names: Vec<&str> = TYPES.iter().map(|ty| ty.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), TYPES.len());
    for ty in TYPES {
        for (name, form) in ty.members {
            let constructor = matches!(form, Form::New(_));
            assert_eq!(*name == "Ctor", constructor, "{}.{name}", ty.name);
        }
    }
}

#[test]
fn a_method_of_an_object_reads_on_its_receiver() {
    let clear = builtin("System.Text.StringBuilder.Clear", vec![p(1)], 1);
    let body = vec![eval(clear.clone())];
    let done = ran(&body, Options::default());
    assert_eq!(done, [eval(as_method(clear))]);
    // The reading comes off again, and none is made with the name off.
    assert_eq!(tree::desugar(done), body);
    assert_eq!(ran(&body, Options::default().without(Pass::Runtime)), body);

    // A method whose value is taken, and one inside the arguments of another.
    let text = builtin("System.Text.StringBuilder.ToString", vec![p(0)], 1);
    let appended = builtin(
        "System.Text.StringBuilder.Append",
        vec![p(1), text.clone()],
        1,
    );
    let body = vec![assign(Var::Local(0), appended)];
    let read = builtin(
        "System.Text.StringBuilder.Append",
        vec![p(1), as_method(text)],
        1,
    );
    let done = ran(&body, Options::default());
    assert_eq!(done, [assign(Var::Local(0), as_method(read))]);
    assert_eq!(tree::desugar(done), body);
}

#[test]
fn a_call_the_table_does_not_prove_a_method_of_its_receiver_stays_as_it_is() {
    let clear = "System.Text.StringBuilder.Clear";
    for left in [
        // Handed an address, a constant, or arguments no overload takes.
        eval(builtin(clear, vec![address(1)], 1)),
        eval(builtin(clear, vec![int(0)], 1)),
        eval(builtin(clear, vec![p(1), p(0)], 1)),
        // A method a script's type may override, and one of a value type.
        eval(builtin("System.Exception.GetBaseException", vec![p(1)], 1)),
        eval(builtin("System.DateTime.AddTicks", vec![p(1), int(1)], 1)),
        // An object's accessor, which `properties` reads, and a method of the type.
        assign(
            Var::Local(0),
            builtin(
                "System.Collections.Generic.Queue_1.get_Count",
                vec![p(1)],
                1,
            ),
        ),
        assign(
            Var::Local(0),
            builtin("System.String.Compare", vec![p(0), p(1)], 1),
        ),
        // A builtin no list names.
        eval(builtin("type_8000003E.Method_4", vec![p(1)], 1)),
    ] {
        let body = std::slice::from_ref(&left);
        assert_eq!(ran(body, Options::default()), body);
    }
}

#[test]
fn a_call_another_reading_holds_is_not_read_again_but_its_arguments_are() {
    let text = builtin("System.Text.StringBuilder.ToString", vec![p(0)], 1);
    let held = |inner: Expr| {
        let call = builtin("System.Text.StringBuilder.Append", vec![p(1), inner], 1);
        Expr::sugar(ExprSugar::Convert, call)
    };
    let body = vec![assign(Var::Local(0), held(text.clone()))];
    let done = ran(&body, Options::default());
    assert_eq!(done, [assign(Var::Local(0), held(as_method(text)))]);
    assert_eq!(tree::desugar(done), tree::desugar(body));
}

#[test]
fn a_property_of_a_type_reads_on_the_type_where_its_value_is_taken() {
    let now = builtin("System.DateTime.get_UtcNow", Vec::new(), 1);
    let body = vec![
        assign(Var::Local(0), now.clone()),
        Stmt::Return(Some(now.clone())),
    ];
    let as_property = Expr::sugar(ExprSugar::Property, now.clone());
    let done = ran(&body, Options::default());
    let read = vec![
        assign(Var::Local(0), as_property.clone()),
        Stmt::Return(Some(as_property)),
    ];
    assert_eq!(done, read);
    assert_eq!(tree::desugar(done), body);
    assert_eq!(ran(&body, Options::default().without(Pass::Runtime)), body);
    // A getter run for nothing, one handed an argument, and one that yields nothing.
    for left in [
        eval(now),
        assign(
            Var::Local(0),
            builtin("System.DateTime.get_UtcNow", vec![p(0)], 1),
        ),
        assign(
            Var::Local(0),
            builtin("System.DateTime.get_UtcNow", Vec::new(), 0),
        ),
    ] {
        let body = std::slice::from_ref(&left);
        assert_eq!(ran(body, Options::default()), body);
    }
}

#[test]
fn an_object_of_the_runtime_made_and_constructed_is_one_new() {
    let made = ir::Stmt::Assign {
        target: Var::Local(0),
        value: Expr::NewObject(made_type(INDEX)),
    };
    let constructed =
        |args: Vec<Expr>| ir::Stmt::Eval(builtin("System.Text.StringBuilder.Ctor", args, 0));
    let ctor = constructed(vec![local(0), int(16)]);
    let body = vec![Stmt::Simple(made.clone()), Stmt::Simple(ctor.clone())];
    let done = ran(&body, Options::default());
    let group = Sugar::Group {
        kind: Group::Construct,
        parts: vec![made.clone(), ctor.clone()],
    };
    assert_eq!(done, [Stmt::Sugar(Box::new(group))]);
    assert_eq!(tree::desugar(done), body);
    assert_eq!(ran(&body, Options::default().without(Pass::Runtime)), body);

    let other = ir::Stmt::Assign {
        target: Var::Local(0),
        value: Expr::NewObject(made_type(INDEX + 1)),
    };
    let elsewhere = eval(builtin(
        "System.Text.StringBuilder.Append",
        vec![p(1), address(0)],
        1,
    ));
    for left in [
        // An object of another type, a constructor handed the object again, or arguments no
        // overload takes.
        vec![Stmt::Simple(other), Stmt::Simple(ctor.clone())],
        vec![
            Stmt::Simple(made.clone()),
            Stmt::Simple(constructed(vec![local(0), local(0)])),
        ],
        vec![
            Stmt::Simple(made.clone()),
            Stmt::Simple(constructed(vec![local(0), int(1), int(2), int(3)])),
        ],
        // A statement between the two, and a pointer that may reach the variable.
        vec![
            Stmt::Simple(made.clone()),
            assign(Var::Local(1), int(0)),
            Stmt::Simple(ctor.clone()),
        ],
        vec![Stmt::Simple(made.clone()), Stmt::Simple(ctor), elsewhere],
    ] {
        let done = ran(&left, Options::default());
        let grouped = done.iter().any(|stmt| matches!(stmt, Stmt::Sugar(_)));
        assert!(!grouped, "{done:?}");
        assert_eq!(tree::desugar(done), left);
    }
}

/// The runtime's type of random numbers as a program states it where an object of it is made.
fn made_random(index: u32) -> VmType {
    VmType {
        name: "System.Random".to_owned(),
        byte_size: 16,
        id: BUILTIN | index,
        flags: 0,
    }
}

#[test]
fn a_static_getter_in_a_constructs_part_reads_where_that_part_takes_its_value() {
    let now = builtin("System.DateTime.get_UtcNow", Vec::new(), 1);
    let property = Expr::sugar(ExprSugar::Property, now.clone());
    let group = |parts: Vec<ir::Stmt>| {
        let kind = Group::Compound;
        Stmt::Sugar(Box::new(Sugar::Group { kind, parts }))
    };
    let switch_on = |test: Expr| {
        let arms = vec![Arm {
            test,
            body: Vec::new(),
        }];
        Stmt::Sugar(Box::new(Sugar::SwitchOn {
            arms,
            default: Vec::new(),
        }))
    };
    let head_of = |init: ir::Stmt, step: ir::Stmt| Stmt::For {
        init: Some(Box::new(init)),
        cond: None,
        step: Some(Box::new(step)),
        body: Vec::new(),
    };
    let kept = |value: Expr| ir::Stmt::Assign {
        target: Var::Local(0),
        value,
    };
    let dropped = ir::Stmt::Eval(now.clone());
    let body = vec![
        group(vec![kept(now.clone()), dropped.clone()]),
        switch_on(now.clone()),
        head_of(kept(now.clone()), dropped.clone()),
    ];
    let read = vec![
        group(vec![kept(property.clone()), dropped.clone()]),
        switch_on(property.clone()),
        head_of(kept(property), dropped),
    ];
    let done = ran(&body, Options::default());
    assert_eq!(done, read);
    assert_eq!(tree::desugar(done), tree::desugar(body.clone()));
    assert_eq!(ran(&body, Options::default().without(Pass::Runtime)), body);
}

#[test]
fn a_method_an_override_may_replace_reads_on_a_variable_that_only_ever_holds_a_new_object_of_its_type(
) {
    let path = "System.Random.NextBytes";
    let made = |var: Var, index: u32| ir::Stmt::Assign {
        target: var,
        value: Expr::NewObject(made_random(index)),
    };
    let filled = |receiver: Expr| eval(builtin(path, vec![receiver, p(1)], 0));
    let body = vec![Stmt::Simple(made(Var::Local(0), INDEX)), filled(local(0))];
    let done = ran(&body, Options::default());
    let read = eval(as_method(builtin(path, vec![local(0), p(1)], 0)));
    assert_eq!(done, [Stmt::Simple(made(Var::Local(0), INDEX)), read]);
    assert_eq!(tree::desugar(done), body);
    assert_eq!(ran(&body, Options::default().without(Pass::Runtime)), body);

    // Made in a construct's part, and in both arms of a test.
    let constructed = ir::Stmt::Eval(builtin("System.Random.Ctor", vec![local(0)], 0));
    let parts = vec![made(Var::Local(0), INDEX), constructed];
    let kind = Group::Construct;
    let grouped = vec![
        Stmt::Sugar(Box::new(Sugar::Group { kind, parts })),
        filled(local(0)),
    ];
    assert!(on_receiver(&ran(&grouped, Options::default()), path));
    let in_arms = vec![
        Stmt::If {
            cond: p(0),
            then: vec![Stmt::Simple(made(Var::Local(0), INDEX))],
            els: vec![Stmt::Simple(made(Var::Local(0), INDEX))],
        },
        filled(local(0)),
    ];
    assert!(on_receiver(&ran(&in_arms, Options::default()), path));

    let pointed = eval(builtin(
        "System.Text.StringBuilder.Append",
        vec![p(1), address(0)],
        1,
    ));
    for left in [
        // An object of another type, and a variable that takes another value too.
        vec![
            Stmt::Simple(made(Var::Local(0), INDEX + 1)),
            filled(local(0)),
        ],
        vec![
            Stmt::Simple(made(Var::Local(0), INDEX)),
            assign(Var::Local(0), p(0)),
            filled(local(0)),
        ],
        // A variable a pointer may reach, a parameter, and a variable nothing made.
        vec![
            Stmt::Simple(made(Var::Local(0), INDEX)),
            pointed.clone(),
            filled(local(0)),
        ],
        vec![Stmt::Simple(made(Var::Param(1), INDEX)), filled(p(1))],
        vec![filled(local(0))],
    ] {
        let done = ran(&left, Options::default());
        assert!(!on_receiver(&done, path), "{done:?}");
        assert_eq!(tree::desugar(done), left);
    }
}

#[test]
fn a_local_a_wider_variable_below_it_may_run_across_proves_no_object() {
    let path = "System.Random.NextBytes";
    let body = vec![
        assign(Var::Local(1), Expr::NewObject(made_random(INDEX))),
        eval(builtin(path, vec![local(1), p(1)], 0)),
    ];
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), Options::default());
    let read = |vars: BTreeMap<Var, VarInfo>| {
        let mut f = tree::Function {
            head: Head { vars, ..head() },
            body: body.clone(),
        };
        run(&mut f, &cx, &mut Stats::default());
        on_receiver(&f.body, path)
    };
    assert!(read(BTreeMap::new()));
    assert!(read(BTreeMap::from([(
        Var::Local(2),
        VarInfo::new(Width::Cells)
    )])));
    assert!(!read(BTreeMap::from([(
        Var::Local(0),
        VarInfo::new(Width::Cells)
    )])));
}

#[test]
fn a_member_every_public_overload_of_which_answers_a_bool_is_a_truth_where_the_call_is_one() {
    let overlaps = "System.Collections.Generic.HashSet_1.Overlaps";
    assert!(truth(overlaps, 2));
    // No overload takes as many arguments.
    assert!(!truth(overlaps, 1));
    assert!(!truth(overlaps, 3));
    // A member of the type, and one of an object and of the type under one name.
    assert!(truth("System.String.IsNullOrWhiteSpace", 1));
    assert!(truth("System.String.Equals", 2));
    assert!(truth(
        "System.Threading.CancellationToken.get_CanBeCanceled",
        1
    ));
    // A member one overload of which answers nothing, one that answers a number, and what
    // the table does not hold.
    assert!(!truth("System.Threading.Tasks.Task.Wait", 2));
    assert!(!truth("System.Random.NextDouble", 1));
    assert!(!truth("System.Text.StringBuilder.Method_7", 1));
    assert!(!truth("IsNullOrWhiteSpace", 1));
}

#[test]
fn every_truth_the_table_states_is_a_member_of_its_type_that_answers_once() {
    for ty in TYPES {
        for name in ty.truths {
            let answers = ty.members.iter().any(|(member, form)| {
                member == name && !matches!(form, Form::Set | Form::OpenSet | Form::New(_))
            });
            assert!(answers, "{}.{name}", ty.name);
        }
        let mut names = ty.truths.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ty.truths.len(), "{}", ty.name);
    }
}
