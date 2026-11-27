//! The `world perf`, `world move` and `model perf` steps: what frames of the World screen or of
//! a GTA VI model cost on the CPU and the device, at rest and while the camera moves, written as
//! JSON beside the shots.
//!
//! ⚠ The device's stages are read a frame or two late, and only on a device that times them.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::Shell;
use crate::state::gta6_world::worldview::viewer;
use crate::state::AppState;
use crate::ui::panels::world_viewer::Pose;

/// The milliseconds of a duration.
fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

/// The least, the middle, the mean, the 95th hundredth and the largest of `values`.
// ⚠ Other work on the machine only ever adds time: the least is the steadiest of the five.
fn spread(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    values.sort_unstable_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let at = |hundredth: usize| values[(values.len() - 1) * hundredth / 100];
    json!({
        "min": at(0),
        "p50": at(50),
        "mean": mean,
        "p95": at(95),
        "max": at(100),
        "n": values.len(),
    })
}

/// A pose `t` of the way from `from` to `to`.
fn between(from: &Pose, to: &Pose, t: f32) -> Pose {
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    Pose {
        position: from.position.lerp(to.position, t),
        yaw: lerp(from.yaw, to.yaw),
        pitch: lerp(from.pitch, to.pitch),
        fov: to.fov.or(from.fov),
    }
}

impl Shell {
    /// Draw a frame and say what it cost.
    fn measured(&mut self, state: &mut AppState) -> Result<Value, String> {
        let started = Instant::now();
        self.idle(state, 1)?;
        let frame = started.elapsed();
        let (cost, calls) = self.bridge.world_cost();
        let drawn = self.bridge.world_drawn();
        let sets = self.bridge.world_sets();
        let stage = |name: &str| {
            (state.gpu_stages.iter())
                .find(|(s, _)| *s == name)
                .map(|s| s.1)
        };
        let viewer = viewer();
        Ok(json!({
            "frame_ms": ms(frame),
            "refresh_ms": ms(cost.refresh),
            "record_ms": ms(self.submitted.0),
            "submit_ms": ms(self.submitted.1),
            "laid": cost.laid,
            "take_ms": ms(cost.taking),
            "lights_ms": ms(cost.lights),
            "day_ms": ms(viewer.day_took),
            "occlude_ms": ms(cost.occlude),
            "cull_ms": ms(cost.cull),
            "layout_ms": ms(cost.layout),
            "gpu_ms": state.gpu_stages.iter().map(|(_, t)| f64::from(*t)).sum::<f64>(),
            "gpu_gbuffer_ms": stage("g-buffer"),
            "gpu_shadows_ms": stage("shadows"),
            "gpu_lighting_ms": stage("lighting"),
            "gpu_stages": state.gpu_stages.iter().map(|(s, t)| json!([s, t])).collect::<Vec<_>>(),
            "selected": drawn.selected,
            "seen": drawn.seen,
            "laid_instances": cost.laid_instances,
            "shadow_only": cost.shadow_only,
            "chunks": cost.chunks,
            "hidden": cost.hidden,
            "occluders": [cost.occluders.0, cost.occluders.1],
            "draws": calls.draws,
            "calls": calls.calls,
            "cast_calls": calls.cast_calls,
            "triangles": calls.triangles,
            "cast_triangles": calls.cast_triangles,
            "models": drawn.models,
            "textures": drawn.textures,
            "device_bytes": drawn.bytes,
            "pages": drawn.pages,
            "uploaded": [cost.uploaded.0, cost.uploaded.1],
            "evicted": [cost.evicted.0, cost.evicted.1],
            "reread": [cost.reread.0, cost.reread.1],
            "models_waiting": viewer.models_waiting,
            "textures_waiting": viewer.textures_waiting,
            "read_not_taken_bytes": viewer.held.load(std::sync::atomic::Ordering::Relaxed),
            "sets": {
                "models": sets.models,
                "missing": sets.missing,
                "textures": sets.textures,
                "materials": sets.materials,
                "free_materials": sets.free_materials,
                "material_keys": sets.material_keys,
                "palettes": sets.palettes,
                "gone": [sets.gone.0, sets.gone.1],
            },
        }))
    }

