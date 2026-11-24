use std::path::Path;

use or_interchange::gltf::{self, Container};
use or_interchange::{Bone, Object};
use or_io::container::rpf::Rpf9Form;
use or_io::gta6::headerless::{Model, Palette, Vertices};

use super::*;

const V: u64 = 0x5000_0000;
const P: u64 = 0x6000_0000;
const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
/// Where the skeleton record, its bones, their inverse binds, their locals and names lie.
const SKELETON: usize = 0x500;
const RECORDS: usize = 0x580;
const INVERSES: usize = 0x620;
const LOCALS: usize = 0x6A0;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn pointer(bytes: &mut [u8], at: usize, tag: u64, target: usize) {
    put(bytes, at, &(tag + target as u64).to_le_bytes());
}

fn le(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A drawable of one geometry: [`TRIANGLE`] as three floats a vertex, its normals and texcoord 0,
/// and one meshlet; its block map an allocation of its own.
fn body() -> Rpf9Sections {
    let mut v = vec![0u8; 0x4C0];
    let at = |v: &mut Vec<u8>, from: usize, to: usize| pointer(v, from, V, to);
    let (group, shaders, shader, infos) = (0xF0, 0x130, 0x140, 0x180);
    let (level, models, model, geometries) = (0x190, 0x1A0, 0x1B0, 0x1F0);
    let (boxes, indices, geometry, vb) = (0x200, 0x220, 0x230, 0x280);
    let (declaration, record, ib, map) = (0x2D0, 0x410, 0x450, 0x4A0);
    at(&mut v, 0x08, map);
    (v[map + 8], v[map + 9]) = (1, 1);
    at(&mut v, 0x10, group);
    put(&mut v, 0x40, &le(&[1.0, 1.0, 0.0]));
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
    pointer(&mut v, vb + 0x18, P, 0);
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
    pointer(&mut v, ib + 0x18, P, 0xC0);
    Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: v,
        physical: p,
    }
}

/// [`body`] with a skeleton of its own at its 0x18: `SKEL_ROOT` and `SKEL_Head` under it.
fn skinned_body() -> Rpf9Sections {
    let mut s = body();
    let v = &mut s.virtual_section;
    v.resize(0x780, 0);
    pointer(v, 0x18, V, SKELETON);
    pointer(v, SKELETON + 0x28, V, RECORDS);
    pointer(v, SKELETON + 0x30, V, INVERSES);
    pointer(v, SKELETON + 0x38, V, LOCALS);
    put(v, SKELETON + 0x6C, &[2, 0, 2, 0]);
    let bones = [("SKEL_ROOT", -1i16, 0x720usize), ("SKEL_Head", 0, 0x740)];
    for (i, (name, parent, text)) in bones.into_iter().enumerate() {
        let at = RECORDS + 0x50 * i;
        put(v, at, &le(&[0.0, 0.0, 0.0, 1.0]));
        put(v, at + 0x1C, &name_hash32(name.as_bytes()).to_le_bytes());
        put(v, at + 0x20, &le(&[1.0; 3]));
        put(v, text, name.as_bytes());
        pointer(v, at + 0x30, V, text);
        put(v, at + 0x44, &(-1i16).to_le_bytes());
        put(v, at + 0x46, &parent.to_le_bytes());
        put(v, at + 0x48, &(i as u16).to_le_bytes());
    }
    s
}

/// The scene `rpf scan-export` makes of `s`.
fn exported(s: &Rpf9Sections) -> Scene {
    let drawable = drawable_of(s, Rpf9Role::Drawable, None).expect("the drawable reads");
    let names = crate::gta6_export::Names {
        effect: &|hash| format!("hash_{hash:08X}"),
        texture: &|key| format!("hash_{key:016X}"),
    };
    crate::gta6_export::scene(&drawable, None, "tri", false, &names)
}

fn only(s: &Rpf9Sections) -> Geometry {
    let mut d = drawable_of(s, Rpf9Role::Drawable, None).expect("the drawable reads");
    let mut models = d.levels[0].take().expect("level 0");
    models
        .remove(0)
        .geometries
        .remove(0)
        .expect("the geometry reads")
}

