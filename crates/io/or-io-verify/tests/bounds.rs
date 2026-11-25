//! `.ybn` and `.ybd` through their declarations, over the corpus: typed reads
//! against the tree, the owned form against the tree's save, the recorded
//! testimony re-keyed by offset, the canonical fixed point and the alignment
//! census; then what the game assumes of a bound, and what a fragment's
//! bounds hold that no `.ybn` does: octant maps, discs and cylinders; then a
//! bound built from a mesh, a rebuilt tree, the refusals a save makes, and a
//! bound authored from nothing.

use std::collections::BTreeMap;
use std::mem::{offset_of, size_of};
use std::path::PathBuf;

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::decl::{Described, File, Open, Raw, Resource, View, ViewError};
use or_io::desc::value::{read_using, ElementTypes};
use or_io::desc::{Image, TypeName};
use or_io::gta5::bounds::build::{self, BoundMesh, EDGE_VERTICES};
use or_io::gta5::bounds::{
    self, BoundType, Color32, CompressedVertex, PhBound, PhBoundAny, PhBoundBVH, PhBoundBox,
    PhBoundComposite, PhBoundCylinder, PhBoundDisc, PhBoundGeometry, PhBoundSphere, PhOptimizedBvh,
    PhPolygon, NO_NEIGHBOUR, PRIMITIVE_TYPE_MASK, VERTEX_INDEX_MASK,
};
use or_io::gta5::shared::{PgDictionary, Vec4V};
use or_io::Generation;
use or_io_verify::gates::{self, file_tree, files, name, Filed, Gates, Opened};
use or_test_support::Corpus;

const T: Generation = Generation::Gen8;

type E = ViewError;

mod bound_walk;
use bound_walk::{bytes, put, walk};

