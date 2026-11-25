//! The program the world is drawn with: one vertex stage for a view and one for a cascade of the sun's shadow, the arithmetic of each family of material, and a fragment stage for each bucket in each kind of pass.

pub(super) const WORLD_SHADER: &str = r#"
// Every field of the viewport's `CameraUniform`, in order: a uniform struct is addressed by byte offset.
struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos_ws: vec4<f32>,
    camera_misc: vec4<f32>,
    key_light: vec4<f32>,
};
// A lamp that lights what blends, as `WorldLamp`'s rows: its place and reach, its colour times its strength and a spot's outer cosine, and a spot's way and one over its span, or a capsule's half segment and a span below nought.
struct Lamp {
    at: vec4<f32>,
    colour: vec4<f32>,
    way: vec4<f32>,
};
// The light of the game's frame, in the units its lighting passes sum, in `frame.x` how far the night emissives are on, in `frame.y` the exposure a material's own light is multiplied by, in `frame.z` how many lamps light what blends and in `frame.w` the seconds the world has run, which the wind moves by.
struct Light {
    sun_travel: vec4<f32>,
    sun_colour: vec4<f32>,
    ambient_down: vec4<f32>,
    ambient_up: vec4<f32>,
    frame: vec4<f32>,
    lamps: array<Lamp, 256>,
    // The wind as `WorldWind`'s rows: the way it blows, its base speed and its most speed, then each gust's centre, strength and half-width.
    wind: vec4<f32>,
    gusts: array<vec4<f32>, 2>,
};
// A material's constants, laid out as `WorldConstants` after a word of the family.
struct Constants {
    family: vec4<u32>,
    fresnel: vec4<f32>,
    bumpiness: vec4<f32>,
    height_scale: vec4<f32>,
    height_bias: vec4<f32>,
    blend_softness: vec4<f32>,
    tint_intensity: vec4<f32>,
    tint_blend: vec4<f32>,
    vertex_tint_scale: vec4<f32>,
    vertex_tint_amount: vec4<f32>,
    control_filter: vec4<f32>,
    opacity: vec4<f32>,
    emissive_scale: vec4<f32>,
    emissive_power: vec4<f32>,
    uv0_transform: vec4<f32>,
    uv1_transform: vec4<f32>,
    uv_sets: vec4<f32>,
    uv_extra: vec4<f32>,
    interior_room: vec4<f32>,
    interior_plane: vec4<f32>,
    interior_glass: vec4<f32>,
    interior_slant: vec4<f32>,
    flags: u32,
    palette_row: f32,
    translucency_as_tint_mask: f32,
    specular_fresnel: f32,
    layer0_tint: f32,
    layer1_tint: f32,
    control_softness: f32,
    control_offset: f32,
    control_entity: f32,
    dirt_intensity: f32,
    grass_tint: f32,
    emissive_multiplier: f32,
    emissive_night: f32,
    emissive_view_power: f32,
    emissive_diffuse_tint: f32,
    grass_bend: f32,
};
@group(0) @binding(0) var<uniform> camera: Camera;
// Slots 9 to 11 are the fourth layer's in the four-layer family, and material B of the first and second layer and the emissive map in every other.
@group(1) @binding(0) var layer0_colour: texture_2d<f32>;
@group(1) @binding(1) var layer0_normal: texture_2d<f32>;
@group(1) @binding(2) var layer0_a: texture_2d<f32>;
@group(1) @binding(3) var layer1_colour: texture_2d<f32>;
@group(1) @binding(4) var layer1_normal: texture_2d<f32>;
@group(1) @binding(5) var layer1_a: texture_2d<f32>;
@group(1) @binding(6) var layer2_colour: texture_2d<f32>;
@group(1) @binding(7) var layer2_normal: texture_2d<f32>;
@group(1) @binding(8) var layer2_a: texture_2d<f32>;
@group(1) @binding(9) var layer3_colour: texture_2d<f32>;
@group(1) @binding(10) var layer3_normal: texture_2d<f32>;
@group(1) @binding(11) var layer3_a: texture_2d<f32>;
@group(1) @binding(12) var weights_map: texture_2d<f32>;
@group(1) @binding(13) var tint_map: texture_2d<f32>;
@group(1) @binding(14) var second_map: texture_2d<f32>;
@group(1) @binding(15) var map_smp: sampler;
@group(1) @binding(16) var<uniform> m: Constants;
@group(2) @binding(0) var<uniform> light: Light;

const STANDARD: u32 = 0u;
const TWO_LAYER: u32 = 1u;
const FOUR_LAYER: u32 = 2u;
const GLASS: u32 = 4u;
const EMISSIVE_FAMILY: u32 = 5u;
const GRASS: u32 = 6u;
const TERRAIN_LOD: u32 = 8u;
const VERTEX_COLOURED: u32 = 9u;

const VERTEX_TINT: u32 = 1u;
const MASK_FROM_ALPHA: u32 = 2u;
const INVERT_CONTROL: u32 = 4u;
const HARD_LIGHT: u32 = 8u;
const MODEL_VERTEX: u32 = 16u;
const INSTANCE_COLOUR: u32 = 32u;
const GROUND_BLEND: u32 = 64u;
const TINT_MULTIPLY: u32 = 128u;
const FOLIAGE: u32 = 256u;
const VERTEX_OVER_TINT: u32 = 512u;
const TINT_MASKED: u32 = 1024u;
const INTERIOR: u32 = 2048u;
const ROOM_BY_VERTEX: u32 = 4096u;
const GLOWS: u32 = 8192u;
const VEIL: u32 = 16384u;
const VEIL_BARES: u32 = 32768u;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal:   vec3<f32>,
    @location(2) uv:       vec2<f32>,
    // A model's vertex as `coloured` packs it: the second texture coordinate, then the first colour's four bytes and the third coordinate's two halves.
    @location(3) uv1:      vec2<f32>,
    @location(9) packed:   vec2<u32>,
    // The vertex's second colour, four bytes where the viewport's vertex keeps its depth bias.
    @location(4) colour1:  u32,
    @location(5) row0:     vec4<f32>,
    @location(6) row1:     vec4<f32>,
    @location(7) row2:     vec4<f32>,
    @location(8) tint:     vec4<f32>,
    @location(10) ambient: vec2<f32>,
    @location(11) cast_only: f32,
};

// ⚠ A surface's own light is drawn a second time over the lit frame, tested against the depth its first draw wrote: both draws must place it exactly alike.
struct VsOut {
    @builtin(position) @invariant clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
    // How much of the instance its fade leaves, which is the alpha of its tint and no alpha of the surface.
    @location(4) @interpolate(flat) fade: f32,
    @location(5) colour1: vec4<f32>,
    @location(6) @interpolate(flat) tint: vec3<f32>,
    @location(7) uv1: vec2<f32>,
    @location(8) uv2: vec2<f32>,
    // How much of the natural and of the artificial ambient light the instance takes.
    @location(9) @interpolate(flat) ambient: vec2<f32>,
    // An interior effect's third colour and fourth texture coordinate, which size its rooms.
    @location(10) colour2: vec4<f32>,
    @location(11) uv3: vec2<f32>,
};

// A normal stored as two signed halves of an octahedron.
fn unpacked_normal(bits: u32) -> vec3<f32> {
    let e = unpack2x16snorm(bits);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let sign = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((1.0 - abs(n.yx)) * sign, n.z);
    }
    return normalize(n);
}

