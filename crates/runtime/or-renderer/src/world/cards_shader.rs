//! The programs of the far stand-ins: the card each record is drawn as, the frames of a species baked from its model, and the halving of the atlas's levels.

/// One camera-facing quad a record, its texels taken from the three frames of the species' octahedral atlas nearest the way it is seen.
pub(super) const CARDS_SHADER: &str = r#"
// Every field of the viewport's `CameraUniform`, in order: a uniform struct is addressed by byte offset.
struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
// A list: its box's least corner and the least scale, its box's size and the most scale, how far it draws and its fade.
struct List {
    low: vec4<f32>,
    size: vec4<f32>,
    reach: vec4<f32>,
};
// A species: its model's bounding centre and radius, the card's width and height over that radius, and its slot.
struct Species {
    centre: vec4<f32>,
    extent: vec2<f32>,
    slot: u32,
    pad: u32,
};
@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<storage, read> records: array<vec4<u32>>;
@group(1) @binding(1) var<storage, read> list_of: array<u32>;
@group(1) @binding(2) var<storage, read> lists: array<List>;
@group(1) @binding(3) var<storage, read> species: array<Species>;
@group(1) @binding(4) var atlas: texture_2d_array<f32>;
@group(1) @binding(5) var atlas_smp: sampler;

const NO_SLOT: u32 = 0xffffffffu;
// A grid of 8 by 8 frames a species, and 2 by 2 species a layer.
const GRID: f32 = 8.0;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    // Where on the quad the pixel lies, in the world.
    @location(0) world_pos: vec3<f32>,
    @location(1) @interpolate(flat) pivot: vec3<f32>,
    @location(2) @interpolate(flat) east: vec3<f32>,
    @location(3) @interpolate(flat) up: vec3<f32>,
    // The half width and half height of the frames in the world, the list's fade, and nothing.
    @location(4) @interpolate(flat) half_size: vec4<f32>,
    @location(5) @interpolate(flat) frames: vec3<u32>,
    @location(6) @interpolate(flat) weights: vec3<f32>,
    @location(7) @interpolate(flat) slot: u32,
    // What moves the card's dither: the place of its record.
    @location(8) @interpolate(flat) seed: u32,
    // Where the way the card is seen along falls on its grid.
    @location(9) @interpolate(flat) grid: vec2<f32>,
};

fn byte_of(word: u32, at: u32) -> f32 {
    return f32((word >> (at * 8u)) & 0xffu);
}

// A record placed: where it stands, its frame turned about its up axis, how large it is, and what it draws with.
struct Card {
    at: vec3<f32>,
    pivot: vec3<f32>,
    east: vec3<f32>,
    north: vec3<f32>,
    up: vec3<f32>,
    half_size: vec2<f32>,
    slot: u32,
    reach: f32,
    fade: f32,
};

