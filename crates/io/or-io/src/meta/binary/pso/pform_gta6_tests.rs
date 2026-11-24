use super::*;
use crate::meta::binary::{parse_meta_binary_full, write_meta_binary, BinaryFormat, MetaLayout};

const ROOT: u32 = 0xC0C0_0001;
const PAIR: u32 = 0xC0C0_0002;
const MAP: u32 = 0xCDCD_0000;
const TEXT: u32 = 0xCDCD_0001;
const HASH: u32 = 0xCDCD_0002;
const WIDE: u32 = 0xCDCD_0003;
const BYTES: u32 = 0xCDCD_0004;
const KEY: u32 = 0xCDCD_0010;
const VALUE: u32 = 0xCDCD_0011;
const PAIRS: [(u32, u32); 2] = [(7, 70), (9, 90)];
const HASH_32: u32 = 0xA1B2_C3D4;
const HASH_64: u64 = 0x0102_0304_0506_0708;
const ROW: [u8; 16] = [
    0x80, 0, 0, 1, 0x80, 0, 0, 2, 0x35, 0x86, 0x37, 0xBD, 0x3F, 0x80, 0, 3,
];
/// Where the root's 0x50 bytes start in the document, after `PSIN`'s frame and 8 pad bytes.
const AT: usize = 0x10;

fn section(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(ident);
    out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(content);
}

fn entry(out: &mut Vec<u8>, hash: u32, code: u8, subtype: u8, offset: u16) {
    out.extend_from_slice(&hash.to_be_bytes());
    out.extend_from_slice(&[code, subtype]);
    out.extend_from_slice(&offset.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
}

/// A GTA VI P-form document whose root holds a map of `map_subtype`, `text` kept in place, a 32-
/// and a 64-bit hash and 16 bytes of type 0x24; the map's pairs are a block of their own.
fn document(map_subtype: u8, text: &[u8]) -> Vec<u8> {
    document_with(map_subtype, text, false)
}

/// [`document`], or with `hashes` its 16 bytes at 0x40 an inline array of two 32-bit hashes in
/// place and the count after them (meta-grow Q12 and Q14).
fn document_with(map_subtype: u8, text: &[u8], hashes: bool) -> Vec<u8> {
    let mut psin = vec![0u8; 0x70 - 8];
    let mut put =
        |at: usize, bytes: &[u8]| psin[at - 8..at - 8 + bytes.len()].copy_from_slice(bytes);
    put(0x08, &[0x70; 8]);
    // `{u32 0x01000000, u32 0, pointer to block 2 at 0, u16 2, u16 2, u32 0}`.
    put(
        AT,
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 2, 0, 2],
    );
    put(AT + 0x18, text);
    put(AT + 0x18 + 15, &[0x80 | text.len() as u8]);
    put(AT + 0x28, &HASH_32.to_be_bytes());
    put(AT + 0x30, &HASH_64.to_be_bytes());
    put(AT + 0x40, &ROW);
    for (i, (key, value)) in PAIRS.iter().enumerate() {
        put(0x60 + 8 * i, &key.to_be_bytes());
        put(0x64 + 8 * i, &value.to_be_bytes());
    }

    let mut pmap = Vec::new();
    pmap.extend_from_slice(&1i32.to_be_bytes());
    pmap.extend_from_slice(&2i16.to_be_bytes());
    pmap.extend_from_slice(&0i16.to_be_bytes());
    for (hash, offset, length) in [(ROOT, 0x10u32, 0x50u32), (PAIR, 0x60, 0x10)] {
        for word in [hash, offset, 0, length] {
            pmap.extend_from_slice(&word.to_be_bytes());
        }
    }

    let mut items = Vec::new();
    let members = if hashes { 6 } else { 5 };
    items.extend_from_slice(&[0, 0, 0, members, 0, 0, 0, 0x50, 0, 0, 0, 0]);
    entry(&mut items, MAP, 0x10, map_subtype, 0);
    entry(&mut items, TEXT, 0x0B, string_subtype::CHAR_ARRAY, 0x18);
    entry(&mut items, HASH, 0x0B, GTA6_STRING_HASH, 0x28);
    entry(&mut items, WIDE, 0x0B, GTA6_STRING_HASH64, 0x30);
    if hashes {
        // Two elements in place, described by member 5.
        entry(&mut items, BYTES, 0x0D, 1, 0x40);
        let key = items.len() - 4;
        items[key..].copy_from_slice(&((2u32 << 16) | 5).to_be_bytes());
        entry(&mut items, 0x100, 0x0B, GTA6_STRING_HASH, 0);
    } else {
        entry(&mut items, BYTES, GTA6_BYTES_16, 0, 0x40);
    }
    let pair_at = items.len();
    items.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 8, 0, 0, 0, 0]);
    entry(&mut items, KEY, 0x06, 0, 0);
    entry(&mut items, VALUE, 0x06, 0, 4);
    let start = 8 + 4 + 2 * 8;
    let mut psch = 2u32.to_be_bytes().to_vec();
    for (hash, offset) in [(ROOT, start), (PAIR, start + pair_at)] {
        psch.extend_from_slice(&hash.to_be_bytes());
        psch.extend_from_slice(&(offset as u32).to_be_bytes());
    }
    psch.extend_from_slice(&items);

    let mut out = Vec::new();
    section(&mut out, b"PSIN", &psin);
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

