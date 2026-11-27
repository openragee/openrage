//! How many rays a frame of the traced mask casts: none once a still view's mask holds every
//! sample, every pixel while it stands still, a quarter of them while the view moves.

/// The samples a still view's mask is the mean of, after which it casts no ray.
pub const SAMPLES: u32 = 64;
/// The side of the block of pixels one ray stands for while the view moves.
pub const MOVING_STEP: u32 = 2;

/// What a frame casts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rate {
    /// A ray a pixel, the `sample`th since the view stood still: the first writes the mask anew.
    Full { sample: u32 },
    /// A ray a block of [`MOVING_STEP`] pixels a side, at the block's pixel `phase`, its share written over the block and rebuilt there from the pixels about it.
    Quarter { phase: u32 },
    /// The mask holds every sample: no ray.
    Converged,
}

impl Rate {
    /// The side of the block a ray stands for.
    pub fn step(self) -> u32 {
        match self {
            Rate::Quarter { .. } => MOVING_STEP,
            _ => 1,
        }
    }

    /// The pixel of its block a ray is cast from, across and down.
    pub fn offset(self) -> [u32; 2] {
        match self {
            // Each corner of the block in turn, so four frames that move little cover it.
            Rate::Quarter { phase } => [[0, 0], [1, 1], [1, 0], [0, 1]][(phase % 4) as usize],
            _ => [0, 0],
        }
    }

    /// The sample a ray adds to its pixel's mean: nought starts the mean again.
    pub fn sample(self) -> u32 {
        match self {
            Rate::Full { sample } => sample,
            _ => 0,
        }
    }

    /// The blocks of a frame of `size`, across and down.
    pub fn blocks(self, (width, height): (u32, u32)) -> (u32, u32) {
        (width.div_ceil(self.step()), height.div_ceil(self.step()))
    }

    /// The rays a frame of `size` casts.
    pub fn rays(self, size: (u32, u32)) -> u64 {
        match self {
            Rate::Converged => 0,
            _ => {
                let (across, down) = self.blocks(size);
                u64::from(across) * u64::from(down)
            }
        }
    }
}

/// The samples the mask holds of the view as it stands.
#[derive(Debug, Default, Clone, Copy)]
pub struct Governor {
    /// The frames the view has stood still for, each a sample.
    stood: u32,
    /// The frames it has moved for.
    moving: u32,
}

impl Governor {
    /// The rate of the next frame, `moved` saying whether the view, the sun, the model or the frame's size changed since the frame before.
    pub fn next(&mut self, moved: bool) -> Rate {
        if moved {
            self.stood = 0;
            self.moving = self.moving.wrapping_add(1);
            return Rate::Quarter {
                phase: self.moving % 4,
            };
        }
        if self.stood >= SAMPLES {
            return Rate::Converged;
        }
        self.stood += 1;
        Rate::Full {
            sample: self.stood - 1,
        }
    }
}
