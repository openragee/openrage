//! The source of a `.ybn`: a bound of every kind built by hand goes to a folder and compiles back to its
//! canonical save, a value the document states otherwise is set, a triangle added to the glb is held by
//! a bound that validates, a source taken from the game is the game's file to a build until a polygon
//! changes, and a document that names what is not there is refused with the name. Needs no game file.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use or_interchange::gltf::read::load;
use or_interchange::scene::Scene;
use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::decl::{Aligned, File, Open, Plain, Raw, Shared};
use or_io::desc::Image;
use or_io::gta5::bounds::build::{self, BoundMesh, COMPOSITE_ITEM_THRESHOLD};
use or_io::gta5::bounds::*;
use or_io::gta5::{Mat34V, Vec3V, Vec4V};
use or_source::asset::{sha256_hex, Source};
use or_source::bounds::{self, BASE, KIND, MODEL, RULES};
use or_source::cook::{cook_source, describe, Cooker, Ignored, Recook};
use or_source::textures::Extraction;
use serde_json::Value;

const NAME: &str = "hull";

fn v4(x: f32, y: f32, z: f32, w: f32) -> Vec4V {
    Vec4V {
        x,
        y,
        z,
        w,
        ..Default::default()
    }
}

/// A header whose every word differs from a default's, so a word a source lost shows.
fn header(ty: BoundType, centre: [f32; 3], half: [f32; 3], margin: f32) -> PhBound {
    let max: [f32; 3] = std::array::from_fn(|a| centre[a] + half[a]);
    let mut h = PhBound {
        r#type: Open::new(ty),
        flags: 3,
        part_index: 7,
        radius_around_centroid: half.iter().map(|h| h * h).sum::<f32>().sqrt(),
        bounding_box_max_xyz_margin_w: v4(max[0], max[1], max[2], margin),
        centroid_offset_xyz_material_id0_w: v4(
            centre[0],
            centre[1],
            centre[2],
            f32::from_bits(0x37),
        ),
        cg_offset_xyz_material_id1_w: v4(
            centre[0],
            centre[1] + 0.125,
            centre[2],
            f32::from_bits(0x1_0002),
        ),
        volume_distribution: v4(0.5, 0.25, 0.125, 2.0),
        ..Default::default()
    };
    h.set_bounding_box_min(std::array::from_fn(|a| centre[a] - half[a]));
    h
}

const AT: [f32; 3] = [1200.5, -340.25, 31.0];

fn sphere() -> PhBoundAny {
    PhBoundAny::PhBoundSphere(PhBoundSphere {
        base: header(BoundType::Sphere, AT, [0.5; 3], 0.5),
        ..Default::default()
    })
}

fn box_bound() -> PhBoundAny {
    PhBoundAny::PhBoundBox(PhBoundBox {
        base: header(BoundType::Box, AT, [2.0, 1.0, 0.5], 0.04),
        ..Default::default()
    })
}

fn capsule() -> PhBoundAny {
    PhBoundAny::PhBoundCapsule(PhBoundCapsule {
        base: header(BoundType::Capsule, AT, [0.25, 1.25, 0.25], 0.25),
        capsule_half_height: 1.0,
        ..Default::default()
    })
}

fn disc() -> PhBoundAny {
    PhBoundAny::PhBoundDisc(PhBoundDisc {
        base: header(BoundType::Disc, AT, [0.05, 0.75, 0.75], 0.05),
        ..Default::default()
    })
}

fn cylinder() -> PhBoundAny {
    PhBoundAny::PhBoundCylinder(PhBoundCylinder {
        base: header(BoundType::Cylinder, AT, [0.5, 1.5, 0.5], 0.04),
        ..Default::default()
    })
}

fn plane() -> PhBoundAny {
    PhBoundAny::PhBoundPlane(PhBoundPlane {
        base: header(BoundType::Plane, AT, [10.0, 10.0, 0.0], 0.0),
        ..Default::default()
    })
}

/// A bound of a type no layout is declared for, which is its header and nothing more.
fn header_only() -> PhBoundAny {
    PhBoundAny::Base(header(BoundType::Triangle, AT, [1.0; 3], 0.0))
}

const MATERIALS: [u64; 2] = [0x11, 0x0000_0001_0000_0022];

/// A slab of twelve triangles in two materials, at a place where a float is a few quanta wide.
fn slab() -> BoundMesh {
    let corner = |i: usize| -> [f32; 3] {
        [
            AT[0] + if i & 1 == 0 { -8.0 } else { 8.0 },
            AT[1] + if i & 2 == 0 { -4.0 } else { 4.0 },
            AT[2] + if i & 4 == 0 { -1.0 } else { 1.0 },
        ]
    };
    let triangles: Vec<[u16; 3]> = vec![
        [0, 1, 3],
        [0, 3, 2],
        [4, 7, 5],
        [4, 6, 7],
        [0, 5, 1],
        [0, 4, 5],
        [2, 3, 7],
        [2, 7, 6],
        [0, 2, 6],
        [0, 6, 4],
        [1, 5, 7],
        [1, 7, 3],
    ];
    BoundMesh {
        positions: (0..8).map(corner).collect(),
        triangle_materials: (0..triangles.len()).map(|i| (i % 2) as u8).collect(),
        triangles,
        materials: MATERIALS
            .iter()
            .map(|id| PhMaterialMgrId {
                id: *id,
                ..Default::default()
            })
            .collect(),
    }
}

/// The polygon the bytes of a primitive's record are.
fn polygon_of<R: Plain>(raw: &R) -> PhPolygon {
    let p = <Raw<PhPolygon> as Plain>::read_from(Plain::as_bytes(raw)).unwrap();
    PhPolygon {
        area: p.area,
        vertex_indices: p.vertex_indices,
        neighboring_polygons: p.neighboring_polygons,
        ..Default::default()
    }
}

/// One polygon of each kind that is no triangle, on vertices the slab holds.
fn primitives() -> Vec<PhPolygon> {
    let mut sphere = <Raw<PhPrimSphere> as Plain>::zeroed();
    sphere.prim_type = 1;
    sphere.center_index = 0;
    sphere.radius = 0.5;
    let mut capsule = <Raw<PhPrimCapsule> as Plain>::zeroed();
    capsule.prim_type = 2;
    capsule.end_index0 = 1;
    capsule.end_index1 = 2;
    capsule.radius = 0.25;
    let mut boxed = <Raw<PhPrimBox> as Plain>::zeroed();
    boxed.prim_type = 3;
    boxed.vertex_indices = [0, 3, 5, 6];
    let mut cylinder = <Raw<PhPrimCylinder> as Plain>::zeroed();
    cylinder.prim_type = 4;
    cylinder.end_index0 = 4;
    cylinder.end_index1 = 7;
    cylinder.radius = 0.125;
    vec![
        polygon_of(&sphere),
        polygon_of(&capsule),
        polygon_of(&boxed),
        polygon_of(&cylinder),
    ]
}

/// A geometry of the slab's triangles and one primitive of each kind, with two material colours.
fn geometry() -> PhBoundGeometry {
    let mut g = build::geometry(&slab()).unwrap();
    g.base.flags = 5;
    g.base.bounding_box_max_xyz_margin_w.w = 0.025;
    for (i, polygon) in primitives().into_iter().enumerate() {
        g.polygons.push(polygon);
        g.poly_mat_index_list.push((i % 2) as u8);
    }
    // A quantum twice the tightest, as a file may state one: building the vertices again would not arrive at it.
    for v in &mut g.compressed_vertices {
        (v.x, v.y, v.z) = (v.x / 2, v.y / 2, v.z / 2);
    }
    let f = &mut g.un_quantize_factor;
    (f.x, f.y, f.z) = (f.x * 2.0, f.y * 2.0, f.z * 2.0);
    // The box holds what the bound holds, the reach of a primitive included.
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in &g.polygons {
        let (lo, hi) = g.polygon_box(p).unwrap();
        min = std::array::from_fn(|a| min[a].min(lo[a]));
        max = std::array::from_fn(|a| max[a].max(hi[a]));
    }
    g.base.set_bounding_box_min(min);
    let v = &mut g.base.bounding_box_max_xyz_margin_w;
    [v.x, v.y, v.z] = max;
    g.material_colors = [0xff10_2030u32, 0x8040_5060]
        .iter()
        .map(|color| Color32 {
            color: *color,
            ..Default::default()
        })
        .collect();
    g
}

fn bvh() -> PhBoundBVH {
    let mut bound = PhBoundBVH {
        base: geometry(),
        ..Default::default()
    };
    bound.base.base.r#type = Open::new(BoundType::Bvh);
    build::rebuild(&mut bound).unwrap();
    bound
}

