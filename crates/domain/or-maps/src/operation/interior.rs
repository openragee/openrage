//! The operations that edit an interior a type file declares: its rooms, its portals, its entity sets, its time cycle modifiers and
//! the entities it holds. Each names the interior by its index among the archetypes of the file, and gives as its inverse another
//! operation of these.
//!
//! ⚠ **An interior holds four kinds of index, and an operation that moves what one names rewrites every one of them.** A portal
//! names its two rooms, and an entity set names a room for each entity it holds (`locations`); a room and a portal each list the
//! entities they attach by their place in the interior's `entities`; and a room states how many portals name it (`portalCount`).
//! A room is taken out only once no portal and no entity set names it, so that taking it out and putting it back are each other's
//! inverse with nothing carried beside the room.
//!
//! ⚠ **What the installs state, which the refusals here hold an edit to** (the suite
//! `what_the_interior_rules_say_of_the_install`, both installs, 2026-11-25): room 0 is the outside, named `limbo` in every interior,
//! and every interior holds a room beside it; every portal has four corners and two different rooms; every room counts exactly the
//! portals that name it; no entity of an interior states a LOD parent; and no two rooms and no two entity sets of one interior
//! share a name.
//!
//! ⚠ **A location with bit 31 set is no room index, and is not read here.** Six locations of each install state it, each with an
//! index below the portal count of its interior in the low bits and five of them one at or above its room count. What it means is
//! not decided from six values: a room renumbering leaves such a location alone, and an interior that states one takes a portal
//! last and gives none up, since either could move what the location names.

use std::collections::BTreeSet;

use or_io::meta::instance::HashedString;
use or_io::meta::typed::{
    CBaseArchetypeDefAny, CEntityDefAny, CMapTypes, CMloArchetypeDef, CMloEntitySet, CMloPortalDef,
    CMloRoomDef, CMloTimeCycleModifier,
};
use or_world::validate::Holder;
use serde::{Deserialize, Serialize};

use super::{first_entity, name_of, Changed, Context, Operation, Record};
use crate::edit;

/// The room of an interior that is the outside.
pub const OUTSIDE: usize = 0;

/// The bit of an entity set's location that marks a value that is no room index.
pub const NOT_A_ROOM: u32 = 1 << 31;

/// The members of a room an operation states, each named as the file names it: one left out is not changed, and a room that is added takes the engine's initial value for it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomMembers {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bb_min: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bb_max: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<f32>,
    /// A name, or `hash_XXXXXXXX` for one known by its hash alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timecycle_name: Option<String>,
    /// A name, or `hash_XXXXXXXX` for one known by its hash alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_timecycle_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    /// How many portals name the room, which adding and taking out a portal keep and this states as it is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exterior_visibilty_depth: Option<i32>,
}

fn corner(v: or_io::decl::Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn stored([x, y, z]: [f32; 3]) -> or_io::decl::Vec3 {
    or_io::decl::Vec3 { x, y, z }
}

/// State `stated` in `held` when it states something, and return what `held` was.
fn put<T: Clone>(stated: &Option<T>, held: &mut T) -> Option<T> {
    (stated.as_ref()).map(|stated| std::mem::replace(held, stated.clone()))
}

/// `name` as an inverse states it: by its hash, and as nothing for the name that is none.
///
/// ⚠ Never by its text. A root a session holds keeps the text an operation stated, and the same file read again from its XML keeps
/// the hash alone, and the two are to give one inverse.
fn spelled(name: &HashedString) -> String {
    HashedString::from_hash(name.hash).to_xml_text()
}

/// State the name `stated` spells in `held` when it states one, and return the name `held` was.
fn put_name(stated: &Option<String>, held: &mut HashedString) -> Option<String> {
    (stated.as_ref()).map(|stated| {
        spelled(&std::mem::replace(
            held,
            HashedString::from_xml_text(stated),
        ))
    })
}

fn finite(numbers: &[f32]) -> bool {
    numbers.iter().all(|number| number.is_finite())
}

impl RoomMembers {
    /// Every member of `room`.
    pub fn of(room: &CMloRoomDef) -> Self {
        Self {
            name: Some(room.name.clone()),
            bb_min: Some(corner(room.bb_min)),
            bb_max: Some(corner(room.bb_max)),
            blend: Some(room.blend),
            timecycle_name: Some(spelled(&room.timecycle_name)),
            secondary_timecycle_name: Some(spelled(&room.secondary_timecycle_name)),
            flags: Some(room.flags),
            portal_count: Some(room.portal_count),
            floor_id: Some(room.floor_id),
            exterior_visibilty_depth: Some(room.exterior_visibilty_depth),
        }
    }

    /// Refuse a set of members that states none, or a number that is not finite.
    fn checked(&self) -> Result<(), String> {
        if *self == Self::default() {
            return Err("no member of the room is stated".to_string());
        }
        let numbers = (self.bb_min.iter().flatten())
            .chain(self.bb_max.iter().flatten())
            .chain(&self.blend);
        if numbers.into_iter().all(|number| number.is_finite()) {
            Ok(())
        } else {
            Err("a box or a blend that is not a finite number cannot be stored".to_string())
        }
    }

    /// State these in `room`, and return the members they replaced.
    fn state(&self, room: &mut CMloRoomDef) -> Self {
        let mut bb_min = corner(room.bb_min);
        let mut bb_max = corner(room.bb_max);
        let was = Self {
            name: put(&self.name, &mut room.name),
            bb_min: put(&self.bb_min, &mut bb_min),
            bb_max: put(&self.bb_max, &mut bb_max),
            blend: put(&self.blend, &mut room.blend),
            timecycle_name: put_name(&self.timecycle_name, &mut room.timecycle_name),
            secondary_timecycle_name: put_name(
                &self.secondary_timecycle_name,
                &mut room.secondary_timecycle_name,
            ),
            flags: put(&self.flags, &mut room.flags),
            portal_count: put(&self.portal_count, &mut room.portal_count),
            floor_id: put(&self.floor_id, &mut room.floor_id),
            exterior_visibilty_depth: put(
                &self.exterior_visibilty_depth,
                &mut room.exterior_visibilty_depth,
            ),
        };
        // ⚠ Only where it was stated: a box put back through its numbers could change the sign of a zero.
        if self.bb_min.is_some() {
            room.bb_min = stored(bb_min);
        }
        if self.bb_max.is_some() {
            room.bb_max = stored(bb_max);
        }
        was
    }
}

/// The members of a portal an operation states, each named as the file names it: one left out is not changed, and a portal that is added takes nought for it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortalMembers {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_from: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_to: Option<u32>,
    /// The flags as the one word the file states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_priority: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_occlusion: Option<u32>,
    /// The four corners of its quad, in the interior's own space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<Vec<[f32; 3]>>,
}

