use super::*;

fn light(kind: LightKind, x: f32, hours: u32) -> Light {
    Light {
        kind,
        position: [x, 0.0, 0.0],
        direction: [0.0, 0.0, -1.0],
        colour: [1.0, 0.5, 0.0],
        intensity: 10.0,
        radius: 5.0,
        falloff_exponent: 8.0,
        hours,
        flags: 0,
        cone_inner: 20.0,
        cone_outer: 40.0,
        length: 4.0,
        corona: [0.0; 3],
    }
}

fn placed(kind: LightKind, x: f32, hours: u32) -> Lamp {
    Lamp {
        light: light(kind, x, hours),
        reach: None,
    }
}

#[test]
fn a_light_comes_on_and_goes_off_over_the_last_minute_of_an_hour() {
    let night = 0x00E0_007F; // 21:00 to 06:59
    assert_eq!(hour_strength(night, 3.5), 1.0);
    assert_eq!(hour_strength(night, 12.0), 0.0);
    assert_eq!(hour_strength(night, 6.5), 1.0, "on through its last hour");
    assert!((hour_strength(night, 6.0 + 59.5 / 60.0) - 0.5).abs() < 1e-3);
    assert_eq!(hour_strength(night, 7.0), 0.0);
    assert!((hour_strength(night, 20.0 + 59.25 / 60.0) - 0.25).abs() < 1e-3);
    assert_eq!(
        hour_strength(night, 20.5),
        0.0,
        "off until the last minute before"
    );
    assert_eq!(hour_strength(night, 24.0), 1.0, "midnight is hour 0");
    assert_eq!(hour_strength(ALL_HOURS, 23.999), 1.0);
}

/// What the point volumes of `lights` give at `d` metres from `at`, as the pass fades each.
fn point_light_at(lights: &FrameLights, at: f32, d: f32) -> f32 {
    (lights.points.iter())
        .filter(|p| p.position.x == at)
        .map(|p| {
            p.intensity
                * (1.0 - d * d / (p.radius * p.radius))
                    .max(0.0)
                    .powf(p.falloff)
        })
        .sum()
}

#[test]
fn where_the_placed_lights_stand_is_one_value_whatever_order_they_come_in() {
    let a = placed(LightKind::Point, 1.0, ALL_HOURS);
    let mut b = a;
    b.light.position[2] = 0.001;
    let c = placed(LightKind::Spot, 9.0, ALL_HOURS);
    assert_eq!(Placed::of(&[a, b, c]), Placed::of(&[c, b, a]));
    assert_ne!(Placed::of(&[a, c]), Placed::of(&[a]));
}

#[test]
fn the_nearest_by_the_edge_of_their_reach_are_kept_however_bright_a_far_one_is() {
    let mut far = placed(LightKind::Point, 60.0, ALL_HOURS);
    far.light.intensity = 10_000.0;
    let mut wide = placed(LightKind::Point, 40.0, ALL_HOURS);
    wide.light.radius = 38.0;
    let dim = placed(LightKind::Point, 10.0, ALL_HOURS);
    let one = frame_lights(&[far, dim], Vec3::ZERO, 12.0, 1.0, 1);
    assert_eq!(one.kept, [(Vec3::new(10.0, 0.0, 0.0), 1.0)]);
    let one = frame_lights(&[dim, wide], Vec3::ZERO, 12.0, 1.0, 1);
    assert_eq!(
        one.points[0].position.x, 40.0,
        "its reach comes to 2 m of the camera"
    );
}

