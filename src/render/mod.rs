pub mod bound_meshes;
pub mod debug_channel;
pub mod dressed;
pub mod game_scene;
pub mod gpu_budget;
pub mod gta6_cloth;
pub mod gta6_drawable;
pub mod gta6_glass_panes;
pub(crate) mod gta6_layers;
pub(crate) mod gta6_meshlets;
pub(crate) mod gta6_shaded;
pub mod heel_floor;
pub mod lines;
pub mod map_overlay;
pub mod meshes;
pub mod navmesh;
pub mod particle_air;
pub mod particle_gizmos;
pub mod particle_models;
pub mod particles;
pub mod plugin_overlay;
pub mod read_back;
pub mod skeleton_lines;
pub mod thumbnails;
pub mod trails;
pub mod viewport;

pub use viewport::RenderBridge;

/// A device with no surface for a test, or `None` after recording the skip.
#[cfg(test)]
pub(crate) fn test_device(what: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    test_gpu(what).map(|(_, device, queue)| (device, queue))
}

/// [`test_device`] with the instance that made it, which wgpu counts its objects through.
#[cfg(test)]
pub(crate) fn test_gpu(what: &str) -> Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(or_renderer::instance::with_dxc(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    }));
    let adapter =
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        {
            Ok(a) => a,
            Err(e) => {
                or_test_support::skip_device(what, format_args!("no wgpu adapter: {e}"));
                return None;
            }
        };
    let mut required_limits = wgpu::Limits::default();
    required_limits.max_bind_groups = required_limits.max_bind_groups.max(5);
    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("openrage-test"),
        required_limits,
        ..Default::default()
    })) {
        Ok((device, queue)) => Some((instance, device, queue)),
        Err(e) => {
            or_test_support::skip_device(what, format_args!("device init failed: {e}"));
            None
        }
    }
}
