use std::sync::Arc;

use glam::{Mat4, Vec3, Vec4};
use or_io::gta6::accel::{candidates, walk_lane, Hit, Outcome, Ray, Triangle, Walk};

use super::pass::{
    bindings_of, disc_point, mask_program, rays_program, refused_by, roles, seed_stack,
    tiles_for_test, WalkBindings, BIAS,
};
use super::*;

fn triangle(corners: [[f32; 3]; 3], primitive: u32) -> Triangle {
    Triangle {
        corners,
        geometry: 0,
        primitive,
        opaque: true,
    }
}

/// A unit square of two triangles that share an edge, at height `z`.
fn square(z: f32) -> [Triangle; 2] {
    [
        triangle([[0.0, 0.0, z], [1.0, 0.0, z], [0.0, 1.0, z]], 0),
        triangle([[1.0, 0.0, z], [1.0, 1.0, z], [0.0, 1.0, z]], 1),
    ]
}

fn walked(arena: &SceneArena, ray: &Ray) -> Outcome {
    walk_lane(
        arena.words(),
        arena.top.base,
        ray,
        &Walk::default(),
        &mut |_: &Hit| true,
    )
}

fn down_at(x: f32, y: f32, from: f32) -> Ray {
    Ray {
        origin: [x, y, from],
        direction: [0.0, 0.0, -1.0],
        t_min: 0.0,
        t_max: 4.0,
    }
}

/// A view straight down onto the plane z = 0, which a depth of a quarter shows: x and y from -2
/// to 2 across the frame, and a depth of z / 4 + 1/4, nearer the camera the higher.
fn from_above() -> Mat4 {
    Mat4::from_cols(
        Vec4::new(0.5, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 0.5, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.25, 0.0),
        Vec4::new(0.0, 0.0, 0.25, 1.0),
    )
}

#[test]
fn a_pixel_s_sun_ray_meets_the_triangle_above_it_at_the_distance_the_arithmetic_gives() {
    let view = PixelView {
        unproject: from_above().inverse(),
        towards: Vec3::Z,
        camera: Vec3::new(0.0, 0.0, 10.0),
        disc: 0.01,
        clouds: 0.75,
    };
    // A triangle a unit over the plane, over the pixel at (5, 9) of 16 by 16.
    let roof = [triangle(
        [[-1.0, -1.0, 1.0], [0.5, -1.0, 1.0], [-1.0, 0.5, 1.0]],
        0,
    )];
    let arena = SceneArena::build(&roof).unwrap();
    let size = (16, 16);
    let cast = cast_of(&view, ([5, 9], size), (0.25, Vec3::Z), 0, arena.sphere());
    // The pixel's middle is (5.5 / 16, 9.5 / 16) of the frame, down being -y.
    let place = Vec3::new(
        (5.5 / 16.0 * 2.0 - 1.0) * 2.0,
        -(9.5 / 16.0 * 2.0 - 1.0) * 2.0,
        0.0,
    );
    let lift = BIAS * place.distance(view.camera);
    let ray = cast.ray.expect("a surface facing the sun casts a ray");
    assert_eq!(
        cast.share, 0.75,
        "what the clouds let through, where it reaches the sun"
    );
    assert!(
        (Vec3::from(ray.origin) - (place + Vec3::Z * lift)).length() < 1e-6,
        "{ray:?}"
    );
    assert_eq!(
        ray.direction,
        [0.0, 0.0, 1.0],
        "the first sample is cast at the disc's middle"
    );
    let (centre, radius) = arena.sphere();
    assert!(
        (ray.t_max - (Vec3::from(ray.origin).distance(Vec3::from(centre)) + radius)).abs() < 1e-6
    );
    let hit = walked(&arena, &ray)
        .hit
        .expect("the triangle over the pixel");
    let wanted = 1.0 - ray.origin[2];
    // The walk divides a distance out of the triangle's own arithmetic: a few units in the last place.
    assert!((hit.t - wanted).abs() <= 1e-6, "{} against {wanted}", hit.t);
    // A pixel the triangle is not over, one that faces away, and one that shows nothing.
    let beside = cast_of(&view, ([12, 4], size), (0.25, Vec3::Z), 0, arena.sphere());
    assert_eq!(walked(&arena, &beside.ray.unwrap()).hit, None);
    let away = cast_of(&view, ([5, 9], size), (0.25, -Vec3::Z), 0, arena.sphere());
    assert_eq!((away.ray, away.share), (None, 0.0));
    let nothing = cast_of(&view, ([5, 9], size), (0.0, Vec3::Z), 0, arena.sphere());
    assert_eq!((nothing.ray, nothing.share), (None, 1.0));
    // A later sample leans within the disc, and never past it.
    let leaning = cast_of(&view, ([5, 9], size), (0.25, Vec3::Z), 9, arena.sphere())
        .ray
        .unwrap();
    let off = Vec3::from(leaning.direction).angle_between(Vec3::Z).tan();
    assert!(off > 0.0 && off <= view.disc * 1.001, "{off}");
    assert_eq!(disc_point(0, [3, 4]), [0.0, 0.0]);
}

