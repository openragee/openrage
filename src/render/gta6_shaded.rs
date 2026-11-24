//! The active GTA VI drawable for Shaded: the geometries and level of detail Solid draws, each with
//! its shader's effect, its parameter block's constants and the textures its keys resolve to, named
//! as the translated programs' bindings name them (`0x` and the +0x14 hash).

use or_io::gta6::headerless::{Drawable, Geometry, Model, ParameterValue, Shader};
use or_renderer::game_shaders::drawable::first_level;
use or_renderer::game_shaders::mesh::{Image, Material, Surface};

use crate::state::gta6_resource::{drawable_of, effect_name, texture_name};
use crate::state::gta6_shaders::Status;
use crate::state::{texture_resolution, AppState, DocPayload};

/// The name hashes of the shaders of the active document's GTA VI drawable; `None` where the
/// active document shows none.
pub(crate) fn shader_hashes(state: &AppState) -> Option<Vec<u32>> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let drawable = drawable_of(resource, state.selection.selected(doc.id))?;
    Some(drawable.shaders.iter().map(|s| s.name_hash).collect())
}

/// The active GTA VI drawable as surfaces for the game's passes; `None` where it shows none.
pub(crate) fn shaded_surfaces(state: &AppState) -> Option<Vec<(Surface, Material)>> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let selection = state.selection.selected(doc.id);
    let drawable = drawable_of(resource, selection)?;
    let materials: Vec<Material> = (drawable.shaders.iter())
        .map(|shader| material(state, selection, shader))
        .collect();
    let mut out = Vec::new();
    for model in drawn_level(drawable, state.preview_scene.lod) {
        for geometry in model.geometries.iter().flatten() {
            let material = materials.get(usize::from(geometry.shader));
            if let (Some(surface), Some(material)) = (surface(geometry), material) {
                out.push((surface, material.clone()));
            }
        }
    }
    Some(out)
}

/// The box the geometries of the active GTA VI drawable's drawn level fill, from its least
/// corner to its greatest; `None` where the document shows none, or none of them reads.
pub(crate) fn bounds(state: &AppState) -> Option<(glam::Vec3, glam::Vec3)> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let drawable = drawable_of(resource, state.selection.selected(doc.id))?;
    let level = drawn_level(drawable, state.preview_scene.lod);
    // Measured once for a drawable and a level: a frame asks again with the same.
    type Measured = ((usize, usize), Option<(glam::Vec3, glam::Vec3)>);
    static KEPT: std::sync::Mutex<Option<Measured>> = std::sync::Mutex::new(None);
    let key = (
        std::ptr::from_ref(drawable) as usize,
        state.preview_scene.lod,
    );
    let mut kept = KEPT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, held)) = kept.as_ref().filter(|(of, _)| *of == key) {
        return *held;
    }
    let geometries = level.flat_map(|model| model.geometries.iter().flatten());
    let read = geometries.filter_map(|geometry| geometry.positions.as_ref().ok());
    let measured = bounds_of(read.flatten());
    *kept = Some((key, measured));
    measured
}

/// The box `positions` fill; `None` where there is none.
fn bounds_of<'a>(
    positions: impl Iterator<Item = &'a [f32; 3]>,
) -> Option<(glam::Vec3, glam::Vec3)> {
    positions.fold(None, |corners, position| {
        let at = glam::Vec3::from(*position);
        Some(
            corners.map_or((at, at), |(least, most): (glam::Vec3, glam::Vec3)| {
                (least.min(at), most.max(at))
            }),
        )
    })
}

/// The models of level `lod`, or of the first level where that one has none, as Solid draws them.
fn drawn_level(drawable: &Drawable, lod: usize) -> impl Iterator<Item = &Model> {
    let held = |level: usize| {
        (drawable.levels.get(level))
            .and_then(Option::as_ref)
            .filter(|models| !models.is_empty())
    };
    held(lod).or_else(|| held(0)).into_iter().flatten()
}

/// A geometry whose positions read, with each channel that holds a value for every vertex.
fn surface(geometry: &Geometry) -> Option<Surface> {
    let positions = geometry.positions.as_ref().ok()?;
    let v = &geometry.vertices;
    let count = positions.len();
    let indices = (geometry.triangles.iter())
        .filter(|t| t.iter().all(|i| (*i as usize) < count))
        .flatten()
        .copied()
        .collect();
    let whole = |len: usize| len == count;
    Some(Surface {
        positions: positions.clone(),
        normals: if whole(v.normals.len()) {
            v.normals.clone()
        } else {
            Vec::new()
        },
        tangents: if whole(v.tangents.len()) {
            v.tangents.clone()
        } else {
            Vec::new()
        },
        uvs: (v.uvs.iter())
            .filter(|(c, values)| (28..32).contains(c) && values.len() == count)
            .map(|(c, values)| (u32::from(c - 28), values.clone()))
            .collect(),
        colours: (v.colours.iter())
            .filter(|(c, values)| (24..28).contains(c) && values.len() == count)
            .map(|(c, values)| (u32::from(c - 24), values.clone()))
            .collect(),
        indices,
        ..Surface::default()
    })
}

