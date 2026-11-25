//! A geometry's +0x148 runs, (key index, primitive count) pairs of u16 that give its primitives in
//! order an entry each of its +0x138 table of 8-byte keys, followed through a primitive added or
//! deleted; a vertex edit leaves them. And a geometry child's record, type 5 or 4, copied with
//! every array it names laid out anew.

use crate::container::rpf::Rpf9Sections;

use super::bound::{child_bound, BOUND_CHILD, GEOMETRY, PLAIN_GEOMETRY};
use super::bytes::{sections, u64_at, Bytes};
use super::write_bound_grow::{room, BoundGrowError};
use super::write_bvh::NODE;
use super::write_physics::{put, refuse};
use super::{BoundGeometry, ChildBound, HeaderlessError, Result};

type Grown<T> = std::result::Result<T, BoundGrowError>;

/// The word that addresses virtual offset 0.
const VIRTUAL: u64 = 0x5000_0000;
/// No neighbour across an edge.
const OPEN: u16 = 0xFFFF;
/// The most primitives one pair counts.
const LONGEST: usize = 0xFFFF;
/// A BVH record's bytes.
const BVH_RECORD: usize = 0x70;
/// The words of a geometry child's record that name an array a copy lays out anew.
const NAMED: [usize; 7] = [0x88, 0xB0, 0xF0, 0x118, 0x130, 0x138, 0x148];
/// A type-4 geometry child's record bytes: the bytes at +0x130 are the next object's.
const PLAIN_RECORD: usize = 0x130;
/// The same words of a type-4 geometry child's record.
const PLAIN_NAMED: [usize; 8] = [0x78, 0x88, 0xB0, 0xC0, 0xC8, 0xF0, 0x108, 0x118];

/// Why a child is not copied.
pub(super) const NOT_COPIED: &str = "a child that does not read is not copied";
const F8_SET: &str = "a type-4 geometry whose +0xF8 is set is not copied: the record it names holds pair arrays whose element size is not known";
const OCTANTS: &str = "a type-4 geometry's octants are not laid out as in every measured one: the 8 counts just before the 8 pointers, each pointer null exactly when its count is 0, the index arrays back to back after them";
const SLOTS: &str = "a slot of a type-4 geometry's +0x108 block is not laid out as in every measured one: a count equal to its capacity, and a pointer set exactly when the count is not 0";
const EXCESS: &str = "the geometry's runs give keys to more primitives than it holds, and which primitive the extra runs belong to is not known";
const SHARED: &str =
    "a word of the geometry child aims at bytes no reader lays out, so a copy would share them";

/// A geometry's key table and runs where it holds them, each primitive's key index expanded.
struct Runs {
    table: usize,
    entries: usize,
    runs: usize,
    words: usize,
    index: Vec<u16>,
}

/// The table and runs of the geometry child at `at`, expanded over its `primitives`; `None` where
/// both are null.
fn runs_of(v: &[u8], at: usize, primitives: usize) -> Result<Option<Runs>> {
    sections(v, Some(&[]), |b, _| {
        let table = b.ptr("key table", at + 0x138)?;
        let entries = usize::from(b.u16("key count", at + 0x140)?);
        let words = b.u32("run word count", at + 0x144)? as usize;
        let runs = b.ptr("key runs", at + 0x148)?;
        let (table, runs) = match (table, runs) {
            (None, None) if entries == 0 && words == 0 => return Ok(None),
            (Some(t), Some(r)) if entries != 0 && words != 0 => (t, r),
            _ => {
                return Err(refuse(
                    "a geometry's key table and runs are both null with no counts, or both set with counts, in every measured one",
                ))
            }
        };
        if !words.is_multiple_of(2) {
            return Err(refuse(
                "a geometry states its runs in an odd number of words, where a run is two",
            ));
        }
        b.array("key table", table, entries, 8)?;
        let pairs = b.array("key runs", runs, words, 2)?;
        let mut index = Vec::with_capacity(primitives);
        for pair in pairs.as_chunks::<4>().0 {
            let key = u16::from_le_bytes([pair[0], pair[1]]);
            let run = usize::from(u16::from_le_bytes([pair[2], pair[3]]));
            if run == 0 {
                return Err(refuse("a geometry's run gives its key to no primitive"));
            }
            if usize::from(key) >= entries {
                return Err(HeaderlessError::IndexPastCount {
                    what: "run's key index",
                    index: key.into(),
                    count: entries as u64,
                });
            }
            if index.len() + run > primitives {
                return Err(refuse(EXCESS));
            }
            index.extend(std::iter::repeat_n(key, run));
        }
        if index.len() != primitives {
            return Err(refuse(
                "the geometry's runs give keys to fewer primitives than it holds",
            ));
        }
        Ok(Some(Runs {
            table,
            entries,
            runs,
            words,
            index,
        }))
    })
}

