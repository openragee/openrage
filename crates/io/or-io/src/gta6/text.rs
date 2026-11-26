//! GTA VI's text tables: a headerless RBM resource with no physical section, whose root is a
//! grouped hash map from a label's hash to one text or to the grammatical variants of one.
//!
//! A table is written as one page: the layout the game's writer chose (where the map block sits,
//! when it splits a section in two pages) is not known, so a written table opens as the same table
//! but is not byte-identical to the one the game ships. Written over the section it was read from,
//! a table keeps every byte and the length that section states while its blocks keep their sizes.

use crate::hash::string_hash;

/// The class of every text table's root, which a scan tells a text table by.
pub const MAP: u32 = 0x9933_1089;
const PAIR: u32 = 0x0000_0100;
const GROUP: u32 = 0x0000_0101;
const ENTRY: u32 = 0xA847_8889;
const VARIANT_ENTRY: u32 = 0x956A_9DF5;
const VARIANT: u32 = 0x1007_5946;
const VARIANT_PAIR: u32 = 0x455D_7E2B;
/// The data-block type of the characters: a basic type code, not a class.
const CHARS: u32 = 0x10;
const VBASE: u64 = 0x5000_0000;
const HEADER: usize = 0x60;
const HEADER_WORD: u16 = 0x7A;
const EMPTY: u8 = 0x80;
const SLOTS: usize = 16;
const GROUP_LEN: usize = 16 + 16 * SLOTS;
const ENTRY_LEN: usize = 16;
const VARIANT_LEN: usize = 32;
/// The most bytes one character block holds, a text's NUL included.
pub const CHAR_BLOCK: usize = 1 << 20;
const PAGE: usize = 4096;
const STREAM: usize = 0x1_0000;
/// The enum sizes of a variant entry's four enum bytes, from +0x18.
const ENTRY_ENUMS: [u8; 4] = [4, 4, 13, 13];
/// The enum sizes of a variant's three enum bytes, from +8.
const FORM_ENUMS: [u8; 3] = [4, 4, 16];

/// A member as the class table states it: name, offset, type, then three words of its kind.
type Member = (u32, u32, u8, u8, u16, u32);

struct Class {
    name: u32,
    signature: u32,
    flags: u8,
    align: u8,
    size: u32,
    members: &'static [Member],
}

/// The classes every text table declares byte for byte: the first four in a table of plain texts,
/// all seven in one with grammatical variants.
const CLASSES: [Class; 7] = [
    Class {
        name: MAP,
        signature: 0xBF55_68A4,
        flags: 0,
        align: 3,
        size: 16,
        members: &[
            (0x100, 0, 0x4A, 0, 0, 0),
            (0x100, 0, 0x07, 0, 0, 0),
            (0xD566_F590, 0, 0x75, 0, 0, 0x1_0000),
        ],
    },
    Class {
        name: PAIR,
        signature: 0xCF6F_033C,
        flags: 12,
        align: 3,
        size: 16,
        members: &[
            (0x6098_A50E, 0, 0x4A, 0, 0, 0),
            (0x063F_A3F2, 8, 0x07, 0, 0, 0),
        ],
    },
    Class {
        name: GROUP,
        signature: 0x9359_D11F,
        flags: 36,
        align: 4,
        size: GROUP_LEN as u32,
        members: &[
            (0x100, 0, 0x11, 0, 0, 0),
            (0xB879_4C89, 0, 0x50, 0, 0, 0x10),
            (0x100, 0, 0x05, 0, 0, 0x100),
            (0x93C4_95AF, 0x10, 0x50, 0, 2, 0x10),
        ],
    },
    Class {
        name: ENTRY,
        signature: 0x5C6A_83CE,
        flags: 0,
        align: 3,
        size: 16,
        members: &[(0xB5A7_CD47, 8, 0x42, 0, 0, 0)],
    },
    Class {
        name: VARIANT_ENTRY,
        signature: 0xC5D4_1932,
        flags: 0,
        align: 3,
        size: 32,
        members: &[
            (0x100, 0, 0x05, 0, 0, VARIANT),
            (0xA41C_95E0, 8, 0x5C, 0, 0, 0),
            (0x1599_02A6, 0x18, 0x60, 0, 0, 0x2A05_3C90),
            (0x3EFA_A23F, 0x19, 0x60, 0, 0, 0xB46A_EA0A),
            (0x27D7_357F, 0x1A, 0x60, 0, 0, 0x12B2_9290),
            (0xE023_7FCF, 0x1B, 0x60, 0, 0, 0x12B2_9290),
        ],
    },
    Class {
        name: VARIANT,
        signature: 0x0BEB_AA12,
        flags: 0,
        align: 3,
        size: 32,
        members: &[
            (0xB5A7_CD47, 0, 0x42, 0, 0, 0),
            (0x1599_02A6, 8, 0x60, 0, 0, 0x2A05_3C90),
            (0x3EFA_A23F, 9, 0x60, 0, 0, 0xB46A_EA0A),
            (0x42DB_2391, 0xA, 0x60, 0, 0, 0x4EF0_1F30),
            (0x100, 0, 0x05, 0, 0, VARIANT_PAIR),
            (0x34A6_43C5, 0xB, 0x50, 0, 4, 9),
        ],
    },
    Class {
        name: VARIANT_PAIR,
        signature: 0x408C_F8A6,
        flags: 0,
        align: 0,
        size: 2,
        members: &[
            (0xE6EC_5281, 0, 0x11, 0, 0, 0),
            (0x9329_A7A6, 1, 0x11, 0, 0, 0),
        ],
    },
];

