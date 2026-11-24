use super::*;
use crate::meta::binary::layout::ContainerLayout;
use crate::meta::binary::pso::table_form::tests::{
    body, hash_maps_and_wide_values, leftover_codes, measured_codes, member, open, pointer, sample,
    Class, KIND,
};
use crate::meta::binary::{write_meta_binary, BinaryFormat, MetaLayout};
use crate::meta::instance::value::HashedString;

// Type hashes whose residues modulo 11 order the block table: the pair class's bucket 1, then
// bucket 3 where the element block, met after the root, comes before it, then text's bucket 5.
const ROOT: u32 = 0x2000_0008;
const ELEMENT: u32 = 0x2000_0013;
const PAIR: u32 = 0x2000_0006;
const TEXT: u32 = 0x10;
const KEYS: [u32; 2] = [0x4444_0005, 0x5555_0025];
const LONG: &str = "a longer string here";
const CHECKSUM: u32 = 0xDEAD_BEEF;

fn measured_root() -> u32 {
    crate::meta::binary::pso::table_form::tests::ROOT
}

fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A body laid out as the measured rules lay one out, with a checksum word.
fn conformant() -> Vec<u8> {
    conformant_in([1, 2, 3, 4])
}

/// [`conformant`] with the pair, element, root and text blocks at the table ids `ids`.
fn conformant_in(ids: [u64; 4]) -> Vec<u8> {
    let [pairs_id, elements_id, root_id, text_id] = ids;
    let anon = |code: u8, u12: u32| member(ARRAY_INFO_NAME_HASH, 0, code, 0, u12);
    let root = Class {
        name: ROOT,
        size: 0x60,
        members: vec![
            member(0x6666_0000, 0x00, COUNTED_ARRAY, 1, 0),
            anon(0x05, ELEMENT),
            member(0x6666_0002, 0x10, 0x07, 0, 0),
            member(0x6666_0003, 0x18, 0x44, 0, 0),
            member(0x6666_0004, 0x28, SLOT_MAP, 0, 6 << 16 | 5),
            anon(0x4A, 0),
            anon(0x15, 0),
            member(0x6666_0007, 0x38, 0x33, 0, 0),
            member(0x6666_0008, 0x48, 0x4D, 0, 0),
            member(0x6666_0009, 0x4C, 0x61, 0, KIND),
            member(0x6666_000A, 0x50, 0x42, 0, 0),
            member(0x6666_000B, 0x58, 0x15, 0, 0),
        ],
    };
    let element = Class {
        name: ELEMENT,
        size: 8,
        members: vec![
            member(0x6666_0100, 0, 0x15, 0, 0),
            member(0x6666_0101, 4, 0x21, 0, 0),
        ],
    };
    let pair = Class {
        name: PAIR,
        size: 8,
        members: vec![
            member(0x6666_0200, 0, 0x4A, 0, 0),
            member(0x6666_0201, 4, 0x15, 0, 0),
        ],
    };
    let mut data = vec![0u8; 0x60];
    let mut put = |at: usize, bytes: &[u8]| data[at..at + bytes.len()].copy_from_slice(bytes);
    put(0x00, &words(&[2, 2]));
    put(0x08, &pointer(elements_id, 0).to_le_bytes());
    put(0x10, &pointer(elements_id, 16).to_le_bytes());
    put(0x18, &pointer(text_id, 0).to_le_bytes());
    put(
        0x20,
        &[LONG.len() as u8, 0, LONG.len() as u8 + 1, 0, 1, 0, 0, 0],
    );
    put(0x28, &words(&[32, 2]));
    put(0x30, &pointer(pairs_id, 0).to_le_bytes());
    // The fourth word of a Vec3V is no value, and is carried.
    put(0x38, &f32_bytes(&[1.0, 2.0, 3.0, 7.0]));
    put(0x48, &0xCAFE_F00Du32.to_le_bytes());
    put(0x4C, &1u16.to_le_bytes());
    put(0x50, &pointer(text_id, LONG.len() as u64 + 1).to_le_bytes());
    put(0x58, &0x0BAD_F00Du32.to_le_bytes());

    let item = |u: u32, f: f32| [u.to_le_bytes(), f.to_le_bytes()].concat();
    let elements = [item(10, 0.25), item(20, 0.5), item(30, 0.75)].concat();
    // The second key's home slot is the first's, so it goes on to the next.
    assert_eq!(KEYS[0] & 31, KEYS[1] & 31);
    let mut slots = vec![0u8; 32 * 8];
    let home = (KEYS[0] & 31) as usize;
    slots[8 * home..8 * home + 8].copy_from_slice(&words(&[KEYS[0], 50]));
    slots[8 * home + 8..8 * home + 16].copy_from_slice(&words(&[KEYS[1], 60]));
    let text = [LONG.as_bytes(), b"\0abc\0"].concat();
    for (ty, bucket) in [(PAIR, 1), (ELEMENT, 3), (ROOT, 3), (TEXT, 5)] {
        assert_eq!(ty % 11, bucket);
    }
    let mut blocks = vec![
        (pairs_id, (PAIR, slots)),
        (elements_id, (ELEMENT, elements)),
        (root_id, (ROOT, data)),
        (text_id, (TEXT, text)),
    ];
    blocks.sort_by_key(|(id, _)| *id);
    let blocks: Vec<_> = blocks.into_iter().map(|(_, b)| b).collect();
    // `body` seeds the loader's CHKS; a save recomputes it, so a conformant body carries the valid
    // one (not a placeholder) to write back byte-identical.
    body(&[root, element, pair], &blocks, root_id as u32)
}