#[test]
fn the_nearest_are_kept_each_in_the_pass_of_its_kind() {
    let lamps = [
        placed(LightKind::Point, 100.0, ALL_HOURS),
        placed(LightKind::Spot, 10.0, ALL_HOURS),
        placed(LightKind::Capsule, 20.0, ALL_HOURS),
        placed(LightKind::Point, 15.0, 1 << 3),
    ];
    let shells = SHELLS.len();
    let two = frame_lights(&lamps, Vec3::ZERO, 12.0, 0.5, 2);
    assert_eq!(two.lights, 2);
    assert!(
        two.points.is_empty(),
        "the far point loses to the two near ones"
    );
    assert_eq!(two.spots.len(), 2 * shells, "two cones a shell");
    assert_eq!(two.capsules.len(), shells);
    let spot = two.spots[0];
    assert_eq!(
        (spot.position.x, spot.radius, spot.direction),
        (10.0, 5.0, Vec3::NEG_Z)
    );
    assert_eq!(
        (spot.outer_degrees, spot.inner_degrees, spot.falloff),
        (40.0, 20.0, SHELL_EXPONENT)
    );
    let capsule = two.capsules[0];
    assert_eq!(
        (capsule.position.x, capsule.length, capsule.radius),
        (20.0, 4.0, 5.0)
    );
    // What blends is lit by a capsule from its segment, half of it each way of its place
    let tube = (two.lamps.iter()).find(|l| l.position.x == 20.0);
    assert_eq!(
        tube.map(|l| (l.direction, l.cone[1] < 0.0)),
        Some((Vec3::new(0.0, 0.0, -2.0), true))
    );
    let all = frame_lights(&lamps, Vec3::ZERO, 3.5, 1.0, 8);
    assert_eq!(all.lights, 4, "the hour-3 light is lit at 03:30");
    assert_eq!(all.points[0].position.x, 15.0, "the nearest point first");
    let none = frame_lights(&lamps, Vec3::ZERO, 3.5, 1.0, 0);
    assert_eq!(none, FrameLights::default());
}

#[test]
fn a_light_falls_as_its_intensity_times_the_window_over_the_distance_squared() {
    // Intensity 10, radius 5, scaled by 0.5: E = 5 · (1 − (d²/25)²)² / d².
    let lights = frame_lights(
        &[placed(LightKind::Point, 15.0, ALL_HOURS)],
        Vec3::ZERO,
        12.0,
        0.5,
        8,
    );
    for d in [0.5f32, 1.0, 2.0, 3.0, 4.0] {
        let x = d * d / 25.0;
        let law = 5.0 * (1.0 - x * x).powi(2) / (d * d);
        let drawn = point_light_at(&lights, 15.0, d);
        assert!(
            (drawn - law).abs() / (law + 0.02 * 5.0 / 25.0) < 0.17,
            "{d}: {drawn} {law}"
        );
    }
    assert_eq!(point_light_at(&lights, 15.0, 5.0), 0.0);
}

#[test]
fn a_level_of_detail_light_stands_at_its_parents_distant_light_and_fades_out_at_its_reach() {
    let lod = MapLod {
        direction: vec![[0.0, 0.0, -1.0]; 2],
        falloff: vec![3.0, 12.0],
        falloff_exponent: vec![1.0; 2],
        cone_inner: vec![51, 0],
        cone_outer: vec![102, 255],
        ..MapLod::default()
    };
    let parent = MapLod {
        distant: vec![
            Distant {
                position: [100.0, 0.0, 0.0],
                rgb: [255, 0, 51],
                intensity: 80.0,
                light_type: 2,
            },
            Distant {
                position: [200.0, 0.0, 0.0],
                rgb: [255, 255, 255],
                intensity: 40.0,
                light_type: 4,
            },
        ],
        category: 1,
        ..MapLod::default()
    };
    let reach = LodReach {
        range: [500.0, 1500.0, 3500.0],
        fadeout: [100.0, 100.0, 150.0],
    };
    let lamps = lod_lamps(&lod, &parent, &reach);
    assert_eq!(lamps.len(), 2);
    let spot = lamps[0].light;
    assert_eq!(
        (spot.kind, spot.position, spot.radius),
        (LightKind::Spot, [100.0, 0.0, 0.0], 3.0)
    );
    assert!((spot.cone_inner - 36.0).abs() < 1e-4 && (spot.cone_outer - 72.0).abs() < 1e-4);
    assert!((Vec3::from(spot.colour) - Vec3::new(1.0, 0.0, 0.2)).length() < 1e-6);
    assert_eq!(lamps[0].reach, Some([1500.0, 100.0]), "the medium class");
    let capsule = lamps[1].light;
    assert_eq!((capsule.kind, capsule.length), (LightKind::Capsule, 140.0));
    // A medium light 1,450 m off is half faded; 1,500 m off, gone; and lit at noon.
    let whole = frame_lights(&lamps[..1], Vec3::new(1300.0, 0.0, 0.0), 12.0, 1.0, 8);
    let at_half = frame_lights(&lamps[..1], Vec3::new(1550.0, 0.0, 0.0), 12.0, 1.0, 8);
    let ratio = at_half.spots[0].intensity / whole.spots[0].intensity;
    assert!((ratio - 0.5).abs() < 1e-3, "{ratio}");
    let gone = frame_lights(&lamps[..1], Vec3::new(1600.0, 0.0, 0.0), 12.0, 1.0, 8);
    assert_eq!(gone.lights, 0);

    let short = MapLod {
        falloff: vec![3.0],
        ..lod.clone()
    };
    assert!(
        lod_lamps(&short, &parent, &reach).is_empty(),
        "lists that do not pair"
    );
    assert!(
        lod_lamps(&lod, &parent, &LodReach::default()).is_empty(),
        "no settings, no reach"
    );
    let odd = MapLod {
        category: 3,
        ..parent.clone()
    };
    assert!(lod_lamps(&lod, &odd, &reach).is_empty());
}