/// The enums of a table with variants: name, signature, and the name of each value from 0.
const ENUMS: [(u32, u32, &[u32]); 4] = [
    (
        0x2A05_3C90,
        0x8A4D_C51B,
        &[0x61C2_4651, 0xE954_4F6D, 0x0663_DEA9, 0xF67F_B169],
    ),
    (
        0xB46A_EA0A,
        0x3678_DD73,
        &[0x61C2_4651, 0x8EE5_BBCE, 0x74BE_8B8F, 0xAB6B_5630],
    ),
    (
        0x12B2_9290,
        0x341B_9BC5,
        &[
            0x61C2_4651,
            0x6EC9_321B,
            0xBE3A_5100,
            0xD090_F5AD,
            0xA9E8_A85D,
            0xFC2E_CCE8,
            0x0753_E332,
            0xF988_479F,
            0x12CA_7A23,
            0xE25B_9946,
            0xA3FF_B0F5,
            0x97BF_DFDA,
            0xE670_8F11,
        ],
    ),
    (
        0x4EF0_1F30,
        0xC662_2EE9,
        &[
            0x61C2_4651,
            0x8184_4703,
            0x93CA_6B8F,
            0x2D19_1E2E,
            0xFF6F_C2DC,
            0x489C_D535,
            0x5ADE_F9B9,
            0xD83E_F46F,
            0xAA84_98FB,
            0xF3BA_AB66,
            0x0BAF_4A2B,
            0x8DF2_CEB4,
            0x7023_9316,
            0x825D_3789,
            0x5116_54FC,
            0xCAF4_C897,
        ],
    ),
];

/// Why a body is no text table, or cannot be read or written as one.
#[derive(Debug, thiserror::Error)]
pub enum TextTableError {
    #[error("not a GTA VI text table: {0}")]
    NotText(String),
    #[error("GTA VI text table: {0}")]
    Malformed(String),
    #[error("the text of key {key:#010X} {why}")]
    Text { key: u32, why: &'static str },
    #[error("GTA VI text table: {0}")]
    Unwritable(String),
}

type Result<T> = std::result::Result<T, TextTableError>;

fn bad(why: String) -> TextTableError {
    TextTableError::Malformed(why)
}

/// A text table: its group count and its used slots, in slot order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextTable {
    pub groups: u32,
    pub entries: Vec<TextEntry>,
}

/// One used slot: the label's hash, where it sits, and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEntry {
    pub key: u32,
    pub group: u32,
    pub slot: u8,
    pub item: TextItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextItem {
    Text(String),
    Variants(Variants),
}

/// A text with grammatical forms; the two enums whose value names are not known keep their hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variants {
    pub gender: u8,
    pub plurality: u8,
    pub hash_27d7357f: u8,
    pub hash_e0237fcf: u8,
    pub forms: Vec<Variant>,
}

/// One grammatical form of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub text: String,
    pub gender: u8,
    pub plurality: u8,
    pub case: u8,
    /// Nine pairs of bytes of class 0x455D7E2B, whose role is not known.
    pub hash_34a643c5: [[u8; 2]; 9],
}

/// Where a body states its blocks: what a reader saw, never what a writer keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLayout {
    pub length: usize,
    /// The block map's virtual block count: 1 or 2.
    pub virtual_blocks: u8,
    /// The header's word at 0x4E: in a one-block body the 4 KiB pages before the class table
    /// when the section runs past them.
    pub word_4e: u16,
    pub root: u32,
    pub blocks: Vec<TextBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBlock {
    /// Its class, or 0x10 for characters.
    pub kind: u32,
    pub offset: usize,
    pub size: usize,
}

/// Whether the class records `classes`, 32 bytes each, declare a text table's map.
pub fn declares_map(classes: &[u8]) -> bool {
    (classes.as_chunks::<32>().0.iter())
        .any(|r| u32::from_le_bytes([r[0], r[1], r[2], r[3]]) == MAP)
}

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn get(&self, at: usize, len: usize) -> Result<&[u8]> {
        at.checked_add(len)
            .and_then(|end| self.0.get(at..end))
            .ok_or_else(|| bad(format!("{len} bytes at {at:#X} run past the section")))
    }

    fn u16(&self, at: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(
            self.get(at, 2)?.try_into().unwrap_or_default(),
        ))
    }

    fn u32(&self, at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.get(at, 4)?.try_into().unwrap_or_default(),
        ))
    }

    fn u64(&self, at: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.get(at, 8)?.try_into().unwrap_or_default(),
        ))
    }

    fn zero(&self, at: usize, len: usize) -> Result<bool> {
        Ok(self.get(at, len)?.iter().all(|b| *b == 0))
    }

    /// The section offset a 0x5000_0000 pointer names, with `len` bytes there.
    fn ptr(&self, p: u64, len: usize, what: &str) -> Result<usize> {
        let at = usize::try_from(p.wrapping_sub(VBASE)).unwrap_or(usize::MAX);
        if p >> 28 != 5 || self.get(at, len).is_err() {
            return Err(bad(format!(
                "{what} ({len} bytes at {p:#X}) is not inside the section"
            )));
        }
        Ok(at)
    }
}

/// The items of one block type, taken in order across its blocks in id order.
struct Run {
    /// Indices into the block list.
    blocks: Vec<usize>,
    at: (usize, usize),
    taken: usize,
}

impl Run {
    fn new(blocks: &[TextBlock], kind: u32) -> Self {
        let blocks = (0..blocks.len())
            .filter(|i| blocks[*i].kind == kind)
            .collect();
        Self {
            blocks,
            at: (0, 0),
            taken: 0,
        }
    }

