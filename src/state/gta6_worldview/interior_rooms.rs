//! Which rooms and portals of the interiors around a camera it sees, by the build's walk: from the
//! room it stands in through the open portals and the links into the interiors they join; from
//! outside, each interior through its portals to the outside; and the light a room's openings and
//! lamps give what stands in it, in the interior's own space.
//!
//! ⚠ Room 0 is the outside of every other room: a portal to it is a door or window of the shell.

use glam::{Mat4, Vec3};
use or_io::gta6::timecycle::ARTIFICIAL_AMBIENT_LUX;
use or_world::interior_portals::{self as rules, LinkTarget, RoomRef};
use or_world::streaming_rules::{
    camera_portal_factor, portal_is_shut, portal_leads_outside, uses_interior_ambient, PortalDoor,
    PORTAL_LINK, PORTAL_MIRROR,
};

/// How many portals deep a camera looks.
const DEPTH: u32 = 8;
/// How near the middles of two portals stand for a link of one to lead through the other, in metres.
// ⚠ Inferred: the build's walk resolves a link to the other instance and its portal to the outside;
// how it finds them was not read, and a link and the portal it meets stand in one place.
const LINKED: f32 = 0.5;

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

/// What a camera sees of an interior: by room and by portal; from a room, the pyramids in the world
/// through the portals to the outside it sees the outside through, `None` where nothing of the
/// outside is hidden.
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
pub(super) fn plane_of(corners: &[Vec3; 4]) -> Option<(Vec3, Vec3)> {
    let normal = (corners[2] - corners[0]).cross(corners[3] - corners[1]);
    let normal = normal.try_normalize()?;
    Some((normal, corners.iter().copied().sum::<Vec3>() / 4.0))
}

/// An interior a camera may see into: its rooms and portals in its own space, and what takes a
/// point of the world into that space.
pub(super) struct Shell<'a> {
    pub(super) rooms: &'a [RoomBox],
    pub(super) portals: &'a [Opening],
    pub(super) own: Mat4,
}

/// What a camera sees of the shells around it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Walked {
    /// By shell: the rooms and portals it sees; from a room, the pyramids in the world the outside is seen through.
    pub(super) seen: Vec<Seen>,
    /// The shell and room the camera stands in.
    pub(super) standing: Option<(usize, usize)>,
    /// The room the camera stands in, or, where the factor takes the interior's light, the room past the nearest portal to the outside.
    pub(super) inside: Option<(usize, usize)>,
}

/// `shell` as the build's walk takes it: its portals in the world, each room's portals, its box in the world.
fn walked(shell: &Shell<'_>) -> rules::Interior {
    let world = shell.own.inverse();
    let portals = (shell.portals.iter())
        .map(|o| rules::Portal {
            def: rules::PortalDef {
                room_a: o.rooms[0],
                room_b: o.rooms[1],
                flags: o.flags,
                opacity: 0,
                corners: o.corners.map(|c| world.transform_point3(c)).to_vec(),
            },
            shut: o.shut,
            link: None,
        })
        .collect();
    let mut rooms = vec![Vec::new(); shell.rooms.len()];
    for (k, o) in shell.portals.iter().enumerate() {
        let [a, b] = o.rooms;
        for room in if a == b { vec![a] } else { vec![a, b] } {
            if let Some(list) = rooms.get_mut(room as usize) {
                list.push(k);
            }
        }
    }
    let (mut least, mut most) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for room in shell.rooms {
        for k in 0..8 {
            let at = |axis: usize| match k >> axis & 1 {
                0 => room.min[axis],
                _ => room.max[axis],
            };
            let p = world.transform_point3(Vec3::new(at(0), at(1), at(2)));
            (least, most) = (least.min(p), most.max(p));
        }
    }
    rules::Interior {
        portals,
        rooms,
        // ⚠ What the viewer draws the contents of is what the build calls populated.
        proxy_state: rules::POPULATED_PROXY_STATE,
        bounds_min: least,
        bounds_max: most,
    }
}

/// The middle of a portal's corners.
fn middle(def: &rules::PortalDef) -> Vec3 {
    def.corners.iter().copied().sum::<Vec3>() / def.corners.len().max(1) as f32
}