fn write(instance: &MetaInstance, layout: &MetaLayout) -> Result<Vec<u8>> {
    write_meta_binary(BinaryFormat::Pso, Some(layout), instance)
}

#[test]
fn a_body_laid_out_by_the_rules_writes_back_byte_identical_with_its_checksum() {
    let bytes = conformant();
    let (instance, layout) = open(&bytes).expect("it reads");
    let written = write(&instance, &layout).expect("it writes");
    assert_eq!(written, bytes);
    let (again, layout) = open(&written).expect("it reads back");
    assert_eq!(again, instance);
    assert_eq!(write(&again, &layout).expect("it writes"), written);
}

fn field(instance: &mut MetaInstance, hash: u32) -> &mut Value {
    instance.get_mut(&name(hash)).expect("the member")
}

#[test]
fn an_edited_tree_lays_out_again_reads_back_and_is_a_fixed_point() {
    let bytes = conformant();
    let (mut instance, layout) = open(&bytes).expect("it reads");
    let Value::Array(items) = field(&mut instance, 0x6666_0000) else {
        panic!("an array")
    };
    let grown = items[1].clone();
    items.push(grown);
    let Value::Map(pairs) = field(&mut instance, 0x6666_0004) else {
        panic!("a map")
    };
    pairs.push((
        Value::Hash(HashedString::from_hash(0x7777_0005)),
        Value::UInt(70),
    ));
    *field(&mut instance, 0x6666_0003) = Value::String("short".into());
    *field(&mut instance, 0x6666_000A) = Value::String("a text longer than it was".into());
    *field(&mut instance, 0x6666_0002) = Value::Pointer(None);
    *field(&mut instance, 0x6666_000B) = Value::UInt(5);

    let written = write(&instance, &layout).expect("it writes");
    assert_ne!(written, bytes);
    let (again, layout) = open(&written).expect("it reads back");
    assert_eq!(again, instance);
    assert_eq!(write(&again, &layout).expect("it writes"), written);

    // An edit recomputes the checksum: it satisfies the loader's rule and differs from the stale
    // value the body carried; the size it follows is the new length.
    let tail = &written[written.len() - 12..];
    assert_eq!(&tail[..4], &(written.len() as u32).to_be_bytes());
    let stored_sum = u32::from_be_bytes(tail[4..8].try_into().unwrap());
    assert_eq!(super::super::chks::checksum(&written), Some(stored_sum));
    assert_ne!(stored_sum, CHECKSUM);
    assert_eq!(&tail[8..], b"zppp");
    // The Vec3V's fourth word is carried too.
    let ContainerLayout::Pso(pso) = &layout.container else {
        panic!("a PSO layout")
    };
    let sections = sections_of(pso);
    let model = super::super::table_form::model(&sections).expect("a model");
    let root = model.block(model.root_block_id as i32).expect("the root");
    assert_eq!(&root.data[0x44..0x48], &7.0f32.to_le_bytes());
}

#[test]
fn a_string_hash_the_document_did_not_hold_is_refused() {
    let (mut instance, layout) = open(&conformant()).expect("it reads");
    *field(&mut instance, 0x6666_0008) = Value::Hash(HashedString::from_hash(0x1234_5678));
    let err = write(&instance, &layout).expect_err("refused").to_string();
    assert!(err.contains("which the document did not hold"), "{err}");
    *field(&mut instance, 0x6666_0008) = Value::Hash(HashedString::from_hash(0xCAFE_F00D));
    write(&instance, &layout).expect("a hash it held writes");
}

#[test]
fn a_body_the_rules_do_not_give_back_is_refused_by_name() {
    // Two blocks of one type, where the rules pack one.
    let (instance, layout) = open(&sample()).expect("it reads");
    let err = write(&instance, &layout).expect_err("refused").to_string();
    assert!(
        err.contains("the measured layout rules do not give back"),
        "{err}"
    );

    // A slot map sized against the rule.
    let (instance, layout) = open(&hash_maps_and_wide_values(|_, _| {})).expect("it reads");
    assert!(write(&instance, &layout).is_err());
}

