use std::collections::HashMap;

use glam::Vec3;
use or_renderer::LineVertex;

use super::document::{DocPayload, Document, DocumentId, DocumentKind};
use super::format_handlers::Tier;
use super::models::ModelListing;
use super::textures::{TextureListing, TextureResolution, TextureSource};
use super::AppState;
use crate::render::meshes::MeshUpload;

pub type SelectionSummaryFn = fn(&DocPayload, u64) -> String;

pub struct GeometryRequest<'a> {
    pub state: &'a AppState,
    pub doc: &'a Document,

    pub selection: Option<u64>,

    pub entry: Option<usize>,
}

pub type InspectorFn = fn(&mut egui::Ui, &mut AppState, DocumentId, Option<u64>);

pub type OutlineFn = fn(&mut egui::Ui, &mut AppState, DocumentId);

pub type HasOutlineChildrenFn = fn(&DocPayload) -> bool;

pub type MeshesFn = fn(&GeometryRequest<'_>) -> Vec<MeshUpload>;
pub type LinesFn = fn(&GeometryRequest<'_>) -> Vec<LineVertex>;
pub type BoundsFn = fn(&GeometryRequest<'_>) -> Option<(Vec3, f32)>;

pub type TextureListFn = fn(&DocPayload, Option<u64>) -> TextureListing;

pub type ModelListFn = fn(&DocPayload) -> ModelListing;

pub type TextureForSelectionFn = fn(&DocPayload, u64) -> TextureResolution;

pub type TextureAtFn =
    for<'p> fn(&'p DocPayload, TextureSource, &str) -> Option<super::textures::Texture<'p>>;

pub type TextureNamedFn =
    for<'p> fn(&'p DocPayload, Option<u64>, &str) -> Option<super::textures::Texture<'p>>;

pub type MeshSourcesFn =
    fn(DocumentId, &DocPayload, &str) -> Vec<super::preview_scene::MeshCandidate>;

pub type SkeletonSourcesFn = fn(DocumentId, &DocPayload) -> Vec<super::SkeletonSource>;

pub type EditedSectionsFn =
    fn(&super::Document) -> Option<Result<super::gta6_resource_save::EditedSave, String>>;

pub type SkeletonDataFn =
    for<'p> fn(&'p DocPayload, super::SkeletonSource) -> Option<&'p or_io::gta5::CrSkeletonData>;

#[derive(Debug, Clone, Copy)]
pub struct FormatCapabilities {
    pub kind: DocumentKind,

    pub selection_summary: Option<SelectionSummaryFn>,

    pub outline: Option<OutlineFn>,

    pub has_outline_children: Option<HasOutlineChildrenFn>,

    pub inspector: Option<InspectorFn>,

    pub meshes: Option<MeshesFn>,

    pub scene_meshes: Option<MeshesFn>,

    pub lines: Option<LinesFn>,

    pub bounds: Option<BoundsFn>,

    pub textures: Option<TextureListFn>,

    /// Which models this format offers the gallery's Models view.
    ///
    /// ⚠ Only a row with [`Self::meshes`] may list models: the listing's ids
    /// are the geometry row's argument, so a row that cannot draw lists a grid
    /// of empty tiles.
    pub models: Option<ModelListFn>,

    pub texture_for_selection: Option<TextureForSelectionFn>,

    pub texture_at: Option<TextureAtFn>,

    pub texture_named: Option<TextureNamedFn>,

    pub mesh_sources: Option<MeshSourcesFn>,

    pub skeleton_sources: Option<SkeletonSourcesFn>,

    pub skeleton_data: Option<SkeletonDataFn>,

    /// The sections a GTA VI resource document holds edited, which a save writes as a loose
    /// resource; `None` from it while nothing is edited.
    pub edited_sections: Option<EditedSectionsFn>,
}

impl FormatCapabilities {
    pub const fn none(kind: DocumentKind) -> Self {
        Self {
            kind,
            selection_summary: None,
            outline: None,
            has_outline_children: None,
            inspector: None,
            meshes: None,
            scene_meshes: None,
            lines: None,
            bounds: None,
            textures: None,
            models: None,
            texture_for_selection: None,
            texture_at: None,
            texture_named: None,
            mesh_sources: None,
            skeleton_sources: None,
            skeleton_data: None,
            edited_sections: None,
        }
    }
}

