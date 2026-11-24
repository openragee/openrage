use super::*;
use crate::gta6::script::ir::{Callee, Expr, Kind, Stmt, Var, Width};
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
/// Two cells, each its own temporary.
const V2: &str = "VmValue v_000010_0 = default; VmValue v_000018_1 = default;";
/// A value of several cells.
const MANY: &str = "VmValues v_000018_1 = null!;";
const STACK: &str = "VmValue stack_000040_0 = default; VmValue stack_000040_1 = default; VmValue stack_000050_0 = default;";

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

fn ran(decls: &str, body: &str) -> cfg::Function {
    let mut f = read(decls, body);
    run(&mut f, &Names::default(), &mut Stats::default());
    f
}

/// Asserts that the pass makes of `before` the blocks `after` reads as.
fn becomes(decls: &str, before: &str, after: &str) {
    let (made, wanted) = (ran(decls, before).cfg, read(decls, after).cfg);
    assert_eq!(made, wanted, "from:\n{before}");
}

/// Asserts that the pass leaves `body` as it reads.
fn stays(decls: &str, body: &str) {
    becomes(decls, body, body);
}

#[test]
fn a_temporary_read_once_moves_into_its_reader() {
    becomes(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(v_000010_0); return;",
        "Native.DEMO.USE(Native.DEMO.GET(p0)); return;",
    );
}

#[test]
fn a_load_does_not_move_past_a_store() {
    stays(
        V,
        "v_000010_0 = load_i32(p0, 0x10); store_i32(p0, 0x10, 5); Native.DEMO.USE(v_000010_0); return;",
    );
}

#[test]
fn a_call_does_not_move_past_a_call() {
    stays(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(p1); Native.DEMO.USE(v_000010_0); return;",
    );
}

#[test]
fn values_move_only_into_the_order_they_were_evaluated_in() {
    becomes(
        V2,
        "v_000010_0 = Native.DEMO.GET(p0); v_000018_1 = Native.DEMO.GET(p1); Native.DEMO.USE2(v_000010_0, v_000018_1); return;",
        "Native.DEMO.USE2(Native.DEMO.GET(p0), Native.DEMO.GET(p1)); return;",
    );
    // Moved into the first argument, the second call would run before the first.
    becomes(
        V2,
        "v_000010_0 = Native.DEMO.GET(p0); v_000018_1 = Native.DEMO.GET(p1); Native.DEMO.USE2(v_000018_1, v_000010_0); return;",
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(Native.DEMO.GET(p1), v_000010_0); return;",
    );
}

#[test]
fn a_call_does_not_move_into_an_arm_that_may_not_run() {
    stays(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE((((((as_i32(p1) == as_i32(5))) != Truth(0)) ? (VmValue)(v_000010_0) : (VmValue)(7)))); return;",
    );
}

#[test]
fn a_local_read_once_moves_into_its_reader_like_a_temporary() {
    becomes(
        "",
        "local_1 = Native.DEMO.GET(p0); Native.DEMO.USE(local_1); return;",
        "Native.DEMO.USE(Native.DEMO.GET(p0)); return;",
    );
    // Read twice, it is a variable.
    stays(
        "",
        "local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;",
    );
    // An address at or below it may reach it: the callee may read the cell the assignment fills.
    stays(
        "",
        "local_3 = ((as_i32(p0) + as_i32(1))); Native.DEMO.USE2(local_3, Vm.Address(locals, 2)); return;",
    );
    // Assigned twice, the read takes whichever ran last.
    stays(
        "",
        "local_1 = 5; if (((as_u32(p1) == 0))) { local_1 = 7; } else { } Native.DEMO.USE(local_1); return;",
    );
}

