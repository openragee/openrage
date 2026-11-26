//! The game's deferred frame: opaque surfaces written to a G-buffer by the game's own passes, then lit by its lighting pass.

use std::collections::BTreeMap;
use std::ops::Range;

use glam::{Mat4, Vec3};

use super::ambient::AmbientPass;
use super::antialias::AntiAliasPass;
use super::bindings::Requirement;
use super::bloom::BloomPass;
use super::capsule_lights::{CapsuleLight, CapsuleLights};
use super::coronas::{Corona, CoronaLook, Coronas};
use super::distant_lights::{DistantLight, DistantLights, SpriteLook};
use super::dome::SkyPass;
use super::engine::EngineConstants;
use super::fog_volumes::{FogSky, FogVolume, FogVolumes};
use super::mesh::scene::{MATERIAL_CLEAR, MATERIAL_TREE, MATERIAL_TYPE_MASK};
use super::mesh::{bounds, Material, MeshScene, PipelineCache, Route, Surface};
use super::pack::GamePack;
use super::pipeline::{DrawState, Target};
use super::point_lights::{PointLight, PointLights};
use super::resolve::ResolvePass;
use super::room_mask::{LightRoom, RoomMask, MOST_BOXES, ROOM_BIT};
use super::samplers::{point, smooth};
use super::screen::{ScreenError, ScreenPass, Space};
use super::shadow::{sphere_of, Fit, ShadowPass};
use super::spot_lights::{SpotLight, SpotLights};
use super::stand_in::Provided;
use super::targets::render_target;
use super::tone::Tone;

/// The format of each colour target the G-buffer passes write.
pub const GBUFFER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The format of the depth and stencil target the G-buffer is drawn with: the stencil tells a lit surface from what nothing drew.
pub const DEPTH_STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// The format of the target the light is summed in before it is mapped to the screen.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The target the light is summed in, for one size of frame.
pub fn hdr_target(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    render_target(
        device,
        "hdr",
        (width, height),
        HDR_FORMAT,
        // Water reads a copy of it, and GTA VI's passes after the light write theirs back into it
        wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
    )
}

