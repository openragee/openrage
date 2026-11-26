//! The doors of maps and type files built by hand: which entities are found and by what, what a
//! leaf's box says of it, which leaves pair, and how far a leaf opens before something is in its way.

use std::sync::Arc;

use glam::{Affine3A, Quat, Vec2, Vec3};
use or_io::meta::instance::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_world::collision::{BoundCollider, CollisionSet};
use or_world::doors::{
    clearance, find, pair, read_leaf, ArcError, Blocked, Blocker, DoorArc, EntityAt, Found,
    FoundBy, HingeSide, Leaf, Motion, NamePatterns, Reason, Unread, Way, WidthAxis, ARC_SILL,
    ARC_SKIN, HINGE_EDGE_SHARE, LIFT_MIN_WIDTH, PAIR_GAP_SLACK, PAIR_PLANE_TOLERANCE,
};
use or_world::{ExtensionKind, MapData, MapTypes, Placement};

#[path = "support/bounds.rs"]
mod bounds;

const QUARTER: f32 = std::f32::consts::FRAC_PI_2;

fn pointers(items: Vec<MetaInstance>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|inst| Value::Pointer(Some(Box::new(inst))))
            .collect(),
    )
}

/// An archetype that states its name as a hash alone, as a file read with no name index does.
fn archetype(name: &str, min: [f32; 3], max: [f32; 3], door: bool) -> MetaInstance {
    let mut inst = MetaInstance::new("CBaseArchetypeDef");
    let hash = or_io::string_hash(name.as_bytes());
    inst.set("name", Value::Hash(HashedString::from_hash(hash)));
    inst.set("bbMin", Value::Vec3(min));
    inst.set("bbMax", Value::Vec3(max));
    if door {
        let extension = ExtensionKind::Door.template().expect("a named kind");
        inst.set("extensions", pointers(vec![extension]));
    }
    inst
}

fn type_file(archetypes: Vec<MetaInstance>) -> MetaInstance {
    let mut root = MetaInstance::new("CMapTypes");
    root.set("archetypes", pointers(archetypes));
    root
}

fn entity_of(type_name: &str, name: &str, position: [f32; 3], turn: Quat) -> MetaInstance {
    let placement = if type_name == "CMloInstanceDef" {
        Placement::InteriorInstance
    } else {
        Placement::Entity
    };
    let mut inst = MetaInstance::new(type_name);
    let hash = or_io::string_hash(name.as_bytes());
    inst.set("archetypeName", Value::Hash(HashedString::from_hash(hash)));
    inst.set("position", Value::Vec3(position));
    inst.set("rotation", Value::Vec4(placement.store(turn)));
    inst.set("scaleXY", Value::Float(1.0));
    inst.set("scaleZ", Value::Float(1.0));
    inst
}

fn entity(name: &str, position: [f32; 3], yaw: f32) -> MetaInstance {
    entity_of("CEntityDef", name, position, Quat::from_rotation_z(yaw))
}

fn map_file(entities: Vec<MetaInstance>) -> MetaInstance {
    let mut root = MetaInstance::new("CMapData");
    root.set("entities", pointers(entities));
    root
}

fn patterns() -> NamePatterns {
    NamePatterns {
        doors: vec!["*door*".into()],
        not_doors: vec!["*frame*".into()],
    }
}

/// A lookup that knows the names in `known` and no other.
fn names(known: &'static [&'static str]) -> impl Fn(u32) -> Option<String> {
    move |hash| {
        (known.iter())
            .find(|name| or_io::string_hash(name.as_bytes()) == hash)
            .map(|name| name.to_string())
    }
}

fn found_in(map: &MetaInstance, types: &MetaInstance, known: &'static [&'static str]) -> Found {
    let map = MapData::new(map).expect("a map file");
    let types = MapTypes::new(types).expect("a type file");
    find(&[map], &[types], &names(known), &patterns())
}

fn at(entity: usize) -> EntityAt {
    EntityAt {
        map: 0,
        entity,
        inside: None,
    }
}

/// A leaf a metre wide, four centimetres thick and two metres high, whose origin is at the foot of one end.
const LEAF_MIN: [f32; 3] = [0.0, -0.02, 0.0];
const LEAF_MAX: [f32; 3] = [1.0, 0.02, 2.0];

#[test]
fn a_door_is_found_by_its_extension_or_by_its_name_and_a_model_with_no_name_is_counted() {
    let types = type_file(vec![
        archetype("gate_post_a", LEAF_MIN, LEAF_MAX, true),
        archetype("shop_door_l", LEAF_MIN, LEAF_MAX, false),
        archetype("crate_a", [-1.0; 3], [1.0; 3], false),
        archetype("shop_doorframe", LEAF_MIN, LEAF_MAX, false),
    ]);
    let map = map_file(vec![
        entity("gate_post_a", [0.0, 0.0, 0.0], 0.0),
        entity("shop_door_l", [10.0, 0.0, 0.0], 0.0),
        entity("crate_a", [20.0, 0.0, 0.0], 0.0),
        entity("a_model_nothing_names", [30.0, 0.0, 0.0], 0.0),
        entity("shop_doorframe", [40.0, 0.0, 0.0], 0.0),
    ]);
    // The lookup knows every name but two: the model the extension finds, and the one nothing finds.
    let found = found_in(&map, &types, &["shop_door_l", "crate_a", "shop_doorframe"]);

    let doors: Vec<(EntityAt, &FoundBy)> = (found.doors.iter())
        .map(|door| (door.entity.at, &door.found_by))
        .collect();
    assert_eq!(
        doors,
        vec![
            (at(0), &FoundBy::Extension),
            (at(1), &FoundBy::Name("*door*".into())),
        ]
    );
    assert_eq!(found.doors[0].entity.name, None);
    assert_eq!(found.doors[1].entity.name.as_deref(), Some("shop_door_l"));
    assert_eq!(found.entities, 5);
    assert_eq!(
        found.unnamed, 1,
        "the model with no name, and not the door found without one"
    );
    let others: Vec<EntityAt> = found.others.iter().map(|e| e.at).collect();
    assert_eq!(others, vec![at(2), at(3), at(4)]);
}

