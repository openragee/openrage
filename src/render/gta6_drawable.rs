//! A scanned GTA VI body in the model view: a drawable's geometries whose positions read, at the
//! level of detail the preview picks, each with its material's base-colour texture and skinned
//! where a GTA VI clip poses a skeleton; or a bound's triangles and primitive children, each child
//! placed by its matrix.

use std::sync::Arc;

use glam::{Mat3, Mat4, Vec3, Vec4};
use or_io::gta5::data::materials_palette::MaterialPalette;
use or_io::gta5::{
    PhBound, PhBoundAny, PhBoundBox, PhBoundCapsule, PhBoundCylinder, PhBoundDisc, PhBoundSphere,
    Vec4V,
};
use or_io::gta6::headerless::{
    material_key, Bound, BoundChild, ChildBound, Drawable, Geometry, Model, ParameterValue,
    Primitive, Resource, Skeleton, Vertices,
};
use or_renderer::MeshVertex;

use super::bound_meshes::{
    append_polygon_box, append_polygon_capsule, append_polygon_cylinder, append_polygon_sphere,
    append_polygon_triangle, build_bound_meshes,
};
use super::debug_channel::DebugChannel;
use super::meshes::{face_normals, MeshUpload};
use crate::state::capabilities::GeometryRequest;
use crate::state::gta6_resource::{drawable_of, texture_name};
use crate::state::gta6_weapon_parts::Part;
use crate::state::{AppState, DocPayload, DocumentId};

/// The skeleton a GTA VI clip poses, and the drawable's own, which its meshes are skinned to.
type Posing<'a> = (&'a Skeleton, Option<&'a Skeleton>);

/// The meshes of the active GTA VI body: its drawable's chosen level of detail, or its bound's triangles.
pub(crate) fn meshes(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
    let DocPayload::Gta6Drawable(resource) = &req.doc.payload else {
        return Vec::new();
    };
    let posed = crate::ui::panels::inspector::gta6_posed(req.state);
    let own = posed
        .as_ref()
        .and_then(|_| own_skeleton(req.doc, req.selection));
    let posing = (posed.as_ref()).map(|p| (p.skeleton, own.as_ref()));
    let (lod, channel) = (
        req.state.preview_scene.lod,
        req.state.preview_scene.debug_channel,
    );
    let srgb = |key: u64| srgb_key(req.state, req.doc.id, key);
    match (drawable_of(resource, req.selection), &**resource) {
        (Some(drawable), _) => {
            // A clip poses the weapon's own mesh: no part is skinned to its skeleton.
            let parts = parts_drawn(req.state, req.doc).filter(|_| posing.is_none());
            let mut out = match &parts {
                Some(parts) => (parts.iter())
                    .flat_map(|part| part_meshes(part, lod, channel, &srgb))
                    .collect(),
                None => meshes_at(drawable, lod, posing, channel, &srgb),
            };
            out.extend(surface_meshes(drawable));
            out
        }
        (None, Resource::Bound(bound)) => bound_meshes(bound),
        (None, Resource::Cloth(cloth)) => super::gta6_cloth::meshes(cloth),
        _ => Vec::new(),
    }
}

/// The parts drawn in place of `doc`'s own mesh: those kept for a weapon, while the row's box is ticked.
pub(crate) fn parts_drawn(
    state: &AppState,
    doc: &crate::state::Document,
) -> Option<Arc<Vec<Part>>> {
    (state.preview_scene.assemble_parts)
        .then(|| crate::state::gta6_weapon_parts::of(doc.path.as_deref()?))
        .flatten()
}

/// A part's meshes at `lod`, in the weapon's frame.
fn part_meshes(
    part: &Part,
    lod: usize,
    channel: Option<DebugChannel>,
    srgb: &dyn Fn(u64) -> Option<bool>,
) -> Vec<MeshUpload> {
    let Some(drawable) = drawable_of(&part.resource, None) else {
        return Vec::new();
    };
    let normals = Mat3::from_mat4(part.place).inverse().transpose();
    let mut meshes = meshes_at(drawable, lod, None, channel, srgb);
    for v in meshes.iter_mut().flat_map(|mesh| &mut mesh.vertices) {
        v.position = (part.place.transform_point3(Vec3::from(v.position))).to_array();
        v.normal = (normals * Vec3::from(v.normal))
            .normalize_or_zero()
            .to_array();
    }
    meshes
}

/// The levels of `doc`'s GTA VI drawable that hold models, each with the distance it is drawn to.
pub(crate) fn levels(state: &AppState, doc: DocumentId) -> Vec<(usize, f32)> {
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc).map(|d| &d.payload) else {
        return Vec::new();
    };
    let Some(drawable) = drawable_of(resource, state.selection.selected(doc)) else {
        return Vec::new();
    };
    (drawable
        .levels
        .iter()
        .zip(drawable.lod_distances)
        .enumerate())
    .filter(|(_, (models, _))| models.as_ref().is_some_and(|m| !m.is_empty()))
    .map(|(level, (_, distance))| (level, distance))
    .collect()
}

/// The bound Collision draws for the active GTA VI body: its drawable's own composite, else its fragment's.
pub(crate) fn embedded_bound(state: &AppState) -> Option<&Bound> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let own =
        drawable_of(resource, state.selection.selected(doc.id)).and_then(|d| d.bound.as_ref());
    match &**resource {
        Resource::DrawableWithBound(r) => own.or(r.bound.as_ref()),
        _ => own,
    }
}

/// The meshes Collision draws for the active GTA VI body: a fragment's children each at its
/// bone, else the composite its drawable embeds.
pub(crate) fn collision_meshes(state: &AppState) -> Vec<MeshUpload> {
    let fragment = state.active_document().and_then(|doc| match &doc.payload {
        DocPayload::Gta6Drawable(r) => match &**r {
            Resource::DrawableWithBound(f) if !f.fragment.children.is_empty() => Some(f),
            _ => None,
        },
        _ => None,
    });
    let Some(f) = fragment else {
        return embedded_bound(state).map(bound_meshes).unwrap_or_default();
    };
    let listed = |k: u16| f.bound.as_ref()?.children.get(usize::from(k));
    (f.fragment.children.iter())
        .flat_map(|c| match c.composite.and_then(listed) {
            Some(listed) if c.bound.is_none() => {
                placed_meshes(&c.matrix, listed.bound.as_ref(), listed.material_id)
            }
            _ => placed_meshes(&c.matrix, c.bound.as_ref(), None),
        })
        .collect()
}

/// The colour the surfaces a drawable's record at 0xD0 holds are drawn in, over its level.
const SURFACE: [f32; 4] = [0.3, 0.8, 0.95, 1.0];

