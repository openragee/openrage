use super::metal::metal_resources;
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::No, false);
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::No, false);
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
            tried(&gpu, &ready, None, Draw::No, false).0,
            Outcome::Created,
            "{:?}",
            gpu.info
        );
        gpu.features.remove(wgpu::Features::CLIP_DISTANCES);
        assert_eq!(
            tried(&gpu, &ready, None, Draw::No, false).0,
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
            tried(&gpu, &ready, None, Draw::No, false).0,
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
            tried(&gpu, &ready, None, Draw::No, false).0,
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
            let (outcome, _) = tried(&gpu, &ready, None, Draw::Empty, false);
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::No, false);
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::Empty, false);
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
        let (outcome, _) = tried(&gpu, &ready, None, Draw::Run, false);
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
            assert_eq!(
                tried(&gpu, &nested, None, Draw::No, false).0,
                Outcome::Created
            );
        }
        (gpu.info.vendor, gpu.info.backend) = (0x1002, wgpu::Backend::Dx12);
        let (outcome, _) = tried(&gpu, &nested, None, Draw::No, false);
        assert!(matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");
        // Asked to, the check hands it to the driver, which here is the machine's own.
        let (outcome, _) = tried(&gpu, &nested, None, Draw::No, true);
        assert!(!matches!(outcome, Outcome::Skipped(_)), "{outcome:?}");
    }
}

#[test]
fn a_check_asked_to_try_what_the_renderer_refuses_puts_it_to_the_driver() {
    let nested = alone(wgsl(NESTED), false).unwrap();
    let amd = described_as(0x1002, wgpu::Backend::Dx12, wgpu::DeviceType::DiscreteGpu);
    assert!(kept_from(&nested.stages, &amd, false).is_some());
    assert_eq!(kept_from(&nested.stages, &amd, true), None);
    let plain = alone(wgsl(NO_OUTPUT), false).unwrap();
    assert_eq!(kept_from(&plain.stages, &amd, false), None);
}

/// A stage that binds `buffers` constant buffers, as its `stage` attribute and result say.
fn many_buffers(buffers: usize, stage: &str, result: &str) -> Ready {
    let mut source = String::new();
    for at in 0..buffers {
        source.push_str(&format!(
            "@group(0) @binding({at}) var<uniform> b{at}: vec4<f32>;
"
        ));
    }
    source.push_str(&format!(
        "{stage} fn main() -> {result} vec4<f32> {{
    var sum = vec4<f32>(0.0);
"
    ));
    for at in 0..buffers {
        source.push_str(&format!(
            "    sum += b{at};
"
        ));
    }
    source.push_str(
        "    return sum;
}",
    );
    alone(wgsl(&source), false).unwrap()
}

/// A stage that binds `buffers` constant buffers declared as a lowered program declares them,
/// rows of four dwords, and reads a row of each: `stage` and `result` are its attribute and its
/// result's, and with `input` it takes an input of a vertex.
fn lowered_buffers(buffers: usize, stage: &str, result: &str, input: bool) -> Ready {
    let mut source = String::new();
    for at in 0..buffers {
        source.push_str(&format!(
            "@group(0) @binding({at}) var<uniform> b{at}: array<vec4<u32>, 2>;\n"
        ));
    }
    let input = match input {
        true => "@location(0) at: vec4<f32>",
        false => "",
    };
    source.push_str(&format!(
        "{stage} fn main({input}) -> {result} vec4<f32> {{\n    var sum = 0u;\n"
    ));
    for at in 0..buffers {
        source.push_str(&format!("    sum += b{at}[1].x;\n"));
    }
    source.push_str("    return vec4<f32>(f32(sum));\n}");
    alone(wgsl(&source), false).unwrap()
}

#[test]
fn a_vertex_stage_is_held_to_the_sixteen_buffers_wgpu_s_metal_backend_gives_a_pipeline_s() {
    let vertex = |buffers, input| lowered_buffers(buffers, "@vertex", "@builtin(position)", input);
    let refused = |layout: u32, vertex_buffers: u32| {
        Outcome::Rejected(format!(
            "wgpu's Metal backend gives the vertex stage of a pipeline 16 buffers, its vertex buffers among them, and the vertex stage binds {layout} and is drawn with {vertex_buffers}"
        ))
    };
    // Fifteen buffers and the buffer of sizes are the sixteen; a pipeline with no vertex
    // buffer is held to them too.
    assert_eq!(
        metal_text(&vertex(15, false).stages, None),
        Outcome::Created
    );
    assert_eq!(metal_text(&vertex(16, false).stages, None), refused(17, 0));
    // A vertex buffer is one of the sixteen.
    assert_eq!(metal::vertex_buffers(&stage_of(&vertex(1, true))), 1);
    assert_eq!(metal::vertex_buffers(&stage_of(&vertex(1, false))), 0);
    assert_eq!(metal_text(&vertex(14, true).stages, None), Outcome::Created);
    assert_eq!(metal_text(&vertex(15, true).stages, None), refused(16, 1));
    // The layout's own count comes first, as the backend lays a pipeline out before it
    // creates it.
    assert_eq!(
        metal_text(&vertex(31, false).stages, None),
        Outcome::Rejected("Metal binds 31 buffers a stage, and the vertex stage binds 32".into())
    );
}

