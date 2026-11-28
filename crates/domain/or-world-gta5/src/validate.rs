//! What a world file has to hold for the engine to read it the way we did.
//!
//! ⚠ Every check here is a number that subscripts an array the file states
//! separately — a place the reader tolerates what the engine does not, so an
//! edit can produce a file OpenRage reads back identically and the game
//! misreads.
//!
//! Entities form a tree by index: a child names its parent's position in the
//! same `entities` array, and a parent states how many children it has. Both
//! halves are stored, neither is derived on load, and the engine trusts them.

use crate::map_data::MapData;

/// Something about a map that would misbehave in the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A parent index that is not a position in this map's entity array.
    ParentOutOfRange {
        entity: usize,
        parent: usize,
        len: usize,
    },
    /// An entity naming itself as its own LOD parent.
    ParentIsSelf { entity: usize },
    /// A cycle in the parent chain.
    ParentCycle { entity: usize },
    /// `numChildren` disagreeing with how many entities name this one.
    ChildCountWrong {
        entity: usize,
        declared: u32,
        actual: u32,
    },
    /// A room's or a portal's attached-object index that is not a position in
    /// the interior's entity array.
    AttachedObjectOutOfRange {
        holder: Holder,
        index: u32,
        len: usize,
    },
    /// An entity set whose two parallel arrays are different lengths.
    EntitySetArraysDiffer {
        set: usize,
        locations: usize,
        entities: usize,
    },
    /// A portal naming a room that does not exist.
    PortalRoomOutOfRange {
        portal: usize,
        room: u32,
        len: usize,
    },
    /// A portal whose quad is not four corners.
    PortalNotAQuad { portal: usize, corners: usize },
    /// A light array of a different length from its siblings.
    LightArraysDiffer {
        field: &'static str,
        len: usize,
        expected: usize,
    },
    /// An occlude model whose `dataSize` is not the count of the bytes it holds.
    OccludeModelSizeDiffers {
        model: usize,
        stated: u32,
        held: usize,
    },
}

impl Problem {
    /// Whether it is of a record a map holds beside its entities, a far light or an occlude model, and not of the LOD hierarchy.
    pub fn is_of_a_record(&self) -> bool {
        matches!(
            self,
            Problem::LightArraysDiffer { .. } | Problem::OccludeModelSizeDiffers { .. }
        )
    }
}

/// What attaches an object of an interior, by its position in its own array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    Room(usize),
    Portal(usize),
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Holder::Room(room) => write!(f, "room {room}"),
            Holder::Portal(portal) => write!(f, "portal {portal}"),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::ParentOutOfRange {
                entity,
                parent,
                len,
            } => write!(
                f,
                "entity {entity} names LOD parent {parent}, past the {len} entities in this map"
            ),
            Problem::ParentIsSelf { entity } => {
                write!(f, "entity {entity} is its own LOD parent")
            }
            Problem::ParentCycle { entity } => {
                write!(f, "entity {entity} is in a LOD parent cycle")
            }
            Problem::ChildCountWrong {
                entity,
                declared,
                actual,
            } => write!(
                f,
                "entity {entity} declares {declared} children and {actual} name it"
            ),
            Problem::AttachedObjectOutOfRange { holder, index, len } => write!(
                f,
                "{holder} attaches object {index}, past the {len} entities in this interior"
            ),
            Problem::EntitySetArraysDiffer {
                set,
                locations,
                entities,
            } => write!(
                f,
                "entity set {set} has {locations} locations for {entities} entities"
            ),
            Problem::PortalRoomOutOfRange { portal, room, len } => write!(
                f,
                "portal {portal} names room {room}, past the {len} rooms in this interior"
            ),
            Problem::PortalNotAQuad { portal, corners } => {
                write!(f, "portal {portal} has {corners} corners rather than four")
            }
            Problem::LightArraysDiffer {
                field,
                len,
                expected,
            } => write!(
                f,
                "light array {field} holds {len} where its siblings hold {expected}"
            ),
            Problem::OccludeModelSizeDiffers {
                model,
                stated,
                held,
            } => write!(
                f,
                "occlude model {model} states {stated} bytes of `verts` (dataSize) and holds {held}"
            ),
        }
    }
}