/// A mesh of each surface the record at 0xD0 of `drawable` holds, in its frame and of every level.
fn surface_meshes(drawable: &Drawable) -> Vec<MeshUpload> {
    let Some(Ok(surfaces)) = drawable.embedded.as_deref() else {
        return Vec::new();
    };
    (surfaces.iter().filter(|s| !s.triangles.is_empty()))
        .map(|s| {
            let mut vertices = Vec::with_capacity(3 * s.triangles.len());
            for t in &s.triangles {
                let [a, b, c] = t.map(Vec3::from);
                let n = (b - a).cross(c - a).normalize_or_zero().to_array();
                vertices.extend(
                    t.iter()
                        .map(|p| MeshVertex::new_coloured(*p, n, [0.0; 2], SURFACE)),
                );
            }
            MeshUpload {
                bump: false,
                indices: (0..vertices.len() as u32).collect(),
                vertices,
                texture_name: None,
                skinning: None,
                palette_bones: Vec::new(),
                channels: Vec::new(),
                source: None,
            }
        })
        .collect()
}

/// A geometry's model, its index in the model and its mesh.
pub(crate) type DrawnGeometry = (usize, usize, MeshUpload);

/// The level the model view draws of `doc`'s GTA VI drawable, and each of its geometries whose
/// positions read, at bind, with its model and its index in the model.
pub(crate) fn drawn_geometries(
    state: &AppState,
    doc: DocumentId,
) -> Option<(usize, Vec<DrawnGeometry>)> {
    let DocPayload::Gta6Drawable(resource) = &state.document(doc)?.payload else {
        return None;
    };
    let drawable = drawable_of(resource, state.selection.selected(doc))?;
    let lod = level_drawn(drawable, state.preview_scene.lod);
    // A weapon drawn as its parts shows none of its own geometries to pick.
    if parts_drawn(state, state.document(doc)?).is_some() {
        return Some((lod, Vec::new()));
    }
    let bases = base_colours(drawable, &|key| srgb_key(state, doc, key));
    let mut out = Vec::new();
    for (m, model) in drawable.levels[lod].iter().flatten().enumerate() {
        for (g, geometry) in model.geometries.iter().enumerate() {
            if let Some(mesh) = geometry.as_ref().ok().and_then(|g| upload(&bases, g, None)) {
                out.push((m, g, mesh));
            }
        }
    }
    Some((lod, out))
}

/// The skeleton of a scanned drawable or fragment, or of the dictionary entry `selection` is in,
/// read when it opened.
fn own_skeleton(doc: &crate::state::Document, selection: Option<u64>) -> Option<Skeleton> {
    crate::state::gta6_entry_skeletons::shown(doc, selection)
}

/// The level of detail `lod`, or the first where that one has no models.
fn level_drawn(drawable: &Drawable, lod: usize) -> usize {
    let models = drawable.levels.get(lod).and_then(Option::as_ref);
    match models.is_some_and(|models| !models.is_empty()) {
        true => lod,
        false => 0,
    }
}

/// The models of the level of detail `lod`, or of the first level where that one has none.
fn drawn_level(drawable: &Drawable, lod: usize) -> impl Iterator<Item = &Model> {
    drawable.levels[level_drawn(drawable, lod)].iter().flatten()
}

pub(crate) fn meshes_at(
    drawable: &Drawable,
    lod: usize,
    posing: Option<Posing<'_>>,
    channel: Option<DebugChannel>,
    srgb: &dyn Fn(u64) -> Option<bool>,
) -> Vec<MeshUpload> {
    let bases = base_colours(drawable, srgb);
    let mut out = Vec::new();
    for model in drawn_level(drawable, lod) {
        for g in model.geometries.iter().flatten() {
            let Some(mut mesh) = upload(&bases, g, channel) else {
                continue;
            };
            if let Some((skeleton, own)) = posing {
                let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.position).collect();
                let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.normal).collect();
                // One that does not join is drawn at bind; the animation's inspector names it.
                let extra = &drawable.extra_skeletons;
                mesh.skinning = or_anim::gta6::skin_with_extra(
                    &g.vertices,
                    &positions,
                    &normals,
                    model,
                    own,
                    extra,
                    skeleton,
                )
                .ok();
            }
            out.push(mesh);
        }
    }
    out
}

/// Each geometry of the drawn level of `doc` that does not join `skeleton`: its model, its index and why.
pub(crate) fn unjoined(
    doc: &crate::state::Document,
    selection: Option<u64>,
    lod: usize,
    skeleton: &Skeleton,
) -> Vec<(usize, usize, or_anim::gta6::Unjoined)> {
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return Vec::new();
    };
    let Some(drawable) = drawable_of(resource, selection) else {
        return Vec::new();
    };
    let own = own_skeleton(doc, selection);
    let mut out = Vec::new();
    for (m, model) in drawn_level(drawable, lod).enumerate() {
        for (g, geometry) in model.geometries.iter().enumerate() {
            let Ok(geometry) = geometry else { continue };
            let Ok(positions) = &geometry.positions else {
                continue;
            };
            let (own, extra) = (own.as_ref(), &drawable.extra_skeletons);
            if let Err(why) = or_anim::gta6::skin_with_extra(
                &geometry.vertices,
                positions,
                &[],
                model,
                own,
                extra,
                skeleton,
            ) {
                out.push((m, g, why));
            }
        }
    }
    out
}

/// The hash a material's base-colour texture record states.
const DIFFUSE_TEX: u32 = or_io::gta6::hash::name_hash32(b"DiffuseTex");

/// Whether the pack that holds `key`, a key of `doc`'s drawable, open or in the install's pack
/// index, states an sRGB format for it, as colour data has: `false` too for a key no pack of its
/// install holds, and `None` for one a pack not indexed or opened yet may hold.
pub(crate) fn srgb_key(state: &AppState, doc: DocumentId, key: u64) -> Option<bool> {
    let name = texture_name(key);
    let entry = crate::state::textures::open_pack_entry(state, &name);
    let code = (entry.map(|e| e.format_code)).or_else(|| state.indexed_texture_format(key));
    let srgb =
        |code: u16| or_io::codec::texture::DxgiFormat::from_raw((code >> 3).into()).is_srgb();
    (code.map(srgb)).or_else(|| state.held_by_no_pack(doc, &name).map(|_| false))
}

/// The hash a material's normal map's texture record states.
const BUMP_TEX: u32 = or_io::gta6::hash::name_hash32(b"BumpTex");

/// A shader's texture key, and whether it is the normal map of a material that has no colour map.
type DrawnTexture = Option<(u64, bool)>;