fn placed(in: VsIn) -> vec3<f32> {
    let local = vec4<f32>(in.position, 1.0);
    return vec3<f32>(dot(in.row0, local), dot(in.row1, local), dot(in.row2, local));
}

// An offset wrapped into the square of half-width `half` about nought, as a gust's place is.
fn wrapped(offset: vec2<f32>, half: f32) -> vec2<f32> {
    return offset - 2.0 * half * round(offset / (2.0 * half));
}

// The wind's speed at `at`: the base speed, and each gust's strength by a fall-off linear over its half-width. ponytail: the fall-off is a stand-in, the game's gust shape was not read.
fn wind_speed_at(at: vec2<f32>) -> f32 {
    var speed = light.wind.z;
    for (var k = 0u; k < 2u; k++) {
        let gust = light.gusts[k];
        if (gust.z > 0.0 && gust.w > 0.0) {
            speed += gust.z * max(1.0 - length(wrapped(at - gust.xy, gust.w)) / gust.w, 0.0);
        }
    }
    return speed;
}

// ponytail: the wind is the game's, its way, its speed and its gusts; how a blade answers it is a stand-in. The game bends a blade by its material's `bendiness` towards global wind volumes it fills at run time, which no file holds; here a blade leans down the wind by its height above its clump's foot, the foot still, as far as the wind's speed over its most, with a flutter of each clump. Read the volumes' writer if a frame of the game's grass is to be matched.
fn swayed(in: VsIn, world: vec3<f32>) -> vec3<f32> {
    if (m.family.x != GRASS || m.grass_bend <= 0.0 || light.wind.w <= 0.0) {
        return world;
    }
    let foot = vec3<f32>(in.row0.w, in.row1.w, in.row2.w);
    let push = max(wind_speed_at(foot.xy), 0.0) / light.wind.w;
    let high = max(world.z - foot.z, 0.0);
    let flutter = 0.15 * sin(4.7 * light.frame.w + dot(foot.xy, vec2<f32>(1.7, 2.3)));
    let lean = m.grass_bend * push * (1.0 + flutter) * high * high / (high + 0.3);
    return world + vec3<f32>(light.wind.xy * lean, -0.5 * lean * lean / max(high, 0.05));
}

// What both vertex stages hand on, but for where the vertex is drawn.
fn handed_on(in: VsIn, world: vec3<f32>) -> VsOut {
    var out: VsOut;
    var normal = in.normal;
    out.colour2 = vec4<f32>(0.0);
    out.uv3 = vec2<f32>(0.0);
    if ((m.flags & INTERIOR) != 0u) {
        normal = unpacked_normal(bitcast<u32>(in.normal.x));
        out.colour2 = unpack4x8unorm(bitcast<u32>(in.normal.y));
        out.uv3 = unpack2x16float(bitcast<u32>(in.normal.z));
    }
    // The rows turn the normal as they turn the surface, which is right for a uniform scale; a non-uniform one shears it slightly.
    out.world_normal = normalize(vec3<f32>(
        dot(in.row0.xyz, normal),
        dot(in.row1.xyz, normal),
        dot(in.row2.xyz, normal),
    ));
    out.uv = in.uv;
    if ((m.flags & MODEL_VERTEX) != 0u) {
        out.colour = unpack4x8unorm(in.packed.x);
        out.uv1 = in.uv1;
        out.uv2 = unpack2x16float(in.packed.y);
    } else {
        // A mesh the viewer builds keeps its colour's four floats as they are, and has one texture coordinate.
        out.colour = vec4<f32>(in.uv1, bitcast<f32>(in.packed.x), bitcast<f32>(in.packed.y));
        out.uv1 = in.uv;
        out.uv2 = in.uv;
    }
    out.colour1 = unpack4x8unorm(in.colour1);
    out.world_pos = world;
    out.fade = in.tint.a;
    out.tint = in.tint.rgb;
    out.ambient = in.ambient;
    return out;
}

// ⚠ A cascade's cast binds no light, so grass casts unswayed.
@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world = swayed(in, placed(in));
    var out = handed_on(in, world);
    out.clip_pos = camera.view_proj * vec4<f32>(world, 1.0);
    // One that only casts its shadow is put past the far plane, where every triangle of it is clipped.
    if (in.cast_only > 0.5) {
        out.clip_pos = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

// Into a tile of the map the sun's shadow is cast into: the camera's uniform holds the view of the cascade.
@vertex
fn vs_cast(in: VsIn) -> VsOut {
    let world = placed(in);
    var out = handed_on(in, world);
    out.clip_pos = camera.view_proj * vec4<f32>(world, 1.0);
    // What stands between the sun and the near side of the tile still casts: it is laid on that side instead of clipped by it.
    out.clip_pos.z = max(out.clip_pos.z, 0.0);
    return out;
}

// The threshold of a pixel in a 4 by 4 ordered dither: sixteen steps from 1/17 to 16/17, so a fade of one keeps every pixel and a fade of nought none.
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

// Whether nothing of the pixel is drawn: its coverage is under `cut`, or the fade of its instance leaves it out.
// ⚠ A fade below nought gives way to children fading in: it keeps the pixels their dither leaves, or the two leave holes.
fn dropped(in: VsOut, alpha: f32, cut: f32) -> bool {
    let t = dither(in.clip_pos.xy);
    let faded = select(in.fade < t, -in.fade < 1.0 - t, in.fade < 0.0);
    return alpha < cut || faded;
}

// Below this a texel of a cutout is not drawn.
const CUTOUT_ALPHA: f32 = 0.5;

// What the vertex's first colour leaves of a surface's coverage: its alpha, but none of a tree's or grass's, whose programs cut by their colour map alone.
// ⚠ A tree's leaves hold other data in that alpha: read as coverage, it cuts every leaf of a near tree away.
fn vertex_coverage(in: VsOut) -> f32 {
    return select(in.colour.a, 1.0, (m.flags & FOLIAGE) != 0u);
}

// The coverage a cutout is cut by: its colour map's first level, whatever the screen needs.
// ⚠ A smaller level averages a mask of thin leaves and blades under the cut: a tree a hundred metres off would lose every leaf.
fn cut_coverage(in: VsOut) -> f32 {
    let top = textureSampleLevel(layer0_colour, map_smp, in.uv, 0.0).a * vertex_coverage(in);
    return select(top, 1.0, m.family.x == FOUR_LAYER);
}
// Below this a texel of what blends adds nothing.
const BLEND_ALPHA: f32 = 1.0 / 255.0;
// The most the G-buffer's word for a surface's own light holds, and the most its exponent word holds.
const EMISSIVE_MAX: f32 = 16.0;
const SPECULAR_EXPONENT_MAX: f32 = 512.0;
const PI: f32 = 3.14159265;
// The albedo below which a dielectric's reflectance fades to none, and the span over which it reaches the whole: the frame's two linear words.
const REFLECTANCE_LOW: f32 = 0.0015;
const REFLECTANCE_SPAN: f32 = 0.0055;

// What a material gives a pixel: its linear albedo before its metalness, its coverage, its unit normal in the world, its metalness and smoothness, its reflectance at normal incidence, and the light it gives.
struct Surface {
    albedo: vec3<f32>,
    coverage: f32,
    normal: vec3<f32>,
    metal: f32,
    smoothness: f32,
    f0: vec3<f32>,
    emissive: vec3<f32>,
    // How much sun a leaf lets through, nought for any other surface.
    translucency: f32,
};

// Every map of the material, each sampled once.
struct Maps {
    c0: vec4<f32>, n0: vec4<f32>, a0: vec4<f32>,
    c1: vec4<f32>, n1: vec4<f32>, a1: vec4<f32>,
    c2: vec4<f32>, n2: vec4<f32>, a2: vec4<f32>,
    c3: vec4<f32>, n3: vec4<f32>, a3: vec4<f32>,
    weights: vec4<f32>,
    tint: vec4<f32>,
    second: vec4<f32>,
};

fn lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2125, 0.7154, 0.0721));
}

