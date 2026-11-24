use std::io::Cursor;

use byteorder::{BigEndian, ReadBytesExt};

use crate::meta::binary::enum_text;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::Nesting;
use crate::meta::diagnostics::{Diagnostics, GapKind};
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::{HashedString, MetaInstance, Value};
use crate::meta::schema::SchemaLibrary;

use super::schema::{
    string_subtype, struct_subtype, PsoDataType, PsoEnumInfo, PsoSchemaSection, PsoStructEntry,
    PsoStructInfo, ARRAY_INFO_NAME_HASH,
};
use super::sections::{
    chks_states_gta6, split_sections, PmapHeaderKind, PmapSection, PsoArrayHeader, PsoPointer,
    PsoSection, RawPsoSection,
};
use super::table_form;

pub fn parse(library: &SchemaLibrary, bytes: &[u8]) -> Result<MetaInstance> {
    let mut diags = Diagnostics::lenient();
    parse_with(library, bytes, &mut diags)
}

pub fn parse_with(
    library: &SchemaLibrary,
    bytes: &[u8],
    diags: &mut Diagnostics,
) -> Result<MetaInstance> {
    parse_full(library, bytes, diags).map(|(inst, _)| inst)
}

pub(crate) fn parse_full(
    _library: &SchemaLibrary,
    bytes: &[u8],
    diags: &mut Diagnostics,
) -> Result<(MetaInstance, PsoParts)> {
    let sections = split_sections(bytes)?;
    if table_form::is_table_form(&sections) {
        let instance = table_form::read(&sections)?;
        let pmap = PmapSection {
            root_id: 0,
            header_kind: PmapHeaderKind::Legacy { unk_0eh: 0 },
            entries: Vec::new(),
        };
        let schema = PsoSchemaSection::default();
        return Ok((
            instance,
            PsoParts {
                schema,
                pmap,
                sections,
            },
        ));
    }
    let psin = find_section(&sections, PsoSection::Psin)
        .ok_or_else(|| MetaError::Other("pso: missing PSIN section".into()))?;
    let pmap = find_section(&sections, PsoSection::Pmap)
        .ok_or_else(|| MetaError::Other("pso: missing PMAP section".into()))?;
    let psch = find_section(&sections, PsoSection::Psch)
        .ok_or_else(|| MetaError::Other("pso: missing PSCH section".into()))?;

    let gta6 = sections
        .iter()
        .any(|s| s.ident == PsoSection::Chks && chks_states_gta6(&s.bytes));
    let _names = gta6.then(crate::meta::binary::hash_index::title_names);
    let psin_data = psin_data(&psin.bytes)?;
    let pmap_section = PmapSection::parse(&pmap.bytes)?;
    let schema_section = PsoSchemaSection::parse_for(&psch.bytes, gta6)?;

    let mut decoder = PsoDecoder {
        data: psin_data,
        pmap: &pmap_section,
        schema: &schema_section,
        diags,
        nesting: Nesting::default(),
        gta6,
    };
    let instance = decoder.decode_root()?;
    Ok((
        instance,
        PsoParts {
            schema: schema_section,
            pmap: pmap_section,
            sections,
        },
    ))
}

pub(crate) struct PsoParts {
    pub schema: PsoSchemaSection,
    pub pmap: PmapSection,
    pub sections: Vec<RawPsoSection>,
}

impl PsoParts {
    pub(crate) fn into_layout(self) -> crate::meta::binary::layout::PsoLayout {
        use crate::meta::binary::layout::{OpaqueSection, PsoLayout};
        let section_order = self.sections.iter().map(|s| s.ident).collect();
        let psin = self
            .sections
            .iter()
            .find(|s| s.ident == PsoSection::Psin)
            .map(|s| s.bytes.clone())
            .unwrap_or_default();
        let opaque_sections = self
            .sections
            .into_iter()
            .filter(|s| {
                !matches!(
                    s.ident,
                    PsoSection::Psin | PsoSection::Pmap | PsoSection::Psch
                )
            })
            .map(|s| OpaqueSection {
                ident: s.ident,
                bytes: s.bytes,
            })
            .collect();
        PsoLayout {
            schema: self.schema,
            pmap: self.pmap,
            psin,
            opaque_sections,
            section_order,
        }
    }
}

fn find_section(sections: &[RawPsoSection], ident: PsoSection) -> Option<&RawPsoSection> {
    sections.iter().find(|s| s.ident == ident)
}

fn psin_data(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.len() < 8 {
        return Err(MetaError::Other("pso: PSIN truncated".into()));
    }
    Ok(bytes)
}

struct PsoDecoder<'a, 'd> {
    data: &'a [u8],
    pmap: &'a PmapSection,
    schema: &'a PsoSchemaSection,
    diags: &'d mut Diagnostics,
    nesting: Nesting,
    /// The file's `CHKS` states GTA VI, whose pointers, subtype-10 arrays and maps differ.
    gta6: bool,
}

