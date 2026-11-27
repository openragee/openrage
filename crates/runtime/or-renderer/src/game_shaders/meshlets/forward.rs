//! The forward surfaces of a model: those whose pixel programs colour or light them themselves.
//! They are drawn after the targets are lit and before the frame is toned, into the lit frame,
//! tested against the depth the targets were drawn with.

use std::sync::Arc;

use glam::Vec3;

use super::shared::Made;
use super::{draw_of, Built, Draw, MeshletScene, MeshletSurface, Stale, DEPTH, VERTICES};
use crate::game_shaders::pipeline::Target;
use crate::game_shaders::targets::render_target;

/// How a surface drawn by a forward pass is laid over the lit frame, and where it stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forward {
    pub blend: wgpu::BlendState,
    pub writes: wgpu::ColorWrites,
    /// The middle of the surface in the world: the surfaces are drawn from the farthest.
    pub centre: Vec3,
}

/// The lit frame's format: light in its own units, which a half float holds and blends.
pub(super) const LIT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Why a scene of forward surfaces alone is not drawn.
pub(super) const ALONE: &str =
    "its forward surfaces are laid over a lit frame, and no surface of the model draws the targets it is lit from";

/// The colour target the pass of a forward surface is built with.
pub(super) fn target(forward: &Forward) -> wgpu::ColorTargetState {
    wgpu::ColorTargetState {
        format: LIT,
        blend: Some(forward.blend),
        write_mask: forward.writes,
    }
}

/// Whether a recording that draws `now` draws the forward surfaces again: whenever it lights the
/// frame again, which it does whenever it draws the targets again and whenever the light moves.
/// A frame that draws neither shows the lit frame kept from the last that did.
pub(super) fn redraws(now: Stale) -> bool {
    now.lit || now.targets
}

/// The places of `centres`, the farthest from `camera` first.
fn farthest_first(centres: &[Vec3], camera: Vec3) -> Vec<usize> {
    let mut order: Vec<usize> = (0..centres.len()).collect();
    let far = |at: &usize| centres[*at].distance_squared(camera);
    order.sort_by(|a, b| far(b).total_cmp(&far(a)));
    order
}

/// The lit frame, the program that lights the targets into it and shows it, and the program that
/// tones it onto the frame.
const LIT_FRAME: &str = r#"
// The frame lit in the light's own units, which the forward surfaces are laid over: the light,
// and one where the model or the ground laid under it is shown.
@group(2) @binding(0) var lit_frame: texture_2d<f32>;

// The most a texel of the lit frame holds: a half float holds no more.
const HELD: vec3<f32> = vec3<f32>(65000.0);

// What target `shown` holds at `texel`, raw.
fn raw_at(texel: vec2<i32>, shown: u32) -> vec3<f32> {
    let packed_normal = textureLoad(normal, texel, 0);
    let albedo = textureLoad(colour, texel, 0);
    let terms = textureLoad(specular, texel, 0);
    if shown == 1u { return albedo.rgb; }
    if shown == 2u { return packed_normal.rgb; }
    if shown == 3u { return normal_of(packed_normal) * 0.5 + 0.5; }
    if shown == 4u { return terms.rgb; }
    if shown == 5u { return textureLoad(ambient, texel, 0).rgb; }
    return vec3<f32>(albedo.a, packed_normal.a, terms.a);
}

// The light `lit` would show, before its exposure and tone.
@fragment fn lit_light(corner: Corner) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(corner.at.xy);
    let far = textureLoad(depth, texel, 0);
    let shown = u32(light.camera.w);
    if far <= 0.0 {
        let ground = ground_at(corner.clip);
        if light.laid.w <= 0.0 || !ground.met || shown != 0u {
            discard;
        }
        return vec4<f32>(min(shade(ground.surface, vec2<f32>(texel)), HELD), 1.0);
    }
    if shown != 0u {
        return vec4<f32>(raw_at(texel, shown), 1.0);
    }
    return vec4<f32>(min(lit_at(texel, corner.clip, far), HELD), 1.0);
}

// The lit frame, the forward surfaces laid over it, exposed, toned and encoded, with the depth
// `lit` writes.
@fragment fn shown_frame(corner: Corner) -> Lit {
    let texel = vec2<i32>(corner.at.xy);
    let held = textureLoad(lit_frame, texel, 0);
    if held.a <= 0.0 {
        discard;
    }
    var far = textureLoad(depth, texel, 0);
    if far <= 0.0 {
        far = ground_at(corner.clip).far;
    }
    if u32(light.camera.w) != 0u {
        return Lit(vec4<f32>(held.rgb, 1.0), far);
    }
    return Lit(vec4<f32>(shown_of(held.rgb), 1.0), far);
}
"#;

