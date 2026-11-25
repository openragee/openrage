//! `.ybn`: collision bounds. A header whose type selects one of nine layouts,
//! a polygon soup whose own kind is three bits of a float, a BVH over it, and
//! composites holding child bounds with per-child transforms.

use std::sync::Arc;

use super::shared::{Mat34V, PgBase, Vec3V, Vec4V, Vector3};
use crate::decl::*;
use crate::desc::value::{BlobCodecs, BlobReader, BlobSource};
use crate::desc::{Unresolvable, Value, ValueError};
use crate::write::{BlobHeap, BlobWriter, BlobWriters, BuildError, Owner, Placement};
use crate::Generation;

pub mod build;

impl Resource for PhBoundAny {
    const EXT: &'static str = "ybn";
    const TARGET: Generation = Generation::Gen8;
}

macro_rules! each_bound {
    ($any:expr, $b:ident => $e:expr) => {
        match $any {
            PhBoundAny::PhBoundSphere($b) => $e,
            PhBoundAny::PhBoundCapsule($b) => $e,
            PhBoundAny::PhBoundBox($b) => $e,
            PhBoundAny::PhBoundGeometry($b) => $e,
            PhBoundAny::PhBoundBVH($b) => $e,
            PhBoundAny::PhBoundComposite($b) => $e,
            PhBoundAny::PhBoundDisc($b) => $e,
            PhBoundAny::PhBoundCylinder($b) => $e,
            PhBoundAny::PhBoundPlane($b) => $e,
            PhBoundAny::Base($b) => $e,
        }
    };
}

impl HasOrigin for PhBoundAny {
    fn origin(&self) -> &Origin {
        each_bound!(self, b => b.origin())
    }

    fn origin_mut(&mut self) -> &mut Origin {
        each_bound!(self, b => b.origin_mut())
    }
}

/// `phBound::BoundType`: the shape a bound is, and so which layout follows its header.
#[record(enum, width = u8)]
pub enum BoundType {
    /// Sphere
    Sphere = 0,
    /// Capsule
    Capsule = 1,
    /// TaperedCapsule
    TaperedCapsule = 2,
    /// Box
    Box = 3,
    /// Geometry
    Geometry = 4,
    /// GeometryCurved
    GeometryCurved = 5,
    /// Grid
    Grid = 6,
    /// Ribbon
    Ribbon = 7,
    /// Bvh
    Bvh = 8,
    /// Surface
    Surface = 9,
    /// Composite
    Composite = 10,
    /// Triangle
    Triangle = 11,
    /// Disc
    Disc = 12,
    /// Cylinder
    Cylinder = 13,
    /// RemovedProbe
    RemovedProbe = 14,
    /// Plane
    Plane = 15,
}

/// What every bound starts with; `type` selects the layout that follows.
#[record(
    size = 0x70,
    tag = "type",
    variant(0 => PhBoundSphere),
    variant(1 => PhBoundCapsule),
    variant(3 => PhBoundBox),
    variant(4 => PhBoundGeometry),
    variant(8 => PhBoundBVH),
    variant(10 => PhBoundComposite),
    variant(12 => PhBoundDisc),
    variant(13 => PhBoundCylinder),
    variant(15 => PhBoundPlane)
)]
pub struct PhBound {
    pub base: PgBase,
    pub r#type: Open<BoundType>,
    pub flags: u8,
    pub part_index: u16,
    #[pad_after(8)]
    pub radius_around_centroid: f32,
    pub bounding_box_max_xyz_margin_w: Vec4V,
    /// ⚠ `w` holds the reference count's bits: reach the corner through [`PhBound::bounding_box_min`].
    #[init(Vec4V { w: f32::from_bits(1), ..Default::default() })]
    bounding_box_min_xyz_ref_count_w: Vec4V,
    pub centroid_offset_xyz_material_id0_w: Vec4V,
    pub cg_offset_xyz_material_id1_w: Vec4V,
    pub volume_distribution: Vec4V,
}

impl PhBound {
    /// The bounding box's min corner.
    pub fn bounding_box_min(&self) -> [f32; 3] {
        let at = std::mem::offset_of!(Raw<Self>, bounding_box_min_xyz_ref_count_w);
        self.__carry
            .hidden::<Vec4V>(at)
            .map_or([0.0; 3], |v| [v.x, v.y, v.z])
    }

