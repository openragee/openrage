//! What a click in the viewport lands on: a ray through the cursor against what the viewport
//! draws, on the CPU.

use glam::{Vec2, Vec3};
use or_io::decl::{Gen8, Gen9};
use or_io::gta5::{CrSkeletonData, RmcDrawable};
use or_plugin::Pick;
use or_renderer::{LineVertex, OrbitCamera};

use super::pick_lines::{bone_under, bound_mesh_under, bound_under, navmesh_under};
use crate::render::meshes::{drawable_parts, level_drawn, Material, MeshUpload};
use crate::state::preview_scene::Picked;
use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::dock::Workspace;
use crate::ui::panels::selection_ids::{
    dict_entry_id, dict_entry_part_id, pack_index, pack_lod_sub_index, pack_selection,
    ydd_selected_entry_hash, ydd_selected_entry_index, ypt_drawable_for_selection, ypt_tag_of,
    ypt_unpack_model_hash, SECT_BOUND, SECT_SKEL, YPT_MODEL, YPT_MODEL_KIND_BONE,
    YPT_MODEL_KIND_BOUND, YPT_MODEL_KIND_LOD,
};

/// A ray from the near plane through one point of the viewport.
pub struct Ray {
    origin: Vec3,
    dir: Vec3,
    /// How far sideways one pixel reaches at one unit from the eye.
    pixel: f32,
    /// How far the near plane, where the ray starts, is from the eye.
    eye_gap: f32,
}