/// `index` as maximal runs in primitive order, each an (index, length) pair of u16.
// A run longer than a pair counts is split.
pub(super) fn runs_encoded(index: &[u16]) -> Vec<u8> {
    let mut out = Vec::new();
    for run in index.chunk_by(|a, b| a == b) {
        for part in run.chunks(LONGEST) {
            out.extend(part[0].to_le_bytes());
            out.extend((part.len() as u16).to_le_bytes());
        }
    }
    out
}

fn word_of(to: usize) -> [u8; 8] {
    (VIRTUAL + to as u64).to_le_bytes()
}

fn primitive_count(v: &[u8], at: usize) -> Result<usize> {
    sections(v, Some(&[]), |b, _| {
        Ok(b.u32("primitive count", at + 0xD4)? as usize)
    })
}

/// `index` written as the runs of the geometry child at `at`, which held `held`: over the array
/// where they fit its 16-byte slot, else laid out past the section's end; the word count with
/// them, and read back.
fn runs_written(s: &mut Rpf9Sections, at: usize, held: &Runs, index: &[u16]) -> Grown<()> {
    let mut bytes = runs_encoded(index);
    let words = bytes.len() / 2;
    // Runs that grow stay only where the bytes up to the next 16 are zero, as measured in every one.
    let (old, end) = (held.runs + 2 * held.words, held.runs + bytes.len());
    let free =
        (s.virtual_section.get(old..end.max(old))).is_some_and(|b| b.iter().all(|x| *x == 0));
    let fits = end <= old || (free && end <= old.next_multiple_of(16));
    let to = match fits {
        true => {
            bytes.resize(bytes.len().max(2 * held.words), 0);
            held.runs
        }
        false => room(s, &[bytes.len()])?[0],
    };
    let v = &mut s.virtual_section;
    put(v, to, &bytes)?;
    put(v, at + 0x148, &word_of(to))?;
    put(v, at + 0x144, &(words as u32).to_le_bytes())?;
    let back = runs_of(&s.virtual_section, at, index.len())?.map(|r| r.index);
    if back.as_deref() != Some(index) {
        return Err(refuse("the geometry's runs as written do not read back").into());
    }
    Ok(())
}

/// The runs of the geometry child at `at` once its primitive `p` is deleted, as +0xD4 states.
// `p`'s key index goes, and the table entry no primitive names then goes with every index past it
// one less, which keeps the table ascending.
pub(super) fn runs_after_delete(s: &mut Rpf9Sections, at: usize, p: usize) -> Grown<()> {
    let now = primitive_count(&s.virtual_section, at)?;
    let Some(held) = runs_of(&s.virtual_section, at, now + 1)? else {
        return Ok(());
    };
    let mut index = held.index.clone();
    if p >= index.len() {
        return Err(HeaderlessError::IndexPastCount {
            what: "primitive to delete",
            index: p as u64,
            count: index.len() as u64,
        }
        .into());
    }
    let gone = index.remove(p);
    if !index.contains(&gone) {
        for i in &mut index {
            if *i > gone {
                *i -= 1;
            }
        }
        let v = &mut s.virtual_section;
        let len = 8 * held.entries;
        let mut table = sections(v, Some(&[]), |b, _| {
            Ok(b.array("key table", held.table, held.entries, 8)?.to_vec())
        })?;
        let from = 8 * usize::from(gone);
        table.drain(from..from + 8);
        table.resize(len, 0);
        put(v, held.table, &table)?;
        put(v, at + 0x140, &((held.entries - 1) as u16).to_le_bytes())?;
    }
    runs_written(s, at, &held, &index)
}

/// The runs of the geometry child at `at` once a primitive is added last, as +0xD4 states.
// It takes the key index of the first triangle it links to across an edge, else the last
// primitive's; no table entry is added, since what an entry means is not known.
pub(super) fn runs_after_add(s: &mut Rpf9Sections, at: usize) -> Grown<()> {
    let now = primitive_count(&s.virtual_section, at)?;
    let n = now
        .checked_sub(1)
        .ok_or(refuse("the geometry holds no primitive added"))?;
    let Some(held) = runs_of(&s.virtual_section, at, n)? else {
        return Ok(());
    };
    let record = sections(&s.virtual_section, Some(&[]), |b, _| {
        let primitives = b.to("primitives", at + 0x88)?;
        Ok(b.get("added primitive", primitives + 16 * n, 16)?.to_vec())
    })?;
    let linked = match record[3] {
        0 => [0xA_usize, 0xC, 0xE]
            .map(|o| u16::from_le_bytes([record[o], record[o + 1]]))
            .into_iter()
            .find(|k| *k != OPEN && usize::from(*k) < n)
            .map(usize::from),
        _ => None,
    };
    let key = (linked.or(n.checked_sub(1)))
        .and_then(|k| held.index.get(k).copied())
        .ok_or(refuse(
            "the added primitive has no primitive before it to take a key from",
        ))?;
    let mut index = held.index.clone();
    index.push(key);
    runs_written(s, at, &held, &index)
}

