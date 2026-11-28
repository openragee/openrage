use super::*;

fn of(bits: u64, kind: Kind) -> Const {
    Const { bits, kind }
}

#[test]
fn a_number_is_decimal_where_it_is_small_or_round_and_hex_otherwise() {
    let cases = [
        (Const::i32(3000), "3000"),
        (Const::i32(-1), "-1"),
        (Const::i32(0x1_0000), "65536"),
        // A count of hundreds below 2^24 is a number, which the form of old wrote in hex.
        (Const::i32(90_000), "90000"),
        (Const::i32(-100_000), "-100000"),
        (Const::i32(90_001), "0x00015F91"),
        (Const::i32(0x1234_ABCD), "0x1234ABCD"),
        // ⚠ Not round at or past 2^24: 0x01000000 is 16,777,216, and stays hex.
        (Const::i32(0x0100_0000), "0x01000000"),
        (Const::i32(20_000_000), "0x01312D00"),
        (of(90_000, Kind::U32), "90000"),
        (of(0xFFFF_FFFF, Kind::U32), "0xFFFFFFFF"),
        (of(7, Kind::U32), "7"),
        (of(90_000, Kind::U64), "90000"),
        (of(90_000, Kind::Unknown), "90000"),
        (of(90_001, Kind::I64), "0x15F91"),
        (of(u64::MAX, Kind::U64), "0xFFFFFFFFFFFFFFFF"),
        (of(0, Kind::Pointer), "null"),
        (Const::bool(true), "true"),
    ];
    for (c, text) in &cases {
        assert_eq!(literal(c), *text, "{c:?}");
    }
}

#[test]
fn an_int_with_its_top_bit_set_says_it_is_an_int_and_a_hash_is_always_its_eight_digits() {
    // Eight hex digits with the top bit set are a `uint` to C#.
    let negative = Const::i32(0x8C0D_0E0Fu32 as i32);
    assert_eq!(literal(&negative), "(int)0x8C0D0E0F");
    assert_eq!(readable_const(&negative), "0x8C0D0E0F", "the form of old");
    // The same bits as a `uint` are the literal C# reads them as.
    assert_eq!(literal(&of(0x8C0D_0E0F, Kind::U32)), "0x8C0D0E0F");
    // An `int` widened with its sign into a cell is still that `int`.
    let widened = of(0xFFFF_FFFF_8C0D_0E0F, Kind::I64);
    assert_eq!(literal(&widened), "(int)0x8C0D0E0F");

    assert_eq!(literal(&of(7, Kind::Hash)), "0x00000007");
    assert_eq!(readable_const(&of(7, Kind::Hash)), "7", "the form of old");
    assert_eq!(literal(&of(0x00A1_B2C3, Kind::Hash)), "0x00A1B2C3");
}

#[test]
fn a_float_is_the_shortest_literal_that_reads_back_and_never_a_bare_word() {
    let cases = [
        (Const::f32(15.0), "15.0f"),
        (Const::f32(0.1), "0.1f"),
        (Const::f32(-0.0), "-0.0f"),
        (Const::f32(1e-7), "1e-7f"),
        (Const::f32(f32::INFINITY), "float.PositiveInfinity"),
        (Const::f32(f32::NEG_INFINITY), "float.NegativeInfinity"),
        (of(0x7FC0_0000, Kind::F32), "float_bits(0x7FC00000)"),
        (of(0xFFC0_0001, Kind::F32), "float_bits(0xFFC00001)"),
        (of(2.5f64.to_bits(), Kind::F64), "2.5"),
        (of(1e300f64.to_bits(), Kind::F64), "1e300"),
        (
            of(f64::INFINITY.to_bits(), Kind::F64),
            "double.PositiveInfinity",
        ),
        (
            of(f64::NEG_INFINITY.to_bits(), Kind::F64),
            "double.NegativeInfinity",
        ),
        (
            of(f64::NAN.to_bits(), Kind::F64),
            "double_bits(0x7FF8000000000000UL)",
        ),
    ];
    for (c, text) in &cases {
        assert_eq!(literal(c), *text, "{c:?}");
    }
    // What is written reads back to the same bits.
    for bits in [0x3DCC_CCCDu32, 0x0000_0001, 0x7F7F_FFFF, 0x8000_0000] {
        let text = literal(&of(u64::from(bits), Kind::F32));
        let read = text.strip_suffix('f').and_then(|t| t.parse::<f32>().ok());
        assert_eq!(read.map(f32::to_bits), Some(bits), "{text}");
    }
}

