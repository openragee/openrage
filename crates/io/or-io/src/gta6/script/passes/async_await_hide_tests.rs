use super::super::kick::{hide_reached, names_in};
use super::super::{hide_machines, hide_machines_in, joined_steps, joins};
use super::*;
use crate::gta6::script::class::{Class, ClassKind, Method, Role, Unit};
use crate::gta6::script::ir::{ExprSugar, Kind, Scalar, TypeKey, VarInfo, Width};
use crate::gta6::script::project::ProgramInfo;
use crate::gta6::script::{Context, Names, Options, Pass, Project};

/// The type of the machine the tests write, the owner `grouped` gives a store.
const MACHINE: TypeKey = TypeKey(0x0123_4567_89AB_CDEF);

fn method(function: tree::Function, role: Role) -> Method {
    Method {
        function,
        role,
        is_static: false,
        is_async: false,
        slot: Some(1),
        overrides: false,
    }
}

/// What the compiler writes beside the step function, in the head of `like`: the builder handed the machine.
fn handing(like: &tree::Function) -> tree::Function {
    let mut handing = like.clone();
    handing.head.at = 0x610;
    handing.body = vec![
        tree::Stmt::Simple(Stmt::Store {
            kind: StoreKind::U64,
            addr: Expr::Var(Var::Param(0)),
            offset: 0x8,
            value: Expr::Var(Var::Param(1)),
        }),
        tree::Stmt::Return(None),
    ];
    handing
}

/// The machine of `MACHINE`: its step function and the compiler's method beside it.
fn machine(step: tree::Function, other: tree::Function) -> Class {
    Class {
        key: MACHINE,
        kind: ClassKind::StateMachine,
        base: None,
        interfaces: Vec::new(),
        fields: Vec::new(),
        methods: vec![method(step, Role::Step), method(other, Role::Method)],
        nested: Vec::new(),
    }
}

fn unit(classes: Vec<Class>, free: Vec<tree::Function>) -> Unit {
    let mut unit = Unit::flat(1, Vec::new(), free);
    unit.classes = classes;
    unit
}

/// Reads each load of the bytes at 0x18 in `body` as the field of the machine there, as the fields pass writes one; how many it read so.
fn fielded(body: &mut Vec<tree::Stmt>) -> usize {
    let mut read = 0usize;
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            tree::own_exprs_mut(stmt, &mut |top| {
                top.walk_mut(&mut |e| {
                    if matches!(e, Expr::Load { offset: 0x18, .. }) {
                        let load = std::mem::replace(e, Expr::GuidEmpty);
                        *e = Expr::sugar(ExprSugar::Field { owner: MACHINE }, load);
                        read += 1;
                    }
                });
            });
            work.extend(tree::nested_mut(stmt));
        }
    }
    read
}

#[test]
fn a_machine_whose_fields_alone_its_method_reads_is_not_written_and_the_readings_go() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let (functions, steps) = joined_steps(program(STARTS, HANDS, options), &cx);
    // The method as the fields pass leaves it: the parameter stored to a field of the machine,
    // and the field read where the step function read it.
    let mut kick = functions[2].clone();
    kick.body.iter_mut().for_each(grouped);
    assert!(fielded(&mut kick.body) > 0, "{:#?}", kick.body);
    let shown = |body: &[tree::Stmt]| format!("{body:?}");
    assert!(shown(&kick.body).contains("Group"), "{:#?}", kick.body);
    let (step, other) = (functions[0].clone(), handing(&functions[1]));

    let held = unit(
        vec![machine(step.clone(), other.clone())],
        vec![kick.clone()],
    );
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 1);
    assert!(written.classes.is_empty());
    let left = &written.functions[0].function;
    assert!(!shown(&left.body).contains("Field"), "{:#?}", left.body);
    // The readings are all that went: written back, the method is what it was.
    assert_eq!(
        tree::desugar(left.body.clone()),
        tree::desugar(kick.body.clone())
    );

    // ⚠ Named besides by a variable of its type, the machine stays, and so do the readings.
    let mut typed = kick.clone();
    let info = VarInfo {
        width: Width::Cell,
        kind: Kind::Object(MACHINE.to_string()),
        name: None,
        single: false,
    };
    typed.head.vars.insert(Var::Local(0), info);
    let held = unit(vec![machine(step.clone(), other.clone())], vec![typed]);
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);

    // ⚠ So it does where a function that awaits nothing reads a field of it.
    let mut reader = functions[1].clone();
    reader.head.at = 0x900;
    let load = Expr::Load {
        ty: Scalar::U64,
        addr: Box::new(Expr::Var(Var::Param(1))),
        offset: 0x18,
    };
    reader.body = vec![
        tree::Stmt::Simple(Stmt::Eval(Expr::sugar(
            ExprSugar::Field { owner: MACHINE },
            load,
        ))),
        tree::Stmt::Return(None),
    ];
    let held = unit(vec![machine(step, other)], vec![kick, reader]);
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
}

