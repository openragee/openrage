use super::functions;
use super::*;

const BASE: u64 = 0x5000_0000;
const NONE: u32 = u32::MAX;
const PROGRAM: u32 = 0xABCD_0001;
/// Where a built program keeps its pooled data: right after the header.
const BLOB: usize = 0x180;
/// The offset in a built program's pooled data of the end of a base list that names nothing.
const LIST: usize = 4;

const NOP: u16 = 0;
const PUSH_FUNCTION: u16 = 126;
const ENTER: u16 = 131;
const ENTER_FLAGGED: u16 = 132;
const CALL: u16 = 133;
const CALL_SHORT: u16 = 134;
const RETURN: u16 = 144;
const RETURN_1: u16 = 145;
const RETURN_N: u16 = 146;
const ENTER_W: u16 = 398;

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// The pointer to byte `offset` of a built program's pooled data.
fn pooled(offset: usize) -> u64 {
    BASE + (BLOB + offset) as u64
}

/// The file offset the pointer at `at` of a built program holds.
fn pointed(bytes: &[u8], at: usize) -> usize {
    let word = bytes[at..at + 8].iter().rev();
    (word.fold(0, |word, byte| (word << 8) | u64::from(*byte)) - BASE) as usize
}

/// An instruction as the code keeps it, written the other way round from how the reader takes it apart.
fn insn(opcode: u16, operands: &[u8]) -> Vec<u8> {
    let mut bytes = opcode.wrapping_mul(0x843D).to_le_bytes().to_vec();
    let keyed = operands.iter().zip(0u8..);
    bytes.extend(keyed.map(|(byte, n)| byte.wrapping_mul(0x65) ^ 0x85 ^ 0x11u8.wrapping_mul(n)));
    bytes
}

/// Appends a section to a program being built and writes the pointer to it at `at`.
fn section(file: &mut Vec<u8>, at: usize, bytes: &[u8]) {
    let pointer = BASE + file.len() as u64;
    file.extend_from_slice(bytes);
    put64(file, at, pointer);
}

/// Appends a table and writes its count, the room for as many and its pointer from `at`; no record leaves the pointer null.
fn array(file: &mut Vec<u8>, at: usize, records: &[Vec<u8>]) {
    put32(file, at, records.len() as u32);
    put32(file, at + 4, records.len() as u32);
    if !records.is_empty() {
        section(file, at + 8, &records.concat());
    }
}

/// A type record with no base type, an empty base list and no vtable, and what `set` changes.
fn type_record(index: u32, name: u64, set: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let mut r = vec![0; 136];
    put64(&mut r, 0x10, name);
    put32(&mut r, 0x1C, index);
    put32(&mut r, 0x20, NONE);
    put32(&mut r, 0x34, NONE);
    put64(&mut r, 0x50, BASE);
    put64(&mut r, 0x58, pooled(LIST));
    put64(&mut r, 0x60, pooled(LIST));
    set(r.as_mut_slice());
    r
}

/// What a test states of a program; `build` lays it out as the container keeps it.
#[derive(Default)]
struct Parts {
    code: Vec<u8>,
    /// What follows the eight bytes the pooled data of every built program starts with.
    pooled: Vec<u8>,
    types: Vec<Vec<u8>>,
    /// The imports after import 0: a program hash and how many types it has.
    imports: Vec<(u32, u32)>,
    natives: Vec<u64>,
    /// A name hash and a code offset each.
    entries: Vec<(u32, u32)>,
    methods: Vec<Vec<u8>>,
    fields: Vec<Vec<u8>>,
    main: Option<u32>,
    /// An offset in the pooled data and a length each.
    constants: Vec<(u32, u32)>,
    /// The bytes of the program's own globals, of the whole global block and of its own statics.
    sizes: [u32; 3],
    /// Where in the pooled data the map of the globals is; stating it states an order of imports too.
    map: Option<u32>,
}

