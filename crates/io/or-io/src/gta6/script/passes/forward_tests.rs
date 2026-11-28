use super::*;
use crate::gta6::script::ir::Callee;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::{Names, Options, Project};

/// A listing of one function of two parameters and eight locals, written by hand, with natives to call.
const FRAME: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static void func_000010(VmValue arg0, VmValue arg1) // 0x000010
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[8];
        VmValue v_000010_0 = default; VmValue v_000018_1 = default;
        BODY
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
            public static void USE2(VmValue p0, VmValue p1) => Vm.Native(0x0000000000000004UL, 0x00000004U, 0, p0, p1);
        }
    }
}
"#;

/// A test of the second parameter, for a branch nothing here decides.
const TEST: &str = "if (((as_u32(p1) == 0)))";

/// A new object.
const NEW: &str = "Vm.NewObject(new VmType(\"T\"))";

/// The blocks of the function that holds `body`.
fn read(body: &str) -> cfg::Function {
    let text = FRAME.replace("BODY", body);
    let listing = Listing::open(&text).expect("the listing opens");
    let f = listing
        .functions()
        .next()
        .expect("the listing holds a function")
        .expect("the function reads");
    cfg::lower(f).expect("the function lowers")
}

/// What this pass alone, with `passes` on, makes of `f`.
fn ran(passes: &[Pass], mut f: cfg::Function) -> cfg::Function {
    let options = (passes.iter()).fold(Options::baseline(), |options, pass| options.with(*pass));
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), options);
    run(&mut f, &cx, &mut Stats::default());
    f
}

/// Asserts that this pass alone, with `passes` on, makes of `before` the blocks `after` reads as.
fn becomes(passes: &[Pass], before: &str, after: &str) {
    let f = ran(passes, read(before));
    assert_eq!(f.cfg, read(after).cfg, "from:\n{before}");
}

fn stays(passes: &[Pass], body: &str) {
    becomes(passes, body, body);
}

/// A constant stored at `p0 + 0x10`, `between`, and the four bytes there read back into a local.
fn back(between: &str, read: &str) -> String {
    format!("store_i32(p0, 0x10, 5); {between} local_1 = {read}; Native.DEMO.USE(local_1); return;")
}

#[test]
fn a_load_of_a_constant_just_stored_is_the_constant() {
    becomes(
        &[Pass::Forward],
        &back("", "load_i32(p0, 0x10)"),
        &back("", "as_i32(5)"),
    );
    // Read as the load reads it: one byte of the four is no load of what was stored.
    stays(&[Pass::Forward], &back("", "load_u8(p0, 0x10)"));
    // The name switched off, the load stays.
    stays(&[Pass::DeadLoads], &back("", "load_i32(p0, 0x10)"));
}

#[test]
fn a_store_is_not_forwarded_past_what_may_write_the_field() {
    // A call may write it.
    stays(
        &[Pass::Forward, Pass::Escape],
        &back("Native.DEMO.USE(p1);", "load_i32(p0, 0x10)"),
    );
    // A store through another pointer may be a store to it.
    stays(
        &[Pass::Forward, Pass::Escape],
        &back("store_i32(p1, 0x10, 7);", "load_i32(p0, 0x10)"),
    );
    // The pointer is another pointer after an assignment.
    stays(
        &[Pass::Forward, Pass::Escape],
        "local_2 = p0; store_i32(local_2, 0x10, 5); local_2 = p1; local_1 = load_i32(local_2, 0x10); Native.DEMO.USE(local_1); return;",
    );
}

#[test]
fn a_store_to_other_bytes_through_the_same_pointer_leaves_the_field() {
    becomes(
        &[Pass::Forward, Pass::Escape],
        &back("store_i32(p0, 0x14, 7);", "load_i32(p0, 0x10)"),
        &back("store_i32(p0, 0x14, 7);", "as_i32(5)"),
    );
    // Eight bytes from 0xC cover the field.
    stays(
        &[Pass::Forward, Pass::Escape],
        &back("store_u64(p0, 0xC, p1);", "load_i32(p0, 0x10)"),
    );
    // Telling two stores apart is `escape`'s: without it any store ends the field.
    stays(
        &[Pass::Forward],
        &back("store_i32(p0, 0x14, 7);", "load_i32(p0, 0x10)"),
    );
}