#[test]
fn a_lookup_that_is_wrong_about_a_hash_names_nothing() {
    let types = type_file(vec![archetype("crate_a", [-1.0; 3], [1.0; 3], false)]);
    let map = map_file(vec![entity("crate_a", [0.0; 3], 0.0)]);
    let map = MapData::new(&map).expect("a map file");
    let types = MapTypes::new(&types).expect("a type file");
    let lying = |_: u32| Some("shop_door_l".to_string());
    let found = find(&[map], &[types], &lying, &patterns());
    assert!(found.doors.is_empty());
    assert_eq!(found.unnamed, 1);
}

/// ⚠ As the installs state a door: no archetype of either states the door extension, and the
/// entities of the maps and of the interiors do.
#[test]
fn a_door_whose_entity_states_the_door_extension_is_found_by_it_whatever_its_archetype_states() {
    let door_extension = || pointers(vec![ExtensionKind::Door.template().expect("a named kind")]);
    let mut stated = entity("hatch_a", [0.0, 0.0, 0.0], 0.0);
    stated.set("extensions", door_extension());
    let mut inside = entity("hatch_a", [1.0, 0.0, 0.0], 0.0);
    inside.set("extensions", door_extension());
    let mut interior = archetype("yard_interior", [-5.0; 3], [5.0; 3], false);
    interior.type_name = "CMloArchetypeDef".into();
    interior.set(
        "entities",
        pointers(vec![inside, entity("hatch_a", [2.0, 0.0, 0.0], 0.0)]),
    );
    let types = type_file(vec![
        interior,
        archetype("hatch_a", LEAF_MIN, LEAF_MAX, false),
    ]);
    let map = map_file(vec![
        stated,
        entity("hatch_a", [10.0, 0.0, 0.0], 0.0),
        entity_of(
            "CMloInstanceDef",
            "yard_interior",
            [100.0, 0.0, 0.0],
            Quat::IDENTITY,
        ),
    ]);
    // Nothing names the model, so nothing but the extension can find it.
    let found = found_in(&map, &types, &[]);

    let doors: Vec<(EntityAt, &FoundBy, Vec3)> = (found.doors.iter())
        .map(|door| (door.entity.at, &door.found_by, door.entity.position))
        .collect();
    let within = EntityAt {
        map: 0,
        entity: 2,
        inside: Some(0),
    };
    assert_eq!(
        doors,
        vec![
            (at(0), &FoundBy::Extension, Vec3::ZERO),
            (within, &FoundBy::Extension, Vec3::new(101.0, 0.0, 0.0)),
        ]
    );
    // The same model placed with no extension is no door, and is counted as one nothing could judge.
    assert_eq!((found.entities, found.unnamed), (5, 2));
}

#[test]
fn a_door_an_interior_places_is_found_where_the_instance_puts_it() {
    // Its own name would make it a door, were a placed interior ever one.
    let mut interior = archetype("back_door_interior", [-5.0; 3], [5.0; 3], false);
    interior.type_name = "CMloArchetypeDef".into();
    interior.set(
        "entities",
        pointers(vec![
            entity("crate_a", [0.0, 0.0, 0.0], 0.0),
            // It leans, so the order the instance's turn and its own are joined in shows.
            entity_of(
                "CEntityDef",
                "shop_door_l",
                [1.0, 0.0, 0.5],
                Quat::from_rotation_x(0.5),
            ),
        ]),
    );
    let types = type_file(vec![
        interior,
        archetype("shop_door_l", LEAF_MIN, LEAF_MAX, false),
        archetype("crate_a", [-1.0; 3], [1.0; 3], false),
    ]);
    let map = map_file(vec![entity_of(
        "CMloInstanceDef",
        "back_door_interior",
        [10.0, 20.0, 30.0],
        Quat::from_rotation_z(QUARTER),
    )]);
    let known = &["back_door_interior", "shop_door_l", "crate_a"];
    let found = found_in(&map, &types, known);

    assert_eq!(found.entities, 3);
    let [door] = found.doors.as_slice() else {
        panic!("one door, not {:?}", found.doors);
    };
    assert_eq!(
        door.entity.at,
        EntityAt {
            map: 0,
            entity: 0,
            inside: Some(1)
        }
    );
    // A quarter turn of the instance carries the interior's X onto the world's Y.
    assert!(door
        .entity
        .position
        .abs_diff_eq(Vec3::new(10.0, 21.0, 30.5), 1e-5));
    let leaf = read_leaf(&door.entity).expect("a leaf");
    let hinge = leaf.hinge.expect("a hinge");
    assert!(hinge.toward_free_edge.abs_diff_eq(Vec3::Y, 1e-5));
    // The door leans in the interior first, and the instance turns what leans.
    let leant = Quat::from_rotation_z(QUARTER) * Quat::from_rotation_x(0.5) * Vec3::Z;
    assert!(hinge.axis.abs_diff_eq(leant, 1e-5), "{:?}", hinge.axis);
    // The interior itself is a building and never a leaf.
    assert!(found.others.iter().any(|e| e.interior_instance));
}

