//! An interior's own records, typed: the rooms, portals, entity sets and time cycle modifiers of a
//! [`CMloArchetypeDef`], each with the members the installs state for it and no other.
//!
//! ⚠ **Three numbers here subscript an array the file states apart.** A room's and a portal's
//! `attachedObjects` are positions in the interior's `entities`, a portal's `roomFrom` and `roomTo`
//! are positions in its `rooms`, and an entity set's `locations` runs beside its `entities` under
//! one subscript. The engine trusts each, nothing here renumbers them when an entity or a room is
//! taken out, and a save refuses the interior where one misses ([`broken`]).
//!
//! ⚠ No class of these four is declared for a file that states none (`world_types`): a record is
//! added as a copy of one the file holds, and one built here saves only into a file whose own table
//! states its class.

use super::*;

meta_record! {
    /// `CMloRoomDef`: one room of an interior; its name is text, where every other name of the family is a hash.
    CMloRoomDef = "CMloRoomDef" {
        name: String = "name",
        bb_min: Vec3 = "bbMin",
        bb_max: Vec3 = "bbMax",
        blend: f32 = "blend",
        timecycle_name: HashedString = "timecycleName",
        secondary_timecycle_name: HashedString = "secondaryTimecycleName",
        flags: u32 = "flags",
        portal_count: u32 = "portalCount",
        floor_id: i32 = "floorId",
        exterior_visibilty_depth: i32 = "exteriorVisibiltyDepth" => -1,
        attached_objects: Vec<u32> = "attachedObjects",
    }
}

meta_record! {
    /// `CMloPortalDef`: an opening between two rooms, as the corners of its quad in the interior's space.
    CMloPortalDef = "CMloPortalDef" {
        room_from: u32 = "roomFrom",
        room_to: u32 = "roomTo",
        flags: u32 = "flags",
        mirror_priority: u32 = "mirrorPriority",
        opacity: u32 = "opacity",
        audio_occlusion: u32 = "audioOcclusion",
        corners: Vec<Vec3> = "corners",
        attached_objects: Vec<u32> = "attachedObjects",
    }
}

meta_record! {
    /// `CMloEntitySet`: entities an instance draws only where it names the set, each with the room it stands in.
    CMloEntitySet = "CMloEntitySet" {
        name: HashedString = "name",
        locations: Vec<u32> = "locations",
        entities: Vec<CEntityDefAny> = "entities",
    }
}

meta_record! {
    /// `CMloTimeCycleModifier`: a time cycle modifier applied inside a sphere of the interior, between two hours.
    CMloTimeCycleModifier = "CMloTimeCycleModifier" {
        name: HashedString = "name",
        sphere: Vec4 = "sphere",
        percentage: f32 = "percentage",
        range: f32 = "range",
        start_hour: u32 = "startHour",
        end_hour: u32 = "endHour",
    }
}

/// The members each class of an interior types, by the class that states them.
pub(super) const MEMBERS: &[(&str, &[&str])] = &[
    ("CMloArchetypeDef", CMloArchetypeDef::MEMBERS),
    ("CMloRoomDef", CMloRoomDef::MEMBERS),
    ("CMloPortalDef", CMloPortalDef::MEMBERS),
    ("CMloEntitySet", CMloEntitySet::MEMBERS),
    ("CMloTimeCycleModifier", CMloTimeCycleModifier::MEMBERS),
];

/// A number of an interior that subscripts an array the interior does not hold that far.
#[derive(Debug, thiserror::Error)]
pub(super) enum Broken {
    #[error("{holder} {at} attaches object {index}, past the {len} entities in this interior")]
    AttachedObjectOutOfRange {
        holder: &'static str,
        at: usize,
        index: u32,
        len: usize,
    },
    #[error("portal {portal} names room {room}, past the {len} rooms in this interior")]
    PortalRoomOutOfRange {
        portal: usize,
        room: u32,
        len: usize,
    },
    #[error("entity set {set} has {locations} locations for {entities} entities")]
    EntitySetArraysDiffer {
        set: usize,
        locations: usize,
        entities: usize,
    },
}

/// The first number of `mlo` that misses the array it subscripts: rooms, then portals, then entity sets.
pub(super) fn broken(mlo: &CMloArchetypeDef) -> Option<Broken> {
    let len = mlo.entities.len();
    let attached = |holder: &'static str, at: usize, attached: &[u32]| {
        let index = *attached.iter().find(|&&index| index as usize >= len)?;
        Some(Broken::AttachedObjectOutOfRange {
            holder,
            at,
            index,
            len,
        })
    };
    let rooms = (mlo.rooms.iter().enumerate())
        .find_map(|(at, room)| attached("room", at, &room.attached_objects));
    let portals = || {
        let len = mlo.rooms.len();
        mlo.portals.iter().enumerate().find_map(|(portal, p)| {
            let room = [p.room_from, p.room_to]
                .into_iter()
                .find(|&room| room as usize >= len);
            match room {
                Some(room) => Some(Broken::PortalRoomOutOfRange { portal, room, len }),
                None => attached("portal", portal, &p.attached_objects),
            }
        })
    };
    let sets = || {
        mlo.entity_sets.iter().enumerate().find_map(|(set, s)| {
            let (locations, entities) = (s.locations.len(), s.entities.len());
            (locations != entities).then_some(Broken::EntitySetArraysDiffer {
                set,
                locations,
                entities,
            })
        })
    };
    rooms.or_else(portals).or_else(sets)
}

