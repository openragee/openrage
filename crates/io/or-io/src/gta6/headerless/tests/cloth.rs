use super::*;
use crate::gta6::hash::name_hash32;

const OWNER: u32 = 0x7D8F_3DB3;

/// A cloth head of type `kind` over its root of `root` bytes, its block map after the root.
fn head(kind: u8, root: usize) -> Image {
    let mut img = Image::default();
    img.alloc(root);
    let map = img.alloc(0x10);
    img.bytes[map + 8] = 1;
    img.bytes[map + 9] = u8::from(kind == 3);
    img.ptr(0x08, map);
    img.bytes[0x1A] = kind;
    img.u32(0x1C, OWNER);
    img
}

/// The array of 16-byte `records` past the root, stated at `at`.
fn array16(img: &mut Image, at: usize, records: &[[u32; 4]]) {
    let target = img.alloc(16 * records.len());
    for (i, r) in records.iter().enumerate() {
        for (k, w) in r.iter().enumerate() {
            img.u32(target + 16 * i + 4 * k, *w);
        }
    }
    img.ptr(at, target);
    img.u16(at + 8, records.len() as u16);
    img.u16(at + 10, records.len() as u16);
}

fn f(x: f32) -> u32 {
    x.to_bits()
}

const VERTICES: [[f32; 4]; 3] = [
    [0.0, 0.0, 0.0, 1.0],
    [1.0, 0.0, 0.0, 0.5],
    [0.0, 2.0, 0.0, 0.25],
];

fn edges_body() -> Image {
    let mut img = head(2, 0x7A0);
    let vertices: Vec<[u32; 4]> = VERTICES.iter().map(|v| v.map(f)).collect();
    array16(&mut img, 0x28, &vertices);
    let edges = [
        [1 << 16, f(1.0), f(0.5), 7],
        [1 | 2 << 16, f(5f32.sqrt()), f(0.25), 9],
        [0, f(1.0), 0, 0],
    ];
    array16(&mut img, 0x38, &edges);
    img
}

#[test]
fn a_type_2_cloth_reads_its_vertices_and_edges_and_the_scan_s_role_reaches_it() {
    let img = edges_body();
    let signature = Rpf9Signature::of(&img.bytes, img.bytes.len() as u64).expect("a signature");
    let Ok(Resource::Cloth(cloth)) = read(signature, &img.bytes, Some(&[])) else {
        panic!("a cloth record for signature {signature}");
    };
    assert_eq!((cloth.flag, cloth.owner, cloth.record), (0, OWNER, None));
    let ClothKind::Edges { vertices, edges } = cloth.kind else {
        panic!("type 2: {:?}", cloth.kind);
    };
    assert_eq!(vertices, VERTICES);
    assert_eq!(edges.len(), 3);
    assert_eq!(edges[1].indices, [1, 2]);
    assert_eq!(
        (edges[1].length, edges[1].weight, edges[1].word),
        (5f32.sqrt(), 0.25, 9)
    );
}

#[test]
fn a_cloth_head_outside_the_measured_ones_is_refused() {
    let refused = |img: &Image| {
        matches!(
            read_cloth(&img.bytes, Some(&[])),
            Err(HeaderlessError::Cloth(_))
        )
    };
    let mut img = edges_body();
    // An edge to a vertex past the count.
    let edges = u64_at(&img.bytes, 0x38) as usize & 0x0FFF_FFFF;
    img.u16(edges, 5);
    assert!(refused(&img));
    for (at, byte) in [(0x1A, 4u8), (0x1A, 6), (0x18, 3), (0x1C, 0x11)] {
        let mut img = edges_body();
        img.bytes[at] = byte;
        assert!(refused(&img), "byte {at:#X} = {byte}");
    }
    // A flag with no record at 0x10.
    let mut img = edges_body();
    img.bytes[0x18] = 1;
    assert!(refused(&img));
    // A physical section, which only type 3 has.
    let img = edges_body();
    assert!(matches!(
        read_cloth(&img.bytes, Some(&[0; 16])),
        Err(HeaderlessError::Cloth(_))
    ));
}