fn max3(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// How much of a dielectric's reflectance a surface of this albedo keeps.
fn reach(albedo: vec3<f32>) -> f32 {
    return clamp((max3(albedo) - REFLECTANCE_LOW) / REFLECTANCE_SPAN, 0.0, 1.0);
}

// A map of two channels as a normal in the frame of the texture, its lean scaled by `bump`: z is worked out from x and y.
fn leaning(texel: vec4<f32>, bump: f32) -> vec3<f32> {
    let xy = texel.xy * 2.0 - 1.0;
    let z = max(sqrt(max(1.0 - dot(xy, xy), 0.0)), 0.01);
    return vec3<f32>(xy * bump, z);
}

// The light laid over `base` as hard light.
fn hard_light(base: vec3<f32>, over: vec3<f32>) -> vec3<f32> {
    let screen = 1.0 - 2.0 * (1.0 - base) * (1.0 - over);
    return select(screen, 2.0 * base * over, over < vec3<f32>(0.5));
}

// The colours of a row of a palette, as the game's mesh stage counts them whatever the width of the image it lies in.
const PALETTE_ROW: u32 = 256u;

// The texel of the palette the vertex's first colour picks in the material's row: a row past the last is the last, a column past the row's end runs on into the next, as in the game's one-dimensional image, and one past the image's last colour reads none, which leaves the tint white.
fn palette(colour0: vec4<f32>) -> vec3<f32> {
    let size = textureDimensions(tint_map);
    let width = max(size.x, 1u);
    let held = width * max(size.y, 1u);
    let rows = max(held / PALETTE_ROW, 1u);
    let row = min(u32(max(m.palette_row, 0.0)), rows - 1u);
    // ⚠ The column is the third byte of the first colour as stored: a far prop list's meshes hold small whole numbers there, within their palette's length, and in the first byte numbers past it.
    let at = row * PALETTE_ROW + u32(255.0 * colour0.b + 0.5);
    let texel = vec2<u32>(at % width, at / width);
    return select(vec3<f32>(1.0), textureLoad(tint_map, texel, 0).rgb, at < held);
}

// A four-layer layer's colour: its vertex tint, then its palette tint, the second unless the vertex's second colour turns it off.
fn layer_colour(d: vec3<f32>, mask: f32, scale: f32, amount: f32, intensity: f32, blend: f32, colour1: vec4<f32>, tint: vec3<f32>) -> vec3<f32> {
    var v = d;
    if ((m.flags & VERTEX_TINT) != 0u) {
        v = mix(d, scale * srgb_to_linear(colour1.rgb) * lum(d), amount * mask);
    }
    let t = intensity * tint * lum(v);
    return mix(mix(v, t, blend * mask), v, colour1.a);
}

// The four-layer family: the layers' weights from the lookup map and the vertex, sharpened by each layer's height.
fn four_layer(s: ptr<function, Surface>, maps: Maps, in: VsOut) -> vec3<f32> {
    let c0 = in.colour;
    let lookup = maps.weights.rgb;
    let rest = mix(1.0 - lookup.r - lookup.g - lookup.b, 1.0 - c0.r - c0.g - c0.b, c0.a);
    let w = max(vec4<f32>(mix(lookup, c0.rgb, c0.a), rest), vec4<f32>(0.0));
    let height = vec4<f32>(maps.a0.b, maps.a1.b, maps.a2.b, maps.a3.b);
    let h = max(vec4<f32>(0.01), m.height_scale * (height - m.height_bias)) * w;
    let highest = max(max(h.x, h.y), max(h.z, h.w));
    let sharp = 20.0 - 33.33 * (m.blend_softness - 0.2);
    let raised = max(pow(h / max(highest, 1e-6), sharp) - 0.001, vec4<f32>(0.0));
    let a = select(raised, vec4<f32>(0.0), w <= vec4<f32>(0.0));
    let total = a.x + a.y + a.z + a.w;
    let b = select(vec4<f32>(1.0, 0.0, 0.0, 0.0), a / max(total, 1e-6), total > 0.0);
    let alphas = vec4<f32>(maps.a0.a, maps.a1.a, maps.a2.a, maps.a3.a);
    let mask = select(1.0 - alphas, alphas, (m.flags & MASK_FROM_ALPHA) != 0u);
    let c1 = in.colour1;
    let tint = maps.tint.rgb;
    let colour = b.x * layer_colour(maps.c0.rgb, mask.x, m.vertex_tint_scale.x, m.vertex_tint_amount.x, m.tint_intensity.x, m.tint_blend.x, c1, tint)
        + b.y * layer_colour(maps.c1.rgb, mask.y, m.vertex_tint_scale.y, m.vertex_tint_amount.y, m.tint_intensity.y, m.tint_blend.y, c1, tint)
        + b.z * layer_colour(maps.c2.rgb, mask.z, m.vertex_tint_scale.z, m.vertex_tint_amount.z, m.tint_intensity.z, m.tint_blend.z, c1, tint)
        + b.w * layer_colour(maps.c3.rgb, mask.w, m.vertex_tint_scale.w, m.vertex_tint_amount.w, m.tint_intensity.w, m.tint_blend.w, c1, tint);
    (*s).albedo = colour;
    (*s).coverage = 1.0;
    (*s).metal = 0.0;
    let rough = clamp(dot(b, vec4<f32>(maps.a0.g, maps.a1.g, maps.a2.g, maps.a3.g)), 0.0, 1.0);
    (*s).smoothness = 1.0 - rough;
    (*s).f0 = vec3<f32>(reach(colour) * dot(b, 0.04 * (1.0 + m.fresnel)));
    let blended = b.x * leaning(maps.n0, m.bumpiness.x) + b.y * leaning(maps.n1, m.bumpiness.y)
        + b.z * leaning(maps.n2, m.bumpiness.z) + b.w * leaning(maps.n3, m.bumpiness.w);
    // The tint normal map leans the blended normal further.
    let over = maps.second.xy * 2.0 - 1.0;
    return vec3<f32>(blended.xy + over, blended.z);
}

// The two-layer family: the second layer over the first by the control map and the vertex's second colour, then the dirt.
fn two_layer(s: ptr<function, Surface>, maps: Maps, in: VsOut, tint: vec3<f32>) -> vec3<f32> {
    var c = dot(maps.weights, m.control_filter);
    if ((m.flags & INVERT_CONTROL) != 0u) {
        c = 1.0 - c;
    }
    let x = clamp(in.colour1.x * (1.0 + c), 0.0, 1.0);
    let soft = max(m.control_softness, 1e-3);
    let shown = clamp(x / max(0.01, mix(1.0 / (soft * soft), 1.0, x)), 0.0, 1.0);
    let w1 = clamp(shown * m.control_entity + m.control_offset, 0.0, 1.0);
    let m0 = 1.0 - m.translucency_as_tint_mask * (1.0 - maps.c3.a);
    let m1 = 1.0 - m.translucency_as_tint_mask * (1.0 - maps.n3.a);
    let l0 = maps.c0.rgb * (1.0 + m.layer0_tint * m0 * (tint - 1.0));
    let l1 = maps.c1.rgb * (1.0 + m.layer1_tint * m1 * (tint - 1.0));
    var colour = mix(l0, l1, w1);
    let dirt = clamp(maps.second.a * in.colour.a * m.dirt_intensity, 0.0, 1.0);
    let laid = select(maps.second.rgb, hard_light(colour, maps.second.rgb), (m.flags & HARD_LIGHT) != 0u);
    colour = mix(colour, laid, dirt);
    (*s).albedo = colour;
    (*s).coverage = maps.c0.a * in.colour.a;
    (*s).metal = mix(maps.a0.r, maps.a1.r, w1);
    (*s).smoothness = 1.0 - clamp(mix(maps.a0.g, maps.a1.g, w1), 0.0, 1.0);
    return mix(leaning(maps.n0, m.bumpiness.x), leaning(maps.n1, m.bumpiness.y), w1);
}

// How much of a ground decal shows: the height its material map states, taken between its low and high, against the vertex's share, the larger winning the more sharply the greater its softness.
fn ground_share(height: f32, share: f32) -> f32 {
    let h = clamp(m.height_scale.x * (height - m.height_bias.x), 1e-4, 0.9999);
    let shown = h * share;
    let hidden = (1.0 - h) * (1.0 - share);
    let top = max(max(shown, hidden), 1e-6);
    let p = 19.0 * m.blend_softness.x + 1.0;
    let a = pow(shown / top, p);
    let b = pow(hidden / top, p);
    return clamp(a / max(a + b, 1e-6), 0.0, 1.0);
}

// The one-layer families: material A is metalness in red and roughness in green, material B's alpha masks the palette tint.
fn one_layer(s: ptr<function, Surface>, maps: Maps, in: VsOut, tint: vec3<f32>) -> vec3<f32> {
    let family = m.family.x;
    var colour = maps.c0.rgb;
    if (family == VERTEX_COLOURED) {
        colour = colour * in.colour.rgb;
    } else if (family == GRASS) {
        // The instance's colour, squared already, as far as the vertex's second colour lets it
        let own = 1.0 + in.colour1.r * (in.tint - 1.0);
        colour = colour * (1.0 + m.grass_tint * (own - 1.0));
    } else if ((m.flags & GROUND_BLEND) != 0u) {
        // The palette's colour at the luminance of the decal's own, or times it, as far as its tint blend and, where its effect says, its material A's alpha say
        let tinted = select(m.tint_intensity.x * tint * lum(colour), clamp(colour * tint * m.tint_intensity.x, vec3<f32>(0.0), vec3<f32>(1.0)), (m.flags & TINT_MULTIPLY) != 0u);
        colour = mix(colour, tinted, m.tint_blend.x * select(1.0, maps.a0.a, (m.flags & TINT_MASKED) != 0u));
    } else if (family != TERRAIN_LOD) {
        let mask = 1.0 - m.translucency_as_tint_mask * (1.0 - maps.c3.a);
        colour = colour * (1.0 + mask * (tint - 1.0));
        // The second colour map over the first by its own alpha
        colour = mix(colour, maps.second.rgb, maps.second.a);
    }
    (*s).albedo = colour;
    (*s).coverage = maps.c0.a * vertex_coverage(in);
    if ((m.flags & GROUND_BLEND) != 0u) {
        (*s).coverage = maps.c0.a * ground_share(maps.a0.b, in.colour.a);
    }
    if (family == TERRAIN_LOD) {
        // Its material map's green is a smoothness, and its normal map holds all three channels
        (*s).metal = 0.0;
        (*s).smoothness = clamp(maps.a0.g, 0.0, 1.0);
        return maps.n0.xyz * 2.0 - 1.0;
    }
    // ⚠ A tree's material A red is one less its translucency, which the lighting's translucent material stands for: read as a metalness it turns leaves into mirrors.
    (*s).metal = select(clamp(maps.a0.r, 0.0, 1.0), 0.0, (m.flags & FOLIAGE) != 0u);
    // The lighting reads the translucency in sixteen steps, the high four bits of its byte
    let stored = u32(255.0 * clamp(1.0 - maps.a0.r, 0.0, 1.0) + 0.5);
    (*s).translucency = select(0.0, f32(stored >> 4u) / 15.0, (m.flags & FOLIAGE) != 0u);
    (*s).smoothness = 1.0 - clamp(maps.a0.g, 0.0, 1.0);
    return leaning(maps.n0, m.bumpiness.x);
}

// The texel a way picks of a cube whose six faces lie stacked down one image in the order a pack stores them, at the level `rooms` rooms to a pixel ask for.
fn cube_stacked(t: texture_2d<f32>, dir: vec3<f32>, rooms: f32) -> vec4<f32> {
    let a = abs(dir);
    var face = 0.0;
    var st = vec2<f32>(0.0);
    var ma = a.x;
    if (a.x >= a.y && a.x >= a.z) {
        face = select(1.0, 0.0, dir.x >= 0.0);
        st = vec2<f32>(select(dir.z, -dir.z, dir.x >= 0.0), -dir.y);
    } else if (a.y >= a.z) {
        ma = a.y;
        face = select(3.0, 2.0, dir.y >= 0.0);
        st = vec2<f32>(dir.x, select(-dir.z, dir.z, dir.y >= 0.0));
    } else {
        ma = a.z;
        face = select(5.0, 4.0, dir.z >= 0.0);
        st = vec2<f32>(select(-dir.x, dir.x, dir.z >= 0.0), -dir.y);
    }
    // A face of the cube is the wall of one room
    let side = f32(textureDimensions(t, 0).x);
    let lod = log2(max(rooms * side, 1.0));
    // ⚠ A texel at a face's edge would be filtered with the next face down the image: the coordinate is held half a texel of the level inside its face.
    let half = 0.5 / max(side * exp2(-lod), 1.0);
    let uv = clamp(st / max(ma, 1e-6) * 0.5 + 0.5, vec2<f32>(half), vec2<f32>(1.0 - half));
    return textureSampleLevel(t, map_smp, vec2<f32>(uv.x, (face + uv.y) / 6.0), lod);
}

// Where `x`, along an axis from the middle of `n` rooms each `size` long, lies in its room, from -1 to 1.
fn in_room(x: f32, size: f32, n: f32) -> f32 {
    let odd = (u32(n) & 1u) == 1u;
    return 2.0 * fract(x / size + select(0.0, 0.5, odd)) - 1.0;
}

// What the interior effect sees through a window: the colour behind the glass, darkened by how deep it is seen, and the room's colour and light undarkened.
struct Room {
    seen: vec3<f32>,
    colour: vec3<f32>,
    light: vec3<f32>,
};

// The darkest a room is seen, at the depth past which it darkens no more.
const ROOM_DARK: f32 = 0.025;

// The room behind the glass by the interior effect's program: the panel the vertex's third coordinate spans is a box of rooms behind it, sized by the vertex's third colour and fourth coordinate or by the constants; the view leaves the box through the wall of a room, whose cube, turned by the vertex's quarter turns, gives that room's colour and light; and a back plane hangs a share of a room's depth behind the glass. `across` and `up` are the frame of the panel's coordinate, `rooms` how many rooms a pixel spans.
fn room_behind(in: VsOut, across: vec3<f32>, up: vec3<f32>, rooms: f32) -> Room {
    var room: Room;
    let n = normalize(in.world_normal);
    let by_vertex = (m.flags & ROOM_BY_VERTEX) != 0u;
    let packed = u32(round(255.0 * in.colour2.x));
    let depth = select(m.interior_room.x, 5.0 * in.colour2.y, by_vertex);
    let width = select(m.interior_room.y, 1.0 - in.uv3.y, by_vertex);
    let height = select(m.interior_room.y * m.interior_room.z, width * in.uv3.x, by_vertex);
    let deep = select(1.0 + round(31.875 * in.colour2.x), f32((packed >> 2u) + 1u), by_vertex);
    let turns = select(round(3.0 * in.colour2.y), f32(packed & 3u), by_vertex);
    let wide = max(round(20.0 * in.colour2.w), 1.0);
    let tall = max(round(m.interior_room.w), 1.0);
    let each = round(m.interior_plane.x);
    let cells = select(max(1.0, each + wide - 1.0), wide * each, each >= 1.0);
    // The box of the panel's rooms, along the normal, across and up, its middle behind the middle of the panel.
    let dims = vec3<f32>(depth * width * deep, width * wide, tall * height);
    let uv = in.uv2;
    let middle = in.world_pos + (0.5 - uv.x) * dims.y * across + (0.5 - uv.y) * width * up - 0.5 * dims.x * n;
    let off = in.world_pos - middle;
    let o = vec3<f32>(dot(n, off), dot(across, off), dot(up, off)) / dims;
    let ray = normalize(in.world_pos - camera.camera_pos_ws.xyz);
    let d = vec3<f32>(dot(n, ray), dot(across, ray), dot(up, ray)) / dims;
    // Seen from inside the box, nothing is behind the glass.
    if (d.x >= 0.0) {
        return room;
    }
    // The way out is the nearest of the far faces along each axis.
    let way = select(vec3<f32>(-1.0), vec3<f32>(1.0), d >= vec3<f32>(0.0));
    let safe = select(d, way * 1e-6, abs(d) < vec3<f32>(1e-6));
    let far = (0.5 * way - o) / safe;
    let t = min(far.x, min(far.y, far.z));
    let q = o + t * d;
    let x = q * dims;
    var p = vec3<f32>(in_room(x.x, dims.x / deep, deep), in_room(x.y, dims.y / cells, cells), in_room(x.z, height, tall));
    if (t == far.x) {
        p.x = way.x;
    } else if (t == far.y) {
        p.y = way.y;
    } else {
        p.z = way.z;
    }
    // A quarter turn of the cube about the up axis for each of the vertex's turns.
    let angle = turns * 0.5 * PI;
    let c = cos(angle);
    let s = sin(angle);
    let dir = vec3<f32>(c * p.x - s * p.y, s * p.x + c * p.y, p.z);
    let colour = cube_stacked(layer2_colour, dir, rooms).rgb;
    let light = cube_stacked(layer2_normal, dir, rooms).rgb;
    let exit = middle + x.x * n + x.y * across + x.z * up;
    let near = clamp(1.0 - distance(exit, in.world_pos) / m.interior_plane.w, 0.0, 1.0);
    // The back plane, where the view meets it inside the panel and the vertex's third colour states one.
    let behind = m.interior_plane.y * depth * width;
    let hit = in.world_pos + ray * (behind / max(-dot(n, ray), 1e-4)) - (middle + (0.5 * dims.x - behind) * n);
    let at = vec2<f32>(dot(across, hit) / dims.y, dot(up, hit) / dims.z);
    let inside = all(abs(at) < vec2<f32>(0.5)) && round(255.0 * in.colour2.z) != 255.0;
    let plane_uv = vec2<f32>(fract(cells * (0.5 + at.x)), fract(tall * (0.5 - at.y)));
    let plane_side = f32(textureDimensions(layer2_a, 0).x);
    let plane = textureSampleLevel(layer2_a, map_smp, plane_uv, log2(max(rooms * plane_side, 1.0)));
    let shown = clamp(m.interior_plane.z * select(0.0, plane.a, inside), 0.0, 1.0);
    let plane_near = clamp(1.0 - m.interior_plane.y * width / m.interior_plane.w, 0.0, 1.0);
    room.seen = (1.0 - shown) * colour * mix(ROOM_DARK, 1.0, near * near) + shown * plane.rgb * mix(ROOM_DARK, 1.0, plane_near * plane_near);
    room.colour = (1.0 - shown) * colour + shown * plane.rgb;
    // ⚠ The back plane gives light of its own as its alpha nears one: about its share where its alpha is low, steeply more past nine tenths.
    let alpha = select(0.0, plane.a, inside);
    let glows = (1.156e-5 * exp2(22.96 * alpha) + 0.96616) * shown;
    room.light = (1.0 - shown) * light + m.interior_slant.z * glows;
    return room;
}

// The strength of a material's own light at this hour: its night value where it states one, by how far the night emissives are on, at the day's exposure.
fn emissive_strength() -> f32 {
    return light.frame.y * select(mix(m.emissive_multiplier, m.emissive_night, light.frame.x), m.emissive_multiplier, m.emissive_night < 0.0);
}

// The texture coordinate of set 1, 2 or 3.
fn coordinate_set(in: VsOut, which: f32) -> vec2<f32> {
    return select(select(in.uv2, in.uv1, which == 2.0), in.uv, which == 1.0);
}

// Where a layer's maps are sampled: its set's coordinate, scaled and moved by that set's transform; the second layer of a two-layer material is scaled again.
fn layer_uv(in: VsOut, layer: u32) -> vec2<f32> {
    let first = m.uv_sets[layer] == 1.0;
    let t = select(m.uv1_transform, m.uv0_transform, first);
    let uv = select(in.uv1, in.uv, first) * t.xy + t.zw;
    return select(uv, uv * m.uv_extra.zw, layer == 1u);
}

// ⚠ Every map is sampled and every derivative taken before anything is discarded, and in control flow every pixel shares.
// ponytail: every slot is sampled whatever the family, fifteen samples a pixel, most of them of a one-texel stand-in; sample by family where the G-buffer pass's cost shows it.
fn surface_of(in: VsOut, front_facing: bool) -> Surface {
    var maps: Maps;
    let uv0 = layer_uv(in, 0u);
    let uv1 = layer_uv(in, 1u);
    let uv2 = layer_uv(in, 2u);
    let uv3 = layer_uv(in, 3u);
    maps.c0 = textureSample(layer0_colour, map_smp, uv0);
    maps.n0 = textureSample(layer0_normal, map_smp, uv0);
    maps.a0 = textureSample(layer0_a, map_smp, uv0);
    maps.c1 = textureSample(layer1_colour, map_smp, uv1);
    maps.n1 = textureSample(layer1_normal, map_smp, uv1);
    maps.a1 = textureSample(layer1_a, map_smp, uv1);
    maps.c2 = textureSample(layer2_colour, map_smp, uv2);
    maps.n2 = textureSample(layer2_normal, map_smp, uv2);
    maps.a2 = textureSample(layer2_a, map_smp, uv2);
    // Material B of the first layers is sampled where its layer is, and the emissive map by the first coordinate
    maps.c3 = textureSample(layer3_colour, map_smp, select(uv0, uv3, m.family.x == FOUR_LAYER));
    maps.n3 = textureSample(layer3_normal, map_smp, select(uv1, uv3, m.family.x == FOUR_LAYER));
    maps.a3 = textureSample(layer3_a, map_smp, select(in.uv, uv3, m.family.x == FOUR_LAYER));
    maps.weights = textureSample(weights_map, map_smp, coordinate_set(in, m.uv_extra.x));
    maps.tint = textureSample(tint_map, map_smp, in.uv);
    maps.second = textureSample(second_map, map_smp, in.uv);
    // Culling is off, so a face seen from behind is shaded by the normal turned towards the viewer.
    let n = normalize(in.world_normal) * select(-1.0, 1.0, front_facing);
    // No vertex carries a tangent: the frame is how the position and the texture coordinate change across the pixel, the map's x along u and its y along v.
    let dp1 = dpdx(in.world_pos);
    let dp2 = dpdy(in.world_pos);
    let duv1 = dpdx(in.uv);
    let duv2 = dpdy(in.uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    // ⚠ Which way round the two changes of position turn about the normal: a frame whose rows run down the screen turns them the other way, and without this both axes come out reversed.
    let handed = sign(dot(dp1, dp2perp));
    let tangent = (dp2perp * duv1.x + dp1perp * duv2.x) * handed;
    let bitangent = (dp2perp * duv1.y + dp1perp * duv2.y) * handed;
    let longest = max(dot(tangent, tangent), dot(bitangent, bitangent));
    let scale = inverseSqrt(max(longest, 1e-30));
    // The interior effect's panel runs along the third coordinate, which has a frame of its own.
    let dst1 = dpdx(in.uv2);
    let dst2 = dpdy(in.uv2);
    let across = (dp2perp * dst1.x + dp1perp * dst2.x) * handed;
    let up = (dp2perp * dst1.y + dp1perp * dst2.y) * handed;
    let panel_step = max(length(dst1), length(dst2));

    let family = m.family.x;
    var s: Surface;
    s.emissive = vec3<f32>(0.0);
    s.translucency = 0.0;
    var lean: vec3<f32>;
    if (family == FOUR_LAYER) {
        lean = four_layer(&s, maps, in);
        s.albedo = s.albedo * in.tint;
    } else {
        let tint = palette(in.colour);
        if (family == TWO_LAYER) {
            lean = two_layer(&s, maps, in, tint);
        } else {
            lean = one_layer(&s, maps, in, tint);
        }
        // A game model's instance tint colours only a region-tint material, which is its colour, or the vertex's colour over it as far as the vertex's alpha: a prop list's tint is no part of a mapped material's colour.
        if ((m.flags & INSTANCE_COLOUR) != 0u) {
            let over = (m.flags & VERTEX_OVER_TINT) != 0u;
            s.albedo = s.albedo * mix(in.tint, in.colour.rgb, select(0.0, in.colour.a, over));
            if (over) {
                s.coverage = maps.c0.a;
            }
        } else if (family != GRASS && (m.flags & MODEL_VERTEX) == 0u) {
            s.albedo = s.albedo * in.tint;
        }
        let dielectric = 0.04 * (1.0 + m.specular_fresnel) * reach(s.albedo * (1.0 - s.metal));
        s.f0 = mix(vec3<f32>(dielectric), s.albedo, s.metal);
        if (family == TERRAIN_LOD) {
            s.f0 = vec3<f32>(0.04 * reach(s.albedo));
        }
    }
    s.normal = normalize(tangent * (lean.x * scale) + bitangent * (lean.y * scale) + n * lean.z);
    if (family != FOUR_LAYER) {
        // The emissive map, by the vertex's second colour, its own scale and power, the view and how much of the surface's colour it takes
        var e = emissive_strength() * pow(max(m.emissive_scale.rgb * maps.a3.rgb * in.colour1.rgb, vec3<f32>(0.0)), m.emissive_power.rgb);
        if (m.emissive_view_power > 0.0) {
            let view_dir = normalize(camera.camera_pos_ws.xyz - in.world_pos);
            e = e * pow(max(dot(s.normal, view_dir), 0.0), m.emissive_view_power);
        }
        s.emissive = e * (1.0 + m.emissive_diffuse_tint * (s.albedo - 1.0));
    }
    if ((m.flags & INTERIOR) != 0u) {
        let wide = max(round(20.0 * in.colour2.w), 1.0);
        behind_glass(&s, maps, in, normalize(across), normalize(up), panel_step * wide);
    }
    return s;
}

// The interior effect's window: how much of the room the two layers' alphas let be seen, the glass's absorption by how far the view crosses it, the room through it over the layers as the glass's open share, and the room's light through the glass by its emissive map at this hour.
fn behind_glass(s: ptr<function, Surface>, maps: Maps, in: VsOut, across: vec3<f32>, up: vec3<f32>, rooms: f32) {
    let room = room_behind(in, across, up, rooms);
    // A pane the first layer covers takes the veil's way where the material states one: its room shows whatever the curtain layer holds, where the effect's words allow it, and lets through only a share of its light.
    let veiled = maps.c0.a > 0.05 && (m.flags & VEIL) != 0u;
    let curtain = pow(clamp(1.0 - 1.186 * (maps.c1.a - 0.15686), 0.0, 1.0), 3.0);
    let shows = clamp(1.0 - maps.c0.a, 0.0, 1.0) * select(curtain, 1.0, veiled && (m.flags & VEIL_BARES) != 0u);
    let let_through = select(1.0, m.interior_slant.w * 0.1 * (1.0 - clamp(1.186 * (maps.c0.a - 0.15686), 0.0, 1.0)), veiled);
    let n = normalize(in.world_normal);
    let facing = max(abs(dot(n, normalize(camera.camera_pos_ws.xyz - in.world_pos))), 1e-4);
    let thin = m.interior_glass.w;
    let thick = max(0.0, m.interior_slant.y * (min(m.interior_slant.x, thin / facing) - thin) + thin);
    let through = exp(-2.0 * m.interior_glass.rgb * thick);
    let layer = (*s).albedo;
    (*s).albedo = mix(layer, through * mix(layer, room.seen, shows), 1.0 - maps.c1.a);
    let own = clamp(1.2439 * (maps.c0.a - 0.19607), 0.0, 1.0);
    (*s).emissive = emissive_strength() * maps.a3.rgb * through * room.light * mix(room.colour, layer, own) * shows * let_through;
}

// The colour the diffuse light multiplies: the albedo less what its metal turns to reflection.
fn diffuse_of(s: Surface) -> vec3<f32> {
    return s.albedo * (1.0 - s.metal);
}

// The Blinn-Phong exponent whose lobe is as wide as the rough lobe of this smoothness.
fn exponent_of(smoothness: f32) -> f32 {
    let alpha = max(0.001, (1.0 - smoothness) * (1.0 - smoothness));
    return clamp(2.0 / (alpha * alpha) - 2.0, 1.0, 8196.0);
}

// The share of the light about it a GGX lobe of this roughness reflects to an eye `n_dot_v` off its normal, as what it takes of the reflectance at normal incidence and of that at grazing: Lazarov's fit of the split sum.
fn split_sum(rough: f32, n_dot_v: f32) -> vec2<f32> {
    let r = rough * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return max(vec2<f32>(-1.04, 1.04) * a004 + r.zw, vec2<f32>(0.0));
}

// The share of the light about it a surface reflects towards the eye: the reflectance at grazing is fifty times the mean reflectance, up to one, the split sum weighs both by how rough the surface is and how it is seen, and the light it scatters more than once is given back.
fn reflected(s: Surface, n_dot_v: f32) -> f32 {
    let f0 = max3(s.f0);
    let f90 = clamp(16.5 * (s.f0.r + s.f0.g + s.f0.b), 0.0, 1.0);
    let ab = split_sum(1.0 - s.smoothness, n_dot_v);
    let once = f0 * ab.x + f90 * ab.y;
    return once * (1.0 + f0 * (1.0 / max(ab.x + ab.y, 1e-4) - 1.0));
}

// The highlight a light towards `towards` leaves on the surface seen from `view_dir`.
fn shine(s: Surface, towards: vec3<f32>, view_dir: vec3<f32>) -> vec3<f32> {
    // A light straight behind the viewer's line has no half way: the sum is then of no length, and is left so
    let sum = towards + view_dir;
    let half_way = sum * inverseSqrt(max(dot(sum, sum), 1e-12));
    let lit = max(dot(s.normal, towards), 0.0);
    let n = exponent_of(s.smoothness);
    return s.f0 * lit * (2.0 + n) / 8.0 * pow(max(dot(s.normal, half_way), 0.0), n);
}

// The key light, the head light and the ambient light of the viewport's material mode.
fn plain_lit(s: Surface, world_pos: vec3<f32>) -> vec3<f32> {
    let view_dir = normalize(camera.camera_pos_ws.xyz - world_pos);
    let key = max(dot(s.normal, camera.key_light.xyz), 0.0) * camera.key_light.w;
    let head = max(dot(s.normal, view_dir), 0.0) * 0.30;
    let highlight = shine(s, camera.key_light.xyz, view_dir) * camera.key_light.w;
    return diffuse_of(s) * (key + head + 0.15) + highlight + s.emissive;
}

// A surface of the plain modes that writes depth: `cut` is the coverage under which it is not drawn, and `glow` how far it shows its own colour unlit.
fn forward(in: VsOut, front_facing: bool, cut: f32, glow: f32) -> vec4<f32> {
    let s = surface_of(in, front_facing);
    if (dropped(in, select(s.coverage, cut_coverage(in), cut >= CUTOUT_ALPHA), cut)) {
        discard;
    }
    return vec4<f32>(mix(plain_lit(s, in.world_pos), diffuse_of(s), glow), 1.0);
}

@fragment
fn fs_forward(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return forward(in, front_facing, 0.0, 0.0);
}

@fragment
fn fs_forward_cutout(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return forward(in, front_facing, CUTOUT_ALPHA, 0.0);
}

@fragment
fn fs_forward_emissive(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return forward(in, front_facing, 0.0, 1.0);
}

// What blends in the plain modes, a decal or a glass: its coverage is the alpha it is blended by.
@fragment
fn fs_forward_blend(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = surface_of(in, front_facing);
    let alpha = s.coverage * select(1.0, m.opacity.x, m.family.x != FOUR_LAYER);
    if (dropped(in, alpha, BLEND_ALPHA)) {
        discard;
    }
    return vec4<f32>(plain_lit(s, in.world_pos), alpha);
}

// The four targets of the game's G-buffer, in the words its lighting passes read.
struct GBufferOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) specular: vec4<f32>,
    @location(3) ambient: vec4<f32>,
};

