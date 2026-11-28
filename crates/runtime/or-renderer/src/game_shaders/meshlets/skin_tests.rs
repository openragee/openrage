//! The skin kind of GTA VI's tiled lighting, as the resolve ports it: the matte of its program on
//! made-up inputs, and skin lit brighter than the standard material under the same sun.

use super::scene_tests::frames_stated;
use super::tests::{surface, SIZE};
use super::translucency_tests::{computed, leaf_functions};
use super::*;

/// What the pixel `x` of a row computes: the skin's matte at four cosines, then the kind of the
/// skin's byte and the resolve's own number for it.
const CASES: &str = r#"
const PI: f32 = 3.14159265;

@vertex fn corner(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let at = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return vec4<f32>(at, 0.0, 1.0);
}

@fragment fn worked(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    switch u32(at.x) {
        case 0u: { return vec4<f32>(skin_matte(1.0), skin_matte(0.5), skin_matte(0.0), skin_matte(-0.5)); }
        case 1u: { return vec4<f32>(f32(kind_of(29.0 / 255.0)), f32(SKIN), f32(skin_matte(0.005)), 0.0); }
        default: { return vec4<f32>(0.0); }
    }
}
"#;

fn near(seen: f32, expected: f64, within: f64) -> bool {
    (f64::from(seen) - expected).abs() <= within
}

#[cfg_attr(
    not(feature = "gpu-tests"),
    ignore = "needs a GPU adapter: acquisition can hang, not just fail"
)]
#[test]
fn skin_s_matte_is_the_falloff_table_s_first_share_over_pi_and_its_kind_is_twenty_nine() {
    let source = format!("{CASES}{}", leaf_functions());
    let Some(seen) = computed("skin_s_matte_is_the_falloff_table_s_first_share", &source) else {
        return;
    };
    // Worked by hand: the table's first share is the cosine, held to 0.999, and soft only within
    // half a degree of the terminator.
    let [head_on, half, edge, behind] = seen[0];
    assert!(near(head_on, 0.317992, 1e-5), "{head_on}");
    assert!(near(half, 0.159155, 1e-5), "{half}");
    assert!(near(edge, 0.000675, 2e-5), "{edge}");
    assert_eq!(behind, 0.0);
    let [kind, skin, grazing, _] = seen[1];
    assert_eq!((kind, skin), (29.0, 29.0));
    assert!(near(grazing, 0.001719, 2e-5), "{grazing}");
}

/// The test model's pixel stage, its colour target's fourth channel the kind `kind` and its
/// specular colour nought, so what it sends back is its matte alone.
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
        vec4<f32>(0.0),
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
fn skin_under_a_sun_overhead_is_lit_brighter_than_the_standard_material() {
    let filmic = Filmic([0.22, 0.3, 0.1, 0.2, 0.01, 0.3, 11.2]);
    let curve = Curve {
        scale: -std::f32::consts::LOG2_E,
        vertical: 10.0,
        horizontal: 0.0,
        bending: 0.0,
    };
    // The model faces up and the sun stands over it; nothing else gives light.
    let overhead = Sky {
        towards: Vec3::Z,
        sun: Vec3::splat(1.0),
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
    for kind in [0, 29] {
        let mut model = surface();
        model.fragment = std::sync::Arc::new(kind_stage(kind));
        let Some(frames) = frames_stated(
            "skin_under_a_sun_overhead_is_lit_brighter",
            model,
            &[(Mat4::IDENTITY, overhead)],
            |_, _, _| {},
        ) else {
            return;
        };
        seen.push(middle(&frames[0]));
    }
    // The standard matte takes the roughness factor, 1 - 0.3377483 a, about 0.92 here, and skin
    // does not: the same colour lit brighter in every channel it holds.
    let (standard, skin) = (seen[0], seen[1]);
    assert!(
        standard[0] > 20,
        "the standard material is lit: {standard:?}"
    );
    assert!(
        (0..3).all(|c| skin[c] > standard[c] || standard[c] == 0),
        "skin {skin:?} against the standard {standard:?}"
    );
}
