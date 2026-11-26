//! The look of the day a GTA VI install states: its time table, weather cycles, weather set,
//! modifiers and visual settings, found among the clear texts of its common archive by what each
//! opens with, and a moment of that day as a frame built for GTA V's day takes it.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::Path;

use glam::Vec3;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9Scan};
use or_io::gta5::data::timecycle::{Keyframe, TimeTable, Timecycle};
use or_io::gta5::data::visual_settings::VisualSettings;
use or_io::gta6::timecycle::{
    apply_modifier, blend_modifiers, frame_hours, frame_keyframe, high_level_vars, modifiers,
    sun_roll, DayTable, FrameLight, Modifier, FRAME_DIFFUSE, LATITUDE, SAMPLE_ANGLES,
};
use or_io::gta6::weather::{WeatherData, WeatherSet};

use crate::gta6_day_frame::{
    apply_mode2, eased_t, f32_to_half, name_hash, pick_samples, sample_curve_index,
    stored_half_value, SampleKey,
};
use crate::gta6_day_sun::{
    moon_cycle, moon_direction, sample_angle, sun_day, MoonTunables, SunTunables,
    DEFAULT_DAY_OF_YEAR,
};
use crate::gta6_day_vars::{read_executable_day, ExecutableDay};
use crate::gta6_exposure::{adapt, bodies, light, target_stops, Bodies, ExposureSettings};

/// What a text of the day may inflate to; a longer one is passed over.
const BODY_LIMIT: u64 = 16 << 20;
/// The inflated bytes a scan keeps of each body, in which a document's root element is sought.
const HEAD: usize = 512;
/// The root element of each XML document of the day.
const ROOTS: [&str; 4] = [
    "<time_data",
    "<timecycle_keyframe_data",
    "<CContentsOfWeatherXmlFile",
    "<timecycle_modifier_data",
];
/// A name the visual settings state, by which their text is told from any other plain text.
const VISUAL_SETTINGS_MARK: &str = "Tonemapping.";
/// The high-level values of a cycle a moment carries among its own variables, for its cloud layer.
const CLOUD_VARS: [&str; 2] = ["cloud_cover", "cloud_type"];
/// The variables the light chain reads by name, which take the defaults of the executable's table where a moment states none.
const LIGHT_VARIABLES: [&str; 27] = [
    "light_dir_col_r",
    "light_dir_col_g",
    "light_dir_col_b",
    "light_dir_mult",
    "light_dir_sun_mult",
    "light_dir_moon_mult",
    "galaxy_light_dir_col_r",
    "galaxy_light_dir_col_g",
    "galaxy_light_dir_col_b",
    "galaxy_light_dir_mult",
    "light_direction_override",
    "light_direction_override_overrides_sun",
    "sun_direction_override",
    "moon_direction_override",
    "sun_direction_x",
    "sun_direction_y",
    "sun_direction_z",
    "moon_direction_x",
    "moon_direction_y",
    "moon_direction_z",
    "sun_fade_threshold",
    "moon_fade_threshold",
    "sun_moon_crossfade",
    "sky_scattering_sun_illuminance",
    "sky_scattering_sun_intensity_multiplier",
    "sky_scattering_moon_illuminance",
    "sky_scattering_moon_intensity_multiplier",
];
/// The east edge of the sun's observer box, in metres, where the visual settings state none.
// ponytail: a calibration knob, the west edge mirrored; the executable's own value was not read.
const EAST_X: f64 = 11_000.0;

/// What a GTA VI install states of the look of its day.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayTables {
    pub table: DayTable,
    pub timecycle: Timecycle,
    /// The `High_Level_Vars` of each cycle, by the cycle's name in lower case.
    pub high_level: BTreeMap<String, BTreeMap<String, f32>>,
    pub weather: WeatherSet,
    pub modifiers: Vec<Modifier>,
    pub visual_settings: VisualSettings,
    /// The variable table and the noon table of the install's executable.
    pub executable: ExecutableDay,
}