fn words(s: Surface, world_pos: vec3<f32>, ambient: vec2<f32>) -> GBufferOut {
    var out: GBufferOut;
    let diffuse = diffuse_of(s);
    // The lighting pass squares the colour it reads here, so the target holds the square root of the linear colour.
    out.albedo = vec4<f32>(sqrt(max(diffuse, vec3<f32>(0.0))), 1.0);
    // The normal in the world, from -1..1 into 0..1; the default effect leaves the alpha at nought for every normal but one straight up.
    out.normal = vec4<f32>(s.normal * 0.5 + 0.5, 0.0);
    // The exponent is stored as the lighting expands it, a third below 1500 and a 558th above.
    let n = exponent_of(s.smoothness);
    let stored = select(500.0 + (n - 1500.0) / 558.0, n / 3.0, n <= 1500.0);
    // ⚠ The lighting weighs the reflection by `intensity * (f0' + (1 - f0') * (1 - n.v)^5)` whatever the roughness, `f0' = 1 - fresnel`, and divides a rough lobe's by pi: the intensity and the fresnel word are chosen here, where n.v is known, so that this is the share `reflected` gives and the reflectance at normal incidence is the surface's. The highlights of the lights then reach `intensity` at grazing.
    let n_dot_v = clamp(dot(s.normal, normalize(camera.camera_pos_ws.xyz - world_pos)), 0.0, 1.0);
    let f0 = max3(s.f0);
    let share = clamp(reflected(s, max(n_dot_v, 1e-4)) / mix(1.0 / PI, 1.0, clamp(n / 563.0, 0.0, 1.0)), 0.0, 1.0);
    let grazing = pow(1.0 - n_dot_v, 5.0);
    let intensity = clamp(f0 + (share - f0) / max(grazing, 1e-6), f0, 1.0);
    out.specular = vec4<f32>(sqrt(intensity), sqrt(stored / SPECULAR_EXPONENT_MAX), 1.0 - f0 / max(intensity, 1e-6), 1.0);
    // The instance's share of the natural ambient light in red and of the artificial in green, as roots since the lighting squares what it reads there, and in blue the surface's own light as a multiple of its colour, which the lighting pass adds.
    let glow = min(lum(s.emissive) / max(lum(diffuse), 1.0 / 255.0), EMISSIVE_MAX);
    // ⚠ This alpha is a flag over a half and the root of the reflection's intensity in twice what is left: the backlit pass tints the sun through a leaf half way to white where the flag is set, a quarter where it is not, and the leaves that let half the sun through or more take the half.
    let tinted = select(1.0, select(0.5, 1.0, s.translucency > 0.5), (m.flags & FOLIAGE) != 0u);
    out.ambient = vec4<f32>(sqrt(ambient), glow / EMISSIVE_MAX, tinted);
    return out;
}

