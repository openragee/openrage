use std::collections::BTreeMap;

use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName, NativeParam};
use crate::gta6::script::cfg::{Block, Cfg, Function};
use crate::gta6::script::ir::VmType;
use crate::gta6::script::listing::Listing;
use crate::gta6::script::project::{FieldFacts, Param, TypeFacts};
use crate::gta6::script::sample;
use crate::gta6::script::Options;

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
        value_types: Default::default(),
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

/// The names a listing gives three types.
const THING: &str = "type_00000000000000AA";
const OTHER: &str = "type_00000000000000BB";
const THIRD: &str = "type_00000000000000CC";

/// `new T()`.
fn made(name: &str) -> Expr {
    Expr::NewObject(VmType {
        name: name.to_owned(),
        byte_size: 32,
        id: 7,
        flags: 0,
    })
}

fn object(name: &str) -> Kind {
    Kind::Object(name.to_owned())
}

/// A context with `kinds` and `signs` on over `project`.
fn kinded<'a>(names: &'a Names, project: &'a Project) -> Context<'a> {
    let options = Options::baseline().with(Pass::Kinds).with(Pass::Signs);
    Context::new(names, project, options)
}

/// A function of one block that holds `stmts`, typed with `kinds` and `signs` on and `project` at hand.
fn typed_in(stmts: Vec<Stmt>, project: &Project) -> Function {
    let mut f = function(stmts);
    let names = Names::default();
    run_in(&mut f, &kinded(&names, project), &mut Stats::default());
    f
}

/// A project that knows the function at `at` of the tests' program: one cell a parameter, and one cell yielded.
fn signed(at: u32, params: Vec<Kind>, result: Kind) -> Project {
    let params = params.into_iter().map(|kind| Param { kind, cells: 1 });
    let signature = Signature {
        params: params.collect(),
        returns: 1,
        result,
    };
    let mut project = Project::default();
    let key = FunctionKey::new(0x1122_3344, at);
    project.signatures.insert(key, signature);
    project
}

/// A project in which an object of `THING` holds a float at 0x10 and a reference to an `OTHER` at 0x18.
fn laid_out() -> Project {
    let field = |offset: u32, bytes: u32, kind: Kind| FieldFacts {
        offset,
        bytes,
        kind,
    };
    let fields = vec![field(0x10, 4, Kind::F32), field(0x18, 8, object(OTHER))];
    let mut project = Project::default();
    project.fields.insert(TypeKey(0xAA), fields);
    project
}

#[test]
fn references_that_have_nothing_to_do_with_each_other_stay_some_reference() {
    assert_eq!(Lat::Refs.join(&Lat::Is(object(THING))), Lat::Refs);
    assert_eq!(Lat::Is(Kind::Pointer).join(&Lat::Refs), Lat::Refs);
    assert_eq!(Lat::Refs.join(&Lat::Int32), Lat::Conflict);
    assert_eq!(Lat::Refs.join(&Lat::Is(Kind::F32)), Lat::Conflict);
    assert_eq!(Lat::Refs.kind(), Kind::Pointer);
}

#[test]
fn a_signature_says_what_a_call_yields_only_where_kinds_is_on() {
    let project = signed(0x100, vec![], Kind::Bool);
    let tested = || vec![set(0, listed(BinOp::Eq, as_u32(call(vec![])), int(0)))];
    let f = typed_in(tested(), &project);
    assert_eq!(f.cfg.blocks[0].stmts, [set(0, not(call(vec![])))]);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Bool);

    // With the name off the project is not read: the cast is what makes 32 bits of the cell.
    let names = Names::default();
    let off = Context::new(&names, &project, Options::baseline());
    let mut f = function(tested());
    run_in(&mut f, &off, &mut Stats::default());
    assert_eq!(f.cfg.blocks[0].stmts, tested());
}

