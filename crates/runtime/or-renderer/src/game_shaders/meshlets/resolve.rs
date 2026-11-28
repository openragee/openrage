//! The program that lights what the targets hold onto the frame and meters it, as WGSL.

/// Lights what the targets hold onto the frame, with the depth the pass drew: the first target's
/// colour as a surface of the second's normal and smoothness and the third's specular colour,
/// under a sun and a sky, then a tone curve and the frame's encoding. Where the light states a
/// tone, the directional light's arithmetic, the metering and the tone curve are GTA VI's: the
/// surfaces are metered into a texel of their own first, and the frame is exposed by it.
// The normal is an octahedral one in the world's space. The third target holds, of the specular
// colour `f`, `sqrt(1 - sqrt(1 - f))`. The fourth target's ambient terms come of buffers the
// editor leaves at nought, so they are not read.
// The colour is linear light: a pixel program writes what it samples of a map that states sRGB,
// scaled, with no transfer of its own.
pub(super) const RESOLVE: &str = r#"struct Light {
    // The way towards the sun or the moon.
    towards: vec4<f32>,
    // What it gives a surface that faces it, and the exposure in w.
    sun: vec4<f32>,
    // What the sky gives from above, and the ground from below.
    sky: vec4<f32>,
    ground: vec4<f32>,
    // Where the camera stands, and which target is shown raw: nought for the lit frame.
    camera: vec4<f32>,
    // From the clip space back to the world.
    unproject: mat4x4<f32>,
    // GTA VI's metering and tone: one where it runs, how far the directional light's roughness
    // is taken towards one, and the shares of the darkest and the brightest pixels left out.
    tone: vec4<f32>,
    // Two curves from a luminance to stops: scale, offset, added to the luminance, bending.
    curve: vec4<f32>,
    curve_blend: vec4<f32>,
    // The least and the most stops, the stops added, and how far to the second curve.
    stops: vec4<f32>,
    // The key luminance, whether the frame is metered by it, and the scale and the offset that
    // take an exposure to how far from the bright filmic curve to the dark one it stands.
    key: vec4<f32>,
    // The bright filmic curve's seven numbers and the tangent of half the light's disc, then
    // the dark curve's and one where the meter takes the whole frame.
    bright: vec4<f32>,
    bright_end: vec4<f32>,
    dark: vec4<f32>,
    dark_end: vec4<f32>,
    // From the world to the tile the sun's shadow is cast into, then: one where a shadow is
    // cast, how far across the tile it is searched for, the depth a surface is held clear of
    // itself by, and the penumbra a unit of the tile's depth opens, as a share of that search.
    to_shadow: mat4x4<f32>,
    shadow: vec4<f32>,
    // The clouds over the model: the share of the directional light they let through, which
    // the mask's writer takes into the mask, how far the light's roughness is taken towards one
    // under them, how far a light they hide is taken towards what the surroundings give along
    // its way, and that light's strength.
    clouds: vec4<f32>,
    // The ground laid under the model: what it sends back of each colour and one where one is
    // laid, then the middle of its square, how high it lies and half its side.
    laid: vec4<f32>,
    plane: vec4<f32>,
    // The nine numbers a probe holds of the light from all around, a row each; the first row's
    // last word is one where they are stated.
    around: array<vec4<f32>, 9>,
    // How bright the sky is along each of seventy-two ways over the upper half: six rings of
    // twelve from the top down, four ways a row.
    dome: array<vec4<f32>, 18>,
    // From the world to the clip space.
    project: mat4x4<f32>,
    // What a surface reflects of a light that comes evenly from all around, by the cosine of
    // the angle it is seen at, across, and its roughness, down, sixteen places a side: the
    // share its facets reflect by the part of their reflectance that grows towards grazing,
    // the share they reflect whole, and the share its matte part sends back.
    fit: array<vec4<f32>, 256>,
}
@group(0) @binding(0) var colour: texture_2d<f32>;
@group(0) @binding(1) var normal: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var<uniform> light: Light;
@group(0) @binding(4) var specular: texture_2d<f32>;
@group(0) @binding(5) var ambient: texture_2d<f32>;
// The model's surfaces as the light sees them: the nearest is nought. The mask's first writer
// reads it, and the lighting does not.
@group(0) @binding(6) var shadow_map: texture_depth_2d;
// The screen's mask of the directional light: in its first channel, the root of the share of
// that light that reaches what each pixel shows.
@group(0) @binding(7) var sun_mask: texture_2d<f32>;
// A word a tile of sixteen pixels a side: bit 14 asks for the share of each of the tile's
// pixels to be rebuilt from the pixels about it.
@group(0) @binding(8) var sun_tiles: texture_2d<u32>;
// The stops the surfaces metered to.
@group(1) @binding(0) var metered_stops: texture_2d<f32>;

