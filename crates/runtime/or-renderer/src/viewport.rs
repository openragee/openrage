use crate::context::CLEAR_COLOR;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadingMode {
    Wireframe,
    #[default]
    Solid,
    Material,
    /// Every surface of the open drawable drawn with the game's own shader passes, from a [`crate::game_shaders::mesh::MeshScene`] the caller prepared; with none, as `Material`.
    Shaded,
    /// One vertex channel, unlit, in the colour slot.
    ///
    /// ⚠ **Which** channel is not the renderer's business. The caller packs
    /// the channel it wants into `MeshVertex::colour` and asks for this
    /// mode; widening the vertex to carry every channel costs 2.7x per
    /// vertex for channels that appear in a fraction of a percent of
    /// buffers.
    Channel,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub colour: [f32; 4],
}

impl LineVertex {
    pub const fn new(position: [f32; 3], colour: [f32; 4]) -> Self {
        Self { position, colour }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub colour: [f32; 4],
    pub depth_bias: f32,
}

impl MeshVertex {
    pub const fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Self {
        Self::new_coloured(position, normal, uv, [1.0, 1.0, 1.0, 1.0])
    }

    pub const fn new_coloured(
        position: [f32; 3],
        normal: [f32; 3],
        uv: [f32; 2],
        colour: [f32; 4],
    ) -> Self {
        Self {
            position,
            normal,
            uv,
            colour,
            depth_bias: 0.0,
        }
    }

    pub const fn with_depth_bias(mut self, depth_bias: f32) -> Self {
        self.depth_bias = depth_bias;
        self
    }
}

/// The attributes of a [`MeshVertex`], which every pipeline that reads one lays out the same.
pub(crate) const MESH_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
    3 => Float32x4,
    4 => Float32,
];

#[derive(Clone)]
pub struct MeshDraw {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub texture_bind_group: Option<Arc<wgpu::BindGroup>>,
    /// Whether the texture is a normal map: Material shades the surface by it and takes no colour from it.
    pub bump: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    /// Where the camera stands in `xyz`, and in `w` how far the plain modes expose what they lit: 1 leaves it as lit.
    camera_pos_ws: [f32; 4],
    camera_misc: [f32; 4],
    /// The way towards the key light of the plain modes in `xyz`, of unit length, and how strongly it lights in `w`.
    key_light: [f32; 4],
}

/// The way towards the key light of the plain modes until a caller sets another, as the shader once spelled it.
pub const KEY_LIGHT_TOWARDS: Vec3 = Vec3::new(0.4, 0.8, 0.3);

/// How strongly the key light lights until a caller sets another.
pub const KEY_LIGHT_STRENGTH: f32 = 0.55;

/// The exposure of the plain modes until a caller sets another: what they lit is drawn as it is.
pub const PLAIN_EXPOSURE: f32 = 1.0;

/// What the shader reads of a key light: the unit way towards it and its strength; the first light where `towards` points nowhere.
fn key_light_uniform(towards: Vec3, strength: f32) -> [f32; 4] {
    let towards = towards
        .try_normalize()
        .unwrap_or(KEY_LIGHT_TOWARDS.normalize());
    [towards.x, towards.y, towards.z, strength.max(0.0)]
}

#[derive(Clone, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: std::f32::consts::FRAC_PI_4,
            pitch: std::f32::consts::FRAC_PI_6,
            distance: 5.0,
            fov_y: 60f32.to_radians(),
            near: 0.05,
            far: 5000.0,
        }
    }
}

impl OrbitCamera {
    pub const WORLD_UP: Vec3 = Vec3::Z;
    pub fn eye(&self) -> Vec3 {
        let cp = self.pitch.cos();
        let sp = self.pitch.sin();
        let cy = self.yaw.cos();
        let sy = self.yaw.sin();
        let dir = Vec3::new(cp * cy, cp * sy, sp);
        self.target + dir * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Self::WORLD_UP)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye(), self.target, Self::WORLD_UP);
        let proj = perspective_reverse_z_rh(self.fov_y, aspect.max(0.0001), self.near, self.far);
        proj * view
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        let sens = 0.005;
        self.yaw -= dx * sens;
        self.pitch = (self.pitch + dy * sens).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let forward = (self.target - self.eye()).normalize_or_zero();
        let right = forward.cross(Self::WORLD_UP).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let scale = self.distance * 0.0015;
        self.target += -right * dx * scale + up * dy * scale;
    }

    pub fn dolly(&mut self, delta_y: f32) {
        let factor = (1.0 - delta_y * 0.1).clamp(0.5, 2.0);
        self.distance = (self.distance * factor).clamp(0.1, 10_000.0);
    }

    pub fn frame_sphere(&mut self, centre: Vec3, radius: f32) {
        self.target = centre;
        let r = radius.max(0.1);
        let dist = r / (self.fov_y * 0.5).tan();
        self.distance = dist * 1.4;
    }
}