#[test]
fn a_string_keeps_to_its_line() {
    assert_eq!(string("hello"), "\"hello\"");
    assert_eq!(string("a\"b\\c\n\r\t\0"), "\"a\\\"b\\\\c\\n\\r\\t\\0\"");
    // Every character a line could end at, and no `\x`.
    assert_eq!(
        string("\u{1}\u{7F}\u{85}\u{2028}\u{2029}"),
        "\"\\u0001\\u007F\\u0085\\u2028\\u2029\""
    );
    // Any other character is itself.
    assert_eq!(string("é'{}"), "\"é'{}\"");
    for text in ["a\nb", "\u{2028}", "\r"] {
        assert_eq!(string(text).lines().count(), 1, "{text:?}");
    }
}

#[test]
fn a_primitive_takes_its_keyword_and_no_other_name_does() {
    assert_eq!(keyword("System.Int32"), Some("int"));
    assert_eq!(keyword("System.Single"), Some("float"));
    assert_eq!(keyword("System.String"), Some("string"));
    // A name that only ends like one, and a type of the runtime that is no primitive.
    assert_eq!(keyword("Int32"), None);
    assert_eq!(keyword("System.Delegate"), None);
    assert_eq!(keyword("type_0123456789ABCDEF"), None);
}

#[test]
fn an_operator_method_names_its_operator() {
    assert_eq!(operator("op_Equality"), Some(BinOp::Eq));
    assert_eq!(operator("op_Addition"), Some(BinOp::Add));
    assert_eq!(operator("op_Implicit"), None);
    assert_eq!(operator("Equals"), None);
}

#[test]
fn only_a_hex_constant_with_its_top_bit_set_is_a_uint_to_c_sharp() {
    let int = |value: u32| Expr::Const(Const::i32(value as i32));
    let cast = |to, e: Expr| Expr::Cast { to, e: Box::new(e) };
    // Written `(int)0x9A3B5C7D` alone, and `0x9A3B5C7D` as the `uint` it is beside a `uint`.
    assert_eq!(high(&int(0x9A3B_5C7D)), Some(of(0x9A3B_5C7D, Kind::U32)));
    assert_eq!(
        high(&cast(Scalar::U32, int(0x9A3B_5C7D))),
        Some(of(0x9A3B_5C7D, Kind::U32))
    );
    assert_eq!(
        high(&Expr::Const(of(0x9A3B_5C7D, Kind::Hash))),
        Some(of(0x9A3B_5C7D, Kind::Hash))
    );
    // The top bit clear, a small negative number and a round one are an `int` as they are written.
    assert_eq!(high(&int(0x1A3B_5C7D)), None);
    assert_eq!(high(&int(-1i32 as u32)), None);
    assert_eq!(high(&int(-100_000i32 as u32)), None);
    // A float's bits and a reading of fewer bits under the cast are no such constant.
    assert_eq!(high(&Expr::Const(of(0xBF80_0000, Kind::F32))), None);
    assert_eq!(
        high(&cast(Scalar::I32, cast(Scalar::U8, int(0x9A3B_5C7D)))),
        None
    );
}

#[test]
fn an_int_alone_where_32_bits_are_taken_is_its_eight_digits_without_the_cast() {
    // Only what `literal` writes under `(int)`: the same bits, and the cast gone.
    let negative = Const::i32(0x8C0D_0E0Fu32 as i32);
    assert_eq!(literal(&negative), "(int)0x8C0D0E0F");
    assert_eq!(narrow_literal(&negative).as_deref(), Some("0x8C0D0E0F"));
    // What has no cast keeps the literal it has.
    for plain in [
        Const::i32(0x1234_ABCD),
        Const::i32(-1),
        Const::i32(-100_000),
        Const::i32(3000),
        of(0x8C0D_0E0F, Kind::U32),
        of(0x8C0D_0E0F, Kind::Hash),
        Const::f32(-1.0),
    ] {
        assert_eq!(narrow_literal(&plain), None, "{plain:?}");
    }
    // ⚠ A whole cell that is a negative `int` widened keeps the cast: it is what says the upper half.
    for wide in [Kind::I64, Kind::U64, Kind::Unknown] {
        let widened = of(0xFFFF_FFFF_8C0D_0E0F, wide.clone());
        assert_eq!(narrow_literal(&widened), None, "{wide:?}");
        assert_eq!(literal(&widened), "(int)0x8C0D0E0F");
    }
    assert!([Kind::I32, Kind::U32, Kind::Hash, Kind::F32]
        .iter()
        .all(narrow_kind));
    assert!([
        Kind::I64,
        Kind::Unknown,
        Kind::Bool,
        Kind::U16,
        Kind::Pointer
    ]
    .iter()
    .all(|kind| !narrow_kind(kind)));
}

