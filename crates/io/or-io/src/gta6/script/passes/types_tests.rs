use std::collections::BTreeMap;

use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName, NativeParam};
use crate::gta6::script::cfg::{Block, Cfg, Function};
use crate::gta6::script::ir::{Head, Returns, VmType};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::sample;

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn float(value: f32) -> Expr {
    Expr::Const(Const::f32(value))
}

fn constant(bits: u64, kind: Kind) -> Expr {
    Expr::Const(Const { bits, kind })
}

fn cast(to: Scalar, e: Expr) -> Expr {
    Expr::cast(to, e)
}

fn as_i32(e: Expr) -> Expr {
    cast(Scalar::I32, e)
}

fn as_u32(e: Expr) -> Expr {
    cast(Scalar::U32, e)
}

fn as_f32(e: Expr) -> Expr {
    cast(Scalar::F32, e)
}

/// `l op r` as a listing's reader types it, from the casts on its operands.
fn listed(op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::binary(op, l, r)
}

/// `l op r` working at `kind`, whatever its operands say of themselves.
fn at(kind: Kind, op: BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Binary {
        op,
        at: kind,
        l: Box::new(l),
        r: Box::new(r),
    }
}

fn not(e: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::Not,
        e: Box::new(e),
    }
}

fn set(n: u32, value: Expr) -> Stmt {
    Stmt::Assign {
        target: Var::Local(n),
        value,
    }
}

fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
    }
}

fn store(kind: StoreKind, addr: Expr, offset: u32, value: Expr) -> Stmt {
    Stmt::Store {
        kind,
        addr,
        offset,
        value,
    }
}

/// `Vm.Address(locals, n)`.
fn address(n: u32) -> Expr {
    Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    }
}

/// `byte_offset(addr, by)`.
fn offset(addr: Expr, by: i32) -> Expr {
    Expr::ByteOffset {
        addr: Box::new(addr),
        offset: Box::new(int(by)),
    }
}

/// A call to a function of the program, of which nothing is known.
fn call(args: Vec<Expr>) -> Expr {
    let callee = Callee::Local {
        at: 0x100,
        name: "func_000100".to_owned(),
    };
    Expr::Call {
        callee,
        args,
        returns: None,
    }
}

fn used(args: Vec<Expr>) -> Stmt {
    Stmt::Eval(call(args))
}

fn intrinsic(name: &str, args: Vec<Expr>) -> Expr {
    let callee = Callee::Intrinsic {
        name: name.to_owned(),
        ty: None,
        wrapper: Some(name.to_owned()),
    };
    Expr::Call {
        callee,
        args,
        returns: Some(1),
    }
}

/// A function of one block that holds `stmts`.
fn function(stmts: Vec<Stmt>) -> Function {
    let head = Head {
        program: 0x1122_3344,
        at: 0,
        name: "func_000000".to_owned(),
        returns: Returns::Void,
        params: 4,
        locals: 8,
        vars: BTreeMap::new(),
        incomplete: None,
    };
    let block = Block {
        at: None,
        stmts,
        term: Terminator::Return(None),
    };
    Function {
        head,
        cfg: Cfg {
            blocks: vec![block],
        },
    }
}

fn typed_function(stmts: Vec<Stmt>, names: &Names) -> Function {
    let mut f = function(stmts);
    run(&mut f, names, &mut Stats::default());
    f
}

/// The statements the pass makes of `stmts`, no names list given.
fn typed(stmts: Vec<Stmt>) -> Vec<Stmt> {
    let mut f = typed_function(stmts, &Names::default());
    f.cfg.blocks.remove(0).stmts
}

fn kind_of(f: &Function, var: Var) -> Kind {
    f.head
        .vars
        .get(&var)
        .map_or(Kind::Unknown, |info| info.kind.clone())
}

