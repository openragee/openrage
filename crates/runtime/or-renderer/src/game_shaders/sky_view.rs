//! The sky drawn from a table of its radiance in every direction above the horizon, worked out by an atmosphere model for a moment of the day, in place of the colours the game's dome blends.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use glam::{Mat4, Vec3};

use super::deferred::{DEPTH_STENCIL_FORMAT, HDR_FORMAT};
use super::engine::EngineConstants;
use super::sky_stars::{DiscsPass, PointStars, StarsPass};

/// How many texels the table has round the compass and from the horizon to the zenith.
pub const AROUND: u32 = 128;
pub const UP: u32 = 64;

/// The radiance of the sky towards each texel of the table, the exposure the frame takes it at, and the clouds drawn over it.
#[derive(Clone, PartialEq)]
pub struct SkyView {
    /// Radiance in cd/m², row by row from the horizon up, each row round the compass from the west through the south.
    pub radiance: Arc<Vec<[f32; 3]>>,
    pub exposure: f32,
    pub clouds: Option<Clouds>,
    /// What the night adds over the table: the glow of the air and the stars.
    pub night: Option<Night>,
    pub maps: Option<Arc<SkyMaps>>,
    /// The catalogue's point stars.
    pub stars: Option<Arc<PointStars>>,
}

/// One map of the night sky, every level as 8-bit RGBA, the largest first.
#[derive(Debug)]
pub struct SkyMap {
    pub width: u32,
    pub height: u32,
    pub srgb: bool,
    pub levels: Vec<Vec<u8>>,
}

/// The maps the night sky is drawn with: the star field (red the band of the galaxy, green the sparse stars, alpha the dense ones) and the glow of the air.
#[derive(Debug, Default)]
pub struct SkyMaps {
    pub stars: Option<SkyMap>,
    pub airglow: Option<SkyMap>,
    /// The profile each point star is laid with.
    pub star: Option<SkyMap>,
    /// The face of the moon: its colour, and in alpha how much of it is there.
    pub moon: Option<SkyMap>,
}

/// Maps are read once and held: two are the same only where they are one.
impl PartialEq for SkyMaps {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// What the night adds to the sky in cd/m² before the exposure, as the build's sky program sums it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Night {
    /// The air's glow everywhere, and the glow its map lays over that.
    pub fill: [f32; 3],
    pub glow: [f32; 3],
    /// Each of the galaxy's band, the dense stars and the sparse stars: a colour times its strength, and how it blends towards the star map's axis.
    pub cloud: [f32; 4],
    pub dense: [f32; 4],
    pub sparse: [f32; 4],
    /// How often the dense and the sparse stars tile a face of the star map, and one over the heights the stars and their blinking blend in over, in metres.
    pub scales: [f32; 4],
    /// The blink's gain and bias, the cosine inside which the moon's disc hides the stars, and the blink's phase.
    pub blink: [f32; 4],
    /// The rows of the turn from the frame's axes (x east, y north, z up) to the star map's.
    pub turn: [[f32; 3]; 3],
    pub moon: [f32; 3],
    /// The frame's scale of every point star's size and colour, the power its profile is raised to twice, and one where they are drawn.
    pub point: [f32; 4],
    /// What the air lets through from the camera towards a way whose up part is `(i / (AIR_SAMPLES - 1))²`.
    pub air: [[f32; 4]; AIR_SAMPLES],
    /// The sun's disc: its way and radius at the stars' distance (none drawn at nought), and its colour with how much its limb darkens.
    pub sun_disc: [f32; 4],
    pub sun_colour: [f32; 4],
    /// The moon's disc: the way its light comes from and its radius at the stars' distance (none drawn at nought), and its colour.
    pub moon_disc: [f32; 4],
    pub moon_colour: [f32; 4],
}

/// How many ways up the air's transmittance is stated at.
pub const AIR_SAMPLES: usize = 16;

/// A layer of clouds marched through over the table, its light in the frame's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clouds {
    /// Its base and top in metres above the sea, how much of the sky it covers, and its kind from heaps to towers.
    pub layer: [f32; 4],
    /// The radius in metres of the shell it is curved on, its extinction per metre where whole, the eccentricity of its scattering, and how far in metres the wind has carried it.
    pub shape: [f32; 4],
    /// The way towards the body that lights it, x east, y north, z up.
    pub towards: [f32; 3],
    /// The light of that body where it falls on the layer, and the sky's radiance over the layer, both exposed.
    pub light: [f32; 3],
    pub sky: [f32; 3],
    /// What the air takes out of light per metre between the camera and a cloud.
    pub haze: f32,
    /// The planet's radius in metres, whose sea hides a cloud seen past it.
    pub planet: f32,
}

