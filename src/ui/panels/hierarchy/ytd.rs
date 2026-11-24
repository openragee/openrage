use super::*;
use crate::ui::panels::inspector::plural;

pub(super) fn draw_ytd(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(textures) = state
        .document(doc_id)
        .and_then(|d| crate::state::textures::legacy_dictionary(&d.payload))
        .map(collect_textures)
    else {
        return;
    };

    draw_import_button(ui, state, doc_id);

    if textures.is_empty() {
        empty_centered(ui, "No textures in this dictionary");
        return;
    }

    let filter_active = !super::ui_state(state).filter.is_empty();
    let filter_lc = super::ui_state(state).filter.to_ascii_lowercase();
    let visible_count = if filter_active {
        textures
            .iter()
            .filter(|(name, _, _, _)| name.to_ascii_lowercase().contains(&filter_lc))
            .count()
    } else {
        textures.len()
    };
    let count_text = if filter_active {
        format!("{} of {} textures match", visible_count, textures.len())
    } else {
        plural(textures.len(), "texture")
    };
    ui.label(egui::RichText::new(count_text).color(theme::TEXT_SECONDARY));
    ui.add_space(crate::ui::theme::SPACING_HAIR);

    let shown: Vec<&(String, u32, u32, GrcTextureFormat)> = textures
        .iter()
        .filter(|(name, ..)| !filter_active || name.to_ascii_lowercase().contains(&filter_lc))
        .collect();
    crate::ui::virtual_rows::show(ui, ("ytd", doc_id), shown.len(), None, |ui, row| {
        let (name, width, height, format) = shown[row];
        let sel_id = name_hash(name);
        let selected = state.selection.is_selected(doc_id, sel_id);
        let label_color = if selected {
            theme::ACCENT_TEXT
        } else {
            theme::TEXT_PRIMARY
        };
        let job = icons::icon_label(&icons::ytd_texture_icon(), name, label_color.into());
        let clicked = ui.selectable_label(selected, job).clicked();
        ui.add(detail_label(format!(
            "{}\u{00d7}{}  {:?}",
            width, height, format
        )));
        if clicked {
            state.selection.select(doc_id, sel_id);
            state.active_workspace = crate::ui::dock::Workspace::Texture;
        }
    });
}

fn collect_textures(
    td: &or_io::gta5::PgDictionary<or_io::gta5::gen8::GrcTexture>,
) -> Vec<(String, u32, u32, GrcTextureFormat)> {
    td.entries
        .iter()
        .flatten()
        .map(|t| {
            let format = GrcTextureFormat::from(t.format);
            (t.name.clone(), t.width as u32, t.height as u32, format)
        })
        .collect()
}

/// Bring an image file in as a new entry.
///
/// ⚠ Here rather than in the texture preview panel because an empty
/// dictionary has nothing to select, and an empty dictionary is exactly the
/// one a user needs to put a texture into.
pub(super) fn draw_import_button(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let editable = state
        .texture_dictionary_is_editable(doc_id, crate::state::textures::TextureSource::Document);
    let clicked = ui
        .add_enabled(editable, egui::Button::new(tr!("Add texture from image…")))
        .on_hover_text(format!(
            "Encode a {} file into this dictionary as a new entry. The entry is named after \
             the file, and its format is chosen from the pixels.",
            or_io::authoring::texture_import::SourceFormat::ALL
                .map(|f| f.label())
                .join(" / ")
        ))
        .clicked();
    ui.add_space(crate::ui::theme::SPACING_HAIR);
    if !clicked {
        return;
    }

    let Some(path) = crate::platform::file_dialogs::pick_source_image("Add texture from image")
    else {
        return;
    };
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("texture")
        .to_ascii_lowercase();
    let outcome = or_io::authoring::texture_import::load(&path)
        .map_err(|e| format!("{}: {e}", path.display()))
        .and_then(|image| {
            state.import_texture_as_new(
                doc_id,
                &name,
                &image,
                or_io::authoring::FormatChoice::Automatic,
                &Default::default(),
            )
        });
    match outcome {
        Ok(edit) => {
            state.selection.select(doc_id, name_hash(&name));
            state.show_toast(edit.describe());
        }
        Err(message) => state.show_error_toast(message),
    }
}
