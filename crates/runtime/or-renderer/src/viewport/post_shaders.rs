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

/// A halving of the bloom: thirteen taps, a box of four inside and four about it; the first halving weighs each tap down by its luminance within its box.
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
    // Each box is the mean of its four taps, each tap weighed down by its own luminance where `karis`; the boxes then weigh what they always weigh, so a box wholly inside a bright source keeps all of it.
    let sum = 0.5 * box_of(d, e, i, j, karis)
        + 0.125 * (box_of(a, b, f, g, karis) + box_of(b, c, g, h, karis)
        + box_of(f, g, k, l, karis) + box_of(g, h, l, m, karis));
    return min(sum, vec3<f32>(65000.0));
}

fn box_of(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>, karis: bool) -> vec3<f32> {
    var w = vec4<f32>(1.0);
    if (karis) {
        w = 1.0 / (1.0 + vec4<f32>(luminance(a), luminance(b), luminance(c), luminance(d)));
    }
    return (a * w.x + b * w.y + c * w.z + d * w.w) / max(w.x + w.y + w.z + w.w, 1e-6);
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
    // the local tone: contrast under middle grey, over it, of the detail, and the blur's share of the light about a pixel
    tone: vec4<f32>,
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
    let t = composite.tone;
    if (any(t.xyz != vec3<f32>(1.0))) {
        // The local tone, in stops of the exposed frame about middle grey: the light about the pixel keeps
        // its contrast t.x under middle grey and t.y over it, and the pixel keeps t.z of its own about that light.
        // Stand-in: the build's light about a pixel blends a bilateral grid, whose making is not read, with a
        // blur; the grid's cell at the pixel's own luminance is taken as the pixel's own.
        let middle = log2(0.18);
        let pixel = log2(max(luminance(light), 1e-12));
        let blurred = log2(max(luminance(smooth_local(v.uv)), 1e-12));
        let base = mix(pixel, blurred, t.w);
        let contrast = select(t.x, t.y, base > middle);
        light *= exp2(middle + contrast * (base - middle) + t.z * (pixel - base) - pixel);
    }
    return vec4<f32>(light, scene);
}
"#;

/// The frame laid over its history where the surface stood the frame before, the history clipped toward the pixels about it as the build's resolve does.
const TEMPORAL: &str = r#"
struct Temporal {
    // From a pixel and its depth to the world, as the frame was drawn this time
    unproject: mat4x4<f32>,
    // From the world to the screen, this frame and the last, both without their offsets
    now: mat4x4<f32>,
    before: mat4x4<f32>,
    eye: vec4<f32>,
    // share taken near, share taken far, the distance far starts at, share taken where the frame responds fast
    shares: vec4<f32>,
    // whether there is a history to read, whether a tree's pixels respond fast, the stencil's material mask and a tree's material
    flags: vec4<f32>,
    // the deviations the history is held within: least, most, the speed in screens a frame the least is reached at, where the frame responds fast
    variance: vec4<f32>,
    // what a flicker adds to the deviations, the relative flicker of the luminance it starts and is whole at, and the factor that brings the history to this frame's exposure
    flicker: vec4<f32>,
    // the nine weights of the 3x3 about the pixel, row by row, that put this frame back on the pixel's middle
    weights: array<vec4<f32>, 3>,
}

@group(0) @binding(2) var history: texture_2d<f32>;
@group(0) @binding(3) var depth: texture_depth_2d;
@group(0) @binding(4) var<uniform> temporal: Temporal;
@group(0) @binding(5) var stencil: texture_2d<u32>;

fn screen_of(m: mat4x4<f32>, world: vec4<f32>) -> vec2<f32> {
    let clip = m * world;
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.25 * (c.r + c.b) + 0.5 * c.g, 0.5 * (c.r - c.b), 0.5 * c.g - 0.25 * (c.r + c.b));
}

fn rgb_of(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// The history at `uv` through five taps of a Catmull-Rom spline, the corners left out and the rest made to sum one
fn history_at(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(history));
    let p = uv * size;
    let c = floor(p - 0.5) + 0.5;
    let f = p - c;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (c - 1.0) / size;
    let t3 = (c + 2.0) / size;
    let t12 = (c + w2 / w12) / size;
    let a = w12.x * w0.y;
    let b = w0.x * w12.y;
    let m = w12.x * w12.y;
    let d = w3.x * w12.y;
    let e = w12.x * w3.y;
    let sum = textureSampleLevel(history, linear_clamp, vec2<f32>(t12.x, t0.y), 0.0).rgb * a
        + textureSampleLevel(history, linear_clamp, vec2<f32>(t0.x, t12.y), 0.0).rgb * b
        + textureSampleLevel(history, linear_clamp, t12, 0.0).rgb * m
        + textureSampleLevel(history, linear_clamp, vec2<f32>(t3.x, t12.y), 0.0).rgb * d
        + textureSampleLevel(history, linear_clamp, vec2<f32>(t12.x, t3.y), 0.0).rgb * e;
    return sum / (a + b + m + d + e);
}