    /// The section offset of the next item of `unit` bytes, which `(index, offset)` must name.
    fn take(&mut self, all: &[TextBlock], unit: usize, named: (usize, usize)) -> Result<usize> {
        let (mut i, mut off) = self.at;
        while i < self.blocks.len() && off >= all[self.blocks[i]].size {
            (i, off) = (i + 1, 0);
        }
        let Some(&index) = self.blocks.get(i) else {
            return Err(bad(format!(
                "more items point into blocks of {:#010X} than they hold",
                all[named.0].kind
            )));
        };
        if named != (index, off) || off + unit > all[index].size {
            return Err(bad(format!(
                "an item at block {} + {:#X} is not the next item of its block, at block {} + {off:#X}",
                named.0 + 1,
                named.1,
                index + 1
            )));
        }
        self.at = (i, off + unit);
        self.taken += 1;
        Ok(all[index].offset + off)
    }

    fn whole(&self, all: &[TextBlock], unit: usize) -> bool {
        self.taken * unit == self.blocks.iter().map(|i| all[*i].size).sum::<usize>()
    }
}

/// The text table a virtual section holds, and where it states its blocks.
pub fn read(v: &[u8]) -> Result<(TextTable, TextLayout)> {
    let r = Bytes(v);
    if v.len() < HEADER || v[0x10..0x14] != *b"0DRP" {
        return Err(TextTableError::NotText(
            "no 0DRP header at 0x10".to_string(),
        ));
    }
    let len = v.len();
    let (nc, ne, nb) = (
        usize::from(r.u16(0x48)?),
        usize::from(r.u16(0x4A)?),
        usize::from(r.u16(0x4C)?),
    );
    let ct = r.ptr(r.u64(0x28)?, 32 * nc, "the class table")?;
    if !declares_map(r.get(ct, 32 * nc)?) {
        return Err(TextTableError::NotText(format!(
            "none of its {nc} classes is {MAP:#010X}"
        )));
    }
    let variant = classes(&r, ct, nc)?;
    if ne != if variant { ENUMS.len() } else { 0 } {
        return Err(bad(format!(
            "{ne} enums, where its classes take {}",
            if variant { ENUMS.len() } else { 0 }
        )));
    }
    enums(&r, ne)?;

    let zero_words = [
        (0x00, 8),
        (0x18, 4),
        (0x1C, 4),
        (0x24, 4),
        (0x50, 8),
        (0x58, 8),
    ];
    for (at, width) in zero_words {
        if !r.zero(at, width)? {
            return Err(bad(format!("the header word at {at:#X} is not 0")));
        }
    }
    if r.u16(0x14)? != HEADER_WORD || v[0x16] != 0 || v[0x17] != 0 {
        return Err(bad(
            "the header's bytes 0x14 to 0x17 are not 7A 00 00 00".into()
        ));
    }
    let bm = r.ptr(r.u64(0x08)?, 16, "the block map")?;
    let map = r.get(bm, 16)?;
    if !map[..8].iter().chain(&map[9..]).all(|b| *b == 0) || !matches!(map[8], 1 | 2) {
        return Err(bad(
            "the block map is not zero beside a virtual block count of 1 or 2 and no physical block"
                .into(),
        ));
    }
    let virtual_blocks = map[8];
    let bt = r.ptr(r.u64(0x38)?, 16 * nb, "the block table")?;
    let st = r.ptr(r.u64(0x40)?, 4, "the string table")?;
    if r.u32(st)? != 0 {
        return Err(bad("the string table counts entries".into()));
    }

    let mut blocks = Vec::with_capacity(nb);
    for i in 0..nb {
        let at = bt + 16 * i;
        let (kind, size) = (r.u32(at)?, r.u32(at + 4)? as usize);
        let offset = r.ptr(r.u64(at + 8)?, size, "a data block")?;
        let known = kind == CHARS || CLASSES.iter().any(|c| c.name == kind);
        if !known || matches!(kind, PAIR | VARIANT_PAIR) {
            return Err(bad(format!(
                "block {} holds {kind:#010X}, which no text table stores as a block",
                i + 1
            )));
        }
        if size > CHAR_BLOCK && kind == CHARS {
            return Err(bad(format!(
                "character block {} holds {size} bytes, more than {CHAR_BLOCK}",
                i + 1
            )));
        }
        blocks.push(TextBlock { kind, offset, size });
    }
    let count = |kind: u32| blocks.iter().filter(|b| b.kind == kind).count();
    if count(MAP) != 1 || count(GROUP) != 1 || count(CHARS) == 0 {
        return Err(bad(
            "not one map block, one group block and at least one character block".into(),
        ));
    }
    if (count(VARIANT_ENTRY) > 0) != variant || (count(VARIANT) > 0) != variant {
        return Err(bad(
            "its variant blocks and its variant classes do not go together".into(),
        ));
    }
    let mut spans: Vec<(usize, usize)> = blocks.iter().map(|b| (b.offset, b.size)).collect();
    spans.sort_unstable();
    if spans.windows(2).any(|w| w[0].0 + w[0].1 > w[1].0) {
        return Err(bad("two data blocks overlap".into()));
    }
    let root = r.u32(0x20)?;
    let map_index = blocks.iter().position(|b| b.kind == MAP).unwrap_or(0);
    if root as usize != map_index + 1 {
        return Err(bad(format!("the root names block {root}, not the map")));
    }
    let table = entries(&r, &blocks, map_index)?;

    let phase_a = (blocks.iter().map(|b| b.offset + b.size))
        .chain([bt + 16 * nb, bm + 16])
        .max()
        .unwrap_or(0);
    let word_4e = r.u16(0x4E)?;
    if virtual_blocks == 1 && word_4e != pages_before_schema(phase_a, len) {
        return Err(bad(format!(
            "the header's word at 0x4E is {word_4e}, not the {} the data's pages give",
            pages_before_schema(phase_a, len)
        )));
    }
    let layout = TextLayout {
        length: len,
        virtual_blocks,
        word_4e,
        root,
        blocks,
    };
    Ok((table, layout))
}

