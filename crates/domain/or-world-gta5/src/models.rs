//! What draws an archetype of GTA V, as the contract's model: asked for by the name of its
//! archetype, found through the index's row of it in a drawable that is a file of its own, an
//! entry of a drawable dictionary or a fragment, and read by the layout its file's own RSC7
//! version names.
//!
//! ⚠ A model that does not read has a reason, which [`Gta5World::read_models`] states and the
//! contract's reader, which hands back models alone, logs and drops.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::WorldSource;
use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexSource};
use or_io::decl::{File, Gen8, Gen9};
use or_io::gta5::drawable::DrawableTarget;
use or_io::gta5::gen8::{EntryData, GrcInstanceData, GrcTexture};
use or_io::gta5::gen9::{GrcInstanceDataG9, GrcTextureG9};
use or_io::gta5::retarget::layout_of;
use or_io::gta5::shaders::shader_defs::hash_name;
use or_io::gta5::{
    GrmGeometry, GrmShaderGroup, GtaDrawable, GtaFragType, PgDictionary, RmcDrawable, TextureSlot,
};
use or_io::Generation;
use or_world::contract::{ModelReader, Worker};
use or_world::model::{
    Alpha, AssetKey, Element, ElementFormat, Index, Lod, Mesh, Model, Semantic, Surface, Texture,
    TextureAsk, VertexLayout,
};

use crate::index::{linked, Holding, Linked};
use crate::install::Gta5World;
use crate::textures::{NoTexture, EMBEDDED};

/// What draws an instance that stands as a box: a name above every archetype's, which is 32 bits.
pub const BOX: AssetKey = AssetKey(1 << 32);

/// `grmPrimitiveType`'s triangle list.
const TRIANGLES: u8 = 3;
/// The draw buckets a shader states that blend over what is behind them: alpha, decal, water and displacement alpha.
const BLENDED: [u8; 4] = [1, 2, 6, 7];
/// The draw bucket of a shader that draws where its alpha passes a test.
const CUTOUT: u8 = 3;
/// The alpha a cut-out surface of the plain pass draws above: the game's own threshold is its programs'.
const CUTOUT_ABOVE: f32 = 0.5;

/// Why an archetype's model does not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoModel {
    #[error("the ask names no archetype of GTA V")]
    NotAnArchetype,
    #[error("no type file of the mount declares the archetype")]
    Undeclared,
    #[error("the archetype names no asset")]
    NoAsset,
    #[error("no drawable, fragment or drawable dictionary of the mount holds the asset")]
    NoFile,
    #[error("the archive that lists the file gave none")]
    Absent,
    #[error("the file's RSC7 version names a model of neither target: {0}")]
    Version(String),
    #[error("the file does not read: {0}")]
    Refused(String),
    #[error("the dictionary holds no drawable under the asset's name")]
    NotInDictionary,
    #[error("the fragment holds no drawable")]
    NoDrawable,
    #[error("the drawable holds no level of detail")]
    NoLevel,
    #[error("a geometry states no layout of its vertices")]
    NoLayout,
    #[error("a geometry states no position, or one in an encoding the model has no format for")]
    NoPosition,
    #[error("a geometry's bytes are no whole vertices of its layout")]
    PartVertex,
    #[error("a geometry's indices are no whole triangles of its vertices")]
    NotTriangles,
}

impl NoModel {
    /// The reason without what a file said of itself, which a count of reasons is kept by.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotAnArchetype => "the ask names no archetype",
            Self::Undeclared => "no type file declares the archetype",
            Self::NoAsset => "the archetype names no asset",
            Self::NoFile => "no file of the mount holds the asset",
            Self::Absent => "the archive gave no file",
            Self::Version(_) => "the file's version names a model of neither target",
            Self::Refused(_) => "the file does not read",
            Self::NotInDictionary => "the dictionary holds no such drawable",
            Self::NoDrawable => "the fragment holds no drawable",
            Self::NoLevel => "the drawable holds no level of detail",
            Self::NoLayout => "a geometry states no layout",
            Self::NoPosition => "a geometry's position has no format",
            Self::PartVertex => "a geometry's bytes are no whole vertices",
            Self::NotTriangles => "a geometry's indices are no whole triangles",
        }
    }
}

