//! The moment of the day the game's passes are lit by: the install's timecycle at the hour and the weather the drivers say.

use std::sync::Arc;

use or_renderer::game_shaders::lighting::{night_emissive, Day};
use or_renderer::game_shaders::tone::ToneSettings;
use or_title::TimecycleData;

use super::RenderBridge;
use crate::state::drivers::WeatherState;
use crate::state::AppState;

/// How far into the moon's cycle the preview stands at the first of its days: near its full.
const NEAR_FULL_MOON: f32 = 13.0;

/// The day `data` states at `hours` of the `days`th day after the first in `weather`, its cycle time as the game counts it: whole days and the fraction of one gone by; none where the install holds no timecycle or not that weather's.
pub(super) fn day_of(
    data: &TimecycleData,
    hours: f32,
    days: f32,
    weather: WeatherState,
) -> Option<Day> {
    let name = weather.cycle_name();
    let keyframe = data.timecycle.keyframe(hours, name, name, 0.0, 0.0)?;
    Some(Day {
        keyframe,
        table: data.timecycle.table.clone(),
        hours,
        cycle_time: NEAR_FULL_MOON + days + hours / 24.0,
        moon_dim: data.visual_settings.get_f32("misc.MoonDimMult", 1.0),
        wetness: weather.wetness(),
        night_emissive: night_emissive(
            hours,
            data.visual_settings
                .get_f32("emissive.night.start.time", 20.0) as u32,
            data.visual_settings.get_f32("emissive.night.end.time", 7.0) as u32,
        ),
        tone: ToneSettings::from_visual(&|name, default| {
            data.visual_settings.get_f32(name, default)
        }),
    })
}

/// The way towards the sun at `hours` of the `days`th day after the first in `weather`, as the game's light of that moment comes from it; none where the install holds no timecycle or not that weather's.
pub(super) fn sun_towards(
    data: &TimecycleData,
    hours: f32,
    days: f32,
    weather: WeatherState,
) -> Option<glam::Vec3> {
    use or_renderer::game_shaders::lighting::View;
    let view = View {
        camera_height: 0.0,
        projection: glam::Mat4::IDENTITY,
        near: 0.1,
    };
    let lighting = day_of(data, hours, days, weather)?.lighting(&view);
    // The light travels away from its source.
    Some(-lighting.directional_light.truncate())
}

/// The built-in sun and sky a GTA VI drawable's own passes are lit by: the hour, the weather and
/// the exposure the drivers say and nothing else, which the install's cycles then tint. The
/// exposure's middle leaves the light as it is, and its ends are a quarter and four times it.
fn gta6_sky(
    hour: f32,
    weather: WeatherState,
    exposure: f32,
) -> or_renderer::game_shaders::meshlets::Sky {
    let overcast = match weather {
        WeatherState::Clear => 0.0,
        WeatherState::Cloudy | WeatherState::Fog => 0.6,
        WeatherState::Rain => 0.8,
        WeatherState::Storm => 0.95,
    };
    let mut sky = or_renderer::game_shaders::meshlets::Sky::at(hour, overcast);
    sky.exposure = 2f32.powf(4.0 * (exposure - 0.5));
    sky
}

impl RenderBridge {
    /// Hand the renderer the day the drivers say, worked out again only when the hour, the weather or the install's timecycle changed.
    pub(super) fn refresh_day(&mut self, state: &AppState) {
        let data = state.clip_regions.timecycle();
        let hour = state.drivers.time_of_day.hour;
        let weather = state.drivers.weather.state;
        let days = state.drivers.time_of_day.days;
        let exposure = state.drivers.exposure.value;
        let tables = crate::state::gta6_timecycle::timecycle(state);
        self.renderer.set_gta6_sky(super::gta6_day::sky(
            tables.as_deref(),
            hour,
            weather,
            |hour| gta6_sky(hour, weather, exposure),
        ));
        let key = (
            Arc::as_ptr(&data) as usize,
            hour.to_bits(),
            days.to_bits(),
            weather,
        );
        if self.day_key == Some(key) {
            return;
        }
        self.day_key = Some(key);
        self.renderer.set_day(day_of(&data, hour, days, weather));
    }

    /// Cast the sun's shadow in Shaded unless the Wardrobe is on screen and its preview has shadows off.
    pub(super) fn refresh_shadows(&mut self, state: &AppState) {
        let wardrobe = state.active_workspace == crate::ui::dock::Workspace::Wardrobe;
        self.renderer
            .set_shadows(!wardrobe || state.workbench.wardrobe.dressed.shadows);
    }

