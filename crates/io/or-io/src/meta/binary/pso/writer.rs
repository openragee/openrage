use std::collections::BTreeMap;

use crate::meta::binary::enum_text;
use crate::meta::binary::layout::PsoLayout;
use crate::meta::binary::EncodePath;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::{MetaInstance, Value};

use super::reader::{
    gta6_pairs, gta6_text_in_place, named_pair, GTA6_BYTES_16, GTA6_GROUP_MAP, GTA6_PAIR_MAP,
    GTA6_STRING_HASH, GTA6_STRING_HASH64,
};
use super::schema::{
    string_subtype, struct_subtype, PsoDataType, PsoStructEntry, PsoStructInfo, ARRAY_HEADER_BYTES,
    ARRAY_INFO_NAME_HASH, POINTER_BYTES,
};
use super::sections::{PsoPointer, PsoSection};

const POISON: u8 = 0xCD;

pub fn write(layout: Option<&PsoLayout>, instance: &MetaInstance) -> Result<Vec<u8>> {
    let Some(layout) = layout else {
        return Err(MetaError::Other(format!(
            "pso: cannot write '{}' without a layout. A PSO file carries its own \
             type table and block allocation, neither of which can be derived from \
             the value tree; load an existing file to obtain one, or save as XML.",
            instance.type_name
        )));
    };
    // A GTA VI P-form tree that no longer fits the allocation it arrived with is laid out anew.
    #[cfg(feature = "gta6")]
    let mut out = if layout.is_gta6() && !layout.is_table_form() {
        encode(layout, instance, None).or_else(|e| {
            if e.outgrew_its_layout() {
                super::pform_layout::write(layout, instance)
            } else {
                Err(e)
            }
        })?
    } else {
        encode(layout, instance, None)?
    };
    #[cfg(not(feature = "gta6"))]
    let out = encode(layout, instance, None)?;
    // The loader recomputes the GTA VI `CHKS` frame's checksum and rejects a mismatch, so recompute
    // it over the final body here, the one exit every save path returns through (table form, P-form
    // and the grown P-form). It is idempotent and a no-op for a non-`zppp` frame, so an unchanged
    // body stays byte-identical.
    #[cfg(feature = "gta6")]
    super::chks::write_checksum(&mut out);
    Ok(out)
}

#[derive(Debug, Clone, Default)]
pub struct CoverageReport {
    pub untouched: Vec<(usize, usize)>,
    pub total_bytes: usize,
}

impl CoverageReport {
    pub fn untouched_bytes(&self) -> usize {
        self.untouched.iter().map(|(_, len)| len).sum()
    }

    pub fn runs_longer_than(&self, min_len: usize) -> Vec<(usize, usize)> {
        self.untouched
            .iter()
            .copied()
            .filter(|(_, len)| *len > min_len)
            .collect()
    }
}

