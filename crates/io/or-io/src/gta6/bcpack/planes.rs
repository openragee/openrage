//! BC1, BC3, BC4 and BC5 blocks from their lanes, through tables that may be fitted on this machine.
//!
//! A block is one or two 8-byte halves. An alpha half is BC4's: two 8-bit endpoints and 16 3-bit
//! indices, two to a symbol of a 64-entry pair table. A colour half is BC1's: two 5:6:5 colours
//! and 16 2-bit indices, four to a symbol of a 256-entry quadrant table. Each endpoint is a
//! difference from the same endpoint of an earlier block, or of a slot of the mode's preset.

use super::bc1::{difference, distance};
use super::bc1_tables::{PRESET_ENDPOINTS, QUADRANT_INDICES};
use super::BcPackError;

/// A field or an index pattern no table holds.
pub(super) const UNKNOWN: u16 = u16::MAX;
/// A pair of indices no table holds.
pub(super) const UNKNOWN_PAIR: u8 = u8::MAX;
/// The most fields a block holds: BC3's two alpha endpoints and two colours.
pub(super) const FIELDS: usize = 8;
/// The pixel at the top left of each quadrant of a colour half, in the order its symbols come.
pub(super) const QUADRANTS: [(usize, usize); 4] = [(0, 0), (2, 0), (0, 2), (2, 2)];

/// What a half of a block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Half {
    Alpha,
    Colour,
}

/// One 8-byte half of a block, and the lanes that carry it.
pub(super) struct Plane {
    pub(super) half: Half,
    /// Where its 8 bytes start in the block.
    pub(super) at: usize,
    /// The first of its fields among the block's.
    pub(super) base: usize,
    /// The lane, and which of the block's symbols in it, that carries each field's difference: endpoint 0's, then endpoint 1's.
    pub(super) fields: &'static [(usize, usize)],
    /// The lane and symbol that names each pair or quadrant of indices, in raster order.
    pub(super) indices: &'static [(usize, usize)],
}

impl Plane {
    /// The width of its field `k`.
    pub(super) fn bits(&self, k: usize) -> u32 {
        match self.half {
            Half::Alpha => 8,
            Half::Colour => [5, 6, 5][k % 3],
        }
    }

    /// What a field with no reference is a difference from.
    pub(super) fn default(&self) -> u16 {
        match self.half {
            Half::Alpha => 128,
            Half::Colour => 0,
        }
    }

    /// The channels of a pixel it gives: one, or red, green and blue.
    pub(super) fn channels(&self) -> usize {
        match self.half {
            Half::Alpha => 1,
            Half::Colour => 3,
        }
    }
}

/// A mode's blocks: how many symbols of each lane one takes, and its halves.
pub(super) struct Mode {
    pub(super) elements: usize,
    pub(super) planes: &'static [Plane],
}

impl Mode {
    pub(super) fn fields(&self) -> usize {
        self.planes.iter().map(|plane| plane.fields.len()).sum()
    }

    pub(super) fn block_bytes(&self) -> usize {
        8 * self.elements
    }

    /// The channels of a pixel over every half.
    pub(super) fn channels(&self) -> usize {
        self.planes.iter().map(Plane::channels).sum()
    }
}

static BC1: Mode = Mode {
    elements: 1,
    planes: &[Plane {
        half: Half::Colour,
        at: 0,
        base: 0,
        fields: &[(2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0)],
        indices: &[(8, 0), (9, 0), (10, 0), (11, 0)],
    }],
};

static BC3: Mode = Mode {
    elements: 2,
    planes: &[
        Plane {
            half: Half::Alpha,
            at: 0,
            base: 0,
            fields: &[(2, 0), (2, 1)],
            indices: &[
                (3, 0),
                (3, 1),
                (4, 0),
                (4, 1),
                (5, 0),
                (5, 1),
                (6, 0),
                (6, 1),
            ],
        },
        Plane {
            half: Half::Colour,
            at: 8,
            base: 2,
            fields: &[(7, 0), (8, 0), (9, 0), (7, 1), (8, 1), (9, 1)],
            indices: &[(10, 0), (10, 1), (11, 0), (11, 1)],
        },
    ],
};