fn pairs(pairs: &[(u32, u32)]) -> Value {
    let pair = |&(k, v): &(u32, u32)| (Value::UInt(k.into()), Value::UInt(v.into()));
    Value::Map(pairs.iter().map(pair).collect())
}

#[test]
fn a_gta6_p_form_reads_its_pair_maps_strings_in_place_hashes_and_16_byte_rows() {
    let (instance, _) = open(&document(GTA6_PAIR_MAP, b"hi there")).expect("it reads");
    assert_eq!(instance.get(&named(MAP)), Some(&pairs(&PAIRS)));
    assert_eq!(
        instance.get(&named(TEXT)),
        Some(&Value::String("hi there".into()))
    );
    let Some(Value::Hash(hash)) = instance.get(&named(HASH)) else {
        panic!("a hash")
    };
    assert_eq!(hash.hash, HASH_32);
    assert_eq!(instance.get(&named(WIDE)), Some(&Value::UInt(HASH_64)));
    assert_eq!(
        instance.get(&named(BYTES)),
        Some(&Value::ByteArray(ROW.to_vec()))
    );

    // A text of 14 bytes is the longest kept in place.
    let (instance, _) = open(&document(GTA6_PAIR_MAP, b"fourteen bytes")).expect("it reads");
    assert_eq!(
        instance.get(&named(TEXT)),
        Some(&Value::String("fourteen bytes".into()))
    );
}

#[test]
fn a_gta6_p_form_saves_its_pairs_and_strings_in_place_and_refuses_what_would_move() {
    let bytes = document(GTA6_PAIR_MAP, b"hi there");
    let (mut instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);

    *instance.get_mut(&named(MAP)).expect("the map") = pairs(&[(8, 80), (9, 91)]);
    *instance.get_mut(&named(WIDE)).expect("the hash") = Value::UInt(!HASH_64);
    let written = write(&instance, &layout).expect("it saves");
    assert_ne!(written, bytes);
    let (again, layout) = open(&written).expect("it reads back");
    assert_eq!(again, instance);
    assert_eq!(write(&again, &layout).expect("it saves"), written);

    // Its string is kept in place with no copy in a string block, so the rules that lay a tree out
    // anew do not give this document back, and what outgrew its allocation is refused.
    let mut grown = instance.clone();
    *grown.get_mut(&named(MAP)).expect("the map") = pairs(&[(8, 80), (9, 91), (10, 100)]);
    let mut retold = instance.clone();
    *retold.get_mut(&named(TEXT)).expect("the text") = Value::String("hi".into());
    for edited in [grown, retold] {
        let err = write(&edited, &layout).expect_err("refused").to_string();
        assert!(
            err.contains("measured layout rules do not give back"),
            "{err}"
        );
    }
}

#[test]
fn a_gta6_inline_array_of_hashes_in_place_reads_its_elements_and_saves_unchanged() {
    let bytes = document_with(GTA6_PAIR_MAP, b"hi there", true);
    let (instance, layout) = open(&bytes).expect("it reads");
    let Some(Value::Array(items)) = instance.get(&named(BYTES)) else {
        panic!("an array")
    };
    assert_eq!(items.len(), 2);
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);
}

#[test]
fn an_empty_gta6_group_map_reads_and_saves_and_one_with_pairs_is_refused_by_name() {
    let mut bytes = document(GTA6_GROUP_MAP, b"hi there");
    let err = open(&bytes).expect_err("refused").to_string();
    assert!(err.contains("GTA VI group map"), "{err}");

    bytes[AT..AT + 0x18].fill(0);
    // Clearing the text changed the body, so its CHKS no longer matches; recompute it, as a save
    // does, before asserting the save is byte-identical.
    crate::meta::binary::pso::chks::write_checksum(&mut bytes);
    let (instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(instance.get(&named(MAP)), Some(&Value::Map(Vec::new())));
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);
}

#[test]
fn a_gta6_p_form_outside_the_measured_layouts_is_refused_by_name() {
    type Patch = fn(&mut Vec<u8>);
    let cases: [(u8, Patch, &str); 5] = [
        (
            2,
            |_| {},
            "GTA VI map subtype 2, whose layout is not measured",
        ),
        (GTA6_PAIR_MAP, |b| b[AT] = 2, "around its pointer"),
        (
            GTA6_PAIR_MAP,
            |b| b[AT + 0x18 + 10] = 1,
            "in place states 8 bytes",
        ),
        (
            GTA6_PAIR_MAP,
            |b| b[AT + 0x18 + 15] = 0x8F,
            "in place states 15 bytes",
        ),
        // 0xFFFF pairs of 8 bytes run past the document.
        (GTA6_PAIR_MAP, |b| b[AT + 16] = 0xFF, "run past PSIN"),
    ];
    for (subtype, patch, want) in cases {
        let mut bytes = document(subtype, b"hi there");
        patch(&mut bytes);
        let err = open(&bytes).expect_err(want).to_string();
        assert!(err.contains(want), "{want}: {err}");
    }
}
