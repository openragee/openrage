use super::*;
use crate::container::rpf::{Rpf9Form, Rpf9Sections};

/// The built child's vertex quantum: its box is 2 m either side of the origin.
const STEP: f32 = 2.0 / 32767.0;
/// The BVH's quanta a metre over the box's 4 m.
const PER_METRE: f32 = 65534.0 / 4.0;
const MARGIN: f32 = 0.04;
/// A unit right triangle on z = 0, and a fourth vertex below and before its first edge.
const AT: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.5, -1.0, -0.5],
];
/// The first triangle's leaf as built, which no edit of the others reaches.
const FIRST_LEAF: [i16; 6] = [-700, -700, -700, 17000, 17000, 700];
const OPEN: u16 = 0xFFFF;

fn quantum(x: f32) -> i16 {
    (f64::from(x) / f64::from(STEP)).round() as i16
}

fn decoded(q: i16) -> f32 {
    (f64::from(q) * f64::from(STEP)) as f32
}

fn node(b: &[u8], nodes: usize, i: usize) -> [i16; 6] {
    std::array::from_fn(|k| {
        let at = nodes + 16 * i + 2 * k;
        i16::from_le_bytes([b[at], b[at + 1]])
    })
}

fn target(b: &[u8], at: usize) -> usize {
    u64_at(b, at) as usize & 0x0FFF_FFFF
}

fn triangle(img: &mut Image, at: usize, vertices: [u16; 3], neighbours: [u16; 3], edges: [u8; 3]) {
    img.put(at, &[edges[0], edges[1], edges[2], 0]);
    for (k, v) in vertices.iter().chain(&neighbours).enumerate() {
        img.u16(at + 4 + 2 * k, *v);
    }
}

/// `img`, whose composite root is at `root`, with a block map of one virtual block, its header and
/// slot, at its end.
fn mapped(mut img: Image, root: usize) -> Rpf9Sections {
    let map = img.alloc(16 + 8);
    img.bytes[map + 8] = 1;
    img.ptr(root + 0x08, map);
    Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: img.bytes,
        physical: Vec::new(),
    }
}

/// The built bound with its child over [`AT`]: triangle 0 over vertices 0, 1, 2 and triangle 1
/// over 1, 3, 2, linked across their edge from 1 to 2, under a BVH of a parent over a leaf each;
/// the sections, where the child lies and where its BVH's nodes lie.
fn two_triangles() -> (Rpf9Sections, usize, usize) {
    let (mut img, parts) = bound();
    let child = child_of(&img, &parts);
    img.vec3(child + 0x20, [2.0; 3]);
    img.vec3(child + 0x30, [-2.0; 3]);
    img.vec3(child + 0x90, [STEP; 3]);
    img.vec3(child + 0xA0, [0.0; 3]);
    img.put(child + 0x2C, &MARGIN.to_le_bytes());
    img.put(child + 0x14, &3.5f32.to_le_bytes());
    img.u16(child + 0x84, AT.len() as u16);
    let vertices = img.alloc(6 * AT.len());
    for (i, v) in AT.iter().enumerate() {
        for (a, x) in v.iter().enumerate() {
            img.u16(vertices + 6 * i + 2 * a, quantum(*x) as u16);
        }
    }
    img.ptr(child + 0xB0, vertices);
    img.u32(child + 0xD0, AT.len() as u32);
    triangle(
        &mut img,
        parts.primitives,
        [0, 1, 2],
        [OPEN, 1, OPEN],
        [0xFF, 9, 0xFF],
    );
    triangle(
        &mut img,
        parts.primitives + 16,
        [1, 3, 2],
        [OPEN, OPEN, 0],
        [0xFF, 0xFF, 9],
    );
    let bvh = target(&img.bytes, child + 0x130);
    img.vec3(bvh + 0x30, [PER_METRE; 3]);
    let nodes = img.alloc(48);
    img.u16(nodes + 0xC, 3);
    for (k, q) in FIRST_LEAF.iter().enumerate() {
        img.u16(nodes + 16 + 2 * k, *q as u16);
    }
    img.u16(nodes + 16 + 0xE, 1);
    img.u16(nodes + 32 + 0xC, 1);
    img.u16(nodes + 32 + 0xE, 1);
    img.ptr(bvh + 0x50, nodes);
    img.u32(bvh + 0x58, 3);
    (mapped(img, parts.root), child, nodes)
}

