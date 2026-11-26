//! The source of a `.ybn`: its triangles as a glb of named nodes, what a glb cannot say as one JSON document, and the file it came from.
//!
//! `bounds.json` states the tree of bounds by the engine's names: every bound's header (its flags,
//! its box and margin, its centroid, its two material words), a capsule's half height, a composite's
//! children with their matrices, boxes and flags, and for a polygon soup its materials by their game
//! values and each polygon that is no triangle (a sphere, a capsule, a box or a cylinder) by the
//! points it stands on. `bounds.glb` holds one node for each soup that holds a triangle, named in the
//! document as `node`, with one primitive for each material its triangles use.
//!
//! The base is the extracted file, kept as `base.ybn`. A compile opens it and sets what the document
//! states otherwise, so an untouched source compiles to the canonical save of the base. A soup whose
//! triangles and primitives are the base's keeps the base's polygons, vertices and tree; one that
//! changed is built again in the base's quantum where the mesh still fits it, and keeps the base's
//! box while its polygons stay inside it.
//!
//! ⚠ What follows from the mesh is not stated and stays the base's: the quantum, the trees, the
//! shrunk hull and the octant map, so a changed mesh is refused by name for a bound that holds a
//! hull or an octant map. Nor are the two runs a bound holds one value of for each vertex, its
//! attributes and its second-surface displacements: each value follows the vertex of the base it
//! was held for, and a mesh that gained a vertex the base does not hold is refused by name.
//!
//! ```text
//! <name>.bounds.src/    asset.json  base.ybn  bounds.json
//!                       bounds.glb                            (when a bound holds a triangle)
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use or_interchange::gltf::read::{load, strip_copy_suffix};
use or_interchange::gltf::{write, Container};
use or_interchange::scene::{Material, Mesh, Object, Primitive, Scene};
use or_io::decl::{Bool8, File, Plain, Raw, Shared};
use or_io::gta5::bounds::build::{self, BoundMesh, COMPOSITE_ITEM_THRESHOLD};
use or_io::gta5::bounds::{
    Color32, CompressedVertex, LocalBoxMinMax, PhBound, PhBoundAny, PhBoundComposite,
    PhBoundGeometry, PhMaterialMgrId, PhPolygon, PhPrimBox, PhPrimCapsule, PhPrimCylinder,
    PhPrimSphere, TypeAndIncludeFlags, NO_NEIGHBOUR, PRIMITIVE_TYPE_MASK, VERTEX_INDEX_MASK,
};
use or_io::gta5::{Mat34V, Vec3V, Vec4V};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::asset::{folder_name, sha256_hex, AssetManifest, How, Origin, Source, MANIFEST};
use crate::document::{hash_from, hash_text, list, member, root, stated, word, Fault, Json, Trail};
use crate::json::canonical;
use crate::models::{problem, target_of, Compiled};
use crate::textures::Extraction;
use crate::Problem;

pub mod edit;

/// The `kind` of a bound's source.
pub const KIND: &str = "bounds";
/// The document of the bounds, which a person edits.
pub const RULES: &str = "bounds.json";
/// The triangles of the bounds, which a modelling tool edits.
pub const MODEL: &str = "bounds.glb";
/// The bound the document and the glb were written from.
pub const BASE: &str = "base.ybn";
/// How the document names the bound at the root of the file; a composite's child `i` is `<its parent>_child<i>`.
pub const ROOT: &str = "bound";

/// The largest word a vertex is quantized to on either side of its bound's centre.
const QUANTIZED_LIMIT: f32 = 32767.0;

/// The `compile` block of `asset.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compile {
    /// The SHA-256 of the file kept as the base.
    base: String,
}

/// Everything a compile reads, so a recook can tell nothing changed before it opens anything.
#[derive(Clone)]
pub struct Inputs {
    manifest: AssetManifest,
    compile: Compile,
    base: Vec<u8>,
    rules: Vec<u8>,
    /// None for a source none of whose bounds holds a triangle.
    model: Option<Vec<u8>>,
}

/// A polygon that is no triangle, by the points it stands on.
#[derive(Debug, Clone, Copy)]
enum Shape {
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    Capsule {
        end0: [f32; 3],
        end1: [f32; 3],
        radius: f32,
    },
    Box {
        vertices: [[f32; 3]; 4],
    },
    Cylinder {
        end0: [f32; 3],
        end1: [f32; 3],
        radius: f32,
    },
}

impl Shape {
    /// The points it stands on and its radius, in the order its record holds them.
    fn parts(&self) -> (Vec<[f32; 3]>, f32) {
        match *self {
            Self::Sphere { center, radius } => (vec![center], radius),
            Self::Capsule { end0, end1, radius } | Self::Cylinder { end0, end1, radius } => {
                (vec![end0, end1], radius)
            }
            Self::Box { vertices } => (vertices.to_vec(), 0.0),
        }
    }

    /// The tag its polygon carries in the low bits of its first word.
    fn tag(&self) -> u8 {
        match self {
            Self::Sphere { .. } => 1,
            Self::Capsule { .. } => 2,
            Self::Box { .. } => 3,
            Self::Cylinder { .. } => 4,
        }
    }

    fn word(&self) -> &'static str {
        match self {
            Self::Sphere { .. } => "sphere",
            Self::Capsule { .. } => "capsule",
            Self::Box { .. } => "box",
            Self::Cylinder { .. } => "cylinder",
        }
    }
}

/// A shape and the material slot of its polygon.
#[derive(Debug, Clone, Copy)]
struct Prim {
    material: Option<u8>,
    shape: Shape,
}

impl Prim {
    /// Whether both state the same numbers, bit for bit.
    fn same(&self, other: &Self) -> bool {
        let (a, ra) = self.shape.parts();
        let (b, rb) = other.shape.parts();
        self.material == other.material
            && self.shape.tag() == other.shape.tag()
            && ra.to_bits() == rb.to_bits()
            && a.len() == b.len()
            && a.iter().zip(&b).all(|(a, b)| bits(*a) == bits(*b))
    }
}

/// A triangle as a glb states it: its material slot and its corners.
type Triangle = (Option<u8>, [[f32; 3]; 3]);

/// What a soup of the base holds: its triangles by vertex index, and its other polygons.
struct Soup {
    triangles: Vec<(Option<u8>, [usize; 3])>,
    primitives: Vec<Prim>,
    /// The place in the soup of the polygon each of `primitives` is.
    primitive_polygons: Vec<usize>,
}

fn bits(p: [f32; 3]) -> [u32; 3] {
    p.map(f32::to_bits)
}

fn words(v: &CompressedVertex) -> [i16; 3] {
    [v.x, v.y, v.z]
}

fn number(x: f32) -> Value {
    x.write()
}

fn three(p: [f32; 3]) -> Value {
    Value::Array(p.iter().map(|x| x.write()).collect())
}

fn xyz(v: &Vec4V) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// The member `name` of `o` as three numbers.
fn three_of(o: &Map<String, Value>, name: &str, at: &mut Trail) -> Result<[f32; 3], Fault> {
    let lanes: Vec<f32> = member(o, name, at)?;
    <[f32; 3]>::try_from(lanes)
        .map_err(|_| at.field(name, |at| at.fault("is not a list of three numbers")))
}

/// The name of the layout a bound is, which is its declaration's.
fn type_name(bound: &PhBoundAny) -> &'static str {
    match bound {
        PhBoundAny::PhBoundSphere(_) => "PhBoundSphere",
        PhBoundAny::PhBoundCapsule(_) => "PhBoundCapsule",
        PhBoundAny::PhBoundBox(_) => "PhBoundBox",
        PhBoundAny::PhBoundGeometry(_) => "PhBoundGeometry",
        PhBoundAny::PhBoundBVH(_) => "PhBoundBVH",
        PhBoundAny::PhBoundComposite(_) => "PhBoundComposite",
        PhBoundAny::PhBoundDisc(_) => "PhBoundDisc",
        PhBoundAny::PhBoundCylinder(_) => "PhBoundCylinder",
        PhBoundAny::PhBoundPlane(_) => "PhBoundPlane",
        PhBoundAny::Base(_) => "PhBound",
    }
}

fn header(bound: &PhBoundAny) -> &PhBound {
    match bound {
        PhBoundAny::PhBoundSphere(b) => &b.base,
        PhBoundAny::PhBoundCapsule(b) => &b.base,
        PhBoundAny::PhBoundBox(b) => &b.base,
        PhBoundAny::PhBoundGeometry(b) => &b.base,
        PhBoundAny::PhBoundBVH(b) => &b.base.base,
        PhBoundAny::PhBoundComposite(b) => &b.base,
        PhBoundAny::PhBoundDisc(b) => &b.base,
        PhBoundAny::PhBoundCylinder(b) => &b.base,
        PhBoundAny::PhBoundPlane(b) => &b.base,
        PhBoundAny::Base(b) => b,
    }
}

