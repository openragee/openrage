//! The shapes a bound is made of and each query against one, exact and in the bound's own space.

use glam::{Affine3A, Vec2, Vec3};

use crate::extents::Aabb;

/// A ray within this cosine of a triangle's plane lies in it, and hits nothing.
const IN_PLANE: f32 = 1e-6;
/// How far past its edges a triangle is still hit, in barycentric units, so the seam of two leaves no gap.
const EDGE_SLACK: f32 = 1e-5;
/// A ray within this sine of a cylinder's axis runs along it.
const ALONG_AXIS: f32 = 1e-6;
/// How far a placing matrix's axes may be from one length and right angles, as a fraction.
const SKEW: f32 = 1e-3;
/// The steps a sweep takes before it gives up.
const SWEEP_STEPS: usize = 64;

/// A rotation, a uniform scale and a translation: what keeps a sphere a sphere.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Similarity {
    matrix: Affine3A,
    scale: f32,
}

impl Similarity {
    pub const IDENTITY: Self = Self {
        matrix: Affine3A::IDENTITY,
        scale: 1.0,
    };

    /// `matrix` as a similarity; `None` where its axes are not three of one length at right angles.
    pub fn new(matrix: Affine3A) -> Option<Self> {
        if !matrix.is_finite() {
            return None;
        }
        let [x, y, z] = [
            matrix.matrix3.x_axis,
            matrix.matrix3.y_axis,
            matrix.matrix3.z_axis,
        ]
        .map(Vec3::from);
        let scale = x.length();
        let slack = SKEW * scale;
        let even = (y.length() - scale).abs() <= slack && (z.length() - scale).abs() <= slack;
        let square = [x.dot(y), x.dot(z), y.dot(z)]
            .iter()
            .all(|d| d.abs() <= slack * scale);
        (scale > 0.0 && scale.is_finite() && even && square).then_some(Self { matrix, scale })
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn point(&self, p: Vec3) -> Vec3 {
        self.matrix.transform_point3(p)
    }

    /// A unit direction, turned and still a unit.
    pub fn turn(&self, direction: Vec3) -> Vec3 {
        self.matrix.transform_vector3(direction) / self.scale
    }

    pub fn inverse(&self) -> Self {
        Self {
            matrix: self.matrix.inverse(),
            scale: 1.0 / self.scale,
        }
    }

    /// This placement of what `first` has already placed.
    pub fn after(&self, first: &Self) -> Self {
        Self {
            matrix: self.matrix * first.matrix,
            scale: self.scale * first.scale,
        }
    }
}

/// A box with its own axes, which are units at right angles.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Obb {
    pub centre: Vec3,
    pub axes: [Vec3; 3],
    pub half: Vec3,
}

impl Obb {
    /// A box from its centre and its three half edges, squared up where they are not quite at right angles.
    pub fn from_half_edges(centre: Vec3, mut edges: [Vec3; 3]) -> Self {
        // The longest first: the frame stands on the edges that say most about it.
        edges.sort_by(|a, b| b.length_squared().total_cmp(&a.length_squared()));
        let x = edges[0].try_normalize().unwrap_or(Vec3::X);
        let y = (edges[1] - x * edges[1].dot(x))
            .try_normalize()
            .unwrap_or_else(|| x.any_orthonormal_vector());
        let z = x.cross(y);
        Self {
            centre,
            axes: [x, y, z],
            half: Vec3::new(
                edges[0].length(),
                edges[1].dot(y).abs(),
                edges[2].dot(z).abs(),
            ),
        }
    }

    /// A point in the box's own frame.
    pub fn local(&self, p: Vec3) -> Vec3 {
        let v = p - self.centre;
        Vec3::new(
            v.dot(self.axes[0]),
            v.dot(self.axes[1]),
            v.dot(self.axes[2]),
        )
    }

    fn world(&self, local: Vec3) -> Vec3 {
        self.centre + self.axes[0] * local.x + self.axes[1] * local.y + self.axes[2] * local.z
    }

