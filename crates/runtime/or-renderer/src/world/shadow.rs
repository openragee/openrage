//! The sun's shadow over a world: four cascades about what the camera sees, each cast into a tile of the game's map and read by the game's own pass.

use std::collections::BTreeMap;
use std::ops::Range;

use glam::{Mat4, Vec3, Vec4};

use super::{WorldDraw, WorldInstance, ARGS};

use crate::fly::slice_sphere;
use crate::game_shaders::deferred::Sun;
use crate::game_shaders::engine::EngineConstants;
use crate::game_shaders::shadow::{Fit, TILE};
use crate::viewport::OrbitCamera;

/// How many cascades the game's pass reads, each a tile of its map.
pub(crate) const CASCADES: usize = 4;

/// The cycle's variable every cascade's end is multiplied by.
pub const DISTANCE_MULTIPLIER: &str = "dir_shadow_distance_multiplier";

/// The ends of the game's split ahead of the camera before the cycle's multiplier, in metres.
const SPLIT_NEAR: f32 = 0.5;
const SPLIT_FAR: f32 = 176.551_71;

/// How far ahead of the camera the sun's shadow of a world is cast at a multiplier of one by a camera that rides nothing, in metres.
pub const SHADOW_FAR: f32 = SPLIT_FAR;

/// What the game's split is stretched by for what the camera rides: every end times `scale`, and each end between the first and the last drawn `weight` of the way from the even split towards the geometric one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowFocus {
    pub scale: f32,
    pub weight: f32,
}

impl ShadowFocus {
    /// A camera that rides nothing, as the game's free camera.
    pub const FREE: Self = Self {
        scale: 1.0,
        weight: 0.9,
    };
    /// A camera that rides an aircraft: the least the game stretches an aircraft's split by, that of a craft of a sphere of 5 m or less, and its weight.
    pub const AIRCRAFT: Self = Self {
        scale: 5.0,
        weight: 0.99,
    };
}

impl Default for ShadowFocus {
    fn default() -> Self {
        Self::FREE
    }
}

/// How far ahead of the camera each cascade ends at the cycle's `multiplier` for a camera of `focus`, in metres: the game's split of four, each end between an even and a geometric split from half a metre to [`SHADOW_FAR`], stretched as `focus` says.
pub fn ends(multiplier: f32, focus: ShadowFocus) -> [f32; CASCADES] {
    std::array::from_fn(|at| {
        let along = (at + 1) as f32 / CASCADES as f32;
        let even = SPLIT_NEAR + (SPLIT_FAR - SPLIT_NEAR) * along;
        let geometric = SPLIT_NEAR * (SPLIT_FAR / SPLIT_NEAR).powf(along);
        multiplier * focus.scale * (even + (geometric - even) * focus.weight)
    })
}

/// How far ahead of the camera the sun's shadow of a world is cast at the cycle's `multiplier` for a camera of `focus`, in metres.
pub fn shadow_reach(multiplier: f32, focus: ShadowFocus) -> f32 {
    ends(multiplier, focus)[CASCADES - 1]
}

/// The part of the reach, at its far end, over which the shadow fades to none.
const FADE: f32 = 0.1;

/// How far below the surface that casts it the shadow is held in a cascade, as a part of the width of its tile in the world, so that a surface does not shade itself.
const DEPTH_BIAS: f32 = 0.0015;

/// The variable the receiving pass reads the axes, the fade and each cascade's tile from, and where the scales and the offsets of the tiles start in it, a register of four a cascade.
const SHARED: &str = "gCSMShaderVars_shared";
const SCALES: usize = 16;
const OFFSETS: usize = 32;

/// The cascades of one frame, the nearest first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cascades {
    /// Where each tile lies across the light.
    fits: [Fit; CASCADES],
    // ⚠ One depth for all four: the receiving pass gives a pixel the first tile it lies across whatever its depth, so a tile that held only the depths of its own sphere would shade everything past them.
    /// What a point of the world is multiplied by for its depth in every tile, from nought at the nearest the sun any cascade reaches to one at the furthest.
    depth: Vec4,
    /// How far ahead of the camera the last ends.
    far: f32,
}