/// Each shader's base-colour key: its `DiffuseTex` record's when bound, else the first key `srgb`
/// holds, else the first key it knows nothing of, which asks for its pack; and where none of the
/// three is, its `BumpTex` record's, as the normal map that shades it. ⚠ A key a pack states
/// as linear is a data or normal map: drawn as a colour it tints the whole mesh green and blue.
fn base_colours(drawable: &Drawable, srgb: &dyn Fn(u64) -> Option<bool>) -> Vec<DrawnTexture> {
    (drawable.shaders.iter())
        .map(|s| {
            let key = |slot: u8| s.texture_keys.get(usize::from(slot)).copied().flatten();
            let record = |hash: u32| {
                (s.parameters.iter()).find_map(|p| match p.value {
                    ParameterValue::Texture { slot } if p.hash == hash => key(slot),
                    _ => None,
                })
            };
            let keys = || s.texture_keys.iter().flatten().copied();
            let colour = record(DIFFUSE_TEX)
                .or_else(|| keys().find(|k| srgb(*k) == Some(true)))
                .or_else(|| keys().find(|k| srgb(*k).is_none()));
            (colour.map(|key| (key, false))).or_else(|| record(BUMP_TEX).map(|key| (key, true)))
        })
        .collect()
}

/// The channels of `v` that hold a value for each of `count` vertices, as the Channel view names
/// them, each with its value for each vertex.
fn channels_of(v: &Vertices, count: usize) -> Vec<(DebugChannel, Vec<[f32; 4]>)> {
    let lanes = |c: &[[f32; 3]]| c.iter().map(|[x, y, z]| [*x, *y, *z, 0.0]).collect();
    let uv = |c: &[[f32; 2]]| c.iter().map(|[u, v]| [*u, *v, 0.0, 0.0]).collect();
    let mut out: Vec<(DebugChannel, Vec<[f32; 4]>)> =
        vec![(DebugChannel::Normal, lanes(&v.normals))];
    out.extend(
        (v.colours.iter()).map(|(c, values)| (DebugChannel::Colour(c - 24), values.clone())),
    );
    out.extend((v.uvs.iter()).map(|(c, values)| (DebugChannel::Texcoord(c - 28), uv(values))));
    out.push((DebugChannel::Tangent(0), v.tangents.clone()));
    out.extend(
        (v.uv_tangents.iter()).map(|(c, values)| (DebugChannel::Tangent(c - 8), values.clone())),
    );
    out.push((DebugChannel::Reference, over_extent(&v.reference)));
    out.retain(|(_, values)| values.len() == count);
    out
}

/// Each reference position from a quarter to one along each axis of the box of those the geometry
/// holds, its source's offset being unknown; black where a vertex holds none.
fn over_extent(reference: &[Option<[f32; 3]>]) -> Vec<[f32; 4]> {
    let held = reference.iter().flatten().map(|p| Vec3::from(*p));
    let (low, high) = held.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
    let span = (high - low).max(Vec3::splat(f32::EPSILON));
    (reference.iter())
        .map(|p| match p {
            Some(p) => (Vec3::splat(0.25) + 0.75 * (Vec3::from(*p) - low) / span).extend(1.0),
            None => Vec4::W,
        })
        .map(|c| c.to_array())
        .collect()
}

fn upload(
    bases: &[DrawnTexture],
    geometry: &Geometry,
    channel: Option<DebugChannel>,
) -> Option<MeshUpload> {
    let texture = bases.get(usize::from(geometry.shader)).copied().flatten();
    let positions = geometry.positions.as_ref().ok()?;
    let channels = channels_of(&geometry.vertices, positions.len());
    let shown = (channels.iter()).find(|(c, _)| Some(*c) == channel);
    let shown = shown.map(|(c, values)| (*c, values));
    // Every corner is a u16 vertex-list entry, so the narrowing loses nothing.
    let corners: Vec<u16> = geometry
        .triangles
        .iter()
        .flatten()
        .map(|&i| i as u16)
        .collect();
    let mut normals = geometry.vertices.normals.clone();
    if normals.len() != positions.len() {
        face_normals(positions, &corners, &mut normals);
    }
    let uvs = geometry
        .vertices
        .uvs
        .iter()
        .find(|(channel, _)| *channel == 28)
        .map(|(_, uvs)| uvs.as_slice());
    let vertices = positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let uv = uvs.and_then(|uvs| uvs.get(i)).copied().unwrap_or_default();
            match shown {
                Some((c, values)) => {
                    MeshVertex::new_coloured(*p, normals[i], uv, c.visualise(values[i]))
                }
                None => MeshVertex::new(*p, normals[i], uv),
            }
        })
        .collect();
    Some(MeshUpload {
        vertices,
        indices: corners.into_iter().map(u32::from).collect(),
        texture_name: texture.map(|(key, _)| texture_name(key)),
        bump: texture.is_some_and(|(_, bump)| bump),
        skinning: None,
        palette_bones: Vec::new(),
        channels: channels.into_iter().map(|(c, _)| c).collect(),
        source: None,
    })
}

/// A child's matrix: its first three rows the axes its vertices are placed along, the fourth its origin.
fn placement(matrix: &[[f32; 4]; 4]) -> Mat4 {
    let axis = |r: [f32; 4]| Vec4::new(r[0], r[1], r[2], 0.0);
    let [x, y, z, w] = *matrix;
    Mat4::from_cols(axis(x), axis(y), axis(z), Vec4::new(w[0], w[1], w[2], 1.0))
}

/// One mesh for each geometry child of `bound`, every primitive of it, and the
/// meshes of each primitive child, each placed by the child's matrix and coloured by its material key.
pub(crate) fn bound_meshes(bound: &Bound) -> Vec<MeshUpload> {
    bound.children.iter().flat_map(child_meshes).collect()
}

/// A colour of its own for each material key, the same in every body.
fn key_tint(key: u16) -> [f32; 4] {
    // The key's 15 bits shuffled one to one, so that near keys differ, then five a channel.
    let mut x = (u32::from(key) * 0x5BD1) & 0x7FFF;
    x ^= x >> 7;
    x = (x * 0x2A37) & 0x7FFF;
    let channel = |shift: u32| 0.25 + 0.75 * ((x >> shift) & 31) as f32 / 31.0;
    [channel(0), channel(5), channel(10), 1.0]
}

/// One child's meshes, placed by its matrix.
pub(crate) fn child_meshes(child: &BoundChild) -> Vec<MeshUpload> {
    placed_meshes(&child.matrix, child.bound.as_ref(), child.material_id)
}

