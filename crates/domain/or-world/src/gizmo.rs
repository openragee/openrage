//! The manipulator: handles drawn at an entity, the one under the pointer, and what dragging it does to the entity's pose.
//!
//! Pure geometry over `glam`, with no window and no renderer: the screen is a `View` (the matrix the viewport draws with and the
//! rectangle it fills), the pointer is a pixel, and a drag is a [`Drag`] that is begun at a pixel and asked for the pose at the next.
//! The same handle is reached from the keyboard by typing the pose (`crate::transform::Pose`), so nothing here is the only way to a value.
//!
//! ⚠ **Scale is two numbers.** A placement stores one scale for X and Y together and one for Z (`scaleXY`, `scaleZ`), so an X or Y
//! handle scales both horizontal axes, the Z handle scales the vertical one, and the centre scales all three. A scale of X alone
//! cannot be stored, and no handle offers it.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use crate::transform::Pose;

/// What a drag changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Move,
    Rotate,
    Scale,
}

/// One of the three world axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn direction(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }

    /// The colour the axis is drawn in: red, green and blue.
    pub fn colour(self) -> [f32; 4] {
        match self {
            Axis::X => [0.92, 0.25, 0.25, 1.0],
            Axis::Y => [0.35, 0.82, 0.30, 1.0],
            Axis::Z => [0.30, 0.50, 0.95, 1.0],
        }
    }

    /// The two axes that span the plane this one is the normal of.
    fn others(self) -> (Axis, Axis) {
        match self {
            Axis::X => (Axis::Y, Axis::Z),
            Axis::Y => (Axis::Z, Axis::X),
            Axis::Z => (Axis::X, Axis::Y),
        }
    }
}

/// A part of the manipulator the pointer can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// An arrow: moves along the axis, or scales along it.
    Axis(Axis),
    /// A square in the plane whose normal is the axis: moves in that plane.
    Plane(Axis),
    /// A ring about the axis: turns about it.
    Ring(Axis),
    /// The centre: scales every axis together.
    Centre,
}

impl Handle {
    /// The words a screen reader and a hover say.
    pub fn name(self) -> String {
        match self {
            Handle::Axis(axis) => format!("{} axis", axis.name()),
            Handle::Plane(axis) => {
                let (a, b) = axis.others();
                format!("{}{} plane", a.name(), b.name())
            }
            Handle::Ring(axis) => format!("{} ring", axis.name()),
            Handle::Centre => "centre".to_string(),
        }
    }
}

/// How a pose is held to a grid.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Snap {
    /// The step of a position in metres, none for no grid.
    pub grid: Option<f32>,
    /// The step of a turn in degrees.
    pub degrees: Option<f32>,
    /// The step of a scale.
    pub scale: Option<f32>,
}

fn snapped(value: f32, step: Option<f32>) -> f32 {
    match step {
        Some(step) if step > f32::EPSILON => (value / step).round() * step,
        _ => value,
    }
}

/// The screen a viewport draws to: the matrix it draws with and the rectangle in pixels that it fills.
///
/// ⚠ The renderer's depth is reversed (1 at the near plane and 0 at the far one), so a ray runs from depth 1 towards depth 0.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub view_proj: Mat4,
    /// The top left corner of the viewport, in pixels.
    pub origin: Vec2,
    /// Its width and height, in pixels.
    pub size: Vec2,
}

