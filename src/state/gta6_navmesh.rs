//! What a scanned GTA VI navigation cell states that GTA V's navigation mesh view has no place
//! for, kept for the inspector beside the view's model of it.

use crate::state::Rpf9Document;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ScanFind, Rpf9ZlibRun};
use or_io::gta6::headerless::{joins, Border, NavMesh, SpecialLink};
use or_job::{Cancel, Progress};

use super::jobs::{JobKind, Outcome, RunningJob, Then};
use super::rpf9_scan::{body_path, view_of, Row};
use super::{AppState, DocPayload, DocumentId};

const BORDERS: [Border; 4] = [Border::East, Border::West, Border::North, Border::South];

// ponytail: one entry per scanned cell opened, never emptied, as the scanned skeletons are.
static CELLS: Mutex<BTreeMap<PathBuf, Arc<NavMesh>>> = Mutex::new(BTreeMap::new());

fn cells() -> MutexGuard<'static, BTreeMap<PathBuf, Arc<NavMesh>>> {
    CELLS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keep `mesh`, the cell the body at `path` opened to, with none of an earlier session's deletions.
pub(super) fn remember(path: &Path, mesh: NavMesh) {
    super::gta6_navmesh_delete::forget(path);
    // A body whose links an edit of another cell renumbered keeps them while that edit holds.
    if super::gta6_navmesh_delete::links::renumbered_by(path).is_some() {
        return;
    }
    cells().insert(path.to_path_buf(), Arc::new(mesh));
}

/// The GTA VI cell the document at `path` opened from; `None` for any other document.
pub(crate) fn scanned_cell(path: &Path) -> Option<Arc<NavMesh>> {
    cells().get(path).cloned()
}

/// Keep `mesh` as the cell the document at `path` shows now, returning the one it held; `None`,
/// changing nothing, where no cell is kept there.
pub(super) fn replace(path: &Path, mesh: Arc<NavMesh>) -> Option<Arc<NavMesh>> {
    cells()
        .get_mut(path)
        .map(|held| std::mem::replace(held, mesh))
}

fn border_name(border: Border) -> &'static str {
    match border {
        Border::East => tr!("east"),
        Border::West => tr!("west"),
        Border::North => tr!("north"),
        Border::South => tr!("south"),
    }
}

fn point([x, y, z]: [f32; 3]) -> String {
    format!("({x:.2}, {y:.2}, {z:.2})")
}

/// Each border's side records and the polygons they name, each special link with its heading and
/// each footprint with its corners in metres.
pub(crate) fn cell_lines(m: &NavMesh) -> Vec<String> {
    let mut lines = vec![
        tr!("Where an archive holds several bodies of one cell, its neighbours join the second in archive order. A side record's chain edge names the polygon on this side of the border; the polygon across it is in the neighbouring cell's body, which opens on its own.").to_string(),
    ];
    if m.polygons.len() > or_io::gta5::navmesh::NO_NEIGHBOUR {
        lines.push(tr!("The view names a polygon across an edge in fifteen bits, so an edge to polygon 32,767 or past it leads nowhere there; the outline's disabled marks and link counts are taken from the cell itself.").to_string());
    }
    let edges = m.border_edges();
    for border in BORDERS {
        let on: Vec<_> = edges.iter().filter(|e| e.border == border).collect();
        if on.is_empty() {
            continue;
        }
        let mut records: Vec<usize> = on.iter().map(|e| e.record).collect();
        records.dedup();
        let mut named: Vec<usize> = on.iter().filter_map(|e| e.polygon).collect();
        named.sort_unstable();
        named.dedup();
        lines.push(tr!(
            "The {border} border, across which lies cell {cell}: {edges} chain edges of {records} side records, which name {polygons} polygons.",
            border = border_name(border),
            cell = border.across(m.cell).map_or_else(|| "-".to_string(), |c| c.to_string()),
            edges = on.len(),
            records = records.len(),
            polygons = named.len(),
        ));
        for e in on {
            lines.push(match e.polygon {
                Some(polygon) => tr!(
                    "The {border} border, side record {record}: polygon {polygon} meets it from {from} to {to}.",
                    border = border_name(border),
                    record = e.record,
                    polygon = polygon,
                    from = point(e.from),
                    to = point(e.to),
                ),
                None => tr!(
                    "The {border} border, side record {record}: its edge from {from} to {to} names no polygon.",
                    border = border_name(border),
                    record = e.record,
                    from = point(e.from),
                    to = point(e.to),
                ),
            });
        }
    }
    for (i, link) in m.special_links.iter().enumerate() {
        lines.push(link_line(m, i, link));
    }
    for (i, f) in m.footprints.iter().enumerate() {
        let corners: Vec<String> = (f.corners().iter())
            .map(|[x, y, _]| format!("({x:.3}, {y:.3})"))
            .collect();
        lines.push(tr!(
            "Footprint {index}, kind {kind}, at a height of {z} m: corners {corners}.",
            index = i,
            kind = format!("{:#06X}", f.kind),
            z = format!("{:.2}", f.position[2]),
            corners = corners.join(" "),
        ));
    }
    lines
}

