//! What a map changed of its base, by object: the entities added, the entities deleted, and what each of the others states differently.
//!
//! [`map_changes`] compares two typed roots of one map, the base and the map as it is now, [`types_changes`] two roots of one type
//! file, and [`source_changes`] cooks a metadata source and the base it keeps and compares the two. Comparing two files by record and
//! field is the app's Compare with original; this says which object is which first, so an entity taken out of the middle of a map is
//! one deletion, whatever index every later entity has now.
//!
//! An entity of the base is the entity of the changed map that
//!
//! 1. states its guid, where one entity of each file states it ([`Matched::Guid`]);
//! 2. else places its archetype at its pose, to the bit ([`Matched::Place`]);
//! 3. else places its archetype nearest to it, each being the nearest of the other with no second as near ([`Matched::Nearest`]).
//!
//! ⚠ **Two entities of one file with one archetype and one guid are twins, and a twin is never said to have moved.** Shipped maps
//! hold such pairs. A twin that stands where it stood is itself. One that does not is listed in [`MapChanges::ambiguous`] with every
//! entity of the other file it may be, each by its index, and so is whatever step 3 leaves of an archetype on both sides.
//!
//! ⚠ Step 3 believes a place over a guid: an entity taken out and another of its archetype put in with a guid of its own, when they
//! are the only two of that archetype the guids leave, read as one entity moved with another guid. [`Kept::matched`] says which step
//! a pair is from, and [`EntityChange::Guid`] that the guid is another.
//!
//! The parent links are compared after the matching and through it: `parentIndex` is a position in the file's own array, which a
//! deletion renumbers, so two links are one link when the entities they name are one entity.
//!
//! What a file states of itself beside its entities or its archetypes (its flags, its extents, the files it depends on) is compared
//! member by member ([`MapChanges::root`], [`TypesChanges::root`]), and what no typed member names, such as a car generator, is said
//! to differ and not listed ([`MapChanges::other`]).

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::path::Path;

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{
    CBaseArchetypeDef, CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
};
use or_world::Placement;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::facts::Name;

/// The flag of an entity that names its parent in the parent map's array, which a change of this file does not renumber.
const IN_PARENT_MAP: u32 = 1 << 3;

/// For each member listed, its engine name beside it: `$differs`, a `&mut` closure, with the name and what each record states where they state another thing, and the member blanked in the two copies `$blank` holds, so that what the copies still differ in is what no member listed names.
macro_rules! members {
    ($base:expr, $now:expr, $blank:expr, { $($field:ident: $name:literal),* $(,)? }, $differs:expr) => {{
        let differs = $differs;
        let (was, is) = $blank;
        $(
            if !Member::same(&$base.$field, &$now.$field) {
                differs($name, Member::stated(&$base.$field), Member::stated(&$now.$field));
            }
            was.$field = Default::default();
            is.$field = Default::default();
        )*
    }};
}

fn name_of(name: &HashedString) -> Name {
    Name {
        hash: name.hash,
        text: (!name.raw.is_empty() && or_io::string_hash(name.raw.as_bytes()) == name.hash)
            .then(|| name.raw.clone()),
    }
}

fn name<S: Serializer>(name: &Name, to: S) -> Result<S::Ok, S::Error> {
    let mut out = to.serialize_struct("Name", 2)?;
    out.serialize_field("hash", &name.hash)?;
    out.serialize_field("text", &name.text)?;
    out.end()
}

/// An entity where a caller finds it: its index among the entities of its file, and what it states of itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityRef {
    pub index: usize,
    pub guid: u32,
    #[serde(serialize_with = "name")]
    pub archetype: Name,
    pub position: [f32; 3],
}

/// The step that said an entity of the base is an entity of the changed map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Matched {
    /// One entity of each file states the guid.
    Guid,
    /// The same archetype at the same pose, to the bit.
    Place,
    /// The same archetype, each the nearest of the other.
    Nearest,
}

/// One thing an entity both files hold states differently.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "what", rename_all = "kebab-case")]
pub enum EntityChange {
    Moved {
        from: [f32; 3],
        to: [f32; 3],
    },
    /// The rotation that places it, as `x, y, z, w`: the file stores its conjugate for an entity and the rotation itself for an interior.
    Turned {
        from: [f32; 4],
        to: [f32; 4],
    },
    /// `scaleXY` and `scaleZ`.
    Scaled {
        from: [f32; 2],
        to: [f32; 2],
    },
    Archetype {
        #[serde(serialize_with = "name")]
        from: Name,
        #[serde(serialize_with = "name")]
        to: Name,
    },
    LodDist {
        from: f32,
        to: f32,
    },
    Flags {
        from: u32,
        to: u32,
    },
    /// The entity named as the parent is another one: `from` is the `parentIndex` the base states and `to` the one the changed map states, each an index of its own file and -1 for none.
    Parent {
        from: i32,
        to: i32,
    },
    Guid {
        from: u32,
        to: u32,
    },
    /// Other members the typed record names, by the engine's names.
    Members {
        names: Vec<&'static str>,
    },
    /// What no typed member names differs (a level of detail, a priority, an extension), or the record is of another type.
    Other,
}