/// The target the frame is mapped to before it is anti-aliased onto the screen, for one size of frame and the format of the screen.
pub fn ldr_target(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> wgpu::TextureView {
    render_target(
        device,
        "mapped-frame",
        (width, height),
        format,
        wgpu::TextureUsages::empty(),
    )
}

/// The colour targets a G-buffer is made of.
pub const GBUFFER_TARGETS: usize = 4;

/// The pass of the directional light's technique that lights a surface by the sun, and the one that lights it by the ambient light alone.
const DIRECTIONAL_STANDARD: usize = 0;
const DIRECTIONAL_AMBIENT: usize = 1;
/// The pass that lights a tree by the sun on either side, the side turned away through its leaves.
const DIRECTIONAL_BACKLIT: usize = 5;

/// How many registers the light parameters of a lighting pass are.
const LIGHT_PARAMS: usize = 14;

/// The target the passes that fill the G-buffer draw into, with the depth of the frame.
pub fn gbuffer_target() -> Target {
    Target {
        colour: GBUFFER_FORMAT,
        depth: DEPTH_STENCIL_FORMAT,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: None,
    }
}

/// The colour targets the G-buffer passes write, for one size of frame.
pub struct GBuffer {
    width: u32,
    height: u32,
    views: [wgpu::TextureView; GBUFFER_TARGETS],
}

impl GBuffer {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        // A target can be copied out, to read what the passes wrote
        let make = |_: usize| {
            render_target(
                device,
                "gbuffer",
                (width, height),
                GBUFFER_FORMAT,
                wgpu::TextureUsages::COPY_SRC,
            )
        };
        Self {
            width,
            height,
            views: std::array::from_fn(make),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The textures of the targets, for the bytes they hold.
    pub fn textures(&self) -> impl Iterator<Item = &wgpu::Texture> {
        self.views.iter().map(wgpu::TextureView::texture)
    }

    /// The target that holds the scales of the ambient light a surface takes, in its red and green.
    pub fn ambient_scales(&self) -> &wgpu::TextureView {
        &self.views[3]
    }

    /// The target that holds how much of the sun a surface keeps, in its alpha.
    pub fn self_shadows(&self) -> &wgpu::TextureView {
        &self.views[2]
    }

    /// The colour attachments of a pass that fills the G-buffer, each cleared to nothing.
    pub fn attachments(&self) -> [Option<wgpu::RenderPassColorAttachment<'_>>; GBUFFER_TARGETS] {
        std::array::from_fn(|i| {
            Some(wgpu::RenderPassColorAttachment {
                view: &self.views[i],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        })
    }
}

/// The texture a map of the sky is kept in, which is written again when the sky changes.
pub struct ReflectionTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    levels: u32,
}

impl ReflectionTarget {
    /// A texture for a map `width` texels across and half as many down, of `levels` levels.
    pub fn new(device: &wgpu::Device, width: u32, levels: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection"),
            size: wgpu::Extent3d {
                width,
                height: (width / 2).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            levels,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn levels(&self) -> u32 {
        self.levels
    }

    /// Write `map`, which must have the size and the levels this was made for.
    pub fn write(&self, queue: &wgpu::Queue, map: &super::sky::ReflectionMap) {
        for (level, texels) in map.levels.iter().enumerate().take(self.levels as usize) {
            let (width, height) = map.size_of(level);
            let bytes: Vec<u8> = texels
                .iter()
                .flat_map(|t| {
                    t.iter()
                        .flat_map(|v| or_io::half::f32_to_half(*v).to_le_bytes())
                })
                .collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 8),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

/// The depth and the stencil of a depth-stencil texture, each as a texture of its own, which is how the lighting passes read them.
pub struct DepthViews {
    pub depth: wgpu::TextureView,
    pub stencil: wgpu::TextureView,
}

impl DepthViews {
    pub fn of(texture: &wgpu::Texture) -> Self {
        let aspect = |aspect| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                aspect,
                ..Default::default()
            })
        };
        Self {
            depth: aspect(wgpu::TextureAspect::DepthOnly),
            stencil: aspect(wgpu::TextureAspect::StencilOnly),
        }
    }
}

/// The stencil value a pixel keeps where no surface was drawn, and the load of the stencil of a pass that fills the G-buffer.
pub fn stencil_clear() -> wgpu::Operations<u32> {
    wgpu::Operations {
        load: wgpu::LoadOp::Clear(MATERIAL_CLEAR),
        store: wgpu::StoreOp::Store,
    }
}

/// The light a lighting pass applies: the way it travels and its colour, the intensity in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sun {
    pub travel: Vec3,
    pub colour: Vec3,
}

impl Sun {
    pub fn of(engine: &EngineConstants) -> Self {
        match &engine.lighting {
            Some(lighting) => Self {
                travel: lighting.directional_light.truncate(),
                colour: lighting.directional_colour.truncate(),
            },
            None => Self {
                travel: -engine.sun_direction,
                colour: engine.sun_colour,
            },
        }
    }

    /// Whether it gives light: a pass with no sun is lit by the ambient light alone.
    pub fn shines(&self) -> bool {
        self.colour.max_element() > 0.0
    }
}

/// How the frame is cut: its size in pixels and the planes of its projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub near: f32,
    pub far: f32,
    /// How many levels the map of the sky has, which the lighting passes pick a level of by how glossy a surface is.
    pub reflection_levels: u32,
}

/// The values of the variables the lighting passes read of the frame and of the light, by the names the game gives them.
pub fn lighting_values(
    engine: &EngineConstants,
    screen: &Screen,
) -> BTreeMap<&'static str, Vec<f32>> {
    let (w, h) = (screen.width.max(1) as f32, screen.height.max(1) as f32);
    let size = vec![w, h, 1.0 / w, 1.0 / h];
    // How far the edge of the picture is from its middle at one unit ahead
    let tan_h = 1.0 / engine.projection.x_axis.x;
    let tan_v = 1.0 / engine.projection.y_axis.y;
    let q = screen.far / (screen.far - screen.near);
    // A depth is turned back into a distance by these two
    let projection = vec![tan_h, tan_v, -screen.near * q, -q];
    // The ray through a pixel is the camera's three axes mixed by its place on the screen
    let camera = engine.view.inverse();
    let (right, up, back) = (
        camera.x_axis.truncate(),
        camera.y_axis.truncate(),
        camera.z_axis.truncate(),
    );
    let shear = |axis: usize| vec![tan_h * right[axis], tan_v * up[axis], -back[axis]];

    let sun = Sun::of(engine);
    let mut light = vec![0.0f32; LIGHT_PARAMS * 4];
    light[4..7].copy_from_slice(&sun.travel.to_array());
    light[12..15].copy_from_slice(&sun.colour.to_array());

    BTreeMap::from([
        ("globalScreenSize", size.clone()),
        ("deferredLightScreenSize", size),
        ("deferredProjectionParams", projection),
        ("deferredPerspectiveShearParams0", shear(0)),
        ("deferredPerspectiveShearParams1", shear(1)),
        ("deferredPerspectiveShearParams2", shear(2)),
        ("deferredLightParams", light),
        ("gReflectionMipCount", vec![screen.reflection_levels as f32]),
    ])
}

/// A single white texel: what a pass that reads the occlusion of the screen sees where nothing has computed it.
fn white() -> Provided {
    Provided::Texture2d {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    }
}

/// What the passes of the frame read and write: the G-buffer, the depth and the stencil, and the target the light is summed in.
pub struct Inputs<'a> {
    pub gbuffer: &'a GBuffer,
    pub depth: &'a DepthViews,
    /// The target the lighting passes write and the resolve reads.
    pub hdr: &'a wgpu::TextureView,
    /// The map of the sky that surfaces reflect.
    pub reflection: &'a wgpu::TextureView,
    /// The target the resolve writes and the anti-aliasing reads, of the format of the screen.
    pub ldr: &'a wgpu::TextureView,
}

