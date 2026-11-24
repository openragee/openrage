use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName};

#[test]
fn a_listing_decompiles_from_end_to_end() {
    let done = decompile(sample::LISTING, &Names::default()).expect("the sample decompiles");
    let statements: usize = sample::STATEMENTS.iter().sum();
    assert_eq!(
        done.stats,
        Stats {
            functions: 5,
            incomplete: 1,
            statements_in: statements,
            // Three tuples are six assignments; three values nothing reads and a load of a
            // global nothing reads go, one temporary moves into the switch that reads it, and
            // the clear of a local that is only ever zero goes with the local.
            statements_out: statements + 3 - 4 - 1 - 1,
            gotos_in: 3,
            gotos_out: 0,
            temporaries_in: 12,
            temporaries_out: 7,
            native_calls: 3,
            natives_named: 2,
        }
    );

    // The first line says what was read: a lifted listing, and no compiled script.
    let first = done.text.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("// Readable source of a lifted listing")
            && first.ends_with("not from the compiled script."),
        "{first}"
    );
    let head = "\
// program 0x11223344
// imports: 0x0A0B0C0D, 0x1A2B3C4D
";
    assert!(done.text.contains(head), "{}", done.text);
    for line in [
        "void func_000000(void* p0, void* p1)",
        "var func_000100(var p0)",
        "var[] func_000200()",
        "// func_000300 at 0x000300: the listing holds no body (Widths are underdetermined.)",
        "void main()",
        "    DEMO.DO_THING(0);",
        "    _0x00000000000000BB(p0, BuiltinGlobal(0)[0]);",
        // A store says how wide it is, and a pointer is gone through with `->`.
        "    p0->f_10 = (byte)1;",
        "        if ((byte)p1->f_8->f_10 == 0)",
        "    v_000028_3.f_0 = 0x1234ABCD;",
        // The loop the two gotos made, and the switch without the one its value was biased by.
        "    while ((int)local_1 < (int)example_length(local_0));",
        "    while (local_0 < 10)",
        "        switch ((int)DEMO.GET_THING(local_0))",
        "            case 2:",
        "                return { local_0, local_1 };",
        "    return { local_0, local_1 };",
    ] {
        assert!(
            done.text.lines().any(|l| l == line),
            "no line `{line}` in:\n{}",
            done.text
        );
    }
    // The temporaries nothing reads are gone with what they held.
    for gone in ["v_00010C_1", "v_000114_3", "v_000118_4", "v_000110_2"] {
        assert!(!done.text.contains(gone), "{gone} in:\n{}", done.text);
    }
}

#[test]
fn a_native_takes_the_name_the_users_list_gives_its_hash() {
    let listed = NativeName {
        namespace: Some("LISTED".to_owned()),
        name: "NAMED_BY_THE_LIST".to_owned(),
        ..NativeName::default()
    };
    let names = Names {
        natives: Some(NativeDb::from_names("a test's", [(0xBB, listed)])),
        ..Names::default()
    };
    let done = decompile(sample::LISTING, &names).expect("the sample decompiles");
    assert_eq!((done.stats.native_calls, done.stats.natives_named), (3, 3));
    assert!(
        done.text
            .contains("LISTED.NAMED_BY_THE_LIST(p0, BuiltinGlobal(0)[0]);"),
        "{}",
        done.text
    );
}

#[test]
fn a_listing_that_does_not_read_or_lower_is_refused() {
    let unread = decompile("not a listing\n", &Names::default());
    assert!(
        matches!(&unread, Err(DecompileError::Listing(e)) if e.line() == 1),
        "{unread:?}"
    );

    let text = "[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static void func_000010() // 0x000010
    {
        VmValue[] parameters = new VmValue[] {  };
        VmValue[] locals = new VmValue[0];
        goto label_000020;
    }
}
";
    let unlowered = decompile(text, &Names::default());
    let Err(DecompileError::Lower { function, source }) = &unlowered else {
        panic!("a goto to no label lowered: {unlowered:?}");
    };
    assert_eq!(function, "func_000010");
    assert_eq!(
        *source,
        cfg::LowerError::UnknownLabel(tree::Label::At(0x20))
    );
}

#[test]
fn stats_add_up() {
    let one = Stats {
        functions: 1,
        incomplete: 2,
        statements_in: 3,
        statements_out: 4,
        gotos_in: 5,
        gotos_out: 6,
        temporaries_in: 7,
        temporaries_out: 8,
        native_calls: 9,
        natives_named: 10,
    };
    let mut sum = one;
    sum.add(&one);
    assert_eq!(
        sum,
        Stats {
            functions: 2,
            incomplete: 4,
            statements_in: 6,
            statements_out: 8,
            gotos_in: 10,
            gotos_out: 12,
            temporaries_in: 14,
            temporaries_out: 16,
            native_calls: 18,
            natives_named: 20,
        }
    );
}
