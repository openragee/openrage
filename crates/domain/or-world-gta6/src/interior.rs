//! The contents of the interiors a map places: each entity of an interior and of the entity sets
//! its instance turns on, in the world under the instance, drawn while the camera is within the
//! interior's own distance and sees the room or portal that holds it; and the lights their
//! archetypes hold.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::File;

use glam::{Mat4, Quat, Vec3};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::map::interior::{self, Interior, InteriorInstance};
use or_io::gta6::map::light_ext::{Light, LightKind};
use or_io::gta6::map::lights::Pose;
use or_io::gta6::map::links::{EntitySetLocation, Portal};
use or_io::gta6::map::stream::StreamMap;
use or_io::meta::binary::rbm::schema::RbmModel;
use or_world::interior_portals::point_polygon_distance;
use or_world::streaming::{Level, View, FADE_SMALL, SMALL};

use crate::index::{BodyRow, ModelRef, WorldIndex};
use crate::interior_rooms::{self, Opening, RoomBox, Seen};
use crate::stream::{Chunk, Instance, Rows};

/// The bytes a body of interiors is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// An archetype's flag: what it draws is a door.
// ⚠ GTA V's bit, which the build's archetypes held by closable portals carry.
const DOOR: u32 = 1 << 26;
/// How far from an opening a door's origin, its hinge, may stand and be the opening's door, in metres.
// ⚠ Measured: a map's door near an opening of a shell stands on it, within centimetres, and none
// stands from there to metres off it.
const IN_OPENING: f32 = 0.25;
/// How near two places are to be one, in metres: a placed instance's origin and its entity's position.
const SAME_PLACE: f32 = 0.01;

fn rows_of(m: Mat4) -> Rows {
    let t = m.transpose().to_cols_array_2d();
    [t[0], t[1], t[2]]
}

/// The length of the longest axis of `m`, which a sphere's radius grows by.
fn stretch(m: Mat4) -> f32 {
    [m.x_axis, m.y_axis, m.z_axis]
        .map(|axis| axis.truncate().length())
        .into_iter()
        .fold(0.0, f32::max)
}

/// One entity an interior draws, in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Child {
    pub model: ModelRef,
    pub rows: Rows,
    /// The sphere of its model.
    pub centre: Vec3,
    pub radius: f32,
    /// How far from where it stands it draws.
    pub lod_dist: f32,
    /// Which of the view's scales its distance takes.
    pub level: Level,
    /// The hours it draws in, bit `h` for the hour from `h`: its archetype's.
    pub hours: u32,
    /// The room or portal that holds it.
    pub location: EntitySetLocation,
    /// How much of the natural and of the artificial ambient light it takes.
    pub ambient: [f32; 2],
}

impl Child {
    /// How opaque its own distance makes it at `away` metres: whole within it, gone its level's band past it.
    fn by_distance(&self, view: &View, away: f32) -> f32 {
        let reach = self.lod_dist * view.scales[self.level as usize];
        // ⚠ The forest's rule: the short band is picked by the distance stated, before any scale.
        let band = match self.level == Level::OrphanHd && self.lod_dist <= SMALL {
            true => FADE_SMALL,
            false => view.fade[self.level as usize],
        };
        ((reach + band - away) / band).clamp(0.0, 1.0)
    }
}

/// A room as the frame and the panel name it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomNames {
    pub name: u32,
    pub timecycle: u32,
    pub secondary_timecycle: u32,
    pub blend: f32,
}

/// The interior and room a camera stands in, and the time-cycle modifiers the room names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inside {
    pub interior: u32,
    pub room: u32,
    pub timecycle: u32,
    pub secondary_timecycle: u32,
    /// How much of the room's modifier lies over the day, 0 to 1.
    pub blend: f32,
}

/// One interior as a map places it.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// The instance's place among its map's entities.
    pub entity: usize,
    pub name: u32,
    /// The sphere of the interior's box.
    pub centre: Vec3,
    pub radius: f32,
    /// How far from `centre` the camera may be while the contents draw.
    pub lod_dist: f32,
    pub children: Vec<Child>,
    /// What takes a point of the world into the interior's own space, where its rooms stand.
    pub own: Mat4,
    pub rooms: Vec<RoomBox>,
    pub names: Vec<RoomNames>,
    pub portals: Vec<Opening>,
    /// The luminous flux of each room's lamps, in lumens.
    pub flux: Vec<f32>,
    /// The doors the map places in the openings of its shell.
    pub doors: Vec<InOpening>,
}

