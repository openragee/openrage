//! The sky as the game draws it: a dome about the camera, drawn by the sky effect's own pass over the pixels nothing was drawn at.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;

use glam::{Mat3, Mat4, Vec3};

use super::bindings::Resource;
use super::deferred::{DEPTH_STENCIL_FORMAT, HDR_FORMAT};
use super::engine::EngineConstants;
use super::mesh::{Image, Material, MeshScene, PipelineCache, Route, Surface};
use super::pack::GamePack;
use super::pipeline::{DrawState, Target};
use super::screen::{ScreenPass, Space};
use super::sky::Sky;
use super::sky_view::{SkyView, SkyViewPass};
use super::stand_in::Provided;

const EFFECT: &str = "sky_system";

/// How many segments the dome is cut into round and from the horizon to the top, which its sky colours are worked out at the corners of.
const AROUND: u32 = 96;
const UP: u32 = 48;

/// A sphere of one unit about the origin, the third axis up, with the texture coordinates the sky's clouds and stars are laid by.
pub fn dome() -> Surface {
    let mut surface = Surface::default();
    let (mut first, mut second) = (Vec::new(), Vec::new());
    for ring in 0..=UP {
        let v = ring as f32 / UP as f32;
        let (sin, cos) = (std::f32::consts::PI * v).sin_cos();
        for segment in 0..=AROUND {
            let u = segment as f32 / AROUND as f32;
            let (y, x) = (std::f32::consts::TAU * u).sin_cos();
            surface.positions.push([x * sin, y * sin, cos]);
            first.push([u, v]);
            second.push([u, v]);
        }
    }
    surface.uvs.insert(0, first);
    surface.uvs.insert(1, second);
    let across = AROUND + 1;
    for ring in 0..UP {
        for segment in 0..AROUND {
            let a = ring * across + segment;
            let b = a + across;
            surface.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    surface
}

/// How far from the camera the game puts the sun and the moon, and how big one unit of the size of a disc is there.
const SKY_DISTANCE: f32 = 20000.0;
const DISC_UNIT: f32 = 250.0;

/// How many segments the rim of a disc is cut into.
const RIM: u32 = 48;

/// How far under the horizon the sun is still drawn, as the share of the sky its disc covers.
const SUN_BELOW: f32 = 0.03;

/// A disc at a size of one unit: a fan about the origin, in the plane the vertex stage of the sun and the moon turns to face the camera, with texture coordinates that run `uv_span` across it.
pub fn disc(uv_span: f32) -> Surface {
    let mut surface = Surface::default();
    let mut uvs = vec![[0.5, 0.5]];
    surface.positions.push([0.0; 3]);
    for i in 0..RIM {
        let (sin, cos) = (std::f32::consts::TAU * i as f32 / RIM as f32).sin_cos();
        surface
            .positions
            .push([cos * DISC_UNIT, 0.0, sin * DISC_UNIT]);
        uvs.push([cos * uv_span + 0.5, sin * uv_span + 0.5]);
        surface.indices.extend([0, 1 + i, 1 + (i + 1) % RIM]);
    }
    surface.uvs.insert(0, uvs);
    surface
}

/// The maps the game draws its sky from out of its own texture dictionary: the stars and the face of the moon, and what the clouds and the dither are made of.
#[derive(Debug, Clone, Default)]
pub struct SkyTextures {
    pub stars: Option<Image>,
    pub moon: Option<Image>,
    /// What the clouds are cut from: the noise the game draws them out of each frame, the fine noise laid over them, and the dither that hides the steps of the sky.
    pub noise: Option<Image>,
    pub detail: Option<Image>,
    pub dither: Option<Image>,
}

/// The target a pass of the sky draws into: the frame the light is summed in, tested against nothing.
fn target() -> Target {
    Target {
        colour: HDR_FORMAT,
        depth: DEPTH_STENCIL_FORMAT,
        depth_compare: wgpu::CompareFunction::Always,
        blend: None,
        colour_attachments: Some(1),
    }
}

/// One surface of the sky effect drawn by pass number `pass` of `route`'s technique with `textures`, or `None` where the pack cannot draw it.
fn sky_scene(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pack: &GamePack,
    surface: Surface,
    route: Route,
    pass: usize,
    textures: BTreeMap<String, Image>,
    views: BTreeMap<String, wgpu::TextureView>,
) -> Option<MeshScene> {
    let material = Material {
        effect: EFFECT.into(),
        textures,
        views,
        pass,
        ..Material::default()
    };
    let scene = MeshScene::prepare_routed(
        &mut PipelineCache::new(),
        device,
        queue,
        pack,
        &target(),
        &[(surface, material)],
        route,
    );
    for (effect, error) in scene.skipped() {
        tracing::warn!(%effect, %error, ?route, pass, "the game's sky is not drawn");
    }
    (scene.drawn() == 1).then_some(scene)
}

/// A disc the game draws over its sky, the sun's or the moon's, turned to face the camera and put at half of how far the camera sees.
struct Disc {
    scene: MeshScene,
    /// The variable the vertex stage reads the place of the disc from.
    position: &'static str,
    /// Whether the frame was written with a disc to draw.
    shown: AtomicBool,
}

impl Disc {
    fn new(scene: MeshScene, position: &'static str) -> Self {
        Self {
            scene,
            position,
            shown: AtomicBool::new(false),
        }
    }

    /// The values of the disc's variables: put at half of how far the camera sees, towards `toward`, and made as big as `size` says, about the camera.
    fn values(
        &self,
        engine: &EngineConstants,
        sky: &Sky,
        far: f32,
        toward: Vec3,
        size: f32,
    ) -> BTreeMap<&'static str, Vec<f32>> {
        let eye = engine.view.inverse().w_axis.truncate();
        // Its size is the plane across the way to it, and the way to it only moves it nearer
        let along = Mat3::from_cols(toward * toward.x, toward * toward.y, toward * toward.z);
        let across = Mat3::IDENTITY - along;
        let linear = (Mat3::IDENTITY + across * (size - 1.0)) * (far * 0.5 / SKY_DISTANCE);
        let placed = EngineConstants {
            world: Mat4::from_translation(eye)
                * Mat4::from_mat3(linear)
                * Mat4::from_translation(-eye),
            ..engine.clone()
        };
        let mut values = placed.values();
        values.extend(sky.constants(eye.z));
        values.insert(
            self.position,
            (eye + toward * SKY_DISTANCE).to_array().to_vec(),
        );
        values
    }

    /// Write the frame's constants where `shown`, and remember whether it is.
    fn write(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        sky: &Sky,
        far: f32,
        shown: bool,
        (toward, size): (Vec3, f32),
    ) {
        self.shown.store(shown, Ordering::Relaxed);
        if shown {
            self.scene
                .update_with(queue, &self.values(engine, sky, far, toward, size));
        }
    }

    fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.shown.load(Ordering::Relaxed) {
            self.scene.record(pass);
        }
    }
}

