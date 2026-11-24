use super::*;
use crate::gta5::script::natives::{NativeDb, NativeName};
use crate::gta6::script::ir::VarInfo;
use crate::gta6::script::sample;
use crate::gta6::script::tree::Label;

fn head() -> Head {
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
fn kinds(of: &[(Var, Kind)]) -> Head {
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

fn local(n: u32) -> Expr {
    Expr::Var(Var::Local(n))
}

fn param(n: u32) -> Expr {
    Expr::Var(Var::Param(n))
}

fn int(value: i32) -> Expr {
    Expr::Const(Const::i32(value))
}

fn constant_of(bits: u64, kind: Kind) -> Expr {
    Expr::Const(Const { bits, kind })
}

fn not(e: Expr) -> Expr {
    Expr::Unary {
        op: UnOp::Not,
        e: Box::new(e),
    }
}

fn truth(e: Expr) -> Expr {
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

fn load(ty: Scalar, addr: Expr, offset: u32) -> Expr {
    Expr::Load {
        ty,
        addr: Box::new(addr),
        offset,
    }
}

fn address(var: Var, cell: u32) -> Expr {
    Expr::AddrOf { var, cell }
}

fn offset(addr: Expr, by: Expr) -> Expr {
    Expr::ByteOffset {
        addr: Box::new(addr),
        offset: Box::new(by),
    }
}

fn intrinsic(name: &str, args: Vec<Expr>) -> Expr {
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

fn native(name: &str, args: Vec<Expr>) -> Expr {
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

fn assign(target: Var, value: Expr) -> Stmt {
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
fn a_step_prints_as_the_listing_wrote_it() {
    let one = Var::Local(1);
    let stepped = |op, from: u32, by| {
        assign(
            one,
            Expr::binary(op, Expr::cast(Scalar::I32, local(from)), int(by)),
        )
    };
    assert_eq!(stmt_of(&stepped(BinOp::Add, 1, 1)), "local_1++");
    assert_eq!(stmt_of(&stepped(BinOp::Sub, 1, 1)), "local_1--");
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
    local_2++;
    DEMO.USE(local_1, local_2);
    for (var local_4 = 0; (int)local_4 < 4; local_4++)
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
// natives: 3 declared, no natives list supplied: 2 named by the listing itself, 1 shown by hash
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
