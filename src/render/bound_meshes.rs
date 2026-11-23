use std::f32::consts::{PI, TAU};

use glam::{Mat3, Mat4, Vec3};
use or_io::gta5::data::materials_palette::{MaterialPalette, MATERIAL_PALETTE_DEFAULT_RGB};
use or_io::gta5::{Mat34V, PhBound, PhBoundAny, PhBoundComposite, PhBoundGeometry};
use or_renderer::MeshVertex;

use crate::render::meshes::MeshUpload;

const SEGMENTS: usize = 32;

const RINGS: usize = 16;

const DEFAULT_BOUND_TINT: [f32; 4] = palette_rgb_to_rgba(MATERIAL_PALETTE_DEFAULT_RGB);

const fn palette_rgb_to_rgba(rgb: [u8; 3]) -> [f32; 4] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        1.0,
    ]
}

/// The header every bound starts with.
pub(crate) fn header(bound: &PhBoundAny) -> &PhBound {
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

/// The bounding box's corners.
pub(crate) fn header_aabb(header: &PhBound) -> (Vec3, Vec3) {
    let max = &header.bounding_box_max_xyz_margin_w;
    (
        Vec3::from(header.bounding_box_min()),
        Vec3::new(max.x, max.y, max.z),
    )
}

pub(crate) fn header_centroid(header: &PhBound) -> Vec3 {
    let c = &header.centroid_offset_xyz_material_id0_w;
    Vec3::new(c.x, c.y, c.z)
}

/// The margin, in the `w` of the bounding box's maximum.
pub(crate) fn header_margin(header: &PhBound) -> f32 {
    header.bounding_box_max_xyz_margin_w.w
}

/// A capsule's radius and half its length, both read off the header.
pub(crate) fn capsule_extent(header: &PhBound) -> (f32, f32) {
    let radius = header_margin(header);
    (radius, header.radius_around_centroid - radius)
}

/// A cylinder's radius and half its height, from its box.
pub(crate) fn cylinder_extent(header: &PhBound) -> (f32, f32) {
    let (mn, mx) = header_aabb(header);
    ((mx.x - mn.x).max(mx.z - mn.z) * 0.5, (mx.y - mn.y) * 0.5)
}

/// Half a disc's thickness: its margin, or half its box along X.
pub(crate) fn disc_margin(header: &PhBound) -> f32 {
    let margin = header_margin(header);
    if margin > 0.0 {
        return margin;
    }
    let (mn, mx) = header_aabb(header);
    (mx.x - mn.x) * 0.5
}

pub(crate) fn mat34_to_mat4(m: &Mat34V) -> Mat4 {
    Mat4::from_cols(
        glam::Vec4::new(m.col0.x, m.col0.y, m.col0.z, 0.0),
        glam::Vec4::new(m.col1.x, m.col1.y, m.col1.z, 0.0),
        glam::Vec4::new(m.col2.x, m.col2.y, m.col2.z, 0.0),
        glam::Vec4::new(m.col3.x, m.col3.y, m.col3.z, 1.0),
    )
}

/// A geometry's vertices, placed by `transform`.
pub(crate) fn geometry_positions(geom: &PhBoundGeometry, transform: Mat4) -> Vec<Vec3> {
    (0..geom.compressed_vertices.len())
        .filter_map(|i| geom.vertex_position(i))
        .map(|p| transform.transform_point3(Vec3::from(p)))
        .collect()
}

pub fn build_bound_meshes(bound: &PhBoundAny, palette: &MaterialPalette) -> Vec<MeshUpload> {
    let mut builder = BoundMeshBuilder::new(palette);
    walk_bound(bound, Mat4::IDENTITY, &mut builder);
    builder.out
}

/// The meshes each leaf of a bound draws, by the number its row has in the Outline, which counts the
/// nodes the bound holds in order, a composite before its children, as `bound_node_lines` does.
pub(crate) fn bound_node_meshes(
    bound: &PhBoundAny,
    palette: &MaterialPalette,
) -> Vec<(u64, Vec<MeshUpload>)> {
    fn walk(
        bound: &PhBoundAny,
        transform: Mat4,
        palette: &MaterialPalette,
        next: &mut u64,
        out: &mut Vec<(u64, Vec<MeshUpload>)>,
    ) {
        let node = *next;
        *next += 1;
        if let PhBoundAny::PhBoundComposite(c) = bound {
            for (child, local) in crate::render::lines::composite_children(c) {
                walk(child, transform * local, palette, next, out);
            }
            return;
        }
        let mut builder = BoundMeshBuilder::new(palette);
        walk_bound(bound, transform, &mut builder);
        out.push((node, builder.out));
    }
    let mut out = Vec::new();
    walk(bound, Mat4::IDENTITY, palette, &mut 0, &mut out);
    out
}

struct BoundMeshBuilder<'a> {
    out: Vec<MeshUpload>,
    next_bias_index: usize,
    palette: &'a MaterialPalette,
}

impl<'a> BoundMeshBuilder<'a> {
    fn new(palette: &'a MaterialPalette) -> Self {
        Self {
            out: Vec::new(),
            next_bias_index: 0,
            palette,
        }
    }

    fn next_bias(&mut self) -> f32 {
        let idx = self.next_bias_index;
        self.next_bias_index += 1;
        depth_bias_for_index(idx)
    }

    fn push_uniform_bias(&mut self, mut vertices: Vec<MeshVertex>, indices: Vec<u32>) {
        if vertices.is_empty() || indices.len() < 3 {
            return;
        }
        let bias = self.next_bias();
        if bias != 0.0 {
            for v in &mut vertices {
                v.depth_bias = bias;
            }
        }
        self.out.push(MeshUpload {
            bump: false,
            channels: Vec::new(),
            vertices,
            indices,
            texture_name: None,
            skinning: None,
            palette_bones: Vec::new(),
            source: None,
        });
    }
}

/// A composite held by value, as a cloth holds its collision.
pub fn build_composite_meshes(
    composite: &PhBoundComposite,
    palette: &MaterialPalette,
) -> Vec<MeshUpload> {
    let mut builder = BoundMeshBuilder::new(palette);
    walk_composite(composite, Mat4::IDENTITY, &mut builder);
    builder.out
}

fn walk_composite(c: &PhBoundComposite, transform: Mat4, builder: &mut BoundMeshBuilder) {
    for (i, child) in c.bounds.iter().enumerate() {
        let Some(child) = child else {
            continue;
        };
        let local = c
            .current_matrices
            .get(i)
            .map(mat34_to_mat4)
            .unwrap_or(Mat4::IDENTITY);
        walk_bound(child, transform * local, builder);
    }
}

