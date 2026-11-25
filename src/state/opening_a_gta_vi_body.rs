//! A GTA VI bound, fragment or dictionary of drawables a scan found opens in the model view, and an
//! effect container it lists is read as a job of the list.

use std::time::{Duration, Instant};

use super::jobs::JobKind;
use super::opening_an_rpf9_archive::{
    le32, row_at, rows, rpf9_head, said, scanned, settle, written, zlib,
};
use super::{AppState, DocPayload, DocumentKind, ToastType};

/// The virtual section of a GTA VI bound: a composite of one geometry child, placed 10 m along X,
/// that holds a triangle of 1 m legs and a capsule.
pub(super) fn bound_body() -> Vec<u8> {
    const V: u64 = 0x5000_0000;
    let mut v = vec![0u8; 0x400];
    let put =
        |v: &mut Vec<u8>, at: usize, bytes: &[u8]| v[at..at + bytes.len()].copy_from_slice(bytes);
    let floats = |v: &mut Vec<u8>, at: usize, xs: &[f32]| {
        for (i, x) in xs.iter().enumerate() {
            put(v, at + 4 * i, &x.to_le_bytes());
        }
    };
    // The block map at 0x3E0, its 24 bytes inside the section: one virtual block.
    put(&mut v, 0x08, &(V + 0x3E0).to_le_bytes());
    v[0x3E8] = 1;
    // The composite: type 6, its radius, box and centroid, one child.
    v[0x10] = 6;
    v[0x11] = 1;
    floats(&mut v, 0x14, &[20.0]);
    floats(&mut v, 0x20, &[11.0, 1.0, 1.0, 0.005]);
    floats(&mut v, 0x30, &[-1.0, -1.0, -1.0]);
    put(&mut v, 0x3C, &1u32.to_le_bytes());
    floats(&mut v, 0x40, &[5.0, 0.5, 0.5]);
    put(&mut v, 0x70, &(V + 0x100).to_le_bytes());
    put(&mut v, 0x78, &(V + 0x110).to_le_bytes());
    put(&mut v, 0x80, &(V + 0x110).to_le_bytes());
    put(&mut v, 0x98, &[1, 0, 1, 0]);
    put(&mut v, 0x100, &(V + 0x180).to_le_bytes());
    // Its matrix: the three axes, then the origin.
    floats(&mut v, 0x110, &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    floats(&mut v, 0x130, &[0.0, 0.0, 1.0, 0.0, 10.0, 0.0, 0.0, 0.0]);
    // The geometry child at 0x180: type 5, its box, quantised vertices and two primitives.
    let g = 0x180;
    v[g + 0x10] = 5;
    floats(&mut v, g + 0x20, &[1.0, 1.0, 1.0]);
    put(&mut v, g + 0x88, &(V + 0x2E0).to_le_bytes());
    floats(&mut v, g + 0x90, &[0.001, 0.001, 0.001]);
    put(&mut v, g + 0xB0, &(V + 0x300).to_le_bytes());
    put(&mut v, g + 0xD0, &3u32.to_le_bytes());
    put(&mut v, g + 0xD4, &2u32.to_le_bytes());
    let mut triangle = [0u8; 16];
    triangle[..3].fill(0xFF);
    triangle[4..10].copy_from_slice(&[0, 0, 1, 0, 2, 0]);
    triangle[10..].fill(0xFF);
    put(&mut v, 0x2E0, &triangle);
    let mut capsule = [0u8; 16];
    capsule[3] = 2;
    capsule[4..8].copy_from_slice(&0.25f32.to_le_bytes());
    capsule[8] = 1;
    capsule[10..].fill(0xFF);
    put(&mut v, 0x2F0, &capsule);
    for (i, q) in [[0i16, 0, 0], [1000, 0, 0], [0, 1000, 0]]
        .iter()
        .enumerate()
    {
        for (a, x) in q.iter().enumerate() {
            put(&mut v, 0x300 + 6 * i + 2 * a, &x.to_le_bytes());
        }
    }
    v
}

#[test]
fn a_gta_vi_bound_opens_in_the_model_view_with_its_triangle_and_capsule_drawn() {
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000 + 16, 0);
    bytes.extend(zlib(&bound_body()));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    state.open_scanned_body(doc, row_at(&rows(&state, doc), 0x1010));
    settle(&mut state);
    let opened = (state.active_document)
        .filter(|&d| d != doc)
        .unwrap_or_else(|| panic!("the bound: {}", state.error_dialog_message));
    let d = state.document(opened).expect("open");
    assert_eq!(d.kind, DocumentKind::Gta6Drawable);
    assert!(d.read_only.is_some());
    assert_eq!(state.active_workspace, crate::ui::dock::Workspace::Model);
    let request = super::capabilities::GeometryRequest {
        state: &state,
        doc: d,
        selection: None,
        entry: None,
    };
    let meshes = crate::render::gta6_drawable::meshes(&request);
    assert_eq!(meshes.len(), 1);
    assert_eq!(&meshes[0].indices[..3], [0, 1, 2], "the triangle first");
    let x = |i: usize| meshes[0].vertices[i].position[0];
    assert!(
        (x(0) - 10.0).abs() < 1e-4 && (x(1) - 11.0).abs() < 1e-3,
        "placed 10 m along X"
    );
    // The capsule from vertex 0 to vertex 1, 0.25 m about its axis.
    let capsule = &meshes[0].vertices[3..];
    assert!(!capsule.is_empty(), "the capsule is drawn");
    assert!(
        capsule.iter().all(
            |v| (-0.25 - 1e-4..=1.25 + 1e-3).contains(&(v.position[0] - 10.0))
                && v.position[1].abs() <= 0.25 + 1e-4
        ),
        "the capsule's extent"
    );
    assert!(crate::render::gta6_drawable::bounds(&request).is_some());
    let DocPayload::Gta6Drawable(body) = &d.payload else {
        panic!("a GTA VI body");
    };
    let text = super::gta6_resource::body_lines(body, None).join("\n");
    assert!(
        text.contains("1 triangles and 1 spheres, capsules"),
        "{text}"
    );
    // It binds no texture, so the gallery says so rather than asking for a drawable.
    assert!(matches!(
        super::gta6_textures::list(&d.payload, None),
        super::TextureListing::Nothing("Active document has no texture dictionary")
    ));
    state.toasts.clear();
    assert!(!state.save_active_document());
    assert!(said(&state, ToastType::Warning, &["read-only"]));
}