/// An entity both files hold, with what it states differently.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Kept {
    pub base: EntityRef,
    pub now: EntityRef,
    pub matched: Matched,
    pub changes: Vec<EntityChange>,
}

/// Entities of one archetype that cannot be told from a twin: any of `base` may be any of `now`, or gone, and any of `now` may be new.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ambiguous {
    #[serde(serialize_with = "name")]
    pub archetype: Name,
    pub base: Vec<EntityRef>,
    pub now: Vec<EntityRef>,
}

/// What the entities of a map changed of its base.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MapChanges {
    /// The entities of the changed map the base lacks.
    pub added: Vec<EntityRef>,
    /// The entities of the base the changed map lacks.
    pub deleted: Vec<EntityRef>,
    /// The entities both hold that state something else now.
    pub changed: Vec<Kept>,
    pub ambiguous: Vec<Ambiguous>,
    /// How many entities both hold that state what they stated.
    pub unchanged: usize,
    /// What the map states of itself differently, each member by the engine's name; empty for the entities of an interior.
    pub root: Vec<MemberChange>,
    /// Whether what no typed member of the map names differs: a car generator, an occluder, a light.
    pub other: bool,
}

impl MapChanges {
    /// Whether the two maps state the same of themselves and hold the same entities stating the same.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.deleted.is_empty()
            && self.changed.is_empty()
            && self.ambiguous.is_empty()
            && self.root.is_empty()
            && !self.other
    }
}

/// How the file stores the rotation of this entity.
fn placement_of(entity: &CEntityDefAny) -> Placement {
    match entity.as_mlo_instance() {
        Some(_) => Placement::InteriorInstance,
        None => Placement::Entity,
    }
}

/// The pose of an entity as the file stores it, bit for bit.
fn pose_key(entity: &CEntityDef) -> [u32; 9] {
    let (p, r) = (entity.position, entity.rotation);
    [
        p.x,
        p.y,
        p.z,
        r.x,
        r.y,
        r.z,
        r.w,
        entity.scale_xy,
        entity.scale_z,
    ]
    .map(f32::to_bits)
}

/// Every scalar an entity states of itself but its links to other entities: two entities with one key are one another's copies.
fn record_key(entity: &CEntityDefAny) -> ([u32; 9], [u32; 8]) {
    (
        pose_key(entity),
        [
            u32::from(entity.as_mlo_instance().is_some()),
            entity.guid,
            entity.flags,
            entity.lod_dist.to_bits(),
            entity.child_lod_dist.to_bits(),
            entity.ambient_occlusion_multiplier as u32,
            entity.artificial_ambient_occlusion as u32,
            entity.tint_value,
        ],
    )
}

/// The entities of `base` and `now` that share a key, paired in file order, and what is left of each list in file order; unless `several`, a key pairs only where one entity of each list has it.
fn pair_by<K: Hash + Eq>(
    base: Vec<usize>,
    now: Vec<usize>,
    key_of_base: impl Fn(usize) -> K,
    key_of_now: impl Fn(usize) -> K,
    several: bool,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let mut keyed: HashMap<K, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for index in base {
        keyed.entry(key_of_base(index)).or_default().0.push(index);
    }
    for index in now {
        keyed.entry(key_of_now(index)).or_default().1.push(index);
    }
    let (mut pairs, mut left_base, mut left_now) = (Vec::new(), Vec::new(), Vec::new());
    for (base, now) in keyed.into_values() {
        let take = if several || (base.len() == 1 && now.len() == 1) {
            base.len().min(now.len())
        } else {
            0
        };
        pairs.extend(
            base[..take]
                .iter()
                .copied()
                .zip(now[..take].iter().copied()),
        );
        left_base.extend_from_slice(&base[take..]);
        left_now.extend_from_slice(&now[take..]);
    }
    left_base.sort_unstable();
    left_now.sort_unstable();
    (pairs, left_base, left_now)
}

/// Where an entity stands, as the nearest one is judged.
struct At {
    position: Vec3,
    rotation: Quat,
    scale: [f32; 2],
}

fn at(entity: &CEntityDefAny) -> At {
    let r = entity.rotation;
    At {
        position: Vec3::new(entity.position.x, entity.position.y, entity.position.z),
        rotation: placement_of(entity).rotation_of([r.x, r.y, r.z, r.w]),
        scale: [entity.scale_xy, entity.scale_z],
    }
}

