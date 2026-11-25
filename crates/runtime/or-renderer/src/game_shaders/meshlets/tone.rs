//! How GTA VI meters a lit frame and tones it: the mean luminance of the pixels between two
//! shares of the frame, an exposure that mean gives by a curve, and a filmic curve whose shape
//! the exposure blends between two. The resolve does this on the device; what is here is the
//! same arithmetic, which the tests hold, and the constants a frame is handed.

/// What a cycle and an install's tuning values state of the metering and the tone curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    /// How far the directional light's roughness is taken towards one.
    pub roughness_mod: f32,
    /// The tangent of the angle the lighting body's disc is seen half of.
    pub disc: f32,
    /// The shares of the darkest and of the brightest pixels the meter leaves out.
    pub lowlights: f32,
    pub highlights: f32,
    /// Two curves from the metered luminance to an exposure in stops, and how far from the first
    /// to the second: a scale of the luminance's natural logarithm, an offset in stops, what is
    /// added to the luminance first, and a scale of the luminance itself.
    pub curve: [Curve; 2],
    pub blend: f32,
    /// The luminance a metered frame is brought to, where a cycle meters by it and not by the
    /// curves.
    pub key: Option<f32>,
    /// The least and the most stops, and the stops a cycle adds.
    pub least: f32,
    pub most: f32,
    pub added: f32,
    /// The filmic curve of a bright frame and of a dark one, and the exposures, as multipliers,
    /// the one and the other stand at.
    pub bright: Filmic,
    pub dark: Filmic,
    pub bright_at: f32,
    pub dark_at: f32,
    /// Whether the meter takes the whole frame, as the game does: the model, the ground under
    /// it and the sky where neither is. Otherwise it takes the model's lit surfaces alone.
    pub whole_frame: bool,
}

/// A curve from a luminance to stops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    pub scale: f32,
    pub vertical: f32,
    pub horizontal: f32,
    pub bending: f32,
}

impl Curve {
    /// The stops a metered `luminance` gives.
    pub fn stops(&self, luminance: f32) -> f32 {
        self.vertical - 10.0
            + self.bending * luminance
            + self.scale * (self.horizontal + luminance).ln()
    }
}

/// The seven numbers of a filmic curve: shoulder, linear strength and angle, toe strength,
/// numerator and denominator, and the white it maps to one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filmic(pub [f32; 7]);

impl Filmic {
    fn raw(&self, x: f32) -> f32 {
        let [a, b, c, d, e, f, _] = self.0;
        (x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f) - e / f
    }

    /// What the curve makes of the linear `x`, before the frame's encoding.
    pub fn toned(&self, x: f32) -> f32 {
        (self.raw(x.max(0.0)) / self.raw(self.0[6])).clamp(0.0, 1.0)
    }

    fn towards(&self, other: &Self, along: f32) -> Self {
        Self(std::array::from_fn(|i| {
            self.0[i] + along * (other.0[i] - self.0[i])
        }))
    }
}

/// The bins of the meter's histogram, laid evenly over the logarithm of the luminance between
/// the frame's least and its greatest.
const BINS: usize = 256;
/// The least luminance a pixel is counted with.
pub const LEAST_LUMINANCE: f32 = 1e-6;

/// The mean luminance of `luminances` as the meter takes it: each goes to its bin and counts as
/// the luminance its bin starts at, the darkest `lowlights` of them and the brightest
/// `highlights` are left out, and the rest are averaged. Nought where there is none.
pub fn metered(luminances: &[f32], lowlights: f32, highlights: f32) -> f32 {
    let lit = || luminances.iter().map(|l| l.max(LEAST_LUMINANCE));
    let (least, most) = lit().fold((f32::MAX, 0f32), |(lo, hi), l| (lo.min(l), hi.max(l)));
    if luminances.is_empty() {
        return 0.0;
    }
    let ratio = if least >= 1e-8 { most / least } else { 1.0 };
    let span = ratio.ln().max(1e-8);
    let mut bins = [0f32; BINS];
    for l in lit() {
        let bin = 255.0 * (l / least).ln() / span;
        bins[(bin.max(0.0) as usize).min(BINS - 1)] += 1.0;
    }
    let count = luminances.len() as f32;
    let lows = (lowlights * count).round();
    let middle_ends = (count - ((highlights * count).round() + lows)).round() + lows;
    let (mut before, mut sum, mut taken) = (0.0, 0.0, 0.0);
    for (bin, held) in bins.iter().enumerate() {
        let (mut left, mut at) = (*held, before);
        if left > 0.0 && lows > at {
            left -= (lows - at).min(left);
            at = lows;
        }
        if left > 0.0 && middle_ends > at {
            let middle = (middle_ends - at).min(left);
            let luminance = least * ratio.powf(bin as f32 / 255.0);
            sum += middle * luminance.clamp(least, most);
            taken += middle;
        }
        before += held;
    }
    if taken > 0.0 {
        sum / taken
    } else {
        0.0
    }
}