#[test]
fn of_two_type_files_that_declare_one_name_the_later_is_believed() {
    let first = type_file(vec![
        archetype("yard_barrier_a", [-1.0; 3], [1.0; 3], false),
        archetype("shed_hatch_a", LEAF_MIN, LEAF_MAX, true),
    ]);
    let later = type_file(vec![
        archetype("yard_barrier_a", LEAF_MIN, LEAF_MAX, true),
        archetype("shed_hatch_a", [-1.0; 3], [1.0; 3], false),
    ]);
    let map = map_file(vec![
        entity("yard_barrier_a", [0.0; 3], 0.0),
        entity("shed_hatch_a", [10.0, 0.0, 0.0], 0.0),
    ]);
    let map = MapData::new(&map).expect("a map file");
    let files = [&first, &later].map(|file| MapTypes::new(file).expect("a type file"));
    let found = find(&[map], &files, &names(&[]), &patterns());

    // The later file gives the first model the door extension and its box, and takes them from the second.
    let doors: Vec<EntityAt> = found.doors.iter().map(|door| door.entity.at).collect();
    assert_eq!(doors, vec![at(0)]);
    let bounds = found.doors[0].entity.bounds.expect("a box");
    assert_eq!((bounds.min, bounds.max), (LEAF_MIN.into(), LEAF_MAX.into()));
    assert_eq!(found.others[0].bounds.map(|b| b.max), Some(Vec3::ONE));
    assert_eq!(found.unnamed, 1);
}

fn leaf_turned(
    min: [f32; 3],
    max: [f32; 3],
    position: [f32; 3],
    turn: Quat,
) -> Result<Leaf, Unread> {
    let types = type_file(vec![archetype("shop_door_l", min, max, false)]);
    let map = map_file(vec![entity_of("CEntityDef", "shop_door_l", position, turn)]);
    let found = found_in(&map, &types, &["shop_door_l"]);
    read_leaf(&found.doors[0].entity)
}

fn leaf_of(min: [f32; 3], max: [f32; 3], position: [f32; 3], yaw: f32) -> Result<Leaf, Unread> {
    leaf_turned(min, max, position, Quat::from_rotation_z(yaw))
}

#[test]
fn a_leafs_box_says_where_its_hinge_is_and_whether_it_swings_slides_or_lifts() {
    // The origin at the low end of X: a hinge there, and the leaf reaches along X.
    let swing = leaf_of(LEAF_MIN, LEAF_MAX, [3.0, 4.0, 5.0], 0.0).expect("a leaf");
    assert_eq!(swing.proposal.motion, Motion::Swing);
    assert_eq!(
        swing.proposal.reason,
        Reason::OriginAtAnEnd {
            from_end: 0.0,
            width: 1.0
        }
    );
    assert_eq!(swing.width_axis, WidthAxis::X);
    assert_eq!((swing.width, swing.height), (1.0, 2.0));
    assert!((swing.thickness - 0.04).abs() < 1e-6);
    let hinge = swing.hinge.expect("a hinge");
    assert_eq!(hinge.side, HingeSide::Low);
    assert_eq!(hinge.position, Vec3::new(3.0, 4.0, 5.0));
    assert!(hinge.axis.abs_diff_eq(Vec3::Z, 1e-6));
    assert!(hinge.toward_free_edge.abs_diff_eq(Vec3::X, 1e-6));
    assert_eq!(hinge.reach, 1.0);

    // The same box turned a quarter, with the origin at the high end of Y: the leaf reaches against Y, which the turn carries onto X.
    let high = leaf_of([-0.02, -1.0, 0.0], [0.02, 0.0, 2.0], [0.0; 3], QUARTER).expect("a leaf");
    let hinge = high.hinge.expect("a hinge");
    assert_eq!(
        (high.width_axis, hinge.side),
        (WidthAxis::Y, HingeSide::High)
    );
    assert!(hinge.toward_free_edge.abs_diff_eq(Vec3::X, 1e-6));
    assert_eq!(hinge.reach, 1.0);

    // The origin just inside the share of the width that makes an end a hinge, and just outside it.
    let width = 1.0;
    let inside = HINGE_EDGE_SHARE * width - 0.01;
    let near = leaf_of(
        [-inside, -0.02, 0.0],
        [width - inside, 0.02, 2.0],
        [0.0; 3],
        0.0,
    );
    assert_eq!(near.expect("a leaf").proposal.motion, Motion::Swing);
    let outside = HINGE_EDGE_SHARE * width + 0.01;
    let far = leaf_of(
        [-outside, -0.02, 0.0],
        [width - outside, 0.02, 2.0],
        [0.0; 3],
        0.0,
    )
    .expect("a leaf");
    assert_eq!(far.proposal.motion, Motion::Slide);
    assert_eq!(far.hinge, None);
    assert!(matches!(far.proposal.reason, Reason::NoEndNarrow { .. }));

    // The origin in the middle of a leaf as wide as the width that lifts.
    let half = LIFT_MIN_WIDTH / 2.0;
    let wide = leaf_of([-half, -0.05, 0.0], [half, 0.05, 2.2], [0.0; 3], 0.0).expect("a leaf");
    assert_eq!(wide.proposal.motion, Motion::Lift);
    assert_eq!(
        wide.proposal.reason,
        Reason::NoEndWide {
            from_nearer_end: half,
            width: LIFT_MIN_WIDTH
        }
    );
    let narrower = leaf_of([-half + 0.01, -0.05, 0.0], [half, 0.05, 2.2], [0.0; 3], 0.0);
    assert_eq!(narrower.expect("a leaf").proposal.motion, Motion::Slide);

    // An entity's scale is its leaf's: twice as wide and half as high again.
    let types = type_file(vec![archetype("shop_door_l", LEAF_MIN, LEAF_MAX, false)]);
    let mut scaled = entity("shop_door_l", [0.0; 3], 0.0);
    scaled.set("scaleXY", Value::Float(2.0));
    scaled.set("scaleZ", Value::Float(1.5));
    let found = found_in(&map_file(vec![scaled]), &types, &["shop_door_l"]);
    let leaf = read_leaf(&found.doors[0].entity).expect("a leaf");
    assert_eq!((leaf.width, leaf.height), (2.0, 3.0));
    assert_eq!(leaf.hinge.expect("a hinge").reach, 2.0);
    let shape = found.doors[0].entity.world_box().expect("a box");
    assert!(shape
        .half_extents
        .abs_diff_eq(Vec3::new(1.0, 0.04, 1.5), 1e-6));
    assert!(shape.centre.abs_diff_eq(Vec3::new(1.0, 0.0, 1.5), 1e-6));
}

