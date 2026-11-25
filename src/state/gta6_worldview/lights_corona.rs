//! The coronas of the placed lights about the camera, worked out as GTA VI works them out: a size
//! kept constant on the screen and an intensity that falls with the square of the distance along
//! the view, faded across a spot's cone and pulled toward the eye by the light's own bias.

use glam::Vec3;
use or_io::gta5::data::visual_settings::VisualSettings;
use or_io::gta6::map::light_ext::LightKind;
use or_renderer::world::light_math::corona_cone;
use or_renderer::{Corona, CoronaLook};

use super::lights_frame::{hour_strength, Lamp};

/// How a corona is sized and lit, as the install's visual settings state it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoronaReach {
    /// Whether a corona keeps its size on the screen, its intensity then falling with the distance squared.
    pub constant: bool,
    /// What a light's corona size and intensity are multiplied by, kept on the screen or not.
    pub size: f32,
    pub intensity: f32,
    /// The tangent of half the reference lens's angle, which a narrower lens grows a corona by where `fov_scaled`.
    pub reference_tan: f32,
    pub fov_scaled: bool,
    /// The distances along the view over which a light's bias comes in, and the least and most it pulls by.
    pub bias_ramp: [f32; 2],
    pub bias_range: [f32; 2],
    /// What every corona of the frame's own view is pulled by beside its light's bias.
    pub bias_added: f32,
    /// The most a corona's intensity is, where the settings hold it to one.
    pub most: Option<f32>,
    pub look: CoronaLook,
}

impl CoronaReach {
    /// As the install's visual settings state it, with the defaults the game takes for a setting it does not find.
    pub fn of(settings: &VisualSettings) -> Self {
        let get =
            |name: &str, default: f32| settings.get_f32(&format!("misc.coronas.{name}"), default);
        let constant = get("useConstantSizeOnScreen", 0.0) == 1.0;
        let (size, intensity) = match constant {
            true => (
                get("constantSizeScaleGlobal", 0.03),
                get("constantSizeIntensityGlobal", 100.0),
            ),
            false => (
                get("sizeScaleGlobal", 1.0),
                get("intensityScaleGlobal", 1.0),
            ),
        };
        let edge = get("screenEdgeMinDistForFade", 0.95);
        Self {
            constant,
            size,
            intensity,
            reference_tan: (get("referenceFOV", 50.0).to_radians() * 0.5).tan(),
            fov_scaled: get("useFOVScale", 1.0) == 1.0,
            bias_ramp: [
                get("zBias.nearScalingStartDist", 1.0),
                get("zBias.nearScalingEndDist", 2.0),
            ],
            bias_range: [get("zBias.min", 0.0), get("zBias.max", 1.0)],
            bias_added: get("zBias.additionalBias.Default", 0.0),
            // What passes the threshold the game draws again into its bloom, which this frame's own bloom stands for
            most: (get("renderExcessEnergyInBloom", 0.0) == 1.0)
                .then(|| get("bloomEnergyThreshold", 0.0)),
            look: CoronaLook {
                occlusion_share: get("occlusionSizeScale", 0.5),
                occlusion_pixels: get("occlusionSizeMax", 32.0),
                screen_edge: if edge < 0.0 { 0.0 } else { edge.min(1.0) },
            },
        }
    }
}

/// Where the camera stands and looks, and the tangent of half its lens's upright angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub eye: Vec3,
    pub forward: Vec3,
    pub tan_half: f32,
}