#[test]
fn bytes_taken_out_of_a_group_of_cells_or_a_call_are_a_field_of_the_one_they_lie_in() {
    use crate::gta6::script::ir::{Stmt, StoreKind};
    use crate::gta6::script::print::tests::{at, head, native, param};
    use crate::gta6::script::{Level, Names};
    let names = Names::default();
    let cx = at(&names, Level::Structured);
    let head = head();
    let written = |e: &Expr| {
        let mut out = String::new();
        Writer::in_context(&mut out, &cx, &head).expr(e, 0)
    };
    let taken = |value: Expr, offset: u32, bytes: u32| Expr::ExtractBytes {
        value: Box::new(value),
        offset,
        bytes,
    };
    let alone = || Expr::Cells(vec![param(1)]);
    // One cell gathered alone is that cell, and its bytes its field.
    assert_eq!(written(&taken(alone(), 4, 4)), "(uint)p1.f_4");
    // An integer of the field's own width says the width by its cast alone.
    let read = |to: Scalar, bytes: u32| Expr::Cast {
        to,
        e: Box::new(taken(alone(), 4, bytes)),
    };
    assert_eq!(written(&read(Scalar::I32, 4)), "(int)p1.f_4");
    assert_eq!(written(&read(Scalar::U32, 4)), "(uint)p1.f_4");
    assert_eq!(written(&read(Scalar::I16, 2)), "(short)p1.f_4");
    // Of several cells of one variable each, the cell the bytes lie in.
    let four = || Expr::Cells(vec![param(1), param(2), param(3), param(4)]);
    assert_eq!(written(&taken(four(), 28, 1)), "(byte)p4.f_4");
    assert_eq!(written(&taken(four(), 8, 8)), "p2");
    // Bytes over two cells, or a group one of whose cells is a call's, stay what they are.
    let over = written(&taken(four(), 6, 4));
    assert!(over.starts_with("extract_bytes("), "{over}");
    let called = Expr::Cells(vec![native("MAKE", Vec::new()), param(2)]);
    let dropped = written(&taken(called, 8, 4));
    assert!(dropped.starts_with("extract_bytes("), "{dropped}");
    // A call's value is a value of its own: its field is read off the call.
    let made = taken(native("MAKE", Vec::new()), 8, 4);
    assert_eq!(written(&made), "(uint)DEMO.MAKE().f_8");
    // ⚠ A store of a whole cell says how the narrower number fills it.
    let stored = Stmt::Store {
        kind: StoreKind::U64,
        addr: param(0),
        offset: 0x10,
        value: taken(alone(), 4, 4),
    };
    let mut out = String::new();
    let text = Writer::in_context(&mut out, &cx, &head).simple(&stored);
    assert_eq!(text, "p0->f_10 = (long)(uint)p1.f_4");
}

/// A natives list of one native, `ZQ.ZQ_SET_PACE`, at the hash the tests' calls of a native use, with parameters of the types given.
fn paced(types: &[&str]) -> crate::gta6::script::Names {
    use crate::gta5::script::natives::{NativeDb, NativeName, NativeParam};
    let params = types.iter().enumerate().map(|(n, ty)| NativeParam {
        ty: (*ty).to_owned(),
        name: format!("zq{n}"),
    });
    let listed = NativeName {
        namespace: Some("ZQ".to_owned()),
        name: "ZQ_SET_PACE".to_owned(),
        params: params.collect(),
        returns: None,
    };
    crate::gta6::script::Names {
        natives: Some(NativeDb::from_names("a test's", [(0xA1, listed)])),
        ..crate::gta6::script::Names::default()
    }
}

/// The statement that evaluates `call`, as true C# writes it with the names, the project and the options given.
fn called_in(
    names: &crate::gta6::script::Names,
    project: &crate::gta6::script::Project,
    options: crate::gta6::script::Options,
    call: Expr,
) -> String {
    let cx = crate::gta6::script::Context::new(names, project, options);
    let (head, mut out) = (crate::gta6::script::print::tests::head(), String::new());
    let s = crate::gta6::script::ir::Stmt::Eval(call);
    Writer::in_context(&mut out, &cx, &head).simple(&s)
}

