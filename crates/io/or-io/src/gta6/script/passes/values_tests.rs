use super::*;
use crate::gta6::script::ir::{Callee, Expr, Kind, Stmt, Var, Width};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::{Options, Project};

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

/// The blocks [`run_in`] makes of `body` with `passes` on beside the four first.
fn ran_with(passes: &[Pass], decls: &str, body: &str) -> cfg::Function {
    let mut f = read(decls, body);
    let options = (passes.iter()).fold(Options::baseline(), |options, pass| options.with(*pass));
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), options);
    run_in(&mut f, &cx, &mut Stats::default());
    f
}

/// Asserts that the pass, with `passes` on, makes of `before` the blocks `after` reads as.
fn becomes_with(passes: &[Pass], decls: &str, before: &str, after: &str) {
    let (made, wanted) = (ran_with(passes, decls, before).cfg, read(decls, after).cfg);
    assert_eq!(made, wanted, "from:\n{before}");
}

fn stays_with(passes: &[Pass], decls: &str, body: &str) {
    let (made, wanted) = (ran_with(passes, decls, body).cfg, read(decls, body).cfg);
    assert_eq!(made, wanted, "from:\n{body}");
}

/// A new object.
const NEW: &str = "Vm.NewObject(new VmType(\"T\"))";

#[test]
fn a_load_moves_past_a_store_into_an_object_in_no_other_hands() {
    // The arguments of an inlined constructor are loaded first, then stored one after another.
    let filled = |first: &str, value: &str| {
        format!(
            "v_000010_0 = {NEW}; {first} store_i32(v_000010_0, 0x10, 5); store_i32(v_000010_0, 0x18, {value}); Native.DEMO.USE(v_000010_0); return;"
        )
    };
    becomes_with(
        &[Pass::Escape],
        V2,
        &filled("v_000018_1 = load_i32(p0, 0x8);", "v_000018_1"),
        &filled("", "load_i32(p0, 0x8)"),
    );
    // Without the name a store is a store: the load stays before it.
    stays(V2, &filled("v_000018_1 = load_i32(p0, 0x8);", "v_000018_1"));
    // Stored where anything may find it, the object may be what `p0` points at.
    stays_with(
        &[Pass::Escape],
        V2,
        &format!(
            "v_000010_0 = {NEW}; store_ref(p1, 0x0, v_000010_0); v_000018_1 = load_i32(p0, 0x8); store_i32(v_000010_0, 0x10, 5); store_i32(v_000010_0, 0x18, v_000018_1); return;"
        ),
    );
}

#[test]
fn a_load_moves_past_a_store_to_other_bytes_through_the_same_pointer() {
    becomes_with(
        &[Pass::Escape],
        V,
        "v_000010_0 = load_i32(p0, 0x8); store_i32(p0, 0x10, 5); store_i32(p0, 0x18, v_000010_0); return;",
        "store_i32(p0, 0x10, 5); store_i32(p0, 0x18, load_i32(p0, 0x8)); return;",
    );
    // The same bytes: the load reads what the store wrote.
    stays_with(
        &[Pass::Escape],
        V,
        "v_000010_0 = load_i32(p0, 0x10); store_i32(p0, 0x10, 5); store_i32(p0, 0x18, v_000010_0); return;",
    );
    // Another pointer may be the same pointer.
    stays_with(
        &[Pass::Escape],
        V,
        "v_000010_0 = load_i32(p0, 0x8); store_i32(p1, 0x10, 5); store_i32(p0, 0x18, v_000010_0); return;",
    );
}

#[test]
fn a_call_and_an_allocation_change_places() {
    let before = format!(
        "v_000010_0 = Native.DEMO.GET(p0); v_000018_1 = {NEW}; Native.DEMO.USE2(v_000018_1, v_000010_0); return;"
    );
    becomes_with(
        &[Pass::Escape],
        V2,
        &before,
        &format!("Native.DEMO.USE2({NEW}, Native.DEMO.GET(p0)); return;"),
    );
    // Without the name the call keeps its place before the allocation.
    becomes(
        V2,
        &before,
        &format!("v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2({NEW}, v_000010_0); return;"),
    );
    // A load is not an allocation: the call may write what it reads.
    stays_with(
        &[Pass::Escape],
        V,
        "v_000010_0 = Native.DEMO.GET(p0); Native.DEMO.USE2(load_i32(p1, 0x8), v_000010_0); return;",
    );
}

