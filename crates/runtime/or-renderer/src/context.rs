use crate::error::RenderError;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use tracing::{debug, info};
use wgpu::{Device, Queue, Surface, SurfaceConfiguration};
pub const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.176,
    g: 0.176,
    b: 0.188,
    a: 1.0,
};

pub struct RenderContext<'window> {
    pub device: Device,
    pub queue: Queue,
    pub surface: Surface<'window>,
    pub config: SurfaceConfiguration,
}

impl<'window> RenderContext<'window> {
    pub async fn new<W>(window: W, width: u32, height: u32) -> Result<Self, RenderError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'window,
    {
        info!("Initializing wgpu render context");

        let instance = wgpu::Instance::new(crate::instance::with_dxc(wgpu::InstanceDescriptor {
            backends: crate::instance::BACKENDS,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        }));

        debug!("Created wgpu instance with PRIMARY backends");

        let surface = instance.create_surface(window)?;

        debug!("Created wgpu surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| RenderError::AdapterNotFound(e.to_string()))?;

        info!(
            "Selected adapter: {} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("or-renderer"),
                required_features: wgpu::Features::empty(),
                required_limits: {
                    let mut l = wgpu::Limits::default();
                    l.max_bind_groups = l.max_bind_groups.max(5);
                    l
                },
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await?;

        debug!("Requested wgpu device and queue");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        debug!("Selected surface format: {:?}", surface_format);

        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        info!(
            "wgpu render context initialized ({}x{}, {:?})",
            width, height, config.present_mode
        );

        Ok(Self {
            device,
            queue,
            surface,
            config,
        })
    }

    pub fn width(&self) -> u32 {
        self.config.width
    }

    pub fn height(&self) -> u32 {
        self.config.height
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}