    /// Sets the bounding box's min corner; the reference count in its `w` word is kept.
    pub fn set_bounding_box_min(&mut self, [x, y, z]: [f32; 3]) {
        let at = std::mem::offset_of!(Raw<Self>, bounding_box_min_xyz_ref_count_w);
        let w = self
            .__carry
            .hidden::<Vec4V>(at)
            .map_or(f32::from_bits(1), |v| v.w);
        let min = Vec4V {
            x,
            y,
            z,
            w,
            ..Default::default()
        };
        self.__carry.hide(at, min);
    }
}

/// A sphere: its radius and centre are the header's.
#[record(size = 0x70)]
pub struct PhBoundSphere {
    pub base: PhBound,
}

/// A box: its extent is the header's bounding box.
#[record(size = 0x70)]
pub struct PhBoundBox {
    pub base: PhBound,
}

/// A plane.
#[record(size = 0x70)]
pub struct PhBoundPlane {
    pub base: PhBound,
}

/// A capsule.
#[record(size = 0x80)]
pub struct PhBoundCapsule {
    pub base: PhBound,
    pub capsule_half_height: f32,
    pad: [u8; 12],
}

/// A disc, flat along X; its radius, centre and thickness are the header's.
#[record(size = 0x80)]
pub struct PhBoundDisc {
    pub base: PhBound,
    pad: [u8; 16],
}

/// A cylinder along Y; its radius and half-height are the header's.
#[record(size = 0x80)]
pub struct PhBoundCylinder {
    pub base: PhBound,
    pad: [u8; 16],
}

/// One vertex as three `CompressedVertexType` words: scale by `un_quantize_factor`, add `bounding_box_center`.
///
/// The engine runs the words back to back and declares no struct for them.
#[record(size = 0x06)]
pub struct CompressedVertex {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

/// `Color32`: a colour packed in one word.
#[record(size = 0x04)]
pub struct Color32 {
    pub color: u32,
}

/// `phMaterialMgr::Id`: a material and its per-polygon flags, packed in one word.
#[record(size = 0x08)]
pub struct PhMaterialMgrId {
    pub id: u64,
}

/// The bits of a polygon's `area` that say which primitive it is.
pub const PRIMITIVE_TYPE_MASK: u32 = 0b111;
/// The mask of a polygon's or a box's vertex index; the top bit is carried.
pub const VERTEX_INDEX_MASK: u16 = 0x7FFF;
/// A polygon edge with no neighbour.
pub const NO_NEIGHBOUR: u16 = 0xFFFF;

/// A polygon, and the base the four primitives tag off: ⚠ the tag is the low
/// three bits of `area`, so writing a plain float there retypes it.
#[record(
    size = 0x10,
    tag = "area",
    tag_bits(0, 3),
    variant(1 => PhPrimSphere),
    variant(2 => PhPrimCapsule),
    variant(3 => PhPrimBox),
    variant(4 => PhPrimCylinder)
)]
pub struct PhPolygon {
    pub area: f32,
    pub vertex_indices: [u16; 3],
    pub neighboring_polygons: [u16; 3],
}

impl PhPolygon {
    /// Which primitive the slot holds: 0 a polygon, 1 to 4 a sphere, capsule, box or cylinder.
    pub fn primitive(&self) -> u32 {
        self.area.to_bits() & PRIMITIVE_TYPE_MASK
    }
}

/// A sphere primitive: a centre vertex and a radius.
#[record(size = 0x10)]
pub struct PhPrimSphere {
    pub prim_type: u8,
    pad0: u8,
    pub center_index: u16,
    pub radius: f32,
    pad1: [u8; 8],
}

/// A capsule primitive: two end vertices and a radius.
#[record(size = 0x10)]
pub struct PhPrimCapsule {
    pub prim_type: u8,
    pad0: u8,
    pub end_index0: u16,
    pub radius: f32,
    pub end_index1: u16,
    pad1: [u8; 6],
}

/// A box primitive: four corner vertices.
#[record(size = 0x10)]
pub struct PhPrimBox {
    pub prim_type: u8,
    #[pad_after(1)]
    pad0: [u8; 2],
    pub vertex_indices: [u16; 4],
    pad1: [u8; 4],
}

