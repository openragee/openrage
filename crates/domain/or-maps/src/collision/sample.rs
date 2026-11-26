//! A small bound for the suites that need one, built from triangles: it is a test fixture, and holds no game content.

use or_io::decl::{Open, Shared};
use or_io::gta5::bounds::build::{self, BoundMesh, COMPOSITE_ITEM_THRESHOLD};
use or_io::gta5::bounds::{
    BoundType, LocalBoxMinMax, PhBound, PhBoundAny, PhBoundBVH, PhBoundComposite, PhMaterialMgrId,
    TypeAndIncludeFlags,
};
use or_io::gta5::{Mat34V, Vec3V, Vec4V};

/// A triangle in the space of its file, with the slot of its material.
pub type Triangle = ([[f32; 3]; 3], u8);

fn v4([x, y, z]: [f32; 3], w: f32) -> Vec4V {
    Vec4V {
        x,
        y,
        z,
        w,
        ..Default::default()
    }
}

fn v3([x, y, z]: [f32; 3]) -> Vec3V {
    Vec3V {
        x,
        y,
        z,
        ..Default::default()
    }
}

/// A composite whose one child, a tree bound the composite places at `child_at`, holds `triangles` with the materials `materials`, in the order of its tree and not the order given.
pub fn area_bound(
    triangles: &[Triangle],
    materials: &[u64],
    child_at: [f32; 3],
) -> Result<PhBoundAny, String> {
    let mut mesh = BoundMesh {
        materials: materials
            .iter()
            .map(|id| PhMaterialMgrId {
                id: *id,
                ..Default::default()
            })
            .collect(),
        triangle_materials: triangles.iter().map(|(_, slot)| *slot).collect(),
        ..Default::default()
    };
    for (corners, _) in triangles {
        let first = u16::try_from(mesh.positions.len())
            .map_err(|_| "more corners than a polygon can name".to_string())?;
        mesh.positions.extend(
            corners
                .iter()
                .map(|p| std::array::from_fn(|a| p[a] - child_at[a])),
        );
        mesh.triangles.push([first, first + 1, first + 2]);
    }
    let mut child = PhBoundBVH {
        base: build::geometry(&mesh).map_err(|e| e.to_string())?,
        ..Default::default()
    };
    child.base.base.r#type = Open::new(BoundType::Bvh);
    build::rebuild(&mut child).map_err(|e| e.to_string())?;

    let max = &child.base.base.bounding_box_max_xyz_margin_w;
    let held = (child.base.base.bounding_box_min(), [max.x, max.y, max.z]);
    let mut header = PhBound {
        r#type: Open::new(BoundType::Composite),
        radius_around_centroid: 100.0,
        bounding_box_max_xyz_margin_w: v4([100.0; 3], 0.0),
        ..Default::default()
    };
    header.set_bounding_box_min([-100.0; 3]);
    Ok(PhBoundAny::PhBoundComposite(PhBoundComposite {
        base: header,
        current_matrices: vec![Mat34V {
            col0: v4([1.0, 0.0, 0.0], 0.0),
            col1: v4([0.0, 1.0, 0.0], 0.0),
            col2: v4([0.0, 0.0, 1.0], 0.0),
            col3: v4(child_at, 0.0),
            ..Default::default()
        }],
        last_matrices: Shared::Same,
        local_box_min_maxs: vec![LocalBoxMinMax {
            min: v3(held.0),
            max: v3(held.1),
            ..Default::default()
        }],
        type_and_include_flags: vec![TypeAndIncludeFlags {
            type_flags: 0x40,
            include_flags: 0x00ff_0000,
            ..Default::default()
        }],
        owned_type_and_include_flags: Shared::Same,
        bvh_structure: Some(Box::new(
            build::build(&[held], COMPOSITE_ITEM_THRESHOLD)
                .map_err(|e| e.to_string())?
                .bvh,
        )),
        bounds: vec![Some(Box::new(PhBoundAny::PhBoundBVH(child)))],
        ..Default::default()
    }))
}