fn header_mut(bound: &mut PhBoundAny) -> &mut PhBound {
    match bound {
        PhBoundAny::PhBoundSphere(b) => &mut b.base,
        PhBoundAny::PhBoundCapsule(b) => &mut b.base,
        PhBoundAny::PhBoundBox(b) => &mut b.base,
        PhBoundAny::PhBoundGeometry(b) => &mut b.base,
        PhBoundAny::PhBoundBVH(b) => &mut b.base.base,
        PhBoundAny::PhBoundComposite(b) => &mut b.base,
        PhBoundAny::PhBoundDisc(b) => &mut b.base,
        PhBoundAny::PhBoundCylinder(b) => &mut b.base,
        PhBoundAny::PhBoundPlane(b) => &mut b.base,
        PhBoundAny::Base(b) => b,
    }
}

/// The members every bound states, which are its header's.
const HEADER: [&str; 12] = [
    "type",
    "flags",
    "part_index",
    "radius_around_centroid",
    "bounding_box_max",
    "margin",
    "bounding_box_min",
    "centroid_offset",
    "material_id0",
    "cg_offset",
    "material_id1",
    "volume_distribution",
];
/// The members a polygon soup states besides its header.
const SOUP: [&str; 6] = [
    "use_active_components",
    "is_flat",
    "node",
    "materials",
    "material_colors",
    "primitives",
];
/// The members a child of a composite states.
const CHILD: [&str; 9] = [
    "bound",
    "current_matrix",
    "last_matrix",
    "local_box_min",
    "local_box_max",
    "type_flags",
    "include_flags",
    "owned_type_flags",
    "owned_include_flags",
];

fn header_document(bound: &PhBoundAny) -> Map<String, Value> {
    let h = header(bound);
    let mut o = Map::new();
    o.insert("type".into(), type_name(bound).into());
    o.insert("flags".into(), h.flags.into());
    o.insert("part_index".into(), h.part_index.into());
    o.insert(
        "radius_around_centroid".into(),
        number(h.radius_around_centroid),
    );
    o.insert(
        "bounding_box_max".into(),
        three(xyz(&h.bounding_box_max_xyz_margin_w)),
    );
    o.insert("margin".into(), number(h.bounding_box_max_xyz_margin_w.w));
    o.insert("bounding_box_min".into(), three(h.bounding_box_min()));
    o.insert(
        "centroid_offset".into(),
        three(xyz(&h.centroid_offset_xyz_material_id0_w)),
    );
    o.insert(
        "material_id0".into(),
        hash_text(h.centroid_offset_xyz_material_id0_w.w.to_bits()),
    );
    o.insert(
        "cg_offset".into(),
        three(xyz(&h.cg_offset_xyz_material_id1_w)),
    );
    o.insert(
        "material_id1".into(),
        hash_text(h.cg_offset_xyz_material_id1_w.w.to_bits()),
    );
    o.insert("volume_distribution".into(), h.volume_distribution.write());
    o
}

/// Sets on `h` each header value `o` states otherwise than `was` holds it, and says which.
fn set_header(
    o: &Map<String, Value>,
    was: &PhBound,
    h: &mut PhBound,
    address: &str,
    report: &mut Vec<String>,
    at: &mut Trail,
) -> Result<(), Fault> {
    let mut set = |what: &str| report.push(format!("{address}: {what} set"));
    let flags: u8 = member(o, "flags", at)?;
    if flags != was.flags {
        h.flags = flags;
        set("flags");
    }
    let part_index: u16 = member(o, "part_index", at)?;
    if part_index != was.part_index {
        h.part_index = part_index;
        set("part_index");
    }
    let radius: f32 = member(o, "radius_around_centroid", at)?;
    if radius.to_bits() != was.radius_around_centroid.to_bits() {
        h.radius_around_centroid = radius;
        set("radius_around_centroid");
    }
    let max = three_of(o, "bounding_box_max", at)?;
    if bits(max) != bits(xyz(&was.bounding_box_max_xyz_margin_w)) {
        let v = &mut h.bounding_box_max_xyz_margin_w;
        [v.x, v.y, v.z] = max;
        set("bounding_box_max");
    }
    let margin: f32 = member(o, "margin", at)?;
    if margin.to_bits() != was.bounding_box_max_xyz_margin_w.w.to_bits() {
        h.bounding_box_max_xyz_margin_w.w = margin;
        set("margin");
    }
    let min = three_of(o, "bounding_box_min", at)?;
    if bits(min) != bits(was.bounding_box_min()) {
        h.set_bounding_box_min(min);
        set("bounding_box_min");
    }
    let centroid = three_of(o, "centroid_offset", at)?;
    if bits(centroid) != bits(xyz(&was.centroid_offset_xyz_material_id0_w)) {
        let v = &mut h.centroid_offset_xyz_material_id0_w;
        [v.x, v.y, v.z] = centroid;
        set("centroid_offset");
    }
    let material = word(o, "material_id0", at)?;
    if material != was.centroid_offset_xyz_material_id0_w.w.to_bits() {
        h.centroid_offset_xyz_material_id0_w.w = f32::from_bits(material);
        set("material_id0");
    }
    let cg = three_of(o, "cg_offset", at)?;
    if bits(cg) != bits(xyz(&was.cg_offset_xyz_material_id1_w)) {
        let v = &mut h.cg_offset_xyz_material_id1_w;
        [v.x, v.y, v.z] = cg;
        set("cg_offset");
    }
    let material = word(o, "material_id1", at)?;
    if material != was.cg_offset_xyz_material_id1_w.w.to_bits() {
        h.cg_offset_xyz_material_id1_w.w = f32::from_bits(material);
        set("material_id1");
    }
    let volume: Vec4V = member(o, "volume_distribution", at)?;
    let lanes = |v: &Vec4V| [v.x, v.y, v.z, v.w].map(f32::to_bits);
    if lanes(&volume) != lanes(&was.volume_distribution) {
        let v = &mut h.volume_distribution;
        [v.x, v.y, v.z, v.w] = [volume.x, volume.y, volume.z, volume.w];
        set("volume_distribution");
    }
    Ok(())
}

/// The triangles and the other polygons of `g`, refused where a polygon cannot be told.
fn soup_of(g: &PhBoundGeometry) -> Result<Soup, String> {
    let mut soup = Soup {
        triangles: Vec::new(),
        primitives: Vec::new(),
        primitive_polygons: Vec::new(),
    };
    for (i, p) in g.polygons.iter().enumerate() {
        let material = g.poly_mat_index_list.get(i).copied();
        let (indices, radius) = g
            .polygon_vertices(p)
            .ok_or_else(|| format!("polygon {i} carries a tag that names no primitive"))?;
        let outside = || format!("polygon {i} names a vertex its bound does not hold");
        let at = |k: usize| {
            indices
                .get(k)
                .and_then(|v| g.vertex_position(*v))
                .ok_or_else(outside)
        };
        let shape = match p.primitive() {
            0 => {
                let corners = <[usize; 3]>::try_from(indices.as_slice()).map_err(|_| outside())?;
                if corners.iter().any(|v| *v >= g.compressed_vertices.len()) {
                    return Err(outside());
                }
                soup.triangles.push((material, corners));
                continue;
            }
            1 => Shape::Sphere {
                center: at(0)?,
                radius,
            },
            2 => Shape::Capsule {
                end0: at(0)?,
                end1: at(1)?,
                radius,
            },
            3 => Shape::Box {
                vertices: [at(0)?, at(1)?, at(2)?, at(3)?],
            },
            _ => Shape::Cylinder {
                end0: at(0)?,
                end1: at(1)?,
                radius,
            },
        };
        soup.primitives.push(Prim { material, shape });
        soup.primitive_polygons.push(i);
    }
    Ok(soup)
}

/// The name the glb gives the material slot `slot` of the bound at `address`.
fn material_name(address: &str, slot: usize) -> String {
    format!("{address}_material{slot}")
}

/// What an extraction gathers for the glb while it writes the document.
struct Writing<'a> {
    name: &'a str,
    meshes: Vec<Mesh>,
    materials: Vec<Material>,
}

