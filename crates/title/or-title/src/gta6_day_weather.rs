//! The weather's state: an automatic change and a queued weather layered on it, the per-weather modifier slots, the uniform pick of the next weather and its generator, the calendar lookup by days and a weather record's viewer fields.
//! Not coded, because not read: the commit routines at the end of a change and of a queue, what the schedule selector's two indices mean, the generator's seed, and the slot strength when both weathers have a modifier. The wind a weather sets is the wind module's.

use crate::gta6_day_frame::QueuedModifier;

/// A weather record's wind, as the wind module takes it.
// lander: a copy of the wind track's `gta6_day_wind::WeatherWind`; keep one when both land.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeatherWind {
    /// Base speed at walk 0, m/s (inferred).
    pub speed_low: f32,
    /// Base speed at walk 1, m/s (inferred).
    pub speed_high: f32,
    /// Heading the wind comes from, degrees clockwise from north (the from is inferred).
    pub heading_degrees: f32,
}

/// Milliseconds a queued weather counts down per unit of dt; that the unit is a second is GTA V's, assumed.
pub const MILLISECONDS_PER_DT: f32 = 1000.0;

/// Duration, in units of dt, the one known caller queues a weather for.
pub const QUEUED_CALLER_DURATION: f32 = 45.0;

/// Day fraction within which a scheduled time counts as now, so its offset is 0.
const SCHEDULE_SNAP: f32 = 0.01;

/// Cycles a weather can name; an index at or past this reads cycle 0.
pub const MAX_CYCLES: usize = 64;

/// A weather slot's priority before its k: the order id is base·1000 + 250 + k.
const WEATHER_SLOT_ORDER_OFFSET: i32 = 250;

/// Bit 7 of a modifier's +0x48 flags, which sends its weather slot to the second queue too.
const SECOND_QUEUE_FLAG: u32 = 0x80;

/// Multiplier of the multiply-with-carry generator.
const MWC_MULTIPLIER: u64 = 0x5cdc_faa7;

/// The low 15 bits of the generator's state a draw takes.
const MWC_UNIT_MASK: u64 = 0x7fff;

/// 2^-15, the step of a draw.
const MWC_UNIT_SCALE: f32 = 1.0 / 32768.0;

/// A weather type's fields the viewer reads: its eight modifiers, its cycle and its wind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherType {
    /// Record +0x04..+0x20: timecycle modifier name hashes, slot k = 0..7.
    pub modifier_hashes: [u32; 8],
    /// Weather object +0x14c: name hash of the weather's cycle; a lookup by name gives its index.
    pub cycle_hash: u32,
    /// Weather object +0x08, +0x0c, +0x10: the base wind speed pair (m/s, inferred) and the heading in degrees, applied by wind::WindState::apply_weather when a weather is applied, not each frame.
    pub wind: WeatherWind,
}

/// The cycle slot a cycle index reads: the index itself below 64, else slot 0.
pub fn cycle_slot(index: u32) -> usize {
    let slot: usize = index as usize;
    if slot < MAX_CYCLES {
        slot
    } else {
        0
    }
}

/// An automatic change from one weather type to another, its weight rising linearly over its duration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoTransition {
    /// Weather type index the change leaves.
    pub from: u32,
    /// Weather type index the change reaches.
    pub to: u32,
    /// Blend weight wx, 0..1.
    pub weight: f32,
    /// Length of the change, units of dt, never negative.
    pub duration: f32,
    /// Units of dt since the change started.
    pub elapsed: f32,
}

/// The weathers and weight the base frame blends; gta6_day_frame::blend_weathers takes next = None exactly when prev == next (day 4.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendInputs {
    /// The previous weather, A of the base blend.
    pub prev: u32,
    /// The next weather, B of the base blend.
    pub next: u32,
    /// The weight wx from prev to next.
    pub weight: f32,
}

impl AutoTransition {
    /// A change at weight 0 over `duration` units of dt (gta6_day_frame::AUTO_TRANSITION_DURATION); a negative or NaN duration is 0, which ends on the first step.
    pub fn start(from: u32, to: u32, duration: f32) -> AutoTransition {
        AutoTransition {
            from,
            to,
            weight: 0.0,
            // A negative duration would hold gta6_day_frame::auto_transition_step at 0 for ever.
            duration: duration.max(0.0),
            elapsed: 0.0,
        }
    }

    /// Whether the weight has reached 1, after which the change no longer advances.
    pub fn finished(&self) -> bool {
        self.weight >= 1.0
    }

