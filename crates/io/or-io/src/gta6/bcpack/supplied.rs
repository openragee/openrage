//! The coder's fixed tables read from a file the user supplies.
//!
//! The presets and the index tables are constants of the console's coder: no stream holds them,
//! and OpenRage ships none but BC1's public ones. [`read`] takes a file that holds them into the
//! shape the decoders here hold a fitted table in. A mode read through them is no estimate.
//!
//! What the decoders here take from a file:
//!
//! - **A preset list of 255 entries** for each of BC1, BC3, BC4, BC5, BC6H and BC7. Entry `e` is
//!   slot `255 - e`; slot 0, which no body reads, has no entry.
//! - **An index table** for BC1 (256 quadrant symbols), for BC3 (64 pair symbols of its alpha and
//!   256 quadrant symbols of its colour), for BC4 and for BC5 (64 pair symbols each). A symbol
//!   holds places between a block's two ends, not the codes a block stores.
//! - **BC7's table of lane 15's pairs**, 512 words, which has to count as the walk here counts.
//! - **128 entropy models** of 9 frequencies each, held to the entropy layer's own and otherwise
//!   not read: a file of another coder stops there.
//!
//! In a file they lie in the order of [`LENGTHS`] with nothing before, between or after them,
//! and a 16-bit value is little-endian.
//!
//! ⚠ Three things a file holds are read by no rule here, because no measured body needs them:
//! the index part of an entry of BC1, BC3, BC4 and BC5 (lane 1, which would name the block a
//! body's indices are differences from, is zero in every such body, and one where it is not is
//! refused); the first byte of a BC6H entry; and the second index set of a BC7 entry.

use super::bc6h;
use super::bptc::{self, Sig, UNPLACED};
use super::calibrate::Tables;
use super::tables::FREQ_TABLE;
use crate::gta6::hash::bytes_hash64;

/// The entries of a preset list: slots 255 down to 1.
const ENTRIES: usize = 255;
/// The bytes of an entry of each preset list, in the order the lists lie: BC1, BC3, BC4, BC5, BC6H, BC7.
const ENTRY: [usize; 6] = [22, 40, 18, 36, 58, 116];
/// The bytes of each table of a file, in the order they lie: [`read`] takes each by its number.
const LENGTHS: [usize; 13] = [
    8 * 16 * 18,
    256,
    64,
    256,
    64,
    64,
    2 * bptc::RANKS,
    ENTRIES * ENTRY[0],
    ENTRIES * ENTRY[1],
    ENTRIES * ENTRY[2],
    ENTRIES * ENTRY[3],
    ENTRIES * ENTRY[4],
    ENTRIES * ENTRY[5],
];
/// The codec kinds a file gives every table of.
const KINDS: [u8; 6] = [1, 3, 4, 5, 6, 7];

/// The length of a tables file.
pub fn length() -> usize {
    LENGTHS.iter().sum()
}

/// Where table `number` of a file starts.
fn start(number: usize) -> usize {
    LENGTHS[..number].iter().sum()
}

/// Table `number` of `bytes`, a file of the right length.
fn table(bytes: &[u8], number: usize) -> &[u8] {
    &bytes[start(number)..start(number + 1)]
}

/// Why a file is not the coder's tables.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SuppliedError {
    #[error("a tables file is {} bytes, and this one is {len}", length())]
    Length { len: usize },
    #[error(
        "entropy model {model} is not the one the entropy layer here reads a stream with, so \
         the file is of another coder"
    )]
    Model { model: usize },
    #[error("entry {entry} of {table} holds {value:#x}, which is {why}")]
    Value {
        table: &'static str,
        entry: usize,
        value: u32,
        why: &'static str,
    },
}

fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// The code a block stores for place `place` between its two ends, `last` being its second
/// end's place: by BC1's and BC4's formats the ends are codes 0 and 1, and the values between
/// them follow from code 2.
fn code(place: u8, last: u8) -> u8 {
    match place {
        0 => 0,
        place if place == last => 1,
        between => between + 1,
    }
}

