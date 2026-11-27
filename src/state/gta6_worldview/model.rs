//! A model as a worker read it: every level of detail its body holds, each mesh with the material
//! and the bucket its shader states.

use std::collections::HashMap;
use std::fs::File;

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::headerless::{
    self, Drawable, Entry, ParameterValue, Resource, Shader, TintPalette,
};
use or_renderer::MeshVertex;

use super::index::{ModelRef, WorldIndex};
use super::texture::Places;

use material::{material_of, GLOW, PALETTE};
#[cfg(test)]
pub use material::{Constants, Emissive, Interior, Layer};
pub use material::{CpuMaterial, Family, RegionTint};
use or_world_gta6::material;

/// The bytes a model's body is read to.
const BODY_LIMIT: u64 = 512 << 20;
/// What the name of a drawable that is a far stand-in of a plant holds: the card pass draws it, never the model pass.
const STAND_IN: &str = "_imposter";

/// A model on the CPU: its levels of detail, from the highest down.
pub struct CpuModel {
    pub lods: Vec<CpuLod>,
    /// The tint palettes the model holds itself, by the key its materials name each by: no pack holds them.
    pub palettes: Vec<(u64, Vec<[u8; 4]>)>,
}

/// One level of a model, drawn while its instance is nearer than `until` metres; the last is drawn for ever.
pub struct CpuLod {
    pub until: f32,
    pub meshes: Vec<CpuMesh>,
}

/// One mesh of a level, before it is on the device.
pub struct CpuMesh {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub material: CpuMaterial,
    pub bucket: Bucket,
}

/// How a mesh is drawn, as its material states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Bucket {
    #[default]
    Opaque,
    Cutout,
    Blend,
    Decal,
    Emissive,
}

impl Bucket {
    /// The bucket of a material whose draw bucket byte is `byte`, which glows where `glows`;
    /// `None` for one that draws no colour.
    fn of(byte: u8, glows: bool) -> Option<Self> {
        // ⚠ The high bit marks a material drawn in batches: it is no part of the bucket.
        match byte & 0x7F {
            0 if glows => Some(Self::Emissive),
            1 => Some(Self::Blend),
            2 => Some(Self::Decal),
            3 => Some(Self::Cutout),
            // Masks that keep water out of a hull, and a surface that marks water for the game's own pass.
            4..=6 => None,
            _ => Some(Self::Opaque),
        }
    }
}

impl CpuModel {
    /// The bytes of its vertices and indices.
    pub fn bytes(&self) -> u64 {
        let meshes = self.lods.iter().flat_map(|lod| &lod.meshes);
        meshes
            .map(|m| (size_of_val(&*m.vertices) + size_of_val(&*m.indices)) as u64)
            .sum()
    }
}

/// Whether `shader` states a glow above none.
fn glows(shader: &Shader) -> bool {
    shader.parameters.iter().any(|p| match &p.value {
        ParameterValue::Constant { words, .. } if p.hash == GLOW => {
            words.first().is_some_and(|w| f32::from_bits(*w) > 0.0)
        }
        _ => false,
    })
}