fn perspective_reverse_z_rh(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    Mat4::from_cols(
        glam::Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, f, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, near / (far - near), -1.0),
        glam::Vec4::new(0.0, 0.0, near * far / (far - near), 0.0),
    )
}

pub struct ViewportRenderer {
    pub colour_format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    dirty: bool,
    colour: wgpu::Texture,
    colour_view: wgpu::TextureView,
    /// The colour target as the post-processing writes it, which gamma-encodes by itself where the target is sRGB.
    colour_display_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    camera_bgl: Arc<wgpu::BindGroupLayout>,
    line_buffer: Option<wgpu::Buffer>,
    line_count: u32,
    /// How many of the lines, at the end of the buffer, draw over everything else.
    lines_in_front: u32,
    line_buffer_capacity: u64,
    front_line_pipeline: wgpu::RenderPipeline,
    solid_pipeline: wgpu::RenderPipeline,
    material_pipeline: wgpu::RenderPipeline,
    /// The material's translucent texels, drawn after every opaque one and writing no depth.
    material_blend_pipeline: wgpu::RenderPipeline,
    /// The material of a mesh whose texture is a normal map.
    bump_pipeline: wgpu::RenderPipeline,
    channel_pipeline: wgpu::RenderPipeline,
    material_bgl: Arc<wgpu::BindGroupLayout>,
    fallback_material_bg: Arc<wgpu::BindGroup>,
    fallback_material_bytes: u64,
    meshes: Vec<MeshDraw>,
    /// The placed meshes of a world, drawn once for each instance.
    pub(crate) world: crate::world::World,
    game_scene: Option<crate::game_shaders::mesh::MeshScene>,
    /// A GTA VI model drawn by its own mesh and pixel programs, which Shaded draws in place of `game_scene` where there is one.
    gta6_scene: Option<crate::game_shaders::meshlets::MeshletScene>,
    gta6_sky: crate::game_shaders::meshlets::Sky,
    /// The scene drawn by the game's deferred passes, which Shaded draws in place of `game_scene` where there is one.
    deferred: Option<DeferredFrame>,
    deferred_scene: Option<crate::game_shaders::deferred::DeferredScene>,
    /// Whether the deferred scene holds water, which reads a copy of the frame behind it and so is prepared again when the targets are made anew.
    refracts: bool,
    refraction_stale: bool,
    /// The pipelines built for the deferred scenes, for the next to take.
    deferred_pipelines: (
        crate::game_shaders::mesh::PipelineCache,
        crate::game_shaders::mesh::PipelineCache,
        crate::game_shaders::mesh::PipelineCache,
        crate::game_shaders::mesh::PipelineCache,
    ),
    /// The sun's colour and the ambient light the game's passes are lit by.
    game_light: (Vec3, Vec3),
    /// The way towards the sun the game's passes are lit from; over the camera's shoulder where none is set.
    game_sun: Option<Vec3>,
    /// The way towards the key light of the plain modes, and its strength.
    key_light: (Vec3, f32),
    plain_exposure: f32,
    /// Whether the game's own sky is drawn where nothing else is, in place of the background colour.
    sky_background: bool,
    /// Whether the sun's shadow is cast and received by the game's passes.
    shadows: bool,
    /// The moment of the day the game's passes are lit by, which takes the place of the sun and the ambient light above.
    day: Option<crate::game_shaders::lighting::Day>,
    /// How many days have been set, so that what depends on the day knows it has changed.
    day_revision: u64,
    /// The sprites, trails and models of the particle batches drawn with the game's own passes; none where there is no pack.
    game_particles: Option<crate::particle_game::GameParticles>,
    /// How long each stage of a frame took on the device, where the device can time them.
    gpu_timer: Option<crate::gpu_timer::GpuTimer>,
    particle_batches: Vec<crate::particles::ParticleBatch>,
    trail_batches: Vec<crate::trails::TrailBatch>,
    model_batches: Vec<crate::particle_models::ModelBatch>,
    /// The lights the particles give, which light what the frame draws.
    point_lights: Vec<crate::game_shaders::point_lights::PointLight>,
    spot_lights: Vec<crate::game_shaders::spot_lights::SpotLight>,
    capsule_lights: Vec<crate::game_shaders::capsule_lights::CapsuleLight>,
    light_rooms: Vec<crate::game_shaders::room_mask::LightRoom>,
    distant_lights: (
        crate::game_shaders::distant_lights::SpriteLook,
        Arc<[crate::game_shaders::distant_lights::DistantLight]>,
    ),
    /// The coronas of the lights, and how the pass reads the depth about each; none draws no corona.
    coronas: Option<(crate::CoronaLook, Vec<crate::Corona>)>,
    fog_volumes: Vec<crate::game_shaders::fog_volumes::FogVolume>,
    /// The stars and the face of the moon the install gives, which a sky background draws at night.
    sky_textures: crate::game_shaders::dome::SkyTextures,
    /// The ripples and the foam of the game's water.
    water_textures: crate::game_shaders::water::WaterTextures,
    /// The skin matrices of the pose the skinned surfaces of the game's passes are drawn in, by bone of the skeleton; none draws them at rest.
    skin: Vec<Mat4>,
    /// What the colour target is cleared to before anything is drawn.
    background: wgpu::Color,
}

