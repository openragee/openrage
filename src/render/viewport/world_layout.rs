//! What a frame of the world lays out: the level of detail each seen instance draws at, the seen
//! instances side by side a model and level, and the order the draws go out in.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use glam::{Mat4, Vec3};
use or_io::gta6::map::occluders::MapOccluders;
use or_renderer::{Frustum, WorldBucket, WorldInstance};
use rayon::prelude::*;

use crate::state::gta6_world::worldview::stream::{Chunk, Instance};

#[path = "world_occlusion.rs"]
mod occlusion;

use occlusion::Occlusion;

/// How many of its own radii past its sphere a thing is still drawn for the shadow it casts into the view.
// ponytail: one reach for every height of the sun, right down to a sun 34 degrees up; the sun's own height once the renderer says it.
const SHADOW_REACH: f32 = 3.0;

/// The level of detail drawn `away` metres from the camera, of levels that each draw while the
/// distance is under its `until` times `scale`; past the last, the last.
pub(super) fn level(untils: &[f32], away: f32, scale: f32) -> usize {
    (untils.iter())
        .position(|until| away < until * scale)
        .unwrap_or(untils.len().saturating_sub(1))
}

/// How much wider than its sphere a thing `away` metres off is tested against the view, where
/// shadows are cast out to `shadow_far` metres.
fn reach(shadow_far: Option<f32>, away: f32, radius: f32) -> f32 {
    match shadow_far {
        Some(far) if away - radius < far => radius * SHADOW_REACH,
        _ => 0.0,
    }
}

/// What a frame is seen through.
pub(super) struct View {
    pub frustum: Frustum,
    /// The view and projection the frustum is of.
    pub view_proj: Mat4,
    pub eye: Vec3,
    /// The scale every level-of-detail distance is multiplied by.
    pub lod_scale: f32,
    /// How far the sun's shadows are cast, where they are.
    pub shadow_far: Option<f32>,
}

/// An instance the camera sees: its place in the selection, the slot it draws in and how far it is.
#[derive(Clone, Copy)]
pub(super) struct Seen {
    pub at: u32,
    pub slot: u32,
    pub away: f32,
    /// Whether the camera sees the instance itself, and not only the shadow it casts into the view.
    pub shown: bool,
}

/// A frame's layout, kept so that the next reuses its memory.
///
/// A slot is one level of detail of one model of the selection: the instances of a slot lie side
/// by side and draw with one run.
#[derive(Default)]
pub(super) struct Layout {
    /// Where each model's slots start, and one past the last model's.
    pub first: Vec<u32>,
    /// The distance each slot draws under.
    pub untils: Vec<f32>,
    pub seen: Vec<Seen>,
    /// Where each slot's run starts and ends among `laid`.
    pub starts: Vec<u32>,
    pub ends: Vec<u32>,
    /// The nearest instance the camera itself sees in each slot: how far, and its place in the
    /// selection; infinitely far for a slot that only casts shadows into the view.
    pub nearest: Vec<(f32, u32)>,
    pub laid: Vec<WorldInstance>,
    /// The sphere about each instance of `laid`, `[x, y, z, radius]`, which the shadow's cascades are cast by.
    pub spheres: Vec<[f32; 4]>,
    /// The chunks in view the last cull left out as the occluders hide them.
    pub hidden: usize,
    /// The occluders as last drawn: a chunk they hide is not laid out.
    occlusion: Occlusion,
}

impl Layout {
    /// Start a layout of no model.
    pub(super) fn clear(&mut self) {
        self.first.clear();
        self.untils.clear();
    }

    /// The next model of the selection, with the distances its levels draw under; none for a
    /// model that is not on the device, whose instances are then not laid out.
    pub(super) fn model(&mut self, untils: impl Iterator<Item = f32>) {
        self.first.push(self.untils.len() as u32);
        self.untils.extend(untils);
    }

    /// The slots of the model at `model` of the selection.
    pub(super) fn slots(&self, model: usize) -> std::ops::Range<u32> {
        let at = |i: usize| self.first.get(i).copied();
        let end = self.untils.len() as u32;
        at(model).unwrap_or(end)..at(model + 1).unwrap_or(end)
    }

    /// Draw `occluders` for `view`, so that the next cull leaves out the chunks they hide.
    pub(super) fn occlude(&mut self, occluders: &Arc<[Arc<MapOccluders>]>, view: &View) {
        self.occlusion.draw(occluders, view.view_proj, view.eye);
    }

