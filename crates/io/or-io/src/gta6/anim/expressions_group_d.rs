//! Group D's tracks added and removed. Its slots number 0 onward by track, group O's follow
//! them, and each track it holds is one of group A's, in A's order.
//!
//! ⚠ Only a facial translation of 1 or 2 components that group A reads is known in group D.

use super::super::expressions::{ExpressionGroup, Expressions};
use super::super::AnimError;
use super::{picked, unwritten};
use crate::gta6::hash::name_hash32;

/// The one track word group D is known to write.
const FACIAL_TRANSLATION: u32 = name_hash32(b"facialTranslation");

/// Group A's track that `track` is, and its components, where group D is known to take it.
fn read_as(set: &Expressions, track: (u32, u32)) -> Result<(usize, u16), AnimError> {
    let k = (set.inputs.tracks.iter().position(|t| *t == track)).ok_or_else(|| {
        unwritten(
            "a facial control the set writes and does not read is not known to be added or removed",
        )
    })?;
    let c = *(set.inputs.components.get(k))
        .ok_or_else(|| unwritten("the facial control it reads states no components"))?;
    if track.1 != FACIAL_TRANSLATION || !(1..=2).contains(&c) {
        return Err(unwritten(
            "a facial control the set writes is known only as a facial translation of 1 or 2 components",
        ));
    }
    Ok((k, c))
}

/// The ids of group A's tracks group D may take: those it is known to take and does not hold.
pub fn offered_facial_outputs(set: &Expressions) -> Vec<u32> {
    (set.inputs.tracks.iter())
        .filter(|t| !set.facial_outputs.tracks.contains(t) && read_as(set, **t).is_ok())
        .map(|t| t.0)
        .collect()
}

/// `set` with D's slots 0 .. `len` and every slot of group O moved by `by`.
fn renumbered(
    mut set: Expressions,
    len: usize,
    by: impl Fn(u16) -> Option<u16>,
) -> Result<Expressions, AnimError> {
    let past = || unwritten("the output slots pass 65,535");
    let outputs = set.outputs.as_ref().map_or(0, |o| o.slots.len());
    if len + outputs > usize::from(u16::MAX) {
        return Err(past());
    }
    set.facial_outputs.slots = (0..len as u16).collect();
    if let Some(o) = set.outputs.as_mut() {
        for slot in &mut o.slots {
            *slot = by(*slot).ok_or_else(past)?;
        }
    }
    Ok(set)
}

/// `set` writing group A's track `id` on the word of D's track `from` as well, or as a facial
/// translation where D is empty and `from` is 0, in A's order: D's slots numbered anew and every
/// slot of group O moved up by its components.
pub(super) fn add_facial_output(
    set: &Expressions,
    from: usize,
    id: u32,
) -> Result<Expressions, AnimError> {
    let d = &set.facial_outputs;
    let word = match d.tracks.is_empty() && from == 0 {
        true => FACIAL_TRANSLATION,
        false => picked(set, ExpressionGroup::FacialOutputs, from)?.1,
    };
    if d.tracks.contains(&(id, word)) {
        return Err(unwritten(
            "the group already states that id on that track word",
        ));
    }
    let (k, c) = read_as(set, (id, word))?;
    let held: Option<Vec<usize>> = (d.tracks.iter())
        .map(|t| set.inputs.tracks.iter().position(|a| a == t))
        .collect();
    let held = (held.filter(|h| h.windows(2).all(|w| w[0] < w[1]))).ok_or_else(|| {
        unwritten("the facial controls the set writes are not ones it reads, in the order it reads them, so where another goes is not known")
    })?;
    let at = held.partition_point(|h| *h < k);
    let len = d.slots.len() + usize::from(c);
    let mut out = set.clone();
    out.facial_outputs.tracks.insert(at, (id, word));
    out.facial_outputs.components.insert(at, c);
    renumbered(out, len, |s| s.checked_add(c))
}

/// `set` without D's track `track`: D's slots numbered anew and every slot of group O moved down
/// by its components; an emptied group D is the form of a set that writes none.
pub(super) fn remove_facial_output(
    set: &Expressions,
    track: usize,
) -> Result<Expressions, AnimError> {
    let held = picked(set, ExpressionGroup::FacialOutputs, track)?;
    let d = &set.facial_outputs;
    let c = *(d.components.get(track)).ok_or_else(|| unwritten("no component"))?;
    if read_as(set, held)?.1 != c {
        return Err(unwritten(
            "the facial control the set writes states other components than the one it reads",
        ));
    }
    let len = (d.slots.len().checked_sub(usize::from(c)))
        .ok_or_else(|| unwritten("its facial outputs hold fewer slots than components"))?;
    let mut out = set.clone();
    out.facial_outputs.tracks.remove(track);
    out.facial_outputs.components.remove(track);
    renumbered(out, len, |s| s.checked_sub(c))
}

#[cfg(test)]
#[path = "expressions_group_d_tests.rs"]
mod tests;