#[test]
fn evidence_joins_to_the_wider_integer_and_disagreement_to_no_kind() {
    let is = Lat::Is;
    assert_eq!(is(Kind::Bool).join(&is(Kind::I32)), is(Kind::I32));
    assert_eq!(is(Kind::I32).join(&is(Kind::U8)), is(Kind::I32));
    assert_eq!(Lat::Int32.join(&is(Kind::U32)), is(Kind::U32));
    assert_eq!(Lat::None.join(&is(Kind::F32)), is(Kind::F32));
    assert_eq!(is(Kind::I32).join(&is(Kind::U32)), Lat::Conflict);
    assert_eq!(Lat::Int32.join(&is(Kind::F32)), Lat::Conflict);
    assert_eq!(is(Kind::I8).join(&is(Kind::U32)), Lat::Conflict);
    assert_eq!(Lat::Conflict.join(&Lat::None), Lat::Conflict);
    let object = || is(Kind::Object("type_00000000000000AA".to_owned()));
    assert_eq!(is(Kind::Pointer).join(&object()), object());
    assert_eq!(Lat::Conflict.kind(), Kind::Unknown);
    assert_eq!(Lat::Int32.kind(), Kind::I32);
}

#[test]
fn a_cast_of_a_constant_is_the_constant_of_that_kind() {
    let out = typed(vec![
        set(0, as_f32(int(0x4170_0000))),
        set(1, as_i32(constant(0xFFFF_FFFF_8E7B_3F2C, Kind::U64))),
        set(2, cast(Scalar::U8, int(0x1FF))),
        set(3, as_u32(int(-1))),
    ]);
    assert_eq!(
        out,
        [
            set(0, float(15.0)),
            set(1, constant(0xFFFF_FFFF_8E7B_3F2C, Kind::I32)),
            // The cast keeps the low byte alone, and so must the constant.
            set(2, constant(0xFF, Kind::U8)),
            // An unsigned value has nothing above its 32 bits.
            set(3, constant(0xFFFF_FFFF, Kind::U32)),
        ]
    );
}

#[test]
fn a_cast_that_restates_a_variables_kind_goes_from_under_an_operator() {
    let f = typed_function(
        vec![set(
            2,
            listed(BinOp::Lt, as_i32(local(0)), as_i32(param(0))),
        )],
        &Names::default(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [set(2, at(Kind::I32, BinOp::Lt, local(0), param(0)))]
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::I32);
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::Bool);
}

#[test]
fn a_variable_ordered_under_both_signs_has_no_kind_and_keeps_its_casts() {
    let f = typed_function(
        vec![
            set(2, listed(BinOp::Lt, as_i32(local(0)), as_i32(param(0)))),
            set(3, listed(BinOp::Lt, as_u32(local(0)), as_u32(param(1)))),
        ],
        &Names::default(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(2, at(Kind::I32, BinOp::Lt, as_i32(local(0)), param(0))),
            set(3, at(Kind::U32, BinOp::Lt, as_u32(local(0)), param(1))),
        ]
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::U32);
}

#[test]
fn an_operator_that_reads_either_sign_alike_drops_the_other_signs_cast() {
    let out = typed(vec![
        set(1, listed(BinOp::Lt, as_i32(local(0)), int(5))),
        set(2, listed(BinOp::And, as_u32(local(0)), as_u32(int(4)))),
    ]);
    assert_eq!(
        out,
        [
            set(1, at(Kind::I32, BinOp::Lt, local(0), int(5))),
            set(
                2,
                at(Kind::U32, BinOp::And, local(0), constant(4, Kind::U32))
            ),
        ]
    );
}

#[test]
fn a_cast_goes_where_a_whole_cell_is_taken_only_if_the_cell_is_already_widened_so() {
    let out = typed(vec![
        set(0, listed(BinOp::Add, as_i32(param(0)), as_i32(param(1)))),
        set(1, as_i32(local(0))),
        set(2, as_i32(param(0))),
    ]);
    assert_eq!(
        out,
        [
            set(0, at(Kind::I32, BinOp::Add, param(0), param(1))),
            set(1, local(0)),
            // A caller filled the parameter's cell: the cast is what makes an `int` of it.
            set(2, as_i32(param(0))),
        ]
    );
}

#[test]
fn a_variable_that_may_be_written_through_its_address_keeps_its_cast() {
    let stmts = vec![
        set(0, listed(BinOp::Add, as_i32(param(0)), as_i32(param(1)))),
        used(vec![address(0)]),
        set(1, as_i32(local(0))),
    ];
    assert_eq!(
        typed(stmts),
        [
            set(0, at(Kind::I32, BinOp::Add, param(0), param(1))),
            used(vec![address(0)]),
            set(1, as_i32(local(0))),
        ]
    );
}