    /// The occluder boxes and models drawn last.
    pub(super) fn occluders(&self) -> (usize, usize) {
        self.occlusion.drawn
    }

    /// The instances of `chunks` the view sees, each with the slot of the level its distance picks.
    ///
    /// ⚠ On every core: a selection holds tens of thousands of instances, and a frame that tested
    /// each on the UI thread spent most of its time here.
    /// A chunk a swap gates draws as `open` says of the swap.
    pub(super) fn cull(
        &mut self,
        (chunks, open): (&[Chunk], &[bool]),
        instances: &[Instance],
        view: &View,
    ) {
        let this = &*self;
        let hidden = AtomicUsize::new(0);
        let seen = (chunks.par_iter())
            .filter(|chunk| {
                let is_open = |swap: u32| open.get(swap as usize).copied().unwrap_or(false);
                (chunk.gate).is_none_or(|(swap, when)| is_open(swap) == when)
            })
            .flat_map_iter(|chunk| {
                let away = view.eye.distance(chunk.centre);
                let wider = chunk.radius + reach(view.shadow_far, away, chunk.radius);
                // A chunk wholly in view is taken whole, and one out of view not at all.
                let whole = view.frustum.holds_sphere(chunk.centre, chunk.radius);
                let part = whole || view.frustum.sees_sphere(chunk.centre, wider);
                // ⚠ As wide as its shadow reaches: what is hidden may cast one into the view.
                let hides = part && this.occlusion.hides(chunk.centre, wider);
                if hides {
                    hidden.fetch_add(1, Ordering::Relaxed);
                }
                let part = part && !hides;
                let run = if part { chunk.from..chunk.to } else { 0..0 };
                run.filter_map(move |at| {
                    let instance = instances.get(at as usize)?;
                    let slots = this.slots(instance.model as usize);
                    if slots.is_empty() {
                        return None;
                    }
                    let away = view.eye.distance(instance.centre);
                    let radius = instance.radius.max(0.01);
                    let wider = radius + reach(view.shadow_far, away, radius);
                    if !whole && !view.frustum.sees_sphere(instance.centre, wider) {
                        return None;
                    }
                    let untils = &this.untils[slots.start as usize..slots.end as usize];
                    Some(Seen {
                        at,
                        slot: slots.start + level(untils, away, view.lod_scale) as u32,
                        away,
                        shown: !instance.cast_only
                            && (whole || view.frustum.sees_sphere(instance.centre, radius)),
                    })
                })
            })
            .collect();
        self.seen = seen;
        self.hidden = hidden.into_inner();
    }

    /// Lay the seen instances out a slot: where each slot's run starts, then each put at its
    /// run's end, its fade in the fourth part of its tint.
    pub(super) fn place(&mut self, instances: &[Instance]) {
        let slots = self.untils.len();
        self.starts.clear();
        self.starts.resize(slots, 0);
        self.nearest.clear();
        self.nearest.resize(slots, (f32::INFINITY, 0));
        for seen in &self.seen {
            self.starts[seen.slot as usize] += 1;
            let nearest = &mut self.nearest[seen.slot as usize];
            if seen.shown && seen.away < nearest.0 {
                *nearest = (seen.away, seen.at);
            }
        }
        let mut total = 0;
        for start in &mut self.starts {
            total += std::mem::replace(start, total);
        }
        self.ends.clone_from(&self.starts);
        self.laid.clear();
        self.laid.resize(
            total as usize,
            WorldInstance {
                rows: [[0.0; 4]; 3],
                tint: [1.0; 4],
                ambient: or_renderer::OUTDOORS,
                cast_only: 0.0,
            },
        );
        self.spheres.clear();
        self.spheres.resize(total as usize, [0.0; 4]);
        for seen in &self.seen {
            let instance = &instances[seen.at as usize];
            let end = &mut self.ends[seen.slot as usize];
            self.laid[*end as usize] = WorldInstance {
                rows: instance.rows,
                tint: [
                    instance.tint[0],
                    instance.tint[1],
                    instance.tint[2],
                    instance.fade,
                ],
                ambient: instance.ambient,
                cast_only: f32::from(u8::from(instance.cast_only)),
            };
            // As wide as the cull takes it.
            self.spheres[*end as usize] = instance.centre.extend(instance.radius.max(0.01)).into();
            *end += 1;
        }
    }