@fragment
fn fs_temporal(v: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let at = vec2<i32>(v.position.xy);
    // This frame on the pixel's middle, each texel weighed down by its brightness so that one bright texel does not flicker
    var resolved = vec3<f32>(0.0);
    var weight = 0.0;
    var mean = vec3<f32>(0.0);
    var square = vec3<f32>(0.0);
    var least = 1e30;
    var most = -1e30;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let raw = textureLoad(source, clamp(at + vec2<i32>(x, y), vec2<i32>(0), size - 1), 0).rgb;
            let k = u32((y + 1) * 3 + x + 1);
            let w = temporal.weights[k / 4u][k % 4u] / (1.0 + dot(raw, vec3<f32>(0.2125, 0.7154, 0.0721)));
            resolved += raw * w;
            weight += w;
            let c = ycocg(raw);
            mean += c;
            square += c * c;
            least = min(least, c.x);
            most = max(most, c.x);
        }
    }
    let alpha = textureLoad(source, at, 0).a;
    let here = resolved / max(weight, 1e-30);
    if (temporal.flags.x < 0.5) {
        return vec4<f32>(here, alpha);
    }
    // Where the surface at this pixel stood on the screen the frame before: the sky lies on the far plane
    let z = textureLoad(depth, at, 0);
    let ndc = vec2<f32>(v.uv.x * 2.0 - 1.0, 1.0 - v.uv.y * 2.0);
    let unprojected = temporal.unproject * vec4<f32>(ndc, z, 1.0);
    let world = vec4<f32>(unprojected.xyz / unprojected.w, 1.0);
    let back = v.uv + screen_of(temporal.before, world) - screen_of(temporal.now, world);
    if (any(back < vec2<f32>(0.0)) || any(back > vec2<f32>(1.0))) {
        return vec4<f32>(here, alpha);
    }
    var held = history_at(back) * temporal.flicker.w;
    if (any(held < vec3<f32>(0.0))) {
        held = here;
    }
    let now = ycocg(here);
    var then = ycocg(held);
    // A flicker: how far the history's luminance, held within the 3x3's, is from this frame's, against the brighter
    let kept = clamp(then.x, least, most);
    let flicker = abs(kept - now.x) / max(max(kept, now.x), 0.2);
    let t = saturate((flicker - temporal.flicker.y) / max(temporal.flicker.z - temporal.flicker.y, 1e-6));
    let flickers = t * t * (3.0 - 2.0 * t);
    let moved = (back - v.uv) * vec2<f32>(size);
    let still = saturate(1.0 - 2.0 * length(moved));
    let speed = saturate(length(moved) / f32(size.x) / max(temporal.variance.z, 1e-6));
    var gamma = mix(temporal.variance.y + temporal.flicker.x * flickers * still, temporal.variance.x, speed);
    let material = textureLoad(stencil, at, 0).r & u32(temporal.flags.z);
    let responsive = temporal.flags.y > 0.5 && material == u32(temporal.flags.w);
    if (responsive) {
        gamma = min(temporal.variance.w, gamma);
    }
    // The history clipped toward the 3x3's mean where it lies outside the deviations gamma allows
    mean /= 9.0;
    let deviation = sqrt(abs(square / 9.0 - mean * mean));
    let off = then - mean;
    let reach = max3(abs(off) / (gamma * deviation + 1e-8));
    if (reach > 1.0) {
        then = mean + off / reach;
    }
    let far = clamp(distance(world.xyz, temporal.eye.xyz) / max(temporal.shares.z, 1e-3), 0.0, 1.0);
    var share = mix(select(temporal.shares.x, temporal.shares.w, responsive), temporal.shares.y, far);
    // A flickering pixel keeps at least nine tenths of its history, one that responds fast the history of one that does not
    let calm = select(min(share, 0.1), temporal.shares.x, responsive);
    share = mix(share, calm, flickers);
    return vec4<f32>(max(rgb_of(mix(then, now, share)), vec3<f32>(0.0)), alpha);
}

fn max3(v: vec3<f32>) -> f32 {
    return max(v.x, max(v.y, v.z));
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
