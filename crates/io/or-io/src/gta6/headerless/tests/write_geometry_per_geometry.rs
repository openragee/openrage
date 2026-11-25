use super::*;
use crate::container::rpf::{Rpf9Form, Rpf9Role, Rpf9Sections};

const AT: GeometryAt = GeometryAt {
    level: 0,
    model: 0,
    geometry: 0,
};
/// Texcoord 0 of [`TRIANGLE_AT`]: a quarter of the world area, so the area ratio is 4.
const UVS: [[f32; 2]; 3] = [[0.0, 0.0], [0.5, 0.0], [0.0, 0.5]];
/// Row 0 as stored: word 0 within 5 % of 16 times the ratio, the others for channels the geometry
/// does not declare.
const DENSITY: [f32; 4] = [4.0 * 16.0 * 1.02, 7.0, 7.0, 7.0];

fn target(v: &[u8], at: usize) -> usize {
    u64_at(v, at) as usize & 0x0FFF_FFFF
}

fn u16_in(v: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([v[at], v[at + 1]])
}

fn f32_in(v: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(v, at))
}

/// A `(first, end)` range.
fn range(img: &mut Image, first: u16, end: u16) -> usize {
    let at = img.alloc(4);
    img.u16(at, first);
    img.u16(at + 2, end);
    at
}

/// [`drawable`] with texcoord 0 and its per-geometry table at +0xB0, the geometry's run two box
/// elements; where the table lies.
fn tabled() -> (Rpf9Sections, usize) {
    let (mut img, mut phys, parts) = drawable([1.0, 1.0, 0.0]);
    let uv = Stream {
        channel: 28,
        code: 0x10,
        size: 8,
        bytes: UVS.iter().flatten().flat_map(|x| x.to_le_bytes()).collect(),
    };
    put_vertex_block(
        &mut img,
        &mut phys,
        &parts,
        3,
        vec![float_positions(), uv],
        None,
    );
    let densities = img.alloc(16);
    for (k, x) in DENSITY.iter().enumerate() {
        img.put(densities + 4 * k, &x.to_le_bytes());
    }
    let elements = img.alloc(2 * 0x50);
    img.bytes[elements..elements + 2 * 0x50].fill(0xAB);
    let rows = [
        densities,
        range(&mut img, 0, 1),
        range(&mut img, 0, 1),
        range(&mut img, 0, 2),
        elements,
    ];
    let table = img.alloc(0x50);
    for (i, (row, count)) in rows.iter().zip([4u16, 1, 1, 1, 2]).enumerate() {
        img.ptr(table + 16 * i, *row);
        img.u16(table + 16 * i + 8, count);
        img.u16(table + 16 * i + 0x0A, count);
    }
    img.ptr(parts.root + 0xB0, table);
    let map = img.alloc(0x30);
    (img.bytes[map + 8], img.bytes[map + 9]) = (2, 2);
    img.ptr(0x08, map);
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: img.bytes,
        physical: phys,
    };
    (sections, table)
}

fn geometry_in(s: &Rpf9Sections) -> Geometry {
    let mut d = read_drawable(&s.virtual_section, Some(&s.physical)).expect("the drawable reads");
    let mut models = d.levels[0].take().expect("level 0");
    (models.remove(0).geometries.remove(0)).expect("the geometry reads")
}