fn walk_bound(bound: &PhBoundAny, transform: Mat4, builder: &mut BoundMeshBuilder) {
    match bound {
        PhBoundAny::PhBoundComposite(c) => walk_composite(c, transform, builder),
        PhBoundAny::PhBoundGeometry(g) => {
            push_polyhedron_upload(g, transform, builder);
        }
        PhBoundAny::PhBoundBVH(b) => {
            push_polyhedron_upload(&b.base, transform, builder);
        }
        PhBoundAny::PhBoundSphere(b) => {
            push_sphere_upload(&b.base, transform, builder);
        }
        PhBoundAny::PhBoundBox(b) => {
            push_box_upload(&b.base, transform, builder);
        }
        // A plane is drawn as its box.
        PhBoundAny::PhBoundPlane(b) => {
            push_box_upload(&b.base, transform, builder);
        }
        PhBoundAny::PhBoundCapsule(b) => {
            push_capsule_upload(&b.base, transform, builder);
        }
        PhBoundAny::PhBoundCylinder(b) => {
            push_cylinder_upload(&b.base, transform, builder);
        }
        PhBoundAny::PhBoundDisc(b) => {
            push_disc_upload(&b.base, transform, builder);
        }
        PhBoundAny::Base(_) => {}
    }
}

fn aabb_centre(mn: Vec3, mx: Vec3) -> Vec3 {
    (mn + mx) * 0.5
}

/// Maximum per-upload anti-Z-fight bias, as a **unit-interval fraction** of
/// the mesh vertex shader's world-space pull toward the camera, which its
/// `BIAS_*` constants bound above by 5 cm.
///
/// A bias in clip-space NDC units was wrong: its world-space pull scales as
/// `z_view²`, so `2e-4` NDC is ~10 m of pull at 50 m and ~160 m at 200 m,
/// enough to pull interior bounds in front of an outer hull and hide it.
///
/// ⚠ **Always non-negative.** A negative bias pushes a vertex behind its
/// surface and can land it past the far clip plane, so the mesh blinks in and
/// out as the camera moves.
const MAX_DEPTH_BIAS: f32 = 1.0;

const GOLDEN_RATIO_FRACT: f64 = 0.6180339887498949;

fn depth_bias_for_index(index: usize) -> f32 {
    if index == 0 {
        return 0.0;
    }
    let unit = ((index as f64) * GOLDEN_RATIO_FRACT).fract() as f32;
    unit * MAX_DEPTH_BIAS
}

fn apply_transform(transform: Mat4, position: Vec3, normal: Vec3) -> ([f32; 3], [f32; 3]) {
    let world_pos = transform.transform_point3(position);
    let normal_mat = Mat3::from_mat4(transform);
    let world_normal = normal_mat.mul_vec3(normal).normalize_or_zero();
    (
        [world_pos.x, world_pos.y, world_pos.z],
        if world_normal == Vec3::ZERO {
            [0.0, 1.0, 0.0]
        } else {
            [world_normal.x, world_normal.y, world_normal.z]
        },
    )
}

/// ⚠ A primitive's material id is the bits of its centroid's `w`; the material type is its low byte.
fn primitive_tint(header: &PhBound, palette: &MaterialPalette) -> [f32; 4] {
    let id = header.centroid_offset_xyz_material_id0_w.w.to_bits();
    material_type_tint(id as u8, palette)
}

/// The material type is the low byte of the polygon's material id.
fn polygon_tint(
    geom: &PhBoundGeometry,
    polygon_index: usize,
    palette: &MaterialPalette,
) -> [f32; 4] {
    let material_idx = match geom.poly_mat_index_list.get(polygon_index) {
        Some(&i) => i as usize,
        None => return DEFAULT_BOUND_TINT,
    };
    let material = match geom.material_ids.get(material_idx) {
        Some(m) => m,
        None => return DEFAULT_BOUND_TINT,
    };
    material_type_tint(material.id as u8, palette)
}

fn material_type_tint(material_type: u8, palette: &MaterialPalette) -> [f32; 4] {
    palette_rgb_to_rgba(palette.rgb_for(material_type))
}

fn push_sphere_upload(header: &PhBound, transform: Mat4, builder: &mut BoundMeshBuilder) {
    let centre = header_centroid(header);
    let radius = header.radius_around_centroid;
    if radius <= 0.0 {
        return;
    }
    let tint = primitive_tint(header, builder.palette);
    let (vertices, indices) = uv_sphere(centre, radius, transform, tint);
    builder.push_uniform_bias(vertices, indices);
}