/// One moment of GTA VI's day as a frame built for GTA V's day takes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    /// The frame's variables, by GTA V's names.
    pub keyframe: Keyframe,
    /// GTA VI's own variables at the moment, by its names.
    pub own: Keyframe,
    /// The time table the frame works its sun and moon out from.
    pub table: TimeTable,
    /// The hour to give the frame: that of its own day at which its sun stands where GTA VI's does.
    pub frame_hours: f32,
    /// The way towards the sun, x east, y north, z up.
    pub sun: [f32; 3],
    /// The way towards the moon.
    pub moon: [f32; 3],
    /// The sun's day phase: 0.25 at sunrise, 0.5 at its noon and 0.75 at sunset, which the lights' cap by day is read on.
    pub day_phase: f64,
    /// The exposure the moment's light is given at, in stops.
    pub stops: f32,
    /// The moment's light, which [`frame_keyframe`] gives the frame again at another exposure.
    pub light: FrameLight,
    /// What a light the game states in its own unit is multiplied by to be the frame's: the exposure over pi.
    pub light_scale: f32,
    pub moon_dim: f32,
    pub wetness: f32,
    /// Which body lights the moment, how faded, and the way its light comes from.
    pub bodies: Bodies,
}

/// The weathers a moment is made of, by name: a change from `from` to `to`, `along` (0 to 1) of the way, and a weather queued over it at its weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherBlend<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub along: f32,
    pub queued: Option<(&'a str, f32)>,
}

impl<'a> WeatherBlend<'a> {
    /// One weather alone.
    pub fn steady(weather: &'a str) -> Self {
        Self {
            from: weather,
            to: weather,
            along: 0.0,
            queued: None,
        }
    }
}

type Vars = BTreeMap<String, [f32; 2]>;

/// The modifier `of` holds in `slot`.
fn in_slot<'a>(of: &[(&str, &'a Vars)], slot: &str) -> Option<&'a Vars> {
    (of.iter()).find(|(s, _)| *s == slot).map(|(_, vars)| *vars)
}

