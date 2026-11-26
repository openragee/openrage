//! GTA VI's headerless resources whose layout was measured, read by role from the virtual and the
//! physical section a scan's walk of their blocks gives; an edit that moves nothing is written back
//! into those sections in place.

mod accel_root;
mod bound;
mod bytes;
mod cloth;
mod dictionary;
mod drawable;
mod forest;
mod fragment;
mod fragment_records;
mod geometry_encode;
mod head;
mod instances;
mod navmesh;
mod octree;
mod particles;
mod skeleton;
mod small;
mod streams;
mod surfaces;
mod texture;
mod write;
mod write_bone_names;
mod write_bound_grow;
mod write_bound_reshape;
mod write_bound_runs;
mod write_bound_values;
mod write_bound_vertex;
mod write_bvh;
mod write_dictionary;
mod write_dictionary_entries;
mod write_dictionary_textures;
mod write_embedded;
mod write_embedded_repack;
mod write_filter_tracks;
mod write_filters;
mod write_fragment_child;
mod write_geometry;
mod write_material;
mod write_navmesh;
mod write_navmesh_delete;
mod write_navmesh_links;
mod write_navmesh_side;
mod write_palette;
mod write_physics;
mod write_skeleton;
mod write_volume;

pub use accel_root::{read_accel_root, AccelHead, AccelRecord, AccelRoot};
pub use bound::{
    material_key, read_bound, Bound, BoundChild, BoundGeometry, Bvh, ChildBound, Primitive,
    UnknownShape,
};
pub use cloth::{read_cloth, Cloth, ClothEdge, ClothKind, ClothMesh, ClothShape, OWNERS};
pub use dictionary::{read_dictionary, BoneRecord, Dictionary, Entry, FilterTrack};
pub use drawable::{
    model_shaders, read_drawable, read_drawable_with_bound, shader_buckets, stated_model_name,
    stated_model_word, trace_words, trace_words_of_entry, Channel, Drawable, DrawableWithBound,
    Geometry, Model, Palette, Parameter, ParameterValue, Shader, TraceGeometry, TraceModel,
    TraceShader, TraceWords,
};
pub use forest::{read_object_trees, ObjectItem, ObjectTrees, OBJECT_POINTER_TAG};
pub use fragment::{Archetype, BoneBound, Fragment, FragmentChild, FragmentGroup};
pub use fragment_records::{
    GlassPane, GlassRow, GlassWindow, GroupLink, GroupLinks, Lattice, LatticeCell, VehicleGlass,
    Wheel,
};
pub use head::{read_drawable_head, BufferKind, DrawableHead, EmbeddedTexture, TintPalette};
pub use instances::{read_instances, Instance, Instances, Structure};
pub use navmesh::{
    join_edges, joins, read_navmesh, Border, BorderEdge, Footprint, Join, NavEdge, NavMesh,
    NavNode, NavPolygon, SpecialLink, EIGHT_CORNERS, FOOTPRINT_UNIT, NOWHERE, SIDE_RECORD, SMALL,
    WATER,
};
pub use octree::{read_octree, NavOctree};
pub use particles::duplicate_particle_effect;
pub use particles::{add_particle_keys, ParticleKeysError};
pub use particles::{
    bake_particle_tables, read_particle_curves, write_particle_curves, ParticleCurve,
    ParticleCurves,
};
pub use particles::{class_of_bit, read_particle_gpu_tables, ClassNamedBy, GpuTables};
pub use particles::{
    copy_particle_behaviour, duplicate_particle_emitter, duplicate_particle_event,
    duplicate_particle_rule,
};
pub use particles::{
    read_particle_effects, write_particle_effects, Behaviour, Domain, DrawableRef, EffectRule,
    EmitterRule, Event, Evolution, EvolutionList, EvolvedProp, Kfp, KfpEntry, ParticleEffects,
    ParticleRule, SpawnedScalars, Spawner, BEHAVIOUR_KFPS, DOMAIN_KFPS, EFFECT_KFPS, EMITTER_KFPS,
};
pub use particles::{read_particle_gpu_buffers, GpuBuffer};
pub use particles::{remove_from_particle_effects, ParticleRemovals};
pub use particles::{
    BehaviourFields, DiffuseMode, EvolvedKeyframe, LightingMode, ProjectionMode, ShaderSwitch,
    Shading, COLOUR_RGBA_KFPS,
};
pub use skeleton::{read_dictionary_skeletons, read_skeleton, read_skeleton_at, Bone, Skeleton};
pub use skeleton::{read_skeleton_of, SkeletonOwner};
pub use small::{
    read_asset_index, read_keyed_blob_descriptor, read_neighbour_table, read_offset_table,
    AssetIndex, KeyedBlobDescriptor, NeighbourTable, OffsetTable,
};
pub use streams::{
    read_keyed_blob, read_morph_target, read_vertex_buffers, KeyedBlob, MorphTarget, VertexBuffers,
    Vertices,
};
pub use surfaces::Surface;
pub use texture::DictionaryTexture;
pub use write::{write_shaders, write_texture};
pub use write_bone_names::{bone_name_holds, rename_bones, BoneNameError};
pub use write_bound_grow::{
    add_bound_primitive, add_bound_vertex, copy_bound_child, delete_bound_child,
    delete_bound_primitive, delete_bound_vertex, fit_composite_box, reshape_bound_body,
    retyped_bound_child, write_retyped, BoundGrowError, BoundHolder, BoundReshape,
};
pub use write_bound_reshape::{edge_byte, rewire_triangle};
pub use write_bound_vertex::{vertex_move_holds, write_bound_vertices};
pub use write_dictionary::{
    read_dictionary_model, write_dictionary, DictionaryModel, DictionaryWriteError, ModelTexture,
};
pub use write_dictionary_entries::{copy_dictionary_entry, entry_key, remove_dictionary_entry};
pub use write_dictionary_textures::{write_dictionary_textures, HeldTexture, TexturePlanError};
pub use write_embedded::write_embedded_texture;
pub use write_embedded_repack::{
    embed_texture, key_word, reshape_embedded_texture, unembed_texture, EmbedError,
};
pub use write_filter_tracks::{write_filter_tracks, write_record_bones, ElementsError};
pub use write_filters::{write_filter_weights, write_record_values};
pub use write_fragment_child::{
    fragment_child_local_in, fragment_child_place, place_fragment_child, regroup_fragment_child,
    write_fragment_level,
};
pub use write_geometry::{bind_positions, rebox_bind_pose};
pub use write_geometry::{
    write_geometry, GeometryAt, GeometryEdit, GeometryWriteError, GeometryWritten,
};
pub use write_geometry::{write_geometry_cut, GeometryCut};
pub use write_material::stated_words;
pub use write_material::{
    shader_layout_holds, write_materials, write_materials_with, MaterialLayoutError,
};
pub use write_navmesh::{cut_edges, links_into, polygon_cut, PolygonCut};
pub use write_navmesh_delete::{
    delete_polygon, deletion, disable_polygon, disabling, polygon_deleted, NavDeleteError,
    NavRebuild,
};
pub use write_navmesh_links::{relinked, with_links, write_links};
pub use write_palette::{write_palette, PaletteAdd};
pub use write_physics::{
    bound_placement_holds, write_bound, write_drawable_bound, write_fragment, write_fragment_bound,
    write_polygon_flags,
};
pub use write_skeleton::{rebind_under, write_skeleton};
pub use write_volume::write_volume_texture;