static BUILTIN: &[FormatCapabilities] = &[
    FormatCapabilities {
        kind: DocumentKind::Ypt,
        selection_summary: Some(ypt_summary),
        outline: Some(outline::ypt),
        has_outline_children: Some(children::ypt),
        inspector: Some(inspector::ypt),
        meshes: Some(meshes::ypt),
        scene_meshes: None,
        lines: Some(lines::ypt),
        bounds: Some(bounds::ypt),
        textures: Some(list::ypt),
        models: Some(model_list::ypt),
        texture_for_selection: Some(selected::ypt),
        texture_at: Some(at::ypt),
        texture_named: Some(named::ypt),
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        // `None` for a GTA V list: only a GTA VI list a scan opened is written in place, and an
        // edit with no GTA VI field to land in refuses the save by name.
        edited_sections: Some(super::gta6_particles_save::edited_save),
    },
    FormatCapabilities {
        kind: DocumentKind::Ytd,
        selection_summary: Some(ytd_summary),
        outline: Some(outline::ytd),
        has_outline_children: Some(children::ytd),
        inspector: Some(inspector::ytd),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: Some(list::ytd),
        models: None,
        texture_for_selection: Some(selected::ytd),
        texture_at: Some(at::ytd),
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        // A scanned GTA VI dictionary's replaced textures, written over its body.
        edited_sections: Some(super::gta6_edit::edited_save),
    },
    FormatCapabilities {
        kind: DocumentKind::Ydr,
        selection_summary: Some(ydr_summary),
        outline: Some(outline::ydr),
        has_outline_children: Some(children::always),
        inspector: Some(inspector::ydr),
        meshes: Some(meshes::ydr),
        scene_meshes: Some(meshes::ydr_scene),
        lines: Some(lines::ydr),
        bounds: Some(bounds::ydr),
        textures: Some(list::ydr),
        models: Some(model_list::ydr),
        texture_for_selection: Some(selected::ydr),
        texture_at: Some(at::ydr),
        texture_named: Some(named::ydr),
        mesh_sources: Some(mesh_sources::whole_document),
        skeleton_sources: Some(skeletons::ydr_sources),
        skeleton_data: Some(skeletons::ydr_data),
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Ydd,
        selection_summary: Some(ydd_summary),
        outline: Some(outline::ydd),
        has_outline_children: Some(children::ydd),
        inspector: Some(inspector::ydd),
        meshes: Some(meshes::ydd),
        scene_meshes: Some(meshes::ydd_scene),
        lines: Some(lines::ydd),
        bounds: Some(bounds::ydd),
        textures: Some(list::ydd),
        models: Some(model_list::ydd),
        texture_for_selection: Some(selected::ydd),
        texture_at: Some(at::ydd),
        texture_named: Some(named::ydd),
        mesh_sources: Some(mesh_sources::ydd),
        skeleton_sources: Some(skeletons::ydd_sources),
        skeleton_data: Some(skeletons::ydd_data),
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Yft,
        selection_summary: Some(yft_summary),
        outline: Some(outline::yft),
        has_outline_children: Some(children::always),
        inspector: Some(inspector::yft),
        meshes: Some(meshes::yft),
        scene_meshes: Some(meshes::yft_scene),
        lines: Some(lines::yft),
        bounds: Some(bounds::yft),
        textures: Some(list::yft),
        models: Some(model_list::yft),
        texture_for_selection: Some(selected::yft),
        texture_at: Some(at::yft),
        texture_named: Some(named::yft),
        mesh_sources: Some(mesh_sources::whole_document),
        skeleton_sources: Some(skeletons::yft_sources),
        skeleton_data: Some(skeletons::yft_data),
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Ybn,
        selection_summary: Some(ybn_summary),
        outline: Some(outline::ybn),
        has_outline_children: Some(children::always),
        inspector: Some(inspector::ybn),
        meshes: Some(meshes::ybn),
        scene_meshes: Some(meshes::ybn_scene),
        lines: Some(lines::ybn),
        bounds: Some(bounds::ybn),
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Yld,
        selection_summary: Some(yld_summary),
        outline: Some(outline::yld),
        has_outline_children: Some(children::yld),
        inspector: Some(inspector::yld),
        meshes: Some(meshes::yld),
        scene_meshes: Some(meshes::yld_scene),
        lines: Some(lines::yld),
        bounds: Some(bounds::yld),
        textures: None,
        models: Some(model_list::yld),
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Ycd,
        selection_summary: Some(ycd_summary),
        outline: Some(outline::ycd),
        has_outline_children: Some(children::ycd),
        inspector: Some(inspector::ycd),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Yed,
        selection_summary: Some(yed_summary),
        outline: Some(outline::yed),
        has_outline_children: Some(children::yed),
        // No bespoke inspector for `.yed` or `.yfd`: the declared layout describes both
        // pgDictionary roots field for field, and a hand-written panel would
        // restate the or_editor::Summary grid and drift from it.
        inspector: None,
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Yfd,
        selection_summary: Some(yfd_summary),
        outline: Some(outline::yfd),
        has_outline_children: Some(children::yfd),
        inspector: None,
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    // The inspector for both is the described root grid.
    FormatCapabilities {
        kind: DocumentKind::Ynd,
        selection_summary: Some(ynd_summary),
        outline: Some(outline::ynd),
        has_outline_children: Some(children::always),
        ..FormatCapabilities::none(DocumentKind::Ynd)
    },
    FormatCapabilities {
        kind: DocumentKind::Ynv,
        inspector: Some(inspector::ynv),
        selection_summary: Some(ynv_summary),
        outline: Some(outline::ynv),
        has_outline_children: Some(children::always),
        meshes: Some(meshes::ynv),
        lines: Some(lines::ynv),
        bounds: Some(bounds::ynv),
        // A scanned GTA VI cell's polygon flags, written over its body.
        edited_sections: Some(super::gta6_physics_edit::edited_cell),
        ..FormatCapabilities::none(DocumentKind::Ynv)
    },
    FormatCapabilities {
        kind: DocumentKind::Ypdb,
        selection_summary: Some(ypdb_summary),
        outline: Some(outline::ypdb),
        has_outline_children: Some(children::ypdb),
        inspector: Some(inspector::ypdb),
        // ⚠ Unpaged, so there is no resource image and no described root
        // grid; the outline is the whole surface.
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Fxc,
        selection_summary: Some(fxc_summary),
        outline: Some(outline::fxc),
        has_outline_children: Some(children::fxc),
        inspector: Some(inspector::fxc),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Gxt2,
        selection_summary: Some(gxt2_summary),
        outline: None,
        has_outline_children: None,
        inspector: Some(inspector::gxt2),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        inspector: Some(inspector::bik),
        ..FormatCapabilities::none(DocumentKind::Bik)
    },
    FormatCapabilities {
        inspector: Some(inspector::awc),
        // A scanned container's replaced stream is saved as the whole rewritten container, by
        // Save As only: Save into its archive stays refused.
        edited_sections: Some(super::gta6_resource_save::edited_awc),
        ..FormatCapabilities::none(DocumentKind::Awc)
    },
    FormatCapabilities {
        inspector: Some(inspector::height_map),
        // A GTA V `.dat` map has no scanned body, so this is `None` for it; a scanned PAMH map's
        // writer keeps the bytes its grid does not hold.
        edited_sections: Some(super::gta6_other_bodies_save::edited),
        ..FormatCapabilities::none(DocumentKind::HeightMap)
    },
    FormatCapabilities {
        inspector: Some(inspector::distant_lights),
        ..FormatCapabilities::none(DocumentKind::DistantLights)
    },
    FormatCapabilities {
        inspector: Some(inspector::water_map),
        ..FormatCapabilities::none(DocumentKind::WaterMap)
    },
    FormatCapabilities {
        inspector: Some(inspector::world_blanket),
        ..FormatCapabilities::none(DocumentKind::WorldBlanket)
    },
    FormatCapabilities {
        inspector: Some(inspector::nav_precalc),
        ..FormatCapabilities::none(DocumentKind::NavPrecalc)
    },
    FormatCapabilities {
        inspector: Some(inspector::dat_table),
        ..FormatCapabilities::none(DocumentKind::DatTable)
    },
    FormatCapabilities {
        inspector: Some(inspector::rel),
        ..FormatCapabilities::none(DocumentKind::Rel)
    },
    FormatCapabilities {
        inspector: Some(inspector::name_table),
        ..FormatCapabilities::none(DocumentKind::NameTable)
    },
    FormatCapabilities {
        inspector: Some(inspector::gfx),
        ..FormatCapabilities::none(DocumentKind::Gfx)
    },
    FormatCapabilities {
        inspector: Some(inspector::move_network),
        ..FormatCapabilities::none(DocumentKind::MoveNetwork)
    },
    FormatCapabilities {
        inspector: Some(inspector::dds),
        textures: Some(list::dds),
        texture_for_selection: Some(selected::dds),
        texture_at: Some(at::dds),
        ..FormatCapabilities::none(DocumentKind::Dds)
    },
    // The textures are listed and shown; the texture views replace one, and a save writes the pack anew.
    FormatCapabilities {
        outline: Some(outline::ttpf),
        has_outline_children: Some(children::ttpf),
        inspector: Some(inspector::ttpf),
        textures: Some(list::ttpf),
        texture_for_selection: Some(selected::ttpf),
        texture_at: Some(at::ttpf),
        ..FormatCapabilities::none(DocumentKind::Ttpf)
    },
    // Both table listings and explicit scans refuse archive saves.
    FormatCapabilities {
        inspector: Some(inspector::rpf9),
        ..FormatCapabilities::none(DocumentKind::Rpf9)
    },
    // Its clips, or its animations, in the outline and what the animation reader states; its keys and clip windows edit, and a save writes them as a loose resource.
    FormatCapabilities {
        selection_summary: Some(gta6_anim_summary),
        outline: Some(outline::gta6_anim),
        has_outline_children: Some(children::gta6_anim),
        inspector: Some(inspector::gta6_anim),
        edited_sections: Some(|d| held(super::gta6_anim_edit::edited_sections(&d.payload))),
        ..FormatCapabilities::none(DocumentKind::Gta6Anim)
    },
    // A drawable or a bound's triangles are drawn and the reader's facts shown; its materials edit in place, and a save writes them as a loose resource.
    FormatCapabilities {
        selection_summary: Some(gta6_drawable_summary),
        outline: Some(outline::gta6_drawable),
        has_outline_children: Some(children::gta6_drawable),
        inspector: Some(inspector::gta6_drawable),
        meshes: Some(crate::render::gta6_drawable::meshes),
        models: Some(model_list::gta6),
        lines: Some(lines::gta6_drawable),
        bounds: Some(crate::render::gta6_drawable::bounds),
        textures: Some(super::gta6_textures::list),
        texture_at: Some(super::gta6_textures::at),
        texture_named: Some(super::gta6_head::texture_named),
        texture_for_selection: Some(super::gta6_textures::selected),
        edited_sections: super::gta6_resource_save::DRAWABLE,
        ..FormatCapabilities::none(DocumentKind::Gta6Drawable)
    },
    // Its texts edit in the centre, and a save writes them as a loose resource.
    FormatCapabilities {
        inspector: Some(inspector::gta6_text),
        edited_sections: Some(|d| held(super::gta6_text::edited_sections(&d.payload))),
        ..FormatCapabilities::none(DocumentKind::Gta6Text)
    },
    crate::ui::panels::inspector::gta6_text::AUDIO_DATA_CAPABILITIES,
    crate::ui::panels::hierarchy::gta6_expressions::CAPABILITIES,
    crate::ui::panels::hierarchy::gta6_save::CAPABILITIES,
    FormatCapabilities {
        kind: DocumentKind::Rpf,
        selection_summary: Some(rpf_summary),
        outline: Some(outline::rpf),
        has_outline_children: Some(children::rpf),
        inspector: Some(inspector::rpf),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities::none(DocumentKind::GameRoot),
    FormatCapabilities::none(DocumentKind::Folder),
    FormatCapabilities {
        kind: DocumentKind::Text,
        selection_summary: Some(text_summary),
        outline: None,
        has_outline_children: None,
        inspector: Some(inspector::text),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: None,
    },
    FormatCapabilities {
        kind: DocumentKind::Meta,
        selection_summary: Some(meta_summary),
        outline: None,
        has_outline_children: None,
        inspector: Some(inspector::meta),
        meshes: None,
        scene_meshes: None,
        lines: None,
        bounds: None,
        textures: None,
        models: None,
        texture_for_selection: None,
        texture_at: None,
        texture_named: None,
        mesh_sources: None,
        skeleton_sources: None,
        skeleton_data: None,
        edited_sections: Some(|d| held(super::gta6_meta_save::edited_sections(&d.payload))),
    },
    FormatCapabilities {
        inspector: Some(inspector::hex),
        ..FormatCapabilities::none(DocumentKind::Hex)
    },
    FormatCapabilities {
        inspector: Some(crate::ui::panels::compare::facts),
        ..FormatCapabilities::none(DocumentKind::Compare)
    },
];

use super::gta6_resource_save::held;
use super::gta6_summaries::{gta6_anim_summary, gta6_drawable_summary};
use super::models::rows as model_list;
use super::preview_scene::{mesh_sources, skeletons};
use super::textures::rows::{at, list, named, selected};
use crate::render::lines::rows as lines;
use crate::render::meshes::rows as meshes;
use crate::render::viewport::bound_rows as bounds;
use crate::ui::panels::hierarchy::{children, outline};
use crate::ui::panels::inspector::plural;
use crate::ui::panels::inspector::rows as inspector;

// Each summary is reached only for a payload of its own kind, so its trailing
// `_` arm is unreachable through [`for_payload`]. It returns an empty string
// rather than `unreachable!()`: a panic in a per-frame draw is a worse answer
// to a lookup bug than an empty status line.

fn ypt_summary(payload: &DocPayload, _sel: u64) -> String {
    fn effects<T: or_io::gta5::DrawableTarget>(list: &or_io::gta5::PtxFxList<T>) -> String {
        format!("{} effects", or_particle::list::effect_rules(list).count())
    }
    match payload {
        DocPayload::Resource(or_io::Asset::Particles(list)) => effects(list),
        DocPayload::Resource(or_io::Asset::ParticlesGen9(list)) => {
            tr!("{list} (Enhanced)", list = effects(list))
        }
        _ => String::new(),
    }
}

fn ytd_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Resource(or_io::Asset::TextureDict(td)) => {
            plural(td.entries.iter().flatten().count(), "texture")
        }
        DocPayload::Resource(or_io::Asset::TextureDictGen9(td)) => {
            tr!(
                "{count} (Enhanced)",
                count = plural(td.entries.iter().flatten().count(), "texture")
            )
        }
        _ => String::new(),
    }
}