/// A cylinder primitive: two end vertices and a radius.
#[record(size = 0x10)]
pub struct PhPrimCylinder {
    pub prim_type: u8,
    pad0: u8,
    pub end_index0: u16,
    pub radius: f32,
    pub end_index1: u16,
    pad1: [u8; 6],
}

/// How many octants a geometry's octant map divides its vertices into.
pub const OCTANT_COUNT: usize = 8;

/// A polygon soup: quantized vertices, polygons and their materials.
///
/// ⚠ `phBoundPolyhedron`'s members are declared here rather than as an
/// embedded base, because a run's `#[len]` names a field of its own record
/// and `poly_mat_index_list` is counted by `num_polygons`.
#[record(size = 0x130, recompute = PhBoundGeometry::attribs_width)]
pub struct PhBoundGeometry {
    pub base: PhBound,
    #[pad_after(4)]
    vertices_pad: u32,
    /// ⚠ Counted by `num_convex_hull_vertices` through its full mask, so an
    /// empty run keeps the count: every `.ybn` geometry states a hull as long
    /// as its vertices beside a null run.
    #[len(masked = "num_convex_hull_vertices", mask = 0xFFFF)]
    pub compressed_shrunk_vertices: Vec<CompressedVertex>,
    pub use_active_components: Bool8,
    pub is_flat: Bool8,
    #[pad_after(1)]
    num_per_vertex_attribs: u8,
    #[pad_after(2)]
    num_convex_hull_vertices: u16,
    #[len(field = "num_polygons")]
    pub polygons: Vec<PhPolygon>,
    pub un_quantize_factor: Vec3V,
    pub bounding_box_center: Vec3V,
    #[len(field = "num_vertices")]
    pub compressed_vertices: Vec<CompressedVertex>,
    #[len(field = "num_vertices")]
    pub vertex_attribs: Vec<Color32>,
    #[note("the octant map's counts, placed and written by its codec")]
    octant_vert_counts: Ptr<Opaque>,
    pub octant_verts: Option<Box<OctantMap>>,
    num_vertices: i32,
    num_polygons: i32,
    pad0: [u32; 6],
    #[len(field = "num_materials")]
    pub material_ids: Vec<PhMaterialMgrId>,
    #[len(field = "num_material_colors")]
    pub material_colors: Vec<Color32>,
    #[pad_after(4)]
    pad1: [u8; 4],
    #[len(field = "num_second_surface_vertex_displacements")]
    pub second_surface_vertex_displacements: Vec<f32>,
    #[pad_after(4)]
    num_second_surface_vertex_displacements: i32,
    #[len(field = "num_polygons")]
    pub poly_mat_index_list: Vec<u8>,
    num_materials: u8,
    num_material_colors: u8,
    pad: [u8; 14],
}

impl PhBoundGeometry {
    /// How many vertices the shrunk hull is stated at beside a null run; `None` for a value built from scratch.
    pub fn num_convex_hull_vertices(&self) -> Option<u16> {
        let at = std::mem::offset_of!(Raw<Self>, num_convex_hull_vertices);
        self.__carry.hidden::<u16>(at).copied()
    }

    /// States the shrunk hull's length beside a null run, which a save keeps as stated; a run that is there is counted instead.
    ///
    /// ⚠ A `.ybn` geometry states its vertex count here, so a bound whose vertices change in number states the new count.
    pub fn set_num_convex_hull_vertices(&mut self, vertices: u16) {
        let at = std::mem::offset_of!(Raw<Self>, num_convex_hull_vertices);
        self.__carry.hide(at, vertices);
    }

    /// ⚠ `vertex_attribs` is read as one colour word per vertex: a live run
    /// states a width of 1, and a carried width above 1 beside one is refused.
    fn attribs_width(
        &self,
        raw: &mut Raw<Self>,
        _: &mut [(u32, Value)],
        _: usize,
        _: &Cx,
    ) -> Result<(), OwnedError> {
        if self.vertex_attribs.is_empty() {
            return Ok(());
        }
        match raw.num_per_vertex_attribs {
            0 | 1 => raw.num_per_vertex_attribs = 1,
            per => {
                return Err(OwnedError::Invalid {
                    what: "PhBoundGeometry::vertex_attribs".into(),
                    why: format!(
                        "the run is one word per vertex, and `num_per_vertex_attribs` states {per}"
                    ),
                })
            }
        }
        Ok(())
    }
}

