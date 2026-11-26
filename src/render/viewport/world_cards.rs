//! The world viewer's far stand-ins on the device: the selection's card lists handed to the
//! renderer when the selection changes, those the camera sees within their distance shown, and a
//! species baked into the atlas once its model and its colour maps are on the device.
//!
//! ⚠ A species whose model the world never drew near is read for its atlas alone, after
//! everything that draws: until it is baked its cards draw nothing.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use or_renderer::world::cards::{CardBatch, CardMesh, CardSpecies};
use or_renderer::world::{WorldSurface, WORLD_MAPS};
use or_renderer::{Frustum, ViewportRenderer, WorldBucket};

use super::textures::ABSENT;
use super::{Slot, WorldGpu};
use crate::state::gta6_world::worldview::index::ModelRef;
use crate::state::gta6_world::worldview::stream::slod::Species;
use crate::state::gta6_world::worldview::stream::Selected;
use stated::{stated, view_of};

#[path = "world_cards_stated.rs"]
pub(super) mod stated;

/// What of the far stand-ins the renderer holds.
#[derive(Default)]
pub(super) struct Cards {
    /// The selection whose lists the renderer holds.
    version: Option<u64>,
    /// The species table the slots were given for, held so that no other comes to lie where it did.
    table: Option<Arc<[Species]>>,
    /// The slot each species id was baked into, and `None` for one that will never bake.
    slots: HashMap<usize, Option<u32>>,
    /// The next slot to bake into: never one baked before, since a bake draws over what a slot holds.
    next: u32,
    /// The level of its model each species id was baked from.
    levels: HashMap<usize, usize>,
    /// The lists shown last, by their place in the selection.
    shown: Vec<u32>,
    /// The colour a species' records state, by each colour map of its model no file holds, and a map of each such colour.
    stated: HashMap<u64, [u8; 3]>,
    views: HashMap<[u8; 3], wgpu::TextureView>,
    /// The colour each species id's records state, worked out once a table.
    by_id: HashMap<usize, Option<[u8; 3]>>,
}

impl Cards {
    /// The colour map that stands in for `key`, a colour map no file holds, where the records of its species state a colour.
    pub(super) fn stated(&self, key: u64) -> Option<&wgpu::TextureView> {
        self.views.get(self.stated.get(&key)?)
    }

    /// Each species id baked as `ID:SLOT:LEVEL`, and one that will never bake as `ID:-`, sorted by id.
    pub(super) fn residency(&self) -> Vec<String> {
        let mut ids: Vec<(&usize, &Option<u32>)> = self.slots.iter().collect();
        ids.sort_unstable();
        (ids.into_iter())
            .map(|(id, slot)| match (slot, self.levels.get(id)) {
                (Some(slot), Some(level)) => format!("{id}:{slot}:{level}"),
                _ => format!("{id}:-"),
            })
            .collect()
    }
}

/// Whether the camera at `eye` sees some of a list's sphere within its distance; a list faded out draws nothing.
fn seen(
    frustum: &Frustum,
    eye: Vec3,
    (centre, radius): (Vec3, f32),
    distance: f32,
    fade: f32,
) -> bool {
    fade > 0.0 && eye.distance(centre) - radius <= distance && frustum.sees_sphere(centre, radius)
}

impl WorldGpu {
    /// Draw no card until the next selection is handed over.
    pub(super) fn hide_cards(&mut self, renderer: &mut ViewportRenderer) {
        if !self.cards.shown.is_empty() {
            renderer.show_card_lists(&[]);
        }
        self.cards.version = None;
        self.cards.shown.clear();
    }

