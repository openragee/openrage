//! The merge of the entities of a map: what each side did to each entity of the base, the entities added, and the merged map laid
//! out with its links, its counts and its guids.

use super::entity::*;
use super::*;

/// One side of a merge of maps: its root, and what the comparison with the base says of it.
struct View<'a> {
    side: Side,
    base: &'a CMapData,
    root: &'a CMapData,
    changes: &'a MapChanges,
    /// For each entity of the base, the entity of this side it is.
    now_of: &'a [Option<usize>],
    changed: HashMap<usize, &'a Kept>,
    deleted: BTreeSet<usize>,
    /// How many entities of this side name each of its entities as their parent.
    own: Vec<i64>,
}

/// What a side did to an entity of the base that no group holds.
enum Class<'a> {
    Same,
    Changed(&'a Kept),
    Deleted,
}

impl<'a> View<'a> {
    fn new(
        side: Side,
        base: &'a CMapData,
        root: &'a CMapData,
        changes: &'a MapChanges,
        now_of: &'a [Option<usize>],
    ) -> Self {
        Self {
            side,
            base,
            root,
            changes,
            now_of,
            changed: (changes.changed.iter())
                .map(|k| (k.base.index, k))
                .collect(),
            deleted: changes.deleted.iter().map(|e| e.index).collect(),
            own: own_children(&root.entities),
        }
    }

    fn class(&self, base: usize) -> Class<'a> {
        match self.changed.get(&base) {
            Some(kept) if self.only_orphaned(kept) => Class::Same,
            Some(kept) => Class::Changed(kept),
            None if self.deleted.contains(&base) => Class::Deleted,
            None => Class::Same,
        }
    }

    /// Whether the entity names no parent on this side because this side deleted the parent it names in the base.
    ///
    /// ⚠ The removal took the link, and the side changed nothing of the entity by it: read as a change, it would leave the entity
    /// without a parent where a conflict keeps the parent, and make a conflict of a child the other side deleted.
    fn orphaned(&self, kept: &Kept) -> bool {
        matches!(link_of(self.entity(kept.now.index)), Link::None)
            && matches!(
                link_of(&self.base.entities[kept.base.index]),
                Link::Own(parent) if self.deleted.contains(&parent)
            )
    }

    /// Whether this side changed the parent `kept` names, by more than deleting it.
    fn changed_parent(&self, kept: &Kept) -> bool {
        changed_parent(kept) && !self.orphaned(kept)
    }

    /// Whether losing a parent this side deleted is all `kept` says this side changed.
    fn only_orphaned(&self, kept: &Kept) -> bool {
        self.orphaned(kept)
            && (kept.changes.iter()).all(|change| matches!(change, EntityChange::Parent { .. }))
    }

    /// The names of what this side changed of `kept`.
    fn labels(&self, kept: &Kept) -> Vec<String> {
        let mut said = labels(kept);
        if self.orphaned(kept) {
            said.retain(|member| member != PARENT);
        }
        said
    }

    fn entity(&self, index: usize) -> &'a CEntityDefAny {
        &self.root.entities[index]
    }

    fn named(&self, index: usize) -> Ref {
        entity_of(index, self.entity(index))
    }

    /// The children an entity of this side counts that its own file does not hold: what a child map adds.
    fn excess(&self, index: usize) -> i64 {
        i64::from(self.entity(index).num_children) - self.own[index]
    }
}

/// What the merged map holds at the place of an entity of the base.
enum Slot {
    /// The record of the base.
    Base,
    /// The record of an entity of a side, whole.
    Whole(Side, usize),
    /// A record merged member by member.
    Merged(Box<CEntityDefAny>),
    Gone,
}

/// Whose link and count of children an entity both hold takes.
#[derive(Clone, Copy)]
enum Links {
    /// What the comparison of each side says of them.
    Sides,
    /// Those of one entity of one side, whatever the other states.
    Only(Side, usize),
}

/// An entity a side added, which follows the entities of the base in the merged map.
struct Appended {
    side: Side,
    index: usize,
    /// The entity of the other side that it is too, where both added it and state the same.
    also: Option<usize>,
    guid: u32,
    /// Whether a conflict put it here, which then says so in its place.
    decided: bool,
    /// Whether it is a twin of the side taken for its group, more than the base holds of them.
    twin: bool,
}

/// The parent an entity of the merged map names, before the deletions renumber it.
#[derive(Debug, Clone, Copy)]
enum Target {
    /// No parent, with the number below zero that says so.
    None(i32),
    /// An entity of the parent map, which this file does not renumber.
    ParentMap(i32),
    /// An entity of the merged map, by its place before the deletions.
    Own(usize),
    /// An entity the merged map does not hold, as the side that named it indexes it: the other side deleted it.
    Lost(Side, usize),
}

impl Target {
    fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::None(_) | Self::Lost(..), Self::None(_) | Self::Lost(..)) => true,
            (Self::ParentMap(a), Self::ParentMap(b)) => a == b,
            (Self::Own(a), Self::Own(b)) => a == b,
            _ => false,
        }
    }
}

/// A change of an entity that the report lists, while the place it has in the merged map is not known yet.
struct Draft {
    applied: Applied,
    /// Its place before the deletions.
    slot: Option<usize>,
}

/// The lowest and the highest corner of two boxes together; a box whose corners are all zero states none.
fn union_of(a: ([f32; 3], [f32; 3]), b: ([f32; 3], [f32; 3])) -> ([f32; 3], [f32; 3]) {
    let none = |(min, max): &([f32; 3], [f32; 3])| min.iter().chain(max).all(|n| *n == 0.0);
    match (none(&a), none(&b)) {
        (true, _) => b,
        (_, true) => a,
        _ => (
            std::array::from_fn(|i| a.0[i].min(b.0[i])),
            std::array::from_fn(|i| a.1[i].max(b.1[i])),
        ),
    }
}