struct Corner {
    @builtin(position) at: vec4<f32>,
    @location(0) clip: vec2<f32>,
}

@vertex fn at_corner(@builtin(vertex_index) i: u32) -> Corner {
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return Corner(vec4<f32>(corner, 0.0, 1.0), corner);
}

struct Lit {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

const PI: f32 = 3.14159265;
// What a colour weighs in its luminance.
const LUMINANCE: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);

// What the sky and the ground give along a way: the one above, the other below.
fn surroundings(way: vec3<f32>) -> vec3<f32> {
    return mix(light.ground.rgb, light.sky.rgb, way.z * 0.5 + 0.5);
}

// What falls on a surface that faces `n` of the light the probe's nine numbers hold, as GTA VI's
// lighting program takes it of a probe.
fn falls_on(n: vec3<f32>) -> vec3<f32> {
    let fallen = 0.886227 * light.around[0].rgb
        + 1.023328 * (light.around[1].rgb * n.y + light.around[2].rgb * n.z + light.around[3].rgb * n.x)
        + 0.858086 * (light.around[4].rgb * n.x * n.y + light.around[5].rgb * n.y * n.z + light.around[7].rgb * n.x * n.z)
        + (0.743125 * n.z * n.z - 0.247708) * light.around[6].rgb
        + 0.429043 * (n.x * n.x - n.y * n.y) * light.around[8].rgb;
    return max(fallen, vec3<f32>(0.0));
}

// What a surface seen at the cosine `n_v` with the roughness `roughness` reflects of an even
// light: the table, read between its four nearest places.
fn fit_at(n_v: f32, roughness: f32) -> vec3<f32> {
    let at = clamp(vec2<f32>(n_v, roughness), vec2<f32>(0.0), vec2<f32>(1.0)) * 15.0;
    let low = min(vec2<u32>(at), vec2<u32>(14u));
    let part = at - vec2<f32>(low);
    let row = low.y * 16u + low.x;
    let upper = mix(light.fit[row].xyz, light.fit[row + 1u].xyz, part.x);
    let lower = mix(light.fit[row + 16u].xyz, light.fit[row + 17u].xyz, part.x);
    return mix(upper, lower, part.y);
}

// The light that comes to a surface of `roughness` along `way` for its gloss, of what the
// probe's nine numbers hold: the mean over a cone about the way, each way of it weighed by its
// cosine to the cone's axis, as GTA VI takes a reflection it does not trace of a probe. The
// cone opens with the roughness and is never narrower than the angle whose cosine is 0.8; the
// probe's first four numbers are all it takes.
// The editor's own: that the probe is the one the matte light is taken of. The game traces the
// reflection, and where it does not, reads a volume of such probes about the camera.
fn mirrored(way: vec3<f32>, roughness: f32) -> vec3<f32> {
    let r4 = roughness * roughness * roughness * roughness;
    let k = min(0.8, inverseSqrt(1.0 + 9.0 * r4 * r4));
    let even = 0.886227 * (1.0 - k * k) * light.around[0].rgb;
    let leaning = 1.023328 * (1.0 - k * k * k)
        * (light.around[1].rgb * way.y + light.around[2].rgb * way.z + light.around[3].rgb * way.x);
    return max(even + leaning, vec3<f32>(0.0)) / (PI * (1.0 - k * k));
}

fn normal_of(packed_normal: vec4<f32>) -> vec3<f32> {
    let bytes = vec3<u32>(round(packed_normal.yzw * 255.0));
    let packed = bytes.x | (bytes.y << 8u) | (bytes.z << 16u);
    let e = vec2<f32>(f32(packed & 0xFFFu), f32(packed >> 12u)) / 2047.5 - 1.0;
    let n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    let fold = max(-n.z, 0.0);
    return normalize(vec3<f32>(n.x + select(fold, -fold, n.x >= 0.0), n.y + select(fold, -fold, n.y >= 0.0), n.z));
}

// The radius of tap `i` of the spiral the shadow is searched on, as a share of the widest.
fn tap_radius(i: i32) -> f32 {
    return pow(f32(3 + 2 * i) / 24.0, 1.5);
}

// One where the tile holds nothing before `deep` at `uv`, nought where something stands there.
fn open_at(uv: vec2<f32>, deep: f32) -> f32 {
    let size = vec2<f32>(textureDimensions(shadow_map));
    let texel = vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * (size - 1.0));
    return select(0.0, 1.0, textureLoad(shadow_map, texel, 0) >= deep);
}

