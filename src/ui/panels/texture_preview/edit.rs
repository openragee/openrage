//! The encoding bar over the texture on screen, and the replace and export a tile's menu shares with it.

use or_io::authoring::{EncodeQuality, FormatChoice, TextureBuildOptions};
use or_io::codec::texture::GrcTextureFormat;

use super::{combo_box, kept_together, ENHANCED_TEXTURES};
use crate::state::texture_edit::TextureEdit;
use crate::state::textures::TextureSource;
use crate::state::{textures, AppState, DocPayload, TextureResolution};
use crate::ui::accessible_name::named;
use crate::ui::panels::selection_ids::DOC_ROOT_SEL;
use crate::ui::theme;

/// Format, quality, mip count and the embed pair, over the texture on screen.
///
/// ⚠ Every control is disabled rather than hidden when the document cannot
/// take the edit, with the reason on hover. A hidden control reads as a
/// missing feature.
pub(super) fn draw_encoding_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: crate::state::DocumentId,
    source: TextureSource,
    texture: &textures::Texture<'_>,
    level: u32,
) {
    let opts_id = ui.make_persistent_id(("texture_encode_opts", doc_id, texture.name()));
    let mut choice: EncodeChoice = ui
        .data(|d| d.get_temp::<EncodeChoice>(opts_id))
        .unwrap_or_else(|| EncodeChoice::from(texture));

    let encodable = choice.format.is_encodable();
    // A GTA VI drawable's texture is replaced in the document its key resolves to.
    let bound = (state.document(doc_id))
        .is_some_and(|d| matches!(d.payload, DocPayload::Gta6Drawable(_)))
        .then(|| state.bound_texture_pack(doc_id, texture.name()).map(|_| ()));
    let editable = match bound {
        Some(held) => held.is_ok(),
        None => state.texture_dictionary_is_editable(doc_id, source),
    };
    let carrier = state
        .document(doc_id)
        .map(|d| {
            matches!(
                d.payload,
                DocPayload::Resource(or_io::Asset::Drawable(_) | or_io::Asset::Fragment(_))
                    | DocPayload::Gta6Drawable(_)
            )
        })
        .unwrap_or(false);
    let enhanced = matches!(texture, textures::Texture::Enhanced(_));
    let why_not_edited = if enhanced {
        ENHANCED_TEXTURES.to_string()
    } else if let Some(Err(why)) = bound {
        why.to_string()
    } else {
        why_not_encodable(encodable, editable, choice.format)
    };
    let why_not_carried = if enhanced {
        ENHANCED_TEXTURES
    } else {
        "Only a drawable or fragment can carry textures"
    };

    // What Apply reads is not offered where Apply cannot write
    let inputs_live = editable && !enhanced;
    // ⚠ A pack's texture reached from a drawable is replaced in its own format and mips, so neither is offered.
    let pack = bound == Some(Ok(()));
    // A texture the drawable embeds is replaced in the drawable, in any format and mips.
    let reshapes = pack && state.bound_texture_pack(doc_id, texture.name()) == Ok(doc_id);
    let why_kept = crate::i18n::tr(crate::state::texture_edit::PACK_KEEPS_ITS_FORMAT);
    let why_not_reencoded: &str = if pack { why_kept } else { &why_not_edited };
    let mut apply = false;
    let mut import = false;
    let mut export = false;
    let mut embed = false;
    let mut unembed = false;

    ui.horizontal_wrapped(|ui| {
        let format = egui::RichText::new(format_label(choice.format)).small();
        let format_box = combo_box(
            ui,
            "texture_encode_format",
            96.0,
            format,
            inputs_live && (!pack || reshapes),
            |ui| {
                ui.selectable_value(&mut choice.format, FormatChoice::Automatic, "Automatic")
                    .on_hover_text(
                        "BC1 for an image with no transparency, BC3 for one that has it.",
                    );
                for format in ENCODE_TARGETS {
                    ui.selectable_value(
                        &mut choice.format,
                        FormatChoice::Fixed(format),
                        format!("{format:?}"),
                    );
                }
            },
        );
        named(format_box, "Texture format")
            .on_hover_text("Re-encode in place. The dictionary entry keeps its name.")
            .on_disabled_hover_text(why_not_reencoded);

        let quality = egui::RichText::new(format!("{:?}", choice.quality)).small();
        let quality_box = combo_box(
            ui,
            "texture_encode_quality",
            72.0,
            quality,
            inputs_live,
            |ui| {
                for quality in [
                    EncodeQuality::Fast,
                    EncodeQuality::Normal,
                    EncodeQuality::High,
                ] {
                    ui.selectable_value(&mut choice.quality, quality, format!("{quality:?}"));
                }
            },
        );
        named(quality_box, "Texture quality").on_disabled_hover_text(&why_not_edited);

        let ceiling = or_io::codec::texture::max_mip_levels(
            u32::from(texture.width()),
            u32::from(texture.height()),
        );
        let mips = ui.add_enabled(
            inputs_live && (!pack || reshapes),
            egui::DragValue::new(&mut choice.mip_levels)
                .range(1..=ceiling.max(1))
                .prefix("mips "),
        );
        named(mips, "Mips to keep")
            .on_hover_text(format!(
                "1 to {ceiling} for {}x{}. Dropping the top mip is how a \
             half-resolution variant is derived.",
                texture.width(),
                texture.height()
            ))
            .on_disabled_hover_text(why_not_reencoded);

        ui.add_enabled(inputs_live, egui::Checkbox::new(&mut choice.srgb, "sRGB"))
            .on_disabled_hover_text(&why_not_edited);

        let apply_button = ui.add_enabled(
            encodable && editable && !pack,
            egui::Button::new("Apply").small(),
        );
        if apply_button
            .on_disabled_hover_text(why_not_reencoded)
            .clicked()
        {
            apply = true;
        }

        if ui
            .add_enabled(
                encodable && editable,
                egui::Button::new("Replace from image…").small(),
            )
            .on_hover_text(format!(
                "Replace these pixels from a {} file. The entry keeps its name \
                 and its usage classification.",
                or_io::authoring::texture_import::SourceFormat::ALL
                    .map(|f| f.label())
                    .join(" / ")
            ))
            .on_disabled_hover_text(&why_not_edited)
            .clicked()
        {
            import = true;
        }

        if ui
            .button(egui::RichText::new("Export…").small())
            .on_hover_text("Write the mip on screen as a PNG, or every mip as a DDS")
            .clicked()
        {
            export = true;
        }

        kept_together(ui, "texture_carry_pair", |ui, separated| {
            if separated {
                ui.separator();
            }
            if ui
                .add_enabled(carrier, egui::Button::new("Embed").small())
                .on_hover_text("Carry this texture in the drawable itself")
                .on_disabled_hover_text(why_not_carried)
                .clicked()
            {
                embed = true;
            }
            if ui
                .add_enabled(carrier, egui::Button::new("Unembed").small())
                .on_hover_text(
                    "Stop carrying it. The material keeps naming it, so it resolves \
                     through an open dictionary instead.",
                )
                .on_disabled_hover_text(why_not_carried)
                .clicked()
            {
                unembed = true;
            }
        });
    });
    let format = texture.format();
    if format.is_compressed() {
        let warning = format!("⚠ Re-encoding {format:?} loses detail");
        ui.label(egui::RichText::new(warning).small().color(theme::WARNING))
            .on_hover_text(
                "Apply decodes the texture and compresses it again. Replace from image… with the original \
                 image to keep its detail.",
            );
    }

    ui.data_mut(|d| d.insert_temp(opts_id, choice));

    if apply {
        let outcome = state.reencode_texture_in_place(
            doc_id,
            source,
            texture.name(),
            choice.format,
            &choice.build_options(),
        );
        report(state, outcome);
    }
    if import {
        let picked = crate::platform::file_dialogs::pick_source_image("Replace from image");
        if let (Some(path), Some(_)) = (&picked, bound) {
            let (name, options) = (texture.name(), choice.build_options());
            state.replace_bound_texture(doc_id, name, path, choice.format, &options);
        } else if let Some(path) = picked {
            let outcome = or_io::authoring::texture_import::load(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|image| {
                    state.import_texture_over(
                        doc_id,
                        source,
                        texture.name(),
                        &image,
                        choice.format,
                        &choice.build_options(),
                    )
                });
            report(state, outcome);
        }
    }
    if export {
        if let Some(path) = crate::platform::file_dialogs::ask("Export Texture", |dialog| {
            dialog
                .set_file_name(format!("{}.png", texture.name()))
                .add_filter("PNG", &["png"])
                .add_filter("DDS", &["dds"])
                .save_file()
        }) {
            match textures::export_texture(texture, level, &path) {
                Ok(()) => {
                    let said = wrote(state, doc_id, texture.name(), &path);
                    state.show_toast(said)
                }
                Err(message) => state.show_error_toast(message),
            }
        }
    }
    // A GTA VI drawable embeds and unembeds by the key its slots bind.
    let not_carried = || Err(tr!("Only a drawable or fragment can carry textures").to_string());
    if embed {
        let outcome = match texture {
            textures::Texture::Owned(t) if bound.is_some() => {
                (state.embed_gta6_texture(doc_id, t)).unwrap_or_else(not_carried)
            }
            textures::Texture::Owned(t) => state.embed_texture(doc_id, t.clone().into_owned()),
            textures::Texture::Enhanced(_) => {
                Err("An Enhanced texture cannot be carried by a Legacy drawable".to_string())
            }
        };
        report(state, outcome);
    }
    if unembed {
        let outcome = match bound {
            Some(_) => {
                (state.unembed_gta6_texture(doc_id, texture.name())).unwrap_or_else(not_carried)
            }
            None => state.unembed_texture(doc_id, texture.name()),
        };
        report(state, outcome);
    }
}

