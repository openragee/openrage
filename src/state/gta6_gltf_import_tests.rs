use std::io::Cursor;
use std::time::{Duration, Instant};

use or_interchange::gltf::{self, Container};
use or_io::container::rpf::{write_rpf9_resource, Rpf9Form};
use or_io::gta6::headerless::{Geometry, Resource};

use super::*;
use crate::state::gltf::{Asked, Model as GltfModel};
use crate::state::opening_an_rpf9_archive::settle;

const V: u64 = 0x5000_0000;
const P: u64 = 0x6000_0000;
const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn le(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A drawable of one geometry: [`TRIANGLE`] as three floats a vertex, its normals and texcoord 0,
/// and one meshlet; its root's sphere and box words those of the other drawables the tests open.
fn body() -> Rpf9Sections {
    let mut v = vec![0u8; 0x4C0];
    let at = |v: &mut Vec<u8>, from: usize, to: usize| put(v, from, &(V + to as u64).to_le_bytes());
    let (group, shaders, shader, infos) = (0xF0, 0x130, 0x140, 0x180);
    let (level, models, model, geometries) = (0x190, 0x1A0, 0x1B0, 0x1F0);
    let (boxes, indices, geometry, vb) = (0x200, 0x220, 0x230, 0x280);
    let (declaration, record, ib, map) = (0x2D0, 0x410, 0x450, 0x4A0);
    at(&mut v, 0x08, map);
    (v[map + 8], v[map + 9]) = (1, 1);
    at(&mut v, 0x10, group);
    put(&mut v, 0x20, &le(&[0.5, 0.5, 0.5, 2.0, -1.0, -1.0, -1.0]));
    put(&mut v, 0x40, &le(&[1.0, 1.0, 1.0]));
    at(&mut v, 0x50, level);
    put(&mut v, 0x7C, &9998.0f32.to_le_bytes());
    put(&mut v, 0xE0, b"tri#dr\0");
    at(&mut v, 0xA8, 0xE0);
    at(&mut v, group + 0x10, shaders);
    put(&mut v, group + 0x18, &1u16.to_le_bytes());
    at(&mut v, shaders, shader);
    at(&mut v, shader + 0x20, infos);
    at(&mut v, level, models);
    put(&mut v, level + 8, &1u16.to_le_bytes());
    at(&mut v, models, model);
    at(&mut v, model + 0x08, geometries);
    put(&mut v, model + 0x10, &1u16.to_le_bytes());
    at(&mut v, model + 0x18, boxes);
    at(&mut v, model + 0x20, indices);
    put(&mut v, model + 0x36, &1u16.to_le_bytes());
    at(&mut v, geometries, geometry);
    put(&mut v, boxes + 0x10, &le(&[1.0, 1.0, 0.0, 1.0]));
    at(&mut v, geometry + 0x08, vb);
    at(&mut v, geometry + 0x40, record);

    // Positions at 0x60, normals at 0x84 and texcoord 0 at 0xA8, behind the slot table.
    let mut p = vec![0u8; 0xC0];
    let streams = [
        (0usize, 0usize, 0x60usize, 12u8, 6u8, 0x63u32),
        (4, 2, 0x84, 12, 6, 0x63),
        (28, 3, 0xA8, 8, 0x10, 0x42),
    ];
    for (channel, slot, offset, size, kind, code) in streams {
        let word = (offset as u32 / 2) << 8 | code;
        put(&mut p, 4 * slot, &word.to_le_bytes());
        put(
            &mut v,
            declaration + 4 * channel,
            &(offset as u32).to_le_bytes(),
        );
        v[declaration + 0xD0 + channel] = size;
        v[declaration + 0x104 + channel] = kind;
    }
    for k in (0..52).filter(|k| ![0, 4, 28].contains(k)) {
        put(&mut v, declaration + 4 * k, &0x60u32.to_le_bytes());
    }
    let counts = 1u64 | 32 << 2 | 3 << 10;
    put(&mut v, declaration + 0x138, &counts.to_le_bytes());
    put(&mut p, 0x5C, &0x200u32.to_le_bytes());
    put(&mut p, 0x60, &le(&TRIANGLE.concat()));
    put(&mut p, 0x84, &le(&[[0.0f32, 0.0, 1.0]; 3].concat()));
    put(&mut p, 0xA8, &le(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
    put(&mut v, vb + 0x08, &0x30u32.to_le_bytes());
    put(&mut v, vb + 0x0C, &4u16.to_le_bytes());
    put(&mut v, vb + 0x0E, &9u16.to_le_bytes());
    put(&mut v, vb + 0x10, &0x1601u32.to_le_bytes());
    put(&mut v, vb + 0x18, &P.to_le_bytes());
    at(&mut v, vb + 0x40, declaration);

    // One meshlet of one triangle over a list of three vertices: roundup4(3 + 4 + 1 + 2) words.
    let words: [u32; 12] = [
        0x8000_0001,
        1,
        3,
        0,
        0,
        0,
        0,
        1 << 7 | 2 << 14,
        1 << 16,
        2,
        0,
        0,
    ];
    p.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    at(&mut v, record, ib);
    put(&mut v, record + 0x08, &1u16.to_le_bytes());
    put(&mut v, record + 0x0A, &1u16.to_le_bytes());
    put(&mut v, record + 0x0C, &(3u32 << 8).to_le_bytes());
    put(&mut v, ib + 0x08, &12u32.to_le_bytes());
    put(&mut v, ib + 0x0C, &4u16.to_le_bytes());
    put(&mut v, ib + 0x0E, &8u16.to_le_bytes());
    put(&mut v, ib + 0x10, &0x1380u32.to_le_bytes());
    put(&mut v, ib + 0x18, &(P + 0xC0).to_le_bytes());
    Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: v,
        physical: p,
    }
}

/// [`body`] with a level 1 of its own: a copy of level 0's objects, from the level to its index
/// buffer's record, placed past the block map over a copy of the physical section.
fn two_levels() -> Rpf9Sections {
    let mut s = body();
    let (from, to) = (0x190usize, 0x4A0usize);
    let (shift, physical) = (s.virtual_section.len() - from, s.physical.len());
    let mut copy = s.virtual_section[from..to].to_vec();
    for k in (0..copy.len()).step_by(8) {
        let word = u64::from_le_bytes(copy[k..k + 8].try_into().expect("eight bytes"));
        let target = (word & 0x0FFF_FFFF) as usize;
        let moved = match word >> 28 {
            5 if (from..to).contains(&target) => word + shift as u64,
            6 => word + physical as u64,
            _ => continue,
        };
        put(&mut copy, k, &moved.to_le_bytes());
    }
    let level = s.virtual_section.len();
    s.virtual_section.extend(copy);
    put(
        &mut s.virtual_section,
        0x58,
        &(V + level as u64).to_le_bytes(),
    );
    let bytes = s.physical.clone();
    s.physical.extend(bytes);
    s
}

/// The vertex count of the one geometry at `level` of the drawable `payload` shows.
fn count_at(payload: &DocPayload, level: usize) -> u32 {
    let DocPayload::Gta6Drawable(r) = payload else {
        panic!("a GTA VI body");
    };
    let Resource::Drawable(d) = &**r else {
        panic!("a drawable");
    };
    let models = d.levels[level].as_ref().expect("the level");
    let g = models[0].geometries[0]
        .as_ref()
        .expect("the geometry reads");
    g.vertex_count
}

/// The one geometry of the drawable `sections` hold.
fn only(sections: &Rpf9Sections) -> Geometry {
    let (v, p) = (
        &sections.virtual_section,
        Some(sections.physical.as_slice()),
    );
    let mut d = read_drawable(v, p).expect("the drawable reads");
    let mut models = d.levels[0].take().expect("level 0");
    models
        .remove(0)
        .geometries
        .remove(0)
        .expect("the geometry reads")
}

/// The vertex count of the geometry the document `doc` shows.
fn shown(state: &AppState, doc: DocumentId) -> u32 {
    let Some(DocPayload::Gta6Drawable(r)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a GTA VI body");
    };
    let Resource::Drawable(d) = &**r else {
        panic!("a drawable");
    };
    let models = d.levels[0].as_ref().expect("level 0");
    let g = models[0].geometries[0]
        .as_ref()
        .expect("the geometry reads");
    g.vertex_count
}

/// The question's answer once its worker ends.
fn worked_out(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        state.poll_gltf();
        if let Some(Asked::Import(asked)) = &state.gltf.asked {
            if asked.made.is_some() {
                return;
            }
        }
        assert!(Instant::now() < deadline, "the import never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn finished(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(Instant::now() < deadline, "the save never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_gltf_replaces_a_gta_vi_drawable_s_geometry_as_a_step_that_undoes_and_saves() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("tri.rsc");
    let sections = body();
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path.clone());
    settle(&mut state);
    let doc =
        (state.active_document).unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));
    assert_eq!(state.gltf_import_into(GltfModel::Document(doc)), Ok(doc));

    // The drawable's own glTF, a fourth vertex and a second triangle added.
    let drawable = read_drawable(&sections.virtual_section, Some(&sections.physical))
        .expect("the drawable reads");
    let names = or_cli::Gta6Names {
        effect: &|hash| format!("hash_{hash:08X}"),
        texture: &|key| format!("hash_{key:016X}"),
    };
    let mut scene = or_cli::gta6_scene(&drawable, None, "tri", false, &names);
    let p = &mut scene.objects[0].meshes[0].primitives[0];
    p.positions.push([-1.0, -1.0, -1.0]);
    p.normals.push([0.0, 0.0, 1.0]);
    p.uvs[0].push([1.0, 1.0]);
    p.indices.extend([1, 3, 2]);
    let edited = p.clone();
    let mesh = dir.path().join("tri.glb");
    std::fs::write(&mesh, gltf::write(&scene, "tri", Container::Glb).primary).expect("written");

    state.import_gltf(doc, mesh.clone());
    worked_out(&mut state);
    let Some(Asked::Import(asked)) = &state.gltf.asked else {
        panic!("the import's question");
    };
    assert!(asked.is_gta6());
    let Some(Ok((_, report))) = &asked.made else {
        panic!(
            "worked out: {:?}",
            asked.made.as_ref().map(|m| m.as_ref().err())
        );
    };
    assert!(report.splits[0].contains("3 vertices in, 4 out, 2 triangles"));
    assert_eq!(shown(&state, doc), 3, "nothing changes before Replace");

    state.apply_gltf_import();
    assert_eq!(shown(&state, doc), 4);
    state.active_document = Some(doc);
    assert!(state.undo().is_some());
    assert_eq!(shown(&state, doc), 3, "undone, it is as it opened");
    assert!(state.redo().is_some());
    assert_eq!(shown(&state, doc), 4);

    // Saved over its own file: the geometry reads back as the mesh gave it.
    assert!(state.save_active_document());
    finished(&mut state);
    let bytes = std::fs::read(&path).expect("the saved file");
    let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
    let saved = Rpf9Sections::of(&find, &read).expect("a resource");
    let g = only(&saved);
    assert_eq!(g.vertex_count, 4);
    assert_eq!(g.positions.as_ref().expect("positions"), &edited.positions);
    let triangles: Vec<[u32; 3]> = edited.indices.as_chunks::<3>().0.to_vec();
    assert_eq!(g.triangles, triangles);

    // The body opened anew, as after a close that did not save, holds none of its imports.
    assert!(imports().contains_key(&path));
    super::super::rpf9_scan::read_body(&path, &path, 0).expect("it opens again");
    assert!(!imports().contains_key(&path));
}