impl PsoDecoder<'_, '_> {
    fn decode_root(&mut self) -> Result<MetaInstance> {
        let block = self
            .pmap
            .block(self.pmap.root_id)
            .ok_or_else(|| MetaError::Other("pso: root block not found in PMAP".into()))?;
        let info = self.schema.struct_for(block.name_hash).ok_or_else(|| {
            MetaError::Other(format!(
                "pso: root block schema {:#X} not found",
                block.name_hash
            ))
        })?;
        self.read_struct(info, block.offset as usize)
    }

    fn read_struct(&mut self, info: &PsoStructInfo, base_offset: usize) -> Result<MetaInstance> {
        let mark = self.nesting.enter(Some((base_offset, info.name_hash)))?;
        let idx = default_hash_index();
        let type_name = idx.name_or_literal(info.name_hash);
        self.diags.push_path(type_name.clone());
        let mut inst = MetaInstance::new(type_name);

        let result = (|| -> Result<()> {
            for entry in &info.entries {
                if entry.name_hash == ARRAY_INFO_NAME_HASH {
                    continue;
                }
                let name = idx.name_or_literal(entry.name_hash);
                let offset = base_offset + entry.offset() as usize;
                self.diags.push_path(name.clone());
                let decoded = self.decode_member(info, entry, offset);
                let value = match decoded {
                    Ok(v) => v,
                    Err(err) => {
                        self.diags
                            .record(GapKind::FieldDecodeError, err.to_string())?;
                        Value::Raw(format!("(decode error: {err})"))
                    }
                };
                self.diags.pop_path();
                inst.fields.push((name, value));
            }
            Ok(())
        })();

        self.diags.pop_path();
        self.nesting.leave(mark)?;
        result?;
        Ok(inst)
    }

    fn decode_member(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
    ) -> Result<Value> {
        match entry.data_type {
            PsoDataType::Bool => Ok(Value::Bool(self.read_u8(offset)? != 0)),
            PsoDataType::SByte => Ok(Value::Int(self.read_u8(offset)? as i8 as i64)),
            PsoDataType::UByte => Ok(Value::UInt(self.read_u8(offset)? as u64)),
            PsoDataType::SShort => Ok(Value::Int(self.read_i16(offset)? as i64)),
            PsoDataType::UShort => Ok(Value::UInt(self.read_u16(offset)? as u64)),
            PsoDataType::SInt => Ok(Value::Int(self.read_i32(offset)? as i64)),
            PsoDataType::UInt => Ok(Value::UInt(self.read_u32(offset)? as u64)),
            PsoDataType::Int64 => Ok(Value::Int(self.read_u64(offset)? as i64)),
            PsoDataType::UInt64 => Ok(Value::UInt(self.read_u64(offset)?)),
            PsoDataType::PtrDiffT => Ok(Value::Int(self.read_u64(offset)? as i64)),
            PsoDataType::SizeT => Ok(Value::UInt(self.read_u64(offset)?)),
            PsoDataType::Float => Ok(Value::Float(self.read_f32(offset)? as f64)),
            PsoDataType::Double => Ok(Value::Float(f64::from_bits(self.read_u64(offset)?))),
            PsoDataType::Float16 => Ok(Value::Float(
                crate::half_to_f32(self.read_u16(offset)?) as f64
            )),
            PsoDataType::ScalarV => Ok(Value::Float(self.read_f32(offset)? as f64)),
            PsoDataType::BoolV => Ok(Value::Bool(self.read_u32(offset)? != 0)),
            PsoDataType::Vector2 | PsoDataType::Vec2V => Ok(Value::Vec2([
                self.read_f32(offset)?,
                self.read_f32(offset + 4)?,
            ])),
            PsoDataType::Vector3 | PsoDataType::Vec3V => Ok(Value::Vec3([
                self.read_f32(offset)?,
                self.read_f32(offset + 4)?,
                self.read_f32(offset + 8)?,
            ])),
            PsoDataType::Vector4 | PsoDataType::Vec4V | PsoDataType::VecBoolV => Ok(Value::Vec4([
                self.read_f32(offset)?,
                self.read_f32(offset + 4)?,
                self.read_f32(offset + 8)?,
                self.read_f32(offset + 12)?,
            ])),
            PsoDataType::Mat33V => self.read_matrix(offset, 3),
            PsoDataType::Matrix34 | PsoDataType::Mat34V => self.read_matrix(offset, 4),
            PsoDataType::Matrix44 | PsoDataType::Mat44V => self.read_matrix(offset, 4),
            PsoDataType::String => self.decode_string(entry, offset),
            PsoDataType::Structure => self.decode_struct_member(entry, offset),
            PsoDataType::Array => self.decode_array(owner, entry, offset),
            PsoDataType::Enum => self.decode_enum(entry, offset),
            PsoDataType::Bitset => self.decode_bitset(owner, entry, offset),
            PsoDataType::Map => self.decode_map(owner, entry, offset),
            PsoDataType::Unknown(GTA6_BYTES_16) if self.gta6 => {
                Ok(Value::ByteArray(self.bytes(offset, 16)?.to_vec()))
            }
            PsoDataType::Unknown(b) => {
                self.diags
                    .record(GapKind::UnknownDataType, format!("type {b:#04X}"))?;
                Ok(Value::Raw(format!("(pso datatype {b:#X})")))
            }
        }
    }

