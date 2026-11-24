//! GTA VI's headerless RBM resources, read only: GTA V's RBM header with one
//! word inserted before the table pointers, 16-byte enum values, and the
//! member layouts of GTA VI's PSO table form.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::meta::binary::layout::{ContainerLayout, MetaLayout, RbmLayout};
use crate::meta::binary::pso::table_form;
use crate::meta::binary::rbm::reader::{read_u16_le, read_u32_le, read_u64_le};
use crate::meta::binary::{BinaryFormat, RBM_PRD0_MARKER_LE};
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::MetaInstance;

use super::header::{RbmHeader, RBM_PRD0_MARKER};
use super::schema::{class_entry, class_header, RbmDataBlock, RbmEnum, RbmModel};

/// What stops a GTA VI headerless RBM resource from being read or saved.
#[derive(Debug, thiserror::Error)]
pub enum Gta6RbmError {
    #[error("GTA VI RBM: {0}")]
    Malformed(String),
    #[error("GTA VI RBM: a headerless resource cannot be saved, because it has no header to write back: the sizes and page flags a save states are in the archive's table, which is encrypted")]
    NoResourceHeader,
    #[error("GTA VI RBM: this resource is not saved, because the measured layout rules do not give back the bytes it was read from: {0}")]
    RulesMiss(String),
}

/// The virtual and physical sections of a headerless resource with `instance` written into the
/// blocks `layout` was read with, each region where it was.
#[cfg(feature = "gta6")]
pub fn write_sections(layout: &RbmLayout, instance: &MetaInstance) -> Result<(Vec<u8>, Vec<u8>)> {
    let _names = crate::meta::binary::hash_index::title_names();
    use crate::meta::binary::pso::table_layout::{
        free_namespace, lay_out_in_place, where_they_miss,
    };
    let model = &layout.model;
    let (laid, mut held) = lay_out_in_place(model, &table_form::decode(model)?, None)?;
    let virtual_section = (layout.decompressed)
        .get(..layout.virtual_page_size)
        .unwrap_or(layout.decompressed.as_slice());
    let described = described_namespaces(virtual_section)?;
    held.extend(
        (0..=u8::MAX)
            .filter(|ns| !described.contains(ns))
            .map(free_namespace),
    );
    let stored = (laid.blocks.iter())
        .zip(&model.data_blocks)
        .all(|((_, d), b)| *d == b.data);
    if !stored {
        return Err(Gta6RbmError::RulesMiss(where_they_miss(model, &laid)).into());
    }
    let (laid, _) = lay_out_in_place(model, instance, Some(&held))?;
    let mut bytes = layout.decompressed.clone();
    for ((_, data), block) in laid.blocks.iter().zip(&model.data_blocks) {
        let at = match block.data_addr >> 28 {
            5 => 0,
            _ => layout.virtual_page_size,
        } + (block.data_addr & 0x0FFF_FFFF) as usize;
        if let Some(dst) = bytes.get_mut(at..at + data.len()) {
            dst.copy_from_slice(data);
        }
    }
    let physical = bytes.split_off(layout.virtual_page_size.min(bytes.len()));
    Ok((bytes, physical))
}

/// A headerless resource with `instance` written into its blocks, as the loose file a read at
/// offset 0 opens.
#[cfg(feature = "gta6")]
pub fn write_loose(layout: &RbmLayout, instance: &MetaInstance) -> Result<Vec<u8>> {
    let _names = crate::meta::binary::hash_index::title_names();
    use crate::container::rpf::{write_rpf9_resource, Rpf9Form, Rpf9Sections};
    let (virtual_section, physical) = write_sections(layout, instance)?;
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section,
        physical,
    };
    write_rpf9_resource(&sections).map_err(|e| MetaError::Other(e.to_string()))
}

impl From<Gta6RbmError> for MetaError {
    fn from(error: Gta6RbmError) -> Self {
        MetaError::Other(error.to_string())
    }
}

fn malformed(why: String) -> MetaError {
    Gta6RbmError::Malformed(why).into()
}

/// The word at 0x14 that every measured GTA VI body states (GTA V's is 0x79).
pub const GTA6_HEADER_WORD: u16 = 0x7A;
const HEADER_LEN: usize = 0x60;
/// The header words every measured body holds zero, as (offset, width).
const ZERO_WORDS: [(usize, usize); 6] = [
    (0x00, 8),
    (0x18, 4),
    (0x1C, 4),
    (0x24, 4),
    (0x58, 4),
    (0x5C, 4),
];

/// What a headerless resource's header states besides its tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderFacts {
    /// Block map bytes +8 and +9: how many blocks the virtual and the physical section span.
    pub virtual_blocks: u8,
    pub physical_blocks: u8,
    /// The string table's entry count and blob bytes, `None` where 0x40 is null.
    // ⚠ The blobs are not read: their bytes are high-entropy and no key for them is known.
    pub strings: Option<(u32, u64)>,
    /// The word at 0x50, zero or a pointer into the virtual section at a table of 16-byte ids.
    pub word_50h: u64,
}