/// The depth and stencil target: the stencil tells what the G-buffer passes drew from what nothing did, and the lighting passes read both.
const DEPTH_FORMAT: wgpu::TextureFormat = crate::game_shaders::deferred::DEPTH_STENCIL_FORMAT;

/// The width of the map of the sky that surfaces reflect, and how many levels it has: the roughest a surface may be is the smallest of them.
const REFLECTION_WIDTH: u32 = 256;
const REFLECTION_LEVELS: u32 = 8;

/// The game's deferred passes for the drawable Shaded shows, with the G-buffer they write and read.
struct DeferredFrame {
    passes: crate::game_shaders::deferred::FramePasses,
    gbuffer: crate::game_shaders::deferred::GBuffer,
    depth: crate::game_shaders::deferred::DepthViews,
    /// Where the light is summed before it is mapped to the screen.
    hdr: wgpu::TextureView,
    /// Where the light is mapped to before the game's anti-aliasing draws it to the screen.
    ldr: wgpu::TextureView,
    /// The map of the sky that surfaces reflect, and the revision of the day it shows.
    reflection: crate::game_shaders::deferred::ReflectionTarget,
    reflection_of: Option<u64>,
    /// Whether the targets were made again since the scene read them.
    stale: bool,
    /// A copy of the lit frame that water reads behind itself, made when a scene holds water and made again with the targets.
    refraction: Option<wgpu::TextureView>,
    /// Where the second colour the water fog pass writes goes, which nothing reads; made with the copy.
    fog_sink: Option<wgpu::TextureView>,
    /// What samples the lit frame for its exposure.
    meter: meter::Meter,
    /// GTA VI's passes after the frame is lit, which a day that states them is drawn through.
    post: post::Post,
    /// GTA VI's colour grade over the lit frame.
    grade: crate::game_shaders::grade::GradePass,
}

/// A pass over `colour` that reads the depth and the stencil of the frame and writes neither, as the lighting, the sky and what blends are drawn: `stencil` says what it does with the stencil where it marks it.
fn over_the_frame<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    colour: &'a wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    depth: &'a wgpu::TextureView,
    stencil: Option<wgpu::Operations<u32>>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: colour,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: None,
            stencil_ops: stencil,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

impl ViewportRenderer {
    pub fn set_particle_batches(&mut self, batches: Vec<crate::particles::ParticleBatch>) {
        self.particle_batches = batches;
    }

    pub fn set_trail_batches(&mut self, batches: Vec<crate::trails::TrailBatch>) {
        self.trail_batches = batches;
    }

    pub fn set_model_batches(&mut self, batches: Vec<crate::particle_models::ModelBatch>) {
        self.model_batches = batches;
    }

    /// The stars and the face of the moon the install gives, which a sky background draws at night; none by default.
    pub fn set_sky_textures(&mut self, textures: crate::game_shaders::dome::SkyTextures) {
        if let Some(frame) = self.deferred.as_mut() {
            frame.passes.set_sky_textures(textures.clone());
        }
        self.sky_textures = textures;
    }

    /// The ripples and the foam the install gives its water, which a scene that holds water is prepared again to read.
    pub fn set_water_textures(&mut self, textures: crate::game_shaders::water::WaterTextures) {
        self.water_textures = textures;
        self.refraction_stale = self.refracts;
    }

    /// The lights the particles give, which light what is drawn in Shaded; none by default.
    pub fn set_point_lights(&mut self, lights: Vec<crate::game_shaders::point_lights::PointLight>) {
        self.point_lights = lights;
    }

    /// Light Shaded with `lights` that shine in a cone, which the game's spot pass draws.
    pub fn set_spot_lights(&mut self, lights: Vec<crate::game_shaders::spot_lights::SpotLight>) {
        self.spot_lights = lights;
    }

    /// Light Shaded with `lights` that shine from a segment, which the game's capsule pass draws.
    pub fn set_capsule_lights(
        &mut self,
        lights: Vec<crate::game_shaders::capsule_lights::CapsuleLight>,
    ) {
        self.capsule_lights = lights;
    }

    /// Say which of the lights set last light only their room's box; none by default.
    pub fn set_light_rooms(&mut self, rooms: Vec<crate::game_shaders::room_mask::LightRoom>) {
        self.light_rooms = rooms;
    }

    /// Show `lights` seen from afar in Shaded as sprites of `size`, which the game's distant lights pass draws.
    pub fn set_distant_lights(
        &mut self,
        size: crate::game_shaders::distant_lights::SpriteLook,
        lights: Arc<[crate::game_shaders::distant_lights::DistantLight]>,
    ) {
        self.distant_lights = (size, lights);
    }

