//! The world's textures on the device: each as its own block-compressed rows where the device
//! reads them and decoded where it does not, and the size on screen each is asked for at.

use std::collections::HashMap;

use glam::{Vec2, Vec3};
use or_io::codec::texture::{decode_texture_data, GrcTextureFormat};
use or_renderer::MeshVertex;
use rayon::prelude::*;

use super::budget::{evicted, Idle};
use crate::state::gta6_world::worldview::CpuTexture;

/// The shortest side a texture is asked for at, in pixels: under it a larger answer saves nothing worth a second read.
const LEAST_SIDE: u32 = 128;
/// The nearest a surface is taken to be to the camera, in metres.
const NEAREST: f32 = 0.5;
/// How many triangles of a mesh are measured for how often its texture turns.
const SAMPLED: usize = 64;

/// What a material is told of a map that is not on the device: not arrived yet, or never to come.
pub(super) const ABSENT: u64 = 0;
pub(super) const MISSING: u64 = 1;

/// A texture on the device.
pub(super) struct Ready {
    pub view: wgpu::TextureView,
    pub bytes: u64,
    /// The frame a material that draws last sampled it in, and where the nearest thing it drew stood.
    pub used: u64,
    pub at: Vec3,
    /// The size of the ask it answers, and whether no larger ask gets more.
    pub got: u32,
    pub top: bool,
    /// What a material made with it remembers it by: another for every arrival.
    pub stamp: u64,
}

/// A texture on the device, or one no pack holds.
pub(super) enum Texture {
    Ready(Ready),
    Missing,
}

/// The bytes one level of `format` takes, of whole blocks where it is made of blocks.
fn level_bytes(format: wgpu::TextureFormat, (width, height): (u32, u32)) -> Option<u64> {
    let (block_width, block_height) = format.block_dimensions();
    let block = u64::from(format.block_copy_size(None)?);
    Some(u64::from(width.div_ceil(block_width)) * u64::from(height.div_ceil(block_height)) * block)
}

/// What the tree's decoder calls a block-compressed format.
fn block_format(format: wgpu::TextureFormat) -> Option<GrcTextureFormat> {
    use wgpu::TextureFormat as F;
    Some(match format {
        F::Bc1RgbaUnorm | F::Bc1RgbaUnormSrgb => GrcTextureFormat::DXT1,
        F::Bc2RgbaUnorm | F::Bc2RgbaUnormSrgb => GrcTextureFormat::DXT3,
        F::Bc3RgbaUnorm | F::Bc3RgbaUnormSrgb => GrcTextureFormat::DXT5,
        F::Bc4RUnorm => GrcTextureFormat::DXT5A,
        F::Bc5RgUnorm => GrcTextureFormat::DXN,
        F::Bc7RgbaUnorm | F::Bc7RgbaUnormSrgb => GrcTextureFormat::BC7,
        _ => return None,
    })
}

fn side(texture: &CpuTexture, level: u32) -> (u32, u32) {
    (
        (texture.width >> level).max(1),
        (texture.height >> level).max(1),
    )
}

/// How many levels of `texture`, from its first, hold exactly the bytes of their size.
fn whole_levels(texture: &CpuTexture) -> usize {
    // A texture has no more levels than halvings bring its longest side to one pixel.
    let most = 32 - texture.width.max(texture.height).leading_zeros();
    (texture.levels.iter().enumerate())
        .take(most as usize)
        .take_while(|(level, rows)| {
            level_bytes(texture.format, side(texture, *level as u32)) == Some(rows.len() as u64)
        })
        .count()
}

/// `texture` on the device with every whole level it has, and the bytes it holds there; `None`
/// for a format the device and the decoder do not take, for rows that are not whole, and for a
/// side longer than `most_side`. `bc` says the device reads block-compressed rows as they are.
// ponytail: a device without block compression decodes on the frame's thread; on a worker if such a device shows the cost.
pub(super) fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &CpuTexture,
    bc: bool,
    most_side: u32,
) -> Option<(wgpu::TextureView, u64)> {
    use wgpu::TextureFormat as F;
    let (format, size) = (texture.format, (texture.width, texture.height));
    if size.0 == 0 || size.1 == 0 || size.0.max(size.1) > most_side {
        return None;
    }
    let levels = &texture.levels[..whole_levels(texture)];
    if levels.is_empty() {
        return None;
    }
    if matches!(format, F::Rgba8Unorm | F::Rgba8UnormSrgb) {
        return Some(write(device, queue, format, size, levels));
    }
    let decoder = block_format(format)?;
    // wgpu takes a block-compressed texture only where its first level is whole blocks.
    if bc && size.0.is_multiple_of(4) && size.1.is_multiple_of(4) {
        return Some(write(device, queue, format, size, levels));
    }
    let decoded: Option<Vec<Vec<u8>>> = (levels.par_iter().enumerate())
        .map(|(level, rows)| {
            let (width, height) = side(texture, level as u32);
            let image = decode_texture_data(decoder, width, height, rows).ok()?;
            (image.data.len() as u64 == u64::from(width) * u64::from(height) * 4)
                .then_some(image.data)
        })
        .collect();
    let plain = match format.is_srgb() {
        true => F::Rgba8UnormSrgb,
        false => F::Rgba8Unorm,
    };
    Some(write(device, queue, plain, size, &decoded?))
}