/// One texture a material binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// The hash of the parameter's name.
    pub parameter: u32,
    /// The texture's name, in lower case.
    pub texture: String,
    /// Whether the model's own dictionary holds it, where the others are found through the archetype's.
    pub embedded: bool,
}

/// One shader of a model as its file states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Material {
    /// The hash of the shader's name.
    pub shader: u32,
    /// The hash of the preset the shader was made from; 0 where the file states none.
    pub preset: u32,
    pub draw_bucket: u8,
    /// Every texture it binds, in the file's order.
    pub textures: Vec<Bound>,
}

/// What a model of GTA V carries as the title's own value: its shaders, and which draws each mesh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Materials {
    pub materials: Vec<Material>,
    /// `of[lod][mesh]` is the place in `materials` of what draws that mesh; `None` where the file names a shader its group lacks.
    pub of: Vec<Vec<Option<u16>>>,
}

/// One model asked for, and how it came out.
pub struct Outcome {
    pub ask: AssetKey,
    /// How the mount holds it, where a file was found.
    pub holding: Option<Holding>,
    /// The target its file is laid out for, where its version names one.
    pub target: Option<Generation>,
    pub model: Result<Model, NoModel>,
}

/// What a batch of asks opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Opened {
    /// How many files were read from the mount, each once.
    pub files: u32,
    /// Their bytes.
    pub bytes: u64,
}

/// Where the textures of one model are asked for.
#[derive(Clone, Copy)]
pub(crate) struct Scopes {
    /// The model's own dictionary.
    pub own: AssetKey,
    /// The dictionary its archetype names.
    pub shared: AssetKey,
}

/// What the two targets lay out differently of a model, read to the same terms.
pub(crate) trait Target: DrawableTarget + Sized {
    /// The name a shader's parameter for its colour texture is hashed under in a file of the target.
    const COLOUR: &'static str;
    /// The name its parameter for its bump texture is hashed under.
    const BUMP: &'static str;

    /// `shader` as the title's record of it; `group` names the textures its own dictionary holds.
    fn material(shader: &Self::InstanceData, group: &GrmShaderGroup<Self>) -> Material;

    /// `texture` as the contract's, with no level above `pixels` a side where a smaller one is stored.
    fn texture(texture: &Self::Texture, pixels: u32) -> Result<Texture, NoTexture>;
}

/// The binding of `slot` by the parameter `parameter`, where it binds a texture; `carried` says whether the model's own dictionary holds a texture of a name's hash.
// ⚠ A texture bound by name is the model's own where its own dictionary holds one of that name:
// the dictionary a model carries is looked in before the archetype's. The Enhanced installs'
// files bind what they carry by name.
fn bound<T, R>(
    parameter: u32,
    slot: &TextureSlot<T, R>,
    name: Option<&str>,
    carried: impl Fn(u32) -> bool,
) -> Option<Bound> {
    let texture = name.filter(|name| !name.is_empty())?.to_ascii_lowercase();
    let embedded = match slot {
        TextureSlot::Unbound => return None,
        TextureSlot::Embedded(_) => true,
        TextureSlot::Reference(_) => carried(or_io::string_hash(texture.as_bytes())),
    };
    Some(Bound {
        parameter,
        texture,
        embedded,
    })
}

/// Whether the dictionary `group` carries holds a texture whose name hashes to `name`.
fn carries<T: Target>(group: &GrmShaderGroup<T>, name: u32) -> bool {
    let own = group.texture_dictionary.as_deref();
    own.is_some_and(|own| {
        own.codes
            .iter()
            .any(|code| u64::from(*code) == u64::from(name))
    })
}

impl Target for Gen8 {
    const COLOUR: &'static str = "DiffuseSampler";
    const BUMP: &'static str = "BumpSampler";

    fn material(shader: &GrcInstanceData, group: &GrmShaderGroup<Self>) -> Material {
        let carried = |name| carries(group, name);
        let textures = shader.entries.iter().filter_map(|entry| match &entry.data {
            EntryData::Texture(slot) => {
                bound(entry.name_hash.get(), slot, group.slot_name(slot), carried)
            }
            EntryData::Float(_) => None,
        });
        Material {
            shader: shader.basis_hash.get(),
            preset: shader.material_hash.get(),
            draw_bucket: shader.draw_bucket,
            textures: textures.collect(),
        }
    }