    /// Show the glow of the lights' sources in Shaded, which the game's corona pass draws where nothing nearer hides them.
    pub fn set_coronas(&mut self, coronas: Option<(crate::CoronaLook, Vec<crate::Corona>)>) {
        self.coronas = coronas;
    }

    /// Fill the air of Shaded with `volumes`, which the game's fog volume pass draws over the lit frame.
    pub fn set_fog_volumes(&mut self, volumes: Vec<crate::game_shaders::fog_volumes::FogVolume>) {
        self.fog_volumes = volumes;
    }

    /// Draw the sprites, trails and models of particle batches with this pack's `ptfx_sprite`, `ptfx_trail` and `ptfx_model` passes; `None` draws no particles.
    pub fn set_game_pack(&mut self, pack: Option<Arc<crate::game_shaders::pack::GamePack>>) {
        self.game_particles =
            pack.map(|pack| crate::particle_game::GameParticles::new(&pack, self.game_target()));
        self.refresh_soft_depth();
    }

    /// Hand the particles the depth of this frame to fade out against, made anew with the targets.
    fn refresh_soft_depth(&mut self) {
        let depth = self
            .depth_view
            .texture()
            .create_view(&wgpu::TextureViewDescriptor {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..Default::default()
            });
        if let Some(particles) = self.game_particles.as_mut() {
            particles.trails.set_depth(Some(depth.clone()));
            particles.sprites.set_depth(Some(depth));
        }
    }

    /// Lay out the sprites, trails and models of this frame's particle batches for `camera`; call it before the frame is drawn.
    pub fn prepare_game_particles(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &OrbitCamera,
    ) {
        let aspect = self.width as f32 / self.height as f32;
        let scene = self.game_constants(camera, aspect);
        if let Some(particles) = self.game_particles.as_mut() {
            let batches = crate::particle_game::FrameBatches {
                sprites: &self.particle_batches,
                trails: &self.trail_batches,
                models: &self.model_batches,
            };
            particles.prepare(device, queue, &batches, &scene, camera.eye());
        }
    }

    pub fn material_bind_group_layout(&self) -> Arc<wgpu::BindGroupLayout> {
        self.material_bgl.clone()
    }

    pub fn camera_bind_group_layout(&self) -> Arc<wgpu::BindGroupLayout> {
        self.camera_bgl.clone()
    }

    pub fn colour_texture(&self) -> &wgpu::Texture {
        &self.colour
    }

    pub fn colour_view(&self) -> &wgpu::TextureView {
        &self.colour_view
    }

    pub fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The bytes of the targets this renderer holds; sized by the viewport and never evicted.
    pub fn target_bytes(&self) -> u64 {
        let deferred = self.deferred.iter().flat_map(|d| {
            d.gbuffer
                .textures()
                .chain([d.hdr.texture(), d.ldr.texture()])
                .chain(d.passes.textures())
        });
        [&self.colour, self.depth_view.texture()]
            .into_iter()
            .chain(deferred)
            .map(texture_bytes)
            .sum()
    }

    /// The bytes this renderer holds outside its targets and mesh draws, by class.
    pub fn resident_bytes(&self) -> ResidentBytes {
        let own = ResidentBytes {
            line_buffers: self.line_buffer.as_ref().map_or(0, wgpu::Buffer::size),
            uniform_buffers: buffer_bytes([Some(&self.camera_buffer)]),
            default_textures: self.fallback_material_bytes,
            ..Default::default()
        };
        [
            Some(own),
            self.game_particles.as_ref().map(|p| p.resident_bytes()),
        ]
        .into_iter()
        .flatten()
        .sum()
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (width, height) = fit_texture_limit(
            width.max(1),
            height.max(1),
            device.limits().max_texture_dimension_2d,
        );
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;
        let (colour, colour_view, colour_display_view, depth_view) =
            create_targets(device, self.colour_format, width, height);
        self.colour = colour;
        self.colour_view = colour_view;
        self.colour_display_view = colour_display_view;
        self.depth_view = depth_view;
        self.refresh_soft_depth();
        if let Some(scene) = self.gta6_scene.as_mut() {
            scene.resize(device, (width, height));
        }
        if let Some(deferred) = self.deferred.as_mut() {
            deferred.gbuffer = crate::game_shaders::deferred::GBuffer::new(device, width, height);
            deferred.hdr = crate::game_shaders::deferred::hdr_target(device, width, height);
            deferred.ldr = crate::game_shaders::deferred::ldr_target(
                device,
                self.colour_format.remove_srgb_suffix(),
                width,
                height,
            );
            deferred.depth =
                crate::game_shaders::deferred::DepthViews::of(self.depth_view.texture());
            deferred.stale = true;
            deferred.refraction = None;
            deferred.fog_sink = None;
            self.refraction_stale = self.refracts;
        }
        self.dirty = true;
    }