fn words(list: &[&Path]) -> Vec<String> {
    list.iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn model_replace_writes_a_new_loose_resource_that_reads_back_with_the_gltf_s_geometry() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let sections = body();
    let mut scene = exported(&sections);
    let p = &mut scene.objects[0].meshes[0].primitives[0];
    p.positions.push([1.0, 1.0, 0.0]);
    p.normals.push([0.0, 0.0, 1.0]);
    p.uvs[0].push([1.0, 1.0]);
    p.indices.extend([1, 3, 2]);
    let edited = p.clone();
    let (input, mesh, out) = (
        dir.path().join("tri.rsc"),
        dir.path().join("tri.glb"),
        dir.path().join("new.rsc"),
    );
    let resource = write_rpf9_resource(&sections).expect("written");
    fs::write(&input, &resource).expect("written");
    fs::write(&mesh, gltf::write(&scene, "tri", Container::Glb).primary).expect("written");
    let mut args = words(&[input.as_path(), mesh.as_path()]);
    args.extend(["--out".to_string(), out.to_string_lossy().into_owned()]);

    let lines = replace(&options(&args).expect("read")).expect("replaced");
    assert!(
        lines[1].contains("3 vertices in, 4 out, 2 triangles"),
        "{lines:?}"
    );
    assert_eq!(lines.last().map(String::as_str), Some(STRUCTURAL));
    let saved = fs::read(&out).expect("the new file");
    let (find, bytes) = read_rpf9_body(Cursor::new(&saved), 0, u64::MAX).expect("it reads");
    let g = only(&Rpf9Sections::of(&find, &bytes).expect("a resource"));
    assert_eq!(g.vertex_count, 4);
    assert_eq!(g.positions.as_ref().expect("positions"), &edited.positions);
    assert_eq!(g.vertices.normals, edited.normals);
    assert_eq!(
        fs::read(&input).expect("read"),
        resource,
        "the input is only read"
    );
}

#[test]
fn an_output_that_is_the_input_or_a_line_with_no_out_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let input = dir.path().join("tri.rsc");
    let resource = write_rpf9_resource(&body()).expect("written");
    fs::write(&input, &resource).expect("written");
    let mut args = words(&[input.as_path(), Path::new("tri.glb")]);
    let refused = options(&args).err().expect("refused");
    assert!(refused.contains("--out"), "{refused}");
    args.extend(["--out".to_string(), input.to_string_lossy().into_owned()]);
    let refused = replace(&options(&args).expect("read")).expect_err("refused");
    assert!(refused.contains("is the input"), "{refused}");
    assert_eq!(fs::read(&input).expect("read"), resource);
    args.extend(["--lods".to_string(), "sideways".to_string()]);
    let refused = options(&args).err().expect("refused");
    assert!(refused.contains("left or replaced"), "{refused}");
}

