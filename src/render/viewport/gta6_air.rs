//! The air over a GTA VI model, as a cycle's rows state it: how much of the sun's or the moon's
//! light comes through it to the ground, and what the sky it scatters gives a surface. One layer
//! of small particles, one of large ones and one of ozone round a planet, marched through as the
//! game's own programs march.

use std::f32::consts::PI;

use glam::Vec3;
use or_io::gta5::data::timecycle::Keyframe;

/// What each layer scatters or absorbs a kilometre where it is densest, before a cycle's own
/// colour and strength: the lengths of these are the strengths, and a cycle states a colour of
/// length one.
const RAYLEIGH: Vec3 = Vec3::new(5.802e-3, 13.558e-3, 33.1e-3);
const MIE: Vec3 = Vec3::splat(3.996e-3);
const MIE_ABSORBED: Vec3 = Vec3::splat(4.439e-4);
const OZONE: Vec3 = Vec3::new(0.65e-3, 1.881e-3, 0.085e-3);
/// Half the angle the sun's or the moon's disc is seen under, in radians.
const DISC: f32 = 0.008_726_646;
/// How high over the ground a model stands, in kilometres: the first entry of the game's table
/// of the light by height.
const STANDING: f32 = 0.0005;
/// The steps of a ray to the top of the air, of a ray of the sky and of a ray of the light
/// scattered more than once.
const STEPS_OUT: usize = 32;
const STEPS_SKY: usize = 24;
const STEPS_AROUND: usize = 20;
/// The table of the light that comes through, by how far the top is and how high the ray starts.
const THROUGH: (usize, usize) = (64, 16);
/// The table of the light scattered more than once, by the sun's height and the place's, a side.
const AROUND: usize = 8;

/// The air of one moment.
pub(super) struct Air {
    bottom: f32,
    top: f32,
    rayleigh: Vec3,
    rayleigh_height: f32,
    mie: Vec3,
    mie_absorbed: Vec3,
    mie_height: f32,
    /// How far forward the large particles scatter.
    forward: f32,
    ozone: Vec3,
    /// Below this height the ozone's density is the first line of a height, above it the second.
    ozone_width: f32,
    ozone_lines: [(f32, f32); 2],
    disc: f32,
    through: Vec<Vec3>,
    around: Vec<Vec3>,
}

/// How far along `way` from `from` a sphere of `radius` about the planet's middle is met: the
/// nearer meeting ahead, or none.
fn meets(from: Vec3, way: Vec3, radius: f32) -> Option<f32> {
    let b = from.dot(way);
    let within = b * b - (from.length_squared() - radius * radius);
    if within < 0.0 {
        return None;
    }
    let (near, far) = (-b - within.sqrt(), -b + within.sqrt());
    match (near >= 0.0, far >= 0.0) {
        (true, _) => Some(near),
        (false, true) => Some(far),
        _ => None,
    }
}