/// A texture of `format` made of `levels`, each of exactly the bytes of its size, and those bytes.
fn write(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    levels: &[Vec<u8>],
) -> (wgpu::TextureView, u64) {
    let extent = |level: u32| wgpu::Extent3d {
        width: (width >> level).max(1),
        height: (height >> level).max(1),
        depth_or_array_layers: 1,
    };
    let on_device = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("world-texture"),
        size: extent(0),
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (block_width, block_height) = format.block_dimensions();
    let block = format.block_copy_size(None).unwrap_or(4);
    let mut bytes = 0;
    for (level, rows) in levels.iter().enumerate() {
        // A level smaller than a block is written as the whole block it is stored as.
        let whole = extent(level as u32).physical_size(format);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &on_device,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rows,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(whole.width / block_width * block),
                rows_per_image: Some(whole.height / block_height),
            },
            whole,
        );
        bytes += rows.len() as u64;
    }
    (on_device.create_view(&Default::default()), bytes)
}

/// The widest row a model's palette is laid in: a longer palette runs on into the next row.
const PALETTE_ROW: usize = 4096;

/// The side and the rows a palette of `colours` is laid in on a device whose textures are at most `most` a side; colours past the last row the device makes are left out, and read as none.
fn palette_size(colours: usize, most: u32) -> (usize, usize) {
    let most = (most as usize).max(1);
    let width = colours.clamp(1, PALETTE_ROW.min(most));
    let rows = colours.div_ceil(width).clamp(1, most);
    (width, rows)
}

/// A model's own tint palette on the device: its colours as stored, which the mesh stage reads unconverted, the last row filled out with white, which reads as no colour; no side longer than `most`.
pub(super) fn palette(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colours: &[[u8; 4]],
    most: u32,
) -> wgpu::TextureView {
    let (width, rows) = palette_size(colours.len(), most);
    if width * rows < colours.len() {
        tracing::warn!(colours = colours.len(), most, "a palette is longer than the device's largest texture holds: its last colours read as none");
    }
    let mut bytes: Vec<u8> = colours
        .iter()
        .take(width * rows)
        .flatten()
        .copied()
        .collect();
    bytes.resize(width * rows * 4, 255);
    let size = (width as u32, rows as u32);
    write(
        device,
        queue,
        wgpu::TextureFormat::Rgba8Unorm,
        size,
        &[bytes],
    )
    .0
}

/// The tint palettes the models on the device hold themselves, by the key their materials name each by, each with how many of those models hold it.
#[derive(Default)]
pub(super) struct Palettes(HashMap<u64, (wgpu::TextureView, u32)>);

impl Palettes {
    /// How many palettes are on the device.
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// One more model holds the palette of `key`, of `colours`: on the device once, of no side longer than `most`.
    pub(super) fn hold(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        key: u64,
        colours: &[[u8; 4]],
        most: u32,
    ) {
        let held =
            (self.0.entry(key)).or_insert_with(|| (palette(device, queue, colours, most), 0));
        held.1 += 1;
    }

    /// One model fewer holds the palette of `key`: it leaves the device with the last.
    pub(super) fn release(&mut self, key: u64) {
        if let Some(held) = self.0.get_mut(&key) {
            held.1 = held.1.saturating_sub(1);
            if held.1 == 0 {
                self.0.remove(&key);
            }
        }
    }

    pub(super) fn get(&self, key: &u64) -> Option<&wgpu::TextureView> {
        self.0.get(key).map(|(view, _)| view)
    }

    pub(super) fn contains_key(&self, key: &u64) -> bool {
        self.0.contains_key(key)
    }
}

