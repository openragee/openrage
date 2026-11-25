use super::*;
use crate::gta6_day_frame;

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

fn entry(weather: u32, time: f32) -> DayEntry {
    DayEntry { weather, time }
}

/// Records 0: (1 at 0.52, 4 at 0.9); 1: (2 at 0.1); 2: (1 at 0.30, 3 at 0.6).
fn records() -> Vec<DayRecord> {
    vec![
        DayRecord {
            entries: vec![entry(1, 0.52), entry(4, 0.9)],
        },
        DayRecord {
            entries: vec![entry(2, 0.1)],
        },
        DayRecord {
            entries: vec![entry(1, 0.30), entry(3, 0.6)],
        },
    ]
}

#[test]
fn a_cycle_index_past_63_reads_cycle_0() {
    assert_eq!(cycle_slot(63), 63);
    assert_eq!(cycle_slot(64), 0);
    assert_eq!(cycle_slot(u32::MAX), 0);
}

#[test]
fn a_change_starts_at_weight_0_and_accumulates_dt() {
    let mut change = AutoTransition::start(2, 5, 60.0);
    assert_eq!((change.weight, change.elapsed), (0.0, 0.0));
    // 0 + 30 = 30; max(0, min(30/60, 1)) = 0.5.
    assert_close(change.elapsed_after(30.0), 30.0, 0.0);
    change.advance(30.0);
    assert_close(change.elapsed, 30.0, 0.0);
    assert_close(change.weight, 0.5, 0.0);
    assert!(!change.finished());
}

#[test]
fn a_finished_change_no_longer_advances() {
    let mut change = AutoTransition::start(2, 5, 60.0);
    // min(60/60, 1) = 1.
    change.advance(60.0);
    assert!(change.finished());
    // Elapsed stays 60, not 60 + 10 = 70, and the weight stays 1.
    change.advance(10.0);
    assert_close(change.elapsed, 60.0, 0.0);
    assert_close(change.weight, 1.0, 0.0);
}

#[test]
fn a_negative_or_nan_duration_is_0() {
    // -1 and NaN become 0, so the step gives min(dt/0 = inf, 1) = 1 at once; -1 kept
    // would give max(0, dt/-1) = 0 for ever.
    assert_close(AutoTransition::start(2, 5, -1.0).duration, 0.0, 0.0);
    assert_close(AutoTransition::start(2, 5, f32::NAN).duration, 0.0, 0.0);
    assert_close(AutoTransition::start(2, 5, 0.0).duration, 0.0, 0.0);
    let mut change = AutoTransition::start(2, 5, -1.0);
    change.advance(1.0);
    assert!(change.finished());
    // -1 s would be -1000 ms; 0 ms gives left/total = -1000/0 = -inf < 0 after dt 1, so weight 1.
    let queued = QueuedWeather::new(7, -1.0);
    assert_close(queued.total_ms, 0.0, 0.0);
    assert_close(queued.left_ms, 0.0, 0.0);
    assert_eq!(queued.advance(1.0), QueueStep::Commit(7));
}

#[test]
fn a_weather_past_the_count_keeps_the_held_one() {
    let kept = BlendInputs {
        prev: 1,
        next: 3,
        weight: 0.0,
    };
    let mut change = AutoTransition::start(2, 5, 60.0);
    // max(0, min(15/60, 1)) = 0.25.
    change.advance(15.0);
    // Count 4: 2 < 4 is taken, 5 is not, so next stays the held 3.
    assert_eq!(
        change.blend_inputs(4, kept),
        BlendInputs {
            prev: 2,
            next: 3,
            weight: 0.25
        }
    );
    // Count 5: 5 < 5 is false, so the held 3; `<=` would give 5.
    assert_eq!(change.blend_inputs(5, kept).next, 3);
    assert_eq!(change.blend_inputs(6, kept).next, 5);
    // Count 2: 2 < 2 is false, so prev stays the held 1.
    assert_eq!(change.blend_inputs(2, kept).prev, 1);
}

#[test]
fn an_unscheduled_end_starts_a_change_from_to() {
    let mut change = AutoTransition::start(2, 5, 60.0);
    change.advance(60.0);
    assert_eq!(change.follow_on(true, 7, 60.0), None);
    // From 5 (the old to), to 7, back at weight 0 and elapsed 0.
    assert_eq!(
        change.follow_on(false, 7, 60.0),
        Some(AutoTransition {
            from: 5,
            to: 7,
            weight: 0.0,
            duration: 60.0,
            elapsed: 0.0
        })
    );
}