    /// Elapsed units after a step of dt, the elapsed argument of gta6_day_frame::auto_transition_step.
    pub fn elapsed_after(&self, dt: f32) -> f32 {
        self.elapsed + dt
    }

    /// One step of dt, the weight from gta6_day_frame::auto_transition_step; a finished change is left as it is.
    pub fn advance(&mut self, dt: f32) {
        if self.finished() {
            return;
        }
        self.elapsed = self.elapsed_after(dt);
        self.weight =
            crate::gta6_day_frame::auto_transition_step(self.weight, self.elapsed, self.duration);
    }

    /// The base blend's inputs: from and to where below weather_count, else the weather kept held.
    pub fn blend_inputs(&self, weather_count: usize, kept: BlendInputs) -> BlendInputs {
        BlendInputs {
            prev: if (self.from as usize) < weather_count {
                self.from
            } else {
                kept.prev
            },
            next: if (self.to as usize) < weather_count {
                self.to
            } else {
                kept.next
            },
            weight: self.weight,
        }
    }

    /// The change after a finished one: None when `to` is in today's schedule, else a change from `to` to `picked` (see pick_next_weather).
    pub fn follow_on(&self, scheduled: bool, picked: u32, duration: f32) -> Option<AutoTransition> {
        if scheduled {
            None
        } else {
            Some(AutoTransition::start(self.to, picked, duration))
        }
    }
}

/// A queued or scripted weather layered over the base, its weight rising linearly over its duration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueuedWeather {
    /// Weather type index queued.
    pub weather: u32,
    /// Milliseconds left of the rise.
    pub left_ms: f32,
    /// Milliseconds the rise lasts in all, never negative.
    pub total_ms: f32,
    /// Layer weight wo, 0..1; also the strength of its eight modifiers in the override slots.
    pub weight: f32,
}

/// A queued weather after a step: still rising, or reached weight 1 and to become the base weather with the queue cleared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueueStep {
    /// The queue holds this state.
    Running(QueuedWeather),
    /// This weather type index becomes the base, and nothing is queued.
    Commit(u32),
}

impl QueuedWeather {
    /// A weather queued to rise over `duration` units of dt, at weight 0; a negative or NaN duration is 0, which ends on the first step.
    pub fn new(weather: u32, duration: f32) -> QueuedWeather {
        // A negative total would give left/total > 1 and so weight 0 for ever.
        let total_ms: f32 = duration.max(0.0) * MILLISECONDS_PER_DT;
        QueuedWeather {
            weather,
            left_ms: total_ms,
            total_ms,
            weight: 0.0,
        }
    }

    /// Milliseconds left after a step of dt, the first argument of gta6_day_frame::queued_weather_weight.
    pub fn left_after(&self, dt: f32) -> f32 {
        self.left_ms - MILLISECONDS_PER_DT * dt
    }

    /// One step of dt, the weight from gta6_day_frame::queued_weather_weight; at weight 1 the weather commits.
    pub fn advance(self, dt: f32) -> QueueStep {
        let left_ms: f32 = self.left_after(dt);
        let weight: f32 = crate::gta6_day_frame::queued_weather_weight(left_ms, self.total_ms);
        if weight >= 1.0 {
            return QueueStep::Commit(self.weather);
        }
        QueueStep::Running(QueuedWeather {
            left_ms,
            weight,
            ..self
        })
    }
}

/// Weather modifier slot k (0..7) as a queue entry, its order key base·1000 + 250 + k (gta6_day_frame::modifier_order_key); that base is the category index is inferred.
pub fn weather_slot_modifier(base: i32, slot: i32) -> QueuedModifier {
    QueuedModifier {
        category: base,
        priority: WEATHER_SLOT_ORDER_OFFSET.wrapping_add(slot),
    }
}

/// The two fields of a timecycle modifier that decide its weather slot's queues.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotModifier {
    /// Modifier +0x20; its meaning was not read, only that non-zero counts.
    pub field_0x20: u32,
    /// Modifier +0x48 flags; only bit 7 was read.
    pub flags_0x48: u32,
}

/// Whether a weather slot, which always goes to the first modifier queue, also goes to the second: either present modifier has +0x20 non-zero or bit 7 of +0x48.
pub fn slot_in_second_queue(prev: Option<SlotModifier>, next: Option<SlotModifier>) -> bool {
    [prev, next]
        .into_iter()
        .flatten()
        .any(|modifier| modifier.field_0x20 != 0 || modifier.flags_0x48 & SECOND_QUEUE_FLAG != 0)
}

