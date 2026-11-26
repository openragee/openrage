//! Dictionaries and images the suites build, so a check does not depend on a staged corpus.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use or_io::authoring::{encode_texture_with_options, TextureBuildOptions};
use or_io::codec::texture::{GrcTextureFormat, RgbaImage, TextureSource};
use or_io::decl::{File, Open, OpenEnum};
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::gen9::{
    GrcTextureG9, PixelFormat, ShaderResourceViewG9, TextureDimension, TileMode,
};
use or_io::gta5::texture_dict::gen8::PROCESSED;
use or_io::gta5::PgDictionary;
use or_source::textures::{self, Extraction};

/// Four flat quadrants, top left first.
pub fn flat(width: u32, height: u32, colours: [[u8; 4]; 4]) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let quadrant = usize::from(x >= width / 2) + 2 * usize::from(y >= height / 2);
            image.set_pixel(x, y, colours[quadrant]);
        }
    }
    image
}

/// Red, green, blue and a translucent grey: a swapped or lost channel shows.
pub fn quadrants(width: u32, height: u32) -> RgbaImage {
    flat(
        width,
        height,
        [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [128, 128, 128, 64],
        ],
    )
}

/// A smooth ramp that differs from every other `salt`'s, in all four channels.
pub fn ramp(width: u32, height: u32, salt: u32) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let channel =
                |k: u32| ((x * (3 + k + salt) + y * (5 + k * salt) + salt * 40 * k) % 256) as u8;
            image.set_pixel(
                x,
                y,
                [channel(0), channel(1), channel(2), 255 - channel(3) / 2],
            );
        }
    }
    image
}

/// The worst difference in each of R, G, B and A between two same-sized images.
pub fn worst(a: &RgbaImage, b: &RgbaImage) -> [u8; 4] {
    assert_eq!((a.width, a.height), (b.width, b.height), "sizes differ");
    let mut worst = [0u8; 4];
    for (pa, pb) in a
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data.as_chunks::<4>().0.iter())
    {
        for c in 0..4 {
            worst[c] = worst[c].max(pa[c].abs_diff(pb[c]));
        }
    }
    worst
}

/// What a test dictionary's texture is made of.
pub struct Spec {
    pub name: &'static str,
    pub format: GrcTextureFormat,
    pub image: RgbaImage,
    pub srgb: bool,
    pub template_type: u32,
}

impl Spec {
    pub fn new(name: &'static str, format: GrcTextureFormat, image: RgbaImage) -> Self {
        Self {
            name,
            format,
            image,
            srgb: false,
            template_type: 0x2000_0054,
        }
    }
}

fn build_options(srgb: bool) -> TextureBuildOptions {
    TextureBuildOptions {
        srgb,
        ..TextureBuildOptions::default()
    }
}

/// A Legacy dictionary whose textures carry header words no default would give them.
pub fn legacy_dictionary(specs: &[Spec]) -> Vec<u8> {
    let mut dictionary = PgDictionary::<GrcTexture>::default();
    for (i, spec) in specs.iter().enumerate() {
        let mut texture = GrcTexture {
            name: spec.name.to_string(),
            resource_type_and_conversion_flags: PROCESSED,
            extra_flags: 0x10 + i as u32,
            handle_index: 7 + i as u32,
            stereo_rt_mode: 2,
            ..Default::default()
        };
        texture
            .set_pixels(spec.format, &spec.image, &build_options(spec.srgb))
            .unwrap();
        texture.set_template_type(spec.template_type);
        dictionary.insert(texture);
    }
    dictionary.to_bytes().unwrap()
}

/// An Enhanced dictionary of linear 2D textures.
pub fn enhanced_dictionary(specs: &[Spec]) -> Vec<u8> {
    let mut built: Vec<(u32, GrcTextureG9)> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let encoded = encode_texture_with_options(
            spec.format,
            spec.image.width,
            spec.image.height,
            &spec.image,
            &build_options(spec.srgb),
        )
        .unwrap();
        let dxgi = spec.format.to_dxgi(spec.srgb) as u32;
        let mut texture = GrcTextureG9 {
            name: spec.name.to_string(),
            width: spec.image.width as u16,
            height: spec.image.height as u16,
            depth: 1,
            dimension: Open::new(TextureDimension::Texture2D),
            format: Open::new(<PixelFormat as OpenEnum>::from_raw(dxgi as u8)),
            tile_mode: Open::new(TileMode::Auto),
            mip_levels: encoded.mip_count,
            pixels: encoded.data,
            flags: 0x26_0208,
            usage_data: 0x80_0254 + i as u32,
            view: ShaderResourceViewG9 {
                view_dimension: 0x41,
                ..Default::default()
            },
            ..Default::default()
        };
        texture.set_block_stride(spec.format.bytes_per_block());
        built.push((or_io::string_hash_str(spec.name), texture));
    }
    built.sort_by_key(|(code, _)| *code);
    PgDictionary {
        codes: built.iter().map(|(code, _)| *code).collect(),
        entries: built.into_iter().map(|(_, t)| Some(Box::new(t))).collect(),
        ..Default::default()
    }
    .to_bytes()
    .unwrap()
}