// The share of the sun's or the moon's light that reaches `world`, on a surface that stands
// `n_l` to it, by the arithmetic GTA VI filters its cascades with: three taps beside the place,
// a search over twelve taps of a spiral for what stands before it, a penumbra as wide as that
// stands far, and the share of the penumbra's disc the taps leave open, ring by ring.
// The editor's own, and no reading of the game's filter: the turn of the spiral by the pixel;
// the 150 degrees from a tap to the next, the game's angles being read and their order not; how
// far the search reaches; the bias; a tap that loads the nearest texel, where the game's
// compares and filters; the one tile about the model, where the game has four cascades about
// its camera; and that the outermost tap, which stands past the search, weighs nothing.
fn sunlit(world: vec3<f32>, n_l: f32, pixel: vec2<f32>) -> f32 {
    if light.shadow.x <= 0.0 {
        return 1.0;
    }
    let clip = light.to_shadow * vec4<f32>(world, 1.0);
    let uv = clip.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return 1.0;
    }
    let size = vec2<f32>(textureDimensions(shadow_map));
    let texel = 1.0 / size;
    // How much deeper the surface lies a unit across the tile.
    let slope = min(sqrt(max(1.0 - n_l * n_l, 0.0)) / max(n_l, 1e-3), 8.0);
    let deep = min(clip.z, 1.0) - light.shadow.z;
    let turn = 6.2831853 * fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
    let c = cos(turn);
    let s = sin(turn);
    // A texel holds the depth at its middle: a place anywhere in it lies that much off.
    let lean = texel.x * slope;
    let beside = deep - 3.0 * lean;
    let near = (open_at(uv + texel * vec2<f32>(2.0 * c, 0.5 * s), beside)
        + open_at(uv + texel * vec2<f32>(-s, 0.25 * c), beside)
        + open_at(uv + texel * vec2<f32>(0.5 * c, -0.125 * s), beside)) / 3.0;
    let reach = light.shadow.y;
    var hidden: array<bool, 12>;
    var before = 0.0;
    var weight = 0.0;
    for (var i = 0; i < 12; i++) {
        let r = tap_radius(i);
        let angle = turn + f32(i) * 2.6179939;
        let at = clamp(uv + reach * r * vec2<f32>(cos(angle), sin(angle)), vec2<f32>(0.0), vec2<f32>(1.0));
        let held = textureLoad(shadow_map, vec2<i32>(at * (size - 1.0)), 0);
        hidden[i] = deep - r * reach * slope - lean > held;
        // A tap weighs less the farther it stands, and the last stands past the search, where
        // that would be under nought: it weighs nothing, so the weights never sum to less than
        // one of them and the mean depth is a depth some tap holds.
        let weighs = max(1.0 - r, 0.0);
        if hidden[i] {
            before += weighs * held;
            weight += weighs;
        }
    }
    var penumbra = 0.0;
    if weight > 0.0 {
        penumbra = (deep - before / weight) * light.shadow.w;
    }
    let p = max(clamp(penumbra, 0.0, 1.0), 0.03125);
    let disc = p * p;
    // The middle of the disc by the taps beside the place, then a ring a tap.
    var open = 0.0009765625 * near;
    var inner = 0.0009765625;
    for (var i = 0; i < 12; i++) {
        let outer = tap_radius(i) * tap_radius(i);
        if !hidden[i] {
            open += clamp(min(outer, disc) - inner, 0.0, 1.0);
        }
        inner = outer;
    }
    let share = clamp(open / disc, 0.0, 1.0);
    let squared = share * share;
    return squared + clamp(3.0 * p, 0.0, 1.0) * (squared * (3.0 - 2.0 * share) - squared);
}

// The pixels a side of a tile of the mask, and the bit of a tile's word that asks for the
// rebuild.
const TILE: i32 = 16;
const REBUILD: u32 = 0x4000u;

// How far from the camera's plane the frame shows something at `texel`: the model's surface,
// else the ground laid under it; nought where it shows neither.
fn seen_depth(texel: vec2<i32>) -> f32 {
    let size = vec2<f32>(textureDimensions(depth));
    let at = (vec2<f32>(texel) + 0.5) / size * 2.0 - 1.0;
    let clip = vec2<f32>(at.x, -at.y);
    var far = textureLoad(depth, texel, 0);
    if far <= 0.0 {
        let ground = ground_at(clip);
        if light.laid.w <= 0.0 || !ground.met {
            return 0.0;
        }
        far = ground.far;
    }
    let world = light.unproject * vec4<f32>(clip, far, 1.0);
    return abs(1.0 / world.w);
}