fn geometry_of(b: &mut Bound) -> &mut BoundGeometry {
    match &mut b.children[0].bound {
        Some(Ok(ChildBound::Geometry(g))) => g,
        other => panic!("a geometry child, not {other:?}"),
    }
}

/// The leaf over the triangles `t`, each over [`AT`] stored to the quantum and grown by the margin,
/// rounded out to whole quanta.
fn leaf_over(t: &[[usize; 3]]) -> [i16; 6] {
    let corners: Vec<[f32; 3]> = (t.iter().flatten())
        .map(|i| AT[*i].map(|x| decoded(quantum(x))))
        .collect();
    let lo: [f32; 3] =
        std::array::from_fn(|a| corners.iter().map(|c| c[a]).fold(f32::MAX, f32::min) - MARGIN);
    let hi: [f32; 3] =
        std::array::from_fn(|a| corners.iter().map(|c| c[a]).fold(f32::MIN, f32::max) + MARGIN);
    std::array::from_fn(|k| match k {
        0..3 => (lo[k] * PER_METRE).floor() as i16,
        _ => (hi[k - 3] * PER_METRE).ceil() as i16,
    })
}

fn joined(a: [i16; 6], b: [i16; 6]) -> [i16; 6] {
    std::array::from_fn(|k| match k {
        0..3 => a[k].min(b[k]),
        _ => a[k].max(b[k]),
    })
}

#[test]
fn a_deleted_primitive_leaves_its_neighbour_open_and_its_emptied_leaf_goes() {
    let (mut s, child, nodes) = two_triangles();
    let len = s.virtual_section.len();
    let mut edited = read_bound(&s.virtual_section).expect("the bound reads");
    delete_bound_primitive(geometry_of(&mut edited), 1).expect("a geometry with a BVH");
    let op = BoundReshape::DeletePrimitive {
        child: 0,
        primitive: 1,
    };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("taken out where it stands");
    assert_eq!(s.virtual_section.len(), len, "nothing is laid out");
    let read = only_geometry(&s.virtual_section).expect("it reads back");
    assert_eq!(
        read.primitives,
        [Primitive::Triangle {
            vertices: [0, 1, 2],
            neighbours: [OPEN; 3],
            edges: [0xFF; 3],
        }]
    );
    assert_eq!(read.primitives, geometry_of(&mut edited).primitives);
    let bvh = read.bvh.expect("a BVH");
    assert_eq!((bvh.nodes, bvh.leaves), (1, 1));
    assert_eq!(
        node(&s.virtual_section, nodes, 0),
        FIRST_LEAF,
        "the sibling takes its parent's place"
    );
    assert_eq!(u32_at(&s.virtual_section, child + 0xD4), 1);

    let before = s.virtual_section.clone();
    write_bound(&mut s.virtual_section, &edited).expect("nothing is left to write");
    assert_eq!(s.virtual_section, before);
    let mut last = read_bound(&s.virtual_section).expect("it reads back");
    assert!(
        delete_bound_primitive(geometry_of(&mut last), 0).is_err(),
        "a geometry keeps one primitive"
    );
}