pub fn legacy(bytes: &[u8]) -> PgDictionary<GrcTexture> {
    PgDictionary::<GrcTexture>::from_bytes(bytes).unwrap()
}

pub fn enhanced(bytes: &[u8]) -> PgDictionary<GrcTextureG9> {
    PgDictionary::<GrcTextureG9>::from_bytes(bytes).unwrap()
}

/// Each declared mip's bytes, largest first, and none of the padding after them.
pub fn mips<T: TextureSource>(texture: &T) -> Vec<Vec<u8>> {
    let format = texture.pixel_format();
    let (width, height) = texture.stored_size();
    (0..texture.stored_levels())
        .map(|level| texture.payload()[format.mip_level_range(width, height, level)].to_vec())
        .collect()
}

/// The `.textures.src` folder `ytd` extracts to inside `parent`.
pub fn extract(parent: &Path, name: &str, ytd: &[u8]) -> PathBuf {
    textures::extract(
        ytd,
        name,
        parent,
        &Extraction {
            from_game: false,
            virtual_path: None,
        },
    )
    .unwrap_or_else(|problems| panic!("extract: {problems:#?}"))
}

/// What the folder at `dir` compiles to.
pub fn compile(dir: &Path) -> Vec<u8> {
    textures::compile(&or_source::asset::Source::open(dir).unwrap())
        .unwrap_or_else(|problems| panic!("compile: {problems:#?}"))
        .bytes
}

/// The problems a folder's compile refuses with.
pub fn refusal(dir: &Path) -> Vec<or_source::Problem> {
    match textures::compile(&or_source::asset::Source::open(dir).unwrap()) {
        Ok(_) => panic!("{} compiled and was expected to be refused", dir.display()),
        Err(problems) => problems,
    }
}

pub fn save_image(path: &Path, image: &RgbaImage) {
    or_io::codec::texture::save_png(path, image).unwrap();
}

/// The entry of `name` in a parsed `textures.json`.
pub fn entry<'a>(rules: &'a mut serde_json::Value, name: &str) -> &'a mut serde_json::Value {
    rules["textures"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no entry {name}"))
}

/// Rewrites the folder's `textures.json` after `edit` has changed it.
pub fn edit_rules(folder: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = folder.join("textures.json");
    let mut rules: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut rules);
    std::fs::write(path, serde_json::to_string_pretty(&rules).unwrap()).unwrap();
}

/// Mip `level` of the texture `name` in the Legacy dictionary `bytes`, decoded.
pub fn level(bytes: &[u8], name: &str, level: u32) -> RgbaImage {
    let dictionary = legacy(bytes);
    let texture = dictionary.get(name).unwrap_or_else(|| panic!("no {name}"));
    or_io::codec::texture::decode_level(texture, level).unwrap()
}

/// `bytes` as the game ships a file: the same payload under another compression, so a cook, which
/// writes OpenRage's own, makes other bytes of it, as it does of every file in the corpus.
pub fn as_the_game_ships_it(bytes: &[u8]) -> Vec<u8> {
    use or_io::container::resource::{Compression, Rsc7};
    use or_io::container::ResourceEnvelope;
    let opened = Rsc7::open(bytes).expect("an RSC7 file");
    let shipped =
        Rsc7::reseal(&opened.data, &opened.seal, Compression::none()).expect("it reseals");
    assert_ne!(shipped, bytes, "the stand-in is the canonical form");
    shipped
}

/// What a person does to a source in an editor, so that its cook is no longer the import's: a
/// texture gets an image of its own beside its DDS, a model gets one vertex moved in its glb.
/// False when the source holds nothing to edit, such as a drawable with no geometry.
pub fn edit_the_source(folder: &Path) -> bool {
    match or_source::asset::Source::open(folder)
        .unwrap()
        .manifest
        .kind
        .as_str()
    {
        textures::KIND => repaint_a_texture(folder),
        _ => move_a_vertex(folder),
    }
}

fn repaint_a_texture(folder: &Path) -> bool {
    let Some(dds) = std::fs::read_dir(folder.join(textures::DIR))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "dds"))
        .min()
    else {
        return false;
    };
    let head = std::fs::read(&dds).unwrap();
    let word = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
    // A DDS header states its height and then its width after the magic, the size and the flags.
    let image = flat(word(16), word(12), [[200, 30, 90, 255]; 4]);
    save_image(&dds.with_extension("png"), &image);
    true
}

