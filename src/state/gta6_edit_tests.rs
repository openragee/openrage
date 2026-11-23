use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::authoring::{FormatChoice, TextureBuildOptions};
use or_io::codec::texture::RgbaImage;
use or_io::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Form, Rpf9Sections};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{read_dictionary, read_drawable, ParameterValue, Resource};

use super::*;
use crate::state::opening_an_rpf9_archive::{
    row_at, rows, rpf9_head, scanned, settle, written, zlib,
};
use crate::state::textures::TextureSource as Source;

const V: u64 = 0x5000_0000;
const SHADER_HASH: u32 = 0xABCD_0123;
const CONSTANT_HASH: u32 = 0x3333_0001;
const KEY: u64 = 0x1122_3344_5566_7788;
/// Where the drawable body puts its constant's two words and its one texture key.
const WORDS_AT: usize = 0x208;
const KEY_AT: usize = 0x2A8;

fn put(v: &mut [u8], at: usize, bytes: &[u8]) {
    v[at..at + bytes.len()].copy_from_slice(bytes);
}

fn ptr(v: &mut [u8], at: usize, to: usize) {
    put(v, at, &(V + to as u64).to_le_bytes());
}

/// The virtual section of a GTA VI drawable of no model and one shader: a texture slot bound to
/// [`KEY`] and a constant of two words; its block map at 0x3E0, one virtual block.
pub(in crate::state) fn drawable_body() -> Vec<u8> {
    let mut v = vec![0u8; 0x400];
    ptr(&mut v, 0x08, 0x3E0);
    v[0x3E8] = 1;
    ptr(&mut v, 0x10, 0x100);
    for (i, x) in [0.5f32, 0.5, 0.5, 2.0, -1.0, -1.0, -1.0].iter().enumerate() {
        put(&mut v, 0x20 + 4 * i, &x.to_le_bytes());
    }
    put(
        &mut v,
        0x40,
        &[1.0f32, 1.0, 1.0].map(f32::to_le_bytes).concat(),
    );
    ptr(&mut v, 0x50, 0x140);
    put(&mut v, 0x7C, &50.0f32.to_le_bytes());
    ptr(&mut v, 0xA8, 0x150);
    put(&mut v, 0x150, b"edit#dr\0");
    // The shader group, its one shader, the parameter infos, the block and the texture binding.
    ptr(&mut v, 0x110, 0x160);
    put(&mut v, 0x118, &1u16.to_le_bytes());
    ptr(&mut v, 0x160, 0x180);
    put(&mut v, 0x180, &SHADER_HASH.to_le_bytes());
    ptr(&mut v, 0x188, 0x200);
    ptr(&mut v, 0x190, 0x218);
    ptr(&mut v, 0x1A0, 0x1C0);
    put(&mut v, 0x1C0, &[1, 1, 0, 0, 2, 0, 0, 8]);
    put(&mut v, 0x1C8, &0x1111_0001u32.to_le_bytes());
    put(&mut v, 0x1D0, &CONSTANT_HASH.to_le_bytes());
    put(&mut v, 0x1D4, &(3u32 | 2 << 22).to_le_bytes());
    ptr(&mut v, 0x200, WORDS_AT);
    put(
        &mut v,
        WORDS_AT,
        &[0.25f32, 0.75].map(f32::to_le_bytes).concat(),
    );
    ptr(&mut v, 0x218, 0x240);
    ptr(&mut v, 0x240 + 0x30, KEY_AT - 8);
    put(&mut v, KEY_AT, &KEY.to_le_bytes());
    v
}