#[test]
fn an_added_triangle_links_across_the_edges_it_shares_and_joins_the_last_leaf() {
    let (mut s, child, nodes) = two_triangles();
    let len = s.virtual_section.len();
    let mut edited = read_bound(&s.virtual_section).expect("the bound reads");
    let g = geometry_of(&mut edited);
    let added = Primitive::Triangle {
        vertices: [0, 3, 1],
        neighbours: [OPEN; 3],
        edges: [0xFF; 3],
    };
    assert_eq!(add_bound_primitive(g, added, 0).expect("inside the box"), 2);
    let record = g.primitives[2];
    let Primitive::Triangle { neighbours, .. } = record else {
        panic!("a triangle, not {record:?}");
    };
    assert_eq!(
        neighbours,
        [OPEN, 1, 0],
        "its edge from 3 to 1 is triangle 1's, from 1 to 0 triangle 0's"
    );
    let op = BoundReshape::AddPrimitive {
        child: 0,
        record,
        material: 0,
    };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("laid out past the end");
    assert!(s.virtual_section.len() > len);
    write_bound(&mut s.virtual_section, &edited).expect("the neighbours' links write in place");
    let read = only_geometry(&s.virtual_section).expect("it reads back");
    assert_eq!(read.primitives, geometry_of(&mut edited).primitives);
    assert_eq!(u32_at(&s.virtual_section, child + 0xD4), 3);
    let bvh = read.bvh.expect("a BVH");
    assert_eq!((bvh.nodes, bvh.leaves), (3, 2), "the last leaf holds both");
    let last = leaf_over(&[[1, 3, 2], [0, 3, 1]]);
    assert_eq!(node(&s.virtual_section, nodes, 2), last);
    assert_eq!(node(&s.virtual_section, nodes, 1), FIRST_LEAF);
    assert_eq!(node(&s.virtual_section, nodes, 0), joined(FIRST_LEAF, last));

    // Triangle 1 deleted from the leaf of two: the leaf keeps the new one, numbered 1 now.
    delete_bound_primitive(geometry_of(&mut edited), 1).expect("a geometry with a BVH");
    let op = BoundReshape::DeletePrimitive {
        child: 0,
        primitive: 1,
    };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("taken out where it stands");
    let read = only_geometry(&s.virtual_section).expect("it reads back");
    assert_eq!(read.primitives, geometry_of(&mut edited).primitives);
    let Primitive::Triangle {
        vertices,
        neighbours,
        edges,
    } = read.primitives[1]
    else {
        panic!("a triangle, not {:?}", read.primitives[1]);
    };
    assert_eq!((vertices, neighbours), ([0, 3, 1], [OPEN, OPEN, 0]));
    assert_eq!(
        edges[..2],
        [0xFF, 0xFF],
        "the edge across the deleted one is open"
    );
    let bvh = read.bvh.expect("a BVH");
    assert_eq!((bvh.nodes, bvh.leaves), (3, 2));
    assert_eq!(node(&s.virtual_section, nodes, 2), leaf_over(&[[0, 3, 1]]));
}

#[test]
fn an_added_sphere_is_stored_as_its_record_and_joins_the_last_leaf() {
    let (mut s, child, _) = two_triangles();
    let mut edited = read_bound(&s.virtual_section).expect("the bound reads");
    let sphere = Primitive::Sphere {
        centre: 0,
        radius: 0.5,
    };
    let g = geometry_of(&mut edited);
    assert_eq!(
        add_bound_primitive(g, sphere, 0).expect("inside the box"),
        2
    );
    let op = BoundReshape::AddPrimitive {
        child: 0,
        record: sphere,
        material: 0,
    };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("laid out past the end");
    let read = only_geometry(&s.virtual_section).expect("it reads back");
    assert_eq!(read.primitives, geometry_of(&mut edited).primitives);
    assert_eq!(read.primitives[2], sphere);
    assert_eq!(u32_at(&s.virtual_section, child + 0xD4), 3);
    let bvh = read.bvh.expect("a BVH");
    assert_eq!((bvh.nodes, bvh.leaves), (3, 2), "the last leaf holds both");
    let past = Primitive::Sphere {
        centre: 0,
        radius: 3.0,
    };
    assert!(
        add_bound_primitive(geometry_of(&mut edited), past, 0).is_err(),
        "past the child's box"
    );
}

#[test]
fn an_added_vertex_is_placed_by_the_save_s_move_and_a_named_one_is_not_deleted() {
    let (mut s, child, _) = two_triangles();
    let mut edited = read_bound(&s.virtual_section).expect("the bound reads");
    let g = geometry_of(&mut edited);
    assert!(
        delete_bound_vertex(g, 3).is_err(),
        "triangle 1 names vertex 3"
    );
    let place = [0.25, 0.5, -0.25];
    assert_eq!(add_bound_vertex(g, place).expect("a finite position"), 4);
    let op = BoundReshape::AddVertex { child: 0 };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("laid out past the end");
    let v = &s.virtual_section;
    assert_eq!(u32_at(v, child + 0xD0), 5);
    assert_eq!(
        u16::from_le_bytes([v[child + 0x84], v[child + 0x85]]),
        5,
        "the hull's count follows the vertex count"
    );
    write_bound_vertices(&mut s.virtual_section, &edited).expect("the vertex moves to its place");
    let read = only_geometry(&s.virtual_section).expect("it reads back");
    assert_eq!(read.vertices.len(), 5);
    for a in 0..3 {
        assert!(
            (read.vertices[4][a] - place[a]).abs() <= STEP,
            "lane {a}: {:?}",
            read.vertices[4]
        );
    }

    let mut again = read_bound(&s.virtual_section).expect("it reads back");
    delete_bound_vertex(geometry_of(&mut again), 4).expect("no primitive names it");
    let op = BoundReshape::DeleteVertex {
        child: 0,
        vertex: 4,
    };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("taken out where it stands");
    let read = only_geometry(&s.virtual_section).expect("it reads back");
    assert_eq!(read.vertices.len(), 4);
    assert_eq!(read.vertices, geometry_of(&mut again).vertices);
    assert_eq!(read.primitives, geometry_of(&mut again).primitives);
}

