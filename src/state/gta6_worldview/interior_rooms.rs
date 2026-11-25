//! Which rooms and portals of an interior a camera sees, in the interior's own space: the room it
//! stands in and the rooms seen through its portals, each portal clipped to the opening the camera
//! looks through to reach it; from outside, the rooms seen through the portals to the outside.
//!
//! ⚠ Room 0 is the outside of every other room: a portal to it is a door or window of the shell.

use glam::Vec3;
use or_io::gta6::timecycle::ARTIFICIAL_AMBIENT_LUX;
use or_world::streaming_rules::{
    portal_is_shut, portal_leads_outside, PortalDoor, PORTAL_LINK, PORTAL_MIRROR,
};

/// How near a portal's plane the camera may stand and still look through the whole of it, in metres.
const DOORWAY: f32 = 0.1;
/// How many portals deep a camera looks.
const DEPTH: usize = 8;
/// How many rooms a walk enters, along all its paths, before it gives up and takes every room as seen.
// ⚠ The paths a walk follows grow as the portals' count to the power of `DEPTH`: rooms whose
// doors all open where the camera stands would hold the world's thread for hours.
const MOST_ENTERED: usize = 1 << 14;

/// A room's box, in the interior's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RoomBox {
    pub(super) min: Vec3,
    pub(super) max: Vec3,
}

impl RoomBox {
    fn holds(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    fn volume(&self) -> f32 {
        (self.max - self.min).max(Vec3::ZERO).element_product()
    }
}

/// A portal between two rooms, with its four corners in the interior's space and its flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Opening {
    pub(super) rooms: [u32; 2],
    pub(super) corners: [Vec3; 4],
    pub(super) flags: u32,
    /// Whether it is shut as the interior stands at rest: its own entities are seen, nothing past it.
    pub(super) shut: bool,
    /// Whether a door the map places stands in it: the sky does not come in through it.
    pub(super) door: bool,
}

/// Whether a portal of `flags` that holds `doors` doors is shut as the interior stands when it is
/// placed: one that may close and holds a door is, whatever rooms it joins; one that holds none, or
/// a link, whose doors never reach it, is open.
// ⚠ Inferred: a placed door stands at rest unopened, and the placed interior's proxy is set.
// ponytail: no door keeps its portal open by its own flags or its archetype's, which the viewer does not read.
pub(super) fn shut_at_rest(flags: u32, doors: usize) -> bool {
    let at_rest = PortalDoor {
        open_ratio: 0.0,
        keeps_open: false,
    };
    let doors = if flags & PORTAL_LINK != 0 { 0 } else { doors };
    portal_is_shut(flags, true, &vec![at_rest; doors])
}

/// What a camera sees of an interior: by room and by portal; from a room, the pyramids through the
/// portals to the outside it sees the outside through, `None` where nothing of the outside is hidden.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Seen {
    pub(super) rooms: Vec<bool>,
    pub(super) portals: Vec<bool>,
    pub(super) outside: Option<Vec<Planes>>,
}

/// Whether the sphere of `radius` about `p` reaches into the pyramid of `planes` through `eye`.
pub(super) fn reaches(planes: &[Vec3], eye: Vec3, p: Vec3, radius: f32) -> bool {
    planes.iter().all(|normal| normal.dot(p - eye) >= -radius)
}

/// The room other than room 0 whose box holds `eye`, the smallest where boxes overlap.
pub(super) fn room_of(rooms: &[RoomBox], eye: Vec3) -> Option<usize> {
    (rooms.iter().enumerate().skip(1))
        .filter(|(_, room)| room.holds(eye))
        .min_by(|(_, a), (_, b)| a.volume().total_cmp(&b.volume()))
        .map(|(k, _)| k)
}