fn card_of(instance: u32) -> Card {
    let r = records[instance];
    let list = lists[list_of[instance]];
    var c: Card;
    // Bytes 0..5 are three u16 that place it in the list's box, in steps of 1/65535 of its size.
    let q = vec3<f32>(f32(r.x & 0xffffu), f32(r.x >> 16u), f32(r.y & 0xffffu));
    c.at = list.low.xyz + q / 65535.0 * list.size.xyz;
    // Bytes 6 and 7 are its up axis, octahedral, folded below the equator.
    var o = vec2<f32>(byte_of(r.y, 2u), byte_of(r.y, 3u)) / 127.5 - 1.0;
    let z = 1.0 - abs(o.x) - abs(o.y);
    if (z < 0.0) {
        let signs = select(vec2<f32>(-1.0), vec2<f32>(1.0), o >= vec2<f32>(0.0));
        o = (1.0 - abs(o.yx)) * signs;
    }
    c.up = normalize(vec3<f32>(o, z));
    // Bytes 11 and 12 are its scales across and up, from the least to the most of the list's range.
    let across = mix(list.low.w, list.size.w, byte_of(r.z, 3u) / 255.0);
    let high = mix(list.low.w, list.size.w, byte_of(r.w, 0u) / 255.0);
    // Byte 13 turns it about its up axis, five degrees a step, from east laid square to the up axis.
    let yaw = radians(byte_of(r.w, 1u) * 5.0);
    var tangent = vec3<f32>(1.0, 0.0, 0.0) - c.up * c.up.x;
    if (dot(tangent, tangent) < 1e-8) {
        tangent = vec3<f32>(0.0, 1.0, 0.0) - c.up * c.up.y;
    }
    tangent = normalize(tangent);
    c.east = tangent * cos(yaw) + cross(c.up, tangent) * sin(yaw);
    c.north = cross(c.up, c.east);
    // Bytes 14 and 15 are its species.
    let id = r.w >> 16u;
    c.slot = NO_SLOT;
    if (id < arrayLength(&species)) {
        let s = species[id];
        c.slot = s.slot;
        c.pivot = c.at + (s.centre.x * c.east + s.centre.y * c.north) * across + s.centre.z * c.up * high;
        c.half_size = s.centre.w * s.extent * vec2<f32>(across, high);
    }
    c.reach = list.reach.x;
    c.fade = list.reach.y;
    return c;
}

// The way a frame of the grid was baked from, in the card's own frame: the middle looks down, the edge along the horizon.
fn frame_direction(cell: vec2<u32>) -> vec3<f32> {
    let col = f32(cell.x);
    let row = f32(cell.y);
    let a = (col - row) / 7.0;
    let b = (row + col) / 7.0 - 1.0;
    return normalize(vec3<f32>(a, b, (8.0 / 7.0) * (1.0 - abs(a) - abs(b)) - 1.0 / 7.0));
}

struct Frames {
    at: vec3<u32>,
    weights: vec3<f32>,
    // Where the way falls on the grid, 0 to 7 along each side.
    grid: vec2<f32>,
};

// The three frames of the cell of the grid the way `d` falls in, in the card's own frame, and their weights.
fn frames_of(d: vec3<f32>) -> Frames {
    // A way from below the horizon is seen as along it; the octahedral fold needs no unit vector.
    let v = vec3<f32>(d.x, d.y, max((14.0 / 15.0) * (d.z + 1.0 / 14.0), 0.0));
    let p = v.xy / max(abs(v.x) + abs(v.y) + v.z, 1e-6);
    let g = 3.5 * vec2<f32>(1.0 + p.x + p.y, 1.0 + p.y - p.x);
    let c0 = floor(g);
    let f = g - c0;
    var first = c0;
    var second = c0 + vec2<f32>(1.0, 0.0);
    var third = c0 + vec2<f32>(0.0, 1.0);
    var weights = vec3<f32>(1.0 - f.x - f.y, f.x, f.y);
    if (f.x + f.y >= 1.0) {
        first = c0 + vec2<f32>(1.0, 1.0);
        second = c0 + vec2<f32>(0.0, 1.0);
        third = c0 + vec2<f32>(1.0, 0.0);
        weights = vec3<f32>(f.x + f.y - 1.0, 1.0 - f.x, 1.0 - f.y);
    }
    var out: Frames;
    out.at = vec3<u32>(index_of(first), index_of(second), index_of(third));
    out.weights = weights;
    out.grid = g;
    return out;
}

// The stand-in block's distances, from which its weight scale runs from least to most, and those two scales: the defaults of the game's tunables.
const WEIGHT_FADE: vec2<f32> = vec2<f32>(300.0, 2000.0);
const WEIGHT_SCALE: vec2<f32> = vec2<f32>(1.0, 10.0);