/// The 4 KiB pages up to `phase_a`, where a section of `len` bytes runs past them; else 0.
fn pages_before_schema(phase_a: usize, len: usize) -> u16 {
    let pages = phase_a.div_ceil(PAGE);
    if pages * PAGE < len {
        u16::try_from(pages).unwrap_or(u16::MAX)
    } else {
        0
    }
}

/// Checks the class table against the classes every text table declares; whether it has variants.
fn classes(r: &Bytes, ct: usize, nc: usize) -> Result<bool> {
    let mut seen = Vec::with_capacity(nc);
    for i in 0..nc {
        let at = ct + 32 * i;
        let name = r.u32(at)?;
        let Some(class) = CLASSES.iter().find(|c| c.name == name) else {
            return Err(bad(format!(
                "class {name:#010X} is no class of a text table"
            )));
        };
        if seen.contains(&name) {
            return Err(bad(format!("class {name:#010X} is declared twice")));
        }
        let count = usize::from(r.u16(at + 30)?);
        let members_at = r.ptr(r.u64(at + 16)?, 16 * count, "a class's members")?;
        let mut members = Vec::with_capacity(count);
        for j in 0..count {
            let m = members_at + 16 * j;
            let b = r.get(m, 16)?;
            members.push((
                r.u32(m)?,
                r.u32(m + 4)?,
                b[8],
                b[9],
                r.u16(m + 10)?,
                r.u32(m + 12)?,
            ));
        }
        let head = r.get(at, 32)?;
        let stated = (
            r.u32(at + 4)?,
            head[8],
            head[9],
            r.u32(at + 24)?,
            r.u16(at + 28)?,
        );
        let wanted = (class.signature, class.flags, class.align, class.size, 0);
        if stated != wanted || !r.zero(at + 10, 6)? || members != class.members {
            return Err(bad(format!(
                "class {name:#010X} differs from the one every text table declares"
            )));
        }
        seen.push(name);
    }
    let plain = CLASSES[..4].iter().all(|c| seen.contains(&c.name));
    match (plain, seen.len()) {
        (true, 4) => Ok(false),
        (true, 7) => Ok(true),
        _ => Err(bad(
            "its classes are neither the plain set nor the set with variants".into(),
        )),
    }
}

fn enums(r: &Bytes, ne: usize) -> Result<()> {
    if ne == 0 {
        return match r.u64(0x30)? {
            0 => Ok(()),
            _ => Err(bad("the enum table pointer is not null".into())),
        };
    }
    let et = r.ptr(r.u64(0x30)?, 24 * ne, "the enum table")?;
    let mut seen = Vec::with_capacity(ne);
    for i in 0..ne {
        let at = et + 24 * i;
        let name = r.u32(at)?;
        let Some((_, signature, values)) = ENUMS.iter().find(|e| e.0 == name) else {
            return Err(bad(format!("enum {name:#010X} is no enum of a text table")));
        };
        let n = r.u32(at + 16)? as usize;
        let differs = || {
            bad(format!(
                "enum {name:#010X} differs from the one every table declares"
            ))
        };
        if seen.contains(&name)
            || n != values.len()
            || r.u32(at + 4)? != *signature
            || r.u32(at + 20)? != 0
        {
            return Err(differs());
        }
        let vo = r.ptr(r.u64(at + 8)?, 16 * n, "an enum's values")?;
        for (j, value) in values.iter().enumerate() {
            let w = vo + 16 * j;
            if r.u64(w)? != j as u64 || r.u32(w + 8)? != *value || r.u32(w + 12)? != 0 {
                return Err(differs());
            }
        }
        seen.push(name);
    }
    Ok(())
}

/// A data pointer: block id in bits 0 to 11, bits 12 to 19 clear, the offset from bit 20.
fn data_ptr(p: u64, blocks: &[TextBlock], kinds: &[u32], what: &str) -> Result<(usize, usize)> {
    let id = (p & 0xFFF) as usize;
    let off = usize::try_from(p >> 20).unwrap_or(usize::MAX);
    let block = id.checked_sub(1).and_then(|i| blocks.get(i));
    match block {
        Some(b) if (p >> 12) & 0xFF == 0 && kinds.contains(&b.kind) && off < b.size => {
            Ok((id - 1, off))
        }
        _ => Err(bad(format!(
            "{what} {p:#X} names no place in a block it may point into"
        ))),
    }
}

fn enum_bytes(bytes: &[u8], sizes: &[u8], what: &str) -> Result<()> {
    match bytes.iter().zip(sizes).find(|(b, n)| b >= n) {
        Some((b, _)) => Err(bad(format!("{what} holds enum value {b}, past its enum"))),
        None => Ok(()),
    }
}

