//! Editing what a map set places and declares, in the source: an entity moved, turned, scaled, added or taken out, an archetype added
//! or taken out, and the extents grown to hold what was placed.
//!
//! The file a metadata source cooks to is opened as the typed root of the world family (`or_io::meta::typed`), the change is made to the
//! value, and the result is written back as the source's XML (`or_source::metadata::replace`), so a change is one write of one file and
//! the source cooks to the file the value saves as. A value is refused at the save, by the typed root, when it would break an index
//! the engine trusts, and the source is then as it was.
//!
//! ⚠ **An element is added by copying one the file holds, or built from nothing where the file can hold one that is.** A copy
//! keeps every member of the record it copies ([`add_entity`], [`add_archetype`]). An entity or an archetype that is no copy
//! ([`add_new_entity`], [`add_new_archetype`]) states the members the file's own class states, each one a person does not choose
//! at its empty value; a file that states no such class takes it as `or-io` declares it when it is a PSO, and is refused when it
//! is an RBM file, whose class would carry a signature word that nothing derives. No schema snapshot is read either way.

use std::path::{Path, PathBuf};

use glam::{EulerRot, Quat, Vec3};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CBaseArchetypeDefAny, CEntityDefAny, CMapData, CMapTypes};
use or_world::Placement;

/// What an edit changed in the XML of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    /// The XML file of the source.
    pub file: PathBuf,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

impl Edited {
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
}

/// Why the edit of the source in `dir` could not be made.
fn source_error(problems: Vec<or_source::Problem>) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The file the source in `dir` cooks to.
fn cooked(dir: &Path) -> Result<Vec<u8>, String> {
    crate::facts::cooked(dir)
}

/// Write `bytes` as the file the source in `dir` cooks to.
fn write_back(dir: &Path, bytes: &[u8]) -> Result<Edited, String> {
    let replaced = or_source::metadata::replace(dir, bytes).map_err(source_error)?;
    Ok(Edited {
        file: replaced.file,
        before: replaced.before,
        after: replaced.after,
    })
}

/// Open the map file the source in `dir` cooks to, let `change` edit it, and write the result back as the XML of the source.
pub fn edit_map(
    dir: &Path,
    change: impl FnOnce(&mut CMapData) -> Result<(), String>,
) -> Result<Edited, String> {
    let mut root = CMapData::from_bytes(&cooked(dir)?).map_err(|e| e.to_string())?;
    change(&mut root)?;
    write_back(dir, &root.to_bytes().map_err(|e| e.to_string())?)
}

/// Open the type file the source in `dir` cooks to, let `change` edit it, and write the result back as the XML of the source.
pub fn edit_types(
    dir: &Path,
    change: impl FnOnce(&mut CMapTypes) -> Result<(), String>,
) -> Result<Edited, String> {
    let mut root = CMapTypes::from_bytes(&cooked(dir)?).map_err(|e| e.to_string())?;
    change(&mut root)?;
    write_back(dir, &root.to_bytes().map_err(|e| e.to_string())?)
}

/// Where an entity stands: its position, its rotation as the rotation that places it, and its scale.
///
/// The file stores a rotation conjugated for an entity and as it is for an interior instance (`or_world::Placement`), and one number
/// of scale for the two horizontal axes (`scaleXY`) beside one for the vertical (`scaleZ`), so a pose is what a person means and
/// [`set_pose`] stores it the way the file does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
    /// The scale of X and Y together.
    pub scale_xy: f32,
    pub scale_z: f32,
}

impl Pose {
    /// The rotation as degrees about X, Y and Z, which `from_degrees` turns back into the same rotation.
    pub fn degrees(&self) -> [f32; 3] {
        let (x, y, z) = self.rotation.to_euler(EulerRot::XYZ);
        [x.to_degrees(), y.to_degrees(), z.to_degrees()]
    }

