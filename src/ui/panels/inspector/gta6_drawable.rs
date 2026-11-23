use super::*;

mod bone_edit;
mod bound_place_edit;
mod filter_edit;
mod head;
mod material_edit;
mod material_layout;
mod physics_edit;
mod selected;
mod vertex_edit;

/// Rows a list shows before it scrolls.
const ROWS_IN_VIEW: f32 = 16.0;

/// What a scanned GTA VI cell states that the navigation mesh view has no place for: the selected
/// polygon's raw values first, then the cell's borders, links and footprints; whether `doc_id` is
/// such a cell.
pub(super) fn draw_gta6_cell(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    selection: Option<u64>,
) -> bool {
    use crate::state::gta6_navmesh::{
        cell_lines, cell_rows, join_lines, neighbours, polygon_lines, scanned_cell,
    };
    let Some(path) = state.document(doc_id).and_then(|d| d.path.clone()) else {
        return false;
    };
    let Some(cell) = scanned_cell(&path) else {
        return false;
    };
    ui.heading(tr!("GTA VI Navigation Cell"));
    ui.separator();
    grid(ui, "gta6_cell_grid", |ui| {
        for (label, value) in cell_rows(&cell) {
            row(ui, &label, &value);
        }
    });
    let polygon = selection.and_then(crate::ui::panels::selection_ids::nav_polygon_of);
    for line in polygon.map(|p| polygon_lines(&cell, p)).unwrap_or_default() {
        ui.label(line);
    }
    physics_edit::draw_cell(ui, state, doc_id, polygon);
    // Built again only when a neighbouring cell opens or closes.
    let near = neighbours(&path, &cell);
    let key: Vec<usize> = (near.iter())
        .map(|(_, _, n)| n.as_ref().map_or(0, |n| std::sync::Arc::as_ptr(n) as usize))
        .collect();
    let id = egui::Id::new(("gta6_cell_joins", doc_id));
    type Joins = (Vec<usize>, std::sync::Arc<Vec<String>>);
    let joins = match ui.data(|d| d.get_temp::<Joins>(id)) {
        Some((k, lines)) if k == key => lines,
        _ => {
            let lines = std::sync::Arc::new(join_lines(&cell, &near));
            ui.data_mut(|d| d.insert_temp(id, (key, lines.clone())));
            lines
        }
    };
    for line in joins.iter() {
        ui.add(egui::Label::new(line.as_str()).extend());
    }
    if near.iter().any(|(_, _, n)| n.is_none())
        && ui
            .button(tr!("Find the neighbouring cells in the scan"))
            .clicked()
    {
        state.find_neighbour_cells(doc_id);
    }
    let id = egui::Id::new(("gta6_cell_lines", doc_id));
    type Cached = (usize, std::sync::Arc<Vec<String>>);
    let stamp = std::sync::Arc::as_ptr(&cell) as usize;
    let lines = match ui.data(|d| d.get_temp::<Cached>(id)) {
        Some((s, lines)) if s == stamp => lines,
        _ => {
            let lines = std::sync::Arc::new(cell_lines(&cell));
            ui.data_mut(|d| d.insert_temp(id, (stamp, lines.clone())));
            lines
        }
    };
    let row_height = ui.spacing().interact_size.y;
    egui::ScrollArea::both()
        .id_salt(("gta6_cell_rows", doc_id))
        .max_height(row_height * ROWS_IN_VIEW)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, lines.len(), |ui, range| {
            for line in &lines[range] {
                ui.add(egui::Label::new(line.as_str()).extend());
            }
        });
    ui.separator();
    true
}

