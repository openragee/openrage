//! A camera that flies through the world, and the frustum that says what a view sees.

use glam::{Mat4, Vec3, Vec4};

use crate::viewport::OrbitCamera;

/// A camera that flies: where it is, where it looks, and how fast it moves.
#[derive(Clone, Debug, PartialEq)]
pub struct FlyCamera {
    pub position: Vec3,
    /// Radians about Z, 0 looking along +Y.
    pub yaw: f32,
    /// Radians above the horizon, within ±89°.
    pub pitch: f32,
    /// Metres a second.
    pub speed: f32,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for FlyCamera {
    fn default() -> Self {
        let orbit = OrbitCamera::default();
        Self {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 20.0,
            fov_y: orbit.fov_y,
            near: orbit.near,
            far: orbit.far,
        }
    }
}

/// Radians a pixel of drag turns the camera by, as the orbit camera turns.
const LOOK_SENSITIVITY: f32 = 0.005;

/// How far ahead of the eye the orbit camera's target is put; the view does not depend on it.
const ORBIT_DISTANCE: f32 = 10.0;

impl FlyCamera {
    pub const MIN_SPEED: f32 = 1.0;
    pub const MAX_SPEED: f32 = 2000.0;
    const MAX_PITCH: f32 = 89.0 * std::f32::consts::PI / 180.0;

    /// The unit way it looks; Z is up.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(-sin_yaw * cos_pitch, cos_yaw * cos_pitch, sin_pitch)
    }

    /// The unit way to its right, level with the horizon.
    pub fn right(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, sin_yaw, 0.0)
    }

    /// Turn by a drag of `dx`, `dy` pixels.
    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * LOOK_SENSITIVITY;
        self.pitch = (self.pitch - dy * LOOK_SENSITIVITY).clamp(-Self::MAX_PITCH, Self::MAX_PITCH);
    }

    /// Move for `dt` seconds: `along` forward, `across` to the right and `up` the world's up, each −1 to 1, `fast` four times as fast.
    pub fn fly(&mut self, along: f32, across: f32, up: f32, fast: bool, dt: f32) {
        let way = self.forward() * along + self.right() * across + OrbitCamera::WORLD_UP * up;
        let speed = self.speed * if fast { 4.0 } else { 1.0 };
        self.position += way * speed * dt;
    }

    /// Scale the speed by a wheel turn of `notches`, a fifth a notch, held within the two bounds.
    pub fn scale_speed(&mut self, notches: f32) {
        self.speed = (self.speed * 1.2f32.powf(notches)).clamp(Self::MIN_SPEED, Self::MAX_SPEED);
    }

    /// The orbit camera that sees what this one sees, which the viewport renders through.
    pub fn as_orbit(&self) -> OrbitCamera {
        OrbitCamera {
            target: self.position + self.forward() * ORBIT_DISTANCE,
            // An orbit camera's angles say where its eye stands from its target, which is behind where this one looks
            yaw: self.yaw - std::f32::consts::FRAC_PI_2,
            pitch: -self.pitch,
            distance: ORBIT_DISTANCE,
            fov_y: self.fov_y,
            near: self.near,
            far: self.far,
        }
    }

    /// The view and projection of a frame of `aspect`, as the viewport draws it.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.as_orbit().view_proj(aspect)
    }
}

/// The six planes of what a view and projection see.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    /// Each plane as its unit normal, pointing inwards, and its offset: a point is inside where the dot product with it is not negative.
    planes: [Vec4; 6],
}

impl Frustum {
    /// The planes of `view_proj`, whose clip space is x and y within ±w and a depth from 0 to w.
    pub fn of(view_proj: Mat4) -> Self {
        let row = |i| view_proj.row(i);
        let w = row(3);
        let planes = [
            w + row(0),
            w - row(0),
            w + row(1),
            w - row(1),
            row(2),
            w - row(2),
        ]
        .map(|plane: Vec4| {
            let length = plane.truncate().length();
            if length > 0.0 {
                plane / length
            } else {
                plane
            }
        });
        Self { planes }
    }

    /// Whether a sphere is at least partly inside.
    pub fn sees_sphere(&self, centre: Vec3, radius: f32) -> bool {
        self.planes
            .iter()
            .all(|plane| plane.dot(centre.extend(1.0)) >= -radius)
    }

    /// Whether a sphere is wholly inside, so that everything in it is seen.
    pub fn holds_sphere(&self, centre: Vec3, radius: f32) -> bool {
        self.planes
            .iter()
            .all(|plane| plane.dot(centre.extend(1.0)) >= radius)
    }
}

#[cfg(test)]
mod tests;