/// A line from a point, in the direction of a unit vector.
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl View {
    /// Where `point` is on the screen, none behind the eye.
    pub fn project(&self, point: Vec3) -> Option<Vec2> {
        let clip = self.view_proj * point.extend(1.0);
        if clip.w <= f32::EPSILON {
            return None;
        }
        let ndc = Vec2::new(clip.x, clip.y) / clip.w;
        let px = Vec2::new((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5) * self.size + self.origin;
        px.is_finite().then_some(px)
    }

    /// The ray through the pixel `at`.
    pub fn ray(&self, at: Vec2) -> Option<Ray> {
        let uv = (at - self.origin) / self.size;
        let ndc = Vec2::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let inverse = self.view_proj.inverse();
        let unproject = |depth: f32| {
            let p: Vec4 = inverse * Vec4::new(ndc.x, ndc.y, depth, 1.0);
            (p.w.abs() > f32::EPSILON).then(|| p.truncate() / p.w)
        };
        // A depth short of the far plane's, which an infinite far plane has no point at.
        let near = unproject(1.0)?;
        let far = unproject(0.001)?;
        let dir = (far - near).try_normalize()?;
        near.is_finite().then_some(Ray { origin: near, dir })
    }

    /// How many world units one pixel spans at `point`.
    pub fn world_per_pixel(&self, point: Vec3) -> Option<f32> {
        let here = self.project(point)?;
        let a = self.ray(here)?;
        let b = self.ray(here + Vec2::X)?;
        // The two rays through neighbouring pixels, cut by the plane across the first at the point's depth.
        let pa = a.origin + a.dir * (point - a.origin).dot(a.dir);
        let facing = b.dir.dot(a.dir);
        if facing.abs() < 1e-6 {
            return None;
        }
        let pb = b.origin + b.dir * ((pa - b.origin).dot(a.dir) / facing);
        Some((pb - pa).length())
    }
}

/// The point of the line through `a` with direction `da` nearest the line through `b` with direction `db`, as how far along the first.
fn along_nearest(a: Vec3, da: Vec3, b: Vec3, db: Vec3) -> Option<f32> {
    let w = a - b;
    let (aa, ab, bb) = (da.dot(da), da.dot(db), db.dot(db));
    let (d, e) = (da.dot(w), db.dot(w));
    let denominator = aa * bb - ab * ab;
    (denominator.abs() > 1e-8).then(|| (ab * e - bb * d) / denominator)
}

/// Where the ray meets the plane through `point` with normal `normal`.
fn meets_plane(ray: &Ray, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let facing = ray.dir.dot(normal);
    if facing.abs() < 1e-6 {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / facing;
    (t >= 0.0).then(|| ray.origin + ray.dir * t)
}

/// The manipulator at a point: how big it draws and which way its axes point.
#[derive(Debug, Clone, Copy)]
pub struct Gizmo {
    pub mode: Mode,
    pub origin: Vec3,
    /// How the axes are turned: the identity for the world's, an entity's rotation for its own.
    pub frame: Quat,
    /// The reach of an arrow in world units.
    pub reach: f32,
}

/// How many pixels an arrow reaches, whatever the distance.
pub const REACH_PIXELS: f32 = 96.0;
/// How near in pixels the pointer has to be to take a handle.
pub const GRAB_PIXELS: f32 = 9.0;
const RING_STEPS: usize = 48;
const PLANE_FRACTION: f32 = 0.3;

impl Gizmo {
    /// A manipulator at `origin` that draws `REACH_PIXELS` long in `view`, none when `origin` is off the screen.
    pub fn at(mode: Mode, origin: Vec3, frame: Quat, view: &View) -> Option<Self> {
        let per_pixel = view.world_per_pixel(origin)?;
        let reach = per_pixel * REACH_PIXELS;
        (reach.is_finite() && reach > 0.0).then_some(Self {
            mode,
            origin,
            frame,
            reach,
        })
    }

    fn axis(&self, axis: Axis) -> Vec3 {
        self.frame * axis.direction()
    }

    /// The segments of the handles as pairs of points, each with the handle it belongs to, for a line renderer; a ring is a polyline cut into segments.
    pub fn lines(&self) -> Vec<(Handle, [Vec3; 2])> {
        let mut out = Vec::new();
        match self.mode {
            Mode::Move | Mode::Scale => {
                for axis in Axis::ALL {
                    let end = self.origin + self.axis(axis) * self.reach;
                    out.push((Handle::Axis(axis), [self.origin, end]));
                    // An arrow head for a move, a cube on the tip for a scale.
                    let (u, v) = {
                        let (a, b) = axis.others();
                        (self.axis(a), self.axis(b))
                    };
                    let head = self.reach * 0.1;
                    let back = end - self.axis(axis) * head * 2.0;
                    if self.mode == Mode::Move {
                        for side in [u, -u, v, -v] {
                            out.push((Handle::Axis(axis), [end, back + side * head]));
                        }
                    } else {
                        let corners = [
                            end + (u + v) * head,
                            end + (u - v) * head,
                            end + (-u - v) * head,
                            end + (-u + v) * head,
                        ];
                        for at in 0..4 {
                            out.push((Handle::Axis(axis), [corners[at], corners[(at + 1) % 4]]));
                        }
                    }
                    if self.mode == Mode::Move {
                        let (a, b) = axis.others();
                        let near = PLANE_FRACTION * self.reach;
                        let (pa, pb) = (self.axis(a) * near, self.axis(b) * near);
                        let square = [self.origin + pa, self.origin + pa + pb, self.origin + pb];
                        out.push((Handle::Plane(axis), [square[0], square[1]]));
                        out.push((Handle::Plane(axis), [square[1], square[2]]));
                    }
                }
                if self.mode == Mode::Scale {
                    let c = self.reach * 0.06;
                    for axis in Axis::ALL {
                        let d = self.axis(axis) * c;
                        out.push((Handle::Centre, [self.origin - d, self.origin + d]));
                    }
                }
            }
            Mode::Rotate => {
                for axis in Axis::ALL {
                    let (a, b) = axis.others();
                    let (ua, ub) = (self.axis(a), self.axis(b));
                    let ring = |step: usize| {
                        let angle = step as f32 / RING_STEPS as f32 * std::f32::consts::TAU;
                        self.origin + (ua * angle.cos() + ub * angle.sin()) * self.reach * 0.8
                    };
                    for step in 0..RING_STEPS {
                        out.push((Handle::Ring(axis), [ring(step), ring(step + 1)]));
                    }
                }
            }
        }
        out
    }

    /// The handle nearest the pixel `at` within [`GRAB_PIXELS`], none when the pointer is over nothing of the manipulator.
    pub fn pick(&self, view: &View, at: Vec2) -> Option<Handle> {
        let mut best: Option<(f32, Handle)> = None;
        for (handle, [a, b]) in self.lines() {
            let (Some(pa), Some(pb)) = (view.project(a), view.project(b)) else {
                continue;
            };
            let away = distance_to_segment(at, pa, pb);
            if away <= GRAB_PIXELS && best.is_none_or(|(d, _)| away < d) {
                best = Some((away, handle));
            }
        }
        // The centre of a scale takes the pointer over the arrows' roots.
        if self.mode == Mode::Scale {
            if let Some(centre) = view.project(self.origin) {
                if centre.distance(at) <= GRAB_PIXELS * 1.4 {
                    return Some(Handle::Centre);
                }
            }
        }
        best.map(|(_, handle)| handle)
    }
}

fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let length = ab.length_squared();
    if length < 1e-9 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / length).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// A drag that has begun on a handle: what the pose was, and where the pointer took it.
#[derive(Debug, Clone, Copy)]
pub struct Drag {
    pub handle: Handle,
    pub mode: Mode,
    start: Pose,
    gizmo: Gizmo,
    /// Where on the handle the pointer first was: a distance along an axis, a point on a plane, or an angle on a ring.
    grabbed: Grab,
}

#[derive(Debug, Clone, Copy)]
enum Grab {
    Along(f32),
    On(Vec3),
    Angle(f32),
    Distance(f32),
}

impl Drag {
    /// Begin dragging `handle` of `gizmo`, which stands at `start`, with the pointer at the ray `ray`; none when the ray never meets the handle's line or plane.
    pub fn begin(gizmo: Gizmo, handle: Handle, start: Pose, ray: &Ray) -> Option<Self> {
        let grabbed = match (gizmo.mode, handle) {
            (Mode::Move | Mode::Scale, Handle::Axis(axis)) => Grab::Along(along_nearest(
                gizmo.origin,
                gizmo.axis(axis),
                ray.origin,
                ray.dir,
            )?),
            (Mode::Move, Handle::Plane(axis)) => {
                Grab::On(meets_plane(ray, gizmo.origin, gizmo.axis(axis))?)
            }
            (Mode::Rotate, Handle::Ring(axis)) => Grab::Angle(angle_on_ring(&gizmo, axis, ray)?),
            (Mode::Scale, Handle::Centre) => {
                let facing = (gizmo.origin - ray.origin).normalize_or_zero();
                let hit = meets_plane(ray, gizmo.origin, -facing)?;
                Grab::Distance((hit - gizmo.origin).length().max(gizmo.reach * 0.1))
            }
            _ => return None,
        };
        Some(Self {
            handle,
            mode: gizmo.mode,
            start,
            gizmo,
            grabbed,
        })
    }

    /// The pose with the pointer at `ray`, held to `snap`.
    pub fn pose(&self, ray: &Ray, snap: Snap) -> Pose {
        let mut pose = self.start;
        match (self.mode, self.handle, self.grabbed) {
            (Mode::Move, Handle::Axis(axis), Grab::Along(from)) => {
                let direction = self.gizmo.axis(axis);
                if let Some(now) = along_nearest(self.gizmo.origin, direction, ray.origin, ray.dir)
                {
                    let mut moved = self.start.position + direction * (now - from);
                    // A step is held on the axis the arrow runs along, in the world's frame.
                    let held = snapped(moved.dot(direction), snap.grid);
                    moved += direction * (held - moved.dot(direction));
                    pose.position = moved;
                }
            }
            (Mode::Move, Handle::Plane(axis), Grab::On(from)) => {
                if let Some(hit) = meets_plane(ray, self.gizmo.origin, self.gizmo.axis(axis)) {
                    let mut moved = self.start.position + (hit - from);
                    let (a, b) = axis.others();
                    for along in [self.gizmo.axis(a), self.gizmo.axis(b)] {
                        let held = snapped(moved.dot(along), snap.grid);
                        moved += along * (held - moved.dot(along));
                    }
                    pose.position = moved;
                }
            }
            (Mode::Rotate, Handle::Ring(axis), Grab::Angle(from)) => {
                if let Some(now) = angle_on_ring(&self.gizmo, axis, ray) {
                    let mut turn = now - from;
                    if let Some(step) = snap.degrees.filter(|s| *s > f32::EPSILON) {
                        turn = (turn.to_degrees() / step).round() * step.to_radians();
                    }
                    let about = Quat::from_axis_angle(self.gizmo.axis(axis).normalize(), turn);
                    pose.rotation = (about * self.start.rotation).normalize();
                }
            }
            (Mode::Scale, Handle::Axis(axis), Grab::Along(from)) => {
                let direction = self.gizmo.axis(axis);
                if let Some(now) = along_nearest(self.gizmo.origin, direction, ray.origin, ray.dir)
                {
                    let factor = 1.0 + (now - from) / self.gizmo.reach;
                    match axis {
                        Axis::Z => pose.scale_z = scaled(self.start.scale_z, factor, snap),
                        Axis::X | Axis::Y => {
                            pose.scale_xy = scaled(self.start.scale_xy, factor, snap)
                        }
                    }
                }
            }
            (Mode::Scale, Handle::Centre, Grab::Distance(from)) => {
                let facing = (self.gizmo.origin - ray.origin).normalize_or_zero();
                if let Some(hit) = meets_plane(ray, self.gizmo.origin, -facing) {
                    let now = (hit - self.gizmo.origin)
                        .length()
                        .max(self.gizmo.reach * 0.1);
                    let factor = now / from;
                    pose.scale_xy = scaled(self.start.scale_xy, factor, snap);
                    pose.scale_z = scaled(self.start.scale_z, factor, snap);
                }
            }
            _ => {}
        }
        pose
    }
}

/// `value` times `factor`, held to the step of the scale and never to nought or below: a scale of nothing hides the entity and cannot be dragged back.
fn scaled(value: f32, factor: f32, snap: Snap) -> f32 {
    let wanted = snapped(value * factor, snap.scale);
    wanted.max(0.01)
}

/// The angle of the pointer's ray about `axis` of the gizmo, measured on the ring's plane.
fn angle_on_ring(gizmo: &Gizmo, axis: Axis, ray: &Ray) -> Option<f32> {
    let normal = gizmo.axis(axis);
    let hit = meets_plane(ray, gizmo.origin, normal)?;
    let (a, b) = axis.others();
    let from_centre = hit - gizmo.origin;
    Some(
        from_centre
            .dot(gizmo.axis(b))
            .atan2(from_centre.dot(gizmo.axis(a))),
    )
}

/// An entity's box in the world: where it is picked and where it stands on the ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Obb {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub min: Vec3,
    pub max: Vec3,
}