static BC4: Mode = Mode {
    elements: 1,
    planes: &[Plane {
        half: Half::Alpha,
        at: 0,
        base: 0,
        fields: &[(2, 0), (3, 0)],
        indices: &[
            (4, 0),
            (5, 0),
            (6, 0),
            (7, 0),
            (8, 0),
            (9, 0),
            (10, 0),
            (11, 0),
        ],
    }],
};

static BC5: Mode = Mode {
    elements: 2,
    planes: &[
        Plane {
            half: Half::Alpha,
            at: 0,
            base: 0,
            fields: &[(2, 0), (2, 1)],
            indices: &[
                (4, 0),
                (4, 1),
                (5, 0),
                (5, 1),
                (6, 0),
                (6, 1),
                (7, 0),
                (7, 1),
            ],
        },
        Plane {
            half: Half::Alpha,
            at: 8,
            base: 2,
            fields: &[(3, 0), (3, 1)],
            indices: &[
                (8, 0),
                (8, 1),
                (9, 0),
                (9, 1),
                (10, 0),
                (10, 1),
                (11, 0),
                (11, 1),
            ],
        },
    ],
};

/// The halves of codec kind `kind`'s blocks; `None` for BC6H and BC7, which are not made of them.
pub(super) fn mode(kind: u8) -> Option<&'static Mode> {
    match kind {
        1 => Some(&BC1),
        3 => Some(&BC3),
        4 => Some(&BC4),
        5 => Some(&BC5),
        _ => None,
    }
}

/// The fixed tables of one mode: `UNKNOWN` and `UNKNOWN_PAIR` where no value is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModeTables {
    /// The fields a reference to before a body's first block reads, by slot.
    pub(super) preset: [[u16; FIELDS]; 256],
    /// The two 3-bit indices a pair symbol names: the left pixel's, then the right's shifted left by 3.
    pub(super) pairs: [u8; 64],
    /// The four 2-bit indices a quadrant symbol names, the top left lowest, then the top right, the bottom left and the bottom right.
    pub(super) quadrants: [u16; 256],
}

impl ModeTables {
    pub(super) fn empty() -> Self {
        Self {
            preset: [[UNKNOWN; FIELDS]; 256],
            pairs: [UNKNOWN_PAIR; 64],
            quadrants: [UNKNOWN; 256],
        }
    }

    /// These tables under the values codec kind `kind` has built in, which a fitted value never replaces.
    pub(super) fn with_built_in(&self, kind: u8) -> Self {
        let mut all = self.clone();
        if kind == 1 {
            for (slot, built_in) in PRESET_ENDPOINTS.iter().enumerate() {
                if let Some(fields) = built_in {
                    for (k, &value) in fields.iter().enumerate() {
                        all.preset[slot][k] = value.into();
                    }
                }
            }
            for (symbol, entry) in QUADRANT_INDICES.iter().enumerate() {
                all.quadrants[symbol] =
                    (entry.iter().rev()).fold(0, |all, &i| all << 2 | u16::from(i & 3));
            }
        }
        all
    }
}

/// Whether codec kind `kind` has slot `slot` of its preset built in.
pub(super) fn built_in(kind: u8, slot: usize) -> bool {
    kind == 1 && PRESET_ENDPOINTS[slot].is_some()
}

/// The highest preset slot BC1 has no built-in value for: only a body's first blocks, as many, can read one.
pub(super) fn bc1_unknown_reach() -> usize {
    (PRESET_ENDPOINTS.iter())
        .rposition(Option::is_none)
        .unwrap_or(0)
}

/// Two 4-bit symbols as one 8-bit symbol, the first on the even bits.
pub(super) fn interleave(first: u16, second: u16) -> u16 {
    (0..4).fold(0, |all, bit| {
        all | ((first >> bit) & 1) << (2 * bit) | ((second >> bit) & 1) << (2 * bit + 1)
    })
}

