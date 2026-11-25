use std::collections::HashMap;
use std::io::Cursor;

use byteorder::{BigEndian, ReadBytesExt};

use crate::meta::error::{MetaError, Result};

use super::sections::read_section_header;

/// The engine's canonical `parMemberType` numbering, which a P-form member's type byte already is:
/// a float is `0x07` and a string `0x0B` (a hashed string carries subtype `0x07`), where the L-form
/// table (`PsoType`) instead marks the same two engine types `0x21` and `0x4A`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PsoDataType {
    Bool,
    SByte,
    UByte,
    SShort,
    UShort,
    SInt,
    UInt,
    Float,
    Vector2,
    Vector3,
    Vector4,
    String,
    Structure,
    Array,
    Enum,
    Bitset,
    Map,
    Matrix34,
    Matrix44,
    Vec2V,
    Vec3V,
    Vec4V,
    Mat33V,
    Mat34V,
    Mat44V,
    ScalarV,
    BoolV,
    VecBoolV,
    PtrDiffT,
    SizeT,
    Float16,
    Int64,
    UInt64,
    Double,
    Unknown(u8),
}

impl PsoDataType {
    pub(crate) fn from_byte(b: u8) -> Self {
        use PsoDataType::*;
        match b {
            0x00 => Bool,
            0x01 => SByte,
            0x02 => UByte,
            0x03 => SShort,
            0x04 => UShort,
            0x05 => SInt,
            0x06 => UInt,
            0x07 => Float,
            0x08 => Vector2,
            0x09 => Vector3,
            0x0a => Vector4,
            0x0b => String,
            0x0c => Structure,
            0x0d => Array,
            0x0e => Enum,
            0x0f => Bitset,
            0x10 => Map,
            0x11 => Matrix34,
            0x12 => Matrix44,
            0x13 => Vec2V,
            0x14 => Vec3V,
            0x15 => Vec4V,
            0x16 => Mat33V,
            0x17 => Mat34V,
            0x18 => Mat44V,
            0x19 => ScalarV,
            0x1a => BoolV,
            0x1b => VecBoolV,
            0x1c => PtrDiffT,
            0x1d => SizeT,
            0x1e => Float16,
            0x1f => Int64,
            0x20 => UInt64,
            0x21 => Double,
            other => Unknown(other),
        }
    }

    pub(crate) fn to_byte(self) -> u8 {
        use PsoDataType::*;
        match self {
            Bool => 0x00,
            SByte => 0x01,
            UByte => 0x02,
            SShort => 0x03,
            UShort => 0x04,
            SInt => 0x05,
            UInt => 0x06,
            Float => 0x07,
            Vector2 => 0x08,
            Vector3 => 0x09,
            Vector4 => 0x0a,
            String => 0x0b,
            Structure => 0x0c,
            Array => 0x0d,
            Enum => 0x0e,
            Bitset => 0x0f,
            Map => 0x10,
            Matrix34 => 0x11,
            Matrix44 => 0x12,
            Vec2V => 0x13,
            Vec3V => 0x14,
            Vec4V => 0x15,
            Mat33V => 0x16,
            Mat34V => 0x17,
            Mat44V => 0x18,
            ScalarV => 0x19,
            BoolV => 0x1a,
            VecBoolV => 0x1b,
            PtrDiffT => 0x1c,
            SizeT => 0x1d,
            Float16 => 0x1e,
            Int64 => 0x1f,
            UInt64 => 0x20,
            Double => 0x21,
            Unknown(b) => b,
        }
    }

    pub fn fixed_size(self) -> Option<usize> {
        use PsoDataType::*;
        Some(match self {
            Bool | SByte | UByte => 1,
            SShort | UShort | Float16 => 2,
            SInt | UInt | Float => 4,
            Vector2 => 8,
            PtrDiffT | SizeT | Int64 | UInt64 | Double => 8,
            Vector3 | Vector4 | Vec2V | Vec3V | Vec4V | ScalarV | BoolV | VecBoolV => 16,
            Mat33V => 48,
            Matrix34 | Matrix44 | Mat34V | Mat44V => 64,
            String | Structure | Array | Enum | Bitset | Map | Unknown(_) => return None,
        })
    }