// The share of the sun's or the moon's light that reaches what the frame shows at `texel`, as
// GTA VI's lighting program takes it of the screen's mask: the square of the first channel;
// and where the texel's tile asks for it, the mean of that square and of the squares at the
// twelve places about the texel that lie as deep as it, to within 0.00275 of its depth.
// The editor's own: a place outside the frame is left out, as one that lies at no depth is.
fn sun_share(texel: vec2<i32>) -> f32 {
    let own = textureLoad(sun_mask, texel, 0).r;
    var share = own * own;
    if (textureLoad(sun_tiles, texel / TILE, 0).r & REBUILD) == 0u {
        return share;
    }
    var about = array<vec2<i32>, 12>(
        vec2<i32>(-1, -1), vec2<i32>(1, -1), vec2<i32>(-1, 1), vec2<i32>(1, 1),
        vec2<i32>(-2, 0), vec2<i32>(2, 0), vec2<i32>(0, -2), vec2<i32>(0, 2),
        vec2<i32>(-4, 0), vec2<i32>(4, 0), vec2<i32>(0, -4), vec2<i32>(0, 4),
    );
    let deep = seen_depth(texel);
    let most = vec2<i32>(textureDimensions(sun_mask)) - 1;
    var held = 1.0;
    for (var i = 0; i < 12; i++) {
        let at = texel + about[i];
        if any(at < vec2<i32>(0)) || any(at > most) {
            continue;
        }
        if abs(seen_depth(at) - deep) < 0.00275 * deep {
            let root = textureLoad(sun_mask, at, 0).r;
            share += root * root;
            held += 1.0;
        }
    }
    return share / held;
}

// A place on a surface: where it is in the world, the way it faces, its colour, what it
// reflects head on and how rough it is; its material's kind and how much light passes through it.
struct Surface {
    at: vec3<f32>,
    n: vec3<f32>,
    albedo: vec3<f32>,
    f0: vec3<f32>,
    roughness: f32,
    kind: u32,
    through: f32,
}

// The kind GTA VI's tiled lighting picks a pixel's lighting program by: the low five bits of the
// colour target's fourth channel as a byte, cut, not rounded.
fn kind_of(colour_a: f32) -> u32 {
    return u32(colour_a * 255.0) & 31u;
}

// The kind of a leaf or a sheet that lets light through, and the kind of skin; every other kind is
// lit as the standard material, kind nought.
const TRANSLUCENCY: u32 = 5u;
const SKIN: u32 = 29u;

// What a disc of light whose angular radius has the squared sine `size` gives a surface it stands
// at the cosine `c` to, over what it gives one that faces it: the share of the disc above the
// surface's horizon, each part by its cosine.
fn disc_lit(cosine: f32, size: f32) -> f32 {
    let s2 = min(size, 0.9999);
    let c = clamp(cosine, -0.999, 0.999);
    var lit = 0.0;
    if c * c > s2 {
        lit = clamp(c, 0.0, 1.0) * s2;
    } else {
        let x = sqrt(1.0 / s2 - 1.0);
        let sin_theta = sqrt(1.0 - c * c);
        let y = -x * c / sin_theta;
        let below = sqrt(max(1.0 - y * y, 0.0)) * sin_theta;
        lit = ((c * acos(y) - x * below) * s2 + atan(below / x)) / PI;
    }
    return max(lit, 0.0) / size;
}

// The two channels of the table GTA VI's translucency program samples at a surface's cosine to
// the light, `n_l`, signed: what a light in front of it gives, and what a light behind it gives,
// on the table's last row, where the first disc's sine is 1/100 and the second's 100^(-1/3).
fn falloff(n_l: f32) -> vec2<f32> {
    return vec2<f32>(disc_lit(n_l, 1e-4), disc_lit(-n_l, 0.04641589));
}

// What GTA VI's translucency program makes of the amount of light that passes through a leaf.
struct Through {
    // A profile of the amount: nought where nothing passes, and then the leaf is lit as kind
    // nought is.
    profile: f32,
    // What the matte part of the sun's light is taken by where something passes.
    matte: f32,
    // The phase lobe's asymmetry, and what the light through the leaf is scaled by beside it.
    g: f32,
    scale: f32,
}

fn through_of(amount: f32) -> Through {
    let x = clamp(2.0 * amount, 0.0, 1.0);
    let y = clamp(2.0 * amount - 1.0, 0.0, 1.0);
    var profile = 0.0;
    if 0.2 * amount > 0.0 {
        let e = exp2((0.09137069 * x * x - 0.09617967) / max(0.2 * amount, 1e-4));
        profile = 0.25 * e * (e * e + 3.0);
    }
    return Through(profile, 1.0 - 0.6 * x, 0.9 * y, profile / PI * (1.0 - 0.8283803 * y));
}

