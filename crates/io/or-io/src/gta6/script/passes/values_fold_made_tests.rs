use super::super::{fold, made};
use crate::gta6::script::cfg;
use crate::gta6::script::listing::Listing;

/// A listing of one function of two parameters and eight locals, written by hand, with a delegate's constructor, an array type and natives to call.
const FRAME: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static void func_000010(VmValue arg0, VmValue arg1) // 0x000010
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[8];
        VmValue v_000010_0 = default; VmValue v_000018_1 = default; VmValue v_000020_2 = default;
        BODY
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Builtins
    {
        public static class System
        {
            public static class Delegate
            {
                public static void Ctor(VmValue p0, VmValue p1, VmValue p2) => Vm.Builtin(2, 0, 0, p0, p1, p2);
            }
        }
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
    public static class Operations
    {
        public static VmValue NewArray_type_00000000000000EE(VmValue p0) => Vm.NewArray(new VmType("type_00000000000000EE", 8, 21U, 0x00000102U), p0);
    }
}
"#;

/// A delegate made into `v_000010_0` and bound to `p0` and a function.
const DELEGATE: &str = "v_000010_0 = Vm.NewObject(new VmType(\"System.Delegate\", 48, 0x80000002U, 0x000C2000U)); Builtins.System.Delegate.Ctor(v_000010_0, p0, Vm.Function(0x00000001U, 0x000010));";

/// An array of two elements made into `v_000020_2`, and both its elements set.
const ARRAY: &str = "v_000020_2 = Operations.NewArray_type_00000000000000EE(2); Vm.Intrinsic(\"array_set_u32\", 0, v_000020_2, 0, 5); Vm.Intrinsic(\"array_set_u32\", 0, v_000020_2, 1, 6);";

/// An object made into `v_000020_2`, and a field of it stored.
const OBJECT: &str = "v_000020_2 = Vm.NewObject(new VmType(\"type_00000000000000EE\", 16, 21U, 0x00000102U)); store_i32(v_000020_2, 0x8, 2);";

/// The value of `GET(p0)` and a copy in `local_1` read, the copy twice so that it stays a variable.
const COPY_READ: &str = "Native.DEMO.USE2(v_000018_1, local_1); Native.DEMO.USE(local_1); return;";

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

/// The blocks [`made`] makes of `body`, with `delegates` and `arrays` as given, and whether it said it changed them.
fn filled(body: &str, delegates: bool, arrays: bool) -> (cfg::Cfg, bool) {
    let mut f = read(body);
    let changed = made(&mut f, delegates, arrays);
    (f.cfg, changed)
}

#[test]
fn a_read_moves_past_a_delegate_made_and_bound_for_the_statement_that_reads_it() {
    let before = format!(
        "v_000018_1 = load_u64(p0, 0x10); {DELEGATE} Native.DEMO.USE2(v_000018_1, v_000010_0); return;"
    );
    let after = format!("{DELEGATE} Native.DEMO.USE2(load_u64(p0, 0x10), v_000010_0); return;");
    assert_eq!(filled(&before, true, false), (read(&after).cfg, true));
    // Without `delegates` the binding is a call, which the read stays before.
    assert_eq!(filled(&before, false, true), (read(&before).cfg, false));
    let mut f = read(&before);
    fold(&mut f, true);
    assert_eq!(f.cfg, read(&before).cfg);
}

#[test]
fn a_value_stays_before_a_delegate_bound_where_it_calls_or_the_binding_is_not_the_making() {
    for body in [
        // The binding may refuse its target: a call before it stays before it.
        format!(
            "v_000018_1 = Native.DEMO.GET(p1); {DELEGATE} Native.DEMO.USE2(v_000018_1, v_000010_0); return;"
        ),
        // A statement between the making and the binding: the two are no pair.
        format!(
            "v_000018_1 = load_u64(p0, 0x10); {} Native.DEMO.USE2(v_000018_1, v_000010_0); return;",
            DELEGATE.replace("; Builtins.", "; local_1 = 3; Builtins.")
        ),
    ] {
        assert_eq!(filled(&body, true, true).0, read(&body).cfg, "{body}");
    }
}

#[test]
fn a_call_moves_past_the_elements_of_an_array_made_for_the_statement_that_reads_it() {
    let before =
        format!("v_000018_1 = Native.DEMO.GET(p0); {ARRAY} Native.DEMO.USE2(v_000018_1, v_000020_2); return;");
    let after = format!("{ARRAY} Native.DEMO.USE2(Native.DEMO.GET(p0), v_000020_2); return;");
    assert_eq!(filled(&before, false, true), (read(&after).cfg, true));
    // Without `initialisers` the setting of an element is a call.
    assert_eq!(filled(&before, true, false), (read(&before).cfg, false));
}

#[test]
fn a_call_stays_before_the_elements_of_an_array_that_may_fault_or_that_someone_else_holds() {
    let read_after = "Native.DEMO.USE2(v_000018_1, v_000020_2); return;";
    for body in [
        // An index past the length.
        format!(
            "v_000018_1 = Native.DEMO.GET(p0); {} {read_after}",
            ARRAY.replace("v_000020_2, 1, 6", "v_000020_2, 2, 6")
        ),
        // A length no constant states.
        format!(
            "v_000018_1 = Native.DEMO.GET(p0); {} {read_after}",
            ARRAY.replace("(2)", "(p1)")
        ),
        // The array copied before an element is set: the copy may be anywhere.
        format!(
            "v_000018_1 = Native.DEMO.GET(p0); {} {COPY_READ}",
            ARRAY.replace(
                "; Vm.Intrinsic(\"array_set_u32\", 0, v_000020_2, 1",
                "; local_1 = v_000020_2; Vm.Intrinsic(\"array_set_u32\", 0, v_000020_2, 1"
            )
        ),
    ] {
        assert_eq!(filled(&body, true, true).0, read(&body).cfg, "{body}");
    }
}

#[test]
fn a_call_stays_before_a_store_into_an_object_just_made() {
    let before =
        format!("v_000018_1 = Native.DEMO.GET(p0); {OBJECT} Native.DEMO.USE2(v_000018_1, v_000020_2); return;");
    let folded = |body: &str, finer: bool| {
        let mut f = read(body);
        fold(&mut f, finer);
        f.cfg
    };
    // ⚠ The before/after check sees the store as an event: the call stays in front of it.
    assert_eq!(folded(&before, true), read(&before).cfg);
    assert_eq!(folded(&before, false), read(&before).cfg);
    // Copied before the store, the object may be in the callee's hands.
    let copied = format!(
        "v_000018_1 = Native.DEMO.GET(p0); {} {COPY_READ}",
        OBJECT.replace("; store_i32", "; local_1 = v_000020_2; store_i32")
    );
    assert_eq!(folded(&copied, true), read(&copied).cfg);
}