/// The used slots of the map, walked group by group, with their texts.
fn entries(r: &Bytes, blocks: &[TextBlock], map_index: usize) -> Result<TextTable> {
    let mb = blocks[map_index];
    if mb.size != 16 {
        return Err(bad("the map block is not 16 bytes".into()));
    }
    let (groups, n) = (r.u32(mb.offset)?, r.u32(mb.offset + 4)?);
    let gi = blocks.iter().position(|b| b.kind == GROUP).unwrap_or(0);
    if data_ptr(
        r.u64(mb.offset + 8)?,
        blocks,
        &[GROUP],
        "the map's group pointer",
    )? != (gi, 0)
    {
        return Err(bad(
            "the map's group pointer is not the group block's start".into(),
        ));
    }
    if !groups.is_power_of_two() || blocks[gi].size != GROUP_LEN * groups as usize {
        return Err(bad(format!(
            "{groups} groups is no power of two the {}-byte group block holds",
            blocks[gi].size
        )));
    }
    let (mut plain, mut sets, mut forms) = (
        Run::new(blocks, ENTRY),
        Run::new(blocks, VARIANT_ENTRY),
        Run::new(blocks, VARIANT),
    );
    let mut texts = Texts::new(blocks);
    let mut out = Vec::new();
    let mut full = Vec::with_capacity(groups as usize);
    for g in 0..groups as usize {
        let base = blocks[gi].offset + GROUP_LEN * g;
        let control = r.get(base, SLOTS)?;
        full.push(control.iter().all(|c| *c != EMPTY));
        for (s, &c) in control.iter().enumerate() {
            let pair = base + 16 + 16 * s;
            if c == EMPTY {
                if !r.zero(pair, 16)? {
                    return Err(bad(format!("empty slot {g}.{s} is not zero")));
                }
                continue;
            }
            let key = r.u32(pair)?;
            if c > EMPTY || u32::from(c) != key & 0x7F || r.u32(pair + 4)? != 0 {
                return Err(bad(format!(
                    "slot {g}.{s}'s control byte {c:#X} is not its key's low 7 bits"
                )));
            }
            let item = r.u64(pair + 8)?;
            let place = data_ptr(item, blocks, &[ENTRY, VARIANT_ENTRY], "an item")?;
            let item = if blocks[place.0].kind == ENTRY {
                let at = plain.take(blocks, ENTRY_LEN, place)?;
                if r.u64(at)? != 0 {
                    return Err(bad(format!(
                        "the entry of slot {g}.{s} does not open with 0"
                    )));
                }
                TextItem::Text(texts.read(r, r.u64(at + 8)?)?)
            } else {
                let at = sets.take(blocks, VARIANT_LEN, place)?;
                let (count, capacity) = (r.u32(at + 8)? as usize, r.u32(at + 12)? as usize);
                if r.u64(at)? != 0 || !r.zero(at + 0x1C, 4)? || count == 0 || count != capacity {
                    return Err(bad(format!(
                        "the variant entry of slot {g}.{s} is not {{0, n of n variants, 4 enums, 0}}"
                    )));
                }
                let e = r.get(at + 0x18, 4)?;
                enum_bytes(e, &ENTRY_ENUMS, "a variant entry")?;
                let first = data_ptr(r.u64(at + 16)?, blocks, &[VARIANT], "a variant list")?;
                let mut list = Vec::new();
                for j in 0..count {
                    let w =
                        forms.take(blocks, VARIANT_LEN, (first.0, first.1 + VARIANT_LEN * j))?;
                    let b = r.get(w, VARIANT_LEN)?;
                    enum_bytes(&b[8..11], &FORM_ENUMS, "a variant")?;
                    if b[0x1D..].iter().any(|x| *x != 0) {
                        return Err(bad(format!(
                            "variant {j} of slot {g}.{s} does not end with zeros"
                        )));
                    }
                    let mut pairs = [[0u8; 2]; 9];
                    for (k, p) in pairs.iter_mut().enumerate() {
                        *p = [b[0xB + 2 * k], b[0xC + 2 * k]];
                    }
                    list.push(Variant {
                        text: texts.read(r, r.u64(w)?)?,
                        gender: b[8],
                        plurality: b[9],
                        case: b[0xA],
                        hash_34a643c5: pairs,
                    });
                }
                TextItem::Variants(Variants {
                    gender: e[0],
                    plurality: e[1],
                    hash_27d7357f: e[2],
                    hash_e0237fcf: e[3],
                    forms: list,
                })
            };
            out.push(TextEntry {
                key,
                group: g as u32,
                slot: s as u8,
                item,
            });
        }
    }
    if out.len() != n as usize {
        return Err(bad(format!(
            "the groups hold {} entries, the map counts {n}",
            out.len()
        )));
    }
    let mut keys: Vec<u32> = out.iter().map(|e| e.key).collect();
    keys.sort_unstable();
    if keys.windows(2).any(|w| w[0] == w[1]) {
        return Err(bad("a key is in the table twice".into()));
    }
    let full_before = super::hash::full_before(&full);
    for e in &out {
        let home = (e.key >> 7) & (groups - 1);
        let distance = e.group.wrapping_sub(home) & (groups - 1);
        if full_before[e.group as usize] < distance as usize {
            return Err(bad(format!(
                "key {:#010X} lies past a group with room, where linear probing does not put it",
                e.key
            )));
        }
    }
    if !plain.whole(blocks, ENTRY_LEN)
        || !sets.whole(blocks, VARIANT_LEN)
        || !forms.whole(blocks, VARIANT_LEN)
    {
        return Err(bad(
            "an entry block holds more than the items that point at it".into(),
        ));
    }
    texts.whole(blocks)?;
    Ok(TextTable {
        groups,
        entries: out,
    })
}

/// The character blocks in id order, filled as their texts are visited.
struct Texts {
    all: Vec<TextBlock>,
    blocks: Vec<usize>,
    fill: Vec<usize>,
}

impl Texts {
    fn new(blocks: &[TextBlock]) -> Self {
        let chars: Vec<usize> = (0..blocks.len())
            .filter(|i| blocks[*i].kind == CHARS)
            .collect();
        let fill = vec![0; chars.len()];
        Self {
            all: blocks.to_vec(),
            blocks: chars,
            fill,
        }
    }

