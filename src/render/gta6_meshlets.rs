//! The active GTA VI drawable for its own mesh and pixel programs: each geometry of the level
//! Solid draws with its two data blocks as the file stores them, its material's constants and
//! textures, and what the editor fills of the buffers the game fills when it runs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use or_cli::{Gta6Fed as Fed, Gta6Feed as Filled, Gta6ShadingPass, Gta6Stage as Stage};
use or_io::gta5::shaders::spirv_pack::ConstantBuffer;
use or_io::gta6::headerless::{Drawable, Geometry, Model, ParameterValue, Shader};
use or_io::hash::string_hash_str;
use or_renderer::game_shaders::mesh::Image;
use or_renderer::game_shaders::meshlets::{Feed, FrameFeed, MeshletSurface, ROW, VERTICES};

use super::gta6_layers::Material;
use crate::state::gta6_resource::{drawable_of, effect_name};
use crate::state::gta6_shaders::layer_index::{self as gta6_layer_index, Index};
use crate::state::{gta6_shaders, AppState, DocPayload};

/// The words a buffer read by address states before its first element: its element size, then
/// dwords of 32 bits read as unsigned numbers.
const DWORDS: u32 = (4 << 8) | 32;

/// The effects whose own passes were last handed to the device, and those a device was lost
/// under: such an effect draws as Material until the program is started again.
type Under = (Vec<u32>, BTreeSet<u32>);
static UNDER: Mutex<Under> = Mutex::new((Vec::new(), BTreeSet::new()));

fn lose((drawing, lost): &mut Under) {
    lost.extend(drawing.drain(..));
}

/// Keep the effects last handed to the device from one made after it: the device was lost
/// while it drew them.
pub(crate) fn device_lost() {
    lose(&mut UNDER.lock().unwrap_or_else(|p| p.into_inner()));
}

/// The active GTA VI drawable drawn by its own programs.
pub(crate) struct Meshlets {
    pub surfaces: Vec<MeshletSurface>,
    /// Each surface's place among the meshes the model view draws, and its effect's name.
    pub of: Vec<(usize, String)>,
    /// The bindings only the game fills, which the editor leaves at nought.
    pub zeroed: usize,
    /// The geometries its programs do not draw, each by its place among the meshes the model
    /// view draws, and why the first is not: they draw as Material in the same frame.
    pub rest: Vec<usize>,
    pub why: Option<String>,
    /// Whether a worker still decodes a map of its: the surfaces bind the blank texel there.
    pub decoding: bool,
}

