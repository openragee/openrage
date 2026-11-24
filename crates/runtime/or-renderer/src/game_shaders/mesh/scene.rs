//! Surfaces made ready to draw: pipelines built, resources bound and vertices uploaded once, so that a frame costs a write of the constants and a draw into a pass its caller owns.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use or_io::gta5::shaders::spirv_pack::{ConstantBuffer, Program};
use wgpu::util::DeviceExt;

use super::super::bindings::{Requirement, Resource};
use super::super::constants::Uniform;
use super::super::deferred::GBUFFER_TARGETS;
use super::super::engine::EngineConstants;
use super::super::link::match_interpolation;
use super::super::pack::{GamePack, PackError};
use super::super::pipeline::{
    build_pass_pipeline_drawn, entry_of, vertex_layout, DrawState, PassPipeline, PipelineError,
    Target,
};
use super::super::quad::pair_of;
use super::super::samplers::point;
use super::super::stand_in::{
    bind_resources_with, declared_sampler, uniform_buffer, uploaded_view, Provided,
};
use super::{
    bone_palette, is_named, is_palette, missing, sees_through, select_pass_at, surface_vertices,
    Coverage, Image, Material, MeshError, Route, Surface, SHADOW_EFFECT,
};

/// The stencil value a surface of the default material leaves in the G-buffer, which the lighting passes tell from what nothing drew.
pub const MATERIAL_DEFAULT: u32 = 0;

/// What the stencil of the G-buffer is cleared to, so that a pixel no surface wrote is left unlit.
pub const MATERIAL_CLEAR: u32 = 7;

/// A pass that writes the material of the surface it draws into the stencil wherever it draws.
pub(crate) fn writes_material() -> wgpu::StencilState {
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::Always,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op: wgpu::StencilOperation::Replace,
    };
    wgpu::StencilState {
        front: face,
        back: face,
        read_mask: 0xff,
        write_mask: 0xff,
    }
}

/// A pass that draws only where nothing was drawn: the stencil still holds the value it was cleared to there, and the pass leaves it be.
fn where_nothing_was_drawn() -> wgpu::StencilState {
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::Equal,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op: wgpu::StencilOperation::Keep,
    };
    wgpu::StencilState {
        front: face,
        back: face,
        read_mask: 0xff,
        write_mask: 0,
    }
}

/// How the disc of the sun takes the sky: by the alpha the sky left, which is how much of it no cloud covers, and leaving that alpha be.
const SUN_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::DstAlpha,
        dst_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// How the disc of the moon is added to the sky: by the alpha the sky left, which is how much of it no cloud covers, and leaving that alpha be.
const MOON_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::DstAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: SUN_BLEND.alpha,
};

struct Prepared {
    pass: Arc<PassPipeline>,
    groups: BTreeMap<u32, wgpu::BindGroup>,
    vertices: wgpu::Buffer,
    count: u32,
    /// Whether it blends over what is behind it, and so draws after what is opaque.
    blended: bool,
}

/// What a buffer holds before the engine's values: the effect's defaults as words, and the material's vectors over them.
type Constants = (Vec<(String, Vec<u32>)>, Vec<(String, Vec<f32>)>);

/// A scene's constant buffers, one for each set of bytes they hold before the engine's values: the buffer of a layout, its size, the effect's defaults and the material's vectors.
type Shared = HashMap<
    (usize, u64, Vec<(String, Vec<u32>)>, Vec<(String, Vec<u32>)>),
    (wgpu::Buffer, Arc<Uniform>),
>;

/// What the surfaces of one scene have in common.
struct Common<'c> {
    buffers: Shared,
    textures: &'c mut TextureCache,
}

/// A texture made, with the texels it was made from and whether the scene being made names it.
struct Made {
    texels: Vec<u8>,
    view: wgpu::TextureView,
    used: bool,
}

/// The textures made for the scenes of one device, by a sample of their texels and their size: a texture that several surfaces name, or that a scene made again after an edit names, is uploaded once.
#[derive(Default)]
struct TextureCache {
    made: HashMap<(u32, u32, bool, u64), Vec<Made>>,
}