// The game's sharpening of the three frames' weights by the camera's distance `d` from the card: the heaviest gains (2/7)·k·(w − 1/3)·F, with k the weight scale at that distance and F how far the cell lies from the middle of the grid, the middle one loses half that and the lightest keeps what is left, each held in 0 to 1.
fn sharpened(w: vec3<f32>, grid: vec2<f32>, d: f32) -> vec3<f32> {
    let k = mix(WEIGHT_SCALE.x, WEIGHT_SCALE.y, saturate((d - WEIGHT_FADE.x) / (WEIGHT_FADE.y - WEIGHT_FADE.x)));
    var ww = w;
    var heavy = 0u;
    if (ww.y > ww[heavy]) {
        heavy = 1u;
    }
    if (ww.z > ww[heavy]) {
        heavy = 2u;
    }
    var light = (heavy + 1u) % 3u;
    let other = (heavy + 2u) % 3u;
    if (ww[other] < ww[light]) {
        light = other;
    }
    let middle = 3u - heavy - light;
    let far = max(abs(grid.x - 3.5), abs(grid.y - 3.5));
    let gain = (2.0 / 7.0) * k * (ww[heavy] - 1.0 / 3.0) * far;
    let most = saturate(ww[heavy] + gain);
    let mid = saturate(ww[middle] - 0.5 * gain);
    var out = vec3<f32>(0.0);
    out[heavy] = most;
    out[middle] = mid;
    out[light] = saturate(saturate(1.0 - most) - mid);
    return out;
}

fn index_of(cell: vec2<f32>) -> u32 {
    let k = vec2<u32>(clamp(cell, vec2<f32>(0.0), vec2<f32>(7.0)));
    return k.y * 8u + k.x;
}

// What is held back of a card the camera does not draw: all four corners in one place outside the clip volume.
fn hidden() -> VsOut {
    var out: VsOut;
    out.clip_pos = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    out.slot = NO_SLOT;
    return out;
}

// The corner `vertex` of a card seen along `toward`, the unit way from its pivot to the viewer.
fn cornered(c: Card, vertex: u32, toward: vec3<f32>) -> VsOut {
    var out: VsOut;
    // The quad stands in the plane square to the view, upright where it can be.
    var up_q = vec3<f32>(0.0, 0.0, 1.0) - toward * toward.z;
    if (dot(up_q, up_q) < 1e-8) {
        up_q = c.east;
    }
    up_q = normalize(up_q);
    let right_q = cross(up_q, toward);
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u)) * 2.0 - 1.0;
    out.world_pos = c.pivot + right_q * corner.x * c.half_size.x + up_q * corner.y * c.half_size.y;
    out.pivot = c.pivot;
    out.east = c.east;
    out.up = c.up;
    out.half_size = vec4<f32>(c.half_size, c.fade, 0.0);
    out.slot = c.slot;
    let frames = frames_of(vec3<f32>(dot(toward, c.east), dot(toward, c.north), dot(toward, c.up)));
    out.frames = frames.at;
    out.weights = frames.weights;
    out.grid = frames.grid;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VsOut {
    let c = card_of(instance);
    let eye = camera.camera_pos_ws.xyz;
    if (c.slot == NO_SLOT || c.fade <= 0.0 || distance(eye, c.at) > c.reach) {
        return hidden();
    }
    var out = cornered(c, vertex, normalize(eye - c.pivot));
    out.weights = sharpened(out.weights, out.grid, distance(eye, c.pivot));
    out.seed = instance;
    out.clip_pos = camera.view_proj * vec4<f32>(out.world_pos, 1.0);
    return out;
}

// The way the light of a cascade travels: the third row of its view is that way over the depth's span.
fn light_travel() -> vec3<f32> {
    let m = camera.view_proj;
    return normalize(vec3<f32>(m[0][2], m[1][2], m[2][2]));
}