#[test]
fn a_queued_weather_counts_milliseconds_down() {
    let queued = QueuedWeather::new(7, QUEUED_CALLER_DURATION);
    // 45 * 1000 = 45000 ms each, weight 0.
    assert_close(queued.left_ms, 45000.0, 0.0);
    assert_close(queued.total_ms, 45000.0, 0.0);
    assert_close(queued.weight, 0.0, 0.0);
    // 45000 - 1000 * 1.5 = 43500.
    assert_close(queued.left_after(1.5), 43500.0, 0.0);
    // 1 - min(1, 43500/45000) = 1 - 0.966667 = 0.033333.
    match queued.advance(1.5) {
        QueueStep::Running(next) => {
            assert_close(next.left_ms, 43500.0, 0.0);
            assert_close(next.weight, 0.033333, 1e-6);
            assert_close(next.total_ms, 45000.0, 0.0);
            assert_eq!(next.weather, 7);
        }
        QueueStep::Commit(weather) => panic!("committed {weather} below weight 1"),
    }
}

#[test]
fn a_queued_weather_at_weight_1_becomes_the_base() {
    let queued = QueuedWeather::new(7, 45.0);
    // 45000 - 45000 = 0 left: 1 - min(1, 0/45000) = 1.
    assert_eq!(queued.advance(45.0), QueueStep::Commit(7));
}

#[test]
fn a_weather_slot_orders_by_base_then_slot() {
    // 2 * 1000 + 250 + 3 = 2253; 0 * 1000 + 250 + 7 = 257.
    let m = weather_slot_modifier(2, 3);
    assert_eq!((m.category, m.priority), (2, 253));
    assert_eq!(
        gta6_day_frame::modifier_order_key(m.category, m.priority),
        2253
    );
    let first = weather_slot_modifier(0, 7);
    assert_eq!(
        gta6_day_frame::modifier_order_key(first.category, first.priority),
        257
    );
}

#[test]
fn a_slot_joins_the_second_queue_on_either_modifier() {
    let plain = SlotModifier {
        field_0x20: 0,
        flags_0x48: 0,
    };
    let flagged = SlotModifier {
        field_0x20: 0,
        flags_0x48: 0x80,
    };
    let counted = SlotModifier {
        field_0x20: 1,
        flags_0x48: 0,
    };
    // Bits 0..6 (0x7f) and bit 8 (0x100) are not bit 7.
    let other_bits = SlotModifier {
        field_0x20: 0,
        flags_0x48: 0x17f,
    };
    assert!(!slot_in_second_queue(Some(plain), None));
    assert!(!slot_in_second_queue(None, None));
    assert!(!slot_in_second_queue(Some(other_bits), Some(plain)));
    assert!(slot_in_second_queue(Some(flagged), None));
    assert!(slot_in_second_queue(Some(plain), Some(counted)));
    assert!(slot_in_second_queue(None, Some(flagged)));
}

#[test]
fn the_generator_multiplies_with_carry() {
    // x = 1: 1 * 0x5cdcfaa7 + 0; its low 15 bits 0x7aa7 = 31399, 31399 / 32768 = 0.958221435546875.
    assert_eq!(mwc_next(1), 0x5cdc_faa7);
    assert_close(mwc_unit(mwc_next(1)), 31_399.0 / 32_768.0, 0.0);
    // x = 2^32: low 0, carry 1, so 1.
    assert_eq!(mwc_next(1 << 32), 1);
    // x = 2^64 - 1: 0xffffffff * 0x5cdcfaa7 = 0x5cdcfaa6_a3230559, + 0xffffffff = 0x5cdcfaa7_a3230558.
    assert_eq!(mwc_next(u64::MAX), 0x5cdc_faa7_a323_0558);
    // Low 15 bits 0x0558 = 1368, 1368 / 32768 = 0.041748046875.
    assert_close(mwc_unit(mwc_next(u64::MAX)), 1_368.0 / 32_768.0, 0.0);
    // Bit 15 is not taken: 0x8000 gives 0, 0x7fff gives 32767 / 32768.
    assert_close(mwc_unit(0x8000), 0.0, 0.0);
    assert_close(mwc_unit(0x7fff), 1.0 - MWC_UNIT_SCALE, 0.0);
}