    /// Measure `count` frames at rest, then `count` laid out again each as though the camera had
    /// moved, and write what they cost to `path`.
    pub(super) fn perf(
        &mut self,
        state: &mut AppState,
        count: usize,
        path: &Path,
    ) -> Result<(), String> {
        let mut rest = Vec::with_capacity(count);
        for _ in 0..count {
            rest.push(self.measured(state)?);
        }
        let mut relaid = Vec::with_capacity(count);
        for _ in 0..count {
            self.bridge.relay_world();
            relaid.push(self.measured(state)?);
        }
        let pose = Pose::of(&viewer().camera);
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        let json = json!({
            "camera": pose.line(),
            // To find the process's memory at this camera in a sample taken beside the run.
            "unix_ms": unix.map_or(0, |t| t.as_millis() as u64),
            "rest": summary(&rest),
            "relaid": summary(&relaid),
            "last": relaid.last(),
        });
        write(path, &json)
    }

    /// Move the camera to `to` over `over` of the clock, a frame at a time as fast as they draw,
    /// and write what the frames cost to `path`.
    pub(super) fn walk(
        &mut self,
        state: &mut AppState,
        to: &Pose,
        over: Duration,
        path: &Path,
    ) -> Result<(), String> {
        let from = Pose::of(&viewer().camera);
        let started = Instant::now();
        let mut frames = Vec::new();
        loop {
            let t = (started.elapsed().as_secs_f32() / over.as_secs_f32()).min(1.0);
            between(&from, to, t).put(&mut viewer().camera);
            let mut frame = self.measured(state)?;
            frame["t"] = json!(t);
            frames.push(frame);
            if t >= 1.0 {
                break;
            }
        }
        let json = json!({
            "from": from.line(),
            "to": to.line(),
            "seconds": over.as_secs_f32(),
            "moving": summary(&frames),
            "frames": frames,
        });
        write(path, &json)
    }
}

/// What a frame of a model asks of the device, by the names its record gives them.
fn calls_of(calls: &or_renderer::game_shaders::meshlets::MeshletCalls) -> Value {
    json!({
        "passes": calls.passes,
        "draws": calls.draws,
        "surface_draws": calls.surface_draws,
        "bind_group_sets": calls.binds,
        "queue_writes": calls.writes,
        "queue_bytes": calls.written,
        "buffer_copies": calls.copies,
        "vertices_times_instances": calls.vertices,
    })
}

/// The frames of one phase of `model perf`, under the spread of each of their times and counts.
fn phase(frames: Vec<Value>) -> Value {
    let of = |key: &str| spread(frames.iter().filter_map(|f| f[key].as_f64()).collect());
    let stage = |name: &str| {
        let timed = |f: &Value| {
            let stages = f["gpu_stages"].as_array()?;
            stages.iter().find(|s| s[0] == name)?[1].as_f64()
        };
        spread(frames.iter().filter_map(timed).collect())
    };
    let count = |key: &str| {
        spread(
            (frames.iter())
                .filter_map(|f| f["calls"][key].as_f64())
                .collect(),
        )
    };
    let keys = frames.first().and_then(|f| f["calls"].as_object());
    let calls: serde_json::Map<String, Value> = (keys.into_iter().flatten())
        .map(|(key, _)| (key.clone(), count(key)))
        .collect();
    json!({
        "frame_ms": of("frame_ms"),
        "record_ms": of("record_ms"),
        "submit_ms": of("submit_ms"),
        "gpu_ms": {
            "model shadow": stage("model shadow"),
            "model targets": stage("model targets"),
            "model meter": stage("model meter"),
            "lines and models": stage("lines and models"),
        },
        "calls": calls,
        "frames": frames,
    })
}

const NOT_ITS_OWN: &str =
    "no GTA VI model is drawn by its own passes: open one, put `mode shaded` and wait for its passes";

impl Shell {
    /// Draw a frame of the open model and say what it cost; `in_full`, with every pass drawn again.
    fn model_frame(&mut self, state: &mut AppState, in_full: bool) -> Result<Value, String> {
        if in_full {
            self.bridge.redraw_model();
        }
        let started = Instant::now();
        self.idle(state, 1)?;
        let frame = started.elapsed();
        let stages = self.bridge.timed_stages();
        let cost = self.bridge.model_cost().ok_or(NOT_ITS_OWN)?;
        let mut calls = calls_of(&cost.calls);
        // The traced writer of the sun's mask: its rays a frame, which a still view takes to nought.
        calls["rays"] = json!(cost.rays);
        // Rays the watchdog left unfinished, a miss each, since the pass was built.
        calls["capped"] = json!(cost.capped);
        Ok(json!({
            "frame_ms": ms(frame),
            "record_ms": ms(self.submitted.0),
            "submit_ms": ms(self.submitted.1),
            "gpu_stages": stages.map(|s| s.iter().map(|(s, t)| json!([s, t])).collect::<Vec<_>>()),
            "calls": calls,
        }))
    }

