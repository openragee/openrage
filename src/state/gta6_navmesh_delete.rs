//! A scanned GTA VI cell's polygon deleted, or cut off where a side record naming it drops a
//! corner, one step of undo: the view's model is drawn again from the cell the inspector reads,
//! each record's flags and each edge cut kept, the open neighbouring bodies' special links made
//! again, and a save makes the step on the body read again, after every earlier one and before the
//! flags and the cuts are written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::Rpf9Sections;
use or_io::gta5::navmesh::NO_NEIGHBOUR;
use or_io::gta5::{CNavMesh, TAdjPoly, TNavMeshPoly};
use or_io::gta6::headerless::{
    delete_polygon, deletion, disable_polygon, disabling, read_navmesh, NavMesh, NavRebuild,
    NOWHERE,
};

use super::edit_history::{Change, Swap};
use super::{AppState, DocPayload, DocumentId};

#[path = "gta6_navmesh_links.rs"]
pub(super) mod links;

#[path = "gta6_navmesh_look.rs"]
mod look;

/// A deletion, or a disable that moves the arrays, as a save makes it again: the polygon, and the
/// records the cell held before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Deletion {
    polygon: usize,
    records: usize,
    disable: bool,
}

// ponytail: one entry per scanned cell a polygon was deleted from, never emptied, as the cells are.
static DELETIONS: Mutex<BTreeMap<PathBuf, Vec<Deletion>>> = Mutex::new(BTreeMap::new());

fn deletions() -> MutexGuard<'static, BTreeMap<PathBuf, Vec<Deletion>>> {
    DELETIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Drop the deletions kept for `path`, whose body opens again as it is stored.
pub(super) fn forget(path: &Path) {
    deletions().remove(path);
    links::forget(path);
}

fn view_of(payload: &mut DocPayload) -> Option<&mut CNavMesh> {
    match payload {
        DocPayload::Resource(or_io::Asset::NavMesh(mesh)) => Some(mesh),
        _ => None,
    }
}

/// The cell the inspector reads and the deletions a save makes, which an undo puts back.
struct CellPart {
    path: PathBuf,
    cell: Arc<NavMesh>,
    deletions: Vec<Deletion>,
}

impl Swap for CellPart {
    fn swap(&mut self, _: &mut DocPayload) -> bool {
        let Some(now) = super::gta6_navmesh::replace(&self.path, Arc::clone(&self.cell)) else {
            return false;
        };
        self.cell = now;
        let mut made = deletions();
        std::mem::swap(
            made.entry(self.path.clone()).or_default(),
            &mut self.deletions,
        );
        true
    }
}

/// Why polygon `polygon` waits: the archive's table is searched for rows of the neighbouring `cells`.
fn looking(polygon: usize, cells: &str) -> String {
    tr!(
        "Polygon {polygon} is not changed while the bodies of the neighbouring cells {cells} are not open and the archive's table is searched for them: a cell the table holds no navigation row of has no special link into this one. Try again once the search ends.",
        polygon = polygon,
        cells = cells
    )
}

/// The open bodies of the cells across `cell`'s borders and at its corners, which a deletion checks
/// for links into it, or why it needs more of them: the `closed` cells, which may hold a body.
fn others(
    closed: &[String],
    cell: &NavMesh,
    near: &[(u32, Option<Arc<NavMesh>>)],
    polygon: usize,
) -> Result<Vec<Arc<NavMesh>>, String> {
    if !closed.is_empty() {
        return Err(tr!(
            "Polygon {polygon} is not deleted while the bodies of the neighbouring cells {cells} are not open: their special links may number this cell's polygons. Find the neighbouring cells in the scan first.",
            polygon = polygon,
            cells = closed.join(", ")
        ));
    }
    let across: Vec<u32> = near.iter().map(|(c, _)| *c).collect();
    for (i, l) in cell.special_links.iter().enumerate() {
        for mesh in [l.from_mesh, l.to_mesh] {
            if mesh != cell.cell && mesh != NOWHERE && !across.contains(&mesh) {
                return Err(tr!(
                    "Polygon {polygon} is not deleted: special link {link} ends in cell {cell}, which is not across a border of this one, so no search opens its body.",
                    polygon = polygon,
                    link = i,
                    cell = mesh
                ));
            }
        }
    }
    Ok(near.iter().filter_map(|(_, body)| body.clone()).collect())
}