    /// The point of the box nearest `p`, which is `p` inside it.
    pub fn closest(&self, p: Vec3) -> Vec3 {
        self.world(self.local(p).clamp(-self.half, self.half))
    }

    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let sign = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
            self.world(self.half * Vec3::new(sign(1), sign(2), sign(4)))
        })
    }

    /// How far the box reaches either side of its centre along `axis`.
    fn extent(&self, axis: Vec3) -> f32 {
        (0..3)
            .map(|i| self.half[i] * self.axes[i].dot(axis).abs())
            .sum()
    }
}

/// Where a query met a shape.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Touch {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
    /// The query began in the shape or against it.
    pub inside: bool,
}

/// A ray's first meeting with a shape: how far, the normal there, and whether it began inside.
type Entry = (f32, Vec3, bool);

/// One convex piece of a bound.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Shape {
    Triangle {
        a: Vec3,
        b: Vec3,
        c: Vec3,
    },
    Sphere {
        centre: Vec3,
        radius: f32,
    },
    Capsule {
        a: Vec3,
        b: Vec3,
        radius: f32,
    },
    /// Flat ended; a disc is one of no length.
    Cylinder {
        centre: Vec3,
        axis: Vec3,
        half_length: f32,
        radius: f32,
    },
    Box(Obb),
}

impl Shape {
    /// Whether every number is finite and no size is negative.
    pub fn sound(&self) -> bool {
        // ⚠ Each place is asked itself, never through the box alone: `min` and `max` pass over
        // a NaN, so the box of a triangle with a corner lost is the box of its other corners.
        let whole = match *self {
            Shape::Triangle { a, b, c } => a.is_finite() && b.is_finite() && c.is_finite(),
            Shape::Sphere { centre, radius } => centre.is_finite() && radius >= 0.0,
            Shape::Capsule { a, b, radius } => a.is_finite() && b.is_finite() && radius >= 0.0,
            Shape::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => centre.is_finite() && axis.is_finite() && half_length >= 0.0 && radius >= 0.0,
            Shape::Box(q) => {
                q.centre.is_finite()
                    && q.axes.iter().all(|axis| axis.is_finite())
                    && q.half.cmpge(Vec3::ZERO).all()
            }
        };
        // A size past what a number holds shows in the box, which is as far as the shape reaches.
        let aabb = self.aabb();
        whole && aabb.min.is_finite() && aabb.max.is_finite()
    }

    /// The shape where `place` puts it.
    pub fn placed(&self, place: &Similarity) -> Self {
        let s = place.scale();
        match *self {
            Shape::Triangle { a, b, c } => Shape::Triangle {
                a: place.point(a),
                b: place.point(b),
                c: place.point(c),
            },
            Shape::Sphere { centre, radius } => Shape::Sphere {
                centre: place.point(centre),
                radius: radius * s,
            },
            Shape::Capsule { a, b, radius } => Shape::Capsule {
                a: place.point(a),
                b: place.point(b),
                radius: radius * s,
            },
            Shape::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => Shape::Cylinder {
                centre: place.point(centre),
                axis: place.turn(axis),
                half_length: half_length * s,
                radius: radius * s,
            },
            Shape::Box(q) => Shape::Box(Obb {
                centre: place.point(q.centre),
                axes: q.axes.map(|axis| place.turn(axis)),
                half: q.half * s,
            }),
        }
    }

    fn moved(&self, by: Vec3) -> Self {
        match *self {
            Shape::Triangle { a, b, c } => Shape::Triangle {
                a: a + by,
                b: b + by,
                c: c + by,
            },
            Shape::Sphere { centre, radius } => Shape::Sphere {
                centre: centre + by,
                radius,
            },
            Shape::Capsule { a, b, radius } => Shape::Capsule {
                a: a + by,
                b: b + by,
                radius,
            },
            Shape::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => Shape::Cylinder {
                centre: centre + by,
                axis,
                half_length,
                radius,
            },
            Shape::Box(q) => Shape::Box(Obb {
                centre: q.centre + by,
                ..q
            }),
        }
    }