fn typed_ybn(image: Image<'_>) -> Result<Filed, String> {
    let mut out = Filed::default();
    let root = View::<Raw<PhBound>>::at_offset(image, 0).map_err(|e| e.to_string())?;
    walk(image, root, &mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn typed_ybd(image: Image<'_>) -> Result<Filed, String> {
    let e = |e: E| e.to_string();
    let mut out = Filed::default();
    let dict = View::<Raw<PgDictionary<PhBoundAny>>>::at_offset(image, 0).map_err(e)?;
    put(&mut out, &dict);
    bytes(image, &mut out, &dict.codes().map_err(e)?);
    let entries = dict.entries().map_err(e)?;
    for i in 0..entries.len() {
        if let Some(bound) = entries.follow(i).map_err(e)? {
            walk(image, bound, &mut out).map_err(e)?;
        }
    }
    Ok(out)
}

const YBN: Gates = Gates {
    census_types: 20,
    ..Gates::new("ybn", T, ("PhOptimizedBvhNode", "node_data"))
};

const YBD: Gates = Gates {
    census_types: 5,
    ..Gates::new("ybd", T, ("PhBound", "radius_around_centroid"))
};

or_io_verify::declared_gates! {
    gates: YBN,
    typed: typed_ybn,
    reads_typed: every_ybn_reads_typed_as_the_tree_reads,
    owned: PhBoundAny => every_ybn_opens_owned_and_saves_as_its_tree_saves,
    fixed_point: every_ybn_saves_to_a_fixed_point,
    census: every_ybn_instance_is_aligned_for_its_view,
    testimony: the_ybn_testimony_holds_through_its_declarations,
    one_byte_late: a_ybn_field_one_byte_late_is_named_by_the_testimony,
}

or_io_verify::declared_gates! {
    gates: YBD,
    typed: typed_ybd,
    reads_typed: every_ybd_reads_typed_as_the_tree_reads,
    owned: PgDictionary<PhBoundAny> => every_ybd_opens_owned_and_saves_as_its_tree_saves,
    fixed_point: every_ybd_saves_to_a_fixed_point,
    census: every_ybd_instance_is_aligned_for_its_view,
    testimony: the_ybd_testimony_holds_through_its_declarations,
    one_byte_late: a_ybd_field_one_byte_late_is_named_by_the_testimony,
}

#[test]
fn a_polygon_whose_tag_names_no_primitive_is_refused() {
    let Some(paths) = files("ybn", Some(T), "a retyped polygon refused") else {
        return;
    };
    fn first_polygon(b: View<'_, Raw<PhBound>>) -> Option<usize> {
        if let Some(g) = View::variant::<Raw<PhBoundBVH>>(&b).ok()? {
            return g.base().polygons().ok()?.view(0).map(|p| View::offset(&p));
        }
        let c = View::variant::<Raw<PhBoundComposite>>(&b).ok()??;
        let children = c.bounds().ok()?;
        (0..children.len()).find_map(|i| first_polygon(children.follow(i).ok()??))
    }
    let ybn = gates::format("ybn");
    for path in &paths {
        let opened = Opened::read(path);
        let root = View::<Raw<PhBound>>::at_offset(opened.image(), 0).unwrap();
        let Some(polygon) = first_polygon(root) else {
            continue;
        };
        let mut bytes = opened.bytes.bytes().to_vec();
        bytes[polygon] = (bytes[polygon] & !0b111) | 0b111;
        let aligned = or_io::decl::Aligned::from_bytes(&bytes);
        let image = Image::new(aligned.bytes(), opened.virtual_size);
        let tree = ybn.read(T, image).expect("reads");
        let why = ybn
            .save(T, &tree)
            .expect_err("a retyped polygon")
            .to_string();
        println!("{}: {why}", name(path));
        assert!(why.contains("names no primitive"), "{why}");
        return;
    }
    panic!("no BVH bound in the corpus to retype a polygon of");
}

#[test]
fn a_ybd_at_a_version_nothing_declares_is_left_out_by_name() {
    let ybd = gates::format("ybd");
    for v in [40, 42] {
        let why = ybd.target_of_version(v).unwrap_err().to_string();
        assert!(why.contains(&format!("version {v}")), "{why}");
    }
}

/// Every staged `.ybd` at an RSC7 version the format refuses, with its version.
fn refused_ybd(what: &str) -> Option<Vec<(PathBuf, u32)>> {
    let ybd = gates::format("ybd");
    let paths = Corpus::with_root(gates::corpus_root()?, "ybd")
        .all_targets()
        .find_or_skip(what)?;
    let out: Vec<_> = paths
        .into_iter()
        .filter_map(|p| {
            let version = Rsc7::peek(&std::fs::read(&p).ok()?).ok()?.version();
            ybd.target_of_version(version)
                .is_err()
                .then_some((p, version))
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

#[test]
fn a_ybd_at_40_or_42_is_refused_naming_what_its_layout_lacks() {
    let Some(refused) = refused_ybd("a .ybd at 40 or 42 refused by name") else {
        return;
    };
    for (path, version) in &refused {
        let why = match PgDictionary::<PhBoundAny>::open(path) {
            Ok(_) => panic!("{} at {version} opened", name(path)),
            Err(e) => e.to_string(),
        };
        println!("{} at {version}: {why}", name(path));
        assert!(why.contains(&format!("version {version}")), "{why}");
        for missing in [
            "num_per_vertex_attribs",
            "octant_vert_counts",
            "octant_verts",
        ] {
            assert!(why.contains(missing), "{why}");
        }
    }
}

/// ⚠ Every `.ybd` at 40 or 42 is Legacy's, and the corpus stages all three.
#[test]
fn a_ybd_at_40_or_42_misreads_its_geometries_at_43s_layout() {
    let Some(refused) = refused_ybd("a .ybd at 40 or 42 measured") else {
        return;
    };
    let (mut geometries, mut others) = (0usize, 0usize);
    for (path, version) in &refused {
        let opened = Opened::read(path);
        let (image, bytes) = (opened.image(), opened.bytes.bytes());
        let dict = View::<Raw<PgDictionary<PhBoundAny>>>::at_offset(image, 0).unwrap();
        let entries = dict.entries().unwrap();
        let mut pending: Vec<_> = (0..entries.len())
            .filter_map(|i| entries.follow(i).unwrap())
            .collect();
        while let Some(b) = pending.pop() {
            let g = match (
                View::variant::<Raw<PhBoundGeometry>>(&b).unwrap(),
                View::variant::<Raw<PhBoundBVH>>(&b).unwrap(),
            ) {
                (Some(g), _) => g,
                (_, Some(bvh)) => bvh.base(),
                _ => {
                    if let Some(c) = View::variant::<Raw<PhBoundComposite>>(&b).unwrap() {
                        let children = c.bounds().unwrap();
                        pending.extend(
                            (0..children.len()).filter_map(|i| children.follow(i).unwrap()),
                        );
                    } else {
                        bounds::check_bound(b, "").unwrap();
                    }
                    others += 1;
                    continue;
                }
            };
            assert_eq!(g.num_vertices, 0, "{} at {version}", name(path));
            assert!(!{ g.compressed_vertices }.is_null());
            // The counts, moved: 0x10 more after the header at 40, the hull's
            // u16 two bytes early and the vertex count eight.
            let at = View::offset(&g) + if *version == 40 { 0x10 } else { 0 };
            let hull = at + offset_of!(Raw<PhBoundGeometry>, num_convex_hull_vertices) - 2;
            let vertices = at + offset_of!(Raw<PhBoundGeometry>, num_vertices) - 8;
            let counts = vertices;
            let hull = u16::from_le_bytes(bytes[hull..hull + 2].try_into().unwrap());
            let vertices = i32::from_le_bytes(bytes[vertices..vertices + 4].try_into().unwrap());
            assert!(vertices > 0, "{} at {version}", name(path));
            assert_eq!(i32::from(hull), vertices, "{} at {version}", name(path));
            // From the vertex attributes to the materials, 0x10 shorter: no
            // byte but the counts is set, so no octant pointer can be placed.
            let attribs = at + offset_of!(Raw<PhBoundGeometry>, vertex_attribs);
            let materials = at + offset_of!(Raw<PhBoundGeometry>, material_ids) - 0x10;
            assert_eq!(bytes[materials + 3] >> 4, 5, "a live `material_ids`");
            for (i, byte) in bytes[attribs..materials].iter().enumerate() {
                let in_counts = (counts..counts + 8).contains(&(attribs + i));
                assert!(in_counts || *byte == 0, "{} at {version}", name(path));
            }
            if *version == 40 {
                let after = View::offset(&g) + size_of::<Raw<PhBound>>();
                let words: Vec<u32> = bytes[after..after + 0x10]
                    .chunks(4)
                    .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
                    .collect();
                let x = 0.04f32.to_bits();
                assert_eq!(words, [x, x, x, 1], "{}", name(path));
            }
            geometries += 1;
        }
    }
    println!(
        "{} files: {geometries} geometries misread at 43's slots, {others} other bounds read clean",
        refused.len()
    );
    assert!(geometries > 0 && others > 0);
}

/// Every geometry a bound holds, a BVH's and a composite's children's included.
fn geometries<'b>(b: &'b PhBoundAny, out: &mut Vec<&'b PhBoundGeometry>) {
    match b {
        PhBoundAny::PhBoundGeometry(g) => out.push(g),
        PhBoundAny::PhBoundBVH(b) => out.push(&b.base),
        PhBoundAny::PhBoundComposite(c) => {
            for child in c.bounds.iter().flatten() {
                geometries(child, out);
            }
        }
        _ => {}
    }
}

/// Every `.ybn` opened owned, by file name.
///
/// ⚠ **Every refusal is fatal.** Skipping a file that will not open lets a
/// present corpus yield nothing, and every invariant below then passes having
/// opened nothing.
fn owned_ybn(what: &str) -> Option<Vec<(String, PhBoundAny)>> {
    let paths = files("ybn", Some(T), what)?;
    let out: Vec<_> = paths
        .iter()
        .map(|p| {
            let bound = PhBoundAny::open(p).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
            (name(p), bound)
        })
        .collect();
    assert!(!out.is_empty());
    Some(out)
}

#[test]
fn every_polygon_indexes_a_vertex_that_exists() {
    let Some(bounds) = owned_ybn("the polygon and vertex index invariant") else {
        return;
    };
    let (mut checked, mut wrong) = (0usize, Vec::new());
    for (file, bound) in &bounds {
        let mut found = Vec::new();
        geometries(bound, &mut found);
        for g in found {
            let vertices = g.compressed_vertices.len();
            for (i, p) in g.polygons.iter().enumerate() {
                if p.primitive() != 0 {
                    continue;
                }
                checked += 1;
                for (corner, v) in p.vertex_indices.iter().enumerate() {
                    let index = usize::from(v & VERTEX_INDEX_MASK);
                    if index >= vertices {
                        wrong.push(format!(
                            "{file}: polygon {i} corner {corner} names vertex {index} of {vertices}"
                        ));
                    }
                }
            }
        }
    }
    println!("{checked} polygons checked against their geometry's vertices");
    assert!(checked > 0, "no polygon in the corpus; nothing was checked");
    wrong.truncate(10);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_material_index_is_in_range() {
    let Some(bounds) = owned_ybn("the material index invariant") else {
        return;
    };
    let (mut checked, mut wrong) = (0usize, Vec::new());
    for (file, bound) in &bounds {
        let mut found = Vec::new();
        geometries(bound, &mut found);
        for g in found {
            let Some(&worst) = g.poly_mat_index_list.iter().max() else {
                continue;
            };
            checked += 1;
            if usize::from(worst) >= g.material_ids.len() {
                wrong.push(format!(
                    "{file}: material index {worst} of {}",
                    g.material_ids.len()
                ));
            }
        }
    }
    println!("{checked} geometries' material indices checked");
    assert!(checked > 0, "no geometry carried material indices");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_geometry_with_vertices_can_unquantize_them() {
    let Some(bounds) = owned_ybn("the unquantize factor invariant") else {
        return;
    };
    let (mut checked, mut wrong) = (0usize, Vec::new());
    for (file, bound) in &bounds {
        let mut found = Vec::new();
        geometries(bound, &mut found);
        for g in found
            .into_iter()
            .filter(|g| !g.compressed_vertices.is_empty())
        {
            checked += 1;
            let f = &g.un_quantize_factor;
            if f.x == 0.0 || f.y == 0.0 || f.z == 0.0 {
                wrong.push(format!(
                    "{file}: {} vertices and an unquantize factor of ({}, {}, {})",
                    g.compressed_vertices.len(),
                    f.x,
                    f.y,
                    f.z
                ));
            }
        }
    }
    println!("{checked} geometries' unquantize factors checked");
    assert!(checked > 0, "no geometry carried vertices");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_composite_fills_no_more_slots_than_it_allocated() {
    let Some(paths) = files("ybn", Some(T), "the composite capacity invariant") else {
        return;
    };
    let (mut checked, mut wrong) = (0usize, Vec::new());
    for path in &paths {
        let file = std::fs::read(path).expect("a corpus file reads");
        let peek = PhBoundAny::peek(&file).unwrap();
        let root = peek.root().unwrap();
        let Some(c) = View::variant::<Raw<PhBoundComposite>>(&root).unwrap() else {
            continue;
        };
        checked += 1;
        if c.num_bounds > c.max_num_bounds {
            wrong.push(format!(
                "{}: {} children in {} allocated slots",
                name(path),
                c.num_bounds,
                c.max_num_bounds
            ));
        }
        let owned = PhBoundAny::from_bytes(&file).unwrap();
        let children = owned.as_composite().map_or(0, |c| c.bounds.len());
        if children != usize::from(c.num_bounds) {
            wrong.push(format!(
                "{}: num_bounds says {} and {children} were read",
                name(path),
                c.num_bounds
            ));
        }
    }
    println!("{checked} composites checked");
    assert!(
        checked > 0,
        "no composite in the corpus; nothing was checked"
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Where a fragment's tree files each bound of `kinds`.
fn fragment_bounds(image: Image<'_>, kinds: &[&str]) -> Vec<usize> {
    let Ok(tree) = gates::format("yft").read(T, image) else {
        return Vec::new();
    };
    let mut filed = Filed::default();
    file_tree(image, &tree, Some(0), &mut filed);
    filed
        .structs
        .keys()
        .filter(|(_, ty)| kinds.contains(&ty.as_str()))
        .map(|(at, _)| *at)
        .collect()
}

/// The bound at `at`, read through the `.ybn` description and opened owned.
fn owned_bound_at(image: Image<'_>, at: usize) -> PhBoundAny {
    let pack = gates::pack("ybn", T);
    let root = TypeName(or_io::decl::intern(&Raw::<PhBound>::name()));
    let tree = read_using(
        &pack,
        root,
        image,
        at,
        &ElementTypes::new(),
        &bounds::readers(),
    )
    .expect("a fragment's bound reads through the .ybn description");
    <PhBoundAny as or_io::decl::Owned>::from_element(&tree).expect("opens owned")
}

/// Only a fragment holds octant maps: 0 of the `.ybn` corpus's 74 geometries
/// carries one. So each fragment geometry is opened owned, saved as a `.ybn`
/// of its own and reopened, and its map compared run by run.
#[test]
fn the_octant_map_survives_a_save() {
    let Some(paths) = files("yft", Some(T), "the octant map across an owned save") else {
        return;
    };
    let (mut geometries, mut with_map, mut runs, mut indices, mut in_range, mut one_block) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let opened = Opened::read(path);
        let image = opened.image();
        for at in fragment_bounds(image, &["PhBoundGeometry", "PhBoundBVH"]) {
            geometries += 1;
            let bound = owned_bound_at(image, at);
            let geometry = |b: &PhBoundAny| -> Option<PhBoundGeometry> {
                b.as_geometry()
                    .cloned()
                    .or_else(|| b.as_bvh().map(|b| b.base.clone()))
            };
            let before = geometry(&bound).expect("a geometry");
            let Some(map) = &before.octant_verts else {
                continue;
            };
            with_map += 1;
            let saved = bound
                .to_bytes()
                .unwrap_or_else(|e| panic!("{} at {at:#x}: {e}", name(path)));
            let again = PhBoundAny::from_bytes(&saved).unwrap();
            let after = geometry(&again).expect("a geometry");
            assert_eq!(
                after.octant_verts.as_deref(),
                Some(&**map),
                "{} at {at:#x}: the octant map changed across a save",
                name(path)
            );
            for octant in &map.octants {
                runs += 1;
                indices += octant.len();
                if octant
                    .iter()
                    .all(|v| (*v as usize) < before.compressed_vertices.len())
                {
                    in_range += 1;
                }
            }
            let peek = PhBoundAny::peek(&saved).unwrap();
            let root = peek.root().unwrap();
            let g = View::variant::<Raw<PhBoundGeometry>>(&root)
                .unwrap()
                .or_else(|| {
                    View::variant::<Raw<PhBoundBVH>>(&root)
                        .unwrap()
                        .map(|b| b.base())
                })
                .expect("a geometry");
            let (counts, table) = ({ g.octant_vert_counts }.raw(), { g.octant_verts }.raw());
            if counts != 0 && table == counts + size_of::<[u32; bounds::OCTANT_COUNT]>() as u64 {
                one_block += 1;
            }
        }
    }
    println!(
        "{geometries} fragment geometries, {with_map} with an octant map: {runs} runs and \
         {indices} indices kept across an owned save, {in_range} runs inside their vertices, \
         {one_block} saved as one block"
    );
    assert!(
        with_map > 0,
        "no fragment geometry carried an octant map, so this compared nothing"
    );
    assert_eq!(
        in_range, runs,
        "a run names a vertex past the end, so the map is read from the wrong place and the \
         equality above is two identical misreadings"
    );
    assert_eq!(
        one_block, with_map,
        "a saved map does not put its table just past its counts, the shape every shipped \
         map has"
    );
}

/// Discs and cylinders occur only in a fragment's physics.
fn discs_and_cylinders(what: &str) -> Option<Vec<(String, Opened, Vec<usize>)>> {
    let paths = files("yft", Some(T), what)?;
    Some(
        paths
            .iter()
            .map(|p| {
                let opened = Opened::read(p);
                let found = fragment_bounds(opened.image(), &["PhBoundDisc", "PhBoundCylinder"]);
                (name(p), opened, found)
            })
            .collect(),
    )
}

/// ⚠ A slot declared to hold nothing has held live data before (the octant
/// map, the joint limits, the vehicle glass), so the bytes decide: a pad a
/// save writes back is measured, not assumed.
#[test]
fn the_pad_of_a_disc_and_a_cylinder_is_measured_not_assumed() {
    let Some(found) = discs_and_cylinders("the trailing pad of a disc and a cylinder") else {
        return;
    };
    let (mut discs, mut cylinders, mut control, mut live) = (0usize, 0usize, 0usize, Vec::new());
    for (file, opened, ats) in &found {
        let image = opened.image();
        for &at in ats {
            let b = View::<Raw<PhBound>>::at_offset(image, at).unwrap();
            let pad = if let Some(d) = View::variant::<Raw<PhBoundDisc>>(&b).unwrap() {
                discs += 1;
                control += usize::from({ d.base.r#type }.get() == BoundType::Disc);
                { d.pad }.to_vec()
            } else if let Some(c) = View::variant::<Raw<PhBoundCylinder>>(&b).unwrap() {
                cylinders += 1;
                control += usize::from({ c.base.r#type }.get() == BoundType::Cylinder);
                { c.pad }.to_vec()
            } else {
                panic!("{file} at {at:#x}: neither a disc nor a cylinder");
            };
            if pad.iter().any(|b| *b != 0) {
                live.push(format!("{file} at {at:#x}: {pad:02x?}"));
            }
        }
    }
    println!(
        "{discs} discs and {cylinders} cylinders; the control reads its own type in {control}; \
         {} hold a non-zero pad byte",
        live.len()
    );
    assert!(
        discs + cylinders > 0,
        "no disc or cylinder was found, so this measured nothing"
    );
    assert_eq!(control, discs + cylinders, "the walk reads the wrong bytes");
    assert!(
        live.is_empty(),
        "a disc's or cylinder's pad holds data, so it is a field the declaration has to name:\n{}",
        live.join("\n")
    );
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1e-3)
}

/// A cylinder's radius and half-height are its header's, not words of its
/// own: its own sixteen bytes are zero and its bounding box reads as a
/// cylinder along Y, in every one; every disc's header reads as a disc along X.
#[test]
fn a_cylinder_does_not_carry_its_own_dimensions() {
    let Some(found) = discs_and_cylinders("what a disc and a cylinder measure") else {
        return;
    };
    let (mut discs, mut disc_shaped, mut cylinders, mut cylinder_shaped, mut own_zero) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for (_, opened, ats) in &found {
        let image = opened.image();
        for &at in ats {
            let b = View::<Raw<PhBound>>::at_offset(image, at).unwrap();
            let (max, min) = ({ b.bounding_box_max_xyz_margin_w }, {
                b.bounding_box_min_xyz_ref_count_w
            });
            let [x, y, z] = [
                (max.x - min.x) / 2.0,
                (max.y - min.y) / 2.0,
                (max.z - min.z) / 2.0,
            ];
            let radius = b.radius_around_centroid;
            if let Some(c) = View::variant::<Raw<PhBoundCylinder>>(&b).unwrap() {
                cylinders += 1;
                own_zero += usize::from({ c.pad }.iter().all(|b| *b == 0));
                if x > 0.0 && close(x, z) && close(radius, (x * x + y * y).sqrt()) {
                    cylinder_shaped += 1;
                }
            } else if View::variant::<Raw<PhBoundDisc>>(&b).unwrap().is_some() {
                discs += 1;
                if close(y, z) && close(radius, y) && close(max.w, x) {
                    disc_shaped += 1;
                }
            }
        }
    }
    println!(
        "{cylinders} cylinders: own words zero in {own_zero}, the header reads as a cylinder in \
         {cylinder_shaped}; {discs} discs: the header reads as a disc in {disc_shaped}"
    );
    assert!(
        cylinders > 0,
        "no cylinder was reached, so this measured nothing"
    );
    assert_eq!(own_zero, cylinders);
    assert_eq!(cylinder_shaped, cylinders);
    assert_eq!(disc_shaped, discs);
}

/// The largest cosine between two of three edges, and the shortest edge; an edge of no length is at right angles to all.
fn worst_cosine(edges: [[f32; 3]; 3]) -> (f32, f32) {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cosine = [(0, 1), (0, 2), (1, 2)]
        .into_iter()
        .map(|(i, j)| {
            let lengths = (dot(edges[i], edges[i]) * dot(edges[j], edges[j])).sqrt();
            if lengths > 0.0 {
                dot(edges[i], edges[j]).abs() / lengths
            } else {
                0.0
            }
        })
        .fold(0.0, f32::max);
    let shortest = edges
        .iter()
        .map(|e| dot(*e, *e).sqrt())
        .filter(|l| *l > 0.0)
        .fold(f32::MAX, f32::min);
    (cosine, shortest)
}

/// A box primitive's four vertices are corners no edge of the box joins, so
/// the sums and differences of the four are its three edges, at right angles.
/// The rival reading, one corner and the three an edge joins it to, is
/// measured beside it: the file states neither.
#[test]
fn a_box_primitive_names_four_corners_no_edge_joins() {
    let what = "what a box primitive's four vertices are";
    let (Some(ybn), Some(yft)) = (owned_ybn(what), files("yft", Some(T), what)) else {
        return;
    };
    let mut bounds: Vec<PhBoundAny> = ybn.into_iter().map(|(_, b)| b).collect();
    for path in &yft {
        let opened = Opened::read(path);
        let image = opened.image();
        for at in fragment_bounds(image, &["PhBoundGeometry", "PhBoundBVH"]) {
            bounds.push(owned_bound_at(image, at));
        }
    }
    let (mut boxes, mut apart, mut adjacent, mut rival_nearer) = (0usize, 0usize, 0usize, 0usize);
    for bound in &bounds {
        let mut found = Vec::new();
        geometries(bound, &mut found);
        for g in found {
            let f = &g.un_quantize_factor;
            let quantum = (f.x * f.x + f.y * f.y + f.z * f.z).sqrt();
            for p in g.polygons.iter().filter(|p| p.primitive() == 3) {
                let (indices, _) = g.polygon_vertices(p).expect("a box primitive");
                let v: Vec<[f32; 3]> = indices
                    .iter()
                    .map(|i| g.vertex_position(*i).expect("a vertex the bound holds"))
                    .collect();
                let mix = |s: [f32; 4]| -> [f32; 3] {
                    std::array::from_fn(|a| (0..4).map(|k| s[k] * v[k][a]).sum::<f32>() * 0.5)
                };
                let (no_edge, shortest) = worst_cosine([
                    mix([1.0, 1.0, -1.0, -1.0]),
                    mix([1.0, -1.0, 1.0, -1.0]),
                    mix([1.0, -1.0, -1.0, 1.0]),
                ]);
                let one_corner = (0..4)
                    .map(|corner| {
                        let others: Vec<[f32; 3]> = (0..4)
                            .filter(|k| *k != corner)
                            .map(|k| std::array::from_fn(|a| v[k][a] - v[corner][a]))
                            .collect();
                        worst_cosine([others[0], others[1], others[2]]).0
                    })
                    .fold(f32::MAX, f32::min);
                boxes += 1;
                // ⚠ A vertex is sixteen bits an axis, so each of the four is up to half a quantum
                // off and an edge, half their sum, up to a whole one: a short edge turns by that
                // over its length.
                let slack = 1e-3 + 2.0 * quantum / shortest;
                apart += usize::from(no_edge <= slack);
                adjacent += usize::from(one_corner <= slack);
                rival_nearer += usize::from(one_corner < no_edge);
            }
        }
    }
    println!(
        "{boxes} box primitives: {apart} are corners no edge joins, {adjacent} one corner and \
         its three neighbours, and the second reading is the squarer in {rival_nearer}"
    );
    assert!(
        boxes > 0,
        "no box primitive was reached, so this measured nothing"
    );
    assert_eq!(apart, boxes);
    assert_eq!(rival_nearer, 0);
}

/// Every polygon's first word and every vertex, through the typed API from the root.
#[inline(never)]
fn typed_sum(b: View<'_, Raw<PhBound>>) -> u64 {
    let mut total = 0u64;
    let mut geometry = |g: View<'_, Raw<PhBoundGeometry>>| {
        if let Ok(p) = g.polygons() {
            total += p
                .iter()
                .map(|p| u64::from({ p.area }.to_bits() & PRIMITIVE_TYPE_MASK))
                .sum::<u64>();
        }
        if let Ok(v) = g.compressed_vertices() {
            total += v
                .iter()
                .map(|v| (v.x as u16 as u64) + (v.z as u16 as u64))
                .sum::<u64>();
        }
    };
    let mut stack = vec![b];
    while let Some(b) = stack.pop() {
        if let Ok(Some(v)) = View::variant::<Raw<PhBoundBVH>>(&b) {
            geometry(v.base());
        } else if let Ok(Some(g)) = View::variant::<Raw<PhBoundGeometry>>(&b) {
            geometry(g);
        } else if let Ok(Some(c)) = View::variant::<Raw<PhBoundComposite>>(&b) {
            if let Ok(children) = c.bounds() {
                stack.extend((0..children.len()).filter_map(|i| children.follow(i).ok().flatten()));
            }
        }
    }
    total
}

#[test]
#[ignore = "a timing, meaningful only in a release build; run by name"]
fn what_a_ybn_read_costs() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};
    let Some(paths) = files("ybn", Some(T), "the .ybn read cost") else {
        return;
    };
    let opened: Vec<Opened> = paths.iter().map(|p| Opened::read(p)).collect();
    let mut instances = BTreeMap::<String, usize>::new();
    for o in &opened {
        for (_, ty) in typed_ybn(o.image()).unwrap().structs.keys() {
            *instances.entry(ty.clone()).or_default() += 1;
        }
    }
    let polygons = instances.get("PhPolygon").copied().unwrap_or(0);
    let best = |rounds: usize, f: &mut dyn FnMut() -> u64| {
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let start = Instant::now();
            let mut sink = 0u64;
            for _ in 0..rounds {
                sink = sink.wrapping_add(f());
            }
            black_box(sink);
            best = best.min(start.elapsed());
        }
        best.as_secs_f64() * 1e3 / rounds as f64
    };
    let typed_ms = best(50, &mut || {
        opened
            .iter()
            .map(|o| typed_sum(View::<Raw<PhBound>>::at_offset(black_box(o.image()), 0).unwrap()))
            .sum()
    });
    let tree_ms = best(3, &mut || {
        opened
            .iter()
            .map(|o| {
                u64::from(
                    gates::format("ybn")
                        .read(T, o.image())
                        .unwrap()
                        .record()
                        .is_some(),
                )
            })
            .sum()
    });
    println!(
        "{} files, {polygons} polygons: every polygon tag and vertex typed from the root \
         {typed_ms:.3} ms; the whole tree read {tree_ms:.2} ms ({:.0}x)",
        paths.len(),
        tree_ms / typed_ms
    );
}

/// Every BVH bound under a root, however deep the composites go.
fn bvhs<'b>(b: &'b PhBoundAny, out: &mut Vec<&'b PhBoundBVH>) {
    match b {
        PhBoundAny::PhBoundBVH(b) => out.push(b),
        PhBoundAny::PhBoundComposite(c) => {
            for child in c.bounds.iter().flatten() {
                bvhs(child, out);
            }
        }
        _ => {}
    }
}

/// A polygon's bytes but its neighbours, which are what a rebuild may renumber.
fn without_neighbours(p: &PhPolygon) -> (u32, [u16; 3]) {
    (p.area.to_bits(), p.vertex_indices)
}

/// How many triangle slots name a triangle across the edge [`EDGE_VERTICES`] gives the slot.
fn edges_shared(polygons: &[PhPolygon]) -> usize {
    let edge = |p: &PhPolygon, k: usize| {
        let c = p.vertex_indices.map(|v| v & VERTEX_INDEX_MASK);
        let (a, b) = EDGE_VERTICES[k];
        (c[a].min(c[b]), c[a].max(c[b]))
    };
    let mut shared = 0;
    for p in polygons.iter().filter(|p| p.primitive() == 0) {
        for (k, n) in p.neighboring_polygons.iter().enumerate() {
            let Some(other) = polygons.get(usize::from(*n)) else {
                continue;
            };
            if other.primitive() == 0 && (0..3).any(|j| edge(other, j) == edge(p, k)) {
                shared += 1;
            }
        }
    }
    shared
}

/// What the traversal assumes of a tree over a geometry's polygons, as the failures it finds.
///
/// ⚠ No slack on containment: it is a property of the integers the builder
/// writes, checked against the decompression the engine performs.
fn traversal(g: &PhBoundGeometry, tree: &PhOptimizedBvh) -> Vec<String> {
    let mut wrong = Vec::new();
    let nodes = &tree.contiguous_nodes;
    let mut next = 0usize;
    for (i, node) in nodes.iter().enumerate() {
        if node.polygon_count == 0 {
            let skip = usize::from(node.node_data);
            if skip < 3 || i + skip > nodes.len() {
                wrong.push(format!("node {i} escapes {skip} of {} nodes", nodes.len()));
            }
            continue;
        }
        if usize::from(node.node_data) != next {
            wrong.push(format!(
                "leaf {i} starts at {} where the last ended at {next}",
                node.node_data
            ));
            break;
        }
        let (lo, hi) = tree.node_box(node);
        for at in next..next + usize::from(node.polygon_count) {
            let Some((min, max)) = g.polygons.get(at).and_then(|p| g.polygon_box(p)) else {
                wrong.push(format!("leaf {i} covers polygon {at}, which has no box"));
                continue;
            };
            if (0..3).any(|a| min[a] < lo[a] || max[a] > hi[a]) {
                wrong.push(format!(
                    "leaf {i} does not contain polygon {at}: {min:?}..{max:?} against {lo:?}..{hi:?}"
                ));
            }
        }
        next += usize::from(node.polygon_count);
    }
    if next != g.polygons.len() {
        wrong.push(format!(
            "the leaves cover {next} of {} polygons",
            g.polygons.len()
        ));
    }
    wrong
}

/// A report of what the shipped trees do; it asserts that a tree was reached
/// and that every polygon's tag names one of the five primitives.
#[test]
fn what_the_shipped_trees_say_about_the_rules() {
    let Some(bounds) = owned_ybn("what the shipped trees say") else {
        return;
    };
    let (mut trees, mut nodes, mut leaves, mut contain, mut polygons) = (0, 0, 0, 0, 0);
    let mut by_primitive = BTreeMap::<u32, usize>::new();
    for (_, root) in &bounds {
        let mut found = Vec::new();
        bvhs(root, &mut found);
        for b in found {
            let g = &b.base;
            for p in &g.polygons {
                polygons += 1;
                *by_primitive.entry(p.primitive()).or_default() += 1;
            }
            let Some(tree) = &b.bvh else {
                continue;
            };
            trees += 1;
            nodes += tree.contiguous_nodes.len();
            let q = &tree.inv_quantize;
            let slack = [q.x, q.y, q.z];
            for node in tree.contiguous_nodes.iter().filter(|n| n.polygon_count > 0) {
                leaves += 1;
                let (lo, hi) = tree.node_box(node);
                let first = usize::from(node.node_data);
                // A quantum of slack: a face on a polygon rounds either way.
                let holds = (first..first + usize::from(node.polygon_count)).all(|at| {
                    g.polygons
                        .get(at)
                        .and_then(|p| g.polygon_box(p))
                        .is_some_and(|(min, max)| {
                            (0..3).all(|a| min[a] >= lo[a] - slack[a] && max[a] <= hi[a] + slack[a])
                        })
                });
                contain += usize::from(holds);
            }
        }
    }
    println!(
        "{} files, {trees} trees, {nodes} nodes, {leaves} leaves; {contain} leaf boxes contain \
         every polygon under them; {polygons} polygons by primitive: {by_primitive:?}",
        bounds.len()
    );
    assert!(trees > 0, "no BVH bound reached, so nothing was measured");
    assert!(
        by_primitive.keys().all(|p| *p <= 4),
        "a shipped polygon's tag names no primitive: {by_primitive:?}"
    );
}

/// A rebuilt tree covers its polygons, and the permutation carries the material
/// indices and the adjacency with them.
#[test]
fn a_rebuilt_tree_covers_the_polygons_it_was_built_over() {
    let Some(bounds) = owned_ybn("a rebuilt tree over the shipped polygons") else {
        return;
    };
    let (mut rebuilt, mut polygons, mut adjacency) = (0usize, 0usize, 0usize);
    let mut wrong = Vec::new();
    for (file, root) in &bounds {
        let mut found = Vec::new();
        bvhs(root, &mut found);
        for before in found.into_iter().filter(|b| !b.base.polygons.is_empty()) {
            let mut after = before.clone();
            if let Err(why) = build::rebuild(&mut after) {
                wrong.push(format!("{file}: {why}"));
                continue;
            }
            rebuilt += 1;
            let (was, now) = (&before.base, &after.base);
            polygons += now.polygons.len();

            let sorted = |g: &PhBoundGeometry| {
                let mut v: Vec<_> = g.polygons.iter().map(without_neighbours).collect();
                v.sort_unstable();
                v
            };
            if sorted(was) != sorted(now) {
                wrong.push(format!(
                    "{file}: the rebuilt polygons are not a permutation of the originals"
                ));
            }
            let tree = after.bvh.as_deref().expect("a rebuild states a tree");
            wrong.extend(
                traversal(now, tree)
                    .into_iter()
                    .take(3)
                    .map(|w| format!("{file}: {w}")),
            );
            let mut materials = [
                was.poly_mat_index_list.clone(),
                now.poly_mat_index_list.clone(),
            ];
            materials.iter_mut().for_each(|m| m.sort_unstable());
            if materials[0] != materials[1] {
                wrong.push(format!(
                    "{file}: the material indices are not a permutation of the originals"
                ));
            }
            let (shared, still) = (edges_shared(&was.polygons), edges_shared(&now.polygons));
            adjacency += shared;
            if still != shared {
                wrong.push(format!(
                    "{file}: {shared} neighbour slots named a triangle across their edge, and {still} do after"
                ));
            }
            if let Err(why) = PhBoundAny::PhBoundBVH(after).to_bytes() {
                wrong.push(format!("{file}: the rebuilt bound does not save: {why}"));
            }
        }
    }
    println!(
        "{rebuilt} trees rebuilt over {polygons} polygons; {adjacency} neighbour slots followed \
         the permutation"
    );
    assert!(rebuilt > 0, "nothing was rebuilt");
    assert!(
        adjacency > 0,
        "no neighbour slot named a triangle across its edge, so the renumbering is unexercised"
    );
    wrong.truncate(12);
    assert!(wrong.is_empty(), "\n  {}", wrong.join("\n  "));
}

/// Slot `k` of `neighboring_polygons` is the triangle across edge
/// `(v[k], v[k+1])`: scored against the other two rotations, it has to win outright.
#[test]
fn a_neighbour_slot_names_the_edge_this_workspace_says_it_does() {
    let Some(bounds) = owned_ybn("which edge a neighbour slot is about") else {
        return;
    };
    let candidates: [[(usize, usize); 3]; 3] = [
        EDGE_VERTICES,
        [(1, 2), (2, 0), (0, 1)],
        [(2, 0), (0, 1), (1, 2)],
    ];
    let (mut agreed, mut checked) = ([0usize; 3], 0usize);
    for (_, root) in &bounds {
        let mut found = Vec::new();
        geometries(root, &mut found);
        for g in found {
            for (i, p) in g.polygons.iter().enumerate() {
                if p.primitive() != 0 {
                    continue;
                }
                let mine = p.vertex_indices.map(|v| v & VERTEX_INDEX_MASK);
                for (slot, n) in p.neighboring_polygons.iter().enumerate() {
                    let Some(other) = g.polygons.get(usize::from(*n)) else {
                        continue;
                    };
                    if other.primitive() != 0 || usize::from(*n) == i {
                        continue;
                    }
                    let theirs = other.vertex_indices.map(|v| v & VERTEX_INDEX_MASK);
                    checked += 1;
                    for (c, layout) in candidates.iter().enumerate() {
                        let (a, b) = layout[slot];
                        let edge = (mine[a].min(mine[b]), mine[a].max(mine[b]));
                        agreed[c] += usize::from(EDGE_VERTICES.iter().any(|(x, y)| {
                            (theirs[*x].min(theirs[*y]), theirs[*x].max(theirs[*y])) == edge
                        }));
                    }
                }
            }
        }
    }
    let pct = |n: usize| 100.0 * n as f64 / checked.max(1) as f64;
    println!(
        "{checked} neighbour claims: (0,1)(1,2)(2,0) {:.2}%, (1,2)(2,0)(0,1) {:.2}%, \
         (2,0)(0,1)(1,2) {:.2}%",
        pct(agreed[0]),
        pct(agreed[1]),
        pct(agreed[2])
    );
    assert!(checked > 1000, "only {checked} neighbour claims reached");
    assert!(agreed[0] > agreed[1] && agreed[0] > agreed[2], "{agreed:?}");
    assert!(pct(agreed[0]) > 99.0, "{:.2}%", pct(agreed[0]));
}

/// A bound built from nothing but a mesh saves, opens back, and its tree holds.
#[test]
fn a_mesh_becomes_a_bound_that_survives_a_save() {
    // A closed tetrahedron: every edge is shared, so a wrong slot shows as a wrong index.
    let mesh = BoundMesh::new(
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ],
        vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    );
    let bound = build::bvh(&mesh).expect("a mesh is a bound");
    let bytes = PhBoundAny::PhBoundBVH(bound).to_bytes().expect("it saves");
    let back = PhBoundAny::from_bytes(&bytes).expect("it opens");
    let b = back.as_bvh().expect("a mesh builds a BVH bound");
    let g = &b.base;
    assert_eq!(g.compressed_vertices.len(), 4);
    assert_eq!(g.polygons.len(), 4);
    for (i, want) in mesh.positions.iter().enumerate() {
        let got = g.vertex_position(i).expect("a vertex");
        assert!(
            (0..3).all(|a| (got[a] - want[a]).abs() < 1e-4),
            "{got:?} for {want:?}"
        );
    }
    assert!(
        g.polygons
            .iter()
            .all(|p| p.neighboring_polygons.iter().all(|n| *n != NO_NEIGHBOUR)),
        "every edge of a closed tetrahedron is shared, so no slot may be empty"
    );
    assert_eq!(
        edges_shared(&g.polygons),
        12,
        "each slot names the triangle across its edge"
    );
    let tree = b.bvh.as_deref().expect("the tree survives the save");
    let wrong = traversal(g, tree);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// No shipped bound is refused: a refusal that stops a real file saving is a bug in the refusal.
#[test]
fn every_shipped_bound_still_saves() {
    let Some(bounds) = owned_ybn("every shipped bound saves") else {
        return;
    };
    let mut refused = Vec::new();
    for (file, b) in &bounds {
        if let Err(why) = b.to_bytes() {
            refused.push(format!("{file}: {why}"));
        }
    }
    let kinds = [
        "PhBoundSphere",
        "PhBoundCapsule",
        "PhBoundBox",
        "PhBoundGeometry",
        "PhBoundBVH",
        "PhBoundComposite",
        "PhBoundDisc",
        "PhBoundCylinder",
    ];
    let mut fragments = 0usize;
    if let Some(paths) = files("yft", Some(T), "every fragment bound saves as a .ybn") {
        for path in &paths {
            let opened = Opened::read(path);
            let image = opened.image();
            for at in fragment_bounds(image, &kinds) {
                fragments += 1;
                if let Err(why) = owned_bound_at(image, at).to_bytes() {
                    refused.push(format!("{} at {at:#x}: {why}", name(path)));
                }
            }
        }
        assert!(fragments > 0, "the fragment walk reached no bound");
    }
    println!(
        "{} .ybn roots and {fragments} fragment bounds saved",
        bounds.len()
    );
    refused.truncate(12);
    assert!(refused.is_empty(), "\n  {}", refused.join("\n  "));
}

/// The first BVH bound in the corpus that `pick` takes, checked to save as it was opened.
fn a_bvh(what: &str, pick: impl Fn(&PhBoundBVH) -> bool) -> Option<(String, PhBoundBVH)> {
    let bounds = owned_ybn(what)?;
    let (file, b) = bounds
        .iter()
        .find_map(|(file, root)| {
            let mut found = Vec::new();
            bvhs(root, &mut found);
            found
                .into_iter()
                .find(|b| pick(b))
                .map(|b| (file.clone(), b.clone()))
        })
        .unwrap_or_else(|| panic!("no BVH bound in the corpus for {what}"));
    PhBoundAny::PhBoundBVH(b.clone())
        .to_bytes()
        .unwrap_or_else(|e| panic!("{file}: the bound as opened does not save: {e}"));
    Some((file, b))
}

/// Why saving `b` once `edit` has run is refused.
fn refusal(file: &str, mut b: PhBoundBVH, edit: impl FnOnce(&mut PhBoundBVH)) -> String {
    edit(&mut b);
    let why = PhBoundAny::PhBoundBVH(b)
        .to_bytes()
        .expect_err("the edit has to be refused")
        .to_string();
    println!("{file}: {why}");
    why
}

fn has_triangle(b: &PhBoundBVH) -> bool {
    b.base.polygons.iter().any(|p| p.primitive() == 0)
}

fn first_triangle(b: &mut PhBoundBVH) -> &mut PhPolygon {
    b.base
        .polygons
        .iter_mut()
        .find(|p| p.primitive() == 0)
        .expect("a triangle")
}

#[test]
fn an_untagged_polygon_is_refused() {
    let Some((file, b)) = a_bvh("an untagged polygon", has_triangle) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        let p = first_triangle(b);
        p.area = f32::from_bits((p.area.to_bits() & !PRIMITIVE_TYPE_MASK) | 0b101);
    });
    assert!(why.contains("names no primitive"), "{why}");
}

#[test]
fn a_polygon_past_the_vertex_array_is_refused() {
    let Some((file, b)) = a_bvh("a polygon past the vertices", has_triangle) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        let past = b.base.compressed_vertices.len() as u16;
        first_triangle(b).vertex_indices[0] = past;
    });
    assert!(why.contains("names vertex"), "{why}");
}

#[test]
fn a_neighbour_past_the_polygon_array_is_refused() {
    let Some((file, b)) = a_bvh("a neighbour past the polygons", has_triangle) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        let past = b.base.polygons.len() as u16;
        first_triangle(b).neighboring_polygons[0] = past;
    });
    assert!(why.contains("names neighbour"), "{why}");
}