    fn read_matrix(&mut self, offset: usize, rows: usize) -> Result<Value> {
        let mut m = [[0.0f32; 4]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            if r >= rows {
                break;
            }
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = self.read_f32(offset + r * 16 + c * 4)?;
            }
        }
        Ok(Value::Mat34(m))
    }

    fn decode_string(&mut self, entry: &PsoStructEntry, offset: usize) -> Result<Value> {
        if self.gta6 && entry.subtype() == string_subtype::CHAR_ARRAY {
            if let Some(text) = gta6_text_in_place(self.bytes(offset, 16)?)? {
                return Ok(Value::String(String::from_utf8_lossy(text).into_owned()));
            }
        }
        match entry.subtype() {
            string_subtype::EMBEDDED_BUFFER => {
                let len = entry.count() as usize;
                let buf = self.data.get(offset..offset + len).unwrap_or(&[]);
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                Ok(Value::String(
                    String::from_utf8_lossy(&buf[..end]).into_owned(),
                ))
            }
            string_subtype::POINTER | string_subtype::POINTER_ALT => {
                let ptr = self.read_pointer(offset)?;
                if ptr.is_null() {
                    return Ok(Value::String(String::new()));
                }
                Ok(Value::String(self.read_block_string(ptr)?))
            }
            string_subtype::CHAR_ARRAY => {
                let arr = self.read_array_header(offset)?;
                if arr.pointer.is_null() {
                    return Ok(Value::String(String::new()));
                }
                let bytes = self.read_block_bytes(arr.pointer, arr.count as usize)?;
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                Ok(Value::String(
                    String::from_utf8_lossy(&bytes[..end]).into_owned(),
                ))
            }
            string_subtype::HASH | string_subtype::HASH_ALT => self.hash_string(offset),
            GTA6_STRING_HASH if self.gta6 => self.hash_string(offset),
            GTA6_STRING_HASH64 if self.gta6 => Ok(Value::UInt(self.read_u64(offset)?)),
            other => {
                self.diags
                    .record(GapKind::UnknownSubType, format!("string variant {other}"))?;
                Ok(Value::Raw(format!("(pso string variant {other})")))
            }
        }
    }

    fn decode_struct_member(&mut self, entry: &PsoStructEntry, offset: usize) -> Result<Value> {
        match entry.subtype() {
            struct_subtype::EMBEDDED => {
                let info = self
                    .schema
                    .struct_for(entry.referent_hash())
                    .ok_or_else(|| {
                        MetaError::Other(format!(
                            "pso: missing struct {:#X} for embedded field",
                            entry.referent_hash()
                        ))
                    })?;
                Ok(Value::Struct(Box::new(self.read_struct(info, offset)?)))
            }
            struct_subtype::POINTER | struct_subtype::POINTER_ALT => {
                let ptr = self.read_pointer(offset)?;
                if ptr.is_null() {
                    return Ok(Value::Pointer(None));
                }
                Ok(Value::Pointer(Some(Box::new(
                    self.read_pointer_object(ptr)?,
                ))))
            }
            other => {
                self.diags
                    .record(GapKind::UnknownSubType, format!("struct variant {other}"))?;
                Ok(Value::Raw(format!("(pso struct variant {other})")))
            }
        }
    }

    fn decode_enum(&mut self, entry: &PsoStructEntry, offset: usize) -> Result<Value> {
        let value = self.read_enum_value(entry, offset)?;
        let type_name = default_hash_index().name_or_literal(entry.referent_hash());
        let variant = match self
            .schema
            .enum_for(entry.referent_hash())
            .and_then(|e| variant_name(e, value))
        {
            Some(name_hash) => enum_text::variant_text(name_hash),
            None => enum_text::value_text(value),
        };
        Ok(Value::Enum { type_name, variant })
    }

    fn read_enum_value(&mut self, entry: &PsoStructEntry, offset: usize) -> Result<i32> {
        Ok(match entry.subtype() {
            2 => self.read_u8(offset)? as i32,
            _ => self.read_u32(offset)? as i32,
        })
    }

    fn decode_bitset(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
    ) -> Result<Value> {
        let bit_count = entry.count();
        let byte_width = bitset_byte_width(bit_count);
        let mut bits: u128 = 0;
        for i in 0..byte_width.min(16) {
            // Big-endian: the first byte holds the most significant bits.
            let byte = self.read_u8(offset + i)? as u128;
            bits |= byte << (8 * (byte_width.min(16) - 1 - i));
        }

        let enum_hash = entry
            .member_index()
            .and_then(|i| owner.entry(i))
            .map(|m| m.referent_hash())
            .unwrap_or(0);
        let type_name = default_hash_index().name_or_literal(enum_hash);
        Ok(Value::Bitset { type_name, bits })
    }

    fn decode_array(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
    ) -> Result<Value> {
        let Some(elem_index) = entry.member_index() else {
            self.diags.record(
                GapKind::MissingSchema,
                "array with no element descriptor".to_string(),
            )?;
            return Ok(Value::Array(Vec::new()));
        };
        let Some(elem) = owner.entry(elem_index).copied() else {
            self.diags.record(
                GapKind::MissingSchema,
                format!("array element descriptor index {elem_index} out of range"),
            )?;
            return Ok(Value::Array(Vec::new()));
        };

        let inline_count = entry.count();
        if inline_count == 0 {
            let header = match (self.gta6, entry.subtype()) {
                (false, _) | (true, 0) => self.read_array_header(offset)?,
                (true, GTA6_COUNT_FIRST_ARRAY) => {
                    let mut cursor = Cursor::new(self.data);
                    cursor.set_position(offset as u64);
                    PsoArrayHeader::read_count_first(&mut cursor)?
                }
                (true, other) => {
                    return Err(MetaError::Other(format!(
                        "pso: GTA VI array subtype {other}, whose header is not known"
                    )))
                }
            };
            if header.pointer.is_null() || header.count == 0 {
                return Ok(Value::Array(Vec::new()));
            }
            let block = self
                .pmap
                .block(header.pointer.block_id() as i32)
                .ok_or_else(|| {
                    MetaError::Other(format!(
                        "pso: array block {} not in PMAP",
                        header.pointer.block_id()
                    ))
                })?;
            let base = block.offset as usize + self.offset(header.pointer)?;
            self.decode_array_elements(owner, &elem, base, header.count as usize)
        } else {
            self.decode_array_elements(owner, &elem, offset, inline_count as usize)
        }
    }

    fn decode_array_elements(
        &mut self,
        owner: &PsoStructInfo,
        elem: &PsoStructEntry,
        base: usize,
        count: usize,
    ) -> Result<Value> {
        let Some(stride) = self.element_stride(elem) else {
            self.diags.record(
                GapKind::UnsupportedArrayElement,
                format!("{:?} subtype {}", elem.data_type, elem.subtype()),
            )?;
            return Ok(Value::Array(Vec::new()));
        };

        let mark = self.nesting.enter(None)?;
        let mut items = Vec::with_capacity(count.min(self.data.len()));
        let decoded: Result<()> = (0..count).try_for_each(|i| {
            items.push(self.decode_member(owner, elem, base + stride * i)?);
            Ok(())
        });
        self.nesting.leave(mark)?;
        decoded?;
        Ok(Value::Array(items))
    }

    fn element_stride(&self, elem: &PsoStructEntry) -> Option<usize> {
        super::schema::element_stride(self.schema, elem)
    }

    fn decode_map(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
    ) -> Result<Value> {
        if self.gta6 {
            return self.decode_gta6_map(entry, offset);
        }
        let key_desc = owner.entry(entry.key_index()).copied();
        let value_desc = owner.entry(entry.value_index()).copied();
        let (Some(key_desc), Some(value_desc)) = (key_desc, value_desc) else {
            self.diags.record(
                GapKind::UnsupportedMap,
                format!(
                    "map key/value descriptor index {}/{} out of range",
                    entry.key_index(),
                    entry.value_index()
                ),
            )?;
            return Ok(Value::Map(Vec::new()));
        };

        let header = self.read_array_header(offset)?;
        if header.pointer.is_null() || header.count == 0 {
            return Ok(Value::Map(Vec::new()));
        }

        let (Some(key_stride), Some(value_stride)) = (
            self.element_stride(&key_desc),
            self.element_stride(&value_desc),
        ) else {
            self.diags.record(
                GapKind::UnsupportedMap,
                format!(
                    "map of {:?} -> {:?}",
                    key_desc.data_type, value_desc.data_type
                ),
            )?;
            return Ok(Value::Map(Vec::new()));
        };

        let (_, pair_size) = super::schema::map_pair_layout(key_stride, value_stride);

        let block = self
            .pmap
            .block(header.pointer.block_id() as i32)
            .ok_or_else(|| {
                MetaError::Other(format!(
                    "pso: map block {} not in PMAP",
                    header.pointer.block_id()
                ))
            })?;
        let base = block.offset as usize + self.offset(header.pointer)?;

        let mark = self.nesting.enter(None)?;
        let mut pairs = Vec::with_capacity(header.count as usize);
        let decoded: Result<()> = (0..header.count as usize).try_for_each(|i| {
            let entry_base = base + pair_size * i;
            let key = self.decode_member(owner, &key_desc, entry_base)?;
            let value = self.decode_member(owner, &value_desc, entry_base + key_stride)?;
            pairs.push((key, value));
            Ok(())
        });
        self.nesting.leave(mark)?;
        decoded?;
        Ok(Value::Map(pairs))
    }

    /// A GTA VI map: a pair map's 24 bytes are the table form's binary map, big-endian, over pairs
    /// of its block's structure; a group map is read only when empty.
    fn decode_gta6_map(&mut self, entry: &PsoStructEntry, offset: usize) -> Result<Value> {
        match entry.subtype() {
            GTA6_PAIR_MAP => {}
            GTA6_GROUP_MAP => {
                let (groups, count) = (self.read_u32(offset)?, self.read_u32(offset + 4)?);
                if groups == 0 && count == 0 && self.read_pointer(offset + 8)?.is_null() {
                    return Ok(Value::Map(Vec::new()));
                }
                return Err(MetaError::Other(format!(
                    "pso: a GTA VI group map of {count} pairs, whose values' layout is not measured"
                )));
            }
            other => {
                return Err(MetaError::Other(format!(
                    "pso: GTA VI map subtype {other}, whose layout is not measured"
                )))
            }
        }
        let words = (self.read_u32(offset)?, self.read_u32(offset + 4)?);
        let last = self.read_u32(offset + 20)?;
        if words != (GTA6_PAIR_MAP_WORD, 0) || last != 0 {
            return Err(MetaError::Other(format!(
                "pso: a GTA VI map states {:#X}, {:#X} and {last:#X} around its pointer, where every measured one states 0x1000000, 0 and 0",
                words.0, words.1
            )));
        }
        let pointer = self.read_pointer(offset + 8)?;
        let count = usize::from(self.read_u16(offset + 16)?);
        if pointer.is_null() {
            if count != 0 {
                return Err(MetaError::Other(format!(
                    "pso: a GTA VI map counts {count} pairs behind a null pointer"
                )));
            }
            return Ok(Value::Map(Vec::new()));
        }
        let (pair, base) = gta6_pairs(self.pmap, self.schema, pointer)?;
        let [key, value] = named_pair(pair)?;
        let size = pair.data_size as usize;
        let fits = (size.checked_mul(count))
            .and_then(|len| len.checked_add(base))
            .is_some_and(|end| end <= self.data.len());
        if size == 0 || !fits {
            return Err(MetaError::Other(format!(
                "pso: a GTA VI map's {count} pairs of {size} bytes at {base:#X} run past PSIN"
            )));
        }
        let mark = self.nesting.enter(None)?;
        let mut pairs = Vec::with_capacity(count);
        let decoded: Result<()> = (0..count).try_for_each(|i| {
            let at = base + size * i;
            let k = self.decode_member(pair, &key, at + key.offset() as usize)?;
            let v = self.decode_member(pair, &value, at + value.offset() as usize)?;
            pairs.push((k, v));
            Ok(())
        });
        self.nesting.leave(mark)?;
        decoded?;
        Ok(Value::Map(pairs))
    }

    fn hash_string(&self, offset: usize) -> Result<Value> {
        let hash = self.read_u32(offset)?;
        let raw = default_hash_index().lookup(hash).map(str::to_string);
        Ok(Value::Hash(HashedString {
            raw: raw.unwrap_or_default(),
            hash,
        }))
    }

    fn bytes(&self, offset: usize, len: usize) -> Result<&[u8]> {
        (offset.checked_add(len))
            .and_then(|end| self.data.get(offset..end))
            .ok_or_else(|| MetaError::Other(format!("pso: {len} bytes read OOB at {offset}")))
    }

    fn read_pointer_object(&mut self, ptr: PsoPointer) -> Result<MetaInstance> {
        let block = self.pmap.block(ptr.block_id() as i32).ok_or_else(|| {
            MetaError::Other(format!("pso: pointer block {} not in PMAP", ptr.block_id()))
        })?;
        let info = self.schema.struct_for(block.name_hash).ok_or_else(|| {
            MetaError::Other(format!(
                "pso: pointer target schema {:#X} not in PSCH",
                block.name_hash
            ))
        })?;
        let offset = block.offset as usize + self.offset(ptr)?;
        self.read_struct(info, offset)
    }

    fn read_block_string(&self, ptr: PsoPointer) -> Result<String> {
        let block = self
            .pmap
            .block(ptr.block_id() as i32)
            .ok_or_else(|| MetaError::Other("pso: string block missing".into()))?;
        let base = block.offset as usize + self.offset(ptr)?;
        let slice = self
            .data
            .get(base..)
            .ok_or_else(|| MetaError::Other("pso: string offset out of bounds".into()))?;
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        Ok(String::from_utf8_lossy(&slice[..end]).into_owned())
    }

    fn read_block_bytes(&self, ptr: PsoPointer, len: usize) -> Result<Vec<u8>> {
        let block = self
            .pmap
            .block(ptr.block_id() as i32)
            .ok_or_else(|| MetaError::Other("pso: byte block missing".into()))?;
        let base = block.offset as usize + self.offset(ptr)?;
        self.data
            .get(base..base + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| MetaError::Other("pso: byte block out of bounds".into()))
    }

    fn offset(&self, ptr: PsoPointer) -> Result<usize> {
        Ok(ptr.data_offset_for(self.gta6)? as usize)
    }

    fn read_pointer(&self, offset: usize) -> Result<PsoPointer> {
        let mut cursor = Cursor::new(self.data);
        cursor.set_position(offset as u64);
        PsoPointer::read(&mut cursor)
    }

    fn read_array_header(&self, offset: usize) -> Result<PsoArrayHeader> {
        let mut cursor = Cursor::new(self.data);
        cursor.set_position(offset as u64);
        PsoArrayHeader::read(&mut cursor)
    }

    fn read_u8(&self, offset: usize) -> Result<u8> {
        self.data
            .get(offset)
            .copied()
            .ok_or_else(|| MetaError::Other(format!("pso: u8 read OOB at {offset}")))
    }
    fn read_u16(&self, offset: usize) -> Result<u16> {
        let mut c = Cursor::new(self.data);
        c.set_position(offset as u64);
        Ok(c.read_u16::<BigEndian>()?)
    }
    fn read_i16(&self, offset: usize) -> Result<i16> {
        let mut c = Cursor::new(self.data);
        c.set_position(offset as u64);
        Ok(c.read_i16::<BigEndian>()?)
    }
    fn read_u32(&self, offset: usize) -> Result<u32> {
        let mut c = Cursor::new(self.data);
        c.set_position(offset as u64);
        Ok(c.read_u32::<BigEndian>()?)
    }
    fn read_i32(&self, offset: usize) -> Result<i32> {
        let mut c = Cursor::new(self.data);
        c.set_position(offset as u64);
        Ok(c.read_i32::<BigEndian>()?)
    }
    fn read_u64(&self, offset: usize) -> Result<u64> {
        let mut c = Cursor::new(self.data);
        c.set_position(offset as u64);
        Ok(c.read_u64::<BigEndian>()?)
    }
    fn read_f32(&self, offset: usize) -> Result<f32> {
        let mut c = Cursor::new(self.data);
        c.set_position(offset as u64);
        Ok(c.read_f32::<BigEndian>()?)
    }
}

