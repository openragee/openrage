//! The scene the traced pass walks: the drawn model's triangles as one bottom level, placed once
//! in one top level, in an arena of the console's node layout.

use std::ops::Range;

use or_io::gta6::accel::layout::HEADER_ROOT;
use or_io::gta6::accel::{word, AccelError, Arena, Instance, Structure, Triangle};

/// The model stands where its file puts it: the one instance turns and moves nothing.
const IN_PLACE: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];
/// The one instance is entered by a walk that asks for any bit of its mask.
const EVERY_MASK: u8 = 0xFF;

/// A model's arena: its bottom level, the top level that places it, and the instance between.
#[derive(Debug, Clone)]
pub struct SceneArena {
    pub arena: Arena,
    pub bottom: Structure,
    pub top: Structure,
    instance: Instance,
}

impl SceneArena {
    /// The arena of `triangles`, each one opaque as it says, in place.
    pub fn build(triangles: &[Triangle]) -> Result<Self, AccelError> {
        let mut arena = Arena::new();
        let bottom = arena.bottom_level(triangles)?;
        let instance = Instance {
            bottom,
            object_to_world: IN_PLACE,
            id: 0,
            mask: EVERY_MASK,
            flags: 0,
            contribution: 0,
        };
        let top = arena.top_level(&[instance])?;
        Ok(Self {
            arena,
            bottom,
            top,
            instance,
        })
    }

    /// The arena's dwords, which the walk's storage buffer holds as they are.
    pub fn words(&self) -> &[u32] {
        self.arena.words()
    }

    /// The node the top level's header states as its root, where a walk starts.
    pub fn root(&self) -> u32 {
        word(self.arena.words(), self.top.base, HEADER_ROOT)
    }

    /// The middle of what the top level bounds and the distance from it to its farthest corner.
    pub fn sphere(&self) -> ([f32; 3], f32) {
        let (min, max) = (self.top.min, self.top.max);
        let centre: [f32; 3] = std::array::from_fn(|k| (min[k] + max[k]) * 0.5);
        let half: [f32; 3] = std::array::from_fn(|k| (max[k] - min[k]) * 0.5);
        (centre, half.iter().map(|h| h * h).sum::<f32>().sqrt())
    }

    /// The arena rewritten in place for `posed`, its triangles where a pose puts them: the byte ranges a device's copy takes again.
    ///
    /// # Errors
    ///
    /// Where the pose lacks a triangle of the model or tears two that share a node; the arena is
    /// then to be built anew from the posed triangles.
    pub fn refit(&mut self, posed: &[Triangle]) -> Result<Vec<Range<u32>>, AccelError> {
        self.bottom = self.arena.refit_bottom(&self.bottom, posed)?;
        self.instance.bottom = self.bottom;
        self.top = self.arena.refit_top(&self.top, &[self.instance])?;
        Ok(vec![
            self.arena.node_bytes(&self.bottom),
            self.arena.node_bytes(&self.top),
        ])
    }
}