#[test]
fn a_test_of_order_between_constants_goes_one_way() {
    let test = |cond: &str| {
        format!("if ({cond}) {{ Native.DEMO.USE(1); }} else {{ Native.DEMO.USE(2); }} return;")
    };
    let (first, second) = ("Native.DEMO.USE(1); return;", "Native.DEMO.USE(2); return;");
    let ordered = &[Pass::Constants];
    becomes_with(ordered, "", &test("((as_i32(5) < as_i32(7)))"), first);
    // As `int` the first is minus one; as `uint` it is the greatest there is.
    becomes_with(
        ordered,
        "",
        &test("((as_i32(0xFFFFFFFF) < as_i32(7)))"),
        first,
    );
    becomes_with(
        ordered,
        "",
        &test("((as_u32(0xFFFFFFFF) < as_u32(7)))"),
        second,
    );
    becomes_with(
        ordered,
        "",
        &test("((as_f32(1.5f) >= as_f32(0.5f)))"),
        first,
    );
    // No number is below, above or equal to anything: `!(a < b)` holds where `a >= b` does not.
    let nan = "as_f32(float_bits(2143289344))";
    becomes_with(
        ordered,
        "",
        &test(&format!("(({nan} >= as_f32(0.5f)))")),
        second,
    );
    becomes_with(
        ordered,
        "",
        &test(&format!("!(({nan} < as_f32(0.5f)))")),
        first,
    );
    // One side a variable, the test is a test.
    stays_with(ordered, "", &test("((as_i32(p0) < as_i32(7)))"));
}

#[test]
fn a_switch_on_a_constant_goes_to_its_case() {
    becomes_with(
        &[Pass::Constants],
        "",
        "switch (as_i32(1)) { case 0: Native.DEMO.USE(1); break; case 1: Native.DEMO.USE(2); break; default: Native.DEMO.USE(3); break; } return;",
        "Native.DEMO.USE(2); return;",
    );
    stays_with(
        &[Pass::Constants],
        "",
        "switch (as_i32(p0)) { case 0: Native.DEMO.USE(1); break; case 1: Native.DEMO.USE(2); break; default: Native.DEMO.USE(3); break; } return;",
    );
}

#[test]
fn a_local_that_copies_a_temporary_across_blocks_keeps_its_name() {
    let body = |first: &str, copy: &str, read: &str| {
        format!(
            "{first} = Native.DEMO.GET(p0); if (((as_u32(p1) == 0))) {{ Native.DEMO.USE({read}); }} else {{ }} {copy} Native.DEMO.USE2(local_1, local_1); return;"
        )
    };
    let made = ran_with(
        &[Pass::Coalesce],
        V,
        &body("v_000010_0", "local_1 = v_000010_0;", "p1"),
    );
    assert_eq!(made.cfg, read(V, &body("local_1", "", "p1")).cfg);
    // Without the name the temporary stands in for the local.
    let plain = ran(V, &body("v_000010_0", "local_1 = v_000010_0;", "p1"));
    assert!(plain
        .head
        .vars
        .contains_key(&Var::Value { at: 0x10, index: 0 }));
    // Read before it takes the copy, the local still holds what it had: it cannot take the value any earlier.
    stays_with(
        &[Pass::Coalesce],
        V,
        &body("v_000010_0", "local_1 = v_000010_0;", "local_1"),
    );
}

/// Whether any block of `f` still ends in a test.
fn branches(f: &cfg::Function) -> bool {
    let test = |block: &Block| matches!(block.term, Terminator::Branch { .. });
    f.cfg.blocks.iter().any(test)
}

