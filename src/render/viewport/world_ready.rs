//! A model put on the device is told to the world's thread only once every map the level it first
//! draws at samples is there too, or never will be: until then what it replaces keeps drawing, as the
//! game holds a parent until its children are whole, and no level draws grey for want of its textures.
//!
//! ⚠ Only that level: asking for the maps of every level doubled the textures held at a street
//! camera and passed the budget, so what was put on the device was let go and read again.

use std::collections::HashMap;

use glam::Vec3;

use super::layout::level;
use super::textures::{self, Texture};
use super::{Slot, WorldGpu};
use crate::state::gta6_world::worldview::index::ModelRef;

impl WorldGpu {
    /// The maps the meshes of level `at` of `lods` sample that no model holds itself, with each
    /// mesh's density and whether the map is read whole.
    fn maps_of<'a>(
        &'a self,
        lods: &'a [super::Lod],
        at: usize,
    ) -> impl Iterator<Item = (u64, f32, bool)> + 'a {
        (lods.get(at).into_iter()).flat_map(move |lod| {
            lod.meshes.iter().flat_map(move |mesh| {
                let material = &self.materials[mesh.material as usize];
                (material.maps.iter().zip(material.whole))
                    .filter_map(move |(key, whole)| Some(((*key)?, mesh.density, whole)))
                    .filter(|(key, ..)| !self.palettes.contains_key(key))
            })
        })
    }

    /// Tell of each model held back whose level that draws first has all its maps, or will never
    /// have them; with no reader of textures nothing is waited for.
    pub(super) fn release_whole(&mut self, reads_textures: bool) {
        let mut released = Vec::new();
        for (model, at) in &self.pending {
            let whole = match self.models.get(model) {
                Some(Slot::Ready { lods, .. }) => {
                    !reads_textures
                        || (self.maps_of(lods, *at))
                            .all(|(key, ..)| self.textures.map.contains_key(&key))
                }
                // Let go before it was whole: what let it go told the thread so.
                _ => {
                    released.push((*model, false));
                    continue;
                }
            };
            if whole {
                released.push((*model, true));
            }
        }
        for (model, tell) in released {
            self.pending.remove(&model);
            if tell {
                self.told.push((model, Some(true)));
            }
        }
    }

    /// What the models held back wait for: each map of the level each first draws at, at the size
    /// it is needed at there, `wanted` saying how far the nearest thing that waits for each model
    /// is. What they hold already is kept on the device this frame. A model nothing in view
    /// (`seen`) waits for is told at once: what draws out of view is never seen grey.
    pub(super) fn ask_whole(
        &mut self,
        wanted: &[(f32, ModelRef)],
        seen: impl Fn(&ModelRef) -> bool,
        (eye, lod_scale): (Vec3, f32),
        (per_metre, most_side): (f32, u32),
        frame: u64,
    ) -> Vec<(f32, u64, u32)> {
        let unseen: Vec<ModelRef> = self.pending.keys().filter(|m| !seen(m)).copied().collect();
        for model in unseen {
            self.pending.remove(&model);
            if matches!(self.models.get(&model), Some(Slot::Ready { .. })) {
                self.told.push((model, Some(true)));
            }
        }
        let mut nearest: HashMap<ModelRef, f32> = HashMap::new();
        for (away, model) in wanted {
            let at = nearest.entry(*model).or_insert(*away);
            *at = at.min(*away);
        }
        let mut asks = Vec::new();
        let mut kept = Vec::new();
        let mut levels = Vec::new();
        for model in self.pending.keys() {
            let Some(Slot::Ready { lods, at, .. }) = self.models.get(model) else {
                continue;
            };
            let away = nearest.get(model).copied().unwrap_or(eye.distance(*at));
            let untils: Vec<f32> = lods.iter().map(|lod| lod.until).collect();
            let first = level(&untils, away, lod_scale);
            levels.push((*model, first));
            for (key, density, whole) in self.maps_of(lods, first) {
                match self.textures.map.get(&key) {
                    Some(Texture::Ready(_)) => kept.push(key),
                    Some(Texture::Missing) => {}
                    None => {
                        let pixels = match whole {
                            true => most_side,
                            false => textures::pixels(density, away, 1.0, per_metre, most_side),
                        };
                        asks.push((away, key, pixels));
                    }
                }
            }
        }
        self.pending.extend(levels);
        for key in kept {
            if let Some(Texture::Ready(ready)) = self.textures.map.get_mut(&key) {
                ready.used = ready.used.max(frame);
            }
        }
        // ⚠ A model held back has not drawn: kept last of what is let go for room.
        for model in self.pending.keys() {
            if let Some(Slot::Ready { held, .. }) = self.models.get_mut(model) {
                *held = frame;
            }
        }
        asks
    }
}