/// GTA VI's array subtype whose header is `{u32 count, u32 capacity, pointer}`.
pub(super) const GTA6_COUNT_FIRST_ARRAY: u8 = 10;
/// GTA VI's string subtypes that are a hash in place, of 4 and of 8 bytes.
pub(super) const GTA6_STRING_HASH: u8 = 10;
pub(super) const GTA6_STRING_HASH64: u8 = 14;
/// GTA VI's map subtypes: the table form's binary map and its group map.
pub(super) const GTA6_PAIR_MAP: u8 = 1;
pub(super) const GTA6_GROUP_MAP: u8 = 3;
/// The first word of every measured GTA VI pair map.
pub(super) const GTA6_PAIR_MAP_WORD: u32 = 0x0100_0000;
/// GTA VI's member type of 16 bytes in place, whose meaning is not known.
pub(super) const GTA6_BYTES_16: u8 = 0x24;

/// The text of a GTA VI string kept in its 16 bytes, `None` where byte 15 leaves bit 7 clear.
// ⚠ In place, byte 15 is 0x80 plus a length of 14 or fewer, the bytes past the text zero; out of
// place, bytes 0 to 7 are a pointer.
pub(super) fn gta6_text_in_place(field: &[u8]) -> Result<Option<&[u8]>> {
    let Some(&last) = field.get(15) else {
        return Err(MetaError::Other(format!(
            "pso: a GTA VI string is {} bytes, short of its 16",
            field.len()
        )));
    };
    if last & 0x80 == 0 {
        return Ok(None);
    }
    let len = usize::from(last & 0x7F);
    if len > 14 || field[len..15].iter().any(|&b| b != 0) {
        return Err(MetaError::Other(format!(
            "pso: a GTA VI string in place states {len} bytes, where every measured one holds 14 or fewer and zeros after them"
        )));
    }
    Ok(Some(&field[..len]))
}

