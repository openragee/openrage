use or_io::gta5::data::timecycle::Timecycle;
use or_io::gta5::data::visual_settings::VisualSettings;

use super::gta6_weather::Gta6Weather;
use crate::Vfs;

pub const TIME_VFS_PATH: &str = "common:/data/levels/gta5/time.xml";
pub const VISUAL_SETTINGS_VFS_PATH: &str = "common:/data/visualsettings.dat";

/// The weather cycles an install ships, each in a file of its own beside the others.
const CYCLES: [&str; 14] = [
    "extrasunny",
    "clear",
    "clearing",
    "clouds",
    "foggy",
    "neutral",
    "overcast",
    "rain",
    "smog",
    "snow",
    "snowlight",
    "thunder",
    "xmas",
    "blizzard",
];

/// What an install says of the look of the day: its time table and weather cycles, and its tuning values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimecycleData {
    pub timecycle: Timecycle,
    pub visual_settings: VisualSettings,
    /// The weather types and modifiers a GTA VI install states; empty for any other.
    pub gta6_weather: Gta6Weather,
}

/// Read the time table, the weather cycles and the visual settings the install holds; whatever it does not hold is left empty.
pub fn load_timecycle(vfs: &Vfs) -> TimecycleData {
    let text = |path: &str| {
        vfs.open(path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    };
    let Some(time) = text(TIME_VFS_PATH) else {
        tracing::debug!(path = TIME_VFS_PATH, "no time table in the install");
        return TimecycleData::default();
    };
    let mut timecycle = Timecycle::new(&time);
    for name in CYCLES {
        match text(&format!("common:/data/timecycle/w_{name}.xml")) {
            Some(cycle) => timecycle.add_cycles(&cycle),
            None => tracing::debug!(cycle = name, "no weather cycle of this name"),
        }
    }
    let visual_settings = text(VISUAL_SETTINGS_VFS_PATH)
        .map(|text| VisualSettings::parse(&text))
        .unwrap_or_default();
    TimecycleData {
        timecycle,
        visual_settings,
        gta6_weather: Gta6Weather::default(),
    }
}

/// The look of the day the XML texts `documents` state, for an install whose files have no names: each is known by its root element, and a text of neither kind is passed over.
pub fn timecycle_of_documents<'a>(documents: impl IntoIterator<Item = &'a str>) -> TimecycleData {
    let mut data = TimecycleData::default();
    for text in documents {
        // The root is the first element after the declaration; the tuning values are no XML
        // and say what they are in a comment.
        let head = String::from_utf8_lossy(&text.as_bytes()[..text.len().min(512)]);
        if head.contains("<time_data") {
            data.timecycle.table = or_io::gta5::data::timecycle::TimeTable::parse(text);
        } else if head.contains("<timecycle_keyframe_data") {
            data.timecycle.add_cycles(text);
        } else if head.contains("<CContentsOfWeatherXmlFile") {
            data.gta6_weather.add_types(text);
        } else if head.contains("<timecycle_modifier_data") {
            data.gta6_weather.add_modifiers(text);
        } else if head.contains("# Visual Configuration Table") {
            data.visual_settings = VisualSettings::parse(text);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::FileDevice;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Debug)]
    struct CannedDevice {
        files: HashMap<String, Vec<u8>>,
    }

    impl FileDevice for CannedDevice {
        fn open(&self, relative_path: &str) -> Option<Vec<u8>> {
            self.files
                .get(&relative_path.trim_start_matches('/').to_ascii_lowercase())
                .cloned()
        }
    }

    fn vfs_of(files: &[(&str, &str)]) -> Vfs {
        let files = files
            .iter()
            .map(|(path, text)| (path.to_string(), text.as_bytes().to_vec()))
            .collect();
        let mut vfs = Vfs::new();
        vfs.mount("common:", Arc::new(CannedDevice { files }));
        vfs
    }

    const TIME: &str =
        r#"<time_data><sample name="a" hour="0"/><sample name="b" hour="12"/></time_data>"#;
    const CYCLE: &str = r#"<timecycle_keyframe_data><cycle name="CLEAR" regions="1">
        <region name="GLOBAL"><fog_start> 1 2 </fog_start></region></cycle></timecycle_keyframe_data>"#;

    #[test]
    fn the_cycles_the_install_holds_are_read_and_the_ones_it_lacks_are_left_out() {
        let vfs = vfs_of(&[
            ("data/levels/gta5/time.xml", TIME),
            ("data/timecycle/w_clear.xml", CYCLE),
            ("data/visualsettings.dat", "misc.MoonDimMult 0.3\n"),
        ]);
        let data = load_timecycle(&vfs);
        assert_eq!(data.timecycle.table.samples.len(), 2);
        assert_eq!(data.timecycle.cycles.len(), 1);
        assert!(data.timecycle.cycle("clear").is_some());
        assert_eq!(data.visual_settings.get_f32("misc.MoonDimMult", 1.0), 0.3);
    }

    #[test]
    fn nameless_documents_are_told_apart_by_their_roots_in_any_order() {
        // A cycle whose file opens with values of its own before the cycle, and whose samples
        // state no duration.
        let cycle = r#"<?xml version="1.0" encoding="UTF-8"?>
<timecycle_keyframe_data>
  <summary><overall> 0.5</overall></summary>
  <cycle name="Rain" regions="2">
    <region name="GLOBAL"><light_dir_mult> 1 3 5</light_dir_mult></region>
    <region name="URBAN"><light_dir_mult> 2 2 2</light_dir_mult></region>
  </cycle>
</timecycle_keyframe_data>"#;
        let time = r#"<?xml version="1.0" encoding="UTF-8"?>
<time_data version="1.000000" mode="dynamic">
  <sample name="00:00" hour="0" uw_tc_mod="underwater"/>
  <sample name="07:00" hour="7" uw_tc_mod="underwater" interpolation_type="EASE_QUARTIC_INOUT"/>
  <sample name="12:00" hour="12" uw_tc_mod="underwater"/>
  <region name="URBAN"/>
</time_data>"#;
        let other = r#"<?xml version="1.0"?><some_other_table/>"#;
        let data = timecycle_of_documents([cycle, other, time]);
        let hours: Vec<f32> = (data.timecycle.table.samples.iter())
            .map(|s| s.hour)
            .collect();
        assert_eq!(hours, [0.0, 7.0, 12.0]);
        use or_io::gta5::data::timecycle::Easing;
        let bends: Vec<Easing> = (data.timecycle.table.samples.iter())
            .map(|s| s.easing)
            .collect();
        assert_eq!(
            bends,
            [Easing::Linear, Easing::QuarticInOut, Easing::Linear]
        );
        assert_eq!(data.timecycle.cycles.len(), 1);
        let rain = data.timecycle.cycle("RAIN").expect("named in its file");
        assert_eq!(rain.regions.len(), 2);
        assert!(
            !rain.regions[0].vars.contains_key("overall"),
            "what stands before the cycle is none of its variables"
        );
        assert_eq!(rain.overall.get("overall"), Some(&0.5));
        let between = (data.timecycle).keyframe_between((1, 2, 0.5), "rain", "rain", 0.0, 0.0);
        assert_eq!(between.expect("read").get("light_dir_mult"), Some(4.0));
        assert_eq!(timecycle_of_documents([other]), TimecycleData::default());
    }

    #[test]
    fn an_install_with_no_time_table_gives_nothing_to_light_the_day_with() {
        let data = load_timecycle(&vfs_of(&[("data/timecycle/w_clear.xml", CYCLE)]));
        assert_eq!(data, TimecycleData::default());
    }
}