/// How far apart two entities stand: the distance squared, then the turn between them, then the scale between them.
fn apart(a: &At, b: &At) -> [f32; 3] {
    [
        a.position.distance_squared(b.position),
        1.0 - a.rotation.dot(b.rotation).abs(),
        (a.scale[0] - b.scale[0]).abs() + (a.scale[1] - b.scale[1]).abs(),
    ]
}

fn nearer(a: &[f32; 3], b: &[f32; 3]) -> std::cmp::Ordering {
    a[0].total_cmp(&b[0])
        .then(a[1].total_cmp(&b[1]))
        .then(a[2].total_cmp(&b[2]))
}

/// For each of `from`, the one of `to` that stands nearest to it; none where two of `to` are as near, `to` is empty or a position is no finite number.
///
/// ⚠ `to` is walked outward from the entity along the axis it is spread widest on, and left once that axis alone is further than the
/// nearest found, so the cost of one entity is the number of `to` that are as near along that axis, and not the number there are.
/// Entities stacked at one coordinate of that axis are each walked for each.
fn nearest(from: &[At], to: &[At]) -> Vec<Option<usize>> {
    let mut order: Vec<usize> = (0..to.len())
        .filter(|&i| to[i].position.is_finite())
        .collect();
    let (lo, hi) = order.iter().fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), &i| (lo.min(to[i].position), hi.max(to[i].position)),
    );
    let spread = hi - lo;
    let axis = match (
        spread.x >= spread.y,
        spread.x >= spread.z,
        spread.y >= spread.z,
    ) {
        (true, true, _) => 0,
        (false, _, true) => 1,
        _ => 2,
    };
    let along = |i: usize| to[i].position[axis];
    order.sort_by(|a, b| along(*a).total_cmp(&along(*b)));
    from.iter()
        .map(|entity| {
            if !entity.position.is_finite() {
                return None;
            }
            let here = entity.position[axis];
            // The entities before `left` and from `right` on are not visited yet, and each step takes the nearer of the two along the axis.
            let mut right = order.partition_point(|&i| along(i) < here);
            let mut left = right;
            let mut best: Option<([f32; 3], usize, bool)> = None;
            loop {
                let before = (left > 0).then(|| here - along(order[left - 1]));
                let after = (right < order.len()).then(|| along(order[right]) - here);
                let (index, gap) = match (before, after) {
                    (Some(before), Some(after)) if before <= after => {
                        left -= 1;
                        (order[left], before)
                    }
                    (_, Some(after)) => {
                        right += 1;
                        (order[right - 1], after)
                    }
                    (Some(before), None) => {
                        left -= 1;
                        (order[left], before)
                    }
                    (None, None) => break,
                };
                if best.is_some_and(|(found, ..)| gap * gap > found[0]) {
                    break;
                }
                let found = apart(entity, &to[index]);
                match &mut best {
                    None => best = Some((found, index, false)),
                    Some((held, at, tied)) => match nearer(&found, held) {
                        std::cmp::Ordering::Less => (*held, *at, *tied) = (found, index, false),
                        std::cmp::Ordering::Equal => *tied = true,
                        std::cmp::Ordering::Greater => {}
                    },
                }
            }
            best.and_then(|(_, index, tied)| (!tied).then_some(index))
        })
        .collect()
}

/// Which entity of the changed list each entity of the base is, and the groups nothing could tell apart.
struct Matching {
    now_of: Vec<Option<(usize, Matched)>>,
    base_of: Vec<Option<usize>>,
    /// The entities of the base and of the changed list in each ambiguous group.
    groups: Vec<(Vec<usize>, Vec<usize>)>,
    group_of_base: Vec<Option<usize>>,
    group_of_now: Vec<Option<usize>>,
}

impl Matching {
    fn pair(&mut self, base: usize, now: usize, by: Matched) {
        self.now_of[base] = Some((now, by));
        self.base_of[now] = Some(base);
    }
}

/// How many entities of `list` state each guid, with the index of the first that does.
fn guids_of(list: &[CEntityDefAny]) -> HashMap<u32, (usize, usize)> {
    let mut guids: HashMap<u32, (usize, usize)> = HashMap::new();
    for (index, entity) in list.iter().enumerate() {
        guids.entry(entity.guid).or_insert((0, index)).0 += 1;
    }
    guids
}

/// How many entities of `list` state each archetype and guid together: more than one are twins.
fn twins_of(list: &[CEntityDefAny]) -> HashMap<(u32, u32), usize> {
    let mut twins: HashMap<(u32, u32), usize> = HashMap::new();
    for entity in list {
        *twins
            .entry((entity.archetype_name.hash, entity.guid))
            .or_default() += 1;
    }
    twins
}