    fn texture(texture: &GrcTexture, pixels: u32) -> Result<Texture, NoTexture> {
        if texture.image_type != 0 || texture.depth > 1 {
            return Err(NoTexture::NotFlat);
        }
        crate::textures::texture_of(texture, pixels)
    }
}

// ⚠ An Enhanced file hashes a parameter under the spelling the shader's program has for it,
// where a Legacy file hashes the name of the sampler.
impl Target for Gen9 {
    const COLOUR: &'static str = "DiffuseTex";
    const BUMP: &'static str = "BumpTex";

    fn material(shader: &GrcInstanceDataG9, group: &GrmShaderGroup<Self>) -> Material {
        let textures = shader.param_infos.params.iter().filter_map(|info| {
            let slot = shader.params.textures.get(info.texture()?)?;
            bound(info.name.get(), slot, group.slot_name(slot), |name| {
                carries(group, name)
            })
        });
        Material {
            shader: shader.name.get(),
            preset: shader.preset.get(),
            draw_bucket: shader.draw_bucket,
            textures: textures.collect(),
        }
    }

    fn texture(texture: &GrcTextureG9, pixels: u32) -> Result<Texture, NoTexture> {
        use or_io::gta5::gen9::{TextureDimension, TileMode};
        if texture.dimension.get() != TextureDimension::Texture2D || texture.depth > 1 {
            return Err(NoTexture::NotFlat);
        }
        // Every other mode stores its pixels in an order of its own.
        if !matches!(texture.tile_mode.get(), TileMode::Auto | TileMode::Linear) {
            return Err(NoTexture::Tiled);
        }
        crate::textures::texture_of(texture, pixels)
    }
}

/// The cube of side 1 about the origin that stands in for an entity nothing says what draws.
fn unit_box() -> Model {
    let mut vertices = Vec::with_capacity(8 * 12);
    for corner in 0..8u8 {
        for axis in 0..3 {
            let side: f32 = if corner >> axis & 1 == 0 { -0.5 } else { 0.5 };
            vertices.extend(side.to_le_bytes());
        }
    }
    let indices = vec![
        0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6,
        1, 3, 5, 3, 7, 5,
    ];
    let mesh = Mesh {
        layout: VertexLayout {
            stride: 12,
            elements: vec![Element {
                semantic: Semantic::Position,
                format: ElementFormat::F32x3,
                offset: 0,
            }],
        },
        vertices,
        indices,
        surface: Surface::default(),
    };
    Model {
        lods: vec![Lod {
            distance: f32::MAX,
            meshes: vec![mesh],
        }],
        title: None,
    }
}

/// What a channel of a vertex means to a shader; `None` for one the core's model has no word for.
fn semantic_of(channel: GrcFvfChannel) -> Option<Semantic> {
    use GrcFvfChannel as C;
    Some(match channel {
        C::Position => Semantic::Position,
        C::Weight => Semantic::BlendWeights,
        C::Binding => Semantic::BlendIndices,
        C::Normal => Semantic::Normal,
        C::Diffuse => Semantic::Colour(0),
        C::Specular => Semantic::Colour(1),
        C::Tangent0 => Semantic::Tangent,
        C::Tangent1 => return None,
        tex => Semantic::TexCoord((tex as u8).checked_sub(C::Tex0 as u8)?),
    })
}

/// How the core's model states an encoding; `None` for one it has no format for.
fn format_of(size: GrcDataSize) -> Option<ElementFormat> {
    Some(match size {
        GrcDataSize::Float2 => ElementFormat::F32x2,
        GrcDataSize::Float3 => ElementFormat::F32x3,
        GrcDataSize::Float4 => ElementFormat::F32x4,
        GrcDataSize::Color => ElementFormat::U8x4Norm,
        GrcDataSize::UByte4 => ElementFormat::U8x4,
        GrcDataSize::Half2 => ElementFormat::F16x2,
        GrcDataSize::Half4 => ElementFormat::F16x4,
        _ => return None,
    })
}

/// One element kept of a vertex: where vertex 0's value starts in the buffer, and the step to the next vertex's.
struct Kept {
    semantic: Semantic,
    format: ElementFormat,
    offset: usize,
    step: usize,
}