    /// The rotation that is `degrees` about X, then Y, then Z.
    pub fn rotation_of_degrees(degrees: [f32; 3]) -> Quat {
        Quat::from_euler(
            EulerRot::XYZ,
            degrees[0].to_radians(),
            degrees[1].to_radians(),
            degrees[2].to_radians(),
        )
        .normalize()
    }
}

/// How the file stores the rotation of this entity.
fn placement_of(entity: &CEntityDefAny) -> Placement {
    match entity.as_mlo_instance() {
        Some(_) => Placement::InteriorInstance,
        None => Placement::Entity,
    }
}

/// The pose of `entity`.
pub fn pose_of(entity: &CEntityDefAny) -> Pose {
    let r = entity.rotation;
    Pose {
        position: Vec3::new(entity.position.x, entity.position.y, entity.position.z),
        rotation: placement_of(entity).rotation_of([r.x, r.y, r.z, r.w]),
        scale_xy: entity.scale_xy,
        scale_z: entity.scale_z,
    }
}

/// Stand `entity` at `pose`.
pub fn set_pose(entity: &mut CEntityDefAny, pose: Pose) {
    let stored = placement_of(entity).store(pose.rotation);
    entity.position.x = pose.position.x;
    entity.position.y = pose.position.y;
    entity.position.z = pose.position.z;
    entity.rotation.x = stored[0];
    entity.rotation.y = stored[1];
    entity.rotation.z = stored[2];
    entity.rotation.w = stored[3];
    entity.scale_xy = pose.scale_xy;
    entity.scale_z = pose.scale_z;
}

/// Stand the entity `index` of `map` at `pose`; an index past the entities is refused.
pub fn place_entity(map: &mut CMapData, index: usize, pose: Pose) -> Result<(), String> {
    let held = map.entities.len();
    let entity = map
        .entities
        .get_mut(index)
        .ok_or_else(|| format!("entity {index} is past the {held} the map holds"))?;
    if !(pose.position.is_finite()
        && pose.rotation.is_finite()
        && pose.scale_xy.is_finite()
        && pose.scale_z.is_finite())
    {
        return Err(
            "a position, rotation or scale that is not a finite number cannot be stored"
                .to_string(),
        );
    }
    set_pose(entity, pose);
    Ok(())
}

/// The entity `index` of the map file the source in `dir` cooks to: the record a new entity is a copy of.
pub fn read_entity(dir: &Path, index: usize) -> Result<CEntityDefAny, String> {
    let map = CMapData::from_bytes(&cooked(dir)?).map_err(|e| e.to_string())?;
    let held = map.entities.len();
    map.entities
        .into_iter()
        .nth(index)
        .ok_or_else(|| format!("entity {index} is past the {held} the map holds"))
}

/// Add an entity to `map` that places `archetype` at `pose`, as a copy of `donor` with that name and pose, no parent, no children and a guid of its own; its index.
pub fn add_entity(
    map: &mut CMapData,
    donor: &CEntityDefAny,
    archetype: crate::facts::Name,
    pose: Pose,
) -> Result<usize, String> {
    let mut entity = donor.clone();
    entity.archetype_name = match archetype.text {
        Some(text) => HashedString::new(text),
        None => HashedString::from_hash(archetype.hash),
    };
    entity.guid = or_world::create::free_guid(map);
    entity.parent_index = -1;
    entity.num_children = 0;
    set_pose(&mut entity, pose);
    map.entities.push(entity);
    Ok(map.entities.len() - 1)
}

/// Whether an entity with these `flags` names its LOD parent in the parent map's array, which one file cannot see.
pub(crate) fn in_parent_map(flags: u32) -> bool {
    flags & (1 << 3) != 0
}

/// Add an entity to `map` that is no copy: `placed`, with no parent, no children and a guid of its own; its index. The save refuses it in an RBM map that holds no entity.
pub fn add_new_entity(
    map: &mut CMapData,
    placed: &or_world::create::Placed,
) -> Result<usize, String> {
    or_world::create::place(map, placed).map_err(|e| e.to_string())
}

