use glam::{Vec3, Vec4};

use super::*;

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

#[test]
fn a_level_camera_at_no_yaw_looks_along_y_with_x_to_its_right_and_a_drag_right_turns_it_right() {
    let mut camera = FlyCamera::default();
    assert!(close(camera.forward(), Vec3::Y), "{}", camera.forward());
    assert!(close(camera.right(), Vec3::X), "{}", camera.right());
    camera.look(100.0, 0.0);
    assert!(camera.forward().x > 0.1, "{}", camera.forward());
    // A drag down looks down, and never past straight down
    camera.look(0.0, 100.0);
    assert!(camera.forward().z < -0.1, "{}", camera.forward());
    camera.look(0.0, 1.0e6);
    assert!((camera.pitch + 89f32.to_radians()).abs() < 1e-6);
    assert!(close(
        camera.right(),
        camera.forward().cross(Vec3::Z).normalize()
    ));
}

#[test]
fn the_orbit_camera_it_gives_stands_where_it_stands_and_sees_what_it_sees() {
    let camera = FlyCamera {
        position: Vec3::new(120.0, -340.0, 56.0),
        yaw: 0.7,
        pitch: -0.4,
        ..FlyCamera::default()
    };
    let orbit = camera.as_orbit();
    assert!(
        (orbit.eye() - camera.position).length() < 1e-3,
        "{}",
        orbit.eye()
    );
    assert!(close(
        (orbit.target - orbit.eye()).normalize(),
        camera.forward()
    ));
    assert_eq!(
        (orbit.fov_y, orbit.near, orbit.far),
        (camera.fov_y, camera.near, camera.far)
    );
    assert_eq!(camera.view_proj(1.5), orbit.view_proj(1.5));
    // A point straight ahead is drawn in the middle of the frame, and one to the right of it on the right
    let ahead = camera.position + camera.forward() * 30.0;
    let clip = camera.view_proj(1.5) * ahead.extend(1.0);
    assert!(
        (clip.x / clip.w).abs() < 1e-3 && (clip.y / clip.w).abs() < 1e-3,
        "{clip}"
    );
    let aside = camera.view_proj(1.5) * (ahead + camera.right() * 5.0).extend(1.0);
    assert!(aside.x / aside.w > 0.05, "{aside}");
}

#[test]
fn flying_moves_by_the_speed_times_the_time_and_four_times_that_when_fast() {
    let start = FlyCamera {
        yaw: 0.3,
        pitch: 0.2,
        speed: 8.0,
        ..FlyCamera::default()
    };
    let moved = |along: f32, across: f32, up: f32, fast: bool| {
        let mut camera = start.clone();
        camera.fly(along, across, up, fast, 0.5);
        camera.position - start.position
    };
    assert!(close(moved(1.0, 0.0, 0.0, false), start.forward() * 4.0));
    assert!(close(moved(-1.0, 0.0, 0.0, false), start.forward() * -4.0));
    assert!(close(moved(0.0, 1.0, 0.0, false), start.right() * 4.0));
    assert!(close(moved(0.0, 0.0, 1.0, false), Vec3::Z * 4.0));
    assert!(close(moved(1.0, 0.0, 0.0, true), start.forward() * 16.0));
}

#[test]
fn a_wheel_notch_scales_the_speed_by_a_fifth_within_the_two_bounds() {
    let mut camera = FlyCamera {
        speed: 10.0,
        ..FlyCamera::default()
    };
    camera.scale_speed(1.0);
    assert!((camera.speed - 12.0).abs() < 1e-4, "{}", camera.speed);
    camera.scale_speed(-1.0);
    assert!((camera.speed - 10.0).abs() < 1e-4, "{}", camera.speed);
    camera.scale_speed(1000.0);
    assert_eq!(camera.speed, FlyCamera::MAX_SPEED);
    camera.scale_speed(-1000.0);
    assert_eq!(camera.speed, FlyCamera::MIN_SPEED);
}

#[test]
fn the_frustum_sees_a_sphere_ahead_and_one_across_its_left_plane_and_none_behind_or_beside_it() {
    // At the origin looking along +Y, sixty degrees high and as wide: the left plane leans thirty degrees off the way it looks
    let camera = FlyCamera::default();
    let frustum = Frustum::of(camera.view_proj(1.0));
    assert!(frustum.sees_sphere(Vec3::new(0.0, 10.0, 0.0), 1.0));
    assert!(!frustum.sees_sphere(Vec3::new(0.0, -10.0, 0.0), 1.0));
    let edge = -10.0 * 30f32.to_radians().tan();
    // Two metres beyond the edge is 1.73 from the plane, and one metre is 0.87
    assert!(!frustum.sees_sphere(Vec3::new(edge - 2.0, 10.0, 0.0), 1.0));
    assert!(frustum.sees_sphere(Vec3::new(edge - 1.0, 10.0, 0.0), 1.0));
    assert!(!frustum.sees_sphere(Vec3::new(edge - 1.0, 10.0, 0.0), 0.8));
    // The near and the far plane cut it as well, whichever way the depth runs
    assert!(!frustum.sees_sphere(Vec3::new(0.0, camera.far + 10.0, 0.0), 1.0));
    assert!(frustum.sees_sphere(Vec3::new(0.0, camera.far + 0.5, 0.0), 1.0));
    assert!(frustum.sees_sphere(Vec3::new(0.0, 0.0, 0.0), 1.0));
    // What the planes keep is what the projection keeps: a point inside is within the clip volume
    for point in [
        Vec3::new(3.0, 8.0, -2.0),
        Vec3::new(-9.0, 12.0, 1.0),
        Vec3::new(0.0, 0.01, 0.0),
    ] {
        let clip: Vec4 = camera.view_proj(1.0) * point.extend(1.0);
        let inside =
            clip.x.abs() <= clip.w && clip.y.abs() <= clip.w && (0.0..=clip.w).contains(&clip.z);
        assert_eq!(frustum.sees_sphere(point, 0.0), inside, "{point}");
    }
}

#[test]
fn a_sphere_is_held_only_while_no_plane_cuts_it() {
    let frustum = Frustum::of(FlyCamera::default().view_proj(1.0));
    assert!(frustum.holds_sphere(Vec3::new(0.0, 10.0, 0.0), 1.0));
    let edge = -10.0 * 30f32.to_radians().tan();
    // On the left plane it is seen and not held; behind the camera it is neither
    assert!(frustum.sees_sphere(Vec3::new(edge, 10.0, 0.0), 1.0));
    assert!(!frustum.holds_sphere(Vec3::new(edge, 10.0, 0.0), 1.0));
    assert!(!frustum.holds_sphere(Vec3::new(0.0, -10.0, 0.0), 1.0));
}
