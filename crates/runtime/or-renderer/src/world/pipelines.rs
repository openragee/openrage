//! The pipelines the world is drawn with: one for each bucket in each kind of pass it is drawn in.

use crate::game_shaders::deferred::{
    DEPTH_STENCIL_FORMAT, GBUFFER_FORMAT, GBUFFER_TARGETS, HDR_FORMAT,
};
use crate::game_shaders::mesh::scene::writes_material;
use crate::game_shaders::shadow::MAP_FORMAT;
use crate::viewport::MeshVertex;

use super::shader::WORLD_SHADER;
use super::{WorldBucket, WorldInstance};

/// A [`MeshVertex`] as the world reads it, as [`super::coloured`] packs it: the colour's four floats as the second texture coordinate and two words, and the depth bias's four bytes as the second colour.
pub(super) const WORLD_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
    3 => Float32x2,
    9 => Uint32x2,
    4 => Uint32,
];

/// The pipeline of each bucket a pass draws, by the bucket's place in the order they draw; none where the pass does not draw it.
pub(super) type ByBucket = [Option<wgpu::RenderPipeline>; WorldBucket::COUNT];

pub(super) struct Pipelines {
    /// Into the viewport's own colour and depth, lit as the plain modes light a surface.
    pub(super) forward: ByBucket,
    /// Into the G-buffer of the game's frame.
    pub(super) gbuffer: ByBucket,
    /// Over the frame the game's passes lit, whose depth is only read: the back faces, then the front faces.
    pub(super) lit: [ByBucket; 2],
    /// A material's own light, added over the lit frame where its surface lies.
    pub(super) glow: ByBucket,
    /// Into a tile of the map the sun's shadow is cast into.
    pub(super) cast: ByBucket,
}

/// How far a decal is held in front of the surface it lies on: in the least steps of the depth target, and by how steeply it meets the view. The nearer a surface the greater its depth, so it is held nearer by adding.
const DECAL_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 16,
    slope_scale: 1.0,
    clamp: 0.0,
};

/// How far a surface that casts a shadow is held back by how steeply it meets the light, so that it does not shade itself: what the game's own casters are held back by.
const CAST_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 4,
    slope_scale: 2.5,
    clamp: 0.0,
};

/// Blended over what is there by its alpha, keeping the alpha that was there: the G-buffer holds a word in it, and so does the lit frame.
const OVER_KEEPING_ALPHA: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Laid over what is there as a colour already multiplied by its alpha, keeping the alpha that was there: what blends over the lit frame states its coverage so, and a forward emissive adds its light with little coverage.
const PREMULTIPLIED_KEEPING_ALPHA: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: OVER_KEEPING_ALPHA.alpha,
};

/// Added to what is there, keeping the alpha that was there.
const ADDED_KEEPING_ALPHA: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: OVER_KEEPING_ALPHA.alpha,
};

