use super::super::sections::split_sections;
use super::super::table_form::tests::{body, member, open, pointer, Class, ELEMENT, KIND, ROOT};
use super::super::table_form::{ARRAY_INFO_NAME_HASH, SLOT_MAP};
use super::super::table_layout::block_order;
use crate::meta::binary::enum_text;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::{write_meta_binary, BinaryFormat, MetaLayout};
use crate::meta::error::Result;
use crate::meta::instance::value::{MetaInstance, Value};

const TEXT: u32 = 0x10;
const PAIR: u32 = 0x1111_0021;
/// An enum no `LENM` record declares.
const UNDECLARED: u32 = 0x9999_0001;
const LONG: &str = "a longer string here";

fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn write(instance: &MetaInstance, layout: &MetaLayout) -> Result<Vec<u8>> {
    write_meta_binary(BinaryFormat::Pso, Some(layout), instance)
}

/// `blocks`, given in the order a layout first reaches them, as the body the rules lay them out
/// in, or in the reverse of that order with `reversed`; `data` writes a block's bytes from the
/// table id of each block.
fn laid_body(
    classes: &[Class],
    types: &[u32],
    reversed: bool,
    data: impl Fn(&dyn Fn(usize) -> u64) -> Vec<Vec<u8>>,
) -> Vec<u8> {
    let empty: Vec<(u32, Vec<u8>)> = types.iter().map(|&ty| (ty, Vec::new())).collect();
    let mut order = block_order(&empty).expect("an order");
    if reversed {
        order.reverse();
    }
    let id = |k: usize| {
        order
            .iter()
            .position(|&b| b == k)
            .map_or(0, |at| at as u64 + 1)
    };
    let mut bytes = data(&id);
    let blocks: Vec<(u32, Vec<u8>)> = (order.iter())
        .map(|&b| (types[b], std::mem::take(&mut bytes[b])))
        .collect();
    let mut out = body(classes, &blocks, id(0) as u32);
    // The fixture carries the checksum the loader's rule gives, as a canonical body does.
    crate::meta::binary::pso::chks::write_checksum(&mut out);
    out
}

/// A root that points at an element, its blocks in the rules' order or the reverse of it.
fn pointed(reversed: bool) -> Vec<u8> {
    let root = Class {
        name: ROOT,
        size: 0x10,
        members: vec![member(0x7777_0000, 0, 0x07, 0, 0)],
    };
    let element = Class {
        name: ELEMENT,
        size: 8,
        members: vec![
            member(0x7777_0001, 0, 0x15, 0, 0),
            member(0x7777_0002, 4, 0x21, 0, 0),
        ],
    };
    laid_body(&[root, element], &[ROOT, ELEMENT], reversed, |id| {
        let mut data = vec![0u8; 0x10];
        data[..8].copy_from_slice(&pointer(id(1), 0).to_le_bytes());
        vec![data, [7u32.to_le_bytes(), 0.5f32.to_le_bytes()].concat()]
    })
}

#[test]
fn a_body_whose_blocks_the_rules_order_otherwise_saves_in_canonical_form_as_a_fixed_point() {
    let canonical = pointed(false);
    let (instance, layout) = open(&canonical).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it saves"), canonical);

    let stored = pointed(true);
    assert_ne!(stored, canonical);
    let (instance, layout) = open(&stored).expect("it reads");
    let saved = write(&instance, &layout).expect("it saves in canonical form");
    assert_eq!(saved, canonical);
    let (again, layout) = open(&saved).expect("it reads back");
    assert_eq!(again, instance);
    assert_eq!(write(&again, &layout).expect("it saves"), saved);

    // An edit saves in the canonical form too.
    let mut edited = instance.clone();
    let Some(Value::Pointer(Some(element))) = edited.get_mut(&name(0x7777_0000)) else {
        panic!("a pointer")
    };
    *element.get_mut(&name(0x7777_0001)).expect("the u32") = Value::UInt(9);
    let (_, layout) = open(&stored).expect("it reads");
    let saved = write(&edited, &layout).expect("it saves");
    let (again, _) = open(&saved).expect("it reads back");
    assert_eq!(again, edited);
}

/// A slot map keyed by an enum of `owner`, whose pair structure's key member names `pair`.
fn enum_keyed(owner: u32, pair: u32) -> Vec<u8> {
    let anon = |code: u8, u12: u32| member(ARRAY_INFO_NAME_HASH, 0, code, 0, u12);
    let root = Class {
        name: ROOT,
        size: 0x10,
        members: vec![
            member(0x7777_0010, 0, SLOT_MAP, 0, 2 << 16 | 1),
            anon(0x60, owner),
            anon(0x15, 0),
        ],
    };
    let pairs = Class {
        name: PAIR,
        size: 8,
        members: vec![
            member(0x7777_0011, 0, 0x60, 0, pair),
            member(0x7777_0012, 4, 0x15, 0, 0),
        ],
    };
    laid_body(&[root, pairs], &[ROOT, PAIR], false, |id| {
        let mut data = vec![0u8; 0x10];
        data[..8].copy_from_slice(&[32, 0, 0, 0, 1, 0, 0, 0]);
        data[8..].copy_from_slice(&pointer(id(1), 0).to_le_bytes());
        // Key 1 in the first of 32 slots, the value 50.
        let mut slots = vec![0u8; 32 * 8];
        slots[0] = 1;
        slots[4..8].copy_from_slice(&50u32.to_le_bytes());
        vec![data, slots]
    })
}