impl Air {
    /// The air `frame` states, each row the game's own where a cycle states none.
    pub(super) fn of(frame: &Keyframe) -> Self {
        let layer = |stem: &str, by_default: Vec3| {
            let unit = by_default.normalize();
            let part = |end: &str, lane: f32| frame.get_or(&format!("{stem}_col_{end}"), lane);
            Vec3::new(part("r", unit.x), part("g", unit.y), part("b", unit.z))
                * by_default.length()
                * frame.get_or(&format!("{stem}_intensity"), 1.0)
        };
        let bottom = frame.get_or("sky_new_planet_radius", 6360.0);
        let mut air = Self {
            bottom,
            top: bottom + frame.get_or("sky_new_karman_line", 100.0),
            rayleigh: layer("sky_new_ray_scattering", RAYLEIGH),
            rayleigh_height: frame.get_or("sky_new_ray_density_scale", 8.0),
            mie: layer("sky_new_mie_scattering", MIE),
            mie_absorbed: layer("sky_new_mie_absorption", MIE_ABSORBED),
            mie_height: frame.get_or("sky_new_mie_density_scale", 1.2),
            forward: frame.get_or("sky_new_mie_phase_eccentricity", 0.78),
            ozone: layer("sky_new_ozo_extinction", OZONE),
            ozone_width: frame.get_or("sky_new_ozo_0_width", 25.0),
            ozone_lines: [
                (
                    frame.get_or("sky_new_ozo_0_constant", -2.0 / 3.0),
                    frame.get_or("sky_new_ozo_0_linear", 1.0 / 15.0),
                ),
                (
                    frame.get_or("sky_new_ozo_1_constant", 8.0 / 3.0),
                    frame.get_or("sky_new_ozo_1_linear", -1.0 / 15.0),
                ),
            ],
            disc: 2.0 * DISC * frame.get_or("sky_new_direct_light_disk_size_mult", 1.0),
            through: Vec::new(),
            around: Vec::new(),
        };
        air.through = (0..THROUGH.0 * THROUGH.1)
            .map(|at| {
                let (u, v) = (at % THROUGH.0, at / THROUGH.0);
                let (radius, cosine) = air.ray_of(
                    u as f32 / (THROUGH.0 - 1) as f32,
                    v as f32 / (THROUGH.1 - 1) as f32,
                );
                air.marched_out(radius, cosine)
            })
            .collect();
        air.around = (0..AROUND * AROUND)
            .map(|at| {
                let cosine = (at % AROUND) as f32 / (AROUND - 1) as f32 * 2.0 - 1.0;
                let height = (at / AROUND) as f32 / (AROUND - 1) as f32;
                air.scattered_again(air.bottom + height * (air.top - air.bottom), cosine)
            })
            .collect();
        air
    }

    /// What the small and the large particles scatter and what all three take away, a kilometre,
    /// `height` kilometres over the ground.
    fn medium(&self, height: f32) -> (Vec3, Vec3, Vec3) {
        let rayleigh = self.rayleigh * (-height / self.rayleigh_height).exp();
        let thick = (-height / self.mie_height).exp();
        let (constant, linear) = self.ozone_lines[usize::from(height >= self.ozone_width)];
        let ozone = (linear * height + constant).clamp(0.0, 1.0);
        let taken = rayleigh + (self.mie + self.mie_absorbed) * thick + self.ozone * ozone;
        (rayleigh, self.mie * thick, taken)
    }

    /// The ray a place of the table of the light that comes through stands for: the radius it
    /// starts at and the cosine of its angle from straight up.
    fn ray_of(&self, u: f32, v: f32) -> (f32, f32) {
        let horizon = (self.top * self.top - self.bottom * self.bottom).sqrt();
        let along = horizon * v;
        let radius = (along * along + self.bottom * self.bottom).sqrt();
        let (least, most) = (self.top - radius, along + horizon);
        let far = least + u * (most - least);
        let cosine = match far == 0.0 {
            true => 1.0,
            false => (horizon * horizon - along * along - far * far) / (2.0 * radius * far),
        };
        (radius, cosine.clamp(-1.0, 1.0))
    }

    /// The light that comes through from the top of the air to `radius` along a ray of `cosine`
    /// from straight up, marched; a ray that meets the ground is marched to the ground.
    fn marched_out(&self, radius: f32, cosine: f32) -> Vec3 {
        let radius = radius.max(self.bottom + 0.05);
        let from = Vec3::new(0.0, 0.0, radius);
        let way = Vec3::new((1.0 - cosine * cosine).max(0.0).sqrt(), 0.0, cosine);
        let Some(far) = self.length_of(from, way) else {
            return Vec3::ONE;
        };
        let step = far / STEPS_OUT as f32;
        let depth: Vec3 = (0..STEPS_OUT)
            .map(|i| {
                let at = from + way * (step * (i as f32 + 0.5));
                self.medium(at.length() - self.bottom).2 * step
            })
            .sum();
        (-depth).exp()
    }

