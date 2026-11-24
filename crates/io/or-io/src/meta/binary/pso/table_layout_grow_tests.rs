use super::*;
use crate::meta::binary::layout::ContainerLayout;
use crate::meta::binary::pso::table_form::tests::{body, member, open, pointer, Class, KIND};
use crate::meta::binary::{write_meta_binary, BinaryFormat, MetaLayout};
use crate::meta::instance::value::HashedString;

// Type hashes whose residues modulo 11 order the block table: 1, then 3, then 5.
const PAIR: u32 = 0x2000_0006;
const ROOT: u32 = 0x2000_0008;
const TEXT: u32 = 0x10;

fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn write(instance: &MetaInstance, layout: &MetaLayout) -> Result<Vec<u8>> {
    write_meta_binary(BinaryFormat::Pso, Some(layout), instance)
}

fn field(instance: &mut MetaInstance, hash: u32) -> &mut Value {
    instance.get_mut(&name(hash)).expect("the member")
}

/// The stored blocks of a document `layout` was read from, as `(type, bytes)`.
fn blocks(layout: &MetaLayout) -> Vec<(u32, Vec<u8>)> {
    let ContainerLayout::Pso(pso) = &layout.container else {
        panic!("a PSO layout")
    };
    let model = super::super::table_form::model(&sections_of(pso)).expect("a model");
    (model.data_blocks.iter())
        .map(|b| (b.data_type, b.data.clone()))
        .collect()
}