impl AppState {
    /// Polygon `polygon` of the scanned GTA VI cell `doc` deleted as one step, or why not; `None`
    /// for any other document.
    pub(super) fn delete_gta6_polygon(
        &mut self,
        doc: DocumentId,
        polygon: usize,
    ) -> Option<Result<String, String>> {
        let path = self.document(doc)?.path.clone()?;
        let cell = super::gta6_navmesh::scanned_cell(&path)?;
        Some(self.delete_in_cell(doc, &path, &cell, polygon))
    }

    fn delete_in_cell(
        &mut self,
        doc: DocumentId,
        path: &Path,
        cell: &NavMesh,
        polygon: usize,
    ) -> Result<String, String> {
        let near = super::gta6_navmesh::linked_cells(path, cell);
        let closed = (self.unopened(path, &near)).map_err(|cells| looking(polygon, &cells))?;
        let open = others(&closed, cell, &near, polygon)?;
        let bodies: Vec<&NavMesh> = open.iter().map(|b| &**b).collect();
        let made = deletion(cell, polygon, &bodies).map_err(|e| e.to_string())?;
        let left = made.mesh.polygons.len();
        let gone = (cell.special_links.len()).saturating_sub(made.mesh.special_links.len());
        let step = Deletion {
            polygon,
            records: cell.polygons.len(),
            disable: false,
        };
        let renumbered = self.make_in_cell(doc, path, cell, made, step)?;
        let mut said = tr!(
            "Deleted polygon {polygon}. Polygons left: {count}.",
            polygon = polygon,
            count = left
        );
        if gone > 0 {
            said.push(' ');
            said.push_str(&tr_n!(
                "{count} special link of this cell ended in a record that went, and was deleted.",
                "{count} special links of this cell ended in a record that went, and were deleted.",
                gone
            ));
        }
        if renumbered > 0 {
            said.push(' ');
            said.push_str(&tr_n!(
                "The special links of {count} neighbouring cell were renumbered: a save of this cell writes its body beside it.",
                "The special links of {count} neighbouring cells were renumbered: a save of this cell writes their bodies beside it.",
                renumbered
            ));
        }
        Ok(said)
    }

    /// Polygon `polygon` of the scanned cell `doc` cut off as one step where a side record naming
    /// it drops a corner, and how many edges now lead nowhere; `None` where the cut moves nothing.
    pub(super) fn disable_rechained(
        &mut self,
        doc: DocumentId,
        path: &Path,
        cell: &NavMesh,
        polygon: usize,
        near: &[(u32, Option<Arc<NavMesh>>)],
    ) -> Option<Result<usize, String>> {
        // A body of no cell has no side record to chain again.
        if cell.cell == 0 {
            return None;
        }
        let open: Vec<Arc<NavMesh>> = near.iter().filter_map(|(_, b)| b.clone()).collect();
        let bodies: Vec<&NavMesh> = open.iter().map(|b| &**b).collect();
        let made = match disabling(cell, polygon, &bodies) {
            Ok(made) if made.mesh.indices.len() == cell.indices.len() => return None,
            Ok(made) => made,
            Err(e) => return Some(Err(e.to_string())),
        };
        if made.records.len() < cell.polygons.len() {
            let closed = match self.unopened(path, near) {
                Ok(closed) => closed,
                Err(cells) => return Some(Err(looking(polygon, &cells))),
            };
            if !closed.is_empty() {
                return Some(Err(tr!(
                    "Polygon {polygon} is not disabled while the bodies of the neighbouring cells {cells} are not open: a side record it empties goes, and their special links may number the records after it. Find the neighbouring cells in the scan first.",
                    polygon = polygon,
                    cells = closed.join(", ")
                )
                .to_string()));
            }
            if let Err(why) = others(&closed, cell, near, polygon) {
                return Some(Err(why));
            }
        }
        let reaching = |m: &NavMesh| m.edges.iter().filter(|e| e.mesh == m.cell).count();
        let cuts = reaching(cell).saturating_sub(reaching(&made.mesh));
        let step = Deletion {
            polygon,
            records: cell.polygons.len(),
            disable: true,
        };
        let made = self.make_in_cell(doc, path, cell, made, step);
        Some(made.map(|_| cuts))
    }