impl Cascades {
    /// The cascades about what `camera` sees in a frame of `aspect`, cast by the sun of `engine` as far as the cycle's `multiplier` takes them for a camera of `focus`; none where it does not shine.
    pub(crate) fn about(
        engine: &EngineConstants,
        camera: &OrbitCamera,
        aspect: f32,
        (multiplier, focus): (f32, ShadowFocus),
    ) -> Option<Self> {
        let sun = Sun::of(engine);
        if !sun.shines() {
            return None;
        }
        let eye = camera.eye();
        let forward = (camera.target - eye).normalize_or(Vec3::Y);
        let ends = ends(multiplier, focus);
        let far = ends[CASCADES - 1].min(camera.far);
        if far.is_nan() || far <= 0.0 {
            return None;
        }
        let mut near = camera.near.min(far);
        let spheres = ends.map(|end| {
            let end = end.min(far).max(near);
            let (centre, radius) = slice_sphere(eye, forward, camera.fov_y, aspect, near, end);
            near = end;
            snapped(sun.travel, centre, radius)
        });
        let fits = spheres.map(|(centre, radius)| Fit::of(sun.travel, centre, radius));
        // The third row of a caster's view is the way the light travels
        let along = fits[0].caster().view.row(2).truncate();
        let reach = |side: f32| spheres.map(|(centre, radius)| centre.dot(along) + side * radius);
        let first = reach(-1.0).into_iter().fold(f32::INFINITY, f32::min);
        let last = reach(1.0).into_iter().fold(f32::NEG_INFINITY, f32::max);
        let span = (last - first).max(1e-3);
        Some(Self {
            fits,
            depth: (along / span).extend(-first / span),
            far,
        })
    }

    /// The world to the clip space of each tile, which whatever casts is drawn through into it: across and down the tile as its own fit lays it, and the one depth of all the tiles.
    pub(crate) fn views(&self) -> [Mat4; CASCADES] {
        self.fits.map(|fit| {
            let caster = fit.caster();
            let rows = (caster.projection * caster.view).transpose();
            Mat4::from_cols(rows.x_axis, rows.y_axis, self.depth, rows.w_axis).transpose()
        })
    }

    /// The constants a scene's own surfaces are cast with, into the nearest tile.
    pub(crate) fn caster(&self) -> EngineConstants {
        let fitted = self.fits[0].caster();
        EngineConstants {
            // The view is a turn and a move, and the projection whatever takes it to the tile
            projection: self.views()[0] * fitted.view.inverse(),
            ..fitted
        }
    }

    /// How whole the cascades' shadow is at a distance ahead: that distance times the first plus the second, whole at one and more, gone at nought.
    pub(crate) fn fade(&self) -> [f32; 2] {
        let over = (self.far * FADE).max(1e-3);
        [-1.0 / over, self.far / over]
    }

    /// The variables the receiving pass reads of the shadow for a viewer at `eye`: what one fit gives, with each cascade's own tile, the one depth of them all, and a fade to no shadow at the end of the reach.
    pub(crate) fn values(&self, eye: Vec3) -> BTreeMap<&'static str, Vec<f32>> {
        let tiles = self.fits.map(|fit| tile_of(&fit, eye));
        let mut values = self.fits[0].values(eye);
        let Some(shared) = values.get_mut(SHARED) else {
            return values;
        };
        // The pass works a place out from the eye, so the depth of the eye is part of the offset
        let deep = self.depth.truncate().length();
        let at_eye = self.depth.dot(eye.extend(1.0));
        for (cascade, tile) in tiles.iter().enumerate() {
            let Some((mut scale, mut offset)) = *tile else {
                continue;
            };
            // A tile's scale is one over its width in the world
            let bias = DEPTH_BIAS * deep / scale[0].max(f32::MIN_POSITIVE);
            (scale[2], offset[2]) = (deep, at_eye - bias);
            for (at, register) in [(SCALES, scale), (OFFSETS, offset)] {
                let from = at + cascade * 4;
                if let Some(place) = shared.get_mut(from..from + 4) {
                    place.copy_from_slice(&register);
                }
            }
        }
        for (at, word) in [3, 7].into_iter().zip(self.fade()) {
            if let Some(place) = shared.get_mut(at) {
                *place = word;
            }
        }
        values
    }
}