fn finished(state: &mut AppState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.extract_jobs.is_running() {
        state.poll_extract_job();
        assert!(Instant::now() < deadline, "the save never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The sections a read at offset 0 of the file at `path` returns.
fn read_back(path: &Path) -> Rpf9Sections {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (find, read) = read_rpf9_body(Cursor::new(bytes), 0, u64::MAX).expect("it reads");
    Rpf9Sections::of(&find, &read).expect("a resource")
}

/// The document `path` opens to, other than `not`.
fn opened(state: &mut AppState, path: PathBuf, not: DocumentId) -> DocumentId {
    state.load_file(path);
    settle(state);
    (state.active_document)
        .filter(|&d| d != not)
        .unwrap_or_else(|| panic!("opened: {}", state.error_dialog_message))
}

/// The texture key of `name`, which the app then knows by that name.
fn known(name: &str) -> u64 {
    let key = or_io::gta6::hash::name_hash64(name.as_bytes());
    super::super::ttpf_document::learn_named([(key, name.to_string())]);
    key
}

fn shader(state: &AppState, doc: DocumentId) -> Shader {
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc).map(|d| &d.payload) else {
        panic!("a GTA VI body");
    };
    let Resource::Drawable(d) = &**resource else {
        panic!("a drawable");
    };
    d.shaders[0].clone()
}

#[test]
fn a_material_edit_undoes_and_saves_as_a_loose_resource_that_reads_back_to_it() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut bytes = rpf9_head(1, 0x1A0);
    bytes.resize(0x1010, 0);
    bytes.extend(zlib(&drawable_body()));
    let mut state = AppState::new();
    let archive = scanned(&mut state, written(dir.path(), &bytes));
    state.open_scanned_body(archive, row_at(&rows(&state, archive), 0x1010));
    settle(&mut state);
    let doc = (state.active_document)
        .filter(|&d| d != archive)
        .unwrap_or_else(|| panic!("the drawable: {}", state.error_dialog_message));
    let opened_as = shader(&state, doc);
    assert_eq!(opened_as.texture_keys, [Some(KEY)]);

    // Nothing edited: Save As copies the body, whose sections are the ones stored.
    let copy = dir.path().join("copy.rsc");
    assert!(state.save_active_document_as(copy.clone()));
    finished(&mut state);
    let said: Vec<_> = state.toasts.iter().map(|t| t.message.clone()).collect();
    assert!(copy.exists(), "{said:?} {}", state.error_dialog_message);
    assert_eq!(read_back(&copy).virtual_section, drawable_body());

    let words = vec![2.5f32.to_bits(), (-4.0f32).to_bits()];
    let constant = MaterialEdit::Constant {
        parameter: 1,
        words: words.clone(),
    };
    // A slot's key record states its key's name hash, so a bound slot takes a key the app names.
    let other = known("edit_test_rebound");
    let bind = MaterialEdit::TextureKey {
        slot: 0,
        key: other,
    };
    let unnamed = MaterialEdit::TextureKey {
        slot: 0,
        key: other ^ 1,
    };
    assert!(state.edit_gta6_material(doc, None, 0, unnamed).is_err());
    state
        .edit_gta6_material(doc, None, 0, constant)
        .expect("a constant of its size");
    state
        .edit_gta6_material(doc, None, 0, bind)
        .expect("a bound slot");
    let edited = shader(&state, doc);
    assert_eq!(edited.texture_keys, [Some(other)]);
    assert!(
        matches!(&edited.parameters[1].value, ParameterValue::Constant { words: w, .. } if *w == words)
    );

    // Refused: another size, an empty slot, a body another holder reads.
    let longer = MaterialEdit::Constant {
        parameter: 1,
        words: vec![0; 3],
    };
    assert!(state.edit_gta6_material(doc, None, 0, longer).is_err());
    let texture = MaterialEdit::Constant {
        parameter: 0,
        words: vec![0],
    };
    assert!(state.edit_gta6_material(doc, None, 0, texture).is_err());
    let held = match &state.document(doc).expect("open").payload {
        DocPayload::Gta6Drawable(r) => Arc::clone(r),
        _ => panic!("a GTA VI body"),
    };
    let again = MaterialEdit::TextureKey { slot: 0, key: 1 };
    assert!(state.edit_gta6_material(doc, None, 0, again).is_err());
    drop(held);
    assert_eq!(
        shader(&state, doc),
        edited,
        "a refused edit changed nothing"
    );

    assert!(state.undo().is_some());
    assert_eq!(shader(&state, doc), opened_as, "undone, it is as it opened");
    assert!(state.redo().is_some());
    assert_eq!(shader(&state, doc), edited, "redone, an edit is lost");

    // Edited: Save As writes the values in place, each byte else as it was.
    let saved = dir.path().join("saved.rsc");
    assert!(state.save_active_document_as(saved.clone()));
    finished(&mut state);
    let said: Vec<_> = state.toasts.iter().map(|t| t.message.clone()).collect();
    assert!(saved.exists(), "{said:?} {}", state.error_dialog_message);
    let mut expected = drawable_body();
    put(
        &mut expected,
        WORDS_AT,
        &[2.5f32, -4.0].map(f32::to_le_bytes).concat(),
    );
    put(&mut expected, KEY_AT, &other.to_le_bytes());
    let sections = read_back(&saved);
    assert!(sections.virtual_section == expected);
    let read = read_drawable(&sections.virtual_section, Some(&sections.physical)).expect("reads");
    assert_eq!(read.shaders, std::slice::from_ref(&edited));
    let twice = dir.path().join("twice.rsc");
    assert!(state.save_active_document_as(twice.clone()));
    finished(&mut state);
    assert!(
        std::fs::read(&twice).ok() == std::fs::read(&saved).ok(),
        "a second save writes the same bytes"
    );

    // The loose resource opens to the edit, and an edit of it saves over its own file.
    let loose = opened(&mut state, saved.clone(), doc);
    assert_eq!(shader(&state, loose), edited);
    let back = MaterialEdit::Constant {
        parameter: 1,
        words: vec![0.25f32.to_bits(), 0.75f32.to_bits()],
    };
    state
        .edit_gta6_material(loose, None, 0, back)
        .expect("edits");
    state.active_document = Some(loose);
    assert!(state.document(loose).is_some_and(|d| d.dirty));
    assert!(state.save_active_document());
    finished(&mut state);
    assert!(
        state.document(loose).is_some_and(|d| !d.dirty),
        "saved over its own file, it is no longer unsaved"
    );
    put(
        &mut expected,
        WORDS_AT,
        &[0.25f32, 0.75].map(f32::to_le_bytes).concat(),
    );
    assert!(read_back(&saved).virtual_section == expected);
}

#[test]
fn a_key_is_typed_as_hash_and_sixteen_hex_digits_or_as_a_name() {
    assert_eq!(
        texture_key("hash_0FEDCBA987654321"),
        Some(0x0FED_CBA9_8765_4321)
    );
    assert_eq!(texture_key(" HASH_ff "), Some(0xFF));
    assert_eq!(texture_key("Hash_xyz"), None);
    assert_eq!(texture_key("zq_glow"), Some(name_hash64(b"zq_glow")));
    assert_eq!(texture_key("  "), None);
}

/// A loose file of a GTA VI dictionary of one 64×64 BC1 texture, "zq_paint", of one level, its
/// tiled pixels a pattern in the physical section.
pub(in crate::state) fn dictionary_file(dir: &Path) -> (PathBuf, Rpf9Sections) {
    let mut v = vec![0u8; 0x200];
    ptr(&mut v, 0x08, 0x1E0);
    put(&mut v, 0x18, &1u64.to_le_bytes());
    ptr(&mut v, 0x20, 0x40);
    put(&mut v, 0x28, &[1, 0, 1, 0]);
    ptr(&mut v, 0x30, 0x50);
    put(&mut v, 0x38, &[1, 0, 1, 0]);
    put(&mut v, 0x40, &name_hash32(b"zq_paint").to_le_bytes());
    ptr(&mut v, 0x50, 0x60);
    let t = 0x60;
    put(&mut v, t + 0x08, &(0x1000u32 / 8).to_le_bytes());
    put(&mut v, t + 0x0C, &8u16.to_le_bytes());
    put(&mut v, t + 0x0E, &8u16.to_le_bytes());
    put(&mut v, t + 0x18, &0x6000_0000u64.to_le_bytes());
    ptr(&mut v, t + 0x28, t + 0x50);
    ptr(&mut v, t + 0x30, 0x100);
    put(&mut v, t + 0x38, &[64, 0, 64, 0, 1, 0, 1, 71, 5, 0, 1]);
    put(&mut v, t + 0x46, &1u16.to_le_bytes());
    put(&mut v, t + 0x4C, &2u32.to_le_bytes());
    put(&mut v, 0x108, b"zq_paint\0");
    let physical = (0..0x1000u32).map(|i| (i * 13 + i / 7) as u8).collect();
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: v,
        physical,
    };
    let path = dir.join("paint.rsc");
    std::fs::write(&path, write_rpf9_resource(&sections).expect("writes")).expect("written");
    let stored = read_back(&path);
    (path, stored)
}

