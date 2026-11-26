use super::*;
use or_renderer::game_shaders::lighting::View;

fn tables() -> Arc<DayTables> {
    tables_with("")
}

/// The tables, their visual settings with `more` after the rest.
fn tables_with(more: &str) -> Arc<DayTables> {
    let samples: String = (0..13)
        .map(|k| format!(r#"<sample name="s{k}" hour="{k}"/>"#))
        .collect();
    let time = format!("<time_data>{samples}</time_data>");
    let cycle = r#"<timecycle_keyframe_data><cycle name="Sunny" regions="1"><region name="GLOBAL">
        <light_dir_mult> 1 1 1 1 1 1 1 1 1 1 1 1 1</light_dir_mult>
        </region></cycle></timecycle_keyframe_data>"#;
    let settings = "Tonemapping.dark.filmic.W 11.2\nExposure.curve.scale -2.0\nemissive.night.start.time 22\nemissive.night.end.time 4\n";
    let settings = format!("{settings}{more}");
    Arc::new(DayTables::of_texts([
        time.as_str(),
        cycle,
        settings.as_str(),
    ]))
}

#[test]
fn the_frames_sun_stands_where_gta_vis_does_at_every_hour_of_its_day() {
    let tables = tables();
    let view = View {
        camera_height: 0.0,
        projection: glam::Mat4::IDENTITY,
        near: 0.1,
    };
    for hour in [7.5f32, 9.0, 13.45, 16.0, 19.0, 23.0, 2.0] {
        let moment = tables
            .moment(hour, "Sunny", 0.0)
            .expect("the cycle is read");
        let day = day_of(&tables, hour, 0.0, WeatherState::Clear, 0.0).expect("a day");
        let (sun, _) = day.lighting(&view).sun_and_moon();
        let own = glam::Vec3::from(moment.sun);
        assert!(
            (sun - own).length() < 1e-3,
            "at {hour}: {sun:?} and {own:?}"
        );
    }
}

#[test]
fn the_day_takes_the_installs_curve_and_night_hours_and_leaves_its_exposure_curve() {
    let tables = tables();
    let day = day_of(&tables, 23.0, 2.0, WeatherState::Rain, 0.0);
    assert!(day.is_none(), "no cycle stands for rain in these tables");
    let day = day_of(&tables, 23.0, 2.0, WeatherState::Clear, 0.0).expect("a day");
    assert_eq!(day.tone.dark.w, 11.2);
    assert_eq!(day.tone.curve_scale, ToneSettings::default().curve_scale);
    assert_eq!(day.night_emissive, 1.0);
    assert_eq!(day.cycle_time, NEAR_FULL_MOON + 2.0 + 23.0 / 24.0);
    let noon = day_of(&tables, 12.0, 0.0, WeatherState::Clear, 0.0).expect("a day");
    assert_eq!(noon.night_emissive, 0.0);
}

#[test]
fn the_frame_maps_the_moment_at_nought_stops_and_the_lights_take_its_exposure() {
    let tables = tables();
    let view = View {
        camera_height: 0.0,
        projection: glam::Mat4::IDENTITY,
        near: 0.1,
    };
    for hour in [13.45f32, 1.0] {
        let day = day_of(&tables, hour, 0.0, WeatherState::Clear, 0.0).expect("a day");
        assert_eq!(day.tone(&day.lighting(&view)).stops, 0.0, "at {hour}");
        let moment = tables.moment(hour, "Sunny", 0.0).expect("a moment");
        let scale = light_scale(Some(&tables), hour, WeatherState::Clear, 0.0);
        assert_eq!(scale, moment.light_scale);
        assert_eq!(scale_of(Some(&moment)), scale);
        let exposed = moment.stops.exp2() / std::f32::consts::PI;
        assert!(
            (scale - exposed).abs() <= 1e-6 * exposed,
            "{scale} {exposed}"
        );
        let once = day_of_moment(
            &tables,
            moment,
            (hour, 0.0),
            Weathers::steady(WeatherState::Clear),
            Vec3::ZERO,
        );
        assert_eq!(once.hours, day.hours, "at {hour}");
    }
    let none = light_scale(None, 12.0, WeatherState::Clear, 0.0);
    assert_eq!(none, NO_DAY_LIGHT_SCALE);
    assert_eq!(scale_of(None), NO_DAY_LIGHT_SCALE);
}

#[test]
fn the_frame_is_exposed_by_its_own_luminance_from_the_first_reading_of_its_own_hour() {
    let tables = tables();
    let at = |metered: Option<&Metered>| {
        let moment = moment(
            &tables,
            9.5,
            Weathers::steady(WeatherState::Clear),
            0.0,
            None,
        )
        .expect("a moment");
        exposed(&tables, moment, (9.5, WeatherState::Clear), 0, metered)
    };
    let unread = moment(
        &tables,
        9.5,
        Weathers::steady(WeatherState::Clear),
        0.0,
        None,
    )
    .expect("a moment");
    // What is read in the first frame of an hour was drawn under the hour before.
    let bright = Metered::of(vec![8.0 / unread.stops.exp2(); 16]);
    let grey = at(Some(&bright));
    assert_eq!(grey.stops, unread.stops);
    // The first reading of its own is taken whole: the key over the scene's luminance.
    let first = at(Some(&bright));
    let scene = 8.0 / grey.stops.exp2();
    let curve = or_title::gta6_exposure::target_stops(&unread.own, scene);
    assert!(
        (first.stops - curve).abs() < 1e-3,
        "{} {curve}",
        first.stops
    );
    assert_eq!(first.light_scale, first.stops.exp2() / std::f32::consts::PI);
    // A frame that reads the same scene keeps its exposure; nothing read holds it.
    let same = Metered::of(vec![scene; 16]);
    assert_eq!(at(Some(&same)).stops, first.stops);
    assert_eq!(at(None).stops, first.stops);
    // A scene four times as bright is moved towards, not jumped to, a moment later.
    let brighter = Metered::of(vec![4.0 * scene; 16]);
    let next = at(Some(&brighter)).stops;
    let wanted = or_title::gta6_exposure::target_stops(&unread.own, 4.0 * scene);
    assert!(next <= first.stops && next > wanted, "{next} {wanted}");
    assert!(exposure_adapting(), "still on its way");
    // A cut of the camera starts again, and its first reading is taken whole.
    let cut = |metered: Option<&Metered>| {
        let moment = moment(
            &tables,
            9.5,
            Weathers::steady(WeatherState::Clear),
            0.0,
            None,
        )
        .expect("a moment");
        exposed(&tables, moment, (9.5, WeatherState::Clear), 1, metered)
    };
    assert_eq!(cut(Some(&brighter)).stops, unread.stops);
    // A scene of one candela a square metre.
    let lit = Metered::of(vec![1.0; 16]);
    let whole = cut(Some(&lit)).stops;
    let curve = (unread.own.get_or("postfx_exposure_key_value", 0.18)).log2();
    assert!(
        whole != unread.stops && (whole - curve).abs() < 1e-3,
        "{whole} {curve}"
    );
    // ⚠ Another test's moment may be on a worker meanwhile, which `adapting` also counts.
    assert!(!exposure_adapting(), "arrived");
}

#[test]
fn a_moment_of_a_new_hour_is_worked_out_at_once_and_one_of_another_place_on_a_worker() {
    // The latitude runs from 20 to 30 degrees over 100 km north.
    let tables = tables_with("sky.latitude.YPosMin -50000\nsky.latitude.YPosMax 50000\nsky.latitude.rangeMin 20\nsky.latitude.rangeMax 30\n");
    let clear = Weathers::steady(WeatherState::Clear);
    let first = moment(&tables, 10.25, clear, 0.0, None).expect("a moment");
    let far = 40_000.0;
    // The moment held stands for the far place while a worker works it out.
    let held = moment(&tables, 10.25, clear, far, None).expect("a moment");
    assert_eq!(held.sun, first.sun);
    let until = Instant::now() + std::time::Duration::from_secs(20);
    while moment_pending() && Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let arrived = moment(&tables, 10.25, clear, far, None).expect("a moment");
    let own = tables.moment(10.25, "Sunny", far).expect("a moment");
    assert_eq!(arrived.sun, own.sun);
    assert_ne!(arrived.sun, first.sun, "the latitude moved the sun");
    // Another hour is worked out at once.
    let later = moment(&tables, 15.5, clear, far, None).expect("a moment");
    assert_eq!(
        later.sun,
        tables.moment(15.5, "Sunny", far).expect("a moment").sun
    );
}

#[test]
fn moments_other_tables_ask_for_never_let_go_of_these_ones() {
    let tables = tables_with("sky.latitude.YPosMin -50000\nsky.latitude.YPosMax 50000\nsky.latitude.rangeMin 20\nsky.latitude.rangeMax 30\n");
    let clear = Weathers::steady(WeatherState::Clear);
    let first = moment(&tables, 10.75, clear, 0.0, None).expect("a moment");
    // As other tests do in the same process, while this one waits.
    let others: Vec<_> = (0..=MOMENTS_HELD).map(|_| super::tests::tables()).collect();
    for other in &others {
        moment(other, 10.75, clear, 0.0, None).expect("a moment");
    }
    let held = moment(&tables, 10.75, clear, 40_000.0, None).expect("a moment");
    assert_eq!(
        held.sun, first.sun,
        "the moment held stands for the far place"
    );
}

#[test]
fn an_even_sky_lights_its_clouds_with_its_own_radiance_and_a_clear_cycle_draws_none() {
    let even = SkyView::of(|_| [2.0, 3.0, 4.0], 1.0);
    let seen = hemisphere_radiance(&even.radiance);
    for (got, want) in seen.iter().zip([2.0, 3.0, 4.0]) {
        assert!((got - want).abs() < 0.02 * want, "{seen:?}");
    }
    let day = day_of(&tables(), 12.0, 0.0, WeatherState::Clear, 0.0).expect("a day");
    assert!(day.sky_view.expect("a table").clouds.is_none());
}

#[test]
fn the_night_glows_by_the_moments_own_air_glow_in_thousandths() {
    let mut tables = tables();
    // ⚠ The last moment is held by the tables' address and the hour: each reading takes an hour of its own.
    let at = |tables: &Arc<DayTables>, hour: f32| {
        let day = day_of(tables, hour, 0.0, WeatherState::Clear, 0.0).expect("a day");
        day.sky_view.expect("a table").night.expect("a night")
    };
    assert_eq!(
        at(&tables, 0.0).fill,
        [0.0; 3],
        "these tables state no glow"
    );
    // A cycle that states the air's glow: its base colour times its intensity, in thousandths.
    // Each value is one a half float holds whole, as the game keeps a cycle's samples.
    for (hour, intensity) in [(1.0, 1.0f32), (2.0, 0.25)] {
        let cycle = format!(
            r#"<timecycle_keyframe_data><cycle name="Sunny" regions="1"><region name="GLOBAL">
            <sky_air_glow_base_color_b>{}</sky_air_glow_base_color_b>
            <sky_air_glow_base_intensity>{}</sky_air_glow_base_intensity>
            </region></cycle></timecycle_keyframe_data>"#,
            " 0.5".repeat(13),
            format!(" {intensity}").repeat(13),
        );
        Arc::make_mut(&mut tables).timecycle = DayTables::of_texts([cycle.as_str()]).timecycle;
        let fill = at(&tables, hour).fill;
        let want = 0.5 * intensity * 0.001;
        assert!((fill[2] - want).abs() < 1e-8, "{fill:?} {want}");
    }
}

#[test]
fn the_day_carries_the_installs_bloom_weights_temporal_times_and_sharpening() {
    let settings = |name: &str, default: f32| match name {
        "bloom.airy.weight2" => 0.712,
        "bloom.airy.weight256" => 0.317,
        "temporalaa.convergenceTimeFar" => 1.0,
        "sharpen.intensity" => 0.15,
        _ => default,
    };
    let mut own = or_io::gta5::data::timecycle::Keyframe::default();
    own.set("postfx_intensity_bloom", 0.1498);
    own.set("postfx_exposure_perpixel_amount", 0.2);
    let post = post_of(&settings, &own);
    assert_eq!(post.bloom_weights[0], 0.712);
    assert_eq!(post.bloom_weights[7], 0.317);
    // A size the settings leave out takes the executable's own weight
    assert_eq!(post.bloom_weights[3], 0.5);
    assert_eq!((post.bloom_mix, post.local.amount), (0.1498, 0.2));
    assert_eq!((post.temporal.far, post.sharpen), (1.0, 0.15));
    // A keyframe that states no compensation curve compensates nothing
    assert!(post.local.compensation.iter().all(|c| *c == 0.0));
}

#[test]
fn every_weather_of_the_bar_names_a_cycle() {
    use WeatherState::*;
    let names = [Clear, Cloudy, Fog, Rain, Storm].map(weather_name);
    assert!(names.iter().all(|name| !name.is_empty()));
    assert_eq!(
        names.iter().collect::<std::collections::HashSet<_>>().len(),
        5
    );
}
