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
            let (outcome, _) = build(&gpu, &ready.stages, None, Draw::No);
            assert_eq!(outcome, Outcome::Created, "{:?}: {source}", gpu.info);
        }
    }
}

#[test]
fn a_pair_whose_stages_do_not_link_is_rejected_in_the_devices_words() {
    for gpu in gpus() {
        // Each module is valid alone; the pixel stage reads a location the vertex stage never writes.
        let stages = Stages::Render(wgsl(NO_OUTPUT), Some(wgsl(COLOUR)));
        let (outcome, _) = build(&gpu, &stages, None, Draw::No);
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::No);
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::No);
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
        let (outcome, _) = build(&gpu, &ready.stages, Some(&cache), Draw::No);
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
            tried(&gpu, &ready, None, Draw::No).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
        gpu.features.remove(wgpu::Features::CLIP_DISTANCES);
        assert_eq!(
            tried(&gpu, &ready, None, Draw::No).0,
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
            tried(&gpu, &ready, None, Draw::No).0,
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
            tried(&gpu, &ready, None, Draw::No).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
    }
}

const SAMPLED: &str = "@group(0) @binding(0) var map: texture_2d<f32>;
@group(0) @binding(1) var taps: sampler;
@group(0) @binding(2) var<uniform> tint: vec4<f32>;
@fragment fn main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    return textureSample(map, taps, at.xy) * tint;
}";

#[test]
fn a_pipeline_the_check_is_asked_to_draw_with_is_drawn_with_stand_ins_at_every_binding() {
    let array = wgpu::Features::TEXTURE_BINDING_ARRAY
        | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING;
    for gpu in gpus() {
        for source in [COLOUR, NO_OUTPUT, COUNTER, SAMPLED, ARRAY] {
            if source == ARRAY && !gpu.features.contains(array) {
                continue;
            }
            let ready = alone(wgsl(source), false).unwrap();
            let (outcome, _) = tried(&gpu, &ready, None, Draw::Empty);
            assert_eq!(outcome, Outcome::Created, "{:?}: {source}", gpu.info);
        }
    }
}

#[test]
fn a_draw_with_a_binding_left_out_is_the_units_refusal() {
    for gpu in gpus() {
        let ready = alone(wgsl(SAMPLED), false).unwrap();
        let mut made = None;
        let created = scoped(&gpu.device, || {
            made = Some(create(&gpu.device, &ready.stages, None, Draw::Empty)?);
            Ok(())
        });
        assert_eq!(created, Outcome::Created, "{:?}", gpu.info);
        let mut made = made.unwrap();
        // The group's layout stays, and what the draw would bind in it is gone.
        made.stated.as_mut().unwrap().0.clear();
        let drawn = scoped(&gpu.device, || {
            made.draw(&gpu, &made.bound(&gpu.device)?, false)
        });
        assert!(
            matches!(drawn, Outcome::Rejected(_)),
            "{:?}: {drawn:?}",
            gpu.info
        );
    }
}

#[test]
fn a_texture_of_wide_texels_is_skipped_where_the_adapter_offers_the_feature_and_updates_none() {
    let wide = "
@group(0) @binding(0) var cells: texture_storage_2d<r64uint, atomic>;
@compute @workgroup_size(1) fn main() { textureAtomicMax(cells, vec2<i32>(0, 0), 1lu); }";
    let ready = alone(wgsl(wide), false).unwrap();
    for mut gpu in gpus() {
        gpu.features |= ready.needs;
        gpu.updates_64_bit_textures = false;
        let (outcome, _) = tried(&gpu, &ready, None, Draw::No);
        assert_eq!(outcome, Outcome::Skipped(NO_64_BIT_UPDATE.into()));
    }
}

#[test]
fn a_unit_is_written_as_metals_language_with_no_device() {
    for source in [COLOUR, NO_OUTPUT, COUNTER, SAMPLED] {
        let ready = alone(wgsl(source), false).unwrap();
        assert_eq!(
            metal_text(&ready.stages, None),
            Outcome::Created,
            "{source}"
        );
    }
    // Each module is valid alone and the writer takes each: only a device links the two.
    let unlinked = Stages::Render(wgsl(NO_OUTPUT), Some(wgsl(COLOUR)));
    assert_eq!(metal_text(&unlinked, None), Outcome::Created);
    assert_eq!(metal_text_adapter().backend, "Metal");
}

