//! The programs of GTA VI's passes after the frame is lit, each a module of its own over the same header.

/// What every pass shares: the triangle over the screen, the texture it reads and the sampler.
const HEADER: &str = r#"
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> Out {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Out;
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2125, 0.7154, 0.0721));
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(source, linear_clamp, uv, 0.0).rgb;
}
"#;

/// A halving of the bloom: thirteen taps, a box of four inside and four about it; the first halving weighs each box down by its luminance.
const HALVE: &str = r#"
fn halve(uv: vec2<f32>, karis: bool) -> vec3<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(source));
    let a = tap(uv + t * vec2<f32>(-2.0, -2.0));
    let b = tap(uv + t * vec2<f32>(0.0, -2.0));
    let c = tap(uv + t * vec2<f32>(2.0, -2.0));
    let d = tap(uv + t * vec2<f32>(-1.0, -1.0));
    let e = tap(uv + t * vec2<f32>(1.0, -1.0));
    let f = tap(uv + t * vec2<f32>(-2.0, 0.0));
    let g = tap(uv);
    let h = tap(uv + t * vec2<f32>(2.0, 0.0));
    let i = tap(uv + t * vec2<f32>(-1.0, 1.0));
    let j = tap(uv + t * vec2<f32>(1.0, 1.0));
    let k = tap(uv + t * vec2<f32>(-2.0, 2.0));
    let l = tap(uv + t * vec2<f32>(0.0, 2.0));
    let m = tap(uv + t * vec2<f32>(2.0, 2.0));
    var boxes = array<vec3<f32>, 5>(
        (d + e + i + j) * 0.25,
        (a + b + f + g) * 0.25,
        (b + c + g + h) * 0.25,
        (f + g + k + l) * 0.25,
        (g + h + l + m) * 0.25,
    );
    var weights = array<f32, 5>(0.5, 0.125, 0.125, 0.125, 0.125);
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    for (var n = 0u; n < 5u; n++) {
        var w = weights[n];
        if (karis) {
            w = w / (1.0 + luminance(boxes[n]));
        }
        sum += boxes[n] * w;
        total += w;
    }
    return min(sum / max(total, 1e-6), vec3<f32>(65000.0));
}

@fragment
fn fs_halve_first(v: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(halve(v.uv, true), 1.0);
}

@fragment
fn fs_halve(v: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(halve(v.uv, false), 1.0);
}

@fragment
fn fs_double(v: Out) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(source));
    var sum = tap(v.uv) * 0.25;
    sum += (tap(v.uv + vec2<f32>(t.x, 0.0)) + tap(v.uv - vec2<f32>(t.x, 0.0))
        + tap(v.uv + vec2<f32>(0.0, t.y)) + tap(v.uv - vec2<f32>(0.0, t.y))) * 0.125;
    sum += (tap(v.uv + t) + tap(v.uv - t) + tap(v.uv + vec2<f32>(t.x, -t.y))
        + tap(v.uv + vec2<f32>(-t.x, t.y))) * 0.0625;
    return vec4<f32>(min(sum, vec3<f32>(65000.0)), 1.0);
}
"#;

/// The bloom laid in place of a part of the frame, then the per-pixel exposure.
const COMPOSITE: &str = r#"
struct Composite {
    // bloom mix, per-pixel amount, the frame's stops, its linear exposure
    mix_amount_stops_exposure: vec4<f32>,
    // the per-pixel curve: scale, horizontal offset, vertical offset, bending
    curve: vec4<f32>,
    // least, most, offset, contrast
    clamps: vec4<f32>,
    // the compensation curve's input range and its output range
    ranges: vec4<f32>,
    compensation: array<vec4<f32>, 8>,
}

@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var local_light: texture_2d<f32>;
@group(0) @binding(4) var<uniform> composite: Composite;

fn compensation_at(ev: f32) -> f32 {
    let r = composite.ranges;
    let along = clamp((ev - r.x) / max(r.y - r.x, 1e-6), 0.0, 1.0) * 31.0;
    let i = min(u32(floor(along)), 31u);
    let j = min(i + 1u, 31u);
    let a = composite.compensation[i / 4u][i % 4u];
    let b = composite.compensation[j / 4u][j % 4u];
    let y = mix(a, b, along - f32(i));
    return clamp(y, min(r.z, r.w), max(r.z, r.w));
}

