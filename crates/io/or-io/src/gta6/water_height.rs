//! The ground under GTA VI's water: the height map a [`GridAtlas`] tiles, its global fallback and
//! the tiles laid on it by their content, since the archive that names them is ciphered.

use std::collections::HashMap;

use super::{GridAtlas, WaterError};
use crate::codec::texture::DxgiFormat;
use crate::gta6::ttpf::Texture;

/// The metres one step of a texel's value stands for, and the height its value 0 stands at.
const METRES_A_STEP: f32 = 1600.0 / 65536.0;
const LOWEST: f32 = -800.0;

/// The height in metres a texel's value `v` stands for.
pub fn height(v: u16) -> f32 {
    f32::from(v) * METRES_A_STEP + LOWEST
}

/// A height texture's values, row by row from its north edge, each row west to east.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heights {
    width: u32,
    height: u32,
    values: Vec<u16>,
}

impl Heights {
    /// `values` as `width` by `height` texels; `None` where they are not that many.
    pub fn new(width: u32, height: u32, values: Vec<u16>) -> Option<Self> {
        (values.len() as u64 == u64::from(width) * u64::from(height) && width > 0 && height > 0)
            .then_some(Self {
                width,
                height,
                values,
            })
    }

    /// The top level of an R16 texture; `None` for another format or too few bytes.
    pub fn of(texture: &Texture) -> Option<Self> {
        if texture.format.dxgi() != DxgiFormat::R16Unorm {
            return None;
        }
        let count = texture.width as usize * texture.height as usize;
        let bytes = texture.pixels.get(..count.checked_mul(2)?)?;
        let values = (bytes.as_chunks::<2>().0.iter())
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        Self::new(texture.width, texture.height, values)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The value at column `i`, row `j`; `None` off the texture.
    pub fn at(&self, i: u32, j: u32) -> Option<u16> {
        if i >= self.width || j >= self.height {
            return None;
        }
        self.values.get((j * self.width + i) as usize).copied()
    }

    /// The mean of each box of a `side` by `side` split, rounded; `None` where `side` divides neither side evenly.
    pub fn shrunk(&self, side: u32) -> Option<Self> {
        if side == 0 || !self.width.is_multiple_of(side) || !self.height.is_multiple_of(side) {
            return None;
        }
        let (across, down) = (self.width / side, self.height / side);
        let mut sums = vec![0u64; (side * side) as usize];
        for (k, v) in self.values.iter().enumerate() {
            let (i, j) = (k as u32 % self.width, k as u32 / self.width);
            sums[((j / down) * side + i / across) as usize] += u64::from(*v);
        }
        let n = u64::from(across) * u64::from(down);
        let values = sums.iter().map(|s| ((s + n / 2) / n) as u16).collect();
        Self::new(side, side, values)
    }

    fn is_flat(&self) -> bool {
        self.values.windows(2).all(|w| w[0] == w[1])
    }
}

/// A tile laid on the grid, and the mean difference to the fallback block it was laid on.
#[derive(Debug, Clone, PartialEq)]
struct Laid {
    error: u64,
    heights: Heights,
}

/// The height map of the ground under the water, as metres at a place.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterGround {
    /// The grid's west and north edges, in metres.
    west: f32,
    north: f32,
    tile_size: f32,
    /// The tiles of the grid along x and along y.
    tiles_across: [u32; 2],
    /// The fallback's texels a tile's side spans.
    block: u32,
    fallback: Heights,
    /// The sum of the fallback's texels in each block, row by row.
    sums: Vec<u64>,
    tiles: HashMap<(u32, u32), Laid>,
}

impl WaterGround {
    /// The ground of the grid `atlas` with its global fallback `fallback` and no tile yet.
    pub fn new(atlas: &GridAtlas, fallback: Heights) -> Result<Self, WaterError> {
        let refused = |why: String| WaterError::Ground(why);
        let tile_size = atlas.tile_size;
        if !(tile_size > 0.0 && atlas.size[0] > 0.0 && atlas.size[1] > 0.0) {
            return Err(refused(format!(
                "a grid of {:?} metres in tiles of {tile_size}",
                atlas.size
            )));
        }
        let tiles_across = atlas.size.map(|s| (s / tile_size).round() as u32);
        let block = fallback.width / tiles_across[0].max(1);
        // ⚠ The tiles across come from the document: a product of them is checked, never trusted.
        // ⚠ The tiles across come from the document: their product with the block is checked.
        let fits = block > 0
            && Some(fallback.width) == tiles_across[0].checked_mul(block)
            && Some(fallback.height) == tiles_across[1].checked_mul(block);
        if !fits {
            return Err(refused(format!(
                "a fallback of {} by {} texels over {} by {} tiles",
                fallback.width, fallback.height, tiles_across[0], tiles_across[1]
            )));
        }
        let mut sums = vec![0u64; (tiles_across[0] * tiles_across[1]) as usize];
        for (k, v) in fallback.values.iter().enumerate() {
            let (i, j) = (k as u32 % fallback.width, k as u32 / fallback.width);
            sums[((j / block) * tiles_across[0] + i / block) as usize] += u64::from(*v);
        }
        Ok(Self {
            west: atlas.west(),
            north: atlas.north(),
            tile_size,
            tiles_across,
            block,
            fallback,
            sums,
            tiles: HashMap::new(),
        })
    }

