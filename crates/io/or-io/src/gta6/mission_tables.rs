//! The mission state tables of GTA VI's script archives. An inflated body there is a metadata
//! document in the table form, and a state table is one of its blocks: the pairs of a map of maps,
//! each 32 bytes, a state's key and the head of the state's own map, whose pairs are a run of
//! elements in another block. A table is found by the body's own
//! block table and type table, never by a scan. Read only: what orders the states is not proven.

use super::hash::name_hash32;

/// The bytes of a state record.
pub const RECORD: usize = 0x20;
/// The least bytes of a placed element: its key, a position, three angles, the key again and a link.
pub const PLACED: usize = 0x40;
/// The flag byte the type table states for the element type of an array.
const ARRAY_ELEMENT: u8 = 0x0C;
/// The one section order every measured table-form document has.
const SECTIONS: [&[u8; 4]; 7] = [
    b"PSIN", b"LSCH", b"LENM", b"LSNS", b"LGID", b"LRSC", b"CHKS",
];

/// Why bytes are not a table-form body, a state record or a placed element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MissionTableError {
    #[error("the bytes are not a metadata document in GTA VI's table form: {why}")]
    NotABody { why: &'static str },
    #[error("the body ends before the {what} at {at:#X} does")]
    Truncated { what: &'static str, at: usize },
    #[error("the bytes at {at:#X} are not a state record: {why}")]
    NotARecord { at: usize, why: &'static str },
    #[error("the bytes at {at:#X} are not a placed element: {why}")]
    NotAnElement { at: usize, why: &'static str },
}

/// The hash a state's name is keyed by: case is folded before it is hashed.
pub const fn state_key(name: &str) -> u32 {
    name_hash32(name.as_bytes())
}

fn words<const N: usize>(
    bytes: &[u8],
    at: usize,
    what: &'static str,
) -> Result<[u32; N], MissionTableError> {
    let slice = (at.checked_add(4 * N))
        .and_then(|end| bytes.get(at..end))
        .ok_or(MissionTableError::Truncated { what, at })?;
    Ok(std::array::from_fn(|k| {
        u32::from_le_bytes(std::array::from_fn(|b| slice[4 * k + b]))
    }))
}

/// One block of a body: a run of elements of one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    /// The hash of the type the block holds, or the number of an array's own element type.
    pub class: u32,
    /// Where the block starts in the body.
    pub offset: usize,
    pub size: usize,
    /// The bytes of one element and the type's flag byte, where the body's type table declares the type.
    pub element: Option<(usize, u8)>,
}

/// The blocks of a table-form body, in the order its block table lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub blocks: Vec<Block>,
}

/// One state of a table: its key and the run of elements it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRecord {
    /// [`state_key`] of the state's name.
    pub key: u32,
    /// The word at +8, the first of the 24-byte head of the state's own map: 1 in every record
    /// measured, as in every sorted map of pairs of the build. That it marks the pairs sorted is
    /// GTA V's reading of such a head, not proven here.
    pub word_08: u32,
    /// The index in [`Body::blocks`] of the block that holds the run and the bytes into it where
    /// the run starts; `None` for a state that owns nothing.
    pub run: Option<(usize, usize)>,
    /// The elements of the run.
    pub count: u16,
}

/// A state table: a block every element of which is a state record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTable {
    /// The index of the table's block in [`Body::blocks`].
    pub block: usize,
    /// Where the table starts in the body.
    pub offset: usize,
    pub records: Vec<StateRecord>,
}

/// An element that stands in the world, Z up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedElement {
    /// A hash that differs by state for one character: an archetype or a state, not a model's name.
    pub archetype: u32,
    pub position: [f32; 3],
    /// Three angles, within a turn either way in all but a few elements measured; their order is not proven.
    pub rotation: [f32; 3],
    /// The hash after the archetype's second copy, which names the character the element is for.
    pub link: u32,
}

/// What a state's run holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    /// Records of the state table at this index of the tables [`Body::tables`] gives.
    Table(usize),
    /// Elements that stand in the world, each with where it starts in the body.
    Placed(Vec<(usize, PlacedElement)>),
    /// Elements of another shape, of this many bytes each.
    Other { element: usize },
}

