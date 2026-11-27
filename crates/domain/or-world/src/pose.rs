//! Where a camera line puts a camera, and the line itself as a script and a person write it.

use glam::Vec3;

/// The steepest a camera line looks up or down, in degrees.
pub const MOST_PITCH: f32 = 89.0;
/// The vertical fields of view a camera line may state, in degrees.
pub const FIELDS_OF_VIEW: std::ops::RangeInclusive<f32> = 5.0..=150.0;

/// Why a line is no camera line.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PoseError {
    #[error("`{0}` is not a number")]
    NotANumber(String),
    #[error("a camera is X Y Z YAW PITCH and an optional field of view, found {0} number(s)")]
    Count(usize),
    #[error("a pitch is within {MOST_PITCH} degrees of the horizon")]
    Pitch,
    #[error("a field of view is from {} to {} degrees", FIELDS_OF_VIEW.start(), FIELDS_OF_VIEW.end())]
    FieldOfView,
}

/// Where a camera line puts the camera: metres, then degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    /// Degrees about Z: 0 looks along +Y, and a growing angle turns towards −X.
    pub yaw: f32,
    /// Degrees above the horizon.
    pub pitch: f32,
    /// The vertical field of view in degrees; `None` leaves the camera's own.
    pub fov: Option<f32>,
}

impl Pose {
    /// The line a script and a person read back: `world camera X Y Z YAW PITCH FOV`.
    pub fn line(&self) -> String {
        let Vec3 { x, y, z } = self.position;
        let (yaw, pitch) = (self.yaw, self.pitch);
        match self.fov {
            Some(fov) => format!("world camera {x:.2} {y:.2} {z:.2} {yaw:.2} {pitch:.2} {fov:.2}"),
            None => format!("world camera {x:.2} {y:.2} {z:.2} {yaw:.2} {pitch:.2}"),
        }
    }

    /// The pose `line` states, with or without its leading `world camera`.
    pub fn read(line: &str) -> Result<Self, PoseError> {
        let mut words: Vec<&str> = line.split_whitespace().collect();
        if words.starts_with(&["world", "camera"]) {
            words.drain(..2);
        }
        let mut numbers = Vec::with_capacity(words.len());
        for word in words {
            let number = word.parse::<f32>().ok().filter(|n| n.is_finite());
            numbers.push(number.ok_or_else(|| PoseError::NotANumber(word.to_owned()))?);
        }
        let (x, y, z, yaw, pitch, fov) = match numbers[..] {
            [x, y, z, yaw, pitch] => (x, y, z, yaw, pitch, None),
            [x, y, z, yaw, pitch, fov] => (x, y, z, yaw, pitch, Some(fov)),
            _ => return Err(PoseError::Count(numbers.len())),
        };
        if pitch.abs() > MOST_PITCH {
            return Err(PoseError::Pitch);
        }
        if fov.is_some_and(|fov| !FIELDS_OF_VIEW.contains(&fov)) {
            return Err(PoseError::FieldOfView);
        }
        Ok(Self {
            position: Vec3::new(x, y, z),
            yaw,
            pitch,
            fov,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pose_reads_back_from_its_own_line() {
        let pose = Pose {
            position: Vec3::new(12.5, -340.25, 71.0),
            yaw: 271.5,
            pitch: -12.25,
            fov: Some(50.0),
        };
        assert_eq!(Pose::read(&pose.line()), Ok(pose));
        let bare = Pose { fov: None, ..pose };
        assert_eq!(Pose::read("12.5 -340.25 71 271.5 -12.25"), Ok(bare));
    }

    #[test]
    fn a_line_that_is_no_camera_is_refused_with_why() {
        assert_eq!(Pose::read("1 2 3"), Err(PoseError::Count(3)));
        assert_eq!(
            Pose::read("1 2 three 4 5"),
            Err(PoseError::NotANumber("three".into()))
        );
        assert_eq!(Pose::read("1 2 3 0 89.5"), Err(PoseError::Pitch));
        assert_eq!(Pose::read("1 2 3 0 0 170"), Err(PoseError::FieldOfView));
    }
}