/// `section` with the trailing zeros a stored last piece leaves off, to a multiple of 16 bytes.
fn padded(section: &[u8]) -> Cow<'_, [u8]> {
    match section.len() % 16 {
        0 => Cow::Borrowed(section),
        short => {
            let mut whole = section.to_vec();
            whole.resize(section.len() + 16 - short, 0);
            Cow::Owned(whole)
        }
    }
}

/// The block map's counts, the string table's size and the word at 0x50 of a virtual section.
pub fn header_facts(virtual_section: &[u8]) -> Result<HeaderFacts> {
    let whole = padded(virtual_section);
    let virtual_section = &*whole;
    let head = header(virtual_section)?;
    let virtual_only = Sections {
        virtual_section,
        physical_section: &[],
    };
    let map = virtual_only.at(read_u64_le(head, 8)?, 10, "the block map")?;
    let strings = match read_u64_le(head, 0x40)? {
        0 => None,
        ptr => {
            let count = read_u32_le(virtual_only.at(ptr, 4, "the string table")?, 0)?;
            let entries = virtual_only.records(
                ptr.wrapping_add(4),
                count as usize,
                8,
                "the string table's entries",
            )?;
            let blobs: u64 = entries
                .as_chunks::<8>()
                .0
                .iter()
                .map(|e| u64::from(u32::from_le_bytes([e[4], e[5], e[6], e[7]])))
                .sum();
            let len = usize::try_from(4 + 8 * u64::from(count) + blobs).unwrap_or(usize::MAX);
            virtual_only.at(ptr, len, "the string table's blobs")?;
            Some((count, blobs))
        }
    };
    let word_50h = read_u64_le(head, 0x50)?;
    if word_50h != 0 && (word_50h >> 28 != 5 || virtual_only.at(word_50h, 0, "").is_err()) {
        return Err(malformed(format!(
            "the word at 0x50 is {word_50h:#X}, which is neither zero nor a pointer into the virtual section"
        )));
    }
    Ok(HeaderFacts {
        virtual_blocks: map[8],
        physical_blocks: map[9],
        strings,
        word_50h,
    })
}

/// The namespace of each entry of a virtual section's string table, none where 0x40 is null.
// An entry is its namespace byte, a kind byte, a u16 and its blob's length; no blob is read.
#[cfg(feature = "gta6")]
fn described_namespaces(virtual_section: &[u8]) -> Result<Vec<u8>> {
    let whole = padded(virtual_section);
    let virtual_section = &*whole;
    let head = header(virtual_section)?;
    let virtual_only = Sections {
        virtual_section,
        physical_section: &[],
    };
    match read_u64_le(head, 0x40)? {
        0 => Ok(Vec::new()),
        ptr => {
            let count = read_u32_le(virtual_only.at(ptr, 4, "the string table")?, 0)?;
            let entries = virtual_only.records(
                ptr.wrapping_add(4),
                count as usize,
                8,
                "the string table's entries",
            )?;
            Ok(entries.as_chunks::<8>().0.iter().map(|e| e[0]).collect())
        }
    }
}

fn header(virtual_section: &[u8]) -> Result<&[u8]> {
    virtual_section.get(..HEADER_LEN).ok_or_else(|| {
        malformed(format!(
            "the virtual section is {} bytes, shorter than the {HEADER_LEN:#X}-byte header",
            virtual_section.len()
        ))
    })
}

/// The two sections of a resource; a pointer's top nibble picks one.
struct Sections<'b> {
    virtual_section: &'b [u8],
    physical_section: &'b [u8],
}

impl<'b> Sections<'b> {
    /// `len` bytes at the resource address `ptr`: 0x5 is the virtual section, 0x6 the physical.
    fn at(&self, ptr: u64, len: usize, what: &str) -> Result<&'b [u8]> {
        let (section, name) = match ptr >> 28 {
            5 => (self.virtual_section, "virtual"),
            6 => (self.physical_section, "physical"),
            _ => {
                return Err(malformed(format!(
                "{what} is at {ptr:#X}, which is in neither the virtual nor the physical section"
            )))
            }
        };
        let at = (ptr & 0x0FFF_FFFF) as usize;
        at.checked_add(len)
            .and_then(|end| section.get(at..end))
            .ok_or_else(|| {
                malformed(format!(
                    "{what} ({len} bytes at {ptr:#X}) runs past the {}-byte {name} section",
                    section.len()
                ))
            })
    }

    fn records(&self, ptr: u64, count: usize, size: usize, what: &str) -> Result<&'b [u8]> {
        if count == 0 {
            return Ok(&[]);
        }
        let len = count
            .checked_mul(size)
            .ok_or_else(|| malformed(format!("{what} states {count} records")))?;
        self.at(ptr, len, what)
    }
}