fn bone(name: &str) -> Bone {
    Bone {
        name: name.to_string(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
        ..Bone::default()
    }
}

#[test]
fn a_gltf_weighting_a_bone_the_drawable_s_own_skeleton_lacks_is_refused_as_a_skeleton_mismatch() {
    let sections = skinned_body();
    let mut scene = exported(&sections);
    let n = scene.objects[0].meshes[0].primitives[0].positions.len();
    let p = &mut scene.objects[0].meshes[0].primitives[0];
    (p.joints, p.weights) = (vec![[0; 4]; n], vec![[1.0, 0.0, 0.0, 0.0]; n]);
    scene.objects[0].bones = vec![bone("SKEL_Toe")];
    let how = ModelReplace::default();
    let refused =
        replace_gta6_model(&sections, Rpf9Role::Drawable, &scene, &how).expect_err("refused");
    assert!(
        refused.contains("skeleton mismatch") && refused.contains("SKEL_Toe"),
        "{refused}"
    );

    // A bone the skeleton holds, by name or by its name's hash, is no mismatch.
    scene.objects[0].bones = vec![bone("SKEL_Head")];
    let (same, _) =
        replace_gta6_model(&sections, Rpf9Role::Drawable, &scene, &how).expect("replaced");
    assert_eq!(same, sections);
    scene.objects[0].bones = Vec::new();
    let refused =
        replace_gta6_model(&sections, Rpf9Role::Drawable, &scene, &how).expect_err("refused");
    assert!(refused.contains("joint 0 names no bone"), "{refused}");
}

#[test]
fn a_material_the_gltf_gives_another_slot_is_named_and_the_slot_is_kept() {
    let sections = body();
    let mut scene = exported(&sections);
    let how = ModelReplace::default();
    scene.objects[0].meshes[0].primitives[0].material_name = Some("shader_0".into());
    let (_, lines) =
        replace_gta6_model(&sections, Rpf9Role::Drawable, &scene, &how).expect("replaced");
    assert!(
        !lines.iter().any(|l| l.contains("material slot")),
        "{lines:?}"
    );
    scene.objects[0].meshes[0].primitives[0].material_name = Some("Body".into());
    let (same, lines) =
        replace_gta6_model(&sections, Rpf9Role::Drawable, &scene, &how).expect("replaced");
    assert_eq!(same, sections, "no write changes a shader index");
    let line = "lod0/model0/geometry0: keeps its material slot, shader 0; the glTF's material \
                Body is not applied";
    assert!(lines.iter().any(|l| l == line), "{lines:?}");
}

/// A geometry of three vertices that draws with `shader`.
fn geometry(shader: u16) -> Result<Geometry, or_io::gta6::headerless::HeaderlessError> {
    Ok(Geometry {
        shader,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: Ok(TRIANGLE.to_vec()),
        vertices: Vertices::default(),
    })
}

fn model(shaders: &[u16]) -> Model {
    Model {
        geometries: shaders.iter().map(|s| geometry(*s)).collect(),
        palette: Ok(Palette::Hashes(Vec::new())),
        matrix_index: 0,
    }
}

#[test]
fn lower_levels_are_left_and_named_or_take_the_level_0_mesh_where_the_shader_matches() {
    let d = Drawable {
        name: "tri".into(),
        box_min: [0.0; 3],
        box_max: [1.0; 3],
        shaders: Vec::new(),
        levels: [
            Some(vec![model(&[0])]),
            Some(vec![model(&[0])]),
            Some(vec![model(&[1])]),
            Some(vec![model(&[0, 0])]),
        ],
        lod_distances: [9998.0; 4],
        bound: None,
        embedded: None,
        extra_skeletons: Vec::new(),
    };
    let scene = Scene {
        name: "tri".into(),
        objects: vec![Object {
            name: "tri".into(),
            meshes: vec![Mesh {
                name: "tri_lod0_model0".into(),
                lod: LodSlot::Level(0),
                primitives: vec![Primitive {
                    positions: TRIANGLE.to_vec(),
                    indices: vec![0, 1, 2],
                    ..Primitive::default()
                }],
            }],
            bones: Vec::new(),
        }],
        ..Scene::default()
    };
    assert_eq!(
        left_levels(&d, &scene),
        [
            "lod1, lod2, lod3 left as the drawable holds them: the glTF names none of their \
          geometries"
        ]
    );

    let (out, lines) = lower_levels(&d, &scene);
    assert_eq!(
        lines,
        [
            "lower levels: 2 geometries take the level-0 mesh at their model and place",
            "lod2/model0/geometry0 left as held: it draws with shader 1, the level-0 geometry \
             with shader 0",
            "lower levels: 1 geometries left as held, the glTF giving no level-0 mesh at their \
             model and place",
        ]
    );
    type Added<'a> = (&'a str, LodSlot, Option<(usize, usize)>);
    let added: Vec<Added> = (out.objects[0].meshes.iter())
        .skip(1)
        .flat_map(|m| {
            m.primitives
                .iter()
                .map(move |p| (m.name.as_str(), p.lod, p.stated_address))
        })
        .collect();
    assert_eq!(
        added,
        [
            ("tri_lod1_model0", LodSlot::Level(1), Some((0, 0))),
            ("tri_lod3_model0", LodSlot::Level(3), Some((0, 0))),
        ]
    );
    assert_eq!(
        left_levels(&d, &out),
        ["lod2 left as the drawable holds them: the glTF names none of their geometries"]
    );
}