#[test]
fn a_start_function_nothing_written_calls_goes_with_the_machine_of_its_step_function() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let kick = HANDS.replace("func_000100(Vm.Address(locals, 0));", START_CALL);
    let (functions, steps) = joined_steps(program(STARTS, &kick, options), &cx);
    let (step, start, kick) = (
        functions[0].clone(),
        functions[1].clone(),
        functions[2].clone(),
    );
    let other = handing(&start);
    let held = unit(
        vec![machine(step.clone(), other.clone())],
        vec![start.clone(), kick.clone()],
    );
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 1);
    assert!(written.classes.is_empty());
    assert_eq!(written.functions.len(), 1);
    assert_eq!(written.functions[0].function, kick);

    // ⚠ Called by a function that is written, the start function stays, and it names the step function.
    let mut caller = start.clone();
    caller.head.at = 0x900;
    let call = Expr::Call {
        callee: Callee::Local {
            at: start.head.at,
            name: start.head.name.clone(),
        },
        args: vec![Expr::Var(Var::Param(0)), Expr::Var(Var::Param(1))],
        returns: None,
    };
    caller.body = vec![
        tree::Stmt::Simple(Stmt::Eval(Expr::Var(Var::Param(0)))),
        tree::Stmt::Simple(Stmt::Eval(call)),
        tree::Stmt::Return(None),
    ];
    let held = unit(vec![machine(step, other)], vec![start, kick, caller]);
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
}

#[test]
fn a_kick_off_that_asks_its_builder_for_the_task_is_joined() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let task = "return load_u64(byte_offset(Vm.Address(locals, 0), 0x8), 0x8);";
    let asked = "return func_000800(byte_offset(Vm.Address(locals, 0), 0x8));";
    let done = program(STARTS, &HANDS.replace(task, asked), options);
    let made = joins(&done, &cx);
    assert_eq!(made.len(), 1, "{:#?}", done[2].body);
    let shown = format!("{:?}", made[0].whole.body);
    assert!(shown.contains("Await"), "{shown}");
    // The task is handed back where the body ends, and every way out hands it back.
    let back = handed_back(&made[0].whole.body).expect("the task is handed back");
    assert!(format!("{back:?}").contains("func_000800"), "{back:?}");
    assert!(!shown.contains("Return(None)"), "{shown}");
    let (front, step) = made[0].parts().expect("the method goes back");
    assert_eq!((&front, &step), (&done[2], &done[0]));

    // A function given more than the builder's address, or the machine's, is not asked for the task.
    for other in [
        "return func_000800(byte_offset(Vm.Address(locals, 0), 0x8), p1);",
        "return func_000800(Vm.Address(locals, 0));",
    ] {
        let done = program(STARTS, &HANDS.replace(task, other), options);
        assert!(joins(&done, &cx).is_empty(), "{other}");
    }
}

#[test]
fn a_field_filled_from_what_a_parameter_points_at_is_a_copy_the_method_makes() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let filled = "store_ref(Vm.Address(locals, 0), 0x18, p1);";
    let through = "store_ref(Vm.Address(locals, 0), 0x18, load_u64(p1, 0x10));";
    let done = program(STARTS, &HANDS.replace(filled, through), options);
    let made = joins(&done, &cx);
    assert_eq!(made.len(), 1, "{:#?}", done[2].body);
    // The copy stays in sight, where it ran.
    let top = stated(&made[0].whole.body);
    assert!(top.contains("Param(1)") && top.contains("Load"), "{top}");
    let (front, step) = made[0].parts().expect("the method goes back");
    assert_eq!((&front, &step), (&done[2], &done[0]));

    // ⚠ A value a call makes, or one read through a variable of the frame, is none the method was given.
    for other in [
        "store_ref(Vm.Address(locals, 0), 0x18, func_000900(p1));",
        "store_ref(Vm.Address(locals, 0), 0x18, load_u64(local_11, 0x10));",
    ] {
        let done = program(STARTS, &HANDS.replace(filled, other), options);
        assert!(joins(&done, &cx).is_empty(), "{other}");
    }
}