impl std::fmt::Debug for SkyView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkyView")
            .field("texels", &self.radiance.len())
            .field("exposure", &self.exposure)
            .finish()
    }
}

/// The way texel column `x` and row `y` of the table looks, at its centre: the rows closer together near the horizon, where the sky changes most.
pub fn direction(x: u32, y: u32) -> Vec3 {
    let u = (x as f32 + 0.5) / AROUND as f32;
    let v = (y as f32 + 0.5) / UP as f32;
    let around = (u - 0.5) * std::f32::consts::TAU;
    let up = v * v * std::f32::consts::FRAC_PI_2;
    Vec3::new(up.cos() * around.cos(), up.cos() * around.sin(), up.sin())
}

impl SkyView {
    /// The table `radiance` gives at each of its directions, taken at `exposure`.
    pub fn of(radiance: impl Fn(Vec3) -> [f32; 3], exposure: f32) -> Self {
        let texels = (0..UP)
            .flat_map(|y| (0..AROUND).map(move |x| (x, y)))
            .map(|(x, y)| radiance(direction(x, y)))
            .collect();
        Self {
            radiance: Arc::new(texels),
            exposure,
            clouds: None,
            night: None,
            maps: None,
            stars: None,
        }
    }

    /// The exposed radiance towards `view`, between the four nearest texels; the horizon's under it.
    pub fn sample(&self, view: Vec3) -> Vec3 {
        let view = view.normalize_or(Vec3::Z);
        let u = view.y.atan2(view.x) / std::f32::consts::TAU + 0.5;
        let v = (view.z.clamp(0.0, 1.0).asin() / std::f32::consts::FRAC_PI_2).sqrt();
        let x = u * AROUND as f32 - 0.5;
        let y = (v * UP as f32 - 0.5).clamp(0.0, (UP - 1) as f32);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |x: f32, y: f32| {
            let column = (x as i64).rem_euclid(i64::from(AROUND)) as usize;
            let row = (y as usize).min(UP as usize - 1);
            Vec3::from(self.radiance[row * AROUND as usize + column])
        };
        let low = at(x0, y0).lerp(at(x0 + 1.0, y0), fx);
        let high = at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), fx);
        low.lerp(high, fy) * self.exposure
    }
}

const SHADER: &str = r#"
struct Frame {
    unproject: mat4x4<f32>,
    exposure: vec4<f32>,
    // xyz the camera, w one where clouds are drawn
    eye: vec4<f32>,
    // base, top, cover, kind
    layer: vec4<f32>,
    // shell radius, extinction, eccentricity, drift
    shape: vec4<f32>,
    // xyz towards the light, w the haze per metre
    towards: vec4<f32>,
    light: vec4<f32>,
    sky: vec4<f32>,
    // rgb the air's glow everywhere, w one where the night is drawn
    fill: vec4<f32>,
    glow: vec4<f32>,
    cloud: vec4<f32>,
    dense: vec4<f32>,
    sparse: vec4<f32>,
    scales: vec4<f32>,
    blink: vec4<f32>,
    turn0: vec4<f32>,
    turn1: vec4<f32>,
    turn2: vec4<f32>,
    moon: vec4<f32>,
};
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var table: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;
@group(0) @binding(3) var stars: texture_2d<f32>;
@group(0) @binding(4) var airglow: texture_2d<f32>;
@group(0) @binding(5) var tiled: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) clip: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let clip = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    // At the depth the frame is cleared to, so that only what nothing was drawn at passes.
    return Out(vec4<f32>(clip, 0.0, 1.0), clip);
}

fn hash(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash(i), hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash(i + vec3<f32>(0.0, 1.0, 0.0)), hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash(i + vec3<f32>(0.0, 0.0, 1.0)), hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash(i + vec3<f32>(0.0, 1.0, 1.0)), hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// ⚠ Inferred: a stand-in for the build's weather map (`textures/weathermaps`), whose texels are not read.
fn coverage(q: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var p = vec3<f32>(q / 2500.0, 0.0);
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + noise(p) * amplitude;
        p = p * 2.03 + vec3<f32>(17.0, 31.0, 0.0);
        amplitude = amplitude * 0.5;
    }
    let even = smoothstep(0.25, 0.70, sum / 0.9375);
    let cover = frame.layer.z;
    return smoothstep(1.0 - cover - 0.1, 1.0 - cover + 0.1, even);
}