/// The structure of the pairs `pointer` names, its block's, and where they start in PSIN.
pub(super) fn gta6_pairs<'s>(
    pmap: &PmapSection,
    schema: &'s PsoSchemaSection,
    pointer: PsoPointer,
) -> Result<(&'s PsoStructInfo, usize)> {
    let block = pmap.block(pointer.block_id() as i32).ok_or_else(|| {
        MetaError::Other(format!("pso: map block {} not in PMAP", pointer.block_id()))
    })?;
    let pair = schema.struct_for(block.name_hash).ok_or_else(|| {
        MetaError::Other(format!(
            "pso: map pair schema {:#X} not in PSCH",
            block.name_hash
        ))
    })?;
    let offset = pointer.data_offset_for(true)? as usize;
    let base = (usize::try_from(block.offset).ok())
        .and_then(|at| at.checked_add(offset))
        .ok_or_else(|| {
            MetaError::Other(format!(
                "pso: map block {} starts at {}",
                pointer.block_id(),
                block.offset
            ))
        })?;
    Ok((pair, base))
}

/// The key and the value of a pair structure: its two named members.
pub(super) fn named_pair(pair: &PsoStructInfo) -> Result<[PsoStructEntry; 2]> {
    let named: Vec<PsoStructEntry> = (pair.entries.iter())
        .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
        .copied()
        .collect();
    match named[..] {
        [key, value] => Ok([key, value]),
        _ => Err(MetaError::Other(format!(
            "pso: a GTA VI map's pairs are a {:#X}, which has {} named members, not a key and a value",
            pair.name_hash,
            named.len()
        ))),
    }
}

