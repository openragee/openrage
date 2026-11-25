//! The program GTA VI's water is drawn with over the lit frame: its quads, and the sea past them to the horizon.

pub(super) const WATER_SHADER: &str = r#"
// Every field of the viewport's `CameraUniform`, in order.
struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
// The world's light: only its layout matters here.
struct Light {
    sun_travel: vec4<f32>,
    sun_colour: vec4<f32>,
    ambient_down: vec4<f32>,
    ambient_up: vec4<f32>,
    frame: vec4<f32>,
};
// `WaterUniform`: the medium a metre at a time, its colour and the light it takes, the frame's size and the day's fog.
struct Water {
    extinction: vec4<f32>,
    // In `w` what the reflection is multiplied by.
    albedo: vec4<f32>,
    // The ambient light the water takes; in `w` the depth over which a shore fades in, in metres.
    light: vec4<f32>,
    // The sun's light on the surface, by how high it stands.
    sun: vec4<f32>,
    // How far each channel's scattering leans forward (the phase's asymmetry).
    phase: vec4<f32>,
    // One over the frame's width and height.
    screen: vec4<f32>,
    // The medium's grid: its west and north edges in metres and one over its extent; nought in `z` where there is none.
    grid: vec4<f32>,
    // The same of the finer medium about the camera.
    near: vec4<f32>,
    // The quads' bounds, west, south, east and north, past which the sea runs on.
    bounds: vec4<f32>,
    // The height of the sea past the quads, and in `y` one where it is drawn.
    sea: vec4<f32>,
    fog: array<vec4<f32>, 5>,
    fog_colour: array<vec4<f32>, 4>,
    // Each wave's number a metre along x and y, its height and its phase.
    waves: array<vec4<f32>, 12>,
    // The world to each cascade of the sun's shadow, and in `shadow_on.x` one where they are cast.
    shadow: array<mat4x4<f32>, 4>,
    shadow_on: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> water: Water;
@group(1) @binding(1) var behind: texture_2d<f32>;
// The frame's opaque depth at half its size, as distances along the view.
@group(1) @binding(2) var behind_depth: texture_2d<f32>;
@group(1) @binding(3) var sky: texture_2d<f32>;
// For the cosine of the angle a ray meets the surface at, how much it reflects, in red.
@group(1) @binding(4) var fresnel_row: texture_2d<f32>;
@group(1) @binding(6) var clamped: sampler;
// The medium by place: how much of each channel a metre takes out, and how much it scatters.
@group(1) @binding(8) var medium_extinction: texture_2d<f32>;
@group(1) @binding(9) var medium_scattering: texture_2d<f32>;
// The same about the camera, at the atlas's own size.
@group(1) @binding(10) var near_extinction: texture_2d<f32>;
@group(1) @binding(11) var near_scattering: texture_2d<f32>;
// The sun's shadow: the cascades' tiles one above the other, each the depth along the light of what casts.
@group(1) @binding(12) var shadow_map: texture_depth_2d;
@group(1) @binding(13) var shadow_compare: sampler_comparison;
@group(2) @binding(0) var<uniform> light: Light;

// A depth of the frame at or past this is the sky, or no depth at all.
const SKY: f32 = 50000.0;
// The least spread of slopes the glint takes, a flat sea's: GTA V's falloff of 1118 as a spread (assumed: the build's own floor is a constant not traced).
const GLINT_FLOOR: f32 = 2.0 / 1120.0;
// How the build's water turns its spread of slopes into the roughness it is lit with: alpha = 0.75 sqrt(2 sigma^2).
const ROUGHNESS_OF_SPREAD: f32 = 0.75;
// The thick-medium blend of the scattering's phase: its step, the frame's 0.15, and the even phase it goes to, 4 times the three lobes' 1.75.
const THICK_STEP: f32 = 0.15;
const THICK_EVEN: f32 = 4.0 * 1.75;
const PI: f32 = 3.14159265;
// How many steps a reflected ray takes across the frame, how many halve the last, and how each step grows.
const TRACE_STEPS: i32 = 32;
const REFINE_STEPS: i32 = 5;
// How many pixels a wave must span before it is drawn in full; a shorter one's slope goes into the surface's roughness instead.
const WAVE_PIXELS: f32 = 6.0;
const TRACE_GROWTH: f32 = 1.35;
// The texels a radian of the sky's map spans at its horizon: each paraboloid's disc is 64 texels across its radius.
const SKY_TEXELS: f32 = 64.0;
// How much slower light goes in water than in air.
const WATER_INDEX: f32 = 1.333;
// The depth given to the sea past the quads, whose bed is not known: deeper than light goes.
const DEEP: f32 = 1000.0;
// How far nearer the sun than its own depth a point of the water is read in a cascade: the surface casts nothing, so only the map's own steps matter.
const SHADOW_BIAS: f32 = 0.0005;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) ground_depth: f32,
    @location(2) view_depth: f32,
};