#[test]
fn an_edit_that_lays_out_bytes_in_a_body_with_no_block_map_is_refused_and_changes_nothing() {
    let (mut s, ..) = two_triangles();
    s.virtual_section[0x08..0x10].fill(0);
    let kept = s.clone();
    let op = BoundReshape::AddVertex { child: 0 };
    let err = reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect_err("no block map");
    assert!(matches!(err, BoundGrowError::Grow(_)), "{err}");
    assert_eq!(s, kept);

    let mut edited = read_bound(&kept.virtual_section).expect("the bound reads");
    let g = geometry_of(&mut edited);
    g.bvh = None;
    assert!(
        add_bound_vertex(g, [0.0; 3]).is_err(),
        "no BVH: shrunk vertices"
    );
    assert!(
        delete_bound_primitive(g, 0).is_err(),
        "no BVH: shrunk vertices"
    );
}

/// A lone bound's composite of two unit boxes, the second placed 2 m along x, its box their union,
/// under a BVH of a parent over a leaf each, the leaves naming the children in reverse.
fn two_boxes() -> Rpf9Sections {
    let mut img = Image::default();
    let root = boxes_into(&mut img);
    mapped(img, root)
}

/// The composite of [`two_boxes`] laid out in `img`; where its root lies.
fn boxes_into(img: &mut Image) -> usize {
    let root = img.alloc(0xB0);
    img.bytes[root + 0x10] = COMPOSITE;
    img.bytes[root + 0x11] = 1;
    img.put(root + 0x14, &2.0f32.to_le_bytes());
    img.vec3(root + 0x20, [2.5, 0.5, 0.5]);
    img.vec3(root + 0x30, [-0.5; 3]);
    img.vec3(root + 0x40, [1.0, 0.0, 0.0]);
    img.u16(root + 0x98, 2);
    img.u16(root + 0x9A, 2);
    let mut records = Vec::new();
    for _ in 0..2 {
        let at = img.alloc(0x70);
        img.bytes[at + 0x10] = 3;
        img.put(at + 0x14, &0.75f32.sqrt().to_le_bytes());
        img.vec3(at + 0x20, [0.5; 3]);
        img.vec3(at + 0x30, [-0.5; 3]);
        records.push(at);
    }
    let pointers = img.pointers(&records);
    img.ptr(root + 0x70, pointers);
    let matrices = img.alloc(128);
    for (i, x) in [0.0f32, 2.0].into_iter().enumerate() {
        for r in 0..4 {
            img.put(matrices + 64 * i + 20 * r, &1.0f32.to_le_bytes());
        }
        img.put(matrices + 64 * i + 48, &x.to_le_bytes());
    }
    img.ptr(root + 0x78, matrices);
    img.ptr(root + 0x80, matrices);
    let flags = img.alloc(32);
    img.u64(flags, 0x11);
    img.u64(flags + 16, 0x22);
    img.ptr(root + 0x88, flags);
    let bvh = img.alloc(0x70);
    img.vec3(bvh, [-0.5; 3]);
    img.vec3(bvh + 0x10, [2.5, 0.5, 0.5]);
    img.vec3(bvh + 0x30, [1000.0; 3]);
    let nodes = img.alloc(48);
    img.u16(nodes + 0xC, 3);
    for (i, (first, lo, hi)) in [(1u16, 10i16, 20i16), (0, -20, -10)]
        .into_iter()
        .enumerate()
    {
        let n = nodes + 16 * (i + 1);
        for (k, q) in [lo, -5, -5, hi, 5, 5].into_iter().enumerate() {
            img.u16(n + 2 * k, q as u16);
        }
        img.u16(n + 0xC, first);
        img.u16(n + 0xE, 1);
    }
    img.ptr(bvh + 0x50, nodes);
    img.u32(bvh + 0x58, 3);
    img.u32(bvh + 0x5C, 3);
    img.ptr(root + 0xA0, bvh);
    root
}