// The same profile as the title's cloud layer: rising over the first few hundredths, gone where the kind's tops end.
fn profile(along: f32, kind: f32) -> f32 {
    let reach = 0.35 + 0.65 * kind;
    return smoothstep(0.0, 0.07, along) * (1.0 - smoothstep(0.55 * reach, reach, along));
}

// The height above the sea of a point on the shell centred under the camera.
fn height_of(p: vec3<f32>) -> f32 {
    let r = frame.shape.x;
    let centred = vec3<f32>(p.xy - frame.eye.xy, p.z + r);
    return length(centred) - r;
}

// The extinction per metre at `p`, and what the cloud of its column takes out above it.
fn cloud_at(p: vec3<f32>) -> vec2<f32> {
    let thick = frame.layer.y - frame.layer.x;
    let h = height_of(p);
    let along = (h - frame.layer.x) / thick;
    if (along <= 0.0 || along >= 1.0) {
        return vec2<f32>(0.0);
    }
    let q = p.xy + vec2<f32>(frame.shape.w, 0.0);
    let covered = coverage(q);
    let shape = covered * profile(along, frame.layer.w);
    if (shape <= 0.0) {
        return vec2<f32>(0.0);
    }
    let detail = noise(vec3<f32>(q, h) / 350.0) * 0.65 + noise(vec3<f32>(q, h) / 110.0) * 0.35;
    let s = clamp((shape - (1.0 - detail) * 0.45) / 0.55, 0.0, 1.0) * frame.shape.y;
    // ⚠ Inferred: the column above as whole cloud up to where its kind's tops end.
    let reach = (0.35 + 0.65 * frame.layer.w) * 0.8;
    let above = max(reach - along, 0.0) * thick * covered * frame.shape.y;
    return vec2<f32>(s, above);
}

fn density(p: vec3<f32>) -> f32 {
    return cloud_at(p).x;
}

// What a slab of optical depth `tau` lets through of an even light, as the title's cloud layer has it.
fn through_slab(tau: f32) -> f32 {
    return 1.0 / (1.0 + 0.75 * tau * (1.0 - frame.shape.z));
}