/// A door the map places in an opening of an interior's shell, in the interior's space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InOpening {
    /// Where it stands: its hinge.
    pub at: Vec3,
    /// The opening's normal, out of the interior.
    pub out: Vec3,
    /// The ambient scales its face turned into the room takes: the room's, the opening it shuts left out.
    pub inward: [f32; 2],
}

/// The luminous flux of `light`, in lumens, its intensity taken in candelas: all round for a
/// point or a capsule, within its outer cone for a spot, and one side's cosine for an area light.
// ⚠ GTA VI's law lights `intensity / d²`, so the stated intensity is a luminous intensity.
fn flux_of(light: &Light) -> f32 {
    use std::f32::consts::{PI, TAU};
    let solid = match light.kind {
        LightKind::Spot => TAU * (1.0 - light.cone_outer.to_radians().cos()),
        LightKind::Area => PI,
        LightKind::Point | LightKind::Capsule => 2.0 * TAU,
    };
    let [r, g, b] = light.colour;
    let lit = f32::from(u8::from(light.lights_around()));
    lit * solid * light.intensity.max(0.0) * (0.2126 * r + 0.7152 * g + 0.0722 * b)
}

fn placing(scale_xy: f32, scale_z: f32, rotation: [f32; 4], position: [f32; 3]) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::new(scale_xy, scale_xy, scale_z),
        Quat::from_array(rotation).normalize(),
        Vec3::from(position),
    )
}

/// The room of `interior` what stands at `location` is in: what a portal holds is in the room past
/// it from the outside.
fn room_at(interior: &Interior, location: EntitySetLocation) -> u32 {
    match location {
        EntitySetLocation::Room(k) => k,
        EntitySetLocation::Portal(k) => {
            (interior.portals.get(k as usize)).map_or(0, |p| p.room_from.max(p.room_to))
        }
    }
}

/// The rooms a camera sees of each interior in range it looks into, by the map and place of the interior's instance.
pub type RoomsSeen = HashMap<(u32, u32), Vec<bool>>;

/// Whether a lamp of room `room` of the interior placed at `at` lights what a camera that sees `rooms` sees: one of a room the camera does not see lights none of it, and one of an interior whose contents do not draw lights nothing.
// ⚠ GTA V's rule: an interior's lights are those of the rooms its portals show, and a lamp is
// its entity's, there while the entity is.
pub fn lit(rooms: &RoomsSeen, at: (u32, u32), room: u32) -> bool {
    (rooms.get(&at)).is_some_and(|seen| seen.get(room as usize).copied().unwrap_or(true))
}