fn ydr_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Resource(or_io::Asset::Drawable(d)) => tr!("Drawable: {name}", name = d.name),
        DocPayload::Resource(or_io::Asset::DrawableGen9(d)) => {
            tr!("Drawable: {name} (Enhanced)", name = d.name)
        }
        _ => String::new(),
    }
}

fn ydd_summary(payload: &DocPayload, sel: u64) -> String {
    use crate::ui::panels::selection_ids::ydd_selected_entry_index;

    fn entry_or_count<T: or_io::gta5::DrawableTarget>(
        dict: &or_io::gta5::PgDictionary<or_io::gta5::GtaDrawable<T>>,
        selected: Option<usize>,
    ) -> String {
        match selected {
            Some(i) => {
                let name = dict.entries[i].as_deref().map_or("", |d| d.name.as_str());
                let label = if name.is_empty() {
                    format!("#{:08X}", dict.codes[i])
                } else {
                    name.to_string()
                };
                tr!(
                    "Drawable: {label} ({at} of {total})",
                    label = label,
                    at = i + 1,
                    total = dict.entries.len()
                )
            }
            None => format!("{} drawables", dict.entries.len()),
        }
    }
    let selected = ydd_selected_entry_index(payload, sel);
    match payload {
        DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => entry_or_count(dict, selected),
        DocPayload::Resource(or_io::Asset::DrawableDictGen9(dict)) => {
            tr!("{dict} (Enhanced)", dict = entry_or_count(dict, selected))
        }
        _ => String::new(),
    }
}

