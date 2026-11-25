use std::sync::Arc;

use glam::Vec3;
use wgpu::util::DeviceExt;

use super::shader::{bake_shader, CARDS_SHADER, MIP_SHADER};
use super::*;
use crate::game_shaders::frame::read_back;
use crate::game_shaders::test_pack::gpu;
use crate::viewport::{MeshVertex, OrbitCamera, ShadingMode};
use crate::world::WorldBucket;

const SIZE: u32 = 64;
const BACKGROUND: [f32; 4] = [0.1, 0.2, 0.3, 1.0];

#[test]
fn every_program_of_the_cards_parses_and_validates() {
    let bake = bake_shader();
    for (name, source) in [
        ("cards", CARDS_SHADER),
        ("bake", bake.as_str()),
        ("mip", MIP_SHADER),
    ] {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("the {name} program parses: {}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("the {name} program validates: {e:?}"));
    }
}

/// The three frames of the grid the vertex stage picks for the way `d`, and their weights: the program's arithmetic, line for line.
fn frames_of(d: Vec3) -> [((u32, u32), f32); 3] {
    let v = Vec3::new(d.x, d.y, ((14.0 / 15.0) * (d.z + 1.0 / 14.0)).max(0.0));
    let s = (v.x.abs() + v.y.abs() + v.z).max(1e-6);
    let (px, py) = (v.x / s, v.y / s);
    let (gu, gv) = (3.5 * (1.0 + px + py), 3.5 * (1.0 + py - px));
    let (c0, r0) = (gu.floor(), gv.floor());
    let (fu, fv) = (gu - c0, gv - r0);
    let cells = if fu + fv < 1.0 {
        [
            ((c0, r0), 1.0 - fu - fv),
            ((c0 + 1.0, r0), fu),
            ((c0, r0 + 1.0), fv),
        ]
    } else {
        [
            ((c0 + 1.0, r0 + 1.0), fu + fv - 1.0),
            ((c0, r0 + 1.0), 1.0 - fu),
            ((c0 + 1.0, r0), 1.0 - fv),
        ]
    };
    cells.map(|((c, r), w)| ((c.clamp(0.0, 7.0) as u32, r.clamp(0.0, 7.0) as u32), w))
}

#[test]
fn the_frame_a_way_is_drawn_from_is_the_one_baked_along_it() {
    for row in 0..CARD_FRAMES {
        for col in 0..CARD_FRAMES {
            let frames = frames_of(frame_direction(col, row));
            let weights: f32 = frames.iter().map(|(_, w)| w).sum();
            assert!((weights - 1.0).abs() < 1e-4, "{col} {row}: {frames:?}");
            let heaviest = frames
                .iter()
                .copied()
                .fold(frames[0], |a, b| if b.1 > a.1 { b } else { a });
            assert_eq!(heaviest.0, (col, row), "{frames:?}");
        }
    }
    // The middle of the grid looks down, and its edge along the horizon
    assert!(frame_direction(3, 4).z > 0.95);
    assert!(frame_direction(7, 0).z < 0.0);
}

#[test]
fn a_frames_axes_are_square_to_it_and_to_each_other_even_looking_straight_down() {
    for d in [Vec3::X, Vec3::new(0.3, -0.5, 0.8).normalize(), Vec3::Z] {
        let (right, up) = frame_axes(d, Vec3::X, Vec3::Z);
        for (a, b) in [(right, d), (up, d), (right, up)] {
            assert!(a.dot(b).abs() < 1e-5, "{d} {right} {up}");
        }
        assert!((right.length() - 1.0).abs() < 1e-5 && (up.length() - 1.0).abs() < 1e-5);
    }
    // Seen from the side, a frame's up is the model's up
    assert!(frame_axes(Vec3::X, Vec3::X, Vec3::Z).1.z > 0.99);
}

#[test]
fn a_frames_view_takes_the_sphere_to_the_whole_cell_and_its_depth_from_near_to_far() {
    let (centre, radius, d) = (Vec3::new(1.0, 2.0, 3.0), 2.0, Vec3::X);
    let view = bake::frame_view(centre, radius, d, frame_axes(d, Vec3::X, Vec3::Z));
    let at = |p: Vec3| view.project_point3(p);
    assert!(
        at(centre).distance(Vec3::new(0.0, 0.0, 0.5)) < 1e-5,
        "the centre is the cell's"
    );
    assert!(
        (at(centre + Vec3::X * radius).z).abs() < 1e-5,
        "the near side is nought"
    );
    assert!(
        (at(centre - Vec3::X * radius).z - 1.0).abs() < 1e-5,
        "the far side is one"
    );
    assert!(
        (at(centre + Vec3::Z * radius).y - 1.0).abs() < 1e-5,
        "up is the top of the cell"
    );
}

/// A cube of `half` a side about `centre`, its vertices white: a vertex's colour is no albedo.
fn cube(device: &wgpu::Device, half: f32, centre: Vec3) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let vertices: Vec<MeshVertex> = (0..8)
        .map(|at| {
            let corner = [0, 1, 2]
                .map(|axis| centre[axis] + if (at >> axis) & 1 == 1 { half } else { -half });
            MeshVertex::new_coloured(corner, [0.0, 0.0, 1.0], [0.0, 0.0], [1.0; 4])
        })
        .collect();
    cube_buffers(device, &vertices)
}