pub fn write_with_coverage(
    layout: &PsoLayout,
    instance: &MetaInstance,
) -> Result<(Vec<u8>, CoverageReport)> {
    let real = encode(layout, instance, None)?;
    let poisoned = encode(layout, instance, Some(POISON))?;

    let (start, len) = psin_span(layout, &real)?;
    let mut report = CoverageReport {
        untouched: Vec::new(),
        total_bytes: len,
    };
    let mut run_start: Option<usize> = None;
    for i in 0..len {
        let untouched = poisoned[start + i] == POISON && real[start + i] != POISON;
        match (untouched, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(s)) => {
                report.untouched.push((s, i - s));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run_start {
        report.untouched.push((s, len - s));
    }
    Ok((real, report))
}

fn psin_span(layout: &PsoLayout, out: &[u8]) -> Result<(usize, usize)> {
    let mut offset = 0usize;
    for ident in &layout.section_order {
        let len = match ident {
            PsoSection::Psin => return Ok((offset, layout.psin.len())),
            PsoSection::Pmap => layout.pmap.write().len(),
            PsoSection::Psch => layout.schema.write().len(),
            other => layout
                .opaque_sections
                .iter()
                .find(|s| s.ident == *other)
                .map(|s| s.bytes.len())
                .unwrap_or(0),
        };
        offset += len;
    }
    let _ = out;
    Err(MetaError::Other("pso: no PSIN section in layout".into()))
}

pub(super) fn encode(
    layout: &PsoLayout,
    instance: &MetaInstance,
    poison: Option<u8>,
) -> Result<Vec<u8>> {
    if layout.is_table_form() {
        #[cfg(feature = "gta6")]
        return super::table_layout::write(layout, instance);
        #[cfg(not(feature = "gta6"))]
        return Err(super::table_form::TableFormError::NoGta6.into());
    }
    if layout.psin.len() < 8 {
        return Err(MetaError::Other("pso: layout has no PSIN payload".into()));
    }

    let mut psin = layout.psin.clone();
    if let Some(byte) = poison {
        psin[8..].fill(byte);
    }

    let mut encoder = PsoEncoder {
        psin: &mut psin,
        layout,
        path: EncodePath::default(),
        gta6: layout.is_gta6(),
    };
    encoder.encode_root(instance)?;

    let mut out = Vec::with_capacity(layout.psin.len() * 2);
    for ident in &layout.section_order {
        match ident {
            PsoSection::Psin => out.extend_from_slice(&psin),
            PsoSection::Pmap => out.extend_from_slice(&layout.pmap.write()),
            PsoSection::Psch => out.extend_from_slice(&layout.schema.write()),
            other => {
                if let Some(section) = layout.opaque_sections.iter().find(|s| s.ident == *other) {
                    out.extend_from_slice(&section.bytes);
                }
            }
        }
    }
    Ok(out)
}

struct PsoEncoder<'a> {
    psin: &'a mut Vec<u8>,
    layout: &'a PsoLayout,
    path: EncodePath,
    gta6: bool,
}

impl PsoEncoder<'_> {
    fn encode_root(&mut self, instance: &MetaInstance) -> Result<()> {
        self.path.push(instance.type_name.clone());
        let block = self
            .layout
            .pmap
            .block(self.layout.pmap.root_id)
            .ok_or_else(|| MetaError::Other("pso: root block not found in PMAP".into()))?;
        let info = self
            .layout
            .schema
            .struct_for(block.name_hash)
            .ok_or_else(|| {
                MetaError::Other(format!(
                    "pso: root block schema {:#X} not in the layout",
                    block.name_hash
                ))
            })?;
        self.write_struct(info, block.offset as usize, instance)
    }

    fn write_struct(
        &mut self,
        info: &PsoStructInfo,
        base: usize,
        inst: &MetaInstance,
    ) -> Result<()> {
        let mut by_name: BTreeMap<&str, &Value> = BTreeMap::new();
        for (name, value) in &inst.fields {
            by_name.insert(name.as_str(), value);
        }

        let idx = crate::meta::binary::hash_index::default_hash_index();
        for entry in &info.entries {
            if entry.name_hash == ARRAY_INFO_NAME_HASH {
                continue;
            }
            let name = idx.name_or_literal(entry.name_hash);
            let Some(value) = by_name.get(name.as_str()) else {
                return Err(self.path.locate(MetaError::Other(format!(
                    "pso: '{}' is missing field '{name}' that its schema declares",
                    inst.type_name
                ))));
            };
            let offset = base + entry.offset() as usize;
            self.path.push(format!(".{name}"));
            self.write_member(info, entry, offset, value)?;
            self.path.pop();
        }
        Ok(())
    }

    fn write_member(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
        value: &Value,
    ) -> Result<()> {
        self.encode_member(owner, entry, offset, value)
            .map_err(|e| self.path.locate(e))
    }

    fn encode_member(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
        value: &Value,
    ) -> Result<()> {
        match entry.data_type {
            PsoDataType::Bool => self.put_u8(offset, u8::from(as_bool(value)?)),
            PsoDataType::SByte => self.put_u8(offset, as_int(value)? as i8 as u8),
            PsoDataType::UByte => self.put_u8(offset, as_uint(value)? as u8),
            PsoDataType::SShort => self.put_bytes(offset, &(as_int(value)? as i16).to_be_bytes()),
            PsoDataType::UShort => self.put_bytes(offset, &(as_uint(value)? as u16).to_be_bytes()),
            PsoDataType::SInt => self.put_bytes(offset, &(as_int(value)? as i32).to_be_bytes()),
            PsoDataType::UInt => self.put_bytes(offset, &(as_uint(value)? as u32).to_be_bytes()),
            PsoDataType::Int64 => self.put_bytes(offset, &as_int(value)?.to_be_bytes()),
            PsoDataType::UInt64 | PsoDataType::SizeT => {
                self.put_bytes(offset, &as_uint(value)?.to_be_bytes())
            }
            PsoDataType::PtrDiffT => self.put_bytes(offset, &as_int(value)?.to_be_bytes()),
            PsoDataType::Float | PsoDataType::ScalarV => {
                self.put_bytes(offset, &(as_float(value)? as f32).to_be_bytes())
            }
            PsoDataType::Double => {
                self.put_bytes(offset, &as_float(value)?.to_bits().to_be_bytes())
            }
            PsoDataType::Float16 => self.put_bytes(
                offset,
                &crate::f32_to_half(as_float(value)? as f32).to_be_bytes(),
            ),
            PsoDataType::BoolV => self.put_bytes(offset, &u32::from(as_bool(value)?).to_be_bytes()),
            PsoDataType::Vector2 | PsoDataType::Vec2V => {
                let v = as_vec2(value)?;
                self.put_f32s(offset, &v)
            }
            PsoDataType::Vector3 | PsoDataType::Vec3V => {
                let v = as_vec3(value)?;
                self.put_f32s(offset, &v)
            }
            PsoDataType::Vector4 | PsoDataType::Vec4V | PsoDataType::VecBoolV => {
                let v = as_vec4(value)?;
                self.put_f32s(offset, &v)
            }
            PsoDataType::Mat33V => self.put_matrix(offset, value, 3),
            PsoDataType::Matrix34 | PsoDataType::Mat34V => self.put_matrix(offset, value, 4),
            PsoDataType::Matrix44 | PsoDataType::Mat44V => self.put_matrix(offset, value, 4),
            PsoDataType::String => self.write_string(entry, offset, value),
            PsoDataType::Structure => self.write_struct_member(entry, offset, value),
            PsoDataType::Enum => self.write_enum(entry, offset, value),
            PsoDataType::Bitset => self.write_bitset(entry, offset, value),
            PsoDataType::Array => self.write_array(owner, entry, offset, value),
            PsoDataType::Map => self.write_map(owner, entry, offset, value),
            PsoDataType::Unknown(GTA6_BYTES_16) if self.gta6 => match value {
                Value::ByteArray(bytes) if bytes.len() == 16 => self.put_bytes(offset, bytes),
                other => Err(MetaError::Other(format!(
                    "expected 16 bytes, found {}",
                    kind_of(other)
                ))),
            },
            PsoDataType::Unknown(b) => Err(MetaError::Other(format!(
                "unknown member type {b:#04X}; the reader could not decode it \
                 so the writer cannot reproduce it"
            ))),
        }
    }

    fn write_string(&mut self, entry: &PsoStructEntry, offset: usize, value: &Value) -> Result<()> {
        if self.gta6 && entry.subtype() == string_subtype::CHAR_ARRAY {
            let layout = self.layout;
            let field = layout.psin.get(offset..offset + 16).unwrap_or_default();
            if let Some(text) = gta6_text_in_place(field)? {
                let s = as_str(value)?;
                if s.as_bytes() != text {
                    return Err(MetaError::OutgrewItsLayout(format!(
                        "string '{s}' replaces one GTA VI keeps in place, whose copy in the string block an in-place save does not place"
                    )));
                }
                return self.emit_allocation(offset, 16);
            }
        }
        match entry.subtype() {
            string_subtype::EMBEDDED_BUFFER => {
                let s = as_str(value)?;
                let cap = entry.count() as usize;
                let bytes = s.as_bytes();
                if bytes.len() > cap {
                    return Err(MetaError::Other(format!(
                        "string '{s}' needs {} bytes but the embedded buffer holds {cap}",
                        bytes.len()
                    )));
                }
                self.put_string_buffer(offset, cap, bytes)
            }
            string_subtype::POINTER | string_subtype::POINTER_ALT => {
                let ptr = self.read_pointer(offset)?;
                self.emit_allocation(offset, POINTER_BYTES)?;
                let s = as_str(value)?;
                if ptr.is_null() {
                    if s.is_empty() {
                        return Ok(());
                    }
                    return Err(MetaError::OutgrewItsLayout(format!(
                        "string '{s}' has no storage: the original pointer was null and \
                         growing the allocation is not supported"
                    )));
                }
                let base = self.block_offset(ptr)?;
                self.write_c_string(base, s, None)
            }
            string_subtype::CHAR_ARRAY => {
                let header = self.read_array_header(offset)?;
                self.emit_allocation(offset, ARRAY_HEADER_BYTES)?;
                let s = as_str(value)?;
                if header.pointer.is_null() {
                    if s.is_empty() {
                        return Ok(());
                    }
                    return Err(MetaError::Other(format!(
                        "string '{s}' has no storage: the original char array was null"
                    )));
                }
                let base = self.block_offset(header.pointer)?;
                self.write_c_string(base, s, Some(header.count as usize))
            }
            string_subtype::HASH | string_subtype::HASH_ALT => {
                let hash = as_hash(value)?;
                self.put_bytes(offset, &hash.to_be_bytes())
            }
            GTA6_STRING_HASH if self.gta6 => self.put_bytes(offset, &as_hash(value)?.to_be_bytes()),
            GTA6_STRING_HASH64 if self.gta6 => {
                self.put_bytes(offset, &as_uint(value)?.to_be_bytes())
            }
            other => Err(MetaError::Other(format!("unknown string variant {other}"))),
        }
    }

    fn write_c_string(&mut self, base: usize, s: &str, cap: Option<usize>) -> Result<()> {
        let bytes = s.as_bytes();
        let available = cap.unwrap_or_else(|| self.original_c_string_len(base) + 1);
        if bytes.len() > available {
            return Err(MetaError::OutgrewItsLayout(format!(
                "string '{s}' needs {} bytes but only {available} are allocated; \
                 growing a PSO allocation is not supported",
                bytes.len()
            )));
        }
        self.put_string_buffer(base, available, bytes)
    }

    /// ⚠ An unchanged string puts its whole original buffer back, slack
    /// included: shipped files carry stale bytes after a terminator (two
    /// Legacy `.ytyp` hold 460 and 406 bytes of leftover XML comment text),
    /// and zero-filling them breaks byte-identity. The buffer is written
    /// rather than skipped so a poisoned encode still sees every byte covered.
    fn put_string_buffer(&mut self, base: usize, len: usize, bytes: &[u8]) -> Result<()> {
        let layout = self.layout;
        if let Some(original) = layout.psin.get(base..base + len) {
            let end = original.iter().position(|&b| b == 0).unwrap_or(len);
            if &original[..end] == bytes {
                return self.put_bytes(base, original);
            }
        }
        self.fill(base, len, 0)?;
        self.put_bytes(base, bytes)
    }

    fn original_c_string_len(&self, base: usize) -> usize {
        let original = &self.layout.psin;
        original
            .get(base..)
            .and_then(|s| s.iter().position(|&b| b == 0))
            .unwrap_or(0)
    }

    fn write_struct_member(
        &mut self,
        entry: &PsoStructEntry,
        offset: usize,
        value: &Value,
    ) -> Result<()> {
        match entry.subtype() {
            struct_subtype::EMBEDDED => {
                let Value::Struct(inner) = value else {
                    return Err(MetaError::Other(format!(
                        "expected an embedded struct, found {}",
                        kind_of(value)
                    )));
                };
                let info = self
                    .layout
                    .schema
                    .struct_for(entry.referent_hash())
                    .ok_or_else(|| {
                        MetaError::Other(format!(
                            "embedded struct {:#X} is not in the layout",
                            entry.referent_hash()
                        ))
                    })?;
                self.write_struct(info, offset, inner)
            }
            struct_subtype::POINTER | struct_subtype::POINTER_ALT => {
                let ptr = self.read_pointer(offset)?;
                self.emit_allocation(offset, POINTER_BYTES)?;
                match value {
                    Value::Pointer(None) | Value::Null => {
                        if !ptr.is_null() {
                            return Err(MetaError::OutgrewItsLayout(
                                "field was set to null but the original had a target; \
                                 changing pointer topology is not supported"
                                    .into(),
                            ));
                        }
                        Ok(())
                    }
                    Value::Pointer(Some(inner)) => {
                        if ptr.is_null() {
                            return Err(MetaError::OutgrewItsLayout(
                                "field gained a target but the original pointer was null; \
                                 changing pointer topology is not supported"
                                    .into(),
                            ));
                        }
                        self.write_pointer_target(ptr, inner)
                    }
                    other => Err(MetaError::Other(format!(
                        "expected a pointer, found {}",
                        kind_of(other)
                    ))),
                }
            }
            other => Err(MetaError::Other(format!("unknown struct variant {other}"))),
        }
    }

    fn write_pointer_target(&mut self, ptr: PsoPointer, inner: &MetaInstance) -> Result<()> {
        let block = self
            .layout
            .pmap
            .block(ptr.block_id() as i32)
            .ok_or_else(|| {
                MetaError::Other(format!("pointer block {} not in PMAP", ptr.block_id()))
            })?;
        let info = self
            .layout
            .schema
            .struct_for(block.name_hash)
            .ok_or_else(|| {
                MetaError::Other(format!(
                    "pointer target schema {:#X} is not in the layout",
                    block.name_hash
                ))
            })?;
        let offset = block.offset as usize + ptr.data_offset_for(self.gta6)? as usize;
        self.write_struct(info, offset, inner)
    }

    fn write_enum(&mut self, entry: &PsoStructEntry, offset: usize, value: &Value) -> Result<()> {
        let Value::Enum { variant, .. } = value else {
            return Err(MetaError::Other(format!(
                "expected an enum, found {}",
                kind_of(value)
            )));
        };
        let numeric = self.resolve_enum_value(entry, variant)?;
        match entry.subtype() {
            2 => {
                if !enum_text::fits_in_bytes(numeric, 1) {
                    return Err(MetaError::Other(format!(
                        "'{variant}' is {numeric}, which does not fit the single byte this \
                         enum member occupies"
                    )));
                }
                self.put_u8(offset, numeric as u8)
            }
            _ => self.put_bytes(offset, &numeric.to_be_bytes()),
        }
    }

    fn resolve_enum_value(&self, entry: &PsoStructEntry, variant: &str) -> Result<i32> {
        let info = self.layout.schema.enum_for(entry.referent_hash());
        if let Some(info) = info {
            if let Some(value) = enum_text::resolve_named(
                variant,
                info.entries.iter().map(|e| (e.name_hash, e.value)),
            ) {
                return Ok(value);
            }
        }
        if let Some(value) = enum_text::value_literal(variant) {
            return Ok(value);
        }
        Err(MetaError::Other(match info {
            Some(_) => format!("'{variant}' is not a value of this enum"),
            None => format!(
                "enum type {:#X} is not in the layout, and '{variant}' is not a numeric value",
                entry.referent_hash()
            ),
        }))
    }

    fn write_bitset(&mut self, entry: &PsoStructEntry, offset: usize, value: &Value) -> Result<()> {
        let Value::Bitset { bits, .. } = value else {
            return Err(MetaError::Other(format!(
                "expected a bitset, found {}",
                kind_of(value)
            )));
        };
        let width = super::reader::bitset_byte_width(entry.count()).min(16);
        for i in 0..width {
            // Big-endian: the first byte holds the most significant bits.
            let shift = 8 * (width - 1 - i);
            self.put_u8(offset + i, ((bits >> shift) & 0xFF) as u8)?;
        }
        Ok(())
    }

    fn write_array(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
        value: &Value,
    ) -> Result<()> {
        let Value::Array(items) = value else {
            return Err(MetaError::Other(format!(
                "expected an array, found {}",
                kind_of(value)
            )));
        };
        let Some(elem_index) = entry.member_index() else {
            return Ok(());
        };
        let Some(elem) = owner.entry(elem_index).copied() else {
            return Err(MetaError::Other(format!(
                "array element descriptor index {elem_index} is out of range"
            )));
        };

        let inline_count = entry.count();
        let (base, count) = if inline_count == 0 {
            let header = if self.gta6 && entry.subtype() == super::reader::GTA6_COUNT_FIRST_ARRAY {
                ArrayHeader {
                    count: self.original_u32(offset)?,
                    pointer: self.read_pointer(offset + 8)?,
                }
            } else {
                self.read_array_header(offset)?
            };
            self.emit_allocation(offset, ARRAY_HEADER_BYTES)?;
            if header.pointer.is_null() || header.count == 0 {
                if !items.is_empty() {
                    return Err(MetaError::OutgrewItsLayout(format!(
                        "array gained {} elements but the original was empty; \
                         growing a PSO allocation is not supported",
                        items.len()
                    )));
                }
                return Ok(());
            }
            (self.block_offset(header.pointer)?, header.count as usize)
        } else {
            (offset, inline_count as usize)
        };

        if items.len() != count {
            return Err(MetaError::OutgrewItsLayout(format!(
                "array has {} elements but {count} are allocated; resizing a PSO \
                 array is not supported",
                items.len()
            )));
        }

        let Some(stride) = self.element_stride(&elem) else {
            return Err(MetaError::Other(format!(
                "array of {:?} has no known element size",
                elem.data_type
            )));
        };
        for (i, item) in items.iter().enumerate() {
            self.path.push(format!("[{i}]"));
            self.write_member(owner, &elem, base + stride * i, item)?;
            self.path.pop();
        }
        Ok(())
    }

    fn write_map(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
        value: &Value,
    ) -> Result<()> {
        let Value::Map(pairs) = value else {
            return Err(MetaError::Other(format!(
                "expected a map, found {}",
                kind_of(value)
            )));
        };
        if self.gta6 {
            return self.write_gta6_map(owner, entry, offset, pairs);
        }
        let (Some(key_desc), Some(value_desc)) = (
            owner.entry(entry.key_index()).copied(),
            owner.entry(entry.value_index()).copied(),
        ) else {
            return Ok(());
        };
        let header = self.read_array_header(offset)?;
        self.emit_allocation(offset, ARRAY_HEADER_BYTES)?;
        if header.pointer.is_null() || header.count == 0 {
            if !pairs.is_empty() {
                return Err(MetaError::OutgrewItsLayout(format!(
                    "map gained {} entries but the original was empty",
                    pairs.len()
                )));
            }
            return Ok(());
        }
        if pairs.len() != header.count as usize {
            return Err(MetaError::OutgrewItsLayout(format!(
                "map has {} entries but {} are allocated",
                pairs.len(),
                header.count
            )));
        }
        let (Some(key_stride), Some(value_stride)) = (
            self.element_stride(&key_desc),
            self.element_stride(&value_desc),
        ) else {
            return Err(MetaError::Other("map key/value has no known size".into()));
        };
        let (_, pair_size) = super::schema::map_pair_layout(key_stride, value_stride);
        let base = self.block_offset(header.pointer)?;
        for (i, (k, v)) in pairs.iter().enumerate() {
            let entry_base = base + pair_size * i;
            self.path.push(format!("[{i}].key"));
            self.write_member(owner, &key_desc, entry_base, k)?;
            self.path.pop();
            self.path.push(format!("[{i}].value"));
            self.write_member(owner, &value_desc, entry_base + key_stride, v)?;
            self.path.pop();
        }
        Ok(())
    }

    /// A GTA VI map's pairs into the ones it holds; a group map's only with GTA VI support.
    fn write_gta6_map(
        &mut self,
        owner: &PsoStructInfo,
        entry: &PsoStructEntry,
        offset: usize,
        pairs: &[(Value, Value)],
    ) -> Result<()> {
        let (header, count) = match entry.subtype() {
            GTA6_PAIR_MAP => (24, usize::from(self.original_u16(offset + 16)?)),
            GTA6_GROUP_MAP => (16, self.original_u32(offset + 4)? as usize),
            other => {
                return Err(MetaError::Other(format!(
                    "GTA VI map subtype {other}, whose layout is not measured"
                )))
            }
        };
        self.emit_allocation(offset, header)?;
        if pairs.len() != count {
            return Err(MetaError::OutgrewItsLayout(format!(
                "map has {} entries but {count} are allocated; growing a GTA VI map in place is not supported",
                pairs.len()
            )));
        }
        if count == 0 {
            return Ok(());
        }
        if entry.subtype() == GTA6_GROUP_MAP {
            #[cfg(feature = "gta6")]
            return self.write_group_map(owner, entry, offset, pairs);
            #[cfg(not(feature = "gta6"))]
            let _ = owner;
            #[cfg(not(feature = "gta6"))]
            return Err(MetaError::Other(
                "a GTA VI group map that holds pairs, whose values' layout is not measured".into(),
            ));
        }
        let pointer = self.read_pointer(offset + 8)?;
        let layout = self.layout;
        let (pair, base) = gta6_pairs(&layout.pmap, &layout.schema, pointer)?;
        let [key, value] = named_pair(pair)?;
        let size = pair.data_size as usize;
        for (i, (k, v)) in pairs.iter().enumerate() {
            let at = base + size * i;
            self.path.push(format!("[{i}].key"));
            self.write_member(pair, &key, at + key.offset() as usize, k)?;
            self.path.pop();
            self.path.push(format!("[{i}].value"));
            self.write_member(pair, &value, at + value.offset() as usize, v)?;
            self.path.pop();
        }
        Ok(())
    }

    fn element_stride(&self, elem: &PsoStructEntry) -> Option<usize> {
        super::schema::element_stride(&self.layout.schema, elem)
    }

    fn block_offset(&self, ptr: PsoPointer) -> Result<usize> {
        let block = self
            .layout
            .pmap
            .block(ptr.block_id() as i32)
            .ok_or_else(|| MetaError::Other(format!("block {} not in PMAP", ptr.block_id())))?;
        Ok(block.offset as usize + ptr.data_offset_for(self.gta6)? as usize)
    }

    fn read_pointer(&self, offset: usize) -> Result<PsoPointer> {
        let raw = self.original_u64(offset)?;
        // Wire order is low u32 then high u32, both big-endian.
        let low = (raw >> 32) & 0xFFFF_FFFF;
        let high = raw & 0xFFFF_FFFF;
        Ok(PsoPointer {
            raw: low | (high << 32),
        })
    }

    fn read_array_header(&self, offset: usize) -> Result<ArrayHeader> {
        let pointer = self.read_pointer(offset)?;
        let count = self.original_u16(offset + 8)?.into();
        Ok(ArrayHeader { pointer, count })
    }

    fn original_u16(&self, offset: usize) -> Result<u16> {
        let b = self
            .layout
            .psin
            .get(offset..offset + 2)
            .ok_or_else(|| MetaError::Other(format!("read past PSIN at {offset:#X}")))?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn original_u32(&self, offset: usize) -> Result<u32> {
        let b = self
            .layout
            .psin
            .get(offset..offset + 4)
            .ok_or_else(|| MetaError::Other(format!("read past PSIN at {offset:#X}")))?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn original_u64(&self, offset: usize) -> Result<u64> {
        let b = self
            .layout
            .psin
            .get(offset..offset + 8)
            .ok_or_else(|| MetaError::Other(format!("read past PSIN at {offset:#X}")))?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    fn put_u8(&mut self, offset: usize, byte: u8) -> Result<()> {
        let slot = self
            .psin
            .get_mut(offset)
            .ok_or_else(|| MetaError::Other(format!("write past PSIN at {offset:#X}")))?;
        *slot = byte;
        Ok(())
    }

    fn put_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<()> {
        let end = offset + bytes.len();
        let slot = self
            .psin
            .get_mut(offset..end)
            .ok_or_else(|| MetaError::Other(format!("write past PSIN at {offset:#X}")))?;
        slot.copy_from_slice(bytes);
        Ok(())
    }

    fn fill(&mut self, offset: usize, len: usize, byte: u8) -> Result<()> {
        let slot = self
            .psin
            .get_mut(offset..offset + len)
            .ok_or_else(|| MetaError::Other(format!("write past PSIN at {offset:#X}")))?;
        slot.fill(byte);
        Ok(())
    }

    fn emit_allocation(&mut self, offset: usize, len: usize) -> Result<()> {
        let src = self
            .layout
            .psin
            .get(offset..offset + len)
            .ok_or_else(|| MetaError::Other(format!("read past PSIN at {offset:#X}")))?
            .to_vec();
        self.put_bytes(offset, &src)
    }

    fn put_f32s(&mut self, offset: usize, values: &[f32]) -> Result<()> {
        for (i, v) in values.iter().enumerate() {
            self.put_bytes(offset + i * 4, &v.to_be_bytes())?;
        }
        Ok(())
    }

    fn put_matrix(&mut self, offset: usize, value: &Value, rows: usize) -> Result<()> {
        let Value::Mat34(m) = value else {
            return Err(MetaError::Other(format!(
                "expected a matrix, found {}",
                kind_of(value)
            )));
        };
        for (r, row) in m.iter().enumerate() {
            if r >= rows {
                break;
            }
            self.put_f32s(offset + r * 16, row)?;
        }
        Ok(())
    }
}

struct ArrayHeader {
    pointer: PsoPointer,
    count: u32,
}

#[cfg(feature = "gta6")]
#[path = "pform_group_write.rs"]
mod group_write;

fn as_bool(v: &Value) -> Result<bool> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::UInt(n) => Ok(*n != 0),
        Value::Int(n) => Ok(*n != 0),
        other => Err(MetaError::Other(format!(
            "expected a bool, found {}",
            kind_of(other)
        ))),
    }
}

fn as_int(v: &Value) -> Result<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        Value::UInt(n) => Ok(*n as i64),
        Value::Bool(b) => Ok(i64::from(*b)),
        other => Err(MetaError::Other(format!(
            "expected an integer, found {}",
            kind_of(other)
        ))),
    }
}