/// A quadrant table as a walk holds it. A symbol's byte says, 2 bits each from the top left, where
/// each pixel of a 2 by 2 quadrant lies between a block's two colours.
fn quadrants(table: &[u8]) -> [u16; 256] {
    std::array::from_fn(|symbol| {
        (0..4)
            .map(|pixel| u16::from(code(table[symbol] >> (2 * pixel) & 3, 3)) << (2 * pixel))
            .sum()
    })
}

/// A pair table as a walk holds it. A symbol's byte says where a pair's first pixel lies between
/// a block's two ends in bits 0 to 2, and its second in bits 4 to 6.
fn pairs(table: &[u8], name: &'static str) -> Result<[u8; 64], SuppliedError> {
    let mut out = [0u8; 64];
    for (symbol, (pair, &places)) in out.iter_mut().zip(table).enumerate() {
        if places & 0x88 != 0 {
            return Err(SuppliedError::Value {
                table: name,
                entry: symbol,
                value: places.into(),
                why: "more than two places of 3 bits",
            });
        }
        *pair = code(places & 7, 7) | code(places >> 4, 7) << 3;
    }
    Ok(out)
}

/// What each pair of lane 15's symbols names in BC7, as the walk here holds it: each rank's mode,
/// and the partition of each place among the ranks of modes 0 and 2.
///
/// A word is indexed by the second symbol shifted left by 5 and the first, and holds the mode in
/// bits 0 to 2, the rotation in 3 and 4, the index selection in 5, the subsets in 6 and 7 and
/// the partition in 8 to 13; zero for a pair no block carries. The walk counts a mode's
/// rotations, selections and, in modes 1, 3 and 7, partitions along its ranks, so a file that
/// says otherwise names blocks the walk would not rebuild, and is refused.
fn codes(table: &[u8], presets: &mut bptc::Presets) -> Result<(), SuppliedError> {
    let named = || {
        (0..32u16)
            .flat_map(|first| (0..16u16).map(move |second| (first, second)))
            .map(|(first, second)| {
                (
                    first,
                    second,
                    word(table, 2 * usize::from(second << 5 | first)),
                )
            })
            .filter(|&(_, _, word)| word != 0)
    };
    for (first, second, word) in named() {
        if let Some(rank) = bptc::rank(first, second) {
            presets.modes[rank] = (word & 7) as u8;
        }
    }
    for (first, second, word) in named() {
        let refuse = |why| SuppliedError::Value {
            table: "BC7's table of lane 15's pairs",
            entry: usize::from(second << 5 | first),
            value: word.into(),
            why,
        };
        let sig = Sig::of(first, second, &presets.modes)
            .ok_or_else(|| refuse("one pair more of its mode than the mode has blocks to name"))?;
        let (rotation, selection) = (usize::from(word >> 3 & 3), usize::from(word >> 5 & 1));
        let (subsets, partition) = (usize::from(word >> 6 & 3), (word >> 8 & 63) as u8);
        let counted = word >> 14 == 0
            && (rotation, selection, subsets) == (sig.rotation(), sig.selection(), sig.subsets());
        if !counted {
            return Err(refuse(
                "not the rotation, selection and subsets its place among its mode's pairs has",
            ));
        }
        let down = (63 - sig.position.min(63)) as u8;
        match sig.mode {
            0 if partition < 16 => presets.order0[sig.position] = partition,
            2 => presets.order2[sig.position] = partition,
            1 | 3 | 7 if partition == down => {}
            4..=6 if partition == 0 => {}
            _ => {
                return Err(refuse(
                    "not a partition its place among its mode's pairs has",
                ))
            }
        }
    }
    let whole = |order: &[u8]| (0..order.len() as u8).all(|partition| order.contains(&partition));
    if !whole(&presets.order0) || !whole(&presets.order2) {
        return Err(SuppliedError::Value {
            table: "BC7's table of lane 15's pairs",
            entry: 0,
            value: u32::from(UNPLACED),
            why: "a table that does not name each partition of modes 0 and 2 once",
        });
    }
    Ok(())
}

