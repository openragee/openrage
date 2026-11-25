//! A naga module built into a pipeline on a real device, which runs the platform's own shader compiler over what naga wrote.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::Arc;

use naga::{Binding, Interpolation, Sampling, ScalarKind, ShaderStage};
use or_renderer::game_shaders::pipeline::{create_render_pipeline, shape};

use crate::spirv_check::ModuleCompiler;

/// A device, and what it is.
pub struct DeviceCompiler {
    device: wgpu::Device,
    pub adapter: String,
}

/// The machine's best adapter with no surface, taking the most it allows, and the name of the adapter; wgpu's `WGPU_BACKEND` and `WGPU_VALIDATION` choose the backend and its validation layer.
pub fn open_device() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    descriptor.backends = or_renderer::instance::backends_from_env()?;
    let instance = wgpu::Instance::new(or_renderer::instance::with_dxc(descriptor));
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .map_err(|e| format!("no adapter: {e}"))?;
    let info = adapter.get_info();
    let mut features = wgpu::Features::empty();
    if adapter.features().contains(wgpu::Features::CLIP_DISTANCES) {
        features |= wgpu::Features::CLIP_DISTANCES;
    }
    // The renderer times the stages of a frame where the device can
    let timing = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if adapter.features().contains(timing) {
        features |= timing;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("or-shader-tool"),
        required_features: features,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|e| format!("no device: {e}"))?;
    device.on_uncaptured_error(Arc::new(|_| {}));
    Ok((device, queue, format!("{} ({:?})", info.name, info.backend)))
}

impl DeviceCompiler {
    /// Open the machine's best adapter with no surface, taking the most it allows.
    pub fn open() -> Result<Self, String> {
        let (device, _queue, adapter) = open_device()?;
        Ok(Self { device, adapter })
    }
}

fn wgsl_type(kind: ScalarKind, count: usize) -> Option<String> {
    let scalar = match kind {
        ScalarKind::Float => "f32",
        ScalarKind::Uint => "u32",
        ScalarKind::Sint => "i32",
        _ => return None,
    };
    Some(if count == 1 {
        scalar.to_string()
    } else {
        format!("vec{count}<{scalar}>")
    })
}

/// A vertex shader that passes zero for every input a fragment entry point takes, with matching interpolation.
pub(crate) fn matching_vertex_shader(
    module: &naga::Module,
    entry: &naga::EntryPoint,
) -> Result<String, String> {
    let mut members = String::new();
    for (index, argument) in entry.function.arguments.iter().enumerate() {
        let Some(Binding::Location {
            location,
            interpolation,
            sampling,
            ..
        }) = &argument.binding
        else {
            continue;
        };
        let (kind, count) =
            shape(module, argument.ty).ok_or("an input that is not a 32-bit scalar or vector")?;
        let ty = wgsl_type(kind, count).ok_or("an input of an unsupported kind")?;
        let mut attributes = format!("@location({location})");
        if let Some(interpolation) = interpolation {
            let kind = match interpolation {
                Interpolation::Perspective => "perspective",
                Interpolation::Linear => "linear",
                Interpolation::Flat => "flat",
                _ => "perspective",
            };
            let sampling = match sampling {
                Some(Sampling::Centroid) => ", centroid",
                Some(Sampling::Sample) => ", sample",
                Some(Sampling::Center) => ", center",
                _ => "",
            };
            let _ = write!(attributes, " @interpolate({kind}{sampling})");
        }
        let _ = writeln!(members, "    {attributes} v{index}: {ty},");
    }
    Ok(format!(
        "struct Out {{\n    @builtin(position) position: vec4<f32>,\n{members}}}\n\n@vertex\nfn main() -> Out {{\n    var out: Out;\n    out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);\n    return out;\n}}\n"
    ))
}

impl DeviceCompiler {
    /// `build` with the device's validation and internal errors caught and joined to one line.
    fn scoped(&self, build: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let built = build();
        let validation = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        built?;
        match validation.or(internal) {
            None => Ok(()),
            Some(wgpu::Error::Validation { description, .. })
            | Some(wgpu::Error::Internal { description, .. }) => Err(one_line(&description)),
            Some(error) => Err(error.to_string()),
        }
    }

    fn build(&self, module: &naga::Module) -> Result<(), String> {
        let entry = module.entry_points.first().ok_or("no entry point")?;
        match entry.stage {
            ShaderStage::Compute => {
                let shader = self
                    .device
                    .create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: None,
                        source: wgpu::ShaderSource::Naga(Cow::Owned(module.clone())),
                    });
                let _pipeline =
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: None,
                            layout: None,
                            module: &shader,
                            entry_point: Some(&entry.name),
                            compilation_options: Default::default(),
                            cache: None,
                        });
            }
            ShaderStage::Vertex => {
                create_render_pipeline(&self.device, module, None, None, false)
                    .map_err(|e| e.to_string())?;
            }
            ShaderStage::Fragment => {
                let source = matching_vertex_shader(module, entry)?;
                let vertex = naga::front::wgsl::parse_str(&source).map_err(|e| e.to_string())?;
                create_render_pipeline(&self.device, &vertex, Some(module), None, false)
                    .map_err(|e| e.to_string())?;
            }
            _ => return Err("a stage this check does not build".into()),
        }
        Ok(())
    }
}

/// A wgpu error's lines joined, without the pipeline's label and naga's source diagram.
pub(crate) fn one_line(description: &str) -> String {
    description
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "Validation Error" && *l != "Caused by:")
        .filter(|l| !l.starts_with("In Device::create_"))
        .filter(|l| !l.contains(['┌', '│', '└']))
        .map(|l| l.strip_prefix("= ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join(" | ")
}

impl ModuleCompiler for DeviceCompiler {
    fn compile(&self, module: &naga::Module) -> Result<(), String> {
        self.scoped(|| self.build(module))
    }

    fn compile_pipeline(
        &self,
        vertex: &naga::Module,
        fragment: Option<&naga::Module>,
    ) -> Result<(), String> {
        self.scoped(|| {
            create_render_pipeline(&self.device, vertex, fragment, None, false)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}

#[cfg(test)]
mod tests;