/// The size of the map of noise the clouds are cut from.
const NOISE_SIZE: u32 = 512;

/// The noise the clouds are cut from, which the sky effect draws into a map of its own for the dome to read.
struct Noise {
    map: wgpu::TextureView,
    pass: ScreenPass,
    /// Whether the frame was written with clouds to draw.
    due: AtomicBool,
}

/// How many passes the sky technique has for the combinations of what is up: the sun, the moon, the clouds and the stars.
const VARIANTS: usize = 16;

/// The sky drawn into the frame the light is summed in: the dome by the pass that suits what is up, and the discs of the sun and the moon on it.
pub struct SkyPass {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pack: GamePack,
    textures: SkyTextures,
    /// The dome drawn by each pass of the sky technique, built when a frame first needs it.
    domes: Vec<OnceLock<Option<MeshScene>>>,
    /// The pass the last frame was written for.
    variant: AtomicUsize,
    /// None where the pack has no sun: the sky is then drawn without its disc.
    sun: Option<Disc>,
    /// None where the pack has no moon or the install gave no map of its face.
    moon: Option<Disc>,
    /// None where the install gave no noise to cut clouds from: the sky is then drawn without them.
    noise: Option<Noise>,
    /// The sky of a day that states it as a table, added over the dome's clouds and stars.
    view: SkyViewPass,
}

