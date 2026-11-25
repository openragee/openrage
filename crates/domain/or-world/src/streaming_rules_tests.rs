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
fn a_move_past_three_hundred_metres_is_a_jump() {
    assert!(!is_focus_jump(Vec3::ZERO, Vec3::new(300.0, 0.0, 0.0)));
    assert!(is_focus_jump(Vec3::ZERO, Vec3::new(300.5, 0.0, 0.0)));
}

#[test]
fn only_a_disabled_group_is_skipped_by_the_search() {
    let disabled = group_state_flag_word(0, 3);
    assert_eq!(disabled, 0x80);
    assert!(search_skips(disabled));
    // States 1 and 2 set 0x180, which is not the skip pattern; state 0 clears 0x80.
    let kept = group_state_flag_word(disabled, 1);
    assert_eq!(kept, 0x180);
    assert!(!search_skips(kept));
    assert_eq!(group_state_flag_word(0x80, 0), 0);
    // 0x180 entering state 3 loses 0x100.
    assert_eq!(group_state_flag_word(0x180, 3), 0x80);
}

#[test]
fn the_lod_distance_resolves_through_the_parent() {
    assert_eq!(resolved_lod_level(0, false), ORPHAN_HD_LEVEL);
    assert_eq!(resolved_lod_level(0, true), 0);
    assert_eq!(resolved_lod_level(2, false), 2);
    assert_eq!(resolved_lod_distance(0.0, 80.0), 80.0);
    assert_eq!(resolved_lod_distance(120.0, 80.0), 120.0);
    // A stated childLodDist of 60 overrides the child's 100 and does not grow.
    let mut stated = LodParent {
        child_lod_distance: 60.0,
        states_child_lod: true,
        stated_children: 0,
        attached_children: 0,
    };
    assert_eq!(attach_child(&mut stated, 100.0), 60.0);
    assert_eq!(stated.child_lod_distance, 60.0);
    // Stated count 0 is raised to the attached 1.
    assert_eq!((stated.stated_children, stated.attached_children), (1, 1));
    // Unstated: the child keeps 100, and the parent's grows from 40 to 100.
    let mut unstated = LodParent {
        child_lod_distance: 40.0,
        states_child_lod: false,
        stated_children: 3,
        attached_children: 0,
    };
    assert_eq!(attach_child(&mut unstated, 100.0), 100.0);
    assert_eq!(unstated.child_lod_distance, 100.0);
    assert_eq!(
        (unstated.stated_children, unstated.attached_children),
        (3, 1)
    );
}

#[test]
fn the_fade_start_blends_toward_the_base_below_scale_one() {
    // base 20, lod 100: scale 0.5 gives 20 + 0.5 * 80 = 60; scale 2 gives 200; scale 1 gives 100.
    assert_eq!(fade_start(100.0, 20.0, 0.5), 60.0);
    assert_eq!(fade_start(100.0, 20.0, 2.0), 200.0);
    assert_eq!(fade_start(100.0, 20.0, 1.0), 100.0);
}

#[test]
fn the_interior_type_raises_the_lodtype_scale_to_one() {
    // 0.5 * 0.5 = 0.25 for an ordinary entity; type 0xC takes max(1, 0.5) = 1, so 0.5.
    assert_eq!(fade_scale(0.5, 0.5, 2), 0.25);
    assert_eq!(fade_scale(0.5, 0.5, INTERIOR_ENTITY_TYPE), 0.5);
}

