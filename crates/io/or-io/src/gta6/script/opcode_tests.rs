use super::*;

fn named(name: &str) -> &'static Spec {
    let found = table::TABLE.iter().find(|row| row.name == name);
    found.expect("an opcode of that name")
}

#[test]
fn the_table_rises_by_opcode_and_names_each_once() {
    let rows = table::TABLE;
    assert!(rows.windows(2).all(|pair| pair[0].opcode < pair[1].opcode));
    for row in rows {
        // The decoder holds four operands and deciphers sixteen operand bytes.
        assert!(row.fields.len() <= 4 && row.bytes() <= 18, "{}", row.name);
        assert_eq!(spec(row.opcode).map(|found| found.name), Some(row.name));
        let same = rows.iter().filter(|other| other.name == row.name).count();
        assert_eq!(same, 1, "{}", row.name);
    }
    assert_eq!(name(0), Some("nop"));
    // The first number past the last, and the last a word can spell.
    assert_eq!((name(COUNT), name(u16::MAX)), (None, None));
}

#[test]
fn every_number_the_interpreter_dispatches_on_has_a_row() {
    let rows = table::TABLE;
    assert_eq!(rows.len(), usize::from(COUNT));
    for (number, row) in rows.iter().enumerate() {
        assert_eq!(usize::from(row.opcode), number, "{}", row.name);
    }
    // What the interpreter steps over for instructions no program was seen to hold.
    let stepped = [
        ("load_global_i8", 4),
        ("call_virtual_w", 8),
        ("postinc_local", 3),
        ("push_local_w", 4),
        ("call_virtual_ww", 9),
        ("push_virtual_function_w", 8),
        ("alloc_value_w", 5),
        ("load_static_u16", 7),
        ("op_465", 4),
        ("op_469", 2),
        ("load_vector_o24", 3),
        ("tail_call", 14),
        ("nop_508", 7),
        ("nop_510", 8),
        ("nop_514", 4),
        ("nop_515", 2),
        ("drop_523", 6),
        ("op_528", 6),
        ("nop_529", 7),
    ];
    for (name, bytes) in stepped {
        let row = named(name);
        assert_eq!((row.bytes(), row.status), (bytes, Status::Unseen), "{name}");
    }
    // Of five only the length is known, and one is written by the engine alone.
    let unknown = rows.iter().filter(|row| row.act == Act::Unknown);
    assert_eq!(unknown.count(), 5);
    assert_eq!(named("tail_call").act, Act::TailCall);
    assert_eq!(named("postdec_local").act, Act::Unwritten);
}

#[test]
fn a_length_is_the_word_and_the_bytes_of_the_operands() {
    // The distances a listing's offsets give between such instructions and the next.
    let lengths = [
        ("return", 2),
        ("brz_short", 3),
        ("call_builtin", 4),
        ("call_short", 5),
        ("call_virtual", 6),
        ("push_i32", 6),
        ("call", 7),
        ("call_native", 8),
        ("push_i32x4", 18),
    ];
    for (name, bytes) in lengths {
        assert_eq!(named(name).bytes(), bytes, "{name}");
    }
    assert_eq!(Type::U24.bytes() + Type::S8.bytes() + Type::F32.bytes(), 8);
}

#[test]
fn a_word_spells_its_opcode_and_a_byte_reads_back_under_its_key() {
    // Opcode 1 times 0x843D is the word itself; opcode 2 wraps past sixteen bits.
    assert_eq!(of_word(0x843D), 1);
    assert_eq!(of_word(u16::from_le_bytes([0x7A, 0x08])), 2);
    for row in table::TABLE {
        assert_eq!(of_word(row.opcode.wrapping_mul(0x843D)), row.opcode);
    }
    // 9 times 0x65 is 0x8D under the first key, 0x85.
    assert_eq!(plain(0x08, 0), 9);
    for place in 0..16usize {
        let key = 0x85 ^ 0x11u8.wrapping_mul(place as u8);
        for byte in 0..=u8::MAX {
            assert_eq!(plain(byte.wrapping_mul(0x65) ^ key, place), byte);
        }
    }
    // The key is the place's: the same stored byte reads otherwise one place on.
    assert_ne!(plain(0x08, 1), 9);
    // The keys of the sixteen places an instruction has, the first and the ninth among them.
    assert_eq!((key(0), key(1), key(8), key(15)), (0x85, 0x94, 0x0D, 0x7A));
    assert_eq!(of_word(1u16.wrapping_mul(0x843D)), 1);
    assert_eq!(WORD_MULTIPLIER.wrapping_mul(0x843D), 1);
    assert_eq!(OPERAND_MULTIPLIER.wrapping_mul(0x65), 1);
}

