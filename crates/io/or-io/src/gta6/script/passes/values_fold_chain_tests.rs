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
        }
    }
}
"#;

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

fn becomes(before: &str, after: &str) {
    let mut f = read(before);
    assert!(chained(&mut f), "nothing moved in:\n{before}");
    assert_eq!(f.cfg, read(after).cfg, "from:\n{before}");
}

fn stays(body: &str) {
    let mut f = read(body);
    assert!(!chained(&mut f), "something moved in:\n{body}");
    assert_eq!(f.cfg, read(body).cfg, "{body}");
}

/// A load through the first parameter and a call, each kept in a temporary, in that order.
const ROW_OF_TWO: &str = "v_000010_0 = load_u64(p0, 0x8); v_000018_1 = Native.DEMO.GET(p1);";

#[test]
fn a_row_of_values_read_in_their_order_by_the_next_statement_goes_into_it() {
    // The address a store goes through is evaluated before the value it stores.
    becomes(
        &format!("{ROW_OF_TWO} store_i32(v_000010_0, 0x10, v_000018_1); return;"),
        "store_i32(load_u64(p0, 0x8), 0x10, Native.DEMO.GET(p1)); return;",
    );
    // A parameter nothing but its assignments changes reads the same before a call as after.
    becomes(
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(p1, v_000010_0); return;",
        "Native.DEMO.USE2(p1, Native.DEMO.GET(p0)); return;",
    );
    // Read in the other order, the later value goes in and the earlier stays before it.
    becomes(
        "v_000010_0 = Native.DEMO.GET(p0); v_000018_1 = Native.DEMO.GET(p1); Native.DEMO.USE2(v_000018_1, v_000010_0); return;",
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(Native.DEMO.GET(p1), v_000010_0); return;",
    );
}

#[test]
fn a_row_stays_where_the_statement_does_something_that_shows_between_its_values() {
    for body in [
        // A load through the first value comes before the call is read: the call would pass it.
        format!("{ROW_OF_TWO} Native.DEMO.USE2(load_i32(v_000010_0, 0x10), v_000018_1); return;"),
        // A parameter whose address is taken may be what the call writes.
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(p1, v_000010_0); Native.DEMO.USE(Vm.Address(parameters, 1)); return;"
            .to_owned(),
        // In an arm that may not run, the call would not always be made.
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(((((as_u32(p1) == 0)) != Truth(0)) ? (VmValue)(v_000010_0) : (VmValue)(p1))); return;"
            .to_owned(),
        // A statement between is not the statement after the row.
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(p1); Native.DEMO.USE(v_000010_0); return;"
            .to_owned(),
    ] {
        stays(&body);
    }
}