#[test]
fn a_store_looks_through_a_cast_that_keeps_every_byte_it_writes() {
    let out = typed(vec![
        store(StoreKind::I32, param(0), 0x10, as_i32(param(1))),
        store(StoreKind::I32, param(0), 0x14, cast(Scalar::U8, param(2))),
    ]);
    assert_eq!(
        out,
        [
            store(StoreKind::I32, param(0), 0x10, param(1)),
            // One byte of the four: the cast clears the other three.
            store(StoreKind::I32, param(0), 0x14, cast(Scalar::U8, param(2))),
        ]
    );
}

#[test]
fn a_cast_under_a_cast_no_wider_goes_and_a_narrowing_one_stays() {
    let out = typed(vec![
        set(0, as_f32(as_i32(param(0)))),
        set(1, as_i32(cast(Scalar::U8, param(1)))),
    ]);
    assert_eq!(
        out,
        [set(0, as_f32(param(0))), set(1, cast(Scalar::U8, param(1))),]
    );
}

#[test]
fn a_zero_test_of_a_comparison_is_the_comparison_or_its_opposite() {
    let less = || listed(BinOp::Lt, as_i32(param(0)), as_i32(param(1)));
    let out = typed(vec![
        set(0, listed(BinOp::Eq, as_u32(less()), int(0))),
        set(1, listed(BinOp::Ne, as_u32(less()), int(0))),
    ]);
    assert_eq!(
        out,
        [
            set(0, at(Kind::I32, BinOp::Ge, param(0), param(1))),
            set(1, at(Kind::I32, BinOp::Lt, param(0), param(1))),
        ]
    );
}

#[test]
fn a_negated_ordering_of_floats_keeps_its_negation() {
    let less = listed(BinOp::Lt, as_f32(param(0)), as_f32(param(1)));
    let out = typed(vec![set(0, listed(BinOp::Eq, as_u32(less), int(0)))]);
    // `!(a < b)` holds for a NaN, which `a >= b` does not.
    assert_eq!(
        out,
        [set(0, not(at(Kind::F32, BinOp::Lt, param(0), param(1))))]
    );
}

#[test]
fn a_zero_test_reads_by_the_kind_of_what_it_tests() {
    let out = typed(vec![
        set(0, listed(BinOp::Lt, as_i32(param(0)), int(5))),
        set(1, listed(BinOp::Eq, as_u32(local(0)), int(0))),
        set(2, listed(BinOp::Ne, as_u32(local(0)), int(0))),
        set(3, listed(BinOp::Eq, as_u32(param(0)), int(0))),
        set(4, listed(BinOp::Mul, as_f32(param(1)), float(2.0))),
        set(5, listed(BinOp::Eq, as_u32(param(1)), int(0))),
    ]);
    assert_eq!(
        out,
        [
            set(0, at(Kind::I32, BinOp::Lt, param(0), int(5))),
            set(1, not(local(0))),
            set(2, local(0)),
            // An `int` is compared with zero, not negated.
            set(3, at(Kind::U32, BinOp::Eq, param(0), int(0))),
            // Read as a float and as an integer, the parameter has no kind: both casts stay.
            set(4, at(Kind::F32, BinOp::Mul, as_f32(param(1)), float(2.0))),
            set(5, at(Kind::U32, BinOp::Eq, as_u32(param(1)), int(0))),
        ]
    );
}

#[test]
fn a_ternary_on_truth_reads_its_condition_alone() {
    let select = |cond: Expr| Expr::Select {
        cond: Box::new(cond),
        then: Box::new(int(7)),
        els: Box::new(int(9)),
    };
    let equal = listed(BinOp::Eq, as_i32(param(0)), as_i32(int(5)));
    let listed_cond = listed(BinOp::Ne, equal, Expr::Truth(Box::new(int(0))));
    let out = typed(vec![set(0, select(listed_cond))]);
    assert_eq!(
        out,
        [set(0, select(at(Kind::I32, BinOp::Eq, param(0), int(5))))]
    );
}