#[test]
fn a_pixel_stage_is_held_to_metal_s_thirty_one_buffers_and_binds_no_buffer_of_sizes_unasked() {
    let pixel = |buffers| lowered_buffers(buffers, "@fragment", "@location(0)", false);
    assert_eq!(metal_text(&pixel(31).stages, None), Outcome::Created);
    assert_eq!(
        metal_text(&pixel(32).stages, None),
        Outcome::Rejected("Metal binds 31 buffers a stage, and the pixel stage binds 32".into())
    );
}

#[test]
fn metal_s_buffers_are_counted_alike_by_the_layout_and_by_the_binder_s_budget() {
    use or_shader_front::rdna_bind::budget::Bound;
    // An array of buffers of dwords is one buffer of arguments, and asks for the buffer of
    // sizes as a buffer of dwords alone does.
    let arrayed = wgsl(
        "struct Cells { first: vec4<u32> }
@group(0) @binding(0) var<uniform> a: vec4<u32>;
@group(2) @binding(0) var<storage, read> tables: binding_array<Cells, 4>;
@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(f32(tables[a.x].first.x)); }",
    );
    let (resources, buffers) = metal_resources(&arrayed, wgpu::ShaderStages::FRAGMENT)
        .unwrap()
        .expect("the renderer's layouts take an array of buffers");
    assert_eq!((buffers, resources.sizes_buffer), (3, Some(2)));
    assert_eq!(Bound::of(&arrayed).together(), 3);
    for (ready, stage) in [
        (
            lowered_buffers(7, "@vertex", "@builtin(position)", false),
            wgpu::ShaderStages::VERTEX,
        ),
        (
            alone(wgsl(COUNTER), false).unwrap(),
            wgpu::ShaderStages::COMPUTE,
        ),
    ] {
        let module = stage_of(&ready);
        let (_, buffers) = metal_resources(&module, stage).unwrap().unwrap();
        assert_eq!(buffers, Bound::of(&module).together());
    }
}

#[test]
fn a_unit_past_metal_s_buffers_is_noted_with_its_count_and_written_once_its_buffers_are_merged() {
    let ready = lowered_buffers(20, "@vertex", "@builtin(position)", false);
    assert!(matches!(
        metal_text(&ready.stages, None),
        Outcome::Rejected(_)
    ));
    assert_eq!(
        past_notes(&ready.stages),
        ["past Metal's buffers, vertex stage: 21 buffers (20 constant buffers, 0 buffers of dwords, 0 bound arrays, the buffer of sizes)"]
    );
    let (stages, notes) = fitted(&ready.stages);
    assert_eq!(
        notes,
        ["fitted, vertex stage: 6 constant buffers merged into 1"]
    );
    assert_eq!(metal_text(&stages, None), Outcome::Created);
    assert!(past_notes(&stages).is_empty());
    // A unit within them is left as it is, and nothing is noted.
    let within = lowered_buffers(15, "@vertex", "@builtin(position)", false);
    assert!(past_notes(&within.stages).is_empty());
    let (stages, notes) = fitted(&within.stages);
    assert!(notes.is_empty());
    assert_eq!(
        format!("{:?}", stage_of(&within)),
        format!(
            "{:?}",
            match &stages {
                Stages::Render(vertex, _) => vertex,
                Stages::Compute(module) => module,
            }
        )
    );
    // Buffers that are no rows of dwords are not merged, and the unit says it is still past.
    let unmerged = many_buffers(20, "@vertex", "@builtin(position)");
    let (stages, notes) = fitted(&unmerged.stages);
    assert_eq!(
        notes,
        ["fitted, vertex stage: still past Metal's buffers, with 20 constant buffers of 16 to 16 bytes, 0 buffers of dwords, 0 bound arrays"]
    );
    assert!(matches!(metal_text(&stages, None), Outcome::Rejected(_)));
    assert_eq!(metal_fitted_adapter().shader_compiler, "text");
    assert_ne!(metal_fitted_adapter().name, metal_text_adapter().name);
}

/// The first stage of `ready`.
fn stage_of(ready: &Ready) -> naga::Module {
    match &ready.stages {
        Stages::Render(vertex, _) => vertex.clone(),
        Stages::Compute(module) => module.clone(),
    }
}

#[test]
fn a_program_s_kind_is_listed_by_its_stage_s_name() {
    use or_shader_front::dxil::ShaderKind;
    assert_eq!(kind_name(Some(ShaderKind::Pixel)), "pixel");
    assert_eq!(kind_name(Some(ShaderKind::Mesh)), "mesh");
    assert_eq!(kind_name(Some(ShaderKind::Other(9))), "other(9)");
    assert_eq!(kind_name(None), "unread");
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

#[test]
fn what_the_binder_did_to_a_fed_stage_is_noted_by_what_became_of_its_noughts() {
    use or_shader_front::rdna_bind::budget::Fitted;
    let done = Fitted {
        own: vec![(0, 1), (0, 2)],
        shared: vec![((0, 5), (0, 4))],
        merged: Vec::new(),
    };
    assert_eq!(
        metal::fed_notes("vertex", &done),
        [
            "as fed, vertex stage: 2 constant buffers left at nought are its own noughts",
            "as fed, vertex stage: 1 buffers left at nought read another's buffer"
        ]
    );
    assert!(metal::fed_notes("pixel", &Fitted::default()).is_empty());
}