fn primitive_document(prim: &Prim) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), prim.shape.word().into());
    o.insert(
        "material".into(),
        prim.material.map_or(Value::Null, Value::from),
    );
    match prim.shape {
        Shape::Sphere { center, radius } => {
            o.insert("center".into(), three(center));
            o.insert("radius".into(), number(radius));
        }
        Shape::Capsule { end0, end1, radius } | Shape::Cylinder { end0, end1, radius } => {
            o.insert("end0".into(), three(end0));
            o.insert("end1".into(), three(end1));
            o.insert("radius".into(), number(radius));
        }
        Shape::Box { vertices } => {
            o.insert(
                "vertices".into(),
                Value::Array(vertices.iter().map(|v| three(*v)).collect()),
            );
        }
    }
    Value::Object(o)
}

/// Adds what the soup `g` states to its bound's document, and its triangles to the glb.
fn soup_document(
    g: &PhBoundGeometry,
    address: &str,
    o: &mut Map<String, Value>,
    out: &mut Writing<'_>,
) -> Result<(), String> {
    let soup = soup_of(g).map_err(|why| format!("{address}: {why}"))?;
    o.insert(
        "use_active_components".into(),
        g.use_active_components.write(),
    );
    o.insert("is_flat".into(), g.is_flat.write());
    let materials: Vec<Value> = g
        .material_ids
        .iter()
        .enumerate()
        .map(|(i, material)| {
            json!({
                "name": material_name(address, i),
                "id": format!("{:#018x}", material.id),
            })
        })
        .collect();
    o.insert("materials".into(), materials.into());
    o.insert(
        "material_colors".into(),
        g.material_colors
            .iter()
            .map(|c| hash_text(c.color))
            .collect::<Vec<_>>()
            .into(),
    );
    o.insert(
        "primitives".into(),
        soup.primitives
            .iter()
            .map(primitive_document)
            .collect::<Vec<_>>()
            .into(),
    );
    if soup.triangles.is_empty() {
        return Ok(());
    }

    let mut by_material: BTreeMap<Option<u8>, Vec<[usize; 3]>> = BTreeMap::new();
    for (material, corners) in &soup.triangles {
        by_material.entry(*material).or_default().push(*corners);
    }
    let mut mesh = Mesh {
        name: address.to_string(),
        ..Mesh::default()
    };
    for (material, triangles) in by_material {
        let mut primitive = Primitive {
            material: material.map(|slot| {
                out.materials.push(Material {
                    name: material_name(address, usize::from(slot)),
                    ..Material::default()
                });
                out.materials.len() - 1
            }),
            ..Primitive::default()
        };
        let mut local: HashMap<usize, u32> = HashMap::new();
        for corner in triangles.iter().flatten() {
            let next = local.len() as u32;
            let index = *local.entry(*corner).or_insert(next);
            if index == next {
                // Held by the soup: `soup_of` refused a corner that is not.
                primitive
                    .positions
                    .push(g.vertex_position(*corner).unwrap_or_default());
            }
            primitive.indices.push(index);
        }
        mesh.primitives.push(primitive);
    }
    o.insert("node".into(), format!("{}_{address}", out.name).into());
    out.meshes.push(mesh);
    Ok(())
}

/// The document of `bound`, whose triangles go to the glb under the name its `node` states.
fn bound_document(
    bound: &PhBoundAny,
    address: &str,
    out: &mut Writing<'_>,
) -> Result<Value, String> {
    let mut o = header_document(bound);
    match bound {
        PhBoundAny::PhBoundCapsule(capsule) => {
            o.insert(
                "capsule_half_height".into(),
                number(capsule.capsule_half_height),
            );
        }
        PhBoundAny::PhBoundGeometry(g) => soup_document(g, address, &mut o, out)?,
        PhBoundAny::PhBoundBVH(b) => soup_document(&b.base, address, &mut o, out)?,
        PhBoundAny::PhBoundComposite(composite) => {
            let children = (0..composite.bounds.len())
                .map(|i| child_document(composite, i, address, out))
                .collect::<Result<Vec<_>, _>>()?;
            o.insert("children".into(), children.into());
        }
        _ => {}
    }
    Ok(Value::Object(o))
}

fn matrix_document(m: &Mat34V) -> Value {
    Value::Array(vec![
        m.col0.write(),
        m.col1.write(),
        m.col2.write(),
        m.col3.write(),
    ])
}

fn child_document(
    composite: &PhBoundComposite,
    i: usize,
    address: &str,
    out: &mut Writing<'_>,
) -> Result<Value, String> {
    let mut o = Map::new();
    let bound = match composite.bounds[i].as_deref() {
        Some(child) => bound_document(child, &format!("{address}_child{i}"), out)?,
        None => Value::Null,
    };
    o.insert("bound".into(), bound);
    if let Some(m) = composite.current_matrices.get(i) {
        o.insert("current_matrix".into(), matrix_document(m));
    }
    if let Shared::Own(matrices) = &composite.last_matrices {
        if let Some(m) = matrices.get(i) {
            o.insert("last_matrix".into(), matrix_document(m));
        }
    }
    if let Some(b) = composite.local_box_min_maxs.get(i) {
        o.insert("local_box_min".into(), b.min.write());
        o.insert("local_box_max".into(), b.max.write());
    }
    if let Some(f) = composite.type_and_include_flags.get(i) {
        o.insert("type_flags".into(), hash_text(f.type_flags));
        o.insert("include_flags".into(), hash_text(f.include_flags));
    }
    if let Shared::Own(flags) = &composite.owned_type_and_include_flags {
        if let Some(f) = flags.get(i) {
            o.insert("owned_type_flags".into(), hash_text(f.type_flags));
            o.insert("owned_include_flags".into(), hash_text(f.include_flags));
        }
    }
    Ok(Value::Object(o))
}

/// What a compile reads the glb as, and what it did.
struct Reading<'a> {
    /// The meshes of the glb by name, without a tool's copy suffix; none when the source holds no glb.
    meshes: HashMap<String, &'a Mesh>,
    used: HashSet<String>,
    report: Vec<String>,
}

fn matrix_of(v: &Value, was: Option<&Mat34V>, at: &mut Trail) -> Result<Mat34V, Fault> {
    let mut columns: Vec<Vec4V> = Vec::new();
    columns.read(v, at)?;
    let [col0, col1, col2, col3] =
        <[Vec4V; 4]>::try_from(columns).map_err(|_| at.fault("is not a list of four columns"))?;
    let mut m = was.cloned().unwrap_or_default();
    let put =
        |to: &mut Vec4V, from: Vec4V| [to.x, to.y, to.z, to.w] = [from.x, from.y, from.z, from.w];
    put(&mut m.col0, col0);
    put(&mut m.col1, col1);
    put(&mut m.col2, col2);
    put(&mut m.col3, col3);
    Ok(m)
}

/// What every child states of one run of its composite, or `None` where no child states it.
fn run_of<T>(stated: Vec<Option<T>>, key: &str, at: &Trail) -> Result<Option<Vec<T>>, Fault> {
    let children = stated.len();
    let run: Vec<T> = stated.into_iter().flatten().collect();
    if run.is_empty() {
        Ok(None)
    } else if run.len() == children {
        Ok(Some(run))
    } else {
        Err(at.fault(format!(
            "states `{key}` for {} of its {children} children, and a composite holds one for each child or none",
            run.len()
        )))
    }
}

fn box_of(h: &PhBound) -> ([f32; 3], [f32; 3]) {
    (h.bounding_box_min(), xyz(&h.bounding_box_max_xyz_margin_w))
}

/// The two members `a` and `b` of `o`, which are stated together or not at all.
fn pair_of<'v>(
    o: &'v Map<String, Value>,
    a: &str,
    b: &str,
    at: &Trail,
) -> Result<Option<(&'v Value, &'v Value)>, Fault> {
    match (o.get(a), o.get(b)) {
        (Some(a), Some(b)) => Ok(Some((a, b))),
        (None, None) => Ok(None),
        _ => Err(at.fault(format!("states one of `{a}` and `{b}` without the other"))),
    }
}

fn flags_of(
    o: &Map<String, Value>,
    type_flags: &str,
    include_flags: &str,
    was: Option<&TypeAndIncludeFlags>,
    at: &mut Trail,
) -> Result<Option<TypeAndIncludeFlags>, Fault> {
    if pair_of(o, type_flags, include_flags, at)?.is_none() {
        return Ok(None);
    }
    let mut flags = was.cloned().unwrap_or_default();
    flags.type_flags = word(o, type_flags, at)?;
    flags.include_flags = word(o, include_flags, at)?;
    Ok(Some(flags))
}