fn bytes_of(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// `bytes` after the header a buffer read by address states, where the binding has one.
pub(super) fn headed(header: Option<u32>, bytes: &[u8]) -> Vec<u8> {
    let mut out = match header {
        Some(stride) => bytes_of(&[stride, DWORDS, 0, 0]),
        None => Vec::new(),
    };
    out.extend_from_slice(bytes);
    out
}

/// Whether `name` is the layout's name of the parameter a drawable states by `hash`: `0x` and
/// the hash, or a name that hashes to it.
fn names(name: &str, hash: u32) -> bool {
    name.eq_ignore_ascii_case(&format!("0x{hash:08X}")) || string_hash_str(name) == hash
}

/// The bytes of a material's constant buffer: each constant the shader sets, at the place the
/// layout gives the variable of its hash.
fn material_bytes(layout: &ConstantBuffer, shader: &Shader) -> Vec<u8> {
    let mut bytes = vec![0; layout.size as usize];
    for parameter in &shader.parameters {
        let ParameterValue::Constant { words, .. } = &parameter.value else {
            continue;
        };
        let Some(variable) = (layout.variables.iter()).find(|v| names(&v.name, parameter.hash))
        else {
            continue;
        };
        let value = bytes_of(words);
        let at = variable.offset as usize;
        let end = (at + value.len()).min(bytes.len());
        if at < end {
            bytes[at..end].copy_from_slice(&value[..end - at]);
        }
    }
    bytes
}

/// The heap a pass picks its textures from: number `i` picks entry `i + 1`, and entry nought is
/// a white texel, which every number the editor leaves at nought picks. `texture` finds the image
/// of a texture parameter by its hash.
fn heap(
    pass: &Gta6ShadingPass,
    texture: &mut dyn FnMut(u32) -> Option<Arc<Image>>,
) -> Vec<Option<Arc<Image>>> {
    let numbered = || pass.mesh.textures.iter().chain(&pass.pixel.textures);
    let last = numbered().map(|(number, _)| *number).max();
    let mut images = vec![None; last.map_or(1, |last| last as usize + 2)];
    for (number, hash) in numbered() {
        let entry = &mut images[*number as usize + 1];
        if entry.is_none() {
            *entry = texture(*hash).or_else(|| neutral(*hash));
        }
    }
    images
}

/// The bytes of decoded maps kept between models, past which they are all let go.
const DECODED_BYTES: usize = 1 << 30;

/// A map as far as it is decoded: nothing while a worker decodes it.
type Decoding = Arc<std::sync::Mutex<Option<Option<Arc<Image>>>>>;

/// The first level of the texture of `key`, decoded once a session, on a worker: a frame never
/// waits for a map, and a model opened again, or another that wears the same maps, decodes
/// nothing. `Err` while the worker decodes it; `stored` gives the level as its pack stores it.
// ponytail: past the cap every map is let go at once; let go the least lately used when a
// session that opens many models shows the decode again.
pub(super) fn decoded(
    key: u64,
    stored: impl FnOnce() -> Option<super::gta6_maps::Stored>,
) -> Result<Option<Arc<Image>>, ()> {
    use std::collections::HashMap;
    static DECODED: std::sync::Mutex<Option<HashMap<u64, Decoding>>> = std::sync::Mutex::new(None);
    let mut kept = DECODED.lock().unwrap_or_else(|p| p.into_inner());
    let kept = kept.get_or_insert_with(HashMap::new);
    if let Some(slot) = kept.get(&key) {
        return slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(());
    }
    // A map no pack read so far holds is asked for again: its pack may be read by then.
    let Some(stored) = stored() else {
        return Ok(None);
    };
    let bytes = |slot: &Decoding| match &*slot.lock().unwrap_or_else(|p| p.into_inner()) {
        Some(Some(image)) => image.rgba.len(),
        _ => 0,
    };
    if kept.values().map(bytes).sum::<usize>() > DECODED_BYTES {
        kept.clear();
    }
    let slot = Decoding::default();
    kept.insert(key, Arc::clone(&slot));
    super::gta6_maps::decode(move || {
        let image = stored.decode().map(Arc::new);
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(image);
    });
    Err(())
}

/// Whether no worker still decodes the map of any of `keys`; none is started.
pub(crate) fn all_in(keys: &[u64]) -> bool {
    keys.iter().all(|key| decoded(*key, || None).is_ok())
}

/// The texel a map no pack holds is read as, where a white one is not neutral: a normal map
/// bends no normal (a program takes the square root of what its two channels leave of one), and
/// a specular map states no metal (a program keeps one less its red of the diffuse colour).
// One image of each a session: a scene reads a texture by the image it is, and binds this one once.
fn neutral(hash: u32) -> Option<Arc<Image>> {
    static NEUTRAL: std::sync::OnceLock<[Arc<Image>; 2]> = std::sync::OnceLock::new();
    let texel = |rgba: Vec<u8>| {
        Arc::new(Image {
            width: 1,
            height: 1,
            rgba,
            srgb: false,
        })
    };
    let [bump, specular] =
        NEUTRAL.get_or_init(|| [texel(vec![128, 128, 255, 255]), texel(vec![0, 0, 0, 255])]);
    match hash {
        h if h == string_hash_str("BumpTex") => Some(Arc::clone(bump)),
        h if h == string_hash_str("SpecularTex") => Some(Arc::clone(specular)),
        _ => None,
    }
}

/// The type field of a two-dimensional image's descriptor.
const IMAGE_2D: u32 = 9;

/// The dwords of `heap` as the console states an image: eight an entry, the width less one in
/// the top two bits of the second dword and the low fourteen of the third, the height less one
/// above them, and the type in the top four bits of the fourth; one level. An entry of `rows`
/// no image fills states the row of blank texels bound there: a program that loads a palette's
/// texel at a row times the width counts the rows from the size, and none takes it past the end.
fn heap_words(heap: &[Option<Arc<Image>>], rows: &[u32]) -> Vec<u8> {
    let mut words = Vec::with_capacity(heap.len() * 8);
    for (entry, image) in (0u32..).zip(heap) {
        let blank = match rows.contains(&entry) {
            true => (ROW, 1),
            false => (1, 1),
        };
        let (width, height) = image.as_ref().map_or(blank, |i| (i.width, i.height));
        let (w, h) = (width.max(1) - 1, height.max(1) - 1);
        let size = ((w >> 2) & 0x3FFF) | ((h & 0x3FFF) << 14);
        words.extend([0, (w & 3) << 30, size, IMAGE_2D << 28, 0, 0, 0, 0]);
    }
    bytes_of(&words)
}

/// The heap entries a pass reads as a row of texels: number `i` picks entry `i + 1`.
fn rows_of(pass: &Gta6ShadingPass) -> Vec<u32> {
    let numbers = pass.mesh.rows.iter().chain(&pass.pixel.rows);
    numbers.map(|number| number + 1).collect()
}

/// One record of texture-coordinate transforms that moves nothing: rows 0 to 3 give u back
/// (`1 * u + 0 * v + 0`) and rows 4 to 7 give v.
fn uv_transforms() -> [u32; 24] {
    let mut floats = [0f32; 24];
    for pair in 0..4 {
        floats[3 * pair] = 1.0;
        floats[3 * (4 + pair) + 1] = 1.0;
    }
    floats.map(f32::to_bits)
}

/// The entries of a table whose buffers a stage binds as one buffer of dwords.
const TABLE_ENTRIES: usize = 256;

/// A table's buffers as the one buffer a stage reads them from: the dword each entry's buffer
/// starts at, then the buffers. Every entry starts at the same run of noughts past the starts.
fn empty_table() -> Vec<u32> {
    let mut words = vec![TABLE_ENTRIES as u32; TABLE_ENTRIES];
    words.extend([0; 8]);
    words
}

/// The matrices a buffer of them is filled with: a skeleton has at most as many bones.
const MATRICES: usize = 256;

/// What fills each binding of `stage` for `geometry` drawn with `shader`, its textures picked
/// from `heap`, with `record` as the record its material's numbers head. Also how many
/// bindings are left at nought.
fn feeds(
    stage: &Stage,
    geometry: &Geometry,
    shader: &Shader,
    heap: &[Option<Arc<Image>>],
    rows: &[u32],
    record: Option<&[u32]>,
    out: &mut BTreeMap<(u32, u32), Feed>,
) -> usize {
    let mut zeroed = 0;
    for Fed {
        group,
        binding,
        feed,
        header,
    } in &stage.feeds
    {
        let filled = match feed {
            Filled::VertexData => Feed::Bytes(headed(*header, &geometry.vertex_data)),
            Filled::Meshlets => Feed::Bytes(headed(*header, &geometry.index_data)),
            Filled::View(layout) => Feed::View {
                bytes: vec![0; layout.size as usize],
                view_proj: (layout.variables.iter())
                    .find(|v| v.name == "gViewProj")
                    .map(|v| v.offset as usize),
            },
            Filled::Material(layout) => Feed::Bytes(material_bytes(layout, shader)),
            // Every bone at rest, or the object where it stands, now and the frame before.
            Filled::Placement => {
                let identity = glam::Mat4::IDENTITY.to_cols_array();
                let each: Vec<u8> = identity.iter().flat_map(|f| f.to_le_bytes()).collect();
                Feed::Bytes(headed(*header, &each.repeat(MATRICES)))
            }
            Filled::UvTransforms => Feed::Bytes(headed(
                *header,
                &bytes_of(&uv_transforms()).repeat(MATRICES),
            )),
            Filled::Bones => {
                let rest = [1f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
                let each: Vec<u8> = rest.iter().flat_map(|f| f.to_le_bytes()).collect();
                Feed::Bytes(headed(*header, &each.repeat(MATRICES)))
            }
            Filled::Instance => {
                let mut floats = [0f32; 72];
                for row in 0..3 {
                    floats[4 + 4 * row] = 1.0;
                }
                let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
                Feed::Bytes(headed(*header, &bytes))
            }
            Filled::InstanceRecords => Feed::Bytes(headed(
                *header,
                &super::gta6_instances::records(super::gta6_instances::IN_PLACE[0]),
            )),
            Filled::Numbers => {
                let last = stage.textures.iter().map(|(number, _)| *number).max();
                let mut words = vec![0u32; last.map_or(0, |last| last as usize + 1)];
                for (number, _) in &stage.textures {
                    words[*number as usize] = number + 1;
                }
                Feed::Bytes(headed(*header, &bytes_of(&words)))
            }
            // Entry nought is the blank one: number `i` picks entry `i + 1`.
            Filled::Listed(listed) => {
                let last = listed.iter().map(|(dword, _)| *dword).max();
                let mut words = vec![0u32; last.map_or(0, |last| last as usize + 1)];
                let aid = or_renderer::game_shaders::meshlets::record_aid();
                if let Some(record) = aid.as_deref().or(record) {
                    words.resize(words.len().max(record.len()), 0);
                    words[..record.len()].copy_from_slice(record);
                }
                for (dword, number) in listed {
                    words[*dword as usize] = number + 1;
                }
                Feed::Bytes(headed(*header, &bytes_of(&words)))
            }
            Filled::Heap { .. } => Feed::Textures(heap.to_vec()),
            Filled::Blank | Filled::White => Feed::Textures(vec![None]),
            Filled::Stated(bytes) => Feed::Bytes(headed(*header, bytes)),
            Filled::Frame(frame) => Feed::Frame(match frame {
                or_cli::Gta6Frame::DirectionalLight => FrameFeed::DirectionalLight,
                or_cli::Gta6Frame::AmbientMap => FrameFeed::AmbientMap,
                or_cli::Gta6Frame::ViewLighting => FrameFeed::ViewLighting,
                or_cli::Gta6Frame::EnvironmentFit => FrameFeed::EnvironmentFit,
            }),
            Filled::HeapWords => Feed::Bytes(heap_words(heap, rows)),
            Filled::Texture(number) => {
                Feed::Textures(vec![heap.get(*number as usize + 1).cloned().flatten()])
            }
            // ⚠ A buffer read by address states its element size and its format before its
            // first element, whatever fills it: a stage divides by what the format states, and
            // with both at nought a vehicle's skinned stage left every vertex at the origin.
            Filled::Zero(game) => {
                zeroed += usize::from(game.is_some());
                match header {
                    Some(_) => Feed::Bytes(headed(*header, &[])),
                    None => continue,
                }
            }
            Filled::HeapBuffers => Feed::Bytes(bytes_of(&empty_table())),
            Filled::Sampler => continue,
            Filled::Terrain(part) => match super::gta6_terrain::feed(*part, *header) {
                Some(feed) => feed,
                None => continue,
            },
            // A ped's component: `super::gta6_ped::fill` feeds them from the drawable's join.
            Filled::PedComponent | Filled::PedTexture(_) => continue,
            // Fed by `game_textures` from the install's texture archive.
            Filled::GameTexture(_) => continue,
            // At rest; `super::gta6_ped::fill_skin` feeds a ped's from the loaded skin tones.
            Filled::SkinTones => Feed::Bytes(headed(
                *header,
                &or_cli::gta6_ped_skin::SkinToneRecord::AT_REST.bytes(),
            )),
            Filled::ObjectData => Feed::Bytes(headed(
                *header,
                &or_cli::gta6_ped_layers::object_data_at_rest(),
            )),
        };
        out.insert((*group, *binding), filled);
    }
    zeroed
}

/// Whether `pass` reads a record a material's texture numbers head.
fn reads_a_record(pass: &Gta6ShadingPass) -> bool {
    let mut feeds = pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
    feeds.any(|fed| matches!(fed.feed, Filled::Listed(_)))
}

/// The record of the layered material `worn`, its layers' textures added to `heap`: `tile` finds
/// the image of one by its key, and one it does not find is the blank entry.
// ponytail: the game numbers a default texture where a layer names none, which is not read yet;
// such a layer takes the blank entry.
fn record_of(
    worn: &(Arc<Index>, Material),
    heap: &mut Vec<Option<Arc<Image>>>,
    tile: &mut dyn FnMut(u64) -> Option<Arc<Image>>,
) -> Vec<u32> {
    let mut numbers = BTreeMap::new();
    super::gta6_layers::record(&worn.1, &worn.0.layered, &mut |key| {
        let mut number = || match tile(key) {
            Some(image) => {
                heap.push(Some(image));
                heap.len() as u32 - 1
            }
            None => 0,
        };
        match key {
            0 => 0,
            key => *numbers.entry(key).or_insert_with(&mut number),
        }
    })
}

/// The primitives of the largest of the first `meshlets` meshlets of a geometry's index data:
/// a meshlet's record is sixteen bytes from the twelfth on, and its first dword holds the
/// vertices less one in its low seven bits and the primitives less one in the seven above.
/// `None` where the data ends before a record.
fn largest_meshlet(index_data: &[u8], meshlets: u32) -> Option<u32> {
    let primitives = |meshlet: usize| {
        let record = index_data.get(12 + 16 * meshlet..)?.first_chunk::<4>()?;
        Some(((u32::from_le_bytes(*record) >> 7) & 0x7F) + 1)
    };
    (0..meshlets as usize).try_fold(0, |most, meshlet| Some(most.max(primitives(meshlet)?)))
}

/// The surface of `geometry` drawn by `pass`, and how many of its bindings are left at nought.
/// `worn` is the layered material the drawable wears, and `tile` finds a texture of its layers.
fn surface(
    pass: &Arc<Gta6ShadingPass>,
    geometry: &Geometry,
    shader: &Shader,
    texture: &mut dyn FnMut(u32) -> Option<Arc<Image>>,
    worn: Option<&(Arc<Index>, Material)>,
    tile: &mut dyn FnMut(u64) -> Option<Arc<Image>>,
) -> Option<(MeshletSurface, usize)> {
    // The meshlet count is the low half of the index data's first word.
    let head = geometry.index_data.first_chunk::<4>()?;
    let meshlets = u32::from_le_bytes(*head) & 0xFFFF;
    let mut filled = BTreeMap::new();
    let mut heap = heap(pass, texture);
    // Before a record's textures are pushed: the terrain's defaults are at entries of their own.
    super::gta6_terrain::extend_heap(pass, &mut heap);
    let record = worn.map(|worn| record_of(worn, &mut heap, tile));
    let record = record.as_deref();
    let rows = rows_of(pass);
    let zeroed = feeds(
        &pass.mesh,
        geometry,
        shader,
        &heap,
        &rows,
        record,
        &mut filled,
    ) + feeds(
        &pass.pixel,
        geometry,
        shader,
        &heap,
        &rows,
        record,
        &mut filled,
    );
    let surface = MeshletSurface {
        vertex: Arc::clone(&pass.mesh.module),
        fragment: Arc::clone(&pass.pixel.module),
        feeds: filled,
        meshlets,
        // Data cut short of a record is drawn as a group has threads: nothing of it is lost.
        primitives: largest_meshlet(&geometry.index_data, meshlets).unwrap_or(VERTICES / 3),
        forward: (pass.forward.as_ref()).map(|blend| forward::forward_of(blend, geometry)),
        // ⚠ A terrain cell's own program winds the faces turned to the eye clockwise on the screen.
        two_sided: super::gta6_terrain::reads_a_tile(pass),
    };
    (meshlets > 0).then_some((surface, zeroed))
}

/// Have workers start on every map the active GTA VI drawable's shaders name, before its
/// programs are translated: the two take about as long, and neither waits for the other.
pub(crate) fn start_decoding(state: &AppState) {
    let Some(doc) = state.active_document() else {
        return;
    };
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return;
    };
    let selection = state.selection.selected(doc.id);
    let Some(drawable) = drawable_of(resource, selection) else {
        return;
    };
    let keys = (drawable.shaders.iter()).flat_map(|shader| shader.texture_keys.iter().flatten());
    for key in keys {
        let _ = decoded(*key, || super::gta6_maps::stored(state, selection, *key));
    }
}

/// The active GTA VI drawable, and the models of its level the view draws.
pub(crate) fn drawn_level(state: &AppState) -> Option<(&Drawable, &[Model])> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let drawable = drawable_of(resource, state.selection.selected(doc.id))?;
    let held = |level: usize| {
        (drawable.levels.get(level))
            .and_then(Option::as_ref)
            .filter(|models| !models.is_empty())
    };
    let models = held(state.preview_scene.lod).or_else(|| held(0))?;
    Some((drawable, models.as_slice()))
}

