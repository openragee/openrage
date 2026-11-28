use super::*;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::{blocks_in, Names, Options, Project};

// ---- what the tests build with ------------------------------------------------------------

/// A listing of one function of one parameter, written by hand, around the runtime methods a state machine calls.
// The methods carry names no runtime has: the pass must know them by where they stand.
const FRAME: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000100)]
    public static void func_000100(VmValue arg0) // 0x000100
    {
        VmValue[] parameters = new VmValue[] { arg0 };
        VmValue[] locals = new VmValue[8];
        BODY
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Builtins
    {
        public static class Demo
        {
            public static class Task
            {
                public static VmValue Done(VmValue p0) => Vm.Builtin(7, 1, 1, p0)[0];
                public static VmValue Finished() => Vm.Builtin(7, 2, 1)[0];
                public static void Finish(VmValue p0) => Vm.Builtin(7, 3, 0, p0);
                public static void Wait(VmValue p0, VmValue p1, VmValue p2, VmValue p3, VmValue p4) => Vm.Builtin(7, 4, 0, p0, p1, p2, p3, p4);
            }
        }
    }
    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
        }
    }
}
"#;

/// A step function of one await: the dispatch, the awaiter taken and tested.
const HEAD: &str = "
        local_0 = load_i32(p0, 0x0);
        local_1 = load_u64(p0, 0x18);
        if (((as_u32(local_0) == 0))) { goto label_000180; } else { goto label_000110; }
        label_000110: ;
        local_2 = Native.DEMO.GET(local_1);
        if (((as_u32(Builtins.Demo.Task.Done(local_2)) != 0))) { goto label_0001A0; } else { goto label_000120; }
        label_000120: ;";