#[test]
fn a_temporary_that_copies_a_variable_nothing_changes_is_the_variable() {
    // Across a join, where no block's own folding reaches the read.
    becomes(
        V,
        "v_000010_0 = p0; if (((as_u32(p1) == 0))) { Native.DEMO.USE(p1); } else { } Native.DEMO.USE2(v_000010_0, v_000010_0); return;",
        "if (((as_u32(p1) == 0))) { Native.DEMO.USE(p1); } else { } Native.DEMO.USE2(p0, p0); return;",
    );
    // A local assigned once, before the copy.
    becomes(
        V,
        "local_1 = Native.DEMO.GET(p0); v_000010_0 = local_1; if (((as_u32(p1) == 0))) { Native.DEMO.USE(p1); } else { } Native.DEMO.USE2(v_000010_0, local_1); return;",
        "local_1 = Native.DEMO.GET(p0); if (((as_u32(p1) == 0))) { Native.DEMO.USE(p1); } else { } Native.DEMO.USE2(local_1, local_1); return;",
    );
    // A local that only ever holds the copy is the variable too.
    becomes(
        "",
        "local_1 = Native.DEMO.GET(p0); local_2 = local_1; if (((as_u32(p1) == 0))) { Native.DEMO.USE(p1); } else { } Native.DEMO.USE2(local_2, local_1); return;",
        "local_1 = Native.DEMO.GET(p0); if (((as_u32(p1) == 0))) { Native.DEMO.USE(p1); } else { } Native.DEMO.USE2(local_1, local_1); return;",
    );
    // A constant is the same wherever it is read.
    becomes(
        "",
        "local_1 = 0x1CB5D124; if (((as_u32(p1) == 0))) { Native.DEMO.USE(local_1); } else { } Native.DEMO.USE2(local_1, p0); return;",
        "if (((as_u32(p1) == 0))) { Native.DEMO.USE(0x1CB5D124); } else { } Native.DEMO.USE2(0x1CB5D124, p0); return;",
    );
    // On the path round the assignment the local still holds its first zero.
    stays(
        "",
        "if (((as_u32(p1) == 0))) { local_1 = 7; } else { } Native.DEMO.USE2(local_1, local_1); return;",
    );
    // Assigned after the copy, the variable is no longer what the temporary holds.
    stays(
        V,
        "v_000010_0 = local_1; local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(v_000010_0, local_1); Native.DEMO.USE(local_1); return;",
    );
    // A pointer may reach the variable, so a call may change it.
    stays(
        V,
        "Native.DEMO.USE(Vm.Address(parameters, 0)); v_000010_0 = p0; Native.DEMO.USE(p1); Native.DEMO.USE2(v_000010_0, v_000010_0); return;",
    );
}

#[test]
fn a_local_that_takes_a_value_after_its_first_readers_holds_it_from_the_start() {
    // The value is filled through the temporary before the local takes it.
    becomes(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); store_ref(v_000010_0, 0x8, p1); local_1 = v_000010_0; Native.DEMO.USE2(local_1, local_1); return;",
        "local_1 = Native.DEMO.GET(p0); store_ref(local_1, 0x8, p1); Native.DEMO.USE2(local_1, local_1); return;",
    );
    // A local kept like a temporary is copied the same way.
    becomes(
        "",
        "local_2 = Native.DEMO.GET(p0); store_ref(local_2, 0x8, p1); local_1 = local_2; Native.DEMO.USE2(local_1, local_1); return;",
        "local_1 = Native.DEMO.GET(p0); store_ref(local_1, 0x8, p1); Native.DEMO.USE2(local_1, local_1); return;",
    );
    // The local's old value is read before the copy: it cannot take the new one any earlier.
    stays(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, v_000010_0); local_1 = v_000010_0; Native.DEMO.USE2(local_1, local_1); return;",
    );
}

