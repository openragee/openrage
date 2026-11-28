//! Prefab catalogues: reusable sets of placements, one catalogue per title.
//!
//! A prefab is a `CMapData` whose entities sit relative to an origin, so it
//! reads, writes and exports through everything else in this crate.
//!
//! ⚠ Enhanced appends members to `CEntityDef` that Legacy does not declare,
//! so a Legacy placement put into an Enhanced map is a document its own type
//! table cannot describe. [`EntityShape`] is that member set:
//! `a_prefab_round_trips_every_map` finds exactly one shape per install and a
//! different one on each.

use std::collections::{BTreeMap, BTreeSet};

use glam::{Quat, Vec3};
use or_io::meta::{MetaInstance, Value};

use crate::field;
use crate::keys;
use crate::map_data::MapData;

/// Why a prefab was not captured or placed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrefabError {
    #[error("not a CMapData")]
    NotAMap,
    #[error("the source holds {len} entities, so there is no entity {index}")]
    NoSuchEntity { index: usize, len: usize },
    #[error("entity {0} links into a parent map, which a prefab has none of")]
    CrossFileLink(usize),
    #[error("entity {entity} names LOD parent {parent}, which the selection leaves out")]
    LinkLeavesSelection { entity: usize, parent: usize },
    #[error("a prefab of one title cannot be placed in a map of another: its entities carry {prefab} members and the map's carry {map}")]
    TitleMismatch { prefab: usize, map: usize },
    #[error("the catalogue is for another title: {0}")]
    WrongCatalogue(String),
    #[error("the prefab holds no entities")]
    Empty,
}

/// The members each kind of entity carries, which is what a title's
/// `CEntityDef` and `CMloInstanceDef` are.
///
/// ⚠ **Per type, because a map need not hold both.** An interior placement
/// map holds only `CMloInstanceDef`s, which carry more members than a plain
/// entity; two shapes agree when every type both of them hold carries the
/// same members.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityShape(BTreeMap<u32, BTreeSet<u32>>);

impl EntityShape {
    pub fn of(entity: &MetaInstance) -> Self {
        let members = entity
            .fields
            .iter()
            .map(|(n, _)| field::name_hash(n))
            .collect();
        Self(BTreeMap::from([(
            field::name_hash(&entity.type_name),
            members,
        )]))
    }

    /// Every entity type a map holds, or `None` for a map with no entity.
    pub fn of_map(root: &MetaInstance) -> Option<Self> {
        let map = MapData::new(root)?;
        let mut out = Self::default();
        for e in map.entities() {
            let ty = field::name_hash(&e.instance().type_name);
            if !out.0.contains_key(&ty) {
                out.0.extend(Self::of(e.instance()).0);
            }
        }
        (!out.0.is_empty()).then_some(out)
    }

    /// Whether every type both shapes hold carries the same members.
    pub fn agrees_with(&self, other: &Self) -> bool {
        self.differences(other).is_empty()
    }

    /// The members a shared type carries in one shape and not the other.
    pub fn differences(&self, other: &Self) -> Vec<u32> {
        let mut out = Vec::new();
        for (ty, mine) in &self.0 {
            if let Some(theirs) = other.0.get(ty) {
                out.extend(mine.symmetric_difference(theirs).copied());
            }
        }
        out
    }

    /// Take in the types `other` holds and this one does not.
    pub fn absorb(&mut self, other: &Self) {
        for (ty, members) in &other.0 {
            self.0.entry(*ty).or_insert_with(|| members.clone());
        }
    }