fn local_box_of(
    o: &Map<String, Value>,
    was: Option<&LocalBoxMinMax>,
    at: &mut Trail,
) -> Result<Option<LocalBoxMinMax>, Fault> {
    if pair_of(o, "local_box_min", "local_box_max", at)?.is_none() {
        return Ok(None);
    }
    let min: Vec3V = member(o, "local_box_min", at)?;
    let max: Vec3V = member(o, "local_box_max", at)?;
    let mut b = was.cloned().unwrap_or_default();
    [b.min.x, b.min.y, b.min.z, b.min.w] = [min.x, min.y, min.z, min.w];
    [b.max.x, b.max.y, b.max.z, b.max.w] = [max.x, max.y, max.z, max.w];
    Ok(Some(b))
}

/// The run a composite shares with another or holds for itself, as its children state it.
fn shared_run<T>(stated: Option<Vec<T>>, was: &Shared<Vec<T>>) -> Shared<Vec<T>> {
    match (stated, was) {
        (Some(run), _) => Shared::Own(run),
        (None, Shared::Same) => Shared::Same,
        (None, Shared::Own(_)) => Shared::Own(Vec::new()),
    }
}

/// The children `o` states, laid over the composite `was`.
fn set_children(
    o: &Map<String, Value>,
    was: &PhBoundComposite,
    composite: &mut PhBoundComposite,
    address: &str,
    reading: &mut Reading<'_>,
    at: &mut Trail,
) -> Result<(), Fault> {
    let children = list(o, "children", at)?;
    at.field("children", |at| -> Result<(), Fault> {
        if children.len() != was.bounds.len() {
            return Err(at.fault(format!(
                "holds {} children and the base holds {}: adding or removing a child of a composite is not an edit this source compiles",
                children.len(),
                was.bounds.len()
            )));
        }
        let mut current = Vec::new();
        let mut last = Vec::new();
        let mut boxes = Vec::new();
        let mut flags = Vec::new();
        let mut owned = Vec::new();
        // The children whose box now reaches outside the box the base states for them.
        let mut outgrown = Vec::new();
        for (i, v) in children.iter().enumerate() {
            at.index(i, |at| -> Result<(), Fault> {
                let child = at.object(v)?;
                at.only(child, &CHILD)?;
                let name = format!("{address}_child{i}");
                composite.bounds[i] = match (stated(child, "bound", at)?, was.bounds[i].as_deref()) {
                    (Value::Null, None) => None,
                    (Value::Null, Some(_)) => {
                        return Err(at.fault(
                            "states no bound where the base holds one: removing a child of a composite is not an edit this source compiles",
                        ))
                    }
                    (_, None) => {
                        return Err(at.fault(
                            "states a bound where the base holds none: adding a child of a composite is not an edit this source compiles",
                        ))
                    }
                    (v, Some(base)) => {
                        let bound = at.field("bound", |at| set_bound(v, base, &name, reading, at))?;
                        let ((was_min, was_max), (min, max)) =
                            (box_of(header(base)), box_of(header(&bound)));
                        if (0..3).any(|a| min[a] < was_min[a] || max[a] > was_max[a]) {
                            outgrown.push(i);
                        }
                        Some(Box::new(bound))
                    }
                };
                current.push(
                    child
                        .get("current_matrix")
                        .map(|v| {
                            at.field("current_matrix", |at| {
                                matrix_of(v, was.current_matrices.get(i), at)
                            })
                        })
                        .transpose()?,
                );
                let was_last = match &was.last_matrices {
                    Shared::Own(run) => run.get(i),
                    Shared::Same => None,
                };
                last.push(
                    child
                        .get("last_matrix")
                        .map(|v| at.field("last_matrix", |at| matrix_of(v, was_last, at)))
                        .transpose()?,
                );
                boxes.push(local_box_of(child, was.local_box_min_maxs.get(i), at)?);
                flags.push(flags_of(
                    child,
                    "type_flags",
                    "include_flags",
                    was.type_and_include_flags.get(i),
                    at,
                )?);
                let was_owned = match &was.owned_type_and_include_flags {
                    Shared::Own(run) => run.get(i),
                    Shared::Same => None,
                };
                owned.push(flags_of(
                    child,
                    "owned_type_flags",
                    "owned_include_flags",
                    was_owned,
                    at,
                )?);
                Ok(())
            })?;
        }
        composite.current_matrices = run_of(current, "current_matrix", at)?.unwrap_or_default();
        composite.last_matrices = shared_run(run_of(last, "last_matrix", at)?, &was.last_matrices);
        composite.local_box_min_maxs = run_of(boxes, "local_box_min", at)?.unwrap_or_default();
        composite.type_and_include_flags = run_of(flags, "type_flags", at)?.unwrap_or_default();
        composite.owned_type_and_include_flags = shared_run(
            run_of(owned, "owned_type_flags", at)?,
            &was.owned_type_and_include_flags,
        );

        let lanes = |b: &LocalBoxMinMax| {
            [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z].map(f32::to_bits)
        };
        let moved: Vec<bool> = (0..composite.local_box_min_maxs.len())
            .map(|i| {
                composite.local_box_min_maxs.get(i).map(lanes)
                    != was.local_box_min_maxs.get(i).map(lanes)
            })
            .collect();
        // ⚠ A composite keeps a box of each child beside the child's own, and nothing here derives one from the other.
        if let Some(i) = outgrown
            .into_iter()
            .find(|i| moved.get(*i) == Some(&false))
        {
            return Err(at.index(i, |at| {
                at.fault(format!(
                    "holds a bound whose box now reaches outside the one the base states for {address}_child{i}, and the composite keeps a box of each child: state its `local_box_min` and `local_box_max`, and the composite's own box, for the new one"
                ))
            }));
        }
        if moved.contains(&true) && was.bvh_structure.is_some() {
            let items: Vec<([f32; 3], [f32; 3])> = composite
                .local_box_min_maxs
                .iter()
                .map(|b| ([b.min.x, b.min.y, b.min.z], [b.max.x, b.max.y, b.max.z]))
                .collect();
            let built = build::build(&items, COMPOSITE_ITEM_THRESHOLD)
                .map_err(|e| at.fault(format!("has boxes no tree is built over: {e}")))?;
            composite.bvh_structure = Some(Box::new(built.bvh));
            reading.report.push(format!(
                "{address}: the boxes of its children changed, so its tree was built again over them"
            ));
        }
        Ok(())
    })
}

/// The polygon a shape is stored as, standing on the vertices `index` gives its points; the bytes its record does not name are those of `like`.
///
/// ⚠ The staged `.ybn` hold other bytes than zero between the members of a primitive, so a primitive the base holds is written over its own polygon.
fn shape_polygon(shape: &Shape, index: &[u16], like: Option<&PhPolygon>) -> Option<PhPolygon> {
    let (_, radius) = shape.parts();
    let mut held = <Raw<PhPolygon> as Plain>::zeroed();
    if let Some(like) = like {
        held.area = like.area;
        held.vertex_indices = like.vertex_indices;
        held.neighboring_polygons = like.neighboring_polygons;
    }
    let held = Plain::as_bytes(&held);
    let tagged = |byte: u8| (byte & !(PRIMITIVE_TYPE_MASK as u8)) | shape.tag();
    let bytes: Vec<u8> = match shape {
        Shape::Sphere { .. } => {
            let mut raw = <Raw<PhPrimSphere> as Plain>::read_from(held)?;
            raw.prim_type = tagged(raw.prim_type);
            raw.center_index = *index.first()?;
            raw.radius = radius;
            Plain::as_bytes(&raw).to_vec()
        }
        Shape::Capsule { .. } => {
            let mut raw = <Raw<PhPrimCapsule> as Plain>::read_from(held)?;
            raw.prim_type = tagged(raw.prim_type);
            raw.end_index0 = *index.first()?;
            raw.end_index1 = *index.get(1)?;
            raw.radius = radius;
            Plain::as_bytes(&raw).to_vec()
        }
        Shape::Cylinder { .. } => {
            let mut raw = <Raw<PhPrimCylinder> as Plain>::read_from(held)?;
            raw.prim_type = tagged(raw.prim_type);
            raw.end_index0 = *index.first()?;
            raw.end_index1 = *index.get(1)?;
            raw.radius = radius;
            Plain::as_bytes(&raw).to_vec()
        }
        Shape::Box { .. } => {
            let mut raw = <Raw<PhPrimBox> as Plain>::read_from(held)?;
            raw.prim_type = tagged(raw.prim_type);
            let (stands_on, carried) = (<[u16; 4]>::try_from(index).ok()?, { raw.vertex_indices });
            raw.vertex_indices =
                std::array::from_fn(|k| stands_on[k] | (carried[k] & !VERTEX_INDEX_MASK));
            Plain::as_bytes(&raw).to_vec()
        }
    };
    let raw = <Raw<PhPolygon> as Plain>::read_from(&bytes)?;
    Some(PhPolygon {
        area: raw.area,
        vertex_indices: raw.vertex_indices,
        neighboring_polygons: raw.neighboring_polygons,
        ..Default::default()
    })
}

