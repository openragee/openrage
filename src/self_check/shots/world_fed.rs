//! The `world` steps over a world drawn through the seam: a camera, an hour, a clean frame, a wait
//! for what streams in, and a dump of what the session waits for, could not read and drew.
//!
//! ⚠ Such a world has no exposure to arrive and no history to forget: a converge is a settle.

use std::path::Path;
use std::time::{Duration, Instant};

use super::world::WorldStep;
use super::Shell;
use crate::state::world_session::{Kind, Standing};
use crate::state::AppState;
use crate::ui::panels::world_viewer::Capture;

/// How many frames in a row nothing may be waited for or change before a fed world counts as settled.
const QUIET_FRAMES: u32 = 24;
/// How long a camera waits for the install's index: a first open reads every archive.
const INDEX_DEADLINE: Duration = Duration::from_secs(2400);
/// How often a wait says what it still waits for.
const TOLD_EVERY: Duration = Duration::from_secs(10);
/// The frames a dump times of a world at rest.
const TIMED_FRAMES: u32 = 30;

impl Shell {
    /// Run a `world` step on the World screen of a world drawn through the seam; a dump goes into `out`.
    pub(super) fn world_fed(
        &mut self,
        state: &mut AppState,
        step: &WorldStep,
        out: &Path,
    ) -> Result<(), String> {
        match step {
            WorldStep::Camera(pose) => {
                self.fed_open(state)?;
                let session = state.world_session().ok_or("no world session is held")?;
                let mut camera = session.camera();
                pose.put(&mut camera);
                session.set_camera(camera);
            }
            WorldStep::Hour(hour) => {
                let session = state.world_session().ok_or("no world session is held")?;
                session.set_hour(*hour);
            }
            WorldStep::Clean(on) => state.panels.get::<Capture>().clean = *on,
            WorldStep::Settle(deadline) | WorldStep::Converge(deadline) => {
                self.fed_open(state)?;
                if let Err(why) = self.fed_settle(state, *deadline) {
                    println!("  {why}");
                    self.failed.push(why);
                }
            }
            WorldStep::Dump(name) => self.fed_dump(state, &out.join(name))?,
            other => {
                return Err(format!(
                    "{other:?} is a step of the GTA VI world alone: this world is drawn through the seam"
                ))
            }
        }
        Ok(())
    }

    /// Draw frames until the session's world is open.
    fn fed_open(&mut self, state: &mut AppState) -> Result<(), String> {
        let until = Instant::now() + INDEX_DEADLINE;
        let mut told = Instant::now();
        loop {
            self.idle(state, 1)?;
            let session = state.world_session().ok_or("no world session is held")?;
            match session.standing() {
                Standing::Open => return Ok(()),
                Standing::Failed(why) => return Err(format!("the world did not open: {why}")),
                Standing::Opening(done) => {
                    if told.elapsed() > TOLD_EVERY {
                        told = Instant::now();
                        println!("  indexing: {:.0} %", done * 100.0);
                    }
                }
            }
            if Instant::now() > until {
                return Err(format!(
                    "the world was still being indexed after {} minutes",
                    INDEX_DEADLINE.as_secs() / 60
                ));
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Draw frames until nothing is waited for and nothing came to the device or changed in the selection for a run of frames, or fail at `deadline`.
    fn fed_settle(&mut self, state: &mut AppState, deadline: Duration) -> Result<(), String> {
        let started = Instant::now();
        let (mut quiet, mut told, mut was) = (0, Duration::ZERO, (0, 0));
        loop {
            self.idle(state, 1)?;
            let session = state.world_session().ok_or("no world session is held")?;
            let waiting = session.waiting();
            let selection = session.feed().and_then(|feed| feed.selection());
            let version = selection.as_ref().map_or(0, |selection| selection.version);
            let fading = selection.is_some_and(|selection| selection.fading);
            let now = (self.bridge.world_feed_changes(), version);
            quiet = match waiting.any() || fading || now != was {
                true => 0,
                false => quiet + 1,
            };
            was = now;
            let elapsed = started.elapsed();
            if quiet >= QUIET_FRAMES {
                println!("  settled after {:.1} s", elapsed.as_secs_f32());
                return Ok(());
            }
            if elapsed > told + TOLD_EVERY {
                told = elapsed;
                let drawn = self.bridge.world_drawn();
                println!(
                    "  at {:.0} s, waiting for {} maps, {} models, {} textures; {} models and {} textures on the device, {} MB",
                    elapsed.as_secs_f32(),
                    waiting.maps,
                    waiting.models,
                    waiting.textures,
                    drawn.models,
                    drawn.textures,
                    drawn.bytes >> 20,
                );
            }
            if elapsed > deadline {
                return Err(format!(
                    "the world had not settled after {:.1} s: {} maps, {} models and {} textures were still waited for",
                    deadline.as_secs_f32(),
                    waiting.maps,
                    waiting.models,
                    waiting.textures
                ));
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    /// Write what the session waits for, could not read and drew, and what a frame of it at rest takes, as JSON.
    fn fed_dump(&mut self, state: &mut AppState, path: &Path) -> Result<(), String> {
        let timed = Instant::now();
        self.idle(state, TIMED_FRAMES as usize)?;
        let frame_ms = timed.elapsed().as_secs_f64() * 1000.0 / f64::from(TIMED_FRAMES);
        // A frame that lays the world out again, as one the camera moved in does.
        self.bridge.relay_world_feed();
        let relaid = Instant::now();
        self.idle(state, 1)?;
        let relaid_ms = relaid.elapsed().as_secs_f64() * 1000.0;
        let drawn = self.bridge.world_drawn();
        let session = state.world_session().ok_or("no world session is held")?;
        let waiting = session.waiting();
        let unread: Vec<serde_json::Value> = (session.unread().iter())
            .map(|unread| {
                serde_json::json!({
                    "kind": match unread.kind {
                        Kind::Map => "map",
                        Kind::Model => "model",
                        Kind::Texture => "texture",
                    },
                    "why": unread.why.shown(),
                    "count": unread.count,
                })
            })
            .collect();
        let json = serde_json::json!({
            "game": session.key().game.label(),
            "frame_ms_at_rest": frame_ms,
            "frame_ms_laid_out_again": relaid_ms,
            "waiting": { "maps": waiting.maps, "models": waiting.models, "textures": waiting.textures },
            "drawn": {
                "selected": drawn.selected,
                "seen": drawn.seen,
                "draws": drawn.draws,
                "models": drawn.models,
                "textures": drawn.textures,
                "textures_unread": drawn.textures_unread,
                "device_bytes": drawn.bytes,
                "pages": drawn.pages,
                "lit_by_the_game": drawn.lit_by_the_game,
            },
            "unread": unread,
        });
        let text = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("dump  {}", path.display());
        Ok(())
    }
}