impl PortalMembers {
    /// Every member of `portal`.
    pub fn of(portal: &CMloPortalDef) -> Self {
        Self {
            room_from: Some(portal.room_from),
            room_to: Some(portal.room_to),
            flags: Some(portal.flags),
            mirror_priority: Some(portal.mirror_priority),
            opacity: Some(portal.opacity),
            audio_occlusion: Some(portal.audio_occlusion),
            corners: Some(portal.corners.iter().map(|c| corner(*c)).collect()),
        }
    }

    /// Refuse a set of members that states none, corners that are not four, or a corner that is not finite.
    fn checked(&self) -> Result<(), String> {
        if *self == Self::default() {
            return Err("no member of the portal is stated".to_string());
        }
        let Some(corners) = &self.corners else {
            return Ok(());
        };
        if corners.len() != 4 {
            return Err(format!(
                "a portal is a quad of four corners, and {} are stated",
                corners.len()
            ));
        }
        if corners.iter().all(|corner| finite(corner)) {
            Ok(())
        } else {
            Err("a corner that is not a finite number cannot be stored".to_string())
        }
    }

    /// State these in `portal`, and return the members they replaced.
    fn state(&self, portal: &mut CMloPortalDef) -> Self {
        let corners = self.corners.as_ref().map(|stated| {
            let stated = stated.iter().map(|c| stored(*c)).collect();
            let was = std::mem::replace(&mut portal.corners, stated);
            was.into_iter().map(corner).collect()
        });
        Self {
            room_from: put(&self.room_from, &mut portal.room_from),
            room_to: put(&self.room_to, &mut portal.room_to),
            flags: put(&self.flags, &mut portal.flags),
            mirror_priority: put(&self.mirror_priority, &mut portal.mirror_priority),
            opacity: put(&self.opacity, &mut portal.opacity),
            audio_occlusion: put(&self.audio_occlusion, &mut portal.audio_occlusion),
            corners,
        }
    }
}

/// A room or a portal that attaches an entity, and the place of the entity in its list: the last when none is stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attached {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
}

/// One entity an entity set holds, with the room it stands in as the file states it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetEntity {
    pub location: u32,
    pub record: Record,
}

/// The room or the portal an operation names: one of the two, and not both.
fn holder(room: Option<usize>, portal: Option<usize>) -> Result<Holder, String> {
    match (room, portal) {
        (Some(room), None) => Ok(Holder::Room(room)),
        (None, Some(portal)) => Ok(Holder::Portal(portal)),
        (None, None) => Err("neither a room nor a portal is named".to_string()),
        (Some(_), Some(_)) => {
            Err("a room and a portal are both named, and an entity is attached to one".to_string())
        }
    }
}

fn stated(holder: Holder) -> (Option<usize>, Option<usize>) {
    match holder {
        Holder::Room(room) => (Some(room), None),
        Holder::Portal(portal) => (None, Some(portal)),
    }
}

fn attached_of(mlo: &CMloArchetypeDef, holder: Holder) -> Result<&Vec<u32>, String> {
    match holder {
        Holder::Room(room) => (mlo.rooms.get(room)).map(|held| &held.attached_objects),
        Holder::Portal(portal) => (mlo.portals.get(portal)).map(|held| &held.attached_objects),
    }
    .ok_or_else(|| past(holder, mlo))
}

fn attached_of_mut(mlo: &mut CMloArchetypeDef, holder: Holder) -> Option<&mut Vec<u32>> {
    match holder {
        Holder::Room(room) => (mlo.rooms.get_mut(room)).map(|held| &mut held.attached_objects),
        Holder::Portal(portal) => {
            (mlo.portals.get_mut(portal)).map(|held| &mut held.attached_objects)
        }
    }
}