    /// The run of `laid` the slot `slot` draws.
    pub(super) fn run(&self, slot: u32) -> std::ops::Range<u32> {
        self.starts[slot as usize]..self.ends[slot as usize]
    }

    /// Put the instances of the slot `slot` furthest from `eye` first, as what blends is drawn, their spheres with them.
    pub(super) fn far_first(&mut self, slot: u32, eye: Vec3) {
        let run = self.run(slot);
        let run = run.start as usize..run.end as usize;
        let away = |sphere: &[f32; 4]| eye.distance(Vec3::new(sphere[0], sphere[1], sphere[2]));
        let mut order: Vec<usize> = run.clone().collect();
        order.sort_unstable_by(|a, b| away(&self.spheres[*b]).total_cmp(&away(&self.spheres[*a])));
        let laid: Vec<WorldInstance> = order.iter().map(|at| self.laid[*at]).collect();
        let spheres: Vec<[f32; 4]> = order.iter().map(|at| self.spheres[*at]).collect();
        self.laid[run.clone()].copy_from_slice(&laid);
        self.spheres[run].copy_from_slice(&spheres);
    }
}

/// What draws are sorted by: the bucket, then the far before the near where surfaces blend, then
/// the page and the material, so that draws the device takes in one call stand together.
pub(super) fn order(
    bucket: WorldBucket,
    away: f32,
    page: u32,
    material: usize,
) -> (WorldBucket, u32, u32, usize) {
    // The bits of a distance, which is never negative, rise with it.
    let depth = match bucket {
        WorldBucket::Blend => u32::MAX - away.max(0.0).to_bits(),
        _ => 0,
    };
    (bucket, depth, page, material)
}

#[cfg(test)]
mod tests {
    use or_renderer::FlyCamera;

    use super::*;

    fn instance(model: u32, at: Vec3, fade: f32) -> Instance {
        Instance {
            model,
            rows: [
                [1.0, 0.0, 0.0, at.x],
                [0.0, 1.0, 0.0, at.y],
                [0.0, 0.0, 1.0, at.z],
            ],
            centre: at,
            radius: 1.0,
            fade,
            tint: [1.0; 3],
            ambient: or_renderer::OUTDOORS,
            cast_only: false,
        }
    }

    fn view(shadow_far: Option<f32>) -> View {
        // The default camera stands at the origin and looks along +Y.
        let view_proj = FlyCamera::default().view_proj(1.0);
        View {
            frustum: Frustum::of(view_proj),
            view_proj,
            eye: Vec3::ZERO,
            lod_scale: 1.0,
            shadow_far,
        }
    }

    #[test]
    fn a_level_draws_while_the_distance_is_under_its_own() {
        let untils = [20.0, 80.0, f32::INFINITY];
        assert_eq!(level(&untils, 10.0, 1.0), 0);
        assert_eq!(level(&untils, 20.0, 1.0), 1);
        assert_eq!(level(&untils, 500.0, 1.0), 2);
        // The scale stretches every distance.
        assert_eq!(level(&untils, 30.0, 2.0), 0);
        assert_eq!(level(&untils, 30.0, 0.5), 1);
        // A last level that ends still draws past its end, and no level is the first.
        assert_eq!(level(&[20.0, 80.0], 500.0, 1.0), 1);
        assert_eq!(level(&[], 5.0, 1.0), 0);
    }