// GTA VI's sun through a leaf that lets `amount` of it through, seen `cos_t` off the sun's travel, past the even share the frame's backlit pass already gives: the forward lobe of a Henyey-Greenstein phase of 0.9 times the amount's upper half, as a multiple of the sun on the leaf's colour.
fn forward_scattered(amount: f32, cos_t: f32) -> f32 {
    let s = clamp(amount, 0.0, 1.0);
    let s2 = clamp(2.0 * s, 0.0, 1.0);
    let e = exp((0.0914 * s2 * s2 - 0.09618) / max(0.2 * s, 1e-4));
    let through = 0.25 * e * (e * e + 3.0);
    let lean = clamp(2.0 * s - 1.0, 0.0, 1.0);
    let g = 0.9 * lean;
    let phase = (1.0 - g * g) / pow(max(1.0 + g * g - 2.0 * g * cos_t, 1e-4), 1.5);
    return through * (1.0 - 0.82838 * lean) * max(phase - 1.0, 0.0);
}

fn gbuffer(in: VsOut, front_facing: bool, cut: f32) -> GBufferOut {
    var s = surface_of(in, front_facing);
    if (dropped(in, select(s.coverage, cut_coverage(in), cut >= CUTOUT_ALPHA), cut)) {
        discard;
    }
    // Its own light is added over the lit frame by `glow`, whole.
    if ((m.flags & GLOWS) != 0u) {
        s.emissive = vec3<f32>(0.0);
    }
    // ponytail: the forward lobe is the surface's own light, so no shadow falls on it; a pass of the viewer's own over the trees' stencil, after the sun's shadow, if leaves in shade show it.
    if (s.translucency > 0.0) {
        let view_dir = normalize(camera.camera_pos_ws.xyz - in.world_pos);
        let lobe = forward_scattered(s.translucency, dot(light.sun_travel.xyz, view_dir));
        s.emissive = s.emissive + diffuse_of(s) * light.sun_colour.rgb * lobe;
    }
    return words(s, in.world_pos, in.ambient);
}