/// The suspend path with the first suspension made in line.
const IN_LINE: &str = "
        local_0 = 0;
        store_i32(p0, 0x0, 0);
        store_ref(p0, 0x20, local_2);
        local_4 = byte_offset(p0, 0x8);
        if (((as_u64(load_u64(local_4, 0x8)) != 0))) { goto label_000160; } else { goto label_000140; }
        label_000140: ;
        store_ref(local_4, 0x8, Vm.NewObject(new VmType(\"made_task\")));
        local_5 = Vm.TypedIntrinsic(\"box_at\", new VmType(\"type_0123456789ABCDEF\", 40, 77U, 0x00100006U), p0);
        store_ref(byte_offset(byte_offset(local_5, 8), 0x8), 0x0, local_5);
        store_ref(local_4, 0x0, local_5);
        goto label_000160;
        label_000160: ;
        Builtins.Demo.Task.Wait(local_4, local_2, 0, 0, 0);
        return;";

/// The suspend path that hands control back through a function of the scripts.
const THROUGH: &str = "
        local_0 = 0;
        store_i32(p0, 0x0, 0);
        store_ref(p0, 0x20, local_2);
        func_000300(byte_offset(p0, 0x8), Vm.Address(locals, 2), p0);
        return;";

/// The resume point, what follows the await, and the end of a machine with no result.
const REST: &str = "
        label_000180: ;
        local_2 = load_u64(p0, 0x20);
        zero_bytes(byte_offset(p0, 0x20), 8);
        local_0 = (-1);
        store_i32(p0, 0x0, (-1));
        goto label_0001A0;
        label_0001A0: ;
        Native.DEMO.USE(local_2);
        store_i32(p0, 0x0, (-2));
        store_ref(p0, 0x20, 0);
        if (((as_u64(load_u64(byte_offset(p0, 0x8), 0x8)) != 0))) { goto label_0001E0; } else { goto label_0001C0; }
        label_0001C0: ;
        store_ref(byte_offset(p0, 0x8), 0x8, Builtins.Demo.Task.Finished());
        return;
        label_0001E0: ;
        Builtins.Demo.Task.Finish(load_u64(byte_offset(p0, 0x8), 0x8));
        return;";

/// Two awaits one after the other, the second state found by a second test, and an end left as it is.
const TWO: &str = "
        local_0 = load_i32(p0, 0x0);
        if (((as_u32(local_0) == 0))) { goto label_000180; } else { goto label_000104; }
        label_000104: ;
        if (((as_i32(local_0) == as_i32(1)))) { goto label_000280; } else { goto label_000110; }
        label_000110: ;
        local_2 = Native.DEMO.GET(load_u64(p0, 0x18));
        if (((as_u32(Builtins.Demo.Task.Done(local_2)) != 0))) { goto label_0001A0; } else { goto label_000120; }
        label_000120: ;
        store_i32(p0, 0x0, 0);
        store_ref(p0, 0x20, local_2);
        func_000300(byte_offset(p0, 0x8), Vm.Address(locals, 2), p0);
        return;
        label_000180: ;
        local_2 = load_u64(p0, 0x20);
        zero_bytes(byte_offset(p0, 0x20), 8);
        store_i32(p0, 0x0, (-1));
        goto label_0001A0;
        label_0001A0: ;
        local_3 = Native.DEMO.GET(local_2);
        if (((as_u32(Builtins.Demo.Task.Done(local_3)) != 0))) { goto label_0002A0; } else { goto label_000220; }
        label_000220: ;
        store_i32(p0, 0x0, 1);
        store_ref(p0, 0x28, local_3);
        func_000300(byte_offset(p0, 0x8), Vm.Address(locals, 3), p0);
        return;
        label_000280: ;
        local_3 = load_u64(p0, 0x28);
        zero_bytes(byte_offset(p0, 0x28), 8);
        store_i32(p0, 0x0, (-1));
        goto label_0002A0;
        label_0002A0: ;
        Native.DEMO.USE(local_3);
        store_i32(p0, 0x0, (-2));
        return;";

/// An await in a loop: the resume point goes on in the middle of the loop's body.
const LOOP: &str = "
        local_0 = load_i32(p0, 0x0);
        if (((as_u32(local_0) == 0))) { goto label_000180; } else { goto label_000108; }
        label_000108: ;
        if (((as_i32(load_i32(p0, 0x28)) < as_i32(3)))) { goto label_000110; } else { goto label_0001B0; }
        label_000110: ;
        local_2 = Native.DEMO.GET(load_i32(p0, 0x28));
        if (((as_u32(Builtins.Demo.Task.Done(local_2)) != 0))) { goto label_0001A0; } else { goto label_000120; }
        label_000120: ;
        local_0 = 0;
        store_i32(p0, 0x0, 0);
        store_ref(p0, 0x20, local_2);
        func_000300(byte_offset(p0, 0x8), Vm.Address(locals, 2), p0);
        return;
        label_000180: ;
        local_2 = load_u64(p0, 0x20);
        zero_bytes(byte_offset(p0, 0x20), 8);
        local_0 = (-1);
        store_i32(p0, 0x0, (-1));
        goto label_0001A0;
        label_0001A0: ;
        increment_i32(p0, 0x28);
        goto label_000108;
        label_0001B0: ;
        store_i32(p0, 0x0, (-2));
        return;";

fn in_line() -> String {
    [HEAD, IN_LINE, REST].concat()
}

/// The blocks of the function that holds `body`, as the reader and the lowering leave them.
fn step(body: &str) -> cfg::Function {
    let text = FRAME.replace("BODY", body);
    let listing = Listing::open(&text).expect("the listing opens");
    let read = listing.functions().next().expect("a function");
    cfg::lower(read.expect("the function reads")).expect("the function lowers")
}

/// `untangle` over the function that holds `body`: the blocks before, the blocks after, and what was taken out.
fn untangled(body: &str) -> (cfg::Function, cfg::Function, Option<Machine>) {
    let read = step(body);
    let mut f = read.clone();
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let machine = untangle(&mut f, &cx, &mut Stats::default());
    (read, f, machine)
}

/// The body once `run` has put the pieces back, with nothing between but the blocks written in their order, and the blocks that body lowers to.
fn put_back(f: cfg::Function, machine: Machine) -> (tree::Function, cfg::Function) {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let mut flat = cfg::flat(f);
    run(&mut flat, machine, &cx, &mut Stats::default());
    let back = cfg::lower(flat.clone()).expect("the body lowers");
    (flat, back)
}

/// Whether two graphs of blocks are one: the same statements and the same tests along every path from the entry.
fn same(a: &Cfg, b: &Cfg) -> bool {
    let shape = |term: &Terminator| {
        let mut term = term.clone();
        term.retarget(|_| ENTRY);
        term
    };
    let mut paired: BTreeMap<BlockId, BlockId> = BTreeMap::new();
    let mut work = vec![(ENTRY, ENTRY)];
    while let Some((x, y)) = work.pop() {
        match paired.insert(x, y) {
            Some(was) if was == y => continue,
            Some(_) => return false,
            None => {}
        }
        let (one, two) = (&a.blocks[x], &b.blocks[y]);
        let (next, other) = (one.term.successors(), two.term.successors());
        if one.stmts != two.stmts
            || shape(&one.term) != shape(&two.term)
            || next.len() != other.len()
        {
            return false;
        }
        work.extend(next.into_iter().zip(other));
    }
    paired.len() == a.blocks.len() && a.blocks.len() == b.blocks.len()
}

/// The awaits a body holds at its top.
fn awaits(body: &[tree::Stmt]) -> usize {
    let awaited = |stmt: &&tree::Stmt| match stmt {
        tree::Stmt::Sugar(sugar) => matches!(&**sugar, Sugar::Await { .. }),
        _ => false,
    };
    body.iter().filter(awaited).count()
}

/// What each await of a body holds as the statements that take its awaiter, as they print for a failure.
fn takings(body: &[tree::Stmt]) -> Vec<String> {
    let mut found = Vec::new();
    tree::each_sugar(body, &mut |sugar| {
        if let Sugar::Await { before, .. } = sugar {
            found.push(format!("{before:?}"));
        }
    });
    found
}

/// The `goto` statements the readable form writes: those a source construct holds are not written.
fn written_gotos(body: &[tree::Stmt]) -> usize {
    body.iter()
        .map(|stmt| match stmt {
            tree::Stmt::Goto(_) => 1,
            tree::Stmt::If { then, els, .. } => written_gotos(then) + written_gotos(els),
            tree::Stmt::While { body, .. }
            | tree::Stmt::DoWhile { body, .. }
            | tree::Stmt::For { body, .. } => written_gotos(body),
            tree::Stmt::Switch { cases, .. } => {
                cases.iter().map(|case| written_gotos(&case.body)).sum()
            }
            _ => 0,
        })
        .sum()
}

/// Asserts that the function that holds `body` is left as it was read.
fn left(body: &str) {
    let (read, f, machine) = untangled(body);
    assert!(machine.is_none(), "taken apart:\n{body}");
    assert_eq!(f, read, "the blocks changed:\n{body}");
}

// ---- a machine that is proved ---------------------------------------------------------------

#[test]
fn a_machine_is_taken_apart_and_runs_as_it_did_once_put_back() {
    let (read, f, machine) = untangled(&in_line());
    let machine = machine.expect("the machine is proved");
    // What is left: the entry, the block that awaits and what follows the await.
    assert_eq!(f.cfg.blocks.len(), 3, "{:#?}", f.cfg);
    // Taken out: the read of the state, the dispatch, the await and the end.
    assert_eq!(machine.pieces.len(), 4);

    let (flat, back) = put_back(f, machine);
    assert_eq!(awaits(&flat.body), 1, "{:#?}", flat.body);
    assert_eq!(written_gotos(&flat.body), 0, "{:#?}", flat.body);
    // Written back, the pieces are the step function's own blocks.
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
}

#[test]
fn a_suspend_path_through_a_script_function_is_one_too() {
    let (read, f, machine) = untangled(&[HEAD, THROUGH, REST].concat());
    let (flat, back) = put_back(f, machine.expect("the machine is proved"));
    assert_eq!(awaits(&flat.body), 1, "{:#?}", flat.body);
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
}

#[test]
fn each_state_of_a_chain_of_tests_is_an_await() {
    let (read, f, machine) = untangled(TWO);
    let machine = machine.expect("the machine is proved");
    assert_eq!(f.cfg.blocks.len(), 4, "{:#?}", f.cfg);
    let (flat, back) = put_back(f, machine);
    assert_eq!(awaits(&flat.body), 2, "{:#?}", flat.body);
    // The end hands a result to nothing the proof knows, so it stays in sight.
    let shown = format!("{:?}", flat.body);
    assert!(shown.matches("Hidden").count() == 2, "{shown}");
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
}

#[test]
fn a_piece_written_twice_keeps_one_resume_point() {
    let (_, f, machine) = untangled(&[HEAD, THROUGH, REST].concat());
    let machine = machine.expect("the machine is proved");
    // What `structure` does to a short tail that leaves: the same statements at two places.
    let mut flat = cfg::flat(f);
    let twice = flat.body.clone();
    flat.body.extend(twice);
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    run(&mut flat, machine, &cx, &mut Stats::default());
    assert_eq!(awaits(&flat.body), 2, "{:#?}", flat.body);
    // The dispatch has one label to jump to: the same label a second time would not lower.
    assert!(cfg::lower(flat).is_ok());
}

#[test]
fn an_await_in_a_loop_leaves_the_loop_whole() {
    let names = Names::default();
    let done = |options: Options| {
        let cx = Context::new(&names, Project::empty(), options);
        blocks_in(step(LOOP), &cx, &mut Stats::default())
    };
    // The dispatch jumps into the loop's body: as it stands the body takes a `goto`.
    let before = done(Options::baseline());
    assert!(tree::counts(&before.body).gotos > 0, "{:#?}", before.body);

    let after = done(Options::baseline().with(Pass::Await));
    assert_eq!(written_gotos(&after.body), 0, "{:#?}", after.body);
    let looped = |stmt: &tree::Stmt| {
        matches!(
            stmt,
            tree::Stmt::While { .. } | tree::Stmt::For { .. } | tree::Stmt::DoWhile { .. }
        )
    };
    assert!(after.body.iter().any(looped), "{:#?}", after.body);
    assert!(format!("{:?}", after.body).contains("Await"));
}

#[test]
fn an_await_is_one_statement_after_the_passes_before_it() {
    let names = Names::default();
    // What a body states outside its source constructs, as the statements print for a failure.
    let written = |options: Options| {
        let cx = Context::new(&names, Project::empty(), options);
        let done = blocks_in(step(&in_line()), &cx, &mut Stats::default());
        let stated: Vec<&Stmt> = done
            .body
            .iter()
            .filter_map(|stmt| match stmt {
                tree::Stmt::Simple(simple) => Some(simple),
                _ => None,
            })
            .collect();
        (
            awaits(&done.body),
            written_gotos(&done.body),
            format!("{stated:?} {:?}", takings(&done.body)),
        )
    };
    // After `values` and `types` the machine is still proved, and `structure` keeps each piece.
    // What is awaited stays in sight: before the await, or as the statement it takes its awaiter from.
    let (awaited, gotos, stated) = written(Options::baseline().with(Pass::Await));
    assert_eq!((awaited, gotos), (1, 0), "{stated}");
    assert!(stated.contains("GET"), "{stated}");
    // `USE` is given the awaiter alone where the await is done: it is the await's own part.
    for hidden in ["USE", "Wait", "Finish", "box_at"] {
        assert!(!stated.contains(hidden), "`{hidden}` in:\n{stated}");
    }

    // Switched off by its name, the pass leaves the step function as it was written.
    let (awaited, _, _) = written(Options::baseline());
    assert_eq!(awaited, 0);
}

// ---- machines that are left -----------------------------------------------------------------

#[test]
fn the_pass_does_nothing_where_its_name_is_off() {
    let read = step(&in_line());
    let mut f = read.clone();
    let names = Names::default();
    let options = Options::baseline().with(Pass::Iterators).with(Pass::Wait);
    let cx = Context::new(&names, Project::empty(), options);
    assert!(untangle(&mut f, &cx, &mut Stats::default()).is_none());
    assert_eq!(f, read);
}

#[test]
fn a_state_read_after_the_dispatch_leaves_the_machine() {
    // What a `finally` leaves behind: the code after the await asks what state it is in.
    left(&in_line().replace("Native.DEMO.USE(local_2);", "Native.DEMO.USE(local_0);"));
}

#[test]
fn the_machines_address_handed_on_leaves_the_machine() {
    left(&in_line().replace("Native.DEMO.USE(local_2);", "Native.DEMO.USE(p0);"));
    // Through a copy of it too.
    let copied = "local_6 = p0; Native.DEMO.USE(local_6);";
    left(&in_line().replace("Native.DEMO.USE(local_2);", copied));
}

#[test]
fn a_resume_point_reached_from_elsewhere_leaves_the_machine() {
    let entered = "local_2 = Native.DEMO.GET(local_1);
        if (((as_u32(local_2) == 0))) { goto label_000180; } else { goto label_000118; }
        label_000118: ;";
    left(&in_line().replace("local_2 = Native.DEMO.GET(local_1);", entered));
}

#[test]
fn a_resume_point_that_does_more_leaves_the_machine() {
    let more = "local_0 = (-1); Native.DEMO.USE(local_2);";
    left(&in_line().replace("local_0 = (-1);", more));
}

#[test]
fn a_suspend_path_that_does_more_leaves_the_machine() {
    let more = "store_ref(p0, 0x20, local_2); Native.DEMO.USE(local_2);";
    left(&in_line().replace("store_ref(p0, 0x20, local_2);", more));
    // The same on the way through a script function.
    let through = [HEAD, THROUGH, REST].concat();
    left(&through.replace("store_ref(p0, 0x20, local_2);", more));
}

#[test]
fn a_suspend_path_of_another_state_leaves_the_machine() {
    left(&in_line().replace("store_i32(p0, 0x0, 0);", "store_i32(p0, 0x0, 1);"));
}

#[test]
fn a_test_that_is_no_runtime_call_leaves_the_machine() {
    let asked = "Builtins.Demo.Task.Done(local_2)";
    left(&in_line().replace(asked, "Native.DEMO.GET(local_2)"));
    // A test that sends a ready awaiter to the suspend path is none either.
    let ready = "as_u32(Builtins.Demo.Task.Done(local_2)) != 0";
    left(&in_line().replace(ready, "as_u32(Builtins.Demo.Task.Done(local_2)) == 0"));
}

#[test]
fn an_entry_that_does_more_than_read_the_machine_leaves_the_machine() {
    // ⚠ The entry runs at every resumed call: read as one body, the call would run once.
    let read = "local_1 = load_u64(p0, 0x18);";
    let called = "local_1 = load_u64(p0, 0x18); Native.DEMO.USE(77);";
    left(&in_line().replace(read, called));
    left(&in_line().replace(read, "local_1 = Native.DEMO.GET(7);"));
}

#[test]
fn a_frame_variable_set_before_an_await_and_read_after_it_leaves_the_machine() {
    let taken = "local_2 = Native.DEMO.GET(local_1);";
    let before = "local_3 = Native.DEMO.GET(9); local_2 = Native.DEMO.GET(local_1);";
    let used = "Native.DEMO.USE(local_2);";
    // ⚠ A resumed call has a new frame: `local_3` is zero there, and the one body would hold it.
    let after = "Native.DEMO.USE(local_2); Native.DEMO.USE(local_3);";
    left(&in_line().replace(taken, before).replace(used, after));
    // What the entry read is read again at a resumed call, unless something else sets it too.
    let again = "Native.DEMO.USE(local_2); Native.DEMO.USE(local_1);";
    let (.., machine) = untangled(&in_line().replace(used, again));
    assert!(machine.is_some());
    let reset = "local_1 = 5; local_2 = Native.DEMO.GET(local_1);";
    left(&in_line().replace(taken, reset).replace(used, again));
    // Set again after the await before it is read, it crosses no await.
    let set = "local_3 = Native.DEMO.GET(8); Native.DEMO.USE(local_2); Native.DEMO.USE(local_3);";
    let (.., machine) = untangled(&in_line().replace(taken, before).replace(used, set));
    assert!(machine.is_some());
    // ⚠ What is written through its address before the await is no more held after it than what is assigned.
    let stored = "store_i32(Vm.Address(locals, 3), 0x0, 9); local_2 = Native.DEMO.GET(local_1);";
    left(&in_line().replace(taken, stored).replace(used, after));
    // Written through its address only after the await, a variable holds nothing across it.
    let later = "store_i32(Vm.Address(locals, 3), 0x0, 9); Native.DEMO.USE(local_2); Native.DEMO.USE(local_3);";
    let (.., machine) = untangled(&in_line().replace(used, later));
    assert!(machine.is_some());
    // ⚠ Its address handed on before the await, whatever took it may have written it.
    let handed = "Native.DEMO.USE(Vm.Address(locals, 3)); local_2 = Native.DEMO.GET(local_1);";
    left(&in_line().replace(taken, handed).replace(used, after));
    // Cleared after the await, a variable is what a new frame holds, whatever was written before.
    let cleared =
        "zero_bytes(Vm.Address(locals, 3), 8); Native.DEMO.USE(local_2); Native.DEMO.USE(local_3);";
    let (.., machine) = untangled(&in_line().replace(taken, stored).replace(used, cleared));
    assert!(machine.is_some());
    // ⚠ The awaiter is the variable the suspend path kept: another one it is taken back into is not.
    let other = in_line()
        .replace(
            "local_2 = load_u64(p0, 0x20);",
            "local_6 = load_u64(p0, 0x20);",
        )
        .replace(used, "Native.DEMO.USE(local_6);");
    left(&other);
}

#[test]
fn a_dispatch_by_range_leaves_the_machine() {
    left(&in_line().replace("as_u32(local_0) == 0", "as_u32(local_0) <= 0"));
}

// ---- what an await and an end hand on ----------------------------------------------------------

/// The resume point, a result taken from the awaiter, and an end that completes the task through a function of the scripts.
const HANDED: &str = "
        label_000180: ;
        local_2 = load_u64(p0, 0x20);
        zero_bytes(byte_offset(p0, 0x20), 8);
        local_0 = (-1);
        store_i32(p0, 0x0, (-1));
        goto label_0001A0;
        label_0001A0: ;
        local_3 = Native.DEMO.GET(local_2);
        store_i32(p0, 0x0, (-2));
        store_ref(p0, 0x20, 0);
        func_000400(byte_offset(p0, 0x8), local_3);
        return;";

/// What each await of a body holds as the statement that takes its result.
fn results(body: &[tree::Stmt]) -> Vec<String> {
    let mut found = Vec::new();
    tree::each_sugar(body, &mut |sugar| {
        if let Sugar::Await { after, .. } = sugar {
            found.push(format!("{after:?}"));
        }
    });
    found
}

/// The statements a body states at its top, outside every source construct, as they print for a failure.
fn stated(body: &[tree::Stmt]) -> String {
    let simples: Vec<&Stmt> = body
        .iter()
        .filter_map(|stmt| match stmt {
            tree::Stmt::Simple(simple) => Some(simple),
            _ => None,
        })
        .collect();
    format!("{simples:?}")
}

/// Whether a statement at the top of a body stores the finished state.
fn finish_in_sight(body: &[tree::Stmt]) -> bool {
    body.iter().any(|stmt| {
        matches!(
            stmt,
            tree::Stmt::Simple(Stmt::Store {
                kind: StoreKind::I32,
                offset: 0,
                value: Expr::Const(c),
                ..
            }) if c.bits as i32 == FINISHED
        )
    })
}

/// The body of the function that holds `body`, its machine proved and put back.
fn rebuilt(body: &str) -> (cfg::Function, tree::Function, cfg::Function) {
    let (read, f, machine) = untangled(body);
    let (flat, back) = put_back(f, machine.expect("the machine is proved"));
    (read, flat, back)
}

#[test]
fn the_statement_that_asks_the_awaiter_for_its_result_is_the_awaits_own() {
    let (read, flat, back) = rebuilt(&in_line());
    let taken = results(&flat.body);
    assert_eq!(taken.len(), 1, "{taken:?}");
    assert!(taken[0].contains("USE"), "{taken:?}");
    assert!(!stated(&flat.body).contains("USE"));
    // Written back, it runs where it ran: first after the await, on either way out of it.
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);

    // A call that is given anything but the awaiter is a statement of the method.
    let other = in_line().replace("Native.DEMO.USE(local_2);", "Native.DEMO.USE(local_1);");
    let (_, flat, _) = rebuilt(&other);
    assert_eq!(results(&flat.body), ["[]"]);
    assert!(stated(&flat.body).contains("USE"));

    // ⚠ So is one in a block another way leads into: taken by the await, it would not run on that way.
    let entered = "if (((as_u32(local_1) == 0))) { goto label_0001A0; } else { goto label_000118; }
        label_000118: ;
        local_2 = Native.DEMO.GET(local_1);";
    let beside = in_line().replace("local_2 = Native.DEMO.GET(local_1);", entered);
    let (read, flat, back) = rebuilt(&beside);
    assert_eq!(results(&flat.body), ["[]"]);
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
}

#[test]
fn an_end_that_completes_the_task_through_a_function_hides_what_the_compiler_wrote() {
    // Given a result, the call is the method's `return` of it and stays; the finished state and the cleared field go.
    let (read, flat, back) = rebuilt(&[HEAD, THROUGH, HANDED].concat());
    assert!(
        stated(&flat.body).contains("func_000400"),
        "{:#?}",
        flat.body
    );
    assert!(!finish_in_sight(&flat.body), "{:#?}", flat.body);
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
    let given = [Expr::Var(Var::Local(3))];
    assert_eq!(result(&flat.body), AsyncResult::TaskOf(&given));
    assert!(results(&flat.body)[0].contains("GET"));
    // Its head says the method's task holds a value, where `await` is on.
    let head = |options: Options| {
        let held = crate::gta6::script::class::Method {
            function: flat.clone(),
            role: crate::gta6::script::class::Role::Method,
            is_static: true,
            is_async: true,
            slot: None,
            overrides: false,
        };
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), options);
        let mut out = String::new();
        crate::gta6::script::print::method(&mut out, &held, None, 0, &cx);
        out.lines().next().unwrap_or_default().to_owned()
    };
    assert!(
        head(Options::default()).starts_with("async Task<"),
        "{}",
        head(Options::default())
    );
    let off = Options::default().without(Pass::Await);
    assert!(head(off).starts_with("async void "), "{}", head(off));

    // Given the builder alone, the call completes a task with no result, and goes with the rest.
    let alone = HANDED.replace("byte_offset(p0, 0x8), local_3", "byte_offset(p0, 0x8)");
    let (read, flat, back) = rebuilt(&[HEAD, THROUGH, &alone].concat());
    assert!(
        !stated(&flat.body).contains("func_000400"),
        "{:#?}",
        flat.body
    );
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
    assert_eq!(result(&flat.body), AsyncResult::Task);

    // ⚠ A call that is not given the builder first is no end: everything stays in sight.
    let turned = "local_3, byte_offset(p0, 0x8)";
    let elsewhere = HANDED.replace("byte_offset(p0, 0x8), local_3", turned);
    let (_, flat, _) = rebuilt(&[HEAD, THROUGH, &elsewhere].concat());
    assert!(finish_in_sight(&flat.body), "{:#?}", flat.body);
    assert_eq!(result(&flat.body), AsyncResult::Unknown);
}

#[test]
fn what_a_method_hands_back_is_read_from_how_its_machine_ends() {
    // The task completed in line is a task with no result.
    let (_, flat, _) = rebuilt(&in_line());
    assert_eq!(result(&flat.body), AsyncResult::Task);
    // An end no rule proves says nothing.
    let (_, flat, _) = rebuilt(TWO);
    assert_eq!(result(&flat.body), AsyncResult::Unknown);
    // A body with no await was never proved a machine, whatever it stores.
    let read = cfg::flat(step(&in_line()));
    assert_eq!(result(&read.body), AsyncResult::Unknown);
}

// ---- a kick-off --------------------------------------------------------------------------------

/// A listing of one function of two parameters that hands back a cell: the form of a kick-off.
const KICK: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000200)]
    public static VmValue func_000200(VmValue arg0, VmValue arg1) // 0x000200
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[16];
        BODY
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Builtins
    {
        public static class Demo
        {
            public static class Task
            {
                public static VmValue Done(VmValue p0) => Vm.Builtin(7, 1, 1, p0)[0];
                public static VmValue Finished() => Vm.Builtin(7, 2, 1)[0];
                public static void Finish(VmValue p0) => Vm.Builtin(7, 3, 0, p0);
            }
        }
    }
    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
        }
    }
}
"#;

