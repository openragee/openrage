use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::{parse_meta_binary_full, write_meta_binary, BinaryFormat, MetaLayout};
use crate::meta::diagnostics::Diagnostics;
use crate::meta::error::Result;
use crate::meta::instance::value::{MetaInstance, Value};
use crate::meta::schema::SchemaLibrary;

use super::super::reader::{GTA6_COUNT_FIRST_ARRAY, GTA6_GROUP_MAP, GTA6_STRING_HASH};
use super::super::schema::ARRAY_INFO_NAME_HASH;

const ROOT: u32 = 0xD0D0_0001;
const PAIR: u32 = 0xD0D0_0002;
const MAP: u32 = 0xDEDE_0000;
const KEY: u32 = 0xDEDE_0010;
const VALUE: u32 = 0xDEDE_0011;
/// The key the one full pair holds, a string hash.
const HELD: u32 = 0xC318_70ED;
const ELEMENTS: [u32; 2] = [7, 9];
/// A pair: the key at 0, the array header at 8.
const PAIR_SIZE: usize = 24;
/// Where the group starts, the array header is and the elements start in `PSIN`.
const GROUP_AT: usize = 0x20;
const HEADER_AT: usize = GROUP_AT + 16 + 8;
const ELEMENTS_AT: usize = GROUP_AT + 16 + 16 * PAIR_SIZE;
/// The second and fourth words of the array header, carried as stored.
const CARRIED: [u32; 2] = [0x0200_0000, 0x1A];

fn section(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(ident);
    out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(content);
}

fn entry(out: &mut Vec<u8>, hash: u32, code: u8, subtype: u8, offset: u16, reference: u32) {
    out.extend_from_slice(&hash.to_be_bytes());
    out.extend_from_slice(&[code, subtype]);
    out.extend_from_slice(&offset.to_be_bytes());
    out.extend_from_slice(&reference.to_be_bytes());
}

/// A GTA VI P-form document whose root holds a group map of one group, one pair of it full: a
/// string hash and a subtype-10 array of two u32s; `patch` edits `PSIN` (absolute offsets) first.
fn document(patch: impl FnOnce(&mut [u8])) -> Vec<u8> {
    let end = ELEMENTS_AT + 4 * ELEMENTS.len();
    let mut psin = vec![0u8; end];
    let mut put = |at: usize, bytes: &[u8]| psin[at..at + bytes.len()].copy_from_slice(bytes);
    put(0x08, &[0x70; 8]);
    // One group, one pair, the pointer to block 2 at 0.
    put(0x10, &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0]);
    put(GROUP_AT, &[0x80; 16]);
    // Slot 0's control byte, the key's low 7 bits, is byte 0 ^ 3 of the big-endian words.
    put(GROUP_AT + 3, &[(HELD & 0x7F) as u8]);
    put(GROUP_AT + 16, &HELD.to_be_bytes());
    // The elements are block 3 at offset 0.
    let header = [ELEMENTS.len() as u32, CARRIED[0], 3, CARRIED[1]];
    for (k, word) in header.iter().enumerate() {
        put(HEADER_AT + 4 * k, &word.to_be_bytes());
    }
    for (i, element) in ELEMENTS.iter().enumerate() {
        put(ELEMENTS_AT + 4 * i, &element.to_be_bytes());
    }
    patch(&mut psin);

    let mut pmap = Vec::new();
    pmap.extend_from_slice(&1i32.to_be_bytes());
    pmap.extend_from_slice(&3i16.to_be_bytes());
    pmap.extend_from_slice(&0i16.to_be_bytes());
    let blocks = [
        (ROOT, 0x10, 0x10),
        (PAIR, GROUP_AT, 16 + 16 * PAIR_SIZE),
        (0x06, ELEMENTS_AT, 4 * ELEMENTS.len()),
    ];
    for (hash, offset, length) in blocks {
        for word in [hash, offset as u32, 0, length as u32] {
            pmap.extend_from_slice(&word.to_be_bytes());
        }
    }

    // The root describes the map's key, element and value; the map names its value as member 2
    // of the root, and the value its elements as member 1 of the root.
    let mut items = Vec::new();
    items.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 0x10, 0, 0, 0, 0]);
    entry(
        &mut items,
        ARRAY_INFO_NAME_HASH,
        0x0B,
        GTA6_STRING_HASH,
        0,
        0,
    );
    entry(&mut items, ARRAY_INFO_NAME_HASH, 0x06, 0, 0, 0);
    entry(
        &mut items,
        ARRAY_INFO_NAME_HASH,
        0x0D,
        GTA6_COUNT_FIRST_ARRAY,
        0,
        1,
    );
    entry(&mut items, MAP, 0x10, GTA6_GROUP_MAP, 0, 2);
    let pair_at = items.len();
    items.extend_from_slice(&[0, 0, 0, 2]);
    items.extend_from_slice(&(PAIR_SIZE as u32).to_be_bytes());
    items.extend_from_slice(&[0; 4]);
    entry(&mut items, KEY, 0x0B, GTA6_STRING_HASH, 0, 0);
    // Member 1 of the pair is the array itself, of 16-byte headers, not its element.
    entry(&mut items, VALUE, 0x0D, GTA6_COUNT_FIRST_ARRAY, 8, 1);
    let start = 8 + 4 + 2 * 8;
    let mut psch = 2u32.to_be_bytes().to_vec();
    for (hash, offset) in [(ROOT, start), (PAIR, start + pair_at)] {
        psch.extend_from_slice(&hash.to_be_bytes());
        psch.extend_from_slice(&(offset as u32).to_be_bytes());
    }
    psch.extend_from_slice(&items);

    let mut out = Vec::new();
    section(&mut out, b"PSIN", &psin[8..]);
    section(&mut out, b"PMAP", &pmap);
    section(&mut out, b"PSCH", &psch);
    let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
    chks.extend_from_slice(&[0; 4]);
    chks.extend_from_slice(b"zppp");
    section(&mut out, b"CHKS", &chks);
    crate::meta::binary::pso::chks::write_checksum(&mut out);
    out
}