fn move_a_vertex(folder: &Path) -> bool {
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "glb") && nudge_a_glb(&path) {
                return true;
            }
        }
    }
    false
}

/// Moves the first vertex of the first mesh of the glb at `path` by a quarter of a unit along x.
fn nudge_a_glb(path: &Path) -> bool {
    let mut bytes = std::fs::read(path).unwrap();
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    let Some(accessor) = json["meshes"][0]["primitives"][0]["attributes"]["POSITION"].as_u64()
    else {
        return false;
    };
    let accessor = &json["accessors"][accessor as usize];
    let view = &json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
    let offset = |value: &serde_json::Value| value["byteOffset"].as_u64().unwrap_or(0) as usize;
    // The binary chunk follows the JSON chunk and its own eight-byte header.
    let at = 20 + json_length + 8 + offset(view) + offset(accessor);
    let x = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) + 0.25;
    bytes[at..at + 4].copy_from_slice(&x.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
    true
}

/// Reads the glb of the bounds source `name` as a tool would, lets `edit` change it, and writes it under the names it had.
pub fn edit_bounds_glb(
    dir: &Path,
    name: &str,
    edit: impl FnOnce(&mut or_interchange::scene::Scene),
) {
    let path = dir.join(or_source::bounds::MODEL);
    let mut scene =
        or_interchange::gltf::read::load(&path, &std::fs::read(&path).unwrap()).unwrap();
    let mut materials: Vec<or_interchange::scene::Material> = Vec::new();
    scene.name = name.to_string();
    for object in &mut scene.objects {
        object.name = name.to_string();
        for mesh in &mut object.meshes {
            mesh.name = mesh
                .name
                .strip_prefix(&format!("{name}_"))
                .expect("a node is named after the asset")
                .to_string();
            for primitive in &mut mesh.primitives {
                primitive.material = primitive.material_name.as_ref().map(|name| {
                    materials
                        .iter()
                        .position(|m| &m.name == name)
                        .unwrap_or_else(|| {
                            materials.push(or_interchange::scene::Material {
                                name: name.clone(),
                                ..Default::default()
                            });
                            materials.len() - 1
                        })
                });
            }
        }
    }
    scene.materials = materials;
    edit(&mut scene);
    std::fs::write(
        path,
        or_interchange::gltf::write(&scene, name, or_interchange::gltf::Container::Glb).primary,
    )
    .unwrap();
}

/// The JSON pointer to every value of `doc` that is a number, a string or a flag.
pub fn stated_values(doc: &serde_json::Value) -> Vec<String> {
    fn walk(v: &serde_json::Value, at: &str, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(o) => {
                for (name, v) in o {
                    walk(v, &format!("{at}/{name}"), out);
                }
            }
            serde_json::Value::Array(list) => {
                for (i, v) in list.iter().enumerate() {
                    walk(v, &format!("{at}/{i}"), out);
                }
            }
            serde_json::Value::Null => {}
            _ => out.push(at.to_string()),
        }
    }
    let mut out = Vec::new();
    walk(doc, "", &mut out);
    out
}

/// Other values a person could state where `v` is: one for each number a string of several holds.
pub fn other_values(v: &serde_json::Value) -> Vec<serde_json::Value> {
    use serde_json::Value;
    let other_word = |text: &str| -> Option<String> {
        let word = u64::from_str_radix(text.strip_prefix("0x")?, 16).ok()? ^ 1;
        Some(format!("{word:#0width$x}", width = text.len()))
    };
    let other_lane = |text: &str| -> Option<String> {
        other_word(text).or_else(|| Some(format!("{:?}", text.parse::<f32>().ok()? + 1.0)))
    };
    match v {
        Value::Bool(flag) => vec![(!flag).into()],
        Value::Number(n) => match n.as_i64() {
            Some(whole) => vec![(whole + 1).into()],
            None => vec![(n.as_f64().expect("a number") + 1.0).into()],
        },
        Value::String(text) => {
            let address = text.split_once(':').and_then(|(region, index)| {
                Some((region.parse::<u16>().ok()?, index.parse::<u16>().ok()?))
            });
            let lanes: Vec<&str> = text.split(' ').collect();
            if let Some((region, index)) = address {
                vec![format!("{region}:{}", index + 1).into()]
            } else if lanes.iter().all(|lane| other_lane(lane).is_some()) {
                (0..lanes.len())
                    .map(|changed| {
                        let stated: Vec<String> = lanes
                            .iter()
                            .enumerate()
                            .map(|(i, lane)| match i == changed {
                                true => other_lane(lane).expect("a number"),
                                false => lane.to_string(),
                            })
                            .collect();
                        stated.join(" ").into()
                    })
                    .collect()
            } else {
                vec![format!("{text}x").into()]
            }
        }
        _ => Vec::new(),
    }
}
