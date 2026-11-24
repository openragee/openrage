//! GTA VI's PSO table form, read only: `PSIN LSCH LENM LSNS LGID LRSC CHKS`,
//! little-endian records inside big-endian section frames, with no `PMAP`.

use std::collections::{HashMap, HashSet};

use crate::meta::binary::enum_text;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::rbm::reader::{
    read_f32_le, read_i16_le, read_i32_le, read_matrix, read_u16_le, read_u32_le, read_u64_le,
    read_u8, variant_name,
};
use crate::meta::binary::rbm::schema::{
    class_entry, class_header, PsoType, RbmClass, RbmClassEntry, RbmDataBlock, RbmEnum,
    RbmEnumValue, RbmModel,
};
use crate::meta::binary::RBM_PRD0_MARKER_LE;
use crate::meta::error::{MetaError, Result};
use crate::meta::instance::value::{MetaInstance, Value};

use super::data_names::{DataNames, Unnamed};
use super::sections::{PsoSection, RawPsoSection, LSCH_IDENT};
use super::table_layout::DEPTH;

/// What stops a document in GTA VI's table form from being read or saved.
#[derive(Debug, thiserror::Error)]
pub enum TableFormError {
    #[error("GTA VI PSO: the sections are {found}, where every measured document has {measured}")]
    Sections { found: String, measured: String },
    #[error("GTA VI metadata: {0}")]
    Malformed(String),
    #[error("GTA VI metadata: member {member} of {structure} has type {code:#04X}, {why}")]
    NotKnown {
        structure: String,
        member: String,
        code: u8,
        why: &'static str,
    },
    #[error("GTA VI PSO: this document is not saved, because the measured layout rules do not give back the bytes it was read from: {0}")]
    RulesMiss(String),
    #[error("GTA VI PSO: this build of OpenRage reads GTA VI's table form but has no GTA VI support to save it")]
    NoGta6,
    #[error("GTA VI PSO: a document can be saved only into the allocation it arrived with, because GTA VI's layout rules are not known")]
    NoLayoutRules,
}

impl From<TableFormError> for MetaError {
    fn from(error: TableFormError) -> Self {
        MetaError::Other(error.to_string())
    }
}

pub(super) const ARRAY_INFO_NAME_HASH: u32 = 0x100;
/// The array kind whose 16-byte header is `{u32 count, u32 capacity, u64 pointer}`.
pub(super) const COUNTED_ARRAY: u8 = 0x5C;
/// Eight bytes in place, read as the u64 they hold; 0x4F also states a namespace in byte 9.
pub(super) const WIDE_HASH: u8 = 0x08;
pub(super) const WIDE_NS_HASH: u8 = 0x4F;
/// A hash map of `slots` pairs, an empty one all zero.
pub(super) const SLOT_MAP: u8 = 0x73;
/// A hash map of `slots` groups of 16 control bytes and 16 pairs.
pub(super) const GROUP_MAP: u8 = 0x75;
/// A bitset of `u10` bits in place, in 64-bit words.
pub(super) const INLINE_BITSET: u8 = 0x68;
pub(super) const EMPTY_CONTROL: u8 = 0x80;
/// Eight bytes in place: a u32, then a u32 every measured one holds zero.
pub(super) const PADDED_U32: u8 = 0x0C;
/// Sixteen bytes in place whose meaning is not known.
pub(super) const BYTES_16: u8 = 0x0D;
/// A u64 pointer to elements of member `u10`, counted by the value of member `u12`.
pub(super) const COUNTED_POINTER: u8 = 0x5A;
/// The key-only sets of [`SLOT_MAP`] and [`GROUP_MAP`].
pub(super) const SLOT_SET: u8 = 0x76;
pub(super) const GROUP_SET: u8 = 0x77;
/// [`COUNTED_ARRAY`] whose count equals its capacity and whose pointer sets [`TAG`], null included.
pub(super) const TAGGED_ARRAY: u8 = 0x5E;
/// A u64 pointer that sets [`TAG`], null included; its target's class is its block's type.
pub(super) const TAGGED_POINTER: u8 = 0x0A;
/// Bit 63 of a tagged pointer, whose meaning is not known.
pub(super) const TAG: u64 = 1 << 63;

/// The bytes a 0x63, 0x64 or 0x65 member of `width`-byte words holds in place.
// ⚠ A named member of more `u10` bits than a word holds as many words as they take; an anonymous
// element row's `u10` does not size it, as rows of 256 bits sit in rooms of 4 and 8 bytes.
pub(super) fn bitset_len(entry: &RbmClassEntry, width: usize) -> usize {
    let bits = usize::from(entry.contents_index());
    match entry.name_hash != ARRAY_INFO_NAME_HASH && bits > 8 * width {
        true => width * bits.div_ceil(8 * width),
        false => width,
    }
}

/// The bytes an element of `ty` takes when its size follows from its type alone.
pub(crate) fn fixed_width(ty: PsoType) -> Option<usize> {
    use PsoType::*;
    Some(match ty {
        Bool | S8 | U8 | Enum8 | Bitset8 => 1,
        S16 | U16 | Float16 | Enum16 | Bitset16 => 2,
        S32 | U32 | Float | StringHash | PartialStringHash | NsStringHash | Enum32 | Bitset32 => 4,
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
        _ => return None,
    })
}

/// Whether `sections` are the table form: `LSCH` second, where the other forms have `PMAP`.
pub(crate) fn is_table_form(sections: &[RawPsoSection]) -> bool {
    sections.get(1).map(|s| s.ident) == Some(PsoSection::Other(LSCH_IDENT))
}

/// The value tree a table-form document holds.
pub(crate) fn read(sections: &[RawPsoSection]) -> Result<MetaInstance> {
    decode(&model(sections)?)
}

/// The value tree a GTA VI type table and its blocks hold, from the root block down.
///
/// A hash value is named by the tree's names and by the document's own strings, which a second
/// read applies when the first found a string that names a value it left unnamed.
pub(crate) fn decode(model: &RbmModel) -> Result<MetaInstance> {
    let _names = crate::meta::binary::hash_index::title_names();
    let mut names = DataNames::of_tree();
    let (instance, unnamed) = decode_with(model, &names)?;
    if !names.add_naming(&unnamed) {
        return Ok(instance);
    }
    Ok(decode_with(model, &names)?.0)
}

fn decode_with(model: &RbmModel, names: &DataNames) -> Result<(MetaInstance, Unnamed)> {
    let block = model.block(model.root_block_id as i32).ok_or_else(|| {
        malformed(format!(
            "the root names block {} of {}",
            model.root_block_id,
            model.data_blocks.len()
        ))
    })?;
    let class = class_of(model, block.data_type)?;
    let bytes: usize = model.data_blocks.iter().map(|b| b.data.len()).sum();
    let mut decoder = Decoder {
        model,
        names,
        unnamed: Unnamed::default(),
        open: HashSet::new(),
        budget: bytes.saturating_mul(VALUES_PER_BYTE),
    };
    let instance = decoder.read_struct(class, &block.data, 0)?;
    Ok((instance, decoder.unnamed))
}

/// A GTA VI enum value record, `{i64 value, u32 name hash, u32}`.
///
/// ⚠ A value above `i32::MAX` is kept as its low 32 bits: a 4-byte member stores it so.
pub(crate) fn enum_value(record: &[u8], enum_hash: u32) -> Result<RbmEnumValue> {
    let value = read_u64_le(record, 0)? as i64;
    if !(i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(&value) {
        return Err(malformed(format!(
            "a value of enum {enum_hash:#010X} is {value}, more than the 32 bits a member holds"
        )));
    }
    Ok(RbmEnumValue {
        value: value as i32,
        name_hash: read_u32_le(record, 8)?,
    })
}

/// The values, and bytes of pointed text, a tree may hold per byte of its blocks.
///
/// ⚠ Two pointers may name one target, so a tree of shared targets doubles per level and a few
/// hundred bytes state billions of values. A tree whose targets are its own has at most one value
/// per byte, and a struct in place adds one over its members' bytes.
const VALUES_PER_BYTE: usize = 4;

pub(super) fn malformed(why: String) -> MetaError {
    TableFormError::Malformed(why).into()
}

pub(super) fn class_of(model: &RbmModel, hash: u32) -> Result<&RbmClass> {
    model
        .class_for(hash)
        .ok_or_else(|| malformed(format!("no structure {hash:#010X} in LSCH")))
}

fn slice<'b>(bytes: &'b [u8], at: usize, len: usize, what: &str) -> Result<&'b [u8]> {
    at.checked_add(len)
        .and_then(|end| bytes.get(at..end))
        .ok_or_else(|| {
            malformed(format!(
                "{what} ({len} bytes at {at:#X}) runs past its section ({} bytes)",
                bytes.len()
            ))
        })
}

/// `count` records of `size` bytes at `at`, refused when `size` is 0 or they run past `bytes`.
fn table<'b>(
    bytes: &'b [u8],
    at: usize,
    size: usize,
    count: usize,
    name: &str,
) -> Result<&'b [u8]> {
    let len = size.checked_mul(count).filter(|_| size != 0);
    len.and_then(|len| bytes.get(at..at.checked_add(len)?))
        .ok_or_else(|| {
            malformed(format!(
                "{name} holds {count} records of {size} bytes at {at:#X} of {} bytes",
                bytes.len()
            ))
        })
}

/// Where the full entries of a hash table of `slots` units of `unit` bytes at `at` start, which
/// must be `count`: ungrouped, a unit not all zero; grouped, an entry of `width` bytes whose
/// control byte is below 0x80.
#[allow(clippy::too_many_arguments)]
fn full_slots(
    bytes: &[u8],
    at: usize,
    unit: usize,
    slots: usize,
    count: usize,
    grouped: bool,
    width: usize,
    name: &str,
) -> Result<Vec<usize>> {
    let stored = table(bytes, at, unit, slots, name)?;
    let mut starts = Vec::new();
    for (i, slot) in stored.chunks_exact(unit).enumerate() {
        let base = at + unit * i;
        if !grouped {
            if slot.iter().any(|&b| b != 0) {
                starts.push(base);
            }
            continue;
        }
        for (j, &control) in slot[..16].iter().enumerate() {
            match control {
                EMPTY_CONTROL => {}
                full if full < EMPTY_CONTROL => starts.push(base + 16 + width * j),
                other => {
                    return Err(malformed(format!(
                        "{name} has control byte {other:#04X}, which is neither 0x80 nor below it"
                    )))
                }
            }
        }
    }
    if starts.len() != count {
        return Err(malformed(format!(
            "{name} counts {count} pairs, where its table holds {}",
            starts.len()
        )));
    }
    Ok(starts)
}

fn align16(at: usize) -> usize {
    at.div_ceil(16) * 16
}

/// The one section order every measured table-form document has.
const SECTION_ORDER: [&[u8; 4]; 7] = [
    b"PSIN", b"LSCH", b"LENM", b"LSNS", b"LGID", b"LRSC", b"CHKS",
];