/// What GTA VI's headerless reader states of a scanned body, and a dictionary's drawables to choose from.
pub(super) fn draw_gta6_drawable(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    selection: Option<u64>,
) {
    use crate::state::gta6_resource::{
        body_lines, dictionary_drawables, drawable_lines_stamp, drawable_of, material_rows,
        Material,
    };
    use crate::ui::panels::selection_ids::dict_entry_id;
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc_id).map(|d| &d.payload)
    else {
        return;
    };
    let resource = std::sync::Arc::clone(resource);
    ui.heading(match &*resource {
        or_io::gta6::headerless::Resource::Bound(_) => tr!("GTA VI Physics Bound"),
        or_io::gta6::headerless::Resource::Cloth(_) => tr!("GTA VI Cloth"),
        _ => tr!("GTA VI Drawable"),
    });
    ui.separator();
    // ⚠ The inspector scrolls both ways, so a label has no width to wrap at but the one set here.
    ui.scope(|ui| {
        ui.set_max_width(ui.clip_rect().width());
        ui.label(tr!("What GTA VI's headerless reader reads from this body. A shader is named by its effect once a scan has opened the SGD5 container that holds it. A material is drawn with its base-colour texture: the one its DiffuseTex slot binds, else the first a texture pack states as sRGB, so a material that binds only data and normal maps takes no colour from them and is shaded by its normal map. A key resolves in an open texture pack or a dictionary the app has read; one that resolves in none is listed as unresolved."));
    });
    ui.add_space(crate::ui::theme::SPACING_TIGHT);
    selected::draw(ui, state, doc_id, selection);
    bone_edit::draw(ui, state, doc_id, selection);
    let row_height = ui.spacing().interact_size.y;

    if let or_io::gta6::headerless::Resource::Bound(b) = &*resource {
        use crate::state::gta6_bound::{bound_rows, child_rows, polygon_rows, selected_child};
        type Rows = std::sync::Arc<(String, Vec<(String, String)>)>;
        let id = egui::Id::new(("gta6_bound_grid", doc_id));
        let stamp = (std::sync::Arc::as_ptr(&resource) as usize, selection);
        let cached = ui.data(|d| d.get_temp::<((usize, Option<u64>), Rows)>(id));
        let shown = match cached.filter(|(s, _)| *s == stamp) {
            Some((_, shown)) => shown,
            None => {
                let shown: Rows = std::sync::Arc::new(match selected_child(b, selection) {
                    Some((index, child)) => {
                        let mut rows = polygon_rows(child, selection);
                        rows.extend(child_rows(child));
                        (tr!("Child {index}", index = index), rows)
                    }
                    None => (tr!("Bound").to_string(), bound_rows(b)),
                });
                ui.data_mut(|d| d.insert_temp(id, (stamp, shown.clone())));
                shown
            }
        };
        let (heading, rows) = &*shown;
        ui.strong(heading);
        grid(ui, "gta6_bound_grid", |ui| {
            for (label, value) in rows {
                row(ui, label, value);
            }
        });
        ui.separator();
    }

    let count = dictionary_drawables(&resource).count();
    if count > 0 {
        ui.strong(tr_n!(
            "{count} drawable: click it to draw it",
            "{count} drawables: click one to draw it",
            count
        ));
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .id_salt(("gta6_dictionary_drawables", doc_id))
            .max_height(row_height * ROWS_IN_VIEW)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, count, |ui, range| {
                let in_view = dictionary_drawables(&resource).skip(range.start);
                for (code, d) in in_view.take(range.len()) {
                    let id = dict_entry_id(code);
                    if ui
                        .selectable_label(selection == Some(id), &d.name)
                        .clicked()
                    {
                        chosen = Some(id);
                    }
                }
            });
        if let Some(id) = chosen {
            state.selection.select(doc_id, id);
        }
        ui.separator();
    }

    // Built again only when the body, the drawable chosen, or what names its keys changes.
    let stamp = (
        drawable_lines_stamp(state),
        std::sync::Arc::as_ptr(&resource) as usize,
        selection,
    );
    let id = egui::Id::new(("gta6_drawable_lines", doc_id));
    type Shown = (Vec<String>, Vec<Material>);
    type Cached = (u64, usize, Option<u64>, std::sync::Arc<Shown>);
    let cached = ui.data(|d| d.get_temp::<Cached>(id));
    let shown = match cached.filter(|(s, p, c, _)| (*s, *p, *c) == stamp) {
        Some((.., shown)) => shown,
        None => {
            let materials = drawable_of(&resource, selection)
                .map(|d| material_rows(state, doc_id, d))
                .unwrap_or_default();
            let shown = std::sync::Arc::new((body_lines(&resource, selection), materials));
            ui.data_mut(|d| d.insert_temp(id, (stamp.0, stamp.1, stamp.2, shown.clone())));
            shown
        }
    };
    let (lines, materials) = &*shown;
    // ⚠ Only the rows in view: a drawable may hold thousands of geometries.
    egui::ScrollArea::both()
        .id_salt(("gta6_drawable_rows", doc_id))
        .max_height(row_height * ROWS_IN_VIEW)
        .auto_shrink([false, true])
        .show_rows(ui, row_height, lines.len(), |ui, range| {
            for line in &lines[range] {
                ui.add(egui::Label::new(line.as_str()).extend());
            }
        });
    // Each material as GTA V's material inspector lists one: a name, then what it holds.
    for (heading, rows) in materials {
        ui.add_space(crate::ui::theme::SPACING_DEFAULT);
        ui.label(
            egui::RichText::new(heading)
                .strong()
                .color(theme::TEXT_PRIMARY),
        );
        for (name, detail, warn) in rows {
            ui.label(egui::RichText::new(name).strong());
            let colour = if *warn {
                theme::WARNING
            } else {
                theme::TEXT_SECONDARY
            };
            ui.label(egui::RichText::new(detail).small().color(colour));
        }
    }
    drop(resource);
    head::draw(ui, state, doc_id);
    material_edit::draw(ui, state, doc_id, selection);
    material_layout::draw(ui, state, doc_id, selection);
    physics_edit::draw(ui, state, doc_id, selection);
    vertex_edit::draw(ui, state, doc_id, selection);
    filter_edit::draw(ui, state, doc_id);
}
