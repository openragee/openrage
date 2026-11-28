//! What the frame hands a lit forward surface's pixel stage of the light it is lit by: the
//! directional light's record and the light from all around, as the game lays them out, made
//! once a scene and written again each time that light moves.

use std::f32::consts::PI;
use std::num::NonZeroU32;

use glam::Vec3;

use super::shared::Made;
use super::{Bound, Feed, Sky};
use crate::game_shaders::bindings::Resource;

/// A binding of a forward stage the frame fills, each time the light it is lit by moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameFeed {
    /// The game's record of the directional light.
    DirectionalLight,
    /// The light from all around, a probe's nine numbers as the game's seven slices hold them.
    AmbientMap,
    /// The game's lighting record of the view ([`ViewLighting`]).
    ViewLighting,
    /// The table of what a surface reflects of an even light, by the cosine to the eye across
    /// and the roughness down.
    EnvironmentFit,
}

/// The view's lighting record, 2,044 bytes, as a lit forward program reads it, with no local
/// probe of light: the pixel lies outside the one probe level it states.
// The places a lit program reads, each a field; what it does not read is padding, noughts.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ViewLighting {
    _head: [u32; 59],
    /// The way towards the lit body; the haze's probe is taken along it.
    pub towards: [f32; 3],
    _lux: [u32; 4],
    /// How far the light's roughness is taken towards one in the open, and in a cloud's shadow.
    // In the open at least ten times the disc's half angle, up to 0.03.
    pub roughness_mod: f32,
    pub roughness_mod_cloud_shadow: f32,
    /// The haze's strength, and how far a light the clouds hide is taken towards it.
    pub haze_intensity: f32,
    pub haze_blend_scale: f32,
    /// The tangent and the cosine of the angle the lit body's disc is seen half of.
    pub disc_tangent: f32,
    pub disc_cosine: f32,
    _before_fog: [u32; 68],
    /// How far blue noise moves the fog's lookup: nought while the game's jitter is off.
    pub fog_jitter: [f32; 2],
    _fog: [u32; 2],
    /// The depth the fog's slices reach, in metres.
    pub fog_reach: f32,
    /// The clouds' share past their map's edge, and what is added to the share through them.
    pub cloud_shadow_base: f32,
    pub cloud_shadow_lift: f32,
    _exponent: u32,
    /// The slices' spacing and their count less one: a slice is `log2(1 + d * count) * scale`.
    pub fog_slice_scale: f32,
    pub fog_slices: f32,
    _before_alpha: [u32; 97],
    /// The two distances a surface of a material that asks fades over.
    pub alpha_fade: [f32; 2],
    _alpha: u32,
    /// How far noise moves the place a probe level is tested at.
    pub probe_jitter: f32,
    _before_grid: [u32; 7],
    /// The probe levels' cells across and up, which divide that noise.
    pub probe_grid: [u32; 2],
    _before_levels: [u32; 40],
    /// The first level's place, which the scales times the world's place are added to.
    // The pixel is inside the level where each of the three sums lies within one.
    pub level_place: [f32; 3],
    pub level_scale: f32,
    _between_levels: [u32; 19],
    pub level_scale_z: f32,
    _before_count: [u32; 135],
    /// How many probe levels there are: both loops over them test before a second turn.
    pub levels: u32,
    _before_remap: [u32; 8],
    /// The glass's weight by depth: `saturate(d * [0] + [1]) * [2] + [3]`.
    pub glass_remap: [f32; 4],
    _before_ambient: [u32; 27],
    /// Where the ambient map lies over the world: `[2] * world + [0, 1]`.
    pub ambient_place: [f32; 2],
    pub ambient_scale: f32,
    /// What the pass's own reflection is multiplied by.
    pub reflection_scale: f32,
    _tail: [u32; 8],
}

const _: () = assert!(std::mem::size_of::<ViewLighting>() == 2044);

/// The bytes of the view's lighting record.
const VIEW_BYTES: usize = 2044;
/// The glass remap's four rows when no cycle states them, the cycle table's own: from nought to
/// five metres, a weight of one at both.
const GLASS_REMAP_ROWS: [f32; 4] = [0.0, 5.0, 1.0, 1.0];
/// The reflection's scale, a row of the cycle the editor's reader does not name: the table's own.
const REFLECTION_SCALE: f32 = 0.3;
/// The game's cap on the fog's reach, in metres.
const FOG_REACH: f32 = 10_000.0;