// The near and far distances at which the way from the camera meets a sphere of radius `r` about the shell's centre, or none.
// ⚠ The shell is hundreds of kilometres across: every square is taken as a difference of the
// heights, or single floats lose the metres the layer lies in.
fn sphere(view: vec3<f32>, r: f32) -> vec2<f32> {
    let shell = frame.shape.x;
    let height = r - shell;
    let b = (frame.eye.z + shell) * view.z;
    // The camera's distance from the centre squared, less the sphere's radius squared.
    let c = (frame.eye.z - height) * (frame.eye.z + height + 2.0 * shell);
    let d = b * b - c;
    if (d < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    let s = sqrt(d);
    let q = select(-b + s, -b - s, b > 0.0);
    let other = select(c / q, 0.0, q == 0.0);
    return vec2<f32>(min(q, other), max(q, other));
}

// How far the way from the camera goes before it meets the sea on the planet, or a long way where it never does.
fn ground(view: vec3<f32>) -> f32 {
    let planet = frame.sky.w;
    let h = frame.eye.z;
    let b = (h + planet) * view.z;
    let c = h * (h + 2.0 * planet);
    let d = b * b - c;
    if (view.z >= 0.0 || d < 0.0 || planet <= 0.0 || h <= 0.0) {
        return 1e9;
    }
    // The nearer root as c over the farther, which keeps the metres the camera stands at.
    return c / (sqrt(d) - b);
}

fn henyey_greenstein(c: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (12.566371 * pow(max(1.0 + g2 - 2.0 * g * c, 1e-4), 1.5));
}

const STEPS: i32 = 64;
const LIGHT_STEPS: i32 = 6;
// The farthest a cloud is drawn, and the most of the layer one way is marched through, in metres.
const FARTHEST: f32 = 40000.0;
const LONGEST: f32 = 10000.0;

// The sky towards `view` with the clouds in front of it: their light, and what they let through as alpha.
fn clouded(view: vec3<f32>, behind: vec3<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let r = frame.shape.x;
    let top = sphere(view, r + frame.layer.y);
    let base = sphere(view, r + frame.layer.x);
    if (top.y <= 0.0) {
        return vec4<f32>(behind, 1.0);
    }
    var start = max(top.x, 0.0);
    var end = top.y;
    if (frame.eye.z < frame.layer.x) {
        start = base.y;
    } else if (base.x > 0.0) {
        end = min(end, base.x);
    }
    // The shell is curved far tighter than the planet: a way down that meets the sea sees no cloud past it.
    end = min(end, ground(view));
    if (start >= end || start > FARTHEST) {
        return vec4<f32>(behind, 1.0);
    }
    let span = min(end - start, LONGEST);
    let dt = span / f32(STEPS);
    // Each pixel starts its march a random part of a step in, so that the steps do not show as bands.
    let jitter = hash(vec3<f32>(pixel, 7.0));
    let towards = frame.towards.xyz;
    let c = dot(view, towards);
    let g = frame.shape.z;
    var t = start + dt * jitter;
    var through = 1.0;
    var light = vec3<f32>(0.0);
    var weighted = 0.0;
    for (var i = 0; i < STEPS; i = i + 1) {
        let p = frame.eye.xyz + view * t;
        let here = cloud_at(p);
        let s = here.x;
        if (s > 0.0) {
            var tau = 0.0;
            var along = 30.0;
            for (var j = 0; j < LIGHT_STEPS; j = j + 1) {
                let reach = 30.0 * exp2(f32(j));
                tau = tau + density(p + towards * (along + 0.5 * reach)) * reach;
                along = along + reach;
            }
            // ⚠ Inferred: what is scattered more than once as octaves of ever less extinction and eccentricity.
            var sun = 0.0;
            var share = 1.0;
            for (var k = 0; k < 3; k = k + 1) {
                sun = sun + share * exp(-share * tau) * henyey_greenstein(c, share * g);
                share = share * 0.5;
            }
            // The sky reaches in as far as the slab above lets an even light through, and the body's
            // light as much as that slab scatters on down of it, beside what it lets straight through.
            let up = max(towards.z, 0.05);
            let slab = through_slab(here.y);
            let scattered = max(slab - exp(-here.y / up), 0.0);
            let down = frame.sky.rgb * slab + frame.light.rgb * up * 0.31830989 * scattered;
            let lit = frame.light.rgb * sun + down;
            let kept = exp(-s * dt);
            light = light + through * lit * (1.0 - kept);
            weighted = weighted + through * (1.0 - kept) * t;
            through = through * kept;
            if (through < 0.01) {
                break;
            }
        }
        t = t + dt;
    }
    // The air between fades a far cloud into the sky behind it.
    let seen = 1.0 - through;
    let air = exp(-frame.towards.w * weighted / max(seen, 1e-4));
    let cloud = light * air + behind * seen * (1.0 - air);
    return vec4<f32>(behind * through + cloud, through);
}

// Where on the face of a cube the way `d` meets it, as the program works it out: the face's two
// coordinates over twice the major axis with its sign, which no cube lookup takes, plus one half.
fn cube_face(d: vec3<f32>) -> vec2<f32> {
    let a = abs(d);
    var st = vec2<f32>(0.0);
    var major = 1.0;
    if (a.z >= a.y && a.z >= a.x) {
        major = d.z;
        st = vec2<f32>(select(d.x, -d.x, d.z < 0.0), -d.y);
    } else if (a.y >= a.x) {
        major = d.y;
        st = vec2<f32>(d.x, select(d.z, -d.z, d.y < 0.0));
    } else {
        major = d.x;
        st = vec2<f32>(select(-d.z, d.z, d.x < 0.0), -d.y);
    }
    return st / (2.0 * major) + 0.5;
}

// The level of a map `texels` wide laid `tiles` times over a quarter turn, seen by a pixel `angle` radians across.
fn level(texels: f32, tiles: f32, angle: f32) -> f32 {
    return max(log2(texels * tiles / 1.5707963 * angle), 0.0);
}

// What the night adds towards `view` in cd/m², as the build's sky program sums it: the air's glow, then the
// galaxy's band, the dense and the sparse stars of its star map, faded to nothing at the horizon.
fn night_sky(view: vec3<f32>, clip: vec2<f32>, angle: f32) -> vec3<f32> {
    let screen = vec2<f32>(clip.x, -clip.y) * 0.5 + 0.5;
    let width = f32(textureDimensions(stars).x);
    let s = normalize(vec3<f32>(dot(frame.turn0.xyz, view), dot(frame.turn1.xyz, view), dot(frame.turn2.xyz, view)));
    let face = cube_face(s);
    let dense_a = textureSampleLevel(stars, tiled, face * frame.scales.x, level(width, frame.scales.x, angle)).a;
    let sparse_g = textureSampleLevel(stars, tiled, face * frame.scales.y, level(width, frame.scales.y, angle)).g;
    let band = textureSampleLevel(stars, tiled, vec2<f32>(atan2(s.x, s.y) / 3.1415927 + 0.5, s.z * 0.5 + 0.5), 0.0).r;
    // The air's map is laid twice across the screen, as the program lays it.
    var sky = frame.fill.rgb + frame.glow.rgb * textureSampleLevel(airglow, tiled, screen * 2.0, 0.0).r;
    let c = frame.cloud.w * (1.0 - s.z) * band;
    // The blink is whole at the horizon and fades as the way climbs through the blink's height.
    let reach = 1.0 - frame.scales.w * 20000.0 * view.z;
    let phase = 3.1415927 * fract(43759.0 * sin(dot(screen, vec2<f32>(12.99, 78.233)))) + frame.blink.w;
    let b = 1.0 + reach * reach * (sin(phase) - 1.0);
    let dense = frame.dense.rgb * (dense_a * dense_a * b) * c * (frame.dense.w * (1.0 - s.z));
    let gain = (2.0 * b - 1.0) * frame.blink.x + frame.blink.y;
    let sparse = frame.sparse.rgb * (sparse_g * sparse_g * gain) * ((frame.sparse.w - 1.0) * sqrt(max(s.z, 0.0)) + 1.0);
    let height = frame.scales.z * 20000.0 * view.z;
    let seen = select(0.0, 1.0, view.z > 0.0 && dot(view, frame.moon.xyz) <= frame.blink.z);
    sky = sky + (frame.cloud.rgb * c * c * c + dense + sparse) * height * height * seen;
    // ⚠ Inferred: the program's sky target holds no negative light, so a star blinked below nought adds nothing.
    return max(sky, vec3<f32>(0.0));
}

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    let point = frame.unproject * vec4<f32>(in.clip, 1.0, 1.0);
    let view = normalize(point.xyz / point.w);
    let angle = max(length(dpdx(view)), length(dpdy(view)));
    let u = atan2(view.y, view.x) / 6.2831853 + 0.5;
    let v = sqrt(asin(clamp(view.z, 0.0, 1.0)) / 1.5707963);
    var radiance = textureSampleLevel(table, linear, vec2<f32>(u, v), 0.0).rgb * frame.exposure.x;
    if (frame.fill.w > 0.5) {
        radiance = radiance + night_sky(view, in.clip, angle) * frame.exposure.x;
    }
    if (frame.eye.w < 0.5) {
        return vec4<f32>(radiance, 1.0);
    }
    return clouded(view, radiance, in.position.xy);
}
"#;