/// What an export of texture `name` of `doc_id` to `path` says: one of a GTA VI pack's texture
/// read through fitted tables says it is an estimate.
fn wrote(
    state: &AppState,
    doc_id: crate::state::DocumentId,
    name: &str,
    path: &std::path::Path,
) -> String {
    let (estimate, doubt) = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Ttpf(ttpf)) => match ttpf.index_of(name) {
            Some(index) => (ttpf.estimate_label(index), ttpf.doubt_label(index)),
            None => (None, None),
        },
        _ => (None, None),
    };
    match (estimate, doubt) {
        (Some(how), _) => tr!(
            "Wrote {path}. This texture is {how}: an estimate, not the blocks the pack stores",
            path = path.display(),
            how = how
        ),
        (None, Some(how)) => format!(
            "Wrote {}. {}",
            path.display(),
            tr!("Not called exact: {how}", how = how)
        ),
        (None, None) => format!("Wrote {}", path.display()),
    }
}

fn report(state: &mut AppState, outcome: Result<TextureEdit, String>) {
    match outcome {
        Ok(edit) => state.show_toast(edit.describe()),
        Err(message) => state.show_error_toast(message),
    }
}

/// The texture the selection of `doc_id` names, or the only one the document holds, and where the document keeps it.
pub(crate) fn selected_texture(
    state: &AppState,
    doc_id: crate::state::DocumentId,
) -> Option<(String, TextureSource)> {
    let doc = state.document(doc_id)?;
    let selection = (state.selection.selected(doc_id)).filter(|&s| s != DOC_ROOT_SEL);
    let resolution = match (selection, textures::sole_texture(&doc.payload)) {
        (_, Some(name)) => return Some((name, TextureSource::Document)),
        (Some(selection), None) => textures::texture_for_selection(&doc.payload, selection),
        (None, None) => return None,
    };
    match resolution {
        TextureResolution::Found { name, source } => Some((name, source)),
        _ => None,
    }
}

