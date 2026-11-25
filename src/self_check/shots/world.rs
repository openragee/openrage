//! The `world` steps of a shots script, which make a frame of the GTA VI world one that draws the
//! same again: a camera, an hour, a clean frame, a wait for what streams in, and a dump of what
//! the viewer wants and draws for a check against what the game loads.
//!
//! ⚠ The viewer is locked for a statement at a time and never across a frame, which locks it too.

use std::path::Path;
use std::time::{Duration, Instant};

use super::script::{file_stem, numbers, split_word};
use super::Shell;
use crate::state::drivers::WeatherState;
use crate::state::gta6_world::worldview::viewer;
use crate::state::AppState;
use crate::ui::dock::Workspace;
use crate::ui::panels::world_viewer::{self, Capture, Pose};

/// How long nothing may arrive or change before the world counts as settled.
const QUIET: Duration = Duration::from_millis(1500);
/// How long `world settle` waits where its line gives no number of seconds.
const SETTLE_DEADLINE: Duration = Duration::from_secs(180);
/// How often a wait says what it still waits for.
const TOLD_EVERY: Duration = Duration::from_secs(10);
/// The longest deadline a line may give, in seconds.
const MOST_SECONDS: f32 = 86_400.0;
/// How long `world camera` waits for the install's index: a first build reads every archive of the level.
const INDEX_DEADLINE: Duration = Duration::from_secs(1800);
/// The levels of detail in the order a map numbers them, named as the truth model names them.
const LEVELS: [&str; 7] = ["HD", "LOD", "SLOD1", "SLOD2", "SLOD3", "ORPHANHD", "SLOD4"];
/// The kinds of node a selection's census counts, in its order.
const KINDS: [&str; 4] = ["entity", "prop list", "grass list", "far grass list"];

/// A step that acts on the World screen.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum WorldStep {
    /// The camera put where a camera line says.
    Camera(Pose),
    /// The hour of the day the world is drawn at, from 0 up to 24.
    Hour(f32),
    /// The view drawn alone over the whole window, or back in the dock.
    Clean(bool),
    /// Frames drawn until nothing arrives any more, for this long at most.
    Settle(Duration),
    /// What the viewer wants and draws at its camera, written as JSON to this file beside the shots.
    Dump(String),
    /// The game modes that stream: bit 0 the base game, bit 1 story, bit 2 the shared minigames.
    Modes(u8),
    /// The camera moved at this velocity, in metres a second, for this long, a frame at a time.
    Fly(glam::Vec3, Duration),
    /// What this many frames cost at rest and laid out again, written as JSON to this file beside the shots.
    Perf(String, usize),
    /// The camera moved to a pose over a time, and what its frames cost written as JSON to this file.
    Move(Pose, Duration, String),
    /// A script-managed group, by its id, switched on or off as the person ticks it; the hour then no longer sets the groups.
    Group(u32, bool),
    /// The weather the world is drawn in, as the Weather bar sets it.
    Weather(WeatherState),
}

/// The words `world weather` takes, each with its state.
const WEATHERS: [(&str, WeatherState); 5] = [
    ("clear", WeatherState::Clear),
    ("cloudy", WeatherState::Cloudy),
    ("rain", WeatherState::Rain),
    ("storm", WeatherState::Storm),
    ("fog", WeatherState::Fog),
];

/// The words `world modes` takes, each with its bit.
const MODES: [(&str, u8); 3] = [("base", 1), ("story", 2), ("minigames", 4)];