impl Body {
    /// The blocks `bytes` state, refused where they are not a table-form document.
    pub fn open(bytes: &[u8]) -> Result<Self, MissionTableError> {
        let no = |why| MissionTableError::NotABody { why };
        // A section is a tag, its length with the frame as a big-endian word, then its bytes.
        let mut sections = [(0usize, 0usize); 7];
        let mut at = 0usize;
        for (section, tag) in sections.iter_mut().zip(SECTIONS) {
            let frame = (bytes.get(at..at.saturating_add(8)))
                .ok_or(no("a section's frame is cut short"))?;
            if frame[..4] != tag[..] {
                return Err(no(
                    "its sections are not PSIN LSCH LENM LSNS LGID LRSC CHKS",
                ));
            }
            let size = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
            if size < 8 || size > bytes.len() - at {
                return Err(no("a section states a length outside the body"));
            }
            *section = (at + 8, at + size);
            at += size;
        }
        let [psin, lsch, _, _, _, lrsc, _] = sections;
        let counts = (bytes.get(lrsc.0..lrsc.1))
            .and_then(|head| head.get(0x38..0x3E))
            .ok_or(no("LRSC is shorter than the header that states the counts"))?;
        let classes = usize::from(u16::from_le_bytes([counts[0], counts[1]]));
        let count = usize::from(u16::from_le_bytes([counts[4], counts[5]]));
        let headers = (bytes.get(lsch.0..lsch.1))
            .and_then(|types| types.get(..32 * classes))
            .ok_or(no("LSCH is shorter than the types LRSC counts"))?;
        let types: Vec<(u32, usize, u8)> = (headers.as_chunks::<32>().0.iter())
            .map(|h| {
                let word = |at: usize| u32::from_le_bytes(std::array::from_fn(|k| h[at + k]));
                (word(0), word(24) as usize, h[8])
            })
            .collect();
        // Offsets in PSIN count from its frame, which starts the body, and a block starts on 16 bytes.
        let rows = (bytes.get(psin.0..psin.1))
            .and_then(|data| data.get(..16 * count))
            .ok_or(no("PSIN is shorter than the block table LRSC counts"))?;
        let mut at = psin.0 + rows.len();
        let mut blocks = Vec::with_capacity(count);
        for row in rows.as_chunks::<16>().0 {
            let word = |at: usize| u32::from_le_bytes(std::array::from_fn(|k| row[at + k]));
            let (class, size) = (word(0), word(4) as usize);
            let offset = at.next_multiple_of(16);
            if offset > psin.1 || size > psin.1 - offset {
                return Err(no("a block lies outside PSIN"));
            }
            at = offset + size;
            let element = (types.iter()).find(|t| t.0 == class).map(|t| (t.1, t.2));
            blocks.push(Block {
                class,
                offset,
                size,
                element,
            });
        }
        Ok(Self { blocks })
    }

