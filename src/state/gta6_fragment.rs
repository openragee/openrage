//! What the inspector and a body's facts say of a GTA VI fragment's physics level, of the surfaces
//! a drawable's record at 0xD0 holds and of a ped cloth record.

use or_io::gta6::headerless::{Cloth, ClothKind, Fragment, HeaderlessError, Surface};

/// A fragment's groups and children, its bounds listed by bone and the records it only counts.
pub(super) fn fragment_lines(f: &Fragment, lines: &mut Vec<String>) {
    let Some(archetype) = f.archetype else {
        lines.push(
            tr!("It has no physics level: its archetype, groups and children are null.")
                .to_string(),
        );
        return;
    };
    let second = f.children.iter().filter(|c| c.composite.is_none()).count();
    lines.push(tr!(
        "Its physics level: {groups} groups, each on a bone, and {children} children placed at their groups' bones, {second} of them with a bound of their own outside the composite; the archetype's mass is {mass} kg.",
        groups = f.groups.len(),
        children = f.children.len(),
        second = second,
        mass = archetype.mass,
    ));
    if !f.bone_bounds.is_empty() {
        lines.push(tr_n!(
            "{count} bound is listed by bone at 0x38; its frame was not measured, so it is not drawn.",
            "{count} bounds are listed by bone at 0x38; their frame was not measured, so they are not drawn.",
            f.bone_bounds.len()
        ));
    }
    super::gta6_fragment_records::record_lines(f, lines);
    // Of the root's records, only the vehicle record at 0xE8 is read for its counts alone.
    let slots: Vec<String> = (f.records.iter())
        .filter(|(slot, _)| *slot == 0xE8)
        .map(|(slot, _)| format!("{slot:#04X}"))
        .collect();
    if !slots.is_empty() {
        lines.push(tr!(
            "The records the root points at from {slots} are counted and not laid out.",
            slots = slots.join(", "),
        ));
    }
}

/// What a drawable's record at 0xD0 holds, or why it is not read.
pub(super) fn surfaces_line(surfaces: &Result<Vec<Surface>, HeaderlessError>) -> String {
    match surfaces {
        Ok(s) => tr!(
            "The record at 0xD0 holds {surfaces} surfaces of {triangles} triangles in the drawable's frame, drawn over the model: they are no level of detail's geometry.",
            surfaces = s.len(),
            triangles = s.iter().map(|s| s.triangles.len()).sum::<usize>(),
        ),
        Err(e) => tr!("The record at 0xD0 is not read: {why}", why = e),
    }
}