#[test]
fn a_gta_vi_dictionary_draws_the_drawable_chosen_from_its_list_and_none_before() {
    use crate::ui::panels::selection_ids::dict_entry_id;
    use or_io::gta6::headerless::{
        Dictionary, Drawable, Entry, Geometry, Model, Resource, Vertices,
    };
    let drawable = |name: &str| Drawable {
        name: name.to_string(),
        box_min: [0.0; 3],
        box_max: [1.0, 1.0, 0.0],
        shaders: Vec::new(),
        levels: [
            Some(vec![Model {
                geometries: vec![Ok(Geometry {
                    vertex_data: Vec::new(),
                    index_data: Vec::new(),
                    shader: 0,
                    vertex_count: 3,
                    channels: Vec::new(),
                    triangles: vec![[0, 1, 2]],
                    positions: Ok(vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
                    vertices: Vertices::default(),
                })],
                palette: Ok(or_io::gta6::headerless::Palette::Own),
                matrix_index: 0,
            }]),
            None,
            None,
            None,
        ],
        lod_distances: [9998.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    };
    let dictionary = Resource::Dictionary(Dictionary {
        entries: vec![
            (0x10, Ok(Entry::Drawable(Box::new(drawable("a.#dr"))))),
            (0x20, Ok(Entry::Drawable(Box::new(drawable("b.#dr"))))),
        ],
    });
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let payload = DocPayload::Gta6Drawable(std::sync::Arc::new(dictionary));
    let doc = state.insert_document(dir.path().join("0x10.txt"), payload, None, None);
    let meshes = |state: &AppState| {
        let request = super::capabilities::GeometryRequest {
            state,
            doc: state.document(doc).expect("open"),
            selection: state.selection.selected(doc),
            entry: None,
        };
        crate::render::gta6_drawable::meshes(&request)
    };
    assert!(
        meshes(&state).is_empty(),
        "nothing is drawn before a drawable is chosen"
    );
    assert!(state
        .exports_gltf(super::gltf::Model::Document(doc))
        .is_err_and(|why| why.contains("Choose")));
    state.selection.select(doc, dict_entry_id(0x20));
    assert_eq!(meshes(&state).len(), 1);
    assert_eq!(
        state.exports_gltf(super::gltf::Model::Document(doc)),
        Ok("0x10.txt".to_string())
    );
    // A chosen drawable of the dictionary takes its geometry from a glTF too.
    assert_eq!(
        state.gltf_import_into(super::gltf::Model::Document(doc)),
        Ok(doc)
    );
}

#[test]
fn the_cells_across_a_gta_vi_cell_s_borders_are_found_in_its_scan_as_a_job_and_open() {
    use super::jobs::Outcome;
    use super::opening_an_rpf9_archive::navmesh_cell;
    let cell = |index: u32| {
        let mut v = navmesh_cell();
        v[0xC8..0xCC].copy_from_slice(&le32(index));
        v
    };
    let here = 256 + 512 * 256;
    let mut bytes = rpf9_head(1, 0x1A0);
    let mut offsets = Vec::new();
    // This cell, the cell east of it twice, then one that is no neighbour.
    for body in [cell(here), cell(here + 1), cell(here + 1), cell(here + 7)] {
        bytes.resize(bytes.len().next_multiple_of(0x1000) + 16, 0);
        offsets.push(bytes.len() as u64);
        bytes.extend(zlib(&body));
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let archive = written(dir.path(), &bytes);
    let doc = scanned(&mut state, archive.clone());
    state.open_scanned_body(doc, row_at(&rows(&state, doc), offsets[0]));
    settle(&mut state);
    let opened = (state.active_document)
        .filter(|&d| d != doc)
        .unwrap_or_else(|| panic!("the cell: {}", state.error_dialog_message));
    let path = state
        .document(opened)
        .and_then(|d| d.path.clone())
        .expect("a path");
    let joins = || {
        let m = super::gta6_navmesh::scanned_cell(&path).expect("the cell");
        let near = super::gta6_navmesh::neighbours(&path, &m);
        super::gta6_navmesh::join_lines(&m, &near).join("\n")
    };
    let east_unjoined = format!("Cell {}, across the east border, has no body", here + 1);
    assert!(joins().contains(&east_unjoined), "{}", joins());

    // The first body of the east cell, opened by hand, is not the one its neighbours join.
    state.open_scanned_body(doc, row_at(&rows(&state, doc), offsets[1]));
    settle(&mut state);
    let first = (state.active_document)
        .filter(|&d| d != doc && d != opened)
        .unwrap_or_else(|| panic!("the first east body: {}", state.error_dialog_message));
    state.close_document(first);
    assert!(joins().contains(&east_unjoined), "{}", joins());

    let finds: Vec<_> = (rows(&state, doc).into_iter())
        .filter_map(|r| match r {
            super::rpf9_scan::Row::Body(find) => Some(*find),
            _ => None,
        })
        .collect();
    let stopped = or_job::Cancel::new();
    stopped.set();
    let progress = or_job::Progress::new();
    let found = super::gta6_navmesh::find_cells(&archive, &finds, &[here + 1], &stopped, &progress);
    assert_eq!(found, Err(None), "a stopped search reads nothing");

    state.find_neighbour_cells(opened);
    let deadline = Instant::now() + Duration::from_secs(30);
    let ended = loop {
        state.poll_jobs();
        let ended = (state.jobs.ended().iter()).find(|j| j.kind == JobKind::NeighbourCells);
        if let Some(ended) = ended {
            break ended.clone();
        }
        assert!(Instant::now() < deadline, "the search never ended");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(ended.outcome, Outcome::Done(super::jobs::Then::Nothing));
    assert!(ended.said.contains("1 neighbouring cell"), "{}", ended.said);
    settle(&mut state);
    let open = |offset: u64| {
        let name = format!("{offset:#X}.rsc");
        (state.documents.iter()).any(|d| d.path.as_deref().is_some_and(|p| p.ends_with(&name)))
    };
    assert!(open(offsets[2]), "the second body of the east cell opens");
    assert!(!open(offsets[1]) && !open(offsets[3]));
    let said = joins();
    assert!(
        said.contains("No polygon meets one of cell 131329"),
        "{said}"
    );
}

/// An `SGD5` head of one binding layout of `records` records.
fn sgd5_head(records: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x40];
    bytes[..4].copy_from_slice(b"SGD5");
    bytes[0x0C..0x10].copy_from_slice(&le32(1));
    bytes[0x10..0x14].copy_from_slice(&le32(24 + 8 * u32::from(records)));
    bytes[0x1C..0x1E].copy_from_slice(&u16::from(records).to_le_bytes());
    bytes[0x1F] = records;
    bytes
}

#[test]
fn the_install_folder_s_effect_containers_are_read_once_as_a_job_without_a_click() {
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000, 0);
    bytes.extend(sgd5_head(3));
    let dir = tempfile::tempdir().expect("a scratch directory");
    std::fs::write(dir.path().join("shaders.rpf"), &bytes).expect("written");
    let reads = |state: &AppState| {
        (state.jobs.ended().iter())
            .filter(|j| j.kind == JobKind::EffectsRead)
            .count()
    };
    let mut state = AppState::new();
    state.poll_jobs();
    assert_eq!(reads(&state), 0, "no install folder is set");
    let root = dir.path().to_path_buf();
    (state.prefs.game_paths).set(or_title::Game::GtaVI, Some(root));
    let deadline = Instant::now() + Duration::from_secs(30);
    while reads(&state) == 0 {
        state.poll_jobs();
        assert!(Instant::now() < deadline, "the read never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..3 {
        state.poll_jobs();
    }
    assert!(
        (state.running_jobs().iter()).all(|j| j.kind != JobKind::EffectsRead),
        "the same folder is read once"
    );
    assert_eq!(reads(&state), 1);
}

#[test]
fn an_effect_container_is_read_as_a_job_with_progress_that_stops_when_asked_and_says_how_it_ended()
{
    use super::jobs::Outcome;
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1000, 0);
    bytes.extend(sgd5_head(3));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = written(dir.path(), &bytes);
    // A long run of zeros after the head, which the container spans to the file's end.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("reopened");
    file.set_len(256 << 20).expect("a long file");
    drop(file);
    let mut state = AppState::new();
    let doc = scanned(&mut state, path);
    state.open_scanned_body(doc, row_at(&rows(&state, doc), 0x1000));
    assert!(said(&state, ToastType::Info, &["effects", "background"]));
    let job = (state.running_jobs().into_iter())
        .find(|j| j.kind == JobKind::EffectsRead)
        .expect("the read is a row of the job list");
    assert!(job.fraction.is_some(), "the row shows how far it has read");
    state.cancel_job(JobKind::EffectsRead);
    let deadline = Instant::now() + Duration::from_secs(30);
    let ended = loop {
        state.poll_jobs();
        let ended = (state.jobs.ended().iter()).find(|j| j.kind == JobKind::EffectsRead);
        if let Some(ended) = ended {
            break ended.clone();
        }
        assert!(Instant::now() < deadline, "the read never ended");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(ended.outcome, Outcome::Stopped(super::jobs::Then::Nothing));
    assert!(state
        .running_jobs()
        .iter()
        .all(|j| j.kind != JobKind::EffectsRead));
}

#[test]
fn a_resource_is_listed_under_what_it_is_and_not_under_one_kind_for_all() {
    use super::opening_an_rpf9_archive::navmesh_cell;
    use super::rpf9_scan::RowKind;
    let mut bytes = rpf9_head(1, 0x1A0);
    let mut offsets = Vec::new();
    for body in [bound_body(), navmesh_cell()] {
        bytes.resize(bytes.len().next_multiple_of(0x1000) + 16, 0);
        offsets.push(bytes.len() as u64);
        bytes.extend(zlib(&body));
    }
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let doc = scanned(&mut state, written(dir.path(), &bytes));
    let listed = rows(&state, doc);
    let kinds: Vec<RowKind> = (offsets.iter())
        .map(|&at| RowKind::of(&listed[row_at(&listed, at)]))
        .collect();
    assert_eq!(kinds, [RowKind::Collision, RowKind::Navigation]);
}

#[test]
fn an_effect_container_opens_as_its_effects_and_a_pipeline_cache_says_why_no_view_shows_it() {
    use or_io::container::rpf::{Rpf9BodyKind, Rpf9Content, Rpf9ScanFind, Rpf9ZlibRun};
    let find = |kind| Rpf9ScanFind {
        offset: 0x1000,
        kind,
        span: 0x1000,
        stored_len: None,
        inflated_len: None,
        container: None,
    };
    let container = find(Rpf9BodyKind::Sgd5 { head_layouts: 3 });
    let view = super::rpf9_scan::view_of(&container);
    assert!(
        matches!(view, Ok(super::rpf9_scan::View::Effects)),
        "{view:?}"
    );
    // Its reader reads it, so it is not shown as its bytes; the cache is.
    assert!(!super::rpf9_scan::opens_as_bytes(&container));
    let run = Rpf9ZlibRun {
        content: Rpf9Content::PipelineCache,
        chunks: 1,
        stored_pieces: 0,
        first_chunk_len: 0,
        first_chunk_inflated: 0,
        inflated: 0,
        signature: None,
        resource: None,
    };
    let cache = find(Rpf9BodyKind::Zlib(run));
    let why = super::rpf9_scan::view_of(&cache).expect_err("no view");
    assert!(why.contains("native GPU code"), "{why}");
    assert!(super::rpf9_scan::opens_as_bytes(&cache));
}