#[test]
fn a_leaf_turns_about_its_own_upright_line_however_it_leans() {
    let leaning = Quat::from_rotation_z(0.4) * Quat::from_rotation_x(0.5);
    let leaf = leaf_turned(LEAF_MIN, LEAF_MAX, [3.0, 4.0, 5.0], leaning).expect("a leaf");
    let hinge = leaf.hinge.expect("a hinge");
    assert!(hinge.axis.abs_diff_eq(leaning * Vec3::Z, 1e-6));
    let middle = (Vec3::from(LEAF_MIN) + Vec3::from(LEAF_MAX)) * 0.5;
    for turn in [-QUARTER, -0.7, 0.0, 0.3, QUARTER] {
        let turned = leaf.box_at(turn);
        // A point of the hinge line stays where it is in the leaf, and the free edge stays its reach from the line.
        let on_the_line = hinge.position + hinge.axis * 1.5;
        let in_the_leaf = turned.rotation.inverse() * (on_the_line - turned.centre) + middle;
        assert!(
            in_the_leaf.abs_diff_eq(Vec3::new(0.0, 0.0, 1.5), 1e-5),
            "{turn}: {in_the_leaf:?}"
        );
        let free_edge = turned.centre + turned.rotation * (Vec3::X * 0.5);
        let from_the_line = (free_edge - hinge.position).reject_from(hinge.axis);
        assert!(
            (from_the_line.length() - hinge.reach).abs() < 1e-5,
            "{turn}"
        );
    }
    // A quarter turn anticlockwise carries the free edge onto the leaf's own Y.
    let open = leaf.box_at(QUARTER);
    let toward = (open.rotation * Vec3::X).normalize();
    assert!(toward.abs_diff_eq(leaning * Vec3::Y, 1e-5), "{toward:?}");
}

#[test]
fn a_door_with_no_box_or_a_box_that_is_none_is_not_read() {
    let types = type_file(vec![archetype("crate_a", [-1.0; 3], [1.0; 3], false)]);
    let map = map_file(vec![entity("shop_door_l", [0.0; 3], 0.0)]);
    let found = found_in(&map, &types, &["shop_door_l"]);
    assert_eq!(read_leaf(&found.doors[0].entity), Err(Unread::NoBox));

    assert_eq!(
        leaf_of([1.0, -0.02, 0.0], [0.0, 0.02, 2.0], [0.0; 3], 0.0),
        Err(Unread::Inverted)
    );
    assert_eq!(
        leaf_of([0.0; 3], [0.0, 0.0, 2.0], [0.0; 3], 0.0),
        Err(Unread::NoWidth)
    );
    assert_eq!(
        leaf_of(LEAF_MIN, [f32::NAN, 0.02, 2.0], [0.0; 3], 0.0),
        Err(Unread::NotFinite)
    );
    // Two finite corners further apart than a 32-bit number counts have no finite width.
    let most = f32::MAX;
    assert_eq!(
        leaf_of([-most, -0.02, 0.0], [most, 0.02, 2.0], [0.0; 3], 0.0),
        Err(Unread::NotFinite)
    );
    // And a finite box at a finite scale that carries it past them has none either.
    let types = type_file(vec![archetype(
        "shop_door_l",
        LEAF_MIN,
        [most / 2.0, 0.02, 2.0],
        false,
    )]);
    let mut scaled = entity("shop_door_l", [0.0; 3], 0.0);
    scaled.set("scaleXY", Value::Float(4.0));
    let found = found_in(&map_file(vec![scaled]), &types, &["shop_door_l"]);
    assert_eq!(read_leaf(&found.doors[0].entity), Err(Unread::NotFinite));
    assert_eq!(found.doors[0].entity.world_box(), None);
}

/// A leaf whose hinge is at `x` on the line y = `y`, reaching toward +X or toward -X.
fn leaf_at(x: f32, y: f32, toward_plus: bool) -> Option<Leaf> {
    let yaw = if toward_plus { 0.0 } else { 2.0 * QUARTER };
    Some(leaf_of(LEAF_MIN, LEAF_MAX, [x, y, 0.0], yaw).expect("a leaf"))
}

