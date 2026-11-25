use super::*;
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

/// Asserts that this pass alone, with `passes` on, makes of `before` the blocks `after` reads as.
fn becomes(passes: &[Pass], before: &str, after: &str) {
    let mut f = read(before);
    let options = (passes.iter()).fold(Options::baseline(), |options, pass| options.with(*pass));
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), options);
    run(&mut f, &cx, &mut Stats::default());
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
