use super::*;
use crate::container::rpf::{Rpf9Form, Rpf9Role, Rpf9Sections};

const AT: GeometryAt = GeometryAt {
    level: 0,
    model: 0,
    geometry: 0,
};
/// The most vertices a geometry holds.
const LIMIT: usize = 0x1_0000;
/// The built geometry's four density words in the table's row 0.
const DENSITY: [f32; 4] = [1.5, 0.25, 0.0, 0.0];

fn target(v: &[u8], at: usize) -> usize {
    u64_at(v, at) as usize & 0x0FFF_FFFF
}

fn u16_in(v: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([v[at], v[at + 1]])
}

fn f32_in(v: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(v, at))
}

/// A `(0, 1)` range: one entry over one.
fn one_range(img: &mut Image) -> usize {
    let at = img.alloc(4);
    img.u16(at + 2, 1);
    at
}

fn sections_of(img: Image, phys: Vec<u8>) -> Rpf9Sections {
    Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: img.bytes,
        physical: phys,
    }
}

/// The block map last, with room for four blocks.
fn map_last(img: &mut Image) {
    let map = img.alloc(0x30);
    (img.bytes[map + 8], img.bytes[map + 9]) = (2, 2);
    img.ptr(0x08, map);
}

/// [`drawable`] with its per-geometry table at +0xB0, one geometry with one box element; where
/// the table lies.
fn tabled() -> (Rpf9Sections, usize) {
    let (mut img, phys, parts) = drawable([1.0, 1.0, 0.0]);
    let densities = img.alloc(16);
    for (k, x) in DENSITY.iter().enumerate() {
        img.put(densities + 4 * k, &x.to_le_bytes());
    }
    let rows = [
        densities,
        one_range(&mut img),
        one_range(&mut img),
        one_range(&mut img),
        img.alloc(0x50),
    ];
    let counts = [4u16, 1, 1, 1, 1];
    let table = img.alloc(0x50);
    for (i, (row, count)) in rows.iter().zip(counts).enumerate() {
        img.ptr(table + 16 * i, *row);
        img.u16(table + 16 * i + 8, count);
        img.u16(table + 16 * i + 0x0A, count);
    }
    img.ptr(parts.root + 0xB0, table);
    map_last(&mut img);
    (sections_of(img, phys), table)
}

/// A ribbon of `triangles` triangles, each the next vertex with the two before it.
fn ribbon(triangles: u32) -> GeometryEdit {
    let n = triangles + 2;
    GeometryEdit {
        positions: (0..n)
            .map(|i| [(i / 2) as f32 * 0.01, (i % 2) as f32 * 0.01, 0.0])
            .collect(),
        triangles: (0..triangles).map(|t| [t, t + 1, t + 2]).collect(),
        ..GeometryEdit::default()
    }
}

fn box_of(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let pick = |a: usize, f: fn(f32, f32) -> f32| {
        (positions.iter().map(|p| p[a]))
            .reduce(f)
            .expect("positions")
    };
    (
        std::array::from_fn(|a| pick(a, f32::min)),
        std::array::from_fn(|a| pick(a, f32::max)),
    )
}

