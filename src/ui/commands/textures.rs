//! The commands of the texture steps of a pack: normal and specular maps, a budget, and the materials of its models completed;
//! and those of a texture a GTA VI model binds, which act in the document that holds it.

use or_io::authoring::{FormatChoice, TextureBuildOptions};

use super::table::{Action, On, Spec, BASE};
use super::target::Target;
use super::wardrobe::a_pack_with_garments;
use crate::platform::file_dialogs;
use crate::state::{textures, AppState, DocPayload, DocumentId, TextureResolution};

/// The first thing a person does with a texture dictionary, as a command: the tab row offers it as the primary action of a texture file.
pub(super) const REPLACE_TEXTURE_FROM_IMAGE: Spec = Spec {
    action: Action::ReplaceTextureFromImage,
    id: "replace_texture_from_image",
    label: tr_key!("Replace from image…"),
    category: tr_key!("Texture"),
    hover: tr_key!(
        "Replace the pixels of the selected texture from an image file. The entry keeps its name, its format and its levels"
    ),
    on: Some(On {
        focused: texture_target,
        enabled: |state, target| match target {
            Target::Texture { doc, name } => state.bound_texture_pack(*doc, name).map(|_| ()),
            Target::Document(doc) => editable_texture(state, *doc).map(|_| ()),
            _ => Err(crate::i18n::tr(NO_TEXTURE)),
        },
        label: None,
        run: |state, _, target| match target {
            Target::Texture { doc, name } => {
                if let Some(image) = file_dialogs::pick_source_image(tr!("Replace from image")) {
                    // A pack keeps the texture's format and mips, so the tile asks for neither.
                    let (format, options) =
                        (FormatChoice::Automatic, TextureBuildOptions::default());
                    state.replace_bound_texture(*doc, name, &image, format, &options);
                }
            }
            Target::Document(doc) => {
                use crate::ui::panels::texture_preview::replace_from_image;
                if let Ok((name, source)) = editable_texture(state, *doc) {
                    replace_from_image(state, *doc, source, &name, None);
                }
            }
            _ => {}
        },
    }),
    ..BASE
};

/// The selected texture written out, which a tile's menu offers beside Replace from image….
pub(super) const EXPORT_TEXTURE: Spec = Spec {
    action: Action::ExportTexture,
    id: "export_texture",
    label: tr_key!("Export texture…"),
    category: tr_key!("Texture"),
    hover: tr_key!(
        "Write the selected texture where you say: its first level as a PNG, or every level as a DDS"
    ),
    on: Some(On {
        focused: texture_target,
        enabled: |state, target| match target {
            Target::Texture { doc, name } => state.bound_texture_holder(*doc, name).map(|_| ()),
            Target::Document(doc) => own_texture(state, *doc).map(|_| ()),
            _ => Err(crate::i18n::tr(NO_TEXTURE)),
        },
        label: None,
        run: |state, _, target| match target {
            // Read from the document that holds it: its top mip as a PNG, or every mip as a DDS.
            Target::Texture { doc, name } => {
                let picked = file_dialogs::ask(tr!("Export Texture"), |dialog| {
                    dialog
                        .set_file_name(format!("{name}.png"))
                        .add_filter("PNG", &["png"])
                        .add_filter("DDS", &["dds"])
                        .save_file()
                });
                if let Some(path) = picked {
                    state.export_bound_texture(*doc, name, &path);
                }
            }
            Target::Document(doc) => {
                if let Ok((name, source)) = own_texture(state, *doc) {
                    crate::ui::panels::texture_preview::export(state, *doc, source, &name);
                }
            }
            _ => {}
        },
    }),
    ..BASE
};

pub(super) const MAKE_MAPS: Spec = Spec {
    action: Action::MakeMaps,
    id: "make_maps",
    label: tr_key!("Make normal and specular maps…"),
    category: tr_key!("Project ▸ Fix"),
    hover: tr_key!("Make a normal map and a specular map of each diffuse texture of the garments picked, or of the whole pack, and list them first. They are estimates from the brightness of the diffuse, written as new pictures in a copy of the texture's source: the garments name the copy, the source it was is untouched, and Undo takes the copy away"),
    enabled: a_pack_with_garments,
    run: |state, _| state.ask_to_make_maps(),
    ..BASE
};

pub(super) const BUDGET_TEXTURES: Spec = Spec {
    action: Action::BudgetTextures,
    id: "budget_textures",
    label: tr_key!("Budget textures…"),
    category: tr_key!("Project ▸ Fix"),
    hover: tr_key!("Pick the memory the textures of the garments picked, or of the whole pack, are to fit: each texture is given the size and format that lose the least picture per byte saved, with the PSNR of every choice and the texture before and after side by side. Nothing is written until the plan is accepted; then the garments name lighter copies of the sources, and Undo takes it back"),
    enabled: a_pack_with_garments,
    run: |state, _| state.ask_to_budget(),
    ..BASE
};

pub(super) const COMPLETE_MATERIALS: Spec = Spec {
    action: Action::CompleteMaterials,
    id: "complete_materials",
    label: tr_key!("Complete materials"),
    category: tr_key!("Project ▸ Fix"),
    hover: tr_key!("Give each material, in the models of the garments picked or of the whole pack, the values its shader expects and the material does not state, at the values the shader itself gives them; a texture a material lacks is left as it is. It reads the shaders from a game install. Each model that changes becomes a new source the garment names, as one step of Undo"),
    enabled: a_pack_with_garments,
    run: |state, _| {
        if let Some(module) = state.wardrobe_module_id() {
            let picked = state.picked_garments();
            state.complete_materials(module, (!picked.is_empty()).then_some(picked));
        }
    },
    ..BASE
};

const NO_TEXTURE: &str = tr_key!("Select a texture of a GTA VI model in the Gallery first");

/// What a texture command acts on: the texture selected in a GTA VI model, which lies in another
/// document, or else the active document, which holds the texture selected in it.
fn texture_target(state: &AppState) -> Result<Target, &'static str> {
    let doc = state.active_document().ok_or(tr!("No document is open"))?;
    match doc.payload {
        DocPayload::Gta6Drawable(_) => selected_texture(state),
        _ => Ok(Target::Document(doc.id)),
    }
}

/// The texture selected in document `doc`, which holds it.
fn own_texture(
    state: &AppState,
    doc: DocumentId,
) -> Result<(String, textures::TextureSource), &'static str> {
    crate::ui::panels::texture_preview::selected_texture(state, doc)
        .ok_or(tr!("Select a texture first"))
}

fn editable_texture(
    state: &AppState,
    doc: DocumentId,
) -> Result<(String, textures::TextureSource), &'static str> {
    let document = state
        .document(doc)
        .ok_or(tr!("That document is no longer open"))?;
    if !crate::state::gta6_edit::is_scanned_dictionary(document) {
        if let Some(why) = document.read_only {
            return Err(crate::i18n::tr(why));
        }
    }
    own_texture(state, doc)
}

/// The texture selected in the active document, where that is a GTA VI model.
fn selected_texture(state: &AppState) -> Result<Target, &'static str> {
    let none = crate::i18n::tr(NO_TEXTURE);
    let doc = (state.active_document())
        .filter(|doc| matches!(doc.payload, DocPayload::Gta6Drawable(_)))
        .ok_or(none)?;
    let selected = state.selection.selected(doc.id).ok_or(none)?;
    match textures::texture_for_selection(&doc.payload, selected) {
        TextureResolution::Found { name, .. } => Ok(Target::Texture { doc: doc.id, name }),
        _ => Err(none),
    }
}
