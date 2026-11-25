pub mod clock;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeRange {
    pub start: f32,
    pub end: f32,
}

impl TimeRange {
    pub const fn new(start: f32, end: f32) -> Self {
        Self { start, end }
    }

    pub const fn from_duration(duration: f32) -> Self {
        Self {
            start: 0.0,
            end: duration,
        }
    }

    pub fn span(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    pub fn contains(&self, t: f32) -> bool {
        t >= self.start && t <= self.end
    }

    pub fn clamp(&self, t: f32) -> f32 {
        if self.is_empty() {
            self.start
        } else {
            t.clamp(self.start, self.end)
        }
    }

    pub fn ratio_of(&self, t: f32) -> f32 {
        let span = self.span();
        if span <= 0.0 {
            0.0
        } else {
            ((t - self.start) / span).clamp(0.0, 1.0)
        }
    }

    pub fn at_ratio(&self, ratio: f32) -> f32 {
        self.start + self.span() * ratio.clamp(0.0, 1.0)
    }

    pub fn clamped_into(self, outer: &TimeRange) -> Self {
        let start = self.start.clamp(outer.start, outer.end);
        let end = self.end.clamp(start, outer.end);
        Self { start, end }
    }
}

impl Default for TimeRange {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    #[default]
    Off,
    Wrap,
}

impl LoopMode {
    pub fn from_bool(looping: bool) -> Self {
        if looping {
            LoopMode::Wrap
        } else {
            LoopMode::Off
        }
    }

