use super::*;

// At (0, 0.5, 0.5), for the distances; a wall at x = k spans y and z in 0..1.
const CAMERA: Vec3 = Vec3::new(0.0, 0.5, 0.5);

fn wall(x: f32) -> Vec<Vec3> {
    vec![
        Vec3::new(x, 0.0, 0.0),
        Vec3::new(x, 0.0, 1.0),
        Vec3::new(x, 1.0, 1.0),
        Vec3::new(x, 1.0, 0.0),
    ]
}

fn portal(room_a: u32, room_b: u32, flags: u32, x: f32) -> Portal {
    let def = PortalDef {
        room_a,
        room_b,
        flags,
        opacity: 100,
        corners: wall(x),
    };
    Portal {
        def,
        shut: false,
        link: None,
    }
}

fn interior(portals: Vec<Portal>, rooms: Vec<Vec<usize>>) -> Interior {
    Interior {
        portals,
        rooms,
        proxy_state: POPULATED_PROXY_STATE,
        bounds_min: Vec3::splat(-10.0),
        bounds_max: Vec3::splat(10.0),
    }
}

fn room(interior: usize, room: usize) -> RoomRef {
    RoomRef { interior, room }
}

fn walk(interiors: &[Interior], start: RoomRef, depth: u32) -> Visibility {
    visible_rooms(interiors, start, &[], depth)
}

fn two_rooms(p: Portal) -> Vec<Interior> {
    vec![interior(vec![p], vec![vec![], vec![0], vec![0]])]
}

#[test]
fn the_wall_normal_is_worked_by_hand() {
    // (0,0,1) x (0,1,1) = (-1,0,0) and (0,1,1) x (0,1,0) = (-1,0,0): sum (-2,0,0).
    assert_eq!(polygon_normal(&wall(1.0)), Vec3::new(-2.0, 0.0, 0.0));
}

#[test]
fn an_open_portal_shows_the_next_room() {
    let seen = walk(&two_rooms(portal(1, 2, 0, 1.0)), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1), room(0, 2)]);
    assert!(!seen.exterior_seen);
}

#[test]
fn a_portal_is_crossed_from_either_room() {
    // Room 1 is room B here; with no facing test the walk still crosses into room A.
    let seen = walk(&two_rooms(portal(2, 1, 0, 1.0)), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1), room(0, 2)]);
}

#[test]
fn a_mirror_is_never_traversed() {
    let seen = walk(&two_rooms(portal(1, 2, PORTAL_MIRROR, 1.0)), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1)]);
}

#[test]
fn a_shut_portal_is_not_traversed() {
    let mut shut = portal(1, 2, 0, 1.0);
    shut.shut = true;
    let seen = walk(&two_rooms(shut), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1)]);
}

#[test]
fn a_portal_outside_the_frustum_is_not_seen() {
    let interiors = two_rooms(portal(1, 2, 0, 1.0));
    // Plane x <= 0.5: every corner gives -1 + 0.5 = -0.5, outside. Plane x <= 1.5: 0.5, inside.
    let near = visible_rooms(&interiors, room(0, 1), &[Vec4::new(-1.0, 0.0, 0.0, 0.5)], 8);
    assert_eq!(near.rooms, vec![room(0, 1)]);
    let far = visible_rooms(&interiors, room(0, 1), &[Vec4::new(-1.0, 0.0, 0.0, 1.5)], 8);
    assert_eq!(far.rooms, vec![room(0, 1), room(0, 2)]);
}

#[test]
fn a_polygon_is_in_unless_one_plane_has_every_corner_outside() {
    let square = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    // x <= 0.5: the corners at x 0 give 0.5, inside, those at x 1 give -0.5, outside; one corner inside is enough.
    let half = Vec4::new(-1.0, 0.0, 0.0, 0.5);
    assert!(polygon_in_frustum(&square, &[half]));
    // x >= 2: the corners give -2 and -1, every one outside, so the square is out although the first plane holds it.
    let beyond = Vec4::new(1.0, 0.0, 0.0, -2.0);
    assert!(!polygon_in_frustum(&square, &[half, beyond]));
    // No corner is no polygon.
    assert!(!polygon_in_frustum(&[], &[half]));
}

