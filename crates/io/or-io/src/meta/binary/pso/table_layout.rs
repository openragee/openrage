//! GTA VI's table form laid out from a value tree: each region packed after the last one of its
//! block in pre-order, the blocks in the iteration order of a chained hash table of their types,
//! headers and hash tables by the measured rules, and every byte no value states carried from
//! the document the tree was read from.

use std::collections::{HashMap, HashSet};

use crate::meta::binary::enum_text;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::layout::PsoLayout;
use crate::meta::binary::rbm::schema::{PsoType, RbmClass, RbmClassEntry, RbmModel};
use crate::meta::error::Result;
use crate::meta::instance::value::{MetaInstance, Value};

use super::table_values::*;

use super::sections::{PsoSection, RawPsoSection};
use super::table_form::{
    bitset_len, class_of, malformed, ARRAY_INFO_NAME_HASH, BYTES_16, COUNTED_ARRAY,
    COUNTED_POINTER, EMPTY_CONTROL, GROUP_MAP, GROUP_SET, INLINE_BITSET, PADDED_U32, SLOT_MAP,
    SLOT_SET, TAG, TAGGED_ARRAY, TAGGED_POINTER, WIDE_HASH, WIDE_NS_HASH,
};

/// The bucket counts a block table's hash table grows through.
const BUCKETS: [usize; 11] = [11, 29, 53, 103, 211, 409, 647, 1031, 1291, 2053, 4099];
/// The most blocks a packed pointer's 12-bit id names.
const MAX_BLOCKS: usize = 0xFFF;
/// How deep a written tree may nest.
pub(super) const DEPTH: usize = 64;
/// The most bytes a block holds, unless one region is larger: that one takes an empty block.
const BLOCK_CAP: usize = 1 << 20;

/// The namespaced string hashes a document holds, by whether they are 64-bit: their text is in
/// `LSNS`, which is encrypted, so a save may write only these.
pub(crate) type Held = HashSet<(bool, u64)>;

/// The key a [`Held`] holds for a namespace whose 64-bit hashes a save may add.
// A body whose string table has no entry of a namespace holds none of its text. No 32-bit hash
// sets bit 63, so this key names no hash.
pub(crate) fn free_namespace(namespace: u8) -> (bool, u64) {
    (false, (1 << 63) | u64::from(namespace))
}

/// The blocks a value tree lays out as: `(type, bytes)` in block-table order, the root block's id.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Laid {
    pub(crate) blocks: Vec<(u32, Vec<u8>)>,
    pub(crate) root: u32,
    /// Per block, whether a headerless resource keeps it in its physical section.
    pub(crate) physical: Vec<bool>,
}

/// A table-form document's bytes with `instance` laid out in it, refused when the rules do not
/// give back the document `layout` was read from.
pub(crate) fn write(layout: &PsoLayout, instance: &MetaInstance) -> Result<Vec<u8>> {
    let sections = sections_of(layout);
    let model = super::table_form::model(&sections)?;
    let stored: Vec<u8> = sections.iter().flat_map(|s| s.bytes.clone()).collect();
    let original = super::table_form::decode(&model)?;
    let (laid, held) = lay_out(&model, &original, None)?;
    let mut bytes = if emit(&sections, &laid) != stored {
        super::table_canonical::write(
            &sections, &model, &stored, &original, &laid, &held, instance,
        )?
    } else {
        let (laid, _) = lay_out(&model, instance, Some(&held))?;
        emit(&sections, &laid)
    };
    // The loader recomputes the `CHKS` and rejects a mismatch, so a changed body gets a fresh
    // checksum; a no-op save keeps the stored bytes, stale value and all, byte for byte.
    if bytes != stored {
        super::chks::write_checksum(&mut bytes);
    }
    Ok(bytes)
}

fn sections_of(layout: &PsoLayout) -> Vec<RawPsoSection> {
    (layout.section_order.iter())
        .map(|&ident| RawPsoSection {
            ident,
            bytes: match ident {
                PsoSection::Psin => layout.psin.clone(),
                other => (layout.opaque_sections.iter())
                    .find(|s| s.ident == other)
                    .map(|s| s.bytes.clone())
                    .unwrap_or_default(),
            },
        })
        .collect()
}

/// Which of the stored blocks the rules do not give back.
pub(crate) fn where_they_miss(model: &RbmModel, laid: &Laid) -> String {
    let stored: Vec<u32> = model.data_blocks.iter().map(|b| b.data_type).collect();
    let types: Vec<u32> = laid.blocks.iter().map(|(t, _)| *t).collect();
    if stored != types || model.root_block_id != laid.root {
        return "its block table is not in the order the hash table of its types gives".into();
    }
    for (i, (stored, (ty, bytes))) in model.data_blocks.iter().zip(&laid.blocks).enumerate() {
        if let Some(at) =
            (0..stored.data.len().max(bytes.len())).find(|&at| stored.data.get(at) != bytes.get(at))
        {
            return format!(
                "block {} ({ty:#010X}) differs from byte {at:#X}, where a hash table holds its pairs in an order of insertion the file does not keep or a header is outside the measured rules",
                i + 1
            );
        }
    }
    "its sections are not the ones the rules write".into()
}

/// The table form's sections with `laid` as their blocks.
pub(crate) fn emit(sections: &[RawPsoSection], laid: &Laid) -> Vec<u8> {
    let mut out = Vec::new();
    for section in sections {
        let start = out.len();
        match &section.ident.ident().to_be_bytes() {
            b"PSIN" => {
                out.extend_from_slice(b"PSIN\0\0\0\0");
                for (ty, data) in &laid.blocks {
                    out.extend_from_slice(&ty.to_le_bytes());
                    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                    out.extend_from_slice(&[0xAA; 8]);
                }
                pad(&mut out, start, 0x70);
                for (_, data) in &laid.blocks {
                    pad(&mut out, start, 0);
                    out.extend_from_slice(data);
                }
                pad(&mut out, start, 0);
                let len = (out.len() - start) as u32;
                out[start + 4..start + 8].copy_from_slice(&len.to_be_bytes());
            }
            b"LRSC" => {
                out.extend_from_slice(&section.bytes);
                put(&mut out, start + 8 + 0x10, &laid.root.to_le_bytes());
                put(
                    &mut out,
                    start + 8 + 0x3C,
                    &(laid.blocks.len() as u16).to_le_bytes(),
                );
            }
            _ => out.extend_from_slice(&section.bytes),
        }
    }
    // `CHKS` is last: its frame, then `{u32 length, u32 checksum, tag}` big-endian. The length is
    // the new total; the checksum is carried here and recomputed by the save path when the body
    // changed (`chks::write_checksum`), so emitting keeps a no-op byte-identical.
    let len = out.len() as u32;
    if let Some(frame) = out.len().checked_sub(20) {
        if &out[frame..frame + 4] == b"CHKS" {
            out[frame + 8..frame + 12].copy_from_slice(&len.to_be_bytes());
        }
    }
    out
}

fn pad(out: &mut Vec<u8>, start: usize, fill: u8) {
    while !(out.len() - start).is_multiple_of(16) {
        out.push(fill);
    }
}

fn put(out: &mut [u8], at: usize, bytes: &[u8]) {
    if let Some(dst) = out.get_mut(at..at + bytes.len()) {
        dst.copy_from_slice(bytes);
    }
}

/// The blocks `instance` lays out as over the classes of `model`, whose blocks carry what no
/// value states, and the namespaced hashes it holds; with `held`, one it does not hold is refused.
pub(crate) fn lay_out(
    model: &RbmModel,
    instance: &MetaInstance,
    held: Option<&Held>,
) -> Result<(Laid, Held)> {
    walk(model, instance, held, false, false)
}

/// [`lay_out`] into the blocks `model` stores, each region where the document had it, refused
/// where a region would change its size.
pub(crate) fn lay_out_in_place(
    model: &RbmModel,
    instance: &MetaInstance,
    held: Option<&Held>,
) -> Result<(Laid, Held)> {
    walk(model, instance, held, true, false)
}

/// [`lay_out`] for a headerless resource: its blocks in their stored order and a new one after
/// them, and the regions of a member whose byte 9 sets bit 2 in blocks of their own.
pub(crate) fn lay_out_headerless(
    model: &RbmModel,
    instance: &MetaInstance,
    held: Option<&Held>,
) -> Result<(Laid, Held)> {
    walk(model, instance, held, false, true)
}