#[test]
fn a_test_of_the_variable_the_statement_before_gave_a_constant_goes_one_way() {
    let body = |tested: &str| {
        format!(
            "local_1 = 0; if (((as_u64({tested}) != 0))) {{ Native.DEMO.USE(1); }} else {{ local_1 = Native.DEMO.GET(p0); }} Native.DEMO.USE2(local_1, local_1); return;"
        )
    };
    let constants = &[Pass::Constants];
    // The arm that runs is kept, with what follows it: the other, and its call of one argument, go.
    let made = ran_with(constants, "", &body("local_1"));
    assert!(!branches(&made), "{:?}", made.cfg);
    let [block] = &made.cfg.blocks[..] else {
        panic!("one block was expected: {:?}", made.cfg);
    };
    let got = |stmt: &Stmt| {
        matches!(
            stmt,
            Stmt::Assign {
                target: Var::Local(1),
                value: Expr::Call { .. }
            }
        )
    };
    let dead = |stmt: &Stmt| matches!(stmt, Stmt::Eval(Expr::Call { args, .. }) if args.len() == 1);
    assert!(block.stmts.iter().any(got), "{:?}", block.stmts);
    assert!(!block.stmts.iter().any(dead), "{:?}", block.stmts);
    // Without `constants` the test is a test.
    stays("", &body("local_1"));
    // Another variable is tested: nothing says what it holds.
    assert!(branches(&ran_with(constants, "", &body("p1"))));
    // A statement stands between the constant and the test.
    let apart = "local_1 = 0; Native.DEMO.USE(p1); if (((as_u64(local_1) != 0))) { Native.DEMO.USE(1); } else { local_1 = Native.DEMO.GET(p0); } Native.DEMO.USE2(local_1, local_1); return;";
    assert!(branches(&ran_with(constants, "", apart)));
}

#[test]
fn a_local_a_statement_of_its_own_assigns_is_proved_one_cell_wide() {
    let single = |decls: &str, body: &str| -> Vec<u32> {
        let proved = effects::single_cells(&read(decls, body));
        proved.into_iter().collect()
    };
    // `local_2` and `local_3` are assigned in a row, the lowest first, as the cells of one value are.
    let row = "local_1 = p0; v_000018_1 = Native.DEMO.PAIR(); local_2 = v_000018_1[0]; local_3 = v_000018_1[1]; local_5 = p1; return;";
    assert_eq!(single(MANY, row), [1, 5]);
    // The higher one first is no row.
    assert_eq!(single("", "local_3 = p0; local_2 = p1; return;"), [2, 3]);
    // A target of a tuple is never proved, whatever else assigns it.
    let tuple = "(local_1, local_3) = (p1, p0); local_3 = p0; local_6 = p1; return;";
    assert_eq!(single("", tuple), [6]);
    // An access through an address that runs over a cell assigned alone contradicts the proof.
    let over = "local_3 = p0; local_6 = p1; zero_bytes(Vm.Address(locals, 2), 16); return;";
    assert!(single("", over).is_empty());
    // So does an address moved a stated count of bytes into such a cell.
    let moved = "local_3 = p0; Native.DEMO.USE(byte_offset(Vm.Address(locals, 2), 0x8)); return;";
    assert!(single("", moved).is_empty());
    // And the same address moved once a local holds it.
    let held = "local_3 = p0; local_6 = Vm.Address(locals, 2); Native.DEMO.USE(byte_offset(local_6, 0x8)); return;";
    assert!(single("", held).is_empty());
    // A cell assigned alone here and as a cell of a row there is proved nothing, and neither is its neighbour.
    let both = "local_3 = p0; Native.DEMO.USE(p1); v_000018_1 = Native.DEMO.PAIR(); local_2 = v_000018_1[0]; local_3 = v_000018_1[1]; local_5 = p1; return;";
    assert_eq!(single(MANY, both), [5]);
}

/// The locals of the frame an address may reach, by what [`effects::Facts`] makes of `f`.
fn reached(f: &cfg::Function) -> Vec<u32> {
    let facts = effects::Facts::of(f);
    let reach = |n: &u32| !facts.named_only(Var::Local(*n));
    (0..8).filter(reach).collect()
}