    /// How many members the plain entity type carries, or the widest type's.
    pub fn len(&self) -> usize {
        let plain = field::name_hash(crate::keys::types::C_ENTITY_DEF);
        self.0
            .get(&plain)
            .or_else(|| self.0.values().max_by_key(|m| m.len()))
            .map_or(0, BTreeSet::len)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every `(type, member)` pair.
    pub fn members(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.0
            .iter()
            .flat_map(|(ty, m)| m.iter().map(move |member| (*ty, *member)))
    }
}

/// The same fields with every array emptied, recursively.
///
/// ⚠ **Built, never cloned-then-cleared.** A map's `instancedData` holds its
/// grass, and across an install that is 38.4 million instances; cloning a
/// root to empty it copies them first.
pub fn emptied(value: &Value) -> Value {
    match value {
        Value::Array(_) => Value::Array(Vec::new()),
        Value::Struct(inner) => Value::Struct(Box::new(emptied_instance(inner))),
        Value::Pointer(Some(inner)) => Value::Pointer(Some(Box::new(emptied_instance(inner)))),
        other => other.clone(),
    }
}

fn emptied_instance(inst: &MetaInstance) -> MetaInstance {
    let mut out = MetaInstance::new(inst.type_name.clone());
    for (name, value) in &inst.fields {
        out.fields.push((name.clone(), emptied(value)));
    }
    out
}

/// Capture entities out of a map as a prefab, placed relative to `origin`.
///
/// The prefab keeps the source's type table, so it is the source's title's.
/// Local LOD links are renumbered into the selection and `numChildren` is
/// recounted within it, because a prefab has no other map to hold children.
pub fn capture(
    source: &MetaInstance,
    indices: &[usize],
    name: u32,
    origin: Vec3,
) -> Result<MetaInstance, PrefabError> {
    let map = MapData::new(source).ok_or(PrefabError::NotAMap)?;
    let all: Vec<&MetaInstance> = map.entities().map(|e| e.instance()).collect();
    let mut position: BTreeMap<usize, usize> = BTreeMap::new();
    for (at, &i) in indices.iter().enumerate() {
        if i >= all.len() {
            return Err(PrefabError::NoSuchEntity {
                index: i,
                len: all.len(),
            });
        }
        position.insert(i, at);
    }
    if indices.is_empty() {
        return Err(PrefabError::Empty);
    }

    let mut picked = Vec::with_capacity(indices.len());
    for &i in indices {
        let mut ent = all[i].clone();
        if in_parent_map(&ent) && parent_index_of(&ent).is_some() {
            return Err(PrefabError::CrossFileLink(i));
        }
        if let Some(p) = parent_index_of(&ent) {
            let Some(&at) = position.get(&(p as usize)) else {
                return Err(PrefabError::LinkLeavesSelection {
                    entity: i,
                    parent: p as usize,
                });
            };
            set_int(&mut ent, keys::PARENT_INDEX, at as i64);
        }
        let p = field::vec3_at(&ent, keys::POSITION)
            .map(Vec3::from)
            .unwrap_or(Vec3::ZERO);
        set_vec3(&mut ent, keys::POSITION, p - origin);
        picked.push(ent);
    }
    let mut children = vec![0i64; picked.len()];
    for ent in &picked {
        if let Some(p) = parent_index_of(ent) {
            children[p as usize] += 1;
        }
    }
    for (ent, n) in picked.iter_mut().zip(children) {
        set_int(ent, keys::NUM_CHILDREN, n);
    }

    let pointer = matches!(
        field::get(source, keys::ENTITIES),
        Some(Value::Array(items)) if matches!(items.first(), Some(Value::Pointer(_)))
    );
    let mut out = emptied_instance(source);
    if let Some(slot) = field::get_mut(&mut out, keys::ENTITIES) {
        *slot = Value::Array(
            picked
                .into_iter()
                .map(|e| {
                    if pointer {
                        Value::Pointer(Some(Box::new(e)))
                    } else {
                        Value::Struct(Box::new(e))
                    }
                })
                .collect(),
        );
    }
    set_hash(&mut out, keys::NAME, name);
    set_hash(&mut out, keys::PARENT, 0);
    Ok(out)
}

/// The indices a map's entities can be captured from whole: every entity
/// whose LOD chain stays inside the map.
pub fn capturable(source: &MetaInstance) -> Vec<usize> {
    let Some(map) = MapData::new(source) else {
        return Vec::new();
    };
    let ents: Vec<_> = map.entities().collect();
    let mut ok = vec![None::<bool>; ents.len()];
    fn resolve(
        i: usize,
        ents: &[crate::map_data::Entity<'_>],
        ok: &mut [Option<bool>],
        depth: usize,
    ) -> bool {
        if let Some(v) = ok[i] {
            return v;
        }
        let e = &ents[i];
        let v = if e.lod_in_parent_map() && e.parent_index().is_some() {
            false
        } else {
            match e.parent_index() {
                None => true,
                Some(p) if p < ents.len() && p != i && depth < ents.len() => {
                    resolve(p, ents, ok, depth + 1)
                }
                Some(_) => false,
            }
        };
        ok[i] = Some(v);
        v
    }
    (0..ents.len())
        .filter(|i| resolve(*i, &ents, &mut ok, 0))
        .collect()
}

/// What placing a prefab did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Placed {
    /// Where the first placed entity landed in the target's array.
    pub base: usize,
    pub entities: usize,
    /// Placed entities whose `guid` collided with one already in the map.
    pub guids_reassigned: usize,
}

/// The entities a prefab would add, transformed to `at` turned by `heading`
/// radians about Z, with local links still relative to the prefab.
///
/// ⚠ **The rotation composes through the stored conjugate.** A `CEntityDef`
/// stores the conjugate of the rotation placing it, so turning a placed
/// entity is `store(R · rotation_of(stored))` and never `R · stored`, which
/// turns it the other way. `a_placed_entity_turns_with_the_prefab` holds it.
pub fn placements(
    prefab: &MetaInstance,
    at: Vec3,
    heading: f32,
) -> Result<Vec<MetaInstance>, PrefabError> {
    let map = MapData::new(prefab).ok_or(PrefabError::NotAMap)?;
    let turn = Quat::from_rotation_z(heading);
    let mut out = Vec::with_capacity(map.entity_count());
    for e in map.entities() {
        let mut ent = e.instance().clone();
        let placement = e.placement();
        set_vec3(&mut ent, keys::POSITION, turn * e.position() + at);
        let stored = placement.store(turn * e.rotation());
        set_vec4(&mut ent, keys::ROTATION, stored);
        out.push(ent);
    }
    if out.is_empty() {
        return Err(PrefabError::Empty);
    }
    Ok(out)
}

/// Place a prefab into a map's value tree.
///
/// ⚠ Refused across titles by [`EntityShape`]; a map with no entity to
/// compare against takes the prefab's word for it. A `guid` already in the
/// map is replaced, because two placements of one prefab would otherwise
/// carry the same identity.
pub fn place(
    target: &mut MetaInstance,
    prefab: &MetaInstance,
    at: Vec3,
    heading: f32,
) -> Result<Placed, PrefabError> {
    let target_map = MapData::new(target).ok_or(PrefabError::NotAMap)?;
    let target_name = target_map.name();
    let mut guids: BTreeSet<u32> = target_map
        .entities()
        .map(|e| e.guid())
        .filter(|g| *g != 0)
        .collect();
    if let (Some(want), Some(have)) = (EntityShape::of_map(target), EntityShape::of_map(prefab)) {
        if !want.agrees_with(&have) {
            return Err(PrefabError::TitleMismatch {
                prefab: have.len(),
                map: want.len(),
            });
        }
    }
    let mut block = placements(prefab, at, heading)?;
    let mut reassigned = 0;
    for (i, ent) in block.iter_mut().enumerate() {
        let guid = field::u32_at(ent, keys::GUID).unwrap_or(0);
        if guid == 0 {
            continue;
        }
        if guids.insert(guid) {
            continue;
        }
        let mut salt = 0u32;
        let fresh = loop {
            let text = format!("{target_name:08x}:{guid:08x}:{i}:{salt}");
            let candidate = or_io::string_hash(text.as_bytes());
            if candidate != 0 && guids.insert(candidate) {
                break candidate;
            }
            salt += 1;
        };
        set_int(ent, keys::GUID, fresh as i64);
        reassigned += 1;
    }

    let Some(Value::Array(items)) = field::get_mut(target, keys::ENTITIES) else {
        return Err(PrefabError::NotAMap);
    };
    let base = items.len();
    let pointer = matches!(items.first(), Some(Value::Pointer(_)) | None);
    let count = block.len();
    for mut ent in block {
        if let Some(p) = parent_index_of(&ent) {
            set_int(&mut ent, keys::PARENT_INDEX, p + base as i64);
        }
        items.push(if pointer {
            Value::Pointer(Some(Box::new(ent)))
        } else {
            Value::Struct(Box::new(ent))
        });
    }
    Ok(Placed {
        base,
        entities: count,
        guids_reassigned: reassigned,
    })
}

/// One prefab as a catalogue lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PrefabInfo {
    pub name: u32,
    pub entities: usize,
    /// Every archetype it places, which is what a map needs stated to hold it.
    pub archetypes: BTreeSet<u32>,
    /// The box its entity positions span, relative to its origin.
    pub span: (Vec3, Vec3),
}