/// The step function, the start function and the kick-off of a method that starts its machine through a start function, joined.
fn started_program() -> (
    BTreeSet<u32>,
    tree::Function,
    tree::Function,
    tree::Function,
) {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let kick = HANDS.replace("func_000100(Vm.Address(locals, 0));", START_CALL);
    let (functions, steps) = joined_steps(program(STARTS, &kick, options), &cx);
    let (step, start, kick) = (
        functions[0].clone(),
        functions[1].clone(),
        functions[2].clone(),
    );
    (steps, step, start, kick)
}

#[test]
fn a_machine_another_program_may_reach_stays_written_and_so_does_its_start_function() {
    let (steps, step, start, kick) = started_program();
    let other = handing(&start);
    let held = unit(
        vec![machine(step.clone(), other.clone())],
        vec![start.clone(), kick.clone()],
    );
    let nowhere = |_: u32| false;
    let unnamed = |_: TypeKey| false;
    // Reached by no other program, the machine and its start function go.
    let mut written = held.clone();
    assert_eq!(hide_reached(&mut written, &steps, &nowhere, &unnamed), 1);
    assert_eq!(written.functions.len(), 1);
    assert_eq!(written.functions[0].function, kick);

    // ⚠ Its step function, its other method or its start function reached from outside keeps it.
    for at in [step.head.at, other.head.at, start.head.at] {
        let reached = move |of: u32| of == at;
        let mut written = held.clone();
        assert_eq!(
            hide_reached(&mut written, &steps, &reached, &unnamed),
            0,
            "{at:#x}"
        );
        assert_eq!(written, held);
    }
    // ⚠ So does its type named outside.
    let named = |key: TypeKey| key == MACHINE;
    let mut written = held.clone();
    assert_eq!(hide_reached(&mut written, &steps, &nowhere, &named), 0);
    assert_eq!(written, held);
    // Another type named outside is none of the machine's.
    let elsewhere = |key: TypeKey| key != MACHINE;
    let mut written = held.clone();
    assert_eq!(hide_reached(&mut written, &steps, &nowhere, &elsewhere), 1);
}

#[test]
fn a_machine_of_a_program_the_project_holds_and_nothing_imports_is_not_written() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let (functions, steps) = joined_steps(program(STARTS, HANDS, options), &cx);
    let (step, other) = (functions[0].clone(), handing(&functions[1]));
    let held = unit(vec![machine(step, other)], vec![functions[2].clone()]);
    let mut alone = Project::default();
    let info = ProgramInfo {
        program: held.program,
        ..ProgramInfo::default()
    };
    alone.programs.insert(held.program, info);
    let mut written = held.clone();
    assert_eq!(hide_machines_in(&mut written, &steps, &alone), 1);
    assert!(written.classes.is_empty());
    // ⚠ A program the project does not hold may be imported by any: its machines stay.
    let mut written = held.clone();
    assert_eq!(hide_machines_in(&mut written, &steps, Project::empty()), 0);
    assert_eq!(written, held);
}

#[test]
fn a_start_function_placed_in_a_class_goes_with_its_machine_as_a_free_one_does() {
    let (steps, step, start, kick) = started_program();
    let other = handing(&start);
    let placed = |slot: Option<u32>| Class {
        key: TypeKey(0x0FED_CBA9_8765_4321),
        kind: ClassKind::Class,
        base: None,
        interfaces: Vec::new(),
        fields: Vec::new(),
        methods: vec![Method {
            function: start.clone(),
            role: Role::Method,
            is_static: true,
            is_async: false,
            slot,
            overrides: false,
        }],
        nested: Vec::new(),
    };
    let held = unit(
        vec![machine(step.clone(), other.clone()), placed(None)],
        vec![kick.clone()],
    );
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 1);
    assert_eq!(written.classes.len(), 1);
    assert!(
        written.classes[0].methods.is_empty(),
        "{:#?}",
        written.classes
    );
    assert_eq!(written.functions.len(), 1);
    assert_eq!(written.functions[0].function, kick);

    // ⚠ Filling a virtual slot, it may be called through the slot: it stays, and so does the machine it starts.
    let held = unit(vec![machine(step, other), placed(Some(2))], vec![kick]);
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
}

#[test]
fn a_reference_whose_top_byte_holds_flags_counts_as_a_use_of_the_function_at_its_offset() {
    let body = vec![tree::Stmt::Simple(Stmt::Eval(Expr::FunctionRef {
        program: 1,
        at: 0x4000_0080,
    }))];
    let mut named = std::collections::BTreeMap::new();
    names_in(&body, 1, &mut named);
    assert_eq!(named, std::collections::BTreeMap::from([(0x80, 1)]));
}