impl Pipelines {
    /// The pipelines for a viewport whose colour target is of `colour_format`, with the layouts of its camera, of a material's maps and of the frame's light.
    pub(super) fn new(
        device: &wgpu::Device,
        camera: &wgpu::BindGroupLayout,
        materials: &wgpu::BindGroupLayout,
        light: &wgpu::BindGroupLayout,
        colour_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-world-shader"),
            source: wgpu::ShaderSource::Wgsl(WORLD_SHADER.into()),
        });
        let viewed = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-world-pipeline-layout"),
            bind_group_layouts: &[Some(camera), Some(materials), Some(light)],
            immediate_size: 0,
        });
        // A cascade is seen through a uniform laid out as a camera's, and is lit by nothing
        let cascade = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-world-cast-pipeline-layout"),
            bind_group_layouts: &[Some(camera), Some(materials)],
            immediate_size: 0,
        });
        let buffers = [
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &WORLD_VERTEX_ATTRIBUTES,
            },
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<WorldInstance>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    5 => Float32x4,
                    6 => Float32x4,
                    7 => Float32x4,
                    8 => Float32x4,
                    10 => Float32x2,
                    11 => Float32,
                ],
            },
        ];
        let culled = |label: &str,
                      cull_mode: Option<wgpu::Face>,
                      layout: &wgpu::PipelineLayout,
                      (vertex, fragment): (&str, &str),
                      targets: &[Option<wgpu::ColorTargetState>],
                      depth_stencil: wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                depth_stencil: Some(depth_stencil),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let build = |label: &str,
                     layout: &wgpu::PipelineLayout,
                     stages: (&str, &str),
                     targets: &[Option<wgpu::ColorTargetState>],
                     depth_stencil: wgpu::DepthStencilState| {
            culled(label, None, layout, stages, targets, depth_stencil)
        };
        // The depth of the viewport and of the game's frame, where the nearer a surface the greater its depth
        let frame = |write: bool, stencil: wgpu::StencilState, bias: wgpu::DepthBiasState| {
            wgpu::DepthStencilState {
                format: DEPTH_STENCIL_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil,
                bias,
            }
        };
        let (untouched, level) = (wgpu::StencilState::default, wgpu::DepthBiasState::default);
        let target = |format, blend, write_mask| {
            Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask,
            })
        };
        let all = wgpu::ColorWrites::ALL;

        let screen = [target(colour_format, None, all)];
        let over_screen = [target(
            colour_format,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            all,
        )];
        let solid = |label: &str, fragment: &str| {
            let depth = frame(true, untouched(), level());
            Some(build(label, &viewed, ("vs_main", fragment), &screen, depth))
        };
        let over = |label: &str, bias: wgpu::DepthBiasState| {
            let depth = frame(false, untouched(), bias);
            let stages = ("vs_main", "fs_forward_blend");
            Some(build(label, &viewed, stages, &over_screen, depth))
        };
        let forward = [
            solid("viewport-world-forward-pipeline", "fs_forward"),
            solid(
                "viewport-world-forward-cutout-pipeline",
                "fs_forward_cutout",
            ),
            solid(
                "viewport-world-forward-emissive-pipeline",
                "fs_forward_emissive",
            ),
            over("viewport-world-forward-decal-pipeline", DECAL_BIAS),
            over("viewport-world-forward-blend-pipeline", level()),
        ];

        let words = vec![target(GBUFFER_FORMAT, None, all); GBUFFER_TARGETS];
        // A decal is blended by its coverage into the colour, the normal and the specular words, keeps the alpha each of them held, and leaves the ambient words be
        let mut decal_words = vec![
            target(
                GBUFFER_FORMAT,
                Some(OVER_KEEPING_ALPHA),
                wgpu::ColorWrites::COLOR
            );
            GBUFFER_TARGETS
        ];
        if let Some(ambient) = decal_words.last_mut() {
            *ambient = target(GBUFFER_FORMAT, None, wgpu::ColorWrites::empty());
        }
        let written = |label: &str, fragment: &str| {
            let depth = frame(true, writes_material(), level());
            Some(build(label, &viewed, ("vs_main", fragment), &words, depth))
        };
        let gbuffer = [
            written("viewport-world-gbuffer-pipeline", "fs_gbuffer"),
            written(
                "viewport-world-gbuffer-cutout-pipeline",
                "fs_gbuffer_cutout",
            ),
            written(
                "viewport-world-gbuffer-emissive-pipeline",
                "fs_gbuffer_emissive",
            ),
            // It lies on a surface that marked the pixel already, and writes no depth
            Some(build(
                "viewport-world-gbuffer-decal-pipeline",
                &viewed,
                ("vs_main", "fs_gbuffer_decal"),
                &decal_words,
                frame(false, untouched(), DECAL_BIAS),
            )),
            None,
        ];

        // A glass draws the faces that turn away first and then those that face the viewer, so its far side shows through its near one
        let lit_faces = |label: &str, dropped: wgpu::Face| {
            [
                None,
                None,
                None,
                None,
                Some(culled(
                    label,
                    Some(dropped),
                    &viewed,
                    ("vs_main", "fs_lit"),
                    &[target(HDR_FORMAT, Some(PREMULTIPLIED_KEEPING_ALPHA), all)],
                    frame(false, untouched(), level()),
                )),
            ]
        };
        let lit = [
            lit_faces("viewport-world-lit-back-pipeline", wgpu::Face::Front),
            lit_faces("viewport-world-lit-front-pipeline", wgpu::Face::Back),
        ];

        // On the surface the G-buffer pass laid, which passes the depth test it wrote
        let glowing = |label: &str, fragment: &str| {
            let depth = frame(false, untouched(), level());
            let added = [target(HDR_FORMAT, Some(ADDED_KEEPING_ALPHA), all)];
            Some(build(label, &viewed, ("vs_main", fragment), &added, depth))
        };
        let glow = [
            glowing("viewport-world-glow-pipeline", "fs_glow"),
            glowing("viewport-world-glow-cutout-pipeline", "fs_glow_cutout"),
            glowing("viewport-world-glow-emissive-pipeline", "fs_glow"),
            None,
            None,
        ];

        // A tile of the map: the nearer the sun a surface the less its depth, and no colour is written
        let tile = || wgpu::DepthStencilState {
            format: MAP_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: untouched(),
            bias: CAST_BIAS,
        };
        let whole = build(
            "viewport-world-cast-pipeline",
            &cascade,
            ("vs_cast", "fs_cast"),
            &[],
            tile(),
        );
        let cut_out = build(
            "viewport-world-cast-cutout-pipeline",
            &cascade,
            ("vs_cast", "fs_cast_cutout"),
            &[],
            tile(),
        );
        // What blends lets the sun through, and a decal lies on what casts already
        let cast = [Some(whole.clone()), Some(cut_out), Some(whole), None, None];

        Self {
            forward,
            gbuffer,
            lit,
            glow,
            cast,
        }
    }
}