#[test]
fn cells_zeroed_and_then_filled_to_the_last_byte_are_the_value_that_fills_them() {
    let two = "VmValues v_000018_1 = null!; VmValues v_000020_2 = null!;";
    let f = ran(
        two,
        "v_000018_1 = Native.DEMO.PAIR(); v_000020_2 = zero_slots(2); store_bytes(Vm.Address(v_000020_2, 0), 0x0, 16, new VmValues(v_000018_1[0], v_000018_1[1])); Native.DEMO.USE3(p0, v_000020_2[0], v_000020_2[1]); return;",
    );
    // The cells are the pair, and the pair is what the call is passed.
    let [Stmt::Eval(Expr::Call { args, .. })] = &f.cfg.blocks[0].stmts[..] else {
        panic!("one call was expected: {:?}", f.cfg.blocks[0].stmts);
    };
    let pair = matches!(
        &args[..],
        [Expr::Var(Var::Param(0)), Expr::Call { callee: Callee::Native { name, .. }, .. }] if name == "PAIR"
    );
    assert!(pair, "{args:?}");
    // Twelve of sixteen bytes leave four zeroed: the cells are not the value, and the store stays.
    let part = ran(
        two,
        "v_000018_1 = Native.DEMO.PAIR(); v_000020_2 = zero_slots(2); store_bytes(Vm.Address(v_000020_2, 0), 0x0, 12, v_000018_1); Native.DEMO.USE3(p0, v_000020_2[0], v_000020_2[1]); return;",
    );
    let stmts = &part.cfg.blocks[0].stmts;
    let stored = |stmt: &Stmt| matches!(stmt, Stmt::StoreBytes { bytes: 12, .. });
    assert!(stmts.iter().any(stored), "{stmts:?}");
}

#[test]
fn a_frame_cell_loaded_through_its_own_address_is_the_variable_read() {
    becomes(
        "",
        "Native.DEMO.USE2(load_i32(Vm.Address(locals, 1), 0x0), load_u64(Vm.Address(parameters, 0), 0x0)); return;",
        "Native.DEMO.USE2(as_i32(local_1), p0); return;",
    );
    becomes(
        "",
        "store_ref(Vm.Address(locals, 1), 0x0, Native.DEMO.GET(p0)); Native.DEMO.USE2(local_1, local_1); return;",
        "local_1 = Native.DEMO.GET(p0); Native.DEMO.USE2(local_1, local_1); return;",
    );
    // Four bytes written leave the cell's other four: no assignment does that.
    stays(
        "",
        "store_i32(Vm.Address(locals, 1), 0x0, p0); Native.DEMO.USE2(local_1, local_1); return;",
    );
    // Past the cell's first byte the load is of memory.
    stays(
        "",
        "Native.DEMO.USE(load_i32(Vm.Address(locals, 1), 0x4)); return;",
    );
}

#[test]
fn a_cell_zeroed_filled_once_and_read_back_is_the_value_cut_to_what_was_stored() {
    // The hash a listing hands over through a cell of its own, with a statement between.
    becomes(
        MANY,
        "v_000018_1 = zero_slots(1); local_1 = 0xFFFFFFFF9ABCDEF0; local_0 = Vm.Address(v_000018_1, 0); store_i32(Vm.Address(v_000018_1, 0), 0x0, local_1); Native.DEMO.USE2(p0, v_000018_1[0]); return;",
        "Native.DEMO.USE2(p0, as_u32(0xFFFFFFFF9ABCDEF0)); return;",
    );
    // An address that goes elsewhere too leaves the cell memory.
    stays(
        MANY,
        "v_000018_1 = zero_slots(1); store_i32(Vm.Address(v_000018_1, 0), 0x0, 5); Native.DEMO.USE2(Vm.Address(v_000018_1, 0), v_000018_1[0]); return;",
    );
}