fn as_uint(v: &Value) -> Result<u64> {
    match v {
        Value::UInt(n) => Ok(*n),
        Value::Int(n) => Ok(*n as u64),
        Value::Bool(b) => Ok(u64::from(*b)),
        Value::Color(c) => Ok(u64::from(*c)),
        other => Err(MetaError::Other(format!(
            "expected an integer, found {}",
            kind_of(other)
        ))),
    }
}

fn as_float(v: &Value) -> Result<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => Ok(*n as f64),
        Value::UInt(n) => Ok(*n as f64),
        other => Err(MetaError::Other(format!(
            "expected a float, found {}",
            kind_of(other)
        ))),
    }
}

fn as_str(v: &Value) -> Result<&str> {
    match v {
        Value::String(s) => Ok(s),
        Value::Hash(h) => Ok(&h.raw),
        other => Err(MetaError::Other(format!(
            "expected a string, found {}",
            kind_of(other)
        ))),
    }
}

fn as_hash(v: &Value) -> Result<u32> {
    match v {
        Value::Hash(h) => Ok(h.hash),
        Value::UInt(n) => Ok(*n as u32),
        Value::String(s) => Ok(crate::string_hash(s.as_bytes())),
        other => Err(MetaError::Other(format!(
            "expected a hash, found {}",
            kind_of(other)
        ))),
    }
}

