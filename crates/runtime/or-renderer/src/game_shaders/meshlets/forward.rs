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
    /// The faces it does not draw, of those whose corners run counter-clockwise on the screen.
    pub cull: Option<wgpu::Face>,
    /// The middle of the surface in the world: the surfaces are drawn from the farthest.
    pub centre: Vec3,
}

impl Forward {
    /// What a pass of it is built by: two surfaces of one pair of stages laid on alike share one.
    pub(super) fn state(&self) -> (wgpu::BlendState, wgpu::ColorWrites, Option<wgpu::Face>) {
        (self.blend, self.writes, self.cull)
    }

    /// Whether its blend takes the second colour its pixel stage exports.
    pub(super) fn dual(&self) -> bool {
        let [colour, alpha] = [self.blend.color, self.blend.alpha];
        [
            colour.src_factor,
            colour.dst_factor,
            alpha.src_factor,
            alpha.dst_factor,
        ]
        .iter()
        .any(wgpu::BlendFactor::ref_second_blend_source)
    }
}

/// Give the two colours `fragment` exports at locations 0 and 1 to the one target as the two
/// sources a blend takes; how many outputs were moved.
pub(super) fn dual_sourced(fragment: &mut naga::Module) -> usize {
    let results: Vec<_> = (fragment.entry_points.iter())
        .filter(|entry| entry.stage == naga::ShaderStage::Fragment)
        .filter_map(|entry| entry.function.result.as_ref().map(|result| result.ty))
        .collect();
    let mut moved = 0;
    for handle in results {
        let mut ty = fragment.types[handle].clone();
        let naga::TypeInner::Struct { members, .. } = &mut ty.inner else {
            continue;
        };
        for member in members.iter_mut() {
            if let Some(naga::Binding::Location {
                location,
                blend_src,
                ..
            }) = &mut member.binding
            {
                if *location < 2 && blend_src.is_none() {
                    *blend_src = Some(*location);
                    *location = 0;
                    moved += 1;
                }
            }
        }
        if moved > 0 {
            fragment.types.replace(handle, ty);
        }
    }
    moved
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

/// The faces the game's rasterizer takes as the front, by wgpu's naming.
// The game's state is "counter-clockwise front, cull back", which Direct3D counts on the target
// as it is seen; wgpu's counter-clockwise front is the same on every backend. A translated mesh
// stage places a vertex by the viewport's own matrix, a camera that mirrors nothing, so a face
// that winds counter-clockwise towards the eye in the world does so on the screen.
pub(super) const FRONT: wgpu::FrontFace = wgpu::FrontFace::Ccw;

/// The format of what the forward surfaces let through of the frame behind them.
const THROUGH: wgpu::TextureFormat = LIT;

/// What multiplies the frame behind a surface laid on by `forward`; `None` where it is one.
fn let_through(forward: &Forward) -> Option<wgpu::BlendFactor> {
    let factor = forward.blend.color.dst_factor;
    (factor != wgpu::BlendFactor::One).then_some(factor)
}

/// The pipeline that multiplies a target cleared to one by what `forward`'s surface lets through
/// of the frame behind it, by the pass's own `layout`, `vertex` and `fragment` stages and their
/// entries; `None` where it lets the frame through whole.
// The blend's destination factor is what multiplies the frame: one less the first colour's
// alpha, or the second colour itself. Taken with nought of the source, it leaves that factor.
pub(super) fn through_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vertex: (&wgpu::ShaderModule, &str),
    fragment: (&wgpu::ShaderModule, &str),
    forward: &Forward,
    compiled: Option<&wgpu::PipelineCache>,
) -> Option<wgpu::RenderPipeline> {
    let factor = let_through(forward)?;
    let target = wgpu::ColorTargetState {
        format: THROUGH,
        blend: Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: factor,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        }),
        write_mask: wgpu::ColorWrites::COLOR,
    };
    Some(
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meshlet-through"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: vertex.0,
                entry_point: Some(vertex.1),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                front_face: FRONT,
                cull_mode: forward.cull,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: fragment.0,
                entry_point: Some(fragment.1),
                compilation_options: Default::default(),
                targets: &[Some(target)],
            }),
            multiview_mask: None,
            cache: compiled,
        }),
    )
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

