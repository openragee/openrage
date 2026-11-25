//! What the inspector states of the part of a scanned GTA VI drawable its Outline selects: a model
//! or a geometry of a level, its own or a dictionary entry's, or a bone of its skeleton.

use glam::{Mat4, Vec4};
use or_io::gta6::headerless::{Drawable, Geometry, Model, Palette, Skeleton};

use super::gta6_resource::drawable_of;
use super::{DocPayload, Document};
use crate::ui::panels::selection_ids::{
    dict_entry_unpack_part, is_dict_entry_selection, payload_index, section_of,
    unpack_lod_sub_index, unpack_selection, DOC_ROOT_SEL, SECT_LOD, SECT_SKEL, YPT_MODEL_KIND_LOD,
};

/// A heading, and the rows of the grid under it.
pub(crate) type Rows = (String, Vec<(String, String)>);

/// A level, a model of it, and a geometry of the model where one is named.
pub(crate) type Part = (usize, usize, Option<usize>);

/// The part `selection` names in a GTA VI drawable's Outline, its own rows or a dictionary entry's.
pub(crate) fn part_of(selection: u64) -> Option<Part> {
    if selection == DOC_ROOT_SEL {
        return None;
    }
    if is_dict_entry_selection(selection) {
        let (_, kind, sub) = dict_entry_unpack_part(selection);
        if kind != YPT_MODEL_KIND_LOD {
            return None;
        }
        let (lod, model, geometry) = unpack_lod_sub_index(sub);
        return Some((lod.into(), model.into(), geometry.map(usize::from)));
    }
    if section_of(selection) != SECT_LOD {
        return None;
    }
    let (lod, model, geometry) = unpack_selection(selection);
    Some((lod.into(), model.into(), geometry.map(|g| g as usize)))
}

/// The bone `selection` names in the Outline's skeleton of a GTA VI drawable or fragment.
pub(crate) fn bone_of(selection: u64) -> Option<usize> {
    (selection != DOC_ROOT_SEL && section_of(selection) == SECT_SKEL)
        .then(|| payload_index(selection) as usize)
}

/// What the status bar says of the part `selection` names in the drawable named `name`.
pub(crate) fn summary(name: &str, selection: u64) -> Option<String> {
    let entry_bone = super::gta6_entry_skeletons::entry_bone_of(selection).map(|(_, b)| b);
    if let Some(bone) = bone_of(selection).or(entry_bone) {
        return Some(tr!("{name}: bone {bone}", name = name, bone = bone));
    }
    Some(match part_of(selection)? {
        (level, model, Some(geometry)) => tr!(
            "{name}: geometry {geometry} of model {model}, level {level}",
            name = name,
            geometry = geometry,
            model = model,
            level = level,
        ),
        (level, model, None) => tr!(
            "{name}: model {model} of level {level}",
            name = name,
            model = model,
            level = level,
        ),
    })
}

/// What `doc`, a scanned GTA VI body, states of the part `selection` names; `None` where it names
/// none, or one the body does not hold.
pub(crate) fn selected_rows(doc: &Document, selection: u64) -> Option<Rows> {
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    if let Some(bone) = bone_of(selection) {
        let skeleton = super::rpf9_scan::scanned_skeleton(doc.path.as_deref()?).ok()?;
        return bone_rows(&skeleton, bone);
    }
    if let Some((_, bone)) = super::gta6_entry_skeletons::entry_bone_of(selection) {
        let skeleton = super::gta6_entry_skeletons::shown(doc, Some(selection))?;
        return bone_rows(&skeleton, bone);
    }
    let part = part_of(selection)?;
    part_rows(drawable_of(resource, Some(selection))?, part)
}

