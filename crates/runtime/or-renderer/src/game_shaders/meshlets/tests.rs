use super::*;
use crate::game_shaders::frame::read_back;

fn wgsl(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the WGSL parses")
}

fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    gpu_with(test, false)
}

/// A device of the adapter's own limits, or `plain`, one asked for nothing: the limits every
/// adapter gives.
fn gpu_with(test: &str, plain: bool) -> Option<(wgpu::Device, wgpu::Queue)> {
    // `WGPU_BACKEND` picks the backend, so the same frame is drawn through each one a machine has.
    let mut backends = wgpu::InstanceDescriptor::new_without_display_handle();
    backends.backends = wgpu::Backends::from_env().unwrap_or(backends.backends);
    let instance = wgpu::Instance::new(backends);
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("meshlet-test"),
        required_limits: match plain {
            true => wgpu::Limits::default(),
            false => adapter.limits(),
        },
        ..Default::default()
    }))
    .ok()
}

/// What `body` made the device refuse, if anything.
fn refused<T>(device: &wgpu::Device, body: impl FnOnce() -> T) -> (T, Option<String>) {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = body();
    (
        made,
        pollster::block_on(validation.pop()).map(|e| e.to_string()),
    )
}

/// A stage shaped as a lowered mesh program is: no vertex buffer, three corners a thread, an
/// instance a meshlet, the vertices and the meshlets read from buffers of dwords.
pub(super) const MESH: &str = "
struct Words { words: array<u32> }
@group(0) @binding(0) var<storage, read> vertices: Words;
@group(0) @binding(1) var<storage, read> meshlets: Words;
@group(0) @binding(2) var<uniform> view: array<vec4<f32>, 4>;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) shade: vec3<f32>,
}

@vertex fn main(@builtin(vertex_index) v: u32, @builtin(instance_index) meshlet: u32) -> Out {
    let thread = v / 3u;
    let index = meshlets.words[meshlet * 3u + v % 3u];
    let at = vec3<f32>(
        bitcast<f32>(vertices.words[index * 3u]),
        bitcast<f32>(vertices.words[index * 3u + 1u]),
        bitcast<f32>(vertices.words[index * 3u + 2u]),
    );
    var out: Out;
    out.position = mat4x4<f32>(view[0], view[1], view[2], view[3]) * vec4<f32>(at, 1.0);
    // A meshlet of one primitive: the threads past it export nothing with an area.
    if thread > 0u {
        out.position = vec4<f32>(0.0);
    }
    out.shade = vec3<f32>(1.0, 0.5, 0.25);
    return out;
}";

/// A stage that writes the five targets: the colour of its texture by its constant, a normal
/// along +Z packed as two 12-bit numbers over three bytes, and noughts.
const PIXEL: &str = "
@group(1) @binding(0) var<uniform> tint: vec4<f32>;
@group(1) @binding(1) var diffuse: texture_2d<f32>;
@group(1) @binding(2) var diffuse_sampler: sampler;

struct Targets {
    @location(0) colour: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) terms: vec4<f32>,
    @location(4) motion: vec2<f32>,
}

@fragment fn main(@location(0) shade: vec3<f32>) -> Targets {
    let texel = textureSample(diffuse, diffuse_sampler, vec2<f32>(0.5));
    let packed = 2047u | (2047u << 12u);
    let bytes = vec3<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u), f32(packed >> 16u));
    return Targets(
        vec4<f32>(texel.rgb * tint.rgb * shade, 1.0),
        vec4<f32>(0.5, bytes / 255.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec2<f32>(0.0),
    );
}";

pub(super) const SIZE: u32 = 16;

fn dwords(values: &[u32]) -> Vec<u8> {
    bytemuck::cast_slice(values).to_vec()
}