impl PrefabInfo {
    pub fn of(prefab: &MetaInstance) -> Result<Self, PrefabError> {
        let map = MapData::new(prefab).ok_or(PrefabError::NotAMap)?;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut archetypes = BTreeSet::new();
        for e in map.entities() {
            archetypes.insert(e.archetype());
            min = min.min(e.position());
            max = max.max(e.position());
        }
        if archetypes.is_empty() {
            return Err(PrefabError::Empty);
        }
        Ok(Self {
            name: map.name(),
            entities: map.entity_count(),
            archetypes,
            span: (min, max),
        })
    }
}

/// One title's prefabs, keyed by the name each was filed under.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    title: String,
    shape: Option<EntityShape>,
    prefabs: BTreeMap<String, PrefabInfo>,
}

impl Catalogue {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn shape(&self) -> Option<&EntityShape> {
        self.shape.as_ref()
    }

    /// File a prefab, refusing one whose entities are another title's.
    pub fn add(
        &mut self,
        key: impl Into<String>,
        prefab: &MetaInstance,
    ) -> Result<(), PrefabError> {
        let info = PrefabInfo::of(prefab)?;
        let shape = EntityShape::of_map(prefab).ok_or(PrefabError::Empty)?;
        match &mut self.shape {
            Some(have) if !have.agrees_with(&shape) => {
                return Err(PrefabError::WrongCatalogue(format!(
                    "`{}` holds prefabs whose entities carry {} members, and this one's carry {}",
                    self.title,
                    have.len(),
                    shape.len()
                )))
            }
            Some(have) => have.absorb(&shape),
            None => self.shape = Some(shape),
        }
        self.prefabs.insert(key.into(), info);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&PrefabInfo> {
        self.prefabs.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &PrefabInfo)> {
        self.prefabs.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.prefabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prefabs.is_empty()
    }
}

fn in_parent_map(ent: &MetaInstance) -> bool {
    field::u32_at(ent, keys::FLAGS).unwrap_or(0) & (1 << 3) != 0
}

fn parent_index_of(ent: &MetaInstance) -> Option<i64> {
    match field::i32_at(ent, keys::PARENT_INDEX) {
        Some(i) if i >= 0 => Some(i as i64),
        _ => None,
    }
}

fn set_int(inst: &mut MetaInstance, key: u32, value: i64) {
    match field::get_mut(inst, key) {
        Some(Value::Int(v)) => *v = value,
        Some(Value::UInt(v)) => *v = value as u64,
        _ => {}
    }
}

fn set_hash(inst: &mut MetaInstance, key: u32, hash: u32) {
    match field::get_mut(inst, key) {
        Some(Value::Hash(h)) => {
            h.hash = hash;
            h.raw.clear();
        }
        Some(Value::UInt(n)) => *n = hash as u64,
        _ => {}
    }
}

fn set_vec3(inst: &mut MetaInstance, key: u32, v: Vec3) {
    match field::get_mut(inst, key) {
        Some(Value::Vec3(slot)) => *slot = v.into(),
        Some(Value::Vec4(slot)) => *slot = [v.x, v.y, v.z, slot[3]],
        _ => {}
    }
}

fn set_vec4(inst: &mut MetaInstance, key: u32, v: [f32; 4]) {
    if let Some(Value::Vec4(slot)) = field::get_mut(inst, key) {
        *slot = v;
    }
}

/// Whether two stored rotations place an entity the same way.
///
/// ⚠ `q` and `-q` are the same rotation, so a component comparison is wrong
/// half the time.
pub fn same_rotation(a: Quat, b: Quat, tolerance: f32) -> bool {
    a.dot(b).abs() >= 1.0 - tolerance
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::Placement;
    use or_io::meta::instance::value::HashedString;

    const IN_PARENT: u32 = 1 << 3;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn ent(arch: &str, pos: [f32; 3], rot: Quat, parent: i32, guid: u32, flags: u32) -> Value {
        let mut e = MetaInstance::new("CEntityDef");
        e.set("archetypeName", Value::Hash(HashedString::new(arch)));
        e.set("flags", Value::UInt(flags as u64));
        e.set("guid", Value::UInt(guid as u64));
        e.set("position", Value::Vec3(pos));
        e.set("rotation", Value::Vec4(Placement::Entity.store(rot)));
        e.set("parentIndex", Value::Int(parent as i64));
        e.set("numChildren", Value::UInt(0));
        Value::Pointer(Some(Box::new(e)))
    }

    fn map(name: &str, entities: Vec<Value>) -> MetaInstance {
        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::Hash(HashedString::new(name)));
        root.set("parent", Value::Hash(HashedString::new("p")));
        root.set("entities", Value::Array(entities));
        root.set("carGenerators", Value::Array(vec![Value::UInt(1)]));
        root
    }

