use or_anim::SkinnedMesh;
use or_io::codec::mesh::{GrcDataSize, GrcFvfChannel, VertexReader, VertexSource};
use or_io::decl::{Gen8, Gen9};
use or_io::gta5::{
    gen8, gen9, DrawableTarget, FragType, GrmShaderGroup, GtaDrawable, PgDictionary, RmcDrawable,
};

use super::debug_channel::{offered_by, DebugChannel};
use or_renderer::MeshVertex;

use crate::state::capabilities::{self, GeometryRequest};
use crate::state::{AppState, DocPayload, MeshSource};
use crate::ui::panels::selection_ids::{
    ydd_selected_entry_index, yld_selected_cloth, ypt_drawable_for_selection, ypt_tag_of, YPT_MODEL,
};

pub struct MeshUpload {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub texture_name: Option<String>,
    /// Whether `texture_name` names a normal map: Material shades the surface by it and takes no colour from it.
    pub bump: bool,
    pub skinning: Option<SkinnedMesh>,
    /// The palette's bones as index and tag, where the drawable carried the
    /// skeleton its palette indexes.
    ///
    /// ⚠ **A matrix palette is an index into one particular skeleton**, and
    /// the preview picks its mesh source and its skeleton source separately.
    /// Empty where the drawable carries no skeleton — a clothing component
    /// usually does not — in which case the file's own indices are all there
    /// is and [`Self::retarget_onto`] says so rather than guessing.
    pub palette_bones: Vec<or_anim::PaletteBone>,
    /// Which channels this geometry's own declaration carries.
    ///
    /// ⚠ Metadata rather than data — at most thirteen entries, whatever the
    /// vertex count. It is what lets the selector offer what the file has
    /// without re-reading the geometry, and it is *not* the channel values;
    /// those are packed into `vertices` only for the selected channel.
    pub channels: Vec<DebugChannel>,
    /// The Animation preview's mesh source this was built for, whose own embedded textures its material binds before any dictionary's.
    pub source: Option<MeshSource>,
}

impl MeshUpload {
    pub fn is_skinned(&self) -> bool {
        self.skinning.is_some()
    }

    /// Move this mesh's influences onto `target`.
    ///
    /// ⚠ **Called once when the skeleton changes, never per frame.** The
    /// influences hold bone numbers that `apply_pose` uses as indices into
    /// the skin matrices, so the remap has to happen before posing — and
    /// doing it again on an already-retargeted mesh would compose two
    /// mappings.
    pub fn retarget_onto(
        &mut self,
        target: &or_anim::Skeleton,
    ) -> Option<or_anim::PaletteRetarget> {
        let skinning = self.skinning.as_mut()?;
        if self.palette_bones.is_empty() {
            return None;
        }
        Some(or_anim::retarget_mesh(
            skinning,
            &self.palette_bones,
            target,
        ))
    }

    pub fn apply_pose(
        &mut self,
        skin_matrices: &[glam::Mat4],
        positions: &mut Vec<[f32; 3]>,
        normals: &mut Vec<[f32; 3]>,
    ) -> bool {
        let Some(skinning) = self.skinning.as_ref() else {
            return false;
        };
        if skinning.is_empty() || skin_matrices.is_empty() {
            return false;
        }
        skinning.skin_into(skin_matrices, positions, normals);
        for (index, vertex) in self.vertices.iter_mut().enumerate() {
            if let Some(position) = positions.get(index) {
                vertex.position = *position;
            }
            if let Some(normal) = normals.get(index) {
                vertex.normal = *normal;
            }
        }
        true
    }
}

pub fn build_meshes_for_active_document(state: &AppState) -> Vec<MeshUpload> {
    if state.active_workspace == crate::ui::dock::Workspace::Animation {
        // A GTA VI animation draws the drawable picked for it, posed where it binds.
        if state.active_document().map(|d| d.kind) == Some(crate::state::DocumentKind::Gta6Anim) {
            let model = crate::ui::panels::inspector::gta6_model_drawn(state);
            let Some(doc) = model.and_then(|(m, _)| state.document(m)) else {
                return Vec::new();
            };
            let request = GeometryRequest {
                state,
                doc,
                selection: state.selection.selected(doc.id),
                entry: None,
            };
            return crate::render::gta6_drawable::meshes(&request);
        }
        return build_meshes_for_preview_scene(state);
    }
    let Some(doc) = state.active_document() else {
        return Vec::new();
    };
    let request = GeometryRequest {
        state,
        doc,
        selection: state.selection.selected(doc.id),
        entry: None,
    };
    capabilities::for_payload(&doc.payload)
        .and_then(|row| row.meshes)
        .map(|build| build(&request))
        .unwrap_or_default()
}

/// The meshes a drawable, a drawable dictionary or a fragment draws at its first level of detail, for a view that holds no document.
#[cfg(test)]
pub(crate) fn meshes_of_asset(asset: &or_io::Asset) -> Vec<MeshUpload> {
    meshes_of_asset_at(asset, 0)
}

/// The meshes the same draws at exactly level `lod`, none where it has no models there.
#[cfg(test)]
pub(crate) fn meshes_of_asset_at(asset: &or_io::Asset, lod: usize) -> Vec<MeshUpload> {
    meshes_of_asset_in(asset, lod, None)
}