// The light that passes through a leaf of `through` from the sun or the moon, before the leaf's
// colour, the light and the table's share from behind: a Henyey-Greenstein lobe about the way to
// the light by `v_l`, plus two, times the matte falloff's two shares `shape`.
fn passed(through: Through, v_l: f32, shape: f32) -> f32 {
    if through.profile <= 0.0 {
        return 0.0;
    }
    let g = through.g;
    let lobe = (1.0 - g * g) / pow(1.0 + g * g + 2.0 * g * v_l, 1.5) + 2.0;
    return through.scale * shape * lobe;
}

// The matte part of the sun's or the moon's light on skin, as GTA VI's skin program takes it at
// the cosine `n_l`, signed, before the share its facets reflect: the falloff table's first share
// over pi, with neither the standard's grazing term nor its roughness factor. Its light through
// the skin, its second gloss lobe and its subsurface blur take a fifth target and the game's
// tables of profiles and materials, which the editor has none of; with none, the program adds
// nothing of them.
fn skin_matte(n_l: f32) -> f32 {
    return falloff(n_l).x / PI;
}

// The surface the targets hold at `texel`; `clip` is where the texel stands on the frame and
// `far` its depth.
fn surface_at(texel: vec2<i32>, clip: vec2<f32>, far: f32) -> Surface {
    let packed_normal = textureLoad(normal, texel, 0);
    let terms = textureLoad(specular, texel, 0);
    let world = light.unproject * vec4<f32>(clip.x, clip.y, far, 1.0);
    let kept = 1.0 - terms.rgb * terms.rgb;
    let albedo = textureLoad(colour, texel, 0);
    // The amount that passes: the specular target's fourth channel, less the fourth target's
    // second.
    let through = clamp(terms.a * (1.0 - textureLoad(ambient, texel, 0).y), 0.0, 1.0);
    return Surface(
        world.xyz / world.w,
        normal_of(packed_normal),
        albedo.rgb,
        1.0 - kept * kept,
        clamp(1.0 - packed_normal.x, 0.08, 1.0),
        kind_of(albedo.a),
        through,
    );
}

