//! Interior portals as pure rules: a portal definition's bits, when a closable portal shuts, its
//! opacity, the walk from the camera's room, and the camera's distances to exterior portals.

use std::collections::{HashSet, VecDeque};

use glam::{Vec3, Vec4};

use crate::streaming_rules::{PORTAL_FACTOR_CAMERA_BOX, PORTAL_LINK, PORTAL_MIRROR};

/// Proxy state byte (+0xC1) from which an interior counts as populated for the camera factor; read from this build.
pub const POPULATED_PROXY_STATE: u8 = 4;

/// Recursion depth of the game's room flood, a `max_depth` a caller may pass; read from this build, what the flood serves is not known.
pub const ROOM_FLOOD_MAX_DEPTH: u32 = 4;

/// The fields of a portal definition the walks read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortalDef {
    /// Room A (+0x08), 0 the exterior; that it is the "from" room is inferred.
    pub room_a: u32,
    /// Room B (+0x0C), 0 the exterior.
    pub room_b: u32,
    /// Flags (+0x10).
    pub flags: u32,
    /// Opacity in percent (+0x18); `portal_opacity` turns it into 0..=1.
    pub opacity: u32,
    /// Corners, metres, in the same space as the camera handed to the walk; that +0x38 holds them is inferred.
    pub corners: Vec<Vec3>,
}

impl PortalDef {
    /// Whether the walk continues through this portal into the linked interior.
    pub fn is_link(&self) -> bool {
        (self.flags & PORTAL_LINK) != 0
    }

    /// Whether this portal is a mirror (bit 0x04), which every walk read that tests the bit skips.
    pub fn is_mirror(&self) -> bool {
        (self.flags & PORTAL_MIRROR) != 0
    }

    /// The room on the far side from `from`, or none when the portal does not touch `from`.
    pub fn other_room(&self, from: usize) -> Option<usize> {
        let a: usize = self.room_a as usize;
        let b: usize = self.room_b as usize;
        if from == a {
            Some(b)
        } else if from == b {
            Some(a)
        } else {
            None
        }
    }
}

/// A linked portal's target, resolved by the caller: the other instance and its room-0 portal slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    /// Index of the other interior in the walk's slice.
    pub interior: usize,
    /// Slot in the other interior's room-0 portal list.
    pub slot: usize,
}

/// A portal of a placed interior.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Portal {
    /// The definition.
    pub def: PortalDef,
    /// Whether the portal is shut, from `portal_is_shut` with the link's other side tested too.
    pub shut: bool,
    /// Where a link portal leads; none for an unlinked portal or an unloaded target.
    pub link: Option<LinkTarget>,
}

/// A placed interior instance.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interior {
    /// Portals by index.
    pub portals: Vec<Portal>,
    /// Each room's portal indices, room 0 the exterior.
    pub rooms: Vec<Vec<usize>>,
    /// The proxy's state byte (+0xC1).
    pub proxy_state: u8,
    /// Least corner of the instance bounds, metres.
    pub bounds_min: Vec3,
    /// Greatest corner of the instance bounds, metres.
    pub bounds_max: Vec3,
}

/// A room of one interior of the walk's slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoomRef {
    /// Index of the interior in the walk's slice.
    pub interior: usize,
    /// Room index, 0 the exterior.
    pub room: usize,
}

/// A portal of one interior of the walk's slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PortalRef {
    /// Index of the interior in the walk's slice.
    pub interior: usize,
    /// Portal index.
    pub portal: usize,
}

/// What a walk sees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visibility {
    /// Rooms to draw, the start first, each once; none for a start in an unknown room.
    pub rooms: Vec<RoomRef>,
    /// Whether the exterior is seen: the walk starts in room 0 or leaves through an exterior portal.
    pub exterior_seen: bool,
    /// The portals through which the walk sees the exterior.
    pub exterior_portals: Vec<PortalRef>,
}