#[test]
fn an_address_handed_out_reaches_its_variable_and_not_the_locals_declared_after_it() {
    let handed = |before: &str, address: &str| {
        format!(
            "{before} Native.DEMO.USE({address}); Native.DEMO.USE3(local_3, local_4, local_5); return;"
        )
    };
    let own = "Vm.Address(locals, 2)";
    let mut f = read("", &handed("local_3 = p0; local_5 = p1;", own));
    // With no width declared the address reaches every local from its own on.
    assert_eq!(reached(&f), [2, 3, 4, 5, 6, 7]);
    widths(&mut f);
    // `local_3` is a variable of its own: the one at `local_2` ends before it.
    assert_eq!(reached(&f), [2]);

    // A cell never assigned alone bounds nothing: the variable may run on to the next that is.
    let mut f = read("", &handed("local_5 = p1;", own));
    widths(&mut f);
    assert_eq!(reached(&f), [2, 3, 4]);

    // The local whose address goes out is itself one cell wide.
    let mut f = read("", &handed("local_2 = p0;", own));
    widths(&mut f);
    assert_eq!(reached(&f), [2]);

    // Moved by a count nothing states, the address may come out anywhere above.
    let adrift = "byte_offset(Vm.Address(locals, 2), p1)";
    let mut f = read("", &handed("local_3 = p0; local_5 = p1;", adrift));
    widths(&mut f);
    assert_eq!(reached(&f), [2, 3, 4, 5, 6, 7]);

    // So may one a local holds first, moved by such a count or copied on.
    let held = "local_3 = p0; local_5 = p1; local_6 = Vm.Address(locals, 2);";
    let mut f = read("", &handed(held, "byte_offset(local_6, p1)"));
    widths(&mut f);
    assert_eq!(reached(&f), [2, 3, 4, 5, 6, 7]);
    let copied = format!("{held} local_7 = local_6;");
    let mut f = read("", &handed(&copied, "local_7"));
    widths(&mut f);
    assert_eq!(reached(&f), [2, 3, 4, 5, 6, 7]);
    // Handed on as it is, the held address reaches what the address in place does.
    let mut f = read("", &handed(held, "local_6"));
    widths(&mut f);
    assert_eq!(reached(&f), [2]);

    // The kind a later pass gives a local takes nothing from the proof.
    let mut f = read("", &handed("local_3 = p0; local_5 = p1;", own));
    widths(&mut f);
    if let Some(info) = f.head.vars.get_mut(&Var::Local(3)) {
        info.kind = Kind::I32;
    }
    assert_eq!(reached(&f), [2]);
    // A local another pass declares a cell wide is no proof of a width.
    let mut f = read("", &handed("local_5 = p1;", own));
    widths(&mut f);
    let declared = VarInfo::new(Width::Cell);
    f.head.vars.insert(Var::Local(3), declared);
    assert_eq!(reached(&f), [2, 3, 4]);
}

/// The blocks the pass makes of `body` once [`widths`] has declared what the form read proves, with `passes` on beside the four first.
fn ran_declared(passes: &[Pass], decls: &str, body: &str) -> cfg::Function {
    let mut f = read(decls, body);
    widths(&mut f);
    let options = (passes.iter()).fold(Options::baseline(), |options, pass| options.with(*pass));
    let names = Names::default();
    let cx = Context::new(&names, Project::empty(), options);
    run_in(&mut f, &cx, &mut Stats::default());
    f
}