/// The refusal of a room or a portal the interior does not hold.
fn past(holder: Holder, mlo: &CMloArchetypeDef) -> String {
    let (held, what) = match holder {
        Holder::Room(_) => (mlo.rooms.len(), "rooms"),
        Holder::Portal(_) => (mlo.portals.len(), "portals"),
    };
    format!("{holder} is past the {held} {what} the interior holds")
}

/// The interior `types` declares at `index`.
fn interior_at(types: &CMapTypes, index: usize) -> Result<&CMloArchetypeDef, String> {
    match types.archetypes.get(index) {
        Some(CBaseArchetypeDefAny::CMloArchetypeDef(mlo)) => Ok(mlo),
        Some(_) => Err(format!("archetype {index} is no interior")),
        None => Err(format!(
            "archetype {index} is past the {} the type file holds",
            types.archetypes.len()
        )),
    }
}

fn interior_at_mut(types: &mut CMapTypes, index: usize) -> Result<&mut CMloArchetypeDef, String> {
    interior_at(types, index)?;
    match types.archetypes.get_mut(index) {
        Some(CBaseArchetypeDefAny::CMloArchetypeDef(mlo)) => Ok(mlo),
        _ => Err(format!("archetype {index} is no interior")),
    }
}

/// `index` as an interior states a room, a portal or an entity.
fn word(index: usize, what: &str) -> Result<u32, String> {
    u32::try_from(index).map_err(|_| format!("{what} {index} is past what the file can state"))
}

/// Refuse a list of attached objects that names an entity past the `held` of the interior, or one twice.
fn attached_hold(attached: &[u32], held: usize) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for index in attached {
        if *index as usize >= held {
            return Err(format!(
                "entity {index} is attached, past the {held} the interior holds"
            ));
        }
        if !seen.insert(index) {
            return Err(format!("entity {index} is attached twice"));
        }
    }
    Ok(())
}

/// Renumber every room a portal or an entity set of `mlo` names by `moved`.
fn renumber_rooms(mlo: &mut CMloArchetypeDef, moved: impl Fn(u32) -> u32) {
    for portal in &mut mlo.portals {
        portal.room_from = moved(portal.room_from);
        portal.room_to = moved(portal.room_to);
    }
    let locations = mlo
        .entity_sets
        .iter_mut()
        .flat_map(|set| &mut set.locations);
    for location in locations.filter(|location| **location & NOT_A_ROOM == 0) {
        *location = moved(*location);
    }
}

/// Renumber every entity a room or a portal of `mlo` attaches by `moved`.
fn renumber_entities(mlo: &mut CMloArchetypeDef, moved: impl Fn(u32) -> u32) {
    let rooms = mlo.rooms.iter_mut().map(|room| &mut room.attached_objects);
    let portals = (mlo.portals.iter_mut()).map(|portal| &mut portal.attached_objects);
    for index in rooms.chain(portals).flatten() {
        *index = moved(*index);
    }
}

/// Whether an entity set of `mlo` states a location that is no room index.
fn states_what_is_no_room(mlo: &CMloArchetypeDef) -> bool {
    (mlo.entity_sets.iter()).any(|set| set.locations.iter().any(|l| l & NOT_A_ROOM != 0))
}

/// Refuse to move the entities of `mlo` while one of them states a LOD parent, an index into them that nothing here renumbers.
fn no_lod_parent(mlo: &CMloArchetypeDef) -> Result<(), String> {
    match mlo.entities.iter().position(|e| e.parent_index >= 0) {
        Some(entity) => Err(format!(
            "entity {entity} of the interior names a LOD parent by its index, which no interior of the installs does and this does not renumber"
        )),
        None => Ok(()),
    }
}

/// The two rooms of a portal, once each, that the interior holds.
fn rooms_of(from: u32, to: u32, held: usize) -> Vec<usize> {
    let mut rooms = vec![from as usize];
    if to != from {
        rooms.push(to as usize);
    }
    rooms.retain(|room| *room < held);
    rooms
}

/// Refuse when a room of `rooms` cannot count one portal more, or one fewer.
fn counts_hold(mlo: &CMloArchetypeDef, rooms: &[usize], more: bool) -> Result<(), String> {
    for room in rooms {
        let counted = mlo.rooms[*room].portal_count;
        if more && counted == u32::MAX {
            return Err(format!("room {room} can count no portal more"));
        }
        if !more && counted == 0 {
            return Err(format!(
                "room {room} counts no portal, so it cannot count one fewer: state its `portal_count` first"
            ));
        }
    }
    Ok(())
}

fn count(mlo: &mut CMloArchetypeDef, rooms: &[usize], more: bool) {
    for room in rooms {
        let counted = &mut mlo.rooms[*room].portal_count;
        *counted = if more { *counted + 1 } else { *counted - 1 };
    }
}

