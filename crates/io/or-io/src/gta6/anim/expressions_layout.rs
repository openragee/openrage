//! An expression set laid out anew from its tracks, so a track can be added or removed: the
//! 0x1C0-byte root, then every array and the block map packed by byte length, the counts and
//! slot numbers following from the tracks.
//!
//! ⚠ Group B's tracks come as a bone's rotation and translation together, so a bone is added or
//! removed as its pair; group D takes only a track group A reads (`group_d`).

use super::expressions::{
    read_expressions, write_expression_ids, ExpressionGroup, ExpressionTracks, Expressions,
};
use super::AnimError;

#[path = "expressions_group_d.rs"]
mod group_d;
pub use group_d::offered_facial_outputs;

/// The root: 21 slots of 16 bytes from 0x18, then zeros.
const ROOT_LEN: usize = 0x1C0;
const VIRTUAL_BASE: u64 = 0x5000_0000;
/// The bytes of a section's whole blocks; what is left over is one more block.
const PIECE: usize = 0x1_0000;
const BLOCK_MAP_ALIGN: usize = 16;
/// The slots of the two counts `{0, n1, n2, n3}` of groups A and D.
const INPUT_COUNTS: usize = 3;
const OUTPUT_COUNTS: usize = 16;
/// The slots of group C's further input slots and of their words, which follow them.
const CONTROL_SLOTS: usize = 11;
const CONTROL_WORDS: usize = 12;

fn slot_at(slot: usize) -> usize {
    0x18 + 16 * slot
}

fn unwritten(why: impl Into<String>) -> AnimError {
    AnimError::Unwritten(why.into())
}

impl ExpressionGroup {
    /// Whether a track of this group can be added or removed.
    pub fn takes_tracks(self) -> bool {
        true
    }
}

/// One array of the root's slots: its slot, element width, element count and bytes.
struct Array {
    slot: usize,
    width: usize,
    count: usize,
    bytes: Vec<u8>,
}

fn tracks(slot: usize, tracks: &[(u32, u32)]) -> Array {
    Array {
        slot,
        width: 8,
        count: tracks.len(),
        bytes: (tracks.iter())
            .flat_map(|(id, word)| [id.to_le_bytes(), word.to_le_bytes()])
            .flatten()
            .collect(),
    }
}

