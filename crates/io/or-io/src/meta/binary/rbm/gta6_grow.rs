//! An `AtArray64` or a counted array of a GTA VI headerless RBM resource taking or losing an
//! element, its sections grown by the resource rule: a block grows at its end, and a full array
//! that another region follows in its block moves to the block's end, leaving its old elements as
//! bytes nothing names. A counted array's capacity is its count, as every one stored states.
//!
//! ⚠ An empty array behind a null pointer has no block to grow into, and a block in the physical
//! section grows only by packing that section again: both are refused.

use crate::container::rpf::{grow_section, Rpf9GrowError, Rpf9Section, Rpf9Sections};
use crate::meta::binary::pso::table_form::fixed_width;
use crate::meta::error::MetaError;

use super::gta6::read_model;
use super::schema::{PsoType, RbmDataBlock, RbmModel};

/// The name of the member that describes an array's element.
const ARRAY_INFO: u32 = 0x100;
/// A counted array's type: its head is `{u32 count, u32 capacity, u64 pointer}`.
pub(super) const COUNTED: u8 = 0x5C;
const OFFSET: u64 = 0x0FFF_FFFF;
/// The header's pointer to the data-block table, whose 16-byte records are
/// `{u32 type, u32 size, u64 address}`.
const BLOCK_TABLE: usize = 0x38;

#[derive(Debug, thiserror::Error)]
pub enum Gta6GrowError {
    #[error(transparent)]
    Meta(#[from] MetaError),
    #[error(transparent)]
    Grow(#[from] Rpf9GrowError),
    #[error("the resource has {count} data blocks, and no block {block}")]
    NoBlock { block: u32, count: usize },
    #[error("block {block} holds no AtArray64 member of its structure at {member:#X}")]
    NotAnArray { block: u32, member: u64 },
    #[error("an array of type {code:#04X} elements is not grown: its element's size does not follow from its type")]
    Element { code: u8 },
    #[error("the element is {len} bytes, where the array's are {stride}")]
    Stride { len: usize, stride: usize },
    #[error("element {index} is past the array's {count}")]
    Index { index: usize, count: usize },
    #[error("the array's pointer {raw:#X} sets bits 12 to 19 or an offset past the address space")]
    Pointer { raw: u64 },
    #[error("the array is empty behind a null pointer, so no block holds its elements to grow")]
    Null,
    #[error(
        "the array states {count} elements in room for {capacity} that its block does not hold"
    )]
    Room { count: usize, capacity: usize },
    #[error("the array holds {0} elements, the most its 16-bit count states")]
    Full(usize),
    #[error("block {block} is in the physical section, which grows only by packing it again, which is not done")]
    Physical { block: u32 },
    #[error("the data-block table is not in the virtual section")]
    TablePhysical,
    #[error("block {block} is empty, so it has no end of its own to grow at")]
    Empty { block: u32 },
    #[error("the bytes after block {block} up to its 16-byte end are not free: a non-zero byte or an allocation lies there")]
    NotFree { block: u32 },
    #[error("the edited resource does not read back as written: {0}")]
    ReadBack(String),
}

type Result<T> = std::result::Result<T, Gta6GrowError>;

/// Inserts `element` at `index` of the `AtArray64` at `member` of `block`, read back before kept.
// The sections grow where the array's room is full.
pub fn insert_element(
    sections: &mut Rpf9Sections,
    block: u32,
    member: usize,
    index: usize,
    element: &[u8],
) -> Result<()> {
    let _names = crate::meta::binary::hash_index::title_names();
    let mut s = sections.clone();
    let before = read_model(&s.virtual_section, &s.physical)?;
    let a = array(&before, block, member)?;
    if element.len() != a.stride {
        return Err(Gta6GrowError::Stride {
            len: element.len(),
            stride: a.stride,
        });
    }
    if index > a.count {
        return Err(Gta6GrowError::Index {
            index,
            count: a.count,
        });
    }
    if a.count >= usize::from(u16::MAX) {
        return Err(Gta6GrowError::Full(a.count));
    }
    let old = elements(&before, &a)?;
    let at = index * a.stride;
    let new = [&old[..at], element, &old[at..]].concat();
    let room = block_of(&before, a.elements)?.data.len();
    let grown = if a.count < a.capacity {
        a
    } else if a.first + a.capacity * a.stride == room {
        append_to_block(&mut s, a.elements, 1, &vec![0; a.stride])?;
        Array {
            capacity: a.capacity + 1,
            ..a
        }
    } else {
        let first = append_to_block(&mut s, a.elements, 16, &vec![0; new.len()])?;
        Array {
            first,
            capacity: a.count + 1,
            ..a
        }
    };
    let grown = Array {
        count: a.count + 1,
        ..grown
    };
    write_array(&mut s, block, member, &grown, &new)?;
    read_back(&s, &before, block, member, &new)?;
    *sections = s;
    Ok(())
}