/// How many blocks back block `block` of `lanes` refers.
fn back(mode: &Mode, lanes: &[Vec<u16>], block: usize) -> usize {
    match mode.elements {
        1 => distance(lanes[0][block]),
        _ => distance(interleave(lanes[0][2 * block], lanes[0][2 * block + 1])),
    }
}

/// The preset slots the first `blocks` blocks of `lanes` read.
pub(super) fn slots_read(mode: &Mode, lanes: &[Vec<u16>], blocks: usize) -> Vec<usize> {
    let blocks = blocks.min(lanes[0].len() / mode.elements);
    (0..blocks)
        .filter_map(|block| {
            let back = back(mode, lanes, block);
            (back > block).then(|| 256 + block - back)
        })
        .collect()
}

/// A body's fields as its lanes chain them.
pub(super) struct Chains {
    pub(super) blocks: usize,
    /// Each block's fields: the field itself where the block's root is `None`, else what it adds to the root slot's.
    pub(super) offsets: Vec<u16>,
    /// The preset slot each block's chain of references starts at.
    pub(super) roots: Vec<Option<u8>>,
}

impl Chains {
    /// Field `field` of block `block`, `bits` wide, through `preset`; `None` where its root slot's value is not known.
    pub(super) fn field(
        &self,
        mode: &Mode,
        block: usize,
        field: usize,
        bits: u32,
        preset: &[[u16; FIELDS]; 256],
    ) -> Option<u16> {
        let offset = self.offsets[block * mode.fields() + field];
        match self.roots[block] {
            None => Some(offset),
            Some(slot) => {
                let base = preset[usize::from(slot)][field];
                (base != UNKNOWN).then(|| (base + offset) & ((1 << bits) - 1))
            }
        }
    }
}

/// The chains of `lanes`, one symbol of each lane for each 8 bytes of a block.
pub(super) fn chains(mode: &Mode, lanes: &[Vec<u16>]) -> Result<Chains, BcPackError> {
    let per = mode.elements;
    let symbols = lanes[0].len();
    if !symbols.is_multiple_of(per) {
        return Err(BcPackError::Size {
            size: 8 * symbols as u64,
        });
    }
    let (blocks, fields) = (symbols / per, mode.fields());
    let mut offsets = vec![0u16; blocks * fields];
    let mut roots: Vec<Option<u8>> = Vec::with_capacity(blocks);
    for block in 0..blocks {
        if let Some(&symbol) = lanes[1][per * block..per * (block + 1)]
            .iter()
            .find(|&&symbol| symbol != 0)
        {
            return Err(BcPackError::Lane1 { block, symbol });
        }
        let back = back(mode, lanes, block);
        // A distance is at most 255, so a slot is 1 to 255.
        let root = if back > block {
            Some((256 + block - back) as u8)
        } else if back > 0 {
            roots[block - back]
        } else {
            None
        };
        roots.push(root);
        for plane in mode.planes {
            for (k, &(lane, sub)) in plane.fields.iter().enumerate() {
                let field = plane.base + k;
                let predicted = if back == 0 {
                    i32::from(plane.default())
                } else if back <= block {
                    i32::from(offsets[(block - back) * fields + field])
                } else {
                    0
                };
                let mask = (1i32 << plane.bits(k)) - 1;
                let moved = predicted + difference(lanes[lane][per * block + sub]);
                offsets[block * fields + field] = (moved & mask) as u16;
            }
        }
    }
    Ok(Chains {
        blocks,
        offsets,
        roots,
    })
}

