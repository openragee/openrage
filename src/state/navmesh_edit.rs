//! Disabling and deleting a polygon of an open `.ynv`, each one step of its history.

use or_io::gta5::CNavMesh;

use super::{AppState, Change, DocPayload, DocumentId};

fn navmesh_of(payload: &mut DocPayload) -> Option<&mut CNavMesh> {
    match payload {
        DocPayload::Resource(or_io::Asset::NavMesh(mesh)) => Some(mesh),
        _ => None,
    }
}

impl AppState {
    fn navmesh_mut(&mut self, doc: DocumentId) -> Result<&mut CNavMesh, String> {
        if let Some(why) = self.document(doc).and_then(|d| d.read_only) {
            return Err(crate::i18n::tr(why).to_string());
        }
        let payload = self.document_mut(doc).map(|d| &mut d.payload);
        payload
            .and_then(navmesh_of)
            .ok_or_else(|| "not a .ynv document".to_string())
    }

    /// Counts the edit `before` was the mesh ahead of as a step of its own.
    fn record_navmesh_edit(&mut self, doc: DocumentId, label: String, before: CNavMesh) {
        let change = Change::part(label.clone(), before, navmesh_of);
        self.record(doc, label, change);
        // ⚠ Never folded into the next edit: polygon numbers change under a deletion.
        if let Some(document) = self.document_mut(doc) {
            document.history.seal();
        }
    }

