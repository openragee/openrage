use super::*;

/// Rows the list shows before it scrolls.
const ROWS_IN_VIEW: f32 = 16.0;

/// What the GTA VI list `doc_id` opened from holds beyond its GTA V form, for no other document.
pub(super) fn draw_scanned(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(path) = state.document(doc_id).and_then(|d| d.path.as_deref()) else {
        return;
    };
    let Some(lines) = crate::state::gta6_particles::scanned_lines(path) else {
        return;
    };
    let table_entry = state
        .document(doc_id)
        .is_some_and(crate::state::Document::is_from_rpf9_table);
    let names_drawables = crate::state::gta6_particle_models::names_drawables(path);
    ui.separator();
    egui::CollapsingHeader::new(tr!("What the GTA VI list holds beyond this GTA V form"))
        .id_salt(("gta6_particles", doc_id))
        .default_open(true)
        .show(ui, |ui| {
            let row_height = ui.spacing().interact_size.y;
            egui::ScrollArea::both()
                .id_salt(("gta6_particle_lines", doc_id))
                .max_height(row_height * ROWS_IN_VIEW)
                .auto_shrink([false, true])
                .show_rows(ui, row_height, lines.len(), |ui, range| {
                    for line in &lines[range] {
                        ui.add(egui::Label::new(line.as_str()).extend());
                    }
                });
        });
    let find = tr!("Find the drawables its model rules and mesh domains name in the open scans");
    if !table_entry && names_drawables && ui.button(find).clicked() {
        if let Err(why) = state.find_gta6_particle_models(doc_id) {
            state.show_error_toast(why);
        }
    }
}
