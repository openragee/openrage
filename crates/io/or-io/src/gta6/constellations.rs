//! GTA VI's star catalogue: the table-form document of the stars its sky draws as points, each a
//! colour, a place on the sky as two angles, a size and an intensity.

use crate::hash::literal_string_hash as lit;
use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::pso::table_form;
use crate::meta::{MetaInstance, Value};

const STARS: u32 = lit(b"stars");
const COLOR: u32 = lit(b"color");
const POSITION: u32 = lit(b"position");
const SCALE: u32 = lit(b"scale");
const INTENSITY: u32 = lit(b"intensity");

/// Why a catalogue does not read.
#[derive(Debug, thiserror::Error)]
pub enum ConstellationError {
    #[error(transparent)]
    Meta(#[from] crate::meta::MetaError),
    #[error("the catalogue's stars are not an array")]
    NoStars,
}

/// One star of the catalogue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    /// Its colour as the document states it.
    pub colour: [f32; 3],
    /// Its angle from the zenith and its angle round the horizon, as the document states them.
    pub position: [f32; 2],
    pub scale: f32,
    pub intensity: f32,
}

/// The stars of the catalogue document `bytes`, in its order.
pub fn read(bytes: &[u8]) -> Result<Vec<Star>, ConstellationError> {
    let tree = table_form::decode(&table_form::model_of(bytes)?)?;
    let Some(Value::Array(stars)) = tree.get(&name(STARS)) else {
        return Err(ConstellationError::NoStars);
    };
    Ok(stars.iter().filter_map(object).map(star).collect())
}

fn name(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn object(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(s) | Value::Pointer(Some(s)) => Some(s),
        _ => None,
    }
}

fn star(of: &MetaInstance) -> Star {
    let lanes = |member: u32| -> [f32; 4] {
        match of.get(&name(member)) {
            Some(Value::Vec2([x, y])) => [*x, *y, 0.0, 0.0],
            Some(Value::Vec3([x, y, z])) => [*x, *y, *z, 0.0],
            Some(Value::Vec4([x, y, z, w])) => [*x, *y, *z, *w],
            Some(Value::Float(v)) => [*v as f32, 0.0, 0.0, 0.0],
            _ => [0.0; 4],
        }
    };
    let [r, g, b, _] = lanes(COLOR);
    let [a, z, _, _] = lanes(POSITION);
    Star {
        colour: [r, g, b],
        position: [a, z],
        scale: lanes(SCALE)[0],
        intensity: lanes(INTENSITY)[0],
    }
}