/// What the lighting pass binds under `name`: the target it reads, the sampler that goes with it, or nothing.
pub(super) fn provide(
    device: &wgpu::Device,
    inputs: &Inputs<'_>,
    name: &str,
    requirement: &Requirement,
) -> Option<Provided> {
    use super::bindings::Resource;
    if matches!(requirement.resource, Resource::Sampler { .. }) {
        return match name {
            "ReflectionSampler" => Some(Provided::Sampler(smooth(device))),
            "fogVolumeDepthSampler" => Some(Provided::Sampler(point(device))),
            _ => name
                .starts_with("GBufferTextureSampler")
                .then(|| Provided::Sampler(point(device))),
        };
    }
    let target = |index: usize| inputs.gbuffer.views[index].clone();
    match name {
        "GBufferTextureSampler0Global" => Some(Provided::View(target(0))),
        "GBufferTextureSampler1Global" => Some(Provided::View(target(1))),
        "GBufferTextureSampler2Global" => Some(Provided::View(target(2))),
        "GBufferTextureSampler3Global" => Some(Provided::View(target(3))),
        "GBufferTextureSamplerDepthGlobal" => Some(Provided::View(inputs.depth.depth.clone())),
        "fogVolumeDepthSampler" => Some(Provided::View(inputs.depth.depth.clone())),
        "gbufferStencilTextureGlobal" => Some(Provided::View(inputs.depth.stencil.clone())),
        "ReflectionSampler" => Some(Provided::View(inputs.reflection.clone())),
        "gDeferredLightSampler2" => Some(white()),
        _ => None,
    }
}

/// How a pass over the whole frame is drawn: where nothing was drawn the stencil still holds its clear value, and the pass leaves those pixels be.
pub fn over_what_was_drawn() -> DrawState {
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::NotEqual,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op: wgpu::StencilOperation::Keep,
    };
    DrawState {
        depth_attachment: true,
        stencil: wgpu::StencilState {
            front: face,
            back: face,
            read_mask: 0xff,
            write_mask: 0,
        },
        ..DrawState::default()
    }
}

/// How a pass over the pixels nothing was drawn at is drawn: where the stencil still holds its clear value, and only there.
pub fn over_what_was_not_drawn() -> DrawState {
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::Equal,
        ..over_what_was_drawn().stencil.front
    };
    DrawState {
        stencil: wgpu::StencilState {
            front: face,
            back: face,
            ..over_what_was_drawn().stencil
        },
        ..over_what_was_drawn()
    }
}

/// How a pass over the pixels of one material is drawn: where the stencil's material bits equal the pass's reference.
fn over_one_material() -> DrawState {
    let drawn = over_what_was_drawn();
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::Equal,
        ..drawn.stencil.front
    };
    DrawState {
        stencil: wgpu::StencilState {
            front: face,
            back: face,
            read_mask: MATERIAL_TYPE_MASK,
            ..drawn.stencil
        },
        ..drawn
    }
}

/// The game's directional light pass over the whole frame: by the sun where it shines and by the ambient light alone where it does not.
pub struct LightingPass {
    standard: ScreenPass,
    ambient: ScreenPass,
    /// None where the pack has no backlit pass: a tree is then lit as any other surface.
    backlit: Option<ScreenPass>,
}