// The light `surface` sends to the camera, in the light's own units, before any exposure;
// `pixel` is where it is seen on the frame.
fn shade(surface: Surface, pixel: vec2<f32>) -> vec3<f32> {
    let n = surface.n;
    let albedo = surface.albedo;
    let f0 = surface.f0;
    let roughness = surface.roughness;
    let v = normalize(light.camera.xyz - surface.at);
    let l = light.towards.xyz;
    let h = normalize(l + v);
    let n_l = max(dot(n, l), 0.0);
    let n_v = max(dot(n, v), 1e-4);
    let n_h = max(dot(n, h), 0.0);
    let v_h = max(dot(h, v), 0.0);
    let game = light.tone.x > 0.0;
    // The game takes the directional light's roughness part of the way to one, by one number
    // in the open and by another in a cloud's shadow.
    let open = light.clouds.x;
    let softened = roughness + select(0.0, mix(light.clouds.y, light.tone.y, open), game) * (1.0 - roughness);
    let a = max(softened * softened, 0.001);
    let a2 = a * a;
    // The microfacet terms: how many facets face halfway, how many of them are seen and lit,
    // and how much a facet reflects at this angle.
    let spread = n_h * n_h * (a2 - 1.0) + 1.0;
    let facets = a2 / (PI * spread * spread);
    let seen = 0.5 / max(n_l * sqrt(n_v * n_v * (1.0 - a2) + a2) + n_v * sqrt(n_l * n_l * (1.0 - a2) + a2), 1e-4);
    let reflected = f0 + (1.0 - f0) * pow(1.0 - v_h, 5.0);
    var gloss = facets * seen * reflected;
    var matte = vec3<f32>(1.0 / PI);
    // A leaf lit by GTA VI's translucency program, and what passes through it.
    let leaf = game && surface.kind == TRANSLUCENCY;
    let skin = game && surface.kind == SKIN;
    let through = through_of(select(0.0, surface.through, leaf));
    let passes = through.profile > 0.0;
    var passing = 0.0;
    if game {
        // The gloss of a light that is a disc and no point, and what the facets reflect onto
        // one another: a fit by the angle and the roughness.
        let disc = light.bright_end.w;
        let lobe = a2 * (v_h + 0.001);
        gloss *= lobe / (lobe + 0.25 * disc * (disc + 3.0 * a));
        let by_angle = (((4.51786 * n_l - 13.70343) * n_l + 15.88844) * n_l - 9.11368) * n_l + 3.09507;
        let by_roughness = ((0.73343 * a - 2.61748) * a + 2.77203) * a - 0.20276;
        let lost = -1.4594 * n_l * by_angle * a * by_roughness;
        gloss *= 1.0 + f0 * (1.0 / (1.0 + lost) - 1.0);
        // The matte part: brighter at grazing angles on a rough surface, dimmer on a smooth.
        let from_light = pow(1.0 - n_l, 5.0);
        let from_view = pow(1.0 - n_v, 5.0);
        let l_h = clamp(dot(l, h), 0.0, 1.0);
        let back = 2.0 * l_h * l_h * a;
        // Where light passes through a leaf the matte's grazing term is fixed at a half.
        let k = select(0.5 * a - 1.0, -0.5, passes);
        let shape = (1.0 + k * from_light) * (1.0 + k * from_view) + back * (from_light + from_view + from_light * from_view * (back - 1.0));
        matte = vec3<f32>((1.0 - 0.3377483 * a) * shape / PI);
        if passes {
            matte *= through.matte;
            passing = passed(through, dot(v, l), (1.0 + k * from_light) * (1.0 + k * from_view));
        }
    }
    // The light from all around is a probe's nine numbers where they are stated, else the two
    // colours of the sky and the ground.
    let probed = light.around[0].w > 0.0;
    var direct = vec3<f32>(0.0);
    if n_l > 0.0 || leaf || skin {
        // A light the clouds hide goes towards what the surroundings give along its way, then
        // the mask dims it: by the model's own shadow and by the clouds', which the mask holds
        // as one share, as the game's does.
        var from_light = light.sun.rgb;
        let hazed = select(surroundings(l) * PI, falls_on(l), probed) * light.clouds.w / max(open, 0.001);
        from_light += light.clouds.z * (1.0 - open) * (hazed - from_light);
        from_light *= sun_share(vec2<i32>(pixel));
        if leaf {
            // The matte falloff is the table's first share and what passes takes its second, from
            // behind, so a leaf the light stands behind is lit through.
            let shares = falloff(dot(n, l));
            direct = (albedo * ((1.0 - reflected) * matte * shares.x + passing * shares.y) + gloss * n_l) * from_light;
        } else if skin {
            direct = (albedo * (1.0 - reflected) * skin_matte(dot(n, l)) + gloss * n_l) * from_light;
        } else {
            direct = (albedo * (1.0 - reflected) * matte + gloss) * from_light * n_l;
        }
    }
    // From all around: the sky and the ground on the colour, and along the mirrored way on the
    // gloss, less as the surface is rougher.
    let grazing = f0 + (max(vec3<f32>(1.0 - roughness), f0) - f0) * pow(1.0 - n_v, 5.0);
    let matte_around = select(surroundings(n), falls_on(n) / PI, probed);
    var around = albedo * matte_around + grazing * surroundings(reflect(-v, n)) * (1.0 - roughness);
    if game && probed {
        // As the game's lighting program weighs the two lights it is handed from all around:
        // the gloss by what the surface reflects of an even light, and by what its facets
        // reflect onto one another; the matte by what is left, and by its own share.
        let fit = fit_at(n_v, roughness);
        let reflects = f0 * fit.y + (1.0 - f0) * fit.x;
        let again = 1.0 + f0 * (1.0 / max(fit.y, 1e-3) - 1.0);
        around = albedo * matte_around * (1.0 - reflects) * fit.z
            + mirrored(reflect(-v, n), roughness) * reflects * again;
    }
    return direct + around;
}

// The light the surface at `texel` sends to the camera.
fn lit_at(texel: vec2<i32>, clip: vec2<f32>, far: f32) -> vec3<f32> {
    return shade(surface_at(texel, clip, far), vec2<f32>(texel));
}

// A place the ray of a pixel meets the ground at, and how deep it is on the frame.
struct Met {
    surface: Surface,
    far: f32,
    met: bool,
}

// Where the ray of the pixel at `clip` starts on the frame's near side, and its way.
struct Ray {
    start: vec3<f32>,
    way: vec3<f32>,
}

fn ray_of(clip: vec2<f32>) -> Ray {
    let near = light.unproject * vec4<f32>(clip, 1.0, 1.0);
    let further = light.unproject * vec4<f32>(clip, 0.5, 1.0);
    let start = near.xyz / near.w;
    return Ray(start, normalize(further.xyz / further.w - start));
}

// Where the ray of the pixel at `clip` meets the square of ground under the model, seen from
// above. A matte ground that reflects what a surface that is no metal does.
fn ground_at(clip: vec2<f32>) -> Met {
    var none: Met;
    let ray = ray_of(clip);
    let start = ray.start;
    let way = ray.way;
    if way.z >= -1e-5 || start.z <= light.plane.z {
        return none;
    }
    let at = start + way * ((light.plane.z - start.z) / way.z);
    if any(abs(at.xy - light.plane.xy) > vec2<f32>(light.plane.w)) {
        return none;
    }
    let seen = light.project * vec4<f32>(at, 1.0);
    let surface = Surface(at, vec3<f32>(0.0, 0.0, 1.0), light.laid.rgb, vec3<f32>(0.04), 1.0, 0u, 0.0);
    return Met(surface, clamp(seen.z / seen.w, 0.0, 1.0), seen.w > 0.0);
}

