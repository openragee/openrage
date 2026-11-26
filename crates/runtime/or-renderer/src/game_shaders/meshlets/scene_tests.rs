//! Frames of a small model lit onto a target and read back: its shadow, the clouds over it and
//! the ground under it.

use super::tests::{surface, SIZE};
use super::*;
use crate::game_shaders::frame::read_back;

/// A small triangle over the middle of the frame at a depth of six tenths, and under it at a
/// tenth a floor: across the whole frame, or a sliver in a corner where `floored` is not.
pub(super) fn two_levels(floored: bool) -> MeshletSurface {
    let floor: [[f32; 3]; 3] = match floored {
        true => [[-1.0, -1.0, 0.1], [3.0, -1.0, 0.1], [-1.0, 3.0, 0.1]],
        false => [[-1.0, 1.0, 0.1], [-0.9, 1.0, 0.1], [-1.0, 0.9, 0.1]],
    };
    let over: [[f32; 3]; 3] = [[-0.3, -0.3, 0.6], [0.3, -0.3, 0.6], [0.0, 0.3, 0.6]];
    let positions: Vec<u32> = (floor.iter().chain(&over))
        .flatten()
        .map(|v| v.to_bits())
        .collect();
    let mut surface = surface();
    (surface.feeds).insert(
        (0, 0),
        Feed::Bytes(bytemuck::cast_slice(&positions).to_vec()),
    );
    surface
}

/// The frames each of `skies` lights `model` onto, seen from straight above, as the bytes of
/// their pixels; none with no device.
fn frames_of(test: &str, model: MeshletSurface, skies: &[Sky]) -> Option<Vec<Vec<u8>>> {
    let seen: Vec<(Mat4, Sky)> = skies.iter().map(|sky| (Mat4::IDENTITY, *sky)).collect();
    frames_seen(test, model, &seen)
}

/// The frames each of `skies` lights `model` onto, seen by its matrix; none with no device.
fn frames_seen(test: &str, model: MeshletSurface, skies: &[(Mat4, Sky)]) -> Option<Vec<Vec<u8>>> {
    frames_stated(test, model, skies, |_, _, _| {})
}

/// The frames each of `skies` lights `model` onto, seen by its matrix, `state` handed the scene
/// before each with the frame's number; none with no device.
pub(super) fn frames_stated(
    test: &str,
    model: MeshletSurface,
    skies: &[(Mat4, Sky)],
    mut state: impl FnMut(&mut MeshletScene, &wgpu::Queue, usize),
) -> Option<Vec<Vec<u8>>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("meshlet-scene-test"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    let frame = Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: Some(1),
    };
    let size = (SIZE, SIZE);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pipelines = MeshletPipelines::default();
    let surfaces = [model];
    let mut scene = loop {
        let scene = MeshletScene::prepare(&device, &queue, &mut pipelines, &surfaces, size, &frame);
        match scene.expect("the scene prepares") {
            Some(scene) => break scene,
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    };
    let colour = render_target(
        &device,
        "frame",
        size,
        frame.colour,
        wgpu::TextureUsages::COPY_SRC,
    );
    let depth = render_target(&device, "depth", size, DEPTH, wgpu::TextureUsages::empty());
    let mut frames = Vec::new();
    for (at, (view_proj, sky)) in skies.iter().enumerate() {
        state(&mut scene, &queue, at);
        let lighting = Lighting {
            view_proj: *view_proj,
            sky: *sky,
            camera: Vec3::new(0.0, 0.0, 4.0),
        };
        scene.update(&queue, &lighting);
        let mut encoder = device.create_command_encoder(&Default::default());
        scene.record_targets(&mut encoder, |_, _| {});
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
        frames.push(read_back(&device, &queue, colour.texture(), frame.colour, SIZE).bytes);
    }
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
    Some(frames)
}