#[test]
fn a_local_a_pointer_reaches_is_read_before_the_next_call() {
    stays(
        V,
        "Native.DEMO.USE(Vm.Address(locals, 2)); v_000010_0 = local_2; Native.DEMO.USE(p0); Native.DEMO.USE(v_000010_0); return;",
    );
    // A struct runs upward from the cell whose address is taken: the local below it is out of reach.
    becomes(
        V,
        "Native.DEMO.USE(Vm.Address(locals, 2)); v_000010_0 = local_1; Native.DEMO.USE(p0); Native.DEMO.USE(v_000010_0); return;",
        "Native.DEMO.USE(Vm.Address(locals, 2)); Native.DEMO.USE(p0); Native.DEMO.USE(local_1); return;",
    );
}

#[test]
fn a_store_through_a_locals_own_address_changes_the_cells_it_covers_and_no_other() {
    becomes(
        V,
        "v_000010_0 = local_5; store_i32(Vm.Address(locals, 3), 0x0, p1); Native.DEMO.USE(v_000010_0); return;",
        "store_i32(Vm.Address(locals, 3), 0x0, p1); Native.DEMO.USE(local_5); return;",
    );
    // Sixteen bytes from local 4 are locals 4 and 5.
    stays(
        V,
        "v_000010_0 = local_5; zero_bytes(Vm.Address(locals, 4), 16); Native.DEMO.USE(v_000010_0); return;",
    );
}

#[test]
fn an_assignment_nothing_reads_goes_and_its_call_stays() {
    becomes(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); return;",
        "Native.DEMO.GET(p0); return;",
    );
    becomes(
        V,
        "v_000010_0 = ((as_i32(p0) + as_i32(1))); return;",
        "return;",
    );
    // A load may fault, and is no statement of its own: the assignment is what holds it.
    stays(V, "v_000010_0 = load_u64(p0, 0x8); return;");
}

#[test]
fn a_local_an_address_may_reach_keeps_its_assignment() {
    becomes(
        "",
        "local_1 = 7; Native.DEMO.USE(Vm.Address(locals, 2)); return;",
        "Native.DEMO.USE(Vm.Address(locals, 2)); return;",
    );
    stays(
        "",
        "local_3 = 7; Native.DEMO.USE(Vm.Address(locals, 2)); return;",
    );
}

#[test]
fn a_local_given_a_temporarys_value_holds_it_for_the_later_readers() {
    becomes(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); local_0 = v_000010_0; Native.DEMO.USE(v_000010_0); Native.DEMO.USE(local_0); return;",
        "local_0 = Native.DEMO.GET(p0); Native.DEMO.USE(local_0); Native.DEMO.USE(local_0); return;",
    );
    // The local is given another value before the temporary is read again.
    stays(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); local_0 = v_000010_0; local_0 = 5; Native.DEMO.USE(v_000010_0); Native.DEMO.USE(local_0); return;",
    );
}

#[test]
fn a_value_evaluated_for_a_temporary_and_again_for_a_local_is_evaluated_once() {
    becomes(
        V,
        "v_000010_0 = load_u64(p0, 0x8); local_0 = load_u64(p0, 0x8); Native.DEMO.USE(v_000010_0); Native.DEMO.USE(local_0); return;",
        "local_0 = load_u64(p0, 0x8); Native.DEMO.USE(local_0); Native.DEMO.USE(local_0); return;",
    );
    // Two calls are two calls.
    stays(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); local_0 = Native.DEMO.GET(p0); Native.DEMO.USE(v_000010_0); Native.DEMO.USE(local_0); return;",
    );
}

#[test]
fn a_tuple_is_taken_apart_unless_a_value_reads_a_target() {
    becomes(
        STACK,
        "(stack_000040_0, stack_000040_1) = (Native.DEMO.GET(p0), Native.DEMO.GET(p1)); Native.DEMO.USE2(stack_000040_0, stack_000040_1); return;",
        "Native.DEMO.USE2(Native.DEMO.GET(p0), Native.DEMO.GET(p1)); return;",
    );
    // A swap reads both before it writes either.
    stays(
        STACK,
        "stack_000040_0 = Native.DEMO.GET(p0); stack_000040_1 = Native.DEMO.GET(p1); (stack_000040_0, stack_000040_1) = (stack_000040_1, stack_000040_0); Native.DEMO.USE2(stack_000040_0, stack_000040_1); return;",
    );
}