    /// The lines to draw, the last `in_front` of them over everything else.
    pub fn set_lines(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[LineVertex],
        in_front: usize,
    ) {
        self.line_count = vertices.len() as u32;
        self.lines_in_front = in_front.min(vertices.len()) as u32;
        if vertices.is_empty() {
            return;
        }

        let needed_bytes = std::mem::size_of_val(vertices) as wgpu::BufferAddress;

        if self.line_buffer.is_none() || self.line_buffer_capacity < needed_bytes {
            let cap = needed_bytes.next_power_of_two().max(4096);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport-line-buffer"),
                size: cap,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.line_buffer = Some(buffer);
            self.line_buffer_capacity = cap;
        }

        if let Some(buffer) = &self.line_buffer {
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(vertices));
        }
    }

    pub fn set_meshes(&mut self, meshes: Vec<MeshDraw>) {
        self.meshes = meshes;
    }

    /// The colour the target is cleared to before anything is drawn.
    pub fn set_background(&mut self, rgba: [f32; 4]) {
        let [r, g, b, a] = rgba.map(f64::from);
        self.background = wgpu::Color { r, g, b, a };
    }

    /// The colour and depth target a scene drawn in [`ShadingMode::Shaded`] is prepared for: this viewport's own, with its reversed depth.
    pub fn game_target(&self) -> crate::game_shaders::pipeline::Target {
        crate::game_shaders::pipeline::Target {
            colour: self.colour_format,
            depth: DEPTH_FORMAT,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            blend: None,
            colour_attachments: Some(1),
        }
    }

    /// The scene [`ShadingMode::Shaded`] draws, prepared for [`Self::game_target`]; `None` takes it away.
    pub fn set_game_scene(&mut self, scene: Option<crate::game_shaders::mesh::MeshScene>) {
        self.game_scene = scene;
    }

    pub fn has_game_scene(&self) -> bool {
        self.game_scene.is_some()
    }

    /// The GTA VI model [`ShadingMode::Shaded`] draws by its own programs, prepared for [`Self::game_target`] and [`Self::size`]; `None` takes it away.
    pub fn set_gta6_scene(&mut self, scene: Option<crate::game_shaders::meshlets::MeshletScene>) {
        self.gta6_scene = scene;
    }

    /// The GTA VI model [`ShadingMode::Shaded`] draws by its own programs, where one is set.
    pub fn gta6_scene(&self) -> Option<&crate::game_shaders::meshlets::MeshletScene> {
        self.gta6_scene.as_ref()
    }

    /// Take the GTA VI model away, for [`ViewportRenderer::set_gta6_scene`] to hand back or another.
    pub fn take_gta6_scene(&mut self) -> Option<crate::game_shaders::meshlets::MeshletScene> {
        self.gta6_scene.take()
    }

    /// The sun and the sky a GTA VI drawable's own passes are lit by.
    pub fn set_gta6_sky(&mut self, sky: crate::game_shaders::meshlets::Sky) {
        self.gta6_sky = sky;
    }

    /// The light the plain modes (solid, material) light a surface by: the way towards it and how strongly it lights. A way of no length puts it where it began.
    ///
    /// ⚠ Only a key light: the head light that follows the camera and the ambient light stay as they are, and the channel and wireframe modes are unlit.
    pub fn set_key_light(&mut self, towards: Vec3, strength: f32) {
        self.key_light = (towards, strength);
    }

    /// How far the plain world pass exposes what it lit: at any exposure but 1 a colour is multiplied by it and bent under white; at 1, where it begins, what was lit is drawn as it is.
    pub fn set_plain_exposure(&mut self, exposure: f32) {
        self.plain_exposure = match exposure.is_finite() && exposure > 0.0 {
            true => exposure,
            false => PLAIN_EXPOSURE,
        };
    }

    /// The exposure of the plain world pass, as it was set.
    pub fn plain_exposure(&self) -> f32 {
        self.plain_exposure
    }

    /// The way towards the key light and its strength, as they were set.
    pub fn key_light(&self) -> (Vec3, f32) {
        self.key_light
    }

    /// The stages of the last frame the device timed, with the milliseconds each took; none where it cannot time them.
    pub fn gpu_stages(&self) -> Vec<(&'static str, f32)> {
        self.gpu_timer.as_ref().map_or_else(Vec::new, |t| t.last())
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        camera: &OrbitCamera,
        mode: ShadingMode,
    ) {
        let aspect = self.width as f32 / self.height as f32;
        // Where the frame is anti-aliased over time, everything drawn takes the camera's offset of this frame
        let view_proj = self.jitter(mode) * camera.view_proj(aspect);
        let inv_view_proj = view_proj.inverse();
        let eye = camera.eye();
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[CameraUniform {
                view_proj: view_proj.to_cols_array_2d(),
                inv_view_proj: inv_view_proj.to_cols_array_2d(),
                camera_pos_ws: [eye.x, eye.y, eye.z, self.plain_exposure],
                camera_misc: [
                    camera.near,
                    camera.far,
                    self.width as f32,
                    self.height as f32,
                ],
                key_light: key_light_uniform(self.key_light.0, self.key_light.1),
            }]),
        );

        if let Some(timer) = &self.gpu_timer {
            timer.begin(encoder);
        }
        let deferred = self.deferred_in(mode);
        if let Some((deferred, scene)) = deferred {
            self.render_deferred(encoder, queue, camera, aspect, deferred, scene);
        } else if let (ShadingMode::Shaded, Some(scene)) = (mode, &self.gta6_scene) {
            let lighting = crate::game_shaders::meshlets::Lighting {
                view_proj,
                sky: self.gta6_sky,
                camera: eye,
            };
            scene.update(queue, &lighting);
            scene.record_targets(encoder, |encoder, stage| self.gpu_mark(encoder, stage));
        } else if let (ShadingMode::Shaded, Some(scene)) = (mode, &self.game_scene) {
            scene.update(queue, &self.game_constants(camera, aspect));
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-opaque-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.colour_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // What the deferred passes lit is kept, and the lines and models are drawn over it
                        load: match deferred {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(self.background),
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: match deferred {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(0.0),
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: match deferred {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(0),
                        },
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            let depth_tested = self.line_count - self.lines_in_front;
            if let (Some(buffer), true) = (&self.line_buffer, depth_tested > 0) {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..depth_tested, 0..1);
            }

            self.draw_surface_meshes(&mut pass, mode);
            // The deferred frame has drawn the world into its G-buffer already, and a wireframe shows no surface
            if deferred.is_none() && mode != ShadingMode::Wireframe {
                self.world.draw_forward(&mut pass, &self.bind_group);
            }

            if let Some(game) = &self.game_particles {
                game.models.draw(&mut pass, &self.model_batches);
            }
            self.draw_lines_in_front(&mut pass);
        }
        self.gpu_mark(encoder, "lines and models");

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-transparent-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.colour_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(game) = &self.game_particles {
                game.sprites
                    .draw(&mut pass, &self.particle_batches, &|b| !b.depth_write);
                game.trails.draw(&mut pass, &self.trail_batches, &|_| true);
            }
        }
        self.gpu_mark(encoder, "particles");

        let has_write_enabled = self
            .particle_batches
            .iter()
            .any(|b| b.depth_write && !b.instances.is_empty());
        if has_write_enabled {
            let mut write_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-transparent-pass-depth-write"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.colour_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(game) = &self.game_particles {
                game.sprites
                    .draw(&mut write_pass, &self.particle_batches, &|b| b.depth_write);
            }
        }
        if let Some(timer) = &self.gpu_timer {
            timer.end(encoder);
        }
    }

    fn draw_lines_in_front<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        let (Some(buffer), true) = (&self.line_buffer, self.lines_in_front > 0) else {
            return;
        };
        pass.set_pipeline(&self.front_line_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(self.line_count - self.lines_in_front..self.line_count, 0..1);
    }

    fn draw_surface_meshes<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        mode: ShadingMode,
    ) {
        let mode = match (mode, &self.game_scene) {
            // What is lit is drawn already, and what blends over it is in it
            (ShadingMode::Shaded, _) if self.deferred_in(ShadingMode::Shaded).is_some() => return,
            (ShadingMode::Shaded, _) if self.gta6_scene.is_some() => {
                if let Some(scene) = &self.gta6_scene {
                    scene.record(pass);
                    // What its programs do not draw is drawn as a material, over the same depth
                    let rest = || scene.rest.iter().filter_map(|mesh| self.meshes.get(*mesh));
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    for pipeline in [&self.material_pipeline, &self.material_blend_pipeline] {
                        pass.set_pipeline(pipeline);
                        for draw in rest() {
                            self.draw_mesh(pass, draw, ShadingMode::Material);
                        }
                    }
                }
                return;
            }
            (ShadingMode::Shaded, Some(scene)) => {
                scene.record(pass);
                return;
            }
            // Without a scene to draw, the surfaces are drawn as a material would
            (ShadingMode::Shaded, None) => ShadingMode::Material,
            (other, _) => other,
        };
        match mode {
            ShadingMode::Wireframe | ShadingMode::Shaded => {}
            ShadingMode::Solid | ShadingMode::Material | ShadingMode::Channel => {
                let default_pipeline = match mode {
                    ShadingMode::Solid => &self.solid_pipeline,
                    ShadingMode::Material => &self.material_pipeline,
                    ShadingMode::Channel => &self.channel_pipeline,
                    ShadingMode::Wireframe | ShadingMode::Shaded => {
                        unreachable!("filtered above")
                    }
                };
                let material = matches!(mode, ShadingMode::Material);
                pass.set_bind_group(0, &self.bind_group, &[]);
                for draw in &self.meshes {
                    pass.set_pipeline(match material && draw.bump {
                        true => &self.bump_pipeline,
                        false => default_pipeline,
                    });
                    self.draw_mesh(pass, draw, mode);
                }
                // In the order the drawable lists them, as the game's alpha bucket is
                if material {
                    pass.set_pipeline(&self.material_blend_pipeline);
                    // A normal map has no translucent texel: its alpha is no coverage.
                    for draw in self.meshes.iter().filter(|draw| !draw.bump) {
                        self.draw_mesh(pass, draw, mode);
                    }
                }
            }
        }
    }

    fn draw_mesh<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        draw: &'pass MeshDraw,
        mode: ShadingMode,
    ) {
        let bg = match (mode, draw.texture_bind_group.as_ref()) {
            (ShadingMode::Material, Some(bg)) => bg.as_ref(),
            _ => self.fallback_material_bg.as_ref(),
        };
        pass.set_bind_group(1, bg, &[]);
        pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
        pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..draw.index_count, 0, 0..1);
    }
}