impl ViewLighting {
    /// The record for `sky`, with one probe level the pixel lies outside of.
    pub(super) fn of(sky: &Sky) -> Self {
        let towards = sky.towards.try_normalize().unwrap_or(Vec3::Z);
        let (roughness_mod, disc) = sky.tone.map_or((0.0, 0.0), |t| (t.roughness_mod, t.disc));
        let half_angle = disc.atan();
        let clouds = &sky.scene.clouds;
        let [near, far, least, most] = sky.scene.glass_remap.unwrap_or(GLASS_REMAP_ROWS);
        let by_depth = 1.0 / (far - near);
        Self {
            _head: [0; 59],
            towards: towards.to_array(),
            _lux: [0; 4],
            roughness_mod: roughness_mod.max((10.0 * half_angle).min(0.03)),
            roughness_mod_cloud_shadow: clouds.roughness_mod,
            haze_intensity: clouds.haze,
            haze_blend_scale: clouds.haze_blend,
            disc_tangent: disc,
            disc_cosine: half_angle.cos(),
            _before_fog: [0; 68],
            fog_jitter: [0.0; 2],
            _fog: [0; 2],
            fog_reach: FOG_REACH,
            // Through the map's texel or past its edge the share is the editor's.
            cloud_shadow_base: clouds.open,
            cloud_shadow_lift: clouds.open - 1.0,
            _exponent: 0,
            fog_slice_scale: 0.1,
            fog_slices: 1023.0,
            _before_alpha: [0; 97],
            alpha_fade: [0.2, 0.1],
            _alpha: 0,
            probe_jitter: 0.25,
            _before_grid: [0; 7],
            // ⚠ A grid of nought makes the level test's bounds not a number, and the pixel
            // inside every level.
            probe_grid: [1, 1],
            _before_levels: [0; 40],
            level_place: [2.0; 3],
            level_scale: 0.0,
            _between_levels: [0; 19],
            level_scale_z: 0.0,
            _before_count: [0; 135],
            // ⚠ Nought is no rest: the count less one wraps, and the loops run on past the record.
            levels: 1,
            _before_remap: [0; 8],
            glass_remap: [by_depth, -near * by_depth, most - least, least],
            _before_ambient: [0; 27],
            ambient_place: [0.0; 2],
            ambient_scale: 0.0,
            reflection_scale: REFLECTION_SCALE,
            _tail: [0; 8],
        }
    }

    pub(super) fn bytes(&self) -> [u8; VIEW_BYTES] {
        use std::mem::offset_of;
        let mut bytes = [0u8; VIEW_BYTES];
        let mut put = |at: usize, words: &[u32]| {
            for (i, word) in words.iter().enumerate() {
                bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&word.to_le_bytes());
            }
        };
        let bits = |floats: &[f32]| floats.iter().map(|f| f.to_bits()).collect::<Vec<u32>>();
        put(offset_of!(Self, towards), &bits(&self.towards));
        let direct = [
            self.roughness_mod,
            self.roughness_mod_cloud_shadow,
            self.haze_intensity,
            self.haze_blend_scale,
            self.disc_tangent,
            self.disc_cosine,
        ];
        put(offset_of!(Self, roughness_mod), &bits(&direct));
        put(offset_of!(Self, fog_jitter), &bits(&self.fog_jitter));
        let fog = [
            self.fog_reach,
            self.cloud_shadow_base,
            self.cloud_shadow_lift,
        ];
        put(offset_of!(Self, fog_reach), &bits(&fog));
        let slices = [self.fog_slice_scale, self.fog_slices];
        put(offset_of!(Self, fog_slice_scale), &bits(&slices));
        put(offset_of!(Self, alpha_fade), &bits(&self.alpha_fade));
        put(offset_of!(Self, probe_jitter), &bits(&[self.probe_jitter]));
        put(offset_of!(Self, probe_grid), &self.probe_grid);
        put(offset_of!(Self, level_place), &bits(&self.level_place));
        put(offset_of!(Self, level_scale), &bits(&[self.level_scale]));
        put(
            offset_of!(Self, level_scale_z),
            &bits(&[self.level_scale_z]),
        );
        put(offset_of!(Self, levels), &[self.levels]);
        put(offset_of!(Self, glass_remap), &bits(&self.glass_remap));
        let ambient = [
            self.ambient_place[0],
            self.ambient_place[1],
            self.ambient_scale,
            self.reflection_scale,
        ];
        put(offset_of!(Self, ambient_place), &bits(&ambient));
        bytes
    }
}