    /// The least and the most of the shape along a unit `axis`.
    pub fn interval(&self, axis: Vec3) -> (f32, f32) {
        let around = |middle: f32, reach: f32| (middle - reach, middle + reach);
        match *self {
            Shape::Triangle { a, b, c } => {
                let (a, b, c) = (a.dot(axis), b.dot(axis), c.dot(axis));
                (a.min(b).min(c), a.max(b).max(c))
            }
            Shape::Sphere { centre, radius } => around(centre.dot(axis), radius),
            Shape::Capsule { a, b, radius } => {
                let (a, b) = (a.dot(axis), b.dot(axis));
                (a.min(b) - radius, a.max(b) + radius)
            }
            Shape::Cylinder {
                centre,
                axis: along,
                half_length,
                radius,
            } => {
                let cosine = along.dot(axis);
                let rim = radius * (1.0 - cosine * cosine).max(0.0).sqrt();
                around(centre.dot(axis), half_length * cosine.abs() + rim)
            }
            Shape::Box(q) => around(q.centre.dot(axis), q.extent(axis)),
        }
    }

    pub fn aabb(&self) -> Aabb {
        let (x, y, z) = (
            self.interval(Vec3::X),
            self.interval(Vec3::Y),
            self.interval(Vec3::Z),
        );
        Aabb {
            min: Vec3::new(x.0, y.0, z.0),
            max: Vec3::new(x.1, y.1, z.1),
        }
    }

    /// The largest coordinate the shape reaches, either way.
    fn reach(&self) -> f32 {
        let aabb = self.aabb();
        aabb.min.abs().max(aabb.max.abs()).max_element()
    }

    /// Where a ray of unit `direction` first meets the shape within `max`.
    pub fn ray(&self, origin: Vec3, direction: Vec3, max: f32) -> Option<Touch> {
        let (t, normal, inside) = match *self {
            Shape::Triangle { a, b, c } => ray_triangle(origin, direction, a, b, c),
            Shape::Sphere { centre, radius } => ray_sphere(origin, direction, centre, radius),
            Shape::Capsule { a, b, radius } => ray_capsule(origin, direction, a, b, radius),
            Shape::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => ray_cylinder(origin, direction, centre, axis, half_length, radius),
            Shape::Box(q) => ray_box(origin, direction, &q),
        }?;
        (t <= max).then(|| Touch {
            t,
            point: origin + direction * t,
            normal,
            inside,
        })
    }

    /// The point of the shape nearest `p`, which is `p` inside a solid.
    pub fn closest(&self, p: Vec3) -> Vec3 {
        let toward = |core: Vec3, radius: f32| core + (p - core).clamp_length_max(radius);
        match *self {
            Shape::Triangle { a, b, c } => closest_on_triangle(p, a, b, c),
            Shape::Sphere { centre, radius } => toward(centre, radius),
            Shape::Capsule { a, b, radius } => toward(closest_on_segment(p, a, b), radius),
            Shape::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => {
                let v = p - centre;
                let along = v.dot(axis);
                let out = v - axis * along;
                centre
                    + axis * along.clamp(-half_length, half_length)
                    + out.clamp_length_max(radius)
            }
            Shape::Box(q) => q.closest(p),
        }
    }

