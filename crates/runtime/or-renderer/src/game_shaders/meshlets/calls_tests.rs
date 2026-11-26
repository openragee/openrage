//! What a frame of a model asks of the device, counted through the viewport that draws it.

use super::scene_tests::{leaning, two_levels, BOX};
use super::*;
use crate::game_shaders::frame::read_back;
use crate::{OrbitCamera, ShadingMode, ViewportRenderer};

/// The pixels a side of the frame the model is drawn onto.
const SIDE: u32 = 64;

/// A viewport that draws `surfaces` by their own passes; none with no device.
fn shown(
    test: &str,
    surfaces: &[MeshletSurface],
) -> Option<(wgpu::Device, wgpu::Queue, ViewportRenderer)> {
    let (device, queue, mut renderer) = viewport(test)?;
    let (frame, mut pipelines) = (renderer.game_target(), MeshletPipelines::default());
    let scene = prepared((&device, &queue), &mut pipelines, surfaces, &frame, 0);
    renderer.set_gta6_scene(Some(scene));
    Some((device, queue, renderer))
}

/// A viewport with no model yet, on a device that times the stages of a frame where it can; none
/// with no device.
fn viewport(test: &str) -> Option<(wgpu::Device, wgpu::Queue, ViewportRenderer)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("meshlet-calls-test"),
        required_features: adapter.features() & crate::gpu_timer::FEATURES,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    let renderer = ViewportRenderer::new(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        SIDE,
        SIDE,
    );
    Some((device, queue, renderer))
}