/// The smallest room other than room 0 whose box holds `p`, the middle of a sphere of `radius`
/// smaller than the box's half diagonal.
// ⚠ Measured: a prop on a shelf or against a wall has a sphere that passes the room's box, and lit
// as outdoors it stood four stops and more over the room. Inferred: a shell's piece is wider than
// the room it walls.
pub(super) fn room_around(rooms: &[RoomBox], p: Vec3, radius: f32) -> Option<usize> {
    (rooms.iter().enumerate().skip(1))
        .filter(|(_, room)| room.holds(p) && 2.0 * radius < (room.max - room.min).length())
        .min_by(|(_, a), (_, b)| a.volume().total_cmp(&b.volume()))
        .map(|(k, _)| k)
}

/// Planes through the camera, each a unit normal: a point `p` is inside where `normal · (p − eye) ≥ 0` for each.
pub(super) type Planes = Vec<Vec3>;

/// The part of `polygon` inside every one of `planes` through `eye`.
fn clip(mut polygon: Vec<Vec3>, eye: Vec3, planes: &[Vec3]) -> Vec<Vec3> {
    for normal in planes {
        if polygon.len() < 3 {
            break;
        }
        let side = |p: Vec3| normal.dot(p - eye);
        let mut out = Vec::with_capacity(polygon.len() + 1);
        for (k, &a) in polygon.iter().enumerate() {
            let b = polygon[(k + 1) % polygon.len()];
            let (sa, sb) = (side(a), side(b));
            if sa >= 0.0 {
                out.push(a);
            }
            if (sa >= 0.0) != (sb >= 0.0) {
                out.push(a + (b - a) * (sa / (sa - sb)));
            }
        }
        polygon = out;
    }
    polygon
}

/// The planes through `eye` and each edge of `polygon`, each facing its middle.
fn pyramid(eye: Vec3, polygon: &[Vec3]) -> Planes {
    let middle = polygon.iter().copied().sum::<Vec3>() / polygon.len() as f32;
    (0..polygon.len())
        .filter_map(|k| {
            let (a, b) = (polygon[k] - eye, polygon[(k + 1) % polygon.len()] - eye);
            let normal = a.cross(b).normalize_or_zero();
            match normal.dot(middle - eye) {
                d if d > 0.0 => Some(normal),
                d if d < 0.0 => Some(-normal),
                _ => None,
            }
        })
        .collect()
}

/// The plane of an opening, as a unit normal and a point on it; `None` for one with no area.
fn plane_of(corners: &[Vec3; 4]) -> Option<(Vec3, Vec3)> {
    let normal = (corners[2] - corners[0]).cross(corners[3] - corners[1]);
    let normal = normal.try_normalize()?;
    Some((normal, corners.iter().copied().sum::<Vec3>() / 4.0))
}

struct Walk<'a> {
    eye: Vec3,
    portals: &'a [Opening],
    /// The middle of the interior's box: the side of a shell's portal its rooms are on.
    middle: Vec3,
    seen: Seen,
    path: Vec<u32>,
    /// How many more rooms it may enter.
    left: usize,
    /// The openings each portal was crossed through, the way into its first room and the way out.
    crossed: Vec<[Vec<Vec<Vec3>>; 2]>,
}

/// Whether every corner of `inner` lies within the convex `outer`, both in a plane facing `normal`.
fn within(inner: &[Vec3], outer: &[Vec3], normal: Vec3) -> bool {
    const SLACK: f32 = 1e-4;
    let turn = |a: Vec3, b: Vec3, p: Vec3| normal.dot((b - a).cross(p - a));
    let &[a, b, c, ..] = outer else {
        return false;
    };
    let wise = turn(a, b, c).signum();
    inner.iter().all(|&p| {
        (0..outer.len()).all(|k| wise * turn(outer[k], outer[(k + 1) % outer.len()], p) >= -SLACK)
    })
}