/// The meshes of `bound` placed by `matrix`, rows as a composite's, a primitive coloured by the key of
/// its `material_id`.
fn placed_meshes(
    matrix: &[[f32; 4]; 4],
    bound: Option<&Result<ChildBound, or_io::gta6::headerless::HeaderlessError>>,
    material_id: Option<u64>,
) -> Vec<MeshUpload> {
    let to = placement(matrix);
    let g = match bound {
        Some(Ok(ChildBound::Geometry(g))) => g,
        Some(Ok(shape)) => {
            let tint = material_id.map(|id| key_tint(material_key(id)));
            let mut out = primitive_meshes(shape);
            for mesh in &mut out {
                for v in &mut mesh.vertices {
                    v.position = to.transform_point3(Vec3::from(v.position)).to_array();
                    let normal = to.transform_vector3(Vec3::from(v.normal));
                    v.normal = normal.normalize_or_zero().to_array();
                    v.colour = tint.unwrap_or(v.colour);
                }
            }
            return out;
        }
        _ => return Vec::new(),
    };
    let positions: Vec<Vec3> = (g.vertices.iter())
        .map(|v| to.transform_point3(Vec3::from(*v)))
        .collect();
    let at = |i: u16| positions.get(usize::from(i)).copied();
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    for (i, p) in g.primitives.iter().enumerate() {
        let tint = g.primitive_key(i).map_or([1.0; 4], key_tint);
        append_primitive(&mut vertices, &mut indices, p, &at, tint);
    }
    if indices.is_empty() {
        return Vec::new();
    }
    vec![MeshUpload {
        bump: false,
        vertices,
        indices,
        texture_name: None,
        skinning: None,
        palette_bones: Vec::new(),
        channels: Vec::new(),
        source: None,
    }]
}

/// Each primitive of a geometry child, by its index, as its own mesh placed by the child's matrix.
pub(crate) fn primitive_meshes_of(child: &BoundChild) -> Vec<(usize, MeshUpload)> {
    let Some(Ok(ChildBound::Geometry(g))) = &child.bound else {
        return Vec::new();
    };
    let to = placement(&child.matrix);
    let positions: Vec<Vec3> = (g.vertices.iter())
        .map(|v| to.transform_point3(Vec3::from(*v)))
        .collect();
    let at = |i: u16| positions.get(usize::from(i)).copied();
    let mut out = Vec::new();
    for (i, p) in g.primitives.iter().enumerate() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        append_primitive(&mut vertices, &mut indices, p, &at, [1.0; 4]);
        if !indices.is_empty() {
            let mesh = MeshUpload {
                bump: false,
                vertices,
                indices,
                texture_name: None,
                skinning: None,
                palette_bones: Vec::new(),
                channels: Vec::new(),
                source: None,
            };
            out.push((i, mesh));
        }
    }
    out
}

/// The radius in metres of the marker drawn where a primitive has no extent.
const POINT_MARKER: f32 = 0.05;

/// The triangles of primitive `p`, whose vertex `i` is at `at(i)`, appended to `v` and `ix`; a
/// capsule whose ends are one vertex is the sphere there.
fn append_primitive(
    v: &mut Vec<MeshVertex>,
    ix: &mut Vec<u32>,
    p: &Primitive,
    at: &dyn Fn(u16) -> Option<Vec3>,
    tint: [f32; 4],
) {
    match *p {
        Primitive::Triangle {
            vertices: [a, b, c],
            ..
        } => {
            if let (Some(a), Some(b), Some(c)) = (at(a), at(b), at(c)) {
                append_polygon_triangle(v, ix, a, b, c, tint);
            }
        }
        Primitive::Sphere { centre, radius } => {
            if let Some(c) = at(centre) {
                append_polygon_sphere(v, ix, c, radius, tint);
            }
        }
        Primitive::Capsule {
            ends: [a, b],
            radius,
        } => {
            if let (Some(a), Some(b)) = (at(a), at(b)) {
                append_polygon_capsule(v, ix, a, b, radius, tint);
            }
        }
        Primitive::Cylinder {
            ends: [a, b],
            radius,
        } => match (at(a), at(b)) {
            // Ends on one vertex state no axis, and GTA V's box of it is the point: a marker.
            (Some(c), Some(_)) if a == b => append_polygon_sphere(v, ix, c, POINT_MARKER, tint),
            (Some(from), Some(to)) => append_polygon_cylinder(v, ix, from, to, radius, tint),
            _ => {}
        },
        Primitive::Box(corners) => {
            if let [Some(a), Some(b), Some(c), Some(d)] = corners.map(at) {
                append_polygon_box(v, ix, [a, b, c, d], tint);
            }
        }
    }
}

/// A mesh of `corners`' triangles over `positions`, with face normals; `None` for no triangle.
fn mesh_of(positions: Vec<[f32; 3]>, corners: Vec<u16>) -> Option<MeshUpload> {
    if corners.is_empty() {
        return None;
    }
    let mut normals = Vec::new();
    face_normals(&positions, &corners, &mut normals);
    Some(MeshUpload {
        bump: false,
        vertices: (positions.iter().zip(&normals))
            .map(|(p, n)| MeshVertex::new(*p, *n, [0.0; 2]))
            .collect(),
        indices: corners.into_iter().map(u32::from).collect(),
        texture_name: None,
        skinning: None,
        palette_bones: Vec::new(),
        channels: Vec::new(),
        source: None,
    })
}

/// A primitive child in its own frame: GTA V's drawing of a sphere, capsule, box, disc or
/// cylinder, and the shapes GTA V has none of built from their fields.
fn primitive_meshes(shape: &ChildBound) -> Vec<MeshUpload> {
    let Some((half, radius)) = shape.extent() else {
        return Vec::new();
    };
    let centre = match *shape {
        ChildBound::Capsule {
            box_min, box_max, ..
        }
        | ChildBound::Box { box_min, box_max } => {
            std::array::from_fn(|a| (box_min[a] + box_max[a]) / 2.0)
        }
        ChildBound::Sphere { centre, .. }
        | ChildBound::TaperedCapsule { centre, .. }
        | ChildBound::Disc { centre, .. }
        | ChildBound::Cylinder { centre, .. }
        | ChildBound::Hull { centre, .. }
        | ChildBound::Frustum { centre, .. } => centre,
        ChildBound::Geometry(_) => return Vec::new(),
    };
    let gta5 = |bound: PhBoundAny| build_bound_meshes(&bound, MaterialPalette::empty());
    let c = Vec3::from(centre);
    match *shape {
        ChildBound::Sphere { radius, .. } => gta5(PhBoundAny::PhBoundSphere(PhBoundSphere {
            base: gta5_header(centre, half, radius, radius),
            ..Default::default()
        })),
        ChildBound::Capsule { radius: margin, .. } => {
            gta5(PhBoundAny::PhBoundCapsule(PhBoundCapsule {
                base: gta5_header(centre, half, radius, margin),
                ..Default::default()
            }))
        }
        ChildBound::Box { .. } => gta5(PhBoundAny::PhBoundBox(PhBoundBox {
            base: gta5_header(centre, half, radius, 0.0),
            ..Default::default()
        })),
        ChildBound::Disc { margin, .. } => gta5(PhBoundAny::PhBoundDisc(PhBoundDisc {
            base: gta5_header(centre, half, radius, margin),
            ..Default::default()
        })),
        ChildBound::Cylinder { .. } => gta5(PhBoundAny::PhBoundCylinder(PhBoundCylinder {
            base: gta5_header(centre, half, radius, 0.0),
            ..Default::default()
        })),
        ChildBound::TaperedCapsule { radii, length, .. } => {
            let mut out: Vec<MeshUpload> = (radii.iter().zip([1.0, -1.0]))
                .flat_map(|(r, side)| {
                    let at = (c + Vec3::Y * side * length / 2.0).to_array();
                    gta5(PhBoundAny::PhBoundSphere(PhBoundSphere {
                        base: gta5_header(at, [*r; 3], *r, *r),
                        ..Default::default()
                    }))
                })
                .collect();
            out.extend(frustum(c, radii, length / 2.0));
            out
        }
        ChildBound::Frustum {
            radii,
            factors,
            height,
            margin,
            ..
        } => {
            let grown = [0, 1].map(|end| radii[end] + margin * factors[end]);
            frustum(c, grown, height / 2.0).into_iter().collect()
        }
        ChildBound::Hull {
            corners, margin, ..
        } => hull(c, &corners, margin).into_iter().collect(),
        ChildBound::Geometry(_) => Vec::new(),
    }
}