/// Bytes a geometry child's copy lays out anew: the record's words that name them, each with where
/// in them it aims, and their own words that aim into them, each with where.
struct Piece {
    bytes: Vec<u8>,
    named_by: Vec<(usize, usize)>,
    inner: Vec<(usize, usize)>,
}

impl Piece {
    fn named(word: usize, bytes: Vec<u8>) -> Self {
        Piece {
            bytes,
            named_by: vec![(word, 0)],
            inner: Vec::new(),
        }
    }
}

/// A type-4 geometry's octant counts, pointers and index arrays as one piece; `None` where its
/// +0xC0 and +0xC8 are both null.
fn octants(b: Bytes, at: usize) -> Result<Option<Piece>> {
    let (counts, pointers) = match (
        b.ptr("octant counts", at + 0xC0)?,
        b.ptr("octants", at + 0xC8)?,
    ) {
        (None, None) => return Ok(None),
        (Some(c), Some(p)) if p == c + 32 => (c, p),
        _ => return Err(refuse(OCTANTS)),
    };
    let mut inner = Vec::new();
    let mut next = 96;
    for i in 0..8 {
        let count = b.u32("octant count", counts + 4 * i)? as usize;
        let to = b.ptr("octant indices", pointers + 8 * i)?;
        if to != (count != 0).then_some(counts + next) {
            return Err(refuse(OCTANTS));
        }
        if count != 0 {
            inner.push((32 + 8 * i, next));
            next += b.array("octant indices", counts + next, count, 4)?.len();
        }
    }
    Ok(Some(Piece {
        bytes: b.get("octants", counts, next)?.to_vec(),
        named_by: vec![(0xC0, 0), (0xC8, 32)],
        inner,
    }))
}

/// A type-4 geometry's +0x108 block of three slots and each slot's u16 array as one piece; `None`
/// where the word is null.
fn displacements(b: Bytes, at: usize) -> Result<Option<Piece>> {
    let Some(block) = b.ptr("+0x108 block", at + 0x108)? else {
        return Ok(None);
    };
    let mut bytes = b.get("+0x108 block", block, 48)?.to_vec();
    named_only(&bytes, &[8, 24, 40])?;
    let mut inner = Vec::new();
    for slot in (0..48).step_by(16) {
        let count = b.u32("+0x108 slot count", block + slot)?;
        let capacity = b.u32("+0x108 slot capacity", block + slot + 4)?;
        let to = match b.ptr("+0x108 slot array", block + slot + 8)? {
            None if count == 0 && capacity == 0 => continue,
            Some(to) if count != 0 && count == capacity => to,
            _ => return Err(refuse(SLOTS)),
        };
        let array = b.array("+0x108 slot array", to, count as usize, 2)?;
        let rel = bytes.len().next_multiple_of(16);
        bytes.resize(rel, 0);
        bytes.extend_from_slice(array);
        inner.push((slot + 8, rel));
    }
    Ok(Some(Piece {
        bytes,
        named_by: vec![(0x108, 0)],
        inner,
    }))
}