@fragment
fn fs_gbuffer(in: VsOut, @builtin(front_facing) front_facing: bool) -> GBufferOut {
    return gbuffer(in, front_facing, 0.0);
}

@fragment
fn fs_gbuffer_cutout(in: VsOut, @builtin(front_facing) front_facing: bool) -> GBufferOut {
    return gbuffer(in, front_facing, CUTOUT_ALPHA);
}

@fragment
fn fs_gbuffer_emissive(in: VsOut, @builtin(front_facing) front_facing: bool) -> GBufferOut {
    return gbuffer(in, front_facing, 0.0);
}

// The light a material of `GLOWS` gives, added over the frame the passes lit, as the game adds its emissive target: the G-buffer's word holds at most a few times the surface's colour, and a lit room behind dark glass gives far more.
fn glow(in: VsOut, front_facing: bool, cut: f32) -> vec4<f32> {
    let s = surface_of(in, front_facing);
    if (dropped(in, select(s.coverage, cut_coverage(in), cut >= CUTOUT_ALPHA), cut)) {
        discard;
    }
    return vec4<f32>(s.emissive, 0.0);
}

@fragment
fn fs_glow(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return glow(in, front_facing, 0.0);
}

@fragment
fn fs_glow_cutout(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return glow(in, front_facing, CUTOUT_ALPHA);
}