    /// Where a sphere of `radius`, moved from `start` along unit `direction`, first touches the shape within `max`.
    pub fn sweep(&self, start: Vec3, direction: Vec3, max: f32, radius: f32) -> Option<Touch> {
        // ⚠ The gap between the sphere and a convex shape is a convex function of the distance
        // moved, so the tangent to it meets zero at or before the first contact: a step along
        // the tangent never passes a contact, and a gap that has stopped closing never closes.
        // Around the start, where the arithmetic is finest.
        let near = self.moved(-start);
        let tolerance = 16.0 * f32::EPSILON * near.reach().max(radius).max(1.0);
        let mut t = 0.0f32;
        for _ in 0..SWEEP_STEPS {
            let centre = direction * t;
            let on = near.closest(centre);
            let apart = centre - on;
            let distance = apart.length();
            let gap = distance - radius;
            if gap <= tolerance {
                return Some(Touch {
                    t,
                    point: on + start,
                    normal: if distance > tolerance {
                        apart / distance
                    } else {
                        -direction
                    },
                    inside: t == 0.0,
                });
            }
            let closing = -apart.dot(direction) / distance;
            if closing <= 0.0 {
                return None;
            }
            let next = (t + gap / closing).min(max);
            if next <= t {
                return None;
            }
            t = next;
        }
        None
    }

    /// Whether the whole shape lies in a box.
    pub fn within(&self, q: &Obb) -> bool {
        (0..3).all(|i| {
            let (low, high) = self.interval(q.axes[i]);
            let middle = q.centre.dot(q.axes[i]);
            low >= middle - q.half[i] && high <= middle + q.half[i]
        })
    }

    /// Whether the shape and a box share a point.
    pub fn touches(&self, q: &Obb) -> bool {
        let ball = |centre: Vec3, radius: f32| {
            (centre - q.closest(centre)).length_squared() <= radius * radius
        };
        match *self {
            Shape::Triangle { a, b, c } => {
                triangle_touches_box(q.local(a), q.local(b), q.local(c), q.half)
            }
            Shape::Sphere { centre, radius } => ball(centre, radius),
            // A capsule is a ball moved along its axis.
            Shape::Capsule { a, b, radius } => match (b - a).try_normalize() {
                Some(along) => Shape::Box(*q)
                    .sweep(a, along, (b - a).length(), radius)
                    .is_some(),
                None => ball(a, radius),
            },
            Shape::Cylinder {
                centre,
                axis,
                half_length,
                radius,
            } => cylinder_touches_box(centre, axis, half_length, radius, q),
            Shape::Box(b) => boxes_touch(&b, q),
        }
    }
}

/// A ray against both sides of a triangle.
fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<Entry> {
    let (e1, e2) = (b - a, c - a);
    let n = e1.cross(e2);
    let h = d.cross(e2);
    let det = e1.dot(h);
    // `det` is the triangle's doubled area by the cosine between the ray and its normal.
    if det * det <= IN_PLANE * IN_PLANE * n.length_squared() {
        return None;
    }
    let s = o - a;
    let u = s.dot(h) / det;
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    if u < -EDGE_SLACK || v < -EDGE_SLACK || u + v > 1.0 + EDGE_SLACK {
        return None;
    }
    let t = e2.dot(q) / det;
    if t < 0.0 {
        return None;
    }
    let normal = n.normalize();
    Some((t, if det > 0.0 { normal } else { -normal }, false))
}

fn ray_sphere(o: Vec3, d: Vec3, centre: Vec3, radius: f32) -> Option<Entry> {
    let from = o - centre;
    if from.length_squared() < radius * radius {
        return Some((0.0, -d, true));
    }
    // ⚠ Through the nearest point of the ray to the centre, never the
    // quadratic in `from`: a far origin squares to a number its radius is lost in.
    let along = -from.dot(d);
    let nearest = from + d * along;
    let short = radius * radius - nearest.length_squared();
    if short < 0.0 || along < 0.0 {
        return None;
    }
    let t = (along - short.sqrt()).max(0.0);
    Some((t, (from + d * t).try_normalize().unwrap_or(-d), false))
}

/// The stretch of a ray inside a convex solid, narrowed one pair of faces at a time.
struct Span {
    enter: f32,
    exit: f32,
    normal: Vec3,
}

impl Span {
    fn whole() -> Self {
        Self {
            enter: f32::NEG_INFINITY,
            exit: f32::INFINITY,
            normal: Vec3::ZERO,
        }
    }