/// A BVH node: a quantized box, and either a leaf's first polygon or an escape index.
#[record(size = 0x10)]
pub struct PhOptimizedBvhNode {
    pub aabb_min: [i16; 3],
    pub aabb_max: [i16; 3],
    pub node_data: u16,
    #[pad_after(1)]
    pub polygon_count: u8,
}

/// A BVH subtree: a quantized box and the range of nodes it spans.
#[record(size = 0x10)]
pub struct PhBVHSubtreeInfo {
    pub aabb_min: [i16; 3],
    pub aabb_max: [i16; 3],
    pub root_node_index: u16,
    /// The node just after the subtree's last.
    pub last_index: u16,
}

/// A bounding-volume hierarchy over a geometry's polygons or a composite's children.
///
/// ⚠ `num_nodes` is an allocated size, carried and grown only when the nodes
/// outgrow it: 11 of the 79 `.ybn` trees are allocated beyond the nodes they fill.
#[record(size = 0x80, recompute = PhOptimizedBvh::counts)]
pub struct PhOptimizedBvh {
    #[len(field = "num_nodes_in_use")]
    pub contiguous_nodes: Vec<PhOptimizedBvhNode>,
    num_nodes_in_use: i32,
    num_nodes: i32,
    #[pad_after(12)]
    pad0: [i32; 1],
    pub aabb_min: Vector3,
    pub aabb_max: Vector3,
    pub aabb_center: Vector3,
    pub quantize: Vector3,
    pub inv_quantize: Vector3,
    #[len(field = "num_subtree_headers")]
    pub subtree_headers: Vec<PhBVHSubtreeInfo>,
    num_subtree_headers: u16,
    #[note("the headers in use; every header, in all 79 .ybn trees")]
    #[pad_after(4)]
    cur_subtree_header_index: u16,
}

impl PhOptimizedBvh {
    fn counts(
        &self,
        raw: &mut Raw<Self>,
        _: &mut [(u32, Value)],
        _: usize,
        _: &Cx,
    ) -> Result<(), OwnedError> {
        raw.num_nodes = raw.num_nodes.max(raw.num_nodes_in_use);
        raw.cur_subtree_header_index = raw.num_subtree_headers;
        Ok(())
    }
}

/// A geometry bound with a BVH over its polygons.
#[record(size = 0x150)]
pub struct PhBoundBVH {
    pub base: PhBoundGeometry,
    pub bvh: Option<Box<PhOptimizedBvh>>,
    active_polygon_indices: Ptr<Opaque>,
    num_active_polygons: u16,
    pad: [u8; 14],
}

/// One child's pair of `type_and_include_flags` words: its type flags, then its include flags.
///
/// The engine runs the words back to back and declares no struct for them.
#[record(size = 0x08)]
pub struct TypeAndIncludeFlags {
    pub type_flags: u32,
    pub include_flags: u32,
}

/// One child's pair of `local_box_min_maxs` vectors: its box's minimum, then its maximum.
///
/// The engine runs the vectors back to back and declares no struct for them.
#[record(size = 0x20)]
pub struct LocalBoxMinMax {
    pub min: Vec3V,
    pub max: Vec3V,
}

/// Several bounds, each with its transform, box and flags.
///
/// ⚠ The two matrix runs are one allocation in every `.ybn` composite, and
/// the two flag runs in 19 of 21: `Shared::Same` keeps the sharing.
#[record(size = 0xb0, recompute = PhBoundComposite::capacity)]
pub struct PhBoundComposite {
    pub base: PhBound,
    #[len(field = "num_bounds")]
    pub bounds: Vec<Option<Box<PhBoundAny>>>,
    #[len(field = "num_bounds")]
    pub current_matrices: Vec<Mat34V>,
    #[shares(current_matrices)]
    #[len(field = "num_bounds")]
    pub last_matrices: Shared<Vec<Mat34V>>,
    #[len(field = "num_bounds")]
    pub local_box_min_maxs: Vec<LocalBoxMinMax>,
    #[len(field = "num_bounds")]
    pub type_and_include_flags: Vec<TypeAndIncludeFlags>,
    #[shares(type_and_include_flags)]
    #[len(field = "num_bounds")]
    pub owned_type_and_include_flags: Shared<Vec<TypeAndIncludeFlags>>,
    max_num_bounds: u16,
    #[pad_after(4)]
    num_bounds: u16,
    pub bvh_structure: Option<Box<PhOptimizedBvh>>,
}