impl Parts {
    fn build(&self) -> Vec<u8> {
        let mut file = vec![0; 0x180];
        put32(&mut file, 0x10, PROGRAM);
        put32(&mut file, 0x14, 1);
        put32(&mut file, 0x18, 0x6F);
        put64(&mut file, 0x1C, 0x5936_CAB2_EADB_CE67);
        put32(&mut file, 0xBC, self.main.unwrap_or(NONE));
        put32(&mut file, 0xC0, NONE);
        put32(&mut file, 0x11C, self.sizes[0]);
        put32(&mut file, 0x118, self.sizes[1]);
        put32(&mut file, 0x128, self.sizes[2]);
        put32(&mut file, 0x130, NONE);
        if let Some(map) = self.map {
            put32(&mut file, 0x130, 1);
            put32(&mut file, 0x120, 0x10);
            put32(&mut file, 0x124, map);
        }

        // A zero, the order of at most one import, and at `LIST` the end of a base list.
        let mut blob = vec![0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        blob.extend_from_slice(&self.pooled);
        blob.resize(blob.len().max(64), 0);
        put32(&mut file, 0xFC, blob.len() as u32);
        section(&mut file, 0xF0, &blob);
        put32(&mut file, 0xF8, self.code.len() as u32);
        section(&mut file, 0xE8, &self.code);
        section(&mut file, 0x08, &words(&[0, 0, 1, 0]));

        let mut imports = vec![vec![0; 24]];
        let mut next = self.types.len() as u32;
        for (program, types) in &self.imports {
            let mut r = vec![0; 24];
            put32(&mut r, 8, *program);
            put32(&mut r, 16, next);
            put32(&mut r, 20, next + types);
            next += types;
            imports.push(r);
        }
        array(&mut file, 0x140, &imports);

        put32(&mut file, 0xB8, self.natives.len() as u32);
        let natives = self.natives.iter().flat_map(|native| native.to_le_bytes());
        section(&mut file, 0xA8, &natives.collect::<Vec<u8>>());
        section(&mut file, 0xB0, &vec![0u8; self.natives.len() * 4]);

        put32(&mut file, 0xC8, self.entries.len() as u32);
        if !self.entries.is_empty() {
            let entries = self.entries.iter().flat_map(|(name, at)| [*name, *at, 1]);
            section(&mut file, 0x100, &words(&entries.collect::<Vec<u32>>()));
        }
        array(&mut file, 0x58, &self.methods);
        array(&mut file, 0x68, &self.fields);
        array(&mut file, 0x28, &self.types);
        if !self.constants.is_empty() {
            let pairs = self.constants.iter().flat_map(|(at, len)| [*at, *len]);
            let pairs = pairs.chain([0, 0]);
            section(&mut file, 0x98, &words(&pairs.collect::<Vec<u32>>()));
        }
        file
    }

    fn read(&self) -> Result<Program, ProgramError> {
        Program::read(&self.build())
    }
}

/// The least a program is: one function that returns, and one type.
fn small() -> Parts {
    Parts {
        code: insn(RETURN, &[]),
        types: vec![type_record(0, 1, |_| {})],
        ..Parts::default()
    }
}

/// What a refusal says is wrong, and at which byte of the file.
fn refused(read: Result<Program, ProgramError>) -> (usize, String) {
    match read {
        Err(ProgramError::Malformed { at, what }) => (at, what),
        other => panic!("not refused as malformed: {other:?}"),
    }
}

/// Three functions and the padding after them: one with an entry that calls the second, which has none, and one with the long entry.
fn three_functions() -> Vec<u8> {
    let code = [
        insn(ENTER, &[0x21]),
        insn(CALL_SHORT, &[0, 11, 0]),
        insn(RETURN_N, &[3]),
        insn(RETURN_1, &[]),
        insn(ENTER_W, &[17, 0x2C, 0x01, 1]),
        insn(RETURN, &[]),
        insn(NOP, &[]),
    ];
    code.concat()
}

#[test]
fn a_program_reads_to_its_tables() {
    let mut pooled_data = words(&[16, NONE]);
    // 16: the three values of an enum two bytes wide.
    pooled_data.extend([1, 0, 2, 0, 0xFE, 0xFF, 0, 0]);
    // 24: a base list of the base type and one interface; 44: a vtable; 76: two type arguments.
    pooled_data.extend(words(&[0x8000_0000, 0, 5, 4, NONE]));
    pooled_data.extend(words(&[0x0102, 0xFF, 11, 0, 0x77, 0x101, NONE, NONE]));
    pooled_data.extend(words(&[7, 0x8000_0001]));
    // 84: six bytes of constants; 90: the map of the globals, two words of 64 bits.
    pooled_data.extend([1, 2, 3, 4, 5, 6]);
    pooled_data.extend([0b101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    let an_enum = type_record(0, 0x10, |r| {
        put64(r, 0x28, pooled(8));
        put32(r, 0x30, 3);
        put32(r, 0x6C, 0x4B);
        put16(r, 0x70, 2);
    });
    let a_class = type_record(1, 0x20, |r| {
        put32(r, 0x18, 0xBEEF);
        put32(r, 0x20, 0x8000_0000);
        put32(r, 0x30, 1);
        put32(r, 0x34, 3);
        put64(r, 0x58, pooled(44));
        put64(r, 0x60, pooled(24));
        put32(r, 0x68, 76);
        put32(r, 0x6C, 2);
        put16(r, 0x70, 312);
        put16(r, 0x74, 2);
        put16(r, 0x76, 1);
        put16(r, 0x7E, 4);
        put16(r, 0x80, 2);
    });
    let mut method = vec![0; 32];
    put32(&mut method, 0, 0x0C70_0001);
    put32(&mut method, 4, 11);
    put32(&mut method, 8, 0x70);
    put16(&mut method, 0x12, 1);
    method[0x14] = 1;
    method[0x15] = 2;
    put32(&mut method, 0x18, 0);
    put32(&mut method, 0x1C, 1);
    let mut field = vec![0; 24];
    put64(&mut field, 0, 0xF1E1D);
    put32(&mut field, 8, 0xF32);
    put32(&mut field, 12, 8);
    put32(&mut field, 16, NONE);
    put16(&mut field, 20, 4);

    let mut parts = Parts {
        code: three_functions(),
        pooled: pooled_data,
        types: vec![an_enum, a_class],
        imports: vec![(0x99, 4)],
        natives: vec![0xAAAA_BBBB_CCCC_DDDD, 2],
        entries: vec![(0x1111, 13)],
        methods: vec![method],
        fields: vec![field],
        main: Some(0),
        constants: vec![(84, 4), (88, 2)],
        sizes: [12, 600, 22],
        map: Some(90),
    };
    // Where a table lands does not depend on what its records hold: a first layout says where the class points.
    let laid = parts.build();
    put64(
        &mut parts.types[1],
        0x28,
        BASE + pointed(&laid, 0x60) as u64,
    );
    put64(
        &mut parts.types[1],
        0x38,
        BASE + pointed(&laid, 0x70) as u64,
    );

    let program = parts.read().expect("the program reads");
    assert_eq!(program.program, PROGRAM);
    assert_eq!(program.code, three_functions());
    let function = |at, end, params, locals, returns| FunctionRow {
        at,
        end,
        params,
        locals,
        returns,
    };
    assert_eq!(
        program.functions,
        [
            function(0, 11, 2, 5, 3),
            function(11, 13, 0, 4, 1),
            function(13, 23, 17, 304, 0),
        ]
    );
    assert_eq!(program.function(11), Some(&function(11, 13, 0, 4, 1)));
    assert_eq!(program.function(12), None);

    let an_enum = TypeRow {
        name: 0x10,
        byte_size: 2,
        flags: 0x4B,
        constants: vec![1, 2, 0xFFFE],
        ..TypeRow::default()
    };
    let a_class = TypeRow {
        name: 0x20,
        name32: 0xBEEF,
        byte_size: 312,
        flags: 2,
        base: Some(0x8000_0000),
        interfaces: vec![5],
        first_slots: vec![4],
        slots: vec![None, Some(11), None, None],
        vtable: vec![
            SlotRow::Builtin {
                ty: 1,
                method: 2,
                unboxes: false,
            },
            SlotRow::Function {
                import: 0,
                at: 11,
                unboxes: false,
            },
            SlotRow::Function {
                import: 1,
                at: 0x77,
                unboxes: true,
            },
            SlotRow::Empty,
        ],
        fields: vec![FieldRow {
            offset: 8,
            bytes: 4,
            ty: None,
            name: 0xF1E1D,
            name32: 0xF32,
        }],
        generic: Some((3, vec![7, 0x8000_0001])),
        methods: 0..1,
        ..TypeRow::default()
    };
    assert_eq!(program.types, [an_enum, a_class]);
    let method = MethodRow {
        name: 0x0C70_0001,
        at: Some(11),
        flags: 0x70,
        slot: Some(1),
        returns: 1,
        params: 2,
        result: Some(0),
        owner: 1,
    };
    assert_eq!(program.methods, [method]);

    let import = ImportRow {
        program: 0x99,
        types: 2..6,
    };
    assert_eq!(program.imports, [import]);
    assert_eq!(program.imported(0), Some(PROGRAM));
    assert_eq!(program.imported(1), Some(0x99));
    assert_eq!(program.imported(2), None);
    let info = program.info();
    assert_eq!((info.program, info.imports.len()), (PROGRAM, 1));
    assert_eq!((info.imports[0].first, info.imports[0].end), (2, 6));

    let stored: Vec<u64> = program.natives.iter().map(|native| native.hash).collect();
    assert_eq!(stored, [0xAAAA_BBBB_CCCC_DDDD, 2]);
    let entry = EntryRow {
        name: 0x1111,
        at: 13,
    };
    assert_eq!(program.entries, [entry]);
    assert_eq!((program.main, program.initialize), (Some(0), None));

    assert_eq!((program.globals.cells, program.statics.cells), (2, 3));
    let references: Vec<bool> = (0..4)
        .map(|cell| program.global_is_reference(cell))
        .collect();
    assert_eq!(references, [true, false, true, false]);
    assert!(
        !program.global_is_reference(4096),
        "a cell the map does not reach holds none"
    );

    assert_eq!(program.strings.len(), 106);
    assert_eq!(program.constants, [1, 2, 3, 4, 5, 6]);
    assert_eq!(program.constant(0), Some(&[1, 2, 3, 4][..]));
    assert_eq!(program.constant(1), Some(&[5, 6][..]));
    assert_eq!(program.constant(2), None);
    assert_eq!(program.string(8), None);
    assert!(program.handlers.is_empty());
}

#[test]
fn bytes_too_short_for_the_header_are_refused() {
    let short = ProgramError::Truncated {
        what: "the header",
        needed: 0x17F,
        len: 0x17F,
    };
    assert_eq!(Program::read(&[0; 0x17F]), Err(short));
    assert!(small().read().is_ok());
}

#[test]
fn another_format_and_another_build_are_refused() {
    let mut bytes = small().build();
    put32(&mut bytes, 0x18, 0x70);
    assert_eq!(refused(Program::read(&bytes)).0, 0x18);
    let mut bytes = small().build();
    put64(&mut bytes, 0x1C, 1);
    assert_eq!(refused(Program::read(&bytes)).0, 0x1C);
    let mut bytes = small().build();
    put32(&mut bytes, 0x164, 1);
    assert_eq!(refused(Program::read(&bytes)).0, 0x164);
    // A group of reflection records no program was seen to hold is said to be that, not a stray word.
    let mut bytes = small().build();
    put32(&mut bytes, 0xD4, 1);
    let (at, what) = refused(Program::read(&bytes));
    assert_eq!(at, 0xD4);
    assert!(what.contains("third group"), "{what}");
}

#[test]
fn a_count_the_file_cannot_hold_is_refused_before_anything_is_sized() {
    // Each count would size a table of gigabytes were it believed.
    let mut bytes = small().build();
    put32(&mut bytes, 0x28, u32::MAX);
    put32(&mut bytes, 0x2C, u32::MAX);
    let read = Program::read(&bytes);
    let what = "the types";
    assert!(
        matches!(read, Err(ProgramError::Truncated { what: w, .. }) if w == what),
        "{read:?}"
    );

    let mut bytes = small().build();
    put32(&mut bytes, 0xB8, u32::MAX);
    let read = Program::read(&bytes);
    let what = "the natives";
    assert!(
        matches!(read, Err(ProgramError::Truncated { what: w, .. }) if w == what),
        "{read:?}"
    );

    let mut parts = small();
    parts.pooled = words(&[16, NONE]);
    parts.types = vec![type_record(0, 1, |r| {
        put64(r, 0x28, pooled(8));
        put32(r, 0x30, u32::MAX);
        put16(r, 0x70, 4);
    })];
    let (at, what) = refused(parts.read());
    assert_eq!(at, pointed(&parts.build(), 0x30) + 0x28);
    assert!(what.contains("an enum's values"), "{what}");
}

#[test]
fn a_pointer_outside_the_file_is_refused() {
    let mut bytes = small().build();
    let past = BASE + bytes.len() as u64 + 1;
    put64(&mut bytes, 0xE8, past);
    assert_eq!(refused(Program::read(&bytes)).0, 0xE8);
    // Below the address the file is mapped at is outside it too.
    let mut bytes = small().build();
    put64(&mut bytes, 0xF0, 0x180);
    assert_eq!(refused(Program::read(&bytes)).0, 0xF0);
    // A table with records and no pointer, and one with a pointer and no record.
    let mut bytes = small().build();
    put32(&mut bytes, 0xC8, 1);
    assert_eq!(refused(Program::read(&bytes)).0, 0x100);
    let mut bytes = small().build();
    put64(&mut bytes, 0x60, BASE);
    assert_eq!(refused(Program::read(&bytes)).0, 0x60);
}

#[test]
fn imports_are_held_to_their_order_and_to_type_ids_that_follow_on() {
    let mut parts = small();
    parts.imports = vec![(0x10, 2), (0x20, 3)];
    let program = parts.read().expect("two imports in order read");
    assert_eq!(program.imports[1].types, 3..6);

    let mut bytes = parts.build();
    let second = pointed(&bytes, 0x148) + 48;
    put32(&mut bytes, second + 16, 4);
    assert_eq!(refused(Program::read(&bytes)).0, second + 16);

    parts.imports = vec![(0x20, 2), (0x10, 3)];
    let bytes = parts.build();
    assert_eq!(
        refused(Program::read(&bytes)).0,
        pointed(&bytes, 0x148) + 48 + 8
    );
}

#[test]
fn what_a_type_implements_is_its_base_list_past_its_base_type() {
    let list = words(&[9, 0, 10, 0, 11, 6, NONE]);
    let with = |base: u32| {
        let mut parts = small();
        parts.pooled = list.clone();
        parts.types = vec![type_record(0, 1, |r| {
            put32(r, 0x20, base);
            put64(r, 0x60, pooled(8));
            put16(r, 0x80, 3);
        })];
        parts
    };
    // An interface has no base type: all its list names is what it extends.
    let program = with(NONE).read().expect("no base type reads");
    let ty = &program.types[0];
    assert_eq!((ty.base, &ty.interfaces[..]), (None, &[9, 10, 11][..]));
    assert_eq!(ty.first_slots, [0, 0, 6]);
    // The ancestors come first and end on the base type.
    let program = with(10).read().expect("a base type in the list reads");
    let ty = &program.types[0];
    assert_eq!((ty.base, &ty.interfaces[..]), (Some(10), &[11][..]));
    assert_eq!(ty.first_slots, [6]);

    let parts = with(12);
    let (at, what) = refused(parts.read());
    assert_eq!(at, pointed(&parts.build(), 0x30) + 0x20);
    assert!(what.contains("base list"), "{what}");
}

#[test]
fn a_base_list_without_its_end_is_refused() {
    let mut parts = small();
    parts.pooled = words(&[9, 0, 10]);
    parts.types = vec![type_record(0, 1, |r| {
        put64(r, 0x60, pooled(8));
        put16(r, 0x80, 1);
    })];
    let (at, _) = refused(parts.read());
    assert_eq!(at, pointed(&parts.build(), 0x30) + 0x60);
}

#[test]
fn a_vtable_slot_is_held_to_the_code_and_to_the_imports() {
    let slot = |target: u32, owner: u32| {
        let mut parts = small();
        parts.pooled = words(&[NONE, NONE, target, owner]);
        parts.types = vec![type_record(0, 1, |r| {
            put64(r, 0x58, pooled(8));
            put16(r, 0x7E, 2);
        })];
        parts.read()
    };
    // `small` has two bytes of code and no import.
    let read = slot(0, 0x100).expect("a slot inside the code reads");
    let own = SlotRow::Function {
        import: 0,
        at: 0,
        unboxes: true,
    };
    assert_eq!(read.types[0].vtable, [SlotRow::Empty, own]);
    assert_eq!(read.types[0].slots, [None, Some(0)]);

    let second = BLOB + 8 + 8;
    assert_eq!(refused(slot(2, 0)).0, second, "outside the code");
    assert_eq!(refused(slot(0, 1)).0, second, "an import that is not there");
    assert_eq!(refused(slot(0, 0x200)).0, second, "a bit above bit 8");
    assert_eq!(refused(slot(0x1_0000, 0xFF)).0, second, "no builtin method");
    assert_eq!(refused(slot(0, NONE)).0, second, "empty with a target");
}

#[test]
fn a_vtable_slot_of_this_program_starts_a_function() {
    let mut parts = small();
    parts.code = [insn(RETURN, &[]), insn(RETURN_1, &[])].concat();
    parts.pooled = words(&[2, 0]);
    parts.types = vec![type_record(0, 1, |r| {
        put64(r, 0x58, pooled(8));
        put16(r, 0x7E, 1);
    })];
    let starts =
        |program: Program| -> Vec<u32> { program.functions.iter().map(|f| f.at).collect() };
    assert_eq!(starts(parts.read().expect("the slot reads")), [0, 2]);
    // Without the slot nothing names the second function.
    parts.types = vec![type_record(0, 1, |_| {})];
    assert_eq!(starts(parts.read().expect("no slot reads")), [0]);
}

#[test]
fn the_tables_of_the_types_cannot_state_more_than_the_pooled_data_holds() {
    // Four empty slots, 32 bytes of the 64 the pooled data has; a type states 4 more for the end of its base list.
    let vtable = |r: &mut [u8]| {
        put64(r, 0x58, pooled(8));
        put16(r, 0x7E, 4);
    };
    let mut parts = small();
    parts.pooled = vec![0xFF; 32];
    parts.types = vec![type_record(0, 1, vtable)];
    assert_eq!(
        parts.read().expect("one type reads").types[0].vtable.len(),
        4
    );

    parts.types.push(type_record(1, 2, vtable));
    let (at, what) = refused(parts.read());
    assert_eq!(at, pointed(&parts.build(), 0x30) + 136 + 0x58);
    assert!(what.contains("more bytes than"), "{what}");
}

#[test]
fn types_are_held_to_their_place_and_to_the_order_of_their_names() {
    let mut parts = small();
    parts.types = vec![type_record(0, 5, |_| {}), type_record(1, 6, |_| {})];
    assert_eq!(parts.read().expect("rising names read").types.len(), 2);

    parts.types = vec![type_record(0, 5, |_| {}), type_record(1, 5, |_| {})];
    let second = pointed(&parts.build(), 0x30) + 136;
    assert_eq!(refused(parts.read()).0, second + 0x10);

    parts.types = vec![type_record(0, 5, |_| {}), type_record(7, 6, |_| {})];
    assert_eq!(refused(parts.read()).0, second + 0x1C);
}

#[test]
fn an_enum_and_a_generic_instance_are_held_to_their_shape() {
    let with = |set: fn(&mut [u8])| {
        let mut parts = small();
        parts.pooled = words(&[16, 24, 3, 4, 0xAA, 0xBB]);
        parts.types = vec![type_record(0, 1, set)];
        (pointed(&parts.build(), 0x30), parts.read())
    };
    // The names of an enum's members are kept where the container has them.
    let (_, read) = with(|r| {
        put64(r, 0x28, pooled(8));
        put32(r, 0x30, 2);
        put16(r, 0x70, 4);
    });
    let program = read.expect("a named enum reads");
    let ty = &program.types[0];
    assert_eq!(ty.constants, [3, 4]);
    assert_eq!(ty.constant_names, [0xAA, 0xBB]);

    let (at, read) = with(|r| {
        put64(r, 0x28, pooled(8));
        put32(r, 0x30, 2);
        put16(r, 0x70, 3);
    });
    assert_eq!(refused(read).0, at + 0x70, "values three bytes wide");

    let (at, read) = with(|r| put32(r, 0x34, 3));
    assert_eq!(refused(read).0, at + 0x34, "a definition and no argument");
    let (at, read) = with(|r| put32(r, 0x68, 16));
    assert_eq!(refused(read).0, at + 0x68, "an offset and no argument");
    let (_, read) = with(|r| {
        put32(r, 0x34, 3);
        put32(r, 0x68, 16);
        put16(r, 0x74, 2);
    });
    let generic = read.expect("a definition with arguments reads").types[0]
        .generic
        .clone();
    assert_eq!(generic, Some((3, vec![3, 4])));
}

#[test]
fn constants_that_share_bytes_are_copied_once() {
    let mut parts = small();
    parts.pooled = vec![1, 2, 3, 4, 5, 6];
    parts.constants = vec![(10, 4), (8, 4)];
    let program = parts.read().expect("constants that overlap read");
    assert_eq!(program.constants, [1, 2, 3, 4, 5, 6]);
    assert_eq!(program.constant(0), Some(&[3, 4, 5, 6][..]));
    assert_eq!(program.constant(1), Some(&[1, 2, 3, 4][..]));

    // The pooled data of a built program is 64 bytes at the least.
    parts.constants = vec![(8, 4), (60, 5)];
    let bytes = parts.build();
    assert_eq!(refused(Program::read(&bytes)).0, pointed(&bytes, 0x98) + 8);
}

#[test]
fn the_map_of_the_globals_is_read_only_beside_an_order_of_imports() {
    let mut parts = small();
    parts.sizes = [16, 16, 0];
    parts.pooled = vec![0b10; 8];
    parts.map = Some(8);
    let program = parts.read().expect("a map inside the pooled data reads");
    assert_eq!(program.global_references, [0b10; 8]);
    assert!(program.global_is_reference(1) && !program.global_is_reference(0));

    parts.map = Some(60);
    assert_eq!(refused(parts.read()).0, 0x124);

    // A program others import states no order, and what its word says is not an offset of its pooled data.
    parts.map = None;
    let mut bytes = parts.build();
    put32(&mut bytes, 0x120, 0x10);
    put32(&mut bytes, 0x124, 0x00F0_0000);
    let program = Program::read(&bytes).expect("no order of imports, no map");
    assert!(program.global_references.is_empty());
}

#[test]
fn functions_start_at_entries_at_calls_of_the_program_and_at_named_offsets() {
    let code = [
        insn(PUSH_FUNCTION, &[0, 16, 0, 0, 0]),
        // A call into import 1: its offset, the middle of the instruction above, is no function of this code.
        insn(CALL, &[1, 2, 0, 0, 0]),
        insn(RETURN, &[]),
        insn(RETURN_1, &[]),
        insn(ENTER_FLAGGED, &[0x10, 1]),
        insn(CALL, &[0, 31, 0, 0, 0]),
        insn(RETURN, &[]),
        insn(RETURN_N, &[2]),
        insn(RETURN, &[]),
    ];
    let code = code.concat();
    let rows = functions::discover(&code, &[34]).expect("the code reads");
    let function = |at, end, params, returns| FunctionRow {
        at,
        end,
        params,
        locals: 4,
        returns,
    };
    assert_eq!(
        rows,
        [
            function(0, 16, 0, 0),
            function(16, 18, 0, 1),
            function(18, 31, 1, 0),
            function(31, 34, 0, 2),
            function(34, 36, 0, 0),
        ]
    );
    // Without the offset a table names, the last function is the tail of the one before it.
    let rows = functions::discover(&code, &[]).expect("the code reads");
    assert_eq!(rows.last(), Some(&function(31, 36, 0, 2)));
}

#[test]
fn a_function_named_where_no_instruction_starts_is_refused() {
    let code = [insn(ENTER, &[0x10]), insn(RETURN, &[])].concat();
    let at = |read: Result<Vec<FunctionRow>, ProgramError>| match read {
        Err(ProgramError::Code { at, .. }) => at,
        other => panic!("not refused by its code: {other:?}"),
    };
    assert_eq!(at(functions::discover(&code, &[1])), 1);
    assert_eq!(at(functions::discover(&code, &[5])), 5);
    assert!(functions::discover(&code, &[3]).is_ok());

    // A call of the program's own into the middle of an instruction.
    let code = [insn(CALL_SHORT, &[0, 1, 0]), insn(RETURN, &[])].concat();
    assert_eq!(at(functions::discover(&code, &[])), 1);
}

#[test]
fn bytes_that_are_no_instruction_are_refused() {
    let at = |code: &[u8]| match functions::discover(code, &[]) {
        Err(ProgramError::Code { at, .. }) => Some(at),
        _ => None,
    };
    let good = [insn(ENTER, &[0x10]), insn(RETURN, &[])].concat();
    assert_eq!(at(&good), None);
    assert_eq!(at(&[]), None);

    assert_eq!(at(&[good.clone(), vec![0xFF, 0xFF]].concat()), Some(5));
    // Number 90 is below 512 and no instruction has it.
    assert_eq!(at(&[good.clone(), insn(90, &[])].concat()), Some(5));
    assert_eq!(at(&good[..4]), Some(3), "half an opcode");
    assert_eq!(
        at(&insn(ENTER_W, &[1, 2, 3, 4])[..5]),
        Some(0),
        "an operand short"
    );
}

#[test]
fn a_function_only_another_program_names_is_split_off_the_one_before_it() {
    let mut parts = small();
    parts.code = [
        insn(ENTER, &[0x10]),
        insn(RETURN_1, &[]),
        insn(RETURN_N, &[2]),
        insn(NOP, &[]),
    ]
    .concat();
    let mut program = parts.read().expect("the program reads");
    let function = |at, end, params, returns| FunctionRow {
        at,
        end,
        params,
        locals: 4,
        returns,
    };
    // The first return is the function's own, whatever the code after it returns.
    assert_eq!(program.functions, [function(0, 10, 1, 1)]);

    assert_eq!(program.add_function(5), Ok(()));
    let split = [function(0, 5, 1, 1), function(5, 10, 0, 2)];
    assert_eq!(program.functions, split);
    assert_eq!(program.add_function(5), Ok(()));
    assert_eq!(program.add_function(0), Ok(()));
    assert_eq!(
        program.functions, split,
        "a start already known adds nothing"
    );

    for at in [4, 6, 10, 400] {
        let refused = program.add_function(at);
        assert!(
            matches!(refused, Err(ProgramError::Code { at: said, .. }) if said == at),
            "{at}: {refused:?}"
        );
    }
    assert_eq!(program.functions, split);
}

#[test]
fn the_functions_of_other_programs_a_program_calls_are_told_by_program_and_offset() {
    let mut parts = small();
    parts.imports = vec![(0x99, 4)];
    parts.code = [
        // A call into the program itself says nothing of another's code: its target is the return below.
        insn(CALL, &[0, 26, 0, 0, 0]),
        insn(CALL, &[1, 0x40, 0x02, 0, 0]),
        insn(CALL_SHORT, &[1, 0x10, 0]),
        insn(PUSH_FUNCTION, &[1, 0x80, 0, 0, 0x40]),
        insn(RETURN, &[]),
    ]
    .concat();
    let program = parts.read().expect("the program reads");
    assert_eq!(
        program.called_elsewhere(),
        [(0x99, 0x240), (0x99, 0x10), (0x99, 0x80)]
    );
}