fn as_vec2(v: &Value) -> Result<[f32; 2]> {
    match v {
        Value::Vec2(a) => Ok(*a),
        other => Err(MetaError::Other(format!(
            "expected a Vec2, found {}",
            kind_of(other)
        ))),
    }
}

fn as_vec3(v: &Value) -> Result<[f32; 3]> {
    match v {
        Value::Vec3(a) => Ok(*a),
        other => Err(MetaError::Other(format!(
            "expected a Vec3, found {}",
            kind_of(other)
        ))),
    }
}

fn as_vec4(v: &Value) -> Result<[f32; 4]> {
    match v {
        Value::Vec4(a) => Ok(*a),
        other => Err(MetaError::Other(format!(
            "expected a Vec4, found {}",
            kind_of(other)
        ))),
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a bool",
        Value::Int(_) => "an int",
        Value::UInt(_) => "a uint",
        Value::Float(_) => "a float",
        Value::String(_) => "a string",
        Value::Hash(_) => "a hash",
        Value::Vec2(_) => "a Vec2",
        Value::Vec3(_) => "a Vec3",
        Value::Vec4(_) => "a Vec4",
        Value::Mat34(_) => "a Mat34",
        Value::Color(_) => "a colour",
        Value::Enum { .. } => "an enum",
        Value::Bitset { .. } => "a bitset",
        Value::Struct(_) => "a struct",
        Value::Pointer(_) => "a pointer",
        Value::Array(_) => "an array",
        Value::Map(_) => "a map",
        Value::ByteArray(_) => "a byte array",
        Value::Raw(_) => "an undecoded placeholder",
    }
}
