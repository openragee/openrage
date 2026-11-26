use crate::container::resource::{Compression, ResourceEnvelope, Rsc7};

use crate::meta::binary::enum_text;
use crate::meta::binary::layout::RbmLayout;
use crate::meta::binary::EncodePath;
use crate::meta::error::{MetaError, Result, UnsupportedShape};
use crate::meta::instance::value::{MetaInstance, Value};

use super::schema::{PsoType, RbmClass, RbmClassEntry};

const POISON: u8 = 0xCD;

const POINTER_BYTES: usize = 8;
const ARRAY_HEADER_BYTES: usize = 16;

pub fn write(layout: Option<&RbmLayout>, instance: &MetaInstance) -> Result<Vec<u8>> {
    let Some(layout) = layout else {
        return Err(MetaError::Other(format!(
            "rbm: cannot write '{}' without a layout. An RBM resource carries its own \
             type table and block allocation, neither of which can be derived from the \
             value tree; load an existing file to obtain one, or save as XML.",
            instance.type_name
        )));
    };
    #[cfg(feature = "gta6")]
    if layout.seal.is_none() {
        return super::gta6::write_loose(layout, instance);
    }
    let payload = encode_payload(layout, instance, None)?;
    wrap_resource(layout, payload)
}

pub type UntouchedRuns = Vec<(usize, usize)>;