    /// How far a ray runs in the air: to the ground where it meets it, else to the top.
    fn length_of(&self, from: Vec3, way: Vec3) -> Option<f32> {
        let (ground, top) = (meets(from, way, self.bottom), meets(from, way, self.top));
        match (ground, top) {
            (Some(ground), Some(top)) if ground > 0.0 => Some(ground.min(top)),
            (_, top) => top.filter(|far| *far > 0.0),
        }
    }

    /// The light that comes through to `radius` along `cosine`, read between four places of the
    /// table.
    fn through(&self, radius: f32, cosine: f32) -> Vec3 {
        let horizon = (self.top * self.top - self.bottom * self.bottom).sqrt();
        let along = (radius * radius - self.bottom * self.bottom)
            .max(0.0)
            .sqrt();
        let behind = radius * radius * (cosine * cosine - 1.0) + self.top * self.top;
        let far = (-radius * cosine + behind.max(0.0).sqrt()).max(0.0);
        let (least, most) = (self.top - radius, along + horizon);
        let u = ((far - least) / (most - least)).clamp(0.0, 1.0) * (THROUGH.0 - 1) as f32;
        let v = (along / horizon).clamp(0.0, 1.0) * (THROUGH.1 - 1) as f32;
        let (u0, v0) = (
            (u as usize).min(THROUGH.0 - 2),
            (v as usize).min(THROUGH.1 - 2),
        );
        let held = |u: usize, v: usize| self.through[v * THROUGH.0 + u];
        let (du, dv) = (u - u0 as f32, v - v0 as f32);
        (held(u0, v0).lerp(held(u0 + 1, v0), du))
            .lerp(held(u0, v0 + 1).lerp(held(u0 + 1, v0 + 1), du), dv)
    }

    /// The light of a body `towards` that reaches `at`: none where the planet stands between.
    fn lit(&self, at: Vec3, towards: Vec3) -> Vec3 {
        match meets(at, towards, self.bottom) {
            Some(_) => Vec3::ZERO,
            None => self.through(at.length(), at.normalize().dot(towards)),
        }
    }

    /// The light scattered more than once at `radius` under a body whose way is `cosine` from
    /// straight up, for each unit of its light: the light scattered twice over every way, and
    /// the share each further scattering keeps of it.
    // ponytail: the game's own program for this table was not followed; this is the published
    // estimate its sky program's use of the table fits, without the second, boosted reading.
    fn scattered_again(&self, radius: f32, cosine: f32) -> Vec3 {
        let from = Vec3::new(0.0, 0.0, radius.clamp(self.bottom + 0.05, self.top - 0.05));
        let towards = Vec3::new((1.0 - cosine * cosine).max(0.0).sqrt(), 0.0, cosine);
        let (mut twice, mut share) = (Vec3::ZERO, Vec3::ZERO);
        let side = 8;
        for at in 0..side * side {
            let (i, j) = ((at % side) as f32 + 0.5, (at / side) as f32 + 0.5);
            let (turn, tilt) = (
                2.0 * PI * i / side as f32,
                (2.0 * j / side as f32 - 1.0).acos(),
            );
            let way = Vec3::new(tilt.sin() * turn.cos(), tilt.sin() * turn.sin(), tilt.cos());
            let (light, kept) = self.marched(from, way, STEPS_AROUND, |place, small, large| {
                (
                    (small + large) * self.lit(place, towards) / (4.0 * PI),
                    small + large,
                )
            });
            twice += light / (side * side) as f32;
            share += kept / (side * side) as f32;
        }
        twice / (Vec3::ONE - share)
    }