#[test]
fn a_ray_held_off_as_the_shadow_map_holds_a_surface_leaves_the_plane_its_normal_lies_along() {
    let view = PixelView {
        unproject: from_above().inverse(),
        towards: Vec3::new(0.0, -0.6, 0.8),
        camera: Vec3::new(0.0, 0.0, 10.0),
        disc: 0.01,
        clouds: 1.0,
    };
    // The plane the pixel shows is drawn, and its targets state a normal along it, as a decal's
    // can: pushed along that normal the ray starts on the plane and meets it at once.
    let plane = [triangle(
        [[-4.0, -4.0, 0.0], [4.0, -4.0, 0.0], [-4.0, 4.0, 0.0]],
        0,
    )];
    let arena = SceneArena::build(&plane).unwrap();
    let along = Vec3::NEG_Y;
    let at = ([5, 9], (16, 16));
    let pushed = cast_of(&view, at, (0.25, along), 0, arena.sphere())
        .ray
        .unwrap();
    assert!(walked(&arena, &pushed).hit.is_some(), "{pushed:?}");
    let lift = 0.01;
    let held = cast_held_off(&view, at, (0.25, along), 0, arena.sphere(), lift);
    let ray = held.ray.unwrap();
    let start = Vec3::from(pushed.origin) + view.towards * lift;
    assert!((Vec3::from(ray.origin) - start).length() < 1e-6, "{ray:?}");
    assert_eq!(walked(&arena, &ray).hit, None);
}

#[test]
fn a_still_view_casts_a_ray_a_pixel_for_sixty_four_frames_then_none_and_a_moving_one_a_quarter() {
    let size = (1920, 1080);
    let mut governor = Governor::default();
    let first = governor.next(true);
    assert!(matches!(first, Rate::Quarter { .. }));
    assert_eq!(first.rays(size), 518_400);
    let mut still = 0;
    for frame in 0..SAMPLES {
        let rate = governor.next(false);
        assert_eq!(rate, Rate::Full { sample: frame });
        still += rate.rays(size);
    }
    assert_eq!(still, 64 * 2_073_600);
    for _ in 0..3 {
        assert_eq!(governor.next(false), Rate::Converged);
        assert_eq!(Rate::Converged.rays(size), 0);
    }
    // Moving again starts over, at the next corner of the block each frame.
    let corners: Vec<[u32; 2]> = (0..4).map(|_| governor.next(true).offset()).collect();
    let mut distinct = corners.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "{corners:?}");
    assert_eq!(governor.next(false), Rate::Full { sample: 0 });
    // An odd frame's last blocks are cut by its edge.
    assert_eq!(Rate::Quarter { phase: 0 }.rays((3, 5)), 2 * 3);
    // The rays of a frame cut into tiles a walk dispatches over whole groups of its lanes.
    let tiles = tiles_for_test(518_400, 32);
    assert_eq!(tiles.len(), 8);
    assert_eq!(tiles.iter().map(|t| u64::from(t.0)).sum::<u64>(), 518_400);
    assert!(tiles.iter().all(|&(count, padded)| count <= pass::TILE_RAYS
        && padded >= count
        && padded % 32 == 0
        && padded - count < 32));
    assert!(tiles_for_test(0, 32).is_empty());
}

#[test]
fn the_arena_of_a_two_triangle_model_answers_the_reference_query() {
    let model = square(0.0);
    let arena = SceneArena::build(&model).unwrap();
    let words = arena.words();
    let mut leaves = Vec::new();
    for (x, y) in [(0.25, 0.25), (0.75, 0.75)] {
        let ray = down_at(x, y, 1.0);
        let hit = walked(&arena, &ray).hit.expect("a ray onto the square");
        assert!((hit.t - 1.0).abs() < 1e-6);
        let met = candidates(words, arena.top.base, &ray, 0xFF);
        assert_eq!(met.len(), 1, "one triangle under {x}, {y}");
        assert_eq!((met[0].leaf, met[0].instance), (hit.leaf, hit.instance));
        leaves.push(hit.leaf);
    }
    assert_ne!(leaves[0], leaves[1], "each half is its own triangle");
    let beside = down_at(1.5, 0.5, 1.0);
    assert_eq!(walked(&arena, &beside).hit, None);
    assert!(candidates(words, arena.top.base, &beside, 0xFF).is_empty());
}

