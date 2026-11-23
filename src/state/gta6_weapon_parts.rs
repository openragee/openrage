//! A GTA VI weapon's parts: the bodies of its archive whose names extend the weapon's, each with
//! its root bone `AAP_<point>` on the bone `WAP_<point>` of the weapon or of a part placed before.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use glam::Mat4;
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Role, Rpf9ZlibRun};
use or_io::gta6::headerless::{self, Resource, Skeleton};
use or_io::gta6::names::Labels;
use or_io::gta6::texture_names::stem;

/// The most of a part that is read: no model of a weapon nears it.
const BODY_LIMIT: u64 = 256 << 20;
/// The tokens a name ends in at a level of detail above the lowest, which has none.
const LEVELS: [&str; 2] = ["_hi", "_mid"];
/// What starts the name of a bone a part is placed on, and of the root bone of a part.
const POINT: &str = "WAP_";
const ROOT: &str = "AAP_";

/// A part as read, with the matrix that takes its vertices to the weapon's frame.
pub(crate) struct Part {
    pub place: Mat4,
    pub resource: Resource,
}

static PARTS: Mutex<BTreeMap<PathBuf, Arc<Vec<Part>>>> = Mutex::new(BTreeMap::new());

fn held() -> MutexGuard<'static, BTreeMap<PathBuf, Arc<Vec<Part>>>> {
    PARTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The parts kept for the weapon the scanned body at `path` is; `None` for any other body.
pub(crate) fn of(path: &Path) -> Option<Arc<Vec<Part>>> {
    held().get(path).cloned()
}

/// Read and keep the parts of `read`, the body at `path` of `archive`, where its skeleton offers a
/// point and the archive's names are kept; a body opened again keeps what this read finds.
pub(super) fn keep(path: &Path, archive: &Path, read: &Resource) {
    let parts = super::rpf9_scan::scanned_skeleton(path)
        .ok()
        .zip(super::gta6_resource::drawable_of(read, None))
        .filter(|(skeleton, _)| !points(skeleton, Mat4::IDENTITY).is_empty())
        .map(|(skeleton, drawable)| {
            let labels = super::gta6_names::labels(archive);
            let Ok(file) = File::open(archive) else {
                return Vec::new();
            };
            let parts = (first_variants(&labels, &drawable.name).into_iter())
                .filter_map(|offset| read_part(&file, offset))
                .collect();
            (placed(&skeleton, parts).into_iter())
                .map(|(resource, place)| Part { place, resource })
                .collect()
        })
        .unwrap_or_default();
    match parts.is_empty() {
        true => held().remove(path),
        false => held().insert(path.to_path_buf(), Arc::new(parts)),
    };
}

/// `stem` without its level-of-detail token, and the token.
fn level_of(stem: &str) -> (&str, &str) {
    (LEVELS.iter())
        .find_map(|level| stem.strip_suffix(level).map(|base| (base, *level)))
        .unwrap_or((stem, ""))
}

/// The offset of the first variant of each kind of part of the weapon named `own`, at its level of
/// detail: a part is named as the weapon, then `_`, a kind and a number.
fn first_variants(labels: &Labels, own: &str) -> Vec<u64> {
    let own = stem(own);
    let (weapon, level) = level_of(&own);
    let mut first: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for (offset, name) in labels {
        let name = stem(name);
        let (base, its_level) = level_of(&name);
        let rest = (base.strip_prefix(weapon)).and_then(|rest| rest.strip_prefix('_'));
        let Some(rest) = rest.filter(|_| its_level == level) else {
            continue;
        };
        let kind = rest.trim_end_matches(|c: char| c.is_ascii_digit());
        // Without a number it is no variant, and without a kind it is another weapon.
        let Some(kind) = kind.strip_suffix('_').filter(|_| kind.len() < rest.len()) else {
            continue;
        };
        if first.get(kind).is_none_or(|(held, _)| name < *held) {
            first.insert(kind.to_string(), (name.clone(), *offset));
        }
    }
    first.into_values().map(|(_, offset)| offset).collect()
}

/// The drawable or fragment at `offset` of `file` where it and its skeleton read.
fn read_part(file: &File, offset: u64) -> Option<(Skeleton, Resource)> {
    let (find, bytes) = read_rpf9_body(file, offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        signature: Some(signature),
        ..
    }) = find.kind
    else {
        return None;
    };
    let fragment = match signature.role()? {
        Rpf9Role::Drawable => false,
        Rpf9Role::DrawableWithBound => true,
        _ => return None,
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let skeleton = headerless::read_skeleton(virtual_section, fragment).ok()??;
    let read = headerless::read(signature, virtual_section, Some(physical)).ok()?;
    Some((skeleton, read))
}

/// `name` without `prefix`, case folded, where it starts with it.
fn point(name: &str, prefix: &str) -> Option<String> {
    let (head, rest) = name.split_at_checked(prefix.len())?;
    (head.eq_ignore_ascii_case(prefix)).then(|| rest.to_ascii_lowercase())
}

