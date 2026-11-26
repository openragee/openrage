use super::*;
use or_world::streaming_rules::{
    PORTAL_ALLOW_CLOSING, PORTAL_LINK as LINK, PORTAL_MIRROR as MIRROR,
};

/// Three rooms 4 m long in a row along x, 4 m wide and 3 high: a front door from the outside into
/// room 1 at x = 0, a door from room 1 into room 2 at x = 4 and one from room 2 into room 3 at
/// x = 8 near the north wall.
fn row() -> (Vec<RoomBox>, Vec<Opening>) {
    let room = |x: f32, to: f32| RoomBox {
        min: Vec3::new(x, -2.0, 0.0),
        max: Vec3::new(to, 2.0, 3.0),
    };
    let door = |rooms, x: f32, y: [f32; 2]| Opening {
        rooms,
        corners: [
            Vec3::new(x, y[0], 0.0),
            Vec3::new(x, y[1], 0.0),
            Vec3::new(x, y[1], 2.0),
            Vec3::new(x, y[0], 2.0),
        ],
        flags: 0,
        shut: false,
        door: false,
    };
    let rooms = vec![
        room(0.0, 12.0),
        room(0.0, 4.0),
        room(4.0, 8.0),
        room(8.0, 12.0),
    ];
    let portals = vec![
        door([1, 0], 0.0, [-0.5, 0.5]),
        door([1, 2], 4.0, [-0.5, 0.5]),
        door([2, 3], 8.0, [1.0, 1.8]),
    ];
    (rooms, portals)
}

/// What a camera at `eye` sees of one interior of `rooms` and `portals` standing where its own space puts it.
fn seen(rooms: &[RoomBox], portals: &[Opening], eye: [f32; 3]) -> Seen {
    let shell = Shell {
        rooms,
        portals,
        own: Mat4::IDENTITY,
    };
    walk(&[shell], Vec3::from(eye)).seen.remove(0)
}

fn rooms_seen(eye: [f32; 3]) -> Vec<bool> {
    let (rooms, portals) = row();
    seen(&rooms, &portals, eye).rooms
}

#[test]
fn the_camera_stands_in_the_smallest_room_that_holds_it() {
    let (rooms, ..) = row();
    assert_eq!(room_of(&rooms, Vec3::new(2.0, 0.0, 1.0)), Some(1));
    assert_eq!(room_of(&rooms, Vec3::new(6.0, 0.0, 1.0)), Some(2));
    // Room 0 holds every room and is never the one the camera stands in.
    assert_eq!(room_of(&rooms, Vec3::new(-5.0, 0.0, 1.0)), None);
}

#[test]
fn from_a_room_the_camera_sees_every_room_its_open_doors_lead_to() {
    // Wherever it stands in room 1, it sees through the doors whatever way it looks.
    for eye in [[1.0, 0.0, 1.0], [1.0, -1.5, 1.0], [1.0, 1.5, 1.0]] {
        assert_eq!(rooms_seen(eye), [true, true, true, true], "{eye:?}");
    }
    let (rooms, portals) = row();
    assert_eq!(seen(&rooms, &portals, [1.0, 1.5, 1.0]).portals, [true; 3]);
}

#[test]
fn from_outside_the_camera_sees_in_through_the_shell_s_open_doors_from_any_side() {
    assert_eq!(rooms_seen([-5.0, 0.0, 1.0]), [true; 4]);
    // The build's walk tests no side of a portal: from behind the building the rooms are seen too.
    assert_eq!(rooms_seen([20.0, 0.0, 1.0]), [true; 4]);
}