/// A kick-off that runs its step function's code in line: the machine in `local_0`, its address in `local_7`.
const IN_PLACE: &str = "
        zero_bytes(byte_offset(Vm.Address(locals, 0), 0x8), 16);
        store_ref(Vm.Address(locals, 0), 0x18, p0);
        store_i32(Vm.Address(locals, 0), 0x20, p1);
        store_i32(Vm.Address(locals, 0), 0x0, (-1));
        local_7 = Vm.Address(locals, 0);
        local_8 = load_i32(local_7, 0x0);
        local_9 = load_u64(local_7, 0x18);
        if (((as_u32(local_8) == 0))) { goto label_000280; } else { goto label_000210; }
        label_000210: ;
        local_10 = Native.DEMO.GET(local_9);
        if (((as_u32(Builtins.Demo.Task.Done(local_10)) != 0))) { goto label_0002A0; } else { goto label_000220; }
        label_000220: ;
        local_8 = 0;
        store_i32(local_7, 0x0, 0);
        store_ref(local_7, 0x28, local_10);
        func_000300(byte_offset(local_7, 0x8), Vm.Address(locals, 10), local_7);
        goto label_0002F0;
        label_000280: ;
        local_10 = load_u64(local_7, 0x28);
        zero_bytes(byte_offset(local_7, 0x28), 8);
        local_8 = (-1);
        store_i32(local_7, 0x0, (-1));
        goto label_0002A0;
        label_0002A0: ;
        Native.DEMO.USE(local_10);
        store_i32(local_7, 0x0, (-2));
        if (((as_u64(load_u64(byte_offset(local_7, 0x8), 0x8)) != 0))) { goto label_0002E0; } else { goto label_0002C0; }
        label_0002C0: ;
        store_ref(byte_offset(local_7, 0x8), 0x8, Builtins.Demo.Task.Finished());
        goto label_0002F0;
        label_0002E0: ;
        Builtins.Demo.Task.Finish(load_u64(byte_offset(local_7, 0x8), 0x8));
        goto label_0002F0;
        label_0002F0: ;
        return load_u64(byte_offset(Vm.Address(locals, 0), 0x8), 0x8);";