#[test]
fn a_signature_types_the_arguments_only_of_a_call_it_fits() {
    let project = signed(0x100, vec![Kind::F32, Kind::Bool], Kind::Unknown);
    let f = typed_in(
        vec![
            used(vec![int(0x4170_0000), param(0)]),
            // An argument short of the signature: nothing says which parameter each cell is.
            used(vec![param(1)]),
        ],
        &project,
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [used(vec![float(15.0), param(0)]), used(vec![param(1)]),]
    );
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::Bool);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::Unknown);
}

#[test]
fn a_functions_own_signature_types_its_parameters_and_what_it_returns() {
    let names = Names::default();
    let typed_with = |project: &Project| {
        let mut f = function(vec![set(0, param(2))]);
        f.head.returns = Returns::Cell;
        f.cfg.blocks[0].term = Terminator::Return(Some(int(1)));
        run_in(&mut f, &kinded(&names, project), &mut Stats::default());
        f
    };
    let none = || Kind::Unknown;
    let f = typed_with(&signed(
        0,
        vec![none(), none(), Kind::F32, none()],
        Kind::Bool,
    ));
    assert_eq!(kind_of(&f, Var::Param(2)), Kind::F32);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::F32);
    let yes = Expr::Const(Const::bool(true));
    assert_eq!(f.cfg.blocks[0].term, Terminator::Return(Some(yes)));

    // Three cells of parameters where the function takes four: the signature is another function's.
    let f = typed_with(&signed(0, vec![none(), none(), Kind::F32], Kind::Bool));
    assert_eq!(kind_of(&f, Var::Param(2)), Kind::Unknown);
}

#[test]
fn a_reference_passed_where_a_signature_takes_a_type_is_of_it_unless_an_assignment_names_another() {
    let project = signed(0x100, vec![object(THING)], Kind::Unknown);
    let f = typed_in(
        vec![
            used(vec![param(0)]),
            set(0, made(OTHER)),
            used(vec![local(0)]),
        ],
        &project,
    );
    assert_eq!(kind_of(&f, Var::Param(0)), object(THING));
    // What is passed may derive from what is taken: the object made says more.
    assert_eq!(kind_of(&f, Var::Local(0)), object(OTHER));
}

#[test]
fn a_variable_read_under_both_signs_takes_the_sign_more_reads_take() {
    let stmts = || {
        vec![
            set(2, listed(BinOp::Lt, as_i32(local(0)), as_i32(param(0)))),
            set(3, listed(BinOp::Lt, as_i32(local(0)), as_i32(param(1)))),
            set(4, listed(BinOp::Lt, as_u32(local(0)), as_u32(param(2)))),
        ]
    };
    let f = typed_in(stmts(), Project::empty());
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(2, at(Kind::I32, BinOp::Lt, local(0), param(0))),
            set(3, at(Kind::I32, BinOp::Lt, local(0), param(1))),
            // The one unsigned read keeps the cast that makes it one.
            set(4, at(Kind::U32, BinOp::Lt, as_u32(local(0)), param(2))),
        ]
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);

    // With `signs` off the two readings are a conflict.
    let f = typed_function(stmts(), &Names::default());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
}