/// One geometry as a mesh of the game's own vertex bytes.
// ⚠ A channel other than the position whose encoding the core has no format for is left out of
// the elements. Where the file interleaves its vertices the bytes are the file's, whole, so such a
// channel's bytes stay in the vertex and nothing reads them. Where it keeps a channel in a run of
// its own the kept elements are gathered a vertex at a time, each value's bytes as the file has
// them, and the others are left behind.
fn mesh_of<T: Target>(geometry: &GrmGeometry<T>) -> Result<Mesh, NoModel> {
    if geometry.primitive_type != TRIANGLES {
        return Err(NoModel::NotTriangles);
    }
    let buffer = &*geometry.vertex_buffer;
    let layout = buffer.vertex_layout().ok_or(NoModel::NoLayout)?;
    let data = buffer.vertex_data();
    let stride = match buffer.vertex_stride() {
        0 => layout.packed_stride(),
        stated => stated,
    };
    if stride == 0 || !data.len().is_multiple_of(stride) {
        return Err(NoModel::PartVertex);
    }
    let count = data.len() / stride;
    let (mut kept, mut interleaved) = (Vec::new(), true);
    for (channel, slot) in layout.present() {
        let step = match slot.stride {
            0 => stride,
            own => own,
        };
        let size = slot.size.byte_size();
        interleaved &= step == stride && slot.offset + size <= stride;
        let last = count.saturating_sub(1) * step + slot.offset + size;
        if count > 0 && last > data.len() {
            return Err(NoModel::PartVertex);
        }
        match (semantic_of(channel), format_of(slot.size)) {
            (Some(semantic), Some(format)) => kept.push(Kept {
                semantic,
                format,
                offset: slot.offset,
                step,
            }),
            _ if channel == GrcFvfChannel::Position => return Err(NoModel::NoPosition),
            _ => {}
        }
    }
    if !kept.iter().any(|kept| kept.semantic == Semantic::Position) {
        return Err(NoModel::NoPosition);
    }
    let indices = T::indices(&geometry.index_buffer);
    if !indices.len().is_multiple_of(3) || indices.iter().any(|index| usize::from(*index) >= count)
    {
        return Err(NoModel::NotTriangles);
    }
    let element = |kept: &Kept, offset: usize| {
        Ok(Element {
            semantic: kept.semantic,
            format: kept.format,
            offset: u32::try_from(offset).map_err(|_| NoModel::PartVertex)?,
        })
    };
    let (stride, vertices, elements) = if interleaved {
        let elements: Result<Vec<Element>, NoModel> =
            kept.iter().map(|kept| element(kept, kept.offset)).collect();
        (stride, data.to_vec(), elements?)
    } else {
        let (mut elements, mut gathered) = (Vec::with_capacity(kept.len()), 0);
        for kept in &kept {
            elements.push(element(kept, gathered)?);
            gathered += kept.format.size() as usize;
        }
        let mut vertices = Vec::with_capacity(count * gathered);
        for vertex in 0..count {
            for kept in &kept {
                let at = kept.offset + vertex * kept.step;
                vertices.extend_from_slice(&data[at..at + kept.format.size() as usize]);
            }
        }
        (gathered, vertices, elements)
    };
    Ok(Mesh {
        layout: VertexLayout {
            stride: u32::try_from(stride).map_err(|_| NoModel::PartVertex)?,
            elements,
        },
        vertices,
        indices: indices.iter().map(|index| u32::from(*index)).collect(),
        surface: Surface::default(),
    })
}

/// What the plain pass draws a mesh of `material` with: its colour the texture its parameter for the colour binds, or the first it binds where it has none of that name.
fn surface_of<T: Target>(material: &Material, scopes: Scopes) -> Surface {
    let ask = |bound: &Bound| TextureAsk {
        scope: match bound.embedded {
            true => scopes.own,
            false => scopes.shared,
        },
        name: u64::from(or_io::string_hash(bound.texture.as_bytes())),
        pixels: u32::MAX,
    };
    let named = |name: &str| {
        let parameter = hash_name(name);
        (material.textures.iter()).find(|bound| bound.parameter == parameter)
    };
    let colour = named(T::COLOUR).or(material.textures.first());
    Surface {
        colour: colour.map(ask),
        normal: named(T::BUMP).map(ask),
        alpha: match material.draw_bucket {
            CUTOUT => Alpha::Cutout(CUTOUT_ABOVE),
            bucket if BLENDED.contains(&bucket) => Alpha::Blend,
            _ => Alpha::Opaque,
        },
        ..Surface::default()
    }
}