fn matrix(translation: [f32; 3]) -> Mat34V {
    Mat34V {
        col0: v4(1.0, 0.0, 0.0, 0.0),
        col1: v4(0.0, 1.0, 0.0, 0.0),
        col2: v4(0.0, 0.0, 1.0, 0.0),
        col3: v4(translation[0], translation[1], translation[2], 0.0),
        ..Default::default()
    }
}

fn header_of(bound: &PhBoundAny) -> &PhBound {
    match bound {
        PhBoundAny::PhBoundSphere(b) => &b.base,
        PhBoundAny::PhBoundCapsule(b) => &b.base,
        PhBoundAny::PhBoundBox(b) => &b.base,
        PhBoundAny::PhBoundGeometry(b) => &b.base,
        PhBoundAny::PhBoundBVH(b) => &b.base.base,
        PhBoundAny::PhBoundComposite(b) => &b.base,
        PhBoundAny::PhBoundDisc(b) => &b.base,
        PhBoundAny::PhBoundCylinder(b) => &b.base,
        PhBoundAny::PhBoundPlane(b) => &b.base,
        PhBoundAny::Base(b) => b,
    }
}

/// A composite of `children`, each with its matrix, its box and its flags, a child that is null among them, and a tree over the boxes.
fn composite(children: Vec<Option<PhBoundAny>>) -> PhBoundAny {
    let vec3 = |[x, y, z]: [f32; 3]| Vec3V {
        x,
        y,
        z,
        ..Default::default()
    };
    let boxes: Vec<([f32; 3], [f32; 3])> = children
        .iter()
        .map(|child| match child {
            Some(child) => {
                let h = header_of(child);
                let max = &h.bounding_box_max_xyz_margin_w;
                (h.bounding_box_min(), [max.x, max.y, max.z])
            }
            None => (AT, AT),
        })
        .collect();
    let mut h = header(BoundType::Composite, AT, [9.0, 5.0, 2.0], 0.0);
    h.flags = 0;
    PhBoundAny::PhBoundComposite(PhBoundComposite {
        base: h,
        current_matrices: (0..children.len())
            .map(|i| matrix([i as f32 * 0.5, 0.0, 0.0]))
            .collect(),
        last_matrices: Shared::Same,
        local_box_min_maxs: boxes
            .iter()
            .map(|(min, max)| LocalBoxMinMax {
                min: vec3(*min),
                max: vec3(*max),
                ..Default::default()
            })
            .collect(),
        type_and_include_flags: (0..children.len())
            .map(|i| TypeAndIncludeFlags {
                type_flags: 0x40 + i as u32,
                include_flags: 0x00ff_0000 | i as u32,
                ..Default::default()
            })
            .collect(),
        owned_type_and_include_flags: Shared::Same,
        bvh_structure: Some(Box::new(
            build::build(&boxes, COMPOSITE_ITEM_THRESHOLD).unwrap().bvh,
        )),
        bounds: children
            .into_iter()
            .map(|child| child.map(Box::new))
            .collect(),
        ..Default::default()
    })
}

/// A composite of a tree bound, a sphere, nothing, a geometry and a capsule.
fn building() -> PhBoundAny {
    composite(vec![
        Some(PhBoundAny::PhBoundBVH(bvh())),
        Some(sphere()),
        None,
        Some(PhBoundAny::PhBoundGeometry(geometry())),
        Some(capsule()),
    ])
}

/// The building, with the matrices and the flags a composite may share held apart.
fn building_that_holds_its_runs_apart() -> PhBoundAny {
    let PhBoundAny::PhBoundComposite(mut composite) = building() else {
        panic!("not a composite");
    };
    composite.last_matrices = Shared::Own(
        (0..composite.bounds.len())
            .map(|i| matrix([0.0, i as f32 * 0.25, 1.0]))
            .collect(),
    );
    composite.owned_type_and_include_flags = Shared::Own(
        (0..composite.bounds.len())
            .map(|i| TypeAndIncludeFlags {
                type_flags: 0x80 + i as u32,
                include_flags: 0x0f00 | i as u32,
                ..Default::default()
            })
            .collect(),
    );
    PhBoundAny::PhBoundComposite(composite)
}

/// Every kind of bound a `.ybn` holds, by the name of its layout.
fn every_kind() -> Vec<(&'static str, PhBoundAny)> {
    vec![
        ("PhBoundSphere", sphere()),
        ("PhBoundCapsule", capsule()),
        ("PhBoundBox", box_bound()),
        ("PhBoundGeometry", PhBoundAny::PhBoundGeometry(geometry())),
        ("PhBoundBVH", PhBoundAny::PhBoundBVH(bvh())),
        ("PhBoundComposite", building()),
        ("PhBoundDisc", disc()),
        ("PhBoundCylinder", cylinder()),
        ("PhBoundPlane", plane()),
        ("PhBound", header_only()),
    ]
}

/// The file a bound is, and what opening and saving that file gives: the canonical save a source owes.
fn file_and_canonical(bound: &PhBoundAny) -> (Vec<u8>, Vec<u8>) {
    let file = bound.to_bytes().expect("the bound saves");
    let canonical = PhBoundAny::from_bytes(&file).unwrap().to_bytes().unwrap();
    (file, canonical)
}

fn extract(parent: &Path, file: &[u8]) -> PathBuf {
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    bounds::extract(file, NAME, parent, &how).unwrap_or_else(|why| panic!("{why:?}"))
}

fn compiled(dir: &Path) -> or_source::models::Compiled {
    bounds::compile(&Source::open(dir).unwrap()).unwrap_or_else(|why| panic!("{why:?}"))
}

/// Why the source in `dir` does not compile, as one text with the file each reason is in.
fn refusal(dir: &Path) -> String {
    let problems = bounds::compile(&Source::open(dir).unwrap()).expect_err("the source compiled");
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn document(dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join(RULES)).unwrap()).unwrap()
}

fn edit_document(dir: &Path, edit: impl FnOnce(&mut Value)) {
    let mut doc = document(dir);
    edit(&mut doc);
    fs::write(dir.join(RULES), serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

fn edit_glb(dir: &Path, edit: impl FnOnce(&mut Scene)) {
    common::edit_bounds_glb(dir, NAME, edit);
}

/// The game's own check of a saved bound, over the file's image.
fn validate(file: &[u8]) {
    let opened = Rsc7::open(file).unwrap();
    let aligned = Aligned::from_bytes(&opened.data);
    let image = Image::new(aligned.bytes(), opened.virtual_size as usize);
    or_io::gta5::bounds::validate(image, 0).expect("the compiled bound validates");
}

fn soup(bound: &PhBoundAny) -> &PhBoundGeometry {
    match bound {
        PhBoundAny::PhBoundGeometry(g) => g,
        PhBoundAny::PhBoundBVH(b) => &b.base,
        other => panic!("{other:?} holds no polygons"),
    }
}

fn child(bound: &PhBoundAny, i: usize) -> &PhBoundAny {
    match bound {
        PhBoundAny::PhBoundComposite(c) => c.bounds[i].as_deref().expect("a child"),
        other => panic!("{other:?} holds no children"),
    }
}

fn triangles(g: &PhBoundGeometry) -> usize {
    g.polygons.iter().filter(|p| p.primitive() == 0).count()
}

#[test]
fn a_bound_of_every_kind_goes_to_a_source_and_cooks_back_to_its_canonical_save() {
    for (kind, bound) in every_kind() {
        let work = tempfile::tempdir().unwrap();
        let (file, canonical) = file_and_canonical(&bound);
        let dir = extract(work.path(), &file);
        assert_eq!(
            dir.file_name().unwrap().to_string_lossy(),
            format!("{NAME}.{KIND}.src"),
            "{kind}"
        );
        assert_eq!(
            fs::read(dir.join(BASE)).unwrap(),
            file,
            "{kind}: the base is the file"
        );
        let manifest = Source::open(&dir).unwrap().manifest;
        assert_eq!(manifest.kind, "bounds");
        assert_eq!(manifest.extra["compile"]["base"], sha256_hex(&file));
        assert_eq!(document(&dir)["bound"]["type"], kind);

        let said = describe(&dir).unwrap_or_else(|why| panic!("{kind}: {why}"));
        assert_eq!(
            (said.kind.as_str(), said.extension.as_str()),
            ("bounds", "ybn")
        );
        let output = cook_source(&dir, &work.path().join("scratch"))
            .unwrap_or_else(|why| panic!("{kind}: {why:?}"));
        assert_eq!(output.file_name().unwrap(), "hull.ybn");
        assert!(
            fs::read(&output).unwrap() == canonical,
            "{kind}: the cook of an untouched source is not the canonical save of its file"
        );
        validate(&canonical);
    }
}

#[test]
fn the_glb_holds_a_node_for_each_bound_with_triangles_and_a_primitive_for_each_material() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&building()).0);
    let doc = document(&dir);
    let children = doc["bound"]["children"].as_array().unwrap();
    assert_eq!(children[0]["bound"]["node"], "hull_bound_child0");
    assert_eq!(children[3]["bound"]["node"], "hull_bound_child3");
    assert!(
        children[1]["bound"].get("node").is_none(),
        "a sphere holds no triangle"
    );

    let glb = dir.join(MODEL);
    let scene = load(&glb, &fs::read(&glb).unwrap()).unwrap();
    let meshes = &scene.objects[0].meshes;
    let names: Vec<&str> = meshes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["hull_bound_child0", "hull_bound_child3"]);
    let slab = slab();
    for mesh in meshes {
        let materials: Vec<Option<&str>> = mesh
            .primitives
            .iter()
            .map(|p| p.material_name.as_deref())
            .collect();
        let address = mesh.name.strip_prefix("hull_").unwrap();
        assert_eq!(
            materials,
            [
                Some(format!("{address}_material0").as_str()),
                Some(format!("{address}_material1").as_str())
            ]
        );
        let held: usize = mesh.primitives.iter().map(|p| p.indices.len() / 3).sum();
        assert_eq!(held, slab.triangles.len(), "{}", mesh.name);
    }

    // A bound with no triangle anywhere is a source with no glb.
    let lone = tempfile::tempdir().unwrap();
    let dir = extract(lone.path(), &file_and_canonical(&sphere()).0);
    assert!(!dir.join(MODEL).exists());
}