/// How often a mesh's texture turns over its surface, in turns a metre: the square root of the
/// UV area over the surface area of a sample of its triangles; nought for a mesh of no surface.
pub(super) fn density(vertices: &[MeshVertex], indices: &[u32]) -> f32 {
    let triangles = indices.len() / 3;
    let (mut turned, mut surface) = (0.0f32, 0.0f32);
    for triangle in (0..triangles).step_by(triangles.div_ceil(SAMPLED).max(1)) {
        let corner = |k: usize| vertices.get(indices[triangle * 3 + k] as usize);
        let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
            continue;
        };
        let at = |v: &MeshVertex| (Vec3::from(v.position), Vec2::from(v.uv));
        let ((pa, ta), (pb, tb), (pc, tc)) = (at(a), at(b), at(c));
        surface += (pb - pa).cross(pc - pa).length();
        turned += (tb - ta).perp_dot(tc - ta).abs();
    }
    if surface > 0.0 {
        (turned / surface).sqrt()
    } else {
        0.0
    }
}

/// How many pixels a metre covers on a screen `height` pixels high, a metre from a camera of `fov_y`.
pub(super) fn per_metre(height: u32, fov_y: f32) -> f32 {
    height as f32 / (2.0 * (fov_y / 2.0).tan())
}

/// The longest side, in pixels, a texture is needed at for a texel of it to cover a pixel: it
/// turns `density` times a metre over a surface `away` metres off and `scale` times its own
/// size, a metre of which covers `per_metre` pixels a metre away. A power of two, at most `most`.
pub(super) fn pixels(density: f32, away: f32, scale: f32, per_metre: f32, most: u32) -> u32 {
    // One turn of the texture covers `scale / density` metres.
    let needed = per_metre * scale / (density * away.max(NEAREST));
    // A mesh whose texture does not turn shows one texel of it; one that turns without end is asked at the most.
    let needed = if density > 0.0 && needed.is_finite() {
        needed.ceil().min(most as f32) as u32
    } else if density > 0.0 {
        most
    } else {
        0
    };
    needed
        .clamp(LEAST_SIDE.min(most), most)
        .next_power_of_two()
        .min(most)
}

/// The world's textures on the device, by the key each is found by in the packs.
#[derive(Default)]
pub(super) struct Textures {
    pub map: HashMap<u64, Texture>,
    /// The bytes those on the device hold.
    pub bytes: u64,
    stamps: u64,
    grey: Option<wgpu::TextureView>,
    /// The size each key is waited for at.
    asked: HashMap<u64, u32>,
}

impl Textures {
    /// Hold each ask of `wanted`, one a key, to the size its key is waited for at already, and
    /// remember what is waited for now.
    ///
    /// ⚠ The readers answer each size of a key apart: an ask that grew with every step of a
    /// camera nearing a wall would have the texture read once a step.
    pub(super) fn hold(&mut self, wanted: &mut [(f32, u64, u32)]) {
        for (_, key, pixels) in wanted.iter_mut() {
            if let Some(waited) = self.asked.get(key) {
                *pixels = *waited;
            }
        }
        self.asked = (wanted.iter())
            .map(|(_, key, pixels)| (*key, *pixels))
            .collect();
    }

    /// Take the answer to an ask for `key` at `pixels`, read as `read`, on a device that reads
    /// block compression where `bc` and makes no side longer than `most_side`; the bytes it put
    /// on the device.
    pub(super) fn arrive(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        (key, pixels): (u64, u32),
        read: Option<&CpuTexture>,
        (bc, most_side): (bool, u32),
        (frame, eye): (u64, Vec3),
    ) -> u64 {
        if self.asked.get(&key) == Some(&pixels) {
            self.asked.remove(&key);
        }
        let (mut used, mut at) = (frame, eye);
        if let Some(Texture::Ready(held)) = self.map.get(&key) {
            // A late answer to a smaller ask leaves the larger one in place.
            if held.got >= pixels {
                return 0;
            }
            (used, at) = (held.used, held.at);
        }
        let made = read.and_then(|texture| {
            let (view, bytes) = upload(device, queue, texture, bc, most_side)?;
            Some((view, bytes, texture.width.max(texture.height)))
        });
        let Some((view, bytes, longest)) = made else {
            if let Some(texture) = read {
                tracing::warn!(key, format = ?texture.format, "a world texture did not go on the device");
            }
            match self.map.get_mut(&key) {
                // A larger answer that did not go on the device leaves the smaller in place, for good.
                Some(Texture::Ready(held)) => held.top = true,
                _ => {
                    self.map.insert(key, Texture::Missing);
                }
            }
            return 0;
        };
        self.stamps += 1;
        let ready = Ready {
            view,
            bytes,
            used,
            at,
            got: pixels,
            // An answer smaller than its ask is the largest there is.
            top: longest < pixels,
            stamp: MISSING + self.stamps,
        };
        if let Some(Texture::Ready(old)) = self.map.insert(key, Texture::Ready(ready)) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
        bytes
    }