/// A drawable as the contract's model, the most detailed level first.
pub(crate) fn model_of<T: Target>(
    drawable: &RmcDrawable<T>,
    scopes: Scopes,
) -> Result<Model, NoModel> {
    let group = drawable.shader_group.as_deref();
    let materials: Vec<Material> = group.map_or_else(Vec::new, |group| {
        let shaders = group.shaders.iter();
        shaders.map(|shader| T::material(shader, group)).collect()
    });
    let (mut lods, mut of) = (Vec::new(), Vec::new());
    let levels = &drawable.lod_group;
    for (lod, distance) in levels.lods.iter().zip(levels.lod_distances) {
        if lod.models.is_empty() {
            continue;
        }
        let (mut meshes, mut drawn_by) = (Vec::new(), Vec::new());
        for model in &lod.models {
            for (at, geometry) in model.geometries.iter().enumerate() {
                let shader = model.shader_indices.get(at).copied();
                let material = shader.and_then(|shader| materials.get(usize::from(shader)));
                let mut mesh = mesh_of(geometry)?;
                if let Some(material) = material {
                    mesh.surface = surface_of::<T>(material, scopes);
                }
                meshes.push(mesh);
                drawn_by.push(shader.filter(|_| material.is_some()));
            }
        }
        lods.push(Lod { distance, meshes });
        of.push(drawn_by);
    }
    if lods.is_empty() {
        return Err(NoModel::NoLevel);
    }
    Ok(Model {
        lods,
        title: Some(Arc::new(Materials { materials, of })),
    })
}

/// One ask of a file: the entry of it the ask names, and where its textures are asked for.
struct Wanted {
    ask: AssetKey,
    entry: u32,
    scopes: Scopes,
}

/// Where an archetype's model lies in the mount.
pub(crate) struct HeldAt<'a> {
    pub holding: Holding,
    pub archive: usize,
    pub logical: &'a str,
    /// The name its drawable has in a dictionary.
    pub entry: u32,
    pub scopes: Scopes,
}

/// The file of the mount that holds the model of the archetype `ask` names.
// ⚠ A drawable dictionary the archetype states and the mount holds outranks a loose file of the
// asset's name, as `or_depends`' catalogue of models has it; a fragment outranks a drawable.
pub(crate) fn held_at(linked: &Linked, ask: AssetKey) -> Result<HeldAt<'_>, NoModel> {
    let archetype = u32::try_from(ask.0).map_err(|_| NoModel::NotAnArchetype)?;
    let row = (linked.world.archetype(archetype)).ok_or(NoModel::Undeclared)?;
    if row.asset_name == 0 {
        return Err(NoModel::NoAsset);
    }
    let dictionary = Some((Holding::Dictionary, row.drawable_dictionary)).filter(|(_, d)| *d != 0);
    let loose = [Holding::Fragment, Holding::File].map(|holding| (holding, row.asset_name));
    let mut holders = dictionary.into_iter().chain(loose);
    let found = holders.find_map(|(holding, stem)| {
        let (archive, logical) = linked.model_file(holding, stem)?;
        Some((holding, archive, logical))
    });
    let (holding, archive, logical) = found.ok_or(NoModel::NoFile)?;
    Ok(HeldAt {
        holding,
        archive,
        logical,
        entry: row.asset_name,
        scopes: Scopes {
            own: AssetKey(EMBEDDED | u64::from(archetype)),
            shared: AssetKey(u64::from(row.texture_dictionary)),
        },
    })
}

fn refused(error: or_io::Error) -> NoModel {
    NoModel::Refused(error.to_string())
}

