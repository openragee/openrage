use super::*;
use crate::game_shaders::pipeline::{build_pass_pipeline, PipelineError};

fn wgsl(source: &str) -> naga::Module {
    naga::front::wgsl::parse_str(source).expect("the WGSL parses")
}

/// The machine's device and queue, or a recorded skip.
fn gpu(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("game-shaders-test"),
        required_limits: adapter.limits(),
        ..Default::default()
    })) {
        Ok(pair) => Some(pair),
        Err(e) => {
            or_test_support::skip_device(test, format_args!("device init failed: {e}"));
            None
        }
    }
}

/// What `body` made the device refuse, if anything.
fn refused<T>(device: &wgpu::Device, body: impl FnOnce() -> T) -> (T, Option<String>) {
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let made = body();
    let validation = pollster::block_on(validation.pop());
    let internal = pollster::block_on(internal.pop());
    (made, validation.or(internal).map(|e| e.to_string()))
}

const FULL_SCREEN_TRIANGLE: &str = "
@vertex fn main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i) / 2) * 4.0 - 1.0;
    let y = f32(i32(i) & 1) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}";

/// The first colour target of a draw as the bytes of its pixels.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let row = 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * TARGET_SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(TARGET_SIZE),
            },
        },
        wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("the buffer maps")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device finishes");
    let view = slice.get_mapped_range();
    let data = view.to_vec();
    drop(view);
    data
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_resource_bound_through_the_layout_the_module_asked_for_reaches_the_shader() {
    let Some((device, queue)) = gpu("a_resource_bound_through_the_layout_the_module_asked_for")
    else {
        return;
    };
    let fragment = wgsl(
        "@group(1) @binding(3) var<uniform> colour: vec4<f32>;
         @fragment fn main() -> @location(0) vec4<f32> { return colour; }",
    );
    let vertex = wgsl(FULL_SCREEN_TRIANGLE);
    let (pass, error) = refused(&device, || {
        build_pass_pipeline(&device, &vertex, Some(&fragment))
    });
    assert_eq!(error, None);
    let pass = pass.expect("the pass builds");
    assert_eq!(pass.layouts.keys().copied().collect::<Vec<_>>(), vec![1]);
    let colour = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[0.25f32, 0.5, 0.75, 1.0]),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pass.layouts[&1],
        entries: &[wgpu::BindGroupEntry {
            binding: 3,
            resource: colour.as_entire_binding(),
        }],
    });
    let (targets, error) = refused(&device, || {
        draw_triangle(&device, &queue, &pass, &BTreeMap::from([(1, group)]))
    });
    assert_eq!(error, None);
    let pixels = read_back(&device, &queue, targets[0].as_ref().unwrap());
    let first = &pixels[..4];
    for (got, want) in first.iter().zip([64u8, 128, 191, 255]) {
        assert!(got.abs_diff(want) <= 1, "{first:?}");
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_stand_in_for_each_kind_of_resource_a_pass_asks_for_draws_without_a_validation_error() {
    let Some((device, queue)) = gpu("a_stand_in_for_each_kind_of_resource") else {
        return;
    };
    let vertex = wgsl(
        "@group(0) @binding(5) var<uniform> matrices: array<vec4<f32>, 4>;
         @vertex fn main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
             return matrices[i % 4u];
         }",
    );
    let fragment = wgsl(
        "@group(1) @binding(0) var<uniform> block: array<vec4<f32>, 3>;
         @group(1) @binding(32) var plain: texture_2d<f32>;
         @group(1) @binding(33) var cube: texture_cube<f32>;
         @group(1) @binding(34) var volume: texture_3d<u32>;
         @group(1) @binding(35) var layers: texture_2d_array<f32>;
         @group(1) @binding(36) var shadow: texture_depth_2d;
         @group(1) @binding(37) var many: texture_multisampled_2d<f32>;
         @group(1) @binding(16) var linear: sampler;
         @group(1) @binding(17) var compare: sampler_comparison;
         @group(1) @binding(50) var<storage, read> table: array<vec4<f32>, 8>;
         @fragment fn main() -> @location(0) vec4<f32> {
             let a = textureSample(plain, linear, vec2<f32>(0.5)) + textureSample(cube, linear, vec3<f32>(0.5))
                 + textureSample(layers, linear, vec2<f32>(0.5), 1);
             let b = vec4<f32>(textureLoad(volume, vec3<i32>(0), 0))
                 + textureLoad(many, vec2<i32>(0), 0)
                 + vec4<f32>(textureSampleCompare(shadow, compare, vec2<f32>(0.5), 0.5));
             return a + b + block[1] + table[2];
         }",
    );
    let (pass, error) = refused(&device, || {
        build_pass_pipeline(&device, &vertex, Some(&fragment))
    });
    assert_eq!(error, None);
    let pass = pass.expect("the pass builds");
    let ((groups, targets), error) = refused(&device, || {
        let groups = bind_groups(&device, &queue, &pass);
        let targets = draw_triangle(&device, &queue, &pass, &groups);
        (groups, targets)
    });
    assert_eq!(error, None);
    assert_eq!(groups.keys().copied().collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(targets.len(), 1);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_depth_stand_in_is_far_so_a_comparison_against_it_finds_nothing_in_the_way() {
    let Some((device, queue)) = gpu("a_depth_stand_in_is_far_so_a_comparison") else {
        return;
    };
    let vertex = wgsl(FULL_SCREEN_TRIANGLE);
    let fragment = wgsl(
        "@group(1) @binding(36) var shadow: texture_depth_2d;
         @group(1) @binding(17) var compare: sampler_comparison;
         @fragment fn main() -> @location(0) vec4<f32> {
             let lit = textureSampleCompare(shadow, compare, vec2<f32>(0.5), 0.5);
             return vec4<f32>(lit, lit, lit, 1.0);
         }",
    );
    let (pass, error) = refused(&device, || {
        build_pass_pipeline(&device, &vertex, Some(&fragment))
    });
    assert_eq!(error, None);
    let pass = pass.expect("the pass builds");
    let groups = bind_groups(&device, &queue, &pass);
    let (targets, error) = refused(&device, || draw_triangle(&device, &queue, &pass, &groups));
    assert_eq!(error, None);
    let pixels = read_back(&device, &queue, targets[0].as_ref().unwrap());
    assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn groups_with_a_gap_between_them_and_a_pass_with_no_pixel_stage_both_build() {
    let Some((device, queue)) = gpu("groups_with_a_gap_between_them") else {
        return;
    };
    let vertex = wgsl(
        "@group(0) @binding(0) var<uniform> m: vec4<f32>;
         @vertex fn main() -> @builtin(position) vec4<f32> { return m; }",
    );
    let fragment = wgsl(
        "@group(3) @binding(1) var<uniform> c: vec4<f32>;
         @fragment fn main() -> @location(0) vec4<f32> { return c; }",
    );
    let (pass, error) = refused(&device, || {
        build_pass_pipeline(&device, &vertex, Some(&fragment))
    });
    assert_eq!(error, None);
    let pass = pass.unwrap();
    assert_eq!(pass.layouts.keys().copied().collect::<Vec<_>>(), vec![0, 3]);
    let groups = bind_groups(&device, &queue, &pass);
    let (_, error) = refused(&device, || draw_triangle(&device, &queue, &pass, &groups));
    assert_eq!(error, None);

    // No pixel stage: no colour, and so the depth target a pipeline has to have
    let (alone, error) = refused(&device, || build_pass_pipeline(&device, &vertex, None));
    assert_eq!(error, None);
    let alone = alone.unwrap();
    assert!(alone.colour_formats.is_empty() && alone.depth);
    let groups = bind_groups(&device, &queue, &alone);
    let (_, error) = refused(&device, || draw_triangle(&device, &queue, &alone, &groups));
    assert_eq!(error, None);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn one_binding_that_is_two_resources_in_the_two_stages_is_a_conflict() {
    let Some((device, _queue)) = gpu("one_binding_that_is_two_resources") else {
        return;
    };
    let vertex = wgsl(
        "@group(0) @binding(0) var<uniform> m: vec4<f32>;
         @vertex fn main() -> @builtin(position) vec4<f32> { return m; }",
    );
    let fragment = wgsl(
        "@group(0) @binding(0) var t: texture_2d<f32>;
         @fragment fn main() -> @location(0) vec4<f32> { return textureLoad(t, vec2<i32>(0), 0); }",
    );
    let Err(error) = build_pass_pipeline(&device, &vertex, Some(&fragment)) else {
        panic!("a uniform and a texture shared a binding");
    };
    assert_eq!(
        error,
        PipelineError::Conflict {
            group: 0,
            binding: 0
        }
    );
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_stand_in_buffer_is_at_least_sixteen_bytes_and_a_multiple_of_sixteen() {
    let Some((device, _queue)) = gpu("a_stand_in_buffer_is_at_least_sixteen_bytes") else {
        return;
    };
    let sizes: Vec<u64> = [0u64, 3, 16, 17, 112]
        .iter()
        .map(|size| buffer(&device, *size, wgpu::BufferUsages::UNIFORM).size())
        .collect();
    assert_eq!(sizes, vec![16, 16, 16, 32, 112]);
}

/// Two triangles that cover the target, each vertex a position and a texture coordinate.
fn full_screen_quad() -> Vec<u8> {
    let corners: [[f32; 4]; 6] = [
        [-1.0, -1.0, 0.0, 1.0],
        [1.0, -1.0, 1.0, 1.0],
        [1.0, 1.0, 1.0, 0.0],
        [-1.0, -1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0, 0.0],
    ];
    bytemuck::cast_slice(&corners).to_vec()
}

const TEXTURED_PAIR: (&str, &str) = (
    "struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
     @vertex fn main(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>) -> Out {
         var out: Out;
         out.position = vec4<f32>(position, 0.0, 1.0);
         out.uv = uv;
         return out;
     }",
    "@group(1) @binding(0) var<uniform> tint: vec4<f32>;
     @group(1) @binding(1) var tex: texture_2d<f32>;
     @group(1) @binding(2) var samp: sampler;
     @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
         return textureSample(tex, samp, uv) * tint;
     }",
);

fn orange_texels() -> Provided {
    Provided::Texture2d {
        width: 2,
        height: 2,
        rgba: [255, 128, 0, 255].repeat(4),
        srgb: false,
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn provided_bytes_and_texels_reach_the_shader_and_the_quad_is_drawn_from_real_vertices() {
    let Some((device, queue)) = gpu("provided_bytes_and_texels_reach_the_shader") else {
        return;
    };
    let vertex = wgsl(TEXTURED_PAIR.0);
    let fragment = wgsl(TEXTURED_PAIR.1);
    let pass = build_pass_pipeline(&device, &vertex, Some(&fragment)).expect("the pass builds");
    assert_eq!(pass.vertex_stride, 16);

    let mut asked = Vec::new();
    let tint: Vec<u8> = bytemuck::cast_slice(&[0.5f32, 1.0, 1.0, 1.0]).to_vec();
    let (groups, error) = refused(&device, || {
        bind_groups_with(&device, &queue, &pass, &mut |requirement| {
            asked.push((requirement.group, requirement.binding));
            match requirement.resource {
                Resource::Uniform { .. } => Some(Provided::Bytes(tint.clone())),
                Resource::Texture { .. } => Some(orange_texels()),
                _ => None,
            }
        })
    });
    assert_eq!(error, None);
    assert_eq!(
        asked,
        vec![(1, 0), (1, 1), (1, 2)],
        "each resource is asked for once, in order"
    );

    let (targets, error) = refused(&device, || {
        draw_vertices(
            &device,
            &queue,
            &pass,
            &groups,
            &full_screen_quad(),
            6,
            TARGET_SIZE,
        )
    });
    assert_eq!(error, None);
    let pixels = read_back(&device, &queue, targets[0].as_ref().unwrap());
    // The corner only each triangle covers: texel (255, 128, 0, 255) scaled by the tint (0.5, 1, 1, 1)
    let last = (TARGET_SIZE - 1) as usize;
    for (x, y) in [(0, 0), (last, last)] {
        let at = y * 256 + x * 4;
        let pixel = &pixels[at..at + 4];
        for (got, want) in pixel.iter().zip([128u8, 128, 0, 255]) {
            assert!(got.abs_diff(want) <= 2, "pixel ({x}, {y}): {pixel:?}");
        }
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn contents_that_do_not_suit_a_resource_are_ignored_and_a_long_buffer_is_cut_to_the_buffer() {
    let Some((device, queue)) = gpu("contents_that_do_not_suit_a_resource") else {
        return;
    };
    let vertex = wgsl(TEXTURED_PAIR.0);
    let fragment = wgsl(TEXTURED_PAIR.1);
    let pass = build_pass_pipeline(&device, &vertex, Some(&fragment)).unwrap();
    // Texels for the buffer, bytes for the texture, and far too many bytes besides
    let (groups, error) = refused(&device, || {
        bind_groups_with(
            &device,
            &queue,
            &pass,
            &mut |requirement| match requirement.resource {
                Resource::Uniform { .. } => Some(orange_texels()),
                Resource::Texture { .. } => Some(Provided::Bytes(vec![7; 4096])),
                _ => None,
            },
        )
    });
    assert_eq!(error, None);
    let (_, error) = refused(&device, || {
        draw_vertices(
            &device,
            &queue,
            &pass,
            &groups,
            &full_screen_quad(),
            6,
            TARGET_SIZE,
        )
    });
    assert_eq!(error, None);
    let (groups, error) = refused(&device, || {
        bind_groups_with(&device, &queue, &pass, &mut |requirement| {
            matches!(requirement.resource, Resource::Uniform { .. })
                .then(|| Provided::Bytes(vec![1; 4096 + 3]))
        })
    });
    assert_eq!(error, None);
    assert_eq!(groups.len(), 1);
    // Seven bytes are four, a whole word, and a write of seven would be refused
    let (_, error) = refused(&device, || {
        bind_groups_with(&device, &queue, &pass, &mut |requirement| {
            matches!(requirement.resource, Resource::Uniform { .. })
                .then(|| Provided::Bytes(vec![1; 7]))
        })
    });
    assert_eq!(error, None);
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_texture_in_the_srgb_colour_space_is_read_as_linear_and_one_that_is_not_is_read_as_it_is() {
    let Some((device, queue)) = gpu("a_texture_in_the_srgb_colour_space") else {
        return;
    };
    let vertex = wgsl(TEXTURED_PAIR.0);
    let fragment = wgsl(TEXTURED_PAIR.1);
    let pass = build_pass_pipeline(&device, &vertex, Some(&fragment)).expect("the pass builds");
    let tint: Vec<u8> = bytemuck::cast_slice(&[1.0f32; 4]).to_vec();
    let grey = |srgb| Provided::Texture2d {
        width: 2,
        height: 2,
        rgba: [188, 188, 188, 255].repeat(4),
        srgb,
    };
    let mut corner = Vec::new();
    for srgb in [false, true] {
        let groups = bind_groups_with(&device, &queue, &pass, &mut |requirement| match requirement
            .resource
        {
            Resource::Uniform { .. } => Some(Provided::Bytes(tint.clone())),
            Resource::Texture { .. } => Some(grey(srgb)),
            _ => None,
        });
        let targets = draw_vertices(
            &device,
            &queue,
            &pass,
            &groups,
            &full_screen_quad(),
            6,
            TARGET_SIZE,
        );
        let pixels = read_back(&device, &queue, targets[0].as_ref().unwrap());
        // The corner the first triangle covers and the second does not
        let last = TARGET_SIZE as usize - 1;
        let at = last * 256 + last * 4;
        corner.push(pixels[at]);
    }
    // 188 is what sRGB encodes a linear 0.5 to, which a target of eight bits holds as 128
    assert_eq!(corner[0], 188);
    assert!(corner[1].abs_diff(128) <= 2, "{corner:?}");
}

#[test]
fn the_samplers_repeat_filter_between_levels_and_keep_detail_along_a_slant_but_a_comparison_does_not(
) {
    let colour = sampler_descriptor(false);
    for mode in [
        colour.address_mode_u,
        colour.address_mode_v,
        colour.address_mode_w,
    ] {
        assert_eq!(mode, wgpu::AddressMode::Repeat);
    }
    assert_eq!(colour.mipmap_filter, wgpu::MipmapFilterMode::Linear);
    assert_eq!(colour.anisotropy_clamp, 8);
    assert_eq!(colour.compare, None);
    let depth = sampler_descriptor(true);
    assert_eq!(depth.anisotropy_clamp, 1);
    assert_eq!(depth.compare, Some(wgpu::CompareFunction::Less));
}

#[test]
fn a_sampler_is_what_the_effect_declares_for_its_texture_and_the_default_where_it_says_nothing() {
    use wgpu::{AddressMode as A, FilterMode as F, MipmapFilterMode as M};
    // A palette: repeat, point on every side (the pairs of the ped effect's `TextureSamplerDiffPal`)
    let palette = declared_sampler(&[0, 1, 1, 1, 6, 1, 5, 1, 4, 1]);
    assert_eq!(
        (palette.address_mode_u, palette.address_mode_v),
        (A::Repeat, A::Repeat)
    );
    assert_eq!(
        (
            palette.mag_filter,
            palette.min_filter,
            palette.mipmap_filter
        ),
        (F::Nearest, F::Nearest, M::Nearest)
    );
    assert_eq!(palette.anisotropy_clamp, 1);
    // A reflection map: clamped, linear between levels and texels
    let reflection = declared_sampler(&[0, 3, 1, 3, 2, 3, 5, 3, 4, 2, 6, 2]);
    assert_eq!(
        (
            reflection.address_mode_u,
            reflection.address_mode_v,
            reflection.address_mode_w
        ),
        (A::ClampToEdge, A::ClampToEdge, A::ClampToEdge)
    );
    assert_eq!(
        (
            reflection.mag_filter,
            reflection.min_filter,
            reflection.mipmap_filter
        ),
        (F::Linear, F::Linear, M::Linear)
    );
    // A diffuse texture: it states no address, and keeps detail four times along a slant
    let diffuse = declared_sampler(&[5, 3, 4, 3, 9, 4]);
    assert_eq!(diffuse.address_mode_u, A::Repeat);
    assert_eq!(diffuse.anisotropy_clamp, 4);
    // A mirror, and no state at all
    assert_eq!(declared_sampler(&[0, 2]).address_mode_u, A::MirrorRepeat);
    let none = declared_sampler(&[]);
    assert_eq!(
        none.address_mode_u,
        sampler_descriptor(false).address_mode_u
    );
}