#[test]
fn an_arena_refit_to_a_pose_answers_as_one_built_of_the_posed_triangles() {
    let mut arena = SceneArena::build(&square(0.0)).unwrap();
    let raised = square(1.0);
    let ray = Ray {
        origin: [0.25, 0.25, 0.5],
        direction: [0.0, 0.0, 1.0],
        t_min: 0.0,
        t_max: 2.0,
    };
    assert_eq!(
        walked(&arena, &ray).hit,
        None,
        "the square lies behind the ray as built"
    );
    let ranges = arena.refit(&raised).unwrap();
    let length = arena.words().len() as u32 * 4;
    assert!(
        ranges.iter().all(|r| r.start < r.end && r.end <= length),
        "{ranges:?}"
    );
    let refit = walked(&arena, &ray).hit.expect("the raised square");
    let built = walked(&SceneArena::build(&raised).unwrap(), &ray)
        .hit
        .unwrap();
    assert_eq!(refit.t.to_bits(), built.t.to_bits());
    assert!(
        arena.refit(&raised[..1]).is_err(),
        "a pose that lacks a triangle"
    );
}

fn walk_of(given: &[&str], shared_dwords: u32) -> WalkModule {
    WalkModule {
        module: Arc::new(naga::Module::default()),
        given: given.iter().map(|word| (*word).to_owned()).collect(),
        left: 5,
        threads: [32, 1, 1],
        shared_dwords,
        bound: 0xFFFF,
        unbounded: 0,
    }
}

#[test]
fn a_walk_is_given_each_register_as_its_file_states_it() {
    let arena = SceneArena::build(&square(0.0)).unwrap();
    let walk = walk_of(
        &[
            "root",
            "top",
            "top_bytes",
            "one",
            "ones",
            "zero",
            "lane4",
            "far",
            "oy",
            "dz",
            "0xffff",
            "7",
        ],
        2048,
    );
    let top = arena.top.base;
    assert_eq!(
        roles(&walk, &arena).unwrap(),
        [
            Role::Value(arena.root()),
            Role::Value(top >> 8),
            Role::Value(top),
            Role::Value(1),
            Role::Value(u32::MAX),
            Role::Value(0),
            Role::Lane4,
            Role::Far,
            Role::Origin(1),
            Role::Direction(2),
            Role::Value(0xFFFF),
            Role::Value(7),
        ]
    );
    let header = or_io::gta6::accel::layout::HEADER_ROOT;
    assert_eq!(
        arena.root(),
        or_io::gta6::accel::word(arena.words(), top, header)
    );
    assert_eq!(
        arena.root(),
        arena.top.root,
        "the header states the tree's own root"
    );
    let why = roles(&walk_of(&["ow"], 0), &arena).unwrap_err();
    assert!(why.contains("ow"), "{why}");
}

fn wgsl(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the WGSL parses")
}

fn valid(module: &naga::Module) {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .expect("a module the validator takes");
}

#[test]
fn every_lane_s_first_stack_entry_is_seeded_before_the_walk_and_nothing_else_moves() {
    let mut module = wgsl(
        "var<workgroup> words: array<u32, 48>;
        @compute @workgroup_size(32) fn main(@builtin(local_invocation_index) i: u32) {
            words[i] = words[i] + 1u;
        }",
    );
    let unseeded = format!("{module:?}");
    assert_eq!(
        format!("{:?}", seed_stack(&module, 32)),
        unseeded,
        "no stack of that name"
    );
    let (handle, _) = module.global_variables.iter().next().unwrap();
    module.global_variables[handle].name = Some("shared".into());
    let seeded = seed_stack(&module, 32);
    valid(&seeded);
    let body = &seeded.entry_points[0].function.body;
    let stores = body
        .iter()
        .take_while(|s| !matches!(s, naga::Statement::ControlBarrier(_)));
    let stored = stores
        .filter(|s| matches!(s, naga::Statement::Store { .. }))
        .count();
    assert_eq!(stored, 32);
    assert!(
        matches!(body[2 * 32], naga::Statement::ControlBarrier(b) if b == naga::Barrier::WORK_GROUP)
    );
    assert_eq!(
        body.len(),
        module.entry_points[0].function.body.len() + 2 * 32 + 1
    );
    // No more dwords are stored than the memory holds.
    let over = seed_stack(&module, 64);
    let body = &over.entry_points[0].function.body;
    assert!(matches!(body[2 * 48], naga::Statement::ControlBarrier(_)));
}

const BOUND: &str = "@group(0) @binding(0) var<storage, read> arena: array<u32>;
    @group(0) @binding(1) var<storage, read> registers_given: array<u32>;
    @group(0) @binding(2) var<storage, read_write> registers_left: array<u32>;";

