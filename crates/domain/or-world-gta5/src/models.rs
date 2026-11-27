//! What draws an archetype of GTA V, as the contract's model: asked for by the name of its
//! archetype, found through the index's row of it, and read from a drawable that is a file of its own.
//!
//! ⚠ Only a Legacy drawable that is a file of its own reads here. An entry of a drawable
//! dictionary, a fragment, an Enhanced drawable and every texture give nothing yet, and the
//! selection that asked goes on waiting for them.

use or_depends::WorldSource;
use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexLayout as Fvf};
use or_io::decl::{File, Gen8};
use or_io::gta5::{GrmGeometry, GtaDrawable};
use or_world::contract::{ModelReader, Worker};
use or_world::model::{
    AssetKey, Element, ElementFormat, Index, Lod, Mesh, Model, Semantic, Surface, Texture,
    TextureAsk, VertexLayout,
};

use crate::index::{linked, Linked};
use crate::install::Gta5World;

/// What draws an instance that stands as a box: a name above every archetype's, which is 32 bits.
pub const BOX: AssetKey = AssetKey(1 << 32);

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

/// What a channel of a Legacy vertex means to a shader; `None` for one the core's model has no word for.
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

/// One geometry as a mesh of the game's own vertex bytes; `None` where its bytes are no whole vertices of its layout, its indices no whole triangles of them, or its position in an encoding the core has no format for.
// ⚠ A channel other than the position whose encoding the core has no format for is left out of
// the elements: its bytes stay in the vertex, and nothing reads them.
fn mesh_of(geometry: &GrmGeometry<Gen8>) -> Option<Mesh> {
    let buffer = &geometry.vertex_buffer;
    let fvf = buffer.fvf.as_ref()?;
    let layout = Fvf::of(fvf.fvf, fvf.channel_sizes);
    let stride = match usize::from(fvf.fvf_size) {
        0 => layout.packed_stride(),
        stated => stated,
    };
    if stride == 0 || stride < layout.packed_stride() || !buffer.data.len().is_multiple_of(stride) {
        return None;
    }
    let mut elements = Vec::new();
    for channel in GrcFvfChannel::ALL {
        let Some(slot) = layout.slot(channel) else {
            continue;
        };
        match (semantic_of(channel), format_of(slot.size)) {
            (Some(semantic), Some(format)) => elements.push(Element {
                semantic,
                format,
                offset: u32::try_from(slot.offset).ok()?,
            }),
            _ if channel == GrcFvfChannel::Position => return None,
            _ => {}
        }
    }
    let count = buffer.data.len() / stride;
    let indices = &geometry.index_buffer.indices;
    if !indices.len().is_multiple_of(3) || indices.iter().any(|index| usize::from(*index) >= count)
    {
        return None;
    }
    Some(Mesh {
        layout: VertexLayout {
            stride: u32::try_from(stride).ok()?,
            elements,
        },
        vertices: buffer.data.clone(),
        indices: indices.iter().map(|index| u32::from(*index)).collect(),
        surface: Surface::default(),
    })
}

/// A Legacy drawable as the contract's model, the most detailed level first; `None` where it holds no level or a geometry of it is no mesh.
fn model_of(drawable: &GtaDrawable<Gen8>) -> Option<Model> {
    let group = &drawable.base.lod_group;
    let mut lods = Vec::new();
    for (lod, distance) in group.lods.iter().zip(group.lod_distances) {
        if lod.models.is_empty() {
            continue;
        }
        let geometries = lod.models.iter().flat_map(|model| &model.geometries);
        let meshes: Option<Vec<Mesh>> = geometries.map(|geometry| mesh_of(geometry)).collect();
        lods.push(Lod {
            distance,
            meshes: meshes?,
        });
    }
    (!lods.is_empty()).then_some(Model { lods, title: None })
}

impl Gta5World {
    /// The model of the archetype `ask` names; `None` where nothing reads of it yet.
    fn model(&self, linked: &Linked, ask: AssetKey) -> Option<Model> {
        let archetype = u32::try_from(ask.0).ok()?;
        let row = linked.world.archetype(archetype)?;
        if row.asset_name == 0 || row.drawable_dictionary != 0 {
            return None;
        }
        let (archive, logical) = linked.drawable(row.asset_name)?;
        let bytes = self.source.open(archive, logical)?;
        match <GtaDrawable<Gen8> as File>::from_bytes(&bytes) {
            Ok(drawable) => model_of(&drawable),
            Err(e) => {
                tracing::debug!(file = logical, error = %e, "a drawable gave the world no model");
                None
            }
        }
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
        let Ok(linked) = linked(index) else {
            return;
        };
        for ask in asks {
            let model = match *ask {
                BOX => Some(unit_box()),
                ask => self.model(linked, ask),
            };
            out.extend(model.map(|model| (*ask, model)));
        }
    }

    // No surface of a model read here asks for a texture, so none is asked for and none is read.
    fn textures(
        &self,
        _index: &Index,
        _asks: &[TextureAsk],
        _worker: &mut Worker,
        _out: &mut Vec<(TextureAsk, Texture)>,
    ) {
    }
}
