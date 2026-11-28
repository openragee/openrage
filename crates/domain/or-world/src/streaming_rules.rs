//! The rules of streaming the forest and the portal walk themselves call, as pure functions and the constants they read.

use glam::Vec3;

/// Distance added to a map file's sphere radius, metres; every map tier has the same, read from this build.
pub const MAP_TIER_DISTANCE: f32 = 50.0;

/// Smallest half extent, metres, below which a map file has no sphere; read from this build.
pub const MIN_SPHERE_HALF_EXTENT: f32 = 1.0;

/// Bias added before truncating a fade alpha, about 0.0255, unitless; read from this build.
pub const FADE_ALPHA_BIAS: f32 = f32::from_bits(0x3CD0_E560);

/// Portal flag bit of a link portal, through which a walk continues into the other instance's room-0 portal; read from this build.
pub const PORTAL_LINK: u32 = 0x02;

/// Portal flag bit of a mirror, which every walk that tests it skips; read from this build.
pub const PORTAL_MIRROR: u32 = 0x04;

/// Half size of the box about the camera that an interior's bounds must overlap for the portal factor, metres; read from this build.
pub const PORTAL_FACTOR_CAMERA_BOX: f32 = 4.0;

/// A map file's streaming sphere, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamingSphere {
    /// Centre of the file's box, metres.
    pub centre: Vec3,
    /// Smallest half extent of the box plus the tier distance, metres.
    pub radius: f32,
}

impl StreamingSphere {
    /// Whether a point, metres, is inside or on the sphere.
    pub fn contains(&self, point: Vec3) -> bool {
        (point - self.centre).length_squared() <= self.radius * self.radius
    }
}

/// The streaming sphere of a map file's box B, metres, or none when a half extent is under 1 m; which of the file's two boxes is B is not known.
pub fn map_streaming_sphere(box_min: Vec3, box_max: Vec3) -> Option<StreamingSphere> {
    let half: Vec3 = 0.5 * (box_max - box_min);
    // A NaN or infinite lane is a malformed box; f32::min would drop a NaN lane, so the whole vector is tested.
    if !half.is_finite() {
        return None;
    }
    let smallest: f32 = half.x.min(half.y).min(half.z);
    if smallest < MIN_SPHERE_HALF_EXTENT {
        return None;
    }
    Some(StreamingSphere {
        centre: 0.5 * (box_min + box_max),
        radius: smallest + MAP_TIER_DISTANCE,
    })
}

/// The distance alpha, 0..=255: 255 up to the fade start, 0 past start plus range; a range of 0 or less gives 0, as the game's unguarded division does (inferred).
pub fn distance_alpha(distance: f32, start: f32, range: f32) -> u8 {
    let end: f32 = start + range;
    // max then min, not f32::clamp, which panics when the range is negative. Both drop a NaN distance, which so takes the start and gives 255.
    let clamped: f32 = distance.max(start).min(end);
    let value: f32 = (end - clamped) * 255.0 / range + FADE_ALPHA_BIAS;
    // The game truncates toward zero; `as` maps the NaN of a zero range to 0.
    value as u8
}

#[cfg(test)]
#[path = "streaming_rules_tests.rs"]
mod tests;