/// The value tree and layout of a GTA VI headerless RBM resource, from its two inflated sections.
pub fn open(virtual_section: &[u8], physical_section: &[u8]) -> Result<(MetaInstance, MetaLayout)> {
    let _names = crate::meta::binary::hash_index::title_names();
    let (whole_virtual, whole_physical) = (padded(virtual_section), padded(physical_section));
    let (virtual_section, physical_section) = (&*whole_virtual, &*whole_physical);
    let model = read_model(virtual_section, physical_section)?;
    let head = header(virtual_section)?;
    let facts = header_facts(virtual_section)?;
    let instance = table_form::decode(&model)?;
    // The words that sit where GTA V's header has them are carried; GTA V's
    // name pointer has no GTA VI counterpart.
    let mut unknown_50h = [0u32; 8];
    unknown_50h[0] = facts.word_50h as u32;
    unknown_50h[1] = (facts.word_50h >> 32) as u32;
    let header = RbmHeader {
        vft: read_u64_le(head, 0)?,
        block_map_ptr: read_u64_le(head, 8)?,
        unknown_10h: RBM_PRD0_MARKER,
        unknown_14h: GTA6_HEADER_WORD,
        has_encrypted_strings: head[0x16],
        unknown_17h: head[0x17],
        unknown_18h: read_u32_le(head, 0x18)?,
        root_block_id: model.root_block_id,
        class_refs_ptr: read_u64_le(head, 0x28)?,
        enum_refs_ptr: read_u64_le(head, 0x30)?,
        data_blocks_ptr: read_u64_le(head, 0x38)?,
        name_ptr: 0,
        encrypted_strings_ptr: read_u64_le(head, 0x40)?,
        class_count: read_u16_le(head, 0x48)?,
        enum_count: read_u16_le(head, 0x4A)?,
        data_block_count: read_u16_le(head, 0x4C)?,
        unknown_4eh: read_u16_le(head, 0x4E)?,
        unknown_50h,
    };
    let layout = RbmLayout {
        header,
        model,
        decompressed: [virtual_section, physical_section].concat(),
        seal: None,
        virtual_page_size: virtual_section.len(),
        physical_page_size: physical_section.len(),
    };
    Ok((
        instance,
        MetaLayout {
            format: BinaryFormat::Rbm,
            container: ContainerLayout::Rbm(Box::new(layout)),
        },
    ))
}

