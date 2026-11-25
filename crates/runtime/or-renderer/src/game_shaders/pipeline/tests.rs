use super::*;

fn wgsl(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the WGSL parses")
}

fn entry(module: &naga::Module) -> &naga::EntryPoint {
    &module.entry_points[0]
}

#[test]
fn a_32_bit_scalar_or_vector_has_a_vertex_format_and_nothing_else_does() {
    use wgpu::VertexFormat as V;
    assert_eq!(vertex_format(ScalarKind::Float, 1), Some(V::Float32));
    assert_eq!(vertex_format(ScalarKind::Float, 4), Some(V::Float32x4));
    assert_eq!(vertex_format(ScalarKind::Uint, 3), Some(V::Uint32x3));
    assert_eq!(vertex_format(ScalarKind::Sint, 2), Some(V::Sint32x2));
    assert_eq!(vertex_format(ScalarKind::Float, 5), None);
    assert_eq!(vertex_format(ScalarKind::Bool, 1), None);
}

#[test]
fn a_vertex_stage_reads_one_buffer_with_its_inputs_packed_in_order() {
    let module = wgsl(
        "@vertex fn main(@location(0) p: vec3<f32>, @location(4) id: u32, @location(1) uv: vec2<f32>)
            -> @builtin(position) vec4<f32> { return vec4<f32>(p, f32(id) + uv.x); }",
    );
    let (attributes, stride) = vertex_layout(&module, entry(&module)).unwrap();
    let described: Vec<_> = attributes
        .iter()
        .map(|a| (a.shader_location, a.offset, a.format))
        .collect();
    assert_eq!(
        described,
        vec![
            (0, 0, wgpu::VertexFormat::Float32x3),
            (4, 12, wgpu::VertexFormat::Uint32),
            (1, 16, wgpu::VertexFormat::Float32x2),
        ]
    );
    assert_eq!(stride, 24);
}

#[test]
fn a_vertex_stage_with_no_inputs_still_has_a_stride() {
    let module =
        wgsl("@vertex fn main() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }");
    let (attributes, stride) = vertex_layout(&module, entry(&module)).unwrap();
    assert!(attributes.is_empty());
    assert_eq!(stride, 4);
}

#[test]
fn a_fragment_output_becomes_a_colour_target_sized_to_its_components_and_kind() {
    use wgpu::TextureFormat as F;
    let formats = |source: &str| {
        let module = wgsl(source);
        let (targets, depth) = fragment_targets(&module, entry(&module)).unwrap();
        (
            targets
                .iter()
                .map(|t| t.as_ref().map(|t| t.format))
                .collect::<Vec<_>>(),
            depth,
        )
    };
    assert_eq!(
        formats("@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }"),
        (vec![Some(F::Rgba8Unorm)], false)
    );
    assert_eq!(
        formats("@fragment fn main() -> @location(0) vec2<f32> { return vec2<f32>(1.0); }"),
        (vec![Some(F::Rg8Unorm)], false)
    );
    assert_eq!(
        formats("@fragment fn main() -> @location(0) f32 { return 1.0; }"),
        (vec![Some(F::R8Unorm)], false)
    );
    assert_eq!(
        formats("@fragment fn main() -> @location(0) vec4<u32> { return vec4<u32>(1u); }"),
        (vec![Some(F::Rgba8Uint)], false)
    );
    assert_eq!(
        formats("@fragment fn main() -> @location(0) vec3<i32> { return vec3<i32>(1); }"),
        (vec![Some(F::Rgba8Sint)], false)
    );
}

#[test]
fn a_single_float_goes_to_the_colour_format_of_the_target_where_that_has_one_channel() {
    use wgpu::TextureFormat as F;
    let module = wgsl("@fragment fn main() -> @location(0) f32 { return 1.0; }");
    let formats = |colour: F| {
        let target = Target {
            colour,
            ..Target::default()
        };
        let (targets, _) = fragment_targets_in(&module, entry(&module), &target).unwrap();
        targets[0].as_ref().map(|t| t.format)
    };
    assert_eq!(formats(F::R16Float), Some(F::R16Float));
    assert_eq!(formats(F::R32Float), Some(F::R32Float));
    // A colour of more channels is not one a single float is written in
    assert_eq!(formats(F::Rgba16Float), Some(F::R8Unorm));
}

