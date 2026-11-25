//! `rpf scan-import`: a scanned or loose GTA VI drawable's geometries replaced from a glTF mesh,
//! as `mesh import` replaces a GTA V drawable's, written as a loose resource that `rpf scan-open`
//! reads at offset 0.

use std::collections::BTreeSet;
use std::fs;
use std::io::Cursor;
use std::path::Path;

use or_interchange::gltf::read::lod_and_model_in_name;
use or_interchange::{LodSlot, Mesh, Primitive, Scene};
use or_io::container::rpf::{
    read_rpf9_body, write_rpf9_resource, Rpf9BodyKind, Rpf9Role, Rpf9Sections,
};
use or_io::gta6::headerless::rebox_bind_pose;
use or_io::gta6::headerless::{
    read_dictionary, read_drawable, read_drawable_with_bound, read_skeleton_of, write_geometry_cut,
    write_palette, Drawable, Entry, Geometry, GeometryAt, GeometryEdit, Model, SkeletonOwner,
};

mod fill;
mod skin;

const USAGE: &str =
    "usage: rpf scan-import <archive.rpf|file> <offset> <mesh.glb|mesh.gltf> <out> \
                     [--entry <code>] [--limit <bytes>] [--fill-missing]";

pub(crate) fn cmd_scan_import(args: &[String]) -> Result<(), String> {
    let (fill, args): (Vec<String>, Vec<String>) =
        args.iter().cloned().partition(|a| a == "--fill-missing");
    let (limit, args) = super::rpf::split_limit(&args)?;
    let (entry, args) = super::rpf::split_value(&args, "--entry")?;
    let entry = entry
        .map(|word| {
            let code = super::rpf::parse_offset(&word)?;
            u32::try_from(code).map_err(|_| format!("{word} is not a 32-bit entry code"))
        })
        .transpose()?;
    let [path, offset, mesh, out] = args.as_slice() else {
        return Err(USAGE.to_string());
    };
    let offset = super::rpf::parse_offset(offset)?;
    if super::navmesh_edit::same_file(path, out) {
        return Err(format!("the output is the input {path}: name another file"));
    }
    let file = fs::File::open(path).map_err(|e| format!("open({path}): {e}"))?;
    let (find, bytes) = read_rpf9_body(&file, offset, limit).map_err(|e| e.to_string())?;
    let sections = Rpf9Sections::of(&find, &bytes)
        .ok_or_else(|| format!("the body at {offset:#X} is not a resource"))?;
    let role = match find.kind {
        Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
        _ => None,
    };
    let role =
        role.ok_or_else(|| format!("the body at {offset:#X} is no headerless GTA VI resource"))?;
    let source = fs::read(mesh).map_err(|e| format!("read({mesh}): {e}"))?;
    let scene = or_interchange::gltf::read::load(Path::new(mesh), &source)
        .map_err(|e| format!("{mesh} is not a mesh this can read: {e}"))?;
    let fill_missing = !fill.is_empty();
    let (written, lines) = replace_gta6_mesh_with(&sections, role, entry, &scene, fill_missing)?;
    let saved = write_rpf9_resource(&written).map_err(|e| e.to_string())?;
    // The file is written only once it reads back as the drawable it was made from.
    let (find, bytes) = read_rpf9_body(Cursor::new(&saved), 0, u64::MAX)
        .map_err(|e| format!("the written resource does not read back: {e}"))?;
    let back = Rpf9Sections::of(&find, &bytes).ok_or("the written resource does not read back")?;
    drawable_of(&back, role, entry)?;
    fs::write(out, &saved).map_err(|e| format!("write({out}): {e}"))?;
    println!(
        "{path} at {offset:#X} + {mesh} -> {out} ({} bytes)",
        saved.len()
    );
    for line in lines {
        println!("  {line}");
    }
    for held in &scene.dropped {
        println!("  not carried: {held}");
    }
    println!("This checks the body's bytes. It does not test that GTA VI loads them.");
    Ok(())
}

/// The drawable a body of `role` holds: its own, a fragment's, or the dictionary entry `entry`.
fn drawable_of(s: &Rpf9Sections, role: Rpf9Role, entry: Option<u32>) -> Result<Drawable, String> {
    let (v, p) = (&s.virtual_section, Some(s.physical.as_slice()));
    let read = match (role, entry) {
        (Rpf9Role::Drawable, None) => read_drawable(v, p),
        (Rpf9Role::DrawableWithBound, None) => read_drawable_with_bound(v, p).map(|r| r.drawable),
        (Rpf9Role::Dictionary, Some(code)) => {
            let found = (read_dictionary(v, p).map_err(|e| e.to_string())?.entries)
                .into_iter()
                .find_map(|(c, e)| match e {
                    Ok(Entry::Drawable(d)) if c == code => Some(*d),
                    _ => None,
                });
            return found.ok_or(format!(
                "the dictionary holds no drawable under {code:#010X}"
            ));
        }
        (Rpf9Role::Dictionary, None) => {
            return Err("this is a dictionary: --entry <code> names the drawable".into())
        }
        (_, Some(_)) => return Err("--entry names a dictionary's drawable; this is none".into()),
        _ => return Err("this body is no drawable, fragment or dictionary".into()),
    };
    read.map_err(|e| e.to_string())
}