/// The environment fit's table as a texture: each texel the fit at its middle, so that a lookup
/// filtered between texels gives the fit where it is sampled.
fn fit_texels() -> &'static [u8] {
    static TEXELS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    TEXELS.get_or_init(|| {
        let side = super::env_fit::SIDE;
        let middle = |place: usize| (place as f32 + 0.5) / side as f32;
        (0..side * side)
            .flat_map(|i| {
                let [grazing, whole, matte] =
                    super::env_fit::fit_at(middle(i % side), middle(i / side));
                [grazing, whole, matte, 1.0]
            })
            .flat_map(|value| or_io::half::f32_to_half(value).to_le_bytes())
            .collect()
    })
}

/// The bytes of the directional light's record.
const RECORD_BYTES: usize = 340;
// Where the way the light travels, its light, its strength and an interior's tint lie in it.
const TRAVELS: usize = 0x110;
const LIGHT: usize = 0x12C;
const STRENGTH: usize = 0x138;
const TINT: usize = 0x148;
/// The slices of the ambient map, four halves each.
const SLICES: u32 = 7;

/// The directional light's record for `sky`.
// The lit program shades by the opposite of the way at +0x110 and multiplies the light at +0x12C
// by the ambient cube's scalar and by the air's row, which the binder states as one: the editor's
// whole sun is stated here. The strength at +0x138 is loaded and not read; an interior's tint is
// taken towards by the interior's share, and one leaves the light as it is whatever that share.
pub(super) fn directional_light(sky: &Sky) -> [u8; RECORD_BYTES] {
    let towards = sky.towards.try_normalize().unwrap_or(Vec3::Z);
    let mut bytes = [0u8; RECORD_BYTES];
    let mut put = |at: usize, floats: &[f32]| {
        for (i, float) in floats.iter().enumerate() {
            bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&float.to_le_bytes());
        }
    };
    put(TRAVELS, &(-towards).to_array());
    put(LIGHT, &sky.sun.to_array());
    put(STRENGTH, &[1.0]);
    put(TINT, &[1.0; 3]);
    bytes
}

/// The nine numbers that give back the sky over the ground as the resolve takes two colours: on
/// a surface that faces `n`, pi times the ground mixed with the sky by `n.z * 0.5 + 0.5`.
fn two_colours(sky: Vec3, ground: Vec3) -> [Vec3; 9] {
    let mut nine = [Vec3::ZERO; 9];
    nine[0] = (sky + ground) * (PI / 2.0 / 0.886_227);
    nine[2] = (sky - ground) * (PI / 2.0 / 1.023_328);
    nine
}

/// The seven slices of the ambient map for `sky`: each colour of each of the probe's nine numbers
/// at the channel of the slice the lit program reads it from.
// Numbers by [`super::nine_of`]'s order: the even one, y, z, x, xy, yz, 3z²-1, xz, x²-y².
pub(super) fn ambient_slices(sky: &Sky) -> [[f32; 4]; SLICES as usize] {
    let nine = (sky.scene.around).unwrap_or_else(|| two_colours(sky.sky, sky.ground));
    let slice = |rgb: Vec3, fourth: f32| [rgb.x, rgb.y, rgb.z, fourth];
    [
        slice(nine[0], nine[3].x),
        slice(nine[1], nine[3].y),
        slice(nine[2], nine[3].z),
        slice(nine[7], nine[8].x),
        slice(nine[5], nine[8].y),
        slice(nine[4], nine[8].z),
        slice(nine[6], 0.0),
    ]
}

/// The buffers and the texture a scene made of the frame's light.
#[derive(Default)]
pub(super) struct FrameHeld {
    /// Each made as large as the first stage that bound it asked.
    lights: Vec<wgpu::Buffer>,
    ambient: Option<wgpu::Texture>,
    /// The view's lighting record, each as large as the first stage that bound it asked.
    views: Vec<wgpu::Buffer>,
    fit: Option<wgpu::Texture>,
}