/// The bytes of the pass's uniforms: the unprojection and eighteen rows of four.
const UNIFORM_BYTES: u64 = 352;

/// The uniforms of a frame looking through `unproject` from `eye` at `view`, as the shader's `Frame` lays them.
fn words(unproject: Mat4, eye: Vec3, view: &SkyView) -> Vec<f32> {
    let mut words: Vec<f32> = unproject.to_cols_array().to_vec();
    words.extend([view.exposure, 0.0, 0.0, 0.0]);
    let drawn = f32::from(u8::from(view.clouds.is_some()));
    words.extend([eye.x, eye.y, eye.z, drawn]);
    let clouds = view.clouds.unwrap_or(Clouds {
        layer: [0.0, 1.0, 0.0, 0.0],
        shape: [1.0, 0.0, 0.0, 0.0],
        towards: [0.0, 0.0, 1.0],
        light: [0.0; 3],
        sky: [0.0; 3],
        haze: 0.0,
        planet: 0.0,
    });
    words.extend(clouds.layer);
    words.extend(clouds.shape);
    words.extend(clouds.towards);
    words.push(clouds.haze);
    words.extend(clouds.light);
    words.push(0.0);
    words.extend(clouds.sky);
    words.push(clouds.planet);
    let night = view.night.unwrap_or_default();
    words.extend(night.fill);
    words.push(f32::from(u8::from(view.night.is_some())));
    words.extend(night.glow);
    words.push(0.0);
    for row in [
        night.cloud,
        night.dense,
        night.sparse,
        night.scales,
        night.blink,
    ] {
        words.extend(row);
    }
    for row in night.turn.into_iter().chain([night.moon]) {
        words.extend(row);
        words.push(0.0);
    }
    words
}

