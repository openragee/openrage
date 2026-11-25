//! GTA VI's P-form laid out anew from a value tree that no longer fits the allocation it arrived
//! with: each region packed after the last one of its block in pre-order, one block per type, the
//! blocks in the iteration order of a chained hash table of their types, placed from sixteen
//! bytes into `PSIN` on 16-byte boundaries padded with 0x70, `PSIN` ending at the last block.
//!
//! ⚠ A short string kept in its 16 bytes also has its text and NUL written, unreferenced, in the
//! string block where the out-of-line form would put them.

use std::collections::HashMap;

use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::layout::PsoLayout;
use crate::meta::diagnostics::Diagnostics;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::{MetaInstance, Value};
use crate::meta::schema::SchemaLibrary;

use super::reader::{
    named_pair, GTA6_COUNT_FIRST_ARRAY, GTA6_GROUP_MAP, GTA6_PAIR_MAP, GTA6_PAIR_MAP_WORD,
};
use super::schema::{
    element_stride, string_subtype, struct_subtype, PsoDataType, PsoSchemaItem, PsoSchemaSection,
    PsoStructEntry, PsoStructInfo, ARRAY_INFO_NAME_HASH,
};
use super::sections::{PmapEntry, PmapSection, PsoSection};
use super::table_form::TableFormError;
use super::table_layout::{block_order, DEPTH};

/// The block that holds string bytes, named by the type code of a character.
const STRING_BLOCK: u32 = 0x01;
/// What a pointer reaches into its block: twelve bits in its first word, eight in its second.
const BLOCK_REACH: usize = 1 << 20;
/// The most blocks a pointer's twelve-bit id names.
const MAX_BLOCKS: usize = 0xFFF;
const PAD: u8 = 0x70;
/// The longest string kept in its 16 bytes.
const IN_PLACE: usize = 14;
/// The inline array subtype that states its count after its elements.
const INLINE_COUNTED: u8 = 1;
/// Why a document the rules do not give back is not laid out anew.
const MISSED: &str =
    "its blocks, their order or their places are not the ones the measured P-form rules write";

/// A GTA VI P-form document's bytes with `instance` laid out anew, refused when the rules do not
/// give back the document `layout` was read from.
pub(crate) fn write(layout: &PsoLayout, instance: &MetaInstance) -> Result<Vec<u8>> {
    let _names = crate::meta::binary::hash_index::title_names();
    let stored = stored_bytes(layout);
    let library = SchemaLibrary::default();
    let (original, _) = super::reader::parse_full(&library, &stored, &mut Diagnostics::lenient())?;
    if laid_out(layout, &original)? != stored {
        return Err(TableFormError::RulesMiss(MISSED.to_string()).into());
    }
    let mut out = laid_out(layout, instance)?;
    // The loader recomputes the `CHKS` and rejects a mismatch, so a changed body gets a fresh
    // checksum; a no-op save keeps the stored bytes byte for byte.
    if out != stored {
        super::chks::write_checksum(&mut out);
    }
    Ok(out)
}

/// The document `layout` was read from.
fn stored_bytes(layout: &PsoLayout) -> Vec<u8> {
    let mut out = Vec::new();
    for ident in &layout.section_order {
        match ident {
            PsoSection::Psin => out.extend_from_slice(&layout.psin),
            PsoSection::Pmap => out.extend_from_slice(&layout.pmap.write()),
            PsoSection::Psch => out.extend_from_slice(&layout.schema.write()),
            other => {
                if let Some(section) = layout.opaque_sections.iter().find(|s| s.ident == *other) {
                    out.extend_from_slice(&section.bytes);
                }
            }
        }
    }
    out
}