/// The pixel of `frame` that shows the place (`x`, `y`) of the clip space.
fn seen_at(frame: &[u8], (x, y): (f32, f32)) -> [u8; 4] {
    let (px, py) = (
        ((x + 1.0) / 2.0 * SIZE as f32) as usize,
        ((1.0 - y) / 2.0 * SIZE as f32) as usize,
    );
    let at = (py * SIZE as usize + px) * 4;
    [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
}

/// A white light that leans towards +x, and nothing from the sky.
pub(super) fn leaning() -> Sky {
    Sky {
        towards: Vec3::new(0.6, 0.0, 0.8),
        sun: Vec3::splat(std::f32::consts::PI),
        sky: Vec3::ZERO,
        ground: Vec3::ZERO,
        exposure: 1.0,
        tone: None,
        scene: Scene::default(),
    }
}

/// The box the two levels fill.
pub(super) const BOX: (Vec3, Vec3) = (Vec3::new(-1.0, -1.0, 0.1), Vec3::new(1.0, 1.0, 0.6));
/// The light leans towards +x, so the triangle's shadow falls three eighths of a unit towards
/// -x: a place under it beside the triangle, where the camera sees it, and one in the open.
const SHADOWED: (f32, f32) = (-0.5, -0.2);
const OPEN: (f32, f32) = (0.6, -0.2);

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_upper_triangle_shadows_the_floor_on_the_side_away_from_the_light() {
    let mut boxed = leaning();
    boxed.scene.bounds = Some(BOX);
    let Some(frames) = frames_of(
        "the_upper_triangle_shadows_the_floor",
        two_levels(true),
        &[leaning(), boxed],
    ) else {
        return;
    };
    let (plain, cast) = (&frames[0], &frames[1]);
    // With no box stated nothing is cast: the floor is lit alike on both sides.
    assert_eq!(seen_at(plain, SHADOWED), seen_at(plain, OPEN));
    let (dark, lit) = (seen_at(cast, SHADOWED), seen_at(cast, OPEN));
    assert_eq!(lit, seen_at(plain, OPEN), "the open floor is as it was");
    assert!(lit[0] > 100 && dark[0] < lit[0] / 4, "{dark:?} {lit:?}");
    // The triangle itself faces the light and nothing stands before it.
    assert_eq!(seen_at(cast, (0.0, -0.1)), seen_at(plain, (0.0, -0.1)));
}

/// A camera that looks straight down at the floor about (`x`, `y`), from `half` to each side.
fn close_on(x: f32, y: f32, half: f32) -> Mat4 {
    Mat4::from_scale(Vec3::new(1.0 / half, 1.0 / half, 1.0))
        * Mat4::from_translation(Vec3::new(-x, -y, 0.0))
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_floor_a_search_s_reach_outside_a_shadow_s_edge_is_lit_whole() {
    // A light dim enough that the frame's curve does not flatten what a tap takes off it.
    let dim = Sky {
        sun: Vec3::splat(0.6),
        ..leaning()
    };
    let mut boxed = dim;
    boxed.scene.bounds = Some(BOX);
    // How far the search reaches over the floor: across the light's way a unit of the world is
    // as long on the tile as anywhere.
    let to_tile = sun_shadow::to_tile(dim.towards, BOX);
    let across = to_tile.project_point3(Vec3::Y) - to_tile.project_point3(Vec3::ZERO);
    let reach = sun_shadow::DISC / (across.truncate().length() * 0.5);
    // The shadow's lower edge lies along y = -0.3 from x = -0.675 to -0.075. Seen close: the
    // floor from half the reach below that edge to a fifth past the whole of it, where only the
    // spiral's outermost taps reach the shadow; then as far above the edge, inside the shadow.
    let (edge, half) = (-0.3, 0.35 * reach);
    let below = close_on(-0.5, edge - 0.85 * reach, half);
    let above = close_on(-0.5, edge + 0.85 * reach, half);
    let Some(frames) = frames_seen(
        "the_floor_a_search_s_reach_outside_a_shadow_s_edge",
        two_levels(true),
        &[(below, dim), (below, boxed), (above, dim), (above, boxed)],
    ) else {
        return;
    };
    let reds = |frame: &[u8]| -> Vec<u8> { frame.chunks(4).map(|pixel| pixel[0]).collect() };
    let (open, cast) = (reds(&frames[0]), reds(&frames[1]));
    assert!(open.iter().all(|red| *red > 60), "{open:?}");
    let dimmed: Vec<(u8, u8)> = (open.iter().zip(&cast))
        .filter(|(open, cast)| open != cast)
        .map(|(open, cast)| (*open, *cast))
        .collect();
    assert!(
        dimmed.is_empty(),
        "{} of {} pixels below the shadow are not lit as with none cast (lit, cast): {dimmed:?}",
        dimmed.len(),
        open.len()
    );
    // The shadow is where the frames were taken beside: as far inside it the floor is dark.
    let (lit, dark) = (reds(&frames[2]), reds(&frames[3]));
    assert!(
        lit.iter().zip(&dark).all(|(lit, dark)| *dark < lit / 4),
        "{lit:?} {dark:?}"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_clouds_dim_the_directional_light_by_the_share_they_let_through() {
    // A light dim enough that the frame's curve does not flatten it.
    let dim = Sky {
        sun: Vec3::splat(0.6),
        ..leaning()
    };
    let mut clouded = dim;
    clouded.scene.clouds.open = 0.25;
    // Half hidden, and what is hidden taken all the way towards surroundings that give
    // nothing: half the light is left, and half of that comes through.
    let mut hazed = dim;
    hazed.scene.clouds = Clouds {
        open: 0.5,
        haze_blend: 1.0,
        ..Clouds::default()
    };
    let Some(frames) = frames_of(
        "the_clouds_dim_the_directional_light",
        two_levels(true),
        &[dim, clouded, hazed],
    ) else {
        return;
    };
    let (under, clear) = (seen_at(&frames[1], OPEN), seen_at(&frames[0], OPEN));
    let (dimmed, whole) = (u32::from(under[0]), u32::from(clear[0]));
    assert!(
        whole > 40 && dimmed < whole * 3 / 4 && dimmed > whole / 8,
        "{under:?} {clear:?}"
    );
    assert_eq!(seen_at(&frames[2], OPEN), under);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_ground_laid_under_the_model_is_lit_where_no_surface_is_and_takes_the_model_s_shadow() {
    let mut boxed = leaning();
    boxed.scene.bounds = Some(BOX);
    let mut laid = boxed;
    laid.scene.laid = Some(Vec3::splat(0.5));
    let Some(frames) = frames_of(
        "a_ground_laid_under_the_model",
        two_levels(false),
        &[boxed, laid],
    ) else {
        return;
    };
    let (bare, ground) = (&frames[0], &frames[1]);
    // No floor of the model's own: nothing is drawn beside the triangle until a ground is laid.
    assert_eq!(seen_at(bare, OPEN), [0; 4]);
    assert_eq!(seen_at(bare, SHADOWED), [0; 4]);
    let (lit, dark) = (seen_at(ground, OPEN), seen_at(ground, SHADOWED));
    assert!(
        lit[0] > 60 && lit[0] == lit[1] && lit[1] == lit[2] && lit[3] == 255,
        "a grey ground in the light: {lit:?}"
    );
    assert!(dark[0] < lit[0] / 4 && dark[3] == 255, "{dark:?} {lit:?}");
    // The model is drawn over its ground as it was without.
    assert_eq!(seen_at(ground, (0.0, -0.1)), seen_at(bare, (0.0, -0.1)));
}

/// A tone whose curve exposes a metered frame to a mean of one.
fn metering(whole_frame: bool) -> Tone {
    let curve = Curve {
        scale: -std::f32::consts::LOG2_E,
        vertical: 10.0,
        horizontal: 0.0,
        bending: 0.0,
    };
    let filmic = Filmic([0.22, 0.3, 0.1, 0.2, 0.01, 0.3, 11.2]);
    Tone {
        roughness_mod: 0.0,
        disc: 0.0,
        lowlights: 0.0,
        highlights: 0.0,
        curve: [curve; 2],
        blend: 0.0,
        key: None,
        least: -20.0,
        most: 20.0,
        added: 0.0,
        bright: filmic,
        dark: filmic,
        bright_at: 1.0,
        dark_at: 3.0,
        whole_frame,
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_meter_that_takes_the_whole_frame_exposes_a_dim_model_under_a_bright_sky_less() {
    // The triangle alone, dimly lit; where it is not, the ground sends up a hundred times its
    // light and the sky is as bright, though neither is drawn.
    let lit = |whole: bool| Sky {
        sun: Vec3::splat(0.6),
        ground: Vec3::splat(15.0),
        tone: Some(metering(whole)),
        scene: Scene {
            dome: Some(Dome([15.0; 72])),
            ..Scene::default()
        },
        ..leaning()
    };
    // Seen from above the rays go down, to the ground; seen from below, with the depth turned
    // over, they go up into the sky.
    let (above, below) = (
        Mat4::IDENTITY,
        Mat4::from_translation(Vec3::Z) * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0)),
    );
    let Some(frames) = frames_seen(
        "a_meter_that_takes_the_whole_frame",
        two_levels(false),
        &[
            (above, lit(false)),
            (above, lit(true)),
            (below, lit(false)),
            (below, lit(true)),
        ],
    ) else {
        return;
    };
    let middle = (0.0, -0.1);
    for (alone, whole) in [(&frames[0], &frames[1]), (&frames[2], &frames[3])] {
        let (bright, dim) = (seen_at(alone, middle), seen_at(whole, middle));
        // Metered alone the triangle is exposed to the middle of the curve; with the frame
        // round it, far under.
        assert!(
            bright[0] > 100 && dim[0] < bright[0] / 3,
            "{bright:?} {dim:?}"
        );
        assert_eq!(seen_at(whole, OPEN), [0; 4], "metered, not drawn");
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_probe_s_nine_numbers_light_the_floor_as_the_sky_they_were_gathered_of() {
    // No sun. A sky and a ground of one dim colour every way, as two colours and as a probe's
    // nine numbers: the floor, which faces up, is lit alike by both.
    let dim = 0.08;
    let unlit = Sky {
        sun: Vec3::ZERO,
        ..leaning()
    };
    let two = Sky {
        sky: Vec3::splat(dim),
        ground: Vec3::splat(dim),
        ..unlit
    };
    // One colour every way is the first number alone: that colour by the angle of the sphere,
    // weighed as the first number weighs.
    let mut nine = [Vec3::ZERO; 9];
    nine[0] = Vec3::splat(dim * 0.282_095 * 4.0 * std::f32::consts::PI);
    let mut probed = unlit;
    probed.scene.around = Some(nine);
    let Some(frames) = frames_of(
        "a_probe_s_nine_numbers_light_the_floor",
        two_levels(true),
        &[two, probed, unlit],
    ) else {
        return;
    };
    let (by_two, by_nine) = (seen_at(&frames[0], OPEN), seen_at(&frames[1], OPEN));
    assert!(by_two[0] > 30, "{by_two:?}");
    assert!(
        by_two[0].abs_diff(by_nine[0]) <= 2,
        "{by_two:?} {by_nine:?}"
    );
    assert_eq!(
        seen_at(&frames[2], OPEN)[..3],
        [0; 3],
        "no numbers, no light"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn each_of_a_probe_s_nine_numbers_lights_a_floor_that_leans_along_every_axis() {
    // A floor of every colour whose pixel stage states a normal that leans along each axis, so
    // that every one of the nine numbers weighs on it: 3040 and 2606 of 4095 across the
    // octahedron, unpacked as the resolve unpacks them.
    let e = Vec3::new(3040.0, 2606.0, 0.0) / 2047.5 - Vec3::new(1.0, 1.0, 0.0);
    let n = Vec3::new(e.x, e.y, 1.0 - e.x.abs() - e.y.abs()).normalize();
    assert!(n.min_element() > 0.35 && n.x - n.y > 0.3, "{n:?}");
    let upright = "2047u | (2047u << 12u)";
    assert!(super::tests::PIXEL.contains(upright));
    let pixel = super::tests::PIXEL.replace(upright, "3040u | (2606u << 12u)");
    let mut floor = two_levels(true);
    floor.fragment = Arc::new(naga::front::wgsl::parse_str(&pixel).expect("the WGSL parses"));
    let white = Image {
        width: 1,
        height: 1,
        rgba: vec![255; 4],
        srgb: false,
    };
    (floor.feeds).insert((1, 1), Feed::Textures(vec![Some(Arc::new(white))]));
    // Nine numbers that each differ from every other, in an order of its own in each colour.
    let reds = [0.80, 0.10, 0.40, 0.25, 0.55, 0.05, 0.45, 0.30, 0.15];
    let greens = [0.70, 0.35, 0.05, 0.50, 0.15, 0.40, 0.10, 0.55, 0.25];
    let blues = [0.90, 0.20, 0.30, 0.05, 0.45, 0.25, 0.60, 0.10, 0.50];
    let every: [Vec3; 9] =
        std::array::from_fn(|at| Vec3::new(reds[at], greens[at], blues[at]) * 0.35);
    // What falls on the floor of them is worked out here, and the frame they light is the one
    // that light gives as a first number alone, which the same every way is.
    let fallen = falls_on(&every, n);
    assert!(fallen.min_element() > 0.2, "{fallen:?}");
    let mut alone = [Vec3::ZERO; 9];
    alone[0] = fallen / 0.886_227;
    // The same nine with two of them changed over, as a slip of a row would have them.
    let mut slipped = every;
    slipped.swap(5, 7);
    let probed = |nine: [Vec3; 9]| {
        let mut sky = Sky {
            sun: Vec3::ZERO,
            ..leaning()
        };
        sky.scene.around = Some(nine);
        sky
    };
    let Some(frames) = frames_of(
        "each_of_a_probe_s_nine_numbers_lights_a_floor",
        floor,
        &[probed(every), probed(alone), probed(slipped)],
    ) else {
        return;
    };
    let (by_nine, by_one) = (seen_at(&frames[0], OPEN), seen_at(&frames[1], OPEN));
    assert!(by_nine[..3].iter().all(|c| *c > 30), "{by_nine:?}");
    for c in 0..3 {
        assert!(
            by_nine[c].abs_diff(by_one[c]) <= 1,
            "{by_nine:?} {by_one:?}"
        );
    }
    assert_ne!(seen_at(&frames[2], OPEN), by_nine, "two rows changed over");
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_surface_s_gloss_takes_the_probe_s_light_from_about_the_mirrored_way_as_the_table_weighs_it() {
    // A floor that leans, as the floor of the nine numbers' test leans, half rough; and the
    // same floor reflecting all the light it takes head on.
    let e = Vec3::new(3040.0, 2606.0, 0.0) / 2047.5 - Vec3::new(1.0, 1.0, 0.0);
    let n = Vec3::new(e.x, e.y, 1.0 - e.x.abs() - e.y.abs()).normalize();
    let upright = "2047u | (2047u << 12u)";
    let dull = "vec4<f32>(0.5, bytes / 255.0),\n        vec4<f32>(0.0),";
    let mirror = "vec4<f32>(0.5, bytes / 255.0),\n        vec4<f32>(1.0, 1.0, 1.0, 0.0),";
    assert!(super::tests::PIXEL.contains(upright) && super::tests::PIXEL.contains(dull));
    let leaning_pixel = super::tests::PIXEL.replace(upright, "3040u | (2606u << 12u)");
    let floor = |pixel: &str| {
        let mut floor = two_levels(true);
        floor.fragment = Arc::new(naga::front::wgsl::parse_str(pixel).expect("the WGSL parses"));
        floor
    };
    // Where the floor is seen, the way to the camera and the way that mirrors it; and a way
    // across the floor's own, along which a probe's light leans without a matte floor's
    // taking more or less of it.
    // The middle of the pixel that shows the place.
    let middle = |along: f32| {
        (((along + 1.0) * 0.5 * SIZE as f32).floor() + 0.5) / (0.5 * SIZE as f32) - 1.0
    };
    let at = Vec3::new(middle(OPEN.0), -middle(-OPEN.1), 0.1);
    let v = (Vec3::new(0.0, 0.0, 4.0) - at).normalize();
    let mirrored = 2.0 * n.dot(v) * n - v;
    let across = (mirrored - mirrored.dot(n) * n).normalize();
    assert!(across.dot(n).abs() < 1e-5 && across.dot(mirrored) > 0.5);
    // An even light from all around, and the same with more of it from that way, or less.
    let (even, lean) = (0.4, 0.2);
    let probe = |lean: f32| {
        let mut nine = [Vec3::ZERO; 9];
        nine[0] = Vec3::splat(even);
        nine[1] = Vec3::splat(lean * across.y);
        nine[2] = Vec3::splat(lean * across.z);
        nine[3] = Vec3::splat(lean * across.x);
        nine
    };
    // A tone that exposes every frame alike, by no stops.
    let tone = Tone {
        least: 0.0,
        most: 0.0,
        ..metering(false)
    };
    let lit = |lean: f32| {
        let mut sky = Sky {
            sun: Vec3::ZERO,
            tone: Some(tone),
            ..leaning()
        };
        sky.scene.around = Some(probe(lean));
        sky
    };
    let skies = [lit(lean), lit(0.0), lit(-lean)];
    let test = "a_surface_s_gloss_takes_the_probe_s_light";
    let pixel = leaning_pixel.replace(dull, mirror);
    let Some(mirrors) = frames_of(test, floor(&pixel), &skies) else {
        return;
    };
    let Some(dulls) = frames_of(test, floor(&leaning_pixel), &skies) else {
        return;
    };
    // The floor is red, so its green is its gloss alone. The mirror's gloss is brighter with
    // the light that leans towards the mirrored way, and dimmer with the light that leans
    // from it; and the dull floor, which reflects only at grazing angles, shows far less.
    let green = |frames: &[Vec<u8>], frame: usize| seen_at(&frames[frame], OPEN)[1];
    let [towards, level, away] = [0, 1, 2].map(|frame| green(&mirrors, frame));
    assert!(
        towards > level + 10 && level > away + 10,
        "{towards} {level} {away}"
    );
    assert!(
        green(&dulls, 0) + 30 < towards,
        "{} {towards}",
        green(&dulls, 0)
    );
    // What the mirror shows is what the cone about the mirrored way holds, whole: half rough,
    // the cone is the narrowest, and a surface that reflects everything head on reflects what
    // its facets send onto one another too.
    let k: f32 = 0.8;
    let cone = |lean: f32| {
        let held = 0.886_227 * (1.0 - k * k) * even
            + 1.023_328 * (1.0 - k.powi(3)) * lean * across.dot(mirrored);
        held / (std::f32::consts::PI * (1.0 - k * k))
    };
    for (frame, lean) in [(0, lean), (1, 0.0), (2, -lean)] {
        let shown = tone.filmic(1.0).toned(cone(lean)).powf(0.4545) * 255.0;
        let seen = f32::from(green(&mirrors, frame));
        assert!((seen - shown).abs() <= 3.0, "{seen} against {shown}");
    }
    // The dull floor reflects nothing head on. Its green is the cone's light by the share its
    // facets reflect at this angle, and its red that and its matte light: the even light that
    // falls on it, less that share, by the matte share the table holds for it.
    let [grazing, _, matte] = env_fit::fit_at(n.dot(v), 0.5);
    let shown = |light: f32| tone.filmic(1.0).toned(light).powf(0.4545) * 255.0;
    let fallen = 0.886_227 * even / std::f32::consts::PI;
    let [red, green, ..] = seen_at(&dulls[1], OPEN).map(f32::from);
    let gloss = cone(0.0) * grazing;
    assert!(
        (green - shown(gloss)).abs() <= 3.0,
        "{green} against {}",
        shown(gloss)
    );
    let weighed = shown(fallen * (1.0 - grazing) * matte + gloss);
    assert!((red - weighed).abs() <= 3.0, "{red} against {weighed}");
    assert!(shown(fallen + gloss) > weighed + 6.0, "the weighing shows");
}