// A decal is blended onto what the G-buffer holds: each target it writes takes its coverage times its own opacity as the alpha it is blended by, and the pipeline keeps the alpha that was there.
@fragment
fn fs_gbuffer_decal(in: VsOut, @builtin(front_facing) front_facing: bool) -> GBufferOut {
    let s = surface_of(in, front_facing);
    if (dropped(in, s.coverage * max3(m.opacity.xyz), BLEND_ALPHA)) {
        discard;
    }
    var out = words(s, in.world_pos, in.ambient);
    out.albedo.a = m.opacity.x * s.coverage;
    out.normal.a = m.opacity.y * s.coverage;
    out.specular.a = m.opacity.z * s.coverage;
    return out;
}

// The light the lamps give a surface: by GTA VI's law, the window `(1 - (d²/R²)²)²` and a spot's angular factor across its cones over the distance squared, by the cosine to its normal (`facing`); and as the glass program sums every light again, with no cosine and `I / 4π` at the most (`round`).
struct LampLight {
    facing: vec3<f32>,
    round: vec3<f32>,
}

// ⚠ The glass program's second sum takes `s² / (d² + s²)` of a light by a source radius its record was read to hold; the record holds none, and a metre stands in for it.
const LAMP_SOURCE_SQUARED: f32 = 1.0;