/// Take the entity `index` out of `mlo`, out of every list that attaches it and with every later index renumbered; the entity and where it was attached.
fn take_out(mlo: &mut CMloArchetypeDef, index: usize) -> (CEntityDefAny, Vec<Attached>) {
    let named = index as u32;
    let mut attached = Vec::new();
    let rooms = (0..mlo.rooms.len()).map(Holder::Room);
    for holder in rooms.chain((0..mlo.portals.len()).map(Holder::Portal)) {
        let Some(list) = attached_of_mut(mlo, holder) else {
            continue;
        };
        let (room, portal) = stated(holder);
        // ⚠ By the place each has in the list as it was, lowest first: put back in that order, each finds the list as long as it left it.
        for (at, _) in list.iter().enumerate().filter(|(_, held)| **held == named) {
            attached.push(Attached {
                room,
                portal,
                at: Some(at),
            });
        }
        list.retain(|held| *held != named);
    }
    renumber_entities(mlo, |held| if held > named { held - 1 } else { held });
    (mlo.entities.remove(index), attached)
}

/// Refuse `attached` when it names a room or a portal `mlo` does not hold, or a place past the end of its list as the ones before leave it.
fn attachments_hold(mlo: &CMloArchetypeDef, attached: &[Attached]) -> Result<(), String> {
    let mut longer: Vec<Holder> = Vec::new();
    for put in attached {
        let holder = holder(put.room, put.portal)?;
        let held =
            attached_of(mlo, holder)?.len() + longer.iter().filter(|h| **h == holder).count();
        if let Some(at) = put.at.filter(|at| *at > held) {
            return Err(format!(
                "{holder} attaches {held} entities by then, and place {at} is past their end"
            ));
        }
        longer.push(holder);
    }
    Ok(())
}

/// Put `entity` into `mlo` at `index`, with every index at or after it renumbered, attached as `attached` says, which [`attachments_hold`] has passed.
fn put_in(mlo: &mut CMloArchetypeDef, index: usize, entity: CEntityDefAny, attached: &[Attached]) {
    let named = index as u32;
    // ⚠ Never past the largest number a file states, which would wrap to the first entity: the save refuses the interior for it.
    renumber_entities(mlo, |held| held.saturating_add(u32::from(held >= named)));
    mlo.entities.insert(index, entity);
    for put in attached {
        let Some(list) =
            (holder(put.room, put.portal).ok()).and_then(|holder| attached_of_mut(mlo, holder))
        else {
            continue;
        };
        list.insert(put.at.unwrap_or(list.len()).min(list.len()), named);
    }
}

/// A guid no entity of `mlo` states, in its own list or in an entity set.
fn free_guid(mlo: &CMloArchetypeDef) -> u32 {
    let in_sets = mlo.entity_sets.iter().flat_map(|set| &set.entities);
    let stated = (mlo.entities.iter().chain(in_sets))
        .map(|e| e.guid)
        .collect();
    or_world::create::free_guid_among(&stated).unwrap_or(0)
}

/// The words every refusal of the outside gives.
const THE_OUTSIDE: &str =
    "room 0 is the outside of an interior, which every interior of the installs holds first";

/// Whether the inverse [`change`] gives for `operation` puts the interior back with nothing to prove.
///
/// ⚠ Only where the inverse states again what the operation replaced and nothing is renumbered or counted: members of a room, members
/// of a portal that leave its two rooms as they are, and an entity put into or taken out of one list. What the operation refuses
/// needs no inverse.
pub(super) fn sure(types: &CMapTypes, operation: &Operation) -> bool {
    let interior = |archetype: &usize| interior_at(types, *archetype).ok();
    match operation {
        Operation::SetRoom {
            archetype,
            room,
            members,
            ..
        } => {
            let Some((mlo, held)) =
                interior(archetype).and_then(|m| Some((m, m.rooms.get(*room)?)))
            else {
                return true;
            };
            let numbers = [corner(held.bb_min), corner(held.bb_max), [held.blend; 3]];
            // The name it has is put back by the inverse, which is refused a name another room has, and no name.
            let refused = held.name.trim().is_empty()
                || (mlo.rooms.iter().enumerate())
                    .any(|(at, other)| at != *room && other.name.eq_ignore_ascii_case(&held.name));
            numbers.iter().all(|n| finite(n)) && !(members.name.is_some() && refused)
        }
        Operation::SetPortal {
            archetype,
            portal,
            members,
            ..
        } => {
            let Some(held) = interior(archetype).and_then(|m| m.portals.get(*portal)) else {
                return true;
            };
            members.room_from.is_none()
                && members.room_to.is_none()
                && PortalMembers::of(held).checked().is_ok()
        }
        Operation::AttachEntity { .. } => true,
        Operation::DetachEntity {
            archetype,
            entity,
            room,
            portal,
            ..
        } => {
            let list = interior(archetype)
                .zip(holder(*room, *portal).ok())
                .and_then(|(mlo, holder)| attached_of(mlo, holder).ok());
            // An entity a list holds twice is put back beside the one that stays, which attaching refuses.
            list.is_none_or(|list| list.iter().filter(|h| **h as usize == *entity).count() <= 1)
        }
        _ => false,
    }
}