    /// The text `p` points at, which must lie where first fit puts the next text: at the end of
    /// the first character block with room for it and its NUL.
    fn read(&mut self, r: &Bytes, p: u64) -> Result<String> {
        let (index, off) = data_ptr(p, &self.all, &[CHARS], "a text")?;
        let block = self.all[index];
        let bytes = r.get(block.offset + off, block.size - off)?;
        let len = (bytes.iter().position(|b| *b == 0))
            .ok_or_else(|| bad(format!("the text at {p:#X} has no NUL inside its block")))?;
        let k = self.blocks.iter().position(|i| *i == index).unwrap_or(0);
        let first = self.fill.iter().position(|f| f + len < CHAR_BLOCK);
        if first != Some(k) || off != self.fill[k] {
            return Err(bad(format!(
                "the text at {p:#X} is not where first fit into the character blocks puts it"
            )));
        }
        self.fill[k] += len + 1;
        String::from_utf8(bytes[..len].to_vec())
            .map_err(|_| bad(format!("the text at {p:#X} is not UTF-8")))
    }

    /// Checks that every character block is as long as the texts it holds.
    fn whole(&self, blocks: &[TextBlock]) -> Result<()> {
        match (self.blocks.iter().zip(&self.fill)).find(|(i, f)| blocks[**i].size != **f) {
            Some((i, _)) => Err(bad(format!(
                "character block {} is longer than the texts it holds",
                i + 1
            ))),
            None => Ok(()),
        }
    }
}

/// Character blocks, and each text's block and offset in them.
type Packed = (Vec<Vec<u8>>, Vec<(usize, usize)>);

impl TextTable {
    /// The entry of `label`, whose key is its hash with `A` to `Z` folded to lower case.
    pub fn get(&self, label: &str) -> Option<&TextEntry> {
        self.find(string_hash(label.as_bytes()))
    }

    /// The entry of `key`, probed for as the game does: groups from its home group on, until one
    /// holds it or has room.
    pub fn find(&self, key: u32) -> Option<&TextEntry> {
        let mask = self.groups.checked_sub(1)?;
        let home = (key >> 7) & mask;
        for d in 0..self.groups {
            let g = (home + d) & mask;
            let from = self.entries.partition_point(|e| e.group < g);
            let to = self.entries.partition_point(|e| e.group <= g);
            let group = &self.entries[from..to];
            if let Some(e) = group.iter().find(|e| e.key == key) {
                return Some(e);
            }
            if group.len() < SLOTS {
                return None;
            }
        }
        None
    }

    /// Each text in the order a table stores them: an entry's, or a variant entry's forms in order.
    pub fn texts(&self) -> impl Iterator<Item = (u32, &str)> {
        self.entries.iter().flat_map(|e| {
            let texts: Vec<&str> = match &e.item {
                TextItem::Text(t) => vec![t.as_str()],
                TextItem::Variants(v) => v.forms.iter().map(|f| f.text.as_str()).collect(),
            };
            texts.into_iter().map(move |t| (e.key, t))
        })
    }

    /// The virtual section of this table as one page, which [`read`] returns as this table.
    pub fn to_section(&self) -> Result<Vec<u8>> {
        let (chars, places) = self.pack(usize::MAX)?;
        if chars.is_empty() {
            return Err(TextTableError::Unwritable(
                "a table holds at least one text".into(),
            ));
        }
        // Block ids from 1: groups, map, entries, variant entries, variants, then the characters.
        let mut kinds = self.record_blocks();
        let variant = kinds.iter().any(|k| k.0 == VARIANT);
        kinds.extend(chars.iter().map(|c| (CHARS, c.len())));
        let data = self.data_blocks(&kinds, &chars, &places)?;
        self.reads_back(lay_out(&kinds, &data, variant)?)
    }

    /// This table written over `original`, the section of a table, at the blocks it states: every
    /// other byte kept, refused where a block would change size and so the section's length.
    pub fn to_section_over(&self, original: &[u8]) -> Result<Vec<u8>> {
        let (_, layout) = read(original)?;
        let stated = |kind: u32| layout.blocks.iter().filter(move |b| b.kind == kind);
        let mut records = self.record_blocks();
        let mut held: Vec<(u32, usize)> = (layout.blocks.iter())
            .filter(|b| b.kind != CHARS)
            .map(|b| (b.kind, b.size))
            .collect();
        records.sort_unstable();
        held.sort_unstable();
        if records != held {
            return Err(TextTableError::Unwritable(
                "its groups, entries or variants are not as many as the section holds, so its length would change".into(),
            ));
        }
        let (chars, places) = self.pack(stated(CHARS).count())?;
        if !chars.iter().map(Vec::len).eq(stated(CHARS).map(|b| b.size)) {
            return Err(TextTableError::Unwritable(
                "its texts do not fill the character blocks to the sizes the section states, so its length would change".into(),
            ));
        }
        let kinds: Vec<(u32, usize)> = layout.blocks.iter().map(|b| (b.kind, b.size)).collect();
        let data = self.data_blocks(&kinds, &chars, &places)?;
        let mut section = original.to_vec();
        for (block, bytes) in layout.blocks.iter().zip(&data) {
            section[block.offset..block.offset + block.size].copy_from_slice(bytes);
        }
        self.reads_back(section)
    }

    /// `section` where it reads back as this table.
    fn reads_back(&self, section: Vec<u8>) -> Result<Vec<u8>> {
        match read(&section) {
            Ok((back, _)) if back == *self => Ok(section),
            Ok(_) => Err(TextTableError::Unwritable(
                "its entries do not read back as written: they are not in slot order, two share a slot, or a key sits where probing does not find it".into(),
            )),
            Err(e) => Err(TextTableError::Unwritable(format!("it does not read back: {e}"))),
        }
    }

