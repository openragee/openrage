use super::*;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::names::Names;
use crate::gta6::script::passes::values;
use crate::gta6::script::Stats;

/// A listing of one function of two parameters and eight locals, written by hand, with natives to call.
const FRAME: &str = r#"[VmProgram(0x00000001U)]
public static class Program_00000001
{
    [VmFunction(0x000010)]
    public static void func_000010(VmValue arg0, VmValue arg1) // 0x000010
    {
        VmValue[] parameters = new VmValue[] { arg0, arg1 };
        VmValue[] locals = new VmValue[8];
        DECLS
        BODY
        throw new global::System.InvalidOperationException("Unexpected bytecode fallthrough.");
    }

    public static class Native
    {
        public static class DEMO
        {
            public static VmValue GET(VmValue p0) => Vm.Native(0x0000000000000001UL, 0x00000001U, 1, p0)[0];
            public static VmValues PAIR() => Vm.Native(0x0000000000000002UL, 0x00000002U, 2);
            public static void USE(VmValue p0) => Vm.Native(0x0000000000000003UL, 0x00000003U, 0, p0);
            public static void USE2(VmValue p0, VmValue p1) => Vm.Native(0x0000000000000004UL, 0x00000004U, 0, p0, p1);
            public static void USE3(VmValue p0, VmValue p1, VmValue p2) => Vm.Native(0x0000000000000005UL, 0x00000005U, 0, p0, p1, p2);
        }
    }
}
"#;

/// One cell.
const V: &str = "VmValue v_000010_0 = default;";
/// Two values of several cells.
const TWO: &str = "VmValues v_000018_1 = null!; VmValues v_000020_2 = null!;";

/// A test of the second parameter, for a branch nothing here decides.
const TEST: &str = "if (((as_u32(p1) == 0)))";

/// The blocks of the function that declares `decls` and holds `body`.
fn read(decls: &str, body: &str) -> cfg::Function {
    let text = FRAME.replace("DECLS", decls).replace("BODY", body);
    let listing = Listing::open(&text).expect("the listing opens");
    let f = listing
        .functions()
        .next()
        .expect("the listing holds a function")
        .expect("the function reads");
    cfg::lower(f).expect("the function lowers")
}

/// The blocks the values pass makes of `body`.
fn ran(decls: &str, body: &str) -> cfg::Function {
    let mut f = read(decls, body);
    values::run(&mut f, &Names::default(), &mut Stats::default());
    f
}

/// Asserts that the values pass makes of `before` the blocks `after` reads as.
fn becomes(decls: &str, before: &str, after: &str) {
    let (made, wanted) = (ran(decls, before).cfg, read(decls, after).cfg);
    assert_eq!(made, wanted, "from:\n{before}");
}

fn stays(decls: &str, body: &str) {
    becomes(decls, body, body);
}

/// The pair `PAIR()` yields, spread over two locals, read back whole through the address of the first and passed on.
const SPREAD: &str = "v_000018_1 = Native.DEMO.PAIR(); local_0 = v_000018_1[0]; local_1 = v_000018_1[1]; v_000020_2 = load_bytes(Vm.Address(locals, 0), 0x0, 16); Native.DEMO.USE3(p0, v_000020_2[0], v_000020_2[1]);";

#[test]
fn a_local_of_several_cells_read_once_moves_into_its_reader() {
    let f = ran(TWO, &format!("{SPREAD} return;"));
    let [Stmt::Eval(Expr::Call { args, .. })] = &f.cfg.blocks[0].stmts[..] else {
        panic!("one call was expected: {:?}", f.cfg.blocks[0].stmts);
    };
    assert_eq!(args.len(), 2, "{args:?}");
    assert_eq!(args[0], Expr::Var(Var::Param(0)));
    let pair = matches!(
        &args[1],
        Expr::Call { callee: Callee::Native { name, .. }, returns: Some(2), .. } if name == "PAIR"
    );
    assert!(pair, "{args:?}");
    // The local is named nowhere any more, and is no variable of the function.
    assert!(
        !f.head.vars.contains_key(&Var::Local(0)),
        "{:?}",
        f.head.vars
    );
}