/// A shader's effect, each constant its parameter block sets in rows of four, and each texture its
/// keys resolve to among the packs the app has, named by the parameter's +0x14 hash.
fn material(state: &AppState, selection: Option<u64>, shader: &Shader) -> Material {
    let mut material = Material {
        effect: effect_name(shader.name_hash),
        ..Material::default()
    };
    for parameter in &shader.parameters {
        let name = format!("0x{:08X}", parameter.hash);
        match &parameter.value {
            ParameterValue::Constant { words, .. } => {
                let rows = words
                    .chunks(4)
                    .map(|chunk| {
                        let mut row = [0.0; 4];
                        for (lane, word) in row.iter_mut().zip(chunk) {
                            *lane = f32::from_bits(*word);
                        }
                        row
                    })
                    .collect();
                material.vectors.push((name, rows));
            }
            ParameterValue::Texture { slot } => {
                let key = shader
                    .texture_keys
                    .get(usize::from(*slot))
                    .copied()
                    .flatten();
                if let Some(image) = key.and_then(|key| texture(state, selection, key)) {
                    material.textures.insert(name, image);
                }
            }
        }
    }
    material
}

/// The first level of the texture of `key`, found as the app finds it for a mesh.
pub(super) fn texture(state: &AppState, selection: Option<u64>, key: u64) -> Option<Image> {
    let resolved = texture_resolution::resolve(state, None, selection, &texture_name(key))?;
    first_level(&resolved.texture)
}

/// The first level of a texture as its pack stores it, copied out of the pack so that a worker
/// can decode it while the frames go on.
pub(super) struct StoredLevel {
    format: or_io::codec::texture::GrcTextureFormat,
    srgb: bool,
    size: (u32, u32),
    payload: Vec<u8>,
}

impl or_io::codec::texture::TextureSource for StoredLevel {
    fn pixel_format(&self) -> or_io::codec::texture::GrcTextureFormat {
        self.format
    }
    fn is_srgb(&self) -> bool {
        self.srgb
    }
    fn stored_size(&self) -> (u32, u32) {
        self.size
    }
    fn stored_levels(&self) -> u32 {
        1
    }
    fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl StoredLevel {
    /// The level decoded, in the colour space it states; `None` where it will not decode.
    pub(super) fn decode(&self) -> Option<Image> {
        first_level(self)
    }
}

/// The first level of the texture of `key` as stored, found as the app finds it for a mesh.
pub(super) fn stored_level(
    state: &AppState,
    selection: Option<u64>,
    key: u64,
) -> Option<StoredLevel> {
    use or_io::codec::texture::TextureSource;
    let resolved = texture_resolution::resolve(state, None, selection, &texture_name(key))?;
    let texture = &resolved.texture;
    let (format, size) = (texture.pixel_format(), texture.stored_size());
    let level = format.mip_level_range(size.0, size.1, 0);
    Some(StoredLevel {
        format,
        srgb: texture.is_srgb(),
        size,
        payload: texture.payload().get(level)?.to_vec(),
    })
}

/// What the Shaded button says of the active GTA VI drawable: where its programs stand and, once
/// translated, how many of `total` surfaces drew and why the first refused program is refused.
pub(crate) fn said(status: &Status, drawn: usize, total: usize) -> String {
    let (refused, first) = match status {
        Status::NoSource => {
            return tr!("Shaded — the game's own passes. No GTA VI effect container read so far holds this drawable's effects, so it draws as Material").to_string()
        }
        Status::Translating => {
            return tr!("Shaded — the game's own passes. This drawable's GTA VI programs are being translated; until then it draws as Material").to_string()
        }
        Status::Done { refused, first } => (*refused, first),
    };
    let mut said = match drawn {
        0 => tr!("Shaded — the game's own passes, translated from GTA VI's programs: none of its surfaces draws with them, so it draws as Material").to_string(),
        _ => tr!(
            "Shaded — the game's own passes, translated from GTA VI's programs: {drawn} of {total} surfaces drawn",
            drawn = drawn,
            total = total,
        ),
    };
    if let Some(why) = first {
        said.push('\n');
        said.push_str(&tr_n!(
            "{count} program is not translated: {why}",
            "{count} programs are not translated; the first: {why}",
            refused,
            why = why,
        ));
    }
    said
}

/// How a drawable's own mesh and pixel programs came out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Own {
    /// The surfaces they draw.
    pub drawn: usize,
    /// The bindings only the game fills, left at nought.
    pub zeroed: usize,
    /// The surfaces drawn as Material in the same frame, and why the first is.
    pub rest: usize,
    pub why: Option<String>,
}