// The root of the share of the directional light that reaches what the pixel shows, the
// model's surface or the ground laid under it: what the shadow map leaves of it, times what the
// clouds let through, as GTA VI's cascade resolve multiplies its clouds' shadow into the share
// before it takes the root. One where the pixel shows neither. A surface that faces away from the
// light is filtered as one that faces it at the same angle, since a leaf takes the light behind
// it; the standard lighting reads no share where it faces away.
@fragment fn masked(corner: Corner) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(corner.at.xy);
    let far = textureLoad(depth, texel, 0);
    var at: vec3<f32>;
    var n: vec3<f32>;
    if far <= 0.0 {
        let ground = ground_at(corner.clip);
        if light.laid.w <= 0.0 || !ground.met {
            return vec4<f32>(1.0);
        }
        at = ground.surface.at;
        n = ground.surface.n;
    } else {
        let world = light.unproject * vec4<f32>(corner.clip.x, corner.clip.y, far, 1.0);
        at = world.xyz / world.w;
        n = normal_of(textureLoad(normal, texel, 0));
    }
    let n_l = abs(dot(n, light.towards.xyz));
    return vec4<f32>(sqrt(sunlit(at, n_l, vec2<f32>(texel)) * light.clouds.x));
}

// What the frame shows of the light `seen`: exposed, toned and encoded.
fn shown_of(seen: vec3<f32>) -> vec3<f32> {
    if light.tone.x > 0.0 {
        let exposure = exp2(textureLoad(metered_stops, vec2<i32>(0), 0).x) * light.sun.w;
        return filmic(seen * exposure, exposure);
    }
    return encoded(toned(seen * light.sun.w));
}

@fragment fn lit(corner: Corner) -> Lit {
    let texel = vec2<i32>(corner.at.xy);
    let far = textureLoad(depth, texel, 0);
    if far <= 0.0 {
        // No surface of the model: the ground under it, where one is laid and the ray meets it.
        let ground = ground_at(corner.clip);
        if light.laid.w <= 0.0 || !ground.met || u32(light.camera.w) != 0u {
            discard;
        }
        return Lit(vec4<f32>(shown_of(shade(ground.surface, vec2<f32>(texel))), 1.0), ground.far);
    }
    let shown = u32(light.camera.w);
    if shown != 0u {
        let packed_normal = textureLoad(normal, texel, 0);
        let albedo = textureLoad(colour, texel, 0);
        let terms = textureLoad(specular, texel, 0);
        if shown == 1u { return Lit(vec4<f32>(albedo.rgb, 1.0), far); }
        if shown == 2u { return Lit(vec4<f32>(packed_normal.rgb, 1.0), far); }
        if shown == 3u { return Lit(vec4<f32>(normal_of(packed_normal) * 0.5 + 0.5, 1.0), far); }
        if shown == 4u { return Lit(vec4<f32>(terms.rgb, 1.0), far); }
        if shown == 5u { return Lit(vec4<f32>(textureLoad(ambient, texel, 0).rgb, 1.0), far); }
        return Lit(vec4<f32>(albedo.a, packed_normal.a, terms.a, 1.0), far);
    }
    return Lit(vec4<f32>(shown_of(lit_at(texel, corner.clip, far)), 1.0), far);
}

