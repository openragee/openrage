//! A GTA VI headerless RBM tree whose maps gained or lost pairs, such as an anim scene's entity
//! or scene: the body is laid out anew, each table rebuilt from its pairs with an added one last
//! and every region after it moved, where the unchanged tree's data blocks lay out anew as stored.
//!
//! ⚠ Where a body's data blocks do not lay out again as stored, where a new pair's regions go and
//! which order its blocks take are not known, so a pair is neither added nor removed there. The
//! gate judges the data blocks alone: where the sections place them is not compared, since a
//! pointer names a block and an offset in it, and a block of a hash table's type may differ
//! inside, an equally valid slot layout of the same pairs. A changed table is refilled from the slot after its
//! first empty slot (a slot map) or the group after its first that is not full (a group map),
//! so a stored pair whose probe chain is unchanged keeps its slot.

use std::collections::HashMap;

use super::schema::{PsoType, RbmClass, RbmClassEntry, RbmModel};
use crate::meta::binary::layout::RbmLayout;
use crate::meta::binary::pso::table_form;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::Value;
use crate::meta::instance::MetaInstance;

const SLOT_MAP: u8 = 0x73;
const GROUP_MAP: u8 = 0x75;
/// The name a class's array-info member takes, which names no member of a pair.
const ARRAY_INFO: u32 = 0x100;
/// A group's control byte for an empty place.
const EMPTY_CONTROL: u8 = 0x80;

/// How a table's block is cut: whether in groups of 16 control bytes then 16 pairs, and a pair's size.
type TableCut = (bool, usize);

/// Whether `instance` adds a pair to or takes one from a map of the tree `layout`'s body stores.
pub(super) fn pairs_changed(layout: &RbmLayout, instance: &MetaInstance) -> Result<bool> {
    let stored = table_form::decode(&layout.model)?;
    if !instance_keys_differ(&stored, instance) {
        return Ok(false);
    }
    let again = super::gta6_relayout::write(layout, &stored)?;
    if !only_tables_differ(&layout.model, &again) {
        return Err(MetaError::Other(
            "GTA VI RBM: a map's pairs changed, and this body's data blocks do not lay out again as it stores them, so where an added or removed pair's regions go is not known".into(),
        ));
    }
    Ok(true)
}

/// Whether the sections `again`, laid out anew from the unchanged tree of `model`'s body, hold
/// its classes, enums and root and its data blocks: as many, each of its type and length, its bytes
/// the same or, in a table's, the same pairs in any slots.
fn only_tables_differ(model: &RbmModel, again: &(Vec<u8>, Vec<u8>)) -> bool {
    let Ok(laid) = super::gta6::read_model(&again.0, &again.1) else {
        return false;
    };
    let tables = table_types(model);
    let (old, new) = (&model.data_blocks, &laid.data_blocks);
    laid.root_block_id == model.root_block_id
        && laid.class_order == model.class_order
        && laid.enum_order == model.enum_order
        && laid.classes == model.classes
        && laid.enums == model.enums
        && old.len() == new.len()
        && old.iter().zip(new).all(|(a, b)| {
            a.data_type == b.data_type
                && a.data.len() == b.data.len()
                && (a.data == b.data
                    || tables.get(&a.data_type).is_some_and(|&cut| {
                        let held = held_pairs(&a.data, cut);
                        held.is_some() && held == held_pairs(&b.data, cut)
                    }))
        })
}

/// The pairs a table's block `data` holds, sorted: each slot not all zero, or each place of a
/// group whose control byte is not empty, with that byte; none where `data` is not whole units.
fn held_pairs(data: &[u8], (grouped, pair): TableCut) -> Option<Vec<(u8, &[u8])>> {
    let unit = if grouped { 16 + 16 * pair } else { pair };
    if !data.len().is_multiple_of(unit) {
        return None;
    }
    let mut held: Vec<(u8, &[u8])> = if grouped {
        (data.chunks_exact(unit))
            .flat_map(|g| g[..16].iter().copied().zip(g[16..].chunks_exact(pair)))
            .filter(|&(control, _)| control != EMPTY_CONTROL)
            .collect()
    } else {
        (data.chunks_exact(pair))
            .filter(|slot| slot.iter().any(|&b| b != 0))
            .map(|slot| (0, slot))
            .collect()
    };
    held.sort_unstable();
    Some(held)
}