    /// Keeps where `from + t * rate` is within `half` of zero along `axis`; false once nothing is left.
    fn slab(&mut self, from: f32, rate: f32, half: f32, axis: Vec3) -> bool {
        // ⚠ A ray along the faces is in or out for good, and dividing by its zero rate would say neither.
        if rate == 0.0 {
            return from.abs() <= half;
        }
        let (near, far) = if rate > 0.0 {
            ((-half - from) / rate, (half - from) / rate)
        } else {
            ((half - from) / rate, (-half - from) / rate)
        };
        self.narrow(near, far, if rate > 0.0 { -axis } else { axis })
    }

    fn narrow(&mut self, near: f32, far: f32, normal: Vec3) -> bool {
        if near > self.enter {
            self.enter = near;
            self.normal = normal;
        }
        self.exit = self.exit.min(far);
        self.enter <= self.exit
    }

    fn entry(&self, d: Vec3) -> Option<Entry> {
        if self.exit < 0.0 {
            return None;
        }
        Some(if self.enter < 0.0 {
            (0.0, -d, true)
        } else {
            (self.enter, self.normal, false)
        })
    }
}

fn ray_box(o: Vec3, d: Vec3, q: &Obb) -> Option<Entry> {
    let from = o - q.centre;
    let mut span = Span::whole();
    for i in 0..3 {
        if !span.slab(from.dot(q.axes[i]), d.dot(q.axes[i]), q.half[i], q.axes[i]) {
            return None;
        }
    }
    span.entry(d)
}

fn ray_cylinder(
    o: Vec3,
    d: Vec3,
    centre: Vec3,
    axis: Vec3,
    half_length: f32,
    radius: f32,
) -> Option<Entry> {
    let from = o - centre;
    let (from_along, rate_along) = (from.dot(axis), d.dot(axis));
    let mut span = Span::whole();
    if !span.slab(from_along, rate_along, half_length, axis) {
        return None;
    }
    // What is left across the axis is a ray against a circle.
    let (from_across, rate_across) = (from - axis * from_along, d - axis * rate_along);
    let speed = rate_across.length_squared();
    if speed > ALONG_AXIS * ALONG_AXIS {
        let nearest_at = -from_across.dot(rate_across) / speed;
        let nearest = from_across + rate_across * nearest_at;
        let short = radius * radius - nearest.length_squared();
        if short < 0.0 {
            return None;
        }
        let half_chord = (short / speed).sqrt();
        let near = nearest_at - half_chord;
        let normal = (from_across + rate_across * near)
            .try_normalize()
            .unwrap_or(-d);
        if !span.narrow(near, nearest_at + half_chord, normal) {
            return None;
        }
    } else if from_across.length_squared() > radius * radius {
        return None;
    }
    span.entry(d)
}

/// ⚠ A capsule is its two end balls and the cylinder between them, and a ray
/// enters the first of the three it reaches: the cylinder's flat ends lie
/// inside the balls, so they are never that first.
fn ray_capsule(o: Vec3, d: Vec3, a: Vec3, b: Vec3, radius: f32) -> Option<Entry> {
    let side = (b - a)
        .try_normalize()
        .and_then(|axis| ray_cylinder(o, d, (a + b) * 0.5, axis, (b - a).length() * 0.5, radius));
    [
        ray_sphere(o, d, a, radius),
        ray_sphere(o, d, b, radius),
        side,
    ]
    .into_iter()
    .flatten()
    .min_by(|x, y| x.0.total_cmp(&y.0))
}

fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let length = ab.length_squared();
    if length == 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / length).clamp(0.0, 1.0)
}

fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let n = (b - a).cross(c - a);
    let area = n.length_squared();
    if area > 0.0 {
        let foot = p - n * ((p - a).dot(n) / area);
        let inside = [(a, b), (b, c), (c, a)]
            .iter()
            .all(|(from, to)| (*to - *from).cross(foot - *from).dot(n) >= 0.0);
        if inside {
            return foot;
        }
    }
    // Off its face, or a triangle of no area: the nearest point is on an edge.
    [(a, b), (b, c), (c, a)]
        .iter()
        .map(|(from, to)| closest_on_segment(p, *from, *to))
        .min_by(|x, y| {
            (p - *x)
                .length_squared()
                .total_cmp(&(p - *y).length_squared())
        })
        .unwrap_or(a)
}

