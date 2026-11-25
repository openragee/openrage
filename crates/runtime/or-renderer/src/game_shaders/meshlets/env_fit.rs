//! What a surface reflects of a light that comes evenly from all around, by the angle it is seen
//! at and its roughness: the table GTA VI bakes for its lighting program, integrated as the
//! game's own bake integrates it.

use std::f32::consts::TAU;

/// The places a side of the table: the cosine of the angle to the eye across, the roughness down.
pub(super) const SIDE: usize = 16;
/// The samples of each of the bake's integrals.
const SAMPLES: u32 = 1024;

/// The second number of sample `i`: its bits turned end for end, as a part of one.
fn turned(i: u32) -> f32 {
    i.reverse_bits() as f32 * 2f32.powi(-32)
}

/// What the table holds for a surface seen at the cosine `n_v` with the roughness `roughness`:
/// the share of an even light its facets reflect by the part of their reflectance that grows
/// towards grazing, the share they reflect whole, and the share its matte part sends back.
// The first two are the gloss the directional light is lit by, with its visibility term, over
// the half vectors a facet of that roughness takes; the third is the mean over the ways a matte
// surface takes light by, of the matte part as the bake states it.
pub(super) fn fit_at(n_v: f32, roughness: f32) -> [f32; 3] {
    let n_v = n_v.clamp(0.0, 1.0);
    let rough = roughness * roughness;
    let a = rough.max(0.001);
    let a2 = a * a;
    let across = (1.0 - n_v * n_v).max(0.0).sqrt();
    let seen = (n_v * n_v * (1.0 - a2) + a2).sqrt();
    let (mut grazing, mut whole) = (0.0, 0.0);
    for i in 0..SAMPLES {
        let first = (i as f32 / SAMPLES as f32).clamp(1e-6, 0.999_999);
        // A half vector by the spread of the facets, and the way the light comes that it
        // mirrors the eye's to.
        let cos2 = ((1.0 - first) / ((a2 - 1.0) * first + 1.0)).clamp(0.0, 1.0);
        let (n_h, sin_h) = (cos2.sqrt(), (1.0 - cos2).clamp(0.0, 1.0).sqrt());
        let v_h = n_v * n_h + across * sin_h * (TAU * turned(i)).sin();
        let n_l = (2.0 * v_h * n_h - n_v).clamp(0.0, 1.0);
        if n_l <= 0.0 {
            continue;
        }
        let lit = (n_l * n_l * (1.0 - a2) + a2).sqrt();
        let visible = 0.5 / (seen * n_l + lit * n_v);
        let weighed = visible * n_l * v_h.clamp(0.0, 1.0) / n_h.clamp(0.0, 1.0);
        whole += weighed;
        grazing += (1.0 - v_h).clamp(0.0, 1.0).powi(5) * weighed;
    }
    let from_view = (1.0 - n_v).powi(5);
    let mut matte = 0.0;
    for i in 0..SAMPLES {
        let first = (i as f32 / SAMPLES as f32).clamp(1e-6, 0.999_999);
        let n_l = (1.0 - first).sqrt();
        if n_l.clamp(0.0, 1.0) <= 0.0 {
            continue;
        }
        let (sin, cos) = (TAU * turned(i)).sin_cos();
        let half = glam::Vec3::new(cos * first.sqrt(), sin * first.sqrt() + across, n_l + n_v);
        let half = half.normalize();
        let l_h = (n_v * half.z + across * half.y).clamp(0.0, 1.0);
        let k = (2.0 * l_h * l_h + 0.5) * rough - 1.0;
        let from_light = (1.0 - n_l.clamp(0.0, 1.0)).powi(5);
        matte += (1.0 + from_view * k) * (1.0 - 0.337_748_3 * rough) * (1.0 + from_light * k);
    }
    let each = SAMPLES as f32;
    [4.0 * grazing / each, 4.0 * whole / each, matte / each]
}

/// The table, a row a roughness from nought to one and a place a cosine from nought to one.
pub(super) fn table() -> &'static [[f32; 3]; SIDE * SIDE] {
    static TABLE: std::sync::OnceLock<[[f32; 3]; SIDE * SIDE]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let at = |place: usize| place as f32 / (SIDE - 1) as f32;
        std::array::from_fn(|i| fit_at(at(i % SIDE), at(i / SIDE)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_smooth_surface_seen_head_on_reflects_an_even_light_whole_and_a_rough_one_loses_some() {
        // Head on, a mirror's facets all reflect the light the eye's way mirrors: the whole
        // share is one, and the part that grows towards grazing nought.
        let [grazing, whole, matte] = fit_at(1.0, 0.0);
        assert!(
            (whole - 1.0).abs() < 0.02 && grazing < 1e-3,
            "{grazing} {whole}"
        );
        // Its matte part sends back all but what leaves at grazing angles.
        assert!((0.94..0.96).contains(&matte), "{matte}");
        // A rough surface's facets shadow one another: less than the whole comes back, and a
        // rough matte surface sends back less too.
        let [grazing, whole, matte] = fit_at(1.0, 1.0);
        assert!(
            (0.25..0.4).contains(&whole) && grazing < 0.1 * whole,
            "{grazing} {whole}"
        );
        assert!((0.6..0.75).contains(&matte), "{matte}");
        // Towards grazing the part that grows there takes over.
        let [grazing, whole, _] = fit_at(0.05, 0.3);
        assert!(grazing > 0.3 * whole, "{grazing} {whole}");
    }

    #[test]
    fn the_table_holds_what_the_bake_gives_at_each_of_its_places() {
        let table = table();
        assert_eq!(table[0], fit_at(0.0, 0.0));
        assert_eq!(table[SIDE - 1], fit_at(1.0, 0.0));
        assert_eq!(table[SIDE * SIDE - 1], fit_at(1.0, 1.0));
        assert_eq!(table[SIDE * 5 + 3], fit_at(3.0 / 15.0, 5.0 / 15.0));
        assert!(table
            .iter()
            .flatten()
            .all(|share| share.is_finite() && *share >= 0.0));
    }
}