/// `said` with how the drawable's own mesh and pixel programs came out: the surfaces they draw
/// and the bindings left at nought, or why it draws without them; `by_cycles` is whether the
/// install's weather cycles light it.
pub(crate) fn said_of_own(
    said: String,
    own: Option<&Result<Own, String>>,
    by_cycles: bool,
) -> String {
    match own {
        None => said,
        Some(Ok(own)) => {
            let mut said = match by_cycles {
                false => tr!(
                    "Shaded — this drawable's own mesh and pixel programs, translated from GTA VI's: {drawn} surfaces drawn, lit by the editor's own sun and sky at the hour and the weather set. {zeroed} bindings only the game fills read as noughts",
                    drawn = own.drawn,
                    zeroed = own.zeroed,
                ),
                true => tr!(
                    "Shaded — this drawable's own mesh and pixel programs, translated from GTA VI's: {drawn} surfaces drawn, lit by the install's weather cycle at the hour and the weather set, under the sky its air gives, with the game's metering and tone curve. {zeroed} bindings only the game fills read as noughts",
                    drawn = own.drawn,
                    zeroed = own.zeroed,
                ),
            };
            if let Some(why) = &own.why {
                said.push('\n');
                said.push_str(&tr!(
                    "Drawn as Material: {count} surfaces; the first: {why}",
                    count = own.rest,
                    why = why,
                ));
            }
            said
        }
        Some(Err(why)) => {
            let own = tr!(
                "Its own mesh and pixel programs do not draw it: {why}",
                why = why,
            );
            format!(
                "{said}
{own}"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shaded_button_names_the_first_refusal_and_how_many_there_are() {
        let done = Status::Done {
            refused: 3,
            first: Some("no bitcode in it".into()),
        };
        let said = said(&done, 0, 5);
        assert!(said.contains("none of its surfaces"), "{said}");
        assert!(
            said.ends_with("3 programs are not translated; the first: no bitcode in it"),
            "{said}"
        );
        let partly = said_of(1, 2, 7);
        assert!(partly.contains("2 of 7 surfaces drawn"), "{partly}");
        assert!(
            partly.ends_with("1 program is not translated: why"),
            "{partly}"
        );
        let clean = Status::Done {
            refused: 0,
            first: None,
        };
        assert!(!super::said(&clean, 4, 4).contains('\n'));
        assert_ne!(
            super::said(&Status::NoSource, 0, 0),
            super::said(&Status::Translating, 0, 0)
        );
    }

    #[test]
    fn the_shaded_button_says_what_draws_as_material_beside_the_drawable_s_own_programs() {
        let mut own = Own {
            drawn: 2,
            zeroed: 9,
            rest: 0,
            why: None,
        };
        let whole = said_of_own(String::new(), Some(&Ok(own.clone())), false);
        assert!(
            whole.contains("2 surfaces drawn") && !whole.contains('\n'),
            "{whole}"
        );
        let lit = said_of_own(String::new(), Some(&Ok(own.clone())), true);
        assert!(
            lit.contains("the install's weather cycle") && !whole.contains("weather cycle"),
            "{lit}"
        );
        own.rest = 3;
        own.why = Some("an effect: why".into());
        let partly = said_of_own(String::new(), Some(&Ok(own)), false);
        assert!(
            partly.ends_with("Drawn as Material: 3 surfaces; the first: an effect: why"),
            "{partly}"
        );
        let none = said_of_own("before".into(), Some(&Err("why".into())), false);
        assert!(
            none.starts_with("before\n") && none.ends_with("why"),
            "{none}"
        );
    }

    fn said_of(refused: usize, drawn: usize, total: usize) -> String {
        let status = Status::Done {
            refused,
            first: Some("why".into()),
        };
        super::said(&status, drawn, total)
    }

    #[test]
    fn a_surface_keeps_each_channel_that_holds_a_value_for_every_vertex() {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.5], [0.0, 2.0, 0.25]];
        let vertices = or_io::gta6::headerless::Vertices {
            normals: vec![[0.0, 0.0, 1.0]; 3],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 2],
            uvs: vec![
                (28, vec![[0.5, 0.25]; 3]),
                (29, vec![[0.75, 0.125]; 2]),
                (30, vec![[0.0, 1.0]; 3]),
            ],
            colours: vec![(25, vec![[0.5, 0.5, 0.5, 1.0]; 3])],
            ..Default::default()
        };
        let geometry = Geometry {
            shader: 0,
            vertex_count: 3,
            channels: Vec::new(),
            triangles: vec![[0, 1, 2], [0, 1, 3], [2, 1, 0]],
            positions: Ok(positions.clone()),
            vertices,
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        };
        let surface = surface(&geometry).unwrap();
        assert_eq!(surface.positions, positions);
        assert_eq!(
            bounds_of(positions.iter()),
            Some((glam::Vec3::ZERO, glam::Vec3::new(1.0, 2.0, 0.5)))
        );
        assert_eq!(bounds_of([].iter()), None);
        assert_eq!(surface.indices, vec![0, 1, 2, 2, 1, 0]);
        assert_eq!(surface.normals.len(), 3);
        assert!(surface.tangents.is_empty());
        assert_eq!(surface.uvs.keys().copied().collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(surface.uvs[&2], vec![[0.0, 1.0]; 3]);
        assert_eq!(surface.colours.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert!(!surface.skinned);
    }
}