#[test]
fn the_document_states_what_a_glb_cannot_say_by_the_values_the_bound_holds() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&building()).0);
    let doc = document(&dir);
    let root = &doc["bound"];
    assert_eq!(root["type"], "PhBoundComposite");
    assert_eq!(root["part_index"], 7);
    assert_eq!(root["material_id0"], format!("{:#010x}", 0x37));
    assert_eq!(root["material_id1"], format!("{:#010x}", 0x1_0002));
    assert_eq!(root["volume_distribution"], "0.5 0.25 0.125 2.0");
    assert_eq!(
        root["bounding_box_max"],
        serde_json::json!([AT[0] + 9.0, AT[1] + 5.0, AT[2] + 2.0])
    );

    let children = root["children"].as_array().unwrap();
    assert_eq!(children.len(), 5);
    assert!(
        children[2]["bound"].is_null(),
        "a child that is not there is stated as null"
    );
    assert_eq!(
        children[1]["current_matrix"],
        serde_json::json!([
            "1.0 0.0 0.0 0.0",
            "0.0 1.0 0.0 0.0",
            "0.0 0.0 1.0 0.0",
            "0.5 0.0 0.0 0.0"
        ])
    );
    assert!(
        children[1].get("last_matrix").is_none(),
        "the matrices a composite shares with the current ones are not stated twice"
    );
    assert_eq!(children[4]["type_flags"], format!("{:#010x}", 0x44));
    assert_eq!(
        children[4]["include_flags"],
        format!("{:#010x}", 0x00ff_0004)
    );
    assert_eq!(children[4]["bound"]["capsule_half_height"], 1.0);
    assert_eq!(children[1]["bound"]["margin"], 0.5);

    let soup = &children[3]["bound"];
    assert_eq!(soup["margin"], 0.025);
    assert_eq!(soup["flags"], 5);
    assert_eq!(
        soup["materials"],
        serde_json::json!([
            { "name": "bound_child3_material0", "id": format!("{:#018x}", MATERIALS[0]) },
            { "name": "bound_child3_material1", "id": format!("{:#018x}", MATERIALS[1]) },
        ])
    );
    assert_eq!(
        soup["material_colors"],
        serde_json::json!(["0xff102030", "0x80405060"])
    );
    let primitives = soup["primitives"].as_array().unwrap();
    let kinds: Vec<&str> = primitives
        .iter()
        .map(|p| p["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["sphere", "capsule", "box", "cylinder"]);
    let corner = slab().positions;
    assert_eq!(primitives[0]["radius"], 0.5);
    assert_eq!(primitives[0]["material"], 0);
    assert_eq!(primitives[1]["material"], 1);
    let near = |v: &Value, p: [f32; 3]| {
        let v: Vec<f64> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        (0..3).all(|a| (v[a] - f64::from(p[a])).abs() < 1e-3)
    };
    assert!(
        near(&primitives[0]["center"], corner[0]),
        "{}",
        primitives[0]
    );
    assert!(near(&primitives[1]["end1"], corner[2]), "{}", primitives[1]);
    assert!(
        near(&primitives[2]["vertices"][3], corner[6]),
        "{}",
        primitives[2]
    );
    assert!(near(&primitives[3]["end0"], corner[4]), "{}", primitives[3]);
}

#[test]
fn a_value_the_document_states_otherwise_is_set_in_the_bound_and_the_polygons_stay_the_bases() {
    let work = tempfile::tempdir().unwrap();
    let (file, canonical) = file_and_canonical(&building());
    let dir = extract(work.path(), &file);
    edit_document(&dir, |doc| {
        let root = &mut doc["bound"];
        root["flags"] = 9.into();
        root["part_index"] = 41.into();
        root["margin"] = 0.375.into();
        root["radius_around_centroid"] = 12.5.into();
        root["material_id0"] = "0x00000099".into();
        root["bounding_box_min"] = serde_json::json!([1.0, 2.0, 3.0]);
        root["volume_distribution"] = "1.0 2.0 3.0 4.0".into();
        let children = root["children"].as_array_mut().unwrap();
        children[1]["current_matrix"][3] = "7.0 8.0 9.0 0.0".into();
        children[1]["type_flags"] = "0x00000abc".into();
        children[4]["bound"]["capsule_half_height"] = 2.5.into();
        children[3]["bound"]["materials"][1]["id"] = "0x00000000000000aa".into();
        children[3]["bound"]["material_colors"][0] = "0x01020304".into();
        children[3]["bound"]["is_flat"] = true.into();
    });
    let made = compiled(&dir);
    assert_ne!(made.bytes, canonical);
    validate(&made.bytes);
    for line in [
        "bound: flags set",
        "bound: margin set",
        "bound_child4: capsule_half_height set",
        "bound_child3: materials set",
    ] {
        assert!(
            made.report.iter().any(|l| l == line),
            "{line}: {:?}",
            made.report
        );
    }
    assert!(
        !made.report.iter().any(|l| l.contains("built again")),
        "a value changed and no polygon did: {:?}",
        made.report
    );

    let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
    let PhBoundAny::PhBoundComposite(composite) = &bound else {
        panic!("not a composite");
    };
    let h = &composite.base;
    assert_eq!((h.flags, h.part_index), (9, 41));
    assert_eq!(h.bounding_box_max_xyz_margin_w.w, 0.375);
    assert_eq!(h.radius_around_centroid, 12.5);
    assert_eq!(h.centroid_offset_xyz_material_id0_w.w.to_bits(), 0x99);
    assert_eq!(h.bounding_box_min(), [1.0, 2.0, 3.0]);
    let v = &h.volume_distribution;
    assert_eq!([v.x, v.y, v.z, v.w], [1.0, 2.0, 3.0, 4.0]);
    let moved = &composite.current_matrices[1].col3;
    assert_eq!([moved.x, moved.y, moved.z], [7.0, 8.0, 9.0]);
    assert_eq!(composite.type_and_include_flags[1].type_flags, 0xabc);
    assert_eq!(composite.type_and_include_flags[0].type_flags, 0x40);
    let PhBoundAny::PhBoundCapsule(capsule) = child(&bound, 4) else {
        panic!("not a capsule");
    };
    assert_eq!(capsule.capsule_half_height, 2.5);
    let g = soup(child(&bound, 3));
    assert_eq!(g.material_ids[1].id, 0xaa);
    assert_eq!(g.material_ids[0].id, MATERIALS[0]);
    assert_eq!(g.material_colors[0].color, 0x0102_0304);
    assert!(g.is_flat.get());

    let base = PhBoundAny::from_bytes(&file).unwrap();
    for i in [0, 3] {
        let (was, is) = (soup(child(&base, i)), soup(child(&bound, i)));
        assert_eq!(was.polygons, is.polygons, "child {i}");
        assert_eq!(was.compressed_vertices, is.compressed_vertices, "child {i}");
    }
}

#[test]
fn every_value_the_document_states_reaches_the_bound_or_is_refused() {
    for bound in [building(), building_that_holds_its_runs_apart()] {
        let work = tempfile::tempdir().unwrap();
        let (file, canonical) = file_and_canonical(&bound);
        let dir = extract(work.path(), &file);
        let pristine = document(&dir);
        let places = common::stated_values(&pristine);
        let mut tried = 0usize;
        for place in &places {
            for other in common::other_values(pristine.pointer(place).unwrap()) {
                let mut doc = pristine.clone();
                *doc.pointer_mut(place).unwrap() = other.clone();
                fs::write(dir.join(RULES), serde_json::to_string(&doc).unwrap()).unwrap();
                if let Ok(made) = bounds::compile(&Source::open(&dir).unwrap()) {
                    assert!(
                        made.bytes != canonical,
                        "{place} states {other} and the compile read nothing of it"
                    );
                }
                tried += 1;
            }
        }
        // A matrix is four strings of four numbers, so more values are tried than the document has places.
        assert!(
            tried > places.len() && places.len() > 150,
            "{tried} values tried at {} places",
            places.len()
        );
    }
    let held_apart = {
        let work = tempfile::tempdir().unwrap();
        let dir = extract(
            work.path(),
            &file_and_canonical(&building_that_holds_its_runs_apart()).0,
        );
        document(&dir)["bound"]["children"][1].clone()
    };
    assert!(
        held_apart.get("last_matrix").is_some() && held_apart.get("owned_type_flags").is_some(),
        "the runs a composite holds apart are not stated: {held_apart}"
    );
}

#[test]
fn a_glb_a_tool_read_and_wrote_again_compiles_to_the_same_bytes() {
    let work = tempfile::tempdir().unwrap();
    let (file, canonical) = file_and_canonical(&building());
    let dir = extract(work.path(), &file);
    let before = fs::read(dir.join(MODEL)).unwrap();
    edit_glb(&dir, |_| {});
    assert_eq!(
        load(&dir.join(MODEL), &fs::read(dir.join(MODEL)).unwrap())
            .unwrap()
            .objects[0]
            .meshes
            .len(),
        load(&dir.join(MODEL), &before).unwrap().objects[0]
            .meshes
            .len()
    );
    assert!(compiled(&dir).bytes == canonical);
}

/// What the staged files hold between the members of a primitive: not zero, and nothing a declaration names.
const BETWEEN_MEMBERS: [u16; 2] = [0x0d0a, 0xbeef];

/// A geometry as a `.ybn` states one: its vertex count as the length of a hull it does not hold, a box wider than its polygons, and in its polygons what no builder arrives at.
fn as_a_ybn_states_it(mut g: PhBoundGeometry) -> PhBoundGeometry {
    g.set_num_convex_hull_vertices(g.compressed_vertices.len() as u16);
    g.base.bounding_box_max_xyz_margin_w.x += 1.0;
    // A bit above a vertex index, and no neighbour across an edge two triangles share.
    let stated = g
        .polygons
        .iter()
        .position(|p| p.primitive() == 0 && p.neighboring_polygons[0] != NO_NEIGHBOUR)
        .expect("a triangle with a neighbour");
    g.polygons[stated].vertex_indices[1] |= !VERTEX_INDEX_MASK;
    g.polygons[stated].neighboring_polygons[0] = NO_NEIGHBOUR;
    // And across another edge a triangle that does not share it: a file's neighbours are not always the builder's.
    let across = g.polygons[stated].neighboring_polygons[1];
    let elsewhere = (0..g.polygons.len() as u16)
        .find(|at| {
            g.polygons[usize::from(*at)].primitive() == 0
                && *at != across
                && usize::from(*at) != stated
        })
        .expect("a third triangle");
    g.polygons[stated].neighboring_polygons[1] = elsewhere;
    // The last four bytes of a primitive's record are no member of any of the four kinds, and a box holds a bit above a vertex index too.
    for p in g.polygons.iter_mut().filter(|p| p.primitive() != 0) {
        [p.neighboring_polygons[1], p.neighboring_polygons[2]] = BETWEEN_MEMBERS;
        if p.primitive() == 3 {
            p.vertex_indices[0] |= !VERTEX_INDEX_MASK;
        }
    }
    g
}

/// The words of the corners of a triangle, which name it wherever its polygon sits.
type Corners = [[i16; 3]; 3];

/// What the record of a triangle states: its area, the bit above each vertex index, and the triangle across each edge.
type Record = (u32, [bool; 3], [Option<Corners>; 3]);

/// The record of each triangle of `g`.
fn records(g: &PhBoundGeometry) -> BTreeMap<Corners, Record> {
    let corners = |p: &PhPolygon| -> Corners {
        p.vertex_indices.map(|v| {
            let v = &g.compressed_vertices[usize::from(v & VERTEX_INDEX_MASK)];
            [v.x, v.y, v.z]
        })
    };
    g.polygons
        .iter()
        .filter(|p| p.primitive() == 0)
        .map(|p| {
            let across = p
                .neighboring_polygons
                .map(|n| (n != NO_NEIGHBOUR).then(|| corners(&g.polygons[usize::from(n)])));
            let above = p.vertex_indices.map(|v| v & !VERTEX_INDEX_MASK != 0);
            (corners(p), (p.area.to_bits(), above, across))
        })
        .collect()
}

/// The box, the centroid and the radius a bound's header states, bit for bit.
fn extent(h: &PhBound) -> Vec<u32> {
    let (max, centroid) = (
        &h.bounding_box_max_xyz_margin_w,
        &h.centroid_offset_xyz_material_id0_w,
    );
    let min = h.bounding_box_min();
    [
        max.x,
        max.y,
        max.z,
        min[0],
        min[1],
        min[2],
        centroid.x,
        centroid.y,
        centroid.z,
        h.radius_around_centroid,
    ]
    .map(f32::to_bits)
    .to_vec()
}

#[test]
fn a_glb_that_gained_a_triangle_cooks_to_a_bound_that_validates_and_holds_it() {
    let mut tree = bvh();
    tree.base = as_a_ybn_states_it(tree.base);
    let cases = [
        ("PhBoundBVH", PhBoundAny::PhBoundBVH(tree)),
        (
            "PhBoundGeometry",
            PhBoundAny::PhBoundGeometry(as_a_ybn_states_it(geometry())),
        ),
    ];
    for (kind, bound) in cases {
        let work = tempfile::tempdir().unwrap();
        let (file, canonical) = file_and_canonical(&bound);
        let dir = extract(work.path(), &file);
        let base = PhBoundAny::from_bytes(&file).unwrap();
        let was = soup(&base);
        // Inside the slab, so the quantum of the base still holds it; on no vertex the slab has.
        let added = [AT[0] + 1.0, AT[1] - 0.5, AT[2] + 0.25];
        edit_glb(&dir, |scene| {
            let primitive = &mut scene.objects[0].meshes[0].primitives[1];
            let new = primitive.positions.len() as u32;
            primitive.positions.push(added);
            primitive.indices.extend([0, 1, new]);
        });

        let made = compiled(&dir);
        assert_ne!(made.bytes, canonical, "{kind}");
        assert!(
            made.report
                .iter()
                .any(|l| l.contains("bound: its polygons changed")),
            "{kind}: {:?}",
            made.report
        );
        validate(&made.bytes);
        let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
        let is = soup(&bound);
        assert_eq!(triangles(is), triangles(was) + 1, "{kind}");
        assert_eq!(
            is.polygons.len(),
            was.polygons.len() + 1,
            "{kind}: the primitives stay"
        );
        assert_eq!(
            is.compressed_vertices.len(),
            was.compressed_vertices.len() + 1,
            "{kind}"
        );
        assert_eq!(
            is.num_convex_hull_vertices(),
            Some(is.compressed_vertices.len() as u16),
            "{kind}: the hull a .ybn states the length of follows its vertices"
        );

        // The triangle is inside the box and the quantum of the base, so both are the base's still.
        assert_eq!(extent(&is.base), extent(&was.base), "{kind}: the box moved");
        let quantum_of = |g: &PhBoundGeometry| {
            let (f, c) = (&g.un_quantize_factor, &g.bounding_box_center);
            [f.x, f.y, f.z, c.x, c.y, c.z].map(f32::to_bits)
        };
        assert_eq!(quantum_of(is), quantum_of(was), "{kind}: the quantum moved");

        // The triangle that stands on the new vertex is there, in the second material, on the corners it was given.
        let quantum = [
            was.un_quantize_factor.x,
            was.un_quantize_factor.y,
            was.un_quantize_factor.z,
        ];
        let is_added = |p: [f32; 3]| (0..3).all(|a| (p[a] - added[a]).abs() <= quantum[a]);
        let holding: Vec<usize> = is
            .polygons
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.primitive() == 0
                    && p.vertex_indices.iter().any(|v| {
                        is_added(
                            is.vertex_position(usize::from(*v & VERTEX_INDEX_MASK))
                                .unwrap(),
                        )
                    })
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            holding.len(),
            1,
            "{kind}: one triangle stands on the new vertex"
        );
        assert_eq!(is.poly_mat_index_list[holding[0]], 1, "{kind}");
        assert_eq!(is.poly_mat_index_list.len(), is.polygons.len(), "{kind}");

        // Every vertex the base held is stored as the word it was, and every triangle of the base keeps its area.
        let words = |g: &PhBoundGeometry| -> Vec<[i16; 3]> {
            g.compressed_vertices
                .iter()
                .map(|v| [v.x, v.y, v.z])
                .collect()
        };
        let (old, new) = (words(was), words(is));
        assert!(
            old.iter().all(|w| new.contains(w)),
            "{kind}: a vertex of the base moved"
        );
        let areas = |g: &PhBoundGeometry| -> Vec<u32> {
            let mut areas: Vec<u32> = g
                .polygons
                .iter()
                .filter(|p| p.primitive() == 0)
                .map(|p| p.area.to_bits())
                .collect();
            areas.sort_unstable();
            areas
        };
        let (old, new) = (areas(was), areas(is));
        assert!(
            old.iter().all(|a| new.contains(a)),
            "{kind}: an area of the base changed"
        );

        // Every triangle of the base keeps what its record states: its area, the bit above a vertex index, and its
        // neighbours, with the new triangle only across an edge where the base states none.
        let (was_records, is_records) = (records(was), records(is));
        let added: Vec<&Corners> = is_records
            .keys()
            .filter(|corners| !was_records.contains_key(*corners))
            .collect();
        assert_eq!(added.len(), 1, "{kind}");
        for (corners, (area, above, across)) in &was_records {
            let (is_area, is_above, is_across) = is_records
                .get(corners)
                .unwrap_or_else(|| panic!("{kind}: the triangle {corners:?} of the base is gone"));
            assert_eq!((area, above), (is_area, is_above), "{kind}: {corners:?}");
            for k in 0..3 {
                assert!(
                    across[k] == is_across[k]
                        || (across[k].is_none() && is_across[k].as_ref() == Some(added[0])),
                    "{kind}: {corners:?} states {:?} across its edge {k} and stated {:?}",
                    is_across[k],
                    across[k]
                );
            }
        }
        let stated = |records: &BTreeMap<Corners, Record>| {
            records
                .values()
                .filter(|(_, above, across)| above[1] && across[0].is_none())
                .count()
        };
        assert_eq!(
            (stated(&was_records), stated(&is_records)),
            (1, 1),
            "{kind}: the triangle that states a bit and no neighbour"
        );
        let between = |g: &PhBoundGeometry| -> Vec<[u16; 2]> {
            g.polygons
                .iter()
                .filter(|p| p.primitive() != 0)
                .map(|p| [p.neighboring_polygons[1], p.neighboring_polygons[2]])
                .collect()
        };
        assert_eq!(
            between(is),
            vec![BETWEEN_MEMBERS; 4],
            "{kind}: a primitive of the base lost what its record holds between its members"
        );
        let boxed = is.polygons.iter().find(|p| p.primitive() == 3).unwrap();
        assert_ne!(
            boxed.vertex_indices[0] & !VERTEX_INDEX_MASK,
            0,
            "{kind}: the bit above a box's vertex index is gone"
        );

        if let PhBoundAny::PhBoundBVH(b) = &bound {
            let tree = b.bvh.as_deref().expect("a tree");
            let covered: usize = tree
                .contiguous_nodes
                .iter()
                .map(|n| usize::from(n.polygon_count))
                .sum();
            assert_eq!(covered, is.polygons.len(), "the leaves cover every polygon");
        }
    }
}