// A filmic curve fitted to the reference one: light past one rolls off and does not clip.
fn toned(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// The frame shows what is written to it as it is, and a program's colour is linear light.
fn encoded(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

fn filmic_raw(x: vec3<f32>, abcd: vec4<f32>, efw: vec4<f32>) -> vec3<f32> {
    return (x * (abcd.x * x + abcd.z * abcd.y) + abcd.w * efw.x) / (x * (abcd.x * x + abcd.y) + abcd.w * efw.y) - efw.x / efw.y;
}

// GTA VI's tone curve of an exposed colour: the filmic curve between the bright and the dark
// one by the exposure, its white taken to one, then a power for the frame.
fn filmic(x: vec3<f32>, exposure: f32) -> vec3<f32> {
    let along = clamp(exposure * light.key.z + light.key.w, 0.0, 1.0);
    let abcd = mix(light.bright, light.dark, along);
    let efw = mix(light.bright_end, light.dark_end, along);
    let white = filmic_raw(vec3<f32>(efw.z), abcd, efw).x;
    let curved = clamp(filmic_raw(max(x, vec3<f32>(0.0)), abcd, efw) / white, vec3<f32>(0.0), vec3<f32>(1.0));
    return pow(curved, vec3<f32>(0.4545));
}

// The surfaces the meter looks at: a lattice of this many a side over the frame.
const LATTICE: i32 = 32;

// How bright a pixel no surface of the model covers is, where the meter takes the whole frame
// as the game sees one: the ground laid under the model where the ray meets it, else the sky
// along the ray, or under the horizon what the ground sends up.
fn beyond(texel: vec2<i32>, clip: vec2<f32>) -> f32 {
    let ground = ground_at(clip);
    if light.laid.w > 0.0 && ground.met {
        return dot(shade(ground.surface, vec2<f32>(texel)), LUMINANCE);
    }
    let way = ray_of(clip).way;
    if way.z <= 0.0 {
        return dot(light.ground.rgb, LUMINANCE);
    }
    let ring = min(u32(acos(min(way.z, 1.0)) / (PI / 2.0) * 6.0), 5u);
    let turn = fract(atan2(way.y, way.x) / (2.0 * PI) + 1.0);
    let at = ring * 12u + min(u32(turn * 12.0), 11u);
    return light.dome[at / 4u][at % 4u];
}

// What the lit surfaces meter to, as GTA VI meters a frame: each sample's luminance goes to one
// of 256 bins laid evenly over the logarithm between the least and the greatest, the darkest
// and the brightest shares are left out, the rest average, and the average gives the stops by
// the curves or by the key. Nought where no surface is seen or no tone is stated. The samples
// are the model's surfaces, or every place of the frame where the light asks for the whole.
@fragment fn metered(corner: Corner) -> @location(0) vec4<f32> {
    if light.tone.x <= 0.0 {
        return vec4<f32>(0.0);
    }
    let size = vec2<i32>(textureDimensions(depth));
    var luminances: array<f32, 1024>;
    var count = 0;
    var least = 1e30;
    var most = 0.0;
    for (var y = 0; y < LATTICE; y++) {
        for (var x = 0; x < LATTICE; x++) {
            let texel = (vec2<i32>(x, y) * 2 + 1) * size / (2 * LATTICE);
            let far = textureLoad(depth, texel, 0);
            let whole = light.dark_end.w > 0.0;
            if far <= 0.0 && !whole {
                continue;
            }
            let at = (vec2<f32>(texel) + 0.5) / vec2<f32>(size) * 2.0 - 1.0;
            let clip = vec2<f32>(at.x, -at.y);
            var luminance = 0.0;
            if far > 0.0 {
                luminance = dot(lit_at(texel, clip, far), LUMINANCE);
            } else {
                luminance = beyond(texel, clip);
            }
            if luminance != luminance {
                continue;
            }
            let held = max(luminance, 1e-6);
            luminances[count] = held;
            count++;
            least = min(least, held);
            most = max(most, held);
        }
    }
    if count == 0 {
        return vec4<f32>(0.0);
    }
    let ratio = select(1.0, most / least, least >= 1e-8);
    let span = max(log(ratio), 1e-8);
    var bins: array<f32, 256>;
    for (var i = 0; i < count; i++) {
        let bin = clamp(255.0 * log(luminances[i] / least) / span, 0.0, 255.0);
        bins[u32(bin)] += 1.0;
    }
    let lows = round(light.tone.z * f32(count));
    let middle_ends = round(f32(count) - (round(light.tone.w * f32(count)) + lows)) + lows;
    var before = 0.0;
    var sum = 0.0;
    var taken = 0.0;
    for (var bin = 0; bin < 256; bin++) {
        var left = bins[bin];
        var reached = before;
        if left > 0.0 && lows > reached {
            left -= min(lows - reached, left);
            reached = lows;
        }
        if left > 0.0 && middle_ends > reached {
            let middle = min(middle_ends - reached, left);
            sum += middle * clamp(least * pow(ratio, f32(bin) / 255.0), least, most);
            taken += middle;
        }
        before += bins[bin];
    }
    let mean = select(0.0, sum / max(taken, 1.0), taken > 0.0);
    var stops = log2(light.key.x / max(mean, 1e-6));
    if light.key.y <= 0.0 {
        let first = light.curve.y - 10.0 + light.curve.w * mean + light.curve.x * log(light.curve.z + mean);
        let second = light.curve_blend.y - 10.0 + light.curve_blend.w * mean + light.curve_blend.x * log(light.curve_blend.z + mean);
        stops = first + light.stops.w * (second - first);
    }
    if stops != stops {
        stops = light.stops.y;
    }
    stops = clamp(clamp(stops, light.stops.x, light.stops.y) + light.stops.z, light.stops.x, light.stops.y);
    return vec4<f32>(stops, 0.0, 0.0, 1.0);
}
"#;
