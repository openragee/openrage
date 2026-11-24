use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::{parse_meta_binary_full, write_meta_binary, BinaryFormat, MetaLayout};
use crate::meta::diagnostics::Diagnostics;
use crate::meta::error::Result;
use crate::meta::instance::value::{HashedString, MetaInstance, Value};
use crate::meta::schema::SchemaLibrary;

use super::super::super::reader::{GTA6_COUNT_FIRST_ARRAY, GTA6_GROUP_MAP, GTA6_STRING_HASH};
use super::super::super::schema::{string_subtype, ARRAY_INFO_NAME_HASH};
use super::super::super::table_layout::block_order;
use super::super::{packed, PAD};

const ROOT: u32 = 0xE1E1_0001;
const PAIR: u32 = 0xE1E1_0002;
const MAP: u32 = 0xE1E1_0010;
const KEY: u32 = 0xE1E1_0011;
const VALUE: u32 = 0xE1E1_0012;
const UINT: u8 = 0x06;
const HASH: u8 = string_subtype::HASH;
/// A key whose bit 7 is set, so its home is group 1 of 2.
const HELD: u32 = 0xC318_70ED;
/// A key whose bit 7 is clear, so its home is group 0 of 2.
const LOW: u32 = 0x1234_5655;
/// The second and fourth words of a pair array's header, carried as stored.
const CARRIED: [u32; 2] = [0x0200_0000, 0x1A];
/// A pair: the key at 0, the array header at 8.
const PAIR_SIZE: usize = 24;
/// A group: sixteen control bytes, then sixteen pairs.
const UNIT: usize = 16 + 16 * PAIR_SIZE;

fn home(key: u32, groups: usize) -> usize {
    ((key >> 7) as usize) & (groups - 1)
}

/// The two u32s a pair of `key` holds, told apart by key.
fn elements(key: u32) -> [u32; 2] {
    [key >> 16, key & 0xFFFF]
}

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

/// A P-form document placed as the measured rules place one: a root whose group map of `groups`
/// groups holds `pairs`, `(key, group * 16 + slot)` in the order the walk reaches them, each with
/// its two `elements` behind a header of count 2 and the carried words.
fn document(groups: usize, key_subtype: u8, pairs: &[(u32, usize)]) -> Vec<u8> {
    let mut table = vec![0u8; UNIT * groups];
    let mut values = Vec::new();
    for group in 0..groups {
        table[UNIT * group..UNIT * group + 16].fill(0x80);
    }
    for &(key, slot) in pairs {
        let (group, j) = (UNIT * (slot / 16), slot % 16);
        table[group + (j ^ 3)] = (key & 0x7F) as u8;
        let at = group + 16 + PAIR_SIZE * j;
        table[at..at + 4].copy_from_slice(&key.to_be_bytes());
        table[at + 8..at + 12].copy_from_slice(&2u32.to_be_bytes());
        table[at + 12..at + 16].copy_from_slice(&CARRIED[0].to_be_bytes());
        table[at + 20..at + 24].copy_from_slice(&CARRIED[1].to_be_bytes());
        values.extend(elements(key).iter().flat_map(|e| e.to_be_bytes()));
    }
    // In the order the walk first reaches them: the root, the table, the arrays' elements.
    let mut blocks = vec![(ROOT, vec![0u8; 0x10])];
    if !pairs.is_empty() {
        blocks.push((PAIR, table));
        blocks.push((u32::from(UINT), values));
    }
    let order = block_order(&blocks).expect("an order");
    let id = |b: usize| order.iter().position(|&x| x == b).expect("placed") + 1;
    if !pairs.is_empty() {
        let header = [
            (groups as u32).to_be_bytes(),
            (pairs.len() as u32).to_be_bytes(),
        ];
        blocks[0].1[..8].copy_from_slice(&header.concat());
        blocks[0].1[8..].copy_from_slice(&packed(id(1), 0));
        for (i, &(_, slot)) in pairs.iter().enumerate() {
            let pointer = UNIT * (slot / 16) + 16 + PAIR_SIZE * (slot % 16) + 16;
            blocks[1].1[pointer..pointer + 4].copy_from_slice(&packed(id(2), 8 * i)[..4]);
        }
    }

    let mut psin = b"PSIN\0\0\0\0".to_vec();
    psin.extend_from_slice(&[PAD; 8]);
    let mut pmap = (id(0) as i32).to_be_bytes().to_vec();
    pmap.extend_from_slice(&(blocks.len() as i16).to_be_bytes());
    pmap.extend_from_slice(&0i16.to_be_bytes());
    for &b in &order {
        while !psin.len().is_multiple_of(16) {
            psin.push(PAD);
        }
        let (hash, data) = &blocks[b];
        for word in [*hash, psin.len() as u32, 0, data.len() as u32] {
            pmap.extend_from_slice(&word.to_be_bytes());
        }
        psin.extend_from_slice(data);
    }
    let len = psin.len() as u32;
    psin[4..8].copy_from_slice(&len.to_be_bytes());

    // The root describes the map's key, element and value; the map names its value as member 2
    // of the root, and the value its elements as member 1 of the root.
    let mut items = Vec::new();
    items.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 0x10, 0, 0, 0, 0]);
    entry(&mut items, ARRAY_INFO_NAME_HASH, 0x0B, key_subtype, 0, 0);
    entry(&mut items, ARRAY_INFO_NAME_HASH, UINT, 0, 0, 0);
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
    entry(&mut items, KEY, 0x0B, key_subtype, 0, 0);
    entry(&mut items, VALUE, 0x0D, GTA6_COUNT_FIRST_ARRAY, 8, 1);
    let start = 8 + 4 + 2 * 8;
    let mut psch = 2u32.to_be_bytes().to_vec();
    for (hash, offset) in [(ROOT, start), (PAIR, start + pair_at)] {
        psch.extend_from_slice(&hash.to_be_bytes());
        psch.extend_from_slice(&(offset as u32).to_be_bytes());
    }
    psch.extend_from_slice(&items);

    let mut out = psin;
    section(&mut out, b"PMAP", &pmap);
    section(&mut out, b"PSCH", &psch);
    let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
    chks.extend_from_slice(&[0; 4]);
    chks.extend_from_slice(b"zppp");
    section(&mut out, b"CHKS", &chks);
    crate::meta::binary::pso::chks::write_checksum(&mut out);
    out
}

