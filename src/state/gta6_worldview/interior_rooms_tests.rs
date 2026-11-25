use super::*;

/// Three rooms 4 m long in a row along x, 4 m wide and 3 high: a front door from the outside into
/// room 1 at x = 0, a door from room 1 into room 2 at x = 4 and one from room 2 into room 3 at
/// x = 8 near the north wall.
fn row() -> (Vec<RoomBox>, Vec<Opening>, Vec3) {
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
    (rooms, portals, Vec3::new(6.0, 0.0, 1.5))
}

fn rooms_seen(eye: [f32; 3]) -> Vec<bool> {
    let (rooms, portals, middle) = row();
    seen(&rooms, &portals, middle, Vec3::from(eye)).rooms
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
fn from_a_room_the_camera_sees_the_rooms_its_doors_open_on() {
    // In the middle of room 1 the door at x = 8 shows through the one at x = 4.
    assert_eq!(rooms_seen([1.0, 0.0, 1.0]), [true, true, true, true]);
    // From the south side of room 1 the north door at x = 8 is out of the line of the door at 4.
    assert_eq!(rooms_seen([1.0, -1.5, 1.0]), [true, true, true, true]);
    assert_eq!(rooms_seen([1.0, 1.5, 1.0]), [true, true, true, false]);
    let (rooms, portals, middle) = row();
    let seen = seen(&rooms, &portals, middle, Vec3::new(1.0, 1.5, 1.0));
    assert_eq!(seen.portals, [true, true, false]);
}

#[test]
fn from_outside_the_camera_sees_in_through_the_shell_s_doors_alone() {
    // Straight in, the door at x = 8 is narrower than what the door at x = 4 lets through, and off its line.
    assert_eq!(rooms_seen([-5.0, 0.0, 1.0]), [true, true, true, false]);
    // From a little south the three doors line up, through a sliver of each.
    assert_eq!(rooms_seen([-5.0, -0.9, 1.0]), [true, true, true, true]);
    // Off to the side, the door at x = 4 is not in line with the front door.
    assert_eq!(rooms_seen([-5.0, 3.0, 1.0]), [true, true, false, false]);
    // From behind the building the front door opens away from the camera.
    assert_eq!(rooms_seen([20.0, 0.0, 1.0]), [true, false, false, false]);
}

#[test]
fn inside_the_sky_s_ambient_light_comes_in_through_the_openings_to_the_outside_alone() {
    let (rooms, portals, _) = row();
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
    let (rooms, portals, _) = row();
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
    let (rooms, mut portals, middle) = row();
    portals[1].flags = MIRROR;
    let seen = seen(&rooms, &portals, middle, Vec3::new(1.0, 0.0, 1.0));
    assert_eq!(seen.rooms, [true, true, false, false]);
    assert_eq!(seen.portals, [true, true, false]);
}

#[test]
fn a_shut_door_is_seen_and_lets_neither_the_eye_nor_the_sky_through() {
    let (rooms, mut portals, middle) = row();
    portals[1].shut = true;
    let from_room_1 = seen(&rooms, &portals, middle, Vec3::new(1.0, 0.0, 1.0));
    assert_eq!(from_room_1.rooms, [true, true, false, false]);
    assert_eq!(from_room_1.portals, [true, true, false]);
    portals[0].shut = true;
    assert_eq!(
        seen(&rooms, &portals, middle, Vec3::new(-5.0, 0.0, 1.0)).rooms,
        [true, false, false, false]
    );
    assert_eq!(
        ambient_of(&rooms, &portals, 1, Vec3::new(0.0, 0.0, 1.0), 0.0),
        [0.0, 0.0]
    );
}

#[test]
fn a_portal_that_may_close_is_shut_by_what_it_holds_to_the_outside_and_by_its_doors_within() {
    let closing = ALLOW_CLOSING | 8;
    // To the outside, whatever it holds shuts it; holding nothing, or looked through one way, never.
    assert!(shut_at_rest(closing, [3, 0], 1, 0));
    assert!(!shut_at_rest(closing, [3, 0], 0, 0));
    assert!(!shut_at_rest(closing | ONE_WAY, [0, 3], 2, 1));
    // Between two rooms, a door shuts it and a frame alone does not.
    assert!(shut_at_rest(closing, [1, 2], 2, 1));
    assert!(!shut_at_rest(closing, [1, 2], 2, 0));
    // One that may not close, or that links another interior, never shuts.
    assert!(!shut_at_rest(8, [1, 0], 2, 1));
    assert!(!shut_at_rest(closing | LINK, [1, 0], 2, 1));
}

#[test]
fn from_a_room_the_outside_is_seen_through_the_doors_to_it_alone() {
    let (rooms, mut portals, middle) = row();
    let eye = Vec3::new(2.0, 0.0, 1.0);
    let outside = |portals: &[Opening]| seen(&rooms, portals, middle, eye).outside;
    let through = outside(&portals).expect("the camera stands in a room");
    assert_eq!(through.len(), 1, "one door to the outside");
    let sees = |p: [f32; 3], r: f32| reaches(&through[0], eye, Vec3::from(p), r);
    // Straight out of the front door, and a sphere that reaches into its line.
    assert!(sees([-10.0, 0.0, 1.0], 0.1));
    assert!(sees([-10.0, 4.0, 1.0], 2.0));
    // Off to the side of it, and behind the camera, through the walls.
    assert!(!sees([-10.0, 8.0, 1.0], 1.0));
    assert!(!sees([10.0, 0.0, 1.0], 1.0));
    // From the outside nothing of it is hidden; with its one door shut, all of it is.
    assert_eq!(
        seen(&rooms, &portals, middle, Vec3::new(-5.0, 0.0, 1.0)).outside,
        None
    );
    portals[0].shut = true;
    assert_eq!(outside(&portals), Some(Vec::new()));
    // A link opens on another interior, not on the outside.
    portals[0].shut = false;
    portals[0].flags = LINK;
    assert_eq!(outside(&portals), Some(Vec::new()));
}

#[test]
fn a_camera_in_a_doorway_sees_through_the_whole_of_it() {
    let (rooms, portals, middle) = row();
    // Five centimetres into room 2 the door's edges pass behind the camera.
    let seen = seen(&rooms, &portals, middle, Vec3::new(4.05, 0.0, 1.0));
    assert!(seen.rooms[1] && seen.rooms[0]);
}

#[test]
fn rooms_that_all_open_on_each_other_where_the_camera_stands_are_walked_in_bounded_time() {
    // Sixteen rooms in one box, each pair joined by a door in the plane x = 0 the camera stands in:
    // every path of eight doors stays open, and there are some five hundred million of them.
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
    };
    let portals: Vec<Opening> = (1..ROOMS)
        .flat_map(|a| (a + 1..ROOMS).map(move |b| door([a, b])))
        .collect();
    let seen = seen(&rooms, &portals, Vec3::ZERO, Vec3::ZERO);
    assert!(
        seen.rooms[1..].iter().all(|room| *room),
        "every room is seen"
    );
    // A door crossed again through what it was crossed through before is not followed: the walk ends.
    assert!(seen.outside.is_some(), "the walk was not cut short");
}

#[test]
fn an_opening_is_within_another_where_each_of_its_corners_is() {
    let square = |x: f32, y: f32, half: f32| {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(a, b)| Vec3::new(x + a * half, y + b * half, 0.0))
    };
    let (big, small) = (square(0.0, 0.0, 2.0), square(0.5, 0.5, 1.0));
    assert!(within(&small, &big, Vec3::Z));
    assert!(within(&big, &big, Vec3::Z));
    assert!(!within(&big, &small, Vec3::Z));
    assert!(!within(&square(2.5, 0.0, 1.0), &big, Vec3::Z));
    // Either way round the corners go, and whichever way the plane faces.
    let mut reversed = big;
    reversed.reverse();
    assert!(within(&small, &reversed, -Vec3::Z));
}