#[test]
fn a_portal_to_room_zero_shows_the_exterior() {
    let interiors = [interior(vec![portal(1, 0, 0, 1.0)], vec![vec![0], vec![0]])];
    let seen = walk(&interiors, room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1)]);
    assert!(seen.exterior_seen);
    let through = PortalRef {
        interior: 0,
        portal: 0,
    };
    assert_eq!(seen.exterior_portals, vec![through]);
}

#[test]
fn a_portal_not_touching_the_room_is_skipped() {
    // Listed under room 1 but joining rooms 2 and 3.
    let interiors = [interior(
        vec![portal(2, 3, 0, 1.0)],
        vec![vec![], vec![0], vec![0], vec![0]],
    )];
    assert_eq!(walk(&interiors, room(0, 1), 8).rooms, vec![room(0, 1)]);
}

fn linked(far: Interior) -> Vec<Interior> {
    let mut link = portal(1, 0, PORTAL_LINK, 1.0);
    link.link = Some(LinkTarget {
        interior: 1,
        slot: 0,
    });
    vec![interior(vec![link], vec![vec![0], vec![0]]), far]
}

#[test]
fn a_link_continues_into_the_other_interior() {
    // Without the link rule the portal touches room 0 and would show the exterior through (0, 0).
    let far = interior(vec![portal(1, 0, PORTAL_LINK, 2.0)], vec![vec![0], vec![0]]);
    let seen = walk(&linked(far), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1), room(1, 1)]);
    assert!(!seen.exterior_seen);
}

#[test]
fn a_link_into_an_interior_with_no_rooms_stops() {
    let far = interior(vec![portal(1, 0, PORTAL_LINK, 2.0)], vec![]);
    let seen = walk(&linked(far), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1)]);
    assert!(!seen.exterior_seen);
}

#[test]
fn a_link_met_from_room_zero_leads_into_its_own_room() {
    // From (0, 0) the link portal (1, 0) leads to room 1 of interior 0; jumping the link would draw (1, 1) instead.
    let far = interior(vec![portal(1, 0, PORTAL_LINK, 2.0)], vec![vec![0], vec![0]]);
    let seen = walk(&linked(far), room(0, 0), 8);
    assert_eq!(seen.rooms, vec![room(0, 0), room(0, 1)]);
    assert!(seen.exterior_seen);
}

#[test]
fn the_walk_never_leaves_by_the_portal_it_entered_through() {
    // The far portal has no link bit and touches room 1 and room 0, so crossing it again would show the exterior through (1, 0).
    let far = interior(vec![portal(1, 0, 0, 2.0)], vec![vec![0], vec![0]]);
    let seen = walk(&linked(far), room(0, 1), 8);
    assert_eq!(seen.rooms, vec![room(0, 1), room(1, 1)]);
    assert!(!seen.exterior_seen);
    assert!(seen.exterior_portals.is_empty());
}

#[test]
fn the_depth_limit_bounds_the_walk() {
    // 1 -> 2 at x 1, 2 -> 3 at x 2.
    let interiors = [interior(
        vec![portal(1, 2, 0, 1.0), portal(2, 3, 0, 2.0)],
        vec![vec![], vec![0], vec![0, 1], vec![1]],
    )];
    let rooms = |depth: u32| walk(&interiors, room(0, 1), depth).rooms;
    assert_eq!(rooms(0), vec![room(0, 1)]);
    assert_eq!(rooms(1), vec![room(0, 1), room(0, 2)]);
    assert_eq!(rooms(2), vec![room(0, 1), room(0, 2), room(0, 3)]);
}