const WEIGHTS: [[u8; 8]; 4] = [
    [255, 0, 0, 0, 0, 0, 0, 0],
    [200, 55, 0, 0, 0, 0, 0, 0],
    [128, 127, 0, 0, 0, 0, 0, 0],
    [100, 90, 64, 0, 0, 0, 0, 0],
];
/// The last index of each strand has no weight, so it may name no bone.
const BONES: [[u8; 8]; 4] = [
    [1, 0, 0, 0, 0, 0, 0, 9],
    [0, 1, 0, 0, 0, 0, 0, 9],
    [1, 0, 0, 0, 0, 0, 0, 9],
    [0, 1, 1, 0, 0, 0, 0, 9],
];
const REST: [f32; 4] = [0.5, 0.75, 1.0, 1.25];
const ROOT_TRIANGLES: [u32; 4] = [0, 1, 0, 1];
/// Halves exactly, and a finite float as the four bytes together.
const ROOT_UVS: [[f32; 2]; 4] = [[0.0, 0.5], [0.25, 0.5], [1.5, 1.0], [-0.75, 0.125]];
const TRIANGLE_BYTES: [[u8; 4]; 2] = [[33, 3, 3, 2], [74, 7, 255, 2]];
/// Where the strands' arrays lie in the physical section.
const SKIN_BONES: usize = 0x80;
const SKIN_WEIGHTS: usize = 0xA0;
const LIST: usize = 0xC8;
const POINTS: usize = 0xD4;

/// Point `k` of strand `s`, every lane a half exactly; the roots lie in the box.
fn point(s: usize, k: usize) -> [f32; 4] {
    let s = s as f32;
    let k = k as f32;
    [0.25 * s - 0.5, 0.5 * s - 0.75, 0.125 - 0.25 * k, 1.0 + k]
}

/// A type-3 body of four strands of three points and two triangles over their roots, and its
/// physical section.
fn mesh_body() -> (Image, Vec<u8>) {
    let mut img = head(3, 0x140);
    img.vec3(0x30, [-1.0, -1.0, 0.0]);
    img.vec3(0x40, [1.0, 2.0, 0.5]);
    let hashes = img.alloc(8);
    img.u32(hashes, name_hash32(b"SKEL_Spine2"));
    img.u32(hashes + 4, name_hash32(b"SKEL_Head"));
    img.ptr(0xA0, hashes);
    img.u32(0x98, 2);
    img.u32(0x9C, 2);
    let records = img.alloc(12);
    img.ptr(0xB0, records);
    img.u32(0xA8, 1);
    img.u32(0xAC, 1);
    let mut phys = vec![0u8; 0x60];
    let put =
        |phys: &mut Vec<u8>, at: usize, w: u32| phys[at..at + 4].copy_from_slice(&w.to_le_bytes());
    for (slot, at) in [(1, 0x60), (2, 0x70), (4, SKIN_BONES), (5, SKIN_WEIGHTS)] {
        put(&mut phys, 4 * slot, at as u32);
    }
    for (entry, at) in [(0, 0xC0), (4, LIST), (5, POINTS), (6, 0x134), (7, 0x13C)] {
        put(&mut phys, 0x30 + 4 * entry, at as u32);
    }
    phys[0x58..0x5A].copy_from_slice(&4u16.to_le_bytes());
    phys[0x5A..0x5C].copy_from_slice(&2u16.to_le_bytes());
    phys.resize(0xC0, 0);
    for (k, triangle) in ROOT_TRIANGLES.into_iter().enumerate() {
        put(&mut phys, 0x60 + 4 * k, triangle);
        let [u, v] = ROOT_UVS[k].map(crate::half::f32_to_half);
        put(&mut phys, 0x70 + 4 * k, u32::from(v) << 16 | u32::from(u));
    }
    phys[SKIN_BONES..][..32].copy_from_slice(BONES.as_flattened());
    phys[SKIN_WEIGHTS..][..32].copy_from_slice(WEIGHTS.as_flattened());
    for length in REST {
        phys.extend(crate::half::f32_to_half(length).to_le_bytes());
    }
    for i in [0u16, 1, 2, 2, 1, 3] {
        phys.extend(i.to_le_bytes());
    }
    for s in 0..4 {
        for k in 0..3 {
            for lane in point(s, k) {
                phys.extend(crate::half::f32_to_half(lane).to_le_bytes());
            }
        }
    }
    // Index arrays 6 and 7: 4 bytes and 1 byte a triangle, the second padded to 4.
    phys.resize(0x140, 0);
    phys[0x134..0x13C].copy_from_slice(TRIANGLE_BYTES.as_flattened());
    let words = phys.len() / 4;
    phys.resize(phys.len().next_multiple_of(16), 0);
    let buffer = img.alloc(0x50);
    img.u32(buffer + 8, words as u32);
    img.u16(buffer + 0xC, 4);
    img.u16(buffer + 0xE, 8);
    img.u32(buffer + 0x10, 0x1380);
    img.phys(buffer + 0x18, 0);
    img.ptr(0x70, buffer);
    (img, phys)
}

