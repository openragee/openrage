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
