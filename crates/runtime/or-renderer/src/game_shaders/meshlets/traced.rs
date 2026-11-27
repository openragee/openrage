//! The scene's second writer of the sun's mask: the game's walk handed over, fed each frame,
//! recorded after the targets, and asked about by the Shaded line and the counters.

use glam::{Mat4, Vec3};
use or_io::gta6::accel::Triangle;

use super::{light_bytes, sun_shadow, Lighting, MeshletScene, Sky, RESOLVE};
use crate::game_shaders::trace::{Frame, TracedMask, View};

/// The program that lights the frame, whose light, depth, normal and ground the traced rays read.
pub(crate) fn resolve_program() -> &'static str {
    RESOLVE
}

/// The light the resolve and the traced rays read, of a frame drawn by `view_proj` from `camera` under the built-in sky at ten with its sun towards `towards`.
pub fn light_for(view_proj: [f32; 16], towards: [f32; 3], camera: [f32; 3]) -> Vec<u8> {
    let sky = Sky {
        towards: Vec3::from(towards),
        ..Sky::default()
    };
    let lighting = Lighting {
        view_proj: Mat4::from_cols_array(&view_proj),
        sky,
        camera: Vec3::from(camera),
    };
    light_bytes(&lighting, false).to_vec()
}

impl MeshletScene {
    /// Have the sun's mask written by `traced` where it runs, or by the shadow map alone for `None`.
    pub fn trace_with(&self, traced: Option<TracedMask>) {
        *self.traced.borrow_mut() = traced;
        self.forget();
    }

    /// The number of the arena the traced writer walks, where one is set.
    pub fn traced_of(&self) -> Option<u64> {
        self.traced.borrow().as_ref().map(TracedMask::generation)
    }

    /// How far the traced writer is, where one is set: `None` while a worker builds it, then `Ok` or why the device refused it.
    pub fn traced_state(&self) -> Option<Option<Result<(), String>>> {
        self.traced.borrow().as_ref().map(TracedMask::state)
    }

    /// The rays the traced writer casts this frame.
    pub fn traced_rays(&self) -> u64 {
        self.traced.borrow().as_ref().map_or(0, TracedMask::rays)
    }

    /// The rays of the traced writer the watchdog left unfinished since it was built.
    pub fn traced_capped(&self) -> u64 {
        self.traced.borrow().as_ref().map_or(0, TracedMask::capped)
    }

    /// The traced writer's arena rewritten for `posed`, the model's triangles where a pose puts them.
    pub fn refit_traced(&self, posed: &[Triangle]) -> Result<(), String> {
        match self.traced.borrow_mut().as_mut() {
            Some(traced) => traced.refit(posed),
            None => Err("no traced writer is set".to_owned()),
        }
    }

    /// Whether this frame's mask is the traced writer's, so the shadow map writes none.
    pub(super) fn traced_writes(&self) -> bool {
        self.traced
            .borrow()
            .as_ref()
            .is_some_and(TracedMask::writes)
    }

    /// Hand the traced writer what this frame is drawn by.
    pub(super) fn update_traced(&self, queue: &wgpu::Queue, lighting: &Lighting) {
        let mut traced = self.traced.borrow_mut();
        let Some(traced) = traced.as_mut() else {
            return;
        };
        let size = self.depth.texture().size();
        let view = View {
            view_proj: lighting.view_proj.to_cols_array(),
            towards: lighting.sky.towards.to_array(),
            disc: sun_shadow::words(&lighting.sky, true)[1],
            clouds: lighting.sky.scene.clouds.open,
        };
        let tiles = self.mask.tiles.texture();
        traced.update(queue, &view, (size.width, size.height), tiles);
    }

    /// Record the traced writer's rays into the mask, once the targets hold the frame: whether it wrote it.
    pub(super) fn record_traced(&self, encoder: &mut wgpu::CommandEncoder) -> bool {
        let mut traced = self.traced.borrow_mut();
        let Some(traced) = traced.as_mut() else {
            return false;
        };
        let Some(Some(normal)) = self.targets.get(1) else {
            return false;
        };
        let frame = Frame {
            depth: &self.depth,
            normal,
            light: &self.light,
            roots: self.mask.roots.texture(),
        };
        traced.record(encoder, &frame)
    }
}