/// Whether a record of `mlo` was built here or moved from another type, so no arrival layout has its shape.
pub(super) fn holds_a_record_built_here(mlo: &CMloArchetypeDef) -> bool {
    mlo.entities.iter().any(CEntityDefAny::reshaped)
        || (mlo.rooms.iter()).any(|r| built_here(&r.carry, "CMloRoomDef"))
        || (mlo.portals.iter()).any(|p| built_here(&p.carry, "CMloPortalDef"))
        || (mlo.time_cycle_modifiers.iter()).any(|m| built_here(&m.carry, "CMloTimeCycleModifier"))
        || mlo.entity_sets.iter().any(|s| {
            built_here(&s.carry, "CMloEntitySet") || s.entities.iter().any(CEntityDefAny::reshaped)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::typed::world_types::WORLD_TYPES;
    use crate::meta::{build_library, canonical, detect_binary_format};

    /// What the installs state for each record of an interior, by name
    /// (`docs/direction/Workbench_Views_Design.md`, "What wave P44 built for Maps").
    const STATED: &[(&str, &[&str])] = &[
        (
            "CMloRoomDef",
            &[
                "attachedObjects",
                "bbMax",
                "bbMin",
                "blend",
                "exteriorVisibiltyDepth",
                "flags",
                "floorId",
                "name",
                "portalCount",
                "secondaryTimecycleName",
                "timecycleName",
            ],
        ),
        (
            "CMloPortalDef",
            &[
                "attachedObjects",
                "audioOcclusion",
                "corners",
                "flags",
                "mirrorPriority",
                "opacity",
                "roomFrom",
                "roomTo",
            ],
        ),
        ("CMloEntitySet", &["entities", "locations", "name"]),
        (
            "CMloTimeCycleModifier",
            &[
                "endHour",
                "name",
                "percentage",
                "range",
                "sphere",
                "startHour",
            ],
        ),
    ];

    /// The classes of a type file with an interior, each record with the members [`STATED`] lists
    /// and the room with one more, which no record types.
    const INTERIOR_TYPES: &str = r#"
  <structdef type="CMloRoomDef">
    <string name="name" type="atString"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <float name="blend"/>
    <string name="timecycleName" type="atHashString"/>
    <string name="secondaryTimecycleName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="portalCount"/>
    <s32 name="floorId"/>
    <s32 name="exteriorVisibiltyDepth" init="-1"/>
    <array name="attachedObjects" type="atArray"><u32/></array>
    <u32 name="aMemberNoRecordTypes"/>
  </structdef>
  <structdef type="CMloPortalDef">
    <u32 name="roomFrom"/>
    <u32 name="roomTo"/>
    <u32 name="flags"/>
    <u32 name="mirrorPriority"/>
    <u32 name="opacity"/>
    <u32 name="audioOcclusion"/>
    <array name="corners" type="atArray"><Vector3/></array>
    <array name="attachedObjects" type="atArray"><u32/></array>
  </structdef>
  <structdef type="CMloEntitySet">
    <string name="name" type="atHashString"/>
    <array name="locations" type="atArray"><u32/></array>
    <array name="entities" type="atArray"><pointer type="rage::fwEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CMloTimeCycleModifier">
    <string name="name" type="atHashString"/>
    <Vector4 name="sphere"/>
    <float name="percentage"/>
    <float name="range"/>
    <u32 name="startHour"/>
    <u32 name="endHour"/>
  </structdef>
  <structdef type="CMloArchetypeDef" base="CBaseArchetypeDef">
    <u32 name="mloFlags"/>
    <array name="entities" type="atArray"><pointer type="rage::fwEntityDef" policy="owner"/></array>
    <array name="rooms" type="atArray"><struct type="CMloRoomDef"/></array>
    <array name="portals" type="atArray"><struct type="CMloPortalDef"/></array>
    <array name="entitySets" type="atArray"><struct type="CMloEntitySet"/></array>
    <array name="timeCycleModifiers" type="atArray"><struct type="CMloTimeCycleModifier"/></array>
  </structdef>
</ParserSchema>"#;

    fn library() -> SchemaLibrary {
        let psc = WORLD_TYPES.replace("</ParserSchema>", INTERIOR_TYPES);
        build_library([("interior.psc", psc.as_bytes())]).expect("the interior's classes parse")
    }

    /// A `class` record of `library` with `members` set, every other member at its empty value.
    fn record(library: &SchemaLibrary, class: &str, members: &[(&str, Value)]) -> MetaInstance {
        let mut inst = MetaInstance::new(class);
        conform(library, &mut inst).expect("the class conforms");
        for (member, value) in members {
            // ⚠ By hash: `conform` spells a member as the index of names does, which lacks some.
            let key = name_hash(member);
            let slot = inst.fields.iter_mut().find(|(n, _)| name_hash(n) == key);
            *slot.map(|(_, v)| v).expect("the class has the member") = value.clone();
        }
        inst
    }

    fn hash(name: &str) -> Value {
        Value::Hash(HashedString::new(name))
    }

    fn structs(items: Vec<MetaInstance>) -> Value {
        Value::Array(
            items
                .into_iter()
                .map(|i| Value::Struct(Box::new(i)))
                .collect(),
        )
    }

    fn pointers(items: Vec<MetaInstance>) -> Value {
        let behind = |i| Value::Pointer(Some(Box::new(i)));
        Value::Array(items.into_iter().map(behind).collect())
    }

    fn indexes(items: &[u32]) -> Value {
        Value::Array(items.iter().map(|&i| Value::UInt(u64::from(i))).collect())
    }

    fn entity(library: &SchemaLibrary, archetype: &str, guid: u32) -> MetaInstance {
        let members = [
            ("archetypeName", hash(archetype)),
            ("guid", Value::UInt(u64::from(guid))),
            ("position", Value::Vec3([guid as f32, 2.0, 3.0])),
            ("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0])),
            ("scaleXY", Value::Float(1.0)),
            ("scaleZ", Value::Float(1.0)),
            ("parentIndex", Value::Int(-1)),
            ("lodDist", Value::Float(60.0)),
        ];
        record(library, "CEntityDef", &members)
    }

    fn room(library: &SchemaLibrary, name: &str, floor: i32, attached: &[u32]) -> MetaInstance {
        let members = [
            ("name", Value::String(name.to_string())),
            ("bbMin", Value::Vec3([-4.0, -3.0, floor as f32])),
            ("bbMax", Value::Vec3([4.0, 3.0, floor as f32 + 2.5])),
            ("blend", Value::Float(0.75)),
            ("timecycleName", hash("int_hall")),
            ("secondaryTimecycleName", hash("int_hall_dark")),
            ("flags", Value::UInt(96)),
            ("portalCount", Value::UInt(7)),
            ("floorId", Value::Int(i64::from(floor))),
            ("exteriorVisibiltyDepth", Value::Int(2)),
            ("attachedObjects", indexes(attached)),
            ("aMemberNoRecordTypes", Value::UInt(9)),
        ];
        record(library, "CMloRoomDef", &members)
    }

    const CORNERS: [[f32; 3]; 4] = [
        [1.0, 3.0, 0.0],
        [-1.0, 3.0, 0.0],
        [-1.0, 3.0, 2.0],
        [1.0, 3.0, 2.0],
    ];

    fn portal(library: &SchemaLibrary, from: u32, to: u32, attached: &[u32]) -> MetaInstance {
        let members = [
            ("roomFrom", Value::UInt(u64::from(from))),
            ("roomTo", Value::UInt(u64::from(to))),
            ("flags", Value::UInt(8)),
            ("mirrorPriority", Value::UInt(3)),
            ("opacity", Value::UInt(40)),
            ("audioOcclusion", Value::UInt(0x1234_5678)),
            (
                "corners",
                Value::Array(CORNERS.iter().map(|c| Value::Vec3(*c)).collect()),
            ),
            ("attachedObjects", indexes(attached)),
        ];
        record(library, "CMloPortalDef", &members)
    }

    fn entity_set(library: &SchemaLibrary, name: &str, held: &[(u32, u32)]) -> MetaInstance {
        let locations: Vec<u32> = held.iter().map(|(room, _)| *room).collect();
        let entities = (held.iter())
            .map(|(_, guid)| entity(library, "prop_party_lamp", *guid))
            .collect();
        let members = [
            ("name", hash(name)),
            ("locations", indexes(&locations)),
            ("entities", pointers(entities)),
        ];
        record(library, "CMloEntitySet", &members)
    }

    fn modifier(library: &SchemaLibrary) -> MetaInstance {
        let members = [
            ("name", hash("int_hall_warm")),
            ("sphere", Value::Vec4([1.0, 2.0, 3.0, 6.5])),
            ("percentage", Value::Float(80.0)),
            ("range", Value::Float(4.5)),
            ("startHour", Value::UInt(20)),
            ("endHour", Value::UInt(6)),
        ];
        record(library, "CMloTimeCycleModifier", &members)
    }

    /// The tree of a type file with one interior: two rooms, a portal between them, two entity sets and a modifier.
    fn interior_tree(library: &SchemaLibrary) -> MetaInstance {
        let members = [
            ("name", hash("v_int_hills")),
            ("lodDist", Value::Float(120.0)),
            ("mloFlags", Value::UInt(1)),
            (
                "entities",
                pointers(vec![
                    entity(library, "prop_bench_01a", 1),
                    entity(library, "prop_bin_01a", 2),
                    entity(library, "v_ilev_hall_door", 3),
                ]),
            ),
            (
                "rooms",
                structs(vec![
                    room(library, "limbo", 0, &[]),
                    room(library, "hall", 1, &[0, 1]),
                ]),
            ),
            ("portals", structs(vec![portal(library, 0, 1, &[2])])),
            (
                "entitySets",
                structs(vec![
                    entity_set(library, "set_party", &[(1, 10), (1, 11)]),
                    entity_set(library, "set_empty", &[(1, 12)]),
                ]),
            ),
            ("timeCycleModifiers", structs(vec![modifier(library)])),
        ];
        let interior = record(library, "CMloArchetypeDef", &members);
        let members = [
            ("name", hash("hills_interiors")),
            ("archetypes", pointers(vec![interior])),
        ];
        record(library, "CMapTypes", &members)
    }

    fn encoded(library: &SchemaLibrary, root: &MetaInstance, format: BinaryFormat) -> Vec<u8> {
        let layout = canonical::layout_for(library, root, format, None).expect("lays out");
        write_meta_binary(format, Some(&layout), root).expect("encodes")
    }

    fn interior_file(format: BinaryFormat) -> Vec<u8> {
        let library = library();
        encoded(&library, &interior_tree(&library), format)
    }

    const FORMATS: [BinaryFormat; 2] = [BinaryFormat::Pso, BinaryFormat::Rbm];

    fn interior(types: &CMapTypes) -> &CMloArchetypeDef {
        match &types.archetypes[0] {
            CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => mlo,
            other => panic!("the archetype is the interior, and it read as {other:?}"),
        }
    }

    fn interior_mut(types: &mut CMapTypes) -> &mut CMloArchetypeDef {
        match &mut types.archetypes[0] {
            CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => mlo,
            other => panic!("the archetype is the interior, and it read as {other:?}"),
        }
    }

    /// The tree `bytes` reads as, by its own type table and no schema.
    fn tree(bytes: &[u8]) -> MetaInstance {
        MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec())
            .expect("the save reads")
            .instance
    }

    /// The `index`th element of `array` in the first archetype of `root`.
    fn element<'a>(root: &'a MetaInstance, array: &str, index: usize) -> &'a MetaInstance {
        let Some(Value::Array(archetypes)) = field(root, "archetypes") else {
            panic!("the type file holds its archetypes");
        };
        let Some(Value::Pointer(Some(interior))) = archetypes.first() else {
            panic!("the type file holds its interior");
        };
        match field(interior, array) {
            Some(Value::Array(items)) => match &items[index] {
                Value::Struct(inst) | Value::Pointer(Some(inst)) => inst,
                other => panic!("`{array}` holds records, and one is {other:?}"),
            },
            other => panic!("the interior holds `{array}`, and it is {other:?}"),
        }
    }

    /// `edited` saved, read back equal to what was saved, and saved again to the same bytes.
    fn saved_to_its_fixed_point(edited: &CMapTypes, format: BinaryFormat) -> (Vec<u8>, CMapTypes) {
        let saved = edited.to_bytes().expect("the edited interior saves");
        assert_eq!(detect_binary_format(&saved), Some(format));
        let back = CMapTypes::from_bytes(&saved).expect("and reopens");
        assert_eq!(&back, edited, "what was saved is what reads back");
        assert_eq!(
            back.to_bytes().expect("saves again"),
            saved,
            "a second save is the first"
        );
        (saved, back)
    }

    #[test]
    fn each_record_types_the_members_the_installs_state_and_no_other() {
        let typed = [
            ("CMloRoomDef", CMloRoomDef::MEMBERS),
            ("CMloPortalDef", CMloPortalDef::MEMBERS),
            ("CMloEntitySet", CMloEntitySet::MEMBERS),
            ("CMloTimeCycleModifier", CMloTimeCycleModifier::MEMBERS),
        ];
        assert_eq!(typed.len(), STATED.len());
        for ((class, typed), (stated_class, stated)) in typed.iter().zip(STATED) {
            assert_eq!(class, stated_class);
            let mut typed: Vec<&str> = typed.to_vec();
            typed.sort_unstable();
            assert_eq!(&typed, stated, "`{class}`");
            assert!(MEMBERS.contains(&(*class, typed_members(class))));
        }
        assert_eq!(
            CMloArchetypeDef::MEMBERS,
            [
                "mloFlags",
                "entities",
                "rooms",
                "portals",
                "entitySets",
                "timeCycleModifiers"
            ]
        );
    }

    fn typed_members(class: &str) -> &'static [&'static str] {
        (MEMBERS.iter())
            .find_map(|(of, members)| (*of == class).then_some(*members))
            .expect("the class is one of an interior")
    }

    #[test]
    fn each_record_of_an_interior_reads_to_the_values_built_and_saves_to_the_same_bytes() {
        for format in FORMATS {
            let bytes = interior_file(format);
            let types = CMapTypes::from_bytes(&bytes).expect("opens typed");
            let mlo = interior(&types);
            assert_eq!(mlo.mlo_flags, 1);
            assert_eq!(mlo.entities.len(), 3);

            assert_eq!(mlo.rooms.len(), 2);
            let hall = &mlo.rooms[1];
            assert_eq!(hall.name, "hall");
            assert_eq!(
                (hall.bb_min, hall.bb_max),
                (
                    Vec3 {
                        x: -4.0,
                        y: -3.0,
                        z: 1.0
                    },
                    Vec3 {
                        x: 4.0,
                        y: 3.0,
                        z: 3.5
                    }
                )
            );
            assert_eq!(hall.blend, 0.75);
            assert!(hall.timecycle_name == "int_hall");
            assert!(hall.secondary_timecycle_name == "int_hall_dark");
            assert_eq!((hall.flags, hall.portal_count, hall.floor_id), (96, 7, 1));
            assert_eq!(hall.exterior_visibilty_depth, 2);
            assert_eq!(hall.attached_objects, [0, 1]);
            assert_eq!(mlo.rooms[0].name, "limbo");
            assert!(mlo.rooms[0].attached_objects.is_empty());

            assert_eq!(mlo.portals.len(), 1);
            let door = &mlo.portals[0];
            assert_eq!((door.room_from, door.room_to, door.flags), (0, 1, 8));
            assert_eq!(
                (door.mirror_priority, door.opacity, door.audio_occlusion),
                (3, 40, 0x1234_5678)
            );
            let corners: Vec<[f32; 3]> = door.corners.iter().map(|c| [c.x, c.y, c.z]).collect();
            assert_eq!(corners, CORNERS);
            assert_eq!(door.attached_objects, [2]);

            assert_eq!(mlo.entity_sets.len(), 2);
            let party = &mlo.entity_sets[0];
            assert!(party.name == "set_party");
            assert_eq!(party.locations, [1, 1]);
            assert_eq!(party.entities.len(), 2);
            assert!(party.entities[1].archetype_name == "prop_party_lamp");
            assert_eq!(party.entities[1].guid, 11);

            assert_eq!(mlo.time_cycle_modifiers.len(), 1);
            let warm = &mlo.time_cycle_modifiers[0];
            assert!(warm.name == "int_hall_warm");
            assert_eq!(
                warm.sphere,
                Vec4 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    w: 6.5
                }
            );
            assert_eq!((warm.percentage, warm.range), (80.0, 4.5));
            assert_eq!((warm.start_hour, warm.end_hour), (20, 6));

            assert_eq!(
                types.to_bytes().expect("an untouched save"),
                bytes,
                "{format:?}: an untouched interior saves to the bytes it arrived as"
            );
        }
    }

    #[test]
    fn a_room_renamed_saves_reads_back_and_keeps_the_member_no_record_types() {
        for format in FORMATS {
            let mut types = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
            interior_mut(&mut types).rooms[1].name = "the long gallery".to_string();
            let (saved, back) = saved_to_its_fixed_point(&types, format);
            assert_eq!(interior(&back).rooms[1].name, "the long gallery");
            assert_eq!(interior(&back).rooms[0].name, "limbo");

            let root = tree(&saved);
            let hall = element(&root, "rooms", 1);
            assert_eq!(
                field(hall, "aMemberNoRecordTypes"),
                Some(&Value::UInt(9)),
                "{format:?}: a member the record does not name survives an edit of one it names"
            );
            assert_eq!(
                field(hall, "name"),
                Some(&Value::String("the long gallery".into()))
            );
        }
    }

    #[test]
    fn a_portal_whose_rooms_are_swapped_saves_and_reads_back() {
        for format in FORMATS {
            let bytes = interior_file(format);
            let mut types = CMapTypes::from_bytes(&bytes).expect("opens typed");
            let door = &mut interior_mut(&mut types).portals[0];
            std::mem::swap(&mut door.room_from, &mut door.room_to);
            let (saved, back) = saved_to_its_fixed_point(&types, format);
            let door = &interior(&back).portals[0];
            assert_eq!((door.room_from, door.room_to), (1, 0));
            assert_eq!(door.audio_occlusion, 0x1234_5678);
            assert_ne!(saved, bytes);

            // Swapped back, the file is the one it was: nothing else moved.
            let mut types = back;
            let door = &mut interior_mut(&mut types).portals[0];
            std::mem::swap(&mut door.room_from, &mut door.room_to);
            assert_eq!(types.to_bytes().expect("saves"), bytes);
        }
    }

    #[test]
    fn an_entity_moved_from_one_entity_set_to_another_saves_and_reads_back() {
        for format in FORMATS {
            let mut types = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
            let mlo = interior_mut(&mut types);
            let moved = mlo.entity_sets[0].entities.remove(1);
            let location = mlo.entity_sets[0].locations.remove(1);
            mlo.entity_sets[1].entities.push(moved);
            mlo.entity_sets[1].locations.push(location);
            let (_, back) = saved_to_its_fixed_point(&types, format);
            let sets = &interior(&back).entity_sets;
            let guids =
                |set: &CMloEntitySet| -> Vec<u32> { set.entities.iter().map(|e| e.guid).collect() };
            assert_eq!(guids(&sets[0]), [10]);
            assert_eq!(guids(&sets[1]), [12, 11]);
            assert_eq!(sets[0].locations, [1]);
            assert_eq!(sets[1].locations, [1, 1]);
        }
    }

    #[test]
    fn a_room_added_as_a_copy_of_one_the_file_holds_saves_and_reads_back() {
        for format in FORMATS {
            let mut types = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
            let mlo = interior_mut(&mut types);
            let mut attic = mlo.rooms[1].clone();
            attic.name = "attic".to_string();
            attic.floor_id = 2;
            attic.attached_objects.clear();
            mlo.rooms.push(attic);
            let (saved, back) = saved_to_its_fixed_point(&types, format);
            let rooms = &interior(&back).rooms;
            assert_eq!(rooms.len(), 3);
            assert_eq!((rooms[2].name.as_str(), rooms[2].floor_id), ("attic", 2));
            assert!(rooms[2].timecycle_name == "int_hall");
            assert_eq!(rooms[1].attached_objects, [0, 1]);
            let root = tree(&saved);
            assert_eq!(
                field(element(&root, "rooms", 2), "aMemberNoRecordTypes"),
                Some(&Value::UInt(9)),
                "the copy keeps what the room it copies states"
            );
        }
    }

    /// What `to_bytes` refuses `types` with.
    fn refusal(types: &CMapTypes) -> String {
        let refused = types
            .to_bytes()
            .expect_err("a broken interior is not saved");
        assert!(matches!(refused, Error::Refused { .. }), "{refused}");
        refused.to_string()
    }

    #[test]
    fn a_broken_interior_is_refused_at_the_save_with_its_sentence() {
        for format in FORMATS {
            let opened = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");

            let mut types = opened.clone();
            interior_mut(&mut types).rooms[1].attached_objects.push(3);
            let why = refusal(&types);
            let said =
                "archetype 0: room 1 attaches object 3, past the 3 entities in this interior";
            assert!(why.ends_with(said), "{why}");

            let mut types = opened.clone();
            interior_mut(&mut types).entities.pop();
            let why = refusal(&types);
            let said =
                "archetype 0: portal 0 attaches object 2, past the 2 entities in this interior";
            assert!(why.ends_with(said), "{why}");

            let mut types = opened.clone();
            interior_mut(&mut types).portals[0].room_to = 2;
            let why = refusal(&types);
            let said = "archetype 0: portal 0 names room 2, past the 2 rooms in this interior";
            assert!(why.ends_with(said), "{why}");

            let mut types = opened.clone();
            interior_mut(&mut types).portals[0].room_from = 2;
            let why = refusal(&types);
            assert!(why.ends_with(said), "the room a portal leaves: {why}");

            let mut types = opened.clone();
            interior_mut(&mut types).rooms.pop();
            let why = refusal(&types);
            let said = "archetype 0: portal 0 names room 1, past the 1 rooms in this interior";
            assert!(why.ends_with(said), "{why}");

            let mut types = opened.clone();
            interior_mut(&mut types).entity_sets[0].entities.pop();
            let why = refusal(&types);
            let said = "archetype 0: entity set 0 has 2 locations for 1 entities";
            assert!(why.ends_with(said), "{why}");

            let mut types = opened.clone();
            interior_mut(&mut types).entity_sets[1].locations.clear();
            let why = refusal(&types);
            let said = "archetype 0: entity set 1 has 0 locations for 1 entities";
            assert!(why.ends_with(said), "{why}");

            opened
                .to_bytes()
                .expect("and the interior as it came saves");
        }
    }

    #[test]
    fn a_room_built_here_takes_the_members_the_files_own_class_states() {
        for format in FORMATS {
            let mut types = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
            let built = CMloRoomDef {
                name: "cellar".to_string(),
                floor_id: -1,
                ..Default::default()
            };
            assert_eq!(built.exterior_visibilty_depth, -1, "the engine's init");
            interior_mut(&mut types).rooms.push(built);
            let saved = types.to_bytes().expect("a room built here saves");
            assert_eq!(detect_binary_format(&saved), Some(format));
            let root = tree(&saved);
            let stated = |inst: &MetaInstance| -> Vec<u32> {
                inst.fields.iter().map(|(n, _)| name_hash(n)).collect()
            };
            assert_eq!(
                stated(element(&root, "rooms", 2)),
                stated(element(&root, "rooms", 1)),
                "the room built here states what the file's own class states"
            );
            let back = CMapTypes::from_bytes(&saved).expect("reopens");
            let cellar = &interior(&back).rooms[2];
            assert_eq!((cellar.name.as_str(), cellar.floor_id), ("cellar", -1));
            assert_eq!(cellar.exterior_visibilty_depth, -1);
            assert_eq!(back.to_bytes().expect("saves again"), saved);
        }
    }

    /// The members `inst` states, by name hash, in the order it states them.
    fn stated(inst: &MetaInstance) -> Vec<u32> {
        inst.fields.iter().map(|(n, _)| name_hash(n)).collect()
    }

    /// ⚠ As a hash alone, which is how a name the index lacks reads back: two names are equal when their text is too.
    fn hashed(name: &str) -> HashedString {
        HashedString::from_hash(crate::string_hash(name.as_bytes()))
    }

    fn lamp(guid: u32) -> CEntityDefAny {
        CEntityDefAny::Base(CEntityDef {
            archetype_name: hashed("prop_party_lamp"),
            guid,
            parent_index: -1,
            ..Default::default()
        })
    }

    /// ⚠ One kind to a save: a save is told that it holds a record built here by any one of them, so two kinds in one
    /// save would each hide that the other went unseen.
    #[test]
    fn each_kind_of_record_built_here_takes_the_members_the_files_own_class_states() {
        type Build = fn(&mut CMloArchetypeDef);
        let builds: [(&str, Build); 5] = [
            ("portals", |mlo| {
                let corners = CORNERS.map(|[x, y, z]| Vec3 { x, y, z }).to_vec();
                mlo.portals.push(CMloPortalDef {
                    room_from: 1,
                    opacity: 9,
                    corners,
                    ..Default::default()
                });
            }),
            ("timeCycleModifiers", |mlo| {
                mlo.time_cycle_modifiers.push(CMloTimeCycleModifier {
                    name: hashed("int_hall_cold"),
                    range: 2.0,
                    end_hour: 9,
                    ..Default::default()
                });
            }),
            ("entitySets", |mlo| {
                mlo.entity_sets.push(CMloEntitySet {
                    name: hashed("set_built"),
                    ..Default::default()
                });
            }),
            ("entities", |mlo| mlo.entities.push(lamp(4))),
            ("an entity set's entities", |mlo| {
                mlo.entity_sets[1].entities.push(lamp(13));
                mlo.entity_sets[1].locations.push(0);
            }),
        ];
        for format in FORMATS {
            for (array, build) in builds {
                let mut types = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
                build(interior_mut(&mut types));
                let (saved, back) = saved_to_its_fixed_point(&types, format);
                let root = tree(&saved);
                let (built, sibling) = match array {
                    "an entity set's entities" => {
                        let Some(Value::Array(held)) =
                            field(element(&root, "entitySets", 1), "entities")
                        else {
                            panic!("the entity set holds its entities");
                        };
                        let [Value::Pointer(Some(sibling)), Value::Pointer(Some(built))] =
                            &held[..]
                        else {
                            panic!("the set holds the entity it held and the one built, and holds {held:?}");
                        };
                        (&**built, &**sibling)
                    }
                    _ => {
                        let held = match array {
                            "portals" => interior(&back).portals.len(),
                            "timeCycleModifiers" => interior(&back).time_cycle_modifiers.len(),
                            "entitySets" => interior(&back).entity_sets.len(),
                            _ => interior(&back).entities.len(),
                        };
                        (element(&root, array, held - 1), element(&root, array, 0))
                    }
                };
                assert_eq!(
                    stated(built),
                    stated(sibling),
                    "{format:?}: a record built into `{array}` states what the file's own class states"
                );
            }
        }
    }

    #[test]
    fn an_interior_copied_into_a_file_that_states_no_class_for_it_is_refused_and_not_laid_out() {
        let library = library();
        let members = [("name", hash("hills_interiors"))];
        let empty = record(&library, "CMapTypes", &members);
        for format in FORMATS {
            let donor = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
            let mut types =
                CMapTypes::from_bytes(&encoded(&library, &empty, format)).expect("opens typed");
            types.archetypes.push(donor.archetypes[0].clone());
            let refused = types
                .to_bytes()
                .expect_err("no class of an interior is stated here or declared");
            assert!(matches!(refused, Error::Save { .. }), "{refused}");
            let why = refused.to_string();
            assert!(why.contains("`CMloArchetypeDef` copied here"), "{why}");
            assert!(!why.contains("synth"), "{why}");
            assert_eq!(
                why.contains("signature"),
                format == BinaryFormat::Rbm,
                "{why}"
            );
        }
    }

    /// ⚠ A text read with no schema calls a room `Item`, the element that holds it, and never by its class.
    #[test]
    fn a_text_type_file_read_with_no_schema_saves_its_interior_as_it_came_and_takes_an_edit() {
        let mut limbo = MetaInstance::new("Item");
        limbo.set("name", Value::String("limbo".into()));
        limbo.set("floorId", Value::Int(0));
        limbo.set("aMemberNoRecordTypes", Value::Int(9));
        let mut mlo = MetaInstance::new("CMloArchetypeDef");
        mlo.set("name", Value::String("v_int_hills".into()));
        mlo.set("rooms", structs(vec![limbo]));
        let mut root = MetaInstance::new("CMapTypes");
        root.set("archetypes", pointers(vec![mlo]));
        let text = write_meta(&root).expect("writes as text").into_bytes();

        let mut types = CMapTypes::from_bytes(&text).expect("opens typed");
        assert_eq!(interior(&types).rooms[0].name, "limbo");
        assert_eq!(
            String::from_utf8(types.to_bytes().expect("an untouched save")),
            String::from_utf8(text),
            "an untouched text saves as it came"
        );
        interior_mut(&mut types).rooms[0].name = "cellar".to_string();
        let saved = types.to_bytes().expect("a renamed room saves");
        let back = CMapTypes::from_bytes(&saved).expect("reopens");
        assert_eq!(back, types);
        assert_eq!(interior(&back).rooms[0].name, "cellar");
        let hall = tree(&saved);
        assert_eq!(
            field(element(&hall, "rooms", 0), "aMemberNoRecordTypes"),
            Some(&Value::Int(9))
        );

        // A text has no table to lay a record out by, so one built here is refused as built, and as nothing else.
        interior_mut(&mut types).rooms.push(CMloRoomDef::default());
        let why = types
            .to_bytes()
            .expect_err("no class is laid out here")
            .to_string();
        assert!(why.contains("`CMloRoomDef` built"), "{why}");
        assert!(!why.contains("copied here"), "{why}");
    }

    /// A text type file with one interior of one room, which states `members`.
    fn text_of_a_room(members: &[(&str, Value)]) -> Vec<u8> {
        let mut limbo = MetaInstance::new("Item");
        for (member, value) in members {
            limbo.set(*member, value.clone());
        }
        let mut mlo = MetaInstance::new("CMloArchetypeDef");
        mlo.set("name", Value::String("v_int_hills".into()));
        mlo.set("rooms", structs(vec![limbo]));
        let mut root = MetaInstance::new("CMapTypes");
        root.set("archetypes", pointers(vec![mlo]));
        write_meta(&root).expect("writes as text").into_bytes()
    }

    /// ⚠ Such a room is called `Item`, so which of its members are typed is told by the record that read it: told by
    /// that name, `blend` is carried, and the value a save replaced is still compared.
    #[test]
    fn a_text_room_edited_in_a_member_only_an_interior_types_reads_back_equal() {
        let members = [
            ("name", Value::String("limbo".into())),
            ("blend", Value::Float(0.75)),
            ("portalCount", Value::Int(1)),
            ("aMemberNoRecordTypes", Value::Int(9)),
        ];
        let mut types = CMapTypes::from_bytes(&text_of_a_room(&members)).expect("opens typed");
        let limbo = &mut interior_mut(&mut types).rooms[0];
        (limbo.blend, limbo.portal_count) = (0.5, 2);
        let saved = types.to_bytes().expect("the edited room saves");
        let back = CMapTypes::from_bytes(&saved).expect("reopens");
        assert_eq!(interior(&back).rooms[0].blend, 0.5);
        assert_eq!(interior(&back).rooms[0].portal_count, 2);
        assert_eq!(back, types, "what was saved is what reads back");
        assert_eq!(back.to_bytes().expect("saves again"), saved);

        // And a member no record types still tells two rooms apart.
        let mut other = members.clone();
        other[3].1 = Value::Int(8);
        let other = CMapTypes::from_bytes(&text_of_a_room(&other)).expect("opens typed");
        let same = CMapTypes::from_bytes(&text_of_a_room(&members)).expect("opens typed");
        assert_eq!(interior(&same).rooms[0], interior(&same).rooms[0].clone());
        assert_ne!(interior(&other).rooms[0], interior(&same).rooms[0]);
    }

    /// `types` as a file whose table names `class` for an array that holds nothing, and states it nowhere.
    ///
    /// ⚠ A table states the classes its document holds, so a shipped interior with no room names the room class in
    /// `rooms` and states it nowhere. A file built here states it all the same, so the statement is taken out of the
    /// layout the file arrived with.
    fn naming_only(types: &mut CMapTypes, class: &str) {
        use crate::meta::binary::pso::schema::{PsoEnumInfo, PsoSchemaItem};
        let class = name_hash(class);
        let arrival = types.carry.arrival.as_mut().expect("the file arrived");
        let layout = arrival.layout.as_mut().expect("as a binary document");
        match &mut layout.container {
            ContainerLayout::Rbm(rbm) => {
                let stated = rbm.model.classes.remove(&class);
                assert!(stated.is_some(), "the table stated the class");
                rbm.model.class_order.retain(|stated| *stated != class);
            }
            ContainerLayout::Pso(pso) => {
                let is_class = |item: &&mut PsoSchemaItem| matches!(item, PsoSchemaItem::Struct(info) if info.name_hash == class);
                let stated = pso.schema.items.iter_mut().find(is_class);
                // An enum in its place, so the table holds no struct under the name.
                *stated.expect("the table stated the class") = PsoSchemaItem::Enum(PsoEnumInfo {
                    name_hash: name_hash("AnEnumNoFileStates"),
                    flags: 0,
                    entries: Vec::new(),
                });
            }
            ContainerLayout::Rbf(_) => panic!("a type file is no RBF"),
        }
    }

    #[test]
    fn a_room_put_into_a_file_whose_table_only_names_its_class_is_refused_by_name() {
        let library = library();
        let mut roomless = interior_tree(&library);
        let Some(Value::Array(archetypes)) = roomless.get_mut("archetypes") else {
            panic!("the type file holds its archetypes");
        };
        let Some(Value::Pointer(Some(mlo))) = archetypes.first_mut() else {
            panic!("the type file holds its interior");
        };
        // No portal either: a portal names its rooms.
        let emptied = [name_hash("rooms"), name_hash("portals")];
        for (member, value) in &mut mlo.fields {
            if emptied.contains(&name_hash(member)) {
                *value = Value::Array(Vec::new());
            }
        }
        for format in FORMATS {
            let donor = CMapTypes::from_bytes(&interior_file(format)).expect("opens typed");
            let bytes = encoded(&library, &roomless, format);
            let mut opened = CMapTypes::from_bytes(&bytes).expect("opens typed");
            naming_only(&mut opened, "CMloRoomDef");

            let named = CMloRoomDef {
                name: "cellar".to_string(),
                ..Default::default()
            };
            for (room, how) in [
                (interior(&donor).rooms[0].clone(), "copied here"),
                // ⚠ Every member at its init: a class of no member would hold it, and say nothing.
                (CMloRoomDef::default(), "built"),
                (named, "built"),
            ] {
                let mut types = opened.clone();
                interior_mut(&mut types).rooms.push(room);
                let refused = types
                    .to_bytes()
                    .expect_err("nothing lays a room out in this file");
                assert!(matches!(refused, Error::Save { .. }), "{refused}");
                let why = refused.to_string();
                assert!(why.contains(&format!("`CMloRoomDef` {how}")), "{why}");
                assert_eq!(
                    why.contains("signature"),
                    format == BinaryFormat::Rbm,
                    "{format:?}: {why}"
                );
                assert!(
                    !why.contains("would drop") && !why.contains("no such member"),
                    "the class is not one of no member: {why}"
                );
            }
        }
    }

    #[test]
    fn a_maps_own_time_cycle_modifiers_still_count_in_what_two_maps_are_compared_by() {
        let mut inst = MetaInstance::new("CMapData");
        inst.set("timeCycleModifiers", Value::Array(vec![Value::UInt(1)]));
        let one = CMapData::read(&inst).expect("reads");
        inst.set("timeCycleModifiers", Value::Array(vec![Value::UInt(2)]));
        let other = CMapData::read(&inst).expect("reads");
        assert_ne!(
            one, other,
            "a member a map carries is no member the interior types"
        );
        assert!(is_typed(
            "CMloArchetypeDef",
            name_hash("timeCycleModifiers")
        ));
        assert!(is_typed("CMloRoomDef", name_hash("blend")));
        assert!(!is_typed("CMapData", name_hash("blend")));
    }
}
