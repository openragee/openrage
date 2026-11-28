//! Creating and deleting world data, as the set operations they are.
//!
//! ⚠ A map name names a set of files and a manifest names the map, so each
//! half of a create or delete can be individually valid and collectively
//! wrong — which [`crate::write::refuse_if_broken`], handed one document,
//! cannot see. [`Plan::check`] answers what the resulting [`MapGroup`] and
//! manifests would be wrong about, before anything is written.
//!
//! ⚠ It plans; it does not write. Authoring a `.ymap` from nothing needs a
//! layout `or_io::meta::binary::synth` derives from the gitignored schema
//! snapshot; without one, clone a donor and edit it, as
//! `openrage-cli meta from-xml --like` does.

use std::collections::{BTreeMap, BTreeSet};

use crate::group::MapGroup;
use crate::manifest::StatedDependencies;

/// One intended change to a set of world files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Add a map, with the parent it will name and the types it will need.
    CreateMap {
        name: u32,
        parent: u32,
        types: BTreeSet<u32>,
    },
    /// Remove a map and every file that declares its name.
    DeleteMap { name: u32 },
    /// Remove one entity from a map.
    DeleteEntity { map: u32, entity: u32 },
}

/// Something the set would be wrong about after the plan ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanProblem {
    /// A created map no manifest would mention.
    CreatedMapUnstated { map: u32 },
    /// A created map needing type files no manifest states for it.
    CreatedMapNeedsUnstatedTypes { map: u32, types: Vec<u32> },
    /// A created map naming a parent the group does not hold.
    CreatedMapParentAbsent { map: u32, parent: u32 },
    /// A created map whose name a map already in the group holds.
    CreatedMapNameTaken { map: u32 },
    /// A deleted map a manifest would still state.
    DeletedMapStillStated { map: u32 },
    /// A deleted map other maps would still name as their LOD parent.
    DeletedMapStillAParent { map: u32, orphans: Vec<u32> },
    /// A deleted entity that other entities name as their LOD parent.
    DeletedEntityIsALodParent {
        map: u32,
        entity: u32,
        children: u32,
    },
    /// Deleting an entity would move every index above it.
    ///
    /// ⚠ An entity's position in `entities` is its identity: removing element
    /// `i` makes every index above it — LOD links in this map and in child
    /// maps, a room's `attachedObjects`, an entity set's `locations` — name
    /// its neighbour, while the edited file still validates on its own.
    DeletedEntityMovesIndices {
        map: u32,
        entity: u32,
        local: u64,
        cross_file: u64,
    },
}

impl std::fmt::Display for PlanProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanProblem::CreatedMapUnstated { map } => write!(
                f,
                "new map {map:#010X} is in no manifest, so the game would not load it"
            ),
            PlanProblem::CreatedMapNeedsUnstatedTypes { map, types } => write!(
                f,
                "new map {map:#010X} needs {} type file(s) no manifest states for it: {}",
                types.len(),
                types
                    .iter()
                    .map(|t| format!("{t:#010X}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PlanProblem::CreatedMapParentAbsent { map, parent } => write!(
                f,
                "new map {map:#010X} names parent {parent:#010X}, which is not in the set"
            ),
            PlanProblem::CreatedMapNameTaken { map } => write!(
                f,
                "new map {map:#010X} takes a name the set already uses; a name is a map, not a file"
            ),
            PlanProblem::DeletedMapStillStated { map } => write!(
                f,
                "deleted map {map:#010X} is still stated by a manifest"
            ),
            PlanProblem::DeletedMapStillAParent { map, orphans } => write!(
                f,
                "deleted map {map:#010X} is the LOD parent of {} map(s) that would be orphaned",
                orphans.len()
            ),
            PlanProblem::DeletedEntityIsALodParent {
                map,
                entity,
                children,
            } => write!(
                f,
                "deleting entity {entity} of map {map:#010X} would orphan {children} child entity(ies)"
            ),
            PlanProblem::DeletedEntityMovesIndices {
                map,
                entity,
                local,
                cross_file,
            } => write!(
                f,
                "deleting entity {entity} of map {map:#010X} moves {local} local and \
                 {cross_file} cross-file index(es); every one would name its neighbour"
            ),
        }
    }
}