/// `instance` laid out anew and its values written in, `CHKS` stating the new length.
fn laid_out(layout: &PsoLayout, instance: &MetaInstance) -> Result<Vec<u8>> {
    let grown = lay_out(layout, instance)?;
    let mut out = super::writer::encode(&grown, instance, None)?;
    // `CHKS` is last: its frame, then `{u32 length, u32 checksum, tag}` big-endian. The length is
    // the new total; the checksum is carried here and recomputed by `write` when the body changed.
    let len = u32::try_from(out.len()).map_err(|_| refused("the document passes 4 GiB"))?;
    if let Some(frame) = out.len().checked_sub(20) {
        if &out[frame..frame + 4] == b"CHKS" {
            out[frame + 8..frame + 12].copy_from_slice(&len.to_be_bytes());
        }
    }
    Ok(out)
}

/// A layout of `instance` whose `PSIN` holds every region, pointer, header and string, its
/// values zero, and whose `PMAP` names its blocks; the type table and other sections carried.
fn lay_out(layout: &PsoLayout, instance: &MetaInstance) -> Result<PsoLayout> {
    let root = (layout.pmap.block(layout.pmap.root_id))
        .and_then(|block| layout.schema.struct_for(block.name_hash))
        .ok_or_else(|| refused("the root block's structure is not in the document's schema"))?;
    let index = default_hash_index();
    let by_name = (layout.schema.items.iter())
        .filter_map(|item| match item {
            PsoSchemaItem::Struct(s) => Some((index.name_or_literal(s.name_hash), s)),
            PsoSchemaItem::Enum(_) => None,
        })
        .collect();
    let mut placer = Placer {
        schema: &layout.schema,
        by_name,
        pairs: stored_pairs(layout),
        groups: group::Groups::stored(layout),
        blocks: Vec::new(),
        open: HashMap::new(),
        pointers: Vec::new(),
    };
    let (block, at) = placer.region(root.name_hash, root.data_size as usize)?;
    placer.structure(root, block, at, instance, 0)?;
    placer.finish(layout)
}

/// The structure of the pairs each pair map holds in the stored document, by `(owner, member)`:
/// the pointer names it only by the block it points into.
fn stored_pairs(layout: &PsoLayout) -> HashMap<(u32, u32), u32> {
    let mut found = HashMap::new();
    for block in &layout.pmap.entries {
        let Some(info) = layout.schema.struct_for(block.name_hash) else {
            continue;
        };
        let Ok(start) = usize::try_from(block.offset) else {
            continue;
        };
        let Ok(len) = usize::try_from(block.length) else {
            continue;
        };
        let size = info.data_size as usize;
        if size == 0 {
            continue;
        }
        // Every region of a block is of its type, back to back.
        for k in 0..len / size {
            scan_pairs(layout, info, start + k * size, 0, &mut found);
        }
    }
    found
}

fn scan_pairs(
    layout: &PsoLayout,
    info: &PsoStructInfo,
    base: usize,
    depth: usize,
    found: &mut HashMap<(u32, u32), u32>,
) {
    if depth > DEPTH {
        return;
    }
    for entry in &info.entries {
        let at = base + entry.offset() as usize;
        match entry.data_type {
            PsoDataType::Map if entry.subtype() == GTA6_PAIR_MAP => {
                let Some(word) = layout.psin.get(at + 8..at + 12) else {
                    continue;
                };
                let id = u32::from_be_bytes([word[0], word[1], word[2], word[3]]) & 0xFFF;
                if let Some(pairs) = layout.pmap.block(id as i32) {
                    found.insert((info.name_hash, entry.name_hash), pairs.name_hash);
                }
            }
            PsoDataType::Structure if entry.subtype() == struct_subtype::EMBEDDED => {
                if let Some(inner) = layout.schema.struct_for(entry.referent_hash()) {
                    scan_pairs(layout, inner, at, depth + 1, found);
                }
            }
            _ => {}
        }
    }
}

fn refused(why: impl std::fmt::Display) -> MetaError {
    MetaError::Other(format!("GTA VI PSO: {why}"))
}

fn unknown_array(subtype: u8) -> MetaError {
    refused(format!(
        "array subtype {subtype}, whose header is not known"
    ))
}

