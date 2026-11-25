//! The parts of an entity that a merge takes one by one, and an entity both sides changed merged part by part.

use super::*;

/// One part of an entity that is merged by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Position,
    Rotation,
    Scale,
    LodDist,
    Flags,
    Guid,
    Named(&'static str),
}

impl Part {
    fn all() -> impl Iterator<Item = Self> {
        [
            Self::Position,
            Self::Rotation,
            Self::Scale,
            Self::LodDist,
            Self::Flags,
            Self::Guid,
        ]
        .into_iter()
        .chain(NAMED.into_iter().chain(OF_INTERIOR).map(Self::Named))
    }

    fn label(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Rotation => "rotation",
            Self::Scale => "scaleXY and scaleZ",
            Self::LodDist => "lodDist",
            Self::Flags => "flags",
            Self::Guid => "guid",
            Self::Named(name) => name,
        }
    }

    fn changed_by(self, kept: &Kept) -> bool {
        kept.changes.iter().any(|change| match (change, self) {
            (EntityChange::Moved { .. }, Self::Position)
            | (EntityChange::Turned { .. }, Self::Rotation)
            | (EntityChange::Scaled { .. }, Self::Scale)
            | (EntityChange::LodDist { .. }, Self::LodDist)
            | (EntityChange::Flags { .. }, Self::Flags)
            | (EntityChange::Guid { .. }, Self::Guid) => true,
            (EntityChange::Members { names }, Self::Named(name)) => names.contains(&name),
            _ => false,
        })
    }

    /// What `entity` states of this part as the bits a file stores, none for a member of an interior instance on an entity that is none.
    fn bits(self, entity: &CEntityDefAny) -> Option<Vec<u32>> {
        let pose = pose_key(entity);
        Some(match self {
            Self::Position => pose[..3].to_vec(),
            Self::Rotation => pose[3..7].to_vec(),
            Self::Scale => pose[7..].to_vec(),
            Self::LodDist => vec![entity.lod_dist.to_bits()],
            Self::Flags => vec![entity.flags],
            Self::Guid => vec![entity.guid],
            Self::Named("childLodDist") => vec![entity.child_lod_dist.to_bits()],
            Self::Named("ambientOcclusionMultiplier") => {
                vec![entity.ambient_occlusion_multiplier as u32]
            }
            Self::Named("artificialAmbientOcclusion") => {
                vec![entity.artificial_ambient_occlusion as u32]
            }
            Self::Named("tintValue") => vec![entity.tint_value],
            Self::Named(name) => {
                let interior = entity.as_mlo_instance()?;
                match name {
                    "groupId" => vec![interior.group_id],
                    "floorId" => vec![interior.floor_id],
                    "defaultEntitySets" => (interior.default_entity_sets.iter())
                        .map(|set| set.hash)
                        .collect(),
                    "numExitPortals" => vec![interior.num_exit_portals],
                    "MLOInstflags" => vec![interior.mlo_instflags],
                    _ => return None,
                }
            }
        })
    }

    /// State in `to` what `from` states of this part; both are of one type.
    fn copy(self, to: &mut CEntityDefAny, from: &CEntityDefAny) {
        match self {
            Self::Position => to.position = from.position,
            Self::Rotation => to.rotation = from.rotation,
            Self::Scale => (to.scale_xy, to.scale_z) = (from.scale_xy, from.scale_z),
            Self::LodDist => to.lod_dist = from.lod_dist,
            Self::Flags => to.flags = from.flags,
            Self::Guid => to.guid = from.guid,
            Self::Named("childLodDist") => to.child_lod_dist = from.child_lod_dist,
            Self::Named("ambientOcclusionMultiplier") => {
                to.ambient_occlusion_multiplier = from.ambient_occlusion_multiplier;
            }
            Self::Named("artificialAmbientOcclusion") => {
                to.artificial_ambient_occlusion = from.artificial_ambient_occlusion;
            }
            Self::Named("tintValue") => to.tint_value = from.tint_value,
            Self::Named(name) => {
                let (CEntityDefAny::CMloInstanceDef(to), CEntityDefAny::CMloInstanceDef(from)) =
                    (to, from)
                else {
                    return;
                };
                match name {
                    "groupId" => to.group_id = from.group_id,
                    "floorId" => to.floor_id = from.floor_id,
                    "defaultEntitySets" => {
                        to.default_entity_sets = from.default_entity_sets.clone();
                    }
                    "numExitPortals" => to.num_exit_portals = from.num_exit_portals,
                    "MLOInstflags" => to.mlo_instflags = from.mlo_instflags,
                    _ => {}
                }
            }
        }
    }

    fn shown(self, entity: &CEntityDefAny) -> String {
        match self {
            Self::Position => numbers(&pose_of(entity).position.to_array()),
            Self::Rotation => numbers(&pose_of(entity).rotation.to_array()),
            Self::Scale => numbers(&[entity.scale_xy, entity.scale_z]),
            Self::LodDist => entity.lod_dist.to_string(),
            Self::Named("childLodDist") => entity.child_lod_dist.to_string(),
            Self::Named("ambientOcclusionMultiplier") => {
                entity.ambient_occlusion_multiplier.to_string()
            }
            Self::Named("artificialAmbientOcclusion") => {
                entity.artificial_ambient_occlusion.to_string()
            }
            _ => {
                let said: Vec<String> = (self.bits(entity).unwrap_or_default().iter())
                    .map(ToString::to_string)
                    .collect();
                said.join(", ")
            }
        }
    }
}