/// Each geometry the model view draws of `models`, in the order it uploads their meshes: those
/// whose positions read.
pub(crate) fn drawn(models: &[Model]) -> impl Iterator<Item = &Geometry> {
    (models.iter())
        .flat_map(|model| model.geometries.iter().flatten())
        .filter(|geometry| geometry.positions.is_ok())
}

/// What the active GTA VI drawable's geometries wear: each one's effect with the streams it
/// holds, which its shading pass is picked by. Empty where the document shows no such drawable.
pub(crate) fn worn(state: &AppState) -> Vec<gta6_shaders::Worn> {
    drawn_level(state).map_or_else(Vec::new, |(drawable, models)| worn_by(drawable, models))
}

/// [`worn`], of the geometries of `models` the view draws of `drawable`.
pub(crate) fn worn_by(drawable: &Drawable, models: &[Model]) -> Vec<gta6_shaders::Worn> {
    let worn = |geometry: &Geometry| {
        let shader = drawable.shaders.get(usize::from(geometry.shader))?;
        let held = or_cli::gta6_held_streams(&geometry.vertex_data);
        Some((shader.name_hash, held))
    };
    drawn(models).filter_map(worn).collect()
}

/// The active GTA VI drawable for its own mesh and pixel programs. `None` where the document
/// shows none; `Err` says why all of it draws as Material: the first geometry whose effect has
/// no pass the renderer can be handed, with the effect's name.
pub(crate) fn meshlets(state: &AppState) -> Option<Result<Meshlets, String>> {
    let (drawable, models) = drawn_level(state)?;
    let selection = state.selection.selected(state.active_document()?.id);
    let stored = |key: u64| super::gta6_maps::stored(state, selection, key);
    let lost = UNDER.lock().unwrap_or_else(|p| p.into_inner()).1.clone();
    let sources = Sources {
        shading: &gta6_shaders::shading,
        map: &|key| decoded(key, || stored(key)),
        layered: &|| gta6_layer_index::worn(state, true),
        lost: &lost,
        ped: &|| crate::state::gta6_shaders::ped_assets::worn(state),
        tile: &|| crate::state::gta6_shaders::terrain_tile::worn(state),
        game_texture: &|name| crate::state::gta6_shaders::global_textures::worn(state, name),
        skin_tones: &|| crate::state::gta6_shaders::ped_assets::skin_tones::worn(state),
    };
    let (mut out, effects) = meshlets_of(drawable, models, &sources)?;
    UNDER.lock().unwrap_or_else(|p| p.into_inner()).0 = effects;
    match (out.surfaces.is_empty(), out.why.take()) {
        (true, Some(why)) => Some(Err(why)),
        (true, None) => None,
        (false, why) => {
            out.why = why;
            Some(Ok(out))
        }
    }
}

