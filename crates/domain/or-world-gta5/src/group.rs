//! A map group: several `.ymap` files that only make sense together.
//!
//! A `.ymap` names a `parent`, and its entities' LOD links reach into that
//! parent's entity array rather than their own, so a tool that edits one file
//! of a group has moved an index the other files hold.
//!
//! ⚠ **This is why [`crate::validate`] cannot do it.** Where a cross-file
//! `parentIndex` lands, and whether `numChildren` is right, need every member
//! present. Checking them from one file reports maps the game ships and
//! loads: over a retail Legacy install, 478,261 entities point past the end
//! of their own map and 455,720 declare a child count their own file cannot
//! account for.
//!
//! ⚠ **A map name names a set of files.** The metadata `.ymap` holds the
//! entities; each `*_grass_N.ymap` in an `_instance_placement.rpf` holds one
//! instanced batch; and every one of them declares the same `name`. So a
//! group merges parts and never replaces on a name, and
//! [`Summary::absorb`] shifts the local indices of a part appended after
//! another. Replacing instead substitutes entity-less grass stubs for 326
//! populated LOD parents.
//!
//! ⚠ **A group summarises; it does not borrow.** [`MapGroup::add`] copies
//! the handful of numbers a cross-file check needs and lets the caller drop
//! the value tree. Holding `MapData<'a>` for every member instead reached
//! 38 GB on a 22,155-file run before the kernel killed it: the corpus's trees
//! hold 3.16 million entity structs and 38.4 million grass instances, and
//! nothing but eight bytes an entity is needed here.

use std::collections::{BTreeMap, HashMap};

use crate::map_data::MapData;

/// Something wrong across a group that no member could see alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupProblem {
    /// A map naming a parent that is not in the group.
    ParentMapMissing { map: u32, parent: u32 },
    /// A cross-file LOD link on a map with no parent to reach into.
    ParentIndexWithoutParentMap { map: u32, entity: u32 },
    /// A cross-file LOD link landing past the end of the parent's array.
    ParentIndexOutOfRange {
        map: u32,
        entity: u32,
        parent_map: u32,
        index: u32,
        len: u32,
    },
    /// `numChildren` lower than the number of entities naming this one.
    ///
    /// ⚠ The direction that loses data: a child the parent does not count is
    /// a child the streamer does not know to swap in. **Zero shipped
    /// entities are in this state**, which is what makes it safe to refuse.
    ChildCountUnderstated {
        map: u32,
        entity: u32,
        declared: u32,
        actual: u32,
    },
    /// `numChildren` higher than the number of entities naming this one.
    ///
    /// ⚠ Advisory, on evidence: the engine looks for a child that is not
    /// there and carries on, and a retail Legacy install ships **9 of
    /// these** out of 171,282 declarations. Refusing them would block saving
    /// four maps and their Heist variants for a defect that predates the
    /// editor.
    ChildCountOverstated {
        map: u32,
        entity: u32,
        declared: u32,
        actual: u32,
    },
    /// Two maps in the group claiming the same name.
    DuplicateName { map: u32 },
    /// A cycle in the parent-map chain.
    ParentMapCycle { map: u32 },
}

impl std::fmt::Display for GroupProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GroupProblem::ParentMapMissing { map, parent } => write!(
                f,
                "map {map:#010X} names parent {parent:#010X}, which is not in the group"
            ),
            GroupProblem::ParentIndexWithoutParentMap { map, entity } => write!(
                f,
                "map {map:#010X} entity {entity} links into a parent map and the map names none"
            ),
            GroupProblem::ParentIndexOutOfRange {
                map,
                entity,
                parent_map,
                index,
                len,
            } => write!(
                f,
                "map {map:#010X} entity {entity} names index {index} in parent \
                 {parent_map:#010X}, which holds {len}"
            ),
            GroupProblem::ChildCountUnderstated {
                map,
                entity,
                declared,
                actual,
            }
            | GroupProblem::ChildCountOverstated {
                map,
                entity,
                declared,
                actual,
            } => write!(
                f,
                "map {map:#010X} entity {entity} declares {declared} children and {actual} name it"
            ),
            GroupProblem::DuplicateName { map } => {
                write!(f, "two maps in the group are named {map:#010X}")
            }
            GroupProblem::ParentMapCycle { map } => {
                write!(f, "map {map:#010X} is in a parent-map cycle")
            }
        }
    }
}

