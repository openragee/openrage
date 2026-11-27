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
        VmValue v_000010_0 = default;
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

#[test]
fn a_variable_every_read_of_which_sees_one_constant_is_the_constant() {
    // `local_1` is two variables in one cell: the first only ever holds 5.
    let two = |first: &str, read: &str| {
        format!(
            "{first} {TEST} {{ Native.DEMO.USE({read}); }} else {{ }} Native.DEMO.USE2({read}, p0); local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;"
        )
    };
    becomes(
        &[Pass::Constants],
        &two("local_1 = 5;", "local_1"),
        &two("", "5"),
    );
    // The name switched off, nothing is carried.
    stays(&[Pass::Coalesce], &two("local_1 = 5;", "local_1"));
}

#[test]
fn the_same_constant_on_every_path_is_the_constant() {
    let arms = |read: &str, then: &str, els: &str| {
        format!(
            "{TEST} {{ {then} Native.DEMO.USE(p0); }} else {{ {els} Native.DEMO.USE(p1); }} Native.DEMO.USE({read}); return;"
        )
    };
    becomes(
        &[Pass::Constants],
        &arms("local_1", "local_1 = 2;", "local_1 = 2;"),
        &arms("2", "", ""),
    );
    // Two constants are a variable.
    stays(
        &[Pass::Constants],
        &arms("local_1", "local_1 = 2;", "local_1 = 3;"),
    );
    // On the path round the assignment the local still holds what it had at the start.
    stays(&[Pass::Constants], &arms("local_1", "local_1 = 2;", ""));
}

#[test]
fn a_variable_a_loop_steps_is_no_constant() {
    stays(
        &[Pass::Constants, Pass::Coalesce],
        "local_1 = 0; while (Truth(true)) { if (((as_i32(local_1) >= as_i32(p0)))) { break; } else { } Native.DEMO.USE(local_1); local_1 = ((as_i32(local_1) + as_i32(1))); } return;",
    );
}

#[test]
fn a_local_an_address_may_reach_is_not_followed() {
    // A callee handed the address of `local_2` may write `local_3`.
    stays(
        &[Pass::Constants, Pass::Coalesce],
        "local_3 = 5; Native.DEMO.USE(Vm.Address(locals, 2)); Native.DEMO.USE(local_3); local_3 = 6; return;",
    );
}

#[test]
fn a_variable_that_only_copies_another_is_that_other() {
    // The source is assigned again, after the last read of the copy.
    let body = |copy: &str, read: &str| {
        format!(
            "local_1 = Native.DEMO.GET(p0); {copy} {TEST} {{ Native.DEMO.USE({read}); }} else {{ }} Native.DEMO.USE2({read}, p0); local_1 = Native.DEMO.GET(p1); Native.DEMO.USE(local_1); return;"
        )
    };
    becomes(
        &[Pass::Coalesce],
        &body("local_2 = local_1;", "local_2"),
        &body("", "local_1"),
    );
    stays(&[Pass::Constants], &body("local_2 = local_1;", "local_2"));
}

#[test]
fn a_copy_read_after_its_source_changed_is_a_variable() {
    stays(
        &[Pass::Coalesce],
        "local_1 = Native.DEMO.GET(p0); local_2 = local_1; local_1 = Native.DEMO.GET(p1); Native.DEMO.USE2(local_2, local_1); return;",
    );
    // Round a loop the copy lags its source by one turn, though one assignment of the source
    // reaches both the copy and the read.
    stays(
        &[Pass::Coalesce],
        "local_1 = Native.DEMO.GET(p0); local_2 = local_1; while (Truth(true)) { if (((as_u32(Native.DEMO.GET(local_2)) == 0))) { break; } else { } local_2 = local_1; local_1 = Native.DEMO.GET(p1); } return;",
    );
}

#[test]
fn a_chain_of_copies_is_its_first_link() {
    becomes(
        &[Pass::Coalesce],
        "local_1 = Native.DEMO.GET(p0); local_2 = local_1; local_3 = local_2; Native.DEMO.USE2(local_3, local_3); return;",
        "local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;",
    );
}