#[test]
fn the_sign_an_assignment_gives_stands_against_more_reads_of_the_other() {
    let f = typed_in(
        vec![
            set(0, listed(BinOp::Add, as_i32(param(0)), as_i32(param(1)))),
            set(2, listed(BinOp::Lt, as_u32(local(0)), as_u32(param(2)))),
            set(3, listed(BinOp::Lt, as_u32(local(0)), as_u32(param(3)))),
        ],
        Project::empty(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(
        f.cfg.blocks[0].stmts[1],
        set(2, at(Kind::U32, BinOp::Lt, as_u32(local(0)), param(2)))
    );
}

#[test]
fn variables_copied_into_each_other_take_one_sign_together() {
    // `local_0` is read signed once; `local_1`, which it is copied into, unsigned twice.
    let f = typed_in(
        vec![
            set(1, local(0)),
            set(2, listed(BinOp::Lt, as_i32(local(0)), as_i32(param(0)))),
            set(3, listed(BinOp::Lt, as_u32(local(1)), as_u32(param(1)))),
            set(4, listed(BinOp::Lt, as_u32(local(1)), as_u32(param(2)))),
        ],
        Project::empty(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::U32);
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::U32);
    assert_eq!(
        f.cfg.blocks[0].stmts[1],
        set(2, at(Kind::I32, BinOp::Lt, as_i32(local(0)), param(0)))
    );
}

#[test]
fn a_complement_reads_no_sign() {
    let flipped = || Expr::Unary {
        op: UnOp::BitNot,
        e: Box::new(as_u32(local(0))),
    };
    let f = typed_in(
        vec![
            set(1, listed(BinOp::Lt, as_i32(local(0)), int(5))),
            set(2, flipped()),
            set(3, flipped()),
        ],
        Project::empty(),
    );
    // Counted as reads of a `uint`, the two would outnumber the one read that orders.
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    // The cast under `~` says how wide the result is, and stays.
    assert_eq!(f.cfg.blocks[0].stmts[1], set(2, flipped()));
}

#[test]
fn a_test_of_32_bits_against_zero_reads_no_sign() {
    let unset = || not(as_u32(local(0)));
    let f = typed_in(
        vec![
            set(1, listed(BinOp::Lt, as_i32(local(0)), int(5))),
            set(2, unset()),
            set(3, unset()),
        ],
        Project::empty(),
    );
    // Counted as reads of a `uint`, the two tests would outnumber the one read that orders.
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(
        f.cfg.blocks[0].stmts[0],
        set(1, at(Kind::I32, BinOp::Lt, local(0), int(5)))
    );
}

#[test]
fn a_variable_given_constants_and_read_by_narrow_stores_alone_is_as_wide_as_they_are() {
    let stmts = || {
        vec![
            set(0, int(0x4C48_1189)),
            store(StoreKind::I32, param(0), 0x10, local(0)),
            set(1, int(1)),
            store(StoreKind::U8, param(0), 0x20, local(1)),
            // Passed whole as well: the cell may hold more than its low four bytes.
            set(2, int(7)),
            store(StoreKind::I32, param(0), 0x14, local(2)),
            used(vec![local(2)]),
            // Given a loaded cell: nothing says those four bytes are an integer.
            set(3, load(Scalar::I32, param(1), 8)),
            store(StoreKind::I32, param(0), 0x18, local(3)),
            // Its address handed on: what is called may fill it with anything.
            set(4, int(9)),
            used(vec![address(4)]),
            store(StoreKind::I32, param(0), 0x1C, local(4)),
        ]
    };
    let f = typed_in(stmts(), Project::empty());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::U8);
    assert_eq!(f.cfg.blocks[0].stmts[2], set(1, constant(1, Kind::U8)));
    for n in 2..=4 {
        assert_eq!(kind_of(&f, Var::Local(n)), Kind::Unknown, "local_{n}");
    }

    // With `kinds` off a store's width types nothing.
    let f = typed_function(stmts(), &Names::default());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);
}

#[test]
fn a_layout_types_what_is_loaded_from_a_field_of_an_object_of_known_type() {
    let f = typed_in(
        vec![
            set(0, made(THING)),
            set(1, load(Scalar::U64, local(0), 0x18)),
            store(StoreKind::I32, local(0), 0x10, int(0x4170_0000)),
            // Four bytes where the layout holds eight: another field, or another type.
            set(2, load(Scalar::I32, local(0), 0x18)),
        ],
        &laid_out(),
    );
    assert_eq!(kind_of(&f, Var::Local(1)), object(OTHER));
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::Unknown);
    assert_eq!(
        f.cfg.blocks[0].stmts[2],
        store(StoreKind::I32, local(0), 0x10, float(15.0))
    );
}

#[test]
fn a_layout_says_nothing_of_a_base_that_is_of_one_type_or_of_another() {
    let f = typed_in(
        vec![
            set(0, made(THING)),
            set(0, made(OTHER)),
            set(1, load(Scalar::U64, local(0), 0x18)),
        ],
        &laid_out(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Pointer);
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);
}