/// The types of the blocks a hash table is laid out in, with how each is cut: a slot map's pair
/// structure and a group map's group structure, found as the writer finds them where no stored
/// table names one.
fn table_types(model: &RbmModel) -> HashMap<u32, TableCut> {
    let types = |e: &RbmClassEntry| (e.data_type, e.union_ch);
    let mut out = HashMap::new();
    for owner in model.classes.values() {
        for map in &owner.entries {
            let grouped = match map.data_type {
                PsoType::Unknown(SLOT_MAP) => false,
                PsoType::Unknown(GROUP_MAP) => true,
                _ => continue,
            };
            let pair = [
                owner.entry(map.map_key_index()).map(types),
                owner.entry(map.map_value_index()).map(types),
            ];
            let is_pair = |c: &RbmClass| {
                let named: Vec<_> = (c.entries.iter())
                    .filter(|e| e.name_hash != ARRAY_INFO)
                    .map(|e| Some(types(e)))
                    .collect();
                named == pair
            };
            for c in model.classes.values() {
                let pair_size = match grouped {
                    false => is_pair(c).then_some(c.data_size),
                    true => (c.entries.len() == 4 && c.entries[2].data_type == PsoType::Struct)
                        .then(|| model.class_for(c.entries[2].referent_hash()))
                        .flatten()
                        .filter(|&p| {
                            is_pair(p) && u64::from(c.data_size) == 16 + 16 * u64::from(p.data_size)
                        })
                        .map(|p| p.data_size),
                };
                if let Some(size) = pair_size.filter(|&s| s > 0) {
                    out.insert(c.name_hash, (grouped, size as usize));
                }
            }
        }
    }
    out
}

fn instance_keys_differ(stored: &MetaInstance, edited: &MetaInstance) -> bool {
    (stored.fields.iter()).any(|(name, s)| edited.get(name).is_some_and(|e| keys_differ(s, e)))
}