impl LightingPass {
    /// Both passes, drawn into colour targets of `colour`, over what `inputs` hold.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        colour: wgpu::TextureFormat,
        inputs: &Inputs<'_>,
    ) -> Result<Self, ScreenError> {
        let target = Target {
            colour,
            depth: DEPTH_STENCIL_FORMAT,
            depth_compare: wgpu::CompareFunction::Always,
            blend: None,
            colour_attachments: Some(1),
        };
        let build = |pass: usize, draw: &DrawState| {
            ScreenPass::new(
                device,
                queue,
                pack,
                "directional",
                "directional",
                pass,
                &target,
                draw,
                Space::Unit,
                &mut |name, requirement| provide(device, inputs, name, requirement),
            )
        };
        let draw = over_what_was_drawn();
        Ok(Self {
            standard: build(DIRECTIONAL_STANDARD, &draw)?,
            ambient: build(DIRECTIONAL_AMBIENT, &draw)?,
            backlit: build(DIRECTIONAL_BACKLIT, &over_one_material()).ok(),
        })
    }

    /// Read `inputs` anew, for targets of another size.
    pub fn bind(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, inputs: &Inputs<'_>) {
        for pass in [&mut self.standard, &mut self.ambient]
            .into_iter()
            .chain(self.backlit.as_mut())
        {
            pass.bind(device, queue, &mut |name, requirement| {
                provide(device, inputs, name, requirement)
            });
        }
    }

    /// Write the constants of this frame to both passes.
    pub fn write(&self, queue: &wgpu::Queue, engine: &EngineConstants, screen: &Screen) {
        let mut values = engine.values();
        values.extend(lighting_values(engine, screen));
        self.standard.write(queue, &values);
        self.ambient.write(queue, &values);
        if let Some(backlit) = &self.backlit {
            backlit.write(queue, &values);
        }
    }

    /// Draw the pass that suits `sun` into `pass`: its colour target of the format this was built for, and the depth and stencil of the frame, which it only reads.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>, sun: &Sun) {
        pass.set_stencil_reference(MATERIAL_CLEAR);
        if sun.shines() {
            self.standard.record(pass);
            // A tree is lit again by the pass that lights its leaves from behind, over what the default pass wrote
            if let Some(backlit) = &self.backlit {
                pass.set_stencil_reference(MATERIAL_TREE);
                backlit.record(pass);
                pass.set_stencil_reference(MATERIAL_CLEAR);
            }
        } else {
            self.ambient.record(pass);
        }
    }
}

/// The passes of the frame that do not depend on what is drawn: the game's lighting and its mapping of the light to the screen.
pub struct FramePasses {
    /// None where the pack has no pass for it: the ambient light is then not darkened by what stands around.
    ambient: Option<AmbientPass>,
    /// None where the pack has no pass for it: the frame is then mapped straight to the screen.
    antialias: Option<AntiAliasPass>,
    /// None where the pack has no pass for it: the sun then shines on every surface it faces.
    shadow: Option<ShadowPass>,
    /// None where the pack has no pass for it: the bright part of the frame is then not put back over it.
    bloom: Option<BloomPass>,
    /// None where the pack has no sky effect: there is then none to show.
    sky: Option<SkyPass>,
    /// None where the pack has no pass for it: the particles then give no light.
    lights: Option<PointLights>,
    /// None where the pack has no pass for it: the particles' lights that shine in a cone are then not drawn.
    spots: Option<SpotLights>,
    /// None where the pack has no pass for it: the lights that shine from a segment are then not drawn.
    capsules: Option<CapsuleLights>,
    /// None where the pack has no pass for it: the particles then fill the air with nothing.
    fog: Option<FogVolumes>,
    /// None where the pack has no pass for it: a light seen from afar then shows no glow.
    distant: Option<DistantLights>,
    /// None where the pack has no pass for it or no depth at half size to read: a light's source then shows no glow.
    coronas: Option<Coronas>,
    lighting: LightingPass,
    resolve: ResolvePass,
    /// The rooms whose lamps light only what lies inside them, and the passes that mark them.
    room_mask: RoomMask,
    light_rooms: std::sync::Mutex<Vec<LightRoom>>,
}

