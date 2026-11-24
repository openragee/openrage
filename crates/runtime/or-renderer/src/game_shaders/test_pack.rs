//! A pack on disk as a bake would write it, made from WGSL, and a device to draw it on, for tests.

use std::collections::BTreeMap;

use or_io::gta5::shaders::spirv_pack::ShaderStageKind;
use or_io::gta5::shaders::spirv_pack::{
    module_path, Binding, BindingClass, ConstantBuffer, Effect, Fidelity, NagaReach, Outcome, Pass,
    Production, Program, Reach, Reflection, SignatureElement, SpirvPack, Technique, Variable,
    MANIFEST_FILE,
};

/// A WGSL module written out as the SPIR-V a pack holds.
///
/// The writer is told not to adjust the coordinate space: its default negates the clip-space y of a vertex stage for Vulkan, which a pack's modules, from a Direct3D translator and read as they are, do not.
pub fn spirv_of(source: &str) -> Vec<u8> {
    let module = naga::front::wgsl::parse_str(source).expect("the WGSL parses");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("the module validates");
    let options = naga::back::spv::Options {
        flags: naga::back::spv::WriterFlags::empty(),
        ..Default::default()
    };
    naga::back::spv::write_vec(&module, &info, &options, None)
        .expect("naga writes it")
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect()
}

pub fn binding(name: &str, kind: &str, class: BindingClass, set: u32, at: u32) -> Binding {
    Binding {
        name: name.into(),
        kind: kind.into(),
        class,
        register: at,
        set,
        binding: at,
    }
}

pub fn element(semantic: &str, index: u32, register: u32, mask: u8) -> SignatureElement {
    SignatureElement {
        semantic: semantic.into(),
        index,
        system_value: 0,
        component_type: 3,
        register,
        mask,
    }
}

/// A `float` variable of `size` bytes at `offset`.
pub fn variable(name: &str, offset: u32, size: u32) -> Variable {
    Variable {
        name: name.into(),
        offset,
        size,
        used: true,
        scalar: "float".into(),
        rows: 1,
        columns: 4,
        elements: 0,
    }
}

/// Builds a manifest and the modules to write beside it.
pub struct PackBuilder {
    manifest: SpirvPack,
    modules: Vec<(String, String)>,
}

impl PackBuilder {
    pub fn new() -> Self {
        Self {
            manifest: SpirvPack::new(Production {
                translator: "test".into(),
                legalizer: "test".into(),
            }),
            modules: Vec::new(),
        }
    }

    /// A constant buffer layout, by the key a program's reflection names it with.
    pub fn buffer(mut self, key: &str, name: &str, size: u32, variables: Vec<Variable>) -> Self {
        self.manifest.constant_buffers.insert(
            key.into(),
            ConstantBuffer {
                name: name.into(),
                size,
                variables,
            },
        );
        self
    }

    /// A translated program, whose module is the SPIR-V of `source`.
    pub fn program(
        mut self,
        key: &str,
        stage: ShaderStageKind,
        source: &str,
        reflection: Reflection,
    ) -> Self {
        self.manifest.programs.insert(
            key.into(),
            Program {
                stage,
                source_bytes: 1,
                outcome: Outcome::Translated {
                    spirv_bytes: 1,
                    fidelity: Fidelity::Equivalent,
                    lowerings: BTreeMap::new(),
                    naga: NagaReach {
                        reached: Reach::MslWritten,
                        cause: None,
                    },
                },
                reflection,
            },
        );
        self.modules.push((key.into(), source.into()));
        self
    }

    /// An effect with one technique of one pass running the programs given.
    pub fn effect(
        mut self,
        name: &str,
        technique: &str,
        stages: &[(ShaderStageKind, &str)],
    ) -> Self {
        self.manifest.effects.push(Effect {
            name: name.into(),
            source_path: format!("{name}.fxc"),
            vertex_type: 0,
            defaults: Default::default(),
            techniques: vec![Technique {
                name: technique.into(),
                passes: vec![Pass {
                    stages: stages
                        .iter()
                        .map(|(stage, key)| (*stage, key.to_string()))
                        .collect(),
                }],
            }],
        });
        self
    }

    /// The effect `effect` declares `words` as the default of the constant `variable`.
    pub fn default_of(mut self, effect: &str, variable: &str, words: &[u32]) -> Self {
        if let Some(effect) = self.manifest.effects.iter_mut().find(|e| e.name == effect) {
            effect.defaults.insert(variable.into(), words.to_vec());
        }
        self
    }

    pub fn write(self) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            serde_json::to_vec(&self.manifest).unwrap(),
        )
        .unwrap();
        for (key, source) in &self.modules {
            let path = dir.path().join(module_path(key));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, spirv_of(source)).unwrap();
        }
        dir
    }
}

/// The pack baked from an install, which `OPENRAGE_GAME_PACK` names, or a recorded skip: the game's own shaders are the only thing that can say what their passes do.
pub fn real_pack(test: &str) -> Option<super::pack::GamePack> {
    let Ok(root) = std::env::var("OPENRAGE_GAME_PACK") else {
        or_test_support::skip_install(test, format_args!("OPENRAGE_GAME_PACK names no pack"));
        return None;
    };
    Some(super::pack::GamePack::open(root).expect("the pack opens"))
}

/// The machine's device and queue, or a recorded skip.
pub fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("game-shaders-test"),
        required_limits: adapter.limits(),
        ..Default::default()
    })) {
        Ok(pair) => Some(pair),
        Err(e) => {
            or_test_support::skip_device(test, format_args!("device init failed: {e}"));
            None
        }
    }
}

/// What `record` draws into a fresh `size`-square colour target of `target`'s format, with a depth target cleared to one.
pub fn draw_into(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &super::pipeline::Target,
    size: u32,
    record: &mut dyn FnMut(&mut wgpu::RenderPass<'_>),
) -> super::frame::Rendered {
    let attachment = |format: wgpu::TextureFormat| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let colour = attachment(target.colour);
    let depth = attachment(target.depth);
    let colour_view = colour.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &colour_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        record(&mut pass);
    }
    queue.submit([encoder.finish()]);
    super::frame::read_back(device, queue, &colour, target.colour, size)
}