/// The word `p` is stored as in a quantum of `factor` about `centre`, or `None` where sixteen bits do not hold it.
fn quantized(p: [f32; 3], centre: [f32; 3], factor: [f32; 3]) -> Option<[i16; 3]> {
    let lane = |a: usize| {
        let q = ((p[a] - centre[a]) / factor[a]).round();
        (q.abs() <= QUANTIZED_LIMIT).then_some(q as i16)
    };
    Some([lane(0)?, lane(1)?, lane(2)?])
}

/// Whether the glb's corner `p` is the vertex `index` of `g`: the position that vertex decodes to, or one stored as its word.
fn is_vertex(g: &PhBoundGeometry, p: [f32; 3], index: usize) -> bool {
    let (f, c) = (&g.un_quantize_factor, &g.bounding_box_center);
    g.vertex_position(index).is_some_and(|q| q == p)
        || g.compressed_vertices
            .get(index)
            .is_some_and(|v| quantized(p, [c.x, c.y, c.z], [f.x, f.y, f.z]) == Some(words(v)))
}

/// Whether the triangles and primitives a source states are those `soup` holds, in its order within each material.
fn unchanged(
    g: &PhBoundGeometry,
    soup: &Soup,
    triangles: &[Triangle],
    primitives: &[Prim],
) -> bool {
    let mut held: Vec<&(Option<u8>, [usize; 3])> = soup.triangles.iter().collect();
    held.sort_by_key(|(material, _)| *material);
    primitives.len() == soup.primitives.len()
        && primitives
            .iter()
            .zip(&soup.primitives)
            .all(|(a, b)| a.same(b))
        && triangles.len() == held.len()
        && triangles.iter().zip(held).all(|(stated, held)| {
            stated.0 == held.0 && (0..3).all(|k| is_vertex(g, stated.1[k], held.1[k]))
        })
}

/// The run a bound holds one value of for each vertex, as the bits of each value.
fn of_each_vertex(g: &PhBoundGeometry) -> Vec<(&'static str, Vec<u32>)> {
    let mut runs = Vec::new();
    if !g.vertex_attribs.is_empty() {
        let values = g.vertex_attribs.iter().map(|c| c.color).collect();
        runs.push(("vertex attributes", values));
    }
    if !g.second_surface_vertex_displacements.is_empty() {
        let values = g
            .second_surface_vertex_displacements
            .iter()
            .map(|d| d.to_bits())
            .collect();
        runs.push(("second-surface displacements", values));
    }
    runs
}