#[test]
fn the_measured_codes_lay_out_as_they_were_built() {
    let bytes = measured_codes(|_, _| {});
    let (instance, layout) = open(&bytes).expect("it reads");
    let ContainerLayout::Pso(pso) = &layout.container else {
        panic!("a PSO layout")
    };
    let model = super::super::table_form::model(&sections_of(pso)).expect("a model");
    let (laid, _) = lay_out(&model, &instance, None).expect("it lays out");
    assert_eq!(laid.blocks.len(), model.data_blocks.len());
    // The built body's block order is not the rules', so a pointer's block id differs.
    let ids = |data: &[u8]| {
        let mut data = data.to_vec();
        for at in [0x48, 0x60, 0x70] {
            data[at..at + 2].fill(0);
        }
        data
    };
    for stored in &model.data_blocks {
        let (_, data) = (laid.blocks.iter())
            .find(|(t, _)| *t == stored.data_type)
            .expect("a block of each type");
        match stored.data_type == measured_root() {
            true => assert_eq!(ids(data), ids(&stored.data)),
            false => assert_eq!(data, &stored.data, "block {:#010X}", stored.data_type),
        }
    }
    // Laid out with nothing to carry, every byte is a value's or a rule's.
    let mut blank = model.clone();
    blank.data_blocks.iter_mut().for_each(|b| b.data.fill(0));
    let (fresh, _) = lay_out(&blank, &instance, None).expect("it lays out");
    assert_eq!(fresh.blocks, laid.blocks);
}

#[test]
fn the_xml_of_a_body_comes_back_to_its_bytes() {
    let bytes = conformant();
    let (instance, layout) = open(&bytes).expect("it reads");
    let xml = crate::meta::instance::write_meta(&instance).expect("XML");
    let tree = crate::meta::instance::parse_schemaless(xml.as_bytes()).expect("it reads back");
    assert_eq!(write(&tree, &layout).expect("it writes"), bytes);
    crate::meta::canonical::converge(&crate::meta::schema::SchemaLibrary::default(), &bytes)
        .expect("a fixed point");

    let measured = measured_codes(|_, _| {});
    let (instance, _) = open(&measured).expect("it reads");
    let xml = crate::meta::instance::write_meta(&instance).expect("XML");
    let tree = crate::meta::instance::parse_schemaless(xml.as_bytes()).expect("bytes come back");
    let ContainerLayout::Pso(pso) = &open(&measured).expect("it reads").1.container else {
        panic!("a PSO layout")
    };
    let model = super::super::table_form::model(&sections_of(pso)).expect("a model");
    assert_eq!(
        lay_out(&model, &tree, None).expect("it lays out").0,
        lay_out(&model, &instance, None).expect("it lays out").0
    );
}