#[test]
fn a_level_of_detail_light_with_a_placed_light_in_its_place_gives_way_to_it() {
    let lod = |x: f32| Lamp {
        light: light(LightKind::Point, x, ALL_HOURS),
        reach: Some([500.0, 100.0]),
    };
    let near = [placed(LightKind::Point, 10.015, 1)];
    let kept = without_placed(vec![lod(10.0), lod(10.05), lod(-0.001)], &near);
    let xs: Vec<f32> = kept.iter().map(|l| l.light.position[0]).collect();
    assert_eq!(xs, [10.05, -0.001]);
}

#[test]
fn a_list_is_passed_over_only_where_none_of_its_lamps_would_light() {
    let lod = |x: f32| Lamp {
        light: light(LightKind::Point, x, ALL_HOURS),
        reach: Some([500.0, 100.0]),
    };
    let list = LodList::of(vec![lod(0.0), lod(20.0)]);
    let at = |x: f32| Vec3::new(x, 0.0, 0.0);
    assert!(list.reaches(at(510.0)) && list.reaches(at(-480.0)));
    assert!(frame_lights(&list.lamps, at(519.0), 12.0, 1.0, 8).lights > 0);
    // Past the reach of the nearer lamp, with a metre to spare, nothing of it would light.
    assert!(!list.reaches(at(521.5)));
    assert_eq!(frame_lights(&list.lamps, at(521.5), 12.0, 1.0, 8).lights, 0);
    assert!(!LodList::of(Vec::new()).reaches(Vec3::ZERO));
}

#[test]
fn the_reach_of_each_size_class_is_read_from_the_visual_settings() {
    let settings = VisualSettings::parse(
        "lodlight.small.range 10\nlodlight.medium.range 20\nlodlight.large.range 30\n\
         lodlight.small.fadeout 1\nlodlight.medium.fadeout 2\nlodlight.large.fadeout 3\n",
    );
    let reach = LodReach::of(&settings);
    assert_eq!(reach.range, [10.0, 20.0, 30.0]);
    assert_eq!(reach.fadeout, [1.0, 2.0, 3.0]);
    assert_eq!(
        LodReach::of(&VisualSettings::parse("")),
        LodReach::default()
    );
}

#[test]
fn a_light_that_states_no_exponent_reaches_the_frame_with_one_above_nought() {
    let mut lamps = [
        placed(LightKind::Point, 1.0, ALL_HOURS),
        placed(LightKind::Spot, 2.0, ALL_HOURS),
        placed(LightKind::Capsule, 3.0, ALL_HOURS),
    ];
    for lamp in &mut lamps {
        lamp.light.falloff_exponent = 0.0;
    }
    let frame = frame_lights(&lamps, Vec3::ZERO, 12.0, 1.0, 8);
    assert!(!frame.points.is_empty() && !frame.spots.is_empty() && !frame.capsules.is_empty());
    let falloffs = (frame.points.iter().map(|p| p.falloff))
        .chain(frame.spots.iter().map(|s| s.falloff))
        .chain(frame.capsules.iter().map(|c| c.falloff));
    for falloff in falloffs {
        assert!(falloff > 0.0);
    }
}