#[test]
fn a_second_block_of_a_type_follows_the_first_in_its_bucket() {
    for (ty, bucket) in [(5u32, 5), (16, 5)] {
        assert_eq!(ty % 11, bucket);
    }
    let blocks =
        |types: &[u32]| -> Vec<(u32, Vec<u8>)> { types.iter().map(|&t| (t, Vec::new())).collect() };
    // Head insertion would give 3, 2, 1, 0.
    assert_eq!(
        block_order(&blocks(&[5, 16, 5, 5])).expect("an order"),
        [1, 0, 2, 3]
    );
    // Eleven blocks of ten types: the split block is no new key, so the table keeps 11 buckets
    // and 29 goes to the head of 7's bucket. Counting blocks would grow it to 29 here.
    assert_eq!(
        block_order(&blocks(&[0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 29])).expect("an order"),
        [0, 1, 2, 3, 4, 5, 6, 7, 10, 8, 9]
    );
    // The eleventh type grows it: the two blocks of type 0 keep their order and 29 goes before them.
    assert_eq!(
        block_order(&blocks(&[0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 29])).expect("an order"),
        [11, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
}

#[test]
fn a_region_goes_to_the_first_block_of_its_type_with_room_and_one_past_the_cap_to_its_own() {
    const BIG: u32 = PAIR;
    const QUARTER: usize = BLOCK_CAP / 4;
    let arrays: [u32; 6] = std::array::from_fn(|i| 0x6868_0000 + i as u32);
    let mut members = vec![member(arrays[0], 0, COUNTED_ARRAY, 1, 0)];
    members.push(member(ARRAY_INFO_NAME_HASH, 0, 0x05, 0, BIG));
    for (i, &hash) in arrays.iter().enumerate().skip(1) {
        members.push(member(hash, 16 * i as u32, COUNTED_ARRAY, 1, 0));
    }
    let root = Class {
        name: ROOT,
        size: 0x60,
        members,
    };
    let big = Class {
        name: BIG,
        size: QUARTER as u32,
        members: vec![member(0x6868_0100, 0, 0x15, 0, 0)],
    };
    let bytes = body(&[root, big], &[(ROOT, vec![0; 0x60])], 1);
    let (mut instance, layout) = open(&bytes).expect("it reads");
    let element = |i: u64| {
        let mut e = MetaInstance::new(name(BIG));
        e.fields.push((name(0x6868_0100), Value::UInt(i)));
        Value::Struct(Box::new(e))
    };
    let counts = [3u64, 2, 1, 1, 5, 1];
    let mut next = 0;
    for (&hash, &count) in arrays.iter().zip(&counts) {
        *field(&mut instance, hash) = Value::Array((next..next + count).map(&element).collect());
        next += count;
    }
    let written = write(&instance, &layout).expect("it writes");
    let (again, layout) = open(&written).expect("it reads back");
    assert_eq!(again, instance);

    // 3, then 2 that do not fit beside them, then 1 that fills the first block to the cap, 1
    // beside the 2, 5 past the cap in a block of their own, and 1 in the second block again.
    let blocks = blocks(&layout);
    let sizes: Vec<(u32, usize)> = blocks.iter().map(|(t, d)| (*t, d.len())).collect();
    assert_eq!(
        sizes,
        [
            (BIG, 4 * QUARTER),
            (BIG, 4 * QUARTER),
            (BIG, 5 * QUARTER),
            (ROOT, 0x60)
        ]
    );
    let root = &blocks[3].1;
    let target = |i: usize| u64::from_le_bytes(root[16 * i + 8..16 * i + 16].try_into().unwrap());
    let quarters = |n: usize| (n * QUARTER) as u64;
    let expected = [
        pointer(1, 0),
        pointer(2, 0),
        pointer(1, quarters(3)),
        pointer(2, quarters(2)),
        pointer(3, 0),
        pointer(2, quarters(3)),
    ];
    assert_eq!((0..6).map(target).collect::<Vec<_>>(), expected);
}

/// A slot map of two pairs whose values are text, at slots 5 and 10, the text in slot order.
fn text_map() -> Vec<u8> {
    for (ty, bucket) in [(PAIR, 1), (ROOT, 3), (TEXT, 5)] {
        assert_eq!(ty % 11, bucket);
    }
    let anon = |code: u8| member(ARRAY_INFO_NAME_HASH, 0, code, 0, 0);
    let root = Class {
        name: ROOT,
        size: 0x10,
        members: vec![
            member(0x6969_0000, 0, SLOT_MAP, 0, 2 << 16 | 1),
            anon(0x4A),
            anon(0x42),
        ],
    };
    let pair = Class {
        name: PAIR,
        size: 16,
        members: vec![
            member(0x6969_0100, 0, 0x4A, 0, 0),
            member(0x6969_0101, 8, 0x42, 0, 0),
        ],
    };
    let mut data = words(&[32, 2]);
    data.extend(pointer(1, 0).to_le_bytes());
    let mut slots = vec![0u8; 32 * 16];
    for (slot, key, text_at) in [(5, 0x4444_0005u32, 0u64), (10, 0x5555_000A, 6)] {
        slots[16 * slot..16 * slot + 4].copy_from_slice(&key.to_le_bytes());
        slots[16 * slot + 8..16 * slot + 16].copy_from_slice(&pointer(3, text_at).to_le_bytes());
    }
    body(
        &[root, pair],
        &[
            (PAIR, slots),
            (ROOT, data),
            (TEXT, b"first\0second\0".to_vec()),
        ],
        2,
    )
}

#[test]
fn a_rebuilt_table_moves_its_slots_and_leaves_its_payloads_in_their_stored_order() {
    let bytes = text_map();
    let (mut instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it writes"), bytes);

    // The first key now goes to slot 20, after the second's 10.
    let moved = Value::Hash(HashedString::from_hash(0x4444_0014));
    let Value::Map(pairs) = field(&mut instance, 0x6969_0000) else {
        panic!("a map")
    };
    pairs[0].0 = moved.clone();
    let written = write(&instance, &layout).expect("it writes");
    let (again, layout) = open(&written).expect("it reads back");
    let Some(Value::Map(placed)) = again.get(&name(0x6969_0000)) else {
        panic!("a map")
    };
    assert_eq!(placed[0].1, Value::String("second".into()));
    assert_eq!(placed[1], (moved, Value::String("first".into())));
    // Visited in slot order, the text would read "second" first.
    let text = (blocks(&layout).into_iter())
        .find(|(ty, _)| *ty == TEXT)
        .expect("the text");
    assert_eq!(text.1, b"first\0second\0");
    // Read back, its slot order is no longer its payloads' order, and it still writes back.
    assert_eq!(write(&again, &layout).expect("it writes"), written);
}

#[test]
fn a_table_keyed_by_64_bit_integers_keeps_its_slots_and_refuses_to_grow() {
    let anon = |code: u8| member(ARRAY_INFO_NAME_HASH, 0, code, 0, 0);
    let root = Class {
        name: ROOT,
        size: 0x10,
        members: vec![
            member(0x6A6A_0000, 0, SLOT_MAP, 0, 2 << 16 | 1),
            anon(0x17),
            anon(0x15),
        ],
    };
    let pair = Class {
        name: PAIR,
        size: 16,
        members: vec![
            member(0x6A6A_0100, 0, 0x17, 0, 0),
            member(0x6A6A_0101, 8, 0x15, 0, 0),
        ],
    };
    let mut data = words(&[32, 1]);
    data.extend(pointer(1, 0).to_le_bytes());
    let mut slots = vec![0u8; 32 * 16];
    slots[48..56].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
    slots[56..60].copy_from_slice(&41u32.to_le_bytes());
    let bytes = body(&[root, pair], &[(PAIR, slots), (ROOT, data)], 2);
    let (mut instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it writes"), bytes);

    let Value::Map(pairs) = field(&mut instance, 0x6A6A_0000) else {
        panic!("a map")
    };
    pairs[0].1 = Value::UInt(42);
    let written = write(&instance, &layout).expect("a value changes in its slot");
    let (again, _) = open(&written).expect("it reads back");
    assert_eq!(again, instance);
    // The one value changed in its slot; the loader's CHKS is recomputed too, so the frame's
    // checksum word differs from the original. Nothing else moved.
    assert_eq!(written.len(), bytes.len());
    let chk = written.len() - 8; // the 0x14 CHKS frame's checksum word at [chk, chk + 4)
    let body_differ = bytes
        .iter()
        .zip(&written)
        .enumerate()
        .filter(|(i, (a, b))| a != b && !(chk..chk + 4).contains(i))
        .count();
    assert_eq!(
        body_differ, 1,
        "only the edited value moves outside the checksum"
    );
    let stored_sum = u32::from_be_bytes(written[chk..chk + 4].try_into().unwrap());
    assert_eq!(super::super::chks::checksum(&written), Some(stored_sum));
    assert_ne!(
        &bytes[chk..chk + 4],
        &written[chk..chk + 4],
        "the CHKS is recomputed for the edit"
    );

    let Value::Map(pairs) = field(&mut instance, 0x6A6A_0000) else {
        panic!("a map")
    };
    pairs.push((Value::UInt(7), Value::UInt(8)));
    let err = write(&instance, &layout).expect_err("refused").to_string();
    assert!(err.contains("its keys are 64-bit integers"), "{err}");
}

#[test]
fn a_named_bitset_of_more_bits_than_a_word_holds_as_many_words_as_they_take() {
    let root = Class {
        name: ROOT,
        size: 0x20,
        members: vec![
            member(0x6B6B_0000, 0x00, 0x65, 87, KIND),
            member(0x6B6B_0001, 0x0C, 0x15, 0, 0),
            member(0x6B6B_0002, 0x10, 0x63, 12, KIND),
            member(0x6B6B_0003, 0x12, 0x64, 0, KIND),
            member(0x6B6B_0004, 0x14, 0x50, 5, 2),
            // An element row's bit count does not size it: these are 4 bytes each.
            member(ARRAY_INFO_NAME_HASH, 0, 0x65, 256, KIND),
        ],
    };
    let wide: u128 = 1 | 1 << 40 | 1 << 86;
    let mut data = wide.to_le_bytes()[..12].to_vec();
    data.extend(0xDEAD_BEEFu32.to_le_bytes());
    data.extend((1u16 | 1 << 11).to_le_bytes());
    data.extend(0x8001u16.to_le_bytes());
    data.extend(words(&[3, 0x8000_0000]));
    data.resize(0x20, 0);
    let bytes = body(&[root], &[(ROOT, data)], 1);
    let (mut instance, layout) = open(&bytes).expect("it reads");
    let bits = |v: Option<&Value>| match v {
        Some(Value::Bitset { bits, .. }) => *bits,
        other => panic!("a bitset, not {other:?}"),
    };
    assert_eq!(bits(instance.get(&name(0x6B6B_0000))), wide);
    assert_eq!(
        instance.get(&name(0x6B6B_0001)),
        Some(&Value::UInt(0xDEAD_BEEF))
    );
    assert_eq!(bits(instance.get(&name(0x6B6B_0002))), 1 | 1 << 11);
    assert_eq!(bits(instance.get(&name(0x6B6B_0003))), 0x8001);
    let Some(Value::Array(items)) = instance.get(&name(0x6B6B_0004)) else {
        panic!("an array")
    };
    let items: Vec<u128> = items.iter().map(|v| bits(Some(v))).collect();
    assert_eq!(items, [3, 0x8000_0000]);
    assert_eq!(write(&instance, &layout).expect("it writes"), bytes);

    let Value::Bitset { bits: set, .. } = field(&mut instance, 0x6B6B_0000) else {
        panic!("a bitset")
    };
    *set = 1 << 95;
    let written = write(&instance, &layout).expect("bit 95 is in its 12 bytes");
    let (again, _) = open(&written).expect("it reads back");
    assert_eq!(again, instance);
    let Value::Bitset { bits: set, .. } = field(&mut instance, 0x6B6B_0000) else {
        panic!("a bitset")
    };
    *set = 1 << 96;
    let err = write(&instance, &layout).expect_err("refused").to_string();
    assert!(err.contains("its 97 bits do not fit 12 bytes"), "{err}");
}

#[test]
fn an_empty_array_the_document_kept_behind_a_pointer_keeps_it() {
    const U32: u32 = 0x15;
    assert_eq!((ROOT % 11, U32 % 11), (3, 10));
    let root = Class {
        name: ROOT,
        size: 0x20,
        members: vec![
            member(0x6C6C_0000, 0x00, 0x52, 1, 0),
            member(ARRAY_INFO_NAME_HASH, 0, 0x15, 0, 0),
            member(0x6C6C_0002, 0x10, COUNTED_ARRAY, 1, 0),
        ],
    };
    // An empty array pointing at the end of block 2, which a one-element array fills.
    let mut data = pointer(2, 4).to_le_bytes().to_vec();
    data.extend([0; 8]);
    data.extend(words(&[1, 1]));
    data.extend(pointer(2, 0).to_le_bytes());
    let bytes = body(
        &[root],
        &[(ROOT, data), (U32, 0x1122_3344u32.to_le_bytes().to_vec())],
        1,
    );
    let (instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(
        instance.get(&name(0x6C6C_0000)),
        Some(&Value::Array(Vec::new()))
    );
    assert_eq!(write(&instance, &layout).expect("it writes"), bytes);
}

#[test]
fn payloads_keep_their_stored_order_and_a_new_one_goes_last() {
    // Entry 1's region is stored before entry 0's; entry 2 reaches none.
    assert_eq!(
        payload_order(&[vec![(0, 8)], vec![(0, 0)], Vec::new()]),
        [1, 0, 2]
    );
    // Two blocks that disagree leave the list order.
    assert_eq!(
        payload_order(&[vec![(0, 0), (1, 8)], vec![(0, 8), (1, 0)]]),
        [0, 1]
    );
}