#[test]
fn a_triangle_added_outside_the_quantum_of_the_base_is_held_in_a_new_one() {
    let work = tempfile::tempdir().unwrap();
    let (file, _) = file_and_canonical(&PhBoundAny::PhBoundBVH(bvh()));
    let dir = extract(work.path(), &file);
    let far = [AT[0] + 40.0, AT[1], AT[2] + 6.0];
    edit_glb(&dir, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        let new = primitive.positions.len() as u32;
        primitive.positions.push(far);
        primitive.indices.extend([0, 1, new]);
    });
    let made = compiled(&dir);
    validate(&made.bytes);
    let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
    let g = soup(&bound);
    let quantum = [
        g.un_quantize_factor.x,
        g.un_quantize_factor.y,
        g.un_quantize_factor.z,
    ];
    let nearest = |p: [f32; 3]| {
        (0..g.compressed_vertices.len())
            .map(|i| g.vertex_position(i).unwrap())
            .any(|q| (0..3).all(|a| (q[a] - p[a]).abs() <= quantum[a]))
    };
    assert!(nearest(far), "the far vertex is not held");
    for p in slab().positions {
        assert!(
            nearest(p),
            "{p:?} of the slab is not held in the new quantum"
        );
    }
    let max = &g.base.bounding_box_max_xyz_margin_w;
    assert!(
        max.x >= far[0] - quantum[0] && max.z >= far[2] - quantum[2],
        "the box does not reach it"
    );
    assert_eq!(max.w, 0.025, "the margin is the base's");
    assert_eq!(g.base.flags, 5, "the flags are the base's");
    // The centroid is the middle of the new box, and the radius reaches the far vertex from it.
    let min = g.base.bounding_box_min();
    let centroid = &g.base.centroid_offset_xyz_material_id0_w;
    let middle: [f32; 3] = std::array::from_fn(|a| (min[a] + [max.x, max.y, max.z][a]) * 0.5);
    assert_eq!([centroid.x, centroid.y, centroid.z], middle);
    let reach = (0..3)
        .map(|a| (far[a] - middle[a]).powi(2))
        .sum::<f32>()
        .sqrt();
    let base = PhBoundAny::from_bytes(&file).unwrap();
    assert!(
        g.base.radius_around_centroid >= reach - quantum[0]
            && g.base.radius_around_centroid > soup(&base).base.radius_around_centroid + 10.0,
        "the radius {} does not reach the far vertex at {reach}",
        g.base.radius_around_centroid
    );
}