fn uv_sphere(
    centre: Vec3,
    radius: f32,
    transform: Mat4,
    tint: [f32; 4],
) -> (Vec<MeshVertex>, Vec<u32>) {
    let rings = RINGS + 2;
    let mut vertices = Vec::with_capacity(rings * (SEGMENTS + 1));
    for ring in 0..rings {
        let theta = PI * (ring as f32) / (rings as f32 - 1.0);
        let (sin_t, cos_t) = theta.sin_cos();
        for seg in 0..=SEGMENTS {
            let phi = TAU * (seg as f32) / SEGMENTS as f32;
            let (sin_p, cos_p) = phi.sin_cos();
            let normal = Vec3::new(sin_t * cos_p, cos_t, sin_t * sin_p);
            let position = centre + normal * radius;
            let (p, n) = apply_transform(transform, position, normal);
            vertices.push(MeshVertex::new_coloured(
                p,
                n,
                [
                    seg as f32 / SEGMENTS as f32,
                    ring as f32 / (rings as f32 - 1.0),
                ],
                tint,
            ));
        }
    }

    let mut indices = Vec::with_capacity(rings * SEGMENTS * 6);
    let row = SEGMENTS + 1;
    for ring in 0..rings - 1 {
        for seg in 0..SEGMENTS {
            let a = (ring * row + seg) as u32;
            let b = (ring * row + seg + 1) as u32;
            let c = ((ring + 1) * row + seg) as u32;
            let d = ((ring + 1) * row + seg + 1) as u32;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    (vertices, indices)
}

fn push_box_upload(header: &PhBound, transform: Mat4, builder: &mut BoundMeshBuilder) {
    let (mn, mx) = header_aabb(header);
    if (mx - mn).length_squared() <= 0.0 {
        return;
    }
    let tint = primitive_tint(header, builder.palette);

    let faces: [(Vec3, [Vec3; 4]); 6] = [
        (
            Vec3::X,
            [
                Vec3::new(mx.x, mn.y, mn.z),
                Vec3::new(mx.x, mx.y, mn.z),
                Vec3::new(mx.x, mx.y, mx.z),
                Vec3::new(mx.x, mn.y, mx.z),
            ],
        ),
        (
            -Vec3::X,
            [
                Vec3::new(mn.x, mn.y, mx.z),
                Vec3::new(mn.x, mx.y, mx.z),
                Vec3::new(mn.x, mx.y, mn.z),
                Vec3::new(mn.x, mn.y, mn.z),
            ],
        ),
        (
            Vec3::Y,
            [
                Vec3::new(mn.x, mx.y, mn.z),
                Vec3::new(mn.x, mx.y, mx.z),
                Vec3::new(mx.x, mx.y, mx.z),
                Vec3::new(mx.x, mx.y, mn.z),
            ],
        ),
        (
            -Vec3::Y,
            [
                Vec3::new(mn.x, mn.y, mx.z),
                Vec3::new(mn.x, mn.y, mn.z),
                Vec3::new(mx.x, mn.y, mn.z),
                Vec3::new(mx.x, mn.y, mx.z),
            ],
        ),
        (
            Vec3::Z,
            [
                Vec3::new(mn.x, mn.y, mx.z),
                Vec3::new(mx.x, mn.y, mx.z),
                Vec3::new(mx.x, mx.y, mx.z),
                Vec3::new(mn.x, mx.y, mx.z),
            ],
        ),
        (
            -Vec3::Z,
            [
                Vec3::new(mx.x, mn.y, mn.z),
                Vec3::new(mn.x, mn.y, mn.z),
                Vec3::new(mn.x, mx.y, mn.z),
                Vec3::new(mx.x, mx.y, mn.z),
            ],
        ),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for (i, corner) in corners.iter().enumerate() {
            let (p, n) = apply_transform(transform, *corner, normal);
            let uv = match i {
                0 => [0.0, 0.0],
                1 => [1.0, 0.0],
                2 => [1.0, 1.0],
                _ => [0.0, 1.0],
            };
            vertices.push(MeshVertex::new_coloured(p, n, uv, tint));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    builder.push_uniform_bias(vertices, indices);
}

fn push_capsule_upload(header: &PhBound, transform: Mat4, builder: &mut BoundMeshBuilder) {
    let (mn, mx) = header_aabb(header);
    let centre = aabb_centre(mn, mx);
    let (r, h) = capsule_extent(header);
    let radius = {
        if r > 0.0 {
            r
        } else {
            (mx.x - mn.x).max(mx.z - mn.z) * 0.5
        }
    };
    let half_length = {
        if h > 0.0 {
            h
        } else {
            ((mx.y - mn.y) * 0.5 - radius).max(0.0)
        }
    };
    if radius <= 0.0 {
        return;
    }

    let tint = primitive_tint(header, builder.palette);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    capsule_geometry(
        centre,
        radius,
        half_length,
        transform,
        tint,
        &mut vertices,
        &mut indices,
    );
    builder.push_uniform_bias(vertices, indices);
}

fn capsule_geometry(
    centre: Vec3,
    radius: f32,
    half_length: f32,
    transform: Mat4,
    tint: [f32; 4],
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    let top_centre = centre + Vec3::Y * half_length;
    let bottom_centre = centre - Vec3::Y * half_length;

    push_hemisphere(top_centre, radius, true, transform, tint, vertices, indices);
    push_hemisphere(
        bottom_centre,
        radius,
        false,
        transform,
        tint,
        vertices,
        indices,
    );

    let base = vertices.len() as u32;
    for ring in 0..2 {
        let y = if ring == 0 { half_length } else { -half_length };
        for seg in 0..=SEGMENTS {
            let phi = TAU * (seg as f32) / SEGMENTS as f32;
            let normal = Vec3::new(phi.cos(), 0.0, phi.sin());
            let position = centre + normal * radius + Vec3::Y * y;
            let (p, n) = apply_transform(transform, position, normal);
            vertices.push(MeshVertex::new_coloured(
                p,
                n,
                [seg as f32 / SEGMENTS as f32, ring as f32],
                tint,
            ));
        }
    }
    let row = (SEGMENTS + 1) as u32;
    for seg in 0..SEGMENTS as u32 {
        let a = base + seg;
        let b = base + seg + 1;
        let c = base + row + seg;
        let d = base + row + seg + 1;
        indices.extend_from_slice(&[a, b, c, b, d, c]);
    }
}

fn push_hemisphere(
    centre: Vec3,
    radius: f32,
    top: bool,
    transform: Mat4,
    tint: [f32; 4],
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
) {
    let rings = RINGS / 2 + 1;
    let base = vertices.len() as u32;
    for ring in 0..=rings {
        let t = (ring as f32) / (rings as f32);
        let theta = if top {
            t * PI * 0.5
        } else {
            (1.0 - t) * PI * 0.5 + PI * 0.5
        };
        let (sin_t, cos_t) = theta.sin_cos();
        for seg in 0..=SEGMENTS {
            let phi = TAU * (seg as f32) / SEGMENTS as f32;
            let (sin_p, cos_p) = phi.sin_cos();
            let normal = Vec3::new(sin_t * cos_p, cos_t, sin_t * sin_p);
            let position = centre + normal * radius;
            let (p, n) = apply_transform(transform, position, normal);
            vertices.push(MeshVertex::new_coloured(
                p,
                n,
                [seg as f32 / SEGMENTS as f32, t],
                tint,
            ));
        }
    }
    let row = (SEGMENTS + 1) as u32;
    for ring in 0..rings as u32 {
        for seg in 0..SEGMENTS as u32 {
            let a = base + ring * row + seg;
            let b = base + ring * row + seg + 1;
            let c = base + (ring + 1) * row + seg;
            let d = base + (ring + 1) * row + seg + 1;
            // The top's rings run down from its pole, the bottom's up to its rim.
            if top {
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            } else {
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
}

fn push_cylinder_upload(header: &PhBound, transform: Mat4, builder: &mut BoundMeshBuilder) {
    let (mn, mx) = header_aabb(header);
    let (radius, half_height) = cylinder_extent(header);
    if radius <= 0.0 || half_height <= 0.0 {
        return;
    }
    let centre = aabb_centre(mn, mx);
    let tint = primitive_tint(header, builder.palette);
    let (vertices, indices) = cylinder_geometry(centre, radius, half_height, transform, tint);
    builder.push_uniform_bias(vertices, indices);
}

fn cylinder_geometry(
    centre: Vec3,
    radius: f32,
    half_height: f32,
    transform: Mat4,
    tint: [f32; 4],
) -> (Vec<MeshVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    let side_base = vertices.len() as u32;
    for ring in 0..2 {
        let y = if ring == 0 { half_height } else { -half_height };
        for seg in 0..=SEGMENTS {
            let phi = TAU * (seg as f32) / SEGMENTS as f32;
            let normal = Vec3::new(phi.cos(), 0.0, phi.sin());
            let position = centre + normal * radius + Vec3::Y * y;
            let (p, n) = apply_transform(transform, position, normal);
            vertices.push(MeshVertex::new_coloured(
                p,
                n,
                [seg as f32 / SEGMENTS as f32, ring as f32],
                tint,
            ));
        }
    }
    let row = (SEGMENTS + 1) as u32;
    for seg in 0..SEGMENTS as u32 {
        let a = side_base + seg;
        let b = side_base + seg + 1;
        let c = side_base + row + seg;
        let d = side_base + row + seg + 1;
        indices.extend_from_slice(&[a, b, c, b, d, c]);
    }

    push_cap_fan(
        &mut vertices,
        &mut indices,
        centre,
        radius,
        half_height,
        true,
        transform,
        tint,
    );
    push_cap_fan(
        &mut vertices,
        &mut indices,
        centre,
        radius,
        half_height,
        false,
        transform,
        tint,
    );

    (vertices, indices)
}

#[allow(clippy::too_many_arguments)]
fn push_cap_fan(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    centre: Vec3,
    radius: f32,
    half_height: f32,
    top: bool,
    transform: Mat4,
    tint: [f32; 4],
) {
    let y = if top { half_height } else { -half_height };
    let normal = if top { Vec3::Y } else { -Vec3::Y };
    let cap_centre = centre + Vec3::Y * y;

    let base = vertices.len() as u32;
    let (cp, cn) = apply_transform(transform, cap_centre, normal);
    vertices.push(MeshVertex::new_coloured(cp, cn, [0.5, 0.5], tint));
    for seg in 0..=SEGMENTS {
        let phi = TAU * (seg as f32) / SEGMENTS as f32;
        let position = cap_centre + Vec3::new(phi.cos(), 0.0, phi.sin()) * radius;
        let (p, n) = apply_transform(transform, position, normal);
        let uv = [phi.cos() * 0.5 + 0.5, phi.sin() * 0.5 + 0.5];
        vertices.push(MeshVertex::new_coloured(p, n, uv, tint));
    }
    for seg in 0..SEGMENTS as u32 {
        let a = base;
        let b = base + 1 + seg;
        let c = base + 2 + seg;
        if top {
            indices.extend_from_slice(&[a, c, b]);
        } else {
            indices.extend_from_slice(&[a, b, c]);
        }
    }
}

fn push_disc_upload(header: &PhBound, transform: Mat4, builder: &mut BoundMeshBuilder) {
    let (mn, mx) = header_aabb(header);
    let radius = if header.radius_around_centroid > 0.0 {
        header.radius_around_centroid
    } else {
        (mx.y - mn.y).max(mx.z - mn.z) * 0.5
    };
    let half_thickness = disc_margin(header).max(0.001);
    if radius <= 0.0 {
        return;
    }
    let centre = aabb_centre(mn, mx);
    let tint = primitive_tint(header, builder.palette);
    let disc_to_world = transform * axis_transform(centre, Vec3::X);
    let (vertices, indices) =
        cylinder_geometry(Vec3::ZERO, radius, half_thickness, disc_to_world, tint);
    builder.push_uniform_bias(vertices, indices);
}

fn push_polyhedron_upload(geom: &PhBoundGeometry, transform: Mat4, builder: &mut BoundMeshBuilder) {
    if geom.compressed_vertices.is_empty() || geom.polygons.is_empty() {
        return;
    }
    let positions = geometry_positions(geom, transform);

    let mut vertices = Vec::with_capacity(geom.polygons.len() * 3);
    let mut indices = Vec::with_capacity(geom.polygons.len() * 3);
    for (poly_idx, poly) in geom.polygons.iter().enumerate() {
        let tint = polygon_tint(geom, poly_idx, builder.palette);
        let Some((named, radius)) = geom.polygon_vertices(poly) else {
            continue;
        };
        let at = |i: usize| named.get(i).and_then(|v| positions.get(*v)).copied();
        match poly.primitive() {
            0 => {
                if let (Some(a), Some(b), Some(c)) = (at(0), at(1), at(2)) {
                    append_polygon_triangle(&mut vertices, &mut indices, a, b, c, tint);
                }
            }
            1 => {
                if let Some(centre) = at(0) {
                    append_polygon_sphere(&mut vertices, &mut indices, centre, radius, tint);
                }
            }
            2 => {
                if let (Some(a), Some(b)) = (at(0), at(1)) {
                    append_polygon_capsule(&mut vertices, &mut indices, a, b, radius, tint);
                }
            }
            4 => {
                if let (Some(a), Some(b)) = (at(0), at(1)) {
                    append_polygon_cylinder(&mut vertices, &mut indices, a, b, radius, tint);
                }
            }
            3 => {
                if let (Some(a), Some(b), Some(c), Some(d)) = (at(0), at(1), at(2), at(3)) {
                    append_polygon_box(&mut vertices, &mut indices, [a, b, c, d], tint);
                }
            }
            _ => {}
        }
    }
    builder.push_uniform_bias(vertices, indices);
}

pub(crate) fn append_polygon_triangle(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    tint: [f32; 4],
) {
    let face_normal = (b - a).cross(c - a).normalize_or_zero();
    let n = if face_normal == Vec3::ZERO {
        [0.0, 0.0, 1.0]
    } else {
        [face_normal.x, face_normal.y, face_normal.z]
    };
    let base = vertices.len() as u32;
    vertices.push(MeshVertex::new_coloured(
        [a.x, a.y, a.z],
        n,
        [0.0, 0.0],
        tint,
    ));
    vertices.push(MeshVertex::new_coloured(
        [b.x, b.y, b.z],
        n,
        [1.0, 0.0],
        tint,
    ));
    vertices.push(MeshVertex::new_coloured(
        [c.x, c.y, c.z],
        n,
        [0.0, 1.0],
        tint,
    ));
    indices.extend_from_slice(&[base, base + 1, base + 2]);
}

pub(crate) fn append_polygon_sphere(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    centre_world: Vec3,
    radius: f32,
    tint: [f32; 4],
) {
    if radius <= 0.0 {
        return;
    }
    let (sphere_v, sphere_i) = uv_sphere(centre_world, radius, Mat4::IDENTITY, tint);
    let base = vertices.len() as u32;
    vertices.extend(sphere_v);
    indices.extend(sphere_i.into_iter().map(|i| base + i));
}

pub(crate) fn append_polygon_capsule(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    endpoint_a: Vec3,
    endpoint_b: Vec3,
    radius: f32,
    tint: [f32; 4],
) {
    if radius <= 0.0 {
        return;
    }
    let centre = (endpoint_a + endpoint_b) * 0.5;
    let half_length = (endpoint_b - endpoint_a).length() * 0.5;
    if half_length <= 0.0 {
        append_polygon_sphere(vertices, indices, centre, radius, tint);
        return;
    }
    let local_to_world = axis_transform(centre, endpoint_b - endpoint_a);
    let mut local_v = Vec::new();
    let mut local_i = Vec::new();
    capsule_geometry(
        Vec3::ZERO,
        radius,
        half_length,
        local_to_world,
        tint,
        &mut local_v,
        &mut local_i,
    );
    let base = vertices.len() as u32;
    vertices.extend(local_v);
    indices.extend(local_i.into_iter().map(|i| base + i));
}

pub(crate) fn append_polygon_cylinder(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    endpoint_a: Vec3,
    endpoint_b: Vec3,
    radius: f32,
    tint: [f32; 4],
) {
    if radius <= 0.0 {
        return;
    }
    let centre = (endpoint_a + endpoint_b) * 0.5;
    let half_height = (endpoint_b - endpoint_a).length() * 0.5;
    if half_height <= 0.0 {
        return;
    }
    let local_to_world = axis_transform(centre, endpoint_b - endpoint_a);
    let (local_v, local_i) =
        cylinder_geometry(Vec3::ZERO, radius, half_height, local_to_world, tint);
    let base = vertices.len() as u32;
    vertices.extend(local_v);
    indices.extend(local_i.into_iter().map(|i| base + i));
}

pub(crate) fn append_polygon_box(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    corners4: [Vec3; 4],
    tint: [f32; 4],
) {
    let p1 = corners4[0];
    let p2 = corners4[1];
    let p3 = corners4[2];
    let p4 = corners4[3];
    let a1 = ((p3 + p4) - (p1 + p2)) * 0.5;
    let corner = p1;
    let edge1 = a1;
    let edge2 = (p3 - a1) - p1;
    let edge3 = (p4 - a1) - p1;

    let mut box_corners = [Vec3::ZERO; 8];
    for (idx, slot) in box_corners.iter_mut().enumerate() {
        let i = (idx & 1) as f32;
        let j = ((idx >> 1) & 1) as f32;
        let k = ((idx >> 2) & 1) as f32;
        *slot = corner + edge1 * i + edge2 * j + edge3 * k;
    }

    let faces: [[usize; 4]; 6] = [
        [0, 4, 6, 2],
        [1, 3, 7, 5],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 2, 3, 1],
        [4, 5, 7, 6],
    ];
    for face in faces {
        let a = box_corners[face[0]];
        let b = box_corners[face[1]];
        let c = box_corners[face[2]];
        let d = box_corners[face[3]];
        let n = (b - a).cross(c - a).normalize_or_zero();
        let n_arr = if n == Vec3::ZERO {
            [0.0, 0.0, 1.0]
        } else {
            [n.x, n.y, n.z]
        };
        let base = vertices.len() as u32;
        let quad = [a, b, c, d];
        for (i, p) in quad.iter().enumerate() {
            let uv = match i {
                0 => [0.0, 0.0],
                1 => [1.0, 0.0],
                2 => [1.0, 1.0],
                _ => [0.0, 1.0],
            };
            vertices.push(MeshVertex::new_coloured([p.x, p.y, p.z], n_arr, uv, tint));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

fn axis_transform(centre: Vec3, direction: Vec3) -> Mat4 {
    let y = direction.normalize_or_zero();
    if y == Vec3::ZERO {
        return Mat4::from_translation(centre);
    }
    let helper = if y.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let x = helper.cross(y).normalize_or_zero();
    let x = if x == Vec3::ZERO { Vec3::X } else { x };
    let z = x.cross(y).normalize_or_zero();
    Mat4::from_cols(
        glam::Vec4::new(x.x, x.y, x.z, 0.0),
        glam::Vec4::new(y.x, y.y, y.z, 0.0),
        glam::Vec4::new(z.x, z.y, z.z, 0.0),
        glam::Vec4::new(centre.x, centre.y, centre.z, 1.0),
    )
}

/// Bounds built in memory, for the render tests.
#[cfg(test)]
pub(crate) mod shapes {
    use or_io::decl::Open;
    use or_io::gta5::{
        BoundType, Mat34V, PhBound, PhBoundAny, PhBoundBox, PhBoundCapsule, PhBoundComposite,
        PhBoundCylinder, PhBoundDisc, PhBoundSphere, Vec4V,
    };

    fn v4(x: f32, y: f32, z: f32, w: f32) -> Vec4V {
        Vec4V {
            x,
            y,
            z,
            w,
            ..Default::default()
        }
    }

    /// A header with `min`..`max` as its box and `margin` in the box's `w`.
    pub(crate) fn header(
        ty: BoundType,
        centre: [f32; 3],
        radius: f32,
        (min, max): ([f32; 3], [f32; 3]),
        margin: f32,
    ) -> PhBound {
        let mut h = PhBound {
            r#type: Open::new(ty),
            radius_around_centroid: radius,
            bounding_box_max_xyz_margin_w: v4(max[0], max[1], max[2], margin),
            centroid_offset_xyz_material_id0_w: v4(centre[0], centre[1], centre[2], 0.0),
            ..Default::default()
        };
        h.set_bounding_box_min(min);
        h
    }

    fn around(c: [f32; 3], half: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        (
            std::array::from_fn(|i| c[i] - half[i]),
            std::array::from_fn(|i| c[i] + half[i]),
        )
    }

    pub(crate) fn sphere(c: [f32; 3], radius: f32) -> PhBoundAny {
        PhBoundAny::PhBoundSphere(PhBoundSphere {
            base: header(BoundType::Sphere, c, radius, around(c, [radius; 3]), radius),
            ..Default::default()
        })
    }

    pub(crate) fn box_bound(c: [f32; 3], half: [f32; 3]) -> PhBoundAny {
        let radius = half.iter().map(|h| h * h).sum::<f32>().sqrt();
        PhBoundAny::PhBoundBox(PhBoundBox {
            base: header(BoundType::Box, c, radius, around(c, half), 0.0),
            ..Default::default()
        })
    }

    /// A capsule along Y whose segment is `2 * half_length` long.
    pub(crate) fn capsule(c: [f32; 3], radius: f32, half_length: f32) -> PhBoundAny {
        PhBoundAny::PhBoundCapsule(PhBoundCapsule {
            base: header(
                BoundType::Capsule,
                c,
                radius + half_length,
                around(c, [radius, radius + half_length, radius]),
                radius,
            ),
            ..Default::default()
        })
    }

    /// A cylinder along Y, `height` tall.
    pub(crate) fn cylinder(c: [f32; 3], radius: f32, height: f32) -> PhBoundAny {
        let half = height * 0.5;
        let around_centroid = (radius * radius + half * half).sqrt();
        PhBoundAny::PhBoundCylinder(PhBoundCylinder {
            base: header(
                BoundType::Cylinder,
                c,
                around_centroid,
                around(c, [radius, half, radius]),
                0.0,
            ),
            ..Default::default()
        })
    }

    /// A disc flat along X, `margin` thick either side.
    pub(crate) fn disc(c: [f32; 3], radius: f32, margin: f32) -> PhBoundAny {
        PhBoundAny::PhBoundDisc(PhBoundDisc {
            base: header(
                BoundType::Disc,
                c,
                radius,
                around(c, [margin, radius, radius]),
                margin,
            ),
            ..Default::default()
        })
    }

    fn identity() -> Mat34V {
        Mat34V {
            col0: v4(1.0, 0.0, 0.0, 0.0),
            col1: v4(0.0, 1.0, 0.0, 0.0),
            col2: v4(0.0, 0.0, 1.0, 0.0),
            col3: v4(0.0, 0.0, 0.0, 0.0),
            ..Default::default()
        }
    }

    /// The children at the composite's origin.
    pub(crate) fn composite(children: Vec<PhBoundAny>) -> PhBoundAny {
        PhBoundAny::PhBoundComposite(PhBoundComposite {
            base: header(
                BoundType::Composite,
                [0.0; 3],
                0.0,
                ([0.0; 3], [0.0; 3]),
                0.0,
            ),
            current_matrices: children.iter().map(|_| identity()).collect(),
            bounds: children.into_iter().map(|c| Some(Box::new(c))).collect(),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::shapes::{box_bound, composite, cylinder, sphere};
    use super::*;
    use or_io::gta5::{Color32, CompressedVertex, PhMaterialMgrId, PhPolygon, Vec3V};

    /// The polygons a bound tree draws as triangles.
    fn triangles(bound: &PhBoundAny) -> usize {
        let geometry = match bound {
            PhBoundAny::PhBoundGeometry(g) => g,
            PhBoundAny::PhBoundBVH(b) => &b.base,
            PhBoundAny::PhBoundComposite(c) => {
                return c
                    .bounds
                    .iter()
                    .flatten()
                    .map(|child| triangles(child))
                    .sum();
            }
            _ => return 0,
        };
        geometry
            .polygons
            .iter()
            .filter(|p| p.primitive() == 0)
            .count()
    }

    #[test]
    fn every_corpus_bound_uploads_its_triangles() {
        const WHAT: &str = "every_corpus_bound_uploads_its_triangles";
        let paths = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ybn").find();
        if paths.is_empty() {
            or_test_support::skip_corpus(WHAT, "no .ybn staged");
            return;
        }
        for path in &paths {
            let data = std::fs::read(path).expect("read a corpus .ybn");
            let owned = <PhBoundAny as or_io::gta5::File>::from_bytes(&data)
                .unwrap_or_else(|e| panic!("{}: the owned bound refuses it: {e}", path.display()));
            let want = triangles(&owned);
            let uploads = build_bound_meshes(&owned, test_palette());
            let indices: usize = uploads.iter().map(|u| u.indices.len()).sum();
            assert!(
                indices >= 3 * want && (want == 0 || !uploads.is_empty()),
                "{}: the owned walk uploads {indices} indices for {want} triangles",
                path.display(),
            );
        }
    }

    #[test]
    fn axis_transform_is_right_handed() {
        for direction in [
            Vec3::X,
            Vec3::Y,
            Vec3::Z,
            Vec3::new(1.0, 1.0, 1.0).normalize(),
            Vec3::new(-0.7, 0.2, 0.5).normalize(),
        ] {
            let m = axis_transform(Vec3::ZERO, direction);
            let m3 = Mat3::from_mat4(m);
            assert!(
                m3.determinant() > 0.0,
                "axis_transform produced a reflection (det = {}) for {:?}",
                m3.determinant(),
                direction
            );
            let mapped = m3 * Vec3::Y;
            assert!(
                mapped.distance(direction) < 1e-5,
                "axis_transform should map +Y onto direction; got {:?} for {:?}",
                mapped,
                direction
            );
        }
    }

    #[test]
    fn polygon_capsule_normals_point_outward() {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        append_polygon_capsule(
            &mut vertices,
            &mut indices,
            Vec3::new(-2.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            0.5,
            [1.0, 1.0, 1.0, 1.0],
        );
        assert!(!vertices.is_empty(), "capsule polygon emitted no geometry");

        let mut tested = 0usize;
        for v in &vertices {
            let radial = Vec3::new(0.0, v.position[1], v.position[2]);
            if radial.length_squared() < 1e-6 {
                continue;
            }
            let radial = radial.normalize();
            let n = Vec3::new(v.normal[0], v.normal[1], v.normal[2]);
            let dot = radial.dot(n);
            assert!(
                dot > -0.05,
                "vertex normal {:?} points inward (radial = {:?}, dot = {dot})",
                n,
                radial
            );
            tested += 1;
        }
        assert!(tested > 0, "test didn't actually inspect any vertices");
    }

    /// The triangles whose winding faces away from their vertex normals, which the shader then flips inward.
    fn inward_triangles(vertices: &[MeshVertex], indices: &[u32]) -> usize {
        (indices.as_chunks::<3>().0.iter())
            .filter(|t| {
                let v = |i: u32| &vertices[i as usize];
                let p = |i: u32| Vec3::from(v(i).position);
                let face = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
                let normal: Vec3 = t.iter().map(|&i| Vec3::from(v(i).normal)).sum();
                face.length_squared() > 1e-12 && face.dot(normal) <= 0.0
            })
            .count()
    }

    #[test]
    fn every_primitive_is_wound_to_face_its_normals() {
        use super::shapes::{capsule, disc};
        let shapes = [
            ("sphere", sphere([0.0; 3], 1.0)),
            ("box", box_bound([3.0, 0.0, 0.0], [0.5, 1.0, 1.5])),
            ("capsule", capsule([6.0, 0.0, 0.0], 0.75, 1.0)),
            ("cylinder", cylinder([9.0, 0.0, 0.0], 0.75, 2.0)),
            ("disc", disc([12.0, 0.0, 0.0], 1.0, 0.1)),
        ];
        for (name, shape) in &shapes {
            let uploads = build(shape);
            assert!(!uploads.is_empty(), "{name} uploads nothing");
            for u in &uploads {
                assert_eq!(inward_triangles(&u.vertices, &u.indices), 0, "{name}");
            }
        }
        let tint = [1.0; 4];
        let (a, b) = (Vec3::new(-1.0, 0.5, 0.0), Vec3::new(1.0, -0.5, 2.0));
        type Append = fn(&mut Vec<MeshVertex>, &mut Vec<u32>, Vec3, Vec3, f32, [f32; 4]);
        let polygons: [(&str, Append); 2] = [
            ("polygon capsule", append_polygon_capsule),
            ("polygon cylinder", append_polygon_cylinder),
        ];
        for (name, append) in polygons {
            let (mut vertices, mut indices) = (Vec::new(), Vec::new());
            append(&mut vertices, &mut indices, a, b, 0.5, tint);
            assert!(!indices.is_empty(), "{name}");
            assert_eq!(inward_triangles(&vertices, &indices), 0, "{name}");
        }
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        append_polygon_sphere(&mut vertices, &mut indices, a, 0.5, tint);
        assert_eq!(inward_triangles(&vertices, &indices), 0, "polygon sphere");
    }

    fn count_geometry(uploads: &[MeshUpload]) -> (usize, usize) {
        let v: usize = uploads.iter().map(|u| u.vertices.len()).sum();
        let i: usize = uploads.iter().map(|u| u.indices.len()).sum();
        (v, i)
    }

    /// A palette with a distinct colour per material type, standing in for
    /// the one an install supplies.
    fn test_palette() -> &'static MaterialPalette {
        static PALETTE: std::sync::OnceLock<MaterialPalette> = std::sync::OnceLock::new();
        PALETTE.get_or_init(|| {
            let entries = (0..=u8::MAX)
                .map(
                    |i| or_io::gta5::data::materials_palette::MaterialPaletteEntry {
                        name: "TEST",
                        fx_group: "TEST",
                        rgb: [i, i.wrapping_mul(3), 255 - i],
                    },
                )
                .collect();
            MaterialPalette::from_runtime(
                entries,
                or_io::gta5::data::materials_palette::MaterialPaletteSource::GtaInstall {
                    install_root: std::path::PathBuf::from("test"),
                },
            )
        })
    }

    fn build(bound: &PhBoundAny) -> Vec<MeshUpload> {
        build_bound_meshes(bound, test_palette())
    }

    /// A header whose primitive material id is `id`.
    fn with_material(id: u32) -> PhBound {
        let mut h = PhBound::default();
        h.centroid_offset_xyz_material_id0_w.w = f32::from_bits(id);
        h
    }

    fn material_type_tint_of(material_type: u8) -> [f32; 4] {
        material_type_tint(material_type, test_palette())
    }

    fn vertex(x: i16, y: i16, z: i16) -> CompressedVertex {
        CompressedVertex {
            x,
            y,
            z,
            ..Default::default()
        }
    }

    fn triangle(vertex_indices: [u16; 3]) -> PhPolygon {
        PhPolygon {
            area: 0.0,
            vertex_indices,
            neighboring_polygons: [or_io::gta5::bounds::NO_NEIGHBOUR; 3],
            ..Default::default()
        }
    }

    fn material(id: u64) -> PhMaterialMgrId {
        PhMaterialMgrId {
            id,
            ..Default::default()
        }
    }

    fn scale(s: f32) -> Vec3V {
        Vec3V {
            x: s,
            y: s,
            z: s,
            ..Default::default()
        }
    }

    /// Three vertices a hundred units apart along X and Y, and `polygons` over them.
    fn geometry(polygons: Vec<PhPolygon>) -> PhBoundGeometry {
        PhBoundGeometry {
            un_quantize_factor: scale(1.0),
            compressed_vertices: vec![vertex(0, 0, 0), vertex(100, 0, 0), vertex(0, 100, 0)],
            polygons,
            ..Default::default()
        }
    }

    #[test]
    fn sphere_bound_emits_solid_mesh() {
        let uploads = build(&sphere([0.0; 3], 1.0));
        let (verts, idx) = count_geometry(&uploads);
        assert!(verts > 0 && idx >= 3, "sphere produced no triangles");
        for u in &uploads {
            for v in &u.vertices {
                let r =
                    (v.position[0].powi(2) + v.position[1].powi(2) + v.position[2].powi(2)).sqrt();
                assert!(
                    (r - 1.0).abs() < 1e-3,
                    "vertex {:?} not on unit sphere (r={r})",
                    v.position
                );
            }
        }
    }

    #[test]
    fn cylinder_bound_emits_a_solid_mesh_from_its_box() {
        let uploads = build(&cylinder([0.0; 3], 1.5, 4.0));
        let (verts, idx) = count_geometry(&uploads);
        assert!(verts > 0 && idx >= 3, "cylinder produced no triangles");
        let on_wall = uploads.iter().flat_map(|u| u.vertices.iter()).any(|v| {
            let r = (v.position[0].powi(2) + v.position[2].powi(2)).sqrt();
            (r - 1.5).abs() < 1e-3 && v.position[1].abs() < 2.0 + 1e-3
        });
        assert!(on_wall, "cylinder solid mesh missing curved side wall");
    }

    #[test]
    fn box_bound_emits_24_vertices() {
        let uploads = build(&box_bound([0.0; 3], [1.0; 3]));
        let (verts, idx) = count_geometry(&uploads);
        assert_eq!(verts, 24, "box should have 24 per-face vertices");
        assert_eq!(idx, 36, "box should have 36 indices (12 tris)");
    }

    #[test]
    fn composite_walks_children() {
        let uploads = build(&composite(vec![
            sphere([10.0, 0.0, 0.0], 1.0),
            box_bound([-10.0, 0.0, 0.0], [1.0; 3]),
        ]));
        assert!(
            uploads.len() >= 2,
            "composite emitted only {}",
            uploads.len()
        );
    }

    #[test]
    fn primitive_tint_is_keyed_by_material_type() {
        // Type 7 with a procedural id and a room, type 7 with a colour index, and type 42.
        let rock_a = primitive_tint(&with_material(7 | (11 << 8) | (3 << 16)), test_palette());
        let rock_b = primitive_tint(&with_material(7 | (99 << 24)), test_palette());
        let wood = primitive_tint(&with_material(42), test_palette());
        assert_eq!(
            rock_a, rock_b,
            "same material type should tint the same regardless of procedural / room / colour bits"
        );
        assert_ne!(
            rock_a, wood,
            "different material types should produce different tints"
        );
    }

    #[test]
    fn polyhedron_polygon_uses_material_type_palette() {
        let mut geom = geometry(vec![triangle([0, 1, 2])]);
        geom.material_ids = vec![material(7)];
        geom.poly_mat_index_list = vec![0];

        let uploads = build(&PhBoundAny::PhBoundGeometry(geom));
        assert!(!uploads.is_empty());

        let expected = primitive_tint(&with_material(7), test_palette());
        let v = uploads[0].vertices[0];
        for (i, (got, want)) in v.colour.iter().zip(expected.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-3,
                "channel {i}: polygon tint {:?} does not match material-type tint {:?}",
                v.colour,
                expected
            );
        }
    }

    #[test]
    fn coplanar_bounds_get_separated_by_per_emit_depth_bias() {
        let uploads = build(&composite(vec![
            sphere([0.0; 3], 1.0),
            sphere([0.0; 3], 1.0),
        ]));
        assert_eq!(uploads.len(), 2, "expected two sphere uploads");

        for u in &uploads {
            for v in &u.vertices {
                let r =
                    (v.position[0].powi(2) + v.position[1].powi(2) + v.position[2].powi(2)).sqrt();
                assert!(
                    (r - 1.0).abs() < 1e-4,
                    "vertex position must stay on the unit sphere; the bias \
                     belongs in `depth_bias`, not in `position`. got r = {r}"
                );
            }
        }

        for v in &uploads[0].vertices {
            assert_eq!(
                v.depth_bias, 0.0,
                "first emit should be unbiased (depth_bias=0)"
            );
        }
        let bias1 = uploads[1].vertices[0].depth_bias;
        assert!(
            bias1 != 0.0,
            "second emit should carry a non-zero depth_bias"
        );
        assert!(
            bias1 <= MAX_DEPTH_BIAS,
            "second emit's depth_bias {bias1} exceeded MAX_DEPTH_BIAS"
        );
        for v in &uploads[1].vertices {
            assert_eq!(
                v.depth_bias, bias1,
                "all vertices of a primitive emit must share the same depth_bias"
            );
        }
    }

    #[test]
    fn polyhedron_polygons_share_one_depth_bias_per_upload() {
        let make_polyhedron = || {
            PhBoundAny::PhBoundGeometry(geometry(vec![triangle([0, 1, 2]), triangle([0, 2, 1])]))
        };

        let uploads = build(&composite(vec![make_polyhedron(), make_polyhedron()]));
        assert_eq!(uploads.len(), 2, "expected two polyhedron uploads");

        for (i, u) in uploads.iter().enumerate() {
            let first = u.vertices[0].depth_bias;
            assert!(
                u.vertices.iter().all(|v| v.depth_bias == first),
                "polyhedron upload {i}: every vertex must share one bias \
                 (per-polygon biases shred the continuous mesh into floating \
                 triangles with bleed-through at every shared edge)"
            );
        }
        assert_ne!(
            uploads[0].vertices[0].depth_bias, uploads[1].vertices[0].depth_bias,
            "different polyhedra in one composite must still get different \
             biases — that's the per-upload separation that defeats Z-fighting \
             between distinct overlapping meshes"
        );
    }

    #[test]
    fn depth_bias_is_never_negative() {
        for i in 0..4096 {
            let bias = depth_bias_for_index(i);
            assert!(
                (0.0..=MAX_DEPTH_BIAS).contains(&bias),
                "depth bias for index {i} = {bias} outside [0, {MAX_DEPTH_BIAS}]"
            );
        }
    }

    #[test]
    fn depth_bias_minimum_spacing_is_well_above_float_precision() {
        const N: usize = 256;
        let mut biases: Vec<f32> = (1..N).map(depth_bias_for_index).collect();
        biases.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut min_gap = f32::INFINITY;
        for w in biases.windows(2) {
            min_gap = min_gap.min(w[1] - w[0]);
        }
        let expected_min = MAX_DEPTH_BIAS / (N as f32 * 4.0);
        assert!(
            min_gap > expected_min,
            "minimum bias gap {min_gap} fell below the LDS floor \
             {expected_min}; the distribution is broken and Z-fighting \
             will return"
        );
        // A unit-fraction gap of `1e-4` is ~5 µm of world-space pull at the
        // shader's 5 cm peak budget, several Float32-NDC ULPs even at long
        // zoom-out.
        assert!(
            min_gap > 1.0e-4,
            "minimum bias gap {min_gap} (unit fraction) is too small \
             to translate to a meaningful depth-buffer separation"
        );
    }

    #[test]
    fn polyhedron_renders_embedded_box_polygon_as_a_parallelepiped() {
        let mut geom = PhBoundGeometry {
            un_quantize_factor: scale(0.01),
            compressed_vertices: vec![
                vertex(0, 0, 0),
                vertex(0, 0, 0),
                vertex(200, 100, 0),
                vertex(200, 0, 100),
            ],
            ..Default::default()
        };

        // A box keeps its four indices at bytes 4..12: a triangle's three and its first neighbour.
        let bx = PhPolygon {
            area: f32::from_bits(3u32),
            vertex_indices: [0, 1, 2],
            neighboring_polygons: [3, 0, 0],
            ..Default::default()
        };
        assert_eq!(
            geom.polygon_vertices(&bx).map(|(v, _)| v),
            Some(vec![0, 1, 2, 3])
        );

        geom.polygons = vec![bx];
        let uploads = build(&PhBoundAny::PhBoundGeometry(geom));
        assert!(!uploads.is_empty(), "box polygon emitted no mesh");

        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for u in &uploads {
            for v in &u.vertices {
                for i in 0..3 {
                    min[i] = min[i].min(v.position[i]);
                    max[i] = max[i].max(v.position[i]);
                }
            }
        }
        for axis in 0..3 {
            assert!(
                max[axis] - min[axis] > 0.1,
                "box polygon has zero extent on axis {axis} ({:?}..{:?})",
                min,
                max
            );
        }
    }

    #[test]
    fn polyhedron_renders_embedded_sphere_polygon_as_a_sphere() {
        // A sphere keeps its centre's index at bytes 2..4 and its radius at 4..8.
        let radius_bits = 5.0_f32.to_bits().to_le_bytes();
        let sphere = PhPolygon {
            area: f32::from_bits(1u32),
            vertex_indices: [
                u16::from_le_bytes([radius_bits[0], radius_bits[1]]),
                u16::from_le_bytes([radius_bits[2], radius_bits[3]]),
                0,
            ],
            neighboring_polygons: [0; 3],
            ..Default::default()
        };
        let geom = geometry(vec![triangle([0, 1, 2]), sphere.clone()]);
        let (centre, radius) = geom.polygon_vertices(&sphere).expect("a sphere");
        assert_eq!(centre, vec![0]);
        assert!((radius - 5.0).abs() < 1e-3);

        let uploads = build(&PhBoundAny::PhBoundGeometry(geom));
        let total_verts: usize = uploads.iter().map(|u| u.vertices.len()).sum();
        let total_indices: usize = uploads.iter().map(|u| u.indices.len()).sum();
        assert!(
            total_verts > 3,
            "expected the sphere polygon to add tessellation vertices, got {total_verts}"
        );
        assert!(
            total_indices > 3,
            "expected the sphere polygon to add tessellation indices, got {total_indices}"
        );
        let on_sphere = uploads.iter().flat_map(|u| u.vertices.iter()).any(|v| {
            let r = (v.position[0].powi(2) + v.position[1].powi(2) + v.position[2].powi(2)).sqrt();
            (r - 5.0).abs() < 1e-3
        });
        assert!(
            on_sphere,
            "no emitted vertex sits on the polygon-sphere surface"
        );
    }

    #[test]
    fn embedded_material_colors_palette_is_ignored() {
        let mut geom = geometry(vec![triangle([0, 1, 2])]);
        geom.material_ids = vec![material(7)];
        geom.material_colors = vec![Color32 {
            color: 0xFF00_00FF,
            ..Default::default()
        }];
        geom.poly_mat_index_list = vec![0];

        let uploads = build(&PhBoundAny::PhBoundGeometry(geom));
        assert!(!uploads.is_empty(), "polyhedron emitted no triangles");
        let red = [1.0_f32, 0.0, 0.0, 1.0];
        let expected = material_type_tint_of(7);
        for u in &uploads {
            for v in &u.vertices {
                assert_ne!(
                    v.colour, red,
                    "embedded material_colors[] should be ignored, but vertex came out red"
                );
                for (i, (got, want)) in v.colour.iter().zip(expected.iter()).enumerate() {
                    assert!(
                        (got - want).abs() < 1e-3,
                        "channel {i}: polygon tint {:?} should match materials.dat palette tint {:?}",
                        v.colour,
                        expected
                    );
                }
            }
        }
    }
}