const WIDE: &str = "
@group(0) @binding(0) var cells: texture_storage_2d<r64uint, atomic>;
@group(0) @binding(1) var<uniform> tint: vec4<f32>;
@compute @workgroup_size(1) fn main() { textureAtomicMax(cells, vec2<i32>(0, 0), u64(tint.x)); }";

#[test]
fn a_texture_of_wide_texels_is_laid_out_and_drawn_with_where_the_adapter_updates_one() {
    let ready = alone(wgsl(WIDE), false).unwrap();
    for gpu in gpus() {
        if !gpu.updates_64_bit_textures || !gpu.features.contains(ready.needs) {
            continue;
        }
        let (outcome, _) = tried(&gpu, &ready, None, Draw::Empty);
        assert_eq!(outcome, Outcome::Created, "{:?}", gpu.info);
    }
}

fn adapter(kind: wgpu::DeviceType, name: &str) -> wgpu::AdapterInfo {
    let mut info = described_as(0, wgpu::Backend::Dx12, kind);
    info.name = name.into();
    info
}

#[test]
fn only_a_cpus_adapter_is_opened_where_the_software_one_is_asked_for() {
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu};
    let (card, built_in, warp) = (
        adapter(DiscreteGpu, "A Card"),
        adapter(IntegratedGpu, "Built In"),
        adapter(Cpu, "Basic Render"),
    );
    assert!(wanted(&card, None, false) && wanted(&warp, None, false));
    assert!(!wanted(&card, None, true) && !wanted(&built_in, None, true));
    assert!(wanted(&warp, None, true));
    // The name is still asked of it.
    assert!(wanted(&warp, Some("basic"), true) && !wanted(&warp, Some("card"), true));
    assert!(wanted(&card, Some("card (dx12)"), false));
}

/// The dwords of the storage buffer a draw with `made` holds at its first binding, afterwards.
fn cells_after(gpu: &Gpu, stages: &Stages, draw: Draw) -> Vec<u32> {
    let made = create(&gpu.device, stages, None, draw).unwrap();
    let bound = made.bound(&gpu.device).unwrap();
    made.draw(gpu, &bound, draw == Draw::Run).unwrap();
    let cells = (bound.held.iter().flatten())
        .find_map(|held| match held {
            Held::Buffers(buffers) => buffers.first(),
            _ => None,
        })
        .expect("the test's module binds a buffer");
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: cells.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(cells, 0, &read, 0, cells.size());
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = read.slice(..).get_mapped_range();
    let (dwords, _) = bytes.as_chunks::<4>();
    dwords
        .iter()
        .map(|dword| u32::from_le_bytes(*dword))
        .collect()
}

const MARKS: &str = "@group(0) @binding(0) var<storage, read_write> cells: array<u32>;
@fragment fn main() -> @location(0) vec4<f32> { cells[1] = 7u; return vec4<f32>(1.0); }";

#[test]
fn a_program_that_is_run_writes_what_it_computes_and_one_drawn_with_no_vertex_does_not() {
    for gpu in gpus() {
        // A program is run on a CPU alone, and this test with it.
        if gpu.info.device_type != wgpu::DeviceType::Cpu {
            continue;
        }
        let counter = alone(wgsl(COUNTER), false).unwrap();
        assert_eq!(cells_after(&gpu, &counter.stages, Draw::Empty), [0; 4]);
        assert_eq!(cells_after(&gpu, &counter.stages, Draw::Run), [0, 1, 2, 3]);
        // A pixel program's partner gives every corner the middle of the target: a point covers it.
        let marks = alone(wgsl(MARKS), false).unwrap();
        assert_eq!(cells_after(&gpu, &marks.stages, Draw::Empty)[1], 0);
        assert_eq!(cells_after(&gpu, &marks.stages, Draw::Run)[1], 7);
    }
}

#[test]
fn a_program_is_not_run_on_an_adapter_that_is_no_cpus() {
    let ready = alone(wgsl(COUNTER), false).unwrap();
    for mut gpu in gpus() {
        gpu.info.device_type = wgpu::DeviceType::DiscreteGpu;
        let (outcome, _) = tried(&gpu, &ready, None, Draw::Run);
        assert_eq!(outcome, Outcome::Skipped(RUN_ON_SOFTWARE.into()));
    }
}