fn match_entities(base: &[CEntityDefAny], now: &[CEntityDefAny]) -> Matching {
    let mut matching = Matching {
        now_of: vec![None; base.len()],
        base_of: vec![None; now.len()],
        groups: Vec::new(),
        group_of_base: vec![None; base.len()],
        group_of_now: vec![None; now.len()],
    };
    let (guids_base, guids_now) = (guids_of(base), guids_of(now));
    let (twins_base, twins_now) = (twins_of(base), twins_of(now));
    for (index, entity) in base.iter().enumerate() {
        if let (Some((1, _)), Some((1, at))) =
            (guids_base.get(&entity.guid), guids_now.get(&entity.guid))
        {
            matching.pair(index, *at, Matched::Guid);
        }
    }

    // What the guids left, by archetype; a BTreeMap so the groups come out in one order.
    let mut rest: BTreeMap<u32, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (index, entity) in base.iter().enumerate() {
        if matching.now_of[index].is_none() {
            let of = rest.entry(entity.archetype_name.hash).or_default();
            of.0.push(index);
        }
    }
    for (index, entity) in now.iter().enumerate() {
        if matching.base_of[index].is_none() {
            let of = rest.entry(entity.archetype_name.hash).or_default();
            of.1.push(index);
        }
    }
    let twin = |twins: &HashMap<(u32, u32), usize>, entity: &CEntityDef| {
        twins
            .get(&(entity.archetype_name.hash, entity.guid))
            .is_some_and(|held| *held > 1)
    };
    for (left_base, left_now) in rest.into_values() {
        // Copies of one another are interchangeable, so they pair in file order and the surplus of one side is what was added or deleted.
        let (copies, left_base, left_now) = pair_by(
            left_base,
            left_now,
            |i| record_key(&base[i]),
            |i| record_key(&now[i]),
            true,
        );
        let (placed, left_base, left_now) = pair_by(
            left_base,
            left_now,
            |i| pose_key(&base[i]),
            |i| pose_key(&now[i]),
            false,
        );
        for (from, to) in copies.into_iter().chain(placed) {
            matching.pair(from, to, Matched::Place);
        }
        if left_base.is_empty() || left_now.is_empty() {
            continue;
        }
        let at_base: Vec<At> = left_base.iter().map(|&i| at(&base[i])).collect();
        let at_now: Vec<At> = left_now.iter().map(|&i| at(&now[i])).collect();
        let (to_now, to_base) = (nearest(&at_base, &at_now), nearest(&at_now, &at_base));
        let mut group = (Vec::new(), Vec::new());
        for (k, &from) in left_base.iter().enumerate() {
            match to_now[k] {
                Some(j)
                    if to_base[j] == Some(k)
                        && !twin(&twins_base, &base[from])
                        && !twin(&twins_now, &now[left_now[j]]) =>
                {
                    matching.pair(from, left_now[j], Matched::Nearest);
                }
                _ => group.0.push(from),
            }
        }
        (group.1).extend(
            left_now
                .iter()
                .filter(|&&i| matching.base_of[i].is_none())
                .copied(),
        );
        // What is left of one side only was added or deleted; of both, nothing says which is which.
        if group.0.is_empty() || group.1.is_empty() {
            continue;
        }
        for &index in &group.0 {
            matching.group_of_base[index] = Some(matching.groups.len());
        }
        for &index in &group.1 {
            matching.group_of_now[index] = Some(matching.groups.len());
        }
        matching.groups.push(group);
    }
    matching
}

/// What an entity names as its parent.
#[derive(Clone, Copy, PartialEq)]
enum Link {
    None,
    /// An index of the file's own entities.
    Own(usize),
    /// An index of the parent map's entities.
    ParentMap(i32),
}

fn link_of(entity: &CEntityDef) -> Link {
    if entity.flags & IN_PARENT_MAP != 0 {
        return Link::ParentMap(entity.parent_index);
    }
    usize::try_from(entity.parent_index).map_or(Link::None, Link::Own)
}

/// How many entities of `list` name each of its entities as their parent.
fn own_children(list: &[CEntityDefAny]) -> Vec<i64> {
    let mut children = vec![0i64; list.len()];
    for entity in list {
        if let Link::Own(parent) = link_of(entity) {
            if let Some(count) = children.get_mut(parent) {
                *count += 1;
            }
        }
    }
    children
}

/// The two lists compared, with what the matching and the links of each say.
struct Compared<'a> {
    base: &'a [CEntityDefAny],
    now: &'a [CEntityDefAny],
    matching: Matching,
    children_base: Vec<i64>,
    children_now: Vec<i64>,
}