fn create_fallback_material_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::BindGroup, u64) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-material-fallback-tex"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255u8, 255, 255, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("viewport-material-fallback-sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport-material-fallback-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    (bg, texture_bytes(&texture))
}

/// GPU bytes a renderer holds outside its targets and mesh draws, by the class the Statistics panel shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResidentBytes {
    pub line_buffers: u64,
    /// The particle, trail, model, decal and particle-shadow passes' instance and quad buffers.
    pub particle_instance_buffers: u64,
    /// Uniform and storage buffers, the light lists and the fog's density grid among them.
    pub uniform_buffers: u64,
    /// The 1x1 fallback textures bound where nothing else is, and the sky's fallback HDRI.
    pub default_textures: u64,
}

impl std::ops::Add for ResidentBytes {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            line_buffers: self.line_buffers + other.line_buffers,
            particle_instance_buffers: self.particle_instance_buffers
                + other.particle_instance_buffers,
            uniform_buffers: self.uniform_buffers + other.uniform_buffers,
            default_textures: self.default_textures + other.default_textures,
        }
    }
}

impl std::iter::Sum for ResidentBytes {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), std::ops::Add::add)
    }
}

/// The sizes of the buffers present.
pub fn buffer_bytes<'a>(buffers: impl IntoIterator<Item = Option<&'a wgpu::Buffer>>) -> u64 {
    buffers.into_iter().flatten().map(wgpu::Buffer::size).sum()
}