/// Take the entity `index` out of `map`, with the LOD links renumbered: what named it as a parent has none now, a parent it named has one child fewer, and what named an entity after it names the one before.
///
/// ⚠ An entity whose `flags` bit 3 is set indexes the parent map's array, which one file cannot see, so its index is left alone.
pub fn remove_entity(map: &mut CMapData, index: usize) -> Result<(), String> {
    if index >= map.entities.len() {
        return Err(format!(
            "entity {index} is past the {} the map holds",
            map.entities.len()
        ));
    }
    let removed = map.entities.remove(index);
    // Its parent, when that is in this map, has a child fewer.
    if let (false, Ok(parent)) = (
        in_parent_map(removed.flags),
        usize::try_from(removed.parent_index),
    ) {
        // ⚠ An entity that named itself has no parent left to count: the one now at its index is another entity.
        let at = match parent.cmp(&index) {
            std::cmp::Ordering::Greater => Some(parent - 1),
            std::cmp::Ordering::Less => Some(parent),
            std::cmp::Ordering::Equal => None,
        };
        if let Some(held) = at.and_then(|at| map.entities.get_mut(at)) {
            held.num_children = held.num_children.saturating_sub(1);
        }
    }
    for entity in &mut map.entities {
        if in_parent_map(entity.flags) {
            continue;
        }
        let Ok(held) = usize::try_from(entity.parent_index) else {
            continue;
        };
        match held.cmp(&index) {
            std::cmp::Ordering::Equal => entity.parent_index = -1,
            std::cmp::Ordering::Greater => entity.parent_index = (held - 1) as i32,
            std::cmp::Ordering::Less => {}
        }
    }
    Ok(())
}

/// The archetype `index` of the type file the source in `dir` cooks to: the record a new archetype is a copy of.
pub fn read_archetype(dir: &Path, index: usize) -> Result<CBaseArchetypeDefAny, String> {
    let types = CMapTypes::from_bytes(&cooked(dir)?).map_err(|e| e.to_string())?;
    let held = types.archetypes.len();
    types
        .archetypes
        .into_iter()
        .nth(index)
        .ok_or_else(|| format!("archetype {index} is past the {held} the type file holds"))
}

/// Add an archetype to `types` called `name`, as a copy of `donor` (a plain archetype, else the first plain one of `types`) with that name and asset name; its index.
pub fn add_archetype(
    types: &mut CMapTypes,
    donor: Option<&CBaseArchetypeDefAny>,
    name: &str,
    asset: Option<&str>,
) -> Result<usize, String> {
    if types.archetypes.iter().any(|a| a.name == name) {
        return Err(format!(
            "the type file already declares `{name}`, and the game resolves a name once"
        ));
    }
    // A copy is the plain archetype the derived records add to: an interior or a timed one is not what a person means by a new prop.
    let plain = |a: &CBaseArchetypeDefAny| matches!(a, CBaseArchetypeDefAny::Base(_));
    let from = donor
        .filter(|d| plain(d))
        .or_else(|| types.archetypes.iter().find(|a| plain(a)))
        .ok_or_else(|| {
            "an archetype is added as a copy of a plain one, and there is none to copy: add the first from another type file".to_string()
        })?;
    let mut archetype = from.clone();
    archetype.name = HashedString::new(name);
    archetype.asset_name = HashedString::new(asset.unwrap_or(name));
    types.archetypes.push(archetype);
    Ok(types.archetypes.len() - 1)
}

/// Add `archetype` to `types`, a record that is no copy (`or_world::create::archetype`); its index. A name the type file declares is refused, and the save refuses it in an RBM type file that holds none.
pub fn add_new_archetype(
    types: &mut CMapTypes,
    archetype: CBaseArchetypeDefAny,
) -> Result<usize, String> {
    if types
        .archetypes
        .iter()
        .any(|a| a.name.hash == archetype.name.hash)
    {
        return Err(format!(
            "the type file already declares `{}`, and the game resolves a name once",
            archetype.name.to_xml_text()
        ));
    }
    types.archetypes.push(archetype);
    Ok(types.archetypes.len() - 1)
}