#[test]
fn a_field_of_an_object_in_no_other_hands_outlasts_a_store_through_a_pointer() {
    let made = |between: &str, read: &str| {
        format!(
            "local_2 = {NEW}; store_i32(local_2, 0x10, 5); {between} local_1 = {read}; Native.DEMO.USE2(local_1, local_2); return;"
        )
    };
    becomes(
        &[Pass::Forward, Pass::Escape],
        &made("store_i32(p1, 0x10, 7);", "load_i32(local_2, 0x10)"),
        &made("store_i32(p1, 0x10, 7);", "as_i32(5)"),
    );
    // Stored where anything may find it, the object is in other hands from there on.
    stays(
        &[Pass::Forward, Pass::Escape],
        &made(
            "store_ref(p1, 0x8, local_2); store_i32(load_u64(p1, 0x8), 0x10, 7);",
            "load_i32(local_2, 0x10)",
        ),
    );
    stays(
        &[Pass::Forward],
        &made("store_i32(p1, 0x10, 7);", "load_i32(local_2, 0x10)"),
    );
}

#[test]
fn a_field_holds_its_constant_at_a_join_where_every_path_left_it() {
    let arms = |els: &str, read: &str| {
        format!(
            "store_i32(p0, 0x10, 5); {TEST} {{ local_2 = 1; }} else {{ {els} }} local_1 = {read}; Native.DEMO.USE2(local_1, local_2); return;"
        )
    };
    becomes(
        &[Pass::Forward],
        &arms("local_2 = 2;", "load_i32(p0, 0x10)"),
        &arms("local_2 = 2;", "as_i32(5)"),
    );
    stays(
        &[Pass::Forward],
        &arms("Native.DEMO.USE(p1);", "load_i32(p0, 0x10)"),
    );
}

#[test]
fn a_call_in_the_test_between_a_store_and_its_load_ends_the_field() {
    let tested = |test: &str, read: &str| {
        format!(
            "store_i32(p0, 0x10, 5); if ((({test} == 0))) {{ local_1 = {read}; Native.DEMO.USE(local_1); }} return;"
        )
    };
    // The callee may write the field before the arm reads it.
    stays(
        &[Pass::Forward, Pass::Escape],
        &tested("as_u32(Native.DEMO.GET(p1))", "load_i32(p0, 0x10)"),
    );
    // A test that calls nothing leaves it.
    becomes(
        &[Pass::Forward, Pass::Escape],
        &tested("as_u32(p1)", "load_i32(p0, 0x10)"),
        &tested("as_u32(p1)", "as_i32(5)"),
    );
}

#[test]
fn a_load_nothing_reads_goes_where_its_address_was_gone_through_before() {
    becomes(
        &[Pass::DeadLoads],
        "store_i32(p0, 0x10, 5); v_000010_0 = load_i32(p0, 0x8); return;",
        "store_i32(p0, 0x10, 5); return;",
    );
    // The first to go through the pointer may be the one that faults.
    stays(
        &[Pass::DeadLoads],
        "v_000010_0 = load_i32(p0, 0x8); store_i32(p0, 0x10, 5); return;",
    );
    stays(
        &[Pass::Forward],
        "store_i32(p0, 0x10, 5); v_000010_0 = load_i32(p0, 0x8); return;",
    );
    // The pointer was assigned since: what went through it before went elsewhere.
    stays(
        &[Pass::DeadLoads],
        "store_i32(local_2, 0x10, 5); local_2 = Native.DEMO.GET(p0); v_000010_0 = load_i32(local_2, 0x8); return;",
    );
}