#[test]
fn an_assignment_nothing_reads_goes_where_no_address_reaches_its_local() {
    let body =
        "local_3 = ((as_i32(p0) + as_i32(1))); Native.DEMO.USE(Vm.Address(locals, 2)); return;";
    // Nothing declared, the local may be a cell of the variable whose address is handed out.
    stays("", body);
    let made = ran_declared(&[Pass::Escape], "", body);
    let wanted = read("", "Native.DEMO.USE(Vm.Address(locals, 2)); return;");
    assert_eq!(made.cfg, wanted.cfg);
    // Assigned in a row with its neighbour, it is not proved to be apart, and stays.
    let row = "local_3 = ((as_i32(p0) + as_i32(1))); local_4 = p1; Native.DEMO.USE(Vm.Address(locals, 2)); return;";
    assert_eq!(
        ran_declared(&[Pass::Escape], "", row).cfg,
        read("", row).cfg
    );
    let assigns = |f: &cfg::Function, n: u32| {
        let to = |stmt: &&Stmt| stmt.targets().contains(&Var::Local(n));
        (f.cfg.blocks.iter())
            .map(|block| block.stmts.iter().filter(to).count())
            .sum::<usize>()
    };
    // Assigned alone and then as the upper cell of a row, it may be the cell the address reaches: both stay.
    let both = "local_3 = p0; Native.DEMO.USE(p1); v_000018_1 = Native.DEMO.PAIR(); local_2 = v_000018_1[0]; local_3 = v_000018_1[1]; Native.DEMO.USE(Vm.Address(locals, 2)); return;";
    assert_eq!(assigns(&ran_declared(&[Pass::Escape], MANY, both), 3), 2);
    // An address a local holds and then moves into the cell reaches it as the address in place does.
    let held = "local_3 = ((as_i32(p0) + as_i32(1))); local_6 = Vm.Address(locals, 2); Native.DEMO.USE(byte_offset(local_6, 0x8)); return;";
    assert_eq!(assigns(&ran_declared(&[Pass::Escape], "", held), 3), 1);
}

#[test]
fn a_run_of_locals_that_holds_one_declared_a_cell_wide_stays_locals() {
    // Without the declaration these two locals are one variable of two cells.
    let body = "v_000018_1 = Native.DEMO.PAIR(); local_0 = v_000018_1[0]; local_1 = v_000018_1[1]; Native.DEMO.USE3(p0, local_0, local_1); Native.DEMO.USE3(p1, local_0, local_1); return;";
    let mut f = read(MANY, body);
    let mut proved = VarInfo::new(Width::Cell);
    proved.single = true;
    f.head.vars.insert(Var::Local(1), proved);
    run(&mut f, &Names::default(), &mut Stats::default());
    let wide = |info: &VarInfo| matches!(info.kind, Kind::Aggregate(_));
    assert!(!f.head.vars.values().any(wide), "{:?}", f.head.vars);
}

/// A branch on the first parameter whose arms hold `yes` and `no`, and `after` where they meet.
fn two_arms(yes: &str, no: &str, after: &str) -> String {
    format!("if (((as_u32(p0) != 0))) {{ {yes} }} else {{ {no} }} {after} return;")
}

/// Both readers of the first stack variable, so that it stays a variable.
const READ_TWICE: &str = "Native.DEMO.USE2(stack_000040_0, stack_000040_0);";

#[test]
fn two_arms_that_each_only_give_one_variable_a_value_are_one_choice() {
    let body = two_arms(
        "stack_000040_0 = p1;",
        "stack_000040_0 = load_i32(local_3, 0x8);",
        READ_TWICE,
    );
    let mut f = read(STACK, &body);
    let Terminator::Branch { cond, then, els } = f.cfg.blocks[0].term.clone() else {
        panic!("a branch was expected: {:?}", f.cfg.blocks[0].term);
    };
    let value_of = |f: &cfg::Function, arm: BlockId| match &f.cfg.blocks[arm].stmts[..] {
        [Stmt::Assign { value, .. }] => value.clone(),
        other => panic!("one assignment was expected: {other:?}"),
    };
    let (yes, no) = (value_of(&f, then), value_of(&f, els));
    let Terminator::Jump(join) = f.cfg.blocks[then].term else {
        panic!("a jump was expected: {:?}", f.cfg.blocks[then].term);
    };
    assert!(joins::selects(&mut f));
    let wanted = Stmt::Assign {
        target: Var::Stack { at: 0x40, index: 0 },
        value: Expr::Select {
            cond: Box::new(cond),
            then: Box::new(yes),
            els: Box::new(no),
        },
    };
    assert_eq!(f.cfg.blocks[0].stmts, [wanted]);
    assert_eq!(f.cfg.blocks[0].term, Terminator::Jump(join));

    // Through the whole pass with `coalesce` on, the arms are gone and the choice is one statement.
    let made = ran_with(&[Pass::Coalesce], STACK, &body);
    assert!(!branches(&made), "{:?}", made.cfg);
    let chosen = |stmt: &Stmt| {
        matches!(
            stmt,
            Stmt::Assign {
                value: Expr::Select { .. },
                ..
            }
        )
    };
    let all = made.cfg.blocks.iter().flat_map(|block| &block.stmts);
    assert_eq!(all.filter(|stmt| chosen(stmt)).count(), 1, "{:?}", made.cfg);
    // Without the name the branch is a branch.
    stays(STACK, &body);
}

