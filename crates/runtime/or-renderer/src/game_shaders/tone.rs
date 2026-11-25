//! How a lit frame is mapped to the screen: the filmic curve the install's visual settings and the moment's timecycle give, and the exposure that brings the light of the moment into the range the screen holds.

use std::collections::BTreeMap;

use or_io::gta5::data::timecycle::Keyframe;

use super::lighting::Lighting;

/// The parameters of one filmic curve, as the game's tone mapping reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filmic {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
    /// The white point: the value the curve maps to one.
    pub w: f32,
}

impl Filmic {
    fn first(self) -> Vec<f32> {
        vec![self.a, self.b, self.c, self.d]
    }

    fn second(self) -> Vec<f32> {
        vec![self.e, self.f, self.w, 0.0]
    }

    /// What the timecycle states for the curve `which` (`"dark"` or `"bright"`).
    fn from_keyframe(keyframe: &Keyframe, which: &str) -> Self {
        // The dark curve is the one with no suffix
        let suffix = if which == "bright" { "_bright" } else { "" };
        let var =
            |name: &str| keyframe.get_or(&format!("postfx_tonemap_filmic_{name}{suffix}"), 0.0);
        Self {
            a: var("a"),
            b: var("b"),
            c: var("c"),
            d: var("d"),
            e: var("e"),
            f: var("f"),
            w: var("w"),
        }
    }
}

/// What the install's visual settings state of the tone mapping and of the exposure, which a day carries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneSettings {
    pub bright: Filmic,
    pub dark: Filmic,
    /// The exposures, in stops, at which the bright and the dark curve are used whole.
    pub bright_exposure: f32,
    pub dark_exposure: f32,
    /// The curve from the average luminance of a frame to the exposure that suits it: `scale * luminance.powf(power) + offset`.
    pub curve_scale: f32,
    pub curve_power: f32,
    pub curve_offset: f32,
    /// How much of each size of blur of the bloom is added to the next size up: the large, the medium and the small.
    pub bloom_blend: [f32; 3],
}

impl Default for ToneSettings {
    /// What the game holds where the visual settings say nothing.
    fn default() -> Self {
        let curve = |e: f32| Filmic {
            a: 0.22,
            b: 0.3,
            c: 0.10,
            d: 0.20,
            e,
            f: 0.30,
            w: 4.0,
        };
        Self {
            bright: curve(0.01),
            dark: curve(0.01),
            bright_exposure: -2.0,
            dark_exposure: 3.0,
            curve_scale: -106.689_07,
            curve_power: 4.189_701_5e-3,
            curve_offset: 104.603_64,
            bloom_blend: [0.5, 0.5, 1.0],
        }
    }
}

impl ToneSettings {
    /// The settings `get` reads from the visual settings by name, each with the game's own value where there is none.
    pub fn from_visual(get: &dyn Fn(&str, f32) -> f32) -> Self {
        let default = Self::default();
        let curve = |which: &str, base: Filmic| Filmic {
            a: get(&format!("Tonemapping.{which}.filmic.A"), base.a),
            b: get(&format!("Tonemapping.{which}.filmic.B"), base.b),
            c: get(&format!("Tonemapping.{which}.filmic.C"), base.c),
            d: get(&format!("Tonemapping.{which}.filmic.D"), base.d),
            e: get(&format!("Tonemapping.{which}.filmic.E"), base.e),
            f: get(&format!("Tonemapping.{which}.filmic.F"), base.f),
            w: get(&format!("Tonemapping.{which}.filmic.W"), base.w),
        };
        Self {
            bright: curve("bright", default.bright),
            dark: curve("dark", default.dark),
            bright_exposure: get(
                "Tonemapping.bright.filmic.exposure",
                default.bright_exposure,
            ),
            dark_exposure: get("Tonemapping.dark.filmic.exposure", default.dark_exposure),
            curve_scale: get("Exposure.curve.scale", default.curve_scale),
            curve_power: get("Exposure.curve.power", default.curve_power),
            curve_offset: get("Exposure.curve.offset", default.curve_offset),
            bloom_blend: [
                get("bloom.blurblendmult.large", default.bloom_blend[0]),
                get("bloom.blurblendmult.med", default.bloom_blend[1]),
                get("bloom.blurblendmult.small", default.bloom_blend[2]),
            ],
        }
    }

    /// The exposure, in stops, for a frame whose average luminance is `luminance`, with the moment's `tweak` added, held between the least and the most the moment allows.
    pub fn exposure(&self, luminance: f32, tweak: f32, least: f32, most: f32) -> f32 {
        let wanted = self.curve_scale * luminance.max(1e-6).powf(self.curve_power)
            + self.curve_offset
            + tweak;
        wanted.clamp(least.min(most), most)
    }
}

/// The luminance of a colour.
fn luminance(c: glam::Vec3) -> f32 {
    c.dot(glam::Vec3::new(0.2125, 0.7154, 0.0721))
}

