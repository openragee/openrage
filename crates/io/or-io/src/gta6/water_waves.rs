//! The presets of GTA VI's ocean simulation: for each kind of water, a curve a wave octave's strength
//! follows, and the place the preset was set up at.

use super::WaterError;
use crate::gta5::data::timecycle::{attr, tokens, Token};

/// One preset of `OceanSimPresets`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OceanPreset {
    pub name: String,
    pub id: f32,
    /// Each octave's control points, ascending in their first value, each a place along the curve and the strength there.
    pub octaves: Vec<Vec<[f32; 2]>>,
    /// Where the preset was set up, in metres.
    pub location: [f32; 3],
}

impl OceanPreset {
    /// The strength octave `octave` takes at `x` along its curve, between its control points by a straight line and held past its ends; 1 where the preset states no such octave.
    pub fn strength(&self, octave: usize, x: f32) -> f32 {
        let Some(points) = self.octaves.get(octave).filter(|p| !p.is_empty()) else {
            return 1.0;
        };
        let after = points.partition_point(|p| p[0] <= x);
        match (after.checked_sub(1).map(|i| points[i]), points.get(after)) {
            (Some(a), Some(b)) if b[0] > a[0] => a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]),
            (Some(a), _) => a[1],
            (None, Some(b)) => b[1],
            (None, None) => 1.0,
        }
    }
}

/// The document `OceanSimPresets`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OceanPresets {
    pub presets: Vec<OceanPreset>,
}