impl Compared<'_> {
    /// Whether the entity `base` names as its parent is the entity `now` names.
    fn same_parent(&self, base: &CEntityDef, now: &CEntityDef) -> bool {
        match (link_of(base), link_of(now)) {
            (Link::None, Link::None) => true,
            (Link::ParentMap(from), Link::ParentMap(to)) => from == to,
            (Link::Own(from), Link::Own(to)) => {
                match (self.matching.now_of.get(from), self.now.get(to)) {
                    (Some(Some((at, _))), Some(_)) => *at == to,
                    // A parent that cannot be told from a twin is the same parent while the link stays among its twins.
                    (Some(None), Some(_)) => {
                        let group = self.matching.group_of_base[from];
                        group.is_some() && group == self.matching.group_of_now[to]
                    }
                    // A link past the end of both files names nothing, and is the same link while it is the same number.
                    (None, None) => from == to,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// What the entity `to` of the changed list states differently from the entity `from` of the base it is.
    fn changes(&self, from: usize, to: usize) -> Vec<EntityChange> {
        let (base, now) = (&self.base[from], &self.now[to]);
        if base == now && self.same_parent(base, now) {
            return Vec::new();
        }
        let mut changes = Vec::new();
        if base.archetype_name.hash != now.archetype_name.hash {
            changes.push(EntityChange::Archetype {
                from: name_of(&base.archetype_name),
                to: name_of(&now.archetype_name),
            });
        }
        let (was, is) = (pose_key(base), pose_key(now));
        if was[..3] != is[..3] {
            changes.push(EntityChange::Moved {
                from: position_of(base),
                to: position_of(now),
            });
        }
        if was[3..7] != is[3..7] {
            changes.push(EntityChange::Turned {
                from: turned(base),
                to: turned(now),
            });
        }
        if was[7..] != is[7..] {
            changes.push(EntityChange::Scaled {
                from: [base.scale_xy, base.scale_z],
                to: [now.scale_xy, now.scale_z],
            });
        }
        if !base.lod_dist.same(&now.lod_dist) {
            changes.push(EntityChange::LodDist {
                from: base.lod_dist,
                to: now.lod_dist,
            });
        }
        if base.flags != now.flags {
            changes.push(EntityChange::Flags {
                from: base.flags,
                to: now.flags,
            });
        }
        if !self.same_parent(base, now) {
            changes.push(EntityChange::Parent {
                from: base.parent_index,
                to: now.parent_index,
            });
        }
        if base.guid != now.guid {
            changes.push(EntityChange::Guid {
                from: base.guid,
                to: now.guid,
            });
        }

        let mut names = Vec::new();
        // A count of children that followed the entities naming this one is no change of its own: a deleted child took one off.
        let stated = i64::from(now.num_children) - i64::from(base.num_children);
        if stated != 0 && stated != self.children_now[to] - self.children_base[from] {
            names.push("numChildren");
        }
        let (mut was, mut is) = (base.clone(), now.clone());
        {
            let (was, is): (&mut CEntityDef, &mut CEntityDef) = (&mut was, &mut is);
            for blank in [&mut *was, &mut *is] {
                blank.archetype_name = HashedString::default();
                blank.position = Default::default();
                blank.rotation = Default::default();
                (blank.scale_xy, blank.scale_z, blank.lod_dist) = (0.0, 0.0, 0.0);
                (blank.flags, blank.guid, blank.num_children) = (0, 0, 0);
                blank.parent_index = 0;
            }
            members!(base, now, (was, is), {
                child_lod_dist: "childLodDist",
                ambient_occlusion_multiplier: "ambientOcclusionMultiplier",
                artificial_ambient_occlusion: "artificialAmbientOcclusion",
                tint_value: "tintValue",
            }, &mut |name, _, _| names.push(name));
        }
        let same_type = match (base, now, &mut was, &mut is) {
            (
                CEntityDefAny::CMloInstanceDef(base),
                CEntityDefAny::CMloInstanceDef(now),
                CEntityDefAny::CMloInstanceDef(was),
                CEntityDefAny::CMloInstanceDef(is),
            ) => {
                members!(base, now, (was, is), {
                    group_id: "groupId",
                    floor_id: "floorId",
                    default_entity_sets: "defaultEntitySets",
                    num_exit_portals: "numExitPortals",
                    mlo_instflags: "MLOInstflags",
                }, &mut |name, _, _| names.push(name));
                true
            }
            (CEntityDefAny::Base(_), CEntityDefAny::Base(_), ..) => true,
            _ => false,
        };
        if !names.is_empty() {
            changes.push(EntityChange::Members { names });
        }
        if !same_type || was != is {
            changes.push(EntityChange::Other);
        }
        changes
    }
}

/// The rotation that places an entity, as `x, y, z, w` with no zero below zero: the conjugate of no rotation holds three.
fn turned(entity: &CEntityDefAny) -> [f32; 4] {
    at(entity).rotation.to_array().map(|v| v + 0.0)
}

fn position_of(entity: &CEntityDef) -> [f32; 3] {
    [entity.position.x, entity.position.y, entity.position.z]
}

fn entity_ref(index: usize, entity: &CEntityDef) -> EntityRef {
    EntityRef {
        index,
        guid: entity.guid,
        archetype: name_of(&entity.archetype_name),
        position: position_of(entity),
    }
}

/// What the map `now` changed of the map `base`: its entities, and what it states of itself.
pub fn map_changes(base: &CMapData, now: &CMapData) -> MapChanges {
    let mut out = entity_changes(&base.entities, &now.entities);
    // ⚠ A typed root carries the tree it was read from, so a copy of one is a copy of the file: none is made of two equal maps.
    if base != now {
        let (mut was, mut is) = (base.clone(), now.clone());
        members!(base, now, (&mut was, &mut is), {
            name: "name",
            parent: "parent",
            flags: "flags",
            content_flags: "contentFlags",
            streaming_extents_min: "streamingExtentsMin",
            streaming_extents_max: "streamingExtentsMax",
            entities_extents_min: "entitiesExtentsMin",
            entities_extents_max: "entitiesExtentsMax",
            physics_dictionaries: "physicsDictionaries",
        }, &mut |member, from, to| out.root.push(MemberChange { member, from, to }));
        (was.entities, is.entities) = (Vec::new(), Vec::new());
        out.other = was != is;
    }
    out
}

/// [`map_changes`] of two lists of entities, which the entities of an interior are too.
pub fn entity_changes(base: &[CEntityDefAny], now: &[CEntityDefAny]) -> MapChanges {
    let compared = Compared {
        base,
        now,
        matching: match_entities(base, now),
        children_base: own_children(base),
        children_now: own_children(now),
    };
    let matching = &compared.matching;
    let mut out = MapChanges::default();
    for (index, entity) in base.iter().enumerate() {
        match matching.now_of[index] {
            Some((to, matched)) => {
                let changes = compared.changes(index, to);
                if changes.is_empty() {
                    out.unchanged += 1;
                } else {
                    out.changed.push(Kept {
                        base: entity_ref(index, entity),
                        now: entity_ref(to, &now[to]),
                        matched,
                        changes,
                    });
                }
            }
            None if matching.group_of_base[index].is_none() => {
                out.deleted.push(entity_ref(index, entity));
            }
            None => {}
        }
    }
    for (index, entity) in now.iter().enumerate() {
        if matching.base_of[index].is_none() && matching.group_of_now[index].is_none() {
            out.added.push(entity_ref(index, entity));
        }
    }
    for (of_base, of_now) in &matching.groups {
        out.ambiguous.push(Ambiguous {
            archetype: name_of(&base[of_base[0]].archetype_name),
            base: of_base.iter().map(|&i| entity_ref(i, &base[i])).collect(),
            now: of_now.iter().map(|&i| entity_ref(i, &now[i])).collect(),
        });
    }
    out
}

/// What a member of a record states, as a comparison shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Stated {
    Float(f32),
    Number(i64),
    Vector([f32; 3]),
    Name(#[serde(serialize_with = "name")] Name),
    Text(String),
}

/// A member of a typed record: whether two records state the same of it, to the bit, and what one states.
trait Member {
    fn same(&self, other: &Self) -> bool;
    fn stated(&self) -> Stated;
}

impl Member for f32 {
    fn same(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }

    fn stated(&self) -> Stated {
        Stated::Float(*self)
    }
}

impl Member for u32 {
    fn same(&self, other: &Self) -> bool {
        self == other
    }

    fn stated(&self) -> Stated {
        Stated::Number(i64::from(*self))
    }
}

impl Member for i32 {
    fn same(&self, other: &Self) -> bool {
        self == other
    }

    fn stated(&self) -> Stated {
        Stated::Number(i64::from(*self))
    }
}

impl Member for or_io::decl::Vec3 {
    fn same(&self, other: &Self) -> bool {
        [self.x, self.y, self.z].map(f32::to_bits) == [other.x, other.y, other.z].map(f32::to_bits)
    }

    fn stated(&self) -> Stated {
        Stated::Vector([self.x, self.y, self.z])
    }
}

impl Member for HashedString {
    fn same(&self, other: &Self) -> bool {
        self.hash == other.hash
    }

    fn stated(&self) -> Stated {
        Stated::Name(name_of(self))
    }
}

impl Member for Vec<HashedString> {
    fn same(&self, other: &Self) -> bool {
        self.iter().map(|n| n.hash).eq(other.iter().map(|n| n.hash))
    }

    fn stated(&self) -> Stated {
        let names: Vec<String> = self.iter().map(|n| name_of(n).shown()).collect();
        Stated::Text(names.join(", "))
    }
}

/// An archetype where a caller finds it: its index among the archetypes of its file, and its name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchetypeRef {
    pub index: usize,
    #[serde(serialize_with = "name")]
    pub name: Name,
}

/// One member a record of both files states differently: an archetype, or the root of a file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemberChange {
    /// The engine's name of the member, or `type` for the type of the record itself.
    pub member: &'static str,
    pub from: Stated,
    pub to: Stated,
}

/// An archetype both files declare, with what it states differently.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArchetypeKept {
    pub base: ArchetypeRef,
    pub now: ArchetypeRef,
    pub changes: Vec<MemberChange>,
    /// What the entities of an interior changed, where both are interiors and their entities differ.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entities: Option<MapChanges>,
    /// Whether what no typed member names differs: a room, a portal, an entity set, an extension.
    pub other: bool,
}