#[test]
fn a_dead_load_goes_below_a_block_that_went_through_its_address() {
    let arms = |then: &str| {
        format!(
            "{TEST} {{ {then} Native.DEMO.USE(p1); }} else {{ Native.DEMO.USE(p0); }} v_000010_0 = load_i32(p0, 0x8); return;"
        )
    };
    // One arm went through `p0`, the other did not.
    stays(&[Pass::DeadLoads], &arms("store_i32(p0, 0x10, 5);"));
    becomes(
        &[Pass::DeadLoads],
        &format!("store_i32(p0, 0x10, 5); {}", arms("")),
        &format!(
            "store_i32(p0, 0x10, 5); {TEST} {{  Native.DEMO.USE(p1); }} else {{ Native.DEMO.USE(p0); }} return;"
        ),
    );
}

#[test]
fn a_load_in_an_arm_that_may_not_run_proves_nothing() {
    stays(
        &[Pass::DeadLoads],
        "local_1 = (((((as_i32(p1) == as_i32(5))) != Truth(0)) ? (VmValue)(load_i32(p0, 0x10)) : (VmValue)(7))); v_000010_0 = load_i32(p0, 0x8); Native.DEMO.USE(local_1); return;",
    );
}

#[test]
fn a_dead_load_from_an_object_the_function_made_goes() {
    becomes(
        &[Pass::DeadLoads],
        &format!("local_2 = {NEW}; v_000010_0 = load_i32(local_2, 0x8); Native.DEMO.USE(local_2); return;"),
        &format!("local_2 = {NEW}; Native.DEMO.USE(local_2); return;"),
    );
}

#[test]
fn a_dead_load_through_an_address_just_loaded_and_gone_through_goes() {
    let chain = |between: &str, dead: &str| {
        format!(
            "local_1 = load_i32(load_u64(p0, 0x8), 0x10); {between} {dead} Native.DEMO.USE(local_1); return;"
        )
    };
    let dead = "v_000010_0 = load_i32(load_u64(p0, 0x8), 0x14);";
    becomes(&[Pass::DeadLoads], &chain("", dead), &chain("", ""));
    // A store between may have changed the address the field holds.
    stays(&[Pass::DeadLoads], &chain("store_u64(p1, 0x0, p0);", dead));
}

/// A temporary of several cells, declared, zeroed as one cell and filled by `fill`.
fn cell(fill: &str) -> String {
    format!("VmValues v_000020_2 = null!;\n v_000020_2 = zero_slots(1); {fill}")
}

/// A store of `value` to the first four bytes of that temporary, through its own address.
fn filled_with(value: &str) -> String {
    format!("store_i32(Vm.Address(v_000020_2, 0), 0x0, {value});")
}

/// Two arms that each read `read`, and the join after them.
fn arms(read: &str) -> String {
    format!(
        "{TEST} {{ Native.DEMO.USE({read}); }} else {{ Native.DEMO.USE2(p0, {read}); }} return;"
    )
}

#[test]
fn a_cell_zeroed_and_filled_is_the_value_cut_wherever_the_filling_block_leads() {
    // A constant goes to each read, in whichever block.
    becomes(
        &[Pass::Forward],
        &format!("{} {}", cell(&filled_with("7")), arms("v_000020_2[0]")),
        &arms("as_u32(7)"),
    );
    // Any other value stays in the temporary, which is then one cell.
    becomes(
        &[Pass::Forward],
        &format!("{} {}", cell(&filled_with("p0")), arms("v_000020_2[0]")),
        &format!(
            "VmValues v_000020_2 = null!;\n v_000020_2 = as_u32(p0); {}",
            arms("v_000020_2")
        ),
    );
    // The name switched off, the cell stays memory.
    stays(
        &[Pass::DeadLoads],
        &format!("{} {}", cell(&filled_with("7")), arms("v_000020_2[0]")),
    );
}

#[test]
fn a_constant_stored_before_the_store_that_fills_a_cell_leaves_nothing() {
    let twice = format!(
        "{} {}",
        filled_with("0"),
        filled_with("Native.DEMO.GET(p0)")
    );
    becomes(
        &[Pass::Forward],
        &format!("{} Native.DEMO.USE(v_000020_2[0]); return;", cell(&twice)),
        "VmValues v_000020_2 = null!;\n v_000020_2 = as_u32(Native.DEMO.GET(p0)); Native.DEMO.USE(v_000020_2); return;",
    );
    // A value that calls is not written over: its call would go with it.
    let calls = format!(
        "{} {}",
        filled_with("Native.DEMO.GET(p0)"),
        filled_with("7")
    );
    stays(
        &[Pass::Forward],
        &format!("{} Native.DEMO.USE(v_000020_2[0]); return;", cell(&calls)),
    );
}