/// Join each link portal of `interiors` to the portal to the outside of another interior that stands where it does.
fn link(interiors: &mut [rules::Interior]) {
    let outward: Vec<(usize, usize, Vec3)> = (interiors.iter().enumerate())
        .flat_map(|(i, interior)| {
            let exterior = interior.rooms.first().map_or(&[][..], Vec::as_slice);
            (exterior.iter().enumerate()).filter_map(move |(slot, &k)| {
                Some((i, slot, middle(&interior.portals.get(k)?.def)))
            })
        })
        .collect();
    for (i, interior) in interiors.iter_mut().enumerate() {
        for portal in interior.portals.iter_mut().filter(|p| p.def.is_link()) {
            let at = middle(&portal.def);
            portal.link = (outward.iter())
                .find(|(j, _, there)| *j != i && there.distance(at) <= LINKED)
                .map(|&(interior, slot, _)| LinkTarget { interior, slot });
        }
    }
}

/// What a camera at `eye` sees of `shells`: from the room it stands in, through its open portals
/// and the links into the shells they join; each other shell from outside, through its open
/// portals to the outside; all of it with no test of where the camera looks.
// ponytail: the world's thread is told where the camera stands and not where it looks, so each
// portal passes the frustum's test, as though the camera looked every way; narrow it with the
// view's planes if the rooms drawn cost.
pub(super) fn walk(shells: &[Shell<'_>], eye: Vec3) -> Walked {
    let mut interiors: Vec<rules::Interior> = shells.iter().map(walked).collect();
    link(&mut interiors);
    let standing = (shells.iter().enumerate())
        .find_map(|(i, s)| Some((i, room_of(s.rooms, s.own.transform_point3(eye))?)));
    let mut seen: Vec<Seen> = (shells.iter())
        .map(|s| Seen {
            rooms: vec![false; s.rooms.len()],
            portals: vec![false; s.portals.len()],
            outside: None,
        })
        .collect();
    for i in 0..shells.len() {
        let room = match standing {
            Some((j, room)) if j == i => room,
            _ => 0,
        };
        let start = RoomRef { interior: i, room };
        let found = rules::visible_rooms(&interiors, start, &[], DEPTH);
        // ⚠ Inferred: the walk ends at the outside, and the build's scan of every shell's portals from the outside, its own among them, is a start of its own.
        let again = (room != 0 && found.exterior_seen).then(|| {
            let outside = RoomRef {
                interior: i,
                room: 0,
            };
            rules::visible_rooms(&interiors, outside, &[], DEPTH)
        });
        let rooms = found
            .rooms
            .iter()
            .chain(again.iter().flat_map(|v| &v.rooms));
        // The outside of an interior is seen through each of its portals to the outside the walk reaches.
        let outward = (found.exterior_portals.iter()).map(|p| (p.interior, 0));
        for (at, room) in rooms.map(|r| (r.interior, r.room)).chain(outward) {
            if let Some(slot) = seen.get_mut(at).and_then(|s| s.rooms.get_mut(room)) {
                *slot = true;
            }
        }
        if room != 0 {
            // ⚠ GTA V's rule: from a room the outside is drawn through the portals to it the camera sees.
            let through = (found.exterior_portals.iter())
                .filter_map(|p| interiors.get(p.interior)?.portals.get(p.portal))
                .map(|p| pyramid(eye, &p.def.corners))
                .collect();
            seen[i].outside = Some(through);
        }
    }
    // What a portal holds is seen with either room it joins.
    for (s, shell) in seen.iter_mut().zip(shells) {
        for (k, o) in shell.portals.iter().enumerate() {
            s.portals[k] = (o.rooms.iter()).any(|r| s.rooms.get(*r as usize) == Some(&true));
        }
    }
    let distances = rules::exterior_portal_distances(eye, &interiors);
    let factor = camera_portal_factor(standing.is_some(), distances);
    let inside = standing.or_else(|| {
        // Where the factor takes the interior's light, the room past the nearest portal to the outside.
        let nearest = (interiors.iter().enumerate())
            .flat_map(|(i, interior)| {
                let exterior = interior.rooms.first().map_or(&[][..], Vec::as_slice);
                (exterior.iter()).filter_map(move |&k| {
                    let def = &interior.portals.get(k)?.def;
                    let away = rules::point_polygon_distance(eye, &def.corners)?;
                    Some((away, i, def.room_a.max(def.room_b) as usize))
                })
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))?;
        uses_interior_ambient(factor).then_some((nearest.1, nearest.2))
    });
    Walked {
        seen,
        standing,
        inside,
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

#[cfg(test)]
#[path = "interior_rooms_tests.rs"]
mod tests;