fn lamps_lit(s: Surface, world_pos: vec3<f32>) -> LampLight {
    var sum = LampLight(vec3<f32>(0.0), vec3<f32>(0.0));
    let count = u32(light.frame.z);
    for (var i = 0u; i < count; i = i + 1u) {
        let l = light.lamps[i];
        var at = l.at.xyz;
        // A capsule's way is half its segment, and its span below nought: it lights from the nearest point of the segment
        let capsule = l.way.w < 0.0;
        if (capsule) {
            let along = clamp(dot(world_pos - at, l.way.xyz) / max(dot(l.way.xyz, l.way.xyz), 1e-12), -1.0, 1.0);
            at = at + l.way.xyz * along;
        }
        let to = at - world_pos;
        // A centimetre at the least: the glass of a lamp lies a hand's breadth from it, never on it.
        let d2 = max(dot(to, to), 1e-4);
        let t = d2 / (l.at.w * l.at.w);
        if (t >= 1.0) {
            continue;
        }
        let w = 1.0 - t * t;
        let towards = to * inverseSqrt(d2);
        var a = 1.0;
        if (!capsule && dot(l.way.xyz, l.way.xyz) > 0.0) {
            let x = clamp((dot(l.way.xyz, -towards) - l.colour.w) * l.way.w, 0.0, 1.0);
            a = x * (x - x * x + x * x * x * x * (3.0 - 2.0 * x * x));
        }
        let lit = l.colour.rgb * (w * w * a);
        sum.facing = sum.facing + lit / d2 * max(dot(s.normal, towards), 0.0);
        sum.round = sum.round + lit * (LAMP_SOURCE_SQUARED / (d2 + LAMP_SOURCE_SQUARED) / (4.0 * PI));
    }
    return sum;
}

// What blends over the frame the game's passes lit, lit by the same sun and ambient light and by the nearest lamps, which the passes cannot reach here, premultiplied by its coverage; a forward emissive adds its own light instead.
@fragment
fn fs_lit(in: VsOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = surface_of(in, front_facing);
    if (m.family.x == EMISSIVE_FAMILY) {
        // The vertex's first colour is a palette column, not a multiplier: only its alpha covers
        let texel = textureSample(layer0_colour, map_smp, in.uv);
        let alpha = texel.a * in.colour.a * m.opacity.x;
        if (dropped(in, max(alpha, max3(texel.rgb)), BLEND_ALPHA)) {
            discard;
        }
        return vec4<f32>(texel.rgb * emissive_strength() * m.opacity.x, alpha);
    }
    let towards = -light.sun_travel.xyz;
    let sun = light.sun_colour.rgb * max(dot(s.normal, towards), 0.0);
    // ⚠ Only the natural ambient light reaches this pass: the artificial is the lighting pass's alone.
    let ambient = mix(light.ambient_down.rgb, light.ambient_up.rgb, s.normal.z * 0.5 + 0.5) * in.ambient.x;
    let view_dir = normalize(camera.camera_pos_ws.xyz - in.world_pos);
    let highlight = light.sun_colour.rgb * shine(s, towards, view_dir);
    // ⚠ What it reflects is no part of its coverage: a clear pane still shows the sky, which the ambient light's colour above and below stands in for, by the share `reflected` gives, and lets through only what it does not reflect.
    let mirrored = mix(light.ambient_down.rgb, light.ambient_up.rgb, reflect(-view_dir, s.normal).z * 0.5 + 0.5);
    let share = reflected(s, max(dot(s.normal, view_dir), 1e-4));
    let surface = highlight + mirrored * share;
    if (dropped(in, max(s.coverage, max3(surface)), BLEND_ALPHA)) {
        discard;
    }
    // Glass takes its lamps' light all round, as the glass program mixes it in whole for a lens that states no share of its own.
    let lamps = lamps_lit(s, in.world_pos);
    let lamp = select(lamps.facing, lamps.round, m.family.x == GLASS);
    let lit = (diffuse_of(s) * (sun + ambient + lamp) + s.emissive) * s.coverage + surface;
    return vec4<f32>(lit, s.coverage + share - s.coverage * share);
}

// What casts the sun's shadow whole: only the fade of its instance leaves a texel out.
@fragment
fn fs_cast(in: VsOut) {
    if (dropped(in, 1.0, 0.0)) {
        discard;
    }
}

// A cutout casts the shadow of what it draws.
@fragment
fn fs_cast_cutout(in: VsOut) {
    if (dropped(in, cut_coverage(in), CUTOUT_ALPHA)) {
        discard;
    }
}
"#;