// The light about a pixel read through a cubic B-spline of its texels, four bilinear taps: a bilinear
// read of so coarse a size bends the clamp's edge along its texels into a square round the sun.
// Stand-in: the build reads its own target here, whose making is not read.
fn smooth_local(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(local_light));
    let p = uv * size - 0.5;
    let i = floor(p);
    let f = p - i;
    let w0 = (1.0 - f) * (1.0 - f) * (1.0 - f) / 6.0;
    let w1 = (4.0 - 6.0 * f * f + 3.0 * f * f * f) / 6.0;
    let w3 = f * f * f / 6.0;
    let w2 = 1.0 - w0 - w1 - w3;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let c0 = (i - 0.5 + w1 / g0) / size;
    let c1 = (i + 1.5 + w3 / g1) / size;
    return g0.y * (g0.x * textureSampleLevel(local_light, linear_clamp, c0, 0.0).rgb
            + g1.x * textureSampleLevel(local_light, linear_clamp, vec2<f32>(c1.x, c0.y), 0.0).rgb)
        + g1.y * (g0.x * textureSampleLevel(local_light, linear_clamp, vec2<f32>(c0.x, c1.y), 0.0).rgb
            + g1.x * textureSampleLevel(local_light, linear_clamp, c1, 0.0).rgb);
}

@fragment
fn fs_composite(v: Out) -> @location(0) vec4<f32> {
    let p = composite.mix_amount_stops_exposure;
    let frame = textureSampleLevel(source, linear_clamp, v.uv, 0.0);
    let blurred = textureSampleLevel(bloom, linear_clamp, v.uv, 0.0).rgb;
    // What the bloom spreads is what it takes from the frame
    var light = mix(frame.rgb, blurred, p.x);
    // ⚠ The alpha leaves here holding the scene's luminance before the per-pixel exposure, in cd/m², which the rod shift after the mapping is weighed by.
    let scene = luminance(light) / max(p.w, 1e-30);
    if (p.y > 0.0) {
        // The frame holds its light at the frame's exposure: the luminance about the pixel is that over the exposure
        let about = luminance(smooth_local(v.uv)) / max(p.w, 1e-30);
        let c = composite.curve;
        let ev = c.x * log(max(c.y + about, 1e-6)) + c.w * about + c.z - 10.0;
        let k = composite.clamps;
        let own = clamp(clamp(ev + k.w * compensation_at(ev), k.x, k.y) + k.z, k.x, k.y);
        light *= exp2(p.y * (own - p.z));
    }
    return vec4<f32>(light, scene);
}
"#;

/// The frame laid over its history where the surface stood the frame before, the history held within the pixels about it.
const TEMPORAL: &str = r#"
struct Temporal {
    // From a pixel and its depth to the world, as the frame was drawn this time
    unproject: mat4x4<f32>,
    // From the world to the screen, this frame and the last, both without their offsets
    now: mat4x4<f32>,
    before: mat4x4<f32>,
    eye: vec4<f32>,
    // share taken near, share taken far, the distance far starts at, the deviations the history is held within
    shares: vec4<f32>,
    // whether there is a history to read, and the factor that brings it to this frame's exposure
    flags: vec4<f32>,
    // the nine tent weights of the 3x3 about the pixel, row by row, that put this frame back on the pixel's middle
    weights: array<vec4<f32>, 3>,
}

@group(0) @binding(2) var history: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var<uniform> temporal: Temporal;

// A colour weighed down by its brightness, so that one bright texel neither sets the history's bounds nor flickers
fn tamed(c: vec3<f32>) -> vec3<f32> {
    return c / (1.0 + max(c.r, max(c.g, c.b)));
}

fn untamed(c: vec3<f32>) -> vec3<f32> {
    return c / max(1.0 - max(c.r, max(c.g, c.b)), 1e-4);
}