/// A headerless member's byte 9 bit that keeps its regions out of every other member's blocks,
/// all in one block or each in its own as the resource stored them.
const OWN_BLOCKS: u8 = 0x04;
/// The byte 9 bit that puts those blocks in the physical section.
const PHYSICAL_BLOCKS: u8 = 0x20;

/// The member whose regions take blocks of their own: its structure, its name, and whether the
/// blocks are physical.
type Own = (u32, u32, bool);

fn walk(
    model: &RbmModel,
    instance: &MetaInstance,
    held: Option<&Held>,
    in_place: bool,
    headerless: bool,
) -> Result<(Laid, Held)> {
    let index = default_hash_index();
    let by_name = (model.classes.values())
        .map(|c| (index.name_or_literal(c.name_hash), c))
        .collect();
    let root = class_of(model, root_type(model, instance, &by_name)?)?;
    let background = model
        .block(model.root_block_id as i32)
        .filter(|b| b.data_type == root.name_hash)
        .map(|b| b.data.as_slice());
    let blocks = match in_place {
        true => (model.data_blocks.iter())
            .map(|b| (b.data_type, b.data.clone()))
            .collect(),
        false => Vec::new(),
    };
    let stored = (model.root_block_id as usize)
        .checked_sub(1)
        .and_then(|block| {
            Some(Stored {
                ty: root.name_hash,
                bytes: background?,
                block,
                at: 0,
            })
        });
    let opened = blocks.len();
    let mut w = Writer {
        model,
        by_name,
        blocks,
        in_place,
        headerless,
        own: None,
        sources: vec![None; opened],
        owners: vec![None; opened],
        index: HashMap::new(),
        trace: None,
        fixups: Vec::new(),
        held,
        laid: Held::new(),
        path: Vec::new(),
        depth: 0,
        stored_count: None,
    };
    let size = root.data_size as usize;
    let (b, at) = w.region(root.name_hash, size, stored, size, background)?;
    w.structure(root, b, at, instance, background)?;
    w.finish()
}

fn root_type(
    model: &RbmModel,
    instance: &MetaInstance,
    by_name: &HashMap<String, &RbmClass>,
) -> Result<u32> {
    if let Some(class) = by_name.get(&instance.type_name) {
        return Ok(class.name_hash);
    }
    // XML read with no schema names its root by its element, which is the class's name.
    (model.block(model.root_block_id as i32).map(|b| b.data_type)).ok_or_else(|| {
        malformed(format!(
            "the root {} is no structure of the document",
            instance.type_name
        ))
    })
}

struct Writer<'m, 'h> {
    model: &'m RbmModel,
    by_name: HashMap<String, &'m RbmClass>,
    /// `(type, bytes)` in the order the walk first reaches a region of each type, or the stored
    /// blocks in their order when [`Self::in_place`].
    blocks: Vec<(u32, Vec<u8>)>,
    /// Whether each region goes where the document had it.
    in_place: bool,
    /// Whether the document is a headerless resource, as [`lay_out_headerless`] lays one out.
    headerless: bool,
    /// The member whose own blocks the regions being laid out go to.
    own: Option<Own>,
    /// Per block, the first stored block any of its regions was read from.
    sources: Vec<Option<usize>>,
    /// Per block, the member it is kept for.
    owners: Vec<Option<Own>>,
    /// The blocks of each type no member keeps for itself, in the order they opened.
    index: HashMap<u32, Vec<usize>>,
    /// While a hash table's payloads are tried, the stored `(block, offset)` of each region they
    /// reach.
    trace: Option<Vec<(usize, usize)>>,
    /// `(block, offset)` of a packed pointer and the `(block, offset)` it names.
    fixups: Vec<(usize, usize, usize, usize)>,
    held: Option<&'h Held>,
    laid: Held,
    path: Vec<String>,
    depth: usize,
    /// The stored count of the [`COUNTED_POINTER`] being written, which bounds what it carries.
    stored_count: Option<usize>,
}

/// Bytes the document was read with at the place being written, which carry what no value states.
type Background<'m> = Option<&'m [u8]>;

/// What a stored pointer named: its block's type and index, the offset, and the bytes from there.
#[derive(Clone, Copy)]
struct Stored<'m> {
    ty: u32,
    bytes: &'m [u8],
    block: usize,
    at: usize,
}