/// A GTA V bound header of the box `centre` ± `half`, with its radius and margin.
fn gta5_header(centre: [f32; 3], half: [f32; 3], radius: f32, margin: f32) -> PhBound {
    let v4 = |p: [f32; 3], w: f32| Vec4V {
        x: p[0],
        y: p[1],
        z: p[2],
        w,
        ..Default::default()
    };
    let mut header = PhBound {
        radius_around_centroid: radius,
        bounding_box_max_xyz_margin_w: v4(std::array::from_fn(|a| centre[a] + half[a]), margin),
        centroid_offset_xyz_material_id0_w: v4(centre, 0.0),
        ..Default::default()
    };
    header.set_bounding_box_min(std::array::from_fn(|a| centre[a] - half[a]));
    header
}

/// Segments around a frustum's rings.
const RING: u16 = 24;

/// A frustum along y about `c`: radius `radii[0]` at +`half_height`, `radii[1]` at its other end.
fn frustum(c: Vec3, radii: [f32; 2], half_height: f32) -> Option<MeshUpload> {
    let mut positions = Vec::new();
    // The rings twice: the side's, then the caps' own, so that no normal is shared at the rims.
    for _ in 0..2 {
        for (r, y) in radii.iter().zip([half_height, -half_height]) {
            for k in 0..RING {
                let a = std::f32::consts::TAU * f32::from(k) / f32::from(RING);
                positions.push((c + Vec3::new(r * a.cos(), y, r * a.sin())).to_array());
            }
        }
    }
    positions.push((c + Vec3::Y * half_height).to_array());
    positions.push((c - Vec3::Y * half_height).to_array());
    let (cap, top, bottom) = (2 * RING, 4 * RING, 4 * RING + 1);
    let mut corners = Vec::new();
    for k in 0..RING {
        let next = (k + 1) % RING;
        corners.extend([k, next, RING + k, next, RING + next, RING + k]);
        corners.extend([top, cap + next, cap + k]);
        corners.extend([bottom, cap + RING + k, cap + RING + next]);
    }
    mesh_of(positions, corners)
}

/// The hexahedron of a hull's corners, the first four one face and the last four the other, each
/// corner moved `margin` outward along every axis, so that it fills the hull's box.
fn hull(c: Vec3, corners: &[[f32; 3]; 8], margin: f32) -> Option<MeshUpload> {
    let points = corners.map(Vec3::from);
    let face = |f: &[Vec3]| f.iter().sum::<Vec3>() / 4.0;
    let (first, second) = (face(&points[..4]), face(&points[4..]));
    let axis = (first - second).try_normalize().unwrap_or(Vec3::Y);
    let (u, w) = axis.any_orthonormal_pair();
    // Each face's corners in turn about the axis, so that a side joins corner i of each.
    let around = |f: &[Vec3], mid: Vec3| {
        let mut f = f.to_vec();
        let angle = |p: &Vec3| (*p - mid).dot(w).atan2((*p - mid).dot(u));
        f.sort_by(|a, b| angle(a).total_cmp(&angle(b)));
        f
    };
    let middle = (first + second) / 2.0;
    let positions: Vec<[f32; 3]> = (around(&points[..4], first).into_iter())
        .chain(around(&points[4..], second))
        .map(|p| (c + p + (p - middle).signum() * margin).to_array())
        .collect();
    let mut corners = vec![0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6];
    for i in 0..4u16 {
        let j = (i + 1) % 4;
        corners.extend([i, 4 + i, j, j, 4 + i, 4 + j]);
    }
    // Each triangle its own corners, so that every face is flat.
    let flat = (corners.iter())
        .map(|&i| positions[usize::from(i)])
        .collect();
    mesh_of(flat, (0..corners.len() as u16).collect())
}