impl FramePasses {
    /// The passes for a frame lit into `hdr` and mapped to colour targets of `display`, reading what `inputs` hold.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        hdr: wgpu::TextureFormat,
        display: wgpu::TextureFormat,
        inputs: &Inputs<'_>,
    ) -> Result<Self, ScreenError> {
        let ambient = AmbientPass::new(device, queue, pack, inputs)
            .map_err(|error| tracing::warn!(%error, "the game's ambient occlusion is not drawn"))
            .ok();
        let antialias = AntiAliasPass::new(device, queue, pack, display, inputs)
            .map_err(|error| tracing::warn!(%error, "the game's anti-aliasing is not drawn"))
            .ok();
        let shadow = ShadowPass::new(device, queue, pack, inputs)
            .map_err(|error| tracing::warn!(%error, "the sun's shadow is not drawn"))
            .ok();
        let sky = SkyPass::new(device, queue, pack);
        let lights = PointLights::new(device, queue, pack, hdr, inputs)
            .map_err(|error| tracing::warn!(%error, "the light of the particles is not drawn"))
            .ok();
        let spots = SpotLights::new(device, queue, pack, hdr, inputs)
            .map_err(|error| tracing::warn!(%error, "the cone light of the particles is not drawn"))
            .ok();
        let capsules = CapsuleLights::new(device, queue, pack, hdr, inputs)
            .map_err(|error| tracing::warn!(%error, "the lights that shine from a segment are not drawn"))
            .ok();
        let fog = FogVolumes::new(device, queue, pack, hdr, inputs)
            .map_err(|error| tracing::warn!(%error, "the fog of the particles is not drawn"))
            .ok();
        let distant = DistantLights::new(device, queue, pack, hdr)
            .map_err(|error| tracing::warn!(%error, "the distant lights are not drawn"))
            .ok();
        let coronas = (ambient.as_ref()).and_then(|ambient| {
            Coronas::new(device, queue, pack, hdr, ambient.depth())
                .map_err(|error| tracing::warn!(%error, "the coronas are not drawn"))
                .ok()
        });
        let bloom = BloomPass::new(device, queue, pack, inputs)
            .map_err(|error| tracing::warn!(%error, "the game's bloom is not drawn"))
            .ok();
        let resolve = ResolvePass::new(device, queue, pack, display, antialias.is_none(), inputs)?;
        Ok(Self {
            ambient,
            antialias,
            shadow,
            bloom,
            sky,
            lights,
            spots,
            capsules,
            fog,
            distant,
            coronas,
            lighting: LightingPass::new(device, queue, pack, hdr, inputs)?,
            resolve,
            room_mask: RoomMask::new(device),
            light_rooms: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Read `inputs` anew, for targets of another size.
    pub fn bind(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, inputs: &Inputs<'_>) {
        if let Some(ambient) = &mut self.ambient {
            ambient.bind(device, queue, inputs);
            if let Some(coronas) = &mut self.coronas {
                coronas.bind(device, queue, ambient.depth());
            }
        }
        if let Some(antialias) = &mut self.antialias {
            antialias.bind(device, queue, inputs);
        }
        if let Some(shadow) = &mut self.shadow {
            shadow.bind(device, queue, inputs);
        }
        if let Some(bloom) = &mut self.bloom {
            bloom.bind(device, queue, inputs);
        }
        self.lighting.bind(device, queue, inputs);
        if let Some(lights) = &mut self.lights {
            lights.bind(device, queue, inputs);
        }
        if let Some(spots) = &mut self.spots {
            spots.bind(device, queue, inputs);
        }
        if let Some(capsules) = &mut self.capsules {
            capsules.bind(device, queue, inputs);
        }
        if let Some(fog) = &mut self.fog {
            fog.bind(device, queue, inputs);
        }
        self.resolve.bind(device, queue, inputs);
    }

    /// The textures the passes keep of their own, for the bytes they hold.
    pub fn textures(&self) -> impl Iterator<Item = &wgpu::Texture> {
        self.ambient
            .iter()
            .flat_map(AmbientPass::textures)
            .chain(self.shadow.iter().map(ShadowPass::texture))
            .chain(self.bloom.iter().flat_map(BloomPass::textures))
    }

    /// Write the constants of this frame to the passes, with the `occlusion` the day's setting gives the ambient light.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        tone: &Tone,
        occlusion: f32,
    ) {
        if let Some(ambient) = &self.ambient {
            ambient.write(queue, engine, screen, occlusion);
        }
        if let Some(antialias) = &self.antialias {
            antialias.write(queue, engine, screen);
        }
        if let Some(bloom) = &self.bloom {
            bloom.write(queue, screen, tone);
        }
        self.lighting.write(queue, engine, screen);
        self.resolve.write(queue, engine, screen, tone);
    }

    /// Write the lights the particles give this frame, which [`Self::record_lighting`] draws.
    pub fn write_point_lights(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        lights: &[PointLight],
    ) {
        if let Some(pass) = &self.lights {
            pass.write(queue, engine, screen, lights);
        }
    }

    /// Write the lights that shine in a cone the particles give this frame, which [`Self::record_lighting`] draws.
    pub fn write_spot_lights(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        lights: &[SpotLight],
    ) {
        if let Some(pass) = &self.spots {
            pass.write(queue, engine, screen, lights);
        }
    }

    /// Write the lights that shine from a segment this frame, which [`Self::record_lighting`] draws.
    pub fn write_capsule_lights(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        lights: &[CapsuleLight],
    ) {
        if let Some(pass) = &self.capsules {
            pass.write(queue, engine, screen, lights);
        }
    }

    /// Write the distant lights of this frame, sprites of `size`, which [`Self::record_distant_lights`] draws.
    pub fn write_distant_lights(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        (look, lights): (SpriteLook, &std::sync::Arc<[DistantLight]>),
    ) {
        if let Some(pass) = &self.distant {
            pass.write(queue, engine, screen, look, lights);
        }
    }

    /// Draw the distant lights the frame was written with into `pass`: the lit frame, with the depth of the frame, which it only reads.
    pub fn record_distant_lights(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(distant) = &self.distant {
            distant.record(pass);
        }
        if let Some(coronas) = &self.coronas {
            coronas.record(pass);
        }
    }

    /// Write the coronas of this frame, which [`Self::record_distant_lights`] draws after the distant lights; they read the depth at half size, which [`Self::record_ambient`] makes when asked.
    pub fn write_coronas(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        (look, coronas): (CoronaLook, &[Corona]),
    ) {
        if let Some(pass) = &self.coronas {
            pass.write(queue, engine, screen, (look, coronas));
        }
    }

    /// Write the fog the particles give this frame, which [`Self::record_fog`] draws.
    pub fn write_fog(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        sky: &FogSky,
        volumes: &[FogVolume],
    ) {
        if let Some(pass) = &self.fog {
            pass.write(queue, engine, screen, sky, volumes);
        }
    }

    /// Draw the fog the frame was written with into `pass`: the frame the light is summed in, with the depth of the frame, which it only reads.
    pub fn record_fog(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(fog) = &self.fog {
            fog.record(pass);
        }
    }

    /// Blur the bright part of the lit frame `hdr` and add it back to the frame, where `tone` has any bloom to add.
    pub fn record_bloom(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        tone: &Tone,
    ) {
        if let Some(bloom) = &self.bloom {
            bloom.record(encoder, hdr, tone);
        }
    }

    /// Whether the pack can draw the sky.
    pub fn has_sky(&self) -> bool {
        self.sky.is_some()
    }

    /// Draw the stars and the moon's face from `textures`, which the install gives.
    pub fn set_sky_textures(&mut self, textures: super::dome::SkyTextures) {
        if let Some(sky) = &mut self.sky {
            sky.set_textures(textures);
        }
    }

    /// Draw the noise the sky's clouds are cut from, where this frame has them; call it before the pass the sky is drawn in.
    pub fn record_sky_noise(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(sky) = &self.sky {
            sky.record_noise(encoder);
        }
    }
    /// Write the constants of the sky of this frame.
    pub fn write_sky(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        sky: &super::sky::Sky,
        far: f32,
        view: Option<&super::sky_view::SkyView>,
    ) {
        if let Some(pass) = &self.sky {
            pass.write(queue, engine, sky, far, view);
        }
    }

    /// Draw the sky into `pass`, over the pixels nothing was drawn at.
    pub fn record_sky(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(sky) = &self.sky {
            sky.record(pass);
        }
    }

    /// Map the frame to the screen where nothing was drawn, in the pass that maps the frame.
    pub fn record_background(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.resolve.record_background(pass);
    }

    /// Whether the frame has any bloom to show beyond what was drawn, which is drawn by [`Self::record_halo`].
    pub fn blooms(&self, tone: &Tone) -> bool {
        self.bloom.is_some() && BloomPass::runs_for(tone)
    }

    /// Map the bloom that lies where nothing was drawn, in the pass that maps the frame.
    pub fn record_halo(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.resolve.record_halo(pass);
    }

    /// Whether the frame is anti-aliased: it is then mapped to a target of its own first, and drawn to the screen by [`Self::record_antialias`].
    pub fn antialiases(&self) -> bool {
        self.antialias.is_some()
    }

    /// Anti-alias the mapped frame onto the screen in `pass`, where the frame is.
    pub fn record_antialias(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(antialias) = &self.antialias {
            antialias.record(pass);
        }
    }

    /// How the sun's shadow falls on the `sphere` of the world the scene is in, where the pass for it is built and the sun shines.
    pub fn shadow_fit(&self, engine: &EngineConstants, sphere: (Vec3, f32)) -> Option<Fit> {
        self.shadow.as_ref()?;
        ShadowPass::fit(engine, sphere)
    }

    /// Write the constants of the pass that receives the shadow.
    pub fn write_shadow(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        fit: &Fit,
    ) {
        if let Some(shadow) = &self.shadow {
            shadow.write(queue, engine, screen, fit);
        }
    }

    /// Whether the pass that receives the sun's shadow is built.
    pub fn has_shadow(&self) -> bool {
        self.shadow.is_some()
    }

    /// The map the sun's shadow is cast into, its tiles one above the other, where the pass for it is built.
    pub fn shadow_map(&self) -> Option<wgpu::TextureView> {
        (self.shadow.as_ref()).map(|s| s.texture().create_view(&Default::default()))
    }

    /// Write the constants of the pass that receives the shadow, the shadow's own being `shadow`.
    pub fn write_shadow_with(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        shadow: BTreeMap<&'static str, Vec<f32>>,
    ) {
        if let Some(pass) = &self.shadow {
            pass.write_with(queue, engine, screen, shadow);
        }
    }

    /// Cast the shadow: `cast` draws the surfaces from the sun into the pass it is given.
    pub fn record_shadow_cast(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cast: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) {
        if let Some(shadow) = &self.shadow {
            shadow.record_cast(encoder, cast);
        }
    }

    /// Multiply the self-shadow of the G-buffer by what the sun lets through, with the depth and stencil of the frame `depth`.
    pub fn record_shadow_receive(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        gbuffer: &GBuffer,
        depth: &wgpu::TextureView,
    ) {
        if let Some(shadow) = &self.shadow {
            shadow.record_receive(encoder, gbuffer.self_shadows(), depth);
        }
    }

    /// Darken the ambient scales of the G-buffer where surfaces stand close around a pixel, before the lighting reads them; with `depth_wanted` the depth of the frame is made even where there is no occlusion to work out.
    pub fn record_ambient(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        occlusion: f32,
        depth_wanted: bool,
    ) {
        if let Some(ambient) = &self.ambient {
            ambient.record(encoder, occlusion, depth_wanted);
        }
    }

    /// The depth of the frame at half size, in distances along the view, where the pack has the ambient occlusion pass that makes it.
    pub fn half_depth(&self) -> Option<&wgpu::TextureView> {
        self.ambient.as_ref().map(AmbientPass::depth)
    }

    /// Draw the lights of `ranges` (points, spots, capsules) into `pass`; `in_room` only over the pixels a box marked, the stencil's reference at its bit.
    fn record_lights_of(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ranges: &[Range<u32>; 3],
        in_room: bool,
    ) {
        if let Some(lights) = &self.lights {
            lights.record_range(pass, ranges[0].clone(), in_room);
        }
        if let Some(spots) = &self.spots {
            spots.record_range(pass, ranges[1].clone(), in_room);
        }
        if let Some(capsules) = &self.capsules {
            capsules.record_range(pass, ranges[2].clone(), in_room);
        }
    }

    /// Light the G-buffer into `pass`, by the sun and then by the lights that light every pixel, those before any box's; [`Self::record_rooms`] draws the rest.
    pub fn record_lighting(&self, pass: &mut wgpu::RenderPass<'_>, engine: &EngineConstants) {
        self.lighting.record(pass, &Sun::of(engine));
        let rooms = self.light_rooms.lock().unwrap_or_else(|p| p.into_inner());
        let free =
            std::array::from_fn(|k| 0..rooms.first().map_or(u32::MAX, |r| r.lights[k].start));
        self.record_lights_of(pass, &free, false);
    }

    /// Draw the lamps of each box into the lit frame `hdr`, over the pixels whose surface lies inside the box: a box after another, its pixels marked in the stencil of `depth`, its lamps drawn over those an earlier box of theirs did not light, the marks cleared after each set of lamps.
    // ⚠ Passes of their own: the lighting pass samples the depth, so the stencil cannot be written there.
    pub fn record_rooms(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        let rooms = self.light_rooms.lock().unwrap_or_else(|p| p.into_inner());
        if rooms.is_empty() {
            return;
        }
        fn stencil_pass<'a>(
            encoder: &'a mut wgpu::CommandEncoder,
            depth: &wgpu::TextureView,
        ) -> wgpu::RenderPass<'a> {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("room-mask"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
        }
        fn light_pass<'a>(
            encoder: &'a mut wgpu::CommandEncoder,
            hdr: &wgpu::TextureView,
            depth: &wgpu::TextureView,
        ) -> wgpu::RenderPass<'a> {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("room-lights"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
        }
        let marked = rooms.len().min(MOST_BOXES);
        // Where the boxes of the set of lamps being drawn start.
        let mut set = 0;
        for k in 0..marked {
            {
                let mut pass = stencil_pass(encoder, depth);
                if k > 0 && rooms[k].lights != rooms[k - 1].lights {
                    // A new set of lamps starts from no mark: the last set's boxes are cleared.
                    (set..k).for_each(|j| self.room_mask.clear(&mut pass, j));
                    set = k;
                } else if k > 0 {
                    self.room_mask.keep(&mut pass, k - 1);
                }
                self.room_mask.mark(&mut pass, k);
            }
            let mut pass = light_pass(encoder, hdr, depth);
            pass.set_stencil_reference(ROOM_BIT);
            self.record_lights_of(&mut pass, &rooms[k].lights, true);
        }
        // ponytail: lamps past the most boxes a frame marks light unmasked; more marks if a frame shows that many.
        if rooms.len() > marked {
            let mut pass = light_pass(encoder, hdr, depth);
            let mut drawn: Vec<&[Range<u32>; 3]> = vec![&rooms[marked - 1].lights];
            for room in &rooms[marked..] {
                if !drawn.contains(&&room.lights) {
                    self.record_lights_of(&mut pass, &room.lights, false);
                    drawn.push(&room.lights);
                }
            }
        }
        let mut pass = stencil_pass(encoder, depth);
        (set..marked).for_each(|j| self.room_mask.clear(&mut pass, j));
    }
    /// Say which lamps of the lists written this frame light only their room's box, which [`Self::record_lighting`] masks them to.
    pub fn write_light_rooms(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        rooms: &[LightRoom],
    ) {
        let bounds: Vec<Mat4> = rooms.iter().map(|r| r.bounds).collect();
        if !bounds.is_empty() {
            let eye = engine.view.inverse().w_axis.truncate();
            (self.room_mask).write(queue, engine.projection * engine.view, eye, &bounds);
        }
        let mut kept = self.light_rooms.lock().unwrap_or_else(|p| p.into_inner());
        kept.clear();
        kept.extend_from_slice(rooms);
    }

    /// Map the lit frame to the screen in `pass`.
    pub fn record_resolve(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.resolve.record(pass);
    }
}