fn yft_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Resource(or_io::Asset::Fragment(frag)) => fragment_summary(frag),
        DocPayload::Resource(or_io::Asset::FragmentGen9(frag)) => {
            tr!("{frag} (Enhanced)", frag = fragment_summary(frag))
        }
        _ => String::new(),
    }
}

fn fragment_summary<T: or_io::gta5::DrawableTarget>(frag: &or_io::gta5::FragType<T>) -> String {
    tr!(
        "Fragment: {name} · {bones} bones · {children} children",
        name = if frag.tune_name.is_empty() {
            tr!("(unnamed)")
        } else {
            frag.tune_name.as_str()
        },
        bones = frag
            .common_drawable
            .as_deref()
            .and_then(|d| d.skeleton.as_deref())
            .map_or(0, |s| s.bones.len()),
        children = frag.child_count(),
    )
}

fn ybn_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Resource(or_io::Asset::Bounds(_)) => tr!("Bound selected").to_string(),
        _ => String::new(),
    }
}

fn yld_summary(payload: &DocPayload, sel: u64) -> String {
    use crate::ui::panels::selection_ids::yld_selected_entry_index;

    let DocPayload::Resource(or_io::Asset::ClothDict(dict)) = payload else {
        return String::new();
    };
    match yld_selected_entry_index(payload, sel) {
        Some(i) => {
            let name = dict.entries[i]
                .as_deref()
                .map_or("", crate::ui::panels::hierarchy::cloth_name);
            let label = if name.is_empty() {
                format!("#{:08X}", dict.codes[i])
            } else {
                name.to_string()
            };
            tr!(
                "Cloth: {label} ({at} of {total})",
                label = label,
                at = i + 1,
                total = dict.entries.len()
            )
        }
        None => tr!("{count} cloth entries", count = dict.entries.len()),
    }
}

fn yed_summary(payload: &DocPayload, sel: u64) -> String {
    use crate::ui::panels::selection_ids::dict_entry_unpack_part;

    let DocPayload::Resource(or_io::Asset::ExpressionDict(dict)) = payload else {
        return String::new();
    };
    let (hash, _, _) = dict_entry_unpack_part(sel);
    let selected = dict
        .codes
        .iter()
        .position(|h| *h == hash)
        .and_then(|i| dict.entries.get(i))
        .and_then(|e| e.as_deref());
    match selected {
        Some(expr) => tr!(
            "Expression: {hierarchy} ({count} tracks, {count_2} programs)",
            hierarchy = crate::ui::panels::hierarchy::short_name(expr),
            count = expr.tracks.len(),
            count_2 = expr.expression_streams.len()
        ),
        None => format!("{} expressions", dict.entries.len()),
    }
}

fn yfd_summary(payload: &DocPayload, sel: u64) -> String {
    use crate::ui::panels::selection_ids::dict_entry_unpack_part;

    let DocPayload::Resource(or_io::Asset::FrameFilterDict(dict)) = payload else {
        return String::new();
    };
    let (hash, _, _) = dict_entry_unpack_part(sel);
    let selected = dict
        .codes
        .iter()
        .position(|h| *h == hash)
        .and_then(|i| dict.entries.get(i))
        .and_then(|e| e.as_deref())
        .and_then(|f| f.as_multi_weight());
    match selected {
        Some(filter) => tr!(
            "Filter: {count} entries over {count_2} weights",
            count = filter.track_id_indices.len(),
            count_2 = filter.weights.len()
        ),
        None => format!("{} filters", dict.entries.len()),
    }
}

fn ynd_summary(payload: &DocPayload, _sel: u64) -> String {
    let DocPayload::Resource(or_io::Asset::PathRegion(region)) = payload else {
        return String::new();
    };
    tr!(
        "{count} nodes, {count_2} links, {count_3} junctions",
        count = region.nodes.len(),
        count_2 = region.links.len(),
        count_3 = region.virtual_junctions.len()
    )
}

fn ynv_summary(payload: &DocPayload, _sel: u64) -> String {
    let DocPayload::Resource(or_io::Asset::NavMesh(mesh)) = payload else {
        return String::new();
    };
    tr!(
        "{polys_array} polygons, {count} special links",
        polys_array = mesh.polys_array.as_deref().map_or(0, |p| p.len()),
        count = mesh.special_links.len()
    )
}

fn ypdb_summary(payload: &DocPayload, _sel: u64) -> String {
    let DocPayload::Ypdb(db) = payload else {
        return String::new();
    };
    tr!(
        "{count} samples · {point_count} cloud points · {count_2} bone tags",
        count = db.samples.len(),
        point_count = db.point_count(),
        count_2 = db.bone_tags.len()
    )
}

fn fxc_summary(payload: &DocPayload, _sel: u64) -> String {
    let DocPayload::Fxc(doc) = payload else {
        return String::new();
    };
    tr!(
        "{count} variables · {count_2} programs · {count_3} techniques",
        count = doc.effect.variables.len(),
        count_2 = doc.effect.programs.len(),
        count_3 = doc.effect.techniques.len()
    )
}

fn ycd_summary(payload: &DocPayload, sel: u64) -> String {
    use crate::ui::panels::selection_ids::ycd_selected_clip_key;

    let DocPayload::Resource(or_io::Asset::Clips(dict)) = payload else {
        return String::new();
    };
    let clips = or_anim::clip::clips(dict).count();
    let selected = ycd_selected_clip_key(payload, sel)
        .and_then(|key| or_anim::clip::clip(dict, key).map(|clip| (key, clip)));
    match selected {
        Some((key, clip)) => {
            let name = &or_anim::clip::header(clip).name.data;
            let label = if name.is_empty() {
                format!("#{key:08X}")
            } else {
                name.clone()
            };
            tr!(
                "Clip: {label} ({seconds}s of {clips} clips)",
                label = label,
                seconds = format!("{:.2}", or_anim::clip::duration(clip)),
                clips = clips
            )
        }
        None => tr!(
            "{clips} clips, {anim_dictionary} animations",
            clips = clips,
            anim_dictionary = dict
                .anim_dictionary
                .as_deref()
                .map_or(0, |a| a.animations.entries().count())
        ),
    }
}

fn gxt2_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Gxt2(t) => tr!("{count} text entries", count = t.len()),
        _ => String::new(),
    }
}

fn rpf_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Rpf(rpf) => tr!(
            "{file_count} files in archive",
            file_count = rpf.session.file_count()
        ),
        _ => String::new(),
    }
}

fn text_summary(payload: &DocPayload, _sel: u64) -> String {
    match payload {
        DocPayload::Text(t) => tr!(
            "{encoding} • {line_ending} • {len_lines} lines",
            encoding = t.encoding.label(),
            line_ending = t.line_ending.label(),
            len_lines = t.text.len_lines()
        ),
        _ => String::new(),
    }
}