#[test]
fn only_a_short_id_orphan_hd_takes_five_metres() {
    assert_eq!(fade_range(5, 0x14, 30.0), 5.0);
    assert_eq!(fade_range(5, 0x15, 30.0), 30.0);
    assert_eq!(fade_range(4, 0x00, 30.0), 30.0);
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

#[test]
fn the_parent_cross_fades_only_when_every_stated_child_is_attached() {
    // Shared fade 100: parent 255 - 100 = 155, in progress.
    assert_eq!(
        parent_cross_fade(255, 100, 3, 3, true),
        CrossFade {
            alpha: 155,
            in_progress: true
        }
    );
    // One child entity missing: the parent keeps its distance alpha.
    assert_eq!(
        parent_cross_fade(255, 100, 3, 2, true),
        CrossFade {
            alpha: 255,
            in_progress: false
        }
    );
    // No stated child, or a record alpha under 255: no cross-fade.
    assert_eq!(
        parent_cross_fade(255, 100, 0, 0, true),
        CrossFade {
            alpha: 255,
            in_progress: false
        }
    );
    assert_eq!(
        parent_cross_fade(200, 100, 3, 3, true),
        CrossFade {
            alpha: 200,
            in_progress: false
        }
    );
    // Flags that forbid it: the parent keeps 255 rather than 155.
    assert_eq!(
        parent_cross_fade(255, 100, 3, 3, false),
        CrossFade {
            alpha: 255,
            in_progress: false
        }
    );
    // A shared fade of 0 or 255 gives 255 or 0 and is not in progress.
    assert_eq!(
        parent_cross_fade(255, 0, 3, 3, true),
        CrossFade {
            alpha: 255,
            in_progress: false
        }
    );
    assert_eq!(
        parent_cross_fade(255, 255, 3, 3, true),
        CrossFade {
            alpha: 0,
            in_progress: false
        }
    );
}

#[test]
fn a_child_takes_its_parents_fade_unless_faded_out_or_indexed() {
    assert_eq!(child_alpha(200, Some(100), false), 100);
    assert_eq!(child_alpha(0, Some(100), false), 0);
    assert_eq!(child_alpha(200, Some(0), false), 200);
    assert_eq!(child_alpha(200, Some(100), true), 200);
    assert_eq!(child_alpha(200, None, false), 200);
}

#[test]
fn a_visible_child_with_its_model_missing_pins_the_parent() {
    // (a): record alpha under 255, bit 0 clear.
    assert!(child_pins_parent(true, 200, 200, 0, false, false));
    // Record 255, child 200, bit 0 set: (a) fails on bit 0, (b) needs the level in the mask.
    assert!(!child_pins_parent(true, 200, 255, 0, true, false));
    assert!(child_pins_parent(true, 200, 255, 0, true, true));
    // Phase 6, a faded-out child or a ready model: no pin.
    assert!(!child_pins_parent(true, 200, 200, 6, false, false));
    assert!(!child_pins_parent(true, 0, 200, 0, false, false));
    assert!(!child_pins_parent(false, 200, 200, 0, false, false));
}

#[test]
fn the_band_follows_the_table() {
    assert_eq!(
        request_band(true, true, 0, 0, BAND_INTERIOR_HIGH),
        BAND_VISIBLE
    );
    assert_eq!(
        request_band(true, false, 0, 0, BAND_INTERIOR_HIGH),
        BAND_IN_VIEW_HIDDEN
    );
    // 0x105 has both band-5 bits and the interior bit: band 5 is tested first; level 5 falls through to the interior.
    assert_eq!(
        request_band(false, false, 0x105, 2, BAND_INTERIOR_MIDDLE),
        BAND_LOW_DETAIL
    );
    assert_eq!(
        request_band(false, false, 0x105, 5, BAND_INTERIOR_MIDDLE),
        BAND_INTERIOR_MIDDLE
    );
    // 0x100 alone is not band 5.
    assert_eq!(
        request_band(false, false, 0x100, 2, BAND_INTERIOR_MIDDLE),
        BAND_OTHER
    );
}

#[test]
fn the_priority_is_the_band_plus_a_capped_inverse_distance() {
    // d 2: 1 / 2.000001 = 0.4999998, so 7.5; d 0 or 1: the inverse is 1e6 or 0.999999, capped at 0.95.
    assert!((request_priority(BAND_VISIBLE, 2.0, None) - 7.5).abs() < 1e-5);
    assert_eq!(request_priority(BAND_VISIBLE, 0.0, None), 7.95);
    assert_eq!(request_priority(BAND_VISIBLE, 1.0, None), 7.95);
    // The band dominates: 6.95 at zero distance is under 7.001 at 1000 m.
    assert!(
        request_priority(BAND_IN_VIEW_HIDDEN, 0.0, None)
            < request_priority(BAND_VISIBLE, 1000.0, None)
    );
}

#[test]
fn the_level_bias_adds_to_the_distance_term() {
    // d 2, level 6: 7 + (0.5 + 0.4) = 7.9; level 13 masks to 5: 7 + (0.5 + 0.02) = 7.52.
    assert!((request_priority(BAND_VISIBLE, 2.0, Some(6)) - 7.9).abs() < 1e-5);
    assert!((request_priority(BAND_VISIBLE, 2.0, Some(13)) - 7.52).abs() < 1e-5);
}

#[test]
fn a_negative_priority_is_queued_as_positive_zero() {
    // d -0.5: 1 / -0.499999 = -2.000004, plus band 1 is -1.000004, floored to +0.0.
    let priority = request_priority(BAND_OTHER, -0.5, None);
    assert_eq!(priority.to_bits(), 0);
}

#[test]
fn requests_issue_highest_first_and_ties_by_the_larger_key() {
    // Ascending sort by (priority, entity) is (1, 3, 2), walked from the end: [2, 3, 1].
    // Without the entity key the stable order is (3, 1, 2), reversed [2, 1, 3].
    let queued = [
        ModelRequest {
            entity: 3,
            priority: 2.0,
        },
        ModelRequest {
            entity: 2,
            priority: 7.5,
        },
        ModelRequest {
            entity: 1,
            priority: 2.0,
        },
    ];
    let keys: Vec<u64> = order_requests(&queued)
        .iter()
        .map(|request| request.entity)
        .collect();
    assert_eq!(keys, vec![2, 3, 1]);
}

#[test]
fn a_hand_built_negative_priority_sorts_as_zero() {
    // -1.0's bit pattern is above every positive one; floored to 0 it issues last.
    let queued = [
        ModelRequest {
            entity: 1,
            priority: 7.95,
        },
        ModelRequest {
            entity: 2,
            priority: -1.0,
        },
        ModelRequest {
            entity: 3,
            priority: f32::NAN,
        },
    ];
    let keys: Vec<u64> = order_requests(&queued)
        .iter()
        .map(|request| request.entity)
        .collect();
    assert_eq!(keys, vec![1, 3, 2]);
}

#[test]
fn the_queue_drops_what_comes_past_its_capacity() {
    // 12288 low entries then one high: the high one arrives past capacity and is never issued.
    let mut queued: Vec<ModelRequest> = (0..REQUEST_QUEUE_CAPACITY as u64)
        .map(|entity| ModelRequest {
            entity,
            priority: 1.0,
        })
        .collect();
    queued.push(ModelRequest {
        entity: 99_999,
        priority: 7.95,
    });
    let ordered = order_requests(&queued);
    assert_eq!(ordered.len(), REQUEST_QUEUE_CAPACITY);
    assert!(ordered.iter().all(|request| request.entity != 99_999));
}

#[test]
fn instances_go_in_groups_of_sixty_four() {
    assert_eq!(instance_groups(0), 0);
    assert_eq!(instance_groups(64), 1);
    assert_eq!(instance_groups(65), 2);
}

#[test]
fn the_lod_scale_blends_and_the_scope_overrides_it() {
    // t 0.5 of 2: f 0.25, so 1 + 1 * 0.25; past the duration it is the target.
    assert_eq!(blend_lod_scale(1.0, 2.0, 0.5, 2.0), 1.25);
    assert_eq!(blend_lod_scale(1.0, 2.0, 3.0, 2.0), 2.0);
    // Duration 0 at t 0 snaps without dividing.
    assert_eq!(blend_lod_scale(1.0, 2.0, 0.0, 0.0), 2.0);
    assert_eq!(scope_lod_scales(4.0, true), Some((2.5, 2.0)));
    assert_eq!(scope_lod_scales(3.0, true), Some((1.8, 1.6)));
    assert_eq!(scope_lod_scales(1.5, true), None);
    assert_eq!(scope_lod_scales(4.0, false), None);
}

#[test]
fn the_stand_in_fade_ramps_from_300_to_2000() {
    // (1150 - 300) / 1700 = 0.5; before 300 it is 0, past 2000 it is 1.
    assert_eq!(
        stand_in_fade(100.0, STAND_IN_FADE_START, STAND_IN_FADE_END),
        0.0
    );
    assert!(stand_in_fade(300.0, STAND_IN_FADE_START, STAND_IN_FADE_END).abs() < 1e-6);
    assert!((stand_in_fade(1150.0, STAND_IN_FADE_START, STAND_IN_FADE_END) - 0.5).abs() < 1e-6);
    assert_eq!(
        stand_in_fade(3000.0, STAND_IN_FADE_START, STAND_IN_FADE_END),
        1.0
    );
    // NaN distance: a * NaN + b is NaN, max(0) gives 0; f32::clamp would return NaN.
    assert_eq!(
        stand_in_fade(f32::NAN, STAND_IN_FADE_START, STAND_IN_FADE_END),
        0.0
    );
    // Equal bounds: a = inf, b = -inf, so NaN and 0 past the start as well.
    assert_eq!(stand_in_fade(400.0, 300.0, 300.0), 0.0);
}

#[test]
fn the_second_range_ramps_from_190_and_keeps_a_minimum_span() {
    // (1095 - 190) / 1810 = 0.5.
    assert!(
        (stand_in_second_fade(1095.0, STAND_IN_SECOND_START, STAND_IN_SECOND_END) - 0.5).abs()
            < 1e-6
    );
    assert_eq!(
        stand_in_second_fade(100.0, STAND_IN_SECOND_START, STAND_IN_SECOND_END),
        0.0
    );
    // Start == end: the span is 1e-4, so 100.00005 (about 5.3e-5 past start in f32) is near 0.53, not 1.
    let ramp = stand_in_second_fade(100.000_05, 100.0, 100.0);
    assert!(ramp > 0.4 && ramp < 0.7);
}

#[test]
fn the_imposter_level_steps_by_the_thresholds() {
    // q 0: 128/256 = 0.5 -> 3; 300/256 = 1.17 -> 2; 450/256 = 1.76 -> 1; 600/256 = 2.34 -> base.
    assert_eq!(imposter_level(128, 0, IMPOSTER_THRESHOLDS, 0, 0), 3);
    assert_eq!(imposter_level(300, 0, IMPOSTER_THRESHOLDS, 0, 0), 2);
    assert_eq!(imposter_level(450, 0, IMPOSTER_THRESHOLDS, 0, 0), 1);
    assert_eq!(imposter_level(600, 0, IMPOSTER_THRESHOLDS, 7, 0), 7);
    // q 2 subtracts 1: 2.34 - 1 = 1.34 -> 2; the item's minimum 4 wins.
    assert_eq!(imposter_level(600, 2, IMPOSTER_THRESHOLDS, 0, 0), 2);
    assert_eq!(imposter_level(600, 2, IMPOSTER_THRESHOLDS, 0, 4), 4);
}

#[test]
fn the_hour_range_includes_its_end_and_wraps_by_or() {
    assert!(hour_in_range(18, 6, 18));
    assert!(!hour_in_range(19, 6, 18));
    assert!(!hour_in_range(5, 6, 18));
    // 22..4 wraps: 23, 3 and 4 hold, 5 and 12 do not.
    assert!(hour_in_range(23, 22, 4));
    assert!(hour_in_range(3, 22, 4));
    assert!(hour_in_range(4, 22, 4));
    assert!(!hour_in_range(5, 22, 4));
    assert!(!hour_in_range(12, 22, 4));
    // A == B takes the wrap branch: h <= 7 || h >= 7 always holds.
    assert!(hour_in_range(3, 7, 7));
}

#[test]
fn the_hour_minute_range_wraps_on_hours_alone() {
    // 8:30..17:45: 17:45 holds, 17:46 and 8:29 do not.
    assert!(hour_minute_in_range(17, 45, 8, 30, 17, 45));
    assert!(!hour_minute_in_range(17, 46, 8, 30, 17, 45));
    assert!(!hour_minute_in_range(8, 29, 8, 30, 17, 45));
    // 10:00..10:30 has equal hours, so it wraps: now <= 630 || now >= 600 always holds, 15:00 included.
    assert!(hour_minute_in_range(15, 0, 10, 0, 10, 30));
    // 10:30..10:15 is false only from 10:16 to 10:29.
    assert!(hour_minute_in_range(10, 15, 10, 30, 10, 15));
    assert!(!hour_minute_in_range(10, 16, 10, 30, 10, 15));
    assert!(!hour_minute_in_range(10, 29, 10, 30, 10, 15));
    assert!(hour_minute_in_range(10, 30, 10, 30, 10, 15));
}

#[test]
fn the_day_phase_range_is_half_open() {
    assert!(day_phase_in_range(0.25, 0.25, 0.75));
    assert!(!day_phase_in_range(0.75, 0.25, 0.75));
    // 0.75..0.25 wraps: 0.9 and 0.1 hold, 0.25 and 0.5 do not.
    assert!(day_phase_in_range(0.9, 0.75, 0.25));
    assert!(day_phase_in_range(0.1, 0.75, 0.25));
    assert!(!day_phase_in_range(0.25, 0.75, 0.25));
    assert!(!day_phase_in_range(0.5, 0.75, 0.25));
    // min == max: p < 0.5 || p >= 0.5 always holds.
    assert!(day_phase_in_range(0.3, 0.5, 0.5));
}

#[test]
fn the_day_phase_is_compared_in_f32() {
    // 0.75 - 1e-10 is under 0.75 in f64, but rounds to 0.75 in f32 (its half ulp is about 3e-8), so the end excludes it.
    assert!(!day_phase_in_range(0.75 - 1e-10, 0.25, 0.75));
}

#[test]
fn the_other_hour_family_excludes_its_end() {
    assert!(hour_in_half_open_range(6, 6, 18));
    assert!(hour_in_half_open_range(17, 6, 18));
    assert!(!hour_in_half_open_range(18, 6, 18));
    assert!(hour_in_half_open_range(22, 22, 4));
    assert!(hour_in_half_open_range(3, 22, 4));
    assert!(!hour_in_half_open_range(4, 22, 4));
    assert!(hour_in_half_open_range(12, 9, 9));
}

#[test]
fn a_layer_named_without_an_archive_is_the_last_registered() {
    let records: [(u32, u32); 3] = [(1, 10), (2, 10), (2, 20)];
    assert_eq!(find_layer(&records, 0, 10), Some(1));
    assert_eq!(find_layer(&records, 1, 10), Some(0));
    assert_eq!(find_layer(&records, 3, 10), None);
    assert_eq!(find_layer(&records, 0, 30), None);
}

#[test]
fn a_layer_stays_on_while_any_requester_holds_it() {
    let mut layer = LayerRequesters::default();
    assert!(layer.request(1));
    assert!(!layer.request(2));
    assert!(!layer.release(1));
    assert!(layer.is_on());
    // Releasing an absent requester changes nothing.
    assert!(!layer.release(7));
    assert!(layer.is_on());
    assert!(layer.release(2));
    assert!(!layer.is_on());
}

#[test]
fn a_repeated_request_is_held_once() {
    // Two requests by the same requester are held as one entry, and one release turns the layer off.
    let mut layer = LayerRequesters::default();
    assert!(layer.request(1));
    // A repeat request is not a new turn-on.
    assert!(!layer.request(1));
    assert_eq!(layer.requesters, vec![1]);
    assert!(layer.release(1));
    assert!(!layer.is_on());
}

#[test]
fn force_off_drops_every_requester() {
    let mut layer = LayerRequesters::default();
    layer.request(REGISTRATION_REQUESTER);
    layer.request(2);
    assert!(layer.force_off());
    assert!(!layer.is_on());
    assert!(!layer.force_off());
}

#[test]
fn the_random_k_is_bounded_by_the_list() {
    assert_eq!(
        random_k_range(LAYER_ACTION_MIN_DEFAULT, LAYER_ACTION_MAX_DEFAULT, 5),
        1..=5
    );
    assert_eq!(random_k_range(0, 3, 5), 0..=3);
    assert!(random_list_applies(31));
    assert!(!random_list_applies(32));
}

#[test]
fn the_selector_takes_the_highest_priority_and_the_earliest_tie() {
    let valid: [u32; 4] = [0xA, 0xB, 0xC, 0xD];
    let tie = [
        SelectorRequest {
            requester: 1,
            state: 0xA,
            priority: 5,
        },
        SelectorRequest {
            requester: 2,
            state: 0xB,
            priority: 5,
        },
    ];
    assert_eq!(pick_selector_state(&tie, 0xD, &valid), Some(0xA));
    let higher = [
        SelectorRequest {
            requester: 1,
            state: 0xA,
            priority: 3,
        },
        SelectorRequest {
            requester: 2,
            state: 0xB,
            priority: 9,
        },
    ];
    assert_eq!(pick_selector_state(&higher, 0xD, &valid), Some(0xB));
    // Priority 0 beats the starting -1; a negative one never wins; state 0 asks for the default.
    assert_eq!(
        pick_selector_state(
            &[SelectorRequest {
                requester: 1,
                state: 0xA,
                priority: 0
            }],
            0xD,
            &valid
        ),
        Some(0xA)
    );
    assert_eq!(
        pick_selector_state(
            &[SelectorRequest {
                requester: 1,
                state: 0xA,
                priority: -1
            }],
            0xD,
            &valid
        ),
        Some(0xD)
    );
    assert_eq!(
        pick_selector_state(
            &[SelectorRequest {
                requester: 1,
                state: 0,
                priority: 10
            }],
            0xD,
            &valid
        ),
        Some(0xD)
    );
    assert_eq!(pick_selector_state(&[], 0xD, &valid), Some(0xD));
}

#[test]
fn a_state_outside_the_class_set_drops_the_request() {
    // 0xB wins on priority 9 but the set is {0xA, 0xD}: dropped, not Some(0xB).
    let higher = [
        SelectorRequest {
            requester: 1,
            state: 0xA,
            priority: 3,
        },
        SelectorRequest {
            requester: 2,
            state: 0xB,
            priority: 9,
        },
    ];
    assert_eq!(pick_selector_state(&higher, 0xD, &[0xA, 0xD]), None);
}

#[test]
fn the_class_defaults_are_valid_states() {
    assert_eq!(class_a_default(true), 0x9C78_3318);
    assert_eq!(class_a_default(false), 0x005F_46C7);
    assert!(CLASS_A_FIXED_STATES.contains(&class_a_default(true)));
    assert!(CLASS_A_FIXED_STATES.contains(&class_a_default(false)));
    assert!(CLASS_D_STATES.contains(&CLASS_D_DEFAULT));
}

#[test]
fn a_repeated_requester_keeps_its_place() {
    // Requester 1 asks again for 0xC at the tied priority: overwritten in place it is still first, so 0xC wins;
    // appended, the old 0xA entry would win the tie.
    let mut requests: Vec<SelectorRequest> = Vec::new();
    add_selector_request(
        &mut requests,
        SelectorRequest {
            requester: 1,
            state: 0xA,
            priority: 5,
        },
    );
    add_selector_request(
        &mut requests,
        SelectorRequest {
            requester: 2,
            state: 0xB,
            priority: 5,
        },
    );
    add_selector_request(
        &mut requests,
        SelectorRequest {
            requester: 1,
            state: 0xC,
            priority: 5,
        },
    );
    assert_eq!(requests.len(), 2);
    assert_eq!(
        pick_selector_state(&requests, 0xD, &[0xA, 0xB, 0xC, 0xD]),
        Some(0xC)
    );
}

#[test]
fn only_a_top_priority_add_or_an_erasing_remove_re_evaluates() {
    let mut requests: Vec<SelectorRequest> = Vec::new();
    // An empty list: the new priority is at least every existing one.
    assert!(add_selector_request(
        &mut requests,
        SelectorRequest {
            requester: 1,
            state: 0xA,
            priority: 5
        }
    ));
    assert!(!add_selector_request(
        &mut requests,
        SelectorRequest {
            requester: 2,
            state: 0xB,
            priority: 3
        }
    ));
    // Lowering the winner from 5 to 2 is compared against its own old 5, so no re-evaluation.
    assert!(!add_selector_request(
        &mut requests,
        SelectorRequest {
            requester: 1,
            state: 0xA,
            priority: 2
        }
    ));
    assert!(remove_selector_requests(
        &mut requests,
        |held: &SelectorRequest| held.requester == 2
    ));
    assert!(!remove_selector_requests(
        &mut requests,
        |held: &SelectorRequest| held.requester == 9
    ));
    assert_eq!(requests.len(), 1);
}

#[test]
fn a_portal_to_room_zero_without_a_link_leads_outside() {
    assert!(portal_leads_outside(0, 3, 0));
    assert!(portal_leads_outside(3, 0, PORTAL_MIRROR));
    assert!(!portal_leads_outside(0, 3, PORTAL_LINK));
    assert!(!portal_leads_outside(1, 3, 0));
}

#[test]
fn the_opacity_is_a_percentage_capped_at_one() {
    assert_eq!(portal_opacity(100), 1.0);
    assert_eq!(portal_opacity(150), 1.0);
    assert_eq!(portal_opacity(0), 0.0);
    assert!((portal_opacity(50) - 0.5).abs() < 1e-6);
}

#[test]
fn the_camera_factor_is_the_nearest_portal_over_twenty_five() {
    assert_eq!(camera_portal_factor(true, [10.0]), 0.0);
    assert_eq!(camera_portal_factor(false, [0.0_f32; 0]), 1.0);
    // 30 is out of reach; min(10, 20) * 0.04 = 0.4.
    let factor = camera_portal_factor(false, [30.0, 10.0, 20.0]);
    assert!((factor - 0.4).abs() < 1e-6);
    assert!(!uses_interior_ambient(factor));
    // 1 m: 0.04 is under 0.05.
    assert!(uses_interior_ambient(camera_portal_factor(false, [1.0])));
}

#[test]
fn a_closable_portal_shuts_only_with_every_door_shut() {
    let shut = PortalDoor {
        open_ratio: 0.0,
        keeps_open: false,
    };
    assert!(portal_is_shut(PORTAL_ALLOW_CLOSING, true, &[shut]));
    // No door placed, no allow-closing bit, or a proxy state byte of 0: open.
    assert!(!portal_is_shut(PORTAL_ALLOW_CLOSING, true, &[]));
    assert!(!portal_is_shut(0, true, &[shut]));
    assert!(!portal_is_shut(PORTAL_ALLOW_CLOSING, false, &[shut]));
    // 0.0049 is under 0.005; 0.005 itself and -0.01 count as open.
    assert!(portal_is_shut(
        PORTAL_ALLOW_CLOSING,
        true,
        &[PortalDoor {
            open_ratio: 0.0049,
            keeps_open: false
        }]
    ));
    assert!(!portal_is_shut(
        PORTAL_ALLOW_CLOSING,
        true,
        &[PortalDoor {
            open_ratio: 0.005,
            keeps_open: false
        }]
    ));
    assert!(!portal_is_shut(
        PORTAL_ALLOW_CLOSING,
        true,
        &[PortalDoor {
            open_ratio: -0.01,
            keeps_open: false
        }]
    ));
    // One open door of two, or one keep-open door: open.
    assert!(!portal_is_shut(
        PORTAL_ALLOW_CLOSING,
        true,
        &[
            shut,
            PortalDoor {
                open_ratio: 0.5,
                keeps_open: false
            }
        ]
    ));
    assert!(!portal_is_shut(
        PORTAL_ALLOW_CLOSING,
        true,
        &[
            shut,
            PortalDoor {
                open_ratio: 0.0,
                keeps_open: true
            }
        ]
    ));
}

#[test]
fn a_door_keeps_its_portal_open_by_its_flags_or_its_archetype() {
    // 0x80000 alone matches the mask; 0x280000 masks to 0x280000, not 0x80000.
    assert!(door_keeps_portal_open(0x8_0000, 0));
    assert!(!door_keeps_portal_open(0x28_0000, 0));
    assert!(door_keeps_portal_open(0x40_0000, 0));
    assert!(door_keeps_portal_open(0, 1));
    assert!(!door_keeps_portal_open(0, 0));
}