    #[test]
    fn seen_instances_lie_side_by_side_a_model_and_level() {
        let instances = [
            instance(0, Vec3::new(0.0, 50.0, 0.0), 1.0),
            instance(1, Vec3::new(0.0, 6.0, 0.0), 1.0),
            instance(0, Vec3::new(0.0, 5.0, 0.0), 0.25),
            instance(0, Vec3::new(0.0, -50.0, 0.0), 1.0),
            instance(0, Vec3::new(0.0, 60.0, 0.0), 1.0),
        ];
        let chunks = [Chunk {
            centre: Vec3::ZERO,
            radius: 100.0,
            from: 0,
            to: instances.len() as u32,
            gate: None,
        }];
        let mut layout = Layout::default();
        layout.clear();
        layout.model([10.0, f32::INFINITY].into_iter());
        // The second model is not on the device: it has no slot.
        layout.model(std::iter::empty());
        assert_eq!(layout.slots(0), 0..2);
        assert!(layout.slots(1).is_empty());
        assert!(layout.slots(7).is_empty());

        layout.cull((&chunks, &[]), &instances, &view(None));
        layout.place(&instances);
        // The one behind the camera and the one of the absent model are left out.
        assert_eq!(layout.seen.len(), 3);
        assert_eq!(layout.run(0), 0..1);
        assert_eq!(layout.run(1), 1..3);
        assert_eq!(layout.laid.len(), 3);
        // The near one draws at the first level, with its fade.
        assert_eq!(layout.laid[0].rows[1][3], 5.0);
        assert_eq!(layout.laid[0].tint, [1.0, 1.0, 1.0, 0.25]);
        assert_eq!(layout.nearest[0], (5.0, 2));
        // The far two draw at the second, and the nearer of them is the nearest.
        assert_eq!(layout.nearest[1], (50.0, 0));
        let far: Vec<f32> = layout.laid[1..].iter().map(|i| i.rows[1][3]).collect();
        assert!(far.contains(&50.0) && far.contains(&60.0));

        // Put far first, the run holds the furthest first and each sphere stays with its instance
        for eye in [Vec3::ZERO, Vec3::new(0.0, 100.0, 0.0)] {
            layout.far_first(1, eye);
            let at = |i: usize| layout.laid[i].rows[1][3];
            let away = |y: f32| (y - eye.y).abs();
            assert!(away(at(1)) >= away(at(2)), "{} before {}", at(1), at(2));
            for (sphere, instance) in layout.spheres.iter().zip(&layout.laid) {
                assert_eq!(sphere[1], instance.rows[1][3]);
            }
        }
        assert_eq!(layout.laid[0].rows[1][3], 5.0);
    }

    #[test]
    fn a_gated_chunk_draws_until_its_swap_is_open_or_from_then() {
        let instances = [
            instance(0, Vec3::new(0.0, 5.0, 0.0), 1.0),
            instance(0, Vec3::new(0.0, 6.0, 0.0), 1.0),
            instance(0, Vec3::new(0.0, 7.0, 0.0), 1.0),
        ];
        let chunk = |at: u32, gate| Chunk {
            centre: instances[at as usize].centre,
            radius: 1.0,
            from: at,
            to: at + 1,
            gate,
        };
        let chunks = [
            chunk(0, None),
            chunk(1, Some((0, false))),
            chunk(2, Some((0, true))),
        ];
        let mut layout = Layout::default();
        layout.clear();
        layout.model([f32::INFINITY].into_iter());
        for (open, drawn) in [(&[][..], [0, 1]), (&[false], [0, 1]), (&[true], [0, 2])] {
            layout.cull((&chunks, open), &instances, &view(None));
            let mut seen: Vec<u32> = layout.seen.iter().map(|s| s.at).collect();
            seen.sort_unstable();
            assert_eq!(seen, drawn, "open {open:?}");
        }
    }

    #[test]
    fn what_casts_a_shadow_into_the_view_is_tested_wider() {
        assert_eq!(reach(None, 10.0, 2.0), 0.0);
        assert_eq!(reach(Some(100.0), 10.0, 2.0), 2.0 * SHADOW_REACH);
        // Past where shadows are cast nothing is wider.
        assert_eq!(reach(Some(100.0), 500.0, 2.0), 0.0);

        // Just behind the camera: out of view, and within reach of its shadow.
        let instances = [instance(0, Vec3::new(0.0, -2.0, 0.0), 1.0)];
        let chunks = [Chunk {
            centre: Vec3::new(0.0, -2.0, 0.0),
            radius: 1.0,
            from: 0,
            to: 1,
            gate: None,
        }];
        let mut layout = Layout::default();
        layout.clear();
        layout.model([f32::INFINITY].into_iter());
        layout.cull((&chunks, &[]), &instances, &view(None));
        assert!(layout.seen.is_empty());
        layout.cull((&chunks, &[]), &instances, &view(Some(100.0)));
        assert_eq!(layout.seen.len(), 1);
        // It draws for its shadow alone, and is no slot's nearest.
        assert!(!layout.seen[0].shown);
        layout.place(&instances);
        assert_eq!(layout.run(0), 0..1);
        assert!(layout.nearest[0].0.is_infinite());
    }