fn painted(colour: [u8; 4]) -> RgbaImage {
    let mut image = RgbaImage::new(64, 64);
    for y in 0..64 {
        for x in 0..64 {
            let c = if (x / 8 + y / 8) % 2 == 0 {
                colour
            } else {
                [0, 0, 0, 255]
            };
            image.set_pixel(x, y, c);
        }
    }
    image
}

fn pixels(state: &AppState, doc: DocumentId) -> Vec<u8> {
    let payload = &state.document(doc).expect("open").payload;
    let dictionary = crate::state::textures::legacy_dictionary(payload).expect("a dictionary");
    dictionary
        .get("zq_paint")
        .expect("its texture")
        .backing_store
        .clone()
}

#[test]
fn a_scanned_dictionary_texture_is_replaced_in_its_size_and_saves_as_a_loose_resource() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, stored) = dictionary_file(dir.path());
    let mut state = AppState::new();
    let doc = opened(&mut state, path, 0);
    assert!(is_scanned_dictionary(state.document(doc).expect("open")));
    assert!(state.texture_dictionary_is_editable(doc, Source::Document));
    let before = pixels(&state, doc);
    let options = TextureBuildOptions::default();
    let auto = FormatChoice::Automatic;

    state
        .import_texture_over(
            doc,
            Source::Document,
            "zq_paint",
            &painted([255, 0, 0, 255]),
            auto,
            &options,
        )
        .expect("an image of its size");
    let replaced = pixels(&state, doc);
    assert!(replaced != before && replaced.len() == before.len());

    let saved = dir.path().join("saved.rsc");
    state.active_document = Some(doc);
    assert!(state.save_active_document_as(saved.clone()));
    finished(&mut state);
    let sections = read_back(&saved);
    assert!(
        sections.virtual_section == stored.virtual_section,
        "the virtual section is left alone"
    );
    assert!(sections.physical != stored.physical);
    let dictionary =
        read_dictionary(&sections.virtual_section, Some(&sections.physical)).expect("reads");
    let [(_, Ok(or_io::gta6::headerless::Entry::Texture(t)))] = dictionary.entries.as_slice()
    else {
        panic!("one texture");
    };
    assert!(
        t.texture.as_ref().expect("reads").pixels == replaced,
        "it reads back as replaced"
    );

    // Undone, Save As copies the body as it is stored.
    assert!(state.undo().is_some());
    assert_eq!(pixels(&state, doc), before);
    let copy = dir.path().join("copy.rsc");
    assert!(state.save_active_document_as(copy.clone()));
    finished(&mut state);
    assert!(read_back(&copy) == stored);
}