/// The soup `was` with the polygons a source states, in the quantum of `was` where they fit it.
fn rebuilt(
    was: &PhBoundGeometry,
    soup: &Soup,
    triangles: &[Triangle],
    primitives: &[Prim],
) -> Result<PhBoundGeometry, String> {
    for (held, what) in [
        (!was.compressed_shrunk_vertices.is_empty(), "a shrunk hull"),
        (was.octant_verts.is_some(), "an octant map"),
    ] {
        if held {
            return Err(format!(
                "has polygons that changed and holds {what}, which follows its vertices one for one and which this build does not make again"
            ));
        }
    }
    let carried = of_each_vertex(was);
    if let Some((what, values)) = carried
        .iter()
        .find(|(_, values)| values.len() != was.compressed_vertices.len())
    {
        return Err(format!(
            "has polygons that changed and holds {} {what} for {} vertices, so which vertex each follows cannot be told",
            values.len(),
            was.compressed_vertices.len()
        ));
    }
    let points: Vec<[f32; 3]> = triangles
        .iter()
        .flat_map(|(_, corners)| corners.iter().copied())
        .chain(primitives.iter().flat_map(|p| p.shape.parts().0))
        .collect();
    if let Some(p) = points.iter().find(|p| p.iter().any(|x| !x.is_finite())) {
        return Err(format!("has a vertex at {p:?}, which is not a place"));
    }
    if points.is_empty() {
        return Err("holds no polygon, and a bound over nothing has no box".to_string());
    }
    // ⚠ The builder numbers a triangle's neighbours in sixteen bits and keeps the last value for none.
    let polygons = triangles.len() + primitives.len();
    if polygons >= usize::from(NO_NEIGHBOUR) {
        return Err(format!(
            "holds {polygons} polygons, and a polygon names its neighbour in sixteen bits, so a bound holds fewer than {NO_NEIGHBOUR}"
        ));
    }

    let (f, c) = (&was.un_quantize_factor, &was.bounding_box_center);
    let (mut centre, mut factor) = ([c.x, c.y, c.z], [f.x, f.y, f.z]);
    // A position a vertex of the base decodes to is that vertex, which matters where a float is coarser than the quantum.
    let mut known: HashMap<[u32; 3], [i16; 3]> = HashMap::new();
    // The vertex of the base stored as each word, and the words two vertices that differ in a carried value share.
    let mut vertex_of: HashMap<[i16; 3], usize> = HashMap::new();
    let mut torn: HashSet<[i16; 3]> = HashSet::new();
    let differ = |a: usize, b: usize| carried.iter().any(|(_, values)| values[a] != values[b]);
    for (i, v) in was.compressed_vertices.iter().enumerate() {
        let first = *vertex_of.entry(words(v)).or_insert(i);
        if differ(first, i) {
            torn.insert(words(v));
        }
        if let Some(p) = was.vertex_position(i) {
            // A corner at this place is read as the first word stored there.
            let word = *known.entry(bits(p)).or_insert(words(v));
            if vertex_of.get(&word).is_some_and(|first| differ(*first, i)) {
                torn.insert(word);
            }
        }
    }
    let in_the_base = |p: &[f32; 3], centre: [f32; 3], factor: [f32; 3]| {
        known
            .get(&bits(*p))
            .copied()
            .or_else(|| quantized(*p, centre, factor))
    };
    let outside = points
        .iter()
        .find(|p| in_the_base(p, centre, factor).is_none());
    // ⚠ A value held for each vertex follows the vertex of the base stored as the same word, and nothing states one for another vertex.
    let new_vertex = |p: &[f32; 3], what: &str| {
        format!(
            "has a vertex at {p:?} that the base does not hold, and the base holds {what} for each of its vertices, which nothing states for a new one"
        )
    };
    if let (Some(p), Some((what, _))) = (outside, carried.first()) {
        return Err(new_vertex(p, what));
    }
    let kept = outside.is_none();
    if !kept {
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &points {
            min = std::array::from_fn(|a| min[a].min(p[a]));
            max = std::array::from_fn(|a| max[a].max(p[a]));
        }
        centre = std::array::from_fn(|a| (min[a] + max[a]) * 0.5);
        // A zero extent stores every vertex on that axis at zero.
        // ⚠ Half the sum of two floats is often no float, so the centre is off the middle and one end is farther from it than half
        // the extent: the quantum is of the farther end, and is stepped up until both ends are words.
        factor = std::array::from_fn(|a| {
            let reach = (max[a] - centre[a]).max(centre[a] - min[a]);
            if reach > 0.0 {
                let mut factor = reach / QUANTIZED_LIMIT;
                let past = |end: f32, factor: f32| {
                    ((end - centre[a]) / factor).round().abs() > QUANTIZED_LIMIT
                };
                while factor.is_finite() && (past(min[a], factor) || past(max[a], factor)) {
                    factor = factor.next_up();
                }
                factor
            } else {
                1.0
            }
        });
    }

    let mut vertices: Vec<[i16; 3]> = Vec::new();
    let mut index_of: HashMap<[i16; 3], u16> = HashMap::new();
    let mut index = |p: &[f32; 3]| -> Result<u16, String> {
        let word = if kept {
            in_the_base(p, centre, factor)
        } else {
            quantized(*p, centre, factor)
        }
        .ok_or_else(|| {
            format!("has a vertex at {p:?}, which is outside what sixteen bits hold of its bound")
        })?;
        if let Some(i) = index_of.get(&word) {
            return Ok(*i);
        }
        if let Some((what, _)) = carried.first() {
            if !vertex_of.contains_key(&word) {
                return Err(new_vertex(p, what));
            }
            if torn.contains(&word) {
                return Err(format!(
                    "has a vertex at {p:?} where the base holds two vertices that differ in their {what}, so which one a corner stands on cannot be told"
                ));
            }
        }
        if vertices.len() > usize::from(VERTEX_INDEX_MASK) {
            return Err(format!(
                "holds more than {} vertices, and a polygon addresses a vertex in fifteen bits",
                usize::from(VERTEX_INDEX_MASK) + 1
            ));
        }
        let i = vertices.len() as u16;
        vertices.push(word);
        index_of.insert(word, i);
        Ok(i)
    };
    let mut corners: Vec<[u16; 3]> = Vec::with_capacity(triangles.len());
    for (_, stated) in triangles {
        corners.push([index(&stated[0])?, index(&stated[1])?, index(&stated[2])?]);
    }
    let mut stands_on: Vec<Vec<u16>> = Vec::with_capacity(primitives.len());
    for prim in primitives {
        let mut on = Vec::new();
        for p in &prim.shape.parts().0 {
            on.push(index(p)?);
        }
        stands_on.push(on);
    }

    let decoded: Vec<[f32; 3]> = vertices
        .iter()
        .map(|w| std::array::from_fn(|a| f32::from(w[a]) * factor[a] + centre[a]))
        .collect();
    // The areas and the adjacency of the triangles the base does not hold are the builder's.
    let mut polygons = build::geometry(&BoundMesh::new(decoded.clone(), corners.clone()))
        .map_err(|e| format!("holds a mesh no bound is built of: {e}"))?
        .polygons;
    if kept {
        // The triangles of the base by the words of their corners, each to be taken once.
        let mut held: HashMap<[[i16; 3]; 3], Vec<usize>> = HashMap::new();
        for (at, p) in was.polygons.iter().enumerate().rev() {
            let corner = |k: usize| {
                was.compressed_vertices
                    .get(usize::from(p.vertex_indices[k] & VERTEX_INDEX_MASK))
                    .map(words)
            };
            if p.primitive() != 0 {
                continue;
            }
            if let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) {
                held.entry([a, b, c]).or_default().push(at);
            }
        }
        // The triangle of the base each triangle is, and where each triangle of the base now is.
        let is: Vec<Option<usize>> = corners
            .iter()
            .zip(triangles)
            .map(|(corners, (material, _))| {
                let spare = held.get_mut(&corners.map(|i| vertices[usize::from(i)]))?;
                // ⚠ Two triangles of the base on the same corners are told apart by their material.
                let of = |at: &usize| was.poly_mat_index_list.get(*at).copied() == *material;
                let at = spare.iter().rposition(of).or(spare.len().checked_sub(1))?;
                Some(spare.remove(at))
            })
            .collect();
        let mut now: HashMap<usize, u16> = HashMap::new();
        for (i, base) in is.iter().enumerate() {
            if let Some(base) = base {
                now.insert(*base, i as u16);
            }
        }
        for (i, base) in is.iter().enumerate() {
            let Some(base) = base.and_then(|at| was.polygons.get(at)) else {
                continue;
            };
            // A triangle the base holds keeps the area the file states for it and the bit above each vertex index.
            polygons[i].area = base.area;
            for k in 0..3 {
                polygons[i].vertex_indices[k] |= base.vertex_indices[k] & !VERTEX_INDEX_MASK;
                // ⚠ Between two triangles the base holds the neighbour is the file's, which is not always the one the builder finds.
                let (stated, built) = (
                    base.neighboring_polygons[k],
                    polygons[i].neighboring_polygons[k],
                );
                let built_is_the_bases = is.get(usize::from(built)).is_some_and(Option::is_some);
                polygons[i].neighboring_polygons[k] = match now.get(&usize::from(stated)) {
                    Some(kept) => *kept,
                    None if stated == NO_NEIGHBOUR && built_is_the_bases => NO_NEIGHBOUR,
                    None => built,
                };
            }
        }
    }
    // A primitive the base holds is written over its own polygon, the one at its place first.
    let mut spare: Vec<Option<usize>> = soup.primitive_polygons.iter().copied().map(Some).collect();
    for (i, (prim, index)) in primitives.iter().zip(&stands_on).enumerate() {
        let same = |k: &usize| spare[*k].is_some() && soup.primitives[*k].same(prim);
        let like = Some(i)
            .filter(|i| *i < spare.len() && same(i))
            .or_else(|| (0..spare.len()).find(same))
            .and_then(|k| spare[k].take())
            .and_then(|at| was.polygons.get(at));
        polygons.push(shape_polygon(&prim.shape, index, like).ok_or_else(|| {
            format!(
                "holds a {} that does not stand on {} points",
                prim.shape.word(),
                index.len()
            )
        })?);
    }

    let materials: Vec<Option<u8>> = triangles
        .iter()
        .map(|(material, _)| *material)
        .chain(primitives.iter().map(|p| p.material))
        .collect();
    let stated = materials.iter().flatten().count();
    if stated != 0 && stated != materials.len() {
        return Err(
            "holds polygons that name a material beside polygons that name none, and a bound holds a material for every polygon or for none"
                .to_string(),
        );
    }

    let mut g = was.clone();
    g.compressed_vertices = vertices
        .iter()
        .map(|w| CompressedVertex {
            x: w[0],
            y: w[1],
            z: w[2],
            ..Default::default()
        })
        .collect();
    if !carried.is_empty() {
        // Every vertex is one of the base's: `index` refused any other while a run follows the vertices.
        let from: Vec<usize> = vertices
            .iter()
            .filter_map(|w| vertex_of.get(w).copied())
            .collect();
        g.vertex_attribs = from
            .iter()
            .filter_map(|at| was.vertex_attribs.get(*at).cloned())
            .collect();
        g.second_surface_vertex_displacements = from
            .iter()
            .filter_map(|at| was.second_surface_vertex_displacements.get(*at).copied())
            .collect();
    }
    g.polygons = polygons;
    g.poly_mat_index_list = materials.into_iter().flatten().collect();
    if !kept {
        let (to, at) = (&mut g.un_quantize_factor, &mut g.bounding_box_center);
        [to.x, to.y, to.z] = factor;
        [at.x, at.y, at.z] = centre;
    }
    // ⚠ A `.ybn` states its vertex count as the length of a hull it does not hold, so the count follows the vertices.
    if was.num_convex_hull_vertices().map(usize::from) == Some(was.compressed_vertices.len()) {
        g.set_num_convex_hull_vertices(g.compressed_vertices.len() as u16);
    }

    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &g.polygons {
        let (lo, hi) = g
            .polygon_box(p)
            .ok_or_else(|| "holds a polygon that names a vertex it does not hold".to_string())?;
        min = std::array::from_fn(|a| min[a].min(lo[a]));
        max = std::array::from_fn(|a| max[a].max(hi[a]));
    }
    // ⚠ The box of the base stays while the polygons stay inside it, a quantum of slack included: a box may be
    // wider than what it holds, and a composite keeps a copy of it that nothing here derives.
    let (was_min, was_max) = box_of(&was.base);
    if (0..3).all(|a| min[a] >= was_min[a] - factor[a] && max[a] <= was_max[a] + factor[a]) {
        return Ok(g);
    }
    let middle: [f32; 3] = std::array::from_fn(|a| (min[a] + max[a]) * 0.5);
    let reach = primitives
        .iter()
        .map(|p| p.shape.parts().1)
        .fold(0.0f32, f32::max);
    let radius = decoded
        .iter()
        .map(|p| {
            (0..3)
                .map(|a| (p[a] - middle[a]).powi(2))
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0f32, f32::max)
        + reach;
    let h = &mut g.base;
    let v = &mut h.bounding_box_max_xyz_margin_w;
    [v.x, v.y, v.z] = max;
    h.set_bounding_box_min(min);
    let v = &mut h.centroid_offset_xyz_material_id0_w;
    [v.x, v.y, v.z] = middle;
    h.radius_around_centroid = radius;
    Ok(g)
}

fn primitive_of(v: &Value, materials: usize, at: &mut Trail) -> Result<Prim, Fault> {
    let o = at.object(v)?;
    let kind: String = member(o, "type", at)?;
    let material = match stated(o, "material", at)? {
        Value::Null => None,
        _ => {
            let slot: u8 = member(o, "material", at)?;
            if usize::from(slot) >= materials {
                return Err(at.field("material", |at| {
                    at.fault(format!("names material {slot} of {materials}"))
                }));
            }
            Some(slot)
        }
    };
    let shape = match kind.as_str() {
        "sphere" => {
            at.only(o, &["type", "material", "center", "radius"])?;
            Shape::Sphere {
                center: three_of(o, "center", at)?,
                radius: member(o, "radius", at)?,
            }
        }
        "capsule" | "cylinder" => {
            at.only(o, &["type", "material", "end0", "end1", "radius"])?;
            let (end0, end1) = (three_of(o, "end0", at)?, three_of(o, "end1", at)?);
            let radius = member(o, "radius", at)?;
            if kind == "capsule" {
                Shape::Capsule { end0, end1, radius }
            } else {
                Shape::Cylinder { end0, end1, radius }
            }
        }
        "box" => {
            at.only(o, &["type", "material", "vertices"])?;
            let stated = list(o, "vertices", at)?;
            let corners = at.field("vertices", |at| -> Result<Vec<[f32; 3]>, Fault> {
                let mut corners = Vec::with_capacity(stated.len());
                for (i, v) in stated.iter().enumerate() {
                    corners.push(at.index(i, |at| -> Result<[f32; 3], Fault> {
                        let mut lanes: Vec<f32> = Vec::new();
                        lanes.read(v, at)?;
                        <[f32; 3]>::try_from(lanes)
                            .map_err(|_| at.fault("is not a list of three numbers"))
                    })?);
                }
                <[[f32; 3]; 4]>::try_from(corners.as_slice())
                    .map_err(|_| at.fault("is not a list of four corners"))?;
                Ok(corners)
            })?;
            Shape::Box {
                vertices: [corners[0], corners[1], corners[2], corners[3]],
            }
        }
        other => {
            return Err(at.field("type", |at| {
                at.fault(format!(
                    "is {other:?}, and a polygon that is no triangle is a sphere, a capsule, a box or a cylinder"
                ))
            }))
        }
    };
    Ok(Prim { material, shape })
}