    /// `made`, made from `cell`, the cell and the view of `doc` hold, with the open neighbouring
    /// bodies' special links made again, as one step of undo that a save makes again; how many
    /// neighbouring bodies it renumbered.
    fn make_in_cell(
        &mut self,
        doc: DocumentId,
        path: &Path,
        cell: &NavMesh,
        made: NavRebuild,
        step: Deletion,
    ) -> Result<usize, String> {
        let closed = || tr!("its document is closed").to_string();
        // ⚠ Each body is written by one save: this one's links are another cell's to write.
        if links::renumbered_by(path).is_some() {
            return Err(tr!("This cell's special links were renumbered by an edit of a neighbouring cell, whose save writes them: save that cell or undo its edit first.").to_string());
        }
        let before = match self.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Resource(or_io::Asset::NavMesh(view))) => view.clone(),
            Some(_) => return Err(tr!("The cell's view holds no edges.").to_string()),
            None => return Err(closed()),
        };
        let edited = view_after(&before, &made)?;
        let relinks = links::planned(path, cell, &made)?;
        let own = |p: &Path| deletions().get(p).is_some_and(|d| !d.is_empty());
        if let Some((_, body)) = relinks.iter().find(|(p, _)| own(p.as_path())) {
            return Err(tr!(
                "The special links of the neighbouring cell {cell} would be renumbered, but its body holds deletions of its own, which its own save writes: undo them first.",
                cell = body.cell
            ));
        }
        let mut planned = deletions().get(path).cloned().unwrap_or_default();
        planned.push(step);
        tried(path, planned)?;
        let held = super::gta6_navmesh::replace(path, Arc::new(made.mesh)).ok_or_else(closed)?;
        if let Some(view) = self.document_mut(doc).and_then(|d| view_of(&mut d.payload)) {
            *view = edited;
        }
        let earlier = {
            let mut made = deletions();
            let list = made.entry(path.to_path_buf()).or_default();
            let earlier = list.clone();
            list.push(step);
            earlier
        };
        let (kind, label) = match step.disable {
            true => (
                "gta6 disable polygon",
                tr!("Disable polygon {polygon}", polygon = step.polygon),
            ),
            false => (
                "gta6 delete polygon",
                tr!("Delete polygon {polygon}", polygon = step.polygon),
            ),
        };
        let bodies = links::apply(path, relinks);
        let renumbered = bodies.len();
        let mut change = Change::part(kind, before, view_of);
        if let Change::Part { parts, .. } = &mut change {
            parts.push(Box::new(CellPart {
                path: path.to_path_buf(),
                cell: held,
                deletions: earlier,
            }));
            parts.extend(bodies);
        }
        self.record(doc, label, change);
        // ⚠ Never folded into the next edit: polygon numbers change under a deletion.
        if let Some(document) = self.document_mut(doc) {
            document.history.seal();
        }
        Ok(renumbered)
    }
}

