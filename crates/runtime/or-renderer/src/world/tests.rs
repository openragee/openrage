use glam::{Mat4, Vec3};

use super::*;
use crate::game_shaders::deferred::GBUFFER_FORMAT;
use crate::game_shaders::frame::read_back;
use crate::game_shaders::pack::GamePack;
use crate::game_shaders::stand_in::uploaded_view;
use crate::game_shaders::test_pack::{gpu, real_pack};
use crate::viewport::{OrbitCamera, ShadingMode};

const SIZE: u32 = 64;
const BACKGROUND: [f32; 4] = [0.1, 0.2, 0.3, 1.0];
const WHITE: [f32; 4] = [1.0; 4];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
/// The most a fully rough dielectric's highlight adds to a channel in the plain modes: a reflectance of 0.04 in a lobe of exponent one.
const HIGHLIGHT: u8 = 8;

/// The corners of a square about `centre` that reaches `half` to each side, facing up, its texture laid once across it with u along x and v along y.
fn corners(centre: [f32; 2], half: f32) -> [MeshVertex; 4] {
    [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|c: [f32; 2]| {
        MeshVertex::new(
            [centre[0] + c[0] * half, centre[1] + c[1] * half, 0.0],
            [0.0, 0.0, 1.0],
            [c[0] * 0.5 + 0.5, c[1] * 0.5 + 0.5],
        )
    })
}

const SQUARE: [u32; 6] = [0, 1, 2, 0, 2, 3];

/// A square about the origin that reaches `half` to each side, drawn with `material`.
fn square(device: &wgpu::Device, half: f32, material: WorldMaterial) -> Arc<WorldMesh> {
    let vertices = corners([0.0, 0.0], half);
    Arc::new(WorldMesh::new(device, &vertices, &SQUARE, material))
}

/// The material of no map in `bucket`.
fn plain(device: &wgpu::Device, renderer: &ViewportRenderer, bucket: WorldBucket) -> WorldMaterial {
    renderer.world_material(device, None, None, None, bucket)
}

/// A texture of the one texel `rgba`, read as it is stored.
fn texel(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> wgpu::TextureView {
    uploaded_view(device, queue, 1, 1, &rgba, false)
}

/// A camera `distance` over the origin, looking down.
fn overhead_from(distance: f32) -> OrbitCamera {
    OrbitCamera {
        yaw: 0.0,
        pitch: 1.5,
        distance,
        ..OrbitCamera::default()
    }
}

/// A camera five metres over the origin, looking down.
fn overhead() -> OrbitCamera {
    overhead_from(5.0)
}

/// The pixel of a square frame of `SIZE` that `camera` draws `point` of the world at.
fn pixel_of(camera: &OrbitCamera, point: Vec3) -> usize {
    let clip = camera.view_proj(1.0) * point.extend(1.0);
    let (x, y) = (clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    (y * SIZE as f32) as usize * SIZE as usize + (x * SIZE as f32) as usize
}

/// Every pixel of the frame `renderer` draws in `mode`, row by row.
fn frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &ViewportRenderer,
    camera: &OrbitCamera,
    mode: ShadingMode,
) -> Vec<[u8; 4]> {
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, queue, camera, mode);
    queue.submit([encoder.finish()]);
    let read = read_back(
        device,
        queue,
        renderer.colour_texture(),
        renderer.colour_format,
        SIZE,
    );
    read.bytes.as_chunks::<4>().0.to_vec()
}

/// The machine's device as every test takes it, and after it one that issues several draws by one call, where the adapter makes such a device.
fn devices(test: &str) -> Vec<(wgpu::Device, wgpu::Queue)> {
    let Some(first) = gpu(test) else {
        return Vec::new();
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: crate::instance::software_asked(),
    }));
    let wanted = wgpu::Features::INDIRECT_FIRST_INSTANCE;
    let second = adapter
        .ok()
        .filter(|adapter| adapter.features().contains(wanted))
        .and_then(|adapter| {
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("world-test-several-draws"),
                required_features: wanted,
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .ok()
        });
    [Some(first), second].into_iter().flatten().collect()
}