#[test]
fn a_type_3_cloth_reads_its_bones_and_triangle_list_from_its_physical_buffer() {
    let (img, phys) = mesh_body();
    let cloth = read_cloth(&img.bytes, Some(&phys)).expect("the mesh reads");
    let ClothKind::Mesh(mesh) = cloth.kind else {
        panic!("type 3: {:?}", cloth.kind);
    };
    assert_eq!(mesh.triangles, [[0, 1, 2], [2, 1, 3]]);
    assert_eq!(mesh.vertex_count, 4);
    assert_eq!(
        mesh.bone_hashes,
        [name_hash32(b"SKEL_Spine2"), name_hash32(b"SKEL_Head")]
    );
    assert_eq!((mesh.records, mesh.box_max), (1, [1.0, 2.0, 0.5]));
    assert_eq!(mesh.index_arrays[4], LIST as u32);
    assert_eq!(mesh.points_per_strand, 3);
    let points: Vec<[f32; 4]> = (0..4)
        .flat_map(|s| (0..3).map(move |k| point(s, k)))
        .collect();
    assert_eq!(mesh.points, points);
    assert_eq!(
        (mesh.bones, mesh.weights),
        (BONES.to_vec(), WEIGHTS.to_vec())
    );
    assert_eq!(mesh.root_triangles, ROOT_TRIANGLES);
    assert_eq!(mesh.rest_lengths, REST);
    assert_eq!(mesh.root_uvs, ROOT_UVS);
    assert_eq!(mesh.triangle_bytes, TRIANGLE_BYTES);

    let mut more = phys.clone();
    more[0x5A] = 3;
    assert!(matches!(
        read_cloth(&img.bytes, Some(&more)),
        Err(HeaderlessError::Cloth(_))
    ));
    let mut past = phys.clone();
    past[LIST + 10] = 9;
    assert!(matches!(
        read_cloth(&img.bytes, Some(&past)),
        Err(HeaderlessError::Cloth(_))
    ));
    assert!(matches!(
        read_cloth(&img.bytes, None),
        Err(HeaderlessError::PhysicalNotRead { .. })
    ));
}

#[test]
fn a_type_3_cloth_refuses_strands_its_rules_do_not_hold_for() {
    let (img, phys) = mesh_body();
    let half = |x: f32| crate::half::f32_to_half(x).to_le_bytes();
    let mutations: [(&str, usize, Vec<u8>); 7] = [
        ("weights out of order", SKIN_WEIGHTS + 8, vec![55, 200]),
        ("weights summing to 200", SKIN_WEIGHTS, vec![200]),
        (
            "a weighted index past the bones",
            SKIN_BONES + 24 + 2,
            vec![2],
        ),
        (
            "a root triangle past the count",
            0x60 + 12,
            2u32.to_le_bytes().to_vec(),
        ),
        (
            "a root above the box",
            POINTS + 2 * 24 + 4,
            half(0.75).to_vec(),
        ),
        (
            "a point that is not a number",
            POINTS + 8 + 6,
            vec![0x00, 0x7E],
        ),
        ("index array 6 absent", 0x30 + 24, vec![0; 4]),
    ];
    for (what, at, bytes) in mutations {
        let mut body = phys.clone();
        body[at..at + bytes.len()].copy_from_slice(&bytes);
        assert!(
            matches!(
                read_cloth(&img.bytes, Some(&body)),
                Err(HeaderlessError::Cloth(_))
            ),
            "{what}"
        );
    }
}

/// A type-4 body: one box in arrays with room for 16, its record in the slot at 0x80.
fn collider_body() -> Image {
    let mut img = Image::default();
    img.alloc(0xC0);
    let shapes = img.alloc(16 * 64);
    let extents = img.alloc(16 * 16);
    let record = img.alloc(0x70);
    let map = img.alloc(0x10);
    img.bytes[map + 8] = 1;
    img.ptr(0x08, map);
    img.bytes[0x1A] = 4;
    img.u32(0x1C, OWNER);
    img.u32(0x20, u32::MAX);
    img.ptr(0x28, shapes);
    img.u16(0x30, 1);
    img.u16(0x32, 16);
    img.ptr(0x38, extents);
    img.u16(0x40, 1);
    img.u16(0x42, 16);
    img.put(shapes + 0x0C, &0.3f32.to_le_bytes());
    for (k, x) in [0.0f32, 0.6, 0.0, 0.8].iter().enumerate() {
        img.put(shapes + 0x10 + 4 * k, &x.to_le_bytes());
    }
    img.vec3(extents, [0.05, 0.02, 0.3]);
    img.u32(extents + 12, 6);
    img.vec3(0x90, [-0.05, -0.02, -0.3]);
    img.vec3(0xA0, [0.05, 0.02, 0.3]);
    img.u32(0x80, 1);
    img.u32(0x84, 1);
    img.ptr(0x88, record);
    img
}