#[test]
fn a_copied_child_is_the_composite_s_last_at_its_place_with_a_leaf_of_its_own() {
    let mut s = two_boxes();
    let mut edited = read_bound(&s.virtual_section).expect("the composite reads");
    assert_eq!(
        copy_bound_child(&mut edited, 1).expect("a primitive child"),
        2
    );
    let op = BoundReshape::CopyChild { child: 1 };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("laid out past the end");
    write_bound(&mut s.virtual_section, &edited).expect("nothing else to write");
    let read = read_bound(&s.virtual_section).expect("it reads back");
    assert_eq!(read.children.len(), 3);
    assert_eq!(read.children[2].matrix, read.children[1].matrix);
    assert_eq!(read.children[2].flags, Some([0x22, 0]));
    assert!(matches!(
        read.children[2].bound,
        Some(Ok(ChildBound::Box { .. }))
    ));
    let bvh = read.bvh.expect("a BVH");
    assert_eq!((bvh.nodes, bvh.leaves), (5, 3));
    let v = &s.virtual_section;
    assert_eq!(target(v, 0x78), target(v, 0x80), "one array of matrices");
    let nodes = target(v, target(v, 0xA0) + 0x50);
    let (second, first) = ([10, -5, -5, 20, 5, 5], [-20, -5, -5, -10, 5, 5]);
    assert_eq!(
        node(v, nodes, 4),
        second,
        "the copy's leaf boxes its source"
    );
    assert_eq!(node(v, nodes, 3), first);
    assert_eq!(node(v, nodes, 2), joined(first, second));
    assert_eq!(u16::from_le_bytes([v[nodes + 0xC], v[nodes + 0xD]]), 5);

    let mut lone = read_bound(&two_triangles().0.virtual_section).expect("it reads");
    geometry_of(&mut lone).bvh = None;
    assert_eq!(
        copy_bound_child(&mut lone, 0).expect("a geometry with no BVH is copied"),
        1
    );
}

#[test]
fn a_deleted_child_takes_its_leaf_and_the_composite_s_box_fits_the_rest() {
    let mut s = two_boxes();
    let mut edited = read_bound(&s.virtual_section).expect("the composite reads");
    delete_bound_child(&mut edited, 0).expect("a child is left");
    assert_eq!(
        (edited.box_min, edited.box_max),
        ([1.5, -0.5, -0.5], [2.5, 0.5, 0.5])
    );
    assert_eq!(
        edited.centroid,
        [2.0, 0.0, 0.0],
        "the old centroid lies outside"
    );
    let op = BoundReshape::DeleteChild { child: 0 };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("taken out where it stands");
    write_bound(&mut s.virtual_section, &edited).expect("the box writes in place");
    let read = read_bound(&s.virtual_section).expect("it reads back");
    assert_eq!(read.children.len(), 1);
    assert_eq!(read.children[0].flags, Some([0x22, 0]));
    assert_eq!(read.children[0].matrix[3][0], 2.0);
    assert_eq!(
        (read.box_min, read.box_max, read.centroid),
        (edited.box_min, edited.box_max, edited.centroid)
    );
    let bvh = read.bvh.expect("a BVH");
    assert_eq!((bvh.nodes, bvh.leaves), (1, 1));
    assert!(
        delete_bound_child(&mut edited, 0).is_err(),
        "a composite keeps one child"
    );
    let mut s = two_boxes();
    let op = BoundReshape::DeleteChild { child: 0 };
    let err = reshape_bound_body(&mut s, BoundHolder::Fragment, &op).expect_err("a fragment");
    assert!(err.to_string().contains("fragment child record"), "{err}");
}