/// The step the words after `world` name.
pub(super) fn parse(rest: &str) -> Result<WorldStep, String> {
    let (word, rest) = split_word(rest);
    let args: Vec<&str> = rest.split_whitespace().collect();
    match word {
        "camera" => Pose::read(rest).map(WorldStep::Camera),
        "hour" => {
            let [hour] = numbers::<1>(&args)?;
            if !(0.0..24.0).contains(&hour) {
                return Err("an hour is from 0 up to 24, and never 24".into());
            }
            Ok(WorldStep::Hour(hour))
        }
        "clean" => match rest {
            "on" => Ok(WorldStep::Clean(true)),
            "off" => Ok(WorldStep::Clean(false)),
            other => Err(format!("no clean `{other}`; use on or off")),
        },
        "settle" if args.is_empty() => Ok(WorldStep::Settle(SETTLE_DEADLINE)),
        "settle" => {
            let [seconds] = numbers::<1>(&args)?;
            if seconds <= 0.0 || seconds > MOST_SECONDS {
                return Err(format!(
                    "a deadline is a number of seconds above 0, {MOST_SECONDS} at most"
                ));
            }
            Ok(WorldStep::Settle(Duration::from_secs_f32(seconds)))
        }
        // ⚠ A name, not a path: a dump lands beside the shots, never in the tree.
        "dump" => file_stem(rest).map(WorldStep::Dump),
        "modes" if !args.is_empty() => (args.iter())
            .try_fold(0, |bits, word| {
                let (_, bit) = (MODES.iter().find(|(name, _)| name == word))
                    .ok_or(format!("no mode `{word}`; use base, story or minigames"))?;
                Ok(bits | bit)
            })
            .map(WorldStep::Modes),
        "fly" => {
            let [x, y, z, seconds] = numbers::<4>(&args)?;
            if seconds <= 0.0 || seconds > MOST_SECONDS {
                return Err(format!(
                    "a flight lasts a number of seconds above 0, {MOST_SECONDS} at most"
                ));
            }
            Ok(WorldStep::Fly(
                glam::Vec3::new(x, y, z),
                Duration::from_secs_f32(seconds),
            ))
        }
        "perf" => {
            let [name, frames] = args[..] else {
                return Err("needs a file's name and a number of frames".into());
            };
            let frames = (frames.parse::<usize>().ok())
                .filter(|n| (1..=10_000).contains(n))
                .ok_or_else(|| format!("`{frames}` is not a number of frames from 1 to 10000"))?;
            Ok(WorldStep::Perf(file_stem(name)?, frames))
        }
        "move" => {
            let (seconds, rest) = split_word(rest);
            let (name, rest) = split_word(rest);
            let [seconds] = numbers::<1>(&[seconds])?;
            if seconds <= 0.0 || seconds > MOST_SECONDS {
                return Err(format!(
                    "a move takes a number of seconds above 0, {MOST_SECONDS} at most"
                ));
            }
            let to = Pose::read(rest)?;
            Ok(WorldStep::Move(
                to,
                Duration::from_secs_f32(seconds),
                file_stem(name)?,
            ))
        }
        "group" => {
            let [id, on] = args[..] else {
                return Err("needs a group's id in hex and on or off".into());
            };
            let id = u32::from_str_radix(id.trim_start_matches("0x"), 16)
                .map_err(|_| format!("`{id}` is not a group's id in hex"))?;
            match on {
                "on" => Ok(WorldStep::Group(id, true)),
                "off" => Ok(WorldStep::Group(id, false)),
                other => Err(format!("no group state `{other}`; use on or off")),
            }
        }
        "weather" => (WEATHERS.iter().find(|(name, _)| *name == rest))
            .map(|(_, state)| WorldStep::Weather(*state))
            .ok_or(format!(
                "no weather `{rest}`; use clear, cloudy, rain, storm or fog"
            )),
        other => Err(format!(
            "no world step `{other}`; use camera, hour, clean, settle, dump, modes, fly, perf, move, group or weather"
        )),
    }
}

/// What one frame finds of the world viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Found {
    /// An index of the install is held.
    pub indexed: bool,
    /// The maps, models, textures and maps' lights asked for and not there yet.
    pub waiting: [usize; 4],
    /// What stands streamed in: a change in it is something that came or went.
    pub standing: [usize; 4],
    /// The frame's exposure is still on its way to what its meter wants.
    pub adapting: bool,
    /// A part of the water that was asked for is still being read.
    pub water: bool,
    /// The world's thread was told something, such as models that came, that its last selection does not stand on yet.
    pub selecting: bool,
}