/// The names a change of an entity is said by, one for each thing [`Kept::changes`] lists.
pub(super) fn labels(kept: &Kept) -> Vec<String> {
    let mut said = Vec::new();
    for change in &kept.changes {
        match change {
            EntityChange::Moved { .. } => said.push(Part::Position.label().to_string()),
            EntityChange::Turned { .. } => said.push(Part::Rotation.label().to_string()),
            EntityChange::Scaled { .. } => said.push(Part::Scale.label().to_string()),
            EntityChange::Archetype { .. } => said.push("archetypeName".to_string()),
            EntityChange::LodDist { .. } => said.push(Part::LodDist.label().to_string()),
            EntityChange::Flags { .. } => said.push(Part::Flags.label().to_string()),
            EntityChange::Parent { .. } => said.push(PARENT.to_string()),
            EntityChange::Guid { .. } => said.push(Part::Guid.label().to_string()),
            EntityChange::Members { names } => said.extend(names.iter().map(ToString::to_string)),
            EntityChange::Other => said.push(OTHER.to_string()),
        }
    }
    said
}

pub(super) fn changed_other(kept: &Kept) -> bool {
    kept.changes.contains(&EntityChange::Other)
}

pub(super) fn changed_parent(kept: &Kept) -> bool {
    (kept.changes.iter()).any(|c| matches!(c, EntityChange::Parent { .. }))
}

pub(super) fn changed_count(kept: &Kept) -> bool {
    (kept.changes.iter())
        .any(|c| matches!(c, EntityChange::Members { names } if names.contains(&COUNT)))
}

/// Whether every change of `kept` is of a part this module merges by itself.
///
/// ⚠ An entity given another archetype is not: under one guid it may be another entity, put in after the first was taken out.
pub(super) fn member_wise(kept: &Kept) -> bool {
    kept.changes.iter().all(|change| match change {
        EntityChange::Archetype { .. } => false,
        EntityChange::Members { names } => names
            .iter()
            .all(|name| *name == COUNT || NAMED.contains(name) || OF_INTERIOR.contains(name)),
        _ => true,
    })
}

pub(super) fn is_interior(entity: &CEntityDefAny) -> bool {
    entity.as_mlo_instance().is_some()
}