/// The view of `made`'s cell, each record's flags and each edge a disable cut there kept from
/// `before`, the view of the cell it was made from.
fn view_after(before: &CNavMesh, made: &NavRebuild) -> Result<CNavMesh, String> {
    let no_view = || tr!("The cell's view holds no edges.").to_string();
    let mut view = super::gta6_resource::cell_view(&made.mesh).ok_or_else(no_view)?;
    let flags: Vec<u32> = (before.polys_array.as_deref()).map_or(Vec::new(), |l| {
        l.iter().map(TNavMeshPoly::as_integer1).collect()
    });
    let open = super::gta6_navmesh_edit::open_edges(before);
    let nowhere = TAdjPoly::leading_to(0, NO_NEIGHBOUR).map_err(|e| e.to_string())?;
    if let Some(polygons) = view.polys_array.as_deref_mut() {
        let all = (polygons.sub_arrays.iter_mut()).flat_map(|s| s.elements.iter_mut());
        for (p, from) in all.zip(&made.records) {
            if let Some(f) = flags.get(*from) {
                p.set_as_integer1(*f);
            }
        }
    }
    if let Some(edges) = view.adjacent_polys_array.as_deref_mut() {
        let all = (edges.sub_arrays.iter_mut()).flat_map(|s| s.elements.iter_mut());
        for (e, from) in all.zip(&made.edges) {
            if open.get(*from) == Some(&false) {
                *e = nowhere.clone();
            }
        }
    }
    Ok(view)
}

/// The deletions a save of the cell at `path` makes, and no neighbouring body: their links were
/// made again at each step and are written beside the cell.
pub(super) fn saved_with(path: &Path, _cell: &NavMesh) -> (Vec<Deletion>, Vec<Arc<NavMesh>>) {
    (
        deletions().get(path).cloned().unwrap_or_default(),
        Vec::new(),
    )
}

/// `made` made on the body at `path` read again, as a save makes them, so a deletion only its bytes
/// refuse is refused when it is made rather than at every save after it.
// ponytail: the body is read on the UI thread, as the edit's other checks run; a job if cells grow.
fn tried(path: &Path, made: Vec<Deletion>) -> Result<(), String> {
    let Some((archive, offset)) = super::rpf9_scan::scanned_body(path) else {
        return Ok(());
    };
    let edit = move |_, s: &mut Rpf9Sections| write_deletions(s, &made, &[]);
    super::gta6_edit::written(archive, offset, &edit).map(drop)
}