/// ⚠ Two runs, one indexing the other, and nothing on the read path comparing them.
#[test]
fn a_polygon_naming_a_material_that_is_not_there_is_refused() {
    let Some((file, b)) = a_bvh("a material past the materials", |b| {
        !b.base.poly_mat_index_list.is_empty()
    }) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        b.base.poly_mat_index_list[0] = b.base.material_ids.len() as u8;
    });
    assert!(why.contains("names material"), "{why}");
}

#[test]
fn a_bvh_leaf_past_the_polygon_array_is_refused() {
    let Some((file, b)) = a_bvh("a leaf past the polygons", |b| {
        b.bvh.is_some() && b.base.polygons.len() > 8
    }) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        b.base.polygons.truncate(4);
        b.base.poly_mat_index_list.truncate(4);
    });
    assert!(why.contains("BVH leaf"), "{why}");
}

#[test]
fn a_bvh_subtree_past_the_node_array_is_refused() {
    let Some((file, b)) = a_bvh("a subtree past the nodes", |b| {
        b.bvh
            .as_ref()
            .is_some_and(|t| !t.subtree_headers.is_empty())
    }) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        let tree = b.bvh.as_mut().expect("a tree");
        tree.subtree_headers[0].last_index = tree.contiguous_nodes.len() as u16 + 1;
    });
    assert!(why.contains("BVH subtree"), "{why}");
}