#[test]
fn a_cell_stays_memory_where_a_read_may_run_before_its_store_or_its_address_goes_elsewhere() {
    // Read before it is filled, it is still zeroes there.
    stays(
        &[Pass::Forward],
        &format!(
            "{} return;",
            cell(&format!(
                "Native.DEMO.USE(v_000020_2[0]); {} Native.DEMO.USE(v_000020_2[0]);",
                filled_with("7")
            ))
        ),
    );
    // The block that fills it is one of two ways to the read.
    stays(
        &[Pass::Forward],
        &format!(
            "VmValues v_000020_2 = null!;\n {TEST} {{ v_000020_2 = zero_slots(1); {} }} else {{ }} Native.DEMO.USE(v_000020_2[0]); return;",
            filled_with("7")
        ),
    );
    // An address handed on is memory a callee may fill.
    stays(
        &[Pass::Forward],
        &format!(
            "{} Native.DEMO.USE2(Vm.Address(v_000020_2, 0), v_000020_2[0]); return;",
            cell(&filled_with("7"))
        ),
    );
}

#[test]
fn a_value_read_once_where_two_arms_meet_moves_there_under_forward() {
    let arms = |first: &str, read: &str| {
        format!(
            "{first} {TEST} {{ local_2 = 1; }} else {{ local_2 = 2; }} Native.DEMO.USE2({read}, local_2); return;"
        )
    };
    let before = arms("v_000010_0 = Native.DEMO.GET(p0);", "v_000010_0");
    becomes(&[Pass::Forward], &before, &arms("", "Native.DEMO.GET(p0)"));
    // The name switched off, the value stays where it was made.
    stays(&[Pass::DeadLoads], &before);
}

/// `before`, then `clear`, then `local_3` read whole.
fn cleared(before: &str, clear: &str) -> String {
    format!("{before} {clear} Native.DEMO.USE(local_3); return;")
}

/// A clear of the low four bytes of `local_3`.
const CLEAR: &str = "zero_bytes(Vm.Address(locals, 3), 4);";

#[test]
fn a_clear_of_the_low_bytes_of_a_local_whose_upper_bytes_nothing_writes_is_zero() {
    becomes(
        &[Pass::Forward],
        &cleared("", CLEAR),
        &cleared("", "local_3 = 0;"),
    );
    // A store of the low four bytes leaves the upper ones as they were: zero.
    let low = "store_i32(Vm.Address(locals, 3), 0x0, p0);";
    becomes(
        &[Pass::Forward],
        &cleared(low, CLEAR),
        &cleared(low, "local_3 = 0;"),
    );
    // The name switched off, the clear stays.
    stays(&[Pass::DeadLoads], &cleared("", CLEAR));
}

#[test]
fn a_clear_of_the_low_bytes_stays_where_the_upper_bytes_may_hold_something() {
    for before in [
        // Assigned whole.
        "local_3 = p0;",
        // Stored above the fourth byte.
        "store_i32(Vm.Address(locals, 3), 0x4, p0);",
        // Stored through the address of the local below it.
        "store_u64(Vm.Address(locals, 2), 0x8, p0);",
        // Handed to a callee, which may write any of it.
        "Native.DEMO.USE(Vm.Address(locals, 3));",
    ] {
        stays(&[Pass::Forward], &cleared(before, CLEAR));
    }
}

/// A load of the four bytes at `p0 + 0x8`, tested for zero.
const LOADED: &str = "((as_u32(load_i32(p0, 0x8)) == 0))";

/// Whether this pass alone, with `passes` on, sends the first block of `body` straight on.
fn decided(passes: &[Pass], body: &str) -> bool {
    let f = ran(passes, read(body));
    matches!(f.cfg.blocks[0].term, Terminator::Jump(_))
}