impl PhBoundComposite {
    /// ⚠ `max_num_bounds` is an allocation, like a counted array's capacity: kept, and grown to the children.
    fn capacity(
        &self,
        raw: &mut Raw<Self>,
        _: &mut [(u32, Value)],
        _: usize,
        _: &Cx,
    ) -> Result<(), OwnedError> {
        raw.max_num_bounds = raw.max_num_bounds.max(raw.num_bounds);
        Ok(())
    }
}

/// A geometry's octant map: per octant of its bounding box, the indices of the vertices it lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OctantMap {
    pub octants: [Vec<u32>; OCTANT_COUNT],
}

/// The table `octant_verts` points at: one pointer per octant, each at that octant's indices.
///
/// ⚠ Each pointer's length is its octant's entry in `octant_vert_counts`,
/// which nothing here can state, so [`OctantMap`] is read and written whole
/// by its codec.
#[doc(hidden)]
#[record(size = 0x40)]
pub struct OctantVertsTable {
    octant_verts: [Ptr<Opaque>; OCTANT_COUNT],
}

const OCTANT_MAP: &str = "PhBoundGeometry::octant_verts";
const OCTANT_VERTS: &str = "octant_verts";
const OCTANT_VERT_COUNTS: &str = "octant_vert_counts";
const RUNS: &str = "runs";
const COUNTS_BYTES: usize = 4 * OCTANT_COUNT;
const TABLE_BYTES: usize = 8 * OCTANT_COUNT;

fn behind_a_pointer(at: usize) -> OwnedError {
    OwnedError::Invalid {
        what: OCTANT_MAP.into(),
        why: format!("an octant map is reached through a pointer, not embedded at {at:#x}"),
    }
}

impl Heap for OctantMap {
    fn heap_bytes(&self) -> usize {
        self.octants.heap_bytes()
    }
}

impl Owned for OctantMap {
    type Raw = Raw<OctantVertsTable>;
    const LEAF: bool = false;

    fn from_raw(_: &Self::Raw, _: Kids<'_>, at: usize, _: &Rx<'_>) -> Result<Self, OwnedError> {
        Err(behind_a_pointer(at))
    }

    fn to_raw(
        &self,
        _: &mut Vec<(u32, Value)>,
        at: usize,
        _: &Cx,
    ) -> Result<Self::Raw, OwnedError> {
        Err(behind_a_pointer(at))
    }

    fn from_value(v: &Value, _: &Rx<'_>) -> Result<Self, OwnedError> {
        octants_of(v)
            .and_then(|runs| runs.try_into().ok())
            .map(|octants| Self { octants })
            .ok_or_else(|| OwnedError::Invalid {
                what: OCTANT_MAP.into(),
                why: "the value was not read by the octant map's codec".into(),
            })
    }

    fn to_element(&self, _: &Cx) -> Result<Value, OwnedError> {
        Ok(Value::Blob {
            codec: OCTANT_MAP.into(),
            parts: vec![(
                RUNS.into(),
                Value::Array {
                    elements: self
                        .octants
                        .iter()
                        .map(|run| Value::Bytes(run.iter().flat_map(|v| v.to_le_bytes()).collect()))
                        .collect(),
                    capacity: None,
                },
            )],
        })
    }
}

/// The runs an octant map's codec value holds, one per octant.
fn octants_of(blob: &Value) -> Option<Vec<Vec<u32>>> {
    let Value::Blob { codec, .. } = blob else {
        return None;
    };
    if codec != OCTANT_MAP {
        return None;
    }
    blob.part(RUNS)?
        .elements()?
        .iter()
        .map(|run| {
            let bytes = run.as_bytes()?;
            (bytes.len() % 4 == 0).then(|| {
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|w| u32::from_le_bytes(*w))
                    .collect()
            })
        })
        .collect()
}