/// Whether a map in `stored` holds other keys than the same map in `edited`.
// ponytail: a map inside an array whose length changed is not compared; match the elements as
// the array grower does if such a map's pairs must change in the same save.
fn keys_differ(stored: &Value, edited: &Value) -> bool {
    match (stored, edited) {
        (Value::Struct(s), Value::Struct(e)) => instance_keys_differ(s, e),
        (Value::Pointer(Some(s)), Value::Pointer(Some(e))) => instance_keys_differ(s, e),
        (Value::Array(s), Value::Array(e)) => {
            s.len() == e.len() && s.iter().zip(e).any(|(s, e)| keys_differ(s, e))
        }
        (Value::Map(s), Value::Map(e)) => {
            !(s.iter().map(|(k, _)| k)).eq(e.iter().map(|(k, _)| k))
                || (s.iter().zip(e)).any(|((_, s), (_, e))| keys_differ(s, e))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::gta6::{open, read_model};
    use super::super::gta6_grow::tests::{body, member, pointer};
    use super::super::gta6_grow_tree::write_grown;
    use super::super::schema::RbmDataBlock;
    use super::*;
    use crate::container::rpf::Rpf9Sections;
    use crate::meta::binary::hash_index::default_hash_index;
    use crate::meta::binary::layout::{ContainerLayout, MetaLayout};
    use crate::meta::instance::value::HashedString;

    const ROOT: u32 = 0x4848_0001;
    const PAIR: u32 = 0x4848_0002;
    const GROUP: u32 = 0x4848_0003;
    const SCENES: u32 = 0x4848_0010;
    const ARRAY_INFO: u32 = 0x100;
    const SLOTS: usize = 32;
    const GROUPS: usize = 64;
    /// Keys of home slots 5 and 19, and one more of home slot 5.
    const KEYS: [u32; 3] = [0x1234_0005, 0x1234_0013, 0x1234_0025];

    fn name(hash: u32) -> String {
        default_hash_index().name_or_literal(hash)
    }

    /// A root whose one member is a hash map of type code `code` of `{hash, u32}` pairs, stating
    /// `head` (its slot or group count, then its pair count) and its table, a block of `ty`.
    fn map_body(code: u8, head: [u32; 2], ty: u32, table: Vec<u8>) -> Rpf9Sections {
        let root = (
            ROOT,
            0x10,
            vec![
                member(SCENES, 0, code, 0, 2 << 16 | 1),
                member(ARRAY_INFO, 0, 0x4A, 0, 0),
                member(ARRAY_INFO, 0, 0x15, 0, 0),
            ],
        );
        let pair = (
            PAIR,
            8,
            vec![
                member(0x4848_0020, 0, 0x4A, 0, 0),
                member(0x4848_0021, 4, 0x15, 0, 0),
            ],
        );
        let mut classes = vec![root, pair];
        if code == GROUP_MAP {
            // Sixteen control bytes, then sixteen pairs.
            let group = vec![
                member(ARRAY_INFO, 0, 0x11, 0, 0),
                member(0x4848_0030, 0, 0x50, 0, 16),
                member(ARRAY_INFO, 0, 0x05, 0, PAIR),
                member(0x4848_0031, 16, 0x50, 2, 16),
            ];
            classes.push((GROUP, 16 + 16 * 8, group));
        }
        let mut data: Vec<u8> = head.iter().flat_map(|w| w.to_le_bytes()).collect();
        data.extend(pointer(2, 0).to_le_bytes());
        body(&classes, &[(ROOT, data), (ty, table)], 1)
    }

    /// A slot map holding each `(key, value, slot)` of `held`.
    fn slot_map_of(held: &[(u32, u32, usize)]) -> Rpf9Sections {
        let mut table = vec![0u8; 8 * SLOTS];
        for &(key, value, slot) in held {
            table[8 * slot..8 * slot + 4].copy_from_slice(&key.to_le_bytes());
            table[8 * slot + 4..8 * slot + 8].copy_from_slice(&value.to_le_bytes());
        }
        let head = [SLOTS as u32, held.len() as u32];
        map_body(SLOT_MAP, head, PAIR, table)
    }

    /// A slot map holding the first two keys at their home slots.
    fn slot_map() -> Rpf9Sections {
        slot_map_of(&[(KEYS[0], 50, 5), (KEYS[1], 60, 19)])
    }

    /// A group map of 447 pairs in 64 groups, where the rule gives 32: seven pairs in each group
    /// but the last, which holds six, each key of its group's home.
    fn group_map() -> Rpf9Sections {
        let mut table = Vec::with_capacity(GROUPS * (16 + 16 * 8));
        let mut n = 0u32;
        for g in 0..GROUPS {
            let mut controls = [0x80u8; 16];
            let mut entries = [0u8; 16 * 8];
            let held = if g + 1 == GROUPS { 6 } else { 7 };
            for (j, control) in controls.iter_mut().enumerate().take(held) {
                let key = 0x4900_0000 | (j as u32) << 16 | (g as u32) << 7 | (j as u32 + 1);
                *control = (key & 0x7F) as u8;
                entries[8 * j..8 * j + 4].copy_from_slice(&key.to_le_bytes());
                entries[8 * j + 4..8 * j + 8].copy_from_slice(&n.to_le_bytes());
                n += 1;
            }
            table.extend(controls);
            table.extend(entries);
        }
        assert_eq!(n, 447);
        map_body(GROUP_MAP, [GROUPS as u32, n], GROUP, table)
    }

    fn opened(v: &[u8], p: &[u8]) -> (MetaInstance, RbmLayout) {
        let (instance, MetaLayout { container, .. }) = open(v, p).expect("it opens");
        let ContainerLayout::Rbm(layout) = container else {
            panic!("a headerless RBM layout");
        };
        (instance, *layout)
    }

    fn pairs(tree: &mut MetaInstance) -> &mut Vec<(Value, Value)> {
        match tree.get_mut(&name(SCENES)) {
            Some(Value::Map(pairs)) => pairs,
            _ => panic!("a map"),
        }
    }

    /// Each pair's key hash and value, by key.
    fn held(tree: &mut MetaInstance) -> Vec<(u32, u64)> {
        let mut held: Vec<(u32, u64)> = (pairs(tree).iter())
            .map(|pair| match pair {
                (Value::Hash(h), Value::UInt(v)) => (h.hash, *v),
                other => panic!("a hash and a u32: {other:?}"),
            })
            .collect();
        held.sort_unstable();
        held
    }

    /// The slot map laid out anew once, which lays out again as it is.
    fn canonical() -> (Vec<u8>, Vec<u8>) {
        let s = slot_map();
        let (tree, layout) = opened(&s.virtual_section, &s.physical);
        super::super::gta6_relayout::write(&layout, &tree).expect("laid out anew")
    }

    fn table_block(model: &RbmModel) -> &RbmDataBlock {
        (model.data_blocks.iter())
            .find(|b| b.data_type == PAIR)
            .expect("the table's block")
    }

    fn start(block: &RbmDataBlock) -> usize {
        (block.data_addr & 0x0FFF_FFFF) as usize
    }

    /// Where the block table's record of `block` lies in the virtual section `v`.
    fn record_of(v: &[u8], block: &RbmDataBlock) -> usize {
        let record = [
            block.data_type.to_le_bytes().as_slice(),
            &(block.data.len() as u32).to_le_bytes(),
            &block.data_addr.to_le_bytes(),
        ]
        .concat();
        (v.windows(16))
            .position(|w| w == record)
            .expect("the block's record")
    }

    /// The slot or group count the root's map states in the sections `s`.
    fn stated_units(s: &(Vec<u8>, Vec<u8>)) -> u32 {
        let model = read_model(&s.0, &s.1).expect("it reads");
        let root = model.block(model.root_block_id as i32).expect("the root");
        u32::from_le_bytes(root.data[..4].try_into().expect("four bytes"))
    }

    #[test]
    fn a_pair_added_last_takes_the_slot_after_its_home_and_one_removed_leaves_the_others() {
        let (v, p) = canonical();
        let (mut tree, layout) = opened(&v, &p);
        assert_eq!(held(&mut tree), [(KEYS[0], 50), (KEYS[1], 60)]);
        assert!(!pairs_changed(&layout, &tree).expect("unchanged"));

        let key = Value::Hash(HashedString::from_hash(KEYS[2]));
        pairs(&mut tree).push((key, Value::UInt(70)));
        assert!(pairs_changed(&layout, &tree).expect("added"));
        let (v2, p2) = write_grown(&layout, &tree).expect("laid out anew");
        let (mut again, again_layout) = opened(&v2, &p2);
        assert_eq!(
            held(&mut again),
            [(KEYS[0], 50), (KEYS[1], 60), (KEYS[2], 70)]
        );
        let saved = write_grown(&again_layout, &again).expect("saved again");
        assert_eq!(saved, (v2.clone(), p2.clone()));
        // Home slot 5 is taken, so the probe goes on to slot 6.
        let model = read_model(&v2, &p2).expect("it reads");
        let table = table_block(&model);
        assert_eq!(table.data[8 * 6..8 * 6 + 4], KEYS[2].to_le_bytes());

        let (mut tree, layout) = opened(&v, &p);
        pairs(&mut tree).remove(0);
        let (v3, p3) = write_grown(&layout, &tree).expect("laid out anew");
        let (mut again, _) = opened(&v3, &p3);
        assert_eq!(held(&mut again), [(KEYS[1], 60)]);
    }

    #[test]
    fn the_gate_takes_a_re_lay_whose_blocks_differ_only_inside_a_table_wherever_they_lie() {
        let (v, p) = canonical();
        let (_, layout) = opened(&v, &p);
        let model = read_model(&v, &p).expect("it reads");
        let table = table_block(&model);
        let root = model.block(model.root_block_id as i32).expect("the root");
        let (slots, root) = (start(table), start(root));
        // Two slots of the table traded, as a rebuild that moves a pair does.
        let mut moved = v.clone();
        let (a, b) = (slots + 8 * 5, slots + 8 * 19);
        let first = moved[a..a + 8].to_vec();
        moved.copy_within(b..b + 8, a);
        moved[b..b + 8].copy_from_slice(&first);
        assert_ne!(moved, v);
        assert!(only_tables_differ(&layout.model, &(moved, p.clone())));
        // A longer section holds the same blocks.
        let mut longer = v.clone();
        longer.extend([0u8; 16]);
        assert!(only_tables_differ(&layout.model, &(longer, p.clone())));

        // More than a table differs: a byte of the root's block, a table's block of another length.
        let mut outside = v.clone();
        outside[root] ^= 1;
        assert!(!only_tables_differ(&layout.model, &(outside, p.clone())));
        let mut shorter = v.clone();
        let record = record_of(&v, table);
        let len = 8 * SLOTS as u32 - 8;
        shorter[record + 4..record + 8].copy_from_slice(&len.to_le_bytes());
        assert!(!only_tables_differ(&layout.model, &(shorter, p.clone())));
    }

    #[test]
    fn the_gate_refuses_a_slot_map_with_one_pair_dropped_and_another_doubled() {
        let (v, p) = canonical();
        let (_, layout) = opened(&v, &p);
        let model = read_model(&v, &p).expect("it reads");
        let slots = start(table_block(&model));
        let slot = |n: usize| slots + 8 * n;
        assert!(only_tables_differ(&layout.model, &(v.clone(), p.clone())));
        // The first pair also over the second: the length holds, the pairs do not.
        let mut doubled = v.clone();
        doubled.copy_within(slot(5)..slot(6), slot(19));
        assert!(!only_tables_differ(&layout.model, &(doubled, p.clone())));
        // The first pair moved to an empty slot: the same pairs in another order.
        let mut moved = v.clone();
        moved.copy_within(slot(5)..slot(6), slot(7));
        moved[slot(5)..slot(6)].fill(0);
        assert!(only_tables_differ(&layout.model, &(moved, p)));
    }

    #[test]
    fn the_gate_refuses_a_group_map_with_one_pair_dropped_and_another_doubled() {
        let s = group_map();
        let (v, p) = (&s.virtual_section, &s.physical);
        let (_, layout) = opened(v, p);
        let model = read_model(v, p).expect("it reads");
        let group = start(
            (model.data_blocks.iter())
                .find(|b| b.data_type == GROUP)
                .expect("the table's block"),
        );
        let entry = |j: usize| group + 16 + 8 * j;
        assert!(only_tables_differ(&layout.model, &(v.clone(), p.clone())));
        // The first group's first pair, control byte and entry, also over its second.
        let mut doubled = v.clone();
        doubled[group + 1] = doubled[group];
        doubled.copy_within(entry(0)..entry(1), entry(1));
        assert!(!only_tables_differ(&layout.model, &(doubled, p.clone())));
        // That pair moved to the group's empty eighth place.
        let mut moved = v.clone();
        moved[group + 7] = moved[group];
        moved[group] = EMPTY_CONTROL;
        moved.copy_within(entry(0)..entry(1), entry(7));
        moved[entry(0)..entry(1)].fill(0);
        assert!(only_tables_differ(&layout.model, &(moved, p.clone())));
    }

    #[test]
    fn a_body_whose_table_lies_elsewhere_than_a_re_lay_puts_it_still_takes_a_pair() {
        let (mut v, p) = canonical();
        let model = read_model(&v, &p).expect("it reads");
        let table = table_block(&model);
        let (record, from, len) = (record_of(&v, table), start(table), table.data.len());
        // The table's block moved past the section's end, its record pointing there.
        let to = v.len().next_multiple_of(16);
        v.resize(to + len, 0);
        v.copy_within(from..from + len, to);
        let addr = (table.data_addr & !0x0FFF_FFFF) | to as u64;
        v[record + 8..record + 16].copy_from_slice(&addr.to_le_bytes());
        let (mut tree, layout) = opened(&v, &p);
        assert_eq!(held(&mut tree), [(KEYS[0], 50), (KEYS[1], 60)]);
        let again = super::super::gta6_relayout::write(&layout, &tree).expect("laid out anew");
        assert_ne!(again.0, v, "a re-lay places the block elsewhere");

        let key = Value::Hash(HashedString::from_hash(KEYS[2]));
        pairs(&mut tree).push((key, Value::UInt(70)));
        assert!(pairs_changed(&layout, &tree).expect("the gate takes it"));
        let (v2, p2) = write_grown(&layout, &tree).expect("laid out anew");
        let (mut again, _) = opened(&v2, &p2);
        assert_eq!(
            held(&mut again),
            [(KEYS[0], 50), (KEYS[1], 60), (KEYS[2], 70)]
        );
    }

    #[test]
    fn a_changed_table_keeps_the_stored_slots_of_a_chain_that_wraps() {
        // Two keys of home slot 31, the second wrapped to slot 0: filled in slot order from slot
        // 0, they would trade places.
        let wrapped = [0x1234_001F_u32, 0x1234_003F];
        let s = slot_map_of(&[(wrapped[1], 60, 0), (wrapped[0], 50, 31)]);
        let (mut tree, layout) = opened(&s.virtual_section, &s.physical);
        let key = Value::Hash(HashedString::from_hash(KEYS[0]));
        pairs(&mut tree).push((key, Value::UInt(70)));
        let (v, p) = write_grown(&layout, &tree).expect("laid out anew");
        let model = read_model(&v, &p).expect("it reads");
        let table = &table_block(&model).data;
        let key_at = |slot: usize| table[8 * slot..8 * slot + 4].to_vec();
        assert_eq!(key_at(31), wrapped[0].to_le_bytes());
        assert_eq!(key_at(0), wrapped[1].to_le_bytes());
        assert_eq!(key_at(5), KEYS[0].to_le_bytes());
    }

    #[test]
    fn a_table_of_64_groups_holding_447_pairs_lays_out_again_at_64_groups() {
        let s = group_map();
        let (mut tree, layout) = opened(&s.virtual_section, &s.physical);
        assert_eq!(pairs(&mut tree).len(), 447);
        let again = super::super::gta6_relayout::write(&layout, &tree).expect("laid out anew");
        assert_eq!(stated_units(&again), 64, "the rule alone gives 32 groups");
        assert!(only_tables_differ(&layout.model, &again));

        // For the 446 pairs a removal leaves the rule gives 32 groups too.
        pairs(&mut tree).remove(0);
        assert!(pairs_changed(&layout, &tree).expect("the gate takes it"));
        let removed = write_grown(&layout, &tree).expect("laid out anew");
        assert_eq!(stated_units(&removed), 64);
        let (mut back, _) = opened(&removed.0, &removed.1);
        assert_eq!(pairs(&mut back).len(), 446);
    }
}