    /// Lay `tile` on the fallback block whose texels differ least from its own means; where it was laid, which is `None` for a tile of one value (the fallback is the same there), one whose sides the block does not divide, or one a closer tile holds that block against.
    pub fn add_tile(&mut self, tile: Heights) -> Option<(u32, u32)> {
        if tile.is_flat() {
            return None;
        }
        let small = tile.shrunk(self.block)?;
        let own: u64 = small.values.iter().map(|v| u64::from(*v)).sum();
        // The summed difference is at least the difference of the sums, so blocks are tried by
        // that, and none is tried once it reaches the best found.
        let mut order: Vec<(u64, u32, u32)> = (0..self.tiles_across[1])
            .flat_map(|row| (0..self.tiles_across[0]).map(move |col| (col, row)))
            .map(|(col, row)| {
                (
                    self.sums[(row * self.tiles_across[0] + col) as usize].abs_diff(own),
                    row,
                    col,
                )
            })
            .collect();
        order.sort_unstable();
        let mut best: Option<((u32, u32), u64)> = None;
        for (least, row, col) in order {
            let bound = best.map_or(u64::MAX, |(_, error)| error);
            if least >= bound {
                break;
            }
            if let Some(error) = self.difference(&small, col, row, bound) {
                best = Some(((col, row), error));
            }
        }
        let (at, error) = best?;
        if self.tiles.get(&at).is_some_and(|laid| laid.error <= error) {
            return None;
        }
        self.tiles.insert(
            at,
            Laid {
                error,
                heights: tile,
            },
        );
        Some(at)
    }

    /// The summed difference of `small` to the fallback block at `col`, `row`; `None` once it reaches `bound`.
    fn difference(&self, small: &Heights, col: u32, row: u32, bound: u64) -> Option<u64> {
        let mut sum = 0u64;
        for j in 0..self.block {
            for i in 0..self.block {
                let own = small.at(i, j)?;
                let under = (self.fallback).at(col * self.block + i, row * self.block + j)?;
                sum += u64::from(own.abs_diff(under));
            }
            if sum >= bound {
                return None;
            }
        }
        Some(sum)
    }

    /// The grid's west and north edges and its extent along x and y, in metres.
    pub fn extent(&self) -> (f32, f32, [f32; 2]) {
        let size = self.tiles_across.map(|n| n as f32 * self.tile_size);
        (self.west, self.north, size)
    }

    /// The tiles laid.
    pub fn tiles(&self) -> usize {
        self.tiles.len()
    }

    /// The height of the ground at `x`, `y`, in metres: a tile's where one is laid there, else the fallback's; `None` off the grid.
    pub fn ground(&self, x: f32, y: f32) -> Option<f32> {
        let (east, south) = (x - self.west, self.north - y);
        let size = self.tiles_across.map(|n| n as f32 * self.tile_size);
        if !(east >= 0.0 && south >= 0.0 && east < size[0] && south < size[1]) {
            return None;
        }
        let (col, row) = (
            (east / self.tile_size) as u32,
            (south / self.tile_size) as u32,
        );
        let value = match self.tiles.get(&(col, row)) {
            Some(Laid { heights, .. }) => {
                let within = |metres: f32, at: u32, texels: u32| {
                    ((metres / self.tile_size - at as f32) * texels as f32) as u32
                };
                heights.at(
                    within(east, col, heights.width).min(heights.width - 1),
                    within(south, row, heights.height).min(heights.height - 1),
                )
            }
            None => {
                let texel = self.tile_size / self.block as f32;
                (self.fallback).at((east / texel) as u32, (south / texel) as u32)
            }
        }?;
        Some(height(value))
    }
}

#[cfg(test)]
#[path = "water_height_tests.rs"]
mod tests;