/// The texture `name` of `doc_id`, owned, so the document can be changed while it is held.
fn owned_texture(
    state: &AppState,
    doc_id: crate::state::DocumentId,
    source: TextureSource,
    name: &str,
) -> Option<textures::Texture<'static>> {
    let doc = state.document(doc_id)?;
    textures::texture_at(&doc.payload, source, name).map(textures::Texture::into_owned)
}

/// Replace the pixels of the texture `name` from the image at `path`, or from one the person picks when there is none: what the bar's Replace from image… does, with the texture's own format and levels, for a tile's menu and an image dropped on a tile.
pub(crate) fn replace_from_image(
    state: &mut AppState,
    doc_id: crate::state::DocumentId,
    source: TextureSource,
    name: &str,
    path: Option<std::path::PathBuf>,
) {
    let Some(texture) = owned_texture(state, doc_id, source, name) else {
        state.show_error_toast(format!("{name} is no longer in the document"));
        return;
    };
    let choice = EncodeChoice::from(&texture);
    let refusal = if matches!(texture, textures::Texture::Enhanced(_)) {
        ENHANCED_TEXTURES.to_string()
    } else {
        let editable = state.texture_dictionary_is_editable(doc_id, source);
        why_not_encodable(choice.format.is_encodable(), editable, choice.format)
    };
    if !refusal.is_empty() {
        state.show_error_toast(refusal);
        return;
    }
    let picked = || crate::platform::file_dialogs::pick_source_image("Replace from image");
    let Some(path) = path.or_else(picked) else {
        return;
    };
    let outcome = or_io::authoring::texture_import::load(&path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|image| {
            state.import_texture_over(
                doc_id,
                source,
                name,
                &image,
                choice.format,
                &choice.build_options(),
            )
        });
    report(state, outcome);
}

