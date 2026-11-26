//! GTA VI's passes after its frame is lit, by the build's own values: the temporal anti-aliasing over the lit frame, the bloom of eight sizes laid in place of a part of it, the per-pixel exposure, and the sharpening of the mapped frame.

use std::cell::Cell;
use std::time::Instant;

use glam::{Mat4, Vec2, Vec3};

use super::post_math::{
    history_weight, jitter_table, neighbourhood_weights, HistoryTimes, ResolveKernel,
    STANDIN_JITTER_PERIOD, STANDIN_JITTER_SCALE,
};
use super::post_shaders::{module, Part};
use crate::game_shaders::deferred::HDR_FORMAT;
use crate::game_shaders::grade::Scotopic;
use crate::game_shaders::tone::{Gta6Post, Temporal, BLOOM_SIZES, COMPENSATION_SAMPLES};

/// The longest a frame counts for when the history takes its share, in seconds: a slow frame converges by frames, not by time.
const LONGEST_FRAME: f32 = 1.0 / 30.0;
/// How far, in metres, the camera goes in one frame before its history is thrown away as another place.
const CUT: f32 = 25.0;
/// The size of blur, counted from the half, whose texels stand for the light about a pixel in the per-pixel exposure.
const LOCAL_SIZE: usize = 4;

/// The share of the frame a pixel's history takes in from this frame, over `seconds`, to converge in `converge` seconds.
pub(super) fn share(seconds: f32, converge: f32) -> f32 {
    let seconds = seconds.clamp(0.0, LONGEST_FRAME);
    1.0 - (-seconds / converge.max(1e-3)).exp()
}

/// The blend factor each doubling of the bloom lays the smaller size over the larger by, from the smallest up: what the weights, made to sum to one, leave of what is still to come.
pub(super) fn doublings(weights: &[f32; BLOOM_SIZES]) -> [f32; BLOOM_SIZES - 1] {
    let total: f32 = weights.iter().sum::<f32>().max(0.01);
    // What is left to come after size k, of all of them
    let mut after = [0.0f32; BLOOM_SIZES + 1];
    for k in (0..BLOOM_SIZES).rev() {
        after[k] = after[k + 1] + weights[k] / total;
    }
    let mut factors = [0.0; BLOOM_SIZES - 1];
    // The doubling into size k lays the sizes after it, already summed among themselves, over size k alone
    for (k, factor) in factors.iter_mut().enumerate() {
        *factor = match after[k] > 1e-6 {
            true => after[k + 1] / after[k],
            false => 0.0,
        };
    }
    factors
}

/// The camera's offset of frame `frame` in pixels, x right and y up: a point of the build's sixteen, cycled over the stand-in period and scale.
pub(super) fn offset(frame: u32) -> Vec2 {
    // Stand-in: the build's period and scale have no writer found; 8 frames and half a pixel are inferred.
    jitter_table()[frame as usize % STANDIN_JITTER_PERIOD] * STANDIN_JITTER_SCALE
}

/// The camera's offset this frame, as a change of the clip space.
pub(super) fn jitter(frame: u32, (width, height): (u32, u32)) -> Mat4 {
    let j = offset(frame);
    let ndc = Vec3::new(
        2.0 * j.x / width.max(1) as f32,
        2.0 * j.y / height.max(1) as f32,
        0.0,
    );
    Mat4::from_translation(ndc)
}

/// The resolve's nine tent weights for frame `frame`: centred where the pixel's own middle fell on the offset frame, normalised to sum one.
pub(super) fn resolve_weights(frame: u32) -> [f32; 9] {
    // The offset moves the image by (x, -y) texels, x right and y down: the pixel's middle lies there.
    // Inferred: the build's (-0.5 j.x, 0.5 j.y) is this point in its own units of the offset.
    let j = offset(frame);
    neighbourhood_weights(ResolveKernel::Tent, Vec2::new(j.x, -j.y), true)
}

/// The share of this frame a pixel near the camera takes in after `seconds`, the camera moving `speed` metres a second along the view: the build's history weight, toward the forward time as the speed rises.
pub(super) fn near_share(seconds: f32, speed: f32, temporal: &Temporal) -> f32 {
    let seconds = seconds.clamp(0.0, LONGEST_FRAME);
    let times = HistoryTimes {
        base: temporal.near,
        second: temporal.near,
        moving: temporal.forward,
        speed_low: temporal.forward_speed[0],
        speed_high: temporal.forward_speed[1],
    };
    // Inferred: the build's speed is the view's forward axis dotted with the camera's motion; its unit was not read.
    1.0 - history_weight(0.0, 0.0, seconds, &times, true, speed, false).0
}