// Into a tile of the map the sun's shadow is cast into: the card faces the sun.
@vertex
fn vs_cast(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VsOut {
    let c = card_of(instance);
    if (c.slot == NO_SLOT || c.fade <= 0.0) {
        return hidden();
    }
    var out = cornered(c, vertex, -light_travel());
    out.seed = instance;
    // ⚠ The card drawn faces the camera and the one cast faces the sun: cast from the far side of the crown, so that no card lies behind its own caster and shades itself, while what stands past the crown is shaded still.
    let behind = light_travel() * max(c.half_size.x, c.half_size.y);
    out.clip_pos = camera.view_proj * vec4<f32>(out.world_pos + behind, 1.0);
    // What stands between the sun and the near side of the tile still casts: it is laid on that side instead of clipped by it.
    out.clip_pos.z = max(out.clip_pos.z, 0.0);
    return out;
}

// The threshold of a pixel in a 4 by 4 ordered dither, the world's own: a fade of one keeps every pixel and a fade of nought none.
fn dither(pixel: vec2<f32>) -> f32 {
    var bayer = array<f32, 16>(
        0.0, 8.0, 2.0, 10.0,
        12.0, 4.0, 14.0, 6.0,
        3.0, 11.0, 1.0, 9.0,
        15.0, 7.0, 13.0, 5.0,
    );
    let at = (u32(pixel.y) & 3u) * 4u + (u32(pixel.x) & 3u);
    return (bayer[at] + 1.0) / 17.0;
}

// One frame's texel where the ray along `ray` through the pixel meets the frame's plane, its coverage weighted by `weight`, and its words.
struct Texel {
    colour: vec4<f32>,
    words: vec3<f32>,
};

fn frame_texel(in: VsOut, ray: vec3<f32>, frame: u32, weight: f32) -> Texel {
    let cell = vec2<u32>(frame & 7u, frame >> 3u);
    let local_d = frame_direction(cell);
    let north = cross(in.up, in.east);
    let d = in.east * local_d.x + north * local_d.y + in.up * local_d.z;
    var right = cross(in.up, d);
    right = select(in.east, normalize(right), dot(right, right) > 1e-8);
    let up = cross(d, right);
    // A ray along the frame's plane meets it nowhere: it is taken as meeting it where it crosses the quad.
    let facing = dot(ray, d);
    let along = select(0.0, dot(in.pivot - in.world_pos, d) / facing, abs(facing) > 1e-6);
    let local = in.world_pos + ray * along - in.pivot;
    let uv = vec2<f32>(
        dot(local, right) / (2.0 * in.half_size.x) + 0.5,
        0.5 - dot(local, up) / (2.0 * in.half_size.y),
    );
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    let quadrant = vec2<f32>(f32(in.slot & 1u), f32((in.slot >> 1u) & 1u));
    let at = (quadrant + (vec2<f32>(cell) + clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))) / GRID) * 0.5;
    // The atlas holds the share of each texel the model covers, at every level: a far card is as airy as its crown.
    // Each slot's colours lie in an even layer, its words in the odd one after it.
    let layer = (in.slot >> 2u) * 2u;
    let texel = textureSample(atlas, atlas_smp, at, layer);
    let words = textureSample(atlas, atlas_smp, at, layer + 1u);
    var out: Texel;
    out.colour = vec4<f32>(texel.rgb, texel.a * weight * select(0.0, 1.0, inside));
    out.words = words.rgb;
    return out;
}

// The three frames blended by their weights: the linear colour and the coverage, and the words.
struct Blend {
    colour: vec4<f32>,
    words: vec3<f32>,
};

// ⚠ Every frame is sampled before anything is discarded.
fn blended(in: VsOut, ray: vec3<f32>) -> Blend {
    let a = frame_texel(in, ray, in.frames.x, in.weights.x);
    let b = frame_texel(in, ray, in.frames.y, in.weights.y);
    let c = frame_texel(in, ray, in.frames.z, in.weights.z);
    let wa = a.colour.a;
    let wb = b.colour.a;
    let wc = c.colour.a;
    let coverage = wa + wb + wc;
    var out: Blend;
    out.colour = vec4<f32>((a.colour.rgb * wa + b.colour.rgb * wb + c.colour.rgb * wc) / max(coverage, 1e-4), coverage);
    out.words = (a.words * wa + b.words * wb + c.words * wc) / max(coverage, 1e-4);
    return out;
}