#[test]
fn a_mesh_past_65_536_vertices_becomes_geometries_of_one_shader_cut_by_triangle() {
    let (sections, table) = tabled();
    // A ribbon's first t triangles use t + 2 vertices. The piece stops where its meshlets' vertex
    // lists would pass the 16-bit offset, which a ribbon reaches before its vertex count, and the
    // next piece starts with the two seam vertices it shares.
    let edit = ribbon(LIMIT as u32);
    let mut written = sections.clone();
    let cut = write_geometry_cut(&mut written, Rpf9Role::Drawable, None, AT, &edit).expect("cut");
    assert_eq!(cut.pieces, 2);
    assert!(cut.written.triangles_rebuilt && cut.written.root_reboxed);

    let v = &written.virtual_section;
    let mut d = read_drawable(v, Some(&written.physical)).expect("the drawable reads");
    let model = d.levels[0].take().expect("level 0").remove(0);
    let g: Vec<Geometry> = (model.geometries.into_iter())
        .map(|g| g.expect("each piece reads"))
        .collect();
    assert_eq!(g.len(), 2);
    // The split lies where the meshlet list bound falls, below the vertex count; a ribbon piece of
    // t triangles holds t + 2 vertices, and the two pieces partition the triangles.
    let first = g[0].triangles.len();
    let first_vertices = first + 2;
    assert!(
        first_vertices < LIMIT,
        "the meshlet list bound splits before the vertex count"
    );
    assert_eq!(g[0].vertex_count as usize, first_vertices);
    let second = &edit.positions[first..];
    assert_eq!(g[1].vertex_count as usize, second.len());
    assert_eq!(g[1].triangles.len(), edit.triangles.len() - first);
    assert_eq!(
        g[0].positions.as_ref().ok(),
        Some(&edit.positions[..first_vertices].to_vec())
    );
    assert_eq!(g[1].positions.as_ref().ok(), Some(&second.to_vec()));
    assert_eq!((g[0].shader, g[1].shader), (0, 0));

    // The model's counts and arrays: the union box first, each box's w its x.
    let model = target(v, target(v, target(v, 0x50)));
    for o in [0x10, 0x12, 0x36] {
        assert_eq!(u16_in(v, model + o), 2, "model +{o:#X}");
    }
    let boxes = target(v, model + 0x18);
    let corners = [
        box_of(&edit.positions),
        box_of(&edit.positions[..first_vertices]),
        box_of(second),
    ];
    for (k, (min, max)) in corners.iter().enumerate() {
        let at = boxes + 32 * k;
        let stored = |o: usize| std::array::from_fn::<f32, 4, _>(|a| f32_in(v, at + o + 4 * a));
        assert_eq!(stored(0), [min[0], min[1], min[2], min[0]], "box {k}");
        assert_eq!(stored(0x10), [max[0], max[1], max[2], max[0]], "box {k}");
    }
    let geometries = target(v, model + 0x08);
    let (g0, g1) = (target(v, geometries), target(v, geometries + 8));
    assert_ne!(u64_at(v, g0 + 0x30), u64_at(v, g1 + 0x30), "a fresh digest");
    assert_eq!(
        u64_at(v, g0 + 0x28),
        u64_at(v, g1 + 0x28),
        "the palette words copied"
    );
    let vb = target(v, g1 + 0x08);
    let record = target(v, g1 + 0x40);
    let ib = target(v, record);
    assert_eq!(
        target(v, vb + 0x28),
        record + 0x20,
        "the vertex buffer's own view"
    );
    assert_eq!(
        target(v, ib + 0x28),
        ib + 0x40,
        "the index buffer's own view"
    );
    for view in [record + 0x20, ib + 0x40] {
        assert_eq!(u64_at(v, view + 0x10), 0xFFFF_FFFF_FFFF_0214);
    }

    // The table: row 0 the density words carried, row 3 a range per geometry over row 4, row 4
    // one box per piece, row 2 the model's range over both geometries.
    let row = |i: usize| (target(v, table + 16 * i), u16_in(v, table + 16 * i + 8));
    let (row0, row0_count) = row(0);
    assert_eq!(row0_count, 8);
    for k in 0..8 {
        assert_eq!(f32_in(v, row0 + 4 * k), DENSITY[k % 4], "row 0 word {k}");
    }
    let (row3, row3_count) = row(3);
    assert_eq!(row3_count, 2);
    let pairs: Vec<u16> = (0..4).map(|k| u16_in(v, row3 + 2 * k)).collect();
    assert_eq!(pairs, [0, 1, 1, 2]);
    let (row4, row4_count) = row(4);
    assert_eq!(row4_count, 2);
    for (k, (min, max)) in corners[1..].iter().enumerate() {
        let at = row4 + 0x50 * k;
        let lane = |o: usize| std::array::from_fn::<f32, 4, _>(|a| f32_in(v, at + o + 4 * a));
        assert_eq!(lane(0), [min[0], min[1], min[2], 0.0], "element {k}");
        assert_eq!(lane(0x10), [max[0], max[1], max[2], 0.0], "element {k}");
        assert_eq!(lane(0x20), [1.0, 0.0, 0.0, 0.0]);
    }
    let (row2, row2_count) = row(2);
    assert_eq!(row2_count, 1);
    assert_eq!((u16_in(v, row2), u16_in(v, row2 + 2)), (0, 2));
    let (row1, _) = row(1);
    assert_eq!(
        (u16_in(v, row1), u16_in(v, row1 + 2)),
        (0, 1),
        "row 1 unchanged"
    );

    // A mesh that fits one geometry is written as write_geometry writes it.
    let small = ribbon(4);
    let (mut one, mut alone) = (sections.clone(), sections.clone());
    let cut = write_geometry_cut(&mut one, Rpf9Role::Drawable, None, AT, &small).expect("written");
    let plain = write_geometry(&mut alone, Rpf9Role::Drawable, None, AT, &small).expect("written");
    assert_eq!((cut.pieces, cut.written), (1, plain));
    assert_eq!(one, alone);
}

#[test]
fn a_cut_is_refused_by_name_without_the_per_geometry_table_or_past_255_palette_entries() {
    let edit = ribbon(LIMIT as u32);
    let (mut img, phys, _) = drawable([1.0, 1.0, 0.0]);
    map_last(&mut img);
    let bare = sections_of(img, phys);
    let mut again = bare.clone();
    let refused = write_geometry_cut(&mut again, Rpf9Role::Drawable, None, AT, &edit)
        .expect_err("refused")
        .to_string();
    assert!(refused.contains("+0xB0"), "{refused}");
    assert_eq!(again, bare);

    let (mut sections, _) = tabled();
    let v = &mut sections.virtual_section;
    let model = target(v, target(v, target(v, 0x50)));
    let geometry = target(v, target(v, model + 0x08));
    v[geometry + 0x2C..geometry + 0x30].copy_from_slice(&256u32.to_le_bytes());
    let mut again = sections.clone();
    let refused = write_geometry_cut(&mut again, Rpf9Role::Drawable, None, AT, &edit)
        .expect_err("refused")
        .to_string();
    assert!(refused.contains("palette"), "{refused}");
    assert_eq!(again, sections);
}