impl<'m> Writer<'m, '_> {
    fn fail(&self, why: impl std::fmt::Display) -> crate::meta::error::MetaError {
        malformed(format!("{}: {why}", self.path.concat()))
    }

    fn finish(self) -> Result<(Laid, Held)> {
        if self.blocks.len() > MAX_BLOCKS {
            return Err(malformed(format!(
                "the tree lays out as {} blocks, more than the {MAX_BLOCKS} a pointer names",
                self.blocks.len()
            )));
        }
        let order = match (self.in_place, self.headerless) {
            (true, _) => (0..self.blocks.len()).collect(),
            // A block keeps the place of the first stored block it holds a region of; a new one
            // follows them in the order it opened.
            (false, true) => {
                let mut order: Vec<usize> = (0..self.blocks.len()).collect();
                order.sort_by_key(|&b| (self.sources[b].is_none(), self.sources[b], b));
                order
            }
            (false, false) => block_order(&self.blocks)?,
        };
        let physical = (order.iter())
            .map(|&b| self.owners[b].is_some_and(|(_, _, physical)| physical))
            .collect();
        let mut id = vec![0usize; order.len()];
        for (k, &b) in order.iter().enumerate() {
            id[b] = k + 1;
        }
        let mut blocks = self.blocks;
        for (b, at, target, offset) in self.fixups {
            // A tagged pointer's bit 63 is set before its fixup.
            let tag = u64::from(blocks[b].1[at + 7] & 0x80) << 56;
            let raw = (offset as u64) << 20 | id[target] as u64 | tag;
            blocks[b].1[at..at + 8].copy_from_slice(&raw.to_le_bytes());
        }
        // The root is the first region laid out, which in place is where the document had it.
        let root = match self.in_place {
            true => self.model.root_block_id,
            false => id[0] as u32,
        };
        let mut slots: Vec<Option<(u32, Vec<u8>)>> = blocks.into_iter().map(Some).collect();
        let blocks = order.iter().filter_map(|&b| slots[b].take()).collect();
        Ok((
            Laid {
                blocks,
                root,
                physical,
            },
            self.laid,
        ))
    }

    /// `len` bytes after the last region of the first block of type `ty` with room for them, else
    /// of a new one, from `background` where it holds them; read from stored block `source`.
    fn alloc(
        &mut self,
        ty: u32,
        len: usize,
        source: Option<usize>,
        background: Background,
    ) -> Result<(usize, usize)> {
        let b = match self.own.filter(|_| self.headerless) {
            Some(own) => self.own_block(ty, own, source),
            None => {
                let blocks = &self.blocks;
                let room = (self.index.get(&ty).into_iter().flatten().copied()).find(|&b| {
                    let used = blocks[b].1.len();
                    len == 0 || used == 0 || used.saturating_add(len) <= BLOCK_CAP
                });
                match room {
                    Some(b) => b,
                    None => {
                        let b = self.open(ty, None);
                        self.index.entry(ty).or_default().push(b);
                        b
                    }
                }
            }
        };
        if let Some(s) = source {
            let first = &mut self.sources[b];
            *first = Some(first.map_or(s, |f| f.min(s)));
        }
        let data = &mut self.blocks[b].1;
        let at = data.len();
        let end = at
            .checked_add(len)
            .filter(|&end| end <= u32::MAX as usize)
            .ok_or_else(|| {
                malformed(format!("block {ty:#010X} outgrows the 4 GiB a size states"))
            })?;
        match background.and_then(|bg| bg.get(..len)) {
            Some(bg) => data.extend_from_slice(bg),
            None => data.resize(end, 0),
        }
        Ok((b, at))
    }

    /// A new empty block of type `ty`, kept for `owner` where one is given.
    fn open(&mut self, ty: u32, owner: Option<Own>) -> usize {
        self.blocks.push((ty, Vec::new()));
        self.sources.push(None);
        self.owners.push(owner);
        self.blocks.len() - 1
    }

    /// The block of type `ty` kept for `own` that holds the regions of stored block `source`, else
    /// a new one: a member stores its regions in one block or each in its own, and a block says which.
    fn own_block(&mut self, ty: u32, own: Own, source: Option<usize>) -> usize {
        let found = source.and_then(|s| {
            (0..self.blocks.len()).find(|&b| {
                self.owners[b] == Some(own) && self.blocks[b].0 == ty && self.sources[b] == Some(s)
            })
        });
        found.unwrap_or_else(|| self.open(ty, Some(own)))
    }

    /// Where a region of `len` bytes of type `ty` goes: after the last of its block, or where the
    /// stored pointer named it when [`Self::in_place`], which must have held `stored_len`.
    fn region(
        &mut self,
        ty: u32,
        len: usize,
        stored: Option<Stored>,
        stored_len: usize,
        background: Background,
    ) -> Result<(usize, usize)> {
        if let (Some(trace), Some(s), true) = (self.trace.as_mut(), stored, stored_len > 0) {
            trace.push((s.block, s.at));
        }
        if !self.in_place {
            let source = stored.map(|s| s.block);
            // A region of no bytes the document pointed at keeps its offset.
            return match (len, stored) {
                (0, Some(s)) => Ok((self.alloc(ty, 0, source, background)?.0, s.at)),
                _ => self.alloc(ty, len, source, background),
            };
        }
        match stored {
            Some(s) if stored_len == len => Ok((s.block, s.at)),
            _ => Err(self.fail(format!(
                "it lays out {len} bytes where the resource holds {}, and a headerless resource is saved only into the allocation it arrived with, because where its blocks sit in its sections is not known",
                stored.map_or(0, |_| stored_len)
            ))),
        }
    }

    fn put(&mut self, b: usize, at: usize, bytes: &[u8]) {
        put(&mut self.blocks[b].1, at, bytes);
    }

    fn pointer(&mut self, b: usize, at: usize, target: Option<(usize, usize)>) {
        match target {
            Some((tb, toff)) => {
                self.put(b, at, &[0; 8]);
                self.fixups.push((b, at, tb, toff));
            }
            None => self.put(b, at, &0u64.to_le_bytes()),
        }
    }

    /// Sets [`TAG`] on the pointer at `at`, written before.
    fn tag(&mut self, b: usize, at: usize) {
        if let Some(top) = self.blocks[b].1.get_mut(at + 7) {
            *top |= 0x80;
        }
    }

    /// What the pointer at `at` of `background`, one the document was read with, named; a tagged
    /// one's [`TAG`] is cleared, as no other pointer read sets it.
    fn stored_target(&self, background: Background, at: usize) -> Option<Stored<'m>> {
        let raw = u64::from_le_bytes(background?.get(at..at + 8)?.try_into().ok()?) & !TAG;
        if raw & 0xFFF == 0 || (raw >> 12) & 0xFF != 0 {
            return None;
        }
        let id = (raw & 0xFFF) as usize;
        let block = self.model.block(id as i32)?;
        let at = usize::try_from(raw >> 20).ok()?;
        Some(Stored {
            ty: block.data_type,
            bytes: block.data.get(at..)?,
            block: id - 1,
            at,
        })
    }

    fn structure(
        &mut self,
        class: &'m RbmClass,
        b: usize,
        at: usize,
        inst: &MetaInstance,
        background: Background<'m>,
    ) -> Result<()> {
        self.depth += 1;
        if self.depth > DEPTH {
            return Err(self.fail(format!("the tree nests past {DEPTH} levels")));
        }
        let index = default_hash_index();
        for entry in class.entries.iter() {
            if entry.name_hash == ARRAY_INFO_NAME_HASH {
                continue;
            }
            let name = index.name_or_literal(entry.name_hash);
            let Some(value) = inst.get(&name) else {
                return Err(self.fail(format!("{} has no member {name}", class_name(class))));
            };
            self.path.push(format!(".{name}"));
            let off = entry.offset as usize;
            let bg = background.and_then(|bg| bg.get(off..));
            if entry.data_type == PsoType::Unknown(COUNTED_POINTER) {
                let counter = class.entries.get(entry.union_ch as usize);
                let width = counter.and_then(|c| self.stride_of(c.data_type));
                self.stored_count = counter
                    .zip(width)
                    .and_then(|(c, w)| read_le(background, c.offset as usize, w));
            }
            self.member(class, entry, b, at + off, value, bg)?;
            self.path.pop();
        }
        for entry in class.entries.iter() {
            if entry.data_type == PsoType::Unknown(COUNTED_POINTER) {
                let name = index.name_or_literal(entry.name_hash);
                let count = inst
                    .get(&name)
                    .map_or(0, |v| items(v).map_or(0, <[Value]>::len));
                self.count_member(class, entry, b, at, count)?;
            }
        }
        self.depth -= 1;
        Ok(())
    }

    /// The count a [`COUNTED_POINTER`]'s elements write into the member that counts them.
    fn count_member(
        &mut self,
        class: &RbmClass,
        entry: &RbmClassEntry,
        b: usize,
        base: usize,
        count: usize,
    ) -> Result<()> {
        let counter = class.entries.get(entry.union_ch as usize).copied();
        let at = base + counter.map_or(0, |c| c.offset as usize);
        let fits = |max: u64| (count as u64 <= max).then_some(count as u64);
        let written = match counter.map(|c| c.data_type) {
            Some(PsoType::U8) => fits(0xFF).map(|n| self.put(b, at, &[n as u8])),
            Some(PsoType::U16) => fits(0xFFFF).map(|n| self.put(b, at, &(n as u16).to_le_bytes())),
            Some(PsoType::U32) => {
                fits(0xFFFF_FFFF).map(|n| self.put(b, at, &(n as u32).to_le_bytes()))
            }
            _ => return Err(self.fail("its count is in no u8, u16 or u32 member")),
        };
        written.ok_or_else(|| self.fail(format!("{count} elements do not fit its count member")))
    }

    /// [`Self::member_value`] with the regions it reaches kept for it where its byte 9 says so.
    fn member(
        &mut self,
        owner: &'m RbmClass,
        entry: &'m RbmClassEntry,
        b: usize,
        at: usize,
        value: &Value,
        bg: Background<'m>,
    ) -> Result<()> {
        let outer = self.own;
        self.own = (self.headerless && entry.union_9h & OWN_BLOCKS != 0).then(|| {
            let physical = entry.union_9h & PHYSICAL_BLOCKS != 0;
            (owner.name_hash, entry.name_hash, physical)
        });
        let written = self.member_value(owner, entry, b, at, value, bg);
        self.own = outer;
        written
    }

    #[allow(clippy::too_many_lines)]
    fn member_value(
        &mut self,
        owner: &'m RbmClass,
        entry: &'m RbmClassEntry,
        b: usize,
        at: usize,
        value: &Value,
        bg: Background<'m>,
    ) -> Result<()> {
        use PsoType::*;
        let code = entry.data_type.to_byte();
        let wrong = |w: &Self| {
            w.fail(format!(
                "holds {}, which a member of type {code:#04X} cannot hold",
                kind(value)
            ))
        };
        match entry.data_type {
            Bool => {
                let v = boolean(value).ok_or_else(|| wrong(self))?;
                self.put(b, at, &[u8::from(v)]);
            }
            S8 | U8 | S16 | U16 | S32 | U32 | S64 | U64 | Unknown(WIDE_HASH) => {
                let (width, signed) = match entry.data_type {
                    S8 => (1, true),
                    U8 => (1, false),
                    S16 => (2, true),
                    U16 => (2, false),
                    S32 => (4, true),
                    U32 => (4, false),
                    S64 => (8, true),
                    _ => (8, false),
                };
                let v = int(value).ok_or_else(|| wrong(self))?;
                let bytes = in_width(v, width, signed)
                    .ok_or_else(|| self.fail(format!("{v} does not fit {width} bytes")))?;
                self.put(b, at, &bytes);
            }
            Unknown(PADDED_U32) => {
                let v = int(value)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| wrong(self))?;
                self.put(b, at, &[v.to_le_bytes(), [0; 4]].concat());
            }
            Float => {
                let v = float(value).ok_or_else(|| wrong(self))?;
                self.put(b, at, &(v as f32).to_le_bytes());
            }
            Double => {
                let v = float(value).ok_or_else(|| wrong(self))?;
                self.put(b, at, &v.to_le_bytes());
            }
            Float16 => {
                let v = float(value).ok_or_else(|| wrong(self))?;
                self.put(b, at, &crate::f32_to_half(v as f32).to_le_bytes());
            }
            ScalarV => {
                let v = float(value).ok_or_else(|| wrong(self))?;
                self.put(b, at, &(v as f32).to_le_bytes().repeat(4));
            }
            Vec2 | Vec2V | Vec3 | Vec3V | Vec4V => {
                let n = match entry.data_type {
                    Vec2 | Vec2V => 2,
                    Vec3 | Vec3V => 3,
                    _ => 4,
                };
                let v = floats(value, n).ok_or_else(|| wrong(self))?;
                self.put(b, at, &f32_bytes(&v));
            }
            Mat33V | Mat34V => {
                let v = floats(value, 12).ok_or_else(|| wrong(self))?;
                for (row, cells) in v.chunks(4).enumerate() {
                    self.put(b, at + 16 * row, &f32_bytes(cells));
                }
            }
            StringHash | PartialStringHash => {
                let h = hash32(value).ok_or_else(|| wrong(self))?;
                self.put(b, at, &h.to_le_bytes());
            }
            NsStringHash => {
                let h = hash32(value).ok_or_else(|| wrong(self))?;
                self.hold(false, h.into())?;
                self.put(b, at, &h.to_le_bytes());
            }
            Unknown(WIDE_NS_HASH) => {
                let h = hash64(value).ok_or_else(|| wrong(self))?;
                self.hold_wide(entry.union_9h, h)?;
                self.put(b, at, &h.to_le_bytes());
            }
            Enum8 | Enum16 | Enum32 => {
                let width = match entry.data_type {
                    Enum8 => 1,
                    Enum16 => 2,
                    _ => 4,
                };
                let v = self.enum_value(entry, value)?;
                let bytes = in_width(v.into(), width, v < 0).ok_or_else(|| {
                    self.fail(format!("enum value {v} does not fit {width} bytes"))
                })?;
                self.put(b, at, &bytes);
            }
            Bitset8 | Bitset16 | Bitset32 => {
                let width = match entry.data_type {
                    Bitset8 => 1,
                    Bitset16 => 2,
                    _ => 4,
                };
                let len = bitset_len(entry, width);
                let (_, needs) = bitset_value(value).ok_or_else(|| wrong(self))?;
                let words = bitset_words(value, len)
                    .ok_or_else(|| self.fail(format!("its {needs} bits do not fit {len} bytes")))?;
                self.put(b, at, &words);
            }
            Unknown(INLINE_BITSET) => {
                let bits = entry.contents_index() as usize;
                // A bit past the count fits the words but is refused when the file is read again.
                if bitset_value(value).is_some_and(|(_, needs)| needs > bits) {
                    return Err(self.fail(format!("it sets a bit past its {bits}")));
                }
                let words =
                    bitset_words(value, bits.div_ceil(64) * 8).ok_or_else(|| wrong(self))?;
                self.put(b, at, &words);
            }
            Unknown(BYTES_16) => match value {
                Value::ByteArray(bytes) if bytes.len() == 16 => self.put(b, at, bytes),
                _ => return Err(wrong(self)),
            },
            StringMember => {
                let text = text(value).ok_or_else(|| wrong(self))?;
                let room = entry.string_count() as usize;
                if text.len() > room {
                    return Err(self.fail(format!(
                        "its {} bytes of text do not fit the {room} in place",
                        text.len()
                    )));
                }
                let mut bytes = text.as_bytes().to_vec();
                bytes.resize(room, 0);
                self.put(b, at, &bytes);
            }
            StringPointer64 => {
                let text = text(value).ok_or_else(|| wrong(self))?;
                let target = self.text_region(text, bg)?;
                self.pointer(b, at, target);
            }
            AtString64 => {
                let text = text(value).ok_or_else(|| wrong(self))?;
                let mut header = [0u8; 16];
                if text.len() <= 14 {
                    header[..text.len()].copy_from_slice(text.as_bytes());
                    if !text.is_empty() {
                        header[15] = 0x80 | text.len() as u8;
                    }
                    // ⚠ An empty text stored as 0x80 in byte 15, the rest zero, keeps that byte,
                    // whose meaning is not known.
                    let marked = bg.and_then(|bg| bg.get(..16)).is_some_and(|stored| {
                        stored[15] == 0x80 && stored[..15].iter().all(|&byte| byte == 0)
                    });
                    if text.is_empty() && marked {
                        header[15] = 0x80;
                    }
                    self.put(b, at, &header);
                } else {
                    let len = u16::try_from(text.len())
                        .ok()
                        .filter(|&n| n < u16::MAX)
                        .ok_or_else(|| self.fail("its text passes 65,534 bytes"))?;
                    header[8..10].copy_from_slice(&len.to_le_bytes());
                    header[10..12].copy_from_slice(&(len + 1).to_le_bytes());
                    header[12] = 1;
                    // Bytes 0 to 7 are a pointer only where bit 7 of byte 15 is clear.
                    let bg = bg.filter(|bg| bg.get(15).is_some_and(|&last| last & 0x80 == 0));
                    // ⚠ The u32 at 12 of a stored text out of place is kept: it is 1 in every
                    // measured header but two, which hold 0, and what it means is not known.
                    let pointed = read_le(bg, 0, 8).is_some_and(|raw| raw & 0xFFF != 0);
                    if let Some(word) = bg.filter(|_| pointed).and_then(|bg| bg.get(12..16)) {
                        header[12..16].copy_from_slice(word);
                    }
                    self.put(b, at, &header);
                    let target = self.text_region(text, bg)?;
                    self.pointer(b, at, target);
                }
            }
            Struct => {
                let class = class_of(self.model, entry.referent_hash())?;
                let inst = instance(value).ok_or_else(|| wrong(self))?;
                self.structure(class, b, at, inst, bg)?;
            }
            Pointer64 | Unknown(TAGGED_POINTER) => match value {
                Value::Pointer(None) | Value::Null => self.pointer(b, at, None),
                Value::String(s) if s.is_empty() => self.pointer(b, at, None),
                Value::Pointer(Some(inst)) => {
                    let class = (self.by_name.get(&inst.type_name).copied()).ok_or_else(|| {
                        self.fail(format!(
                            "points at a {}, which is no structure of the document",
                            inst.type_name
                        ))
                    })?;
                    let stored = (self.stored_target(bg, 0)).filter(|s| s.ty == class.name_hash);
                    let bg = stored.map(|s| s.bytes);
                    let size = class.data_size as usize;
                    let target = self.region(class.name_hash, size, stored, size, bg)?;
                    self.pointer(b, at, Some(target));
                    self.structure(class, target.0, target.1, inst, bg)?;
                }
                _ => return Err(wrong(self)),
            },
            ArrayMember => {
                let items = items(value).ok_or_else(|| wrong(self))?;
                let count = entry.array_count() as usize;
                if items.len() != count {
                    return Err(self.fail(format!(
                        "holds {} elements, where its structure holds {count}",
                        items.len()
                    )));
                }
                self.in_place(owner, entry, b, at, items, bg)?;
            }
            AtFixedArray => {
                let items = items(value).ok_or_else(|| wrong(self))?;
                let element = element(owner, entry)?;
                let stride = self.stride(owner, element)?;
                let room = entry.array_count() as usize;
                if items.len() > room {
                    return Err(
                        self.fail(format!("holds {} elements in room for {room}", items.len()))
                    );
                }
                self.in_place(owner, entry, b, at, items, bg)?;
                self.put(b, at + stride * room, &(items.len() as u32).to_le_bytes());
            }
            AtArray32With32BitIdx => {
                let items = items(value).ok_or_else(|| wrong(self))?;
                let element = element(owner, entry)?;
                let stride = self.stride(owner, element)?;
                let room = entry.array_count() as usize;
                if items.len() > room {
                    return Err(
                        self.fail(format!("holds {} elements in room for {room}", items.len()))
                    );
                }
                let first = 8usize.next_multiple_of(self.align(element)?);
                let len = (stride.checked_mul(room))
                    .filter(|&len| len <= owner.data_size as usize)
                    .ok_or_else(|| {
                        self.fail(format!("room for {room} elements passes its structure"))
                    })?;
                self.put(
                    b,
                    at,
                    &[
                        (room as u32).to_le_bytes(),
                        (items.len() as u32).to_le_bytes(),
                    ]
                    .concat(),
                );
                // Room past the count is zero.
                self.put(b, at + first, &vec![0; len]);
                // ⚠ The lanes past a stored Vec2V's or Vec3V's hold live values no value states, so
                // the elements the document stored keep them.
                if matches!(element.data_type, Vec2V | Vec3V) {
                    let stored = read_le(bg, 4, 4).unwrap_or(0).min(items.len());
                    let room = bg.and_then(|bg| bg.get(first..));
                    let kept = room.map(|bg| bounded(bg, stride, stored));
                    self.carry((b, at + first), kept, len);
                }
                self.in_place(
                    owner,
                    entry,
                    b,
                    at + first,
                    items,
                    bg.and_then(|bg| bg.get(first..)),
                )?;
            }
            Unknown(COUNTED_ARRAY | TAGGED_ARRAY)
            | AtArray64
            | ArrayPointer32
            | Unknown(COUNTED_POINTER) => {
                let items = items(value).ok_or_else(|| wrong(self))?;
                self.pointed_array(owner, entry, b, at, items, bg)?;
            }
            AtBinaryMap64 => {
                let pairs = pairs(value).ok_or_else(|| wrong(self))?;
                self.binary_map(owner, entry, b, at, &pairs, bg)?;
            }
            Unknown(SLOT_MAP | GROUP_MAP) => {
                let pairs = pairs(value).ok_or_else(|| wrong(self))?;
                self.hash_table(owner, entry, b, at, Table::Pairs(&pairs), bg)?;
            }
            Unknown(SLOT_SET | GROUP_SET) => {
                let keys = items(value).ok_or_else(|| wrong(self))?;
                self.hash_table(owner, entry, b, at, Table::Keys(keys), bg)?;
            }
            AtBitset64 => {
                let stored_bits = bg
                    .and_then(|bg| bg.get(10..12))
                    .map(|w| u16::from_le_bytes([w[0], w[1]]) as usize);
                let set = bitset_value(value).ok_or_else(|| wrong(self))?;
                let bits = stored_bits.filter(|&n| n > 0).unwrap_or(set.1).max(set.1);
                let words = bits.div_ceil(32);
                if words == 0 {
                    self.put(b, at, &[0; 16]);
                    return Ok(());
                }
                let bytes = bitset_words(value, 4 * words)
                    .ok_or_else(|| self.fail(format!("its bits do not fit {bits}")))?;
                let (bits, words) = (u16::try_from(bits), u16::try_from(words));
                let (Ok(bits), Ok(words)) = (bits, words) else {
                    return Err(self.fail("it passes 65,535 bits"));
                };
                let stored = self.stored_target(bg, 0);
                let stored_len = 4 * read_le(bg, 8, 2).unwrap_or(0);
                let ty = stored.map_or(0x15, |s| s.ty);
                let target = self.region(ty, bytes.len(), stored, stored_len, None)?;
                self.put(target.0, target.1, &bytes);
                self.put(
                    b,
                    at + 8,
                    &[words.to_le_bytes(), bits.to_le_bytes(), [0; 2], [0; 2]].concat(),
                );
                self.pointer(b, at, Some(target));
            }
            _ => {
                return Err(self.fail(format!(
                    "a member of type {code:#04X} has no measured layout to write"
                )))
            }
        }
        // ⚠ A tagged pointer sets bit 63 when it is null too.
        match entry.data_type {
            Unknown(TAGGED_POINTER) => self.tag(b, at),
            Unknown(TAGGED_ARRAY) => self.tag(b, at + 8),
            _ => {}
        }
        Ok(())
    }

    fn hold(&mut self, wide: bool, hash: u64) -> Result<()> {
        if hash != 0 && self.held.is_some_and(|held| !held.contains(&(wide, hash))) {
            return Err(self.fail(format!(
                "holds the string hash {hash:#X}, which the document did not hold: its text belongs in LSNS, which is encrypted, so none can be added"
            )));
        }
        self.laid.insert((wide, hash));
        Ok(())
    }

    /// [`Self::hold`] for a 64-bit hash of `namespace`, which a free namespace takes unheld.
    fn hold_wide(&mut self, namespace: u8, hash: u64) -> Result<()> {
        if (self.held).is_some_and(|held| held.contains(&free_namespace(namespace))) {
            self.laid.insert((true, hash));
            return Ok(());
        }
        self.hold(true, hash)
    }

    /// `text` and its NUL as a region of the block the stored pointer at the start of `bg` named,
    /// else of type 0x10; `None` for no text, unless the document pointed at an empty one.
    fn text_region(&mut self, text: &str, bg: Background) -> Result<Option<(usize, usize)>> {
        if text.as_bytes().contains(&0) {
            return Err(self.fail("its text holds a NUL"));
        }
        let stored = self.stored_target(bg, 0);
        let stored_len =
            (stored.and_then(|s| s.bytes.iter().position(|&b| b == 0))).map_or(0, |n| n + 1);
        if text.is_empty() && stored_len != 1 {
            return Ok(None);
        }
        let ty = stored.map_or(0x10, |s| s.ty);
        let target = self.region(ty, text.len() + 1, stored, stored_len, None)?;
        self.put(target.0, target.1, &[text.as_bytes(), &[0]].concat());
        Ok(Some(target))
    }

    fn in_place(
        &mut self,
        owner: &'m RbmClass,
        array: &'m RbmClassEntry,
        b: usize,
        at: usize,
        items: &[Value],
        bg: Background<'m>,
    ) -> Result<()> {
        let element = element(owner, array)?;
        let stride = self.stride(owner, element)?;
        for (i, item) in items.iter().enumerate() {
            self.path.push(format!("[{i}]"));
            let bg = bg.and_then(|bg| bg.get(stride * i..));
            self.member(owner, element, b, at + stride * i, item, bg)?;
            self.path.pop();
        }
        Ok(())
    }

    /// A pointer to `items` with its count: `{u32 count, u32 capacity, u64}` (0x5C),
    /// `{u64, u16 count, u16 capacity, u32 0}` (0x52), `{u64, u32 count, u32 capacity}` (0x56), or
    /// the pointer alone (0x5A); all zero for none.
    fn pointed_array(
        &mut self,
        owner: &'m RbmClass,
        entry: &'m RbmClassEntry,
        b: usize,
        at: usize,
        items: &[Value],
        bg: Background<'m>,
    ) -> Result<()> {
        let (ptr, count_at) = match entry.data_type {
            PsoType::Unknown(COUNTED_ARRAY | TAGGED_ARRAY) => (8, Some(0)),
            PsoType::Unknown(COUNTED_POINTER) => (0, None),
            _ => (0, Some(8)),
        };
        let counted = matches!(entry.data_type, PsoType::Unknown(COUNTED_POINTER));
        let wide = matches!(entry.data_type, PsoType::AtArray64);
        let width = if wide { 2 } else { 4 };
        let n = items.len();
        let element = element(owner, entry)?;
        let stored_n = match count_at {
            Some(c) => read_le(bg, c, width),
            None => self.stored_count.take(),
        };
        let stored = self.stored_target(bg, ptr);
        // An empty array the document kept behind a pointer keeps it, as a region of no bytes.
        let kept_empty = n == 0 && stored.is_some() && stored_n == Some(0);
        let mut head = vec![0u8; if counted { 8 } else { 16 }];
        if let (Some(c), true) = (count_at, n > 0 || kept_empty) {
            // A capacity the document stated for this count is kept; otherwise it is the count.
            let room = (stored_n == Some(n))
                .then(|| read_le(bg, c + width, width))
                .flatten();
            let room = room.unwrap_or(n);
            let (Ok(n), Ok(room)) = (u32::try_from(n), u32::try_from(room)) else {
                return Err(self.fail(format!("{n} elements pass a u32")));
            };
            if wide && (n > 0xFFFF || room > 0xFFFF) {
                return Err(self.fail(format!("{n} elements pass the 65,535 a u16 counts")));
            }
            head[c..c + width].copy_from_slice(&n.to_le_bytes()[..width]);
            head[c + width..c + 2 * width].copy_from_slice(&room.to_le_bytes()[..width]);
        }
        self.put(b, at, &head);
        // An empty array's element may name a structure the document does not declare.
        if n == 0 && !kept_empty {
            return Ok(());
        }
        let stride = match n {
            0 => 0,
            _ => self.stride(owner, element)?,
        };
        let ty = stored.map_or_else(|| region_type(element), |s| s.ty);
        let len = (stride.checked_mul(n))
            .ok_or_else(|| self.fail("its elements pass the address space"))?;
        let stored_len = stride.saturating_mul(stored_n.unwrap_or(0));
        let target = self.region(ty, len, stored, stored_len, None)?;
        self.pointer(b, at + ptr, Some(target));
        if n == 0 {
            return Ok(());
        }
        // Only the stored elements carry: what follows them in their block is another region.
        let bg = stored.map(|s| bounded(s.bytes, stride, stored_n.unwrap_or(0)));
        self.carry(target, bg, len);
        self.in_place(owner, entry, target.0, target.1, items, bg)
    }

    /// The first `len` bytes of `bg` at `target`, as many as it holds.
    fn carry(&mut self, target: (usize, usize), bg: Background, len: usize) {
        if let Some(bg) = bg {
            let bytes = bg[..bg.len().min(len)].to_vec();
            self.put(target.0, target.1, &bytes);
        }
    }

    /// `{u32 1, u32 0, u64, u16 count, u16 capacity, u32 0}` and its pairs in the order given.
    fn binary_map(
        &mut self,
        owner: &'m RbmClass,
        entry: &'m RbmClassEntry,
        b: usize,
        at: usize,
        pairs: &[(&Value, &Value)],
        bg: Background<'m>,
    ) -> Result<()> {
        let mut head = [0u8; 24];
        head[0] = 1;
        let n = u16::try_from(pairs.len())
            .map_err(|_| self.fail("its pairs pass the 65,535 a u16 counts"))?;
        head[16..18].copy_from_slice(&n.to_le_bytes());
        head[18..20].copy_from_slice(&n.to_le_bytes());
        self.put(b, at, &head);
        if pairs.is_empty() {
            return Ok(());
        }
        let stored = self.stored_target(bg, 8);
        let pair = self.unit(owner, entry, stored.map(|s| s.ty), false)?;
        let size = pair.data_size as usize;
        let len = size * pairs.len();
        let stored_n = read_le(bg, 16, 2).unwrap_or(0);
        let target = self.region(pair.name_hash, len, stored, size * stored_n, None)?;
        self.pointer(b, at + 8, Some(target));
        let bg = stored.map(|s| bounded(s.bytes, size, stored_n));
        self.carry(target, bg, len);
        for (i, (key, value)) in pairs.iter().enumerate() {
            let bg = bg.and_then(|bg| bg.get(size * i..));
            self.pair(pair, target.0, target.1 + size * i, (key, None), value, bg)?;
        }
        Ok(())
    }

    /// A pair at `at`; with `owner_key`, an enum key is written in its owner's key member's enum,
    /// which a pair structure several maps share need not name.
    fn pair(
        &mut self,
        pair: &'m RbmClass,
        b: usize,
        at: usize,
        (key, owner_key): (&Value, Option<&'m RbmClassEntry>),
        value: &Value,
        bg: Background<'m>,
    ) -> Result<()> {
        use PsoType::{Enum16, Enum32, Enum8};
        let named: Vec<&'m RbmClassEntry> = (pair.entries.iter())
            .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
            .collect();
        let [k, v] = named[..] else {
            return Err(self.fail(format!(
                "its pairs are a {}, which has no key and value",
                class_name(pair)
            )));
        };
        // The pair's key member gives the place, its owner's the enum.
        let enum_key = matches!(k.data_type, Enum8 | Enum16 | Enum32);
        let written = match owner_key {
            Some(o) if enum_key && o.data_type == k.data_type => o,
            _ => k,
        };
        for (place, entry, value, what) in [(k, written, key, "{key}"), (v, v, value, "{value}")] {
            self.path.push(what.into());
            let off = place.offset as usize;
            self.member(
                pair,
                entry,
                b,
                at + off,
                value,
                bg.and_then(|bg| bg.get(off..)),
            )?;
            self.path.pop();
        }
        Ok(())
    }

    /// A hash map's pair structure (ungrouped), a group map's group structure, or a group set's.
    fn unit(
        &self,
        owner: &RbmClass,
        entry: &RbmClassEntry,
        stored: Option<u32>,
        grouped: bool,
    ) -> Result<&'m RbmClass> {
        if let Some(ty) = stored {
            return class_of(self.model, ty);
        }
        let types = |e: &RbmClassEntry| (e.data_type, e.union_ch);
        let key = owner.entry(entry.map_key_index()).map(types);
        let value = owner.entry(entry.map_value_index()).map(types);
        let is_pair = |c: &RbmClass| {
            let named: Vec<_> = (c.entries.iter())
                .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
                .map(|e| Some(types(e)))
                .collect();
            named == [key, value]
        };
        let fits = |c: &&RbmClass| match grouped {
            false => is_pair(c),
            true => (c.entries.len() == 4 && c.entries[2].data_type == PsoType::Struct)
                .then(|| self.model.class_for(c.entries[2].referent_hash()))
                .flatten()
                .is_some_and(|p| {
                    is_pair(p) && u64::from(c.data_size) == 16 + 16 * u64::from(p.data_size)
                }),
        };
        let mut found = self.model.classes.values().filter(fits);
        match (found.next(), found.next()) {
            (Some(class), None) => Ok(class),
            (None, _) => Err(self.fail("no structure of the document holds its pairs")),
            (Some(_), Some(_)) => Err(self.fail("more than one structure of the document could hold its pairs, and it held none to say which")),
        }
    }

    fn hash_table(
        &mut self,
        owner: &'m RbmClass,
        entry: &'m RbmClassEntry,
        b: usize,
        at: usize,
        table: Table,
        bg: Background<'m>,
    ) -> Result<()> {
        let n = table.len();
        if n == 0 {
            self.put(b, at, &[0; 16]);
            return Ok(());
        }
        let grouped = matches!(entry.data_type, PsoType::Unknown(GROUP_MAP | GROUP_SET));
        let slots = if grouped {
            let mut g = 2;
            while n >= 14 * g {
                g *= 2;
            }
            g
        } else {
            let mut s = 32;
            while 4 * n > 3 * s {
                s *= 2;
            }
            s
        };
        let stored = self.stored_target(bg, 8);
        let key = owner
            .entry(entry.map_key_index())
            .ok_or_else(|| self.fail("its key member is not in its structure"))?;
        let (unit_type, unit_size, pair, width) = match table {
            Table::Pairs(_) => {
                let unit = self.unit(owner, entry, stored.map(|s| s.ty), grouped)?;
                let pair = match grouped {
                    true => class_of(
                        self.model,
                        unit.entries.get(2).map_or(0, |e| e.referent_hash()),
                    )?,
                    false => unit,
                };
                (
                    unit.name_hash,
                    unit.data_size as usize,
                    Some(pair),
                    pair.data_size as usize,
                )
            }
            Table::Keys(_) => {
                let width = self.stride(owner, key)?;
                let ty = match (stored, grouped) {
                    (Some(s), _) => s.ty,
                    (None, false) => region_type(key),
                    (None, true) => self.group_of_keys(key, width)?,
                };
                (
                    ty,
                    if grouped { 16 + 16 * width } else { width },
                    None,
                    width,
                )
            }
        };
        if grouped && unit_size != 16 + 16 * width {
            return Err(self.fail("its group structure is not 16 control bytes and 16 entries"));
        }
        let key_off = pair
            .and_then(|p| {
                p.entries
                    .iter()
                    .find(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
            })
            .map_or(0, |e| e.offset as usize);
        let stored_slots = read_le(bg, 0, 4).unwrap_or(0);
        let stored_table = stored.map(|s| bounded(s.bytes, unit_size, stored_slots));
        // The tree lists a table's entries in their stored slot order, so the i-th stored entry is
        // the i-th entry's background.
        let stored_starts = stored_table.map_or_else(Vec::new, |bytes| {
            stored_entries(bytes, unit_size, grouped, width)
        });
        let key_width = self.stride(owner, key)?;
        let bits = (0..n)
            .map(|i| self.key_bits(key, table.key(i)))
            .collect::<Result<Vec<u64>>>()?;
        // ⚠ While the keys are the ones stored, in their order, each keeps its stored place: the
        // order the table was filled in, which a place depends on, is not in the file.
        let kept = stored_slots == slots
            && stored_starts.len() == n
            && (0..n).all(|i| {
                read_le(stored_table, stored_starts[i] + key_off, key_width)
                    .is_some_and(|stored| stored as u64 == bits[i])
            });
        if !kept && matches!(key.data_type, PsoType::S64 | PsoType::U64) {
            return Err(self.fail(
                "its keys are 64-bit integers, whose places in a table no measured hash gives, so it is written only with the keys it held, in their slots",
            ));
        }
        // Where each entry goes, in the order given: home is the hash's low bits, then linear.
        let mut places = Vec::with_capacity(n);
        let mut used = vec![0usize; slots];
        let mut taken = vec![false; slots];
        for (i, &bits) in bits.iter().enumerate() {
            let h = key_hash(key.data_type, bits);
            let control = (h & 0x7F) as u8;
            if kept {
                let pos = stored_starts[i];
                let g = pos / unit_size;
                let control =
                    grouped.then(|| (g * unit_size + (pos - g * unit_size - 16) / width, control));
                places.push((pos, control));
            } else if grouped {
                let mut g = ((h >> 7) as usize) & (slots - 1);
                while used[g] == 16 {
                    g = (g + 1) & (slots - 1);
                }
                places.push((
                    g * unit_size + 16 + width * used[g],
                    Some((g * unit_size + used[g], control)),
                ));
                used[g] += 1;
            } else {
                let mut s = (h as usize) & (slots - 1);
                while taken[s] {
                    s = (s + 1) & (slots - 1);
                }
                taken[s] = true;
                places.push((s * unit_size, None));
            }
        }
        let len = slots * unit_size;
        let target = self.region(unit_type, len, stored, stored_slots * unit_size, None)?;
        // Every slot is rebuilt: an empty one is zero, a group's empty control byte 0x80.
        self.put(target.0, target.1, &vec![0; len]);
        if grouped {
            for g in 0..slots {
                self.put(target.0, target.1 + g * unit_size, &[EMPTY_CONTROL; 16]);
            }
        }
        let mut head = [0u8; 16];
        head[0..4].copy_from_slice(&(slots as u32).to_le_bytes());
        head[4..8].copy_from_slice(&(n as u32).to_le_bytes());
        self.put(b, at, &head);
        self.pointer(b, at + 8, Some(target));
        let visit = |w: &mut Self, i: usize| -> Result<()> {
            let (pos, control) = places[i];
            if let Some((c, byte)) = control {
                w.put(target.0, target.1 + c, &[byte]);
            }
            let bg =
                stored_table.and_then(|bytes| stored_starts.get(i).and_then(|&s| bytes.get(s..)));
            if let Some(bg) = bg.and_then(|bg| bg.get(..width)) {
                let bytes = bg.to_vec();
                w.put(target.0, target.1 + pos, &bytes);
            }
            w.path.push(format!("{{{i}}}"));
            match (&table, pair) {
                (Table::Pairs(pairs), Some(pair)) => w.pair(
                    pair,
                    target.0,
                    target.1 + pos,
                    (pairs[i].0, Some(key)),
                    pairs[i].1,
                    bg,
                )?,
                _ => w.member(
                    owner,
                    key,
                    target.0,
                    target.1 + pos + key_off,
                    table.key(i),
                    bg,
                )?,
            }
            w.path.pop();
            Ok(())
        };
        // A rebuilt table moves only its slots: the regions its entries reach keep the order they
        // are stored in, which a first pass, undone, finds; an entry the document did not hold
        // reaches none.
        let order = match stored_table.is_some() && self.trace.is_none() && !self.in_place {
            true => {
                let mark = self.mark();
                self.trace = Some(Vec::new());
                let mut reached = Vec::with_capacity(n);
                for i in 0..n {
                    let from = self.trace.as_ref().map_or(0, Vec::len);
                    visit(self, i)?;
                    let trace = self.trace.as_deref().unwrap_or_default();
                    reached.push(trace.get(from..).unwrap_or_default().to_vec());
                }
                self.trace = None;
                self.rewind(mark);
                payload_order(&reached)
            }
            false => (0..n).collect(),
        };
        for i in order {
            visit(self, i)?;
        }
        Ok(())
    }

    /// Where every block ends and how many pointers wait, for [`Self::rewind`].
    fn mark(&self) -> (Vec<usize>, usize) {
        let ends = self.blocks.iter().map(|(_, data)| data.len()).collect();
        (ends, self.fixups.len())
    }

    /// Undoes every region and pointer laid out since `mark`.
    fn rewind(&mut self, (ends, fixups): (Vec<usize>, usize)) {
        self.blocks.truncate(ends.len());
        self.sources.truncate(ends.len());
        self.owners.truncate(ends.len());
        for ((_, data), end) in self.blocks.iter_mut().zip(ends) {
            data.truncate(end);
        }
        let open = self.blocks.len();
        (self.index.values_mut()).for_each(|held| held.retain(|&b| b < open));
        self.fixups.truncate(fixups);
    }

    /// The structure that groups 16 control bytes and 16 keys of `key`, for a set the document
    /// held empty.
    fn group_of_keys(&self, key: &RbmClassEntry, width: usize) -> Result<u32> {
        let mut found = self.model.classes.values().filter(|c| {
            c.data_size as usize == 16 + 16 * width
                && c.entries.len() == 4
                && c.entries[2].data_type == key.data_type
                && c.entries[2].union_ch == key.union_ch
        });
        match (found.next(), found.next()) {
            (Some(class), None) => Ok(class.name_hash),
            _ => Err(self.fail("the document has no one structure for its groups of keys, and it held none to say which")),
        }
    }

    /// A key's bits as its table hashes them: a hash as it is, an integer or enum value in its width.
    fn key_bits(&self, key: &RbmClassEntry, value: &Value) -> Result<u64> {
        use PsoType::*;
        let wrong = || {
            self.fail(format!(
                "its key holds {}, which no measured hash places",
                kind(value)
            ))
        };
        Ok(match key.data_type {
            StringHash | NsStringHash => u64::from(hash32(value).ok_or_else(wrong)?),
            Unknown(WIDE_NS_HASH) => hash64(value).ok_or_else(wrong)?,
            Unknown(WIDE_HASH) | S64 | U64 => int(value).ok_or_else(wrong)? as u64,
            U8 | U16 | U32 | S8 | S16 | S32 => {
                let width = self.stride_of(key.data_type).unwrap_or(4);
                mask(int(value).ok_or_else(wrong)? as u64, width)
            }
            Enum8 | Enum16 | Enum32 => {
                let width = self.stride_of(key.data_type).unwrap_or(4);
                mask(self.enum_value(key, value)? as u64, width)
            }
            _ => return Err(wrong()),
        })
    }
    fn stride_of(&self, ty: PsoType) -> Option<usize> {
        use PsoType::*;
        Some(match ty {
            U8 | S8 | Enum8 => 1,
            U16 | S16 | Enum16 => 2,
            U32 | S32 | Enum32 => 4,
            _ => return None,
        })
    }

    fn enum_value(&self, entry: &RbmClassEntry, value: &Value) -> Result<i32> {
        let variant = match value {
            Value::Enum { variant, .. } => variant.as_str(),
            Value::String(s) => s.as_str(),
            other => {
                return int(other)
                    .and_then(|v| {
                        i32::try_from(v)
                            .ok()
                            .or_else(|| u32::try_from(v).ok().map(|v| v as i32))
                    })
                    .ok_or_else(|| {
                        self.fail(format!("holds {}, which no enum value is", kind(other)))
                    })
            }
        };
        if let Some(v) = enum_text::value_literal(variant) {
            return Ok(v);
        }
        let values = (self.model.enum_for(entry.referent_hash()).into_iter())
            .flat_map(|e| e.values.iter().map(|v| (v.name_hash, v.value)));
        enum_text::resolve_named(variant, values)
            .ok_or_else(|| self.fail(format!("{variant} names no value of its enum")))
    }

    fn stride(&self, owner: &RbmClass, element: &RbmClassEntry) -> Result<usize> {
        use PsoType::*;
        Ok(match element.data_type {
            Struct => class_of(self.model, element.referent_hash())?.data_size as usize,
            Bool | S8 | U8 | Enum8 | Bitset8 => 1,
            S16 | U16 | Float16 | Enum16 | Bitset16 => 2,
            S32 | U32 | Float | StringHash | PartialStringHash | NsStringHash | Enum32
            | Bitset32 => 4,
            S64
            | U64
            | Double
            | Pointer64
            | Vec2
            | StringPointer64
            | Unknown(WIDE_HASH | WIDE_NS_HASH | PADDED_U32 | TAGGED_POINTER) => 8,
            Vec3 => 12,
            Vec2V
            | Vec3V
            | Vec4V
            | ScalarV
            | AtArray64
            | AtString64
            | Unknown(COUNTED_ARRAY | BYTES_16) => 16,
            Mat33V => 48,
            Mat34V => 64,
            // `N` elements, then a u32 count, rounded up to the elements' alignment and at least 4.
            AtFixedArray => {
                let inner = (owner.entry(element.contents_index()))
                    .filter(|inner| inner.data_type != AtFixedArray)
                    .ok_or_else(|| {
                        self.fail(format!(
                            "a fixed array of {} has no element with a measured stride",
                            class_name(owner)
                        ))
                    })?;
                let room = element.array_count() as usize;
                let size = self.stride(owner, inner)?;
                let align = self.align(inner)?.max(4);
                (size.checked_mul(room))
                    .and_then(|len| len.checked_add(4))
                    .and_then(|len| len.checked_next_multiple_of(align))
                    .ok_or_else(|| self.fail(format!("{room} elements pass the address space")))?
            }
            other => {
                return Err(self.fail(format!(
                    "an element of type {:#04X} of {} has no measured stride",
                    other.to_byte(),
                    class_name(owner)
                )))
            }
        })
    }

    fn align(&self, element: &RbmClassEntry) -> Result<usize> {
        use PsoType::*;
        Ok(match element.data_type {
            Struct => {
                1usize
                    << class_of(self.model, element.referent_hash())?
                        .align_power
                        .min(12)
            }
            Vec2V | Vec3V | Vec4V | Mat33V | Mat34V | ScalarV => 16,
            _ => self
                .stride_of(element.data_type)
                .unwrap_or(match element.data_type {
                    Bool | Bitset8 => 1,
                    Float16 | Bitset16 => 2,
                    Float | Vec3 | StringHash | PartialStringHash | NsStringHash | Bitset32 => 4,
                    _ => 8,
                }),
        })
    }
}