// A pixel is kept as often as the share the model covers there, by the list's fade; each card's dither is moved by its own place, so that cards one behind another cover as two crowns would.
fn dropped(in: VsOut, coverage: f32) -> bool {
    let moved = vec2<f32>(f32(in.seed & 3u), f32((in.seed >> 2u) & 3u));
    return coverage * in.half_size.z < dither(in.clip_pos.xy + moved);
}

// A card has no normal map: its normal is that of the half of its crown's sphere turned to the viewer, square to the view at the rim.
fn normal_of(in: VsOut) -> vec3<f32> {
    let toward = normalize(camera.camera_pos_ws.xyz - in.pivot);
    let o = (in.world_pos - in.pivot) / max(in.half_size.x, in.half_size.y);
    return normalize(o + toward * sqrt(max(1.0 - dot(o, o), 0.0)));
}

@fragment
fn fs_forward(in: VsOut) -> @location(0) vec4<f32> {
    let s = blended(in, in.world_pos - camera.camera_pos_ws.xyz).colour;
    if (dropped(in, s.a)) {
        discard;
    }
    let n = normal_of(in);
    let view_dir = normalize(camera.camera_pos_ws.xyz - in.world_pos);
    let key = max(dot(n, camera.key_light.xyz), 0.0) * camera.key_light.w;
    let head = max(dot(n, view_dir), 0.0) * 0.30;
    return vec4<f32>(s.rgb * (key + head + 0.15), 1.0);
}

// The four targets of the game's G-buffer, in the words its lighting passes read, as the world's cutout writes them.
struct GBufferOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) ambient: vec4<f32>,
};

const PI: f32 = 3.14159265;
const SPECULAR_EXPONENT_MAX: f32 = 512.0;

// The world's: the Blinn-Phong exponent whose lobe is as wide as the rough lobe of this smoothness.
fn exponent_of(smoothness: f32) -> f32 {
    let alpha = max(0.001, (1.0 - smoothness) * (1.0 - smoothness));
    return clamp(2.0 / (alpha * alpha) - 2.0, 1.0, 8196.0);
}

// The world's: Lazarov's fit of the split sum, what a GGX lobe takes of the reflectance at normal incidence and at grazing.
fn split_sum(rough: f32, n_dot_v: f32) -> vec2<f32> {
    let r = rough * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return max(vec2<f32>(-1.04, 1.04) * a004 + r.zw, vec2<f32>(0.0));
}

// The world's share of the light about it a grey surface of reflectance `f0` reflects to the eye.
fn reflected(f0: f32, smoothness: f32, n_dot_v: f32) -> f32 {
    let f90 = clamp(16.5 * 3.0 * f0, 0.0, 1.0);
    let ab = split_sum(1.0 - smoothness, n_dot_v);
    let once = f0 * ab.x + f90 * ab.y;
    return once * (1.0 + f0 * (1.0 / max(ab.x + ab.y, 1e-4) - 1.0));
}

// The specular target as the world's G-buffer writes it for a surface of the baked smoothness and reflectance, seen along the card's normal: the intensity and the fresnel word chosen so that the lighting's reflection is the share `reflected` gives.
fn specular_words(words: vec3<f32>, n: vec3<f32>, world_pos: vec3<f32>) -> vec4<f32> {
    let smoothness = clamp(words.x, 0.0, 1.0);
    let f0 = clamp(words.y, 0.0, 1.0);
    let e = exponent_of(smoothness);
    let stored = select(500.0 + (e - 1500.0) / 558.0, e / 3.0, e <= 1500.0);
    let n_dot_v = clamp(dot(n, normalize(camera.camera_pos_ws.xyz - world_pos)), 0.0, 1.0);
    let share = clamp(reflected(f0, smoothness, max(n_dot_v, 1e-4)) / mix(1.0 / PI, 1.0, clamp(e / 563.0, 0.0, 1.0)), 0.0, 1.0);
    let grazing = pow(1.0 - n_dot_v, 5.0);
    let intensity = clamp(f0 + (share - f0) / max(grazing, 1e-6), f0, 1.0);
    return vec4<f32>(sqrt(intensity), sqrt(stored / SPECULAR_EXPONENT_MAX), 1.0 - f0 / max(intensity, 1e-6), 1.0);
}