#[test]
fn a_palette_is_the_u16_at_0x2c_so_a_non_zero_word_after_it_still_cuts() {
    let edit = ribbon(LIMIT as u32);
    let (mut sections, _) = tabled();
    let v = &mut sections.virtual_section;
    let model = target(v, target(v, target(v, 0x50)));
    let geometry = target(v, target(v, model + 0x08));
    v[geometry + 0x2C..geometry + 0x2E].copy_from_slice(&10u16.to_le_bytes());
    v[geometry + 0x2E..geometry + 0x30].copy_from_slice(&0x8001u16.to_le_bytes());
    assert!(
        u32_at(v, geometry + 0x2C) > 0xFF,
        "the dword passes the ceiling"
    );
    let mut written = sections.clone();
    let cut = write_geometry_cut(&mut written, Rpf9Role::Drawable, None, AT, &edit).expect("cut");
    assert_eq!(cut.pieces, 2);

    let v = &mut sections.virtual_section;
    v[geometry + 0x2C..geometry + 0x2E].copy_from_slice(&256u16.to_le_bytes());
    let mut again = sections.clone();
    let refused = write_geometry_cut(&mut again, Rpf9Role::Drawable, None, AT, &edit)
        .expect_err("refused")
        .to_string();
    assert!(
        refused.contains("one byte at +0x30") && refused.contains("209"),
        "{refused}"
    );
    assert_eq!(again, sections);
}

#[test]
fn channel_33_s_four_byte_form_is_texcoord_5_and_takes_a_new_vertex_count() {
    let lanes = |uv: [f32; 2]| -> [u8; 4] {
        let q = |x: f32| ((x * 8192.0).round() as i16).to_le_bytes();
        let (u, v) = (q(uv[0]), q(uv[1]));
        [u[0], u[1], v[0], v[1]]
    };
    let held = [[0.25, 1.0], [0.0157, 0.0], [0.5, 3.5]];
    let decoded: Vec<[f32; 2]> = (held.iter())
        .map(|uv| {
            let e = lanes(*uv);
            let s16 = |at: usize| f32::from(i16::from_le_bytes([e[at], e[at + 1]])) / 8192.0;
            [s16(0), s16(2)]
        })
        .collect();
    let (mut img, mut phys, parts) = drawable([1.0, 1.0, 0.0]);
    map_last(&mut img);
    let texcoord5 = Stream {
        channel: 33,
        code: 0x26,
        size: 4,
        bytes: held.iter().flat_map(|uv| lanes(*uv)).collect(),
    };
    put_vertex_block(
        &mut img,
        &mut phys,
        &parts,
        3,
        vec![float_positions(), texcoord5],
        None,
    );
    let sections = sections_of(img, phys);
    let read = |s: &Rpf9Sections| -> Geometry {
        let mut d = read_drawable(&s.virtual_section, Some(&s.physical)).expect("it reads");
        let mut models = d.levels[0].take().expect("level 0");
        (models.remove(0).geometries.remove(0)).expect("the geometry reads")
    };
    let g = read(&sections);
    assert_eq!(g.vertices.uvs, [(33, decoded.clone())]);
    assert!(!g.vertices.skipped.iter().any(|(c, _)| *c == 33));

    let mut edit = GeometryEdit {
        positions: g.positions.as_ref().expect("positions").clone(),
        uvs: g.vertices.uvs.clone(),
        triangles: g.triangles.clone(),
        ..GeometryEdit::default()
    };
    edit.positions.push([1.0, 1.0, 0.0]);
    edit.uvs[0].1.push([0.75, -2.0]);
    edit.triangles.push([1, 3, 2]);
    let mut written = sections.clone();
    write_geometry(&mut written, Rpf9Role::Drawable, None, AT, &edit).expect("written");
    let back = read(&written);
    assert_eq!(back.vertex_count, 4);
    let mut wanted = decoded;
    wanted.push([0.75, -2.0]);
    assert_eq!(back.vertices.uvs, [(33, wanted)]);

    // A lane of 4.0 passes the 16-bit form's 4 - 2^-13 and is refused, not clamped.
    edit.uvs[0].1[3] = [0.0, 4.0];
    let mut again = sections.clone();
    let refused = write_geometry(&mut again, Rpf9Role::Drawable, None, AT, &edit);
    assert!(
        matches!(
            refused,
            Err(GeometryWriteError::Channel { channel: 33, .. })
        ),
        "{refused:?}"
    );
    assert_eq!(again, sections);
}