/// The runs `num_vertices` counts: a colour run of another length has no count to be written at.
#[test]
fn a_vertex_colour_run_of_its_own_length_is_refused() {
    let Some((file, b)) = a_bvh("a colour run of its own length", |b| {
        b.base.compressed_vertices.len() > 4
    }) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        b.base.vertex_attribs = vec![Color32::default(); b.base.compressed_vertices.len() - 1];
    });
    assert!(
        why.contains("vertex_attribs") && why.contains("num_vertices"),
        "{why}"
    );
}

/// ⚠ The shrunk hull has a count of its own, and it is one vertex per vertex.
#[test]
fn a_shrunk_hull_of_its_own_length_is_refused() {
    let Some((file, b)) = a_bvh("a shrunk hull of its own length", |b| {
        b.base.compressed_vertices.len() > 4
    }) else {
        return;
    };
    let why = refusal(&file, b, |b| {
        b.base.compressed_shrunk_vertices =
            vec![CompressedVertex::default(); b.base.compressed_vertices.len() - 1];
    });
    assert!(why.contains("shrunk hull"), "{why}");
}

/// ⚠ Five runs are counted by `num_bounds`: a child pushed without its transform has none.
#[test]
fn a_composite_child_pushed_without_its_runs_is_refused() {
    let Some(bounds) = owned_ybn("a child pushed without its runs") else {
        return;
    };
    let (file, mut root) = bounds
        .into_iter()
        .find(|(_, b)| {
            b.as_composite()
                .is_some_and(|c| !c.bounds.is_empty() && !c.current_matrices.is_empty())
        })
        .expect("a composite with children and transforms");
    let c = root.as_composite_mut().expect("a composite");
    let child = c.bounds[0].clone();
    c.bounds.push(child);
    let why = root
        .to_bytes()
        .expect_err("a child with no transform")
        .to_string();
    println!("{file}: {why}");
    assert!(why.contains("num_bounds"), "{why}");
}

