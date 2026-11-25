//! The time-of-day tables of an install: the hours the day is sampled at, and for each weather cycle the value of every variable at each of them.

use std::collections::BTreeMap;

/// Degrees to radians, which the angles of `time.xml` are read through.
const DEGREES: f32 = std::f32::consts::PI / 180.0;

/// How the way from one sample to the next is bent: a sample of GTA VI names it, and GTA V's go straight.
// The other kinds a sample may name are not read here, and go straight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Easing {
    #[default]
    Linear,
    QuarticInOut,
}

impl Easing {
    fn named(name: &str) -> Self {
        match name.trim().eq_ignore_ascii_case("EASE_QUARTIC_INOUT") {
            true => Self::QuarticInOut,
            false => Self::Linear,
        }
    }

    /// How far along `along` is once bent, both from nought to one.
    pub fn ease(self, along: f32) -> f32 {
        match self {
            Self::Linear => along,
            Self::QuarticInOut if along < 0.5 => 8.0 * along.powi(4),
            Self::QuarticInOut => 1.0 - 0.5 * (2.0 * along - 2.0).powi(4),
        }
    }
}

/// One hour the day is sampled at: the value holds for `duration` hours, then blends to the next sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSample {
    pub hour: f32,
    pub duration: f32,
    /// How the blend from this sample to the next is bent.
    pub easing: Easing,
}

/// What `time.xml` states of the day: its samples, and where the sun and moon travel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimeTable {
    pub samples: Vec<TimeSample>,
    /// How far the sun's path leans, in radians.
    pub sun_roll: f32,
    pub sun_yaw: f32,
    /// How far the moon's path is turned from the sun's, in radians.
    pub moon_roll: f32,
    pub moon_wobble_frequency: f32,
    pub moon_wobble_amplitude: f32,
    pub moon_wobble_offset: f32,
}

/// The variables of one region of a cycle: each has a value at every sample of the day.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Region {
    pub name: String,
    pub vars: BTreeMap<String, Vec<f32>>,
}

/// One weather: a region for the world at large, and others that a place can blend in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cycle {
    pub name: String,
    pub regions: Vec<Region>,
    /// The single values a file of GTA VI states of the weather as a whole, before the cycle.
    pub overall: BTreeMap<String, f32>,
}

/// The value of each variable at one moment, by the name its file gives it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keyframe(BTreeMap<String, f32>);

impl Keyframe {
    pub fn get(&self, name: &str) -> Option<f32> {
        self.0.get(name).copied()
    }

    /// The value of `name`, or `default` where the cycles do not state it.
    pub fn get_or(&self, name: &str, default: f32) -> f32 {
        self.get(name).unwrap_or(default)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn set(&mut self, name: &str, value: f32) {
        self.0.insert(name.to_string(), value);
    }
}

/// The day's table and every weather cycle read from an install.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timecycle {
    pub table: TimeTable,
    pub cycles: Vec<Cycle>,
}

impl TimeTable {
    pub fn parse(text: &str) -> Self {
        let mut table = Self::default();
        for token in tokens(text) {
            let Token::Open { name, attrs, .. } = token else {
                continue;
            };
            let number = |key: &str| attr(attrs, key).and_then(|v| v.trim().parse::<f32>().ok());
            match name {
                "suninfo" => {
                    table.sun_roll = number("sun_roll").unwrap_or(0.0) * DEGREES;
                    table.sun_yaw = number("sun_yaw").unwrap_or(0.0) * DEGREES;
                }
                "mooninfo" => {
                    table.moon_roll = number("moon_roll").unwrap_or(0.0) * DEGREES;
                    table.moon_wobble_frequency = number("moon_wobble_freq").unwrap_or(0.0);
                    table.moon_wobble_amplitude = number("moon_wobble_amp").unwrap_or(0.0);
                    table.moon_wobble_offset = number("moon_wobble_offset").unwrap_or(0.0);
                }
                "sample" => {
                    if let Some(hour) = number("hour") {
                        table.samples.push(TimeSample {
                            hour,
                            duration: number("duration").unwrap_or(0.0),
                            easing: attr(attrs, "interpolation_type")
                                .map(Easing::named)
                                .unwrap_or_default(),
                        });
                    }
                }
                _ => {}
            }
        }
        table
    }