#[test]
fn two_leaves_that_face_each_other_pair_and_two_side_by_side_stay_single() {
    // Hinges two metres apart, each leaf a metre: they meet in the middle.
    let facing = [leaf_at(0.0, 0.0, true), leaf_at(2.0, 0.0, false)];
    let pairs = pair(&facing);
    assert_eq!(pairs.len(), 1);
    assert_eq!((pairs[0].a, pairs[0].b), (0, 1));
    assert!((pairs[0].opening - 2.0).abs() < 1e-5);

    // Side by side, both hinged on the same hand: neither reaches toward the other's hinge.
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, true), leaf_at(1.0, 0.0, true)]),
        vec![]
    );
    // The same two hung on the other hand: the first reaches away from the second.
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, false), leaf_at(1.0, 0.0, false)]),
        vec![]
    );
    // Back to back: the hinges share a post and the leaves reach away from it.
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, false), leaf_at(0.1, 0.0, true)]),
        vec![]
    );
    // Two hinges the width of a pair apart whose leaves reach away from each other close no opening between them.
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, false), leaf_at(2.0, 0.0, true)]),
        vec![]
    );
    // A leaf that leans has a hinge line the other's is not parallel to.
    let leaning = Quat::from_rotation_z(2.0 * QUARTER) * Quat::from_rotation_x(0.5);
    let leant = leaf_turned(LEAF_MIN, LEAF_MAX, [2.0, 0.0, 0.0], leaning).ok();
    assert!(leant.is_some_and(|leaf| leaf.hinge.is_some()));
    assert_eq!(pair(&[leaf_at(0.0, 0.0, true), leant]), vec![]);

    // A left and a right model at one heading: the right one's origin is at the high end of its box.
    let right = leaf_of([-1.0, -0.02, 0.0], [0.0, 0.02, 2.0], [2.0, 0.0, 0.0], 0.0).ok();
    assert_eq!(
        right.and_then(|leaf| leaf.hinge).map(|h| (h.side, h.reach)),
        Some((HingeSide::High, 1.0))
    );
    let pairs = pair(&[leaf_at(0.0, 0.0, true), right]);
    assert_eq!(pairs.len(), 1);
    assert!((pairs[0].opening - 2.0).abs() < 1e-5);

    // A leaf that stands ajar, a twelfth of a turn out of the wall: its hinge is in the other's plane and the other's hinge is
    // not in its own, whichever of the two is met first.
    let ajar = |x: f32, toward_plus: bool| {
        let yaw = if toward_plus { 0.0 } else { 2.0 * QUARTER } + QUARTER / 3.0;
        leaf_of(LEAF_MIN, LEAF_MAX, [x, 0.0, 0.0], yaw).ok()
    };
    assert_eq!(pair(&[leaf_at(0.0, 0.0, true), ajar(2.0, false)]), vec![]);
    assert_eq!(pair(&[ajar(0.0, true), leaf_at(2.0, 0.0, false)]), vec![]);
}

#[test]
fn a_pair_is_held_to_one_plane_and_to_an_opening_its_two_leaves_fill() {
    let inside = PAIR_PLANE_TOLERANCE - 0.01;
    let outside = PAIR_PLANE_TOLERANCE + 0.01;
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, true), leaf_at(2.0, inside, false)]).len(),
        1
    );
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, true), leaf_at(2.0, outside, false)]),
        vec![]
    );

    // And to one height.
    let facing_at = |z: f32| leaf_of(LEAF_MIN, LEAF_MAX, [2.0, 0.0, z], 2.0 * QUARTER).ok();
    assert_eq!(pair(&[leaf_at(0.0, 0.0, true), facing_at(inside)]).len(), 1);
    assert_eq!(pair(&[leaf_at(0.0, 0.0, true), facing_at(outside)]), vec![]);

    // The opening may be wider than the two leaves by the gap they leave, and no wider.
    let widest = 2.0 + PAIR_GAP_SLACK;
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, true), leaf_at(widest - 0.01, 0.0, false)]).len(),
        1
    );
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, true), leaf_at(widest + 0.01, 0.0, false)]),
        vec![]
    );
    // And no narrower than one leaf, whose free edge would pass the other's hinge.
    assert_eq!(
        pair(&[leaf_at(0.0, 0.0, true), leaf_at(0.99, 0.0, false)]),
        vec![]
    );

    // A leaf that does not swing, and a door whose leaf was not read, pair with nothing.
    let sliding = leaf_of([-0.5, -0.02, 0.0], [0.5, 0.02, 2.0], [2.0, 0.0, 0.0], 0.0).ok();
    assert_eq!(pair(&[leaf_at(0.0, 0.0, true), sliding]), vec![]);
    assert_eq!(pair(&[leaf_at(0.0, 0.0, true), None]), vec![]);
}

#[test]
fn a_leaf_is_paired_once() {
    // The first leaf faces two others; the one whose opening the two leaves fill is its pair.
    let leaves = [
        leaf_at(0.0, 0.0, true),
        leaf_at(2.1, 0.0, false),
        leaf_at(2.0, 0.0, false),
        leaf_at(-2.0, 0.0, true),
    ];
    let pairs = pair(&leaves);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!((pairs[0].a, pairs[0].b), (0, 2));
    let mut in_a_pair = [0; 4];
    for found in &pairs {
        in_a_pair[found.a] += 1;
        in_a_pair[found.b] += 1;
    }
    assert_eq!(in_a_pair, [1, 0, 1, 0]);

    // Two double doors in one wall are two pairs, each leaf in one.
    let wall = [
        leaf_at(0.0, 0.0, true),
        leaf_at(2.0, 0.0, false),
        leaf_at(2.1, 0.0, true),
        leaf_at(4.1, 0.0, false),
    ];
    let pairs: Vec<(usize, usize)> = pair(&wall).iter().map(|p| (p.a, p.b)).collect();
    assert_eq!(pairs, vec![(0, 1), (2, 3)]);
}

/// The turn at which the leading corner of the swept leaf, turning toward +Y about the origin, comes to the line y = `y`.
fn turn_to_reach(y: f32) -> f32 {
    let corner = Vec2::new(LEAF_MAX[0] - ARC_SKIN, LEAF_MAX[1] - ARC_SKIN);
    (y / corner.length()).asin() - corner.y.atan2(corner.x)
}

fn room(furniture: bool) -> Found {
    let types = type_file(vec![
        archetype("shop_door_l", LEAF_MIN, LEAF_MAX, false),
        archetype("sofa_a", [-0.5, -0.5, 0.0], [0.5, 0.5, 1.0], false),
        archetype("shop_shell", [-20.0; 3], [20.0; 3], false),
    ]);
    let mut entities = vec![
        entity("shop_door_l", [0.0; 3], 0.0),
        // The building the door is in, whose box holds the whole room.
        entity("shop_shell", [0.0; 3], 0.0),
    ];
    if furniture {
        // Its near face is the line y = 0.4, from x = 0.1 to x = 1.1.
        entities.push(entity("sofa_a", [0.6, 0.9, 0.0], 0.0));
    }
    found_in(
        &map_file(entities),
        &types,
        &["shop_door_l", "sofa_a", "shop_shell"],
    )
}