/// How many cross-file links fall inside each candidate index space.
///
/// A candidate that accounts for every link is a model; one that accounts for
/// most is a coincidence to be suspicious of, because an index into a small
/// array is inside a large one too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexSpaceFit {
    /// Cross-file links considered, over maps whose parent is present.
    pub total: u64,
    /// The parent map's own `entities` array.
    pub parent_own: u64,
    /// The parent's array plus every map that names it as parent.
    pub parent_merged: u64,
    /// The parent's array plus its descendants, as a merge would assemble it.
    pub parent_chain: u64,
    /// The child's own array — the reading bit 3 is supposed to rule out.
    pub own_array: u64,
    /// Links whose parent map holds no entities at all.
    ///
    /// ⚠ The discriminator between a wrong model and wrong data. An index
    /// into an empty array cannot be satisfied by *any* index space, so
    /// these are links the shipped file cannot resolve either.
    pub outside_with_empty_parent: u64,
    /// Distinct parent maps that are empty and are pointed into.
    pub empty_parents: u64,
    /// How far past the parent's own array the overflowing links reach.
    pub overflow_sum: u64,
    pub worst_overflow: u64,
}

impl IndexSpaceFit {
    /// Whether one candidate accounts for every link considered.
    pub fn is_settled(&self) -> bool {
        self.total > 0
            && (self.parent_own == self.total
                || self.parent_merged == self.total
                || self.parent_chain == self.total
                || self.own_array == self.total)
    }

    /// The mean distance past the parent's array, over the links that pass it.
    pub fn mean_overflow(&self) -> f64 {
        let outside = self.total - self.parent_own;
        if outside == 0 {
            return 0.0;
        }
        self.overflow_sum as f64 / outside as f64
    }
}

fn disagreement(map: u32, entity: u32, declared: u32, actual: u32) -> Option<GroupProblem> {
    match declared.cmp(&actual) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Less => Some(GroupProblem::ChildCountUnderstated {
            map,
            entity,
            declared,
            actual,
        }),
        std::cmp::Ordering::Greater => Some(GroupProblem::ChildCountOverstated {
            map,
            entity,
            declared,
            actual,
        }),
    }
}

/// One cross-file link that reaches past the parent's own array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow {
    pub map: u32,
    pub map_entities: u32,
    pub entity: u32,
    pub index: u32,
    pub parent: u32,
    pub parent_entities: u32,
}

impl std::fmt::Display for Overflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:#010X} ({} entities) entity {} -> index {} of parent {:#010X} ({} entities)",
            self.map, self.map_entities, self.entity, self.index, self.parent, self.parent_entities
        )
    }
}

/// One entity's LOD link, as the group needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Link {
    entity: u32,
    index: u32,
    in_parent_map: bool,
}

/// What a group keeps of one map.
///
/// ⚠ Entities with neither a link nor a declared child count are not stored
/// at all, which is most of them: the two vectors are sparse on purpose.
#[derive(Debug, Clone, Default)]
struct Summary {
    parent: u32,
    entity_count: u32,
    /// Where the first part came from, so a real collision can name it.
    origin: String,
    /// How many files contributed to this map.
    parts: u32,
    links: Vec<Link>,
    declared_children: Vec<(u32, u32)>,
}

impl Summary {
    fn of(map: &MapData<'_>) -> Self {
        let mut links = Vec::new();
        let mut declared_children = Vec::new();
        for (i, entity) in map.entities().enumerate() {
            let i = i as u32;
            if let Some(index) = entity.parent_index() {
                links.push(Link {
                    entity: i,
                    index: index as u32,
                    in_parent_map: entity.lod_in_parent_map(),
                });
            }
            let declared = entity.num_children();
            if declared != 0 {
                declared_children.push((i, declared));
            }
        }
        links.shrink_to_fit();
        declared_children.shrink_to_fit();
        Self {
            parent: map.parent(),
            entity_count: map.entity_count() as u32,
            origin: String::new(),
            parts: 1,
            links,
            declared_children,
        }
    }