/// A kick-off that calls its step function, the function of `FRAME`.
const CALLS: &str = "
        zero_bytes(byte_offset(Vm.Address(locals, 0), 0x8), 16);
        store_ref(Vm.Address(locals, 0), 0x18, p1);
        store_i32(Vm.Address(locals, 0), 0x0, (-1));
        local_7 = Vm.Address(locals, 0);
        func_000100(local_7);
        return load_u64(byte_offset(Vm.Address(locals, 0), 0x8), 0x8);";

/// The blocks of the kick-off that holds `body`.
fn kick_off(body: &str) -> cfg::Function {
    let text = KICK.replace("BODY", body);
    let listing = Listing::open(&text).expect("the listing opens");
    let read = listing.functions().next().expect("a function");
    cfg::lower(read.expect("the function reads")).expect("the function lowers")
}

/// `untangle` over the kick-off that holds `body`: the blocks before, the blocks after, and what was taken out.
fn kicked(body: &str) -> (cfg::Function, cfg::Function, Option<Machine>) {
    let read = kick_off(body);
    let mut f = read.clone();
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let machine = untangle(&mut f, &cx, &mut Stats::default());
    (read, f, machine)
}

/// Asserts that the kick-off that holds `body` is left as it was read.
fn left_kicked(body: &str) {
    let (read, f, machine) = kicked(body);
    assert!(machine.is_none(), "taken apart:\n{body}");
    assert_eq!(f, read, "the blocks changed:\n{body}");
}

