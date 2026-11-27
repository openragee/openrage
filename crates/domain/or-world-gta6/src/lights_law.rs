//! GTA VI's law of a local light, `E = I · w · a / d²` with the window `w = (1 − (d²/R²)²)²` and a
//! spot's angular factor `a`, drawn by passes that fade a light by `(1 − d²/r²)^e` and a straight
//! ramp across the cone: each light as a few nested volumes whose sum follows the law.

/// The exponent every nested volume fades by.
pub const SHELL_EXPONENT: f32 = 2.0;

/// Each nested volume: its radius squared as a share of the light's, and its strength times `R²`.
/// A least-squares fit of `(1 − t²)²/t`, `t = d²/R²`, between `R/32` and `R`; how near it keeps is
/// `the_shells_follow_the_window_over_the_distance_squared`.
pub const SHELLS: [(f32, f32); 8] = [
    (1.0, 4.426_201),
    (0.4, 7.504_387),
    (0.16, 16.259_49),
    (0.064, 43.694_015),
    (0.025_6, 106.876_38),
    (0.010_24, 267.191_7),
    (0.004_096, 683.169_2),
    (0.001_638_4, 1_558.555_3),
];

/// The window over the distance squared, times `R²`, at `t = d²/R²`.
#[cfg(test)]
fn law(t: f32) -> f32 {
    let w = (1.0 - t * t).clamp(0.0, 1.0);
    w * w / t.max(1e-12)
}

/// What the nested volumes give together at `t = d²/R²`, times `R²`.
#[cfg(test)]
fn shells(t: f32) -> f32 {
    (SHELLS.iter())
        .map(|(share, strength)| strength * (1.0 - t / share).max(0.0).powf(SHELL_EXPONENT))
        .sum()
}

/// A spot's angular factor at `x`, how far a direction lies from the outer cone (0) to the inner (1).
pub fn angular(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * (x - x * x + x.powi(4) * (3.0 - 2.0 * x * x))
}

/// The cones a spot of half angles `inner` and `outer` (degrees) is drawn as, each fading straight
/// from its outer edge to `inner`, as its outer half angle and its share of the light: two ramps
/// whose sum meets [`angular`] at the cone's edge, half way across it and inside it.
pub fn spot_cones(inner: f32, outer: f32) -> [(f32, f32); 2] {
    let (cos_in, cos_out) = (inner.to_radians().cos(), outer.to_radians().cos());
    if cos_in <= cos_out {
        return [(outer, 1.0), (outer, 0.0)];
    }
    let half_way = (cos_out + (cos_in - cos_out) / 2.0).acos().to_degrees();
    let wide = 2.0 * angular(0.5);
    [(outer, wide), (half_way, 1.0 - wide)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shells_follow_the_window_over_the_distance_squared() {
        // Relative where the law is large, to 2 % of R⁻² where it nears nought at the edge.
        let worst = (1..=1000)
            .map(|i| {
                let d = 1.0 / 16.0 + (0.98 - 1.0 / 16.0) * i as f32 / 1000.0;
                let t = d * d;
                (shells(t) - law(t)).abs() / (law(t) + 0.02)
            })
            .fold(0.0, f32::max);
        assert!(worst < 0.11, "{worst}");
        // Nothing past the light's radius, and a straight fall with the exponent the pass is given would not do
        assert_eq!(shells(1.0), 0.0);
        let one = |t: f32| law(0.25) * (1.0 - t).powi(2) / 0.75f32.powi(2);
        assert!((one(0.01) - law(0.01)).abs() / law(0.01) > 0.5);
    }

    #[test]
    fn two_ramps_meet_the_angular_factor_where_they_are_fitted() {
        let (inner, outer) = (25.0f32, 70.0f32);
        let [(o1, s1), (o2, s2)] = spot_cones(inner, outer);
        let (cos_in, cos_out) = (inner.to_radians().cos(), outer.to_radians().cos());
        // A ramp as the pass draws it: from the outer edge it is given to the inner.
        let ramp = |c: f32, outer: f32| {
            let cos_o = outer.to_radians().cos();
            ((c - cos_o) / (cos_in - cos_o)).clamp(0.0, 1.0)
        };
        let at = |x: f32| {
            let c = cos_out + x * (cos_in - cos_out);
            s1 * ramp(c, o1) + s2 * ramp(c, o2)
        };
        for x in [0.0, 0.5, 1.0] {
            assert!(
                (at(x) - angular(x)).abs() < 1e-4,
                "{x}: {} {}",
                at(x),
                angular(x)
            );
        }
        assert!((at(0.25) - angular(0.25)).abs() < 0.06);
        assert!((at(0.75) - angular(0.75)).abs() < 0.06);
        // One straight ramp, as the pass draws alone, is far off half way across
        assert!((0.5 - angular(0.5)).abs() > 0.25);
    }

    #[test]
    fn a_cone_with_no_room_inside_it_is_one_ramp() {
        assert_eq!(spot_cones(40.0, 40.0), [(40.0, 1.0), (40.0, 0.0)]);
    }
}
