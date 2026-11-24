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
use or_io::gta6::map::light_ext::Light;
use or_io::gta6::map::lights::Pose;
use or_io::gta6::map::links::EntitySetLocation;
use or_io::gta6::map::stream::StreamMap;
use or_io::meta::binary::rbm::schema::RbmModel;
use or_world::streaming::{Level, View, FADE_SMALL, SMALL};

use super::index::{BodyRow, ModelRef, WorldIndex};
use super::interior_rooms::{self, Opening, RoomBox, Seen};
use super::stream::{Chunk, Instance, Rows};

/// The bytes a body of interiors is read to.
const BODY_LIMIT: u64 = 256 << 20;
/// An archetype's flag: what it draws is a door.
// ⚠ GTA V's bit, which the build's archetypes held by closable portals carry.
const DOOR: u32 = 1 << 26;

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
pub(super) struct Child {
    pub(super) model: ModelRef,
    pub(super) rows: Rows,
    /// The sphere of its model.
    pub(super) centre: Vec3,
    pub(super) radius: f32,
    /// How far from where it stands it draws.
    pub(super) lod_dist: f32,
    /// Which of the view's scales its distance takes.
    pub(super) level: Level,
    /// The hours it draws in, bit `h` for the hour from `h`: its archetype's.
    pub(super) hours: u32,
    /// The room or portal that holds it.
    pub(super) location: EntitySetLocation,
    /// How much of the natural and of the artificial ambient light it takes.
    pub(super) ambient: [f32; 2],
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
pub(super) struct RoomNames {
    pub(super) name: u32,
    pub(super) timecycle: u32,
    pub(super) secondary_timecycle: u32,
    pub(super) blend: f32,
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
pub(super) struct Placed {
    /// The instance's place among its map's entities.
    pub(super) entity: usize,
    pub(super) name: u32,
    /// The sphere of the interior's box.
    pub(super) centre: Vec3,
    pub(super) radius: f32,
    /// How far from `centre` the camera may be while the contents draw.
    pub(super) lod_dist: f32,
    pub(super) children: Vec<Child>,
    /// What takes a point of the world into the interior's own space, where its rooms stand.
    pub(super) own: Mat4,
    /// The middle of its box, in its own space.
    pub(super) middle: Vec3,
    pub(super) rooms: Vec<RoomBox>,
    pub(super) names: Vec<RoomNames>,
    pub(super) portals: Vec<Opening>,
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

/// The rooms a camera sees of each interior in range it looks into, by the map and place of the
/// interior's instance.
pub(super) type RoomsSeen = HashMap<(u32, u32), Vec<bool>>;

/// Whether a lamp of room `room` of the interior placed at `at` lights what a camera that sees
/// `rooms` sees: one of a room the camera does not see lights none of it.
// ⚠ GTA V's rule: an interior's lights are those of the rooms its portals show.
pub(super) fn lit(rooms: &RoomsSeen, at: (u32, u32), room: u32) -> bool {
    (rooms.get(&at))
        .and_then(|seen| seen.get(room as usize))
        .copied()
        .unwrap_or(true)
}

/// `interior` as `instance` places it: each entity it draws that the index has a model for.
pub(super) fn place(
    index: &WorldIndex,
    instance: &InteriorInstance,
    interior: &Interior,
) -> Placed {
    let outer = placing(
        instance.scale_xy,
        instance.scale_z,
        instance.rotation,
        instance.position,
    );
    // What each portal holds, and how many of those are doors.
    let mut held = vec![(0, 0); interior.portals.len()];
    for (e, location) in interior.drawn(&instance.default_entity_sets) {
        let EntitySetLocation::Portal(k) = location else {
            continue;
        };
        if let Some((all, doors)) = held.get_mut(k as usize) {
            *all += 1;
            let flags = index.archetype(e.archetype_word).map_or(0, |a| a.flags);
            *doors += usize::from(flags & DOOR != 0);
        }
    }
    let portals: Vec<Opening> = (interior.portals.iter().zip(held))
        .map(|(p, (all, doors))| Opening {
            rooms: [p.room_from, p.room_to],
            corners: p.corners.map(Vec3::from),
            flags: p.flags,
            shut: interior_rooms::shut_at_rest(p.flags, [p.room_from, p.room_to], all, doors),
        })
        .collect();
    let rooms: Vec<RoomBox> = (interior.rooms.iter())
        .map(|r| RoomBox {
            min: Vec3::from(r.room.bb_min),
            max: Vec3::from(r.room.bb_max),
        })
        .collect();
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
                ambient: interior_rooms::ambient_of(&rooms, &portals, room, own.w_axis.truncate()),
            })
        })
        .collect();
    let (min, max) = (Vec3::from(interior.bb_min), Vec3::from(interior.bb_max));
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
        middle: (min + max) / 2.0,
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
    }
}

