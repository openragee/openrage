//! A scanned GTA VI drawable as the interchange scene a glTF export writes: one mesh per model, one
//! primitive per geometry whose positions read, its skeleton's bones with each vertex's weights,
//! and a material per shader that binds its texture keys by name.

use std::path::{Path, PathBuf};

use or_interchange::{
    Bone, ExtractReport, Image, LodSlot, Material, Mesh, Object, Primitive, Scene, TextureBinding,
};
use or_io::codec::texture::{decode_level, encode_png, TextureSource};
use or_io::container::rpf::{Rpf9Resource, Rpf9Signature};
use or_io::gta5::shaders::shader_defs::ShaderDefinitions;
use or_io::gta6::headerless::{
    Drawable, EmbeddedTexture, Geometry, Model, ParameterValue, Shader, Skeleton,
};
use or_io::gta6::ttpf::{Texture, TexturePack};

/// The parameter a material binds its colour map by, which is its base colour.
const DIFFUSE_TEX: &str = "DiffuseTex";
const DIFFUSE_TEX_HASH: u32 = or_io::gta6::hash::name_hash32(DIFFUSE_TEX.as_bytes());

/// What names a drawable's effects and texture keys.
pub struct Names<'a> {
    pub effect: &'a dyn Fn(u32) -> String,
    pub texture: &'a dyn Fn(u64) -> String,
}

/// The scene `rpf scan-export` writes of the drawable `bytes` hold, a read of `resource`: its own,
/// its fragment's, or the dictionary entry `entry` keys; its effects named by `effects` and its
/// texture keys by the names the body states, else `hash_` and the hash.
pub(crate) fn export(
    resource: &Rpf9Resource,
    bytes: &[u8],
    signature: Rpf9Signature,
    entry: Option<u32>,
    every_lod: bool,
    effects: Option<&ShaderDefinitions>,
    stem: &str,
) -> Result<Scene, String> {
    export_textured(
        resource, bytes, signature, entry, every_lod, effects, stem, None,
    )
}