fn screen_of(m: mat4x4<f32>, world: vec4<f32>) -> vec2<f32> {
    let clip = m * world;
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

@fragment
fn fs_temporal(v: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(v.position.xy);
    var mean = vec3<f32>(0.0);
    var square = vec3<f32>(0.0);
    var resolved = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let raw = textureLoad(source, clamp(at + vec2<i32>(x, y), vec2<i32>(0), size - 1), 0).rgb;
            let k = u32((y + 1) * 3 + x + 1);
            resolved += raw * temporal.weights[k / 4u][k % 4u];
            let c = tamed(raw);
            mean += c;
            square += c * c;
        }
    }
    let here = vec4<f32>(resolved, textureLoad(source, at, 0).a);
    if (temporal.flags.x < 0.5) {
        return here;
    }
    mean /= 9.0;
    let deviation = sqrt(max(square / 9.0 - mean * mean, vec3<f32>(0.0)));
    let low = mean - deviation * temporal.shares.w;
    let high = mean + deviation * temporal.shares.w;
    // Where the surface at this pixel stood on the screen the frame before: the sky lies on the far plane
    let z = textureLoad(depth, at, 0);
    let ndc = vec2<f32>(v.uv.x * 2.0 - 1.0, 1.0 - v.uv.y * 2.0);
    let unprojected = temporal.unproject * vec4<f32>(ndc, z, 1.0);
    let world = vec4<f32>(unprojected.xyz / unprojected.w, 1.0);
    let back = v.uv + screen_of(temporal.before, world) - screen_of(temporal.now, world);
    if (any(back < vec2<f32>(0.0)) || any(back > vec2<f32>(1.0))) {
        return here;
    }
    let before = textureSampleLevel(history, linear_clamp, back, 0.0).rgb * temporal.flags.y;
    let held = clamp(tamed(before), low, high);
    let far = clamp(distance(world.xyz, temporal.eye.xyz) / max(temporal.shares.z, 1e-3), 0.0, 1.0);
    let share = mix(temporal.shares.x, temporal.shares.y, far);
    return vec4<f32>(untamed(mix(held, tamed(here.rgb), share)), here.a);
}
"#;

/// The mapped frame's darkest pixels shifted towards a rod's sight, then pushed from its four neighbours by the build's amount.
const SHARPEN: &str = r#"
struct Sharpen {
    amount: vec4<f32>,
    // the rod shift's tint and strength, then its weight's scale and bias
    tint: vec4<f32>,
    weight: vec4<f32>,
}

@group(0) @binding(2) var<uniform> sharpen: Sharpen;
// The lit frame, whose alpha holds the scene's luminance in cd/m²
@group(0) @binding(3) var scene: texture_2d<f32>;

// A rod's luminance of the colour from its XYZ, mixed in where the scene is darker than about
// 0.04 cd/m², as the composite does after its curve.
// ⚠ Inferred: on the values the mapped target holds; whether the build shifts before or after its own encoding is a branch the program does not show.
fn rod_shifted(c: vec3<f32>, luminance: f32) -> vec3<f32> {
    let x = dot(c, vec3<f32>(0.5149, 0.3244, 0.1607));
    let y = max(0.0, dot(c, vec3<f32>(0.2654, 0.6704, 0.0642)));
    let z = max(0.0, dot(c, vec3<f32>(0.0248, 0.1248, 0.8504)));
    let rod = ((z + y) / max(0.01, x) + 1.0) * 1.33 - 1.68;
    let w = saturate(0.04 / (0.04 + luminance) * sharpen.weight.x + sharpen.weight.y);
    return c + w * (sharpen.tint.w * rod * y * sharpen.tint.rgb - c);
}

fn texel(at: vec2<i32>, size: vec2<i32>) -> vec3<f32> {
    let p = clamp(at, vec2<i32>(0), size - 1);
    return rod_shifted(textureLoad(source, p, 0).rgb, textureLoad(scene, p, 0).a);
}

@fragment
fn fs_sharpen(v: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(v.position.xy);
    let here = texel(at, size);
    let around = texel(at + vec2<i32>(1, 0), size) + texel(at - vec2<i32>(1, 0), size)
        + texel(at + vec2<i32>(0, 1), size) + texel(at - vec2<i32>(0, 1), size);
    return vec4<f32>(max(here + (4.0 * here - around) * sharpen.amount.x, vec3<f32>(0.0)), 1.0);
}
"#;

/// The source of the module that holds `part`'s passes.
pub(super) fn module(part: Part) -> String {
    let body = match part {
        Part::Halve => HALVE,
        Part::Composite => COMPOSITE,
        Part::Temporal => TEMPORAL,
        Part::Sharpen => SHARPEN,
    };
    format!("{HEADER}{body}")
}

/// A module of the passes.
#[derive(Debug, Clone, Copy)]
pub(super) enum Part {
    Halve,
    Composite,
    Temporal,
    Sharpen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_of_the_passes_parses_and_validates() {
        for part in [Part::Halve, Part::Composite, Part::Temporal, Part::Sharpen] {
            let parsed = naga::front::wgsl::parse_str(&module(part))
                .unwrap_or_else(|e| panic!("{part:?}: {e}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&parsed)
            .unwrap_or_else(|e| panic!("{part:?}: {e:?}"));
        }
    }
}
