//! The game's wind about the viewer: its way and base speed from the weather's data, its gusts
//! and timecycle speeds from the moment's variables, stepped each frame, as the grass bends in it.

use or_renderer::world::{WorldWind, WORLD_GUSTS};
use or_title::gta6_day_weather::mwc_next;
use or_title::gta6_day_wind::{
    max_speed, unit_random, GustRandoms, WeatherWind, WindConfig, WindState, WindVariables,
    DEFAULT_VOLUME_MAX_SPEED_KMH,
};
use or_transport::clock;

use super::sky;
use super::Viewer;
use crate::state::drivers::WeatherState;

/// Where the base speed stands between the weather's least and most.
// ponytail: the game walks it at random over [0, 1) at a rate and by thresholds not read; the middle
// stands in until they are.
const WALK: f32 = 0.5;
/// The state the gusts' generator starts from.
// ponytail: the game's seed was not read; any fixed one gives the same gusts each run.
const SEED: u64 = 0x2545_f491_4f6c_dd1d;
/// The longest step a frame takes, in seconds: a frame held up longer is not a gust's whole life gone.
const LONGEST_STEP: f32 = 0.25;

/// The wind's state and what it was last stepped from.
pub(super) struct Wind {
    state: WindState,
    /// The weather whose data set the way and base speed.
    weather: Option<WeatherState>,
    random: u64,
    /// The seconds the wind has run, which the gusts are timed by.
    clock: f32,
    /// The game time it was last stepped at.
    last: Option<f64>,
}

impl Default for Wind {
    fn default() -> Self {
        Self {
            state: WindState::calm(),
            weather: None,
            random: SEED,
            clock: 0.0,
            last: None,
        }
    }
}

impl Wind {
    fn draw(&mut self) -> f32 {
        self.random = mwc_next(self.random);
        unit_random(self.random)
    }

    /// The wind one frame on, at game time `now` in seconds, `weather` its data where the install states one, `variables` the moment's.
    fn step(
        &mut self,
        now: f64,
        (state, weather): (WeatherState, Option<WeatherWind>),
        variables: &WindVariables,
    ) -> WorldWind {
        let dt = (self.last.map_or(0.0, |last| (now - last).max(0.0) as f32)).min(LONGEST_STEP);
        self.last = Some(now);
        self.clock += dt;
        // As the game does, the way and base speed are taken when the weather changes, not each frame.
        if self.weather != Some(state) {
            self.weather = Some(state);
            if let Some(weather) = weather {
                self.state.apply_weather(&weather, variables, WALK);
            }
        }
        let randoms = GustRandoms {
            interval: self.draw(),
            strength: self.draw(),
            duration: self.draw(),
            angle: self.draw(),
            x: self.draw(),
            y: self.draw(),
        };
        let config = WindConfig::image_defaults();
        (self.state).step(self.clock, dt, variables, WALK, &randoms, &config);
        world_wind(&self.state)
    }
}

/// The wind as the world's program takes it: the way, the base speed and the most speed, and each live gust.
fn world_wind(state: &WindState) -> WorldWind {
    let mut gusts = [[0.0; 4]; WORLD_GUSTS];
    for (row, gust) in gusts.iter_mut().zip(&state.gusts) {
        *row = [
            gust.position.x,
            gust.position.y,
            gust.strength * gust.weight,
            gust.radius,
        ];
    }
    WorldWind {
        direction: [state.direction.x, state.direction.y],
        speed: state.base_speed,
        most: max_speed([DEFAULT_VOLUME_MAX_SPEED_KMH; 2]),
        gusts,
    }
}

impl Viewer {
    /// The wind the grass bends in this frame, in `weathers`, its way and base speed the weather's they go towards: calm where the install's day or its executable's variable table is not read.
    pub fn wind(&mut self, weathers: sky::Weathers) -> WorldWind {
        let weather = weathers.target();
        let Some(tables) = self.root.as_deref().and_then(sky::tables) else {
            return WorldWind::default();
        };
        let Some(table) = &tables.executable.variables else {
            return WorldWind::default();
        };
        let room = (self.inside())
            .map(|inside| ([inside.timecycle, inside.secondary_timecycle], inside.blend));
        let moment = sky::moment(&tables, self.hour, weathers, self.camera.position.y, room);
        let Some(variables) = moment.and_then(|m| WindVariables::of_moment(&m.own, table)) else {
            return WorldWind::default();
        };
        let name = sky::weather_name(weather);
        let data = (tables.weather.weather(name))
            .and_then(|w| tables.weather.data(&w.data))
            .or_else(|| tables.weather.data(name));
        (self.wind).step(
            clock::game_seconds(),
            (weather, data.map(WeatherWind::of)),
            &variables,
        )
    }

    /// Start the wind again as it starts: no gust, its generator from its seed, its clock at nought.
    pub fn restart_wind(&mut self) {
        self.wind = Wind::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_takes_the_way_the_base_speed_the_most_and_each_gust_by_its_ramp() {
        let weather = WeatherWind {
            speed_low: 1.0,
            speed_high: 2.0,
            heading_degrees: 0.0,
        };
        let mut wind = Wind::default();
        let calm = WindVariables {
            heading_weight: 0.0,
            heading_degrees: 0.0,
            gust_strength_kmh: [36.0, 36.0],
            gust_duration: [10.0, 10.0],
            gust_interval: [0.0, 0.0],
            speed_a_kmh: [0.0, 0.0],
            speed_b_kmh: [0.0, 0.0],
        };
        let first = wind.step(0.0, (WeatherState::Clear, Some(weather)), &calm);
        // Heading 0: the wind comes from the north and blows south, at lerp(1, 2, 0.5).
        assert_eq!(first.direction, [0.0, -1.0]);
        assert_eq!(first.speed, 1.5);
        // 74 km/h in m/s.
        assert!((first.most - 74.0 / 3.6).abs() < 1e-4, "{}", first.most);
        // A gust of 36 km/h is born on the first frame at no weight: it adds nothing yet.
        assert_eq!(first.gusts[0][2], 0.0);
        assert_eq!(first.gusts[0][3], 800.0);
        // A frame later it has ramped in by the time gone over a fifth of its ten seconds.
        let later = wind.step(0.02, (WeatherState::Clear, Some(weather)), &calm);
        assert!(later.gusts[0][2] > 0.0, "{:?}", later.gusts);
        assert!(later.gusts[0][2] < 10.0, "{:?}", later.gusts);
        // The same game time after a restart gives the same gusts: the wind runs on the game's clock alone.
        let mut again = Wind::default();
        again.step(0.0, (WeatherState::Clear, Some(weather)), &calm);
        let twice = again.step(0.02, (WeatherState::Clear, Some(weather)), &calm);
        assert_eq!(twice.gusts, later.gusts);
    }
}