#[test]
fn a_unit_that_updates_a_texture_in_a_step_is_skipped_by_the_metal_version_that_does() {
    let source = "@group(0) @binding(0) var cells: texture_storage_2d<r32uint, atomic>;
@compute @workgroup_size(1) fn main() { textureAtomicMax(cells, vec2<i32>(0, 0), 1u); }";
    let ready = alone(wgsl(source), false).unwrap();
    assert_eq!(
        metal_text(&ready.stages, None),
        Outcome::Skipped("Metal's language 3.1 (macOS 14, iOS 17)".into())
    );
}

#[test]
fn a_stage_that_binds_more_than_metal_does_is_refused_by_the_count() {
    let mut source = String::new();
    for at in 0..17 {
        source.push_str(&format!("@group(0) @binding({at}) var s{at}: sampler;\n"));
    }
    source.push_str("@group(0) @binding(40) var map: texture_2d<f32>;\n@fragment fn main() -> @location(0) vec4<f32> {\n    var sum = vec4<f32>(0.0);\n");
    for at in 0..17 {
        source.push_str(&format!(
            "    sum += textureSampleLevel(map, s{at}, vec2<f32>(0.0), 0.0);\n"
        ));
    }
    source.push_str("    return sum;\n}");
    let ready = alone(wgsl(&source), false).unwrap();
    let Outcome::Rejected(why) = metal_text(&ready.stages, None) else {
        panic!("seventeen samplers were taken");
    };
    assert_eq!(
        why,
        "Metal binds 16 samplers a stage, and the pixel stage binds 17"
    );
}

#[test]
fn the_metal_text_of_each_stage_is_kept_under_its_version() {
    let dir = tempfile::tempdir().unwrap();
    let ready = alone(wgsl(COLOUR), false).unwrap();
    let outcome = metal_text(&ready.stages, Some((dir.path(), "v1+p2")));
    assert_eq!(outcome, Outcome::Created);
    for stage in ["vertex", "pixel"] {
        let file = dir.path().join("3.0").join(format!("v1+p2.{stage}.metal"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("metal_stdlib"), "{}", file.display());
    }
    // Numbered as a Metal device numbers them: a compiler takes no made-up place.
    let ready = alone(wgsl(SAMPLED), false).unwrap();
    assert_eq!(
        metal_text(&ready.stages, Some((dir.path(), "5"))),
        Outcome::Created
    );
    let text = std::fs::read_to_string(dir.path().join("3.0").join("5.pixel.metal")).unwrap();
    for place in ["[[texture(0)]]", "[[sampler(0)]]", "[[buffer(0)]]"] {
        assert!(text.contains(place), "{place} of {text}");
    }
    assert!(!text.contains("fake"), "{text}");
}

const NESTED: &str =
    "@vertex fn main(@builtin(vertex_index) at: u32) -> @builtin(position) vec4<f32> {
    var sum = 0u;
    for (var i = 0u; i < at; i++) { for (var j = 0u; j < i; j++) { sum += j; } }
    return vec4<f32>(f32(sum), 0.0, 0.0, 1.0);
}";

#[test]
fn a_stage_the_renderer_refuses_for_amds_direct3d_12_driver_is_noted_and_not_put_to_that_driver() {
    let nested = alone(wgsl(NESTED), false).unwrap();
    let noted = |ready: &Ready| ready.notes.iter().any(|note| note.starts_with(AMD_REFUSED));
    assert!(noted(&nested), "{:?}", nested.notes);
    assert!(!noted(&alone(wgsl(NO_OUTPUT), false).unwrap()));
    for mut gpu in gpus() {
        // Built on the adapter the machine has, whose description is not AMD's under Direct3D 12 here.
        if !instance::amd_under_dx12(&gpu.info) {
            assert_eq!(tried(&gpu, &nested, None, Draw::No).0, Outcome::Created);
        }
        (gpu.info.vendor, gpu.info.backend) = (0x1002, wgpu::Backend::Dx12);
        let (outcome, _) = tried(&gpu, &nested, None, Draw::No);
        assert!(matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");
    }
}

#[test]
fn a_unit_that_crosses_its_stages_with_more_locations_than_ios_links_is_noted() {
    let wide = |locations: usize| {
        let members: String = (0..locations)
            .map(|at| format!("@location({at}) v{at}: f32, "))
            .collect();
        let source = format!(
            "struct Out {{ @builtin(position) position: vec4<f32>, {members}}}
@vertex fn main() -> Out {{ var out: Out; return out; }}"
        );
        alone(wgsl(&source), false).unwrap()
    };
    assert!(wide(16).notes.iter().any(|note| note == PAST_IOS));
    assert!(!wide(15).notes.iter().any(|note| note == PAST_IOS));
}