/// ⚠ Far from the origin a float is wide: the middle of two of them is often no float, so one end of the new box is farther from
/// the centre than the other, and a quantum of half the extent would store that end as a word past the last.
#[test]
fn a_bound_far_from_the_origin_that_grows_past_its_quantum_is_held_whatever_float_its_new_end_is() {
    let at = [5000.5_f32, -3400.25, 31.0];
    let mut mesh = slab();
    for p in &mut mesh.positions {
        *p = std::array::from_fn(|a| p[a] - AT[a] + at[a]);
    }
    let mut bound = PhBoundBVH {
        base: build::geometry(&mesh).unwrap(),
        ..Default::default()
    };
    bound.base.base.r#type = Open::new(BoundType::Bvh);
    build::rebuild(&mut bound).unwrap();
    let (file, _) = file_and_canonical(&PhBoundAny::PhBoundBVH(bound));

    let mut refused = Vec::new();
    let mut far = at[0] + 9.0;
    for step in 0..40 {
        let work = tempfile::tempdir().unwrap();
        let dir = extract(work.path(), &file);
        edit_glb(&dir, |scene| {
            let primitive = &mut scene.objects[0].meshes[0].primitives[0];
            let new = primitive.positions.len() as u32;
            primitive.positions.push([far, at[1], at[2]]);
            primitive.indices.extend([0, 1, new]);
        });
        match bounds::compile(&Source::open(&dir).unwrap()) {
            Ok(made) => validate(&made.bytes),
            Err(problems) => refused.push((step, far, problems[0].to_string())),
        }
        far = far.next_up();
    }
    assert!(
        refused.is_empty(),
        "{} of 40 ends are refused, the first {:?}",
        refused.len(),
        refused.first()
    );
}