#[test]
fn the_block_table_grows_before_an_insert_would_fill_it() {
    let types = [0u32, 29, 1, 2, 3, 4, 5, 6, 7, 8, 58];
    let blocks: Vec<(u32, Vec<u8>)> = types.iter().map(|&t| (t, Vec::new())).collect();
    let order = block_order(&blocks).expect("an order");
    // Grown to 29 buckets, 58 and 29 share 0's bucket, the later first.
    assert_eq!(order, [10, 1, 0, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(block_order(&blocks[..3]).expect("an order"), [0, 2, 1]);
}

#[test]
fn a_hash_table_keeps_its_stored_places_while_its_keys_are_the_ones_stored() {
    // Two keys of home slot 31: the second wraps to slot 0, so the table's slot order is not
    // the order it was filled in, and filling it in slot order would swap them.
    let keys = [0x4444_001Fu32, 0x5555_003F];
    let mut bytes = conformant();
    // The pair block is the first, after an 8-byte frame and four 16-byte rows padded to 80.
    let slots = &mut bytes[80..80 + 32 * 8];
    slots.fill(0);
    slots[8 * 31..8 * 32].copy_from_slice(&words(&[keys[0], 50]));
    slots[..8].copy_from_slice(&words(&[keys[1], 60]));
    // The edited slots changed the body, so reseed its CHKS (a save recomputes it) before the
    // byte-identical assertion.
    super::super::chks::write_checksum(&mut bytes);
    let (mut instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it writes"), bytes);

    // A key that changes places every key by the rules, in the order the tree lists them.
    let Value::Map(pairs) = field(&mut instance, 0x6666_0004) else {
        panic!("a map")
    };
    pairs[1].0 = Value::Hash(HashedString::from_hash(keys[1] + 0x20));
    let pairs = pairs.clone();
    let written = write(&instance, &layout).expect("it writes");
    let (again, _) = open(&written).expect("it reads back");
    let Some(Value::Map(placed)) = again.get(&name(0x6666_0004)) else {
        panic!("a map")
    };
    assert_eq!(
        placed[0].0, pairs[1].0,
        "listed first, it now fills slot 31 last and wraps"
    );
    assert_eq!(placed[1].0, pairs[0].0);
}

#[test]
fn the_leftover_codes_write_back_in_place_and_keep_bit_63_on_a_null() {
    let bytes = leftover_codes(|_, _| {});
    let (mut instance, layout) = open(&bytes).expect("it reads");
    let ContainerLayout::Pso(pso) = &layout.container else {
        panic!("a PSO layout")
    };
    let sections = sections_of(pso);
    let model = super::super::table_form::model(&sections).expect("a model");
    let stored: Vec<(u32, Vec<u8>)> = (model.data_blocks.iter())
        .map(|b| (b.data_type, b.data.clone()))
        .collect();
    let (laid, _) = lay_out_in_place(&model, &instance, None).expect("it lays out");
    assert_eq!(laid.blocks, stored);

    *field(&mut instance, 0x7777_0002) = Value::Pointer(None);
    *field(&mut instance, 0x7777_0008) = Value::Float(-0.125e-300);
    let Value::Array(vectors) = field(&mut instance, 0x7777_0003) else {
        panic!("an array")
    };
    vectors[1] = Value::Vec3([-1.0, 0.5, 9.0]);
    let Value::Array(fixed) = field(&mut instance, 0x7777_0005) else {
        panic!("an array")
    };
    fixed[0] = Value::Array(vec![Value::Float(3.5)]);
    let (laid, _) = lay_out_in_place(&model, &instance, None).expect("it lays out");
    let root = &laid.blocks[0].1;
    assert_eq!(&root[0x18..0x20], &TAG.to_le_bytes());
    assert_eq!(root[0x0F] & 0x80, 0x80, "a tagged array's pointer");
    let (again, _) = open(&emit(&sections, &laid)).expect("it reads back");
    assert_eq!(again, instance);
}

#[test]
fn an_array_of_strings_reads_back_and_writes_back_byte_identical() {
    const STRINGS: u32 = 0x44;
    for (ty, bucket) in [(STRINGS, 2), (ROOT, 3), (TEXT, 5)] {
        assert_eq!(ty % 11, bucket);
    }
    let anon = member(ARRAY_INFO_NAME_HASH, 0, 0x44, 0, 0);
    let root = Class {
        name: ROOT,
        size: 0x10,
        members: vec![member(0x6666_0000, 0x00, COUNTED_ARRAY, 1, 0), anon],
    };
    let mut data = words(&[2, 2]);
    data.extend(pointer(1, 0).to_le_bytes());
    // One string in place, its length in the last byte; one behind a pointer into the text.
    let mut elements = vec![0u8; 32];
    elements[..5].copy_from_slice(b"short");
    elements[15] = 0x80 | 5;
    elements[16..24].copy_from_slice(&pointer(3, 0).to_le_bytes());
    let len = LONG.len() as u8;
    elements[24..29].copy_from_slice(&[len, 0, len + 1, 0, 1]);
    let text = [LONG.as_bytes(), b"\0"].concat();
    let bytes = body(
        &[root],
        &[(STRINGS, elements), (ROOT, data), (TEXT, text)],
        2,
    );
    let (instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(
        instance.get(&name(0x6666_0000)),
        Some(&Value::Array(vec![
            Value::String("short".into()),
            Value::String(LONG.into())
        ]))
    );
    let written = write(&instance, &layout).expect("it writes");
    assert_eq!(written, bytes);
    let (again, layout) = open(&written).expect("it reads back");
    assert_eq!(again, instance);
    assert_eq!(write(&again, &layout).expect("it writes"), written);
}

#[test]
fn an_inline_bitset_refuses_a_bit_past_the_count_it_holds() {
    let root = Class {
        name: ROOT,
        size: 0x20,
        members: vec![member(0x6666_0000, 0, INLINE_BITSET, 129, 0)],
    };
    let bytes = body(&[root], &[(ROOT, vec![0; 0x20])], 1);
    let (mut instance, layout) = open(&bytes).expect("it reads");
    let ContainerLayout::Pso(pso) = &layout.container else {
        panic!("a PSO layout")
    };
    let model = super::super::table_form::model(&sections_of(pso)).expect("a model");
    // 129 bits take three 64-bit words; bit 128 is the last the count holds.
    let mut words = vec![0u8; 24];
    words[16] = 1;
    *field(&mut instance, 0x6666_0000) = Value::ByteArray(words.clone());
    lay_out(&model, &instance, None).expect("bit 128 is held");
    words[16] = 2;
    *field(&mut instance, 0x6666_0000) = Value::ByteArray(words);
    let err = lay_out(&model, &instance, None)
        .expect_err("refused")
        .to_string();
    assert!(err.contains("it sets a bit past its 129"), "{err}");
}