/// A model's counts and what its blend indices index, or a geometry's facts as read.
pub(crate) fn part_rows(drawable: &Drawable, (level, model, geometry): Part) -> Option<Rows> {
    let held = drawable.levels.get(level)?.as_ref()?.get(model)?;
    let Some(g) = geometry else {
        let heading = tr!("Level {level}, model {model}", level = level, model = model);
        return Some((heading, model_rows(held)));
    };
    let heading = tr!(
        "Level {level}, model {model}, geometry {geometry}",
        level = level,
        model = model,
        geometry = g,
    );
    let rows = match held.geometries.get(g)? {
        Ok(read) => geometry_rows(drawable, read),
        Err(why) => vec![(tr!("Not read").to_string(), why.to_string())],
    };
    Some((heading, rows))
}

fn model_rows(model: &Model) -> Vec<(String, String)> {
    let read = || model.geometries.iter().flatten();
    let vertices: u64 = read().map(|g| u64::from(g.vertex_count)).sum();
    let triangles: usize = read().map(|g| g.triangles.len()).sum();
    let palette = match &model.palette {
        Ok(Palette::Hashes(hashes)) => tr_n!(
            "{count} bone of the wearer's skeleton, by its name's hash",
            "{count} bones of the wearer's skeleton, by their names' hashes",
            hashes.len()
        ),
        Ok(Palette::Own) => tr!("The drawable's own skeleton").to_string(),
        Err(why) => why.to_string(),
    };
    vec![
        (
            tr!("Geometries").to_string(),
            model.geometries.len().to_string(),
        ),
        (tr!("Vertices").to_string(), vertices.to_string()),
        (tr!("Triangles").to_string(), triangles.to_string()),
        (tr!("Blend indices index").to_string(), palette),
        (
            tr!("Bone of a geometry without blend indices").to_string(),
            model.matrix_index.to_string(),
        ),
    ]
}

fn geometry_rows(drawable: &Drawable, g: &Geometry) -> Vec<(String, String)> {
    let shader = match drawable.shaders.get(usize::from(g.shader)) {
        Some(s) => format!("{} (hash_{:08X})", g.shader, s.name_hash),
        None => g.shader.to_string(),
    };
    let channels: Vec<String> = (g.channels.iter())
        .map(|c| format!("{} ({:#04X}, {})", c.index, c.code, c.size))
        .collect();
    let mut rows = vec![
        (tr!("Shader").to_string(), shader),
        (tr!("Vertices").to_string(), g.vertex_count.to_string()),
        (tr!("Triangles").to_string(), g.triangles.len().to_string()),
        (
            tr!("Channels (index, DXGI format, bytes)").to_string(),
            channels.join(", "),
        ),
    ];
    rows.push(match &g.positions {
        Ok(positions) => (tr!("Box").to_string(), box_of(positions)),
        Err(why) => (tr!("Positions not read").to_string(), why.to_string()),
    });
    if !g.vertices.skipped.is_empty() {
        let skipped: Vec<String> = (g.vertices.skipped.iter())
            .map(|(channel, why)| format!("{channel}: {why}"))
            .collect();
        rows.push((tr!("Channels not read").to_string(), skipped.join("; ")));
    }
    rows
}

/// The least and the greatest corner of `positions`, or a dash for none.
fn box_of(positions: &[[f32; 3]]) -> String {
    let Some(&first) = positions.first() else {
        return "-".to_string();
    };
    let first = glam::Vec3::from(first);
    let (min, max) = (positions.iter().map(|p| glam::Vec3::from(*p)))
        .fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
    format!("{} - {}", point(min.to_array()), point(max.to_array()))
}

fn point([x, y, z]: [f32; 3]) -> String {
    format!("({x:.4}, {y:.4}, {z:.4})")
}

