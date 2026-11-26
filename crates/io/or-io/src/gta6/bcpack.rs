//! BCPack, the console's entropy coder over BC blocks, which a `.ttpf` body of codec kind 1 to 7
//! is stored through: the kind is the BC number of the blocks.
//!
//! A body is one byte stream shared by 12 or 16 rANS lanes, each giving one symbol for every 8
//! decoded bytes. The lanes of every mode decode here; BC1's symbols are rebuilt into blocks
//! through its public tables, and BC3's, BC4's, BC5's, BC6H's and BC7's through tables the user
//! supplies ([`supplied`]) or, without them, through tables fitted on the user's machine.
//! BC1's public tables were fitted too, by their author, and nothing confirms them value for
//! value: a read through them alone is not called exact, and through tables the user supplies a
//! block that reads a value the two give differently is counted ([`Decode::unseconded`]).
//!
//! A block's endpoints are predicted from an earlier block, and a reference to before a body's
//! first block reads a 256-slot preset that is a constant of the coder and is in no stream: only
//! BC1's is in the tree, from a source OpenRage can take it from. The presets and index tables of
//! every mode are read from a file the user supplies ([`supplied`]); without one those of BC3,
//! BC4 and BC5, BC6H's and BC7's two presets and the BC1 slot with no value are fitted from the user's own
//! packs ([`calibrate`]); a body rebuilt through a fitted table is an estimate, and [`Decode`]
//! says so. That a file reads does not make it the coder's tables: [`Decode`] counts the blocks
//! that speak against the tables they went through, and [`Reading`] is what a read may be called.

mod bc1;
mod bc1_tables;
mod bc6h;
mod bptc;
pub mod calibrate;
#[cfg(feature = "made-up")]
pub mod made_up;
mod planes;
mod rans;
pub mod supplied;
mod tables;

pub use calibrate::{Checked, Standing, Tables};

/// Decoded bytes for each symbol of a lane, in every mode.
const ELEMENT_BYTES: u64 = 8;

/// Why a BCPack body does not decode.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BcPackError {
    #[error("codec kind {kind} is not a BCPack mode: the modes are 1 and 3 to 7")]
    NotAMode { kind: u8 },
    #[error(
        "the entropy layer of BC{kind} decodes, but {}, so the body does not decode",
        missing(*kind)
    )]
    NoBlockTables { kind: u8 },
    #[error("a decoded size of {size} bytes is not a whole, non-zero number of 8-byte elements")]
    Size { size: u64 },
    #[error("lane {lane}'s fragment header narrows its symbols below one bit")]
    Header { lane: usize },
    #[error(
        "the stream ends after {consumed} bytes with symbols still owed; a piece of the file may \
         be missing"
    )]
    Truncated { consumed: usize },
    #[error("lanes {lanes:?} do not end in a state the coder leaves a lane in")]
    EndState { lanes: Vec<usize> },
    #[error("the lanes read {consumed} bytes, and the {rest} after them are not all zero")]
    Trailing { consumed: usize, rest: usize },
    #[error("block {block} carries {symbol} in lane 1, which is zero in every measured body")]
    Lane1 { block: usize, symbol: u16 },
    #[error(
        "block {block} carries {first} and {second} in lane 15, which name no mode of its blocks"
    )]
    Signature {
        block: usize,
        first: u16,
        second: u16,
    },
    #[error(
        "block {block} is predicted from slot {slot} of the endpoints preset before a body's \
         first block, and no value is known for that slot: `openrage-cli ttpf tables <file>` \
         reads one from a tables file you supply, and `openrage-cli ttpf calibrate <folder>` \
         fits one on this machine from the packs of a folder"
    )]
    PresetNotKnown { block: usize, slot: usize },
}