/// Write the texture `name` where the person says, its first level as a PNG or every level as a DDS: the bar's Export…, for a tile's menu.
pub(crate) fn export(
    state: &mut AppState,
    doc_id: crate::state::DocumentId,
    source: TextureSource,
    name: &str,
) {
    let Some(texture) = owned_texture(state, doc_id, source, name) else {
        state.show_error_toast(format!("{name} is no longer in the document"));
        return;
    };
    let asked = crate::platform::file_dialogs::ask("Export Texture", |dialog| {
        dialog
            .set_file_name(format!("{name}.png"))
            .add_filter("PNG", &["png"])
            .add_filter("DDS", &["dds"])
            .save_file()
    });
    if let Some(path) = asked {
        match textures::export_texture(&texture, 0, &path) {
            Ok(()) => {
                let said = wrote(state, doc_id, name, &path);
                state.show_toast(said)
            }
            Err(message) => state.show_error_toast(message),
        }
    }
}

/// The formats a texture can be re-encoded *into*.
///
/// ⚠ Narrower than what can be decoded on purpose: a format with no encoder
/// offered here would fail on Apply rather than at the point of choosing.
/// BC7 is listed because `is_encodable` reports honestly on whether this
/// build has its encoder, and the button says so when it does not.
const ENCODE_TARGETS: [GrcTextureFormat; 7] = [
    GrcTextureFormat::DXT1,
    GrcTextureFormat::DXT3,
    GrcTextureFormat::DXT5,
    GrcTextureFormat::DXT5A,
    GrcTextureFormat::DXN,
    GrcTextureFormat::BC7,
    GrcTextureFormat::A8R8G8B8,
];

fn format_label(choice: FormatChoice) -> String {
    match choice {
        FormatChoice::Automatic => "Automatic".to_string(),
        FormatChoice::Fixed(format) => format!("{format:?}"),
    }
}

fn why_not_encodable(encodable: bool, editable: bool, choice: FormatChoice) -> String {
    if !editable {
        return "This document has no editable Legacy texture dictionary.".to_string();
    }
    if !encodable {
        return format!(
            "{} has no encoder in this build. BC7 needs the bc7-encode feature, \
             which is off by default.",
            format_label(choice)
        );
    }
    String::new()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct EncodeChoice {
    format: FormatChoice,
    quality: EncodeQuality,
    mip_levels: u32,
    srgb: bool,
}

impl EncodeChoice {
    fn from(texture: &textures::Texture<'_>) -> Self {
        Self {
            format: FormatChoice::Fixed(texture.format()),
            quality: EncodeQuality::default(),
            mip_levels: u32::from(texture.mip_count()).max(1),
            srgb: true,
        }
    }

    fn build_options(self) -> TextureBuildOptions {
        TextureBuildOptions {
            mip_levels: Some(self.mip_levels),
            srgb: self.srgb,
            quality: self.quality,
            ..TextureBuildOptions::default()
        }
    }
}