/// `interior` as `instance` places it: each entity it draws that the index has a model for; the doors its map places, `map_doors` in the world, close the openings of its shell they stand in.
pub fn place(
    index: &WorldIndex,
    instance: &InteriorInstance,
    interior: &Interior,
    map_doors: &[Vec3],
) -> Placed {
    let outer = placing(
        instance.scale_xy,
        instance.scale_z,
        instance.rotation,
        instance.position,
    );
    // How many doors each portal holds.
    let mut doors = vec![0; interior.portals.len()];
    for (e, location) in interior.drawn(&instance.default_entity_sets) {
        let EntitySetLocation::Portal(k) = location else {
            continue;
        };
        if let Some(count) = doors.get_mut(k as usize) {
            let flags = index.archetype(e.archetype_word).map_or(0, |a| a.flags);
            *count += usize::from(flags & DOOR != 0);
        }
    }
    // ⚠ The door of a shell's opening is the map's own entity, of which the interior holds the
    // frame alone; whether the build puts it on the portal was not read, so it shuts nothing.
    let inward = outer.inverse();
    let map_doors: Vec<Vec3> = map_doors
        .iter()
        .map(|d| inward.transform_point3(*d))
        .collect();
    let in_opening = |p: &Portal, d: &Vec3| {
        let corners = p.corners.map(Vec3::from);
        let near = point_polygon_distance(*d, &corners).is_some_and(|a| a <= IN_OPENING);
        (p.room_from == 0 || p.room_to == 0) && near
    };
    let portals: Vec<Opening> = (interior.portals.iter().zip(doors))
        .map(|(p, doors)| Opening {
            rooms: [p.room_from, p.room_to],
            corners: p.corners.map(Vec3::from),
            flags: p.flags,
            shut: interior_rooms::shut_at_rest(p.flags, doors),
            door: map_doors.iter().any(|d| in_opening(p, d)),
        })
        .collect();
    let rooms: Vec<RoomBox> = (interior.rooms.iter())
        .map(|r| RoomBox {
            min: Vec3::from(r.room.bb_min),
            max: Vec3::from(r.room.bb_max),
        })
        .collect();
    // ponytail: every lamp of a room counts at every hour; by the hour if a room's lamps keep hours apart.
    let mut flux = vec![0.0; rooms.len()];
    for (light, room) in lights_of(index, instance, interior) {
        if let Some(sum) = flux.get_mut(room as usize) {
            *sum += flux_of(&light);
        }
    }
    let children = (interior.drawn(&instance.default_entity_sets))
        .filter_map(|(e, location)| {
            let archetype = index.archetype(e.archetype_word)?;
            let model = index.model(archetype)?;
            let own = placing(e.scale_xy, e.scale_z, e.rotation, e.position);
            let m = outer * own;
            let rows = rows_of(m);
            let room = room_at(interior, location);
            Some(Child {
                model,
                rows,
                centre: m.transform_point3(Vec3::from(archetype.centre)),
                radius: archetype.radius * stretch(m),
                // An entity that states no distance takes its archetype's.
                lod_dist: if e.lod_dist > 0.0 {
                    e.lod_dist
                } else {
                    archetype.lod_dist
                },
                level: Level::from_raw(e.lod_level),
                hours: archetype.hours,
                location,
                ambient: interior_rooms::ambient_of(
                    &rooms,
                    &portals,
                    room,
                    own.w_axis.truncate(),
                    flux.get(room as usize).copied().unwrap_or(0.0),
                ),
            })
        })
        .collect();
    let (min, max) = (Vec3::from(interior.bb_min), Vec3::from(interior.bb_max));
    let doors = (map_doors.iter())
        .filter_map(|&at| {
            let k = (interior.portals.iter()).position(|p| in_opening(p, &at))?;
            let opening = portals.get(k)?;
            let (normal, point) = interior_rooms::plane_of(&opening.corners)?;
            // Out of the interior is away from the middle of its box, where its rooms are.
            let out = match normal.dot((min + max) / 2.0 - point) > 0.0 {
                true => -normal,
                false => normal,
            };
            let room = opening.rooms[0].max(opening.rooms[1]);
            let mut behind = portals.clone();
            behind[k].shut = true;
            let lamps = flux.get(room as usize).copied().unwrap_or(0.0);
            let inward = interior_rooms::ambient_of(&rooms, &behind, room, at, lamps);
            Some(InOpening { at, out, inward })
        })
        .collect();
    Placed {
        entity: instance.entity,
        name: interior.name,
        centre: outer.transform_point3((min + max) / 2.0),
        radius: (max - min).length() / 2.0 * stretch(outer),
        lod_dist: if instance.lod_dist > 0.0 {
            instance.lod_dist
        } else {
            interior.lod_dist
        },
        children,
        own: outer.inverse(),
        rooms,
        names: (interior.rooms.iter())
            .map(|r| RoomNames {
                name: r.room.name,
                timecycle: r.timecycle_name,
                secondary_timecycle: r.secondary_timecycle_name,
                blend: r.blend,
            })
            .collect(),
        portals,
        flux,
        doors,
    }
}

