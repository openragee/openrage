//! The viewport's GPU texture budget: each upload's bytes are counted as it is
//! made, and the least recently used texture goes once the total passes the
//! budget the preferences keep.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use or_io::codec::texture::TextureSource;

/// `None` is the active selection's texture, `Some(entry)` a thumbnail entry's.
pub type GpuTextureKey = (Option<u64>, String);

/// What an upload is made from: a texture's format, size and levels, and every byte it stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureStamp(u64);

impl TextureStamp {
    pub fn of(texture: &impl TextureSource) -> Self {
        let mut hasher = std::hash::DefaultHasher::new();
        let format = texture.pixel_format() as u32;
        let shape = (texture.stored_size(), texture.stored_levels());
        (format, texture.is_srgb(), shape).hash(&mut hasher);
        texture.payload().hash(&mut hasher);
        Self(hasher.finish())
    }

    /// The stamp of an upload made from what `read` identifies, where hashing its bytes would read them.
    pub fn of_read(read: u64) -> Self {
        Self(read)
    }
}

/// What the viewport holds on the GPU, published to the state for the Statistics panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuUsage {
    pub texture_bytes: usize,
    pub buffer_bytes: usize,
    pub target_bytes: usize,
    pub thumbnail_bytes: usize,
    pub line_buffer_bytes: usize,
    pub particle_instance_bytes: usize,
    pub uniform_buffer_bytes: usize,
    pub default_texture_bytes: usize,
    pub driver: DriverObjects,
}

/// What wgpu reports: a count for each kind of its own objects, which it gives no size, and what
/// its allocator holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DriverObjects {
    pub render_pipelines: usize,
    pub compute_pipelines: usize,
    pub shader_modules: usize,
    pub bind_groups: usize,
    pub bind_group_layouts: usize,
    pub pipeline_layouts: usize,
    pub samplers: usize,
    /// The bytes the backend's allocator handed out for buffers and textures and the memory it
    /// reserved for them, where the backend keeps a report.
    pub allocator: Option<(u64, u64)>,
}

impl DriverObjects {
    /// Each kind at the objects wgpu still holds for the app, and the allocator's two totals.
    pub fn of(
        report: &wgpu::wgc::global::GlobalReport,
        allocator: Option<&wgpu::AllocatorReport>,
    ) -> Self {
        let hub = &report.hub;
        let live = |kind: &wgpu::wgc::registry::RegistryReport| kind.num_kept_from_user;
        Self {
            render_pipelines: live(&hub.render_pipelines),
            compute_pipelines: live(&hub.compute_pipelines),
            shader_modules: live(&hub.shader_modules),
            bind_groups: live(&hub.bind_groups),
            bind_group_layouts: live(&hub.bind_group_layouts),
            pipeline_layouts: live(&hub.pipeline_layouts),
            samplers: live(&hub.samplers),
            allocator: allocator.map(|a| (a.total_allocated_bytes, a.total_reserved_bytes)),
        }
    }

    /// What `device`, made by `instance`, holds now; nothing without the instance.
    pub fn read(instance: Option<&wgpu::Instance>, device: &wgpu::Device) -> Self {
        let Some(report) = instance.and_then(wgpu::Instance::generate_report) else {
            return Self::default();
        };
        Self::of(&report, device.generate_allocator_report().as_ref())
    }
}

struct Entry<V> {
    value: V,
    bytes: usize,
    last_used: u64,
    stamp: TextureStamp,
}

/// ⚠ An evicted texture that a live mesh draw still binds stays on the GPU
/// until that draw is rebuilt; the count drops at eviction.
pub struct GpuTextures<V> {
    entries: HashMap<GpuTextureKey, Entry<V>>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl<V: Clone> GpuTextures<V> {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            tick: 0,
            bytes: 0,
            budget,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Evicts at once down to a lowered budget.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        self.evict_to_budget();
    }