/// The same with the colours of `channel` packed into the vertices of every mesh that has it, as the viewport's debug views draw them.
pub(crate) fn meshes_of_asset_in(
    asset: &or_io::Asset,
    lod: usize,
    channel: Option<DebugChannel>,
) -> Vec<MeshUpload> {
    use or_io::Asset;
    fn drawn<T: Material>(
        drawable: &RmcDrawable<T>,
        lod: usize,
        channel: Option<DebugChannel>,
    ) -> Vec<MeshUpload> {
        drawable_parts(drawable, lod, channel)
            .into_iter()
            .map(|(_, upload)| upload)
            .collect()
    }
    match asset {
        Asset::Drawable(d) => drawn(&d.base, lod, channel),
        Asset::DrawableGen9(d) => drawn(&d.base, lod, channel),
        Asset::DrawableDict(dict) => (dict.entries.iter().flatten())
            .flat_map(|d| drawn(&d.base, lod, channel))
            .collect(),
        Asset::DrawableDictGen9(dict) => (dict.entries.iter().flatten())
            .flat_map(|d| drawn(&d.base, lod, channel))
            .collect(),
        Asset::Fragment(frag) => (frag.common_drawable.as_deref())
            .map(|d| drawn(&d.base, lod, channel))
            .unwrap_or_default(),
        Asset::FragmentGen9(frag) => (frag.common_drawable.as_deref())
            .map(|d| drawn(&d.base, lod, channel))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// What builds a tile's meshes on a worker thread.
pub(crate) type TileJob = Box<dyn FnOnce() -> Vec<MeshUpload> + Send>;

/// The meshes of `selection`'s tile as a job for a worker, over a copy of that one model; `None` for a document whose tile is built from the state it stands in.
///
/// ⚠ A copy and not the document: the worker outlives the frame, and a model's bytes copy in
/// a fraction of the time its meshes take to build.
pub(crate) fn tile_job(
    payload: &DocPayload,
    selection: u64,
    lod: usize,
    channel: Option<DebugChannel>,
) -> Option<TileJob> {
    use or_io::Asset;
    fn drawn<T: Material>(
        drawable: &RmcDrawable<T>,
        lod: usize,
        channel: Option<DebugChannel>,
    ) -> TileJob
    where
        RmcDrawable<T>: Clone + Send + 'static,
    {
        let drawable = drawable.clone();
        Box::new(move || drawable_meshes(&drawable, lod, channel))
    }
    fn fragment<T: Material>(
        fragment: &FragType<T>,
        lod: usize,
        channel: Option<DebugChannel>,
    ) -> TileJob
    where
        FragType<T>: Clone + Send + 'static,
    {
        let fragment = fragment.clone();
        Box::new(move || fragment_meshes(&fragment, lod, channel))
    }
    let DocPayload::Resource(asset) = payload else {
        return None;
    };
    let entry = || ydd_selected_entry_index(payload, selection);
    Some(match asset {
        Asset::Drawable(d) => drawn(&d.base, lod, channel),
        Asset::DrawableGen9(d) => drawn(&d.base, lod, channel),
        Asset::DrawableDict(dict) => {
            drawn(&dict.entries.get(entry()?)?.as_deref()?.base, lod, channel)
        }
        Asset::DrawableDictGen9(dict) => {
            drawn(&dict.entries.get(entry()?)?.as_deref()?.base, lod, channel)
        }
        Asset::Fragment(frag) => fragment(frag, lod, channel),
        Asset::FragmentGen9(frag) => fragment(frag, lod, channel),
        _ => return None,
    })
}

pub(crate) mod rows {
    use super::*;

    pub(crate) fn ydr(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        let (lod, channel) = (
            req.state.preview_scene.lod,
            req.state.preview_scene.debug_channel,
        );
        match &req.doc.payload {
            DocPayload::Resource(or_io::Asset::Drawable(d)) => {
                drawable_meshes(&d.base, lod, channel)
            }
            DocPayload::Resource(or_io::Asset::DrawableGen9(d)) => {
                drawable_meshes(&d.base, lod, channel)
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn ydd(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        let Some(index) = req
            .selection
            .and_then(|sel| ydd_selected_entry_index(&req.doc.payload, sel))
        else {
            return Vec::new();
        };
        dictionary_meshes(req, Some(index))
    }

    pub(crate) fn yft(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        let (lod, channel) = (
            req.state.preview_scene.lod,
            req.state.preview_scene.debug_channel,
        );
        match &req.doc.payload {
            DocPayload::Resource(or_io::Asset::Fragment(frag)) => {
                fragment_meshes(frag, lod, channel)
            }
            DocPayload::Resource(or_io::Asset::FragmentGen9(frag)) => {
                fragment_meshes(frag, lod, channel)
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn ybn(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        match &req.doc.payload {
            DocPayload::Resource(or_io::Asset::Bounds(bound)) => {
                crate::render::bound_meshes::build_bound_meshes(
                    bound,
                    req.state.materials_palette.palette_ref(),
                )
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn yld(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        let Some(cloth) = req
            .selection
            .and_then(|sel| yld_selected_cloth(&req.doc.payload, sel))
        else {
            return Vec::new();
        };
        let mut meshes = build_cloth_meshes(cloth);
        if let Some(bound) = cloth.bound_composite.as_ref() {
            meshes.extend(crate::render::bound_meshes::build_composite_meshes(
                bound,
                req.state.materials_palette.palette_ref(),
            ));
        }
        meshes
    }

    pub(crate) fn ypt(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        build_ypt_meshes(
            &req.doc.payload,
            req.selection,
            req.state.preview_scene.lod,
            req.state.preview_scene.debug_channel,
        )
    }

    pub(crate) fn ydr_scene(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        ydr(req)
    }

    pub(crate) fn yft_scene(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        yft(req)
    }

    pub(crate) fn ydd_scene(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        dictionary_meshes(req, req.entry)
    }

    /// The entry `index` of a drawable dictionary, or every entry when `None`.
    fn dictionary_meshes(req: &GeometryRequest<'_>, index: Option<usize>) -> Vec<MeshUpload> {
        fn chosen<T: Material>(
            dict: &PgDictionary<GtaDrawable<T>>,
            index: Option<usize>,
            (lod, channel): (usize, Option<DebugChannel>),
        ) -> Vec<MeshUpload> {
            let entries: Vec<&GtaDrawable<T>> = match index {
                Some(index) => dict
                    .entries
                    .get(index)
                    .and_then(Option::as_deref)
                    .into_iter()
                    .collect(),
                None => dict.entries.iter().flatten().map(|d| &**d).collect(),
            };
            entries
                .into_iter()
                .flat_map(|d| drawable_meshes(&d.base, lod, channel))
                .collect()
        }
        let drawn = (
            req.state.preview_scene.lod,
            req.state.preview_scene.debug_channel,
        );
        match &req.doc.payload {
            DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => chosen(dict, index, drawn),
            DocPayload::Resource(or_io::Asset::DrawableDictGen9(dict)) => {
                chosen(dict, index, drawn)
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn yld_scene(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        let DocPayload::Resource(or_io::Asset::ClothDict(dict)) = &req.doc.payload else {
            return Vec::new();
        };
        match req
            .entry
            .and_then(|i| dict.entries.get(i))
            .and_then(|c| c.as_deref())
        {
            Some(cloth) => build_cloth_meshes(cloth),
            None => Vec::new(),
        }
    }

    pub(crate) fn ybn_scene(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        ybn(req)
    }

    /// Every polygon as a fan in one neutral colour: no polygon flag bit the corpus sets has a known meaning to colour by.
    pub(crate) fn ynv(req: &GeometryRequest<'_>) -> Vec<MeshUpload> {
        const FILL: [f32; 4] = [0.36, 0.52, 0.26, 1.0];
        let DocPayload::Resource(or_io::Asset::NavMesh(mesh)) = &req.doc.payload else {
            return Vec::new();
        };
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        for corners in crate::render::navmesh::polygons(mesh) {
            if corners.len() < 3 {
                continue;
            }
            let mut normal = glam::Vec3::ZERO;
            for (i, a) in corners.iter().enumerate() {
                normal += a.cross(corners[(i + 1) % corners.len()]);
            }
            // ⚠ Walkable faces up whatever the winding, so the studio shade lights the top.
            let normal = if normal.z < 0.0 { -normal } else { normal }.normalize_or(glam::Vec3::Z);
            let base = vertices.len() as u32;
            vertices.extend(corners.iter().map(|c| {
                MeshVertex::new_coloured(c.to_array(), normal.to_array(), [0.0, 0.0], FILL)
            }));
            for k in 1..corners.len() as u32 - 1 {
                indices.extend([base, base + k, base + k + 1]);
            }
        }
        vec![MeshUpload {
            bump: false,
            vertices,
            indices,
            texture_name: None,
            skinning: None,
            palette_bones: Vec::new(),
            source: None,
            channels: Vec::new(),
        }]
    }
}

fn build_meshes_for_preview_scene(state: &AppState) -> Vec<MeshUpload> {
    let scene = &state.preview_scene;
    let mut out = Vec::new();

    for source in &scene.mesh_sources {
        let Some(doc) = state.document(source.document) else {
            continue;
        };
        extend_from_mesh_source(&mut out, state, doc, source.entry);
    }

    if out.is_empty() && scene.draw_skeleton_model {
        if let Some(skeleton_source) = scene.skeleton_source {
            if let Some(doc) = state.document(skeleton_source.document()) {
                let entry = match skeleton_source {
                    crate::state::SkeletonSource::DictionaryEntry(_, index) => Some(index),
                    _ => None,
                };
                extend_from_mesh_source(&mut out, state, doc, entry);
            }
        }
    }

    out
}

fn extend_from_mesh_source(
    out: &mut Vec<MeshUpload>,
    state: &AppState,
    doc: &crate::state::Document,
    entry: Option<usize>,
) {
    let request = GeometryRequest {
        state,
        doc,
        selection: None,
        entry,
    };
    if let Some(build) = capabilities::for_payload(&doc.payload).and_then(|row| row.scene_meshes) {
        let source = Some(MeshSource {
            document: doc.id,
            entry,
        });
        out.extend(
            build(&request)
                .into_iter()
                .map(|upload| MeshUpload { source, ..upload }),
        );
    }
}

pub(super) const CLOTH_SURFACE: [f32; 4] = [0.35, 0.72, 0.78, 1.0];

fn build_cloth_meshes(cloth: &or_io::gta5::CharacterCloth) -> Vec<MeshUpload> {
    let Some(controller) = cloth.cloth_controller.as_ref() else {
        return Vec::new();
    };
    let Some(verlet) = crate::ui::panels::hierarchy::primary_cloth(controller) else {
        return Vec::new();
    };
    let positions = &verlet.cloth_data.vertex_positions;
    if positions.is_empty() || controller.tri_indices.is_empty() {
        return Vec::new();
    }

    let pos: Vec<[f32; 3]> = positions.iter().map(|v| [v.x, v.y, v.z]).collect();
    let mut normals = Vec::new();
    face_normals(&pos, &controller.tri_indices, &mut normals);

    let vertices: Vec<MeshVertex> = pos
        .iter()
        .zip(normals.iter())
        .map(|(p, n)| MeshVertex::new_coloured(*p, *n, [0.0, 0.0], CLOTH_SURFACE))
        .collect();
    let indices: Vec<u32> = controller.tri_indices.iter().map(|&i| i as u32).collect();

    vec![MeshUpload {
        bump: false,
        channels: Vec::new(),
        vertices,
        indices,
        texture_name: None,
        skinning: None,
        palette_bones: Vec::new(),
        source: None,
    }]
}

fn build_ypt_meshes(
    payload: &DocPayload,
    selection: Option<u64>,
    lod: usize,
    selected_channel: Option<DebugChannel>,
) -> Vec<MeshUpload> {
    let Some(sel) = selection else {
        return Vec::new();
    };
    if ypt_tag_of(sel) != YPT_MODEL {
        return Vec::new();
    }
    match payload {
        DocPayload::Resource(or_io::Asset::Particles(list)) => {
            ypt_drawable_for_selection(list, sel).map_or_else(Vec::new, |mesh| {
                drawable_meshes(mesh, lod, selected_channel)
            })
        }
        DocPayload::Resource(or_io::Asset::ParticlesGen9(list)) => {
            ypt_drawable_for_selection(list, sel).map_or_else(Vec::new, |mesh| {
                drawable_meshes(mesh, lod, selected_channel)
            })
        }
        _ => Vec::new(),
    }
}

pub fn build_meshes_from_ypt_drawable(
    asset: &or_io::gta5::PtxFxList,
    drawable_hash: u32,
) -> Vec<MeshUpload> {
    or_particle::list::model(asset, drawable_hash)
        .map_or_else(Vec::new, |mesh| drawable_meshes(mesh, 0, None))
}

/// What an upload names of a material, per target.
pub(crate) trait Material: DrawableTarget + Sized {
    /// The first texture the material binds, lowercase, by the name its group binds it by.
    fn diffuse_texture_name(
        shader: &Self::InstanceData,
        group: &GrmShaderGroup<Self>,
    ) -> Option<String>;

    /// Every texture the material binds, by the name its group binds it by; a slot that binds none is left out.
    fn bound_names<'a>(
        shader: &'a Self::InstanceData,
        group: &'a GrmShaderGroup<Self>,
    ) -> Vec<&'a str>;

    /// The hash of the effect the material draws with.
    fn shader_name_hash(shader: &Self::InstanceData) -> u32;
}

impl Material for Gen8 {
    fn diffuse_texture_name(
        shader: &gen8::GrcInstanceData,
        group: &GrmShaderGroup<Gen8>,
    ) -> Option<String> {
        first_named(shader.textures().filter_map(|slot| group.slot_name(slot)))
    }

    fn bound_names<'a>(
        shader: &'a gen8::GrcInstanceData,
        group: &'a GrmShaderGroup<Gen8>,
    ) -> Vec<&'a str> {
        (shader.textures().filter_map(|slot| group.slot_name(slot))).collect()
    }

    fn shader_name_hash(shader: &gen8::GrcInstanceData) -> u32 {
        shader.basis_hash.get()
    }
}

impl Material for Gen9 {
    fn diffuse_texture_name(
        shader: &gen9::GrcInstanceDataG9,
        group: &GrmShaderGroup<Gen9>,
    ) -> Option<String> {
        first_named(shader.textures().filter_map(|slot| group.slot_name(slot)))
    }

    fn bound_names<'a>(
        shader: &'a gen9::GrcInstanceDataG9,
        group: &'a GrmShaderGroup<Gen9>,
    ) -> Vec<&'a str> {
        (shader.textures().filter_map(|slot| group.slot_name(slot))).collect()
    }

    fn shader_name_hash(shader: &gen9::GrcInstanceDataG9) -> u32 {
        shader.name.get()
    }
}

fn first_named<'a>(mut names: impl Iterator<Item = &'a str>) -> Option<String> {
    names.find_map(|name| (!name.is_empty()).then(|| name.to_ascii_lowercase()))
}

/// The meshes a drawable draws at level `lod`, or at its first where it has no models at `lod`.
fn drawable_meshes<T: Material>(
    drawable: &RmcDrawable<T>,
    lod: usize,
    selected_channel: Option<DebugChannel>,
) -> Vec<MeshUpload> {
    drawable_parts(drawable, level_drawn(drawable, lod), selected_channel)
        .into_iter()
        .map(|(_, upload)| upload)
        .collect()
}

/// The meshes a fragment draws at level `lod`: its body's, then its wheels', so a body geometry keeps its index.
fn fragment_meshes<T: Material>(
    fragment: &FragType<T>,
    lod: usize,
    channel: Option<DebugChannel>,
) -> Vec<MeshUpload> {
    let Some(body) = fragment.common_drawable.as_deref() else {
        return Vec::new();
    };
    let mut meshes = drawable_meshes(&body.base, lod, channel);
    meshes.extend(wheel_meshes(fragment, &body.base, lod, channel));
    meshes
}

/// The meshes of a vehicle's wheels at level `lod`, each carried to its wheel bone of `body`'s skeleton.
fn wheel_meshes<T: Material>(
    fragment: &FragType<T>,
    body: &RmcDrawable<T>,
    lod: usize,
    channel: Option<DebugChannel>,
) -> Vec<MeshUpload> {
    let mut out = Vec::new();
    for (wheel, at) in or_renderer::game_shaders::drawable::wheel_placements(fragment) {
        let wheel = &wheel.base;
        let level = level_drawn(wheel, lod);
        // A child drawable ships no shader group, it indexes the vehicle's
        let group = wheel
            .shader_group
            .as_deref()
            .or(body.shader_group.as_deref());
        for (_, mut mesh) in drawable_parts_in(wheel, group, level, channel) {
            for vertex in &mut mesh.vertices {
                vertex.position = at.transform_point3(vertex.position.into()).into();
                vertex.normal = at.transform_vector3(vertex.normal.into()).into();
            }
            // Placed by `at`, no pose of the vehicle's skeleton moves it again
            mesh.skinning = None;
            mesh.palette_bones.clear();
            out.push(mesh);
        }
    }
    out
}

/// The level of detail the viewport draws of a drawable for the level chosen.
pub(crate) fn level_drawn<T: DrawableTarget>(drawable: &RmcDrawable<T>, chosen: usize) -> usize {
    let has_models = |level: &usize| {
        (drawable.lod_group.lods.get(*level)).is_some_and(|lod| !lod.models.is_empty())
    };
    Some(chosen).filter(has_models).unwrap_or(0)
}

/// The mesh each geometry of level `lod` draws, by its model's and its own index.
pub(crate) fn drawable_parts<T: Material>(
    drawable: &RmcDrawable<T>,
    lod: usize,
    selected_channel: Option<DebugChannel>,
) -> Vec<((usize, usize), MeshUpload)> {
    drawable_parts_in(
        drawable,
        drawable.shader_group.as_deref(),
        lod,
        selected_channel,
    )
}

/// As [`drawable_parts`], its shaders indexing `group`, which a fragment's child drawable (it ships none) takes from the fragment.
fn drawable_parts_in<T: Material>(
    drawable: &RmcDrawable<T>,
    group: Option<&GrmShaderGroup<T>>,
    lod: usize,
    selected_channel: Option<DebugChannel>,
) -> Vec<((usize, usize), MeshUpload)> {
    let authoring = drawable
        .skeleton
        .as_deref()
        .filter(|s| !s.bones.is_empty())
        .map(or_anim::Skeleton::build);
    let mut out = Vec::new();
    let Some(level) = drawable.lod_group.lods.get(lod) else {
        return out;
    };
    for (model_idx, model) in level.models.iter().enumerate() {
        for (geom_idx, geom) in model.geometries.iter().enumerate() {
            let shader = model
                .shader_indices
                .get(geom_idx)
                .and_then(|&i| group?.shaders.get(usize::from(i)));
            let texture_name = shader.and_then(|s| T::diffuse_texture_name(s, group?));
            let upload = geometry_upload(
                &*geom.vertex_buffer,
                T::indices(&geom.index_buffer),
                !geom.matrix_palette.is_empty(),
                &geom.matrix_palette,
                texture_name,
                selected_channel,
                authoring.as_ref(),
            );
            out.extend(upload.map(|upload| ((model_idx, geom_idx), upload)));
        }
    }
    out
}

/// One geometry's upload from any vertex buffer: its positions, normals and first uvs, and its skinning where `skinned`.
#[allow(clippy::too_many_arguments)]
fn geometry_upload<V: VertexSource + ?Sized>(
    vb: &V,
    ib_indices: &[u16],
    skinned: bool,
    matrix_palette: &[u16],
    texture_name: Option<String>,
    selected_channel: Option<DebugChannel>,
    authoring: Option<&or_anim::Skeleton>,
) -> Option<MeshUpload> {
    let reader = VertexReader::new(vb)?;
    let layout = *reader.layout();
    if vb.vertex_stride() == 0 || reader.stride() == 0 {
        return None;
    }
    if reader.declared_count() == 0 || ib_indices.is_empty() {
        return None;
    }

    let size = |c: GrcFvfChannel| layout.slot(c).map(|s| s.size);
    if size(GrcFvfChannel::Position)? != GrcDataSize::Float3 {
        return None;
    }
    let normal_size = size(GrcFvfChannel::Normal);
    let tex0_size = size(GrcFvfChannel::Tex0);

    let vertex_count = reader.count();

    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(vertex_count);
    let mut uvs = Vec::with_capacity(vertex_count);

    for i in 0..vertex_count {
        let Some(pos) = reader.get(GrcFvfChannel::Position, i) else {
            break;
        };
        positions.push([pos[0], pos[1], pos[2]]);

        let normal = reader.get(GrcFvfChannel::Normal, i);
        normals.push(match (normal_size, normal) {
            (Some(GrcDataSize::Float3), Some(n)) => [n[0], n[1], n[2]],
            (Some(GrcDataSize::PackedNormal), Some(n)) => {
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len > 1e-6 {
                    [n[0] / len, n[1] / len, n[2] / len]
                } else {
                    [0.0, 1.0, 0.0]
                }
            }
            _ => [0.0, 0.0, 0.0],
        });

        let uv = reader.get(GrcFvfChannel::Tex0, i);
        uvs.push(match (tex0_size, uv) {
            (Some(GrcDataSize::Float2 | GrcDataSize::Half2), Some(t)) => [t[0], t[1]],
            _ => [0.0, 0.0],
        });
    }

    if normal_size.is_none() {
        face_normals(&positions, ib_indices, &mut normals);
    }

    let mut indices = Vec::with_capacity(ib_indices.len());
    for &i in ib_indices {
        let i = i as u32;
        if (i as usize) < positions.len() {
            indices.push(i);
        }
    }
    if indices.len() < 3 {
        return None;
    }
    let triangle_count = indices.len() / 3;
    indices.truncate(triangle_count * 3);

    let channels = offered_by([&layout]);

    // ⚠ Only the selected channel is read, and only when one is selected.
    // Packing every channel a buffer carries would put 16 to 48 bytes on
    // every vertex for a debug view.
    let debug = selected_channel.filter(|c| channels.contains(c));
    let channel_colours = debug.map(|c| {
        (0..positions.len())
            .map(|i| {
                reader
                    .get(c.channel(), i)
                    .map(|v| c.visualise(v))
                    .unwrap_or([0.0, 0.0, 0.0, 1.0])
            })
            .collect::<Vec<_>>()
    });

    let mut vertices = Vec::with_capacity(positions.len());
    for (i, pos) in positions.iter().enumerate() {
        let position = *pos;
        let normal = normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
        let uv = uvs.get(i).copied().unwrap_or([0.0, 0.0]);
        vertices.push(match channel_colours.as_ref().and_then(|c| c.get(i)) {
            Some(colour) => MeshVertex::new_coloured(position, normal, uv, *colour),
            None => MeshVertex::new(position, normal, uv),
        });
    }

    Some(MeshUpload {
        bump: false,
        vertices,
        indices,
        texture_name,
        skinning: skinned
            .then(|| or_anim::decode_skinning(vb, matrix_palette))
            .flatten(),
        palette_bones: authoring
            .map(|s| or_anim::palette_bones(matrix_palette, s))
            .unwrap_or_default(),
        channels,
        source: None,
    })
}

pub(super) fn face_normals(positions: &[[f32; 3]], indices: &[u16], normals: &mut Vec<[f32; 3]>) {
    normals.clear();
    normals.resize(positions.len(), [0.0, 0.0, 0.0]);
    for tri in indices.as_chunks::<3>().0 {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= positions.len() || i1 >= positions.len() || i2 >= positions.len() {
            continue;
        }
        let a = positions[i0];
        let b = positions[i1];
        let c = positions[i2];
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        for &i in &[i0, i1, i2] {
            normals[i][0] += n[0];
            normals[i][1] += n[1];
            normals[i][2] += n[2];
        }
    }
    for n in normals.iter_mut() {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-6 {
            n[0] /= len;
            n[1] /= len;
            n[2] /= len;
        } else {
            *n = [0.0, 1.0, 0.0];
        }
    }
}
// ⚠ Declared here, not inside `as_recorded`: there `#[path]` resolves from
// src/render/meshes/as_recorded/, which does not exist, and Linux refuses the path.
#[cfg(test)]
#[path = "../../crates/domain/or-interchange/tests/baseline/mod.rs"]
mod baseline;

/// Every Gen8 corpus drawable uploads the geometry, and names the textures, it did when the baselines were recorded.
#[cfg(test)]
mod as_recorded {
    use super::baseline;

    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use baseline::{Key, Now};
    use or_io::decl::Gen8;
    use or_io::gta5::{File, GtaDrawable, GtaFragType, PgDictionary, RmcDrawable};
    use or_test_support::{shared_corpus_root, Corpus, TARGET_GEN8, TARGET_GEN9};

    use super::MeshUpload;

    const GEOMETRY: &str = "mesh";
    const TEXTURES: &str = "mesh_texture";
    const EXTENSIONS: [&str; 3] = ["ydr", "ydd", "yft"];

    /// Each upload's vertices and indices, in order.
    fn geometry(uploads: &[MeshUpload]) -> Vec<u8> {
        let mut out = Vec::new();
        for upload in uploads {
            out.extend((upload.vertices.len() as u64).to_le_bytes());
            out.extend_from_slice(bytemuck::cast_slice(&upload.vertices));
            out.extend((upload.indices.len() as u64).to_le_bytes());
            out.extend_from_slice(bytemuck::cast_slice(&upload.indices));
        }
        out
    }

    /// Each upload's texture name, in order; a NUL where it names none.
    fn textures(uploads: &[MeshUpload]) -> Vec<u8> {
        let mut out = Vec::new();
        for upload in uploads {
            out.extend(upload.texture_name.as_deref().unwrap_or("\0").as_bytes());
            out.push(0);
        }
        out
    }

    /// A drawable's uploads, with the sub-key a dictionary entry is recorded under.
    type Drawn = (Option<String>, Vec<MeshUpload>);

    /// Each drawable a file of `target` holds, with the sub-key a dictionary entry is recorded under.
    fn uploads_of(path: &Path, target: &str) -> Result<Vec<Drawn>, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if target == TARGET_GEN9 {
            drawn::<or_io::decl::Gen9>(ext, &bytes)
        } else {
            drawn::<Gen8>(ext, &bytes)
        }
    }

    fn drawn<T: super::Material>(ext: &str, bytes: &[u8]) -> Result<Vec<Drawn>, String>
    where
        GtaDrawable<T>: File,
        GtaFragType<T>: File,
        PgDictionary<GtaDrawable<T>>: File,
    {
        let uploads = |d: &RmcDrawable<T>| super::drawable_meshes(d, 0, None);
        Ok(match ext {
            "ydr" => {
                let drawable = GtaDrawable::<T>::from_bytes(bytes).map_err(|e| e.to_string())?;
                vec![(None, uploads(&drawable.base))]
            }
            "yft" => {
                let fragment = GtaFragType::<T>::from_bytes(bytes).map_err(|e| e.to_string())?;
                let common = fragment.common_drawable.as_deref();
                vec![(None, common.map(|d| uploads(&d.base)).unwrap_or_default())]
            }
            _ => {
                let dict =
                    PgDictionary::<GtaDrawable<T>>::from_bytes(bytes).map_err(|e| e.to_string())?;
                (dict.codes.iter().zip(&dict.entries))
                    .map(|(code, entry)| {
                        let meshes = entry.as_deref().map(|d| uploads(&d.base));
                        (Some(format!("{code:08x}")), meshes.unwrap_or_default())
                    })
                    .collect()
            }
        })
    }

    /// Every corpus drawable file, with its target.
    fn corpus(what: &str) -> Option<(PathBuf, Vec<(PathBuf, &'static str)>)> {
        let root = shared_corpus_root(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for target in [TARGET_GEN8, TARGET_GEN9] {
            for ext in EXTENSIONS {
                let found = Corpus::with_root(&root, ext).target(target).find();
                files.extend(found.into_iter().map(|path| (path, target)));
            }
        }
        if files.is_empty() {
            or_test_support::skip(what, &root, "ydr, .ydd or .yft");
            return None;
        }
        Some((root, files))
    }

    #[test]
    fn every_corpus_drawable_uploads_as_recorded() {
        let Some((root, files)) = corpus("every_corpus_drawable_uploads_as_recorded") else {
            return;
        };
        let mut shapes: BTreeMap<Key, Now> = BTreeMap::new();
        let mut names: BTreeMap<Key, Now> = BTreeMap::new();
        for (path, target) in &files {
            match uploads_of(path, target) {
                Ok(drawables) => {
                    for (sub, uploads) in drawables.into_iter().filter(|(_, u)| !u.is_empty()) {
                        let key = baseline::key(&root, path, sub.as_deref(), target);
                        let digest = |bytes: Vec<u8>| Now::Digest(baseline::digest(&bytes));
                        shapes.insert(key.clone(), digest(geometry(&uploads)));
                        names.insert(key, digest(textures(&uploads)));
                    }
                }
                Err(why) => {
                    let key = baseline::key(&root, path, None, target);
                    shapes.insert(key.clone(), Now::Refused(why.clone()));
                    names.insert(key, Now::Refused(why));
                }
            }
        }
        baseline::settle(env!("CARGO_MANIFEST_DIR"), GEOMETRY, &shapes);
        baseline::settle(env!("CARGO_MANIFEST_DIR"), TEXTURES, &names);
    }

    #[test]
    fn a_perturbed_upload_names_its_file() {
        let Some((root, files)) = corpus("a_perturbed_upload_names_its_file") else {
            return;
        };
        let (path, target, bytes) = files
            .iter()
            .find_map(|(path, target)| {
                let (_, uploads) = uploads_of(path, target).ok()?.into_iter().next()?;
                (!uploads.is_empty()).then(|| (path, *target, geometry(&uploads)))
            })
            .expect("some corpus drawable uploads geometry");
        baseline::a_perturbed_stream_is_named(baseline::key(&root, path, None, target), &bytes, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_normals_unit_triangle() {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        let indices = vec![0u16, 1, 2];
        let mut normals = Vec::new();
        face_normals(&positions, &indices, &mut normals);
        assert_eq!(normals.len(), 3);
        for n in &normals {
            assert!((n[0]).abs() < 1e-5);
            assert!((n[1] + 1.0).abs() < 1e-5, "expected -Y, got {:?}", n);
            assert!((n[2]).abs() < 1e-5);
        }
    }

    struct Soa {
        data: Vec<u8>,
        layout: or_io::codec::mesh::VertexLayout,
    }

    impl VertexSource for Soa {
        fn vertex_data(&self) -> &[u8] {
            &self.data
        }
        fn vertex_layout(&self) -> Option<or_io::codec::mesh::VertexLayout> {
            Some(self.layout)
        }
        fn vertex_stride(&self) -> usize {
            24
        }
        fn vertex_declared(&self) -> usize {
            2
        }
    }

    /// ⚠ An Enhanced buffer keeps each channel in its own run, so the normals start after every position.
    #[test]
    fn an_enhanced_structure_of_arrays_buffer_uploads_each_channel_from_its_own_run() {
        use or_io::codec::mesh::{elements_g9, VertexElementFormatG9, VertexLayout};
        let (mut offsets, mut sizes, mut types) = ([0u32; 52], [0u8; 52], [0u8; 52]);
        types[0] = VertexElementFormatG9::R32G32B32Float.to_raw();
        sizes[0] = 12;
        types[4] = VertexElementFormatG9::R32G32B32Float.to_raw();
        offsets[4] = 24;
        sizes[4] = 12;
        let floats = [
            1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0,
        ];
        let source = Soa {
            data: floats.iter().flat_map(|v| v.to_le_bytes()).collect(),
            layout: VertexLayout::of_g9(&elements_g9(&offsets, &sizes, &types)),
        };
        let upload = geometry_upload(&source, &[0, 1, 1], false, &[], None, None, None)
            .expect("the buffer uploads");
        let positions: Vec<[f32; 3]> = upload.vertices.iter().map(|v| v.position).collect();
        let normals: Vec<[f32; 3]> = upload.vertices.iter().map(|v| v.normal).collect();
        assert_eq!(positions, [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        assert_eq!(normals, [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]);
    }
}

#[cfg(test)]
mod wheels {
    use super::*;
    use glam::Vec3;
    use or_io::gta5::{File, GtaFragType};
    use or_renderer::game_shaders::drawable::wheel_placements;
    use or_test_support::{shared_corpus_root, Corpus, TARGET_GEN8};

    /// The mean of a mesh's vertices.
    fn centroid(mesh: &MeshUpload) -> Vec3 {
        let sum: Vec3 = mesh.vertices.iter().map(|v| Vec3::from(v.position)).sum();
        sum / mesh.vertices.len() as f32
    }

    #[test]
    fn a_corpus_vehicle_is_drawn_with_its_wheels_at_its_wheel_bones() {
        let test = "a_corpus_vehicle_is_drawn_with_its_wheels_at_its_wheel_bones";
        let root = shared_corpus_root(env!("CARGO_MANIFEST_DIR"));
        let files = Corpus::with_root(&root, "yft").target(TARGET_GEN8).find();
        if files.is_empty() {
            or_test_support::skip(test, &root, "yft");
            return;
        }
        let mut dressed = 0;
        for path in &files {
            let bytes = std::fs::read(path).expect("a corpus file reads");
            let fragment =
                GtaFragType::<Gen8>::from_bytes(&bytes).expect("a corpus fragment opens");
            let Some(body) = fragment.common_drawable.as_deref() else {
                continue;
            };
            let placed = wheel_placements(&fragment);
            if placed.is_empty() {
                continue;
            }
            let name = path.display().to_string();
            let wheels = wheel_meshes(&fragment, &body.base, 0, None);
            assert!(!wheels.is_empty(), "{name}: wheels to draw");
            // A wheel's origin is its hub: carried to its bone, each part of it stays within a tyre's width of that bone
            let hubs: Vec<Vec3> = (placed.iter())
                .map(|(_, at)| at.transform_point3(Vec3::ZERO))
                .collect();
            for mesh in &wheels {
                let near = (hubs.iter())
                    .map(|hub| hub.distance(centroid(mesh)))
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    near < 0.3,
                    "{name}: a wheel part {near} m from any wheel bone"
                );
                assert!(mesh.skinning.is_none(), "{name}");
            }
            // The body first, then the wheels, so a body geometry keeps the index it had
            let body_only = drawable_meshes(&body.base, 0, None);
            let row = fragment_meshes(&fragment, 0, None);
            assert_eq!(row.len(), body_only.len() + wheels.len(), "{name}");
            for (own, drawn) in body_only.iter().zip(&row) {
                assert_eq!(own.vertices.len(), drawn.vertices.len(), "{name}");
            }
            dressed += 1;
        }
        assert!(dressed > 0, "a corpus vehicle with wheels");
    }
}