/// The drawable of each of `wanted` in the file `bytes`, which holds models as `holding` and is laid out for `T`, handed to `each`.
pub(crate) fn drawables_of<T: Target, W>(
    holding: Holding,
    bytes: &[u8],
    wanted: &[W],
    entry: impl Fn(&W) -> u32,
    mut each: impl FnMut(&W, Result<&RmcDrawable<T>, NoModel>),
) where
    GtaDrawable<T>: File,
    PgDictionary<GtaDrawable<T>>: File,
    GtaFragType<T>: File,
{
    match holding {
        Holding::File => {
            let read = <GtaDrawable<T> as File>::from_bytes(bytes).map_err(refused);
            for wanted in wanted {
                each(wanted, read.as_ref().map(|d| &d.base).map_err(Clone::clone));
            }
        }
        Holding::Fragment => {
            let read = <GtaFragType<T> as File>::from_bytes(bytes).map_err(refused);
            let drawable = read.as_ref().map_err(Clone::clone).and_then(|fragment| {
                let common = fragment.common_drawable.as_deref();
                common.map(|d| &d.base).ok_or(NoModel::NoDrawable)
            });
            for wanted in wanted {
                each(wanted, drawable.clone());
            }
        }
        Holding::Dictionary => {
            let read = <PgDictionary<GtaDrawable<T>> as File>::from_bytes(bytes).map_err(refused);
            for wanted in wanted {
                let drawable = read.as_ref().map_err(Clone::clone).and_then(|dictionary| {
                    let at = dictionary.codes.iter().position(|c| *c == entry(wanted));
                    let held = at.and_then(|at| dictionary.entries.get(at)?.as_deref());
                    held.map(|d| &d.base).ok_or(NoModel::NotInDictionary)
                });
                each(wanted, drawable);
            }
        }
    }
}

impl Gta5World {
    /// The model of each of `asks`, or why it does not read; each file that holds any is opened once.
    pub fn read_models(&self, index: &Index, asks: &[AssetKey], out: &mut Vec<Outcome>) -> Opened {
        let mut opened = Opened::default();
        let unread = |ask: AssetKey, why: NoModel| Outcome {
            ask,
            holding: None,
            target: None,
            model: Err(why),
        };
        let Ok(linked) = linked(index) else {
            out.extend(asks.iter().map(|ask| unread(*ask, NoModel::Undeclared)));
            return opened;
        };
        let mut files: BTreeMap<(usize, &str), (Holding, Vec<Wanted>)> = BTreeMap::new();
        for ask in asks {
            if *ask == BOX {
                out.push(Outcome {
                    model: Ok(unit_box()),
                    ..unread(*ask, NoModel::NoFile)
                });
                continue;
            }
            match held_at(linked, *ask) {
                Ok(at) => {
                    let file = files.entry((at.archive, at.logical));
                    let (_, wanted) = file.or_insert_with(|| (at.holding, Vec::new()));
                    wanted.push(Wanted {
                        ask: *ask,
                        entry: at.entry,
                        scopes: at.scopes,
                    });
                }
                Err(why) => out.push(unread(*ask, why)),
            }
        }
        for ((archive, logical), (holding, wanted)) in files {
            let outcome = |wanted: &Wanted, target, model| Outcome {
                ask: wanted.ask,
                holding: Some(holding),
                target,
                model,
            };
            let Some(bytes) = self.source.open(archive, logical) else {
                let absent = |wanted| outcome(wanted, None, Err(NoModel::Absent));
                out.extend(wanted.iter().map(absent));
                continue;
            };
            opened.files += 1;
            opened.bytes += bytes.len() as u64;
            let target = match layout_of(holding.extension(), &bytes) {
                Ok(target) => target,
                Err(why) => {
                    let version =
                        |wanted| outcome(wanted, None, Err(NoModel::Version(why.clone())));
                    out.extend(wanted.iter().map(version));
                    continue;
                }
            };
            let entry = |wanted: &Wanted| wanted.entry;
            if target.is_gen9() {
                drawables_of::<Gen9, _>(holding, &bytes, &wanted, entry, |wanted, drawable| {
                    let model = drawable.and_then(|d| model_of(d, wanted.scopes));
                    out.push(outcome(wanted, Some(target), model));
                });
            } else {
                drawables_of::<Gen8, _>(holding, &bytes, &wanted, entry, |wanted, drawable| {
                    let model = drawable.and_then(|d| model_of(d, wanted.scopes));
                    out.push(outcome(wanted, Some(target), model));
                });
            }
        }
        opened
    }
}

impl ModelReader for Gta5World {
    fn models(
        &self,
        index: &Index,
        asks: &[AssetKey],
        _worker: &mut Worker,
        out: &mut Vec<(AssetKey, Model)>,
    ) {
        let mut outcomes = Vec::with_capacity(asks.len());
        self.read_models(index, asks, &mut outcomes);
        for outcome in outcomes {
            match outcome.model {
                Ok(model) => out.push((outcome.ask, model)),
                Err(why) => {
                    tracing::debug!(model = outcome.ask.0, %why, "a model gave the world nothing")
                }
            }
        }
    }