/// What the surfaces of a drawable are made from besides the drawable.
pub(crate) struct Sources<'s> {
    /// The pass that draws a geometry of this vertex data with the effect of this hash
    /// ([`gta6_shaders::shading`]).
    pub shading: &'s dyn Fn(u32, &[u8]) -> gta6_shaders::Shading,
    /// The map of a key, decoded; `Err` while a worker decodes it.
    pub map: &'s dyn Fn(u64) -> Result<Option<Arc<Image>>, ()>,
    /// The layered material the drawable wears, asked for by the first pass that reads a record.
    pub layered: &'s dyn Fn() -> Option<(Arc<Index>, Material)>,
    /// The effects a device was lost under.
    pub lost: &'s BTreeSet<u32>,
    /// What the drawable joins to as a ped's part, asked for by the first pass fed of a component.
    pub ped: &'s dyn Fn() -> Option<crate::state::gta6_shaders::ped_assets::Worn>,
    /// The terrain tile the drawable is a cell of, asked for by the first pass fed of a tile's.
    pub tile: &'s dyn Fn() -> Option<Arc<crate::state::gta6_shaders::terrain_tile::Tile>>,
    /// The slices of the game's own texture of a name; `Err` while a worker reads it.
    pub game_texture: &'s dyn Fn(u32) -> Result<Option<GameSlices>, ()>,
    /// The install's skin tones, asked for by the first pass that reads the table of their
    /// records; `Err` while a worker reads them.
    pub skin_tones: &'s dyn Fn() -> Result<Option<Arc<SkinTones>>, ()>,
}