impl Walk<'_> {
    fn enter(&mut self, room: u32, planes: &[Vec3]) {
        if let Some(slot) = self.seen.rooms.get_mut(room as usize) {
            *slot = true;
        }
        self.left = self.left.saturating_sub(1);
        if self.path.len() >= DEPTH || self.left == 0 {
            return;
        }
        self.path.push(room);
        for (k, portal) in self.portals.iter().enumerate() {
            let other = match portal.rooms {
                [a, b] if a == room && b != room => b,
                [a, b] if b == room && a != room => a,
                _ => continue,
            };
            // A mirror is seen in the room it hangs in, and nothing is seen through it.
            if portal.flags & PORTAL_MIRROR != 0 {
                self.seen.portals[k] = true;
                continue;
            }
            if self.path.contains(&other) {
                continue;
            }
            let Some((normal, point)) = plane_of(&portal.corners) else {
                continue;
            };
            let off = normal.dot(self.eye - point);
            // From outside, a shell's portal is looked into only from the side away from its rooms.
            if room == 0 && off * normal.dot(self.middle - point) > 0.0 {
                continue;
            }
            let opening = clip(portal.corners.to_vec(), self.eye, planes);
            let doorway = off.abs() < DOORWAY;
            if opening.len() < 3 && !doorway {
                continue;
            }
            let next = match doorway {
                // Standing in the opening, the camera sees through all of it what it saw before.
                true => planes.to_vec(),
                false => pyramid(self.eye, &opening),
            };
            self.seen.portals[k] = true;
            // A shut portal's door is seen, and nothing past it.
            if portal.shut {
                continue;
            }
            // ⚠ Through an opening within one this portal was crossed through the same way, the eye
            // sees nothing it did not: the walk would otherwise follow every order of the same doors.
            let way = &mut self.crossed[k][usize::from(other == portal.rooms[0])];
            if opening.len() >= 3 {
                if way.iter().any(|seen| within(&opening, seen, normal)) {
                    continue;
                }
                way.push(opening);
            }
            // ⚠ GTA V's rule: from a room the outside is drawn through the portals to it the camera sees.
            let outside = self.seen.outside.as_mut();
            let leads_out = other == 0 && portal_leads_outside(room, other, portal.flags);
            if let Some(outside) = outside.filter(|_| leads_out) {
                outside.push(next.clone());
            }
            self.enter(other, &next);
        }
        self.path.pop();
    }
}

/// How far into the light of an opening `p` stands, 1 within the ellipse of its half sizes about
/// its middle and nought past 1.25 times it, in the planes across the opening along its upright
/// side and along its level one.
fn feather(corners: &[Vec3; 4], p: Vec3) -> f32 {
    let (a, b) = (corners[0] - corners[1], corners[1] - corners[2]);
    let (upright, level) = match a.z.abs() > b.z.abs() {
        true => (a, b),
        false => (b, a),
    };
    let (Some(normal), Some(u1), Some(u2)) = (
        upright.cross(level).try_normalize(),
        upright.try_normalize(),
        level.try_normalize(),
    ) else {
        return 0.0;
    };
    let to = p - (corners[0] + corners[2]) / 2.0;
    let across = normal.dot(to) * normal;
    let fade = |d: f32, r: f32| 1.0 - ((d - r) / (r * (FEATHER * FEATHER - 1.0))).clamp(0.0, 1.0);
    let along = across + u1.dot(to) * u1;
    let facing = normal.dot(along.normalize_or_zero()).clamp(0.0, 1.0);
    // Squares of lengths throughout: the half diagonal straight ahead, the shorter half side edge on.
    let diagonal = (corners[0] - corners[2]).length_squared() / 4.0;
    let shortest = upright.length_squared().min(level.length_squared()) / 4.0;
    let inner = diagonal + (shortest - diagonal) * facing;
    if along.length_squared() > inner * FEATHER * FEATHER {
        return 0.0;
    }
    let sideways = across + u2.dot(to) * u2;
    fade(along.length_squared(), inner)
        * fade(sideways.length_squared(), level.length_squared() / 4.0)
}