#[test]
fn inside_the_sky_s_ambient_light_comes_in_through_the_openings_to_the_outside_alone() {
    let (rooms, portals) = row();
    let at = |room, p: [f32; 3]| ambient_of(&rooms, &portals, room, Vec3::from(p), 0.0);
    // In the front door, the whole of the sky's.
    assert_eq!(at(1, [0.0, 0.0, 1.0]), [1.0, 0.0]);
    // A metre in, the room's daylight factor: a door of 2 m² in a room of 80 m² of surface.
    let factor = 2.0 * 90.0 / (100.0 * 80.0 * 0.75);
    let inside = at(1, [1.0, 0.0, 1.0]);
    assert!(
        (inside[0] - factor).abs() < 1e-6 && inside[1] == 0.0,
        "{inside:?} {factor}"
    );
    // Half way out along the door's width past its edge, more than the room's and less than the whole.
    let edge = at(1, [0.0, 0.5 * 1.125, 1.0]);
    assert!(
        edge[0] > factor && edge[0] < 1.0 && edge[1] == 0.0,
        "{edge:?}"
    );
    // Room 2 has no opening to the outside, even beside the door into room 1.
    assert_eq!(at(2, [4.0, 0.0, 1.0]), [0.0, 0.0]);
    // What stands in room 0 stands outdoors.
    assert_eq!(at(0, [1.0, 0.0, 1.0]), or_renderer::OUTDOORS);
    // A mirror on the front, or a link to another interior, lets in none of the sky's.
    for flags in [MIRROR, LINK] {
        let mut shut = portals.clone();
        shut[0].flags = flags;
        assert_eq!(
            ambient_of(&rooms, &shut, 1, Vec3::new(0.0, 0.0, 1.0), 0.0),
            [0.0, 0.0]
        );
    }
}

#[test]
fn a_room_s_lamps_light_it_all_round_by_what_its_surfaces_send_back() {
    let (rooms, portals) = row();
    let at = |room, flux| ambient_of(&rooms, &portals, room, Vec3::new(5.0, 0.0, 1.0), flux)[1];
    // Room 2, 80 m² of surface: 8,000 lm, half of it sent back again and again, are 100 lux.
    let bounced = 8000.0 * 0.5 / (80.0 * 0.5);
    assert!((at(2, 8000.0) - bounced / ARTIFICIAL_AMBIENT_LUX).abs() < 1e-6);
    assert_eq!(at(2, 0.0), 0.0);
    // The share is at most the whole of the frame's artificial light.
    assert_eq!(at(2, 1e9), 1.0);
    // What stands outdoors takes none.
    assert_eq!(at(0, 8000.0), 0.0);
}

#[test]
fn a_sphere_is_in_the_smallest_room_that_holds_its_middle_and_is_wider_than_it() {
    let (rooms, ..) = row();
    assert_eq!(room_around(&rooms, Vec3::new(2.0, 0.0, 1.0), 0.5), Some(1));
    // A prop against the wall whose sphere passes the room's box is still in the room.
    assert_eq!(room_around(&rooms, Vec3::new(2.0, 1.8, 1.0), 1.0), Some(1));
    // A shell wider than its room is in no room, nor what stands outside every room.
    assert_eq!(room_around(&rooms, Vec3::new(2.0, 0.0, 1.5), 3.5), None);
    assert_eq!(room_around(&rooms, Vec3::new(2.0, 5.0, 1.0), 0.5), None);
}

#[test]
fn a_mirror_is_seen_and_nothing_is_seen_through_it() {
    let (rooms, mut portals) = row();
    portals[1].flags = MIRROR;
    let seen = seen(&rooms, &portals, [1.0, 0.0, 1.0]);
    assert_eq!(seen.rooms, [true, true, false, false]);
    assert_eq!(seen.portals, [true, true, false]);
}

#[test]
fn a_shut_door_is_seen_and_lets_neither_the_eye_nor_the_sky_through() {
    let (rooms, mut portals) = row();
    portals[1].shut = true;
    let from_room_1 = seen(&rooms, &portals, [1.0, 0.0, 1.0]);
    assert_eq!(from_room_1.rooms, [true, true, false, false]);
    assert_eq!(from_room_1.portals, [true, true, false]);
    portals[0].shut = true;
    assert_eq!(
        seen(&rooms, &portals, [-5.0, 0.0, 1.0]).rooms,
        [true, false, false, false]
    );
    assert_eq!(
        ambient_of(&rooms, &portals, 1, Vec3::new(0.0, 0.0, 1.0), 0.0),
        [0.0, 0.0]
    );
}