    /// A ray from `from` along `way` in `steps`: what `source` says is scattered towards the eye
    /// at each place, as a pair, gathered through the air between.
    fn marched(
        &self,
        from: Vec3,
        way: Vec3,
        steps: usize,
        source: impl Fn(Vec3, Vec3, Vec3) -> (Vec3, Vec3),
    ) -> (Vec3, Vec3) {
        let Some(far) = self.length_of(from, way) else {
            return (Vec3::ZERO, Vec3::ZERO);
        };
        let step = far / steps as f32;
        let (mut first, mut second, mut left) = (Vec3::ZERO, Vec3::ZERO, Vec3::ONE);
        for i in 0..steps {
            let place = from + way * (step * (i as f32 + 0.5));
            let (small, large, taken) = self.medium(place.length() - self.bottom);
            let through = (-taken * step).exp();
            let (a, b) = source(place, small, large);
            // What a step sends on, with what it takes of its own light on the way.
            let gathered = left * (Vec3::ONE - through) / taken.max(Vec3::splat(1e-12));
            first += a * gathered;
            second += b * gathered;
            left *= through;
        }
        (first, second)
    }

    /// The light the sky sends along `way` to a model, for each unit of the light of the body
    /// `towards`.
    pub(super) fn sky(&self, way: Vec3, towards: Vec3) -> Vec3 {
        let from = Vec3::new(0.0, 0.0, self.bottom + STANDING);
        let cosine = way.dot(towards);
        let g = self.forward;
        let small = 3.0 / (16.0 * PI) * (1.0 + cosine * cosine);
        let large = 3.0 / (8.0 * PI) * (1.0 - g * g) * (1.0 + cosine * cosine)
            / ((2.0 + g * g) * (1.0 + g * g - 2.0 * g * cosine).powf(1.5));
        let side = (AROUND - 1) as f32;
        let (seen, _) = self.marched(from, way, STEPS_SKY, |place, by_small, by_large| {
            let up = place.normalize();
            let u = (up.dot(towards) * 0.5 + 0.5) * side;
            let v =
                ((place.length() - self.bottom) / (self.top - self.bottom)).clamp(0.0, 1.0) * side;
            let (u0, v0) = ((u as usize).min(AROUND - 2), (v as usize).min(AROUND - 2));
            let held = |u: usize, v: usize| self.around[v * AROUND + u];
            let (du, dv) = (u - u0 as f32, v - v0 as f32);
            let again = (held(u0, v0).lerp(held(u0 + 1, v0), du))
                .lerp(held(u0, v0 + 1).lerp(held(u0 + 1, v0 + 1), du), dv);
            let once = (by_small * small + by_large * large) * self.lit(place, towards);
            (once + (by_small + by_large) * again, Vec3::ZERO)
        });
        seen
    }

    /// The tangent of the angle half the lighting body's disc is seen under.
    pub(super) fn disc_tangent(&self) -> f32 {
        (self.disc / 2.0).tan()
    }

    /// The ways the sky is looked along to gather it, each with the angle its patch of the sky
    /// fills: six rings of twelve over the upper half, from the top down.
    pub(super) fn ways() -> impl Iterator<Item = (Vec3, f32)> {
        let (rings, around) = (6, 12);
        (0..rings * around).map(move |at| {
            let tilt = ((at / around) as f32 + 0.5) / rings as f32 * (PI / 2.0);
            let turn = ((at % around) as f32 + 0.5) / around as f32 * 2.0 * PI;
            let way = Vec3::new(tilt.sin() * turn.cos(), tilt.sin() * turn.sin(), tilt.cos());
            let patch = tilt.sin() * (PI / 2.0 / rings as f32) * (2.0 * PI / around as f32);
            (way, patch)
        })
    }

    /// What the whole sky gives a surface that faces up, for each unit of the light of the body
    /// `towards`.
    #[cfg(test)]
    pub(super) fn sky_on_the_ground(&self, towards: Vec3) -> Vec3 {
        (Self::ways().map(|(way, patch)| self.sky(way, towards) * way.z * patch)).sum()
    }