/// The triangles of the node `node` of the glb, each with the slot of `names` its material is.
fn triangles_of(
    node: &str,
    names: &[String],
    reading: &mut Reading<'_>,
    at: &Trail,
) -> Result<Vec<Triangle>, Fault> {
    let key = strip_copy_suffix(node);
    let Some(mesh) = reading.meshes.get(key) else {
        return Err(at.fault(format!(
            "names the node `{node}`, which {MODEL} does not hold"
        )));
    };
    if !reading.used.insert(key.to_string()) {
        return Err(at.fault(format!(
            "names the node `{node}`, which another bound names too, and a node holds the triangles of one bound"
        )));
    }
    let mut triangles = Vec::new();
    for primitive in &mesh.primitives {
        let material = match primitive.material_name.as_deref() {
            None => None,
            Some(name) => {
                let slot = names
                    .iter()
                    .position(|held| held == strip_copy_suffix(name))
                    .ok_or_else(|| {
                        at.fault(format!(
                            "names the node `{node}`, which draws with the material `{name}`, and `materials` names none so"
                        ))
                    })?;
                Some(u8::try_from(slot).map_err(|_| {
                    at.fault(format!(
                        "names the node `{node}`, which draws with material {slot}, and a polygon names its material in eight bits"
                    ))
                })?)
            }
        };
        let corner = |i: u32| primitive.positions.get(i as usize).copied();
        for [a, b, c] in primitive.indices.as_chunks::<3>().0 {
            let (Some(a), Some(b), Some(c)) = (corner(*a), corner(*b), corner(*c)) else {
                return Err(at.fault(format!(
                    "names the node `{node}`, a triangle of which names a vertex it does not hold"
                )));
            };
            triangles.push((material, [a, b, c]));
        }
    }
    // Stable, so the triangles of one material keep the order the file gives them.
    triangles.sort_by_key(|(material, _)| *material);
    Ok(triangles)
}

/// Sets on the soup `g` what `o` states otherwise than `was` holds it; true when its polygons were built again.
fn set_soup(
    o: &Map<String, Value>,
    was: &PhBoundGeometry,
    g: &mut PhBoundGeometry,
    address: &str,
    reading: &mut Reading<'_>,
    at: &mut Trail,
) -> Result<bool, Fault> {
    let materials = list(o, "materials", at)?;
    let mut names: Vec<String> = Vec::with_capacity(materials.len());
    let mut ids: Vec<u64> = Vec::with_capacity(materials.len());
    at.field("materials", |at| -> Result<(), Fault> {
        for (i, v) in materials.iter().enumerate() {
            at.index(i, |at| -> Result<(), Fault> {
                let o = at.object(v)?;
                at.only(o, &["name", "id"])?;
                let name: String = member(o, "name", at)?;
                if names.contains(&name) {
                    return Err(at.field("name", |at| {
                        at.fault(format!(
                            "is `{name}`, which another material of this bound is called too, so a node could not say which it draws with"
                        ))
                    }));
                }
                names.push(name);
                let v = stated(o, "id", at)?;
                ids.push(at.field("id", |at| {
                    v.as_str()
                        .and_then(|text| u64::from_str_radix(text.strip_prefix("0x")?, 16).ok())
                        .or_else(|| v.as_u64())
                        .ok_or_else(|| at.fault("is not a material id, which is written `0x…`"))
                })?);
                Ok(())
            })?;
        }
        Ok(())
    })?;
    let stated_colors = list(o, "material_colors", at)?;
    let colors = at.field("material_colors", |at| {
        stated_colors
            .iter()
            .enumerate()
            .map(|(i, v)| {
                at.index(i, |at| {
                    hash_from(v).ok_or_else(|| at.fault("is not a colour word"))
                })
            })
            .collect::<Result<Vec<u32>, Fault>>()
    })?;
    let stated_primitives = list(o, "primitives", at)?;
    let primitives = at.field("primitives", |at| {
        stated_primitives
            .iter()
            .enumerate()
            .map(|(i, v)| at.index(i, |at| primitive_of(v, ids.len(), at)))
            .collect::<Result<Vec<Prim>, Fault>>()
    })?;
    let triangles = match o.get("node") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => at.field("node", |at| {
            let node = v.as_str().ok_or_else(|| at.fault("is not a name"))?;
            triangles_of(node, &names, reading, at)
        })?,
    };

    let soup =
        soup_of(was).map_err(|why| at.fault(format!("has a base that cannot be told: {why}")))?;
    let again = !unchanged(was, &soup, &triangles, &primitives);
    if again {
        *g = rebuilt(was, &soup, &triangles, &primitives).map_err(|why| at.fault(why))?;
        reading.report.push(format!(
            "{address}: its polygons changed, so they were built again: {} triangles and {} other polygons over {} vertices",
            triangles.len(),
            primitives.len(),
            g.compressed_vertices.len()
        ));
    }

    let use_active_components: Bool8 = member(o, "use_active_components", at)?;
    g.use_active_components = use_active_components;
    let is_flat: Bool8 = member(o, "is_flat", at)?;
    g.is_flat = is_flat;
    if ids != was.material_ids.iter().map(|m| m.id).collect::<Vec<_>>() {
        g.material_ids = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let mut material: PhMaterialMgrId =
                    was.material_ids.get(i).cloned().unwrap_or_default();
                material.id = *id;
                material
            })
            .collect();
        reading.report.push(format!("{address}: materials set"));
    }
    if colors
        != was
            .material_colors
            .iter()
            .map(|c| c.color)
            .collect::<Vec<_>>()
    {
        g.material_colors = colors
            .iter()
            .enumerate()
            .map(|(i, color)| {
                let mut held: Color32 = was.material_colors.get(i).cloned().unwrap_or_default();
                held.color = *color;
                held
            })
            .collect();
        reading
            .report
            .push(format!("{address}: material_colors set"));
    }
    Ok(again)
}