    /// The indices of the blocks the type table declares as arrays of 32-byte elements.
    fn table_blocks(&self) -> impl Iterator<Item = usize> + '_ {
        (self.blocks.iter().enumerate())
            .filter(|(_, b)| b.element == Some((RECORD, ARRAY_ELEMENT)) && b.size > 0)
            .map(|(k, _)| k)
    }

    /// Whether the body declares a block of state records; no record is read.
    pub fn holds_tables(&self) -> bool {
        self.table_blocks().next().is_some()
    }

    /// The state record at `at` of `bytes`, refused where a word that is zero in every record is
    /// not, where its two counts differ, or where its run is not inside the block it names.
    pub fn read_record(&self, bytes: &[u8], at: usize) -> Result<StateRecord, MissionTableError> {
        let w: [u32; 8] = words(bytes, at, "state record")?;
        let no = |why| Err(MissionTableError::NotARecord { at, why });
        if w[1] != 0 || w[3] != 0 || w[7] != 0 {
            return no("a word at +4, +0xC or +0x1C is not zero");
        }
        let [count, capacity] = [w[6] as u16, (w[6] >> 16) as u16];
        if count != capacity {
            return no("the two counts at +0x18 differ");
        }
        // A pointer is a block's number from 1 in its low 12 bits and a byte offset from bit 20.
        let pointer = u64::from(w[4]) | (u64::from(w[5]) << 32);
        if (pointer >> 12) & 0xFF != 0 {
            return no("bits 12 to 19 of its pointer are not zero");
        }
        let run = match ((pointer & 0xFFF) as usize, count) {
            (0, 0) if pointer == 0 => None,
            (0, _) | (_, 0) => return no("its pointer and its count do not agree on an empty run"),
            (number, _) => {
                let offset = usize::try_from(pointer >> 20).unwrap_or(usize::MAX);
                let fits = self.blocks.get(number - 1).is_some_and(|block| {
                    let size = block.element.map_or(0, |e| e.0);
                    let end = (usize::from(count).saturating_mul(size)).saturating_add(offset);
                    size > 0 && offset.is_multiple_of(size) && end <= block.size
                });
                if !fits {
                    return no("its run is not whole elements inside the block its pointer names");
                }
                Some((number - 1, offset))
            }
        };
        Ok(StateRecord {
            key: w[0],
            word_08: w[2],
            run,
            count,
        })
    }

    /// Every state table of the body, refused where a block declared as one holds a row that is no record.
    pub fn tables(&self, bytes: &[u8]) -> Result<Vec<StateTable>, MissionTableError> {
        (self.table_blocks())
            .map(|k| -> Result<StateTable, MissionTableError> {
                let block = self.blocks[k];
                let records = (0..block.size / RECORD)
                    .map(|row| self.read_record(bytes, block.offset + row * RECORD))
                    .collect::<Result<_, _>>()?;
                Ok(StateTable {
                    block: k,
                    offset: block.offset,
                    records,
                })
            })
            .collect()
    }

    /// What the run of `record` holds, where `tables` is what [`Body::tables`] gave; `None` for a state that owns nothing.
    pub fn run(&self, bytes: &[u8], tables: &[StateTable], record: &StateRecord) -> Option<Run> {
        let (k, offset) = record.run?;
        if let Some(table) = tables.iter().position(|t| t.block == k) {
            return Some(Run::Table(table));
        }
        let block = self.blocks.get(k)?;
        let element = block.element?.0;
        let placed: Result<Vec<_>, _> = (0..usize::from(record.count))
            .map(|n| block.offset + offset + n * element)
            .map(|at| read_element(bytes, at, element).map(|e| (at, e)))
            .collect();
        Some(placed.map_or(Run::Other { element }, Run::Placed))
    }
}

/// The placed element of `size` bytes at `at`: a key alone in 16 bytes, a position and three angles
/// each closed by a copy of its first component, then the key again before the link.
pub fn read_element(
    bytes: &[u8],
    at: usize,
    size: usize,
) -> Result<PlacedElement, MissionTableError> {
    let no = |why| Err(MissionTableError::NotAnElement { at, why });
    if size < PLACED {
        return no("it is shorter than a placed element");
    }
    let w: [u32; 12] = words(bytes, at, "placed element")?;
    if w[1] != 0 || w[2] != 0 || w[3] != 0 {
        return no("a word at +4, +8 or +0xC is not zero");
    }
    if w[7] != w[4] || w[11] != w[8] {
        return no("a vector is not closed by a copy of its first component");
    }
    // The key's copy and the link lie at +0x30, or at +0x40 behind one more vector.
    let tail = [0x30usize, 0x40]
        .into_iter()
        .filter(|tail| tail + 8 <= size)
        .filter_map(|tail| words::<2>(bytes, at + tail, "placed element").ok())
        .find(|tail| tail[0] == w[0]);
    let Some([_, link]) = tail else {
        return no("its key is repeated at neither +0x30 nor +0x40");
    };
    let [position, rotation] = [4usize, 8].map(|k| [w[k], w[k + 1], w[k + 2]].map(f32::from_bits));
    if position.iter().chain(&rotation).any(|c| !c.is_finite()) {
        return no("its position or its rotation is not finite");
    }
    Ok(PlacedElement {
        archetype: w[0],
        position,
        rotation,
        link,
    })
}

#[cfg(test)]
#[path = "mission_tables_tests.rs"]
mod tests;