    fn textures(
        &self,
        index: &Index,
        asks: &[TextureAsk],
        _worker: &mut Worker,
        out: &mut Vec<(TextureAsk, Texture)>,
    ) {
        let mut outcomes = Vec::with_capacity(asks.len());
        self.read_textures(index, asks, &mut outcomes);
        for (ask, texture) in outcomes {
            match texture {
                Ok(texture) => out.push((ask, texture)),
                Err(why) => {
                    tracing::debug!(texture = ask.name, scope = ask.scope.0, %why, "a texture gave the world nothing")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::decl::Shared;
    use or_io::gta5::drawable::gen8;

    /// A Legacy geometry of `bytes` zeroed bytes of vertices of `channels`, drawn by `indices`: what `or_io`'s writer refuses and a file may still hold.
    fn geometry(
        channels: &[(GrcFvfChannel, GrcDataSize)],
        bytes: usize,
        indices: &[u16],
    ) -> GrmGeometry<Gen8> {
        let mut fvf = gen8::GrcFvf::default();
        for (channel, size) in channels {
            fvf.fvf |= 1 << *channel as u32;
            fvf.channel_sizes |= (*size as u64) << (*channel as u64 * 4);
        }
        fvf.channel_count = channels.len() as u8;
        let layout = or_io::codec::mesh::VertexLayout::of(fvf.fvf, fvf.channel_sizes);
        fvf.fvf_size = layout.packed_stride() as u8;
        GrmGeometry::<Gen8> {
            primitive_type: TRIANGLES,
            vertex_buffer: Box::new(gen8::GrcVertexBuffer {
                data: vec![0; bytes],
                fvf: Some(Box::new(fvf)),
                lock_ptr: Shared::Same,
                ..Default::default()
            }),
            index_buffer: Box::new(gen8::GrcIndexBuffer {
                indices: indices.to_vec(),
                ..Default::default()
            }),
            vertex_data: Shared::Same,
            ..Default::default()
        }
    }

    #[test]
    fn a_geometry_whose_vertices_or_indices_are_not_whole_is_refused_with_its_reason() {
        use GrcDataSize::{Float2, Float3, PackedNormal, Short4};
        use GrcFvfChannel::{Normal, Position, Tex0};
        let placed = [(Position, Float3), (Normal, PackedNormal), (Tex0, Float2)];
        // Three vertices of twelve, four and eight bytes: the normal's encoding has no format, so its bytes stay in the vertex and no element names them.
        let whole = mesh_of(&geometry(&placed, 72, &[0, 1, 2])).expect("it reads");
        assert_eq!((whole.layout.stride, whole.vertices.len()), (24, 72));
        let kept: Vec<(Semantic, u32)> = (whole.layout.elements.iter())
            .map(|element| (element.semantic, element.offset))
            .collect();
        assert_eq!(kept, [(Semantic::Position, 0), (Semantic::TexCoord(0), 16)]);
        let why = |geometry: GrmGeometry<Gen8>| mesh_of(&geometry).err();
        // An index that names a vertex past the last, and indices that are no whole triangles.
        assert_eq!(
            why(geometry(&placed, 72, &[0, 1, 3])),
            Some(NoModel::NotTriangles)
        );
        assert_eq!(
            why(geometry(&placed, 72, &[0, 1, 2, 0])),
            Some(NoModel::NotTriangles)
        );
        assert_eq!(
            why(geometry(&placed, 71, &[0, 1, 2])),
            Some(NoModel::PartVertex)
        );
        assert_eq!(
            why(geometry(&[(Normal, Float3)], 36, &[0, 1, 2])),
            Some(NoModel::NoPosition)
        );
        assert_eq!(
            why(geometry(&[(Position, Short4)], 24, &[0, 1, 2])),
            Some(NoModel::NoPosition)
        );
        let mut bare = geometry(&placed, 72, &[0, 1, 2]);
        bare.vertex_buffer.fvf = None;
        assert_eq!(why(bare), Some(NoModel::NoLayout));
    }
}
