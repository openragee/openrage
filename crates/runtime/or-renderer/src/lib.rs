// wgpu's auto-trait chain (`BindGroupLayout: Send`) is deeper than the default 128 on this nightly.
#![recursion_limit = "256"]
#![allow(
    clippy::needless_range_loop,
    clippy::doc_lazy_continuation,
    clippy::too_many_arguments,
    clippy::derivable_impls,
    clippy::result_unit_err,
    clippy::collapsible_if
)]
pub mod context;
pub mod error;
pub mod fly;
pub mod game_shaders;
pub mod gpu_timer;
pub mod particle_game;
pub mod particle_meshes;
pub mod particle_models;
mod particle_slots;
pub mod particle_sprites;
pub mod particle_trails;
pub mod particles;
pub mod stencil;
pub mod surface;
pub mod trails;
pub mod viewport;
pub mod world;
pub use context::{RenderContext, CLEAR_COLOR};
pub use error::RenderError;
pub use fly::{FlyCamera, Frustum};
pub use game_shaders::fog_volumes::{FogLight, FogVolume};
pub use game_shaders::point_lights::PointLight;
pub use game_shaders::spot_lights::SpotLight;
pub use particle_models::{ModelBatch, ModelGeometry, ModelInstance, ModelVertex};
pub use particles::{ParticleAlignment, ParticleBatch, ParticleBlendMode, ParticleInstance};
pub use surface::{FrameStats, Renderer};
pub use trails::{TrailBatch, TrailVertex, MAX_TRAIL_CONTROL_POINTS};
pub use viewport::{LineVertex, MeshDraw, MeshVertex, OrbitCamera, ShadingMode, ViewportRenderer};
pub use world::{WorldDraw, WorldInstance, WorldMesh};