/// The octant map's codec: eight counts, eight pointers, and a run of vertex indices behind each.
///
/// ⚠ One allocation in every shipped map, 257 of 257 through `.yft`: the
/// counts, the pointer table 0x20 after them, then the runs in octant order.
/// The codec sits on `octant_verts` and places `octant_vert_counts` as its sibling.
struct OctantMapBlob;

/// The readers a bounds-bearing format's tree is read with.
pub fn readers() -> BlobCodecs {
    BlobCodecs::new().at("PhBoundGeometry", OCTANT_VERTS, Arc::new(OctantMapBlob))
}

/// The writers its canonical save is built with.
pub fn writers() -> BlobWriters {
    BlobWriters::new().at("PhBoundGeometry", OCTANT_VERTS, Arc::new(OctantMapBlob))
}

impl BlobReader for OctantMapBlob {
    fn read(&self, blob: &dyn BlobSource) -> Result<Value, ValueError> {
        let (owner, owner_at) = blob.owner();
        let (_, table_at) = blob.blob();
        let image = blob.image();
        let bad = |field: &str, reason| ValueError::BadPointer {
            field: field.to_string(),
            reason,
        };
        let counts_field =
            owner
                .field_by_name(OCTANT_VERT_COUNTS)
                .ok_or_else(|| ValueError::NoCountField {
                    field: OCTANT_VERTS.to_string(),
                    count_field: OCTANT_VERT_COUNTS.to_string(),
                })?;
        let counts_at = image
            .pointer_at(owner_at + counts_field.offset)
            .and_then(|address| image.resolve(address))
            .map_err(|reason| bad(OCTANT_VERT_COUNTS, reason))?;
        let word = |at: usize| {
            image
                .slice_at(at, 4)
                .map(|b| u32::from_le_bytes(b.try_into().expect("four bytes")))
                .map_err(|reason| bad(OCTANT_VERT_COUNTS, reason))
        };
        let mut runs = Vec::with_capacity(OCTANT_COUNT);
        for i in 0..OCTANT_COUNT {
            let count = word(counts_at + 4 * i)? as usize;
            let bytes = if count == 0 {
                Vec::new()
            } else {
                let run = image
                    .pointer_at(table_at + 8 * i)
                    .and_then(|address| image.resolve(address))
                    .map_err(|reason| bad(OCTANT_VERTS, reason))?;
                let length = count
                    .checked_mul(4)
                    .ok_or_else(|| bad(OCTANT_VERTS, Unresolvable::Null))?;
                image
                    .slice_at(run, length)
                    .map_err(|reason| bad(OCTANT_VERTS, reason))?
                    .to_vec()
            };
            runs.push(Value::Bytes(bytes));
        }
        Ok(Value::Blob {
            codec: OCTANT_MAP.to_string(),
            parts: vec![(
                RUNS.to_string(),
                Value::Array {
                    elements: runs,
                    capacity: None,
                },
            )],
        })
    }
}

impl BlobWriter for OctantMapBlob {
    fn write(
        &self,
        heap: &mut dyn BlobHeap,
        owner: Owner<'_>,
        value: &Value,
    ) -> Result<Placement, BuildError> {
        let octants = octants_of(value)
            .filter(|runs| runs.len() == OCTANT_COUNT)
            .ok_or_else(|| BuildError::Blob {
                type_name: owner.ty.name.clone(),
                field: OCTANT_VERTS.to_string(),
                message: format!("the value is not a `{OCTANT_MAP}` of {OCTANT_COUNT} runs"),
            })?;
        let runs: usize = octants.iter().map(|run| 4 * run.len()).sum();
        let mut bytes = vec![0u8; COUNTS_BYTES + TABLE_BYTES + runs];
        let block = heap.alloc(bytes.len(), 16);
        let base = heap.offset_of(block);
        let mut next = COUNTS_BYTES + TABLE_BYTES;
        let mut table = Vec::new();
        for (i, run) in octants.iter().enumerate() {
            let count = u32::try_from(run.len()).map_err(|_| BuildError::Blob {
                type_name: owner.ty.name.clone(),
                field: OCTANT_VERTS.to_string(),
                message: format!("octant {i} holds {} vertices", run.len()),
            })?;
            bytes[4 * i..4 * i + 4].copy_from_slice(&count.to_le_bytes());
            if run.is_empty() {
                continue;
            }
            table.push((COUNTS_BYTES + 8 * i, block + next as u64));
            for v in run {
                bytes[next..next + 4].copy_from_slice(&v.to_le_bytes());
                next += 4;
            }
        }
        heap.put(base, &bytes)?;
        for (slot, target) in table {
            heap.put(base + slot, &target.to_le_bytes())?;
            heap.fixup(base + slot, target)?;
        }
        Ok(Placement {
            address: block + COUNTS_BYTES as u64,
            siblings: vec![(OCTANT_VERT_COUNTS.to_string(), block)],
        })
    }