#[test]
fn a_kick_off_that_runs_its_step_in_line_is_read_through_the_machine_it_holds() {
    use crate::gta6::script::exec;

    let (read, f, machine) = kicked(IN_PLACE);
    let (flat, back) = put_back(f, machine.expect("the kick-off is proved"));
    assert_eq!(awaits(&flat.body), 1, "{:#?}", flat.body);
    assert_eq!(written_gotos(&flat.body), 0, "{:#?}", flat.body);
    assert!(results(&flat.body)[0].contains("USE"));
    // In sight: each parameter copied into the machine, the read of one back, and what is awaited.
    let copies = |stmt: &&tree::Stmt| matches!(stmt, tree::Stmt::Simple(Stmt::Store { .. }));
    assert_eq!(
        flat.body.iter().filter(copies).count(),
        2,
        "{:#?}",
        flat.body
    );
    let shown = stated(&flat.body);
    // What is awaited is the await's own: the statement that makes the task it awaits.
    assert!(!shown.contains("GET"), "{shown}");
    assert_eq!(takings(&flat.body).len(), 1, "{:#?}", flat.body);
    assert!(takings(&flat.body)[0].contains("GET"), "{:#?}", flat.body);
    // Out of sight: the builder cleared, the states stored, and the variable given the machine's
    // address, in whose place the address itself is written.
    assert!(
        !shown.contains("ZeroBytes") && !finish_in_sight(&flat.body),
        "{shown}"
    );
    assert!(!shown.contains("Var(Local(7))"), "{shown}");
    let keeps = |stmt: &tree::Stmt| {
        matches!(
            stmt,
            tree::Stmt::Simple(Stmt::Assign {
                target: Var::Local(7),
                ..
            })
        )
    };
    assert!(!flat.body.iter().any(keeps), "{:#?}", flat.body);
    // The task is handed back where the body ends, and no source wrote that.
    assert!(handed_back(&flat.body).is_some(), "{:#?}", flat.body);
    assert_eq!(result(&flat.body), AsyncResult::Task);

    // Written back it does what the kick-off did.
    for seed in 0..exec::SEEDS {
        let (was, is) = (exec::run(&read, seed), exec::run(&back, seed));
        assert_eq!(exec::same(&was, &is), Ok(()), "seed {seed}");
    }
}

#[test]
fn a_kick_off_whose_start_does_more_than_make_the_machine_is_left() {
    let kept = "local_7 = Vm.Address(locals, 0);";
    // ⚠ A call before the machine runs would go out of sight with what makes the machine.
    let called = "Native.DEMO.USE(p1); local_7 = Vm.Address(locals, 0);";
    left_kicked(&IN_PLACE.replace(kept, called));
    // A field filled from anything but a parameter is no parameter of the method.
    let filled = "store_ref(Vm.Address(locals, 0), 0x18, p0);";
    let other = "store_ref(Vm.Address(locals, 0), 0x18, local_11);";
    left_kicked(&IN_PLACE.replace(filled, other));
    // The state must start as running, once.
    let runs = "store_i32(Vm.Address(locals, 0), 0x0, (-1));";
    left_kicked(&IN_PLACE.replace(runs, ""));
    left_kicked(&IN_PLACE.replace(runs, "store_i32(Vm.Address(locals, 0), 0x0, 0);"));
    // ⚠ The variable that holds the machine's address is written as the address: given another value later, it would not be that.
    let used = "Native.DEMO.USE(local_10);";
    let again = "Native.DEMO.USE(local_10); local_7 = Vm.Address(locals, 0);";
    left_kicked(&IN_PLACE.replace(used, again));
    // So would it not be once a clear of an earlier local has run over it.
    let cleared = "Native.DEMO.USE(local_10); zero_bytes(Vm.Address(locals, 6), 16);";
    left_kicked(&IN_PLACE.replace(used, cleared));
    // ⚠ A variable a hidden statement set may not be read where its value cannot be seen.
    let also =
        "local_6 = byte_offset(Vm.Address(locals, 0), 0x8); local_7 = Vm.Address(locals, 0);";
    let (.., machine) = kicked(&IN_PLACE.replace(kept, also));
    assert!(machine.is_some());
    let read = "Native.DEMO.USE(local_10); Native.DEMO.USE(local_6);";
    left_kicked(&IN_PLACE.replace(kept, also).replace(used, read));
    // A way out that hands back anything but the builder's task is no kick-off's.
    let task = "return load_u64(byte_offset(Vm.Address(locals, 0), 0x8), 0x8);";
    left_kicked(&IN_PLACE.replace(task, "return load_u64(Vm.Address(locals, 0), 0x18);"));
}