/// [`export`], each texture the drawable embeds or one of `packs` holds carried as an image where
/// `packs` is given.
#[allow(clippy::too_many_arguments)]
pub(crate) fn export_textured(
    resource: &Rpf9Resource,
    bytes: &[u8],
    signature: Rpf9Signature,
    entry: Option<u32>,
    every_lod: bool,
    effects: Option<&ShaderDefinitions>,
    stem: &str,
    packs: Option<&[TexturePack]>,
) -> Result<Scene, String> {
    use or_io::gta6::headerless::{
        read, read_dictionary_skeletons, read_drawable_head, read_skeleton, Entry, Resource,
    };
    let (virtual_section, physical) = resource.sections(bytes);
    let read = read(signature, virtual_section, Some(physical)).map_err(|e| e.to_string())?;
    let (drawable, fragment, stated) = match (&read, entry) {
        (Resource::Drawable(d), None) => (d, Some(false), None),
        (Resource::DrawableWithBound(r), None) => (&r.drawable, Some(true), Some(r.name.as_str())),
        (Resource::Dictionary(dict), Some(code)) => {
            let found = dict.entries.iter().find_map(|(c, e)| match e {
                Ok(Entry::Drawable(d)) if *c == code => Some(&**d),
                _ => None,
            });
            let found = found.ok_or(format!(
                "the dictionary holds no drawable under {code:#010X}"
            ))?;
            (found, None, None)
        }
        (Resource::Dictionary(_), None) => {
            return Err("this is a dictionary: --entry <code> names the drawable to export".into())
        }
        (_, Some(_)) => return Err("--entry names a dictionary's drawable; this is none".into()),
        _ => return Err("this body is no drawable, fragment or dictionary".into()),
    };
    // A dictionary's entry is skinned to the skeleton it holds itself, where it holds one.
    let skeleton = match (fragment, entry) {
        (Some(fragment), _) => {
            read_skeleton(virtual_section, fragment).map_err(|e| e.to_string())?
        }
        (None, Some(code)) => (read_dictionary_skeletons(virtual_section))
            .map_err(|e| e.to_string())?
            .into_iter()
            .find_map(|(c, skeleton)| (c == code).then_some(skeleton)),
        (None, None) => None,
    };
    let keys = (drawable.shaders.iter())
        .flat_map(|s| s.texture_keys.iter().flatten().copied())
        .collect();
    let names = stated.into_iter().chain([drawable.name.as_str()]);
    let textures = or_io::gta6::texture_names::name_keys(names, &keys);
    let texture = |key: u64| {
        (textures.get(&key)).map_or_else(
            || format!("hash_{key:016X}"),
            |(n, _)| n.to_ascii_lowercase(),
        )
    };
    let effect = |hash: u32| {
        (effects.and_then(|e| e.name_material(hash, &[]).shader))
            .unwrap_or_else(|| format!("hash_{hash:08X}"))
    };
    let names = Names {
        effect: &effect,
        texture: &texture,
    };
    let mut scene = scene(drawable, skeleton.as_ref(), stem, every_lod, &names);
    if let Some(packs) = packs {
        // A head that does not read embeds nothing the export carries; its keys stay named.
        let embedded: Vec<EmbeddedTexture> = match fragment {
            Some(fragment) => read_drawable_head(virtual_section, Some(physical), fragment)
                .map(|head| head.textures.into_iter().flatten().collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let (carried, unread) = textures_of(drawable, packs, embedded, &texture);
        carry_images(&mut scene, &carried);
        (scene.report.textures_not_carried).retain(|name| !unread.contains(name));
        scene.report.textures_not_decoded.extend(unread);
    }
    Ok(scene)
}

/// The `--textures <pack.ttpf|folder>` a textured export takes, and the other words.
pub(crate) fn split_textures(args: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    match args.iter().position(|a| a == "--textures") {
        Some(i) => {
            let path = args
                .get(i + 1)
                .ok_or("--textures needs a .ttpf texture pack or a folder of them")?;
            let path = PathBuf::from(path);
            let mut rest = args.to_vec();
            rest.drain(i..i + 2);
            Ok((Some(path), rest))
        }
        None => Ok((None, args.to_vec())),
    }
}

/// The pack at `path`, or each `.ttpf` directly in the folder `path` is, in name order.
pub(crate) fn open_packs(path: &Path) -> Result<Vec<TexturePack>, String> {
    let paths = match path.is_dir() {
        false => vec![path.to_path_buf()],
        true => {
            let listed = std::fs::read_dir(path)
                .map_err(|e| format!("read_dir({}): {e}", path.display()))?;
            let mut paths: Vec<PathBuf> = (listed.filter_map(|e| e.ok().map(|e| e.path())))
                .filter(|p| (p.extension()).is_some_and(|e| e.eq_ignore_ascii_case("ttpf")))
                .collect();
            paths.sort();
            paths
        }
    };
    (paths.iter())
        .map(|p| TexturePack::open(p).map_err(|e| format!("{}: {e}", p.display())))
        .collect()
}

/// Each texture a key of `drawable` binds, by the name `texture` gives its key: the one `embedded`
/// holds, else the first of `packs` that lists the key; and the names of those that do not read.
fn textures_of(
    drawable: &Drawable,
    packs: &[TexturePack],
    mut embedded: Vec<EmbeddedTexture>,
    texture: &dyn Fn(u64) -> String,
) -> (Vec<(String, Texture)>, Vec<String>) {
    let (mut found, mut unread): (Vec<(String, Texture)>, Vec<String>) = Default::default();
    for key in (drawable.shaders.iter()).flat_map(|s| s.texture_keys.iter().flatten().copied()) {
        let name = texture(key);
        if found.iter().any(|(n, _)| *n == name) || unread.contains(&name) {
            continue;
        }
        let read = match embedded.iter().position(|e| e.key == key) {
            Some(i) => Some(embedded.swap_remove(i).texture.texture.ok()),
            None => packs.iter().find_map(|pack| {
                let index = (pack.entries.iter()).position(|e| e.on_disk && e.key == key)?;
                Some(pack.texture(index).ok())
            }),
        };
        match read {
            Some(Some(pixels)) => found.push((name, pixels)),
            Some(None) => unread.push(name),
            None => {}
        }
    }
    (found, unread)
}

/// Each bound texture decoded once; a base colour is the `DiffuseTex` binding, else the first sRGB.
pub fn carry_images<T: TextureSource>(scene: &mut Scene, textures: &[(String, T)]) {
    let mut decoded: Vec<(String, Option<usize>)> = Vec::new();
    for material in &mut scene.materials {
        // ⚠ A linear texture is a data or normal map: as a base colour it tints the mesh.
        let mut first_srgb: Option<Option<usize>> = None;
        for binding in &mut material.textures {
            let Some((_, texture)) = textures.iter().find(|(n, _)| *n == binding.texture) else {
                continue;
            };
            let image = match decoded.iter().find(|(n, _)| *n == binding.texture) {
                Some((_, image)) => *image,
                None => {
                    let png = decode_level(texture, 0).and_then(|rgba| encode_png(&rgba));
                    let image = png.ok().map(|png| {
                        scene.images.push(Image {
                            name: binding.texture.clone(),
                            png,
                        });
                        scene.images.len() - 1
                    });
                    if image.is_none() {
                        (scene.report.textures_not_decoded).push(binding.texture.clone());
                    }
                    decoded.push((binding.texture.clone(), image));
                    image
                }
            };
            binding.image = image;
            if first_srgb.is_none() && texture.is_srgb() {
                first_srgb = Some(image);
            }
        }
        let diffuse = (material.textures.iter()).find(|b| b.parameter == DIFFUSE_TEX);
        material.base_colour = match diffuse {
            Some(binding) => binding.image,
            None => first_srgb.flatten(),
        };
    }
    (scene.report.textures_not_carried).retain(|name| !decoded.iter().any(|(n, _)| n == name));
}

/// The parameter a material's binding of texture slot `slot` of `shader` is named by:
/// `DiffuseTex` where that record binds the slot, else the slot.
fn parameter_of(shader: &Shader, slot: usize) -> String {
    let diffuse = (shader.parameters.iter()).any(|p| {
        p.hash == DIFFUSE_TEX_HASH
            && matches!(p.value, ParameterValue::Texture { slot: s } if usize::from(s) == slot)
    });
    match diffuse {
        true => DIFFUSE_TEX.to_string(),
        false => format!("texture_slot_{slot}"),
    }
}

/// `drawable` as a scene named `stem`, its highest level of detail or every level, skinned to
/// `skeleton`, its own, where it has one.
pub fn scene(
    drawable: &Drawable,
    skeleton: Option<&Skeleton>,
    stem: &str,
    every_lod: bool,
    names: &Names<'_>,
) -> Scene {
    let mut report = ExtractReport::default();
    let materials: Vec<Material> = (drawable.shaders.iter().enumerate())
        .map(|(i, s)| Material {
            name: format!("shader_{i}"),
            shader: (names.effect)(s.name_hash),
            textures: (s.texture_keys.iter().enumerate())
                .filter_map(|(slot, key)| {
                    Some(TextureBinding {
                        parameter: parameter_of(s, slot),
                        texture: (names.texture)((*key)?),
                        image: None,
                    })
                })
                .collect(),
            ..Material::default()
        })
        .collect();
    let mut meshes = Vec::new();
    for (l, level) in drawable.levels.iter().enumerate() {
        let Some(models) = level else { continue };
        let lod = LodSlot::Level(l as u8);
        if !every_lod && !meshes.is_empty() {
            break;
        }
        for (m, model) in models.iter().enumerate() {
            let mut primitives = Vec::new();
            for (g, geometry) in model.geometries.iter().enumerate() {
                report.geometries += 1;
                let Some(mut p) = (geometry.as_ref().ok())
                    .and_then(|geometry| primitive(geometry, lod, m, g, materials.len()))
                else {
                    report.geometries_without_positions += 1;
                    continue;
                };
                if let (Some(skeleton), Ok(geometry)) = (skeleton, geometry) {
                    match skin(geometry, &p.positions, model, skeleton) {
                        Some((joints, weights)) => (p.joints, p.weights) = (joints, weights),
                        None => report.influences_outside_the_palette += 1,
                    }
                }
                primitives.push(p);
            }
            if !primitives.is_empty() {
                meshes.push(Mesh {
                    name: format!("{}_{}_model{m}", drawable.name, lod.label()),
                    lod,
                    primitives,
                });
            }
        }
    }
    let mut not_carried: Vec<String> = (materials.iter())
        .flat_map(|m| m.textures.iter().map(|t| t.texture.clone()))
        .collect();
    not_carried.sort();
    not_carried.dedup();
    report.textures_not_carried = not_carried;
    Scene {
        name: stem.to_string(),
        objects: vec![Object {
            name: drawable.name.clone(),
            meshes,
            bones: skeleton.map(bones).unwrap_or_default(),
        }],
        materials,
        images: Vec::new(),
        report,
        dropped: Vec::new(),
    }
}

/// The skeleton's bones in the order it stores them, each in its parent's frame.
fn bones(skeleton: &Skeleton) -> Vec<Bone> {
    (skeleton.bones.iter().enumerate())
        .map(|(i, b)| Bone {
            name: b.name.clone(),
            tag: i as u16,
            parent: b.parent.map(usize::from),
            translation: b.translation,
            rotation: b.rotation,
            scale: b.scale,
        })
        .collect()
}

/// Each vertex's bones and their weights, four lanes a vertex.
type Skin = (Vec<[u16; 4]>, Vec<[f32; 4]>);

/// Each vertex's four strongest bones of `skeleton`, which its blend indices name through the
/// model's palette, and their weights summing to one; `None` where the geometry does not join it.
fn skin(
    geometry: &Geometry,
    positions: &[[f32; 3]],
    model: &Model,
    skeleton: &Skeleton,
) -> Option<Skin> {
    let skinned = or_anim::gta6::skin(
        &geometry.vertices,
        positions,
        &[],
        model,
        Some(skeleton),
        skeleton,
    )
    .ok()?;
    let mut joints = Vec::with_capacity(skinned.influences.len());
    let mut weights = Vec::with_capacity(skinned.influences.len());
    for v in &skinned.influences {
        let mut lanes: Vec<(u16, f32)> = (v.bones.iter().zip(v.weights))
            .take(usize::from(v.count))
            .map(|(b, w)| (*b, w))
            .collect();
        lanes.sort_by(|a, b| b.1.total_cmp(&a.1));
        lanes.truncate(4);
        let sum: f32 = lanes.iter().map(|(_, w)| w).sum();
        let (mut j, mut w) = ([0u16; 4], [0f32; 4]);
        for (k, (bone, weight)) in lanes.into_iter().enumerate() {
            j[k] = bone;
            w[k] = if sum > 0.0 { weight / sum } else { 0.0 };
        }
        joints.push(j);
        weights.push(w);
    }
    Some((joints, weights))
}

/// A geometry whose positions read, with the channels decoded for every vertex; `None` otherwise.
fn primitive(
    geometry: &Geometry,
    lod: LodSlot,
    model: usize,
    index: usize,
    materials: usize,
) -> Option<Primitive> {
    let positions = geometry.positions.as_ref().ok()?.clone();
    let n = positions.len();
    let whole = |len: usize| len == n;
    let v = &geometry.vertices;
    let (uv_channels, uvs): (Vec<u8>, Vec<Vec<[f32; 2]>>) = (v.uvs.iter())
        .filter(|(_, uvs)| whole(uvs.len()))
        .map(|(c, uvs)| (*c, uvs.clone()))
        .unzip();
    let (colour_channels, colours): (Vec<u8>, Vec<Vec<[f32; 4]>>) = (v.colours.iter())
        .filter(|(_, c)| whole(c.len()))
        .map(|(c, colours)| (*c, colours.clone()))
        .unzip();
    let material = usize::from(geometry.shader);
    Some(Primitive {
        material: (material < materials).then_some(material),
        material_name: None,
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
        positions,
        uvs,
        uv_channels,
        colours,
        colour_channels,
        joints: Vec::new(),
        weights: Vec::new(),
        indices: geometry.triangles.iter().flatten().copied().collect(),
        lod,
        model,
        geometry: index,
        stated_address: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_interchange::gltf::{self, Container};
    use or_io::gta6::hash::name_hash32;
    use or_io::gta6::headerless::{
        Bone as Gta6Bone, DictionaryTexture, HeaderlessError, Palette, Parameter, Shader, Vertices,
    };
    use or_io::gta6::ttpf::Format;

    fn hashed(key: u64) -> String {
        format!("hash_{key:016X}")
    }

    const NAMES: Names<'static> = Names {
        effect: &|hash| format!("hash_{hash:08X}"),
        texture: &hashed,
    };

    fn geometry(positions: Result<Vec<[f32; 3]>, HeaderlessError>) -> Geometry {
        Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 0,
            vertex_count: 3,
            channels: Vec::new(),
            triangles: vec![[0, 1, 2]],
            positions,
            vertices: Vertices {
                normals: vec![[0.0, 0.0, 1.0]; 3],
                uvs: vec![(28, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]])],
                // Vertex 0 on palette entry 1 alone, vertex 1 shared 3:1, vertex 2 on entry 0.
                weights: vec![(
                    16,
                    vec![
                        [1.0, 0.0, 0.0, 0.0],
                        [0.75, 0.25, 0.0, 0.0],
                        [1.0, 0.0, 0.0, 0.0],
                    ],
                )],
                joints: vec![(20, vec![[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 0, 0]])],
                ..Vertices::default()
            },
        }
    }

    /// Two bones: the root, and the head 1.5 m above it.
    fn skeleton() -> Skeleton {
        let bone = |name: &str, parent, y: f32| Gta6Bone {
            name: name.to_string(),
            hash: name_hash32(name.as_bytes()),
            parent,
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [0.0, y, 0.0],
            scale: [1.0; 3],
            inverse_bind: [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, -y, 0.0],
            ],
        };
        Skeleton {
            bones: vec![
                bone("SKEL_ROOT", None, 0.0),
                bone("SKEL_Head", Some(0), 1.5),
            ],
        }
    }

    fn drawable() -> Drawable {
        let triangle = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let refused = Err(HeaderlessError::Positions {
            why: "the decoded vertices do not reach the geometry's box",
        });
        let level = |geometries: Vec<Geometry>| {
            Some(vec![Model {
                geometries: geometries.into_iter().map(Ok).collect(),
                palette: Ok(Palette::Own),
                matrix_index: 0,
            }])
        };
        Drawable {
            name: "prop_cube.#dr".to_string(),
            box_min: [0.0; 3],
            box_max: [1.0, 1.0, 0.0],
            shaders: vec![Shader {
                name_hash: 0x1234_5678,
                texture_keys: vec![Some(0x0123_4567_89AB_CDEF), None],
                parameters: Vec::new(),
            }],
            levels: [
                level(vec![geometry(Ok(triangle.clone())), geometry(refused)]),
                level(vec![geometry(Ok(triangle))]),
                None,
                None,
            ],
            lod_distances: [9998.0; 4],
            bound: None,
            embedded: None,
            extra_skeletons: Vec::new(),
        }
    }

    #[test]
    fn a_gta_vi_drawable_exports_its_highest_level_with_its_channels_and_counts_what_it_drops() {
        let scene = scene(&drawable(), None, "prop_cube", false, &NAMES);
        assert_eq!(scene.objects.len(), 1);
        let meshes = &scene.objects[0].meshes;
        assert_eq!(meshes.len(), 1, "only the highest level");
        let p = &meshes[0].primitives;
        assert_eq!(
            p.len(),
            1,
            "the geometry whose positions were refused is left out"
        );
        assert_eq!(p[0].indices, [0, 1, 2]);
        assert_eq!(p[0].normals.len(), 3);
        assert_eq!(p[0].uv_channels, [28]);
        assert_eq!(p[0].material, Some(0));
        assert!(p[0].joints.is_empty(), "no skeleton, no skin");
        assert_eq!(scene.report.geometries, 2);
        assert_eq!(scene.report.geometries_without_positions, 1);
        assert_eq!(
            scene.materials[0].textures.len(),
            1,
            "the empty slot binds nothing"
        );
        assert_eq!(
            scene.report.textures_not_carried,
            [hashed(0x0123_4567_89AB_CDEF)]
        );
        let every = super::scene(&drawable(), None, "prop_cube", true, &NAMES);
        assert_eq!(every.objects[0].meshes.len(), 2, "every level");
    }

    #[test]
    fn a_skinned_gta_vi_drawable_exports_its_bones_and_each_vertex_s_weights_by_bone() {
        let skeleton = skeleton();
        let scene = scene(&drawable(), Some(&skeleton), "prop_cube", false, &NAMES);
        let bones = &scene.objects[0].bones;
        assert_eq!(bones.len(), 2);
        assert_eq!(bones[1].name, "SKEL_Head");
        assert_eq!(bones[1].parent, Some(0));
        assert_eq!(bones[1].translation, [0.0, 1.5, 0.0]);
        let p = &scene.objects[0].meshes[0].primitives[0];
        assert_eq!(p.joints.len(), 3);
        assert_eq!(
            (p.joints[0][0], p.weights[0][0]),
            (1, 1.0),
            "the head alone"
        );
        assert_eq!(p.joints[1][..2], [0, 1], "the root, then the head");
        assert_eq!(p.weights[1][..2], [0.75, 0.25]);
        assert_eq!(
            (p.joints[2][0], p.weights[2][0]),
            (0, 1.0),
            "the root alone"
        );
        assert!(scene.objects[0].is_skinned());

        let files = gltf::write(&scene, "prop_cube", Container::Glb);
        assert_eq!(&files.primary[..4], b"glTF");
        let json = String::from_utf8_lossy(&files.primary);
        assert!(json.contains("\"skins\""), "the file carries a skin");
        assert!(json.contains("SKEL_Head"));
    }

    #[test]
    fn the_slot_a_diffuse_tex_record_binds_is_named_by_it_and_every_other_by_its_slot() {
        let mut drawable = drawable();
        drawable.shaders[0].texture_keys = vec![Some(0xA1), Some(0xB2)];
        let diffuse = name_hash32(b"DiffuseTex");
        drawable.shaders[0].parameters = vec![
            Parameter {
                hash: diffuse ^ 1,
                value: ParameterValue::Texture { slot: 0 },
            },
            Parameter {
                hash: diffuse,
                value: ParameterValue::Texture { slot: 1 },
            },
        ];
        let scene = scene(&drawable, None, "prop_cube", false, &NAMES);
        let named: Vec<&str> = (scene.materials[0].textures.iter())
            .map(|t| t.parameter.as_str())
            .collect();
        assert_eq!(named, ["texture_slot_0", "DiffuseTex"]);
    }

    /// A 4×4 texture of one colour, uncompressed, its format sRGB where `srgb`.
    fn decodable(srgb: bool) -> or_io::gta5::gen8::GrcTexture {
        or_io::gta5::gen8::GrcTexture {
            width: 4,
            height: 4,
            depth: 1,
            format: or_io::codec::texture::GrcTextureFormat::A8R8G8B8.to_raw_format(),
            mip_count: 1,
            info_bits: u8::from(srgb),
            backing_store: vec![0x7F; 4 * 4 * 4],
            ..Default::default()
        }
    }

    fn binding(parameter: &str, texture: &str) -> TextureBinding {
        TextureBinding {
            parameter: parameter.to_string(),
            texture: texture.to_string(),
            image: None,
        }
    }

    #[test]
    fn a_base_colour_is_the_diffuse_tex_binding_else_the_first_srgb_one() {
        let material = |textures: Vec<TextureBinding>| Material {
            textures,
            ..Material::default()
        };
        let missing = "hash_00000000000000AA";
        let mut scene = Scene {
            materials: vec![
                // A normal map bound first, the colour map by its record after it.
                material(vec![
                    binding("texture_slot_0", "normal"),
                    binding(DIFFUSE_TEX, "colour"),
                ]),
                // No record names the colour map: the first sRGB texture is it.
                material(vec![
                    binding("texture_slot_0", "normal"),
                    binding("texture_slot_1", "colour"),
                ]),
                // The record binds a texture nothing holds: no base colour, though one is sRGB.
                material(vec![
                    binding(DIFFUSE_TEX, missing),
                    binding("texture_slot_1", "colour"),
                ]),
                // Only a linear texture: no base colour.
                material(vec![binding("texture_slot_0", "normal")]),
            ],
            ..Scene::default()
        };
        scene.report.textures_not_carried =
            ["colour", missing, "normal"].map(String::from).to_vec();
        let textures = [
            ("normal".to_string(), decodable(false)),
            ("colour".to_string(), decodable(true)),
        ];
        carry_images(&mut scene, &textures);
        assert_eq!(scene.images.len(), 2, "each texture decoded once");
        assert!(scene.images.iter().all(|i| &i.png[1..4] == b"PNG"));
        let colour = scene.images.iter().position(|i| i.name == "colour");
        assert!(colour.is_some());
        let bases: Vec<Option<usize>> = scene.materials.iter().map(|m| m.base_colour).collect();
        assert_eq!(bases, [colour, colour, None, None]);
        assert_eq!(scene.materials[2].textures[0].image, None);
        assert_eq!(scene.materials[2].textures[1].image, colour);
        assert_eq!(scene.report.textures_not_carried, [missing]);
    }

    #[test]
    fn an_embedded_texture_is_found_by_its_key_and_one_whose_pixels_do_not_read_is_named() {
        let pixels = Texture {
            width: 4,
            height: 4,
            format: Format::of_dxgi(71).expect("BC1 is measured"),
            levels: 1,
            faces: 1,
            pixels: vec![0x5A; 8],
        };
        let embedded = |key: u64, texture| EmbeddedTexture {
            key,
            hash: 0,
            texture: DictionaryTexture {
                name: format!("hash_{key:016X}"),
                width: 4,
                height: 4,
                depth: 1,
                dimension: 1,
                format: 71,
                mip_levels: 1,
                pixels_at: 0,
                pixels_len: 8,
                texture,
            },
        };
        let mut drawable = drawable();
        // The read key twice, then one whose pixels do not read, then one nothing holds.
        drawable.shaders[0].texture_keys = vec![Some(0xA1), Some(0xA1), Some(0xB2), Some(0xC3)];
        let unread = Err(HeaderlessError::Positions {
            why: "made up for the test",
        });
        let (found, not_read) = textures_of(
            &drawable,
            &[],
            vec![embedded(0xB2, unread), embedded(0xA1, Ok(pixels.clone()))],
            &hashed,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].0.as_str(), &found[0].1),
            (hashed(0xA1).as_str(), &pixels)
        );
        assert_eq!(not_read, [hashed(0xB2)]);
    }

    #[test]
    fn textures_names_a_pack_or_a_folder_and_needs_one() {
        let args: Vec<String> = ["a.rpf", "--textures", "packs", "0x100"]
            .map(String::from)
            .to_vec();
        let (path, rest) = split_textures(&args).expect("parsed");
        assert_eq!(path, Some(PathBuf::from("packs")));
        assert_eq!(rest, ["a.rpf", "0x100"].map(String::from));
        assert!(split_textures(&["--textures".to_string()]).is_err());
        let dir = tempfile::tempdir().expect("a folder");
        assert!(open_packs(dir.path()).expect("an empty folder").is_empty());
        assert!(open_packs(&dir.path().join("none.ttpf")).is_err());
    }

    #[test]
    fn a_gta_vi_drawable_s_scene_writes_a_binary_gltf() {
        let files = gltf::write(
            &scene(&drawable(), None, "prop_cube", false, &NAMES),
            "prop_cube",
            Container::Glb,
        );
        assert_eq!(&files.primary[..4], b"glTF");
        assert!(files.sidecars.is_empty(), "a .glb carries everything");
    }
}