fn meta_summary(payload: &DocPayload, _sel: u64) -> String {
    let DocPayload::Meta(m) = payload else {
        return String::new();
    };
    let type_name = m.type_name();
    let label = if type_name.is_empty() {
        tr!("(unbound)")
    } else {
        type_name
    };
    match (&m.parsed, &m.parse_error) {
        (Some(p), _) => tr!(
            "Meta: {label} ({fields} fields)",
            label = label,
            fields = p.instance.fields.len()
        ),
        (None, Some(_)) => tr!("Meta: {label} (parse error)", label = label),
        (None, None) => tr!("Meta: {label}", label = label),
    }
}

#[derive(Default)]
pub struct CapabilityRegistry {
    tiers: HashMap<Tier, HashMap<DocumentKind, FormatCapabilities>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builtin() -> Self {
        let mut registry = Self::new();
        register_builtin(&mut registry);
        registry
    }

    pub fn register_at(&mut self, tier: Tier, capabilities: FormatCapabilities) {
        self.tiers
            .entry(tier)
            .or_default()
            .insert(capabilities.kind, capabilities);
    }

    pub fn register(&mut self, capabilities: FormatCapabilities) {
        self.register_at(Tier::Standard, capabilities);
    }

    pub fn get(&self, kind: DocumentKind) -> Option<FormatCapabilities> {
        for tier in [Tier::Preferred, Tier::Standard, Tier::Fallback] {
            if let Some(found) = self.tiers.get(&tier).and_then(|m| m.get(&kind)) {
                return Some(*found);
            }
        }
        None
    }
}

pub fn register_builtin(registry: &mut CapabilityRegistry) {
    for row in BUILTIN {
        registry.register(*row);
    }
    // Outside `BUILTIN`, which holds one row per named kind: `.ysc` is a
    // registered kind with no declaration for the described grid to draw.
    registry.register(FormatCapabilities {
        inspector: Some(inspector::ysc),
        ..FormatCapabilities::none(DocumentKind::Registered(
            or_io::container::resource::ResourceType::Ysc,
        ))
    });
}

fn live() -> &'static std::sync::RwLock<CapabilityRegistry> {
    static LIVE: std::sync::OnceLock<std::sync::RwLock<CapabilityRegistry>> =
        std::sync::OnceLock::new();
    LIVE.get_or_init(|| std::sync::RwLock::new(CapabilityRegistry::builtin()))
}

// A registration seam: called by nothing in-tree by definition, which is the
// property `adding_a_format` exists to prove. Retires when a plugin host
// calls it; a dead-code sweep that deletes it deletes the plugin API.
#[allow(dead_code)]
pub fn register(capabilities: FormatCapabilities) {
    register_at(Tier::Standard, capabilities);
}

pub fn register_at(tier: Tier, capabilities: FormatCapabilities) {
    live()
        .write()
        .expect("capability table poisoned")
        .register_at(tier, capabilities);
}

pub fn for_kind(kind: DocumentKind) -> Option<FormatCapabilities> {
    live().read().expect("capability table poisoned").get(kind)
}

