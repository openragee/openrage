//! The translucency kind of GTA VI's tiled lighting, as the resolve ports it: the arithmetic of
//! its program on made-up inputs, and a leaf lit through from behind where an opaque surface is not.

use super::scene_tests::frames_stated;
use super::tests::{surface, SIZE};
use super::*;
use crate::game_shaders::frame::read_back;

/// The resolve's functions of the material kind and the light through a leaf, cut from it.
pub(super) fn leaf_functions() -> &'static str {
    let from = RESOLVE
        .find("// The kind GTA VI's tiled lighting picks")
        .expect("the resolve reads the kind");
    let to = RESOLVE
        .find("// The surface the targets hold at")
        .expect("the resolve reads a surface");
    &RESOLVE[from..to]
}

/// What the pixel `x` of a row of cases computes: the kind of the bytes `y` and `y + 16`, then
/// three leaves, then the table's two shares at four cosines.
const CASES: &str = r#"
const PI: f32 = 3.14159265;
// The matte falloff's two shares of a leaf the light stands behind, seen at a cosine of 0.8.
const SHAPE: f32 = 0.49992;

@vertex fn corner(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let at = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return vec4<f32>(at, 0.0, 1.0);
}

fn leaf(amount: f32, v_l: f32) -> vec4<f32> {
    let through = through_of(amount);
    return vec4<f32>(through.profile, through.matte, passed(through, v_l, SHAPE), through.scale);
}

@fragment fn worked(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let x = u32(at.x);
    let y = f32(u32(at.y));
    switch x {
        case 0u: { return vec4<f32>(f32(kind_of(y / 255.0)), f32(kind_of((y + 16.0) / 255.0)), 0.0, 0.0); }
        case 1u: { return leaf(0.75, -0.9); }
        case 2u: { return leaf(0.75, 0.6); }
        case 3u: { return leaf(0.0, -0.9); }
        case 4u: { return vec4<f32>(falloff(0.7), falloff(-0.7)); }
        case 5u: { return vec4<f32>(falloff(0.05), falloff(0.0)); }
        default: { return vec4<f32>(0.0); }
    }
}
"#;

/// The row `y` of what [`CASES`] computes, four numbers a pixel; none with no device.
fn cases(test: &str) -> Option<Vec<[f32; 4]>> {
    computed(test, &format!("{CASES}{}", leaf_functions()))
}

