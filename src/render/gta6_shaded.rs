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

/// The active GTA VI drawable as surfaces for the pack's passes.
pub(crate) struct Fallback {
    pub surfaces: Vec<(Surface, Material)>,
    /// Each geometry whose effect the pack draws nothing of: the effect, and why.
    pub left_out: Vec<(String, String)>,
    /// The keys of the maps a worker still decodes, which the surfaces lack.
    pub decoding: Vec<u64>,
}

/// The active GTA VI drawable as surfaces for the game's passes, of the geometries whose effect
/// `draws` takes (`Err` says why not); `None` where it shows none.
// ⚠ Never reads or decodes a map on this thread, and never for a geometry the pack draws nothing
// of: a building's maps decoded here held its first own frame for tens of seconds.
pub(crate) fn shaded_surfaces(
    state: &AppState,
    draws: &dyn Fn(&str) -> Result<(), String>,
) -> Option<Fallback> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let selection = state.selection.selected(doc.id);
    let drawable = drawable_of(resource, selection)?;
    let mut decoding = Vec::new();
    let materials: Vec<(String, Result<Material, String>)> = (drawable.shaders.iter())
        .map(|shader| {
            let effect = effect_name(shader.name_hash);
            let made = draws(&effect)
                .map(|()| material(state, selection, shader, effect.clone(), &mut decoding));
            (effect, made)
        })
        .collect();
    let (mut surfaces, mut left_out) = (Vec::new(), Vec::new());
    for model in drawn_level(drawable, state.preview_scene.lod) {
        for geometry in model.geometries.iter().flatten() {
            let Some((effect, material)) = materials.get(usize::from(geometry.shader)) else {
                continue;
            };
            match (material, surface(geometry)) {
                (Ok(material), Some(surface)) => surfaces.push((surface, material.clone())),
                (Err(why), Some(_)) => left_out.push((effect.clone(), why.clone())),
                (_, None) => {}
            }
        }
    }
    Some(Fallback {
        surfaces,
        left_out,
        decoding,
    })
}

/// Whether `pack` draws anything of the effect named `effect`, as a scene of its passes finds it
/// by name; `Err` says why not.
pub(crate) fn pack_draws(
    pack: &or_renderer::game_shaders::pack::GamePack,
    effect: &str,
) -> Result<(), String> {
    use or_renderer::game_shaders::mesh::MeshError;
    let manifest = pack.manifest();
    let found = (manifest.effects.iter()).find(|e| e.name.eq_ignore_ascii_case(effect));
    match found {
        None => Err(MeshError::NoEffect(effect.to_string()).to_string()),
        Some(e) if e.techniques.iter().all(|t| t.passes.is_empty()) => {
            Err(MeshError::NoPass(effect.to_string()).to_string())
        }
        Some(_) => Ok(()),
    }
}

/// The box the geometries of the active GTA VI drawable's drawn level fill, from its least
/// corner to its greatest; `None` where the document shows none, or none of them reads.
pub(crate) fn bounds(state: &AppState) -> Option<(glam::Vec3, glam::Vec3)> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let selection = state.selection.selected(doc.id);
    let drawable = drawable_of(resource, selection)?;
    let lod = state.preview_scene.lod;
    kept_box((doc.id, doc.payload_epoch(), selection, lod), || {
        let geometries = drawn_level(drawable, lod).flat_map(|model| model.geometries.iter());
        let read = (geometries.flatten()).filter_map(|geometry| geometry.positions.as_ref().ok());
        bounds_of(read.flatten())
    })
}

/// What a box was measured of: the document, what it held when it was read, the drawable picked
/// in it and the level drawn.
// ⚠ Never where the drawable lies: a document opened after another is closed may lie there.
type Measured = (crate::state::DocumentId, u64, Option<u64>, usize);

/// The box kept for `of`, else the one `measure` gives, kept in its place: a frame asks again
/// with the same.
fn kept_box(
    of: Measured,
    measure: impl FnOnce() -> Option<(glam::Vec3, glam::Vec3)>,
) -> Option<(glam::Vec3, glam::Vec3)> {
    type Kept = (Measured, Option<(glam::Vec3, glam::Vec3)>);
    static KEPT: std::sync::Mutex<Option<Kept>> = std::sync::Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, held)) = kept.as_ref().filter(|(held, _)| *held == of) {
        return *held;
    }
    let measured = measure();
    *kept = Some((of, measured));
    measured
}