/// The class table, enums and data blocks of a GTA VI headerless RBM resource, read without
/// decoding its value tree.
pub fn read_model(virtual_section: &[u8], physical_section: &[u8]) -> Result<RbmModel> {
    let _names = crate::meta::binary::hash_index::title_names();
    // ⚠ Where each section ends is the caller's to say: a section is the sum
    // of the blocks the block map counts, and no word of this header states
    // its length.
    let (whole_virtual, whole_physical) = (padded(virtual_section), padded(physical_section));
    let (virtual_section, physical_section) = (&*whole_virtual, &*whole_physical);
    let head = header(virtual_section)?;
    if head[0x10..0x14] != RBM_PRD0_MARKER_LE {
        return Err(malformed(
            "the header does not hold the PRD0 marker at 0x10".into(),
        ));
    }
    let word = read_u16_le(head, 0x14)?;
    if word != GTA6_HEADER_WORD {
        return Err(malformed(format!(
            "the header states {word:#X} at 0x14, where every measured GTA VI body states {GTA6_HEADER_WORD:#X}"
        )));
    }
    for (at, width) in ZERO_WORDS {
        let value = match width {
            8 => read_u64_le(head, at)?,
            _ => u64::from(read_u32_le(head, at)?),
        };
        if value != 0 {
            return Err(malformed(format!(
                "the header holds {value:#X} at {at:#X}, where every measured body holds 0"
            )));
        }
    }
    let facts = header_facts(virtual_section)?;
    let sections = Sections {
        virtual_section,
        physical_section,
    };
    let root_block_id = read_u32_le(head, 0x20)?;
    let class_ptr = read_u64_le(head, 0x28)?;
    let enum_ptr = read_u64_le(head, 0x30)?;
    let block_ptr = read_u64_le(head, 0x38)?;
    let class_count = read_u16_le(head, 0x48)? as usize;
    let enum_count = read_u16_le(head, 0x4A)? as usize;
    let block_count = read_u16_le(head, 0x4C)? as usize;

    let mut classes = HashMap::with_capacity(class_count);
    let mut class_order = Vec::with_capacity(class_count);
    let table = sections.records(class_ptr, class_count, 32, "the class table")?;
    for record in table.as_chunks::<32>().0 {
        let (mut class, members_ptr, member_count) = class_header(record)?;
        let members =
            sections.records(members_ptr, member_count.into(), 16, "a class's members")?;
        for member in members.as_chunks::<16>().0 {
            class.entries.push(class_entry(member)?);
        }
        class_order.push(class.name_hash);
        if let Some(twice) = classes.insert(class.name_hash, class) {
            return Err(malformed(format!(
                "the class table declares structure {:#010X} twice",
                twice.name_hash
            )));
        }
    }

    let mut enums = HashMap::with_capacity(enum_count);
    let mut enum_order = Vec::with_capacity(enum_count);
    let table = sections.records(enum_ptr, enum_count, 24, "the enum table")?;
    for record in table.as_chunks::<24>().0 {
        let name_hash = read_u32_le(record, 0)?;
        let value_count = read_u32_le(record, 0x10)? as usize;
        let records =
            sections.records(read_u64_le(record, 8)?, value_count, 16, "an enum's values")?;
        let values = records
            .as_chunks::<16>()
            .0
            .iter()
            .map(|v| {
                let last = read_u32_le(v, 12)?;
                if last != 0 {
                    return Err(malformed(format!(
                        "a value of enum {name_hash:#010X} holds {last:#X} at +0xC, where every measured one holds 0"
                    )));
                }
                table_form::enum_value(v, name_hash)
            })
            .collect::<Result<Vec<_>>>()?;
        enum_order.push(name_hash);
        let declared = RbmEnum {
            name_hash,
            signature: read_u32_le(record, 4)?,
            flags: 0,
            values,
        };
        if enums.insert(name_hash, declared).is_some() {
            return Err(malformed(format!(
                "the enum table declares enum {name_hash:#010X} twice"
            )));
        }
    }

    let table = sections.records(block_ptr, block_count, 16, "the data-block table")?;
    // ⚠ Each block is copied: blocks that overlap would copy the sections many times over.
    let mut stated = 0u64;
    for record in table.as_chunks::<16>().0 {
        stated += u64::from(read_u32_le(record, 4)?);
    }
    let held = (virtual_section.len() + physical_section.len()) as u64;
    if stated > held {
        return Err(malformed(format!(
            "the data blocks state {stated} bytes, more than the {held} bytes of the two sections"
        )));
    }
    let data_blocks = table
        .as_chunks::<16>()
        .0
        .iter()
        .enumerate()
        .map(|(i, record)| {
            let size = read_u32_le(record, 4)? as usize;
            let data_addr = read_u64_le(record, 8)?;
            let data = match size {
                0 => Vec::new(),
                _ => sections
                    .at(data_addr, size, &format!("data block {}", i + 1))?
                    .to_vec(),
            };
            Ok(RbmDataBlock {
                data_type: read_u32_le(record, 0)?,
                data,
                data_addr,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let physical = data_blocks
        .iter()
        .filter(|b| !b.data.is_empty() && b.data_addr >> 28 == 6)
        .count();
    if (facts.physical_blocks != 0) != (physical != 0) {
        return Err(malformed(format!(
            "the block map counts {} physical block(s), where {physical} data block(s) are in the physical section",
            facts.physical_blocks
        )));
    }

    Ok(RbmModel {
        classes,
        enums,
        data_blocks,
        root_block_id,
        class_order,
        enum_order,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::binary::enum_text;
    use crate::meta::binary::hash_index::default_hash_index;
    use crate::meta::binary::write_meta_binary;
    use crate::meta::instance::value::Value;

    const ROOT: u32 = 0x4444_0001;
    const ELEMENT: u32 = 0x4444_0002;
    const KIND: u32 = 0x4444_0003;
    const VIRTUAL: u64 = 0x5000_0000;
    const PHYSICAL: u64 = 0x6000_0000;

    fn member(name: u32, offset: u32, code: u8, u10: u16, u12: u32) -> [u8; 16] {
        let mut m = [0u8; 16];
        m[0..4].copy_from_slice(&name.to_le_bytes());
        m[4..8].copy_from_slice(&offset.to_le_bytes());
        m[8] = code;
        m[10..12].copy_from_slice(&u10.to_le_bytes());
        m[12..16].copy_from_slice(&u12.to_le_bytes());
        m
    }

    /// A packed pointer inside a data block: id in bits 0 to 11, offset from bit 20.
    fn pointer(block: u64, offset: u64) -> u64 {
        block | offset << 20
    }

    struct Body {
        virtual_section: Vec<u8>,
        physical_section: Vec<u8>,
    }

    /// Where each table and block of [`body`] starts in the virtual section.
    const CLASSES: usize = 0x60;
    const MEMBERS: usize = 0xA0;
    const ENUMS: usize = 0x160;
    const VALUES: usize = 0x180;
    const BLOCKS: usize = 0x1C0;
    const ROOT_DATA: usize = 0x200;
    const ELEMENTS: usize = 0x240;
    const BLOCK_MAP: usize = 0x250;

    /// A body laid out as the measured ones are: the root block and its
    /// array in the virtual section, the pointed-to element in the physical.
    fn body() -> Body {
        let mut v = vec![0u8; 0x260];
        let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x08, &(VIRTUAL + BLOCK_MAP as u64).to_le_bytes());
        put(BLOCK_MAP + 8, &[1, 1]);
        put(0x10, &RBM_PRD0_MARKER_LE);
        put(0x14, &GTA6_HEADER_WORD.to_le_bytes());
        put(0x20, &1u32.to_le_bytes());
        put(0x28, &(VIRTUAL + CLASSES as u64).to_le_bytes());
        put(0x30, &(VIRTUAL + ENUMS as u64).to_le_bytes());
        put(0x38, &(VIRTUAL + BLOCKS as u64).to_le_bytes());
        put(0x48, &[2, 0, 1, 0, 3, 0, 5, 0]);

        let root = [
            member(0x5555_0000, 0x00, 0x15, 0, 0),
            member(0x5555_0001, 0x04, 0x20, 0, 0),
            member(0x5555_0002, 0x08, 0x62, 0, KIND),
            member(0x5555_0003, 0x10, 0x5C, 4, 0),
            member(ARRAY_INFO, 0, 0x05, 0, ELEMENT),
            member(0x5555_0005, 0x20, 0x07, 0, 0),
            member(0x5555_0006, 0x28, 0x4A, 0, 0),
        ];
        let element = [
            member(0x5555_0010, 0, 0x14, 0, 0),
            member(0x5555_0011, 4, 0x21, 0, 0),
        ];
        let class = |name: u32, members_at: usize, count: usize, size: u32| {
            let mut c = [0u8; 32];
            c[0..4].copy_from_slice(&name.to_le_bytes());
            c[4..8].copy_from_slice(&0x5151_5151u32.to_le_bytes());
            c[9] = 3;
            c[16..24].copy_from_slice(&(VIRTUAL + members_at as u64).to_le_bytes());
            c[24..28].copy_from_slice(&size.to_le_bytes());
            c[30..32].copy_from_slice(&(count as u16).to_le_bytes());
            c
        };
        put(CLASSES, &class(ROOT, MEMBERS, root.len(), 0x30));
        put(
            CLASSES + 32,
            &class(ELEMENT, MEMBERS + 16 * root.len(), element.len(), 8),
        );
        put(MEMBERS, &root.concat());
        put(MEMBERS + 16 * root.len(), &element.concat());

        put(ENUMS, &KIND.to_le_bytes());
        put(ENUMS + 4, &0x6161_6161u32.to_le_bytes());
        put(ENUMS + 8, &(VIRTUAL + VALUES as u64).to_le_bytes());
        put(ENUMS + 0x10, &3u32.to_le_bytes());
        for (i, (value, name)) in [
            (0i64, 0x7777_0000u32),
            (-1, 0x7777_0001),
            (0xB53A_C261, 0x7777_0002),
        ]
        .iter()
        .enumerate()
        {
            let at = VALUES + 16 * i;
            put(at, &value.to_le_bytes());
            put(at + 8, &name.to_le_bytes());
        }

        let block = |kind: u32, size: u32, at: u64| -> Vec<u8> {
            [kind.to_le_bytes(), size.to_le_bytes()]
                .concat()
                .into_iter()
                .chain(at.to_le_bytes())
                .collect()
        };
        put(BLOCKS, &block(ROOT, 0x30, VIRTUAL + ROOT_DATA as u64));
        put(BLOCKS + 16, &block(ELEMENT, 16, VIRTUAL + ELEMENTS as u64));
        put(BLOCKS + 32, &block(ELEMENT, 16, PHYSICAL + 0x10));

        put(ROOT_DATA, &77u32.to_le_bytes());
        put(ROOT_DATA + 4, &crate::f32_to_half(2.5).to_le_bytes());
        put(ROOT_DATA + 8, &(-1i32).to_le_bytes());
        put(ROOT_DATA + 0x10, &[2, 0, 0, 0, 2, 0, 0, 0]);
        put(ROOT_DATA + 0x18, &pointer(2, 0).to_le_bytes());
        put(ROOT_DATA + 0x20, &pointer(3, 8).to_le_bytes());
        put(ROOT_DATA + 0x28, &crate::string_hash(b"name").to_le_bytes());
        let elements = |pairs: &[(i32, f32)]| -> Vec<u8> {
            pairs
                .iter()
                .flat_map(|(i, f)| [i.to_le_bytes(), f.to_le_bytes()].concat())
                .collect()
        };
        put(ELEMENTS, &elements(&[(10, 0.25), (-20, 0.5)]));
        let mut p = vec![0u8; 0x20];
        p[0x10..0x20].copy_from_slice(&elements(&[(0, 0.0), (30, 0.75)]));
        Body {
            virtual_section: v,
            physical_section: p,
        }
    }

    const ARRAY_INFO: u32 = 0x100;

    fn name(hash: u32) -> String {
        default_hash_index().name_or_literal(hash)
    }

    fn hashed(hash: u32) -> crate::meta::instance::value::HashedString {
        crate::meta::binary::pso::data_names::DataNames::of_tree().hashed(hash)
    }

    fn element(i: i64, f: f64) -> MetaInstance {
        let mut e = MetaInstance::new(name(ELEMENT));
        e.fields.push((name(0x5555_0010), Value::Int(i)));
        e.fields.push((name(0x5555_0011), Value::Float(f)));
        e
    }

    fn open_body(b: &Body) -> Result<(MetaInstance, MetaLayout)> {
        open(&b.virtual_section, &b.physical_section)
    }

    #[test]
    fn a_headerless_body_reads_back_to_the_values_it_was_built_with() {
        let (instance, layout) = open_body(&body()).expect("it reads");
        let mut expected = MetaInstance::new(name(ROOT));
        for (hash, value) in [
            (0x5555_0000, Value::UInt(77)),
            (0x5555_0001, Value::Float(2.5)),
            (
                0x5555_0002,
                Value::Enum {
                    type_name: name(KIND),
                    variant: enum_text::variant_text(0x7777_0001),
                },
            ),
            (
                0x5555_0003,
                Value::Array(vec![
                    Value::Struct(Box::new(element(10, 0.25))),
                    Value::Struct(Box::new(element(-20, 0.5))),
                ]),
            ),
            (
                0x5555_0005,
                Value::Pointer(Some(Box::new(element(30, 0.75)))),
            ),
            (
                0x5555_0006,
                Value::Hash(hashed(crate::string_hash(b"name"))),
            ),
        ] {
            expected.fields.push((name(hash), value));
        }
        assert_eq!(instance, expected);
        assert_eq!(layout.format, BinaryFormat::Rbm);
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            panic!("an RBM layout");
        };
        assert!(rbm.seal.is_none());
        assert_eq!(rbm.model.data_blocks[2].data_addr, PHYSICAL + 0x10);
    }

    #[test]
    fn an_enum_value_above_i32_max_names_the_member_that_stores_its_32_bits() {
        let mut b = body();
        b.virtual_section[ROOT_DATA + 8..ROOT_DATA + 12]
            .copy_from_slice(&0xB53A_C261u32.to_le_bytes());
        let (instance, _) = open_body(&b).expect("it reads");
        assert_eq!(
            instance.fields[2].1,
            Value::Enum {
                type_name: name(KIND),
                variant: enum_text::variant_text(0x7777_0002),
            }
        );
    }

    #[cfg(feature = "gta6")]
    #[test]
    fn a_headerless_body_saves_into_its_blocks_and_refuses_a_region_that_changes_size() {
        let b = body();
        let (mut instance, layout) = open_body(&b).expect("it reads");
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            panic!("an RBM layout");
        };
        let sections = write_sections(rbm, &instance).expect("it writes");
        assert_eq!(
            sections,
            (b.virtual_section.clone(), b.physical_section.clone())
        );

        instance.fields[0].1 = Value::UInt(78);
        let Value::Pointer(Some(pointed)) = &mut instance.fields[4].1 else {
            panic!("a pointer");
        };
        pointed.fields[1].1 = Value::Float(-1.25);
        let (v, p) = write_sections(rbm, &instance).expect("it writes");
        assert_ne!(v, b.virtual_section);
        assert_ne!(p, b.physical_section);
        let (again, layout) = open(&v, &p).expect("it reads back");
        assert_eq!(again, instance);
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            panic!("an RBM layout");
        };
        assert_eq!(write_sections(rbm, &again).expect("it writes"), (v, p));

        let Value::Array(items) = &mut instance.fields[3].1 else {
            panic!("an array");
        };
        items.push(items[0].clone());
        let err = write_sections(rbm, &instance)
            .expect_err("refused")
            .to_string();
        assert!(
            err.contains("saved only into the allocation it arrived with"),
            "{err}"
        );
    }

    #[cfg(feature = "gta6")]
    #[test]
    fn a_new_64_bit_hash_is_written_only_where_the_string_table_holds_no_entry_of_its_namespace() {
        let mut b = body();
        // The root's last member as a 64-bit hash of namespace 5.
        let at = MEMBERS + 16 * 6;
        b.virtual_section[at..at + 16].copy_from_slice(&member(0x5555_0006, 0x28, 0x4F, 0, 0));
        b.virtual_section[at + 9] = 5;
        let word = 0x1111_2222_3333_4444u64;
        b.virtual_section[ROOT_DATA + 0x28..ROOT_DATA + 0x30].copy_from_slice(&word.to_le_bytes());
        let other = 0x5555_6666_7777_8888u64;
        let edit = |b: &Body| -> Result<(Vec<u8>, Vec<u8>)> {
            let (mut instance, layout) = open_body(b)?;
            assert_eq!(instance.fields[5].1, Value::UInt(word));
            instance.fields[5].1 = Value::UInt(other);
            let ContainerLayout::Rbm(rbm) = &layout.container else {
                panic!("an RBM layout");
            };
            write_sections(rbm, &instance)
        };
        let (v, p) = edit(&b).expect("no string table, so no text of namespace 5");
        let (again, _) = open(&v, &p).expect("it reads back");
        assert_eq!(again.fields[5].1, Value::UInt(other));

        // One entry of namespace 5 and 3 blob bytes, after the block map.
        let end = b.virtual_section.len();
        let table = [
            1u32.to_le_bytes().as_slice(),
            &[5, 1, 0, 0],
            &3u32.to_le_bytes(),
            &[0xA5; 4],
        ]
        .concat();
        b.virtual_section.extend_from_slice(&table);
        b.virtual_section[0x40..0x48].copy_from_slice(&(VIRTUAL + end as u64).to_le_bytes());
        let err = edit(&b).expect_err("refused").to_string();
        assert!(err.contains("which the document did not hold"), "{err}");
        b.virtual_section[end + 4] = 7;
        assert!(edit(&b).is_ok(), "an entry of another namespace");
    }

    #[test]
    fn a_headerless_body_refuses_what_it_cannot_save_and_says_why() {
        let (instance, layout) = open_body(&body()).expect("it reads");
        let want = "a headerless resource cannot be saved, because it has no header to write back";
        if cfg!(not(feature = "gta6")) {
            let err = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &instance)
                .expect_err("refused")
                .to_string();
            assert!(err.contains(want), "{err}");
        }
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            panic!("an RBM layout");
        };
        let err = super::super::writer::write_with_coverage(rbm, &instance)
            .expect_err("refused")
            .to_string();
        assert!(err.contains(want), "{err}");
    }

    #[test]
    fn the_header_states_its_block_counts_its_string_table_and_the_word_at_0x50() {
        let mut b = body();
        assert_eq!(
            header_facts(&b.virtual_section).expect("it reads"),
            HeaderFacts {
                virtual_blocks: 1,
                physical_blocks: 1,
                strings: None,
                word_50h: 0,
            }
        );
        // Two entries of 3 and 21 blob bytes, after the block map.
        let table = [
            2u32.to_le_bytes().as_slice(),
            &[1, 0, 0, 0],
            &3u32.to_le_bytes(),
            &[2, 0, 0, 0],
            &21u32.to_le_bytes(),
            &[0xA5; 24],
        ]
        .concat();
        let at = b.virtual_section.len();
        b.virtual_section.extend_from_slice(&table);
        let v = &mut b.virtual_section;
        v[0x40..0x48].copy_from_slice(&(VIRTUAL + at as u64).to_le_bytes());
        v[0x50..0x58].copy_from_slice(&(VIRTUAL + ROOT_DATA as u64).to_le_bytes());
        let facts = header_facts(v).expect("it reads");
        assert_eq!(facts.strings, Some((2, 24)));
        assert_eq!(facts.word_50h, VIRTUAL + ROOT_DATA as u64);
        let (_, layout) = open_body(&b).expect("it reads");
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            panic!("an RBM layout");
        };
        assert_eq!(
            rbm.header.unknown_50h[0],
            (VIRTUAL + ROOT_DATA as u64) as u32
        );

        // A section short of its last 16-byte line reads as if zeros filled it, so the table's
        // last 16 bytes are cut.
        let len = b.virtual_section.len();
        b.virtual_section.truncate(len - 16);
        let err = open_body(&b).expect_err("refused").to_string();
        assert!(err.contains("the string table's blobs ("), "{err}");
    }

    #[test]
    fn a_body_that_breaks_a_measured_relation_is_refused_with_the_relation() {
        type Patch = Box<dyn Fn(&mut Body)>;
        let set = |at: usize, word: Vec<u8>| -> Patch {
            Box::new(move |b: &mut Body| {
                b.virtual_section[at..at + word.len()].copy_from_slice(&word)
            })
        };
        let cases: Vec<(Patch, &str)> = vec![
            (
                set(0x14, 0x79u16.to_le_bytes().to_vec()),
                "states 0x79 at 0x14, where every measured GTA VI body states 0x7A",
            ),
            (set(0x10, b"PSIN".to_vec()), "PRD0 marker"),
            (
                set(0x1C, 1u32.to_le_bytes().to_vec()),
                "holds 0x1 at 0x1C, where every measured body holds 0",
            ),
            (set(0x24, 1u32.to_le_bytes().to_vec()), "holds 0x1 at 0x24"),
            (set(0x00, 1u64.to_le_bytes().to_vec()), "holds 0x1 at 0x0,"),
            (set(0x18, 1u32.to_le_bytes().to_vec()), "holds 0x1 at 0x18"),
            (set(0x58, 1u32.to_le_bytes().to_vec()), "holds 0x1 at 0x58"),
            (
                set(BLOCK_MAP + 9, vec![0]),
                "the block map counts 0 physical block(s), where 1 data block(s) are in the physical section",
            ),
            (
                set(0x08, (VIRTUAL + 0x300).to_le_bytes().to_vec()),
                "the block map (10 bytes at 0x50000300) runs past",
            ),
            (
                set(0x50, 0x6000_0000u64.to_le_bytes().to_vec()),
                "the word at 0x50 is 0x60000000, which is neither zero nor a pointer into the virtual section",
            ),
            (
                set(VALUES + 16 + 12, 1u32.to_le_bytes().to_vec()),
                "holds 0x1 at +0xC, where every measured one holds 0",
            ),
            (
                Box::new(|b: &mut Body| b.physical_section.clear()),
                "data block 3 (16 bytes at 0x60000010) runs past the 0-byte physical section",
            ),
            (
                set(0x38, 0x7000_0000u64.to_le_bytes().to_vec()),
                "the data-block table is at 0x70000000, which is in neither",
            ),
            (
                set(0x48, u16::MAX.to_le_bytes().to_vec()),
                "the class table (2097120 bytes at 0x50000060) runs past",
            ),
            (
                set(ENUMS + 0x10, u32::MAX.to_le_bytes().to_vec()),
                "an enum's values (",
            ),
            (
                set(VALUES + 16 + 4, 1u32.to_le_bytes().to_vec()),
                "is 8589934591, more than the 32 bits a member holds",
            ),
            (
                set(CLASSES + 32, ROOT.to_le_bytes().to_vec()),
                "declares structure 0x44440001 twice",
            ),
            (
                set(0x20, 9u32.to_le_bytes().to_vec()),
                "the root names block 9 of 3",
            ),
            (
                set(
                    ROOT_DATA + 0x20,
                    (pointer(3, 0) | 1 << 12).to_le_bytes().to_vec(),
                ),
                "sets bits 12 to 19",
            ),
            (
                // Every block names the whole virtual section, as no two do in a resource.
                Box::new(|b: &mut Body| {
                    let len = b.virtual_section.len() as u32;
                    for i in 0..3 {
                        let at = BLOCKS + 16 * i;
                        b.virtual_section[at + 4..at + 8].copy_from_slice(&len.to_le_bytes());
                        b.virtual_section[at + 8..at + 16].copy_from_slice(&VIRTUAL.to_le_bytes());
                    }
                }),
                "bytes of the two sections",
            ),
        ];
        for (patch, want) in cases {
            let mut b = body();
            patch(&mut b);
            let err = open_body(&b).expect_err(want).to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn a_type_code_whose_layout_is_not_known_is_refused_by_name() {
        let mut b = body();
        b.virtual_section[MEMBERS + 8] = 0x0E;
        let err = open_body(&b).expect_err("refused").to_string();
        let want = format!(
            "member {} of {} has type 0x0E",
            name(0x5555_0000),
            name(ROOT)
        );
        assert!(err.contains(&want), "{err}");
    }

    #[test]
    fn a_tagged_pointer_or_array_with_bit_63_clear_is_refused() {
        // Member 0 at 0x00 holds 77 and a half float, so neither bit 63 nor count = capacity.
        for (code, want) in [
            (
                0x0A,
                "has bit 63 clear, where every measured tagged pointer sets it",
            ),
            (0x5E, "where every measured tagged array fills its room"),
        ] {
            let mut b = body();
            b.virtual_section[MEMBERS + 8] = code;
            let err = open_body(&b).expect_err(want).to_string();
            assert!(err.contains(want), "{code:#04X}: {err}");
        }
    }

    #[test]
    fn a_layout_the_table_form_reads_reads_here_too() {
        let mut b = body();
        b.virtual_section[MEMBERS + 8] = 0x08;
        let (instance, _) = open_body(&b).expect("it reads");
        let half = u64::from(crate::f32_to_half(2.5));
        assert_eq!(instance.fields[0].1, Value::UInt(77 | half << 32));
    }

    #[test]
    fn every_truncation_is_an_error_and_never_a_panic() {
        let b = body();
        for len in 0..b.virtual_section.len().min(ELEMENTS + 16) {
            assert!(
                open(&b.virtual_section[..len], &b.physical_section).is_err(),
                "{len} virtual bytes read"
            );
        }
        // A section short of only its last 16-byte line reads with zeros there.
        let whole = b.physical_section.len();
        for len in (0..whole).filter(|len| len.next_multiple_of(16) < whole) {
            assert!(
                open(&b.virtual_section, &b.physical_section[..len]).is_err(),
                "{len} physical bytes read"
            );
        }
    }

    #[test]
    fn a_section_whose_stored_last_piece_lacks_its_trailing_zeros_reads_as_if_they_were_there() {
        let b = body();
        let (whole, _) = open_body(&b).expect("it reads");
        // The physical block ends with 0.75's four bytes; one is dropped and read back as zero.
        let mut p = b.physical_section.clone();
        let last = p.pop().expect("a byte");
        let (short, layout) = open(&b.virtual_section, &p).expect("it reads");
        let Value::Pointer(Some(pointed)) = &short.fields[4].1 else {
            panic!("a pointer");
        };
        let mut bytes = 0.75f32.to_le_bytes();
        assert_eq!(bytes[3], last);
        bytes[3] = 0;
        assert_eq!(
            pointed.fields[1].1,
            Value::Float(f64::from(f32::from_le_bytes(bytes)))
        );
        assert_ne!(short, whole);
        let ContainerLayout::Rbm(rbm) = &layout.container else {
            panic!("an RBM layout");
        };
        assert_eq!(rbm.physical_page_size, b.physical_section.len());
    }
}