#[test]
fn a_child_given_another_type_is_laid_out_anew_at_that_type_s_size() {
    let mut s = two_boxes();
    let len = s.virtual_section.len();
    let mut edited = read_bound(&s.virtual_section).expect("the composite reads");
    let Some(Ok(shape)) = &edited.children[1].bound else {
        panic!("a shape, not {:?}", edited.children[1].bound);
    };
    assert!(
        retyped_bound_child(shape, 13).is_err(),
        "no box gives a hull's corners"
    );
    let sphere = retyped_bound_child(shape, 0).expect("a box becomes a sphere");
    let ChildBound::Sphere { centre, radius } = sphere else {
        panic!("a sphere, not {sphere:?}");
    };
    assert_eq!((centre, radius), ([0.0; 3], 0.5));
    edited.children[1].bound = Some(Ok(sphere));
    fit_composite_box(&mut edited).expect("every child's box is read");
    write_retyped(&mut s, BoundHolder::Bound, &edited).expect("laid out past the end");
    assert!(s.virtual_section.len() > len);
    write_bound(&mut s.virtual_section, &edited).expect("the rest writes in place");
    let read = read_bound(&s.virtual_section).expect("it reads back");
    let Some(Ok(ChildBound::Sphere { centre, radius })) = &read.children[1].bound else {
        panic!("a sphere, not {:?}", read.children[1].bound);
    };
    assert_eq!((*centre, *radius), ([0.0; 3], 0.5));
    assert_eq!(read.children[1].flags, Some([0x22, 0]));
    let before = s.clone();
    write_retyped(&mut s, BoundHolder::Bound, &edited).expect("no type changes now");
    assert_eq!(s, before);
}

/// A drawable whose 0xC8 names the composite of [`two_boxes`]: its sections, and where the
/// composite lies.
fn drawable_with_boxes() -> (Rpf9Sections, usize) {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let root = boxes_into(&mut img);
    img.ptr(parts.root + 0xC8, root);
    let s = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: img.bytes,
        physical: phys,
    };
    (s, root)
}

fn drawable_bound(s: &Rpf9Sections) -> Bound {
    BoundHolder::Drawable
        .read(s)
        .expect("the drawable's composite reads")
}

#[test]
fn a_drawable_s_composite_has_a_child_copied_and_deleted_as_a_lone_bound_s() {
    let (mut s, root) = drawable_with_boxes();
    let mut edited = drawable_bound(&s);
    assert_eq!(
        copy_bound_child(&mut edited, 1).expect("a primitive child"),
        2
    );
    let op = BoundReshape::CopyChild { child: 1 };
    reshape_bound_body(&mut s, BoundHolder::Drawable, &op).expect("laid out past the end");
    let read = drawable_bound(&s);
    assert_eq!(read.children.len(), 3);
    assert_eq!(read.children[2].matrix, read.children[1].matrix);
    assert_eq!(read.children[2].flags, Some([0x22, 0]));
    assert_eq!(read.bvh.map(|b| (b.nodes, b.leaves)), Some((5, 3)));
    let v = &s.virtual_section;
    assert_eq!(
        target(v, root + 0x78),
        target(v, root + 0x80),
        "one array of matrices"
    );

    let op = BoundReshape::DeleteChild { child: 0 };
    reshape_bound_body(&mut s, BoundHolder::Drawable, &op).expect("taken out where it stands");
    let read = drawable_bound(&s);
    assert_eq!(read.children.len(), 2);
    for c in &read.children {
        assert_eq!((c.flags, c.matrix[3][0]), (Some([0x22, 0]), 2.0));
    }
    assert_eq!(read.bvh.map(|b| (b.nodes, b.leaves)), Some((3, 2)));
}

#[test]
fn a_child_edit_is_refused_where_a_drawable_s_bound_is_not_laid_out_as_a_lone_bound_s() {
    let refused = |s: &mut Rpf9Sections| {
        let kept = s.clone();
        let op = BoundReshape::CopyChild { child: 0 };
        let err = reshape_bound_body(s, BoundHolder::Drawable, &op).expect_err("refused");
        assert_eq!(*s, kept, "{err}");
        err.to_string()
    };
    // +0x80 null: the reader takes it, and no lone bound is so laid out.
    let (mut s, root) = drawable_with_boxes();
    s.virtual_section[root + 0x80..root + 0x88].fill(0);
    drawable_bound(&s);
    assert!(refused(&mut s).contains("lone bound's"));

    // A box alone at 0xC8, which the reader takes as a composite of it.
    let (mut s, root) = drawable_with_boxes();
    let first = target(&s.virtual_section, target(&s.virtual_section, root + 0x70));
    let word = (0x5000_0000 + first as u64).to_le_bytes();
    s.virtual_section[0xC8..0xD0].copy_from_slice(&word);
    assert_eq!(drawable_bound(&s).children.len(), 1);
    assert!(refused(&mut s).contains("lone bound's"));
}

#[path = "write_bound_runs.rs"]
mod write_bound_runs;

#[path = "write_bound_plain_copy.rs"]
mod write_bound_plain_copy;