    #[test]
    fn one_that_only_casts_is_laid_out_unseen_whatever_its_fade() {
        let mut instances = [instance(0, Vec3::new(0.0, 5.0, 0.0), 0.5)];
        instances[0].cast_only = true;
        let chunks = [Chunk {
            centre: Vec3::new(0.0, 5.0, 0.0),
            radius: 1.0,
            from: 0,
            to: 1,
            gate: None,
        }];
        let mut layout = Layout::default();
        layout.clear();
        layout.model([f32::INFINITY].into_iter());
        layout.cull((&chunks, &[]), &instances, &view(None));
        assert!(!layout.seen[0].shown);
        layout.place(&instances);
        assert_eq!(layout.laid[0].cast_only, 1.0);
        assert_eq!(layout.laid[0].tint[3], 0.5);
        assert!(layout.nearest[0].0.is_infinite());
    }

    #[test]
    fn a_chunk_the_occluders_hide_is_not_laid_out() {
        use or_io::gta6::map::occluders::BoxOccluder;
        // A wall 20 m ahead, 20 m wide and high: corner k is high in x, y, z by its bits 0, 1, 2.
        let corners = std::array::from_fn(|k| {
            let pick = |bit: usize, low: f32, high: f32| if k >> bit & 1 == 0 { low } else { high };
            [
                pick(0, -10.0, 10.0),
                pick(1, 20.0, 21.0),
                pick(2, -10.0, 10.0),
            ]
        });
        let occluders: Arc<[Arc<MapOccluders>]> = Arc::from([Arc::new(MapOccluders {
            boxes: vec![BoxOccluder { corners }],
            models: Vec::new(),
        })]);
        let chunk = |at: Vec3, from: u32| Chunk {
            centre: at,
            radius: 1.0,
            from,
            to: from + 1,
            gate: None,
        };
        // Behind the wall, half out from behind its side, and before it.
        let places = [
            Vec3::new(0.0, 50.0, 0.0),
            Vec3::new(25.0, 50.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
        ];
        let instances = places.map(|at| instance(0, at, 1.0));
        let chunks: Vec<Chunk> = (places.iter().zip(0..))
            .map(|(at, k)| chunk(*at, k))
            .collect();
        let mut layout = Layout::default();
        layout.clear();
        layout.model([f32::INFINITY].into_iter());
        layout.cull((&chunks, &[]), &instances, &view(None));
        assert_eq!(layout.seen.len(), 3);
        layout.occlude(&occluders, &view(None));
        layout.cull((&chunks, &[]), &instances, &view(None));
        let seen: Vec<u32> = layout.seen.iter().map(|s| s.at).collect();
        assert_eq!(seen, [1, 2]);
        // Tested as wide as its shadow reaches, it is still wholly behind the wall.
        layout.cull((&chunks, &[]), &instances, &view(Some(100.0)));
        let seen: Vec<u32> = layout.seen.iter().map(|s| s.at).collect();
        assert_eq!(seen, [1, 2]);
        // Larger, its shadow's reach passes the wall's side: it draws for that.
        let mut wide = chunks.clone();
        wide[0].radius = 8.0;
        layout.cull((&wide, &[]), &instances, &view(Some(100.0)));
        assert_eq!(layout.seen.len(), 3);
    }

    #[test]
    fn draws_go_out_a_bucket_then_a_page_then_a_material() {
        use WorldBucket::*;
        let mut keys = vec![
            order(Emissive, 1.0, 0, 0),
            order(Blend, 5.0, 0, 0),
            order(Blend, 90.0, 3, 9),
            order(Decal, 1.0, 0, 0),
            order(Opaque, 1.0, 2, 1),
            order(Cutout, 1.0, 0, 0),
            order(Opaque, 70.0, 1, 8),
            order(Opaque, 3.0, 1, 4),
            order(Opaque, 9.0, 2, 1),
        ];
        keys.sort_unstable();
        // The buckets go out in the renderer's own order, each bucket's draws together.
        assert!(keys.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        let of = |bucket: WorldBucket| -> Vec<(u32, usize)> {
            let draws = keys.iter().filter(|key| key.0 == bucket);
            draws.map(|key| (key.2, key.3)).collect()
        };
        // Opaque draws of one page stand together, and in it those of one material.
        assert_eq!(of(Opaque), [(1, 4), (1, 8), (2, 1), (2, 1)]);
        // What blends goes out the far first, whatever its page.
        assert_eq!(of(Blend), [(3, 9), (0, 0)]);
    }
}