#[test]
fn a_constant_takes_the_kind_of_the_variable_it_is_assigned_to() {
    let out = typed(vec![
        set(0, int(0x4170_0000)),
        set(4, listed(BinOp::Mul, as_f32(local(0)), float(2.0))),
        set(1, listed(BinOp::Lt, as_i32(param(0)), int(3))),
        set(1, int(1)),
        set(2, listed(BinOp::Add, as_i32(param(0)), int(1))),
        set(2, int(1)),
    ]);
    assert_eq!(
        out,
        [
            set(0, float(15.0)),
            set(4, at(Kind::F32, BinOp::Mul, local(0), float(2.0))),
            set(1, at(Kind::I32, BinOp::Lt, param(0), int(3))),
            set(1, Expr::Const(Const::bool(true))),
            set(2, at(Kind::I32, BinOp::Add, param(0), int(1))),
            // The same 1 assigned to an `int` stays a number.
            set(2, int(1)),
        ]
    );
}

#[test]
fn a_float_literal_that_is_an_integer_is_written_as_one_unless_a_float_reads_it() {
    let out = typed(vec![
        used(vec![constant(3, Kind::F32)]),
        used(vec![constant(0xFFFF_FFFF, Kind::F32)]),
        set(0, constant(3, Kind::F32)),
        set(1, listed(BinOp::Mul, as_f32(local(0)), float(2.0))),
        set(2, constant(0xFFFF_FFFF, Kind::F32)),
        set(3, listed(BinOp::Lt, as_i32(local(2)), int(0))),
    ]);
    assert_eq!(
        out,
        [
            used(vec![int(3)]),
            // All ones with nothing above them in the cell: the unsigned number is the cell.
            used(vec![constant(0xFFFF_FFFF, Kind::U32)]),
            // What a float variable takes stays the float the listing wrote.
            set(0, constant(3, Kind::F32)),
            set(1, at(Kind::F32, BinOp::Mul, local(0), float(2.0))),
            set(2, constant(0xFFFF_FFFF, Kind::I32)),
            set(3, at(Kind::I32, BinOp::Lt, local(2), int(0))),
        ]
    );
}

#[test]
fn a_field_of_an_embedded_struct_is_one_offset_unless_the_struct_is_also_a_value() {
    let out = typed(vec![
        set(0, load(Scalar::I32, offset(param(0), 0x28), 4)),
        used(vec![offset(param(1), 0x28)]),
        set(1, load(Scalar::I32, offset(param(1), 0x28), 4)),
    ]);
    assert_eq!(
        out,
        [
            set(0, load(Scalar::I32, param(0), 0x2C)),
            used(vec![offset(param(1), 0x28)]),
            set(1, load(Scalar::I32, offset(param(1), 0x28), 4)),
        ]
    );
}

#[test]
fn a_load_read_as_another_type_of_its_width_is_a_load_of_that_type() {
    let out = typed(vec![
        set(0, as_f32(load(Scalar::I32, param(0), 0x10))),
        set(1, cast(Scalar::U8, load(Scalar::I32, param(0), 0x14))),
    ]);
    assert_eq!(
        out,
        [
            set(0, load(Scalar::F32, param(0), 0x10)),
            // A byte of the four is no field of its own: the load keeps its width.
            set(1, cast(Scalar::U8, load(Scalar::I32, param(0), 0x14))),
        ]
    );
}

#[test]
fn a_local_reached_through_its_own_address_is_read_and_written_by_name() {
    let out = typed(vec![
        store(StoreKind::I32, address(3), 0, int(7)),
        set(0, load(Scalar::I32, address(3), 0)),
        Stmt::ZeroBytes {
            addr: address(4),
            bytes: 8,
        },
        set(1, load(Scalar::U64, address(4), 0)),
    ]);
    assert_eq!(
        out,
        [
            set(3, int(7)),
            set(0, as_i32(local(3))),
            set(4, int(0)),
            set(1, local(4)),
        ]
    );
}

#[test]
fn a_write_narrower_than_a_read_of_the_local_stays_a_write_through_its_address() {
    let stmts = vec![
        store(StoreKind::I32, address(3), 0, int(7)),
        used(vec![local(3)]),
        Stmt::ZeroBytes {
            addr: address(5),
            bytes: 4,
        },
        used(vec![local(5)]),
    ];
    // Four bytes written and eight read: an assignment would clear what the store leaves.
    assert_eq!(typed(stmts.clone()), stmts);
}