#[test]
fn a_replaced_mesh_takes_one_box_round_its_vertices_and_the_density_the_rule_gives() {
    let (sections, table) = tabled();
    assert_eq!(geometry_in(&sections).vertices.uvs, [(28, UVS.to_vec())]);
    let positions = vec![
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.5],
    ];
    let edit = GeometryEdit {
        positions: positions.clone(),
        // A quarter of each face's texcoord area again, at four times the world area: ratio 16.
        uvs: vec![(28, vec![[0.0, 0.0], [0.5, 0.0], [0.5, 0.5], [0.0, 0.5]])],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        ..GeometryEdit::default()
    };
    let mut written = sections.clone();
    let w = write_geometry(&mut written, Rpf9Role::Drawable, None, AT, &edit).expect("written");
    assert!(w.table_rederived);
    assert_eq!(geometry_in(&written).vertex_count, 4);

    let v = &written.virtual_section;
    let row = |i: usize| (target(v, table + 16 * i), u16_in(v, table + 16 * i + 8));
    // Word 0: the new ratio times the power of 4 the stored word stood at over the old one.
    let world: f64 = 4.0 + 18f64.sqrt();
    let wanted = (world / 0.5 * 16.0) as f32;
    let words: Vec<f32> = (0..4).map(|k| f32_in(v, row(0).0 + 4 * k)).collect();
    assert_eq!(words, [wanted, 7.0, 7.0, 7.0]);

    // Row 4: the run of two is one element that holds every vertex, and the ranges follow.
    let (row4, count) = row(4);
    assert_eq!((count, u16_in(v, table + 16 * 4 + 0x0A)), (1, 1));
    let (row3, _) = row(3);
    assert_eq!((u16_in(v, row3), u16_in(v, row3 + 2)), (0, 1));
    let lane = |o: usize| std::array::from_fn::<f32, 4, _>(|a| f32_in(v, row4 + o + 4 * a));
    let (min, max) = (lane(0), lane(0x10));
    let axes = [lane(0x20), lane(0x30), lane(0x40)];
    for p in &positions {
        for (a, axis) in axes.iter().enumerate() {
            let along: f32 = (0..3).map(|k| p[k] * axis[k]).sum();
            assert!((min[a]..=max[a]).contains(&along), "{p:?} on axis {a}");
        }
    }
    assert_eq!(&min[..3], &[0.0, 0.0, 0.0]);
    assert_eq!(&max[..3], &[2.0, 2.0, 0.5]);
    assert!(v[row4 + 0x50..row4 + 0xA0].iter().all(|b| *b == 0));
}

#[test]
fn an_unchanged_edit_leaves_the_table_and_a_drawable_without_one_writes_as_before() {
    let (sections, _) = tabled();
    let g = geometry_in(&sections);
    let edit = GeometryEdit {
        positions: g.positions.as_ref().expect("positions").clone(),
        uvs: g.vertices.uvs.clone(),
        triangles: g.triangles.clone(),
        ..GeometryEdit::default()
    };
    let mut written = sections.clone();
    let w = write_geometry(&mut written, Rpf9Role::Drawable, None, AT, &edit).expect("written");
    assert!(!w.table_rederived);
    assert_eq!(written, sections);

    let (img, phys, _) = drawable([1.0, 1.0, 0.0]);
    let mut bare = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: img.bytes,
        physical: phys,
    };
    let mut moved = edit.positions.clone();
    moved[1][0] = 0.5;
    let edit = GeometryEdit {
        positions: moved,
        triangles: g.triangles.clone(),
        ..GeometryEdit::default()
    };
    let w = write_geometry(&mut bare, Rpf9Role::Drawable, None, AT, &edit).expect("written");
    assert!(!w.table_rederived && w.bounds_rewritten);
}

const H: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// [`tabled`] with the geometry's run one element: [`TRIANGLE_AT`]'s box on axes turned 45 degrees
/// about z, axis 1's `w` 0.5; where the element lies.
fn turned_box(s: &mut Rpf9Sections, table: usize) -> usize {
    let v = &mut s.virtual_section;
    let row3 = target(v, table + 0x30);
    v[row3 + 2..row3 + 4].copy_from_slice(&1u16.to_le_bytes());
    for o in [0x48, 0x4A] {
        v[table + o..table + o + 2].copy_from_slice(&1u16.to_le_bytes());
    }
    let row4 = target(v, table + 0x40);
    let lanes = [
        0.0, -H, 0.0, 0.0, H, H, 0.0, 0.0, H, H, 0.0, 0.0, -H, H, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0,
    ];
    for (k, x) in lanes.iter().enumerate() {
        v[row4 + 4 * k..row4 + 4 * k + 4].copy_from_slice(&f32::to_le_bytes(*x));
    }
    row4
}

/// [`TRIANGLE_AT`] with its second vertex moved to x = 2, over the same triangle.
fn moved(s: &Rpf9Sections) -> GeometryEdit {
    let g = geometry_in(s);
    let mut positions = g.positions.as_ref().expect("positions").clone();
    positions[1] = [2.0, 0.0, 0.0];
    GeometryEdit {
        positions,
        triangles: g.triangles.clone(),
        ..GeometryEdit::default()
    }
}