fn map_key(instance: &MetaInstance) -> &Value {
    let Some(Value::Map(pairs)) = instance.get(&name(0x7777_0010)) else {
        panic!("a map")
    };
    &pairs[0].0
}

#[test]
fn a_hash_map_key_is_named_and_written_in_its_owner_s_key_enum_not_its_pair_structure_s() {
    // The pair structure names an enum where 1 has a name the owner's enum does not hold.
    let bytes = enum_keyed(UNDECLARED, KIND);
    let (instance, layout) = open(&bytes).expect("it reads");
    let expected = Value::Enum {
        type_name: name(UNDECLARED),
        variant: enum_text::value_text(1),
    };
    assert_eq!(map_key(&instance), &expected);
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);

    // The owner's enum names the key where the pair structure's declares no value.
    let bytes = enum_keyed(KIND, UNDECLARED);
    let (instance, layout) = open(&bytes).expect("it reads");
    let Value::Enum { type_name, variant } = map_key(&instance) else {
        panic!("an enum")
    };
    assert_eq!(type_name, &name(KIND));
    assert_ne!(variant, &enum_text::value_text(1));
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);
}

/// A root with an empty text stored with 0x80 in its last byte, a text out of place whose word at
/// 12 is 0, and two Vec3V elements of a 0x54 array whose fourth lanes hold values.
fn carried() -> Vec<u8> {
    let root = Class {
        name: ROOT,
        size: 0x50,
        members: vec![
            member(0x7777_0020, 0x00, 0x44, 0, 0),
            member(0x7777_0021, 0x10, 0x44, 0, 0),
            member(0x7777_0022, 0x20, 0x54, 3, 2),
            member(ARRAY_INFO_NAME_HASH, 0, 0x33, 0, 0),
        ],
    };
    laid_body(&[root], &[ROOT, TEXT], false, |id| {
        let mut data = vec![0u8; 0x50];
        data[0x0F] = 0x80;
        data[0x10..0x18].copy_from_slice(&pointer(id(1), 0).to_le_bytes());
        let len = LONG.len() as u8;
        data[0x18..0x20].copy_from_slice(&[len, 0, len + 1, 0, 0, 0, 0, 0]);
        data[0x20..0x28].copy_from_slice(&[2, 0, 0, 0, 2, 0, 0, 0]);
        for (at, lanes) in [
            (0x30, [1.0f32, 2.0, 3.0, 7.0]),
            (0x40, [4.0, 5.0, 6.0, -1.25]),
        ] {
            let bytes: Vec<u8> = lanes.iter().flat_map(|f| f.to_le_bytes()).collect();
            data[at..at + 16].copy_from_slice(&bytes);
        }
        vec![data, [LONG.as_bytes(), b"\0"].concat()]
    })
}

#[test]
fn header_words_and_vector_lanes_no_value_states_are_kept_through_a_save_and_an_edit() {
    let bytes = carried();
    let (mut instance, layout) = open(&bytes).expect("it reads");
    assert_eq!(write(&instance, &layout).expect("it saves"), bytes);

    let Some(Value::Array(items)) = instance.get_mut(&name(0x7777_0022)) else {
        panic!("an array")
    };
    items[0] = Value::Vec3([8.0, 9.0, 10.0]);
    *instance.get_mut(&name(0x7777_0021)).expect("the text") =
        Value::String("another string, longer".into());
    let saved = write(&instance, &layout).expect("it saves");
    let (again, _) = open(&saved).expect("it reads back");
    assert_eq!(again, instance);

    let sections = split_sections(&saved).expect("its sections");
    let model = super::super::table_form::model(&sections).expect("a model");
    let root = &model
        .block(model.root_block_id as i32)
        .expect("the root")
        .data;
    // The empty text keeps its 0x80, the pointed one its word at 12 of 0, the edited element and
    // the one left alone their fourth lanes.
    assert!(root[..0x0F].iter().all(|&byte| byte == 0));
    assert_eq!(root[0x0F], 0x80);
    assert_eq!(&root[0x1C..0x20], &[0, 0, 0, 0]);
    assert_eq!(&root[0x3C..0x40], &7.0f32.to_le_bytes());
    assert_eq!(&root[0x4C..0x50], &(-1.25f32).to_le_bytes());
}