/// A triangle against a box at the origin of its own frame, by the thirteen axes that can part them.
fn triangle_touches_box(v0: Vec3, v1: Vec3, v2: Vec3, half: Vec3) -> bool {
    let parted = |axis: Vec3| {
        let (p0, p1, p2) = (v0.dot(axis), v1.dot(axis), v2.dot(axis));
        let reach = half.dot(axis.abs());
        p0.min(p1).min(p2) > reach || p0.max(p1).max(p2) < -reach
    };
    let edges = [v1 - v0, v2 - v1, v0 - v2];
    if Vec3::AXES.iter().any(|axis| parted(*axis)) || parted(edges[0].cross(edges[1])) {
        return false;
    }
    !edges
        .iter()
        .any(|edge| Vec3::AXES.iter().any(|axis| parted(axis.cross(*edge))))
}

/// Two boxes, by the fifteen axes that can part them.
fn boxes_touch(a: &Obb, b: &Obb) -> bool {
    let between = b.centre - a.centre;
    let parted = |axis: Vec3| between.dot(axis).abs() > a.extent(axis) + b.extent(axis);
    if a.axes.iter().chain(&b.axes).any(|axis| parted(*axis)) {
        return false;
    }
    !a.axes.iter().any(|i| {
        b.axes.iter().any(|j| {
            // Two edges that run together part nothing the faces have not.
            i.cross(*j).try_normalize().is_some_and(parted)
        })
    })
}

/// A flat-ended cylinder against a box.
///
/// ⚠ In the cylinder's frame the box is cut to the cylinder's length, and
/// what is left, seen along the axis, is the hull of its cut edges: the two
/// touch where that hull comes within a radius of the axis. No finite list of
/// axes parts a cylinder from a box, which is why this is not the boxes' test.
fn cylinder_touches_box(centre: Vec3, axis: Vec3, half_length: f32, radius: f32, q: &Obb) -> bool {
    let across = axis.any_orthonormal_vector();
    let frame = Obb {
        centre,
        axes: [across, axis.cross(across), axis],
        half: Vec3::ZERO,
    };
    let corners = q.corners().map(|corner| frame.local(corner));
    let mut seen: Vec<Vec2> = Vec::with_capacity(24);
    for from in 0..8usize {
        for bit in [1, 2, 4] {
            if from & bit != 0 {
                continue;
            }
            let (a, b) = (corners[from], corners[from | bit]);
            let rise = b.z - a.z;
            let (mut enter, mut exit) = (0.0f32, 1.0f32);
            if rise == 0.0 {
                if a.z.abs() > half_length {
                    continue;
                }
            } else {
                let (low, high) = ((-half_length - a.z) / rise, (half_length - a.z) / rise);
                enter = enter.max(low.min(high));
                exit = exit.min(low.max(high));
                if enter > exit {
                    continue;
                }
            }
            seen.push(a.lerp(b, enter).truncate());
            seen.push(a.lerp(b, exit).truncate());
        }
    }
    if seen.is_empty() {
        return false;
    }
    // A circle and a hull are apart only along a normal of the hull's edge or the line to its corner.
    let parted = |along: Vec2| {
        let (low, high) = seen.iter().fold((f32::MAX, f32::MIN), |(low, high), p| {
            let at = p.dot(along);
            (low.min(at), high.max(at))
        });
        low > radius || high < -radius
    };
    for (i, p) in seen.iter().enumerate() {
        if p.try_normalize().is_some_and(parted) {
            return false;
        }
        for other in &seen[i + 1..] {
            if (*other - *p)
                .try_normalize()
                .is_some_and(|edge| parted(edge.perp()))
            {
                return false;
            }
        }
    }
    true
}