/// The box `positions` fill; `None` where there is none.
pub(super) fn bounds_of<'a>(
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
/// keys resolve to among the packs the app has as a worker has decoded it, named by the
/// parameter's +0x14 hash; the key of one a worker still decodes is added to `decoding`.
fn material(
    state: &AppState,
    selection: Option<u64>,
    shader: &Shader,
    effect: String,
    decoding: &mut Vec<u64>,
) -> Material {
    let mut material = Material {
        effect,
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
                let Some(key) = key else {
                    continue;
                };
                let stored = || super::gta6_maps::stored(state, selection, key);
                match super::gta6_meshlets::decoded(key, stored) {
                    Ok(Some(image)) => {
                        material.textures.insert(name, Image::clone(&image));
                    }
                    Ok(None) => {}
                    Err(()) => decoding.push(key),
                }
            }
        }
    }
    material
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
    /// Those of them drawn by a forward pass, laid over the lit frame.
    pub forward: usize,
    /// The bindings only the game fills, left at nought.
    pub zeroed: usize,
    /// The surfaces drawn as Material in the same frame, and why the first is.
    pub rest: usize,
    pub why: Option<String>,
    /// Whether the model casts no shadow, for a surface of it that cannot be cast.
    pub unshadowed: bool,
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
            if by_cycles {
                said.push('\n');
                said.push_str(tr!("Of the game's thirty material kinds, the standard and the translucent are lit by their own arithmetic; skin, clear coat, sheen and the rest are lit as the standard one for now"));
            }
            if own.forward > 0 {
                said.push('\n');
                said.push_str(&tr!(
                    "Drawn by their forward passes, laid over the lit frame before it is toned: {count} of those surfaces",
                    count = own.forward,
                ));
                said.push(' ');
                said.push_str(tr!("A lit one among them takes the sun with no shadow: the game's cascades are stated empty, and the editor's shadow tile is not handed to it"));
            }
            if let Some(why) = &own.why {
                said.push('\n');
                said.push_str(&tr!(
                    "Drawn as Material: {count} surfaces; the first: {why}",
                    count = own.rest,
                    why = why,
                ));
            }
            if own.unshadowed {
                said.push('\n');
                said.push_str(tr!("It casts no shadow: one of its surfaces reads the view at an offset the light's view cannot be copied to"));
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
#[path = "gta6_fallback_tests.rs"]
mod fallback_tests;

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
            forward: 0,
            zeroed: 9,
            rest: 0,
            why: None,
            unshadowed: false,
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
        // The game's lighting names the material kinds it lights as the standard one.
        assert!(
            lit.contains("lit as the standard one for now") && !whole.contains("material kinds"),
            "{lit}"
        );
        let laid = Own {
            forward: 1,
            ..own.clone()
        };
        let laid = said_of_own(String::new(), Some(&Ok(laid)), false);
        assert!(
            laid.contains("laid over the lit frame before it is toned: 1 of those surfaces"),
            "{laid}"
        );
        assert!(
            laid.ends_with("the editor's shadow tile is not handed to it"),
            "{laid}"
        );
        assert!(!whole.contains("forward"), "{whole}");
        own.rest = 3;
        own.why = Some("an effect: why".into());
        let partly = said_of_own(String::new(), Some(&Ok(own.clone())), false);
        assert!(
            partly.ends_with("Drawn as Material: 3 surfaces; the first: an effect: why"),
            "{partly}"
        );
        assert!(!partly.contains("casts no shadow"), "{partly}");
        own.unshadowed = true;
        let unshadowed = said_of_own(String::new(), Some(&Ok(own)), false);
        let last = unshadowed.lines().last().unwrap_or_default();
        assert!(last.starts_with("It casts no shadow: "), "{unshadowed}");
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
    fn a_box_measured_of_one_document_is_not_handed_to_the_next() {
        let (first, second) = (
            Some((glam::Vec3::ZERO, glam::Vec3::ONE)),
            Some((glam::Vec3::NEG_ONE, glam::Vec3::splat(4.0))),
        );
        // Documents no other test opens: the box is kept for the whole process.
        let (one, next) = (u64::MAX - 7, u64::MAX - 6);
        assert_eq!(kept_box((one, 0, None, 0), || first), first);
        // Another document, though it may lie where the first did, is measured itself.
        assert_eq!(kept_box((next, 0, None, 0), || second), second);
        // And so is the same document read again, another drawable of it, and another level.
        assert_eq!(kept_box((next, 1, None, 0), || first), first);
        assert_eq!(kept_box((next, 1, Some(3), 0), || second), second);
        assert_eq!(kept_box((next, 1, Some(3), 2), || None), None);
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
