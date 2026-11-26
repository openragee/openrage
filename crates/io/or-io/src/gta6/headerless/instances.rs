//! Ray-tracing instances: a root of N transforms and N records, and a physical section tiled with
//! one serialized acceleration structure per record of non-zero size.
//!
//! The key and a record's word are read as what they state by `read_accel_root`, over this
//! reader. Not read, since what they hold is not known: bits 6 and 7 and the top byte of a record
//! word, a structure's words +0x130 to +0x13C, and its nodes past +0x140, which are the console
//! driver's.

use super::bytes::{f32_at, sections, u32_at, u64_at, Bytes};
use super::{HeaderlessError, Result};

/// The root's header, before its arrays.
const HEADER: usize = 0x50;
/// A root is one stream under one piece.
const ROOT_LIMIT: usize = 0x1_0000;
const TRANSFORM: usize = 48;
/// The bytes of a serialized structure's head before its nodes.
const STRUCTURE_HEAD: usize = 0x140;
const FORMAT_WORD: u32 = 0x8102_0006;

#[derive(Debug, Clone, PartialEq)]
pub struct Instances {
    /// v+0x40: the key the game makes of the geometries its ray tracer is given of one level
    /// of the model, which `gta6::accel::select` computes.
    pub key: u64,
    pub instances: Vec<Instance>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// A row-major 3x4 matrix.
    pub transform: [f32; 12],
    /// The bytes of the instance's structure in the physical section, a multiple of 256.
    pub size: u32,
    /// The record's word: bits 8 to 17 the geometries described to the builder, bits 18 to 21
    /// the model's number in its level, bits 22 and 23 the level; its low six bits are clear.
    pub word: u32,
    /// `None` for a record of size 0.
    pub structure: Option<Structure>,
}

/// The head of a serialized acceleration structure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Structure {
    /// A multiple of 64 that rounds up to the record's size; the deserialized size is 0x100 less.
    pub serialized_size: u64,
    /// +0x104, above the primitive count. What it counts is not known: it is neither the
    /// vertices nor the triangles of the build description.
    pub count: u32,
    /// The CRC-32C of the description the structure was built from.
    pub word_108: u32,
    /// +0x10C: inferred from the triangle counts to be the structure's triangle nodes.
    pub primitives: u32,
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    /// +0x128, a multiple of 128 within the deserialized size.
    pub offset_128: u32,
    pub words_130: [u32; 4],
}

impl Instances {
    /// The bytes the records tile the physical section with.
    pub fn records_size(&self) -> u64 {
        self.instances.iter().map(|i| u64::from(i.size)).sum()
    }
}

pub fn read_instances(virtual_section: &[u8], physical: Option<&[u8]>) -> Result<Instances> {
    sections(virtual_section, physical, instances_at)
}

fn constant(what: &'static str, at: usize, found: u64) -> HeaderlessError {
    HeaderlessError::Constant {
        what,
        at: at as u64,
        found,
    }
}

fn disagree(what: &'static str, first: usize, second: usize) -> HeaderlessError {
    HeaderlessError::Disagree {
        what,
        first: first as u64,
        second: second as u64,
    }
}