enum Table<'v> {
    Pairs(&'v [(&'v Value, &'v Value)]),
    Keys(&'v [Value]),
}

impl Table<'_> {
    fn len(&self) -> usize {
        match self {
            Table::Pairs(p) => p.len(),
            Table::Keys(k) => k.len(),
        }
    }

    fn key(&self, i: usize) -> &Value {
        match self {
            Table::Pairs(p) => p[i].0,
            Table::Keys(k) => &k[i],
        }
    }
}

/// The first `count` elements of `stride` bytes of `bytes`, as many as it holds.
fn bounded(bytes: &[u8], stride: usize, count: usize) -> &[u8] {
    &bytes[..bytes.len().min(stride.saturating_mul(count))]
}

/// The `width` little-endian bytes at `at` of `bg`.
fn read_le(bg: Background, at: usize, width: usize) -> Option<usize> {
    let bytes = bg?.get(at..at.checked_add(width)?)?;
    Some(
        bytes
            .iter()
            .rev()
            .fold(0usize, |a, &b| a << 8 | usize::from(b)),
    )
}

/// Where the full entries of a stored hash table start, in slot order.
fn stored_entries(bytes: &[u8], unit: usize, grouped: bool, width: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    for (i, slot) in bytes.chunks_exact(unit).enumerate() {
        if !grouped {
            if slot.iter().any(|&b| b != 0) {
                starts.push(unit * i);
            }
            continue;
        }
        for (j, &control) in slot[..16].iter().enumerate() {
            if control < EMPTY_CONTROL {
                starts.push(unit * i + 16 + width * j);
            }
        }
    }
    starts
}

