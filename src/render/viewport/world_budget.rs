//! How much of the world the device keeps and takes in a frame, what goes first when it holds
//! too much, and what is asked for first while the camera moves.

use std::time::{Duration, Instant};

use glam::Vec3;

/// The bytes of models, and of textures, put on the device in one frame; the rest wait for the next.
const MODELS_A_FRAME: u64 = 48 << 20;
const TEXTURES_A_FRAME: u64 = 8 << 20;
/// How long a frame spends putting models and textures on the device, all told, once each kind has put something: a quarter of a frame at 60 a second.
// ⚠ Bytes alone do not bound the time: a frame of many small models paid tens of milliseconds under the byte budget.
const TAKING: Duration = Duration::from_millis(4);
/// The bytes of models, and of textures, kept where the device does not say how much memory it
/// has: those of a card of 8 GiB.
const MODELS_KEPT: u64 = 3 << 30;
const TEXTURES_KEPT: u64 = 1 << 30;
/// The least of either kept, however little memory the device says it has.
const LEAST_KEPT: u64 = 256 << 20;
/// The bytes of a page's vertex buffer and of its index buffer, where the device takes buffers that large.
const PAGE_VERTICES: u64 = 192 << 20;
const PAGE_INDICES: u64 = 64 << 20;
/// How far ahead in time the camera is taken to be when what loads first is chosen, in seconds,
/// and the most it is taken to move in that time, in metres.
const LEAD: f32 = 0.5;
const MOST_LEAD: f32 = 150.0;

/// What the world may hold on a device and put there in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Budgets {
    /// The bytes of models, and of textures, kept, past which those that do not draw go.
    pub models_kept: u64,
    pub textures_kept: u64,
    pub models_a_frame: u64,
    pub textures_a_frame: u64,
    /// The bytes of a page's vertex buffer and of its index buffer.
    pub page: (u64, u64),
    /// The largest buffer the device makes, in bytes.
    pub most_buffer: u64,
    /// The longest side a texture is asked for at, in pixels.
    pub most_side: u32,
}

/// The variable a person sets to the device memory, in MiB, the world's budgets are worked out from, where the device states none.
pub(super) const MEMORY_VARIABLE: &str = "OPENRAGE_WORLD_MEMORY_MIB";

/// The bytes of device memory `value` of [`MEMORY_VARIABLE`] states: a whole number of MiB, none for anything else.
pub(super) fn memory_set(value: Option<&str>) -> Option<u64> {
    let mib: u64 = value?.trim().parse().ok()?;
    mib.checked_mul(1 << 20).filter(|bytes| *bytes > 0)
}

impl Budgets {
    /// The budgets of a device of these limits and of `memory` bytes, where it says how many.
    pub(super) fn of(limits: &wgpu::Limits, memory: Option<u64>) -> Self {
        // Half of the memory is the world's, three parts of four models: the fixed budgets are these of 8 GiB.
        let (models_kept, textures_kept) = match memory {
            Some(memory) => (
                (memory / 8 * 3).max(LEAST_KEPT),
                (memory / 8).max(LEAST_KEPT),
            ),
            None => (MODELS_KEPT, TEXTURES_KEPT),
        };
        Self {
            models_kept,
            textures_kept,
            models_a_frame: MODELS_A_FRAME,
            textures_a_frame: TEXTURES_A_FRAME,
            page: (
                PAGE_VERTICES.min(limits.max_buffer_size),
                PAGE_INDICES.min(limits.max_buffer_size),
            ),
            most_buffer: limits.max_buffer_size,
            most_side: limits.max_texture_dimension_2d,
        }
    }
}

/// What the last frame of the world cost the CPU, and what it laid out, for a measurement to read.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WorldCost {
    /// The whole of the frame's world on the CPU, and whether it laid the world out again.
    pub refresh: Duration,
    pub laid: bool,
    /// Of the frame: putting what arrived on the device, and working out the day and the lights about the camera.
    pub taking: Duration,
    pub lights: Duration,
    /// Of the last layout: drawing the occluders, the cull and the placing, and all of it.
    pub occlude: Duration,
    pub cull: Duration,
    pub layout: Duration,
    /// Chunks the selection gave, and those in view the occluders hid.
    pub chunks: usize,
    pub hidden: usize,
    /// Occluder boxes and models drawn.
    pub occluders: (usize, usize),
    /// Instances laid out, and those of them laid out for their shadow alone.
    pub laid_instances: usize,
    pub shadow_only: usize,
    /// Bytes of models and of textures put on the device this frame.
    pub uploaded: (u64, u64),
    /// Models and textures let go so far, and those of them that came back.
    pub evicted: (u64, u64),
    pub reread: (u64, u64),
}

