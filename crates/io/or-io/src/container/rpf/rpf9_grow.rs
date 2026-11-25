//! A section of a block-mapped GTA VI resource grown or shrunk at the end of an allocation: every
//! pointer at or past the edit moves with the bytes, and the block map takes the slots the new
//! lengths need.
//!
//! ⚠ What a pointer is comes from a typed reader: a word that looks like one, aims at or past the
//! edit and that no reader names refuses the edit, since moving data or leaving a pointer behind
//! are both wrong.

use std::collections::HashSet;
use std::ops::Range;

use byteorder::{ByteOrder, LE};

use super::rpf9_scan::RPF9_SLOT;
use super::rpf9_write::{cuts, Rpf9Form, Rpf9Sections, SECTION_LIMIT};

/// No physical object crosses one of these.
const PHYSICAL_BLOCK: usize = 0x1_0000;
/// The offset a pointer word holds below its section nibble.
const TARGET: u64 = 0x0FFF_FFFF;

/// The section a pointer's nibble names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpf9Section {
    /// Addressed by the words `0x5000_0000 + offset`.
    Virtual,
    /// Addressed by the words `0x6000_0000 + offset`.
    Physical,
}

impl Rpf9Section {
    fn nibble(self) -> u64 {
        match self {
            Self::Virtual => 5,
            Self::Physical => 6,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Virtual => "virtual",
            Self::Physical => "physical",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Rpf9GrowError {
    #[error("a stored RSC8 has no block map and its end region stays last, so it is not grown")]
    StoredRsc8,
    #[error("{by} bytes is not a multiple of 16, which keeps every allocation after it aligned")]
    NotSixteen { by: isize },
    #[error("the {section} section is edited at {at:#X}, which is not 16-aligned")]
    NotAligned { section: &'static str, at: u64 },
    #[error("an edit of {by} bytes at {at:#X} does not lie in the {len}-byte {section} section")]
    Outside {
        section: &'static str,
        at: u64,
        by: i64,
        len: u64,
    },
    #[error(
        "the {section} section would be {len} bytes, past the {SECTION_LIMIT} bytes a pointer reaches"
    )]
    TooLong { section: &'static str, len: u64 },
    #[error("the pointer the reader names at {at:#X} lies past the virtual section")]
    PointerOutside { at: u64 },
    #[error("the word at {at:#X} the reader names a pointer holds {word:#X}, which addresses no byte of either section")]
    NotAPointer { at: u64, word: u64 },
    #[error("the {section} section's word at {at:#X} holds {word:#X}, which aims at or past the edit as a pointer would, and no reader says whether it is one")]
    LooksLikePointer {
        section: &'static str,
        at: u64,
        word: u64,
    },
    #[error("the pointer at {at:#X} lies in the bytes removed")]
    RemovesPointer { at: u64 },
    #[error("the pointer at {at:#X} aims at {target:#X}, inside the bytes removed")]
    AimsIntoRemoved { at: u64, target: u64 },
    #[error("the virtual section's word at 0x08 is no pointer to a block map inside it")]
    NoBlockMap,
    #[error("the bytes at {map:#X}, where the word at 0x08 points, are no block map, whose head holds noughts but for its two counts: the byte at {byte:#X} is none, so nothing is grown")]
    NotTheMap { map: u64, byte: u64 },
    #[error("the edit at {at:#X} falls inside the block map at {map:#X}, whose slots stay zero")]
    InsideMap { at: u64, map: u64 },
    #[error("the physical section is edited at {at:#X}, before its end at {len:#X}: what follows would be packed again into its blocks, which is not done")]
    PhysicalRepack { at: u64, len: u64 },
    #[error(
        "{by} bytes at {at:#X} would cross a 64 KiB physical block, which no physical object does"
    )]
    CrossesBlock { at: u64, by: u64 },
}

/// Inserts `by` bytes at allocation end `at`, or removes `-by`, moving every pointer with them.
// `pointers` are the virtual-section offsets of every word a typed reader reads as a pointer; a
// pointer aiming at `at` moves, since the object there moves. Returns where the edit sits after
// the block map's slots grew.
pub fn grow_section(
    sections: &mut Rpf9Sections,
    pointers: &[usize],
    section: Rpf9Section,
    at: usize,
    by: isize,
) -> Result<usize, Rpf9GrowError> {
    grow_section_with_data(sections, pointers, &[], section, at, by)
}

/// [`grow_section`], where the guard also passes over the virtual-section words inside `data`.
// `data` are spans a typed reader reads as data, such as block payloads; they move with the
// bytes as the pointers do.
pub fn grow_section_with_data(
    sections: &mut Rpf9Sections,
    pointers: &[usize],
    data: &[Range<usize>],
    section: Rpf9Section,
    at: usize,
    by: isize,
) -> Result<usize, Rpf9GrowError> {
    if sections.form != Rpf9Form::BlockMapped {
        return Err(Rpf9GrowError::StoredRsc8);
    }
    if by % 16 != 0 {
        return Err(Rpf9GrowError::NotSixteen { by });
    }
    let name = section.name();
    if !at.is_multiple_of(16) {
        return Err(Rpf9GrowError::NotAligned {
            section: name,
            at: at as u64,
        });
    }
    let len = bytes(sections, section).len();
    let removed = if by < 0 { by.unsigned_abs() } else { 0 };
    if at.checked_add(removed).is_none_or(|end| end > len) {
        return Err(Rpf9GrowError::Outside {
            section: name,
            at: at as u64,
            by: by as i64,
            len: len as u64,
        });
    }
    let mut pointers = pointers.to_vec();
    pointers.sort_unstable();
    pointers.dedup();
    check_pointers(sections, &pointers)?;
    let map = map_at(&sections.virtual_section)?;
    let v = &sections.virtual_section;
    let mut room = usize::from(v[map + 8]) + usize::from(v[map + 9]);
    let slots_end = map + 16 + 8 * room;
    // No reader reads the map, so a body whose word at 0x08 points elsewhere reads; growing it
    // would write the counts into whatever lies there. A map is told by its head, which is as
    // the writer holds one to be: noughts but for its two counts.
    // ⚠ Not by its counts, which may be behind its sections: a caller lengthens a section and
    // then asks for the growth, which is what brings them up to it. Nor by its slots: a map
    // may state more of them than it has room for before the next allocation.
    let counts_at = [map + 8, map + 9];
    if let Some(byte) = (map..map + 16).find(|at| !counts_at.contains(at) && v[*at] != 0) {
        let (map, byte) = (map as u64, byte as u64);
        return Err(Rpf9GrowError::NotTheMap { map, byte });
    }
    let inside = match by > 0 {
        true => at > map && at < slots_end,
        false => at < slots_end && at + removed > map,
    };
    if section == Rpf9Section::Virtual && inside {
        return Err(Rpf9GrowError::InsideMap {
            at: at as u64,
            map: map as u64,
        });
    }
    if section == Rpf9Section::Physical {
        if at + removed != len {
            return Err(Rpf9GrowError::PhysicalRepack {
                at: at as u64,
                len: len as u64,
            });
        }
        let last = (at + by.unsigned_abs()).saturating_sub(1);
        if by > 0 && at / PHYSICAL_BLOCK != last / PHYSICAL_BLOCK {
            return Err(Rpf9GrowError::CrossesBlock {
                at: at as u64,
                by: by as u64,
            });
        }
    }
    if by == 0 {
        return Ok(at);
    }
    let mut data = merged(data);
    let mut grown = sections.clone();
    splice(&mut grown, &mut pointers, &mut data, section, at, by)?;
    let mut start = at;
    // The map moved with the bytes when it lay past the edit; its slots grow after the old ones,
    // never into the bytes after them, where the next allocation may start.
    let map = map_at(&grown.virtual_section)?;
    loop {
        let (virtual_blocks, physical_blocks) = counts(&grown);
        if virtual_blocks + physical_blocks <= room {
            break;
        }
        let grow = (8 * (virtual_blocks + physical_blocks - room)).next_multiple_of(16);
        let slots_end = map + 16 + 8 * room;
        splice(
            &mut grown,
            &mut pointers,
            &mut data,
            Rpf9Section::Virtual,
            slots_end,
            grow as isize,
        )?;
        if section == Rpf9Section::Virtual && start >= slots_end {
            start += grow;
        }
        room += grow / 8;
    }
    let (virtual_blocks, physical_blocks) = counts(&grown);
    grown.virtual_section[map + 8] = virtual_blocks as u8;
    grown.virtual_section[map + 9] = physical_blocks as u8;
    *sections = grown;
    Ok(start)
}

fn bytes(sections: &Rpf9Sections, section: Rpf9Section) -> &[u8] {
    match section {
        Rpf9Section::Virtual => &sections.virtual_section,
        Rpf9Section::Physical => &sections.physical,
    }
}

/// The blocks each section is cut into once written.
fn counts(sections: &Rpf9Sections) -> (usize, usize) {
    let blocks = |len: usize| cuts(len.next_multiple_of(RPF9_SLOT as usize)).len();
    (
        blocks(sections.virtual_section.len()),
        blocks(sections.physical.len()),
    )
}

fn map_at(v: &[u8]) -> Result<usize, Rpf9GrowError> {
    let word = v
        .get(8..16)
        .map(LE::read_u64)
        .ok_or(Rpf9GrowError::NoBlockMap)?;
    let map = (word & TARGET) as usize;
    match word >> 28 == 5 && map + 16 <= v.len() {
        true => Ok(map),
        false => Err(Rpf9GrowError::NoBlockMap),
    }
}

fn check_pointers(sections: &Rpf9Sections, pointers: &[usize]) -> Result<(), Rpf9GrowError> {
    for &o in pointers {
        let word = (o.checked_add(8))
            .and_then(|end| sections.virtual_section.get(o..end))
            .map(LE::read_u64)
            .ok_or(Rpf9GrowError::PointerOutside { at: o as u64 })?;
        if word == 0 {
            continue;
        }
        let len = match word >> 28 {
            5 => Some(sections.virtual_section.len()),
            6 => Some(sections.physical.len()),
            _ => None,
        };
        if len.is_none_or(|len| (word & TARGET) as usize > len) {
            return Err(Rpf9GrowError::NotAPointer { at: o as u64, word });
        }
    }
    Ok(())
}

/// `data` sorted by start, overlapping or touching spans joined.
fn merged(data: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut spans: Vec<Range<usize>> = data.iter().filter(|r| !r.is_empty()).cloned().collect();
    spans.sort_unstable_by_key(|r| r.start);
    let mut out: Vec<Range<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => out.push(span),
        }
    }
    out
}