#[test]
fn an_address_that_leaves_the_function_keeps_every_access_through_it() {
    let stmts = vec![
        used(vec![address(3)]),
        store(StoreKind::U64, address(3), 0, int(7)),
        set(0, load(Scalar::I32, address(3), 0)),
    ];
    assert_eq!(typed(stmts.clone()), stmts);
}

#[test]
fn a_shift_of_two_constants_is_one_constant_and_a_shift_by_a_variable_stays() {
    let one = || constant(1, Kind::U64);
    let out = typed(vec![
        set(0, listed(BinOp::Shl, one(), as_i32(int(10)))),
        set(1, listed(BinOp::Shl, one(), as_i32(param(0)))),
    ]);
    assert_eq!(
        out,
        [
            set(0, constant(0x400, Kind::U64)),
            set(1, at(Kind::U64, BinOp::Shl, one(), param(0))),
        ]
    );
}

#[test]
fn a_mask_that_clears_a_bit_is_written_by_the_bit_it_clears() {
    let out = typed(vec![
        set(0, listed(BinOp::And, as_u32(param(0)), as_u32(int(-17)))),
        set(1, listed(BinOp::And, as_u32(param(0)), as_u32(int(0xFF)))),
    ]);
    let cleared = Expr::Unary {
        op: UnOp::BitNot,
        e: Box::new(constant(0x10, Kind::U32)),
    };
    assert_eq!(
        out,
        [
            set(0, at(Kind::U32, BinOp::And, param(0), cleared)),
            // A mask that keeps a byte is read as what it keeps.
            set(
                1,
                at(Kind::U32, BinOp::And, param(0), constant(0xFF, Kind::U32))
            ),
        ]
    );
}

/// A list that names one native, `BOOL SET_SPEED(float speed, BOOL now)`.
fn one_native() -> Names {
    let takes = |ty: &str, name: &str| NativeParam {
        ty: ty.to_owned(),
        name: name.to_owned(),
    };
    let native = NativeName {
        namespace: Some("DEMO".to_owned()),
        name: "SET_SPEED".to_owned(),
        params: vec![takes("float", "speed"), takes("BOOL", "now")],
        returns: Some("BOOL".to_owned()),
    };
    Names {
        natives: Some(NativeDb::from_names("a test's", [(0xA1, native)])),
        ..Names::default()
    }
}

fn native(args: Vec<Expr>) -> Expr {
    let callee = Callee::Native {
        namespace: Some("DEMO".to_owned()),
        name: "SET_SPEED".to_owned(),
        hash: 0xA1,
        rva: 0x0102_0304,
    };
    Expr::Call {
        callee,
        args,
        returns: Some(1),
    }
}

#[test]
fn a_natives_list_types_what_a_native_takes_and_yields() {
    let f = typed_function(
        vec![
            set(
                0,
                listed(
                    BinOp::Eq,
                    as_u32(native(vec![int(0x4170_0000), int(1)])),
                    int(0),
                ),
            ),
            Stmt::Eval(native(vec![param(0), param(1)])),
            Stmt::Eval(native(vec![int(0x4170_0000), int(1), int(2)])),
        ],
        &one_native(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(
                0,
                not(native(vec![float(15.0), Expr::Const(Const::bool(true))]))
            ),
            Stmt::Eval(native(vec![param(0), param(1)])),
            // A cell more than the signature takes: the list says nothing of this call's arguments.
            Stmt::Eval(native(vec![int(0x4170_0000), int(1), int(2)])),
        ]
    );
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::F32);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::Bool);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Bool);
}

#[test]
fn without_a_list_a_native_types_nothing() {
    let stmts = vec![Stmt::Eval(native(vec![int(0x4170_0000), int(1)]))];
    assert_eq!(typed(stmts.clone()), stmts);
}

#[test]
fn a_truth_value_stands_for_its_own_test_only_where_the_test_alone_is_taken() {
    let answer = || native(vec![param(0), param(1)]);
    let tested = || listed(BinOp::Ne, as_u32(answer()), int(0));
    let branch = |cond: Expr| Terminator::Branch {
        cond,
        then: 0,
        els: 0,
    };
    let mut f = function(vec![set(0, tested())]);
    f.cfg.blocks[0].term = branch(tested());
    run(&mut f, &one_native(), &mut Stats::default());
    // A branch takes only whether the native's answer is zero.
    assert_eq!(f.cfg.blocks[0].term, branch(answer()));
    // A variable takes the cell, and nothing says the native yields 0 or 1 and no more.
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [set(0, at(Kind::U32, BinOp::Ne, answer(), int(0)))]
    );
}