#[test]
fn what_a_natives_list_says_a_parameter_is_gives_zero_cells_their_type_and_bits_their_float() {
    use crate::gta6::script::print::tests::{int, native};
    use crate::gta6::script::{Level, Options, Pass, Project};
    let structured = Options::at(Level::Structured);
    let pace = || {
        let bits = Expr::Const(Const {
            bits: 0x4040_0000,
            kind: Kind::I32,
        });
        native("ZQ_SET_PACE", vec![Expr::ZeroSlots(2), int(7), bits])
    };
    let names = paced(&["Ped", "int", "float"]);
    let text = called_in(&names, Project::empty(), structured, pace());
    assert_eq!(text, "ZQ.ZQ_SET_PACE(default(Handle), 7, 3.0f)");
    let names = paced(&["Vector3", "int", "float"]);
    let text = called_in(&names, Project::empty(), structured, pace());
    assert_eq!(text, "ZQ.ZQ_SET_PACE(default(Vector3), 7, 3.0f)");

    // ⚠ A list that does not answer each argument with one parameter says nothing of any.
    let unsaid = "ZQ.ZQ_SET_PACE(default(Cells2), 7, 0x40400000 /* 3.0f */)";
    let names = paced(&["Ped", "int", "float", "BOOL"]);
    let text = called_in(&names, Project::empty(), structured, pace());
    assert_eq!(text, unsaid);
    let names = paced(&["int", "int", "float"]);
    let text = called_in(&names, Project::empty(), structured, pace());
    assert_eq!(text, unsaid);
    // An integer the list types is no float.
    let names = paced(&["Ped", "int", "int"]);
    let text = called_in(&names, Project::empty(), structured, pace());
    assert_eq!(
        text,
        "ZQ.ZQ_SET_PACE(default(Handle), 7, 0x40400000 /* 3.0f */)"
    );
    // With `kinds` off the arguments are what they were.
    let names = paced(&["Ped", "int", "float"]);
    let off = structured.without(Pass::Kinds);
    assert_eq!(called_in(&names, Project::empty(), off, pace()), unsaid);
}

#[test]
fn what_the_project_proves_a_parameter_is_gives_zero_cells_their_type_and_bits_their_float() {
    use crate::gta6::script::print::tests::{high, int};
    use crate::gta6::script::project::{Param, Signature};
    use crate::gta6::script::{Level, Names, Options, Project};
    let param = |kind: Kind, cells: u32| Param { kind, cells };
    let signature = Signature {
        params: vec![
            param(Kind::F32, 1),
            param(Kind::Handle, 2),
            param(Kind::Unknown, 1),
        ],
        returns: 0,
        result: Kind::Unknown,
    };
    let mut project = Project::default();
    project
        .signatures
        .insert(FunctionKey::new(0x1122_3344, 0x100), signature);
    let call = |args: Vec<Expr>| Expr::Call {
        callee: Callee::Local {
            at: 0x100,
            name: "func_000100".to_owned(),
        },
        args,
        returns: Some(0),
    };
    let (names, structured) = (Names::default(), Options::at(Level::Structured));
    let text_of = |args: Vec<Expr>| called_in(&names, &project, structured, call(args));
    let minus_one = || {
        Expr::Const(Const {
            bits: 0xFFFF_FFFF_BF80_0000,
            kind: Kind::I64,
        })
    };
    // The float only where the parameter is one: one of no proved kind keeps the number.
    let text = text_of(vec![minus_one(), Expr::ZeroSlots(2), minus_one()]);
    let expected = "func_000100(-1.0f, default(Handle), (int)0xBF800000 /* -1.0f */)";
    assert_eq!(text, expected);

    // ⚠ Bits of no float a script states, and a constant whose upper half is no widening, stay numbers.
    let text = text_of(vec![int(5), Expr::ZeroSlots(2), int(1)]);
    assert_eq!(text, "func_000100(5, default(Handle), 1)");
    let odd = Expr::Const(Const {
        bits: 0x0000_0001_3F80_0000,
        kind: Kind::I64,
    });
    let text = text_of(vec![odd, Expr::ZeroSlots(2), high()]);
    assert!(text.starts_with("func_000100(0x13F800000, "), "{text}");
    // ⚠ An argument of other cells than its parameter takes: nothing from it on answers one.
    let text = text_of(vec![Expr::ZeroSlots(2), minus_one()]);
    let expected = "func_000100(default(Cells2), (int)0xBF800000 /* -1.0f */)";
    assert_eq!(text, expected);
}

#[test]
fn a_float_parameter_takes_a_numbers_bits_as_its_float_only_where_a_script_could_state_it() {
    assert_eq!(
        taken_float(&Const::i32(0x3F80_0000)).as_deref(),
        Some("1.0f")
    );
    let tenth = of(0x3DCC_CCCD, Kind::Unknown);
    assert_eq!(taken_float(&tenth).as_deref(), Some("0.1f"));
    // No NaN, no infinity, no number below the normal ones, and never a float or a hash, whose own literal stands.
    let kept = [
        Const::i32(0x7FC0_0000),
        Const::i32(0x7F80_0000),
        Const::i32(1),
        Const::f32(1.0),
        of(0x3F80_0000, Kind::Hash),
    ];
    for c in &kept {
        assert_eq!(taken_float(c), None, "{c:?}");
    }
}