    /// What a material remembers the texture of `key` by, and its view where it is on the device.
    pub(super) fn of(&self, key: u64) -> (u64, Option<&wgpu::TextureView>) {
        match self.map.get(&key) {
            Some(Texture::Ready(ready)) => (ready.stamp, Some(&ready.view)),
            Some(Texture::Missing) => (MISSING, None),
            None => (ABSENT, None),
        }
    }

    /// What a mesh whose colour map is not on the device is drawn with: a mid grey, never the white of no map.
    pub(super) fn grey(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        let grey = self.grey.get_or_insert_with(|| {
            let format = wgpu::TextureFormat::Rgba8UnormSrgb;
            write(device, queue, format, (1, 1), &[vec![128, 128, 128, 255]]).0
        });
        grey.clone()
    }

    /// Let the textures no material sampled in the frame `frame` go, the farthest from `eye`
    /// first, until the rest fit `budget`; the keys of those that went.
    pub(super) fn evict(&mut self, budget: u64, frame: u64, eye: Vec3) -> Vec<u64> {
        if self.bytes <= budget {
            return Vec::new();
        }
        let idle = (self.map.iter())
            .filter_map(|(key, texture)| match texture {
                Texture::Ready(ready) if ready.used < frame => Some(Idle {
                    key: *key,
                    bytes: ready.bytes,
                    held: false,
                    away: eye.distance(ready.at),
                }),
                _ => None,
            })
            .collect();
        let gone = evicted(idle, self.bytes - budget);
        for idle in &gone {
            self.map.remove(&idle.key);
            self.bytes -= idle.bytes;
        }
        gone.into_iter().map(|idle| idle.key).collect()
    }

    /// How many are on the device, and how many no pack holds or did not read.
    pub(super) fn counts(&self) -> (usize, usize) {
        let ready = |t: &&Texture| matches!(t, Texture::Ready(_));
        let on_device = self.map.values().filter(ready).count();
        (on_device, self.map.len() - on_device)
    }
}

#[cfg(test)]
mod tests {
    use wgpu::TextureFormat as F;

    use super::*;

    #[test]
    fn a_palette_is_laid_in_no_side_longer_than_the_device_makes() {
        assert_eq!(palette_size(0, 8192), (1, 1));
        assert_eq!(palette_size(300, 8192), (300, 1));
        assert_eq!(palette_size(PALETTE_ROW * 3 + 1, 8192), (PALETTE_ROW, 4));
        // A device of 2,048 a side: rows of 2,048, and no more rows than that.
        assert_eq!(palette_size(5000, 2048), (2048, 3));
        assert_eq!(palette_size(2048 * 2048 + 9, 2048), (2048, 2048));
        assert_eq!(palette_size(PALETTE_ROW * 9000, 8192), (PALETTE_ROW, 8192));
    }

    #[test]
    fn a_level_of_blocks_is_whole_blocks() {
        assert_eq!(level_bytes(F::Rgba8Unorm, (5, 3)), Some(60));
        assert_eq!(level_bytes(F::Bc1RgbaUnormSrgb, (8, 8)), Some(32));
        assert_eq!(level_bytes(F::Bc7RgbaUnorm, (8, 4)), Some(32));
        // A level smaller than a block is one block.
        assert_eq!(level_bytes(F::Bc3RgbaUnorm, (2, 1)), Some(16));
        assert_eq!(level_bytes(F::Bc5RgUnorm, (5, 5)), Some(64));
    }