#[test]
fn a_kick_off_in_line_whose_task_a_call_asks_of_its_builder_is_left_as_it_is() {
    // ⚠ Nothing proves the function the builder's task getter: hidden, a call of anything else
    // would leave no trace in what is written.
    let task = "return load_u64(byte_offset(Vm.Address(locals, 0), 0x8), 0x8);";
    for other in [
        "return func_000800(byte_offset(Vm.Address(locals, 0), 0x8));",
        "return func_000800(byte_offset(Vm.Address(locals, 0), 0x8), p1);",
        "return func_000800(Vm.Address(locals, 0));",
    ] {
        left_kicked(&IN_PLACE.replace(task, other));
    }
}

/// The step function of `FRAME` with its machine proved, and the kick-off that holds `body`, each as its blocks written in their order.
fn two(body: &str) -> Vec<tree::Function> {
    let (_, f, machine) = untangled(&in_line());
    let (step, _) = put_back(f, machine.expect("the machine is proved"));
    vec![step, cfg::flat(kick_off(body))]
}

#[test]
fn a_kick_off_takes_the_body_of_the_step_function_it_calls() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let read = two(CALLS);
    let done = joined(read.clone(), &cx);
    // The step function stays: the runtime calls it again at every resumed await.
    assert_eq!(done[0], read[0]);

    let kick = &done[1];
    let shown = format!("{:?}", kick.body);
    assert!(shown.contains("Await") && shown.contains("GET"), "{shown}");
    assert!(!shown.contains("func_000100"), "{shown}");
    // The step function's parameter is the address it was handed, and its locals stand after the kick-off's.
    assert!(!shown.contains("Param(0)"), "{shown}");
    assert_eq!(kick.head.locals, 16 + read[0].head.locals);
    assert!(
        shown.contains("Var(Local(17))") && !shown.contains("Var(Local(1))"),
        "{shown}"
    );
    // The parameter copied into the machine stays in sight, and what makes the machine does not.
    let top = stated(&kick.body);
    assert!(
        top.contains("Param(1)") && !top.contains("ZeroBytes"),
        "{top}"
    );
    assert!(handed_back(&kick.body).is_some(), "{:#?}", kick.body);
    assert_eq!(result(&kick.body), AsyncResult::Task);
    // Every way out hands the task back: the body lowers, and no `return` of nothing is left in it.
    assert!(!shown.contains("Return(None)"), "{shown}");
    assert!(cfg::lower(kick.clone()).is_ok());

    // Given back, the method is the kick-off and the step function it was made of.
    let mut made = joins(&read, &cx);
    assert_eq!((made.len(), made[0].kick, made[0].step), (1, 1, 0));
    assert_eq!(made[0].whole, *kick);
    let (front, step) = made[0].parts().expect("the method goes back");
    assert_eq!((&front, &step), (&read[1], &read[0]));
    // What the method lost is lost from what it gives back.
    let last = made[0].whole.body.len() - 2;
    made[0].whole.body.remove(last);
    let (_, step) = made[0].parts().expect("the method goes back");
    assert_ne!(step, read[0]);

    // Switched off by its name, nothing is joined.
    let off = Context::new(&names, Project::empty(), Options::baseline());
    assert_eq!(joined(read.clone(), &off), read);
}

#[test]
fn a_function_that_is_no_kick_off_is_not_joined_to_what_it_calls() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let kept = "local_7 = Vm.Address(locals, 0);";
    // ⚠ A call before the step function's would go out of sight.
    let called = "Native.DEMO.USE(p1); local_7 = Vm.Address(locals, 0);";
    let read = two(&CALLS.replace(kept, called));
    assert_eq!(joined(read.clone(), &cx), read);
    // What is handed is not the machine's address.
    let read = two(&CALLS.replace("func_000100(local_7);", "func_000100(p1);"));
    assert_eq!(joined(read.clone(), &cx), read);
    // What is called is no step function: one whose machine was not proved holds no await.
    let handed = in_line().replace("Native.DEMO.USE(local_2);", "Native.DEMO.USE(p0);");
    let (_, raw, _) = untangled(&handed);
    let read = vec![cfg::flat(raw), cfg::flat(kick_off(CALLS))];
    assert_eq!(joined(read.clone(), &cx), read);
    // A function of another offset is not the one called.
    let read = two(&CALLS.replace("func_000100(local_7);", "func_000500(local_7);"));
    assert_eq!(joined(read.clone(), &cx), read);
}

#[test]
fn what_an_await_takes_its_awaiter_from_is_written_in_place_where_nothing_else_uses_it() {
    let call = |name: &str, args: Vec<Expr>| Expr::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: None,
        },
        args,
        returns: Some(1),
    };
    let task = || call("demo_task", vec![Expr::Var(Var::Param(0))]);
    let awaited = |from: Expr| {
        let taking = Stmt::Assign {
            target: Var::Local(4),
            value: call("demo_awaiter", vec![from]),
        };
        tree::Stmt::Sugar(Box::new(Sugar::Await {
            before: vec![taking],
            ready: call("demo_ready", vec![Expr::Var(Var::Local(4))]),
            suspend: Vec::new(),
            resumed: Label::Block(0),
            resume: Vec::new(),
            after: Vec::new(),
        }))
    };
    let kept = tree::Stmt::Simple(Stmt::Assign {
        target: Var::Local(3),
        value: task(),
    });
    let head = Head {
        value_types: Default::default(),
        program: 1,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 2,
        locals: 8,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    let mut body = vec![kept.clone(), awaited(Expr::Var(Var::Local(3)))];
    in_place(&mut body, &head);
    assert_eq!(body, vec![awaited(task())]);
    // A variable read again keeps what it was given.
    let again = tree::Stmt::Simple(Stmt::Eval(call("demo_use", vec![Expr::Var(Var::Local(3))])));
    let mut body = vec![kept.clone(), awaited(Expr::Var(Var::Local(3))), again];
    let held = body.clone();
    in_place(&mut body, &head);
    assert_eq!(body, held);
    // So does one whose assignment does not stand just before the await.
    let between = tree::Stmt::Simple(Stmt::Eval(call("demo_use", vec![Expr::Var(Var::Param(1))])));
    let mut body = vec![kept.clone(), between, awaited(Expr::Var(Var::Local(3)))];
    let held = body.clone();
    in_place(&mut body, &head);
    assert_eq!(body, held);
    // And one a callee may see through the address of a local below it: its assignment is the
    // memory that address reaches.
    let handed = |local: u32| {
        let address = Expr::AddrOf {
            var: Var::Local(local),
            cell: 0,
        };
        tree::Stmt::Simple(Stmt::Eval(call("demo_fill", vec![address])))
    };
    let mut body = vec![handed(1), kept.clone(), awaited(Expr::Var(Var::Local(3)))];
    let held = body.clone();
    in_place(&mut body, &head);
    assert_eq!(body, held);
    // A local proved one cell wide between the two ends that reach, and an address above reaches nothing below it.
    let mut walled = head.clone();
    let mut single = crate::gta6::script::ir::VarInfo::new(Width::Cell);
    single.single = true;
    walled.vars.insert(Var::Local(2), single);
    let mut body = vec![handed(1), kept.clone(), awaited(Expr::Var(Var::Local(3)))];
    in_place(&mut body, &walled);
    assert_eq!(body, vec![handed(1), awaited(task())]);
    let mut body = vec![handed(5), kept, awaited(Expr::Var(Var::Local(3)))];
    in_place(&mut body, &head);
    assert_eq!(body, vec![handed(5), awaited(task())]);
}