#[test]
fn an_opcode_no_program_holds_is_marked_and_the_others_are_not() {
    assert_eq!(named("add_i32").status, Status::Matched);
    assert_eq!(named("dup").status, Status::Inferred);
    assert_eq!(named("i64_to_i32").status, Status::Inferred);
    assert_eq!(named("ceq_f64").status, Status::Unseen);
    let unseen = table::TABLE
        .iter()
        .filter(|row| row.status == Status::Unseen);
    // 39 that a family places, and 50 that only the interpreter's own steps place.
    assert_eq!(unseen.count(), 89);
}

#[test]
fn a_row_says_what_the_interpreter_does_where_its_place_in_its_family_says_otherwise() {
    // 168 sits with the four tests for equal and is run as the test for unequal of the same type.
    let (among_equals, unequal) = (named("bne_f64_alt"), named("bne_f64"));
    assert_eq!((among_equals.opcode, unequal.opcode), (168, 172));
    assert_eq!(among_equals.act, unequal.act);
    assert_eq!(among_equals.status, Status::Matched);
    assert_eq!(named("beq_f32").opcode, 167);
    assert_ne!(named("beq_f32").act, named("bne_f32").act);
    // 349 and 352 sit with the conversions and are run as the no-op.
    for name in ["i64_to_i32", "u64_to_u32"] {
        assert_eq!(named(name).act, named("nop").act, "{name}");
    }
    assert_ne!(named("i32_to_i64").act, Act::Nop);
}

#[test]
fn a_builtin_has_cells_only_where_the_table_holds_it() {
    assert_eq!(builtin_method(0, 0), Some((1, 0)));
    assert_eq!(builtin_method(0, 1), None);
    assert_eq!(builtin_type(11), Some((4, 0x000D_2005)));
    // Sized from the cells its instructions move, with no flags stated anywhere.
    assert_eq!(builtin_type(16), Some((8, 0)));
    assert_eq!(builtin_type(1), None);
    assert_eq!(builtin_slot(0, 2), Some(1));
    assert_eq!(builtin_slot(0, 0), None);
}

#[test]
fn a_step_states_the_cells_it_pops_and_pushes() {
    let callee = Callee::Local {
        at: 0x10,
        name: "func_000010".to_owned(),
    };
    let unsized_call = Op::Unsized {
        callee: callee.clone(),
        args: Some(2),
        returns: None,
    };
    let call = Op::Call {
        callee,
        args: 2,
        returns: 3,
    };
    assert_eq!(call.stack(), (2, 3));
    // A count that is not stated is given as none: the sum past such a call is not the depth.
    assert_eq!(unsized_call.stack(), (2, 0));
    assert_eq!(Op::Sink { depth: 3 }.stack(), (3, 3));
    assert_eq!(Op::Raise { depth: 4 }.stack(), (4, 4));
    assert_eq!(Op::AddrOfTop { cells: 2 }.stack(), (0, 1));
    assert_eq!(
        Op::LoadBytes {
            offset: 8,
            bytes: 12
        }
        .stack(),
        (1, 2)
    );
    let stored = Op::StoreBytes {
        offset: 0,
        bytes: 16,
        cells: 2,
    };
    assert_eq!(stored.stack(), (3, 0));
    assert!(Op::Return { cells: 1 }.leaves() && Op::Jump { to: 0 }.leaves());
    assert!(!Op::Branch { to: 0 }.leaves() && !call.leaves() && !unsized_call.leaves());
}