/// Archetypes one file declares more than once under one name, which no name tells apart.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AmbiguousArchetypes {
    #[serde(serialize_with = "name")]
    pub name: Name,
    /// The indexes of the archetypes of that name in the base.
    pub base: Vec<usize>,
    /// The indexes of the archetypes of that name in the changed file.
    pub now: Vec<usize>,
}

/// What the archetypes of a type file changed of its base.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TypesChanges {
    pub added: Vec<ArchetypeRef>,
    pub deleted: Vec<ArchetypeRef>,
    pub changed: Vec<ArchetypeKept>,
    pub ambiguous: Vec<AmbiguousArchetypes>,
    pub unchanged: usize,
    /// What the type file states of itself differently, each member by the engine's name.
    pub root: Vec<MemberChange>,
    /// Whether what no typed member of the type file names differs: a composite entity type, an extension.
    pub other: bool,
}

impl TypesChanges {
    /// Whether the two files state the same of themselves and declare the same archetypes stating the same.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.deleted.is_empty()
            && self.changed.is_empty()
            && self.ambiguous.is_empty()
            && self.root.is_empty()
            && !self.other
    }
}

fn type_of(archetype: &CBaseArchetypeDefAny) -> &'static str {
    match archetype {
        CBaseArchetypeDefAny::CTimeArchetypeDef(_) => "CTimeArchetypeDef",
        CBaseArchetypeDefAny::CMloArchetypeDef(_) => "CMloArchetypeDef",
        CBaseArchetypeDefAny::Base(_) => "CBaseArchetypeDef",
    }
}