    /// Measure `count` frames of the open GTA VI model at rest, then `count` with the camera
    /// turned once round it by even steps, and write what they cost to `path` after each;
    /// `in_full`, every frame draws every pass, as each did before a frame skipped what had
    /// not moved.
    pub(super) fn model_perf(
        &mut self,
        state: &mut AppState,
        count: usize,
        path: &Path,
        in_full: bool,
    ) -> Result<(), String> {
        let cost = self.bridge.model_cost().ok_or(NOT_ITS_OWN)?;
        // The timer times no frame while the last one's stages are unread.
        let timed = self.bridge.timed_stages().is_some();
        let info = self.device.adapter_info();
        let digested = |target: &Option<(u64, usize)>| {
            let (digest, filled) = (*target)?;
            Some(json!({ "digest": format!("{digest:016x}"), "bytes_not_nought": filled }))
        };
        let targets = self.bridge.model_targets();
        // The targets drawn fitted, then with the most vertices an instance has, then fitted
        // again, in this one process: the same maps are bound each time.
        let drawn = |shell: &mut Self, state: &mut AppState, most: bool| {
            shell.bridge.model_at_most(most);
            shell.idle(state, 1)?;
            Ok::<_, String>(shell.bridge.model_targets())
        };
        let fitted = drawn(self, state, false)?;
        let at_most = drawn(self, state, true)?;
        let again = drawn(self, state, false)?;
        let differ = (fitted.iter().zip(&at_most).zip(&again))
            .filter(|((fitted, most), again)| fitted != most || fitted != again)
            .count();
        let mut json = json!({
            "adapter": info.name,
            "backend": format!("{:?}", info.backend),
            "kind": format!("{:?}", info.device_type),
            "driver": format!("{} {}", info.driver, info.driver_info),
            "size": [cost.size.0, cost.size.1],
            "surfaces": cost.surfaces,
            "surfaces_waiting": cost.waiting,
            // A surface's meshlets, and the vertices one of them is drawn with.
            "surfaces_drawn_by": self.bridge.model_surfaces(),
            "meshlets": cost.meshlets,
            "passes_built": cost.passes,
            "casts_a_shadow": cost.casts,
            // What the scene made on the device since it was prepared, and how its maps came in.
            "made": {
                "buffers": cost.made.buffers,
                "buffer_bytes": cost.made.buffer_bytes,
                "textures": cost.made.textures,
                "image_bytes": cost.made.image_bytes,
                "samplers": cost.made.samplers,
                "bind_groups": cost.made.groups,
                "bind_groups_bound_again": cost.made.rebound,
                "scenes_prepared": cost.maps.prepares,
                "maps_bound_late": cost.maps.late,
            },
            "every_pass_every_frame": in_full,
            // As the frames before this step left them: equal in two runs of one build that
            // draw the same triangles from the same place.
            "targets": targets.iter().map(digested).collect::<Vec<_>>(),
            // Drawn in this process with the vertices fitted to each surface, with the most an
            // instance has, and fitted again: the targets that do not hold the same bytes all
            // three times, of those it has.
            "targets_fitted": fitted.iter().map(digested).collect::<Vec<_>>(),
            "targets_at_the_most_vertices": at_most.iter().map(digested).collect::<Vec<_>>(),
            "targets_that_differ_at_the_most_vertices": differ,
            // Where the device times none, a frame's `gpu_stages` is null and never noughts.
            "device_times_stages": timed,
        });
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count {
            frames.push(self.model_frame(state, in_full)?);
        }
        json["rest"] = phase(frames);
        write(path, &json)?;
        // By the angle and never by a drag, so that two runs draw the same frames.
        let yaw = self.bridge.camera().yaw;
        let mut frames = Vec::with_capacity(count);
        for turned in 1..=count {
            let part = turned as f32 / count as f32;
            state.turn_active_camera(yaw + std::f32::consts::TAU * part);
            frames.push(self.model_frame(state, in_full)?);
        }
        state.turn_active_camera(yaw);
        json["orbit"] = phase(frames);
        write(path, &json)?;
        match differ {
            0 => Ok(()),
            _ => Err(format!(
                "{differ} of {} targets differ between the draw fitted to each surface and the draw with the most vertices",
                fitted.len()
            )),
        }
    }
}