/// A pointer to `offset` of block `id`: the id and the offset's low twelve bits in the first
/// big-endian word, its next eight bits in the second word's top byte.
fn packed(id: usize, offset: usize) -> [u8; 8] {
    let (id, offset) = (id as u32, offset as u32);
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&(id | ((offset & 0xFFF) << 20)).to_be_bytes());
    out[4..].copy_from_slice(&((offset >> 12) << 24).to_be_bytes());
    out
}

fn text_of(value: Option<&Value>) -> &str {
    match value {
        Some(Value::String(s)) => s,
        Some(Value::Hash(h)) => &h.raw,
        _ => "",
    }
}

struct Placer<'a> {
    schema: &'a PsoSchemaSection,
    by_name: HashMap<String, &'a PsoStructInfo>,
    pairs: HashMap<(u32, u32), u32>,
    groups: group::Groups,
    /// `(type, bytes)` in the order the walk first reaches a region of each type.
    blocks: Vec<(u32, Vec<u8>)>,
    open: HashMap<u32, usize>,
    /// `(block, at, target block, target offset)` of every pointer, written once blocks have ids.
    pointers: Vec<(usize, usize, usize, usize)>,
}

impl<'a> Placer<'a> {
    /// `len` zero bytes after the last region of the block of `tag`.
    fn region(&mut self, tag: u32, len: usize) -> Result<(usize, usize)> {
        let block = match self.open.get(&tag) {
            Some(&block) => block,
            None => {
                if self.blocks.len() >= MAX_BLOCKS {
                    return Err(refused(format!(
                        "the document needs more than the {MAX_BLOCKS} blocks a pointer names"
                    )));
                }
                self.blocks.push((tag, Vec::new()));
                self.open.insert(tag, self.blocks.len() - 1);
                self.blocks.len() - 1
            }
        };
        let data = &mut self.blocks[block].1;
        let at = data.len();
        if at >= BLOCK_REACH {
            return Err(refused(format!(
                "the block of type {tag:#010X} passes the 1 MiB a pointer reaches, and how a P-form opens a second block of a type is not measured"
            )));
        }
        data.resize(at + len, 0);
        Ok((block, at))
    }

    fn put(&mut self, block: usize, at: usize, bytes: &[u8]) -> Result<()> {
        let slot = (self.blocks.get_mut(block))
            .and_then(|(_, data)| data.get_mut(at..at + bytes.len()))
            .ok_or_else(|| refused(format!("a write past block {block} at {at:#X}")))?;
        slot.copy_from_slice(bytes);
        Ok(())
    }