#[test]
fn a_drawable_edited_in_place_still_poses_and_draws_the_base_colour_its_edited_key_binds() {
    use glam::{Mat4, Vec3};
    use or_io::gta6::headerless::{
        Bone, Drawable, Geometry, Model, Palette, Parameter, Skeleton, Vertices,
    };
    let _serial = crate::ui::theme::serial();
    let bone = |name: &str, t: [f32; 3]| Bone {
        name: name.to_string(),
        hash: name_hash32(name.as_bytes()),
        parent: None,
        rotation: [0.0, 0.0, 0.0, 1.0],
        translation: t,
        scale: [1.0; 3],
        inverse_bind: [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            t.map(|a| -a),
        ],
    };
    let skeleton = Skeleton {
        bones: vec![
            bone("SKEL_ROOT", [0.0, 0.0, 1.0]),
            bone("SKEL_Head", [0.2, -0.1, 1.6]),
        ],
    };
    let triangle = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let geometry = Geometry {
        vertex_data: Vec::new(),
        index_data: Vec::new(),
        shader: 0,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: Ok(triangle.clone()),
        vertices: Vertices {
            weights: vec![(16, vec![[1.0, 0.0, 0.0, 0.0]; 3])],
            joints: vec![(20, vec![[0; 4]; 3])],
            ..Vertices::default()
        },
    };
    let (normal, diffuse, edited) = (
        0x0A0A_0A0A_0A0A_0A0Au64,
        0x0D1F_0D1Fu64,
        known("edit_test_posed"),
    );
    let drawable = Drawable {
        name: "posed#dr".to_string(),
        box_min: [0.0; 3],
        box_max: [1.0, 1.0, 0.0],
        shaders: vec![Shader {
            name_hash: SHADER_HASH,
            texture_keys: vec![Some(normal), Some(diffuse)],
            parameters: vec![
                Parameter {
                    hash: 0x1E77_BF4C,
                    value: ParameterValue::Texture { slot: 0 },
                },
                Parameter {
                    hash: name_hash32(b"DiffuseTex"),
                    value: ParameterValue::Texture { slot: 1 },
                },
            ],
        }],
        levels: [
            Some(vec![Model {
                geometries: vec![Ok(geometry)],
                palette: Ok(Palette::Hashes(vec![name_hash32(b"SKEL_Head")])),
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
    let mut state = AppState::new();
    let payload = DocPayload::Gta6Drawable(Arc::new(Resource::Drawable(drawable)));
    let doc = state.insert_document(PathBuf::from("posed.rsc"), payload, None, None);
    let drawn = |state: &AppState| {
        let Some(DocPayload::Gta6Drawable(r)) = state.document(doc).map(|d| &d.payload) else {
            panic!("a GTA VI drawable");
        };
        let Resource::Drawable(d) = &**r else {
            panic!("a drawable");
        };
        let posing = Some((&skeleton, None));
        crate::render::gta6_drawable::meshes_at(d, 0, posing, None, &|_| None).remove(0)
    };
    let name = super::super::gta6_resource::texture_name;
    assert_eq!(drawn(&state).texture_name, Some(name(diffuse)));

    // The key DiffuseTex's slot binds is edited: the mesh draws the texture it names, still posed.
    let edit = MaterialEdit::TextureKey {
        slot: 1,
        key: edited,
    };
    state
        .edit_gta6_material(doc, None, 0, edit)
        .expect("the edit is made");
    let mut mesh = drawn(&state);
    assert_eq!(mesh.texture_name, Some(name(edited)));
    let skinning = mesh
        .skinning
        .as_ref()
        .expect("the geometry still joins the posed skeleton");
    assert_eq!(skinning.bones_used, [1]);
    let step = Vec3::new(0.5, 0.25, -0.75);
    assert!(mesh.apply_pose(
        &[Mat4::IDENTITY, Mat4::from_translation(step)],
        &mut Vec::new(),
        &mut Vec::new()
    ));
    let moved = Vec3::from(mesh.vertices[1].position);
    assert!(
        moved.distance(Vec3::from(triangle[1]) + step) < 1e-5,
        "{moved}"
    );

    state.active_document = Some(doc);
    state.undo().expect("a step to undo");
    assert_eq!(drawn(&state).texture_name, Some(name(diffuse)));
}

#[test]
fn an_empty_slot_takes_only_a_key_whose_name_hash_is_stated() {
    let mut s = Shader {
        name_hash: SHADER_HASH,
        texture_keys: vec![None],
        parameters: vec![Parameter {
            hash: 0x1111_0001,
            value: ParameterValue::Texture { slot: 0 },
        }],
    };
    let bind = MaterialEdit::TextureKey { slot: 0, key: 7 };
    assert!(apply(&mut s, bind.clone(), false).is_err());
    assert_eq!(s.texture_keys, [None], "a refused key changed nothing");
    apply(&mut s, bind, true).expect("a stated word");
    assert_eq!(s.texture_keys, [Some(7)]);
    let past = MaterialEdit::TextureKey { slot: 1, key: 7 };
    assert!(apply(&mut s, past, true).is_err(), "no such slot");

    // A bound slot's key record states its word too, so another key needs the same.
    let rebind = MaterialEdit::TextureKey { slot: 0, key: 9 };
    assert!(apply(&mut s, rebind.clone(), false).is_err());
    assert_eq!(s.texture_keys, [Some(7)], "a refused key changed nothing");
    let same = MaterialEdit::TextureKey { slot: 0, key: 7 };
    apply(&mut s, same, false).expect("its own key");
    apply(&mut s, rebind, true).expect("a stated word");
    assert_eq!(s.texture_keys, [Some(9)]);
}
