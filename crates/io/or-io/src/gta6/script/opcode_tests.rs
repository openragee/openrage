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
    // A number between two instructions, and one past the last.
    assert_eq!((name(90), name(505), name(u16::MAX)), (None, None, None));
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
}

#[test]
fn an_opcode_no_program_holds_is_marked_and_the_others_are_not() {
    assert_eq!(named("add_i32").status, Status::Matched);
    assert_eq!(named("dup").status, Status::Inferred);
    assert_eq!(named("beq_f64").status, Status::Inferred);
    assert_eq!(named("ceq_f64").status, Status::Unseen);
    let unseen = table::TABLE
        .iter()
        .filter(|row| row.status == Status::Unseen);
    assert_eq!(unseen.count(), 39);
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