// ponytail: no forward lobe of the sun through the leaves, which the world adds as a leaf's own light: the card program binds no light; bind it if crowns against a low sun show the lack.
@fragment
fn fs_gbuffer(in: VsOut) -> GBufferOut {
    let b = blended(in, in.world_pos - camera.camera_pos_ws.xyz);
    let s = b.colour;
    if (dropped(in, s.a)) {
        discard;
    }
    let n = normal_of(in);
    var out: GBufferOut;
    // The lighting pass squares the colour it reads here, so the target holds the square root of the linear colour.
    out.albedo = vec4<f32>(sqrt(max(s.rgb, vec3<f32>(0.0))), 1.0);
    out.normal = vec4<f32>(n * 0.5 + 0.5, 0.0);
    out.specular = specular_words(b.words, n, in.world_pos);
    // Outdoors, no light of its own, and the backlit pass's flag of the leaves most of the pixel is
    out.ambient = vec4<f32>(1.0, 0.0, 0.0, select(0.5, 1.0, b.words.z > 0.75));
    return out;
}

// A card casts the shadow of what its frames cover, seen along the light.
@fragment
fn fs_cast(in: VsOut) {
    let s = blended(in, light_travel()).colour;
    if (dropped(in, s.a)) {
        discard;
    }
}
"#;

/// A species' model drawn into one frame of its atlas by the world's own program, its frame's view in the camera's place at a dynamic offset: these entry points follow it.
pub(super) fn bake_shader() -> String {
    format!("{}{BAKE_ENTRIES}", super::super::shader::WORLD_SHADER)
}

const BAKE_ENTRIES: &str = r#"
// The albedo the world's G-buffer takes of the surface, its palette, its second map and its metal worked as the near model's are, and no light: a card is lit where it stands. Beside it the words its reflection is worked from: its smoothness, its reflectance at normal incidence, and the backlit pass's flag as the world's G-buffer writes it.
struct Baked {
    @location(0) colour: vec4<f32>,
    @location(1) words: vec4<f32>,
};

fn baked(s: Surface) -> Baked {
    var out: Baked;
    out.colour = vec4<f32>(diffuse_of(s), 1.0);
    let flag = select(1.0, select(0.5, 1.0, s.translucency > 0.5), (m.flags & FOLIAGE) != 0u);
    out.words = vec4<f32>(s.smoothness, max3(s.f0), flag, 1.0);
    return out;
}

@fragment
fn fs_bake(in: VsOut, @builtin(front_facing) front_facing: bool) -> Baked {
    return baked(surface_of(in, front_facing));
}

// ⚠ Cut by the colour map's first level, as the world cuts: a frame 32 texels wide samples a level that has averaged the leaves away.
@fragment
fn fs_bake_cutout(in: VsOut, @builtin(front_facing) front_facing: bool) -> Baked {
    let s = surface_of(in, front_facing);
    if (dropped(in, cut_coverage(in), CUTOUT_ALPHA)) {
        discard;
    }
    return baked(s);
}
"#;

/// One level of the atlas from the level above it: each texel the mean of four, its colour weighted by their coverage so that an empty texel darkens nothing.
pub(super) const MIP_SHADER: &str = r#"
@group(0) @binding(0) var above: texture_2d<f32>;

@vertex
fn vs_full(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let at = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    return vec4<f32>(at * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_half(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let base = vec2<i32>(pixel.xy) * 2;
    var colour = vec3<f32>(0.0);
    var coverage = 0.0;
    for (var k = 0; k < 4; k++) {
        let texel = textureLoad(above, base + vec2<i32>(k & 1, k >> 1), 0);
        colour += texel.rgb * texel.a;
        coverage += texel.a;
    }
    return vec4<f32>(colour / max(coverage, 1e-4), coverage * 0.25);
}
"#;