#[test]
fn the_next_weather_is_two_truncated_draws() {
    let list = [2u32, 0];
    let records = records();
    // int(0.6 * 2) = 1 -> list[1] = record 0; int(0.5 * 2) = 1 -> entry 1, weather 4.
    assert_eq!(pick_next_weather(&list, &records, 0.6, 0.5), Some(4));
    // int(0.4 * 2) = 0 -> record 2; int(0.99 * 2) = 1 -> weather 3.
    assert_eq!(pick_next_weather(&list, &records, 0.4, 0.99), Some(3));
    assert_eq!(pick_next_weather(&[], &records, 0.5, 0.5), None);
    // Record index 9 is past the three records.
    assert_eq!(pick_next_weather(&[9], &records, 0.5, 0.5), None);
}

#[test]
fn a_draw_outside_0_1_picks_nothing() {
    let list = [2u32, 0];
    let records = records();
    // Unguarded, -0.1 * 2 = -0.2 casts to 0 -> record 2, int(0.5 * 2) = 1 -> weather 3.
    assert_eq!(pick_next_weather(&list, &records, -0.1, 0.5), None);
    // Unguarded, NaN casts to 0 -> record 2, then weather 3.
    assert_eq!(pick_next_weather(&list, &records, f32::NAN, 0.5), None);
    // Unguarded, r2 NaN casts to 0 -> record 2's entry 0, weather 1.
    assert_eq!(pick_next_weather(&list, &records, 0.4, f32::NAN), None);
    // 1 * 2 = 2 is past the list.
    assert_eq!(pick_next_weather(&list, &records, 1.0, 0.5), None);
}

#[test]
fn the_schedule_walk_starts_at_day_mod_count() {
    let list = [0u32, 1, 2];
    let records = records();
    // Day 7: start 7 % 3 = 1, walk records 1, 2, 0. Weather 1 is at 0.30 (step 1, |0.30 - 0.5| = 0.2)
    // and at 0.52 (step 2, |0.02|): the nearer is step 2, offset 0.52 - 0.5 = 0.02.
    // Starting at 0 would give step 0.
    let hit = find_in_schedule(&list, &records, 7, 0.5, 1).expect("weather 1 is scheduled");
    assert_eq!(hit.day_offset, 2);
    assert_close(hit.time_offset, 0.02, 1e-6);
    // Day -1 is outside what was read; the guard's rem_euclid gives start 2, walk 2, 0, 1, so
    // 0.52 is at step 1 and the walk stays in range.
    let back = find_in_schedule(&list, &records, -1, 0.5, 1).expect("weather 1 is scheduled");
    assert_eq!(back.day_offset, 1);
    assert_eq!(find_in_schedule(&list, &records, 7, 0.5, 9), None);
    assert_eq!(find_in_schedule(&[], &records, 7, 0.5, 1), None);
}

#[test]
fn a_tie_keeps_the_earliest_step() {
    let tied = vec![
        DayRecord {
            entries: vec![entry(1, 0.25)],
        },
        DayRecord {
            entries: vec![entry(1, 0.75)],
        },
    ];
    // 0.25 - 0.5 = -0.25 (step 0) and 0.75 - 0.5 = 0.25 (step 1), both exact: step 0 is kept.
    // `<=` would give step 1 and +0.25.
    let hit = find_in_schedule(&[0, 1], &tied, 0, 0.5, 1).expect("weather 1 is scheduled");
    assert_eq!(hit.day_offset, 0);
    assert_close(hit.time_offset, -0.25, 0.0);
}

#[test]
fn a_nan_time_is_passed_over() {
    let records = vec![DayRecord {
        entries: vec![entry(1, f32::NAN), entry(1, 0.52)],
    }];
    // NaN - 0.5 is skipped; 0.52 - 0.5 = 0.02. Kept, the NaN would stay best and be returned.
    let hit = find_in_schedule(&[0], &records, 0, 0.5, 1).expect("weather 1 is scheduled");
    assert_eq!(hit.day_offset, 0);
    assert_close(hit.time_offset, 0.02, 1e-6);
    // A NaN day fraction makes every offset NaN: nothing is found.
    assert_eq!(find_in_schedule(&[0], &records, 0, f32::NAN, 1), None);
}

#[test]
fn a_scheduled_time_within_0_01_is_now() {
    let list = [0u32];
    let records = records();
    // 0.52 - 0.515 = 0.005 < 0.01: 0. 0.52 - 0.5 = 0.02 is kept.
    let near = find_in_schedule(&list, &records, 0, 0.515, 1).expect("weather 1 is scheduled");
    assert_eq!(near.time_offset, 0.0);
    let far = find_in_schedule(&list, &records, 0, 0.5, 1).expect("weather 1 is scheduled");
    assert_close(far.time_offset, 0.02, 1e-6);
}