type SkinTones = or_io::gta6::skin_tones::SkinTones;

type GameSlices = crate::state::gta6_shaders::global_textures::Slices;

/// Feed each binding of `pass` the game binds a texture of its own dictionaries to with that
/// texture from `found`, a volume; one still read sets `decoding`, and one no dictionary holds is
/// left at nought.
fn game_textures(
    pass: &Gta6ShadingPass,
    found: &dyn Fn(u32) -> Result<Option<GameSlices>, ()>,
    decoding: &std::cell::Cell<bool>,
    feeds: &mut BTreeMap<(u32, u32), Feed>,
) {
    for fed in pass.mesh.feeds.iter().chain(&pass.pixel.feeds) {
        let Filled::GameTexture(name) = fed.feed else {
            continue;
        };
        match found(name) {
            Ok(Some(volume)) => {
                feeds.insert((fed.group, fed.binding), Feed::Volume(volume));
            }
            Ok(None) => {}
            Err(()) => decoding.set(true),
        }
    }
}

/// The geometries of `models` the view draws of `drawable` as surfaces of their own passes, with
/// `why` the first that is none is not, and the effects they draw with. `None` where a geometry
/// names a shader the drawable does not list.
// ⚠ A geometry's pass is asked for by its own vertex data, which is what [`worn_by`] reads the
// streams it asked a job for from: another geometry's would hand it a pass it cannot draw by.
pub(crate) fn meshlets_of(
    drawable: &Drawable,
    models: &[Model],
    from: &Sources<'_>,
) -> Option<(Meshlets, Vec<u32>)> {
    let mut out = Meshlets {
        surfaces: Vec::new(),
        of: Vec::new(),
        zeroed: 0,
        rest: Vec::new(),
        why: None,
        decoding: false,
    };
    let decoding = std::cell::Cell::new(false);
    // The layered material the drawable wears, asked for by the first pass that reads a record.
    let mut worn: Option<Option<(Arc<Index>, Material)>> = None;
    let mut ped = None;
    let mut terrain_tile = None;
    let mut skin_tones: Option<Option<Arc<SkinTones>>> = None;
    let map = |key: u64| {
        (from.map)(key).unwrap_or_else(|()| {
            decoding.set(true);
            None
        })
    };
    let mut tile = map;
    let mut effects = Vec::new();
    for (mesh, geometry) in drawn(models).enumerate() {
        let shader = drawable.shaders.get(usize::from(geometry.shader))?;
        let effect = effect_name(shader.name_hash);
        let pass = match (from.shading)(shader.name_hash, &geometry.vertex_data) {
            _ if from.lost.contains(&shader.name_hash) => Err(tr!(
                "{effect}: the graphics device was lost while its own passes drew, so it draws as Material until OpenRage is started again",
                effect = effect,
            )),
            Some(Ok(pass)) => Ok(pass),
            Some(Err(why)) => Err(format!("{effect}: {why}")),
            None => Err(tr!(
                "{effect}: its programs are not translated yet",
                effect = effect,
            )),
        };
        let pass = &match pass {
            Ok(pass) => pass,
            Err(why) => {
                out.rest.push(mesh);
                out.why.get_or_insert(why);
                continue;
            }
        };
        let mut texture = |hash: u32| {
            let slot = (shader.parameters.iter()).find_map(|p| match p.value {
                ParameterValue::Texture { slot } if p.hash == hash => Some(slot),
                _ => None,
            })?;
            map((*shader.texture_keys.get(usize::from(slot))?)?)
        };
        let worn = match reads_a_record(pass) {
            true => worn.get_or_insert_with(|| (from.layered)()).as_ref(),
            false => None,
        };
        let mut drawn = surface(pass, geometry, shader, &mut texture, worn, &mut tile);
        if let Some((surface, _)) = drawn.as_mut() {
            game_textures(pass, from.game_texture, &decoding, &mut surface.feeds);
            let stages = || pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
            if stages().any(|fed| fed.feed == Filled::SkinTones) {
                let tones = skin_tones.get_or_insert_with(|| {
                    (from.skin_tones)().unwrap_or_else(|()| {
                        decoding.set(true);
                        None
                    })
                });
                super::gta6_ped::fill_skin(pass, tones.as_deref(), &mut surface.feeds);
            }
            let joined = super::gta6_ped::joined_for(pass, &mut ped, from.ped, &decoding);
            let mut ped_map = map;
            let fed = super::gta6_ped::fill(pass, joined, &mut ped_map, &mut surface.feeds);
            if let Err(why) = fed {
                out.rest.push(mesh);
                out.why.get_or_insert(format!("{effect}: {why}"));
                continue;
            }
            if super::gta6_terrain::reads_a_tile(pass) {
                let tile = terrain_tile.get_or_insert_with(|| (from.tile)()).clone();
                let mut tile_map = map;
                let feeds = &mut surface.feeds;
                let fed = super::gta6_terrain::fill(pass, tile.as_deref(), &mut tile_map, feeds);
                if let Err(why) = fed {
                    out.rest.push(mesh);
                    out.why.get_or_insert(format!("{effect}: {why}"));
                    continue;
                }
            }
        }
        if let Some((surface, zeroed)) = drawn {
            let instances = super::gta6_instances::IN_PLACE;
            for surface in super::gta6_instances::placed(pass, surface, &instances) {
                out.surfaces.push(surface);
                out.of.push((mesh, effect.clone()));
            }
            out.zeroed += zeroed;
            effects.push(shader.name_hash);
        }
        // A forward technique's later passes draw the geometry again after it, in their order.
        let mut later = pass.then.clone();
        while let Some(next) = later {
            if let Some((surface, _)) =
                surface(&next, geometry, shader, &mut texture, None, &mut tile)
            {
                out.surfaces.push(surface);
                out.of.push((mesh, effect.clone()));
            }
            later = next.then.clone();
        }
    }
    out.decoding = decoding.get();
    Some((out, effects))
}