    fn writes_siblings(&self) -> &'static [&'static str] {
        &[OCTANT_VERT_COUNTS]
    }
}

/// Why a bound would be misread by the game, and where.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Broken {
    #[error("{at}polygon {index}'s area bits end {bits:03b}, which names no primitive: the low three bits are the type tag")]
    UnknownPrimitive { at: String, index: usize, bits: u32 },
    #[error("{at}polygon {index} names vertex {vertex} of {vertices}")]
    Vertex {
        at: String,
        index: usize,
        vertex: usize,
        vertices: usize,
    },
    #[error("{at}polygon {index} names neighbour {neighbour} of {polygons}")]
    Neighbour {
        at: String,
        index: usize,
        neighbour: u16,
        polygons: usize,
    },
    #[error("{at}`vertex_attribs` is read as one word per vertex, and `num_per_vertex_attribs` states {per}")]
    VertexAttribs { at: String, per: u8 },
    #[error(
        "{at}the shrunk hull holds {hull} vertices against {vertices}, and it is one per vertex"
    )]
    ShrunkHull {
        at: String,
        hull: usize,
        vertices: usize,
    },
    #[error("{at}a polygon names material {material} of {materials}")]
    Material {
        at: String,
        material: u8,
        materials: usize,
    },
    #[error("{at}BVH leaf {index} covers items {first}..{end} of {items}")]
    Leaf {
        at: String,
        index: usize,
        first: usize,
        end: usize,
        items: usize,
    },
    #[error("{at}BVH subtree {index} names nodes {first}..{last} of {nodes}")]
    Subtree {
        at: String,
        index: usize,
        first: u16,
        last: u16,
        nodes: usize,
    },
    #[error("{at}{what} holds {count} in an allocation stated at {capacity}")]
    Capacity {
        at: String,
        what: &'static str,
        count: usize,
        capacity: usize,
    },
    #[error("{0}")]
    View(#[from] ViewError),
}

/// Every bound reachable from `root`, checked for what the game assumes and never checks.
pub fn validate(image: Image<'_>, root: usize) -> Result<(), Broken> {
    check_bound(View::<Raw<PhBound>>::at_offset(image, root)?, "")
}

/// A bound reached some other way: a dictionary entry, a composite's child.
pub fn check_bound(bound: View<'_, Raw<PhBound>>, at: &str) -> Result<(), Broken> {
    if let Some(g) = View::variant::<Raw<PhBoundGeometry>>(&bound)? {
        check_geometry(&g, at)?;
    }
    if let Some(b) = View::variant::<Raw<PhBoundBVH>>(&bound)? {
        let g = b.base();
        check_geometry(&g, at)?;
        if let Some(tree) = b.bvh()? {
            check_tree(&tree, g.num_polygons.max(0) as usize, at)?;
        }
    }
    if let Some(c) = View::variant::<Raw<PhBoundComposite>>(&bound)? {
        if c.num_bounds > c.max_num_bounds {
            return Err(Broken::Capacity {
                at: at.to_string(),
                what: "the composite",
                count: c.num_bounds.into(),
                capacity: c.max_num_bounds.into(),
            });
        }
        let children = c.bounds()?;
        for i in 0..children.len() {
            if let Some(child) = children.follow(i)? {
                check_bound(child, &format!("{at}child {i}: "))?;
            }
        }
        if let Some(tree) = c.bvh_structure()? {
            check_tree(&tree, children.len(), at)?;
        }
    }
    Ok(())
}