impl TextureCache {
    /// The view of `image`, made at its first ask.
    fn view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &Image,
    ) -> wgpu::TextureView {
        let sample = image.rgba.iter().step_by(61).fold(0u64, |sum, byte| {
            sum.wrapping_mul(31).wrapping_add(u64::from(*byte))
        });
        let same = self
            .made
            .entry((image.width, image.height, image.srgb, sample))
            .or_default();
        if let Some(made) = same.iter_mut().find(|m| m.texels == image.rgba) {
            made.used = true;
            return made.view.clone();
        }
        let view = uploaded_view(
            device,
            queue,
            image.width,
            image.height,
            &image.rgba,
            image.srgb,
        );
        same.push(Made {
            texels: image.rgba.clone(),
            view: view.clone(),
            used: true,
        });
        view
    }

    /// Start a scene, which has named nothing yet.
    fn begin(&mut self) {
        for made in self.made.values_mut().flatten() {
            made.used = false;
        }
    }

    /// End a scene: what it did not name is let go.
    fn end(&mut self) {
        self.made.retain(|_, same| {
            same.retain(|made| made.used);
            !same.is_empty()
        });
    }
}

/// A pass built, with the one vertex buffer its vertex stage reads from: the layout of its attributes and its stride.
#[derive(Clone)]
struct Built {
    pass: Arc<PassPipeline>,
    attributes: Vec<wgpu::VertexAttribute>,
    stride: u64,
}

/// The pipeline of a pass is built once for each pair of programs and whether it blends, and a pair that will not build is not tried again.
type Pipelines = HashMap<(String, Option<String>, bool, bool), Result<Built, MeshError>>;

/// The pipelines built for one pack and one target, kept from a scene to the next so that a scene made again, after an edit, builds none it has built.
///
/// ⚠ Only for the pack, the target and the device it was filled for: make a new one when any of them changes.
#[derive(Default)]
pub struct PipelineCache {
    built: Pipelines,
    textures: TextureCache,
}

impl PipelineCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many pairs of programs have been tried, built or refused.
    pub fn len(&self) -> usize {
        self.built.len()
    }

    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }

    /// Let go of the textures kept for the last scene.
    pub fn forget_textures(&mut self) {
        self.textures = TextureCache::default();
    }
}

/// Surfaces ready to draw with the passes of a pack.
pub struct MeshScene {
    surfaces: Vec<Prepared>,
    uniforms: Vec<Arc<Uniform>>,
    skipped: Vec<(String, MeshError)>,
    route: Route,
}