/// Whether a sphere at `centre` of `radius` reaches the tile `view` casts into, one of [`Cascades::views`]: its square across the light, at any depth along it.
// ⚠ Exact for what a cascade draws: a caster wholly outside its square leaves no fragment in its tile, whose viewport and scissor are the square.
pub(crate) fn reaches(view: &Mat4, centre: Vec3, radius: f32) -> bool {
    let clip = *view * centre.extend(1.0);
    // A tile's view is orthographic; anything else is taken as reaching it
    if (clip.w - 1.0).abs() > 1e-3 || !radius.is_finite() {
        return true;
    }
    let across = |row: usize| view.row(row).truncate().length() * radius;
    !(clip.x.abs() > 1.0 + across(0) || clip.y.abs() > 1.0 + across(1))
}

/// What the frame gives the cast of its shadow.
pub(super) struct Frame<'a> {
    pub draws: &'a [WorldDraw],
    pub instances: &'a [WorldInstance],
    /// The sphere about each instance, `[x, y, z, radius]`; one with none reaches every tile.
    pub spheres: &'a [[f32; 4]],
    /// The runs of `draws` one call casts.
    pub batches: &'a [Range<u32>],
    /// Whether draws are issued from a buffer of arguments.
    pub multi: bool,
}

/// What each cascade casts: the instances of the frame that reach its tile, side by side, and the arguments of each draw.
#[derive(Default)]
pub(super) struct Casting {
    /// The views and the set of the frame the lists were made for.
    of: Option<([[u32; 16]; CASCADES], u64)>,
    pub instances: Option<wgpu::Buffer>,
    pub indirect: Option<wgpu::Buffer>,
    /// For each cascade, each draw's instances in `instances`, and the runs of the cast that draw any.
    pub ranges: [Vec<Range<u32>>; CASCADES],
    pub batches: [Vec<Range<u32>>; CASCADES],
    laid: Vec<WorldInstance>,
    arguments: Vec<u8>,
}