    pub fn is_looping(self) -> bool {
        matches!(self, LoopMode::Wrap)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub time: f32,
    pub playing: bool,
    pub rate: f32,
    pub loop_mode: LoopMode,
    pub range: TimeRange,
    pub bounds: TimeRange,
    pub frame_rate: Option<f32>,
    pub seek_pending: Option<f32>,
}

impl Default for Transport {
    fn default() -> Self {
        let range = TimeRange::from_duration(5.0);
        Self {
            time: 0.0,
            playing: false,
            rate: 1.0,
            loop_mode: LoopMode::Wrap,
            range,
            bounds: range,
            frame_rate: None,
            seek_pending: None,
        }
    }
}

impl Transport {
    pub fn with_duration(duration: f32) -> Self {
        let range = TimeRange::from_duration(duration.max(0.0));
        Self {
            range,
            bounds: range,
            ..Default::default()
        }
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn toggle(&mut self) {
        self.playing = !self.playing;
    }

    pub fn stop(&mut self) {
        self.playing = false;
        self.seek(self.range.start);
    }

    pub fn seek(&mut self, t: f32) {
        let t = self.range.clamp(t).max(0.0);
        self.time = t;
        self.seek_pending = Some(t);
    }

    pub fn take_seek(&mut self) -> Option<f32> {
        self.seek_pending.take()
    }

    pub fn seek_ratio(&mut self, ratio: f32) {
        self.seek(self.range.at_ratio(ratio));
    }

    pub fn ratio(&self) -> f32 {
        self.range.ratio_of(self.time)
    }

    pub fn is_looping(&self) -> bool {
        self.loop_mode.is_looping()
    }

    pub fn set_looping(&mut self, looping: bool) {
        self.loop_mode = LoopMode::from_bool(looping);
    }

    pub fn set_duration(&mut self, duration: f32) {
        let range = TimeRange::from_duration(duration.max(0.0));
        self.bounds = range;
        self.range = range;
        self.time = range.clamp(self.time);
    }

    pub fn set_range(&mut self, range: TimeRange) {
        self.range = range.clamped_into(&self.bounds);
        self.time = self.range.clamp(self.time);
    }

    pub fn reset_range(&mut self) {
        self.range = self.bounds;
    }

    pub fn frame_step(&self) -> Option<f32> {
        self.frame_rate
            .filter(|fps| *fps > 0.0)
            .map(|fps| 1.0 / fps)
    }

    pub fn frame(&self) -> Option<f32> {
        self.frame_rate
            .filter(|fps| *fps > 0.0)
            .map(|fps| (self.time - self.bounds.start) * fps)
    }

    pub fn frame_count(&self) -> Option<f32> {
        self.frame_rate
            .filter(|fps| *fps > 0.0)
            .map(|fps| self.bounds.span() * fps)
    }

    pub fn step_frames(&mut self, frames: i32, fallback: f32) {
        if frames == 0 {
            return;
        }
        let delta = match self.frame_step() {
            Some(step) => step * frames as f32,
            None => fallback * frames as f32,
        };
        let target = self.time + delta;
        let span = self.range.span();
        if self.is_looping() && span > 0.0 {
            let wrapped = (target - self.range.start).rem_euclid(span) + self.range.start;
            self.seek(wrapped);
        } else {
            self.seek(target);
        }
    }

    /// Advance the playhead by `real_dt` seconds of wall clock and return the simulated
    /// delta that elapsed.
    ///
    /// ⚠ The two boundary cases are load-bearing for the particle simulation's
    /// replay accounting: on a wrap the caller gets the *pre-wrap* elapsed time
    /// so the residual is replayed inside the wrapped section, and on a clamp
    /// only the portion of `dt` that fit before the end.
    pub fn tick(&mut self, real_dt: f32) -> f32 {
        if !self.playing {
            return 0.0;
        }
        let dt = (real_dt * self.rate).max(0.0);
        let new_time = self.time + dt;
        let end = self.range.end;
        let start = self.range.start;
        let span = self.range.span();

        if span <= 0.0 || new_time < end {
            self.time = new_time;
            return dt;
        }

        if self.is_looping() {
            let wrapped = start + (new_time - end);
            self.time = wrapped;
            self.seek_pending = Some(wrapped);
            span + (wrapped - start)
        } else {
            self.time = end;
            self.playing = false;
            end - (new_time - dt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_transport_does_not_advance() {
        let mut t = Transport::default();
        assert_eq!(t.tick(0.5), 0.0);
        assert_eq!(t.time, 0.0);
    }

    #[test]
    fn play_then_tick_advances_at_rate() {
        let mut t = Transport {
            rate: 2.0,
            ..Default::default()
        };
        t.play();
        let _ = t.tick(0.5);
        assert!((t.time - 1.0).abs() < 1e-5);
    }

    #[test]
    fn looping_wraps_at_range_end_and_flags_a_seek() {
        let mut t = Transport {
            playing: true,
            ..Transport::with_duration(1.0)
        };
        t.set_looping(true);
        let _ = t.tick(0.7);
        let _ = t.tick(0.5);
        assert!(t.time < 1.0);
        assert!(t.seek_pending.is_some());
    }

    #[test]
    fn nonlooping_clamps_and_pauses() {
        let mut t = Transport {
            playing: true,
            ..Transport::with_duration(1.0)
        };
        t.set_looping(false);
        let _ = t.tick(2.0);
        assert!((t.time - 1.0).abs() < 1e-5);
        assert!(!t.playing);
    }

    #[test]
    fn nonlooping_tick_returns_only_the_dt_that_fit() {
        let mut t = Transport {
            playing: true,
            time: 0.8,
            ..Transport::with_duration(1.0)
        };
        t.set_looping(false);
        let elapsed = t.tick(0.5);
        assert!((elapsed - 0.2).abs() < 1e-5, "elapsed was {elapsed}");
    }

    #[test]
    fn zero_span_range_never_wraps() {
        let mut t = Transport {
            playing: true,
            ..Transport::with_duration(0.0)
        };
        let elapsed = t.tick(0.25);
        assert!((elapsed - 0.25).abs() < 1e-6);
        assert!((t.time - 0.25).abs() < 1e-6);
    }

    #[test]
    fn working_range_is_clamped_into_bounds() {
        let mut t = Transport::with_duration(2.0);
        t.set_range(TimeRange::new(-1.0, 5.0));
        assert_eq!(t.range, TimeRange::new(0.0, 2.0));

        t.set_range(TimeRange::new(0.5, 1.5));
        assert_eq!(t.range, TimeRange::new(0.5, 1.5));
        assert!((t.time - 0.5).abs() < 1e-6);
    }

    #[test]
    fn seek_clamps_into_the_working_range() {
        let mut t = Transport::with_duration(2.0);
        t.set_range(TimeRange::new(0.5, 1.5));
        t.seek(0.0);
        assert!((t.time - 0.5).abs() < 1e-6);
        t.seek(9.0);
        assert!((t.time - 1.5).abs() < 1e-6);
    }

    #[test]
    fn narrowed_range_loops_inside_itself() {
        let mut t = Transport::with_duration(10.0);
        t.set_range(TimeRange::new(2.0, 3.0));
        t.set_looping(true);
        t.seek(2.9);
        t.play();
        let _ = t.tick(0.2);
        assert!((t.time - 2.1).abs() < 1e-5, "time was {}", t.time);
    }

    #[test]
    fn frame_stepping_uses_the_frame_rate() {
        let mut t = Transport {
            frame_rate: Some(30.0),
            ..Transport::with_duration(1.0)
        };
        assert!((t.frame_step().unwrap() - 1.0 / 30.0).abs() < 1e-6);
        t.step_frames(3, 0.1);
        assert!((t.time - 3.0 / 30.0).abs() < 1e-5);
        assert!((t.frame().unwrap() - 3.0).abs() < 1e-3);
    }

    #[test]
    fn frame_stepping_falls_back_when_no_rate_is_known() {
        let mut t = Transport::with_duration(1.0);
        assert!(t.frame_step().is_none());
        t.step_frames(1, 0.1);
        assert!((t.time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn stepping_back_from_zero_wraps_when_looping() {
        let mut t = Transport {
            frame_rate: Some(10.0),
            ..Transport::with_duration(1.0)
        };
        t.set_looping(true);
        t.seek(0.0);
        t.step_frames(-1, 0.1);
        assert!((t.time - 0.9).abs() < 1e-5, "time was {}", t.time);
    }

    #[test]
    fn stepping_back_from_zero_stalls_when_not_looping() {
        let mut t = Transport {
            frame_rate: Some(10.0),
            ..Transport::with_duration(1.0)
        };
        t.set_looping(false);
        t.seek(0.0);
        t.step_frames(-1, 0.1);
        assert!((t.time - 0.0).abs() < 1e-6);
    }

    #[test]
    fn frames_are_numbered_from_bounds_not_range() {
        let mut t = Transport {
            frame_rate: Some(30.0),
            ..Transport::with_duration(2.0)
        };
        t.set_range(TimeRange::new(1.0, 2.0));
        assert!((t.frame().unwrap() - 30.0).abs() < 1e-3);
        assert!((t.frame_count().unwrap() - 60.0).abs() < 1e-3);
    }

    #[test]
    fn ratio_maps_both_ways_within_the_range() {
        let mut t = Transport::with_duration(4.0);
        t.seek_ratio(0.25);
        assert!((t.time - 1.0).abs() < 1e-6);
        assert!((t.ratio() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn set_duration_resets_the_working_range() {
        let mut t = Transport::with_duration(10.0);
        t.set_range(TimeRange::new(4.0, 6.0));
        t.set_duration(2.0);
        assert_eq!(t.range, TimeRange::new(0.0, 2.0));
        assert_eq!(t.bounds, TimeRange::new(0.0, 2.0));
    }

    #[test]
    fn stop_rewinds_to_the_range_start() {
        let mut t = Transport::with_duration(5.0);
        t.set_range(TimeRange::new(1.0, 3.0));
        t.seek(2.0);
        t.play();
        t.stop();
        assert!(!t.playing);
        assert!((t.time - 1.0).abs() < 1e-6);
        assert_eq!(t.take_seek(), Some(1.0));
    }
}