/// The bound `v` states, laid over `was`, the bound of the base at `address`.
fn set_bound(
    v: &Value,
    was: &PhBoundAny,
    address: &str,
    reading: &mut Reading<'_>,
    at: &mut Trail,
) -> Result<PhBoundAny, Fault> {
    let o = at.object(v)?;
    let kind: String = member(o, "type", at)?;
    if kind != type_name(was) {
        return Err(at.field("type", |at| {
            at.fault(format!(
                "is {kind} and the base holds a {} there: the kind of a bound is not an edit this source compiles",
                type_name(was)
            ))
        }));
    }
    let known = |more: &[&'static str]| -> Vec<&'static str> {
        HEADER.iter().chain(more).copied().collect()
    };
    let mut bound = was.clone();
    match (&mut bound, was) {
        (PhBoundAny::PhBoundCapsule(capsule), PhBoundAny::PhBoundCapsule(base)) => {
            at.only(o, &known(&["capsule_half_height"]))?;
            let half: f32 = member(o, "capsule_half_height", at)?;
            if half.to_bits() != base.capsule_half_height.to_bits() {
                capsule.capsule_half_height = half;
                reading
                    .report
                    .push(format!("{address}: capsule_half_height set"));
            }
        }
        (PhBoundAny::PhBoundGeometry(g), PhBoundAny::PhBoundGeometry(base)) => {
            at.only(o, &known(&SOUP))?;
            set_soup(o, base, g, address, reading, at)?;
        }
        (PhBoundAny::PhBoundBVH(b), PhBoundAny::PhBoundBVH(base)) => {
            at.only(o, &known(&SOUP))?;
            if set_soup(o, &base.base, &mut b.base, address, reading, at)? {
                build::rebuild(b)
                    .map_err(|e| at.fault(format!("has polygons no tree is built over: {e}")))?;
            }
        }
        (PhBoundAny::PhBoundComposite(composite), PhBoundAny::PhBoundComposite(base)) => {
            at.only(o, &known(&["children"]))?;
            set_children(o, base, composite, address, reading, at)?;
        }
        _ => at.only(o, &HEADER)?,
    }
    set_header(
        o,
        header(was),
        header_mut(&mut bound),
        address,
        &mut reading.report,
        at,
    )?;
    Ok(bound)
}

/// Writes `ybn` into `parent` as `<name>.bounds.src` and returns the folder; nothing is left behind on a refusal.
///
/// ⚠ A bound whose source does not compile to the canonical save of `ybn` is refused, so no source is made that would ship as changed.
pub fn extract(
    ybn: &[u8],
    name: &str,
    parent: &Path,
    how: &Extraction<'_>,
) -> Result<PathBuf, Vec<Problem>> {
    let refused = |why: String| vec![Problem::new(why)];
    let target = target_of(ybn, "ybn")?;
    let bound =
        PhBoundAny::from_bytes(ybn).map_err(|e| refused(format!("cannot read the .ybn: {e}")))?;
    let canonical_save = bound.to_bytes().map_err(|e| {
        refused(format!(
            "the .ybn does not save, so the game would misread what a source of it compiles to: {e}"
        ))
    })?;
    let mut out = Writing {
        name,
        meshes: Vec::new(),
        materials: Vec::new(),
    };
    let document = bound_document(&bound, ROOT, &mut out).map_err(refused)?;
    let model = (!out.meshes.is_empty()).then(|| {
        let scene = Scene {
            name: name.to_string(),
            objects: vec![Object {
                name: name.to_string(),
                meshes: out.meshes,
                ..Object::default()
            }],
            materials: out.materials,
            ..Scene::default()
        };
        write(&scene, name, Container::Glb).primary
    });

    let origin = Origin {
        how: How::Extracted,
        from_game: how.from_game,
        virtual_path: how.virtual_path.map(str::to_string),
        game_build: None,
        sha256: Some(sha256_hex(ybn)),
        cooked_sha256: None,
        extra: Default::default(),
    };
    let mut manifest =
        AssetManifest::new(KIND, name, target.slug(), origin).map_err(|p| vec![p])?;
    let compile = Compile {
        base: sha256_hex(ybn),
    };
    // Built from a struct of one string, which always serialises.
    manifest.extra.insert(
        "compile".to_string(),
        serde_json::to_value(&compile).unwrap_or_default(),
    );
    let inputs = Inputs {
        manifest,
        compile,
        base: ybn.to_vec(),
        // A value of numbers and strings always serialises.
        rules: canonical(&json!({ "bound": document }))
            .unwrap_or_default()
            .into_bytes(),
        model,
    };
    let dir = parent.join(folder_name(name, KIND));
    if dir.exists() {
        return Err(refused(format!("{} already exists", dir.display())));
    }
    // ⚠ A source the compile would refuse is not written, nor one that compiles to another file than the base saves as.
    if inputs.compile()?.bytes != canonical_save {
        return Err(refused(
            "its source does not compile back to the canonical save of the file".to_string(),
        ));
    }
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(MANIFEST), inputs.manifest.to_text()))
        .and_then(|()| std::fs::write(dir.join(BASE), &inputs.base))
        .and_then(|()| std::fs::write(dir.join(RULES), &inputs.rules))
        .and_then(|()| match &inputs.model {
            Some(glb) => std::fs::write(dir.join(MODEL), glb),
            None => Ok(()),
        });
    match written {
        Ok(()) => Ok(dir),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            Err(refused(format!("cannot write {}: {e}", dir.display())))
        }
    }
}

impl Inputs {
    pub fn read(source: &Source) -> Result<Self, Vec<Problem>> {
        let manifest = source.manifest.clone();
        if manifest.kind != KIND {
            return Err(vec![problem(
                format!("{:?} is not a bounds source", manifest.kind),
                MANIFEST,
            )]);
        }
        let compile = manifest
            .extra
            .get("compile")
            .ok_or_else(|| {
                "a bounds source states what it compiles, and there is no `compile` block"
                    .to_string()
            })
            .and_then(|stated| {
                serde_json::from_value::<Compile>(stated.clone())
                    .map_err(|e| format!("compile: {e}"))
            })
            .map_err(|why| vec![problem(why, MANIFEST)])?;
        let base = std::fs::read(source.dir.join(BASE))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), BASE)])?;
        let rules = std::fs::read(source.dir.join(RULES))
            .map_err(|e| vec![problem(format!("cannot read: {e}"), RULES)])?;
        let model = match std::fs::read(source.dir.join(MODEL)) {
            Ok(glb) => Some(glb),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(vec![problem(format!("cannot read: {e}"), MODEL)]),
        };
        Ok(Self {
            manifest,
            compile,
            base,
            rules,
            model,
        })
    }

    /// A digest of every input.
    pub fn digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        let mut put = |bytes: &[u8]| {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        };
        put(self.manifest.digest_text().as_bytes());
        put(&self.base);
        put(&self.rules);
        put(self.model.as_deref().unwrap_or_default());
        hash.finalize().into()
    }

    /// The `.ybn` the inputs compile to, read back through the reader before it is returned.
    pub fn compile(&self) -> Result<Compiled, Vec<Problem>> {
        let held = sha256_hex(&self.base);
        if held != self.compile.base {
            return Err(vec![problem(
                format!(
                    "the base holds {held} and compile.base says {}; the base is not the file the document and the glb were written from",
                    self.compile.base
                ),
                BASE,
            )]);
        }
        let base = PhBoundAny::from_bytes(&self.base)
            .map_err(|e| vec![problem(format!("the base does not read: {e}"), BASE)])?;
        let scene = self
            .model
            .as_deref()
            .map(|glb| load(Path::new(MODEL), glb))
            .transpose()
            .map_err(|e| vec![problem(e.to_string(), MODEL)])?;
        let mut reading = Reading {
            meshes: HashMap::new(),
            used: HashSet::new(),
            report: Vec::new(),
        };
        for mesh in scene
            .iter()
            .flat_map(|s| &s.objects)
            .flat_map(|o| &o.meshes)
        {
            let name = strip_copy_suffix(&mesh.name).to_string();
            if reading.meshes.insert(name.clone(), mesh).is_some() {
                return Err(vec![problem(
                    format!(
                        "holds two nodes called `{name}`, and a bound names its node by that name"
                    ),
                    MODEL,
                )]);
            }
        }
        let bound = root(&self.rules)
            .and_then(|doc| {
                let at = &mut Trail::default();
                at.only(&doc, &["bound"])?;
                let v = stated(&doc, "bound", at)?;
                at.field("bound", |at| set_bound(v, &base, ROOT, &mut reading, at))
            })
            .map_err(|fault| vec![problem(fault.to_string(), RULES)])?;
        let mut unnamed: Vec<&String> = reading
            .meshes
            .keys()
            .filter(|name| !reading.used.contains(*name))
            .collect();
        unnamed.sort();
        if let Some(name) = unnamed.first() {
            return Err(vec![problem(
                format!("holds the node `{name}`, which no bound of {RULES} names as its `node`"),
                MODEL,
            )]);
        }
        let bytes = bound.to_bytes().map_err(|e| {
            vec![problem(
                format!("the bound would not save, and the game would misread it: {e}"),
                RULES,
            )]
        })?;
        // ⚠ A file the reader refuses would be shipped as a file the game misreads.
        PhBoundAny::from_bytes(&bytes).map_err(|e| {
            vec![problem(
                format!("the compiled .ybn does not read back: {e}"),
                RULES,
            )]
        })?;
        Ok(Compiled {
            file_name: output_name(&self.manifest),
            bytes,
            report: reading.report,
        })
    }

    /// The name of the file the inputs compile to.
    pub fn output_name(&self) -> String {
        output_name(&self.manifest)
    }
}

/// What the folder `source` names compiles to.
pub fn compile(source: &Source) -> Result<Compiled, Vec<Problem>> {
    Inputs::read(source)?.compile()
}

/// The name of the file a bounds source compiles to.
pub fn output_name(manifest: &AssetManifest) -> String {
    format!("{}.ybn", manifest.name)
}

/// Whether a path inside a bounds source, relative to its folder, is a file the compile reads.
pub fn is_input(inside: &Path) -> bool {
    let parts: Vec<&str> = inside
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [] => true,
        [file] => [MANIFEST, BASE, RULES, MODEL].contains(file),
        _ => false,
    }
}
