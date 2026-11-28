use super::*;

/// One row per texture of a pack, drawing only the rows in view: a pack can list ten thousand.
pub(super) fn draw_ttpf(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Ttpf(ttpf)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let ttpf = ttpf.clone();
    let count = ttpf.pack.entries.len();
    ui.label(
        egui::RichText::new(tr_n!("{count} texture", "{count} textures", count))
            .color(theme::TEXT_SECONDARY),
    );
    // A pack holds no names: one typed whose hash keys a texture names it from then on.
    let id = egui::Id::new(("ttpf_name", doc_id));
    let mut typed: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let field = ui.add(
        egui::TextEdit::singleline(&mut typed).hint_text(tr!("Type a texture's name to find it")),
    );
    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !typed.is_empty() {
        match ttpf.learn(&typed) {
            Some(index) => state.selection.select(doc_id, index as u64),
            None => state.show_info_toast(tr!(
                "No texture of this pack is keyed by the hash of {name}",
                name = typed
            )),
        }
    }
    ui.data_mut(|d| d.insert_temp(id, typed));
    // A texture with no name is still used by a model: the rows narrow to a model's.
    let id = egui::Id::new(("ttpf_used_by", doc_id));
    let mut model: String = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    ui.add(
        egui::TextEdit::singleline(&mut model)
            .hint_text(tr!("Filter by the model that uses a texture")),
    )
    .on_hover_text(tr!(
        "Show only the textures used by a model whose name holds this text. A harvest of the game's names finds, for each texture, the first model whose material binds it"
    ));
    let shown = rows_used_by(ui, doc_id, &ttpf, &model);
    ui.data_mut(|d| d.insert_temp(id, model));
    let load = ui.button(tr!("Load names…")).on_hover_text(tr!(
        "Name the textures of the open packs from a text file: each line a texture's name, or a drawable's name the naming rules expand"
    ));
    if load.clicked() {
        let picked = crate::platform::file_dialogs::ask(tr!("Load texture names"), |dialog| {
            (dialog.add_filter("Text", &["txt"]))
                .add_filter("All Files", &["*"])
                .pick_file()
        });
        if let Some(path) = picked {
            state.load_texture_names(&path);
        }
    }
    // The coder's tables are in no file: they are fitted from every pack of the game together.
    // A game whose textures wait on that is offered it, until it is done or declined.
    use crate::state::ttpf_document::calibration;
    let id = egui::Id::new(("ttpf_offer", doc_id));
    let offered = ui
        .data_mut(|d| *d.get_temp_mut_or_insert_with(id, || calibration::offered(&ttpf.pack)))
        && !ttpf.pack.is_calibrated()
        && !calibration::running();
    if offered {
        ui.label(tr!(
            "Some textures of this game are stored through the console's coder and do not read yet. A calibration fits what they need from the game's own packs, once: about a quarter of an hour, which can be stopped and taken up again. They then read as estimates, marked calibrated"
        ));
        if ui.button(tr!("Not now")).clicked() {
            calibration::decline(&ttpf.pack);
            ui.data_mut(|d| d.insert_temp(id, false));
        }
    }
    // Tables the user supplied are read in place of a calibration, so none is offered: the list
    // says how the file stands.
    let supplied = ttpf.tables_note();
    if let Some(note) = &supplied {
        ui.label(egui::RichText::new(note).color(theme::TEXT_SECONDARY));
    }
    if supplied.is_none() && ttpf.pack.envelope().has_coder() {
        let calibrate = ui.button(tr!("Calibrate decoding")).on_hover_text(tr!(
            "Fit the tables of the console's texture coder from every pack of this game, so that the textures stored through it for BC3, BC4, BC5, BC6H and BC7 read. No file holds those tables, so such a texture is an estimate and is marked calibrated"
        ));
        if calibrate.clicked() {
            let folder = crate::state::ttpf_document::calibration::folder_of(ttpf.pack.path());
            state.calibrate_texture_decoding(&folder);
        }
    }
    ui.add_space(crate::ui::theme::SPACING_HAIR);

    let row_height = ui.spacing().interact_size.y;
    let mut clicked = None;
    // The rows are every texture, or those the filter keeps.
    let count = shown.as_ref().map_or(count, |shown| shown.len());
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let top = ui.cursor().top();
        let clip = ui.clip_rect();
        let at = |y: f32| (((y - top) / row_height).max(0.0) as usize).min(count);
        let (first, end) = (at(clip.top()), (at(clip.bottom()) + 1).min(count));
        let first = first.min(end);
        ui.add_space(first as f32 * row_height);
        for row in first..end {
            let index = match &shown {
                Some(shown) => shown.get(row).copied(),
                None => Some(row),
            };
            let Some((index, entry)) = index.and_then(|i| Some((i, ttpf.pack.entries.get(i)?)))
            else {
                continue;
            };
            let format = entry
                .format()
                .map_or_else(|| format!("{:#05x}", entry.format_code), |f| f.name());
            // A texture through the coder says what its read may be called.
            let status = (ttpf.refusal(index))
                .or_else(|| ttpf.decode_label(index))
                .unwrap_or_else(|| tr!("readable").to_string());
            let selected = state.selection.is_selected(doc_id, index as u64);
            let response = ui
                .horizontal(|ui| {
                    ui.set_height(row_height);
                    // A texture with no name says the model that uses it, as its tile does.
                    // ⚠ The panel is narrow: what leads the row is what is read, and the pointer
                    // over it says the rest, the status before the format since it varies.
                    let name = ttpf.name(index);
                    let used = ttpf.use_label(index);
                    let detail = format!(
                        "{}\u{00d7}{}  {status}  {format}",
                        entry.width, entry.height
                    );
                    let whole = match &used {
                        Some(used) => format!("{name}\n{used}\n{detail}"),
                        None => format!("{name}\n{detail}"),
                    };
                    let response = ui.selectable_label(selected, used.unwrap_or(name));
                    // A detail cut short says itself whole under the pointer.
                    ui.add(detail_label(detail));
                    response.on_hover_text(whole)
                })
                .inner;
            if response.clicked() {
                clicked = Some(index);
            }
        }
        ui.add_space((count - end) as f32 * row_height);
    });
    if let Some(index) = clicked {
        state.selection.select(doc_id, index as u64);
        state.active_workspace = crate::ui::dock::Workspace::Texture;
    }
}