#[test]
fn an_instance_is_the_first_three_rows_of_its_matrix_and_its_tint() {
    let matrix = Mat4::from_scale_rotation_translation(
        Vec3::splat(2.0),
        glam::Quat::from_rotation_z(0.5),
        Vec3::new(10.0, 20.0, 30.0),
    );
    let instance = WorldInstance::of(matrix, [0.1, 0.2, 0.3, 0.4]);
    let point = Vec3::new(1.0, -2.0, 3.0);
    // What the vertex stage does with the rows is what the matrix does to a point
    let placed = Vec3::from_array(
        instance
            .rows
            .map(|row| glam::Vec4::from(row).dot(point.extend(1.0))),
    );
    assert!((placed - matrix.transform_point3(point)).length() < 1e-4);
    assert_eq!(instance.tint, [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(instance.ambient, crate::world::OUTDOORS);
    assert_eq!(instance.cast_only, 0.0);
    assert_eq!(std::mem::size_of::<WorldInstance>(), 76);
}

/// The members of the struct `name` the world's program declares, by name, with their offsets, and its size.
fn declared(name: &str) -> (Vec<(String, u32)>, u32) {
    let module = naga::front::wgsl::parse_str(shader::WORLD_SHADER).expect("the WGSL parses");
    let found = (module.types.iter()).find_map(|(_, ty)| match &ty.inner {
        naga::TypeInner::Struct { members, span } if ty.name.as_deref() == Some(name) => {
            let members = (members.iter())
                .map(|member| (member.name.clone().unwrap_or_default(), member.offset))
                .collect();
            Some((members, *span))
        }
        _ => None,
    });
    found.expect("the struct is declared")
}

#[test]
fn the_world_program_is_valid_and_reads_a_materials_constants_where_the_renderer_writes_them() {
    let module = naga::front::wgsl::parse_str(shader::WORLD_SHADER).expect("the WGSL parses");
    let flags = naga::valid::ValidationFlags::all();
    let mut validator = naga::valid::Validator::new(flags, naga::valid::Capabilities::all());
    validator
        .validate(&module)
        .expect("the program is valid, its samples in uniform control flow");

    let (members, span) = declared("Constants");
    let bytes = uniform_bytes(WorldFamily::FourLayer, &WorldConstants::default());
    assert_eq!(bytes.len() as u32, span);
    let at = |name: &str| {
        (members.iter())
            .find(|(member, _)| member == name)
            .map(|(_, offset)| *offset)
            .expect("the member is declared")
    };
    // The family in front, then each field where `WorldConstants` puts it
    assert_eq!(at("family"), 0);
    assert_eq!(bytes[..4], (WorldFamily::FourLayer as u32).to_le_bytes());
    let fields = [
        ("fresnel", std::mem::offset_of!(WorldConstants, fresnel)),
        (
            "control_filter",
            std::mem::offset_of!(WorldConstants, control_filter),
        ),
        (
            "emissive_power",
            std::mem::offset_of!(WorldConstants, emissive_power),
        ),
        ("flags", std::mem::offset_of!(WorldConstants, flags)),
        (
            "palette_row",
            std::mem::offset_of!(WorldConstants, palette_row),
        ),
        (
            "control_entity",
            std::mem::offset_of!(WorldConstants, control_entity),
        ),
        (
            "emissive_diffuse_tint",
            std::mem::offset_of!(WorldConstants, emissive_diffuse_tint),
        ),
        (
            "grass_bend",
            std::mem::offset_of!(WorldConstants, grass_bend),
        ),
    ];
    for (name, offset) in fields {
        assert_eq!(at(name), 16 + offset as u32, "{name}");
    }
    assert_eq!(
        at("uv_extra"),
        16 + std::mem::offset_of!(WorldConstants, uv_extra) as u32
    );
    assert_eq!(
        at("interior_slant"),
        16 + std::mem::offset_of!(WorldConstants, interior_slant) as u32
    );
    // The family, twenty-one vectors and sixteen scalars
    assert_eq!(members.len(), 38);
    // The frame's light: four vectors, the night's, then the lamps' rows
    assert_eq!(declared("Light").1 as usize, lamps::LIGHT_FLOATS * 4);
}

#[test]
fn a_models_vertex_packs_its_colours_and_coordinates_and_one_it_lacks_is_white_or_its_first() {
    assert_eq!(colour1([1, 2, 3, 4]).to_bits(), 0x0403_0201);
    let mut vertices = corners([0.0, 0.0], 1.0);
    let first = [[0.25, 0.5, 0.75, 1.0]; 4];
    let uv1 = [[0.5, 2.0]; 4];
    coloured(&mut vertices, (Some(&first), None), (Some(&uv1), None));
    let packed = u32::from_le_bytes([64, 128, 191, 255]);
    assert!(vertices.iter().all(|v| v.colour[2].to_bits() == packed));
    assert!(vertices.iter().all(|v| v.colour[..2] == [0.5, 2.0]));
    assert!(vertices.iter().all(|v| v.depth_bias.to_bits() == u32::MAX));
    // No third coordinate: the first, as two halves
    let [u, v] = vertices[2].uv.map(or_io::half::f32_to_half);
    assert_eq!(
        vertices[2].colour[3].to_bits(),
        u32::from(u) | u32::from(v) << 16
    );
    let second = [[0.0, 0.5, 1.0, 0.2]; 3];
    coloured(&mut vertices, (None, Some(&second)), (None, None));
    assert_eq!(vertices[0].colour[2].to_bits(), u32::MAX);
    assert_eq!(vertices[1].colour[..2], vertices[1].uv);
    assert_eq!(
        vertices[0].depth_bias.to_bits(),
        u32::from_le_bytes([0, 128, 255, 51])
    );
    // A list shorter than the mesh leaves the rest white
    assert_eq!(vertices[3].depth_bias.to_bits(), u32::MAX);
    // The stand-ins: no layer but the first where no weights are given, no dirt, no glow
    assert_eq!(STAND_INS[slot::WEIGHTS], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(STAND_INS[slot::SECOND][3], 0.0);
    assert_eq!(STAND_INS[slot::EMISSIVE], [0.0; 4]);
    assert_eq!(slot::material_a(3), slot::EMISSIVE);
}

#[test]
fn the_buckets_are_declared_in_the_order_they_draw_and_what_blends_or_lies_on_a_surface_casts_no_shadow(
) {
    use WorldBucket::{Blend, Cutout, Decal, Emissive, Opaque};
    let mut buckets = [Blend, Decal, Emissive, Cutout, Opaque];
    buckets.sort();
    assert_eq!(buckets, [Opaque, Cutout, Emissive, Decal, Blend]);
    assert_eq!(buckets.len(), WorldBucket::COUNT);
    assert_eq!(
        buckets.map(WorldBucket::casts),
        [true, true, true, false, false]
    );
    assert_eq!(WorldBucket::default(), Opaque);
}

#[test]
fn the_cascades_hold_what_the_camera_sees_each_in_its_own_tile_and_their_texels_stay_put_in_the_world(
) {
    use crate::game_shaders::engine::EngineConstants;
    use crate::game_shaders::shadow::TILE;
    let engine = EngineConstants {
        sun_direction: Vec3::new(0.4, -0.2, 0.8).normalize(),
        ..EngineConstants::default()
    };
    let camera = OrbitCamera {
        target: Vec3::new(120.0, -75.0, 20.0),
        yaw: 0.7,
        pitch: 0.3,
        distance: 10.0,
        ..OrbitCamera::default()
    };
    let aspect = 16.0 / 9.0;
    let eye = camera.eye();
    let forward = (camera.target - eye).normalize();
    let cascades =
        shadow::Cascades::about(&engine, &camera, aspect, (1.0, shadow::ShadowFocus::FREE))
            .expect("the sun shines");
    let values = cascades.values(eye);
    let shared = &values["gCSMShaderVars_shared"];
    assert_eq!(shared.len(), 48);
    let register = |at: usize| Vec3::new(shared[at * 4], shared[at * 4 + 1], shared[at * 4 + 2]);
    // Where a point of the world is in the tile of `cascade`, as the receiving pass works it out: its place along each axis from the eye, scaled and moved
    let read = |point: Vec3, cascade: usize| {
        let from_eye = point - eye;
        let place = from_eye.x * register(0) + from_eye.y * register(1) + from_eye.z * register(2);
        place * register(4 + cascade) + register(8 + cascade)
    };
    let views = cascades.views();
    // A point on the view's axis in each cascade's slice, and one beside it within the view
    let side = camera.view().inverse().x_axis.truncate();
    let ends = shadow::ends(1.0, shadow::ShadowFocus::FREE);
    let middles = std::array::from_fn::<f32, { shadow::CASCADES }, _>(|at| match at {
        0 => ends[0] * 0.6,
        _ => (ends[at - 1] + ends[at]) * 0.5,
    });
    for (cascade, ahead) in middles.into_iter().enumerate() {
        for point in [
            eye + forward * ahead,
            eye + forward * ahead + side * (ahead * 0.5),
        ] {
            let at = read(point, cascade);
            assert!(at.x.abs().max(at.y.abs()) < 0.49, "{cascade} {at}");
            // The pass takes the first tile a pixel lies across, whatever its depth: so its depth is one every tile holds
            for tile in 0..shadow::CASCADES {
                let deep = read(point, tile).z;
                assert!(deep > 0.0 && deep < 1.0, "{cascade} {tile} {deep}");
                assert!((deep - at.z).abs() < 0.002, "{cascade} {tile} {deep}");
            }
            // And the instances are drawn into the tile where the pass reads them: across from the left, down from the top, a little nearer the sun than it compares with
            let clip = views[cascade] * point.extend(1.0);
            let drawn = Vec3::new(clip.x * 0.5 + 0.5, (1.0 - clip.y) * 0.5, clip.z);
            assert!((clip.w - 1.0).abs() < 1e-6);
            assert!((drawn.x - (at.x + 0.5)).abs() < 1e-3, "{drawn} {at}");
            assert!((drawn.y - (at.y + 0.5)).abs() < 1e-3, "{drawn} {at}");
            assert!((0.0..0.01).contains(&(drawn.z - at.z)), "{drawn} {at}");
        }
    }
    // The shadow is whole until the last tenth of the reach, and gone at its end
    let fade = |ahead: f32| (ahead * shared[3] + shared[7]).clamp(0.0, 1.0);
    assert!(fade(SHADOW_FAR * 0.9) > 0.999);
    assert!(fade(SHADOW_FAR * 0.95) > 0.4 && fade(SHADOW_FAR * 0.95) < 0.6);
    assert!(fade(SHADOW_FAR) < 0.001);
    // No fade by anything but the distance
    assert_eq!(shared[11], 0.0);

    // A camera that moved a little and turned: a point of the world is a whole number of texels from where it was in each tile
    let moved = OrbitCamera {
        target: camera.target + Vec3::new(0.37, -0.21, 0.13),
        yaw: 0.9,
        ..camera.clone()
    };
    let after = shadow::Cascades::about(&engine, &moved, aspect, (1.0, shadow::ShadowFocus::FREE))
        .expect("the sun shines");
    let point = Vec3::new(118.0, -70.0, 12.0);
    for (cascade, (was, is)) in views.iter().zip(after.views()).enumerate() {
        let texels = |view: Mat4| (view * point.extend(1.0)).truncate() * (TILE as f32 * 0.5);
        let step = texels(is) - texels(*was);
        for across in [step.x, step.y] {
            assert!((across - across.round()).abs() < 0.05, "{cascade} {step}");
        }
    }

    // A sun that gives no light casts none
    let dark = EngineConstants {
        sun_colour: Vec3::ZERO,
        ..engine
    };
    assert!(
        shadow::Cascades::about(&dark, &camera, aspect, (1.0, shadow::ShadowFocus::FREE)).is_none()
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn each_instance_of_a_mesh_is_drawn_where_its_matrix_puts_it_in_its_tint_and_nowhere_else() {
    let test = "each_instance_of_a_mesh_is_drawn_where_its_matrix_puts_it";
    for (device, queue) in devices(test) {
        let mut renderer =
            ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
        renderer.set_background(BACKGROUND);
        let camera = overhead();
        let (red_at, green_at, empty_at) = (
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, 0.5, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
        );
        let instances = [
            WorldInstance::of(Mat4::from_translation(red_at), RED),
            WorldInstance::of(Mat4::from_translation(green_at), [0.0, 1.0, 0.0, 1.0]),
        ];
        let material = plain(&device, &renderer, WorldBucket::Opaque);
        let draws = |device: &wgpu::Device, instances| {
            vec![WorldDraw {
                mesh: square(device, 0.3, material.clone()),
                instances,
            }]
        };
        assert!(!renderer.has_world());
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let nothing = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
        let background = nothing[0];
        assert!(nothing.iter().all(|pixel| *pixel == background));

        renderer.set_world(&device, &queue, &instances, draws(&device, 0..2));
        assert!(renderer.has_world());
        assert_eq!(renderer.world_instances(), 2);
        // The draws go out by one call each batch only where the device issues an indirect draw from an instance other than the first
        let several = device
            .features()
            .contains(wgpu::Features::INDIRECT_FIRST_INSTANCE);
        assert_eq!(renderer.world.indirect.is_some(), several);
        for mode in [
            ShadingMode::Solid,
            ShadingMode::Material,
            ShadingMode::Shaded,
        ] {
            let image = frame(&device, &queue, &renderer, &camera, mode);
            let (red, green) = (
                image[pixel_of(&camera, red_at)],
                image[pixel_of(&camera, green_at)],
            );
            // A dielectric's grey highlight is all the other channels hold
            assert!(
                red[0] > 100 && red[1] <= HIGHLIGHT && red[2] <= HIGHLIGHT,
                "{mode:?} {red:?}"
            );
            assert!(
                green[1] > 100 && green[0] <= HIGHLIGHT && green[2] <= HIGHLIGHT,
                "{mode:?} {green:?}"
            );
            assert_eq!(image[pixel_of(&camera, empty_at)], background, "{mode:?}");
            assert_eq!(image[pixel_of(&camera, Vec3::ZERO)], background, "{mode:?}");
            // Two squares of 0.6 seen from five metres: the instances cover little of the frame
            let covered = image.iter().filter(|pixel| **pixel != background).count();
            assert!((40..400).contains(&covered), "{mode:?} {covered}");
        }

        // A wireframe shows no surface, and a run that ends past the instances draws the ones there are
        let wires = frame(&device, &queue, &renderer, &camera, ShadingMode::Wireframe);
        assert!(wires.iter().all(|pixel| *pixel == background));
        renderer.set_world(&device, &queue, &instances[..1], draws(&device, 0..9));
        let one = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
        assert_ne!(one[pixel_of(&camera, red_at)], background);
        assert_eq!(one[pixel_of(&camera, green_at)], background);
        // A run of no instance and a mesh of no index are no draw at all
        let whole = square(&device, 0.3, material.clone());
        let (vertices, indices) = (whole.vertices.clone(), whole.indices.clone());
        let mut none = draws(&device, 1..1);
        none.push(WorldDraw {
            mesh: Arc::new(WorldMesh::shared(
                vertices,
                indices,
                0,
                0,
                0,
                material.clone(),
            )),
            instances: 0..1,
        });
        renderer.set_world(&device, &queue, &instances, none);
        assert!(!renderer.has_world());

        renderer.set_world(&device, &queue, &[], Vec::new());
        assert!(!renderer.has_world());
        let emptied = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
        assert!(emptied.iter().all(|pixel| *pixel == background));
        assert!(
            pollster::block_on(validation.pop()).is_none(),
            "the device accepted every draw"
        );
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn meshes_that_are_ranges_of_one_pair_of_buffers_draw_their_own_range_and_go_out_together_while_their_material_is_one(
) {
    let test = "meshes_that_are_ranges_of_one_pair_of_buffers";
    for (device, queue) in devices(test) {
        let mut renderer =
            ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
        renderer.set_background(BACKGROUND);
        let camera = overhead();
        let places = [[-1.0, -1.0], [1.0, 0.5], [-1.0, 1.0]];
        // One page of vertices and one of indices: three squares, the indices of each counted from its own first vertex
        let vertices: Vec<MeshVertex> = places.iter().flat_map(|at| corners(*at, 0.3)).collect();
        let indices: Vec<u32> = places.iter().flat_map(|_| SQUARE).collect();
        let page = |contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents,
                usage,
            })
        };
        let vertex_page = page(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
        let index_page = page(bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);
        let opaque = plain(&device, &renderer, WorldBucket::Opaque);
        let blue_map = texel(&device, &queue, [0, 0, 255, 255]);
        let mapped =
            renderer.world_material(&device, Some(&blue_map), None, None, WorldBucket::Opaque);
        // A material is its maps and its bucket: a clone is the same, and so is another made of the same maps while the first is alive
        assert!(opaque == plain(&device, &renderer, WorldBucket::Opaque));
        assert_eq!(opaque.key(), opaque.clone().key());
        assert!(opaque != plain(&device, &renderer, WorldBucket::Cutout));
        assert!(
            mapped
                == renderer.world_material(&device, Some(&blue_map), None, None, mapped.bucket())
        );
        let red_map = texel(&device, &queue, [255, 0, 0, 255]);
        assert!(
            mapped != renderer.world_material(&device, Some(&red_map), None, None, mapped.bucket())
        );
        assert_ne!(mapped.key(), opaque.key());
        let part = |nth: u32, material: &WorldMaterial| WorldDraw {
            mesh: Arc::new(WorldMesh::shared(
                vertex_page.clone(),
                index_page.clone(),
                nth as i32 * 4,
                nth * 6,
                6,
                material.clone(),
            )),
            instances: nth..nth + 1,
        };
        let instances =
            [RED, [0.0, 1.0, 0.0, 1.0], WHITE].map(|tint| WorldInstance::of(Mat4::IDENTITY, tint));
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        renderer.set_world(
            &device,
            &queue,
            &instances,
            vec![part(0, &opaque), part(1, &opaque), part(2, &mapped)],
        );
        // The first two are one call; the third samples other maps, which the cast of a shadow has no use for
        assert_eq!(renderer.world.batches, [0..2, 2..3]);
        assert_eq!(
            renderer.world.cast_batches,
            std::iter::once(0..3).collect::<Vec<_>>()
        );
        let image = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
        let at = |place: [f32; 2]| image[pixel_of(&camera, Vec3::new(place[0], place[1], 0.0))];
        let (red, green, blue) = (at(places[0]), at(places[1]), at(places[2]));
        let dim = |c: u8| c <= HIGHLIGHT;
        assert!(red[0] > 100 && dim(red[1]) && dim(red[2]), "{red:?}");
        assert!(
            green[1] > 100 && dim(green[0]) && dim(green[2]),
            "{green:?}"
        );
        assert!(blue[2] > 100 && dim(blue[0]) && dim(blue[1]), "{blue:?}");
        let covered = image.iter().filter(|pixel| **pixel != image[0]).count();
        assert!((60..600).contains(&covered), "{covered}");

        // Handed over in another order, the draws are put in the order of their buckets, and what does not cast is no part of the cast
        let decal = plain(&device, &renderer, WorldBucket::Decal);
        let cutout = plain(&device, &renderer, WorldBucket::Cutout);
        renderer.set_world(
            &device,
            &queue,
            &instances,
            vec![part(0, &decal), part(1, &cutout), part(2, &opaque)],
        );
        let order: Vec<WorldBucket> = (renderer.world.draws.iter())
            .map(|draw| draw.mesh.material().bucket())
            .collect();
        assert_eq!(
            order,
            [WorldBucket::Opaque, WorldBucket::Cutout, WorldBucket::Decal]
        );
        assert_eq!(renderer.world.batches, [0..1, 1..2, 2..3]);
        assert_eq!(renderer.world.cast_batches, [0..1, 1..2]);
        let image = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
        let covered = image.iter().filter(|pixel| **pixel != image[0]).count();
        assert!((60..600).contains(&covered), "{covered}");
        assert!(
            pollster::block_on(validation.pop()).is_none(),
            "the device accepted every draw"
        );
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_frame_of_more_instances_than_the_device_s_largest_buffer_holds_draws_every_one_and_is_accepted(
) {
    let test = "a_frame_of_more_instances_than_the_device_s_largest_buffer_holds";
    if gpu(test).is_none() {
        return;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: crate::instance::software_asked(),
        ..Default::default()
    }))
    .expect("the adapter the first device came from");
    let most = 1u64 << 20;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("world-test-small-buffers"),
        required_limits: wgpu::Limits {
            max_buffer_size: most,
            ..wgpu::Limits::default()
        },
        ..Default::default()
    }))
    .expect("a device of smaller buffers");
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    let fit = most as usize / std::mem::size_of::<WorldInstance>();
    // Two buffers and a part of a third.
    let instances = vec![WorldInstance::of(Mat4::IDENTITY, WHITE); 2 * fit + fit / 2];
    let square = square(&device, 1.0, plain(&device, &renderer, WorldBucket::Opaque));
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    renderer.set_world(
        &device,
        &queue,
        &instances,
        vec![WorldDraw {
            mesh: square,
            instances: 0..instances.len() as u32,
        }],
    );
    frame(
        &device,
        &queue,
        &renderer,
        &overhead(),
        ShadingMode::Material,
    );
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every buffer and draw"
    );
    assert_eq!(renderer.world_instances() as usize, instances.len());
    let calls = renderer.world_calls();
    assert_eq!(
        calls.draws, 3,
        "the one run is cut at the end of each buffer"
    );
    assert_eq!(
        calls.triangles,
        2 * instances.len() as u64,
        "every instance draws"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_fade_leaves_its_share_of_the_pixels_of_an_instance_and_a_cutout_only_the_texels_that_cover() {
    let test = "a_fade_leaves_its_share_of_the_pixels_of_an_instance";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    renderer.set_background(BACKGROUND);
    let camera = overhead();
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let material = |bucket, alpha: u8| {
        let colour = texel(&device, &queue, [255, 255, 255, alpha]);
        renderer.world_material(&device, Some(&colour), None, None, bucket)
    };
    let (opaque, thin) = (
        material(WorldBucket::Opaque, 255),
        material(WorldBucket::Opaque, 40),
    );
    let (cut_whole, cut_thin) = (
        material(WorldBucket::Cutout, 255),
        material(WorldBucket::Cutout, 40),
    );
    let blend_thin = material(WorldBucket::Blend, 40);
    // How many pixels a square of one metre a side covers, drawn with `material` and faded to `fade`
    let mut covered = |material: WorldMaterial, fade: f32| {
        renderer.set_world(
            &device,
            &queue,
            &[WorldInstance::of(Mat4::IDENTITY, [1.0, 1.0, 1.0, fade])],
            vec![WorldDraw {
                mesh: square(&device, 0.5, material),
                instances: 0..1,
            }],
        );
        let image = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
        image.iter().filter(|pixel| **pixel != image[0]).count()
    };
    let whole = covered(opaque.clone(), 1.0);
    assert!((60..400).contains(&whole), "{whole}");
    // The fade is no alpha: the pixels it leaves are drawn whole, in the share it states
    let half = covered(opaque.clone(), 0.5);
    assert!(
        half * 5 > whole * 2 && half * 5 < whole * 3,
        "{half} of {whole}"
    );
    let quarter = covered(opaque.clone(), 0.25);
    assert!(quarter > 0 && quarter < half, "{quarter} of {whole}");
    assert_eq!(covered(opaque, 0.0), 0);
    // What is opaque draws whatever its coverage, a cutout only what covers, and both fade alike
    assert_eq!(covered(thin, 1.0), whole);
    assert_eq!(covered(cut_whole.clone(), 1.0), whole);
    assert_eq!(covered(cut_thin, 1.0), 0);
    assert_eq!(covered(cut_whole, 0.5), half);
    // What blends shows through, and fades by the same pixels
    assert_eq!(covered(blend_thin.clone(), 1.0), whole);
    assert_eq!(covered(blend_thin, 0.5), half);
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every draw"
    );
}

/// Whether every channel of two pixels is within `tolerance` of the other's.
fn near(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= tolerance)
}

/// `x` from nought to one as a byte of a target.
fn unorm(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The specular word of a surface of `smoothness` whose reflectance at normal incidence is `f0`, as the lighting decodes it: an intensity of one, the exponent whose lobe is as wide as the rough lobe's undone of the lighting's expansion, and one less the reflectance.
fn specular_word(smoothness: f32, f0: f32) -> [u8; 4] {
    let alpha = ((1.0 - smoothness) * (1.0 - smoothness)).max(0.001);
    let n = (2.0 / (alpha * alpha) - 2.0).clamp(1.0, 8196.0);
    let stored = if n <= 1500.0 {
        n / 3.0
    } else {
        500.0 + (n - 1500.0) / 558.0
    };
    [255, unorm((stored / 512.0).sqrt()), unorm(1.0 - f0), 255]
}

/// Whether a specular word holds the exponent of `smoothness` and, as intensity times one less the fresnel word, the reflectance `f0`: what the view leaves of it whatever the intensity the view picks.
fn same_lobe(word: [u8; 4], smoothness: f32, f0: f32) -> bool {
    let intensity = (f32::from(word[0]) / 255.0).powi(2);
    let reflectance = intensity * (1.0 - f32::from(word[2]) / 255.0);
    word[1].abs_diff(specular_word(smoothness, f0)[1]) <= 2 && (reflectance - f0).abs() < 0.012
}

/// A square about the origin that reaches half a metre to each side, its vertices of the first colour `first` and the second `second`, drawn with `material`.
fn coloured_square(
    device: &wgpu::Device,
    first: [f32; 4],
    second: [f32; 4],
    material: WorldMaterial,
) -> Arc<WorldMesh> {
    let mut vertices = corners([0.0, 0.0], 0.5);
    // A material not of a model reads the colour as it is and the second colour from the depth bias
    for vertex in &mut vertices {
        vertex.colour = first;
        vertex.depth_bias = colour1(second.map(|c| (c * 255.0).round() as u8));
    }
    Arc::new(WorldMesh::new(device, &vertices, &SQUARE, material))
}

/// A material of `family` in `bucket` that samples each of `maps` in its slot, of the constants `constants`.
fn surface(
    device: &wgpu::Device,
    renderer: &ViewportRenderer,
    (family, bucket): (WorldFamily, WorldBucket),
    maps: &[(usize, wgpu::TextureView)],
    constants: WorldConstants,
) -> WorldMaterial {
    let mut slots = [None; WORLD_MAPS];
    for (at, view) in maps {
        slots[*at] = Some(view);
    }
    let surface = WorldSurface {
        family,
        bucket,
        maps: slots,
        constants,
    };
    renderer.world_surface(device, &surface)
}

/// The G-buffer at the middle of a deferred frame of `meshes` laid one over another about the origin, each one instance tinted `tint`, and the screen there.
fn shade(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut ViewportRenderer,
    meshes: &[(Arc<WorldMesh>, [f32; 4])],
) -> (Vec<[u8; 4]>, [u8; 4]) {
    let camera = overhead();
    let middle = (SIZE / 2 * SIZE + SIZE / 2) as usize;
    let instances: Vec<WorldInstance> = (meshes.iter())
        .map(|(_, tint)| WorldInstance::of(Mat4::IDENTITY, *tint))
        .collect();
    let draws = (meshes.iter().zip(0..))
        .map(|((mesh, _), at)| WorldDraw {
            mesh: Arc::clone(mesh),
            instances: at..at + 1,
        })
        .collect();
    renderer.set_world(device, queue, &instances, draws);
    let image = frame(device, queue, renderer, &camera, ShadingMode::Shaded);
    (words(device, queue, renderer, middle), image[middle])
}

/// A deferred viewport for one family's test, with no shadow to write the targets it reads, or none where the machine has no adapter or no pack.
fn family_frame(test: &str) -> Option<(wgpu::Device, wgpu::Queue, ViewportRenderer)> {
    let (device, queue) = gpu(test)?;
    let pack = real_pack(test)?;
    let mut renderer = deferred(&device, &queue, &pack, Vec3::new(0.3, 0.2, 0.9).normalize());
    renderer.set_shadows(false);
    Some((device, queue, renderer))
}

/// A viewport whose Shaded frame is the game's deferred one with no surface of its own, lit by a sun that stands towards `sun`.
fn deferred(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pack: &GamePack,
    sun: Vec3,
) -> ViewportRenderer {
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = ViewportRenderer::new(device, queue, format, SIZE, SIZE);
    renderer.set_background(BACKGROUND);
    renderer.set_game_sun(Some(sun));
    let report = renderer
        .prepare_deferred(device, queue, pack, &[])
        .expect("the deferred passes prepare");
    assert_eq!(report, (0, Vec::new()));
    renderer
}

/// The four targets of the G-buffer of the frame `renderer` drew last, at the pixel `at`.
fn words(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &ViewportRenderer,
    at: usize,
) -> Vec<[u8; 4]> {
    renderer
        .gbuffer_textures()
        .into_iter()
        .map(|target| {
            let read = read_back(device, queue, target, GBUFFER_FORMAT, SIZE);
            read.bytes.as_chunks::<4>().0[at]
        })
        .collect()
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_games_passes_light_an_instance_of_the_world_as_they_light_a_surface_of_the_default_effect() {
    use crate::game_shaders::lighting::Day;
    use crate::game_shaders::mesh::{Image, Material, Surface};
    use crate::game_shaders::synthetic::quad;
    let test = "the_games_passes_light_an_instance_of_the_world";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let Some(pack) = real_pack(test) else {
        return;
    };
    let camera = overhead();
    let middle = (SIZE / 2 * SIZE + SIZE / 2) as usize;
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // A deferred frame of `surfaces`, of one instance of a square tinted `tint` where there is one, under `day` and its sky where there is one: the four targets of its G-buffer and the screen at the middle, and the screen at a corner
    let shaded = |surfaces: &[(Surface, Material)], tint: Option<f32>, day: Option<Day>| {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let mut renderer = ViewportRenderer::new(&device, &queue, format, SIZE, SIZE);
        renderer.set_background(BACKGROUND);
        // One flat square shades nothing, and the pass that receives the shadow writes the target compared below
        renderer.set_shadows(false);
        let report = renderer
            .prepare_deferred(&device, &queue, &pack, surfaces)
            .expect("the deferred passes prepare");
        assert_eq!(report, (surfaces.len(), Vec::new()));
        assert!(renderer.has_deferred());
        if let Some(tint) = tint {
            let material = plain(&device, &renderer, WorldBucket::Opaque);
            renderer.set_world(
                &device,
                &queue,
                &[WorldInstance::of(Mat4::IDENTITY, [tint, tint, tint, 1.0])],
                vec![WorldDraw {
                    mesh: square(&device, 1.0, material),
                    instances: 0..1,
                }],
            );
        }
        if day.is_some() {
            renderer.set_day(day);
            renderer.set_sky_background(true);
            renderer.prepare_game_frame(&device, &queue);
        }
        let image = frame(&device, &queue, &renderer, &camera, ShadingMode::Shaded);
        let targets = words(&device, &queue, &renderer, middle);
        (targets, image[middle], image[0])
    };
    // The same square as a surface of the game's default effect, its texture of one grey
    let surface = |grey: u8| {
        let texture = Image {
            width: 1,
            height: 1,
            rgba: vec![grey, grey, grey, 255],
            srgb: false,
        };
        [(
            quad(0.0),
            Material {
                effect: "default".into(),
                textures: [("DiffuseSampler".to_string(), texture)].into(),
                ..Material::default()
            },
        )]
    };

    // A frame with no surface of its own and no world is the background, and its G-buffer is as it was cleared
    let (untouched, nothing, background) = shaded(&[], None, None);
    assert_eq!(untouched, [[0; 4]; 4]);
    assert!(near(nothing, background, 2), "{nothing:?} {background:?}");

    let (world, lit, corner) = shaded(&[], Some(1.0), None);
    let (game, game_lit, _) = shaded(&surface(255), None, None);
    // The albedo, the normal as half of it and a half, the specular word of a fully rough dielectric, and the whole of the natural ambient light
    let written = [
        [255, 255, 255, 255],
        [127, 127, 255, 0],
        specular_word(0.0, 0.04),
        [255, 0, 0, 255],
    ];
    for (target, want) in world.iter().zip(written) {
        assert!(near(*target, want, 1), "{world:?}");
    }
    // Which is what the game's own effect writes, but for the alpha it gives a normal straight up, the artificial light it takes and its matte specular word
    let (mut same, mut game_same) = (world.clone(), game.clone());
    for targets in [&mut same, &mut game_same] {
        targets[1][3] = 0;
        targets[3][1] = 0;
        targets[2] = [0; 4];
    }
    for (target, want) in same.iter().zip(game_same) {
        assert!(near(*target, want, 1), "{world:?} {game:?}");
    }
    assert!(
        lit[..3].iter().all(|c| *c > 30),
        "the world is lit: {lit:?}"
    );
    assert!(!near(lit, background, 8), "{lit:?} {background:?}");
    assert!(near(corner, background, 2), "{corner:?} {background:?}");
    // Lit as the game's surface, and a highlight on it
    let as_lit = |lit: [u8; 4], game: [u8; 4]| {
        let (lit, game) = (lit.map(u16::from), game.map(u16::from));
        (0..3).all(|c| lit[c] + 3 >= game[c] && lit[c] <= game[c] + 40)
    };
    assert!(as_lit(lit, game_lit), "{lit:?} {game_lit:?}");

    // The lighting squares the albedo: a tint of a quarter is stored as a half, as a texture of a half is, and lit as it
    let grey = 128.0 / 255.0;
    let (world, lit, _) = shaded(&[], Some(grey * grey), None);
    let (game, game_lit, _) = shaded(&surface(128), None, None);
    assert!(near(world[0], [128, 128, 128, 255], 1), "{world:?}");
    assert!(near(world[0], game[0], 1), "{world:?} {game:?}");
    assert!(as_lit(lit, game_lit), "{lit:?} {game_lit:?}");

    // Under a day, with its occlusion over the world and its sky round it, the frame of no surface is lit as well
    let mut keyframe = or_io::gta5::data::timecycle::Keyframe::default();
    keyframe.set("ssao_inten", 4.0);
    let day = Day {
        keyframe,
        ..Day::default()
    };
    let (_, lit, sky) = shaded(&[], Some(1.0), Some(day));
    assert!(lit[..3].iter().all(|c| *c > 30), "{lit:?}");
    assert!(!near(lit, sky, 8), "{lit:?} {sky:?}");
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every pass"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_maps_and_the_bucket_of_a_material_reach_the_g_buffer_in_the_words_the_lighting_reads() {
    let test = "the_maps_and_the_bucket_of_a_material_reach_the_g_buffer";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let Some(pack) = real_pack(test) else {
        return;
    };
    let camera = overhead();
    let middle = (SIZE / 2 * SIZE + SIZE / 2) as usize;
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut renderer = deferred(&device, &queue, &pack, Vec3::new(0.3, 0.2, 0.9).normalize());
    // The pass that receives the shadow writes a target read below
    renderer.set_shadows(false);
    let material = |maps: [Option<[u8; 4]>; 3], bucket| {
        let [colour, normal, specular] =
            maps.map(|map| map.map(|rgba| texel(&device, &queue, rgba)));
        renderer.world_material(
            &device,
            colour.as_ref(),
            normal.as_ref(),
            specular.as_ref(),
            bucket,
        )
    };
    let opaque = material([None; 3], WorldBucket::Opaque);
    let mapped = material(
        [
            Some([64, 64, 64, 255]),
            Some([204, 128, 255, 255]),
            Some([64, 0, 0, 255]),
        ],
        WorldBucket::Opaque,
    );
    let covering = material([Some([255; 4]), None, None], WorldBucket::Cutout);
    let clear = material([Some([255, 255, 255, 40]), None, None], WorldBucket::Cutout);
    let stain = material([Some([0, 255, 0, 128]), None, None], WorldBucket::Decal);
    let glass = material([Some([0, 0, 255, 128]), None, None], WorldBucket::Blend);
    // A frame of squares of one metre a side about the origin, each with its material and its tint, all in one plane: the G-buffer and the screen at the middle, and the screen at a corner
    let mut drawn = |squares: &[(&WorldMaterial, [f32; 4])]| {
        let instances: Vec<WorldInstance> = (squares.iter())
            .map(|(_, tint)| WorldInstance::of(Mat4::IDENTITY, *tint))
            .collect();
        let draws = (squares.iter().zip(0..))
            .map(|((material, _), at)| WorldDraw {
                mesh: square(&device, 0.5, (*material).clone()),
                instances: at..at + 1,
            })
            .collect();
        renderer.set_world(&device, &queue, &instances, draws);
        let image = frame(&device, &queue, &renderer, &camera, ShadingMode::Shaded);
        (
            words(&device, &queue, &renderer, middle),
            image[middle],
            image[0],
        )
    };

    // A colour of a quarter less its metal of a quarter is stored as its root; a normal map that leans three fifths along the texture's u, which runs along x here, leans the normal so; a material map of a quarter in red is that metal, and of nought in green a mirror's smoothness, its reflectance the metal's share of the colour over a dielectric's
    let (targets, lit, background) = drawn(&[(&mapped, WHITE)]);
    let (colour, metal) = (64.0_f32 / 255.0, 64.0_f32 / 255.0);
    let f0 = 0.04 * (1.0 - metal) + colour * metal;
    let stored = unorm((colour * (1.0 - metal)).sqrt());
    let want = [
        [stored, stored, stored, 255],
        [204, 128, 230, 0],
        specular_word(1.0, f0),
        [255, 0, 0, 255],
    ];
    for (target, want) in targets.iter().zip(want) {
        assert!(near(*target, want, 2), "{targets:?}");
    }
    assert!(!near(lit, background, 8), "{lit:?} {background:?}");

    // A cutout writes where its texels cover and nowhere else
    let (targets, ..) = drawn(&[(&covering, WHITE)]);
    assert!(near(targets[0], [255; 4], 1), "{targets:?}");
    let (targets, lit, background) = drawn(&[(&clear, WHITE)]);
    assert_eq!(targets, [[0; 4]; 4]);
    assert!(near(lit, background, 2), "{lit:?} {background:?}");

    // A decal of half a green on a red surface it lies in the plane of: the colour is half of each as the target stores them, and the alpha of every target and the ambient words are the surface's
    let (targets, ..) = drawn(&[(&stain, WHITE), (&opaque, RED)]);
    assert!(near(targets[0], [127, 128, 0, 255], 2), "{targets:?}");
    assert!(near(targets[1], [127, 127, 255, 0], 1), "{targets:?}");
    assert!(near(targets[2], specular_word(0.0, 0.04), 1), "{targets:?}");
    assert!(near(targets[3], [255, 0, 0, 255], 1), "{targets:?}");
    // On nothing it marks no pixel, and the frame shows none of it
    let (_, lit, background) = drawn(&[(&stain, WHITE)]);
    assert!(near(lit, background, 2), "{lit:?} {background:?}");

    // What blends is no part of the G-buffer: it is drawn over the lit frame, by its coverage
    let (under, under_lit, _) = drawn(&[(&opaque, RED)]);
    let (targets, lit, _) = drawn(&[(&glass, WHITE), (&opaque, RED)]);
    assert_eq!(targets, under);
    assert!(
        lit[2] > under_lit[2] + 20 && lit[0] + 20 < under_lit[0] && lit[0] > 20,
        "{lit:?} {under_lit:?}"
    );
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every pass"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_rough_surface_seen_at_grazing_hands_the_lighting_little_reflection_and_a_mirror_keeps_it_whole(
) {
    let test = "a_rough_surface_seen_at_grazing_hands_the_lighting_little_reflection";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let Some(pack) = real_pack(test) else {
        return;
    };
    let middle = (SIZE / 2 * SIZE + SIZE / 2) as usize;
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut renderer = deferred(&device, &queue, &pack, Vec3::new(0.3, 0.2, 0.9).normalize());
    renderer.set_shadows(false);
    let rough = renderer.world_material(&device, None, None, None, WorldBucket::Opaque);
    let smooth = texel(&device, &queue, [0, 0, 0, 255]);
    let mirror = renderer.world_material(&device, None, None, Some(&smooth), WorldBucket::Opaque);
    // The intensity and the fresnel word at the middle of a square of `material` seen by `camera`, as the lighting decodes them
    let mut seen = |material: &WorldMaterial, camera: &OrbitCamera| {
        let draws = vec![WorldDraw {
            mesh: square(&device, 0.5, material.clone()),
            instances: 0..1,
        }];
        renderer.set_world(
            &device,
            &queue,
            &[WorldInstance::of(Mat4::IDENTITY, WHITE)],
            draws,
        );
        frame(&device, &queue, &renderer, camera, ShadingMode::Shaded);
        let [r, _, fresnel, _] = words(&device, &queue, &renderer, middle)[2];
        ((f32::from(r) / 255.0).powi(2), f32::from(fresnel) / 255.0)
    };
    // Some eight degrees off the plane, n.v about a seventh
    let grazing = OrbitCamera {
        yaw: 0.0,
        pitch: 0.15,
        distance: 5.0,
        ..OrbitCamera::default()
    };
    let (rough_above, _) = seen(&rough, &overhead());
    let (rough_grazing, fresnel) = seen(&rough, &grazing);
    let (mirror_grazing, _) = seen(&mirror, &grazing);
    assert!(rough_above > 0.98, "{rough_above}");
    // The lighting weighs the reflection by intensity * (f0' + (1 - f0') * (1 - n.v)^5): a rough dielectric's at grazing is a few hundredths, where intensity one would give a half
    let p = (1.0 - 0.15_f32.sin()).powi(5);
    let weight = rough_grazing * ((1.0 - fresnel) + fresnel * p);
    assert!(
        rough_grazing < 0.25 && weight < 0.1,
        "{rough_grazing} {fresnel} {weight}"
    );
    // A mirror's share there is near the plain Fresnel's, so its intensity stays near one
    assert!(mirror_grazing > 0.8, "{mirror_grazing}");
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every pass"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn an_instance_of_the_world_shades_the_one_below_it_within_the_reach_of_the_shadow_and_not_past_it()
{
    let test = "an_instance_of_the_world_shades_the_one_below_it";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let Some(pack) = real_pack(test) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // The light travels towards the negative x and down, half a unit across for each it falls
    let mut renderer = deferred(&device, &queue, &pack, Vec3::new(1.0, 0.0, 2.0).normalize());
    let material = plain(&device, &renderer, WorldBucket::Opaque);
    // How much of the sun the ground keeps where the top throws its shadow and where it does not, by the alpha of the G-buffer's self-shadow: a ground of eight units a side and, a unit over it, a top of one, all `scale` times as large, seen from five such units above, the top faded to `fade`
    let kept = |renderer: &mut ViewportRenderer, scale: f32, fade: f32| {
        let camera = overhead_from(5.0 * scale);
        let sized = Mat4::from_scale(Vec3::new(scale, scale, 1.0));
        let instances = [
            WorldInstance::of(sized, WHITE),
            WorldInstance::of(
                Mat4::from_translation(Vec3::Z * scale) * sized,
                [1.0, 1.0, 1.0, fade],
            ),
        ];
        let draws = vec![
            WorldDraw {
                mesh: square(&device, 4.0, material.clone()),
                instances: 0..1,
            },
            WorldDraw {
                mesh: square(&device, 0.5, material.clone()),
                instances: 1..2,
            },
        ];
        renderer.set_world(&device, &queue, &instances, draws);
        let _ = frame(&device, &queue, renderer, &camera, ShadingMode::Shaded);
        // The top, from x -0.5 to 0.5 a unit up, throws its shadow from -1 to 0, and hides the ground as far as -0.71 from this camera
        let at = |x: f32| {
            let pixel = pixel_of(&camera, Vec3::new(x * scale, 0.0, 0.0));
            words(&device, &queue, renderer, pixel)[2][3]
        };
        (at(-0.85), at(0.8))
    };
    let (shaded, sunlit) = kept(&mut renderer, 1.0, 1.0);
    assert!(shaded < 60, "shaded ground {shaded}");
    assert!(sunlit > 200, "sunlit ground {sunlit}");
    // A top faded to nothing casts nothing, and one faded to a half casts half a shadow
    let (shaded, _) = kept(&mut renderer, 1.0, 0.0);
    assert!(shaded > 200, "under a top that is not drawn {shaded}");
    let (half, _) = kept(&mut renderer, 1.0, 0.5);
    assert!((40..215).contains(&half), "under a top half drawn {half}");
    // With the shadow off the sun reaches everything
    renderer.set_shadows(false);
    let (shaded, sunlit) = kept(&mut renderer, 1.0, 1.0);
    assert!(shaded > 200 && sunlit > 200, "{shaded} {sunlit}");
    // Past the reach of the shadow there is none: the same scene a hundred times as large is seen from five hundred metres, further than the shadow is cast
    renderer.set_shadows(true);
    let (shaded, sunlit) = kept(&mut renderer, 100.0, 1.0);
    assert!(shaded > 200 && sunlit > 200, "{shaded} {sunlit}");
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every pass"
    );
}

/// Whether the device accepted everything since `scope` was pushed.
fn accepted(scope: wgpu::ErrorScopeGuard) -> bool {
    pollster::block_on(scope.pop()).is_none()
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_standard_material_is_metal_and_rough_by_its_material_a_and_tinted_by_the_palette_column_its_vertex_picks(
) {
    let Some((device, queue, mut renderer)) =
        family_frame("a_standard_material_is_metal_and_rough")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    // A palette of one row of two texels: red, then green
    let palette = || {
        uploaded_view(
            &device,
            &queue,
            2,
            1,
            &[255, 0, 0, 255, 0, 255, 0, 255],
            false,
        )
    };
    let material = |mask: u8, translucency_as_tint_mask: f32| {
        let maps = [
            (slot::colour(0), one([255; 4])),
            (slot::material_a(0), one([128, 64, 0, 0])),
            (slot::MATERIAL_B[0], one([0, 0, 128, mask])),
            (slot::TINT, palette()),
        ];
        let constants = WorldConstants {
            translucency_as_tint_mask,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Standard, WorldBucket::Opaque);
        surface(&device, &renderer, family, &maps, constants)
    };
    let (tinted, masked) = (material(255, 0.0), material(0, 1.0));
    let (metal, rough) = (128.0_f32 / 255.0, 64.0_f32 / 255.0);
    let kept = unorm((1.0 - metal).sqrt());
    // The third byte of the vertex's first colour of one picks the second column: the albedo is green, less its metal
    let second = coloured_square(
        &device,
        [1.0, 1.0, 1.0 / 255.0, 1.0],
        [1.0; 4],
        tinted.clone(),
    );
    let (targets, lit) = shade(&device, &queue, &mut renderer, &[(second, WHITE)]);
    assert!(near(targets[0], [0, kept, 0, 255], 2), "{targets:?}");
    // The metal's reflectance is its colour's, the rest a dielectric's
    let f0 = 0.04 * (1.0 - metal) + metal;
    assert!(
        near(targets[2], specular_word(1.0 - rough, f0), 2),
        "{targets:?}"
    );
    assert!(lit[1] > lit[0] && lit[1] > lit[2], "{lit:?}");
    // Of nought it picks the first, and the vertex's colour is no multiple of the albedo
    let first = coloured_square(&device, [0.0, 0.0, 0.0, 1.0], [1.0; 4], tinted);
    let (targets, _) = shade(&device, &queue, &mut renderer, &[(first, WHITE)]);
    assert!(near(targets[0], [kept, 0, 0, 255], 2), "{targets:?}");
    // Where material B's alpha masks the tint off, the albedo is the colour map's
    let off = coloured_square(&device, [1.0, 1.0, 1.0 / 255.0, 1.0], [1.0; 4], masked);
    let (targets, _) = shade(&device, &queue, &mut renderer, &[(off, WHITE)]);
    assert!(near(targets[0], [kept, kept, kept, 255], 2), "{targets:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_palette_row_is_256_colours_whatever_the_width_its_image_is_laid_in_and_one_past_the_last_is_the_last(
) {
    let Some((device, queue, mut renderer)) = family_frame("a_palette_row_is_256_colours") else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // Four rows of 256 in one row of 1024 texels, white but for the second column of each row: red, green, blue, black
    let mut bytes = vec![255u8; 1024 * 4];
    for (row, rgb) in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 0, 0]]
        .iter()
        .enumerate()
    {
        bytes[(row * 256 + 1) * 4..][..3].copy_from_slice(rgb);
    }
    let palette = uploaded_view(&device, &queue, 1024, 1, &bytes, false);
    // A row past the palette's fourth reads the fourth
    for (row, wanted) in [
        (1.0, [0, 255, 0, 255]),
        (2.0, [0, 0, 255, 255]),
        (9.0, [0, 0, 0, 255]),
    ] {
        let maps = [
            (slot::colour(0), texel(&device, &queue, [255; 4])),
            (slot::TINT, palette.clone()),
        ];
        let constants = WorldConstants {
            palette_row: row,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Standard, WorldBucket::Opaque);
        let material = surface(&device, &renderer, family, &maps, constants);
        let square = coloured_square(&device, [1.0, 1.0, 1.0 / 255.0, 1.0], [1.0; 4], material);
        let (targets, _) = shade(&device, &queue, &mut renderer, &[(square, WHITE)]);
        assert!(near(targets[0], wanted, 2), "row {row}: {targets:?}");
    }
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn foliage_reads_no_metal_in_its_material_a_red_and_keeps_a_dielectrics_reflectance() {
    let Some((device, queue, mut renderer)) = family_frame("foliage_reads_no_metal") else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    let leaf = |flags: u32| {
        let maps = [
            (slot::colour(0), one([255; 4])),
            (slot::material_a(0), one([255, 64, 0, 0])),
        ];
        let constants = WorldConstants {
            flags: WorldConstants::default().flags | flags,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Standard, WorldBucket::Opaque);
        surface(&device, &renderer, family, &maps, constants)
    };
    let (foliage, other) = (leaf(WorldConstants::FOLIAGE), leaf(0));
    let rough = 64.0_f32 / 255.0;
    let square = coloured_square(&device, [1.0; 4], [1.0; 4], foliage);
    let (targets, _) = shade(&device, &queue, &mut renderer, &[(square, WHITE)]);
    assert!(near(targets[0], [255, 255, 255, 255], 2), "{targets:?}");
    assert!(
        near(targets[2], specular_word(1.0 - rough, 0.04), 2),
        "{targets:?}"
    );
    // The same maps as any other material are all metal: a black albedo
    let square = coloured_square(&device, [1.0; 4], [1.0; 4], other);
    let (targets, _) = shade(&device, &queue, &mut renderer, &[(square, WHITE)]);
    assert!(near(targets[0], [0, 0, 0, 255], 2), "{targets:?}");
    // A leaf's vertex alpha is no coverage: a cutout leaf whose vertices hold 0 there is drawn, any other cut away
    let cutout = |flags: u32| {
        let maps = [
            (slot::colour(0), one([255; 4])),
            (slot::material_a(0), one([0, 64, 0, 0])),
        ];
        let constants = WorldConstants {
            flags: WorldConstants::default().flags | WorldConstants::MODEL_VERTEX | flags,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Standard, WorldBucket::Cutout);
        surface(&device, &renderer, family, &maps, constants)
    };
    let (leaf, plain) = (cutout(WorldConstants::FOLIAGE), cutout(0));
    let model_square = |material| {
        let mut vertices = corners([0.0, 0.0], 0.5);
        coloured(
            &mut vertices,
            (Some(&[[1.0, 1.0, 1.0, 0.0]; 4]), None),
            (None, None),
        );
        Arc::new(WorldMesh::new(&device, &vertices, &SQUARE, material))
    };
    let (targets, _) = shade(
        &device,
        &queue,
        &mut renderer,
        &[(model_square(leaf), WHITE)],
    );
    assert!(near(targets[0], [255, 255, 255, 255], 2), "{targets:?}");
    let (cut, _) = shade(
        &device,
        &queue,
        &mut renderer,
        &[(model_square(plain), WHITE)],
    );
    assert!(!near(cut[0], [255, 255, 255, 255], 2), "{cut:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_leaf_that_lets_more_than_half_the_sun_through_sets_the_backlit_flag_and_one_seen_from_the_sun_glows_not(
) {
    let Some((device, queue, mut renderer)) = family_frame("a_leaf_that_lets_more_than_half")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // Material A's red is one less the translucency: 0 lets all of it through, 200 a fifth, 255 none
    for (red, flags, alpha) in [
        (0u8, WorldConstants::FOLIAGE, 255u8),
        (200, WorldConstants::FOLIAGE, 128),
        (255, WorldConstants::FOLIAGE, 128),
        (0, 0, 255),
    ] {
        let maps = [
            (slot::colour(0), texel(&device, &queue, [255; 4])),
            (slot::material_a(0), texel(&device, &queue, [red, 64, 0, 0])),
        ];
        let constants = WorldConstants {
            flags: WorldConstants::default().flags | flags,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Standard, WorldBucket::Opaque);
        let material = surface(&device, &renderer, family, &maps, constants);
        let square = coloured_square(&device, [1.0; 4], [1.0; 4], material);
        let (targets, _) = shade(&device, &queue, &mut renderer, &[(square, WHITE)]);
        assert!(targets[3][3].abs_diff(alpha) <= 1, "red {red}: {targets:?}");
        // The camera above sees the leaf from the sun's side: its light through is all behind it
        assert_eq!(targets[3][2], 0, "red {red}: {targets:?}");
    }
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_ground_decal_whose_tint_is_masked_takes_it_only_as_far_as_its_material_a_alpha() {
    let Some((device, queue, mut renderer)) = family_frame("a_ground_decal_whose_tint_is_masked")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // A black tint, over a white decal: where the mask is nought the decal keeps its own colour
    for (alpha, wanted) in [(0u8, 255u8), (255, 0)] {
        let maps = [
            (slot::colour(0), texel(&device, &queue, [255; 4])),
            (
                slot::material_a(0),
                texel(&device, &queue, [0, 255, 128, alpha]),
            ),
            (slot::TINT, texel(&device, &queue, [0, 0, 0, 255])),
        ];
        let flags = WorldConstants::GROUND_BLEND | WorldConstants::TINT_MASKED;
        let constants = WorldConstants {
            flags: WorldConstants::default().flags | flags,
            tint_intensity: [1.0; 4],
            tint_blend: [1.0; 4],
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Decal, WorldBucket::Opaque);
        let material = surface(&device, &renderer, family, &maps, constants);
        // The first colour's third byte of nought picks the palette's first column
        let square = coloured_square(&device, [1.0, 1.0, 0.0, 1.0], [1.0; 4], material);
        let (targets, _) = shade(&device, &queue, &mut renderer, &[(square, WHITE)]);
        let grey = [wanted, wanted, wanted, 255];
        assert!(near(targets[0], grey, 2), "alpha {alpha}: {targets:?}");
    }
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_two_layer_material_shows_its_second_layer_by_the_control_map_and_the_vertexs_second_colour() {
    let Some((device, queue, mut renderer)) = family_frame("a_two_layer_material_shows_its_second")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    let material = |control: [u8; 4], flags: u32| {
        let maps = [
            (slot::colour(0), one([255, 0, 0, 255])),
            (slot::colour(1), one([0, 0, 255, 255])),
            // The first layer fully rough, the second a mirror
            (slot::material_a(0), one([0, 255, 0, 0])),
            (slot::material_a(1), one([0, 0, 0, 0])),
            (slot::WEIGHTS, one(control)),
        ];
        let constants = WorldConstants {
            flags,
            ..WorldConstants::default()
        };
        surface(
            &device,
            &renderer,
            (WorldFamily::TwoLayer, WorldBucket::Opaque),
            &maps,
            constants,
        )
    };
    let blue = [0, 0, 255, 0];
    let (none, whole, inverted) = (
        material([0; 4], 0),
        material(blue, 0),
        material(blue, WorldConstants::INVERT_CONTROL),
    );
    // With no control the second colour's red of a half shows half of the second layer
    let share = 128.0 / 255.0;
    let mut drawn = |material: WorldMaterial| {
        let mesh = coloured_square(&device, [1.0; 4], [share, 1.0, 1.0, 1.0], material);
        shade(&device, &queue, &mut renderer, &[(mesh, WHITE)]).0
    };
    let half = [unorm((1.0 - share).sqrt()), 0, unorm(share.sqrt()), 255];
    let targets = drawn(none);
    assert!(near(targets[0], half, 2), "{targets:?}");
    assert!(
        near(targets[2], specular_word(share, 0.04), 2),
        "{targets:?}"
    );
    // The control map's blue, which the filter takes, shows the whole of it
    let targets = drawn(whole);
    assert!(near(targets[0], [0, 0, 255, 255], 2), "{targets:?}");
    assert!(near(targets[2], specular_word(1.0, 0.04), 2), "{targets:?}");
    // And taken from one, none of what the control map gives
    let targets = drawn(inverted);
    assert!(near(targets[0], half, 2), "{targets:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_four_layer_material_blends_its_layers_by_the_lookup_or_the_vertex_and_lets_the_highest_show() {
    let Some((device, queue, mut renderer)) =
        family_frame("a_four_layer_material_blends_its_layers")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    // Red, green, blue and white layers, each a mirror at the height its material A's blue states
    let material = |heights: [u8; 4]| {
        let colours = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255; 4],
        ];
        let mut maps = vec![(slot::WEIGHTS, one([128, 128, 0, 0]))];
        for (layer, (colour, height)) in colours.into_iter().zip(heights).enumerate() {
            maps.push((slot::colour(layer), one(colour)));
            maps.push((slot::material_a(layer), one([0, 0, height, 255])));
        }
        let family = (WorldFamily::FourLayer, WorldBucket::Opaque);
        surface(&device, &renderer, family, &maps, WorldConstants::default())
    };
    let (level, lower) = (material([255; 4]), material([255, 64, 255, 255]));
    let mut drawn = |material: &WorldMaterial, first: [f32; 4]| {
        let mesh = coloured_square(&device, first, [1.0; 4], material.clone());
        shade(&device, &queue, &mut renderer, &[(mesh, WHITE)]).0
    };
    // The lookup map weighs the first two layers alike, and where they stand as high they share the pixel
    let lookup = [1.0, 1.0, 1.0, 0.0];
    let targets = drawn(&level, lookup);
    let half = unorm(0.5f32.sqrt());
    assert!(near(targets[0], [half, half, 0, 255], 2), "{targets:?}");
    assert!(near(targets[2], specular_word(1.0, 0.04), 2), "{targets:?}");
    // Where the second stands lower the first covers it
    let targets = drawn(&lower, lookup);
    assert!(near(targets[0], [255, 0, 0, 255], 2), "{targets:?}");
    // A vertex whose first colour has an alpha of one weighs the layers itself: blue is the third
    let targets = drawn(&level, [0.0, 0.0, 1.0, 1.0]);
    assert!(near(targets[0], [0, 0, 255, 255], 2), "{targets:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_decal_blends_each_word_of_the_g_buffer_by_its_own_opacity_and_leaves_the_ambient_words() {
    let Some((device, queue, mut renderer)) = family_frame("a_decal_blends_each_word") else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    let under = plain(&device, &renderer, WorldBucket::Opaque);
    let decal = |opacity: [f32; 4]| {
        let maps = [
            (slot::colour(0), one([0, 255, 0, 255])),
            (slot::normal(0), one([204, 128, 255, 255])),
            // A mirror, so its specular word differs from the rough surface's under it
            (slot::material_a(0), one([0, 0, 0, 0])),
        ];
        let constants = WorldConstants {
            opacity,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Decal, WorldBucket::Decal);
        surface(&device, &renderer, family, &maps, constants)
    };
    let (colour_only, normal_only) = (decal([1.0, 0.0, 0.5, 0.0]), decal([0.0, 1.0, 1.0, 1.0]));
    let mut drawn = |decal: WorldMaterial| {
        let meshes = [
            (square(&device, 0.5, decal), WHITE),
            (square(&device, 0.5, under.clone()), RED),
        ];
        shade(&device, &queue, &mut renderer, &meshes).0
    };
    // Its colour whole, its normal not at all, and its specular word half over the surface's, whose alpha it keeps
    let targets = drawn(colour_only);
    assert!(near(targets[0], [0, 255, 0, 255], 2), "{targets:?}");
    assert!(near(targets[1], [127, 127, 255, 0], 1), "{targets:?}");
    let (mirror, rough) = (specular_word(1.0, 0.04), specular_word(0.0, 0.04));
    let mut half: [u8; 4] =
        std::array::from_fn(|c| ((u16::from(mirror[c]) + u16::from(rough[c])) / 2) as u8);
    half[3] = 255;
    assert!(near(targets[2], half, 2), "{targets:?}");
    assert!(near(targets[3], [255, 0, 0, 255], 1), "{targets:?}");
    // Its normal whole and its colour not at all: the surface's colour, the decal's lean
    let targets = drawn(normal_only);
    assert!(near(targets[0], [255, 0, 0, 255], 2), "{targets:?}");
    assert!(near(targets[1], [204, 128, 230, 0], 2), "{targets:?}");
    assert!(near(targets[3], [255, 0, 0, 255], 1), "{targets:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_glass_lies_over_the_lit_frame_by_its_coverage_and_a_forward_glow_adds_its_light_without_covering(
) {
    let Some((device, queue, mut renderer)) = family_frame("a_glass_lies_over_the_lit_frame")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    let under = plain(&device, &renderer, WorldBucket::Opaque);
    let blended = |family: WorldFamily, colour: [u8; 4]| {
        let maps = [(slot::colour(0), one(colour))];
        let family = (family, WorldBucket::Blend);
        surface(&device, &renderer, family, &maps, WorldConstants::default())
    };
    let glass = blended(WorldFamily::Glass, [0, 0, 255, 128]);
    let glow = blended(WorldFamily::Emissive, [0, 255, 0, 0]);
    let mut drawn = |over: Option<&WorldMaterial>| {
        let mut meshes = vec![(square(&device, 0.5, under.clone()), RED)];
        meshes.extend(over.map(|over| (square(&device, 0.5, over.clone()), WHITE)));
        shade(&device, &queue, &mut renderer, &meshes)
    };
    let (words_under, lit_under) = drawn(None);
    // Half a blue over the red: no word of the G-buffer changes, and the screen turns from red to blue
    let (targets, lit) = drawn(Some(&glass));
    assert_eq!(targets, words_under);
    assert!(
        lit[2] > lit_under[2] + 20 && lit[0] + 20 < lit_under[0] && lit[0] > 20,
        "{lit:?} {lit_under:?}"
    );
    // A glow of no coverage adds its green and takes nothing of the red
    let (targets, lit) = drawn(Some(&glow));
    assert_eq!(targets, words_under);
    assert!(lit[1] > lit_under[1] + 20, "{lit:?} {lit_under:?}");
    assert!(lit[0].abs_diff(lit_under[0]) <= 3, "{lit:?} {lit_under:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn an_emissive_map_lights_the_surface_by_its_strength_its_night_value_and_the_vertexs_second_colour(
) {
    let Some((device, queue, mut renderer)) = family_frame("an_emissive_map_lights_the_surface")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    let glowing = |emissive_night: f32| {
        let maps = [(slot::EMISSIVE, one([255; 4]))];
        let constants = WorldConstants {
            emissive_night,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Emissive, WorldBucket::Emissive);
        surface(&device, &renderer, family, &maps, constants)
    };
    let matte = plain(&device, &renderer, WorldBucket::Opaque);
    let (by_day, by_night) = (glowing(-1.0), glowing(3.0));
    let grey = [0.5, 0.5, 0.5, 1.0];
    let mut drawn = |material: &WorldMaterial, second: [f32; 4]| {
        let mesh = coloured_square(&device, [1.0; 4], second, material.clone());
        shade(&device, &queue, &mut renderer, &[(mesh, grey)])
    };
    let (plain_words, plain_lit) = drawn(&matte, [1.0; 4]);
    // A map of one at a strength of one, taking the whole of the surface's colour: as bright as the surface's colour, a sixteenth of the most the word holds
    let (targets, lit) = drawn(&by_day, [1.0; 4]);
    assert!(near(targets[3], [255, 0, 16, 255], 1), "{targets:?}");
    assert_eq!(targets[..3], plain_words[..3]);
    assert!(
        (0..3).all(|c| lit[c] >= plain_lit[c]) && lit != plain_lit,
        "{lit:?} {plain_lit:?}"
    );
    // With no day the night emissives are on, and a night value of three is the strength
    let (targets, _) = drawn(&by_night, [1.0; 4]);
    assert!(near(targets[3], [255, 0, 48, 255], 1), "{targets:?}");
    // A vertex whose second colour is black gives no light
    let (targets, _) = drawn(&by_day, [0.0, 0.0, 0.0, 1.0]);
    assert!(near(targets[3], [255, 0, 0, 255], 1), "{targets:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_grass_blade_takes_its_instances_colour_as_far_as_its_tint_and_the_vertexs_second_red_allow() {
    let Some((device, queue, mut renderer)) =
        family_frame("a_grass_blade_takes_its_instances_colour")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let grass = |grass_tint: f32| {
        let constants = WorldConstants {
            grass_tint,
            ..WorldConstants::default()
        };
        let family = (WorldFamily::Grass, WorldBucket::Opaque);
        surface(&device, &renderer, family, &[], constants)
    };
    let (tinted, untinted) = (grass(1.0), grass(0.0));
    // The instance's colour, squared already: a quarter of red
    let colour = [0.25, 1.0, 1.0, 1.0];
    let mut drawn = |material: &WorldMaterial, red: f32| {
        let mesh = coloured_square(&device, [1.0; 4], [red, 1.0, 1.0, 1.0], material.clone());
        shade(&device, &queue, &mut renderer, &[(mesh, colour)]).0
    };
    let targets = drawn(&tinted, 1.0);
    assert!(near(targets[0], [128, 255, 255, 255], 2), "{targets:?}");
    // The vertex's second red of nought keeps the blade its own colour, and so does a tint of nought
    let targets = drawn(&tinted, 0.0);
    assert!(near(targets[0], [255; 4], 2), "{targets:?}");
    let targets = drawn(&untinted, 1.0);
    assert!(near(targets[0], [255; 4], 2), "{targets:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_terrain_lod_reads_its_material_green_as_a_smoothness_and_its_normal_map_whole_and_takes_no_tint(
) {
    let Some((device, queue, mut renderer)) =
        family_frame("a_terrain_lod_reads_its_material_green")
    else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let one = |rgba| texel(&device, &queue, rgba);
    let material = |family: WorldFamily| {
        let maps = [
            (slot::normal(0), one([204, 128, 128, 255])),
            (slot::material_a(0), one([0, 191, 0, 0])),
            (slot::TINT, one([255, 0, 0, 255])),
        ];
        surface(
            &device,
            &renderer,
            (family, WorldBucket::Opaque),
            &maps,
            WorldConstants::default(),
        )
    };
    let (terrain_lod, standard) = (
        material(WorldFamily::TerrainLod),
        material(WorldFamily::Standard),
    );
    let mut drawn = |material: WorldMaterial| {
        // The palette's first column, which a terrain LOD does not read
        let mesh = coloured_square(&device, [1.0, 1.0, 0.0, 1.0], [1.0; 4], material);
        shade(&device, &queue, &mut renderer, &[(mesh, WHITE)]).0
    };
    let green = 191.0 / 255.0;
    let terrain = drawn(terrain_lod);
    assert!(near(terrain[0], [255; 4], 2), "{terrain:?}");
    // Its normal leans most of the way off the view, so its intensity is the grazing one: only its lobe and its reflectance at normal incidence are its material's
    assert!(same_lobe(terrain[2], green, 0.04), "{terrain:?}");
    // Its blue is the normal's own z, so the lean along u is nearly the whole of it
    assert!(terrain[1][0] > 245, "{terrain:?}");
    // The standard family reads the same maps as roughness, a lean rebuilt from two channels, and the palette
    let standard = drawn(standard);
    assert!(near(standard[0], [255, 0, 0, 255], 2), "{standard:?}");
    assert!(same_lobe(standard[2], 1.0 - green, 0.04), "{standard:?}");
    assert!(near(standard[1], [204, 128, 230, 0], 2), "{standard:?}");
    assert!(accepted(scope), "the device accepted every pass");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_vertex_coloured_mesh_takes_its_albedo_from_its_first_colour_where_a_standard_one_reads_a_palette_column(
) {
    let Some((device, queue, mut renderer)) = family_frame("a_vertex_coloured_mesh") else {
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut drawn = |family: WorldFamily| {
        let family = (family, WorldBucket::Opaque);
        let material = surface(&device, &renderer, family, &[], WorldConstants::default());
        let mesh = coloured_square(&device, [1.0, 0.0, 0.0, 1.0], [1.0; 4], material);
        shade(&device, &queue, &mut renderer, &[(mesh, WHITE)]).0
    };
    let coloured = drawn(WorldFamily::VertexColoured);
    assert!(near(coloured[0], [255, 0, 0, 255], 2), "{coloured:?}");
    let standard = drawn(WorldFamily::Standard);
    assert!(near(standard[0], [255; 4], 2), "{standard:?}");
    assert!(accepted(scope), "the device accepted every pass");
}