#[test]
fn layout_evidence_that_undoes_the_kind_it_rests_on_is_not_taken() {
    // The field's `OTHER`, copied back into the base, would leave the base of no one type.
    let f = typed_in(
        vec![
            set(0, made(THING)),
            set(1, load(Scalar::U64, local(0), 0x18)),
            set(0, local(1)),
        ],
        &laid_out(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), object(THING));
    // What the copy alone says, as with no layout at hand.
    assert_eq!(kind_of(&f, Var::Local(1)), object(THING));
}

#[test]
fn objects_of_unrelated_types_leave_some_reference_and_a_derived_one_its_base() {
    let names = Names::default();
    let kind_in = |project: &Project| {
        let stmts = vec![
            set(0, made(THING)),
            set(0, made(OTHER)),
            set(0, made(THIRD)),
        ];
        let mut f = function(stmts);
        run_in(&mut f, &kinded(&names, project), &mut Stats::default());
        kind_of(&f, Var::Local(0))
    };
    // Not the last of the three.
    assert_eq!(kind_in(Project::empty()), Kind::Pointer);

    let mut project = Project::default();
    for derived in [0xBB, 0xCC] {
        let facts = TypeFacts {
            base: Some(TypeKey(0xAA)),
            ..TypeFacts::default()
        };
        project.types.insert(TypeKey(derived), facts);
    }
    assert_eq!(kind_in(&project), object(THING));
}

#[test]
fn a_global_the_project_types_types_what_is_read_from_it() {
    let global = |offset: u32| {
        Expr::Var(Var::Global {
            program: 0x1122_3344,
            offset,
            statics: false,
        })
    };
    let mut project = Project::default();
    let info = project.programs.entry(0x1122_3344).or_default();
    info.globals.insert(0x48, object(THING));
    let f = typed_in(vec![set(0, global(0x48)), set(1, global(0x50))], &project);
    assert_eq!(kind_of(&f, Var::Local(0)), object(THING));
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);
}

#[test]
fn a_kind_another_pass_wrote_is_evidence_and_goes_where_the_code_disagrees() {
    let stated = |kind: Kind| VarInfo {
        width: Width::Cell,
        kind,
        name: None,
        single: false,
    };
    let mut f = function(vec![
        set(2, listed(BinOp::Lt, as_i32(param(0)), int(5))),
        set(3, listed(BinOp::Mul, as_f32(param(1)), float(2.0))),
        set(4, param(2)),
    ]);
    for n in 0..3 {
        f.head.vars.insert(Var::Param(n), stated(Kind::I32));
    }
    let names = Names::default();
    run_in(
        &mut f,
        &kinded(&names, Project::empty()),
        &mut Stats::default(),
    );
    assert_eq!(kind_of(&f, Var::Param(0)), Kind::I32);
    // Read as a float: neither kind is the parameter's.
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::Unknown);
    // Nothing in the code types the copy but the kind its source was given.
    assert_eq!(kind_of(&f, Var::Local(4)), Kind::I32);
}

#[test]
fn a_value_of_several_cells_keeps_the_type_another_pass_proved() {
    let slot = Var::Value { at: 0x10, index: 0 };
    let proved = Kind::Value {
        ty: THING.to_owned(),
        cells: 3,
    };
    let mut f = function(vec![Stmt::Assign {
        target: slot,
        value: Expr::ZeroSlots(3),
    }]);
    let info = VarInfo {
        width: Width::Cells,
        kind: proved.clone(),
        name: None,
        single: false,
    };
    f.head.vars.insert(slot, info);
    run(&mut f, &Names::default(), &mut Stats::default());
    assert_eq!(kind_of(&f, slot), proved);
}

