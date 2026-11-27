//! What a map body states that the index keeps and the map readers do not give: the group a
//! script-managed map is switched with, the hours a time archetype is on, and the kind of each
//! archetype its load flags say.
//!
//! ⚠ Each member is checked against the body's own class table first, as the map readers check
//! theirs: a class that puts it elsewhere would read other bytes.

use crate::gta6::map;
use crate::hash::literal_string_hash as lit;
use crate::meta::binary::rbm::schema::{RbmDataBlock, RbmModel};

/// A member read here: its name's hash, its offset and its type code.
type Member = (u32, u32, u8);

const TIME_ARCHETYPE: u32 = lit(b"CTimeArchetypeDef");
/// The hours a time archetype is on, one bit an hour from bit 0.
const TIME_FLAGS: Member = (lit(b"timeFlags"), 0xB8, 0x15);
/// The 32-bit name of the group a script-managed map is switched with.
const GROUP: Member = (0x4C0A_CB96, 0x7C, 0x4A);
/// The same name as the 64-bit word the world-state data names the group by.
const GROUP_WORD: Member = (0x6587_6A4D, 0x48, 0x4F);
/// An archetype's load flags, a `u64` of which the bits read lie in the low half.
const LOAD_FLAGS: Member = (lit(b"loadFlags"), 0x30, 0x17);
/// Where a map-types body's array of archetype pointers lies in its root.
const ARCHETYPES: usize = 0x18;

/// Every hour of the day: what an archetype that is no time archetype is on.
pub const ALL_HOURS: u32 = 0x00FF_FFFF;

fn states(model: &RbmModel, class: u32, (member, offset, code): Member) -> bool {
    model.classes.get(&class).is_some_and(|class| {
        (class.entries.iter())
            .any(|e| e.name_hash == member && e.offset == offset && e.data_type.to_byte() == code)
    })
}

fn bytes<const N: usize>(from: &[u8], at: usize) -> Option<[u8; N]> {
    from.get(at..at.checked_add(N)?)?.try_into().ok()
}

/// The block a pointer inside data names, and the offset in it: the block's 1-based id in bits 0
/// to 11, bits 12 to 19 clear, the offset from bit 20.
fn target(blocks: &[RbmDataBlock], raw: u64) -> Option<(&RbmDataBlock, usize)> {
    if (raw >> 12) & 0xFF != 0 {
        return None;
    }
    let block = usize::try_from(raw & 0xFFF).ok()?.checked_sub(1)?;
    Some((blocks.get(block)?, usize::try_from(raw >> 20).ok()?))
}

fn root(blocks: &[RbmDataBlock], id: u32) -> Option<&[u8]> {
    let id = usize::try_from(id).ok()?.checked_sub(1)?;
    Some(&blocks.get(id)?.data)
}

/// The name of the group the map of `model` is switched with, 0 where it states none; `None` where the body's class table does not put the member where every measured body has it.
pub fn group_of(model: &RbmModel) -> Option<u32> {
    if !states(model, map::C_MAP_DATA, GROUP) {
        return None;
    }
    group_in(&model.data_blocks, model.root_block_id)
}

fn group_in(blocks: &[RbmDataBlock], root_id: u32) -> Option<u32> {
    bytes(root(blocks, root_id)?, GROUP.1 as usize).map(u32::from_le_bytes)
}

/// The 64-bit word of the group the map of `model` is switched with, 0 where it states none; `None` where the class table puts it elsewhere.
pub fn word_of(model: &RbmModel) -> Option<u64> {
    if !states(model, map::C_MAP_DATA, GROUP_WORD) {
        return None;
    }
    word_in(&model.data_blocks, model.root_block_id)
}

fn word_in(blocks: &[RbmDataBlock], root_id: u32) -> Option<u64> {
    bytes(root(blocks, root_id)?, GROUP_WORD.1 as usize).map(u64::from_le_bytes)
}

/// Each archetype record of a map-types body's root, in the order it states them: its block and offset.
fn records(blocks: &[RbmDataBlock], root_id: u32) -> Option<Vec<(&RbmDataBlock, usize)>> {
    let root = root(blocks, root_id)?;
    let raw = u64::from_le_bytes(bytes(root, ARCHETYPES)?);
    let count = usize::from(u16::from_le_bytes(bytes(root, ARCHETYPES + 8)?));
    if count == 0 {
        return Some(Vec::new());
    }
    let (list, from) = target(blocks, raw)?;
    (0..count)
        .map(|k| target(blocks, u64::from_le_bytes(bytes(&list.data, from + 8 * k)?)))
        .collect()
}

/// The hours each archetype of the map-types body `model` is on, in the order the body states them: every hour for one that is no time archetype; `None` where the body does not read.
pub fn hours_of(model: &RbmModel) -> Option<Vec<u32>> {
    let timed = states(model, TIME_ARCHETYPE, TIME_FLAGS);
    hours_in(&model.data_blocks, model.root_block_id, timed)
}

/// The same from the blocks, `timed` saying whether the class table puts the hours where they are read.
fn hours_in(blocks: &[RbmDataBlock], root_id: u32, timed: bool) -> Option<Vec<u32>> {
    (records(blocks, root_id)?.into_iter())
        .map(|(block, at)| {
            if block.data_type != TIME_ARCHETYPE {
                return Some(ALL_HOURS);
            }
            let flags = bytes(&block.data, at + TIME_FLAGS.1 as usize)?;
            timed.then_some(u32::from_le_bytes(flags) & ALL_HOURS)
        })
        .collect()
}

/// The low half of each archetype's load flags of the map-types body `model`, in the order it states them: 0 for an archetype whose class puts them elsewhere; `None` where the body does not read.
pub fn load_flags_of(model: &RbmModel) -> Option<Vec<u32>> {
    load_flags_in(&model.data_blocks, model.root_block_id, &|class| {
        states(model, class, LOAD_FLAGS)
    })
}

/// The same from the blocks, `stated` saying whether a class's table puts the flags where they are read.
fn load_flags_in(
    blocks: &[RbmDataBlock],
    root_id: u32,
    stated: &dyn Fn(u32) -> bool,
) -> Option<Vec<u32>> {
    (records(blocks, root_id)?.into_iter())
        .map(|(block, at)| match stated(block.data_type) {
            true => bytes(&block.data, at + LOAD_FLAGS.1 as usize).map(u32::from_le_bytes),
            false => Some(0),
        })
        .collect()
}

#[cfg(test)]
#[path = "map_index_types_tests.rs"]
mod tests;