impl Placed {
    /// The interior and room a camera at `eye` stands in, where it stands in one of this interior's rooms.
    pub(super) fn inside(&self, eye: Vec3) -> Option<Inside> {
        let room = interior_rooms::room_of(&self.rooms, self.own.transform_point3(eye))?;
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
    pub(super) fn in_range(&self, eye: Vec3, view: &View) -> bool {
        // A camera inside the interior's own sphere is in range whatever distance it states.
        let scale = view.scales[Level::Hd as usize];
        eye.distance(self.centre) <= (self.lod_dist * scale).max(self.radius)
    }

    /// Give each instance of `chunks` that stands wholly within one of this interior's rooms the
    /// ambient light of what stands there: a map's props and lists placed inside its shell.
    // ⚠ Inferred: an instance whose sphere fits in a room is in it; a shell's walls, wider than their
    // room, keep the outdoors'.
    pub(super) fn shade(&self, chunks: &[Chunk], instances: &mut [Instance]) {
        let reach = |c: Vec3, r: f32| c.distance(self.centre) <= r + self.radius;
        let scale = stretch(self.own);
        for chunk in chunks.iter().filter(|c| reach(c.centre, c.radius)) {
            let run = instances.get_mut(chunk.from as usize..chunk.to as usize);
            for instance in run.into_iter().flatten() {
                if !reach(instance.centre, instance.radius) {
                    continue;
                }
                let p = self.own.transform_point3(instance.centre);
                let room = interior_rooms::room_around(&self.rooms, p, instance.radius * scale);
                if let Some(room) = room {
                    instance.ambient =
                        interior_rooms::ambient_of(&self.rooms, &self.portals, room as u32, p);
                }
            }
        }
    }

    /// Whether the sphere of `radius` about `p` in the world reaches into one of the interior's rooms.
    pub(super) fn reaches_a_room(&self, p: Vec3, radius: f32) -> bool {
        let (p, r) = (self.own.transform_point3(p), radius * stretch(self.own));
        (self.rooms.iter().skip(1))
            .any(|room| p.clamp(room.min, room.max).distance_squared(p) <= r * r)
    }

    /// `pyramids`, planes through the camera in the interior's own space, as planes in the world.
    pub(super) fn to_world(
        &self,
        pyramids: &[interior_rooms::Planes],
    ) -> Vec<interior_rooms::Planes> {
        // A plane through the eye takes the transpose of the map into the interior's space.
        let linear = glam::Mat3::from_mat4(self.own).transpose();
        let world = |n: &Vec3| (linear * *n).normalize_or_zero();
        (pyramids.iter())
            .map(|planes| planes.iter().map(world).collect())
            .collect()
    }

    /// Call `draw` with each child a camera of `view` sees at the hour `hour`, 0 to 23, how far from
    /// the camera it stands and how opaque it draws, the interior itself drawing `fade` opaque; what
    /// it sees of the rooms, where the interior is in range and tells rooms apart.
    pub(super) fn each(
        &self,
        view: &View,
        hour: u32,
        fade: f32,
        mut draw: impl FnMut(&Child, f32, f32),
    ) -> Option<Seen> {
        let eye = view.eye;
        if !self.in_range(eye, view) {
            return None;
        }
        // An interior of no room but the outside has nothing to see through.
        let seen = (self.rooms.len() > 1).then(|| {
            interior_rooms::seen(
                &self.rooms,
                &self.portals,
                self.middle,
                self.own.transform_point3(eye),
            )
        });
        for child in &self.children {
            if (child.hours >> hour) & 1 == 0 || !sees(seen.as_ref(), child.location) {
                continue;
            }
            let r = &child.rows;
            let away = eye.distance(Vec3::new(r[0][3], r[1][3], r[2][3]));
            let own = child.by_distance(view, away);
            if own > 0.0 {
                draw(child, away, fade * own);
            }
        }
        seen
    }
}

/// Whether what is at `location` is seen; everything where no room is told apart.
fn sees(seen: Option<&Seen>, location: EntitySetLocation) -> bool {
    let Some(seen) = seen else {
        return true;
    };
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

/// The interiors the map body of these two sections places, each from its archetype's body, read
/// through `files`; a map that places none costs one look at `map`.
pub(super) fn read_placed(
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
    let mut placed = Vec::with_capacity(instances.len());
    with_interiors(index, files, &instances, |instance, interior| {
        placed.push(place(index, instance, interior));
    });
    placed
}

/// The lights the archetypes of `interior`'s contents hold, in the world under `instance`, each
/// with the room it is in.
pub(super) fn lights_of(
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

/// The lights of the contents of the interiors the map body `model` places, read through `files`,
/// each with the place of its interior's instance among the map's entities and its room.
pub(super) fn lights_placed(
    index: &WorldIndex,
    files: &mut HashMap<u32, File>,
    model: &RbmModel,
) -> Vec<(Light, [u32; 2])> {
    if !interior::places_interiors(model) {
        return Vec::new();
    }
    let instances = interior::read_instances(model)
        .inspect_err(|error| tracing::debug!(%error, "a map's interior instances were not read"))
        .unwrap_or_default();
    let mut lights = Vec::new();
    with_interiors(index, files, &instances, |instance, interior| {
        let at = instance.entity as u32;
        lights.extend(
            (lights_of(index, instance, interior).into_iter()).map(|(l, room)| (l, [at, room])),
        );
    });
    lights
}

#[cfg(test)]
#[path = "interior_tests.rs"]
mod tests;