/// What the fragment entry `worked` of `source` writes over a target of [`SIZE`] a side, four
/// numbers a pixel, its entry `corner` drawing a triangle over it; none with no device.
pub(super) fn computed(test: &str, source: &str) -> Option<Vec<[f32; 4]>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match crate::instance::headless_adapter(&instance) {
        Ok(adapter) => adapter,
        Err(e) => {
            or_test_support::skip_device(test, format_args!("no wgpu adapter: {e}"));
            return None;
        }
    };
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("translucency-cases"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let format = wgpu::TextureFormat::Rgba32Float;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("translucency-cases"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("corner"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("worked"),
            compilation_options: Default::default(),
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let target = render_target(
        &device,
        "translucency-cases",
        (SIZE, SIZE),
        format,
        wgpu::TextureUsages::COPY_SRC,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    let bytes = read_back(&device, &queue, target.texture(), format, SIZE).bytes;
    let numbers: &[f32] = bytemuck::cast_slice(&bytes);
    Some(
        numbers
            .chunks(4)
            .map(|n| [n[0], n[1], n[2], n[3]])
            .collect(),
    )
}

fn near(seen: f32, expected: f64, within: f64) -> bool {
    (f64::from(seen) - expected).abs() <= within
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn the_kind_is_the_low_five_bits_of_the_colour_s_fourth_channel_as_a_byte() {
    let Some(seen) = cases("the_kind_is_the_low_five_bits") else {
        return;
    };
    for y in 0..SIZE as usize {
        let [low, high, ..] = seen[y * SIZE as usize];
        assert_eq!((low, high), (y as f32, y as f32 + 16.0), "byte {y}");
    }
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_leaf_s_light_through_is_the_translucency_program_s_on_three_made_up_inputs() {
    let Some(seen) = cases("a_leaf_s_light_through") else {
        return;
    };
    // Worked by hand from the program's constants: an amount of 0.75 gives a profile of
    // 0.96739, the matte taken by 0.4, a lobe's asymmetry of 0.45 and a scale of 0.18039.
    let [profile, matte, back, scale] = seen[1];
    assert!(near(profile, 0.967394, 1e-4), "{profile}");
    assert!(near(matte, 0.4, 1e-6) && near(scale, 0.180389, 1e-5));
    // Seen against the light behind the leaf, the lobe is strong; seen with the light at the
    // viewer's back, weak.
    assert!(near(back, 0.472831, 1e-4), "{back}");
    let front = seen[2][2];
    assert!(near(front, 0.211627, 1e-4), "{front}");
    // Nothing passes: no profile, nothing through, and the matte untouched, so the leaf is lit
    // as the standard material is.
    assert_eq!(seen[3], [0.0, 1.0, 0.0, 0.0]);
    // The table: the first share is the cosine of a light in front, the second that of one
    // behind, both soft about the terminator.
    let [front_lit, front_behind, back_lit, back_behind] = seen[4];
    assert!(near(front_lit, 0.7, 1e-4) && near(front_behind, 0.0, 1e-6));
    assert!(near(back_lit, 0.0, 1e-6) && near(back_behind, 0.7, 1e-4));
    let [grazing, grazing_behind, edge, edge_behind] = seen[5];
    assert!(near(grazing, 0.05, 1e-4) && near(grazing_behind, 0.024965, 1e-4));
    assert!(near(edge, 0.002122, 1e-4) && near(edge_behind, 0.046371, 1e-4));
}

/// The test model's pixel stage, its colour target's fourth channel the kind `kind`, and all
/// of the light passing where the leaf takes any.
fn kind_stage(kind: u32) -> naga::Module {
    let text = format!(
        "
@group(1) @binding(0) var<uniform> tint: vec4<f32>;
@group(1) @binding(1) var diffuse: texture_2d<f32>;
@group(1) @binding(2) var diffuse_sampler: sampler;

struct Targets {{
    @location(0) colour: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) terms: vec4<f32>,
}}

@fragment fn main(@location(0) shade: vec3<f32>) -> Targets {{
    let texel = textureSample(diffuse, diffuse_sampler, vec2<f32>(0.5));
    let packed = 2047u | (2047u << 12u);
    let bytes = vec3<f32>(f32(packed & 255u), f32((packed >> 8u) & 255u), f32(packed >> 16u));
    return Targets(
        vec4<f32>(texel.rgb * tint.rgb * shade, {kind}.0 / 255.0),
        vec4<f32>(0.5, bytes / 255.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
        vec4<f32>(0.0),
    );
}}"
    );
    naga::front::wgsl::parse_str(&text).expect("the WGSL parses")
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn a_leaf_is_lit_through_by_a_sun_behind_it_and_an_opaque_surface_is_not() {
    let filmic = Filmic([0.22, 0.3, 0.1, 0.2, 0.01, 0.3, 11.2]);
    let curve = Curve {
        scale: -std::f32::consts::LOG2_E,
        vertical: 10.0,
        horizontal: 0.0,
        bending: 0.0,
    };
    // The model faces the camera, up; the sun stands under it, and nothing else gives light.
    let behind = Sky {
        towards: Vec3::new(0.6, 0.0, -0.8),
        sun: Vec3::splat(std::f32::consts::PI),
        sky: Vec3::ZERO,
        ground: Vec3::ZERO,
        exposure: 1.0,
        tone: Some(Tone {
            roughness_mod: 0.0,
            disc: 0.0,
            lowlights: 0.0,
            highlights: 0.0,
            curve: [curve; 2],
            blend: 0.0,
            key: None,
            least: -2.0,
            most: 2.0,
            added: 0.0,
            bright: filmic,
            dark: filmic,
            bright_at: 1.0,
            dark_at: 3.0,
            whole_frame: false,
        }),
        scene: Scene::default(),
    };
    let middle = |frame: &[u8]| {
        let at = ((SIZE / 2 + 2) * SIZE + SIZE / 2) as usize * 4;
        [frame[at], frame[at + 1], frame[at + 2]]
    };
    let mut seen = Vec::new();
    for kind in [0, 5] {
        let mut model = surface();
        model.fragment = std::sync::Arc::new(kind_stage(kind));
        let Some(frames) = frames_stated(
            "a_leaf_is_lit_through_by_a_sun_behind_it",
            model,
            &[(Mat4::IDENTITY, behind)],
            |_, _, _| {},
        ) else {
            return;
        };
        seen.push(middle(&frames[0]));
    }
    assert_eq!(
        seen[0],
        [0, 0, 0],
        "an opaque surface the sun stands behind"
    );
    assert!(
        seen[1][0] > 40,
        "a leaf the sun stands behind: {:?}",
        seen[1]
    );
}