fn halves(slot: usize, values: &[u16]) -> Array {
    Array {
        slot,
        width: 2,
        count: values.len(),
        bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

fn words(slot: usize, values: &[u32]) -> Array {
    Array {
        slot,
        width: 4,
        count: values.len(),
        bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

fn bytes(slot: usize, values: &[u8]) -> Array {
    Array {
        slot,
        width: 1,
        count: values.len(),
        bytes: values.to_vec(),
    }
}

/// Components a byte each, as groups A, C and D state them.
fn narrow(slot: usize, values: &[u16]) -> Result<Array, AnimError> {
    let values: Vec<u8> = (values.iter())
        .map(|v| u8::try_from(*v))
        .collect::<Result<_, _>>()
        .map_err(|_| unwritten(format!("slot {slot} holds a component past a byte")))?;
    Ok(bytes(slot, &values))
}

/// Every array of `set`, by slot; an empty one included.
fn arrays(set: &Expressions) -> Result<Vec<Array>, AnimError> {
    let (a, b, c, d) = (&set.inputs, &set.bones, &set.controls, &set.facial_outputs);
    let mut out = vec![
        tracks(0, &a.tracks),
        halves(1, &a.slots),
        narrow(2, &a.components)?,
        tracks(4, &b.tracks),
        halves(5, &b.slots),
        halves(6, &b.components),
        bytes(7, &set.bone_masks),
        tracks(8, &c.tracks),
        halves(9, &c.slots),
        narrow(10, &c.components)?,
        halves(CONTROL_SLOTS, &set.control_slots),
        words(CONTROL_WORDS, &set.control_words),
        tracks(13, &d.tracks),
        halves(14, &d.slots),
        narrow(15, &d.components)?,
    ];
    if let Some(o) = &set.outputs {
        out.extend([
            tracks(17, &o.tracks),
            halves(18, &o.slots),
            halves(19, &o.components),
            bytes(20, &set.output_masks),
        ]);
    }
    Ok(out)
}

/// The blocks a section of `len` bytes is cut into.
fn blocks(len: usize) -> usize {
    usize::from(len >= PIECE) + usize::from(!len.is_multiple_of(PIECE))
}

/// `set` laid out as a virtual section beside a physical one of `physical_len` bytes.
// Every non-empty array and the block map go by byte length descending, a tie to the larger
// alignment, then the lower slot, the block map first; each starts at its element's width, the
// block map at 16. Slot 12's words follow slot 11's slots at the next 4 bytes.
pub fn lay_out_expressions(set: &Expressions, physical_len: usize) -> Result<Vec<u8>, AnimError> {
    let arrays = arrays(set)?;
    let physical = blocks(physical_len.next_multiple_of(16));
    let mut virtual_blocks = 1;
    // The map's length feeds the section's, which feeds the map's: a count only grows.
    loop {
        let out = placed(set, &arrays, [virtual_blocks, physical])?;
        let counted = blocks(out.len());
        if counted <= virtual_blocks {
            return Ok(out);
        }
        virtual_blocks = counted;
    }
}

fn placed(set: &Expressions, arrays: &[Array], counts: [usize; 2]) -> Result<Vec<u8>, AnimError> {
    let map_len = 16 + 8 * (counts[0] + counts[1]);
    let follows = |a: &Array| {
        a.slot == CONTROL_WORDS
            && arrays
                .iter()
                .any(|b| b.slot == CONTROL_SLOTS && b.count > 0)
    };
    // (length, alignment, slot), the block map's slot none.
    let mut order: Vec<(usize, usize, Option<usize>)> = (arrays.iter())
        .filter(|a| a.count > 0 && !follows(a))
        .map(|a| (a.bytes.len(), a.width, Some(a.slot)))
        .collect();
    order.push((map_len, BLOCK_MAP_ALIGN, None));
    order.sort_by(|x, y| (y.0, y.1).cmp(&(x.0, x.1)).then(x.2.cmp(&y.2)));

    let mut out = vec![0u8; ROOT_LEN];
    out[0x10..0x14].copy_from_slice(&set.key.to_le_bytes());
    out[0x14..0x18].copy_from_slice(&set.key.to_le_bytes());
    let put = |out: &mut Vec<u8>, align: usize, bytes: &[u8]| {
        let at = out.len().next_multiple_of(align);
        out.resize(at, 0);
        out.extend_from_slice(bytes);
        at
    };
    let place = |out: &mut Vec<u8>, a: &Array| -> Result<(), AnimError> {
        let at = put(out, a.width, &a.bytes);
        let count = u32::try_from(a.count)
            .map_err(|_| unwritten(format!("slot {} holds too many elements", a.slot)))?;
        let s = slot_at(a.slot);
        out[s..s + 4].copy_from_slice(&count.to_le_bytes());
        out[s + 4..s + 8].copy_from_slice(&count.to_le_bytes());
        out[s + 8..s + 16].copy_from_slice(&(VIRTUAL_BASE + at as u64).to_le_bytes());
        Ok(())
    };
    for (_, _, slot) in order {
        let Some(slot) = slot else {
            let mut map = vec![0u8; map_len];
            (map[8], map[9]) = (counts[0] as u8, counts[1] as u8);
            let at = put(&mut out, BLOCK_MAP_ALIGN, &map);
            out[8..16].copy_from_slice(&(VIRTUAL_BASE + at as u64).to_le_bytes());
            continue;
        };
        for a in arrays.iter().filter(|a| a.slot == slot) {
            place(&mut out, a)?;
        }
        if slot == CONTROL_SLOTS {
            for a in arrays.iter().filter(|a| follows(a) && a.count > 0) {
                place(&mut out, a)?;
            }
        }
    }
    for (slot, group) in [
        (INPUT_COUNTS, &set.inputs),
        (OUTPUT_COUNTS, &set.facial_outputs),
    ] {
        let by = |width: u16| group.components.iter().filter(|c| **c == width).count() as u32;
        let s = slot_at(slot);
        for (i, n) in [0, by(1), by(2), by(3)].into_iter().enumerate() {
            out[s + 4 * i..s + 4 * i + 4].copy_from_slice(&n.to_le_bytes());
        }
    }
    out.resize(out.len().next_multiple_of(16), 0);
    Ok(out)
}

/// The input slots of groups A, B and C and slot 11, which together number 0 onward once each.
fn input_slots(set: &mut Expressions) -> impl Iterator<Item = &mut u16> + '_ {
    (set.inputs.slots.iter_mut())
        .chain(set.bones.slots.iter_mut())
        .chain(set.controls.slots.iter_mut())
        .chain(set.control_slots.iter_mut())
}

/// The group's track `track`, or an error naming what is missing.
fn picked(
    set: &Expressions,
    group: ExpressionGroup,
    track: usize,
) -> Result<(u32, u32), AnimError> {
    let tracks = group
        .of(set)
        .ok_or_else(|| unwritten("the set drives no bone, so it has no such group"))?;
    tracks
        .tracks
        .get(track)
        .copied()
        .ok_or_else(|| unwritten(format!("the group has no track {track}")))
}

/// `set` with a copy of track `from` of `group`, a bone's pair in group B, under id `id`, last.
pub fn add_expression_track(
    set: &Expressions,
    group: ExpressionGroup,
    from: usize,
    id: u32,
) -> Result<Expressions, AnimError> {
    if group == ExpressionGroup::FacialOutputs {
        return group_d::add_facial_output(set, from, id);
    }
    let (_, word) = picked(set, group, from)?;
    if group == ExpressionGroup::Bones {
        return add_bone_pair(set, from, id);
    }
    if group
        .of(set)
        .is_some_and(|t| t.tracks.contains(&(id, word)))
    {
        return Err(unwritten(
            "the group already states that id on that track word",
        ));
    }
    let mut out = set.clone();
    if group == ExpressionGroup::Outputs {
        let first = set.facial_outputs.slots.len();
        let mask = *(set.output_masks.get(from)).ok_or_else(|| unwritten("no mask"))?;
        let o = out
            .outputs
            .as_mut()
            .ok_or_else(|| unwritten("no outputs"))?;
        let start = o.slots.len();
        let end = first + start + (mask & 7).count_ones() as usize;
        let numbers = (first + start..end).map(u16::try_from);
        let numbers: Vec<u16> = numbers
            .collect::<Result<_, _>>()
            .map_err(|_| unwritten("the output slots pass 65,535"))?;
        o.components
            .push(u16::try_from(start).map_err(|_| unwritten("the output slots pass 65,535"))?);
        o.slots.extend(numbers);
        o.tracks.push((id, word));
        out.output_masks.push(mask);
        return Ok(out);
    }
    let held = group.of(set).ok_or_else(|| unwritten("no group"))?;
    let component = *(held.components.get(from)).ok_or_else(|| unwritten("no component"))?;
    let taken = match group {
        ExpressionGroup::Inputs => component,
        _ => 1,
    };
    let total = set.inputs.slots.len()
        + set.bones.slots.len()
        + set.controls.slots.len()
        + set.control_slots.len();
    // The new slots follow the group's last, every slot after them moved up.
    let at = held
        .slots
        .iter()
        .max()
        .map_or(total, |s| usize::from(*s) + 1);
    let past = || unwritten("the input slots pass 65,535");
    if total.max(at) + usize::from(taken) > usize::from(u16::MAX) {
        return Err(past());
    }
    let at = at as u16;
    for slot in input_slots(&mut out) {
        if *slot >= at {
            *slot = slot.checked_add(taken).ok_or_else(past)?;
        }
    }
    let edited = group
        .of_mut(&mut out)
        .ok_or_else(|| unwritten("no group"))?;
    edited.slots.extend(at..at + taken);
    edited.components.push(component);
    edited.tracks.push((id, word));
    Ok(out)
}

/// `set` without track `track` of `group`, a bone's pair in group B, the slots after moved down.
pub fn remove_expression_track(
    set: &Expressions,
    group: ExpressionGroup,
    track: usize,
) -> Result<Expressions, AnimError> {
    if group == ExpressionGroup::FacialOutputs {
        return group_d::remove_facial_output(set, track);
    }
    picked(set, group, track)?;
    if group == ExpressionGroup::Bones {
        return remove_bone_pair(set, track);
    }
    let mut out = set.clone();
    let slots = group
        .slots(set, track)
        .ok_or_else(|| unwritten("the track's slots are not where its group states them"))?
        .to_vec();
    if group == ExpressionGroup::Outputs {
        let first = set.facial_outputs.slots.len();
        let o = out
            .outputs
            .as_mut()
            .ok_or_else(|| unwritten("no outputs"))?;
        o.tracks.remove(track);
        o.components.remove(track);
        out.output_masks.remove(track);
        let mut run = 0usize;
        for (start, mask) in o.components.iter_mut().zip(&out.output_masks) {
            *start = run as u16;
            run += (mask & 7).count_ones() as usize;
        }
        o.slots = (first..first + run).map(|s| s as u16).collect();
        if o.tracks.is_empty() {
            if !out.bones.tracks.is_empty() {
                return Err(unwritten(
                    "the set reads bones, so it drives one bone at least",
                ));
            }
            out.outputs = None;
        }
        return Ok(out);
    }
    if group == ExpressionGroup::Inputs && set.inputs.tracks.len() == 1 {
        return Err(unwritten(
            "every set seen reads one facial control at least",
        ));
    }
    let edited = group
        .of_mut(&mut out)
        .ok_or_else(|| unwritten("no group"))?;
    let start = match group {
        ExpressionGroup::Inputs => (edited.components[..track].iter())
            .map(|c| usize::from(*c))
            .sum(),
        _ => track,
    };
    edited.slots.drain(start..start + slots.len());
    edited.components.remove(track);
    edited.tracks.remove(track);
    for slot in input_slots(&mut out) {
        let below = slots.iter().filter(|s| **s < *slot).count() as u16;
        *slot -= below;
    }
    Ok(out)
}

/// The tracks of group B that read the bone of track `track`: its rotation and its translation.
fn bone_pair(set: &Expressions, track: usize) -> Result<[usize; 2], AnimError> {
    let b = &set.bones;
    if b.slots.len() != 3 * b.tracks.len() || set.bone_masks.len() != b.tracks.len() {
        return Err(unwritten(
            "the bones it reads are not three slots and a mask a track",
        ));
    }
    let (id, _) = picked(set, ExpressionGroup::Bones, track)?;
    let pair: Vec<usize> = (b.tracks.iter().enumerate())
        .filter(|(_, t)| t.0 == id)
        .map(|(k, _)| k)
        .collect();
    <[usize; 2]>::try_from(pair).map_err(|pair| {
        unwritten(format!(
            "the bone of track {track} is read by {} track(s), where a bone read is a rotation and a translation",
            pair.len()
        ))
    })
}

/// `set` with a copy of the bone pair track `from` belongs to, reading bone `id`: its six slots
/// after group B's last, every input slot after them moved up.
// Both stored places of group B keep: A's slots before B's, or A's last slot after B's.
fn add_bone_pair(set: &Expressions, from: usize, id: u32) -> Result<Expressions, AnimError> {
    let pair = bone_pair(set, from)?;
    if set.bones.tracks.iter().any(|t| t.0 == id) {
        return Err(unwritten("the set already reads that bone"));
    }
    let past = || unwritten("the input slots pass 65,535");
    let last = (set.bones.slots.iter().max())
        .ok_or_else(|| unwritten("the bones it reads hold no slot"))?;
    let at = last.checked_add(1).ok_or_else(past)?;
    let total = set.inputs.slots.len()
        + set.bones.slots.len()
        + set.controls.slots.len()
        + set.control_slots.len();
    if total.max(usize::from(at)) + 6 > usize::from(u16::MAX) {
        return Err(past());
    }
    let mut out = set.clone();
    for slot in input_slots(&mut out) {
        if *slot >= at {
            *slot = slot.checked_add(6).ok_or_else(past)?;
        }
    }
    for (k, held) in pair.into_iter().enumerate() {
        let start = u16::try_from(out.bones.slots.len()).map_err(|_| past())?;
        let first = at + 3 * k as u16;
        out.bones.tracks.push((id, set.bones.tracks[held].1));
        out.bones.components.push(start);
        out.bones.slots.extend(first..first + 3);
        out.bone_masks.push(set.bone_masks[held]);
    }
    Ok(out)
}

/// `set` without the bone pair track `track` belongs to, the input slots after its six moved down.
fn remove_bone_pair(set: &Expressions, track: usize) -> Result<Expressions, AnimError> {
    let pair = bone_pair(set, track)?;
    let mut bones = ExpressionTracks {
        tracks: Vec::new(),
        slots: Vec::new(),
        components: Vec::new(),
    };
    let (mut masks, mut gone) = (Vec::new(), Vec::new());
    for k in 0..set.bones.tracks.len() {
        let slots = (ExpressionGroup::Bones.slots(set, k))
            .ok_or_else(|| unwritten("the track's slots are not where its group states them"))?;
        if pair.contains(&k) {
            gone.extend_from_slice(slots);
            continue;
        }
        let start = u16::try_from(bones.slots.len())
            .map_err(|_| unwritten("the input slots pass 65,535"))?;
        bones.components.push(start);
        bones.slots.extend_from_slice(slots);
        bones.tracks.push(set.bones.tracks[k]);
        masks.push(set.bone_masks[k]);
    }
    let mut out = set.clone();
    out.bones = bones;
    out.bone_masks = masks;
    for slot in input_slots(&mut out) {
        let below = gone.iter().filter(|s| **s < *slot).count() as u16;
        *slot = slot.saturating_sub(below);
    }
    Ok(out)
}

/// `set` with every track's id zero: what an edit of ids alone leaves as it was.
fn without_ids(set: &Expressions) -> Expressions {
    let mut set = set.clone();
    for group in ExpressionGroup::ALL {
        if let Some(tracks) = group.of_mut(&mut set) {
            tracks.tracks.iter_mut().for_each(|(id, _)| *id = 0);
        }
    }
    set
}

/// `set` written over the set `section` holds: ids in place, else the whole set laid out anew.
// Ids are written in place only where the set differs from the stored one in ids alone, so an
// edit that keeps every group's count of tracks but changes a shape is laid out anew. A stored set
// that does not lay out again to its own bytes is refused a new layout: where its arrays go
// follows no rule known.
pub fn write_edited_expressions(
    section: &mut Vec<u8>,
    physical_len: usize,
    set: &Expressions,
) -> Result<(), AnimError> {
    let stored = read_expressions(section)?;
    if without_ids(&stored) == without_ids(set) {
        return write_expression_ids(section, set);
    }
    if lay_out_expressions(&stored, physical_len)? != *section {
        return Err(unwritten(
            "the stored set's arrays do not lie where a layout puts them, so where a track goes is not known",
        ));
    }
    let out = lay_out_expressions(set, physical_len)?;
    if read_expressions(&out)? != *set {
        return Err(unwritten("the set laid out does not read back as edited"));
    }
    *section = out;
    Ok(())
}

#[cfg(test)]
#[path = "expressions_layout_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "expressions_shape_tests.rs"]
mod shape_tests;
