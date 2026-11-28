//! The traced writer of the sun's mask against the shadow map, where nothing stands in the light.

use super::scene_tests::{frames_on, leaning};
use super::tests::surface;
use super::*;
use crate::game_shaders::trace::{SceneArena, TracedMask, WalkModule};
use or_io::gta6::accel::Triangle;

/// A floor across the whole frame at a depth of a tenth, and nothing over it.
fn floor_alone() -> MeshletSurface {
    let floor: [[f32; 3]; 3] = [[-1.0, -1.0, 0.1], [3.0, -1.0, 0.1], [-1.0, 3.0, 0.1]];
    let positions: Vec<u32> = (floor.iter().chain(&floor))
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

/// A walk that commits no hit: every ray it is given reaches the sun.
fn missing_walk() -> WalkModule {
    let module = naga::front::wgsl::parse_str(
        "@group(0) @binding(0) var<storage, read> arena: array<u32>;
        @group(0) @binding(1) var<storage, read_write> registers_given: array<u32>;
        @group(0) @binding(2) var<storage, read_write> registers_left: array<u32>;
        @compute @workgroup_size(32) fn walk(@builtin(global_invocation_id) id: vec3<u32>) {
            registers_left[id.x] = arena[0] * registers_given[id.x] * 0u;
        }",
    )
    .expect("the walk parses");
    WalkModule {
        module: Arc::new(module),
        given: vec!["far".to_owned()],
        left: 1,
        threads: [32, 1, 1],
        shared_dwords: 0,
        bound: 0xFFFF,
        unbounded: 0,
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn where_nothing_stands_in_the_light_the_traced_mask_lights_the_frame_as_the_shadow_map() {
    // No meter: one would expose a floor dimmed all over back to the same frame.
    // The box the floor stands in, so that the shadow map casts it and holds it clear of itself.
    let mut sky = Sky {
        sun: Vec3::splat(0.5),
        ..leaning()
    };
    sky.scene.bounds = Some((Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.2)));
    // The moving frame, every sample of the still view, and the converged frames after them.
    let frames = 1 + 64 + 2;
    let skies = vec![(Mat4::IDENTITY, sky); frames];
    let test = "where_nothing_stands_in_the_light";
    let Some(mapped) = frames_on(test, floor_alone(), &skies, |_, _, _| {}) else {
        return;
    };
    // Far below the floor, out of every ray's way, as the walk misses it anyway.
    let off = Triangle {
        corners: [
            [50.0, 50.0, -50.0],
            [51.0, 50.0, -50.0],
            [50.0, 51.0, -50.0],
        ],
        geometry: 0,
        primitive: 0,
        opaque: true,
    };
    let arena = Arc::new(SceneArena::build(&[off]).expect("the arena builds"));
    let walk = Arc::new(missing_walk());
    let hand = |scene: &mut MeshletScene, (device, queue): (&wgpu::Device, &wgpu::Queue), at| {
        if at > 0 {
            // Each ray starts where the shadow map's test does.
            let lift = scene.traced.borrow().as_ref().map(TracedMask::lift);
            assert_eq!(lift, Some(sun_shadow::held_off(&sky)));
            assert!(lift > Some(0.0));
            return;
        }
        let traced = TracedMask::new(device, queue, Arc::clone(&walk), Arc::clone(&arena), 1);
        scene.trace_with(Some(traced.expect("the device runs the walk")));
        while scene.traced_state() == Some(None) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(scene.traced_state(), Some(Some(Ok(()))));
    };
    let Some(traced) = frames_on(test, floor_alone(), &skies, hand) else {
        return;
    };
    assert!(
        mapped[0].chunks(4).all(|pixel| pixel[0] > 0),
        "the floor is lit"
    );
    for (at, (traced, mapped)) in traced.iter().zip(&mapped).enumerate() {
        assert!(traced == mapped, "frame {at} differs");
    }
}