#[test]
fn a_type_4_cloth_reads_its_box_and_refuses_one_its_rules_do_not_hold_for() {
    let img = collider_body();
    let cloth = read_cloth(&img.bytes, Some(&[])).expect("the box reads");
    let ClothKind::Collider {
        rotation,
        half_extents,
        radius,
    } = cloth.kind
    else {
        panic!("type 4: {:?}", cloth.kind);
    };
    assert_eq!(
        (rotation, half_extents, radius),
        ([0.0, 0.6, 0.0, 0.8], [0.05, 0.02, 0.3], 0.3)
    );
    let shapes = u64_at(&img.bytes, 0x28) as usize & 0x0FFF_FFFF;
    let extents = u64_at(&img.bytes, 0x38) as usize & 0x0FFF_FFFF;
    // The slot at 0x80 copied to the one at 0x70.
    let both = img.bytes[0x80..0x90].to_vec();
    let mutations: [(&str, usize, Vec<u8>); 5] = [
        ("two shapes", 0x30, 2u16.to_le_bytes().to_vec()),
        (
            "a half extent not the box's",
            extents,
            0.06f32.to_le_bytes().to_vec(),
        ),
        (
            "a scaled rotation",
            shapes + 0x1C,
            1.6f32.to_le_bytes().to_vec(),
        ),
        ("the word at 0x20 cleared", 0x20, vec![0; 4]),
        ("a record in both slots", 0x70, both),
    ];
    for (what, at, bytes) in mutations {
        let mut body = collider_body();
        body.put(at, &bytes);
        assert!(
            matches!(
                read_cloth(&body.bytes, Some(&[])),
                Err(HeaderlessError::Cloth(_))
            ),
            "{what}"
        );
    }
}

#[test]
fn a_type_5_cloth_reads_its_shapes_and_the_key_that_lists_them() {
    let mut img = head(5, 0x50);
    let shapes = img.alloc(2 * 0x90);
    for (i, bone) in [0x1111_0001u32, 0x1111_0002].iter().enumerate() {
        img.put(shapes + 0x90 * i + 0x1C, &1.0f32.to_le_bytes());
        img.u32(shapes + 0x90 * i + 0x28, *bone);
        img.vec3(shapes + 0x90 * i + 0x50, [0.1, 0.2 * (i + 1) as f32, 0.3]);
    }
    img.ptr(0x30, shapes);
    img.u32(0x28, 2);
    img.u32(0x2C, 2);
    let key = name_hash32(b"l_foot");
    let list = img.alloc(4);
    img.u16(list + 2, 1);
    let table = img.alloc(0x190);
    img.bytes[table..table + 16].fill(0x80);
    img.bytes[table] = (key & 0x7F) as u8;
    img.u32(table + 0x10, key);
    img.u32(table + 0x18, 2);
    img.u32(table + 0x1C, 2);
    img.ptr(table + 0x20, list);
    img.ptr(0x40, table);
    img.u32(0x38, 1);
    img.u32(0x3C, 1);
    let cloth = read_cloth(&img.bytes, Some(&[])).expect("the shapes read");
    let ClothKind::Shapes { shapes: read, keys } = cloth.kind else {
        panic!("type 5: {:?}", cloth.kind);
    };
    assert_eq!(keys, [(key, vec![0, 1])]);
    assert_eq!(read[1].bone_hash, 0x1111_0002);
    assert_eq!(read[1].half_extents, [0.1, 0.4, 0.3]);
    img.bytes[table] ^= 1;
    assert!(matches!(
        read_cloth(&img.bytes, Some(&[])),
        Err(HeaderlessError::Cloth(_))
    ));
}

#[test]
fn a_type_1_cloth_reads_its_elements_and_refuses_a_record_whose_float_is_not_one() {
    let mut img = head(1, 0x240);
    array16(&mut img, 0x28, &[[0, 0, 0, f(1.0)], [f(1.0), 0, 0, f(0.5)]]);
    array16(&mut img, 0x38, &[[1 << 16, f(1.0), f(0.3), 0x7FFF_0000]]);
    let cloth = read_cloth(&img.bytes, Some(&[])).expect("the elements read");
    let ClothKind::Elements { elements, records } = cloth.kind else {
        panic!("type 1: {:?}", cloth.kind);
    };
    assert_eq!(elements[1], [1.0, 0.0, 0.0, 0.5]);
    assert_eq!((records[0].indices, records[0].weight), ([0, 1], 0.3));
    let at = u64_at(&img.bytes, 0x38) as usize & 0x0FFF_FFFF;
    img.put(at + 4, &0.5f32.to_le_bytes());
    assert!(matches!(
        read_cloth(&img.bytes, Some(&[])),
        Err(HeaderlessError::Cloth(_))
    ));
}