/// A bone's name, hash, parent and bind transform, and where its inverse bind places it.
pub(crate) fn bone_rows(skeleton: &Skeleton, index: usize) -> Option<Rows> {
    let bone = skeleton.bones.get(index)?;
    let parent = match bone.parent {
        Some(p) => match skeleton.bones.get(usize::from(p)) {
            Some(parent) => format!("{} ({p})", parent.name),
            None => p.to_string(),
        },
        None => tr!("none").to_string(),
    };
    let [x, y, z, w] = bone.rotation;
    let rotation = degrees(glam::Quat::from_xyzw(x, y, z, w));
    let [c0, c1, c2, c3] = bone.inverse_bind;
    let column = |[x, y, z]: [f32; 3], w| Vec4::new(x, y, z, w);
    let inverse = Mat4::from_cols(
        column(c0, 0.0),
        column(c1, 0.0),
        column(c2, 0.0),
        column(c3, 1.0),
    );
    let mut rows = vec![
        (tr!("Name").to_string(), bone.name.clone()),
        (tr!("Hash").to_string(), format!("{:#010X}", bone.hash)),
        (tr!("Parent").to_string(), parent),
        (tr!("Position").to_string(), point(bone.translation)),
        (tr!("Rotation (°, XYZ)").to_string(), point(rotation)),
        (tr!("Scale").to_string(), point(bone.scale)),
    ];
    // ⚠ A singular inverse bind places the bone nowhere: its world is not said.
    if inverse.determinant().abs() > f32::EPSILON {
        let (_, turn, at) = inverse.inverse().to_scale_rotation_translation();
        rows.push((tr!("World position").to_string(), point(at.to_array())));
        rows.push((
            tr!("World rotation (°, XYZ)").to_string(),
            point(degrees(turn)),
        ));
    }
    let heading = tr!("Bone {index}", index = index);
    Some((heading, rows))
}

/// `q` as the bone inspector states a rotation: degrees about X, then Y, then Z.
fn degrees(q: glam::Quat) -> [f32; 3] {
    let held = or_io::gta5::Vec4V {
        x: q.x,
        y: q.y,
        z: q.z,
        w: q.w,
        ..Default::default()
    };
    super::bone_numbers::euler_degrees(&held)
}

#[cfg(test)]
mod tests {
    use or_io::gta6::headerless::{Bone, Channel, Shader, Vertices};

    use super::*;
    use crate::ui::panels::selection_ids::{
        dict_entry_id, dict_entry_part_id, pack_index, pack_lod_sub_index, pack_selection,
        SECT_SHADER,
    };

