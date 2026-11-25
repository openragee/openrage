//! The polygons of a bounds source read and stated again: a soup restated with what it holds compiles to the file it compiled to, one
//! restated with fewer polygons holds the rest as they were, and what cannot be stated is refused with its place. Needs no game file.

use std::path::{Path, PathBuf};

use or_io::decl::{File, Plain, Raw};
use or_io::gta5::bounds::build::{self, BoundMesh};
use or_io::gta5::bounds::{
    PhBoundAny, PhMaterialMgrId, PhPolygon, PhPrimBox, PhPrimCapsule, PhPrimCylinder, PhPrimSphere,
};
use or_source::asset::Source;
use or_source::bounds::edit::{leaves, write_files, Leaf, Polygon, PolygonKind, Soup};
use or_source::bounds::{self, Inputs, MODEL, RULES};
use or_source::textures::Extraction;

const NAME: &str = "hull";
const MATERIALS: [u64; 2] = [0x11, 0x0000_0001_0000_0022];

fn polygon_of<R: Plain>(raw: &R) -> PhPolygon {
    let p = <Raw<PhPolygon> as Plain>::read_from(Plain::as_bytes(raw)).unwrap();
    PhPolygon {
        area: p.area,
        vertex_indices: p.vertex_indices,
        neighboring_polygons: p.neighboring_polygons,
        ..Default::default()
    }
}