#[test]
fn a_narrower_variable_read_as_an_int_keeps_its_kind_and_a_bool_its_cast() {
    let stmts = || {
        vec![
            // Above an address handed on, so that no cell here is known to be widened as its kind.
            used(vec![address(0)]),
            set(1, load(Scalar::U8, param(0), 0x20)),
            set(2, listed(BinOp::Lt, as_i32(local(1)), int(3))),
            set(3, listed(BinOp::Lt, as_i32(param(1)), as_i32(param(2)))),
            set(4, listed(BinOp::Add, as_i32(local(3)), int(1))),
        ]
    };
    let f = typed_in(stmts(), Project::empty());
    // ⚠ Read past the byte it is given and not proved to hold no more: no kind, and its cast stays.
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::Unknown);
    assert_eq!(
        f.cfg.blocks[0].stmts[2],
        set(2, at(Kind::I32, BinOp::Lt, as_i32(local(1)), int(3)))
    );
    // C# adds no `bool` to a number: the cast is what makes one of it.
    assert_eq!(kind_of(&f, Var::Local(3)), Kind::Bool);
    assert_eq!(
        f.cfg.blocks[0].stmts[4],
        set(4, at(Kind::I32, BinOp::Add, as_i32(local(3)), int(1)))
    );

    // With `signs` off the read makes an `int` of the variable, as it always did.
    let f = typed_function(stmts(), &Names::default());
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::I32);
}

#[test]
fn a_number_read_past_its_kind_and_not_proved_widened_keeps_no_kind() {
    let ordered = |n: u32| set(n, listed(BinOp::Lt, as_i32(local(0)), int(5)));
    let loaded = || set(0, load(Scalar::U64, param(0), 8));
    // Eight bytes loaded, read as an `int` and handed on whole: an `int` would hide the other 32 bits.
    let f = typed_function(
        vec![loaded(), ordered(1), used(vec![local(0)])],
        &Names::default(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    assert_eq!(f.cfg.blocks[0].stmts[1], ordered(1));

    // Read as an `int` alone, nothing sees more of the cell than an `int` holds.
    let f = typed_function(vec![loaded(), ordered(1)], &Names::default());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(
        f.cfg.blocks[0].stmts[1],
        set(1, at(Kind::I32, BinOp::Lt, local(0), int(5)))
    );

    // Four bytes loaded widen as an `int` does: the whole cell is the `int`.
    let narrow = set(0, load(Scalar::I32, param(0), 8));
    let f = typed_function(
        vec![narrow, ordered(1), used(vec![local(0)])],
        &Names::default(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);

    // A copy of a variable that lost its kind takes a whole cell from it, and loses its own.
    let copied = vec![
        loaded(),
        set(2, local(0)),
        set(3, listed(BinOp::Lt, as_i32(local(2)), int(5))),
        used(vec![local(2)]),
        ordered(1),
    ];
    let f = typed_function(copied, &Names::default());
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::Unknown);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
}

/// `1UL << n`, as a compiler writes a constant of one bit.
fn bit(n: i32) -> Expr {
    at(
        Kind::U64,
        BinOp::Shl,
        constant(1, Kind::U64),
        as_i32(int(n)),
    )
}

#[test]
fn a_constant_of_one_bit_takes_the_kind_of_what_reads_it() {
    let masked = at(Kind::U32, BinOp::And, as_u32(param(0)), as_u32(local(0)));
    let f = typed_function(
        vec![
            set(0, bit(6)),
            set(1, at(Kind::I32, BinOp::Eq, masked, as_i32(local(0)))),
            // Handed on whole and read as nothing: the type the shift is written at stands.
            set(2, bit(10)),
            used(vec![local(2)]),
            set(3, bit(30)),
            set(4, intrinsic("sqrtf", vec![as_f32(local(3))])),
            set(5, intrinsic("sqrtf", vec![bit(30)])),
            // 41 bits are no value of an `int`: the two disagree, and the cast stays.
            set(6, bit(40)),
            set(7, at(Kind::I32, BinOp::Lt, as_i32(local(6)), int(3))),
        ],
        &Names::default(),
    );
    let tested = at(Kind::U32, BinOp::And, param(0), local(0));
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(0, int(64)),
            set(1, at(Kind::I32, BinOp::Eq, tested, local(0))),
            set(2, constant(0x400, Kind::U64)),
            used(vec![local(2)]),
            set(3, float(2.0)),
            set(4, intrinsic("sqrtf", vec![local(3)])),
            set(5, intrinsic("sqrtf", vec![float(2.0)])),
            set(6, constant(1_u64 << 40, Kind::U64)),
            set(7, at(Kind::I32, BinOp::Lt, as_i32(local(6)), int(3))),
        ]
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::U64);
    assert_eq!(kind_of(&f, Var::Local(3)), Kind::F32);
    assert_eq!(kind_of(&f, Var::Local(6)), Kind::Unknown);
}