/// Where a primitive of `mesh`, its `slot`-th, goes back: the address it states, else the model
/// its mesh's name ends in and its place among the mesh's primitives.
fn address(mesh: &Mesh, slot: usize, p: &Primitive) -> Result<GeometryAt, String> {
    let LodSlot::Level(level) = p.lod else {
        return Err(format!(
            "{}: a GTA VI drawable has no container level",
            mesh.name
        ));
    };
    let (model, geometry) = match (p.stated_address, lod_and_model_in_name(&mesh.name)) {
        (Some(stated), _) => stated,
        (None, Some((_, model))) => (model, slot),
        (None, None) => {
            return Err(format!(
                "{}: the mesh says neither which model nor which geometry it replaces",
                mesh.name
            ))
        }
    };
    Ok(GeometryAt {
        level: usize::from(level),
        model,
        geometry,
    })
}

fn label(at: GeometryAt) -> String {
    format!("lod{}/model{}/geometry{}", at.level, at.model, at.geometry)
}

/// The geometry `at` names, where it reads.
fn geometry_at(drawable: &Drawable, at: GeometryAt) -> Option<&Geometry> {
    let models = drawable.levels.get(at.level)?.as_ref()?;
    models
        .get(at.model)?
        .geometries
        .get(at.geometry)?
        .as_ref()
        .ok()
}

/// The geometry `at` names, where its positions read, and its model.
fn located<'a>(
    drawable: &'a Drawable,
    at: GeometryAt,
    named: &str,
) -> Result<(&'a Geometry, &'a Model), String> {
    let g = geometry_at(drawable, at).ok_or(format!(
        "{named}: the drawable holds no geometry there whose positions read"
    ))?;
    let model = (drawable.levels.get(at.level).and_then(Option::as_ref))
        .and_then(|models| models.get(at.model))
        .ok_or(format!("{named}: the drawable holds no model there"))?;
    Ok((g, model))
}

/// The channels of `g` among `range`, in ascending order.
fn declared(g: &Geometry, range: std::ops::RangeInclusive<u8>) -> Vec<u8> {
    (g.channels.iter().map(|c| c.index))
        .filter(|c| range.contains(c))
        .collect()
}

/// Each set of `values` paired with the channel of `g` it goes to: the one `stated` names where
/// `g` declares it, else the set's place among the declared channels; and how many sets had none.
fn paired<T: Clone>(
    values: &[Vec<T>],
    stated: &[u8],
    declared: &[u8],
) -> (Vec<(u8, Vec<T>)>, usize) {
    let mut out: Vec<(u8, Vec<T>)> = Vec::new();
    let mut dropped = 0;
    for (k, set) in values.iter().enumerate() {
        let channel = (stated.get(k).filter(|c| declared.contains(c)))
            .or_else(|| declared.get(k))
            .copied()
            .filter(|c| out.iter().all(|(taken, _)| taken != c));
        match channel {
            Some(channel) => out.push((channel, set.clone())),
            None => dropped += 1,
        }
    }
    (out, dropped)
}

/// What `p` gives geometry `g`, and the texcoord and colour sets it has no channel for.
fn edit_of(g: &Geometry, p: &Primitive) -> (GeometryEdit, usize) {
    let (uvs, no_uv) = paired(&p.uvs, &p.uv_channels, &declared(g, 28..=33));
    let (colours, no_colour) = paired(&p.colours, &p.colour_channels, &declared(g, 24..=26));
    let edit = GeometryEdit {
        positions: p.positions.clone(),
        normals: match g.channels.iter().any(|c| c.index == 4) {
            true => p.normals.clone(),
            false => Vec::new(),
        },
        tangents: match g.vertices.tangents.is_empty() {
            true => Vec::new(),
            false => p.tangents.clone(),
        },
        uvs,
        colours,
        triangles: p.indices.as_chunks::<3>().0.to_vec(),
        ..GeometryEdit::default()
    };
    (edit, no_uv + no_colour)
}