impl Obb {
    /// How far along `ray` it is met, none when it is not.
    pub fn meets(&self, ray: &Ray) -> Option<f32> {
        let inverse = self.rotation.conjugate();
        let to_local = |v: Vec3| (inverse * v) / self.scale;
        let origin = to_local(ray.origin - self.position);
        let dir = to_local(ray.dir);
        let (mut near, mut far) = (f32::NEG_INFINITY, f32::INFINITY);
        for axis in 0..3 {
            let (o, d, lo, hi) = (origin[axis], dir[axis], self.min[axis], self.max[axis]);
            if d.abs() < 1e-9 {
                if o < lo || o > hi {
                    return None;
                }
                continue;
            }
            let (t1, t2) = ((lo - o) / d, (hi - o) / d);
            near = near.max(t1.min(t2));
            far = far.min(t1.max(t2));
        }
        // The ray is not normalised in local space, so the distances are along the world ray all the same.
        (near <= far && far >= 0.0).then_some(near.max(0.0))
    }

    /// The eight corners in the world.
    pub fn corners(&self) -> [Vec3; 8] {
        crate::transform::placed_corners(
            self.min * self.scale,
            self.max * self.scale,
            self.rotation,
            self.position,
        )
    }

    /// The lowest and the highest point in the world.
    pub fn heights(&self) -> (f32, f32) {
        self.corners()
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), c| {
                (lo.min(c.z), hi.max(c.z))
            })
    }

    /// Whether the point `at` is over the box's footprint (its box in the horizontal plane).
    fn over(&self, at: Vec2) -> bool {
        let corners = self.corners();
        let (lo, hi) = corners.iter().fold(
            (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
            |(lo, hi), c| (lo.min(c.truncate()), hi.max(c.truncate())),
        );
        at.cmpge(lo).all() && at.cmple(hi).all()
    }
}