/// The opaque surfaces of a drawable, which write the G-buffer, and what blends over them, which is drawn into the lit frame.
pub struct DeferredScene {
    gbuffer: MeshScene,
    blended: MeshScene,
    /// The water as the colour its fog gives it, which is laid into the lit frame before the water reads a copy of it.
    water_fog: MeshScene,
    /// The opaque surfaces as depth the sun's shadow is cast by.
    shadow: MeshScene,
    /// The sphere the surfaces lie in, which the shadow is fitted to.
    sphere: Option<(Vec3, f32)>,
    /// The bones of the skeleton the skinned surfaces are weighted to, in the order of the matrices the pose is written to.
    palette: Vec<u16>,
}

impl DeferredScene {
    /// Prepare `surfaces` for a frame lit into `hdr`.
    ///
    /// `caches` hold the pipelines built for the last scene, which a scene made again takes instead of building them; the one for what writes the G-buffer first.
    pub fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        caches: &mut (PipelineCache, PipelineCache, PipelineCache, PipelineCache),
        hdr: wgpu::TextureFormat,
        surfaces: &[(Surface, Material)],
    ) -> Self {
        let (gbuffer_cache, blended_cache, shadow_cache, fog_cache) = caches;
        let gbuffer = MeshScene::prepare_routed(
            gbuffer_cache,
            device,
            queue,
            pack,
            &gbuffer_target(),
            surfaces,
            Route::GBuffer,
        );
        let forward = Target {
            colour: hdr,
            depth: DEPTH_STENCIL_FORMAT,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            blend: None,
            colour_attachments: Some(1),
        };
        let blended = MeshScene::prepare_routed(
            blended_cache,
            device,
            queue,
            pack,
            &forward,
            surfaces,
            Route::Blended,
        );
        // Two colour attachments, the second of which nothing keeps; the colour is scaled by the constant the pass is given
        let fog = MeshScene::prepare_routed(
            fog_cache,
            device,
            queue,
            pack,
            &forward,
            surfaces,
            Route::WaterFog,
        );
        let depth_only = Target {
            colour: GBUFFER_FORMAT,
            depth: super::shadow::MAP_FORMAT,
            depth_compare: wgpu::CompareFunction::Less,
            blend: None,
            colour_attachments: Some(0),
        };
        let shadow = MeshScene::prepare_routed(
            shadow_cache,
            device,
            queue,
            pack,
            &depth_only,
            surfaces,
            Route::Shadow,
        );
        let sphere = bounds(surfaces).map(|(min, max)| sphere_of(min, max));
        let palette = super::mesh::bone_palette(surfaces);
        Self {
            gbuffer,
            blended,
            water_fog: fog,
            shadow,
            sphere,
            palette,
        }
    }

    /// The bones of the skeleton the skinned surfaces are weighted to, whose skin matrices the pose is read by.
    pub fn palette(&self) -> &[u16] {
        &self.palette
    }

    /// The sphere the surfaces lie in.
    pub fn sphere(&self) -> Option<(Vec3, f32)> {
        self.sphere
    }

    /// Write the constants the shadow is cast with.
    pub fn update_shadow(&self, queue: &wgpu::Queue, caster: &EngineConstants) {
        self.shadow.update(queue, caster);
    }

    /// Draw the surfaces that cast the shadow into `pass`.
    pub fn record_shadow(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.shadow.record(pass);
    }

    /// How many surfaces are drawn, and the effect of each that is left out and why.
    pub fn report(&self) -> (usize, Vec<(String, String)>) {
        let left_out = self
            .gbuffer
            .skipped()
            .iter()
            .chain(self.blended.skipped())
            .map(|(effect, error)| (effect.clone(), error.to_string()))
            .collect();
        (self.gbuffer.drawn() + self.blended.drawn(), left_out)
    }

    /// Write the constants of this frame to every pass of the scene.
    pub fn update(&self, queue: &wgpu::Queue, engine: &EngineConstants) {
        self.gbuffer.update(queue, engine);
        self.blended.update(queue, engine);
        self.water_fog.update(queue, engine);
    }

    /// Draw the surfaces that write the G-buffer into `pass`.
    pub fn record_gbuffer(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.gbuffer.record(pass);
    }

    /// Draw what blends over the lit frame into `pass`.
    pub fn record_blended(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.blended.record(pass);
    }

    /// Whether the scene holds water, which the fog pass lays into the frame.
    pub fn has_water(&self) -> bool {
        self.water_fog.drawn() > 0
    }

    /// Draw the colour of the water into `pass`, which scales it by its blend constant.
    pub fn record_water_fog(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.water_fog.record(pass);
    }
}

#[cfg(test)]
mod tests;