#[test]
fn a_tuples_target_nothing_reads_leaves_the_tuple() {
    becomes(
        STACK,
        "stack_000040_0 = Native.DEMO.GET(p0); stack_000040_1 = Native.DEMO.GET(p1); (stack_000040_0, stack_000040_1, stack_000050_0) = (stack_000040_1, stack_000040_0, p0); Native.DEMO.USE2(stack_000040_0, stack_000040_1); return;",
        "stack_000040_0 = Native.DEMO.GET(p0); stack_000040_1 = Native.DEMO.GET(p1); (stack_000040_0, stack_000040_1) = (stack_000040_1, stack_000040_0); Native.DEMO.USE2(stack_000040_0, stack_000040_1); return;",
    );
}

#[test]
fn a_join_given_the_same_thing_on_every_path_is_that_thing() {
    becomes(
        STACK,
        "if (((as_u32(p1) == 0))) { stack_000040_0 = p0; } else { stack_000040_0 = p0; } Native.DEMO.USE(stack_000040_0); return;",
        "Native.DEMO.USE(p0); return;",
    );
}

#[test]
fn a_value_renamed_from_join_to_join_is_the_value() {
    becomes(
        STACK,
        "stack_000040_0 = p0; if (((as_u32(p1) == 0))) { stack_000050_0 = stack_000040_0; } else { Native.DEMO.USE(p1); stack_000050_0 = stack_000040_0; } Native.DEMO.USE(stack_000050_0); return;",
        "if (((as_u32(p1) == 0))) { } else { Native.DEMO.USE(p1); } Native.DEMO.USE(p0); return;",
    );
}

#[test]
fn a_join_is_kept_where_what_it_carries_changes_before_it_is_read() {
    stays(
        STACK,
        "stack_000040_0 = local_0; local_0 = 5; Native.DEMO.USE(stack_000040_0); Native.DEMO.USE(local_0); return;",
    );
}

#[test]
fn a_join_only_copied_into_the_next_join_is_that_join() {
    becomes(
        STACK,
        "if (((as_u32(p0) == 0))) { stack_000040_0 = Native.DEMO.GET(p0); } else { stack_000040_0 = Native.DEMO.GET(p1); } if (((as_u32(p1) == 0))) { stack_000050_0 = stack_000040_0; } else { stack_000050_0 = 7; } Native.DEMO.USE(stack_000050_0); return;",
        "if (((as_u32(p0) == 0))) { stack_000050_0 = Native.DEMO.GET(p0); } else { stack_000050_0 = Native.DEMO.GET(p1); } if (((as_u32(p1) == 0))) { } else { stack_000050_0 = 7; } Native.DEMO.USE(stack_000050_0); return;",
    );
    // A local that takes the join's value is assigned where the join was.
    becomes(
        STACK,
        "if (((as_u32(p0) == 0))) { stack_000040_0 = Native.DEMO.GET(p0); } else { stack_000040_0 = Native.DEMO.GET(p1); } local_1 = stack_000040_0; Native.DEMO.USE2(local_1, local_1); return;",
        "if (((as_u32(p0) == 0))) { local_1 = Native.DEMO.GET(p0); } else { local_1 = Native.DEMO.GET(p1); } Native.DEMO.USE2(local_1, local_1); return;",
    );
    // Read while it still holds its old value, the local cannot take the new one any earlier.
    stays(
        STACK,
        "if (((as_u32(p0) == 0))) { stack_000040_0 = Native.DEMO.GET(p0); } else { stack_000040_0 = Native.DEMO.GET(p1); } Native.DEMO.USE(local_1); local_1 = stack_000040_0; Native.DEMO.USE2(local_1, local_1); return;",
    );
    // A callee handed the local's address may read the old value where no statement names the local.
    stays(
        STACK,
        "if (((as_u32(p0) == 0))) { stack_000040_0 = Native.DEMO.GET(p0); } else { stack_000040_0 = Native.DEMO.GET(p1); } Native.DEMO.USE(Vm.Address(locals, 1)); local_1 = stack_000040_0; Native.DEMO.USE2(local_1, local_1); return;",
    );
    // The next join is given another value, and read, before the copy.
    stays(
        STACK,
        "stack_000040_0 = Native.DEMO.GET(p0); stack_000050_0 = Native.DEMO.GET(p1); Native.DEMO.USE(stack_000050_0); stack_000050_0 = stack_000040_0; Native.DEMO.USE(stack_000050_0); return;",
    );
}