    /// Fold another part of the same logical map into this one.
    ///
    /// ⚠ An entity index is relative to its own part, so a part appended
    /// after `entity_count` entities has every local index shifted by it. A
    /// *cross-file* index is not shifted: it names a position in the parent's
    /// assembled array, which this part played no role in building.
    fn absorb(&mut self, other: Summary) {
        let base = self.entity_count;
        self.entity_count += other.entity_count;
        if self.parent == 0 {
            self.parent = other.parent;
        }
        self.parts += other.parts;
        self.links.extend(other.links.into_iter().map(|mut link| {
            link.entity += base;
            if !link.in_parent_map {
                link.index += base;
            }
            link
        }));
        self.declared_children.extend(
            other
                .declared_children
                .into_iter()
                .map(|(i, n)| (i + base, n)),
        );
    }

    /// Roughly how many bytes this summary holds.
    fn footprint(&self) -> usize {
        self.links.capacity() * std::mem::size_of::<Link>()
            + self.declared_children.capacity() * std::mem::size_of::<(u32, u32)>()
    }
}

/// Several maps' cross-file facts, indexed by the name each one carries.
#[derive(Debug, Default, Clone)]
pub struct MapGroup {
    by_name: HashMap<u32, Summary>,
    duplicates: Vec<Collision>,
}

/// Two files declaring the same map name, and which one the group kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub map: u32,
    pub kept: String,
    pub kept_entities: u32,
    pub dropped: String,
    pub dropped_entities: u32,
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:#010X}: kept {} ({} entities), dropped {} ({} entities)",
            self.map, self.kept, self.kept_entities, self.dropped, self.dropped_entities
        )
    }
}