/// The same cube with its vertices packed as a world model's are, each first colour `first`.
fn model_cube(
    device: &wgpu::Device,
    half: f32,
    first: [f32; 4],
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let mut vertices: Vec<MeshVertex> = (0..8)
        .map(|at| {
            let corner = [0, 1, 2].map(|axis| if (at >> axis) & 1 == 1 { half } else { -half });
            MeshVertex::new_coloured(corner, [0.0, 0.0, 1.0], [0.0, 0.0], [1.0; 4])
        })
        .collect();
    let colours = [first; 8];
    crate::world::coloured(&mut vertices, (Some(&colours), None), (None, None));
    cube_buffers(device, &vertices)
}

/// A standard material of a model in `bucket`, its colour map `colour` and its palette `palette`.
fn model_material(
    renderer: &ViewportRenderer,
    device: &wgpu::Device,
    colour: &wgpu::TextureView,
    palette: Option<&wgpu::TextureView>,
    bucket: WorldBucket,
) -> crate::world::WorldMaterial {
    let mut maps = [None; crate::world::WORLD_MAPS];
    maps[crate::world::slot::colour(0)] = Some(colour);
    maps[crate::world::slot::TINT] = palette;
    let mut constants = crate::world::WorldConstants::default();
    constants.flags |= crate::world::WorldConstants::MODEL_VERTEX;
    let surface = crate::world::WorldSurface {
        family: crate::world::WorldFamily::Standard,
        bucket,
        maps,
        constants,
    };
    renderer.world_surface(device, &surface)
}

fn cube_buffers(
    device: &wgpu::Device,
    vertices: &[MeshVertex],
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let faces: [[u32; 4]; 6] = [
        [0, 1, 3, 2],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 3, 7, 5],
    ];
    let indices: Vec<u32> = faces
        .iter()
        .flat_map(|[a, b, c, d]| [*a, *b, *c, *a, *c, *d])
        .collect();
    let buffer = |label, contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
    };
    (
        buffer(
            "cube-vertices",
            bytemuck::cast_slice(vertices),
            wgpu::BufferUsages::VERTEX,
        ),
        buffer(
            "cube-indices",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        ),
        indices.len() as u32,
    )
}

/// A record at the least corner of its list's box, upright, of the most scale, unturned, of species `id`.
fn record(id: u16) -> [u8; 16] {
    let [lo, hi] = id.to_le_bytes();
    [
        0, 0, 0, 0, 0, 0, 128, 128, 255, 255, 255, 255, 255, 0, lo, hi,
    ]
}