#[test]
fn a_constant_of_one_bit_read_by_a_narrow_store_alone_is_as_wide_as_the_store() {
    let f = typed_in(
        vec![
            set(0, bit(3)),
            store(StoreKind::U8, param(0), 0x20, local(0)),
        ],
        Project::empty(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::U8);
    assert_eq!(f.cfg.blocks[0].stmts[0], set(0, constant(8, Kind::U8)));
}

#[test]
fn a_variable_given_0_and_1_and_only_tested_against_zero_is_a_truth() {
    let tested = |n: u32, op: BinOp| at(Kind::U32, op, as_u32(local(n)), int(0));
    let branch = |cond: Expr| Terminator::Branch {
        cond,
        then: 0,
        els: 0,
    };
    let stmts = || {
        vec![
            set(0, int(0)),
            set(0, int(1)),
            set(1, tested(0, BinOp::Eq)),
            // Given 0 alone: it may be a reference nothing has set yet.
            set(2, int(0)),
            set(3, tested(2, BinOp::Eq)),
            // Counted with as well as tested: a number.
            set(4, int(0)),
            set(4, int(1)),
            set(5, tested(4, BinOp::Ne)),
            set(6, at(Kind::I32, BinOp::Add, as_i32(local(4)), int(1))),
            // An operand of `&&` is read for whether it is zero too.
            set(7, at(Kind::Bool, BinOp::LogicalAnd, local(0), local(1))),
        ]
    };
    let mut f = function(stmts());
    f.cfg.blocks[0].term = branch(tested(0, BinOp::Ne));
    let names = Names::default();
    run_in(
        &mut f,
        &kinded(&names, Project::empty()),
        &mut Stats::default(),
    );
    let truth = |value: bool| Expr::Const(Const::bool(value));
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            set(0, truth(false)),
            set(0, truth(true)),
            set(1, not(local(0))),
            set(2, int(0)),
            set(3, at(Kind::U32, BinOp::Eq, local(2), int(0))),
            set(4, int(0)),
            set(4, int(1)),
            set(5, at(Kind::U32, BinOp::Ne, local(4), int(0))),
            set(6, at(Kind::I32, BinOp::Add, local(4), int(1))),
            set(7, at(Kind::Bool, BinOp::LogicalAnd, local(0), local(1))),
        ]
    );
    assert_eq!(f.cfg.blocks[0].term, branch(local(0)));
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Bool);
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::I32);
    assert_eq!(kind_of(&f, Var::Local(4)), Kind::I32);

    // With `kinds` off the tests of 32 bits make an `int` of it, as they always did.
    let mut f = function(stmts());
    f.cfg.blocks[0].term = branch(tested(0, BinOp::Ne));
    run(&mut f, &names, &mut Stats::default());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(f.cfg.blocks[0].stmts[0], set(0, int(0)));
}

#[test]
fn the_scale_of_a_vector_is_its_first_argument_and_a_float() {
    let scale = |args: Vec<Expr>| Stmt::Eval(intrinsic("packed_vector_scale", args));
    let f = typed_function(
        vec![
            scale(vec![int(0x3F00_0000), param(0), param(1)]),
            scale(vec![local(0), param(0), as_u32(param(1))]),
        ],
        &Names::default(),
    );
    assert_eq!(
        f.cfg.blocks[0].stmts,
        [
            scale(vec![float(0.5), param(0), param(1)]),
            // The third cell is the vector's second, read whole: a cast of it says something.
            scale(vec![local(0), param(0), as_u32(param(1))]),
        ]
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::F32);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::Unknown);
}