/// A guid none of `stated` is, by the rule a new entity of a map takes one, which `stated` then holds.
fn free_guid(stated: &mut BTreeSet<u32>) -> Option<u32> {
    let free = or_world_gta5::create::free_guid_among(stated)?;
    stated.insert(free);
    Some(free)
}

/// What the merged map holds once every conflict is answered, each entity at its place before the deletions.
struct Laid<'a> {
    base: &'a CMapData,
    sides: [&'a CMapData; 2],
    slots: &'a [Slot],
    appended: &'a [Appended],
    targets: &'a [Option<Target>],
    /// The children each entity counts that the merged map does not hold.
    excess: &'a [i64],
}

impl Laid<'_> {
    /// The record at a place, none where the entity of the base is gone.
    fn held(&self, place: usize) -> Option<&CEntityDefAny> {
        match self.slots.get(place) {
            Some(Slot::Base) => Some(&self.base.entities[place]),
            Some(Slot::Whole(side, now)) => Some(&self.sides[*side as usize].entities[*now]),
            Some(Slot::Merged(record)) => Some(record),
            Some(Slot::Gone) => None,
            None => {
                let added = self.appended.get(place - self.slots.len())?;
                Some(&self.sides[added.side as usize].entities[added.index])
            }
        }
    }

    /// The merged map, made of the base: what `other` changed of the records beside the entities and of what no typed member names, the members `file_members` name from their sides, then each entity added after the last, each record put in its place by the operation an edit puts one with, and the entities that are gone taken out in one pass, which renumbers the links after them.
    fn build(
        &self,
        other: Option<Side>,
        file_members: &[(&'static str, Side, Option<By>)],
    ) -> Result<CMapData, Error> {
        let count = self.slots.len();
        let root_of = |side: Option<Side>| match side {
            None => self.base,
            Some(side) => self.sides[side as usize],
        };
        let mut map = root_of(other).clone();
        map.entities = self.base.entities.clone();
        for member in [
            "name",
            "parent",
            "flags",
            "contentFlags",
            "physicsDictionaries",
        ] {
            let side = (file_members.iter())
                .find(|(taken, ..)| *taken == member)
                .map(|(_, side, _)| *side);
            let from = root_of(side);
            match member {
                "name" => map.name = from.name.clone(),
                "parent" => map.parent = from.parent.clone(),
                "flags" => map.flags = from.flags,
                "contentFlags" => map.content_flags = from.content_flags,
                _ => map.physics_dictionaries = from.physics_dictionaries.clone(),
            }
        }
        let source = String::new();
        let done = |map: &mut CMapData, operation: Operation| {
            apply_to_map(map, &operation).map_err(|why| {
                Error::Invalid(format!("{} could not be done: {why}", operation.name()))
            })
        };
        let record_of = |entity: &CEntityDefAny| {
            Record::entity(entity).map_err(|why| Error::Invalid(format!("an entity: {why}")))
        };

        // How many entities name each place as their parent, before the deletions: an entity that goes still names its parent of
        // the base, and the removal takes it off that parent's count.
        let mut children = vec![0i64; count + self.appended.len()];
        for (place, target) in self.targets.iter().enumerate() {
            let parent = match target {
                Some(Target::Own(parent)) => Some(*parent),
                Some(_) => None,
                None => match link_of(&self.base.entities[place]) {
                    Link::Own(parent) => Some(parent),
                    _ => None,
                },
            };
            if let Some(parent) = parent {
                children[parent] += 1;
            }
        }
        // The record at a place with the link and the count of children it has there.
        let linked = |place: usize| -> Result<Option<CEntityDefAny>, Error> {
            let Some(held) = self.held(place) else {
                return Ok(None);
            };
            let mut entity = held.clone();
            if let Some(added) = self.appended.get(place.wrapping_sub(count)) {
                entity.guid = added.guid;
            }
            let in_parent_map = entity.flags & IN_PARENT_MAP != 0;
            entity.parent_index = match self.targets[place] {
                Some(Target::ParentMap(index)) if in_parent_map => index,
                Some(Target::None(index)) if !in_parent_map => index,
                Some(Target::Lost(..)) if !in_parent_map => -1,
                Some(Target::Own(parent)) if !in_parent_map => {
                    i32::try_from(parent).map_err(|_| {
                        Error::Invalid(format!("entity {parent} is past what a LOD link can name"))
                    })?
                }
                _ => {
                    let named = match self.base.entities.get(place) {
                        Some(was) => format!(
                            "entity {place} of the base ({})",
                            entity_of(place, was).described()
                        ),
                        None => "an entity a side added".to_string(),
                    };
                    return Err(Error::Invalid(format!(
                        "{named} takes its flags from one side and its LOD parent from the other, and they do not agree on which map the parent is in"
                    )));
                }
            };
            let counted = (self.excess[place] + children[place]).clamp(0, i64::from(u32::MAX));
            entity.num_children = counted as u32;
            Ok(Some(entity))
        };
        // Each entity added goes after the last, where no link is renumbered, as the record its own file carries it as; the
        // links are stated below, once every place holds its entity.
        for place in count..count + self.appended.len() {
            let Some(record) = linked(place)? else {
                continue;
            };
            let carried = record_of(&record)?.as_entity();
            map.entities
                .push(carried.map_err(|why| Error::Invalid(format!("an entity: {why}")))?);
        }
        for place in 0..count {
            let Some(record) = linked(place)? else {
                continue;
            };
            // ⚠ By what a link is made of and not by `==` of the records, under which a zero that changed its sign is unchanged.
            let untouched = matches!(self.slots[place], Slot::Base) && {
                let was = &self.base.entities[place];
                record.parent_index == was.parent_index && record.num_children == was.num_children
            };
            if !untouched {
                let operation = Operation::ReplaceEntity {
                    source: source.clone(),
                    expects: None,
                    entity: place,
                    record: record_of(&record)?,
                    extents: None,
                };
                done(&mut map, operation)?;
            }
        }
        // ⚠ In one operation, and not one removal each: a removal walks the whole map. It refuses where a container LOD names
        // an entity that would move or go, as an edit does.
        let gone: Vec<usize> = (0..count)
            .filter(|&index| matches!(self.slots[index], Slot::Gone))
            .collect();
        if !gone.is_empty() {
            let operation = Operation::RemoveEntities {
                source: source.clone(),
                expects: None,
                entities: gone,
                extents: None,
            };
            done(&mut map, operation)?;
        }
        let boxes = |map: &CMapData| {
            let e = Extents::of(map);
            (
                (e.entities_extents_min, e.entities_extents_max),
                (e.streaming_extents_min, e.streaming_extents_max),
            )
        };
        let (a, b) = (boxes(self.sides[0]), boxes(self.sides[1]));
        let (entities, streaming) = (union_of(a.0, b.0), union_of(a.1, b.1));
        let extents = Extents {
            entities_extents_min: entities.0,
            entities_extents_max: entities.1,
            streaming_extents_min: streaming.0,
            streaming_extents_max: streaming.1,
        };
        done(
            &mut map,
            Operation::SetExtents {
                source: source.clone(),
                expects: None,
                extents,
            },
        )?;
        done(
            &mut map,
            Operation::GrowExtents {
                source,
                expects: None,
            },
        )?;
        Ok(map)
    }
}

/// The base, ours and theirs merged as one map, a conflict answered by `policy`; an error when the merged map would not be valid.
pub fn merge_maps(
    base: &CMapData,
    ours: &CMapData,
    theirs: &CMapData,
    policy: &mut dyn FnMut(&Conflict) -> Take,
) -> Result<Merge<CMapData>, Error> {
    let (changes_ours, now_ours) = map_changes_paired(base, ours);
    let (changes_theirs, now_theirs) = map_changes_paired(base, theirs);
    let views = [
        View::new(Side::Ours, base, ours, &changes_ours, &now_ours),
        View::new(Side::Theirs, base, theirs, &changes_theirs, &now_theirs),
    ];
    let view = |side: Side| &views[side as usize];
    let mut asked = Asked {
        policy,
        conflicts: Vec::new(),
        stopped: false,
    };
    let mut notes: Vec<String> = Vec::new();
    let count = base.entities.len();
    let in_base = |index: usize| entity_of(index, &base.entities[index]);
    // ⚠ A link of the base past its entities would name an entity a side added, once the merged map holds more.
    for (index, entity) in base.entities.iter().enumerate() {
        if let Link::Own(parent) = link_of(entity) {
            if parent >= count {
                return Err(Error::Input {
                    which: "the base",
                    why: format!(
                        "entity {index} names LOD parent {parent}, past the {count} entities it holds"
                    ),
                });
            }
        }
    }

    // What the file states of itself.
    let file_members = merge_file_members(
        &[
            "name",
            "parent",
            "flags",
            "contentFlags",
            "physicsDictionaries",
        ],
        &changes_ours.root,
        &changes_theirs.root,
        &mut asked,
    );
    // ⚠ The records a map types beside its entities are not merged inside: they go with what no typed member names, whole and
    // together from one side, and are named by the members that hold them.
    let mut rest: Vec<&str> = (crate::changes::RECORDS.into_iter())
        .filter(|member| {
            [&changes_ours, &changes_theirs]
                .iter()
                .any(|c| c.records.contains(member))
        })
        .collect();
    if changes_ours.other || changes_theirs.other || rest.is_empty() {
        rest.push(OTHER);
    }
    let rest = rest.join(", ");
    let other = other_from(
        &rest,
        changes_ours.rest_differs(),
        changes_theirs.rest_differs(),
        || !map_changes(ours, theirs).rest_differs(),
        &mut asked,
    );

    let mut slots: Vec<Slot> = (0..count).map(|_| Slot::Base).collect();
    let mut links = vec![Links::Sides; count];
    let mut decided = vec![false; count];
    // The side taken for the twins of a group, at each place of the base the group holds, and whether the entity of that side at
    // the place is one of its twins, which stands there by the order of its file alone.
    let mut twins_from: Vec<Option<Side>> = vec![None; count];
    let mut by_order = vec![false; count];
    let mut drafts: Vec<Draft> = Vec::new();
    // The entities merged member by member, each with the members no conflict decided: its links join them before it is listed.
    let mut merged_by_member: Vec<(usize, Vec<MemberBy>)> = Vec::new();
    // The entity of a side that stands where an entity of the base stood, where a group of twins pairs them by their order alone.
    let mut stands_at: [HashMap<usize, usize>; 2] = [HashMap::new(), HashMap::new()];
    let mut appended: Vec<Appended> = Vec::new();
    // An entity a side added that the merged map holds as the other side's: a double the policy decided.
    let mut held_as: Vec<(Side, usize, usize)> = Vec::new();

    // The twins: every group of one archetype that either comparison could not tell apart is one conflict.
    let mut groups: BTreeMap<u32, [Option<&crate::changes::Ambiguous>; 2]> = BTreeMap::new();
    for view in &views {
        for group in &view.changes.ambiguous {
            groups.entry(group.archetype.hash).or_default()[view.side as usize] = Some(group);
        }
    }
    for of_side in groups.into_values() {
        let of_base: BTreeSet<usize> = (of_side.iter().flatten())
            .flat_map(|group| group.base.iter().map(|e| e.index))
            .collect();
        // What a side holds in the place of these: the entities of its group, and the ones it was found to hold of the rest.
        let held = |side: Side| -> (Vec<usize>, Vec<(usize, usize)>, Vec<usize>) {
            let group = of_side[side as usize];
            let in_group: Vec<usize> =
                group.map_or(Vec::new(), |g| g.base.iter().map(|e| e.index).collect());
            let twins: Vec<usize> =
                group.map_or(Vec::new(), |g| g.now.iter().map(|e| e.index).collect());
            let found: Vec<(usize, usize)> = (of_base.iter())
                .filter(|index| !in_group.contains(index))
                .filter_map(|&index| Some((index, view(side).now_of[index]?)))
                .collect();
            (in_group, found, twins)
        };
        let names = |side: Side| -> Vec<Ref> {
            let (_, found, twins) = held(side);
            let mut all: Vec<usize> = found.iter().map(|(_, now)| *now).chain(twins).collect();
            all.sort_unstable();
            all.into_iter().map(|i| view(side).named(i)).collect()
        };
        let taken = asked.ask(
            Disagreement::Ambiguous,
            of_base.iter().map(|&i| in_base(i)).collect(),
            names(Side::Ours),
            names(Side::Theirs),
        );
        for &index in &of_base {
            decided[index] = true;
            slots[index] = Slot::Gone;
            twins_from[index] = Some(taken);
        }
        let (in_group, found, twins) = held(taken);
        for (index, now) in found {
            slots[index] = Slot::Whole(taken, now);
            links[index] = Links::Only(taken, now);
        }
        // ⚠ Which twin is which is not known: the twins of the side taken stand in the places of the base's in the order of
        // their files, so that no entity after them changes its index, and no member of one is merged with another's.
        for (at, &now) in twins.iter().enumerate() {
            match in_group.get(at) {
                Some(&index) => {
                    slots[index] = Slot::Whole(taken, now);
                    links[index] = Links::Only(taken, now);
                    by_order[index] = true;
                }
                None => appended.push(Appended {
                    side: taken,
                    index: now,
                    also: None,
                    guid: view(taken).entity(now).guid,
                    decided: true,
                    twin: true,
                }),
            }
        }
        let (in_group, _, twins) = held(taken.other());
        for (&index, &now) in in_group.iter().zip(&twins) {
            stands_at[taken.other() as usize].insert(now, index);
        }
    }

    // The entities both comparisons found.
    for index in 0..count {
        if decided[index] {
            continue;
        }
        let whole = |kept: &Kept, by: By| Draft {
            applied: Applied {
                did: Did::Changed,
                by,
                base: Some(in_base(index)),
                ours: (views[0].now_of[index]).map(|i| views[0].named(i)),
                theirs: (views[1].now_of[index]).map(|i| views[1].named(i)),
                merged: None,
                members: (views[usize::from(by == By::Theirs)]
                    .labels(kept)
                    .into_iter())
                .map(|member| MemberBy { member, by })
                .collect(),
                guid: None,
            },
            slot: Some(index),
        };
        let deleted = |by: By| Draft {
            applied: Applied {
                did: Did::Deleted,
                by,
                base: Some(in_base(index)),
                ours: (views[0].now_of[index]).map(|i| views[0].named(i)),
                theirs: (views[1].now_of[index]).map(|i| views[1].named(i)),
                merged: None,
                members: Vec::new(),
                guid: None,
            },
            slot: None,
        };
        match (views[0].class(index), views[1].class(index)) {
            (Class::Same, Class::Same) => {}
            (Class::Changed(kept), Class::Same) => {
                slots[index] = Slot::Whole(Side::Ours, kept.now.index);
                drafts.push(whole(kept, By::Ours));
            }
            (Class::Same, Class::Changed(kept)) => {
                slots[index] = Slot::Whole(Side::Theirs, kept.now.index);
                drafts.push(whole(kept, By::Theirs));
            }
            (Class::Deleted, Class::Same) => {
                slots[index] = Slot::Gone;
                drafts.push(deleted(By::Ours));
            }
            (Class::Same, Class::Deleted) => {
                slots[index] = Slot::Gone;
                drafts.push(deleted(By::Theirs));
            }
            (Class::Deleted, Class::Deleted) => {
                slots[index] = Slot::Gone;
                drafts.push(deleted(By::Both));
            }
            (Class::Changed(kept), Class::Deleted) | (Class::Deleted, Class::Changed(kept)) => {
                let changer = match views[0].class(index) {
                    Class::Changed(_) => Side::Ours,
                    _ => Side::Theirs,
                };
                let named = vec![view(changer).named(kept.now.index)];
                let (of_ours, of_theirs) = match changer {
                    Side::Ours => (named, Vec::new()),
                    Side::Theirs => (Vec::new(), named),
                };
                let taken = asked.ask(
                    Disagreement::ChangedAndDeleted {
                        deleted_by: changer.other(),
                        changed: view(changer).labels(kept),
                    },
                    vec![in_base(index)],
                    of_ours,
                    of_theirs,
                );
                if taken == changer {
                    slots[index] = Slot::Whole(changer, kept.now.index);
                    links[index] = Links::Only(changer, kept.now.index);
                } else {
                    slots[index] = Slot::Gone;
                }
            }
            (Class::Changed(of_ours), Class::Changed(of_theirs)) => {
                let (a, b) = (
                    views[0].entity(of_ours.now.index),
                    views[1].entity(of_theirs.now.index),
                );
                let was = &base.entities[index];
                let named = || {
                    (
                        vec![in_base(index)],
                        vec![views[0].named(of_ours.now.index)],
                        vec![views[1].named(of_theirs.now.index)],
                    )
                };
                let by_member = member_wise(of_ours)
                    && member_wise(of_theirs)
                    && is_interior(a) == is_interior(was)
                    && is_interior(b) == is_interior(was);
                if by_member {
                    let mut ask = |what: Disagreement| {
                        let (base, ours, theirs) = named();
                        asked.ask(what, base, ours, theirs)
                    };
                    let (record, members) =
                        merge_record(was, (a, of_ours), (b, of_theirs), &mut ask);
                    slots[index] = Slot::Merged(Box::new(record));
                    merged_by_member.push((index, members));
                } else if same_record(a, b) {
                    slots[index] = Slot::Whole(Side::Ours, of_ours.now.index);
                    drafts.push(whole(of_ours, By::Both));
                } else {
                    let (base, ours, theirs) = named();
                    let what = Disagreement::Record {
                        ours: views[0].labels(of_ours),
                        theirs: views[1].labels(of_theirs),
                    };
                    let taken = asked.ask(what, base, ours, theirs);
                    let now = match taken {
                        Side::Ours => of_ours.now.index,
                        Side::Theirs => of_theirs.now.index,
                    };
                    slots[index] = Slot::Whole(taken, now);
                    links[index] = Links::Only(taken, now);
                }
            }
        }
    }

    // The entities a side added: two of one archetype at one position are a likely double.
    let mut placed: BTreeMap<(u32, [u32; 3]), [Vec<usize>; 2]> = BTreeMap::new();
    for view in &views {
        for added in &view.changes.added {
            let entity = view.entity(added.index);
            let pose = pose_key(entity);
            let key = (entity.archetype_name.hash, [pose[0], pose[1], pose[2]]);
            placed.entry(key).or_default()[view.side as usize].push(added.index);
        }
    }
    for [of_ours, of_theirs] in placed.into_values() {
        let stand_alone = |side: Side, index: usize| {
            !matches!(link_of(view(side).entity(index)), Link::Own(_)) && view(side).own[index] == 0
        };
        let one = |side: Side, index: usize| Appended {
            side,
            index,
            also: None,
            guid: view(side).entity(index).guid,
            decided: false,
            twin: false,
        };
        if of_ours.is_empty() || of_theirs.is_empty() {
            appended.extend(of_ours.iter().map(|&i| one(Side::Ours, i)));
            appended.extend(of_theirs.iter().map(|&i| one(Side::Theirs, i)));
            continue;
        }
        // ⚠ Kept once only when the two state the same of every member, their links and their counts included, and stand alone
        // in their own files: a link is an index of one file, and nothing says the two name one parent.
        let agree = of_ours.len() == of_theirs.len()
            && of_ours.iter().zip(&of_theirs).all(|(&i, &j)| {
                let (a, b) = (views[0].entity(i), views[1].entity(j));
                same_record(a, b)
                    && a.parent_index == b.parent_index
                    && a.num_children == b.num_children
                    && stand_alone(Side::Ours, i)
                    && stand_alone(Side::Theirs, j)
            });
        if agree {
            for (&i, &j) in of_ours.iter().zip(&of_theirs) {
                appended.push(Appended {
                    also: Some(j),
                    ..one(Side::Ours, i)
                });
            }
            continue;
        }
        let mut differ: Vec<String> = Vec::new();
        let mut say = |member: &str| {
            if !differ.iter().any(|said| said == member) {
                differ.push(member.to_string());
            }
        };
        if of_ours.len() != of_theirs.len() {
            say("how many each side added there");
        }
        for (&i, &j) in of_ours.iter().zip(&of_theirs) {
            let (a, b) = (views[0].entity(i), views[1].entity(j));
            parts_that_differ(a, b)
                .iter()
                .for_each(|member| say(member));
            // ⚠ A link is an index of its own file: two that name a parent or count a child never agree, whatever number each states.
            if a.parent_index != b.parent_index
                || [link_of(a), link_of(b)]
                    .iter()
                    .any(|l| matches!(l, Link::Own(_)))
            {
                say(PARENT);
            }
            if a.num_children != b.num_children || views[0].own[i] != 0 || views[1].own[j] != 0 {
                say(COUNT);
            }
        }
        let taken = asked.ask(
            Disagreement::Double { differ },
            Vec::new(),
            of_ours.iter().map(|&i| views[0].named(i)).collect(),
            of_theirs.iter().map(|&i| views[1].named(i)).collect(),
        );
        let (kept, left) = match taken {
            Side::Ours => (&of_ours, &of_theirs),
            Side::Theirs => (&of_theirs, &of_ours),
        };
        appended.extend(kept.iter().map(|&i| Appended {
            decided: true,
            ..one(taken, i)
        }));
        held_as
            .extend((left.iter().zip(kept)).map(|(&gone, &stands)| (taken.other(), gone, stands)));
    }
    // ⚠ The order must not depend on which side is called ours: two entities a side each added never share an archetype and a
    // position here, so what an entity states orders them, and only two of one side are left to the order of their file.
    appended.sort_by_cached_key(|added| {
        let entity = view(added.side).entity(added.index);
        (
            entity.archetype_name.hash,
            pose_key(entity),
            entity.guid,
            entity.flags,
            entity.lod_dist.to_bits(),
            added.side as usize,
            added.index,
        )
    });

    // When one side states what the base states, the result is the other side as it is.
    let exact = [Side::Ours, Side::Theirs].into_iter().find(|&side| {
        let untouched = match side {
            Side::Ours => theirs == base,
            Side::Theirs => ours == base,
        };
        untouched && !asked.stopped && asked.all_took(side)
    });

    // A guid of its own for each entity added: free in the merged map, and in the base, whose deleted entities a comparison would
    // else find again in it.
    if exact.is_none() {
        let mut held: BTreeSet<u32> = base.entities.iter().map(|e| e.guid).collect();
        held.extend(slots.iter().enumerate().filter_map(|(index, slot)| {
            Some(match slot {
                Slot::Base => base.entities[index].guid,
                Slot::Whole(side, now) => view(*side).entity(*now).guid,
                Slot::Merged(record) => record.guid,
                Slot::Gone => return None,
            })
        }));
        let mut times: HashMap<u32, usize> = HashMap::new();
        for added in &appended {
            *times.entry(added.guid).or_default() += 1;
        }
        let mut stated = held.clone();
        stated.extend(ours.entities.iter().map(|e| e.guid));
        stated.extend(theirs.entities.iter().map(|e| e.guid));
        for added in &mut appended {
            if held.contains(&added.guid) || times.get(&added.guid).is_some_and(|n| *n > 1) {
                added.guid = free_guid(&mut stated).ok_or_else(|| {
                    Error::Invalid("no guid is free for an entity that was added".to_string())
                })?;
            }
        }
    }

    // Where each entity of a side is in the merged map, before the deletions.
    let mut place_of: [HashMap<usize, usize>; 2] = [HashMap::new(), HashMap::new()];
    for view in &views {
        let places = &mut place_of[view.side as usize];
        for (index, now) in view.now_of.iter().enumerate() {
            if let (Some(now), false) = (now, matches!(slots[index], Slot::Gone)) {
                places.insert(*now, index);
            }
        }
        for (now, index) in &stands_at[view.side as usize] {
            if !matches!(slots[*index], Slot::Gone) {
                places.entry(*now).or_insert(*index);
            }
        }
    }
    for (index, slot) in slots.iter().enumerate() {
        if let Slot::Whole(side, now) = slot {
            place_of[*side as usize].insert(*now, index);
        }
    }
    for (at, added) in appended.iter().enumerate() {
        place_of[added.side as usize].insert(added.index, count + at);
        if let Some(also) = added.also {
            place_of[added.side.other() as usize].insert(also, count + at);
        }
    }
    for (side, gone, stands) in held_as {
        if let Some(&place) = place_of[side.other() as usize].get(&stands) {
            place_of[side as usize].insert(gone, place);
        }
    }

    // The link of each entity, through the places.
    let target_of = |side: Side, now: usize| -> Result<Target, Error> {
        let view = view(side);
        let entity = view.entity(now);
        Ok(match link_of(entity) {
            Link::None => Target::None(entity.parent_index),
            Link::ParentMap(index) => Target::ParentMap(index),
            Link::Own(parent) if parent >= view.root.entities.len() => {
                return Err(Error::Input {
                    which: side.word(),
                    why: format!(
                        "entity {now} names LOD parent {parent}, past the {} entities it holds",
                        view.root.entities.len()
                    ),
                })
            }
            Link::Own(parent) => match place_of[side as usize].get(&parent) {
                Some(&place) => Target::Own(place),
                None => Target::Lost(side, parent),
            },
        })
    };
    let target_in_base = |index: usize| -> Target {
        let entity = &base.entities[index];
        match link_of(entity) {
            Link::None => Target::None(entity.parent_index),
            Link::ParentMap(parent) => Target::ParentMap(parent),
            // ⚠ Also when the parent is deleted: the removal takes the link with it, as it does in an edit.
            Link::Own(parent) => Target::Own(parent),
        }
    };
    // The entity of the base that each entity of a side is.
    let base_of: [HashMap<usize, usize>; 2] = [0, 1].map(|side| {
        (views[side].now_of.iter().enumerate())
            .filter_map(|(was, now)| Some(((*now)?, was)))
            .collect()
    });
    // An entity a side names that the merged map does not hold, as a sentence names it.
    let lost = |side: Side, parent: usize| match base_of[side as usize].get(&parent) {
        Some(&was) => format!(
            "entity {was} of the base ({}), which {}",
            in_base(was).described(),
            match view(side.other()).deleted.contains(&was) {
                true => format!("{} deleted", side.other().word()),
                false => "is not in the merged map".to_string(),
            }
        ),
        None => format!(
            "entity {parent} of its own file ({}), which is not in the merged map",
            view(side).named(parent).described()
        ),
    };
    let shown = |target: Target| match target {
        Target::None(_) => "none".to_string(),
        Target::Lost(side, parent) => lost(side, parent),
        Target::ParentMap(index) => format!("entity {index} of the parent map"),
        Target::Own(place) if place < count => {
            format!(
                "entity {place} of the base ({})",
                in_base(place).described()
            )
        }
        Target::Own(place) => {
            let added = &appended[place - count];
            format!(
                "entity {} that {} added ({})",
                added.index,
                added.side.word(),
                view(added.side).named(added.index).described()
            )
        }
    };
    // The link of an entity of the base that neither side gave another parent.
    //
    // ⚠ A comparison calls a link that stays among twins the same link, so under twins a side was taken for, the link is the one
    // that side states: its twins stand in the order of its own file, which may be another than the base's.
    let unchanged = |index: usize| -> Result<(Target, Option<Side>), Error> {
        if let Link::Own(parent) = link_of(&base.entities[index]) {
            let taken = twins_from.get(parent).copied().flatten();
            if let Some((side, now)) = taken.and_then(|s| Some((s, view(s).now_of[index]?))) {
                return Ok((target_of(side, now)?, Some(side)));
            }
        }
        Ok((target_in_base(index), None))
    };
    let own_in_base = own_children(&base.entities);
    // The side each link is from, none for a link of the base.
    let mut link_from: Vec<Option<Side>> = Vec::with_capacity(count + appended.len());
    let mut targets: Vec<Option<Target>> = Vec::with_capacity(count + appended.len());
    let mut excess: Vec<i64> = Vec::with_capacity(count + appended.len());
    let mut link_members: Vec<(usize, MemberBy)> = Vec::new();
    // How many links into the parent map were taken as each side changed them.
    let mut renumbered = [0usize; 2];
    for index in 0..count {
        if matches!(slots[index], Slot::Gone) {
            targets.push(None);
            link_from.push(None);
            excess.push(0);
            continue;
        }
        let kept = [views[0].changed.get(&index), views[1].changed.get(&index)];
        let now = |side: Side| kept[side as usize].map(|k| k.now.index);
        let named = || {
            (
                vec![in_base(index)],
                (now(Side::Ours).map(|i| views[0].named(i)))
                    .into_iter()
                    .collect::<Vec<_>>(),
                (now(Side::Theirs).map(|i| views[1].named(i)))
                    .into_iter()
                    .collect::<Vec<_>>(),
            )
        };
        let ((target, from_side), counted) = match links[index] {
            Links::Only(side, now) => {
                let orphaned = kept[side as usize].is_some_and(|k| view(side).orphaned(k));
                let target = match orphaned {
                    true => (target_in_base(index), None),
                    false => (target_of(side, now)?, Some(side)),
                };
                (target, view(side).excess(now))
            }
            Links::Sides => {
                let of = |side: Side| -> Result<Option<Target>, Error> {
                    match kept[side as usize].filter(|k| view(side).changed_parent(k)) {
                        Some(k) => target_of(side, k.now.index).map(Some),
                        None => Ok(None),
                    }
                };
                let (a, b) = (of(Side::Ours)?, of(Side::Theirs)?);
                // ⚠ A link into the parent map is a place in another file, which a deletion there renumbers in every map under it:
                // two sides that both changed one agree in nothing by stating one number, since the merged parent map may be
                // numbered as neither side's is.
                let into_parent_map = matches!(
                    (a, b),
                    (Some(Target::ParentMap(_)), Some(Target::ParentMap(_)))
                );
                let from = three_way(
                    a.is_some(),
                    b.is_some(),
                    || !into_parent_map && matches!((a, b), (Some(a), Some(b)) if a.same(b)),
                    || {
                        let (base, ours, theirs) = named();
                        let said = (
                            a.map(shown).unwrap_or_default(),
                            b.map(shown).unwrap_or_default(),
                        );
                        let what = match into_parent_map {
                            true => Disagreement::ParentMapLink {
                                ours: said.0,
                                theirs: said.1,
                            },
                            false => Disagreement::Member {
                                member: PARENT.to_string(),
                                ours: said.0,
                                theirs: said.1,
                            },
                        };
                        asked.ask(what, base, ours, theirs)
                    },
                );
                let taken = from.map(|(side, _)| (side, [a, b][side as usize]));
                if let Some((side, Some(Target::ParentMap(_)))) = taken {
                    renumbered[side as usize] += 1;
                }
                let target = match (from, a, b) {
                    (Some((Side::Ours, _)), Some(a), _) => (a, Some(Side::Ours)),
                    (Some((Side::Theirs, _)), _, Some(b)) => (b, Some(Side::Theirs)),
                    _ => unchanged(index)?,
                };
                if let (Some((_, Some(by))), Slot::Merged(_)) = (from, &slots[index]) {
                    let member = PARENT.to_string();
                    link_members.push((index, MemberBy { member, by }));
                }
                let of = |side: Side| kept[side as usize].filter(|k| changed_count(k));
                let (a, b) = (of(Side::Ours), of(Side::Theirs));
                let stated =
                    |side: Side, kept: Option<&&Kept>| kept.map(|k| view(side).excess(k.now.index));
                let (a, b) = (stated(Side::Ours, a), stated(Side::Theirs, b));
                let from = three_way(
                    a.is_some(),
                    b.is_some(),
                    || a == b,
                    || {
                        let (base, ours, theirs) = named();
                        let count_of = |side: Side| {
                            now(side).map_or(String::new(), |i| {
                                view(side).entity(i).num_children.to_string()
                            })
                        };
                        let what = Disagreement::Member {
                            member: COUNT.to_string(),
                            ours: count_of(Side::Ours),
                            theirs: count_of(Side::Theirs),
                        };
                        asked.ask(what, base, ours, theirs)
                    },
                );
                let counted = match from {
                    None => i64::from(base.entities[index].num_children) - own_in_base[index],
                    Some((Side::Ours, _)) => a.unwrap_or(0),
                    Some((Side::Theirs, _)) => b.unwrap_or(0),
                };
                if let (Some((_, Some(by))), Slot::Merged(_)) = (from, &slots[index]) {
                    let member = COUNT.to_string();
                    link_members.push((index, MemberBy { member, by }));
                }
                (target, counted)
            }
        };
        targets.push(Some(target));
        link_from.push(from_side);
        excess.push(counted);
    }
    for added in &appended {
        targets.push(Some(target_of(added.side, added.index)?));
        link_from.push(Some(added.side));
        excess.push(view(added.side).excess(added.index));
    }
    let listed: HashMap<usize, usize> = (merged_by_member.iter().enumerate())
        .map(|(at, (index, _))| (*index, at))
        .collect();
    for (index, member) in link_members {
        if let Some(&at) = listed.get(&index) {
            merged_by_member[at].1.push(member);
        }
    }
    for (index, members) in merged_by_member {
        if members.is_empty() {
            continue;
        }
        let named = |side: Side| {
            (view(side).changed.get(&index)).map(|kept| view(side).named(kept.now.index))
        };
        drafts.push(Draft {
            applied: Applied {
                did: Did::Changed,
                by: By::Both,
                base: Some(in_base(index)),
                ours: named(Side::Ours),
                theirs: named(Side::Theirs),
                merged: None,
                members,
                guid: None,
            },
            slot: Some(index),
        });
    }
    drafts.sort_by_key(|draft| match &draft.applied.base {
        Some(base) => (0, base.index()),
        None => (1, 0),
    });

    let mut gone_before = vec![0usize; count + 1];
    for (index, slot) in slots.iter().enumerate() {
        gone_before[index + 1] = gone_before[index] + usize::from(matches!(slot, Slot::Gone));
    }
    // The index an entity has in the merged map: its place less the deletions before it, or its index in the side that is the result.
    let merged_index = |place: usize| -> Option<usize> {
        match exact {
            None => Some(place - gone_before[place.min(count)]),
            Some(side) if place < count => {
                view(side).now_of[place].or_else(|| match slots[place] {
                    Slot::Whole(_, now) => Some(now),
                    _ => None,
                })
            }
            Some(_) => Some(appended[place - count].index),
        }
    };
    for (at, added) in appended.iter().enumerate() {
        if added.decided {
            continue;
        }
        let (near, far) = (
            Some(view(added.side).named(added.index)),
            added.also.map(|i| view(added.side.other()).named(i)),
        );
        let (of_ours, of_theirs) = match added.side {
            Side::Ours => (near, far),
            Side::Theirs => (far, near),
        };
        drafts.push(Draft {
            applied: Applied {
                did: Did::Added,
                by: match added.also {
                    Some(_) => By::Both,
                    None => added.side.into(),
                },
                base: None,
                ours: of_ours,
                theirs: of_theirs,
                merged: None,
                members: Vec::new(),
                guid: Some(added.guid),
            },
            slot: Some(count + at),
        });
    }
    // A merge that stopped made no file for a change to have an index in.
    let stopped = asked.stopped;
    let applied: Vec<Applied> = (drafts.into_iter())
        .map(|draft| Applied {
            merged: (draft.slot.filter(|_| !stopped)).and_then(merged_index),
            guid: draft.applied.guid.filter(|_| !stopped),
            ..draft.applied
        })
        .collect();

    let mut report = Report {
        ours: SourceChanges::Map(changes_ours.clone()),
        theirs: SourceChanges::Map(changes_theirs.clone()),
        applied,
        file: file_said(&file_members, other, &rest),
        conflicts: asked.conflicts,
        merged: !asked.stopped,
        not_compared: not_compared(true),
        notes: Vec::new(),
        findings: Vec::new(),
    };
    if asked.stopped {
        return Ok(Merge {
            merged: None,
            report,
        });
    }

    let merged = match exact {
        Some(side) => {
            notes.push(format!(
                "{} states what the base states, so the merged map is {} as it is",
                side.other().word(),
                side.word()
            ));
            view(side).root.clone()
        }
        None => {
            let laid = Laid {
                base,
                sides: [ours, theirs],
                slots: &slots,
                appended: &appended,
                targets: &targets,
                excess: &excess,
            };
            let root = root_from(other, &rest, base, [ours, theirs])?;
            let map = laid.build(root.map(|(side, _)| side), &file_members)?;
            // An entity of the merged map as a note names it.
            let here = |place: usize| -> Option<(usize, String)> {
                let index = merged_index(place)?;
                let entity = map.entities.get(index)?;
                let said = entity_of(index, entity).described();
                Some((index, format!("entity {index} of the merged map ({said})")))
            };
            // The entity of a side that stands at a place.
            let now_on = |side: Side, place: usize| -> Option<usize> {
                match appended.get(place.wrapping_sub(count)) {
                    Some(added) if added.side == side => Some(added.index),
                    Some(added) => added.also,
                    None => match links[place] {
                        Links::Only(only, now) if only == side => Some(now),
                        _ => view(side).now_of[place],
                    },
                }
            };
            for (place, target) in targets.iter().enumerate() {
                let Some((_, named)) = here(place) else {
                    continue;
                };
                match target {
                    Some(Target::Lost(side, parent)) => notes.push(format!(
                        "{named} names no LOD parent: {} named {}",
                        side.word(),
                        lost(*side, *parent)
                    )),
                    // An entity whose parent a side deleted is no news where that side left it without one itself.
                    Some(Target::Own(parent))
                        if matches!(slots.get(*parent), Some(Slot::Gone))
                            && !(views.iter())
                                .any(|v| v.changed.get(&place).is_some_and(|k| v.orphaned(k))) =>
                    {
                        notes.push(format!(
                            "{named} names no LOD parent: entity {parent} of the base ({}), which it named, is not in the merged map",
                            in_base(*parent).described()
                        ));
                    }
                    // ⚠ A link from one side to a twin the other side was taken for: which twin is which is the question the
                    // conflict asked, and the link follows the order of the files alone.
                    Some(Target::Own(parent)) => {
                        let Some(from) = link_from[place] else {
                            continue;
                        };
                        let twin_of = match appended.get(parent.wrapping_sub(count)) {
                            Some(added) => added.twin.then_some(added.side),
                            None => twins_from[*parent].filter(|_| by_order[*parent]),
                        };
                        let its_own = now_on(from, place)
                            .and_then(|now| match link_of(view(from).entity(now)) {
                                Link::Own(parent) => Some(parent),
                                _ => None,
                            })
                            .is_some_and(|parent| stands_at[from as usize].contains_key(&parent));
                        let taken = match (twin_of, its_own) {
                            (Some(taken), _) if taken != from => taken,
                            (None, true) => from.other(),
                            _ => continue,
                        };
                        if let Some((_, parent)) = here(*parent) {
                            notes.push(format!(
                                "{named} names {parent} as its LOD parent by the order of the files alone: {} named a twin, the conflict of the twins took {}, and nothing says which twin of {} that one is",
                                from.word(),
                                taken.word(),
                                taken.word()
                            ));
                        }
                    }
                    _ => {}
                }
            }
            for side in [Side::Ours, Side::Theirs] {
                let links = renumbered[side as usize];
                if links > 0 {
                    notes.push(format!(
                        "{links} link(s) into the parent map are as {} changed them: each names the right entity only where the merged parent map holds its entities in the order {}'s does, and this merge read no parent map",
                        side.word(),
                        side.word()
                    ));
                }
            }
            // An entity a conflict kept is in no list of what was applied, so the guid it was given is said here.
            for (at, added) in appended.iter().enumerate() {
                let was = view(added.side).entity(added.index).guid;
                if let (true, true, Some((_, named))) =
                    (added.decided, added.guid != was, here(count + at))
                {
                    notes.push(format!(
                        "{named} was given the guid {}: {} states {was} for it, which another entity of the merged map or of the base states",
                        added.guid,
                        added.side.word()
                    ));
                }
            }
            map
        }
    };
    let bytes = merged
        .to_bytes()
        .map_err(|why| Error::Invalid(why.to_string()))?;
    report.findings = judged(&bytes, CMapData::EXTENSION).map_err(|refusal| {
        refused_of_itself([("the base", base), ("ours", ours), ("theirs", theirs)])
            .unwrap_or(refusal)
    })?;
    report.notes = notes;
    Ok(Merge {
        merged: Some(merged),
        report,
    })
}