/// A model of six vertices and two meshlets of a triangle each: one across the middle of the
/// frame, one in its upper left corner.
pub(super) fn surface() -> MeshletSurface {
    let vertices: [[f32; 3]; 6] = [
        [-0.6, -0.6, 0.5],
        [0.6, -0.6, 0.5],
        [0.0, 0.6, 0.5],
        [-1.0, 1.0, 0.5],
        [-0.5, 1.0, 0.5],
        [-1.0, 0.5, 0.5],
    ];
    let positions: Vec<u32> = vertices.iter().flatten().map(|v| v.to_bits()).collect();
    let red = Image {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
        srgb: false,
    };
    let feeds = BTreeMap::from([
        ((0, 0), Feed::Bytes(dwords(&positions))),
        ((0, 1), Feed::Bytes(dwords(&[0, 1, 2, 3, 4, 5]))),
        (
            (0, 2),
            Feed::View {
                bytes: vec![0; 64],
                view_proj: Some(0),
            },
        ),
        (
            (1, 0),
            Feed::Bytes(bytemuck::cast_slice(&[1.0f32; 4]).to_vec()),
        ),
        ((1, 1), Feed::Textures(vec![Some(Arc::new(red))])),
    ]);
    MeshletSurface {
        vertex: Arc::new(wgsl(MESH)),
        fragment: Arc::new(wgsl(PIXEL)),
        feeds,
        meshlets: 2,
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_model_drawn_by_a_mesh_and_a_pixel_stage_is_lit_onto_the_frame() {
    let Some((device, queue)) = gpu("a_model_drawn_by_a_mesh_and_a_pixel_stage") else {
        return;
    };
    let frame = Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: Some(1),
    };
    let size = (SIZE, SIZE);
    let (shown, error) = refused(&device, || {
        let mut pipelines = MeshletPipelines::default();
        let surfaces = [surface()];
        // A worker builds the pass: until it has, there is no scene, and asking again starts no other.
        let mut prepare = || loop {
            let scene =
                MeshletScene::prepare(&device, &queue, &mut pipelines, &surfaces, size, &frame)
                    .expect("the scene prepares");
            match scene {
                Some(scene) => break scene,
                None => std::thread::sleep(std::time::Duration::from_millis(5)),
            }
        };
        let scene = prepare();
        // The same two stages again build nothing more.
        prepare();
        assert_eq!(pipelines.len(), 1);
        assert_eq!(scene.drawn(), 1);
        assert!(scene.refused.is_empty());
        // Beside a surface whose pixel stage is none, the first still draws, and the other is
        // named with why; alone, it is the reason there is no scene.
        let mut unbound = surface();
        unbound.fragment = Arc::clone(&unbound.vertex);
        let mixed = [surface(), unbound.clone()];
        let mut prepare = |surfaces: &[MeshletSurface]| loop {
            match MeshletScene::prepare(&device, &queue, &mut pipelines, surfaces, size, &frame) {
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
                Ok(Some(scene)) => break Ok((scene.drawn(), scene.refused)),
                Err(why) => break Err(why),
            }
        };
        let (drawn, refused) = prepare(&mixed).expect("one surface draws");
        assert_eq!((drawn, refused.len()), (1, 1));
        assert_eq!(refused[0].0, 1);
        assert_eq!(prepare(&[unbound]).err(), Some(refused[0].1.clone()));
        let colour = render_target(
            &device,
            "frame",
            size,
            frame.colour,
            wgpu::TextureUsages::COPY_SRC,
        );
        let depth = render_target(&device, "depth", size, DEPTH, wgpu::TextureUsages::empty());
        let lighting = super::Lighting {
            view_proj: Mat4::IDENTITY,
            sky: overhead(),
            camera: Vec3::Z,
        };
        scene.update(&queue, &lighting);
        let mut encoder = device.create_command_encoder(&Default::default());
        scene.record_targets(&mut encoder);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &colour,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            scene.record(&mut pass);
        }
        queue.submit([encoder.finish()]);
        let normal = scene.target(1).expect("a second target");
        (
            read_back(&device, &queue, colour.texture(), frame.colour, SIZE),
            read_back(
                &device,
                &queue,
                normal,
                wgpu::TextureFormat::Rgba8Unorm,
                SIZE,
            ),
        )
    });
    assert_eq!(error, None);
    let (lit, normal) = shown;
    // The triangle across the middle covers the centre and leaves the corners; the one in the
    // corner is the second instance.
    let pixel = |x: u32, y: u32| {
        let at = ((y * SIZE + x) * 4) as usize;
        [
            lit.bytes[at],
            lit.bytes[at + 1],
            lit.bytes[at + 2],
            lit.bytes[at + 3],
        ]
    };
    let [r, g, b, a] = pixel(SIZE / 2, SIZE / 2);
    // Red by the texture, bright since the light lies along the normal; the gloss of the
    // surface adds some of every colour.
    assert!(
        r > 200 && g < r / 2 && b < r / 2 && a == 255,
        "{:?}",
        [r, g, b, a]
    );
    assert_eq!(pixel(SIZE - 1, SIZE - 1), [0; 4]);
    assert_ne!(pixel(1, 1), [0; 4], "the second meshlet drew nothing");
    assert!(lit.touched() < (SIZE * SIZE) as usize);
    assert!(normal.touched() > 0);
}

#[test]
fn a_stage_s_bindings_are_read_with_their_group_and_a_light_of_no_length_points_up() {
    let module = wgsl(MESH);
    let required = arrayed(&module).expect("its bindings read");
    assert_eq!(required.len(), 3);
    assert!(required
        .iter()
        .all(|(r, count)| r.group == 0 && count.is_none()));
    // A way of no length is the way up.
    let lit = |towards: Vec3| {
        let lighting = super::Lighting {
            view_proj: Mat4::IDENTITY,
            sky: Sky {
                towards,
                ..overhead()
            },
            camera: Vec3::ONE,
        };
        light_bytes(&lighting, false)
    };
    assert_eq!(lit(Vec3::ZERO), lit(Vec3::Z));
}

