//! A test of every triangle that shares nothing with the walks or the instruction: the models'
//! own triangles, moved to the world by their instances' own transforms in double precision, each
//! tested against the ray's line by the signed volumes of its edges.
//!
//! Single precision puts a ray's line a few millionths of the scene's size away from where it
//! is, so a triangle whose edge the line passes within [`SLACK`] of, or whose distance lies within
//! [`NEAR`] of an end of the ray, is not said to be hit or missed: it is *unsure*, and a ray is
//! compared exactly only where no unsure triangle could be its answer.

use super::intersect::word;
use super::layout::*;
use super::{Cull, Hit, Instance, Ray, Triangle};

/// The distance, in the scene's units, between a ray's line and an edge's under which the
/// triangle is unsure. The scenes reach 64 units from their origin, where a float's last place is
/// 4e-6: the walks were seen to differ from this reference by under a tenth of the slack.
pub const SLACK: f64 = 2e-4;
/// The share of a ray's length a distance may differ by, and the margin about an end of the ray.
pub const NEAR: f64 = 2e-5;
/// What a weight may differ by.
pub const WEIGHT: f64 = 5e-4;

/// One triangle in the world.
pub struct Placed {
    corners: [[f64; 3]; 3],
    /// A sphere about the corners: a line that passes it by more than [`SLACK`] meets no part
    /// of the triangle, sure or unsure.
    centre: [f64; 3],
    radius: f64,
    /// The place of its instance in the list the top level was built from.
    pub instance: u32,
    pub geometry: u32,
    pub primitive: u32,
    mask: u8,
    flags: u8,
}

/// Every triangle of `models`, each a bottom level's triangles and one placement of it.
pub fn world(models: &[(&[Triangle], &Instance)]) -> Vec<Placed> {
    let mut placed = Vec::new();
    for (number, (triangles, instance)) in models.iter().enumerate() {
        let m = instance.object_to_world.map(|row| row.map(f64::from));
        for triangle in *triangles {
            let corners = triangle.corners.map(|corner| {
                let p = corner.map(f64::from);
                std::array::from_fn(|k| m[k][0] * p[0] + m[k][1] * p[1] + m[k][2] * p[2] + m[k][3])
            });
            let centre = std::array::from_fn(|k| corners.iter().map(|c| c[k]).sum::<f64>() / 3.0);
            let reach = corners.map(|corner| length(sub(corner, centre)));
            placed.push(Placed {
                corners,
                centre,
                radius: reach.into_iter().fold(0.0, f64::max),
                instance: number as u32,
                geometry: triangle.geometry,
                primitive: triangle.primitive,
                mask: instance.mask,
                flags: instance.flags,
            });
        }
    }
    placed
}

/// A triangle a ray meets, or one a walk says it met.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Met {
    pub t: f64,
    pub instance: u32,
    pub geometry: u32,
    pub primitive: u32,
    /// The weights of the model's second and third corner.
    pub weights: [f64; 2],
    /// After the instance's winding flag.
    pub front: bool,
}

/// What a walk's hit names, read from the arena's nodes.
pub fn met(words: &[u32], top: u32, hit: &Hit) -> Met {
    let instance = top + offset(hit.instance);
    let leaf = (word(words, instance, INSTANCE_BOTTOM) << 8) + offset(hit.leaf);
    Met {
        t: hit.t.into(),
        instance: word(words, instance, INSTANCE_INDEX) & 0xFFFF,
        geometry: word(words, leaf, TRIANGLE_WORD) >> 8,
        primitive: word(words, leaf, TRIANGLE_PRIMITIVE + 4 * (hit.leaf & 1)),
        weights: hit.weights.map(f64::from),
        front: hit.kind == 0xFE,
    }
}