#[test]
fn a_gltf_import_is_in_the_loose_resource_a_save_or_a_write_into_an_archive_carries() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("tri.rsc");
    let sections = body();
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path.clone());
    settle(&mut state);
    let doc =
        (state.active_document).unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));

    let drawable = read_drawable(&sections.virtual_section, Some(&sections.physical))
        .expect("the drawable reads");
    let names = or_cli::Gta6Names {
        effect: &|hash| format!("hash_{hash:08X}"),
        texture: &|key| format!("hash_{key:016X}"),
    };
    let mut scene = or_cli::gta6_scene(&drawable, None, "tri", false, &names);
    let p = &mut scene.objects[0].meshes[0].primitives[0];
    p.positions.push([-1.0, -1.0, -1.0]);
    p.normals.push([0.0, 0.0, 1.0]);
    p.uvs[0].push([1.0, 1.0]);
    p.indices.extend([1, 3, 2]);
    let mesh = dir.path().join("tri.glb");
    std::fs::write(&mesh, gltf::write(&scene, "tri", Container::Glb).primary).expect("written");
    state.import_gltf(doc, mesh);
    worked_out(&mut state);
    state.apply_gltf_import();

    // What the row hands a save, and a write into an archive, is the body with the import made.
    let Some(Ok(EditedSave::Body {
        archive,
        offset,
        edit,
    })) = state.edited_sections(doc)
    else {
        panic!("the import is an edit a save writes");
    };
    let loose = super::super::gta6_edit::written(&archive, offset, &*edit).expect("written");
    let (find, read) = read_rpf9_body(Cursor::new(&loose), 0, u64::MAX).expect("it reads");
    assert_eq!(
        only(&Rpf9Sections::of(&find, &read).expect("a resource")).vertex_count,
        4
    );

    state.active_document = Some(doc);
    assert!(state.save_active_document());
    finished(&mut state);
    let saved = std::fs::read(&path).expect("the saved file");
    assert_eq!(saved, loose, "a save writes that same loose resource");
}