#[test]
fn a_local_does_not_become_the_temporary_it_copies() {
    stays(
        &[Pass::Coalesce],
        "v_000010_0 = Native.DEMO.GET(p0); local_1 = v_000010_0; Native.DEMO.USE2(local_1, v_000010_0); return;",
    );
}

#[test]
fn an_assignment_no_read_can_see_goes() {
    let body = |first: &str, els: &str| {
        format!(
            "{first} {TEST} {{ local_1 = 2; }} else {{ {els} }} Native.DEMO.USE(local_1); return;"
        )
    };
    becomes(
        &[Pass::Coalesce],
        &body("local_1 = 1;", "local_1 = 3;"),
        &body("", "local_1 = 3;"),
    );
    // On the path that assigns nothing more, the read sees the first assignment.
    stays(
        &[Pass::Coalesce],
        &body("local_1 = 1;", "Native.DEMO.USE(p0);"),
    );
    stays(&[Pass::Constants], &body("local_1 = 1;", "local_1 = 3;"));
    // The call of an assignment that goes stays, where it was.
    becomes(
        &[Pass::Coalesce],
        &body("local_1 = Native.DEMO.GET(p0);", "local_1 = 3;"),
        &body("Native.DEMO.GET(p0);", "local_1 = 3;"),
    );
}

#[test]
fn what_operators_make_of_constants_is_carried_like_a_constant() {
    // `local_1` is two variables in one cell: the first only ever holds five cut to 32 bits.
    let two = |first: &str, read: &str| {
        format!(
            "{first} {TEST} {{ Native.DEMO.USE({read}); }} else {{ }} Native.DEMO.USE2({read}, p0); local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;"
        )
    };
    becomes(
        &[Pass::Constants],
        &two("local_1 = as_u32(5);", "local_1"),
        &two("", "as_u32(5)"),
    );
    // Cut out of a variable, it is what the variable held then.
    stays(&[Pass::Constants], &two("local_1 = as_u32(p0);", "local_1"));
}

#[test]
fn a_value_made_for_the_next_statement_alone_is_settled_into_it() {
    let body = "v_000010_0 = Native.DEMO.GET(p0); local_1 = v_000010_0; Native.DEMO.USE2(local_1, local_1); return;";
    let settled = |options: Options| {
        let mut f = read(body);
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), options);
        let changed = settle(&mut f, &cx, &mut Stats::default());
        (f, changed)
    };
    let (made, changed) = settled(Options::baseline());
    assert!(changed);
    let wanted = read("local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;");
    assert_eq!(made.cfg, wanted.cfg);
    let temporary = Var::Value { at: 0x10, index: 0 };
    assert!(
        !made.head.vars.contains_key(&temporary),
        "{:?}",
        made.head.vars
    );
    // The fold is `values`': with that name off nothing moves.
    let (left, changed) = settled(Options::baseline().without(Pass::Values));
    assert!(!changed);
    assert_eq!(left.cfg, read(body).cfg);
}

#[test]
fn a_value_read_after_what_it_cannot_change_is_settled_into_its_reader_under_coalesce() {
    let body = "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(p1, v_000010_0); return;";
    let settled = |options: Options| {
        let mut f = read(body);
        let names = Names::default();
        let cx = Context::new(&names, Project::empty(), options);
        let changed = settle(&mut f, &cx, &mut Stats::default());
        (f, changed)
    };
    let (made, changed) = settled(Options::baseline().with(Pass::Coalesce));
    assert!(changed);
    let wanted = read("Native.DEMO.USE2(p1, Native.DEMO.GET(p0)); return;");
    assert_eq!(made.cfg, wanted.cfg);
    // With `coalesce` off only a value read first moves, and this one is read second.
    let (left, changed) = settled(Options::baseline());
    assert!(!changed);
    assert_eq!(left.cfg, read(body).cfg);
}