fn collider(bound: or_io::gta5::PhBoundAny) -> Arc<BoundCollider> {
    Arc::new(BoundCollider::new(&bound).expect("a bound built by hand is whole"))
}

#[test]
fn furniture_in_the_arc_blocks_a_door_at_the_angle_the_geometry_gives_and_an_empty_room_does_not() {
    let nothing: CollisionSet<&str> = CollisionSet::new();
    let furnished = room(true);
    let leaf = read_leaf(&furnished.doors[0].entity).expect("a leaf");
    let obstacles = furnished.obstacles_of(0);
    assert_eq!(obstacles.len(), 2, "the shell and the sofa");

    let toward = clearance(&leaf, Way::Anticlockwise, QUARTER, &nothing, &obstacles).unwrap();
    let blocked = toward.blocked.as_ref().expect("the sofa is in the way");
    assert_eq!(blocked.by, Blocker::Entity(at(2)));
    let expected = turn_to_reach(0.4);
    assert!(
        (blocked.angle - expected).abs() < 1e-4,
        "blocked at {} for {expected}",
        blocked.angle
    );
    assert_eq!(toward.reaches(), blocked.angle);
    // Away from the sofa nothing is in the way, and the box of the building it stands in is not.
    let away = clearance(&leaf, Way::Clockwise, QUARTER, &nothing, &obstacles).unwrap();
    assert_eq!(away.blocked, None);
    assert_eq!(away.reaches(), QUARTER);

    let empty = room(false);
    let leaf = read_leaf(&empty.doors[0].entity).expect("a leaf");
    for way in [Way::Anticlockwise, Way::Clockwise] {
        let open = clearance(&leaf, way, QUARTER, &nothing, &empty.obstacles_of(0)).unwrap();
        assert_eq!(open.blocked, None, "{way:?}");
    }
}

#[test]
fn a_door_against_a_wall_opens_one_way_and_not_the_other() {
    let found = room(false);
    let leaf = read_leaf(&found.doors[0].entity).expect("a leaf");
    let mut colliders = CollisionSet::new();
    // A wall beside the door on its +Y side, three tenths of a metre off, and a floor the leaf stands on.
    let wall = bounds::boxed(Vec3::new(-1.0, 0.3, 0.0), Vec3::new(2.0, 0.5, 3.0), 1);
    let floor = bounds::boxed(Vec3::new(-5.0, -5.0, -1.0), Vec3::new(5.0, 5.0, 0.0), 2);
    colliders
        .add("wall", collider(wall), Affine3A::IDENTITY)
        .unwrap();
    colliders
        .add("floor", collider(floor), Affine3A::IDENTITY)
        .unwrap();

    let toward = clearance(&leaf, Way::Anticlockwise, QUARTER, &colliders, &[]).unwrap();
    let blocked = toward.blocked.expect("the wall is in the way");
    assert!(matches!(blocked.by, Blocker::Collider { key: "wall", .. }));
    let expected = turn_to_reach(0.3);
    assert!(
        (blocked.angle - expected).abs() < 1e-4,
        "blocked at {} for {expected}",
        blocked.angle
    );
    let away = clearance(&leaf, Way::Clockwise, QUARTER, &colliders, &[]).unwrap();
    assert_eq!(away.blocked, None);
}

#[test]
fn what_the_closed_leaf_stands_in_does_not_block_it() {
    // A leaf whose box dips two tenths of a metre into the floor it stands on.
    let leaf = leaf_of([0.0, -0.02, -0.2], LEAF_MAX, [0.0; 3], 0.0).expect("a leaf");
    let mut colliders = CollisionSet::new();
    let floor = bounds::boxed(Vec3::new(-5.0, -5.0, -1.0), Vec3::new(5.0, 5.0, 0.0), 2);
    colliders
        .add("floor", collider(floor), Affine3A::IDENTITY)
        .unwrap();
    for way in [Way::Anticlockwise, Way::Clockwise] {
        let open = clearance(&leaf, way, QUARTER, &colliders, &[]).unwrap();
        assert_eq!(open.blocked, None, "{way:?}");
    }
}

#[test]
fn the_wall_a_leaf_hangs_in_and_the_floor_its_box_dips_into_do_not_block_it() {
    // The wall ends half a centimetre short of the hinge, so the closed leaf does not touch it; the end of the leaf by the hinge
    // turns inside it all the same.
    let leaf = leaf_of(LEAF_MIN, LEAF_MAX, [0.0; 3], 0.0).expect("a leaf");
    let mut hung = CollisionSet::new();
    let wall = bounds::boxed(Vec3::new(-2.0, -0.1, 0.0), Vec3::new(-0.005, 0.1, 3.0), 1);
    hung.add("wall", collider(wall), Affine3A::IDENTITY)
        .unwrap();
    for way in [Way::Anticlockwise, Way::Clockwise] {
        let open = clearance(&leaf, way, QUARTER, &hung, &[]).unwrap();
        assert_eq!(open.blocked, None, "{way:?}");
    }

    // A floor of two triangles that meet along the line y = x: the closed leaf lies over one, and its box dips half the sill
    // into both as it turns.
    let dip = ARC_SILL * 0.5;
    let dipping = leaf_of([0.0, -0.02, -dip], LEAF_MAX, [0.0; 3], 0.0).expect("a leaf");
    let corners = [
        Vec3::new(-5.0, -5.0, 0.0),
        Vec3::new(5.0, -5.0, 0.0),
        Vec3::new(5.0, 5.0, 0.0),
        Vec3::new(-5.0, 5.0, 0.0),
    ];
    let floor = bounds::triangles(&corners, &[[0, 1, 2], [0, 2, 3]]);
    let mut stood_on = CollisionSet::new();
    let floor = or_io::gta5::PhBoundAny::PhBoundGeometry(floor);
    stood_on
        .add("floor", collider(floor), Affine3A::IDENTITY)
        .unwrap();
    for way in [Way::Anticlockwise, Way::Clockwise] {
        let open = clearance(&dipping, way, QUARTER, &stood_on, &[]).unwrap();
        assert_eq!(open.blocked, None, "{way:?}");
    }
    // A box that dips further than the sill meets the triangle it did not stand on.
    let sunk = leaf_of([0.0, -0.02, -2.0 * ARC_SILL], LEAF_MAX, [0.0; 3], 0.0).expect("a leaf");
    let met = clearance(&sunk, Way::Anticlockwise, QUARTER, &stood_on, &[]).unwrap();
    assert!(matches!(
        met.blocked,
        Some(Blocked {
            by: Blocker::Collider { key: "floor", .. },
            ..
        })
    ));
}