/// The rooms seen from `start` through open, unmirrored portals inside the frustum, at most `max_depth` portals deep.
pub fn visible_rooms(
    interiors: &[Interior],
    start: RoomRef,
    frustum: &[Vec4],
    max_depth: u32,
) -> Visibility {
    // A camera in room 0 is outside, so it sees the exterior even when the interior has no rooms loaded.
    let mut seen: Visibility = Visibility {
        exterior_seen: start.room == 0,
        ..Visibility::default()
    };
    if !room_exists(interiors, start) {
        return seen;
    }
    seen.rooms.push(start);
    // ponytail: every portal is tested against the whole frustum, not one narrowed through the portals before it; narrow it if too many rooms draw.
    // No facing test: the game's room enumerator has none, and its exterior scan uses the facing side only to pick the points it tests.
    // That scan tests a facing exterior portal's quad extruded by inst+0xF8 (8 points); not done, since +0xF8 overlaps the unresolved instance bounds.
    // Bounded by portals crossed and rooms seen; the exterior scan's visited list of 0x30 instances is not this walk's.
    // The game reads flags 0 for portal index 0xFF and takes no index of 0x100 or more; an archetype with that many portals is outside this walk.
    let mut crossed: HashSet<PortalRef> = HashSet::new();
    let mut queue: VecDeque<(RoomRef, u32)> = VecDeque::new();
    queue.push_back((start, 0));
    // Breadth first, so a room is reached at its least depth whatever the portal order.
    while let Some((room, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let Some(interior) = interiors.get(room.interior) else {
            continue;
        };
        let Some(indices) = interior.rooms.get(room.room) else {
            continue;
        };
        for &index in indices {
            let here: PortalRef = PortalRef {
                interior: room.interior,
                portal: index,
            };
            let Some(portal) = interior.portals.get(index) else {
                continue;
            };
            if portal.shut || portal.def.is_mirror() || crossed.contains(&here) {
                continue;
            }
            let Some(other) = portal.def.other_room(room.room) else {
                continue;
            };
            if !polygon_in_frustum(&portal.def.corners, frustum) {
                continue;
            }
            crossed.insert(here);
            // From room 0 a link portal is an ordinary one into its own room: the linked interior is on the camera's side (inferred).
            if portal.def.is_link() && room.room != 0 {
                // An unresolved link, or one into an instance with no rooms, ends the walk at the link.
                let Some((next, far)) = portal
                    .link
                    .and_then(|target: LinkTarget| cross_link(interiors, target))
                else {
                    continue;
                };
                if !crossed.insert(far) {
                    continue;
                }
                enter(interiors, &mut seen, &mut queue, next, depth + 1);
            } else if other == 0 {
                // ponytail: the walk never goes on from the exterior back into the interior; the exterior scan of room 0 is a separate start.
                seen.exterior_seen = true;
                seen.exterior_portals.push(here);
            } else {
                let next = RoomRef {
                    interior: room.interior,
                    room: other,
                };
                enter(interiors, &mut seen, &mut queue, next, depth + 1);
            }
        }
    }
    seen
}

fn room_exists(interiors: &[Interior], room: RoomRef) -> bool {
    interiors
        .get(room.interior)
        .is_some_and(|interior: &Interior| room.room < interior.rooms.len())
}

fn enter(
    interiors: &[Interior],
    seen: &mut Visibility,
    queue: &mut VecDeque<(RoomRef, u32)>,
    room: RoomRef,
    depth: u32,
) {
    if room_exists(interiors, room) && !seen.rooms.contains(&room) {
        seen.rooms.push(room);
        queue.push_back((room, depth));
    }
}

fn cross_link(interiors: &[Interior], target: LinkTarget) -> Option<(RoomRef, PortalRef)> {
    let other: &Interior = interiors.get(target.interior)?;
    let index: usize = *other.rooms.first()?.get(target.slot)?;
    let def: &PortalDef = &other.portals.get(index)?.def;
    // That the walk enters the far portal's non-exterior room is inferred.
    let room: usize = if def.room_a == 0 {
        def.room_b as usize
    } else {
        def.room_a as usize
    };
    if room == 0 {
        return None;
    }
    let far = PortalRef {
        interior: target.interior,
        portal: index,
    };
    Some((
        RoomRef {
            interior: target.interior,
            room,
        },
        far,
    ))
}

/// Whether a polygon is in a frustum of planes (xyz normal, w offset, inside where n.p + w >= 0): no plane has every corner outside.
pub fn polygon_in_frustum(corners: &[Vec3], planes: &[Vec4]) -> bool {
    // Conservative: a polygon outside the frustum near one of its corners, but not wholly outside any one plane, passes.
    !corners.is_empty()
        && planes.iter().all(|plane: &Vec4| {
            corners
                .iter()
                .any(|corner: &Vec3| plane.truncate().dot(*corner) + plane.w >= 0.0)
        })
}

fn polygon_normal(corners: &[Vec3]) -> Vec3 {
    // A fan about the first corner, so large world coordinates keep their precision; twice the area for a planar convex polygon.
    let Some(first) = corners.first() else {
        return Vec3::ZERO;
    };
    let mut normal: Vec3 = Vec3::ZERO;
    for pair in corners.windows(2).skip(1) {
        normal += (pair[0] - *first).cross(pair[1] - *first);
    }
    normal
}

/// Distance, metres, from a point to a convex polygon, or none for a polygon with no corner.
pub fn point_polygon_distance(point: Vec3, corners: &[Vec3]) -> Option<f32> {
    let first: Vec3 = *corners.first()?;
    let normal: Vec3 = polygon_normal(corners);
    if normal.length_squared() > 0.0 {
        let unit: Vec3 = normal.normalize();
        let height: f32 = unit.dot(point - first);
        let foot: Vec3 = point - unit * height;
        let inside: bool =
            edges(corners).all(|(a, b): (Vec3, Vec3)| unit.dot((b - a).cross(foot - a)) >= 0.0);
        if inside {
            return Some(height.abs());
        }
    }
    let nearest: f32 = edges(corners)
        .map(|(a, b): (Vec3, Vec3)| point_segment_distance(point, a, b))
        .fold(f32::INFINITY, f32::min);
    Some(nearest)
}

fn edges(corners: &[Vec3]) -> impl Iterator<Item = (Vec3, Vec3)> + '_ {
    // Each corner with the next, the last with the first; one corner gives one zero-length edge.
    corners
        .iter()
        .copied()
        .zip(corners.iter().copied().cycle().skip(1))
}