// ---- a kick-off as the passes leave it ------------------------------------------------------

/// One program of three functions: the step function of `FRAME`, a function of two parameters, and a kick-off.
const PROGRAM: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000100)]
    public static void func_000100(VmValue arg0) // 0x000100
    {
        VmValue[] parameters = new VmValue[] { arg0 };
        VmValue[] locals = new VmValue[8];
        STEP
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    [VmFunction(0x000600)]
    public static void func_000600(VmValue arg0, VmValue arg1) // 0x000600
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[0];
        START
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    [VmFunction(0x000700)]
    public static VmValue func_000700(VmValue arg0, VmValue arg1) // 0x000700
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[16];
        KICK
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Builtins
    {
        public static class Demo
        {
            public static class Task
            {
                public static VmValue Done(VmValue p0) => Vm.Builtin(7, 1, 1, p0)[0];
                public static VmValue Finished() => Vm.Builtin(7, 2, 1)[0];
                public static void Finish(VmValue p0) => Vm.Builtin(7, 3, 0, p0);
                public static void Wait(VmValue p0, VmValue p1, VmValue p2, VmValue p3, VmValue p4) => Vm.Builtin(7, 4, 0, p0, p1, p2, p3, p4);
            }
        }
    }
    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
        }
    }
}
"#;

/// A kick-off that hands its step function the address of its machine in place, as the passes leave it where no variable is left to hold it.
const HANDS: &str = "
        zero_bytes(byte_offset(Vm.Address(locals, 0), 0x8), 16);
        store_ref(Vm.Address(locals, 0), 0x18, p1);
        store_i32(Vm.Address(locals, 0), 0x0, (-1));
        func_000100(Vm.Address(locals, 0));
        return load_u64(byte_offset(Vm.Address(locals, 0), 0x8), 0x8);";

/// A start function: it calls the step function with the machine's address, its second parameter, and does nothing else.
const STARTS: &str = "
        func_000100(p1);
        return;";

/// The call of the start function a kick-off makes in the place of its call of the step function: the builder's address, then the machine's.
const START_CALL: &str =
    "func_000600(byte_offset(Vm.Address(locals, 0), 0x8), Vm.Address(locals, 0));";

/// The functions of `PROGRAM` with `start` and `kick` for bodies, each through the passes `options` asks.
fn program(start: &str, kick: &str, options: Options) -> Vec<tree::Function> {
    let text = PROGRAM
        .replace("STEP", &in_line())
        .replace("START", start)
        .replace("KICK", kick);
    let listing = Listing::open(&text).expect("the listing opens");
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), options);
    let mut done = Vec::new();
    for read in listing.functions() {
        let read = read.expect("the function reads");
        let finished = crate::gta6::script::function_in(read, &cx, &mut Stats::default());
        done.push(finished.expect("the passes run"));
    }
    done
}

/// Writes a store as the fields pass writes one through a reference of known type: a group of the one store.
fn grouped(stmt: &mut tree::Stmt) {
    if !matches!(stmt, tree::Stmt::Simple(Stmt::Store { .. })) {
        return;
    }
    let taken = std::mem::replace(stmt, tree::Stmt::Break);
    if let tree::Stmt::Simple(store) = taken {
        let owner = crate::gta6::script::ir::TypeKey(0x0123_4567_89AB_CDEF);
        let kind = tree::Group::Field { owner };
        let parts = vec![store];
        *stmt = tree::Stmt::Sugar(Box::new(Sugar::Group { kind, parts }));
    }
}

#[test]
fn a_kick_off_that_hands_its_machine_in_place_is_joined_after_the_passes() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let through = HANDS.replace("func_000100(Vm.Address(locals, 0));", START_CALL);
    for (kick, called) in [(HANDS.to_owned(), "func_000100"), (through, "func_000600")] {
        let done = program(STARTS, &kick, options);
        assert_eq!(awaits(&done[0].body), 1, "{:#?}", done[0].body);
        let made = joins(&done, &cx);
        assert_eq!(made.len(), 1, "not joined:\n{kick}");
        assert_eq!((made[0].kick, made[0].step), (2, 0));
        let shown = format!("{:?}", made[0].whole.body);
        assert!(shown.contains("Await") && shown.contains("GET"), "{shown}");
        assert!(!shown.contains(called), "{shown}");
        assert!(handed_back(&made[0].whole.body).is_some(), "{shown}");
        // Given back, the two are the functions as the passes left them, which the corpus check holds them to.
        let (front, step) = made[0].parts().expect("the method goes back");
        assert_eq!(step, done[0]);
        assert_eq!(front, done[2]);
        assert_eq!(joined(done.clone(), &cx)[2], made[0].whole);
    }
}

#[test]
fn a_store_made_one_of_a_type_is_read_as_the_store_it_holds() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let mut done = program(STARTS, HANDS, options);
    // The kick-off's stores, and the step function's store of the running state where its await resumes.
    done[2].body.iter_mut().for_each(grouped);
    for stmt in &mut done[0].body {
        if let tree::Stmt::Sugar(sugar) = stmt {
            if let Sugar::Await { resume, .. } = &mut **sugar {
                resume.iter_mut().for_each(grouped);
            }
        }
    }
    let grouped_in = |body: &[tree::Stmt]| format!("{body:?}").contains("Field");
    assert!(grouped_in(&done[0].body) && grouped_in(&done[2].body));
    let made = joins(&done, &cx);
    assert_eq!(made.len(), 1, "{:#?}", done[2].body);
    let (front, step) = made[0].parts().expect("the method goes back");
    assert_eq!(front, done[2]);
    assert_eq!(step, done[0]);
}

