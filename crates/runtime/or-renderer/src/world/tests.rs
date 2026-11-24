use glam::{Mat4, Vec3};

use super::*;
use crate::game_shaders::frame::read_back;
use crate::game_shaders::test_pack::{gpu, real_pack};
use crate::viewport::{OrbitCamera, ShadingMode};

const SIZE: u32 = 64;

/// A square about the origin that reaches `half` to each side, facing up.
fn square(device: &wgpu::Device, half: f32) -> Arc<WorldMesh> {
    let corners = [[-half, -half], [half, -half], [half, half], [-half, half]];
    let vertices = corners.map(|c| MeshVertex::new([c[0], c[1], 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]));
    Arc::new(WorldMesh::new(device, &vertices, &[0, 1, 2, 0, 2, 3], None))
}

/// A camera five metres over the origin, looking down.
fn overhead() -> OrbitCamera {
    OrbitCamera {
        yaw: 0.0,
        pitch: 1.5,
        distance: 5.0,
        ..OrbitCamera::default()
    }
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
    assert_eq!(std::mem::size_of::<WorldInstance>(), 64);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn each_instance_of_a_mesh_is_drawn_where_its_matrix_puts_it_in_its_tint_and_nowhere_else() {
    let test = "each_instance_of_a_mesh_is_drawn_where_its_matrix_puts_it";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    renderer.set_background([0.1, 0.2, 0.3, 1.0]);
    let camera = overhead();
    let (red_at, green_at, empty_at) = (
        Vec3::new(-1.0, -1.0, 0.0),
        Vec3::new(1.0, 0.5, 0.0),
        Vec3::new(1.0, -1.0, 0.0),
    );
    let instances = [
        WorldInstance::of(Mat4::from_translation(red_at), [1.0, 0.0, 0.0, 1.0]),
        WorldInstance::of(Mat4::from_translation(green_at), [0.0, 1.0, 0.0, 1.0]),
    ];
    let draws = |device: &wgpu::Device| {
        vec![WorldDraw {
            mesh: square(device, 0.3),
            instances: 0..2,
        }]
    };
    assert!(!renderer.has_world());
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let nothing = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
    let background = nothing[0];
    assert!(nothing.iter().all(|pixel| *pixel == background));

    renderer.set_world(&device, &queue, &instances, draws(&device));
    assert!(renderer.has_world());
    assert_eq!(renderer.world_instances(), 2);
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
        assert!(
            red[0] > 100 && red[1] == 0 && red[2] == 0,
            "{mode:?} {red:?}"
        );
        assert!(
            green[1] > 100 && green[0] == 0 && green[2] == 0,
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
    renderer.set_world(
        &device,
        &queue,
        &instances[..1],
        vec![WorldDraw {
            mesh: square(&device, 0.3),
            instances: 0..9,
        }],
    );
    let one = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
    assert_ne!(one[pixel_of(&camera, red_at)], background);
    assert_eq!(one[pixel_of(&camera, green_at)], background);

    renderer.set_world(&device, &queue, &[], Vec::new());
    assert!(!renderer.has_world());
    let emptied = frame(&device, &queue, &renderer, &camera, ShadingMode::Material);
    assert!(emptied.iter().all(|pixel| *pixel == background));
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every draw"
    );
}

/// Whether every channel of two pixels is within `tolerance` of the other's.
fn near(a: [u8; 4], b: [u8; 4], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= tolerance)
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_games_passes_light_an_instance_of_the_world_as_they_light_a_surface_of_the_default_effect() {
    use crate::game_shaders::deferred::GBUFFER_FORMAT;
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
        renderer.set_background([0.1, 0.2, 0.3, 1.0]);
        // One flat square shades nothing, and the pass that receives the shadow writes the target compared below
        renderer.set_shadows(false);
        let report = renderer
            .prepare_deferred(&device, &queue, &pack, surfaces)
            .expect("the deferred passes prepare");
        assert_eq!(report, (surfaces.len(), Vec::new()));
        assert!(renderer.has_deferred());
        if let Some(tint) = tint {
            renderer.set_world(
                &device,
                &queue,
                &[WorldInstance::of(Mat4::IDENTITY, [tint, tint, tint, 1.0])],
                vec![WorldDraw {
                    mesh: square(&device, 1.0),
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
        let targets: Vec<[u8; 4]> = renderer
            .gbuffer_textures()
            .into_iter()
            .map(|target| {
                let read = read_back(&device, &queue, target, GBUFFER_FORMAT, SIZE);
                read.bytes.as_chunks::<4>().0[middle]
            })
            .collect();
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
    // The albedo, the normal as half of it and a half, a matte specular the sun reaches, and the whole of the natural ambient light
    let written = [
        [255, 255, 255, 255],
        [127, 127, 255, 0],
        [0, 0, 250, 255],
        [255, 0, 0, 255],
    ];
    for (target, want) in world.iter().zip(written) {
        assert!(near(*target, want, 1), "{world:?}");
    }
    // Which is what the game's own effect writes, but for the alpha it gives a normal straight up and the artificial light it takes
    let (mut same, mut game_same) = (world.clone(), game.clone());
    for targets in [&mut same, &mut game_same] {
        targets[1][3] = 0;
        targets[3][1] = 0;
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
    assert!(near(lit, game_lit, 3), "{lit:?} {game_lit:?}");

    // The lighting squares the albedo: a tint of a quarter is stored as a half, as a texture of a half is, and lit as it
    let grey = 128.0 / 255.0;
    let (world, lit, _) = shaded(&[], Some(grey * grey), None);
    let (game, game_lit, _) = shaded(&surface(128), None, None);
    assert!(near(world[0], [128, 128, 128, 255], 1), "{world:?}");
    assert!(near(world[0], game[0], 1), "{world:?} {game:?}");
    assert!(near(lit, game_lit, 3), "{lit:?} {game_lit:?}");

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