pub fn write_with_coverage(
    layout: &RbmLayout,
    instance: &MetaInstance,
) -> Result<(Vec<u8>, UntouchedRuns)> {
    let real = encode_payload(layout, instance, None)?;
    let poisoned = encode_payload(layout, instance, Some(POISON))?;

    let mut untouched = Vec::new();
    let mut run_start: Option<usize> = None;
    for i in 0..real.len().min(poisoned.len()) {
        let missed = poisoned[i] == POISON && real[i] != POISON;
        match (missed, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(s)) => {
                untouched.push((s, i - s));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run_start {
        untouched.push((s, real.len() - s));
    }
    Ok((real, untouched))
}

fn wrap_resource(layout: &RbmLayout, payload: Vec<u8>) -> Result<Vec<u8>> {
    let Some(seal) = &layout.seal else {
        return Err(super::gta6::Gta6RbmError::NoResourceHeader.into());
    };
    let expected = layout.virtual_page_size + layout.physical_page_size;
    if payload.len() != expected {
        return Err(MetaError::Other(format!(
            "rbm: payload is {} bytes but the page flags describe {expected}",
            payload.len()
        )));
    }
    Rsc7::reseal(&payload, seal, Compression::best())
        .map_err(|e| MetaError::Other(format!("rbm: reseal: {e}")))
}

fn encode_payload(
    layout: &RbmLayout,
    instance: &MetaInstance,
    poison: Option<u8>,
) -> Result<Vec<u8>> {
    if layout.seal.is_none() {
        return Err(super::gta6::Gta6RbmError::NoResourceHeader.into());
    }
    let mut buffer = layout.decompressed.clone();
    if let Some(byte) = poison {
        for block in &layout.model.data_blocks {
            if let Some(offset) = resolve(layout, block.data_addr) {
                let end = (offset + block.data.len()).min(buffer.len());
                buffer[offset..end].fill(byte);
            }
        }
    }

    let mut encoder = RbmEncoder {
        layout,
        buffer,
        path: EncodePath::default(),
    };
    encoder.encode_root(instance)?;
    Ok(encoder.buffer)
}

fn resolve(layout: &RbmLayout, addr: u64) -> Option<usize> {
    const VIRTUAL_BASE: u64 = 0x5000_0000;
    const PHYSICAL_BASE: u64 = 0x6000_0000;
    const PAGE_MASK: u64 = 0x0FFF_FFFF;
    if addr == 0 {
        return None;
    }
    let base = addr & !PAGE_MASK;
    let offset = (addr & PAGE_MASK) as usize;
    if base == VIRTUAL_BASE {
        Some(offset)
    } else if base == PHYSICAL_BASE {
        Some(layout.virtual_page_size + offset)
    } else {
        None
    }
}

struct RbmEncoder<'a> {
    layout: &'a RbmLayout,
    buffer: Vec<u8>,
    path: EncodePath,
}

impl RbmEncoder<'_> {
    fn encode_root(&mut self, instance: &MetaInstance) -> Result<()> {
        self.path.push(instance.type_name.clone());
        let model = &self.layout.model;
        let block = model
            .block(model.root_block_id as i32)
            .ok_or_else(|| MetaError::Other("rbm: root data block missing".into()))?;
        let class = model
            .class_for(block.data_type)
            .ok_or_else(|| {
                MetaError::Other(format!(
                    "rbm: root schema {:#X} is not in the layout",
                    block.data_type
                ))
            })?
            .clone();
        let base = self.block_base(model.root_block_id)?;
        self.write_struct(&class, base, instance)
    }

    fn block_base(&self, block_id: u32) -> Result<usize> {
        let block = self
            .layout
            .model
            .block(block_id as i32)
            .ok_or_else(|| MetaError::Other(format!("rbm: block {block_id} missing")))?;
        resolve(self.layout, block.data_addr)
            .ok_or_else(|| MetaError::Other(format!("rbm: block {block_id} has no address")))
    }

    fn write_struct(&mut self, class: &RbmClass, base: usize, inst: &MetaInstance) -> Result<()> {
        let idx = crate::meta::binary::hash_index::default_hash_index();
        for entry in &class.entries {
            if entry.name_hash == ARRAY_INFO_NAME_HASH {
                continue;
            }
            let name = idx.name_or_literal(entry.name_hash);
            let Some(value) = inst.get(&name) else {
                return Err(self.path.locate(MetaError::Other(format!(
                    "rbm: '{}' is missing field '{name}' that its schema declares",
                    inst.type_name
                ))));
            };
            let offset = base + entry.offset as usize;
            self.path.push(format!(".{name}"));
            self.write_member(class, entry, offset, value)?;
            self.path.pop();
        }
        Ok(())
    }

    fn write_member(
        &mut self,
        owner: &RbmClass,
        entry: &RbmClassEntry,
        off: usize,
        value: &Value,
    ) -> Result<()> {
        self.encode_member(owner, entry, off, value)
            .map_err(|e| self.path.locate(e))
    }

    fn encode_member(
        &mut self,
        owner: &RbmClass,
        entry: &RbmClassEntry,
        off: usize,
        value: &Value,
    ) -> Result<()> {
        match entry.data_type {
            PsoType::Bool => self.put(off, &[u8::from(as_bool(value)?)]),
            PsoType::S8 => self.put(off, &[as_int(value)? as i8 as u8]),
            PsoType::U8 => self.put(off, &[as_uint(value)? as u8]),
            PsoType::S16 => self.put(off, &(as_int(value)? as i16).to_le_bytes()),
            PsoType::U16 => self.put(off, &(as_uint(value)? as u16).to_le_bytes()),
            PsoType::S32 => self.put(off, &(as_int(value)? as i32).to_le_bytes()),
            PsoType::U32 => self.put(off, &(as_uint(value)? as u32).to_le_bytes()),
            PsoType::S64 => self.put(off, &as_int(value)?.to_le_bytes()),
            PsoType::U64 => self.put(off, &as_uint(value)?.to_le_bytes()),
            PsoType::Float | PsoType::ScalarV => {
                self.put(off, &(as_float(value)? as f32).to_le_bytes())
            }
            PsoType::Double => self.put(off, &as_float(value)?.to_bits().to_le_bytes()),
            PsoType::Float16 => self.put(
                off,
                &crate::f32_to_half(as_float(value)? as f32).to_le_bytes(),
            ),
            PsoType::BoolV => self.put(off, &u32::from(as_bool(value)?).to_le_bytes()),
            PsoType::Vec2 | PsoType::Vec2V => self.put_f32s(off, &as_vec2(value)?),
            PsoType::Vec3 | PsoType::Vec3V => self.put_f32s(off, &as_vec3(value)?),
            PsoType::Vec4V | PsoType::VecBoolV => self.put_f32s(off, &as_vec4(value)?),
            PsoType::Mat33V | PsoType::Mat43 => self.put_matrix(off, value, 3),
            PsoType::Mat34V | PsoType::Mat44V => self.put_matrix(off, value, 4),
            PsoType::StringHash
            | PsoType::LiteralStringHash
            | PsoType::PartialStringHash
            | PsoType::StructId
            | PsoType::NsStringHash => self.put(off, &as_hash(value)?.to_le_bytes()),
            PsoType::Enum8 => {
                let v = self.resolve_enum(entry, value, 1)?;
                self.put(off, &[v as u8])
            }
            PsoType::Enum16 => {
                let v = self.resolve_enum(entry, value, 2)?;
                self.put(off, &(v as u16).to_le_bytes())
            }
            PsoType::Enum32 => {
                let v = self.resolve_enum(entry, value, 4)?;
                self.put(off, &v.to_le_bytes())
            }
            PsoType::Bitset8 => self.put(off, &[as_bits(value)? as u8]),
            PsoType::Bitset16 => self.put(off, &(as_bits(value)? as u16).to_le_bytes()),
            PsoType::Bitset32 => self.put(off, &(as_bits(value)? as u32).to_le_bytes()),
            PsoType::AtBitset32 | PsoType::AtBitset64 => {
                let header = self.read_array_header(off)?;
                self.emit_allocation(off, ARRAY_HEADER_BYTES)?;
                if header.block_id == 0 {
                    return Ok(());
                }
                let base = self.block_base(header.block_id)? + header.data_offset as usize;
                let bits = as_bits(value)?;
                for i in 0..(header.count as usize).min(4) {
                    let word = ((bits >> (32 * i)) & 0xFFFF_FFFF) as u32;
                    self.put(base + 4 * i, &word.to_le_bytes())?;
                }
                Ok(())
            }
            PsoType::Struct => {
                let Value::Struct(inner) = value else {
                    return Err(MetaError::Other(format!(
                        "expected an embedded struct, found {}",
                        kind_of(value)
                    )));
                };
                let class = self
                    .layout
                    .model
                    .class_for(entry.referent_hash())
                    .ok_or_else(|| {
                        MetaError::Other(format!(
                            "embedded struct {:#X} is not in the layout",
                            entry.referent_hash()
                        ))
                    })?
                    .clone();
                self.write_struct(&class, off, inner)
            }
            PsoType::Pointer32 | PsoType::Pointer64 => {
                let ptr = self.read_pointer(off)?;
                self.emit_allocation(off, POINTER_BYTES)?;
                match value {
                    Value::Pointer(None) | Value::Null => Ok(()),
                    Value::Pointer(Some(inner)) => {
                        if ptr.block_id == 0 {
                            return Err(MetaError::OutgrewItsLayout(
                                "pointer gained a target but the original was null; \
                                 changing pointer topology is not supported"
                                    .into(),
                            ));
                        }
                        let target = self
                            .layout
                            .model
                            .block(ptr.block_id as i32)
                            .map(|b| b.data_type)
                            .ok_or_else(|| {
                                MetaError::Other(format!("pointer block {} missing", ptr.block_id))
                            })?;
                        let class = self
                            .layout
                            .model
                            .class_for(target)
                            .ok_or_else(|| {
                                MetaError::Other(format!(
                                    "pointer target schema {target:#X} is not in the layout"
                                ))
                            })?
                            .clone();
                        let base = self.block_base(ptr.block_id)? + ptr.data_offset as usize;
                        self.write_struct(&class, base, inner)
                    }
                    other => Err(MetaError::Other(format!(
                        "expected a pointer, found {}",
                        kind_of(other)
                    ))),
                }
            }
            PsoType::StringMember | PsoType::WideStringMember => {
                let s = as_str(value)?;
                let cap = entry.string_count() as usize;
                if s.len() > cap {
                    return Err(MetaError::Other(format!(
                        "string '{s}' needs {} bytes but the buffer holds {cap}",
                        s.len()
                    )));
                }
                self.fill(off, cap, 0)?;
                self.put(off, s.as_bytes())
            }
            PsoType::AtString32 | PsoType::AtString64 => {
                let header = self.read_array_header(off)?;
                self.emit_allocation(off, ARRAY_HEADER_BYTES)?;
                let s = as_str(value)?;
                if header.block_id == 0 {
                    if s.is_empty() {
                        return Ok(());
                    }
                    return Err(MetaError::Other(format!(
                        "string '{s}' has no storage: the original was null"
                    )));
                }
                let cap = header.count as usize;
                if s.len() > cap {
                    return Err(MetaError::OutgrewItsLayout(format!(
                        "string '{s}' needs {} bytes but {cap} are allocated",
                        s.len()
                    )));
                }
                let base = self.block_base(header.block_id)? + header.data_offset as usize;
                self.fill(base, cap, 0)?;
                self.put(base, s.as_bytes())
            }
            PsoType::StringPointer32 | PsoType::StringPointer64 => {
                let ptr = self.read_pointer(off)?;
                self.emit_allocation(off, POINTER_BYTES)?;
                let s = as_str(value)?;
                if ptr.block_id == 0 {
                    if s.is_empty() {
                        return Ok(());
                    }
                    return Err(MetaError::Other(format!(
                        "string '{s}' has no storage: the original pointer was null"
                    )));
                }
                let base = self.block_base(ptr.block_id)? + ptr.data_offset as usize;
                let cap = self.original_c_string_len(base) + 1;
                if s.len() >= cap {
                    return Err(MetaError::OutgrewItsLayout(format!(
                        "string '{s}' needs {} bytes but only {cap} are allocated",
                        s.len() + 1
                    )));
                }
                self.fill(base, cap, 0)?;
                self.put(base, s.as_bytes())
            }
            PsoType::ArrayMember | PsoType::AtFixedArray => {
                let count = entry.array_count() as usize;
                self.write_array_elements(owner, entry, off, count, value)
            }
            PsoType::AtArray32
            | PsoType::AtArray64
            | PsoType::AtArray32With32BitIdx
            | PsoType::AtArray64With32BitIdx => {
                let header = self.read_array_header(off)?;
                self.emit_allocation(off, ARRAY_HEADER_BYTES)?;
                let Value::Array(items) = value else {
                    return Err(MetaError::Other(format!(
                        "expected an array, found {}",
                        kind_of(value)
                    )));
                };
                if header.block_id == 0 || header.count == 0 {
                    if !items.is_empty() {
                        return Err(MetaError::OutgrewItsLayout(format!(
                            "array gained {} elements but the original was empty",
                            items.len()
                        )));
                    }
                    return Ok(());
                }
                let base = self.block_base(header.block_id)? + header.data_offset as usize;
                self.write_array_elements(owner, entry, base, header.count as usize, value)
            }
            PsoType::ArrayPointer32
            | PsoType::ArrayPointer64
            | PsoType::ArrayPointer32WithCount
            | PsoType::ArrayPointer64WithCount => {
                let ptr = self.read_pointer(off)?;
                self.emit_allocation(off, POINTER_BYTES)?;
                let bytes = match value {
                    Value::ByteArray(bytes) => bytes.clone(),
                    // What the same bytes import as from XML, typed by a schema
                    // that calls the member an array of `u8`.
                    Value::Array(items) => items
                        .iter()
                        .map(|v| as_uint(v).map(|n| n as u8))
                        .collect::<Result<Vec<u8>>>()?,
                    Value::Null => Vec::new(),
                    _ => return Ok(()),
                };
                // ⚠ A run is read back as its block from the pointer to the block's end, and nothing here checks a
                // write against that end: a run of another length would be cut short or keep the end of the run it
                // replaces, and a longer one would write into the block that follows.
                let allocated = match ptr.block_id {
                    0 => 0,
                    id => {
                        let block = self.layout.model.block(id as i32);
                        let block = block
                            .ok_or_else(|| MetaError::Other(format!("rbm: block {id} missing")))?;
                        block.data.len().saturating_sub(ptr.data_offset as usize)
                    }
                };
                if bytes.len() != allocated {
                    return Err(MetaError::OutgrewItsLayout(format!(
                        "a run of {} bytes where {allocated} are allocated",
                        bytes.len()
                    )));
                }
                if ptr.block_id == 0 {
                    return Ok(());
                }
                let base = self.block_base(ptr.block_id)? + ptr.data_offset as usize;
                self.put(base, &bytes)
            }
            PsoType::AtBinaryMap32 | PsoType::AtBinaryMap64 => {
                self.emit_allocation(off, ARRAY_HEADER_BYTES)?;
                match value {
                    Value::Map(pairs) if pairs.is_empty() => Ok(()),
                    Value::Map(pairs) => Err(MetaError::UnsupportedShape {
                        shape: UnsupportedShape::PopulatedAtBinaryMap,
                        detail: format!(
                            "{} entries in field {:#010X}; every atBinaryMap in a 6,071-file \
                             corpus is empty, so there is no case to encode against",
                            pairs.len(),
                            entry.name_hash
                        ),
                    }),
                    other => Err(MetaError::Other(format!(
                        "expected a map, found {}",
                        kind_of(other)
                    ))),
                }
            }
            PsoType::Invalid => Ok(()),
            PsoType::AtWideString32
            | PsoType::AtWideString64
            | PsoType::WideStringPointer32
            | PsoType::WideStringPointer64 => Err(MetaError::UnsupportedShape {
                shape: UnsupportedShape::WideString,
                detail: format!(
                    "{:?} in field {:#010X}; the reader decodes wide strings as empty, so \
                     nothing survives to write back",
                    entry.data_type, entry.name_hash
                ),
            }),
            PsoType::Unknown(b) => Err(MetaError::Other(format!(
                "unknown member type {b:#04X}; the reader could not decode it \
                 so the writer cannot reproduce it"
            ))),
        }
    }

    fn write_array_elements(
        &mut self,
        owner: &RbmClass,
        array_entry: &RbmClassEntry,
        base: usize,
        count: usize,
        value: &Value,
    ) -> Result<()> {
        let Value::Array(items) = value else {
            return Err(MetaError::Other(format!(
                "expected an array, found {}",
                kind_of(value)
            )));
        };
        if items.len() != count {
            return Err(MetaError::OutgrewItsLayout(format!(
                "array has {} elements but {count} are allocated; resizing an RBM \
                 array is not supported",
                items.len()
            )));
        }
        let Some(elem) = owner.entry(array_entry.contents_index()).copied() else {
            return Err(MetaError::Other(format!(
                "array element descriptor index {} is out of range",
                array_entry.contents_index()
            )));
        };
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

    fn element_stride(&self, elem: &RbmClassEntry) -> Option<usize> {
        match elem.data_type {
            PsoType::Struct => self
                .layout
                .model
                .class_for(elem.referent_hash())
                .map(|c| c.data_size as usize),
            PsoType::StringMember | PsoType::WideStringMember => Some(elem.string_count() as usize),
            other => other.fixed_size(),
        }
    }

    /// Map a variant back to the number the file stores for it, in `bytes`.
    ///
    /// ⚠ **The file's own enum table is asked first, and the `0xHHHHHHHH`
    /// literal is only a fallback** — the other order wrote a name hash where
    /// the value belonged, and `Enum8` then narrowed it to its low byte. See
    /// [`crate::meta::binary::enum_text`].
    ///
    /// A value wider than the field is refused: it cannot arise on a
    /// round-trip, but on an authored or text-edited document a value silently
    /// losing its top bits is a *different variant* on disk.
    fn resolve_enum(&self, entry: &RbmClassEntry, value: &Value, bytes: u32) -> Result<i32> {
        let Value::Enum { variant, .. } = value else {
            return Err(MetaError::Other(format!(
                "expected an enum, found {}",
                kind_of(value)
            )));
        };
        let info = self.layout.model.enum_for(entry.referent_hash());
        let resolved = info
            .and_then(|info| {
                enum_text::resolve_named(
                    variant,
                    info.values.iter().map(|v| (v.name_hash, v.value)),
                )
            })
            .or_else(|| enum_text::value_literal(variant));
        let Some(resolved) = resolved else {
            return Err(MetaError::Other(match info {
                Some(_) => format!("'{variant}' is not a value of this enum"),
                None => format!(
                    "enum type {:#X} is not in the layout, and '{variant}' is not a numeric value",
                    entry.referent_hash()
                ),
            }));
        };
        if !enum_text::fits_in_bytes(resolved, bytes) {
            return Err(MetaError::Other(format!(
                "'{variant}' is {resolved}, which does not fit the {bytes} byte(s) this enum \
                 member occupies"
            )));
        }
        Ok(resolved)
    }

    fn read_pointer(&self, off: usize) -> Result<PackedPointer> {
        let raw = self.original_u64(off)?;
        Ok(PackedPointer {
            block_id: (raw & 0xFFF) as u32,
            data_offset: ((raw >> 12) & 0xFFFFF) as u32,
        })
    }

    fn read_array_header(&self, off: usize) -> Result<ArrayHeader> {
        let p = self.read_pointer(off)?;
        let count = self.original_u16(off + 8)?;
        Ok(ArrayHeader {
            block_id: p.block_id,
            data_offset: p.data_offset,
            count,
        })
    }

    fn original_u16(&self, off: usize) -> Result<u16> {
        let b = self
            .layout
            .decompressed
            .get(off..off + 2)
            .ok_or_else(|| MetaError::Other(format!("read past payload at {off:#X}")))?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn original_u64(&self, off: usize) -> Result<u64> {
        let b = self
            .layout
            .decompressed
            .get(off..off + 8)
            .ok_or_else(|| MetaError::Other(format!("read past payload at {off:#X}")))?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn original_c_string_len(&self, base: usize) -> usize {
        self.layout
            .decompressed
            .get(base..)
            .and_then(|s| s.iter().position(|&b| b == 0))
            .unwrap_or(0)
    }

    fn emit_allocation(&mut self, off: usize, len: usize) -> Result<()> {
        let src = self
            .layout
            .decompressed
            .get(off..off + len)
            .ok_or_else(|| MetaError::Other(format!("read past payload at {off:#X}")))?
            .to_vec();
        self.put(off, &src)
    }

    fn put(&mut self, off: usize, bytes: &[u8]) -> Result<()> {
        let slot = self
            .buffer
            .get_mut(off..off + bytes.len())
            .ok_or_else(|| MetaError::Other(format!("write past payload at {off:#X}")))?;
        slot.copy_from_slice(bytes);
        Ok(())
    }

    fn fill(&mut self, off: usize, len: usize, byte: u8) -> Result<()> {
        let slot = self
            .buffer
            .get_mut(off..off + len)
            .ok_or_else(|| MetaError::Other(format!("write past payload at {off:#X}")))?;
        slot.fill(byte);
        Ok(())
    }

    fn put_f32s(&mut self, off: usize, values: &[f32]) -> Result<()> {
        for (i, v) in values.iter().enumerate() {
            self.put(off + i * 4, &v.to_le_bytes())?;
        }
        Ok(())
    }

    fn put_matrix(&mut self, off: usize, value: &Value, rows: usize) -> Result<()> {
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
            self.put_f32s(off + r * 16, row)?;
        }
        Ok(())
    }
}

const ARRAY_INFO_NAME_HASH: u32 = 0x100;

struct PackedPointer {
    block_id: u32,
    data_offset: u32,
}

struct ArrayHeader {
    block_id: u32,
    data_offset: u32,
    count: u16,
}

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

fn as_bits(v: &Value) -> Result<u128> {
    match v {
        Value::Bitset { bits, .. } => Ok(*bits),
        Value::UInt(n) => Ok(*n as u128),
        other => Err(MetaError::Other(format!(
            "expected a bitset, found {}",
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