// Whether no surface of the model and no ground lies under `corner`, as `lit_light` tells it:
// the lit frame holds there only what the forward surfaces laid on, whose alpha a lit one's blend
// leaves as its own.
fn behind_all(corner: Corner) -> bool {
    if textureLoad(depth, vec2<i32>(corner.at.xy), 0) > 0.0 {
        return false;
    }
    return light.laid.w <= 0.0 || !ground_at(corner.clip).met;
}

// The lit frame, the forward surfaces laid over it, exposed, toned and encoded, with the depth
// `lit` writes.
@fragment fn shown_frame(corner: Corner) -> Lit {
    let texel = vec2<i32>(corner.at.xy);
    let held = textureLoad(lit_frame, texel, 0);
    if behind_all(corner) {
        // No surface of the model and no ground lies here: what a forward surface laid over
        // nothing is added to the viewport's own frame, its alpha nought, at the farthest.
        if max(held.r, max(held.g, held.b)) <= 0.0 || u32(light.camera.w) != 0u {
            discard;
        }
        return Lit(vec4<f32>(shown_of(held.rgb), 0.0), 0.0);
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

// What the forward surfaces let through of what lies behind them: a product, from one.
@group(3) @binding(0) var let_through: texture_2d<f32>;

// Where no surface of the model and no ground lies, what the forward surfaces let through of the
// viewport's own frame, which the frame is multiplied by before their light is added.
@fragment fn through_frame(corner: Corner) -> @location(0) vec4<f32> {
    if !behind_all(corner) || u32(light.camera.w) != 0u {
        discard;
    }
    return vec4<f32>(textureLoad(let_through, vec2<i32>(corner.at.xy), 0).rgb, 1.0);
}
"#;

/// The forward surfaces of a scene and what they are drawn into.
// Where a forward surface lies over no surface of the model and no ground, the viewport's own
// frame is multiplied by what the surfaces let through of it, then their toned light is added.
// ponytail: that frame is toned already and the surfaces' light is toned alone, so toning the
// sum of the two is not what is shown; light the viewport's background into the lit frame if that
// difference shows.
pub(super) struct Over {
    /// Each with the middle of its surface.
    draws: Vec<(Draw, Vec3)>,
    lit: wgpu::TextureView,
    lit_layout: wgpu::BindGroupLayout,
    lit_group: wgpu::BindGroup,
    /// What the surfaces let through of the frame behind them, and its group.
    through: wgpu::TextureView,
    through_group: wgpu::BindGroup,
    /// Lights the targets into the lit frame.
    lights: wgpu::RenderPipeline,
    /// Tones the lit frame onto the frame.
    shows: wgpu::RenderPipeline,
    /// Multiplies the frame behind the surfaces, where no surface of the model lies, by `through`.
    lets_through: wgpu::RenderPipeline,
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
            // A lit texel is shown whole (its alpha one); a forward surface over nothing adds.
            wgpu::ColorTargetState {
                format: frame.colour,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        let multiplied = |dst_factor| wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        let lets_through = pipeline(
            "meshlet-lit-through",
            &[
                Some(layouts.resolve),
                Some(layouts.metered),
                Some(&lit_layout),
                Some(&lit_layout),
            ],
            "through_frame",
            wgpu::ColorTargetState {
                format: frame.colour,
                blend: Some(wgpu::BlendState {
                    color: multiplied(wgpu::BlendFactor::Src),
                    alpha: multiplied(wgpu::BlendFactor::One),
                }),
                write_mask: wgpu::ColorWrites::COLOR,
            },
            wgpu::DepthStencilState {
                format: frame.depth,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            },
        );
        let lit = lit_of(device, size);
        let lit_group = lit_group_of(device, &lit_layout, &lit);
        let through = lit_of(device, size);
        let through_group = lit_group_of(device, &lit_layout, &through);
        let mut over = Self {
            draws: Vec::new(),
            lit,
            lit_layout,
            lit_group,
            through,
            through_group,
            lights,
            shows,
            lets_through,
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

    /// The draws of its surfaces.
    pub(super) fn draws(&self) -> impl Iterator<Item = &Draw> + '_ {
        self.draws.iter().map(|(draw, _)| draw)
    }

    /// The same, to bind again.
    pub(super) fn draws_mut(&mut self) -> impl Iterator<Item = &mut Draw> + '_ {
        self.draws.iter_mut().map(|(draw, _)| draw)
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
        self.through = lit_of(device, size);
        self.through_group = lit_group_of(device, &self.lit_layout, &self.through);
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
        self.record_through(scene, encoder);
    }

    /// Multiply a target cleared to one by what each surface lets through of the frame behind it,
    /// in a pass of its own, where any surface lets less than the whole through.
    fn record_through(&self, scene: &MeshletScene, encoder: &mut wgpu::CommandEncoder) {
        let draws: Vec<(&Draw, &wgpu::RenderPipeline)> = (self.draws.iter())
            .filter_map(|(draw, _)| draw.through.as_ref().map(|through| (draw, through)))
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet-forward-through"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.through,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &scene.depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let (mut binds, mut vertices) = (0, 0);
        // A product: the order the surfaces are drawn in does not change it.
        for (draw, through) in &draws {
            pass.set_pipeline(through);
            for (group, bound) in &draw.groups {
                pass.set_bind_group(*group, bound, &[]);
                binds += 1;
            }
            let fitted = match scene.at_most.get() {
                true => VERTICES,
                false => draw.vertices,
            };
            vertices += u64::from(fitted) * u64::from(draw.meshlets);
            pass.draw(0..fitted, 0..draw.meshlets);
        }
        drop(pass);
        let drawn = draws.len() as u32;
        scene.count(|calls| {
            calls.passes += 1;
            calls.draws += drawn;
            calls.surface_draws += drawn;
            calls.binds += binds;
            calls.vertices += vertices;
        });
    }

    /// Tone the lit frame onto the frame `pass` draws, with the depth the resolve writes.
    pub(super) fn show(&self, scene: &MeshletScene, pass: &mut wgpu::RenderPass<'_>) {
        // What lies behind the surfaces where the model does not is multiplied first, then their
        // light is added over it.
        pass.set_pipeline(&self.lets_through);
        pass.set_bind_group(0, &scene.resolve_group, &[]);
        pass.set_bind_group(1, &scene.metered_group, &[]);
        pass.set_bind_group(2, &self.lit_group, &[]);
        pass.set_bind_group(3, &self.through_group, &[]);
        pass.draw(0..3, 0..1);
        scene.count_full_screen(0, 4);
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
    fn a_pass_is_built_again_for_each_way_its_stages_are_laid_on() {
        use super::super::MeshletPipelines;
        let module = Arc::new(naga::Module::default());
        let laid = |blend: wgpu::BlendState, cull: Option<wgpu::Face>| MeshletSurface {
            vertex: Arc::clone(&module),
            fragment: Arc::clone(&module),
            feeds: Default::default(),
            meshlets: 1,
            primitives: 1,
            forward: Some(Forward {
                blend,
                writes: wgpu::ColorWrites::ALL,
                cull,
                centre: Vec3::ZERO,
            }),
        };
        let premultiplied = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
        let over = laid(premultiplied, None);
        let key = MeshletPipelines::key;
        assert_eq!(key(&over), key(&laid(premultiplied, None)));
        assert_ne!(
            key(&over),
            key(&laid(wgpu::BlendState::ALPHA_BLENDING, None))
        );
        assert_ne!(
            key(&over),
            key(&laid(premultiplied, Some(wgpu::Face::Back)))
        );
        let deferred = MeshletSurface {
            forward: None,
            ..over.clone()
        };
        assert_ne!(key(&over), key(&deferred));
    }

    #[test]
    fn a_lit_stage_s_two_colours_become_the_two_sources_of_its_one_target() {
        let mut module = naga::front::wgsl::parse_str(
            "
struct Out { @location(0) light: vec4<f32>, @location(1) through: vec4<f32> }
@fragment fn main() -> Out { return Out(vec4<f32>(1.0), vec4<f32>(0.5)); }",
        )
        .expect("the WGSL parses");
        assert_eq!(dual_sourced(&mut module), 2);
        let ty = (module.entry_points[0].function.result.as_ref())
            .expect("a result")
            .ty;
        let naga::TypeInner::Struct { members, .. } = &module.types[ty].inner else {
            panic!("the result is no struct");
        };
        let bound: Vec<_> = (members.iter())
            .map(|member| match &member.binding {
                Some(naga::Binding::Location {
                    location,
                    blend_src,
                    ..
                }) => (*location, *blend_src),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(bound, [(0, Some(0)), (0, Some(1))]);
        let valid = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module);
        assert!(valid.is_ok(), "{valid:?}");
        // Only a blend that takes the second colour asks for it.
        let forward = |dst_factor| Forward {
            blend: wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::REPLACE,
            },
            writes: wgpu::ColorWrites::ALL,
            cull: None,
            centre: Vec3::ZERO,
        };
        assert!(forward(wgpu::BlendFactor::Src1).dual());
        assert!(!forward(wgpu::BlendFactor::OneMinusSrcAlpha).dual());
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

    #[test]
    fn the_game_s_front_faces_wind_counter_clockwise_through_the_viewport_s_camera() {
        use crate::viewport::OrbitCamera;
        assert_eq!(FRONT, wgpu::FrontFace::Ccw);
        let turned = OrbitCamera {
            yaw: 2.0,
            pitch: -0.4,
            distance: 7.0,
            ..OrbitCamera::default()
        };
        for camera in [OrbitCamera::default(), turned] {
            let view_proj = camera.view_proj(16.0 / 9.0);
            // Corners that run counter-clockwise about a normal towards the eye.
            let towards = (camera.eye() - camera.target).normalize();
            let across = Vec3::Z.cross(towards).normalize();
            let up = towards.cross(across);
            let corners = [camera.target, camera.target + across, camera.target + up];
            let [a, b, c] = corners.map(|at| {
                let clip = view_proj * at.extend(1.0);
                clip.truncate().truncate() / clip.w
            });
            let area = (b - a).perp_dot(c - a);
            assert!(area > 0.0, "a face towards the eye winds as {area}");
            // The same corners the other way round face away: a back face, culled.
            assert!((c - a).perp_dot(b - a) < 0.0);
        }
    }

    #[test]
    fn what_a_surface_lets_through_of_the_frame_is_its_blend_s_factor_on_the_frame() {
        let laid = |dst_factor| Forward {
            blend: wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::REPLACE,
            },
            writes: wgpu::ColorWrites::ALL,
            cull: None,
            centre: Vec3::ZERO,
        };
        let through = |factor| let_through(&laid(factor));
        let second = wgpu::BlendFactor::Src1;
        assert_eq!(
            through(second),
            Some(second),
            "a lit surface's second colour"
        );
        let rest = wgpu::BlendFactor::OneMinusSrcAlpha;
        assert_eq!(through(rest), Some(rest), "an emissive one's alpha");
        assert_eq!(through(wgpu::BlendFactor::One), None, "one that only adds");
    }

    /// A stage that lays one premultiplied colour, its constant, over what is behind it.
    const OVER: &str = "
@group(1) @binding(0) var<uniform> premultiplied: vec4<f32>;

@fragment fn main(@location(0) shade: vec3<f32>) -> @location(0) vec4<f32> {
    return premultiplied;
}";

    fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
        gpu_with(test, wgpu::Features::empty())
    }

    /// A device with `features`, where the adapter has them; `None`, said, where not.
    fn gpu_with(test: &str, features: wgpu::Features) -> Option<(wgpu::Device, wgpu::Queue)> {
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
        if !adapter.features().contains(features) {
            let missing = features.difference(adapter.features());
            or_test_support::skip_device(test, format_args!("the adapter lacks {missing:?}"));
            return None;
        }
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("forward-test"),
            required_features: features,
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
                cull: None,
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
                3,
                "the opaque surface, the square, and the square into what it lets through"
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

    /// A stage that exports a light to add and a colour to let what is behind it through by.
    const LIT_OVER: &str = "
@group(1) @binding(0) var<uniform> two: array<vec4<f32>, 2>;

struct Out { @location(0) light: vec4<f32>, @location(1) through: vec4<f32> }

@fragment fn main(@location(0) shade: vec3<f32>) -> Out {
    return Out(two[0], two[1]);
}";

    #[cfg_attr(
        not(feature = "gpu-tests"),
        ignore = "needs a GPU adapter: acquisition can hang, not just fail"
    )]
    #[test]
    fn a_lit_quad_adds_its_light_and_lets_its_second_colour_of_what_is_behind_it_through() {
        use super::super::tests::{surface, MESH, SIZE};
        use super::super::{Feed, Lighting, MeshletPipelines, Scene, Sky};
        let test = "a_lit_quad_adds_its_light_and_lets_its_second_colour_through";
        let Some((device, queue)) = gpu_with(test, wgpu::Features::DUAL_SOURCE_BLENDING) else {
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
        let (vertex, fragment) = (stage(MESH), stage(LIT_OVER));
        // A square over the middle, nearer than the opaque surface, its corners counter-clockwise
        // on the screen: the front the engine's lit state draws, its back culled.
        let corners: [[f32; 3]; 6] = [
            [-0.4, -0.4, 0.75],
            [0.4, -0.4, 0.75],
            [0.4, 0.4, 0.75],
            [-0.4, -0.4, 0.75],
            [0.4, 0.4, 0.75],
            [-0.4, 0.4, 0.75],
        ];
        let words: Vec<u32> = corners.iter().flatten().map(|v| v.to_bits()).collect();
        let summed = |src_factor, dst_factor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
        let quad = |two: [[f32; 4]; 2]| MeshletSurface {
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
                ((1, 0), Feed::Bytes(bytemuck::cast_slice(&two).to_vec())),
            ]),
            meshlets: 2,
            primitives: 1,
            forward: Some(Forward {
                blend: wgpu::BlendState {
                    color: summed(wgpu::BlendFactor::One, wgpu::BlendFactor::Src1),
                    alpha: summed(wgpu::BlendFactor::One, wgpu::BlendFactor::Src1Alpha),
                },
                writes: wgpu::ColorWrites::ALL,
                cull: Some(wgpu::Face::Back),
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
        // The lit frame and what the square lets through, at the middle.
        let mut middle = |two: [[f32; 4]; 2]| -> ([f32; 4], [f32; 4]) {
            let surfaces = [surface(), quad(two)];
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
            let over = scene.over.as_ref().expect("a forward surface");
            let at = ((SIZE / 2 * SIZE + SIZE / 2) * 8) as usize;
            let texel = |texture: &wgpu::Texture| {
                let read =
                    crate::game_shaders::frame::read_back(&device, &queue, texture, LIT, SIZE);
                let half = |i: usize| {
                    let bytes = [read.bytes[at + 2 * i], read.bytes[at + 2 * i + 1]];
                    or_io::half::half_to_f32(u16::from_le_bytes(bytes))
                };
                [half(0), half(1), half(2), half(3)]
            };
            (texel(over.lit.texture()), texel(over.through.texture()))
        };
        let (behind, whole) = middle([[0.0; 4], [1.0; 4]]);
        let light = [0.25, 0.5, 0.125, 0.5];
        let through = [0.5, 0.25, 1.0, 0.5];
        let (laid, let_through) = middle([light, through]);
        assert_eq!(
            pollster::block_on(validation.pop()).map(|e| e.to_string()),
            None
        );
        assert!(behind[0] > 0.1 && behind[3] == 1.0, "{behind:?}");
        assert_eq!(whole[..3], [1.0; 3], "nothing let through but the whole");
        // Its light, and the frame times its second colour, on every channel the alpha's too.
        for c in 0..4 {
            let wanted = light[c] + behind[c] * through[c];
            let got = laid[c];
            assert!(
                (got - wanted).abs() <= 0.01 * wanted.abs().max(1.0),
                "{laid:?} against {behind:?}"
            );
        }
        assert_eq!(
            let_through[..3],
            through[..3],
            "what is behind is let through by"
        );
    }
}