    #[test]
    fn a_key_waited_for_is_asked_for_at_the_size_it_waits_at() {
        let mut textures = Textures::default();
        let mut wanted = [(5.0, 7, 128), (9.0, 8, 256)];
        textures.hold(&mut wanted);
        assert_eq!(wanted, [(5.0, 7, 128), (9.0, 8, 256)]);
        // The camera came nearer before either was answered: both keep their size.
        let mut wanted = [(2.0, 7, 512), (9.0, 8, 256)];
        textures.hold(&mut wanted);
        assert_eq!(wanted, [(2.0, 7, 128), (9.0, 8, 256)]);
        // A key no longer wanted is no longer waited for: asked for again, it takes its new size.
        textures.hold(&mut []);
        let mut wanted = [(2.0, 7, 512)];
        textures.hold(&mut wanted);
        assert_eq!(wanted, [(2.0, 7, 512)]);
    }

    #[test]
    fn only_levels_of_the_right_size_are_taken() {
        let texture = |levels: Vec<usize>| CpuTexture {
            width: 8,
            height: 8,
            format: F::Bc1RgbaUnorm,
            levels: levels.into_iter().map(|bytes| vec![0; bytes]).collect(),
        };
        assert_eq!(whole_levels(&texture(vec![32, 8, 8, 8])), 4);
        // A level cut short ends the chain, and so does one past the last a texture this size has.
        assert_eq!(whole_levels(&texture(vec![32, 8, 7, 8])), 2);
        assert_eq!(whole_levels(&texture(vec![32, 8, 8, 8, 8])), 4);
        assert_eq!(whole_levels(&texture(vec![31])), 0);
    }

    #[test]
    fn every_block_format_handed_over_has_a_decoder() {
        for format in [
            F::Bc1RgbaUnorm,
            F::Bc1RgbaUnormSrgb,
            F::Bc2RgbaUnorm,
            F::Bc3RgbaUnorm,
            F::Bc3RgbaUnormSrgb,
            F::Bc4RUnorm,
            F::Bc5RgUnorm,
            F::Bc7RgbaUnorm,
            F::Bc7RgbaUnormSrgb,
        ] {
            assert!(block_format(format).is_some(), "{format:?}");
        }
        assert!(block_format(F::Rgba8Unorm).is_none());
        assert!(block_format(F::Bc6hRgbUfloat).is_none());
    }

    fn quad(size: f32, turns: f32) -> (Vec<MeshVertex>, Vec<u32>) {
        let corner = |x: f32, y: f32| {
            MeshVertex::new(
                [x * size, y * size, 0.0],
                [0.0, 0.0, 1.0],
                [x * turns, y * turns],
            )
        };
        (
            vec![
                corner(0.0, 0.0),
                corner(1.0, 0.0),
                corner(1.0, 1.0),
                corner(0.0, 1.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    #[test]
    fn the_density_is_the_turns_of_the_texture_a_metre() {
        // Ten metres across, the texture once over it.
        let (vertices, indices) = quad(10.0, 1.0);
        assert!((density(&vertices, &indices) - 0.1).abs() < 1e-6);
        // The same wall with the texture five times over it.
        let (vertices, indices) = quad(10.0, 5.0);
        assert!((density(&vertices, &indices) - 0.5).abs() < 1e-6);
        // No surface, an index past the vertices, no triangle: nought.
        assert_eq!(density(&quad(0.0, 1.0).0, &indices), 0.0);
        assert_eq!(density(&vertices, &[0, 1, 9]), 0.0);
        assert_eq!(density(&vertices, &[]), 0.0);
    }

    #[test]
    fn a_texture_is_asked_for_at_the_size_its_surface_shows_it() {
        // A screen 1,000 pixels high seen through 90 degrees: 500 pixels a metre at a metre.
        let scale = per_metre(1000, std::f32::consts::FRAC_PI_2);
        assert!((scale - 500.0).abs() < 1e-2);
        // A texture once over ten metres, ten metres off: 500 pixels, so the next power of two.
        assert_eq!(pixels(0.1, 10.0, 1.0, scale, 8192), 512);
        // Twice as far needs half; twice the size of the instance needs double.
        assert_eq!(pixels(0.1, 20.0, 1.0, scale, 8192), 256);
        assert_eq!(pixels(0.1, 10.0, 2.0, scale, 8192), 1024);
        // Never under the least side, never over what the device makes.
        assert_eq!(pixels(0.1, 5000.0, 1.0, scale, 8192), LEAST_SIDE);
        assert_eq!(pixels(0.001, 0.0, 1.0, scale, 8192), 8192);
        assert_eq!(pixels(0.1, 10.0, 1.0, scale, 100), 100);
        // A texture that does not turn shows one texel.
        assert_eq!(pixels(0.0, 1.0, 1.0, scale, 8192), LEAST_SIDE);
    }
}