/// Whether offset `o` lies in one of the sorted, disjoint `data` spans.
fn in_data(data: &[Range<usize>], o: usize) -> bool {
    let i = data.partition_point(|r| r.start <= o);
    i > 0 && o < data[i - 1].end
}

/// Refuses an 8-aligned word of either section that no reader names, that lies in no virtual
/// `data` span and that aims at or past `at` of `section`.
fn guard(
    sections: &Rpf9Sections,
    pointers: &[usize],
    data: &[Range<usize>],
    section: Rpf9Section,
    at: usize,
) -> Result<(), Rpf9GrowError> {
    let listed: HashSet<usize> = pointers.iter().copied().collect();
    let len = bytes(sections, section).len();
    for holder in [Rpf9Section::Virtual, Rpf9Section::Physical] {
        let held = bytes(sections, holder);
        for o in (0..held.len().saturating_sub(7)).step_by(8) {
            if holder == Rpf9Section::Virtual && (listed.contains(&o) || in_data(data, o)) {
                continue;
            }
            let word = LE::read_u64(&held[o..o + 8]);
            if word >> 28 == section.nibble() && (at..=len).contains(&((word & TARGET) as usize)) {
                return Err(Rpf9GrowError::LooksLikePointer {
                    section: holder.name(),
                    at: o as u64,
                    word,
                });
            }
        }
    }
    Ok(())
}

