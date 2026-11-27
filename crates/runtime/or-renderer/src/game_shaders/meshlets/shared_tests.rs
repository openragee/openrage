//! What a scene makes once for the surfaces that bind it alike, and a map bound after the scene was prepared.

use super::super::tests::{surface, PIXEL, SIZE};
use super::super::*;
use super::*;
use crate::game_shaders::frame::read_back;

/// A device of the adapter's own limits; none with no adapter.
fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("meshlet-shared-test"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()
}

fn frame() -> Target {
    Target {
        colour: wgpu::TextureFormat::Rgba8Unorm,
        depth: DEPTH,
        depth_compare: wgpu::CompareFunction::GreaterEqual,
        blend: None,
        colour_attachments: Some(1),
    }
}

/// The scene of `surfaces` once every pass of theirs is built.
fn prepared(
    (device, queue): (&wgpu::Device, &wgpu::Queue),
    pipelines: &mut MeshletPipelines,
    surfaces: &[MeshletSurface],
) -> MeshletScene {
    for _ in 0..4000 {
        let scene =
            MeshletScene::prepare(device, queue, pipelines, surfaces, (SIZE, SIZE), &frame());
        match scene.expect("the scene prepares") {
            Some(scene) if scene.waiting.is_empty() => return scene,
            _ => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    panic!("no pass was built");
}

/// The middle pixel of the colour target the surfaces draw into, under the plain view.
fn middle((device, queue): (&wgpu::Device, &wgpu::Queue), scene: &MeshletScene) -> [u8; 4] {
    let lighting = Lighting {
        view_proj: Mat4::IDENTITY,
        sky: Sky::default(),
        camera: Vec3::Z,
    };
    scene.update(queue, &lighting);
    let mut encoder = device.create_command_encoder(&Default::default());
    scene.record_targets(&mut encoder, |_, _| {});
    queue.submit([encoder.finish()]);
    let colour = scene.target(0).expect("a colour target");
    let read = read_back(device, queue, colour, colour.format(), SIZE);
    let at = (((SIZE / 2) * SIZE + SIZE / 2) * 4) as usize;
    [
        read.bytes[at],
        read.bytes[at + 1],
        read.bytes[at + 2],
        read.bytes[at + 3],
    ]
}

/// `model` with `image` as its texture, or none, which is the blank texel.
// ⚠ A pass is found by its stages' places in memory: surfaces of one pass are worn from one model.
fn wearing(model: &MeshletSurface, image: Option<Arc<Image>>) -> MeshletSurface {
    let mut worn = model.clone();
    worn.feeds.insert((1, 1), Feed::Textures(vec![image]));
    worn
}

fn red() -> Arc<Image> {
    Arc::new(Image {
        width: 1,
        height: 1,
        rgba: vec![255, 0, 0, 255],
        srgb: false,
    })
}

#[test]
fn a_group_s_digest_moves_with_its_own_feeds_and_with_the_image_it_binds_alone() {
    let (one, other) = (red(), red());
    let model = surface();
    let bound = |image: &Arc<Image>| wearing(&model, Some(Arc::clone(image))).feeds;
    let (first, again, second) = (bound(&one), bound(&one), bound(&other));
    assert_eq!(digest(&first, 1), digest(&again, 1), "the same image");
    assert_ne!(
        digest(&first, 1),
        digest(&second, 1),
        "another image of the same texels"
    );
    assert_eq!(
        digest(&first, 0),
        digest(&second, 0),
        "the mesh stage's group binds neither"
    );
    let mut tinted = first.clone();
    tinted.insert(
        (1, 0),
        Feed::Bytes(bytemuck::cast_slice(&[0.5f32; 4]).to_vec()),
    );
    assert_ne!(digest(&first, 1), digest(&tinted, 1), "another constant");
    assert_eq!(
        digest(&first, 2),
        digest(&BTreeMap::new(), 2),
        "a group fed nothing"
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn two_surfaces_that_bind_one_map_and_the_same_bytes_upload_them_once() {
    let Some((device, queue)) = gpu("two_surfaces_that_bind_one_map_and_the_same_bytes") else {
        return;
    };
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pipelines = MeshletPipelines::default();
    let map = red();
    let model = surface();
    let surfaces = [
        wearing(&model, Some(Arc::clone(&map))),
        wearing(&model, Some(map)),
    ];
    let scene = prepared((&device, &queue), &mut pipelines, &surfaces);
    assert_eq!((scene.drawn(), pipelines.len()), (2, 1));
    // The vertices, the meshlets and the tint once; the view a buffer a surface, which a frame
    // writes; one texture of four bytes; one sampler; two groups a surface.
    let made = Uploads {
        buffers: 3 + 2,
        buffer_bytes: scene.uploads().buffer_bytes,
        textures: 1,
        image_bytes: 4,
        samplers: 1,
        groups: 4,
        rebound: 0,
    };
    assert_eq!(scene.uploads(), made);
    // Each surface its own view, which the light's matrix is copied into in turn.
    let views: Vec<_> = (scene.draws.iter()).map(|draw| draw.views.len()).collect();
    assert_eq!(views, [1, 1]);
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_map_that_comes_in_late_binds_the_group_that_reads_it_and_builds_no_pass() {
    let Some((device, queue)) = gpu("a_map_that_comes_in_late_binds_the_group_that_reads_it")
    else {
        return;
    };
    let gpu = (&device, &queue);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut pipelines = MeshletPipelines::default();
    // Drawn by its own pass with the blank texel while its map is decoded.
    let model = surface();
    let waiting = [wearing(&model, None), wearing(&model, None)];
    let mut scene = prepared(gpu, &mut pipelines, &waiting);
    let before = scene.uploads();
    let [_, green, ..] = middle(gpu, &scene);
    assert!(green > 50, "white by its tint and shade: {green}");

    let map = red();
    let arrived = [
        wearing(&model, Some(Arc::clone(&map))),
        wearing(&model, Some(map)),
    ];
    let rebound = scene.rebind(&device, &queue, &arrived);
    assert_eq!(
        rebound,
        Ok(Some(2)),
        "the pixel stage's group of each surface"
    );
    assert_eq!(pipelines.len(), 1, "no other pass");
    let after = scene.uploads();
    let made = Uploads {
        textures: before.textures + 1,
        image_bytes: before.image_bytes + 4,
        groups: before.groups + 2,
        rebound: 2,
        ..before
    };
    assert_eq!(after, made, "one texture and two groups, nothing else");
    let [red, green, ..] = middle(gpu, &scene);
    assert!(red > 200 && green < 5, "the map is drawn: {red} {green}");

    // Asked again with nothing new, nothing is bound; other surfaces are not the scene's.
    assert_eq!(scene.rebind(&device, &queue, &arrived), Ok(Some(0)));
    assert_eq!(scene.uploads(), after);
    let mut other = surface();
    other.fragment = Arc::new(naga::front::wgsl::parse_str(PIXEL).expect("parses"));
    let others = [other.clone(), other];
    assert_eq!(scene.rebind(&device, &queue, &others), Ok(None));
    let short = scene.rebind(&device, &queue, &arrived[..1]);
    assert_eq!(short, Ok(None), "one surface short");
    let refused = pollster::block_on(validation.pop()).map(|e| e.to_string());
    assert_eq!(refused, None);
}