use crate::container::rpf::{Rpf9Role, Rpf9Signature};

/// Why a headerless resource, or a part of it, is not read.
#[derive(Debug, thiserror::Error)]
pub enum HeaderlessError {
    #[error("the {what} at {at:#X} reaches past the {len} bytes of the {section} section")]
    PastEnd {
        what: &'static str,
        at: u64,
        section: &'static str,
        len: u64,
    },
    #[error("the {what} at {at:#X} is not a pointer of the measured form")]
    NotAPointer { what: &'static str, at: u64 },
    #[error("the {what} at {at:#X} is null where the measured layout has one")]
    Null { what: &'static str, at: u64 },
    #[error(
        "the record at {at:#X} has type byte {found}, where the measured layout has {expected}"
    )]
    BoundType { at: u64, found: u8, expected: u8 },
    #[error("the primitive at {at:#X} has type {found}, which is none of the five measured")]
    PrimitiveType { at: u64, found: u8 },
    #[error("a {what} is {index}, which is not below its count of {count}")]
    IndexPastCount {
        what: &'static str,
        index: u64,
        count: u64,
    },
    #[error("the {what} disagree: {first} and {second}")]
    Disagree {
        what: &'static str,
        first: u64,
        second: u64,
    },
    #[error(
        "the {what} at {at:#X} holds {found:#X}, not the value every measured body holds there"
    )]
    Constant {
        what: &'static str,
        at: u64,
        found: u64,
    },
    #[error("the BVH's leaves cover {covered} primitives, not the {primitives} the bound holds")]
    BvhCover { covered: u64, primitives: u64 },
    #[error("the record at {at:#X} is not a drawable root: it lacks a shader group at +0x10, a level at +0x50 or a positive distance at +0x7C, so what it is is not known")]
    NotADrawable { at: u64 },
    #[error("the dictionary's codes do not ascend")]
    CodesNotAscending,
    #[error("the record at {at:#X} is none of the kinds a dictionary is measured to hold: a texture, a drawable, a frame filter, an animation record or a 64-bit word")]
    UnknownEntry { at: u64 },
    #[error("the texture's DXGI format {0} has no measured element and tile order")]
    TextureFormat(u8),
    #[error("the texture's top mip sits in its tail tile, where no slot is measured for {format}")]
    TopMipInTail { format: String },
    #[error("the vertex declaration at {at:#X} does not hold to the measured layout: {why}")]
    Declaration { at: u64, why: &'static str },
    #[error("the index buffer is not the measured meshlet list: {why}")]
    Meshlets { why: &'static str },
    #[error("position encoding not understood: {why}")]
    Positions { why: &'static str },
    #[error("the vertex data does not hold to the measured layout: {why}")]
    Streams { why: &'static str },
    #[error("the palette at {at:#X} is neither ascending bone-name hashes nor a mask of no bone at or past its count of {count}")]
    Palette { at: u64, count: u32 },
    #[error("no layout of signature {0} was measured, so no reader reads it")]
    NoLayout(Rpf9Signature),
    #[error("the {what} at {at:#X} is in the physical section, which the block map states and which was not read")]
    PhysicalNotRead { what: &'static str, at: u64 },
    #[error("the body names its arrays more often than its {size} bytes could hold them, so it names an object more than once")]
    Aliased { size: u64 },
    #[error("the bound at {at:#X} has type {found}, whose layout was not measured with confidence, so it is not read")]
    BoundNotLaidOut { at: u64, found: u8 },
    #[error("the primitive bound at {at:#X}, of type {found}, states a box, centroid, radius or margin its fields do not give, as every measured one does")]
    PrimitiveBox { at: u64, found: u8 },
    #[error("the primitive at {at:#X} has type {found} in a geometry with no BVH, where every measured one is a triangle")]
    NotATriangle { at: u64, found: u8 },
    #[error("the primitive at {at:#X} states radius {radius}, which is not a length")]
    Radius { at: u64, radius: f32 },
    #[error("the material id {id:#X} at {at:#X} sets bit 15, which no measured id sets")]
    MaterialBit15 { at: u64, id: u64 },
    #[error(
        "the triangle at {at:#X} marks edge {edge} open with 0xFF, yet names a neighbour across it"
    )]
    OpenEdgeLinked { at: u64, edge: usize },
    #[error("the quadtree passes {nodes} nodes or {depth} levels, more than any measured, so it may loop")]
    TreeTooLarge { nodes: usize, depth: usize },
    #[error("the hash slot of key {key:#X} holds tag {tag:#04X}, not the low seven bits of the key's hash")]
    HashTag { key: u64, tag: u8 },
    #[error("the octree key {key:#X} is not a location code under the root: a 1, then three bits a level, its parent present")]
    OctreeKey { key: u64 },
    #[error("octree node {node} holds {word:#010X}, which is neither an internal node's word nor a placeholder, open or surface leaf's")]
    NodeWord { node: usize, word: u32 },
    #[error("the edit is not written in place: {0}")]
    NotInPlace(&'static str),
    #[error("the fragment does not hold to the measured layout: {0}")]
    Fragment(&'static str),
    #[error("the surfaces record at 0xD0 does not hold to the measured layout: {0}")]
    Surfaces(&'static str),
    #[error("the cloth record does not hold to the measured layout: {0}")]
    Cloth(&'static str),
    #[error("the particle effects list does not hold to the measured layout: {0}")]
    Particles(&'static str),
    #[error("the {shape} does not hold to the measured layout: {why}")]
    Layout {
        shape: &'static str,
        why: &'static str,
    },
}

type Result<T> = std::result::Result<T, HeaderlessError>;

/// What a headerless resource of a signature with a measured layout reads to.
#[derive(Debug)]
pub enum Resource {
    Bound(Bound),
    Drawable(Drawable),
    Dictionary(Dictionary),
    DrawableWithBound(DrawableWithBound),
    Instances(Instances),
    MorphTarget(MorphTarget),
    NavMesh(NavMesh),
    NavOctree(NavOctree),
    Cloth(Cloth),
    VertexBuffers(VertexBuffers),
    KeyedBlob(KeyedBlob),
    KeyedBlobDescriptor(KeyedBlobDescriptor),
    AssetIndex(AssetIndex),
    NeighbourTable(NeighbourTable),
    ParticleEffects(ParticleEffects),
}

/// The resource a body of `signature` holds, from its virtual and physical sections; `physical` is
/// `None` where the block map states one that was not read.
pub fn read(
    signature: Rpf9Signature,
    virtual_section: &[u8],
    physical: Option<&[u8]>,
) -> Result<Resource> {
    match signature.role() {
        Some(Rpf9Role::Bound) => read_bound(virtual_section).map(Resource::Bound),
        Some(Rpf9Role::Drawable) => {
            read_drawable(virtual_section, physical).map(Resource::Drawable)
        }
        Some(Rpf9Role::Dictionary) => {
            read_dictionary(virtual_section, physical).map(Resource::Dictionary)
        }
        Some(Rpf9Role::DrawableWithBound) => {
            read_drawable_with_bound(virtual_section, physical).map(Resource::DrawableWithBound)
        }
        Some(Rpf9Role::AccelerationStructure) => {
            read_instances(virtual_section, physical).map(Resource::Instances)
        }
        Some(Rpf9Role::MorphTarget) => {
            read_morph_target(virtual_section, physical).map(Resource::MorphTarget)
        }
        Some(Rpf9Role::NavMesh) => read_navmesh(virtual_section).map(Resource::NavMesh),
        Some(Rpf9Role::NavOctree) => read_octree(virtual_section).map(Resource::NavOctree),
        Some(Rpf9Role::Cloth) => read_cloth(virtual_section, physical).map(Resource::Cloth),
        Some(Rpf9Role::VertexBuffers) => {
            read_vertex_buffers(virtual_section, physical).map(Resource::VertexBuffers)
        }
        Some(Rpf9Role::KeyedBlob) => {
            read_keyed_blob(virtual_section, physical).map(Resource::KeyedBlob)
        }
        Some(Rpf9Role::KeyedBlobDescriptor) => {
            read_keyed_blob_descriptor(virtual_section, physical).map(Resource::KeyedBlobDescriptor)
        }
        Some(Rpf9Role::AssetIndex) => {
            read_asset_index(virtual_section, physical).map(Resource::AssetIndex)
        }
        Some(Rpf9Role::NeighbourTable) => {
            read_neighbour_table(virtual_section, physical).map(Resource::NeighbourTable)
        }
        Some(Rpf9Role::ParticleEffects) => {
            read_particle_effects(virtual_section, physical).map(Resource::ParticleEffects)
        }
        None => Err(HeaderlessError::NoLayout(signature)),
    }
}

#[cfg(test)]
mod tests;