    /// Its record blocks as a written table numbers them: groups, map, entries, variant entries,
    /// variants; each with its size.
    fn record_blocks(&self) -> Vec<(u32, usize)> {
        let (sets, forms): (usize, usize) = (self.entries.iter())
            .filter_map(|e| match &e.item {
                TextItem::Variants(v) => Some(v.forms.len()),
                TextItem::Text(_) => None,
            })
            .fold((0, 0), |(s, f), n| (s + 1, f + n));
        let plain = self.entries.len() - sets;
        let mut kinds = vec![(GROUP, GROUP_LEN * self.groups as usize), (MAP, 16)];
        if plain > 0 {
            kinds.push((ENTRY, ENTRY_LEN * plain));
        }
        if sets > 0 {
            kinds.push((VARIANT_ENTRY, VARIANT_LEN * sets));
            kinds.push((VARIANT, VARIANT_LEN * forms));
        }
        kinds
    }

    /// Its texts in visit order by first fit into at most `most` character blocks: the blocks,
    /// and each text's block and offset.
    fn pack(&self, most: usize) -> Result<Packed> {
        let mut chars: Vec<Vec<u8>> = Vec::new();
        let mut places = Vec::new();
        for (key, text) in self.texts() {
            if text.contains('\0') {
                return Err(TextTableError::Text {
                    key,
                    why: "holds a NUL, which ends a text",
                });
            }
            let n = text.len() + 1;
            if n > CHAR_BLOCK {
                return Err(TextTableError::Text {
                    key,
                    why: "is longer than a character block holds",
                });
            }
            let k = match chars.iter().position(|b| b.len() + n <= CHAR_BLOCK) {
                Some(k) => k,
                None if chars.len() < most => {
                    chars.push(Vec::new());
                    chars.len() - 1
                }
                None => {
                    return Err(TextTableError::Unwritable(format!(
                        "its texts need more than the {most} character blocks the section states"
                    )))
                }
            };
            places.push((k, chars[k].len()));
            chars[k].extend_from_slice(text.as_bytes());
            chars[k].push(0);
        }
        Ok((chars, places))
    }

    /// The bytes of the blocks `kinds` (kind and size, in id order), its texts at `places` in
    /// the character blocks `chars`, which are those of `kinds` in order.
    fn data_blocks(
        &self,
        kinds: &[(u32, usize)],
        chars: &[Vec<u8>],
        places: &[(usize, usize)],
    ) -> Result<Vec<Vec<u8>>> {
        let groups = self.groups as usize;
        let index = |kind: u32| kinds.iter().position(|k| k.0 == kind).unwrap_or(0);
        let (gi, mi, ei, si, fi) = (
            index(GROUP),
            index(MAP),
            index(ENTRY),
            index(VARIANT_ENTRY),
            index(VARIANT),
        );
        let char_blocks: Vec<usize> = (0..kinds.len()).filter(|i| kinds[*i].0 == CHARS).collect();
        let ptr = |index: usize, off: usize| (index as u64 + 1) | (off as u64) << 20;

        let mut data: Vec<Vec<u8>> = kinds.iter().map(|(_, size)| vec![0u8; *size]).collect();
        for (&i, c) in char_blocks.iter().zip(chars) {
            data[i].copy_from_slice(c);
        }
        let mut texts = places.iter();
        let mut text_ptr = || {
            (texts.next())
                .and_then(|(k, off)| char_blocks.get(*k).map(|&i| ptr(i, *off)))
                .unwrap_or(0)
        };
        let (mut p, mut s, mut f) = (0, 0, 0);
        for e in &self.entries {
            if e.group as usize >= groups || usize::from(e.slot) >= SLOTS {
                return Err(TextTableError::Unwritable(format!(
                    "key {:#010X} sits at slot {}.{}, outside {groups} groups of {SLOTS}",
                    e.key, e.group, e.slot
                )));
            }
            let base = GROUP_LEN * e.group as usize;
            let pair = base + 16 + 16 * usize::from(e.slot);
            let item = match &e.item {
                TextItem::Text(_) => {
                    let at = ENTRY_LEN * p;
                    data[ei][at + 8..at + 16].copy_from_slice(&text_ptr().to_le_bytes());
                    p += 1;
                    ptr(ei, at)
                }
                TextItem::Variants(v) => {
                    let at = VARIANT_LEN * s;
                    let n = (v.forms.len() as u32).to_le_bytes();
                    let w = &mut data[si][at..at + VARIANT_LEN];
                    w[8..12].copy_from_slice(&n);
                    w[12..16].copy_from_slice(&n);
                    w[16..24].copy_from_slice(&ptr(fi, VARIANT_LEN * f).to_le_bytes());
                    w[0x18..0x1C].copy_from_slice(&[
                        v.gender,
                        v.plurality,
                        v.hash_27d7357f,
                        v.hash_e0237fcf,
                    ]);
                    for form in &v.forms {
                        let w = &mut data[fi][VARIANT_LEN * f..VARIANT_LEN * (f + 1)];
                        w[..8].copy_from_slice(&text_ptr().to_le_bytes());
                        w[8..11].copy_from_slice(&[form.gender, form.plurality, form.case]);
                        w[0xB..0x1D].copy_from_slice(form.hash_34a643c5.as_flattened());
                        f += 1;
                    }
                    s += 1;
                    ptr(si, at)
                }
            };
            let g = &mut data[gi];
            g[base + usize::from(e.slot)] = (e.key & 0x7F) as u8;
            g[pair..pair + 4].copy_from_slice(&e.key.to_le_bytes());
            g[pair + 8..pair + 16].copy_from_slice(&item.to_le_bytes());
        }
        let g = &mut data[gi];
        for group in 0..groups {
            for slot in 0..SLOTS {
                let pair = GROUP_LEN * group + 16 + 16 * slot;
                if g[pair..pair + 16].iter().all(|b| *b == 0) {
                    g[GROUP_LEN * group + slot] = EMPTY;
                }
            }
        }
        let m = &mut data[mi];
        m[..4].copy_from_slice(&self.groups.to_le_bytes());
        m[4..8].copy_from_slice(&(self.entries.len() as u32).to_le_bytes());
        m[8..16].copy_from_slice(&ptr(gi, 0).to_le_bytes());
        Ok(data)
    }
}

