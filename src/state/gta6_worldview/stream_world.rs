//! The world's own thread: it streams the maps around the camera and selects what draws whenever
//! something it was told changes, so that a frame only reads the last selection.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use glam::Vec3;
use or_world::streaming::{Level, View, FADE};

use super::{hour_of, Enabled, ModelRef, Selected, Streamer, WorldIndex, NOON};

/// How far the camera moves before what draws is selected again, in metres.
const RESELECT: f32 = 0.5;
/// How long the world's thread waits for a change before it looks for map bodies that arrived.
const TICK: std::time::Duration = std::time::Duration::from_millis(20);
/// The shortest time between two passes a speed is measured over, in seconds.
const SPEED_STEP: f32 = 0.001;
/// How long a camera told no new place keeps its speed: the thread passes more often than frames come.
// ponytail: a second, longer than a frame of the shots run under load; tell the thread each frame's own velocity if a
// camera that stops must count as still sooner.
const STILL: std::time::Duration = std::time::Duration::from_secs(1);

/// The camera's velocity at `now` where it is at `eye`, given where and when it last moved (`moved`)
/// and the velocity it had then.
// ⚠ A pass between two frames finds the camera where it was: measured there, a camera in flight
// would read as still, and a map that arrives then would fade in as if it were.
pub(super) fn velocity_at(
    moved: &mut Option<(Vec3, Instant)>,
    velocity: Vec3,
    eye: Vec3,
    now: Instant,
) -> Vec3 {
    match *moved {
        None => {
            *moved = Some((eye, now));
            Vec3::ZERO
        }
        Some((at, then)) if at != eye => {
            let step = now.duration_since(then).as_secs_f32();
            if step < SPEED_STEP {
                return velocity;
            }
            *moved = Some((eye, now));
            (eye - at) / step
        }
        // A camera at rest that moves again is measured from this pass, not from when it stopped.
        Some((at, then)) if now.duration_since(then) >= STILL => {
            *moved = Some((at, now));
            Vec3::ZERO
        }
        Some(_) => velocity,
    }
}

/// What the screen tells the world's thread.
#[derive(Default)]
struct Told {
    eye: Vec3,
    scale: f32,
    hour: f32,
    /// The band past its distance each level fades out over, by `Level as usize`, in metres.
    fade: [f32; Level::COUNT],
    /// What of the game to switch on, where that was told since the thread last took it.
    enabled: Option<Enabled>,
    /// Models that came to or left the device since the last selection, with whether each is
    /// there now and `None` for one that will never be.
    models: Vec<(ModelRef, Option<bool>)>,
    /// Whether anything was told since the thread last took what was.
    changed: bool,
    /// How many times something was told.
    times: u64,
    stopped: bool,
}

/// What a selection waits for, handed out before the selection is whole.
#[derive(Default)]
pub(in super::super) struct Asked {
    pub version: u64,
    pub wanted: Vec<(f32, ModelRef)>,
    pub bands: HashMap<ModelRef, f32>,
}

pub(super) struct Shared {
    told: Mutex<Told>,
    woken: Condvar,
    pub(super) latest: Mutex<Arc<Selected>>,
    /// What the selection under way waits for, as [`Selected::wanted`] and [`Selected::bands`], by the version it will have.
    asked: Mutex<Arc<Asked>>,
    /// The maps the thread took in since its last whole selection.
    unproven: Arc<Mutex<Vec<u32>>>,
    /// Up to how many tellings the thread has taken and, where they called for one, selected after.
    done: std::sync::atomic::AtomicU64,
}

/// The world as a thread of its own streams and selects it, so that a frame only reads what was
/// selected last and never waits for the selection.
pub(in super::super) struct World {
    pub(super) shared: Arc<Shared>,
    thread: std::thread::JoinHandle<()>,
}

impl World {
    /// The world of `index` about a camera at `eye`, its distances `scale`d, that never takes in the maps `skipped`.
    pub(in super::super) fn new(
        index: Arc<WorldIndex>,
        eye: Vec3,
        scale: f32,
        skipped: HashSet<u32>,
    ) -> Self {
        let shared = Arc::new(Shared {
            told: Mutex::new(Told {
                eye,
                scale,
                hour: NOON,
                fade: [FADE; Level::COUNT],
                ..Told::default()
            }),
            woken: Condvar::new(),
            latest: Mutex::default(),
            asked: Mutex::default(),
            unproven: Arc::default(),
            done: Default::default(),
        });
        let theirs = Arc::clone(&shared);
        let thread = std::thread::spawn(move || run(index, skipped, &theirs));
        Self { shared, thread }
    }

    /// The maps the thread took in since its last whole selection: for a thread that died, what it is taken to have died on.
    pub(in super::super) fn unproven(&self) -> Vec<u32> {
        (self
            .shared
            .unproven
            .lock()
            .unwrap_or_else(|p| p.into_inner()))
        .clone()
    }

    /// Say the thread took `map` in, as the thread does before it takes one.
    #[cfg(test)]
    pub(in super::super) fn took(&self, map: u32) {
        (self
            .shared
            .unproven
            .lock()
            .unwrap_or_else(|p| p.into_inner()))
        .push(map);
    }

    /// Whether the thread ended though it was not stopped: a panic in a build that unwinds.
    pub(in super::super) fn died(&self) -> bool {
        self.thread.is_finished()
    }

    /// End the thread as a panic on it would.
    #[cfg(test)]
    pub(in super::super) fn end(&self) {
        (self.shared.told.lock().unwrap_or_else(|p| p.into_inner())).stopped = true;
        self.shared.woken.notify_one();
    }