#[test]
fn a_door_in_the_front_opening_keeps_the_sky_from_the_room_and_takes_it_where_it_stands() {
    let (rooms, mut portals) = row();
    portals[0].door = true;
    let at = |portals: &[Opening], p: [f32; 3]| ambient_of(&rooms, portals, 1, Vec3::from(p), 0.0);
    assert_eq!(at(&portals, [1.0, 0.0, 1.0]), [0.0, 0.0]);
    assert_eq!(at(&portals, [0.0, 0.0, 1.0]), [1.0, 0.0]);
    // It shuts nothing: the eye still goes in.
    assert_eq!(seen(&rooms, &portals, [-5.0, 0.0, 1.0]).rooms, [true; 4]);
}

#[test]
fn a_portal_that_may_close_is_shut_at_rest_by_a_door_and_open_with_none() {
    let closing = PORTAL_ALLOW_CLOSING | 8;
    // A door shuts it, one-way or not; with no door it stays open.
    assert!(shut_at_rest(closing, 1));
    assert!(shut_at_rest(closing | 1, 2));
    assert!(!shut_at_rest(closing, 0));
    // A link's doors never reach it, and one that may not close never shuts.
    assert!(!shut_at_rest(closing | LINK, 1));
    assert!(!shut_at_rest(8 | 1, 1));
}

#[test]
fn from_a_room_the_outside_is_seen_through_the_doors_to_it_alone() {
    let (rooms, mut portals) = row();
    let eye = [2.0, 0.0, 1.0];
    let outside = |portals: &[Opening]| seen(&rooms, portals, eye).outside;
    let through = outside(&portals).expect("the camera stands in a room");
    assert_eq!(through.len(), 1, "one door to the outside");
    let eye = Vec3::from(eye);
    let sees = |p: [f32; 3], r: f32| reaches(&through[0], eye, Vec3::from(p), r);
    // Straight out of the front door, and a sphere that reaches into its line.
    assert!(sees([-10.0, 0.0, 1.0], 0.1));
    assert!(sees([-10.0, 4.0, 1.0], 2.0));
    // Off to the side of it, and behind the camera, through the walls.
    assert!(!sees([-10.0, 8.0, 1.0], 1.0));
    assert!(!sees([10.0, 0.0, 1.0], 1.0));
    // From the outside nothing of it is hidden; with its one door shut, all of it is.
    assert_eq!(seen(&rooms, &portals, [-5.0, 0.0, 1.0]).outside, None);
    portals[0].shut = true;
    assert_eq!(outside(&portals), Some(Vec::new()));
    // A link opens on another interior, not on the outside.
    portals[0].shut = false;
    portals[0].flags = LINK;
    assert_eq!(outside(&portals), Some(Vec::new()));
}

#[test]
fn from_a_room_that_sees_outside_the_rooms_seen_from_outside_are_seen_too() {
    let (rooms, mut portals) = row();
    portals[1].shut = true;
    assert_eq!(
        seen(&rooms, &portals, [1.0, 0.0, 1.0]).rooms,
        [true, true, false, false]
    );
    // A second door from the outside, into room 2 through its south wall.
    let mut side = portals[0];
    side.rooms = [2, 0];
    side.corners = [
        Vec3::new(5.0, -2.0, 0.0),
        Vec3::new(6.0, -2.0, 0.0),
        Vec3::new(6.0, -2.0, 2.0),
        Vec3::new(5.0, -2.0, 2.0),
    ];
    portals.push(side);
    assert_eq!(seen(&rooms, &portals, [1.0, 0.0, 1.0]).rooms, [true; 4]);
    // With the front door shut, room 1 sees nothing of the outside, nor what the outside sees.
    portals[0].shut = true;
    assert_eq!(
        seen(&rooms, &portals, [1.0, 0.0, 1.0]).rooms,
        [false, true, false, false]
    );
}