/// What the rows kept were found for: the text typed, the uses served and the pack's length.
type UsedByStamp = (String, usize, usize);

/// The textures of `ttpf` used by a model whose name holds `typed`, by index; `None` with
/// nothing typed, which shows every texture.
// Kept until the text, the uses served or the pack changes: a pack lists a hundred thousand keys.
fn rows_used_by(
    ui: &egui::Ui,
    doc_id: DocumentId,
    ttpf: &crate::state::ttpf_document::TtpfDocument,
    typed: &str,
) -> Option<std::sync::Arc<Vec<usize>>> {
    use std::sync::Arc;
    let needle = typed.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    let uses = crate::state::gta6_names::texture_uses();
    let entries = &ttpf.pack.entries;
    let stamp: UsedByStamp = (needle, Arc::as_ptr(&uses) as usize, entries.len());
    let id = egui::Id::new(("ttpf_used_by_rows", doc_id));
    let kept: Option<(UsedByStamp, Arc<Vec<usize>>)> = ui.data_mut(|d| d.get_temp(id));
    if let Some((_, rows)) = kept.filter(|(kept, _)| *kept == stamp) {
        return Some(rows);
    }
    let keys = entries.iter().map(|entry| entry.key);
    let rows = Arc::new(crate::state::gta6_names::used_by_model(
        keys, &uses, &stamp.0,
    ));
    ui.data_mut(|d| d.insert_temp(id, (stamp, rows.clone())));
    Some(rows)
}