#[test]
fn a_polygon_that_changed_in_one_thing_alone_is_a_changed_polygon() {
    let (file, canonical) = file_and_canonical(&PhBoundAny::PhBoundGeometry(geometry()));
    let made = |edit_doc: &dyn Fn(&mut Value), edit_model: &dyn Fn(&mut Scene)| {
        let work = tempfile::tempdir().unwrap();
        let dir = extract(work.path(), &file);
        edit_document(&dir, edit_doc);
        edit_glb(&dir, edit_model);
        let made = compiled(&dir);
        assert_ne!(made.bytes, canonical);
        assert!(
            made.report
                .iter()
                .any(|l| l.contains("its polygons changed")),
            "{:?}",
            made.report
        );
        validate(&made.bytes);
        let PhBoundAny::PhBoundGeometry(g) = PhBoundAny::from_bytes(&made.bytes).unwrap() else {
            panic!("not a geometry");
        };
        g
    };
    let material_of = |g: &PhBoundGeometry, kind: u32| -> Vec<u8> {
        (0..g.polygons.len())
            .filter(|i| g.polygons[*i].primitive() == kind)
            .map(|i| g.poly_mat_index_list[i])
            .collect()
    };
    let count = |list: &[u8], material: u8| list.iter().filter(|m| **m == material).count();
    let base = geometry();
    let was = material_of(&base, 0);

    // One triangle is given the other material and nothing of it moves.
    let g = made(&|_| {}, &|scene| {
        let primitives = &mut scene.objects[0].meshes[0].primitives;
        let from = primitives[0].indices.len() - 3;
        let moved: Vec<[f32; 3]> = primitives[0].indices[from..]
            .iter()
            .map(|i| primitives[0].positions[*i as usize])
            .collect();
        primitives[0].indices.truncate(from);
        let first = primitives[1].positions.len() as u32;
        primitives[1].positions.extend(moved);
        primitives[1].indices.extend([first, first + 1, first + 2]);
    });
    let is = material_of(&g, 0);
    assert_eq!(
        (count(&is, 0), count(&is, 1)),
        (count(&was, 0) - 1, count(&was, 1) + 1)
    );
    assert_eq!(g.compressed_vertices.len(), base.compressed_vertices.len());

    // The radius of a sphere, its material, the kind of a capsule and the place of a sphere, each alone.
    let g = made(
        &|doc| doc["bound"]["primitives"][0]["radius"] = 0.75.into(),
        &|_| {},
    );
    let sphere = g.polygons.iter().find(|p| p.primitive() == 1).unwrap();
    assert_eq!(g.polygon_vertices(sphere).unwrap().1, 0.75);

    assert_eq!(material_of(&base, 1), [0]);
    let g = made(
        &|doc| doc["bound"]["primitives"][0]["material"] = 1.into(),
        &|_| {},
    );
    assert_eq!(material_of(&g, 1), [1]);

    let g = made(
        &|doc| doc["bound"]["primitives"][1]["type"] = "cylinder".into(),
        &|_| {},
    );
    let by_kind = |kind: u32| g.polygons.iter().filter(|p| p.primitive() == kind).count();
    assert_eq!(
        [by_kind(1), by_kind(2), by_kind(3), by_kind(4)],
        [1, 0, 1, 2]
    );

    let corner = slab().positions;
    let g = made(
        &|doc| {
            doc["bound"]["primitives"][0]["center"] =
                serde_json::json!([corner[7][0], corner[7][1], corner[7][2]])
        },
        &|_| {},
    );
    let sphere = g.polygons.iter().find(|p| p.primitive() == 1).unwrap();
    let centre = g
        .vertex_position(g.polygon_vertices(sphere).unwrap().0[0])
        .unwrap();
    assert!(
        (0..3).all(|a| (centre[a] - corner[7][a]).abs() <= g.un_quantize_factor.x.max(0.01)),
        "{centre:?}"
    );
}

#[test]
fn a_changed_primitive_and_a_removed_triangle_reach_the_bound() {
    let work = tempfile::tempdir().unwrap();
    let (file, _) = file_and_canonical(&PhBoundAny::PhBoundGeometry(geometry()));
    let dir = extract(work.path(), &file);
    edit_document(&dir, |doc| {
        doc["bound"]["primitives"][0]["radius"] = 0.75.into();
        doc["bound"]["primitives"].as_array_mut().unwrap().pop();
    });
    edit_glb(&dir, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        primitive.indices.truncate(primitive.indices.len() - 3);
    });
    let made = compiled(&dir);
    validate(&made.bytes);
    let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
    let g = soup(&bound);
    assert_eq!(triangles(g), slab().triangles.len() - 1);
    let by_kind = |kind: u32| g.polygons.iter().filter(|p| p.primitive() == kind).count();
    assert_eq!(
        [by_kind(1), by_kind(2), by_kind(3), by_kind(4)],
        [1, 1, 1, 0]
    );
    let sphere = g.polygons.iter().find(|p| p.primitive() == 1).unwrap();
    assert_eq!(g.polygon_vertices(sphere).unwrap().1, 0.75);
}

#[test]
fn two_triangles_on_the_same_corners_each_keep_what_their_record_states() {
    // The twin is in the first material and after its triangle, which is in the second: a glb states it first.
    let mut g = geometry();
    let first = (0..g.polygons.len())
        .find(|at| g.polygons[*at].primitive() == 0 && g.poly_mat_index_list[*at] == 1)
        .unwrap();
    let mut twin = g.polygons[first].clone();
    twin.area = f32::from_bits(twin.area.to_bits() + (PRIMITIVE_TYPE_MASK + 1) * 5);
    g.polygons.push(twin);
    g.poly_mat_index_list.push(0);
    let work = tempfile::tempdir().unwrap();
    let (file, _) = file_and_canonical(&PhBoundAny::PhBoundGeometry(g));
    let dir = extract(work.path(), &file);
    edit_glb(&dir, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[1];
        let new = primitive.positions.len() as u32;
        primitive
            .positions
            .push([AT[0] + 1.0, AT[1] - 0.5, AT[2] + 0.25]);
        primitive.indices.extend([0, 1, new]);
    });
    let made = compiled(&dir);
    validate(&made.bytes);
    let base = PhBoundAny::from_bytes(&file).unwrap();
    let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
    // The material and the area of every triangle, which only the two on the same corners tell apart.
    let stated = |g: &PhBoundGeometry| -> Vec<(u8, u32)> {
        let mut stated: Vec<(u8, u32)> = (0..g.polygons.len())
            .filter(|at| g.polygons[*at].primitive() == 0)
            .map(|at| (g.poly_mat_index_list[at], g.polygons[at].area.to_bits()))
            .collect();
        stated.sort_unstable();
        stated
    };
    let (mut was, is) = (stated(soup(&base)), stated(soup(&bound)));
    assert_eq!(is.len(), was.len() + 1);
    let added: Vec<(u8, u32)> = is
        .iter()
        .copied()
        .filter(|triangle| {
            let held = was.iter().position(|t| t == triangle);
            held.map(|at| was.remove(at)).is_none()
        })
        .collect();
    assert_eq!(
        (added.len(), was.len()),
        (1, 0),
        "a triangle of the base states another area: {was:?}"
    );
}

fn import_from_the_game(project: &Path, file: &[u8]) -> PathBuf {
    use or_source::game_import::{import, Facts, Input, Options, Verdict};
    let facts = Facts {
        from_game: true,
        virtual_path: Some("x64a.rpf/hull.ybn".to_string()),
        ..Facts::default()
    };
    let report = import(
        project,
        &[Input::Bytes {
            name: "hull.ybn",
            bytes: file,
            facts,
        }],
        &Options::default(),
    );
    match &report.files[0].verdict {
        Verdict::Made { folder } => folder.clone(),
        other => panic!("{other:?}: {}", report.files[0].reason),
    }
}

#[test]
fn an_untouched_source_taken_from_the_game_is_the_games_file_to_a_build_and_one_with_a_changed_polygon_is_not(
) {
    let project = tempfile::tempdir().unwrap();
    let (file, _) = file_and_canonical(&building());
    let folder = import_from_the_game(project.path(), &file);
    assert_eq!(
        folder,
        project.path().join("sources").join("hull.bounds.src")
    );
    let manifest = Source::open(&folder).unwrap().manifest;
    assert!(manifest.origin.from_game);
    assert_eq!(manifest.origin.sha256, Some(sha256_hex(&file)));

    let cook = |scratch: &str| {
        let output = cook_source(&folder, &project.path().join(scratch)).unwrap();
        sha256_hex(&fs::read(output).unwrap())
    };
    let untouched = cook("first");
    assert_eq!(
        manifest.origin.cooked_sha256.as_deref(),
        Some(untouched.as_str())
    );
    assert!(
        describe(&folder).unwrap().game.contains(&untouched),
        "an import nobody touched cooks to a file the build does not take for the game's, so it would ship"
    );

    // One corner of one triangle moves half a metre into the slab: a changed polygon.
    edit_glb(&folder, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        let corner = primitive.indices[0] as usize;
        let z = &mut primitive.positions[corner][2];
        *z += if *z > AT[2] { -0.5 } else { 0.5 };
    });
    let edited = cook("second");
    assert_ne!(
        untouched, edited,
        "the changed polygon did not reach the cook"
    );
    assert!(
        !describe(&folder).unwrap().game.contains(&edited),
        "a source with a changed polygon still cooks to what the build takes for the game's file"
    );
}