/// The head of `text` a root element is sought in.
fn head(text: &str) -> &str {
    let mut end = text.len().min(HEAD);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

impl DayTables {
    /// The tables the texts `documents` state, each told by its root element or, for the visual settings, by a name they state; any other text is passed over.
    pub fn of_texts<'a>(documents: impl IntoIterator<Item = &'a str>) -> Self {
        let mut tables = Self::default();
        for text in documents {
            let opens = |root: &str| head(text).contains(root);
            if opens(ROOTS[0]) {
                tables.table = DayTable::parse(text);
            } else if opens(ROOTS[1]) {
                let before = tables.timecycle.cycles.len();
                tables.timecycle.add_cycles(text);
                let vars = high_level_vars(text);
                // The game holds a cycle's samples as half floats of half their value.
                let samples = (tables.timecycle.cycles[before..].iter_mut())
                    .flat_map(|cycle| cycle.regions.iter_mut())
                    .flat_map(|region| region.vars.values_mut())
                    .flatten();
                for value in samples {
                    *value = stored_half_value(f32_to_half(*value / 2.0));
                }
                for cycle in &tables.timecycle.cycles[before..] {
                    (tables.high_level).insert(cycle.name.to_ascii_lowercase(), vars.clone());
                }
            } else if opens(ROOTS[2]) {
                match WeatherSet::parse(text) {
                    Ok(set) => tables.weather = set,
                    Err(error) => tracing::warn!(%error, "GTA VI's weather set is not read"),
                }
            } else if opens(ROOTS[3]) {
                tables.modifiers.extend(modifiers(text));
            } else if !text.trim_start().starts_with('<') && text.contains(VISUAL_SETTINGS_MARK) {
                tables.visual_settings = VisualSettings::parse(text);
            }
        }
        tables
    }

    /// Whether no cycle or no time table was read, so that no moment can be given.
    pub fn is_empty(&self) -> bool {
        self.timecycle.cycles.is_empty() || self.table.samples.is_empty()
    }

    /// The names a caller may pass [`Self::moment`] as its weather: the cycles read.
    pub fn weathers(&self) -> Vec<&str> {
        (self.timecycle.cycles.iter())
            .map(|c| c.name.as_str())
            .collect()
    }

    /// The latitude, in radians, of a point `y` metres north on the map, by the range the visual settings state.
    pub fn latitude(&self, y: f32) -> f32 {
        self.stated_latitude(y).unwrap_or(LATITUDE)
    }

    fn stated_latitude(&self, y: f32) -> Option<f32> {
        let get = |name: &str| (self.visual_settings.get(name)).and_then(|v| v.parse::<f32>().ok());
        let (low, high) = (get("sky.latitude.YPosMin")?, get("sky.latitude.YPosMax")?);
        let (from, to) = (get("sky.latitude.rangeMin")?, get("sky.latitude.rangeMax")?);
        let along = ((y - low) / (high - low)).clamp(0.0, 1.0);
        (high > low).then(|| (from + (to - from) * along).to_radians())
    }

    /// The weather data `weather` names: that of the weather type of that name, else the data of that name.
    fn data_of(&self, weather: &str) -> Option<&WeatherData> {
        (self.weather.weather(weather))
            .and_then(|w| self.weather.data(&w.data))
            .or_else(|| self.weather.data(weather))
    }

    /// The cycle that stands for `weather`: that of the weather type of that name, else the cycle of that name.
    fn cycle_of(&self, weather: &str) -> Option<&str> {
        let of_type = (self.data_of(weather)).and_then(|d| self.timecycle.cycle(&d.time_cycle));
        (of_type.or_else(|| self.timecycle.cycle(weather))).map(|c| c.name.as_str())
    }

    /// The modifier of the weather type `weather` in each of its slots, by the slot's element, in the file's order.
    fn modifiers_of(&self, weather: &str) -> Vec<(&str, &Vars)> {
        let named =
            |name: &str| (self.modifiers.iter()).find(|m| m.name.eq_ignore_ascii_case(name));
        let slots = self
            .weather
            .weather(weather)
            .map(|w| w.modifiers.as_slice());
        (slots.unwrap_or_default().iter())
            .filter_map(|(slot, name)| Some((slot.as_str(), &named(name)?.vars)))
            .collect()
    }

    /// Lay the modifiers of the weathers `from` and `to` over `own`, slot by slot, each blended by `along`.
    fn modify(&self, own: &mut Keyframe, from: &str, to: &str, along: f32) {
        let (a, b) = (self.modifiers_of(from), self.modifiers_of(to));
        let mut slots: Vec<&str> = a.iter().map(|(slot, _)| *slot).collect();
        let more: Vec<&str> = (b.iter().map(|(slot, _)| *slot))
            .filter(|slot| !slots.contains(slot))
            .collect();
        slots.extend(more);
        let along = along.clamp(0.0, 1.0);
        for slot in slots {
            // A slot only the weather left fills is laid at what is left of the change; ⚠ one only the next fills, at how far it has come (inferred).
            match (in_slot(&a, slot), in_slot(&b, slot)) {
                (Some(x), None) => self.apply(own, x, 1.0 - along),
                (None, Some(y)) => self.apply(own, y, along),
                // ⚠ The strength where both weathers fill the slot is not read: their values are blended and laid whole.
                (Some(x), Some(y)) => self.apply(own, &blend_modifiers(x, y, along), 1.0),
                (None, None) => {}
            }
        }
    }

    /// Lay the modifier `vars` over `own` at `strength`, each variable as its mode in the executable's table has it; by its name where no table is read.
    fn apply(&self, own: &mut Keyframe, vars: &Vars, strength: f32) {
        if strength <= 0.0 {
            return;
        }
        let Some(table) = &self.executable.variables else {
            apply_modifier(own, vars);
            return;
        };
        for (name, &[first, second]) in vars {
            // A variable the table does not name is one the game's loader finds no place for.
            let Some(entry) = table.index_of(name_hash(name)).map(|i| table.entries[i]) else {
                continue;
            };
            let current = own.get(name).unwrap_or(entry.default);
            let value = match entry.mode {
                // ⚠ Only mode 2 is read. Mode 3 holds the light's colours and strengths, which
                // the viewer measured to move by the second value; mode 6 the level-of-detail
                // scales, which a modifier leaves; the rest are taken as mode 2.
                3 => current + (first - current) * second * strength,
                6 => continue,
                _ => apply_mode2(current, first, strength),
            };
            own.set(name, value);
        }
    }

    /// Give each variable the light chain reads that `own` does not state the default the executable's table has for it.
    fn with_defaults(&self, own: &mut Keyframe) {
        let Some(table) = &self.executable.variables else {
            return;
        };
        for name in LIGHT_VARIABLES {
            if let (None, Some(i)) = (own.get(name), table.index_of(name_hash(name))) {
                own.set(name, table.entries[i].default);
            }
        }
    }

    /// The observer's box and the daylight-saving days, by the names the executable hashes to find them in the visual settings, its own where they state none.
    fn sun_tunables(&self) -> SunTunables {
        let get = |name: &str, default: f64| {
            f64::from(self.visual_settings.get_f32(name, default as f32))
        };
        let image = SunTunables::image_defaults(EAST_X);
        SunTunables {
            south_y: get("sky.latitude.YPosMin", image.south_y),
            north_y: get("sky.latitude.YPosMax", image.north_y),
            south_latitude: get("sky.latitude.rangeMin", image.south_latitude),
            north_latitude: get("sky.latitude.rangeMax", image.north_latitude),
            west_x: get("sky.longitude.XPosMin", image.west_x),
            // ⚠ The executable's key for the east edge is not this name's hash, and its own value is not read.
            east_x: get("sky.longitude.XPosMax", image.east_x),
            west_longitude: get("sky.longitude.rangeMin", image.west_longitude),
            east_longitude: get("sky.longitude.rangeMax", image.east_longitude),
            daylight_start_day: get("sky.daylightsavingstime.start", 79.0) as i32,
            daylight_end_day: get("sky.daylightsavingstime.end", 298.0) as i32,
        }
    }

    /// The moon's orbit, by the names the executable hashes to find it in the visual settings, its own where they state none.
    fn moon_tunables(&self) -> MoonTunables {
        let get = |name: &str, default: f64| {
            f64::from(self.visual_settings.get_f32(name, default as f32))
        };
        let image = MoonTunables::image_defaults();
        MoonTunables {
            rate: get("misc.Moon.Speed", image.rate),
            orbit_angle: get("misc.Sun.Roll", image.orbit_angle),
            orbit_angle_offset: get("misc.Moon.RollOffset", image.orbit_angle_offset),
            height: get("misc.Moon.Wobble.Offset", image.height),
            height_swing: get("misc.Moon.Wobble.Amplitude", image.height_swing),
            height_swing_rate: get("misc.Moon.Wobble.Frequency", image.height_swing_rate),
        }
    }

    /// The two samples the sun towards `sun` lies between and how far along, eased by the curve the first names; `None` unless the time table holds every sample the angles name.
    fn between(&self, sun: Vec3) -> Option<(usize, usize, f32)> {
        let keys = SAMPLE_ANGLES.map(|(angle, sample)| SampleKey {
            angle,
            sample: sample as i32,
        });
        if self.table.samples.len() < keys.len() {
            return None;
        }
        let curves: Vec<usize> = (self.table.samples.iter())
            .map(|s| sample_curve_index(name_hash(&s.interpolation_type)))
            .collect();
        let pick = pick_samples(sample_angle(sun), &keys).ok()?;
        Some((pick.i0 as usize, pick.i1 as usize, eased_t(&pick, &curves)))
    }

    /// The moment at `hour` (0 to 24) in `weather` (a weather type's name, else a cycle's) for a camera `y` metres north on the map; `None` where no cycle is read for it or the time table is not GTA VI's.
    pub fn moment(&self, hour: f32, weather: &str, y: f32) -> Option<Moment> {
        self.moment_in(hour, WeatherBlend::steady(weather), y, None)
    }

    /// The moment at `hour` in `weather` inside a room whose time-cycle modifier and second modifier the hashes `modifiers` name, laid `strength` (0 to 1) of the way over the weather's; the weather's alone where no first modifier of that name is read.
    pub fn moment_inside(
        &self,
        hour: f32,
        weather: &str,
        y: f32,
        modifiers: [u32; 2],
        strength: f32,
    ) -> Option<Moment> {
        self.moment_in(
            hour,
            WeatherBlend::steady(weather),
            y,
            Some((modifiers, strength)),
        )
    }

    /// The moment at `hour` on the way from weather `from` to weather `to`, `along` (0 to 1) of it: their cycles and their modifiers blended, in the GLOBAL region.
    pub fn moment_between(
        &self,
        hour: f32,
        from: &str,
        to: &str,
        along: f32,
        y: f32,
    ) -> Option<Moment> {
        let weathers = WeatherBlend {
            from,
            to,
            along,
            queued: None,
        };
        self.moment_in(hour, weathers, y, None)
    }

    /// The moment at `hour` in `weathers` for a camera `y` metres north, inside a room where `room` names its two modifiers' hashes and how far in it stands (0 to 1).
    pub fn moment_in(
        &self,
        hour: f32,
        weathers: WeatherBlend<'_>,
        y: f32,
        room: Option<([u32; 2], f32)>,
    ) -> Option<Moment> {
        let named = |hash: u32| {
            (self.modifiers.iter()).find(|m| or_io::hash::string_hash_str(&m.name) == hash)
        };
        // ⚠ GTA V's, assumed: each variable the second modifier states stands in for the first's.
        let vars = room.and_then(|(modifiers, _)| {
            named(modifiers[0]).map(|first| {
                let mut vars = first.vars.clone();
                if let Some(second) = named(modifiers[1]) {
                    vars.extend(second.vars.iter().map(|(k, v)| (k.clone(), *v)));
                }
                vars
            })
        });
        let room = (vars.as_ref()).zip(room.map(|(_, strength)| strength.clamp(0.0, 1.0)));
        self.moment_of(hour, weathers, y, room)
    }

    /// [`Self::moment_in`] with the room's variables read.
    fn moment_of(
        &self,
        hour: f32,
        weathers: WeatherBlend<'_>,
        y: f32,
        room: Option<(&Vars, f32)>,
    ) -> Option<Moment> {
        let WeatherBlend {
            from, to, along, ..
        } = weathers;
        let latitude = self.latitude(y);
        // The sun from the clock for an observer at the camera's place north; ⚠ its place east is
        // taken as nought, which moves the sun by under a minute of its day across the map.
        let day = f64::from(hour.rem_euclid(24.0)) / 24.0;
        let observer = Vec3::new(0.0, y, 0.0);
        let noon = &self.executable.noon_minutes;
        let ephemeris = sun_day(
            day,
            DEFAULT_DAY_OF_YEAR,
            noon,
            observer,
            &self.sun_tunables(),
        );
        let round = sample_angle(ephemeris.direction);
        let between = self.between(ephemeris.direction)?;
        let (a, b) = (self.cycle_of(from)?, self.cycle_of(to)?);
        let along = along.clamp(0.0, 1.0);
        let mut own = (self.timecycle).keyframe_between(between, a, b, along, 0.0)?;
        // A queued weather's cycle, sampled at the same pair, is laid over the base by its weight.
        let queued = (weathers.queued)
            .filter(|(_, weight)| *weight > 0.0)
            .and_then(|(name, weight)| Some((name, self.cycle_of(name)?, weight.min(1.0))));
        if let Some((_, cycle, weight)) = queued {
            let over = (self.timecycle).keyframe_between(between, cycle, cycle, 0.0, 0.0)?;
            for (name, value) in over.iter() {
                let base = own.get(name).unwrap_or(value);
                own.set(name, base + (value - base) * weight);
            }
        }
        self.modify(&mut own, from, to, along);
        // ⚠ Inferred: its eight modifiers, in slots of their own at its weight, apply after the base's.
        if let Some((name, _, weight)) = queued {
            for (_, vars) in self.modifiers_of(name) {
                self.apply(&mut own, vars, weight);
            }
        }
        self.with_defaults(&mut own);
        let sun: [f32; 3] = ephemeris.direction.into();
        // The cloud layer's cover and kind are the cycles' high-level values, blended as the weathers are.
        for name in CLOUD_VARS {
            let of = |cycle: &str| {
                (self.high_level.get(&cycle.to_ascii_lowercase())).and_then(|v| v.get(name))
            };
            if let (Some(x), Some(y)) = (of(a), of(b)) {
                let mut value = x + (y - x) * along;
                if let Some(q) = queued.and_then(|(_, cycle, w)| Some((*of(cycle)?, w))) {
                    value += (q.0 - value) * q.1;
                }
                own.set(name, value);
            }
        }
        if let Some((vars, strength)) = room {
            let mut full = own.clone();
            self.apply(&mut full, vars, 1.0);
            // A variable the cycle leaves unstated is the modifier's whole, as a weather's is.
            for name in vars.keys() {
                if let Some(b) = full.get(name) {
                    let a = own.get(name).unwrap_or(b);
                    own.set(name, a + (b - a) * strength);
                }
            }
        }
        let frame_hours = frame_hours(round);
        let visual = &self.visual_settings;
        let degrees = |name: &str| visual.get_f32(name, 0.0).to_radians();
        // The moon turns with the clock's own day, not the sun's bent one; the viewer counts no days.
        let tunables = self.moon_tunables();
        let moon: [f32; 3] = moon_direction(moon_cycle(0, day, tunables.rate), &tunables).into();
        let settings = self.exposure_settings();
        let (light, stops) = light(&own, sun, moon, &settings);
        let lit = bodies(&own, sun, moon, &settings.bodies);
        let rain = |weather: &str| {
            self.data_of(weather)
                .map_or(0.0, |d| d.rain.clamp(0.0, 1.0))
        };
        Some(Moment {
            keyframe: frame_keyframe(&own, &light),
            own,
            table: TimeTable {
                sun_roll: sun_roll(latitude),
                // ⚠ Whether the game still reads these four of its visual settings is not checked.
                moon_roll: degrees("misc.Moon.RollOffset"),
                moon_wobble_frequency: (self.visual_settings)
                    .get_f32("misc.Moon.Wobble.Frequency", 0.0),
                moon_wobble_amplitude: (self.visual_settings)
                    .get_f32("misc.Moon.Wobble.Amplitude", 0.0),
                moon_wobble_offset: (self.visual_settings).get_f32("misc.Moon.Wobble.Offset", 0.0),
                ..TimeTable::default()
            },
            frame_hours,
            sun,
            moon,
            day_phase: ephemeris.day_phase,
            stops,
            light_scale: light.exposure * FRAME_DIFFUSE,
            light,
            moon_dim: visual.get_f32("misc.MoonDimMult", 1.0),
            wetness: {
                let base = rain(from) + (rain(to) - rain(from)) * along;
                queued.map_or(base, |(name, _, w)| base + (rain(name) - base) * w)
            },
            bodies: lit,
        })
    }

    /// What the visual settings state of the exposure.
    pub fn exposure_settings(&self) -> ExposureSettings {
        ExposureSettings::of(&|name, default| self.visual_settings.get_f32(name, default))
    }
}