#[path = "gta6_forward.rs"]
mod forward;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_effects_a_device_was_lost_under_are_kept_and_no_other() {
        let mut under: Under = (vec![7, 9], BTreeSet::from([3]));
        lose(&mut under);
        assert_eq!(under, (Vec::new(), BTreeSet::from([3, 7, 9])));
        // A device lost with nothing of a drawable's own on it blames no effect.
        lose(&mut under);
        assert_eq!(under.1.len(), 3);
    }
    use or_io::gta5::shaders::spirv_pack::Variable;
    use or_io::gta6::headerless::Parameter;

    fn variable(name: &str, offset: u32) -> Variable {
        Variable {
            name: name.to_owned(),
            offset,
            size: 16,
            used: true,
            scalar: "float".to_owned(),
            rows: 1,
            columns: 4,
            elements: 0,
        }
    }

    #[test]
    fn a_constant_lands_where_the_layout_puts_the_variable_of_its_hash() {
        let named = string_hash_str("gSomething");
        let layout = ConstantBuffer {
            name: "material".to_owned(),
            size: 32,
            variables: vec![variable("0x0000ABCD", 16), variable("gSomething", 0)],
        };
        let constant = |hash: u32, words: Vec<u32>| Parameter {
            hash,
            value: ParameterValue::Constant {
                buffer: 0,
                offset: 0,
                words,
            },
        };
        let shader = Shader {
            name_hash: 1,
            texture_keys: Vec::new(),
            parameters: vec![
                constant(0xABCD, vec![1, 2, 3, 4, 5]),
                constant(named, vec![7]),
                // No variable of the layout is of this hash.
                constant(0x1234, vec![9]),
            ],
        };
        let bytes = material_bytes(&layout, &shader);
        // The fifth word of the first constant lies past the buffer and is left out.
        assert_eq!(bytes, bytes_of(&[7, 0, 0, 0, 1, 2, 3, 4]));
        assert_eq!(
            headed(Some(64), &[1, 2]),
            [bytes_of(&[64, DWORDS, 0, 0]), vec![1, 2]].concat()
        );
        assert_eq!(headed(None, &[1, 2]), [1, 2]);
    }

    fn pass(streams: &[u32]) -> Arc<Gta6ShadingPass> {
        let stage = |streams: &[u32]| Stage {
            approximations: Default::default(),
            module: Arc::default(),
            feeds: Vec::new(),
            textures: Vec::new(),
            streams: streams.iter().copied().collect(),
            rows: Vec::new(),
        };
        Arc::new(Gta6ShadingPass {
            mesh: stage(streams),
            pixel: stage(&[]),
            linked: Default::default(),
            forward: None,
            then: None,
        })
    }

    #[test]
    fn a_geometry_is_drawn_by_the_first_pass_that_reads_the_most_of_the_streams_it_has() {
        // The table of streams at the head of the vertex data: slots 0 to 3, 6 and 8 hold one.
        let geometry = |held: &[usize]| {
            let mut vertex_data = vec![0u8; 4 * 23];
            for slot in held {
                vertex_data[4 * slot] = 1;
            }
            Geometry {
                shader: 0,
                vertex_count: 0,
                channels: Vec::new(),
                triangles: Vec::new(),
                positions: Ok(Vec::new()),
                vertices: Default::default(),
                vertex_data,
                index_data: Vec::new(),
            }
        };
        let passes = [
            pass(&[0, 1, 2, 3]),
            pass(&[0, 1, 2, 3]),
            pass(&[0, 1, 2, 3, 6, 8]),
            pass(&[0, 1, 2, 3, 6, 8]),
        ];
        let chosen = |held: &[usize]| {
            let picked = or_cli::gta6_drawing_pass(&passes, &geometry(held).vertex_data)?;
            passes.iter().position(|pass| Arc::ptr_eq(pass, picked))
        };
        assert_eq!(chosen(&[0, 1, 2, 3, 6, 8]), Some(2), "a skinned geometry");
        assert_eq!(
            chosen(&[0, 1, 2, 3]),
            Some(0),
            "an unskinned one, by the first"
        );
        assert_eq!(
            chosen(&[0, 1, 2, 3, 6]),
            Some(0),
            "one stream of the two is not skin"
        );
        assert_eq!(
            chosen(&[0, 1, 2]),
            Some(0),
            "a stream every pass reads is missing: the pass that misses the fewest"
        );
        assert_eq!(
            chosen(&[0, 1, 2, 6, 8]),
            Some(2),
            "each kind misses one stream: the one that reads more"
        );
        let none = or_cli::gta6_drawing_pass(&passes[..0], &geometry(&[0]).vertex_data);
        assert!(none.is_none(), "no pass");
    }

    #[test]
    fn a_surface_states_the_primitives_of_the_largest_meshlet_its_index_data_records() {
        // The head (bit 31 and the count), two words, then four words a meshlet: the first
        // holds the vertices less one, the primitives less one and a radius above them.
        let index_data = |primitives: &[u32], cut: usize| {
            let mut words = vec![0x8000_0000 | primitives.len() as u32, 0, 0];
            for (meshlet, primitives) in primitives.iter().enumerate() {
                let vertices = [0x7F, 0, 0x55][meshlet % 3];
                words.extend([vertices | (primitives - 1) << 7 | 0xFFFF << 16, !0, !0, !0]);
            }
            let mut bytes = bytes_of(&words);
            bytes.truncate(bytes.len() - cut);
            bytes
        };
        let stated = |primitives: &[u32], cut: usize| {
            let geometry = Geometry {
                shader: 0,
                vertex_count: 0,
                channels: Vec::new(),
                triangles: Vec::new(),
                positions: Ok(Vec::new()),
                vertices: Default::default(),
                vertex_data: Vec::new(),
                index_data: index_data(primitives, cut),
            };
            let shader = Shader {
                name_hash: 1,
                texture_keys: Vec::new(),
                parameters: Vec::new(),
            };
            let (surface, _) = surface(
                &pass(&[]),
                &geometry,
                &shader,
                &mut |_| None,
                None,
                &mut |_| None,
            )?;
            Some((surface.meshlets, surface.primitives))
        };
        assert_eq!(stated(&[5, 77, 12], 0), Some((3, 77)));
        assert_eq!(stated(&[1], 0), Some((1, 1)));
        assert_eq!(stated(&[3, 128], 0), Some((2, 128)));
        assert_eq!(stated(&[26, 2, 9, 4], 0), Some((4, 26)));
        // Cut inside its last record's first dword, it is drawn as a group has threads.
        assert_eq!(stated(&[5, 77], 13), Some((2, VERTICES / 3)));
        assert_eq!(stated(&[], 0), None, "no meshlet, no surface");
    }

    #[test]
    fn a_surface_of_a_terrain_cell_draws_both_faces_and_another_does_not() {
        let geometry = Geometry {
            shader: 0,
            vertex_count: 0,
            channels: Vec::new(),
            triangles: Vec::new(),
            positions: Ok(Vec::new()),
            vertices: Default::default(),
            vertex_data: Vec::new(),
            index_data: bytes_of(&[1, 0, 0, 0, 0, 0, 0]),
        };
        let shader = Shader {
            name_hash: 1,
            texture_keys: Vec::new(),
            parameters: Vec::new(),
        };
        let two_sided = |pass: &Arc<Gta6ShadingPass>| {
            let none = &mut |_: u64| None;
            let drawn = surface(pass, &geometry, &shader, &mut |_| None, None, none);
            drawn.map(|(surface, _)| surface.two_sided)
        };
        let mut terrain = (*pass(&[])).clone();
        terrain.pixel.feeds.push(Fed {
            group: 1,
            binding: 0,
            feed: Filled::Terrain(or_cli::gta6_terrain::Terrain::Record),
            header: Some(372),
        });
        assert_eq!(two_sided(&Arc::new(terrain)), Some(true));
        assert_eq!(two_sided(&pass(&[])), Some(false));
    }

    #[test]
    fn a_buffer_read_by_address_states_its_header_where_its_contents_are_left_at_nought() {
        let fed = |binding: u32, feed: Filled, header: Option<u32>| Fed {
            group: 0,
            binding,
            feed,
            header,
        };
        let stage = Stage {
            feeds: vec![
                fed(0, Filled::Zero(Some(7)), Some(48)),
                fed(1, Filled::Zero(Some(8)), None),
                fed(2, Filled::Zero(None), Some(4)),
            ],
            ..pass(&[]).mesh.clone()
        };
        let geometry = Geometry {
            shader: 0,
            vertex_count: 0,
            channels: Vec::new(),
            triangles: Vec::new(),
            positions: Ok(Vec::new()),
            vertices: Default::default(),
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        };
        let shader = Shader {
            name_hash: 1,
            texture_keys: Vec::new(),
            parameters: Vec::new(),
        };
        let mut out = BTreeMap::new();
        let zeroed = feeds(&stage, &geometry, &shader, &[], &[], None, &mut out);
        assert_eq!(zeroed, 2, "the two only the game fills");
        let bytes = |binding: u32| match out.get(&(0, binding)) {
            Some(Feed::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        };
        // Read by address: the element size and the format, and nothing after them.
        assert_eq!(bytes(0), Some(bytes_of(&[48, DWORDS, 0, 0])));
        assert_eq!(bytes(2), Some(bytes_of(&[4, DWORDS, 0, 0])));
        // A table's record has no header: nothing is bound, which reads as noughts.
        assert_eq!(bytes(1), None);
    }

    #[test]
    fn a_record_of_texture_coordinate_transforms_moves_nothing() {
        let record = uv_transforms().map(f32::from_bits);
        let (u, v) = (0.25f32, 0.75f32);
        for pair in 0..4 {
            let row = |at: usize| record[3 * at] * u + record[3 * at + 1] * v + record[3 * at + 2];
            assert_eq!((row(pair), row(4 + pair)), (u, v));
        }
    }

    #[test]
    fn the_heap_states_each_image_s_size_where_a_program_reads_it() {
        let image = Image {
            width: 1024,
            height: 6,
            rgba: Vec::new(),
            srgb: false,
        };
        let words = heap_words(&[None, Some(Arc::new(image)), None], &[2]);
        let word = |i: usize| u32::from_le_bytes(words[4 * i..4 * i + 4].try_into().unwrap());
        // As a program reads them back: the low two bits of the width less one, then the rest.
        let size = |entry: usize| {
            let (low, high) = (word(8 * entry + 1) >> 30, word(8 * entry + 2));
            (
                (low | ((high & 0x3FFF) << 2)) + 1,
                ((high >> 14) & 0x3FFF) + 1,
            )
        };
        assert_eq!(words.len(), 3 * 8 * 4);
        assert_eq!(
            size(2),
            (ROW, 1),
            "a palette no image fills is one blank row"
        );
        assert_eq!(size(0), (1, 1), "an entry with no image is one white texel");
        assert_eq!(size(1), (1024, 6));
        assert_eq!(word(8 + 3) >> 28, IMAGE_2D);
    }

    #[test]
    fn a_terrain_stage_is_given_the_default_record_its_texel_and_no_feedback() {
        use or_cli::gta6_terrain::{DefaultTexture, Terrain, TileRecord};
        let fed = |binding: u32, part: Terrain, header: Option<u32>| Fed {
            group: 1,
            binding,
            feed: Filled::Terrain(part),
            header,
        };
        let stage = Stage {
            feeds: vec![
                fed(0, Terrain::Record, Some(372)),
                fed(1, Terrain::Texture(DefaultTexture::Flat), None),
                fed(2, Terrain::Feedback, None),
            ],
            ..pass(&[]).mesh.clone()
        };
        let geometry = Geometry {
            shader: 0,
            vertex_count: 0,
            channels: Vec::new(),
            triangles: Vec::new(),
            positions: Ok(Vec::new()),
            vertices: Default::default(),
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        };
        let shader = Shader {
            name_hash: 1,
            texture_keys: Vec::new(),
            parameters: Vec::new(),
        };
        let mut out = BTreeMap::new();
        assert_eq!(
            feeds(&stage, &geometry, &shader, &[], &[], None, &mut out),
            0
        );
        let Some(Feed::Bytes(record)) = out.get(&(1, 0)) else {
            panic!("the record is not bound");
        };
        assert_eq!(record.len(), 16 + std::mem::size_of::<TileRecord>());
        let at = 16 + std::mem::offset_of!(TileRecord, weights);
        let weights = u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        assert_eq!(weights, DefaultTexture::Clear.entry());
        let Some(Feed::Textures(texel)) = out.get(&(1, 1)) else {
            panic!("the texture is not bound");
        };
        let rgba = texel[0].as_ref().map(|image| image.rgba.clone());
        assert_eq!(rgba, Some(DefaultTexture::Flat.texel().to_vec()));
        assert!(!out.contains_key(&(1, 2)), "the feedback binds nothing");
    }

    #[test]
    fn a_texture_the_game_binds_by_name_is_fed_its_slices_once_read() {
        const NAME: u32 = 0xBC20_EC5C;
        let mut pass = Arc::unwrap_or_clone(pass(&[]));
        pass.pixel.feeds = vec![Fed {
            group: 1,
            binding: 30,
            feed: Filled::GameTexture(NAME),
            header: None,
        }];
        let slices: GameSlices = Arc::new(or_renderer::game_shaders::meshlets::Volume {
            width: 1,
            height: 1,
            depth: 2,
            half: false,
            texels: vec![10, 0, 0, 255, 20, 0, 0, 255],
        });
        let fill = |found: &dyn Fn(u32) -> Result<Option<GameSlices>, ()>| {
            let (decoding, mut feeds) = (std::cell::Cell::new(false), BTreeMap::new());
            game_textures(&pass, found, &decoding, &mut feeds);
            (decoding.get(), feeds.remove(&(1, 30)))
        };
        let (decoding, fed) = fill(&|name| Ok((name == NAME).then(|| Arc::clone(&slices))));
        let Some(Feed::Volume(volume)) = fed else {
            panic!("the texture is not fed as a volume");
        };
        assert_eq!((decoding, volume.depth, volume.texels[4]), (false, 2, 20));
        // While it is read the frame is told so, and where no dictionary holds it, nothing binds.
        assert!(matches!(fill(&|_| Err(())), (true, None)));
        assert!(matches!(fill(&|_| Ok(None)), (false, None)));
    }
}