/// The spread of each time over `frames`, the layout's over the frames that laid out, and what
/// came, went and came back over them.
fn summary(frames: &[Value]) -> Value {
    let of = |key: &str, laid_only: bool| {
        let values = (frames.iter())
            .filter(|f| !laid_only || f["laid"] == true)
            .filter_map(|f| f[key].as_f64());
        spread(values.collect())
    };
    let total =
        |key: &str, at: usize| -> u64 { frames.iter().filter_map(|f| f[key][at].as_u64()).sum() };
    let delta = |key: &str, at: usize| -> u64 {
        let value = |f: Option<&Value>| f.and_then(|f| f[key][at].as_u64()).unwrap_or(0);
        value(frames.last()).saturating_sub(value(frames.first()))
    };
    json!({
        "frames": frames.len(),
        "frame_ms": of("frame_ms", false),
        "refresh_ms": of("refresh_ms", false),
        "take_ms": of("take_ms", false),
        "lights_ms": of("lights_ms", false),
        "day_ms": of("day_ms", false),
        "record_ms": of("record_ms", false),
        "submit_ms": of("submit_ms", false),
        "layout_ms": of("layout_ms", true),
        "cull_ms": of("cull_ms", true),
        "occlude_ms": of("occlude_ms", true),
        "gpu_ms": of("gpu_ms", false),
        "gpu_gbuffer_ms": of("gpu_gbuffer_ms", false),
        "gpu_shadows_ms": of("gpu_shadows_ms", false),
        "gpu_lighting_ms": of("gpu_lighting_ms", false),
        "laid_frames": frames.iter().filter(|f| f["laid"] == true).count(),
        "uploaded_bytes": [total("uploaded", 0), total("uploaded", 1)],
        "evicted": [delta("evicted", 0), delta("evicted", 1)],
        "reread": [delta("reread", 0), delta("reread", 1)],
    })
}

fn write(path: &Path, json: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    println!("perf  {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spread_is_of_the_sorted_values_and_a_walk_lands_on_its_end() {
        let found = spread(vec![4.0, 1.0, 2.0, 3.0]);
        assert_eq!(found["mean"], 2.5);
        assert_eq!(found["max"], 4.0);
        assert_eq!(found["p95"], 3.0);
        assert_eq!(
            (found["min"].clone(), found["p50"].clone()),
            (json!(1.0), json!(2.0))
        );
        assert_eq!(spread(Vec::new()), Value::Null);
        let a = Pose::read("0 0 0 10 -10 60").expect("a pose");
        let b = Pose::read("50 0 0 30 -10 60").expect("a pose");
        let half = between(&a, &b, 0.5);
        assert_eq!((half.position.x, half.yaw), (25.0, 20.0));
        assert_eq!(between(&a, &b, 1.0), b);
    }

    #[test]
    fn a_phase_of_a_model_s_frames_spreads_their_times_their_stages_and_their_counts() {
        let calls = |draws: u32| {
            calls_of(&or_renderer::game_shaders::meshlets::MeshletCalls {
                draws,
                ..Default::default()
            })
        };
        let frame = |ms: f64, draws: u32, targets: Option<f64>| {
            json!({
                "frame_ms": ms,
                "gpu_stages": targets.map(|t| json!([["model targets", t]])),
                "calls": calls(draws),
            })
        };
        let found = phase(vec![frame(2.0, 3, Some(0.5)), frame(4.0, 7, None)]);
        assert_eq!(found["frame_ms"]["mean"], 3.0);
        assert_eq!(found["calls"]["draws"]["min"], 3.0);
        assert_eq!(found["calls"]["draws"]["max"], 7.0);
        assert_eq!(found["calls"]["vertices_times_instances"]["max"], 0.0);
        // A frame the device did not time is left out, and a stage none timed has no spread.
        assert_eq!(found["gpu_ms"]["model targets"]["n"], 1);
        assert_eq!(found["gpu_ms"]["model shadow"], Value::Null);
        assert_eq!(found["frames"].as_array().map(Vec::len), Some(2));
    }
}