/// A texture's bytes from its format and extent over every mip level and sample; driver padding is not counted.
pub fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    let (size, format) = (texture.size(), texture.format());
    let levels: u64 = (0..texture.mip_level_count())
        .map(|level| {
            format.theoretical_memory_footprint(size.mip_level_size(level, texture.dimension()))
        })
        .sum();
    levels * u64::from(texture.sample_count())
}

/// The size scaled down, aspect kept, until neither side passes the device's texture limit.
fn fit_texture_limit(width: u32, height: u32, limit: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= limit {
        return (width, height);
    }
    let scaled =
        |side: u32| (u64::from(side) * u64::from(limit) / u64::from(longest)).max(1) as u32;
    (scaled(width), scaled(height))
}

fn create_targets(
    device: &wgpu::Device,
    colour_format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::TextureView,
    wgpu::TextureView,
) {
    let display = colour_format.remove_srgb_suffix();
    let colour = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-colour"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: colour_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: if display == colour_format {
            &[]
        } else {
            std::slice::from_ref(&display)
        },
    });
    let colour_view = colour.create_view(&Default::default());
    let colour_display_view = colour.create_view(&wgpu::TextureViewDescriptor {
        format: Some(display),
        ..Default::default()
    });

    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport-depth"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let depth_view = depth.create_view(&Default::default());
    (colour, colour_view, colour_display_view, depth_view)
}

/// Lines that read the depth buffer by `depth_compare` and never write it.
fn build_overlay_line_pipeline(
    device: &wgpu::Device,
    camera_bgl: &wgpu::BindGroupLayout,
    colour_format: wgpu::TextureFormat,
    depth_compare: wgpu::CompareFunction,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("viewport-overlay-line-shader"),
        source: wgpu::ShaderSource::Wgsl(LINE_SHADER.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("viewport-overlay-line-pl"),
        bind_group_layouts: &[Some(camera_bgl)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("viewport-overlay-line-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3,
                    1 => Float32x4,
                ],
            }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(depth_compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: colour_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

const LINE_SHADER: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    // ⚠ Every field of `CameraUniform` has to be declared here, in order,
    // whether or not this shader reads it — a uniform struct is addressed by
    // byte offset, so an omitted field silently shifts every field after it.
    // Lines read only `view_proj`, but the three below still have to be here.
    //
    // This is not hypothetical: `inv_view_proj` was added to the Rust struct
    // for the deferred passes and *not* to this declaration or the mesh
    // shader's, so both forward shaders read `camera_pos_ws` out of the
    // inverse matrix's first column for as long as that lasted. The comment
    // that used to sit here said the layout must match "the mesh shader's
    // view of the same uniform buffer", which is how the two stayed
    // consistent with each other and wrong together.
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) colour:   vec4<f32>,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) colour: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip_pos = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.colour   = in.colour;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.colour;
}
"#;