/// The coronas of `lamps` lit at `hour`, seen from `view`, their radiance times `exposure`.
pub fn coronas<'a>(
    lamps: impl IntoIterator<Item = &'a Lamp>,
    view: &View,
    hour: f32,
    exposure: f32,
    reach: &CoronaReach,
) -> Vec<Corona> {
    let fov = match reach.constant && reach.fov_scaled && reach.reference_tan > 0.0 {
        true => view.tan_half / reach.reference_tan,
        false => 1.0,
    };
    let mut out = Vec::new();
    for lamp in lamps {
        let l = &lamp.light;
        let [size, intensity, bias] = l.corona;
        let lit = hour_strength(l.hours, hour);
        if !(size > 0.0 && intensity > 0.0 && lit > 0.0) {
            continue;
        }
        let position = Vec3::from(l.position);
        let toward = position - view.eye;
        let z = toward.dot(view.forward);
        if z <= 0.0 {
            continue;
        }
        // A spot's corona shows as the eye stands within its cones, and grows with them; the item's remap width is not known, and one leaves the cone weight as it is.
        let fade = match l.kind {
            LightKind::Spot => {
                let to_eye = -toward.normalize_or_zero();
                let cones = [l.cone_inner, l.cone_outer];
                corona_cone(Vec3::from(l.direction), to_eye, cones, 1.0).1
            }
            _ => 1.0,
        };
        let mut glow = intensity * reach.intensity * fade * lit;
        let mut side = size * reach.size * fade;
        side *= match reach.constant {
            true => fov * z,
            false => 0.5,
        };
        if reach.constant {
            glow /= z * z;
        }
        let ramp = ((z - reach.bias_ramp[0]) / (reach.bias_ramp[1] - reach.bias_ramp[0]).max(1e-3))
            .clamp(0.0, 1.0);
        // ⚠ Not `clamp`: the bounds are the install's own numbers, which may stand reversed.
        let held = (ramp * bias)
            .max(reach.bias_range[0])
            .min(reach.bias_range[1]);
        let pull = (held + reach.bias_added) / z;
        side -= side * pull;
        if let Some(most) = reach.most {
            glow = glow.min(most);
        }
        if side > 0.0 && glow > 0.0 {
            out.push(Corona {
                position: position - toward * pull,
                size: side,
                colour: Vec3::from(l.colour),
                intensity: glow * exposure,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::map::light_ext::Light;

    fn reach() -> CoronaReach {
        CoronaReach::of(&VisualSettings::parse(
            "misc.coronas.useConstantSizeOnScreen 1\nmisc.coronas.constantSizeScaleGlobal 0.01\n\
             misc.coronas.constantSizeIntensityGlobal 100\nmisc.coronas.referenceFOV 50\n\
             misc.coronas.zBias.max 0.35\n",
        ))
    }

    fn lamp(kind: LightKind, y: f32, corona: [f32; 3]) -> Lamp {
        Lamp {
            light: Light {
                kind,
                position: [0.0, y, 0.0],
                direction: [0.0, -1.0, 0.0],
                colour: [1.0, 0.5, 0.0],
                intensity: 10.0,
                radius: 5.0,
                falloff_exponent: 8.0,
                hours: 0x00FF_FFFF,
                flags: 0,
                cone_inner: 20.0,
                cone_outer: 40.0,
                length: 0.0,
                corona,
            },
            reach: None,
        }
    }

    fn view() -> View {
        View {
            eye: Vec3::ZERO,
            forward: Vec3::Y,
            tan_half: (25.0f32).to_radians().tan(),
        }
    }

    #[test]
    fn a_corona_keeps_its_size_on_the_screen_and_falls_with_the_distance_squared() {
        let near = coronas(
            &[lamp(LightKind::Point, 10.0, [2.0, 3.0, 0.0])],
            &view(),
            12.0,
            0.5,
            &reach(),
        );
        let far = coronas(
            &[lamp(LightKind::Point, 40.0, [2.0, 3.0, 0.0])],
            &view(),
            12.0,
            0.5,
            &reach(),
        );
        assert_eq!((near.len(), far.len()), (1, 1));
        assert!(
            (near[0].size / 10.0 - far[0].size / 40.0).abs() < 1e-6,
            "{near:?} {far:?}"
        );
        assert!((near[0].size - 2.0 * 0.01 * 10.0).abs() < 1e-5, "{near:?}");
        assert!((near[0].intensity - 3.0 * 100.0 / 100.0 * 0.5).abs() < 1e-5);
        assert!((near[0].intensity / far[0].intensity - 16.0).abs() < 1e-3);
        assert_eq!(near[0].colour, Vec3::new(1.0, 0.5, 0.0));
    }

    #[test]
    fn a_corona_is_pulled_toward_the_eye_by_its_bias_and_shrinks_as_much() {
        let out = coronas(
            &[lamp(LightKind::Point, 20.0, [2.0, 3.0, 0.2])],
            &view(),
            12.0,
            1.0,
            &reach(),
        );
        let pull = 0.2 / 20.0;
        assert!(
            (out[0].position.y - 20.0 * (1.0 - pull)).abs() < 1e-4,
            "{out:?}"
        );
        assert!((out[0].size - 2.0 * 0.01 * 20.0 * (1.0 - pull)).abs() < 1e-5);
        let held = coronas(
            &[lamp(LightKind::Point, 20.0, [2.0, 3.0, 9.0])],
            &view(),
            12.0,
            1.0,
            &reach(),
        );
        assert!(
            (held[0].position.y - 20.0 * (1.0 - 0.35 / 20.0)).abs() < 1e-4,
            "held to the most"
        );
    }

    #[test]
    fn settings_whose_bias_bounds_stand_reversed_or_are_no_number_still_draw_coronas() {
        let lamps = [lamp(LightKind::Point, 20.0, [2.0, 3.0, 0.2])];
        for range in [[0.5, 0.1], [f32::NAN; 2]] {
            let reach = CoronaReach {
                bias_range: range,
                ..reach()
            };
            let out = coronas(&lamps, &view(), 12.0, 1.0, &reach);
            assert_eq!(out.len(), 1, "{range:?}");
        }
    }

    #[test]
    fn a_spot_shows_its_corona_only_to_an_eye_within_its_cones() {
        let facing = coronas(
            &[lamp(LightKind::Spot, 20.0, [2.0, 3.0, 0.0])],
            &view(),
            12.0,
            1.0,
            &reach(),
        );
        assert_eq!(facing.len(), 1);
        let mut away = lamp(LightKind::Spot, 20.0, [2.0, 3.0, 0.0]);
        away.light.direction = [0.0, 1.0, 0.0];
        assert!(coronas(&[away], &view(), 12.0, 1.0, &reach()).is_empty());
        let behind = lamp(LightKind::Point, -5.0, [2.0, 3.0, 0.0]);
        let none = lamp(LightKind::Point, 5.0, [0.0, 3.0, 0.0]);
        assert!(coronas(&[behind, none], &view(), 12.0, 1.0, &reach()).is_empty());
    }
}