/// `sections` with each geometry a primitive of `scene` names replaced from it, and a line each.
// The drawable is the body's own, a fragment's, or dictionary entry `entry`; `Err` says why the
// import is refused.
pub fn replace_gta6_mesh(
    sections: &Rpf9Sections,
    role: Rpf9Role,
    entry: Option<u32>,
    scene: &Scene,
) -> Result<(Rpf9Sections, Vec<String>), String> {
    replace_gta6_mesh_with(sections, role, entry, scene, false)
}

/// [`replace_gta6_mesh`], with `fill_missing` as `mesh import --fill-missing` takes it.
// A channel a geometry declares and the mesh omits takes its stated default where the vertex
// count changes, each named in the geometry's line.
pub fn replace_gta6_mesh_with(
    sections: &Rpf9Sections,
    role: Rpf9Role,
    entry: Option<u32>,
    scene: &Scene,
    fill_missing: bool,
) -> Result<(Rpf9Sections, Vec<String>), String> {
    let mut drawable = drawable_of(sections, role, entry)?;
    let mut out = sections.clone();
    let mut lines = Vec::new();
    let mut taken = BTreeSet::new();
    // Each geometry cut into several, at its stated place, and the geometries it added.
    let mut cuts: Vec<(GeometryAt, usize)> = Vec::new();
    let (mut before, mut after, mut triangles) = (0, 0, 0);
    // Whether a level-0 geometry of a model that hangs off bones moved, which the writer leaves to
    // the drawable's bind pose.
    let mut posed = false;
    let owner = match (role, entry) {
        (_, Some(code)) => SkeletonOwner::Entry(code),
        (Rpf9Role::DrawableWithBound, None) => SkeletonOwner::Fragment,
        _ => SkeletonOwner::Drawable,
    };
    // Without an own skeleton a joint names its bone by its name's hash.
    let own = read_skeleton_of(&sections.virtual_section, owner)
        .ok()
        .flatten();
    let meshes = (scene.objects.iter()).flat_map(|o| o.meshes.iter().map(move |m| (o, m)));
    for (object, mesh) in meshes {
        for (slot, p) in mesh.primitives.iter().enumerate() {
            let at = address(mesh, slot, p)?;
            let named = label(at);
            if !taken.insert((at.level, at.model, at.geometry)) {
                return Err(format!("{named}: two primitives of the mesh replace it"));
            }
            // A geometry cut earlier in the same model moved the ones after it.
            let stated = at;
            let moved: usize = (cuts.iter())
                .filter(|(c, _)| (c.level, c.model) == (at.level, at.model))
                .filter(|(c, _)| c.geometry < at.geometry)
                .map(|(_, added)| added)
                .sum();
            let at = GeometryAt {
                geometry: at.geometry + moved,
                ..at
            };
            let wearer = skin::Wearer {
                bones: &object.bones,
                own: own.as_ref(),
            };
            let (g, model) = located(&drawable, at, &named)?;
            let add = skin::additions(g, model, p, &wearer).map_err(|e| format!("{named}: {e}"))?;
            if !add.is_empty() {
                write_palette(&mut out, role, entry, at.level, at.model, &add)
                    .map_err(|e| format!("{named}: {e}"))?;
                // Every blend index of the model may have moved with its palette.
                drawable = drawable_of(&out, role, entry)?;
            }
            let (g, model) = located(&drawable, at, &named)?;
            let (mut edit, dropped) = edit_of(g, p);
            let n = p.positions.len();
            let bare = p.joints.len() != n || p.weights.len() != n;
            match skin::fill(&mut edit, g, model, p, &wearer, &add.bones) {
                // A mesh with no skin is refused only at another count, where the fill gives one.
                Err(_) if fill_missing && bare => {}
                done => done.map_err(|e| format!("{named}: {e}"))?,
            }
            let filled = match fill_missing {
                true => fill::defaults(&mut edit, g),
                false => Vec::new(),
            };
            let cut = write_geometry_cut(&mut out, role, entry, at, &edit)
                .map_err(|e| format!("{named}: {e}"))?;
            let written = cut.written;
            posed |= at.level == 0
                && !written.root_reboxed
                && g.positions
                    .as_ref()
                    .is_ok_and(|held| *held != edit.positions);
            before += g.vertex_count as usize;
            after += edit.positions.len();
            triangles += edit.triangles.len();
            let mut said = vec![format!(
                "{named}: {} vertices in, {} out, {} triangles",
                g.vertex_count,
                edit.positions.len(),
                edit.triangles.len()
            )];
            if written.triangles_rebuilt {
                said.push("its meshlets built again".to_string());
            } else if written.bounds_rewritten {
                said.push("its meshlets' bounds written again".to_string());
            }
            if !written.carried.is_empty() {
                let carried: Vec<String> = written.carried.iter().map(u8::to_string).collect();
                said.push(format!(
                    "channels {} kept as it held them",
                    carried.join(", ")
                ));
            }
            if !filled.is_empty() {
                let filled: Vec<String> = filled.iter().map(u8::to_string).collect();
                said.push(format!(
                    "channels {} filled with a stated default the mesh does not give",
                    filled.join(", ")
                ));
            }
            if !add.hashes.is_empty() {
                said.push(format!(
                    "{} bones added to its model's palette",
                    add.hashes.len()
                ));
            }
            if !add.bones.is_empty() {
                said.push(format!(
                    "{} bones set in its model's bone mask",
                    add.bones.len()
                ));
            }
            if !edit.joints.is_empty() {
                said.push(
                    "its weights and blend indices written from the mesh's joints".to_string(),
                );
            } else if !p.joints.is_empty() {
                said.push("its weights and blend indices kept as it held them".to_string());
            }
            if dropped > 0 {
                said.push(format!(
                    "{dropped} texcoord or colour sets it has no channel for left out"
                ));
            }
            if written.root_reboxed {
                said.push("the drawable's box and sphere derived again".to_string());
            }
            if cut.pieces > 1 {
                said.push(format!(
                    "cut into {} geometries of at most 65,536 vertices, each after the first \
                     taking its shader and its density words and a fresh content word at +0x30",
                    cut.pieces
                ));
            }
            lines.push(said.join("; "));
            if cut.pieces > 1 {
                cuts.push((stated, cut.pieces - 1));
                drawable = drawable_of(&out, role, entry)?;
            }
        }
    }
    if taken.is_empty() {
        return Err("the mesh names no geometry of the drawable".into());
    }
    if posed {
        lines.push(match read_skeleton_of(&out.virtual_section, owner) {
            Ok(Some(skeleton)) => {
                rebox_bind_pose(&mut out, role, entry, &skeleton)
                    .map_err(|e| format!("the drawable's box and sphere: {e}"))?;
                "the drawable's box and sphere derived again from its level-0 vertices posed at \
                 bind"
                    .to_string()
            }
            Ok(None) => "the drawable's box and sphere left as read: its models hang off bones \
                         and it has no skeleton of its own to pose them at bind"
                .to_string(),
            Err(e) => {
                return Err(format!(
                    "the drawable's skeleton does not read, so its box and sphere are not \
                     derived from the bind pose: {e}"
                ))
            }
        });
    }
    lines.push(format!(
        "{} geometries replaced: {before} vertices in, {after} out, {triangles} triangles; each \
         geometry's content word at +0x30 kept as read",
        taken.len()
    ));
    Ok((out, lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_interchange::gltf::{self, Container};

    /// `value` little-endian at `at`.
    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn pointer(bytes: &mut [u8], at: usize, tag: u64, target: usize) {
        put(bytes, at, &(tag + target as u64).to_le_bytes());
    }

    fn le(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    /// A drawable of one geometry: [`TRIANGLE`] as three floats a vertex, its normals and its
    /// texcoord 0, and one meshlet; its block map an allocation of its own.
    fn body() -> Rpf9Sections {
        let (virtual_tag, physical_tag) = (0x5000_0000, 0x6000_0000);
        let mut v = vec![0u8; 0x4C0];
        let at = |v: &mut Vec<u8>, from: usize, to: usize| pointer(v, from, virtual_tag, to);
        let (group, shaders, shader, infos) = (0xF0, 0x130, 0x140, 0x180);
        let (level, models, model, geometries) = (0x190, 0x1A0, 0x1B0, 0x1F0);
        let (boxes, indices, geometry, vb) = (0x200, 0x220, 0x230, 0x280);
        let (declaration, record, ib, map) = (0x2D0, 0x410, 0x450, 0x4A0);
        at(&mut v, 0x08, map);
        (v[map + 8], v[map + 9]) = (1, 1);
        at(&mut v, 0x10, group);
        put(&mut v, 0x40, &le(&[1.0, 1.0, 0.0]));
        at(&mut v, 0x50, level);
        put(&mut v, 0x7C, &9998.0f32.to_le_bytes());
        put(&mut v, 0xE0, b"tri#dr\0");
        at(&mut v, 0xA8, 0xE0);
        at(&mut v, group + 0x10, shaders);
        put(&mut v, group + 0x18, &1u16.to_le_bytes());
        at(&mut v, shaders, shader);
        at(&mut v, shader + 0x20, infos);
        at(&mut v, level, models);
        put(&mut v, level + 8, &1u16.to_le_bytes());
        at(&mut v, models, model);
        at(&mut v, model + 0x08, geometries);
        put(&mut v, model + 0x10, &1u16.to_le_bytes());
        at(&mut v, model + 0x18, boxes);
        at(&mut v, model + 0x20, indices);
        put(&mut v, model + 0x36, &1u16.to_le_bytes());
        at(&mut v, geometries, geometry);
        put(&mut v, boxes + 0x10, &le(&[1.0, 1.0, 0.0, 1.0]));
        at(&mut v, geometry + 0x08, vb);
        at(&mut v, geometry + 0x40, record);

        // Positions at 0x60, normals at 0x84 and texcoord 0 at 0xA8, behind the slot table.
        let mut p = vec![0u8; 0xC0];
        let streams = [
            (0usize, 0usize, 0x60usize, 12u8, 6u8, 0x63u32),
            (4, 2, 0x84, 12, 6, 0x63),
        ];
        let streams = streams.into_iter().chain([(28, 3, 0xA8, 8, 0x10, 0x42)]);
        for (channel, slot, offset, size, kind, code) in streams {
            put(
                &mut p,
                4 * slot,
                &((offset as u32 / 2) << 8 | code).to_le_bytes(),
            );
            put(
                &mut v,
                declaration + 4 * channel,
                &(offset as u32).to_le_bytes(),
            );
            v[declaration + 0xD0 + channel] = size;
            v[declaration + 0x104 + channel] = kind;
        }
        for k in (0..52).filter(|k| ![0, 4, 28].contains(k)) {
            put(&mut v, declaration + 4 * k, &0x60u32.to_le_bytes());
        }
        put(
            &mut v,
            declaration + 0x138,
            &(1u64 | 32 << 2 | 3 << 10).to_le_bytes(),
        );
        put(&mut p, 0x5C, &0x200u32.to_le_bytes());
        put(&mut p, 0x60, &le(&TRIANGLE.concat()));
        put(&mut p, 0x84, &le(&[[0.0f32, 0.0, 1.0]; 3].concat()));
        put(&mut p, 0xA8, &le(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]));
        put(&mut v, vb + 0x08, &0x30u32.to_le_bytes());
        put(&mut v, vb + 0x0C, &4u16.to_le_bytes());
        put(&mut v, vb + 0x0E, &9u16.to_le_bytes());
        put(&mut v, vb + 0x10, &0x1601u32.to_le_bytes());
        pointer(&mut v, vb + 0x18, physical_tag, 0);
        at(&mut v, vb + 0x40, declaration);

        // One meshlet of one triangle over a list of three vertices: roundup4(3 + 4 + 1 + 2) words.
        let words: [u32; 12] = [
            0x8000_0001,
            1,
            3,
            0,
            0,
            0,
            0,
            1 << 7 | 2 << 14,
            1 << 16,
            2,
            0,
            0,
        ];
        p.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        at(&mut v, record, ib);
        put(&mut v, record + 0x08, &1u16.to_le_bytes());
        put(&mut v, record + 0x0A, &1u16.to_le_bytes());
        put(&mut v, record + 0x0C, &(3u32 << 8).to_le_bytes());
        put(&mut v, ib + 0x08, &12u32.to_le_bytes());
        put(&mut v, ib + 0x0C, &4u16.to_le_bytes());
        put(&mut v, ib + 0x0E, &8u16.to_le_bytes());
        put(&mut v, ib + 0x10, &0x1380u32.to_le_bytes());
        pointer(&mut v, ib + 0x18, physical_tag, 0xC0);
        Rpf9Sections {
            form: or_io::container::rpf::Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: p,
        }
    }

    /// The scene `rpf scan-export` makes of `s`, written as a `.glb` and read back.
    fn through_gltf(s: &Rpf9Sections) -> Scene {
        let drawable = drawable_of(s, Rpf9Role::Drawable, None).expect("the drawable reads");
        let names = crate::gta6_export::Names {
            effect: &|hash| format!("hash_{hash:08X}"),
            texture: &|key| format!("hash_{key:016X}"),
        };
        let scene = crate::gta6_export::scene(&drawable, None, "tri", false, &names);
        let files = gltf::write(&scene, "tri", Container::Glb);
        gltf::read::load(Path::new("tri.glb"), &files.primary).expect("the .glb reads")
    }

    fn only(s: &Rpf9Sections) -> Geometry {
        let mut d = drawable_of(s, Rpf9Role::Drawable, None).expect("the drawable reads");
        let mut models = d.levels[0].take().expect("level 0");
        models
            .remove(0)
            .geometries
            .remove(0)
            .expect("the geometry reads")
    }

    #[test]
    fn a_gta_vi_drawable_s_own_gltf_puts_back_the_bytes_it_came_from() {
        let sections = body();
        let (same, lines) = replace_gta6_mesh(
            &sections,
            Rpf9Role::Drawable,
            None,
            &through_gltf(&sections),
        )
        .expect("replaced");
        assert_eq!(same, sections);
        assert!(
            lines[0].starts_with("lod0/model0/geometry0: 3 vertices in, 3 out"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_gltf_with_another_vertex_count_saves_as_a_loose_resource_that_reads_back() {
        let sections = body();
        let mut scene = through_gltf(&sections);
        let p = &mut scene.objects[0].meshes[0].primitives[0];
        p.positions.push([1.0, 1.0, 0.0]);
        p.normals.push([0.0, 0.0, 1.0]);
        p.uvs[0].push([1.0, 1.0]);
        p.indices.extend([1, 3, 2]);
        let edited = p.clone();
        let (written, lines) =
            replace_gta6_mesh(&sections, Rpf9Role::Drawable, None, &scene).expect("replaced");
        assert!(
            lines[0].contains("3 vertices in, 4 out, 2 triangles"),
            "{lines:?}"
        );
        assert!(lines[0].contains("meshlets built again"), "{lines:?}");

        let file = write_rpf9_resource(&written).expect("written");
        let (find, bytes) = read_rpf9_body(Cursor::new(&file), 0, u64::MAX).expect("it reads");
        let back = Rpf9Sections::of(&find, &bytes).expect("a resource");
        let g = only(&back);
        assert_eq!(g.vertex_count, 4);
        assert_eq!(g.positions.as_ref().expect("positions"), &edited.positions);
        assert_eq!(g.vertices.normals, edited.normals);
        assert_eq!(g.vertices.uvs, [(28, edited.uvs[0].clone())]);
        let triangles: Vec<[u32; 3]> = edited.indices.as_chunks::<3>().0.to_vec();
        assert_eq!(g.triangles, triangles);
        assert_eq!(write_rpf9_resource(&back).expect("again"), file);
    }

    #[test]
    fn a_channel_the_mesh_omits_at_another_count_is_filled_only_when_asked() {
        let sections = body();
        let mut scene = through_gltf(&sections);
        let p = &mut scene.objects[0].meshes[0].primitives[0];
        p.positions.push([1.0, 1.0, 0.0]);
        p.indices.extend([1, 3, 2]);
        p.normals.clear();
        p.uvs.clear();
        let refused =
            replace_gta6_mesh(&sections, Rpf9Role::Drawable, None, &scene).expect_err("refused");
        assert!(refused.contains("channel 4"), "{refused}");
        let (written, lines) =
            replace_gta6_mesh_with(&sections, Rpf9Role::Drawable, None, &scene, true)
                .expect("filled");
        assert!(lines[0].contains("channels 4, 28 filled"), "{lines:?}");
        let g = only(&written);
        assert_eq!(g.vertex_count, 4);
        assert_eq!(g.vertices.normals, vec![[0.0, 0.0, 1.0]; 4]);
        assert_eq!(g.vertices.uvs, [(28, vec![[0.0, 0.0]; 4])]);

        // At the geometry's own count the fill takes nothing: the channels are kept as held.
        let (same, lines) = replace_gta6_mesh_with(
            &sections,
            Rpf9Role::Drawable,
            None,
            &through_gltf(&sections),
            true,
        )
        .expect("replaced");
        assert_eq!(same, sections);
        assert!(!lines[0].contains("filled"), "{lines:?}");
    }

    #[test]
    fn scan_import_fills_a_mesh_without_normals_only_under_fill_missing() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let sections = body();
        let drawable = drawable_of(&sections, Rpf9Role::Drawable, None).expect("it reads");
        let names = crate::gta6_export::Names {
            effect: &|hash| format!("hash_{hash:08X}"),
            texture: &|key| format!("hash_{key:016X}"),
        };
        let mut scene = crate::gta6_export::scene(&drawable, None, "tri", false, &names);
        let p = &mut scene.objects[0].meshes[0].primitives[0];
        p.positions.push([1.0, 1.0, 0.0]);
        p.uvs[0].push([1.0, 1.0]);
        p.indices.extend([1, 3, 2]);
        p.normals.clear();
        let (input, mesh, out) = (
            dir.path().join("tri.rsc"),
            dir.path().join("tri.glb"),
            dir.path().join("out.rsc"),
        );
        let resource = write_rpf9_resource(&sections).expect("written");
        fs::write(&input, resource).expect("written");
        let glb = gltf::write(&scene, "tri", Container::Glb).primary;
        fs::write(&mesh, glb).expect("written");
        let words = [&input, &mesh, &out].map(|p| p.to_string_lossy().into_owned());
        let mut args = vec![words[0].clone(), "0".to_string(), words[1].clone()];
        args.push(words[2].clone());
        let refused = cmd_scan_import(&args).expect_err("refused");
        assert!(refused.contains("channel 4"), "{refused}");
        assert!(!out.exists(), "nothing is written for a refusal");

        args.push("--fill-missing".to_string());
        cmd_scan_import(&args).expect("filled");
        let saved = fs::read(&out).expect("the written resource");
        let (find, bytes) = read_rpf9_body(Cursor::new(&saved), 0, u64::MAX).expect("it reads");
        let back = Rpf9Sections::of(&find, &bytes).expect("a resource");
        let g = only(&back);
        assert_eq!(g.vertex_count, 4);
        assert_eq!(g.vertices.normals, vec![[0.0, 0.0, 1.0]; 4]);
    }

    #[test]
    fn a_mesh_that_names_no_geometry_or_one_past_the_drawable_is_refused_by_name() {
        let sections = body();
        let mut scene = through_gltf(&sections);
        scene.objects[0].meshes[0].primitives[0].stated_address = Some((0, 1));
        let refused =
            replace_gta6_mesh(&sections, Rpf9Role::Drawable, None, &scene).expect_err("refused");
        assert!(refused.contains("lod0/model0/geometry1"), "{refused}");
        scene.objects[0].meshes[0].primitives.clear();
        let refused =
            replace_gta6_mesh(&sections, Rpf9Role::Drawable, None, &scene).expect_err("refused");
        assert!(refused.contains("names no geometry"), "{refused}");
    }

    #[test]
    fn an_output_that_is_the_input_archive_is_refused_and_leaves_it_whole() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let archive = dir.path().join("game.rpf");
        fs::write(&archive, b"an archive").expect("written");
        let path = archive.to_string_lossy().into_owned();
        let args = [path.as_str(), "0x1010", "mesh.glb", path.as_str()].map(String::from);
        let refused = cmd_scan_import(&args).expect_err("refused");
        assert!(refused.contains("is the input"), "{refused}");
        assert_eq!(fs::read(&archive).expect("read"), b"an archive");
    }

    const QUARTER: f32 = std::f32::consts::FRAC_1_SQRT_2;
    /// An inverse bind's four columns, the identity.
    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    const SKELETON: usize = 0x4C0;

    /// [`body`] with a skeleton of its own, a root and a child turned a quarter about z and
    /// raised 5 m whose inverse bind is the identity, and its model hung off the child.
    fn bone_attached() -> Rpf9Sections {
        let mut s = body();
        let v = &mut s.virtual_section;
        let tag = 0x5000_0000;
        let (records, inverses, names) = (0x540, 0x5E0, 0x660);
        v.resize(0x680, 0);
        pointer(v, 0x18, tag, SKELETON);
        pointer(v, SKELETON + 0x28, tag, records);
        pointer(v, SKELETON + 0x30, tag, inverses);
        put(v, SKELETON + 0x6C, &[2, 0, 2, 0]);
        let bones = [
            ("root", -1i16, [0.0, 0.0, 0.0, 1.0], [0.0; 3]),
            ("hung", 0, [0.0, 0.0, QUARTER, QUARTER], [0.0, 0.0, 5.0]),
        ];
        for (i, (name, parent, rotation, translation)) in bones.into_iter().enumerate() {
            let at = records + 0x50 * i;
            put(v, at, &le(&rotation));
            put(v, at + 0x10, &le(&translation));
            put(v, at + 0x20, &le(&[1.0; 3]));
            let text = names + 0x10 * i;
            put(v, text, name.as_bytes());
            pointer(v, at + 0x30, tag, text);
            put(v, at + 0x46, &parent.to_le_bytes());
            put(v, at + 0x48, &(i as u16).to_le_bytes());
            put(v, inverses + 0x40 * i, &le(&IDENTITY));
        }
        // A palette count makes the model no rigid one; its matrix index hangs it off bone 1.
        let model = 0x1B0;
        (v[model + 0x30], v[model + 0x33]) = (1, 1);
        s
    }

    /// `p` posed on [`bone_attached`]'s bone 1: turned a quarter about z, then raised 5 m.
    fn hung(p: [f32; 3]) -> [f32; 3] {
        [-p[1], p[0], p[2] + 5.0]
    }

    #[test]
    fn a_bone_attached_drawable_s_box_and_sphere_hold_every_vertex_posed_at_bind() {
        let sections = bone_attached();
        let mut scene = through_gltf(&sections);
        let p = &mut scene.objects[0].meshes[0].primitives[0];
        p.positions.push([1.0, 1.0, 0.0]);
        p.normals.push([0.0, 0.0, 1.0]);
        p.uvs[0].push([1.0, 1.0]);
        p.indices.extend([1, 3, 2]);
        let positions = p.positions.clone();
        let (written, lines) =
            replace_gta6_mesh(&sections, Rpf9Role::Drawable, None, &scene).expect("replaced");
        assert!(
            lines.iter().any(|l| l.contains("posed at bind")),
            "{lines:?}"
        );
        let d = drawable_of(&written, Rpf9Role::Drawable, None).expect("it reads");
        let v = &written.virtual_section;
        let sphere: Vec<f32> = (0..4)
            .map(|k| f32::from_le_bytes(v[0x20 + 4 * k..0x24 + 4 * k].try_into().expect("4")))
            .collect();
        let eps = 1e-4;
        for q in positions.iter().map(|p| hung(*p)) {
            let inside = (0..3).all(|a| d.box_min[a] - eps <= q[a] && q[a] <= d.box_max[a] + eps);
            assert!(inside, "{q:?} outside {:?} {:?}", d.box_min, d.box_max);
            let off: f32 = (0..3)
                .map(|a| (q[a] - sphere[a]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(
                off <= sphere[3] + eps,
                "{q:?} outside the sphere {sphere:?}"
            );
        }
        // The vertices as stored lie below the box: it is the bind pose's, not theirs.
        assert!(d.box_min[2] > 4.0, "{:?}", d.box_min);

        // A skeleton that does not read is refused by name, and nothing is written.
        let mut broken = sections.clone();
        put(
            &mut broken.virtual_section,
            SKELETON + 0x6E,
            &3u16.to_le_bytes(),
        );
        let refused =
            replace_gta6_mesh(&broken, Rpf9Role::Drawable, None, &scene).expect_err("refused");
        assert!(refused.contains("skeleton does not read"), "{refused}");
    }

    #[test]
    fn a_skinned_vertex_is_posed_through_world_times_inverse_bind_weighted_over_its_bones() {
        use or_io::gta6::hash::name_hash32;
        use or_io::gta6::headerless::{bind_positions, Bone, Channel, Palette, Skeleton, Vertices};
        let bone = |name: &str, parent, rotation, translation, inverse: [[f32; 3]; 4]| Bone {
            name: name.to_string(),
            hash: name_hash32(name.as_bytes()),
            parent,
            rotation,
            translation,
            scale: [1.0; 3],
            inverse_bind: inverse,
        };
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]];
        let mut shifted = identity;
        shifted[3] = [-1.0, 0.0, 0.0];
        // Bone 1 turns a quarter about z at 2 m along x; its inverse bind moves 1 m back.
        let skeleton = Skeleton {
            bones: vec![
                bone("root", None, [0.0, 0.0, 0.0, 1.0], [0.0; 3], identity),
                bone(
                    "arm",
                    Some(0),
                    [0.0, 0.0, QUARTER, QUARTER],
                    [2.0, 0.0, 0.0],
                    shifted,
                ),
            ],
        };
        let channel = |index, code, size| Channel {
            index,
            offset: 0,
            size,
            code,
        };
        let g = Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 0,
            vertex_count: 2,
            channels: vec![
                channel(0, 6, 12),
                channel(16, 0x1C, 4),
                channel(20, 0x1E, 4),
            ],
            triangles: Vec::new(),
            positions: Ok(vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
            vertices: Vertices {
                weights: vec![(16, vec![[0.25, 0.75, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]])],
                joints: vec![(20, vec![[0, 1, 0, 0], [0, 1, 0, 0]])],
                ..Vertices::default()
            },
        };
        let hashes = vec![name_hash32(b"root"), name_hash32(b"arm")];
        let model = Model {
            geometries: Vec::new(),
            palette: Ok(Palette::Hashes(hashes)),
            matrix_index: 0,
        };
        let posed = bind_positions(&g, &model, &skeleton).expect("posed");
        // World times inverse bind takes (1, 0, 0) to (2, 0, 0) on bone 1, and (0, 1, 0) to
        // (1, -1, 0); inverse bind times world would give (1, 1, 0) and (0, 0, 0). Vertex 0 is a
        // quarter on bone 0, which leaves it at (1, 0, 0), and three quarters on bone 1.
        let expected = [[0.25 + 0.75 * 2.0, 0.0, 0.0], [1.0, -1.0, 0.0]];
        for (q, e) in posed.iter().zip(expected) {
            assert!((0..3).all(|a| (q[a] - e[a]).abs() < 1e-5), "{posed:?}");
        }

        // A palette entry the skeleton does not hold leaves the bind pose unknown, said by name.
        let model = Model {
            palette: Ok(Palette::Hashes(vec![
                name_hash32(b"root"),
                name_hash32(b"leg"),
            ])),
            ..model
        };
        let refused = bind_positions(&g, &model, &skeleton).expect_err("refused");
        assert!(refused.to_string().contains("does not hold"), "{refused}");
    }
}
