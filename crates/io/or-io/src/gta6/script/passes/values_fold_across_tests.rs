use super::*;
use crate::gta6::script::listing::Listing;

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
            public static void USE3(VmValue p0, VmValue p1, VmValue p2) => Vm.Native(0x0000000000000005UL, 0x00000005U, 0, p0, p1, p2);
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

/// `first`, a choice of two arms that each give `local_2` a value, the first doing `then` too, and `after` where the arms meet.
fn arms(first: &str, then: &str, after: &str) -> String {
    format!("{first} {TEST} {{ {then} local_2 = 1; }} else {{ local_2 = 2; }} {after} return;")
}

fn becomes(before: &str, after: &str) {
    let mut f = read(before);
    assert!(across(&mut f, false), "nothing moved in:\n{before}");
    assert_eq!(f.cfg, read(after).cfg, "from:\n{before}");
}

fn stays(body: &str) {
    let mut f = read(body);
    assert!(!across(&mut f, false), "something moved in:\n{body}");
    assert_eq!(f.cfg, read(body).cfg, "{body}");
}

#[test]
fn a_value_read_once_where_two_arms_meet_moves_into_its_reader() {
    becomes(
        &arms(
            "v_000010_0 = Native.DEMO.GET(p0);",
            "",
            "Native.DEMO.USE2(v_000010_0, local_2);",
        ),
        &arms("", "", "Native.DEMO.USE2(Native.DEMO.GET(p0), local_2);"),
    );
    // The later of two moves first, and leaves the way clear for the earlier.
    becomes(
        &arms(
            "v_000010_0 = Native.DEMO.GET(p0); v_000018_1 = Native.DEMO.GET(p1);",
            "",
            "Native.DEMO.USE3(v_000010_0, v_000018_1, local_2);",
        ),
        &arms(
            "",
            "",
            "Native.DEMO.USE3(Native.DEMO.GET(p0), Native.DEMO.GET(p1), local_2);",
        ),
    );
}

#[test]
fn a_value_stays_where_something_between_may_see_it_move() {
    let first = "v_000010_0 = Native.DEMO.GET(p0);";
    let after = "Native.DEMO.USE2(v_000010_0, local_2);";
    // A call in an arm keeps its place among calls.
    stays(&arms(first, "Native.DEMO.USE(p0);", after));
    // A store in an arm may write what a load reads.
    stays(&arms(
        "v_000010_0 = load_i32(p0, 0x10);",
        "store_i32(p1, 0x10, 5);",
        after,
    ));
    // The reader evaluates a call before it comes to the value.
    stays(&arms(
        first,
        "",
        "Native.DEMO.USE2(Native.DEMO.GET(p1), v_000010_0);",
    ));
}

#[test]
fn a_value_stays_where_a_way_from_it_may_not_come_to_its_read_once() {
    let first = "v_000010_0 = Native.DEMO.GET(p0);";
    // An arm leaves the function: there the call would never be made.
    stays(&format!(
        "{first} {TEST} {{ return; }} else {{ local_2 = 2; }} Native.DEMO.USE2(v_000010_0, local_2); return;"
    ));
    // Read in one arm, it is read on one way of two.
    stays(&format!(
        "{first} {TEST} {{ Native.DEMO.USE(v_000010_0); }} else {{ local_2 = 2; }} return;"
    ));
    // Read in a loop, it would be made once a round.
    stays(&format!(
        "{first} while (((as_u32(p1) == 0))) {{ Native.DEMO.USE(v_000010_0); }} return;"
    ));
}