/// The forward surfaces of a scene and what they are drawn into.
// ponytail: a forward surface over no surface of the model and no ground is not shown, since the
// frame behind it is the viewport's and is never lit; and a map of one that comes in late is not
// bound again. Lay them over the viewport's own frame, and rebind them as the targets' are, when
// either shows.
pub(super) struct Over {
    /// Each with the middle of its surface.
    draws: Vec<(Draw, Vec3)>,
    lit: wgpu::TextureView,
    lit_layout: wgpu::BindGroupLayout,
    lit_group: wgpu::BindGroup,
    /// Lights the targets into the lit frame.
    lights: wgpu::RenderPipeline,
    /// Tones the lit frame onto the frame.
    shows: wgpu::RenderPipeline,
    /// Where the camera stood at the last update.
    camera: std::cell::Cell<Vec3>,
}

fn lit_group_of(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    lit: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("meshlet-lit"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(lit),
        }],
    })
}

fn lit_of(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
    render_target(
        device,
        "meshlet-lit",
        size,
        LIT,
        wgpu::TextureUsages::COPY_SRC,
    )
}

/// The layouts the passes of a scene read: the resolve's, and the metered stops'.
pub(super) struct Layouts<'l> {
    pub resolve: &'l wgpu::BindGroupLayout,
    pub metered: &'l wgpu::BindGroupLayout,
}