impl MeshScene {
    /// Prepare each surface to draw into `target`, a pass of one colour attachment; one the pack cannot draw, or whose pass will not build, is left out and listed.
    pub fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        target: &Target,
        surfaces: &[(Surface, Material)],
    ) -> Self {
        Self::prepare_with(
            &mut PipelineCache::new(),
            device,
            queue,
            pack,
            target,
            surfaces,
        )
    }

    /// [`Self::prepare`], taking the pipelines `cache` holds and leaving it the ones it builds.
    pub fn prepare_with(
        cache: &mut PipelineCache,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        target: &Target,
        surfaces: &[(Surface, Material)],
    ) -> Self {
        Self::prepare_routed(cache, device, queue, pack, target, surfaces, Route::Forward)
    }

    /// [`Self::prepare_with`] for the surfaces `route` draws, with the passes it draws them with.
    ///
    /// The G-buffer route draws into a pass of as many colour attachments as the game's passes write, and the others into one.
    pub fn prepare_routed(
        cache: &mut PipelineCache,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        target: &Target,
        surfaces: &[(Surface, Material)],
        route: Route,
    ) -> Self {
        let target = &Target {
            colour_attachments: Some(match route {
                Route::GBuffer => GBUFFER_TARGETS,
                // The fog pass writes the colour and what the water reads of it, of which the second goes nowhere
                Route::WaterFog => 2,
                _ => 1,
            }),
            ..*target
        };
        let pipelines = &mut cache.built;
        cache.textures.begin();
        // What blends over what is opaque shows the opaque through it; over nothing it is the whole picture, and marks the pixel as drawn, which leaves a clear glass black where the background should show through
        let all_blend = surfaces
            .iter()
            .all(|(_, m)| m.coverage == Coverage::Blended);
        let mut prepared = Vec::new();
        let mut common = Common {
            buffers: Shared::new(),
            textures: &mut cache.textures,
        };
        let mut skipped = Vec::new();
        // The bones the skinned surfaces are weighted to, whose matrices the pose is written to in this order
        let palette = bone_palette(surfaces);
        let takes = |m: &Material| {
            route.takes(m.coverage)
                && (route != Route::WaterFog || m.effect.to_ascii_lowercase().starts_with("water"))
        };
        for item in surfaces.iter().filter(|(_, m)| takes(m)) {
            // What is seen through leaves the pixel to what is behind it; any other blended surface (a decal, a ghost of glowing parts) is what it shows, and marks it
            let marks = route != Route::Blended || all_blend || !sees_through(&item.1.effect);
            match prepare_surface(
                device,
                queue,
                pack,
                target,
                (pipelines, &mut common),
                item,
                route,
                marks,
                &palette,
            ) {
                Ok(one) => prepared.push(one),
                Err(error) => skipped.push((item.1.effect.clone(), error)),
            }
        }
        let uniforms = common
            .buffers
            .into_values()
            .map(|(_, uniform)| uniform)
            .collect();
        cache.textures.end();
        Self {
            surfaces: prepared,
            uniforms,
            skipped,
            route,
        }
    }

    /// How many surfaces will be drawn.
    pub fn drawn(&self) -> usize {
        self.surfaces.len()
    }

    /// The effect of each surface left out, and why.
    pub fn skipped(&self) -> &[(String, MeshError)] {
        &self.skipped
    }

    /// How many constant buffers the surfaces share out between them.
    #[cfg(test)]
    pub(super) fn buffers(&self) -> usize {
        self.uniforms.len()
    }

    /// Write the constants of this frame: what each effect declares, each material's over it, then the scene's over those.
    pub fn update(&self, queue: &wgpu::Queue, scene: &EngineConstants) {
        self.update_with(queue, &scene.values());
    }

    /// [`Self::update`] with the values of the scene already worked out, and added to, by the caller.
    pub fn update_with(&self, queue: &wgpu::Queue, values: &BTreeMap<&'static str, Vec<f32>>) {
        for uniform in &self.uniforms {
            uniform.write(queue, values);
        }
    }

    /// Draw every surface into `pass`, which must have the colour and depth formats the scene was prepared for: what is opaque first, then what blends over it.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        match self.route {
            Route::GBuffer => pass.set_stencil_reference(MATERIAL_DEFAULT),
            Route::Sky | Route::Sun | Route::Moon => pass.set_stencil_reference(MATERIAL_CLEAR),
            _ => {}
        }
        let opaque = self.surfaces.iter().filter(|s| !s.blended);
        let blended = self.surfaces.iter().filter(|s| s.blended);
        for surface in opaque.chain(blended) {
            pass.set_pipeline(&surface.pass.pipeline);
            pass.set_vertex_buffer(0, surface.vertices.slice(..));
            for (group, bind_group) in &surface.groups {
                pass.set_bind_group(*group, bind_group, &[]);
            }
            pass.draw(0..surface.count, 0..1);
        }
    }
}

/// The pairs of state and value the effect `effect` declares for its sampler `sampler`, when the install's effects are read and it states any.
fn declared_states(effect: &str, sampler: &str) -> Option<Vec<u32>> {
    use or_io::gta5::shaders::shader_defs::{describe, ShaderParamValue};
    let def = describe(
        or_io::hash::string_hash_str(effect),
        or_io::hash::string_hash_str(sampler),
    )?;
    match def.default? {
        ShaderParamValue::Uints(states) if !states.is_empty() => Some(states),
        _ => None,
    }
}