#[test]
fn moved_vertices_re_extend_the_element_on_its_own_axes_and_take_the_density_the_rule_gives() {
    let (mut sections, table) = tabled();
    let row4 = turned_box(&mut sections, table);
    let before = sections.virtual_section.clone();
    let edit = moved(&sections);
    let mut written = sections.clone();
    let w = write_geometry(&mut written, Rpf9Role::Drawable, None, AT, &edit).expect("written");
    assert!(w.table_rederived);
    let v = &written.virtual_section;
    let lanes: Vec<f32> = (0..8).map(|k| f32_in(v, row4 + 4 * k)).collect();
    assert_eq!(lanes, [0.0, -2.0 * H, 0.0, 0.0, 2.0 * H, H, 0.0, 0.0]);
    assert_eq!(
        v[row4 + 0x20..row4 + 0x50],
        before[row4 + 0x20..row4 + 0x50]
    );
    assert_eq!(u16_in(v, table + 0x48), 1);
    // Twice the world area over the same texcoords: ratio 8, at the stored power 16.
    assert_eq!(f32_in(v, target(v, table)), 128.0);
}

#[test]
fn a_density_at_no_power_of_4_times_its_ratio_is_refused_by_name() {
    let (mut sections, table) = tabled();
    turned_box(&mut sections, table);
    let row0 = target(&sections.virtual_section, table);
    // Twice the ratio of 4 as read.
    sections.virtual_section[row0..row0 + 4].copy_from_slice(&8f32.to_le_bytes());
    let edit = moved(&sections);
    let mut written = sections.clone();
    let refused = write_geometry(&mut written, Rpf9Role::Drawable, None, AT, &edit)
        .expect_err("refused")
        .to_string();
    assert!(refused.contains("no power of 4"), "{refused}");
    assert_eq!(written, sections);
}

#[test]
fn each_piece_of_a_cut_takes_the_density_of_its_own_triangles() {
    // A ribbon past one geometry's vertices, its texcoords the positions; the second piece's at half
    // scale, so its area ratio is 4 where the first piece's is about 1.
    let n = 0x1_0000 + 2;
    let ribbon = |half_from: usize| GeometryEdit {
        positions: (0..n)
            .map(|i| [(i / 2) as f32 * 0.01, (i % 2) as f32 * 0.01, 0.0])
            .collect(),
        uvs: vec![(
            28,
            (0..n)
                .map(|i| {
                    let [x, y] = [(i / 2) as f32 * 0.01, (i % 2) as f32 * 0.01];
                    // Half scale from the seam on, joined to the texcoords before it.
                    let seam = (half_from / 2) as f32 * 0.01;
                    match i >= half_from {
                        true => [seam + (x - seam) * 0.5, y * 0.5],
                        false => [x, y],
                    }
                })
                .collect(),
        )],
        triangles: (0..n as u32 - 2).map(|t| [t, t + 1, t + 2]).collect(),
        ..GeometryEdit::default()
    };
    let (sections, table) = tabled();
    let mut first_cut = sections.clone();
    let cut =
        write_geometry_cut(&mut first_cut, Rpf9Role::Drawable, None, AT, &ribbon(n)).expect("cut");
    assert_eq!(cut.pieces, 2);
    let v = &first_cut.virtual_section;
    let mut d = read_drawable(v, Some(&first_cut.physical)).expect("the drawable reads");
    let model = d.levels[0].take().expect("level 0").remove(0);
    let first = model.geometries[0]
        .as_ref()
        .expect("piece 0")
        .triangles
        .len();

    let mut written = sections.clone();
    write_geometry_cut(&mut written, Rpf9Role::Drawable, None, AT, &ribbon(first)).expect("cut");
    let v = &written.virtual_section;
    let row0 = target(v, table);
    let (word0, word1) = (f32_in(v, row0), f32_in(v, row0 + 16));
    // The stored word is 16 times the old ratio, so the power of 4 is 16.
    assert!(
        (word0 / 16.0 - 1.0).abs() < 0.01,
        "piece 0: {word0}, piece 1: {word1}"
    );
    assert!((word1 / 64.0 - 1.0).abs() < 1e-4, "piece 1: {word1}");
}