    /// The upload under `key` if `stamp` made it; one made from other pixels is dropped with its bytes.
    pub fn get(&mut self, key: &GpuTextureKey, stamp: TextureStamp) -> Option<V> {
        self.tick += 1;
        if self.entries.get(key)?.stamp != stamp {
            self.remove(key);
            return None;
        }
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.tick;
        Some(entry.value.clone())
    }

    pub fn insert(&mut self, key: GpuTextureKey, value: V, bytes: usize, stamp: TextureStamp) {
        self.tick += 1;
        self.bytes += bytes;
        let entry = Entry {
            value,
            bytes,
            last_used: self.tick,
            stamp,
        };
        if let Some(old) = self.entries.insert(key, entry) {
            self.bytes -= old.bytes;
        }
        self.evict_to_budget();
    }

    /// Drop the upload under `key`, and its bytes with it.
    pub fn remove(&mut self, key: &GpuTextureKey) {
        if let Some(old) = self.entries.remove(key) {
            self.bytes -= old.bytes;
        }
    }

    pub fn retain(&mut self, keep: impl Fn(&GpuTextureKey) -> bool) {
        self.entries.retain(|key, _| keep(key));
        self.bytes = self.entries.values().map(|e| e.bytes).sum();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn evict_to_budget(&mut self) {
        while self.bytes > self.budget && self.entries.len() > 1 {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.entries.remove(&victim) {
                self.bytes -= e.bytes;
                tracing::trace!(texture = %victim.1, "Evicted viewport GPU texture");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXELS: TextureStamp = TextureStamp(1);

    fn key(name: &str) -> GpuTextureKey {
        (None, name.to_string())
    }

    #[test]
    fn uploads_past_the_budget_evict_the_least_recently_used() {
        let mut gpu = GpuTextures::new(250);
        gpu.insert(key("oldest"), (), 100, PIXELS);
        gpu.insert(key("middle"), (), 100, PIXELS);
        assert!(gpu.get(&key("oldest"), PIXELS).is_some());
        gpu.insert(key("newest"), (), 100, PIXELS);

        assert_eq!(gpu.bytes(), 200);
        assert!(
            gpu.get(&key("middle"), PIXELS).is_none(),
            "the untouched one goes"
        );
        assert!(gpu.get(&key("oldest"), PIXELS).is_some());
        assert!(gpu.get(&key("newest"), PIXELS).is_some());
    }

    #[test]
    fn a_lowered_budget_evicts_at_once_and_keeps_the_newest() {
        let mut gpu = GpuTextures::new(1000);
        gpu.insert(key("older"), (), 400, PIXELS);
        gpu.insert(key("newer"), (), 400, PIXELS);
        gpu.set_budget(10);
        assert_eq!(gpu.bytes(), 400, "one texture is kept rather than thrashed");
        assert!(gpu.get(&key("newer"), PIXELS).is_some());
    }

    #[test]
    fn retain_and_reinsert_keep_the_count_true() {
        let mut gpu = GpuTextures::new(1000);
        gpu.insert(key("active"), (), 100, PIXELS);
        gpu.insert((Some(7), "thumb".into()), (), 50, PIXELS);
        gpu.insert(key("active"), (), 30, PIXELS);
        assert_eq!(gpu.bytes(), 80);
        gpu.retain(|(entry, _)| entry.is_some());
        assert_eq!(gpu.bytes(), 50);
        gpu.clear();
        assert_eq!(gpu.bytes(), 0);
    }

    #[test]
    fn an_upload_of_other_pixels_is_dropped_with_its_bytes() {
        let mut gpu = GpuTextures::new(1000);
        gpu.insert(key("edited"), (), 100, PIXELS);
        gpu.insert(key("untouched"), (), 50, PIXELS);

        let edited = TextureStamp(2);
        assert!(
            gpu.get(&key("edited"), edited).is_none(),
            "the copy of the pixels before the edit is drawn"
        );
        assert_eq!(gpu.bytes(), 50, "the dropped copy is still counted");
        assert!(gpu.get(&key("untouched"), PIXELS).is_some());
        gpu.remove(&key("untouched"));
        assert_eq!(gpu.bytes(), 0, "a removed copy is still counted");
    }
}

#[cfg(test)]
mod driver_tests {
    use super::*;
    use wgpu::wgc::global::GlobalReport;
    use wgpu::wgc::hub::HubReport;
    use wgpu::wgc::registry::RegistryReport;

    /// A report whose every kind holds a different number of live objects, and `released` freed slots beside each.
    fn report(released: usize) -> GlobalReport {
        let kind = |live: usize| RegistryReport {
            num_kept_from_user: live,
            num_released_from_user: released,
            ..Default::default()
        };
        GlobalReport {
            surfaces: kind(0),
            hub: HubReport {
                adapters: kind(1),
                devices: kind(1),
                queues: kind(1),
                pipeline_layouts: kind(2),
                shader_modules: kind(3),
                bind_group_layouts: kind(4),
                bind_groups: kind(5),
                command_encoders: kind(6),
                command_buffers: kind(7),
                render_bundles: kind(8),
                render_pipelines: kind(9),
                compute_pipelines: kind(10),
                pipeline_caches: kind(11),
                query_sets: kind(12),
                buffers: kind(13),
                textures: kind(14),
                texture_views: kind(15),
                external_textures: kind(16),
                samplers: kind(17),
            },
        }
    }

    #[test]
    fn each_kind_reads_the_objects_wgpu_still_holds_for_it() {
        let driver = DriverObjects::of(&report(0), None);
        assert_eq!(
            driver,
            DriverObjects {
                render_pipelines: 9,
                compute_pipelines: 10,
                shader_modules: 3,
                bind_groups: 5,
                bind_group_layouts: 4,
                pipeline_layouts: 2,
                samplers: 17,
                allocator: None,
            }
        );
        assert_eq!(
            DriverObjects::of(&report(40), None),
            driver,
            "a slot wgpu freed is not an object"
        );
    }

    #[test]
    fn the_allocators_two_totals_read_as_used_and_reserved() {
        let allocator = wgpu::AllocatorReport {
            allocations: Vec::new(),
            blocks: Vec::new(),
            total_allocated_bytes: 3 << 20,
            total_reserved_bytes: 8 << 20,
        };
        let driver = DriverObjects::of(&report(0), Some(&allocator));
        assert_eq!(driver.allocator, Some((3 << 20, 8 << 20)));
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn the_driver_objects_grow_with_a_sampler_a_shader_and_a_pipeline() {
        const WHAT: &str = "the_driver_objects_grow_with_a_sampler_a_shader_and_a_pipeline";
        let Some((instance, device, _queue)) = crate::render::test_gpu(WHAT) else {
            return;
        };
        let read = || DriverObjects::read(Some(&instance), &device);
        let before = read();

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("driver-objects"),
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); } \
                 @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }"
                    .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("driver-objects"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let grown = read();
        assert_eq!(grown.samplers, before.samplers + 1);
        assert_eq!(grown.shader_modules, before.shader_modules + 1);
        assert_eq!(grown.render_pipelines, before.render_pipelines + 1);

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("driver-objects"),
            size: 4 << 20,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let (Some((used, _)), Some((now, reserved))) = (grown.allocator, read().allocator) {
            assert!(
                now >= used + (4 << 20),
                "a 4 MiB buffer: {used} -> {now} bytes in use"
            );
            assert!(reserved >= now);
        }
        drop(buffer);

        drop((sampler, module, pipeline));
        assert_eq!(
            read().samplers,
            before.samplers,
            "a dropped sampler leaves the count"
        );
        assert_eq!(read().render_pipelines, before.render_pipelines);
    }
}