/// The average luminance of a frame that is a middling grey surface lit by the light of a moment: half of the sun and the ambient light a surface turned up and one turned down share.
///
/// ⚠ The game measures it from the frame it has drawn, and a drawable alone in the viewport is not the scene it tunes the curve for, so this stands in for the measure.
pub fn expected_luminance(lighting: &Lighting) -> f32 {
    const MIDDLE_GREY: f32 = 0.2;
    let sun = luminance(lighting.directional_colour.truncate());
    let [down, up] = lighting.natural_ambient;
    let ambient = (luminance(down.truncate()) + luminance(up.truncate())) * 0.5;
    MIDDLE_GREY * (sun * 0.5 + ambient)
}

/// What a moment states of the bloom: the brightness from which the light of a frame is put back over it, how wide the edge of that is, and how much of it comes back.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bloom {
    pub threshold: f32,
    pub width: f32,
    pub intensity: f32,
}

impl Bloom {
    /// The variable of the post-processing buffer the bright pass reads: where the brightness starts, the intensity, and one over the span from where it starts to where it is whole.
    pub fn params(&self) -> [f32; 4] {
        let (threshold, span) = if self.width > 0.0 {
            let most = (self.threshold + self.width * 0.5).max(0.0);
            let least = (self.threshold - self.width * 0.5).max(0.0);
            (least, 1.0 / (most - least).max(1e-4))
        } else {
            (self.threshold, 1.0)
        };
        [threshold, self.intensity, span, 0.0]
    }

    /// How much of the blurred light is put back: a quarter of the intensity.
    pub fn amount(&self) -> f32 {
        self.intensity * 0.25
    }
}

/// The tone mapping of one frame: the curve to map by, and the exposure to map it at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub bright: Filmic,
    pub dark: Filmic,
    pub bright_exposure: f32,
    pub dark_exposure: f32,
    /// The exposure, in stops.
    pub stops: f32,
    /// The colour correction of the moment.
    pub correct: [f32; 4],
    pub shift: [f32; 4],
    pub desaturation: f32,
    pub bloom: Bloom,
    /// How much of each size of blur of the bloom is added to the next size up.
    pub bloom_blend: [f32; 3],
    /// How far the curve is blended from the bright one to the dark, where that is fixed rather than read from the frame's stops.
    pub blend: Option<f32>,
}

impl Tone {
    /// The tone of a frame lit by `lighting`, mapped at the exposure `settings` give it, with what `keyframe` states for the moment where there is one.
    pub fn new(settings: &ToneSettings, keyframe: Option<&Keyframe>, lighting: &Lighting) -> Self {
        let empty = Keyframe::default();
        let moment = keyframe.unwrap_or(&empty);
        let mut tone = Self {
            bright: settings.bright,
            dark: settings.dark,
            bright_exposure: settings.bright_exposure,
            dark_exposure: settings.dark_exposure,
            stops: 0.0,
            correct: [
                moment.get_or("postfx_correct_col_r", 1.0),
                moment.get_or("postfx_correct_col_g", 1.0),
                moment.get_or("postfx_correct_col_b", 1.0),
                moment.get_or("postfx_correct_cutoff", 0.0),
            ],
            shift: [
                moment.get_or("postfx_shift_col_r", 1.0),
                moment.get_or("postfx_shift_col_g", 1.0),
                moment.get_or("postfx_shift_col_b", 1.0),
                moment.get_or("postfx_shift_cutoff", 0.05),
            ],
            desaturation: moment.get_or("postfx_desaturation", 1.0),
            bloom: Bloom {
                threshold: moment.get_or("postfx_bright_pass_thresh", 0.0),
                width: moment.get_or("postfx_bright_pass_thresh_width", 0.0),
                intensity: moment.get_or("postfx_intensity_bloom", 0.0),
            },
            bloom_blend: settings.bloom_blend,
            blend: None,
        };
        // A moment may state its own curves in place of the install's: the dark one alone for both, or each
        let dark_stated = moment.get_or("postfx_tonemap_filmic_override_dark", 0.0) > 0.0;
        let bright_stated = moment.get_or("postfx_tonemap_filmic_override_bright", 0.0) > 0.0;
        if dark_stated || bright_stated {
            tone.dark = Filmic::from_keyframe(moment, "dark");
            tone.dark_exposure =
                moment.get_or("postfx_tonemap_filmic_exposure_dark", tone.dark_exposure);
            if bright_stated {
                tone.bright = Filmic::from_keyframe(moment, "bright");
                tone.bright_exposure = moment.get_or(
                    "postfx_tonemap_filmic_exposure_bright",
                    tone.bright_exposure,
                );
            } else {
                tone.bright = tone.dark;
                tone.bright_exposure = tone.dark_exposure;
            }
        }
        tone.stops = settings.exposure(
            expected_luminance(lighting),
            moment.get_or("postfx_exposure", 0.0),
            moment.get_or("postfx_exposure_min", -3.5),
            moment.get_or("postfx_exposure_max", 5.0),
        );
        tone
    }