/// How far past an opening's half size its light fades out.
const FEATHER: f32 = 1.25;

/// The visible sky angle, in degrees, of an opening the daylight factor takes: an unobstructed one.
const SKY_ANGLE: f32 = 90.0;
/// The mean reflectance of a room's surfaces the daylight factor takes.
const REFLECTANCE: f32 = 0.5;

/// How much of the natural and of the artificial ambient light what stands at `p` in room `room`
/// of `rooms` takes: of the sky's, the room's average daylight factor over its openings to the
/// outside, rising to the whole of it in an opening; of the artificial, the light its lamps of
/// `flux` lumens send round it.
// ⚠ Inferred: the build states no ambient scale for a room (its light inside is traced through the
// openings), so the architect's average daylight factor, `W θ / (A (1 − R²))` for openings of area
// W in a room of surface A, stands for what its tracing brings in. It states no artificial ambient
// light either, and the lighting keeps as much of the sky's reflection as the greater of the two.
pub(super) fn ambient_of(
    rooms: &[RoomBox],
    portals: &[Opening],
    room: u32,
    p: Vec3,
    flux: f32,
) -> [f32; 2] {
    let Some(of) = rooms.get(room as usize).filter(|_| room != 0) else {
        return or_renderer::OUTDOORS;
    };
    let outward = portals.iter().filter(|o| {
        let [a, b] = o.rooms;
        o.rooms.contains(&room)
            && portal_leads_outside(a, b, o.flags)
            && o.flags & PORTAL_MIRROR == 0
            && !o.shut
    });
    let (mut window, mut near) = (0.0, 0.0);
    for o in outward {
        let c = &o.corners;
        // ⚠ A door in an opening stands in its light, which the room past it does not take.
        near += feather(c, p);
        if !o.door {
            window += (c[2] - c[0]).cross(c[3] - c[1]).length() / 2.0;
        }
    }
    let size = (of.max - of.min).max(Vec3::ZERO);
    let surface = 2.0 * (size.x * size.y + size.x * size.z + size.y * size.z);
    if surface <= 0.0 {
        return [near.clamp(0.0, 1.0), 0.0];
    }
    let factor = window * SKY_ANGLE / (100.0 * surface * (1.0 - REFLECTANCE * REFLECTANCE));
    let factor = factor.clamp(0.0, 1.0);
    // ⚠ Inferred: the build traces the light a room's lamps bounce off its surfaces and states none;
    // the textbook interreflected illuminance `Φ R / (A (1 − R))` stands for it.
    let bounced = flux * REFLECTANCE / (surface * (1.0 - REFLECTANCE));
    [
        factor + (1.0 - factor) * near.clamp(0.0, 1.0),
        (bounced / ARTIFICIAL_AMBIENT_LUX).clamp(0.0, 1.0),
    ]
}

/// What a camera at `eye` sees of an interior of `rooms` and `portals` whose box's middle is
/// `middle`: from the room it stands in, else from outside.
pub(super) fn seen(rooms: &[RoomBox], portals: &[Opening], middle: Vec3, eye: Vec3) -> Seen {
    let mut walk = Walk {
        eye,
        portals,
        middle,
        seen: Seen {
            rooms: vec![false; rooms.len()],
            portals: vec![false; portals.len()],
            outside: None,
        },
        path: Vec::new(),
        left: MOST_ENTERED,
        crossed: vec![Default::default(); portals.len()],
    };
    let start = room_of(rooms, eye).unwrap_or(0) as u32;
    if start != 0 {
        walk.seen.outside = Some(Vec::new());
    }
    walk.enter(start, &[]);
    if walk.left == 0 {
        // A walk cut short has not seen what it left out: it hides nothing.
        walk.seen.rooms.fill(true);
        walk.seen.portals.fill(true);
        walk.seen.outside = None;
    }
    walk.seen
}

#[cfg(test)]
#[path = "interior_rooms_tests.rs"]
mod tests;