/// Every triangle's verdict on one ray.
#[derive(Debug, Default)]
pub struct Answer {
    /// The triangles the ray meets inside its extent beyond doubt, nearest first.
    pub sure: Vec<Met>,
    /// The distances of the unsure ones; not a number where the ray runs in the triangle's plane.
    pub unsure: Vec<f64>,
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// What every triangle of `placed` whose instance answers to `mask` says of `ray`, with the faces
/// `cull` names left out as each instance's flags turn or drop the choice.
pub fn every_triangle(placed: &[Placed], ray: &Ray, mask: u8, cull: Cull) -> Answer {
    let (o, d) = (ray.origin.map(f64::from), ray.direction.map(f64::from));
    let (t_min, t_max) = (f64::from(ray.t_min), f64::from(ray.t_max));
    let mut answer = Answer::default();
    for triangle in placed.iter().filter(|p| p.mask & mask != 0) {
        let to = sub(triangle.centre, o);
        let along = dot(to, d) / dot(d, d);
        let off = length(sub(to, d.map(|x| x * along)));
        if off > triangle.radius + SLACK {
            continue;
        }
        let [a, b, c] = triangle.corners.map(|corner| sub(corner, o));
        // Six times the signed volume the ray's direction spans with each edge, the edge
        // opposite the first corner first; over the length of the direction's cross with the
        // edge it is the signed distance between the two lines.
        let volumes = [(b, c), (c, a), (a, b)].map(|(p, q)| dot(d, cross(p, q)));
        let apart = [(b, c), (c, a), (a, b)].map(|(p, q)| {
            let across = length(cross(d, sub(q, p)));
            dot(d, cross(p, q)) / across
        });
        let beyond = |sign: f64| apart.iter().filter(|x| **x * sign > SLACK).count();
        let (above, below) = (beyond(1.0), beyond(-1.0));
        if above > 0 && below > 0 {
            continue;
        }
        let sum = volumes[0] + volumes[1] + volumes[2];
        // All three volumes are below nought where the corners run counter-clockwise seen from
        // the ray's origin.
        let flip = u32::from(triangle.flags) & FLAG_FLIP_WINDING != 0;
        let front = (sum < 0.0) != flip;
        let normal = cross(sub(b, a), sub(c, a));
        let facing = dot(d, normal);
        let t = dot(a, normal) / facing;
        // A face seen edge-on has no side and no distance to speak of.
        let edge_on = facing.abs() <= 1e-9 * length(d) * length(normal);
        if edge_on || !t.is_finite() {
            answer.unsure.push(f64::NAN);
            continue;
        }
        let culled = match cull {
            Cull::None => false,
            Cull::Back => !front,
            Cull::Front => front,
        };
        if culled && u32::from(triangle.flags) & FLAG_NO_CULL == 0 {
            continue;
        }
        if t < t_min - NEAR || t >= t_max + NEAR {
            continue;
        }
        let inside = t >= t_min + NEAR && t < t_max - NEAR;
        if inside && (above == 3 || below == 3) {
            answer.sure.push(Met {
                t,
                instance: triangle.instance,
                geometry: triangle.geometry,
                primitive: triangle.primitive,
                weights: [volumes[1] / sum, volumes[2] / sum],
                front,
            });
        } else {
            answer.unsure.push(t);
        }
    }
    answer.sure.sort_by(|p, q| p.t.total_cmp(&q.t));
    answer
}

/// How a walk's answer stands against every triangle's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// No unsure triangle could be the answer, and the walk gave it: the hit or the miss, the
    /// distance within [`NEAR`], and the same triangle, weights and face.
    Same,
    /// An unsure triangle could be the answer, and the walk gave one of the possible ones.
    Possible,
    Differs,
}

fn same(found: &Met, wanted: &Met) -> bool {
    (found.t - wanted.t).abs() <= NEAR
        && (found.instance, found.geometry, found.primitive, found.front)
            == (
                wanted.instance,
                wanted.geometry,
                wanted.primitive,
                wanted.front,
            )
        && (0..2).all(|k| (found.weights[k] - wanted.weights[k]).abs() <= WEIGHT)
}

impl Answer {
    fn possible(&self, found: &Met) -> bool {
        let near = |t: &f64| t.is_nan() || (found.t - t).abs() <= NEAR;
        self.unsure.iter().any(near) || self.sure.iter().any(|met| same(found, met))
    }

    /// A walk for the closest hit that found `found`.
    pub fn closest(&self, found: Option<Met>) -> Verdict {
        let best = self.sure.first();
        let limit = best.map_or(f64::INFINITY, |met| met.t + NEAR);
        let doubt = self.unsure.iter().any(|t| t.is_nan() || *t < limit);
        // Two sure triangles as near as each other: either is the answer.
        let tied = (self.sure.get(1)).is_some_and(|second| second.t < limit);
        match (best, found) {
            (None, None) if !doubt => Verdict::Same,
            (Some(best), Some(found)) if !doubt && !tied && same(&found, best) => Verdict::Same,
            (_, None) if doubt && best.is_none() => Verdict::Possible,
            (_, Some(found)) if (doubt || tied) && found.t < limit && self.possible(&found) => {
                Verdict::Possible
            }
            _ => Verdict::Differs,
        }
    }

    /// A walk that ends on its first hit and found `found`, where `offered` is what it could
    /// commit, in the order it walked: the hits its candidate code was asked of and took.
    pub fn first(&self, found: Option<Met>, offered: &[Met]) -> Verdict {
        // Such a walk is done on the first it could commit: one offered after it is a walk gone
        // on, whatever every triangle says of that one.
        if found.as_ref() != offered.first() {
            return Verdict::Differs;
        }
        let doubt = !self.unsure.is_empty();
        match found {
            None if self.sure.is_empty() && !doubt => Verdict::Same,
            None if self.sure.is_empty() => Verdict::Possible,
            Some(found) if !doubt && self.sure.iter().any(|met| same(&found, met)) => Verdict::Same,
            Some(found) if doubt && self.possible(&found) => Verdict::Possible,
            _ => Verdict::Differs,
        }
    }
}