    fn struct_named(&self, hash: u32) -> Result<&'a PsoStructInfo> {
        let schema: &'a PsoSchemaSection = self.schema;
        schema
            .struct_for(hash)
            .ok_or_else(|| refused(format!("structure {hash:#010X} is not in the schema")))
    }

    fn structure(
        &mut self,
        info: &'a PsoStructInfo,
        block: usize,
        base: usize,
        inst: &MetaInstance,
        depth: usize,
    ) -> Result<()> {
        let index = default_hash_index();
        for entry in &info.entries {
            if entry.name_hash == ARRAY_INFO_NAME_HASH {
                continue;
            }
            let name = index.name_or_literal(entry.name_hash);
            let at = base + entry.offset() as usize;
            self.member(info, entry, block, at, inst.get(&name), depth)?;
        }
        Ok(())
    }

    /// The regions a member reaches, taken as the walk reaches it and walked into at once.
    fn member(
        &mut self,
        owner: &'a PsoStructInfo,
        entry: &PsoStructEntry,
        block: usize,
        at: usize,
        value: Option<&Value>,
        depth: usize,
    ) -> Result<()> {
        if depth > DEPTH {
            return Err(refused(format!(
                "the tree nests deeper than {DEPTH} levels"
            )));
        }
        match entry.data_type {
            PsoDataType::Structure => match (entry.subtype(), value) {
                (struct_subtype::EMBEDDED, Some(Value::Struct(inner))) => {
                    let info = self.struct_named(entry.referent_hash())?;
                    self.structure(info, block, at, inner, depth + 1)
                }
                (
                    struct_subtype::POINTER | struct_subtype::POINTER_ALT,
                    Some(Value::Pointer(Some(inner))),
                ) => {
                    let info = (self.by_name.get(&inner.type_name).copied()).ok_or_else(|| {
                        refused(format!(
                            "the pointer's target {} is no structure of the document",
                            inner.type_name
                        ))
                    })?;
                    let target = self.region(info.name_hash, info.data_size as usize)?;
                    self.pointers.push((block, at, target.0, target.1));
                    self.structure(info, target.0, target.1, inner, depth + 1)
                }
                _ => Ok(()),
            },
            PsoDataType::String => self.string(entry, block, at, value),
            PsoDataType::Array => self.array(owner, entry, block, at, value, depth),
            PsoDataType::Map => {
                let pairs: &[(Value, Value)] = match value {
                    Some(Value::Map(pairs)) => pairs,
                    _ => &[],
                };
                match entry.subtype() {
                    GTA6_PAIR_MAP => self.pair_map(owner, entry, block, at, pairs, depth),
                    // An emptied group map is written null, as the table form stores one.
                    GTA6_GROUP_MAP if pairs.is_empty() => Ok(()),
                    GTA6_GROUP_MAP => self.group_map(owner, entry, block, at, pairs, depth),
                    other => Err(refused(format!(
                        "map subtype {other}, whose layout is not measured"
                    ))),
                }
            }
            _ => Ok(()),
        }
    }

    fn string(
        &mut self,
        entry: &PsoStructEntry,
        block: usize,
        at: usize,
        value: Option<&Value>,
    ) -> Result<()> {
        let text = text_of(value).as_bytes();
        match entry.subtype() {
            string_subtype::CHAR_ARRAY if text.len() <= IN_PLACE => {
                let mut field = [0u8; 16];
                field[..text.len()].copy_from_slice(text);
                field[15] = 0x80 | text.len() as u8;
                self.put(block, at, &field)?;
                self.text_bytes(text)?;
                Ok(())
            }
            string_subtype::CHAR_ARRAY => {
                let count = u16::try_from(text.len() + 1)
                    .map_err(|_| refused("a string is longer than its count states"))?;
                let target = self.text_bytes(text)?;
                self.pointers.push((block, at, target.0, target.1));
                let tail = [(count - 1).to_be_bytes(), count.to_be_bytes()].concat();
                self.put(block, at + 8, &tail)
            }
            string_subtype::POINTER | string_subtype::POINTER_ALT if !text.is_empty() => {
                let target = self.text_bytes(text)?;
                self.pointers.push((block, at, target.0, target.1));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// `text` and its NUL in the string block.
    fn text_bytes(&mut self, text: &[u8]) -> Result<(usize, usize)> {
        let (block, at) = self.region(STRING_BLOCK, text.len() + 1)?;
        self.put(block, at, text)?;
        Ok((block, at))
    }

    fn array(
        &mut self,
        owner: &'a PsoStructInfo,
        entry: &PsoStructEntry,
        block: usize,
        at: usize,
        value: Option<&Value>,
        depth: usize,
    ) -> Result<()> {
        let Some(elem) = entry.member_index().and_then(|i| owner.entry(i)).copied() else {
            return Ok(());
        };
        let Some(Value::Array(items)) = value else {
            return Ok(());
        };
        let stride = element_stride(self.schema, &elem).ok_or_else(|| {
            refused(format!(
                "an array of {:?} subtype {} has no known element size",
                elem.data_type,
                elem.subtype()
            ))
        })?;
        let inline = usize::from(entry.count());
        if inline != 0 {
            for (i, item) in items.iter().take(inline).enumerate() {
                self.member(owner, &elem, block, at + stride * i, Some(item), depth + 1)?;
            }
            if entry.subtype() == INLINE_COUNTED {
                let count = items.len().min(inline) as u32;
                self.put(block, at + stride * inline, &count.to_be_bytes())?;
            }
            return Ok(());
        }
        let (count_at, pointer_at) = match entry.subtype() {
            0 => (at + 8, at),
            GTA6_COUNT_FIRST_ARRAY => (at, at + 8),
            other => return Err(unknown_array(other)),
        };
        if items.is_empty() {
            return Ok(());
        }
        let len = (stride.checked_mul(items.len()))
            .ok_or_else(|| refused("an array is larger than a block holds"))?;
        let tag = match elem.data_type {
            PsoDataType::Structure if elem.subtype() == struct_subtype::EMBEDDED => {
                elem.referent_hash()
            }
            other => u32::from(other.to_byte()),
        };
        let target = self.region(tag, len)?;
        self.pointers.push((block, pointer_at, target.0, target.1));
        let too_many = || refused("an array holds more elements than its count states");
        let counts = if entry.subtype() == 0 {
            let n = u16::try_from(items.len()).map_err(|_| too_many())?;
            [n.to_be_bytes(), n.to_be_bytes()].concat()
        } else {
            let n = u32::try_from(items.len()).map_err(|_| too_many())?;
            [n.to_be_bytes(), n.to_be_bytes()].concat()
        };
        self.put(block, count_at, &counts)?;
        for (i, item) in items.iter().enumerate() {
            let element = target.1 + stride * i;
            self.member(owner, &elem, target.0, element, Some(item), depth + 1)?;
        }
        Ok(())
    }

    /// `{u32 0x01000000, u32 0, pointer, u16 count, u16 count, u32 0}` over pairs of the structure
    /// the stored document's map of this member holds.
    fn pair_map(
        &mut self,
        owner: &'a PsoStructInfo,
        entry: &PsoStructEntry,
        block: usize,
        at: usize,
        pairs: &[(Value, Value)],
        depth: usize,
    ) -> Result<()> {
        self.put(block, at, &GTA6_PAIR_MAP_WORD.to_be_bytes())?;
        if pairs.is_empty() {
            return Ok(());
        }
        let class = (self.pairs.get(&(owner.name_hash, entry.name_hash)).copied()).ok_or_else(
            || {
                refused(
                    "a map's pairs are of a structure only the block that holds them names, and no stored map of this member holds any",
                )
            },
        )?;
        let info = self.struct_named(class)?;
        let [key, value] = named_pair(info)?;
        let size = info.data_size as usize;
        let n = u16::try_from(pairs.len())
            .map_err(|_| refused("a map holds more pairs than its count states"))?;
        let len = (size.checked_mul(pairs.len()))
            .ok_or_else(|| refused("a map is larger than a block holds"))?;
        let target = self.region(class, len)?;
        self.pointers.push((block, at + 8, target.0, target.1));
        self.put(block, at + 16, &[n.to_be_bytes(), n.to_be_bytes()].concat())?;
        for (i, (k, v)) in pairs.iter().enumerate() {
            let base = target.1 + size * i;
            let (key_at, value_at) = (base + key.offset() as usize, base + value.offset() as usize);
            self.member(info, &key, target.0, key_at, Some(k), depth + 1)?;
            self.member(info, &value, target.0, value_at, Some(v), depth + 1)?;
        }
        Ok(())
    }

    /// The blocks in table order from sixteen bytes into `PSIN`, every pointer written by the ids
    /// that order gives, and a `PMAP` that names them.
    fn finish(self, layout: &PsoLayout) -> Result<PsoLayout> {
        let Placer {
            mut blocks,
            pointers,
            groups,
            ..
        } = self;
        let order = block_order(&blocks)?;
        let mut id = vec![0usize; blocks.len()];
        for (k, &b) in order.iter().enumerate() {
            id[b] = k + 1;
        }
        for (b, at, target, offset) in pointers {
            let slot = (blocks.get_mut(b))
                .and_then(|(_, data)| data.get_mut(at..at + 8))
                .ok_or_else(|| refused(format!("a pointer past block {b} at {at:#X}")))?;
            slot.copy_from_slice(&packed(id[target], offset));
        }
        groups.overwrite(&mut blocks)?;
        let too_large = || refused("the document is larger than a block offset states");
        let mut psin = b"PSIN\0\0\0\0".to_vec();
        psin.extend_from_slice(&[PAD; 8]);
        let mut entries = Vec::with_capacity(order.len());
        for &b in &order {
            while !psin.len().is_multiple_of(16) {
                psin.push(PAD);
            }
            let (name_hash, data) = &blocks[b];
            entries.push(PmapEntry {
                name_hash: *name_hash,
                offset: i32::try_from(psin.len()).map_err(|_| too_large())?,
                unknown: 0,
                length: i32::try_from(data.len()).map_err(|_| too_large())?,
            });
            psin.extend_from_slice(data);
        }
        let len = u32::try_from(psin.len()).map_err(|_| too_large())?;
        psin[4..8].copy_from_slice(&len.to_be_bytes());
        Ok(PsoLayout {
            schema: layout.schema.clone(),
            pmap: PmapSection {
                root_id: id[0] as i32,
                header_kind: layout.pmap.header_kind,
                entries,
            },
            psin,
            opaque_sections: layout.opaque_sections.clone(),
            section_order: layout.section_order.clone(),
        })
    }
}

#[path = "pform_layout_group.rs"]
mod group;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::binary::{
        parse_meta_binary_full, write_meta_binary, BinaryFormat, MetaLayout,
    };

    const ROOT: u32 = 0xC1C1_0001;
    const LIST: u32 = 0xC1C1_0010;
    const TEXT: u32 = 0xC1C1_0011;
    const COUNT: u32 = 0xC1C1_0012;
    /// The root's bytes: an array header at 0, a string's 16 bytes at 0x10 and a u32 at 0x20.
    const SIZE: u32 = 0x28;
    const UINT: u8 = 0x06;

    fn section(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
        out.extend_from_slice(ident);
        out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(content);
    }

    fn entry(out: &mut Vec<u8>, hash: u32, code: u8, subtype: u8, offset: u16, reference: u32) {
        out.extend_from_slice(&hash.to_be_bytes());
        out.extend_from_slice(&[code, subtype]);
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&reference.to_be_bytes());
    }

    /// A P-form document placed as the measured rules place one: a root whose array of u32
    /// holds `list` and whose `text` is kept in place, its copy in the string block.
    fn document(list: &[u32], text: &str) -> Vec<u8> {
        let mut root = vec![0u8; SIZE as usize];
        let n = list.len() as u16;
        root[8..12].copy_from_slice(&[n.to_be_bytes(), n.to_be_bytes()].concat());
        root[0x10..0x10 + text.len()].copy_from_slice(text.as_bytes());
        root[0x1F] = 0x80 | text.len() as u8;
        root[0x20..0x24].copy_from_slice(&5u32.to_be_bytes());
        let values: Vec<u8> = list.iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut copy = text.as_bytes().to_vec();
        copy.push(0);
        // In the order the walk first reaches them: the root, the array's values, the string.
        let mut blocks = vec![(ROOT, root)];
        if !list.is_empty() {
            blocks.push((u32::from(UINT), values));
        }
        blocks.push((STRING_BLOCK, copy));
        let order = block_order(&blocks).expect("an order");
        let id = |b: usize| order.iter().position(|&x| x == b).expect("placed") + 1;
        if !list.is_empty() {
            blocks[0].1[..4].copy_from_slice(&(id(1) as u32).to_be_bytes());
        }

        let mut psin = b"PSIN\0\0\0\0".to_vec();
        psin.extend_from_slice(&[PAD; 8]);
        let mut pmap = (id(0) as i32).to_be_bytes().to_vec();
        pmap.extend_from_slice(&(blocks.len() as i16).to_be_bytes());
        pmap.extend_from_slice(&0i16.to_be_bytes());
        for &b in &order {
            while !psin.len().is_multiple_of(16) {
                psin.push(PAD);
            }
            let (hash, data) = &blocks[b];
            for word in [*hash, psin.len() as u32, 0, data.len() as u32] {
                pmap.extend_from_slice(&word.to_be_bytes());
            }
            psin.extend_from_slice(data);
        }
        let len = psin.len() as u32;
        psin[4..8].copy_from_slice(&len.to_be_bytes());

        let mut items = vec![0, 0, 0, 4];
        items.extend_from_slice(&SIZE.to_be_bytes());
        items.extend_from_slice(&[0; 4]);
        entry(&mut items, LIST, 0x0D, 0, 0, 3);
        entry(&mut items, TEXT, 0x0B, string_subtype::CHAR_ARRAY, 0x10, 0);
        entry(&mut items, COUNT, UINT, 0, 0x20, 0);
        entry(&mut items, ARRAY_INFO_NAME_HASH, UINT, 0, 0, 0);
        let mut psch = 1u32.to_be_bytes().to_vec();
        psch.extend_from_slice(&ROOT.to_be_bytes());
        psch.extend_from_slice(&(8u32 + 4 + 8).to_be_bytes());
        psch.extend_from_slice(&items);

        let mut out = psin;
        section(&mut out, b"PMAP", &pmap);
        section(&mut out, b"PSCH", &psch);
        let mut chks = (out.len() as u32 + 20).to_be_bytes().to_vec();
        chks.extend_from_slice(&[0; 4]);
        chks.extend_from_slice(b"zppp");
        section(&mut out, b"CHKS", &chks);
        // The fixture carries the checksum the loader's rule gives, as a real body does.
        super::super::chks::write_checksum(&mut out);
        out
    }

    fn open(bytes: &[u8]) -> (MetaInstance, MetaLayout) {
        parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut Diagnostics::strict())
            .expect("it reads")
    }

    fn write(instance: &MetaInstance, layout: &MetaLayout) -> Vec<u8> {
        write_meta_binary(BinaryFormat::Pso, Some(layout), instance).expect("it saves")
    }

    fn named(hash: u32) -> String {
        default_hash_index().name_or_literal(hash)
    }

    #[test]
    fn a_gta6_p_form_tree_that_outgrew_its_allocation_is_laid_out_as_the_rules_lay_one_out() {
        let bytes = document(&[7, 9], "hi");
        let (tree, layout) = open(&bytes);
        assert_eq!(
            write(&tree, &layout),
            bytes,
            "an unchanged tree saves in place"
        );
        // Grown, emptied, which drops the block of its values, and a string kept in place retold.
        for (list, text) in [
            (&[7u32, 9, 11][..], "hi"),
            (&[], "hi"),
            (&[7, 9], "a new text"),
        ] {
            let mut edited = tree.clone();
            let values = list.iter().map(|&v| Value::UInt(u64::from(v))).collect();
            *edited.get_mut(&named(LIST)).expect("the list") = Value::Array(values);
            *edited.get_mut(&named(TEXT)).expect("the text") = Value::String(text.into());
            let written = write(&edited, &layout);
            assert_eq!(written, document(list, text), "{list:?} {text}");
            let (again, layout) = open(&written);
            assert_eq!(again, edited, "{list:?} {text}");
            assert_eq!(write(&again, &layout), written, "{list:?} {text}");
        }
    }

    #[test]
    fn a_gta6_p_form_converges_through_xml_to_the_layout_the_rules_give() {
        for (list, text) in [(&[7u32, 9][..], "hi"), (&[], "a new text")] {
            let bytes = document(list, text);
            let converged = crate::meta::canonical::converge(&SchemaLibrary::default(), &bytes)
                .unwrap_or_else(|e| panic!("{list:?} {text}: {e}"));
            assert_eq!(converged.bytes, bytes, "{list:?} {text}");
        }
    }

    #[test]
    fn a_pointer_packs_its_offset_in_both_words_and_its_block_in_the_first() {
        let offset = 0xA_BCDE;
        let packed = packed(0x123, offset);
        let low = u32::from_be_bytes([packed[0], packed[1], packed[2], packed[3]]);
        let high = u32::from_be_bytes([packed[4], packed[5], packed[6], packed[7]]);
        let pointer = super::super::sections::PsoPointer {
            raw: u64::from(low) | (u64::from(high) << 32),
        };
        assert_eq!(pointer.block_id(), 0x123);
        assert_eq!(pointer.gta6_data_offset(), Some(offset as u32));
    }
}