fn corner(v: Raw<Vec4V>) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// A sphere authored from nothing, read back through its view rather than the reader that wrote it.
#[test]
fn a_sphere_authored_from_nothing_reads_back_through_its_view() {
    const RADIUS: f32 = 12.5;
    const MAX: [f32; 3] = [3.25, 4.5, 5.75];
    const MIN: [f32; 3] = [-3.25, -4.5, -5.75];
    let mut sphere = PhBoundSphere::default();
    sphere.base.radius_around_centroid = RADIUS;
    sphere.base.bounding_box_max_xyz_margin_w = Vec4V {
        x: MAX[0],
        y: MAX[1],
        z: MAX[2],
        ..Default::default()
    };
    sphere.base.set_bounding_box_min(MIN);
    let bytes = PhBoundAny::PhBoundSphere(sphere)
        .to_bytes()
        .expect("an authored sphere saves");
    let peek = PhBoundAny::peek(&bytes).unwrap();
    let root = peek.root().unwrap();
    assert_eq!({ root.r#type }.get(), BoundType::Sphere);
    assert_eq!({ root.radius_around_centroid }.to_bits(), RADIUS.to_bits());
    assert_eq!(corner(root.bounding_box_max_xyz_margin_w), MAX);
    assert_eq!(corner(root.bounding_box_min_xyz_ref_count_w), MIN);
}

/// ⚠ A variant does not stamp its tag: a box left at the default tag is
/// refused naming what a reader would bind, rather than saved as a sphere.
#[test]
fn an_authored_box_saves_only_under_the_tag_its_variant_takes() {
    let mut boxed = PhBoundBox::default();
    let why = PhBoundAny::PhBoundBox(boxed.clone())
        .to_bytes()
        .expect_err("a box whose tag says sphere")
        .to_string();
    assert!(
        why.contains("PhBoundBox") && why.contains("PhBoundSphere"),
        "{why}"
    );
    boxed.base.r#type = Open::new(BoundType::Box);
    let bytes = PhBoundAny::PhBoundBox(boxed)
        .to_bytes()
        .expect("an authored box saves");
    let peek = PhBoundAny::peek(&bytes).unwrap();
    let root = peek.root().unwrap();
    assert_eq!({ root.r#type }.get(), BoundType::Box);
}

#[test]
fn an_edit_to_an_owned_bound_reaches_the_file() {
    const AFTER: f32 = 1234.5;
    let Some(bounds) = owned_ybn("an edit reaching the file") else {
        return;
    };
    let (file, mut root) = bounds
        .into_iter()
        .find(|(_, b)| b.as_composite().is_some())
        .expect("a composite root");
    let header = &mut root.as_composite_mut().expect("a composite").base;
    assert_ne!(header.radius_around_centroid.to_bits(), AFTER.to_bits());
    let part_index = header.part_index;
    header.radius_around_centroid = AFTER;
    let bytes = root.to_bytes().expect("an edited bound saves");
    let peek = PhBoundAny::peek(&bytes).unwrap();
    let saved = peek.root().unwrap();
    assert_eq!(
        { saved.radius_around_centroid }.to_bits(),
        AFTER.to_bits(),
        "{file}: the edit did not reach the file"
    );
    assert_eq!(
        { saved.part_index },
        part_index,
        "{file}: a neighbour of the edited field changed too"
    );
}
