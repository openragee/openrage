use super::*;

/// Every adapter of the machine, or a recorded skip: a CI machine may have none.
fn gpus() -> Vec<Gpu> {
    let gpus = open_all(None);
    if gpus.is_empty() {
        or_test_support::skip_device("the native programs' device check", "no adapter");
    }
    gpus
}

fn wgsl(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the test's shader parses")
}

const COLOUR: &str =
    "@fragment fn main(@location(0) tint: vec4<f32>) -> @location(0) vec4<f32> { return tint; }";
const NO_OUTPUT: &str =
    "@vertex fn main() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }";
const COUNTER: &str = "@group(0) @binding(0) var<storage, read_write> cells: array<u32>;
@compute @workgroup_size(4) fn main(@builtin(global_invocation_id) at: vec3<u32>) { cells[at.x] = at.x; }";

#[test]
fn a_module_with_a_partner_that_feeds_it_is_created_on_every_adapter() {
    for gpu in gpus() {
        for source in [COLOUR, NO_OUTPUT, COUNTER] {
            let ready = alone(wgsl(source), false).unwrap();
            let (outcome, _) = build(&gpu, &ready.stages, None);
            assert_eq!(outcome, Outcome::Created, "{:?}: {source}", gpu.info);
        }
    }
}

#[test]
fn a_pair_whose_stages_do_not_link_is_rejected_in_the_devices_words() {
    for gpu in gpus() {
        // Each module is valid alone; the pixel stage reads a location the vertex stage never writes.
        let stages = Stages::Render(wgsl(NO_OUTPUT), Some(wgsl(COLOUR)));
        let (outcome, _) = build(&gpu, &stages, None);
        let Outcome::Rejected(why) = outcome else {
            panic!("{:?} created a pair that does not link", gpu.info);
        };
        assert!(why.contains("not provided"), "{why}");
    }
}

#[test]
fn a_module_of_subgroup_operations_is_skipped_where_the_device_has_none() {
    for mut gpu in gpus() {
        gpu.features.remove(wgpu::Features::SUBGROUP);
        let ready = alone(wgsl(NO_OUTPUT), true).unwrap();
        let (outcome, _) = tried(&gpu, &ready, None);
        assert_eq!(outcome, Outcome::Skipped("SUBGROUP".into()));
    }
}

#[test]
fn a_module_that_updates_a_texture_of_wide_texels_is_skipped_by_the_features_it_needs() {
    let wide = "
@group(0) @binding(0) var cells: texture_storage_2d<r64uint, atomic>;
@compute @workgroup_size(1) fn main() { textureAtomicMax(cells, vec2<i32>(0, 0), 1lu); }";
    let ready = alone(wgsl(wide), false).unwrap();
    for mut gpu in gpus() {
        gpu.features.remove(wgpu::Features::SHADER_INT64);
        gpu.features
            .insert(wgpu::Features::TEXTURE_ATOMIC | wgpu::Features::TEXTURE_INT64_ATOMIC);
        let (outcome, _) = tried(&gpu, &ready, None);
        assert_eq!(outcome, Outcome::Skipped("SHADER_INT64".into()));
    }
}

#[test]
fn what_a_module_asks_of_any_device_is_named() {
    assert_eq!(asked(&wgsl(COLOUR)), [] as [&str; 0]);
    assert_eq!(asked(&wgsl(CLIPPED)), ["CLIP_DISTANCE"]);
    // The pass a device with no clip distances is given asks for none.
    let ready = alone(wgsl(CLIPPED), false).unwrap();
    assert_eq!(ready.notes, [] as [&str; 0]);
    let ready = alone(wgsl(ARRAY), false).unwrap();
    assert!(
        ready.notes.iter().any(|note| note.starts_with("needs ")),
        "{:?}",
        ready.notes
    );
    // Past a cast to a 64-bit integer, which the validator refuses in words that name nothing.
    let wide = "enable clip_distances;
@group(0) @binding(0) var cells: texture_storage_2d<r64uint, atomic>;
struct Out { @builtin(position) position: vec4<f32>, @builtin(clip_distances) clip: array<f32, 1> }
@vertex fn main(@builtin(vertex_index) at: u32) -> Out {
    textureAtomicMax(cells, vec2<i32>(0, 0), u64(at));
    return Out(vec4<f32>(0.0), array<f32, 1>(0.0));
}";
    let all = asked(&wgsl(wide));
    for name in [
        "SHADER_INT64",
        "TEXTURE_INT64_ATOMIC",
        "TEXTURE_ATOMIC",
        "CLIP_DISTANCE",
    ] {
        assert!(all.iter().any(|asked| asked == name), "{name} of {all:?}");
    }
}

