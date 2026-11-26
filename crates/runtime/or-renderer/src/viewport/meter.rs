//! The lit frame's own luminance, counted pixel by pixel into 256 logarithmic bins between its least and its most before it is mapped, and read back a frame or two late, which the exposure is metered on.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The threads of one workgroup, across and down: each counts a tile of the frame.
const TILE: u32 = 16;
/// The bins of the meter's histogram, logarithmic between the least and the most luminance of the frame.
const BINS: usize = 256;
/// The words read back: the least and the most luminance's bits, then the count of each bin.
const WORDS: usize = 2 + BINS;
/// The least luminance a pixel counts at, in cd/m², so that black and what is not a number fall in the first bin.
// Stand-in: the build floors at a member of its meter's block whose value was not read.
const LEAST: f32 = 1e-6;

const SHADER: &str = r#"
@group(0) @binding(0) var frame: texture_2d<f32>;
// The least and the most luminance's bits (positive floats order as their bits), then the bins
@group(0) @binding(1) var<storage, read_write> counted: array<atomic<u32>>;
// The linear exposure the frame holds its light at
@group(0) @binding(2) var<uniform> exposure: vec4<f32>;

const LEAST: f32 = 1e-6;
var<workgroup> tile: array<atomic<u32>, 256>;

// The scene's luminance at a pixel, in cd/m², as the build's meter takes it out of its pre-exposed frame.
fn luminance_at(at: vec2<u32>) -> f32 {
    let l = dot(textureLoad(frame, at, 0).rgb, vec3<f32>(0.2125, 0.7154, 0.0721));
    // Nothing that is not a number, nor past what half floats hold, counts above the least.
    if (!(l >= 0.0) || l > 65504.0) {
        return LEAST;
    }
    return max(LEAST, l / exposure.x);
}

@compute @workgroup_size(16, 16)
fn bounds(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(frame))) {
        return;
    }
    let bits = bitcast<u32>(luminance_at(id.xy));
    atomicMin(&counted[0], bits);
    atomicMax(&counted[1], bits);
}

@compute @workgroup_size(16, 16)
fn histogram(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    atomicStore(&tile[local], 0u);
    workgroupBarrier();
    if (all(id.xy < textureDimensions(frame))) {
        let least = bitcast<f32>(atomicLoad(&counted[0]));
        let most = bitcast<f32>(atomicLoad(&counted[1]));
        let along = log(luminance_at(id.xy) / least) / max(1e-8, log(most / least));
        atomicAdd(&tile[min(u32(max(0.0, 255.0 * along)), 255u)], 1u);
    }
    workgroupBarrier();
    let n = atomicLoad(&tile[local]);
    if (n > 0u) {
        atomicAdd(&counted[2u + local], n);
    }
}
"#;

/// The bin of 256 logarithmic between `least` and `most` that `luminance` falls in, as the build counts it: cut down, never rounded.
fn bin_of(luminance: f32, least: f32, most: f32) -> usize {
    let span = (most / least).ln().max(1e-8);
    (((luminance / least).ln() / span * 255.0).max(0.0) as usize).min(BINS - 1)
}

/// The frame's pixels as the meter counted them: the scene's least and most luminance in cd/m², and how many pixels fell in each bin.
#[derive(Debug, Clone, PartialEq)]
pub struct Metered {
    least: f32,
    most: f32,
    bins: Vec<u32>,
}

impl Metered {
    /// The scene's luminance at the pixels `samples` of a frame, in cd/m², counted as the pass counts them.
    pub fn of(samples: Vec<f32>) -> Self {
        let floor = |l: f32| match l >= LEAST && l.is_finite() {
            true => l,
            false => LEAST,
        };
        let least = samples.iter().copied().map(floor).fold(f32::MAX, f32::min);
        let most = samples.iter().copied().map(floor).fold(LEAST, f32::max);
        let mut bins = vec![0; BINS];
        for l in samples {
            bins[bin_of(floor(l), least, most)] += 1;
        }
        Self { least, most, bins }
    }

    /// The average the meter reads: the mean luminance of the pixels left when the darkest `low` and the brightest `high` parts of them are put aside, each counted at its bin's lower edge; `None` where nothing was counted.
    pub fn average(&self, low: f32, high: f32) -> Option<f32> {
        let total: u32 = self.bins.iter().sum();
        if total == 0 {
            return None;
        }
        let total = total as f32;
        let from = (low.clamp(0.0, 1.0) * total).round();
        let to = from + (total - from - (high.clamp(0.0, 1.0) * total).round()).max(0.0);
        let ratio = (self.most / self.least).max(1.0);
        let (mut below, mut sum, mut kept) = (0.0f32, 0.0f32, 0.0f32);
        for (bin, &n) in self.bins.iter().enumerate() {
            let n = n as f32;
            let taken = (below + n).clamp(from, to) - below.clamp(from, to);
            let edge = self.least * ratio.powf(bin as f32 / 255.0);
            sum += taken * edge.clamp(self.least, self.most);
            kept += taken;
            below += n;
        }
        (kept > 0.0).then(|| sum / kept)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    /// A frame's counts are copied for reading, once its commands have been submitted.
    Recorded,
    Mapping,
}

/// The passes that count the lit frame, and the buffers their counts are read back through.
pub(super) struct Meter {
    bounds: wgpu::ComputePipeline,
    histogram: wgpu::ComputePipeline,
    counted: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// The linear exposure the frame holds its light at.
    exposure: wgpu::Buffer,
    bound: RefCell<Option<(wgpu::BindGroup, (u32, u32))>>,
    phase: Cell<Phase>,
    ready: Arc<AtomicBool>,
    /// The revision of the day the frame being read was drawn under.
    recorded_of: Cell<u64>,
    last: RefCell<Option<(u64, Metered)>>,
}

impl Meter {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport-meter"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport-meter"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport-meter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("viewport-meter"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let bytes = (WORDS * 4) as u64;
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            bounds: pipeline("bounds"),
            histogram: pipeline("histogram"),
            counted: buffer(
                "viewport-meter-counted",
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            ),
            readback: buffer(
                "viewport-meter-readback",
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ),
            exposure: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport-meter-exposure"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            bound: RefCell::new(None),
            phase: Cell::new(Phase::Idle),
            ready: Arc::new(AtomicBool::new(false)),
            recorded_of: Cell::new(0),
            last: RefCell::new(None),
        }
    }