#[test]
fn a_room_is_reached_at_its_least_depth() {
    // p0 1->2, p1 1->3, p2 2->3, p3 3->4, max depth 2. Breadth first: 2 and 3 at depth 1, then 4 at depth 2.
    // Depth first would go 1->2->3 (depth 2), stop there, find 3 seen through p1, and never reach 4.
    let interiors = [interior(
        vec![
            portal(1, 2, 0, 1.0),
            portal(1, 3, 0, 1.5),
            portal(2, 3, 0, 2.0),
            portal(3, 4, 0, 3.0),
        ],
        vec![vec![], vec![0, 1], vec![0, 2], vec![1, 2, 3], vec![3]],
    )];
    let all = vec![room(0, 1), room(0, 2), room(0, 3), room(0, 4)];
    assert_eq!(walk(&interiors, room(0, 1), 2).rooms, all);
}

#[test]
fn a_walk_from_outside_sees_the_exterior_and_the_room_behind_the_door() {
    let interiors = [interior(vec![portal(0, 1, 0, 1.0)], vec![vec![0], vec![0]])];
    let seen = walk(&interiors, room(0, 0), 8);
    assert_eq!(seen.rooms, vec![room(0, 0), room(0, 1)]);
    assert!(seen.exterior_seen);
    assert!(seen.exterior_portals.is_empty());
}

#[test]
fn a_start_in_no_known_room_sees_nothing() {
    let interiors = two_rooms(portal(1, 2, 0, 1.0));
    assert_eq!(walk(&interiors, room(5, 1), 8), Visibility::default());
    assert_eq!(walk(&interiors, room(0, 9), 8), Visibility::default());
}

#[test]
fn a_start_outside_an_interior_with_no_rooms_still_sees_the_exterior() {
    let seen = walk(&[interior(vec![], vec![])], room(0, 0), 8);
    assert!(seen.rooms.is_empty());
    assert!(seen.exterior_seen);
}

#[test]
fn the_point_to_polygon_distance_is_worked_by_hand() {
    let square = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    let at = |p: Vec3| point_polygon_distance(p, &square).unwrap_or(f32::NAN);
    // Above the inside: the height 2 (the nearest edge alone would give sqrt(0.25 + 4) = 2.06).
    assert!((at(Vec3::new(0.5, 0.5, 2.0)) - 2.0).abs() < 1e-6);
    // Beside the edge x 1: (3, 0.5, 0) to (1, 0.5, 0) is 2.
    assert!((at(Vec3::new(3.0, 0.5, 0.0)) - 2.0).abs() < 1e-6);
    // Past the corner (1, 1, 0): sqrt(1 + 1 + 1).
    assert!((at(Vec3::new(2.0, 2.0, 1.0)) - 3.0_f32.sqrt()).abs() < 1e-6);
    // One corner is a point; none is no polygon.
    let point = point_polygon_distance(Vec3::ONE, &[Vec3::ZERO]).unwrap_or(f32::NAN);
    assert!((point - 3.0_f32.sqrt()).abs() < 1e-6);
    assert_eq!(point_polygon_distance(Vec3::ONE, &[]), None);
}

#[test]
fn only_populated_interiors_near_the_camera_give_distances() {
    // The wall at x 1 is 1 m from the camera: its foot (1, 0.5, 0.5) is inside the quad.
    let near = interior(vec![portal(0, 1, 0, 1.0)], vec![vec![0], vec![0]]);
    let distances = exterior_portal_distances(CAMERA, std::slice::from_ref(&near));
    assert_eq!(distances.len(), 1);
    assert!((distances[0] - 1.0).abs() < 1e-6);
    // Proxy state 3 is under 4.
    let mut unpopulated = near.clone();
    unpopulated.proxy_state = 3;
    assert!(exterior_portal_distances(CAMERA, &[unpopulated]).is_empty());
    // Bounds from x 5: the camera box ends at x 4. From x 3 they overlap.
    let mut away = near.clone();
    away.bounds_min = Vec3::new(5.0, -10.0, -10.0);
    assert!(exterior_portal_distances(CAMERA, std::slice::from_ref(&away)).is_empty());
    away.bounds_min = Vec3::new(3.0, -10.0, -10.0);
    assert_eq!(exterior_portal_distances(CAMERA, &[away]).len(), 1);
}