/// What the archetype `now` states differently from `base`, the same archetype of the base.
fn archetype_changes(
    from: usize,
    base: &CBaseArchetypeDefAny,
    to: usize,
    now: &CBaseArchetypeDefAny,
) -> ArchetypeKept {
    let mut changes = Vec::new();
    let mut differs = |member, from, to| changes.push(MemberChange { member, from, to });
    let (mut was, mut is) = (base.clone(), now.clone());
    {
        let blank: (&mut CBaseArchetypeDef, &mut CBaseArchetypeDef) = (&mut was, &mut is);
        members!(base, now, blank, {
            lod_dist: "lodDist",
            flags: "flags",
            special_attribute: "specialAttribute",
            bb_min: "bbMin",
            bb_max: "bbMax",
            bs_centre: "bsCentre",
            bs_radius: "bsRadius",
            hd_texture_dist: "hdTextureDist",
            name: "name",
            texture_dictionary: "textureDictionary",
            clip_dictionary: "clipDictionary",
            drawable_dictionary: "drawableDictionary",
            physics_dictionary: "physicsDictionary",
            asset_name: "assetName",
        }, &mut differs);
    }
    let mut entities = None;
    let same_type = match (base, now, &mut was, &mut is) {
        (
            CBaseArchetypeDefAny::CTimeArchetypeDef(base),
            CBaseArchetypeDefAny::CTimeArchetypeDef(now),
            CBaseArchetypeDefAny::CTimeArchetypeDef(was),
            CBaseArchetypeDefAny::CTimeArchetypeDef(is),
        ) => {
            members!(base, now, (was, is), { time_flags: "timeFlags" }, &mut differs);
            true
        }
        (
            CBaseArchetypeDefAny::CMloArchetypeDef(base),
            CBaseArchetypeDefAny::CMloArchetypeDef(now),
            CBaseArchetypeDefAny::CMloArchetypeDef(was),
            CBaseArchetypeDefAny::CMloArchetypeDef(is),
        ) => {
            members!(base, now, (&mut *was, &mut *is), { mlo_flags: "mloFlags" }, &mut differs);
            let inside = entity_changes(&base.entities, &now.entities);
            entities = (!inside.is_empty()).then_some(inside);
            (was.entities, is.entities) = (Vec::new(), Vec::new());
            true
        }
        (CBaseArchetypeDefAny::Base(_), CBaseArchetypeDefAny::Base(_), ..) => true,
        _ => false,
    };
    if !same_type {
        changes.push(MemberChange {
            member: "type",
            from: Stated::Text(type_of(base).to_string()),
            to: Stated::Text(type_of(now).to_string()),
        });
    }
    ArchetypeKept {
        base: ArchetypeRef {
            index: from,
            name: name_of(&base.name),
        },
        now: ArchetypeRef {
            index: to,
            name: name_of(&now.name),
        },
        changes,
        entities,
        other: same_type && was != is,
    }
}