/// How many entries each of the world's maps and sets holds, for a census of what grows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldSets {
    /// Models on the device, and those of them that did not read.
    pub models: usize,
    pub missing: usize,
    /// Textures held or known missing.
    pub textures: usize,
    /// Material slots, those of them free, and the keys they are found by.
    pub materials: usize,
    pub free_materials: usize,
    pub material_keys: usize,
    pub palettes: usize,
    /// The models and the textures remembered as let go.
    pub gone: (usize, usize),
}

impl super::WorldGpu {
    /// How many entries each map and set holds now.
    pub(super) fn sets(&self) -> WorldSets {
        let missing = (self.models.values())
            .filter(|slot| matches!(slot, super::Slot::Missing))
            .count();
        WorldSets {
            models: self.models.len(),
            missing,
            textures: self.textures.map.len(),
            materials: self.materials.len(),
            free_materials: self.free_materials.len(),
            material_keys: self.material_of.len(),
            palettes: self.palettes.len(),
            gone: (self.gone.0.len(), self.gone.1.len()),
        }
    }
}

impl super::RenderBridge {
    /// How many entries each of the world's maps and sets holds.
    pub fn world_sets(&self) -> WorldSets {
        self.world.sets()
    }
}

/// The most keys a set of what was let go holds before it starts again: a key let go before that is not counted when it comes back.
const GONE_KEPT: usize = 1 << 16;

/// Remember `key` among `gone`, which starts again once it holds [`GONE_KEPT`].
pub(super) fn remember<K: Eq + std::hash::Hash>(gone: &mut std::collections::HashSet<K>, key: K) {
    if gone.len() >= GONE_KEPT {
        *gone = Default::default();
    }
    gone.insert(key);
}

/// The longest a frame spends putting what arrived on the device, however slow the frames are.
const MOST_TAKING: Duration = Duration::from_millis(250);

/// How long a frame after one of `last` spends putting what arrived on the device: [`TAKING`], or a quarter of the last frame where that is longer, and how many times the bytes a frame puts grow with it.
// ⚠ A fixed 4 ms of frames that each took a second put a model and a texture a frame on the device: a street scene settled in tens of minutes, its readers idle.
pub(super) fn taking(last: Option<Duration>) -> (Duration, u64) {
    let time = last.map_or(TAKING, |last| (last / 4).clamp(TAKING, MOST_TAKING));
    (time, (time.as_micros() / TAKING.as_micros()) as u64)
}

/// Whether a frame that began putting things on the device at `since`, and has put `bytes` of this kind so far, puts another within `time`.
pub(super) fn may_take(since: Instant, bytes: u64, time: Duration) -> bool {
    bytes == 0 || since.elapsed() < time
}

/// Something on the device that did not draw this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Idle<K> {
    pub key: K,
    pub bytes: u64,
    /// Whether the selection still holds it, so that letting it go has it read again.
    pub held: bool,
    /// How far from the camera it last drew.
    pub away: f32,
}

/// Those of `idle` that go so that `over` bytes are let go: what the selection no longer holds
/// first, and of each kind the farthest first.
pub(super) fn evicted<K>(mut idle: Vec<Idle<K>>, over: u64) -> Vec<Idle<K>> {
    idle.sort_unstable_by(|a, b| a.held.cmp(&b.held).then(b.away.total_cmp(&a.away)));
    let mut freed = 0;
    (idle.into_iter())
        .take_while(|idle| {
            let more = freed < over;
            freed += idle.bytes;
            more
        })
        .collect()
}