/// A texture of `map`'s levels, or one black texel where there is none.
pub(super) fn map_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    map: Option<&SkyMap>,
) -> wgpu::Texture {
    let black = SkyMap {
        width: 1,
        height: 1,
        srgb: false,
        levels: vec![vec![0; 4]],
    };
    let map = map.filter(|m| !m.levels.is_empty()).unwrap_or(&black);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky-night-map"),
        size: wgpu::Extent3d {
            width: map.width,
            height: map.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: map.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: match map.srgb {
            true => wgpu::TextureFormat::Rgba8UnormSrgb,
            false => wgpu::TextureFormat::Rgba8Unorm,
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, bytes) in map.levels.iter().enumerate() {
        let (width, height) = ((map.width >> level).max(1), (map.height >> level).max(1));
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    texture
}

/// The pass that adds the table's sky over the pixels nothing was drawn at, after the game's dome has laid its stars and discs there, and the clouds in front of all of it.
pub struct SkyViewPass {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    texture: wgpu::Texture,
    samplers: [wgpu::Sampler; 2],
    bind: RefCell<wgpu::BindGroup>,
    /// The table last written, the night's maps last bound, and whether the frame has a table to draw.
    written: Cell<Option<Arc<Vec<[f32; 3]>>>>,
    maps: RefCell<Option<Arc<SkyMaps>>>,
    shown: Cell<bool>,
    stars: StarsPass,
    discs: DiscsPass,
}

/// A one-texel map left as the device made it, all zeros.
fn map_texture_unwritten(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky-night-none"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The pass's bindings: its uniforms, the table, the star map and the air's map, and the two samplers.
fn bind_group(
    device: &wgpu::Device,
    pipeline: &wgpu::RenderPipeline,
    uniforms: &wgpu::Buffer,
    table: &wgpu::Texture,
    [stars, airglow]: [&wgpu::Texture; 2],
    [linear, tiled]: &[wgpu::Sampler; 2],
) -> wgpu::BindGroup {
    let view = |t: &wgpu::Texture| t.create_view(&Default::default());
    let (table, stars, airglow) = (view(table), view(stars), view(airglow));
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky-view"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&table),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(linear),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&stars),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&airglow),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(tiled),
            },
        ],
    })
}

/// The rows of a table in its texture, each a half-float colour and an alpha of one.
fn texels(radiance: &[[f32; 3]]) -> Vec<u8> {
    radiance
        .iter()
        .flat_map(|c| [c[0], c[1], c[2], 1.0])
        .flat_map(|v| or_io::half::f32_to_half(v).to_le_bytes())
        .collect()
}

