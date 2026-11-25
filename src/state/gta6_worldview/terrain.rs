//! The terrain's heights the sun's shadow past the cascades is marched over: the install's height map of the ground sampled on an even grid, once on a worker.

use std::sync::{Arc, Mutex, OnceLock};

use or_io::gta6::water::height::WaterGround;

/// The side of a texel of the grid, in metres.
// ponytail: twice the 4 m the ground is held at, a quarter of the device memory; the held size if a shadow's edge shows the step.
const TEXEL: f32 = 8.0;
/// The height a place off the ground's grid stands at, which casts on nothing.
const OFF_THE_GRID: f32 = -800.0;

/// The terrain's heights: rows from the north edge, each west to east, `texel` metres a side.
pub struct Terrain {
    pub west: f32,
    pub north: f32,
    pub texel: f32,
    pub width: u32,
    pub height: u32,
    pub heights: Vec<f32>,
}

impl Terrain {
    /// `ground` sampled at the middle of each texel of `texel` metres over its grid; `None` where the grid holds no whole texel.
    pub fn of(ground: &WaterGround, texel: f32) -> Option<Self> {
        let (west, north, size) = ground.extent();
        let [width, height] = size.map(|metres| (metres / texel).floor() as u32);
        if width == 0 || height == 0 {
            return None;
        }
        let mut heights = Vec::with_capacity(width as usize * height as usize);
        for j in 0..height {
            let y = north - (j as f32 + 0.5) * texel;
            for i in 0..width {
                let x = west + (i as f32 + 0.5) * texel;
                heights.push(ground.ground(x, y).unwrap_or(OFF_THE_GRID));
            }
        }
        Some(Self {
            west,
            north,
            texel,
            width,
            height,
            heights,
        })
    }
}

type Slot = Arc<OnceLock<Option<Arc<Terrain>>>>;

/// Each ground's terrain, by the ground's place in memory, empty while it is worked out.
static HELD: Mutex<Vec<(usize, Slot)>> = Mutex::new(Vec::new());

/// The terrain of `ground`, worked out once on a worker: `None` until it is.
fn of_ground(ground: &Arc<WaterGround>) -> Option<Arc<Terrain>> {
    let key = Arc::as_ptr(ground) as usize;
    let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
    let slot = match held.iter().find(|(at, _)| *at == key) {
        Some((_, slot)) => Arc::clone(slot),
        None => {
            let slot = Slot::default();
            held.push((key, Arc::clone(&slot)));
            let (filled, ground) = (Arc::clone(&slot), Arc::clone(ground));
            std::thread::spawn(move || {
                let work = || Terrain::of(&ground, TEXEL).map(Arc::new);
                let terrain = super::pool::or_on_panic("the terrain", work, None);
                if let Some(terrain) = &terrain {
                    let lowest = terrain.heights.iter().fold(f32::MAX, |lo, h| lo.min(*h));
                    let (top, highest) = (terrain.heights.iter().enumerate()).fold(
                        (0, f32::MIN),
                        |(at, hi), (k, h)| match *h > hi {
                            true => (k, *h),
                            false => (at, hi),
                        },
                    );
                    let (i, j) = (top as u32 % terrain.width, top as u32 / terrain.width);
                    tracing::info!(
                        width = terrain.width,
                        height = terrain.height,
                        lowest,
                        highest,
                        highest_x = terrain.west + (i as f32 + 0.5) * terrain.texel,
                        highest_y = terrain.north - (j as f32 + 0.5) * terrain.texel,
                        "the terrain's heights were laid for the sun's shadow past the cascades"
                    );
                }
                let _ = filled.set(terrain);
            });
            slot
        }
    };
    slot.get()?.clone()
}

impl super::Viewer {
    /// The terrain's heights of the install shown, once its ground is read and sampled.
    pub fn terrain(&self) -> Option<Arc<Terrain>> {
        of_ground(&super::water::ground(self.root.as_deref()?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::water::height::Heights;
    use or_io::gta6::water::GridAtlas;

    #[test]
    fn the_terrain_is_the_ground_at_the_middle_of_each_texel_row_by_row_from_the_north() {
        // One tile of 32 m about the origin.
        let atlas = GridAtlas {
            size: [32.0; 2],
            tile_size: 32.0,
            ..GridAtlas::default()
        };
        // Two by two texels of 16 m: north-west, north-east, south-west, south-east.
        let values = [0u16, 4096, 8192, 12288];
        let fallback = Heights::new(2, 2, values.to_vec()).expect("four values");
        let ground = WaterGround::new(&atlas, fallback).expect("a ground");
        let terrain = Terrain::of(&ground, 16.0).expect("a terrain");
        assert_eq!((terrain.width, terrain.height), (2, 2));
        assert_eq!((terrain.west, terrain.north), (-16.0, 16.0));
        let want: Vec<f32> = values.map(or_io::gta6::water::height::height).to_vec();
        assert_eq!(terrain.heights, want);
    }
}