#[test]
fn a_document_that_names_a_node_the_glb_does_not_hold_is_refused_with_the_name() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&building()).0);
    edit_document(&dir, |doc| {
        doc["bound"]["children"][3]["bound"]["node"] = "hull_bound_annexe".into();
    });
    let why = refusal(&dir);
    assert!(why.contains("`hull_bound_annexe`"), "{why}");
    assert!(
        why.starts_with("bounds.json: bound.children[3].bound.node "),
        "{why}"
    );
    assert!(why.contains("bounds.glb does not hold"), "{why}");
}

#[test]
fn a_node_of_the_glb_no_bound_names_is_refused_with_its_name_and_never_left_out() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&building()).0);
    edit_glb(&dir, |scene| {
        let mut annexe = scene.objects[0].meshes[0].clone();
        annexe.name = "annexe".to_string();
        scene.objects[0].meshes.push(annexe);
    });
    let why = refusal(&dir);
    assert!(
        why.starts_with("bounds.glb: ") && why.contains("`hull_annexe`"),
        "{why}"
    );

    // A bound that stops naming its node leaves it named by nobody, which is refused the same way.
    let dir = extract(
        &work.path().join("again"),
        &file_and_canonical(&building()).0,
    );
    edit_document(&dir, |doc| {
        doc["bound"]["children"][3]["bound"]
            .as_object_mut()
            .unwrap()
            .remove("node");
    });
    let why = refusal(&dir);
    assert!(
        why.starts_with("bounds.glb: ") && why.contains("`hull_bound_child3`"),
        "{why}"
    );
}

#[test]
fn an_edit_this_source_does_not_compile_is_refused_where_it_is_stated() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&building()).0);
    let pristine = fs::read_to_string(dir.join(RULES)).unwrap();
    let refused = |edit: &dyn Fn(&mut Value), place: &str, word: &str| {
        fs::write(dir.join(RULES), &pristine).unwrap();
        edit_document(&dir, edit);
        let why = refusal(&dir);
        assert!(
            why.starts_with(&format!("bounds.json: {place} ")),
            "{place}: {why}"
        );
        assert!(why.contains(word), "{word}: {why}");
    };
    refused(
        &|doc| doc["bound"]["children"][1]["bound"]["type"] = "PhBoundBox".into(),
        "bound.children[1].bound.type",
        "PhBoundSphere",
    );
    refused(
        &|doc| {
            doc["bound"]["children"].as_array_mut().unwrap().pop();
        },
        "bound.children",
        "holds 4 children and the base holds 5",
    );
    refused(
        &|doc| {
            doc["bound"]["children"][2]["bound"] = serde_json::json!({ "type": "PhBoundSphere" })
        },
        "bound.children[2]",
        "adding a child",
    );
    refused(
        &|doc| doc["bound"]["children"][0]["bound"]["colour"] = 1.into(),
        "bound.children[0].bound",
        "has no member `colour`",
    );
    refused(
        &|doc| {
            doc["bound"]["children"][0]
                .as_object_mut()
                .unwrap()
                .remove("local_box_max");
        },
        "bound.children[0]",
        "`local_box_min` and `local_box_max`",
    );
    refused(
        &|doc| doc["bound"]["children"][3]["bound"]["primitives"][1]["material"] = 6.into(),
        "bound.children[3].bound.primitives[1].material",
        "names material 6 of 2",
    );
    refused(
        &|doc| doc["bound"]["children"][3]["bound"]["primitives"][2]["type"] = "cone".into(),
        "bound.children[3].bound.primitives[2].type",
        "\"cone\"",
    );
    refused(
        &|doc| doc["bound"]["margin"] = "wide".into(),
        "bound.margin",
        "is not a number",
    );
    refused(
        &|doc| doc["bound"]["children"][3]["bound"]["materials"][0]["name"] = "stone".into(),
        "bound.children[3].bound.node",
        "`bound_child3_material0`",
    );
    refused(
        &|doc| {
            doc["bound"]["children"][3]["bound"]["materials"][1]["name"] =
                "bound_child3_material0".into()
        },
        "bound.children[3].bound.materials[1].name",
        "another material of this bound is called too",
    );
    refused(
        &|doc| doc["bound"]["children"][1]["bound"] = Value::Null,
        "bound.children[1]",
        "removing a child",
    );
    // A number no float holds would be read as an infinity.
    refused(
        &|doc| doc["bound"]["margin"] = 1e40.into(),
        "bound.margin",
        "is not a number",
    );
    refused(
        &|doc| {
            doc["bound"]["children"][3]["bound"]["primitives"][0]["center"][2] =
                format!("{:#010x}", f32::NAN.to_bits()).into()
        },
        "bound.children[3].bound",
        "which is not a place",
    );
    refused(
        &|doc| {
            doc["bound"]["children"][3]["bound"]["node"] = "hull_bound_child0".into();
        },
        "bound.children[3].bound.node",
        "which another bound names too",
    );
}

#[test]
fn a_glb_this_source_does_not_compile_is_refused_by_what_is_wrong_with_it() {
    let building = file_and_canonical(&building()).0;
    let refused = |file: &[u8], edit: &dyn Fn(&mut Scene), place: &str, word: &str| {
        let work = tempfile::tempdir().unwrap();
        let dir = extract(work.path(), file);
        edit_glb(&dir, edit);
        let why = refusal(&dir);
        assert!(why.starts_with(place), "{place}: {why}");
        assert!(why.contains(word), "{word}: {why}");
    };
    refused(
        &building,
        &|scene| {
            let primitive = &mut scene.objects[0].meshes[0].primitives[0];
            (primitive.material, primitive.material_name) = (None, None);
        },
        "bounds.json: bound.children[0].bound ",
        "name a material beside polygons that name none",
    );
    refused(
        &building,
        &|scene| {
            let mut copy = scene.objects[0].meshes[0].clone();
            copy.name = "bound_child0.001".to_string();
            scene.objects[0].meshes.push(copy);
        },
        "bounds.glb: ",
        "two nodes called `hull_bound_child0`",
    );
    // A polygon names its neighbour in sixteen bits, the last value of which says there is none.
    refused(
        &file_and_canonical(&PhBoundAny::PhBoundGeometry(geometry())).0,
        &|scene| {
            let primitive = &mut scene.objects[0].meshes[0].primitives[0];
            let one: Vec<u32> = primitive.indices[..3].to_vec();
            primitive.indices = one.repeat(usize::from(NO_NEIGHBOUR));
        },
        "bounds.json: bound ",
        "names its neighbour in sixteen bits",
    );
}

#[test]
fn a_child_that_outgrows_the_box_its_composite_keeps_of_it_is_refused_until_the_box_is_stated() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&building()).0);
    let far = [AT[0] + 40.0, AT[1], AT[2]];
    edit_glb(&dir, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        let new = primitive.positions.len() as u32;
        primitive.positions.push(far);
        primitive.indices.extend([0, 1, new]);
    });
    let why = refusal(&dir);
    assert!(why.starts_with("bounds.json: bound.children[0] "), "{why}");
    assert!(
        why.contains("bound_child0") && why.contains("`local_box_max`"),
        "{why}"
    );

    edit_document(&dir, |doc| {
        doc["bound"]["children"][0]["local_box_max"] =
            format!("{:?} {:?} {:?} 0.0", far[0], AT[1] + 4.0, AT[2] + 1.0).into();
    });
    let made = compiled(&dir);
    validate(&made.bytes);
    assert!(
        made.report
            .iter()
            .any(|l| l.starts_with("bound: the boxes of its children changed")),
        "{:?}",
        made.report
    );
    let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
    let PhBoundAny::PhBoundComposite(composite) = &bound else {
        panic!("not a composite");
    };
    assert_eq!(composite.local_box_min_maxs[0].max.x, far[0]);
    let tree = composite
        .bvh_structure
        .as_deref()
        .expect("a tree over the children");
    let leaves: Vec<u16> = tree
        .contiguous_nodes
        .iter()
        .filter(|n| n.polygon_count > 0)
        .map(|n| n.node_data)
        .collect();
    let mut named = leaves.clone();
    named.sort_unstable();
    assert_eq!(
        named,
        [0, 1, 2, 3, 4],
        "each child is the item of one leaf: {leaves:?}"
    );
    assert!(
        tree.aabb_max.x >= far[0],
        "the tree does not reach the new box"
    );
}