/// Removes element `index` of the `AtArray64` at `member` of `block`, read back before kept.
// Its room goes back to the section when the array fills the end of its block.
pub fn remove_element(
    sections: &mut Rpf9Sections,
    block: u32,
    member: usize,
    index: usize,
) -> Result<()> {
    let _names = crate::meta::binary::hash_index::title_names();
    let mut s = sections.clone();
    let before = read_model(&s.virtual_section, &s.physical)?;
    let a = array(&before, block, member)?;
    if index >= a.count {
        return Err(Gta6GrowError::Index {
            index,
            count: a.count,
        });
    }
    let old = elements(&before, &a)?;
    let at = index * a.stride;
    let new = [&old[..at], &old[at + a.stride..]].concat();
    let room = block_of(&before, a.elements)?.data.len();
    let last = a.count == a.capacity && a.first + a.capacity * a.stride == room && room > a.stride;
    let mut bytes = new.clone();
    let capacity = match last {
        true => {
            shrink_block(&mut s, a.elements, a.stride)?;
            a.capacity - 1
        }
        // The freed slot is zeroed and stays the room of an `AtArray64`; a counted array's
        // capacity follows its count.
        false => {
            bytes.resize(new.len() + a.stride, 0);
            if a.counted {
                a.count - 1
            } else {
                a.capacity
            }
        }
    };
    let shrunk = Array {
        count: a.count - 1,
        capacity,
        ..a
    };
    write_array(&mut s, block, member, &shrunk, &bytes)?;
    read_back(&s, &before, block, member, &new)?;
    *sections = s;
    Ok(())
}

/// Appends `bytes` to block `block` at its first offset that is a multiple of `align` (at least
/// 1), growing the virtual section at the block's 16-byte end; returns that offset.
pub(crate) fn append_to_block(
    sections: &mut Rpf9Sections,
    block: u32,
    align: usize,
    bytes: &[u8],
) -> Result<usize> {
    let _names = crate::meta::binary::hash_index::title_names();
    let model = read_model(&sections.virtual_section, &sections.physical)?;
    let b = block_of(&model, block)?;
    let at = virtual_at(b, block)?;
    let size = b.data.len();
    if size == 0 {
        return Err(Gta6GrowError::Empty { block });
    }
    let first = size.next_multiple_of(align);
    let end = at + size;
    let free_to = end.next_multiple_of(16);
    let pointers = based_pointers(&sections.virtual_section);
    if !is_free(&sections.virtual_section, &pointers, end, free_to) {
        return Err(Gta6GrowError::NotFree { block });
    }
    let need = (at + first + bytes.len()).next_multiple_of(16);
    if need > free_to {
        grow_section(
            sections,
            &pointers,
            Rpf9Section::Virtual,
            free_to,
            (need - free_to) as isize,
        )?;
    }
    let addr = set_size(sections, block, first + bytes.len())?;
    put(sections, addr, first, bytes)?;
    Ok(first)
}

/// Takes `by` bytes, fewer than it holds, off the end of block `block`, and gives the section back
/// the 16-byte room they free.
fn shrink_block(sections: &mut Rpf9Sections, block: u32, by: usize) -> Result<()> {
    let model = read_model(&sections.virtual_section, &sections.physical)?;
    let b = block_of(&model, block)?;
    let at = virtual_at(b, block)?;
    let size = b.data.len();
    let kept = size - by;
    put(sections, b.data_addr, kept, &vec![0; by])?;
    let (from, to) = (
        (at + kept).next_multiple_of(16),
        (at + size).next_multiple_of(16),
    );
    let pointers = based_pointers(&sections.virtual_section);
    if !is_free(&sections.virtual_section, &pointers, at + size, to) {
        return Err(Gta6GrowError::NotFree { block });
    }
    if to > from {
        grow_section(
            sections,
            &pointers,
            Rpf9Section::Virtual,
            from,
            -((to - from) as isize),
        )?;
    }
    set_size(sections, block, kept)?;
    Ok(())
}

