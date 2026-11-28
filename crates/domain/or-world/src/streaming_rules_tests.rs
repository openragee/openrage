use super::*;

#[test]
fn the_sphere_is_the_smallest_half_extent_plus_fifty() {
    // Box (0,0,0)..(10,4,20): half (5,2,10), smallest 2, radius 52, centre (5,2,10).
    let sphere = map_streaming_sphere(Vec3::ZERO, Vec3::new(10.0, 4.0, 20.0)).unwrap();
    assert_eq!(sphere.centre, Vec3::new(5.0, 2.0, 10.0));
    assert_eq!(sphere.radius, 52.0);
    // 52 above the centre is on the sphere; 52.5 is outside (a largest-extent radius of 60 would hold it).
    assert!(sphere.contains(Vec3::new(5.0, 2.0, 62.0)));
    assert!(!sphere.contains(Vec3::new(5.0, 2.0, 62.5)));
}

#[test]
fn a_box_thinner_than_two_metres_has_no_sphere() {
    // Half extent 0.9 on y is under 1; exactly 1.0 still has a sphere of radius 51.
    assert!(map_streaming_sphere(Vec3::ZERO, Vec3::new(10.0, 1.8, 10.0)).is_none());
    let sphere = map_streaming_sphere(Vec3::ZERO, Vec3::new(10.0, 2.0, 10.0)).unwrap();
    assert_eq!(sphere.radius, 51.0);
}

#[test]
fn a_malformed_box_has_no_sphere() {
    // Half (NaN, 5, 5): f32::min alone would give 5 and a sphere of radius 55.
    assert!(map_streaming_sphere(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::splat(10.0)).is_none());
    assert!(
        map_streaming_sphere(Vec3::new(f32::NEG_INFINITY, 0.0, 0.0), Vec3::splat(10.0)).is_none()
    );
}

#[test]
fn the_alpha_fades_out_beyond_the_start() {
    // L 100, R 10: at or before 100 the alpha is 255 + 0.0255 -> 255; at 110 or beyond 0.0255 -> 0;
    // at 105, 5 * 255 / 10 = 127.5, plus 0.0255 truncates to 127.
    assert_eq!(distance_alpha(50.0, 100.0, 10.0), 255);
    assert_eq!(distance_alpha(100.0, 100.0, 10.0), 255);
    assert_eq!(distance_alpha(105.0, 100.0, 10.0), 127);
    assert_eq!(distance_alpha(110.0, 100.0, 10.0), 0);
    assert_eq!(distance_alpha(500.0, 100.0, 10.0), 0);
    // Range 0: 0 * 255 / 0 is NaN, which truncates to 0 even before the start.
    assert_eq!(distance_alpha(50.0, 100.0, 0.0), 0);
}

#[test]
fn the_bias_lifts_an_alpha_just_under_one() {
    // L 0, R 255, d 254.02: (255 - 254.02) * 255 / 255 = 0.98, which truncates to 0 alone;
    // plus 0.0255 it is 1.0055, so 1.
    assert_eq!(distance_alpha(254.02, 0.0, 255.0), 1);
}