impl Tone {
    /// The stops a frame whose metered luminance is `mean` is exposed by.
    // No history: the game moves from one frame's stops towards the next's by a step, and takes
    // them at once when it resets, as a viewer does every frame.
    pub fn stops(&self, mean: f32) -> f32 {
        let metered = match self.key {
            Some(key) => (key / mean.max(LEAST_LUMINANCE)).log2(),
            None => {
                let [first, second] = self.curve.map(|curve| curve.stops(mean));
                first + self.blend * (second - first)
            }
        };
        // A frame of no light meters to no number: it is held to the most stops, as the device
        // holds an infinity.
        let metered = if metered.is_nan() { self.most } else { metered };
        (metered.clamp(self.least, self.most) + self.added).clamp(self.least, self.most)
    }

    /// How far from the bright curve to the dark one a frame exposed by the multiplier
    /// `exposure` stands: its scale and its offset.
    fn dark_share(&self) -> (f32, f32) {
        let apart = self.dark_at - self.bright_at;
        let apart = if apart == 0.0 { 1.0 } else { apart };
        (1.0 / apart, -self.bright_at / apart)
    }

    /// The filmic curve of a frame exposed by the multiplier `exposure`.
    pub fn filmic(&self, exposure: f32) -> Filmic {
        let (scale, offset) = self.dark_share();
        (self.bright).towards(&self.dark, (exposure * scale + offset).clamp(0.0, 1.0))
    }