pub(super) fn bitset_byte_width(bit_count: u16) -> usize {
    match bit_count {
        0..=8 => 1,
        9..=16 => 2,
        17..=32 => 4,
        n => (n as usize).div_ceil(8),
    }
}

fn variant_name(info: &PsoEnumInfo, value: i32) -> Option<u32> {
    info.entries
        .iter()
        .find(|e| e.value == value)
        .map(|e| e.name_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_form_is_named_rather_than_missing_a_pmap() {
        let mut bytes = Vec::new();
        for ident in [b"PSIN", b"LSCH"] {
            bytes.extend_from_slice(ident);
            bytes.extend_from_slice(&8u32.to_be_bytes());
        }
        let library = SchemaLibrary::default();
        let err = parse_full(&library, &bytes, &mut Diagnostics::lenient())
            .err()
            .expect("refused")
            .to_string();
        assert!(err.contains("GTA VI"), "{err}");
        assert!(!err.contains("missing PMAP"), "{err}");
    }

    const ROOT: u32 = 0xAAAA_0001;
    const SUB: u32 = 0xAAAA_0002;
    const KIND: u32 = 0xAAAA_00E0;
    const KIND_VALUES: [(u32, i32); 3] = [(0xE0E0_0000, 0), (0xE0E0_0001, 1), (0xE0E0_0002, 2)];

    /// A P-form document as GTA VI's are laid out, with what a case changes.
    struct PForm {
        tag: [u8; 4],
        member_type: u8,
        subtype: u8,
        reference: u32,
        header: [u8; 16],
        enum_word: u32,
        pointer_low: u32,
    }

    impl Default for PForm {
        fn default() -> Self {
            // `{u32 count 2, u32 capacity 2}`, then block 2 at offset 0x1000:
            // offset bits 0 to 11 in bits 20 to 31, bits 12 to 19 in the next byte.
            let mut header = [0u8; 16];
            header[..12].copy_from_slice(&[0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]);
            header[12] = (0x1000u32 >> 12) as u8;
            Self {
                tag: *b"zppp",
                member_type: 0x0D,
                subtype: 10,
                reference: 3,
                header,
                enum_word: 0xFFFF_FFFF,
                pointer_low: 3 | 16 << 20,
            }
        }
    }

    fn section(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
        out.extend_from_slice(ident);
        out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(content);
    }

    impl PForm {
        fn bytes(&self) -> Vec<u8> {
            let mut psin = vec![0u8; 0x1060 - 8];
            let mut put =
                |at: usize, bytes: &[u8]| psin[at - 8..at - 8 + bytes.len()].copy_from_slice(bytes);
            put(0x10, &2u32.to_be_bytes());
            put(0x18, &self.header);
            put(0x28, &self.pointer_low.to_be_bytes());
            put(0x30 + 0x1000, &[0, 0, 0, 7, 0, 0, 0, 9]);
            put(0x1040 + 16, &0x1234u32.to_be_bytes());

            let mut pmap = Vec::new();
            pmap.extend_from_slice(&1i32.to_be_bytes());
            pmap.extend_from_slice(&3i16.to_be_bytes());
            pmap.extend_from_slice(&0i16.to_be_bytes());
            for (hash, offset, length) in
                [(ROOT, 0x10, 0x20), (6, 0x30, 0x1008), (SUB, 0x1040, 0x20)]
            {
                for word in [hash, offset, 0, length] {
                    pmap.extend_from_slice(&word.to_be_bytes());
                }
            }

            let entry = |out: &mut Vec<u8>,
                         hash: u32,
                         code: u8,
                         subtype: u8,
                         offset: u16,
                         reference: u32| {
                out.extend_from_slice(&hash.to_be_bytes());
                out.extend_from_slice(&[code, subtype]);
                out.extend_from_slice(&offset.to_be_bytes());
                out.extend_from_slice(&reference.to_be_bytes());
            };
            let mut items = Vec::new();
            items.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 0x20, 0, 0, 0, 0]);
            entry(&mut items, 0xBBBB_0000, 0x0E, 0, 0, KIND);
            entry(
                &mut items,
                0xBBBB_0001,
                self.member_type,
                self.subtype,
                8,
                self.reference,
            );
            entry(&mut items, 0xBBBB_0002, 0x0C, 3, 0x18, 0);
            entry(&mut items, ARRAY_INFO_NAME_HASH, 0x06, 0, 0, 0);
            items.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0]);
            entry(&mut items, 0xBBBB_0010, 0x06, 0, 0, 0);
            items.extend_from_slice(&[1, 0, 0, 3]);
            for (hash, value) in KIND_VALUES {
                items.extend_from_slice(&hash.to_be_bytes());
                items.extend_from_slice(&self.enum_word.to_be_bytes());
                items.extend_from_slice(&i64::from(value).to_be_bytes());
            }
            let mut psch = 3u32.to_be_bytes().to_vec();
            for (hash, offset) in [(ROOT, 36u32), (SUB, 36 + 60), (KIND, 36 + 60 + 24)] {
                psch.extend_from_slice(&hash.to_be_bytes());
                psch.extend_from_slice(&offset.to_be_bytes());
            }
            psch.extend_from_slice(&items);

            let mut out = Vec::new();
            section(&mut out, b"PSIN", &psin);
            section(&mut out, b"PMAP", &pmap);
            section(&mut out, b"PSCH", &psch);
            let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
            chks.extend_from_slice(&[0; 4]);
            chks.extend_from_slice(&self.tag);
            section(&mut out, b"CHKS", &chks);
            out
        }

        fn read(&self, diags: &mut Diagnostics) -> Result<MetaInstance> {
            parse_full(&SchemaLibrary::default(), &self.bytes(), diags).map(|(i, _)| i)
        }
    }

    fn named(hash: u32) -> String {
        default_hash_index().name_or_literal(hash)
    }

    #[test]
    fn a_gta6_p_form_reads_its_enums_pointers_and_count_first_arrays() {
        let instance = PForm::default()
            .read(&mut Diagnostics::strict())
            .expect("it reads");
        let mut sub = MetaInstance::new(named(SUB));
        sub.fields.push((named(0xBBBB_0010), Value::UInt(0x1234)));
        let mut expected = MetaInstance::new(named(ROOT));
        expected.fields = vec![
            (
                named(0xBBBB_0000),
                Value::Enum {
                    type_name: named(KIND),
                    variant: enum_text::variant_text(KIND_VALUES[2].0),
                },
            ),
            (
                named(0xBBBB_0001),
                Value::Array(vec![Value::UInt(7), Value::UInt(9)]),
            ),
            (named(0xBBBB_0002), Value::Pointer(Some(Box::new(sub)))),
        ];
        assert_eq!(instance, expected);

        let gta5 = PForm {
            tag: *b"yppp",
            ..PForm::default()
        };
        let read_as_gta5 = gta5.read(&mut Diagnostics::lenient());
        assert!(
            read_as_gta5.map_or(true, |i| i != expected),
            "the CHKS tag keys the reading"
        );
    }

    #[test]
    fn a_gta6_p_form_saves_back_to_its_own_bytes() {
        use crate::meta::binary::{parse_meta_binary_full, write_meta_binary, BinaryFormat};
        for enum_word in [0xFFFF_FFFF, 0] {
            let bytes = PForm {
                enum_word,
                ..PForm::default()
            }
            .bytes();
            let library = SchemaLibrary::default();
            let (instance, layout) =
                parse_meta_binary_full(&library, &bytes, &mut Diagnostics::strict())
                    .expect("it reads");
            let written =
                write_meta_binary(BinaryFormat::Pso, Some(&layout), &instance).expect("it saves");
            assert_eq!(written, bytes, "{enum_word:#X}");
        }
    }

    #[test]
    fn a_gta6_p_form_has_no_canonical_form_to_be_laid_out_again_into() {
        let err =
            crate::meta::canonical::converge(&SchemaLibrary::default(), &PForm::default().bytes())
                .expect_err("refused")
                .to_string();
        assert!(err.contains("GTA VI's layout rules are not known"), "{err}");
    }

    #[test]
    fn a_gta6_p_form_refuses_what_no_measured_document_holds() {
        let cases = [
            (
                PForm {
                    enum_word: 1,
                    ..PForm::default()
                },
                "GTA VI enum value 0xE0E00000 is {0x00000001, 0}",
            ),
            (
                PForm {
                    pointer_low: 3 | 1 << 12,
                    ..PForm::default()
                },
                "sets bits whose meaning is not known",
            ),
            (
                PForm {
                    subtype: 3,
                    ..PForm::default()
                },
                "GTA VI array subtype 3, whose header is not known",
            ),
            (
                PForm {
                    member_type: 0x10,
                    reference: 3 << 16 | 3,
                    header: [0, 0, 0, 2, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0],
                    ..PForm::default()
                },
                "GTA VI map subtype 10, whose layout is not measured",
            ),
            (
                PForm {
                    header: [
                        0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0xFF, 0, 0, 0, 2, 1, 0, 0, 0,
                    ],
                    ..PForm::default()
                },
                "failed to fill whole buffer",
            ),
        ];
        for (doc, want) in cases {
            let err = doc
                .read(&mut Diagnostics::strict())
                .expect_err(want)
                .to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn bitset_widths_follow_storage_classes() {
        assert_eq!(bitset_byte_width(1), 1);
        assert_eq!(bitset_byte_width(8), 1);
        assert_eq!(bitset_byte_width(9), 2);
        assert_eq!(bitset_byte_width(16), 2);
        assert_eq!(bitset_byte_width(17), 4);
        assert_eq!(bitset_byte_width(32), 4);
        assert_eq!(bitset_byte_width(33), 5);
        assert_eq!(bitset_byte_width(64), 8);
    }
}

#[cfg(test)]
#[path = "pform_gta6_tests.rs"]
mod pform_gta6_tests;