fn prepare_surface(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pack: &GamePack,
    target: &Target,
    (pipelines, common): (&mut Pipelines, &mut Common),
    (surface, material): &(Surface, Material),
    route: Route,
    marks: bool,
    palette: &[u16],
) -> Result<Prepared, MeshError> {
    let pack_error = |e: PackError| MeshError::Pack(e.to_string());
    let pipeline_error = |e: PipelineError| MeshError::Pipeline(e.to_string());
    let manifest = pack.manifest();
    // A shadow is cast by the depth-only passes of one effect, which draw a surface by its positions alone
    let effect_name = if route == Route::Shadow {
        SHADOW_EFFECT
    } else {
        &material.effect
    };
    let effect = manifest
        .effects
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(effect_name))
        .ok_or_else(|| MeshError::NoEffect(material.effect.clone()))?;
    let pass = select_pass_at(
        effect,
        route,
        surface.skinned,
        material.coverage,
        material.pass,
    )
    .ok_or_else(|| MeshError::NoPass(material.effect.clone()))?;
    let (vertex_key, pixel_key) =
        pair_of(pass).ok_or_else(|| MeshError::NoPass(material.effect.clone()))?;
    let blended = material.coverage == Coverage::Blended;
    let key = (vertex_key.clone(), pixel_key.cloned(), blended, marks);
    if !pipelines.contains_key(&key) {
        let vertex_module = pack.module(vertex_key).map_err(pack_error)?;
        let mut vertex = vertex_module.clone();
        let pixel = pixel_key
            .map(|k| pack.module(k))
            .transpose()
            .map_err(pack_error)?;
        if let Some(pixel) = &pixel {
            match_interpolation(&mut vertex, pixel);
        }
        let target = Target {
            blend: match route {
                Route::Sun => Some(SUN_BLEND),
                // The colour is scaled by the constant the frame sets, which is the light the water takes
                Route::WaterFog => Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Constant,
                        dst_factor: wgpu::BlendFactor::Zero,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                Route::Moon => Some(MOON_BLEND),
                // Water writes its whole colour and a nought alpha, over a copy of what lies behind it: it replaces what it covers
                _ if effect_name.to_ascii_lowercase().starts_with("water") => {
                    Some(wgpu::BlendState {
                        color: wgpu::BlendComponent::REPLACE,
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    })
                }
                _ => blended.then_some(wgpu::BlendState::ALPHA_BLENDING),
            },
            ..*target
        };
        let draw = DrawState {
            stencil: match route {
                Route::GBuffer => writes_material(),
                Route::Blended if marks => writes_material(),
                Route::Blended => wgpu::StencilState::default(),
                Route::Sky | Route::Sun | Route::Moon => where_nothing_was_drawn(),
                Route::Forward | Route::Shadow | Route::WaterFog => wgpu::StencilState::default(),
            },
            depth_write: matches!(route, Route::Sky | Route::Sun | Route::Moon).then_some(false),
            // ⚠ The game culls a ped's back faces, and its clothes are often two-sided by a back-to-back copy of every face: drawn without culling the two coincide and z-fight
            // (hair cards, fur and decals are left two-sided: they are drawn as thin sheets)
            cull_mode: (route != Route::Shadow && {
                let effect = material.effect.to_ascii_lowercase();
                effect.starts_with("ped")
                    && !["hair", "fur", "decal"]
                        .iter()
                        .any(|thin| effect.contains(thin))
            })
            .then_some(wgpu::Face::Back),
            // A surface that casts a shadow is held back by how steeply it meets the light, so that it does not shade itself
            depth_bias: if route == Route::Shadow {
                wgpu::DepthBiasState {
                    constant: 4,
                    slope_scale: 2.5,
                    clamp: 0.0,
                }
            } else {
                wgpu::DepthBiasState::default()
            },
            ..DrawState::default()
        };
        let built =
            build_pass_pipeline_drawn(device, &vertex, pixel.as_ref(), true, &target, &draw)
                .map_err(pipeline_error)
                .and_then(|pass| {
                    let entry = entry_of(&vertex_module, naga::ShaderStage::Vertex)
                        .map_err(pipeline_error)?;
                    let (attributes, stride) =
                        vertex_layout(&vertex_module, entry).map_err(pipeline_error)?;
                    Ok(Built {
                        pass: Arc::new(pass),
                        attributes,
                        stride,
                    })
                });
        pipelines.insert(key.clone(), built);
    }
    let Built {
        pass: built,
        attributes,
        stride,
    } = pipelines[&key].clone()?;
    let programs: Vec<&Program> = [Some(vertex_key), pixel_key]
        .into_iter()
        .flatten()
        .filter_map(|k| manifest.programs.get(k))
        .collect();

    // What each binding is called, and the layout of the buffer of that name
    let mut names: BTreeMap<(u32, u32), &str> = BTreeMap::new();
    let mut layouts: BTreeMap<&str, &ConstantBuffer> = BTreeMap::new();
    for program in &programs {
        for binding in &program.reflection.bindings {
            names.insert((binding.set, binding.binding), &binding.name);
        }
        for layout in manifest.constant_buffers_of(program) {
            layouts.insert(&layout.name, layout);
        }
    }
    // What a buffer holds before the engine's values: the effect's defaults, and the material's vectors over them
    let constants_of = |layout: &ConstantBuffer| -> Constants {
        let mut set = Vec::new();
        for (parameter, rows) in &material.vectors {
            let Some(variable) = layout
                .variables
                .iter()
                .find(|v| is_named(parameter, &v.name))
            else {
                continue;
            };
            let flat: Vec<f32> = rows.iter().flatten().copied().collect();
            // A row of four a register, cut to the variable where it is shorter
            let room = (variable.size / 4) as usize;
            set.push((variable.name.clone(), flat[..flat.len().min(room)].to_vec()));
        }
        let defaults = layout
            .variables
            .iter()
            .filter_map(|variable| {
                let words = effect.defaults.get(&variable.name)?;
                let room = (variable.size / 4) as usize;
                Some((
                    variable.name.clone(),
                    words[..words.len().min(room)].to_vec(),
                ))
            })
            .collect();
        (defaults, set)
    };
    let mut provide = |requirement: &Requirement| -> Option<Provided> {
        let name = names.get(&(requirement.group, requirement.binding))?;
        match &requirement.resource {
            Resource::Texture {
                dimension,
                sample,
                multisampled,
            } => {
                if let Some((_, view)) = material
                    .views
                    .iter()
                    .find(|(parameter, _)| is_named(parameter, name))
                {
                    return Some(Provided::View(view.clone()));
                }
                let found = material
                    .textures
                    .iter()
                    .find(|(parameter, _)| is_named(parameter, name));
                let plain_2d = *dimension == wgpu::TextureViewDimension::D2
                    && matches!(sample, wgpu::TextureSampleType::Float { .. })
                    && !multisampled;
                Some(match found {
                    None => missing(name),
                    // A texture the shader reads as a plane is uploaded once for every surface that names it
                    Some((_, image)) if plain_2d => {
                        Provided::View(common.textures.view(device, queue, image))
                    }
                    Some((_, image)) => Provided::Texture2d {
                        width: image.width,
                        height: image.height,
                        rgba: image.rgba.clone(),
                        srgb: image.srgb,
                    },
                })
            }
            // The effect's own states for the texture, where its file is read; else, a palette is read a row at a time and filtered its rows would run into each other
            Resource::Sampler { comparison: false } => {
                match declared_states(&material.effect, name) {
                    Some(states) => Some(Provided::Sampler(
                        device.create_sampler(&declared_sampler(&states)),
                    )),
                    None if is_palette(name) => Some(Provided::Sampler(point(device))),
                    None => None,
                }
            }
            // A buffer that comes to the same bytes as another surface's is that surface's, written once a frame
            Resource::Uniform { size } => {
                let layout = *layouts.get(name)?;
                let (defaults, set) = constants_of(layout);
                let key = (
                    std::ptr::from_ref(layout) as usize,
                    *size,
                    defaults.clone(),
                    set.iter()
                        .map(|(variable, numbers)| {
                            (
                                variable.clone(),
                                numbers.iter().map(|n| n.to_bits()).collect(),
                            )
                        })
                        .collect(),
                );
                let (buffer, _) = common.buffers.entry(key).or_insert_with(|| {
                    let buffer = uniform_buffer(device, *size);
                    let uniform = Uniform::new(buffer.clone(), layout.clone(), defaults, set);
                    (buffer, Arc::new(uniform))
                });
                Some(Provided::Buffer(buffer.clone()))
            }
            _ => None,
        }
    };
    let bound = bind_resources_with(device, queue, &built, &mut provide);

    let inputs = manifest
        .programs
        .get(vertex_key)
        .map(|p| p.reflection.inputs.as_slice())
        .unwrap_or_default();
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &surface_vertices(surface, &attributes, stride, inputs, palette),
        usage: wgpu::BufferUsages::VERTEX,
    });
    Ok(Prepared {
        pass: built,
        groups: bound.groups,
        vertices,
        count: u32::try_from(surface.indices.len()).unwrap_or(u32::MAX),
        blended,
    })
}

#[cfg(test)]
mod tests;