/// The 16 indices of `plane` in block `block`, in raster order; `None` where a symbol's entry is not known.
pub(super) fn indices(
    mode: &Mode,
    plane: &Plane,
    lanes: &[Vec<u16>],
    block: usize,
    tables: &ModeTables,
) -> [Option<u8>; 16] {
    let mut out = [None; 16];
    for (k, &(lane, sub)) in plane.indices.iter().enumerate() {
        let symbol = usize::from(lanes[lane][mode.elements * block + sub]);
        match plane.half {
            Half::Alpha => {
                let pair = tables.pairs[symbol & 63];
                if pair != UNKNOWN_PAIR {
                    out[2 * k] = Some(pair & 7);
                    out[2 * k + 1] = Some((pair >> 3) & 7);
                }
            }
            Half::Colour => {
                let entry = tables.quadrants[symbol & 255];
                if entry != UNKNOWN {
                    let (left, top) = QUADRANTS[k];
                    for j in 0..4 {
                        out[4 * (top + j / 2) + left + j % 2] =
                            Some(((entry >> (2 * j)) & 3) as u8);
                    }
                }
            }
        }
    }
    out
}

/// A body's blocks, and how many were drawn through a preset slot or an index symbol no table knows.
pub(super) struct Rebuilt {
    pub(super) bytes: Vec<u8>,
    pub(super) unknown: usize,
}

/// The value of the solved slot nearest `slot` for `field`, which stands in for one that is not known.
fn nearest(preset: &[[u16; FIELDS]; 256], slot: usize, field: usize) -> Option<u16> {
    (1..256).find_map(|away| {
        let known = |at: Option<usize>| {
            at.filter(|&at| at < 256)
                .map(|at| preset[at][field])
                .filter(|&value| value != UNKNOWN)
        };
        known(slot.checked_sub(away)).or_else(|| known(Some(slot + away)))
    })
}

/// The blocks of codec kind `kind` that `lanes` rebuild through `tables`, the mode's built-in values included.
///
/// A block whose preset slot or index symbol is not known is drawn from the nearest solved slot,
/// or with index 0, and counted.
pub(super) fn blocks(
    kind: u8,
    lanes: &[Vec<u16>],
    tables: &ModeTables,
) -> Result<Rebuilt, BcPackError> {
    let mode = mode(kind).ok_or(BcPackError::NoBlockTables { kind })?;
    let chains = chains(mode, lanes)?;
    let size = mode.block_bytes();
    let mut bytes = vec![0u8; chains.blocks * size];
    let mut unknown = 0;
    for block in 0..chains.blocks {
        let mut known = true;
        for plane in mode.planes {
            let mut values = [0u16; 6];
            for (k, value) in values.iter_mut().enumerate().take(plane.fields.len()) {
                let (field, bits) = (plane.base + k, plane.bits(k));
                *value = match chains.field(mode, block, field, bits, &tables.preset) {
                    Some(value) => value,
                    None => {
                        known = false;
                        let slot = chains.roots[block].map_or(0, usize::from);
                        let stand_in =
                            nearest(&tables.preset, slot, field).unwrap_or(plane.default());
                        let offset = chains.offsets[block * mode.fields() + field];
                        (stand_in + offset) & ((1 << bits) - 1)
                    }
                };
            }
            let indices = indices(mode, plane, lanes, block, tables);
            known &= indices.iter().all(Option::is_some);
            let out = &mut bytes[block * size + plane.at..][..8];
            match plane.half {
                Half::Alpha => {
                    out[0] = values[0] as u8;
                    out[1] = values[1] as u8;
                    let word = (indices.iter().enumerate()).fold(0u64, |word, (pixel, index)| {
                        word | u64::from(index.unwrap_or(0)) << (3 * pixel)
                    });
                    out[2..].copy_from_slice(&word.to_le_bytes()[..6]);
                }
                Half::Colour => {
                    let colour = |fields: &[u16]| fields[0] << 11 | fields[1] << 5 | fields[2];
                    out[..2].copy_from_slice(&colour(&values[..3]).to_le_bytes());
                    out[2..4].copy_from_slice(&colour(&values[3..]).to_le_bytes());
                    let word = (indices.iter().enumerate()).fold(0u32, |word, (pixel, index)| {
                        word | u32::from(index.unwrap_or(0)) << (2 * pixel)
                    });
                    out[4..].copy_from_slice(&word.to_le_bytes());
                }
            }
        }
        unknown += usize::from(!known);
    }
    Ok(Rebuilt { bytes, unknown })
}