pub fn for_payload(payload: &DocPayload) -> Option<FormatCapabilities> {
    for_kind(payload.kind())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins() -> CapabilityRegistry {
        CapabilityRegistry::builtin()
    }

    #[test]
    fn every_kind_has_exactly_one_row() {
        for kind in DocumentKind::NAMED {
            let rows = BUILTIN.iter().filter(|r| r.kind == *kind).count();
            assert_eq!(
                rows, 1,
                "{kind:?} has {rows} rows in BUILTIN, expected exactly 1. A kind \
                 with no row answers `None` to every capability, so it opens and \
                 then shows nothing — no outline, no inspector, no status line."
            );
        }
        assert_eq!(
            BUILTIN.len(),
            DocumentKind::NAMED.len(),
            "the table has a row for a kind the enum does not name"
        );
    }

    #[test]
    fn every_payload_a_handler_can_produce_finds_a_row() {
        let builtins = builtins();
        for handler in super::super::payload_handlers::HANDLERS {
            // From the handler's id, which is what the lookup keys on: a
            // pack-authored format has no blank payload to construct.
            let kind = super::DocumentKind::for_resource(handler.id.resource);
            // A registered kind with no row falls through
            // `inspector::body_for` to the described root grid, so only
            // a named kind must have one.
            if matches!(kind, super::DocumentKind::Registered(_)) {
                continue;
            }
            assert!(
                builtins.get(kind).is_some(),
                "{:?} produces a payload with no capability row",
                handler.id
            );
        }
    }

    #[test]
    fn an_enhanced_payload_shares_its_formats_row() {
        let builtins = builtins();
        let legacy = builtins
            .get(DocPayload::Resource(or_io::Asset::TextureDict(Default::default())).kind())
            .expect("a Legacy row");
        let enhanced = builtins
            .get(DocPayload::Resource(or_io::Asset::TextureDictGen9(Default::default())).kind())
            .expect("an Enhanced row");
        assert_eq!(legacy.kind, enhanced.kind);
        assert_eq!(legacy.kind, DocumentKind::Ytd);
    }

    #[test]
    fn a_builtin_row_summarises_its_own_payload() {
        let builtins = builtins();
        let summary = |payload: &DocPayload| {
            let row = builtins.get(payload.kind()).expect("a row");
            let f = row.selection_summary.expect("a summary");
            f(payload, 0)
        };

        assert_eq!(
            summary(&DocPayload::Resource(or_io::Asset::TextureDict(
                Default::default()
            ))),
            "0 textures"
        );
        assert_eq!(
            summary(&DocPayload::Resource(or_io::Asset::TextureDictGen9(
                Default::default()
            ))),
            "0 textures (Enhanced)"
        );
        assert_eq!(
            summary(&DocPayload::Resource(or_io::Asset::Bounds(
                or_io::gta5::PhBoundAny::PhBoundSphere(Default::default())
            ))),
            "Bound selected"
        );
        assert_eq!(
            summary(&DocPayload::Resource(or_io::Asset::Clips(
                Default::default()
            ))),
            "0 clips, 0 animations"
        );
    }

    #[test]
    fn a_preferred_row_beats_a_standard_one_registered_after_it() {
        fn preferred(_: &DocPayload, _: u64) -> String {
            "preferred".to_string()
        }
        fn standard(_: &DocPayload, _: u64) -> String {
            "standard".to_string()
        }

        let mut registry = CapabilityRegistry::builtin();
        registry.register_at(
            Tier::Preferred,
            FormatCapabilities {
                selection_summary: Some(preferred),
                ..FormatCapabilities::none(DocumentKind::Gxt2)
            },
        );
        registry.register_at(
            Tier::Standard,
            FormatCapabilities {
                selection_summary: Some(standard),
                ..FormatCapabilities::none(DocumentKind::Gxt2)
            },
        );

        let row = registry.get(DocumentKind::Gxt2).expect("a row");
        let f = row.selection_summary.expect("a summary");
        assert_eq!(
            f(&DocPayload::Gxt2(Default::default()), 0),
            "preferred",
            "tier precedence, not registration order, decides"
        );
    }

    #[test]
    fn an_outline_and_its_chevron_agree() {
        for row in BUILTIN {
            assert_eq!(
                row.outline.is_some(),
                row.has_outline_children.is_some(),
                "{:?}: one of outline / has_outline_children is missing",
                row.kind
            );
        }
    }

    #[test]
    fn only_the_formats_another_panel_draws_are_leaves() {
        let leaves: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.outline.is_none())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            leaves,
            vec![
                DocumentKind::Gxt2,
                DocumentKind::Bik,
                DocumentKind::Awc,
                DocumentKind::HeightMap,
                DocumentKind::DistantLights,
                DocumentKind::WaterMap,
                DocumentKind::WorldBlanket,
                DocumentKind::NavPrecalc,
                DocumentKind::DatTable,
                DocumentKind::Rel,
                DocumentKind::NameTable,
                DocumentKind::Gfx,
                DocumentKind::MoveNetwork,
                DocumentKind::Dds,
                DocumentKind::Rpf9,
                DocumentKind::Gta6Text,
                DocumentKind::Gta6AudioData,
                DocumentKind::GameRoot,
                DocumentKind::Folder,
                DocumentKind::Text,
                DocumentKind::Meta,
                DocumentKind::Hex,
                DocumentKind::Compare,
            ]
        );
    }

    #[test]
    fn emptiness_decides_the_chevron_only_where_the_body_is_a_list() {
        let builtins = builtins();
        let has = |payload: &DocPayload| {
            builtins
                .get(payload.kind())
                .and_then(|row| row.has_outline_children)
                .is_some_and(|f| f(payload))
        };

        assert!(!has(&DocPayload::Resource(or_io::Asset::TextureDict(
            Default::default()
        ))));
        assert!(!has(&DocPayload::Resource(or_io::Asset::Clips(
            Default::default()
        ))));
        assert!(has(&DocPayload::Resource(or_io::Asset::Drawable(
            Default::default()
        ))));
        assert!(has(&DocPayload::Resource(or_io::Asset::Bounds(
            or_io::gta5::PhBoundAny::PhBoundSphere(Default::default())
        ))));
    }

    #[test]
    fn an_enhanced_dictionary_answers_the_chevron_question_too() {
        let builtins = builtins();
        let row = builtins.get(DocumentKind::Ytd).expect("a row");
        let has = row.has_outline_children.expect("a cell");

        assert!(!has(&DocPayload::Resource(or_io::Asset::TextureDictGen9(
            Default::default()
        ))));
        assert!(!has(&DocPayload::Resource(or_io::Asset::TextureDict(
            Default::default()
        ))));
    }

    #[test]
    fn every_reachable_format_has_an_inspector_or_a_described_root() {
        for row in BUILTIN {
            if matches!(row.kind, DocumentKind::GameRoot | DocumentKind::Folder) {
                continue;
            }
            assert!(
                row.inspector.is_some()
                    || or_io::gta5::formats::format(row.kind.extension()).is_some(),
                "{:?} would show a blank inspector",
                row.kind
            );
        }
    }

    #[test]
    fn a_format_with_no_describable_root_keeps_its_own_panel() {
        let row = builtins().get(DocumentKind::Ypdb).expect("a row");
        assert!(or_io::gta5::formats::format(row.kind.extension()).is_none());
        assert!(row.inspector.is_some());
    }

    #[test]
    fn every_row_that_draws_can_also_be_framed() {
        for row in BUILTIN {
            assert_eq!(
                row.meshes.is_some(),
                row.bounds.is_some(),
                "{:?}: meshes and bounds disagree",
                row.kind
            );
            assert_eq!(
                row.meshes.is_some(),
                row.lines.is_some(),
                "{:?}: meshes and wireframe disagree",
                row.kind
            );
        }
    }

    #[test]
    fn listing_a_texture_and_fetching_it_move_together() {
        for row in BUILTIN {
            assert_eq!(
                row.textures.is_some(),
                row.texture_at.is_some(),
                "{:?}: lists textures it cannot fetch, or the reverse",
                row.kind
            );
            assert_eq!(
                row.texture_for_selection.is_some(),
                row.texture_at.is_some(),
                "{:?}: resolves a texture selection it cannot fetch, or the reverse",
                row.kind
            );
        }
    }

    #[test]
    fn the_formats_with_browsable_textures_are_the_eight_that_have_them() {
        let browsable: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.textures.is_some())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            browsable,
            vec![
                DocumentKind::Ypt,
                DocumentKind::Ytd,
                DocumentKind::Ydr,
                DocumentKind::Ydd,
                DocumentKind::Yft,
                DocumentKind::Dds,
                DocumentKind::Ttpf,
                DocumentKind::Gta6Drawable,
            ]
        );
    }

    #[test]
    fn a_material_lookup_is_not_the_same_set_as_a_browsable_dictionary() {
        let looking_up: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.texture_named.is_some())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            looking_up,
            vec![
                DocumentKind::Ypt,
                DocumentKind::Ydr,
                DocumentKind::Ydd,
                DocumentKind::Yft,
                DocumentKind::Gta6Drawable,
            ]
        );

        let row_for = |kind| BUILTIN.iter().find(|r| r.kind == kind).expect("a row");
        let fragment = row_for(DocumentKind::Yft);
        assert!(
            fragment.texture_named.is_some() && fragment.textures.is_some(),
            "the fragment gap is closed; a .yft browses and resolves"
        );
        let dictionary = row_for(DocumentKind::Ytd);
        assert!(dictionary.textures.is_some() && dictionary.texture_named.is_none());
    }

    #[test]
    fn a_gallery_tile_resolves_back_to_its_own_texture() {
        let mut dictionary = or_io::gta5::PgDictionary::default();
        dictionary.insert(or_io::gta5::gen8::GrcTexture {
            name: "diffuse".to_string(),
            width: 4,
            height: 8,
            ..Default::default()
        });
        let payload = DocPayload::Resource(or_io::Asset::TextureDict(dictionary));

        let row = builtins().get(DocumentKind::Ytd).expect("a row");
        let TextureListing::Textures { textures, source } =
            (row.textures.expect("a listing cell"))(&payload, None)
        else {
            panic!("a one-entry dictionary should list one texture");
        };
        assert_eq!(textures.len(), 1);
        assert_eq!(textures[0].name, "diffuse");
        assert_eq!((textures[0].width, textures[0].height), (4, 8));

        let resolved = (row.texture_for_selection.expect("a resolution cell"))(
            &payload,
            textures[0].selection_id,
        );
        let TextureResolution::Found { name, source: back } = resolved else {
            panic!("the tile's own id should name the tile's own texture, got {resolved:?}");
        };
        assert_eq!(name, "diffuse");
        assert_eq!(back, source, "and point at the same dictionary");

        let fetched = (row.texture_at.expect("a fetch cell"))(&payload, source, &name);
        assert_eq!(
            fetched.map(|t| (t.width(), t.height())),
            Some((4, 8)),
            "and reach the bytes"
        );
    }

    #[test]
    fn a_row_that_lists_models_can_draw_one() {
        for row in BUILTIN {
            if row.models.is_some() {
                assert!(
                    row.meshes.is_some(),
                    "{:?} lists models it has no way to draw",
                    row.kind
                );
            }
        }
    }

    #[test]
    /// A scanned GTA VI body's drawables are models now, each of a dictionary on its own.
    fn a_collision_bound_and_a_navmesh_draw_and_are_not_listed_as_models() {
        let drawable_but_unlisted: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.meshes.is_some() && row.models.is_none())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            drawable_but_unlisted,
            vec![DocumentKind::Ybn, DocumentKind::Ynv]
        );
    }

    #[test]
    fn the_formats_with_browsable_models_are_the_six_that_have_them() {
        let browsable: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.models.is_some())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            browsable,
            vec![
                DocumentKind::Ypt,
                DocumentKind::Ydr,
                DocumentKind::Ydd,
                DocumentKind::Yft,
                DocumentKind::Yld,
                DocumentKind::Gta6Drawable,
            ]
        );
    }

    #[test]
    fn a_model_tile_resolves_back_to_its_own_entry() {
        use crate::ui::panels::selection_ids::ydd_selected_entry_index;

        let mut first = or_io::gta5::GtaDrawable::default();
        first.base.name = "first".to_string();
        let dict = or_io::gta5::PgDictionary {
            codes: vec![0x1111_1111, 0x2222_2222],
            entries: vec![Some(Box::new(first)), Some(Box::default())],
            ..Default::default()
        };
        let payload = DocPayload::Resource(or_io::Asset::DrawableDict(dict));

        let row = builtins().get(DocumentKind::Ydd).expect("a row");
        let ModelListing::Models(models) = (row.models.expect("a model cell"))(&payload) else {
            panic!("a two-entry dictionary should list two models");
        };
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "first");
        assert_eq!(models[1].name, "#22222222");

        for (index, model) in models.iter().enumerate() {
            assert_eq!(
                ydd_selected_entry_index(&payload, model.selection_id),
                Some(index),
                "tile {index} must resolve to entry {index}"
            );
        }
    }

    #[test]
    fn the_summary_names_the_entry_selected_in_a_dictionary_of_either_target() {
        use crate::ui::panels::selection_ids::dict_entry_id;

        let mut legacy = or_io::gta5::GtaDrawable::default();
        legacy.base.name = "first".to_string();
        let mut enhanced = or_io::gta5::GtaDrawable::<or_io::decl::Gen9>::default();
        enhanced.base.name = "first".to_string();
        let codes = vec![0x1111_1111, 0x2222_2222];
        let payloads = [
            or_io::Asset::DrawableDict(or_io::gta5::PgDictionary {
                codes: codes.clone(),
                entries: vec![Some(Box::new(legacy)), Some(Box::default())],
                ..Default::default()
            }),
            or_io::Asset::DrawableDictGen9(or_io::gta5::PgDictionary {
                codes,
                entries: vec![Some(Box::new(enhanced)), Some(Box::default())],
                ..Default::default()
            }),
        ];

        let row = builtins().get(DocumentKind::Ydd).expect("a row");
        let summary = row.selection_summary.expect("a summary");
        for payload in payloads.map(DocPayload::Resource) {
            let first = summary(&payload, dict_entry_id(0x1111_1111));
            assert!(first.starts_with("Drawable: first (1 of 2)"), "{first}");
            let second = summary(&payload, dict_entry_id(0x2222_2222));
            assert!(
                second.starts_with("Drawable: #22222222 (2 of 2)"),
                "{second}"
            );
        }
    }

    #[test]
    fn a_row_the_picker_offers_can_also_join_a_scene() {
        for row in BUILTIN {
            if row.mesh_sources.is_some() {
                assert!(
                    row.scene_meshes.is_some(),
                    "{:?} is offered as a preview mesh and draws nothing into a scene",
                    row.kind
                );
            }
        }
    }

    #[test]
    fn two_formats_can_be_drawn_into_a_scene_and_never_offered_to_one() {
        let offered: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.mesh_sources.is_some())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            offered,
            vec![DocumentKind::Ydr, DocumentKind::Ydd, DocumentKind::Yft]
        );

        let unreachable: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.scene_meshes.is_some() && row.mesh_sources.is_none())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            unreachable,
            vec![DocumentKind::Ybn, DocumentKind::Yld],
            "the set of scene_meshes rows nothing can reach"
        );
    }

    #[test]
    fn a_fragments_tile_is_addressed_the_way_its_outline_row_is() {
        use crate::ui::panels::selection_ids::is_dict_entry_selection;
        let mut dictionary = or_io::gta5::PgDictionary::default();
        dictionary.insert(or_io::gta5::gen8::GrcTexture {
            name: "frag_diffuse".to_string(),
            width: 2,
            height: 2,
            ..Default::default()
        });
        let group = or_io::gta5::GrmShaderGroup {
            texture_dictionary: Some(Box::new(dictionary)),
            ..Default::default()
        };
        let mut drawable = or_io::gta5::FragDrawable::default();
        drawable.base.shader_group = Some(Box::new(group));
        let mut frag = or_io::gta5::GtaFragType::default();
        frag.base.common_drawable = Some(Box::new(drawable));
        let payload = DocPayload::Resource(or_io::Asset::Fragment(frag));

        let row = builtins().get(DocumentKind::Yft).expect("a row");
        let TextureListing::Textures { textures, source } =
            (row.textures.expect("a listing cell"))(&payload, None)
        else {
            panic!("a fragment with one embedded texture should list it");
        };
        assert_eq!(textures.len(), 1);
        assert!(
            is_dict_entry_selection(textures[0].selection_id),
            "a fragment's tile must carry the hash-keyed entry tag the outline uses, \
             not the drawable section tag"
        );

        let resolved =
            (row.texture_for_selection.expect("a cell"))(&payload, textures[0].selection_id);
        let TextureResolution::Found { name, source: back } = resolved else {
            panic!("the tile's own id should name the tile's own texture, got {resolved:?}");
        };
        assert_eq!(name, "frag_diffuse");
        assert_eq!(back, source);
        assert!((row.texture_at.expect("a cell"))(&payload, source, &name).is_some());
    }

    #[test]
    fn offering_a_skeleton_and_producing_one_move_together() {
        for row in BUILTIN {
            assert_eq!(
                row.skeleton_sources.is_some(),
                row.skeleton_data.is_some(),
                "{:?}: one of the two skeleton cells is missing",
                row.kind
            );
        }
    }

    #[test]
    fn the_formats_with_skeletons_are_the_three_that_have_them() {
        let offering: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.skeleton_sources.is_some())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            offering,
            vec![DocumentKind::Ydr, DocumentKind::Ydd, DocumentKind::Yft]
        );
    }

    #[test]
    fn a_row_that_joins_a_scene_also_draws_on_its_own() {
        for row in BUILTIN {
            if row.scene_meshes.is_some() {
                assert!(
                    row.meshes.is_some(),
                    "{:?} contributes to a scene and cannot draw itself",
                    row.kind
                );
            }
        }
    }

    #[test]
    fn the_formats_with_geometry_are_the_eight_that_have_it() {
        let drawing: Vec<DocumentKind> = BUILTIN
            .iter()
            .filter(|row| row.meshes.is_some())
            .map(|row| row.kind)
            .collect();
        assert_eq!(
            drawing,
            vec![
                DocumentKind::Ypt,
                DocumentKind::Ydr,
                DocumentKind::Ydd,
                DocumentKind::Yft,
                DocumentKind::Ybn,
                DocumentKind::Yld,
                DocumentKind::Ynv,
                DocumentKind::Gta6Drawable,
            ]
        );
    }

    #[test]
    fn a_real_bound_still_draws_through_the_table() {
        let Some(path) = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ybn")
            .find()
            .into_iter()
            .min_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX))
        else {
            or_test_support::skip_corpus(
                "a_real_bound_still_draws_through_the_table",
                "no .ybn corpus staged",
            );
            return;
        };
        let data = std::fs::read(&path).expect("read");
        let bound = <or_io::gta5::PhBoundAny as or_io::gta5::File>::from_bytes(&data)
            .expect("a retail bound should parse");

        let state = AppState::new();
        let doc = Document::new(
            0,
            Some(path.clone()),
            DocPayload::Resource(or_io::Asset::Bounds(bound)),
        );
        let request = GeometryRequest {
            state: &state,
            doc: &doc,
            selection: None,
            entry: None,
        };

        let row = builtins().get(DocumentKind::Ybn).expect("a row");
        let meshes = (row.meshes.expect("a mesh cell"))(&request);
        let lines = (row.lines.expect("a wireframe cell"))(&request);
        let bounds = (row.bounds.expect("a framing cell"))(&request);

        assert!(
            !meshes.is_empty(),
            "{}: a retail bound should tessellate",
            path.display()
        );
        assert!(!lines.is_empty(), "{}: and have edges", path.display());
        let (_, radius) = bounds.unwrap_or_else(|| panic!("{}: and be framable", path.display()));
        assert!(radius > 0.0, "a degenerate framing sphere");
    }

    #[test]
    fn a_format_with_no_geometry_draws_nothing() {
        let state = AppState::new();
        let doc = Document::new(0, None, DocPayload::Gxt2(Default::default()));
        let request = GeometryRequest {
            state: &state,
            doc: &doc,
            selection: None,
            entry: None,
        };

        let row = builtins().get(DocumentKind::Gxt2).expect("a row");
        assert!(row.meshes.is_none());
        assert!(row.lines.is_none());
        assert!(row.bounds.is_none());
        assert!(request.selection.is_none());
    }

    #[test]
    fn a_later_registration_replaces_an_earlier_one_in_the_same_tier() {
        fn second(_: &DocPayload, _: u64) -> String {
            "second".to_string()
        }

        let mut registry = CapabilityRegistry::builtin();
        registry.register(FormatCapabilities {
            selection_summary: Some(second),
            ..FormatCapabilities::none(DocumentKind::Gxt2)
        });

        let row = registry.get(DocumentKind::Gxt2).expect("a row");
        let f = row.selection_summary.expect("a summary");
        assert_eq!(f(&DocPayload::Gxt2(Default::default()), 0), "second");
    }

    /// The payload the loader makes of `bytes` saved as `synthetic.{ext}`.
    fn opened(ext: &str, bytes: &[u8]) -> DocPayload {
        let path = std::path::PathBuf::from(format!("synthetic.{ext}"));
        let kind = DocumentKind::from_path(&path).expect("a kind for the extension");
        let crypto = std::sync::Arc::new(or_io::container::NoOpCryptoProvider::new());
        super::super::document_io::parse_payload(&path, kind, bytes, crypto).expect("it opens")
    }

    /// A region with two vehicle nodes, the first linked, and one junction over a one-sample patch.
    fn synthetic_ynd() -> Vec<u8> {
        use or_io::gta5::{CPathNode, CPathNodeLink, CPathRegion, CPathVirtualJunction};
        let mut linked = CPathNode::default();
        linked.set_num_links(1).expect("one link fits");
        let region = CPathRegion {
            nodes: vec![linked, CPathNode::default()],
            num_nodes_car_nodes: 2,
            links: vec![CPathNodeLink::default()],
            virtual_junctions: vec![CPathVirtualJunction {
                x_samples: 1,
                y_samples: 1,
                ..Default::default()
            }],
            height_samples: vec![0],
            ..Default::default()
        };
        or_io::Asset::PathRegion(region)
            .to_bytes()
            .expect("the region saves")
    }

    /// A mesh with one polygon under a one-leaf quadtree.
    fn synthetic_ynv() -> Vec<u8> {
        use or_io::gta5::{
            AiSplitArray, CNavMesh, CNavMeshQuadTree, SubArray, TNavMeshPoly,
            TNavMeshQuadTreeLeafData,
        };
        let mesh = CNavMesh {
            polys_array: Some(Box::new(AiSplitArray {
                sub_arrays: vec![SubArray {
                    elements: vec![TNavMeshPoly::default()],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            quad_tree: Some(Box::new(CNavMeshQuadTree {
                leaf_data: Some(Box::new(TNavMeshQuadTreeLeafData {
                    polys: vec![0],
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        or_io::Asset::NavMesh(mesh)
            .to_bytes()
            .expect("the mesh saves")
    }

    #[test]
    fn a_path_region_and_a_navmesh_open_as_kinds_of_their_own_with_an_outline() {
        for (ext, bytes, expected, section) in [
            ("ynd", synthetic_ynd(), DocumentKind::Ynd, "Nodes (2)"),
            ("ynv", synthetic_ynv(), DocumentKind::Ynv, "Polygons (1)"),
        ] {
            let payload = opened(ext, &bytes);
            assert_eq!(payload.kind(), expected, ".{ext} opens as its own kind");
            let row = for_payload(&payload).unwrap_or_else(|| panic!(".{ext} has no row"));
            assert!(
                row.has_outline_children.is_some_and(|has| has(&payload)),
                ".{ext}'s outline shows no children"
            );
            let draw = row
                .outline
                .unwrap_or_else(|| panic!(".{ext} has no outline"));

            let mut state = AppState::default();
            state.documents.push(Document::new(1, None, payload));
            let output = egui::Context::default()
                .run_ui(egui::RawInput::default(), |ui| draw(ui, &mut state, 1));
            let texts: Vec<String> = output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_string()),
                    _ => None,
                })
                .collect();
            assert!(
                texts.iter().any(|t| t == section),
                ".{ext}'s outline drew {texts:?}, not {section:?}"
            );
        }
    }
}