#[test]
fn a_mesh_without_normals_at_another_count_is_refused_until_the_fill_is_ticked_and_saves_filled() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("tri.rsc");
    let sections = body();
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path.clone());
    settle(&mut state);
    let doc =
        (state.active_document).unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));

    // A fourth vertex, and no normals at all.
    let drawable = read_drawable(&sections.virtual_section, Some(&sections.physical))
        .expect("the drawable reads");
    let names = or_cli::Gta6Names {
        effect: &|hash| format!("hash_{hash:08X}"),
        texture: &|key| format!("hash_{key:016X}"),
    };
    let mut scene = or_cli::gta6_scene(&drawable, None, "tri", false, &names);
    let p = &mut scene.objects[0].meshes[0].primitives[0];
    p.positions.push([-1.0, -1.0, -1.0]);
    p.uvs[0].push([1.0, 1.0]);
    p.indices.extend([1, 3, 2]);
    p.normals.clear();
    let mesh = dir.path().join("tri.glb");
    std::fs::write(&mesh, gltf::write(&scene, "tri", Container::Glb).primary).expect("written");

    state.import_gltf(doc, mesh);
    worked_out(&mut state);
    let Some(Asked::Import(asked)) = &mut state.gltf.asked else {
        panic!("the import's question");
    };
    assert!(
        matches!(&asked.made, Some(Err(why)) if why.contains("channel 4")),
        "refused while the fill is off"
    );
    asked.choice.fill_missing = true;
    state.rerun_gltf();
    worked_out(&mut state);
    let Some(Asked::Import(asked)) = &state.gltf.asked else {
        panic!("the import's question");
    };
    let Some(Ok((_, report))) = &asked.made else {
        panic!(
            "filled: {:?}",
            asked.made.as_ref().map(|m| m.as_ref().err())
        );
    };
    let line = &report.splits[0];
    assert!(line.contains("channels 4 filled"), "{line}");

    state.apply_gltf_import();
    assert_eq!(shown(&state, doc), 4);
    state.active_document = Some(doc);
    assert!(state.save_active_document());
    finished(&mut state);
    let bytes = std::fs::read(&path).expect("the saved file");
    let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
    let saved = Rpf9Sections::of(&find, &read).expect("a resource");
    let g = only(&saved);
    assert_eq!(g.vertex_count, 4);
    assert_eq!(g.vertices.normals, vec![[0.0, 0.0, 1.0]; 4]);
}