/// What rebuilding the blocks of codec kind `kind` lacks.
fn missing(kind: u8) -> &'static str {
    match kind {
        4 | 5 => {
            "its blocks follow a 256-slot preset of endpoints, which every measured body reads, \
             and a table of index pairs, which OpenRage does not ship: `openrage-cli ttpf \
             tables <file>` reads them from a tables file you supply, and `openrage-cli ttpf \
             calibrate <folder>` fits them on this machine from the packs of a folder"
        }
        3 => {
            "its blocks follow a 256-slot preset of endpoints, which most measured bodies read, \
             and tables of index pairs and quadrants, which OpenRage does not ship: \
             `openrage-cli ttpf tables <file>` reads them from a tables file you supply, and \
             `openrage-cli ttpf calibrate <folder>` fits them on this machine from the packs of \
             a folder"
        }
        _ => {
            "its blocks follow two 256-slot presets, of endpoints and of indices, which every \
             measured body reads and OpenRage does not ship: `openrage-cli ttpf tables <file>` \
             reads them from a tables file you supply, and `openrage-cli ttpf calibrate <folder>` \
             fits them on this machine from the packs of a folder"
        }
    }
}

/// Each lane's symbol width in bits for codec kind `kind`, in stream order; `None` if the kind is no BCPack mode a pack uses.
pub fn lane_widths(kind: u8) -> Option<&'static [u8]> {
    Some(match kind {
        1 => &[8, 8, 5, 6, 5, 5, 6, 5, 8, 8, 8, 8],
        3 => &[4, 4, 8, 6, 6, 6, 6, 5, 6, 5, 8, 8],
        4 => &[8, 8, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6],
        5 => &[4, 4, 8, 8, 6, 6, 6, 6, 6, 6, 6, 6],
        6 => &[4, 4, 8, 8, 8, 5, 5, 5, 6, 6, 6, 8, 8, 8, 8, 5],
        7 => &[4, 4, 8, 8, 8, 8, 8, 7, 7, 5, 6, 8, 8, 8, 8, 5],
        _ => return None,
    })
}

/// Whether [`decode`] rebuilds blocks for codec kind `kind`.
pub fn decodes(kind: u8) -> bool {
    kind == 1
}

/// The symbols of each lane of a body of codec kind `kind` that decodes to `size` bytes: `size / 8` for each lane.
pub fn lanes(kind: u8, stored: &[u8], size: u64) -> Result<Vec<Vec<u16>>, BcPackError> {
    let widths = lane_widths(kind).ok_or(BcPackError::NotAMode { kind })?;
    let target = usize::try_from(size / ELEMENT_BYTES)
        .ok()
        .filter(|&n| n > 0 && size.is_multiple_of(ELEMENT_BYTES))
        .ok_or(BcPackError::Size { size })?;
    let alphabet_bits: Vec<u8> = widths.iter().map(|width| width - 1).collect();
    let mut bytes = stored.iter().copied();
    let (lanes, consumed) =
        rans::decode_lanes(|_| bytes.next(), &alphabet_bits, target, stored.len())?;
    if stored[consumed..].iter().any(|&b| b != 0) {
        return Err(BcPackError::Trailing {
            consumed,
            rest: stored.len() - consumed,
        });
    }
    Ok(lanes)
}

/// The `size` bytes of BC blocks a body of codec kind `kind` decodes to, in the order the body's row stores them.
pub fn decode(kind: u8, stored: &[u8], size: u64) -> Result<Vec<u8>, BcPackError> {
    let lanes = lanes(kind, stored, size)?;
    match kind {
        1 => bc1::blocks(&lanes),
        _ => Err(BcPackError::NoBlockTables { kind }),
    }
}

/// How a body's blocks were rebuilt: every count zero for a body that reads through no table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decode {
    /// Blocks rebuilt through tables fitted on this machine, which are an estimate.
    pub calibrated: u64,
    /// Blocks rebuilt through tables the user supplied, and the codec kinds they are of, a bit
    /// each.
    pub supplied: u64,
    pub kinds: u8,
    /// Blocks drawn through a preset slot or an index symbol no table gave a value.
    pub unknown: u64,
    /// BC6H and BC7 blocks that break a rule every encoder of their format keeps, which a table
    /// that is right leaves none of: an end a difference takes out of its base's range, an
    /// anchor's index with its top bit set, two ends that disagree on the p-bit they share.
    pub stray: u64,
    /// BC1 blocks only one table stands behind: every block rebuilt through the built-in tables
    /// alone, and of those rebuilt through tables the user supplied, each that reads a preset
    /// slot or a quadrant symbol the built-in tables give another value for, or none.
    pub unseconded: u64,
    /// Bodies stored through the coder that were not read, under the levels that were: a read
    /// stops at the first body that does not read, and gives the levels above it.
    pub unread: u64,
}