/// A polygon's open values, raw: its byte +0x25 and each corner's edge bytes 6 to 11; then each
/// special link it is an end of.
pub(crate) fn polygon_lines(m: &NavMesh, polygon: usize) -> Vec<String> {
    let Some(p) = m.polygons.get(polygon) else {
        return Vec::new();
    };
    let mut lines = vec![tr!(
        "Polygon {polygon}: steepness {steepness}, raw, whose rule is not known.",
        polygon = polygon,
        steepness = p.steepness,
    )];
    let first = p.first as usize;
    for (k, e) in (m.edges.iter().skip(first).take(usize::from(p.corners))).enumerate() {
        lines.push(tr!(
            "Edge {edge}: word_06 {word}, flags {flags}, raw.",
            edge = k,
            word = format!("{:#06X}", e.word_06),
            flags = format!("{:#010X}", e.flags),
        ));
    }
    for (i, link) in m.special_links.iter().enumerate() {
        if link_ends(m, link).contains(&polygon) {
            lines.push(link_line(m, i, link));
        }
    }
    lines
}

/// The polygons of `m` a special link starts or ends at: an end in another cell is not one.
pub(crate) fn link_ends(m: &NavMesh, link: &SpecialLink) -> Vec<usize> {
    [
        m.link_polygon(link.from_mesh, link.from_polygon, link.from),
        m.link_polygon(link.to_mesh, link.to_polygon, link.to),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Each polygon's special links, as GTA V's polygon names a run of the link indices: a GTA VI
/// polygon states none, so its run is the links it is an end of.
pub(crate) fn link_runs(m: &NavMesh) -> Vec<Vec<u16>> {
    let mut runs = vec![Vec::new(); m.polygons.len()];
    for (i, link) in m.special_links.iter().enumerate() {
        let Ok(i) = u16::try_from(i) else { break };
        for end in link_ends(m, link) {
            if let Some(run) = runs.get_mut(end).filter(|r| r.last() != Some(&i)) {
                run.push(i);
            }
        }
    }
    runs
}

/// A special link's line: its ends in this cell by the polygons they lie in, others by number.
fn link_line(m: &NavMesh, i: usize, link: &SpecialLink) -> String {
    let end = |mesh, id: u16, at| m.link_polygon(mesh, id, at).unwrap_or(usize::from(id));
    tr!(
        "Special link {index}, from polygon {from} to polygon {to}: heading {degrees} degrees counter-clockwise from +y; word_18 {word}, raw.",
        index = i,
        from = end(link.from_mesh, link.from_polygon, link.from),
        to = end(link.to_mesh, link.to_polygon, link.to),
        degrees = format!("{:.1}", link.heading_degrees()),
        word = format!("{:#010X}", link.word_18),
    )
}

/// The polygon GTA V's edge for the edge at `index` names: its fifteen bits name none past 32,766.
pub(super) fn view_neighbour(m: &NavMesh, index: usize) -> usize {
    use or_io::gta5::navmesh::NO_NEIGHBOUR;
    (m.neighbour(index))
        .filter(|p| *p < NO_NEIGHBOUR)
        .unwrap_or(NO_NEIGHBOUR)
}

/// Per polygon, whether no open edge of the view's model `mesh` joins it to a polygon of the cell
/// `m`, out or in, at any polygon count; `None` where the two hold other numbers of edges.
pub(crate) fn disabled_polygons(mesh: &or_io::gta5::CNavMesh, m: &NavMesh) -> Option<Vec<bool>> {
    let open = super::gta6_navmesh_edit::open_edges(mesh);
    if open.len() != m.edges.len() {
        return None;
    }
    let mut disabled = vec![true; m.polygons.len()];
    for (polygon, p) in m.polygons.iter().enumerate() {
        let first = p.first as usize;
        for j in first..first.saturating_add(usize::from(p.corners)) {
            // ⚠ An edge into another cell is not a join here, as the view's model leads it nowhere.
            let joins =
                open.get(j) == Some(&true) && m.edges.get(j).is_some_and(|e| e.mesh == m.cell);
            if !joins {
                continue;
            }
            disabled[polygon] = false;
            if let Some(led_to) = m.neighbour(j).and_then(|t| disabled.get_mut(t)) {
                *led_to = false;
            }
        }
    }
    Some(disabled)
}

/// The cell's own root fields, as the inspector's grid shows them.
pub(crate) fn cell_rows(m: &NavMesh) -> Vec<(String, String)> {
    let [lo, hi] = [m.tree.box_min, m.tree.box_max].map(point);
    vec![
        (
            tr!("Cell").to_string(),
            format!("{} (x {}, y {})", m.cell, m.cell % 512, m.cell / 512),
        ),
        (tr!("Flags").to_string(), m.flags.to_string()),
        (tr!("Build id").to_string(), format!("{:08X}", m.build_id)),
        (tr!("Box").to_string(), format!("{lo} - {hi}")),
    ]
}

/// The archive and offset of the scanned body at `path`, from its name alone.
fn body_at(path: &Path) -> Option<(&Path, u64)> {
    let stem = path.file_stem()?.to_str()?.strip_prefix("0x")?;
    Some((path.parent()?, u64::from_str_radix(stem, 16).ok()?))
}

// ponytail: every body a search chose, never emptied, as the cells are.
static CHOSEN: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn chosen() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    CHOSEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Each border of the cell opened at `path`, the cell across it, and that cell's open body a
/// search of the scan chose: only a read of every body of the archive knows which is the second.
pub(crate) fn neighbours(path: &Path, m: &NavMesh) -> Vec<(Border, u32, Option<Arc<NavMesh>>)> {
    // A body of no cell has no border, and no other body's special link ends in it.
    if m.cell == 0 {
        return Vec::new();
    }
    let archive = body_at(path).map(|(archive, _)| archive);
    let (open, chosen) = (cells(), chosen());
    BORDERS
        .into_iter()
        .filter_map(|border| {
            let across = border.across(m.cell)?;
            let body = chosen_body(&open, &chosen, archive, across).map(|(_, n)| n);
            Some((border, across, body))
        })
        .collect()
}

/// The open body of `cell` among `open` that a search of the scan of `archive` chose.
fn chosen_body(
    open: &BTreeMap<PathBuf, Arc<NavMesh>>,
    chosen: &BTreeSet<PathBuf>,
    archive: Option<&Path>,
    cell: u32,
) -> Option<(PathBuf, Arc<NavMesh>)> {
    (open.iter())
        .find(|(p, n)| {
            n.cell == cell && chosen.contains(*p) && body_at(p).map(|(a, _)| a) == archive
        })
        .map(|(p, n)| (p.clone(), Arc::clone(n)))
}

/// Each cell across `m`'s borders or at its corners, where a link of either may end in the other,
/// and that cell's open body a search of the scan chose.
pub(crate) fn linked_cells(path: &Path, m: &NavMesh) -> Vec<(u32, Option<Arc<NavMesh>>)> {
    if m.cell == 0 {
        return Vec::new();
    }
    let archive = body_at(path).map(|(archive, _)| archive);
    let (open, chosen) = (cells(), chosen());
    (Border::around(m.cell).into_iter())
        .map(|c| (c, chosen_body(&open, &chosen, archive, c).map(|(_, n)| n)))
        .collect()
}

/// The path and the open body of each cell of [`linked_cells`] that has one.
pub(super) fn neighbour_paths(path: &Path, m: &NavMesh) -> Vec<(PathBuf, Arc<NavMesh>)> {
    if m.cell == 0 {
        return Vec::new();
    }
    let archive = body_at(path).map(|(archive, _)| archive);
    let (open, chosen) = (cells(), chosen());
    (Border::around(m.cell).into_iter())
        .filter_map(|c| chosen_body(&open, &chosen, archive, c))
        .collect()
}

/// Where the cell's polygons meet those of each neighbour that is open, or that it is not.
pub(crate) fn join_lines(m: &NavMesh, near: &[(Border, u32, Option<Arc<NavMesh>>)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (border, cell, body) in near {
        let Some(other) = body else {
            lines.push(tr!(
                "Cell {cell}, across the {border} border, has no body a search opened: find the neighbouring cells in the scan to name the polygons that meet this cell's.",
                cell = cell,
                border = border_name(*border),
            ));
            continue;
        };
        let joins = joins(m, other);
        if joins.is_empty() {
            lines.push(tr!(
                "No polygon meets one of cell {cell} across the {border} border.",
                cell = cell,
                border = border_name(*border),
            ));
        }
        for j in joins {
            lines.push(tr!(
                "Polygon {polygon} meets polygon {other} of cell {cell} across the {border} border.",
                polygon = j.polygon,
                other = j.other,
                cell = cell,
                border = border_name(*border),
            ));
        }
    }
    lines
}

/// How a search of a scan for a cell's neighbours ended: the bodies it found, or why it did not,
/// `None` where it was stopped.
type Searched = Result<Vec<PathBuf>, Option<String>>;

/// A search of a scan's navigation cells for those across a cell's borders, on a worker.
// ponytail: one search at a time; a new one replaces it and stops it.
pub(super) struct Search {
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Searched>>>,
}

impl Drop for Search {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// The bodies of `wanted` cells among `finds` of `archive`, read one after another: where a cell
/// has several, the second in archive order.
pub(super) fn find_cells(
    archive: &Path,
    finds: &[Rpf9ScanFind],
    wanted: &[u32],
    cancel: &Cancel,
    progress: &Progress,
) -> Searched {
    let file = File::open(archive).map_err(|e| Some(format!("{}: {e}", archive.display())))?;
    let mut found: BTreeMap<u32, Vec<&Rpf9ScanFind>> = BTreeMap::new();
    for (i, find) in finds.iter().enumerate() {
        if cancel.is_set() {
            return Err(None);
        }
        progress.reached(i as u64, finds.len() as u64);
        let Ok((read, bytes)) = read_rpf9_body(&file, find.offset, BODY_LIMIT) else {
            continue;
        };
        let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) = read.kind
        else {
            continue;
        };
        let cell = resource.sections(&bytes).0.get(CELL..CELL + 4);
        if let Some(cell) = cell.and_then(|c| <[u8; 4]>::try_from(c).ok()) {
            let cell = u32::from_le_bytes(cell);
            if wanted.contains(&cell) {
                found.entry(cell).or_default().push(find);
            }
        }
    }
    Ok(found
        .values()
        .filter_map(|bodies| bodies.get(1).or(bodies.first()))
        .filter_map(|find| Some(body_path(archive, find.offset, view_of(find).ok()?)))
        .collect())
}

/// Where a navigation cell's root states its cell.
const CELL: usize = 0xC8;
/// The most a cell's body inflates to.
const BODY_LIMIT: u64 = 1 << 30;

impl AppState {
    /// Look through the scan of the archive the cell `doc` came from for the cells across its
    /// borders and at its corners, on a worker; each found opens once the search ends.
    pub(crate) fn find_neighbour_cells(&mut self, doc: DocumentId) {
        let Some(path) = self.document(doc).and_then(|d| d.path.clone()) else {
            return;
        };
        let (Some(m), Some((archive, _))) = (scanned_cell(&path), body_at(&path)) else {
            return;
        };
        let finds: Vec<Rpf9ScanFind> = (self.documents.iter())
            .filter_map(|d| match &d.payload {
                DocPayload::Rpf9(Rpf9Document::Scan(scan)) if scan.path == archive => {
                    Some(Arc::clone(scan))
                }
                _ => None,
            })
            .flat_map(|scan| {
                (scan.found().rows.iter())
                    .filter_map(|row| match row {
                        Row::Body(find) if is_cell(find) => Some(**find),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if finds.is_empty() {
            self.show_info_toast(tr!("The scan of the archive this cell came from is not open, or has found no navigation cell: open the archive to search it."));
            return;
        }
        let wanted = Border::around(m.cell);
        let found = Search {
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let (cancel, progress, ended) = (
            found.cancel.clone(),
            found.progress.clone(),
            Arc::clone(&found.ended),
        );
        let archive = archive.to_path_buf();
        std::thread::spawn(move || {
            let outcome = find_cells(&archive, &finds, &wanted, &cancel, &progress);
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        self.neighbour_search = Some(found);
    }

    /// Open the cells the search found and say how it ended, once.
    pub(super) fn poll_neighbour_search(&mut self) {
        let ended = (self.neighbour_search.as_ref()).and_then(|s| s.ended.lock().ok()?.take());
        if ended.is_some() {
            self.neighbour_search = None;
        }
        let (outcome, said) = match ended {
            None => return,
            Some(Ok(paths)) => {
                let said = tr_n!(
                    "{count} neighbouring cell found in the scan; it opens",
                    "{count} neighbouring cells found in the scan; they open",
                    paths.len()
                );
                chosen().extend(paths.iter().cloned());
                self.load_files(paths);
                (Outcome::Done(Then::Nothing), said)
            }
            Some(Err(None)) => (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished").to_string(),
            ),
            Some(Err(Some(why))) => (Outcome::Failed, why),
        };
        self.jobs.record(JobKind::NeighbourCells, outcome, said);
    }

    /// The search while it runs, as the job list draws it.
    pub(super) fn neighbour_search_job(&self) -> Option<RunningJob> {
        let s = self.neighbour_search.as_ref()?;
        if s.ended.lock().ok()?.is_some() {
            return None;
        }
        Some(RunningJob {
            kind: JobKind::NeighbourCells,
            doing: tr!("Finding a GTA VI cell's neighbours in its scan").to_string(),
            fraction: Some(s.progress.snapshot().fraction()),
            detail: String::new(),
            stopping: s.cancel.is_set(),
        })
    }

    pub(super) fn stop_neighbour_search(&self) {
        if let Some(s) = &self.neighbour_search {
            s.cancel.set();
        }
    }
}

/// Whether a scan's row is a headerless navigation cell.
fn is_cell(find: &Rpf9ScanFind) -> bool {
    matches!(find.kind, Rpf9BodyKind::Zlib(Rpf9ZlibRun { signature: Some(s), resource: Some(_), .. })
        if s.role() == Some(or_io::container::rpf::Rpf9Role::NavMesh))
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{
        Footprint, NavEdge, NavNode, NavPolygon, SpecialLink, NOWHERE, SIDE_RECORD,
    };

    #[test]
    fn a_search_that_is_replaced_is_stopped() {
        let replaced = Search {
            cancel: Cancel::new(),
            progress: Progress::new(),
            ended: Arc::default(),
        };
        let cancel = replaced.cancel.clone();
        drop(replaced);
        assert!(cancel.is_set());
    }

    /// A cell at the grid's (10, 20) of one polygon and one side record on its east border,
    /// one link heading a quarter turn and one footprint.
    fn cell() -> NavMesh {
        let cell = 10 + 512 * 20;
        let polygon = |first, corners, side_flags, steepness| NavPolygon {
            first,
            corners,
            flags: 0,
            min_max: [0; 6],
            centroid: [0; 2],
            side_flags,
            steepness,
            word_20: 0,
        };
        let edge = |mesh, polygon, word_06, flags| NavEdge {
            mesh,
            polygon,
            word_06,
            flags,
        };
        NavMesh {
            flags: 9,
            cell,
            build_id: 0,
            tree: NavNode {
                box_min: [0.0; 3],
                box_max: [128.0, 128.0, 8.0],
                polygons: None,
                children: Default::default(),
            },
            // The polygon's corners, then the side record's at x 128 from y 4 to y 0.
            vertices: vec![
                [0, 0, 0],
                [100, 0, 0],
                [0, 100, 0],
                [65535, 2048, 8192],
                [65535, 0, 8192],
            ],
            indices: vec![0, 1, 2, 3, 4],
            edges: vec![
                edge(cell, 0, 0x1234, 0x10),
                edge(NOWHERE, 0, 0x5678, 0x02),
                edge(NOWHERE, 0, 0, 0x12),
                edge(NOWHERE, 0, 0, 0),
                edge(cell, 0, 0, 0),
            ],
            polygons: vec![polygon(0, 3, 0, 49), polygon(3, 2, SIDE_RECORD, 0)],
            polygons_proper: 1,
            nodes: 1,
            depth: 0,
            leaves: 1,
            special_links: vec![SpecialLink {
                from: [0; 3],
                to: [0; 3],
                from_polygon: 0,
                to_polygon: 0,
                from_mesh: cell,
                to_mesh: cell,
                word_18: 0x0072_5B03,
                heading: 64,
            }],
            special_link_indices: vec![0],
            footprints: vec![Footprint {
                key: 0,
                position: [10.0, -20.0, 3.0],
                kind: 0x0408,
                outline: [
                    [512, 256],
                    [-1024, 128],
                    [-256, -512],
                    [100, -50],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                ],
            }],
        }
    }

    #[test]
    fn a_cell_lists_its_border_s_polygons_its_links_headings_and_its_footprints_in_metres() {
        let text = cell_lines(&cell()).join("\n");
        for said in [
            "The east border, across which lies cell 10251: 1 chain edges of 1 side records, which name 1 polygons.",
            "The east border, side record 1: polygon 0 meets it from (128.00, 0.00, 1.00) to (128.00, 4.00, 1.00).",
            "heading 90.0 degrees counter-clockwise from +y; word_18 0x00725B03, raw.",
            "kind 0x0408, at a height of 3.00 m: corners (11.000, -19.500) (8.000, -19.750) (9.500, -21.000) (10.195, -20.098).",
        ] {
            assert!(text.contains(said), "{said}\n{text}");
        }
        let raw = polygon_lines(&cell(), 0).join("\n");
        assert!(raw.contains("Polygon 0: steepness 49, raw"), "{raw}");
        assert!(
            raw.contains("Edge 1: word_06 0x5678, flags 0x00000002, raw."),
            "{raw}"
        );
        assert_eq!(
            polygon_lines(&cell(), 0).len(),
            5,
            "the polygon, its three edges and the link it is both ends of"
        );
        assert!(polygon_lines(&cell(), 1)
            .iter()
            .all(|l| !l.contains("Special link")));
        let mut linked = cell();
        linked.special_links[0].to_polygon = 1;
        assert_eq!(
            link_runs(&cell()),
            [vec![0], vec![]],
            "once, though both ends"
        );
        assert_eq!(link_runs(&linked), [vec![0], vec![0]]);
        linked.special_links[0].to_mesh = NOWHERE;
        assert_eq!(
            link_runs(&linked),
            [vec![0], vec![]],
            "an end in another cell"
        );
        let rows = cell_rows(&cell());
        assert!(rows.contains(&("Cell".into(), "10250 (x 10, y 20)".into())));
        assert!(rows.contains(&("Flags".into(), "9".into())));
    }

    #[test]
    fn a_cell_past_fifteen_bits_of_polygons_opens_its_edges_past_them_leading_nowhere() {
        use or_io::gta5::navmesh::NO_NEIGHBOUR;
        let small = cell();
        assert_eq!(view_neighbour(&small, 0), 0);
        assert_eq!(
            view_neighbour(&small, 1),
            NO_NEIGHBOUR,
            "an edge to no cell"
        );
        let said = "The view names a polygon across an edge in fifteen bits";
        assert!(cell_lines(&small).iter().all(|l| !l.contains(said)));

        let mut big = cell();
        let past = NO_NEIGHBOUR + 3;
        // Fillers whose corners name no index, so no border runs through them.
        let filler = NavPolygon {
            first: 100,
            ..big.polygons[0]
        };
        big.polygons.resize(past + 1, filler);
        big.edges[4].polygon = past as u16;
        big.edges[0].polygon = 7;
        assert_eq!(view_neighbour(&big, 0), 7);
        assert_eq!(view_neighbour(&big, 4), NO_NEIGHBOUR);
        assert!(cell_lines(&big).iter().any(|l| l.contains(said)));
    }

    #[test]
    fn an_edge_into_the_open_neighbouring_cell_names_the_polygon_across_it() {
        let a = cell();
        // The cell east of it, whose side record runs along its west border at height `z`.
        let east_of = |z: u16| {
            let mut b = cell();
            b.cell += 1;
            b.vertices[3] = [0, 2048, z];
            b.vertices[4] = [0, 0, z];
            for e in b.edges.iter_mut().filter(|e| e.mesh != NOWHERE) {
                e.mesh = b.cell;
            }
            b
        };
        let b = east_of(8192);
        let near = vec![
            (Border::East, b.cell, Some(Arc::new(b))),
            (Border::North, a.cell + 512, None),
        ];
        let text = join_lines(&a, &near).join("\n");
        assert!(
            text.contains("Polygon 0 meets polygon 0 of cell 10251 across the east border."),
            "{text}"
        );
        assert!(
            text.contains("Cell 10762, across the north border, has no body a search opened"),
            "{text}"
        );
        let lower = east_of(0);
        let near = vec![(Border::East, lower.cell, Some(Arc::new(lower)))];
        let text = join_lines(&a, &near).join("\n");
        assert!(
            text.contains("No polygon meets one of cell 10251"),
            "{text}"
        );
    }

    /// The view's model of `m`'s edges, as a scanned cell opens with it.
    fn view(m: &NavMesh) -> or_io::gta5::CNavMesh {
        use or_io::gta5::{AiSplitArray, CNavMesh, SubArray};
        let elements = (0..m.edges.len())
            .map(|i| crate::state::gta6_navmesh_edit::view_edge(m, i).expect("an edge"))
            .collect();
        CNavMesh {
            adjacent_polys_array: Some(Box::new(AiSplitArray {
                sub_arrays: vec![SubArray {
                    elements,
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    #[test]
    fn a_cell_past_fifteen_bits_marks_a_polygon_disabled_only_where_no_open_edge_joins_it() {
        use or_io::gta5::navmesh::NO_NEIGHBOUR;
        use or_io::gta5::{CNavMesh, TAdjPoly};
        let mut big = cell();
        let past = NO_NEIGHBOUR + 3;
        let filler = NavPolygon {
            first: 100,
            ..big.polygons[0]
        };
        big.polygons.resize(past + 1, filler);
        // The side record's edge leads to a polygon past the view's fifteen bits.
        big.edges[4].polygon = past as u16;
        let open = view(&big);
        let disabled = disabled_polygons(&open, &big).expect("as many edges");
        assert_eq!(disabled.len(), past + 1);
        assert!(!disabled[0] && !disabled[1] && !disabled[past], "joined");
        assert!(disabled[2] && disabled[past - 1], "fillers join nothing");

        let mut cut = open.clone();
        let nowhere = TAdjPoly::leading_to(0, NO_NEIGHBOUR).expect("an edge to nowhere");
        cut.adjacent_polys_array.as_mut().expect("edges").sub_arrays[0].elements[4] = nowhere;
        let disabled = disabled_polygons(&cut, &big).expect("as many edges");
        assert!(
            disabled[1] && disabled[past],
            "only the cut edge joined them"
        );
        assert!(!disabled[0], "its edge into itself is open");
        assert_eq!(disabled_polygons(&CNavMesh::default(), &big), None);
    }

    #[test]
    fn a_body_chosen_at_a_corner_is_one_an_edit_renumbers_though_no_border_names_it() {
        let m = cell();
        let here = PathBuf::from("a_body_chosen_at_a_corner/0x1010.rsc");
        let corner = PathBuf::from("a_body_chosen_at_a_corner/0x2010.rsc");
        let mut at_corner = cell();
        at_corner.cell = m.cell + 512 + 1;
        remember(&corner, at_corner);
        chosen().insert(corner.clone());
        let linked = linked_cells(&here, &m);
        assert_eq!(linked.len(), 8, "four borders and four corners");
        let open: Vec<u32> = (linked.iter())
            .filter(|(_, body)| body.is_some())
            .map(|(c, _)| *c)
            .collect();
        assert_eq!(open, [m.cell + 512 + 1]);
        let paths: Vec<PathBuf> = (neighbour_paths(&here, &m).into_iter())
            .map(|(p, _)| p)
            .collect();
        assert_eq!(paths, [corner]);
        assert!(neighbours(&here, &m)
            .iter()
            .all(|(_, _, body)| body.is_none()));
    }
}