/// The nearest of `boxes` the ray meets: its index, and how far along.
pub fn pick_entity(boxes: &[Obb], ray: &Ray) -> Option<(usize, f32)> {
    boxes
        .iter()
        .enumerate()
        .filter_map(|(at, obb)| obb.meets(ray).map(|t| (at, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The height the entity `moving` (whose box is `obb`, standing at `at`) rests on: the top of the highest other box under it that is no higher than `from`, else `floor`.
///
/// ⚠ A box is held to its footprint in the horizontal plane, its extreme corners, so a turned box counts a little wider than it is.
pub fn ground_under(boxes: &[Obb], moving: usize, at: Vec2, from: f32, floor: f32) -> f32 {
    boxes
        .iter()
        .enumerate()
        .filter(|(i, obb)| *i != moving && obb.over(at))
        .map(|(_, obb)| obb.heights().1)
        .filter(|top| *top <= from + 0.05)
        .fold(floor, f32::max)
}

/// The pose with its entity stood on the ground: the lowest point of its box at the height it rests on.
pub fn drop_to_ground(boxes: &[Obb], moving: usize, pose: Pose, floor: f32) -> Pose {
    let Some(obb) = boxes.get(moving) else {
        return pose;
    };
    let here = Obb {
        position: pose.position,
        rotation: pose.rotation,
        scale: Vec3::new(pose.scale_xy, pose.scale_xy, pose.scale_z),
        ..*obb
    };
    let (bottom, _) = here.heights();
    let ground = ground_under(boxes, moving, pose.position.truncate(), bottom, floor);
    Pose {
        position: pose.position + Vec3::Z * (ground - bottom),
        ..pose
    }
}

/// The convex hull of the horizontal footprint of `points`, in order, for a plan view.
pub fn footprint(points: &[Vec3]) -> Vec<Vec2> {
    let mut flat: Vec<Vec2> = points.iter().map(|p| p.truncate()).collect();
    flat.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    flat.dedup();
    if flat.len() < 3 {
        return flat;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 {
            Box::new(flat.iter())
        } else {
            Box::new(flat.iter().rev().skip(1))
        };
        for p in iter {
            // The upper pass keeps the last point of the lower one, so it pops only past it.
            while hull.len() >= if pass == 0 { 2 } else { start + 1 }
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 0.0
            {
                hull.pop();
            }
            hull.push(*p);
        }
    }
    hull.pop();
    hull
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera 20 metres from the origin looking down the Y axis, as the viewport's matrices lay out.
    fn view() -> View {
        let eye = Vec3::new(0.0, -20.0, 0.0);
        let look = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Z);
        // Reverse-Z, as the renderer builds it (`OrbitCamera::view_proj`).
        let (fov, near, far) = (1.0_f32, 0.05_f32, 5000.0_f32);
        let f = 1.0 / (fov * 0.5).tan();
        let projection = Mat4::from_cols(
            Vec4::new(f, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, near / (far - near), -1.0),
            Vec4::new(0.0, 0.0, near * far / (far - near), 0.0),
        );
        View {
            view_proj: projection * look,
            origin: Vec2::ZERO,
            size: Vec2::new(800.0, 800.0),
        }
    }

    fn pose() -> Pose {
        Pose {
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
            scale_xy: 1.0,
            scale_z: 1.0,
        }
    }

    #[test]
    fn a_point_projects_to_the_pixel_a_ray_through_it_comes_back_by() {
        let view = view();
        let point = Vec3::new(2.0, 5.0, -1.0);
        let pixel = view.project(point).expect("in front of the eye");
        let ray = view.ray(pixel).expect("a ray");
        let nearest = ray.origin + ray.dir * (point - ray.origin).dot(ray.dir);
        assert!(nearest.distance(point) < 1e-2, "{nearest} vs {point}");
        assert!(
            view.project(Vec3::new(0.0, -40.0, 0.0)).is_none(),
            "behind the eye"
        );
    }

    #[test]
    fn the_manipulator_draws_the_same_length_on_the_screen_near_and_far() {
        let view = view();
        let near = Gizmo::at(
            Mode::Move,
            Vec3::new(0.0, -10.0, 0.0),
            Quat::IDENTITY,
            &view,
        )
        .unwrap();
        let far = Gizmo::at(Mode::Move, Vec3::new(0.0, 40.0, 0.0), Quat::IDENTITY, &view).unwrap();
        let on_screen = |g: &Gizmo| {
            let (a, b) = (g.origin, g.origin + Vec3::Z * g.reach);
            view.project(a).unwrap().distance(view.project(b).unwrap())
        };
        assert!(
            (on_screen(&near) - REACH_PIXELS).abs() < 2.0,
            "{}",
            on_screen(&near)
        );
        assert!(
            (on_screen(&far) - REACH_PIXELS).abs() < 2.0,
            "{}",
            on_screen(&far)
        );
        assert!(far.reach > near.reach * 3.0);
    }

    #[test]
    fn the_pointer_takes_the_handle_it_is_over_and_nothing_elsewhere() {
        let view = view();
        let gizmo = Gizmo::at(Mode::Move, Vec3::ZERO, Quat::IDENTITY, &view).unwrap();
        let tip = |axis: Axis| {
            view.project(gizmo.origin + axis.direction() * gizmo.reach * 0.6)
                .unwrap()
        };
        assert_eq!(gizmo.pick(&view, tip(Axis::X)), Some(Handle::Axis(Axis::X)));
        assert_eq!(gizmo.pick(&view, tip(Axis::Z)), Some(Handle::Axis(Axis::Z)));
        assert_eq!(gizmo.pick(&view, Vec2::new(700.0, 700.0)), None);
    }

    #[test]
    fn dragging_an_arrow_moves_along_its_axis_only_and_a_step_holds_it_to_the_grid() {
        let view = view();
        let start = pose();
        let gizmo = Gizmo::at(Mode::Move, start.position, Quat::IDENTITY, &view).unwrap();
        let grab = |p: Vec3| view.ray(view.project(p).unwrap()).unwrap();
        let from = gizmo.origin + Vec3::X * gizmo.reach * 0.5;
        let drag =
            Drag::begin(gizmo, Handle::Axis(Axis::X), start, &grab(from)).expect("on the arrow");
        let to = from + Vec3::X * 2.37;
        let moved = drag.pose(&grab(to), Snap::default());
        assert!(
            (moved.position - (start.position + Vec3::X * 2.37)).length() < 0.05,
            "{:?}",
            moved.position
        );
        assert_eq!(moved.position.y, start.position.y);
        assert_eq!(moved.position.z, start.position.z);

        let held = drag.pose(
            &grab(to),
            Snap {
                grid: Some(0.5),
                ..Snap::default()
            },
        );
        assert_eq!(
            held.position.x, 3.5,
            "the world x of 3.37 is held to the half metre"
        );
        assert_eq!(
            held.position.y, start.position.y,
            "the other axes stay where they were"
        );
    }

    #[test]
    fn dragging_a_ring_turns_about_its_axis_and_a_step_holds_the_angle() {
        let view = view();
        let start = pose();
        let gizmo = Gizmo::at(Mode::Rotate, start.position, Quat::IDENTITY, &view).unwrap();
        // The ring about Z lies in the XY plane; the camera looks along Y, so it is edge on, and grabbing it goes by its plane.
        let on = |angle: f32| {
            let p =
                gizmo.origin + (Vec3::X * angle.cos() + Vec3::Y * angle.sin()) * gizmo.reach * 0.8;
            view.ray(view.project(p).unwrap()).unwrap()
        };
        let drag =
            Drag::begin(gizmo, Handle::Ring(Axis::Z), start, &on(0.0)).expect("the plane is met");
        let turned = drag.pose(&on(0.5), Snap::default());
        let (_, _, z) = turned.rotation.to_euler(glam::EulerRot::XYZ);
        assert!(
            (z - 0.5).abs() < 0.2,
            "turned about Z by roughly the angle the pointer went: {z}"
        );
        let held = drag.pose(
            &on(0.5),
            Snap {
                degrees: Some(15.0),
                ..Snap::default()
            },
        );
        let (_, _, z) = held.rotation.to_euler(glam::EulerRot::XYZ);
        let steps = z.to_degrees() / 15.0;
        assert!((steps - steps.round()).abs() < 1e-3, "{steps}");
    }

    #[test]
    fn a_scale_is_two_numbers_and_never_reaches_nought() {
        let view = view();
        let start = pose();
        let gizmo = Gizmo::at(Mode::Scale, start.position, Quat::IDENTITY, &view).unwrap();
        let grab = |p: Vec3| view.ray(view.project(p).unwrap()).unwrap();
        let tip = gizmo.origin + Vec3::Z * gizmo.reach;
        let drag = Drag::begin(gizmo, Handle::Axis(Axis::Z), start, &grab(tip)).unwrap();
        let taller = drag.pose(&grab(tip + Vec3::Z * gizmo.reach), Snap::default());
        assert!(taller.scale_z > 1.8, "{}", taller.scale_z);
        assert_eq!(
            taller.scale_xy, 1.0,
            "the vertical handle leaves the horizontal scale"
        );
        let crushed = drag.pose(&grab(tip - Vec3::Z * gizmo.reach * 3.0), Snap::default());
        assert!(crushed.scale_z >= 0.01 && crushed.scale_z.is_finite());
    }

    fn crate_at(position: Vec3) -> Obb {
        Obb {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        }
    }

    #[test]
    fn a_ray_picks_the_nearest_box_it_goes_through_and_a_miss_picks_nothing() {
        let boxes = [
            crate_at(Vec3::new(0.0, 5.0, 0.0)),
            crate_at(Vec3::new(0.0, 12.0, 0.0)),
            crate_at(Vec3::new(9.0, 5.0, 0.0)),
        ];
        let ray = Ray {
            origin: Vec3::new(0.0, -20.0, 0.0),
            dir: Vec3::Y,
        };
        let (at, t) = pick_entity(&boxes, &ray).expect("hits");
        assert_eq!(at, 0);
        assert!((t - 24.0).abs() < 1e-3, "{t}");
        let miss = Ray {
            origin: Vec3::new(0.0, -20.0, 5.0),
            dir: Vec3::Y,
        };
        assert!(pick_entity(&boxes, &miss).is_none());
    }

    #[test]
    fn a_turned_and_scaled_box_is_picked_where_it_is_and_not_where_its_unturned_box_was() {
        let mut long = crate_at(Vec3::ZERO);
        long.scale = Vec3::new(4.0, 1.0, 1.0);
        long.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        // Turned a quarter, its long side runs along Y.
        let along_y = Ray {
            origin: Vec3::new(0.0, -10.0, 0.0),
            dir: Vec3::Y,
        };
        assert!(long.meets(&along_y).is_some());
        let off = Ray {
            origin: Vec3::new(3.0, -10.0, 0.0),
            dir: Vec3::Y,
        };
        assert!(
            long.meets(&off).is_none(),
            "x=3 is past a turned box one metre wide"
        );
        let up = Ray {
            origin: Vec3::new(0.0, 3.0, -10.0),
            dir: Vec3::Z,
        };
        assert!(
            long.meets(&up).is_some(),
            "y=3 is inside the four metres it reaches along Y"
        );
    }

    #[test]
    fn an_entity_is_dropped_onto_the_highest_box_under_it_or_the_floor() {
        let floor_box = crate_at(Vec3::new(0.0, 0.0, 0.0)); // top at 1
        let moving = crate_at(Vec3::new(0.0, 0.0, 9.0));
        let boxes = [floor_box, moving];
        let on_box = drop_to_ground(
            &boxes,
            1,
            Pose {
                position: Vec3::new(0.5, 0.0, 9.0),
                rotation: Quat::IDENTITY,
                scale_xy: 1.0,
                scale_z: 1.0,
            },
            -3.0,
        );
        assert!(
            (on_box.position.z - 2.0).abs() < 1e-4,
            "bottom -1 of its centre sits on the top at 1: {}",
            on_box.position.z
        );
        let beside = drop_to_ground(
            &boxes,
            1,
            Pose {
                position: Vec3::new(30.0, 0.0, 9.0),
                rotation: Quat::IDENTITY,
                scale_xy: 1.0,
                scale_z: 1.0,
            },
            -3.0,
        );
        assert!(
            (beside.position.z - (-2.0)).abs() < 1e-4,
            "on the floor at -3: {}",
            beside.position.z
        );
    }

    #[test]
    fn the_footprint_of_a_turned_box_is_its_four_corners_in_order() {
        let obb = Obb {
            rotation: Quat::from_rotation_z(0.7),
            ..crate_at(Vec3::new(10.0, 0.0, 0.0))
        };
        let hull = footprint(&obb.corners());
        assert_eq!(hull.len(), 4, "{hull:?}");
        let area: f32 = hull
            .iter()
            .zip(hull.iter().cycle().skip(1))
            .map(|(a, b)| a.perp_dot(*b))
            .sum::<f32>()
            / 2.0;
        assert!((area.abs() - 4.0).abs() < 1e-3, "two metres square: {area}");
    }

    #[test]
    fn the_footprint_of_an_upright_box_is_its_four_corners_too() {
        let hull = footprint(&crate_at(Vec3::new(10.0, 20.0, 0.0)).corners());
        assert_eq!(hull.len(), 4, "{hull:?}");
    }
}
