//! A frame filter's tracks or an animation record's bones written again where they change: each
//! element is built from its named values over the element at its place, or over the one it
//! copies, or over zero bytes, and the array grows or shrinks at its end through the section's
//! growth, which moves every pointer past it; an entry that held none takes an array at the
//! section's end.
//!
//! A changed filter takes the signature at 0x0C its elements give, 0 when it holds none; a
//! record's word at 0x68 follows from no rule anyone found and is kept as read.

use super::bytes::{sections, u32_at, Bytes};
use super::dictionary::entry_of;
use super::{BoneRecord, FilterTrack, HeaderlessError};
use crate::container::rpf::{grow_section, Rpf9GrowError, Rpf9Section, Rpf9Sections};

/// Where the virtual section is addressed from.
const VIRTUAL_BASE: u64 = 0x5000_0000;
/// A filter's signature word.
const SIGNATURE: usize = 0x0C;

/// Why an entry's elements are not written.
#[derive(Debug, thiserror::Error)]
pub enum ElementsError {
    #[error(transparent)]
    Read(#[from] HeaderlessError),
    #[error("the entry's elements do not move: {0}")]
    Grow(#[from] Rpf9GrowError),
    #[error("the entry's elements are not written: {0}")]
    Refused(&'static str),
}

/// Where an entry's elements are stated.
struct Layout {
    what: &'static str,
    /// The entry's pointer to its first element.
    pointer: usize,
    /// The entry's count, then its capacity: two u32 when `wide`, else two u16.
    count: usize,
    wide: bool,
    stride: usize,
}

const FILTER: Layout = Layout {
    what: "frame filter tracks",
    pointer: 0x28,
    count: 0x30,
    wide: false,
    stride: 48,
};

const RECORD: Layout = Layout {
    what: "animation record bones",
    pointer: 0x20,
    count: 0x18,
    wide: true,
    stride: 144,
};

/// What [`read_entry`] finds of an entry.
struct Read {
    first: Option<usize>,
    elements: Vec<Vec<u8>>,
    pointers: Vec<usize>,
}

/// The element each of `count` edited ones is built over: the one at its place where the number
/// is unchanged, else the first `same` names that no earlier one took, else none.
fn bases(read: &Read, count: usize, same: impl Fn(usize, &[u8]) -> bool) -> Vec<Option<Vec<u8>>> {
    if count == read.elements.len() {
        return read.elements.iter().cloned().map(Some).collect();
    }
    let mut used = vec![false; read.elements.len()];
    (0..count)
        .map(|i| {
            let held = |j: usize| same(i, read.elements[j].as_slice());
            let found = (0..used.len()).find(|&j| !used[j] && held(j));
            let found = found.or_else(|| (0..used.len()).find(|&j| held(j)))?;
            used[found] = true;
            Some(read.elements[found].clone())
        })
        .collect()
}

fn put(element: &mut [u8], at: usize, word: u32) {
    element[at..at + 4].copy_from_slice(&word.to_le_bytes());
}

/// The frame filter of `code` given `tracks`, in any number and order.
pub fn write_filter_tracks(
    sections: &mut Rpf9Sections,
    code: u32,
    tracks: &[FilterTrack],
) -> Result<(), ElementsError> {
    let read = read_entry(&sections.virtual_section, code, &FILTER)?;
    let named = |e: &[u8]| (u32_at(e, 0), u32_at(e, 0x28), u32_at(e, 0x2C));
    let bases = bases(&read, tracks.len(), |i, e| {
        named(e) == (tracks[i].track, tracks[i].group, tracks[i].word_2c)
    });
    let mut bytes = Vec::with_capacity(FILTER.stride * tracks.len());
    for (t, base) in tracks.iter().zip(bases) {
        if !t.weight.is_finite() {
            return Err(ElementsError::Refused(
                "a track's weight is a finite number",
            ));
        }
        if t.word_2c > 2 && base.as_deref().is_none_or(|b| u32_at(b, 0x2C) != t.word_2c) {
            return Err(ElementsError::Refused(
                "a track's mode at 0x2C is 0, 1 or 2, the only ones any filter holds",
            ));
        }
        // An element is its id, weight, a name of zero bytes or inline, 16 zero bytes, its
        // track and its mode.
        let mut element = base.unwrap_or_else(|| vec![0; FILTER.stride]);
        put(&mut element, 0, t.track);
        put(&mut element, 0x04, t.weight.to_bits());
        put(&mut element, 0x28, t.group);
        put(&mut element, 0x2C, t.word_2c);
        bytes.extend(element);
    }
    let changed = bytes != read.elements.concat();
    relay(sections, code, &FILTER, read, tracks.len(), &bytes)?;
    if changed {
        sign(&mut sections.virtual_section, code)?;
    }
    Ok(())
}

/// The frame filter of `code` given the signature at 0x0C its elements give.
pub(super) fn sign(virtual_section: &mut [u8], code: u32) -> Result<(), HeaderlessError> {
    let refuse = HeaderlessError::NotInPlace("no entry of the dictionary has that code");
    let (at, signature) = sections(virtual_section, None, |v, _| {
        let at = entry_of(v, code)?.ok_or(refuse)?;
        Ok((at, filter_signature(v, at)?))
    })?;
    virtual_section[at + SIGNATURE..at + SIGNATURE + 4].copy_from_slice(&signature.to_le_bytes());
    Ok(())
}

/// The signature the elements of the frame filter at `at` give: one CRC-32 over them all.
// Per element in stored order: its first and second name where not empty, its weight, its id, the
// low byte of its mode and its track. The file holds it without the XOR with 0x55F5B477 the game
// applies at load, so a filter of no element holds 0.
fn filter_signature(v: Bytes, at: usize) -> Result<u32, HeaderlessError> {
    let count = usize::from(v.u16(FILTER.what, at + FILTER.count)?);
    if count == 0 {
        return Ok(0);
    }
    let first = v.to(FILTER.what, at + FILTER.pointer)?;
    let mut crc = flate2::Crc::new();
    for e in (0..count).map(|i| first + FILTER.stride * i) {
        let element = v.get(FILTER.what, e, FILTER.stride)?;
        crc.update(element_name(v, e + 0x08)?);
        crc.update(element_name(v, e + 0x18)?);
        for field in [0x04..0x08, 0x00..0x04, 0x2C..0x2D, 0x28..0x2C] {
            crc.update(&element[field]);
        }
    }
    Ok(crc.sum())
}

/// The text of an element's 16-byte name at `at`: inline, its length in byte 15 with the top bit
/// set, or a pointer and a u16 length.
fn element_name<'a>(v: Bytes<'a>, at: usize) -> Result<&'a [u8], HeaderlessError> {
    let what = "frame filter track name";
    let field = v.get(what, at, 16)?;
    if field[15] & 0x80 != 0 {
        let len = usize::from(field[15] & 0x7F);
        if len > 15 {
            return Err(HeaderlessError::Constant {
                what: "frame filter track name's inline length, which 15 bytes bound",
                at: at as u64 + 15,
                found: len as u64,
            });
        }
        return Ok(&field[..len]);
    }
    match usize::from(v.u16(what, at + 8)?) {
        0 => Ok(&[]),
        len => v.get(what, v.to(what, at)?, len),
    }
}

/// Each frame filter of the dictionary `virtual_section` holds: its code, the signature it states
/// and the one its elements give.
#[cfg(test)]
pub(super) fn filter_signatures(
    virtual_section: &[u8],
) -> Result<Vec<(u32, u32, u32)>, HeaderlessError> {
    sections(virtual_section, None, |v, _| {
        let count = usize::from(v.u16("code count", 0x28)?);
        let mut filters = Vec::new();
        if count == 0 {
            return Ok(filters);
        }
        let (codes, entries) = (v.to("codes", 0x20)?, v.to("entries", 0x30)?);
        for i in 0..count {
            let at = v.to("entry", entries + 8 * i)?;
            if is_filter(v, at) {
                let code = v.u32("code", codes + 4 * i)?;
                let stored = v.u32("frame filter", at + SIGNATURE)?;
                filters.push((code, stored, filter_signature(v, at)?));
            }
        }
        Ok(filters)
    })
}

/// The animation record of `code` given `bones`, in any number and order.
pub fn write_record_bones(
    sections: &mut Rpf9Sections,
    code: u32,
    bones: &[BoneRecord],
) -> Result<(), ElementsError> {
    if bones.is_empty() {
        return Err(ElementsError::Refused(
            "a record keeps a bone: a null array reads as another kind of entry, and no record was found empty",
        ));
    }
    let read = read_entry(&sections.virtual_section, code, &RECORD)?;
    let named = |e: &[u8]| (u32_at(e, 0x80), u32_at(e, 0x78), u32_at(e, 0x7C));
    let bases = bases(&read, bones.len(), |i, e| {
        named(e) == (bones[i].bone, bones[i].kind, bones[i].flags)
    });
    let mut bytes = Vec::with_capacity(RECORD.stride * bones.len());
    for (b, base) in bones.iter().zip(bases) {
        // An element is 12 zero bytes, its 27 values, its kind, flags and bone, 12 zero bytes.
        let mut element = base.unwrap_or_else(|| vec![0; RECORD.stride]);
        for (f, value) in b.values.iter().enumerate() {
            put(&mut element, 0x0C + 4 * f, value.to_bits());
        }
        put(&mut element, 0x78, b.kind);
        put(&mut element, 0x7C, b.flags);
        put(&mut element, 0x80, b.bone);
        bytes.extend(element);
    }
    relay(sections, code, &RECORD, read, bones.len(), &bytes)
}

pub(super) fn is_filter(v: Bytes, at: usize) -> bool {
    let half = |o| v.u32("frame filter", at + o).ok();
    v.u64("frame filter", at).ok() == Some(0)
        && half(0x08) == Some(1)
        && half(0x10) == Some(14)
        && v.ptr("frame filter name", at + 0x18)
            .ok()
            .flatten()
            .is_some()
}

pub(super) fn is_record(v: Bytes, at: usize) -> bool {
    let word = |o| v.u64("animation record", at + o).ok();
    word(0) == Some(0)
        && word(0x08) == Some(0)
        && v.u32("animation record", at + 0x10).ok() == Some(1)
        && v.ptr("animation record bones", at + 0x20)
            .ok()
            .flatten()
            .is_some()
}

/// The entry of `code` of the kind `layout` states, its elements as read, and every word of the
/// dictionary its readers name a pointer (the root's, each entry's, and each frame filter's and
/// animation record's).
fn read_entry(virtual_section: &[u8], code: u32, layout: &Layout) -> Result<Read, ElementsError> {
    let read = sections(virtual_section, None, |v, _| {
        let refuse = HeaderlessError::NotInPlace;
        let at = entry_of(v, code)?.ok_or(refuse("no entry of the dictionary has that code"))?;
        let kind = match layout.wide {
            true => is_record(v, at),
            false => is_filter(v, at),
        };
        if !kind {
            return Err(refuse("the entry of that code is not of the kind edited"));
        }
        let count = match layout.wide {
            true => v.u32(layout.what, at + layout.count)? as usize,
            false => usize::from(v.u16(layout.what, at + layout.count)?),
        };
        let (first, elements) = match count {
            0 => (None, Vec::new()),
            _ => {
                let first = v.to(layout.what, at + layout.pointer)?;
                let all = v.array(layout.what, first, count, layout.stride)?;
                (
                    Some(first),
                    all.chunks(layout.stride).map(<[u8]>::to_vec).collect(),
                )
            }
        };
        let mut pointers = vec![0x08, 0x10, 0x20, 0x30];
        if let Some(entries) = v.ptr("entries", 0x30)? {
            for i in 0..usize::from(v.u16("code count", 0x28)?) {
                let slot = entries + 8 * i;
                pointers.push(slot);
                let entry = v.to("entry", slot)?;
                if is_filter(v, entry) {
                    pointers.extend([entry + 0x18, entry + 0x28, entry + 0x38]);
                } else if is_record(v, entry) {
                    pointers.push(entry + 0x20);
                    // A name of 15 bytes or fewer lies inline, its length in its last byte.
                    if v.u8("animation record name", entry + 0x67)? & 0x80 == 0 {
                        pointers.push(entry + 0x58);
                    }
                }
            }
        }
        Ok(Read {
            first,
            elements,
            pointers,
        })
    })?;
    Ok(read)
}

/// `bytes`, `count` elements, written over the entry of `code` that `read` read, its array grown or
/// shrunk at its end, or placed at the section's end where it held none, and its count and
/// capacity set; an entry left with none holds a null array, as every empty entry read does.
fn relay(
    body: &mut Rpf9Sections,
    code: u32,
    layout: &Layout,
    read: Read,
    count: usize,
    bytes: &[u8],
) -> Result<(), ElementsError> {
    let limit = if layout.wide {
        u32::MAX as usize
    } else {
        usize::from(u16::MAX)
    };
    if count > limit {
        return Err(ElementsError::Refused(
            "the entry would hold more elements than its count states",
        ));
    }
    let (old, new) = (layout.stride * read.elements.len(), bytes.len());
    let placed = match read.first {
        Some(first) if new != old => {
            if new == 0 {
                let at = entry_at(&body.virtual_section, code)? + layout.pointer;
                body.virtual_section[at..at + 8].fill(0);
            }
            let edge = first + old.min(new);
            let by = new as isize - old as isize;
            grow_section(body, &read.pointers, Rpf9Section::Virtual, edge, by)?;
            None
        }
        None if new > 0 => {
            // Every element's stride is a whole number of 16 bytes, as the growth keeps them.
            let end = body.virtual_section.len().next_multiple_of(16);
            body.virtual_section.resize(end, 0);
            let by = new as isize;
            Some(grow_section(
                body,
                &read.pointers,
                Rpf9Section::Virtual,
                end,
                by,
            )?)
        }
        _ => None,
    };
    let at = entry_at(&body.virtual_section, code)?;
    let v = &mut body.virtual_section;
    if let Some(first) = placed {
        let pointer = VIRTUAL_BASE + first as u64;
        v[at + layout.pointer..at + layout.pointer + 8].copy_from_slice(&pointer.to_le_bytes());
    }
    match layout.wide {
        true => {
            let n = (count as u32).to_le_bytes();
            v[at + layout.count..at + layout.count + 4].copy_from_slice(&n);
            v[at + layout.count + 4..at + layout.count + 8].copy_from_slice(&n);
        }
        false => {
            let n = (count as u16).to_le_bytes();
            v[at + layout.count..at + layout.count + 2].copy_from_slice(&n);
            v[at + layout.count + 2..at + layout.count + 4].copy_from_slice(&n);
        }
    }
    if new > 0 {
        let first = sections(v, None, |b, _| b.to(layout.what, at + layout.pointer))?;
        let end = (first.checked_add(new))
            .filter(|end| *end <= v.len())
            .ok_or(ElementsError::Refused(
                "the entry's elements run past the section",
            ))?;
        v[first..end].copy_from_slice(bytes);
    }
    Ok(())
}

/// Where the entry of `code` lies.
fn entry_at(virtual_section: &[u8], code: u32) -> Result<usize, ElementsError> {
    let at = sections(virtual_section, None, |v, _| entry_of(v, code))?;
    at.ok_or(ElementsError::Refused(
        "no entry of the dictionary has that code",
    ))
}