    /// This tone with its curves blended by the `stops` the light was taken at, against the curves' exposures, as GTA VI blends them for its frame.
    // ⚠ Read: the blend is worked out on the CPU from the stops the exposure pass wrote the frame
    // before; a linear exposure here would give the dark curve at every hour.
    pub fn blended_by(self, stops: f32) -> Self {
        let range = self.dark_exposure - self.bright_exposure;
        let t = match range == 0.0 {
            true => 1.0,
            false => ((stops - self.bright_exposure) / range).clamp(0.0, 1.0),
        };
        Self {
            blend: Some(t),
            ..self
        }
    }

    /// How far the curve is blended from the bright one to the dark.
    fn dark_share(&self) -> f32 {
        let range = self.dark_exposure - self.bright_exposure;
        self.blend.unwrap_or(match range == 0.0 {
            true => 1.0,
            false => ((self.stops - self.bright_exposure) / range).clamp(0.0, 1.0),
        })
    }

    /// One over the white point of the curve this exposure maps by, which the bright pass divides the light by.
    pub fn one_over_white_point(&self) -> f32 {
        let t = self.dark_share();
        1.0 / (self.bright.w + (self.dark.w - self.bright.w) * t).max(1e-4)
    }

    /// The texel the passes read the exposure from: the factor it scales the light by, and the stops it is.
    pub fn adaptation(&self) -> [f32; 2] {
        [self.stops.exp2(), self.stops]
    }

    /// The tone mapping variables of the post-processing constant buffer, by name.
    pub fn values(&self) -> BTreeMap<&'static str, Vec<f32>> {
        // The two curves are blended by how the exposure sits between theirs
        let range = if self.dark_exposure == self.bright_exposure {
            1.0
        } else {
            self.dark_exposure - self.bright_exposure
        };
        BTreeMap::from([
            ("BrightTonemapParams0", self.bright.first()),
            ("BrightTonemapParams1", self.bright.second()),
            ("DarkTonemapParams0", self.dark.first()),
            ("DarkTonemapParams1", self.dark.second()),
            (
                "TonemapParams",
                match self.blend {
                    Some(t) => vec![0.0, t],
                    None => vec![1.0 / range, -self.bright_exposure / range],
                },
            ),
            ("ColorCorrectHighLum", self.correct.to_vec()),
            ("ColorShiftLowLum", self.shift.to_vec()),
            ("Desaturate", vec![self.desaturation]),
        ])
    }
}

/// The sizes of GTA VI's bloom: blurs of a half of the frame down to a 256th.
pub const BLOOM_SIZES: usize = 8;
/// The samples of the exposure's compensation curve the per-pixel exposure reads.
pub const COMPENSATION_SAMPLES: usize = 32;

/// What GTA VI states of the passes after its frame is lit: its bloom, its per-pixel exposure, its temporal anti-aliasing and its sharpening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gta6Post {
    /// How much each size of blur, from a half of the frame to a 256th, weighs in the bloom.
    pub bloom_weights: [f32; BLOOM_SIZES],
    /// How much of the frame the bloom takes the place of.
    pub bloom_mix: f32,
    pub local: LocalExposure,
    pub temporal: Temporal,
    /// How much of the difference from its four neighbours a pixel of the mapped frame is pushed by.
    pub sharpen: f32,
    /// The moment's colour grade.
    pub grade: super::grade::Grade,
}

/// GTA VI's per-pixel exposure: the stops its curve gives the luminance about a pixel, of which `amount` takes the place of the frame's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalExposure {
    pub amount: f32,
    /// The per-pixel curve: `scale * ln(horizontal_offset + L) + bending * L + vertical_offset - 10`.
    pub scale: f32,
    pub horizontal_offset: f32,
    pub vertical_offset: f32,
    pub bending: f32,
    /// The least and the most stops, and what the moment adds to them.
    pub least: f32,
    pub most: f32,
    pub offset: f32,
    /// How much of the compensation curve is added, and the curve sampled evenly over `input` and held to `output`.
    pub contrast: f32,
    pub compensation: [f32; COMPENSATION_SAMPLES],
    pub input: [f32; 2],
    pub output: [f32; 2],
}

/// GTA VI's temporal anti-aliasing: how long a pixel takes to converge near and far, and how far its history may stray from what is about it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Temporal {
    /// Seconds to converge at the camera and from `far_depth` metres on.
    pub near: f32,
    pub far: f32,
    pub far_depth: f32,
    /// Seconds to converge at the camera while it moves forward, and the speeds along the view, in metres a second, the move starts and ends at.
    pub forward: f32,
    pub forward_speed: [f32; 2],
    /// How many deviations of the pixels about it the history is held within.
    pub gamma: f32,
}

#[cfg(test)]
mod tests;
