//! The mesh of the water about the camera: each quad, or the triangle of it a cut leaves, cut along
//! one grid of the world so that quads meet, the depth of the ground under each vertex in its colour.

use glam::Vec3;
use or_io::gta6::water::height::WaterGround;
use or_io::gta6::water::{WaterData, WaterQuad};
use or_renderer::world::water::{OpenSea, WaterLook};
use or_renderer::MeshVertex;

/// The side of a cell the quads are cut into past `MIDDLE` of the camera, within it, and within `NEAR` of it, in metres.
// ⚠ Every vertex of a quad stands at its height, so cells of two sizes meet with no crack.
const FAR: f32 = 512.0;
const COARSE: f32 = 64.0;
const FINE: f32 = 16.0;
const MIDDLE: f32 = 3_000.0;
const NEAR: f32 = 1_000.0;
/// The depth over which a shore fades in where the frame's own water pass does not draw it, in metres.
const SHORE: f32 = 1.0;
/// The depth a vertex is given where the ground under it is not known: deeper than light goes.
pub const DEEP: f32 = 1_000.0;
/// What turns the day's extinction and scattering into ones a metre: the day states them a kilometre at a time.
// ⚠ Inferred: the fog atlas's commonest texel, which is a metre's, is the day's values over a thousand in three of six channels.
const PATH_SCALE: f32 = 0.001;
/// The water's colour where the day states no medium.
const TINT: [f32; 3] = [0.05, 0.18, 0.22];

/// The water as a medium light passes through, from the day's water variables.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Medium {
    /// How fast each channel is taken out along a path, in the day's unit.
    pub extinction: [f32; 3],
    /// The share of each channel taken out that comes back as the water's own colour.
    pub albedo: [f32; 3],
    /// What the sky the water reflects is multiplied by.
    pub reflection: f32,
    /// How far each channel's scattering leans forward.
    pub phase: [f32; 3],
}

impl Medium {
    /// The medium the variables `get` names state; `None` where they state no positive extinction.
    pub fn of(get: impl Fn(&str) -> Option<f32>) -> Option<Self> {
        let rgb = |stem: &str| -> Option<[f32; 3]> {
            Some([
                get(&format!("{stem}_r"))?,
                get(&format!("{stem}_g"))?,
                get(&format!("{stem}_b"))?,
            ])
        };
        let scale = |name: &str| get(name).unwrap_or(1.0);
        let extinction =
            rgb("water_clip_extinction")?.map(|e| e * scale("water_clip_extinction_scale"));
        let scattering = rgb("water_clip_scattering")
            .unwrap_or_default()
            .map(|s| s * scale("water_clip_scattering_scale"));
        let tint = rgb("water_clip_fog_color_tint").unwrap_or([1.0; 3]);
        let brightness = scale("water_clip_fog_brightness");
        if !extinction.iter().all(|e| *e > 0.0) {
            return None;
        }
        Some(Self {
            extinction,
            albedo: std::array::from_fn(|c| scattering[c] / extinction[c] * tint[c] * brightness),
            reflection: scale("water_reflection_color_mult"),
            phase: rgb("water_clip_phase").unwrap_or_default(),
        })
    }

    /// The medium as the frame's water pass takes it, its extinction a metre at a time, and the sea past the quads of `water`.
    pub fn look(&self, water: &WaterData) -> WaterLook {
        WaterLook {
            extinction: self.extinction.map(|e| e * PATH_SCALE),
            albedo: self.albedo,
            reflection: self.reflection,
            phase: self.phase,
            sea: open_sea(water),
        }
    }
}

/// The sea past the drawn quads of `water`, where it states its border: a plane at the border's height.
// ponytail: one plane at the corners' mean, and the border's segments not laid; this build states every corner at 0 and no segment.
pub fn open_sea(water: &WaterData) -> Option<OpenSea> {
    let border = water.border.as_ref()?;
    let drawn = (water.quads.iter())
        .filter(|q| q.kind <= 4 && !q.invisible && q.min[0] < q.max[0] && q.min[1] < q.max[1]);
    let bounds = drawn.fold(None, |b: Option<[f32; 4]>, q| {
        let q = [q.min[0], q.min[1], q.max[0], q.max[1]].map(f32::from);
        Some(b.map_or(q, |b| {
            [
                b[0].min(q[0]),
                b[1].min(q[1]),
                b[2].max(q[2]),
                b[3].max(q[3]),
            ]
        }))
    })?;
    Some(OpenSea {
        bounds,
        height: border.corners.iter().sum::<f32>() / 4.0,
        // Every quad, until the mesh says how far about the camera it drew them.
        drawn: [0.0, 0.0, f32::MAX],
    })
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The colour the water's one instance is tinted by: the day's medium, else a stand-in.
pub fn tint(medium: Option<&Medium>) -> [f32; 3] {
    medium.map_or(TINT, |m| m.albedo)
}

/// The vertex colour of the water where the ground lies `depth` under it: the depth in red for the frame's water pass, and in alpha the opacity a plain blended surface takes, which dry ground makes nought.
// ⚠ A quad's alpha bytes are not its opacity: blended by them the surface does not show.
fn shade(depth: Option<f32>) -> [f32; 4] {
    let shore = depth.map_or(1.0, |d| smoothstep(d / SHORE));
    [depth.unwrap_or(DEEP), 0.0, 0.0, shore]
}

/// Where a point at `u`, `v` across a quad of `kind` is on its water: at or above nought.
// A cut and its mirror are exact negations, so that the two meet at the very points of their diagonal.
fn side(kind: u8, u: f32, v: f32) -> f32 {
    match kind {
        1 => 1.0 - u - v,
        2 => v - u,
        3 => -(1.0 - u - v),
        4 => -(v - u),
        _ => 1.0,
    }
}

/// The pieces of `lo..hi` cut at every multiple of `step`.
fn spans(lo: f32, hi: f32, step: f32) -> impl Iterator<Item = (f32, f32)> {
    let mut at = lo;
    std::iter::from_fn(move || {
        let end = (((at / step).floor() + 1.0) * step).min(hi);
        // A piece that would not move on ends the pieces, as one past `hi` does.
        if at >= hi || end <= at {
            return None;
        }
        let piece = (at, end);
        at = end;
        Some(piece)
    })
}

/// A quad's corners in metres, and how to colour a point of it.
struct Quad<'a> {
    quad: &'a WaterQuad,
    min: [f32; 2],
    max: [f32; 2],
    ground: Option<&'a WaterGround>,
}