#[test]
fn a_local_of_several_cells_stays_where_its_address_is_taken_or_it_is_read_again() {
    // The second cell is read through the address of the first.
    let through = ran(
        TWO,
        &format!("{SPREAD} Native.DEMO.USE(load_u64(Vm.Address(locals, 0), 0x8)); return;"),
    );
    let held = |f: &cfg::Function| {
        matches!(
            f.cfg.blocks[0].stmts.first(),
            Some(Stmt::Assign {
                target: Var::Local(0),
                value: Expr::Call { .. }
            })
        )
    };
    assert!(held(&through), "{:?}", through.cfg.blocks[0].stmts);
    // Passed on twice, it is a variable.
    let twice = ran(
        TWO,
        &format!("{SPREAD} Native.DEMO.USE3(p1, local_0, local_1); return;"),
    );
    assert!(held(&twice), "{:?}", twice.cfg.blocks[0].stmts);
}

/// A field worked on in place through `addr`, held in a local for that one statement, with `operand` as what it is worked with.
fn in_place(addr: &str, operand: &str) -> String {
    format!(
        "local_1 = {addr}; store_u64(local_1, 0x0, ((as_u64(load_u64(local_1, 0x0)) | as_u64({operand})))); return;"
    )
}

/// The same statement with the address written where the local was read.
fn in_line(addr: &str, operand: &str) -> String {
    format!(
        "store_u64({addr}, 0x0, ((as_u64(load_u64({addr}, 0x0)) | as_u64({operand})))); return;"
    )
}

#[test]
fn an_address_held_for_the_one_statement_that_works_through_it_is_written_in_both_its_places() {
    let field = "byte_offset(load_u64(p0, 0x10), 96)";
    becomes("", &in_place(field, "4"), &in_line(field, "4"));
    // An address that reads no memory is the same after a call as before it.
    let own = "byte_offset(p0, 96)";
    becomes(
        "",
        &in_place(own, "Native.DEMO.GET(p1)"),
        &in_line(own, "Native.DEMO.GET(p1)"),
    );
}

#[test]
fn an_address_stays_in_its_variable_where_evaluating_it_twice_may_differ() {
    // The source evaluates the address once: a call in the other operand keeps it so.
    stays(
        "",
        &in_place("byte_offset(load_u64(p0, 0x10), 96)", "Native.DEMO.GET(p1)"),
    );
    // Two calls are two calls.
    stays("", &in_place("Native.DEMO.GET(p0)", "4"));
    // Read a third time, it is a variable.
    stays(
        "",
        "local_1 = byte_offset(p0, 96); store_u64(local_1, 0x0, ((as_u64(load_u64(local_1, 0x0)) | as_u64(4)))); Native.DEMO.USE(local_1); return;",
    );
    // A load of another field is no work on the field stored.
    stays(
        "",
        "local_1 = byte_offset(p0, 96); store_u64(local_1, 0x0, ((as_u64(load_u64(local_1, 0x8)) | as_u64(4)))); return;",
    );
}

#[test]
fn what_operators_make_of_constants_is_a_constant() {
    let five = || Expr::Const(Const::i32(5));
    assert!(constant(&five()));
    assert!(constant(&Expr::cast(Scalar::U32, five())));
    assert!(constant(&Expr::binary(BinOp::Shl, five(), five())));
    assert!(constant(&Expr::Truth(Box::new(five()))));
    // A division may fault where it is evaluated.
    assert!(!constant(&Expr::binary(BinOp::Div, five(), five())));
    assert!(!constant(&Expr::binary(BinOp::Rem, five(), five())));
    assert!(!constant(&Expr::Var(Var::Param(0))));
    assert!(!constant(&Expr::binary(
        BinOp::Add,
        five(),
        Expr::Var(Var::Local(1))
    )));
    // Past a few nodes it is no longer written at every read.
    let mut long = five();
    for _ in 0..CONSTANT {
        long = Expr::cast(Scalar::I32, long);
    }
    assert!(!constant(&long));
}