/// The sphere about the drawable's or the bound's box, which a camera frames.
pub(crate) fn bounds(req: &GeometryRequest<'_>) -> Option<(Vec3, f32)> {
    let DocPayload::Gta6Drawable(resource) = &req.doc.payload else {
        return None;
    };
    let (min, max) = match (drawable_of(resource, req.selection), &**resource) {
        (Some(d), _) => (d.box_min, d.box_max),
        (None, Resource::Bound(b)) => (b.box_min, b.box_max),
        (None, Resource::Cloth(c)) => return super::gta6_cloth::bounds(c),
        _ => return None,
    };
    let (min, max) = (Vec3::from(min), Vec3::from(max));
    let radius = (max - min).length() / 2.0;
    (radius.is_finite() && radius > 0.0).then(|| ((min + max) / 2.0, radius))
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{BoundChild, BoundGeometry, Model, Palette, Shader, Vertices};

    fn geometry(positions: Vec<[f32; 3]>, vertices: Vertices) -> Geometry {
        Geometry {
            shader: 0,
            vertex_count: positions.len() as u32,
            channels: Vec::new(),
            triangles: vec![[0, 1, 2]],
            positions: Ok(positions),
            vertices,
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        }
    }

    fn drawable(geometries: Vec<Geometry>) -> Drawable {
        Drawable {
            name: "box#dr".to_string(),
            box_min: [0.0; 3],
            box_max: [1.0, 1.0, 0.0],
            shaders: Vec::new(),
            levels: [
                Some(vec![Model {
                    geometries: geometries.into_iter().map(Ok).collect(),
                    palette: Ok(Palette::Own),
                    matrix_index: 0,
                }]),
                None,
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
    fn a_geometry_whose_palette_joins_the_posed_skeleton_moves_with_it_and_one_that_does_not_stays()
    {
        use or_io::gta6::hash::name_hash32;
        use or_io::gta6::headerless::Bone;
        let bone = |name: &str, [x, y, z]: [f32; 3]| Bone {
            name: name.to_string(),
            hash: name_hash32(name.as_bytes()),
            parent: None,
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [x, y, z],
            scale: [1.0; 3],
            inverse_bind: [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [-x, -y, -z],
            ],
        };
        let skeleton = Skeleton {
            bones: vec![
                bone("SKEL_ROOT", [0.0, 0.0, 1.0]),
                bone("SKEL_Head", [0.2, -0.1, 1.6]),
            ],
        };
        let triangle = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let lanes = Vertices {
            weights: vec![(16, vec![[1.0, 0.0, 0.0, 0.0]; 3])],
            joints: vec![(20, vec![[0; 4]; 3])],
            ..Vertices::default()
        };
        let with_palette = |hash: &[u8]| {
            let mut d = drawable(vec![geometry(triangle.clone(), lanes.clone())]);
            d.levels[0].as_mut().unwrap()[0].palette = Ok(Palette::Hashes(vec![name_hash32(hash)]));
            d
        };

        let mut meshes = meshes_at(
            &with_palette(b"SKEL_Head"),
            0,
            Some((&skeleton, None)),
            None,
            &|_| None,
        );
        let skinning = meshes[0].skinning.as_ref().expect("the geometry joins");
        assert_eq!(
            skinning.bones_used,
            [1],
            "palette entry 0 is the head, bone 1"
        );
        let step = Vec3::new(0.5, 0.25, -0.75);
        let skin = [Mat4::IDENTITY, Mat4::from_translation(step)];
        assert!(meshes[0].apply_pose(&skin, &mut Vec::new(), &mut Vec::new()));
        let moved = Vec3::from(meshes[0].vertices[1].position);
        assert!(
            moved.distance(Vec3::from(triangle[1]) + step) < 1e-5,
            "{moved}"
        );

        let unposed = meshes_at(&with_palette(b"SKEL_Head"), 0, None, None, &|_| None);
        assert!(unposed[0].skinning.is_none(), "no clip poses a skeleton");
        let missing = meshes_at(
            &with_palette(b"SKEL_Missing"),
            0,
            Some((&skeleton, None)),
            None,
            &|_| None,
        );
        assert!(missing[0].skinning.is_none(), "drawn at bind");
        assert_eq!(missing[0].vertices[1].position, triangle[1]);
    }

    #[test]
    fn a_level_with_no_models_draws_the_first_with_face_normals_where_none_is_stored() {
        let triangle = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let uvs = Vertices {
            uvs: vec![(28, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]])],
            ..Vertices::default()
        };
        let meshes = meshes_at(
            &drawable(vec![geometry(triangle.clone(), uvs)]),
            2,
            None,
            None,
            &|_| None,
        );
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].indices, [0, 1, 2]);
        let drawn: Vec<_> = meshes[0]
            .vertices
            .iter()
            .map(|v| (v.position, v.normal, v.uv))
            .collect();
        assert_eq!(
            drawn,
            [
                (triangle[0], [0.0, 0.0, 1.0], [0.0, 0.0]),
                (triangle[1], [0.0, 0.0, 1.0], [1.0, 0.0]),
                (triangle[2], [0.0, 0.0, 1.0], [0.0, 1.0]),
            ]
        );
        assert!(meshes[0].texture_name.is_none());
    }

    #[test]
    fn a_geometry_whose_positions_were_refused_draws_nothing() {
        let mut refused = geometry(vec![[0.0; 3]; 3], Vertices::default());
        refused.positions = Err(or_io::gta6::headerless::HeaderlessError::Positions {
            why: "the decoded vertices do not reach the geometry's box",
        });
        assert!(meshes_at(&drawable(vec![refused]), 0, None, None, &|_| None).is_empty());
    }

    #[test]
    fn the_channel_view_shows_a_reference_position_over_its_box_and_black_where_none_is_held() {
        let triangle = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let reference = Vertices {
            reference: vec![Some([2.0, -1.0, 0.5]), None, Some([4.0, 3.0, 0.5])],
            ..Vertices::default()
        };
        let d = drawable(vec![geometry(triangle, reference)]);
        let shown = Some(DebugChannel::Reference);
        let mesh = meshes_at(&d, 0, None, shown, &|_| None).remove(0);
        assert!(mesh.channels.contains(&DebugChannel::Reference));
        let colours: Vec<[f32; 4]> = mesh.vertices.iter().map(|v| v.colour).collect();
        assert_eq!(
            colours,
            [
                [0.25, 0.25, 0.25, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                [1.0, 1.0, 0.25, 1.0]
            ]
        );
        let bare = drawable(vec![geometry(vec![[0.0; 3]; 3], Vertices::default())]);
        let bare = meshes_at(&bare, 0, None, None, &|_| None).remove(0);
        assert!(!bare.channels.contains(&DebugChannel::Reference));
    }

    #[test]
    fn a_geometry_binds_its_diffuse_record_else_its_first_srgb_key_and_never_a_linear_one() {
        use or_io::gta6::headerless::Parameter;
        let (normal, colour, diffuse) = (0x0123_4567_89AB_CDEFu64, 0xC0105_u64, 0xD1F_u64);
        let texture = |hash: u32, slot: u8| Parameter {
            hash,
            value: ParameterValue::Texture { slot },
        };
        let drawn = |keys: Vec<Option<u64>>,
                     diffuse_slot: Option<u8>,
                     srgb: &dyn Fn(u64) -> Option<bool>| {
            let mut d = drawable(vec![geometry(vec![[0.0; 3]; 3], Vertices::default())]);
            let parameters = (0..keys.len() as u8)
                .map(|slot| match Some(slot) == diffuse_slot {
                    true => texture(DIFFUSE_TEX, slot),
                    false => texture(DIFFUSE_TEX ^ (1 + u32::from(slot)), slot),
                })
                .collect();
            d.shaders = vec![Shader {
                name_hash: 1,
                texture_keys: keys,
                parameters,
            }];
            meshes_at(&d, 0, None, None, srgb).remove(0).texture_name
        };
        // Every key in an open pack, `colour` alone as sRGB.
        let packed = |key: u64| Some(key == colour);
        let keys = vec![None, Some(normal), Some(colour), Some(diffuse)];
        assert_eq!(
            drawn(keys.clone(), Some(3), &packed),
            Some(texture_name(diffuse))
        );
        assert_eq!(
            drawn(keys.clone(), None, &packed),
            Some(texture_name(colour))
        );
        // A `DiffuseTex` record whose slot is unbound gives way to the rest of the rule.
        let unbound = vec![Some(normal), None, Some(colour)];
        assert_eq!(drawn(unbound, Some(1), &packed), Some(texture_name(colour)));
        // Data and normal maps alone: nothing is drawn as the colour.
        assert_eq!(drawn(keys.clone(), None, &|_| Some(false)), None);
        // The first key no open pack holds is the one asked for, past those a pack states as linear.
        let unopened = |key: u64| (key == normal).then_some(false);
        assert_eq!(drawn(keys, None, &unopened), Some(texture_name(colour)));
        assert!(texture_name(normal).ends_with("0123456789ABCDEF"));
    }

    #[test]
    fn a_material_with_no_colour_map_is_shaded_by_its_bump_record_and_one_with_a_colour_is_not() {
        use or_io::gta6::headerless::Parameter;
        let (specular, normal, colour) = (0x5EC_u64, 0xB0B_u64, 0xC0105_u64);
        let drawn = |keys: Vec<Option<u64>>, srgb: &dyn Fn(u64) -> Option<bool>| {
            let mut d = drawable(vec![geometry(vec![[0.0; 3]; 3], Vertices::default())]);
            let hashes = [BUMP_TEX ^ 1, BUMP_TEX, DIFFUSE_TEX ^ 9];
            let parameters = (hashes.iter().zip(0u8..))
                .map(|(&hash, slot)| Parameter {
                    hash,
                    value: ParameterValue::Texture { slot },
                })
                .collect();
            d.shaders = vec![Shader {
                name_hash: 1,
                texture_keys: keys,
                parameters,
            }];
            let mesh = meshes_at(&d, 0, None, None, srgb).remove(0);
            (mesh.texture_name, mesh.bump)
        };
        let linear = |_: u64| Some(false);
        // Data and normal maps alone: the normal map shades the mesh.
        assert_eq!(
            drawn(vec![Some(specular), Some(normal), None], &linear),
            (Some(texture_name(normal)), true)
        );
        // A colour map is drawn as the colour, and no normal map stands in for it.
        let keys = vec![Some(specular), Some(normal), Some(colour)];
        assert_eq!(
            drawn(keys, &|key| Some(key == colour)),
            (Some(texture_name(colour)), false)
        );
        // A `BumpTex` record whose slot is unbound shades nothing.
        assert_eq!(
            drawn(vec![Some(specular), None, None], &linear),
            (None, false)
        );
    }

    #[test]
    fn every_material_key_has_a_colour_no_other_key_has() {
        let colours: std::collections::HashSet<[u32; 4]> = (0..0x8000u16)
            .map(|key| key_tint(key).map(f32::to_bits))
            .collect();
        assert_eq!(colours.len(), 0x8000);
        assert_ne!(key_tint(1)[..3], key_tint(2)[..3]);
    }

    #[test]
    fn a_bound_draws_its_primitives_and_primitive_children_each_placed_by_its_child_s_matrix() {
        let triangle = Primitive::Triangle {
            vertices: [0, 1, 2],
            neighbours: [0xFFFF; 3],
            edges: [0xFF; 3],
        };
        let sphere = Primitive::Sphere {
            centre: 0,
            radius: 0.5,
        };
        let on_one_vertex = Primitive::Capsule {
            ends: [1, 1],
            radius: 0.5,
        };
        let no_axis = Primitive::Cylinder {
            ends: [2, 2],
            radius: 0.5,
        };
        let geometry = |primitives: Vec<Primitive>| BoundGeometry {
            box_min: [0.0; 3],
            box_max: [1.0; 3],
            vertices: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            primitives,
            material_ids: vec![0x11, 0x4200_0000_0022],
            primitive_materials: vec![0, 1, 0],
            bvh: None,
            unknown_shapes: vec![(3, or_io::gta6::headerless::UnknownShape::EndsOnOneVertex)],
        };
        let known = |primitives| {
            child_meshes(&BoundChild {
                matrix: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0; 4],
                ],
                flags: None,
                bound: Some(Ok(ChildBound::Geometry(geometry(primitives)))),
                material_id: None,
            })
        };
        let without = known(vec![triangle, sphere]);
        let geometry = geometry(vec![triangle, sphere, on_one_vertex, no_axis]);
        // A quarter turn about Z, then 10 m along X.
        let matrix = [
            [0.0, 1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [10.0, 0.0, 0.0, 0.0],
        ];
        let bound = Bound {
            radius: 1.0,
            box_min: [0.0; 3],
            box_max: [11.0, 1.0, 1.0],
            centroid: [0.0; 3],
            children: vec![
                BoundChild {
                    matrix,
                    flags: None,
                    bound: Some(Ok(ChildBound::Geometry(geometry))),
                    material_id: None,
                },
                BoundChild {
                    matrix,
                    flags: None,
                    bound: Some(Ok(ChildBound::Sphere {
                        centre: [0.0; 3],
                        radius: 1.0,
                    })),
                    material_id: Some(0x33),
                },
                BoundChild {
                    matrix,
                    flags: None,
                    bound: None,
                    material_id: None,
                },
            ],
            bvh: None,
        };
        let meshes = bound_meshes(&bound);
        assert_eq!(
            meshes.len(),
            2,
            "the geometry child's mesh, then the sphere's"
        );
        let sphere_vertices = without[0].vertices.len() - 3;
        assert_eq!(
            meshes[0].vertices.len(),
            3 + 3 * sphere_vertices,
            "the capsule on one vertex as a sphere, the cylinder on one as a marker"
        );
        assert_eq!(&meshes[0].indices[..3], [0, 1, 2], "the triangle first");
        let drawn = &meshes[0].vertices;
        let (sphere_drawn, rest) = drawn[3..].split_at(sphere_vertices);
        let (capsule_drawn, marker_drawn) = rest.split_at(sphere_vertices);
        assert!(drawn[..3].iter().all(|v| v.colour == key_tint(0x11)));
        assert!(sphere_drawn.iter().all(|v| v.colour == key_tint(0x22)));
        assert!(capsule_drawn.iter().all(|v| v.colour == key_tint(0x11)));
        assert_ne!(key_tint(0x11), key_tint(0x22));
        // Vertices 0, 1 and 2 placed: the turn takes x to y and y to -x.
        for (shape, centre, radius) in [
            (sphere_drawn, Vec3::new(10.0, 0.0, 0.0), 0.5),
            (capsule_drawn, Vec3::new(10.0, 1.0, 0.0), 0.5),
            (marker_drawn, Vec3::new(9.0, 0.0, 0.0), POINT_MARKER),
        ] {
            for v in shape {
                let from_centre = Vec3::from(v.position).distance(centre);
                assert!((from_centre - radius).abs() < 1e-4, "{radius}: {v:?}");
            }
        }
        assert!(meshes[1]
            .vertices
            .iter()
            .all(|v| v.colour == key_tint(0x33)));
        let at = |i: usize| Vec3::from(meshes[0].vertices[i].position);
        assert!(
            at(1).distance(Vec3::new(10.0, 1.0, 0.0)) < 1e-5,
            "{:?}",
            at(1)
        );
        assert!(
            at(2).distance(Vec3::new(9.0, 0.0, 0.0)) < 1e-5,
            "{:?}",
            at(2)
        );
        for v in &meshes[1].vertices {
            let from_centre = Vec3::from(v.position).distance(Vec3::new(10.0, 0.0, 0.0));
            assert!((from_centre - 1.0).abs() < 1e-4, "{v:?}");
        }
    }

    #[test]
    fn each_primitive_child_is_drawn_inside_its_box_and_a_hull_or_frustum_fills_it() {
        let centre = [0.5, -1.25, 2.0];
        let corners = [
            [0.2921, 0.2796, 0.3036],
            [-0.2921, 0.2796, -0.3036],
            [0.2921, 0.2796, -0.3036],
            [-0.2921, 0.2796, 0.3036],
            [0.2921, -0.2796, 0.3298],
            [-0.2921, -0.2796, -0.3298],
            [-0.2921, -0.2796, 0.3298],
            [0.2921, -0.2796, -0.3298],
        ];
        let ka = 0.8618;
        let shapes = [
            (
                ChildBound::Hull {
                    centre,
                    corners,
                    margin: 0.0399,
                },
                true,
            ),
            (
                ChildBound::Frustum {
                    centre,
                    radii: [0.3082, 0.3122],
                    factors: [ka, 1.0 / ka],
                    height: 0.0355,
                    margin: 0.0044,
                },
                true,
            ),
            (
                ChildBound::TaperedCapsule {
                    centre,
                    radii: [0.0303, 0.038],
                    length: 0.2317,
                },
                false,
            ),
            (
                ChildBound::Disc {
                    centre,
                    radius: 0.2456,
                    margin: 0.0246,
                },
                false,
            ),
            (
                ChildBound::Cylinder {
                    centre,
                    radius: 0.019,
                    half_height: 0.0052,
                },
                false,
            ),
        ];
        let shift = Vec3::new(10.0, 0.0, 0.0);
        for (shape, fills) in shapes {
            let (half, _) = shape.extent().expect("a primitive");
            let (low, high) = (
                Vec3::from(centre) - Vec3::from(half) + shift,
                Vec3::from(centre) + Vec3::from(half) + shift,
            );
            let mut matrix = [[0.0; 4]; 4];
            for (r, row) in matrix.iter_mut().enumerate().take(3) {
                row[r] = 1.0;
            }
            matrix[3] = [shift.x, shift.y, shift.z, 0.0];
            let bound = Bound {
                radius: 1.0,
                box_min: low.to_array(),
                box_max: high.to_array(),
                centroid: [0.0; 3],
                children: vec![BoundChild {
                    matrix,
                    flags: None,
                    bound: Some(Ok(shape)),
                    material_id: None,
                }],
                bvh: None,
            };
            let meshes = bound_meshes(&bound);
            assert!(!meshes.is_empty(), "{:?}", bound.children[0].bound);
            let points: Vec<Vec3> = (meshes.iter())
                .flat_map(|m| m.vertices.iter().map(|v| Vec3::from(v.position)))
                .collect();
            let (min, max) = points
                .iter()
                .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
            let slack = Vec3::splat(1e-4);
            assert!(
                min.cmpge(low - slack).all() && max.cmple(high + slack).all(),
                "{:?}: {min} to {max} outside {low} to {high}",
                bound.children[0].bound
            );
            if fills {
                assert!(
                    min.distance(low) < 1e-4 && max.distance(high) < 1e-4,
                    "{:?}: {min} to {max} short of {low} to {high}",
                    bound.children[0].bound
                );
            }
        }
    }

    #[test]
    fn each_primitive_child_is_wound_outward_to_face_its_normals_once_turned_and_placed() {
        let centre = [0.25, -0.5, 0.75];
        let shapes = [
            ChildBound::Sphere {
                centre,
                radius: 0.6,
            },
            ChildBound::Capsule {
                box_min: [-0.25, -1.5, 0.25],
                box_max: [0.75, 0.5, 1.25],
                radius: 0.5,
            },
            ChildBound::Disc {
                centre,
                radius: 0.8,
                margin: 0.05,
            },
            ChildBound::Cylinder {
                centre,
                radius: 0.4,
                half_height: 0.9,
            },
            ChildBound::Frustum {
                centre,
                radii: [0.3, 0.5],
                factors: [1.25, 0.8],
                height: 0.7,
                margin: 0.04,
            },
            ChildBound::TaperedCapsule {
                centre,
                radii: [0.2, 0.35],
                length: 0.6,
            },
            ChildBound::Hull {
                centre,
                corners: [
                    [0.3, 0.3, 0.3],
                    [-0.3, 0.3, 0.3],
                    [-0.3, 0.3, -0.3],
                    [0.3, 0.3, -0.3],
                    [0.3, -0.3, 0.35],
                    [-0.3, -0.3, 0.35],
                    [-0.3, -0.3, -0.35],
                    [0.3, -0.3, -0.35],
                ],
                margin: 0.04,
            },
        ];
        // A quarter turn about z, then 10 m along x.
        let matrix = [
            [0.0, 1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [10.0, 0.0, 0.0, 0.0],
        ];
        for shape in shapes {
            let name = format!("{shape:?}");
            let bound = Bound {
                radius: 2.0,
                box_min: [8.0, -2.0, -2.0],
                box_max: [12.0, 2.0, 2.0],
                centroid: [0.0; 3],
                children: vec![BoundChild {
                    matrix,
                    flags: None,
                    bound: Some(Ok(shape)),
                    material_id: None,
                }],
                bvh: None,
            };
            let meshes = bound_meshes(&bound);
            assert!(!meshes.is_empty(), "{name}");
            for mesh in &meshes {
                let p = |i: u32| Vec3::from(mesh.vertices[i as usize].position);
                let n = |i: u32| Vec3::from(mesh.vertices[i as usize].normal);
                // Every mesh is convex about the middle of its vertices.
                let middle = (mesh.vertices.iter().map(|v| Vec3::from(v.position))).sum::<Vec3>()
                    / mesh.vertices.len() as f32;
                let inward = (mesh.indices.as_chunks::<3>().0.iter())
                    .filter(|t| {
                        let face = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                        let out = p(t[0]) + p(t[1]) + p(t[2]) - 3.0 * middle;
                        // A corner normal far from its face shades a flat face as a curve.
                        let bent = t.iter().any(|&i| face.normalize().dot(n(i)) < 0.8);
                        face.length_squared() > 1e-12
                            && (face.dot(n(t[0]) + n(t[1]) + n(t[2])) <= 0.0
                                || face.dot(out) <= 0.0
                                || bent)
                    })
                    .count();
                assert_eq!(inward, 0, "{name}");
                assert!(
                    mesh.vertices.iter().all(|v| v.position[0] > 8.0),
                    "{name}: placed 10 m along x"
                );
            }
        }
    }
}