/// A section of `blocks` (kind and size, ids from 1) holding `data`: the header, the data blocks,
/// the block table and the block map by size, largest first; then the class table, the member
/// arrays, the enums and an empty string table.
fn lay_out(blocks: &[(u32, usize)], data: &[Vec<u8>], variant: bool) -> Result<Vec<u8>> {
    const BLOCK_TABLE: usize = usize::MAX;
    const BLOCK_MAP: usize = usize::MAX - 1;
    let mut items: Vec<(usize, usize)> = (0..blocks.len()).map(|i| (i, blocks[i].1)).collect();
    items.push((BLOCK_TABLE, 16 * blocks.len()));
    // The loose writer's block map holds a slot after its 16 bytes, which stays zero here.
    items.push((BLOCK_MAP, 24));
    items.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    let mut at = HEADER;
    let mut placed = vec![0usize; blocks.len()];
    let (mut bt, mut bm) = (0, 0);
    for (item, size) in &items {
        at = at.next_multiple_of(16);
        match *item {
            BLOCK_TABLE => bt = at,
            BLOCK_MAP => bm = at,
            i => placed[i] = at,
        }
        at += size;
    }
    let phase_a = at.max(bm + 16);
    let classes = if variant { &CLASSES[..] } else { &CLASSES[..4] };
    let enums = if variant { &ENUMS[..] } else { &ENUMS[..0] };
    let ct = phase_a.next_multiple_of(16);
    let mut members = Vec::new();
    let mut end = ct + 32 * classes.len();
    for c in classes {
        members.push(end);
        end += 16 * c.members.len();
    }
    let et = end;
    end += 24 * enums.len();
    let mut values = Vec::new();
    for e in enums {
        values.push(end);
        end += 16 * e.2.len();
    }
    let st = end;
    end += 4;
    let len = if end <= STREAM {
        end.next_multiple_of(16)
    } else {
        end.next_multiple_of(STREAM)
    };
    if len > 0x0FFF_FFFF {
        return Err(TextTableError::Unwritable(format!(
            "its {len} bytes pass what a pointer reaches"
        )));
    }
    let mut v = vec![0u8; len];
    let mut put = |at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    let vp = |at: usize| (VBASE + at as u64).to_le_bytes();
    put(0x08, &vp(bm));
    put(0x10, b"0DRP");
    put(0x14, &HEADER_WORD.to_le_bytes());
    let root = blocks
        .iter()
        .position(|b| b.0 == MAP)
        .map_or(0, |i| i as u32 + 1);
    put(0x20, &root.to_le_bytes());
    put(0x28, &vp(ct));
    if !enums.is_empty() {
        put(0x30, &vp(et));
    }
    put(0x38, &vp(bt));
    put(0x40, &vp(st));
    put(0x48, &(classes.len() as u16).to_le_bytes());
    put(0x4A, &(enums.len() as u16).to_le_bytes());
    put(0x4C, &(blocks.len() as u16).to_le_bytes());
    put(0x4E, &pages_before_schema(phase_a, len).to_le_bytes());
    for (i, ((kind, size), bytes)) in blocks.iter().zip(data).enumerate() {
        put(placed[i], bytes);
        let row = bt + 16 * i;
        put(row, &kind.to_le_bytes());
        put(row + 4, &(*size as u32).to_le_bytes());
        put(row + 8, &vp(placed[i]));
    }
    put(bm + 8, &[1]);
    for (i, c) in classes.iter().enumerate() {
        let r = ct + 32 * i;
        put(r, &c.name.to_le_bytes());
        put(r + 4, &c.signature.to_le_bytes());
        put(r + 8, &[c.flags, c.align]);
        put(r + 16, &vp(members[i]));
        put(r + 24, &c.size.to_le_bytes());
        put(r + 30, &(c.members.len() as u16).to_le_bytes());
        for (j, (name, offset, kind, b9, w10, w12)) in c.members.iter().enumerate() {
            let m = members[i] + 16 * j;
            put(m, &name.to_le_bytes());
            put(m + 4, &offset.to_le_bytes());
            put(m + 8, &[*kind, *b9]);
            put(m + 10, &w10.to_le_bytes());
            put(m + 12, &w12.to_le_bytes());
        }
    }
    for (i, (name, signature, names)) in enums.iter().enumerate() {
        let r = et + 24 * i;
        put(r, &name.to_le_bytes());
        put(r + 4, &signature.to_le_bytes());
        put(r + 8, &vp(values[i]));
        put(r + 16, &(names.len() as u32).to_le_bytes());
        for (j, value) in names.iter().enumerate() {
            put(values[i] + 16 * j, &(j as u64).to_le_bytes());
            put(values[i] + 16 * j + 8, &value.to_le_bytes());
        }
    }
    Ok(v)
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