#[test]
fn a_variable_that_holds_what_operators_make_of_constants_is_that() {
    let body = |first: &str, read: &str| {
        format!(
            "{first} {TEST} {{ Native.DEMO.USE({read}); }} else {{ }} Native.DEMO.USE2({read}, p0); return;"
        )
    };
    becomes(
        "",
        &body("local_1 = as_u32(5);", "local_1"),
        &body("", "as_u32(5)"),
    );
    becomes(
        V,
        &body("v_000010_0 = ((as_i32(3) + as_i32(4)));", "v_000010_0"),
        &body("", "((as_i32(3) + as_i32(4)))"),
    );
    // Worked out of a variable, it is what the variable held then.
    stays(
        "",
        &body("local_1 = as_u32(Native.DEMO.GET(p0));", "local_1"),
    );
}

#[test]
fn bytes_of_variables_compared_for_nothing_leave_nothing() {
    let facts = Facts::of(&read("", "return;"));
    let compared = |args: Vec<Expr>| Expr::Call {
        callee: Callee::Intrinsic {
            name: "bytes_equal_16".to_owned(),
            ty: None,
            wrapper: None,
        },
        args,
        returns: Some(1),
    };
    let own = |n: u32| Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    };
    assert_eq!(leftover(&facts, compared(vec![own(1), own(3)])), Ok(None));
    // Through a pointer the comparison may fault, and stays.
    let through = compared(vec![Expr::Var(Var::Param(0)), own(3)]);
    assert_eq!(leftover(&facts, through.clone()), Ok(Some(through)));
}

#[test]
fn a_value_made_for_the_next_statement_alone_is_evaluated_by_it() {
    let settled = |body: &str| {
        let mut f = read(V, body);
        let changed = adjacent(&mut f);
        (f.cfg, changed)
    };
    let (made, changed) = settled(
        "v_000010_0 = Native.DEMO.GET(p0); local_1 = v_000010_0; Native.DEMO.USE2(local_1, local_1); return;",
    );
    let wanted = read(
        V,
        "local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;",
    );
    assert!(changed);
    assert_eq!(made, wanted.cfg);
    // The test a block ends on reads it first.
    let (made, _) = settled(
        "v_000010_0 = Native.DEMO.GET(p0); if (((as_u32(v_000010_0) == 0))) { Native.DEMO.USE(p0); } else { } return;",
    );
    let wanted = read(
        V,
        "if (((as_u32(Native.DEMO.GET(p0)) == 0))) { Native.DEMO.USE(p0); } else { } return;",
    );
    assert_eq!(made, wanted.cfg);
}

#[test]
fn a_value_stays_where_its_reader_evaluates_something_first_or_another_reads_it() {
    for body in [
        // The first argument is evaluated before the second.
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(p1, v_000010_0); return;",
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(v_000010_0, v_000010_0); return;",
        // The statement after the next is not the next.
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(p1); Native.DEMO.USE(v_000010_0); return;",
    ] {
        let mut f = read(V, body);
        assert!(!adjacent(&mut f), "{body}");
        assert_eq!(f.cfg, read(V, body).cfg, "{body}");
    }
}

/// One cell, and a value of two cells.
const BUILT: &str = "VmValue v_000010_0 = default; VmValues v_000020_2 = null!;";

/// A temporary of two cells given zeroes and written through its own address, `BETWEEN` first.
const BUILDS: &str = "v_000020_2 = zero_slots(2); BETWEEN store_i32(Vm.Address(v_000020_2, 0), 0x0, 5); zero_bytes(byte_offset(Vm.Address(v_000020_2, 0), 8), 8);";

