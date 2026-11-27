use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName};
use crate::gta6::script::ir::VarInfo;
use crate::gta6::script::sample;
use crate::gta6::script::tree::Label;

pub(super) fn head() -> Head {
    Head {
        program: 0x1122_3344,
        at: 0x40,
        name: "func_000040".to_owned(),
        returns: Returns::Void,
        params: 1,
        locals: 2,
        vars: BTreeMap::new(),
        incomplete: None,
    }
}

/// A head whose variables have the kinds given.
pub(super) fn kinds(of: &[(Var, Kind)]) -> Head {
    let mut head = head();
    for (var, kind) in of {
        let mut info = VarInfo::new(Width::Cell);
        info.kind = kind.clone();
        head.vars.insert(*var, info);
    }
    head
}

fn text_in(head: &Head, names: &Names, e: &Expr, plain: bool) -> String {
    let mut out = String::new();
    Writer::new(&mut out, plain, names, head).expr(e, 0)
}

fn text_of(e: &Expr, plain: bool) -> String {
    text_in(&head(), &Names::default(), e, plain)
}

fn readable(e: &Expr) -> String {
    text_of(e, false)
}

/// `e` as the condition of an `if`.
fn cond_of(e: &Expr) -> String {
    let (head, names, mut out) = (head(), Names::default(), String::new());
    Writer::new(&mut out, false, &names, &head).cond(e, 0)
}

/// A simple statement of the readable form, without its semicolon.
fn stmt_of(s: &Stmt) -> String {
    let (head, names, mut out) = (head(), Names::default(), String::new());
    Writer::new(&mut out, false, &names, &head).simple(s)
}

fn written(f: &tree::Function, names: &Names) -> String {
    let mut out = String::new();
    function(&mut out, f, names);
    out
}

pub(super) fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

pub(super) fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

pub(super) fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

pub(super) fn constant_of(bits: u64, kind: Kind) -> Expr {
    Expr::Const(Const { bits, kind })
}

pub(super) fn not(e: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::Not,
        e: Box::new(e),
    }
}

pub(super) fn truth(e: Expr) -> Expr {
    Expr::Truth(Box::new(e))
}

/// `(int)local_0 < (int)local_1`, as a listing compares two cells.
fn less() -> Expr {
    Expr::binary(
        BinOp::Lt,
        Expr::cast(Scalar::I32, local(0)),
        Expr::cast(Scalar::I32, local(1)),
    )
}

pub(super) fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
    }
}

pub(super) fn address(var: Var, cell: u32) -> Expr {
    Expr::AddrOf { var, cell }
}

fn offset(addr: Expr, by: Expr) -> Expr {
    Expr::ByteOffset {
        addr: Box::new(addr),
        offset: Box::new(by),
    }
}

pub(super) fn intrinsic(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Intrinsic {
            name: name.to_owned(),
            ty: None,
            wrapper: None,
        },
        args,
        returns: Some(1),
    }
}

pub(super) fn native(name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Callee::Native {
            namespace: Some("DEMO".to_owned()),
            name: name.to_owned(),
            hash: 0xA1,
            rva: 0,
        },
        args,
        returns: Some(0),
    }
}

pub(super) fn assign(target: Var, value: Expr) -> Stmt {
    Stmt::Assign { target, value }
}

fn read(text: &str) -> (Listing<'_>, Vec<tree::Function>) {
    let listing = Listing::open(text).expect("the listing opens");
    let functions = listing
        .functions()
        .collect::<Result<_, _>>()
        .expect("every function reads");
    (listing, functions)
}

#[test]
fn what_is_read_and_written_plainly_reads_back_the_same() {
    let (listing, functions) = read(sample::LISTING);
    let mut writer = ListingWriter::new(listing.program, &listing.imports);
    for f in &functions {
        writer.function(f);
    }
    let written = writer.finish();

    let (again, read_back) = read(&written);
    assert_eq!(again.program, listing.program);
    assert_eq!(again.imports, listing.imports);
    assert_eq!(again.declarations().len(), listing.declarations().len());
    for (path, wrapper) in listing.declarations().iter() {
        assert_eq!(again.declarations().get(path), Some(wrapper), "{path}");
    }
    assert_eq!(read_back.len(), functions.len());
    for (back, first) in read_back.iter().zip(&functions) {
        assert_eq!(back, first, "{} in:\n{written}", first.head.name);
    }
}