fn list(records: &[[u8; 16]], fade: f32) -> CardBatch {
    CardBatch {
        min: [0.0; 3],
        size: [1.0; 3],
        scale_range: [1.0, 1.0],
        distance: 1000.0,
        fade,
        records: Arc::from(records),
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_card_draws_its_species_baked_frames_only_once_baked_and_while_its_list_fades_in() {
    let test = "a_card_draws_its_species_baked_frames";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    renderer.set_background(BACKGROUND);
    let camera = OrbitCamera {
        distance: 12.0,
        ..OrbitCamera::default()
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let radius = 3f32.sqrt();
    let species = |slot| CardSpecies {
        centre: [0.0; 3],
        radius,
        slot,
    };
    // The places of the red pixels, row by row, how many cards were drawn, and how many pixels are green.
    let reds = |renderer: &mut ViewportRenderer,
                camera: &OrbitCamera,
                lists: &[CardBatch],
                shown: &[u32]| {
        renderer.set_card_lists(&device, &queue, lists);
        renderer.show_card_lists(shown);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, camera, ShadingMode::Material);
        queue.submit([encoder.finish()]);
        let read = read_back(
            &device,
            &queue,
            renderer.colour_texture(),
            renderer.colour_format,
            SIZE,
        );
        let red: Vec<usize> = (read.bytes.as_chunks::<4>().0.iter().enumerate())
            .filter(|(_, p)| u16::from(p[0]) > 2 * u16::from(p[1].max(p[2])) && p[0] > 40)
            .map(|(at, _)| at)
            .collect();
        let green = (read.bytes.as_chunks::<4>().0.iter())
            .filter(|p| u16::from(p[1]) > 2 * u16::from(p[0].max(p[2])) && p[1] > 40)
            .count();
        (red, renderer.card_instances(), green)
    };
    let drawn = |renderer: &mut ViewportRenderer, lists: &[CardBatch], shown: &[u32]| {
        let (red, instances, _) = reds(renderer, &camera, lists, shown);
        (red.len(), instances)
    };
    let one = [list(&[record(0)], 1.0)];
    // Before its species is baked a card draws nothing
    renderer.set_card_species(&device, &queue, &[species(None)]);
    assert_eq!(drawn(&mut renderer, &one, &[0]), (0, 1));
    // The red is the colour map's alone: the vertices are white
    let red = crate::game_shaders::stand_in::constant_view(&device, &queue, [1.0, 0.0, 0.0, 1.0]);
    let (vertices, indices, count) = cube(&device, 1.0, Vec3::ZERO);
    let mesh = CardMesh {
        vertices: &vertices,
        indices: &indices,
        base_vertex: 0,
        first_index: 0,
        index_count: count,
        material: renderer.world_material(&device, Some(&red), None, None, WorldBucket::Opaque),
    };
    assert!(renderer.bake_card_slot(&device, &queue, 0, [0.0; 3], radius, &[mesh]));
    renderer.set_card_species(&device, &queue, &[species(Some(0))]);
    let (whole, instances) = drawn(&mut renderer, &one, &[0]);
    assert!(whole > 20 && whole < (SIZE * SIZE) as usize, "{whole}");
    assert_eq!(instances, 1);
    // A list not shown, faded out, or a record of a species there is none of, draws nothing
    assert_eq!(drawn(&mut renderer, &one, &[]).0, 0);
    assert_eq!(drawn(&mut renderer, &[list(&[record(0)], 0.0)], &[0]).0, 0);
    assert_eq!(drawn(&mut renderer, &[list(&[record(7)], 1.0)], &[0]).0, 0);
    // Records rewritten in place, their lists laid out as before, draw as they now stand: hidden, then back
    assert_eq!(
        drawn(&mut renderer, &[list(&[record(u16::MAX)], 1.0)], &[0]).0,
        0
    );
    assert_eq!(drawn(&mut renderer, &one, &[0]).0, whole);
    // Half a fade leaves about half the pixels, by the world's dither
    let half = drawn(&mut renderer, &[list(&[record(0)], 0.5)], &[0]).0;
    assert!(
        half * 5 > whole * 2 && half * 5 < whole * 3,
        "{half} of {whole}"
    );
    // Lists side by side are drawn as one run, and every record of them draws
    let two = [list(&[record(0)], 1.0), list(&[record(0), record(0)], 1.0)];
    assert_eq!(drawn(&mut renderer, &two, &[0, 1]).1, 3);
    assert_eq!(drawn(&mut renderer, &two, &[1]).1, 2);
    // A model off its centre is seen on the side of the card it stands on. Placed across the
    // view's diagonal, a frame chosen or laid out mirrored in either axis shows it in the middle.
    let off = Vec3::new(0.85, -0.85, 0.0);
    let (off_vertices, off_indices, off_count) = cube(&device, 0.3, off);
    let mesh = CardMesh {
        vertices: &off_vertices,
        indices: &off_indices,
        base_vertex: 0,
        first_index: 0,
        index_count: off_count,
        material: renderer.world_material(&device, Some(&red), None, None, WorldBucket::Opaque),
    };
    assert!(renderer.bake_card_slot(&device, &queue, 1, [0.0; 3], 1.6, &[mesh]));
    let aside = CardSpecies {
        centre: [0.0; 3],
        radius: 1.6,
        slot: Some(1),
    };
    renderer.set_card_species(&device, &queue, &[aside]);
    let near = OrbitCamera::default();
    let (red_at, _, _) = reds(&mut renderer, &near, &one, &[0]);
    assert!(!red_at.is_empty());
    let column = red_at
        .iter()
        .map(|at| (at % SIZE as usize) as f32)
        .sum::<f32>()
        / red_at.len() as f32;
    let expected = (near.view_proj(1.0).project_point3(off).x + 1.0) * 0.5 * SIZE as f32;
    let middle = SIZE as f32 / 2.0;
    assert!(
        (column - middle) * (expected - middle) > 0.0
            && (column - middle).abs() > SIZE as f32 / 8.0,
        "the red lies at column {column}, the model's side is at {expected}"
    );
    // A cutout of a world model is covered where its map is, whatever its vertex packing puts in the colour's floats
    let (model_vertices, model_indices, model_count) = model_cube(&device, 1.0, [1.0; 4]);
    let mesh = CardMesh {
        vertices: &model_vertices,
        indices: &model_indices,
        base_vertex: 0,
        first_index: 0,
        index_count: model_count,
        material: model_material(&renderer, &device, &red, None, WorldBucket::Cutout),
    };
    assert!(renderer.bake_card_slot(&device, &queue, 2, [0.0; 3], radius, &[mesh]));
    renderer.set_card_species(&device, &queue, &[species(Some(2))]);
    let cut = drawn(&mut renderer, &one, &[0]).0;
    assert!(cut * 10 > whole * 9, "{cut} of {whole}");
    // A white leaf whose vertex picks the palette's second column is baked that column's green, as the near model is drawn
    let white = crate::game_shaders::stand_in::constant_view(&device, &queue, [1.0; 4]);
    let palette = crate::game_shaders::stand_in::row_view(
        &device,
        &queue,
        &[[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
    );
    let (tinted_vertices, tinted_indices, tinted_count) =
        model_cube(&device, 1.0, [1.0, 1.0, 1.0 / 255.0, 1.0]);
    let mesh = CardMesh {
        vertices: &tinted_vertices,
        indices: &tinted_indices,
        base_vertex: 0,
        first_index: 0,
        index_count: tinted_count,
        material: model_material(
            &renderer,
            &device,
            &white,
            Some(&palette),
            WorldBucket::Cutout,
        ),
    };
    assert!(renderer.bake_card_slot(&device, &queue, 3, [0.0; 3], radius, &[mesh]));
    renderer.set_card_species(&device, &queue, &[species(Some(3))]);
    let (red_at, _, greens) = reds(&mut renderer, &camera, &one, &[0]);
    assert!(
        red_at.is_empty() && greens * 10 > whole * 9,
        "{} red and {greens} green of {whole}",
        red_at.len()
    );
    // A slot past the device's is refused
    let past = renderer.card_slots(&device);
    let mesh = CardMesh {
        vertices: &vertices,
        indices: &indices,
        base_vertex: 0,
        first_index: 0,
        index_count: count,
        material: renderer.world_material(&device, None, None, None, WorldBucket::Opaque),
    };
    assert!(!renderer.bake_card_slot(&device, &queue, past, [0.0; 3], radius, &[mesh]));
    assert!(
        pollster::block_on(validation.pop()).is_none(),
        "the device accepted every draw"
    );
}

/// A square of `half` lying flat at the origin, its texture coordinates running across it from corner to corner, of model-packed vertices.
fn model_square(device: &wgpu::Device, half: f32) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let mut vertices: Vec<MeshVertex> = (0..4)
        .map(|at| {
            let (u, v) = ((at & 1) as f32, (at >> 1) as f32);
            let corner = [(2.0 * u - 1.0) * half, (2.0 * v - 1.0) * half, 0.0];
            MeshVertex::new_coloured(corner, [0.0, 0.0, 1.0], [u, v], [1.0; 4])
        })
        .collect();
    crate::world::coloured(&mut vertices, (Some(&[[1.0; 4]; 4]), None), (None, None));
    let indices: [u32; 6] = [0, 1, 3, 0, 3, 2];
    let buffer = |label, contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
    };
    (
        buffer(
            "square-vertices",
            bytemuck::cast_slice(&vertices),
            wgpu::BufferUsages::VERTEX,
        ),
        buffer(
            "square-indices",
            bytemuck::cast_slice(&indices),
            wgpu::BufferUsages::INDEX,
        ),
        6,
    )
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_card_covers_the_share_of_its_crown_its_model_covers_where_the_leaves_are_finer_than_a_frame() {
    let test = "a_card_covers_the_share";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    renderer.set_background(BACKGROUND);
    // Looking down on a square seen from above, which a frame looks at whole
    let camera = OrbitCamera {
        distance: 6.0,
        pitch: 1.5,
        ..OrbitCamera::default()
    };
    // Leaves strewn over seven tenths of the square, each finer than a texel of a frame
    let side = 256u32;
    let mut seed = 0x2545_f491u32;
    let strewn: Vec<u8> = (0..side * side)
        .flat_map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            [
                255,
                0,
                0,
                if seed >> 8 < (0.7 * f64::from(1u32 << 24)) as u32 {
                    255
                } else {
                    0
                },
            ]
        })
        .collect();
    let leaves =
        crate::game_shaders::stand_in::uploaded_view(&device, &queue, side, side, &strewn, false);
    let red = crate::game_shaders::stand_in::constant_view(&device, &queue, [1.0, 0.0, 0.0, 1.0]);
    let (vertices, indices, count) = model_square(&device, 1.0);
    let radius = 2f32.sqrt();
    let reds = |renderer: &mut ViewportRenderer, slot: u32, map: &wgpu::TextureView| {
        let mesh = CardMesh {
            vertices: &vertices,
            indices: &indices,
            base_vertex: 0,
            first_index: 0,
            index_count: count,
            material: model_material(renderer, &device, map, None, WorldBucket::Cutout),
        };
        assert!(renderer.bake_card_slot(&device, &queue, slot, [0.0; 3], radius, &[mesh]));
        let species = CardSpecies {
            centre: [0.0; 3],
            radius,
            slot: Some(slot),
        };
        renderer.set_card_species(&device, &queue, &[species]);
        renderer.set_card_lists(&device, &queue, &[list(&[record(0)], 1.0)]);
        renderer.show_card_lists(&[0]);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &queue, &camera, ShadingMode::Material);
        queue.submit([encoder.finish()]);
        let read = read_back(
            &device,
            &queue,
            renderer.colour_texture(),
            renderer.colour_format,
            SIZE,
        );
        (read.bytes.as_chunks::<4>().0.iter())
            .filter(|p| u16::from(p[0]) > 2 * u16::from(p[1].max(p[2])) && p[0] > 40)
            .count()
    };
    let whole = reds(&mut renderer, 0, &red);
    let share = reds(&mut renderer, 1, &leaves) as f32 / whole as f32;
    assert!(whole > 100, "{whole}");
    // A frame cut texel by texel and blended under a cut covers more: 0.82 of it
    assert!(
        share > 0.62 && share < 0.78,
        "the card covers {share} of the square, its leaves 0.7"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_species_is_baked_with_the_words_of_its_reflection_in_the_layer_beside_its_colours() {
    let test = "a_species_is_baked_with_the_words";
    let Some((device, queue)) = gpu(test) else {
        return;
    };
    let mut renderer =
        ViewportRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE);
    // A cube no metal and a quarter rough, its reflectance at normal incidence the dielectric's
    let grey = crate::game_shaders::stand_in::constant_view(&device, &queue, [0.5, 0.5, 0.5, 1.0]);
    let material_a =
        crate::game_shaders::stand_in::constant_view(&device, &queue, [0.0, 0.25, 0.0, 1.0]);
    let mut maps = [None; crate::world::WORLD_MAPS];
    maps[crate::world::slot::colour(0)] = Some(&grey);
    maps[crate::world::slot::material_a(0)] = Some(&material_a);
    let mut constants = crate::world::WorldConstants::default();
    constants.flags |= crate::world::WorldConstants::MODEL_VERTEX;
    let surface = crate::world::WorldSurface {
        family: crate::world::WorldFamily::Standard,
        bucket: WorldBucket::Opaque,
        maps,
        constants,
    };
    let (vertices, indices, count) = model_cube(&device, 1.0, [1.0; 4]);
    let mesh = CardMesh {
        vertices: &vertices,
        indices: &indices,
        base_vertex: 0,
        first_index: 0,
        index_count: count,
        material: renderer.world_surface(&device, &surface),
    };
    // Slot 5 is the second of the second pair of layers
    assert!(renderer.bake_card_slot(&device, &queue, 5, [0.0; 3], 3f32.sqrt(), &[mesh]));
    let atlas = renderer.world.cards.bake.texture();
    let layer = |at: u32| {
        let side = bake::SIDE;
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: bake::FORMAT,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: atlas,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: at },
                aspect: wgpu::TextureAspect::All,
            },
            out.as_image_copy(),
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        read_back(&device, &queue, &out, bake::FORMAT, side).bytes
    };
    assert_eq!(bake::layer_of(5), 2);
    // The middle texel of the slot's first frame, where the cube stands
    let cell = bake::SIDE / 2 / CARD_FRAMES;
    let texel = |bytes: &[u8]| {
        let (x, y) = (bake::SIDE / 2 + cell / 2, cell / 2);
        let at = ((y * bake::SIDE + x) * 4) as usize;
        let linear = |byte: u8| {
            let c = f32::from(byte) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [0, 1, 2, 3].map(|k| {
            if k < 3 {
                linear(bytes[at + k])
            } else {
                f32::from(bytes[at + k]) / 255.0
            }
        })
    };
    let colours = texel(&layer(2));
    let words = texel(&layer(3));
    assert!(
        (colours[0] - 0.5).abs() < 0.02 && colours[3] > 0.99,
        "{colours:?}"
    );
    // Smoothness one less the roughness, the dielectric's 0.04, no foliage's flag
    assert!((words[0] - 0.75).abs() < 0.02, "{words:?}");
    assert!((words[1] - 0.04).abs() < 0.005, "{words:?}");
    assert!(
        (words[2] - 1.0).abs() < 0.02 && words[3] > 0.99,
        "{words:?}"
    );
    // A layer of no slot holds nothing
    assert!(layer(0).iter().all(|b| *b == 0));
}