#[test]
fn a_saved_pipeline_cache_holds_data_where_the_backend_has_one() {
    for gpu in gpus() {
        if gpu.cache_key().is_none() {
            continue;
        }
        // SAFETY: no data is given.
        let cache = unsafe {
            gpu.device
                .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: None,
                    data: None,
                    fallback: true,
                })
        };
        let ready = alone(wgsl(COLOUR), false).unwrap();
        let (outcome, _) = build(&gpu, &ready.stages, Some(&cache));
        assert_eq!(outcome, Outcome::Created);
        assert!(cache.get_data().is_some_and(|data| !data.is_empty()));
    }
}

#[test]
fn a_unit_is_named_by_its_programs() {
    assert_eq!(Unit::Program(12).name(0), "12");
    assert_eq!(Unit::Program(12).name(1), "12.1");
    let pair = Unit::Pair {
        vertex: 3,
        pixel: 40,
    };
    assert_eq!(pair.name(0), "v3+p40");
}

const CLIPPED: &str = "enable clip_distances;
struct Out { @builtin(position) position: vec4<f32>, @builtin(clip_distances) clip: array<f32, 4>, @location(0) tint: vec4<f32> }
@vertex fn main(@builtin(vertex_index) at: u32) -> Out {
    let x = f32(at);
    return Out(vec4<f32>(x, 0.0, 0.0, 1.0), array<f32, 4>(x, 0.0, 0.0, 0.0), vec4<f32>(x));
}";
const ARRAY: &str = "@group(0) @binding(0) var maps: binding_array<texture_2d<f32>, 4>;
@group(0) @binding(1) var<uniform> pick: vec4<u32>;
@group(0) @binding(2) var<uniform> other: vec4<f32>;
@fragment fn main() -> @location(0) vec4<f32> { return textureLoad(maps[pick.x], vec2<i32>(0, 0), 0) + other; }";

#[test]
fn a_stage_that_gives_clip_distances_is_built_without_them_where_the_device_has_none() {
    let ready = alone(wgsl(CLIPPED), false).unwrap();
    assert!(ready.clips && ready.bare.is_some());
    for mut gpu in gpus() {
        // As lowered where the device clips by them, as a discard where it does not: created on both.
        assert_eq!(
            tried(&gpu, &ready, None).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
        gpu.features.remove(wgpu::Features::CLIP_DISTANCES);
        assert_eq!(
            tried(&gpu, &ready, None).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
    }
}

#[test]
fn a_texture_updated_in_a_step_beside_a_binding_array_is_laid_out_as_one() {
    let source = "@group(0) @binding(0) var maps: binding_array<texture_2d<f32>, 4>;
@group(0) @binding(1) var cells: texture_storage_2d<r32uint, atomic>;
@fragment fn main() -> @location(0) vec4<f32> {
    textureAtomicMax(cells, vec2<i32>(0, 0), 1u);
    return textureLoad(maps[1], vec2<i32>(0, 0), 0);
}";
    let wanted = wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::TEXTURE_ATOMIC;
    for gpu in gpus() {
        if !gpu.features.contains(wanted) {
            continue;
        }
        let ready = alone(wgsl(source), false).unwrap();
        assert_eq!(
            tried(&gpu, &ready, None).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
    }
}

#[test]
fn a_binding_array_is_built_in_a_group_of_its_own_with_its_length_stated() {
    let array = wgpu::Features::TEXTURE_BINDING_ARRAY
        | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
    for gpu in gpus() {
        if !gpu.features.contains(array) {
            continue;
        }
        // Beside two constant buffers in its group, and with a layout derived from the module,
        // Direct3D 12 places the buffers on the array's registers and wgpu refuses the group.
        let ready = alone(wgsl(ARRAY), false).unwrap();
        assert_eq!(
            tried(&gpu, &ready, None).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
    }
}
