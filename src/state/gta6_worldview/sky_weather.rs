//! The weathers a frame of the GTA VI world is drawn in: the game's change from one to another on the frame clock, and a weather queued over it.

use std::time::Instant;

use or_title::gta6_day_frame::AUTO_TRANSITION_DURATION;
use or_title::gta6_day_weather::{
    AutoTransition, QueueStep, QueuedWeather, QUEUED_CALLER_DURATION,
};

use super::sky::weather_name;
use crate::state::drivers::WeatherState;

/// The weathers a frame is drawn in: a change from one to another and how far it has come (0 to 1), and a weather queued over it with its weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weathers {
    pub from: WeatherState,
    pub to: WeatherState,
    pub along: f32,
    pub queued: Option<(WeatherState, f32)>,
}

impl Weathers {
    /// One weather alone: a change to it that has come all the way.
    #[cfg(test)]
    pub fn steady(weather: WeatherState) -> Self {
        Self {
            from: weather,
            to: weather,
            along: 1.0,
            queued: None,
        }
    }

    /// The weather the frame is on its way to: the queued one where one is, else the change's.
    pub fn target(&self) -> WeatherState {
        self.queued.map_or(self.to, |(weather, _)| weather)
    }

    /// The weathers by the names of the cycles they stand for.
    pub(super) fn named(&self) -> or_title::gta6_day::WeatherBlend<'static> {
        or_title::gta6_day::WeatherBlend {
            from: weather_name(self.from),
            to: weather_name(self.to),
            along: self.along,
            queued: self.queued.map(|(w, weight)| (weather_name(w), weight)),
        }
    }

    /// The weathers as a key: the weights by their bits.
    pub fn key(&self) -> WeathersKey {
        (
            self.from,
            self.to,
            self.along.to_bits(),
            self.queued.map(|(w, weight)| (w, weight.to_bits())),
        )
    }
}

/// [`Weathers`] with the weights by their bits.
pub type WeathersKey = (WeatherState, WeatherState, u32, Option<(WeatherState, u32)>);

/// The game's change of weather as the viewer runs it on the frame clock: the weather chosen is reached over the game's automatic change, and one chosen while a change runs is queued over it until it becomes the base.
// ⚠ The game picks the next weather from a calendar the install's data does not state: the viewer's next weather is the one chosen.
#[derive(Debug, Default)]
pub struct Weathering {
    change: Option<AutoTransition>,
    queued: Option<QueuedWeather>,
    last: Option<Instant>,
}

/// A weather's place in [`WeatherState::ALL`], the index the game's change counts it by.
fn index_of(weather: WeatherState) -> u32 {
    (WeatherState::ALL.iter())
        .position(|w| *w == weather)
        .unwrap_or(0) as u32
}

fn state_of(index: u32) -> WeatherState {
    (WeatherState::ALL.get(index as usize).copied()).unwrap_or_default()
}

impl Weathering {
    /// The weathers of this frame, `chosen` the one the bar now names: a change towards it started or queued, then the change and the queue stepped by the frame's time.
    pub fn step(&mut self, chosen: WeatherState) -> Weathers {
        let now = or_transport::clock::now();
        let dt = self
            .last
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last = Some(now);
        self.step_by(chosen, dt)
    }

    /// [`Self::step`] by `dt` seconds.
    fn step_by(&mut self, chosen: WeatherState, dt: f32) -> Weathers {
        let to = index_of(chosen);
        // The first weather is the one the screen opens in, with no change.
        let change = self.change.get_or_insert(AutoTransition {
            weight: 1.0,
            ..AutoTransition::start(to, to, 0.0)
        });
        let towards = self.queued.map_or(change.to, |q| q.weather);
        if towards != to {
            match change.finished() && self.queued.is_none() {
                true => *change = AutoTransition::start(change.to, to, AUTO_TRANSITION_DURATION),
                false => self.queued = Some(QueuedWeather::new(to, QUEUED_CALLER_DURATION)),
            }
        }
        change.advance(dt);
        self.queued = match self.queued.map(|q| q.advance(dt)) {
            Some(QueueStep::Running(q)) => Some(q),
            Some(QueueStep::Commit(weather)) => {
                // ⚠ The game's commit routine was not read: the queued weather becomes the base whole.
                *change = AutoTransition {
                    weight: 1.0,
                    ..AutoTransition::start(weather, weather, 0.0)
                };
                None
            }
            None => None,
        };
        Weathers {
            from: state_of(change.from),
            to: state_of(change.to),
            along: change.weight,
            queued: self.queued.map(|q| (state_of(q.weather), q.weight)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_weather_chosen_is_reached_over_a_minute_and_one_chosen_meanwhile_is_queued_over_it() {
        use WeatherState::*;
        let mut weathering = Weathering::default();
        assert_eq!(weathering.step_by(Clear, 0.0), Weathers::steady(Clear));
        assert_eq!(weathering.step_by(Clear, 5.0), Weathers::steady(Clear));
        // A change is linear over sixty seconds.
        let started = weathering.step_by(Rain, 0.0);
        assert_eq!(
            (started.from, started.to, started.along),
            (Clear, Rain, 0.0)
        );
        let half = weathering.step_by(Rain, 30.0);
        assert!((half.along - 0.5).abs() < 1e-6, "{half:?}");
        // One chosen during the change rises over it in forty-five seconds, then is the base.
        let queued = weathering.step_by(Fog, 0.0);
        assert_eq!(queued.queued, Some((Fog, 0.0)));
        let on = weathering.step_by(Fog, 15.0);
        let (weather, weight) = on.queued.expect("queued");
        assert_eq!(weather, Fog);
        assert!((weight - 1.0 / 3.0).abs() < 1e-5, "{on:?}");
        assert!((on.along - 0.75).abs() < 1e-6, "{on:?}");
        let committed = weathering.step_by(Fog, 30.0);
        assert_eq!(committed, Weathers::steady(Fog));
        assert_eq!(committed.target(), Fog);
    }
}