    /// What the resolve reads of the tone, after the light's own words: whether the game's
    /// metering runs, then the numbers above in rows of four.
    pub(super) fn words(tone: Option<&Self>) -> [f32; 36] {
        let mut words = [0.0; 36];
        let Some(tone) = tone else {
            return words;
        };
        let (scale, offset) = tone.dark_share();
        let curve = |c: &Curve| [c.scale, c.vertical, c.horizontal, c.bending];
        let rows: [[f32; 4]; 9] = [
            [1.0, tone.roughness_mod, tone.lowlights, tone.highlights],
            curve(&tone.curve[0]),
            curve(&tone.curve[1]),
            [tone.least, tone.most, tone.added, tone.blend],
            [
                tone.key.unwrap_or(0.0),
                f32::from(u8::from(tone.key.is_some())),
                scale,
                offset,
            ],
            [
                tone.bright.0[0],
                tone.bright.0[1],
                tone.bright.0[2],
                tone.bright.0[3],
            ],
            [
                tone.bright.0[4],
                tone.bright.0[5],
                tone.bright.0[6],
                tone.disc,
            ],
            [
                tone.dark.0[0],
                tone.dark.0[1],
                tone.dark.0[2],
                tone.dark.0[3],
            ],
            [
                tone.dark.0[4],
                tone.dark.0[5],
                tone.dark.0[6],
                f32::from(u8::from(tone.whole_frame)),
            ],
        ];
        for (at, row) in rows.iter().enumerate() {
            words[at * 4..at * 4 + 4].copy_from_slice(row);
        }
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Tone {
        let curve = Curve {
            scale: -1.0,
            vertical: 10.0,
            horizontal: 0.0,
            bending: 0.0,
        };
        Tone {
            roughness_mod: 0.0,
            disc: 0.0,
            lowlights: 0.25,
            highlights: 0.25,
            curve: [
                curve,
                Curve {
                    vertical: 12.0,
                    ..curve
                },
            ],
            blend: 0.0,
            key: None,
            least: -4.0,
            most: 3.0,
            added: 0.0,
            bright: Filmic([0.22, 0.3, 0.1, 0.2, 0.01, 0.3, 11.2]),
            dark: Filmic([0.22, 0.3, 0.1, 0.2, 0.0, 0.3, 11.2]),
            bright_at: 1.0,
            dark_at: 3.0,
            whole_frame: false,
        }
    }

    #[test]
    fn the_resolve_is_a_program_a_device_takes_with_a_stage_that_meters_and_one_that_lights() {
        let module = naga::front::wgsl::parse_str(super::super::RESOLVE).expect("it parses");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        validator.validate(&module).expect("it is valid");
        let stages: Vec<&str> = (module.entry_points.iter())
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(stages, ["at_corner", "masked", "lit", "metered"]);
        // The light's block is as long as the bytes a frame writes of it.
        let light = (module.types.iter())
            .find(|(_, ty)| ty.name.as_deref() == Some("Light"))
            .expect("the light's block");
        assert_eq!(
            light.1.inner.size(module.to_ctx()) as usize,
            super::super::LIGHT_BYTES
        );
    }

    #[test]
    fn the_meter_leaves_out_the_darkest_and_the_brightest_shares_and_averages_the_rest() {
        // Eight pixels, a quarter left out at each end: the four of 8 are what is averaged,
        // whatever the two ends hold.
        let frame = [0.5, 0.5, 8.0, 8.0, 8.0, 8.0, 512.0, 512.0];
        let mean = metered(&frame, 0.25, 0.25);
        // A bin counts as the luminance it starts at: a little under the 8 of its pixels.
        assert!((7.7..=8.01).contains(&mean), "{mean}");
        let all = metered(&frame, 0.0, 0.0);
        assert!(
            (all - (1.0 + 4.0 * mean + 1024.0) / 8.0).abs() < 4.0,
            "{all}"
        );
        assert!(metered(&frame, 0.0, 0.75) < mean && metered(&frame, 0.75, 0.0) > mean);
        // One luminance all over is that luminance, and no pixel is nought.
        assert_eq!(metered(&[3.0; 5], 0.3, 0.2), 3.0);
        assert_eq!(metered(&[], 0.3, 0.2), 0.0);
    }

    #[test]
    fn the_stops_follow_the_curve_are_held_between_the_least_and_the_most_and_take_what_is_added() {
        let mut tone = tone();
        // The first curve: no stop at a luminance of one, one less for each e-fold brighter.
        assert_eq!(tone.stops(1.0), 0.0);
        assert!((tone.stops(std::f32::consts::E) + 1.0).abs() < 1e-5);
        assert_eq!(tone.stops(1e9), -4.0);
        assert_eq!(tone.stops(0.0), 3.0, "a black frame is held to the most");
        tone.added = 0.5;
        assert_eq!(tone.stops(1.0), 0.5);
        assert_eq!(tone.stops(1e-9), 3.0, "held again after what is added");
        tone.blend = 0.5;
        assert_eq!(tone.stops(1.0), 1.5, "half way to a curve two stops over");
        tone.key = Some(0.5);
        assert_eq!(
            tone.stops(2.0),
            -1.5,
            "by the key: a quarter, then what is added"
        );
    }

    #[test]
    fn the_filmic_curve_maps_its_white_to_one_and_blends_from_bright_to_dark_by_the_exposure() {
        let tone = tone();
        assert_eq!(tone.filmic(0.5), tone.bright);
        assert_eq!(tone.filmic(8.0), tone.dark);
        let between = tone.filmic(2.0);
        assert!((between.0[4] - 0.005).abs() < 1e-6, "{between:?}");
        let curve = tone.dark;
        assert_eq!(curve.toned(0.0), 0.0);
        assert!((curve.toned(11.2) - 1.0).abs() < 1e-6);
        assert_eq!(curve.toned(1e6), 1.0);
        // At one, by the curve's numbers: the toe's are nought in the dark curve.
        let [a, b, c, d, _, f, w] = curve.0;
        let raw = |x: f32| x * (a * x + c * b) / (x * (a * x + b) + d * f);
        assert!((curve.toned(1.0) - raw(1.0) / raw(w)).abs() < 1e-6);
        assert!(curve.toned(0.18) < curve.toned(1.0) && curve.toned(1.0) < 0.6);
        // The rows a frame is handed: the first says the game's metering runs.
        assert_eq!(Tone::words(None), [0.0; 36]);
        let words = Tone::words(Some(&tone));
        assert_eq!(words[..4], [1.0, 0.0, 0.25, 0.25]);
        assert_eq!(words[16..20], [0.0, 0.0, 0.5, -0.5]);
        // The last word says what the meter takes.
        assert_eq!(words[35], 0.0);
        let whole = Tone {
            whole_frame: true,
            ..tone
        };
        assert_eq!(Tone::words(Some(&whole))[35], 1.0);
    }
}