/// A geometry that holds an attribute and a second-surface displacement for each vertex, each telling its vertex.
fn with_a_value_for_each_vertex(mut g: PhBoundGeometry) -> PhBoundGeometry {
    let vertices = g.compressed_vertices.len() as u32;
    g.vertex_attribs = (0..vertices)
        .map(|i| Color32 {
            color: 0x1000 + i,
            ..Default::default()
        })
        .collect();
    g.second_surface_vertex_displacements = (0..vertices).map(|i| 0.125 * i as f32).collect();
    g
}

#[test]
fn a_value_held_for_each_vertex_follows_its_vertex_and_a_new_vertex_is_refused_by_name() {
    let g = with_a_value_for_each_vertex(geometry());
    let work = tempfile::tempdir().unwrap();
    let (file, canonical) = file_and_canonical(&PhBoundAny::PhBoundGeometry(g));
    let dir = extract(work.path(), &file);
    assert!(
        compiled(&dir).bytes == canonical,
        "untouched, the values are the base's"
    );
    let pristine = fs::read(dir.join(MODEL)).unwrap();

    // The triangles of the second material go first and one of the first goes: the vertices come in another order.
    edit_glb(&dir, |scene| {
        let primitives = &mut scene.objects[0].meshes[0].primitives;
        let len = primitives[0].indices.len();
        primitives[0].indices.truncate(len - 3);
        primitives.swap(0, 1);
    });
    let made = compiled(&dir);
    validate(&made.bytes);
    let base = PhBoundAny::from_bytes(&file).unwrap();
    let bound = PhBoundAny::from_bytes(&made.bytes).unwrap();
    let (was, is) = (soup(&base), soup(&bound));
    assert_eq!(triangles(is), triangles(was) - 1);
    let held = |g: &PhBoundGeometry| -> BTreeMap<[i16; 3], (u32, u32)> {
        assert_eq!(g.vertex_attribs.len(), g.compressed_vertices.len());
        assert_eq!(
            g.second_surface_vertex_displacements.len(),
            g.compressed_vertices.len()
        );
        g.compressed_vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let displacement = g.second_surface_vertex_displacements[i].to_bits();
                ([v.x, v.y, v.z], (g.vertex_attribs[i].color, displacement))
            })
            .collect()
    };
    assert_eq!(
        held(is),
        held(was),
        "a vertex holds another value than the base held for it"
    );
    let order = |g: &PhBoundGeometry| -> Vec<[i16; 3]> {
        g.compressed_vertices
            .iter()
            .map(|v| [v.x, v.y, v.z])
            .collect()
    };
    assert_ne!(
        order(is),
        order(was),
        "the vertices kept their order, so nothing had to follow"
    );

    // A vertex the base does not hold has no value to follow.
    fs::write(dir.join(MODEL), &pristine).unwrap();
    edit_glb(&dir, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        let new = primitive.positions.len() as u32;
        primitive
            .positions
            .push([AT[0] + 1.0, AT[1] - 0.5, AT[2] + 0.25]);
        primitive.indices.extend([0, 1, new]);
    });
    let why = refusal(&dir);
    assert!(
        why.starts_with("bounds.json: bound has a vertex at ")
            && why.contains("that the base does not hold")
            && why.contains("vertex attributes"),
        "{why}"
    );
}

#[test]
fn a_changed_mesh_is_refused_by_name_where_what_follows_the_vertices_cannot_be_told() {
    let take_a_triangle = |scene: &mut Scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        primitive.indices.truncate(primitive.indices.len() - 3);
    };
    let refused = |g: PhBoundGeometry, word: &str| {
        let work = tempfile::tempdir().unwrap();
        let (file, canonical) = file_and_canonical(&PhBoundAny::PhBoundGeometry(g));
        let dir = extract(work.path(), &file);
        assert!(compiled(&dir).bytes == canonical, "{word}: untouched");
        edit_glb(&dir, take_a_triangle);
        let why = refusal(&dir);
        assert!(
            why.starts_with("bounds.json: bound ") && why.contains(word),
            "{word}: {why}"
        );
    };

    let mut hull = geometry();
    hull.compressed_shrunk_vertices = hull.compressed_vertices.clone();
    refused(hull, "a shrunk hull");

    let mut octants = geometry();
    octants.octant_verts = Some(Box::new(OctantMap {
        octants: std::array::from_fn(|octant| vec![octant as u32]),
    }));
    refused(octants, "an octant map");

    // Two vertices at one place that differ in their attribute: a corner of the glb is a place and names neither.
    let mut torn = geometry();
    let again = torn.compressed_vertices[0].clone();
    torn.compressed_vertices.push(again);
    let twin = (torn.compressed_vertices.len() - 1) as u16;
    let stands_on_the_first = torn
        .polygons
        .iter()
        .position(|p| p.primitive() == 0 && p.vertex_indices[0] == 0)
        .expect("a triangle on the first vertex");
    torn.polygons[stands_on_the_first].vertex_indices[0] = twin;
    refused(
        with_a_value_for_each_vertex(torn),
        "two vertices that differ in their vertex attributes",
    );

    // A run that is not one value for each vertex follows nothing that can be told.
    let mut short = with_a_value_for_each_vertex(geometry());
    short.second_surface_vertex_displacements.pop();
    refused(short, "second-surface displacements for 8 vertices");
}

#[test]
fn a_bounds_source_is_recognised_by_its_document_and_recooked_when_one_of_its_files_changes() {
    let work = tempfile::tempdir().unwrap();
    let (file, canonical) = file_and_canonical(&building());
    let dir = extract(work.path(), &file);
    let mut cooker = Cooker::new(work.path().join("vfs"));
    for input in [RULES, MODEL, BASE, "asset.json"] {
        let mut fresh = Cooker::new(work.path().join(format!("vfs_{input}")));
        match fresh.recook(&dir.join(input)) {
            Recook::Cooked { output, .. } => {
                assert!(fs::read(output).unwrap() == canonical, "{input}")
            }
            other => panic!("{input}: {other:?}"),
        }
    }
    assert!(matches!(
        cooker.recook(&dir.join(RULES)),
        Recook::Cooked { .. }
    ));
    assert!(matches!(
        cooker.recook(&dir.join(MODEL)),
        Recook::Unchanged { .. }
    ));
    assert_eq!(
        cooker.recook(&dir.join("notes.txt")),
        Recook::Ignored(Ignored::NotAnInput)
    );

    // A save of a source nobody edited changes no byte of it.
    let before = (
        fs::read(dir.join(RULES)).unwrap(),
        fs::read(dir.join("asset.json")).unwrap(),
    );
    Source::open(&dir).unwrap().save().unwrap();
    let after = (
        fs::read(dir.join(RULES)).unwrap(),
        fs::read(dir.join("asset.json")).unwrap(),
    );
    assert!(
        before == after,
        "a save rewrote a file of an untouched source"
    );

    fs::remove_file(dir.join(RULES)).unwrap();
    let problem = Source::open(&dir).unwrap_err();
    assert_eq!(problem.file.as_deref(), Some("bounds.json"));

    // An edit of the glb, then one of the document, is seen by the cooker that cooked the source before.
    let edited = tempfile::tempdir().unwrap();
    let dir = extract(edited.path(), &file);
    let mut cooker = Cooker::new(edited.path().join("vfs"));
    assert!(matches!(
        cooker.recook(&dir.join(MODEL)),
        Recook::Cooked { .. }
    ));
    edit_glb(&dir, |scene| {
        let primitive = &mut scene.objects[0].meshes[0].primitives[0];
        primitive.indices.truncate(primitive.indices.len() - 3);
    });
    let after_the_glb = match cooker.recook(&dir.join(MODEL)) {
        Recook::Cooked { output, .. } => fs::read(output).unwrap(),
        other => panic!("the edit of the glb was not cooked: {other:?}"),
    };
    assert!(after_the_glb != canonical);
    edit_document(&dir, |doc| doc["bound"]["margin"] = 0.5.into());
    match cooker.recook(&dir.join(RULES)) {
        Recook::Cooked { output, .. } => assert!(fs::read(output).unwrap() != after_the_glb),
        other => panic!("the edit of the document was not cooked: {other:?}"),
    }

    let other = tempfile::tempdir().unwrap();
    let dir = extract(other.path(), &file);
    fs::write(dir.join(BASE), &file[..file.len() - 1]).unwrap();
    let why = refusal(&dir);
    assert!(
        why.starts_with("base.ybn: ") && why.contains("compile.base"),
        "{why}"
    );
}

#[test]
fn a_file_that_is_no_bound_is_refused_and_nothing_is_left_behind() {
    let work = tempfile::tempdir().unwrap();
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    assert!(bounds::extract(b"hull", NAME, work.path(), &how).is_err());
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
}
