//! BCPack, the console's entropy coder over BC blocks, which a `.ttpf` body of codec kind 1 to 7
//! is stored through: the kind is the BC number of the blocks.
//!
//! A body is one byte stream shared by 12 or 16 rANS lanes, each giving one symbol for every 8
//! decoded bytes. The lanes of every mode decode here; BC1's symbols are rebuilt into blocks
//! exactly, and BC3's, BC4's, BC5's, BC6H's and BC7's through tables fitted on the user's machine.
//!
//! A block's endpoints are predicted from an earlier block, and a reference to before a body's
//! first block reads a 256-slot preset that is a constant of the coder and is in no stream: only
//! BC1's is known from a source OpenRage can take it from. The presets and index tables of BC3,
//! BC4 and BC5, BC6H's and BC7's two presets and the BC1 slot with no value are fitted from the user's own
//! packs ([`calibrate`]); a body rebuilt through a fitted table is an estimate, and [`Decode`]
//! says so.

mod bc1;
mod bc1_tables;
mod bc6h;
mod bptc;
pub mod calibrate;
mod planes;
mod rans;
mod tables;

pub use calibrate::Tables;

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
         first block, and no value is known for that slot: `openrage-cli ttpf calibrate \
         <folder>` fits one on this machine from the packs of a folder"
    )]
    PresetNotKnown { block: usize, slot: usize },
}

/// What rebuilding the blocks of codec kind `kind` lacks.
fn missing(kind: u8) -> &'static str {
    match kind {
        4 | 5 => {
            "its blocks follow a 256-slot preset of endpoints, which every measured body reads, \
             and a table of index pairs, which no source gives: `openrage-cli ttpf calibrate \
             <folder>` fits them on this machine from the packs of a folder"
        }
        3 => {
            "its blocks follow a 256-slot preset of endpoints, which most measured bodies read, \
             and tables of index pairs and quadrants, which no source gives: `openrage-cli ttpf \
             calibrate <folder>` fits them on this machine from the packs of a folder"
        }
        _ => {
            "its blocks follow two 256-slot presets, of endpoints and of indices, which every \
             measured body reads and no source gives: `openrage-cli ttpf calibrate <folder>` fits \
             them on this machine from the packs of a folder"
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

/// How a body's blocks were rebuilt: both counts zero for a body that decodes exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Decode {
    /// Blocks rebuilt through tables fitted on this machine, which are an estimate.
    pub calibrated: u64,
    /// Those of them drawn through a preset slot or an index symbol the fit did not solve.
    pub unknown: u64,
}

impl Decode {
    pub fn add(&mut self, other: Decode) {
        self.calibrated += other.calibrated;
        self.unknown += other.unknown;
    }

    /// Whether nothing was rebuilt through a fitted table.
    pub fn is_exact(&self) -> bool {
        self.calibrated == 0
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
    let lanes = lanes(kind, stored, size)?;
    let exact = match kind {
        1 => bc1::blocks(&lanes),
        _ => Err(BcPackError::NoBlockTables { kind }),
    };
    let refused = match exact {
        Ok(blocks) => return Ok((blocks, Decode::default())),
        Err(refused) => refused,
    };
    let lacks_a_table = matches!(
        refused,
        BcPackError::NoBlockTables { .. } | BcPackError::PresetNotKnown { .. }
    );
    let covering = tables.filter(|tables| lacks_a_table && tables.covers(kind));
    if let (6 | 7, Some(tables)) = (kind, covering) {
        let rebuilt = match kind {
            6 => bc6h::blocks(&lanes, tables.hdr())?,
            _ => bptc::blocks(&lanes, tables.bptc())?,
        };
        let decode = Decode {
            calibrated: (rebuilt.bytes.len() / 16) as u64,
            unknown: rebuilt.unknown as u64,
        };
        return Ok((rebuilt.bytes, decode));
    }
    let fitted = covering.and_then(|tables| tables.of(kind));
    let (Some(fitted), Some(mode)) = (fitted, planes::mode(kind)) else {
        return Err(refused);
    };
    let rebuilt = planes::blocks(kind, &lanes, &fitted.with_built_in(kind))?;
    let decode = Decode {
        calibrated: (rebuilt.bytes.len() / mode.block_bytes()) as u64,
        unknown: rebuilt.unknown as u64,
    };
    Ok((rebuilt.bytes, decode))
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

#[cfg(test)]
pub(super) mod tests;