/// What a texture's read through the coder may be called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Every body of it read, through no table of the coder at all; or through tables the user
    /// supplied, on one of two grounds. BC6H and BC7: no block of the read breaks a rule of its format, and a whole
    /// check has held the file's tables of that format. BC1: every value the read took is one
    /// BC1's built-in tables give too.
    Exact,
    /// Through BC1's built-in tables alone, which nothing confirms value for value.
    BuiltIn,
    /// Through tables the user supplied, `blocks` of its BC1 blocks read a value BC1's built-in
    /// tables give otherwise, or not at all.
    Unseconded { blocks: u64 },
    /// An estimate through tables fitted on this machine.
    Calibrated,
    /// Through tables the user supplied, BC6H or BC7 blocks none of which breaks a rule, before
    /// a whole check has held the file's tables of their format: one texture reads few of a
    /// preset's entries, and a wrong entry it reads need not break a rule in it.
    Unchecked,
    /// Through tables the user supplied in a format none of whose rules a wrong table breaks
    /// and with no second table to hold them against, BC3, BC4 or BC5: nothing can confirm the
    /// read, whatever a check has held.
    Unruled,
    /// Through tables the user supplied, `disagree` of the texture's `blocks` coded blocks read
    /// a value no table gives or hold what no encoder writes.
    Disagrees { disagree: u64, blocks: u64 },
    /// Through a tables file a whole check, or a listing or an export of the texture's own
    /// pack, found `disagree` of `blocks` coded blocks such in: the file is not this coder's
    /// tables, for that pack at the least.
    NotTheCoders { disagree: u64, blocks: u64 },
    /// The levels read are not all of it: a body of it stored through the coder was not read,
    /// and nothing is said of the levels it holds.
    CodedUnread,
}

impl Decode {
    pub fn add(&mut self, other: Decode) {
        self.calibrated += other.calibrated;
        self.supplied += other.supplied;
        self.kinds |= other.kinds;
        self.unknown += other.unknown;
        self.stray += other.stray;
        self.unseconded += other.unseconded;
        self.unread += other.unread;
    }

    /// The blocks that speak against the tables they were rebuilt through: those through a value
    /// no table gives and those no encoder writes. A block that is both counts twice.
    pub fn disagree(&self) -> u64 {
        self.unknown + self.stray
    }

    /// What the read may be called, `standing` being how the tables it went through stand.
    ///
    /// ⚠ No texture is stored both coded and plain, so exact never means compared with the
    /// pack's own blocks, and a table that is wrong where nothing looks passes for right. The
    /// rules see an entry of a preset through the blocks that read it: whether it is in its
    /// place, a p-bit and an anchor's top bit, not each end value, and not an entry few blocks
    /// read. Two tables that agree may both be wrong.
    pub fn reading(&self, standing: Standing) -> Reading {
        if self.calibrated > 0 {
            return Reading::Calibrated;
        }
        if self.supplied == 0 {
            return match (self.unseconded, self.unread) {
                (0, 0) => Reading::Exact,
                (0, _) => Reading::CodedUnread,
                _ => Reading::BuiltIn,
            };
        }
        if let Some(Checked { disagree, blocks }) = standing.failed() {
            return Reading::NotTheCoders { disagree, blocks };
        }
        if self.disagree() > 0 {
            return Reading::Disagrees {
                disagree: self.disagree(),
                blocks: self.supplied,
            };
        }
        // A texture that read clean in a pack another texture of which did not.
        if let Some(Checked { disagree, blocks }) = standing.failed_in_pack() {
            return Reading::NotTheCoders { disagree, blocks };
        }
        if self.unseconded > 0 {
            return Reading::Unseconded {
                blocks: self.unseconded,
            };
        }
        let read = |kind: u8| self.kinds >> kind & 1 != 0;
        if (3..=5).any(read) {
            return Reading::Unruled;
        }
        match (6..=7).any(|kind| read(kind) && standing.held(kind).is_none()) {
            true => Reading::Unchecked,
            false if self.unread > 0 => Reading::CodedUnread,
            false => Reading::Exact,
        }
    }

    /// The share of the fitted blocks drawn through a slot or symbol that is not known, in percent.
    pub fn unknown_percent(&self) -> f64 {
        100.0 * self.unknown as f64 / self.calibrated.max(1) as f64
    }
}