impl Over {
    /// What draws `surfaces`, each the one at its place by its pass, over a lit frame of `size`
    /// toned onto a frame of `frame`'s formats.
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layouts: Layouts<'_>,
        frame: &Target,
        size: (u32, u32),
        surfaces: &[(usize, &MeshletSurface, Arc<Built>)],
        made: &mut Made,
    ) -> Result<Self, String> {
        let source = format!("{}{LIT_FRAME}", super::resolve::RESOLVE);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet-lit"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let lit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet-lit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline = |label: &str,
                        groups: &[Option<&wgpu::BindGroupLayout>],
                        entry: &str,
                        colour: wgpu::ColorTargetState,
                        depth: wgpu::DepthStencilState| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("at_corner"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(colour)],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // Drawn in the pass of the forward surfaces, whose depth target is read and not written.
        let lights = pipeline(
            "meshlet-lit-light",
            &[Some(layouts.resolve)],
            "lit_light",
            LIT.into(),
            wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        let shows = pipeline(
            "meshlet-lit-shown",
            &[
                Some(layouts.resolve),
                Some(layouts.metered),
                Some(&lit_layout),
            ],
            "shown_frame",
            wgpu::ColorTargetState {
                format: frame.colour,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            wgpu::DepthStencilState {
                format: frame.depth,
                depth_write_enabled: Some(true),
                depth_compare: Some(frame.depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        let lit = lit_of(device, size);
        let lit_group = lit_group_of(device, &lit_layout, &lit);
        let mut over = Self {
            draws: Vec::new(),
            lit,
            lit_layout,
            lit_group,
            lights,
            shows,
            camera: Default::default(),
        };
        for (at, surface, built) in surfaces {
            over.take_on(device, queue, *at, surface, built, made)?;
        }
        Ok(over)
    }

    /// Draw as well `surface`, the one at `at`, by its pass `built`.
    pub(super) fn take_on(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        at: usize,
        surface: &MeshletSurface,
        built: &Arc<Built>,
        made: &mut Made,
    ) -> Result<(), String> {
        let centre = surface.forward.map_or(Vec3::ZERO, |forward| forward.centre);
        let draw = draw_of(device, queue, at, surface, built, made)?;
        self.draws.push((draw, centre));
        Ok(())
    }

    /// How many surfaces it draws.
    pub(super) fn drawn(&self) -> usize {
        self.draws.len()
    }

    /// The buffers of its surfaces each frame writes the view into.
    pub(super) fn views(&self) -> impl Iterator<Item = &super::ViewWrite> + '_ {
        self.draws.iter().flat_map(|(draw, _)| &draw.views)
    }

    /// Take in where the camera stands this frame.
    pub(super) fn update(&self, camera: Vec3) {
        self.camera.set(camera);
    }

    /// Make the lit frame anew for a frame of `size`.
    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.lit = lit_of(device, size);
        self.lit_group = lit_group_of(device, &self.lit_layout, &self.lit);
    }

    /// Light the targets of `scene` into the lit frame and lay the forward surfaces over it, in a
    /// pass of their own.
    pub(super) fn record(&self, scene: &MeshletScene, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet-forward"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.lit,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            // Read and not written: the lighting reads the same depth as a texture.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &scene.depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.lights);
        pass.set_bind_group(0, &scene.resolve_group, &[]);
        pass.draw(0..3, 0..1);
        let (mut binds, mut vertices) = (1, 0);
        let centres: Vec<Vec3> = self.draws.iter().map(|(_, centre)| *centre).collect();
        for at in farthest_first(&centres, self.camera.get()) {
            let (draw, _) = &self.draws[at];
            pass.set_pipeline(&draw.pipeline);
            for (group, bound) in &draw.groups {
                pass.set_bind_group(*group, bound, &[]);
                binds += 1;
            }
            let fitted = match scene.at_most.get() {
                true => VERTICES,
                false => draw.vertices,
            };
            let (corners, meshlets) = (0..fitted, 0..draw.meshlets);
            vertices += u64::from(corners.end) * u64::from(meshlets.end);
            pass.draw(corners, meshlets);
        }
        drop(pass);
        let drawn = self.draws.len() as u32;
        scene.count(|calls| {
            calls.passes += 1;
            calls.draws += 1 + drawn;
            calls.surface_draws += drawn;
            calls.binds += binds;
            calls.vertices += vertices;
        });
    }

    /// Tone the lit frame onto the frame `pass` draws, with the depth the resolve writes.
    pub(super) fn show(&self, scene: &MeshletScene, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.shows);
        pass.set_bind_group(0, &scene.resolve_group, &[]);
        pass.set_bind_group(1, &scene.metered_group, &[]);
        pass.set_bind_group(2, &self.lit_group, &[]);
        pass.draw(0..3, 0..1);
        scene.count_full_screen(0, 3);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Kept, LIGHT_BYTES};
    use super::*;

    #[test]
    fn the_forward_surfaces_are_drawn_again_exactly_when_the_frame_is_lit_again() {
        let view = |at: u8| [at; 64];
        let (light, other) = ([0u8; LIGHT_BYTES], [1u8; LIGHT_BYTES]);
        let mut kept = Kept::default();
        let mut drawn = |view: [u8; 64], light: &[u8; LIGHT_BYTES]| {
            kept.take(view, None, light);
            let now = kept.draws();
            (now.targets, redraws(now))
        };
        assert_eq!(drawn(view(1), &light), (true, true), "the first frame");
        assert_eq!(drawn(view(1), &light), (false, false), "at rest");
        assert_eq!(drawn(view(2), &light), (true, true), "the camera moved");
        // The targets stand, but the frame they are lit into is lit again, and so is drawn over again.
        assert_eq!(drawn(view(2), &other), (false, true), "the light moved");
        assert_eq!(drawn(view(2), &other), (false, false), "at rest again");
        // Each of the three stale marks alone.
        let stale = |shadow, targets, lit| Stale {
            shadow,
            targets,
            lit,
        };
        assert!(!redraws(stale(true, false, false)), "a tile cast alone");
        assert!(redraws(stale(false, true, false)));
        assert!(redraws(stale(false, false, true)));
    }

    #[test]
    fn the_forward_surfaces_are_drawn_from_the_farthest() {
        let centres = [
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::ZERO,
        ];
        assert_eq!(
            farthest_first(&centres, Vec3::new(0.0, 0.0, 10.0)),
            [2, 0, 1]
        );
        assert_eq!(
            farthest_first(&centres, Vec3::new(0.0, 0.0, -10.0)),
            [1, 0, 2]
        );
        assert!(farthest_first(&[], Vec3::ZERO).is_empty());
    }

    /// A stage that lays one premultiplied colour, its constant, over what is behind it.
    const OVER: &str = "
@group(1) @binding(0) var<uniform> premultiplied: vec4<f32>;

@fragment fn main(@location(0) shade: vec3<f32>) -> @location(0) vec4<f32> {
    return premultiplied;
}";

    fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
        let mut backends = wgpu::InstanceDescriptor::new_without_display_handle();
        backends.backends = wgpu::Backends::from_env().unwrap_or(backends.backends);
        let instance = wgpu::Instance::new(backends);
        let adapter = match crate::instance::headless_adapter(&instance) {
            Ok(adapter) => adapter,
            Err(e) => {
                or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
                return None;
            }
        };
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("forward-test"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()
    }

    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "needs a GPU adapter: acquisition can hang, not just fail"
    )]
    #[test]
    fn a_blended_quad_over_an_opaque_surface_lays_its_colour_on_by_its_blend() {
        use super::super::tests::{surface, MESH, SIZE};
        use super::super::{Feed, Lighting, MeshletPipelines, Scene, Sky};
        let Some((device, queue)) = gpu("a_blended_quad_over_an_opaque_surface") else {
            return;
        };
        let frame = Target {
            colour: wgpu::TextureFormat::Rgba8Unorm,
            depth: DEPTH,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            blend: None,
            colour_attachments: Some(1),
        };
        let stage =
            |text: &str| Arc::new(naga::front::wgsl::parse_str(text).expect("the WGSL parses"));
        let (vertex, fragment) = (stage(MESH), stage(OVER));
        // A square over the middle of the frame, nearer than the opaque surface's 0.5.
        let corners: [[f32; 3]; 6] = [
            [-0.4, -0.4, 0.75],
            [0.4, -0.4, 0.75],
            [0.4, 0.4, 0.75],
            [-0.4, -0.4, 0.75],
            [0.4, 0.4, 0.75],
            [-0.4, 0.4, 0.75],
        ];
        let words: Vec<u32> = corners.iter().flatten().map(|v| v.to_bits()).collect();
        let quad = |premultiplied: [f32; 4]| MeshletSurface {
            vertex: Arc::clone(&vertex),
            fragment: Arc::clone(&fragment),
            feeds: std::collections::BTreeMap::from([
                ((0, 0), Feed::Bytes(bytemuck::cast_slice(&words).to_vec())),
                (
                    (0, 1),
                    Feed::Bytes(bytemuck::cast_slice(&[0u32, 1, 2, 3, 4, 5]).to_vec()),
                ),
                (
                    (0, 2),
                    Feed::View {
                        bytes: vec![0; 64],
                        view_proj: Some(0),
                    },
                ),
                (
                    (1, 0),
                    Feed::Bytes(bytemuck::cast_slice(&premultiplied).to_vec()),
                ),
            ]),
            meshlets: 2,
            primitives: 1,
            forward: Some(Forward {
                blend: wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                },
                writes: wgpu::ColorWrites::COLOR,
                centre: Vec3::new(0.0, 0.0, 0.75),
            }),
        };
        let lighting = Lighting {
            view_proj: glam::Mat4::IDENTITY,
            sky: Sky {
                towards: Vec3::Z,
                sun: Vec3::splat(std::f32::consts::PI),
                sky: Vec3::ZERO,
                ground: Vec3::ZERO,
                exposure: 1.0,
                tone: None,
                scene: Scene::default(),
            },
            camera: Vec3::Z,
        };
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut pipelines = MeshletPipelines::default();
        // The lit frame at the middle, where the square lies over the opaque surface.
        let mut middle = |premultiplied: [f32; 4]| -> [f32; 4] {
            let surfaces = [surface(), quad(premultiplied)];
            let scene = loop {
                match MeshletScene::prepare(
                    &device,
                    &queue,
                    &mut pipelines,
                    &surfaces,
                    (SIZE, SIZE),
                    &frame,
                ) {
                    Ok(Some(scene)) if scene.waiting.is_empty() => break scene,
                    Ok(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    Err(why) => panic!("the scene does not prepare: {why}"),
                }
            };
            assert_eq!((scene.drawn(), scene.drawn_forward()), (2, 1));
            scene.update(&queue, &lighting);
            let mut encoder = device.create_command_encoder(&Default::default());
            scene.record_targets(&mut encoder, |_, _| {});
            queue.submit([encoder.finish()]);
            assert_eq!(
                scene.calls().surface_draws,
                2,
                "the opaque surface and the square"
            );
            let over = scene.over.as_ref().expect("a forward surface");
            let lit = crate::game_shaders::frame::read_back(
                &device,
                &queue,
                over.lit.texture(),
                LIT,
                SIZE,
            );
            let at = ((SIZE / 2 * SIZE + SIZE / 2) * 8) as usize;
            let half = |i: usize| {
                or_io::half::half_to_f32(u16::from_le_bytes([
                    lit.bytes[at + 2 * i],
                    lit.bytes[at + 2 * i + 1],
                ]))
            };
            [half(0), half(1), half(2), half(3)]
        };
        let behind = middle([0.0; 4]);
        let laid = middle([0.25, 0.5, 0.125, 0.5]);
        assert_eq!(
            pollster::block_on(validation.pop()).map(|e| e.to_string()),
            None
        );
        // The opaque surface is lit and shown; laid over it with nought, it is as it was.
        assert!(behind[0] > 0.1 && behind[3] == 1.0, "{behind:?}");
        // Source times one and the destination times one less the source's alpha; the alpha kept.
        let expected = [
            0.25 + 0.5 * behind[0],
            0.5 + 0.5 * behind[1],
            0.125 + 0.5 * behind[2],
            behind[3],
        ];
        for (got, wanted) in laid.iter().zip(expected) {
            assert!(
                (got - wanted).abs() <= 0.01 * wanted.abs().max(1.0),
                "{laid:?} against {expected:?}"
            );
        }
    }
}
