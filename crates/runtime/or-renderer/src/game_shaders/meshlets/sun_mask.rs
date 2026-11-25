//! The screen's mask of the directional light, as GTA VI's lighting program takes the sun's
//! shadow of one: a channel that holds, for each pixel, the root of the share of that light that
//! reaches what the pixel shows, and a word a tile that asks for a pixel's share to be rebuilt
//! from the pixels about it. The frame is lit through the mask and never by what fills it: the
//! shadow map is its first writer.

use super::super::targets::render_target;

/// The format of the mask's one channel.
// The game's target holds eight bits. The editor's holds a float, so that a frame lit through
// the mask is the frame the shadow map gave.
const ROOTS: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// The format of a tile's word.
const WORDS: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// The pixels a side of a tile.
pub(super) const TILE: u32 = 16;
/// The bit of a tile's word that asks for the share of the tile's pixels to be rebuilt: the
/// program that lights the frame holds it, and no writer of the editor's sets it so far.
#[cfg(test)]
pub(super) const REBUILD: u32 = 1 << 14;

/// The mask of a frame, and what its first writer reads to fill it.
pub(super) struct SunMask {
    /// The root of the share of the light a pixel takes, from nought to one.
    pub(super) roots: wgpu::TextureView,
    /// A word a tile: no writer of the editor's sets a bit of it so far.
    pub(super) tiles: wgpu::TextureView,
    group: wgpu::BindGroup,
    /// Whether the mask is left as a test stated it, the shadow map writing nothing.
    #[cfg(test)]
    pub(super) stated: bool,
}

/// What the first writer reads: the normals and the depth the targets hold, the frame's light
/// and the tile the shadow is cast into.
pub(super) fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("meshlet-mask"),
        entries: &[
            texture(1, wgpu::TextureSampleType::Float { filterable: false }),
            texture(2, wgpu::TextureSampleType::Depth),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture(6, wgpu::TextureSampleType::Depth),
        ],
    })
}

/// The pass that fills the mask from the shadow's tile: `masked` of the program that lights the
/// frame, so that the place a pixel shows is found as the lighting finds it.
pub(super) fn writer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("meshlet-mask"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("meshlet-mask"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("at_corner"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("masked"),
            compilation_options: Default::default(),
            targets: &[Some(ROOTS.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl SunMask {
    /// The mask of a frame of `size`, which the first writer fills from what `targets` hold
    /// under `light` and the tile `shadow_map`. Its words are nought until a writer sets one.
    pub(super) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        size: (u32, u32),
        (targets, depth): (&[Option<wgpu::TextureView>], &wgpu::TextureView),
        light: &wgpu::Buffer,
        shadow_map: &wgpu::TextureView,
    ) -> Result<Self, String> {
        let Some(Some(normal)) = targets.get(1) else {
            return Err("the pixel programs write no normal the frame is lit from".into());
        };
        // A writer that is no pass of the editor's fills them by a copy.
        let filled = wgpu::TextureUsages::COPY_DST;
        let roots = render_target(device, "meshlet-mask", size, ROOTS, filled);
        let tiled = (size.0.div_ceil(TILE), size.1.div_ceil(TILE));
        let tiles = render_target(device, "meshlet-mask-tiles", tiled, WORDS, filled);
        let view = wgpu::BindingResource::TextureView;
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet-mask"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: view(normal),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: view(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: light.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: view(shadow_map),
                },
            ],
        });
        Ok(Self {
            roots,
            tiles,
            group,
            #[cfg(test)]
            stated: false,
        })
    }

    /// Fill the mask by `writer`, once the targets and the shadow's tile hold the frame.
    pub(super) fn record(&self, encoder: &mut wgpu::CommandEncoder, writer: &wgpu::RenderPipeline) {
        #[cfg(test)]
        if self.stated {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet-mask"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.roots,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(writer);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_that_lights_the_frame_reads_the_tiles_as_the_mask_lays_them() {
        let program = super::super::RESOLVE;
        assert!(program.contains(&format!("const TILE: i32 = {TILE};")));
        assert!(program.contains(&format!("const REBUILD: u32 = {REBUILD:#x}u;")));
    }
}