/// `made` over the cell `sections` hold, in order, each where the cell holds the records it began
/// from: a body saved with a deletion holds fewer, and it is not made twice.
pub(super) fn write_deletions(
    sections: &mut Rpf9Sections,
    made: &[Deletion],
    others: &[Arc<NavMesh>],
) -> Result<(), String> {
    let bodies: Vec<&NavMesh> = others.iter().map(|b| &**b).collect();
    for d in made {
        let read = read_navmesh(&sections.virtual_section).map_err(|e| e.to_string())?;
        let records = read.polygons.len();
        if records < d.records {
            continue;
        }
        if records > d.records {
            return Err(tr!(
                "The cell read again holds {found} polygon records, more than the {records} its deletion of polygon {polygon} began from.",
                found = records,
                records = d.records,
                polygon = d.polygon
            ));
        }
        // A disable that empties no side record leaves the records it began from, and is made
        // again on a body saved with it to no change.
        match d.disable {
            true => disable_polygon(sections, d.polygon, &bodies),
            false => delete_polygon(sections, d.polygon, &bodies),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use or_io::container::rpf::Rpf9Form;
    use or_io::gta6::headerless::{Border, NavDeleteError, SpecialLink};

    use super::*;
    use crate::state::opening_an_rpf9_archive::{
        navmesh_cell, row_at, rows, rpf9_head, scanned, settle, written, zlib,
    };

    fn opened(dir: &Path, body: &[u8]) -> (AppState, DocumentId) {
        let mut bytes = rpf9_head(1, 0x1A0);
        bytes.resize(0x1010, 0);
        bytes.extend(zlib(body));
        let mut state = AppState::new();
        let archive = scanned(&mut state, written(dir, &bytes));
        state.open_scanned_body(archive, row_at(&rows(&state, archive), 0x1010));
        settle(&mut state);
        let doc = (state.active_document)
            .filter(|&d| d != archive)
            .unwrap_or_else(|| panic!("the cell: {}", state.error_dialog_message));
        (state, doc)
    }

    fn cell() -> NavMesh {
        read_navmesh(&navmesh_cell()).expect("the cell reads")
    }

    fn archive_of(state: &AppState, doc: DocumentId) -> PathBuf {
        let path = state.document(doc).and_then(|d| d.path.clone());
        let path = path.expect("a path");
        path.parent().expect("its archive").to_path_buf()
    }

    #[test]
    fn a_polygon_whose_neighbours_bodies_are_not_open_waits_for_the_table_and_makes_no_step() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (mut state, doc) = opened(dir.path(), &navmesh_cell());
        let refused = state.delete_navmesh_polygon(doc, 0).unwrap_err();
        assert!(refused.contains("table is searched for them"), "{refused}");
        look::settled(&archive_of(&state, doc));
        // With no key the table does not open, so every neighbour may hold a body.
        let refused = state.delete_navmesh_polygon(doc, 0).unwrap_err();
        assert!(refused.contains("are not open"), "{refused}");
        // A table that holds no neighbour leaves only the cell's own rules to refuse it.
        let around = Border::around(cell().cell);
        look::given_none_of(&archive_of(&state, doc), &around);
        let refused = state.delete_navmesh_polygon(doc, 0).unwrap_err();
        assert!(refused.contains("last chunk"), "{refused}");
        assert!(!state.document(doc).expect("open").history.can_undo());
    }

    /// The virtual section of a cell at [`navmesh_cell`]'s grid place of two water triangles over
    /// 128 m whose edges lead nowhere, so either goes alone.
    fn two_triangles() -> Vec<u8> {
        let mut v = vec![0u8; 0x3B0];
        let put = |v: &mut Vec<u8>, at: usize, bytes: &[u8]| {
            v[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let ptr = |v: &mut Vec<u8>, at: usize, to: usize| {
            put(v, at, &(0x5000_0000 + to as u64).to_le_bytes())
        };
        // A split array at `at` of one chunk: header, chunk row, start, then the items.
        let list = |v: &mut Vec<u8>, at: usize, items: &[u8], stride: usize| {
            let count = (items.len() / stride) as u64;
            put(v, at + 8, &count.to_le_bytes());
            ptr(v, at + 0x10, at + 0x30);
            ptr(v, at + 0x18, at + 0x40);
            put(v, at + 0x20, &1u64.to_le_bytes());
            ptr(v, at + 0x30, at + 0x50);
            put(v, at + 0x38, &count.to_le_bytes());
            put(v, at + 0x50, items);
        };
        ptr(&mut v, 0x08, 0xF0);
        v[0xF8] = 1;
        put(&mut v, 0x10, &9u32.to_le_bytes());
        put(&mut v, 0x14, &0x0001_0015u32.to_le_bytes());
        for at in [0x20, 0x34, 0x48] {
            put(&mut v, at, &1.0f32.to_le_bytes());
        }
        let vertices: Vec<u8> = [0u16, 0, 0, 65535, 0, 0, 0, 65535, 0, 65535, 65535, 0]
            .iter()
            .flat_map(|q| q.to_le_bytes())
            .collect();
        list(&mut v, 0x100, &vertices, 6);
        list(&mut v, 0x170, &[0, 0, 1, 0, 2, 0, 1, 0, 3, 0, 2, 0], 2);
        let edges = [[0xFF; 4], [0; 4], [0; 4]].repeat(6).concat();
        list(&mut v, 0x1D0, &edges, 12);
        // The second triangle's corners start at index 3.
        let mut polygons = [0u8; 88];
        (polygons[4], polygons[8]) = (3, 0x80);
        (polygons[44 + 4], polygons[44 + 8]) = (0x33, 0x80);
        list(&mut v, 0x270, &polygons, 44);
        for (slot, at) in [
            (0x70, 0x100),
            (0x80, 0x170),
            (0x88, 0x1D0),
            (0x98, 0x270),
            (0xA0, 0x320),
        ] {
            ptr(&mut v, slot, at);
        }
        put(&mut v, 0x90, &6u32.to_le_bytes());
        for (at, x) in [(0x330, 128.0f32), (0x334, 128.0), (0x338, 4.0)] {
            put(&mut v, at, &x.to_le_bytes());
        }
        // The quadtree's one leaf names both triangles.
        ptr(&mut v, 0x320 + 0x2C, 0x380);
        ptr(&mut v, 0x380 + 0x08, 0x3A0);
        v[0x380 + 0x18] = 2;
        put(&mut v, 0x3A0, &[0, 0, 1, 0]);
        let cell = read_navmesh(&navmesh_cell()).expect("the cell reads").cell;
        for (at, word) in [
            (0xC0, 4),
            (0xC4, 2),
            (0xC8, cell),
            (0xCC, 6 * 4 + 14 * 6 + 44 * 2),
        ] {
            put(&mut v, at, &word.to_le_bytes());
        }
        v
    }

    fn finished(state: &mut AppState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.extract_jobs.is_running() {
            state.poll_extract_job();
            assert!(std::time::Instant::now() < deadline, "the save never ended");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_deletion_at_a_cell_whose_neighbours_the_table_holds_no_row_of_saves_and_reads_back() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (mut state, doc) = opened(dir.path(), &two_triangles());
        let refused = state.delete_navmesh_polygon(doc, 0).unwrap_err();
        assert!(refused.contains("table is searched for them"), "{refused}");
        assert!(!state.document(doc).expect("open").history.can_undo());
        look::settled(&archive_of(&state, doc));
        let around = Border::around(cell().cell);
        look::given_none_of(&archive_of(&state, doc), &around);

        let said = state.delete_navmesh_polygon(doc, 0).expect("deleted");
        assert!(said.contains("Polygons left: 1."), "{said}");
        let saved = dir.path().join("saved.rsc");
        assert!(state.save_active_document_as(saved.clone()));
        finished(&mut state);
        let bytes = std::fs::read(&saved).unwrap_or_else(|e| panic!("{}: {e}", saved.display()));
        let (find, read) =
            or_io::container::rpf::read_rpf9_body(std::io::Cursor::new(bytes), 0, u64::MAX)
                .expect("it reads");
        let s = Rpf9Sections::of(&find, &read).expect("a resource");
        let m = read_navmesh(&s.virtual_section).expect("the saved cell reads");
        assert_eq!(
            (m.polygons.len(), m.vertices.len(), m.indices),
            (1, 3, vec![0, 2, 1])
        );
    }

    #[test]
    fn a_link_into_a_corner_cell_is_taken_one_farther_is_named_and_the_body_s_own_rules_hold() {
        let mut m = cell();
        let near: Vec<(u32, Option<Arc<NavMesh>>)> = (Border::around(m.cell).into_iter())
            .map(|c| (c, Some(Arc::new(cell()))))
            .collect();
        assert_eq!(near.len(), 8, "four borders and four corners");
        let own = m.cell;
        let link = |to_mesh| SpecialLink {
            from: [0; 3],
            to: [0; 3],
            from_polygon: 0,
            to_polygon: 0,
            from_mesh: own,
            to_mesh,
            word_18: 0,
            heading: 0,
        };
        let diagonal = m.cell + 512 + 1;
        let farther = m.cell + 2;
        m.special_links.push(link(diagonal));
        let open = others(&[], &m, &near, 0).expect("the corner cell's body is open");
        assert_eq!(open.len(), near.len());
        let closed = near[4].0.to_string();
        let refused = others(std::slice::from_ref(&closed), &m, &near, 0).unwrap_err();
        assert!(refused.contains(&closed), "{refused}");
        m.special_links.push(link(farther));
        let refused = others(&[], &m, &near, 0).unwrap_err();
        assert!(refused.contains(&farther.to_string()), "{refused}");
        m.special_links.clear();
        let open = others(&[], &m, &near, 0).expect("every border's body is open");
        assert_eq!(open.len(), near.len());
        // The cell's one polygon takes its vertices with it, and a split array keeps its chunk.
        let bodies: Vec<&NavMesh> = open.iter().map(|b| &**b).collect();
        let refused = deletion(&m, 0, &bodies).unwrap_err();
        assert!(
            matches!(refused, NavDeleteError::ChunkEmpties { .. }),
            "{refused}"
        );
    }

    /// The built cell with its first edge leading back into its one polygon.
    fn linked() -> NavMesh {
        let mut m = cell();
        m.edges[0].mesh = m.cell;
        m.edges[0].polygon = 0;
        m
    }

    #[test]
    fn a_view_drawn_again_from_the_cell_keeps_each_record_s_flags_and_each_edge_cut_in_it() {
        let made = || NavRebuild {
            mesh: linked(),
            records: vec![0],
            edges: vec![0, 1, 2],
        };
        let mut before = crate::state::gta6_resource::cell_view(&linked()).expect("a view");
        let polygons = before.polys_array.as_deref_mut().expect("polygons");
        polygons.sub_arrays[0].elements[0].set_as_integer1(0x81);
        let view = view_after(&before, &made()).expect("a view");
        let flags = |v: &CNavMesh| -> Vec<u32> {
            let l = v.polys_array.as_deref().expect("polygons");
            l.iter().map(TNavMeshPoly::as_integer1).collect()
        };
        assert_eq!(flags(&view), [0x81]);
        let open = super::super::gta6_navmesh_edit::open_edges;
        assert!(open(&view)[0], "edge 0 leads into the polygon");
        let edges = before.adjacent_polys_array.as_deref_mut().expect("edges");
        edges.sub_arrays[0].elements[0] = TAdjPoly::leading_to(0, NO_NEIGHBOUR).expect("an edge");
        let view = view_after(&before, &made()).expect("a view");
        assert!(!open(&view)[0], "the cut the view made is kept");
    }

    #[test]
    fn a_cell_part_puts_the_cell_and_the_deletions_back_and_forth() {
        let path = PathBuf::from("a_cell_part_puts_the_cell_back/0x1010.cell");
        let mut after = cell();
        after.flags = 11;
        super::super::gta6_navmesh::remember(&path, after);
        let made = vec![Deletion {
            polygon: 0,
            records: 1,
            disable: false,
        }];
        deletions().insert(path.clone(), made.clone());
        let mut part = CellPart {
            path: path.clone(),
            cell: Arc::new(cell()),
            deletions: Vec::new(),
        };
        let mut payload = DocPayload::Resource(or_io::Asset::NavMesh(CNavMesh::default()));
        let flags = |p: &Path| super::super::gta6_navmesh::scanned_cell(p).map(|c| c.flags);
        let held = |p: &Path| deletions().get(p).cloned();
        assert!(part.swap(&mut payload));
        assert_eq!((flags(&path), held(&path)), (Some(9), Some(Vec::new())));
        assert!(part.swap(&mut payload));
        assert_eq!((flags(&path), held(&path)), (Some(11), Some(made)));

        let mut nowhere = CellPart {
            path: PathBuf::from("no cell is kept here"),
            cell: Arc::clone(&part.cell),
            deletions: Vec::new(),
        };
        assert!(!nowhere.swap(&mut payload), "nothing to put back");
        assert_eq!(held(&nowhere.path), None, "and nothing changed");
    }

    #[test]
    fn a_cell_opened_again_keeps_none_of_the_deletions_an_earlier_session_made() {
        let path = PathBuf::from("a_cell_opened_again/0x1010.cell");
        let made = vec![Deletion {
            polygon: 0,
            records: 1,
            disable: false,
        }];
        deletions().insert(path.clone(), made);
        super::super::gta6_navmesh::remember(&path, cell());
        assert_eq!(deletions().get(&path), None);
    }

    #[test]
    fn a_save_makes_each_deletion_once_from_the_records_it_began_from() {
        let mut s = Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: navmesh_cell(),
            physical: Vec::new(),
        };
        let kept = s.clone();
        let made = |records| Deletion {
            polygon: 0,
            records,
            disable: false,
        };
        write_deletions(&mut s, &[made(5)], &[]).expect("a body saved with it holds it");
        assert_eq!(s, kept);
        let err = write_deletions(&mut s, &[made(0)], &[]).unwrap_err();
        assert!(err.contains("more than the 0"), "{err}");
        let err = write_deletions(&mut s, &[made(1)], &[]).unwrap_err();
        assert!(err.contains("last chunk"), "{err}");
        assert_eq!(s, kept);
    }
}