    pub fn fixed_align(self) -> Option<usize> {
        use PsoDataType::*;
        Some(match self {
            Bool | SByte | UByte => 1,
            SShort | UShort | Float16 => 2,
            SInt | UInt | Float | Vector2 => 4,
            PtrDiffT | SizeT | Int64 | UInt64 | Double => 8,
            Vector3 | Vector4 | Vec2V | Vec3V | Vec4V | ScalarV | BoolV | VecBoolV | Mat33V
            | Matrix34 | Matrix44 | Mat34V | Mat44V => 16,
            String | Structure | Array | Enum | Bitset | Map | Unknown(_) => return None,
        })
    }
}

pub mod string_subtype {
    pub const EMBEDDED_BUFFER: u8 = 0;
    pub const POINTER: u8 = 1;
    pub const POINTER_ALT: u8 = 2;
    pub const CHAR_ARRAY: u8 = 3;
    pub const HASH: u8 = 7;
    pub const HASH_ALT: u8 = 8;
}

pub mod struct_subtype {
    pub const EMBEDDED: u8 = 0;
    pub const POINTER: u8 = 3;
    pub const POINTER_ALT: u8 = 4;
}

pub const POINTER_BYTES: usize = 8;
pub const ARRAY_HEADER_BYTES: usize = 16;
pub const STRUCT_ALIGN: usize = 16;

/// The stride between two array elements described by `elem`.
///
/// ⚠ The reader, the writer and the synthesiser must agree on this to the
/// byte: an allocation sized by one rule and read back by another is off by a
/// stride and reads a neighbour's bytes.
pub fn element_stride(schema: &PsoSchemaSection, elem: &PsoStructEntry) -> Option<usize> {
    match elem.data_type {
        PsoDataType::Structure => match elem.subtype() {
            struct_subtype::EMBEDDED => schema
                .struct_for(elem.referent_hash())
                .map(|s| s.data_size as usize),
            struct_subtype::POINTER | struct_subtype::POINTER_ALT => Some(POINTER_BYTES),
            _ => None,
        },
        PsoDataType::String => match elem.subtype() {
            string_subtype::EMBEDDED_BUFFER => Some(elem.count() as usize),
            string_subtype::POINTER | string_subtype::POINTER_ALT => Some(POINTER_BYTES),
            string_subtype::CHAR_ARRAY => Some(ARRAY_HEADER_BYTES),
            string_subtype::HASH | string_subtype::HASH_ALT => Some(4),
            // Only GTA VI states these: a hash in place of 4 and of 8 bytes (meta-grow Q12).
            super::reader::GTA6_STRING_HASH => Some(4),
            super::reader::GTA6_STRING_HASH64 => Some(8),
            _ => None,
        },
        PsoDataType::Enum => Some(if elem.subtype() == 2 { 1 } else { 4 }),
        PsoDataType::Bitset => Some(super::reader::bitset_byte_width(elem.count())),
        PsoDataType::Array | PsoDataType::Map => Some(ARRAY_HEADER_BYTES),
        other => other.fixed_size(),
    }
}

/// The boundary an allocation of `elem` has to start on.
pub fn element_align(elem: &PsoStructEntry) -> usize {
    match elem.data_type {
        PsoDataType::Structure => match elem.subtype() {
            struct_subtype::EMBEDDED => STRUCT_ALIGN,
            _ => POINTER_BYTES,
        },
        PsoDataType::String => match elem.subtype() {
            string_subtype::EMBEDDED_BUFFER => 1,
            string_subtype::HASH | string_subtype::HASH_ALT | super::reader::GTA6_STRING_HASH => 4,
            _ => POINTER_BYTES,
        },
        PsoDataType::Enum => {
            if elem.subtype() == 2 {
                1
            } else {
                4
            }
        }
        PsoDataType::Bitset => super::reader::bitset_byte_width(elem.count()).clamp(1, 8),
        PsoDataType::Array | PsoDataType::Map => POINTER_BYTES,
        other => other.fixed_align().unwrap_or(1),
    }
}

/// The alignment and the stride of one key/value pair of a map.
pub fn map_pair_layout(key_stride: usize, value_stride: usize) -> (usize, usize) {
    let align = key_stride.max(value_stride).clamp(1, 16);
    (align, align_up(key_stride + value_stride, align))
}

