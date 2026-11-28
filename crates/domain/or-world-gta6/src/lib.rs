//! GTA VI's world behind the world contract: the index of an install with its kept file, mount
//! binding, scan and states, the maps streamed in around a camera with what of them draws, their
//! interiors and occluders, a model's material record, and the law and variables of its lights.

pub mod ambient;
pub mod index;
pub mod interior;
pub mod interior_rooms;
pub mod lights_law;
pub mod lights_vars;
pub mod material;
pub mod occluders;
pub mod stream;
pub mod streaming_rules;