    /// Tell the thread what `tell` changes, and wake it where that is a change.
    fn tell_of(&self, tell: impl FnOnce(&mut Told) -> bool) {
        let mut told = self.shared.told.lock().unwrap_or_else(|p| p.into_inner());
        let changed = tell(&mut told);
        told.changed |= changed;
        told.times += u64::from(changed);
        drop(told);
        if changed {
            self.shared.woken.notify_one();
        }
    }

    /// Say where the camera is and the scale of its distances.
    pub(in super::super) fn look(&self, eye: Vec3, scale: f32) {
        self.tell_of(|told| {
            let changed = (told.eye, told.scale) != (eye, scale);
            (told.eye, told.scale) = (eye, scale);
            changed
        });
    }

    /// Say the hour of the day, 0.0 to 24.0: a time archetype draws inside its hours alone.
    pub(in super::super) fn set_hour(&self, hour: f32) {
        self.tell_of(|told| {
            let changed = hour_of(told.hour) != hour_of(hour);
            told.hour = hour;
            changed
        });
    }

    /// Say the band past its distance each level fades out over, by `Level as usize`, in metres:
    /// [`fade_of`] the install's visual settings.
    pub(in super::super) fn set_fade(&self, fade: [f32; Level::COUNT]) {
        self.tell_of(|told| {
            let changed = told.fade != fade;
            told.fade = fade;
            changed
        });
    }

    /// Say what of the game is switched on: a map it does not allow is not streamed in.
    pub(in super::super) fn set_enabled(&self, enabled: Enabled) {
        self.tell_of(|told| {
            told.enabled = Some(enabled);
            true
        });
    }

    /// Say which models came to the device, will never come, or left it.
    pub(in super::super) fn tell(&self, models: &mut Vec<(ModelRef, Option<bool>)>) {
        if models.is_empty() {
            return;
        }
        self.tell_of(|told| {
            told.models.append(models);
            true
        });
    }

    /// Whether something was told the thread that the last selection does not stand on yet.
    pub(in super::super) fn unselected(&self) -> bool {
        let times = (self.shared.told.lock().unwrap_or_else(|p| p.into_inner())).times;
        self.shared.done.load(std::sync::atomic::Ordering::Acquire) != times
    }

    /// What was selected last.
    pub(in super::super) fn latest(&self) -> Arc<Selected> {
        Arc::clone(&self.shared.latest.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// What the selection under way, or the last, waits for.
    pub(in super::super) fn asked(&self) -> Arc<Asked> {
        Arc::clone(&self.shared.asked.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl Drop for World {
    fn drop(&mut self) {
        (self.shared.told.lock().unwrap_or_else(|p| p.into_inner())).stopped = true;
        self.shared.woken.notify_one();
    }
}

/// The world's thread: stream the maps around the camera, and select again whenever the camera
/// moved, a map came or went, a model did, the hour, the fade bands or what is switched on
/// changed, or a fade in time is under way.
fn run(index: Arc<WorldIndex>, skipped: HashSet<u32>, shared: &Shared) {
    let mut streamer = Streamer::skipping(index, skipped, Arc::clone(&shared.unproven));
    let mut ready: HashMap<ModelRef, Option<bool>> = HashMap::new();
    let mut selected_at: Option<(Vec3, f32, [f32; Level::COUNT])> = None;
    let mut moved_at: Option<(Vec3, Instant)> = None;
    let (mut velocity, mut fading, mut version) = (Vec3::ZERO, false, 0);
    loop {
        let (eye, scale, hour, fade, enabled, models, times) = {
            let mut told = shared.told.lock().unwrap_or_else(|p| p.into_inner());
            // What was told while the last selection was made is taken at once.
            if !told.changed && !told.stopped {
                (told, _) =
                    (shared.woken.wait_timeout(told, TICK)).unwrap_or_else(|p| p.into_inner());
            }
            if told.stopped {
                return;
            }
            told.changed = false;
            let (enabled, models) = (told.enabled.take(), std::mem::take(&mut told.models));
            (
                told.eye, told.scale, told.hour, told.fade, enabled, models, told.times,
            )
        };
        let began = Instant::now();
        velocity = velocity_at(&mut moved_at, velocity, eye, began);
        let mut stale = !models.is_empty() || fading;
        for (model, state) in models {
            match state {
                // A model that left the device is waited for again where it is wanted.
                Some(false) => ready.remove(&model),
                state => ready.insert(model, state),
            };
        }
        stale |= enabled.is_some_and(|enabled| streamer.set_enabled(enabled));
        stale |= streamer.set_hour(hour);
        let mut view = View::at(eye, scale);
        view.velocity = velocity;
        view.fade = fade;
        stale |= streamer.stream(&view);
        stale |= selected_at.is_none_or(|(at, by, faded)| {
            at.distance(eye) >= RESELECT || by != scale || faded != fade
        });
        if stale {
            let state = |model: ModelRef| ready.get(&model).copied().unwrap_or(Some(false));
            version += 1;
            let mut ask = |wanted: &HashMap<ModelRef, f32>, bands: &HashMap<ModelRef, f32>| {
                let asked = Asked {
                    version,
                    wanted: wanted.iter().map(|(model, away)| (*away, *model)).collect(),
                    bands: bands.clone(),
                };
                *shared.asked.lock().unwrap_or_else(|p| p.into_inner()) = Arc::new(asked);
            };
            let mut selected = streamer.select_asking(&view, &state, &mut ask);
            (selected.version, selected.took) = (version, began.elapsed());
            fading = selected.fading;
            selected_at = Some((eye, scale, fade));
            *shared.latest.lock().unwrap_or_else(|p| p.into_inner()) = Arc::new(selected);
            // What was taken in has been through a whole selection.
            (shared.unproven.lock().unwrap_or_else(|p| p.into_inner())).clear();
        }
        shared
            .done
            .store(times, std::sync::atomic::Ordering::Release);
    }
}