fn point_segment_distance(point: Vec3, a: Vec3, b: Vec3) -> f32 {
    let edge: Vec3 = b - a;
    let length_squared: f32 = edge.length_squared();
    let t: f32 = if length_squared > 0.0 {
        (edge.dot(point - a) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point - (a + edge * t)).length()
}

/// Distances, metres, from the camera to each room-0 portal of every populated interior whose bounds overlap the camera ± 4 m box (the metric is inferred).
pub fn exterior_portal_distances(camera: Vec3, interiors: &[Interior]) -> Vec<f32> {
    // The caller passes live instances only, and whether the camera is inside an interior goes to the factor apart.
    let low: Vec3 = camera - Vec3::splat(PORTAL_FACTOR_CAMERA_BOX);
    let high: Vec3 = camera + Vec3::splat(PORTAL_FACTOR_CAMERA_BOX);
    let mut distances: Vec<f32> = Vec::new();
    // That d is the distance to the quad's nearest point is inferred; the build is read only as distance(camera, quad).
    for interior in interiors {
        let overlaps: bool =
            low.cmple(interior.bounds_max).all() && high.cmpge(interior.bounds_min).all();
        if interior.proxy_state < POPULATED_PROXY_STATE || !overlaps {
            continue;
        }
        let Some(exterior) = interior.rooms.first() else {
            continue;
        };
        for &index in exterior {
            if let Some(distance) = interior
                .portals
                .get(index)
                .and_then(|portal: &Portal| point_polygon_distance(camera, &portal.def.corners))
            {
                distances.push(distance);
            }
        }
    }
    distances
}

#[cfg(test)]
#[path = "interior_portals_tests.rs"]
mod tests;
