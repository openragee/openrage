//! The game's own shaders as `naga` modules, built into pipelines: linking the stages of a pass, and the layout a vertex stage's inputs and a pixel stage's outputs ask for.

pub mod ambient;
pub mod antialias;
pub mod bindings;
pub mod bloom;
pub mod capsule_lights;
pub mod constants;
pub mod coronas;
pub mod deferred;
pub mod distant_lights;
pub mod dome;
pub mod drawable;
pub mod effect_pass;
pub mod engine;
pub mod fog_volumes;
pub mod frame;
pub mod grade;
pub mod light_volumes;
pub mod lighting;
pub mod link;
pub mod mesh;
pub mod meshlets;
pub mod mips;
pub mod model;
pub mod pack;
pub mod pipeline;
pub mod point_lights;
pub mod quad;
pub mod resolve;
pub mod room_mask;
pub mod samplers;
pub mod screen;
pub mod shadow;
pub mod sky;
pub mod sky_stars;
pub mod sky_view;
pub mod spot_lights;
pub mod sprite;
pub mod stand_in;
pub mod targets;
pub mod tone;
pub mod trail;
pub mod water;

#[cfg(test)]
pub(crate) mod synthetic;
#[cfg(test)]
pub(crate) mod test_pack;