/// The rule `world settle` waits by: an index held, nothing waited for, the exposure arrived, no water being read, and nothing changed for [`QUIET`].
// ⚠ Nothing waited for is not enough: for a frame or two after the camera jumps, the world's
// thread has not yet asked for the maps of the new place.
#[derive(Debug, Default)]
pub(super) struct Settle {
    last: Option<Found>,
    since: Duration,
    /// When each of the four waits was last above nought, and when the exposure last adapted.
    waited: [Option<Duration>; 5],
}

impl Settle {
    /// Whether the world has settled at `now`, a time on a clock that only runs forward, given what this frame found.
    pub(super) fn settled(&mut self, now: Duration, found: Found) -> bool {
        for (waited, more) in self.waited.iter_mut().zip(found.waiting.map(|n| n > 0)) {
            if more {
                *waited = Some(now);
            }
        }
        if found.adapting {
            self.waited[4] = Some(now);
        }
        // ⚠ A selection can take longer than the quiet time: models told ready and not yet selected were dumped as not drawn.
        let quiet = found.indexed
            && found.waiting == [0; 4]
            && !found.adapting
            && !found.water
            && !found.selecting;
        if !quiet || self.last != Some(found) {
            (self.last, self.since) = (Some(found), now);
            return false;
        }
        now.saturating_sub(self.since) >= QUIET
    }