/// The targets of one size of frame.
struct Targets {
    size: (u32, u32),
    sizes: Vec<wgpu::TextureView>,
    local: wgpu::TextureView,
    scratch: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
}

impl Targets {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let target = |label, (w, h): (u32, u32), usage| {
            crate::game_shaders::targets::render_target(device, label, (w, h), HDR_FORMAT, usage)
        };
        let sizes = (1..=BLOOM_SIZES as u32)
            .map(|k| {
                let at = ((size.0 >> k).max(1), (size.1 >> k).max(1));
                target("post-bloom", at, wgpu::TextureUsages::COPY_SRC)
            })
            .collect::<Vec<_>>();
        let local_size = sizes[LOCAL_SIZE].texture().size();
        Self {
            size,
            local: target(
                "post-local",
                (local_size.width, local_size.height),
                wgpu::TextureUsages::COPY_DST,
            ),
            sizes,
            scratch: target("post-scratch", size, wgpu::TextureUsages::COPY_SRC),
            history: [0, 1].map(|_| target("post-history", size, wgpu::TextureUsages::COPY_SRC)),
        }
    }
}

/// The bind groups of one frame's targets.
struct Bound {
    halve: Vec<wgpu::BindGroup>,
    double: Vec<wgpu::BindGroup>,
    composite: wgpu::BindGroup,
    /// Reading history 0 and history 1.
    temporal: [wgpu::BindGroup; 2],
    sharpen: Option<wgpu::BindGroup>,
}

/// Where the history stands from frame to frame.
#[derive(Clone, Copy)]
struct Kept {
    frame: u32,
    /// The history last written, and whether it holds a frame of this place.
    written: usize,
    valid: bool,
    before: Mat4,
    eye: Vec3,
    at: Option<Instant>,
    /// The linear exposure the history holds its light at.
    exposure: f32,
}

/// GTA VI's passes after the frame is lit.
pub(super) struct Post {
    halve_first: wgpu::RenderPipeline,
    halve: wgpu::RenderPipeline,
    double: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    temporal: wgpu::RenderPipeline,
    sharpen: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    composite_values: wgpu::Buffer,
    temporal_values: wgpu::Buffer,
    sharpen_values: wgpu::Buffer,
    targets: Option<Targets>,
    bound: Option<Bound>,
    kept: Cell<Kept>,
}