#[test]
fn a_branch_whose_arms_meet_keeps_a_condition_that_calls() {
    becomes(
        STACK,
        "if (((as_u32(Native.DEMO.GET(p0)) == 0))) { stack_000040_0 = p1; } else { stack_000040_0 = p1; } Native.DEMO.USE(stack_000040_0); return;",
        "if (((as_u32(Native.DEMO.GET(p0)) == 0))) { } else { } Native.DEMO.USE(p1); return;",
    );
}

#[test]
fn code_nothing_reaches_goes() {
    becomes("", "return; Native.DEMO.USE(p0); return;", "return;");
}

#[test]
fn a_block_two_blocks_lead_to_stays_a_block() {
    stays(
        "",
        "if (((as_u32(p1) == 0))) { Native.DEMO.USE(p0); } else { Native.DEMO.USE(p1); } Native.DEMO.USE(p0); return;",
    );
}

#[test]
fn every_cell_of_a_value_passed_in_order_is_the_value_passed() {
    let f = ran(
        MANY,
        "v_000018_1 = Native.DEMO.PAIR(); Native.DEMO.USE3(p0, v_000018_1[0], v_000018_1[1]); return;",
    );
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

    // Cells out of their order are not the value.
    stays(
        MANY,
        "v_000018_1 = Native.DEMO.PAIR(); Native.DEMO.USE2(v_000018_1[1], v_000018_1[0]); return;",
    );
}

#[test]
fn locals_that_only_hold_a_value_whole_are_one_variable() {
    let f = ran(
        MANY,
        "v_000018_1 = Native.DEMO.PAIR(); local_0 = v_000018_1[0]; local_1 = v_000018_1[1]; Native.DEMO.USE3(p0, local_0, local_1); Native.DEMO.USE3(p1, local_0, local_1); return;",
    );
    let [Stmt::Assign {
        target: Var::Local(0),
        value: Expr::Call {
            returns: Some(2), ..
        },
    }, Stmt::Eval(Expr::Call { args, .. }), Stmt::Eval(Expr::Call { args: again, .. })] =
        &f.cfg.blocks[0].stmts[..]
    else {
        panic!(
            "an assignment and two calls were expected: {:?}",
            f.cfg.blocks[0].stmts
        );
    };
    assert_eq!(
        args,
        &vec![Expr::Var(Var::Param(0)), Expr::Var(Var::Local(0))]
    );
    assert_eq!(
        again,
        &vec![Expr::Var(Var::Param(1)), Expr::Var(Var::Local(0))]
    );
    let info = &f.head.vars[&Var::Local(0)];
    assert_eq!(
        (info.width, info.kind.clone()),
        (Width::Cells, Kind::Aggregate(2))
    );

    // One cell read apart from the other keeps both locals.
    stays(
        MANY,
        "v_000018_1 = Native.DEMO.PAIR(); local_0 = v_000018_1[0]; local_1 = v_000018_1[1]; Native.DEMO.USE3(p0, local_0, local_1); Native.DEMO.USE(local_1); Native.DEMO.USE(local_0); return;",
    );
}