fn open(bytes: &[u8]) -> Result<(MetaInstance, MetaLayout)> {
    parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut Diagnostics::strict())
}

fn write(instance: &MetaInstance, layout: &MetaLayout) -> Result<Vec<u8>> {
    write_meta_binary(BinaryFormat::Pso, Some(layout), instance)
}

fn named(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn the_pair(instance: &mut MetaInstance) -> &mut (Value, Value) {
    let Some(Value::Map(pairs)) = instance.get_mut(&named(MAP)) else {
        panic!("a map")
    };
    assert_eq!(pairs.len(), 1);
    &mut pairs[0]
}

#[test]
fn a_gta6_group_map_reads_its_full_pair_and_the_array_its_header_names() {
    let (mut instance, _) = open(&document(|_| {})).expect("it reads");
    let (key, value) = the_pair(&mut instance);
    let Value::Hash(key) = key else {
        panic!("a hash")
    };
    assert_eq!(key.hash, HELD);
    let expected: Vec<Value> = ELEMENTS.iter().map(|&e| Value::UInt(e.into())).collect();
    assert_eq!(value, &Value::Array(expected));
}

#[test]
fn a_gta6_group_map_saves_in_place_and_keeps_its_array_header_words() {
    let bytes = document(|_| {});
    let (mut instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);

    let Value::Array(items) = &mut the_pair(&mut instance).1 else {
        panic!("an array")
    };
    items[1] = Value::UInt(11);
    let saved = write(&instance, &layout).expect("it saves");
    let expected = document(|psin| {
        psin[ELEMENTS_AT + 4..ELEMENTS_AT + 8].copy_from_slice(&11u32.to_be_bytes())
    });
    assert_eq!(saved, expected);
    let (again, _) = open(&saved).expect("it reads back");
    assert_eq!(again, instance);
}

#[test]
fn a_gta6_group_map_refuses_a_new_key_or_a_changed_count_by_name() {
    let (instance, layout) = open(&document(|_| {})).expect("it reads");
    let mut grown = instance.clone();
    if let Value::Array(items) = &mut the_pair(&mut grown).1 {
        items.push(Value::UInt(13));
    }
    let err = write(&grown, &layout).expect_err("refused").to_string();
    assert!(err.contains("for another count is not known"), "{err}");

    let mut rekeyed = instance.clone();
    the_pair(&mut rekeyed).0 = Value::UInt(u64::from(HELD ^ 1));
    let err = write(&rekeyed, &layout).expect_err("refused").to_string();
    assert!(err.contains("a key other than the one stored"), "{err}");
}

#[test]
fn a_gta6_group_map_outside_the_measured_layout_is_refused_by_name() {
    type Patch = fn(&mut [u8]);
    let cases: [(Patch, &str); 5] = [
        (
            |p| p[0x17] = 2,
            "holds 1 full pairs, where its header states 2",
        ),
        (|p| p[0x13] = 3, "states 3 groups"),
        (|p| p[GROUP_AT + 3] = 0x81, "control byte 0x81"),
        (|p| p[HEADER_AT + 10] = 0x10, "sets bits 12 to 19"),
        (|p| p[HEADER_AT + 3] = 0xFF, "runs past PSIN"),
    ];
    for (patch, want) in cases {
        let err = open(&document(patch)).expect_err(want).to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
}