#[test]
fn arms_that_do_more_than_choose_a_value_stay_arms() {
    let stays_arms = |decls: &str, body: &str| {
        let mut f = read(decls, body);
        let before = f.cfg.clone();
        assert!(!joins::selects(&mut f), "from:\n{body}");
        assert_eq!(f.cfg, before, "from:\n{body}");
    };
    let plain = "stack_000040_0 = local_3;";
    // A call is made where its arm runs, and stays a statement of that arm.
    let call = "stack_000040_0 = Native.DEMO.GET(p1);";
    stays_arms(STACK, &two_arms(call, plain, READ_TWICE));
    // An arm that holds another statement.
    let more = "Native.DEMO.USE(p1); stack_000040_0 = p1;";
    stays_arms(STACK, &two_arms(more, plain, READ_TWICE));
    // The arms assign two variables.
    let other = "stack_000050_0 = p1;";
    stays_arms(STACK, &two_arms(other, plain, READ_TWICE));
    // Two truths are the value of a condition.
    let (one, zero) = ("stack_000040_0 = 1;", "stack_000040_0 = 0;");
    stays_arms(STACK, &two_arms(one, zero, READ_TWICE));
    // Where the arms meet only tests the variable: the arms hold the parts of a condition.
    let test = "if (((as_u32(stack_000040_0) != 0))) { Native.DEMO.USE(p1); } else { }";
    stays_arms(STACK, &two_arms("stack_000040_0 = p1;", plain, test));
    // A value of several cells is no arm of a choice.
    let pair = "v_000018_1 = zero_slots(2);";
    let read_pair = "Native.DEMO.USE2(v_000018_1[0], v_000018_1[1]);";
    stays_arms(MANY, &two_arms(pair, pair, read_pair));
}

#[test]
fn a_clear_that_states_more_cells_than_the_frame_holds_makes_no_variable() {
    // ⚠ The count is the program's: a run that wide is no variable of the frame, and is never
    // walked a cell at a time.
    let body = "zero_bytes(Vm.Address(locals, 6), 0x7FFFFFF8); Native.DEMO.USE(Vm.Address(locals, 6)); return;";
    let f = ran("", body);
    let wide = |info: &VarInfo| matches!(info.kind, Kind::Aggregate(_));
    assert!(!f.head.vars.values().any(wide), "{:?}", f.head.vars);
    assert_eq!(f.cfg, read("", body).cfg);
    // The two cells the frame holds from there are one variable, cleared as one.
    let body =
        "zero_bytes(Vm.Address(locals, 6), 16); Native.DEMO.USE(Vm.Address(locals, 6)); return;";
    let f = ran("", body);
    assert!(f.head.vars.values().any(wide), "{:?}", f.head.vars);
}

#[test]
fn one_cell_read_through_the_address_of_a_variable_of_several_is_not_the_variable() {
    // The three locals are cleared and read as one value, so they are one variable. The load
    // takes two of its cells and the call one of those: by the variable's name it would take all three.
    let body = "zero_bytes(Vm.Address(locals, 0), 24); Native.DEMO.USE3(local_0, local_1, local_2); v_000018_1 = load_bytes(Vm.Address(locals, 0), 0x0, 16); Native.DEMO.USE(v_000018_1[0]); return;";
    let f = ran(MANY, body);
    let last = f.cfg.blocks.iter().flat_map(|block| &block.stmts).last();
    let Some(Stmt::Eval(Expr::Call { args, .. })) = last else {
        panic!("the last statement is the call: {last:?}");
    };
    assert!(
        args.len() == 1 && effects::one_cell(&f.head, &args[0]),
        "{args:?}"
    );
}