    /// Light the plain modes by the key light the person set in the Wardrobe's preview while that is on screen, and by the one the viewport began with everywhere else; and cast the sun's shadow as that preview says.
    pub(super) fn refresh_key_light(&mut self, state: &AppState) {
        use or_renderer::viewport::{KEY_LIGHT_STRENGTH, KEY_LIGHT_TOWARDS};
        let (towards, strength) =
            match state.active_workspace == crate::ui::dock::Workspace::Wardrobe {
                false => (KEY_LIGHT_TOWARDS, KEY_LIGHT_STRENGTH),
                true => {
                    let light = state.workbench.wardrobe.dressed.light;
                    let sun = light.sun.then(|| {
                        let time = &state.drivers.time_of_day;
                        let data = state.clip_regions.timecycle();
                        sun_towards(&data, time.hour, time.days, state.drivers.weather.state)
                    });
                    (sun.flatten().unwrap_or_else(|| light.way()), light.strength)
                }
            };
        self.renderer.set_key_light(towards, strength);
        self.refresh_shadows(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> TimecycleData {
        let mut data = TimecycleData::default();
        data.timecycle = or_io::gta5::data::timecycle::Timecycle::new(
            r#"<time_data><sample name="a" hour="0"/><sample name="b" hour="12"/></time_data>"#,
        );
        data.timecycle.add_cycles(
            r#"<timecycle_keyframe_data><cycle name="RAIN" regions="1"><region name="GLOBAL">
            <light_dir_mult> 1 3 </light_dir_mult></region></cycle></timecycle_keyframe_data>"#,
        );
        data
    }

    #[test]
    fn the_weather_picks_the_cycle_and_the_hour_the_blend_between_its_samples() {
        let day = day_of(&data(), 6.0, 0.0, WeatherState::Rain).expect("the rain cycle is read");
        assert_eq!(day.keyframe.get("light_dir_mult"), Some(2.0));
        assert_eq!(day.hours, 6.0);
    }

    #[test]
    fn the_cycle_time_counts_the_days_and_the_part_of_the_day_gone_by() {
        let at = |hours: f32, days: f32| {
            day_of(&data(), hours, days, WeatherState::Rain)
                .expect("the rain cycle is read")
                .cycle_time
        };
        assert_eq!(at(0.0, 0.0), NEAR_FULL_MOON);
        assert_eq!(at(12.0, 0.0), NEAR_FULL_MOON + 0.5);
        assert_eq!(at(6.0, 2.0), NEAR_FULL_MOON + 2.25);
    }

    #[test]
    fn the_night_emissives_are_off_at_noon_and_on_at_night_by_the_installs_hours() {
        let at = |hours: f32| {
            day_of(&data(), hours, 0.0, WeatherState::Rain)
                .expect("the rain cycle is read")
                .night_emissive
        };
        assert_eq!((at(12.0), at(23.0), at(3.0)), (0.0, 1.0, 1.0));
    }

    #[test]
    fn the_rain_and_the_storm_wet_the_world_and_the_other_weathers_leave_it_dry() {
        let wet = |weather: WeatherState| weather.wetness();
        assert_eq!(wet(WeatherState::Rain), 1.0);
        assert_eq!(wet(WeatherState::Storm), 1.0);
        for dry in [WeatherState::Clear, WeatherState::Cloudy, WeatherState::Fog] {
            assert_eq!(wet(dry), 0.0, "{dry:?}");
        }
        let day = day_of(&data(), 6.0, 0.0, WeatherState::Rain).expect("the rain cycle is read");
        assert_eq!(day.wetness, 1.0, "the day carries it to the lights");
    }

    #[test]
    fn a_weather_the_install_has_no_cycle_for_gives_no_day() {
        assert!(day_of(&data(), 6.0, 0.0, WeatherState::Clear).is_none());
        assert!(day_of(&TimecycleData::default(), 6.0, 0.0, WeatherState::Rain).is_none());
    }

    #[test]
    fn the_sun_the_wardrobe_is_lit_from_is_a_way_that_moves_with_the_hour_and_is_none_without_a_day(
    ) {
        let towards = |hours| sun_towards(&data(), hours, 0.0, WeatherState::Rain);
        let (morning, evening) = (towards(8.0).expect("a day"), towards(17.0).expect("a day"));
        assert!((morning.length() - 1.0).abs() < 1e-4, "{morning:?}");
        assert!((evening.length() - 1.0).abs() < 1e-4, "{evening:?}");
        assert!(
            (morning - evening).length() > 0.1,
            "the sun is somewhere else at another hour: {morning:?} and {evening:?}"
        );
        assert!(
            morning.z > 0.0 && evening.z > 0.0,
            "the light of the day comes from above the floor"
        );
        assert!(sun_towards(&data(), 8.0, 0.0, WeatherState::Clear).is_none());
        assert!(sun_towards(&TimecycleData::default(), 8.0, 0.0, WeatherState::Rain).is_none());
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn the_wardrobe_holds_its_shadows_back_for_its_own_view_and_every_other_workspace_casts_them() {
        use crate::ui::dock::Workspace;
        const WHAT: &str = "the_wardrobe_holds_its_shadows_back_for_its_own_view";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let mut bridge = RenderBridge::new(
            Arc::new(device),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.workbench.wardrobe.dressed.shadows = false;
        state.active_workspace = Workspace::Model;
        bridge.refresh_key_light(&state);
        assert!(bridge.renderer.shadows(), "another workspace casts them");
        state.active_workspace = Workspace::Wardrobe;
        bridge.refresh_key_light(&state);
        assert!(!bridge.renderer.shadows(), "the preview's own choice");
        state.workbench.wardrobe.dressed.shadows = true;
        bridge.refresh_key_light(&state);
        assert!(bridge.renderer.shadows());
    }
}
