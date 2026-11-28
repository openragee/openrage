use or_io::codec::texture::{decode_level, ChannelView, GrcTextureFormat};

use crate::render::gpu_budget::TextureStamp;
use crate::state::texture_resolution;
use crate::state::textures::Texture;
use crate::state::{AppState, MeshSource};

/// The texture bound as `name` for thumbnail `entry`, else for the active selection, with its stamp.
///
/// A mesh of the Animation preview names its source in `from`, whose own embedded textures come first.
pub(crate) fn bound_texture<'a>(
    state: &'a AppState,
    from: Option<MeshSource>,
    entry: Option<u64>,
    name: &str,
) -> Option<(Texture<'a>, TextureStamp)> {
    let selection = entry.or_else(|| state.selection.selected(state.active_document()?.id));
    let texture = find_texture_for_selection(state, from, selection, name)?;
    let stamp = TextureStamp::of(&texture);
    Some((texture, stamp))
}

fn find_texture_for_selection<'a>(
    state: &'a AppState,
    from: Option<MeshSource>,
    selection: Option<u64>,
    name: &str,
) -> Option<Texture<'a>> {
    let found = texture_resolution::resolve(state, from, selection, name)?;
    if found.also_in.is_empty() {
        tracing::trace!(texture = name, source = %found.origin.describe(), "texture resolved");
    } else {
        tracing::debug!(
            texture = name,
            source = %found.origin.describe(),
            others = found.also_in.len(),
            "texture name is held by more than one open dictionary"
        );
    }
    Some(found.texture)
}

pub(super) fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    texture: &Texture<'_>,
    bump: bool,
) -> Option<(wgpu::BindGroup, usize)> {
    let map = prepare(texture, bump)?;
    Some(upload_rgba(
        device, queue, layout, map.size, &map.rgba, map.srgb,
    ))
}

/// The first level of `texture` as the viewport uploads it: decoded, shown as a preview shows its
/// channels, and linear where it is read `bump`.
pub(super) fn prepare(texture: &Texture<'_>, bump: bool) -> Option<super::material_maps::Prepared> {
    if !texture.has_pixel_data() || texture.width() == 0 || texture.height() == 0 {
        return None;
    }
    let rgba = match decode_level(texture, 0) {
        Ok(image) => image,
        Err(e) => {
            tracing::warn!(texture = texture.name(), error = %e, "viewport texture decode failed");
            return None;
        }
    };
    let mut rgba = rgba.with_channel_view(ChannelView::preview_default(texture.format()));
    opaque_where_wholly_clear(&mut rgba.data);
    // ⚠ A normal map holds vectors: read as sRGB, its level half tilts every normal the same way.
    let srgb = !bump
        && !matches!(
            texture.format(),
            GrcTextureFormat::A8 | GrcTextureFormat::L8
        );
    Some(super::material_maps::Prepared {
        size: (rgba.width, rgba.height),
        rgba: rgba.data,
        srgb,
    })
}

/// Make `rgba` opaque where every texel of it is fully clear: such a map hides the whole surface
/// it colours, so its fourth channel is not an opacity. A GTA VI base-colour map often holds
/// nought there, and its surface drew as nothing.
// ponytail: a map whose fourth channel is data that is not nought everywhere still cuts its
// surface out; tell an alpha effect by its technique when the game's choice of it is read.
fn opaque_where_wholly_clear(rgba: &mut [u8]) {
    if rgba.as_chunks::<4>().0.iter().all(|texel| texel[3] == 0) {
        (rgba.as_chunks_mut::<4>().0.iter_mut()).for_each(|texel| texel[3] = 255);
    }
}

/// The pixels as the bind group a mesh's material samples, and the bytes it holds.
pub(super) fn upload_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    (width, height): (u32, u32),
    data: &[u8],
    srgb: bool,
) -> (wgpu::BindGroup, usize) {
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let wgpu_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-mesh-diffuse"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &wgpu_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = wgpu_tex.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("viewport-mesh-diffuse-sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let bytes = data.len();
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport-mesh-diffuse-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (bind_group, bytes)
}

#[cfg(test)]
mod tests {
    use super::opaque_where_wholly_clear;

    #[test]
    fn a_map_that_would_hide_its_whole_surface_is_drawn_opaque() {
        let mut clear = vec![130, 63, 40, 0, 1, 2, 3, 0];
        opaque_where_wholly_clear(&mut clear);
        assert_eq!(clear, [130, 63, 40, 255, 1, 2, 3, 255]);
        // One texel that shows keeps the map a cutout.
        let mut cutout = vec![130, 63, 40, 0, 1, 2, 3, 9];
        opaque_where_wholly_clear(&mut cutout);
        assert_eq!(cutout, [130, 63, 40, 0, 1, 2, 3, 9]);
    }
}