    fn value<'a>(rows: &'a [(String, String)], label: &str) -> Option<&'a str> {
        rows.iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_part_is_named_by_the_outlines_own_rows_and_by_a_dictionary_entrys() {
        assert_eq!(
            part_of(pack_selection(2, 5, Some(7))),
            Some((2, 5, Some(7)))
        );
        assert_eq!(part_of(pack_selection(1, 3, None)), Some((1, 3, None)));
        let sub = pack_lod_sub_index(3, 4, Some(6));
        let entry_part = dict_entry_part_id(0xABCD_0123, YPT_MODEL_KIND_LOD, sub);
        assert_eq!(part_of(entry_part), Some((3, 4, Some(6))));
        for none in [
            DOC_ROOT_SEL,
            dict_entry_id(0xABCD_0123),
            pack_index(SECT_SHADER, 2),
            pack_index(SECT_SKEL, 9),
        ] {
            assert_eq!(part_of(none), None, "{none:#X}");
        }
        assert_eq!(bone_of(pack_index(SECT_SKEL, 9)), Some(9));
        assert_eq!(bone_of(pack_selection(0, 0, None)), None);
        assert_eq!(bone_of(DOC_ROOT_SEL), None);
    }

    fn geometry(positions: Vec<[f32; 3]>, triangles: usize) -> Geometry {
        Geometry {
            vertex_data: Vec::new(),
            index_data: Vec::new(),
            shader: 1,
            vertex_count: positions.len() as u32,
            channels: vec![Channel {
                index: 4,
                offset: 0x30,
                size: 4,
                code: 0x19,
            }],
            triangles: vec![[0, 1, 2]; triangles],
            positions: Ok(positions),
            vertices: Vertices {
                skipped: vec![(33, "lanes whose meaning is not known")],
                ..Vertices::default()
            },
        }
    }

    #[test]
    fn a_model_and_a_geometry_state_what_the_reader_read_of_them() {
        let first = geometry(
            vec![[1.0, -2.0, 3.0], [-4.0, 5.0, 0.5], [0.0, 0.0, -6.0]],
            1,
        );
        let second = geometry(vec![[0.0; 3]; 5], 4);
        let drawable = Drawable {
            name: "d".to_string(),
            box_min: [0.0; 3],
            box_max: [1.0; 3],
            shaders: vec![
                Shader {
                    name_hash: 0x1111_2222,
                    texture_keys: Vec::new(),
                    parameters: Vec::new(),
                },
                Shader {
                    name_hash: 0x3333_4444,
                    texture_keys: Vec::new(),
                    parameters: Vec::new(),
                },
            ],
            levels: [
                None,
                Some(vec![Model {
                    geometries: vec![Ok(first), Ok(second)],
                    palette: Ok(Palette::Own),
                    matrix_index: 7,
                }]),
                None,
                None,
            ],
            lod_distances: [0.0; 4],
            bound: None,
            embedded: None,
            extra_skeletons: Vec::new(),
        };
        let (_, rows) = part_rows(&drawable, (1, 0, None)).expect("the model");
        assert_eq!(value(&rows, tr!("Geometries")), Some("2"));
        assert_eq!(value(&rows, tr!("Vertices")), Some("8"));
        assert_eq!(value(&rows, tr!("Triangles")), Some("5"));
        assert_eq!(
            value(&rows, tr!("Bone of a geometry without blend indices")),
            Some("7")
        );

        let (_, rows) = part_rows(&drawable, (1, 0, Some(0))).expect("the geometry");
        assert_eq!(value(&rows, tr!("Vertices")), Some("3"));
        assert_eq!(value(&rows, tr!("Triangles")), Some("1"));
        assert_eq!(value(&rows, tr!("Shader")), Some("1 (hash_33334444)"));
        assert_eq!(
            value(&rows, tr!("Box")),
            Some(format!("{} - {}", point([-4.0, -2.0, -6.0]), point([1.0, 5.0, 3.0])).as_str())
        );
        assert_eq!(
            value(&rows, tr!("Channels not read")),
            Some("33: lanes whose meaning is not known")
        );
        assert!(
            part_rows(&drawable, (0, 0, None)).is_none(),
            "an empty slot"
        );
        assert!(
            part_rows(&drawable, (1, 0, Some(2))).is_none(),
            "past the geometries"
        );
    }

    #[test]
    fn a_bone_states_its_name_hash_parent_and_where_its_inverse_bind_places_it() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]];
        let mut moved = identity;
        moved[3] = [-1.5, 2.0, -0.25];
        let bone = |name: &str, hash, parent, inverse_bind| Bone {
            name: name.to_string(),
            hash,
            parent,
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [0.5, 0.75, -1.0],
            scale: [1.0, 2.0, 3.0],
            inverse_bind,
        };
        let skeleton = Skeleton {
            bones: vec![
                bone("root", 0x0102_0304, None, identity),
                bone("SKEL_Spine0", 0xA0B0_C0D0, Some(0), moved),
            ],
        };
        let (_, rows) = bone_rows(&skeleton, 1).expect("the bone");
        assert_eq!(value(&rows, tr!("Name")), Some("SKEL_Spine0"));
        assert_eq!(value(&rows, tr!("Hash")), Some("0xA0B0C0D0"));
        assert_eq!(value(&rows, tr!("Parent")), Some("root (0)"));
        assert_eq!(
            value(&rows, tr!("Scale")),
            Some(point([1.0, 2.0, 3.0]).as_str())
        );
        assert_eq!(
            value(&rows, tr!("World position")),
            Some(point([1.5, -2.0, 0.25]).as_str()),
            "the bind world is the inverse of the inverse bind"
        );
        let (_, rows) = bone_rows(&skeleton, 0).expect("the root");
        assert_eq!(value(&rows, tr!("Parent")), Some(tr!("none")));
        assert!(bone_rows(&skeleton, 2).is_none());
    }
}