const MESH_SHADER: &str = r#"
struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var diffuse_tex: texture_2d<f32>;
@group(1) @binding(1) var diffuse_smp: sampler;

struct VsIn {
    @location(0) position:   vec3<f32>,
    @location(1) normal:     vec3<f32>,
    @location(2) uv:         vec2<f32>,
    @location(3) colour:     vec4<f32>,
    @location(4) depth_bias: f32,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
};

// regression that a constant-NDC bias kept reproducing.
// The renderer pairs this scheme with a *reverse-Z* depth buffer
// (see `perspective_reverse_z_rh` in the host code), which gives
const BIAS_MAX_PULL: f32 = 0.05;
const BIAS_DIST_SCALE: f32 = 0.002;

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let to_camera = camera.camera_pos_ws.xyz - in.position;
    let dist_to_cam = length(to_camera);
    let view_dir = to_camera / max(dist_to_cam, 1e-6);
    let pull_metres = in.depth_bias * min(BIAS_MAX_PULL, dist_to_cam * BIAS_DIST_SCALE);
    let biased_pos = in.position + view_dir * pull_metres;
    out.clip_pos = camera.view_proj * vec4<f32>(biased_pos, 1.0);
    out.world_normal = in.normal;
    out.uv = in.uv;
    out.colour = in.colour;
    out.world_pos = in.position;
    return out;
}

fn shaded_normal(world_normal: vec3<f32>, front_facing: bool) -> vec3<f32> {
    if (front_facing) {
        return world_normal;
    }
    return -world_normal;
}

fn shade(world_normal: vec3<f32>, world_pos: vec3<f32>, front_facing: bool) -> f32 {
    let n = normalize(shaded_normal(world_normal, front_facing));
    let key_dir = camera.key_light.xyz;
    let view_dir = normalize(camera.camera_pos_ws.xyz - world_pos);
    let key = max(dot(n, key_dir), 0.0) * camera.key_light.w;
    let head = max(dot(n, view_dir), 0.0) * 0.30;
    return key + head + 0.15;
}

@fragment
fn fs_solid(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let s = shade(in.world_normal, in.world_pos, front_facing);
    return vec4<f32>(in.colour.rgb * s, in.colour.a);
}

// Unlit on purpose. The colour slot carries a channel's value rather than a
// colour, and `shade` would scale it by a lighting term the user cannot
// subtract back out by eye.
@fragment
fn fs_channel(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.colour.rgb, 1.0);
}

// Opaque texels write depth in `fs_material`; the rest wait for `fs_material_blend`, which
// writes none, so a translucent shell never hides the surface inside it.
const OPAQUE_ALPHA: f32 = 0.999;

fn material_colour(in: VsOut, front_facing: bool) -> vec4<f32> {
    let tex = textureSample(diffuse_tex, diffuse_smp, in.uv);
    let s = shade(in.world_normal, in.world_pos, front_facing);
    return vec4<f32>(tex.rgb * in.colour.rgb * s, tex.a * in.colour.a);
}

@fragment
fn fs_material(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let c = material_colour(in, front_facing);
    if (c.a < OPAQUE_ALPHA) {
        discard;
    }
    return c;
}

@fragment
fn fs_material_blend(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let c = material_colour(in, front_facing);
    if (c.a >= OPAQUE_ALPHA) {
        discard;
    }
    return c;
}

// The texture is a tangent-space normal map: it tilts the normal the surface is shaded by and
// colours nothing. Its z is rebuilt from x and y, so a two-channel map reads as a three-channel one.
// No vertex carries a tangent here, so the frame is how the position and the texture coordinate
// change across the pixel; where the coordinate does not change, the normal stays the vertex's.
@fragment
fn fs_bump(
    in: VsOut,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let t = textureSample(diffuse_tex, diffuse_smp, in.uv).xy * 2.0 - 1.0;
    let n = normalize(shaded_normal(in.world_normal, front_facing));
    let dp1 = dpdx(in.world_pos);
    let dp2 = dpdy(in.world_pos);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let tangent = dp2perp * duv1.x + dp1perp * duv2.x;
    let bitangent = dp2perp * duv1.y + dp1perp * duv2.y;
    let longest = max(dot(tangent, tangent), dot(bitangent, bitangent));
    let scale = inverseSqrt(max(longest, 1e-30));
    let z = max(sqrt(max(1.0 - dot(t, t), 0.0)), 0.01);
    let bumped = tangent * (t.x * scale) + bitangent * (t.y * scale) + n * z;
    let s = shade(bumped, in.world_pos, true);
    return vec4<f32>(in.colour.rgb * s, in.colour.a);
}
"#;

mod build;
mod game_frame;
mod meter;
mod post;
pub mod post_math;
mod post_shaders;
pub use meter::Metered;

#[cfg(test)]
mod tests;