pub(crate) fn align_up(value: usize, align: usize) -> usize {
    value.next_multiple_of(align.max(1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PsoStructEntry {
    pub name_hash: u32,
    pub data_type: PsoDataType,
    offset_high_and_subtype: u8,
    offset_low: u16,
    reference_key: u32,
}

impl PsoStructEntry {
    pub const MAX_OFFSET: u32 = 1 << 18;
    pub const NO_MEMBER_INDEX: u16 = (1 << 12) - 1;

    pub fn new(
        name_hash: u32,
        data_type: PsoDataType,
        offset: u32,
        subtype: u8,
        reference_key: u32,
    ) -> Self {
        let mut entry = Self {
            name_hash,
            data_type,
            offset_high_and_subtype: 0,
            offset_low: 0,
            reference_key,
        };
        entry.set_subtype(subtype);
        entry.set_offset(offset);
        entry
    }

    pub fn offset(self) -> u32 {
        ((self.offset_high_and_subtype as u32 >> 6) << 16) | self.offset_low as u32
    }

    pub fn set_offset(&mut self, offset: u32) {
        debug_assert!(
            offset < Self::MAX_OFFSET,
            "offset {offset} too large for PSO"
        );
        self.offset_low = offset as u16;
        let high = ((offset >> 16) as u8) << 6;
        self.offset_high_and_subtype = high | (self.offset_high_and_subtype & 0x3F);
    }

    pub fn subtype(self) -> u8 {
        self.offset_high_and_subtype & 0x3F
    }

    pub fn set_subtype(&mut self, subtype: u8) {
        debug_assert!(subtype < 0x40, "subtype {subtype} does not fit in 6 bits");
        self.offset_high_and_subtype = (self.offset_high_and_subtype & !0x3F) | (subtype & 0x3F);
    }

    pub fn referent_hash(self) -> u32 {
        self.reference_key
    }

    pub fn reference_key(self) -> u32 {
        self.reference_key
    }

    pub fn count(self) -> u16 {
        (self.reference_key >> 16) as u16
    }

    fn align_and_member(self) -> u16 {
        (self.reference_key & 0xFFFF) as u16
    }

    pub fn member_index(self) -> Option<u16> {
        let idx = self.align_and_member() & 0x0FFF;
        (idx != Self::NO_MEMBER_INDEX).then_some(idx)
    }

    pub fn align_power(self) -> u8 {
        (self.align_and_member() >> 12) as u8
    }

    pub fn key_index(self) -> u16 {
        (self.reference_key >> 16) as u16
    }

    pub fn value_index(self) -> u16 {
        (self.reference_key & 0xFFFF) as u16
    }

    fn read(cursor: &mut Cursor<&[u8]>) -> Result<Self> {
        let name_hash = cursor.read_u32::<BigEndian>()?;
        let data_type = PsoDataType::from_byte(cursor.read_u8()?);
        let offset_high_and_subtype = cursor.read_u8()?;
        let offset_low = cursor.read_u16::<BigEndian>()?;
        let reference_key = cursor.read_u32::<BigEndian>()?;
        Ok(Self {
            name_hash,
            data_type,
            offset_high_and_subtype,
            offset_low,
            reference_key,
        })
    }

    pub fn write(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.name_hash.to_be_bytes());
        out.push(self.data_type.to_byte());
        out.push(self.offset_high_and_subtype);
        out.extend_from_slice(&self.offset_low.to_be_bytes());
        out.extend_from_slice(&self.reference_key.to_be_bytes());
    }
}

pub const ARRAY_INFO_NAME_HASH: u32 = 0x100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsoStructInfo {
    pub name_hash: u32,
    pub flags: u8,
    pub data_size: u32,
    pub major_version: u16,
    pub pad: u16,
    pub entries: Vec<PsoStructEntry>,
}

impl PsoStructInfo {
    pub fn entry(&self, index: u16) -> Option<&PsoStructEntry> {
        self.entries.get(index as usize)
    }
}

/// Why a parsed `parStructure` fails the engine's layout invariants.
#[derive(Debug, thiserror::Error)]
pub enum StructLayoutError {
    #[error("parStructure {structure:#010X}: member {member:#010X} at offset {offset:#X} does not fit inside the structure's {size:#X} bytes")]
    MemberPastEnd {
        structure: u32,
        member: u32,
        offset: u32,
        size: u32,
    },
}

impl From<StructLayoutError> for MetaError {
    fn from(error: StructLayoutError) -> Self {
        MetaError::Other(error.to_string())
    }
}

/// Checks a parsed `parStructure` against the engine's layout invariants: no member starts past
/// the declared `size`, and a fixed-size member ends inside it. The array element descriptors
/// (name hash [`ARRAY_INFO_NAME_HASH`]) are not members, and are skipped as the readers skip them.
/// `members` are `(name hash, offset, engine type)`.
///
/// ⚠ Two rules shipped GTA V files break, so neither is checked: members do not ascend by offset
/// (`junctions.pso` lists the member at 0x21 before the one at 0x20), and a member with no stated
/// width may start at `size` itself (an empty structure as the last member).
pub(crate) fn validate_struct_layout(
    structure: u32,
    size: u32,
    members: impl IntoIterator<Item = (u32, u32, PsoDataType)>,
) -> std::result::Result<(), StructLayoutError> {
    for (member, offset, data_type) in members {
        if member == ARRAY_INFO_NAME_HASH {
            continue;
        }
        let end = data_type
            .fixed_size()
            .map_or(offset, |bytes| offset.saturating_add(bytes as u32));
        if end > size {
            return Err(StructLayoutError::MemberPastEnd {
                structure,
                member,
                offset,
                size,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PsoEnumEntry {
    pub name_hash: u32,
    pub value: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsoEnumInfo {
    pub name_hash: u32,
    pub flags: u8,
    pub entries: Vec<PsoEnumEntry>,
}

const SCHEMA_TYPE_STRUCTURE: u8 = 0;
const SCHEMA_TYPE_ENUM: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsoSchemaItem {
    Struct(PsoStructInfo),
    Enum(PsoEnumInfo),
}

impl PsoSchemaItem {
    pub fn name_hash(&self) -> u32 {
        match self {
            PsoSchemaItem::Struct(s) => s.name_hash,
            PsoSchemaItem::Enum(e) => e.name_hash,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PsoSchemaSection {
    pub items: Vec<PsoSchemaItem>,
    struct_index: HashMap<u32, usize>,
    enum_index: HashMap<u32, usize>,
    /// GTA VI's enum value records are 16 bytes, `{u32 name hash, u32 word, i64 value}`, where GTA V's are `{u32 name hash, i32 value}`.
    gta6_enum_values: bool,
    /// Each GTA VI enum's word, 0 or 0xFFFFFFFF, by the enum's name hash: one enum states one.
    gta6_enum_words: HashMap<u32, u32>,
}

impl PsoSchemaSection {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_for(bytes, false)
    }

    /// The schema of a file whose `CHKS` states GTA VI (`gta6`) or not.
    pub(crate) fn parse_for(bytes: &[u8], gta6: bool) -> Result<Self> {
        let (payload_off, _end, _ident) = read_section_header(bytes)?;
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(payload_off as u64);
        let count = cursor.read_u32::<BigEndian>()?;

        // ⚠ No capacity from `count`: a corrupt one asks for gigabytes, and an
        // allocation that fails aborts rather than panics.
        let mut idx: Vec<(u32, i32)> = Vec::new();
        for _ in 0..count {
            let name = cursor.read_u32::<BigEndian>()?;
            let offset = cursor.read_i32::<BigEndian>()?;
            idx.push((name, offset));
        }

        let mut out = PsoSchemaSection {
            gta6_enum_values: gta6,
            ..PsoSchemaSection::default()
        };
        for (name_hash, entry_offset) in &idx {
            if *entry_offset < 0 || *entry_offset as usize >= bytes.len() {
                return Err(MetaError::Other(format!(
                    "pso schema: entry offset {entry_offset} out of range"
                )));
            }
            cursor.set_position(*entry_offset as u64);
            let schema_type = cursor.read_u8()?;
            cursor.set_position(*entry_offset as u64);
            match schema_type {
                SCHEMA_TYPE_STRUCTURE => {
                    let info = parse_struct_info(*name_hash, &mut cursor)?;
                    validate_struct_layout(
                        info.name_hash,
                        info.data_size,
                        info.entries
                            .iter()
                            .map(|e| (e.name_hash, e.offset(), e.data_type)),
                    )?;
                    out.struct_index.insert(*name_hash, out.items.len());
                    out.items.push(PsoSchemaItem::Struct(info));
                }
                SCHEMA_TYPE_ENUM => {
                    let (info, word) = parse_enum_info(*name_hash, &mut cursor, gta6)?;
                    if let Some(word) = word {
                        out.gta6_enum_words.insert(*name_hash, word);
                    }
                    out.enum_index.insert(*name_hash, out.items.len());
                    out.items.push(PsoSchemaItem::Enum(info));
                }
                other => {
                    return Err(MetaError::Other(format!(
                        "pso schema: unknown entry type {other:#X}"
                    )));
                }
            }
        }
        Ok(out)
    }

    pub fn struct_for(&self, name_hash: u32) -> Option<&PsoStructInfo> {
        let idx = *self.struct_index.get(&name_hash)?;
        match &self.items[idx] {
            PsoSchemaItem::Struct(s) => Some(s),
            PsoSchemaItem::Enum(_) => None,
        }
    }

    pub fn enum_for(&self, name_hash: u32) -> Option<&PsoEnumInfo> {
        let idx = *self.enum_index.get(&name_hash)?;
        match &self.items[idx] {
            PsoSchemaItem::Enum(e) => Some(e),
            PsoSchemaItem::Struct(_) => None,
        }
    }

    pub fn insert_struct(&mut self, info: PsoStructInfo) {
        match self.struct_index.get(&info.name_hash) {
            Some(&idx) => self.items[idx] = PsoSchemaItem::Struct(info),
            None => {
                self.struct_index.insert(info.name_hash, self.items.len());
                self.items.push(PsoSchemaItem::Struct(info));
            }
        }
    }

    pub fn insert_enum(&mut self, info: PsoEnumInfo) {
        match self.enum_index.get(&info.name_hash) {
            Some(&idx) => self.items[idx] = PsoSchemaItem::Enum(info),
            None => {
                self.enum_index.insert(info.name_hash, self.items.len());
                self.items.push(PsoSchemaItem::Enum(info));
            }
        }
    }

    pub fn write(&self) -> Vec<u8> {
        let index_bytes = self.items.len() * 8;
        let body_start = 8 + 4 + index_bytes;

        let mut bodies = Vec::new();
        let mut offsets = Vec::with_capacity(self.items.len());
        for item in &self.items {
            offsets.push((body_start + bodies.len()) as i32);
            match item {
                PsoSchemaItem::Struct(s) => write_struct_info(s, &mut bodies),
                PsoSchemaItem::Enum(e) => {
                    let word = (self.gta6_enum_words.get(&e.name_hash).copied())
                        .unwrap_or(GTA6_ENUM_VALUE_WORD);
                    write_enum_info(e, &mut bodies, self.gta6_enum_values.then_some(word))
                }
            }
        }

        let mut body = Vec::with_capacity(4 + index_bytes + bodies.len());
        body.extend_from_slice(&(self.items.len() as u32).to_be_bytes());
        for (item, offset) in self.items.iter().zip(&offsets) {
            body.extend_from_slice(&item.name_hash().to_be_bytes());
            body.extend_from_slice(&offset.to_be_bytes());
        }
        body.extend_from_slice(&bodies);
        super::sections::wrap_section(super::sections::PsoSection::Psch, &body)
    }
}

fn write_struct_info(s: &PsoStructInfo, out: &mut Vec<u8>) {
    out.push(SCHEMA_TYPE_STRUCTURE);
    out.push(s.flags);
    out.extend_from_slice(&(s.entries.len() as u16).to_be_bytes());
    out.extend_from_slice(&s.data_size.to_be_bytes());
    out.extend_from_slice(&s.major_version.to_be_bytes());
    out.extend_from_slice(&s.pad.to_be_bytes());
    for entry in &s.entries {
        entry.write(out);
    }
}

/// The word an enum the document did not state is written with; its meaning is not known.
const GTA6_ENUM_VALUE_WORD: u32 = 0xFFFF_FFFF;
/// The words between a GTA VI enum value's name hash and its value, one per enum.
const GTA6_ENUM_VALUE_WORDS: [u32; 2] = [0, 0xFFFF_FFFF];

/// `gta6_word` is the word GTA VI's 16-byte records state, `None` for GTA V's 8-byte ones.
fn write_enum_info(e: &PsoEnumInfo, out: &mut Vec<u8>, gta6_word: Option<u32>) {
    out.push(SCHEMA_TYPE_ENUM);
    out.push(e.flags);
    out.extend_from_slice(&(e.entries.len() as u16).to_be_bytes());
    for v in &e.entries {
        out.extend_from_slice(&v.name_hash.to_be_bytes());
        if let Some(word) = gta6_word {
            out.extend_from_slice(&word.to_be_bytes());
            out.extend_from_slice(&i64::from(v.value).to_be_bytes());
        } else {
            out.extend_from_slice(&v.value.to_be_bytes());
        }
    }
}

fn parse_struct_info(name_hash: u32, cursor: &mut Cursor<&[u8]>) -> Result<PsoStructInfo> {
    let _schema_type = cursor.read_u8()?;
    let flags = cursor.read_u8()?;
    let num_members = cursor.read_u16::<BigEndian>()?;
    let data_size = cursor.read_u32::<BigEndian>()?;
    let major_version = cursor.read_u16::<BigEndian>()?;
    let pad = cursor.read_u16::<BigEndian>()?;

    let mut entries = Vec::with_capacity(num_members as usize);
    for _ in 0..num_members {
        entries.push(PsoStructEntry::read(cursor)?);
    }

    Ok(PsoStructInfo {
        name_hash,
        flags,
        data_size,
        major_version,
        pad,
        entries,
    })
}

/// An enum and, for GTA VI's records, the one word its values state.
fn parse_enum_info(
    name_hash: u32,
    cursor: &mut Cursor<&[u8]>,
    gta6: bool,
) -> Result<(PsoEnumInfo, Option<u32>)> {
    let enum_hash = name_hash;
    let _schema_type = cursor.read_u8()?;
    let flags = cursor.read_u8()?;
    let num_values = cursor.read_u16::<BigEndian>()?;
    let mut entries = Vec::with_capacity(num_values as usize);
    let mut enum_word = None;
    for _ in 0..num_values {
        let name_hash = cursor.read_u32::<BigEndian>()?;
        let value = if gta6 {
            let word = cursor.read_u32::<BigEndian>()?;
            let wide = cursor.read_i64::<BigEndian>()?;
            let value = match i32::try_from(wide) {
                Ok(value) if GTA6_ENUM_VALUE_WORDS.contains(&word) => value,
                _ => {
                    return Err(MetaError::Other(format!(
                    "pso schema: GTA VI enum value {name_hash:#010X} is {{{word:#010X}, {wide}}}, \
                         where every measured record holds {{0 or 0xFFFFFFFF, a 32-bit value}}"
                )))
                }
            };
            if enum_word.is_some_and(|w| w != word) {
                return Err(MetaError::Other(format!(
                    "pso schema: GTA VI enum {enum_hash:#010X} states both words 0 and 0xFFFFFFFF, \
                     where every measured enum states one"
                )));
            }
            enum_word = Some(word);
            value
        } else {
            cursor.read_i32::<BigEndian>()?
        };
        entries.push(PsoEnumEntry { name_hash, value });
    }
    let info = PsoEnumInfo {
        name_hash: enum_hash,
        flags,
        entries,
    };
    Ok((info, enum_word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_spans_eighteen_bits() {
        let mut e = PsoStructEntry::new(1, PsoDataType::UInt, 0, 0, 0);
        e.set_offset(0x3_1234);
        assert_eq!(e.offset(), 0x3_1234);
        e.set_subtype(7);
        assert_eq!(e.offset(), 0x3_1234);
        assert_eq!(e.subtype(), 7);
    }

    #[test]
    fn the_two_on_disk_forms_name_one_engine_member_type() {
        use crate::meta::binary::rbm::schema::PsoType;
        // The engine's canonical parMemberType: FLOAT is 0x07, STRING is 0x0B, a hashed string
        // carries subtype 0x07. A P-form member's type byte already is this numbering.
        assert_eq!(PsoDataType::from_byte(0x07), PsoDataType::Float);
        assert_eq!(PsoDataType::Float.to_byte(), 0x07);
        assert_eq!(PsoDataType::from_byte(0x0B), PsoDataType::String);
        assert_eq!(PsoDataType::String.to_byte(), 0x0B);
        assert_eq!(string_subtype::HASH, 0x07);
        // The L-form table marks the same two engine types with its own distinct codes.
        assert_eq!(PsoType::from_byte(0x21), PsoType::Float);
        assert_eq!(PsoType::from_byte(0x4A), PsoType::StringHash);
    }

    /// One struct of `members`, `(name hash, type, offset)`, with a declared `size`, written as a
    /// `PSCH` section the parser reads back.
    fn one_struct(size: u32, members: &[(u32, PsoDataType, u32)]) -> Vec<u8> {
        let mut section = PsoSchemaSection::default();
        section.insert_struct(PsoStructInfo {
            name_hash: 0x16F5_3758,
            flags: 0,
            data_size: size,
            major_version: 0,
            pad: 0,
            entries: members
                .iter()
                .map(|&(name, ty, off)| PsoStructEntry::new(name, ty, off, 0, 0))
                .collect(),
        });
        section.write()
    }

    #[test]
    fn a_well_formed_struct_validates_and_a_corrupt_layout_is_refused() {
        let good = [
            (0x1111_1111, PsoDataType::Float, 0x00),
            (0x2222_2222, PsoDataType::Float, 0x04),
            (0x3333_3333, PsoDataType::UInt, 0x08),
        ];
        PsoSchemaSection::parse(&one_struct(0x20, &good)).expect("a well-formed struct reads");

        // A member offset past the declared size.
        let past = [(0x1111_1111, PsoDataType::Float, 0x40)];
        let err = PsoSchemaSection::parse(&one_struct(0x20, &past))
            .expect_err("a member past the end is refused")
            .to_string();
        assert!(err.contains("does not fit"), "{err}");

        // A declared size too small for the last member to fit.
        assert!(PsoSchemaSection::parse(&one_struct(0x08, &good)).is_err());

        // What shipped files hold: members out of offset order, and one of no width at the end.
        let shipped = [
            (0x1111_1111, PsoDataType::Bool, 0x09),
            (0x2222_2222, PsoDataType::Bool, 0x08),
            (0x3333_3333, PsoDataType::Structure, 0x20),
        ];
        PsoSchemaSection::parse(&one_struct(0x20, &shipped)).expect("a shipped layout reads");

        // A member of no width past the end is still past the end.
        let past = [(0x1111_1111, PsoDataType::Structure, 0x21)];
        assert!(PsoSchemaSection::parse(&one_struct(0x20, &past)).is_err());
    }

    #[test]
    fn subtype_is_masked_off_the_offset_high_bits() {
        // Offset 0x20000 sets bit 17, i.e. 0b01 in the high pair.
        let e = PsoStructEntry::new(1, PsoDataType::String, 0x2_0000, 3, 0);
        assert_eq!(e.offset(), 0x2_0000);
        assert_eq!(e.subtype(), 3);
    }

    #[test]
    fn member_index_masks_off_alignment_power() {
        let key = (4u32 << 12) | 9;
        let e = PsoStructEntry::new(1, PsoDataType::Array, 0, 0, key);
        assert_eq!(e.member_index(), Some(9));
        assert_eq!(e.align_power(), 4);
    }

    #[test]
    fn no_member_index_sentinel_is_none() {
        let key = 0x0FFFu32;
        let e = PsoStructEntry::new(1, PsoDataType::Array, 0, 0, key);
        assert_eq!(e.member_index(), None);
    }

    #[test]
    fn count_is_the_high_half() {
        let key = (12u32 << 16) | 5;
        let e = PsoStructEntry::new(1, PsoDataType::Array, 0, 0, key);
        assert_eq!(e.count(), 12);
        assert_eq!(e.member_index(), Some(5));
    }

    #[test]
    fn map_key_is_high_and_value_is_low() {
        let key = (3u32 << 16) | 4;
        let e = PsoStructEntry::new(1, PsoDataType::Map, 0, 0, key);
        assert_eq!(e.key_index(), 3);
        assert_eq!(e.value_index(), 4);
    }

    #[test]
    fn entry_write_round_trips() {
        let e = PsoStructEntry::new(0xDEADBEEF, PsoDataType::Vec4V, 0x1_2345, 0x2A, 0xCAFEF00D);
        let mut buf = Vec::new();
        e.write(&mut buf);
        assert_eq!(buf.len(), 12);
        let mut cursor = Cursor::new(buf.as_slice());
        let back = PsoStructEntry::read(&mut cursor).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.offset(), 0x1_2345);
        assert_eq!(back.subtype(), 0x2A);
    }

    #[test]
    fn type_bytes_round_trip_across_the_whole_enum() {
        for b in 0u8..=0x21 {
            let t = PsoDataType::from_byte(b);
            assert_eq!(t.to_byte(), b, "type {b:#04X} does not round-trip");
            assert!(
                !matches!(t, PsoDataType::Unknown(_)),
                "type {b:#04X} should be known"
            );
        }
        assert_eq!(PsoDataType::from_byte(0x22), PsoDataType::Unknown(0x22));
    }

    #[test]
    fn align_up_rounds_to_the_next_multiple() {
        assert_eq!(align_up(0, 16), 0);
        assert_eq!(align_up(1, 16), 16);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(17, 16), 32);
        assert_eq!(align_up(5, 1), 5);
    }

    #[test]
    fn align_up_with_align_zero_one_and_sixteen() {
        for value in [0usize, 1, 5, 15, 16, 17, 32, 33] {
            assert_eq!(align_up(value, 0), value);
            assert_eq!(align_up(value, 1), value);
            let aligned = align_up(value, 16);
            assert_eq!(aligned % 16, 0, "{value}");
            assert!(aligned >= value && aligned - value < 16, "{value}");
        }
    }

    #[test]
    fn a_map_pair_is_padded_to_the_wider_of_its_halves() {
        assert_eq!(map_pair_layout(4, 4), (4, 8));
        assert_eq!(map_pair_layout(4, 16), (16, 32));
        assert_eq!(map_pair_layout(1, 1), (1, 2));
    }

    /// A GTA VI schema of one enum whose two values state `words`.
    fn gta6_enum(words: [u32; 2]) -> Vec<u8> {
        let mut body = 1u32.to_be_bytes().to_vec();
        body.extend_from_slice(&0xAAAA_00E0u32.to_be_bytes());
        body.extend_from_slice(&20i32.to_be_bytes());
        body.extend_from_slice(&[SCHEMA_TYPE_ENUM, 0, 0, 2]);
        for (i, word) in words.into_iter().enumerate() {
            body.extend_from_slice(&(0xE0E0_0000 + i as u32).to_be_bytes());
            body.extend_from_slice(&word.to_be_bytes());
            body.extend_from_slice(&(i as i64 - 3).to_be_bytes());
        }
        super::super::sections::wrap_section(super::super::sections::PsoSection::Psch, &body)
    }

    #[test]
    fn a_gta6_enum_keeps_the_one_word_its_values_state() {
        for word in GTA6_ENUM_VALUE_WORDS {
            let bytes = gta6_enum([word, word]);
            let schema = PsoSchemaSection::parse_for(&bytes, true).expect("it reads");
            let values = schema.enum_for(0xAAAA_00E0).map(|e| e.entries[1].value);
            assert_eq!(values, Some(-2), "{word:#X}");
            assert_eq!(schema.write(), bytes, "{word:#X}");
        }
        for (words, want) in [
            ([0, 0xFFFF_FFFF], "states both words 0 and 0xFFFFFFFF"),
            ([0xFFFF_FFFF, 0], "states both words 0 and 0xFFFFFFFF"),
            ([0, 1], "is {0x00000001, -2}"),
        ] {
            let err = PsoSchemaSection::parse_for(&gta6_enum(words), true)
                .expect_err(want)
                .to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }
}