    /// The meshes of the least detailed level of `model` that has something to bake, once its every
    /// map is on the device or known never to come, each with the material the world draws it
    /// with, and the maps of that level that have not arrived put in `absent` with whether each is
    /// read whole; what blends or lies on a surface is left out of a stand-in. A colour map no file
    /// holds is `stand_in` where its species' records state a colour, and its key put in `stood`.
    // ponytail: every map a material names is waited for, normal maps too, which a bake reads
    // nothing of; wait for those of colour and coverage alone if the bakes are seen to lag.
    fn card_meshes<'a>(
        &'a self,
        (renderer, device): (&ViewportRenderer, &wgpu::Device),
        model: &ModelRef,
        (grey, stand_in): (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>),
        absent: &mut Vec<(u64, bool)>,
        stood: &mut Vec<u64>,
    ) -> Option<(Vec<CardMesh<'a>>, usize)> {
        let Some(Slot::Ready { lods, .. }) = self.models.get(model) else {
            return None;
        };
        for (level, lod) in lods.iter().enumerate().rev() {
            let mut ready = Vec::with_capacity(lod.meshes.len());
            let mut waits = false;
            for mesh in &lod.meshes {
                let Some(material) = self.materials.get(mesh.material as usize) else {
                    continue;
                };
                if matches!(material.bucket, WorldBucket::Decal | WorldBucket::Blend) {
                    continue;
                }
                // As the world binds them: a model's own palette, a map on the device, for a colour map no pack holds the colour its records state or a mid grey
                let mut maps = [None; WORLD_MAPS];
                for (at, key) in material.maps.iter().enumerate() {
                    let Some(key) = *key else {
                        continue;
                    };
                    maps[at] = match (self.palettes.get(&key), self.textures.of(key)) {
                        (Some(palette), _) => Some(palette),
                        (None, (_, Some(view))) => Some(view),
                        // Not arrived yet: the level waits for it.
                        (None, (ABSENT, None)) => {
                            waits = true;
                            absent.push((key, material.whole[at]));
                            None
                        }
                        (None, (_, None)) if material.colours[at] => {
                            if stand_in.is_some() {
                                stood.push(key);
                            }
                            Some(stand_in.unwrap_or(grey))
                        }
                        (None, (_, None)) => None,
                    };
                }
                match self.pages.buffers(mesh.place.page) {
                    Some(buffers) => ready.push((mesh, material, maps, buffers)),
                    None => waits = true,
                }
            }
            // ⚠ A level that waits is waited for, never passed over for a more detailed one already here: what a bake draws would hang on what arrived first.
            if waits {
                return None;
            }
            if ready.is_empty() {
                continue;
            }
            let meshes = (ready.into_iter())
                .map(|(mesh, material, maps, (vertices, indices))| {
                    let surface = WorldSurface {
                        family: material.family,
                        bucket: material.bucket,
                        maps,
                        constants: material.constants,
                    };
                    CardMesh {
                        vertices,
                        indices,
                        base_vertex: mesh.place.base_vertex as i32,
                        first_index: mesh.place.first_index,
                        index_count: mesh.place.indices,
                        material: renderer.world_surface(device, &surface),
                    }
                })
                .collect();
            return Some((meshes, level));
        }
        None
    }

    /// Hand the renderer the selection's card lists when they change, show those whose sphere the
    /// camera at `eye` sees within their distance, and bake the species whose model and colour
    /// maps are on the device; `model_of` is the model of the species of an id. Gives what the
    /// species of the shown lists wait for, asked for as the frame's models and textures are.
    pub(super) fn refresh_cards(
        &mut self,
        renderer: &mut ViewportRenderer,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        selected: &Selected,
        (frustum, eye): (&Frustum, Vec3),
        model_of: &dyn Fn(usize) -> Option<ModelRef>,
    ) -> CardAsks {
        let fresh = self.cards.version != Some(selected.version);
        if fresh {
            self.cards.version = Some(selected.version);
            let lists: Vec<CardBatch> = (selected.cards.iter())
                .map(|list| CardBatch {
                    min: list.min,
                    size: list.size,
                    scale_range: list.scale_range,
                    distance: list.distance,
                    fade: list.fade,
                    records: Arc::clone(&list.records),
                })
                .collect();
            renderer.set_card_lists(device, queue, &lists);
        }
        let shown: Vec<u32> = (selected.cards.iter().enumerate())
            .filter(|(_, list)| {
                let sphere = (Vec3::from(list.centre), list.radius);
                seen(frustum, eye, sphere, list.distance, list.fade)
            })
            .map(|(at, _)| at as u32)
            .collect();
        // Setting the lists shows none of them.
        if fresh || shown != self.cards.shown {
            renderer.show_card_lists(&shown);
            self.cards.shown = shown;
        }

        let species = &selected.species;
        let mut changed =
            !(self.cards.table.as_ref()).is_some_and(|table| Arc::ptr_eq(table, species));
        if changed {
            self.cards.table = Some(Arc::clone(species));
            self.cards.slots.clear();
            self.cards.levels.clear();
            self.cards.by_id.clear();
        }
        // The model of each species a shown list names and no slot holds, by the nearest list that names it.
        let mut waits: HashMap<usize, f32> = HashMap::new();
        for list in self
            .cards
            .shown
            .iter()
            .filter_map(|at| selected.cards.get(*at as usize))
        {
            let away = (eye.distance(Vec3::from(list.centre)) - list.radius).max(0.0);
            for id in list.ids.iter().map(|id| usize::from(*id)) {
                if !self.cards.slots.contains_key(&id) {
                    let nearest = waits.entry(id).or_insert(away);
                    *nearest = nearest.min(away);
                }
            }
        }
        // ponytail: a few bakes a frame, by id; nearest first if the bakes are seen to lag.
        let (mut settled, mut absent, mut models) = (Vec::new(), Vec::new(), Vec::new());
        let mut stood = Vec::new();
        let grey = self.textures.grey(device, queue);
        let mut bakes = BAKES_A_FRAME;
        let mut more = false;
        for (id, kind) in species.iter().enumerate() {
            if kind.radius <= 0.0 || self.cards.slots.contains_key(&id) {
                continue;
            }
            let Some(model) = model_of(id) else {
                settled.push((id, None));
                continue;
            };
            match self.models.get(&model) {
                // A model that will never come bakes nothing.
                Some(Slot::Missing) => {
                    settled.push((id, None));
                    continue;
                }
                // A species never drawn near is read for its atlas alone, after what draws.
                None => {
                    if let Some(away) = waits.get(&id) {
                        models.push((ASKED_LAST + away, model));
                    }
                    continue;
                }
                Some(Slot::Ready { .. }) => {}
            }
            let lists = &selected.cards;
            let colour = *(self.cards.by_id.entry(id))
                .or_insert_with(|| u16::try_from(id).ok().and_then(|id| stated(lists, id)));
            if let Some(colour) = colour {
                (self.cards.views.entry(colour)).or_insert_with(|| view_of(device, queue, colour));
            }
            let stand_in = colour.and_then(|colour| self.cards.views.get(&colour));
            let mut keys = Vec::new();
            let found = self.card_meshes(
                (&*renderer, device),
                &model,
                (&grey, stand_in),
                &mut absent,
                &mut keys,
            );
            stood.extend(keys.into_iter().zip(std::iter::repeat(colour)));
            let Some((meshes, level)) = found else {
                continue;
            };
            if bakes == 0 {
                more = true;
                continue;
            }
            bakes -= 1;
            let slot = self.cards.next;
            let fits = slot < renderer.card_slots(device)
                && renderer.bake_card_slot(device, queue, slot, kind.centre, kind.radius, &meshes);
            if fits {
                self.cards.next += 1;
                self.cards.levels.insert(id, level);
            }
            settled.push((id, fits.then_some(slot)));
        }
        for (key, colour) in stood {
            if let Some(colour) = colour {
                self.cards.stated.insert(key, colour);
            }
        }
        for (id, slot) in settled {
            self.cards.slots.insert(id, slot);
            changed = true;
        }
        if changed {
            let table: Vec<CardSpecies> = (species.iter().enumerate())
                .map(|(id, kind)| CardSpecies {
                    centre: kind.centre,
                    radius: kind.radius,
                    slot: self.cards.slots.get(&id).copied().flatten(),
                })
                .collect();
            renderer.set_card_species(device, queue, &table);
        }
        // ⚠ A palette is read texel by texel: a smaller copy of it moves every column.
        let whole = (self.budgets.as_ref()).map_or(BAKED_PIXELS, |budgets| budgets.most_side);
        CardAsks {
            // Asked for after everything the camera sees, at the size a frame of the atlas needs.
            textures: (absent.into_iter())
                .map(|(key, read_whole)| {
                    (
                        ASKED_LAST,
                        key,
                        if read_whole { whole } else { BAKED_PIXELS },
                    )
                })
                .collect(),
            models,
            more,
        }
    }
}