/// Take the archetype `index` out of `types`.
pub fn remove_archetype(types: &mut CMapTypes, index: usize) -> Result<(), String> {
    if index >= types.archetypes.len() {
        return Err(format!(
            "archetype {index} is past the {} the type file holds",
            types.archetypes.len()
        ));
    }
    types.archetypes.remove(index);
    Ok(())
}

/// Grow the extents `map` states to hold the entity boxes `boxes` gives (a local box and the stream distance of each entity), never shrinking them: what else holds them up, a car generator or an occluder, is not the entities'. The archetypes `boxes` has none for, each once.
pub fn grow_extents(
    map: &mut CMapData,
    boxes: impl Fn(u32) -> Option<(Vec3, Vec3, f32)>,
) -> Vec<u32> {
    let mut without_box: Vec<u32> = Vec::new();
    let mut entities: Option<(Vec3, Vec3)> = None;
    let mut streaming: Option<(Vec3, Vec3)> = None;
    let add = |into: &mut Option<(Vec3, Vec3)>, point: Vec3| {
        let (min, max) = into.get_or_insert((point, point));
        *min = min.min(point);
        *max = max.max(point);
    };
    for entity in &map.entities {
        let pose = pose_of(entity);
        let Some((min, max, stream)) = boxes(entity.archetype_name.hash) else {
            // ⚠ An entity with no box is held where it stands, and as far around as it states it streams from: a lower bound, where
            // leaving it out would leave the game streaming the map by extents the entity stands outside.
            if !without_box.contains(&entity.archetype_name.hash) {
                without_box.push(entity.archetype_name.hash);
            }
            let reach = match entity.lod_dist.is_finite() && entity.lod_dist > 0.0 {
                true => Vec3::splat(entity.lod_dist),
                false => Vec3::ZERO,
            };
            add(&mut entities, pose.position);
            add(&mut streaming, pose.position - reach);
            add(&mut streaming, pose.position + reach);
            continue;
        };
        let scale = Vec3::new(pose.scale_xy, pose.scale_xy, pose.scale_z);
        let reach = if entity.lod_dist > 0.0 {
            entity.lod_dist
        } else {
            stream
        };
        // ⚠ By the size of the scale: an entity mirrored by a scale below nought is drawn as far as one that is not.
        let spread = reach / scale.abs().max_element().max(f32::MIN_POSITIVE);
        for (into, grow) in [(&mut entities, 0.0), (&mut streaming, spread)] {
            for corner in or_world::transform::placed_corners(
                (min - Vec3::splat(grow)) * scale,
                (max + Vec3::splat(grow)) * scale,
                pose.rotation,
                pose.position,
            ) {
                add(into, corner);
            }
        }
    }
    let to_vec = |v: or_io::decl::Vec3| Vec3::new(v.x, v.y, v.z);
    let from_vec = |v: Vec3| or_io::decl::Vec3 {
        x: v.x,
        y: v.y,
        z: v.z,
    };
    for (found, min, max) in [
        (
            entities,
            &mut map.entities_extents_min,
            &mut map.entities_extents_max,
        ),
        (
            streaming,
            &mut map.streaming_extents_min,
            &mut map.streaming_extents_max,
        ),
    ] {
        let Some((lo, hi)) = found else {
            continue;
        };
        // ⚠ Extents that are all zero are a map that states none yet, as one made from nothing does, and are set and not grown:
        // grown, they would hold the origin of the world beside the entity, and the game would stream the map from there to it.
        let stated_none = to_vec(*min) == Vec3::ZERO && to_vec(*max) == Vec3::ZERO;
        let (held_lo, held_hi) = match stated_none {
            true => (lo, hi),
            false => (to_vec(*min), to_vec(*max)),
        };
        *min = from_vec(held_lo.min(lo));
        *max = from_vec(held_hi.max(hi));
    }
    without_box
}