// The vertex's colour holds in red how deep the ground lies under it.
@vertex
fn vs_water(@location(0) position: vec3<f32>, @location(3) colour: vec4<f32>) -> VsOut {
    var out: VsOut;
    out.clip = camera.view_proj * vec4<f32>(position, 1.0);
    out.world = position;
    out.ground_depth = colour.r;
    out.view_depth = out.clip.w;
    return out;
}

// The texel of the map of two paraboloids that looks along `d`: the upper half of the sky on the right, the lower on the left.
fn paraboloid(d: vec3<f32>) -> vec2<f32> {
    let s = d.xy / (1.0 + abs(d.z));
    if (d.z >= 0.0) {
        return vec2<f32>(0.75 + 0.25 * s.x, 0.5 - 0.5 * s.y);
    }
    return vec2<f32>(0.25 - 0.25 * s.x, 0.5 - 0.5 * s.y);
}

// Where a point lies on the frame, and how far along the view; `w` at or below nought behind the eye.
fn on_frame(p: vec3<f32>) -> vec3<f32> {
    let clip = camera.view_proj * vec4<f32>(p, 1.0);
    let ndc = clip.xy / max(clip.w, 1e-4);
    return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, clip.w);
}

// Whether the frame drew something at the depth scene: the sky and no depth at all read at or past the far plane.
fn drawn_at(scene: f32) -> bool {
    return scene > 0.0 && scene < min(SKY, camera.camera_misc.y * 0.995);
}

// Whether a point on the frame lies behind what the frame drew there, and by how far.
fn behind_drawn(at: vec3<f32>) -> f32 {
    let scene = textureSampleLevel(behind_depth, clamped, at.xy, 0.0).r;
    if (!drawn_at(scene)) {
        return -1.0;
    }
    return at.z - scene;
}