    fn source() -> MetaInstance {
        map(
            "street",
            vec![
                ent("lamp", [10.0, 0.0, 0.0], Quat::IDENTITY, -1, 11, 0),
                ent("house_lod", [20.0, 0.0, 0.0], Quat::IDENTITY, -1, 12, 0),
                ent(
                    "house",
                    [20.0, 1.0, 0.0],
                    Quat::from_rotation_z(0.5),
                    1,
                    13,
                    0,
                ),
                ent("hd_child", [0.0; 3], Quat::IDENTITY, 3, 14, IN_PARENT),
            ],
        )
    }

    #[test]
    fn a_capture_is_relative_self_contained_and_nothing_but_entities() {
        let src = source();
        let prefab = capture(
            &src,
            &[1, 2],
            hash("house_prefab"),
            Vec3::new(20.0, 0.0, 0.0),
        )
        .unwrap();
        let map = MapData::new(&prefab).unwrap();
        assert_eq!(map.name(), hash("house_prefab"));
        assert_eq!(map.parent(), 0);
        let ents: Vec<_> = map.entities().collect();
        assert_eq!(ents.len(), 2);
        assert_eq!(ents[0].position(), Vec3::ZERO);
        assert_eq!(ents[1].position(), Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(
            ents[1].parent_index(),
            Some(0),
            "renumbered into the selection"
        );
        assert_eq!(ents[0].num_children(), 1, "recounted within it");
        assert!(field::array_at(&prefab, literal("carGenerators")).is_empty());
        assert!(crate::validate::check(&map).is_empty());
    }

    fn literal(name: &str) -> u32 {
        or_io::hash::literal_string_hash(name.as_bytes())
    }

    #[test]
    fn a_selection_that_cuts_a_link_or_crosses_files_is_refused() {
        let src = source();
        assert_eq!(
            capture(&src, &[2], 1, Vec3::ZERO),
            Err(PrefabError::LinkLeavesSelection {
                entity: 2,
                parent: 1
            })
        );
        assert_eq!(
            capture(&src, &[3], 1, Vec3::ZERO),
            Err(PrefabError::CrossFileLink(3))
        );
        assert_eq!(capturable(&src), vec![0, 1, 2]);
    }

    #[test]
    fn a_placed_entity_turns_with_the_prefab() {
        let src = source();
        let prefab = capture(&src, &[1, 2], 1, Vec3::new(20.0, 0.0, 0.0)).unwrap();
        let quarter = std::f32::consts::FRAC_PI_2;
        let placed = placements(&prefab, Vec3::new(100.0, 0.0, 0.0), quarter).unwrap();
        let house = crate::map_data::Entity::new(&placed[1]);
        assert!(house
            .position()
            .abs_diff_eq(Vec3::new(99.0, 0.0, 0.0), 1e-5));
        let want = Quat::from_rotation_z(quarter) * Quat::from_rotation_z(0.5);
        assert!(same_rotation(house.rotation(), want, 1e-6));

        // The wrong composition is observably different, so the test can fail.
        let stored = field::vec4_at(&placed[1], keys::ROTATION).unwrap();
        let naive = Quat::from_rotation_z(quarter)
            * Quat::from_array(Placement::Entity.store(Quat::from_rotation_z(0.5)));
        assert!(!same_rotation(Quat::from_array(stored), naive, 1e-3));
    }

    #[test]
    fn placing_appends_offsets_links_and_keeps_guids_unique() {
        let src = source();
        let prefab = capture(&src, &[1, 2], 1, Vec3::new(20.0, 0.0, 0.0)).unwrap();
        let mut target = map(
            "target",
            vec![ent("x", [0.0; 3], Quat::IDENTITY, -1, 12, 0)],
        );
        let placed = place(&mut target, &prefab, Vec3::ZERO, 0.0).unwrap();
        assert_eq!(placed.base, 1);
        assert_eq!(placed.guids_reassigned, 1, "guid 12 was taken");
        let again = place(&mut target, &prefab, Vec3::X, 0.0).unwrap();
        assert_eq!(again.guids_reassigned, 2);

        let map = MapData::new(&target).unwrap();
        let guids: Vec<u32> = map.entities().map(|e| e.guid()).collect();
        let distinct: BTreeSet<u32> = guids.iter().copied().collect();
        assert_eq!(distinct.len(), guids.len(), "{guids:?}");
        assert_eq!(map.entities().nth(2).unwrap().parent_index(), Some(1));
        assert_eq!(map.entities().nth(4).unwrap().parent_index(), Some(3));
        assert!(crate::validate::check(&map).is_empty());
    }

    /// ⚠ The type check, falsified with the member Enhanced adds.
    #[test]
    fn a_prefab_of_another_title_is_refused() {
        let src = source();
        let prefab = capture(&src, &[0], 1, Vec3::ZERO).unwrap();
        let mut enhanced = map("e", vec![ent("x", [0.0; 3], Quat::IDENTITY, -1, 0, 0)]);
        if let Some(Value::Array(items)) = field::get_mut(&mut enhanced, keys::ENTITIES) {
            if let Some(Value::Pointer(Some(e))) = items.first_mut() {
                e.set("tintValue", Value::UInt(0));
            }
        }
        assert!(matches!(
            place(&mut enhanced, &prefab, Vec3::ZERO, 0.0),
            Err(PrefabError::TitleMismatch { .. })
        ));

        let mut catalogue = Catalogue::new("gta5-legacy");
        catalogue.add("a", &prefab).unwrap();
        let mut other = capture(&src, &[0], 2, Vec3::ZERO).unwrap();
        if let Some(Value::Array(items)) = field::get_mut(&mut other, keys::ENTITIES) {
            if let Some(Value::Pointer(Some(e))) = items.first_mut() {
                e.set("tintValue", Value::UInt(0));
            }
        }
        assert!(matches!(
            catalogue.add("b", &other),
            Err(PrefabError::WrongCatalogue(_))
        ));
        assert_eq!(catalogue.len(), 1);
    }

    #[test]
    fn emptying_a_value_keeps_its_fields_and_drops_its_arrays() {
        let mut inner = MetaInstance::new("rage__fwInstancedMapData");
        inner.set("GrassInstanceList", Value::Array(vec![Value::UInt(1); 3]));
        inner.set("ImapLink", Value::Hash(HashedString::new("x")));
        let out = emptied(&Value::Struct(Box::new(inner)));
        let Value::Struct(out) = out else { panic!() };
        assert_eq!(out.fields.len(), 2);
        assert!(field::array_at(&out, literal("GrassInstanceList")).is_empty());
    }
}