/// One insert or removal, every pointer word moved with its bytes and every target at or past
/// `at` moved by `by`.
fn splice(
    sections: &mut Rpf9Sections,
    pointers: &mut [usize],
    data: &mut [Range<usize>],
    section: Rpf9Section,
    at: usize,
    by: isize,
) -> Result<(), Rpf9GrowError> {
    guard(sections, pointers, data, section, at)?;
    let nibble = section.nibble();
    let removed = if by < 0 { by.unsigned_abs() } else { 0 };
    let gone = at..at + removed;
    for &o in pointers.iter() {
        let word = LE::read_u64(&sections.virtual_section[o..o + 8]);
        if section == Rpf9Section::Virtual && gone.contains(&o) {
            return Err(Rpf9GrowError::RemovesPointer { at: o as u64 });
        }
        if word >> 28 == nibble && gone.contains(&((word & TARGET) as usize)) {
            return Err(Rpf9GrowError::AimsIntoRemoved {
                at: o as u64,
                target: word & TARGET,
            });
        }
    }
    let edited = match section {
        Rpf9Section::Virtual => &mut sections.virtual_section,
        Rpf9Section::Physical => &mut sections.physical,
    };
    // A block map whose slots overhang its section asks for an insert past the end.
    if at + removed > edited.len() {
        return Err(Rpf9GrowError::Outside {
            section: section.name(),
            at: at as u64,
            by: by as i64,
            len: edited.len() as u64,
        });
    }
    let len = edited.len().wrapping_add_signed(by);
    if len >= SECTION_LIMIT {
        return Err(Rpf9GrowError::TooLong {
            section: section.name(),
            len: len as u64,
        });
    }
    match by > 0 {
        true => {
            let tail = edited.split_off(at);
            edited.resize(at + by as usize, 0);
            edited.extend_from_slice(&tail);
        }
        false => {
            edited.drain(gone);
        }
    }
    if section == Rpf9Section::Virtual {
        // Monotone, so the spans stay sorted and disjoint; one in the bytes removed empties.
        let moved = |x: usize, past: bool| match past {
            true => x.wrapping_add_signed(by).max(at),
            false => x,
        };
        for span in data.iter_mut() {
            *span = moved(span.start, span.start >= at)..moved(span.end, span.end > at);
        }
    }
    let v = &mut sections.virtual_section;
    for o in pointers.iter_mut() {
        if section == Rpf9Section::Virtual && *o >= at {
            *o = o.wrapping_add_signed(by);
        }
        let word = LE::read_u64(&v[*o..*o + 8]);
        if word >> 28 == nibble && (word & TARGET) as usize >= at {
            LE::write_u64(&mut v[*o..*o + 8], word.wrapping_add_signed(by as i64));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::super::rpf9_scan::{read_rpf9_body, Rpf9BodyKind};
    use super::super::rpf9_write::write_rpf9_resource;
    use super::*;

    /// A virtual section of `len` bytes, no word of which addresses either section, with its block
    /// map at `map` counting `blocks`.
    fn virtual_section(len: usize, map: usize, blocks: [u8; 2]) -> Vec<u8> {
        let mut v: Vec<u8> = (0..len).map(|i| (i % 251) as u8 | 0x80).collect();
        v[..8].fill(0);
        v[map..map + 16 + 8 * usize::from(blocks[0] + blocks[1])].fill(0);
        (v[map + 8], v[map + 9]) = (blocks[0], blocks[1]);
        word(&mut v, 8, 0x5000_0000 + map as u64);
        v
    }

    fn physical(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 253) as u8 | 0x81).collect()
    }

    fn word(bytes: &mut [u8], at: usize, value: u64) {
        LE::write_u64(&mut bytes[at..at + 8], value);
    }

    fn mapped(virtual_section: Vec<u8>, physical: Vec<u8>) -> Rpf9Sections {
        Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section,
            physical,
        }
    }

    /// `bytes` with `by` zero bytes inserted at `at`.
    fn inserted(bytes: &[u8], at: usize, by: usize) -> Vec<u8> {
        [&bytes[..at], &vec![0; by], &bytes[at..]].concat()
    }

    /// What a read of `file` at offset 0 returns, and the blocks it counted.
    fn read(file: &[u8]) -> (Rpf9Sections, [u8; 2]) {
        let (find, bytes) = read_rpf9_body(Cursor::new(file), 0, u64::MAX).expect("it reads");
        let counted = match find.kind {
            Rpf9BodyKind::Zlib(run) => run.resource.map(|r| [r.virtual_blocks, r.physical_blocks]),
            _ => None,
        };
        (
            Rpf9Sections::of(&find, &bytes).expect("a resource"),
            counted.expect("a block-mapped resource"),
        )
    }

    #[test]
    fn an_insert_moves_every_pointer_at_or_past_it_and_the_words_that_follow_it() {
        let (map, at, by) = (0x1C0, 0x100, 0x30);
        let mut v = virtual_section(0x200, map, [1, 1]);
        let p = physical(0x40);
        // Below the edit, at it, past it, a word past it aiming below, a physical one and a null.
        for (o, value) in [
            (0x20, 0x5000_0040),
            (0x28, 0x5000_0100),
            (0x30, 0x5000_0180),
            (0x38, 0),
            (0x110, 0x5000_0060),
            (0x118, 0x6000_0010),
        ] {
            word(&mut v, o, value);
        }
        // A word no reader names, aiming below the edit, is left as it is.
        word(&mut v, 0x50, 0x5000_0040);
        let pointers = [0x08, 0x20, 0x28, 0x30, 0x38, 0x110, 0x118];
        let mut sections = mapped(v.clone(), p.clone());
        let start = grow_section(
            &mut sections,
            &pointers,
            Rpf9Section::Virtual,
            at,
            by as isize,
        )
        .expect("grown");
        assert_eq!(start, at);

        let mut expected = inserted(&v, at, by);
        for (o, value) in [
            (0x08, 0x5000_0000 + (map + by) as u64),
            (0x28, 0x5000_0100 + by as u64),
            (0x30, 0x5000_0180 + by as u64),
        ] {
            word(&mut expected, o, value);
        }
        assert_eq!(sections, mapped(expected, p));
        assert_eq!(
            LE::read_u64(&sections.virtual_section[0x110 + by..]),
            0x5000_0060
        );

        let file = write_rpf9_resource(&sections).expect("written");
        let (again, counted) = read(&file);
        assert_eq!(again, sections);
        assert_eq!(counted, [1, 1]);
        assert_eq!(write_rpf9_resource(&again).expect("again"), file);
    }

    #[test]
    fn a_section_past_64_kib_takes_its_new_slot_after_the_old_ones_and_moves_what_follows_them() {
        let (map, at, by) = (0x40, 0x100, 0x40);
        let mut v = virtual_section(0x1_0000 - 0x20, map, [1, 0]);
        let after_slots = map + 16 + 8;
        word(&mut v, 0x20, 0x5000_0000 + after_slots as u64);
        word(&mut v, 0x28, 0x5000_0000 + at as u64);
        let mut sections = mapped(v.clone(), Vec::new());
        let start = grow_section(
            &mut sections,
            &[0x08, 0x20, 0x28],
            Rpf9Section::Virtual,
            at,
            by as isize,
        )
        .expect("grown");

        let mut expected = inserted(&inserted(&v, at, by), after_slots, 16);
        word(&mut expected, 0x20, 0x5000_0000 + after_slots as u64 + 16);
        word(&mut expected, 0x28, 0x5000_0000 + (at + by + 16) as u64);
        expected[map + 8] = 2;
        assert_eq!(start, at + 16);
        assert_eq!(sections.virtual_section, expected);

        let file = write_rpf9_resource(&sections).expect("written");
        let (again, counted) = read(&file);
        assert_eq!(counted, [2, 0]);
        assert_eq!(again, sections);
    }

    #[test]
    fn a_removal_moves_what_follows_it_back_and_refuses_a_pointer_into_what_it_removes() {
        let (map, at, by) = (0x1C0, 0x100, 0x40);
        let mut v = virtual_section(0x200, map, [1, 0]);
        word(&mut v, 0x20, 0x5000_0000 + (at + by) as u64);
        word(&mut v, 0x28, 0x5000_0040);
        let mut sections = mapped(v.clone(), Vec::new());
        grow_section(
            &mut sections,
            &[0x08, 0x20, 0x28],
            Rpf9Section::Virtual,
            at,
            -(by as isize),
        )
        .expect("shrunk");
        let mut expected = [&v[..at], &v[at + by..]].concat();
        word(&mut expected, 0x08, 0x5000_0000 + (map - by) as u64);
        word(&mut expected, 0x20, 0x5000_0000 + at as u64);
        assert_eq!(sections.virtual_section, expected);
        let file = write_rpf9_resource(&sections).expect("written");
        assert_eq!(read(&file).0, sections);

        let mut into = v.clone();
        word(&mut into, 0x20, 0x5000_0120);
        let mut removes = v.clone();
        word(&mut removes, 0x108, 0x5000_0040);
        for (body, pointers, want) in [
            (
                into,
                vec![0x08, 0x20],
                "aims at 0x120, inside the bytes removed",
            ),
            (
                removes,
                vec![0x08, 0x20, 0x108],
                "pointer at 0x108 lies in the bytes removed",
            ),
        ] {
            let mut sections = mapped(body, Vec::new());
            let err = grow_section(
                &mut sections,
                &pointers,
                Rpf9Section::Virtual,
                at,
                -(by as isize),
            )
            .expect_err(want)
            .to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn a_physical_append_moves_the_pointers_at_the_end_and_no_virtual_one() {
        let mut v = virtual_section(0x100, 0xC0, [1, 1]);
        word(&mut v, 0x20, 0x6000_0040);
        word(&mut v, 0x28, 0x6000_0020);
        word(&mut v, 0x30, 0x5000_0040);
        let p = physical(0x40);
        let mut sections = mapped(v.clone(), p.clone());
        grow_section(
            &mut sections,
            &[0x08, 0x20, 0x28, 0x30],
            Rpf9Section::Physical,
            0x40,
            0x20,
        )
        .expect("grown");
        let mut expected = v;
        word(&mut expected, 0x20, 0x6000_0060);
        assert_eq!(sections, mapped(expected, inserted(&p, 0x40, 0x20)));
    }

    /// What growing a patched body refuses with, checked to leave the sections as they were.
    fn refused(
        patch: impl Fn(&mut Rpf9Sections),
        section: Rpf9Section,
        at: usize,
        by: isize,
    ) -> String {
        let mut v = virtual_section(0x200, 0x1C0, [1, 1]);
        word(&mut v, 0x20, 0x5000_0100);
        let mut sections = mapped(v, physical(0x40));
        patch(&mut sections);
        let kept = sections.clone();
        let err = grow_section(&mut sections, &[0x08, 0x20], section, at, by)
            .expect_err("refused")
            .to_string();
        assert_eq!(sections, kept, "{err}");
        err
    }

    #[test]
    fn an_edit_the_rule_does_not_cover_is_refused_by_name_and_changes_nothing() {
        let none = |_: &mut Rpf9Sections| ();
        let (v, p) = (Rpf9Section::Virtual, Rpf9Section::Physical);
        let cases = [
            (
                refused(none, v, 0x100, 8),
                "8 bytes is not a multiple of 16",
            ),
            (
                refused(none, v, 0x108, 16),
                "at 0x108, which is not 16-aligned",
            ),
            (
                refused(none, v, 0x210, 16),
                "not lie in the 512-byte virtual",
            ),
            (refused(none, v, 0x1D0, 16), "inside the block map at 0x1C0"),
            (refused(none, p, 0x10, 16), "before its end at 0x40"),
            (
                refused(
                    |s| word(&mut s.virtual_section, 0x48, 0x5000_0180),
                    v,
                    0x100,
                    16,
                ),
                "virtual section's word at 0x48 holds 0x50000180",
            ),
            (
                refused(|s| word(&mut s.physical, 0x18, 0x5000_0100), v, 0x100, 16),
                "physical section's word at 0x18 holds 0x50000100",
            ),
            (
                refused(
                    |s| word(&mut s.virtual_section, 0x20, 0x7000_0100),
                    v,
                    0x100,
                    16,
                ),
                "holds 0x70000100, which addresses no byte",
            ),
            (
                refused(
                    |s| s.form = Rpf9Form::StoredRsc8 { config: 0 },
                    v,
                    0x100,
                    16,
                ),
                "a stored RSC8 has no block map",
            ),
        ];
        for (err, want) in cases {
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn a_word_inside_a_data_span_is_passed_over_and_kept_where_the_slots_grow_before_it() {
        let (map, at, by) = (0x40, 0x1_0000 - 0x20, 0x40);
        let mut v = virtual_section(0x1_0000 - 0x20, map, [1, 0]);
        let after_slots = map + 16 + 8;
        let (span, data_word) = (0x200..0x240, 0x5000_0000 + at as u64);
        word(&mut v, 0x210, data_word);
        let bare = mapped(v.clone(), Vec::new());
        let mut refused = bare.clone();
        let err = grow_section(&mut refused, &[0x08], Rpf9Section::Virtual, at, by as isize)
            .expect_err("a word aims at the edit");
        assert!(
            matches!(err, Rpf9GrowError::LooksLikePointer { at: 0x210, .. }),
            "{err}"
        );
        assert_eq!(refused, bare);
        // A span that lies past the slots moves with them; one that misses the word passes nothing.
        let mut missed = bare.clone();
        let err = grow_section_with_data(
            &mut missed,
            &[0x08],
            std::slice::from_ref(&(0x218..0x240)),
            Rpf9Section::Virtual,
            at,
            by as isize,
        )
        .expect_err("the word lies before the span");
        assert!(
            matches!(err, Rpf9GrowError::LooksLikePointer { .. }),
            "{err}"
        );

        let mut sections = bare.clone();
        let start = grow_section_with_data(
            &mut sections,
            &[0x08],
            std::slice::from_ref(&span),
            Rpf9Section::Virtual,
            at,
            by as isize,
        )
        .expect("grown");
        let mut expected = inserted(&inserted(&v, at, by), after_slots, 16);
        expected[map + 8] = 2;
        assert_eq!(start, at + 16);
        assert_eq!(sections.virtual_section, expected);
        assert_eq!(
            LE::read_u64(&sections.virtual_section[span.start + 0x10 + 16..]),
            data_word
        );
    }

    #[test]
    fn data_spans_are_sorted_joined_and_found_by_offset() {
        let spans = merged(&[0x40..0x50, 0x10..0x20, 0x18..0x30, 0x60..0x60, 0x30..0x38]);
        assert_eq!(spans, vec![0x10..0x38, 0x40..0x50]);
        let found: Vec<bool> = [0x08, 0x10, 0x30, 0x38, 0x48, 0x50]
            .into_iter()
            .map(|o| in_data(&spans, o))
            .collect();
        assert_eq!(found, [false, true, true, false, true, false]);
    }

    #[test]
    fn a_word_at_8_that_points_at_no_block_map_is_refused_and_nothing_is_written_there() {
        // The map is at 0x100 and counts one block a section; the pointer is moved to 0x200,
        // where the bytes are data that happen to state one virtual block and no physical one: a
        // growth would write the physical count there.
        let mut v = virtual_section(0x400, 0x100, [1, 1]);
        word(&mut v, 8, 0x5000_0200);
        (v[0x208], v[0x209]) = (1, 0);
        let mut sections = mapped(v, physical(0x40));
        let before = sections.clone();
        let end = sections.virtual_section.len();
        let grown = grow_section(&mut sections, &[], Rpf9Section::Virtual, end, 16);
        assert!(
            matches!(
                grown,
                Err(Rpf9GrowError::NotTheMap {
                    map: 0x200,
                    byte: 0x200
                })
            ),
            "{grown:?}"
        );
        assert_eq!(sections, before);
        // One byte of data in the head tells it, the last as the first.
        sections.virtual_section[0x200..0x20F].fill(0);
        sections.virtual_section[0x208] = 1;
        let before = sections.clone();
        let grown = grow_section(&mut sections, &[], Rpf9Section::Virtual, end, 16);
        assert!(
            matches!(
                grown,
                Err(Rpf9GrowError::NotTheMap {
                    map: 0x200,
                    byte: 0x20F
                })
            ),
            "{grown:?}"
        );
        assert_eq!(sections, before);
        // The map itself, counting an empty tail after whole pieces, is grown as before.
        let mut whole = mapped(virtual_section(0x1_0000, 0x100, [2, 1]), physical(0x40));
        let end = whole.virtual_section.len();
        grow_section(&mut whole, &[], Rpf9Section::Virtual, end, 16).expect("grown");
        // And so is one whose counts are behind its sections, which the growth brings up to them:
        // a physical section it does not count yet, and a virtual one past its first piece.
        let mut behind = mapped(virtual_section(0x1_0100, 0x100, [1, 0]), physical(0x40));
        let end = behind.virtual_section.len();
        grow_section(&mut behind, &[], Rpf9Section::Virtual, end, 16).expect("grown");
        let v = &behind.virtual_section;
        assert_eq!((v[0x108], v[0x109]), (2, 1));
    }

    #[test]
    fn a_block_map_whose_slots_overhang_its_section_is_refused_when_it_needs_another() {
        let (map, len) = (0xFFE0, 0xFFF0);
        let mut v = virtual_section(len, map, [0, 0]);
        v[map + 8] = 1;
        let mut sections = mapped(v, Vec::new());
        let kept = sections.clone();
        let err = grow_section(&mut sections, &[0x08], Rpf9Section::Virtual, 0x100, 0x20)
            .expect_err("refused")
            .to_string();
        assert!(err.contains("virtual"), "{err}");
        assert_eq!(sections, kept);
    }
}
