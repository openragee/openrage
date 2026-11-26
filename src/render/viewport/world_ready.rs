//! A model put on the device is told to the world's thread only once every map its levels sample is
//! there too, or never will be: until then what it replaces keeps drawing, as the game holds a parent
//! until its children are whole, and no level draws grey for want of its textures.
//!
//! ⚠ Only for what is in view: holding every model waited for read the textures of everything around
//! the camera, twice what a street camera draws, past the budget, so what was put on the device was
//! let go and read again. ⚠ Every level: holding for the first level alone left the nearer levels of
//! a model grey as a fast camera closed on it.

use std::collections::HashMap;

use glam::Vec3;

use super::textures::{self, Texture};
use super::{Slot, WorldGpu};
use crate::state::gta6_world::worldview::index::ModelRef;

impl WorldGpu {
    /// The maps the meshes of `lods` sample that no model holds itself, with each one's level and mesh density, and whether it is read whole.
    fn maps_of<'a>(
        &'a self,
        lods: &'a [super::Lod],
    ) -> impl Iterator<Item = (usize, u64, f32, bool)> + 'a {
        lods.iter().enumerate().flat_map(move |(level, lod)| {
            lod.meshes.iter().flat_map(move |mesh| {
                let material = &self.materials[mesh.material as usize];
                (material.maps.iter().zip(material.whole))
                    .filter_map(move |(key, whole)| Some((level, (*key)?, mesh.density, whole)))
                    .filter(|(_, key, ..)| !self.palettes.contains_key(key))
            })
        })
    }

    /// Tell of each model held back whose maps have all come or will never come; with no reader of
    /// textures nothing is waited for.
    pub(super) fn release_whole(&mut self, reads_textures: bool) {
        let mut released = Vec::new();
        for model in self.pending.keys() {
            let whole = match self.models.get(model) {
                Some(Slot::Ready { lods, .. }) => {
                    !reads_textures
                        || (self.maps_of(lods))
                            .all(|(_, key, ..)| self.textures.map.contains_key(&key))
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
        let now = std::time::Instant::now();
        for (model, tell) in released {
            let put = self.pending.remove(&model);
            if tell {
                self.told.push((model, Some(true)));
            }
            if let (true, Some(put)) = (tell, put) {
                let waited = now.duration_since(put).as_secs_f32();
                let (count, all, most) = &mut self.whole;
                (*count, *all, *most) = (*count + 1, *all + f64::from(waited), most.max(waited));
            }
        }
    }

    /// What the models held back wait for: each map at the size its level is needed at where that
    /// level first draws, `wanted` saying how far the nearest thing that waits for each model is.
    /// What they hold already is kept on the device this frame. A model nothing in view (`seen`)
    /// waits for is told at once: what draws out of view is never seen grey.
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
        for model in self.pending.keys() {
            let Some(Slot::Ready { lods, at, .. }) = self.models.get(model) else {
                continue;
            };
            let away = nearest.get(model).copied().unwrap_or(eye.distance(*at));
            for (level, key, density, whole) in self.maps_of(lods) {
                // A level draws from where the one before it ends.
                let from = level
                    .checked_sub(1)
                    .map_or(0.0, |before| lods[before].until * lod_scale);
                let near = away.max(from);
                match self.textures.map.get(&key) {
                    Some(Texture::Ready(_)) => kept.push(key),
                    Some(Texture::Missing) => {}
                    None => {
                        let pixels = match whole {
                            true => most_side,
                            false => textures::pixels(density, near, 1.0, per_metre, most_side),
                        };
                        asks.push((near, key, pixels));
                    }
                }
            }
        }
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