impl Ray {
    /// Through `at` in a viewport laid out over `rect` and drawn at `pixels`, as the renderer draws `camera`.
    pub fn at(
        camera: &OrbitCamera,
        rect: egui::Rect,
        pixels: (u32, u32),
        at: egui::Pos2,
    ) -> Option<Self> {
        let aspect = pixels.0.max(1) as f32 / pixels.1.max(1) as f32;
        let unproject = camera.view_proj(aspect).inverse();
        let cast = |at: egui::Pos2| {
            let uv = (at - rect.min) / rect.size();
            let ndc = Vec2::new(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
            // ⚠ Reverse-Z: depth 1 is the near plane and 0 the far one.
            let near = unproject.project_point3(ndc.extend(1.0));
            let far = unproject.project_point3(ndc.extend(0.0));
            Some((near, (far - near).try_normalize()?))
        };
        let (origin, dir) = cast(at)?;
        let (_, beside) = cast(at + egui::vec2(rect.width() / pixels.0.max(1) as f32, 0.0))?;
        origin.is_finite().then_some(Self {
            origin,
            dir,
            pixel: (beside - dir).length(),
            eye_gap: (origin - camera.eye()).length(),
        })
    }

    /// How far along the ray it passes the segment `a b`, and how many pixels from it, if within `pixels`.
    fn passes(&self, a: Vec3, b: Vec3, pixels: f32) -> Option<(f32, f32)> {
        let (v, w) = (b - a, self.origin - a);
        let (along, length, ahead, behind) = (self.dir.dot(v), v.dot(v), self.dir.dot(w), v.dot(w));
        // The point of the segment nearest the ray's line, then the point of the ray nearest that.
        let apart = length - along * along;
        let s = match apart > length * 1e-6 {
            true => ((behind - along * ahead) / apart).clamp(0.0, 1.0),
            false => 0.0,
        };
        let t = (s * along - ahead).max(0.0);
        let on_ray = self.origin + self.dir * t;
        let s = match length > 0.0 {
            true => ((on_ray - a).dot(v) / length).clamp(0.0, 1.0),
            false => 0.0,
        };
        let one_pixel = self.pixel * (t + self.eye_gap);
        let away = (on_ray - (a + v * s)).length() / one_pixel;
        (away <= pixels).then_some((t, away))
    }

    /// The segment of `lines`, pairs of vertices, nearest the ray within `pixels` and `shown` where the ray passes it: how far along, and how far from it.
    ///
    /// ⚠ `shown` is asked before the nearest is chosen. Edges that line up behind one another
    /// are the same distance from the cursor whatever it clicks, and which is nearer is decided
    /// by the last bit of the arithmetic, so a hidden one must not be able to win.
    pub(super) fn near_lines(
        &self,
        lines: &[LineVertex],
        pixels: f32,
        shown: impl Fn(f32) -> bool,
    ) -> Option<(f32, f32)> {
        let at = |v: &LineVertex| Vec3::from(v.position);
        (lines.as_chunks::<2>().0.iter())
            .filter_map(|pair| self.passes(at(&pair[0]), at(&pair[1]), pixels))
            .filter(|(t, _)| shown(*t))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// How far along the ray it meets the triangle, from either side.
    pub(super) fn meets(&self, [a, b, c]: [Vec3; 3]) -> Option<f32> {
        let (ab, ac) = (b - a, c - a);
        let p = self.dir.cross(ac);
        let inverse = ab.dot(p).recip();
        if !inverse.is_finite() {
            return None;
        }
        let s = self.origin - a;
        let u = s.dot(p) * inverse;
        let q = s.cross(ab);
        let v = self.dir.dot(q) * inverse;
        if u < 0.0 || v < 0.0 || u + v > 1.0 {
            return None;
        }
        Some(ac.dot(q) * inverse).filter(|t| *t >= 0.0)
    }

    /// Whether the ray passes through the box no further than `within`.
    fn enters(&self, min: Vec3, max: Vec3, within: f32) -> bool {
        let (a, b) = (
            (min - self.origin) / self.dir,
            (max - self.origin) / self.dir,
        );
        let (near, far) = (a.min(b).max_element().max(0.0), a.max(b).min_element());
        // ⚠ A ray along a face of the box divides zero by zero: test its triangles.
        !(near > far || near > within)
    }
}

/// A drawable the viewport draws, at either target.
enum Drawn<'a> {
    Gen8(&'a RmcDrawable<Gen8>),
    Gen9(&'a RmcDrawable<Gen9>),
}

/// How the Outline names the parts of a drawable it lists.
#[derive(Clone, Copy)]
pub(super) enum Named {
    /// By section, in a drawable's own tree.
    Drawable,
    /// Under the hash of a dictionary's entry, a fragment's drawable or a particle model.
    Entry(u32),
}

impl Named {
    fn geometry(self, lod: usize, model: usize, geometry: usize) -> u64 {
        match self {
            Self::Drawable => pack_selection(lod as u8, model as u16, Some(geometry as u32)),
            Self::Entry(hash) => {
                let sub = pack_lod_sub_index(lod as u8, model as u8, Some(geometry as u16));
                dict_entry_part_id(hash, YPT_MODEL_KIND_LOD, sub)
            }
        }
    }

    fn model(self, lod: usize, model: usize) -> u64 {
        match self {
            Self::Drawable => pack_selection(lod as u8, model as u16, None),
            Self::Entry(hash) => {
                let sub = pack_lod_sub_index(lod as u8, model as u8, None);
                dict_entry_part_id(hash, YPT_MODEL_KIND_LOD, sub)
            }
        }
    }

    pub(super) fn bone(self, index: u16) -> u64 {
        match self {
            Self::Drawable => pack_index(SECT_SKEL, index.into()),
            Self::Entry(hash) => dict_entry_part_id(hash, YPT_MODEL_KIND_BONE, index.into()),
        }
    }

    /// The row of the bound's `node`th node, where the Outline gives each one; an entry has the one row.
    pub(super) fn bound(self, node: u64) -> u64 {
        match self {
            Self::Drawable => pack_index(SECT_BOUND, node as u32),
            Self::Entry(hash) => dict_entry_part_id(hash, YPT_MODEL_KIND_BOUND, 0),
        }
    }
}

/// What a click landed on: in which document, which of its Outline rows, and how far along the ray.
pub(super) struct Hit {
    pub t: f32,
    pub doc: DocumentId,
    pub row: u64,
    /// What the Animation preview's box says was picked.
    pub label: String,
    /// The row, and the label, of the model a geometry belongs to.
    pub model: Option<(u64, String)>,
}

/// The drawables the viewport draws for `doc`, as the Outline names their parts.
fn drawn(state: &AppState, doc: DocumentId) -> Vec<(Drawn<'_>, Named)> {
    use or_io::Asset as A;
    let Some(payload) = state.document(doc).map(|d| &d.payload) else {
        return Vec::new();
    };
    let DocPayload::Resource(asset) = payload else {
        return Vec::new();
    };
    let animation = state.active_workspace == Workspace::Animation;
    let selected = state.selection.selected(doc);
    // What of `doc` is drawn: `None` a drawable or a whole dictionary, `Some` one entry.
    let shown: Vec<Option<usize>> = if animation {
        state
            .preview_scene
            .drawn_sources()
            .into_iter()
            .filter(|(source, _)| *source == doc)
            .map(|(_, entry)| entry)
            .collect()
    } else if let A::DrawableDict(_) | A::DrawableDictGen9(_) = asset {
        (selected.and_then(|sel| ydd_selected_entry_index(payload, sel)))
            .map(Some)
            .into_iter()
            .collect()
    } else {
        vec![None]
    };
    let entry = |codes: &[u32], index: usize| {
        let drawn = shown.contains(&None) || shown.contains(&Some(index));
        drawn.then(|| Named::Entry(codes[index]))
    };
    let fragment = Named::Entry(crate::state::textures::FRAGMENT_ENTRY_HASH);
    let model = selected
        .filter(|sel| !animation && ypt_tag_of(*sel) == YPT_MODEL)
        .map(|sel| (sel, Named::Entry(ypt_unpack_model_hash(sel))));
    let mut out = Vec::new();
    match asset {
        A::Drawable(d) if !shown.is_empty() => out.push((Drawn::Gen8(&d.base), Named::Drawable)),
        A::DrawableGen9(d) if !shown.is_empty() => {
            out.push((Drawn::Gen9(&d.base), Named::Drawable))
        }
        A::DrawableDict(dict) => {
            out.extend((dict.entries.iter().enumerate()).filter_map(|(i, d)| {
                Some((Drawn::Gen8(&d.as_deref()?.base), entry(&dict.codes, i)?))
            }))
        }
        A::DrawableDictGen9(dict) => {
            out.extend((dict.entries.iter().enumerate()).filter_map(|(i, d)| {
                Some((Drawn::Gen9(&d.as_deref()?.base), entry(&dict.codes, i)?))
            }))
        }
        A::Fragment(f) if !shown.is_empty() => {
            out.extend((f.common_drawable.as_deref()).map(|d| (Drawn::Gen8(&d.base), fragment)))
        }
        A::FragmentGen9(f) if !shown.is_empty() => {
            out.extend((f.common_drawable.as_deref()).map(|d| (Drawn::Gen9(&d.base), fragment)))
        }
        A::Particles(list) => out.extend(model.and_then(|(sel, named)| {
            Some((Drawn::Gen8(ypt_drawable_for_selection(list, sel)?), named))
        })),
        A::ParticlesGen9(list) => out.extend(model.and_then(|(sel, named)| {
            Some((Drawn::Gen9(ypt_drawable_for_selection(list, sel)?), named))
        })),
        _ => {}
    }
    out
}

/// The skeleton of each drawable the viewport draws for `doc`, as the Outline names its bones.
pub(super) fn skeletons(state: &AppState, doc: DocumentId) -> Vec<(&CrSkeletonData, Named)> {
    (drawn(state, doc).into_iter())
        .filter_map(|(drawable, named)| {
            let skeleton = match drawable {
                Drawn::Gen8(d) => d.skeleton.as_deref(),
                Drawn::Gen9(d) => d.skeleton.as_deref(),
            };
            Some((skeleton?, named))
        })
        .collect()
}

/// The documents a click can land in: the active one's, or the ones the Animation preview draws, which it draws with no document active as well.
fn reached_documents(state: &AppState) -> Vec<DocumentId> {
    match (state.active_workspace, state.active_document) {
        (Workspace::Animation, _) => state.preview_scene.drawn_documents(),
        (_, Some(active)) => vec![active],
        (_, None) => Vec::new(),
    }
}

/// The levels of detail the drawables the viewport draws have models at, each with the distance the
/// first drawable that has it draws it to.
///
/// ⚠ The Animation preview draws every mesh source at the one level chosen, and a drawable without
/// it draws level 0, so a level any of them has is one to offer.
pub fn levels(state: &AppState) -> Vec<(usize, f32)> {
    fn of<T: or_io::gta5::DrawableTarget>(drawable: &RmcDrawable<T>) -> Vec<(usize, f32)> {
        let group = &drawable.lod_group;
        (group.lods.iter().zip(group.lod_distances).enumerate())
            .filter(|(_, (lod, _))| !lod.models.is_empty())
            .map(|(level, (_, distance))| (level, distance))
            .collect()
    }
    let mut out: Vec<(usize, f32)> = Vec::new();
    let documents = reached_documents(state);
    let gta6 = (documents.iter()).map(|doc| crate::render::gta6_drawable::levels(state, *doc));
    let gta5 =
        (documents.iter().flat_map(|doc| drawn(state, *doc))).map(|(drawable, _)| match drawable {
            Drawn::Gen8(d) => of(d),
            Drawn::Gen9(d) => of(d),
        });
    for found in gta5.chain(gta6) {
        for (level, distance) in found {
            if out.iter().all(|(seen, _)| *seen != level) {
                out.push((level, distance));
            }
        }
    }
    out.sort_by_key(|(level, _)| *level);
    out
}

/// The geometry of `doc`'s `drawable` nearest along `ray` of the level the viewport draws, posed as
/// it poses it.
fn nearest_part<T: Material>(
    drawable: &RmcDrawable<T>,
    doc: DocumentId,
    named: Named,
    state: &AppState,
    ray: &Ray,
) -> Option<Hit> {
    let lod = level_drawn(drawable, state.preview_scene.lod);
    let mut nearest: Option<(f32, (usize, usize))> = None;
    for (part, mut upload) in drawable_parts(drawable, lod, None) {
        posed_as_drawn(&mut upload, state);
        let within = nearest.map_or(f32::INFINITY, |(t, _)| t);
        if let Some(t) = nearest_triangle(&upload, ray, within) {
            nearest = Some((t, part));
        }
    }
    let (t, (model, geometry)) = nearest?;
    Some(Hit {
        t,
        doc,
        row: named.geometry(lod, model, geometry),
        label: format!("Geometry {geometry} of model {model}"),
        model: Some((named.model(lod, model), format!("Model {model}"))),
    })
}

/// The geometry of `doc`'s GTA VI drawable nearest along `ray` of the level the model view draws,
/// at bind, named as the Outline names it.
fn gta6_part(state: &AppState, doc: DocumentId, ray: &Ray) -> Option<Hit> {
    use crate::ui::panels::selection_ids::{dict_entry_unpack_part, is_dict_entry_selection};
    let (lod, parts) = crate::render::gta6_drawable::drawn_geometries(state, doc)?;
    let named = match state.selection.selected(doc) {
        Some(sel) if is_dict_entry_selection(sel) => Named::Entry(dict_entry_unpack_part(sel).0),
        _ => Named::Drawable,
    };
    let mut nearest: Option<(f32, (usize, usize))> = None;
    for (model, geometry, upload) in &parts {
        let within = nearest.map_or(f32::INFINITY, |(t, _)| t);
        if let Some(t) = nearest_triangle(upload, ray, within) {
            nearest = Some((t, (*model, *geometry)));
        }
    }
    let (t, (model, geometry)) = nearest?;
    Some(Hit {
        t,
        doc,
        row: named.geometry(lod, model, geometry),
        label: format!("Geometry {geometry} of model {model}"),
        model: Some((named.model(lod, model), format!("Model {model}"))),
    })
}

/// The retarget and the pose the viewport's mesh cache and skinning give an upload.
fn posed_as_drawn(upload: &mut MeshUpload, state: &AppState) {
    if let Some(binding) = &state.anim.binding {
        upload.retarget_onto(&binding.skeleton);
    }
    upload.apply_pose(
        super::posing::skin_of(state).1,
        &mut Vec::new(),
        &mut Vec::new(),
    );
}

/// How far along `ray` the nearest of the upload's triangles is, if nearer than `within`.
fn nearest_triangle(upload: &MeshUpload, ray: &Ray, within: f32) -> Option<f32> {
    nearest_polygon(upload, ray, within).map(|(t, _)| t)
}

/// How far along `ray` the nearest of the upload's triangles is, and its index among them, if nearer than `within`.
pub(super) fn nearest_polygon(upload: &MeshUpload, ray: &Ray, within: f32) -> Option<(f32, usize)> {
    let corner = |i: u32| {
        upload
            .vertices
            .get(i as usize)
            .map(|v| Vec3::from(v.position))
    };
    let (min, max) = (upload.vertices.iter())
        .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(min, max), v| {
            (min.min(v.position.into()), max.max(v.position.into()))
        });
    if !ray.enters(min, max, within) {
        return None;
    }
    (upload.indices.as_chunks::<3>().0.iter())
        .enumerate()
        .filter_map(|(at, t)| {
            Some((
                ray.meets([corner(t[0])?, corner(t[1])?, corner(t[2])?])?,
                at,
            ))
        })
        .filter(|(t, _)| *t < within)
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The index, in the upload's vertices, of the corner of triangle `polygon` nearest the point `ray` meets it at `t` along.
fn nearest_corner(upload: &MeshUpload, ray: &Ray, t: f32, polygon: usize) -> u32 {
    let hit = ray.origin + ray.dir * t;
    let distance = |i: u32| {
        let at = upload
            .vertices
            .get(i as usize)
            .map(|v| Vec3::from(v.position));
        at.map_or(f32::INFINITY, |at| at.distance_squared(hit))
    };
    (upload.indices[polygon * 3..polygon * 3 + 3].iter().copied())
        .min_by(|a, b| distance(*a).total_cmp(&distance(*b)))
        .expect("a triangle has three corners")
}

/// What a click lands on in the active document: the geometry of the level drawn nearest along `ray`, its triangle under the cursor and that triangle's corner nearest the hit.
///
/// ⚠ The indices are those of the mesh the viewport draws, which is the geometry's own except
/// that a corner whose vertex is not in the buffer is dropped from the index list, so the triangles
/// after it count one fewer; the vertex indices are always the buffer's.
pub fn pick_under(state: &AppState, ray: &Ray) -> Option<Pick> {
    fn nearest<T: Material>(
        drawable: &RmcDrawable<T>,
        state: &AppState,
        ray: &Ray,
        best: &mut Option<(f32, Pick)>,
    ) {
        let lod = level_drawn(drawable, state.preview_scene.lod);
        for ((model, geometry), mut upload) in drawable_parts(drawable, lod, None) {
            posed_as_drawn(&mut upload, state);
            let within = best.map_or(f32::INFINITY, |(t, _)| t);
            if let Some((t, polygon)) = nearest_polygon(&upload, ray, within) {
                let pick = Pick {
                    lod: lod as u32,
                    model: model as u32,
                    geometry: geometry as u32,
                    polygon: polygon as u32,
                    vertex: nearest_corner(&upload, ray, t, polygon),
                };
                *best = Some((t, pick));
            }
        }
    }
    let doc = state.active_document?;
    let mut best = None;
    for (drawable, _) in drawn(state, doc) {
        match drawable {
            Drawn::Gen8(d) => nearest(d, state, ray, &mut best),
            Drawn::Gen9(d) => nearest(d, state, ray, &mut best),
        }
    }
    best.map(|(_, pick)| pick)
}

/// Select what the viewport draws nearest along `ray`, a geometry, and reveal it in the Outline.
///
/// A bone, or its bind-pose ghost, is picked first, since bones draw in front of everything. A
/// bound drawn as lines, which Collision draws for a drawable and Wireframe for a fragment's
/// physics and a `.ybn`, is picked where no mesh hides it. In the Animation workspace the ray reaches
/// every mesh the preview draws, in the document it comes from; a pick in a document other than the
/// active one is noted in `PreviewScene::picked`, with its row for [`open_picked`], and leaves the
/// selection alone. A click on nothing clears the selection down to the dictionary entry or
/// particle model the viewport draws, which it needs selected to draw at all.
pub fn select_under(state: &mut AppState, ray: &Ray) {
    pick(state, ray, false);
}

/// Select as [`select_under`] does, but the model that holds a geometry rather than the geometry.
pub fn select_model_under(state: &mut AppState, ray: &Ray) {
    pick(state, ray, true);
}

fn pick(state: &mut AppState, ray: &Ray, whole_model: bool) {
    let Some(active) = state.active_document else {
        return;
    };
    let reached = reached_documents(state);
    let drawn: Vec<_> = (reached.iter())
        .flat_map(|doc| {
            drawn(state, *doc)
                .into_iter()
                .map(|(d, named)| (*doc, d, named))
        })
        .collect();
    let mesh = (drawn.iter())
        .filter_map(|(doc, drawable, named)| match drawable {
            Drawn::Gen8(d) => nearest_part(d, *doc, *named, state, ray),
            Drawn::Gen9(d) => nearest_part(d, *doc, *named, state, ray),
        })
        .chain(gta6_part(state, active, ray))
        .min_by(|a, b| a.t.total_cmp(&b.t));
    let named = (drawn.iter())
        .find(|(doc, ..)| *doc == active)
        .map(|(_, _, named)| *named);
    let bound = bound_under(state, named, ray, mesh.as_ref().map(|m| m.t))
        .or_else(|| bound_mesh_under(state, ray))
        .or_else(|| navmesh_under(state, ray))
        .or_else(|| super::pick_gta6_bound::gta6_bound_under(state, ray));
    let bone = bone_under(state, ray);
    // ⚠ Lines draw with no mesh: a bone while every chosen mesh draws nothing, and a `.ybn`'s bounds.
    // A click that lands on none of them, with nothing drawn, may be on a document the viewport
    // does not draw at all, and leaves its selection alone.
    if drawn.is_empty() && mesh.is_none() && bone.is_none() && bound.is_none() {
        return;
    }
    state.preview_scene.picked = None;
    match bone.or(bound).or(mesh) {
        // The Outline lists the active document alone: a pick in another is noted in the preview.
        Some(hit) if hit.doc != active => {
            let (row, label) = match hit.model.filter(|_| whole_model) {
                Some(model) => model,
                None => (hit.row, hit.label),
            };
            state.preview_scene.picked = Some(Picked {
                document: hit.doc,
                label,
                row,
            });
        }
        Some(hit) => reveal(
            state,
            hit.doc,
            hit.model.filter(|_| whole_model).map_or(hit.row, |m| m.0),
        ),
        None => {
            for doc in reached.into_iter().filter(|doc| *doc == active) {
                match drawn_because_selected(state, doc) {
                    Some(row) => reveal(state, doc, row),
                    None => state.selection.clear(doc),
                }
            }
        }
    }
}

/// Select `row` of the active document `doc`, and have the Outline scroll to it.
fn reveal(state: &mut AppState, doc: DocumentId, row: u64) {
    state.selection.select(doc, row);
    crate::ui::panels::hierarchy::ui_state(state).scroll_to = Some((doc, row));
}

/// Open the document the Animation preview's last pick is in, with the picked row selected and shown in its Outline.
///
/// ⚠ A click only holds the pick: the Outline follows the clip dictionary there, and a click that
/// took it to the rig would take the user off the clip list on every pick.
pub fn open_picked(state: &mut AppState) {
    let Some(picked) = state.preview_scene.picked.clone() else {
        return;
    };
    if state.document(picked.document).is_none() {
        return;
    }
    state.set_active_document(picked.document);
    reveal(state, picked.document, picked.row);
    let workspace = state.active_workspace;
    (state.workspace_docks).reveal(workspace, crate::ui::panels::registry::ids::OUTLINE);
}

/// The dictionary entry or particle model the viewport draws only because it is selected.
fn drawn_because_selected(state: &AppState, doc: DocumentId) -> Option<u64> {
    use or_io::Asset as A;
    if state.active_workspace == Workspace::Animation {
        return None;
    }
    let sel = state.selection.selected(doc)?;
    let payload = &state.document(doc)?.payload;
    let hash = match payload {
        DocPayload::Resource(A::DrawableDict(_) | A::DrawableDictGen9(_)) => {
            ydd_selected_entry_hash(payload, sel)?
        }
        DocPayload::Resource(A::Particles(_) | A::ParticlesGen9(_)) => {
            (ypt_tag_of(sel) == YPT_MODEL).then(|| ypt_unpack_model_hash(sel))?
        }
        _ => return None,
    };
    Some(dict_entry_id(hash))
}

#[cfg(test)]
mod tests {
    use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexLayout};
    use or_io::decl::{Gen8, Hash32};
    use or_io::gta5::drawable::gen8;
    use or_io::gta5::{GrmGeometry, GrmModel, GrmShaderGroup, GtaDrawable, RmcLod};

    use super::*;
    use crate::state::{Document, Prefs};

    /// A drawable of one model with one geometry: the unit square in the XZ plane, as two triangles,
    /// `(0 1 2)` below the diagonal from the origin and `(0 2 3)` above it.
    fn square() -> GtaDrawable<Gen8> {
        let mut fvf = gen8::GrcFvf::default();
        for channel in [GrcFvfChannel::Position, GrcFvfChannel::Normal] {
            fvf.fvf |= 1 << channel as u32;
            fvf.channel_sizes |= (GrcDataSize::Float3 as u64) << (channel as u64 * 4);
        }
        fvf.channel_count = 2;
        fvf.fvf_size = VertexLayout::of(fvf.fvf, fvf.channel_sizes).packed_stride() as u8;

        let mut data = Vec::new();
        for point in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ] {
            for axis in point.into_iter().chain([0.0, 1.0, 0.0]) {
                data.extend_from_slice(&axis.to_le_bytes());
            }
        }
        let geometry = GrmGeometry::<Gen8> {
            primitive_type: 3,
            vertex_buffer: Box::new(gen8::GrcVertexBuffer {
                data,
                fvf: Some(Box::new(fvf)),
                ..Default::default()
            }),
            index_buffer: Box::new(gen8::GrcIndexBuffer {
                indices: vec![0, 1, 2, 0, 2, 3],
                ..Default::default()
            }),
            ..Default::default()
        };
        let model = GrmModel {
            aabbs: vec![Default::default()],
            shader_indices: vec![0],
            geometries: vec![Box::new(geometry)],
            ..Default::default()
        };
        let mut drawable = GtaDrawable::<Gen8>::default();
        drawable.base.shader_group = Some(Box::new(GrmShaderGroup {
            shaders: vec![Box::new(gen8::GrcInstanceData {
                basis_hash: Hash32::of("default"),
                ..Default::default()
            })],
            ..Default::default()
        }));
        drawable.base.lod_group.lods[0] = RmcLod {
            models: vec![Box::new(model)],
            ..Default::default()
        }
        .into();
        drawable
    }

    fn opened() -> AppState {
        let mut state = AppState::from_prefs(Prefs::default());
        let payload = DocPayload::Resource(or_io::Asset::Drawable(square()));
        state
            .documents
            .push(Document::new(1, Some("square.ydr".into()), payload));
        state.active_document = Some(1);
        state
    }

    /// A ray along +Y through the square's plane at `(x, z)`.
    fn through(x: f32, z: f32) -> Ray {
        Ray {
            origin: Vec3::new(x, -5.0, z),
            dir: Vec3::Y,
            pixel: 0.0,
            eye_gap: 0.0,
        }
    }

    #[test]
    fn a_pick_names_the_geometry_the_triangle_and_the_corner_nearest_the_hit() {
        let state = opened();
        let pick = |x, z| pick_under(&state, &through(x, z));

        assert_eq!(
            pick(0.9, 0.2),
            Some(Pick {
                lod: 0,
                model: 0,
                geometry: 0,
                polygon: 0,
                vertex: 1
            }),
            "below the diagonal, nearest the corner at (1, 0, 0)"
        );
        assert_eq!(
            pick(0.1, 0.8),
            Some(Pick {
                lod: 0,
                model: 0,
                geometry: 0,
                polygon: 1,
                vertex: 3
            }),
            "above it, nearest the corner at (0, 0, 1)"
        );
        assert_eq!(pick(2.0, 2.0), None, "a click on nothing lands on nothing");
    }

    #[test]
    fn a_click_on_a_gta_vi_geometry_selects_its_row_and_in_a_dictionary_under_its_entry() {
        use or_io::gta6::headerless::{
            Dictionary, Drawable, Entry, Geometry, Model, Palette, Resource, Vertices,
        };
        let square = |g: usize| Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 0,
            vertex_count: 4,
            channels: Vec::new(),
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            positions: Ok(vec![
                [g as f32, 0.0, 0.0],
                [g as f32 + 1.0, 0.0, 0.0],
                [g as f32 + 1.0, 0.0, 1.0],
                [g as f32, 0.0, 1.0],
            ]),
            vertices: Vertices::default(),
        };
        let drawable = || Drawable {
            name: "d".to_string(),
            box_min: [0.0; 3],
            box_max: [2.0, 0.0, 1.0],
            shaders: Vec::new(),
            levels: [
                Some(vec![Model {
                    geometries: vec![Ok(square(0)), Ok(square(1))],
                    palette: Ok(Palette::Own),
                    matrix_index: 0,
                }]),
                None,
                None,
                None,
            ],
            lod_distances: [10.0; 4],
            bound: None,
            embedded: None,
            extra_skeletons: Vec::new(),
        };
        let open = |resource| {
            let mut state = AppState::from_prefs(Prefs::default());
            let payload = DocPayload::Gta6Drawable(std::sync::Arc::new(resource));
            (state.documents).push(Document::new(1, Some("0x10.drawable".into()), payload));
            state.active_document = Some(1);
            state
        };
        let mut state = open(Resource::Drawable(drawable()));
        select_under(&mut state, &through(1.5, 0.5));
        assert_eq!(
            state.selection.selected(1),
            Some(pack_selection(0, 0, Some(1)))
        );

        let entry = Entry::Drawable(Box::new(drawable()));
        let mut state = open(Resource::Dictionary(Dictionary {
            entries: vec![(0x20, Ok(entry))],
        }));
        state.selection.select(1, dict_entry_id(0x20));
        select_under(&mut state, &through(0.5, 0.5));
        let geometry = pack_lod_sub_index(0, 0, Some(0));
        assert_eq!(
            state.selection.selected(1),
            Some(dict_entry_part_id(0x20, YPT_MODEL_KIND_LOD, geometry))
        );
    }
}