#[test]
fn from_a_room_the_outside_room_is_seen_through_a_door_to_it() {
    let seen = rooms_seen([4.05, 0.0, 1.0]);
    assert!(seen[1] && seen[0], "{seen:?}");
}

#[test]
fn a_link_leads_the_eye_into_the_interior_whose_portal_to_the_outside_stands_where_it_does() {
    let (rooms, mut portals) = row();
    portals[0].flags = LINK;
    // A second interior of one room whose door to the outside is the first one's front door.
    let next_rooms = vec![rooms[0], rooms[1]];
    let next_portals = vec![portals[0]];
    // Shut from the outside, so that only the link shows its room.
    let mut lone = next_portals.clone();
    lone[0].flags = 0;
    lone[0].shut = true;
    let moved = Mat4::from_translation(Vec3::new(0.0, 10.0, 0.0));
    let shell = |rooms, portals, own| Shell {
        rooms,
        portals,
        own,
    };
    let eye = Vec3::new(6.0, 0.0, 1.0);
    let met = walk(
        &[
            shell(&rooms, &portals, Mat4::IDENTITY),
            shell(&next_rooms, &lone, Mat4::IDENTITY),
        ],
        eye,
    );
    assert!(met.seen[1].rooms[1], "through the link");
    assert_eq!(met.standing, Some((0, 2)));
    // Where the other's door stands elsewhere, the link leads nowhere.
    let apart = walk(
        &[
            shell(&rooms, &portals, Mat4::IDENTITY),
            shell(&next_rooms, &lone, moved),
        ],
        eye,
    );
    assert!(!apart.seen[1].rooms[1]);
}

#[test]
fn near_a_door_to_the_outside_the_camera_takes_the_room_past_it() {
    let (rooms, portals) = row();
    let at = |eye: [f32; 3]| {
        let shell = Shell {
            rooms: &rooms,
            portals: &portals,
            own: Mat4::IDENTITY,
        };
        walk(&[shell], Vec3::from(eye))
    };
    // In a room, that room; a metre out of the front door, within a twentieth of the 25 m, room 1.
    assert_eq!(at([6.0, 0.0, 1.0]).inside, Some((0, 2)));
    assert_eq!(at([-1.0, 0.0, 1.0]).inside, Some((0, 1)));
    assert_eq!(at([-1.0, 0.0, 1.0]).standing, None);
    // Ten metres out, outdoors.
    assert_eq!(at([-10.0, 0.0, 1.0]).inside, None);
}

#[test]
fn rooms_that_all_open_on_each_other_are_walked_in_bounded_time() {
    // Sixteen rooms in one box, each pair joined by a door in the plane x = 0 the camera stands in.
    const ROOMS: u32 = 16;
    let rooms = vec![
        RoomBox {
            min: Vec3::splat(-5.0),
            max: Vec3::splat(5.0),
        };
        ROOMS as usize
    ];
    let door = |rooms| Opening {
        rooms,
        corners: [
            Vec3::new(0.0, -1.0, -1.0),
            Vec3::new(0.0, 1.0, -1.0),
            Vec3::new(0.0, 1.0, 1.0),
            Vec3::new(0.0, -1.0, 1.0),
        ],
        flags: 0,
        shut: false,
        door: false,
    };
    let portals: Vec<Opening> = (1..ROOMS)
        .flat_map(|a| (a + 1..ROOMS).map(move |b| door([a, b])))
        .collect();
    let seen = seen(&rooms, &portals, [0.0; 3]);
    assert!(
        seen.rooms[1..].iter().all(|room| *room),
        "every room is seen"
    );
}