impl OceanPresets {
    pub fn parse(text: &str) -> Result<Self, WaterError> {
        let mut out = Self::default();
        let mut open: Vec<&str> = Vec::new();
        let number = |attrs: &str, key: &str| attr(attrs, key).and_then(|v| v.trim().parse().ok());
        for token in tokens(text) {
            match token {
                Token::Open { name, attrs, empty } => {
                    if open.is_empty() && name != "OceanSimPresets" {
                        return Err(WaterError::Broken(format!(
                            "the ocean's presets have the root `{name}`"
                        )));
                    }
                    let parent = open.last().copied();
                    let preset = out.presets.last_mut();
                    match (name, parent, preset) {
                        ("Item", Some("Presets"), _) => out.presets.push(OceanPreset::default()),
                        ("Item", Some("Octaves"), Some(p)) => p.octaves.push(Vec::new()),
                        ("Id", _, Some(p)) => p.id = number(attrs, "value").unwrap_or(0.0),
                        ("Location", _, Some(p)) => {
                            p.location = ["x", "y", "z"].map(|k| number(attrs, k).unwrap_or(0.0))
                        }
                        _ => {}
                    }
                    if !empty {
                        open.push(name);
                    }
                }
                Token::Close(_) => {
                    open.pop();
                }
                Token::Text(text) => {
                    let Some(preset) = out.presets.last_mut() else {
                        continue;
                    };
                    match open.last().copied() {
                        Some("Name") => preset.name = text.to_string(),
                        Some("ControlPoints") => {
                            let values: Vec<f32> = text
                                .split_whitespace()
                                .filter_map(|v| v.parse().ok())
                                .collect();
                            if let Some(octave) = preset.octaves.last_mut() {
                                octave.extend(values.as_chunks::<2>().0.iter().copied());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if out.presets.is_empty() {
            return Err(WaterError::Broken("the ocean's presets state none".into()));
        }
        Ok(out)
    }

    /// The preset set up nearest to `x`, `y`.
    pub fn nearest(&self, x: f32, y: f32) -> Option<&OceanPreset> {
        let away = |p: &&OceanPreset| (p.location[0] - x).powi(2) + (p.location[1] - y).powi(2);
        self.presets
            .iter()
            .min_by(|a, b| away(a).total_cmp(&away(b)))
    }
}

/// One keyframe of a cascade of `WaveData`: its wind in metres a second and the side of its patch in metres; nought where it states neither.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaveKey {
    pub wind_speed: f32,
    pub size: f32,
}

/// The document `WaveData`, one cascade of the ocean simulation: a keyframe for each Beaufort number from nought.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WaveParams {
    pub keys: Vec<WaveKey>,
}

impl WaveParams {
    pub fn parse(text: &str) -> Result<Self, WaterError> {
        let mut out = Self::default();
        let mut open: Vec<&str> = Vec::new();
        for token in tokens(text) {
            match token {
                Token::Open { name, attrs, empty } => {
                    if open.is_empty() && name != "WaveData" {
                        return Err(WaterError::Broken(format!(
                            "the wave parameters have the root `{name}`"
                        )));
                    }
                    let value = || attr(attrs, "value").and_then(|v| v.trim().parse().ok());
                    match (name, open.last().copied(), out.keys.last_mut()) {
                        ("Item", Some("Params"), _) => out.keys.push(WaveKey::default()),
                        ("fWindSpeed", Some("Item"), Some(k)) => {
                            k.wind_speed = value().unwrap_or(0.0)
                        }
                        ("Lx", Some("Item"), Some(k)) => k.size = value().unwrap_or(0.0),
                        _ => {}
                    }
                    if !empty {
                        open.push(name);
                    }
                }
                Token::Close(_) => {
                    open.pop();
                }
                Token::Text(_) => {}
            }
        }
        if out.keys.is_empty() {
            return Err(WaterError::Broken(
                "the wave parameters state no keyframe".into(),
            ));
        }
        Ok(out)
    }

    /// The keyframe at Beaufort `beaufort`, held to nought and twelve and between its two keyframes by a straight line, as the game reads it.
    pub fn at(&self, beaufort: f32) -> WaveKey {
        let last = self.keys.len().saturating_sub(1);
        let x = beaufort.clamp(0.0, 12.0).min(last as f32);
        let (low, high) = (x.floor() as usize, x.ceil() as usize);
        let (a, b) = (self.keys.get(low).copied(), self.keys.get(high).copied());
        let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
        let t = x - low as f32;
        WaveKey {
            wind_speed: a.wind_speed + (b.wind_speed - a.wind_speed) * t,
            size: a.size + (b.size - a.size) * t,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wave_parameters_read_by_keyframe_and_are_read_between_them() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<WaveData>
  <Params>
    <Item><nWindAlignment value="2" /><fWindSpeed value="1.0" /><Lx value="800.0" /></Item>
    <Item><fWindSpeed value="3.0" /><Lx value="800.0" /><fChoppiness value="2.0" /></Item>
    <Item><Lx value="110.0" /></Item>
  </Params>
</WaveData>"#;
        let params = WaveParams::parse(text).expect("reads");
        assert_eq!(params.keys.len(), 3);
        assert_eq!(
            params.keys[1],
            WaveKey {
                wind_speed: 3.0,
                size: 800.0
            }
        );
        assert_eq!(
            params.keys[2].wind_speed, 0.0,
            "a keyframe that states no wind"
        );
        assert_eq!(params.at(0.5).wind_speed, 2.0);
        assert_eq!(params.at(-1.0).wind_speed, 1.0);
        assert_eq!(params.at(9.0).size, 110.0, "held at the last keyframe");
        assert!(WaveParams::parse("<Other></Other>").is_err());
    }

    const TEXT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<OceanSimPresets>
  <Presets>
    <Item>
      <Name>Open</Name>
      <Id value="1.000000" />
      <Octaves>
        <Item>
          <ControlPoints content="vector2_array">
            0.0	0.5
            0.5	0.25
            1.0	1.0
          </ControlPoints>
        </Item>
        <Item>
          <ControlPoints content="vector2_array">
            0.0	1.0
            1.0	1.0
          </ControlPoints>
        </Item>
      </Octaves>
      <Location x="100.0" y="0.0" z="2.0" />
      <Direction x="-20.0" y="-0.0" z="130.0" />
    </Item>
    <Item>
      <Name>Lake</Name>
      <Id value="0.5" />
      <Octaves></Octaves>
      <Location x="-100.0" y="50.0" z="9.0" />
    </Item>
  </Presets>
</OceanSimPresets>"#;

    #[test]
    fn the_presets_read_with_their_curves_and_places_and_the_nearest_is_found() {
        let presets = OceanPresets::parse(TEXT).expect("reads");
        assert_eq!(presets.presets.len(), 2);
        let open = &presets.presets[0];
        assert_eq!((open.name.as_str(), open.id), ("Open", 1.0));
        assert_eq!(open.octaves[0], vec![[0.0, 0.5], [0.5, 0.25], [1.0, 1.0]]);
        assert_eq!(
            open.location,
            [100.0, 0.0, 2.0],
            "not the Direction after it"
        );
        assert_eq!(open.strength(0, 0.25), 0.375);
        assert_eq!(open.strength(0, 0.75), 0.625);
        assert_eq!((open.strength(0, -1.0), open.strength(0, 2.0)), (0.5, 1.0));
        assert_eq!(open.strength(2, 0.5), 1.0, "an octave not stated");
        assert_eq!(presets.presets[1].strength(0, 0.5), 1.0);
        assert_eq!(
            presets.nearest(90.0, 10.0).map(|p| p.name.as_str()),
            Some("Open")
        );
        assert_eq!(
            presets.nearest(-50.0, 40.0).map(|p| p.name.as_str()),
            Some("Lake")
        );
        assert!(OceanPresets::parse("<Other></Other>").is_err());
    }
}