impl SkyPass {
    /// The pass, where the pack has the sky effect.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, pack: &GamePack) -> Option<Self> {
        let mut pass = Self {
            device: device.clone(),
            queue: queue.clone(),
            pack: pack.clone(),
            textures: SkyTextures::default(),
            domes: (0..VARIANTS).map(|_| OnceLock::new()).collect(),
            variant: AtomicUsize::new(0),
            sun: None,
            moon: None,
            noise: None,
            view: SkyViewPass::new(device),
        };
        // A pack with no dome for the plainest sky has no sky
        pass.dome(0)?;
        pass.sun = sky_scene(
            device,
            queue,
            pack,
            disc(1.0),
            Route::Sun,
            0,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .map(|scene| Disc::new(scene, "sunPosition"));
        Some(pass)
    }

    /// Draw the stars and the face of the moon from `textures`, which the install gives; the passes that drew without them are made again.
    pub fn set_textures(&mut self, textures: SkyTextures) {
        self.domes = (0..VARIANTS).map(|_| OnceLock::new()).collect();
        self.moon = textures.moon.clone().and_then(|image| {
            let scene = sky_scene(
                &self.device,
                &self.queue,
                &self.pack,
                disc(0.5),
                Route::Moon,
                0,
                BTreeMap::from([("moonSampler".to_string(), image)]),
                BTreeMap::new(),
            )?;
            Some(Disc::new(scene, "moonPosition"))
        });
        self.noise = self.make_noise(&textures);
        self.textures = textures;
    }