/// Check a map's LOD hierarchy, its light arrays and its occlude models.
pub fn check(map: &MapData<'_>) -> Vec<Problem> {
    let mut problems = check_hierarchy(map);
    problems.extend(check_light_arrays(map));
    problems.extend(check_occlude_models(map));
    problems
}

/// ⚠ `dataSize` counts the bytes behind `verts`, and the engine reads that many: a save of the typed map refuses a model
/// whose two differ.
fn check_occlude_models(map: &MapData<'_>) -> Vec<Problem> {
    use crate::field;
    use or_io::hash::literal_string_hash;
    use or_io::meta::Value;

    let models = field::structs_at(map.instance(), crate::keys::OCCLUDE_MODELS);
    (models.enumerate())
        .filter_map(|(model, record)| {
            let stated = field::u32_at(record, literal_string_hash(b"dataSize")).unwrap_or(0);
            let held = match field::get(record, literal_string_hash(b"verts")) {
                Some(Value::Array(items)) => items.len(),
                Some(Value::ByteArray(bytes)) => bytes.len(),
                _ => 0,
            };
            (stated as usize != held).then_some(Problem::OccludeModelSizeDiffers {
                model,
                stated,
                held,
            })
        })
        .collect()
}

/// ⚠ **Only the links this file can resolve.** An entity whose `flags` bit 3
/// is set indexes the *parent* map's array, which one file cannot see, and
/// `numChildren` counts children in child maps as well as this one. Both are
/// [`crate::group`]'s to check, and checking them here reports every shipped
/// map as broken.
fn check_hierarchy(map: &MapData<'_>) -> Vec<Problem> {
    let entities: Vec<_> = map.entities().collect();
    let len = entities.len();
    let mut problems = Vec::new();

    for (i, entity) in entities.iter().enumerate() {
        let Some(parent) = entity.local_parent_index() else {
            continue;
        };
        if parent >= len {
            problems.push(Problem::ParentOutOfRange {
                entity: i,
                parent,
                len,
            });
        } else if parent == i {
            problems.push(Problem::ParentIsSelf { entity: i });
        }
    }

    // A chain longer than the array has to revisit a node.
    for (i, _) in entities.iter().enumerate() {
        let mut at = i;
        for _ in 0..len {
            match entities[at].local_parent_index() {
                Some(p) if p < len && p != at => at = p,
                _ => break,
            }
        }
        if entities[at]
            .local_parent_index()
            .is_some_and(|p| p < len && p != at)
        {
            problems.push(Problem::ParentCycle { entity: i });
            break;
        }
    }

    problems
}

/// ⚠ **The LOD-light structs are eight parallel arrays under one subscript**,
/// and `DistantLODLightsSOA` is two more beside a separately-stated count.
/// The engine walks them together, so one short array is a read past its end
/// at whatever index the longest one reaches.
fn check_light_arrays(map: &MapData<'_>) -> Vec<Problem> {
    use crate::field;
    use or_io::hash::literal_string_hash;

    const LOD_LIGHT_FIELDS: &[&str] = &[
        "direction",
        "falloff",
        "falloffExponent",
        "timeAndStateFlags",
        "hash",
        "coneInnerAngle",
        "coneOuterAngleOrCapExt",
        "coronaIntensity",
    ];
    const DISTANT_FIELDS: &[&str] = &["position", "RGBI"];

    let mut problems = Vec::new();
    for (key, fields) in [
        (crate::keys::LOD_LIGHTS_SOA, LOD_LIGHT_FIELDS),
        (crate::keys::DISTANT_LOD_LIGHTS_SOA, DISTANT_FIELDS),
    ] {
        let Some(soa) = field::struct_at(map.instance(), key) else {
            continue;
        };
        let lengths: Vec<usize> = fields
            .iter()
            .map(|name| field::array_at(soa, literal_string_hash(name.as_bytes())).len())
            .collect();
        let Some(&expected) = lengths.iter().max() else {
            continue;
        };
        if expected == 0 {
            continue;
        }
        for (name, len) in fields.iter().zip(&lengths) {
            if *len != expected {
                problems.push(Problem::LightArraysDiffer {
                    field: name,
                    len: *len,
                    expected,
                });
            }
        }
    }
    problems
}