/// A white sun straight overhead and no sky.
fn overhead() -> Sky {
    Sky {
        towards: Vec3::Z,
        sun: Vec3::splat(std::f32::consts::PI),
        sky: Vec3::ZERO,
        ground: Vec3::ZERO,
        exposure: 1.0,
        tone: None,
        scene: Scene::default(),
    }
}

#[test]
fn the_built_in_sky_has_a_sun_by_day_a_dim_moon_by_night_and_less_of_both_under_clouds() {
    let (noon, night, dusk) = (Sky::at(12.0, 0.0), Sky::at(0.0, 0.0), Sky::at(17.5, 0.0));
    assert!(
        noon.towards.z > 0.9 && night.towards.z > 0.9,
        "{noon:?} {night:?}"
    );
    assert!(noon.sun.max_element() > 5.0 * night.sun.max_element());
    assert!(noon.sky.z > 5.0 * night.sky.z);
    assert!(night.sun.min_element() > 0.1 && night.sky.min_element() > 0.03);
    // Low in the west, and warmer than at noon.
    assert!(dusk.towards.x < 0.0 && dusk.towards.z > 0.0 && dusk.towards.z < 0.3);
    assert!(dusk.sun.z / dusk.sun.x < 0.6 * noon.sun.z / noon.sun.x);
    let storm = Sky::at(12.0, 0.95);
    assert!(storm.sun.max_element() < 0.3 * noon.sun.max_element());
    assert!((storm.sky.x - storm.sky.z).abs() < 0.05, "{storm:?}");
    for hours in 0..48 {
        let sky = Sky::at(hours as f32 / 2.0, 0.3);
        assert!((sky.towards.length() - 1.0).abs() < 1e-4 && sky.towards.z > -0.06);
    }
}

#[test]
fn the_developer_s_aids_are_words_separated_by_commas() {
    let aids = Aids::parse(
        "target=3, pixel=C:/a b/p.wgsl,plain-device,unknown,mesh=m.wgsl,record=r.bin,unguarded",
    );
    assert_eq!(
        aids,
        Aids {
            target: 3.0,
            pixel: Some("C:/a b/p.wgsl".into()),
            mesh: Some("m.wgsl".into()),
            record: Some("r.bin".into()),
            plain_device: true,
            unguarded: true,
        }
    );
    assert_eq!(Aids::parse(""), Aids::default());
}

/// The pixel stage with `more` constant buffers beside its own, each read, declared as bound
/// or as the stage's own noughts.
fn pixel_with(more: u32, bound: impl Fn(u32) -> bool) -> naga::Module {
    let mut text = PIXEL.replace(
        "texel.rgb * tint.rgb * shade",
        "texel.rgb * tint.rgb * shade + others()",
    );
    let mut sum = String::from("vec3<f32>(0.0)");
    for n in 0..more {
        let declared = match bound(n) {
            true => format!("@group(1) @binding({}) var<uniform>", 3 + n),
            false => "var<private>".to_string(),
        };
        text += &format!("\n{declared} other{n}: vec4<f32>;");
        sum += &format!(" + other{n}.rgb");
    }
    text += &format!("\nfn others() -> vec3<f32> {{ return {sum}; }}");
    wgsl(&text)
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_stage_with_more_constant_buffers_than_a_default_device_gives_draws_once_the_noughts_bind_nothing(
) {
    let Some((device, queue)) = gpu_with("a_stage_with_more_constant_buffers", true) else {
        return;
    };
    let frame = Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: Some(1),
    };
    // With its own, the stage declares two more than the device gives.
    let more = device.limits().max_uniform_buffers_per_shader_stage + 1;
    let prepared = |bound: &dyn Fn(u32) -> bool| {
        let mut surface = surface();
        surface.fragment = Arc::new(pixel_with(more, bound));
        let mut pipelines = MeshletPipelines::default();
        loop {
            let size = (SIZE, SIZE);
            match MeshletScene::prepare(
                &device,
                &queue,
                &mut pipelines,
                &[surface.clone()],
                size,
                &frame,
            ) {
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(5)),
                Ok(Some(scene)) => break Ok(scene.drawn()),
                Err(why) => break Err(why),
            }
        }
    };
    let (all_bound, _) = refused(&device, || prepared(&|_| true));
    assert!(all_bound.is_err(), "the device took {more} more buffers");
    let (two_less, error) = refused(&device, || prepared(&|n| n >= 2));
    assert_eq!((two_less, error), (Ok(1), None));
}