    /// Bind `frame`, the lit target before it is mapped, for the next [`Meter::record`].
    pub(super) fn bind(&self, device: &wgpu::Device, frame: &wgpu::TextureView) {
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport-meter"),
            layout: &self.bounds.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(frame),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.counted.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.exposure.as_entire_binding(),
                },
            ],
        });
        let size = frame.texture().size();
        *self.bound.borrow_mut() = Some((bind, (size.width, size.height)));
    }

    /// Count the bound frame, drawn under the day of `revision` at the linear `exposure`, unless the last counts are still being read.
    pub(super) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        (revision, exposure): (u64, f32),
    ) {
        let bound = self.bound.borrow();
        let Some((bind, (width, height))) =
            bound.as_ref().filter(|_| self.phase.get() == Phase::Idle)
        else {
            return;
        };
        let mut start = vec![0u32; WORDS];
        start[0] = f32::MAX.to_bits();
        queue.write_buffer(&self.counted, 0, bytemuck::cast_slice(&start));
        let exposure = [exposure.max(f32::MIN_POSITIVE), 0.0, 0.0, 0.0];
        queue.write_buffer(&self.exposure, 0, bytemuck::cast_slice(&exposure));
        let groups = (width.div_ceil(TILE), height.div_ceil(TILE));
        // The bounds are whole before any pixel is binned between them
        for pipeline in [&self.bounds, &self.histogram] {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("viewport-meter"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.counted, 0, &self.readback, 0, self.counted.size());
        self.recorded_of.set(revision);
        self.phase.set(Phase::Recorded);
    }

    /// Read the counts an earlier [`Meter::record`] copied, once the device has finished them; call it after that frame's commands were submitted.
    pub(super) fn collect(&self, device: &wgpu::Device) {
        match self.phase.get() {
            Phase::Idle => return,
            Phase::Recorded => {
                self.ready.store(false, Ordering::Release);
                let ready = Arc::clone(&self.ready);
                self.readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |read| {
                        ready.store(read.is_ok(), Ordering::Release)
                    });
                self.phase.set(Phase::Mapping);
            }
            Phase::Mapping => {}
        }
        // A poll that has nothing to finish is not an error
        let _ = device.poll(wgpu::PollType::Poll);
        if !self.ready.load(Ordering::Acquire) {
            return;
        }
        let words: Vec<u32> = {
            let view = self.readback.slice(..).get_mapped_range();
            let words = view.as_chunks::<4>().0;
            words.iter().map(|b| u32::from_le_bytes(*b)).collect()
        };
        self.readback.unmap();
        let metered = Metered {
            least: f32::from_bits(words[0]),
            most: f32::from_bits(words[1]),
            bins: words[2..].to_vec(),
        };
        *self.last.borrow_mut() = Some((self.recorded_of.get(), metered));
        self.phase.set(Phase::Idle);
    }

    /// The last counts read, where they were drawn under the day of `revision`.
    pub(super) fn last(&self, revision: u64) -> Option<Metered> {
        let last = self.last.borrow();
        let (of, metered) = last.as_ref()?;
        (*of == revision).then(|| metered.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_reads_the_mean_of_what_is_left_once_the_darkest_and_brightest_parts_are_put_aside()
    {
        let even = Metered::of(vec![3.0; 50]);
        assert!((even.average(0.3, 0.3).expect("lit") - 3.0).abs() < 1e-5);
        // Seventy dark and thirty bright: the middle two fifths are all dark.
        let mut frame = vec![1.0; 70];
        frame.extend([100.0; 30]);
        let frame = Metered::of(frame);
        let middle = frame.average(0.3, 0.3).expect("lit");
        assert!((middle - 1.0).abs() < 1e-4, "{middle}");
        let whole = frame.average(0.0, 0.0).expect("lit");
        assert!((whole - 30.7).abs() < 1e-3, "{whole}");
        // A bright quarter put aside leaves the rest's mean.
        let top = frame.average(0.0, 0.3).expect("lit");
        assert!((top - 1.0).abs() < 1e-4, "{top}");
        assert_eq!(Metered::of(Vec::new()).average(0.0, 0.0), None);
    }

    #[test]
    fn black_pixels_count_at_the_least_and_a_bin_counts_at_its_lower_edge() {
        // Half the frame black: it is counted, so the darkest half put aside leaves the lit half.
        let mut frame = vec![0.0; 50];
        frame.extend([2.0; 50]);
        let frame = Metered::of(frame);
        let lit = frame.average(0.5, 0.0).expect("counted");
        assert!((lit - 2.0).abs() < 1e-4, "{lit}");
        let all = frame.average(0.0, 0.0).expect("counted");
        assert!((all - 1.0).abs() < 1e-3, "{all}");
        // A pixel just under the next bin's edge counts at its own bin's edge.
        let edge = 10.0f32.powf(1.0 / 255.0);
        let frame = Metered::of(vec![1.0, 1.0, 10.0, edge * 0.999]);
        let mean = frame.average(0.0, 0.0).expect("counted");
        assert!((mean - 3.25).abs() < 1e-4, "{mean}");
    }
}