#[test]
fn an_intrinsic_reads_its_arguments_as_its_name_says() {
    let f = typed_function(
        vec![
            set(0, intrinsic("sqrtf", vec![as_f32(param(0))])),
            set(1, as_f32(local(0))),
            set(2, intrinsic("example_gate", vec![as_f32(param(1))])),
        ],
        &Names::default(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(0, intrinsic("sqrtf", vec![param(0)])),
            set(1, local(0)),
            // A name that says nothing of its arguments takes whole cells: the cast stays.
            set(2, intrinsic("example_gate", vec![as_f32(param(1))])),
        ]
    );
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::F32);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::F32);
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::Unknown);
}

#[test]
fn kinds_flow_through_copies_and_back_to_a_variable_nothing_else_types() {
    let scaled = |by: f32| listed(BinOp::Mul, as_f32(param(0)), float(by));
    let f = typed_function(
        vec![
            set(0, scaled(2.0)),
            set(1, local(0)),
            set(2, local(1)),
            set(3, param(1)),
            set(3, scaled(3.0)),
            set(4, param(2)),
            set(4, scaled(4.0)),
            set(5, param(2)),
            set(5, listed(BinOp::Add, as_i32(param(3)), int(1))),
        ],
        &Names::default(),
    );
    for n in 0..=4 {
        assert_eq!(kind_of(&f, Var::Local(n)), Kind::F32, "local_{n}");
    }
    assert_eq!(kind_of(&f, Var::Local(5)), Kind::I32);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::F32);
    // Copied into a float and into an `int`: nothing says which it is.
    assert_eq!(kind_of(&f, Var::Param(2)), Kind::Unknown);
}

#[test]
fn an_increment_says_only_that_its_variable_is_32_bits_and_keeps_its_shape() {
    let step = |n: u32| set(n, listed(BinOp::Add, as_i32(local(n)), int(1)));
    let f = typed_function(
        vec![
            step(0),
            set(1, listed(BinOp::Lt, as_u32(local(0)), as_u32(param(0)))),
            step(2),
            set(3, listed(BinOp::Lt, as_i32(local(2)), int(9))),
        ],
        &Names::default(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            step(0),
            set(1, at(Kind::U32, BinOp::Lt, local(0), param(0))),
            // The cast restates the kind here, and the statement still reads back as `local_2++`.
            step(2),
            set(3, at(Kind::I32, BinOp::Lt, local(2), int(9))),
        ]
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::U32);
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::I32);
}

#[test]
fn what_an_access_goes_through_is_a_reference() {
    let made = || {
        Expr::NewObject(VmType {
            name: "type_00000000000000AA".to_owned(),
            byte_size: 16,
            id: 7,
            flags: 0,
        })
    };
    let f = typed_function(
        vec![
            set(0, made()),
            store(StoreKind::Ref, local(0), 8, int(0)),
            store(StoreKind::I32, param(0), 0x10, int(3)),
        ],
        &Names::default(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(0, made()),
            store(StoreKind::Ref, local(0), 8, constant(0, Kind::Pointer)),
            store(StoreKind::I32, param(0), 0x10, int(3)),
        ]
    );
    let object = Kind::Object("type_00000000000000AA".to_owned());
    assert_eq!(kind_of(&f, Var::Local(0)), object);
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::Pointer);
}

#[test]
fn a_value_of_several_cells_takes_the_kind_of_what_makes_it() {
    let value = |index: u32| Var::Value { at: 0x10, index };
    let assign = |index: u32, value_of: Expr| Stmt::Assign {
        target: value(index),
        value: value_of,
    };
    let vector = Expr::Vector3(Box::new([float(1.0), float(2.0), float(3.0)]));
    let field = Expr::LoadBytes {
        addr: Box::new(param(0)),
        offset: 0x20,
        bytes: 12,
    };
    let mut f = function(vec![
        assign(0, vector),
        assign(1, Expr::ZeroSlots(3)),
        assign(2, field),
    ]);
    for index in 0..3 {
        f.head.vars.insert(value(index), VarInfo::new(Width::Cells));
    }
    run(&mut f, &Names::default(), &mut Stats::default());
    assert_eq!(kind_of(&f, value(0)), Kind::Vector);
    assert_eq!(kind_of(&f, value(1)), Kind::Aggregate(3));
    assert_eq!(kind_of(&f, value(2)), Kind::Aggregate(2));
    assert_eq!(f.head.vars[&value(0)].width, Width::Cells);
}