/// The hash a table places a key by: a name hash as it is, a 64-bit integer through fmix64, any
/// other through fmix32.
fn key_hash(ty: PsoType, bits: u64) -> u64 {
    use PsoType::*;
    match ty {
        StringHash | NsStringHash | Unknown(WIDE_NS_HASH | WIDE_HASH) => bits,
        S64 | U64 => fmix64(bits),
        _ => u64::from(fmix32(bits as u32)),
    }
}

/// The block-table order of blocks met in `blocks`' order: buckets 0 up, each from the block last
/// inserted, the table growing before a new type would fill its buckets.
// ⚠ A second block of a type goes right after the one before it in its bucket, not at the head,
// and is no new key: the growth step counts types, not blocks.
pub(super) fn block_order(blocks: &[(u32, Vec<u8>)]) -> Result<Vec<usize>> {
    let mut last = HashMap::new();
    let before: Vec<Option<usize>> = (blocks.iter().enumerate())
        .map(|(i, (ty, _))| last.insert(*ty, i))
        .collect();
    let insert = |bucket: &mut Vec<usize>, x: usize| match before[x]
        .and_then(|b| bucket.iter().position(|&y| y == b))
    {
        Some(at) => bucket.insert(at + 1, x),
        None => bucket.insert(0, x),
    };
    let mut k = 0;
    let mut types = 0;
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); BUCKETS[0]];
    for (i, (ty, _)) in blocks.iter().enumerate() {
        if before[i].is_some() {
            let size = buckets.len();
            insert(&mut buckets[*ty as usize % size], i);
            continue;
        }
        types += 1;
        if types >= BUCKETS[k] {
            k += 1;
            let size = *BUCKETS.get(k).ok_or_else(|| {
                malformed(format!(
                    "{} blocks pass every measured bucket count",
                    blocks.len()
                ))
            })?;
            let old: Vec<usize> = buckets.concat();
            buckets = vec![Vec::new(); size];
            for x in old {
                insert(&mut buckets[blocks[x].0 as usize % size], x);
            }
        }
        let size = buckets.len();
        insert(&mut buckets[*ty as usize % size], i);
    }
    Ok(buckets.concat())
}