#[test]
fn outputs_at_separate_locations_leave_the_slots_between_empty_and_depth_is_noted() {
    let module = wgsl(
        "struct Out { @location(2) a: vec4<f32>, @builtin(frag_depth) d: f32 }
         @fragment fn main() -> Out { var o: Out; return o; }",
    );
    let (targets, depth) = fragment_targets(&module, entry(&module)).unwrap();
    assert!(depth);
    assert_eq!(targets.len(), 3);
    assert!(targets[0].is_none() && targets[1].is_none() && targets[2].is_some());
}

#[test]
fn a_fragment_stage_that_writes_nothing_has_no_targets() {
    let module = wgsl("@fragment fn main() {}");
    let (targets, depth) = fragment_targets(&module, entry(&module)).unwrap();
    assert!(targets.is_empty() && !depth);
}

#[test]
fn the_entry_point_of_a_stage_is_found_by_stage_and_a_missing_one_is_named() {
    let module = wgsl("@fragment fn main() {}");
    assert!(entry_of(&module, ShaderStage::Fragment).is_ok());
    let missing = entry_of(&module, ShaderStage::Vertex).unwrap_err();
    assert_eq!(missing.to_string(), "no Vertex entry point");
}

#[test]
fn a_resource_both_stages_use_the_same_way_is_visible_to_both() {
    let vertex = wgsl(
        "@group(0) @binding(2) var<uniform> m: vec4<f32>;
         @group(0) @binding(3) var<uniform> only_here: vec4<f32>;
         @vertex fn main() -> @builtin(position) vec4<f32> { return m + only_here; }",
    );
    let fragment = wgsl(
        "@group(0) @binding(2) var<uniform> m: vec4<f32>;
         @fragment fn main() -> @location(0) vec4<f32> { return m; }",
    );
    let groups = merge_groups(&[
        (&vertex, wgpu::ShaderStages::VERTEX),
        (&fragment, wgpu::ShaderStages::FRAGMENT),
    ])
    .unwrap();
    let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
    assert_eq!(groups[&0][&2].1, both);
    assert_eq!(groups[&0][&3].1, wgpu::ShaderStages::VERTEX);
}

#[test]
fn two_stages_that_make_one_binding_two_resources_conflict_in_the_merge() {
    let vertex = wgsl(
        "@group(0) @binding(0) var<uniform> m: vec4<f32>;
         @vertex fn main() -> @builtin(position) vec4<f32> { return m; }",
    );
    let fragment = wgsl(
        "@group(0) @binding(0) var t: texture_2d<f32>;
         @fragment fn main() -> @location(0) vec4<f32> { return textureLoad(t, vec2<i32>(0), 0); }",
    );
    let error = merge_groups(&[
        (&vertex, wgpu::ShaderStages::VERTEX),
        (&fragment, wgpu::ShaderStages::FRAGMENT),
    ])
    .expect_err("a conflict");
    assert_eq!(
        error,
        PipelineError::Conflict {
            group: 0,
            binding: 0
        }
    );
}