impl Moment {
    /// This moment exposed by a frame drawn at `previous` stops whose average luminance, in the frame's units, was `metered`: adapted from `previous` over `seconds` towards what its key wants of it.
    pub fn metered(
        &self,
        settings: &ExposureSettings,
        metered: f32,
        previous: f32,
        seconds: f32,
    ) -> Moment {
        // The frame holds every light times its exposure, so its luminance over that is the scene's.
        let luminance = metered / previous.exp2();
        let target = target_stops(&self.own, luminance);
        self.exposed_at(adapt(settings, &self.own, previous, target, seconds))
    }

    /// This moment's light given to the frame at `stops`.
    pub fn exposed_at(&self, stops: f32) -> Moment {
        let light = FrameLight {
            exposure: stops.exp2(),
            ..self.light
        };
        Moment {
            keyframe: frame_keyframe(&self.own, &light),
            stops,
            light_scale: light.exposure * FRAME_DIFFUSE,
            light,
            ..self.clone()
        }
    }
}

/// Read the day's tables from the GTA VI archive at `archive`, the install's common archive: every clear XML body one of the day's documents opens, and every clear plain text; `stop` ends the scan with `Interrupted`.
pub fn read_day(archive: &Path, stop: &dyn Fn() -> bool) -> io::Result<DayTables> {
    let file = File::open(archive)?;
    let mut scan = Rpf9Scan::new(&file, 0..u64::MAX)
        .map_err(io::Error::other)?
        .keeping(HEAD);
    let mut texts = Vec::new();
    while let Some(find) = scan.next() {
        if stop() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let find = find.map_err(io::Error::other)?;
        let wanted = match find.kind {
            Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::Xml => {
                let kept = String::from_utf8_lossy(scan.kept(&find));
                ROOTS.iter().any(|root| kept.contains(root))
            }
            // The visual settings state the name they are told by past the head a scan keeps.
            Rpf9BodyKind::Zlib(run) => run.content == Rpf9Content::Text,
            _ => false,
        };
        if !wanted {
            continue;
        }
        match read_rpf9_body(&file, find.offset, BODY_LIMIT) {
            Ok((_, body)) => texts.push(String::from_utf8_lossy(&body).into_owned()),
            Err(error) => {
                tracing::warn!(offset = find.offset, %error, "a text of the day is not read")
            }
        }
    }
    let mut tables = DayTables::of_texts(texts.iter().map(String::as_str));
    tables.executable = read_executable_day(archive);
    Ok(tables)
}

#[cfg(test)]
#[path = "gta6_day_tests.rs"]
mod tests;