    /// The pass that draws the noise of the clouds into its map, where the install gave the noise to draw it from and the pack has the pass.
    fn make_noise(&self, textures: &SkyTextures) -> Option<Noise> {
        let source = textures.noise.as_ref()?;
        textures.detail.as_ref()?;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-noise"),
            size: wgpu::Extent3d {
                width: NOISE_SIZE,
                height: NOISE_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let target = Target {
            colour: wgpu::TextureFormat::Rgba8Unorm,
            colour_attachments: Some(1),
            ..target()
        };
        let pass = ScreenPass::new(
            &self.device,
            &self.queue,
            &self.pack,
            EFFECT,
            "draw_perlin_noise",
            0,
            &target,
            &DrawState::default(),
            Space::Unit,
            &mut |name, requirement| match (&requirement.resource, name) {
                (Resource::Texture { .. }, "NoiseSampler") => Some(Provided::Texture2d {
                    width: source.width,
                    height: source.height,
                    rgba: source.rgba.clone(),
                    srgb: source.srgb,
                }),
                _ => None,
            },
        )
        .map_err(|error| tracing::warn!(%error, "the noise of the game's clouds is not drawn"))
        .ok()?;
        Some(Noise {
            map: texture.create_view(&Default::default()),
            pass,
            due: AtomicBool::new(false),
        })
    }

    /// Draw the noise of the clouds into its map, where the frame was written with clouds to draw; call it before the pass the dome is drawn in.
    pub fn record_noise(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(noise) = self
            .noise
            .as_ref()
            .filter(|n| n.due.load(Ordering::Relaxed))
        else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sky-noise-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &noise.map,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        noise.pass.record(&mut pass);
    }

    /// The dome drawn by pass `variant`, built now if no frame has needed it yet.
    fn dome(&self, variant: usize) -> Option<&MeshScene> {
        self.domes[variant]
            .get_or_init(|| {
                // The dither is read by every pass; the stars, and the noise the clouds are cut from, by the passes that draw them
                let wanted = [
                    (true, "ditherSampler", &self.textures.dither),
                    (variant & 1 != 0, "starFieldSampler", &self.textures.stars),
                    (variant & 2 != 0, "highDetailSampler", &self.textures.detail),
                ];
                let textures = wanted
                    .into_iter()
                    .filter_map(|(wanted, name, image)| {
                        Some((name.to_string(), image.clone().filter(|_| wanted)?))
                    })
                    .collect();
                let views = match (variant & 2 != 0, &self.noise) {
                    (true, Some(noise)) => {
                        BTreeMap::from([("perlinSampler".to_string(), noise.map.clone())])
                    }
                    _ => BTreeMap::new(),
                };
                sky_scene(
                    &self.device,
                    &self.queue,
                    &self.pack,
                    dome(),
                    Route::Sky,
                    variant,
                    textures,
                    views,
                )
            })
            .as_ref()
    }

    /// The values of the sky effect's variables: the frame's own, with the dome put about its camera and out to half of how far the camera sees, and what `sky` gives it.
    pub fn values(
        engine: &EngineConstants,
        sky: &Sky,
        far: f32,
    ) -> BTreeMap<&'static str, Vec<f32>> {
        let eye = engine.view.inverse().w_axis.truncate();
        let placed = EngineConstants {
            world: Mat4::from_translation(eye) * Mat4::from_scale(Vec3::splat(far * 0.5)),
            ..engine.clone()
        };
        let mut values = placed.values();
        // The camera is always at the horizon, so that the sun does not climb the dome as the camera does
        values.extend(sky.constants(eye.z));
        values
    }

    /// Write the constants of this frame, with the table of the sky where the day states one.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        sky: &Sky,
        far: f32,
        view: Option<&SkyView>,
    ) {
        self.view.write(queue, engine, view);
        let clouds = self.noise.is_some() && self.textures.detail.is_some();
        let mut variant = sky.variant(self.textures.stars.is_some(), clouds);
        if self.dome(variant).is_none() {
            // A pack that lacks the pass for what is up draws the plain sky
            variant = 0;
        }
        self.variant.store(variant, Ordering::Relaxed);
        let values = Self::values(engine, sky, far);
        if let Some(dome) = self.dome(variant) {
            dome.update_with(queue, &values);
        }
        if let Some(noise) = &self.noise {
            noise.due.store(variant & 2 != 0, Ordering::Relaxed);
            noise.pass.write(queue, &values);
        }
        // A sky whose night is GTA VI's draws its own moon.
        let own_discs = view.is_some_and(|v| v.night.is_some());
        if let Some(sun) = &self.sun {
            let (colour, size) = sky.disc();
            let toward = sky.sun_direction();
            let shown = colour.max_element() > 0.0 && toward.z > -SUN_BELOW;
            sun.write(queue, engine, sky, far, shown, (toward, size));
        }
        if let Some(moon) = &self.moon {
            let (toward, _, size, intensity) = sky.moon();
            let shown = !own_discs && intensity >= 1e-4 && toward.z >= -1e-6;
            moon.write(queue, engine, sky, far, shown, (toward, size));
        }
    }

    /// Draw the dome and the discs on it into `pass`: the frame the light is summed in, with the depth and stencil of the frame, which it only reads.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        let variant = self.variant.load(Ordering::Relaxed);
        if let Some(dome) = self.domes[variant].get().and_then(Option::as_ref) {
            dome.record(pass);
        }
        if let Some(sun) = &self.sun {
            sun.record(pass);
        }
        if let Some(moon) = &self.moon {
            moon.record(pass);
        }
        self.view.record(pass);
    }
}

#[cfg(test)]
mod tests;