#[test]
fn a_callees_integer_result_is_widened_as_its_signature_says_where_interproc_is_on() {
    let project = signed(0x100, vec![], Kind::I32);
    let names = Names::default();
    let typed_with = |options: Options| {
        let mut f = function(vec![
            set(0, call(vec![])),
            set(1, as_i32(local(0))),
            // An argument the signature does not take: the whole cell is handed on.
            used(vec![local(0)]),
        ]);
        let cx = Context::new(&names, &project, options);
        run_in(&mut f, &cx, &mut Stats::default());
        f
    };
    let kinds = Options::baseline().with(Pass::Kinds);
    let f = typed_with(kinds.with(Pass::Interproc));
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::I32);
    assert_eq!(f.cfg.blocks[0].stmts[1], set(1, local(0)));

    // With `interproc` off the signature names the kind and no more: the cell is not proved to hold only an `int`.
    let f = typed_with(kinds);
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Unknown);
    assert_eq!(f.cfg.blocks[0].stmts[1], set(1, as_i32(local(0))));
}

#[test]
fn a_variable_every_assignment_of_which_makes_an_array_is_an_array_of_its_elements() {
    let array = |name: &str, flags: u32| Expr::NewArray {
        ty: VmType {
            name: name.to_owned(),
            byte_size: 4,
            id: 3,
            flags,
        },
        len: Box::new(int(2)),
        wrapper: String::new(),
    };
    let stmts = vec![
        set(0, array("type_00000000000000AA", 0)),
        set(1, array("type_00000000000000BB", 0x08)),
        // Two arrays of other elements are some reference, and no array of either.
        set(2, array("type_00000000000000AA", 0)),
        set(2, array("type_00000000000000BB", 0x08)),
    ];
    let f = typed_in(stmts.clone(), Project::empty());
    let object = Kind::Object("type_00000000000000AA".to_owned());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Array(Box::new(object)));
    let value = Kind::Value {
        ty: "type_00000000000000BB".to_owned(),
        cells: 1,
    };
    assert_eq!(kind_of(&f, Var::Local(1)), Kind::Array(Box::new(value)));
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::Pointer);
    // With `kinds` off an array is a pointer, as before.
    let f = typed_function(stmts, &Names::default());
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Pointer);
}

#[test]
fn a_value_of_two_cells_only_natives_make_compare_and_take_is_a_handle() {
    let native = |returns: u32, args: Vec<Expr>| Expr::Call {
        callee: Callee::Native {
            namespace: None,
            name: "_0x00000000000000A1".to_owned(),
            hash: 0xA1,
            rva: 0,
        },
        args,
        returns: Some(returns),
    };
    let compared = Expr::Call {
        callee: Callee::Intrinsic {
            name: "bytes_equal_16".to_owned(),
            ty: None,
            wrapper: None,
        },
        args: vec![
            Expr::AddrOf {
                var: Var::Local(0),
                cell: 0,
            },
            Expr::AddrOf {
                var: Var::Local(2),
                cell: 0,
            },
        ],
        returns: Some(1),
    };
    let stmts = vec![
        set(0, native(2, Vec::new())),
        set(2, native(2, Vec::new())),
        Stmt::Eval(native(0, vec![local(0)])),
        set(6, compared),
        // Made by a native, and read a cell at a time.
        set(4, native(2, Vec::new())),
        set(
            7,
            Expr::Cell {
                base: Box::new(local(4)),
                index: 1,
            },
        ),
    ];
    let mut f = function(stmts);
    for n in [0, 2, 4] {
        let mut info = VarInfo::new(Width::Cells);
        info.kind = Kind::Aggregate(2);
        f.head.vars.insert(Var::Local(n), info);
    }
    let names = Names::default();
    run_in(
        &mut f,
        &kinded(&names, Project::empty()),
        &mut Stats::default(),
    );
    assert_eq!(kind_of(&f, Var::Local(0)), Kind::Handle);
    assert_eq!(kind_of(&f, Var::Local(2)), Kind::Handle);
    assert_eq!(kind_of(&f, Var::Local(4)), Kind::Aggregate(2));
}