/// What the type file `now` changed of the type file `base`: its archetypes, an archetype being the one of its name, and what it states of itself.
pub fn types_changes(base: &CMapTypes, now: &CMapTypes) -> TypesChanges {
    let mut named: BTreeMap<u32, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (index, archetype) in base.archetypes.iter().enumerate() {
        named.entry(archetype.name.hash).or_default().0.push(index);
    }
    for (index, archetype) in now.archetypes.iter().enumerate() {
        named.entry(archetype.name.hash).or_default().1.push(index);
    }
    let of = |list: &[CBaseArchetypeDefAny], index: usize| ArchetypeRef {
        index,
        name: name_of(&list[index].name),
    };
    let mut out = TypesChanges::default();
    for (of_base, of_now) in named.into_values() {
        match (of_base.as_slice(), of_now.as_slice()) {
            (held, []) => (out.deleted).extend(held.iter().map(|&i| of(&base.archetypes, i))),
            ([], held) => (out.added).extend(held.iter().map(|&i| of(&now.archetypes, i))),
            (&[from], &[to]) => {
                let kept = archetype_changes(from, &base.archetypes[from], to, &now.archetypes[to]);
                if kept.changes.is_empty() && kept.entities.is_none() && !kept.other {
                    out.unchanged += 1;
                } else {
                    out.changed.push(kept);
                }
            }
            // A name declared twice is told apart by nothing: the same records in the same order are unchanged, and anything else is not judged.
            (from, to) => {
                let same = from.len() == to.len()
                    && (from.iter().zip(to))
                        .all(|(&a, &b)| base.archetypes[a] == now.archetypes[b]);
                if same {
                    out.unchanged += from.len();
                } else {
                    out.ambiguous.push(AmbiguousArchetypes {
                        name: name_of(&base.archetypes[from[0]].name),
                        base: from.to_vec(),
                        now: to.to_vec(),
                    });
                }
            }
        }
    }
    out.added.sort_by_key(|a| a.index);
    out.deleted.sort_by_key(|a| a.index);
    out.changed.sort_by_key(|a| a.base.index);
    if base != now {
        let (mut was, mut is) = (base.clone(), now.clone());
        members!(base, now, (&mut was, &mut is), {
            name: "name",
            dependencies: "dependencies",
        }, &mut |member, from, to| out.root.push(MemberChange { member, from, to }));
        (was.archetypes, is.archetypes) = (Vec::new(), Vec::new());
        out.other = was != is;
    }
    out
}

/// Why a source could not be compared with its base.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Source(String),
    #[error("{0} is neither a map file nor a type file")]
    Kind(String),
    #[error("the source does not cook: {0}")]
    Cook(String),
    #[error("the base of the source does not cook: {0}")]
    Base(String),
    #[error("{what} does not read as a {root}: {why}")]
    Read {
        what: &'static str,
        root: &'static str,
        why: String,
    },
}

/// What a metadata source changed of its base: the entities of a map file, or the archetypes of a type file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SourceChanges {
    Map(MapChanges),
    Types(TypesChanges),
}

impl SourceChanges {
    /// Whether the source states what its base states.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Map(changes) => changes.is_empty(),
            Self::Types(changes) => changes.is_empty(),
        }
    }
}

/// The typed root `bytes` read as, `what` naming whose bytes they are in the refusal.
fn read<T: File>(what: &'static str, root: &'static str, bytes: &[u8]) -> Result<T, Error> {
    T::from_bytes(bytes).map_err(|e| Error::Read {
        what,
        root,
        why: e.to_string(),
    })
}

/// What the map file or type file the source in `dir` cooks to changed of the file its base cooks to.
pub fn source_changes(dir: &Path) -> Result<SourceChanges, Error> {
    let source = or_source::asset::Source::open(dir).map_err(|p| Error::Source(p.to_string()))?;
    let file = or_source::metadata::cooked_name(&source.manifest)
        .map_err(|p| Error::Source(p.to_string()))?;
    let extension = file.rsplit_once('.').map_or("", |(_, e)| e);
    if !matches!(extension, "ymap" | "ytyp") {
        return Err(Error::Kind(file));
    }
    let said = |problems: Vec<or_source::Problem>| {
        let said: Vec<String> = problems.iter().map(ToString::to_string).collect();
        said.join("; ")
    };
    let now = or_source::metadata::compile(&source).map_err(|p| Error::Cook(said(p)))?;
    let base = or_source::metadata::compile_base(&source).map_err(|p| Error::Base(said(p)))?;
    Ok(if extension == "ymap" {
        SourceChanges::Map(map_changes(
            &read("the base", "map file", &base.bytes)?,
            &read("the source", "map file", &now.bytes)?,
        ))
    } else {
        SourceChanges::Types(types_changes(
            &read("the base", "type file", &base.bytes)?,
            &read("the source", "type file", &now.bytes)?,
        ))
    })
}