pub(super) fn model(sections: &[RawPsoSection]) -> Result<RbmModel> {
    let idents: Vec<[u8; 4]> = sections
        .iter()
        .map(|s| s.ident.ident().to_be_bytes())
        .collect();
    let in_order = idents.iter().eq(SECTION_ORDER.iter().copied());
    let ([psin, lsch, lenm, _, _, lrsc, _], true) = (sections, in_order) else {
        let spell = |idents: &mut dyn Iterator<Item = &[u8; 4]>| {
            idents
                .map(|i| String::from_utf8_lossy(i).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        };
        return Err(TableFormError::Sections {
            found: spell(&mut idents.iter()),
            measured: spell(&mut SECTION_ORDER.into_iter()),
        }
        .into());
    };
    let psin = psin.bytes.as_slice();
    let lsch = &lsch.bytes[8..];
    let lenm = &lenm.bytes[8..];
    let lrsc = &lrsc.bytes[8..];
    if lrsc.len() < 0x40 || lrsc[..4] != RBM_PRD0_MARKER_LE {
        return Err(malformed(
            "LRSC is not a 0x40-byte header opening with the PRD0 marker".into(),
        ));
    }
    let root_block_id = read_u32_le(lrsc, 0x10)?;
    let class_count = read_u16_le(lrsc, 0x38)? as usize;
    let enum_count = read_u16_le(lrsc, 0x3A)? as usize;
    let block_count = read_u16_le(lrsc, 0x3C)? as usize;

    // Offsets in PSIN count from its frame, and each block starts on 16 bytes.
    let table = slice(psin, 8, 16 * block_count, "the PSIN block table")?;
    let mut at = 8 + table.len();
    let mut data_blocks = Vec::with_capacity(block_count);
    for row in table.as_chunks::<16>().0.iter() {
        let data_type = read_u32_le(row, 0)?;
        let size = read_u32_le(row, 4)? as usize;
        at = align16(at);
        let data = slice(psin, at, size, "a PSIN block")?.to_vec();
        at += size;
        data_blocks.push(RbmDataBlock {
            data_type,
            data,
            data_addr: 0,
        });
    }

    let headers = slice(lsch, 0, 32 * class_count, "the LSCH class records")?;
    let mut at = headers.len();
    let mut classes = HashMap::with_capacity(class_count);
    let mut class_order = Vec::with_capacity(class_count);
    for header in headers.as_chunks::<32>().0.iter() {
        let (mut class, _, member_count) = class_header(header)?;
        let members = slice(lsch, at, 16 * member_count as usize, "a class's members")?;
        at += members.len();
        for member in members.as_chunks::<16>().0.iter() {
            class.entries.push(class_entry(member)?);
        }
        class_order.push(class.name_hash);
        if let Some(twice) = classes.insert(class.name_hash, class) {
            return Err(malformed(format!(
                "LSCH declares structure {:#010X} twice",
                twice.name_hash
            )));
        }
    }

    let headers = slice(lenm, 0, 24 * enum_count, "the LENM enum records")?;
    let mut at = headers.len();
    let mut enums = HashMap::with_capacity(enum_count);
    let mut enum_order = Vec::with_capacity(enum_count);
    for header in headers.as_chunks::<24>().0.iter() {
        let name_hash = read_u32_le(header, 0)?;
        let value_count = read_u32_le(header, 0x10)? as usize;
        let len = value_count.checked_mul(16).ok_or_else(|| {
            malformed(format!(
                "enum {name_hash:#010X} states {value_count} values"
            ))
        })?;
        let records = slice(lenm, at, len, "an enum's values")?;
        at += len;
        let values = records
            .as_chunks::<16>()
            .0
            .iter()
            .map(|v| enum_value(v, name_hash))
            .collect::<Result<Vec<_>>>()?;
        enum_order.push(name_hash);
        let signature = read_u32_le(header, 4)?;
        let declared = RbmEnum {
            name_hash,
            signature,
            flags: 0,
            values,
        };
        if enums.insert(name_hash, declared).is_some() {
            return Err(malformed(format!(
                "LENM declares enum {name_hash:#010X} twice"
            )));
        }
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

const NOT_MEASURED: &str = "a type code whose GTA VI layout has not been measured";

/// `raw` with [`TAG`] cleared, refused where it is clear.
fn untagged(raw: u64, name: &str) -> Result<u64> {
    match raw & TAG {
        0 => Err(malformed(format!(
            "{name}'s pointer {raw:#018X} has bit 63 clear, where every measured tagged pointer sets it"
        ))),
        _ => Ok(raw & !TAG),
    }
}

/// What the walk is inside, with what it has read of it so far.
enum Open<'m> {
    /// A structure, its members read up to `next`; `pending` names the one read below it.
    Struct {
        class: &'m RbmClass,
        data: &'m [u8],
        base: usize,
        next: usize,
        pending: String,
        instance: MetaInstance,
        place: (usize, u32),
        wrap: Wrap,
    },
    /// The elements or keys of member `entry` of `owner`, read up to `next`.
    Run {
        owner: &'m RbmClass,
        entry: &'m RbmClassEntry,
        data: &'m [u8],
        starts: Starts,
        next: usize,
        label: String,
        items: Vec<Value>,
    },
    /// A map's pair structures, read up to `next`.
    Pairs {
        pair: &'m RbmClass,
        data: &'m [u8],
        starts: Vec<usize>,
        next: usize,
        pairs: Vec<(Value, Value)>,
    },
}

impl Open<'_> {
    /// Takes `value`, which what was opened below this read as.
    fn take(&mut self, value: Value) {
        match self {
            Open::Struct {
                instance, pending, ..
            } => instance.fields.push((std::mem::take(pending), value)),
            Open::Run { items, .. } => items.push(value),
            // A pair structure's first two members are the key and the value.
            Open::Pairs { pairs, .. } => {
                if let Value::Struct(pair) = value {
                    let pair = *pair;
                    let mut fields = pair.fields.into_iter();
                    if let (Some((_, key)), Some((_, value))) = (fields.next(), fields.next()) {
                        pairs.push((key, value));
                    }
                }
            }
        }
    }
}

/// Where a run's items start: every `stride` bytes from `base`, or at the offsets listed.
enum Starts {
    Every {
        base: usize,
        stride: usize,
        count: usize,
    },
    At(Vec<usize>),
}

impl Starts {
    fn nth(&self, i: usize) -> Option<usize> {
        match self {
            Starts::Every {
                base,
                stride,
                count,
            } => (i < *count).then_some(*base + *stride * i),
            Starts::At(at) => at.get(i).copied(),
        }
    }
}

/// The pair structures `pair` at `starts` of `data`, opened as a map.
fn open_pairs<'m>(pair: &'m RbmClass, data: &'m [u8], starts: Vec<usize>) -> Step<'m> {
    Step::Open(Open::Pairs {
        pair,
        data,
        pairs: Vec::with_capacity(starts.len()),
        starts,
        next: 0,
    })
}

/// What a structure reads as in what holds it.
#[derive(Debug, Clone, Copy)]
enum Wrap {
    Struct,
    Pointer,
}

/// A member read: its value, or what the walk opens below it to read it.
enum Step<'m> {
    Value(Value),
    Open(Open<'m>),
}

/// What one step of the walk leaves.
enum Next<'m> {
    Stay,
    Open(Open<'m>),
    Done,
}

struct Decoder<'m> {
    model: &'m RbmModel,
    names: &'m DataNames,
    unnamed: Unnamed,
    /// The structures open, by address and type: one met again inside itself is refused.
    open: HashSet<(usize, u32)>,
    /// What is left of [`VALUES_PER_BYTE`] times the blocks' bytes.
    budget: usize,
}

impl<'m> Decoder<'m> {
    /// The text of a string, which ends at its first NUL or at the end of `bytes`.
    fn text(&mut self, bytes: &[u8]) -> Result<Value> {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        self.spend(end)?;
        let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
        self.unnamed.text(&text);
        Ok(Value::String(text))
    }

    fn hash32(&mut self, hash: u32, namespaced: bool) -> Value {
        let hashed = match namespaced {
            true => self.names.ns_hashed(hash),
            false => self.names.hashed(hash),
        };
        if hashed.raw.is_empty() {
            self.unnamed.narrow.insert(hash);
        }
        Value::Hash(hashed)
    }

    /// The 64-bit string hash 0x4F holds: the text it hashes where a name gives it, else the number.
    fn hash64(&mut self, hash: u64) -> Value {
        match self.names.name64(hash) {
            Some(name) => Value::String(name.to_string()),
            None => {
                self.unnamed.wide.insert(hash);
                Value::UInt(hash)
            }
        }
    }

    fn spend(&mut self, n: usize) -> Result<()> {
        self.budget = self.budget.checked_sub(n).ok_or_else(|| {
            malformed(format!(
                "the value tree holds more values than {VALUES_PER_BYTE} per byte of its blocks, \
                 as pointers that share their targets make it"
            ))
        })?;
        Ok(())
    }

    fn not_known(&self, owner: &RbmClass, member: &str, code: PsoType) -> MetaError {
        let code = code.to_byte();
        TableFormError::NotKnown {
            structure: default_hash_index().name_or_literal(owner.name_hash),
            member: member.to_string(),
            code,
            why: NOT_MEASURED,
        }
        .into()
    }

    /// The structure `class` at `base` of `data` and everything below it.
    ///
    /// ⚠ The walk keeps its own stack, but the tree it returns is dropped, cloned, compared and
    /// shown by recursion, so it nests at most [`DEPTH`] structures, which a save also recurses to.
    fn read_struct(
        &mut self,
        class: &'m RbmClass,
        data: &'m [u8],
        base: usize,
    ) -> Result<MetaInstance> {
        let mut stack = vec![self.open_struct(class, data, base, Wrap::Struct)?];
        while let Some(top) = stack.last_mut() {
            match self.advance(top)? {
                Next::Stay => {}
                Next::Open(below) => stack.push(below),
                Next::Done => {
                    let Some(done) = stack.pop() else { break };
                    let value = self.close(done);
                    match stack.last_mut() {
                        Some(holder) => holder.take(value),
                        None => {
                            return match value {
                                Value::Struct(instance) => Ok(*instance),
                                _ => Err(malformed("the walk ended on no structure".into())),
                            }
                        }
                    }
                }
            }
        }
        Err(malformed("the walk ended before its root structure".into()))
    }