/// A buffer in `slot` of `bytes` at least.
fn room(
    device: &wgpu::Device,
    slot: &mut Option<wgpu::Buffer>,
    label: &'static str,
    usage: wgpu::BufferUsages,
    bytes: u64,
) {
    let most = device.limits().max_buffer_size;
    if slot
        .as_ref()
        .is_none_or(|buffer| buffer.size() < bytes.min(most))
    {
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.next_power_of_two().max(4096).min(most),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
}

impl Casting {
    /// Make room for `instances` and `draws` in every cascade.
    pub(super) fn room(&mut self, device: &wgpu::Device, instances: usize, draws: usize) {
        let every = CASCADES as u64;
        let instance = std::mem::size_of::<WorldInstance>() as u64;
        let usage = wgpu::BufferUsages::VERTEX;
        let label = "viewport-world-cast-instances";
        room(
            device,
            &mut self.instances,
            label,
            usage,
            every * instances as u64 * instance,
        );
        let usage = wgpu::BufferUsages::INDIRECT;
        let label = "viewport-world-cast-draws";
        room(
            device,
            &mut self.indirect,
            label,
            usage,
            every * draws as u64 * ARGS,
        );
    }

    /// Make each cascade's lists for its view of `views`, unless they are made already for these views and the set `set` of the frame.
    pub(super) fn cast(
        &mut self,
        queue: &wgpu::Queue,
        views: &[Mat4; CASCADES],
        set: u64,
        frame: &Frame<'_>,
    ) {
        let of = (
            views.map(|view| view.to_cols_array().map(f32::to_bits)),
            set,
        );
        if self.of == Some(of) {
            return;
        }
        self.of = Some(of);
        (self.laid).clear();
        self.arguments.clear();
        // ⚠ The buffer is no larger than the device allows: what does not fit in it casts nothing.
        let instance = std::mem::size_of::<WorldInstance>() as u64;
        let most = (self.instances.as_ref()).map_or(0, |buffer| buffer.size() / instance) as usize;
        let mut left = 0usize;
        for (cascade, view) in views.iter().enumerate() {
            let ranges = &mut self.ranges[cascade];
            ranges.clear();
            for draw in frame.draws {
                let start = self.laid.len() as u32;
                if draw.mesh.material.bucket.casts() {
                    for at in draw.instances.clone() {
                        let sphere = frame.spheres.get(at as usize);
                        let near = |s: &[f32; 4]| reaches(view, Vec3::new(s[0], s[1], s[2]), s[3]);
                        let room = self.laid.len() < most;
                        if let (true, Some(instance)) =
                            (sphere.is_none_or(near), frame.instances.get(at as usize))
                        {
                            match room {
                                true => self.laid.push(*instance),
                                false => left += 1,
                            }
                        }
                    }
                }
                let range = start..self.laid.len() as u32;
                if frame.multi {
                    let call = wgpu::util::DrawIndexedIndirectArgs {
                        index_count: draw.mesh.index_count,
                        instance_count: range.end - range.start,
                        first_index: draw.mesh.first_index,
                        base_vertex: draw.mesh.base_vertex,
                        first_instance: range.start,
                    };
                    self.arguments.extend_from_slice(call.as_bytes());
                }
                ranges.push(range);
            }
            let draws_any = |batch: &&Range<u32>| {
                let of = ranges.get(batch.start as usize..batch.end as usize);
                of.is_some_and(|of| of.iter().any(|range| !range.is_empty()))
            };
            self.batches[cascade] = frame.batches.iter().filter(draws_any).cloned().collect();
        }
        if left > 0 {
            tracing::warn!(
                left,
                "the sun's shadow casts no more instances than the device's largest buffer holds: the rest cast none this frame"
            );
        }
        if let (Some(buffer), false) = (&self.instances, self.laid.is_empty()) {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.laid));
        }
        if let (Some(buffer), false) = (&self.indirect, self.arguments.is_empty()) {
            queue.write_buffer(buffer, 0, &self.arguments);
        }
    }
}

impl Casting {
    /// The calls and the triangles the four cascades cast together, of `draws` as the lists were made for them; none where the lists are of an earlier set of the frame, as where the sun does not shine.
    pub(super) fn cost(&self, draws: &[WorldDraw], multi: bool, set: u64) -> (usize, u64) {
        if self.of.is_none_or(|(_, of)| of != set) {
            return (0, 0);
        }
        let mut calls = 0;
        let mut triangles = 0;
        for (ranges, batches) in self.ranges.iter().zip(&self.batches) {
            calls += match multi {
                true => batches.len(),
                false => ranges.iter().filter(|range| !range.is_empty()).count(),
            };
            for (draw, range) in draws.iter().zip(ranges) {
                triangles +=
                    u64::from(draw.mesh.index_count / 3) * u64::from(range.end - range.start);
            }
        }
        (calls, triangles)
    }
}

/// The scale and the offset a fit gives the tile of every cascade, which are its own tile's.
fn tile_of(fit: &Fit, eye: Vec3) -> Option<([f32; 4], [f32; 4])> {
    let values = fit.values(eye);
    let shared = values.get(SHARED)?;
    let register = |at: usize| <[f32; 4]>::try_from(shared.get(at..at + 4)?).ok();
    Some((register(SCALES)?, register(OFFSETS)?))
}