    /// When the maps, models, textures, maps' lights and exposure were last waited for, in seconds.
    pub(super) fn last_waits(&self) -> String {
        let names = ["maps", "models", "textures", "lights", "exposure"];
        let at = |waited: &Option<Duration>| waited.map_or(0.0, |t| t.as_secs_f32());
        (names.iter().zip(&self.waited))
            .map(|(name, waited)| format!("{name} {:.1} s", at(waited)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// What was still arriving when the wait gave up, for the line that fails the run.
    pub(super) fn still(&self) -> String {
        match self.last {
            None => "no frame was drawn".into(),
            Some(found) if !found.indexed => "no index of the install is held".into(),
            Some(found) if found.waiting == [0; 4] && found.water => {
                "the water's ground or fog was still being read".into()
            }
            Some(found) if found.waiting == [0; 4] && found.selecting => {
                "the world's thread had not yet selected on what it was told".into()
            }
            Some(Found {
                waiting: [0, 0, 0, 0],
                adapting: true,
                ..
            }) => "the exposure was still adapting".into(),
            Some(Found {
                waiting: [0, 0, 0, 0], ..
            }) => "what is streamed in was still changing".into(),
            Some(Found {
                waiting: [maps, models, textures, lights],
                ..
            }) => format!(
                "{maps} maps, {models} models, {textures} textures and {lights} maps' lights were still waited for"
            ),
        }
    }
}

/// What the viewer holds and waits for now, or why it has no index.
fn found() -> Result<Found, String> {
    let viewer = viewer();
    if let Some(why) = &viewer.failed {
        return Err(format!("the GTA VI world was not indexed: {why}"));
    }
    let streaming = viewer.streaming();
    Ok(Found {
        indexed: viewer.index.is_some(),
        waiting: [
            streaming.maps_waiting,
            viewer.models_waiting,
            viewer.textures_waiting,
            viewer.lights.as_ref().map_or(0, |lights| lights.waiting()),
        ],
        standing: [
            streaming.maps_wanted,
            streaming.maps_loaded,
            streaming.nodes,
            streaming.drawn,
        ],
        adapting: crate::state::gta6_world::worldview::sky::adapting(),
        water: viewer.water_reading(),
        selecting: viewer.selecting(),
    })
}

/// A census's counts as `"KIND LEVEL": count`, leaving out what it counts none of.
fn by_level(counts: &[[usize; 7]; 4]) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for (kind, row) in KINDS.iter().zip(counts) {
        for (level, n) in LEVELS.iter().zip(row) {
            if *n > 0 {
                out.insert(format!("{kind} {level}"), (*n).into());
            }
        }
    }
    out
}

/// 32-bit names as eight hexadecimal digits, sorted.
fn hex(names: &[u32]) -> Vec<String> {
    let mut out: Vec<String> = names.iter().map(|n| format!("{n:08X}")).collect();
    out.sort_unstable();
    out
}

/// Write what the viewer wants and draws at its camera to `path` as JSON.
fn dump(path: &Path) -> Result<(), String> {
    let mut viewer = viewer();
    let selected = (viewer.selected(&mut Vec::new()))
        .ok_or("no world streams yet: put `world settle` before `world dump`")?;
    let pose = Pose::of(&viewer.camera);
    let (census, s) = (&selected.census, selected.streaming);
    let records: usize = selected.cards.iter().map(|list| list.records.len()).sum();
    // ⚠ A node's fade is written as `MAP:INDEX:FADE`, so a fade in time can be judged against the distance's.
    let mut drawn_nodes: Vec<String> = (census.drawn_nodes.iter())
        .map(|(map, at, fade)| format!("{map:08X}:{at}:{fade:.3}"))
        .collect();
    drawn_nodes.sort_unstable();
    let json = serde_json::json!({
        "camera": {
            "pos": [pose.position.x, pose.position.y, pose.position.z],
            "yaw": pose.yaw,
            "pitch": pose.pitch,
            "fov": pose.fov,
        },
        // ⚠ The world's thread selects a frame or more behind the camera: in flight this is not `camera`.
        "selected_eye": [selected.eye.x, selected.eye.y, selected.eye.z],
        "selection_ms": selected.took.as_secs_f64() * 1000.0,
        "hour": viewer.hour,
        "ground": viewer.ground_under(pose.position.x, pose.position.y),
        "lod_scale": viewer.lod_scale,
        "modes": viewer.enabled.modes,
        "groups_on": hex(&viewer.enabled.groups),
        "streaming": {
            "maps_wanted": s.maps_wanted,
            "maps_loaded": s.maps_loaded,
            "maps_waiting": s.maps_waiting,
            "nodes": s.nodes,
            "drawn": s.drawn,
        },
        "wanted_map_names": hex(&census.wanted),
        "loaded_map_names": hex(&census.loaded),
        "nodes_by_level": by_level(&census.nodes),
        "drawn_by_level": by_level(&census.drawn),
        "list_instances": census.list_instances,
        "drawn_nodes": drawn_nodes,
        "cards": { "lists": selected.cards.len(), "records": records },
    });
    drop(viewer);
    let text = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    println!("dump  {}", path.display());
    Ok(())
}

impl Shell {
    /// Run a `world` step on the World screen, which the script has put up before it; a dump goes into `out`.
    pub(super) fn world(
        &mut self,
        state: &mut AppState,
        step: &WorldStep,
        out: &Path,
    ) -> Result<(), String> {
        // Every step ends on drawn frames, so with the World screen up its viewer has opened the install.
        if state.active_workspace != Workspace::World || !world_viewer::offered(state) {
            return Err(
                "the World screen is not drawn: put `game gta6 FOLDER` and `workspace World` first"
                    .into(),
            );
        }
        match step {
            WorldStep::Camera(pose) => {
                // ⚠ The viewer puts its camera over the middle of the maps as it takes the index, so a camera is set after that.
                self.indexed(state)?;
                let mut viewer = viewer();
                pose.put(&mut viewer.camera);
                viewer.cuts += 1;
            }
            WorldStep::Hour(hour) => {
                // What is placed for some hours alone follows the viewer's hour and the light follows the day's: a frame has one.
                viewer().hour = *hour;
                let day = &mut state.drivers.time_of_day;
                day.set_hour(*hour);
                // ⚠ A day that runs, or days gone by, which the moon and the clouds follow, would draw no two runs alike.
                (day.playing, day.days) = (false, 0.0);
            }
            WorldStep::Clean(on) => state.panels.get::<Capture>().clean = *on,
            // A wait that runs out fails the run at its end, as a comparison does, so that every camera after it is still judged.
            WorldStep::Settle(deadline) => {
                if let Err(why) = self.settle(state, *deadline) {
                    println!("FAILED  {why}");
                    self.failed.push(why);
                }
            }
            WorldStep::Dump(name) => dump(&out.join(name))?,
            // The groups follow the modes, as they do on the screen until a group is ticked.
            WorldStep::Modes(modes) => viewer().enabled.modes = *modes,
            WorldStep::Fly(velocity, seconds) => self.fly(state, *velocity, *seconds)?,
            WorldStep::Perf(name, frames) => self.perf(state, *frames, &out.join(name))?,
            WorldStep::Move(to, over, name) => self.walk(state, to, *over, &out.join(name))?,
            WorldStep::Group(id, on) => {
                let mut viewer = viewer();
                viewer.groups_chosen = true;
                viewer.enabled.groups.retain(|g| g != id);
                if *on {
                    viewer.enabled.groups.push(*id);
                    viewer.enabled.groups.sort_unstable();
                }
            }
            WorldStep::Weather(weather) => {
                state.set_weather_state(*weather);
            }
        }
        Ok(())
    }

    /// Draw frames for `seconds`, the camera moved at `velocity` by the time each is drawn.
    fn fly(
        &mut self,
        state: &mut AppState,
        velocity: glam::Vec3,
        seconds: Duration,
    ) -> Result<(), String> {
        let (started, from) = (Instant::now(), viewer().camera.position);
        // ⚠ The last frame is not cut short at `seconds`: a short last step would read as a camera
        // slowing to a stop, and a dump after the flight is to see it still flying.
        loop {
            let flown = started.elapsed();
            viewer().camera.position = from + velocity * flown.as_secs_f32();
            self.idle(state, 1)?;
            if flown >= seconds {
                return Ok(());
            }
        }
    }

    /// Draw frames until the install's index is held.
    fn indexed(&mut self, state: &mut AppState) -> Result<(), String> {
        let until = Instant::now() + INDEX_DEADLINE;
        while !found()?.indexed {
            if Instant::now() > until {
                return Err(format!(
                    "the GTA VI world was still being indexed after {} minutes",
                    INDEX_DEADLINE.as_secs() / 60
                ));
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(30));
        }
        Ok(())
    }

    /// Draw frames until the world has settled, or fail at `deadline` by what was still arriving.
    fn settle(&mut self, state: &mut AppState, deadline: Duration) -> Result<(), String> {
        let started = Instant::now();
        let mut rule = Settle::default();
        let (mut told, mut frames) = (Duration::ZERO, 0u32);
        loop {
            self.idle(state, 1)?;
            frames += 1;
            let now = started.elapsed();
            let mut found = found()?;
            // ⚠ The plain pass meters no frame, so its exposure never arrives: only a frame the game lights adapts.
            found.adapting &= self.bridge.world_drawn().lit_by_the_game;
            if rule.settled(now, found) {
                println!(
                    "  settled after {:.1} s; last waited for: {}",
                    now.as_secs_f32(),
                    rule.last_waits()
                );
                return Ok(());
            }
            if now > told + TOLD_EVERY {
                told = now;
                let (drawn, (cost, _)) = (self.bridge.world_drawn(), self.bridge.world_cost());
                let [maps, models, textures, lights] = found.waiting;
                println!(
                    "  at {:.0} s, {frames} frames since, waiting for {maps} maps, {models} models, {textures} textures, {lights} lights; {} models and {} textures on the device, {} unread; let go {:?}, read again {:?}",
                    now.as_secs_f32(),
                    drawn.models,
                    drawn.textures,
                    drawn.textures_unread,
                    cost.evicted,
                    cost.reread,
                );
                frames = 0;
            }
            if now > deadline {
                return Err(format!(
                    "the world had not settled after {:.1} s: {}; last waited for: {}",
                    deadline.as_secs_f32(),
                    rule.still(),
                    rule.last_waits()
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE: Found = Found {
        indexed: true,
        waiting: [0; 4],
        standing: [40, 40, 900, 700],
        adapting: false,
        water: false,
        selecting: false,
    };

    fn at(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn the_steps_of_the_world_are_read() {
        assert_eq!(
            parse("camera -120.5 340 62.25 135 -20 60"),
            Ok(WorldStep::Camera(Pose {
                position: glam::Vec3::new(-120.5, 340.0, 62.25),
                yaw: 135.0,
                pitch: -20.0,
                fov: Some(60.0),
            }))
        );
        assert_eq!(parse("hour 17.5"), Ok(WorldStep::Hour(17.5)));
        assert_eq!(parse("hour 0"), Ok(WorldStep::Hour(0.0)));
        assert_eq!(parse("clean on"), Ok(WorldStep::Clean(true)));
        assert_eq!(parse("clean off"), Ok(WorldStep::Clean(false)));
        assert_eq!(parse("settle"), Ok(WorldStep::Settle(SETTLE_DEADLINE)));
        assert_eq!(parse("settle 2.5"), Ok(WorldStep::Settle(at(2500))));
        assert_eq!(
            parse("dump truth-01.json"),
            Ok(WorldStep::Dump("truth-01.json".into()))
        );
        assert_eq!(parse("modes base story"), Ok(WorldStep::Modes(3)));
        assert_eq!(parse("modes minigames"), Ok(WorldStep::Modes(4)));
        assert_eq!(
            parse("fly 30 0 -1.5 2"),
            Ok(WorldStep::Fly(glam::Vec3::new(30.0, 0.0, -1.5), at(2000)))
        );
        assert_eq!(
            parse("perf cost-01.json 30"),
            Ok(WorldStep::Perf("cost-01.json".into(), 30))
        );
        assert_eq!(
            parse("move 1 walk.json 50 0 10 0 -5 60"),
            Ok(WorldStep::Move(
                Pose {
                    position: glam::Vec3::new(50.0, 0.0, 10.0),
                    yaw: 0.0,
                    pitch: -5.0,
                    fov: Some(60.0),
                },
                at(1000),
                "walk.json".into()
            ))
        );
        assert_eq!(
            parse("group 0D93912D on"),
            Ok(WorldStep::Group(0x0D93_912D, true))
        );
        assert_eq!(parse("group 0x1 off"), Ok(WorldStep::Group(1, false)));
        assert_eq!(
            parse("weather cloudy"),
            Ok(WorldStep::Weather(WeatherState::Cloudy))
        );
    }

    #[test]
    fn a_census_is_written_by_kind_and_level_and_names_as_sorted_hexadecimal() {
        let mut counts = [[0; 7]; 4];
        (counts[0][0], counts[2][5], counts[3][2]) = (12, 3, 1);
        let written = by_level(&counts);
        assert_eq!(written.len(), 3, "a count of none is left out: {written:?}");
        assert_eq!(written["entity HD"], 12);
        assert_eq!(written["grass list ORPHANHD"], 3);
        assert_eq!(written["far grass list SLOD1"], 1);
        assert_eq!(
            hex(&[0xABC, 0x1, 0xFFFF_FFFF]),
            ["00000001", "00000ABC", "FFFFFFFF"]
        );
    }

    #[test]
    fn a_wrong_world_step_is_refused() {
        for bad in [
            "",
            "fly",
            "camera",
            "camera 1 2 3",
            "camera 1 2 3 0 95",
            "hour",
            "hour 24",
            "hour -1",
            "hour noon",
            "clean",
            "clean yes",
            "settle 0",
            "settle -3",
            "settle soon",
            "settle 1 2",
            "settle 1e30",
            "dump",
            "dump ../out.json",
            "dump C:/out.json",
            "dump two words",
            "modes",
            "modes base online",
            "weather",
            "weather sunny",
            "group",
            "group 0D93912D",
            "group zz on",
            "group 1 maybe",
            "fly 1 2 3",
            "fly 1 2 3 0",
            "fly 1 2 3 soon",
            "perf",
            "perf a.json",
            "perf a.json 0",
            "perf ../a.json 3",
            "move",
            "move 1 a.json",
            "move 0 a.json 1 2 3 0 0",
            "move 1 ../a.json 1 2 3 0 0",
        ] {
            assert!(parse(bad).is_err(), "`world {bad}` should be refused");
        }
    }

    #[test]
    fn the_world_settles_once_nothing_is_waited_for_and_nothing_has_changed_for_the_quiet_time() {
        let mut rule = Settle::default();
        assert!(!rule.settled(at(0), IDLE), "one frame is no quiet time");
        assert!(!rule.settled(QUIET - at(1), IDLE));
        assert!(rule.settled(QUIET, IDLE));
    }

    #[test]
    fn something_waited_for_or_changed_starts_the_quiet_time_again() {
        let waiting = Found {
            waiting: [0, 3, 0, 0],
            ..IDLE
        };
        let grown = Found {
            standing: [40, 40, 900, 701],
            ..IDLE
        };
        let mut rule = Settle::default();
        assert!(!rule.settled(at(0), IDLE));
        // A model asked for after a quiet stretch: the camera jumped and the selection caught up.
        assert!(!rule.settled(QUIET - at(1), waiting));
        assert!(
            !rule.settled(QUIET * 3, waiting),
            "never while something is waited for"
        );
        assert!(!rule.settled(QUIET * 3 + at(1), IDLE));
        assert_eq!(
            rule.last_waits(),
            "maps 0.0 s, models 4.5 s, textures 0.0 s, lights 0.0 s, exposure 0.0 s"
        );
        assert!(
            !rule.settled(QUIET * 4, IDLE),
            "the quiet time runs from the last change, not from the first frame"
        );
        // Nothing is waited for, yet what stands streamed in moved: a parent gave way to its children.
        assert!(!rule.settled(QUIET * 4 + at(1), grown));
        assert!(!rule.settled(QUIET * 5, grown));
        assert!(rule.settled(QUIET * 5 + at(1), grown));
    }

    #[test]
    fn a_world_with_no_index_never_settles_and_the_wait_says_what_was_still_arriving() {
        let unindexed = Found {
            indexed: false,
            ..Found::default()
        };
        let mut rule = Settle::default();
        assert_eq!(rule.still(), "no frame was drawn");
        assert!(!rule.settled(at(0), unindexed));
        assert!(
            !rule.settled(QUIET * 10, unindexed),
            "no index is nothing to shoot"
        );
        assert_eq!(rule.still(), "no index of the install is held");
        let waiting = Found {
            waiting: [2, 0, 7, 0],
            ..IDLE
        };
        assert!(!rule.settled(QUIET * 11, waiting));
        assert_eq!(
            rule.still(),
            "2 maps, 0 models, 7 textures and 0 maps' lights were still waited for"
        );
        let water = Found {
            water: true,
            ..IDLE
        };
        assert!(!rule.settled(QUIET * 12, water));
        assert!(
            !rule.settled(QUIET * 20, water),
            "never while the water is read"
        );
        assert_eq!(
            rule.still(),
            "the water's ground or fog was still being read"
        );
        let selecting = Found {
            selecting: true,
            ..IDLE
        };
        assert!(!rule.settled(QUIET * 21, selecting));
        assert!(
            !rule.settled(QUIET * 30, selecting),
            "never while the world's thread has not selected on what it was told"
        );
        assert_eq!(
            rule.still(),
            "the world's thread had not yet selected on what it was told"
        );
        assert!(!rule.settled(QUIET * 31, IDLE));
        assert_eq!(rule.still(), "what is streamed in was still changing");
    }
}