#[test]
fn a_start_function_that_does_more_than_call_the_step_function_is_not_joined() {
    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let kick = HANDS.replace("func_000100(Vm.Address(locals, 0));", START_CALL);
    assert_eq!(joins(&program(STARTS, &kick, options), &cx).len(), 1);
    // ⚠ What it does besides would go out of sight with the call.
    let more = "Native.DEMO.USE(p0); func_000100(p1); return;";
    assert!(joins(&program(more, &kick, options), &cx).is_empty());
    // It hands the step function its first parameter, the builder's address, and not the machine's.
    let first = "func_000100(p0); return;";
    assert!(joins(&program(first, &kick, options), &cx).is_empty());
    // Given first anything but the builder's address, it is no start function of the machine.
    let other = kick.replace(START_CALL, "func_000600(p1, Vm.Address(locals, 0));");
    assert!(joins(&program(STARTS, &other, options), &cx).is_empty());
    // ⚠ The machine's local read as a value is not reached through its address alone.
    let read = HANDS.replace(
        "store_ref(Vm.Address(locals, 0), 0x18, p1);",
        "store_ref(Vm.Address(locals, 0), 0x18, p1);
        store_ref(Vm.Address(locals, 0), 0x20, local_0);",
    );
    assert!(joins(&program(STARTS, &read, options), &cx).is_empty());
    // Switched off by its name, nothing is joined.
    let off = Context::new(&names, Project::empty(), Options::baseline());
    assert!(joins(&program(STARTS, &kick, options), &off).is_empty());
}

// ---- an await of the task made before it ---------------------------------------------------------

#[test]
fn an_await_takes_the_statement_that_makes_its_task_and_runs_as_it_did() {
    // The suspend path keeps the awaiter in the machine and hands it, or its address, to the
    // runtime: those are the compiler's own uses of it.
    for body in [in_line(), [HEAD, THROUGH, REST].concat()] {
        let (read, flat, back) = rebuilt(&body);
        assert!(!stated(&flat.body).contains("GET"), "{:#?}", flat.body);
        let taken = takings(&flat.body);
        assert_eq!(taken.len(), 1, "{taken:?}");
        assert!(taken[0].contains("GET"), "{taken:?}");
        assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
    }
    // ⚠ A task the method reads again after the await is a variable of the method: it stays where it is made.
    let end = "store_i32(p0, 0x0, (-2));";
    let again = in_line().replace(end, "Native.DEMO.USE(local_2); store_i32(p0, 0x0, (-2));");
    let (read, flat, back) = rebuilt(&again);
    assert!(stated(&flat.body).contains("GET"), "{:#?}", flat.body);
    assert_eq!(takings(&flat.body), ["[]"]);
    assert!(same(&read.cfg, &back.cfg), "{:#?}", back.cfg);
}

// ---- a machine read into its kick-off is not written --------------------------------------------

#[test]
fn a_state_machine_whose_step_function_is_read_into_its_kick_off_is_not_written() {
    use crate::gta6::script::class::{Class, ClassKind, Method, Role, Unit};
    use crate::gta6::script::ir::TypeKey;

    let names = Names::default();
    let options = Options::baseline().with(Pass::Await);
    let cx = Context::new(&names, Project::empty(), options);
    let done = program(STARTS, HANDS, options);
    let (functions, steps) = joined_steps(done.clone(), &cx);
    assert_eq!(steps, BTreeSet::from([0x100]));
    assert_eq!(functions, joined(done.clone(), &cx));

    let method = |function: tree::Function, role: Role| Method {
        function,
        role,
        is_static: false,
        is_async: false,
        slot: Some(1),
        overrides: false,
    };
    // What the compiler writes beside the step function: the builder handed the machine.
    let mut handing = done[1].clone();
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
    let machine = |kind: ClassKind, other: tree::Function| Class {
        key: TypeKey(0x0123_4567_89AB_CDEF),
        kind,
        base: None,
        interfaces: Vec::new(),
        fields: Vec::new(),
        methods: vec![
            method(functions[0].clone(), Role::Step),
            method(other, Role::Method),
        ],
        nested: Vec::new(),
    };
    let unit = |classes: Vec<Class>, free: Vec<tree::Function>| {
        let mut unit = Unit::flat(1, Vec::new(), free);
        unit.classes = classes;
        unit
    };
    let kick = || vec![functions[2].clone()];

    let mut written = unit(
        vec![machine(ClassKind::StateMachine, handing.clone())],
        kick(),
    );
    assert_eq!(hide_machines(&mut written, &steps), 1);
    assert!(written.classes.is_empty());
    assert_eq!(written.functions.len(), 1);
    // Nested in the class whose method it is, it goes from there.
    let mut outer = machine(ClassKind::Class, handing.clone());
    outer.methods.clear();
    outer
        .nested
        .push(machine(ClassKind::StateMachine, handing.clone()));
    let mut written = unit(vec![outer], kick());
    assert_eq!(hide_machines(&mut written, &steps), 1);
    assert_eq!(written.classes.len(), 1);
    assert!(written.classes[0].nested.is_empty());

    // Kept: where a function of the program names the step function, here one that does more than start it.
    let mut free = kick();
    let mut calling = functions[1].clone();
    calling
        .body
        .insert(0, tree::Stmt::Simple(Stmt::Eval(Expr::Var(Var::Param(0)))));
    free.push(calling);
    let held = unit(
        vec![machine(ClassKind::StateMachine, handing.clone())],
        free,
    );
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
    // Where no step function was read into its kick-off.
    let held = unit(
        vec![machine(ClassKind::StateMachine, handing.clone())],
        kick(),
    );
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &BTreeSet::new()), 0);
    assert_eq!(written, held);
    // ⚠ Where another method holds more than the compiler writes: it would be written nowhere.
    let mut more = handing.clone();
    more.body
        .insert(0, tree::Stmt::Simple(Stmt::Eval(Expr::Var(Var::Param(1)))));
    let held = unit(vec![machine(ClassKind::StateMachine, more)], kick());
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
    // ⚠ Where a class is nested in the machine: a lambda's body would be written nowhere.
    let mut holding = machine(ClassKind::StateMachine, handing.clone());
    holding
        .nested
        .push(machine(ClassKind::Closure, handing.clone()));
    let held = unit(vec![holding], kick());
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
    // ⚠ Where a function written outside the machine names its type: it would be declared nowhere.
    let mut naming = kick();
    let typed = crate::gta6::script::ir::VarInfo {
        width: crate::gta6::script::ir::Width::Cell,
        kind: crate::gta6::script::ir::Kind::Object(TypeKey(0x0123_4567_89AB_CDEF).to_string()),
        name: None,
        single: false,
    };
    naming[0].head.vars.insert(Var::Local(0), typed);
    naming[0]
        .body
        .insert(0, tree::Stmt::Simple(Stmt::Eval(Expr::Var(Var::Local(0)))));
    let held = unit(
        vec![machine(ClassKind::StateMachine, handing.clone())],
        naming,
    );
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
    // Where the class is no state machine.
    let held = unit(vec![machine(ClassKind::Struct, handing)], kick());
    let mut written = held.clone();
    assert_eq!(hide_machines(&mut written, &steps), 0);
    assert_eq!(written, held);
}

#[test]
fn without_the_await_name_no_statement_moves_into_an_await() {
    let (_, f, machine) = untangled(&in_line());
    let names = Names::default();
    let options = Options::baseline().with(Pass::Iterators);
    let cx = Context::new(&names, Project::empty(), options);
    let mut flat = cfg::flat(f);
    run(
        &mut flat,
        machine.expect("the machine is proved"),
        &cx,
        &mut Stats::default(),
    );
    assert!(stated(&flat.body).contains("GET"), "{:#?}", flat.body);
    assert_eq!(takings(&flat.body), ["[]"]);
}

#[path = "async_await_hide_tests.rs"]
mod hide;