#[test]
fn an_arc_holds_what_the_leaf_passes_through_and_a_leaf_with_no_hinge_sweeps_none() {
    let leaf = leaf_of(LEAF_MIN, LEAF_MAX, [0.0; 3], 0.0).expect("a leaf");
    let arc = DoorArc::of(&leaf, QUARTER).expect("an arc");
    assert_eq!((arc.bottom, arc.top), (0.0, 2.0));
    assert!((arc.radius - Vec2::new(1.0, 0.02).length()).abs() < 1e-6);
    assert!(arc.closed.abs_diff_eq(Vec3::X, 1e-6));

    let on = |degrees: f32, from_hinge: f32, height: f32| {
        let turn = degrees.to_radians();
        Vec3::new(turn.cos(), turn.sin(), 0.0) * from_hinge + Vec3::Z * height
    };
    // In the closed leaf, and where the leaf is at 45 degrees either way and at the quarter turn.
    for (degrees, from_hinge) in [
        (0.0, 0.5),
        (45.0, 0.9),
        (-45.0, 0.9),
        (90.0, 0.99),
        (-90.0, 0.3),
    ] {
        assert!(
            arc.contains(on(degrees, from_hinge, 1.0)),
            "{degrees} at {from_hinge}"
        );
    }
    // Past the quarter turn, past the free edge, and above the leaf.
    assert!(!arc.contains(on(100.0, 0.9, 1.0)));
    assert!(!arc.contains(on(-100.0, 0.9, 1.0)));
    assert!(!arc.contains(on(45.0, 1.1, 1.0)));
    assert!(!arc.contains(on(45.0, 0.9, 2.5)));
    // A smaller arc holds less.
    let narrow = DoorArc::of(&leaf, 30.0_f32.to_radians()).expect("an arc");
    assert!(!narrow.contains(on(45.0, 0.9, 1.0)));

    // The arc of a leaf that stands elsewhere, turned and leaning, is the same arc carried there.
    let (stands, leans) = (
        Vec3::new(30.0, -40.0, 5.0),
        Quat::from_rotation_z(2.0) * Quat::from_rotation_x(0.4),
    );
    let placed = leaf_turned(LEAF_MIN, LEAF_MAX, stands.to_array(), leans).expect("a leaf");
    let carried = DoorArc::of(&placed, QUARTER).expect("an arc");
    for (degrees, from_hinge, height) in [
        (0.0, 0.5, 1.0),
        (45.0, 0.9, 1.0),
        (-90.0, 0.3, 1.0),
        (100.0, 0.9, 1.0),
        (45.0, 1.1, 1.0),
        (45.0, 0.9, 2.5),
    ] {
        let point = on(degrees, from_hinge, height);
        assert_eq!(
            carried.contains(stands + leans * point),
            arc.contains(point),
            "{degrees} at {from_hinge}, {height} up"
        );
    }
    assert!(!carried.contains(on(45.0, 0.9, 1.0)));

    let nothing: CollisionSet<&str> = CollisionSet::new();
    let sliding = leaf_of([-0.5, -0.02, 0.0], [0.5, 0.02, 2.0], [0.0; 3], 0.0).expect("a leaf");
    assert_eq!(DoorArc::of(&sliding, QUARTER), Err(ArcError::NoHinge));
    assert_eq!(
        clearance(&sliding, Way::Clockwise, QUARTER, &nothing, &[]),
        Err(ArcError::NoHinge)
    );
    for angle in [0.0, -1.0, 4.0, f32::NAN] {
        assert!(matches!(
            clearance(&leaf, Way::Clockwise, angle, &nothing, &[]),
            Err(ArcError::Angle(_))
        ));
    }
}

/// The turn at which the leading face of the swept leaf, turning toward +Y about the origin, comes to the point (`x`, `y`).
fn turn_to_touch(x: f32, y: f32) -> f32 {
    let point = Vec2::new(x, y);
    point.y.atan2(point.x) - ((LEAF_MAX[1] - ARC_SKIN) / point.length()).asin()
}

/// A leaf at the origin beside a bench three metres long whose origin is at one end, stood on that end's side of the leaf and
/// turned a quarter, so that it covers x from 0.3 to 0.7 and y from 0.6 to 3.6; the whole carried by `turn` and then `shift`.
fn beside_a_bench(turn: Quat, shift: Vec3) -> Found {
    let types = type_file(vec![
        archetype("shop_door_l", LEAF_MIN, LEAF_MAX, false),
        archetype("bench_a", [0.0, -0.2, 0.0], [3.0, 0.2, 1.0], false),
    ]);
    let placed = |name: &str, position: [f32; 3], yaw: f32| {
        let position = shift + turn * Vec3::from(position);
        let rotation = turn * Quat::from_rotation_z(yaw);
        entity_of("CEntityDef", name, position.to_array(), rotation)
    };
    let map = map_file(vec![
        placed("shop_door_l", [0.0; 3], 0.0),
        placed("bench_a", [0.5, 0.6, 0.0], QUARTER),
    ]);
    found_in(&map, &types, &["shop_door_l", "bench_a"])
}

