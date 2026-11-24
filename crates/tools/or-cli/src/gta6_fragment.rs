//! `rpf scan-open` on a GTA VI fragment's physics level, a drawable's surfaces at 0xD0 and a ped
//! cloth record: their counts, tables and ranges, with `hash_XXXXXXXX` for a name only its hash gives.

use or_io::gta6::headerless::{
    ChildBound, Cloth, ClothKind, Fragment, HeaderlessError, Skeleton, Surface,
};

fn bone_name(skeleton: Option<&Skeleton>, bone: u16, hash: u32) -> String {
    let name = skeleton.and_then(|s| s.bones.get(usize::from(bone)));
    name.map_or_else(|| format!("hash_{hash:08X}"), |b| b.name.clone())
}

fn kind_of(bound: &ChildBound) -> &'static str {
    match bound {
        ChildBound::Geometry(_) => "geometry",
        ChildBound::Sphere { .. } => "sphere",
        ChildBound::Capsule { .. } => "capsule",
        ChildBound::Box { .. } => "box",
        ChildBound::TaperedCapsule { .. } => "tapered capsule",
        ChildBound::Disc { .. } => "disc",
        ChildBound::Cylinder { .. } => "cylinder",
        ChildBound::Hull { .. } => "hull",
        ChildBound::Frustum { .. } => "frustum",
    }
}

pub(crate) fn print_fragment(f: &Fragment, skeleton: Option<&Skeleton>) {
    let matrices = f.matrices.map_or("no matrix set".to_string(), |m| {
        format!("matrix set of {m}")
    });
    println!(
        "  fragment      counts (sets, root groups, groups, children) [{}, {}, {}, {}]; {matrices}; {} glass panes",
        f.sets,
        f.groups.iter().filter(|g| g.parent.is_none()).count(),
        f.groups.len(),
        f.children.len(),
        f.glass_panes
    );
    for (slot, at) in &f.records {
        println!("  record        {slot:#04X} at {at:#X}, counted and not laid out");
    }
    let Some(a) = f.archetype else {
        return;
    };
    println!(
        "  archetype     mass {}, angular inertia {:?}, type flags {:#X}",
        a.mass, a.angular_inertia, a.type_flags
    );
    for (i, g) in f.groups.iter().enumerate() {
        let parent = g.parent.map_or("none".to_string(), |p| p.to_string());
        println!(
            "    group {i:<4} bone {} {} (hash_{:08X}), parent {parent}, child groups {:?}, children {:?}, second set {:?}",
            g.bone,
            bone_name(skeleton, g.bone, g.bone_hash),
            g.bone_hash,
            g.child_groups,
            g.children,
            g.second_set
        );
    }
    let listed = f.children.iter().filter(|c| c.composite.is_some()).count();
    let mass: f32 = (f.children.iter())
        .filter(|c| c.composite.is_some())
        .map(|c| c.mass)
        .sum();
    println!(
        "  children      per set [{listed}, {}], the composite's of mass {mass}; each placed by its bone's bind world, as its composite matrix",
        f.children.len() - listed
    );
    for (i, c) in f.children.iter().enumerate() {
        let Some(bound) = &c.bound else { continue };
        let bound = match bound {
            Ok(b) => kind_of(b).to_string(),
            Err(e) => format!("not read: {e}"),
        };
        println!(
            "    child {i:<4} second set, group {}, mass {}, at {:?}, bound {bound}",
            c.group,
            c.mass,
            &c.matrix[3][..3]
        );
    }
    if !f.bone_bounds.is_empty() {
        let read = f.bone_bounds.iter().filter(|b| b.bound.is_ok()).count();
        println!(
            "  bone bounds   {}, {read} read; their frame was not measured",
            f.bone_bounds.len()
        );
    }
}

pub(crate) fn print_surfaces(indent: &str, surfaces: &Result<Vec<Surface>, HeaderlessError>) {
    match surfaces {
        Ok(s) => {
            let codes: Vec<String> = s
                .iter()
                .take(8)
                .map(|s| format!("{:08X}", s.code))
                .collect();
            println!(
                "{indent}    surfaces  at 0xD0, {} entries (codes {}{}), {} triangles; neither a level nor its geometry",
                s.len(),
                codes.join(" "),
                if s.len() > 8 { " ..." } else { "" },
                s.iter().map(|s| s.triangles.len()).sum::<usize>()
            );
        }
        Err(e) => println!("{indent}    surfaces  at 0xD0, not read: {e}"),
    }
}

pub(crate) fn print_cloth(c: &Cloth) {
    let record = c
        .record
        .map_or("none".to_string(), |r| format!("at {r:#X}"));
    println!(
        "  cloth         flag {}, owner {:#010X}, bytes 0x19 and 0x1B {:#04X} {:#04X}; record at 0x10 {record}",
        c.flag, c.owner, c.bytes[0], c.bytes[1]
    );
    match &c.kind {
        ClothKind::Elements { elements, records } => println!(
            "  type 1        {} elements, {} records, {} of two distinct indices",
            elements.len(),
            records.len(),
            records
                .iter()
                .filter(|r| r.indices[0] != r.indices[1])
                .count()
        ),
        ClothKind::Edges { vertices, edges } => {
            let real: Vec<_> = edges
                .iter()
                .filter(|e| e.indices[0] != e.indices[1])
                .collect();
            let at_rest = (real.iter())
                .filter(|e| {
                    let [i, j] = e.indices.map(usize::from);
                    let d: f32 = (0..3)
                        .map(|a| (vertices[i][a] - vertices[j][a]).powi(2))
                        .sum();
                    (e.length - d.sqrt()).abs() <= 0.05 * e.length.max(1e-6)
                })
                .count();
            println!(
                "  type 2        {} vertices, {} edges and {} fillers; {at_rest} edges within 5 % of their vertices' distance",
                vertices.len(),
                real.len(),
                edges.len() - real.len()
            );
        }
        ClothKind::Mesh(m) => {
            let set = |t: &[u32]| -> Vec<usize> { (0..t.len()).filter(|k| t[*k] != 0).collect() };
            println!(
                "  type 3        root box {:?} to {:?}; {} bone hashes; {} 12-byte records; {} strands of {} points; per-strand arrays {:?}, index arrays {:?}; {} triangles over the roots",
                m.box_min,
                m.box_max,
                m.bone_hashes.len(),
                m.records,
                m.vertex_count,
                m.points_per_strand,
                set(&m.vertex_arrays),
                set(&m.index_arrays),
                m.triangles.len()
            );
            println!(
                "  not read      the points' w, per-strand arrays 0, 3 and 6 to 10, index arrays 1, 2, 3 and 7, what index array 6's four bytes a triangle count, the box at 0x50"
            );
        }
        ClothKind::Collider {
            rotation,
            half_extents,
            radius,
        } => println!(
            "  type 4        one box of half extents {half_extents:?}, rotation {rotation:?}, radius {radius}; its record is not laid out"
        ),
        ClothKind::Shapes { shapes, keys } => {
            let named: Vec<String> = (keys.iter().take(8))
                .map(|(k, list)| format!("hash_{k:08X} {list:?}"))
                .collect();
            println!(
                "  type 5        {} shapes, {} keys: {}{}",
                shapes.len(),
                keys.len(),
                named.join(", "),
                if keys.len() > 8 { ", ..." } else { "" }
            );
        }
    }
}