/// A set of intended edits, checked together.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    edits: Vec<Edit>,
}

impl Plan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, edit: Edit) -> Self {
        self.edits.push(edit);
        self
    }

    pub fn push(&mut self, edit: Edit) {
        self.edits.push(edit);
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// What would be wrong about the set once this plan had run.
    ///
    /// ⚠ Judged against the plan's outcome, not the current state: `stated`
    /// is the manifests as they will be once the plan has run, so a map
    /// created and stated in the same change is accepted.
    pub fn check(&self, group: &MapGroup, stated: &StatedDependencies) -> Vec<PlanProblem> {
        let mut problems = Vec::new();

        let deleted: BTreeSet<u32> = self
            .edits
            .iter()
            .filter_map(|e| match e {
                Edit::DeleteMap { name } => Some(*name),
                _ => None,
            })
            .collect();
        let created: BTreeMap<u32, (&u32, &BTreeSet<u32>)> = self
            .edits
            .iter()
            .filter_map(|e| match e {
                Edit::CreateMap {
                    name,
                    parent,
                    types,
                } => Some((*name, (parent, types))),
                _ => None,
            })
            .collect();

        for (&name, (parent, types)) in &created {
            if group.contains(name) && !deleted.contains(&name) {
                problems.push(PlanProblem::CreatedMapNameTaken { map: name });
            }
            if **parent != 0
                && !created.contains_key(*parent)
                && (!group.contains(**parent) || deleted.contains(*parent))
            {
                problems.push(PlanProblem::CreatedMapParentAbsent {
                    map: name,
                    parent: **parent,
                });
            }
            match stated.of(name) {
                None => problems.push(PlanProblem::CreatedMapUnstated { map: name }),
                Some(have) => {
                    let missing: Vec<u32> = types.difference(have).copied().collect();
                    if !missing.is_empty() {
                        problems.push(PlanProblem::CreatedMapNeedsUnstatedTypes {
                            map: name,
                            types: missing,
                        });
                    }
                }
            }
        }

        for &name in &deleted {
            if stated.is_stated(name) {
                problems.push(PlanProblem::DeletedMapStillStated { map: name });
            }
            let orphans: Vec<u32> = group
                .children_of(name)
                .filter(|child| !deleted.contains(child))
                .collect();
            if !orphans.is_empty() {
                problems.push(PlanProblem::DeletedMapStillAParent { map: name, orphans });
            }
        }

        for edit in &self.edits {
            let Edit::DeleteEntity { map, entity } = edit else {
                continue;
            };
            if deleted.contains(map) {
                continue;
            }
            let children = group.children_naming(*map, *entity);
            if children > 0 {
                problems.push(PlanProblem::DeletedEntityIsALodParent {
                    map: *map,
                    entity: *entity,
                    children,
                });
            }
            let (local, cross_file) = group.indices_above(*map, *entity);
            if local + cross_file > 0 {
                problems.push(PlanProblem::DeletedEntityMovesIndices {
                    map: *map,
                    entity: *entity,
                    local,
                    cross_file,
                });
            }
        }

        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_data::MapData;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::{MetaInstance, Value};

    const IN_PARENT: u32 = 1 << 3;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn map(name: &str, parent: Option<&str>, entities: &[(i32, u32, u32)]) -> MetaInstance {
        let items = entities
            .iter()
            .map(|(parent_index, num_children, flags)| {
                let mut ent = MetaInstance::new("CEntityDef");
                ent.set("parentIndex", Value::Int(*parent_index as i64));
                ent.set("numChildren", Value::UInt(*num_children as u64));
                ent.set("flags", Value::UInt(*flags as u64));
                Value::Pointer(Some(Box::new(ent)))
            })
            .collect();
        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::Hash(HashedString::new(name)));
        root.set(
            "parent",
            match parent {
                Some(p) => Value::Hash(HashedString::new(p)),
                None => Value::Hash(HashedString::from_hash(0)),
            },
        );
        root.set("entities", Value::Array(items));
        root
    }

    fn manifest_stating(entries: &[(&str, &[&str])]) -> MetaInstance {
        let items = entries
            .iter()
            .map(|(m, types)| {
                let mut inst = MetaInstance::new("CImapDependencies");
                inst.set("imapName", Value::Hash(HashedString::new(*m)));
                inst.set(
                    "itypDepArray",
                    Value::Array(
                        types
                            .iter()
                            .map(|t| Value::Hash(HashedString::new(*t)))
                            .collect(),
                    ),
                );
                Value::Struct(Box::new(inst))
            })
            .collect();
        let mut root = MetaInstance::new("CPackFileMetaData");
        root.set("imapDependencies_2", Value::Array(items));
        root
    }

    #[test]
    fn a_created_map_no_manifest_states_is_refused() {
        let existing = map("lod", None, &[(-1, 0, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&existing).unwrap());
        let stated = StatedDependencies::new();

        let plan = Plan::new().with(Edit::CreateMap {
            name: hash("new_hd"),
            parent: hash("lod"),
            types: BTreeSet::new(),
        });
        let problems = plan.check(&group, &stated);
        assert!(
            problems.contains(&PlanProblem::CreatedMapUnstated {
                map: hash("new_hd")
            }),
            "{problems:?}"
        );
    }

    #[test]
    fn a_created_map_a_manifest_states_is_accepted() {
        let existing = map("lod", None, &[(-1, 0, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&existing).unwrap());

        let tree = manifest_stating(&[("new_hd", &["t1"])]);
        let mut stated = StatedDependencies::new();
        stated.add(crate::manifest::Manifest::new(&tree).unwrap());

        let plan = Plan::new().with(Edit::CreateMap {
            name: hash("new_hd"),
            parent: hash("lod"),
            types: BTreeSet::from([hash("t1")]),
        });
        assert_eq!(plan.check(&group, &stated), vec![]);
    }

    #[test]
    fn a_created_map_needing_an_unstated_type_is_refused() {
        let tree = manifest_stating(&[("new_hd", &["t1"])]);
        let mut stated = StatedDependencies::new();
        stated.add(crate::manifest::Manifest::new(&tree).unwrap());

        let plan = Plan::new().with(Edit::CreateMap {
            name: hash("new_hd"),
            parent: 0,
            types: BTreeSet::from([hash("t1"), hash("t2")]),
        });
        let problems = plan.check(&MapGroup::new(), &stated);
        assert!(problems.iter().any(|p| matches!(
            p,
            PlanProblem::CreatedMapNeedsUnstatedTypes { types, .. } if types == &vec![hash("t2")]
        )), "{problems:?}");
    }

    /// ⚠ A name is a map, not a file, so reusing one merges into it.
    #[test]
    fn a_created_map_cannot_take_a_name_already_in_use() {
        let existing = map("taken", None, &[]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&existing).unwrap());

        let plan = Plan::new().with(Edit::CreateMap {
            name: hash("taken"),
            parent: 0,
            types: BTreeSet::new(),
        });
        assert!(plan
            .check(&group, &StatedDependencies::new())
            .contains(&PlanProblem::CreatedMapNameTaken { map: hash("taken") }));
    }

    #[test]
    fn deleting_a_map_that_is_still_a_parent_is_refused() {
        let lod = map("lod", None, &[(-1, 1, 0)]);
        let hd = map("hd", Some("lod"), &[(0, 0, IN_PARENT)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&lod).unwrap());
        group.add(&MapData::new(&hd).unwrap());

        let plan = Plan::new().with(Edit::DeleteMap { name: hash("lod") });
        let problems = plan.check(&group, &StatedDependencies::new());
        assert!(
            problems.iter().any(|p| matches!(
                p,
                PlanProblem::DeletedMapStillAParent { orphans, .. } if orphans == &vec![hash("hd")]
            )),
            "{problems:?}"
        );

        let both = Plan::new()
            .with(Edit::DeleteMap { name: hash("lod") })
            .with(Edit::DeleteMap { name: hash("hd") });
        assert_eq!(both.check(&group, &StatedDependencies::new()), vec![]);
    }

    #[test]
    fn deleting_a_map_a_manifest_still_states_is_refused() {
        let tree = manifest_stating(&[("doomed", &["t1"])]);
        let mut stated = StatedDependencies::new();
        stated.add(crate::manifest::Manifest::new(&tree).unwrap());

        let plan = Plan::new().with(Edit::DeleteMap {
            name: hash("doomed"),
        });
        assert!(plan.check(&MapGroup::new(), &stated).contains(
            &PlanProblem::DeletedMapStillStated {
                map: hash("doomed")
            }
        ));
    }

    /// `lod` holds four entities: 0 and 2 each declare one child, 1 links
    /// locally to 0, and 3 links locally to 2; `hd` links across files to 3.
    /// Deleting 0 orphans 1 and moves 3's local link and `hd`'s cross-file
    /// link. 1's link names 0 itself, which is removed rather than moved.
    #[test]
    fn deleting_an_entity_reports_both_the_orphan_and_the_renumbering() {
        let lod = map("lod", None, &[(-1, 1, 0), (0, 0, 0), (-1, 1, 0), (2, 0, 0)]);
        let hd = map("hd", Some("lod"), &[(3, 0, IN_PARENT)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&lod).unwrap());
        group.add(&MapData::new(&hd).unwrap());

        assert_eq!(group.children_naming(hash("lod"), 0), 1);
        assert_eq!(group.indices_above(hash("lod"), 0), (1, 1));

        let plan = Plan::new().with(Edit::DeleteEntity {
            map: hash("lod"),
            entity: 0,
        });
        let problems = plan.check(&group, &StatedDependencies::new());

        assert!(
            problems.iter().any(|p| matches!(
                p,
                PlanProblem::DeletedEntityIsALodParent { children: 1, .. }
            )),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| matches!(
                p,
                PlanProblem::DeletedEntityMovesIndices {
                    local: 1,
                    cross_file: 1,
                    ..
                }
            )),
            "{problems:?}"
        );
    }

    /// The control: without it the renumbering check could refuse every
    /// delete and still pass.
    #[test]
    fn deleting_the_topmost_entity_moves_no_index() {
        let lod = map("lod", None, &[(-1, 1, 0), (0, 0, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&lod).unwrap());

        assert_eq!(group.indices_above(hash("lod"), 1), (0, 0));
        let plan = Plan::new().with(Edit::DeleteEntity {
            map: hash("lod"),
            entity: 1,
        });
        let problems = plan.check(&group, &StatedDependencies::new());
        assert!(
            !problems
                .iter()
                .any(|p| matches!(p, PlanProblem::DeletedEntityMovesIndices { .. })),
            "{problems:?}"
        );

        // Entity 1's link names 0 itself, which is removed rather than moved.
        assert_eq!(group.indices_above(hash("lod"), 0), (0, 0));
    }

    #[test]
    fn an_empty_plan_has_nothing_to_report() {
        assert!(Plan::new().is_empty());
        assert_eq!(
            Plan::new().check(&MapGroup::new(), &StatedDependencies::new()),
            vec![]
        );
    }

    #[test]
    fn a_problem_says_what_it_is_in_a_sentence() {
        let text = PlanProblem::DeletedEntityMovesIndices {
            map: 0x1234_5678,
            entity: 4,
            local: 9,
            cross_file: 3,
        }
        .to_string();
        assert!(text.contains("entity 4"), "{text}");
        assert!(text.contains('9') && text.contains('3'), "{text}");
    }
}