/// Whether the virtual bytes `from..to` are zero and no pointer aims into them.
fn is_free(v: &[u8], pointers: &[usize], from: usize, to: usize) -> bool {
    let zero = v
        .get(from..to)
        .is_some_and(|pad| pad.iter().all(|b| *b == 0));
    let aimed = pointers.iter().any(|&o| {
        let w = word(v, o);
        w >> 28 == 5 && (from..to).contains(&((w & OFFSET) as usize))
    });
    zero && !aimed
}

/// Writes block `block`'s size into its record; returns the block's address.
fn set_size(sections: &mut Rpf9Sections, block: u32, size: usize) -> Result<u64> {
    let v = &mut sections.virtual_section;
    let table = word(v, BLOCK_TABLE);
    if table >> 28 != 5 {
        return Err(Gta6GrowError::TablePhysical);
    }
    let record = (table & OFFSET) as usize + 16 * (block as usize).saturating_sub(1);
    let fields = v.get_mut(record..record + 16).ok_or_else(|| {
        Gta6GrowError::ReadBack(format!(
            "block {block}'s record at {record:#X} lies past the virtual section"
        ))
    })?;
    fields[4..8].copy_from_slice(&(size as u32).to_le_bytes());
    Ok(word(fields, 8))
}

/// The virtual-section offset of every word an RBM body states as a pointer: the header's, and
/// each record's in the class, enum and data-block tables. Pointers inside data name a block by
/// its id, so a move of the bytes leaves them alone.
fn based_pointers(v: &[u8]) -> Vec<usize> {
    let mut out = vec![0x08, 0x28, 0x30, BLOCK_TABLE, 0x40, 0x50];
    for (table, count_at, size, field) in [
        (0x28, 0x48, 32, 16),
        (0x30, 0x4A, 24, 8),
        (BLOCK_TABLE, 0x4C, 16, 8),
    ] {
        let ptr = word(v, table);
        if ptr >> 28 == 5 {
            let at = (ptr & OFFSET) as usize;
            out.extend((0..half(v, count_at)).map(|i| at + size * i + field));
        }
    }
    out
}