#[test]
fn a_branch_whose_arms_meet_goes_straight_on_where_its_test_cannot_fault() {
    let gone_through = format!("store_i32(p0, 0x10, 5); if ({LOADED}) {{ }} else {{ }} return;");
    assert!(decided(&[Pass::DeadLoads], &gone_through));
    // The name switched off, the test stays.
    stays(&[Pass::Forward], &gone_through);
    for body in [
        // The test may be the first to go through the pointer.
        format!("if ({LOADED}) {{ }} else {{ }} return;"),
        // A call is made whichever way the test goes.
        "store_i32(p0, 0x10, 5); if (((as_u32(Native.DEMO.GET(p0)) == 0))) { } else { } return;"
            .to_owned(),
        // The arms do not meet.
        format!(
            "store_i32(p0, 0x10, 5); if ({LOADED}) {{ Native.DEMO.USE(p1); }} else {{ }} return;"
        ),
    ] {
        stays(&[Pass::DeadLoads], &body);
    }
}

/// An element of the array at `p0 + 0x10`, at `p1`.
const ELEMENT: &str = r#"Vm.Intrinsic("array_get_u64", 1, load_u64(p0, 0x10), p1)[0]"#;

/// The blocks of `body` with the first element `body` reads also read before it, for nothing.
fn read_before(body: &str) -> cfg::Function {
    let mut f = read(body);
    let mut found = None;
    for stmt in &f.cfg.blocks[0].stmts {
        stmt.each_expr(&mut |top| {
            top.walk(&mut |e| {
                let element = matches!(
                    e,
                    Expr::Call {
                        callee: Callee::Intrinsic { .. },
                        ..
                    }
                );
                if element && found.is_none() {
                    found = Some(e.clone());
                }
            });
        });
    }
    let element = found.expect("the body reads an element");
    f.cfg.blocks[0].stmts.insert(0, Stmt::Eval(element));
    f
}

#[test]
fn a_read_kept_for_nothing_that_the_next_statement_makes_first_goes() {
    let next = format!("Native.DEMO.USE2(p0, {ELEMENT}); return;");
    let f = ran(&[Pass::DeadLoads], read_before(&next));
    assert_eq!(f.cfg, read(&next).cfg);
    // The name switched off, the read stays.
    let f = ran(&[Pass::Forward], read_before(&next));
    assert_eq!(f.cfg, read_before(&next).cfg);
}

#[test]
fn a_read_kept_for_nothing_stays_where_the_next_statement_may_fault_or_act_first() {
    for next in [
        // A call comes before the element.
        format!("Native.DEMO.USE2(Native.DEMO.GET(p1), {ELEMENT}); return;"),
        // The statement after is another, which may change what the element is.
        format!("store_u64(p0, 0x10, p1); Native.DEMO.USE({ELEMENT}); return;"),
        // In an arm that may not run.
        format!(
            "local_1 = (((((as_i32(p1) == as_i32(5))) != Truth(0)) ? (VmValue)({ELEMENT}) : (VmValue)(7))); Native.DEMO.USE(local_1); return;"
        ),
    ] {
        let f = ran(&[Pass::DeadLoads], read_before(&next));
        assert_eq!(f.cfg, read_before(&next).cfg, "{next}");
    }
}

/// A temporary of two cells, declared and given zero cells, then `rest`.
fn zero_pair(rest: &str) -> String {
    format!("VmValues v_000020_2 = null!;\n v_000020_2 = zero_slots(2); {rest}")
}

/// A clear of all sixteen bytes of that temporary through its own address.
const CLEAR_PAIR: &str = "zero_bytes(Vm.Address(v_000020_2, 0), 16);";

/// The temporary stored whole at `p0 + 0x10`.
const STORE_PAIR: &str = "store_bytes(p0, 0x10, 16, v_000020_2); return;";

