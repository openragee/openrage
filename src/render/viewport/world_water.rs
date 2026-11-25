//! GTA VI's water as one mesh about the camera, built again once the camera has gone far from where
//! it was built or a part arrived: drawn by the frame's water pass where the game's passes light
//! the frame, else as a blended surface tinted by the day.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use or_renderer::world::water::WaterMedia;
use or_renderer::{ViewportRenderer, WorldBucket, WorldDraw, WorldInstance, WorldMesh};

use crate::state::gta6_world::worldview::water::{mesh, tint, Seen, SeenKey};

/// How far from the camera the water's quads are drawn, in metres.
const REACH: f32 = 16_000.0;
/// How far the camera goes before the mesh is built again, in metres.
const REBUILD: f32 = 500.0;

/// What the mesh was built of and for: the water's parts, whether the frame's water pass draws it, and where the camera stood.
type Built = (SeenKey, bool, Vec3);

#[derive(Default)]
pub(super) struct Water {
    built: Option<Built>,
    /// The blended surface and its tint, where the frame's water pass does not draw the water.
    plain: Option<(Arc<WorldMesh>, [f32; 3])>,
    /// Which fog atlas the frame's water pass was given, by where it is held.
    media: Option<usize>,
    /// The same for the finer medium about the camera, and the tile it was laid about.
    near: Option<(usize, Option<(u32, u32)>)>,
}

impl Water {
    /// Build the mesh again where a part of the water arrived, the frame changed how it is lit, or the camera left where it was built; whether it was.
    pub(super) fn refresh(
        &mut self,
        renderer: &mut ViewportRenderer,
        device: &wgpu::Device,
        water: Option<&Seen>,
        eye: Vec3,
    ) -> bool {
        let Some(water) = water else {
            self.plain = None;
            renderer.set_world_water(device, None);
            return self.built.take().is_some();
        };
        renderer.set_world_waves(water.waves(eye.x, eye.y));
        let fog = water.fog.as_deref();
        let held = fog.map(|f| f as *const _ as usize);
        if self.media != held {
            self.media = held;
            renderer.set_world_water_media(
                device,
                fog.map(|f| WaterMedia {
                    west: f.atlas.west(),
                    north: f.atlas.north(),
                    size: f.atlas.size,
                    width: f.extinction.width(),
                    height: f.extinction.height(),
                    extinction: f.extinction.texels(),
                    scattering: f.scattering.texels(),
                }),
            );
        }
        let tile = fog.and_then(|f| f.tile_of(eye.x, eye.y));
        let near = held.map(|h| (h, tile));
        if self.near != near {
            self.near = near;
            let laid = fog.zip(tile).and_then(|(f, t)| f.near(t));
            renderer.set_world_water_near(
                device,
                laid.as_ref().map(|n| WaterMedia {
                    west: n.west,
                    north: n.north,
                    size: [n.size; 2],
                    width: n.side,
                    height: n.side,
                    extinction: &n.extinction,
                    scattering: &n.scattering,
                }),
            );
        }
        let (of, lit) = (water.key(), renderer.has_deferred());
        if let Some((was, was_lit, at)) = &self.built {
            if *was == of && *was_lit == lit && at.distance(eye) < REBUILD {
                return false;
            }
        }
        let medium = water.medium();
        let ground = water.ground.as_deref();
        let (vertices, indices) = mesh(&water.quads.quads, eye, REACH, ground);
        // ⚠ The pass wants the day's medium: until the day is read it draws nothing.
        let look = medium.as_ref().filter(|_| lit).map(|m| {
            let mut look = m.look(&water.quads);
            // Off the fog atlas's grid the water is the open sea the atlas holds most of.
            if let Some((extinction, scattering)) = fog.map(|f| f.commonest) {
                look.extinction = extinction;
                look.albedo =
                    std::array::from_fn(|c| (scattering[c] / extinction[c].max(1e-6)).min(1.0));
            }
            // The mesh holds the quads within reach of where it was built; the sea runs on past them.
            if let Some(sea) = &mut look.sea {
                sea.drawn = [eye.x, eye.y, REACH];
            }
            look
        });
        renderer.set_world_water(device, look.map(|look| (&vertices[..], &indices[..], look)));
        self.plain = (!lit && !indices.is_empty()).then(|| {
            let material = renderer.world_material(device, None, None, None, WorldBucket::Blend);
            let mesh = Arc::new(WorldMesh::new(device, &vertices, &indices, material));
            (mesh, tint(medium.as_ref()))
        });
        self.built = Some((of, lit, eye));
        true
    }

    /// Add the blended surface to a frame's instances and draws, after the `laid` ones, where the frame's water pass does not draw the water.
    pub(super) fn draw(&self, laid: &mut Vec<WorldInstance>, draws: &mut Vec<WorldDraw>) {
        let Some((mesh, [r, g, b])) = &self.plain else {
            return;
        };
        let at = laid.len() as u32;
        // ⚠ The Standard family's albedo is its colour map times the instance's tint, never the vertex colour.
        laid.push(WorldInstance::of(Mat4::IDENTITY, [*r, *g, *b, 1.0]));
        draws.push(WorldDraw {
            mesh: Arc::clone(mesh),
            instances: at..at + 1,
        });
    }
}