/// The six fields of a colour half's two ends, each at its own width: red, green and blue of the first, then of the second.
fn colour(
    to: &mut [u16],
    from: &[u8],
    table: &'static str,
    entry: usize,
) -> Result<(), SuppliedError> {
    for (k, (field, &value)) in to.iter_mut().zip(from).enumerate() {
        if u32::from(value) >> [5, 6, 5][k % 3] != 0 {
            return Err(SuppliedError::Value {
                table,
                entry,
                value: value.into(),
                why: "wider than its field of a 5:6:5 colour",
            });
        }
        *field = value.into();
    }
    Ok(())
}

/// The two ends of each alpha half. A file counts an end from 128, where an end with no
/// reference starts (`Plane::default`), modulo its 8 bits.
fn alpha(to: &mut [u16], from: &[u8]) {
    for (field, &value) in to.iter_mut().zip(from) {
        *field = u16::from(value.wrapping_add(128));
    }
}

/// A signed index of `depth` bits, left-aligned in 4 bits as a walk holds it.
fn index(
    bytes: &[u8],
    at: usize,
    depth: u32,
    (table, entry): (&'static str, usize),
) -> Result<u8, SuppliedError> {
    let value = word(bytes, at) as i16;
    let half = 1i16 << (depth - 1);
    if !(-half..half).contains(&value) {
        return Err(SuppliedError::Value {
            table,
            entry,
            value: (value as u16).into(),
            why: "an index wider than its entry says its indices are",
        });
    }
    Ok(((value << (4 - depth)) & 0xF) as u8)
}

/// BC6H's presets. An entry is a byte no rule here reads; twelve 16-bit fields, red, green and
/// blue of each of four ends, each left-aligned and counted from where a field with no
/// reference starts ([`bc6h::HALF`]); the mode, counted from 1, whose index depth the entry's
/// indices have; and sixteen signed 16-bit indices, each counted from half its range, as an
/// index with no reference is.
fn presets_hdr(list: &[u8], presets: &mut bc6h::Presets) -> Result<(), SuppliedError> {
    const TABLE: &str = "BC6H's presets";
    for (entry, bytes) in list.as_chunks::<{ ENTRY[4] }>().0.iter().enumerate() {
        let slot = ENTRIES - entry;
        for (field, value) in presets.ends[slot].iter_mut().enumerate() {
            *value = word(bytes, 1 + 2 * field).wrapping_add(bc6h::HALF);
        }
        presets.ends_bits[slot] = [16; bc6h::FIELDS];
        let shape = (usize::from(bytes[25]).checked_sub(1))
            .and_then(|mode| bc6h::shapes().get(mode))
            .ok_or(SuppliedError::Value {
                table: TABLE,
                entry,
                value: bytes[25].into(),
                why: "no mode of a BC6H block",
            })?;
        let depth = shape.index_depth();
        for (pixel, value) in presets.first[slot].iter_mut().enumerate() {
            *value = (index(bytes, 26 + 2 * pixel, depth, (TABLE, entry))? + 8) & 0xF;
        }
        presets.first_known[slot] = true;
    }
    Ok(())
}

/// BC7's presets. An entry is a word of the form of the table of lane 15's pairs, of which the
/// subsets are read; three subsets' eight 16-bit values, the two ends of red, green, blue and
/// alpha, each left-aligned; the depths of its two index sets; and sixteen signed 16-bit
/// indices of each set. A subset past those the entry has takes the first one's values, as a
/// block does in a body.
///
/// ⚠ How a second set reads is not measured: an entry whose second depth is not zero is refused,
/// and every second index reads as zero.
fn presets_bptc(list: &[u8], presets: &mut bptc::Presets) -> Result<(), SuppliedError> {
    const TABLE: &str = "BC7's presets";
    for (entry, bytes) in list.as_chunks::<{ ENTRY[5] }>().0.iter().enumerate() {
        let slot = ENTRIES - entry;
        let refuse = |value: u16, why| SuppliedError::Value {
            table: TABLE,
            entry,
            value: value.into(),
            why,
        };
        let own = 8 * usize::from(word(bytes, 0) >> 6 & 3).max(1);
        for (at, value) in presets.ends[slot].iter_mut().enumerate() {
            let held = word(bytes, 2 + 2 * if at < own { at } else { at % 8 });
            if held & 0xFF != 0 {
                return Err(refuse(held, "an end of more than 8 bits"));
            }
            *value = (held >> 8) as u8;
        }
        presets.ends_known[slot] = [true; bptc::VALUES];
        let depth = u32::from(bytes[50]);
        if !(2..=4).contains(&depth) {
            return Err(refuse(bytes[50].into(), "no depth of a BC7 index"));
        }
        for (pixel, value) in presets.first[slot].iter_mut().enumerate() {
            *value = index(bytes, 52 + 2 * pixel, depth, (TABLE, entry))?;
        }
        if bytes[51] != 0 {
            return Err(refuse(
                bytes[51].into(),
                "a second index set, which no rule here reads",
            ));
        }
        presets.first_known[slot] = true;
        presets.second_known[slot] = true;
    }
    Ok(())
}

/// The tables `bytes`, a tables file, holds, each of its modes marked as supplied.
pub fn read(bytes: &[u8]) -> Result<Tables, SuppliedError> {
    if bytes.len() != length() {
        return Err(SuppliedError::Length { len: bytes.len() });
    }
    // A model is 9 frequencies: symbol 0's, then that of each symbol of 2^(k-1) to 2^k - 1.
    for (model, held) in table(bytes, 0).as_chunks::<18>().0.iter().enumerate() {
        let built_in = &FREQ_TABLE[model / 16][model % 16];
        if (0..9).any(|k| word(held, 2 * k) != built_in[k]) {
            return Err(SuppliedError::Model { model });
        }
    }
    let mut tables = Tables::empty();
    let at = |kind: u8| Tables::at(kind).unwrap_or(0);
    tables.modes[at(1)].quadrants = quadrants(table(bytes, 1));
    tables.modes[at(3)].pairs = pairs(table(bytes, 2), "BC3's table of alpha pairs")?;
    tables.modes[at(3)].quadrants = quadrants(table(bytes, 3));
    tables.modes[at(4)].pairs = pairs(table(bytes, 4), "BC4's table of pairs")?;
    tables.modes[at(5)].pairs = pairs(table(bytes, 5), "BC5's table of pairs")?;
    codes(table(bytes, 6), &mut tables.bptc)?;
    // An entry of a mode of halves is its ends, then its indices, which no rule here reads.
    for (entry, held) in table(bytes, 7)
        .as_chunks::<{ ENTRY[0] }>()
        .0
        .iter()
        .enumerate()
    {
        let fields = &mut tables.modes[at(1)].preset[ENTRIES - entry];
        colour(&mut fields[..6], &held[..6], "BC1's presets", entry)?;
    }
    // BC3's colour comes before its alpha in an entry, and after it among a block's fields.
    for (entry, held) in table(bytes, 8)
        .as_chunks::<{ ENTRY[1] }>()
        .0
        .iter()
        .enumerate()
    {
        let fields = &mut tables.modes[at(3)].preset[ENTRIES - entry];
        colour(&mut fields[2..8], &held[..6], "BC3's presets", entry)?;
        alpha(&mut fields[..2], &held[6..8]);
    }
    for (entry, held) in table(bytes, 9)
        .as_chunks::<{ ENTRY[2] }>()
        .0
        .iter()
        .enumerate()
    {
        alpha(
            &mut tables.modes[at(4)].preset[ENTRIES - entry][..2],
            &held[..2],
        );
    }
    for (entry, held) in table(bytes, 10)
        .as_chunks::<{ ENTRY[3] }>()
        .0
        .iter()
        .enumerate()
    {
        alpha(
            &mut tables.modes[at(5)].preset[ENTRIES - entry][..4],
            &held[..4],
        );
    }
    presets_hdr(table(bytes, 11), &mut tables.hdr)?;
    presets_bptc(table(bytes, 12), &mut tables.bptc)?;
    tables.supplied = KINDS.iter().fold(0, |all, kind| all | 1 << kind);
    tables.digest = bytes_hash64(bytes);
    Ok(tables)
}

#[cfg(any(test, feature = "made-up"))]
#[path = "supplied_tests.rs"]
pub(in crate::gta6) mod tests;