#[test]
fn a_temporary_given_zeroes_and_only_cleared_through_its_address_reads_as_its_zeroes() {
    becomes(
        &[Pass::Forward],
        &zero_pair(&format!("{CLEAR_PAIR} {STORE_PAIR}")),
        "store_bytes(p0, 0x10, 16, zero_slots(2)); return;",
    );
    // A cell of it read alone is a zero cell.
    becomes(
        &[Pass::Forward],
        &format!(
            "{} Native.DEMO.USE2(p0, v_000020_2[1]); return;",
            zero_pair(CLEAR_PAIR)
        ),
        "Native.DEMO.USE2(p0, 0); return;",
    );
    // Of one cell, its cell is zero.
    becomes(
        &[Pass::Forward],
        &format!(
            "{} Native.DEMO.USE(v_000020_2[0]); return;",
            cell("zero_bytes(Vm.Address(v_000020_2, 0), 8);")
        ),
        "Native.DEMO.USE(0); return;",
    );
    // The name switched off, the clear and the temporary stay.
    stays(
        &[Pass::DeadLoads],
        &zero_pair(&format!("{CLEAR_PAIR} {STORE_PAIR}")),
    );
}

#[test]
fn a_temporary_given_zeroes_stays_where_something_may_write_it_otherwise_or_a_read_comes_first() {
    for body in [
        // A store through its address writes what is no zero.
        zero_pair(&format!(
            "store_i32(Vm.Address(v_000020_2, 0), 0x0, 5); {STORE_PAIR}"
        )),
        // A clear that runs past its cells writes what is not its own.
        zero_pair(&format!(
            "zero_bytes(Vm.Address(v_000020_2, 0), 24); {STORE_PAIR}"
        )),
        // An address handed on is memory a callee may write.
        zero_pair(&format!(
            "{CLEAR_PAIR} Native.DEMO.USE(Vm.Address(v_000020_2, 0)); {STORE_PAIR}"
        )),
        // On one way of two the read comes with no assignment before it.
        format!(
            "VmValues v_000020_2 = null!;\n {TEST} {{ v_000020_2 = zero_slots(2); {CLEAR_PAIR} }} else {{ }} {STORE_PAIR}"
        ),
    ] {
        stays(&[Pass::Forward], &body);
    }
}

/// [`CLEAR`], `local_3` copied, `between`, then `second`, and the copy and `local_3` read.
fn cleared_twice(between: &str, second: &str) -> String {
    format!(
        "{CLEAR} v_000010_0 = local_3; {between} {second} Native.DEMO.USE2(v_000010_0, local_3); return;"
    )
}

/// Whether [`cleared_again`] changes `body`, and the blocks it leaves.
fn again(body: &str) -> (bool, cfg::Cfg) {
    let mut f = read(body);
    let changed = cleared_again(&mut f);
    (changed, f.cfg)
}

#[test]
fn a_clear_of_bytes_a_clear_before_it_left_zero_goes() {
    let once = read(&cleared_twice("", "")).cfg;
    assert_eq!(again(&cleared_twice("", CLEAR)), (true, once.clone()));
    // Fewer of the bytes cleared are zero too.
    let fewer = "zero_bytes(Vm.Address(locals, 3), 2);";
    assert_eq!(again(&cleared_twice("", fewer)), (true, once));
    // The name switched off, both clears stay.
    stays(&[Pass::DeadLoads], &cleared_twice("", CLEAR));
}

#[test]
fn a_clear_stays_where_its_bytes_may_have_been_written_since_or_it_reaches_further() {
    for body in [
        // Written by its name.
        cleared_twice("local_3 = p0;", CLEAR),
        // Written through its own address.
        cleared_twice("store_u8(Vm.Address(locals, 3), 0x1, 7);", CLEAR),
        // Handed to a callee, which may write it.
        cleared_twice("Native.DEMO.USE(Vm.Address(locals, 3));", CLEAR),
        // Bytes the first clear did not cover.
        cleared_twice("", "zero_bytes(Vm.Address(locals, 3), 8);"),
        // The bytes of another variable.
        cleared_twice("", "zero_bytes(Vm.Address(locals, 4), 4);"),
    ] {
        assert_eq!(again(&body), (false, read(&body).cfg), "{body}");
    }
}