#[test]
fn a_pass_of_a_pack_whose_vertex_stage_nests_loops_is_not_built_on_an_amd_card_under_direct3d_12() {
    use crate::instance::{made_up, AMD};
    let nested = wgsl(
        "@vertex fn main(@builtin(vertex_index) at: u32) -> @builtin(position) vec4<f32> {
            var sum = 0u;
            for (var i = 0u; i < at; i++) { for (var j = 0u; j < i; j++) { sum += j; } }
            return vec4<f32>(f32(sum));
        }",
    );
    let fragment = wgsl("@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }");
    let stages = [
        (&nested, wgpu::ShaderStages::VERTEX),
        (&fragment, wgpu::ShaderStages::FRAGMENT),
    ];
    let amd = made_up(AMD, 0x1681, wgpu::Backend::Dx12);
    let refused = groups_on(&amd, &stages).expect_err("the driver is not handed the pass");
    assert!(
        matches!(&refused, PipelineError::Driver(why) if why.contains("loop inside a loop")),
        "{refused}"
    );
    // The same card under Vulkan builds it, and a pixel stage's loops are not the matter.
    assert!(groups_on(&made_up(AMD, 0x1681, wgpu::Backend::Vulkan), &stages).is_ok());
    let turned = [
        (&fragment, wgpu::ShaderStages::VERTEX),
        (&nested, wgpu::ShaderStages::FRAGMENT),
    ];
    assert!(groups_on(&amd, &turned).is_ok());
}

#[test]
fn inputs_told_to_step_per_instance_are_read_from_a_second_buffer_packed_on_its_own() {
    use wgpu::VertexFormat as V;
    let module = wgsl(
        "@vertex fn main(@location(0) corner: vec2<f32>, @location(1) centre: vec3<f32>, @location(2) tint: vec4<f32>)
            -> @builtin(position) vec4<f32> { return vec4<f32>(corner + centre.xy, tint.x, 1.0); }",
    );
    let described = |attributes: &[wgpu::VertexAttribute]| {
        attributes
            .iter()
            .map(|a| (a.shader_location, a.offset, a.format))
            .collect::<Vec<_>>()
    };
    let streams = vertex_streams(&module, entry(&module), &[1, 2]).unwrap();
    assert_eq!(described(&streams.vertex), vec![(0, 0, V::Float32x2)]);
    assert_eq!(streams.vertex_stride, 8);
    assert_eq!(
        described(&streams.instance),
        vec![(1, 0, V::Float32x3), (2, 12, V::Float32x4)]
    );
    assert_eq!(streams.instance_stride, 28);

    // With none read per instance it is the one buffer `vertex_layout` describes
    let all = vertex_streams(&module, entry(&module), &[]).unwrap();
    assert!(all.instance.is_empty());
    assert_eq!(all.instance_stride, 0);
    let (attributes, stride) = vertex_layout(&module, entry(&module)).unwrap();
    assert_eq!((all.vertex, all.vertex_stride), (attributes, stride));
}