    /// The structure `class` at `base` of `data`, opened; refused where it is already open.
    fn open_struct(
        &mut self,
        class: &'m RbmClass,
        data: &'m [u8],
        base: usize,
        wrap: Wrap,
    ) -> Result<Open<'m>> {
        let (at, type_hash) = ((data.as_ptr() as usize).wrapping_add(base), class.name_hash);
        let place = (at, type_hash);
        if !self.open.insert(place) {
            return Err(MetaError::Other(format!(
                "binary meta: the struct {type_hash:#X} at {at:#X} contains itself"
            )));
        }
        if self.open.len() > DEPTH {
            return Err(malformed(format!(
                "the value tree nests deeper than {DEPTH} structures, as deep as a save writes one"
            )));
        }
        Ok(Open::Struct {
            class,
            data,
            base,
            next: 0,
            pending: String::new(),
            instance: MetaInstance::new(default_hash_index().name_or_literal(class.name_hash)),
            place,
            wrap,
        })
    }

    /// One member, element or pair of `top` read, or what opens below it to read it.
    fn advance(&mut self, top: &mut Open<'m>) -> Result<Next<'m>> {
        let below = match top {
            Open::Struct {
                class,
                data,
                base,
                next,
                pending,
                instance,
                ..
            } => {
                let (class, data) = (*class, *data);
                let Some(entry) = class.entries.get(*next) else {
                    return Ok(Next::Done);
                };
                *next += 1;
                if entry.name_hash == ARRAY_INFO_NAME_HASH {
                    return Ok(Next::Stay);
                }
                let name = default_hash_index().name_or_literal(entry.name_hash);
                let off = *base + entry.offset as usize;
                match self.member(class, entry, data, off, &name)? {
                    Step::Value(value) => {
                        instance.fields.push((name, value));
                        return Ok(Next::Stay);
                    }
                    Step::Open(below) => {
                        *pending = name;
                        below
                    }
                }
            }
            Open::Run {
                owner,
                entry,
                data,
                starts,
                next,
                label,
                items,
            } => {
                let Some(at) = starts.nth(*next) else {
                    return Ok(Next::Done);
                };
                *next += 1;
                let (owner, entry, data) = (*owner, *entry, *data);
                match self.member(owner, entry, data, at, label.as_str())? {
                    Step::Value(value) => {
                        items.push(value);
                        return Ok(Next::Stay);
                    }
                    Step::Open(below) => below,
                }
            }
            Open::Pairs {
                pair,
                data,
                starts,
                next,
                ..
            } => {
                let Some(&at) = starts.get(*next) else {
                    return Ok(Next::Done);
                };
                *next += 1;
                let (pair, data) = (*pair, *data);
                self.open_struct(pair, data, at, Wrap::Struct)?
            }
        };
        Ok(Next::Open(below))
    }

    /// What `done` reads as in what holds it; a structure's place is left.
    fn close(&mut self, done: Open<'m>) -> Value {
        match done {
            Open::Struct {
                instance,
                place,
                wrap,
                ..
            } => {
                self.open.remove(&place);
                match wrap {
                    Wrap::Struct => Value::Struct(Box::new(instance)),
                    Wrap::Pointer => Value::Pointer(Some(Box::new(instance))),
                }
            }
            Open::Run { items, .. } => Value::Array(items),
            Open::Pairs { pairs, .. } => Value::Map(pairs),
        }
    }

    /// A member's value, or the structure, run or map the walk opens below it to read it.
    fn member(
        &mut self,
        owner: &'m RbmClass,
        entry: &RbmClassEntry,
        data: &'m [u8],
        off: usize,
        name: &str,
    ) -> Result<Step<'m>> {
        use PsoType::*;
        self.spend(1)?;
        Ok(match entry.data_type {
            // `{u32 capacity, u32 count}`, then `capacity` elements in place from 8 bytes rounded
            // up to the element's alignment.
            AtArray32With32BitIdx => {
                let element = self.element(owner, entry, name)?;
                let capacity = read_u32_le(data, off)?;
                let count = read_u32_le(data, off + 4)?;
                if capacity != entry.array_count() || count > capacity {
                    return Err(malformed(format!(
                        "{name} states {count} elements in room for {capacity}, where its structure declares room for {}",
                        entry.array_count()
                    )));
                }
                let first = 8usize.next_multiple_of(self.align(owner, element, name)?);
                self.elements(owner, entry, data, off + first, count as usize, name)?
            }
            Unknown(COUNTED_POINTER) => {
                let raw = read_u64_le(data, off)?;
                let count = self.count_member(owner, entry, data, off, name)?;
                self.pointed_elements(owner, entry, raw, count, name)?
            }
            Unknown(SLOT_SET) => self.hash_set(owner, entry, data, off, name, false)?,
            Unknown(GROUP_SET) => self.hash_set(owner, entry, data, off, name, true)?,
            Struct => {
                let class = class_of(self.model, entry.referent_hash())?;
                Step::Open(self.open_struct(class, data, off, Wrap::Struct)?)
            }
            Pointer64 | Unknown(TAGGED_POINTER) => {
                let mut raw = read_u64_le(data, off)?;
                if entry.data_type == Unknown(TAGGED_POINTER) {
                    raw = untagged(raw, name)?;
                }
                if raw & 0xFFF == 0 {
                    return Ok(Step::Value(Value::Pointer(None)));
                }
                let (block, at) = self.target(raw, name)?;
                let class = class_of(self.model, block.data_type)?;
                Step::Open(self.open_struct(class, &block.data, at, Wrap::Pointer)?)
            }
            ArrayMember => {
                let count = entry.array_count() as usize;
                self.elements(owner, entry, data, off, count, name)?
            }
            Unknown(COUNTED_ARRAY) => {
                let count = read_u32_le(data, off)? as usize;
                let raw = read_u64_le(data, off + 8)?;
                self.pointed_elements(owner, entry, raw, count, name)?
            }
            // The bytes from 16 to the member's end, zero in every measured one, are not read.
            Unknown(TAGGED_ARRAY) => {
                let count = read_u32_le(data, off)?;
                let capacity = read_u32_le(data, off + 4)?;
                if count != capacity {
                    return Err(malformed(format!(
                        "{name} counts {count} elements in room for {capacity}, where every measured tagged array fills its room"
                    )));
                }
                let raw = untagged(read_u64_le(data, off + 8)?, name)?;
                self.pointed_elements(owner, entry, raw, count as usize, name)?
            }
            // `{u64 pointer, u16 count, u16 capacity, u32}`.
            AtArray64 => {
                let count = read_u16_le(data, off + 8)? as usize;
                let raw = read_u64_le(data, off)?;
                self.pointed_elements(owner, entry, raw, count, name)?
            }
            // The elements in place, then a u32 count of those in use.
            AtFixedArray => {
                let element = self.element(owner, entry, name)?;
                let capacity = entry.array_count() as usize;
                let stride = self.stride(owner, element, &format!("{name}[]"))?;
                let at = stride
                    .checked_mul(capacity)
                    .and_then(|len| len.checked_add(off))
                    .ok_or_else(|| malformed(format!("{name} holds {capacity} elements")))?;
                let count = read_u32_le(data, at)? as usize;
                if count > capacity {
                    return Err(malformed(format!(
                        "{name} counts {count} elements in room for {capacity}"
                    )));
                }
                self.elements(owner, entry, data, off, count, name)?
            }
            AtBinaryMap64 => self.map(owner, entry, data, off, name)?,
            Unknown(SLOT_MAP) => self.hash_map(owner, entry, data, off, name, false)?,
            Unknown(GROUP_MAP) => self.hash_map(owner, entry, data, off, name, true)?,
            // `{u64 pointer, u32 count, u32 capacity}`.
            ArrayPointer32 => {
                let raw = read_u64_le(data, off)?;
                let count = read_u32_le(data, off + 8)? as usize;
                let capacity = read_u32_le(data, off + 12)? as usize;
                if count > capacity {
                    return Err(malformed(format!(
                        "{name} counts {count} elements in room for {capacity}"
                    )));
                }
                self.pointed_elements(owner, entry, raw, count, name)?
            }
            _ => Step::Value(self.leaf(owner, entry, data, off, name)?),
        })
    }

    /// The value of a member that holds no structure, run or map.
    fn leaf(
        &mut self,
        owner: &'m RbmClass,
        entry: &RbmClassEntry,
        data: &'m [u8],
        off: usize,
        name: &str,
    ) -> Result<Value> {
        use PsoType::*;
        let floats = |n: usize| -> Result<Vec<f32>> {
            (0..n).map(|i| read_f32_le(data, off + 4 * i)).collect()
        };
        Ok(match entry.data_type {
            Bool => Value::Bool(read_u8(data, off)? != 0),
            S8 => Value::Int(read_u8(data, off)? as i8 as i64),
            U8 => Value::UInt(read_u8(data, off)? as u64),
            S16 => Value::Int(read_i16_le(data, off)? as i64),
            U16 => Value::UInt(read_u16_le(data, off)? as u64),
            S32 => Value::Int(read_i32_le(data, off)? as i64),
            U32 => Value::UInt(read_u32_le(data, off)? as u64),
            S64 => Value::Int(read_u64_le(data, off)? as i64),
            U64 => Value::UInt(read_u64_le(data, off)?),
            Float => Value::Float(read_f32_le(data, off)? as f64),
            Double => Value::Float(f64::from_bits(read_u64_le(data, off)?)),
            Float16 => Value::Float(crate::half_to_f32(read_u16_le(data, off)?) as f64),
            Vec2 | Vec2V => {
                let v = floats(2)?;
                Value::Vec2([v[0], v[1]])
            }
            Vec3 | Vec3V => {
                let v = floats(3)?;
                Value::Vec3([v[0], v[1], v[2]])
            }
            Vec4V => {
                let v = floats(4)?;
                Value::Vec4([v[0], v[1], v[2], v[3]])
            }
            Mat33V => read_matrix(data, off, 3)?,
            Mat34V => read_matrix(data, off, 4)?,
            StringHash => self.hash32(read_u32_le(data, off)?, false),
            NsStringHash => self.hash32(read_u32_le(data, off)?, true),
            Enum8 => self.enum_value(entry, read_u8(data, off)? as i32),
            Enum16 => self.enum_value(entry, read_u16_le(data, off)? as i32),
            Enum32 => self.enum_value(entry, read_i32_le(data, off)?),
            PartialStringHash => self.hash32(read_u32_le(data, off)?, false),
            // One float written four times.
            ScalarV => {
                let words = slice(data, off, 16, name)?;
                if words.as_chunks::<4>().0.iter().any(|w| w[..] != words[..4]) {
                    return Err(malformed(format!(
                        "{name} holds four different words, where a ScalarV holds one float four times"
                    )));
                }
                Value::Float(read_f32_le(words, 0)? as f64)
            }
            Unknown(PADDED_U32) => {
                let high = read_u32_le(data, off + 4)?;
                if high != 0 {
                    return Err(malformed(format!(
                        "{name} holds {high:#X} in its high word, where every measured one holds 0"
                    )));
                }
                Value::UInt(read_u32_le(data, off)?.into())
            }
            Unknown(BYTES_16) => {
                let bytes = slice(data, off, 16, name)?;
                self.spend(bytes.len())?;
                Value::ByteArray(bytes.to_vec())
            }
            Bitset8 | Bitset16 | Bitset32 => {
                let width = match entry.data_type {
                    Bitset8 => 1,
                    Bitset16 => 2,
                    _ => 4,
                };
                // Little-endian words, word 0 lowest.
                let words = slice(data, off, bitset_len(entry, width), name)?;
                if words.len() > 16 {
                    self.spend(words.len())?;
                    return Ok(Value::ByteArray(words.to_vec()));
                }
                let mut le = [0u8; 16];
                le[..words.len()].copy_from_slice(words);
                self.bitset(entry, u128::from_le_bytes(le))
            }
            Unknown(WIDE_HASH) => Value::UInt(read_u64_le(data, off)?),
            Unknown(WIDE_NS_HASH) => self.hash64(read_u64_le(data, off)?),
            Unknown(INLINE_BITSET) => {
                let bits = entry.contents_index() as usize;
                let words = slice(data, off, bits.div_ceil(64) * 8, name)?;
                self.wide_bitset(entry, words, bits, name)?
            }
            // `{u64 pointer, u16 words, u16 bits, u32 0}`, the words u32.
            AtBitset64 => {
                let raw = read_u64_le(data, off)?;
                let words = read_u16_le(data, off + 8)? as usize;
                let bits = read_u16_le(data, off + 10)? as usize;
                let last = read_u32_le(data, off + 12)?;
                if last != 0 {
                    return Err(malformed(format!(
                        "{name} has {last:#X} in its last word, where every measured one has 0"
                    )));
                }
                if raw & 0xFFF == 0 {
                    if words != 0 || bits != 0 {
                        return Err(malformed(format!(
                            "{name} states {bits} bits in {words} words behind a null pointer"
                        )));
                    }
                    return Ok(self.bitset(entry, 0));
                }
                if words != bits.div_ceil(32) {
                    return Err(malformed(format!(
                        "{name} holds {bits} bits in {words} words"
                    )));
                }
                let (block, at) = self.target(raw, name)?;
                let stored = slice(&block.data, at, 4 * words, name)?;
                self.wide_bitset(entry, stored, bits, name)?
            }
            StringMember => self.text(slice(data, off, entry.string_count() as usize, name)?)?,
            StringPointer64 => {
                let raw = read_u64_le(data, off)?;
                if raw & 0xFFF == 0 {
                    return Ok(Value::String(String::new()));
                }
                let (block, at) = self.target(raw, name)?;
                let rest = &block.data[at..];
                if !rest.contains(&0) {
                    return Err(malformed(format!("{name}'s text has no terminating NUL")));
                }
                self.text(rest)?
            }
            // 16 bytes: the text in place when bit 7 of the last byte is set
            // (its length in bits 0 to 6), else `{u64 pointer, u16 length, u16
            // capacity, u32}`.
            AtString64 => {
                let header = slice(data, off, 16, name)?;
                if header[15] & 0x80 != 0 {
                    let len = (header[15] & 0x7F) as usize;
                    if len > 15 {
                        return Err(malformed(format!(
                            "{name} states {len} bytes of text in place, where 15 fit"
                        )));
                    }
                    return self.text(&header[..len]);
                }
                let raw = read_u64_le(header, 0)?;
                let len = read_u16_le(header, 8)? as usize;
                if raw & 0xFFF == 0 {
                    if len != 0 {
                        return Err(malformed(format!(
                            "{name} states {len} bytes of text behind a null pointer"
                        )));
                    }
                    return Ok(Value::String(String::new()));
                }
                let (block, at) = self.target(raw, name)?;
                let stored = slice(&block.data, at, len + 1, name)?;
                if stored[len] != 0 {
                    return Err(malformed(format!(
                        "{name}'s text does not end in a NUL at its stated length {len}"
                    )));
                }
                self.text(&stored[..len])?
            }
            other => return Err(self.not_known(owner, name, other)),
        })
    }

    fn pointed_elements(
        &mut self,
        owner: &'m RbmClass,
        entry: &RbmClassEntry,
        raw: u64,
        count: usize,
        name: &str,
    ) -> Result<Step<'m>> {
        if raw & 0xFFF == 0 {
            if count != 0 {
                return Err(malformed(format!(
                    "{name} counts {count} elements behind a null pointer"
                )));
            }
            return Ok(Step::Value(Value::Array(Vec::new())));
        }
        let (block, at) = self.target(raw, name)?;
        self.elements(owner, entry, &block.data, at, count, name)
    }

    /// A map's 24-byte header is `{u32, u32, u64 pointer, u16 count, u16
    /// capacity, u32}`; the pointer names a block of the map's pair structure,
    /// whose two members are the key and the value.
    fn map(
        &mut self,
        owner: &'m RbmClass,
        entry: &RbmClassEntry,
        data: &'m [u8],
        off: usize,
        name: &str,
    ) -> Result<Step<'m>> {
        let raw = read_u64_le(data, off + 8)?;
        let count = read_u16_le(data, off + 16)? as usize;
        if raw & 0xFFF == 0 {
            if count != 0 {
                return Err(malformed(format!(
                    "{name} counts {count} pairs behind a null pointer"
                )));
            }
            return Ok(Step::Value(Value::Map(Vec::new())));
        }
        let (block, at) = self.target(raw, name)?;
        let pair = class_of(self.model, block.data_type)?;
        self.check_pair(owner, entry, pair, name)?;
        let size = pair.data_size as usize;
        table(&block.data, at, size, count, name)?;
        let starts: Vec<usize> = (0..count).map(|i| at + size * i).collect();
        Ok(open_pairs(pair, &block.data, starts))
    }

    /// A hash map's 16-byte header is `{u32 slots, u32 count, u64 pointer}`.
    /// Ungrouped, the pointer names `slots` pairs, an empty one all zero;
    /// grouped, `slots` groups of 16 control bytes then 16 pairs, where 0x80
    /// marks an empty pair and a byte below it a full one.
    fn hash_map(
        &mut self,
        owner: &'m RbmClass,
        entry: &RbmClassEntry,
        data: &'m [u8],
        off: usize,
        name: &str,
        grouped: bool,
    ) -> Result<Step<'m>> {
        let slots = read_u32_le(data, off)? as usize;
        let count = read_u32_le(data, off + 4)? as usize;
        let raw = read_u64_le(data, off + 8)?;
        if raw & 0xFFF == 0 {
            if slots != 0 || count != 0 {
                return Err(malformed(format!(
                    "{name} states {slots} slots and {count} pairs behind a null pointer"
                )));
            }
            return Ok(Step::Value(Value::Map(Vec::new())));
        }
        let (block, at) = self.target(raw, name)?;
        let unit = class_of(self.model, block.data_type)?;
        let pair = if grouped {
            self.group_pair(unit, name)?
        } else {
            unit
        };
        self.check_pair(owner, entry, pair, name)?;
        let size = pair.data_size as usize;
        let unit = unit.data_size as usize;
        let starts = full_slots(&block.data, at, unit, slots, count, grouped, size, name)?;
        Ok(open_pairs(pair, &block.data, starts))
    }

    /// A hash set's header is a hash map's; its table holds keys of member `u12 & 0xFFFF`
    /// where the map's holds pairs, and it reads as the array of those keys.
    fn hash_set(
        &mut self,
        owner: &'m RbmClass,
        entry: &RbmClassEntry,
        data: &'m [u8],
        off: usize,
        name: &str,
        grouped: bool,
    ) -> Result<Step<'m>> {
        let slots = read_u32_le(data, off)? as usize;
        let count = read_u32_le(data, off + 4)? as usize;
        let raw = read_u64_le(data, off + 8)?;
        if raw & 0xFFF == 0 {
            if slots != 0 || count != 0 {
                return Err(malformed(format!(
                    "{name} states {slots} slots and {count} keys behind a null pointer"
                )));
            }
            return Ok(Step::Value(Value::Array(Vec::new())));
        }
        let key = owner.entry(entry.map_key_index()).ok_or_else(|| {
            malformed(format!(
                "{name} names key member {}, which its structure does not have",
                entry.map_key_index()
            ))
        })?;
        let width = self.stride(owner, key, &format!("{name}{{}}"))?;
        let unit = if grouped { 16 + 16 * width } else { width };
        let (block, at) = self.target(raw, name)?;
        let starts = full_slots(&block.data, at, unit, slots, count, grouped, width, name)?;
        Ok(Step::Open(Open::Run {
            owner,
            entry: key,
            data: &block.data,
            items: Vec::with_capacity(starts.len()),
            starts: Starts::At(starts),
            next: 0,
            label: name.to_string(),
        }))
    }

    /// The count a [`COUNTED_POINTER`] at `off` takes from the u8, u16 or u32 member `u12` of
    /// its structure.
    fn count_member(
        &self,
        owner: &RbmClass,
        entry: &RbmClassEntry,
        data: &[u8],
        off: usize,
        name: &str,
    ) -> Result<usize> {
        let counter = (owner.entries.get(entry.union_ch as usize))
            .filter(|c| c.name_hash != ARRAY_INFO_NAME_HASH);
        let base = off.checked_sub(entry.offset as usize);
        let (Some(counter), Some(base)) = (counter, base) else {
            return Err(malformed(format!(
                "{name} takes its count from member {}, which its structure does not have",
                entry.union_ch
            )));
        };
        let at = base + counter.offset as usize;
        Ok(match counter.data_type {
            PsoType::U8 => read_u8(data, at)? as usize,
            PsoType::U16 => read_u16_le(data, at)? as usize,
            PsoType::U32 => read_u32_le(data, at)? as usize,
            other => {
                return Err(malformed(format!(
                    "{name} takes its count from a member of type {:#04X}, where every measured one is a u8, u16 or u32",
                    other.to_byte()
                )))
            }
        })
    }

    /// The alignment of an element in place: a structure's own, 16 for a vector or matrix, else
    /// its stride.
    fn align(&self, owner: &RbmClass, element: &RbmClassEntry, name: &str) -> Result<usize> {
        use PsoType::*;
        match element.data_type {
            Struct => {
                let power = class_of(self.model, element.referent_hash())?.align_power;
                (1usize.checked_shl(power.into()))
                    .filter(|&a| a <= 4096)
                    .ok_or_else(|| {
                        malformed(format!(
                            "{name}'s elements state alignment 2^{power}, past a page"
                        ))
                    })
            }
            Vec2V | Vec3V | Vec4V | Mat33V | Mat34V | ScalarV => Ok(16),
            Vec3 => Ok(4),
            _ => self.stride(owner, element, name),
        }
    }

    /// The pair structure of a group `{u8 control[16]; pair[16]}`, which is 16 + 16 × its size.
    fn group_pair(&self, group: &'m RbmClass, name: &str) -> Result<&'m RbmClass> {
        use PsoType::*;
        let e = &group.entries;
        let array = |i: usize, offset: u32, element: u16| {
            e[i].data_type == ArrayMember
                && e[i].offset == offset
                && e[i].array_count() == 16
                && e[i].contents_index() == element
        };
        let shaped = e.len() == 4
            && [(0, U8), (2, Struct)]
                .iter()
                .all(|&(i, t)| e[i].name_hash == ARRAY_INFO_NAME_HASH && e[i].data_type == t)
            && array(1, 0, 0)
            && array(3, 16, 2);
        match shaped.then(|| self.model.class_for(e[2].referent_hash())) {
            Some(Some(pair))
                if u64::from(group.data_size) == 16 + 16 * u64::from(pair.data_size) =>
            {
                Ok(pair)
            }
            _ => Err(malformed(format!(
                "{name}'s table is a {}, which is not 16 control bytes and 16 pairs",
                default_hash_index().name_or_literal(group.name_hash)
            ))),
        }
    }

    /// Refuses a pair structure whose two members are not the map's key and value.
    fn check_pair(
        &self,
        owner: &RbmClass,
        entry: &RbmClassEntry,
        pair: &RbmClass,
        name: &str,
    ) -> Result<()> {
        let named: Vec<_> = pair
            .entries
            .iter()
            .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
            .map(|e| e.data_type)
            .collect();
        let declared = [entry.map_key_index(), entry.map_value_index()]
            .map(|i| owner.entry(i).map(|e| e.data_type));
        if named.len() != 2 || [Some(named[0]), Some(named[1])] != declared {
            return Err(malformed(format!(
                "{name}'s pairs are a {} whose members are not the map's key and value",
                default_hash_index().name_or_literal(pair.name_hash)
            )));
        }
        Ok(())
    }

    /// A bitset of `bits` bits held little-endian in `stored`.
    ///
    /// ⚠ A value holds 128 bits, so a wider set is its stored bytes, which name no enum.
    fn wide_bitset(
        &mut self,
        entry: &RbmClassEntry,
        stored: &[u8],
        bits: usize,
        name: &str,
    ) -> Result<Value> {
        let clear_past = stored.iter().enumerate().all(|(i, &byte)| {
            let first = 8 * i;
            first + 8 <= bits || byte >> bits.saturating_sub(first) == 0
        });
        if !clear_past {
            return Err(malformed(format!("{name} holds bits past its {bits}")));
        }
        if bits > 128 {
            self.spend(stored.len())?;
            return Ok(Value::ByteArray(stored.to_vec()));
        }
        let mut le = [0u8; 16];
        le[..stored.len()].copy_from_slice(stored);
        Ok(self.bitset(entry, u128::from_le_bytes(le)))
    }

    /// The block a packed pointer names and the offset in it: id in bits 0 to
    /// 11, offset in bits 20 to 63, and bits 12 to 19 clear.
    fn target(&self, raw: u64, name: &str) -> Result<(&'m RbmDataBlock, usize)> {
        if (raw >> 12) & 0xFF != 0 {
            return Err(malformed(format!(
                "{name}'s pointer {raw:#018X} sets bits 12 to 19, whose meaning is not known"
            )));
        }
        let id = (raw & 0xFFF) as i32;
        let at = usize::try_from(raw >> 20).unwrap_or(usize::MAX);
        let size = match self.model.block(id) {
            Some(block) if at <= block.data.len() => return Ok((block, at)),
            Some(block) => format!("which is {} bytes", block.data.len()),
            None => format!("of {}", self.model.data_blocks.len()),
        };
        Err(malformed(format!(
            "{name}'s pointer {raw:#X} names offset {at:#X} of block {id}, {size}"
        )))
    }

    fn element(
        &self,
        owner: &'m RbmClass,
        array: &RbmClassEntry,
        name: &str,
    ) -> Result<&'m RbmClassEntry> {
        owner.entry(array.contents_index()).ok_or_else(|| {
            malformed(format!(
                "{name} names element member {}, which its structure does not have",
                array.contents_index()
            ))
        })
    }

    fn elements(
        &mut self,
        owner: &'m RbmClass,
        array: &RbmClassEntry,
        data: &'m [u8],
        base: usize,
        count: usize,
        name: &str,
    ) -> Result<Step<'m>> {
        let element = self.element(owner, array, name)?;
        let label = format!("{name}[]");
        let stride = self.stride(owner, element, &label)?;
        let fits = stride
            .checked_mul(count)
            .and_then(|len| len.checked_add(base))
            .is_some_and(|end| end <= data.len());
        if stride == 0 || !fits {
            return Err(malformed(format!(
                "{name} holds {count} elements of {stride} bytes at {base:#X} of {} bytes",
                data.len()
            )));
        }
        Ok(Step::Open(Open::Run {
            owner,
            entry: element,
            data,
            starts: Starts::Every {
                base,
                stride,
                count,
            },
            next: 0,
            label,
            items: Vec::with_capacity(count),
        }))
    }

    fn stride(&self, owner: &RbmClass, element: &RbmClassEntry, label: &str) -> Result<usize> {
        use PsoType::*;
        if let Some(width) = fixed_width(element.data_type) {
            return Ok(width);
        }
        Ok(match element.data_type {
            Struct => class_of(self.model, element.referent_hash())?.data_size as usize,
            // `N` elements, then a u32 count, rounded up to the elements' alignment and at least 4.
            AtFixedArray => {
                let inner = (owner.entry(element.contents_index()))
                    .filter(|inner| inner.data_type != AtFixedArray)
                    .ok_or_else(|| self.not_known(owner, label, AtFixedArray))?;
                let room = element.array_count() as usize;
                let size = self.stride(owner, inner, label)?;
                let align = self.align(owner, inner, label)?.max(4);
                (size.checked_mul(room))
                    .and_then(|len| len.checked_add(4))
                    .and_then(|len| len.checked_next_multiple_of(align))
                    .ok_or_else(|| malformed(format!("{label} holds {room} elements")))?
            }
            other => return Err(self.not_known(owner, label, other)),
        })
    }

    fn enum_value(&self, entry: &RbmClassEntry, value: i32) -> Value {
        let variant = match self
            .model
            .enum_for(entry.referent_hash())
            .and_then(|e| variant_name(e, value))
        {
            Some(name_hash) => enum_text::variant_text(name_hash),
            None => enum_text::value_text(value),
        };
        Value::Enum {
            type_name: default_hash_index().name_or_literal(entry.referent_hash()),
            variant,
        }
    }

    fn bitset(&self, entry: &RbmClassEntry, bits: u128) -> Value {
        Value::Bitset {
            type_name: default_hash_index().name_or_literal(entry.bitset_enum_hash()),
            bits,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::meta::binary::{parse_meta_binary_full, write_meta_binary, BinaryFormat};
    use crate::meta::diagnostics::Diagnostics;
    use crate::meta::schema::SchemaLibrary;

    pub(crate) const ROOT: u32 = 0x1111_0001;
    pub(crate) const ELEMENT: u32 = 0x1111_0002;
    pub(crate) const KIND: u32 = 0x1111_0003;
    const KIND_VALUES: [(i32, u32); 2] = [(0, 0x3333_0000), (1, 0x3333_0001)];

    pub(crate) fn member(name: u32, offset: u32, code: u8, u10: u16, u12: u32) -> [u8; 16] {
        let mut m = [0u8; 16];
        m[0..4].copy_from_slice(&name.to_le_bytes());
        m[4..8].copy_from_slice(&offset.to_le_bytes());
        m[8] = code;
        m[10..12].copy_from_slice(&u10.to_le_bytes());
        m[12..16].copy_from_slice(&u12.to_le_bytes());
        m
    }

    pub(crate) struct Class {
        pub(crate) name: u32,
        pub(crate) size: u32,
        pub(crate) members: Vec<[u8; 16]>,
    }

    fn frame(out: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]) {
        out.extend_from_slice(ident);
        out.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(content);
    }

    fn pad_psin(psin: &mut Vec<u8>, fill: u8) {
        while !(8 + psin.len()).is_multiple_of(16) {
            psin.push(fill);
        }
    }

    /// A table-form document laid out as the measured bodies are, fill bytes included.
    pub(crate) fn body(classes: &[Class], blocks: &[(u32, Vec<u8>)], root: u32) -> Vec<u8> {
        let mut psin = Vec::new();
        for (data_type, data) in blocks {
            psin.extend_from_slice(&data_type.to_le_bytes());
            psin.extend_from_slice(&(data.len() as u32).to_le_bytes());
            psin.extend_from_slice(&[0xAA; 8]);
        }
        pad_psin(&mut psin, 0x70);
        for (_, data) in blocks {
            pad_psin(&mut psin, 0);
            psin.extend_from_slice(data);
        }
        pad_psin(&mut psin, 0);

        let mut lsch = Vec::new();
        for class in classes {
            lsch.extend_from_slice(&class.name.to_le_bytes());
            lsch.extend_from_slice(&0x5151_5151u32.to_le_bytes());
            lsch.extend_from_slice(&[0, 3, 0, 0, 0, 0, 0, 0]);
            lsch.extend_from_slice(&[0xAA; 8]);
            lsch.extend_from_slice(&class.size.to_le_bytes());
            lsch.extend_from_slice(&0u16.to_le_bytes());
            lsch.extend_from_slice(&(class.members.len() as u16).to_le_bytes());
        }
        for class in classes {
            class.members.iter().for_each(|m| lsch.extend_from_slice(m));
        }

        let mut lenm = Vec::new();
        lenm.extend_from_slice(&KIND.to_le_bytes());
        lenm.extend_from_slice(&0x6161_6161u32.to_le_bytes());
        lenm.extend_from_slice(&[0xEE; 8]);
        lenm.extend_from_slice(&(KIND_VALUES.len() as u32).to_le_bytes());
        lenm.extend_from_slice(&0u32.to_le_bytes());
        for (value, name) in KIND_VALUES {
            lenm.extend_from_slice(&value.to_le_bytes());
            lenm.extend_from_slice(&(if value < 0 { u32::MAX } else { 0 }).to_le_bytes());
            lenm.extend_from_slice(&name.to_le_bytes());
            lenm.extend_from_slice(&0u32.to_le_bytes());
        }

        let mut lrsc = vec![0u8; 0x50];
        lrsc[0..4].copy_from_slice(&RBM_PRD0_MARKER_LE);
        lrsc[4..6].copy_from_slice(&0x7Au16.to_le_bytes());
        lrsc[0x10..0x14].copy_from_slice(&root.to_le_bytes());
        lrsc[0x18..0x38].fill(0xBB);
        lrsc[0x38..0x3A].copy_from_slice(&(classes.len() as u16).to_le_bytes());
        lrsc[0x3A..0x3C].copy_from_slice(&1u16.to_le_bytes());
        lrsc[0x3C..0x3E].copy_from_slice(&(blocks.len() as u16).to_le_bytes());
        lrsc[0x40..0x48].fill(0xBB);

        let mut out = Vec::new();
        frame(&mut out, b"PSIN", &psin);
        frame(&mut out, b"LSCH", &lsch);
        frame(&mut out, b"LENM", &lenm);
        frame(&mut out, b"LSNS", &0u32.to_le_bytes());
        frame(&mut out, b"LGID", &[]);
        frame(&mut out, b"LRSC", &lrsc);
        let size = out.len() as u32 + 20;
        let mut chks = size.to_be_bytes().to_vec();
        chks.extend_from_slice(&[0, 0, 0, 0]);
        chks.extend_from_slice(b"zppp");
        frame(&mut out, b"CHKS", &chks);
        out
    }

    /// A packed pointer: block id in bits 0 to 11, offset from bit 20 up.
    pub(crate) fn pointer(block: u64, offset: u64) -> u64 {
        block | offset << 20
    }

    pub(crate) fn sample() -> Vec<u8> {
        let root = Class {
            name: ROOT,
            size: 0x50,
            members: vec![
                member(0x2222_0000, 0x00, 0x01, 0, 0),
                member(0x2222_0001, 0x02, 0x13, 0, 0),
                member(0x2222_0002, 0x04, 0x14, 0, 0),
                member(0x2222_0003, 0x08, 0x21, 0, 0),
                member(0x2222_0004, 0x0C, 0x4A, 0, 0),
                member(0x2222_0005, 0x10, 0x4D, 0, 0),
                member(0x2222_0006, 0x14, 0x60, 0, KIND),
                member(0x2222_0007, 0x20, 0x33, 0, 0),
                member(0x2222_0008, 0x30, COUNTED_ARRAY, 10, 0),
                member(0x2222_0009, 0x40, 0x07, 0, 0),
                member(ARRAY_INFO_NAME_HASH, 0, 0x05, 0, ELEMENT),
            ],
        };
        let element = Class {
            name: ELEMENT,
            size: 8,
            members: vec![
                member(0x2222_0010, 0, 0x15, 0, 0),
                member(0x2222_0011, 4, 0x21, 0, 0),
            ],
        };
        let mut data = vec![0u8; 0x50];
        data[0x00] = 1;
        data[0x02..0x04].copy_from_slice(&0xBEEFu16.to_le_bytes());
        data[0x04..0x08].copy_from_slice(&(-7i32).to_le_bytes());
        data[0x08..0x0C].copy_from_slice(&1.5f32.to_le_bytes());
        data[0x0C..0x10].copy_from_slice(&crate::string_hash(b"name").to_le_bytes());
        data[0x10..0x14].copy_from_slice(&0xCAFE_F00Du32.to_le_bytes());
        data[0x14] = 1;
        for (i, f) in [1.0f32, 2.0, 3.0, 0.0].iter().enumerate() {
            data[0x20 + 4 * i..0x24 + 4 * i].copy_from_slice(&f.to_le_bytes());
        }
        data[0x30..0x34].copy_from_slice(&2u32.to_le_bytes());
        data[0x34..0x38].copy_from_slice(&2u32.to_le_bytes());
        data[0x38..0x40].copy_from_slice(&pointer(2, 0).to_le_bytes());
        data[0x40..0x48].copy_from_slice(&pointer(3, 8).to_le_bytes());
        let elements = |pairs: &[(u32, f32)]| -> Vec<u8> {
            pairs
                .iter()
                .flat_map(|(u, f)| [u.to_le_bytes(), f.to_le_bytes()].concat())
                .collect()
        };
        body(
            &[root, element],
            &[
                (ROOT, data),
                (ELEMENT, elements(&[(10, 0.25), (20, 0.5)])),
                (ELEMENT, elements(&[(0, 0.0), (30, 0.75)])),
            ],
            1,
        )
    }

    pub(crate) fn open(bytes: &[u8]) -> Result<(MetaInstance, crate::meta::binary::MetaLayout)> {
        parse_meta_binary_full(&SchemaLibrary::default(), bytes, &mut Diagnostics::strict())
    }

    fn name(hash: u32) -> String {
        default_hash_index().name_or_literal(hash)
    }

    fn hashed(hash: u32) -> crate::meta::instance::value::HashedString {
        DataNames::of_tree().hashed(hash)
    }

    fn element(u: u64, f: f64) -> Value {
        let mut e = MetaInstance::new(name(ELEMENT));
        e.fields.push((name(0x2222_0010), Value::UInt(u)));
        e.fields.push((name(0x2222_0011), Value::Float(f)));
        Value::Struct(Box::new(e))
    }

    #[test]
    fn a_table_form_body_reads_back_to_the_values_it_was_built_with() {
        let (instance, layout) = open(&sample()).expect("it reads");
        let mut expected = MetaInstance::new(name(ROOT));
        let pointed = match element(30, 0.75) {
            Value::Struct(e) => Value::Pointer(Some(e)),
            _ => unreachable!(),
        };
        for (hash, value) in [
            (0x2222_0000, Value::Bool(true)),
            (0x2222_0001, Value::UInt(0xBEEF)),
            (0x2222_0002, Value::Int(-7)),
            (0x2222_0003, Value::Float(1.5)),
            (
                0x2222_0004,
                Value::Hash(hashed(crate::string_hash(b"name"))),
            ),
            (
                0x2222_0005,
                Value::Hash(DataNames::of_tree().ns_hashed(0xCAFE_F00D)),
            ),
            (
                0x2222_0006,
                Value::Enum {
                    type_name: name(KIND),
                    variant: enum_text::variant_text(KIND_VALUES[1].1),
                },
            ),
            (0x2222_0007, Value::Vec3([1.0, 2.0, 3.0])),
            (
                0x2222_0008,
                Value::Array(vec![element(10, 0.25), element(20, 0.5)]),
            ),
            (0x2222_0009, pointed),
        ] {
            expected.fields.push((name(hash), value));
        }
        assert_eq!(instance, expected);
        assert_eq!(layout.format, BinaryFormat::Pso);
        let crate::meta::binary::ContainerLayout::Pso(pso) = &layout.container else {
            panic!("a PSO layout");
        };
        assert!(pso.is_table_form());
    }

    #[test]
    fn a_table_form_document_the_layout_rules_do_not_give_back_refuses_to_save() {
        // Two blocks of one type, where the rules pack one.
        let bytes = sample();
        let (instance, layout) = open(&bytes).expect("it reads");
        let err = write_meta_binary(BinaryFormat::Pso, Some(&layout), &instance)
            .expect_err("refused")
            .to_string();
        let want = match cfg!(feature = "gta6") {
            true => "the measured layout rules do not give back the bytes it was read from",
            false => "has no GTA VI support to save it",
        };
        assert!(err.contains(want), "{err}");

        let doc =
            crate::meta::instance::MetaDocument::parse(&SchemaLibrary::default(), bytes.clone())
                .expect("it reads");
        assert!(doc.to_bytes().is_err());

        let err = crate::meta::canonical::converge(&SchemaLibrary::default(), &bytes)
            .expect_err("no canonical form");
        assert!(err.to_string().contains(want), "{err}");
    }

    fn lenm_value_at(bytes: &[u8], value: usize) -> usize {
        let lenm = bytes.windows(4).position(|w| w == b"LENM").expect("LENM") + 8;
        lenm + 24 + 16 * value
    }

    #[test]
    fn an_enum_value_is_read_as_64_bits_and_refused_past_the_32_a_member_holds() {
        let mut bytes = sample();
        let at = lenm_value_at(&bytes, 1);
        bytes[at..at + 8].copy_from_slice(&0xB53A_C261u64.to_le_bytes());
        open(&bytes).expect("a value in 2^31..2^32 reads");
        bytes[at + 4] = 1;
        let err = open(&bytes).expect_err("refused").to_string();
        assert!(
            err.contains(&format!(
                "a value of enum {KIND:#010X} is {}, more than the 32 bits a member holds",
                0x1_B53A_C261i64
            )),
            "{err}"
        );
    }

    #[test]
    fn a_hash_value_is_named_by_a_lower_cased_tree_name_or_a_string_of_the_document() {
        // A tree name of mixed case, whose literal hash is not its data hash.
        let mixed = "CMapData";
        assert!(default_hash_index().names().any(|n| n == mixed));
        let data_hash = crate::string_hash(mixed.as_bytes());
        assert_ne!(data_hash, crate::literal_string_hash(mixed.as_bytes()));
        let heaps = crate::string_hash(b"heaps");
        let bytes = arrays_maps_and_strings(|_, b| {
            b[2].1[8..12].copy_from_slice(&data_hash.to_le_bytes());
            b[2].1[16..20].copy_from_slice(&heaps.to_le_bytes());
        });
        let (instance, _) = open(&bytes).expect("it reads");
        let Some(Value::Map(pairs)) = instance.get(&name(0x3333_0004)) else {
            panic!("a map");
        };
        let named = |raw: &str, hash: u32| {
            Value::Hash(crate::meta::instance::value::HashedString {
                raw: raw.into(),
                hash,
            })
        };
        assert_eq!(pairs[0].0, named(mixed, data_hash));
        assert_eq!(pairs[1].0, named("heaps", heaps));

        // A namespaced hash may hash the text as written; a plain one does not.
        let literal = crate::literal_string_hash(mixed.as_bytes());
        let class = Class {
            name: ROOT,
            size: 8,
            members: vec![
                member(0x2222_0000, 0, 0x4A, 0, 0),
                member(0x2222_0001, 4, 0x4D, 0, 0),
            ],
        };
        let data = [literal.to_le_bytes(), literal.to_le_bytes()].concat();
        let (instance, _) = open(&body(&[class], &[(ROOT, data)], 1)).expect("it reads");
        assert_eq!(instance.fields[1].1, named(mixed, literal));
        assert_ne!(instance.fields[0].1, named(mixed, literal));
    }

    #[cfg(feature = "gta6")]
    #[test]
    fn a_64_bit_string_hash_is_named_by_the_text_it_hashes() {
        let mixed = "CMapData";
        let wide = crate::gta6::hash::name_hash64(mixed.as_bytes());
        let bytes = hash_maps_and_wide_values(|_, b| {
            b[0].1[0x08..0x10].copy_from_slice(&wide.to_le_bytes())
        });
        let (instance, _) = open(&bytes).expect("it reads");
        assert_eq!(
            instance.get(&name(0x4444_0001)),
            Some(&Value::String(mixed.into()))
        );
        assert_eq!(
            instance.get(&name(0x4444_0000)),
            Some(&Value::UInt(0x0123_4567_89AB_CDEF)),
            "0x08 is no string hash"
        );
    }

    #[test]
    fn a_bitset_wider_than_128_bits_reads_as_its_stored_bytes() {
        let wide = one_member(INLINE_BITSET, 129);
        let (instance, _) = open(&wide).expect("it reads");
        let mut words = vec![0u8; 24];
        words[0] = 1;
        words[8] = 2;
        assert_eq!(
            instance.get(&name(0x2222_0000)),
            Some(&Value::ByteArray(words))
        );

        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|v| v.to_le_bytes()).collect() };
        let bytes = hash_maps_and_wide_values(|_, b| {
            b[0].1[0x48..0x4C].copy_from_slice(&[5, 0, 129, 0]);
            b[3].1 = words(&[0xDEAD, 0xFFFF_FFFF, 1, 0x20, 0, 1]);
        });
        let (instance, _) = open(&bytes).expect("it reads");
        assert_eq!(
            instance.get(&name(0x4444_0009)),
            Some(&Value::ByteArray(words(&[0xFFFF_FFFF, 1, 0x20, 0, 1])))
        );
        let bytes = hash_maps_and_wide_values(|_, b| {
            b[0].1[0x48..0x4C].copy_from_slice(&[5, 0, 129, 0]);
            b[3].1 = words(&[0xDEAD, 0xFFFF_FFFF, 1, 0x20, 0, 2]);
        });
        let err = open(&bytes).expect_err("refused").to_string();
        assert!(err.contains("holds bits past its 129"), "{err}");
    }

    fn one_member(code: u8, u10: u16) -> Vec<u8> {
        let root = Class {
            name: ROOT,
            size: 0x20,
            members: vec![
                member(0x2222_0000, 0, code, u10, 0),
                member(ARRAY_INFO_NAME_HASH, 0, 0x32, 0, 0),
            ],
        };
        let mut data = vec![0u8; 0x20];
        data[0..4].copy_from_slice(&1u32.to_le_bytes());
        data[8..16].copy_from_slice(&pointer(2, 0).to_le_bytes());
        body(&[root], &[(ROOT, data), (0x32, vec![0; 16])], 1)
    }

    #[test]
    fn a_type_code_whose_layout_is_not_known_is_refused_by_name() {
        let err = open(&one_member(0x0E, 0)).expect_err("refused").to_string();
        let want = format!(
            "member {} of {} has type 0x0E, {NOT_MEASURED}",
            name(0x2222_0000),
            name(ROOT),
        );
        assert!(err.contains(&want), "{err}");
    }

    const GROUP_KEYS: u32 = 0x1111_0013;
    const SET_KEYS: [u32; 4] = [0xAAAA_0003, 0xBBBB_0011, 0x0000_0105, 0x0000_0186];

    /// A body with each member code measured after the first readers, edited by `patch` before
    /// it is laid out.
    pub(crate) fn measured_codes(
        patch: impl FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>),
    ) -> Vec<u8> {
        let anon = |code: u8| member(ARRAY_INFO_NAME_HASH, 0, code, 0, 0);
        let root = Class {
            name: ROOT,
            size: 0x78,
            members: vec![
                member(0x5555_0000, 0x00, 0x61, 0, KIND),
                member(0x5555_0001, 0x04, 0x4B, 0, 0),
                member(0x5555_0002, 0x08, PADDED_U32, 0, 0),
                member(0x5555_0003, 0x10, BYTES_16, 0, 0),
                member(0x5555_0004, 0x20, 0x22, 0, 0),
                member(0x5555_0005, 0x30, 0x54, 6, 3),
                anon(0x15),
                member(0x5555_0007, 0x48, COUNTED_POINTER, 8, 9),
                anon(0x21),
                member(0x5555_0009, 0x50, 0x13, 0, 0),
                member(0x5555_000A, 0x58, SLOT_SET, 0, 11),
                anon(0x4A),
                member(0x5555_000C, 0x68, GROUP_SET, 0, 11),
            ],
        };
        let group = Class {
            name: GROUP_KEYS,
            size: 16 + 16 * 4,
            members: vec![
                anon(0x11),
                member(0xB879_4C89, 0, 0x50, 0, 16),
                anon(0x4A),
                member(0x93C4_95AF, 16, 0x50, 2, 16),
            ],
        };
        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|v| v.to_le_bytes()).collect() };
        let mut data = vec![0u8; 0x78];
        let mut put = |at: usize, bytes: &[u8]| data[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x00, &1u16.to_le_bytes());
        put(0x04, &0xEA07_B3D3u32.to_le_bytes());
        put(0x08, &0x1234_5678u32.to_le_bytes());
        put(0x10, &(0..16).collect::<Vec<u8>>());
        put(0x20, &(-27.0f32).to_le_bytes().repeat(4));
        put(0x30, &words(&[3, 2, 7, 9, 0]));
        put(0x48, &pointer(2, 0).to_le_bytes());
        put(0x50, &2u16.to_le_bytes());
        put(0x58, &words(&[32, 2]));
        put(0x60, &pointer(3, 0).to_le_bytes());
        put(0x68, &words(&[2, 2]));
        put(0x70, &pointer(4, 0).to_le_bytes());

        // A key's slot is its hash modulo the slots; a group's control byte is its low 7 bits.
        let mut slots = vec![0u32; 32];
        slots[(SET_KEYS[0] & 31) as usize] = SET_KEYS[0];
        slots[(SET_KEYS[1] & 31) as usize] = SET_KEYS[1];
        let mut groups = Vec::new();
        for key in &SET_KEYS[2..] {
            let mut g = vec![EMPTY_CONTROL; 16];
            g[0] = (key & 0x7F) as u8;
            g.extend(words(&[*key]));
            g.resize(16 + 16 * 4, 0);
            groups.extend(g);
        }
        let mut classes = vec![root, group];
        let mut blocks = vec![
            (ROOT, data),
            (
                0x21,
                [0.5f32.to_le_bytes(), (-2.0f32).to_le_bytes()].concat(),
            ),
            (0x4A, words(&slots)),
            (GROUP_KEYS, groups),
        ];
        patch(&mut classes, &mut blocks);
        body(&classes, &blocks, 1)
    }

    #[test]
    fn the_member_codes_measured_since_read_back_to_the_values_they_were_built_with() {
        let (instance, _) = open(&measured_codes(|_, _| {})).expect("it reads");
        let key = |h: u32| Value::Hash(hashed(h));
        let mut expected = MetaInstance::new(name(ROOT));
        for (hash, value) in [
            (
                0x5555_0000,
                Value::Enum {
                    type_name: name(KIND),
                    variant: enum_text::variant_text(KIND_VALUES[1].1),
                },
            ),
            (0x5555_0001, key(0xEA07_B3D3)),
            (0x5555_0002, Value::UInt(0x1234_5678)),
            (0x5555_0003, Value::ByteArray((0..16).collect())),
            (0x5555_0004, Value::Float(-27.0)),
            (
                0x5555_0005,
                Value::Array(vec![Value::UInt(7), Value::UInt(9)]),
            ),
            (
                0x5555_0007,
                Value::Array(vec![Value::Float(0.5), Value::Float(-2.0)]),
            ),
            (0x5555_0009, Value::UInt(2)),
            (
                0x5555_000A,
                Value::Array(vec![key(SET_KEYS[0]), key(SET_KEYS[1])]),
            ),
            (
                0x5555_000C,
                Value::Array(vec![key(SET_KEYS[2]), key(SET_KEYS[3])]),
            ),
        ] {
            expected.fields.push((name(hash), value));
        }
        assert_eq!(instance, expected);
    }

    #[test]
    fn a_measured_code_that_breaks_its_measured_relation_is_refused() {
        type Patch = Box<dyn FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>)>;
        let root = |b: &mut Vec<(u32, Vec<u8>)>, at: usize, bytes: &[u8]| {
            b[0].1[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let cases: Vec<(Patch, &str)> = vec![
            (
                Box::new(move |_, b| root(b, 0x0C, &[1])),
                "holds 0x1 in its high word, where every measured one holds 0",
            ),
            (
                Box::new(move |_, b| root(b, 0x24, &[1])),
                "holds four different words, where a ScalarV holds one float four times",
            ),
            (
                Box::new(move |_, b| root(b, 0x30, &[4])),
                "states 2 elements in room for 4, where its structure declares room for 3",
            ),
            (
                Box::new(move |_, b| root(b, 0x34, &[4])),
                "states 4 elements in room for 3",
            ),
            (
                Box::new(|c, _| c[0].members[9] = member(0x5555_0009, 0x50, 0x21, 0, 0)),
                "takes its count from a member of type 0x21",
            ),
            (
                Box::new(move |_, b| root(b, 0x48, &[0; 8])),
                "counts 2 elements behind a null pointer",
            ),
            (
                Box::new(move |_, b| root(b, 0x5C, &[3])),
                "counts 3 pairs, where its table holds 2",
            ),
            (
                Box::new(move |_, b| root(b, 0x70, &[0; 8])),
                "states 2 slots and 2 keys behind a null pointer",
            ),
        ];
        for (patch, want) in cases {
            let err = open(&measured_codes(patch)).expect_err(want).to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    const OTHER: u32 = 0x1111_0004;

    /// A body with tagged arrays and pointers, an array of Vec3, an array of fixed arrays of
    /// doubles and a double in place, edited by `patch` before it is laid out.
    pub(crate) fn leftover_codes(
        patch: impl FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>),
    ) -> Vec<u8> {
        let anon = |code: u8, u10: u16, u12: u32| member(ARRAY_INFO_NAME_HASH, 0, code, u10, u12);
        let root = Class {
            name: ROOT,
            size: 0x60,
            members: vec![
                member(0x7777_0000, 0x00, TAGGED_ARRAY, 1, 0),
                anon(TAGGED_POINTER, 0, 0),
                member(0x7777_0002, 0x18, TAGGED_POINTER, 0, 0),
                member(0x7777_0003, 0x20, COUNTED_ARRAY, 4, 0),
                anon(0x32, 0, 0),
                member(0x7777_0005, 0x30, COUNTED_ARRAY, 6, 0),
                anon(0x53, 7, 3),
                anon(0x23, 0, 0),
                member(0x7777_0008, 0x40, 0x23, 0, 0),
                member(0x7777_0009, 0x48, TAGGED_POINTER, 0, 0),
                member(0x7777_000A, 0x50, TAGGED_ARRAY, 1, 0),
            ],
        };
        let element = Class {
            name: ELEMENT,
            size: 8,
            members: vec![
                member(0x2222_0010, 0, 0x15, 0, 0),
                member(0x2222_0011, 4, 0x21, 0, 0),
            ],
        };
        let other = Class {
            name: OTHER,
            size: 4,
            members: vec![member(0x7777_0020, 0, 0x14, 0, 0)],
        };
        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|v| v.to_le_bytes()).collect() };
        let tagged = |block: u64, offset: u64| (pointer(block, offset) | TAG).to_le_bytes();
        let mut data = vec![0u8; 0x60];
        let mut put = |at: usize, bytes: &[u8]| data[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x00, &words(&[3, 3]));
        put(0x08, &tagged(2, 0));
        put(0x18, &tagged(3, 8));
        put(0x20, &words(&[2, 2]));
        put(0x28, &pointer(5, 0).to_le_bytes());
        put(0x30, &words(&[2, 2]));
        put(0x38, &pointer(6, 0).to_le_bytes());
        put(0x40, &1.0e300f64.to_le_bytes());
        put(0x48, &TAG.to_le_bytes());
        put(0x58, &TAG.to_le_bytes());

        let doubles = |d: &[f64]| -> Vec<u8> { d.iter().flat_map(|v| v.to_le_bytes()).collect() };
        // Each fixed array is 3 doubles and a count, 28 bytes rounded up to the doubles' 8.
        let mut fixed = vec![0u8; 64];
        fixed[..16].copy_from_slice(&doubles(&[0.1, -2.5]));
        fixed[24..28].copy_from_slice(&2u32.to_le_bytes());
        fixed[32..56].copy_from_slice(&doubles(&[-1.0e-300, 0.5, 7.25]));
        fixed[56..60].copy_from_slice(&3u32.to_le_bytes());
        let mut classes = vec![root, element, other];
        let mut blocks = vec![
            (ROOT, data),
            (
                u32::from(TAGGED_POINTER),
                [tagged(3, 0), tagged(4, 0), tagged(3, 8)].concat(),
            ),
            (
                ELEMENT,
                words(&[10, 0.25f32.to_bits(), 20, 0.5f32.to_bits()]),
            ),
            (OTHER, (-9i32).to_le_bytes().to_vec()),
            (
                0x32,
                words(&[1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0].map(f32::to_bits)),
            ),
            (0x53, fixed),
        ];
        patch(&mut classes, &mut blocks);
        body(&classes, &blocks, 1)
    }

    #[test]
    fn the_leftover_codes_read_back_to_the_values_they_were_built_with() {
        let (instance, _) = open(&leftover_codes(|_, _| {})).expect("it reads");
        let pointed = |v: Value| match v {
            Value::Struct(e) => Value::Pointer(Some(e)),
            other => other,
        };
        let mut other = MetaInstance::new(name(OTHER));
        other.fields.push((name(0x7777_0020), Value::Int(-9)));
        let floats = |f: &[f64]| Value::Array(f.iter().map(|&v| Value::Float(v)).collect());
        let mut expected = MetaInstance::new(name(ROOT));
        for (hash, value) in [
            (
                0x7777_0000,
                Value::Array(vec![
                    pointed(element(10, 0.25)),
                    Value::Pointer(Some(Box::new(other))),
                    pointed(element(20, 0.5)),
                ]),
            ),
            (0x7777_0002, pointed(element(20, 0.5))),
            (
                0x7777_0003,
                Value::Array(vec![
                    Value::Vec3([1.0, 2.0, 3.0]),
                    Value::Vec3([4.0, 5.0, 6.0]),
                ]),
            ),
            (
                0x7777_0005,
                Value::Array(vec![floats(&[0.1, -2.5]), floats(&[-1.0e-300, 0.5, 7.25])]),
            ),
            (0x7777_0008, Value::Float(1.0e300)),
            (0x7777_0009, Value::Pointer(None)),
            (0x7777_000A, Value::Array(Vec::new())),
        ] {
            expected.fields.push((name(hash), value));
        }
        assert_eq!(instance, expected);
    }

    #[test]
    fn a_leftover_code_that_breaks_its_measured_relation_is_refused() {
        type Patch = Box<dyn FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>)>;
        let clear = "has bit 63 clear, where every measured tagged pointer sets it";
        let cases: Vec<(Patch, &str)> = vec![
            (Box::new(|_, b| b[0].1[0x0F] = 0), clear),
            (Box::new(|_, b| b[0].1[0x1F] = 0), clear),
            (Box::new(|_, b| b[0].1[0x4F] = 0), clear),
            (Box::new(|_, b| b[1].1[15] = 0), clear),
            (
                Box::new(|_, b| b[0].1[0x04] = 4),
                "counts 3 elements in room for 4, where every measured tagged array fills its room",
            ),
            // A Vec3 array strides 12 bytes: 16 would need 32 of the 24 the block holds.
            (
                Box::new(|_, b| b[0].1[0x20] = 3),
                "holds 3 elements of 12 bytes",
            ),
            // A fixed array of doubles strides 32 bytes, not the bare 28.
            (
                Box::new(|_, b| b[5].1.truncate(60)),
                "holds 2 elements of 32 bytes",
            ),
            (
                Box::new(|c, _| c[0].members[7] = member(ARRAY_INFO_NAME_HASH, 0, 0x53, 7, 3)),
                "has type 0x53",
            ),
        ];
        for (patch, want) in cases {
            let err = open(&leftover_codes(patch)).expect_err(want).to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn a_body_that_breaks_a_measured_relation_is_refused_with_the_relation() {
        // The root block is the first, at 64 bytes into the file: an 8-byte
        // frame and three 16-byte table rows, rounded up to 16.
        let root = 64;
        let at = move |field: usize| root + field;
        let set = |bytes: &mut Vec<u8>, at: usize, word: &[u8]| {
            bytes[at..at + word.len()].copy_from_slice(word);
        };
        type Patch = Box<dyn Fn(&mut Vec<u8>)>;
        let cases: Vec<(Patch, &str)> = vec![
            (
                Box::new(move |b| set(b, at(0x38), &pointer(2, 0x100).to_le_bytes())),
                "names offset 0x100 of block 2, which is 16 bytes",
            ),
            (
                Box::new(move |b| set(b, at(0x40), &pointer(9, 0).to_le_bytes())),
                "names offset 0x0 of block 9, of 3",
            ),
            (
                Box::new(move |b| set(b, at(0x40), &(pointer(3, 0) | 1 << 40).to_le_bytes())),
                "names offset 0x100000 of block 3",
            ),
            (
                Box::new(move |b| set(b, at(0x40), &(pointer(3, 0) | 1 << 12).to_le_bytes())),
                "sets bits 12 to 19",
            ),
            (
                Box::new(move |b| set(b, at(0x38), &0u64.to_le_bytes())),
                "counts 2 elements behind a null pointer",
            ),
            (
                Box::new(move |b| set(b, at(0x30), &u32::MAX.to_le_bytes())),
                "elements of 8 bytes",
            ),
            (
                Box::new(|b| {
                    let lrsc = b.windows(4).position(|w| w == b"LRSC").expect("LRSC") + 8;
                    b[lrsc] = b'X';
                }),
                "PRD0 marker",
            ),
            (
                Box::new(|b| {
                    let lrsc = b.windows(4).position(|w| w == b"LRSC").expect("LRSC") + 8;
                    b[lrsc + 0x10..lrsc + 0x14].copy_from_slice(&7u32.to_le_bytes());
                }),
                "the root names block 7 of 3",
            ),
            (
                Box::new(|b| {
                    let lgid = b.windows(4).position(|w| w == b"LGID").expect("LGID");
                    b[lgid..lgid + 4].copy_from_slice(b"LGIX");
                }),
                "where every measured document has PSIN LSCH LENM LSNS LGID LRSC CHKS",
            ),
        ];
        for (patch, want) in cases {
            let mut bytes = sample();
            patch(&mut bytes);
            let err = open(&bytes).expect_err(want).to_string();
            assert!(err.contains(want), "{want}: {err}");
        }

        let twice = || Class {
            name: ROOT,
            size: 4,
            members: vec![member(0x2222_0000, 0, 0x15, 0, 0)],
        };
        let err = open(&body(&[twice(), twice()], &[(ROOT, vec![0; 4])], 1))
            .expect_err("refused")
            .to_string();
        assert!(err.contains("declares structure 0x11110001 twice"), "{err}");
    }

    #[test]
    fn every_truncation_is_an_error_and_never_a_panic() {
        for bytes in [
            sample(),
            arrays_maps_and_strings(|_, _| {}),
            hash_maps_and_wide_values(|_, _| {}),
            measured_codes(|_, _| {}),
            leftover_codes(|_, _| {}),
        ] {
            for len in 0..bytes.len() {
                assert!(open(&bytes[..len]).is_err(), "{len} bytes read");
            }
        }
    }

    const PAIR: u32 = 0x1111_0011;
    const FAR: usize = 0x1004;

    /// A body with a pointed and an inline array, a map and each string layout, edited by `patch`
    /// before it is laid out.
    pub(crate) fn arrays_maps_and_strings(
        patch: impl FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>),
    ) -> Vec<u8> {
        let anon = |code: u8| member(ARRAY_INFO_NAME_HASH, 0, code, 0, 0);
        let root = Class {
            name: ROOT,
            size: 0x68,
            members: vec![
                member(0x3333_0000, 0x00, 0x52, 1, 0),
                anon(0x15),
                member(0x3333_0002, 0x10, 0x53, 3, 3),
                anon(0x21),
                member(0x3333_0004, 0x20, 0x71, 0, 6 << 16 | 5),
                anon(0x4A),
                anon(0x15),
                member(0x3333_0007, 0x38, 0x40, 0, 8),
                member(0x3333_0008, 0x40, 0x42, 0, 0),
                member(0x3333_0009, 0x48, 0x44, 0, 0),
                member(0x3333_000A, 0x58, 0x44, 0, 0),
            ],
        };
        let pair = Class {
            name: PAIR,
            size: 8,
            members: vec![
                member(0x3333_0100, 0, 0x4A, 0, 0),
                member(0x3333_0101, 4, 0x15, 0, 0),
            ],
        };
        let mut data = vec![0u8; 0x68];
        let mut put = |at: usize, bytes: &[u8]| data[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x00, &pointer(2, 0).to_le_bytes());
        put(0x08, &[3, 0, 3, 0]);
        put(0x10, &[1.0f32.to_le_bytes(), 2.0f32.to_le_bytes()].concat());
        put(0x1C, &2u32.to_le_bytes());
        put(0x20, &1u32.to_le_bytes());
        put(0x28, &pointer(3, 8).to_le_bytes());
        put(0x30, &[2, 0, 2, 0]);
        put(0x38, b"abc");
        put(0x40, &pointer(4, FAR as u64).to_le_bytes());
        put(0x48, b"inline");
        put(0x57, &[0x80 | 6]);
        put(0x58, &pointer(4, 0).to_le_bytes());
        put(0x60, &[5, 0, 6, 0, 1, 0, 0, 0]);

        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|v| v.to_le_bytes()).collect() };
        let mut text = vec![0u8; FAR + 12];
        text[..6].copy_from_slice(b"heaps\0");
        text[FAR..FAR + 4].copy_from_slice(b"far\0");
        let mut classes = vec![root, pair];
        let mut blocks = vec![
            (ROOT, data),
            (0x15, words(&[10, 20, 30])),
            (
                PAIR,
                words(&[0xDEAD, 0xBEEF, 0xAAAA_0001, 5, 0xAAAA_0002, 6]),
            ),
            (0x10, text),
        ];
        patch(&mut classes, &mut blocks);
        body(&classes, &blocks, 1)
    }

    #[test]
    fn arrays_maps_and_strings_read_back_to_the_values_they_were_built_with() {
        let (instance, _) = open(&arrays_maps_and_strings(|_, _| {})).expect("it reads");
        let uints = |v: &[u64]| Value::Array(v.iter().map(|&u| Value::UInt(u)).collect());
        let key = |h: u32| Value::Hash(hashed(h));
        let mut expected = MetaInstance::new(name(ROOT));
        for (hash, value) in [
            (0x3333_0000, uints(&[10, 20, 30])),
            (
                0x3333_0002,
                Value::Array(vec![Value::Float(1.0), Value::Float(2.0)]),
            ),
            (
                0x3333_0004,
                Value::Map(vec![
                    (key(0xAAAA_0001), Value::UInt(5)),
                    (key(0xAAAA_0002), Value::UInt(6)),
                ]),
            ),
            (0x3333_0007, Value::String("abc".into())),
            (0x3333_0008, Value::String("far".into())),
            (0x3333_0009, Value::String("inline".into())),
            (0x3333_000A, Value::String("heaps".into())),
        ] {
            expected.fields.push((name(hash), value));
        }
        assert_eq!(instance, expected);
        // The far string's offset needs the pointer's high word.
        assert!(pointer(4, FAR as u64) >> 32 != 0);
    }

    #[test]
    fn an_array_map_or_string_that_breaks_its_measured_relation_is_refused() {
        type Patch = Box<dyn FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>)>;
        let root = |b: &mut Vec<(u32, Vec<u8>)>, at: usize, bytes: &[u8]| {
            b[0].1[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let cases: Vec<(Patch, &str)> = vec![
            (
                Box::new(|c, _| c[1].members[0] = member(0x3333_0100, 0, 0x15, 0, 0)),
                "pairs are a hash_11110011 whose members are not the map's key and value",
            ),
            (
                Box::new(move |_, b| root(b, 0x28, &0u64.to_le_bytes())),
                "counts 2 pairs behind a null pointer",
            ),
            (
                Box::new(move |_, b| root(b, 0x1C, &4u32.to_le_bytes())),
                "counts 4 elements in room for 3",
            ),
            (
                Box::new(move |_, b| root(b, 0x57, &[0x80 | 16])),
                "states 16 bytes of text in place, where 15 fit",
            ),
            (
                Box::new(|_, b| b[3].1[5] = b'x'),
                "does not end in a NUL at its stated length 5",
            ),
            (
                Box::new(|_, b| b[3].1[FAR + 3..].fill(b'x')),
                "text has no terminating NUL",
            ),
            (
                Box::new(move |_, b| root(b, 0x58, &[0; 8])),
                "states 5 bytes of text behind a null pointer",
            ),
        ];
        for (patch, want) in cases {
            let err = open(&arrays_maps_and_strings(patch))
                .expect_err(want)
                .to_string();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    const GROUP: u32 = 0x1111_0012;
    const KEY_1: u32 = 0xAAAA_0005;
    const KEY_2: u32 = 0xAAAA_0106;

    /// A body with both hash maps, 64-bit hashes and wide bitsets, edited by `patch` before it is
    /// laid out.
    pub(crate) fn hash_maps_and_wide_values(
        patch: impl FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>),
    ) -> Vec<u8> {
        let anon = |code: u8| member(ARRAY_INFO_NAME_HASH, 0, code, 0, 0);
        let root = Class {
            name: ROOT,
            size: 0x70,
            members: vec![
                member(0x4444_0000, 0x00, 0x08, 0, 0),
                member(0x4444_0001, 0x08, 0x4F, 0, 0),
                member(0x4444_0002, 0x10, 0x73, 0, 4 << 16 | 3),
                anon(0x4A),
                anon(0x15),
                member(0x4444_0005, 0x20, 0x75, 0, 7 << 16 | 6),
                anon(0x4A),
                anon(0x15),
                member(0x4444_0008, 0x30, 0x68, 70, KIND),
                member(0x4444_0009, 0x40, 0x67, 0, KIND),
                member(0x4444_000A, 0x50, 0x56, 11, 0),
                anon(0x11),
                member(0x4444_000C, 0x60, COUNTED_ARRAY, 13, 0),
                anon(0x08),
            ],
        };
        let pair = Class {
            name: PAIR,
            size: 8,
            members: vec![
                member(0x6098_A50E, 0, 0x4A, 0, 0),
                member(0x063F_A3F2, 4, 0x15, 0, 0),
            ],
        };
        let group = Class {
            name: GROUP,
            size: 16 + 16 * 8,
            members: vec![
                anon(0x11),
                member(0xB879_4C89, 0, 0x50, 0, 16),
                member(ARRAY_INFO_NAME_HASH, 0, 0x05, 0, PAIR),
                member(0x93C4_95AF, 16, 0x50, 2, 16),
            ],
        };
        let mut data = vec![0u8; 0x70];
        let mut put = |at: usize, bytes: &[u8]| data[at..at + bytes.len()].copy_from_slice(bytes);
        put(0x00, &0x0123_4567_89AB_CDEFu64.to_le_bytes());
        put(0x08, &0xFEDC_BA98_7654_3210u64.to_le_bytes());
        put(0x10, &[4, 0, 0, 0, 2, 0, 0, 0]);
        put(0x18, &pointer(2, 0).to_le_bytes());
        put(0x20, &[1, 0, 0, 0, 2, 0, 0, 0]);
        put(0x28, &pointer(3, 0).to_le_bytes());
        put(
            0x30,
            &[1u64.to_le_bytes(), (1u64 << 5).to_le_bytes()].concat(),
        );
        put(0x40, &pointer(4, 4).to_le_bytes());
        put(0x48, &[3, 0, 70, 0]);
        put(0x50, &pointer(5, 0).to_le_bytes());
        put(0x58, &[3, 0, 0, 0, 3, 0, 0, 0]);
        put(0x60, &[2, 0, 0, 0, 2, 0, 0, 0]);
        put(0x68, &pointer(6, 0).to_le_bytes());

        let words = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|v| v.to_le_bytes()).collect() };
        // Slot by slot; an empty one is all zero.
        let slots = words(&[0, 0, 0xAAAA_0001, 5, 0, 0, 0xAAAA_0003, 7]);
        // Sixteen control bytes, 0x80 for an empty pair and the key's low 7 bits for a full one.
        let mut grouped = vec![0x80u8; 16];
        grouped[5] = (KEY_1 & 0x7F) as u8;
        grouped[6] = (KEY_2 & 0x7F) as u8;
        grouped.resize(16 + 16 * 8, 0);
        grouped[16 + 5 * 8..16 + 6 * 8].copy_from_slice(&words(&[KEY_1, 50]));
        grouped[16 + 6 * 8..16 + 7 * 8].copy_from_slice(&words(&[KEY_2, 60]));
        let mut classes = vec![root, pair, group];
        let mut blocks = vec![
            (ROOT, data),
            (PAIR, slots),
            (GROUP, grouped),
            (0x15, words(&[0xDEAD, 0xFFFF_FFFF, 1, 0x20])),
            (0x11, vec![1, 2, 3]),
            (0x08, words(&[1, 2, 3, 4])),
        ];
        patch(&mut classes, &mut blocks);
        body(&classes, &blocks, 1)
    }

    #[test]
    fn hash_maps_and_wide_values_read_back_to_the_values_they_were_built_with() {
        let (instance, _) = open(&hash_maps_and_wide_values(|_, _| {})).expect("it reads");
        let key = |h: u32| Value::Hash(hashed(h));
        let map = |pairs: &[(u32, u64)]| {
            Value::Map(
                pairs
                    .iter()
                    .map(|&(k, v)| (key(k), Value::UInt(v)))
                    .collect(),
            )
        };
        let bitset = |bits: u128| Value::Bitset {
            type_name: name(KIND),
            bits,
        };
        let mut expected = MetaInstance::new(name(ROOT));
        for (hash, value) in [
            (0x4444_0000, Value::UInt(0x0123_4567_89AB_CDEF)),
            (0x4444_0001, Value::UInt(0xFEDC_BA98_7654_3210)),
            (0x4444_0002, map(&[(0xAAAA_0001, 5), (0xAAAA_0003, 7)])),
            (0x4444_0005, map(&[(KEY_1, 50), (KEY_2, 60)])),
            (0x4444_0008, bitset(1 | 1 << 69)),
            (0x4444_0009, bitset(0xFFFF_FFFF | 1 << 32 | 1 << 69)),
            (
                0x4444_000A,
                Value::Array(vec![Value::UInt(1), Value::UInt(2), Value::UInt(3)]),
            ),
            (
                0x4444_000C,
                Value::Array(vec![Value::UInt(1 | 2 << 32), Value::UInt(3 | 4 << 32)]),
            ),
        ] {
            expected.fields.push((name(hash), value));
        }
        assert_eq!(instance, expected);
    }

    #[test]
    fn a_hash_map_or_wide_value_that_breaks_its_measured_relation_is_refused() {
        type Patch = Box<dyn FnOnce(&mut Vec<Class>, &mut Vec<(u32, Vec<u8>)>)>;
        let root = |b: &mut Vec<(u32, Vec<u8>)>, at: usize, bytes: &[u8]| {
            b[0].1[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let cases: Vec<(Patch, String)> = vec![
            (
                Box::new(move |_, b| root(b, 0x14, &3u32.to_le_bytes())),
                "counts 3 pairs, where its table holds 2".into(),
            ),
            (
                Box::new(move |_, b| root(b, 0x18, &0u64.to_le_bytes())),
                "states 4 slots and 2 pairs behind a null pointer".into(),
            ),
            (
                Box::new(|_, b| b[2].1[0] = 0x81),
                "control byte 0x81, which is neither 0x80 nor below it".into(),
            ),
            (
                Box::new(|_, b| b[2].1[7] = 0x07),
                "counts 2 pairs, where its table holds 3".into(),
            ),
            (
                Box::new(|c, _| c[2].size += 8),
                format!(
                    "table is a {}, which is not 16 control bytes and 16 pairs",
                    name(GROUP)
                ),
            ),
            (
                Box::new(|c, _| c[1].members[1] = member(0x063F_A3F2, 4, 0x14, 0, 0)),
                "pairs are a hash_11110011 whose members are not the map's key and value".into(),
            ),
            (
                Box::new(move |_, b| root(b, 0x38, &[0x40])),
                "holds bits past its 70".into(),
            ),
            (
                Box::new(|_, b| b[3].1[15] = 0x40),
                "holds bits past its 70".into(),
            ),
            (
                Box::new(move |_, b| root(b, 0x48, &[2])),
                "holds 70 bits in 2 words".into(),
            ),
            (
                Box::new(move |_, b| root(b, 0x4C, &[1])),
                "has 0x1 in its last word, where every measured one has 0".into(),
            ),
            (
                Box::new(move |_, b| root(b, 0x58, &[4])),
                "counts 4 elements in room for 3".into(),
            ),
        ];
        for (patch, want) in cases {
            let err = open(&hash_maps_and_wide_values(patch))
                .expect_err(&want)
                .to_string();
            assert!(err.contains(&want), "{want}: {err}");
        }
    }

    #[test]
    fn pointers_that_share_their_targets_are_refused_before_the_tree_outgrows_the_blocks() {
        // Each level's two pointers name the next level's one block, so the tree doubles per level.
        const LEVELS: u32 = 20;
        let class = |k: u32| 0x1111_1000 + k;
        let mut classes = Vec::new();
        let mut blocks = Vec::new();
        for k in 0..LEVELS {
            let last = k + 1 == LEVELS;
            let members = if last {
                Vec::new()
            } else {
                vec![
                    member(0x5555_0000, 0, 0x07, 0, 0),
                    member(0x5555_0001, 8, 0x07, 0, 0),
                ]
            };
            classes.push(Class {
                name: class(k),
                size: 16,
                members,
            });
            let next = pointer(u64::from(k) + 2, 0).to_le_bytes();
            let data = if last {
                vec![0; 16]
            } else {
                [next, next].concat()
            };
            blocks.push((class(k), data));
        }
        let err = open(&body(&classes, &blocks, 1))
            .map(|_| ())
            .expect_err("a tree of 2^20 values from 320 bytes")
            .to_string();
        assert!(err.contains("more values than"), "{err}");
    }

    #[test]
    fn a_count_larger_than_its_section_is_refused_before_anything_is_allocated() {
        let bytes = sample();
        let lrsc = bytes
            .windows(4)
            .position(|w| w == b"LRSC")
            .expect("an LRSC")
            + 8;
        let refusal = |huge: &[u8]| open(huge).expect_err("refused").to_string();
        for (at, what) in [
            (0x38, "the LSCH class records"),
            (0x3A, "the LENM enum records"),
            (0x3C, "the PSIN block table"),
        ] {
            let mut huge = bytes.clone();
            huge[lrsc + at..lrsc + at + 2].copy_from_slice(&u16::MAX.to_le_bytes());
            let err = refusal(&huge);
            assert!(err.contains(&format!("{what} (")), "{at:#X}: {err}");
        }
        let lenm = bytes
            .windows(4)
            .position(|w| w == b"LENM")
            .expect("an LENM")
            + 8;
        let mut huge = bytes.clone();
        huge[lenm + 0x10..lenm + 0x14].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = refusal(&huge);
        assert!(err.contains("an enum's values ("), "{err}");
    }
}