fn open(bytes: &[u8]) -> (MetaInstance, MetaLayout) {
    parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut Diagnostics::strict())
        .expect("it reads")
}

fn write(instance: &MetaInstance, layout: &MetaLayout) -> Result<Vec<u8>> {
    write_meta_binary(BinaryFormat::Pso, Some(layout), instance)
}

fn named(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

/// The pair of `key` as a read gives it, holding `count` of its elements.
fn pair(key: u32, count: usize) -> (Value, Value) {
    let raw = default_hash_index().lookup(key).map(str::to_string);
    let key_value = Value::Hash(HashedString {
        raw: raw.unwrap_or_default(),
        hash: key,
    });
    let items = (elements(key).iter().cycle().take(count))
        .map(|&e| Value::UInt(e.into()))
        .collect();
    (key_value, Value::Array(items))
}

/// `tree` with its map holding `pairs` in that order.
fn with_pairs(tree: &MetaInstance, pairs: Vec<(Value, Value)>) -> MetaInstance {
    let mut edited = tree.clone();
    *edited.get_mut(&named(MAP)).expect("the map") = Value::Map(pairs);
    edited
}

fn sorted_pairs(tree: &MetaInstance) -> Vec<(Value, Value)> {
    let Some(Value::Map(pairs)) = tree.get(&named(MAP)) else {
        panic!("a map")
    };
    let mut pairs = pairs.clone();
    pairs.sort_by_key(|(k, _)| match k {
        Value::Hash(h) => h.hash,
        _ => 0,
    });
    pairs
}

#[test]
fn a_re_keyed_pair_moves_to_the_home_group_of_its_new_key() {
    let bytes = document(2, HASH, &[(HELD, 16 * home(HELD, 2))]);
    let (tree, layout) = open(&bytes);
    assert_eq!(write(&tree, &layout).expect("it saves"), bytes);
    assert_ne!(home(LOW, 2), home(HELD, 2), "the new key changes group");

    let edited = with_pairs(&tree, vec![pair(LOW, 2)]);
    let written = write(&edited, &layout).expect("it lays out anew");
    assert_eq!(written, document(2, HASH, &[(LOW, 16 * home(LOW, 2))]));
    let (again, layout) = open(&written);
    assert_eq!(again, edited);
    assert_eq!(write(&again, &layout).expect("it saves"), written);
}

#[test]
fn a_removed_pair_added_back_last_gives_the_stored_bytes_back() {
    let stored = [(LOW, 16 * home(LOW, 2)), (HELD, 16 * home(HELD, 2))];
    let bytes = document(2, HASH, &stored);
    let (tree, layout) = open(&bytes);
    assert_eq!(write(&tree, &layout).expect("it saves"), bytes);
    // The same pairs in another order are saved into the slots that hold their keys.
    let reversed = with_pairs(&tree, vec![pair(HELD, 2), pair(LOW, 2)]);
    assert_eq!(write(&reversed, &layout).expect("it saves"), bytes);

    let removed = with_pairs(&tree, vec![pair(LOW, 2)]);
    let written = write(&removed, &layout).expect("it lays out anew");
    assert_eq!(written, document(2, HASH, &stored[..1]));
    let (again, layout) = open(&written);
    assert_eq!(again, removed);
    let restored = with_pairs(&again, vec![pair(LOW, 2), pair(HELD, 2)]);
    assert_eq!(write(&restored, &layout).expect("it lays out anew"), bytes);
}

#[test]
fn twenty_eight_pairs_take_four_groups_each_key_in_its_home_group() {
    let bytes = document(2, HASH, &[(HELD, 16 * home(HELD, 2))]);
    let (tree, layout) = open(&bytes);
    let keys: Vec<u32> = std::iter::once(HELD)
        .chain((1..28).map(|i| (i << 7) | 0x11))
        .collect();
    let mut full = [0usize; 4];
    let placed: Vec<(u32, usize)> = (keys.iter())
        .map(|&key| {
            let group = home(key, 4);
            full[group] += 1;
            (key, 16 * group + full[group] - 1)
        })
        .collect();
    assert!(full.iter().all(|&n| n < 16), "no group overflows");

    let edited = with_pairs(&tree, keys.iter().map(|&k| pair(k, 2)).collect());
    let written = write(&edited, &layout).expect("it lays out anew");
    assert_eq!(written, document(4, HASH, &placed));
    // A read gives the pairs in slot order.
    let (again, layout) = open(&written);
    assert_eq!(sorted_pairs(&again), sorted_pairs(&edited));
    assert_eq!(write(&again, &layout).expect("it saves"), written);
}

#[test]
fn an_emptied_group_map_is_written_null() {
    let bytes = document(2, HASH, &[(HELD, 16 * home(HELD, 2))]);
    let (tree, layout) = open(&bytes);
    let emptied = with_pairs(&tree, Vec::new());
    let written = write(&emptied, &layout).expect("it lays out anew");
    assert_eq!(written, document(2, HASH, &[]));
    let (again, _) = open(&written);
    assert_eq!(again, emptied);
}

#[test]
fn a_group_map_outside_the_measured_rules_is_refused_by_name() {
    let bytes = document(2, HASH, &[(HELD, 16 * home(HELD, 2))]);
    let (tree, layout) = open(&bytes);
    let longer = with_pairs(&tree, vec![pair(HELD, 2), pair(LOW, 3)]);
    let err = write(&longer, &layout).expect_err("refused").to_string();
    assert!(err.contains("no stored pair array of that count"), "{err}");
    let repeated = with_pairs(&tree, vec![pair(HELD, 2), pair(HELD, 2)]);
    let err = write(&repeated, &layout).expect_err("refused").to_string();
    assert!(err.contains("twice"), "{err}");

    let bytes = document(2, GTA6_STRING_HASH, &[(HELD, 16 * home(HELD, 2))]);
    let (tree, layout) = open(&bytes);
    let grown = with_pairs(&tree, vec![pair(HELD, 2), pair(LOW, 2)]);
    let err = write(&grown, &layout).expect_err("refused").to_string();
    assert!(err.contains("no other pair shape is known"), "{err}");
}