/// The order to write a hash table's entries in that keeps each block's stored regions in their
/// stored order, `reached[i]` being the stored `(block, offset)` of every region entry `i`
/// reaches: the list order where nothing decides it, and where no one order keeps them all.
fn payload_order(reached: &[Vec<(usize, usize)>]) -> Vec<usize> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = reached.len();
    let mut per_block: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for (i, regions) in reached.iter().enumerate() {
        for &(block, at) in regions {
            per_block.entry(block).or_default().push((at, i));
        }
    }
    let mut after: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    let mut waits = vec![0usize; n];
    for regions in per_block.values_mut() {
        regions.sort_unstable();
        for pair in regions.windows(2) {
            let (a, b) = (pair[0].1, pair[1].1);
            if a != b && after[a].insert(b) {
                waits[b] += 1;
            }
        }
    }
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&i| waits[i] == 0).map(Reverse).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(Reverse(i)) = ready.pop() {
        order.push(i);
        for &j in &after[i] {
            waits[j] -= 1;
            if waits[j] == 0 {
                ready.push(Reverse(j));
            }
        }
    }
    match order.len() == n {
        true => order,
        false => (0..n).collect(),
    }
}

/// The block a region of elements of `element` goes in: its structure's, else its type code's.
fn region_type(element: &RbmClassEntry) -> u32 {
    match element.data_type {
        PsoType::Struct => element.referent_hash(),
        other => other.to_byte().into(),
    }
}

fn element<'c>(owner: &'c RbmClass, array: &RbmClassEntry) -> Result<&'c RbmClassEntry> {
    owner.entry(array.contents_index()).ok_or_else(|| {
        malformed(format!(
            "{} names element member {}, which it does not have",
            class_name(owner),
            array.contents_index()
        ))
    })
}

fn class_name(class: &RbmClass) -> String {
    default_hash_index().name_or_literal(class.name_hash)
}

#[cfg(test)]
#[path = "table_layout_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "table_layout_grow_tests.rs"]
mod grow_tests;