/// A sphere a little larger than this one, its centre moved to a whole texel of the tile a fit about it has: the texels then stay where they are in the world as the camera moves, and a shadow's edge does not crawl.
fn snapped(travel: Vec3, centre: Vec3, radius: f32) -> (Vec3, f32) {
    // Room for the move, of half a texel at most each way
    let radius = radius * (1.0 + 2.0 / TILE as f32);
    let texel = 2.0 * radius / TILE as f32;
    // The first two rows of a caster's view are the axes of its tile
    let view = Fit::of(travel, centre, radius).caster().view;
    let moved = [0, 1].into_iter().fold(centre, |centre, row| {
        let axis = view.row(row).truncate();
        let along = centre.dot(axis);
        centre + axis * ((along / texel).round() * texel - along)
    });
    (moved, radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cascades_end_where_the_games_split_puts_them_scaled_by_the_cycles_multiplier() {
        let free = ShadowFocus::FREE;
        let one = ends(1.0, free);
        assert!(one.windows(2).all(|pair| pair[0] < pair[1]), "{one:?}");
        assert_eq!(one[CASCADES - 1], SHADOW_FAR);
        assert_eq!(shadow_reach(2.0, free), 2.0 * SHADOW_FAR);
        for (at, end) in one.iter().enumerate().take(CASCADES - 1) {
            let along = (at + 1) as f32 / CASCADES as f32;
            let even = SPLIT_NEAR + (SPLIT_FAR - SPLIT_NEAR) * along;
            let geometric = SPLIT_NEAR * (SPLIT_FAR / SPLIT_NEAR).powf(along);
            // Nine tenths of the way from the even split to the geometric one
            let drawn = (even - end) / (even - geometric);
            assert!((drawn - free.weight).abs() < 1e-4, "{at}: {drawn}");
            assert!((ends(2.0, free)[at] - 2.0 * end).abs() < 1e-3);
        }
    }

    #[test]
    fn an_aircrafts_cascades_reach_five_times_as_far_and_lie_nearer_the_geometric_split() {
        let (free, air) = (
            ends(1.0, ShadowFocus::FREE),
            ends(1.0, ShadowFocus::AIRCRAFT),
        );
        assert!(
            (air[CASCADES - 1] - 5.0 * SHADOW_FAR).abs() < 1e-2,
            "{air:?}"
        );
        assert!(air.windows(2).all(|pair| pair[0] < pair[1]), "{air:?}");
        // Each inner end of the aircraft's split is nearer the camera than five times the free one's.
        for at in 0..CASCADES - 1 {
            assert!(air[at] < 5.0 * free[at], "{at}: {air:?} {free:?}");
        }
    }

    #[test]
    fn a_caster_reaches_a_tile_while_its_sphere_overlaps_the_square_at_any_depth() {
        // A tile 20 m across about the origin, looking down the light along -Z
        let view = Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, 0.0, 100.0);
        assert!(reaches(&view, Vec3::ZERO, 1.0));
        let deep = Vec3::new(0.0, 0.0, -5000.0);
        assert!(reaches(&view, deep, 1.0), "whatever its depth");
        // Past the side by less than its radius, and by more
        assert!(reaches(&view, Vec3::new(10.5, 0.0, -1.0), 1.0));
        assert!(!reaches(&view, Vec3::new(11.5, 0.0, -1.0), 1.0));
        assert!(!reaches(&view, Vec3::new(0.0, -12.0, -1.0), 1.0));
        // Of no known size, it reaches every tile
        assert!(reaches(&view, Vec3::new(500.0, 0.0, 0.0), f32::INFINITY));
        assert!(reaches(&view, Vec3::new(500.0, 0.0, 0.0), f32::NAN));
    }
}