impl Quad<'_> {
    fn across(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x - self.min[0]) / (self.max[0] - self.min[0]),
            (y - self.min[1]) / (self.max[1] - self.min[1]),
        )
    }

    fn vertex(&self, x: f32, y: f32) -> MeshVertex {
        let z = self.quad.z;
        let depth = self.ground.and_then(|g| g.ground(x, y)).map(|h| z - h);
        MeshVertex::new_coloured([x, y, z], [0.0, 0.0, 1.0], [x, y], shade(depth))
    }

    /// The cell `x0..x1` by `y0..y1` of the quad, less what its cut leaves out, as a fan of triangles counter-clockwise from above.
    fn cell(&self, x: (f32, f32), y: (f32, f32), out: &mut (Vec<MeshVertex>, Vec<u32>)) {
        let corners = [(x.0, y.0), (x.1, y.0), (x.1, y.1), (x.0, y.1)];
        let kind = self.quad.kind;
        let on = |(px, py): (f32, f32)| {
            let (u, v) = self.across(px, py);
            side(kind, u, v)
        };
        let mut kept: Vec<(f32, f32)> = Vec::with_capacity(5);
        for k in 0..4 {
            let (p, q) = (corners[k], corners[(k + 1) % 4]);
            let (fp, fq) = (on(p), on(q));
            if fp >= 0.0 {
                kept.push(p);
            }
            if (fp > 0.0 && fq < 0.0) || (fp < 0.0 && fq > 0.0) {
                let t = fp / (fp - fq);
                kept.push((p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t));
            }
        }
        if kept.len() < 3 {
            return;
        }
        let first = out.0.len() as u32;
        out.0
            .extend(kept.iter().map(|&(px, py)| self.vertex(px, py)));
        for k in 1..kept.len() as u32 - 1 {
            out.1.extend([first, first + k, first + k + 1]);
        }
    }
}

/// The mesh of the quads of `quads` that are drawn and lie within `reach` metres of `eye`, each cut into cells of one grid of the world: in the world, the surface up, the texture coordinate the place in metres, and in the colour the depth of `ground` under it where that is known.
pub fn mesh(
    quads: &[WaterQuad],
    eye: Vec3,
    reach: f32,
    ground: Option<&WaterGround>,
) -> (Vec<MeshVertex>, Vec<u32>) {
    let mut out = (Vec::new(), Vec::new());
    // A quad with no area is passed over before its bounds are clamped to, which a reversed pair panics.
    let whole = |q: &&WaterQuad| q.min[0] < q.max[0] && q.min[1] < q.max[1];
    for quad in (quads.iter()).filter(|q| q.kind <= 4 && !q.invisible && whole(q)) {
        let (min, max) = (quad.min.map(f32::from), quad.max.map(f32::from));
        let nearest = |lo: [f32; 2], hi: [f32; 2]| {
            Vec3::new(eye.x.clamp(lo[0], hi[0]), eye.y.clamp(lo[1], hi[1]), eye.z)
        };
        if nearest(min, max).distance(eye) > reach {
            continue;
        }
        let quad = Quad {
            quad,
            min,
            max,
            ground,
        };
        // Only what lies within reach along each axis is cut into cells.
        let lo = [min[0].max(eye.x - reach), min[1].max(eye.y - reach)];
        let hi = [max[0].min(eye.x + reach), max[1].min(eye.y + reach)];
        // Each cell is cut again into the next size down while it lies within that size's reach.
        let mut cells = Vec::new();
        for x in spans(lo[0], hi[0], FAR) {
            cells.extend(spans(lo[1], hi[1], FAR).map(|y| (x, y)));
        }
        for (step, within) in [(COARSE, MIDDLE), (FINE, NEAR)] {
            cells = (cells.into_iter())
                .flat_map(|(x, y)| {
                    let near = nearest([x.0, y.0], [x.1, y.1]).distance(eye) < within;
                    let cut = |(lo, hi): (f32, f32)| -> Vec<(f32, f32)> {
                        match near {
                            true => spans(lo, hi, step).collect(),
                            false => vec![(lo, hi)],
                        }
                    };
                    let ys = cut(y);
                    cut(x)
                        .into_iter()
                        .flat_map(move |x| ys.clone().into_iter().map(move |y| (x, y)))
                })
                .collect();
        }
        for (x, y) in cells {
            quad.cell(x, y, &mut out);
        }
    }
    out
}

#[cfg(test)]
#[path = "water_mesh_tests.rs"]
mod tests;