    /// How much of the light of the body `towards` comes through to a model: the mean over its
    /// disc, in eight strips by height, of what comes through the air, a strip under the
    /// horizon giving none.
    pub(super) fn light_through(&self, towards: Vec3) -> Vec3 {
        let radius = self.bottom + STANDING;
        let strip = self.disc / 8.0;
        let first = towards.z.clamp(-1.0, 1.0).acos() - self.disc / 2.0 + strip / 2.0;
        let dip = (self.bottom / (radius * radius - self.bottom * self.bottom).sqrt()).atan();
        let horizon = PI - dip;
        let mut through = Vec3::ZERO;
        for i in 0..8 {
            let angle = first + i as f32 * strip;
            if angle > horizon {
                break;
            }
            let over = ((horizon - angle) / strip).clamp(0.0, 1.0);
            through += self.through(radius, angle.cos()) * over / 8.0;
        }
        through
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn air() -> Air {
        Air::of(&Keyframe::default())
    }

    #[test]
    fn the_light_comes_through_whole_from_overhead_and_red_and_dim_from_the_horizon() {
        let air = air();
        let over = air.light_through(Vec3::Z);
        // Straight down through eight kilometres of small particles and what is under them:
        // the blue loses most, as the layers' numbers say.
        let depth = RAYLEIGH * 8.0 * (1.0 - (-100f32 / 8.0).exp());
        assert!(over.x > over.y && over.y > over.z, "{over:?}");
        assert!(over.z < (-depth.z).exp() && over.z > 0.6, "{over:?}");
        let low = air.light_through(Vec3::new(0.999, 0.0, 0.035).normalize());
        assert!(low.x < over.x * 0.6 && low.z < low.x * 0.2, "{low:?}");
        // Under the horizon by more than its disc: none.
        assert_eq!(
            air.light_through(Vec3::new(0.99, 0.0, -0.1).normalize()),
            Vec3::ZERO
        );
        // Half the disc under: about half of what the low sun gives.
        let dip =
            (air.bottom / ((air.bottom + STANDING).powi(2) - air.bottom.powi(2)).sqrt()).atan();
        let half = air.light_through(Vec3::new(dip.sin(), 0.0, -dip.cos()));
        assert!(half.x > 0.0 && half.x < low.x * 0.7, "{half:?} {low:?}");
    }

    #[test]
    fn a_ray_of_the_table_is_read_back_where_it_was_marched() {
        let air = air();
        for (u, v) in [(0.0, 0.0), (0.5, 0.25), (1.0, 1.0), (0.2, 0.9)] {
            let (radius, cosine) = air.ray_of(u, v);
            let (read, marched) = (air.through(radius, cosine), air.marched_out(radius, cosine));
            assert!(
                (read - marched).abs().max_element() < 0.03,
                "{u} {v}: {read:?} {marched:?}"
            );
        }
    }

    #[test]
    fn the_sky_is_blue_by_day_gives_a_fraction_of_the_sun_and_fades_as_it_sets() {
        let air = air();
        let noon = air.sky_on_the_ground(Vec3::new(0.3, 0.0, 0.954).normalize());
        assert!(noon.z > noon.y && noon.y > noon.x, "{noon:?}");
        // Of each unit the sun gives, the sky gives a surface that faces up a tenth or two.
        assert!((0.05..0.35).contains(&noon.y), "{noon:?}");
        let dusk = air.sky_on_the_ground(Vec3::new(0.998, 0.0, 0.05).normalize());
        let night = air.sky_on_the_ground(Vec3::new(0.94, 0.0, -0.34).normalize());
        assert!(
            dusk.y < noon.y * 0.5 && night.y < dusk.y * 0.05,
            "{dusk:?} {night:?}"
        );
        // The zenith is bluer than the way towards a low sun.
        let low = Vec3::new(0.998, 0.0, 0.05).normalize();
        let (zenith, at_sun) = (
            air.sky(Vec3::Z, low),
            air.sky(Vec3::new(0.9, 0.0, 0.43).normalize(), low),
        );
        assert!(
            zenith.z / zenith.x > at_sun.z / at_sun.x,
            "{zenith:?} {at_sun:?}"
        );
    }
}