#[test]
fn a_pass_built_with_an_instance_stream_draws_each_instance_where_its_own_values_put_it() {
    use crate::game_shaders::frame::read_back;
    use crate::game_shaders::stand_in::draw_instances;
    use crate::game_shaders::test_pack::gpu;
    use std::collections::BTreeMap;

    let Some((device, queue)) = gpu("a_pass_built_with_an_instance_stream_draws_each_instance")
    else {
        return;
    };
    let vertex = wgsl(
        "struct Out { @builtin(position) position: vec4<f32>, @location(0) tint: vec4<f32> }
         @vertex fn main(@location(0) corner: vec2<f32>, @location(1) centre: vec2<f32>, @location(2) tint: vec4<f32>) -> Out {
             var out: Out;
             out.position = vec4<f32>(centre + corner * 0.25, 0.0, 1.0);
             out.tint = tint;
             return out;
         }",
    );
    let fragment = wgsl(
        "@fragment fn main(@location(0) tint: vec4<f32>) -> @location(0) vec4<f32> { return tint; }",
    );
    let pass = build_pass_pipeline_drawn(
        &device,
        &vertex,
        Some(&fragment),
        false,
        &Target::default(),
        &DrawState {
            instance_locations: vec![1, 2],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((pass.vertex_stride, pass.instance_stride), (8, 24));

    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let corners = bytes(&[
        -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
    ]);
    // A red one on the left and a green one on the right
    let instances = bytes(&[
        -0.5, 0.0, 1.0, 0.0, 0.0, 1.0, //
        0.5, 0.0, 0.0, 1.0, 0.0, 1.0,
    ]);
    let size = 16u32;
    let target = draw_instances(
        &device,
        &queue,
        &pass,
        &BTreeMap::new(),
        &corners,
        &instances,
        6,
        size,
    )
    .into_iter()
    .next()
    .flatten()
    .expect("a colour target");
    let rendered = read_back(
        &device,
        &queue,
        &target,
        wgpu::TextureFormat::Rgba8Unorm,
        size,
    );
    let at = |x: u32, y: u32| {
        let i = ((y * size + x) * 4) as usize;
        [
            rendered.bytes[i],
            rendered.bytes[i + 1],
            rendered.bytes[i + 2],
            rendered.bytes[i + 3],
        ]
    };
    assert_eq!(at(4, 8), [255, 0, 0, 255], "the left instance is red");
    assert_eq!(at(12, 8), [0, 255, 0, 255], "the right instance is green");
    assert_eq!(at(8, 8), [0, 0, 0, 0], "nothing is drawn between them");
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn a_pass_that_culls_back_faces_draws_a_counter_clockwise_triangle_and_not_its_mirror() {
    use crate::game_shaders::stand_in::draw_instances;
    use crate::game_shaders::test_pack::gpu;
    use std::collections::BTreeMap;

    let Some((device, queue)) = gpu("a_pass_that_culls_back_faces") else {
        return;
    };
    let vertex = wgsl(
        "@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p, 0.0, 1.0); }",
    );
    let fragment = wgsl("@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }");
    let counter_clockwise = floats(&[-0.5, -0.5, 0.5, -0.5, 0.0, 0.5]);
    let clockwise = floats(&[-0.5, -0.5, 0.0, 0.5, 0.5, -0.5]);
    let touched = |cull: Option<wgpu::Face>, triangle: &[u8]| {
        let pass = build_pass_pipeline_drawn(
            &device,
            &vertex,
            Some(&fragment),
            false,
            &Target::default(),
            &DrawState {
                cull_mode: cull,
                ..Default::default()
            },
        )
        .unwrap();
        let target = draw_instances(
            &device,
            &queue,
            &pass,
            &BTreeMap::new(),
            triangle,
            &[],
            3,
            16,
        )
        .into_iter()
        .next()
        .flatten()
        .expect("a colour target");
        crate::game_shaders::frame::read_back(
            &device,
            &queue,
            &target,
            wgpu::TextureFormat::Rgba8Unorm,
            16,
        )
        .touched()
    };
    assert!(touched(None, &counter_clockwise) > 0);
    assert!(
        touched(None, &clockwise) > 0,
        "nothing is culled by default"
    );
    assert!(touched(Some(wgpu::Face::Back), &counter_clockwise) > 0);
    assert_eq!(touched(Some(wgpu::Face::Back), &clockwise), 0);
}

#[test]
fn a_blended_pass_writes_depth_only_where_it_is_told_to() {
    use crate::game_shaders::frame::read_back;
    use crate::game_shaders::stand_in::draw_instances;
    use crate::game_shaders::test_pack::gpu;
    use std::collections::BTreeMap;

    let Some((device, queue)) = gpu("a_blended_pass_writes_depth_only_where_it_is_told_to") else {
        return;
    };
    let vertex = wgsl(
        "struct Out { @builtin(position) position: vec4<f32>, @location(0) tint: vec4<f32> }
         @vertex fn main(@location(0) corner: vec2<f32>, @location(1) centre: vec3<f32>, @location(2) tint: vec4<f32>) -> Out {
             var out: Out;
             out.position = vec4<f32>(centre.xy + corner * 0.5, centre.z, 1.0);
             out.tint = tint;
             return out;
         }",
    );
    let fragment = wgsl(
        "@fragment fn main(@location(0) tint: vec4<f32>) -> @location(0) vec4<f32> { return tint; }",
    );
    let corners = floats(&[
        -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
    ]);
    // A near red quad and then a far green one, both over the middle
    let instances = floats(&[
        0.0, 0.0, 0.2, 1.0, 0.0, 0.0, 1.0, //
        0.0, 0.0, 0.8, 0.0, 1.0, 0.0, 1.0,
    ]);
    let middle = |depth_write: Option<bool>| {
        let target = Target {
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            ..Default::default()
        };
        let pass = build_pass_pipeline_drawn(
            &device,
            &vertex,
            Some(&fragment),
            true,
            &target,
            &DrawState {
                instance_locations: vec![1, 2],
                depth_write,
                ..Default::default()
            },
        )
        .unwrap();
        let drawn = draw_instances(
            &device,
            &queue,
            &pass,
            &BTreeMap::new(),
            &corners,
            &instances,
            6,
            16,
        )
        .into_iter()
        .next()
        .flatten()
        .expect("a colour target");
        let rendered = read_back(&device, &queue, &drawn, wgpu::TextureFormat::Rgba8Unorm, 16);
        let i = ((8 * 16 + 8) * 4) as usize;
        [
            rendered.bytes[i],
            rendered.bytes[i + 1],
            rendered.bytes[i + 2],
            rendered.bytes[i + 3],
        ]
    };
    assert_eq!(
        middle(None),
        [0, 255, 0, 255],
        "a blended pass tests depth and writes none, so the far quad is drawn over"
    );
    assert_eq!(
        middle(Some(true)),
        [255, 0, 0, 255],
        "told to write, the near quad keeps the far one out"
    );
}

#[test]
fn a_pass_told_it_has_a_depth_attachment_has_one_it_does_not_test() {
    use crate::game_shaders::test_pack::gpu;

    let Some((device, _queue)) = gpu("a_pass_told_it_has_a_depth_attachment") else {
        return;
    };
    let vertex = wgsl(
        "@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p, 0.0, 1.0); }",
    );
    let fragment = wgsl("@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }");
    let build = |depth_attachment: bool| {
        build_pass_pipeline_drawn(
            &device,
            &vertex,
            Some(&fragment),
            false,
            &Target::default(),
            &DrawState {
                depth_attachment,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let without = build(false);
    assert!(!without.depth && !without.depth_test);
    let with = build(true);
    assert!(with.depth && !with.depth_test);
}

#[test]
fn a_strip_makes_a_triangle_of_each_vertex_after_the_second_and_a_list_does_not() {
    use crate::game_shaders::stand_in::draw_instances;
    use crate::game_shaders::test_pack::gpu;
    use std::collections::BTreeMap;

    let Some((device, queue)) = gpu("a_strip_makes_a_triangle_of_each_vertex") else {
        return;
    };
    let vertex = wgsl(
        "@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p, 0.0, 1.0); }",
    );
    let fragment = wgsl("@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }");
    // The corners of a square in the order a strip wants them
    let square = floats(&[-0.5, -0.5, 0.5, -0.5, -0.5, 0.5, 0.5, 0.5]);
    let touched = |topology: wgpu::PrimitiveTopology| {
        let pass = build_pass_pipeline_drawn(
            &device,
            &vertex,
            Some(&fragment),
            false,
            &Target::default(),
            &DrawState {
                topology,
                ..Default::default()
            },
        )
        .unwrap();
        let target = draw_instances(
            &device,
            &queue,
            &pass,
            &BTreeMap::new(),
            &square,
            &[],
            4,
            16,
        )
        .into_iter()
        .next()
        .flatten()
        .expect("a colour target");
        crate::game_shaders::frame::read_back(
            &device,
            &queue,
            &target,
            wgpu::TextureFormat::Rgba8Unorm,
            16,
        )
        .touched()
    };
    // Columns and rows 4 to 11 have their centres inside the square
    assert_eq!(touched(wgpu::PrimitiveTopology::TriangleStrip), 8 * 8);
    let list = touched(wgpu::PrimitiveTopology::TriangleList);
    assert!(
        list > 0 && list < 8 * 8,
        "a list of four vertices is one triangle: {list}"
    );
}