/// The scene of `surfaces` as `pipelines` have it once all but `waiting` of them have a pass.
fn prepared(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    pipelines: &mut MeshletPipelines,
    surfaces: &[MeshletSurface],
    frame: &Target,
    waiting: usize,
) -> MeshletScene {
    for _ in 0..4000 {
        let scene = MeshletScene::prepare(device, queue, pipelines, surfaces, (SIDE, SIDE), frame);
        match scene.expect("the scene prepares") {
            Some(scene) if scene.waiting.len() == waiting => return scene,
            _ => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    panic!("no scene came with {waiting} surfaces waiting for a pass");
}

/// Draw a frame of `renderer` through `camera` under `sky`: what its model asked of the device.
fn frame(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    renderer: &mut ViewportRenderer,
    camera: &OrbitCamera,
    sky: Sky,
) -> MeshletCalls {
    renderer.set_gta6_sky(sky);
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, queue, camera, ShadingMode::Shaded);
    queue.submit([encoder.finish()]);
    renderer.gta6_scene().expect("a model is set").calls()
}

/// The leaning light over the box of the two levels, which casts their shadow.
fn casting() -> Sky {
    let mut sky = leaning();
    sky.scene.bounds = Some(BOX);
    sky
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_frame_of_two_surfaces_is_counted_pass_by_pass_and_its_three_stages_are_marked() {
    let test = "a_frame_of_two_surfaces_is_counted_pass_by_pass";
    let surfaces = [two_levels(true), two_levels(true)];
    let Some((device, queue, mut renderer)) = shown(test, &surfaces) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    // Surfaces, the groups of one, the buffers that hold a view matrix, and meshlets in all,
    // each of one primitive.
    let (s, g, v, m) = (2, 2, 2, 4);
    let camera = OrbitCamera::default();
    let calls = frame((&device, &queue), &mut renderer, &camera, casting());
    let cast = MeshletCalls {
        // The shadow's tile, the targets, the mask and the meter.
        passes: 4,
        // Each surface into the tile and into the targets, then the mask, the meter and the
        // frame lit: three full-screen draws, of one group, one group and two.
        draws: 2 * s + 3,
        surface_draws: 2 * s,
        binds: 2 * g * s + 4,
        // The view matrix into each buffer that holds one, the shadow's two matrices, the light.
        writes: v + 2,
        written: u64::from(64 * v + 128) + LIGHT_BYTES as u64,
        // The light's matrix into each of those buffers, and the camera's back.
        copies: 2 * v,
        vertices: u64::from(2 * 3 * m),
    };
    assert_eq!(calls, cast);
    // Under a light that casts nothing the tile's pass goes, with its copies and its write. Seen
    // from elsewhere, so that the targets are drawn again.
    let camera = OrbitCamera { yaw: 1.5, ..camera };
    let calls = frame((&device, &queue), &mut renderer, &camera, leaning());
    let uncast = MeshletCalls {
        passes: 3,
        draws: s + 3,
        surface_draws: s,
        binds: g * s + 4,
        writes: v + 1,
        written: u64::from(64 * v) + LIGHT_BYTES as u64,
        copies: 0,
        vertices: u64::from(3 * m),
    };
    assert_eq!(calls, uncast);

    // And with nothing forgotten in between, a frame like the last is the frame lit alone.
    let calls = frame((&device, &queue), &mut renderer, &camera, leaning());
    let lit_alone = MeshletCalls {
        draws: 1,
        binds: 2,
        ..Default::default()
    };
    assert_eq!(calls, lit_alone);
    renderer.gta6_scene().expect("a model is set").forget();
    let calls = frame((&device, &queue), &mut renderer, &camera, leaning());
    assert_eq!(calls, uncast);

    let scene = renderer.gta6_scene().expect("a model is set");
    assert_eq!((scene.drawn(), scene.meshlets()), (s as usize, m));
    let mut stages = Vec::new();
    let mut encoder = device.create_command_encoder(&Default::default());
    scene.record_targets(&mut encoder, |_, stage| stages.push(stage));
    assert_eq!(stages, ["model shadow", "model targets", "model meter"]);
    drop(encoder);
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);

    // The viewport hands the three to the timer, on a device that times a frame.
    if !device.features().contains(crate::gpu_timer::FEATURES) {
        or_test_support::skip_device(test, "the device times no stage of a frame");
        return;
    }
    let read = (0..1000).any(|_| {
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        renderer.collect_gpu_times(&device)
    });
    assert!(read, "the timestamps were never read");
    let timed: Vec<_> = (renderer.gpu_stages().iter())
        .map(|(stage, _)| *stage)
        .collect();
    assert_eq!(timed[..3], stages[..], "{timed:?}");
}

/// The same light from the other side, which casts the shadow elsewhere.
fn cast_from_the_other_side() -> Sky {
    let mut sky = casting();
    sky.towards = Vec3::new(-0.6, 0.0, 0.8);
    sky
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_frame_draws_a_surface_again_only_where_the_camera_or_the_sun_moved() {
    let test = "a_frame_draws_a_surface_again_only_where_the_camera_or_the_sun_moved";
    let surfaces = [two_levels(true), two_levels(true)];
    let Some((device, queue, mut renderer)) = shown(test, &surfaces) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let s = 2;
    let camera = OrbitCamera::default();
    let turned = OrbitCamera {
        yaw: 1.5,
        ..OrbitCamera::default()
    };
    let (sun, moved) = (casting(), cast_from_the_other_side());
    let frames = [
        (&camera, sun),
        (&camera, sun),
        (&turned, sun),
        (&turned, moved),
        (&camera, sun),
    ];
    let drawn: Vec<u32> = (frames.iter())
        .map(|(camera, sky)| frame((&device, &queue), &mut renderer, camera, *sky).surface_draws)
        .collect();
    // Every surface into the tile and into the targets; then nothing moved, and none; the
    // camera moved, into the targets; the sun moved, into the tile; both moved, into both.
    assert_eq!(drawn, [2 * s, 0, s, s, 2 * s]);
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_surface_draws_by_its_own_pass_while_a_worker_builds_another_s_and_both_as_before_once_built() {
    let test = "a_surface_draws_by_its_own_pass_while_a_worker_builds_another_s";
    // The floor across the frame, and a sliver of one: the frame shows which of the two drew.
    // The second is dimmer, and both hold the triangle over the floor at one depth: the frame
    // shows which of the two drew last as well.
    let mut dim = two_levels(false);
    let half = bytemuck::cast_slice(&[0.5f32, 0.5, 0.5, 1.0]).to_vec();
    dim.feeds.insert((1, 0), Feed::Bytes(half));
    let surfaces = [two_levels(true), dim];
    let Some((device, queue, mut renderer)) = shown(test, &surfaces) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let (gpu, target) = ((&device, &queue), renderer.game_target());
    let camera = OrbitCamera {
        distance: 2.5,
        ..OrbitCamera::default()
    };
    // With every pass built before the scene was asked for: what the frame has to come to.
    let whole = frame(gpu, &mut renderer, &camera, casting());
    let whole = (whole, seen(gpu, &renderer));

    let mut pipelines = MeshletPipelines::default();
    pipelines.hold(&surfaces[0], true);
    let ahead = prepared(gpu, &mut pipelines, &surfaces, &target, 1);
    assert_eq!((ahead.drawn(), ahead.meshlets()), (1, 2));
    assert_eq!(
        (ahead.waiting.as_slice(), ahead.refused.len()),
        (&[0][..], 0)
    );
    renderer.set_gta6_scene(Some(ahead));
    // The one surface into the tile and into the targets, and the floor is not in them.
    let calls = frame(gpu, &mut renderer, &camera, casting());
    assert_eq!((calls.passes, calls.surface_draws), (4, 2));
    let ahead = seen(gpu, &renderer);
    assert!(ahead[1] != whole.1[1] && ahead[1].iter().any(|byte| *byte != 0));
    // Asked to build on, the held pass is still the only one waited for, no third is started,
    // and the next frame draws no surface again.
    let mut scene = renderer.take_gta6_scene().expect("a model is set");
    let more = scene.build_on(&device, &queue, &mut pipelines, &surfaces);
    assert_eq!((more, scene.waiting.as_slice()), (Ok(false), &[0][..]));
    assert_eq!(pipelines.len(), 2);
    renderer.set_gta6_scene(Some(scene));
    let calls = frame(gpu, &mut renderer, &camera, casting());
    assert_eq!(calls.surface_draws, 0);

    // Once its pass is built the scene takes the surface on, in its place before the other,
    // and is the scene prepared with every pass built.
    pipelines.hold(&surfaces[0], false);
    let mut scene = renderer.take_gta6_scene().expect("a model is set");
    for _ in 0..4000 {
        match scene.build_on(&device, &queue, &mut pipelines, &surfaces) {
            Ok(false) => std::thread::sleep(std::time::Duration::from_millis(5)),
            more => {
                assert_eq!(more, Ok(true));
                break;
            }
        }
    }
    assert_eq!((scene.drawn(), scene.waiting.len()), (2, 0));
    assert_eq!(scene.fitted().collect::<Vec<_>>(), [(2, 3), (2, 3)]);
    renderer.set_gta6_scene(Some(scene));
    assert_eq!(frame(gpu, &mut renderer, &camera, casting()), whole.0);
    for (part, (now, before)) in seen(gpu, &renderer).iter().zip(&whole.1).enumerate() {
        let differ = now.iter().zip(before).filter(|(a, b)| a != b).count();
        assert_eq!(
            differ, 0,
            "part {part} of the frame differs in {differ} bytes"
        );
    }
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_surface_is_drawn_three_vertices_a_primitive_of_its_largest_meshlet_and_fills_the_targets_as_at_the_most(
) {
    let test = "a_surface_is_drawn_three_vertices_a_primitive_of_its_largest_meshlet";
    let camera = OrbitCamera {
        distance: 2.5,
        ..OrbitCamera::default()
    };
    // The two levels state one primitive a meshlet; then as many as a group has threads, which
    // is every vertex a draw has ever had; then more than that, which is drawn as the most.
    let drawn = |primitives: u32| {
        let mut surface = two_levels(true);
        surface.primitives = primitives;
        let (device, queue, mut renderer) = shown(test, &[surface])?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let calls = frame((&device, &queue), &mut renderer, &camera, casting());
        let seen = seen((&device, &queue), &renderer);
        let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
        assert_eq!(refused, None);
        Some((calls, seen))
    };
    let (Some(fitted), Some(most), Some(past)) = (drawn(1), drawn(VERTICES / 3), drawn(4000))
    else {
        return;
    };
    // Two meshlets, into the tile and into the targets.
    assert_eq!(fitted.0.vertices, 2 * 2 * 3);
    assert_eq!(most.0.vertices, u64::from(2 * 2 * VERTICES));
    assert_eq!(past.0, most.0);
    let counted = |calls: MeshletCalls| MeshletCalls {
        vertices: 0,
        ..calls
    };
    assert_eq!(counted(fitted.0), counted(most.0));
    for (part, (fitted, most)) in fitted.1.iter().zip(&most.1).enumerate() {
        let differ = fitted.iter().zip(most).filter(|(a, b)| a != b).count();
        assert_eq!(
            differ, 0,
            "part {part} of the frame differs in {differ} bytes"
        );
        assert!(fitted.iter().any(|byte| *byte != 0), "part {part} is blank");
    }
}

/// A mesh stage whose every thread makes a triangle in a cell of its own, of a grid of eight
/// by sixteen: a meshlet's threads fill eight rows from the eighth of its number on. The frame
/// then shows which vertices the device ran.
const CELLS: &str = "
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) shade: vec3<f32>,
}

@vertex fn main(@builtin(vertex_index) v: u32, @builtin(instance_index) meshlet: u32) -> Out {
    let thread = v / 3u;
    let cell = vec2<f32>(f32(thread % 8u), f32(thread / 8u + 8u * meshlet));
    var corners = array<vec2<f32>, 3>(vec2(0.1, 0.1), vec2(0.9, 0.1), vec2(0.1, 0.9));
    let at = (cell + corners[v % 3u]) / vec2<f32>(8.0, 16.0);
    var out: Out;
    out.position = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.5, 1.0);
    out.shade = vec3<f32>(1.0);
    return out;
}";

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_device_draws_the_primitives_a_surface_states_for_each_meshlet_and_no_thread_past_them() {
    let test = "the_device_draws_the_primitives_a_surface_states_for_each_meshlet";
    let mut cells = super::tests::surface();
    cells.vertex = Arc::new(naga::front::wgsl::parse_str(CELLS).expect("the stage parses"));
    cells.primitives = 5;
    let Some((device, queue, mut renderer)) = shown(test, &[cells]) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let gpu = (&device, &queue);
    // The cells of the grid the colour target holds anything in after a frame, by column and
    // row, and what the frame counted.
    let cells = |renderer: &mut ViewportRenderer| {
        let calls = frame(gpu, renderer, &OrbitCamera::default(), casting());
        let [_, colour, _] = seen(gpu, renderer);
        let (wide, high) = (SIDE as usize / 8, SIDE as usize / 16);
        let filled = |column: usize, row: usize| {
            let texel =
                |x: usize, y: usize| 4 * ((row * high + y) * SIDE as usize + column * wide + x);
            (0..wide).any(|x| (0..high).any(|y| colour[texel(x, y)..][..4] != [0; 4]))
        };
        let drawn: Vec<(usize, usize)> = (0..16)
            .flat_map(|row| (0..8).map(move |column| (column, row)))
            .filter(|(column, row)| filled(*column, *row))
            .collect();
        (drawn, calls.vertices)
    };
    // Five threads of each of the two meshlets, and not the hundred and twenty-three after them.
    let stated: Vec<(usize, usize)> = [0, 8]
        .iter()
        .flat_map(|row| (0..5).map(move |column| (column, *row)))
        .collect();
    // What the frame counts is what the device was handed: three corners a primitive, for each
    // meshlet, into the tile and into the targets.
    assert_eq!(cells(&mut renderer), (stated.clone(), 2 * 3 * 5 * 2));
    // Asked for the most an instance has, every thread of each meshlet fills its cell; asked
    // for the fitted draw again, the five do.
    let scene = |renderer: &ViewportRenderer, most: bool| {
        (renderer.gta6_scene().expect("a model is set")).draw_at_most(most);
    };
    scene(&renderer, true);
    let (drawn, vertices) = cells(&mut renderer);
    assert_eq!(
        (drawn.len(), vertices),
        (2 * 64, u64::from(2 * 2 * VERTICES))
    );
    scene(&renderer, false);
    assert_eq!(cells(&mut renderer), (stated, 2 * 3 * 5 * 2));
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

/// The frame `renderer` drew last and the first two targets of its model, as bytes.
fn seen(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    renderer: &ViewportRenderer,
) -> [Vec<u8>; 3] {
    let scene = renderer.gta6_scene().expect("a model is set");
    let target = |at: usize| scene.target(at).expect("a target");
    [renderer.colour_texture(), target(0), target(1)].map(|texture| {
        let rgba = wgpu::TextureFormat::Rgba8Unorm;
        read_back(device, queue, texture, rgba, SIDE).bytes
    })
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_frame_drawn_after_a_skip_is_byte_for_byte_the_frame_drawn_in_full() {
    let test = "a_frame_drawn_after_a_skip_is_byte_for_byte_the_frame_drawn_in_full";
    let Some((device, queue, mut renderer)) = shown(test, &[two_levels(true)]) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let near = OrbitCamera {
        distance: 2.5,
        ..OrbitCamera::default()
    };
    let turned = OrbitCamera {
        yaw: 1.5,
        ..near.clone()
    };
    let (sun, moved) = (casting(), cast_from_the_other_side());
    // The camera moves, then the sun, then nothing, then the camera again under a light that
    // casts none, and under the first light from where the first frame was seen.
    let frames = [
        (&near, sun),
        (&turned, sun),
        (&turned, moved),
        (&turned, moved),
        (&near, leaning()),
        (&near, sun),
    ];
    let mut run = |in_full: bool| -> Vec<(u32, [Vec<u8>; 3])> {
        let mut drawn = Vec::new();
        for (camera, sky) in &frames {
            if in_full {
                renderer.gta6_scene().expect("a model is set").forget();
            }
            let calls = frame((&device, &queue), &mut renderer, camera, *sky);
            drawn.push((calls.surface_draws, seen((&device, &queue), &renderer)));
        }
        drawn
    };
    let (skipped, in_full) = (run(false), run(true));
    for (at, (skipped, in_full)) in skipped.iter().zip(&in_full).enumerate() {
        for (part, what) in ["the frame", "the colour target", "the normal target"]
            .iter()
            .enumerate()
        {
            let differ = (skipped.1[part].iter().zip(&in_full.1[part]))
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(differ, 0, "{what} of frame {at} differs in {differ} bytes");
        }
    }
    let draws =
        |frames: &[(u32, [Vec<u8>; 3])]| -> Vec<u32> { frames.iter().map(|f| f.0).collect() };
    assert_eq!(draws(&skipped), [2, 1, 1, 0, 1, 1]);
    assert_eq!(draws(&in_full), [2, 2, 2, 2, 1, 2]);
    // The frames are of a model, and each move shows: nothing here is equal for being blank.
    let lit = |at: usize| &in_full[at].1[0];
    assert!(lit(0) != lit(1) && lit(1) != lit(2) && lit(4) != lit(5));
    assert_eq!(lit(2), lit(3));
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_frame_after_the_viewport_is_resized_and_back_is_the_frame_drawn_in_full() {
    let test = "a_frame_after_the_viewport_is_resized_and_back_is_the_frame_drawn_in_full";
    let Some((device, queue, mut renderer)) = shown(test, &[two_levels(true)]) else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let gpu = (&device, &queue);
    let camera = OrbitCamera {
        distance: 2.5,
        ..OrbitCamera::default()
    };
    let in_full = frame(gpu, &mut renderer, &camera, casting());
    let whole = seen(gpu, &renderer);
    // To another size and back under the same camera and sky: the targets are made anew each
    // time, so every pass draws again though no byte a frame is drawn by moved.
    renderer.resize(&device, SIDE / 2, SIDE / 2);
    let halved = frame(gpu, &mut renderer, &camera, casting());
    renderer.resize(&device, SIDE, SIDE);
    let back = frame(gpu, &mut renderer, &camera, casting());
    assert_eq!((halved, back), (in_full, in_full));
    for (part, (now, before)) in seen(gpu, &renderer).iter().zip(&whole).enumerate() {
        let differ = now.iter().zip(before).filter(|(a, b)| a != b).count();
        assert_eq!(
            differ, 0,
            "part {part} of the frame differs in {differ} bytes"
        );
        assert!(before.iter().any(|byte| *byte != 0), "part {part} is blank");
    }
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

#[test]
fn a_frame_writes_and_draws_again_only_what_the_bytes_it_is_drawn_by_moved() {
    let light = |fill: u8| [fill; LIGHT_BYTES];
    let (a, b) = ([1u8; 64], [2u8; 64]);
    let writes = |view, matrices, light| Writes {
        view,
        matrices,
        light,
    };
    let draws = |shadow, targets, lit| Stale {
        shadow,
        targets,
        lit,
    };
    let (nothing, none) = (Writes::default(), Stale::default());
    let mut kept = Kept::default();
    // A frame as `update` and `record_targets` take it: what it writes, then what it draws.
    let frame = |kept: &mut Kept, view, tile, fill| {
        let writes = kept.take(view, tile, &light(fill));
        (writes, kept.draws())
    };
    // A first frame writes and draws everything, and the same frame again nothing.
    let all = (writes(true, true, true), draws(true, true, true));
    assert_eq!(frame(&mut kept, a, Some(a), 0), all);
    assert_eq!(frame(&mut kept, a, Some(a), 0), (nothing, none));
    // The camera moved: its matrix, and the surfaces into the targets.
    let moved = (writes(true, false, false), draws(false, true, true));
    assert_eq!(frame(&mut kept, b, Some(a), 0), moved);
    // The sun moved: the tile's two matrices, and the surfaces into the tile.
    let cast = (writes(false, true, false), draws(true, false, true));
    assert_eq!(frame(&mut kept, b, Some(b), 0), cast);
    // The light alone: no surface is drawn again.
    let relit = (writes(false, false, true), draws(false, false, true));
    assert_eq!(frame(&mut kept, b, Some(b), 1), relit);
    // Both moved: the surfaces into both.
    let both = (writes(true, true, false), draws(true, true, true));
    assert_eq!(frame(&mut kept, a, Some(a), 1), both);
    // A frame that casts nothing leaves the tile, which the same light then finds as it cast it.
    assert_eq!(frame(&mut kept, a, None, 1), (nothing, none));
    assert_eq!(frame(&mut kept, a, Some(a), 1), (nothing, none));

    // What no recording drew is owed still, whatever the frames after it lack: the tile's pass
    // is handed the matrices again, since it copies the camera's back.
    assert_eq!(kept.take(b, Some(b), &light(1)), writes(true, true, false));
    assert_eq!(kept.take(b, Some(b), &light(1)), writes(false, true, false));
    // A tile no frame cast into stays owed to the next one that casts.
    let uncast = (nothing, draws(false, true, true));
    assert_eq!(frame(&mut kept, b, None, 1), uncast);
    let owed = (writes(false, true, false), draws(true, false, false));
    assert_eq!(frame(&mut kept, b, Some(b), 1), owed);
    assert_eq!(frame(&mut kept, b, Some(b), 1), (nothing, none));
}