// The lit frame where a ray from `start` along `r` first passes behind what the frame drew, and in `a` how far it is to be trusted: nought where it leaves the frame or meets nothing.
fn traced(start: vec3<f32>, r: vec3<f32>) -> vec4<f32> {
    var before = 0.0;
    var t = 0.5;
    for (var i = 0; i < TRACE_STEPS; i++) {
        let at = on_frame(start + r * t);
        if (at.z <= 0.0 || any(at.xy < vec2<f32>(0.0)) || any(at.xy > vec2<f32>(1.0))) {
            return vec4<f32>(0.0);
        }
        let past = behind_drawn(at);
        if (past > 0.0) {
            // A ray that has gone far behind a surface went past its edge, not into it.
            if (past > 2.0 * (t - before) + 1.0) {
                return vec4<f32>(0.0);
            }
            var lo = before;
            var hi = t;
            for (var k = 0; k < REFINE_STEPS; k++) {
                let mid = 0.5 * (lo + hi);
                if (behind_drawn(on_frame(start + r * mid)) > 0.0) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let hit = on_frame(start + r * hi);
            let edge = min(min(hit.x, 1.0 - hit.x), min(hit.y, 1.0 - hit.y));
            let trust = smoothstep(0.0, 0.08, edge) * smoothstep(0.0, 0.15, r.z + 0.15);
            return vec4<f32>(textureSampleLevel(behind, clamped, hit.xy, 0.0).rgb, trust);
        }
        before = t;
        t = t * TRACE_GROWTH + 0.5;
    }
    return vec4<f32>(0.0);
}

// The day's fog along `ray`, from the eye to a point: its colour, and in `a` how much of the point it hides.
fn fog_of(ray: vec3<f32>) -> vec4<f32> {
    let p = water.fog;
    let full = max(length(ray), 1e-3);
    let dist = max(0.0, full - p[0].x);
    let dz = ray.z * (dist / full);
    let t = p[2].z * dz;
    var integral = 1.0;
    if (abs(dz) > 0.01) {
        integral = (1.0 - exp(-t)) / t;
    }
    let ground = (1.0 - saturate(exp(min(1.0, p[1].w * dist * integral)))) * p[2].y;
    let towards = ray / full;
    let moon = pow(saturate(dot(towards, p[4].xyz)), p[4].w);
    let sun = pow(saturate(dot(towards, p[3].xyz)), p[3].w);
    let haze_blend = p[1].y * (1.0 - ground);
    let haze = haze_blend * (1.0 - exp(p[1].x * max(0.0, dist - p[2].x)));
    let atmosphere = 1.0 - exp(-p[1].z * dist);
    let c = water.fog_colour;
    let moonlit = mix(c[1].rgb, c[3].rgb, moon);
    let air = mix(moonlit, c[0].rgb, sun);
    let near = mix(c[2].rgb, air, atmosphere);
    let colour = mix(near, vec3<f32>(c[0].a, c[1].a, c[2].a), haze_blend);
    return vec4<f32>(colour, saturate(haze + ground));
}

// How much of the sun reaches `p`: the first cascade it lies across, as the frame's own receiving pass takes it; whole past them.
fn sunlit(p: vec3<f32>) -> f32 {
    if (water.shadow_on.x < 0.5) {
        return 1.0;
    }
    for (var i = 0; i < 4; i++) {
        let clip = water.shadow[i] * vec4<f32>(p, 1.0);
        let q = clip.xyz / clip.w;
        if (all(abs(q.xy) < vec2<f32>(0.98)) && q.z >= 0.0 && q.z <= 1.0) {
            let uv = vec2<f32>(0.5 + 0.5 * q.x, (f32(i) + 0.5 - 0.5 * q.y) * 0.25);
            return textureSampleCompareLevel(shadow_map, shadow_compare, uv, q.z - SHADOW_BIAS);
        }
    }
    return 1.0;
}

// Henyey and Greenstein's phase for each channel's asymmetry `g`, against the even phase.
fn lobe(c: f32, g: vec3<f32>) -> vec3<f32> {
    let gg = g * g;
    return (1.0 - gg) / pow(max(1.0 + gg - 2.0 * g * c, vec3<f32>(1e-4)), vec3<f32>(1.5));
}

// The water's phase, against the even phase, through the angle whose cosine is `c`: three lobes of the asymmetry, its half and its quarter, weighted 1, 1/2 and 1/4, so that light scattered more than once comes back more evenly.
// ⚠ Their sum is 1.75 times the even phase's light, as the game's water programs take it.
fn phase_of(c: f32, g: vec3<f32>) -> vec3<f32> {
    return lobe(c, g) + 0.5 * lobe(c, 0.5 * g) + 0.25 * lobe(c, 0.25 * g);
}

// The water's colour at `world`, seen at the frame's pixel `pixel`, `ground_depth` above the ground and `view_depth` along the view; in `a` how far a shore lets it show.
fn shade(world: vec3<f32>, pixel: vec2<f32>, ground_depth: f32, view_depth: f32) -> vec4<f32> {
    let to_eye = camera.camera_pos_ws.xyz - world;
    let dist = max(length(to_eye), 1e-3);
    let v = to_eye / dist;
    var e = vec2<f32>(1.0, 0.0);
    if (length(to_eye.xy) > 1e-3) {
        e = normalize(to_eye.xy);
    }
    // The surface's slope, summed over its waves: each a sine along its number; what the frame cannot show of one is its share of the slopes' spread, towards the eye and across.
    let footprint = max(length(fwidth(world.xy)), 1e-4);
    var slope = vec2<f32>(0.0);
    var spread_along = 0.0;
    var spread_across = 0.0;
    for (var i = 0; i < 12; i++) {
        let w = water.waves[i];
        let number = length(w.xy);
        if (number <= 0.0) {
            continue;
        }
        let seen = smoothstep(1.0, 2.0, 6.2831853 / (number * footprint * WAVE_PIXELS * 0.5));
        let amplitude = w.xy * w.z;
        slope += amplitude * (cos(dot(w.xy, world.xy) + w.w) * seen);
        // A sine's slope spreads by half its amplitude squared; what is drawn of it keeps `seen` of that amplitude.
        let gone = 0.5 * (1.0 - seen * seen);
        let along = dot(amplitude, e);
        spread_along += gone * along * along;
        spread_across += gone * max(dot(amplitude, amplitude) - along * along, 0.0);
    }
    let n = normalize(vec3<f32>(-slope, 1.0));
    // How far the ray runs in the water: to what is opaque behind it, or where the frame drew nothing there, down to the ground the height map gives.
    // ⚠ Read between the texels of the half-size depth: the nearest one is a pixel off every other row, which over flat ground at a glance is metres.
    let at_pixel = pixel * water.screen.xy;
    let scene = textureSampleLevel(behind_depth, clamped, at_pixel, 0.0).r;
    let steep = max(v.z, 0.02);
    var path = max(ground_depth, 0.0) / steep;
    let drawn = drawn_at(scene);
    if (drawn) {
        // ⚠ The depth is held in half floats: a difference under one step of them is no water, or flat ground just under the surface shows in bands.
        let step = scene / 1024.0;
        path = max(scene - view_depth - step, 0.0) * dist / max(view_depth, 1e-3);
    }
    let under = textureSampleLevel(behind, clamped, at_pixel, 0.0).rgb;
    // ⚠ What lies behind was fogged as the frame was lit, all the way from the eye: the air's fog is taken off it again, since under the surface it is the water's medium that hides it.
    var bed = under;
    if (drawn) {
        let fogged = fog_of(-to_eye - v * path);
        bed = max(under - fogged.rgb * fogged.a, vec3<f32>(0.0)) / max(1.0 - fogged.a, 0.02);
    }
    var extinction = water.extinction.rgb;
    var albedo = water.albedo.rgb;
    let at = vec2<f32>((world.x - water.grid.x) * water.grid.z, (water.grid.y - world.y) * water.grid.w);
    let close = vec2<f32>((world.x - water.near.x) * water.near.z, (water.near.y - world.y) * water.near.w);
    // The finer medium where it reaches; off its grid the medium is the one the uniform holds, which with a grid is its commonest.
    if (water.near.z > 0.0 && all(close >= vec2<f32>(0.0)) && all(close <= vec2<f32>(1.0))) {
        extinction = max(textureSampleLevel(near_extinction, clamped, close, 0.0).rgb, vec3<f32>(1e-6));
        let scattering = textureSampleLevel(near_scattering, clamped, close, 0.0).rgb;
        albedo = min(scattering / extinction, vec3<f32>(1.0));
    } else if (water.grid.z > 0.0 && all(at >= vec2<f32>(0.0)) && all(at <= vec2<f32>(1.0))) {
        extinction = max(textureSampleLevel(medium_extinction, clamped, at, 0.0).rgb, vec3<f32>(1e-6));
        let scattering = textureSampleLevel(medium_scattering, clamped, at, 0.0).rgb;
        albedo = min(scattering / extinction, vec3<f32>(1.0));
    }
    let through = exp(-extinction * path);
    let fog = fog_of(-to_eye);
    // What the water mirrors, over the slopes the frame cannot show: three normals tilted towards and away from the eye, each drawn from a third of the spread at a place that moves from pixel to pixel, the sky's map blurred by the spread across.
    // ⚠ Fixed points of the spread (Gauss and Hermite's) give two thirds of their weight to the untilted normal: a far skyline then mirrors whole whatever the spread.
    let deviation = sqrt(spread_along);
    var points = 1;
    if (deviation > 1e-3) {
        points = 3;
    }
    let jitter = fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
    var fresnel = 0.0;
    var reflected = vec3<f32>(0.0);
    var weights = 0.0;
    for (var k = 0; k < points; k++) {
        var tilt = 0.0;
        let weight = 1.0;
        if (points > 1) {
            // The normal deviate at the share p of the spread, by the logistic's inverse.
            let p = clamp((f32(k) + jitter) / f32(points), 0.01, 0.99);
            tilt = deviation * log(p / (1.0 - p)) / 1.702;
        }
        let tilted = normalize(vec3<f32>(-(slope + e * tilt), 1.0));
        let cosine = saturate(dot(tilted, v));
        let f = textureSampleLevel(fresnel_row, clamped, vec2<f32>(cosine, 0.5), 0.0).r;
        var r = reflect(-v, tilted);
        // A ray a wave turns down into the water still leaves upward.
        r.z = abs(r.z);
        let blur = log2(max(1.0, 2.0 * sqrt(spread_across) * SKY_TEXELS / (1.0 + r.z)));
        let sky_seen = textureSampleLevel(sky, clamped, paraboloid(r), blur).rgb;
        let world_seen = traced(world, r);
        // ⚠ The frame was fogged where the reflected ray meets it; the sky's map was not, so the air before the surface fogs it here.
        let mirrored = mix(mix(sky_seen, fog.rgb, fog.a), world_seen.rgb, world_seen.a) * water.albedo.w;
        fresnel += weight * f;
        reflected += weight * f * mirrored;
        weights += weight;
    }
    fresnel /= weights;
    reflected /= weights;
    // The glint as the build lights its water: GGX at the roughness its spread of slopes gives, Smith's visibility and the water's reflectance.
    let towards_sun = -light.sun_travel.xyz;
    let a = ROUGHNESS_OF_SPREAD * sqrt(spread_along + spread_across + GLINT_FLOOR);
    let a2 = a * a;
    let h = normalize(towards_sun + v);
    let nh = saturate(dot(n, h));
    let nl = saturate(dot(n, towards_sun));
    let nv = max(dot(n, v), 1e-3);
    let lobe_d = a2 / (PI * pow(nh * nh * (a2 - 1.0) + 1.0, 2.0));
    let visible = 0.5 / max(nl * sqrt(nv * nv * (1.0 - a2) + a2) + nv * sqrt(nl * nl * (1.0 - a2) + a2), 1e-5);
    let glint_f = textureSampleLevel(fresnel_row, clamped, vec2<f32>(saturate(dot(h, v)), 0.5), 0.0).r;
    let glint = sunlit(world) * light.sun_colour.rgb * (lobe_d * visible * glint_f * nl);
    // The light the water scatters to the eye: the sky's even light, and the sun's bent into the water and turned towards the eye by the phase, which goes even as the medium thickens.
    let up = vec3<f32>(0.0, 0.0, 1.0);
    let sunk = refract(normalize(light.sun_travel.xyz), up, 1.0 / WATER_INDEX);
    let looking = refract(-v, up, 1.0 / WATER_INDEX);
    let thick = THICK_STEP * max(max(extinction.r, extinction.g), extinction.b);
    let lobes = phase_of(dot(sunk, -looking), water.phase.rgb);
    let turned = lobes + thick * thick * (vec3<f32>(THICK_EVEN) - lobes);
    let scattered = (1.0 - through) * albedo * (water.light.rgb + water.sun.rgb * turned);
    let own = (1.0 - fresnel) * (bed * through + scattered) + glint;
    let colour = own * (1.0 - fog.a) + (1.0 - fresnel) * fog.rgb * fog.a + reflected;
    // A shore fades in over its first depth, so that the water meets the ground in no line.
    let shore = smoothstep(0.0, water.light.w, path * steep);
    return vec4<f32>(mix(under, colour, shore), 0.0);
}

@fragment
fn fs_water(in: VsOut) -> @location(0) vec4<f32> {
    return shade(in.world, in.clip.xy, in.ground_depth, in.view_depth);
}

struct SeaVs {
    @builtin(position) clip: vec4<f32>,
};

struct SeaOut {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// One triangle over the whole frame.
@vertex
fn vs_sea(@builtin(vertex_index) index: u32) -> SeaVs {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: SeaVs;
    out.clip = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// The sea past the quads: a plane at its height, met by each pixel's ray out to the horizon.
@fragment
fn fs_sea(in: SeaVs) -> SeaOut {
    let ndc = vec2<f32>(in.clip.x * water.screen.x * 2.0 - 1.0, 1.0 - in.clip.y * water.screen.y * 2.0);
    let far = camera.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    let eye = camera.camera_pos_ws.xyz;
    let d = normalize(far.xyz / far.w - eye);
    let above = eye.z - water.sea.x;
    let t = min(above / max(-d.z, 1e-6), 1e6);
    let hit = eye + d * t;
    let inside = all(hit.xy >= water.bounds.xy) && all(hit.xy <= water.bounds.zw);
    let clip = camera.view_proj * vec4<f32>(hit, 1.0);
    var out: SeaOut;
    // ⚠ Past the frame's far plane the sea still runs on: its depth is held at the far plane's, where only the sky is.
    out.depth = clamp(clip.z / max(clip.w, 1e-6), 0.0, 1.0);
    out.colour = shade(hit, in.clip.xy, DEEP, clip.w);
    if (water.sea.y < 0.5 || above <= 0.0 || d.z >= -1e-6 || inside) {
        discard;
    }
    return out;
}
"#;