impl Placed {
    /// What the walk takes of it.
    pub fn shell(&self) -> interior_rooms::Shell<'_> {
        interior_rooms::Shell {
            rooms: &self.rooms,
            portals: &self.portals,
            own: self.own,
        }
    }

    /// This interior and its room `room`, as a camera inside it stands in them.
    pub fn inside(&self, room: usize) -> Option<Inside> {
        let names = self.names.get(room)?;
        Some(Inside {
            interior: self.name,
            room: names.name,
            timecycle: names.timecycle,
            secondary_timecycle: names.secondary_timecycle,
            blend: names.blend,
        })
    }

    /// Whether a camera at `eye` is within the interior's own distance, scaled as its high level's.
    pub fn in_range(&self, eye: Vec3, view: &View) -> bool {
        // A camera inside the interior's own sphere is in range whatever distance it states.
        let scale = view.scales[Level::Hd as usize];
        eye.distance(self.centre) <= (self.lod_dist * scale).max(self.radius)
    }

    /// Give each instance of `chunks` that stands wholly within one of this interior's rooms the ambient light of what stands there: a map's props and lists placed inside its shell.
    // ⚠ Inferred: an instance whose sphere fits in a room is in it; a shell's walls, wider than their
    // room, keep the outdoors'.
    pub fn shade(&self, chunks: &[Chunk], instances: &mut [Instance]) {
        let reach = |c: Vec3, r: f32| c.distance(self.centre) <= r + self.radius;
        let scale = stretch(self.own);
        for chunk in chunks.iter().filter(|c| reach(c.centre, c.radius)) {
            let run = instances.get_mut(chunk.from as usize..chunk.to as usize);
            for instance in run.into_iter().flatten() {
                if !reach(instance.centre, instance.radius) {
                    continue;
                }
                if let Some(ambient) = self.door_ambient(&instance.rows) {
                    instance.ambient = ambient;
                    continue;
                }
                let p = self.own.transform_point3(instance.centre);
                let room = interior_rooms::room_around(&self.rooms, p, instance.radius * scale);
                if let Some(room) = room {
                    let flux = self.flux.get(room).copied().unwrap_or(0.0);
                    instance.ambient = interior_rooms::ambient_of(
                        &self.rooms,
                        &self.portals,
                        room as u32,
                        p,
                        flux,
                    );
                }
            }
        }
    }

    /// The ambient scales by face of an instance placed by `rows` where it is a door of one of the shell's openings: the outdoors' on the face turned out, the room's on the other.
    fn door_ambient(&self, rows: &Rows) -> Option<[f32; 2]> {
        let at = self
            .own
            .transform_point3(Vec3::new(rows[0][3], rows[1][3], rows[2][3]));
        let door = (self.doors.iter()).find(|d| d.at.distance(at) <= SAME_PLACE)?;
        // The model's axis nearest the opening's normal, and which way along it is out.
        let (axis, out) = (0..3)
            .map(|j| {
                let along = Vec3::new(rows[0][j], rows[1][j], rows[2][j]);
                let along = self.own.transform_vector3(along).normalize_or_zero();
                (j, along.dot(door.out))
            })
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?;
        Some(crate::ambient::by_face(axis, out, door.inward))
    }

    /// Whether the sphere of `radius` about `p` in the world reaches into one of the interior's rooms.
    pub fn reaches_a_room(&self, p: Vec3, radius: f32) -> bool {
        let (p, r) = (self.own.transform_point3(p), radius * stretch(self.own));
        (self.rooms.iter().skip(1))
            .any(|room| p.clamp(room.min, room.max).distance_squared(p) <= r * r)
    }

    /// Call `draw` with each child of what `seen` holds a camera of `view` sees at the hour `hour`, 0 to 23, how far from the camera it stands and how opaque it draws, the interior itself drawing `fade` opaque.
    pub fn each(
        &self,
        view: &View,
        hour: u32,
        fade: f32,
        seen: &Seen,
        mut draw: impl FnMut(&Child, f32, f32),
    ) {
        let eye = view.eye;
        for child in &self.children {
            if (child.hours >> hour) & 1 == 0 || !sees(seen, child.location) {
                continue;
            }
            let r = &child.rows;
            let away = eye.distance(Vec3::new(r[0][3], r[1][3], r[2][3]));
            let own = child.by_distance(view, away);
            if own > 0.0 {
                draw(child, away, fade * own);
            }
        }
    }
}

/// Whether what is at `location` is seen.
fn sees(seen: &Seen, location: EntitySetLocation) -> bool {
    let (of, at) = match location {
        EntitySetLocation::Room(k) => (&seen.rooms, k),
        EntitySetLocation::Portal(k) => (&seen.portals, k),
    };
    of.get(at as usize).copied().unwrap_or(true)
}