#[test]
fn a_value_of_several_cells_an_instruction_names_the_type_of_is_a_value_of_that_type() {
    use crate::gta6::script::class::Role;
    use crate::gta6::script::project::Member;
    let (boxed, method) = ("type_00000000000000AA", TypeKey(0xBB));
    let call = |callee: Callee, args: Vec<Expr>| Expr::Call {
        callee,
        args,
        returns: Some(1),
    };
    let address = |n: u32| Expr::AddrOf {
        var: Var::Local(n),
        cell: 0,
    };
    let boxing = |n: u32, ty: &str| {
        let callee = Callee::Intrinsic {
            name: format!("box<{ty}>"),
            ty: None,
            wrapper: None,
        };
        call(callee, vec![local(n)])
    };
    let local_call = |at: u32| Callee::Local {
        at,
        name: format!("func_{at:06X}"),
    };
    let made_in_place = Callee::Builtin {
        path: "Demo.Pair.Ctor".to_owned(),
        ty: 3,
        method: 0,
    };
    let not_made = Callee::Builtin {
        path: "Demo.Pair.Reset".to_owned(),
        ty: 3,
        method: 1,
    };
    let stmts = vec![
        set(6, boxing(0, boxed)),
        set(6, call(local_call(0x40), vec![address(2), int(1)])),
        // Two types named of one variable name neither.
        set(6, boxing(4, boxed)),
        set(6, call(local_call(0x40), vec![address(4)])),
        // A builtin's constructor handed the variable's address.
        set(6, call(made_in_place, vec![address(8), int(2)])),
        // A builtin's method of another name says nothing of its type.
        set(6, call(not_made, vec![address(10)])),
    ];
    let mut f = function(stmts);
    for n in [0, 2, 4, 8, 10] {
        let mut info = VarInfo::new(Width::Cells);
        info.kind = Kind::Aggregate(3);
        f.head.vars.insert(Var::Local(n), info);
    }
    let mut project = Project::default();
    let value = TypeFacts {
        flags: 0x04,
        byte_size: 24,
        ..TypeFacts::default()
    };
    project.types.insert(method, value);
    let member = Member {
        owner: method,
        role: Role::Method,
        is_static: false,
        slot: None,
    };
    project
        .members
        .insert(FunctionKey::new(0x1122_3344, 0x40), member);
    let names = Names::default();
    run_in(&mut f, &kinded(&names, &project), &mut Stats::default());
    let of = |ty: &str| Kind::Value {
        ty: ty.to_owned(),
        cells: 3,
    };
    assert_eq!(kind_of(&f, Var::Local(0)), of(boxed));
    assert_eq!(kind_of(&f, Var::Local(2)), of(&method.to_string()));
    assert_eq!(kind_of(&f, Var::Local(4)), Kind::Aggregate(3));
    assert_eq!(kind_of(&f, Var::Local(8)), of("Demo.Pair"));
    assert_eq!(kind_of(&f, Var::Local(10)), Kind::Aggregate(3));
}

#[test]
fn a_parameter_the_project_proves_an_int_widened_by_its_sign_is_read_as_one_without_a_cast() {
    let reads = || {
        vec![
            set(0, at(Kind::I32, BinOp::Add, as_i32(param(1)), int(1))),
            set(1, as_i32(param(1))),
        ]
    };
    let mut project = Project::default();
    let own = FunctionKey::new(0x1122_3344, 0);
    project
        .widened
        .insert(own, std::collections::BTreeSet::from([1]));
    let names = Names::default();
    let options = Options::baseline()
        .with(Pass::Kinds)
        .with(Pass::Signs)
        .with(Pass::Interproc);
    let typed = |project: &Project| {
        let mut f = function(reads());
        run_in(
            &mut f,
            &Context::new(&names, project, options),
            &mut Stats::default(),
        );
        f
    };
    let f = typed(&project);
    assert_eq!(kind_of(&f, Var::Param(1)), Kind::I32);
    let stmts = &f.cfg.blocks[0].stmts;
    assert_eq!(stmts[1], set(1, param(1)), "{stmts:?}");
    // Of no such proof the reading stays.
    let f = typed(Project::empty());
    assert_eq!(f.cfg.blocks[0].stmts[1], set(1, as_i32(param(1))));
}