#[test]
fn parentheses_are_written_only_where_binding_needs_them() {
    let (a, b, c) = (local(0), local(1), local(2));
    let binary = |op, l: &Expr, r: &Expr| Expr::binary(op, l.clone(), r.clone());
    let sum = binary(BinOp::Add, &a, &b);
    let cases = [
        (
            binary(BinOp::Mul, &sum, &c),
            "(local_0 + local_1) * local_2",
        ),
        (binary(BinOp::Add, &sum, &c), "local_0 + local_1 + local_2"),
        (
            binary(BinOp::Add, &c, &sum),
            "local_2 + (local_0 + local_1)",
        ),
        (
            binary(BinOp::Sub, &a, &binary(BinOp::Mul, &b, &c)),
            "local_0 - local_1 * local_2",
        ),
        (
            binary(BinOp::And, &binary(BinOp::Eq, &a, &b), &c),
            "local_0 == local_1 & local_2",
        ),
        (
            binary(BinOp::Eq, &a, &binary(BinOp::And, &b, &c)),
            "local_0 == (local_1 & local_2)",
        ),
        (
            binary(BinOp::Shl, &binary(BinOp::Shl, &a, &b), &c),
            "local_0 << local_1 << local_2",
        ),
        (binary(BinOp::Sub, &a, &int(-1)), "local_0 - (-1)"),
        (
            Expr::Unary {
                op: UnOp::Neg,
                e: Box::new(Expr::Unary {
                    op: UnOp::Neg,
                    e: Box::new(a.clone()),
                }),
            },
            "-(-local_0)",
        ),
        (not(binary(BinOp::Lt, &a, &b)), "!(local_0 < local_1)"),
        (
            Expr::Cell {
                base: Box::new(Expr::Cells(vec![a.clone(), b.clone()])),
                index: 1,
            },
            "new VmValues(local_0, local_1)[1]",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(text_of(e, true), *text);
    }
    assert_eq!(
        text_of(&int(-1), true),
        "-1",
        "nothing stands before it to run into"
    );

    // The readable form keeps a sign off the operator before it, and nowhere else.
    assert_eq!(
        readable(&binary(BinOp::Sub, &a, &int(-1))),
        "local_0 - (-1)"
    );
    assert_eq!(readable(&binary(BinOp::Eq, &a, &int(-1))), "local_0 == -1");
    assert_eq!(readable(&binary(BinOp::Mul, &sum, &c)), cases[0].1);
}

#[test]
fn the_plain_form_spells_what_the_listing_spells() {
    let p0 = param(0);
    let cases = [
        (Expr::cast(Scalar::U16, p0.clone()), "as_u16(p0)"),
        (truth(int(0)), "Truth(0)"),
        (load(Scalar::I32, p0.clone(), 0x1C), "load_i32(p0, 0x1C)"),
        (address(Var::Local(1), 0), "Vm.Address(locals, 1)"),
        (
            address(Var::Value { at: 0x96, index: 1 }, 2),
            "Vm.Address(v_000096_1, 2)",
        ),
        (
            Expr::Var(Var::Global {
                program: 0xAB,
                offset: 0x48,
                statics: false,
            }),
            "Vm.Globals(0x000000ABU)[0x48]",
        ),
        (Expr::Const(Const::f32(15.0)), "15.0f"),
        (Expr::Const(Const::f32(-0.5)), "-0.5f"),
        (Expr::Const(Const::f32(f32::from_bits(1))), "1e-45f"),
        (
            Expr::Const(Const::f32(f32::from_bits(0xFFFF_FFFF))),
            "float_bits(0xFFFFFFFF)",
        ),
        (
            constant_of(0xFFFF_FFFF_8000_0001, Kind::U64),
            "0xFFFFFFFF80000001UL",
        ),
        (constant_of(4_000_000_000, Kind::U32), "4000000000U"),
        (
            Expr::Select {
                cond: Box::new(p0.clone()),
                then: Box::new(int(1)),
                els: Box::new(int(2)),
            },
            "p0 ? (VmValue)(1) : (VmValue)(2)",
        ),
        (
            Expr::NewObject(VmType {
                name: "a\"b".to_owned(),
                byte_size: 16,
                id: 7,
                flags: 0x100,
            }),
            "Vm.NewObject(new VmType(\"a\\\"b\", 16, 7U, 0x00000100U))",
        ),
        // The plain form states every reading, the 64-bit ones too.
        (
            Expr::binary(BinOp::Eq, Expr::cast(Scalar::U64, p0.clone()), int(0)),
            "as_u64(p0) == 0",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(text_of(e, true), *text);
    }
}

#[test]
fn the_readable_form_spells_what_c_sharp_would() {
    let p0 = param(0);
    let cases = [
        (Expr::cast(Scalar::I32, p0.clone()), "(int)p0"),
        (
            Expr::binary(BinOp::Add, Expr::cast(Scalar::F32, p0.clone()), int(2)),
            "(float)p0 + 2",
        ),
        (
            Expr::cast(Scalar::U32, Expr::binary(BinOp::Add, p0.clone(), int(2))),
            "(uint)(p0 + 2)",
        ),
        (address(Var::Local(1), 0), "&local_1"),
        (
            Expr::Var(Var::Global {
                program: 0x1122_3344,
                offset: 0x48,
                statics: false,
            }),
            "Global_48",
        ),
        (
            Expr::Var(Var::Global {
                program: 0xAB,
                offset: 0x48,
                statics: true,
            }),
            "Static_000000AB_48",
        ),
        (Expr::Const(Const::f32(15.0)), "15.0f"),
        (Expr::Const(Const::bool(true)), "true"),
        (int(-1), "-1"),
        (int(0x1234_ABCD), "0x1234ABCD"),
        (constant_of(0x1234_5678, Kind::Hash), "0x12345678"),
        (constant_of(u64::MAX, Kind::U64), "0xFFFFFFFFFFFFFFFF"),
        (constant_of(7, Kind::U64), "7"),
        (Expr::GuidEmpty, "Guid.Empty"),
        (
            Expr::Vector3(Box::new([int(1), int(2), int(3)])),
            "new Vector3(1, 2, 3)",
        ),
        (
            Expr::NewObject(VmType {
                name: "type_0123456789ABCDEF".to_owned(),
                byte_size: 16,
                id: 7,
                flags: 0x100,
            }),
            "new type_0123456789ABCDEF()",
        ),
        // A name a listing states in a string may be anything: what is no name stays a string.
        (
            Expr::NewObject(VmType {
                name: "no name\n".to_owned(),
                byte_size: 16,
                id: 7,
                flags: 0x100,
            }),
            "new \"no name\\u000A\"()",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(readable(e), *text);
    }
}

#[test]
fn a_cast_that_says_nothing_is_not_written() {
    let typed = kinds(&[
        (Var::Param(0), Kind::I32),
        (Var::Local(5), Kind::Hash),
        (Var::Local(6), Kind::Bool),
    ]);
    let none = Names::default();
    let (p0, l0) = (param(0), local(0));
    let cast = Expr::cast;
    let cases = [
        // A value of the very kind the cast reads it as.
        (cast(Scalar::I32, p0.clone()), "p0"),
        // The same bits under another sign are another value to a comparison: the cast stays.
        (cast(Scalar::U32, p0.clone()), "(uint)p0"),
        // ⚠ An integer's bits read as a float are no conversion, which a C# cast would be.
        (cast(Scalar::F32, p0.clone()), "as_f32(p0)"),
        (cast(Scalar::I32, l0.clone()), "(int)local_0"),
        (cast(Scalar::U32, local(5)), "local_5"),
        (cast(Scalar::U8, local(5)), "(byte)local_5"),
        (cast(Scalar::U32, local(6)), "local_6"),
        (cast(Scalar::F32, local(6)), "as_f32(local_6)"),
        // A reading of as many bits under this one is not seen by it.
        (
            cast(Scalar::I32, cast(Scalar::I32, l0.clone())),
            "(int)local_0",
        ),
        (
            cast(Scalar::F32, cast(Scalar::I32, l0.clone())),
            "(float)local_0",
        ),
        (
            cast(Scalar::U8, cast(Scalar::U64, l0.clone())),
            "(byte)local_0",
        ),
        // A reading of fewer bits under it is: it cuts the value and gives it a sign.
        (
            cast(Scalar::I64, cast(Scalar::I32, l0.clone())),
            "(long)(int)local_0",
        ),
        (
            cast(Scalar::I32, load(Scalar::I32, l0.clone(), 0x10)),
            "(int)local_0->f_10",
        ),
        (
            cast(Scalar::I32, load(Scalar::U8, l0.clone(), 0x10)),
            "(int)(byte)local_0->f_10",
        ),
        (cast(Scalar::U32, less()), "(int)local_0 < (int)local_1"),
        (
            cast(Scalar::F32, less()),
            "as_f32((int)local_0 < (int)local_1)",
        ),
        // A constant under a cast is the constant the cast reads.
        (cast(Scalar::I32, int(5)), "5"),
        (cast(Scalar::F32, int(0x3F00_0000)), "0.5f"),
        (cast(Scalar::F32, int(0)), "0.0f"),
        (
            cast(Scalar::I32, constant_of(0xFFFF_FFFF_9ABC_DEF0, Kind::U64)),
            "0x9ABCDEF0",
        ),
        (cast(Scalar::U32, int(-2)), "0xFFFFFFFE"),
        (cast(Scalar::I32, constant_of(1, Kind::F32)), "1"),
        (cast(Scalar::U8, int(0x1FF)), "255"),
        (cast(Scalar::I8, int(0x1FF)), "-1"),
        (
            cast(Scalar::I32, constant_of(0x1234_5678, Kind::Hash)),
            "0x12345678",
        ),
        (cast(Scalar::U32, Expr::Const(Const::bool(true))), "true"),
    ];
    for (e, text) in &cases {
        assert_eq!(text_in(&typed, &none, e, false), *text);
    }
}

#[test]
fn a_64_bit_reading_is_not_written_where_only_the_bits_count() {
    let wide = |e: Expr| Expr::cast(Scalar::U64, e);
    let signed = |e: Expr| Expr::cast(Scalar::I64, e);
    let cases = [
        (
            Expr::binary(BinOp::Eq, wide(wide(local(0))), int(0)),
            "local_0 == 0",
        ),
        (
            Expr::binary(BinOp::Or, wide(local(0)), wide(int(2))),
            "local_0 | 2",
        ),
        (
            Expr::binary(BinOp::Add, signed(local(0)), signed(int(-1))),
            "local_0 + (-1)",
        ),
        // The 32 bits under a 64-bit reading are still read as 32.
        (
            Expr::binary(
                BinOp::Ne,
                wide(Expr::cast(Scalar::I32, local(0))),
                wide(local(1)),
            ),
            "(int)local_0 != local_1",
        ),
        // To an order, a division and a shift to the right the sign of the cells counts.
        (
            Expr::binary(BinOp::Lt, wide(local(0)), wide(local(1))),
            "(ulong)local_0 < (ulong)local_1",
        ),
        (
            Expr::binary(BinOp::Div, signed(local(0)), signed(local(1))),
            "(long)local_0 / (long)local_1",
        ),
        (
            Expr::binary(BinOp::Shr, signed(local(0)), int(5)),
            "(long)local_0 >> 5",
        ),
        // A 32-bit reading cuts the cell, whatever the operator.
        (
            Expr::binary(
                BinOp::Add,
                Expr::cast(Scalar::U32, local(0)),
                Expr::cast(Scalar::U32, local(1)),
            ),
            "(uint)local_0 + (uint)local_1",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(readable(e), *text);
    }
    assert_eq!(
        stmt_of(&assign(Var::Local(0), wide(local(1)))),
        "local_0 = local_1"
    );
    assert_eq!(
        stmt_of(&assign(Var::Local(0), Expr::cast(Scalar::I32, local(1)))),
        "local_0 = (int)local_1"
    );
    let call = native("USE", vec![wide(local(0)), signed(int(3))]);
    assert_eq!(stmt_of(&Stmt::Eval(call)), "DEMO.USE(local_0, 3)");
}

#[test]
fn a_condition_is_written_as_a_condition() {
    let float_less = || {
        Expr::binary(
            BinOp::Lt,
            Expr::cast(Scalar::F32, local(0)),
            Expr::cast(Scalar::F32, local(1)),
        )
    };
    let float_same = Expr::binary(
        BinOp::Eq,
        Expr::cast(Scalar::F32, local(0)),
        Expr::cast(Scalar::F32, local(1)),
    );
    let zero_test = |op, of: Expr| Expr::binary(op, Expr::cast(Scalar::U32, of), int(0));
    let cases = [
        (truth(Expr::Const(Const::bool(true))), "true"),
        (truth(int(0)), "false"),
        (truth(local(0)), "local_0"),
        (not(truth(local(0))), "!local_0"),
        (not(not(truth(local(0)))), "local_0"),
        (less(), "(int)local_0 < (int)local_1"),
        (not(less()), "(int)local_0 >= (int)local_1"),
        // ⚠ An order of floats is not turned round: neither holds for a NaN.
        (not(float_less()), "!((float)local_0 < (float)local_1)"),
        (not(float_same), "(float)local_0 != (float)local_1"),
        (zero_test(BinOp::Eq, less()), "(int)local_0 >= (int)local_1"),
        (zero_test(BinOp::Ne, less()), "(int)local_0 < (int)local_1"),
        (
            Expr::binary(BinOp::Ne, less(), truth(int(0))),
            "(int)local_0 < (int)local_1",
        ),
        (
            zero_test(BinOp::Eq, float_less()),
            "!((float)local_0 < (float)local_1)",
        ),
        // A cell of no known kind tested in 32 bits is not a truth: the test stays as it is.
        (zero_test(BinOp::Eq, local(0)), "(uint)local_0 == 0"),
        (zero_test(BinOp::Ne, local(0)), "(uint)local_0 != 0"),
        (
            Expr::binary(BinOp::Eq, less(), int(1)),
            "(int)local_0 < (int)local_1 == 1",
        ),
        (
            Expr::Binary {
                op: BinOp::LogicalAnd,
                at: Kind::Bool,
                l: Box::new(truth(local(0))),
                r: Box::new(not(less())),
            },
            "local_0 && (int)local_0 >= (int)local_1",
        ),
        (
            not(Expr::Binary {
                op: BinOp::LogicalOr,
                at: Kind::Bool,
                l: Box::new(truth(local(0))),
                r: Box::new(less()),
            }),
            "!(local_0 || (int)local_0 < (int)local_1)",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(cond_of(e), *text);
    }

    // Outside a condition the value is the truth of the cell, never the cell.
    assert_eq!(readable(&truth(local(0))), "local_0 != 0");
    assert_eq!(readable(&not(truth(local(0)))), "local_0 == 0");
    assert_eq!(readable(&not(less())), "(int)local_0 >= (int)local_1");
    let flag = kinds(&[(Var::Local(0), Kind::Bool)]);
    let none = Names::default();
    assert_eq!(text_in(&flag, &none, &truth(local(0)), false), "local_0");
    assert_eq!(
        text_in(&flag, &none, &not(truth(local(0))), false),
        "!local_0"
    );
    let select = Expr::Select {
        cond: Box::new(Expr::binary(BinOp::Ne, less(), truth(int(0)))),
        then: Box::new(Expr::cast(Scalar::U64, local(0))),
        els: Box::new(int(7)),
    };
    assert_eq!(
        readable(&select),
        "(int)local_0 < (int)local_1 ? local_0 : 7"
    );
}

#[test]
fn memory_is_written_as_the_fields_of_what_an_address_points_at() {
    let p0 = param(0);
    let v = Var::Value { at: 0x96, index: 1 };
    let l3 = || address(Var::Local(3), 0);
    let cases = [
        (load(Scalar::U64, p0.clone(), 0x10), "p0->f_10"),
        (load(Scalar::U8, p0.clone(), 0x10), "(byte)p0->f_10"),
        (
            load(Scalar::U64, load(Scalar::U64, p0.clone(), 0x10), 0x8),
            "p0->f_10->f_8",
        ),
        // The first bytes of what a pointer points at are the thing itself.
        (load(Scalar::U64, p0.clone(), 0), "*p0"),
        (load(Scalar::I32, p0.clone(), 0), "(int)*p0"),
        (
            load(Scalar::U64, load(Scalar::U64, p0.clone(), 0), 0x8),
            "(*p0)->f_8",
        ),
        // Through the address of a variable, the variable.
        (load(Scalar::U64, l3(), 0), "local_3"),
        (load(Scalar::I32, l3(), 0), "(int)local_3"),
        (load(Scalar::U64, l3(), 8), "local_3.f_8"),
        (load(Scalar::U64, address(v, 2), 8), "v_000096_1[2].f_8"),
        // A struct inside another: an offset, then the field's own.
        (
            load(Scalar::I32, offset(p0.clone(), int(0x28)), 4),
            "(int)p0->f_28.f_4",
        ),
        (load(Scalar::U64, offset(p0.clone(), int(8)), 0), "p0->f_8"),
        (
            load(Scalar::U64, offset(offset(l3(), int(0x10)), int(0x20)), 4),
            "local_3.f_10.f_20.f_4",
        ),
        (offset(p0.clone(), int(0x28)), "&p0->f_28"),
        (offset(p0.clone(), int(0)), "p0"),
        // An offset that is no constant is arithmetic on the address.
        (offset(p0.clone(), local(0)), "byte_offset(p0, local_0)"),
        (
            load(Scalar::U64, offset(p0.clone(), local(0)), 8),
            "byte_offset(p0, local_0)->f_8",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(readable(e), *text);
    }

    let store = |kind, addr: Expr, offset, value: Expr| Stmt::Store {
        kind,
        addr,
        offset,
        value,
    };
    let step = |addr: Expr, offset, down| Stmt::StepI32 { addr, offset, down };
    let statements = [
        (
            store(StoreKind::U8, p0.clone(), 0x10, int(1)),
            "p0->f_10 = (byte)1",
        ),
        (
            store(StoreKind::I32, p0.clone(), 0x10, local(1)),
            "p0->f_10 = (int)local_1",
        ),
        (
            store(StoreKind::I32, p0.clone(), 0x10, int(0x1234_ABCD)),
            "p0->f_10 = 0x1234ABCD",
        ),
        (
            store(
                StoreKind::I32,
                p0.clone(),
                0x10,
                Expr::Const(Const::f32(1.5)),
            ),
            "p0->f_10 = 1.5f",
        ),
        (
            store(
                StoreKind::U16,
                p0.clone(),
                0x10,
                Expr::cast(Scalar::U16, local(1)),
            ),
            "p0->f_10 = (ushort)local_1",
        ),
        (
            store(
                StoreKind::U64,
                p0.clone(),
                0x8,
                Expr::cast(Scalar::U64, local(1)),
            ),
            "p0->f_8 = local_1",
        ),
        (
            store(StoreKind::Ref, p0.clone(), 0, local(1)),
            "*p0 = local_1",
        ),
        (
            store(StoreKind::I32, p0.clone(), 0, local(1)),
            "*p0 = (int)local_1",
        ),
        // A whole cell written through a variable's address is an assignment to it.
        (
            store(StoreKind::U64, l3(), 0, local(1)),
            "local_3 = local_1",
        ),
        // ⚠ Four bytes of it are not: the other four stay.
        (store(StoreKind::I32, l3(), 0, int(5)), "local_3.f_0 = 5"),
        (store(StoreKind::I32, l3(), 4, int(5)), "local_3.f_4 = 5"),
        (step(p0.clone(), 12, false), "p0->f_C++"),
        (step(p0.clone(), 0, true), "(*p0)--"),
        (step(l3(), 0, false), "local_3.f_0++"),
        // A bare number reads as the four bytes of an `int`: a store of another width says so.
        (
            store(StoreKind::U16, p0.clone(), 0x10, int(7)),
            "p0->f_10 = (ushort)7",
        ),
        (
            store(StoreKind::U64, p0.clone(), 0x8, int(5)),
            "p0->f_8 = 5L",
        ),
        (
            store(
                StoreKind::Ref,
                p0.clone(),
                0x8,
                constant_of(0, Kind::Pointer),
            ),
            "p0->f_8 = null",
        ),
    ];
    for (s, text) in &statements {
        assert_eq!(stmt_of(s), *text);
    }

    // ⚠ The pointer a variable holds and the variable's own cells are two places.
    let through = load(Scalar::U64, local(3), 8);
    let in_place = load(Scalar::U64, l3(), 8);
    assert_eq!(readable(&through), "local_3->f_8");
    assert_eq!(readable(&in_place), "local_3.f_8");
}

#[test]
fn a_store_of_a_whole_cell_says_so_where_its_value_is_narrower() {
    let typed = kinds(&[
        (Var::Local(0), Kind::I32),
        (Var::Local(1), Kind::F32),
        (Var::Local(2), Kind::Bool),
    ]);
    let stored = |kind: StoreKind, value: Expr| {
        let (names, mut out) = (Names::default(), String::new());
        let cx = at(&names, Level::Structured);
        let store = Stmt::Store {
            kind,
            addr: param(1),
            offset: 0x8,
            value,
        };
        Writer::in_context(&mut out, &cx, &typed).simple(&store)
    };
    // ⚠ A store reads as wide as its value: eight bytes of an `int` would read as four.
    assert_eq!(stored(StoreKind::U64, local(0)), "p1->f_8 = (long)local_0");
    assert_eq!(stored(StoreKind::I32, local(0)), "p1->f_8 = local_0");
    assert_eq!(
        stored(StoreKind::U64, local(1)),
        "p1->f_8 = as_u64(local_1)"
    );
    assert_eq!(stored(StoreKind::U64, local(2)), "p1->f_8 = (long)local_2");
    // A whole cell stored whole says nothing more, and a number says `L`.
    assert_eq!(stored(StoreKind::U64, local(3)), "p1->f_8 = local_3");
    assert_eq!(stored(StoreKind::U64, int(5)), "p1->f_8 = 5L");
}

#[test]
fn a_sum_of_whole_cells_is_not_written_as_one_of_narrower_numbers() {
    let typed = kinds(&[(Var::Local(0), Kind::I32), (Var::Local(1), Kind::U32)]);
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    let whole = |op: BinOp, l: Expr, r: Expr| {
        let sum = Expr::Binary {
            op,
            at: Kind::I64,
            l: Box::new(Expr::cast(Scalar::I64, l)),
            r: Box::new(Expr::cast(Scalar::I64, r)),
        };
        sharp_in(&typed, &cx, &sum)
    };
    // ⚠ C# adds two `int`s in 32 bits: the sum of their cells is of 64.
    let narrow = whole(BinOp::Add, local(0), local(1));
    assert_eq!(narrow, "(long)local_0 + local_1");
    assert_eq!(whole(BinOp::Mul, local(0), int(3)), "(long)local_0 * 3");
    assert_eq!(whole(BinOp::Sub, int(3), local(0)), "3 - (long)local_0");
    // A cell of no proved kind is 64 bits to C# already, and so is the sum with it.
    assert_eq!(whole(BinOp::Add, local(2), local(0)), "local_2 + local_0");
    // What the sign of a cell does not count to keeps no reading.
    assert_eq!(whole(BinOp::Or, local(0), local(1)), "local_0 | local_1");
}

#[test]
fn a_step_prints_as_the_listing_wrote_it() {
    let one = Var::Local(1);
    let stepped = |op, from: u32, by| {
        assign(
            one,
            Expr::binary(op, Expr::cast(Scalar::I32, local(from)), int(by)),
        )
    };
    let ints = kinds(&[(one, Kind::I32)]);
    let step_of = |s: &Stmt| {
        let (names, mut out) = (Names::default(), String::new());
        Writer::new(&mut out, false, &names, &ints).simple(s)
    };
    assert_eq!(step_of(&stepped(BinOp::Add, 1, 1)), "local_1++");
    assert_eq!(step_of(&stepped(BinOp::Sub, 1, 1)), "local_1--");
    // ⚠ On a variable of no proved kind `++` would read as a step of the whole cell.
    assert_eq!(
        stmt_of(&stepped(BinOp::Add, 1, 1)),
        "local_1 = (int)local_1 + 1"
    );
    // Another variable, another amount and another operator are no step.
    assert_eq!(
        stmt_of(&stepped(BinOp::Add, 0, 1)),
        "local_1 = (int)local_0 + 1"
    );
    assert_eq!(
        stmt_of(&stepped(BinOp::Add, 1, 2)),
        "local_1 = (int)local_1 + 2"
    );
    assert_eq!(
        stmt_of(&stepped(BinOp::Mul, 1, 1)),
        "local_1 = (int)local_1 * 1"
    );
    // A step of a listing is in 32 bits: the same in 64 is an addition.
    let wide = Expr::binary(BinOp::Add, Expr::cast(Scalar::I64, local(1)), int(1));
    assert_eq!(stmt_of(&assign(one, wide)), "local_1 = local_1 + 1");
}

#[test]
fn an_intrinsic_c_sharp_has_an_operator_for_prints_as_it() {
    let mut names = Names::default();
    names
        .types
        .insert(0x0123_4567_89AB_CDEF, "CDemoThing".to_owned());
    let head = head();
    let (a, i, x) = (local(0), local(1), local(2));
    let two = || vec![a.clone(), i.clone()];
    let three = || vec![a.clone(), i.clone(), x.clone()];
    let cases = [
        (intrinsic("array_get_u64", two()), "local_0[local_1]"),
        (intrinsic("array_get_i32", two()), "(int)local_0[local_1]"),
        (
            Expr::cast(Scalar::I32, intrinsic("array_get_i32", two())),
            "(int)local_0[local_1]",
        ),
        (intrinsic("array_get_value", two()), "local_0[local_1]"),
        (intrinsic("array_length", vec![a.clone()]), "local_0.Length"),
        (
            intrinsic("array_element_address", two()),
            "&local_0[local_1]",
        ),
        (
            intrinsic("try_cast<type_0123456789ABCDEF>", vec![a.clone()]),
            "local_0 as CDemoThing",
        ),
        (
            Expr::binary(
                BinOp::Eq,
                intrinsic("try_cast<type_00000000000000DD>", vec![a.clone()]),
                int(0),
            ),
            "(local_0 as type_00000000000000DD) == 0",
        ),
        (
            intrinsic("box<type_0123456789ABCDEF>", two()),
            "box<CDemoThing>(local_0, local_1)",
        ),
        (intrinsic("invoke_delegate", two()), "local_0(local_1)"),
        (intrinsic("invoke_delegate", vec![a.clone()]), "local_0()"),
        // Another count of arguments is another intrinsic than the one known.
        (
            intrinsic("array_get_u64", three()),
            "array_get_u64(local_0, local_1, local_2)",
        ),
        (
            intrinsic("array_length", two()),
            "array_length(local_0, local_1)",
        ),
        (
            intrinsic("i32_to_f32", vec![a.clone()]),
            "i32_to_f32(local_0)",
        ),
        (intrinsic("no name", vec![]), "\"no name\"()"),
    ];
    for (e, text) in &cases {
        assert_eq!(text_in(&head, &names, e, false), *text);
    }
    // An intrinsic written with its type says the type.
    let boxed = Expr::Call {
        callee: Callee::Intrinsic {
            name: "box_at".to_owned(),
            ty: Some(VmType {
                name: "type_0123456789ABCDEF".to_owned(),
                byte_size: 16,
                id: 40,
                flags: 0,
            }),
            wrapper: None,
        },
        args: vec![address(Var::Local(2), 0)],
        returns: Some(1),
    };
    assert_eq!(
        text_in(&head, &names, &boxed, false),
        "box_at<CDemoThing>(&local_2)"
    );

    let statements = [
        (
            intrinsic("array_set_ref", three()),
            "local_0[local_1] = local_2",
        ),
        (
            intrinsic("array_set_u32", three()),
            "local_0[local_1] = (uint)local_2",
        ),
        (
            intrinsic("array_set_u8", vec![a.clone(), i.clone(), int(1)]),
            "local_0[local_1] = (byte)1",
        ),
        (
            intrinsic(
                "array_set_slots",
                vec![a.clone(), i.clone(), x.clone(), int(0)],
            ),
            "local_0[local_1] = { local_2, 0 }",
        ),
        (
            intrinsic("array_set_ref", two()),
            "array_set_ref(local_0, local_1)",
        ),
        (
            intrinsic("array_set_f32", three()),
            "array_set_f32(local_0, local_1, local_2)",
        ),
    ];
    for (e, text) in statements {
        assert_eq!(stmt_of(&Stmt::Eval(e)), text);
    }
}

#[test]
fn a_virtual_call_is_a_method_of_a_receiver_whose_type_says_whose() {
    let class = "type_00000000000000AA";
    let call = Expr::Call {
        callee: Callee::Virtual {
            class: class.to_owned(),
            method: "Method_3".to_owned(),
            ty: class.to_owned(),
            slot: 3,
        },
        args: vec![param(0), local(1)],
        returns: Some(1),
    };
    let none = Names::default();
    let typed = kinds(&[(Var::Param(0), Kind::Object(class.to_owned()))]);
    assert_eq!(text_in(&typed, &none, &call, false), "p0.Method_3(local_1)");
    // A receiver of another type, or of none known, does not say which type's slot 3 it is.
    let other = kinds(&[(
        Var::Param(0),
        Kind::Object("type_00000000000000BB".to_owned()),
    )]);
    let unsaid = "type_00000000000000AA.Method_3(p0, local_1)";
    assert_eq!(text_in(&other, &none, &call, false), unsaid);
    assert_eq!(readable(&call), unsaid);
}

#[test]
fn what_another_program_holds_prints_under_the_programs_name() {
    let mut names = Names::default();
    names.programs.insert(0xAB, "shared".to_owned());
    let head = head();
    let global = |program| {
        Expr::Var(Var::Global {
            program,
            offset: 0x48,
            statics: false,
        })
    };
    let import = |module: &str, program| Expr::Call {
        callee: Callee::Import {
            module: module.to_owned(),
            name: "func_0000B0".to_owned(),
            program,
            at: 0xB0,
        },
        args: vec![],
        returns: Some(1),
    };
    let cases = [
        (global(0xAB), "shared.Global_48"),
        (global(0xCD), "Global_000000CD_48"),
        (global(0x1122_3344), "Global_48"),
        (
            Expr::FunctionRef {
                program: 0xAB,
                at: 0x100,
            },
            "&shared.func_000100",
        ),
        (
            Expr::FunctionRef {
                program: 0xCD,
                at: 0x100,
            },
            "&program_000000CD.func_000100",
        ),
        (
            Expr::StringConstant {
                program: 0xAB,
                offset: 8,
            },
            "shared.String_8",
        ),
        (
            Expr::StringConstant {
                program: 0x1122_3344,
                offset: 8,
            },
            "String_8",
        ),
        // A module the listing knows by a hash takes the name of the program it is.
        (
            import("module_0x00000000000000EE", 0xAB),
            "shared.func_0000B0()",
        ),
        (
            import("module_0x00000000000000EE", 0xCD),
            "module_0x00000000000000EE.func_0000B0()",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(text_in(&head, &names, e, false), *text);
    }
}

#[test]
fn a_readable_function_names_what_the_names_name() {
    let (_, mut functions) = read(sample::LISTING);
    let mut names = Names::default();
    names
        .types
        .insert(0x0123_4567_89AB_CDEF, "CDemoThing".to_owned());
    names.programs.insert(0xCC, "demo_module".to_owned());

    let out = written(&functions[0], &names);
    for line in [
        "void func_000000(var p0, var p1)",
        "    var local_0, local_1, local_3, stack_000040_0, stack_000040_1;",
        "    if (Global_0A0B0C0D_1230 == 0)",
        "        var v_000010_0 = new CDemoThing();",
        "        Global_0A0B0C0D_1230 = v_000010_0;",
        "        (stack_000040_0, stack_000040_1) = (Static_0A0B0C0D_10, 0);",
        "    var[] v_000018_1 = shared.func_0000A0(p1);",
        "    local_1 = v_000018_1[0];",
        "    var local_2 = v_000018_1[1];",
        "    var[] v_000028_3 = default(var[2]);",
        "    v_000028_3.f_0 = 0x1234ABCD;",
        "    p1->f_8 = p1->f_8 | 2;",
        "    p0->f_C++;",
        "    DEMO.DO_THING(0);",
        "    _0x00000000000000BB(p0, BuiltinGlobal(0)[0]);",
        "    Example.Thing.Ctor(v_000020_2, local_0, &func_000100);",
        "    demo_module.func_0000B0(BuiltinFunction(2, 1));",
        "    func_000100(&p0);",
        "    unreachable;",
    ] {
        assert!(
            out.lines().any(|l| l == line),
            "no line `{line}` in:\n{out}"
        );
    }

    functions[0]
        .head
        .vars
        .entry(Var::Param(1))
        .or_insert_with(|| {
            let mut info = VarInfo::new(Width::Cell);
            info.kind = Kind::Object("CDemoThing".to_owned());
            info.name = Some("thing".to_owned());
            info
        });
    let renamed = written(&functions[0], &names);
    assert!(
        renamed.starts_with("void func_000000(var p0, CDemoThing thing)\n"),
        "{renamed}"
    );
    assert!(
        renamed.contains("thing->f_8 = thing->f_8 | 2;"),
        "{renamed}"
    );

    assert_eq!(
        written(&functions[3], &names),
        "// func_000300 at 0x000300: the listing holds no body (Widths are underdetermined.)\n"
    );
}

#[test]
fn a_variable_is_declared_where_it_is_first_assigned_or_at_the_top() {
    let simple = |s: Stmt| tree::Stmt::Simple(s);
    let v = Var::Value { at: 0x10, index: 0 };
    let below =
        |n: u32, limit| Expr::binary(BinOp::Lt, Expr::cast(Scalar::I32, local(n)), int(limit));
    let step = |n: u32| {
        assign(
            Var::Local(n),
            Expr::binary(BinOp::Add, Expr::cast(Scalar::I32, local(n)), int(1)),
        )
    };
    let body = vec![
        simple(assign(Var::Local(0), int(5))),
        simple(assign(v, Expr::ZeroSlots(2))),
        simple(Stmt::Store {
            kind: StoreKind::I32,
            addr: address(v, 0),
            offset: 0,
            value: local(0),
        }),
        tree::Stmt::If {
            cond: param(0),
            then: vec![simple(assign(Var::Local(1), local(0)))],
            els: vec![tree::Stmt::If {
                cond: below(0, 3),
                then: vec![simple(assign(Var::Local(1), int(0)))],
                els: vec![
                    simple(assign(Var::Local(3), int(1))),
                    simple(Stmt::Eval(native("FILL", vec![address(Var::Local(3), 0)]))),
                ],
            }],
        },
        simple(step(2)),
        simple(Stmt::Eval(native("USE", vec![local(1), local(2)]))),
        tree::Stmt::For {
            init: Some(Box::new(assign(Var::Local(4), int(0)))),
            cond: Some(below(4, 4)),
            step: Some(Box::new(step(4))),
            body: vec![tree::Stmt::Continue],
        },
        simple(assign(Var::Local(5), int(1))),
        tree::Stmt::Label(Label::Block(3)),
        simple(Stmt::Eval(native("USE", vec![local(5)]))),
    ];
    let mut head = kinds(&[(Var::Local(0), Kind::I32)]);
    head.locals = 6;
    head.vars.insert(v, VarInfo::new(Width::Cells));
    let f = tree::Function { head, body };
    // local_1 is read outside the block that assigns it, local_2 before any assignment,
    // local_3 through its address, and local_5 after a label a jump may come to round its
    // assignment. The address of `v_000010_0` is only ever taken after its one assignment.
    let expected = "\
void func_000040(var p0)
{
    var local_1, local_2, local_3, local_5;
    int local_0 = 5;
    var[] v_000010_0 = default(var[2]);
    v_000010_0.f_0 = local_0;
    if (p0)
    {
        local_1 = local_0;
    }
    else if (local_0 < 3)
    {
        local_1 = 0;
    }
    else
    {
        local_3 = 1;
        DEMO.FILL(&local_3);
    }
    local_2 = (int)local_2 + 1;
    DEMO.USE(local_1, local_2);
    for (var local_4 = 0; (int)local_4 < 4; local_4 = (int)local_4 + 1)
    {
        continue;
    }
    local_5 = 1;
    block_3: ;
    DEMO.USE(local_5);
}
";
    assert_eq!(written(&f, &Names::default()), expected);
}

#[test]
fn an_else_is_chained_only_when_it_is_one_if_and_never_in_the_plain_form() {
    let leave = || vec![tree::Stmt::Break];
    let inner = || tree::Stmt::If {
        cond: local(0),
        then: leave(),
        els: Vec::new(),
    };
    let chain = |els: Vec<tree::Stmt>| tree::Function {
        head: head(),
        body: vec![tree::Stmt::If {
            cond: param(0),
            then: leave(),
            els,
        }],
    };
    let none = Names::default();
    let chained = written(&chain(vec![inner()]), &none);
    assert!(
        chained.contains("    }\n    else if (local_0)\n    {\n"),
        "{chained}"
    );

    // A statement beside the `if` makes the `else` a block of its own.
    let block = written(&chain(vec![inner(), tree::Stmt::Continue]), &none);
    assert!(
        block.contains("    else\n    {\n        if (local_0)\n"),
        "{block}"
    );

    let mut plain = ListingWriter::new(0x1122_3344, &[]);
    plain.function(&chain(vec![inner()]));
    let plain = plain.finish();
    assert!(
        plain.contains("        else\n        {\n            if (local_0)\n"),
        "{plain}"
    );
}

#[test]
fn a_structured_body_is_written_with_its_loops_and_labels() {
    let body = vec![
        tree::Stmt::For {
            init: Some(Box::new(assign(Var::Local(0), int(0)))),
            cond: Some(Expr::binary(BinOp::Lt, local(0), int(4))),
            step: None,
            body: vec![tree::Stmt::Continue],
        },
        tree::Stmt::DoWhile {
            body: vec![tree::Stmt::Goto(Label::Block(3))],
            cond: local(1),
        },
        tree::Stmt::Label(Label::Block(3)),
        tree::Stmt::Switch {
            value: Expr::cast(Scalar::I32, local(0)),
            cases: vec![
                tree::Case {
                    labels: vec![Some(0), Some(2)],
                    body: vec![tree::Stmt::Break],
                },
                tree::Case {
                    labels: vec![None],
                    body: vec![tree::Stmt::Return(Some(int(1)))],
                },
            ],
        },
        tree::Stmt::Return(Some(local(0))),
    ];
    let f = tree::Function { head: head(), body };
    // local_0 is read after the loop whose head assigns it, so the head does not declare it.
    let expected = "\
void func_000040(var p0)
{
    var local_0, local_1;
    for (local_0 = 0; local_0 < 4; )
    {
        continue;
    }
    do
    {
        goto block_3;
    }
    while (local_1);
    block_3: ;
    switch ((int)local_0)
    {
        case 0:
        case 2:
            break;
        default:
            return 1;
    }
    return local_0;
}
";
    assert_eq!(written(&f, &Names::default()), expected);
}

#[test]
fn a_listing_is_written_under_a_header_that_says_what_named_things() {
    let (listing, functions) = read(sample::LISTING);
    let stats = Stats {
        functions: 5,
        incomplete: 1,
        gotos_in: 3,
        gotos_out: 2,
        ..Stats::default()
    };
    let source = |names: &Names| {
        let mut writer = SourceWriter::new(names);
        for f in &functions {
            writer.function(f);
        }
        writer.finish(&listing, &stats)
    };

    let unnamed = source(&Names::default());
    // The first line says what was read: a lifted listing, and no compiled script.
    let said = "// Readable source of a lifted listing: read from the text another decompiler \
                wrote of this GTA VI script, not from the compiled script.\n";
    assert!(unnamed.starts_with(said), "{unnamed}");
    let head = "\
// program 0x11223344
// imports: 0x0A0B0C0D, 0x1A2B3C4D
// natives: 3 declared, no natives list names a native: 2 named by the listing itself, 1 shown by hash
// names: no names list supplied: a type or a program the listing does not name is shown by its hash
// 5 function(s), 1 of them with no body in the listing; 2 goto(s) left, of 3 read

void func_000000(var p0, var p1)
";
    assert!(unnamed.contains(head), "{unnamed}");
    assert!(
        unnamed.contains("}\n\nvar func_000100(var p0)\n{\n"),
        "{unnamed}"
    );
    assert_eq!(unnamed, source(&Names::default()), "the same bytes twice");

    let listed = NativeName {
        namespace: Some("LISTED".to_owned()),
        name: "NAMED_BY_THE_LIST".to_owned(),
        ..NativeName::default()
    };
    let mut names = Names {
        natives: Some(NativeDb::from_names("a test's", [(0xBB, listed)])),
        ..Names::default()
    };
    let list = "0x11223344 demo.program\n0x0A0B0C0D shared\n";
    names.read_list("names.txt", list).expect("the list reads");
    let named = source(&names);
    let head = "\
// program 0x11223344 (demo.program)
// imports: 0x0A0B0C0D (shared), 0x1A2B3C4D
// natives: 3 declared: 1 named by `a test's`, 2 by the listing itself, 0 shown by hash
// names: types and programs from `names.txt`
";
    assert!(named.contains(head), "{named}");
    assert!(
        named.contains("    LISTED.NAMED_BY_THE_LIST(p0, BuiltinGlobal(0)[0]);\n"),
        "{named}"
    );
}

/// A context of every pass of `level`, with nothing gathered over an install.
pub(super) fn at(names: &Names, level: Level) -> Context<'_> {
    Context::new(names, Project::empty(), Options::at(level))
}

/// `e` as true C# writes it in a function of the head given.
fn sharp_in(head: &Head, cx: &Context<'_>, e: &Expr) -> String {
    let mut out = String::new();
    Writer::in_context(&mut out, cx, head).expr(e, 0)
}

fn sharp(e: &Expr) -> String {
    let (head, names) = (head(), Names::default());
    sharp_in(&head, &at(&names, Level::Structured), e)
}

/// `e` as true C# writes the condition of an `if`.
fn sharp_cond_in(head: &Head, e: &Expr) -> String {
    let (names, mut out) = (Names::default(), String::new());
    let cx = at(&names, Level::Structured);
    Writer::in_context(&mut out, &cx, head).cond(e, 0)
}

/// A simple statement of true C#, without its semicolon.
fn sharp_stmt(s: &Stmt) -> String {
    let (head, names, mut out) = (head(), Names::default(), String::new());
    let cx = at(&names, Level::Structured);
    Writer::in_context(&mut out, &cx, &head).simple(s)
}

pub(super) fn written_in(f: &tree::Function, cx: &Context<'_>) -> String {
    let mut out = String::new();
    function_in(&mut out, f, cx);
    out
}

/// `f` as true C# with no name supplied.
fn written_sharp(f: &tree::Function) -> String {
    let names = Names::default();
    written_in(f, &at(&names, Level::Structured))
}

pub(super) fn simple(s: Stmt) -> tree::Stmt {
    tree::Stmt::Simple(s)
}

pub(super) fn function_of(head: Head, body: Vec<tree::Stmt>) -> tree::Function {
    tree::Function { head, body }
}

/// A 32-bit constant with its top bit set, as an `int`.
pub(super) fn high() -> Expr {
    int(0x9A3B_5C7Du32 as i32)
}

#[test]
fn true_c_sharp_is_what_any_pass_beyond_the_four_first_asks_for() {
    // ⚠ The four first passes, and fewer of them, write what `decompile` always wrote.
    assert!(!true_csharp(&Options::baseline()));
    assert!(!true_csharp(&Options::baseline().without(Pass::Structure)));
    assert!(!true_csharp(&Options::at(Level::Lifted)));
    assert!(true_csharp(&Options::baseline().with(Pass::HashText)));
    assert!(true_csharp(&Options::at(Level::Structured)));
    assert!(true_csharp(&Options::default()));

    // The entry that takes a context writes the form of old for the four first passes.
    let names = Names::default();
    let returned = Expr::Cells(vec![local(0), local(1)]);
    let f = function_of(head(), vec![tree::Stmt::Return(Some(returned))]);
    let baseline = written_in(&f, &Context::baseline(&names));
    assert_eq!(baseline, written(&f, &names));
    assert!(
        baseline.contains("return { local_0, local_1 };"),
        "{baseline}"
    );
}

#[test]
fn true_c_sharp_names_no_type_and_writes_no_group_that_c_sharp_has_not() {
    let v = Var::Value { at: 0x10, index: 0 };
    let wide = Expr::LoadBytes {
        addr: Box::new(param(0)),
        offset: 0x58,
        bytes: 16,
    };
    let used = native("USE", vec![wide, Expr::ZeroSlots(2), local(3)]);
    let body = vec![
        simple(assign(Var::Local(0), Expr::ZeroSlots(2))),
        simple(assign(v, Expr::Cells(vec![param(0), int(1)]))),
        simple(assign(Var::Local(1), param(0))),
        simple(assign(Var::Local(2), Expr::ZeroSlots(1))),
        simple(Stmt::Eval(used)),
        tree::Stmt::Return(Some(Expr::Cells(vec![local(1), local(2)]))),
    ];
    let mut head = head();
    head.returns = Returns::Cells;
    head.locals = 4;
    head.vars.insert(v, VarInfo::new(Width::Cells));
    let mut pair = VarInfo::new(Width::Cells);
    pair.kind = Kind::Aggregate(2);
    head.vars.insert(Var::Local(0), pair);
    let f = function_of(head, body);

    // `var` only where a value says the type; `Cell` and `Cells` where a type must stand.
    let expected = "\
Cells func_000040(Cell p0)
{
    Cell local_3;
    Cells2 local_0 = default;
    Cells v_000010_0 = (p0, 1);
    Cell local_1 = p0;
    Cell local_2 = default;
    DEMO.USE((Cells2)p0->f_58, default(Cells2), local_3);
    return (local_1, local_2);
}
";
    assert_eq!(written_sharp(&f), expected);

    // The form of old, which the four first passes still write, is untouched.
    let old = "\
var[] func_000040(var p0)
{
    var local_3;
    var[2] local_0 = default;
    var[] v_000010_0 = { p0, 1 };
    var local_1 = p0;
    var local_2 = default;
    DEMO.USE((var[2])p0->f_58, default(var[2]), local_3);
    return { local_1, local_2 };
}
";
    assert_eq!(written(&f, &Names::default()), old);

    // One cell gathered alone is that cell, and cells set in an array are a tuple.
    let one = Expr::Cells(vec![local(1)]);
    assert_eq!(sharp(&one), "local_1");
    assert_eq!(readable(&one), "{ local_1 }");
    let cells = vec![local(0), local(1), local(2), int(0)];
    let set = Stmt::Eval(intrinsic("array_set_slots", cells));
    assert_eq!(sharp_stmt(&set), "local_0[local_1] = (local_2, 0)");
}

#[test]
fn an_expression_that_is_no_statement_is_given_to_a_discard() {
    let (a, i, x) = (local(0), local(1), local(2));
    let object = VmType {
        name: "type_00000000000000AB".to_owned(),
        byte_size: 16,
        id: 7,
        flags: 0,
    };
    let array = Expr::NewArray {
        ty: VmType {
            name: "System.Int32".to_owned(),
            byte_size: 4,
            id: 0x8000_0003,
            flags: 0,
        },
        len: Box::new(int(4)),
        wrapper: "NewArray_3".to_owned(),
    };
    let cases = [
        (load(Scalar::U64, param(0), 0x8), "_ = p0->f_8"),
        (
            Expr::binary(BinOp::Eq, a.clone(), i.clone()),
            "_ = local_0 == local_1",
        ),
        (
            intrinsic("array_get_u64", vec![a.clone(), i.clone()]),
            "_ = local_0[local_1]",
        ),
        (
            intrinsic("try_cast<type_00000000000000DD>", vec![a.clone()]),
            "_ = local_0 as type_00000000000000DD",
        ),
        (array, "_ = new int[4]"),
        // What C# takes for a statement stands as it is: a call, a new object, an assignment.
        (native("USE", vec![]), "DEMO.USE()"),
        (intrinsic("invoke_delegate", vec![a.clone()]), "local_0()"),
        (Expr::NewObject(object), "new type_00000000000000AB()"),
        (
            intrinsic("array_set_ref", vec![a.clone(), i.clone(), x.clone()]),
            "local_0[local_1] = local_2",
        ),
    ];
    for (e, text) in cases {
        assert_eq!(sharp_stmt(&Stmt::Eval(e)), text);
    }
    // The form of old writes the expression bare.
    let read = Stmt::Eval(load(Scalar::U64, param(0), 0x8));
    assert_eq!(stmt_of(&read), "p0->f_8");
}

#[test]
fn a_case_of_true_c_sharp_says_where_it_goes_on_to() {
    let used = || simple(Stmt::Eval(native("USE", vec![])));
    let leaves = |els: Vec<tree::Stmt>| tree::Stmt::If {
        cond: param(0),
        then: vec![tree::Stmt::Break],
        els,
    };
    let case = |label: Option<i64>, body: Vec<tree::Stmt>| tree::Case {
        labels: vec![label],
        body,
    };
    let body = vec![
        tree::Stmt::Switch {
            value: Expr::cast(Scalar::I32, local(0)),
            cases: vec![
                case(Some(0), vec![used()]),
                case(Some(1), Vec::new()),
                case(Some(2), vec![tree::Stmt::Return(None)]),
                case(Some(3), vec![leaves(vec![tree::Stmt::Return(None)])]),
                case(Some(4), vec![leaves(Vec::new())]),
                case(None, vec![used()]),
            ],
        },
        tree::Stmt::Unreachable,
    ];
    let f = function_of(head(), body);
    // A body that can be left by its end names the next case; one that cannot, and an empty
    // one whose labels share the next case's, are written as they are.
    let expected = "\
void func_000040(Cell p0)
{
    Cell local_0;
    switch ((int)local_0)
    {
        case 0:
            DEMO.USE();
            goto case 1;
        case 1:
        case 2:
            return;
        case 3:
            if (p0)
            {
                break;
            }
            else
            {
                return;
            }
        case 4:
            if (p0)
            {
                break;
            }
            goto default;
        default:
            DEMO.USE();
            break;
    }
    throw new System.Diagnostics.UnreachableException();
}
";
    assert_eq!(written_sharp(&f), expected);

    let old = written(&f, &Names::default());
    assert!(!old.contains("goto "), "{old}");
    assert_eq!(old.matches("break;").count(), 2, "{old}");
    assert!(old.contains("    unreachable;\n"), "{old}");
}

#[test]
fn a_switch_on_hashes_is_written_in_32_unsigned_bits() {
    let switch = |value: Expr, labels: Vec<Option<i64>>| {
        let first = tree::Case {
            labels,
            body: vec![tree::Stmt::Break],
        };
        let other = tree::Case {
            labels: vec![None],
            body: vec![tree::Stmt::Return(None)],
        };
        let cases = vec![first, other];
        vec![tree::Stmt::Switch { value, cases }]
    };
    let has = |text: &str, line: &str| text.lines().any(|l| l == line);
    let hashes = vec![Some(-1_978_987_507), Some(5)];

    let narrow = Expr::cast(Scalar::I32, param(0));
    let f = function_of(head(), switch(narrow.clone(), hashes.clone()));
    let text = written_sharp(&f);
    for line in [
        "    switch ((uint)p0)",
        "        case 0x8A0B0C0D:",
        "        case 0x00000005:",
    ] {
        assert!(has(&text, line), "no line `{line}` in:\n{text}");
    }

    // ⚠ The labels are compared with the low 32 bits: on a whole cell the text says so.
    let f = function_of(head(), switch(param(0), hashes.clone()));
    let whole = written_sharp(&f);
    assert!(has(&whole, "    switch ((int)p0)"), "{whole}");
    assert!(has(&whole, "        case -1978987507:"), "{whole}");
    // Small labels stay numbers, and a label that fits no 32 bits leaves them all decimal.
    let labels = vec![Some(-3), Some(65_536)];
    let small = written_sharp(&function_of(head(), switch(narrow.clone(), labels.clone())));
    assert!(has(&small, "    switch ((int)p0)"), "{small}");
    assert!(has(&small, "        case -3:"), "{small}");
    // A subject declared an `int` is compared as one already, and a byte widens to one.
    for kind in [Kind::I32, Kind::U8] {
        let declared = kinds(&[(Var::Param(0), kind)]);
        let bare = written_sharp(&function_of(declared, switch(param(0), labels.clone())));
        assert!(has(&bare, "    switch (p0)"), "{bare}");
    }
    let labels = vec![Some(0x1_0000_0001)];
    let long = written_sharp(&function_of(head(), switch(narrow, labels)));
    assert!(has(&long, "        case 4294967297:"), "{long}");

    // A value that is a hash shows the text a supplied vocabulary has for a label.
    let mut names = Names::default();
    names.texts.insert(0x8A0B_0C0D, "door_left".to_owned());
    let hashed = kinds(&[(Var::Param(0), Kind::Hash)]);
    let f = function_of(hashed, switch(param(0), hashes));
    let named = written_in(&f, &at(&names, Level::Structured));
    assert!(has(&named, "    switch (p0)"), "{named}");
    assert!(has(&named, "        case Hash(\"door_left\"):"), "{named}");
    assert!(has(&named, "        case 0x00000005:"), "{named}");
    // With `hash-text` off the label is its eight digits.
    let off = Options::at(Level::Structured).without(Pass::HashText);
    let unnamed = written_in(&f, &Context::new(&names, Project::empty(), off));
    assert!(has(&unnamed, "        case 0x8A0B0C0D:"), "{unnamed}");
    assert!(!unnamed.contains("door_left"), "{unnamed}");
}

#[test]
fn a_hash_constant_shows_its_text_only_under_its_pass_and_its_kind() {
    let mut names = Names::default();
    names.texts.insert(0x8A0B_0C0D, "door_left".to_owned());
    let head = head();
    let hash = constant_of(0x8A0B_0C0D, Kind::Hash);
    let on = at(&names, Level::Structured);
    assert_eq!(sharp_in(&head, &on, &hash), "Hash(\"door_left\")");
    // The same bits of another kind are a number: a text of the vocabulary may be theirs by accident.
    let number = constant_of(0x8A0B_0C0D, Kind::U32);
    assert_eq!(sharp_in(&head, &on, &number), "0x8A0B0C0D");
    let off = Options::at(Level::Structured).without(Pass::HashText);
    let off = Context::new(&names, Project::empty(), off);
    assert_eq!(sharp_in(&head, &off, &hash), "0x8A0B0C0D");
    assert_eq!(text_in(&head, &names, &hash, false), "0x8A0B0C0D");
}

#[test]
fn bits_read_as_a_float_are_never_written_as_a_cast() {
    let typed = kinds(&[(Var::Local(0), Kind::F32)]);
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    let field = |offset| load(Scalar::F32, param(0), offset);
    let cases = [
        (Expr::cast(Scalar::F32, local(1)), "as_f32(local_1)"),
        (field(0x20), "as_f32(p0->f_20)"),
        (
            Expr::Binary {
                op: BinOp::Mul,
                at: Kind::F32,
                l: Box::new(field(0x20)),
                r: Box::new(field(0x24)),
            },
            "as_f32(p0->f_20) * as_f32(p0->f_24)",
        ),
        // ⚠ Whole cells compared are their bits compared: C# would compare two floats as floats.
        (
            Expr::Binary {
                op: BinOp::Eq,
                at: Kind::U64,
                l: Box::new(local(0)),
                r: Box::new(field(0x24)),
            },
            "as_i64(local_0) == as_i64(as_f32(p0->f_24))",
        ),
        (
            intrinsic("array_get_f32", vec![local(1), local(2)]),
            "as_f32(local_1[local_2])",
        ),
        (Expr::cast(Scalar::F64, local(1)), "as_f64(local_1)"),
        // A float read as a float says nothing, and an integer reading of one keeps its word.
        (Expr::cast(Scalar::F32, local(0)), "local_0"),
        (Expr::cast(Scalar::I32, local(0)), "as_i32(local_0)"),
        // An integer reading is the cast it always was.
        (Expr::cast(Scalar::I32, local(1)), "(int)local_1"),
        (load(Scalar::I32, param(0), 0x10), "(int)p0->f_10"),
    ];
    for (e, text) in &cases {
        assert_eq!(sharp_in(&typed, &cx, e), *text);
    }
    // The form of old: `(float)x` for the bits of a cell of no known kind.
    let cast = Expr::cast(Scalar::F32, local(1));
    assert_eq!(readable(&cast), "(float)local_1");
    assert_eq!(readable(&field(0x20)), "(float)p0->f_20");
}

#[test]
fn equality_with_a_hex_constant_reads_the_other_side_unsigned() {
    let int_p0 = || Expr::cast(Scalar::I32, param(0));
    let at_i32 = |op, l: Expr, r: Expr| Expr::Binary {
        op,
        at: Kind::I32,
        l: Box::new(l),
        r: Box::new(r),
    };
    let typed = kinds(&[(Var::Local(0), Kind::I32)]);
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    let unsigned = Expr::cast(Scalar::U32, param(0));
    let top = constant_of(0x8000_0000, Kind::U32);
    let cases = [
        (
            Expr::binary(BinOp::Eq, int_p0(), high()),
            "(uint)p0 == 0x9A3B5C7D",
        ),
        (
            at_i32(BinOp::Ne, load(Scalar::I32, param(0), 0x10), high()),
            "(uint)p0->f_10 != 0x9A3B5C7D",
        ),
        (
            Expr::binary(BinOp::Eq, high(), int_p0()),
            "0x9A3B5C7D == (uint)p0",
        ),
        (
            at_i32(BinOp::Eq, local(0), high()),
            "(uint)local_0 == 0x9A3B5C7D",
        ),
        // ⚠ Never for an order: which of two numbers is the greater asks their sign.
        (
            Expr::binary(BinOp::Lt, int_p0(), high()),
            "(int)p0 < (int)0x9A3B5C7D",
        ),
        (
            Expr::binary(BinOp::Lt, unsigned, top),
            "(uint)p0 < 0x80000000",
        ),
        // A small number and one with its top bit clear are an `int` as they are written.
        (Expr::binary(BinOp::Eq, int_p0(), int(-1)), "(int)p0 == -1"),
        (
            Expr::binary(BinOp::Eq, int_p0(), int(0x1A3B_5C7D)),
            "(int)p0 == 0x1A3B5C7D",
        ),
    ];
    for (e, text) in &cases {
        assert_eq!(sharp_in(&typed, &cx, e), *text);
    }
    // A comparison of whole cells needs nothing.
    let wide = kinds(&[(Var::Local(0), Kind::I64)]);
    let whole = Expr::Binary {
        op: BinOp::Eq,
        at: Kind::I64,
        l: Box::new(local(0)),
        r: Box::new(constant_of(0x9A3B_5C7D, Kind::I64)),
    };
    assert_eq!(sharp_in(&wide, &cx, &whole), "local_0 == 0x9A3B5C7D");
    // The form of old.
    let old = Expr::binary(BinOp::Eq, int_p0(), high());
    assert_eq!(readable(&old), "(int)p0 == 0x9A3B5C7D");
}

#[test]
fn a_shift_of_64_bits_says_so_on_a_bare_number() {
    let shift = |at: Kind, kind: Kind| Expr::Binary {
        op: BinOp::Shl,
        at,
        l: Box::new(constant_of(1, kind)),
        r: Box::new(Expr::cast(Scalar::I32, local(1))),
    };
    assert_eq!(sharp(&shift(Kind::I64, Kind::I64)), "1L << (int)local_1");
    assert_eq!(sharp(&shift(Kind::U64, Kind::U64)), "1UL << (int)local_1");
    let cell = shift(Kind::Unknown, Kind::Unknown);
    assert_eq!(sharp(&cell), "1L << (int)local_1");
    // ⚠ A shift of 32 bits stays bare: `1L` there would shift 64.
    let narrow = Expr::binary(BinOp::Shl, int(1), Expr::cast(Scalar::I32, local(1)));
    assert_eq!(sharp(&narrow), "1 << (int)local_1");
    // A variable on the left states its own width.
    let var = Expr::Binary {
        op: BinOp::Shl,
        at: Kind::I64,
        l: Box::new(local(0)),
        r: Box::new(int(3)),
    };
    assert_eq!(sharp(&var), "local_0 << 3");
    let old = readable(&shift(Kind::I64, Kind::I64));
    assert_eq!(old, "1 << (int)local_1", "the form of old");
}

#[test]
fn a_reference_is_compared_with_null() {
    let class = Kind::Object("type_00000000000000AA".to_owned());
    let object = kinds(&[
        (Var::Local(0), class),
        (Var::Local(2), Kind::String),
        (Var::Local(3), Kind::Pointer),
    ]);
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    let as_cast = || intrinsic("try_cast<type_00000000000000DD>", vec![local(1)]);
    let signed = Expr::Binary {
        op: BinOp::Gt,
        at: Kind::I64,
        l: Box::new(local(0)),
        r: Box::new(int(0)),
    };
    let cases = [
        (
            Expr::binary(BinOp::Gt, Expr::cast(Scalar::U64, as_cast()), int(0)),
            "(local_1 as type_00000000000000DD) != null",
        ),
        (
            Expr::binary(BinOp::Eq, as_cast(), int(0)),
            "(local_1 as type_00000000000000DD) == null",
        ),
        (Expr::binary(BinOp::Ne, local(0), int(0)), "local_0 != null"),
        (Expr::binary(BinOp::Eq, local(2), int(0)), "local_2 == null"),
        // A pointer is compared with `null` as C# compares one.
        (Expr::binary(BinOp::Ne, local(3), int(0)), "local_3 != null"),
        // A cell of no known kind is compared with the number, and a signed order is no null test.
        (Expr::binary(BinOp::Eq, local(1), int(0)), "local_1 == 0"),
        (signed, "local_0 > 0"),
        (Expr::binary(BinOp::Eq, local(0), int(5)), "local_0 == 5"),
    ];
    for (e, text) in &cases {
        assert_eq!(sharp_in(&object, &cx, e), *text);
    }
    // A reference tested for being set is one tested against `null`.
    assert_eq!(sharp_cond_in(&object, &truth(local(0))), "local_0 != null");
    let unset = not(truth(local(0)));
    assert_eq!(sharp_cond_in(&object, &unset), "local_0 == null");
    assert_eq!(sharp_cond_in(&object, &truth(local(1))), "local_1");
    // The form of old compares with the number.
    let set = Expr::binary(BinOp::Ne, local(0), int(0));
    assert_eq!(text_in(&object, &names, &set, false), "local_0 != 0");
}

#[test]
fn true_c_sharp_names_a_call_by_the_type_and_the_names_supplied() {
    let virtual_call = |class: &str, ty: &str, slot: u32, args: Vec<Expr>| Expr::Call {
        callee: Callee::Virtual {
            class: class.to_owned(),
            method: format!("Method_{slot}"),
            ty: ty.to_owned(),
            slot,
        },
        args,
        returns: Some(1),
    };
    // ⚠ The listing's wrapper class is no type: the type the slot is of is.
    let dispose = virtual_call(
        "System_IDisposable",
        "System.IDisposable",
        0,
        vec![local(0)],
    );
    assert_eq!(sharp(&dispose), "System.IDisposable.Method_0(local_0)");
    assert_eq!(readable(&dispose), "System_IDisposable.Method_0(local_0)");

    // A receiver's type no longer makes a call a method's: the receivers pass does.
    let class = "type_00000000000000AA";
    let update = virtual_call(class, class, 3, vec![param(0), local(1)]);
    let typed = kinds(&[(Var::Param(0), Kind::Object(class.to_owned()))]);
    let mut names = Names::default();
    let plain = sharp_in(&typed, &at(&names, Level::Structured), &update);
    assert_eq!(plain, "type_00000000000000AA.Method_3(p0, local_1)");
    names
        .methods
        .insert((TypeKey(0xAA), 3), "Update".to_owned());
    names.types.insert(0xAA, "CThing".to_owned());
    let named = sharp_in(&typed, &at(&names, Level::Structured), &update);
    assert_eq!(named, "CThing.Update(p0, local_1)");

    // A .NET primitive is its keyword wherever a type is named.
    let int32 = VmType {
        name: "System.Int32".to_owned(),
        byte_size: 4,
        id: 0x8000_0003,
        flags: 0,
    };
    let array = Expr::NewArray {
        ty: int32,
        len: Box::new(int(72)),
        wrapper: "NewArray_3".to_owned(),
    };
    let compare = Expr::Call {
        callee: Callee::Builtin {
            path: "System.Int32.CompareTo".to_owned(),
            ty: 3,
            method: 1,
        },
        args: vec![local(0), local(1)],
        returns: Some(1),
    };
    let boxed = intrinsic("box<System.Int32>", vec![local(0)]);
    for (e, new, old) in [
        (&array, "new int[72]", "new System.Int32[72]"),
        (
            &compare,
            "int.CompareTo(local_0, local_1)",
            "System.Int32.CompareTo(local_0, local_1)",
        ),
        (&boxed, "box<int>(local_0)", "box<System.Int32>(local_0)"),
    ] {
        assert_eq!(sharp(e), new);
        assert_eq!(readable(e), old);
    }
}

#[test]
fn a_function_goes_by_the_users_name_at_its_head_and_wherever_it_is_named() {
    let mut names = Names::default();
    names.programs.insert(0xAB, "shared".to_owned());
    let own = FunctionKey::new(0x1122_3344, 0x40);
    names.functions.insert(own, "DoThing".to_owned());
    let other = FunctionKey::new(0xAB, 0xB0);
    names.functions.insert(other, "Helper".to_owned());
    let call = Expr::Call {
        callee: Callee::Local {
            at: 0x40,
            name: "func_000040".to_owned(),
        },
        args: vec![],
        returns: Some(0),
    };
    let import = Expr::Call {
        callee: Callee::Import {
            module: "module_0x00000000000000EE".to_owned(),
            name: "func_0000B0".to_owned(),
            program: 0xAB,
            at: 0xB0,
        },
        args: vec![],
        returns: Some(0),
    };
    let reference = Expr::FunctionRef {
        program: 0x1122_3344,
        at: 0x40,
    };
    let head = head();
    let cx = at(&names, Level::Structured);
    for (e, new, old) in [
        (&call, "DoThing()", "func_000040()"),
        (&import, "shared.Helper()", "shared.func_0000B0()"),
        (&reference, "&DoThing", "&func_000040"),
    ] {
        assert_eq!(sharp_in(&head, &cx, e), new);
        assert_eq!(text_in(&head, &names, e, false), old);
    }

    // A head whose name is not its offset says the offset after it, always.
    let f = function_of(head.clone(), vec![tree::Stmt::Return(None)]);
    let named = written_in(&f, &cx);
    let first = "void DoThing(Cell p0) // func_000040\n";
    assert!(named.starts_with(first), "{named}");
    let mut entry = head.clone();
    entry.name = "main".to_owned();
    entry.at = 0x80;
    let main = function_of(entry, vec![tree::Stmt::Return(None)]);
    let entered = written_in(&main, &cx);
    let first = "void main(Cell p0) // func_000080\n";
    assert!(entered.starts_with(first), "{entered}");
    // One that is its offset says nothing more, and the form of old never does.
    let unnamed = written_sharp(&f);
    let first = "void func_000040(Cell p0)\n";
    assert!(unnamed.starts_with(first), "{unnamed}");
    let old = written(&main, &names);
    assert!(old.starts_with("void main(var p0)\n"), "{old}");

    // A function with no body is a comment under its offset.
    let mut unread = head;
    unread.incomplete = Some("Widths are underdetermined.".to_owned());
    assert_eq!(
        written_sharp(&function_of(unread, Vec::new())),
        "// func_000040: no body (Widths are underdetermined.)\n"
    );
}

#[test]
fn a_parameter_of_several_cells_is_written_once() {
    let mut head = head();
    head.params = 4;
    let mut vector = VarInfo::new(Width::Cells);
    vector.kind = Kind::Vector;
    head.vars.insert(Var::Param(1), vector);
    let quiet = function_of(head.clone(), vec![tree::Stmt::Return(None)]);
    let text = written_sharp(&quiet);
    let first = "void func_000040(Cell p0, Vector3 p1, Cell p3)\n";
    assert!(text.starts_with(first), "{text}");
    // ⚠ A slot the body names is a parameter of its own, and is never dropped.
    let used = simple(Stmt::Eval(native("USE", vec![param(2)])));
    let named = written_sharp(&function_of(head.clone(), vec![used]));
    let first = "void func_000040(Cell p0, Vector3 p1, Cell p2, Cell p3)\n";
    assert!(named.starts_with(first), "{named}");
    // So is one a pass knows something of.
    let mut known = head;
    let mut flag = VarInfo::new(Width::Cell);
    flag.kind = Kind::Bool;
    known.vars.insert(Var::Param(2), flag);
    let typed = written_sharp(&function_of(known, vec![tree::Stmt::Return(None)]));
    let first = "void func_000040(Cell p0, Vector3 p1, bool p2, Cell p3)\n";
    assert!(typed.starts_with(first), "{typed}");
    let old = written(&quiet, &Names::default());
    let first = "void func_000040(var p0, Vector3 p1, var p2, var p3)\n";
    assert!(old.starts_with(first), "{old}");
}

#[test]
fn a_statement_of_true_c_sharp_is_broken_at_the_width() {
    let args: Vec<Expr> = (0..30).map(local).collect();
    let test = |n: u32| {
        let object = load(Scalar::U64, param(0), 0x18);
        let field = load(Scalar::U64, object, 0x10 + n);
        let read = Expr::cast(Scalar::I32, field);
        Expr::binary(BinOp::Lt, read, int(1000 + n as i32))
    };
    let either = |l: Expr, r: Expr| Expr::Binary {
        op: BinOp::LogicalOr,
        at: Kind::Bool,
        l: Box::new(l),
        r: Box::new(r),
    };
    let last = either(test(3), test(4));
    let chain = either(test(0), either(test(1), either(test(2), last)));
    let body = vec![tree::Stmt::If {
        cond: chain,
        then: vec![simple(Stmt::Eval(native("USE", args)))],
        els: Vec::new(),
    }];
    let f = function_of(head(), body);
    let text = written_sharp(&f);
    let fits = |line: &str| line.chars().count() <= 120;
    assert!(text.lines().all(fits), "{text}");
    // A chain of tests breaks before every operator, and a list after a comma where a line is full.
    let head = "    if ((int)p0->f_18->f_10 < 1000
        || (int)p0->f_18->f_11 < 1001
        || (int)p0->f_18->f_12 < 1002
        || (int)p0->f_18->f_13 < 1003
        || (int)p0->f_18->f_14 < 1004)
    {
        DEMO.USE(local_0, local_1,";
    assert!(text.contains(head), "{text}");
    assert!(text.contains(",\n            local_"), "{text}");
    assert!(text.contains("local_29);\n    }\n"), "{text}");

    // The form of old breaks a chain of tests and nothing else.
    let old = written(&f, &Names::default());
    let broken = "\n        || (int)p0->f_18->f_14 < 1004)\n";
    assert!(old.contains(broken), "{old}");
    assert!(old.lines().any(|line| !fits(line)), "{old}");
}

#[test]
fn a_listing_of_true_c_sharp_opens_with_one_fact_a_line() {
    let (listing, functions) = read(sample::LISTING);
    let stats = Stats {
        functions: 5,
        incomplete: 1,
        gotos_in: 3,
        gotos_out: 2,
        ..Stats::default()
    };
    let source = |cx: &Context<'_>| {
        let mut writer = SourceWriter::in_context(cx);
        for f in &functions {
            writer.function(f);
        }
        writer.finish(&listing, &stats)
    };
    let names = Names::default();
    let text = source(&at(&names, Level::Structured));
    let words = words(&Options::at(Level::Structured));
    let head = format!(
        "\
// OpenRage: GTA VI script, program 0x11223344
// read from: a lifted listing, the text another decompiler wrote of this script
// level: structured; passes off: none
// imports: 0x0A0B0C0D, 0x1A2B3C4D
// natives: 3 declared: 2 named by the program itself, 1 shown by hash; no natives list names a native
// names: no names list supplied
// 5 function(s), 1 with no body; 2 goto(s) left
{words}
void func_000000(Cell p0, Cell p1)
"
    );
    assert!(text.starts_with(&head), "{text}");
    let next = "}\n\nCell func_000100(Cell p0)\n{\n";
    assert!(text.contains(next), "{text}");
    let unread = "\n// func_000300: no body (Widths are underdetermined.)\n";
    assert!(text.contains(unread), "{text}");
    let again = source(&at(&names, Level::Structured));
    assert_eq!(text, again, "the same bytes twice");
    // No path, no date and no token that spans a line: every line of the header is a comment.
    let top: Vec<&str> = text.lines().take_while(|line| !line.is_empty()).collect();
    assert_eq!(top.len(), 7 + words.lines().count());
    assert!(top.iter().all(|line| line.starts_with("// ")), "{text}");
    assert!(top.iter().all(|line| line.chars().count() <= 120), "{text}");

    // A pass switched off is named, in the order of `Pass::ALL`.
    let off = Options::at(Level::Structured)
        .without(Pass::RoleNames)
        .without(Pass::Gotos);
    let partly = source(&Context::new(&names, Project::empty(), off));
    let said = "\n// level: structured; passes off: gotos, role-names\n";
    assert!(partly.contains(said), "{partly}");

    // The lists that named things are named by their labels.
    let listed = NativeName {
        namespace: Some("LISTED".to_owned()),
        name: "NAMED_BY_THE_LIST".to_owned(),
        ..NativeName::default()
    };
    let mut named = Names {
        natives: Some(NativeDb::from_names("a test's", [(0xBB, listed)])),
        ..Names::default()
    };
    let list = "0x11223344 demo.program\n0x0A0B0C0D shared\n";
    named.read_list("names.txt", list).expect("the list reads");
    let text = source(&at(&named, Level::Structured));
    let head = "\
// OpenRage: GTA VI script, program 0x11223344 (demo.program)
// read from: a lifted listing, the text another decompiler wrote of this script
// level: structured; passes off: none
// imports: 0x0A0B0C0D (shared), 0x1A2B3C4D
// natives: 3 declared: 1 named by `a test's`, 2 by the program itself, 0 shown by hash
// names: programs from `names.txt` (0 proved by hash, 2 on trust)
";
    assert!(text.starts_with(head), "{text}");
}

#[test]
fn a_header_says_how_each_native_is_shown_what_the_lists_named_and_what_its_words_mean() {
    let names = Names::default();
    let cx = at(&names, Level::Source);
    let stats = Stats::default();
    let mut hashed = BTreeMap::new();
    hashed.insert(0xA1u64, false);
    // ⚠ A compiled program names no native: the ones no list reaches are known by an address alone.
    let compiled = |natives: &BTreeMap<u64, bool>, by_address: usize| {
        let read = Read {
            front: Some(PROGRAM),
            natives,
            by_address,
            counted: "called",
        };
        header(0x1122_3344, &[], &read, &cx, &stats)
    };
    let both = "\n// natives: 4 called: 1 shown by hash, 3 shown by the address of their handler; \
                no natives list names a native\n";
    let text = compiled(&hashed, 3);
    assert!(text.contains(both), "{text}");
    let addresses = "\n// natives: 2 called: 2 shown by the address of their handler; \
                     no natives list names a native\n";
    let text = compiled(&BTreeMap::new(), 2);
    assert!(text.contains(addresses), "{text}");
    assert!(
        !text.contains("by hash") && !text.contains("itself"),
        "{text}"
    );

    // What the passes on write, the words say: a pass off takes its sentence with it.
    assert!(text.contains("unchecked((int)0x89ABCDEF)"), "{text}");
    assert!(text.contains("typeof(T).TypeHandle"), "{text}");
    assert!(text.contains("derived"), "{text}");
    let bare = Options::baseline().without(Pass::Names);
    let said = words(&bare);
    assert!(
        said.starts_with("// words: Cell is one 64-bit cell"),
        "{said}"
    );
    for gone in ["unchecked", "TypeHandle", "derived"] {
        assert!(!said.contains(gone), "{said}");
    }

    // Each kind of thing a list named is said, and a long line of lists is broken at the width.
    let mut named = Names::default();
    named.types.insert(1, "CThing".to_owned());
    named.fields.insert((TypeKey(1), 8), "zq_held".to_owned());
    named.texts.insert(5, "zq_made".to_owned());
    named
        .builtins
        .insert((1, 2), "System.Object.ToString".to_owned());
    named.lists.push("one.tsv".to_owned());
    let cx = at(&named, Level::Source);
    let read = Read {
        front: None,
        natives: &BTreeMap::new(),
        by_address: 0,
        counted: "called",
    };
    let text = header(0x1122_3344, &[], &read, &cx, &stats);
    let kinds = "\n// names: types, fields, the texts of hashes and the runtime's methods from \
                 `one.tsv`\n";
    assert!(text.contains(kinds), "{text}");
    for n in 0..40 {
        named.lists.push(format!("list_{n}.tsv"));
    }
    let cx = at(&named, Level::Source);
    let text = header(0x1122_3344, &[], &read, &cx, &stats);
    assert!(text.contains("`list_39.tsv`\n"), "{text}");
    assert!(text.contains("\n//   `list_"), "{text}");
    assert!(
        text.lines().all(|line| line.chars().count() <= 120),
        "{text}"
    );
    assert!(text.lines().all(|line| line.starts_with("// ")), "{text}");
}

#[test]
fn what_stands_at_the_address_of_an_element_is_the_element() {
    let names = Names::default();
    let cx = at(&names, Level::Source);
    let array = |of: Kind| kinds(&[(Var::Local(0), Kind::Array(Box::new(of)))]);
    let cells = array(Kind::U64);
    let element = || intrinsic("array_element_address", vec![local(0), local(1)]);
    let whole = load(Scalar::U64, element(), 0);
    assert_eq!(sharp_in(&cells, &cx, &whole), "local_0[local_1]");
    // What the element points at is reached through it.
    let through = load(Scalar::U64, load(Scalar::U64, element(), 0), 0x18);
    assert_eq!(sharp_in(&cells, &cx, &through), "local_0[local_1]->f_18");
    let set = Stmt::Store {
        kind: StoreKind::U8,
        addr: element(),
        offset: 0,
        value: local(2),
    };
    let stored = |head: &Head| {
        let mut out = String::new();
        Writer::in_context(&mut out, &cx, head).simple(&set)
    };
    let bytes = array(Kind::U8);
    assert_eq!(stored(&bytes), "local_0[local_1] = (byte)local_2");
    let byte = load(Scalar::U8, element(), 0);
    assert_eq!(sharp_in(&bytes, &cx, &byte), "(byte)local_0[local_1]");

    // ⚠ Fewer bytes than the element holds are a part of it, and so is any access of an
    // element whose width nothing states: `a[i] = v` would replace the whole element.
    assert_eq!(stored(&cells), "(&local_0[local_1])->f_0 = (byte)local_2");
    assert_eq!(
        sharp_in(&cells, &cx, &byte),
        "(byte)(&local_0[local_1])->f_0"
    );
    let pair = array(Kind::Aggregate(2));
    assert_eq!(sharp_in(&pair, &cx, &whole), "(&local_0[local_1])->f_0");
    let unknown = head();
    assert_eq!(sharp_in(&unknown, &cx, &whole), "(&local_0[local_1])->f_0");
    assert_eq!(stored(&unknown), "(&local_0[local_1])->f_0 = (byte)local_2");

    // ⚠ A byte further into the element is no member of what the element holds: the address stays.
    let further = load(Scalar::U64, element(), 8);
    assert_eq!(sharp_in(&cells, &cx, &further), "(&local_0[local_1])->f_8");
    // With `fields` off, and in the form of old, the load is written as the code makes it.
    let off = Context::new(
        &names,
        Project::empty(),
        Options::default().without(Pass::Fields),
    );
    assert_eq!(sharp_in(&cells, &off, &whole), "*&local_0[local_1]");
    assert_eq!(readable(&whole), "*&local_0[local_1]");
}

#[test]
fn an_int_alone_where_32_bits_are_taken_stands_without_its_cast() {
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    let head = kinds(&[(Var::Local(0), Kind::I32), (Var::Local(1), Kind::Unknown)]);
    let stmt = |s: &Stmt| {
        let mut out = String::new();
        Writer::in_context(&mut out, &cx, &head).simple(s)
    };
    // An argument of a native, a variable of 32 bits and a store of four bytes.
    let passed = Stmt::Eval(native("USE", vec![high(), param(0)]));
    assert_eq!(stmt(&passed), "DEMO.USE(0x9A3B5C7D, p0)");
    assert_eq!(stmt(&assign(Var::Local(0), high())), "local_0 = 0x9A3B5C7D");
    let store = |kind, value: Expr| Stmt::Store {
        kind,
        addr: param(0),
        offset: 0x10,
        value,
    };
    assert_eq!(
        stmt(&store(StoreKind::I32, high())),
        "p0->f_10 = 0x9A3B5C7D"
    );

    // ⚠ A whole cell keeps the cast, which says what its upper half is.
    assert_eq!(
        stmt(&assign(Var::Local(1), high())),
        "local_1 = (int)0x9A3B5C7D"
    );
    assert_eq!(
        stmt(&store(StoreKind::U64, high())),
        "p0->f_10 = (int)0x9A3B5C7D"
    );
    let widened = constant_of(0xFFFF_FFFF_9A3B_5C7D, Kind::I64);
    let wide = Stmt::Eval(native("USE", vec![widened]));
    assert_eq!(stmt(&wide), "DEMO.USE((int)0x9A3B5C7D)");
    // ⚠ So does an operand: C# would compare, or add, a `uint`.
    let less = Expr::binary(BinOp::Lt, Expr::cast(Scalar::I32, param(0)), high());
    let tested = Stmt::Eval(native("USE", vec![less]));
    assert_eq!(stmt(&tested), "DEMO.USE((int)p0 < (int)0x9A3B5C7D)");
    // A small number and a positive one are written as they always were.
    let plain = Stmt::Eval(native("USE", vec![int(-1), int(0x1234_ABCD)]));
    assert_eq!(stmt(&plain), "DEMO.USE(-1, 0x1234ABCD)");

    // A float read as 32 bits is the float where exactly 32 bits are kept.
    let bits = || Expr::cast(Scalar::I32, Expr::Const(Const::f32(-1.0)));
    assert_eq!(stmt(&store(StoreKind::I32, bits())), "p0->f_10 = -1.0f");
    assert_eq!(stmt(&assign(Var::Local(0), bits())), "local_0 = -1.0f");
    let handed = Stmt::Eval(native("USE", vec![bits()]));
    assert_eq!(stmt(&handed), "DEMO.USE(0xBF800000)");

    // `signs` off writes the cast again, and the form of old never dropped it.
    let off = Options::at(Level::Structured).without(Pass::Signs);
    let cx = Context::new(&names, Project::empty(), off);
    let mut out = String::new();
    let cast = Writer::in_context(&mut out, &cx, &head).simple(&passed);
    assert_eq!(cast, "DEMO.USE((int)0x9A3B5C7D, p0)");
}

#[test]
fn a_parameter_and_a_result_the_project_proves_32_bits_take_an_int_without_its_cast() {
    use crate::gta6::script::project::{Param, Signature};
    let names = Names::default();
    let param_of = |kind: Kind| Param { kind, cells: 1 };
    let mut project = Project::default();
    let called = Signature {
        params: vec![param_of(Kind::Unknown), param_of(Kind::I32)],
        returns: 0,
        result: Kind::Unknown,
    };
    let own = Signature {
        params: vec![param_of(Kind::Unknown)],
        returns: 1,
        result: Kind::Hash,
    };
    project
        .signatures
        .insert(FunctionKey::new(0x1122_3344, 0x100), called);
    project
        .signatures
        .insert(FunctionKey::new(0x1122_3344, 0x40), own);
    let cx = Context::new(&names, &project, Options::at(Level::Structured));
    let call = |args: Vec<Expr>| {
        let callee = Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        };
        simple(Stmt::Eval(Expr::Call {
            callee,
            args,
            returns: Some(0),
        }))
    };
    let mut head = head();
    head.returns = Returns::Cell;
    let body = vec![
        call(vec![high(), high()]),
        // ⚠ An argument of two cells where the signature has one: nothing after it answers a parameter.
        call(vec![Expr::ZeroSlots(2), high()]),
        tree::Stmt::Return(Some(high())),
    ];
    let f = function_of(head, body);
    let text = written_in(&f, &cx);
    let expected = "\
Hash func_000040(Cell p0)
{
    func_000100((int)0x9A3B5C7D, 0x9A3B5C7D);
    func_000100(default(Cells2), (int)0x9A3B5C7D);
    return 0x9A3B5C7D;
}
";
    assert_eq!(text, expected);

    // With no project nothing is proved of either: every cast stays.
    let alone = written_in(&f, &at(&names, Level::Structured));
    assert!(
        alone.contains("    func_000100((int)0x9A3B5C7D, (int)0x9A3B5C7D);\n"),
        "{alone}"
    );
    assert!(alone.contains("    return (int)0x9A3B5C7D;\n"), "{alone}");
}

#[test]
fn the_type_made_of_a_token_is_typeof_and_the_token_alone_says_it_is_one() {
    let ty = VmType {
        name: "type_00000000000000AB".to_owned(),
        byte_size: 32,
        id: 7,
        flags: 0,
    };
    let token = || Expr::Call {
        callee: Callee::Intrinsic {
            name: TYPE_TOKEN.to_owned(),
            ty: Some(ty.clone()),
            wrapper: None,
        },
        args: Vec::new(),
        returns: Some(1),
    };
    let builtin = |path: &str, args: Vec<Expr>| Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 3,
            method: 5,
        },
        args,
        returns: Some(1),
    };
    let made = builtin("System.Type.GetTypeFromHandle", vec![token()]);
    let (head, names) = (head(), Names::default());
    let cx = at(&names, Level::Source);
    assert_eq!(sharp_in(&head, &cx, &made), "typeof(type_00000000000000AB)");
    assert_eq!(
        sharp_in(&head, &cx, &token()),
        "typeof(type_00000000000000AB).TypeHandle"
    );
    // ⚠ Only that one method of the runtime, and only of a token: any other call keeps its name.
    let other = builtin("System.Type.GetTypeFromName", vec![token()]);
    assert_eq!(
        sharp_in(&head, &cx, &other),
        "System.Type.GetTypeFromName(typeof(type_00000000000000AB).TypeHandle)"
    );
    let of_value = builtin("System.Type.GetTypeFromHandle", vec![local(0)]);
    assert_eq!(
        sharp_in(&head, &cx, &of_value),
        "System.Type.GetTypeFromHandle(local_0)"
    );

    // With `operators` off both are written as the calls they are.
    let off = Context::new(
        &names,
        Project::empty(),
        Options::default().without(Pass::Operators),
    );
    assert_eq!(
        sharp_in(&head, &off, &made),
        "System.Type.GetTypeFromHandle(typeof(type_00000000000000AB))"
    );
    assert_eq!(
        sharp_in(&head, &off, &token()),
        "typeof(type_00000000000000AB)"
    );
    assert_eq!(readable(&token()), "typeof(type_00000000000000AB)");
}

#[test]
fn an_arm_nothing_is_written_for_is_no_else_and_turns_the_test_round_where_it_is_the_first() {
    let used = || simple(Stmt::Eval(native("USE", Vec::new())));
    let hidden = || tree::Stmt::Sugar(Box::new(tree::Sugar::Hidden(vec![used()])));
    let branch = |cond: Expr, then: Vec<tree::Stmt>, els: Vec<tree::Stmt>| {
        function_of(head(), vec![tree::Stmt::If { cond, then, els }])
    };
    let names = Names::default();
    let source = at(&names, Level::Source);
    let body = |f: &tree::Function, cx: &Context<'_>| {
        let text = written_in(f, cx);
        let open = text.find("{\n").map_or(0, |at| at + 2);
        text[open..text.len() - 2].to_owned()
    };
    let turned = "    if (!p0)\n    {\n        DEMO.USE();\n    }\n";
    let empty = branch(param(0), Vec::new(), vec![used()]);
    assert_eq!(body(&empty, &source), turned);
    // A comparison is turned by its operator.
    let less = Expr::binary(
        BinOp::Lt,
        Expr::cast(Scalar::I32, param(0)),
        Expr::cast(Scalar::I32, int(4)),
    );
    let compared = branch(less, Vec::new(), vec![used()]);
    let text = body(&compared, &source);
    assert!(text.starts_with("    if ((int)p0 >= 4)\n"), "{text}");

    // What the compiler added is written for nothing at the source level, and is there below it.
    let added = branch(param(0), vec![hidden()], vec![used()]);
    assert_eq!(body(&added, &source), turned);
    let below = body(&added, &at(&names, Level::Structured));
    assert!(below.starts_with("    if (p0)\n"), "{below}");
    assert!(below.contains("    else\n"), "{below}");
    let kept = "    if (p0)\n    {\n        DEMO.USE();\n    }\n";
    let trailing = branch(param(0), vec![used()], vec![hidden()]);
    assert_eq!(body(&trailing, &source), kept);

    // ⚠ An `if` with nothing in either arm still makes its test: it stays.
    let bare = branch(param(0), Vec::new(), Vec::new());
    assert_eq!(body(&bare, &source), "    if (p0)\n    {\n    }\n");
    // The form of old writes both arms as they are.
    let old = written(&empty, &names);
    assert!(
        old.contains("    if (p0)\n    {\n    }\n    else\n"),
        "{old}"
    );
}

#[test]
fn the_first_cells_of_a_variable_of_several_are_its_first_field_and_not_the_variable() {
    let names = Names::default();
    let cx = at(&names, Level::Source);
    let mut head = head();
    let mut three = VarInfo::new(Width::Cells);
    three.kind = Kind::Aggregate(3);
    head.vars.insert(Var::Local(0), three);
    let own = || Expr::AddrOf {
        var: Var::Local(0),
        cell: 0,
    };
    // ⚠ The name of a variable of three cells is all three: one cell of it says which.
    let first = load(Scalar::U64, own(), 0);
    assert_eq!(sharp_in(&head, &cx, &first), "local_0.f_0");
    let set = Stmt::Store {
        kind: StoreKind::U64,
        addr: own(),
        offset: 0,
        value: param(0),
    };
    let mut out = String::new();
    let stored = Writer::in_context(&mut out, &cx, &head).simple(&set);
    assert_eq!(stored, "local_0.f_0 = p0");
    // Every cell of it is the variable, and a variable of one cell is its name.
    let all = Expr::LoadBytes {
        addr: Box::new(own()),
        offset: 0,
        bytes: 24,
    };
    assert_eq!(sharp_in(&head, &cx, &all), "(Cells3)local_0");
    let one = Expr::AddrOf {
        var: Var::Local(1),
        cell: 0,
    };
    assert_eq!(sharp_in(&head, &cx, &load(Scalar::U64, one, 0)), "local_1");
    // The form of old is as it was.
    assert_eq!(text_in(&head, &names, &first, false), "local_0");
}

#[test]
fn a_clear_of_every_byte_of_a_variable_is_its_default_and_may_declare_it() {
    let clear = |var: Var, bytes: u32| Stmt::ZeroBytes {
        addr: address(var, 0),
        bytes,
    };
    let mut head = head();
    let mut pair = VarInfo::new(Width::Cells);
    pair.kind = Kind::Aggregate(2);
    head.vars.insert(Var::Local(0), pair);
    head.vars.insert(Var::Local(1), VarInfo::new(Width::Cell));
    let use_both = Stmt::Eval(native("USE2", vec![local(0), local(1)]));
    let body = vec![
        simple(clear(Var::Local(0), 16)),
        simple(clear(Var::Local(1), 8)),
        simple(use_both.clone()),
        // Fewer bytes than the variable holds leave the rest of it.
        simple(clear(Var::Local(1), 4)),
        simple(use_both),
    ];
    let f = function_of(head, body);
    let text = written_sharp(&f);
    assert!(text.contains("Cells2 local_0 = default;"), "{text}");
    // A local whose address is taken elsewhere is declared at the top, as any such local is.
    assert!(text.contains("    local_1 = default;"), "{text}");
    assert!(text.contains("zero_bytes(&local_1, 4);"), "{text}");
    // The readable form is read back, and keeps the clear as it is.
    let names = Names::default();
    let plain = written(&f, &names);
    assert!(plain.contains("zero_bytes(&local_0, 16)"), "{plain}");
}

#[test]
fn values_of_one_type_boxed_to_be_compared_or_converted_read_as_the_operator_and_the_cast() {
    let (small, wide) = (
        TypeKey(0x0000_0001_0000_0E01),
        TypeKey(0x0000_0001_0000_0E02),
    );
    let mut project = Project::default();
    let enumeration =
        |byte_size: u32, constants: Vec<i64>| crate::gta6::script::project::TypeFacts {
            flags: 0x08,
            byte_size,
            constants,
            ..Default::default()
        };
    project.types.insert(small, enumeration(2, vec![0, 1]));
    project.types.insert(wide, enumeration(4, vec![0, 1]));
    let boxed = |ty: TypeKey, v: Expr| intrinsic(&format!("box<{ty}>"), vec![v]);
    let builtin = |path: &str, args: Vec<Expr>| Expr::Call {
        callee: Callee::Builtin {
            path: path.to_owned(),
            ty: 0,
            method: 0,
        },
        args,
        returns: Some(1),
    };
    let equals = |a: TypeKey, b: TypeKey| {
        let args = vec![boxed(a, local(0)), boxed(b, local(1))];
        builtin("System.Object.Equals", args)
    };
    let converted = |ty: TypeKey| builtin("System.Convert.ToInt32", vec![boxed(ty, local(0))]);
    let names = Names::default();
    let written = |e: &Expr, options: Options, cond: bool| {
        let cx = Context::new(&names, &project, options);
        let head = head();
        let mut out = String::new();
        let mut writer = Writer::in_context(&mut out, &cx, &head);
        match cond {
            true => writer.cond(e, 0),
            false => writer.expr(e, 0),
        }
    };
    let on = Options::default();
    let tested = Expr::binary(
        BinOp::Ne,
        Expr::cast(Scalar::U32, equals(small, small)),
        int(0),
    );
    assert_eq!(written(&tested, on, true), "local_0 == local_1");
    assert_eq!(written(&converted(small), on, false), "(int)local_0");
    // Two types, an enum of four bytes that may hold a value past an `int`'s, and the name off.
    let apart = equals(small, wide);
    assert!(
        written(&apart, on, false).contains("Equals("),
        "{}",
        written(&apart, on, false)
    );
    assert!(written(&converted(wide), on, false).contains("ToInt32("));
    let off = on.without(Pass::Boxing);
    assert!(written(&equals(small, small), off, false).contains("Equals("));
}

#[test]
fn a_global_every_access_of_which_holds_a_reference_is_compared_with_null() {
    let global = Var::Global {
        program: 0x1122_3344,
        offset: 0x10,
        statics: false,
    };
    let mut project = Project::default();
    let info = crate::gta6::script::project::ProgramInfo {
        global_cells: vec![crate::gta6::script::project::FieldFacts {
            offset: 0x10,
            bytes: 8,
            kind: Kind::Object("type_00000000000000AA".to_owned()),
        }],
        ..Default::default()
    };
    project.programs.insert(0x1122_3344, info);
    let names = Names::default();
    let tested = Expr::binary(BinOp::Eq, Expr::Var(global), int(0));
    let written = |project: &Project| {
        let cx = Context::new(&names, project, Options::at(Level::Structured));
        let head = head();
        let mut out = String::new();
        Writer::in_context(&mut out, &cx, &head).cond(&tested, 0)
    };
    assert!(
        written(&project).ends_with(" == null"),
        "{}",
        written(&project)
    );
    assert!(
        written(Project::empty()).ends_with(" == 0"),
        "{}",
        written(Project::empty())
    );
}

#[test]
fn a_native_every_program_only_tests_the_result_of_is_tested_as_a_truth() {
    let asked = || {
        let mut call = native("ASK", vec![local(0)]);
        if let Expr::Call { returns, .. } = &mut call {
            *returns = Some(1);
        }
        call
    };
    let tested = Expr::binary(BinOp::Ne, Expr::cast(Scalar::U32, asked()), int(0));
    let unset = Expr::binary(BinOp::Eq, Expr::cast(Scalar::U32, asked()), int(0));
    let mut project = Project::default();
    project.native_truths.insert(0xA1);
    let names = Names::default();
    let written = |project: &Project, options: Options, e: &Expr| {
        let cx = Context::new(&names, project, options);
        let head = head();
        let mut out = String::new();
        Writer::in_context(&mut out, &cx, &head).cond(e, 0)
    };
    let on = Options::at(Level::Structured);
    assert_eq!(written(&project, on, &tested), "DEMO.ASK(local_0)");
    assert_eq!(written(&project, on, &unset), "!DEMO.ASK(local_0)");
    // Of no such project, or with the name off, the test stays.
    let kept = written(Project::empty(), on, &tested);
    assert!(kept.ends_with(" != 0"), "{kept}");
    let off = written(&project, on.without(Pass::Interproc), &tested);
    assert!(off.ends_with(" != 0"), "{off}");
}

#[test]
fn the_bits_of_a_round_float_in_a_constant_of_no_proved_kind_are_said_beside_it() {
    assert_eq!(sharp(&int(0x3F80_0000)), "0x3F800000 /* 1.0f */");
    assert_eq!(
        sharp(&int(0xBF80_0000u32 as i32)),
        "(int)0xBF800000 /* -1.0f */"
    );
    // Bits of no round float, a float, and a hash say nothing more.
    assert_eq!(sharp(&high()), "(int)0x9A3B5C7D");
    assert!(!sharp(&constant_of(0x3F80_0000, Kind::F32)).contains("/*"));
    assert!(!sharp(&constant_of(0x3F80_0000, Kind::Hash)).contains("/*"));
}

#[test]
fn two_whole_variables_compared_byte_by_byte_are_compared_with_equals_signs() {
    let mut head = head();
    for (n, kind) in [
        (0, Kind::Aggregate(2)),
        (2, Kind::Aggregate(2)),
        (4, Kind::Vector),
        (6, Kind::Vector),
    ] {
        let mut info = VarInfo::new(Width::Cells);
        info.kind = kind;
        head.vars.insert(Var::Local(n), info);
    }
    let compared = |name: &str, a: u32, b: u32| {
        let args = vec![address(Var::Local(a), 0), address(Var::Local(b), 0)];
        intrinsic(name, args)
    };
    let names = Names::default();
    let cx = at(&names, Level::Source);
    let tested = Expr::binary(BinOp::Ne, compared("bytes_equal_16", 0, 2), int(0));
    let mut out = String::new();
    assert_eq!(
        Writer::in_context(&mut out, &cx, &head).cond(&tested, 0),
        "local_0 == local_2"
    );
    assert_eq!(
        sharp_in(&head, &cx, &compared("bytes_equal_12", 4, 6)),
        "local_4 == local_6"
    );
    // Fewer bytes than the variables hold are a part of each.
    let part = sharp_in(&head, &cx, &compared("bytes_equal_8", 0, 2));
    assert!(part.starts_with("bytes_equal_8("), "{part}");
}

#[test]
fn bytes_taken_out_of_a_variable_are_its_field_read_as_the_unsigned_number_they_make() {
    let mut head = head();
    let mut pair = VarInfo::new(Width::Cells);
    pair.kind = Kind::Aggregate(2);
    head.vars.insert(Var::Local(0), pair);
    let taken = |offset: u32, bytes: u32| Expr::ExtractBytes {
        value: Box::new(local(0)),
        offset,
        bytes,
    };
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    assert_eq!(sharp_in(&head, &cx, &taken(8, 4)), "(uint)local_0.f_8");
    assert_eq!(sharp_in(&head, &cx, &taken(9, 1)), "(byte)local_0.f_9");
    assert_eq!(sharp_in(&head, &cx, &taken(8, 8)), "local_0.f_8");
    // Bytes no number is as wide as stay what they are.
    assert_eq!(
        sharp_in(&head, &cx, &taken(0, 12)),
        "extract_bytes(local_0, 0, 12)"
    );
}

#[test]
fn a_variable_used_apart_in_sibling_blocks_each_entered_at_its_assignment_is_declared_in_each() {
    let simple = |s: Stmt| tree::Stmt::Simple(s);
    let used = |n: u32| simple(Stmt::Eval(native("USE", vec![local(n)])));
    let apart = |els: Vec<tree::Stmt>| tree::Stmt::If {
        cond: param(0),
        then: vec![simple(assign(Var::Local(0), int(1))), used(0)],
        els,
    };
    let written_with = |els: Vec<tree::Stmt>| {
        let mut head = kinds(&[]);
        head.locals = 1;
        let f = tree::Function {
            head,
            body: vec![apart(els)],
        };
        written(&f, &Names::default())
    };
    let both = written_with(vec![simple(assign(Var::Local(0), int(2))), used(0)]);
    let expected = "\
void func_000040(var p0)
{
    if (p0)
    {
        var local_0 = 1;
        DEMO.USE(local_0);
    }
    else
    {
        var local_0 = 2;
        DEMO.USE(local_0);
    }
}
";
    assert_eq!(both, expected);
    // A block that reads the variable before it assigns it reads what the other left, or zero.
    let read_first = written_with(vec![used(0), simple(assign(Var::Local(0), int(2)))]);
    assert!(read_first.contains("    var local_0;\n"), "{read_first}");
}

#[test]
fn either_arm_of_a_choice_where_32_bits_are_taken_stands_without_its_cast() {
    let chosen = Expr::Select {
        cond: Box::new(param(0)),
        then: Box::new(int(0x8C0D_0E0Fu32 as i32)),
        els: Box::new(int(1)),
    };
    let body = vec![tree::Stmt::Simple(assign(Var::Local(0), chosen))];
    let mut head = kinds(&[(Var::Local(0), Kind::I32)]);
    head.locals = 1;
    let f = tree::Function { head, body };
    let names = Names::default();
    let text = written_in(
        &f,
        &Context::new(&names, Project::empty(), Options::default()),
    );
    assert!(text.contains("? 0x8C0D0E0F : 1;"), "{text}");
}

#[test]
fn an_instance_of_a_generic_type_a_list_names_is_written_as_that_type_and_its_arguments() {
    use crate::gta6::script::project::TypeFacts;
    let (of, arg, instance) = (
        TypeKey(0xA0A0_0001),
        TypeKey(0xA0A0_0002),
        TypeKey(0xA0A0_0003),
    );
    let mut project = Project::default();
    let facts = TypeFacts {
        generic: Some((of, vec![arg])),
        ..TypeFacts::default()
    };
    project.types.insert(instance, facts);
    let mut names = Names::default();
    names.types.insert(of.0, "Shelf_1".to_owned());
    names.types.insert(arg.0, "Item".to_owned());
    let mut head = kinds(&[(Var::Local(0), Kind::Object(instance.to_string()))]);
    head.locals = 1;
    let made = Expr::NewObject(crate::gta6::script::ir::VmType {
        name: instance.to_string(),
        byte_size: 16,
        id: 1,
        flags: 0,
    });
    let body = vec![tree::Stmt::Simple(assign(Var::Local(0), made))];
    let f = tree::Function { head, body };
    let text = written_in(&f, &Context::new(&names, &project, Options::default()));
    assert!(
        text.contains("Shelf<Item> local_0 = new Shelf<Item>();"),
        "{text}"
    );
    // Of no list that names the generic type, the instance keeps its own hash.
    names.types.remove(&of.0);
    let text = written_in(&f, &Context::new(&names, &project, Options::default()));
    assert!(text.contains(&format!("new {instance}()")), "{text}");
}