impl SkyViewPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky-view"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky-view"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_STENCIL_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Equal),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    // What the dome laid (its stars, the discs) shows through as much as the clouds let through.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::SrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky-view"),
            size: UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky-view"),
            size: wgpu::Extent3d {
                width: AROUND,
                height: UP,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky-view"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // The star map tiles and is seen from afar, so it is filtered between its levels.
        let tiled = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky-night"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // Textures start as zeros: one black texel stands for a map not read yet.
        let black = map_texture_unwritten(device);
        let samplers = [sampler, tiled];
        let bind = bind_group(
            device,
            &pipeline,
            &uniforms,
            &texture,
            [&black, &black],
            &samplers,
        );
        Self {
            device: device.clone(),
            pipeline,
            uniforms,
            texture,
            samplers,
            bind: RefCell::new(bind),
            written: Cell::new(None),
            maps: RefCell::new(None),
            shown: Cell::new(false),
            stars: StarsPass::new(device),
            discs: DiscsPass::new(device),
        }
    }

    /// Write this frame's camera and `view`'s exposure, and its table where it is another; none draws nothing.
    pub fn write(&self, queue: &wgpu::Queue, engine: &EngineConstants, view: Option<&SkyView>) {
        self.shown.set(view.is_some());
        let Some(view) = view else {
            return;
        };
        let last = self.written.take();
        if !last
            .as_ref()
            .is_some_and(|l| Arc::ptr_eq(l, &view.radiance))
        {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &texels(&view.radiance),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(AROUND * 8),
                    rows_per_image: Some(UP),
                },
                wgpu::Extent3d {
                    width: AROUND,
                    height: UP,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.written.set(Some(Arc::clone(&view.radiance)));
        let maps_changed = *self.maps.borrow() != view.maps;
        if maps_changed {
            let maps = view.maps.as_deref();
            let stars = map_texture(&self.device, queue, maps.and_then(|m| m.stars.as_ref()));
            let airglow = map_texture(&self.device, queue, maps.and_then(|m| m.airglow.as_ref()));
            *self.bind.borrow_mut() = bind_group(
                &self.device,
                &self.pipeline,
                &self.uniforms,
                &self.texture,
                [&stars, &airglow],
                &self.samplers,
            );
            *self.maps.borrow_mut() = view.maps.clone();
        }
        // The way each pixel looks, with the camera at the centre of the sky.
        let turn = Mat4::from_mat3(glam::Mat3::from_mat4(engine.view));
        let unproject = (engine.projection * turn).inverse();
        let eye = engine.view.inverse().w_axis.truncate();
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::cast_slice(&words(unproject, eye, view)),
        );
        let profile = view.maps.as_deref().and_then(|m| m.star.as_ref());
        self.stars.write(
            queue,
            (engine.projection, engine.view),
            view.night.as_ref(),
            view.exposure,
            (view.stars.as_ref(), profile),
        );
        let face = view.maps.as_deref().and_then(|m| m.moon.as_ref());
        self.discs.write(
            queue,
            (engine.projection, engine.view),
            view.night.as_ref(),
            view.exposure,
            face,
        );
    }

    /// Add the sky into `pass`, where a frame was written with a table: the discs and the point stars first, as the build draws them, so that the clouds in front of the sky hide them.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.shown.get() {
            self.discs.record(pass);
            self.stars.record(pass);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &*self.bind.borrow(), &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_validates_and_its_uniforms_fill_the_frame_it_declares() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("the WGSL parses");
        naga::valid::Validator::new(Default::default(), naga::valid::Capabilities::empty())
            .validate(&module)
            .expect("the WGSL validates");
        let frame = (module.types.iter())
            .find(|(_, t)| t.name.as_deref() == Some("Frame"))
            .map(|(_, t)| t.inner.size(module.to_ctx()))
            .expect("a Frame");
        let mut view = SkyView::of(|_| [1.0; 3], 1.0);
        assert_eq!(
            words(Mat4::IDENTITY, Vec3::ZERO, &view).len() * 4,
            frame as usize
        );
        assert_eq!(u64::from(frame), UNIFORM_BYTES);
        view.clouds = Some(Clouds {
            layer: [200.0, 1500.0, 0.5, 0.5],
            shape: [318_000.0, 0.04, 0.9, 0.0],
            towards: [0.0, 0.0, 1.0],
            light: [1.0; 3],
            sky: [0.5; 3],
            haze: 1e-5,
            planet: 6_360_000.0,
        });
        let drawn = words(Mat4::IDENTITY, Vec3::new(1.0, 2.0, 3.0), &view);
        assert_eq!(&drawn[20..24], &[1.0, 2.0, 3.0, 1.0]);
        assert_eq!(drawn[24], 200.0);
    }

    #[test]
    fn a_table_samples_back_the_radiance_it_was_made_of_towards_each_way() {
        let radiance = |v: Vec3| [v.x + 2.0, v.y + 2.0, v.z];
        let table = SkyView::of(radiance, 2.0);
        for way in [
            Vec3::new(1.0, 0.0, 0.2),
            Vec3::new(-0.3, 0.8, 0.5),
            Vec3::new(0.0, -1.0, 0.05),
            Vec3::Z,
        ] {
            let way = way.normalize();
            let want = Vec3::from(radiance(way)) * 2.0;
            let got = table.sample(way);
            assert!((got - want).length() < 0.08, "{way:?}: {got:?} {want:?}");
        }
        // Under the horizon is the horizon's.
        let under = table.sample(Vec3::new(1.0, 0.0, -0.5));
        assert!((under - table.sample(Vec3::X)).length() < 1e-4);
    }
}