impl MapGroup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take what a cross-file check needs, so the caller can drop the tree.
    ///
    /// ⚠ Keyed by the map's own `name` field, never by its filename. A
    /// `parent` is a hash of the parent's declared name, and a file renamed
    /// on disk keeps the name it declares.
    pub fn add(&mut self, map: &MapData<'_>) {
        self.add_from(map, "");
    }

    /// As [`Self::add`], recording where the part came from.
    ///
    /// Parts declaring the same `name` are merged rather than replaced,
    /// because a map name names a set of files (see the module header).
    pub fn add_from(&mut self, map: &MapData<'_>, origin: &str) {
        let name = map.name();
        if name == 0 {
            return;
        }
        let mut summary = Summary::of(map);
        summary.origin = origin.to_string();
        match self.by_name.get_mut(&name) {
            None => {
                self.by_name.insert(name, summary);
            }
            Some(existing) => {
                if existing.entity_count > 0 && summary.entity_count > 0 {
                    // Two parts both holding entities is a genuine ambiguity
                    // about whose index space is whose, and is not the grass
                    // case. Recorded rather than silently concatenated.
                    self.duplicates.push(Collision {
                        map: name,
                        kept: existing.origin.clone(),
                        kept_entities: existing.entity_count,
                        dropped: summary.origin.clone(),
                        dropped_entities: summary.entity_count,
                    });
                }
                existing.absorb(summary);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// How many bytes the group is holding.
    pub fn footprint(&self) -> usize {
        self.by_name.values().map(Summary::footprint).sum::<usize>()
            + self.by_name.len() * std::mem::size_of::<(u32, Summary)>()
    }

    /// Whether the group holds a map of this name.
    pub fn contains(&self, name: u32) -> bool {
        self.by_name.contains_key(&name)
    }

    /// The maps naming `name` as their LOD parent.
    pub fn children_of(&self, name: u32) -> impl Iterator<Item = u32> + '_ {
        self.by_name
            .iter()
            .filter(move |(_, s)| s.parent == name)
            .map(|(&child, _)| child)
    }

    /// How many entities in the group name `(map, entity)` as their parent.
    pub fn children_naming(&self, map: u32, entity: u32) -> u32 {
        self.children_by_parent()
            .get(&(map, entity))
            .copied()
            .unwrap_or(0)
    }

    /// Indices into `map`'s entity array that are above `entity`.
    ///
    /// ⚠ **What removing an entity would move.** An entity's position is its
    /// identity, so deleting element `i` shifts every index above it and each
    /// of them then names its neighbour. Returned as local and cross-file
    /// separately, because only the second kind is in another file and
    /// therefore invisible to a per-file check.
    pub fn indices_above(&self, map: u32, entity: u32) -> (u64, u64) {
        let mut local = 0u64;
        let mut cross_file = 0u64;
        for (&name, summary) in &self.by_name {
            for link in &summary.links {
                let owner = if link.in_parent_map {
                    summary.parent
                } else {
                    name
                };
                if owner != map || link.index <= entity {
                    continue;
                }
                if link.in_parent_map {
                    cross_file += 1;
                } else {
                    local += 1;
                }
            }
        }
        (local, cross_file)
    }

    /// Names claimed by two parts that both hold entities.
    ///
    /// ⚠ Not the grass case, which is normal and merged. This is an
    /// ambiguity about whose index space is whose.
    pub fn collisions(&self) -> &[Collision] {
        &self.duplicates
    }

    /// How many files were folded into the group's maps.
    pub fn part_count(&self) -> u64 {
        self.by_name.values().map(|s| s.parts as u64).sum()
    }

    /// The number of entities the group has seen, across every member.
    pub fn entity_count(&self) -> u64 {
        self.by_name.values().map(|s| s.entity_count as u64).sum()
    }

    /// How many cross-file links each candidate index space accounts for,
    /// measured side by side (see [`IndexSpaceFit`]).
    pub fn index_space_fit(&self) -> IndexSpaceFit {
        // Children per parent, so the merged-array candidate can be sized
        // without a second pass over the links.
        let mut merged_len: HashMap<u32, u64> = HashMap::new();
        for (&name, summary) in &self.by_name {
            *merged_len.entry(name).or_default() += summary.entity_count as u64;
            if summary.parent != 0 {
                *merged_len.entry(summary.parent).or_default() += summary.entity_count as u64;
            }
        }

        let mut fit = IndexSpaceFit::default();
        let mut empty_parents: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for summary in self.by_name.values() {
            let Some(parent) = self.by_name.get(&summary.parent) else {
                continue;
            };
            for link in summary.links.iter().filter(|l| l.in_parent_map) {
                fit.total += 1;
                if parent.entity_count == 0 {
                    fit.outside_with_empty_parent += 1;
                    empty_parents.insert(summary.parent);
                }
                let index = link.index as u64;
                if index < parent.entity_count as u64 {
                    fit.parent_own += 1;
                }
                if index < merged_len.get(&summary.parent).copied().unwrap_or(0) {
                    fit.parent_merged += 1;
                }
                if index < self.chain_len(summary.parent) {
                    fit.parent_chain += 1;
                }
                if index < summary.entity_count as u64 {
                    fit.own_array += 1;
                }
                let overflow = index.saturating_sub(parent.entity_count as u64);
                if overflow > 0 {
                    fit.overflow_sum += overflow;
                    fit.worst_overflow = fit.worst_overflow.max(overflow);
                }
            }
        }
        fit.empty_parents = empty_parents.len() as u64;
        fit
    }

    /// The links that reach furthest past the parent's own array.
    pub fn worst_overflows(&self, take: usize) -> Vec<Overflow> {
        let mut out: Vec<Overflow> = Vec::new();
        for (&name, summary) in &self.by_name {
            let Some(parent) = self.by_name.get(&summary.parent) else {
                continue;
            };
            for link in summary.links.iter().filter(|l| l.in_parent_map) {
                if link.index < parent.entity_count {
                    continue;
                }
                out.push(Overflow {
                    map: name,
                    map_entities: summary.entity_count,
                    entity: link.entity,
                    index: link.index,
                    parent: summary.parent,
                    parent_entities: parent.entity_count,
                });
            }
        }
        out.sort_by_key(|o| std::cmp::Reverse(o.index - o.parent_entities));
        out.truncate(take);
        out
    }

    /// Entities in a map and every map that names it as parent, one level down.
    fn chain_len(&self, root: u32) -> u64 {
        let mut total = self.by_name.get(&root).map_or(0, |s| s.entity_count as u64);
        // One level of descendants is what a merge reaches; deeper would
        // need the tree.
        for summary in self.by_name.values() {
            if summary.parent == root {
                total += summary.entity_count as u64;
            }
        }
        total
    }

    /// The cross-file facts a corpus sweep agrees with.
    ///
    /// ⚠ **Enforceable, and measured to be.** Over a mounted retail Legacy
    /// install — 10,603 maps assembled from 12,783 files, 1,689,873
    /// entities — this returns nothing. It leaves out
    /// [`GroupProblem::ChildCountOverstated`], which the game itself ships.
    ///
    /// ⚠ **It is still not a write gate**: a group needs every member and a
    /// save has one document, so [`crate::write::refuse_if_broken`] stays
    /// per-file. A pre-existing disagreement three files away must not block
    /// an unrelated edit.
    ///
    /// ⚠ A member whose parent is absent is reported once, and its entities
    /// are then left alone: a partial group must not produce one complaint
    /// per entity for a file the caller simply did not load.
    pub fn check(&self) -> Vec<GroupProblem> {
        let mut problems: Vec<GroupProblem> = self
            .duplicates
            .iter()
            .map(|c| GroupProblem::DuplicateName { map: c.map })
            .collect();

        for (&name, summary) in &self.by_name {
            if summary.parent != 0 && !self.by_name.contains_key(&summary.parent) {
                problems.push(GroupProblem::ParentMapMissing {
                    map: name,
                    parent: summary.parent,
                });
                continue;
            }
            let parent = self.by_name.get(&summary.parent);
            for link in summary.links.iter().filter(|l| l.in_parent_map) {
                match parent {
                    None => problems.push(GroupProblem::ParentIndexWithoutParentMap {
                        map: name,
                        entity: link.entity,
                    }),
                    Some(parent) if link.index >= parent.entity_count => {
                        problems.push(GroupProblem::ParentIndexOutOfRange {
                            map: name,
                            entity: link.entity,
                            parent_map: summary.parent,
                            index: link.index,
                            len: parent.entity_count,
                        })
                    }
                    Some(_) => {}
                }
            }
        }

        // ⚠ Only for a group with every parent present. A partial group is
        // missing exactly the members whose links would account for the
        // counts, so judging them there reports the caller's selection.
        if problems
            .iter()
            .all(|p| !matches!(p, GroupProblem::ParentMapMissing { .. }))
        {
            problems.extend(
                self.child_count_disagreements()
                    .into_iter()
                    .filter(|p| matches!(p, GroupProblem::ChildCountUnderstated { .. })),
            );
        }

        for &name in self.by_name.keys() {
            let mut at = name;
            for _ in 0..self.by_name.len() {
                match self.by_name.get(&at).map(|s| s.parent) {
                    Some(next) if next != 0 && next != at && self.by_name.contains_key(&next) => {
                        at = next
                    }
                    _ => break,
                }
            }
            let still_going = self
                .by_name
                .get(&at)
                .map(|s| s.parent)
                .is_some_and(|p| p != 0 && p != at && self.by_name.contains_key(&p));
            if still_going {
                problems.push(GroupProblem::ParentMapCycle { map: name });
                break;
            }
        }

        problems
    }

    /// How many cross-file LOD links land inside the parent map's own
    /// `entities` array, and how many do not.
    ///
    /// ⚠ **The model is settled and the second number is a defect count.**
    /// A cross-file `parentIndex` is a position in the parent map's own
    /// array, where "the parent map" is every file declaring that name.
    /// Merging those parts takes the fit from 42.72% to **707,456 of
    /// 707,456** over a mounted retail Legacy install, with nothing left
    /// over. Two readings are refuted: checking the index locally condemns
    /// 478,261 entities, and grouping without merging parts condemns a
    /// further 401,888 — every one of which pointed into a grass placement
    /// part that had replaced its own map.
    pub fn unresolved_links(&self) -> (u64, u64) {
        let mut inside = 0u64;
        let mut outside = 0u64;
        for summary in self.by_name.values() {
            let Some(parent) = self.by_name.get(&summary.parent) else {
                continue;
            };
            for link in summary.links.iter().filter(|l| l.in_parent_map) {
                if link.index < parent.entity_count {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        (inside, outside)
    }

    /// Children named by anyone in the group, per `(map, entity index)`.
    ///
    /// ⚠ Both kinds of link land here: a LOD parent's children are mostly in
    /// *other* files, so counting only local links accounts for a twelfth of
    /// them.
    fn children_by_parent(&self) -> BTreeMap<(u32, u32), u32> {
        let mut children: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for (&name, summary) in &self.by_name {
            for link in &summary.links {
                let (owner, len) = if link.in_parent_map {
                    match self.by_name.get(&summary.parent) {
                        Some(parent) => (summary.parent, parent.entity_count),
                        None => continue,
                    }
                } else {
                    (name, summary.entity_count)
                };
                if link.index < len {
                    *children.entry((owner, link.index)).or_default() += 1;
                }
            }
        }
        children
    }

    /// How many entities declare a LOD child count, and how many of those the
    /// group accounts for.
    pub fn declared_child_counts(&self) -> (u64, u64) {
        let children = self.children_by_parent();
        let mut declared = 0u64;
        let mut agreed = 0u64;
        for (&name, summary) in &self.by_name {
            for &(entity, count) in &summary.declared_children {
                declared += 1;
                if children.get(&(name, entity)).copied().unwrap_or(0) == count {
                    agreed += 1;
                }
            }
        }
        (declared, agreed)
    }

    /// Every entity whose declared child count the group disagrees with.
    ///
    /// ⚠ Both directions, and they are different problems — see
    /// [`GroupProblem::ChildCountUnderstated`]. Only the understating one
    /// reaches [`Self::check`].
    pub fn child_count_disagreements(&self) -> Vec<GroupProblem> {
        let children = self.children_by_parent();
        let mut out = Vec::new();
        for (&name, summary) in &self.by_name {
            let mut declared_here: BTreeMap<u32, u32> = BTreeMap::new();
            for &(entity, declared) in &summary.declared_children {
                declared_here.insert(entity, declared);
            }
            for (&entity, &declared) in &declared_here {
                let actual = children.get(&(name, entity)).copied().unwrap_or(0);
                out.extend(disagreement(name, entity, declared, actual));
            }
            // An entity with children and no declaration declares zero.
            for (&(map, entity), &actual) in &children {
                if map == name && !declared_here.contains_key(&entity) {
                    out.extend(disagreement(name, entity, 0, actual));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::{MetaInstance, Value};

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

    const IN_PARENT: u32 = 1 << 3;

    #[test]
    fn a_group_outlives_the_trees_it_was_built_from() {
        let mut group = MapGroup::new();
        for (name, parent) in [("hills_lod", None), ("hills_hd", Some("hills_lod"))] {
            let tree = map(name, parent, &[(-1, 0, 0)]);
            group.add(&MapData::new(&tree).unwrap());
        }
        assert_eq!(group.len(), 2);
        assert_eq!(group.entity_count(), 2);
        assert!(group.check().is_empty());
    }

    #[test]
    fn a_plain_entity_is_not_stored() {
        let tree = map("m", None, &[(-1, 0, 0), (-1, 0, 0), (-1, 0, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&tree).unwrap());
        assert_eq!(group.footprint(), std::mem::size_of::<(u32, Summary)>());
    }

    #[test]
    fn parts_sharing_a_name_are_merged_rather_than_replaced() {
        let entities = map("hills_lod", None, &[(-1, 1, 0), (-1, 0, 0)]);
        let grass = map("hills_lod", None, &[]);
        let hd = map("hills_hd", Some("hills_lod"), &[(0, 0, IN_PARENT)]);

        let mut group = MapGroup::new();
        group.add_from(&MapData::new(&entities).unwrap(), "metadata.ymap");
        group.add_from(&MapData::new(&grass).unwrap(), "grass_0.ymap");
        group.add_from(&MapData::new(&hd).unwrap(), "hd.ymap");

        assert_eq!(group.len(), 2, "two names, three parts");
        assert_eq!(group.part_count(), 3);
        assert!(group.collisions().is_empty(), "grass is not a conflict");
        assert_eq!(group.index_space_fit().parent_own, 1);
        assert!(group.check().is_empty(), "{:?}", group.check());
    }

    #[test]
    fn merging_shifts_local_indices_and_leaves_cross_file_ones() {
        let first = map("m", None, &[(-1, 1, 0), (0, 0, 0)]);
        let second = map("m", Some("p"), &[(-1, 1, 0), (0, 0, 0)]);
        let parent = map("p", None, &[(-1, 0, 0)]);

        let mut group = MapGroup::new();
        group.add(&MapData::new(&first).unwrap());
        group.add(&MapData::new(&second).unwrap());
        group.add(&MapData::new(&parent).unwrap());

        // The second part's local link now names entity 2, not entity 0, so
        // both declarations are satisfied. Without the shift the first
        // part's entity 0 would be credited twice and its sibling not at all.
        assert_eq!(group.declared_child_counts(), (2, 2));
        assert!(
            !group
                .check()
                .iter()
                .any(|p| matches!(p, GroupProblem::ChildCountUnderstated { .. })),
            "{:?}",
            group.check()
        );
        assert!(group
            .check()
            .iter()
            .any(|p| matches!(p, GroupProblem::DuplicateName { .. })));
    }

    #[test]
    fn a_cross_file_link_past_the_parents_end_is_refused() {
        let lod = map("hills_lod", None, &[(-1, 0, 0)]);
        let hd = map(
            "hills_hd",
            Some("hills_lod"),
            &[(0, 0, IN_PARENT), (7, 0, IN_PARENT)],
        );
        let mut group = MapGroup::new();
        group.add(&MapData::new(&lod).unwrap());
        group.add(&MapData::new(&hd).unwrap());

        assert_eq!(group.unresolved_links(), (1, 1));
        assert!(group.check().iter().any(|p| matches!(
            p,
            GroupProblem::ParentIndexOutOfRange {
                index: 7,
                len: 1,
                ..
            }
        )));
    }

    #[test]
    fn only_an_understated_child_count_is_refused() {
        let understated = map("a", None, &[(-1, 0, 0), (0, 0, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&understated).unwrap());
        assert!(group.check().iter().any(|p| matches!(
            p,
            GroupProblem::ChildCountUnderstated {
                declared: 0,
                actual: 1,
                ..
            }
        )));

        let overstated = map("b", None, &[(-1, 4, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&overstated).unwrap());
        assert_eq!(group.check(), vec![], "advisory, not refused");
        assert!(group.child_count_disagreements().iter().any(|p| matches!(
            p,
            GroupProblem::ChildCountOverstated {
                declared: 4,
                actual: 0,
                ..
            }
        )));
    }

    #[test]
    fn a_cross_file_link_with_no_parent_map_is_refused() {
        let orphan = map("hd_only", None, &[(3, 0, IN_PARENT)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&orphan).unwrap());
        assert!(group.check().iter().any(|p| matches!(
            p,
            GroupProblem::ParentIndexWithoutParentMap { entity: 0, .. }
        )));
    }

    #[test]
    fn an_absent_parent_is_reported_once_and_its_children_are_left_alone() {
        let hd = map(
            "hills_hd",
            Some("hills_lod"),
            &[(7, 0, IN_PARENT), (9, 0, IN_PARENT)],
        );
        let mut group = MapGroup::new();
        group.add(&MapData::new(&hd).unwrap());
        let problems = group.check();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(matches!(problems[0], GroupProblem::ParentMapMissing { .. }));
    }

    /// An overstated count lowers the agreement rate without being refused.
    #[test]
    fn declared_child_counts_are_reported_rather_than_judged() {
        let lod = map("hills_lod", None, &[(-1, 1, 0), (0, 0, 0)]);
        let wrong = map("other_lod", None, &[(-1, 4, 0)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&lod).unwrap());
        group.add(&MapData::new(&wrong).unwrap());
        assert_eq!(group.declared_child_counts(), (2, 1));
        assert_eq!(group.check(), vec![], "a disagreement is not a verdict");
    }

    #[test]
    fn a_consistent_group_has_nothing_to_report() {
        let lod = map("hills_lod", None, &[(-1, 1, 0)]);
        let hd = map("hills_hd", Some("hills_lod"), &[(0, 0, IN_PARENT)]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&lod).unwrap());
        group.add(&MapData::new(&hd).unwrap());
        assert_eq!(group.check(), vec![]);
    }

    #[test]
    fn a_parent_cycle_terminates() {
        let a = map("a", Some("b"), &[]);
        let b = map("b", Some("a"), &[]);
        let mut group = MapGroup::new();
        group.add(&MapData::new(&a).unwrap());
        group.add(&MapData::new(&b).unwrap());
        assert!(group
            .check()
            .iter()
            .any(|p| matches!(p, GroupProblem::ParentMapCycle { .. })));
    }
}