#[test]
fn a_lamp_the_camera_cannot_see_is_not_chosen_and_one_it_can_takes_its_place() {
    let lamps = [
        placed(LightKind::Point, -10.0, ALL_HOURS),
        placed(LightKind::Point, 30.0, ALL_HOURS),
        placed(LightKind::Point, 60.0, ALL_HOURS),
    ];
    let ahead = |p: Vec3, radius: f32| p.x + radius > 0.0;
    let one = choose(&lamps, Vec3::ZERO, 12.0, 1, &ahead);
    assert_eq!(one.len(), 1);
    assert_eq!(
        one[0].0.position[0], 30.0,
        "the nearer lamp behind lights nothing seen"
    );
    let all = choose(&lamps, Vec3::ZERO, 12.0, 8, &|_, _| true);
    assert_eq!(all[0].0.position[0], -10.0);
}

/// A straight ramp across the cosine, from the outer half angle (0) to the inner (1), as the spot pass fades a cone.
fn ramp(c: f32, outer: f32, inner: f32) -> f32 {
    let (o, i) = (outer.to_radians().cos(), inner.to_radians().cos());
    ((c - o) / (i - o)).clamp(0.0, 1.0)
}

/// What the pieces of a cone give together at the cosine `c` from the light's axis.
fn pieces_at(pieces: &[(f32, f32, f32, f32)], c: f32) -> f32 {
    (pieces.iter())
        .map(|&(way, outer, inner, share)| match outer >= 180.0 {
            true => share,
            false => share * ramp(way * c, outer.min(89.0), inner),
        })
        .sum()
}

#[test]
fn a_cone_wider_than_a_right_angle_is_a_point_less_the_cone_behind_it() {
    assert_eq!(cones_within(20.0, 60.0), [(1.0, 60.0, 20.0, 1.0)]);
    for (inner, outer) in [(40.0f32, 120.0f32), (100.0, 150.0), (10.0, 95.0)] {
        let pieces = cones_within(inner, outer);
        // Away from the degree either side of a right angle that the pass cannot reach.
        for degrees in [0.0f32, 30.0, 60.0, 80.0, 100.0, 120.0, 140.0, 170.0, 180.0] {
            let c = degrees.to_radians().cos();
            let (got, wanted) = (pieces_at(&pieces, c), ramp(c, outer, inner));
            assert!(
                (got - wanted).abs() < 0.03,
                "{inner} {outer} at {degrees}: {got} {wanted}"
            );
        }
    }
}

#[test]
fn an_area_light_shines_from_one_side_as_the_cosine_from_its_axis() {
    let mut area = placed(LightKind::Area, 10.0, ALL_HOURS);
    area.light.direction = [0.0, 0.0, -1.0];
    let frame = frame_lights(&[area], Vec3::ZERO, 12.0, 1.0, 8);
    assert!(frame.points.is_empty() && !frame.spots.is_empty());
    let spot = frame.spots[0];
    assert_eq!(
        (spot.direction, spot.outer_degrees, spot.inner_degrees),
        (Vec3::NEG_Z, 89.0, 0.0)
    );
    let pieces = cones_within(0.0, 90.0);
    for degrees in [0.0f32, 45.0, 60.0, 85.0] {
        let c = degrees.to_radians().cos();
        assert!((pieces_at(&pieces, c) - c).abs() < 0.03, "{degrees}");
    }
}

#[test]
fn a_light_that_only_draws_a_corona_lights_nothing() {
    let mut corona = placed(LightKind::Point, 5.0, ALL_HOURS);
    corona.light.flags = or_io::gta6::map::light_ext::CORONA_ONLY;
    assert_eq!(frame_lights(&[corona], Vec3::ZERO, 12.0, 1.0, 8).lights, 0);
}