/// A buffer of at least `size` bytes, and of `least`, from `held` or made into it.
fn record(
    device: &wgpu::Device,
    held: &mut Vec<wgpu::Buffer>,
    size: u64,
    least: usize,
) -> wgpu::Buffer {
    let size = size.max(least as u64).next_multiple_of(16);
    if let Some(buffer) = held.iter().find(|buffer| buffer.size() >= size) {
        return buffer.clone();
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("meshlet-frame-record"),
        size,
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    held.push(buffer.clone());
    buffer
}

impl FrameHeld {
    /// A buffer of at least `size` bytes the directional light's record is written into.
    fn light(&mut self, device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        record(device, &mut self.lights, size, RECORD_BYTES)
    }

    /// A buffer of at least `size` bytes the view's lighting record is written into.
    fn view(&mut self, device: &wgpu::Device, size: u64) -> wgpu::Buffer {
        record(device, &mut self.views, size, VIEW_BYTES)
    }

    /// The environment fit's table.
    fn fit(&mut self, device: &wgpu::Device) -> wgpu::TextureView {
        let side = super::env_fit::SIDE as u32;
        let texture = self.fit.get_or_insert_with(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("meshlet-environment-fit"),
                size: wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// The ambient map as an array of slices.
    fn ambient(&mut self, device: &wgpu::Device) -> wgpu::TextureView {
        let texture = self.ambient.get_or_insert_with(|| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("meshlet-ambient-map"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: SLICES,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        })
    }

    /// Write what `sky` lights by into each of them made: how many writes, and their bytes.
    pub(super) fn write(&self, queue: &wgpu::Queue, sky: &Sky) -> (u32, u64) {
        let (mut writes, mut bytes) = (0, 0);
        if !self.lights.is_empty() {
            let record = directional_light(sky);
            for buffer in &self.lights {
                queue.write_buffer(buffer, 0, &record);
                writes += 1;
                bytes += record.len() as u64;
            }
        }
        if let Some(texture) = &self.ambient {
            let texels: Vec<u8> = (ambient_slices(sky).iter().flatten())
                .flat_map(|value| or_io::half::f32_to_half(*value).to_le_bytes())
                .collect();
            queue.write_texture(
                texture.as_image_copy(),
                &texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: SLICES,
                },
            );
            writes += 1;
            bytes += texels.len() as u64;
        }
        if !self.views.is_empty() {
            let record = ViewLighting::of(sky).bytes();
            for buffer in &self.views {
                queue.write_buffer(buffer, 0, &record);
                writes += 1;
                bytes += record.len() as u64;
            }
        }
        // ponytail: the table is the same every frame and is written again with the light, 2 KiB;
        // write it once when it is made if a frame's writes are counted that finely.
        if let Some(texture) = &self.fit {
            let side = super::env_fit::SIDE as u32;
            let texels = fit_texels();
            queue.write_texture(
                texture.as_image_copy(),
                texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8 * side),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
            writes += 1;
            bytes += texels.len() as u64;
        }
        (writes, bytes)
    }
}

/// What a binding of `resource` fed `feed` is bound to where the frame fills it, made into
/// `buffers` or `textures`; `None` where the frame does not, or not for that kind of binding.
pub(super) fn bound(
    device: &wgpu::Device,
    feed: Option<&Feed>,
    resource: &Resource,
    count: Option<NonZeroU32>,
    made: &mut Made,
    buffers: &mut Vec<wgpu::Buffer>,
    textures: &mut Vec<wgpu::TextureView>,
) -> Option<Bound> {
    let Some(Feed::Frame(frame)) = feed else {
        return None;
    };
    match (frame, resource) {
        (
            FrameFeed::DirectionalLight,
            Resource::Uniform { size } | Resource::Storage { size, .. },
        ) => {
            buffers.push(made.frame.light(device, *size));
            Some(match count {
                Some(entries) => Bound::Buffers(buffers.len() - 1, entries.get() as usize),
                None => Bound::Buffer(buffers.len() - 1),
            })
        }
        // Sampled as the game samples it: filtered, slice by slice.
        (
            FrameFeed::AmbientMap,
            Resource::Texture {
                dimension: wgpu::TextureViewDimension::D2Array,
                sample: wgpu::TextureSampleType::Float { .. },
                multisampled: false,
            },
        ) if count.is_none() => {
            textures.push(made.frame.ambient(device));
            Some(Bound::View(textures.len() - 1))
        }
        (FrameFeed::ViewLighting, Resource::Uniform { size } | Resource::Storage { size, .. }) => {
            buffers.push(made.frame.view(device, *size));
            Some(match count {
                Some(entries) => Bound::Buffers(buffers.len() - 1, entries.get() as usize),
                None => Bound::Buffer(buffers.len() - 1),
            })
        }
        // Sampled filtered, as the game samples its own.
        (
            FrameFeed::EnvironmentFit,
            Resource::Texture {
                dimension: wgpu::TextureViewDimension::D2,
                sample: wgpu::TextureSampleType::Float { .. },
                multisampled: false,
            },
        ) if count.is_none() => {
            textures.push(made.frame.fit(device));
            Some(Bound::View(textures.len() - 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{falls_on, Scene};
    use super::*;

    fn float(bytes: &[u8], at: usize) -> f32 {
        f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
    }

    #[test]
    fn the_directional_light_s_record_holds_the_editor_s_sun_where_the_lit_program_reads_it() {
        let sky = Sky {
            towards: Vec3::new(0.0, 3.0, 4.0),
            sun: Vec3::new(2.0, 1.5, 1.0),
            ..Sky::default()
        };
        let record = directional_light(&sky);
        let vec = |at: usize| {
            Vec3::new(
                float(&record, at),
                float(&record, at + 4),
                float(&record, at + 8),
            )
        };
        // The way the light travels: from the sun, unit length.
        let travels = vec(0x110);
        assert!(
            (travels - Vec3::new(0.0, -0.6, -0.8)).length() < 1e-6,
            "{travels}"
        );
        assert_eq!(vec(0x12C), sky.sun);
        assert_eq!(vec(0x148), Vec3::ONE);
        let read = [0x110..0x11C, 0x12C..0x13C, 0x148..0x154];
        let mut elsewhere = (0..RECORD_BYTES).filter(|b| !read.iter().any(|r| r.contains(b)));
        assert!(elsewhere.all(|b| record[b] == 0));
        // The light moved: the record the frame writes moves with it.
        let moved = Sky {
            towards: Vec3::X,
            ..sky
        };
        assert_ne!(directional_light(&moved), record);
    }

    #[test]
    fn the_frame_s_light_is_written_again_and_its_surfaces_drawn_again_when_the_light_moves() {
        use super::super::{forward::redraws, light_bytes, Kept, Lighting};
        let lit = |towards: Vec3| Lighting {
            view_proj: glam::Mat4::IDENTITY,
            sky: Sky {
                towards,
                ..Sky::default()
            },
            camera: Vec3::Z,
        };
        let mut kept = Kept::default();
        // Whether the frame's records are written (the scene writes them with its light), and
        // whether the forward surfaces are drawn again over the frame lit anew.
        let mut take = |lighting: &Lighting| {
            let writes = kept.take([0; 64], None, &light_bytes(lighting, false));
            (writes.light, redraws(kept.draws()))
        };
        let (first, moved) = (lit(Vec3::Z), lit(Vec3::new(0.6, 0.0, 0.8)));
        assert_eq!(take(&first), (true, true), "the first frame");
        assert_eq!(take(&first), (false, false), "at rest");
        assert_eq!(take(&moved), (true, true), "the sun moved");
        assert_ne!(directional_light(&first.sky), directional_light(&moved.sky));
    }

    #[test]
    fn the_view_s_lighting_record_holds_each_field_at_the_byte_the_lit_program_reads() {
        use std::mem::offset_of;
        let at = [
            (offset_of!(ViewLighting, towards), 0xEC),
            (offset_of!(ViewLighting, roughness_mod), 0x108),
            (offset_of!(ViewLighting, roughness_mod_cloud_shadow), 0x10C),
            (offset_of!(ViewLighting, haze_intensity), 0x110),
            (offset_of!(ViewLighting, haze_blend_scale), 0x114),
            (offset_of!(ViewLighting, disc_tangent), 0x118),
            (offset_of!(ViewLighting, disc_cosine), 0x11C),
            (offset_of!(ViewLighting, fog_jitter), 0x230),
            (offset_of!(ViewLighting, fog_reach), 0x240),
            (offset_of!(ViewLighting, cloud_shadow_base), 0x244),
            (offset_of!(ViewLighting, cloud_shadow_lift), 0x248),
            (offset_of!(ViewLighting, fog_slice_scale), 0x250),
            (offset_of!(ViewLighting, fog_slices), 0x254),
            (offset_of!(ViewLighting, alpha_fade), 0x3DC),
            (offset_of!(ViewLighting, probe_jitter), 0x3E8),
            (offset_of!(ViewLighting, probe_grid), 0x408),
            (offset_of!(ViewLighting, level_place), 0x4B0),
            (offset_of!(ViewLighting, level_scale), 0x4BC),
            (offset_of!(ViewLighting, level_scale_z), 0x50C),
            (offset_of!(ViewLighting, levels), 0x72C),
            (offset_of!(ViewLighting, glass_remap), 0x750),
            (offset_of!(ViewLighting, ambient_place), 0x7CC),
            (offset_of!(ViewLighting, ambient_scale), 0x7D4),
            (offset_of!(ViewLighting, reflection_scale), 0x7D8),
        ];
        for (field, byte) in at {
            assert_eq!(field, byte, "0x{byte:X}");
        }
        // The bytes put each field where the struct lays it.
        let record = ViewLighting::of(&Sky::default());
        let bytes = record.bytes();
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four"));
        assert_eq!(word(0x72C), record.levels);
        assert_eq!(float(&bytes, 0x7D8), record.reflection_scale);
        assert_eq!(float(&bytes, 0x4B8), record.level_place[2]);
        assert!(
            bytes[..0xEC].iter().all(|b| *b == 0),
            "nothing before the light's way"
        );
    }

    #[test]
    fn the_view_s_record_states_one_probe_level_the_pixel_lies_outside_of() {
        let record = ViewLighting::of(&Sky::default());
        // The loops go on while the count less one is past the level: one level ends both after
        // their first test, where nought would wrap to the most a word holds.
        assert_eq!(record.levels, 1);
        // The pixel's place in the level is the level's place alone, two along each axis: past
        // one, so outside, and the probes' blocks are passed over for the ambient map.
        assert_eq!(record.level_place, [2.0; 3]);
        assert_eq!((record.level_scale, record.level_scale_z), (0.0, 0.0));
        // The jitter's divisors are not nought, so the bounds it moves stay numbers.
        assert!(record.probe_grid.iter().all(|cells| *cells > 0));
        assert_eq!(record.probe_jitter, 0.25);
        // The writer's constants and the cycle table's defaults where no cycle states them.
        assert_eq!(record.alpha_fade, [0.2, 0.1]);
        assert_eq!((record.fog_slice_scale, record.fog_slices), (0.1, 1023.0));
        assert_eq!(record.glass_remap, [0.2, 0.0, 0.0, 1.0]);
        assert_eq!(record.reflection_scale, 0.3);
    }

    #[test]
    fn the_view_s_record_carries_the_editor_s_sun_clouds_and_remap() {
        let mut sky = Sky {
            towards: Vec3::new(0.0, 3.0, 4.0),
            ..Sky::default()
        };
        sky.scene.clouds.open = 0.4;
        sky.scene.clouds.haze = 2.0;
        sky.scene.glass_remap = Some([1.0, 3.0, 0.5, 0.75]);
        let record = ViewLighting::of(&sky);
        // The way towards the light, unlike the directional record's way it travels.
        let towards = Vec3::from_array(record.towards);
        assert!(
            (towards - Vec3::new(0.0, 0.6, 0.8)).length() < 1e-6,
            "{towards}"
        );
        assert_eq!(record.haze_intensity, 2.0);
        // Through the map's texel (`1 + lift`) or past its edge (`base`) the share is the editor's.
        assert!((1.0 + record.cloud_shadow_lift - 0.4).abs() < 1e-6);
        assert_eq!(record.cloud_shadow_base, 0.4);
        // `saturate(d * a + b) * c + d`: nought at one metre, one at three, from 0.5 to 0.75.
        let [a, b, c, d] = record.glass_remap;
        let weight = |depth: f32| (depth * a + b).clamp(0.0, 1.0) * c + d;
        assert_eq!((weight(1.0), weight(3.0)), (0.5, 0.75));
        // No cycle: no disc, a cosine of one.
        assert_eq!((record.disc_tangent, record.disc_cosine), (0.0, 1.0));
        assert_ne!(ViewLighting::of(&Sky::default()), record, "the sun moved");
    }

    #[test]
    fn the_frame_s_records_are_written_again_when_the_camera_moves() {
        use super::super::{light_bytes, Kept, Lighting};
        let lit = |camera: Vec3| Lighting {
            view_proj: glam::Mat4::IDENTITY,
            sky: Sky::default(),
            camera,
        };
        let mut kept = Kept::default();
        let mut take = |lighting: &Lighting| {
            let light = light_bytes(lighting, false);
            kept.take([0; 64], None, &light).light
        };
        assert!(take(&lit(Vec3::Z)), "the first frame");
        assert!(!take(&lit(Vec3::Z)), "at rest");
        assert!(take(&lit(Vec3::new(4.0, 0.0, 1.0))), "the camera moved");
    }

    #[test]
    fn the_environment_fit_s_texels_hold_the_fit_at_their_middles() {
        let texels = fit_texels();
        let side = super::super::env_fit::SIDE;
        assert_eq!(texels.len(), side * side * 8);
        let half =
            |at: usize| or_io::half::half_to_f32(u16::from_le_bytes([texels[at], texels[at + 1]]));
        let middle = 0.5 / side as f32;
        let [grazing, whole, matte] = super::super::env_fit::fit_at(middle, middle);
        for (channel, wanted) in [grazing, whole, matte].into_iter().enumerate() {
            assert!((half(2 * channel) - wanted).abs() < 2e-3, "{channel}");
        }
    }

    /// What the lit program takes of the slices along `d`, in the order and with the constants
    /// of its instructions: the even number, the three that lean, the three products, then the
    /// two that bend.
    fn as_the_program_reads(slices: &[[f32; 4]; 7], d: Vec3) -> Vec3 {
        let rgb = |s: usize| Vec3::new(slices[s][0], slices[s][1], slices[s][2]);
        let fourth =
            |a: usize, b: usize, c: usize| Vec3::new(slices[a][3], slices[b][3], slices[c][3]);
        let Vec3 { x, y, z } = d;
        0.886_227 * rgb(0)
            + 1.023_328 * z * rgb(2)
            + 1.023_328 * y * rgb(1)
            + 1.023_328 * x * fourth(0, 1, 2)
            + 0.858_086 * y * z * rgb(4)
            + 0.858_086 * x * z * rgb(3)
            + 0.858_086 * x * y * rgb(5)
            + (0.743_125 * z * z - 0.247_708) * rgb(6)
            + 0.429_043 * (x * x - y * y) * fourth(3, 4, 5)
    }

    #[test]
    fn the_ambient_map_s_slices_give_back_what_the_probe_s_nine_numbers_give() {
        // Every number its own, the even one large enough that nothing falls under nought.
        let nine: [Vec3; 9] = std::array::from_fn(|i| match i {
            0 => Vec3::new(5.0, 6.0, 7.0),
            _ => Vec3::new(0.1, 0.05, -0.02) * i as f32,
        });
        let sky = Sky {
            scene: Scene {
                around: Some(nine),
                ..Scene::default()
            },
            ..Sky::default()
        };
        let slices = ambient_slices(&sky);
        for way in [
            Vec3::Z,
            Vec3::new(0.48, 0.6, 0.64),
            Vec3::new(-0.8, 0.0, -0.6),
        ] {
            let (read, given) = (as_the_program_reads(&slices, way), falls_on(&nine, way));
            assert!(
                (read - given).abs().max_element() < 1e-3,
                "{way}: {read} {given}"
            );
        }
        // Where the sky and the ground are two colours: what the resolve gives a surface of them.
        let two = Sky::default();
        let slices = ambient_slices(&two);
        for way in [Vec3::Z, -Vec3::Z, Vec3::X, Vec3::new(0.0, 0.6, 0.8)] {
            let wanted = two.ground.lerp(two.sky, way.z * 0.5 + 0.5) * PI;
            let read = as_the_program_reads(&slices, way);
            assert!(
                (read - wanted).abs().max_element() < 1e-4,
                "{way}: {read} {wanted}"
            );
        }
        assert_ne!(ambient_slices(&Sky::at(19.0, 0.5)), slices, "the sky moved");
    }
}