fn pipeline(
    device: &wgpu::Device,
    part: Part,
    entry: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("post"),
        source: wgpu::ShaderSource::Wgsl(module(part).into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn uniform(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("post-values"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A pass drawing the screen's triangle into `view`.
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    blend: Option<f32>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("post"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    if let Some(factor) = blend {
        let f = f64::from(factor);
        pass.set_blend_constant(wgpu::Color {
            r: f,
            g: f,
            b: f,
            a: f,
        });
    }
    pass.draw(0..3, 0..1);
}

fn copy(encoder: &mut wgpu::CommandEncoder, from: &wgpu::TextureView, to: &wgpu::TextureView) {
    encoder.copy_texture_to_texture(
        from.texture().as_image_copy(),
        to.texture().as_image_copy(),
        from.texture().size(),
    );
}

impl Post {
    /// The passes, the sharpening drawn into targets of `display`.
    pub(super) fn new(device: &wgpu::Device, display: wgpu::TextureFormat) -> Self {
        let all = wgpu::ColorWrites::ALL;
        // A doubling lays the smaller size over the larger by the blend constant
        let laid = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Constant,
                dst_factor: wgpu::BlendFactor::OneMinusConstant,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        Self {
            halve_first: pipeline(device, Part::Halve, "fs_halve_first", HDR_FORMAT, None, all),
            halve: pipeline(device, Part::Halve, "fs_halve", HDR_FORMAT, None, all),
            double: pipeline(
                device,
                Part::Halve,
                "fs_double",
                HDR_FORMAT,
                Some(laid),
                all,
            ),
            composite: pipeline(
                device,
                Part::Composite,
                "fs_composite",
                HDR_FORMAT,
                None,
                all,
            ),
            temporal: pipeline(device, Part::Temporal, "fs_temporal", HDR_FORMAT, None, all),
            // The screen keeps the alpha it was cleared to
            sharpen: pipeline(
                device,
                Part::Sharpen,
                "fs_sharpen",
                display,
                None,
                wgpu::ColorWrites::COLOR,
            ),
            sampler: crate::game_shaders::samplers::linear(device),
            composite_values: uniform(device, 16 * (4 + COMPENSATION_SAMPLES as u64 / 4)),
            temporal_values: uniform(device, 64 * 3 + 16 * 6),
            sharpen_values: uniform(device, 48),
            targets: None,
            bound: None,
            kept: Cell::new(Kept {
                frame: 0,
                written: 0,
                valid: false,
                before: Mat4::IDENTITY,
                eye: Vec3::ZERO,
                at: None,
                exposure: 1.0,
            }),
        }
    }

    /// Make the targets for a frame of `size` where they are of another, bind this frame's `hdr`, `depth` and `ldr`, and step the camera's offset on.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        ldr: &wgpu::TextureView,
    ) {
        let size = hdr.texture().size();
        let size = (size.width, size.height);
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(Targets::new(device, size));
            let kept = self.kept.get();
            self.kept.set(Kept {
                valid: false,
                ..kept
            });
        }
        let Some(t) = &self.targets else {
            return;
        };
        let group = |pipeline: &wgpu::RenderPipeline,
                     views: &[(u32, &wgpu::TextureView)],
                     values: Option<(u32, &wgpu::Buffer)>| {
            let mut entries: Vec<wgpu::BindGroupEntry> = views
                .iter()
                .map(|(binding, view)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: wgpu::BindingResource::TextureView(view),
                })
                .collect();
            // The sharpening reads texels whole and binds no sampler
            if !std::ptr::eq(pipeline, &self.sharpen) {
                entries.push(wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                });
            }
            if let Some((binding, buffer)) = values {
                entries.push(wgpu::BindGroupEntry {
                    binding,
                    resource: buffer.as_entire_binding(),
                });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            })
        };
        // Each halving reads the size above it, the first the frame; each doubling the size below it
        let halve = (0..BLOOM_SIZES)
            .map(|k| {
                let (pipeline, from) = match k {
                    0 => (&self.halve_first, hdr),
                    _ => (&self.halve, &t.sizes[k - 1]),
                };
                group(pipeline, &[(0, from)], None)
            })
            .collect();
        let double = (1..BLOOM_SIZES)
            .map(|k| group(&self.double, &[(0, &t.sizes[k])], None))
            .collect();
        let composite = group(
            &self.composite,
            &[(0, hdr), (2, &t.sizes[0]), (3, &t.local)],
            Some((4, &self.composite_values)),
        );
        let temporal = [0, 1].map(|read| {
            group(
                &self.temporal,
                &[(0, hdr), (2, &t.history[read]), (3, depth)],
                Some((4, &self.temporal_values)),
            )
        });
        let sharpen = Some(group(
            &self.sharpen,
            &[(0, ldr), (3, hdr)],
            Some((2, &self.sharpen_values)),
        ));
        self.bound = Some(Bound {
            halve,
            double,
            composite,
            temporal,
            sharpen,
        });
        let kept = self.kept.get();
        self.kept.set(Kept {
            frame: kept.frame.wrapping_add(1),
            ..kept
        });
    }

    /// The camera's offset of this frame, as a change of the clip space.
    pub(super) fn jitter(&self, size: (u32, u32)) -> Mat4 {
        jitter(self.kept.get().frame, size)
    }

    /// Lay the lit frame `hdr`, drawn at the linear `exposure`, over its history and leave the result in `hdr`; `unjittered` is this frame's view and projection without the offset, `jittered` with it, and `eye` the camera.
    pub(super) fn record_temporal(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        hdr: &wgpu::TextureView,
        (settings, exposure): (&Gta6Post, f32),
        (unjittered, jittered, eye): (Mat4, Mat4, Vec3),
    ) {
        let (Some(t), Some(bound)) = (&self.targets, &self.bound) else {
            return;
        };
        let kept = self.kept.get();
        let now = or_transport::clock::now();
        let seconds = kept
            .at
            .map_or(0.0, |at| now.duration_since(at).as_secs_f32());
        let valid = kept.valid && kept.eye.distance(eye) < CUT;
        let temporal = settings.temporal;
        // A point ahead on the middle of the screen gives the view's forward axis, whatever the depth's direction.
        let ahead = unjittered
            .inverse()
            .project_point3(Vec3::new(0.0, 0.0, 0.5));
        let forward = (ahead - eye).normalize_or_zero();
        let speed = match valid && seconds > 0.0 {
            true => forward.dot(eye - kept.eye) / seconds,
            false => 0.0,
        };
        let mut values: Vec<f32> = Vec::with_capacity(72);
        values.extend(jittered.inverse().to_cols_array());
        values.extend(unjittered.to_cols_array());
        values.extend(kept.before.to_cols_array());
        values.extend([eye.x, eye.y, eye.z, 1.0]);
        values.extend([
            near_share(seconds, speed, &temporal),
            share(seconds, temporal.far),
            temporal.far_depth,
            temporal.gamma,
        ]);
        // The history is brought to this frame's exposure: the frame holds its light times the exposure, which moves as it adapts.
        let rescale = exposure / kept.exposure.max(f32::MIN_POSITIVE);
        values.extend([f32::from(u8::from(valid)), rescale, 0.0, 0.0]);
        values.extend(resolve_weights(kept.frame));
        values.extend([0.0; 3]);
        queue.write_buffer(&self.temporal_values, 0, bytemuck::cast_slice(&values));
        let (read, write) = (kept.written, 1 - kept.written);
        draw(
            encoder,
            &t.history[write],
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            &self.temporal,
            &bound.temporal[read],
            None,
        );
        copy(encoder, &t.history[write], hdr);
        self.kept.set(Kept {
            written: write,
            valid: true,
            before: unjittered,
            eye,
            at: Some(now),
            exposure,
            ..kept
        });
    }

    /// Forget the history and start the camera's offsets from their first, so that a frame drawn a given number of frames later is one frame whatever came before.
    pub(super) fn restart(&self) {
        self.kept.set(Kept {
            frame: 0,
            written: 0,
            valid: false,
            at: None,
            ..self.kept.get()
        });
    }

    /// Forget the history, so that the next frame starts it again.
    pub(super) fn forget(&self) {
        let kept = self.kept.get();
        self.kept.set(Kept {
            valid: false,
            at: None,
            ..kept
        });
    }

    /// Blur the lit frame `hdr` at eight sizes, lay the blur in place of a part of it and expose each pixel by the light about it, leaving the result in `hdr`; `exposure` is the linear exposure the frame was lit at.
    pub(super) fn record_bloom(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        hdr: &wgpu::TextureView,
        settings: &Gta6Post,
        exposure: f32,
    ) {
        let (Some(t), Some(bound)) = (&self.targets, &self.bound) else {
            return;
        };
        let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
        for (k, bind) in bound.halve.iter().enumerate() {
            let pipeline = if k == 0 {
                &self.halve_first
            } else {
                &self.halve
            };
            draw(encoder, &t.sizes[k], clear, pipeline, bind, None);
        }
        // The light about a pixel is read before the doublings sum the sizes into it
        copy(encoder, &t.sizes[LOCAL_SIZE], &t.local);
        let factors = doublings(&settings.bloom_weights);
        for k in (1..BLOOM_SIZES).rev() {
            let into = &t.sizes[k - 1];
            let factor = factors[k - 1];
            draw(
                encoder,
                into,
                wgpu::LoadOp::Load,
                &self.double,
                &bound.double[k - 1],
                Some(factor),
            );
        }
        let local = settings.local;
        let mut values: Vec<f32> = vec![
            settings.bloom_mix,
            local.amount,
            exposure.max(f32::MIN_POSITIVE).log2(),
            exposure,
            local.scale,
            local.horizontal_offset,
            local.vertical_offset,
            local.bending,
            local.least,
            local.most,
            local.offset,
            local.contrast,
            local.input[0],
            local.input[1],
            local.output[0],
            local.output[1],
        ];
        values.extend(local.compensation);
        queue.write_buffer(&self.composite_values, 0, bytemuck::cast_slice(&values));
        draw(
            encoder,
            &t.scratch,
            clear,
            &self.composite,
            &bound.composite,
            None,
        );
        copy(encoder, &t.scratch, hdr);
    }

    /// Shift the mapped frame's darkest pixels by `scotopic` and sharpen it by `amount` into `screen`, a target of the display format.
    pub(super) fn record_sharpen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        screen: &wgpu::TextureView,
        background: wgpu::Color,
        (amount, scotopic): (f32, &Scotopic),
    ) {
        let Some(bind) = self.bound.as_ref().and_then(|b| b.sharpen.as_ref()) else {
            return;
        };
        let mut values = vec![amount, 0.0, 0.0, 0.0];
        values.extend(scotopic.words());
        queue.write_buffer(&self.sharpen_values, 0, bytemuck::cast_slice(&values));
        draw(
            encoder,
            screen,
            wgpu::LoadOp::Clear(background),
            &self.sharpen,
            bind,
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_doublings_leave_each_size_of_the_bloom_its_weight_of_the_whole() {
        let weights = [0.712, 0.588, 0.449, 0.384, 0.345, 0.325, 0.319, 0.317];
        let factors = doublings(&weights);
        // Lay the sizes up as the passes do: each doubling mixes what is summed below over the size by its factor
        let mut share = [0.0f32; BLOOM_SIZES];
        share[BLOOM_SIZES - 1] = 1.0;
        for k in (1..BLOOM_SIZES).rev() {
            let f = factors[k - 1];
            for s in share.iter_mut() {
                *s *= f;
            }
            share[k - 1] += 1.0 - f;
        }
        let total: f32 = weights.iter().sum();
        for (k, s) in share.iter().enumerate() {
            assert!((s - weights[k] / total).abs() < 1e-5, "size {k}: {s}");
        }
        // The last doubling keeps of the half size its own weight
        assert!((factors[0] - (1.0 - weights[0] / total)).abs() < 1e-5);
    }

    #[test]
    fn a_history_converges_in_its_time_and_a_slow_frame_counts_as_a_thirtieth() {
        let fast = share(1.0 / 120.0, 0.15);
        assert!(
            (fast - (1.0 - (-1.0f32 / 18.0).exp())).abs() < 1e-6,
            "{fast}"
        );
        assert_eq!(share(2.0, 0.15), share(LONGEST_FRAME, 0.15));
        assert_eq!(share(0.0, 0.15), 0.0);
        assert!(share(LONGEST_FRAME, 1.0) < share(LONGEST_FRAME, 0.15));
    }

    #[test]
    fn the_offsets_stay_within_the_pixel_and_average_to_its_middle() {
        let offsets: Vec<Vec2> = (0..STANDIN_JITTER_PERIOD as u32).map(offset).collect();
        let mean = offsets.iter().copied().sum::<Vec2>() / offsets.len() as f32;
        assert!(mean.x.abs() < 0.07 && mean.y.abs() < 0.07, "{mean:?}");
        assert!(offsets.iter().all(|j| j.x.abs() <= 0.5 && j.y.abs() <= 0.5));
        // Frame 2 is the table's entry 2, (1/2, -7/9), at half a pixel.
        let m = jitter(2, (1280, 720));
        let moved = m * glam::Vec4::new(0.0, 0.0, 0.5, 2.0);
        assert!((moved.x / moved.w - 2.0 * 0.25 / 1280.0).abs() < 1e-7);
    }

    #[test]
    fn the_resolve_weighs_the_texels_about_where_the_pixels_middle_fell() {
        // Frame 2 offsets the image by (0.25, 7/18) texels right and down: the tent splits x 3:1
        // between the middle and the right, y 11:7 between the middle and the row below.
        let w = resolve_weights(2);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        let (x, y) = (0.25f32, 7.0f32 / 18.0);
        assert!((w[4] - (1.0 - x) * (1.0 - y)).abs() < 1e-5, "{w:?}");
        assert!((w[5] - x * (1.0 - y)).abs() < 1e-5);
        assert!((w[7] - (1.0 - x) * y).abs() < 1e-5);
        assert_eq!([w[0], w[1], w[2], w[3], w[6]], [0.0; 5]);
        // Frame 0 is entry 0, (0, -1/3): no move across, a sixth of a texel down.
        let w = resolve_weights(0);
        assert!((w[4] - 5.0 / 6.0).abs() < 1e-5 && (w[7] - 1.0 / 6.0).abs() < 1e-5);
    }

    #[test]
    fn a_camera_moving_forward_takes_in_less_of_its_frame_near_it() {
        let temporal = Temporal {
            near: 0.15,
            far: 1.0,
            far_depth: 250.0,
            forward: 0.2,
            forward_speed: [0.0, 0.1],
            gamma: 1.25,
        };
        let dt = 1.0 / 60.0;
        let still = near_share(dt, 0.0, &temporal);
        assert!((still - share(dt, 0.15)).abs() < 1e-6, "{still}");
        let moving = near_share(dt, 5.0, &temporal);
        assert!((moving - share(dt, 0.2)).abs() < 1e-6, "{moving}");
        // Half way between the speeds, the history weight is half way between the two.
        let half = near_share(dt, 0.05, &temporal);
        let weight = 0.5 * ((-dt / 0.15f32).exp() + (-dt / 0.2f32).exp());
        assert!((half - (1.0 - weight)).abs() < 1e-6, "{half}");
    }
}