/// Check an interior's rooms, portals and entity sets against each other.
pub fn check_interior(interior: &crate::map_types::Interior<'_>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let entity_count = interior.entity_count();
    let room_count = interior.room_count() as u32;

    for (i, room) in interior.rooms().enumerate() {
        for index in room.attached_objects() {
            if index as usize >= entity_count {
                problems.push(Problem::AttachedObjectOutOfRange {
                    holder: Holder::Room(i),
                    index,
                    len: entity_count,
                });
            }
        }
    }

    for (i, portal) in interior.portals().enumerate() {
        for room in [portal.room_from(), portal.room_to()] {
            if room >= room_count {
                problems.push(Problem::PortalRoomOutOfRange {
                    portal: i,
                    room,
                    len: room_count as usize,
                });
            }
        }
        let corners = portal.corners().len();
        if corners != 4 {
            problems.push(Problem::PortalNotAQuad { portal: i, corners });
        }
        for index in portal.attached_objects() {
            if index as usize >= entity_count {
                problems.push(Problem::AttachedObjectOutOfRange {
                    holder: Holder::Portal(i),
                    index,
                    len: entity_count,
                });
            }
        }
    }

    for (i, set) in interior.entity_sets().enumerate() {
        let locations = set.location_count();
        let entities = set.entity_count();
        if locations != entities {
            problems.push(Problem::EntitySetArraysDiffer {
                set: i,
                locations,
                entities,
            });
        }
    }

    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::{MetaInstance, Value};

    pub(crate) fn map_with_flags(parents: &[i32], children: &[u32], flags: &[u32]) -> MetaInstance {
        let items = (0..parents.len())
            .map(|i| {
                let mut ent = MetaInstance::new("CEntityDef");
                ent.set("parentIndex", Value::Int(parents[i] as i64));
                ent.set("numChildren", Value::UInt(children[i] as u64));
                ent.set("flags", Value::UInt(flags[i] as u64));
                Value::Pointer(Some(Box::new(ent)))
            })
            .collect();
        let mut root = MetaInstance::new("CMapData");
        root.set("entities", Value::Array(items));
        root
    }

    fn map(parents: &[i32], children: &[u32]) -> MetaInstance {
        map_with_flags(parents, children, &vec![0; parents.len()])
    }

    #[test]
    fn a_consistent_hierarchy_has_nothing_to_report() {
        let root = map(&[-1, 0, 0], &[2, 0, 0]);
        assert_eq!(check(&MapData::new(&root).unwrap()), vec![]);
    }

    /// ⚠ The index is into the same array, so one past the end is a read of
    /// whatever follows rather than a missing link.
    #[test]
    fn a_parent_index_past_the_end_is_caught() {
        let root = map(&[-1, 5], &[0, 0]);
        let problems = check(&MapData::new(&root).unwrap());
        assert!(problems.contains(&Problem::ParentOutOfRange {
            entity: 1,
            parent: 5,
            len: 2
        }));
    }

    #[test]
    fn an_index_into_the_parent_map_is_not_this_files_to_judge() {
        let root = map_with_flags(&[-1, 5], &[0, 0], &[0, 1 << 3]);
        assert_eq!(check(&MapData::new(&root).unwrap()), vec![]);
    }

    #[test]
    fn an_entity_cannot_be_its_own_parent() {
        let root = map(&[0], &[0]);
        let problems = check(&MapData::new(&root).unwrap());
        assert!(problems.contains(&Problem::ParentIsSelf { entity: 0 }));
    }

    #[test]
    fn a_cycle_is_found_rather_than_walked_forever() {
        let root = map(&[1, 0], &[1, 1]);
        let problems = check(&MapData::new(&root).unwrap());
        assert!(problems
            .iter()
            .any(|p| matches!(p, Problem::ParentCycle { .. })));
    }

    #[test]
    fn a_child_count_is_not_checked_from_one_file() {
        let root = map(&[-1, 0], &[7, 0]);
        let problems = check(&MapData::new(&root).unwrap());
        assert!(!problems
            .iter()
            .any(|p| matches!(p, Problem::ChildCountWrong { .. })));
    }

    fn slot<'a>(inst: &'a mut MetaInstance, name: &str, at: usize) -> &'a mut MetaInstance {
        let Some(Value::Array(items)) = inst.get_mut(name) else {
            panic!("the interior holds `{name}`");
        };
        match &mut items[at] {
            Value::Struct(inner) | Value::Pointer(Some(inner)) => inner,
            other => panic!("`{name}` holds records, and one is {other:?}"),
        }
    }

    fn truncate(inst: &mut MetaInstance, name: &str, len: usize) {
        let Some(Value::Array(items)) = inst.get_mut(name) else {
            panic!("the record holds `{name}`");
        };
        items.truncate(len);
    }

    /// ⚠ A save of the typed root in `or-io` refuses what this check reports, and the two crates
    /// each spell the sentence: this holds them to one.
    #[test]
    fn a_broken_interior_is_refused_at_the_save_with_the_sentence_the_check_gives() {
        use crate::map_types::tests::{interior_file, tree};
        use or_io::decl::File;

        type Break = fn(&mut MetaInstance);
        let breaks: [(Break, &str); 6] = [
            (
                |i| slot(i, "rooms", 1).set("attachedObjects", Value::Array(vec![Value::UInt(7)])),
                "room 1 attaches object 7, past the 3 entities in this interior",
            ),
            (
                |i| truncate(i, "entities", 2),
                "portal 0 attaches object 2, past the 2 entities in this interior",
            ),
            (
                |i| slot(i, "portals", 0).set("roomTo", Value::UInt(2)),
                "portal 0 names room 2, past the 2 rooms in this interior",
            ),
            (
                |i| slot(i, "portals", 0).set("roomFrom", Value::UInt(3)),
                "portal 0 names room 3, past the 2 rooms in this interior",
            ),
            (
                |i| truncate(slot(i, "entitySets", 0), "entities", 1),
                "entity set 0 has 2 locations for 1 entities",
            ),
            (
                |i| truncate(slot(i, "entitySets", 0), "locations", 0),
                "entity set 0 has 0 locations for 2 entities",
            ),
        ];
        for (broken, sentence) in breaks {
            let bytes = interior_file(broken);
            let root = tree(&bytes);
            let types = crate::MapTypes::new(&root).expect("a type file");
            let interior = types.archetypes().next().and_then(|a| a.interior());
            let problems = check_interior(&interior.expect("the archetype is an interior"));
            let said: Vec<String> = problems.iter().map(ToString::to_string).collect();
            assert_eq!(said, [sentence]);

            let typed = or_io::meta::CMapTypes::from_bytes(&bytes).expect("opens typed");
            let refused = typed
                .to_bytes()
                .expect_err("a broken interior is not saved");
            assert!(matches!(refused, or_io::Error::Refused { .. }), "{refused}");
            let why = refused.to_string();
            assert!(why.ends_with(&format!("archetype 0: {sentence}")), "{why}");
        }

        let bytes = interior_file(|_| {});
        let root = tree(&bytes);
        let types = crate::MapTypes::new(&root).expect("a type file");
        let interior = types.archetypes().next().and_then(|a| a.interior());
        assert_eq!(check_interior(&interior.expect("an interior")), vec![]);
        let typed = or_io::meta::CMapTypes::from_bytes(&bytes).expect("opens typed");
        typed.to_bytes().expect("an interior that holds saves");
    }

    /// ⚠ As for an interior: a save of the typed map in `or-io` refuses what [`check`] reports of the far lights, and
    /// this holds the two crates' sentences to one.
    #[test]
    fn far_lights_whose_arrays_differ_are_refused_at_the_save_with_the_sentence_the_check_gives() {
        use crate::map_data::tests::{map_file, tree};
        use or_io::decl::File;

        let breaks: [(&str, &str, usize, &str); 4] = [
            (
                "LODLightsSOA",
                "hash",
                2,
                "light array hash holds 2 where its siblings hold 3",
            ),
            (
                "LODLightsSOA",
                "direction",
                0,
                "light array direction holds 0 where its siblings hold 3",
            ),
            (
                "LODLightsSOA",
                "coronaIntensity",
                1,
                "light array coronaIntensity holds 1 where its siblings hold 3",
            ),
            (
                "DistantLODLightsSOA",
                "RGBI",
                2,
                "light array RGBI holds 2 where its siblings hold 3",
            ),
        ];
        for (family, array, len, sentence) in breaks {
            let bytes = map_file(|root| {
                let Some(Value::Struct(family)) = root.get_mut(family) else {
                    panic!("the map holds `{family}`");
                };
                truncate(family, array, len);
            });
            let root = tree(&bytes);
            let problems = check(&MapData::new(&root).expect("a map"));
            let said: Vec<String> = problems.iter().map(ToString::to_string).collect();
            assert_eq!(said, [sentence]);

            let typed = or_io::meta::CMapData::from_bytes(&bytes).expect("opens typed");
            let refused = typed
                .to_bytes()
                .expect_err("far lights whose arrays differ are not saved");
            assert!(matches!(refused, or_io::Error::Refused { .. }), "{refused}");
            assert!(refused.to_string().ends_with(sentence), "{refused}");
        }

        let bytes = map_file(|_| {});
        let root = tree(&bytes);
        assert_eq!(check(&MapData::new(&root).expect("a map")), vec![]);
        let typed = or_io::meta::CMapData::from_bytes(&bytes).expect("opens typed");
        typed.to_bytes().expect("far lights that hold save");
    }

    /// ⚠ As for the far lights: a save of the typed map refuses an occlude model whose `dataSize` misses its bytes, and
    /// this holds the two crates' sentences to one.
    #[test]
    fn an_occlude_model_whose_size_misses_its_bytes_is_refused_at_the_save_with_the_sentence_the_check_gives(
    ) {
        use crate::map_data::tests::{map_file, tree};
        use or_io::decl::File;

        let with_a_model = |stated: i64, held: usize| {
            map_file(|root| {
                let mut model = MetaInstance::new("Item");
                model.set("dataSize", Value::Int(stated));
                model.set("verts", Value::ByteArray(vec![7; held]));
                model.set("numTris", Value::Int(1));
                let models = vec![Value::Struct(Box::new(model))];
                root.set("occludeModels", Value::Array(models));
            })
        };
        for (stated, held, sentence) in [
            (
                11,
                12,
                "occlude model 0 states 11 bytes of `verts` (dataSize) and holds 12",
            ),
            (
                12,
                0,
                "occlude model 0 states 12 bytes of `verts` (dataSize) and holds 0",
            ),
        ] {
            let bytes = with_a_model(stated, held);
            let root = tree(&bytes);
            let problems = check(&MapData::new(&root).expect("a map"));
            let said: Vec<String> = problems.iter().map(ToString::to_string).collect();
            assert_eq!(said, [sentence]);
            assert!(problems[0].is_of_a_record());

            let typed = or_io::meta::CMapData::from_bytes(&bytes).expect("opens typed");
            let refused = typed
                .to_bytes()
                .expect_err("an occlude model whose size misses its bytes is not saved");
            assert!(matches!(refused, or_io::Error::Refused { .. }), "{refused}");
            assert!(refused.to_string().ends_with(sentence), "{refused}");
        }

        let bytes = with_a_model(12, 12);
        let root = tree(&bytes);
        assert_eq!(check(&MapData::new(&root).expect("a map")), vec![]);
        let typed = or_io::meta::CMapData::from_bytes(&bytes).expect("opens typed");
        typed.to_bytes().expect("a model that holds saves");
    }

    #[test]
    fn a_problem_says_what_it_is_in_a_sentence() {
        let text = Problem::ParentOutOfRange {
            entity: 1,
            parent: 5,
            len: 2,
        }
        .to_string();
        assert!(text.contains("entity 1"), "{text}");
        assert!(text.contains('5'), "{text}");
    }
}
