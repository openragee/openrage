//! What a GTA VI install states of a weather beside its cycles: the type a weather's name stands
//! for, the cycle that type's data names, and the modifiers the type lays over it.

use std::collections::BTreeMap;

/// The modifiers a weather type names, in the order their elements stand in it.
const MODIFIER_TAGS: [&str; 8] = [
    "ModAtmosphere",
    "ModPrecipitation",
    "ModTemperature",
    "ModHumidity",
    "ModWind",
    "ModFetch",
    "ModMisc",
    "ModNiceness",
];

/// The weather types of an install and the modifiers they name, every name lower-cased.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gta6Weather {
    /// A name a weather is asked by, and the type that stands for it.
    indirections: BTreeMap<String, String>,
    /// A type: its data set and its modifiers.
    types: BTreeMap<String, (String, Vec<String>)>,
    /// A data set, and the cycle it names.
    cycles: BTreeMap<String, String>,
    /// A modifier: each variable it states with the first of its numbers.
    modifiers: BTreeMap<String, Vec<(String, f32)>>,
}

/// The text between `<tag>` and `</tag>` in `block`, trimmed.
fn text_of<'a>(block: &'a str, tag: &str) -> Option<&'a str> {
    let from = block.find(&format!("<{tag}>"))? + tag.len() + 2;
    let to = block[from..].find(&format!("</{tag}"))?;
    Some(block[from..from + to].trim())
}

/// The `<Item>` blocks of the element `section` of `text`.
fn items<'a>(text: &'a str, section: &str) -> impl Iterator<Item = &'a str> {
    let body = text_of(text, section).unwrap_or("");
    body.split("<Item>").skip(1)
}

impl Gta6Weather {
    /// Whether nothing was read.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Add what the weather set `text` states: the data sets' cycles, the types and the
    /// indirections.
    pub fn add_types(&mut self, text: &str) {
        let lower = |block: &str, tag: &str| text_of(block, tag).map(str::to_ascii_lowercase);
        for item in items(text, "WeatherTypeDatas") {
            if let (Some(name), Some(cycle)) = (lower(item, "Name"), lower(item, "TimeCycleName")) {
                self.cycles.insert(name, cycle);
            }
        }
        for item in items(text, "WeatherTypes") {
            let (Some(name), Some(data)) = (lower(item, "Name"), lower(item, "DataName")) else {
                continue;
            };
            let modifiers = (MODIFIER_TAGS.iter())
                .filter_map(|tag| lower(item, tag))
                .collect();
            self.types.insert(name, (data, modifiers));
        }
        for item in items(text, "WeatherTypeIndirectionList") {
            if let (Some(from), Some(to)) = (lower(item, "From"), lower(item, "To")) {
                self.indirections.insert(from, to);
            }
        }
    }

    /// Add the modifiers of the modifier set `text`.
    pub fn add_modifiers(&mut self, text: &str) {
        for block in text.split("<modifier").skip(1) {
            let Some(name) = (block.split_once("name=\""))
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(name, _)| name.to_ascii_lowercase())
            else {
                continue;
            };
            let body = block.split_once('>').map_or("", |(_, body)| body);
            let mut variables = Vec::new();
            for element in body.split('<').skip(1) {
                let Some((tag, value)) = element.split_once('>') else {
                    continue;
                };
                let first = value.split_whitespace().next().and_then(|n| n.parse().ok());
                if let (false, Some(first)) = (tag.starts_with('/'), first) {
                    variables.push((tag.trim().to_string(), first));
                }
            }
            self.modifiers.insert(name, variables);
        }
    }

    /// The cycle a weather asked by `name` is drawn with, and every variable its type's
    /// modifiers state, a later modifier's over an earlier one's; `None` where the install names
    /// no such weather.
    pub fn resolve(&self, name: &str) -> Option<(&str, Vec<(&str, f32)>)> {
        let name = name.to_ascii_lowercase();
        let kind = self.indirections.get(&name).unwrap_or(&name);
        let (data, modifiers) = self.types.get(kind)?;
        let cycle = self.cycles.get(data)?;
        let stated = (modifiers.iter())
            .filter_map(|modifier| self.modifiers.get(modifier))
            .flatten()
            .map(|(variable, value)| (variable.as_str(), *value))
            .collect();
        Some((cycle, stated))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPES: &str = r#"<CContentsOfWeatherXmlFile>
        <WeatherTypeDatas>
          <Item><Name>Plain</Name><TimeCycleName>PLAIN</TimeCycleName></Item>
          <Item><Name>LAYERED</Name><Sun value="1"/><TimeCycleName>Layered_Cycle</TimeCycleName></Item>
        </WeatherTypeDatas>
        <WeatherTypes>
          <Item><Name>PLAIN</Name><DataName>Plain</DataName><ModAtmosphere /></Item>
          <Item><Name>AM_Plain</Name><DataName>LAYERED</DataName>
            <ModAtmosphere>M_air</ModAtmosphere><ModWind>M_wind</ModWind><ModMisc>M_none</ModMisc></Item>
        </WeatherTypes>
        <WeatherTypeIndirectionList>
          <Item><From>plain</From><To>AM_plain</To></Item>
        </WeatherTypeIndirectionList>
        </CContentsOfWeatherXmlFile>"#;
    const MODIFIERS: &str = r#"<timecycle_modifier_data>
        <modifier name="--- a heading ---" userFlags="0" />
        <modifier name="M_air" userFlags="0">
          <fog_density>0.250000 0.000000</fog_density>
          <sun_mult>2.000000 1.000000</sun_mult>
        </modifier>
        <modifier name="M_wind"><sun_mult>3.000000 0.000000</sun_mult></modifier>
        </timecycle_modifier_data>"#;

    #[test]
    fn a_weather_is_its_indirection_s_type_that_type_s_cycle_and_its_modifiers_in_order() {
        let mut weather = Gta6Weather::default();
        assert!(weather.is_empty() && weather.resolve("plain").is_none());
        weather.add_types(TYPES);
        weather.add_modifiers(MODIFIERS);
        let (cycle, stated) = weather.resolve("Plain").expect("an indirection names it");
        assert_eq!(cycle, "layered_cycle");
        assert_eq!(
            stated,
            [("fog_density", 0.25), ("sun_mult", 2.0), ("sun_mult", 3.0)],
            "the first number of each, the later modifier last; a modifier no set holds is passed over"
        );
        // A type no indirection leads to is asked by its own name.
        assert_eq!(
            weather.resolve("am_PLAIN").map(|(cycle, _)| cycle),
            Some("layered_cycle")
        );
        assert!(weather.resolve("storm").is_none());
    }
}