/// The interiors of the map types body `row` names, read through the worker's open archives.
fn interiors_at(
    index: &WorldIndex,
    files: &mut HashMap<u32, File>,
    row: &BodyRow,
) -> Option<Vec<Interior>> {
    let file = match files.entry(row.archive) {
        Entry::Occupied(open) => open.into_mut(),
        Entry::Vacant(slot) => {
            let path = index.archives.get(row.archive as usize)?;
            slot.insert(File::open(path).ok()?)
        }
    };
    let (find, bytes) = read_rpf9_body(&*file, row.offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    interior::open(virtual_section, physical)
        .inspect_err(|error| tracing::debug!(offset = row.offset, %error, "interiors not read"))
        .ok()
}

/// Each of `instances` with its interior, each body of interiors read once, through `files`.
// ponytail: a body of interiors is read again for each map that places one of its interiors; a cache a worker if map loads show it.
fn with_interiors(
    index: &WorldIndex,
    files: &mut HashMap<u32, File>,
    instances: &[InteriorInstance],
    mut each: impl FnMut(&InteriorInstance, &Interior),
) {
    let mut bodies: HashMap<(u32, u64), Vec<Interior>> = HashMap::new();
    for instance in instances {
        let Some(row) = index.interior(instance.archetype_word) else {
            continue;
        };
        let body = match bodies.entry((row.archive, row.offset)) {
            Entry::Occupied(read) => read.into_mut(),
            Entry::Vacant(slot) => slot.insert(interiors_at(index, files, row).unwrap_or_default()),
        };
        if let Some(interior) = (body.iter()).find(|i| i.name_word == instance.archetype_word) {
            each(instance, interior);
        }
    }
}

/// The interiors the map body of these two sections places, each from its archetype's body, read through `files`; a map that places none costs one look at `map`.
pub fn read_placed(
    index: &WorldIndex,
    files: &mut HashMap<u32, File>,
    map: &StreamMap,
    virtual_section: &[u8],
    physical: &[u8],
) -> Vec<Placed> {
    if !map.entities.iter().any(|e| e.interior) {
        return Vec::new();
    }
    let instances = match interior::open_instances(virtual_section, physical) {
        Ok(instances) => instances,
        Err(error) => {
            tracing::debug!(map = map.name, %error, "a map's interior instances were not read");
            return Vec::new();
        }
    };
    let is_door = |word: u64| index.archetype(word).is_some_and(|a| a.flags & DOOR != 0);
    let doors: Vec<Vec3> = (map.entities.iter())
        .filter(|e| !e.interior && is_door(e.archetype_word))
        .map(|e| Vec3::from(e.position))
        .collect();
    let mut placed = Vec::with_capacity(instances.len());
    with_interiors(index, files, &instances, |instance, interior| {
        placed.push(place(index, instance, interior, &doors));
    });
    placed
}

/// The lights the archetypes of `interior`'s contents hold, in the world under `instance`, each with the room it is in.
pub fn lights_of(
    index: &WorldIndex,
    instance: &InteriorInstance,
    interior: &Interior,
) -> Vec<(Light, u32)> {
    let outer = Pose {
        position: instance.position,
        rotation: instance.rotation,
        scale_xy: instance.scale_xy,
        scale_z: instance.scale_z,
    };
    (interior.drawn(&instance.default_entity_sets))
        .flat_map(|(e, location)| {
            let inner = Pose {
                position: e.position,
                rotation: e.rotation,
                scale_xy: e.scale_xy,
                scale_z: e.scale_z,
            };
            let room = room_at(interior, location);
            (index.archetype_lights(e.archetype_word).iter())
                .filter_map(move |row| Some((row.light()?.placed(&inner).placed(&outer), room)))
        })
        .collect()
}

/// The boxes the lamps of each room light, by the place of its interior's instance and the room.
pub type RoomBoxes = HashMap<[u32; 2], Vec<Mat4>>;

/// How much larger than its stated box a room is taken to be on each side, in metres, so that its own walls lie in it.
// ⚠ Assumed: thinner than an outer wall, so the far face of a wall stays out.
const ROOM_GROWN: f32 = 0.1;
/// How far either side of its plane a portal's own box reaches, in metres: what stands in an opening and just before it.
// ponytail: a fixed depth; the light that goes far out through an opening would need the opening's view, not a box.
const PORTAL_DEPTH: f32 = 1.0;
/// A portal's flag: a mirror, which no light passes.
const MIRROR: u32 = 0x04;

/// The box of room `room` of `interior` placed by `outer`, `ROOM_GROWN` larger, as the world from a cube of minus one to one; none for room 0, the outside.
fn room_bounds(interior: &Interior, outer: Mat4, room: u32) -> Option<Mat4> {
    let r = &interior.rooms.get(room as usize).filter(|_| room > 0)?.room;
    let (min, max) = (Vec3::from(r.bb_min), Vec3::from(r.bb_max));
    let half = ((max - min) / 2.0 + Vec3::splat(ROOM_GROWN)).max(Vec3::splat(ROOM_GROWN));
    Some(outer * Mat4::from_scale_rotation_translation(half, Quat::IDENTITY, (min + max) / 2.0))
}

/// The boxes a lamp of room `room` of `interior` placed by `outer` lights: its room's, and for each portal of the room but a mirror, the room it opens on and the opening's own box, `PORTAL_DEPTH` either side of it; none for room 0.
// ponytail: every portal counts as open; the door rule if a shut door shows light through it.
pub fn light_bounds(interior: &Interior, outer: Mat4, room: u32) -> Vec<Mat4> {
    let Some(own) = room_bounds(interior, outer, room) else {
        return Vec::new();
    };
    let mut boxes = vec![own];
    for p in (interior.portals.iter()).filter(|p| p.flags & MIRROR == 0) {
        let other = match (p.room_from == room, p.room_to == room) {
            (true, _) => p.room_to,
            (_, true) => p.room_from,
            _ => continue,
        };
        boxes.extend(room_bounds(interior, outer, other).filter(|b| !boxes.contains(b)));
        let c = p.corners.map(Vec3::from);
        if let Some((normal, middle)) = interior_rooms::plane_of(&c) {
            let (across, up) = ((c[1] - c[0]) / 2.0, (c[3] - c[0]) / 2.0);
            let middle = middle.extend(1.0);
            let opening = Mat4::from_cols(
                across.extend(0.0),
                up.extend(0.0),
                (normal * PORTAL_DEPTH).extend(0.0),
                middle,
            );
            boxes.push(outer * opening);
        }
    }
    boxes
}

/// The lights of the contents of the interiors the map body `model` places, read through `files`, each with the place of its interior's instance among the map's entities, its room, and whether it stands inside its room's box; and the boxes [`light_bounds`] gives each room that holds a lamp, by the instance's place and the room.
pub fn lights_placed(
    index: &WorldIndex,
    files: &mut HashMap<u32, File>,
    model: &RbmModel,
) -> (Vec<(Light, [u32; 2], bool)>, RoomBoxes) {
    let mut lights = Vec::new();
    let mut boxes = HashMap::new();
    if !interior::places_interiors(model) {
        return (lights, boxes);
    }
    let instances = interior::read_instances(model)
        .inspect_err(|error| tracing::debug!(%error, "a map's interior instances were not read"))
        .unwrap_or_default();
    with_interiors(index, files, &instances, |instance, interior| {
        let at = instance.entity as u32;
        let outer = placing(
            instance.scale_xy,
            instance.scale_z,
            instance.rotation,
            instance.position,
        );
        for (l, room) in lights_of(index, instance, interior) {
            // A lamp that stands outside its room's box is not held to it: what it lights is not known.
            let inside = room_bounds(interior, outer, room).is_some_and(|m| {
                let p = m.inverse().transform_point3(Vec3::from(l.position));
                p.abs().max_element() <= 1.0
            });
            if inside {
                (boxes.entry([at, room])).or_insert_with(|| light_bounds(interior, outer, room));
            }
            lights.push((l, [at, room], inside));
        }
    });
    (lights, boxes)
}
#[cfg(test)]
#[path = "interior_tests.rs"]
mod tests;