/// `drawable` as a model: each level that holds a mesh, with the distance its body states, each
/// mesh's bucket and family from its shader's byte of `buckets`; a water mask's mesh is dropped,
/// and so is every mesh of a far stand-in.
pub(super) fn model_of(
    drawable: &Drawable,
    buckets: &[u8],
    srgb: &dyn Fn(u64) -> Option<bool>,
) -> CpuModel {
    // ⚠ A stand-in's texture is an atlas of views of its plant, one a cell: drawn as a mesh its coordinates span every cell, which shows as pale speckles.
    if drawable.name.contains(STAND_IN) {
        let nothing = CpuLod {
            until: f32::INFINITY,
            meshes: Vec::new(),
        };
        return CpuModel {
            lods: vec![nothing],
            palettes: Vec::new(),
        };
    }
    let materials: Vec<Option<(CpuMaterial, Bucket)>> = (drawable.shaders.iter().enumerate())
        .map(|(at, shader)| {
            let byte = buckets.get(at).copied().unwrap_or(0);
            let bucket = Bucket::of(byte, glows(shader))?;
            Some((material_of(shader, byte, srgb), bucket))
        })
        .collect();
    let mut lods = Vec::new();
    for (level, models) in drawable.levels.iter().enumerate() {
        let Some(models) = models.as_ref().filter(|models| !models.is_empty()) else {
            continue;
        };
        let read = crate::render::gta6_drawable::meshes_at(drawable, level, None, None, &|_| None);
        // ⚠ One mesh comes back for each geometry whose positions read, in the models' order.
        let geometries = (models.iter())
            .flat_map(|model| model.geometries.iter().flatten())
            .filter(|geometry| geometry.positions.is_ok());
        let meshes: Vec<CpuMesh> = (read.into_iter().zip(geometries))
            .filter(|(mesh, _)| !mesh.vertices.is_empty() && !mesh.indices.is_empty())
            .filter_map(|(mut mesh, geometry)| {
                let (material, bucket) = materials.get(usize::from(geometry.shader))?.as_ref()?;
                // The first colour is channel 24 and the second 25: the palette column, the coverage, the tint and the glow; a four-layer mesh keeps its layer weights in 25 and its tint in 24, and grass its tint in 24, which their effects read as the tint they take
                let colour = |channel: u8| {
                    let mut colours = geometry.vertices.colours.iter();
                    colours
                        .find(|(c, _)| *c == channel)
                        .map(|(_, values)| values.as_slice())
                };
                let uvs = |channel: u8| {
                    let mut uvs = geometry.vertices.uvs.iter();
                    uvs.find(|(c, _)| *c == channel)
                        .map(|(_, values)| values.as_slice())
                };
                let colours = match matches!(material.family, Family::FourLayer | Family::Grass) {
                    true => (colour(25), colour(24)),
                    false => (colour(24), colour(25)),
                };
                or_renderer::world::coloured(&mut mesh.vertices, colours, (uvs(29), uvs(30)));
                // The interior effect sizes its rooms by the third colour and the fourth coordinate
                if material.interior.is_some() {
                    or_renderer::world::interior(&mut mesh.vertices, colour(26), uvs(31));
                }
                Some(CpuMesh {
                    vertices: mesh.vertices,
                    indices: mesh.indices,
                    material: material.clone(),
                    bucket: *bucket,
                })
            })
            .collect();
        if !meshes.is_empty() {
            lods.push(CpuLod {
                until: drawable.lod_distances[level],
                meshes,
            });
        }
    }
    // What cuts a model off is its entity's own distance, never its last level's.
    match lods.last_mut() {
        Some(last) => last.until = f32::INFINITY,
        None => lods.push(CpuLod {
            until: f32::INFINITY,
            meshes: Vec::new(),
        }),
    }
    CpuModel {
        lods,
        palettes: Vec::new(),
    }
}

/// Each palette key `drawable`'s shaders name, the first named taking the first of `rows` and the second the second, with that row's colours.
fn palettes_of(drawable: &Drawable, rows: &[TintPalette]) -> Vec<(u64, Vec<[u8; 4]>)> {
    let mut keys: Vec<u64> = Vec::new();
    for shader in &drawable.shaders {
        let named = shader.parameters.iter().filter_map(|p| match p.value {
            ParameterValue::Texture { slot } if p.hash == PALETTE => shader
                .texture_keys
                .get(usize::from(slot))
                .copied()
                .flatten(),
            _ => None,
        });
        for key in named {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    (keys.into_iter().enumerate())
        .filter_map(|(at, key)| {
            let row = rows.get(at).or(rows.first())?;
            Some((key, row.colours.clone()))
        })
        .collect()
}

/// The model `model` names, read from its archive; `None` where it does not read.
///
/// A fragment is its one drawable: every part, its glass too, is a geometry of it at rest.
pub(super) fn read_model(
    files: &mut HashMap<u32, File>,
    index: &WorldIndex,
    places: Option<&Places>,
    model: ModelRef,
) -> Option<CpuModel> {
    let file = match files.entry(model.archive) {
        std::collections::hash_map::Entry::Occupied(open) => open.into_mut(),
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(File::open(index.archives.get(model.archive as usize)?).ok()?)
        }
    };
    let (find, bytes) = read_rpf9_body(&*file, model.offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let read = headerless::read(signature, virtual_section, Some(physical)).ok()?;
    let drawable = match (&read, model.entry) {
        (Resource::Drawable(d), None) => d,
        (Resource::DrawableWithBound(r), None) => &r.drawable,
        (Resource::Dictionary(dictionary), Some(code)) => {
            (dictionary.entries.iter()).find_map(|(c, e)| match e {
                Ok(Entry::Drawable(d)) if *c == code => Some(&**d),
                _ => None,
            })?
        }
        _ => return None,
    };
    // Bytes that do not read leave every mesh opaque.
    let buckets =
        headerless::shader_buckets(signature, virtual_section, model.entry).unwrap_or_default();
    // With the packs indexed a key is a colour map or it is not; with none nothing is known of it.
    let srgb = |key: u64| places.map(|places| places.is_colour(key));
    let mut cpu = model_of(drawable, &buckets, &srgb);
    // ponytail: a drawable of a dictionary keeps its palettes unread; read its entry's head where its props show grey.
    if model.entry.is_none() {
        if let Ok(head) = headerless::read_drawable_head(virtual_section, Some(physical), false) {
            let rows: Vec<_> = head.palettes.into_iter().flatten().collect();
            cpu.palettes = palettes_of(drawable, &rows);
        }
    }
    Some(cpu)
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