#[test]
fn the_lower_levels_are_left_unless_asked_and_then_take_the_level_0_mesh_through_the_save() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("tri.rsc");
    let sections = two_levels();
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let mut state = AppState::new();
    state.load_file(path.clone());
    settle(&mut state);
    let doc =
        (state.active_document).unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message));

    // Level 0's own glTF, a fourth vertex and a second triangle added; it names no level 1.
    let drawable = read_drawable(&sections.virtual_section, Some(&sections.physical))
        .expect("the drawable reads");
    let names = or_cli::Gta6Names {
        effect: &|hash| format!("hash_{hash:08X}"),
        texture: &|key| format!("hash_{key:016X}"),
    };
    let mut scene = or_cli::gta6_scene(&drawable, None, "tri", false, &names);
    let p = &mut scene.objects[0].meshes[0].primitives[0];
    p.positions.push([-1.0, -1.0, -1.0]);
    p.normals.push([0.0, 0.0, 1.0]);
    p.uvs[0].push([1.0, 1.0]);
    p.indices.extend([1, 3, 2]);
    let mesh = dir.path().join("tri.glb");
    std::fs::write(&mesh, gltf::write(&scene, "tri", Container::Glb).primary).expect("written");

    state.import_gltf(doc, mesh);
    worked_out(&mut state);
    let Some(Asked::Import(asked)) = &mut state.gltf.asked else {
        panic!("the import's question");
    };
    assert!(
        !asked.choice.lower_lods,
        "the lower levels are left unless asked"
    );
    let Some(Ok((payload, report))) = &asked.made else {
        panic!(
            "worked out: {:?}",
            asked.made.as_ref().map(|m| m.as_ref().err())
        );
    };
    assert_eq!((count_at(payload, 0), count_at(payload, 1)), (4, 3));
    let left = report
        .splits
        .iter()
        .any(|l| l.starts_with("lod1 left as the drawable holds"));
    assert!(left, "{:?}", report.splits);

    asked.choice.lower_lods = true;
    state.rerun_gltf();
    worked_out(&mut state);
    let Some(Asked::Import(asked)) = &state.gltf.asked else {
        panic!("the import's question");
    };
    let Some(Ok((payload, report))) = &asked.made else {
        panic!(
            "worked out: {:?}",
            asked.made.as_ref().map(|m| m.as_ref().err())
        );
    };
    assert_eq!((count_at(payload, 0), count_at(payload, 1)), (4, 4));
    let taken = "lower levels: 1 geometries take the level-0 mesh at their model and place";
    assert!(
        report.splits.iter().any(|l| l == taken),
        "{:?}",
        report.splits
    );

    state.apply_gltf_import();
    let shown =
        |state: &AppState, level| count_at(&state.document(doc).expect("open").payload, level);
    assert_eq!((shown(&state, 0), shown(&state, 1)), (4, 4));

    // The save makes the import again as it was asked, its lower level replaced too.
    state.active_document = Some(doc);
    assert!(state.save_active_document());
    finished(&mut state);
    let bytes = std::fs::read(&path).expect("the saved file");
    let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
    let saved = Rpf9Sections::of(&find, &read).expect("a resource");
    let d = read_drawable(&saved.virtual_section, Some(&saved.physical)).expect("it reads");
    let level1 = d.levels[1].as_ref().expect("level 1");
    let g = level1[0].geometries[0]
        .as_ref()
        .expect("the geometry reads");
    assert_eq!(g.vertex_count, 4);
}