/// A slab of twelve triangles in two materials and then a sphere, a capsule, a box and a cylinder on its corners.
fn slab() -> PhBoundAny {
    let corner = |i: usize| -> [f32; 3] {
        [
            if i & 1 == 0 { -8.0 } else { 8.0 },
            if i & 2 == 0 { -4.0 } else { 4.0 },
            if i & 4 == 0 { -1.0 } else { 1.0 },
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
    let mut g = build::geometry(&BoundMesh {
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
    })
    .unwrap();
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
    for (i, polygon) in [
        polygon_of(&sphere),
        polygon_of(&capsule),
        polygon_of(&boxed),
        polygon_of(&cylinder),
    ]
    .into_iter()
    .enumerate()
    {
        g.polygons.push(polygon);
        g.poly_mat_index_list.push((i % 2) as u8);
    }
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
    PhBoundAny::PhBoundGeometry(g)
}

/// The source of the slab in `parent`, and the canonical save of its file.
fn source(parent: &Path) -> (PathBuf, Vec<u8>) {
    let file = slab().to_bytes().unwrap();
    let canonical = PhBoundAny::from_bytes(&file).unwrap().to_bytes().unwrap();
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    let dir = bounds::extract(&file, NAME, parent, &how).unwrap_or_else(|why| panic!("{why:?}"));
    (dir, canonical)
}

fn inputs(dir: &Path) -> Inputs {
    Inputs::read(&Source::open(dir).unwrap()).unwrap_or_else(|why| panic!("{why:?}"))
}

fn compiled(inputs: &Inputs) -> Vec<u8> {
    inputs
        .compile()
        .unwrap_or_else(|why| panic!("{why:?}"))
        .bytes
}

/// The one leaf of what `inputs` compile to, which is the soup at the root.
fn root(inputs: &Inputs) -> (Leaf, Soup) {
    let bound = PhBoundAny::from_bytes(&compiled(inputs)).unwrap();
    let mut held = leaves(&bound).unwrap();
    assert_eq!(held.len(), 1);
    let leaf = held.remove(0);
    assert_eq!(
        (leaf.address.as_str(), leaf.kind),
        ("bound", "PhBoundGeometry")
    );
    assert!(leaf.placed_by.is_empty(), "no composite places the root");
    let soup = leaf.soup.clone().expect("a geometry is a soup");
    (leaf, soup)
}

fn refusal(restated: Result<Inputs, Vec<or_source::Problem>>) -> String {
    let problems = match restated {
        Err(problems) => problems,
        Ok(_) => panic!("it was restated"),
    };
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// What a polygon states, by the bits of its numbers.
fn stated(polygon: &Polygon) -> (PolygonKind, Option<u8>, Vec<[u32; 3]>, u32) {
    (
        polygon.kind,
        polygon.material,
        polygon.points.iter().map(|p| p.map(f32::to_bits)).collect(),
        polygon.radius.to_bits(),
    )
}

#[test]
fn the_polygons_of_a_soup_are_read_in_the_order_its_bound_holds_them() {
    let work = tempfile::tempdir().unwrap();
    let (dir, _) = source(work.path());
    let (_, soup) = root(&inputs(&dir));
    assert_eq!(soup.materials, MATERIALS);
    let PhBoundAny::PhBoundGeometry(g) = slab() else {
        panic!("a geometry");
    };
    assert_eq!(soup.polygons.len(), g.polygons.len());
    let kinds: Vec<PolygonKind> = soup.polygons.iter().map(|p| p.kind).collect();
    let mut wanted = vec![PolygonKind::Triangle; 12];
    wanted.extend([
        PolygonKind::Sphere,
        PolygonKind::Capsule,
        PolygonKind::Box,
        PolygonKind::Cylinder,
    ]);
    assert_eq!(kinds, wanted);
    for (i, (polygon, held)) in soup.polygons.iter().zip(&g.polygons).enumerate() {
        assert_eq!(polygon.points.len(), polygon.kind.points(), "polygon {i}");
        assert_eq!(polygon.material, Some((i % 2) as u8), "polygon {i}");
        let (indices, radius) = g.polygon_vertices(held).unwrap();
        let points: Vec<[f32; 3]> = indices
            .iter()
            .map(|v| g.vertex_position(*v).unwrap())
            .collect();
        assert_eq!(polygon.points, points, "polygon {i}");
        assert_eq!(polygon.radius, radius, "polygon {i}");
    }
}

#[test]
fn a_soup_restated_with_what_it_holds_compiles_to_the_file_it_compiled_to() {
    let work = tempfile::tempdir().unwrap();
    let (dir, canonical) = source(work.path());
    let held = inputs(&dir);
    assert!(compiled(&held) == canonical);
    let (leaf, soup) = root(&held);
    let again = held.restated(&leaf.address, &soup).unwrap();
    assert!(
        compiled(&again) == canonical,
        "stating the same polygons changes nothing of the file"
    );
    // Written into the folder, the source reads back as what was restated.
    write_files(&dir, &again).unwrap();
    let read = inputs(&dir);
    assert!(read.document() == again.document() && read.model() == again.model());
    assert!(compiled(&read) == canonical);
    let left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".part"))
        .collect();
    assert_eq!(left, Vec::<String>::new(), "no part file is left behind");
}

#[test]
fn a_soup_restated_with_fewer_polygons_holds_the_rest_as_they_were() {
    let work = tempfile::tempdir().unwrap();
    let (dir, canonical) = source(work.path());
    let held = inputs(&dir);
    let (leaf, soup) = root(&held);
    // The third triangle and the sphere go.
    let gone = [2usize, 12];
    let mut fewer = soup.clone();
    fewer.polygons = (soup.polygons.iter().enumerate())
        .filter(|(i, _)| !gone.contains(i))
        .map(|(_, polygon)| polygon.clone())
        .collect();
    let restated = held.restated(&leaf.address, &fewer).unwrap();
    assert!(compiled(&restated) != canonical);
    let (_, now) = root(&restated);
    assert_eq!(now.materials, MATERIALS);
    let mut wanted: Vec<_> = fewer.polygons.iter().map(stated).collect();
    let mut found: Vec<_> = now.polygons.iter().map(stated).collect();
    // ⚠ A soup built again holds its triangles by material, so the two are compared as sets.
    let key = |s: &(PolygonKind, Option<u8>, Vec<[u32; 3]>, u32)| (s.2.clone(), s.3, s.1);
    wanted.sort_by_key(key);
    found.sort_by_key(key);
    assert_eq!(
        found, wanted,
        "each polygon left states what it stated, to the bit"
    );

    // With no triangle left the source holds no glb, and the folder is left with none.
    let mut shapes = soup.clone();
    shapes.polygons.retain(|p| p.kind != PolygonKind::Triangle);
    let restated = held.restated(&leaf.address, &shapes).unwrap();
    assert!(restated.model().is_none());
    assert!(!String::from_utf8_lossy(restated.document()).contains("\"node\""));
    write_files(&dir, &restated).unwrap();
    assert!(!dir.join(MODEL).exists());
    assert!(dir.join(RULES).exists());
    let (_, now) = root(&inputs(&dir));
    assert_eq!(now.polygons.len(), 4);
}

#[test]
fn a_material_past_those_a_soup_listed_is_listed_under_a_name_of_its_own() {
    let work = tempfile::tempdir().unwrap();
    let (dir, _) = source(work.path());
    let held = inputs(&dir);
    let (leaf, mut soup) = root(&held);
    let glass = 0x0000_0002_0000_0033u64;
    soup.materials.push(glass);
    soup.polygons[0].material = Some(2);
    soup.polygons[13].material = Some(2);
    let restated = held.restated(&leaf.address, &soup).unwrap();
    let document: serde_json::Value = serde_json::from_slice(restated.document()).unwrap();
    let names: Vec<&str> = document["bound"]["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["bound_material0", "bound_material1", "bound_material2"]
    );
    let (_, now) = root(&restated);
    assert_eq!(now.materials, [MATERIALS[0], MATERIALS[1], glass]);
    let of_glass = |kind: PolygonKind| {
        (now.polygons.iter())
            .filter(|p| p.kind == kind && p.material == Some(2))
            .count()
    };
    assert_eq!(
        [
            of_glass(PolygonKind::Triangle),
            of_glass(PolygonKind::Capsule)
        ],
        [1, 1]
    );
    assert_eq!(
        now.polygons
            .iter()
            .filter(|p| p.material == Some(2))
            .count(),
        2
    );
}

#[test]
fn what_cannot_be_stated_is_refused_with_its_place() {
    let work = tempfile::tempdir().unwrap();
    let (dir, _) = source(work.path());
    let held = inputs(&dir);
    let (leaf, soup) = root(&held);

    let why = refusal(held.restated("bound_child0", &soup));
    assert!(
        why.contains("holds no polygon soup at `bound_child0`"),
        "{why}"
    );

    let mut short = soup.clone();
    short.polygons[14].points.pop();
    let why = refusal(held.restated(&leaf.address, &short));
    assert!(
        why.contains("bound: polygon 14 is a box on 3 points, and one stands on 4"),
        "{why}"
    );

    let mut unlisted = soup.clone();
    unlisted.polygons[3].material = Some(2);
    let why = refusal(held.restated(&leaf.address, &unlisted));
    assert!(
        why.contains("bound: polygon 3 names material 2 of 2"),
        "{why}"
    );

    // A soup stated with nothing in it is restated, and it is the compile that refuses a bound over nothing.
    let empty = Soup {
        materials: soup.materials.clone(),
        polygons: Vec::new(),
    };
    let restated = held.restated(&leaf.address, &empty).unwrap();
    let why = restated
        .compile()
        .expect_err("a bound over nothing")
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    assert!(why.contains("holds no polygon"), "{why}");
}
