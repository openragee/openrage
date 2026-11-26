//! GTA VI's layer of clouds as its cycles state it: where it lies, how much of the sky it covers and
//! how high its tops reach, and what it lets through to the ground of the light that falls on it.

use or_io::gta5::data::timecycle::Keyframe;

/// What a cloud takes out of light per metre where it is whole.
// ⚠ Inferred: the build's coefficient is a constant of its cloud system that is not read; a cumulus takes some hundredths a metre.
pub const EXTINCTION: f32 = 0.04;
/// The planet's radius in metres where a cycle states none, which the shell of the clouds is a share of.
const PLANET: f32 = 6_360_000.0;
/// The steps a column of the layer is summed in.
const COLUMN_STEPS: u32 = 64;

/// The layer of clouds of one moment, its heights in metres above the sea.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudLayer {
    pub base: f32,
    pub top: f32,
    /// How much of the sky the layer covers, 0 to 1.
    pub cover: f32,
    /// The kind of cloud, 0 to 1: from low heaps to towers that fill the layer.
    pub kind: f32,
    /// The eccentricity of the light a cloud scatters, forward.
    pub anisotropy: f32,
    /// The radius of the shell the layer is curved on, in metres.
    pub sphere: f32,
    /// What the layer is lit by of the leading body's light, per channel: the local atmosphere's direct light colour times its intensity.
    pub light: [f32; 3],
}

/// The share of a cloud's density at `along` (0 at the base, 1 at the top) of the layer for a cloud of kind `kind`: rising over the first few hundredths, and gone where the tops end.
// ⚠ Inferred: the build looks this up in a table by height and kind (`CS_CloudDensityField`) whose texels are not read.
pub fn profile(along: f32, kind: f32) -> f32 {
    let reach = 0.35 + 0.65 * kind.clamp(0.0, 1.0);
    smoothstep(0.0, 0.07, along) * (1.0 - smoothstep(0.55 * reach, reach, along))
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl CloudLayer {
    /// The layer keyframe `k` states: its heights and anisotropy by the cycle's own variables, its cover and kind by the high-level ones laid into it.
    pub fn of(k: &Keyframe) -> Self {
        let get = |name: &str, default: f32| k.get_or(name, default);
        let base = get("local_atmosphere_cloud_height", 200.0);
        Self {
            base,
            top: get("local_atmosphere_cloud_height_top", 1500.0).max(base + 1.0),
            cover: get("cloud_cover", 0.0).clamp(0.0, 1.0),
            kind: get("cloud_type", 0.0).clamp(0.0, 1.0),
            anisotropy: get("local_atmosphere_anisotropy_high_density", 0.8).clamp(0.0, 0.99),
            // ⚠ Inferred: a shell of the planet's radius times the stated scale, so that the layer meets the horizon.
            sphere: get("sky_new_planet_radius", PLANET / 1000.0)
                * 1000.0
                * get("local_atmosphere_cloud_sphere_radius_scale", 0.05),
            light: ["r", "g", "b"].map(|c| {
                get(&format!("local_atmosphere_direct_light_color_{c}"), 1.0)
                    * get("local_atmosphere_direct_light_intensity", 1.0)
            }),
        }
    }

    /// What a whole cloud of the layer takes out of light straight up through it: its extinction over the depth its kind fills.
    pub fn optical_depth(&self) -> f32 {
        let step = 1.0 / COLUMN_STEPS as f32;
        let filled: f32 = (0..COLUMN_STEPS)
            .map(|i| profile((i as f32 + 0.5) * step, self.kind) * step)
            .sum();
        EXTINCTION * (self.top - self.base) * filled
    }

    /// The share of a light whose way has the up part `mu` that reaches the ground straight: through the gaps, and what a cloud lets through.
    pub fn direct_through(&self, mu: f32) -> f32 {
        let slant = self.optical_depth() / mu.max(0.05);
        (1.0 - self.cover) + self.cover * (-slant).exp()
    }

    /// The share of a light falling on the layer that its clouds send on down scattered, as a slab of their depth does.
    // ⚠ Inferred: the two-stream form of a slab, `1 / (1 + 3/4 τ (1 - g))`; the build traces it.
    pub fn diffuse_through(&self, mu: f32) -> f32 {
        let tau = self.optical_depth();
        let through = 1.0 / (1.0 + 0.75 * tau * (1.0 - self.anisotropy));
        self.cover * (through - (-tau / mu.max(0.05)).exp()).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(cover: f32, kind: f32) -> CloudLayer {
        let mut k = Keyframe::default();
        k.set("cloud_cover", cover);
        k.set("cloud_type", kind);
        k.set("local_atmosphere_anisotropy_high_density", 0.9);
        CloudLayer::of(&k)
    }

    #[test]
    fn a_clear_sky_lets_the_sun_through_whole_and_a_storm_lets_through_only_what_it_scatters() {
        let clear = layer(0.0, 0.0);
        assert_eq!(
            (clear.direct_through(0.9), clear.diffuse_through(0.9)),
            (1.0, 0.0)
        );
        let storm = layer(1.0, 1.0);
        assert!(
            storm.direct_through(0.9) < 1e-6,
            "{}",
            storm.direct_through(0.9)
        );
        let diffuse = storm.diffuse_through(0.9);
        assert!((0.1..0.4).contains(&diffuse), "{diffuse}");
        // Half the sky covered lets half the sun through, and towers take more of it than heaps.
        let half = layer(0.5, 0.5);
        assert!((half.direct_through(0.9) - 0.5).abs() < 1e-3);
        assert!(layer(1.0, 1.0).optical_depth() > layer(1.0, 0.2).optical_depth());
    }

    #[test]
    fn the_layer_is_lit_by_the_local_atmospheres_direct_light_colour_times_its_intensity() {
        assert_eq!(
            layer(0.5, 0.5).light,
            [1.0; 3],
            "unstated, the body's light whole"
        );
        let mut k = Keyframe::default();
        k.set("local_atmosphere_direct_light_intensity", 0.3);
        k.set("local_atmosphere_direct_light_color_b", 0.5);
        let light = CloudLayer::of(&k).light;
        assert!(
            (light[0] - 0.3).abs() < 1e-6 && (light[2] - 0.15).abs() < 1e-6,
            "{light:?}"
        );
    }

    #[test]
    fn a_cloud_is_densest_in_the_middle_of_its_kinds_reach_and_none_past_it() {
        assert_eq!(profile(0.0, 0.5), 0.0);
        assert_eq!(profile(0.9, 0.0), 0.0);
        assert!(profile(0.2, 0.0) > 0.99);
        assert!(profile(0.9, 1.0) > 0.0);
    }
}