    /// Cuts `polygon` off, as one step; says what it did and what still reaches it, or why it could not.
    pub fn disable_navmesh_polygon(
        &mut self,
        doc: DocumentId,
        polygon: usize,
    ) -> Result<String, String> {
        if self
            .document(doc)
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return Err(crate::i18n::tr(super::document::rpf9::entry::READ_ONLY).to_string());
        }
        if let Some(said) = self.disable_gta6_polygon(doc, polygon) {
            return said;
        }
        let mesh = self.navmesh_mut(doc)?;
        let before = mesh.clone();
        let cut = mesh.disable_polygon(polygon).map_err(|e| e.to_string())?;
        if cut == 0 {
            return Ok(format!(
                "Polygon {polygon} already has no edge into or out of it"
            ));
        }
        let left = mesh.references(polygon).map_err(|e| e.to_string())?;
        self.record_navmesh_edit(doc, format!("Disable polygon {polygon}"), before);
        let mut said = format!("Disabled polygon {polygon}: {cut} edges now lead nowhere");
        if !left.is_empty() {
            said.push_str(&format!(". Still reaching it: {left}"));
        }
        Ok(said)
    }

    /// Deletes `polygon` and renumbers what names the ones after it, as one step; a refusal names what it would break and changes nothing.
    pub fn delete_navmesh_polygon(
        &mut self,
        doc: DocumentId,
        polygon: usize,
    ) -> Result<String, String> {
        if self
            .document(doc)
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return Err(crate::i18n::tr(super::document::rpf9::entry::READ_ONLY).to_string());
        }
        if let Some(said) = self.delete_gta6_polygon(doc, polygon) {
            return said;
        }
        let mesh = self.navmesh_mut(doc)?;
        let before = mesh.clone();
        mesh.delete_polygon(polygon).map_err(|e| e.to_string())?;
        let left = mesh.polys_array.as_deref().map_or(0, |l| l.len());
        self.record_navmesh_edit(doc, format!("Delete polygon {polygon}"), before);
        Ok(format!("Deleted polygon {polygon}: {left} polygons remain"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use or_io::decl::{Element, File, Owned, Param, Plain, Raw};
    use or_io::gta5::navmesh::NO_NEIGHBOUR;
    use or_io::gta5::{AiSplitArray, CNavMeshCompressedVertex, SubArray, TAdjPoly, TNavMeshPoly};

    use super::super::the_files_view_in_the_centre::labels;
    use super::*;
    use crate::state::Document;

    const CELL: u32 = 7;

    fn split<T: Owned>(elements: Vec<T>) -> Option<Box<AiSplitArray<T>>>
    where
        Raw<T>: Plain + Element + Param,
    {
        let sub_arrays = vec![SubArray {
            elements,
            ..Default::default()
        }];
        Some(Box::new(AiSplitArray {
            sub_arrays,
            ..Default::default()
        }))
    }

    /// Four triangles in a strip, in `CELL`: corner 0 of polygon i leads to i + 1 and corner 1 to i - 1;
    /// when `next`, the table lists cell 8 and polygon 0's corner 2 leads into it.
    fn strip(next: bool) -> CNavMesh {
        let mut triangle = TNavMeshPoly::default();
        triangle.set_num_vertices(3).unwrap();
        let to = |slot, polygon| TAdjPoly::leading_to(slot, polygon).unwrap();
        let nowhere = to(1, NO_NEIGHBOUR);
        let mut edges = Vec::new();
        for i in 0..4usize {
            edges.push(if i < 3 { to(0, i + 1) } else { nowhere.clone() });
            edges.push(if i > 0 { to(0, i - 1) } else { nowhere.clone() });
            edges.push(if i == 0 && next {
                to(1, 5)
            } else {
                nowhere.clone()
            });
        }
        let mut mesh = CNavMesh {
            index_of_mesh: CELL,
            compressed_vertex_array: split(vec![CNavMeshCompressedVertex::default(); 3]),
            vertex_index_array: split((0..12).map(|i| i % 3).collect()),
            adjacent_polys_array: split(edges),
            polys_array: split(vec![triangle; 4]),
            ..Default::default()
        };
        mesh.adjacent_meshes.navmesh_indices[0] = CELL;
        mesh.adjacent_meshes.navmesh_indices[1] = if next { 8 } else { 0 };
        mesh
    }

    fn open(state: &mut AppState, mesh: CNavMesh) -> DocumentId {
        state.documents.clear();
        let payload = DocPayload::Resource(or_io::Asset::NavMesh(mesh));
        let id: DocumentId = 1;
        state
            .documents
            .push(Document::new(id, Some(PathBuf::from("cell.ynv")), payload));
        state.active_document = Some(id);
        id
    }

    fn mesh(state: &mut AppState, doc: DocumentId) -> &mut CNavMesh {
        let payload = &mut state.document_mut(doc).expect("open").payload;
        navmesh_of(payload).expect("a navmesh")
    }

    #[test]
    fn a_table_navigation_cell_refuses_edits_before_the_cached_gta6_route() {
        let _serial = crate::ui::theme::serial();
        let bytes = crate::state::opening_an_rpf9_archive::navmesh_cell();
        let mut cell = or_io::gta6::headerless::read_navmesh(&bytes).unwrap();
        cell.edges[0].mesh = cell.cell;
        cell.edges[0].polygon = 0;
        let view = crate::state::gta6_resource::cell_view(&cell).unwrap();
        let path = PathBuf::from("navigation.rpf/.rpf9-table/0/hash_cell.ynv");
        crate::state::gta6_navmesh::remember(&path, cell);
        let cached = crate::state::gta6_navmesh::scanned_cell(&path).unwrap();
        let mut state = AppState::new();
        state.documents.push(Document::new(
            1,
            Some(path.clone()),
            DocPayload::Resource(or_io::Asset::NavMesh(view)),
        ));
        let before = crate::state::gta6_navmesh_edit::open_edges(mesh(&mut state, 1));
        assert!(
            before[0],
            "the cached cell has an edge the old route would cut"
        );
        let reason = crate::i18n::tr(crate::state::document::rpf9::entry::READ_ONLY);
        assert_eq!(state.disable_navmesh_polygon(1, 0).unwrap_err(), reason);
        assert_eq!(state.delete_navmesh_polygon(1, 0).unwrap_err(), reason);
        assert_eq!(
            crate::state::gta6_navmesh_edit::open_edges(mesh(&mut state, 1)),
            before
        );
        let after = crate::state::gta6_navmesh::scanned_cell(&path).unwrap();
        assert!(std::sync::Arc::ptr_eq(&cached, &after));
        assert!(!state.document(1).unwrap().history.can_undo());
        assert!(!path.exists());
    }

    #[test]
    fn disabling_a_polygon_is_one_step_that_undoes_and_saves_disabled() {
        let mut state = AppState::new();
        let doc = open(&mut state, strip(false));
        let said = state.disable_navmesh_polygon(doc, 0).unwrap();
        assert_eq!(said, "Disabled polygon 0: 2 edges now lead nowhere");
        assert_eq!(
            mesh(&mut state, doc).disabled_polygons(),
            [true, false, false, false]
        );

        let saved = or_io::Asset::NavMesh(mesh(&mut state, doc).clone())
            .to_bytes()
            .expect("the edited mesh saves");
        let reopened = CNavMesh::from_bytes(&saved).expect("and opens");
        assert_eq!(reopened.disabled_polygons(), [true, false, false, false]);

        let history = &state.document(doc).unwrap().history;
        assert_eq!(history.undo_label(), Some("Disable polygon 0"));
        assert_eq!(state.undo().as_deref(), Some("Disable polygon 0"));
        assert_eq!(mesh(&mut state, doc).disabled_polygons(), [false; 4]);
        assert_eq!(state.redo().as_deref(), Some("Disable polygon 0"));
        assert_eq!(
            mesh(&mut state, doc).disabled_polygons(),
            [true, false, false, false]
        );
    }

    #[test]
    fn a_polygon_already_cut_off_is_said_so_and_makes_no_step() {
        let mut state = AppState::new();
        let doc = open(&mut state, strip(false));
        state.disable_navmesh_polygon(doc, 0).unwrap();
        let said = state.disable_navmesh_polygon(doc, 0).unwrap();
        assert!(said.contains("already"), "{said}");
        assert!(state.undo().is_some() && state.undo().is_none());
    }

    #[test]
    fn a_disable_names_what_still_reaches_the_polygon() {
        let mut state = AppState::new();
        let doc = open(&mut state, strip(true));
        let said = state.disable_navmesh_polygon(doc, 0).unwrap();
        assert!(
            said.contains("Still reaching it") && said.contains("cell 8"),
            "{said}"
        );
    }

    #[test]
    fn a_refused_deletion_names_what_it_would_break_and_leaves_no_step() {
        let mut state = AppState::new();
        let doc = open(&mut state, strip(true));
        let refused = state.delete_navmesh_polygon(doc, 1).unwrap_err();
        assert!(
            refused.contains("polygon 0 corner 0") && refused.contains("cell 8"),
            "{refused}"
        );
        assert!(!state.document(doc).unwrap().history.can_undo());
        assert_eq!(mesh(&mut state, doc).disabled_polygons(), [false; 4]);
    }

    #[test]
    fn a_deletion_that_breaks_nothing_is_one_step_that_undoes() {
        let mut state = AppState::new();
        let doc = open(&mut state, strip(false));
        state.disable_navmesh_polygon(doc, 1).unwrap();
        let said = state.delete_navmesh_polygon(doc, 1).unwrap();
        assert_eq!(said, "Deleted polygon 1: 3 polygons remain");
        let polygons =
            |state: &mut AppState| mesh(state, doc).polys_array.as_deref().unwrap().len();
        assert_eq!(polygons(&mut state), 3);
        assert_eq!(state.undo().as_deref(), Some("Delete polygon 1"));
        assert_eq!(polygons(&mut state), 4);
    }

    /// One frame of the outline `doc` shows, with `events` arriving first.
    fn outline(
        ctx: &egui::Context,
        state: &mut AppState,
        doc: DocumentId,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Pos2)> {
        let payload = &state.document(doc).expect("open").payload;
        let draw = super::super::capabilities::for_payload(payload)
            .and_then(|row| row.outline)
            .expect("a navmesh has an outline");
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        labels(&ctx.run_ui(input, |ui| draw(ui, state, doc)))
    }

    fn click(
        ctx: &egui::Context,
        state: &mut AppState,
        doc: DocumentId,
        at: egui::Pos2,
        button: egui::PointerButton,
    ) -> Vec<(String, egui::Pos2)> {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let moved = vec![egui::Event::PointerMoved(at), press(true)];
        outline(ctx, state, doc, moved);
        outline(ctx, state, doc, vec![press(false)]);
        outline(ctx, state, doc, Vec::new())
    }

    fn at(shown: &[(String, egui::Pos2)], label: &str) -> egui::Pos2 {
        let found = shown.iter().find(|(text, _)| text == label);
        (found.map(|(_, at)| *at)).unwrap_or_else(|| panic!("no `{label}` in {shown:?}"))
    }

    /// The menu on a polygon row reaches the state: the real outline, a right click and a click on its item.
    #[test]
    fn the_outline_disables_and_refuses_through_a_polygon_row_menu() {
        let mut state = AppState::new();
        let doc = open(&mut state, strip(true));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        // ⚠ The pulse opens every header, so the polygon rows are drawn without a click on each arrow.
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("hierarchy_pulse_force_open"), true));
        outline(&ctx, &mut state, doc, Vec::new());
        let shown = outline(&ctx, &mut state, doc, Vec::new());
        assert!(shown.iter().any(|(t, _)| t == "Polygons (4)"), "{shown:?}");

        let (secondary, primary) = (egui::PointerButton::Secondary, egui::PointerButton::Primary);
        let menu = click(&ctx, &mut state, doc, at(&shown, "Polygon 1"), secondary);
        let delete = at(&menu, "Delete polygon");
        click(&ctx, &mut state, doc, delete, primary);
        assert!(
            state.toasts.iter().any(|t| t.message.contains("cell 8")),
            "the refusal did not reach a toast"
        );
        assert!(!state.document(doc).unwrap().history.can_undo());

        let shown = outline(&ctx, &mut state, doc, Vec::new());
        let menu = click(&ctx, &mut state, doc, at(&shown, "Polygon 1"), secondary);
        let disable = at(&menu, "Disable polygon");
        let shown = click(&ctx, &mut state, doc, disable, primary);
        assert_eq!(
            mesh(&mut state, doc).disabled_polygons(),
            [false, true, false, false]
        );
        assert!(
            shown.iter().any(|(t, _)| t.ends_with("disabled")),
            "the row does not say so: {shown:?}"
        );
    }
}