/// Where the camera at `eye` is taken to be shortly, having been at `was` `dt` seconds ago: what
/// is nearest there is asked for first, so that what is ahead of a moving camera comes first.
pub(super) fn ahead(eye: Vec3, was: Vec3, dt: f32) -> Vec3 {
    if dt <= 0.0 {
        return eye;
    }
    eye + ((eye - was) / dt * LEAD).clamp_length_max(MOST_LEAD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_states_no_memory_takes_the_fixed_budgets() {
        let limits = wgpu::Limits::default();
        let fixed = Budgets::of(&limits, None);
        assert_eq!((fixed.models_kept, fixed.textures_kept), (3 << 30, 1 << 30));
        assert_eq!(fixed.most_side, limits.max_texture_dimension_2d);
        // A card of 8 GiB is the one the fixed budgets are of.
        assert_eq!(Budgets::of(&limits, Some(8 << 30)), fixed);
    }

    #[test]
    fn the_budgets_follow_the_memory_down_to_a_floor() {
        let limits = wgpu::Limits::default();
        let large = Budgets::of(&limits, Some(24 << 30));
        assert_eq!((large.models_kept, large.textures_kept), (9 << 30, 3 << 30));
        let small = Budgets::of(&limits, Some(1 << 30));
        assert_eq!(small.models_kept, 384 << 20);
        assert_eq!(small.textures_kept, LEAST_KEPT);
    }

    #[test]
    fn the_memory_a_person_sets_is_whole_mebibytes() {
        assert_eq!(memory_set(Some("4096")), Some(4 << 30));
        assert_eq!(memory_set(Some(" 512 ")), Some(512 << 20));
        for bad in ["", "0", "-1", "4 GiB", "1.5", "99999999999999999999"] {
            assert_eq!(memory_set(Some(bad)), None, "{bad}");
        }
        assert_eq!(memory_set(None), None);
        // Four GiB set: the world keeps half of it, three parts of four models.
        let limits = wgpu::Limits::default();
        let set = Budgets::of(&limits, memory_set(Some("4096")));
        assert_eq!(
            (set.models_kept, set.textures_kept),
            (1536 << 20, 512 << 20)
        );
    }

    #[test]
    fn a_page_is_no_larger_than_the_device_makes_a_buffer() {
        let limits = wgpu::Limits {
            max_buffer_size: 32 << 20,
            ..Default::default()
        };
        let budgets = Budgets::of(&limits, None);
        assert_eq!(budgets.page, (32 << 20, 32 << 20));
        assert_eq!(budgets.most_buffer, 32 << 20);
    }

    fn idle(key: u32, bytes: u64, held: bool, away: f32) -> Idle<u32> {
        Idle {
            key,
            bytes,
            held,
            away,
        }
    }

    #[test]
    fn the_farthest_goes_first_and_what_the_selection_holds_last() {
        let all = vec![
            idle(1, 10, false, 5.0),
            idle(2, 10, true, 900.0),
            idle(3, 10, false, 300.0),
            idle(4, 10, false, 40.0),
        ];
        let keys =
            |over| -> Vec<u32> { (evicted(all.clone(), over).iter()).map(|i| i.key).collect() };
        assert_eq!(keys(0), [0u32; 0]);
        assert_eq!(keys(1), [3]);
        assert_eq!(keys(10), [3]);
        assert_eq!(keys(11), [3, 4]);
        // What the selection holds goes only once the rest has.
        assert_eq!(keys(31), [3, 4, 1, 2]);
        // And asking for more than there is lets everything go.
        assert_eq!(keys(1 << 40), [3, 4, 1, 2]);
    }

    #[test]
    fn a_frame_takes_one_of_a_kind_whatever_the_time_and_more_only_within_it() {
        let now = Instant::now();
        assert!(may_take(now, 0, TAKING) && may_take(now, 1 << 20, TAKING));
        let Some(long_ago) = now.checked_sub(TAKING * 2) else {
            return;
        };
        assert!(may_take(long_ago, 0, TAKING), "the first is always taken");
        assert!(!may_take(long_ago, 1, TAKING));
        assert!(may_take(long_ago, 1, TAKING * 3));
    }

    #[test]
    fn a_slow_frame_takes_a_quarter_of_its_time_and_the_bytes_grow_with_it() {
        // At sixty frames a second, and on the first frame, the fixed time.
        assert_eq!(taking(None), (TAKING, 1));
        assert_eq!(taking(Some(Duration::from_millis(16))), (TAKING, 1));
        // A frame of 80 ms: 20 ms, five times the bytes.
        assert_eq!(taking(Some(Duration::from_millis(80))), (TAKING * 5, 5));
        // However slow, at most a quarter of a second.
        assert_eq!(taking(Some(Duration::from_secs(9))), (MOST_TAKING, 62));
    }

    #[test]
    fn what_was_let_go_is_remembered_up_to_a_bound() {
        let mut gone = std::collections::HashSet::new();
        for key in 0..GONE_KEPT {
            remember(&mut gone, key);
        }
        assert_eq!(gone.len(), GONE_KEPT);
        remember(&mut gone, GONE_KEPT);
        assert_eq!(gone.len(), 1, "it starts again");
        assert!(gone.contains(&GONE_KEPT));
    }

    #[test]
    fn a_moving_camera_is_taken_to_be_ahead_of_itself() {
        let eye = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(ahead(eye, eye, 0.016), eye);
        assert_eq!(ahead(eye, Vec3::ZERO, 0.0), eye);
        // Ten metres in a second: five metres ahead in half a second.
        assert_eq!(ahead(eye, Vec3::ZERO, 1.0), Vec3::new(15.0, 0.0, 0.0));
        // A jump is no speed to follow: the lead is held.
        let jumped = ahead(Vec3::new(5000.0, 0.0, 0.0), Vec3::ZERO, 0.016);
        assert!((jumped.x - (5000.0 + MOST_LEAD)).abs() < 0.01 && jumped.y == 0.0);
    }
}