fn word(bytes: &[u8], at: usize) -> u64 {
    (bytes.get(at..at + 8))
        .and_then(|b| b.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

fn half(bytes: &[u8], at: usize) -> usize {
    bytes
        .get(at..at + 2)
        .map_or(0, |b| usize::from(b[0]) | usize::from(b[1]) << 8)
}

fn block_of(model: &RbmModel, block: u32) -> Result<&RbmDataBlock> {
    (i32::try_from(block).ok())
        .and_then(|id| model.block(id))
        .ok_or(Gta6GrowError::NoBlock {
            block,
            count: model.data_blocks.len(),
        })
}

fn virtual_at(b: &RbmDataBlock, block: u32) -> Result<usize> {
    match b.data_addr >> 28 {
        5 => Ok((b.data_addr & OFFSET) as usize),
        _ => Err(Gta6GrowError::Physical { block }),
    }
}

/// `bytes` written at `at` of the block whose address is `addr`.
fn put(sections: &mut Rpf9Sections, addr: u64, at: usize, bytes: &[u8]) -> Result<()> {
    let section = match addr >> 28 {
        5 => &mut sections.virtual_section,
        _ => &mut sections.physical,
    };
    let from = (addr & OFFSET) as usize + at;
    let to = from + bytes.len();
    let held = section.len();
    section
        .get_mut(from..to)
        .ok_or_else(|| {
            Gta6GrowError::ReadBack(format!(
                "{} bytes at {from:#X} lie past their {held}-byte section",
                bytes.len()
            ))
        })?
        .copy_from_slice(bytes);
    Ok(())
}

/// An `AtArray64` or a counted array: the block of its elements, the first one's offset there,
/// its count and room, its element's size, and whether it is counted.
#[derive(Debug, Clone, Copy)]
struct Array {
    elements: u32,
    first: usize,
    count: usize,
    capacity: usize,
    stride: usize,
    counted: bool,
}

impl Array {
    /// The bytes of its head in the member that holds it.
    fn head_len(&self) -> usize {
        if self.counted {
            16
        } else {
            12
        }
    }
}

fn quad(bytes: &[u8], at: usize) -> usize {
    (bytes.get(at..at + 4)).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

fn array(model: &RbmModel, block: u32, member: usize) -> Result<Array> {
    let not = || Gta6GrowError::NotAnArray {
        block,
        member: member as u64,
    };
    let holder = block_of(model, block)?;
    let class = model.classes.get(&holder.data_type).ok_or_else(not)?;
    let at = member
        .checked_rem(class.data_size as usize)
        .ok_or_else(not)?;
    let entry = (class.entries.iter())
        .find(|e| {
            e.name_hash != ARRAY_INFO
                && e.offset as usize == at
                && matches!(e.data_type, PsoType::AtArray64 | PsoType::Unknown(COUNTED))
        })
        .ok_or_else(not)?;
    let counted = entry.data_type == PsoType::Unknown(COUNTED);
    let element = class.entry(entry.contents_index()).ok_or_else(not)?;
    let stride = match element.data_type {
        PsoType::Struct => {
            (model.classes.get(&element.referent_hash())).map_or(0, |c| c.data_size as usize)
        }
        other => fixed_width(other).unwrap_or(0),
    };
    if stride == 0 {
        return Err(Gta6GrowError::Element {
            code: element.data_type.to_byte(),
        });
    }
    let head = (member.checked_add(if counted { 16 } else { 12 }))
        .and_then(|end| holder.data.get(member..end))
        .ok_or_else(not)?;
    let (raw, count, capacity) = match counted {
        true => (word(head, 8), quad(head, 0), quad(head, 4)),
        false => (word(head, 0), half(head, 8), half(head, 10)),
    };
    if raw & 0xFFF == 0 {
        return Err(Gta6GrowError::Null);
    }
    if (raw >> 12) & 0xFF != 0 {
        return Err(Gta6GrowError::Pointer { raw });
    }
    let elements = (raw & 0xFFF) as u32;
    let first = usize::try_from(raw >> 20).map_err(|_| Gta6GrowError::Pointer { raw })?;
    let room = block_of(model, elements)?.data.len();
    let fits = (capacity.checked_mul(stride))
        .and_then(|len| len.checked_add(first))
        .is_some_and(|end| end <= room);
    if count > capacity || !fits {
        return Err(Gta6GrowError::Room { count, capacity });
    }
    Ok(Array {
        elements,
        first,
        count,
        capacity,
        stride,
        counted,
    })
}

/// The bytes of the array's `count` elements.
fn elements(model: &RbmModel, a: &Array) -> Result<Vec<u8>> {
    let data = &block_of(model, a.elements)?.data;
    Ok(data[a.first..a.first + a.count * a.stride].to_vec())
}

/// `bytes` written as the array's elements and its pointer, count and capacity into its member.
fn write_array(
    sections: &mut Rpf9Sections,
    block: u32,
    member: usize,
    a: &Array,
    bytes: &[u8],
) -> Result<()> {
    let model = read_model(&sections.virtual_section, &sections.physical)?;
    let (holder, elements) = (
        block_of(&model, block)?.data_addr,
        block_of(&model, a.elements)?.data_addr,
    );
    put(sections, elements, a.first, bytes)?;
    let raw = u64::from(a.elements) | (a.first as u64) << 20;
    let mut head = Vec::with_capacity(a.head_len());
    if a.counted {
        head.extend((a.count as u32).to_le_bytes());
        head.extend((a.capacity as u32).to_le_bytes());
        head.extend(raw.to_le_bytes());
    } else {
        head.extend(raw.to_le_bytes());
        head.extend((a.count as u16).to_le_bytes());
        head.extend((a.capacity as u16).to_le_bytes());
    }
    put(sections, holder, member, &head)
}

/// Refuses an edit whose sections do not read back to `expected` as the array's elements with every
/// other block as it was.
fn read_back(
    s: &Rpf9Sections,
    before: &RbmModel,
    block: u32,
    member: usize,
    expected: &[u8],
) -> Result<()> {
    let fail = Gta6GrowError::ReadBack;
    let after = read_model(&s.virtual_section, &s.physical).map_err(|e| fail(e.to_string()))?;
    let a = array(&after, block, member).map_err(|e| fail(e.to_string()))?;
    if elements(&after, &a)? != expected {
        return Err(fail(format!(
            "the array reads {} elements, which are not the {} written",
            a.count,
            expected.len() / a.stride
        )));
    }
    if after.data_blocks.len() != before.data_blocks.len() {
        return Err(fail(format!(
            "it holds {} data blocks, not {}",
            after.data_blocks.len(),
            before.data_blocks.len()
        )));
    }
    for (i, (was, is)) in before
        .data_blocks
        .iter()
        .zip(&after.data_blocks)
        .enumerate()
    {
        let id = i as u32 + 1;
        let same = if id == a.elements {
            true
        } else if id == block {
            let (w, n, end) = (&was.data, &is.data, member + a.head_len());
            w.len() == n.len() && w[..member] == n[..member] && w[end..] == n[end..]
        } else {
            was.data == is.data
        };
        if was.data_type != is.data_type || !same {
            return Err(fail(format!("block {id} changed")));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Form};
    use crate::meta::binary::hash_index::default_hash_index;
    use crate::meta::binary::pso::table_form;
    use crate::meta::instance::value::Value;

    const VIRTUAL: u64 = 0x5000_0000;
    const ROOT: u32 = 0x4545_0001;
    const ELEMENT: u32 = 0x4545_0002;

    /// A 16-byte member record.
    pub(crate) fn member(name: u32, offset: u32, code: u8, ah: u16, ch: u32) -> [u8; 16] {
        let mut m = [0u8; 16];
        m[0..4].copy_from_slice(&name.to_le_bytes());
        m[4..8].copy_from_slice(&offset.to_le_bytes());
        m[8] = code;
        m[10..12].copy_from_slice(&ah.to_le_bytes());
        m[12..16].copy_from_slice(&ch.to_le_bytes());
        m
    }

    /// A packed pointer inside data: the block id in bits 0 to 11, the offset from bit 20.
    pub(crate) fn pointer(block: u64, offset: u64) -> u64 {
        block | offset << 20
    }

    /// A headerless virtual section of `(name, size, members)` classes and `(type, bytes)` blocks,
    /// each at a 16-byte boundary after the tables, and its one-slot block map last.
    pub(crate) fn body(
        classes: &[(u32, u32, Vec<[u8; 16]>)],
        blocks: &[(u32, Vec<u8>)],
        root: u32,
    ) -> Rpf9Sections {
        let members_at = 0x60 + 32 * classes.len();
        let members: usize = classes.iter().map(|c| c.2.len()).sum();
        let table = (members_at + 16 * members).next_multiple_of(16);
        let mut end = table + 16 * blocks.len();
        let mut addrs = Vec::new();
        for (_, data) in blocks {
            end = end.next_multiple_of(16);
            addrs.push(end);
            end += data.len();
        }
        let map = end.next_multiple_of(16);
        let mut v = vec![0u8; (map + 24).next_multiple_of(16)];
        let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x08, &(VIRTUAL + map as u64).to_le_bytes());
        put(0x10, b"0DRP");
        put(0x14, &0x7Au16.to_le_bytes());
        put(0x20, &root.to_le_bytes());
        put(0x28, &(VIRTUAL + 0x60).to_le_bytes());
        put(0x38, &(VIRTUAL + table as u64).to_le_bytes());
        put(0x48, &(classes.len() as u16).to_le_bytes());
        put(0x4C, &(blocks.len() as u16).to_le_bytes());
        let mut next = members_at;
        for (i, (name, size, members)) in classes.iter().enumerate() {
            let c = 0x60 + 32 * i;
            put(c, &name.to_le_bytes());
            put(c + 4, &0x5151_5151u32.to_le_bytes());
            put(c + 9, &[4]);
            put(c + 16, &(VIRTUAL + next as u64).to_le_bytes());
            put(c + 24, &size.to_le_bytes());
            put(c + 30, &(members.len() as u16).to_le_bytes());
            for m in members {
                put(next, m);
                next += 16;
            }
        }
        for (i, ((data_type, data), addr)) in blocks.iter().zip(&addrs).enumerate() {
            let record = table + 16 * i;
            put(record, &data_type.to_le_bytes());
            put(record + 4, &(data.len() as u32).to_le_bytes());
            put(record + 8, &(VIRTUAL + *addr as u64).to_le_bytes());
            put(*addr, data);
        }
        put(map + 8, &[1, 0]);
        Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: Vec::new(),
        }
    }

    fn element(i: i32, f: f32) -> Vec<u8> {
        [i.to_le_bytes(), f.to_le_bytes()].concat()
    }

    /// A root of two arrays of 8-byte structures in one block: `first` holds two at 0, `second`
    /// one at 16, the last region of the block.
    fn two_arrays() -> Rpf9Sections {
        let root = (
            ROOT,
            0x30,
            vec![
                member(0x4545_0010, 0x00, 0x52, 1, 0),
                member(ARRAY_INFO, 0, 0x05, 0, ELEMENT),
                member(0x4545_0011, 0x10, 0x52, 3, 0),
                member(ARRAY_INFO, 0, 0x05, 0, ELEMENT),
                member(0x4545_0012, 0x20, 0x15, 0, 0),
            ],
        );
        let pair = (
            ELEMENT,
            8,
            vec![
                member(0x4545_0020, 0, 0x14, 0, 0),
                member(0x4545_0021, 4, 0x21, 0, 0),
            ],
        );
        let mut data = vec![0u8; 0x30];
        data[0x00..0x08].copy_from_slice(&pointer(2, 0).to_le_bytes());
        data[0x08..0x0C].copy_from_slice(&[2, 0, 2, 0]);
        data[0x10..0x18].copy_from_slice(&pointer(2, 16).to_le_bytes());
        data[0x18..0x1C].copy_from_slice(&[1, 0, 1, 0]);
        data[0x20..0x24].copy_from_slice(&0x77u32.to_le_bytes());
        let elements = [element(10, 0.25), element(-20, 0.5), element(30, 0.75)].concat();
        body(&[root, pair], &[(ROOT, data), (ELEMENT, elements)], 1)
    }

    /// The structures of the root's two arrays, as the table form decodes them.
    fn decoded(s: &Rpf9Sections) -> [Vec<(i64, f64)>; 2] {
        let model = read_model(&s.virtual_section, &s.physical).expect("it reads");
        let root = table_form::decode(&model).expect("it decodes");
        let name = |hash: u32| default_hash_index().name_or_literal(hash);
        let pairs = |array: u32| -> Vec<(i64, f64)> {
            let Some(Value::Array(items)) = root.get(&name(array)) else {
                panic!("an array");
            };
            (items.iter())
                .map(|item| {
                    let Value::Struct(e) = item else {
                        panic!("a structure");
                    };
                    let (Some(Value::Int(i)), Some(Value::Float(f))) =
                        (e.get(&name(0x4545_0020)), e.get(&name(0x4545_0021)))
                    else {
                        panic!("an element");
                    };
                    (*i, *f)
                })
                .collect()
        };
        [pairs(0x4545_0010), pairs(0x4545_0011)]
    }

    /// What a read of the loose file `s` writes returns.
    fn loose(s: &Rpf9Sections) -> Rpf9Sections {
        let file = write_rpf9_resource(s).expect("written");
        let (find, bytes) = read_rpf9_body(Cursor::new(&file), 0, u64::MAX).expect("it reads");
        Rpf9Sections::of(&find, &bytes).expect("a resource")
    }

    #[test]
    fn an_array_at_its_blocks_end_grows_the_section_and_shrinks_back_to_the_same_bytes() {
        let original = two_arrays();
        let mut s = original.clone();
        insert_element(&mut s, 1, 0x10, 0, &element(-40, 1.5)).expect("inserted");
        insert_element(&mut s, 1, 0x10, 2, &element(50, -2.0)).expect("inserted");
        assert_eq!(
            decoded(&s),
            [
                vec![(10, 0.25), (-20, 0.5)],
                vec![(-40, 1.5), (30, 0.75), (50, -2.0)]
            ]
        );
        // The block grew from 24 to 40 bytes, past its 16-byte end by one row.
        assert_eq!(s.virtual_section.len(), original.virtual_section.len() + 16);
        assert_eq!(loose(&s), s);

        remove_element(&mut s, 1, 0x10, 2).expect("removed");
        remove_element(&mut s, 1, 0x10, 0).expect("removed");
        assert_eq!(s, original);
        assert_eq!(
            write_rpf9_resource(&s).expect("written"),
            write_rpf9_resource(&original).expect("written")
        );
    }

    #[test]
    fn a_full_array_another_region_follows_moves_to_its_blocks_end() {
        let mut s = two_arrays();
        insert_element(&mut s, 1, 0x00, 1, &element(5, 2.0)).expect("inserted");
        assert_eq!(
            decoded(&s),
            [vec![(10, 0.25), (5, 2.0), (-20, 0.5)], vec![(30, 0.75)]]
        );
        let model = read_model(&s.virtual_section, &s.physical).expect("it reads");
        let root = &model.data_blocks[0].data;
        assert_eq!(word(root, 0), pointer(2, 32));
        assert_eq!(root[8..12], [3, 0, 3, 0]);
        assert_eq!(model.data_blocks[1].data.len(), 32 + 3 * 8);
        assert_eq!(loose(&s), s);

        // The moved array fills its block's end, so a removal gives that room back.
        remove_element(&mut s, 1, 0x00, 0).expect("removed");
        assert_eq!(decoded(&s), [vec![(5, 2.0), (-20, 0.5)], vec![(30, 0.75)]]);
        assert_eq!(loose(&s), s);
    }

    /// A root whose one member is a counted array of two 8-byte structures, the end of their block.
    fn counted() -> Rpf9Sections {
        let root = (
            ROOT,
            0x10,
            vec![
                member(0x4545_0013, 0x00, COUNTED, 1, 0),
                member(ARRAY_INFO, 0, 0x05, 0, ELEMENT),
            ],
        );
        let pair = (
            ELEMENT,
            8,
            vec![
                member(0x4545_0020, 0, 0x14, 0, 0),
                member(0x4545_0021, 4, 0x21, 0, 0),
            ],
        );
        let mut data = vec![0u8; 0x10];
        data[0x00..0x08].copy_from_slice(&[2, 0, 0, 0, 2, 0, 0, 0]);
        data[0x08..0x10].copy_from_slice(&pointer(2, 0).to_le_bytes());
        let elements = [element(10, 0.25), element(-20, 0.5)].concat();
        body(&[root, pair], &[(ROOT, data), (ELEMENT, elements)], 1)
    }

    /// The counted array's structures, as the table form decodes them, and its head.
    fn counted_read(s: &Rpf9Sections) -> (Vec<(i64, f64)>, Vec<u8>) {
        let model = read_model(&s.virtual_section, &s.physical).expect("it reads");
        let root = table_form::decode(&model).expect("it decodes");
        let name = |hash: u32| default_hash_index().name_or_literal(hash);
        let Some(Value::Array(items)) = root.get(&name(0x4545_0013)) else {
            panic!("an array");
        };
        let pairs = (items.iter())
            .map(|item| {
                let Value::Struct(e) = item else {
                    panic!("a structure");
                };
                let (Some(Value::Int(i)), Some(Value::Float(f))) =
                    (e.get(&name(0x4545_0020)), e.get(&name(0x4545_0021)))
                else {
                    panic!("an element");
                };
                (*i, *f)
            })
            .collect();
        (pairs, model.data_blocks[0].data[..16].to_vec())
    }

    #[test]
    fn a_counted_array_keeps_its_capacity_at_its_count_as_it_grows_and_shrinks() {
        let original = counted();
        let mut s = original.clone();
        insert_element(&mut s, 1, 0, 1, &element(5, 2.0)).expect("inserted");
        let (pairs, head) = counted_read(&s);
        assert_eq!(pairs, vec![(10, 0.25), (5, 2.0), (-20, 0.5)]);
        assert_eq!(head[..8], [3, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(word(&head, 8), pointer(2, 0));
        assert_eq!(loose(&s), s);

        remove_element(&mut s, 1, 0, 0).expect("removed");
        let (pairs, head) = counted_read(&s);
        assert_eq!(pairs, vec![(5, 2.0), (-20, 0.5)]);
        assert_eq!(head[..8], [2, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(loose(&s), s);
    }

    #[test]
    fn an_edit_the_array_cannot_take_is_refused_and_changes_nothing() {
        let original = two_arrays();
        let mut s = original.clone();
        let cases = [
            (
                insert_element(&mut s, 1, 0x10, 2, &element(1, 1.0)),
                "element 2 is past the array's 1",
            ),
            (
                insert_element(&mut s, 1, 0x10, 0, &[0; 4]),
                "the element is 4 bytes, where the array's are 8",
            ),
            (
                insert_element(&mut s, 1, 0x20, 0, &element(1, 1.0)),
                "block 1 holds no AtArray64 member of its structure at 0x20",
            ),
            (
                insert_element(&mut s, 9, 0x10, 0, &element(1, 1.0)),
                "the resource has 2 data blocks, and no block 9",
            ),
            (
                remove_element(&mut s, 1, 0x10, 1),
                "element 1 is past the array's 1",
            ),
        ];
        for (result, want) in cases {
            let err = result.expect_err(want).to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
        assert_eq!(s, original);
    }
}