#[test]
fn a_copy_of_every_local_of_one_run_into_another_is_one_copy() {
    let f = ran(
        MANY,
        // The second run is read before it takes the copy, so it cannot stand in for the first from the start.
        "v_000018_1 = Native.DEMO.PAIR(); local_0 = v_000018_1[0]; local_1 = v_000018_1[1]; Native.DEMO.USE3(p1, local_2, local_3); local_2 = local_0; local_3 = local_1; Native.DEMO.USE3(p0, local_2, local_3); Native.DEMO.USE3(p1, local_0, local_1); return;",
    );
    let [Stmt::Assign {
        target: Var::Local(0),
        value: Expr::Call { .. },
    }, Stmt::Eval(_), Stmt::Assign {
        target: Var::Local(2),
        value: Expr::Var(Var::Local(0)),
    }, Stmt::Eval(Expr::Call { args, .. }), Stmt::Eval(_)] = &f.cfg.blocks[0].stmts[..]
    else {
        panic!(
            "an assignment, a call, a copy and two calls were expected: {:?}",
            f.cfg.blocks[0].stmts
        );
    };
    assert_eq!(
        args,
        &vec![Expr::Var(Var::Param(0)), Expr::Var(Var::Local(2))]
    );
}

#[test]
fn whole_cells_taken_out_of_a_group_are_those_cells() {
    becomes(
        MANY,
        "v_000018_1 = extract_bytes(new VmValues(p0, p1), 8, 8); Native.DEMO.USE(v_000018_1[0]); return;",
        "Native.DEMO.USE(p1); return;",
    );
    // Half of one cell and half of the next are no cell of the group.
    becomes(
        MANY,
        "v_000018_1 = extract_bytes(new VmValues(p0, p1), 4, 8); Native.DEMO.USE(v_000018_1[0]); return;",
        "Native.DEMO.USE(extract_bytes(new VmValues(p0, p1), 4, 8)[0]); return;",
    );
}

#[test]
fn a_many_cell_temporary_of_one_cell_is_the_cell() {
    becomes(
        MANY,
        "v_000018_1 = load_bytes(p0, 0x20, 8); Native.DEMO.USE2(v_000018_1[0], v_000018_1[0]); return;",
        "v_000018_1 = load_u64(p0, 0x20); Native.DEMO.USE2(v_000018_1, v_000018_1); return;",
    );
    // Its address taken, it is memory a pointer reaches.
    stays(
        MANY,
        "v_000018_1 = load_bytes(p0, 0x20, 8); Native.DEMO.USE(Vm.Address(v_000018_1, 0)); Native.DEMO.USE(v_000018_1[0]); return;",
    );
}

#[test]
fn a_field_loaded_stepped_and_stored_back_is_a_step() {
    becomes(
        "",
        "store_i32(p0, 0x10, ((as_i32(load_i32(p0, 0x10)) + as_i32(1)))); return;",
        "increment_i32(p0, 0x10); return;",
    );
    becomes(
        "",
        "local_1 = ((as_i32(local_1) - as_i32(1))); Native.DEMO.USE(local_1); return;",
        "local_1--; Native.DEMO.USE(local_1); return;",
    );
    // Another field's value, one more, is no step of this field.
    stays(
        "",
        "store_i32(p0, 0x10, ((as_i32(load_i32(p0, 0x14)) + as_i32(1)))); return;",
    );
}

#[test]
fn the_function_declares_the_temporaries_it_still_mentions() {
    let folded = ran(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(v_000010_0); return;",
    );
    assert!(folded.head.vars.is_empty(), "{:?}", folded.head.vars);
    let kept = ran(
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE(p1); Native.DEMO.USE(v_000010_0); return;",
    );
    let v = Var::Value { at: 0x10, index: 0 };
    assert!(kept.head.vars.contains_key(&v), "{:?}", kept.head.vars);
}
