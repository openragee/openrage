//! The `world perf` and `world move` steps: what frames of the World screen cost on the CPU and
//! the device, at rest and while the camera moves, written as JSON beside the shots.
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
}
