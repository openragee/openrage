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
            format!("{stated:?}"),
        )
    };
    // After `values` and `types` the machine is still proved, and `structure` keeps each piece.
    let (awaited, gotos, stated) = written(Options::baseline().with(Pass::Await));
    assert_eq!((awaited, gotos), (1, 0), "{stated}");
    assert!(stated.contains("GET") && stated.contains("USE"), "{stated}");
    for hidden in ["Wait", "Finish", "box_at"] {
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