#[test]
fn a_turned_box_blocks_a_door_where_it_stands_and_a_scene_carried_whole_blocks_it_at_the_same_angle(
) {
    let nothing: CollisionSet<&str> = CollisionSet::new();
    // The leaf's free edge passes the line of the bench's face beyond the bench's end, and its face then comes to the bench's
    // nearest corner.
    let expected = turn_to_touch(0.7, 0.6);
    assert!(expected > turn_to_reach(0.6));
    let carried = [
        (Quat::IDENTITY, Vec3::ZERO),
        (Quat::from_rotation_z(2.0), Vec3::new(300.0, -40.0, 12.0)),
        // A scene that leans: the leaf turns about its own upright line, not the world's.
        (
            Quat::from_rotation_z(-1.1) * Quat::from_rotation_x(0.5),
            Vec3::new(-25.0, 8.0, 3.0),
        ),
    ];
    for (turn, shift) in carried {
        let found = beside_a_bench(turn, shift);
        let leaf = read_leaf(&found.doors[0].entity).expect("a leaf");
        let obstacles = found.obstacles_of(0);
        let toward = clearance(&leaf, Way::Anticlockwise, QUARTER, &nothing, &obstacles).unwrap();
        let blocked = toward.blocked.expect("the bench is in the way");
        assert_eq!(blocked.by, Blocker::Entity(at(1)), "{turn:?}");
        // What a 32-bit position far from the origin cannot tell apart is forgiven the carried scenes.
        assert!(
            (blocked.angle - expected).abs() < 2e-4,
            "{turn:?}: blocked at {} for {expected}",
            blocked.angle
        );
        let away = clearance(&leaf, Way::Clockwise, QUARTER, &nothing, &obstacles).unwrap();
        assert_eq!(away.blocked, None, "{turn:?}");
    }
}

#[test]
fn a_post_thinner_than_a_step_of_a_long_leaf_is_not_passed_over() {
    let nothing: CollisionSet<&str> = CollisionSet::new();
    let (reach, half, out) = (5.0, 0.02, 4.9);
    let types = type_file(vec![
        archetype("yard_door_long", LEAF_MIN, [reach, 0.02, 2.0], false),
        archetype("post_a", [-half, -half, 0.0], [half, half, 2.0], false),
    ]);
    // The post stands square to the leaf that meets it, half a degree past a whole number of them.
    let bearing = 10.5_f32.to_radians();
    let post = [out * bearing.cos(), out * bearing.sin(), 0.0];
    let map = map_file(vec![
        entity("yard_door_long", [0.0; 3], 0.0),
        entity("post_a", post, bearing),
    ]);
    let found = found_in(&map, &types, &["yard_door_long", "post_a"]);
    let leaf = read_leaf(&found.doors[0].entity).expect("a leaf");

    let toward = clearance(
        &leaf,
        Way::Anticlockwise,
        QUARTER,
        &nothing,
        &found.obstacles_of(0),
    )
    .unwrap();
    let blocked = toward.blocked.expect("the post is in the way");
    assert_eq!(blocked.by, Blocker::Entity(at(1)));
    // The leaf's face meets the corner of the post nearest the hinge on the side it comes from.
    let corner = Vec2::new(out - half, -half);
    let expected =
        bearing + corner.y.atan2(corner.x) - ((0.02 - ARC_SKIN) / corner.length()).asin();
    assert!(
        (blocked.angle - expected).abs() < 1e-4,
        "blocked at {} for {expected}",
        blocked.angle
    );
}

#[test]
fn a_leaf_hinged_at_the_high_end_of_its_y_is_swept_as_one_hinged_at_the_low_end_of_its_x() {
    // The scenes of the two tests above it, turned a quarter clockwise: X goes to -Y and Y to X.
    let leaf = leaf_of([-0.02, -1.0, 0.0], [0.02, 0.0, 2.0], [0.0; 3], 0.0).expect("a leaf");
    let hinge = leaf.hinge.expect("a hinge");
    assert_eq!(
        (leaf.width_axis, hinge.side),
        (WidthAxis::Y, HingeSide::High)
    );

    // The wall the leaf hangs in ends half a centimetre short of the hinge.
    let mut hung = CollisionSet::new();
    let wall = bounds::boxed(Vec3::new(-0.1, 0.005, 0.0), Vec3::new(0.1, 2.0, 3.0), 1);
    hung.add("wall", collider(wall), Affine3A::IDENTITY)
        .unwrap();
    for way in [Way::Anticlockwise, Way::Clockwise] {
        let open = clearance(&leaf, way, QUARTER, &hung, &[]).unwrap();
        assert_eq!(open.blocked, None, "{way:?}");
    }

    // A wall beside the door, three tenths of a metre off the side it turns to anticlockwise.
    let mut beside = CollisionSet::new();
    let wall = bounds::boxed(Vec3::new(0.3, -2.0, 0.0), Vec3::new(0.5, 1.0, 3.0), 1);
    beside
        .add("wall", collider(wall), Affine3A::IDENTITY)
        .unwrap();
    let toward = clearance(&leaf, Way::Anticlockwise, QUARTER, &beside, &[]).unwrap();
    let blocked = toward.blocked.expect("the wall is in the way");
    let expected = turn_to_reach(0.3);
    assert!(
        (blocked.angle - expected).abs() < 1e-4,
        "blocked at {} for {expected}",
        blocked.angle
    );
    let away = clearance(&leaf, Way::Clockwise, QUARTER, &beside, &[]).unwrap();
    assert_eq!(away.blocked, None);
}