/// Do `operation`, one of an interior, to `types`; a refusal leaves `types` as it was.
pub(super) fn change(
    types: &mut CMapTypes,
    operation: &Operation,
    context: &Context,
) -> Result<Changed, String> {
    let source = operation.source().to_string();
    match operation {
        Operation::AddRoom {
            archetype,
            room,
            members,
            attached_objects,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.rooms.len();
            let index = room.unwrap_or(held);
            if index > held {
                return Err(format!(
                    "room {index} is past the end of the {held} the interior holds"
                ));
            }
            if index == OUTSIDE && held > 0 {
                return Err(format!("{THE_OUTSIDE}, so no room is put before it"));
            }
            word(held, "room")?;
            members.checked()?;
            let name = members.name.as_deref().unwrap_or_default();
            if name.trim().is_empty() {
                return Err("a room has a name, and none is given".to_string());
            }
            name_is_free(mlo, name, None)?;
            attached_hold(attached_objects, mlo.entities.len())?;
            let mut built = CMloRoomDef::default();
            members.state(&mut built);
            built.attached_objects = attached_objects.clone();
            let at = index as u32;
            // ⚠ Never past the largest number a file states, which would wrap to the outside: such an index names no room before
            // or after, and the save refuses the interior for it.
            let later = |named: u32| named.saturating_add(u32::from(named >= at));
            renumber_rooms(mlo, later);
            mlo.rooms.insert(index, built);
            let inverse = Operation::RemoveRoom {
                source,
                expects: None,
                archetype: *archetype,
                room: index,
            };
            Ok((inverse, Some(index)))
        }
        Operation::RemoveRoom {
            archetype, room, ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let index = *room;
            let held = (mlo.rooms.get(index)).ok_or_else(|| past(Holder::Room(index), mlo))?;
            if index == OUTSIDE && mlo.rooms.len() > 1 {
                return Err(format!(
                    "{THE_OUTSIDE}, so it is not taken out while the interior holds another room"
                ));
            }
            let at = word(index, "room")?;
            let portals: Vec<String> = (mlo.portals.iter().enumerate())
                .filter(|(_, portal)| portal.room_from == at || portal.room_to == at)
                .map(|(portal, _)| portal.to_string())
                .collect();
            if !portals.is_empty() {
                return Err(format!(
                    "portal(s) {} name the room: take each out, or lead it to another room, first",
                    portals.join(", ")
                ));
            }
            let placed = (mlo.entity_sets.iter().enumerate())
                .find(|(_, set)| set.locations.contains(&at))
                .map(|(set, _)| set);
            if let Some(set) = placed {
                return Err(format!(
                    "entity set {set} places an entity in the room: move it out of the set, or take the set out, first"
                ));
            }
            let inverse = Operation::AddRoom {
                source,
                expects: None,
                archetype: *archetype,
                room: Some(index),
                members: RoomMembers::of(held),
                attached_objects: held.attached_objects.clone(),
            };
            mlo.rooms.remove(index);
            renumber_rooms(mlo, |named| if named > at { named - 1 } else { named });
            Ok((inverse, None))
        }
        Operation::SetRoom {
            archetype,
            room,
            members,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            if *room >= mlo.rooms.len() {
                return Err(past(Holder::Room(*room), mlo));
            }
            members.checked()?;
            if let Some(name) = &members.name {
                if name.trim().is_empty() {
                    return Err("a room has a name, and none is given".to_string());
                }
                name_is_free(mlo, name, Some(*room))?;
            }
            let was = members.state(&mut mlo.rooms[*room]);
            let inverse = Operation::SetRoom {
                source,
                expects: None,
                archetype: *archetype,
                room: *room,
                members: was,
            };
            Ok((inverse, None))
        }
        Operation::AddPortal {
            archetype,
            portal,
            members,
            attached_objects,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.portals.len();
            let index = portal.unwrap_or(held);
            if index > held {
                return Err(format!(
                    "portal {index} is past the end of the {held} the interior holds"
                ));
            }
            if index != held && states_what_is_no_room(mlo) {
                return Err(
                    "an entity set of the interior states a location that is no room, which this build does not read, so a portal is added last here"
                        .to_string(),
                );
            }
            members.checked()?;
            let (Some(from), Some(to), Some(_)) =
                (members.room_from, members.room_to, &members.corners)
            else {
                return Err(
                    "a portal has two rooms and four corners, and `room_from`, `room_to` and `corners` are not all stated"
                        .to_string(),
                );
            };
            let rooms = joined(mlo, from, to)?;
            counts_hold(mlo, &rooms, true)?;
            attached_hold(attached_objects, mlo.entities.len())?;
            let mut built = CMloPortalDef::default();
            members.state(&mut built);
            built.attached_objects = attached_objects.clone();
            mlo.portals.insert(index, built);
            count(mlo, &rooms, true);
            leaves_the_outside(context, from);
            let inverse = Operation::RemovePortal {
                source,
                expects: None,
                archetype: *archetype,
                portal: index,
            };
            Ok((inverse, Some(index)))
        }
        Operation::RemovePortal {
            archetype, portal, ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let index = *portal;
            let held = (mlo.portals.get(index)).ok_or_else(|| past(Holder::Portal(index), mlo))?;
            if states_what_is_no_room(mlo) {
                return Err(
                    "an entity set of the interior states a location that is no room, which this build does not read and a portal taken out could leave naming another: no portal is taken out here"
                        .to_string(),
                );
            }
            let rooms = rooms_of(held.room_from, held.room_to, mlo.rooms.len());
            counts_hold(mlo, &rooms, false)?;
            let inverse = Operation::AddPortal {
                source,
                expects: None,
                archetype: *archetype,
                portal: Some(index),
                members: PortalMembers::of(held),
                attached_objects: held.attached_objects.clone(),
            };
            mlo.portals.remove(index);
            count(mlo, &rooms, false);
            Ok((inverse, None))
        }
        Operation::SetPortal {
            archetype,
            portal,
            members,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held =
                (mlo.portals.get(*portal)).ok_or_else(|| past(Holder::Portal(*portal), mlo))?;
            members.checked()?;
            let (was_from, was_to) = (held.room_from, held.room_to);
            let from = members.room_from.unwrap_or(was_from);
            let to = members.room_to.unwrap_or(was_to);
            let moved = (from, to) != (was_from, was_to);
            let (leaves, joins) = if moved {
                let joins = joined(mlo, from, to)?;
                let leaves = rooms_of(was_from, was_to, mlo.rooms.len());
                // A room on both sides of the change counts what it counted.
                let only = |of: &[usize], other: &[usize]| -> Vec<usize> {
                    (of.iter().copied())
                        .filter(|r| !other.contains(r))
                        .collect()
                };
                (only(&leaves, &joins), only(&joins, &leaves))
            } else {
                (Vec::new(), Vec::new())
            };
            counts_hold(mlo, &leaves, false)?;
            counts_hold(mlo, &joins, true)?;
            let was = members.state(&mut mlo.portals[*portal]);
            count(mlo, &leaves, false);
            count(mlo, &joins, true);
            if moved {
                leaves_the_outside(context, from);
            }
            let inverse = Operation::SetPortal {
                source,
                expects: None,
                archetype: *archetype,
                portal: *portal,
                members: was,
            };
            Ok((inverse, None))
        }
        Operation::AttachEntity {
            archetype,
            entity,
            room,
            portal,
            at,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let holder = holder(*room, *portal)?;
            let held = mlo.entities.len();
            if *entity >= held {
                return Err(format!(
                    "entity {entity} is past the {held} the interior holds"
                ));
            }
            let named = word(*entity, "entity")?;
            let list = attached_of(mlo, holder)?;
            if list.contains(&named) {
                return Err(format!("{holder} attaches entity {entity} already"));
            }
            let place = at.unwrap_or(list.len());
            if place > list.len() {
                return Err(format!(
                    "{holder} attaches {} entities, and place {place} is past their end",
                    list.len()
                ));
            }
            if let Some(list) = attached_of_mut(mlo, holder) {
                list.insert(place, named);
            }
            let inverse = Operation::DetachEntity {
                source,
                expects: None,
                archetype: *archetype,
                entity: *entity,
                room: *room,
                portal: *portal,
            };
            Ok((inverse, None))
        }
        Operation::DetachEntity {
            archetype,
            entity,
            room,
            portal,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let holder = holder(*room, *portal)?;
            let list = attached_of(mlo, holder)?;
            let place = (list.iter().position(|held| *held as usize == *entity))
                .ok_or_else(|| format!("{holder} does not attach entity {entity}"))?;
            if let Some(list) = attached_of_mut(mlo, holder) {
                list.remove(place);
            }
            let inverse = Operation::AttachEntity {
                source,
                expects: None,
                archetype: *archetype,
                entity: *entity,
                room: *room,
                portal: *portal,
                at: Some(place),
            };
            Ok((inverse, None))
        }
        Operation::AddInteriorEntity {
            archetype,
            archetype_name,
            pose,
            donor,
            room,
            ..
        } => {
            let placed = name_of(archetype_name);
            let an_interior = types.archetypes.iter().any(|declared| {
                declared.name.hash == placed.hash
                    && matches!(declared, CBaseArchetypeDefAny::CMloArchetypeDef(_))
            });
            let mlo = interior_at_mut(types, *archetype)?;
            let pose = pose.checked()?;
            if placed.is_empty() {
                return Err("an entity places an archetype, and none is named".to_string());
            }
            if an_interior {
                return Err(
                    "the archetype named is an interior this file declares, and an interior holds no interior"
                        .to_string(),
                );
            }
            if let Some(room) = room.filter(|room| *room >= mlo.rooms.len()) {
                return Err(past(Holder::Room(room), mlo));
            }
            let index = mlo.entities.len();
            let named = word(index, "entity")?;
            let copied = match donor {
                Some(donor) => Some(mlo.entities.get(*donor).ok_or_else(|| {
                    format!("the donor, entity {donor}, is past the {index} the interior holds")
                })?),
                None => mlo.entities.first(),
            };
            let mut entity = match copied {
                Some(copied) => {
                    let mut entity = copied.clone();
                    entity.archetype_name = match placed.text {
                        Some(text) => HashedString::new(text),
                        None => HashedString::from_hash(placed.hash),
                    };
                    entity.parent_index = -1;
                    entity.num_children = 0;
                    edit::set_pose(&mut entity, pose);
                    entity
                }
                // ⚠ Never a copy, as the first entity of a map is none: an interior with no entity states no class of one.
                None => first_entity(placed, pose, None)?,
            };
            entity.guid = free_guid(mlo);
            mlo.entities.push(entity);
            if let Some(room) = room {
                mlo.rooms[*room].attached_objects.push(named);
            }
            let inverse = Operation::RemoveInteriorEntity {
                source,
                expects: None,
                archetype: *archetype,
                entity: index,
            };
            Ok((inverse, Some(index)))
        }
        Operation::RemoveInteriorEntity {
            archetype, entity, ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.entities.len();
            let removed = (mlo.entities.get(*entity))
                .ok_or_else(|| format!("entity {entity} is past the {held} the interior holds"))?;
            let record = Record::entity(removed)?;
            no_lod_parent(mlo)?;
            let (_, attached) = take_out(mlo, *entity);
            let inverse = Operation::InsertInteriorEntity {
                source,
                expects: None,
                archetype: *archetype,
                entity: *entity,
                record,
                attached,
            };
            Ok((inverse, None))
        }
        Operation::InsertInteriorEntity {
            archetype,
            entity,
            record,
            attached,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.entities.len();
            if *entity > held {
                return Err(format!(
                    "entity {entity} is past the end of the {held} the interior holds"
                ));
            }
            word(held, "entity")?;
            let inserted = record.as_entity()?;
            no_lod_parent(mlo)?;
            if inserted.parent_index >= 0 {
                return Err(
                    "the record names a LOD parent, which no entity of an interior of the installs does"
                        .to_string(),
                );
            }
            attachments_hold(mlo, attached)?;
            put_in(mlo, *entity, inserted, attached);
            let inverse = Operation::RemoveInteriorEntity {
                source,
                expects: None,
                archetype: *archetype,
                entity: *entity,
            };
            Ok((inverse, Some(*entity)))
        }
        Operation::AddEntitySet {
            archetype,
            set,
            name,
            entities,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.entity_sets.len();
            let index = set.unwrap_or(held);
            if index > held {
                return Err(format!(
                    "entity set {index} is past the end of the {held} the interior holds"
                ));
            }
            let named = name_of(name);
            if named.is_empty() {
                return Err("an entity set has a name, and none is given".to_string());
            }
            if mlo
                .entity_sets
                .iter()
                .any(|set| set.name.hash == named.hash)
            {
                return Err(
                    "the interior already holds an entity set of that name, and a set is turned on by its name"
                        .to_string(),
                );
            }
            let mut built = CMloEntitySet::default();
            built.name = HashedString::from_xml_text(name);
            for held in entities {
                location_is_a_room(mlo, held.location)?;
                built.locations.push(held.location);
                built.entities.push(held.record.as_entity()?);
            }
            mlo.entity_sets.insert(index, built);
            let inverse = Operation::RemoveEntitySet {
                source,
                expects: None,
                archetype: *archetype,
                set: index,
            };
            Ok((inverse, Some(index)))
        }
        Operation::RemoveEntitySet { archetype, set, .. } => {
            let mlo = interior_at_mut(types, *archetype)?;
            // ⚠ A set whose two arrays differ goes too, which is what mends such an interior: the inverse below then states what
            // the shorter array pairs, the proof finds that it does not put the set back, and the XML as it was is given.
            let held = set_held(mlo, *set)?;
            let entities = (held.locations.iter().zip(&held.entities))
                .map(|(location, entity)| {
                    Ok(SetEntity {
                        location: *location,
                        record: Record::entity(entity)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let inverse = Operation::AddEntitySet {
                source,
                expects: None,
                archetype: *archetype,
                set: Some(*set),
                name: spelled(&held.name),
                entities,
            };
            mlo.entity_sets.remove(*set);
            Ok((inverse, None))
        }
        Operation::MoveEntityIntoSet {
            archetype,
            entity,
            set,
            location,
            at,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.entities.len();
            if *entity >= held {
                return Err(format!(
                    "entity {entity} is past the {held} the interior holds"
                ));
            }
            let holds = set_at(mlo, *set)?.entities.len();
            let place = at.unwrap_or(holds);
            if place > holds {
                return Err(format!(
                    "entity set {set} holds {holds} entities, and place {place} is past their end"
                ));
            }
            location_is_a_room(mlo, *location)?;
            no_lod_parent(mlo)?;
            let (moved, attached) = take_out(mlo, *entity);
            let into = &mut mlo.entity_sets[*set];
            into.entities.insert(place, moved);
            into.locations.insert(place, *location);
            let inverse = Operation::MoveEntityOutOfSet {
                source,
                expects: None,
                archetype: *archetype,
                set: *set,
                entity: place,
                to: Some(*entity),
                attached,
            };
            Ok((inverse, Some(place)))
        }
        Operation::MoveEntityOutOfSet {
            archetype,
            set,
            entity,
            to,
            attached,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let holds = set_at(mlo, *set)?.entities.len();
            if *entity >= holds {
                return Err(format!(
                    "entity {entity} is past the {holds} that entity set {set} holds"
                ));
            }
            let held = mlo.entities.len();
            let index = to.unwrap_or(held);
            if index > held {
                return Err(format!(
                    "entity {index} is past the end of the {held} the interior holds"
                ));
            }
            word(held, "entity")?;
            no_lod_parent(mlo)?;
            attachments_hold(mlo, attached)?;
            let from = &mut mlo.entity_sets[*set];
            let location = from.locations.remove(*entity);
            let moved = from.entities.remove(*entity);
            put_in(mlo, index, moved, attached);
            let inverse = Operation::MoveEntityIntoSet {
                source,
                expects: None,
                archetype: *archetype,
                entity: index,
                set: *set,
                location,
                at: Some(*entity),
            };
            Ok((inverse, Some(index)))
        }
        Operation::AddTimeCycleModifier {
            archetype,
            modifier,
            name,
            sphere,
            percentage,
            range,
            start_hour,
            end_hour,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let held = mlo.time_cycle_modifiers.len();
            let index = modifier.unwrap_or(held);
            if index > held {
                return Err(format!(
                    "time cycle modifier {index} is past the end of the {held} the interior holds"
                ));
            }
            if name_of(name).is_empty() {
                return Err("a time cycle modifier has a name, and none is given".to_string());
            }
            if !(finite(sphere) && finite(&[*percentage, *range])) {
                return Err(
                    "a sphere, a percentage or a range that is not a finite number cannot be stored"
                        .to_string(),
                );
            }
            let mut built = CMloTimeCycleModifier::default();
            built.name = HashedString::from_xml_text(name);
            let [x, y, z, w] = *sphere;
            built.sphere = or_io::decl::Vec4 { x, y, z, w };
            built.percentage = *percentage;
            built.range = *range;
            built.start_hour = *start_hour;
            built.end_hour = *end_hour;
            mlo.time_cycle_modifiers.insert(index, built);
            let inverse = Operation::RemoveTimeCycleModifier {
                source,
                expects: None,
                archetype: *archetype,
                modifier: index,
            };
            Ok((inverse, Some(index)))
        }
        Operation::RemoveTimeCycleModifier {
            archetype,
            modifier,
            ..
        } => {
            let mlo = interior_at_mut(types, *archetype)?;
            let holds = mlo.time_cycle_modifiers.len();
            let held = mlo.time_cycle_modifiers.get(*modifier).ok_or_else(|| {
                format!("time cycle modifier {modifier} is past the {holds} the interior holds")
            })?;
            let inverse = Operation::AddTimeCycleModifier {
                source,
                expects: None,
                archetype: *archetype,
                modifier: Some(*modifier),
                name: spelled(&held.name),
                sphere: [held.sphere.x, held.sphere.y, held.sphere.z, held.sphere.w],
                percentage: held.percentage,
                range: held.range,
                start_hour: held.start_hour,
                end_hour: held.end_hour,
            };
            mlo.time_cycle_modifiers.remove(*modifier);
            Ok((inverse, None))
        }
        other => Err(format!("{} does not edit an interior", other.name())),
    }
}

/// Refuse `name` for a room of `mlo` when another room than `but` has it, whatever its case and the white space around it.
fn name_is_free(mlo: &CMloArchetypeDef, name: &str, but: Option<usize>) -> Result<(), String> {
    let taken = (mlo.rooms.iter().enumerate())
        .any(|(at, room)| Some(at) != but && room.name.trim().eq_ignore_ascii_case(name.trim()));
    if taken {
        Err(
            "the interior already holds a room of that name, and a room is found by its name"
                .to_string(),
        )
    } else {
        Ok(())
    }
}

/// The two rooms a portal from `from` to `to` joins, refused when the interior lacks one or they are one room.
fn joined(mlo: &CMloArchetypeDef, from: u32, to: u32) -> Result<Vec<usize>, String> {
    let held = mlo.rooms.len();
    if let Some(room) = [from, to].into_iter().find(|room| *room as usize >= held) {
        return Err(format!(
            "the portal would name room {room}, past the {held} rooms the interior holds"
        ));
    }
    if from == to {
        return Err(format!(
            "a portal joins two rooms, and room {from} is named on both sides"
        ));
    }
    Ok(rooms_of(from, to, held))
}

/// Note that a portal leaves the outside, where the installs name the outside as the room a portal leads to.
fn leaves_the_outside(context: &Context, from: u32) {
    if from as usize == OUTSIDE {
        context.notes.borrow_mut().push(
            "the portal names the outside as the room it leaves (`room_from`), where every portal of the installs that touches the outside names it as the room it leads to (`room_to`)"
                .to_string(),
        );
    }
}

/// Refuse a location that is no room `mlo` holds.
fn location_is_a_room(mlo: &CMloArchetypeDef, location: u32) -> Result<(), String> {
    let held = mlo.rooms.len();
    if location & NOT_A_ROOM != 0 {
        return Err(format!(
            "the location {location} is no room index, and what else a location states this build does not read"
        ));
    }
    if location as usize >= held {
        return Err(format!(
            "the location names room {location}, past the {held} rooms the interior holds"
        ));
    }
    Ok(())
}

/// The entity set `index` of `mlo`, refused when it is past the sets.
fn set_held(mlo: &CMloArchetypeDef, index: usize) -> Result<&CMloEntitySet, String> {
    let held = mlo.entity_sets.len();
    (mlo.entity_sets.get(index))
        .ok_or_else(|| format!("entity set {index} is past the {held} the interior holds"))
}

/// The entity set `index` of `mlo`, refused when it is past the sets or its two arrays differ.
fn set_at(mlo: &CMloArchetypeDef, index: usize) -> Result<&CMloEntitySet, String> {
    let set = set_held(mlo, index)?;
    if set.locations.len() != set.entities.len() {
        return Err(format!(
            "entity set {index} has {} locations for {} entities, so no entity of it has a place",
            set.locations.len(),
            set.entities.len()
        ));
    }
    Ok(set)
}