    /// The two samples `hours` lies between and how far along from the first to the second it is, or `None` with no samples.
    pub fn locate(&self, hours: f32) -> Option<(usize, usize, f32)> {
        let count = self.samples.len();
        if count == 0 {
            return None;
        }
        // The day is out of range after a mission, and the last sample is a day before the first
        let now = hours.clamp(0.0, 23.999);
        let mut times = Vec::with_capacity(count + 2);
        let mut holds = Vec::with_capacity(count + 2);
        times.push(self.samples[count - 1].hour - 24.0);
        holds.push(self.samples[count - 1].duration);
        for sample in &self.samples {
            times.push(sample.hour);
            holds.push(sample.duration);
        }
        times.push(self.samples[0].hour + 24.0);
        holds.push(self.samples[0].duration);

        let mut before = count + 1;
        while before > 0 && now < times[before] {
            before -= 1;
        }
        let before = before.min(count);
        let first = if before == 0 { count - 1 } else { before - 1 };
        let second = if before == count { 0 } else { before };
        let from = times[before] + holds[before];
        let span = times[before + 1] - from;
        let along = if span > 0.0 {
            ((now - from) / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Some((first, second, along))
    }
}

impl Cycle {
    /// Add the cycles `text` holds to `out`.
    fn parse_into(text: &str, out: &mut Vec<Cycle>) {
        let mut open: Option<(String, String)> = None;
        let mut cycle: Option<Cycle> = None;
        let mut overall = BTreeMap::new();
        for token in tokens(text) {
            match token {
                Token::Open { name, attrs, empty } => match name {
                    "cycle" => {
                        cycle = Some(Cycle {
                            name: attr(attrs, "name").unwrap_or_default().to_string(),
                            regions: Vec::new(),
                            overall: std::mem::take(&mut overall),
                        });
                    }
                    "region" => {
                        if let Some(cycle) = cycle.as_mut() {
                            cycle.regions.push(Region {
                                name: attr(attrs, "name").unwrap_or_default().to_string(),
                                vars: BTreeMap::new(),
                            });
                        }
                    }
                    other if !empty => {
                        open = Some((other.to_string(), String::new()));
                    }
                    _ => {}
                },
                Token::Text(text) => {
                    if let Some((_, body)) = open.as_mut() {
                        body.push(' ');
                        body.push_str(text);
                    }
                }
                Token::Close(name) => match name {
                    "cycle" => out.extend(cycle.take()),
                    "region" => {}
                    _ => {
                        let Some((tag, body)) = open.take() else {
                            continue;
                        };
                        let values: Option<Vec<f32>> = body
                            .split_whitespace()
                            .map(|v| v.parse::<f32>().ok())
                            .collect();
                        let (Some(values), true) = (values, tag == name && !body.trim().is_empty())
                        else {
                            continue;
                        };
                        match cycle.as_mut().map(|c| c.regions.last_mut()) {
                            Some(Some(region)) => {
                                region.vars.insert(tag, values);
                            }
                            Some(None) => {}
                            // Before the cycle a value stands alone, with no sample to its name.
                            None => {
                                if let [value] = values[..] {
                                    overall.insert(tag, value);
                                }
                            }
                        }
                    }
                },
            }
        }
    }

    fn region_value(&self, region: usize, name: &str, sample: usize) -> Option<f32> {
        let values = self.regions.get(region)?.vars.get(name)?;
        values
            .get(sample.min(values.len().saturating_sub(1)))
            .copied()
    }
}

impl Timecycle {
    /// The table `time_xml` states, with no cycles yet.
    pub fn new(time_xml: &str) -> Self {
        Self {
            table: TimeTable::parse(time_xml),
            cycles: Vec::new(),
        }
    }

    /// Add the cycles `text` holds, the file of one weather.
    pub fn add_cycles(&mut self, text: &str) {
        Cycle::parse_into(text, &mut self.cycles);
    }

    pub fn cycle(&self, name: &str) -> Option<&Cycle> {
        self.cycles
            .iter()
            .find(|cycle| cycle.name.eq_ignore_ascii_case(name))
    }

    /// The value of every variable at `hours`, blending weather `from` into weather `to` by `weather` (0 to 1) and the urban region in by `urban` (0 to 1); `None` where a cycle is not read or the day has no samples.
    pub fn keyframe(
        &self,
        hours: f32,
        from: &str,
        to: &str,
        weather: f32,
        urban: f32,
    ) -> Option<Keyframe> {
        self.keyframe_between(self.table.locate(hours)?, from, to, weather, urban)
    }

    /// The value of every variable `along` the way from sample `first` to sample `second`, for a day whose samples are not found by the hour; the weathers and the urban region blend as in [`Self::keyframe`].
    pub fn keyframe_between(
        &self,
        (first, second, along): (usize, usize, f32),
        from: &str,
        to: &str,
        weather: f32,
        urban: f32,
    ) -> Option<Keyframe> {
        let (a, b) = (self.cycle(from)?, self.cycle(to)?);
        let weather = weather.clamp(0.0, 1.0);
        let urban = urban.clamp(0.0, 1.0);
        // Within a city the second region is the whole picture, and elsewhere the first is
        let base = usize::from(urban >= 1.0 && a.regions.len() > 1);
        let mut frame = blend_region(a, b, base, first, second, weather, along);
        for region in base + 1..a.regions.len().max(b.regions.len()) {
            if region == 1 && urban > 0.0 {
                let other = blend_region(a, b, region, first, second, weather, along);
                for (name, value) in &other.0 {
                    let base = frame.get(name).unwrap_or(*value);
                    frame.set(name, base + (value - base) * urban);
                }
            }
        }
        Some(frame)
    }
}

/// Each variable of `region` at the moment between samples `first` and `second` of cycles `a` and `b`.
fn blend_region(
    a: &Cycle,
    b: &Cycle,
    region: usize,
    first: usize,
    second: usize,
    weather: f32,
    along: f32,
) -> Keyframe {
    let lerp = |x: f32, y: f32, t: f32| x + (y - x) * t;
    let mut names: Vec<&String> = Vec::new();
    for cycle in [a, b] {
        if let Some(region) = cycle.regions.get(region) {
            names.extend(region.vars.keys());
        }
    }
    let mut frame = Keyframe::default();
    for name in names {
        let at = |sample: usize| {
            let (x, y) = (
                a.region_value(region, name, sample),
                b.region_value(region, name, sample),
            );
            match (x, y) {
                (Some(x), Some(y)) => Some(lerp(x, y, weather)),
                (x, y) => x.or(y),
            }
        };
        if let (Some(early), Some(late)) = (at(first), at(second)) {
            frame.set(name, lerp(early, late, along));
        }
    }
    frame
}

/// What the scanner meets in an XML text.
pub(crate) enum Token<'a> {
    Open {
        name: &'a str,
        attrs: &'a str,
        empty: bool,
    },
    Close(&'a str),
    Text(&'a str),
}

pub(crate) fn tokens(text: &str) -> impl Iterator<Item = Token<'_>> {
    let mut rest = text;
    std::iter::from_fn(move || loop {
        if rest.is_empty() {
            return None;
        }
        let Some(start) = rest.find('<') else {
            let text = std::mem::take(&mut rest).trim();
            return (!text.is_empty()).then_some(Token::Text(text));
        };
        if start > 0 {
            let text = rest[..start].trim();
            rest = &rest[start..];
            if !text.is_empty() {
                return Some(Token::Text(text));
            }
        }
        let end = rest.find('>')?;
        let inside = &rest[1..end];
        rest = &rest[end + 1..];
        if inside.starts_with('?') || inside.starts_with('!') {
            continue;
        }
        if let Some(name) = inside.strip_prefix('/') {
            return Some(Token::Close(name.trim()));
        }
        let (inside, empty) = match inside.strip_suffix('/') {
            Some(inside) => (inside, true),
            None => (inside, false),
        };
        let (name, attrs) = inside
            .split_once(char::is_whitespace)
            .unwrap_or((inside, ""));
        return Some(Token::Open { name, attrs, empty });
    })
}

/// The value of attribute `key` in the text of a tag's attributes.
pub(crate) fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(found) = attrs[from..].find(key) {
        let at = from + found;
        let before_ok = at == 0 || attrs[..at].ends_with(char::is_whitespace);
        let after = &attrs[at + key.len()..];
        if before_ok {
            if let Some(value) = after.trim_start().strip_prefix('=') {
                let value = value.trim_start();
                let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
                let value = &value[1..];
                return value.find(quote).map(|end| &value[..end]);
            }
        }
        from = at + key.len();
    }
    None
}

#[cfg(test)]
mod tests;
