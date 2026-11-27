//! A height map's cell, picked by column and row, its height set as the byte the file stores.

use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::theme;

pub(super) fn draw(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::HeightMap(map)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let (Some(last_col), Some(last_row)) =
        (map.num_cols.checked_sub(1), map.num_rows.checked_sub(1))
    else {
        return;
    };
    let id = egui::Id::new(("height_map_cell", doc_id));
    let [col, row] = ui.data(|d| d.get_temp::<[u16; 2]>(id)).unwrap_or_default();
    let mut pick = [col.min(last_col), row.min(last_row)];
    let at = usize::from(pick[1]) * usize::from(map.num_cols) + usize::from(pick[0]);
    let (Some(&max), Some(&min)) = (map.max.get(at), map.min.get(at)) else {
        return;
    };
    let mut height = max;
    let mut changed = false;
    ui.add_space(theme::SPACING_SNUG);
    ui.strong(tr!("Cell"));
    ui.horizontal(|ui| {
        ui.label(tr!("Column"));
        ui.add(egui::DragValue::new(&mut pick[0]).range(0..=last_col));
        ui.label(tr!("Row"));
        ui.add(egui::DragValue::new(&mut pick[1]).range(0..=last_row));
    });
    ui.horizontal(|ui| {
        ui.label(tr!("Height"));
        changed = (ui.add(egui::DragValue::new(&mut height).range(0..=u8::MAX)))
            .on_hover_text(tr!(
                "The byte the file stores for the cell's highest point, 0 to 255; its lowest follows where it was the same and never stays above it."
            ))
            .changed();
        ui.label(tr!("lowest {min}", min = min));
    });
    ui.data_mut(|d| d.insert_temp(id, pick));
    if changed {
        if let Err(why) = state.set_height_map_cell(doc_id, pick[0], pick[1], height) {
            state.show_error_toast(why);
        }
    }
}