/// The generator's next state, (x & 0xffffffff)·0x5cdcfaa7 + (x >> 32).
pub fn mwc_next(x: u64) -> u64 {
    // Cannot overflow: (2^32 - 1)·0x5cdcfaa7 + (2^32 - 1) < 2^63.
    (x & 0xffff_ffff) * MWC_MULTIPLIER + (x >> 32)
}

/// A draw in [0, 1) from a state mwc_next returned: its low 15 bits times 2^-15.
pub fn mwc_unit(x: u64) -> f32 {
    (x & MWC_UNIT_MASK) as f32 * MWC_UNIT_SCALE
}

/// One entry of a day record: a weather type index and its time of day, a day fraction in [0, 1].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayEntry {
    /// Weather type index.
    pub weather: u32,
    /// Time of day, day fraction.
    pub time: f32,
}

/// A day record: the weathers of one calendar day with their times.
#[derive(Clone, Debug, PartialEq)]
pub struct DayRecord {
    /// The day's entries; one count serves both the weather and the time list.
    pub entries: Vec<DayEntry>,
}

/// Where a weather is found in the schedule: the walk's day step and its time minus the clock's day fraction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduleHit {
    /// Step i of the walk, days after today's record.
    pub day_offset: usize,
    /// Entry time minus the day fraction, 0 when within 0.01.
    pub time_offset: f32,
}

/// The day record at `day_list[index]`, or None when either index is past its table.
fn day_record<'a>(
    day_list: &[u32],
    records: &'a [DayRecord],
    index: usize,
) -> Option<&'a DayRecord> {
    let record: u32 = *day_list.get(index)?;
    records.get(record as usize)
}

/// Truncated r·count, the index a draw r picks from `count` items; None for a draw outside [0, 1), NaN included.
fn draw_index(r: f32, count: usize) -> Option<usize> {
    if !(0.0..1.0).contains(&r) {
        return None;
    }
    // A product rounded up to count is refused by the caller's get.
    Some((r * count as f32) as usize)
}

/// The next weather type: day record day_list[int(r1·len)], then its entry int(r2·count), r1 and r2 two successive mwc_unit draws; None when a draw is outside [0, 1), a list is empty or an index is past its table.
pub fn pick_next_weather(day_list: &[u32], records: &[DayRecord], r1: f32, r2: f32) -> Option<u32> {
    let record: &DayRecord = day_record(day_list, records, draw_index(r1, day_list.len())?)?;
    let entry: &DayEntry = record.entries.get(draw_index(r2, record.entries.len())?)?;
    Some(entry.weather)
}

/// Whether `weather` is in the schedule: walking day_list[(day % len + i) % len], the entry of that weather whose time is nearest `day_fraction` (gta6_day_sun::clock_day_fraction as f32); `day` counts days since 1970 (inferred).
pub fn find_in_schedule(
    day_list: &[u32],
    records: &[DayRecord],
    day: i64,
    day_fraction: f32,
    weather: u32,
) -> Option<ScheduleHit> {
    let count: usize = day_list.len();
    if count == 0 {
        return None;
    }
    // Only a day >= 0 was read; rem_euclid keeps a negative one in range and is a guard, not the game's rule.
    let start: usize = day.rem_euclid(count as i64) as usize;
    // Trap: the nearest is taken over every walked day, the earliest step on a tie; whether the game
    // stops at the first day holding the weather was not read.
    let mut best: Option<(usize, f32)> = None;
    for step in 0..count {
        let record: &DayRecord = match day_record(day_list, records, (start + step) % count) {
            Some(record) => record,
            None => continue,
        };
        for entry in record
            .entries
            .iter()
            .filter(|entry| entry.weather == weather)
        {
            let offset: f32 = entry.time - day_fraction;
            // A NaN offset would stay best, since nothing compares below it.
            if !offset.is_finite() {
                continue;
            }
            let nearer: bool = match best {
                Some((_, best_offset)) => offset.abs() < best_offset.abs(),
                None => true,
            };
            if nearer {
                best = Some((step, offset));
            }
        }
    }
    best.map(|(step, offset)| ScheduleHit {
        day_offset: step,
        time_offset: if offset.abs() < SCHEDULE_SNAP {
            0.0
        } else {
            offset
        },
    })
}

#[cfg(test)]
#[path = "gta6_day_weather_tests.rs"]
mod tests;