/// What the far stand-ins wait for: the colour maps and the models of species to bake, and whether a species ready to bake was left for the next frame.
pub(super) struct CardAsks {
    pub textures: Vec<(f32, u64, u32)>,
    pub models: Vec<(f32, ModelRef)>,
    pub more: bool,
}

/// The priority a stand-in's colour map and model are asked for at, after everything that draws.
const ASKED_LAST: f32 = 1.0e9;
/// The most species baked in one frame.
const BAKES_A_FRAME: usize = 16;
/// The longest side a stand-in's colour map is asked for at: a frame of the atlas is 32 texels, a model's map a few times its size.
const BAKED_PIXELS: u32 = 128;

#[cfg(test)]
mod tests {
    use super::*;
    use or_renderer::FlyCamera;

    #[test]
    fn a_list_is_seen_while_some_of_it_is_in_view_within_its_distance_and_not_faded_out() {
        // The camera at the origin looks along +Y.
        let frustum = Frustum::of(FlyCamera::default().view_proj(1.0));
        let ahead = (Vec3::new(0.0, 1000.0, 0.0), 50.0);
        assert!(seen(&frustum, Vec3::ZERO, ahead, 1000.0, 1.0));
        // Its nearest point is what is measured, not its centre
        assert!(seen(&frustum, Vec3::ZERO, ahead, 951.0, 1.0));
        assert!(!seen(&frustum, Vec3::ZERO, ahead, 949.0, 1.0));
        assert!(!seen(&frustum, Vec3::ZERO, ahead, 1000.0, 0.0));
        let behind = (Vec3::new(0.0, -1000.0, 0.0), 50.0);
        assert!(!seen(&frustum, Vec3::ZERO, behind, 5000.0, 1.0));
    }
}