/// A cloth record's head, then its type's counts and tables.
pub(super) fn cloth_lines(c: &Cloth, lines: &mut Vec<String>) {
    let kind = match c.kind {
        ClothKind::Elements { .. } => 1,
        ClothKind::Edges { .. } => 2,
        ClothKind::Mesh(_) => 3,
        ClothKind::Collider { .. } => 4,
        ClothKind::Shapes { .. } => 5,
    };
    lines.push(tr!(
        "A cloth record of type {kind}, flag {flag} and owner word {owner}, whose names are not known.",
        kind = kind,
        flag = c.flag,
        owner = format!("{:#010X}", c.owner),
    ));
    lines.push(match &c.kind {
        ClothKind::Elements { elements, records } => tr!(
            "Its {elements} elements and {records} records of two indices, {pairs} of them naming two elements; what they constrain is not known.",
            elements = elements.len(),
            records = records.len(),
            pairs = records.iter().filter(|r| r.indices[0] != r.indices[1]).count(),
        ),
        ClothKind::Edges { vertices, edges } => {
            let fillers = edges.iter().filter(|e| e.indices[0] == e.indices[1]).count();
            tr!(
                "Its {vertices} simulation vertices and {edges} edges, each with its rest length, and {fillers} filler records.",
                vertices = vertices.len(),
                edges = edges.len() - fillers,
                fillers = fillers,
            )
        }
        ClothKind::Mesh(m) => {
            let set = |t: &[u32]| {
                let set: Vec<String> = (0..t.len()).filter(|k| t[*k] != 0).map(|k| k.to_string()).collect();
                set.join(", ")
            };
            tr!(
                "Its {strands} guide strands of {points} points each, skinned to {bones} bones, and {triangles} triangles over their roots, which lie in the box {min} to {max}; per-strand arrays {arrays} and index arrays {indices} of its physical buffer. They are drawn in the ped's bind pose, which their box and bones suggest and no field states, each root marked as the point its strand is held by.",
                strands = m.vertex_count,
                points = m.points_per_strand,
                triangles = m.triangles.len(),
                bones = m.bone_hashes.len(),
                min = format!("{:?}", m.box_min),
                max = format!("{:?}", m.box_max),
                arrays = set(&m.vertex_arrays),
                indices = set(&m.index_arrays),
            )
        }
        ClothKind::Shapes { shapes, keys } => {
            let keyed: Vec<String> = (keys.iter())
                .map(|(key, list)| format!("hash_{key:08X} {list:?}"))
                .collect();
            tr!(
                "Its {shapes} shapes, under {keys} part-name keys: {list}.",
                shapes = shapes.len(),
                keys = keys.len(),
                list = keyed.join(", "),
            )
        }
        ClothKind::Collider {
            rotation,
            half_extents,
            radius,
        } => tr!(
            "Its one box: half extents {extents}, rotation {rotation} and radius {radius}. The record after its arrays is not laid out, so the box is listed and not drawn.",
            extents = format!("{half_extents:?}"),
            rotation = format!("{rotation:?}"),
            radius = radius,
        ),
    });
    if matches!(c.kind, ClothKind::Edges { .. }) {
        lines.push(
            tr!("Which of its vertices are pinned is not known, so none is marked.").to_string(),
        );
        lines.push(
            tr!("It stores no triangles, and its edges close into a surface in few cloths; as in GTA V, its triangles are taken to be those of the drawable it is worn with, which no field names and the archive's encrypted names hide, so it is drawn as its vertices and edges.").to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::headerless::{ClothEdge, ClothMesh};

    #[test]
    fn a_cloth_s_lines_state_its_type_and_each_type_s_counts_and_tables() {
        let mesh = ClothMesh {
            box_min: [-1.0, -2.0, 0.0],
            box_max: [1.0, 2.0, 0.5],
            bone_hashes: vec![1, 2, 3],
            records: 4,
            vertex_count: 9,
            vertex_arrays: [0, 0x60, 0x70, 0, 0, 0x80, 0, 0, 0, 0, 0],
            index_arrays: [0, 0, 0, 0, 0xA0, 0, 0, 0],
            triangles: vec![[0, 1, 2], [2, 1, 3]],
            points: vec![[0.0; 4]; 9 * 7],
            points_per_strand: 7,
            bones: vec![[0; 8]; 9],
            weights: vec![[255, 0, 0, 0, 0, 0, 0, 0]; 9],
            root_triangles: vec![0; 9],
            rest_lengths: vec![1.0; 9],
            ..ClothMesh::default()
        };
        let cloth = |kind| Cloth {
            flag: 1,
            bytes: [0, 0],
            owner: 0x7D8F_3DB3,
            record: None,
            kind,
        };
        let mut lines = Vec::new();
        cloth_lines(&cloth(ClothKind::Mesh(Box::new(mesh))), &mut lines);
        assert!(
            lines[0].contains("type 3, flag 1 and owner word 0x7D8F3DB3"),
            "{lines:?}"
        );
        assert!(
            lines[1].contains("9 guide strands of 7 points each, skinned to 3 bones, and 2"),
            "{lines:?}"
        );
        assert!(
            lines[1].contains("per-strand arrays 1, 2, 5 and index arrays 4"),
            "{lines:?}"
        );
        let edge = |indices| ClothEdge {
            indices,
            length: 1.0,
            weight: 0.0,
            word: 0,
        };
        let edges = ClothKind::Edges {
            vertices: vec![[0.0; 4]; 5],
            edges: vec![edge([0, 1]), edge([1, 4]), edge([0, 0])],
        };
        let mut lines = Vec::new();
        cloth_lines(&cloth(edges), &mut lines);
        assert!(
            lines[1].contains("5 simulation vertices and 2 edges"),
            "{lines:?}"
        );
        assert!(lines[1].contains("1 filler records"), "{lines:?}");
        assert!(lines[2].contains("pinned is not known"), "{lines:?}");
        assert!(lines[3].contains("stores no triangles"), "{lines:?}");
        let collider = ClothKind::Collider {
            rotation: [0.0, 0.0, 0.0, 1.0],
            half_extents: [0.1, 0.05, 0.2],
            radius: 0.25,
        };
        let mut lines = Vec::new();
        cloth_lines(&cloth(collider), &mut lines);
        assert!(lines[0].contains("type 4"), "{lines:?}");
        assert!(
            lines[1].contains("half extents [0.1, 0.05, 0.2]"),
            "{lines:?}"
        );
    }
}