#[test]
fn a_cell_zeroed_and_filled_with_four_bytes_is_the_value_cut_to_them() {
    let slot = Var::Value { at: 0x50, index: 0 };
    let made = || Stmt::Assign {
        target: slot,
        value: Expr::ZeroSlots(1),
    };
    let at_slot = || Expr::AddrOf { var: slot, cell: 0 };
    let filled = |value: Expr| store(StoreKind::I32, at_slot(), 0, value);
    let read = || Expr::Cell {
        base: Box::new(Expr::Var(slot)),
        index: 0,
    };
    let cut = |stmts: Vec<Stmt>| {
        let mut f = function(stmts);
        f.head.vars.insert(slot, VarInfo::new(Width::Cells));
        run(&mut f, &Names::default(), &mut Stats::default());
        f
    };

    // A variable nothing else can change is read where the cell was read.
    let f = cut(vec![made(), filled(param(1)), used(vec![param(0), read()])]);
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [used(vec![param(0), as_u32(param(1))])]
    );
    assert!(!f.head.vars.contains_key(&slot));

    // A call must run where it stood, before the call the next statement makes first: the
    // cell stays a variable, now of one cell.
    let f = cut(vec![
        made(),
        filled(call(vec![])),
        used(vec![call(vec![]), read()]),
    ]);
    let kept = Stmt::Assign {
        target: slot,
        value: as_u32(call(vec![])),
    };
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [kept, used(vec![call(vec![]), Expr::Var(slot)])]
    );
    assert_eq!(f.head.vars[&slot].width, Width::Cell);

    // Its address handed on, the cell is memory, and stays so.
    let memory = vec![
        made(),
        filled(param(1)),
        used(vec![read()]),
        used(vec![at_slot()]),
    ];
    assert_eq!(cut(memory.clone()).cfg.blocks[0].stmts, memory);
}

#[test]
fn an_expression_nested_too_deep_leaves_its_function_as_it_was() {
    let mut deep = as_i32(param(0));
    for _ in 0..MAX_DEPTH + 8 {
        deep = Expr::Truth(Box::new(deep));
    }
    // The second statement would fold to a constant had the pass gone on.
    let stmts = vec![set(0, deep), set(1, as_i32(int(5)))];
    assert_eq!(typed(stmts.clone()), stmts);
}

#[test]
fn the_sample_listing_types_with_every_statement_kept() {
    let listing = Listing::open(sample::LISTING).expect("the sample opens");
    let mut functions = Vec::new();
    for (f, statements) in listing.functions().zip(sample::STATEMENTS) {
        let f = f.expect("the function reads");
        let mut lowered = cfg::lower(f).expect("the function lowers");
        if lowered.head.incomplete.is_none() {
            run(&mut lowered, &Names::default(), &mut Stats::default());
        }
        assert_eq!(lowered.cfg.statements(), statements);
        functions.push(lowered);
    }
    assert_eq!(functions.len(), sample::STATEMENTS.len());

    // `func_000200` counts `local_0` up to 10 and switches on what a native yields, less one.
    let counter = &functions[2];
    let result = Var::Value {
        at: 0x204,
        index: 0,
    };
    let test = at(Kind::I32, BinOp::Ge, local(0), int(10));
    let switched = at(Kind::I32, BinOp::Sub, Expr::Var(result), int(1));
    let terms = || counter.cfg.blocks.iter().map(|block| &block.term);
    assert!(
        terms().any(|term| matches!(term, Terminator::Branch { cond, .. } if *cond == test)),
        "{:?}",
        counter.cfg
    );
    assert!(
        terms().any(|term| matches!(term, Terminator::Switch { value, .. } if *value == switched)),
        "{:?}",
        counter.cfg
    );
    assert_eq!(kind_of(counter, Var::Local(0)), Kind::I32);
    assert_eq!(kind_of(counter, result), Kind::I32);
}