fn check_geometry(g: &View<'_, Raw<PhBoundGeometry>>, at: &str) -> Result<(), Broken> {
    if !{ g.vertex_attribs }.is_null() && g.num_per_vertex_attribs != 1 {
        return Err(Broken::VertexAttribs {
            at: at.to_string(),
            per: g.num_per_vertex_attribs,
        });
    }
    let vertices = g.num_vertices.max(0) as usize;
    let at = || at.to_string();
    // ⚠ A null hull keeps its count, so only a hull that is there is held to the vertices.
    let hull = usize::from(g.num_convex_hull_vertices);
    if !{ g.compressed_shrunk_vertices }.is_null() && hull != vertices {
        return Err(Broken::ShrunkHull {
            at: at(),
            hull,
            vertices,
        });
    }
    let polygons = g.polygons()?;
    for (index, p) in polygons.views().enumerate() {
        let vertex_list: Vec<usize> = match { p.area }.to_bits() & PRIMITIVE_TYPE_MASK {
            0 => p
                .vertex_indices
                .iter()
                .map(|v| (v & VERTEX_INDEX_MASK) as usize)
                .collect(),
            1 => {
                vec![View::variant::<Raw<PhPrimSphere>>(&p)?.map_or(0, |s| s.center_index as usize)]
            }
            2 | 4 => {
                let c = View::<Raw<PhPrimCapsule>>::at_offset(View::image(&p), View::offset(&p))?;
                vec![c.end_index0 as usize, c.end_index1 as usize]
            }
            3 => View::variant::<Raw<PhPrimBox>>(&p)?
                .map(|b| {
                    b.vertex_indices
                        .iter()
                        .map(|v| (v & VERTEX_INDEX_MASK) as usize)
                        .collect()
                })
                .unwrap_or_default(),
            bits => {
                return Err(Broken::UnknownPrimitive {
                    at: at(),
                    index,
                    bits,
                })
            }
        };
        if let Some(vertex) = vertex_list.into_iter().find(|v| *v >= vertices) {
            return Err(Broken::Vertex {
                at: at(),
                index,
                vertex,
                vertices,
            });
        }
        if { p.area }.to_bits() & PRIMITIVE_TYPE_MASK == 0 {
            if let Some(&neighbour) = p
                .neighboring_polygons
                .iter()
                .find(|n| **n != NO_NEIGHBOUR && **n as usize >= polygons.len())
            {
                return Err(Broken::Neighbour {
                    at: at(),
                    index,
                    neighbour,
                    polygons: polygons.len(),
                });
            }
        }
    }
    let materials = g.num_materials as usize;
    if let Ok(indices) = g.poly_mat_index_list() {
        if let Some(&material) = indices.iter().filter(|m| **m as usize >= materials).max() {
            return Err(Broken::Material {
                at: at(),
                material,
                materials,
            });
        }
    }
    Ok(())
}

fn check_tree(tree: &View<'_, Raw<PhOptimizedBvh>>, items: usize, at: &str) -> Result<(), Broken> {
    let at = || at.to_string();
    if tree.num_nodes_in_use > tree.num_nodes {
        return Err(Broken::Capacity {
            at: at(),
            what: "the BVH",
            count: tree.num_nodes_in_use.max(0) as usize,
            capacity: tree.num_nodes.max(0) as usize,
        });
    }
    let nodes = tree.contiguous_nodes()?;
    for (index, n) in nodes.iter().enumerate() {
        if n.polygon_count > 0 {
            let (first, end) = (
                n.node_data as usize,
                n.node_data as usize + n.polygon_count as usize,
            );
            if end > items {
                return Err(Broken::Leaf {
                    at: at(),
                    index,
                    first,
                    end,
                    items,
                });
            }
        }
    }
    for (index, s) in tree.subtree_headers()?.iter().enumerate() {
        if s.last_index as usize > nodes.len() || s.root_node_index > s.last_index {
            return Err(Broken::Subtree {
                at: at(),
                index,
                first: s.root_node_index,
                last: s.last_index,
                nodes: nodes.len(),
            });
        }
    }
    Ok(())
}