/// What `GET(p0)` gives, held in a temporary before [`BUILDS`] and stored into the built value after, then `after`.
fn built(between: &str, after: &str) -> String {
    let builds = BUILDS.replace("BETWEEN", between);
    format!("v_000010_0 = Native.DEMO.GET(p0); {builds} store_i32(Vm.Address(v_000020_2, 0), 0x4, v_000010_0); {after}")
}

/// The same with the call made where the temporary was read.
fn built_in_place(after: &str) -> String {
    let builds = BUILDS.replace("BETWEEN", "");
    format!("{builds} store_i32(Vm.Address(v_000020_2, 0), 0x4, Native.DEMO.GET(p0)); {after}")
}

/// The blocks [`fold`] makes of `body`, with `escape` on or off.
fn folded(body: &str, finer: bool) -> cfg::Cfg {
    let mut f = read(BUILT, body);
    fold(&mut f, finer);
    f.cfg
}

#[test]
fn a_value_moves_past_the_writes_of_a_temporary_no_one_holds_the_address_of_yet() {
    let passed = "Native.DEMO.USE3(p0, v_000020_2[0], v_000020_2[1]); return;";
    assert_eq!(
        folded(&built("", passed), true),
        read(BUILT, &built_in_place(passed)).cfg
    );
    // Handed on after the value is read, the address is in no hands before it.
    let handed = "Native.DEMO.USE(Vm.Address(v_000020_2, 0)); return;";
    assert_eq!(
        folded(&built("", handed), true),
        read(BUILT, &built_in_place(handed)).cfg
    );
    // Without `escape`, the temporary is memory a call may reach.
    let body = built("", passed);
    assert_eq!(folded(&body, false), read(BUILT, &body).cfg);
}

#[test]
fn a_value_stays_before_the_writes_of_a_temporary_whose_address_someone_may_hold() {
    for body in [
        // Kept in a local before the read.
        built(
            "local_5 = Vm.Address(v_000020_2, 0);",
            "Native.DEMO.USE2(local_5, local_5); return;",
        ),
        // Handed on in a loop: the round before handed it to a callee that may still hold it.
        format!(
            "while (((as_u32(p1) == 0))) {{ {} }} return;",
            built("", "Native.DEMO.USE(Vm.Address(v_000020_2, 0));")
        ),
        // The value reads the temporary the statements write.
        built("", "return;").replace(
            "Native.DEMO.GET(p0)",
            "Native.DEMO.GET(load_i32(Vm.Address(v_000020_2, 0), 0x0))",
        ),
    ] {
        assert_eq!(folded(&body, true), read(BUILT, &body).cfg, "{body}");
    }
}

#[test]
fn a_block_on_a_way_that_comes_back_to_it_circles() {
    let f = read(
        "",
        "local_1 = 1; while (((as_u32(p1) == 0))) { local_1 = 2; } Native.DEMO.USE(local_1); return;",
    );
    let circles = circling(&f.cfg);
    let holding = |value: u64| {
        (f.cfg.blocks.iter()).position(|block| {
            (block.stmts.iter()).any(|stmt| {
                matches!(stmt, Stmt::Assign { value: Expr::Const(c), .. } if c.bits == value)
            })
        })
    };
    // The block before the loop runs once; the loop's body comes back to itself.
    assert_eq!(holding(1).map(|b| circles[b]), Some(false), "{circles:?}");
    assert_eq!(holding(2).map(|b| circles[b]), Some(true), "{circles:?}");
    let called = (f.cfg.blocks.iter()).position(|block| {
        (block.stmts.iter()).any(|stmt| matches!(stmt, Stmt::Eval(Expr::Call { .. })))
    });
    assert_eq!(called.map(|b| circles[b]), Some(false), "{circles:?}");
    let straight = read("", "local_1 = 1; Native.DEMO.USE(local_1); return;");
    assert_eq!(
        circling(&straight.cfg),
        vec![false; straight.cfg.blocks.len()]
    );
}