/// Whether two entities state the same of everything but their links: every part to the bit, and what no typed member names.
pub(super) fn same_record(a: &CEntityDefAny, b: &CEntityDefAny) -> bool {
    Part::all().all(|part| part.bits(a) == part.bits(b)) && same_rest(a, b)
}

/// Whether two entities are of one type, place one archetype and state the same of what no typed member names, whatever each
/// states of a part.
pub(super) fn same_rest(a: &CEntityDefAny, b: &CEntityDefAny) -> bool {
    if is_interior(a) != is_interior(b) || a.archetype_name.hash != b.archetype_name.hash {
        return false;
    }
    // Each with the parts of `a`, and blank where a number is compared as a number: what is left to differ is what no part names.
    let blank = |entity: &CEntityDefAny| {
        let mut blank = entity.clone();
        Part::all().for_each(|part| part.copy(&mut blank, a));
        let record: &mut CEntityDef = &mut blank;
        // ⚠ A name is compared with its text, which one file may give and another not.
        record.archetype_name = Default::default();
        record.position = Default::default();
        record.rotation = Default::default();
        (record.scale_xy, record.scale_z) = (0.0, 0.0);
        (record.lod_dist, record.child_lod_dist) = (0.0, 0.0);
        (record.parent_index, record.num_children) = (0, 0);
        blank
    };
    blank(a) == blank(b)
}

/// The parts two entities state differently, by their names, and the words for what no typed member names when that is all they differ in.
pub(super) fn parts_that_differ(a: &CEntityDefAny, b: &CEntityDefAny) -> Vec<&'static str> {
    let mut said: Vec<&'static str> = Part::all()
        .filter(|part| part.bits(a) != part.bits(b))
        .map(Part::label)
        .collect();
    if said.is_empty() && !same_record(a, b) {
        said.push(OTHER);
    }
    said
}

/// An entity both sides changed, merged part by part from `was`, the entity of the base: the record, with the links of none, and the parts no conflict decided, each with the side it is from. Each side is its entity and what the comparison says it changed.
pub(super) fn merge_record(
    was: &CEntityDefAny,
    ours: (&CEntityDefAny, &Kept),
    theirs: (&CEntityDefAny, &Kept),
    ask: &mut dyn FnMut(Disagreement) -> Side,
) -> (CEntityDefAny, Vec<MemberBy>) {
    let ((a, of_ours), (b, of_theirs)) = (ours, theirs);
    let record_of = |side: Option<Side>| match side {
        None => was,
        Some(Side::Ours) => a,
        Some(Side::Theirs) => b,
    };
    let mut members = Vec::new();
    let mut said = |member: &str, from: Option<(Side, Option<By>)>| {
        if let Some((_, Some(by))) = from {
            let member = member.to_string();
            members.push(MemberBy { member, by });
        }
    };
    // What no typed member names is in the record it is taken with, so the record starts as that side's.
    let rest = three_way(
        changed_other(of_ours),
        changed_other(of_theirs),
        || same_rest(a, b),
        || {
            ask(Disagreement::Member {
                member: OTHER.to_string(),
                ours: "changed".to_string(),
                theirs: "changed".to_string(),
            })
        },
    );
    said(OTHER, rest);
    let mut record = record_of(rest.map(|(side, _)| side)).clone();
    for part in Part::all() {
        let from = three_way(
            part.changed_by(of_ours),
            part.changed_by(of_theirs),
            || part.bits(a) == part.bits(b),
            || {
                ask(Disagreement::Member {
                    member: part.label().to_string(),
                    ours: part.shown(a),
                    theirs: part.shown(b),
                })
            },
        );
        // ⚠ Also a part neither changed, from the base: the record may have started as a side's.
        part.copy(&mut record, record_of(from.map(|(side, _)| side)));
        said(part.label(), from);
    }
    (record, members)
}

pub(super) fn entity_of(index: usize, entity: &CEntityDef) -> Ref {
    Ref::Entity(entity_ref(index, entity))
}