/// A geometry child's record of type `kind` and what its copy lays out: each array it names, and
/// a type-5 child's BVH record with the nodes at the count it allocates.
fn parts(v: &[u8], at: usize, kind: u8) -> Result<(Vec<u8>, Vec<Piece>)> {
    sections(v, Some(&[]), |b, _| {
        let plain = kind == PLAIN_GEOMETRY;
        let len = if plain { PLAIN_RECORD } else { BOUND_CHILD };
        let mut record = b.get("geometry child", at, len)?.to_vec();
        if plain && b.u64("+0xF8 record", at + 0xF8)? != 0 {
            return Err(refuse(F8_SET));
        }
        let primitives = b.u32("primitive count", at + 0xD4)? as usize;
        let mut lists = vec![
            (0x88, "primitives", primitives, 16),
            (
                0xB0,
                "vertices",
                b.u32("vertex count", at + 0xD0)? as usize,
                6,
            ),
            (
                0xF0,
                "material ids",
                usize::from(b.u8("material count", at + 0x120)?),
                8,
            ),
            (0x118, "per-primitive material indices", primitives, 1),
        ];
        match plain {
            true => lists.push((
                0x78,
                "shrunk vertices",
                usize::from(b.u16("hull vertex count", at + 0x84)?),
                6,
            )),
            false => lists.extend([
                (
                    0x138,
                    "key table",
                    usize::from(b.u16("key count", at + 0x140)?),
                    8,
                ),
                (
                    0x148,
                    "key runs",
                    b.u32("run word count", at + 0x144)? as usize,
                    2,
                ),
            ]),
        }
        let mut pieces = Vec::new();
        for (word, what, count, stride) in lists {
            let Some(to) = b.ptr(what, at + word)? else {
                continue;
            };
            let bytes = b.array(what, to, count, stride)?;
            // An empty array names nothing a copy could share, so the copy's word is null rather
            // than left aiming into the source.
            match bytes.is_empty() {
                true => record[word..word + 8].fill(0),
                false => pieces.push(Piece::named(word, bytes.to_vec())),
            }
        }
        if plain {
            pieces.extend(octants(b, at)?);
            pieces.extend(displacements(b, at)?);
        } else if let Some(h) = b.ptr("BVH", at + 0x130)? {
            let mut bytes = b.get("BVH", h, BVH_RECORD)?.to_vec();
            named_only(&bytes, &[0x50])?;
            let count = b.u32("BVH node count", h + 0x58)? as usize;
            let held = count.max(b.u32("BVH allocated nodes", h + 0x5C)? as usize);
            let nodes = b.array("BVH nodes", b.to("BVH nodes", h + 0x50)?, count, NODE)?;
            let len = (held.checked_mul(NODE))
                .filter(|l| *l <= b.len())
                .ok_or(refuse(
                    "the geometry's BVH allocates more nodes than its section holds",
                ))?;
            bytes.extend_from_slice(nodes);
            bytes.resize(BVH_RECORD + len, 0);
            pieces.push(Piece {
                bytes,
                named_by: vec![(0x130, 0)],
                inner: vec![(0x50, BVH_RECORD)],
            });
        }
        Ok((record, pieces))
    })
}

/// Refused where a word of `bytes` other than those at `known` aims at either section.
fn named_only(bytes: &[u8], known: &[usize]) -> Result<()> {
    for o in (0..bytes.len().saturating_sub(7)).step_by(8) {
        let word = u64_at(bytes, o);
        if word >> 32 == 0 && matches!((word as u32) >> 28, 5 | 6) && !known.contains(&o) {
            return Err(refuse(SHARED));
        }
    }
    Ok(())
}

/// The record of a copy of the geometry child at `source`; `None` for a child that is no geometry.
// Every array it names, a type-5 child's BVH record and nodes, a type-4 child's octants and +0x108
// block are laid out anew past the virtual section's end and named by it. Its keys are copied as
// they are: a key may be named by several geometries of a body.
pub(super) fn geometry_copied(s: &mut Rpf9Sections, source: usize) -> Grown<Option<Vec<u8>>> {
    let v = &s.virtual_section;
    let kind = sections(v, Some(&[]), |b, _| b.u8("bound type", source + 0x10))?;
    let named: &[usize] = match kind {
        GEOMETRY => &NAMED,
        PLAIN_GEOMETRY => &PLAIN_NAMED,
        _ => return Ok(None),
    };
    sections(v, Some(&[]), |b, _| child_bound(b, source))?;
    let (mut record, pieces) = parts(v, source, kind)?;
    named_only(&record, named)?;
    let lens: Vec<usize> = pieces.iter().map(|p| p.bytes.len()).collect();
    let places = room(s, &lens)?;
    let v = &mut s.virtual_section;
    for (mut piece, to) in pieces.into_iter().zip(places) {
        for (word, rel) in piece.inner {
            put(&mut piece.bytes, word, &word_of(to + rel))?;
        }
        put(v, to, &piece.bytes)?;
        for (word, rel) in piece.named_by {
            put(&mut record, word, &word_of(to + rel))?;
        }
    }
    Ok(Some(record))
}

/// A copy of geometry `g`.
pub(super) fn geometry_value_copied(g: &BoundGeometry) -> ChildBound {
    ChildBound::Geometry(BoundGeometry {
        box_min: g.box_min,
        box_max: g.box_max,
        vertices: g.vertices.clone(),
        primitives: g.primitives.clone(),
        material_ids: g.material_ids.clone(),
        primitive_materials: g.primitive_materials.clone(),
        bvh: g.bvh,
        unknown_shapes: g.unknown_shapes.clone(),
    })
}