#[test]
fn a_walk_is_bound_by_what_the_pass_fills_and_refused_where_it_binds_more() {
    let entry =
        "@compute @workgroup_size(32) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
        registers_left[id.x] = arena[registers_given[id.x]];
    }";
    let module = wgsl(&format!("{BOUND}\n{entry}"));
    assert_eq!(
        bindings_of(&module),
        Ok(WalkBindings {
            arena: 0,
            given: 1,
            left: 2,
            structures: None,
        })
    );
    assert_eq!(pass::walk_entry(&module), Some("main"));
    let more = wgsl(&format!(
        "{BOUND}\n@group(0) @binding(3) var<storage, read> table: array<u32>;\n{entry}"
    ));
    let why = bindings_of(&more).unwrap_err();
    assert!(why.contains("table"), "{why}");
}

#[test]
fn a_walk_that_binds_the_device_s_structure_is_refused_without_one_and_bound_with_one() {
    let entry =
        "@compute @workgroup_size(32) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
        registers_left[id.x] = arena[registers_given[id.x]];
    }";
    let mut module = wgsl(&format!("{BOUND}\n{entry}"));
    let plain = bindings_of(&module).unwrap();
    assert_eq!(pass::structure_slot(&plain, false), Ok(None));
    // The device's top level, as body Q declares it, under any name.
    let ty = naga::Type {
        name: None,
        inner: naga::TypeInner::AccelerationStructure {
            vertex_return: false,
        },
    };
    let ty = module.types.insert(ty, naga::Span::UNDEFINED);
    let global = naga::GlobalVariable {
        name: Some("a_structure".into()),
        space: naga::AddressSpace::Handle,
        binding: Some(naga::ResourceBinding {
            group: 0,
            binding: 5,
        }),
        ty,
        init: None,
        memory_decorations: naga::MemoryDecorations::empty(),
    };
    module
        .global_variables
        .append(global, naga::Span::UNDEFINED);
    let asked = bindings_of(&module).unwrap();
    assert_eq!(asked.structures, Some(5));
    let why = pass::structure_slot(&asked, false).unwrap_err();
    assert!(why.contains("structure"), "{why}");
    assert_eq!(pass::structure_slot(&asked, true), Ok(Some(5)));
}

#[test]
fn a_walk_through_the_query_is_given_the_query_s_arena_and_its_root_and_the_plain_walk_the_arena_s_own(
) {
    let arena = SceneArena::build(&square(0.0)).unwrap();
    let walk = walk_of(&["root", "top_bytes", "zero"], 2048);
    let plain = arena.words().to_vec();
    let (none, roles_s) = pass::given_arena(&walk, &arena, false).unwrap();
    assert!(none.is_none(), "body S binds the arena's own words");
    assert_eq!(roles_s, roles(&walk, &arena).unwrap());
    let (asked, roles_q) = pass::given_arena(&walk, &arena, true).unwrap();
    let asked = asked.expect("body Q binds the query's copy");
    let header = or_io::gta6::accel::layout::HEADER_ROOT;
    let root = or_io::gta6::accel::word(&asked, arena.top.base, header);
    assert_ne!(
        root,
        arena.root(),
        "the copy states the query's box as its root"
    );
    assert_eq!(
        roles_q,
        [
            Role::Value(root),
            Role::Value(arena.top.base),
            Role::Value(0)
        ]
    );
    // The scene's own words, which the CPU's reference walks, are left as built.
    assert_eq!(arena.words(), plain.as_slice());
}

#[test]
fn a_device_without_what_the_walk_needs_keeps_the_shadow_map_and_says_why() {
    let given = ["zero"; 24];
    let walk = walk_of(&given, 2048);
    assert_eq!(refused_by(&wgpu::Limits::default(), &walk, 1 << 20), None);
    let no_compute = refused_by(&wgpu::Limits::downlevel_webgl2_defaults(), &walk, 1 << 20);
    assert!(no_compute.is_some_and(|why| why.contains("compute")));
    let crowded = refused_by(&wgpu::Limits::default(), &walk_of(&given, 1 << 20), 1 << 20);
    assert!(
        crowded.is_some_and(|why| why.contains("shares")),
        "the group's memory"
    );
    let large = refused_by(&wgpu::Limits::default(), &walk, 1 << 40);
    assert!(large.is_some_and(|why| why.contains("arena")));
}

#[test]
fn the_aid_names_the_traced_writer_and_its_walk_file() {
    assert_eq!(traced_of("target=3,traced"), Some(String::new()));
    assert_eq!(
        traced_of("traced=C:/a b/w.walk, plain-device"),
        Some("C:/a b/w.walk".into())
    );
    assert_eq!(traced_of("plain-device,traceds"), None);
    assert_eq!(traced_of(""), None);
}

#[test]
fn the_programs_around_the_walk_are_modules_the_validator_takes() {
    for program in [rays_program(), mask_program()] {
        valid(&wgsl(&program));
    }
    let rays = wgsl(&rays_program());
    assert!(rays.entry_points.iter().any(|e| e.name == "cast_rays"));
}