/// Each point `skeleton` offers once placed by `place`, with the matrix of its bone at bind.
fn points(skeleton: &Skeleton, place: Mat4) -> Vec<(String, Mat4)> {
    let world = or_anim::gta6::skeleton(skeleton).bind_world;
    (skeleton.bones.iter().zip(world))
        .filter_map(|(bone, world)| Some((point(&bone.name, POINT)?, place * world)))
        .collect()
}

/// Each of `parts` whose root bone names a point the weapon or a part placed before it offers,
/// with the matrix that takes its vertices there; a point takes the first part that names it.
fn placed<T>(weapon: &Skeleton, parts: Vec<(Skeleton, T)>) -> Vec<(T, Mat4)> {
    let mut offered = points(weapon, Mat4::IDENTITY);
    let mut waiting: Vec<Option<(Skeleton, T)>> = parts.into_iter().map(Some).collect();
    let mut out = Vec::new();
    let mut next = 0;
    while let Some((name, at)) = offered.get(next).cloned() {
        next += 1;
        let root = |s: &Skeleton| s.bones.first().and_then(|bone| point(&bone.name, ROOT));
        let fits = (waiting.iter_mut())
            .find(|part| (part.as_ref()).is_some_and(|(s, _)| root(s).as_ref() == Some(&name)));
        if let Some((skeleton, part)) = fits.and_then(Option::take) {
            let bind = or_anim::gta6::skeleton(&skeleton).bind_world;
            let place = at * bind.first().map_or(Mat4::IDENTITY, Mat4::inverse);
            offered.extend(points(&skeleton, place));
            out.push((part, place));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use or_io::gta6::headerless::Bone;

    fn bone(name: &str, parent: Option<u16>, rotation: Quat, translation: Vec3) -> Bone {
        Bone {
            name: name.to_string(),
            hash: or_io::gta6::hash::name_hash32(name.as_bytes()),
            parent,
            rotation: rotation.to_array(),
            translation: translation.to_array(),
            scale: [1.0; 3],
            inverse_bind: [[0.0; 3]; 4],
        }
    }

    #[test]
    fn the_first_variant_of_each_kind_is_taken_at_the_weapon_s_own_level_of_detail() {
        let labels: Labels = [
            (0x10, "w_xx_test_01_hi.#dr"),
            (0x20, "w_xx_test_01_barrel_02_hi.#dr"),
            (0x30, "w_xx_test_01_barrel_01_hi.#dr"),
            (0x40, "w_xx_test_01_barrel_01.#dr"),
            (0x50, "w_xx_test_01_barrel_01_mid.#dr"),
            (0x60, "W_XX_Test_01_Sight_R_03_hi"),
            (0x70, "w_xx_test_010_stock_01_hi"),
            (0x80, "w_xx_test_01_02_hi.#dr"),
            (0x90, "w_xx_test_01_stock_hi"),
        ]
        .into_iter()
        .map(|(offset, name)| (offset, name.to_string()))
        .collect();
        // The barrel's first variant and the lone sight, a fragment named in another case.
        assert_eq!(first_variants(&labels, "w_xx_test_01_hi.#dr"), [0x30, 0x60]);
        assert_eq!(first_variants(&labels, "w_xx_test_01.#dr"), [0x40]);
        assert_eq!(first_variants(&labels, "w_xx_test_01_mid.#dr"), [0x50]);
        // A part is no weapon: no name extends it.
        assert!(first_variants(&labels, "w_xx_test_01_barrel_01_hi.#dr").is_empty());
    }

    #[test]
    fn a_part_sits_with_its_root_bone_on_the_point_of_its_name_and_offers_its_own_points() {
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let weapon = Skeleton {
            bones: vec![
                bone("Root", None, Quat::IDENTITY, Vec3::ZERO),
                bone("WAP_Body", Some(0), quarter, Vec3::new(1.0, 0.0, 0.0)),
            ],
        };
        let body = Skeleton {
            bones: vec![
                bone("AAP_Body", None, Quat::IDENTITY, Vec3::ZERO),
                bone("WAP_Tip", Some(0), Quat::IDENTITY, Vec3::new(0.0, 2.0, 0.0)),
            ],
        };
        let part = |root: &str| Skeleton {
            bones: vec![bone(root, None, Quat::IDENTITY, Vec3::ZERO)],
        };
        let parts = vec![
            (part("AAP_None"), "no point"),
            (part("aap_tip"), "tip"),
            (part("AAP_Tip"), "second tip"),
            (body, "body"),
        ];
        let placed = placed(&weapon, parts);
        let names: Vec<&str> = placed.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["body", "tip"]);
        // The quarter turn takes the body's +y to the weapon's -x.
        let at = |place: &Mat4| place.transform_point3(Vec3::ZERO);
        assert!(at(&placed[0].1).distance(Vec3::new(1.0, 0.0, 0.0)) < 1e-5);
        assert!(
            at(&placed[1].1).distance(Vec3::new(-1.0, 0.0, 0.0)) < 1e-5,
            "{}",
            at(&placed[1].1)
        );
    }
}