/// [`decode`], and where the body does not decode exactly, its blocks through `tables`.
pub fn decode_with(
    kind: u8,
    stored: &[u8],
    size: u64,
    tables: Option<&Tables>,
) -> Result<(Vec<u8>, Decode), BcPackError> {
    decode_checked(kind, stored, size, tables, false)
}

/// [`decode_with`], a BC6H body's blocks held to the range of the signed format where `signed`.
///
/// A mode whose tables the user supplied reads through them alone, BC1 too, and is no estimate.
pub fn decode_checked(
    kind: u8,
    stored: &[u8],
    size: u64,
    tables: Option<&Tables>,
    signed: bool,
) -> Result<(Vec<u8>, Decode), BcPackError> {
    let lanes = lanes(kind, stored, size)?;
    let given = tables.filter(|tables| tables.supplied(kind));
    let refused = match (kind, given) {
        (_, Some(_)) => None,
        (1, None) => match bc1::blocks(&lanes) {
            Ok(blocks) => {
                let how = Decode {
                    unseconded: lanes[0].len() as u64,
                    ..Decode::default()
                };
                return Ok((blocks, how));
            }
            Err(refused) => Some(refused),
        },
        (_, None) => Some(BcPackError::NoBlockTables { kind }),
    };
    let lacks_a_table = matches!(
        refused,
        Some(BcPackError::NoBlockTables { .. } | BcPackError::PresetNotKnown { .. })
    );
    let covering = given.or(tables.filter(|tables| lacks_a_table && tables.covers(kind)));
    let refused = refused.unwrap_or(BcPackError::NoBlockTables { kind });
    let (Some(tables), Some(bytes)) = (covering, block_bytes(kind)) else {
        return Err(refused);
    };
    let rebuilt = match kind {
        6 => bc6h::blocks(&lanes, tables.hdr(), signed)?,
        7 => bptc::blocks(&lanes, tables.bptc())?,
        _ => planes::blocks(kind, &lanes, &tables.resolved(kind).ok_or(refused)?)?,
    };
    let blocks = (rebuilt.bytes.len() / bytes) as u64;
    let through = |counted: bool| if counted { blocks } else { 0 };
    let decode = Decode {
        calibrated: through(given.is_none()),
        supplied: through(given.is_some()),
        kinds: given.map_or(0, |_| 1 << kind),
        unknown: rebuilt.unknown as u64,
        stray: rebuilt.stray as u64,
        // Held against the built-in tables as the user's own file reads, at each read: where the
        // two differ is nowhere in the tree.
        unseconded: match (kind, given.and_then(|tables| tables.bc1_apart())) {
            (1, Some(apart)) => apart.blocks(&lanes) as u64,
            _ => 0,
        },
        unread: 0,
    };
    Ok((rebuilt.bytes, decode))
}

/// The bytes of a block of codec kind `kind`; `None` for a kind that is no mode.
fn block_bytes(kind: u8) -> Option<usize> {
    match kind {
        6 | 7 => Some(16),
        _ => planes::mode(kind).map(planes::Mode::block_bytes),
    }
}

/// Whether the body of codec kind 1 whose stream starts with `head` reads a preset slot with no built-in value.
///
/// Only a body's first blocks can, so `head` need hold only the stream's start.
pub fn bc1_reads_unknown_slot(head: &[u8], size: u64) -> bool {
    let reach = planes::bc1_unknown_reach();
    let (Some(widths), Some(mode)) = (lane_widths(1), planes::mode(1)) else {
        return false;
    };
    let Ok(target) = usize::try_from(size / ELEMENT_BYTES) else {
        return false;
    };
    let alphabet_bits: Vec<u8> = widths.iter().map(|width| width - 1).collect();
    let mut bytes = head.iter().copied();
    let decoded = rans::decode_lanes_until(
        |_| bytes.next(),
        &alphabet_bits,
        target,
        head.len(),
        reach.min(target),
    );
    decoded.is_ok_and(|(lanes, _)| {
        (planes::slots_read(mode, &lanes, reach).into_iter()).any(|slot| !planes::built_in(1, slot))
    })
}

#[cfg(any(test, feature = "made-up"))]
pub(super) mod tests;