fn instances_at(v: Bytes, p: Bytes) -> Result<Instances> {
    let root = v.get("root", 0, 0).map(|_| v.len())?;
    if root >= ROOT_LIMIT || !root.is_multiple_of(16) || root < 0xD0 {
        return Err(disagree(
            "root's length and the 16-aligned 0xD0 to 0xFFF0 bytes measured",
            root,
            0xD0,
        ));
    }
    for (what, at, expected) in [
        ("vtable slot", 0x00, 0),
        ("physical pointer", 0x20, 0x6000_0000),
        ("word after the key", 0x48, 0),
        ("transforms pointer", 0x38, 0x5000_0050),
    ] {
        let found = v.u64(what, at)?;
        if found != expected {
            return Err(constant(what, at, found));
        }
    }
    let n = v.u64("instance count", 0x10)?;
    if n == 0 || n > (ROOT_LIMIT / TRANSFORM) as u64 {
        return Err(constant("instance count, 1 or more,", 0x10, n));
    }
    let n = n as usize;
    let slots = (8 * n).next_multiple_of(16);
    let stated = HEADER + TRANSFORM * n + 3 * slots + 32;
    if stated != root {
        return Err(disagree(
            "root's length and the length its instance count gives",
            root,
            stated,
        ));
    }
    let (map, records, a, b) = (
        v.to("block map", 0x08)?,
        v.to("records", 0x18)?,
        v.to("slot array A", 0x28)?,
        v.to("slot array B", 0x30)?,
    );
    let mut arrays = [
        (0, HEADER),
        (HEADER, TRANSFORM * n),
        (records, 8 * n),
        (a, 8 * n),
        (b, 8 * n),
        (map, 16),
    ];
    arrays.sort_unstable();
    let mut end = 0;
    for (start, len) in arrays {
        if !start.is_multiple_of(16) || start < end {
            return Err(disagree(
                "array's start and the end of the one before it",
                start,
                end,
            ));
        }
        let bytes = v.get("root array", start, len)?;
        // The block map and slot arrays A and B are zero but for the map's two counts, one each.
        if [map, a, b].contains(&start) {
            let counts = if start == map { 8..10 } else { 0..0 };
            if let Some(k) = (0..len).find(|k| !counts.contains(k) && bytes[*k] != 0) {
                return Err(constant(
                    "zero byte of a slot array or the block map",
                    start + k,
                    bytes[k].into(),
                ));
            }
            // An array that starts where the map does is shorter than the map, and no map.
            let counts = bytes
                .get(8..10)
                .map_or(0, |c| u16::from_le_bytes([c[0], c[1]]));
            if start == map && counts != 0x0101 {
                return Err(constant("block map's counts", start + 8, counts.into()));
            }
        }
        let gap = v.get("root", end, start - end)?;
        if let Some(k) = gap.iter().position(|b| *b != 0) {
            return Err(constant(
                "byte outside the root's arrays",
                end + k,
                gap[k].into(),
            ));
        }
        end = start + len;
    }
    let tail = v.get("root", end, root - end)?;
    if let Some(k) = tail.iter().position(|b| *b != 0) {
        return Err(constant(
            "byte outside the root's arrays",
            end + k,
            tail[k].into(),
        ));
    }

    let transforms = v.get("transforms", HEADER, TRANSFORM * n)?;
    let words = v.get("records", records, 8 * n)?;
    let mut instances = Vec::with_capacity(n);
    let mut at = 0usize;
    for i in 0..n {
        let transform: [f32; 12] =
            std::array::from_fn(|k| f32_at(transforms, TRANSFORM * i + 4 * k));
        if let Some(k) = transform.iter().position(|f| !f.is_finite()) {
            let at = HEADER + TRANSFORM * i + 4 * k;
            return Err(constant(
                "transform float, finite in every measured root,",
                at,
                transform[k].to_bits().into(),
            ));
        }
        let (size, word) = (u32_at(words, 8 * i), u32_at(words, 8 * i + 4));
        if !size.is_multiple_of(256) {
            return Err(constant(
                "record size, a multiple of 256,",
                records + 8 * i,
                size.into(),
            ));
        }
        if word & 0x3F != 0 {
            return Err(constant(
                "record word, its low six bits clear,",
                records + 8 * i + 4,
                word.into(),
            ));
        }
        let structure = match size {
            0 => None,
            size => Some(structure(p, at, size as usize)?),
        };
        at += size as usize;
        instances.push(Instance {
            transform,
            size,
            word,
            structure,
        });
    }
    if at == 0 {
        return Err(constant(
            "record sizes, not all zero in any measured root,",
            records,
            0,
        ));
    }
    let len = p.get("physical section", 0, 0).map(|_| p.len())?;
    if len != at && len != at.next_multiple_of(ROOT_LIMIT) {
        return Err(disagree(
            "physical section's length and the records' sizes, or those rounded up to 64 KiB",
            len,
            at,
        ));
    }
    let past = p.get("physical section", at, len - at)?;
    if let Some(k) = past.iter().position(|b| *b != 0) {
        return Err(constant(
            "byte past the records' structures",
            at + k,
            past[k].into(),
        ));
    }
    Ok(Instances {
        key: v.u64("key", 0x40)?,
        instances,
    })
}

/// The serialized structure of a record of `size` bytes at `at` of the physical section.
fn structure(p: Bytes, at: usize, size: usize) -> Result<Structure> {
    let bytes = p.get("serialized acceleration structure", at, size)?;
    if size < STRUCTURE_HEAD {
        return Err(disagree(
            "record size and a structure's head",
            size,
            STRUCTURE_HEAD,
        ));
    }
    let zero = |from: usize, to: usize| -> Result<()> {
        match (from..to).find(|k| bytes[*k] != 0) {
            Some(k) => Err(constant(
                "zero byte of a serialized structure's head",
                at + k,
                bytes[k].into(),
            )),
            None => Ok(()),
        }
    };
    zero(0, 0x20)?;
    let serialized = u64_at(bytes, 0x20);
    if !serialized.is_multiple_of(64)
        || serialized > size as u64
        || (serialized as usize).next_multiple_of(256) != size
    {
        return Err(constant(
            "serialized size, a multiple of 64 that rounds up to the record size,",
            at + 0x20,
            serialized,
        ));
    }
    let deserialized = u64_at(bytes, 0x28);
    if deserialized.checked_add(0x100) != Some(serialized) {
        return Err(constant(
            "deserialized size, 0x100 below the serialized size,",
            at + 0x28,
            deserialized,
        ));
    }
    zero(0x30, 0x100)?;
    zero(serialized as usize, size)?;
    let format = u32_at(bytes, 0x100);
    if format != FORMAT_WORD {
        return Err(constant(
            "structure's format word",
            at + 0x100,
            format.into(),
        ));
    }
    let (count, primitives) = (u32_at(bytes, 0x104), u32_at(bytes, 0x10C));
    if primitives == 0 || primitives >= count {
        return Err(disagree(
            "structure's primitive count and the count above it",
            primitives as usize,
            count as usize,
        ));
    }
    let box_min: [f32; 3] = std::array::from_fn(|k| f32_at(bytes, 0x110 + 4 * k));
    let box_max: [f32; 3] = std::array::from_fn(|k| f32_at(bytes, 0x11C + 4 * k));
    if !(0..3).all(|k| box_min[k].is_finite() && box_max[k].is_finite() && box_min[k] <= box_max[k])
    {
        return Err(constant(
            "structure's box, finite with its minimum below its maximum,",
            at + 0x110,
            u64::from(box_min[0].to_bits()),
        ));
    }
    let offset_128 = u32_at(bytes, 0x128);
    if offset_128 == 0 || !offset_128.is_multiple_of(128) || u64::from(offset_128) > deserialized {
        return Err(constant(
            "structure's +0x128, a multiple of 128 within its deserialized size,",
            at + 0x128,
            offset_128.into(),
        ));
    }
    Ok(Structure {
        serialized_size: serialized,
        count,
        word_108: u32_at(bytes, 0x108),
        primitives,
        box_min,
        box_max,
        offset_128,
        words_130: std::array::from_fn(|k| u32_at(bytes, 0x130 + 4 * k)),
    })
}
