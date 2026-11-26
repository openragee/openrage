//! Carrying out a create or a delete across every file it touches.
//!
//! A [`WorkingSet`] holds the documents an edit can reach — the map, the maps
//! that name it as parent, the manifests that state them — and every
//! operation edits their value trees in place. Nothing becomes bytes until
//! [`WorkingSet::finish`], which refuses the whole set if any member would be
//! wrong.
//!
//! ⚠ **Deleting an entity renumbers rather than refusing.** An entity's
//! position in `entities` is its identity, so removing element `i` moves
//! every index above it — local LOD links in the same file and cross-file
//! links in every child map — and every such index is repaired in the same
//! operation. `executing_a_plan_keeps_every_link` holds that every surviving
//! link names the same entity it named before, by identity.
//!
//! ⚠ **A grown or shrunk array reaches the file through synthesis**, which
//! is [`crate::write::encode`]'s fallback, and synthesis needs the schema
//! snapshot. With none staged, [`WorkingSet::finish`] refuses with the
//! writer's own reason and writes nothing.

use std::collections::{BTreeMap, BTreeSet};

use or_io::meta::binary::MetaLayout;
use or_io::meta::{MetaInstance, Value};

use crate::field;
use crate::group::{GroupProblem, MapGroup};
use crate::keys;
use crate::manifest::{Manifest, StatedDependencies};
use crate::map_data::MapData;
use crate::write::{self, WriteError};

/// Why an edit was not carried out.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("no map named {0:#010X} in the set")]
    NoSuchMap(u32),
    #[error("map {map:#010X} holds {len} entities, so there is no entity {index}")]
    NoSuchEntity { map: u32, index: usize, len: usize },
    #[error("map {0:#010X} is split over several files that each hold entities, so an index does not say which")]
    AmbiguousParts(u32),
    #[error("entity {index} of map {map:#010X} is the LOD parent of {children} entity(ies); delete them first")]
    LodParent {
        map: u32,
        index: usize,
        children: u32,
    },
    #[error("map {0:#010X} carries container LODs, whose indices this edit does not renumber")]
    ContainerLods(u32),
    #[error("map {map:#010X} is the LOD parent of {children} map(s) that are not being deleted")]
    MapIsAParent { map: u32, children: usize },
    #[error("a map named {0:#010X} is already in the set; a name is a map, not a file")]
    NameTaken(u32),
    #[error("no document labelled `{0}` in the set")]
    NoSuchDocument(String),
    #[error("`{0}` is not a manifest")]
    NotAManifest(String),
    #[error("no manifest in the set states the donor map {0:#010X}, and the entry of a new map is a copy of the entry of its donor")]
    DonorUnstated(u32),
    #[error("the donor names parent {donor:#010X} and carries cross-file LOD links into it; as a child of {asked:#010X} they would name other entities")]
    ForeignLinks { donor: u32, asked: u32 },
    #[error("no manifest in the set states map {0:#010X} in `imapDependencies_2`, so there is no entry to add its types to")]
    Unstated(u32),
    #[error("an entity to add names LOD parent {index} past the {len} being added")]
    AddedParentOutOfRange { index: usize, len: usize },
    #[error("an entity to add links into a parent map, which a map it is added to need not have")]
    AddedCrossFileLink,
    #[error("after the edit, {0}")]
    Broken(String),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error("`{label}` does not read back: {reason}")]
    DoesNotReadBack { label: String, reason: String },
}

/// One document an edit may touch.
#[derive(Debug, Clone)]
pub struct Document {
    pub label: String,
    pub root: MetaInstance,
    pub layout: MetaLayout,
    changed: bool,
    removed: bool,
}

impl Document {
    pub fn is_changed(&self) -> bool {
        self.changed && !self.removed
    }

    pub fn is_removed(&self) -> bool {
        self.removed
    }
}

/// What a delete renumbered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Renumbered {
    /// Local LOD links in the same file moved down by one.
    pub local: u64,
    /// Cross-file LOD links in child maps moved down by one.
    pub cross_file: u64,
    /// Whether a parent's `numChildren` was lowered for the removed child.
    pub parent_count_lowered: bool,
}

/// What a subtree delete removed and moved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubtreeDeleted {
    pub entities: u64,
    pub renumbered: Renumbered,
}

/// What recomputing the derived fields did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refreshed {
    pub maps: u64,
    pub flags_changed: u64,
    pub extents_recomputed: u64,
    pub extents_changed: u64,
    /// Maps that shrank and name an archetype the index lacks: stored box kept.
    pub extents_kept: u64,
    pub unresolved: u64,
}

/// An entity's identity, as [`identity`] computes it.
pub type Key = (u32, [u32; 3], u32);

/// The documents an edit reaches, and the edits made to them so far.
#[derive(Debug, Clone, Default)]
pub struct WorkingSet {
    docs: Vec<Document>,
    before: Vec<GroupProblem>,
    created: BTreeSet<u32>,
    deleted: BTreeSet<u32>,
    /// Labels of documents whose entity array an edit grew.
    grown: BTreeSet<String>,
}

impl WorkingSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a document as it was read, before any edit.
    pub fn add(&mut self, label: impl Into<String>, root: MetaInstance, layout: MetaLayout) {
        self.docs.push(Document {
            label: label.into(),
            root,
            layout,
            changed: false,
            removed: false,
        });
    }

    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.docs.iter()
    }

    pub fn document(&self, label: &str) -> Option<&Document> {
        self.docs.iter().find(|d| d.label == label && !d.removed)
    }

    /// The group the set assembles right now.
    pub fn group(&self) -> MapGroup {
        let mut group = MapGroup::new();
        for doc in self.live() {
            if let Some(map) = MapData::new(&doc.root) {
                group.add_from(&map, &doc.label);
            }
        }
        group
    }

    /// Every statement the set's manifests make right now.
    pub fn stated(&self) -> StatedDependencies {
        let mut stated = StatedDependencies::new();
        for doc in self.live() {
            if let Some(m) = Manifest::new(&doc.root) {
                stated.add(m);
            }
        }
        stated
    }

    fn live(&self) -> impl Iterator<Item = &Document> {
        self.docs.iter().filter(|d| !d.removed)
    }

    fn baseline(&mut self) {
        if !self.docs.iter().any(|d| d.changed || d.removed) {
            self.before = self.group().check();
        }
    }

    fn map_docs(&self, name: u32) -> Vec<usize> {
        self.docs
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.removed && MapData::new(&d.root).is_some_and(|m| m.name() == name))
            .map(|(i, _)| i)
            .collect()
    }

    fn entity_doc(&self, name: u32) -> Result<usize, ExecError> {
        let docs = self.map_docs(name);
        if docs.is_empty() {
            return Err(ExecError::NoSuchMap(name));
        }
        let holding: Vec<usize> = docs
            .iter()
            .copied()
            .filter(|i| MapData::new(&self.docs[*i].root).is_some_and(|m| m.entity_count() > 0))
            .collect();
        match holding.as_slice() {
            [] => Ok(docs[0]),
            [one] => Ok(*one),
            _ => Err(ExecError::AmbiguousParts(name)),
        }
    }

    /// Remove one entity and renumber every index that named a position
    /// above it, in this file and in every child map.
    pub fn delete_entity(&mut self, map: u32, index: usize) -> Result<Renumbered, ExecError> {
        self.baseline();
        let doc = self.entity_doc(map)?;
        let (len, parent_map, lod_in_parent, parent_index, containers) = {
            let data = MapData::new(&self.docs[doc].root).ok_or(ExecError::NoSuchMap(map))?;
            let len = data.entity_count();
            let Some(entity) = data.entities().nth(index) else {
                return Err(ExecError::NoSuchEntity { map, index, len });
            };
            (
                len,
                data.parent(),
                entity.lod_in_parent_map(),
                entity.parent_index(),
                data.container_lod_count(),
            )
        };
        if containers > 0 {
            return Err(ExecError::ContainerLods(map));
        }
        let children = self.group().children_naming(map, index as u32);
        if children > 0 {
            return Err(ExecError::LodParent {
                map,
                index,
                children,
            });
        }
        let _ = len;

        let mut out = Renumbered::default();
        if let Some(p) = parent_index {
            let target = if lod_in_parent {
                self.entity_doc(parent_map).ok()
            } else {
                Some(doc)
            };
            if let Some(t) = target {
                if let Some(parent) = entity_mut(&mut self.docs[t].root, p) {
                    let n = field::u32_at(parent, keys::NUM_CHILDREN).unwrap_or(0);
                    if n > 0 {
                        set_int(parent, keys::NUM_CHILDREN, n as i64 - 1);
                        out.parent_count_lowered = true;
                        self.docs[t].changed = true;
                    }
                }
            }
        }

        {
            let root = &mut self.docs[doc].root;
            let Some(Value::Array(items)) = field::get_mut(root, keys::ENTITIES) else {
                return Err(ExecError::NoSuchMap(map));
            };
            items.remove(index);
            for item in items.iter_mut() {
                let Some(ent) = struct_mut(item) else {
                    continue;
                };
                if in_parent_map(ent) {
                    continue;
                }
                if let Some(p) = parent_index_of(ent) {
                    if p > index as i64 {
                        set_int(ent, keys::PARENT_INDEX, p - 1);
                        out.local += 1;
                    }
                }
            }
            self.docs[doc].changed = true;
        }

        for child in self.child_docs(map) {
            let mut moved = 0;
            if let Some(Value::Array(items)) =
                field::get_mut(&mut self.docs[child].root, keys::ENTITIES)
            {
                for item in items.iter_mut() {
                    let Some(ent) = struct_mut(item) else {
                        continue;
                    };
                    if !in_parent_map(ent) {
                        continue;
                    }
                    if let Some(p) = parent_index_of(ent) {
                        if p > index as i64 {
                            set_int(ent, keys::PARENT_INDEX, p - 1);
                            moved += 1;
                        }
                    }
                }
            }
            if moved > 0 {
                out.cross_file += moved;
                self.docs[child].changed = true;
            }
        }
        Ok(out)
    }

    /// Remove an entity with every entity that descends from it, in this map
    /// and in its child maps, renumbering as each one goes.
    ///
    /// ⚠ **This is the delete that moves indices.** A shipped map lists its
    /// LOD parents ahead of the entities nothing names, so deleting a leaf
    /// moves an index in 3 of 7,528 retail Legacy maps; deleting a building
    /// takes its LOD parent with it, and that shifts the rest of the array.
    /// Children go first, so each single delete is one the refusal allows.
    pub fn delete_subtree(&mut self, map: u32, index: usize) -> Result<SubtreeDeleted, ExecError> {
        let mut doomed: Vec<(u32, Key)> = Vec::new();
        let root_key = {
            let doc = self.entity_doc(map)?;
            let data = MapData::new(&self.docs[doc].root).ok_or(ExecError::NoSuchMap(map))?;
            let len = data.entity_count();
            let e =
                data.entities()
                    .nth(index)
                    .ok_or(ExecError::NoSuchEntity { map, index, len })?;
            identity(e.instance())
        };
        let mut frontier = vec![(map, index)];
        doomed.push((map, root_key));
        while let Some((owner, at)) = frontier.pop() {
            for (child_map, child_index, key) in self.lod_children(owner, at) {
                doomed.push((child_map, key));
                frontier.push((child_map, child_index));
            }
        }

        let mut out = SubtreeDeleted::default();
        // Deepest first: the last found are the furthest from the root.
        for (owner, key) in doomed.iter().rev() {
            let doc = self.entity_doc(*owner)?;
            let data = MapData::new(&self.docs[doc].root).ok_or(ExecError::NoSuchMap(*owner))?;
            let Some(at) = data.entities().position(|e| identity(e.instance()) == *key) else {
                continue;
            };
            let moved = self.delete_entity(*owner, at)?;
            out.entities += 1;
            out.renumbered.local += moved.local;
            out.renumbered.cross_file += moved.cross_file;
        }
        Ok(out)
    }

    /// The entities naming `(map, index)` as their LOD parent, as
    /// `(map, index, identity)`, in whichever document of the set holds them.
    pub fn lod_children(&self, map: u32, index: usize) -> Vec<(u32, usize, Key)> {
        let mut out = Vec::new();
        for doc in self.live() {
            let Some(data) = MapData::new(&doc.root) else {
                continue;
            };
            let local = data.name() == map;
            let child_map = data.parent() == map;
            if !local && !child_map {
                continue;
            }
            for (i, e) in data.entities().enumerate() {
                let hit = e.parent_index() == Some(index)
                    && ((local && !e.lod_in_parent_map()) || (child_map && e.lod_in_parent_map()));
                if hit {
                    out.push((data.name(), i, identity(e.instance())));
                }
            }
        }
        out
    }

    fn child_docs(&self, parent: u32) -> Vec<usize> {
        self.docs
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                !d.removed && MapData::new(&d.root).is_some_and(|m| m.parent() == parent)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Append entities to a map, their local LOD links taken relative to the
    /// block being added. Returns the index the first one lands at.
    ///
    /// ⚠ A link into a parent map is refused: the entities come from
    /// somewhere else, and the map they join need not have that parent.
    pub fn add_entities(
        &mut self,
        map: u32,
        entities: Vec<MetaInstance>,
    ) -> Result<usize, ExecError> {
        self.baseline();
        for ent in &entities {
            if in_parent_map(ent) && parent_index_of(ent).is_some() {
                return Err(ExecError::AddedCrossFileLink);
            }
            if let Some(p) = parent_index_of(ent) {
                if p as usize >= entities.len() {
                    return Err(ExecError::AddedParentOutOfRange {
                        index: p as usize,
                        len: entities.len(),
                    });
                }
            }
        }
        let doc = self.entity_doc(map)?;
        let root = &mut self.docs[doc].root;
        let Some(Value::Array(items)) = field::get_mut(root, keys::ENTITIES) else {
            return Err(ExecError::NoSuchMap(map));
        };
        let base = items.len();
        let pointer = matches!(items.first(), Some(Value::Pointer(_)) | None);
        for mut ent in entities {
            if let Some(p) = parent_index_of(&ent) {
                set_int(&mut ent, keys::PARENT_INDEX, p + base as i64);
            }
            items.push(if pointer {
                Value::Pointer(Some(Box::new(ent)))
            } else {
                Value::Struct(Box::new(ent))
            });
        }
        self.docs[doc].changed = true;
        self.grown.insert(self.docs[doc].label.clone());
        Ok(base)
    }

    /// Place a prefab into a map, as [`crate::prefab::place`] does.
    pub fn place_prefab(
        &mut self,
        map: u32,
        prefab: &MetaInstance,
        at: glam::Vec3,
        heading: f32,
    ) -> Result<crate::prefab::Placed, ExecError> {
        self.baseline();
        let doc = self.entity_doc(map)?;
        let placed = crate::prefab::place(&mut self.docs[doc].root, prefab, at, heading)
            .map_err(|e| ExecError::Broken(e.to_string()))?;
        self.docs[doc].changed = true;
        self.grown.insert(self.docs[doc].label.clone());
        Ok(placed)
    }

    /// Add type files to what the manifests state a map depends on.
    ///
    /// ⚠ **Placing something is not finished until the streamer is told to
    /// load its model.** An archetype whose type file the map's manifest
    /// entry does not name is a placement that renders nothing. The missing
    /// types go into the first entry stating the map; returns how many went.
    pub fn state_types(&mut self, map: u32, types: &BTreeSet<u32>) -> Result<usize, ExecError> {
        self.baseline();
        let have = self
            .stated()
            .of(map)
            .cloned()
            .ok_or(ExecError::Unstated(map))?;
        let missing: Vec<u32> = types.difference(&have).copied().collect();
        if missing.is_empty() {
            return Ok(0);
        }
        for doc in self.docs.iter_mut().filter(|d| !d.removed) {
            if Manifest::new(&doc.root).is_none() {
                continue;
            }
            let Some(Value::Array(items)) =
                field::get_mut(&mut doc.root, keys::IMAP_DEPENDENCIES_2)
            else {
                continue;
            };
            let Some(entry) = items
                .iter_mut()
                .filter_map(struct_mut)
                .find(|e| field::hash_at(e, keys::IMAP_NAME) == Some(map))
            else {
                continue;
            };
            if let Some(Value::Array(deps)) = field::get_mut(entry, keys::ITYP_DEP_ARRAY) {
                let like = deps.first().cloned();
                for t in &missing {
                    deps.push(hash_value_like(like.as_ref(), *t));
                }
                doc.changed = true;
                return Ok(missing.len());
            }
        }
        Err(ExecError::Unstated(map))
    }

    /// Add a map cloned from a donor, and state it in a manifest.
    ///
    /// ⚠ **A map is a file and an entry.** The entry is cloned from the one
    /// stating the donor, `manifestFlags` and all: the new map places what its
    /// donor places, so the flags that follow what a map places
    /// ([`crate::manifest_build::interior_flags`]) are the donor's. They are
    /// not derived again when a later operation changes what the map places.
    /// The types are what the new map places, which the caller derives.
    #[allow(clippy::too_many_arguments)]
    pub fn create_map(
        &mut self,
        label: impl Into<String>,
        mut root: MetaInstance,
        layout: MetaLayout,
        name: u32,
        parent: u32,
        types: &BTreeSet<u32>,
        manifest: &str,
    ) -> Result<(), ExecError> {
        self.baseline();
        if !self.map_docs(name).is_empty() {
            return Err(ExecError::NameTaken(name));
        }
        let (donor, donor_parent, foreign) = {
            let data = MapData::new(&root).ok_or(ExecError::NoSuchMap(name))?;
            (
                data.name(),
                data.parent(),
                data.entities()
                    .any(|e| e.lod_in_parent_map() && e.parent_index().is_some()),
            )
        };
        if foreign && donor_parent != parent {
            return Err(ExecError::ForeignLinks {
                donor: donor_parent,
                asked: parent,
            });
        }
        let template = self
            .live()
            .filter_map(|d| Manifest::new(&d.root))
            .flat_map(|m| {
                field::structs_at(m.instance(), keys::IMAP_DEPENDENCIES_2).collect::<Vec<_>>()
            })
            .find(|e| field::hash_at(e, keys::IMAP_NAME) == Some(donor))
            .cloned()
            .ok_or(ExecError::DonorUnstated(donor))?;

        let target = self
            .docs
            .iter()
            .position(|d| d.label == manifest && !d.removed)
            .ok_or_else(|| ExecError::NoSuchDocument(manifest.to_string()))?;
        if Manifest::new(&self.docs[target].root).is_none() {
            return Err(ExecError::NotAManifest(manifest.to_string()));
        }

        let mut entry = template;
        set_hash(&mut entry, keys::IMAP_NAME, name);
        if let Some(Value::Array(deps)) = field::get_mut(&mut entry, keys::ITYP_DEP_ARRAY) {
            let like = deps.first().cloned();
            deps.clear();
            for t in types {
                deps.push(hash_value_like(like.as_ref(), *t));
            }
        }
        match field::get_mut(&mut self.docs[target].root, keys::IMAP_DEPENDENCIES_2) {
            Some(Value::Array(items)) => items.push(Value::Struct(Box::new(entry))),
            _ => return Err(ExecError::NotAManifest(manifest.to_string())),
        }
        self.docs[target].changed = true;

        set_hash(&mut root, keys::NAME, name);
        set_hash(&mut root, keys::PARENT, parent);
        self.docs.push(Document {
            label: label.into(),
            root,
            layout,
            changed: true,
            removed: false,
        });
        self.created.insert(name);
        Ok(())
    }

    /// Remove a map: every file declaring its name, every manifest entry
    /// stating it, and the world-state group named for it.
    ///
    /// Returns the labels of the files that go.
    pub fn delete_map(&mut self, name: u32) -> Result<Vec<String>, ExecError> {
        self.baseline();
        let docs = self.map_docs(name);
        if docs.is_empty() {
            return Err(ExecError::NoSuchMap(name));
        }
        let children: Vec<usize> = self
            .child_docs(name)
            .into_iter()
            .filter(|i| !docs.contains(i))
            .collect();
        if !children.is_empty() {
            return Err(ExecError::MapIsAParent {
                map: name,
                children: children.len(),
            });
        }
        let mut labels = Vec::new();
        for i in docs {
            self.docs[i].removed = true;
            labels.push(self.docs[i].label.clone());
        }
        for doc in self.docs.iter_mut().filter(|d| !d.removed) {
            if Manifest::new(&doc.root).is_none() {
                continue;
            }
            let mut touched = false;
            for key in [keys::IMAP_DEPENDENCIES_2, keys::IMAP_DEPENDENCIES] {
                if let Some(Value::Array(items)) = field::get_mut(&mut doc.root, key) {
                    let before = items.len();
                    items.retain(|v| {
                        field::as_struct(v).and_then(|e| field::hash_at(e, keys::IMAP_NAME))
                            != Some(name)
                    });
                    touched |= items.len() != before;
                }
            }
            if let Some(Value::Array(items)) = field::get_mut(&mut doc.root, keys::MAP_DATA_GROUPS)
            {
                let before = items.len();
                items.retain(|v| {
                    field::as_struct(v).and_then(|g| field::hash_at(g, keys::GROUP_NAME))
                        != Some(name)
                });
                touched |= items.len() != before;
            }
            doc.changed |= touched;
        }
        self.deleted.insert(name);
        Ok(labels)
    }

    /// Recompute the flag words and extents of every map an edit changed.
    ///
    /// ⚠ **Extents a map cannot resolve are kept when it shrank and refused
    /// when it grew.** A delete leaves the stored box a superset of the
    /// truth, which streams the map a little early; an addition whose box
    /// stays stale streams the new entities late or never, which is the
    /// failure [`write::apply_extents`] exists to refuse. Flags are additive
    /// either way, for the reason [`write::apply_flags`] gives.
    pub fn refresh_derived(
        &mut self,
        index: &crate::archetypes::ArchetypeIndex,
    ) -> Result<Refreshed, ExecError> {
        let mut out = Refreshed::default();
        for doc in self.docs.iter_mut().filter(|d| d.changed && !d.removed) {
            if MapData::new(&doc.root).is_none() {
                continue;
            }
            out.maps += 1;
            out.flags_changed += write::apply_flags(&mut doc.root)? as u64;
            match write::apply_extents(&mut doc.root, index) {
                Ok(changed) => {
                    out.extents_recomputed += 1;
                    out.extents_changed += changed as u64;
                }
                Err(WriteError::IncompleteExtents(n)) if !self.grown.contains(&doc.label) => {
                    out.extents_kept += 1;
                    out.unresolved += n as u64;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(out)
    }

    /// Judge the set as the edits left it, and encode what changed.
    ///
    /// ⚠ **Judged by outcome, against the set as it arrived.** A group
    /// problem the set already had is not this edit's, so only new ones
    /// refuse — the same reason [`write::refuse_if_broken`] is per-file. A
    /// created map has to be stated and a deleted one must not be.
    pub fn finish(&self) -> Result<Vec<(String, Vec<u8>)>, ExecError> {
        let after = self.group().check();
        let new: Vec<&GroupProblem> = after
            .iter()
            .filter(|p| {
                !self.before.contains(p) && !matches!(p, GroupProblem::ChildCountOverstated { .. })
            })
            .collect();
        if let Some(first) = new.first() {
            return Err(ExecError::Broken(format!(
                "{} group problem(s), first: {first}",
                new.len()
            )));
        }
        let stated = self.stated();
        for map in &self.created {
            if !stated.is_stated(*map) {
                return Err(ExecError::Broken(format!(
                    "new map {map:#010X} is stated by no manifest"
                )));
            }
        }
        for map in &self.deleted {
            if stated.is_stated(*map) {
                return Err(ExecError::Broken(format!(
                    "deleted map {map:#010X} is still stated"
                )));
            }
        }

        let mut out = Vec::new();
        for doc in self.docs.iter().filter(|d| d.is_changed()) {
            let bytes = write::encode(&doc.root, &doc.layout)?;
            let mut diags = or_io::meta::Diagnostics::lenient();
            if let Err(e) = or_io::meta::binary::parse_meta_binary_full(
                or_io::meta::default_library(),
                &bytes,
                &mut diags,
            ) {
                return Err(ExecError::DoesNotReadBack {
                    label: doc.label.clone(),
                    reason: e.to_string(),
                });
            }
            out.push((doc.label.clone(), bytes));
        }
        Ok(out)
    }

    /// The labels of documents the edits removed.
    pub fn removed(&self) -> Vec<String> {
        self.docs
            .iter()
            .filter(|d| d.removed)
            .map(|d| d.label.clone())
            .collect()
    }
}

/// Which entity is which, independent of where it sits in the array.
///
/// ⚠ **Position is not identity.** Archetype, placement and `guid` together
/// are what a test compares a link's target by before and after the numbers
/// moved.
pub fn identity(ent: &MetaInstance) -> Key {
    let pos = field::vec3_at(ent, keys::POSITION).unwrap_or([0.0; 3]);
    (
        field::hash_at(ent, keys::ARCHETYPE_NAME).unwrap_or(0),
        [pos[0].to_bits(), pos[1].to_bits(), pos[2].to_bits()],
        field::u32_at(ent, keys::GUID).unwrap_or(0),
    )
}

/// Every LOD link in a set of maps, as `(child, parent)` identities.
///
/// A link whose target is not held is left out: it cannot be named.
pub fn links_by_identity<'a>(
    maps: impl IntoIterator<Item = &'a MetaInstance>,
) -> BTreeMap<Key, Key> {
    let maps: Vec<&MetaInstance> = maps.into_iter().collect();
    let mut by_name: BTreeMap<u32, &MetaInstance> = BTreeMap::new();
    for m in &maps {
        if let Some(data) = MapData::new(m) {
            if data.entity_count() > 0 {
                by_name.insert(data.name(), m);
            }
        }
    }
    let mut out = BTreeMap::new();
    for m in &maps {
        let Some(data) = MapData::new(m) else {
            continue;
        };
        let own: Vec<&MetaInstance> = data.entities().map(|e| e.instance()).collect();
        let parent: Vec<&MetaInstance> = by_name
            .get(&data.parent())
            .and_then(|p| MapData::new(p))
            .map(|p| p.entities().map(|e| e.instance()).collect())
            .unwrap_or_default();
        for e in data.entities() {
            let Some(p) = e.parent_index() else {
                continue;
            };
            let target = if e.lod_in_parent_map() {
                parent.get(p)
            } else {
                own.get(p)
            };
            if let Some(t) = target {
                out.insert(identity(e.instance()), identity(t));
            }
        }
    }
    out
}

fn entity_mut(root: &mut MetaInstance, index: usize) -> Option<&mut MetaInstance> {
    match field::get_mut(root, keys::ENTITIES)? {
        Value::Array(items) => struct_mut(items.get_mut(index)?),
        _ => None,
    }
}

fn struct_mut(value: &mut Value) -> Option<&mut MetaInstance> {
    match value {
        Value::Struct(inner) => Some(inner),
        Value::Pointer(Some(inner)) => Some(inner),
        _ => None,
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

/// ⚠ Through the field's own slot and in its own signedness, for the reason
/// `write::set_u32` gives: a type table's field cannot be added or retyped.
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

fn hash_value_like(like: Option<&Value>, hash: u32) -> Value {
    match like {
        Some(Value::UInt(_)) => Value::UInt(hash as u64),
        _ => Value::Hash(or_io::meta::instance::value::HashedString::from_hash(hash)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;

    const IN_PARENT: u32 = 1 << 3;

    fn hash(name: &str) -> u32 {
        or_io::string_hash(name.as_bytes())
    }

    fn ent(arch: &str, parent_index: i32, num_children: u32, flags: u32) -> Value {
        let mut e = MetaInstance::new("CEntityDef");
        e.set("archetypeName", Value::Hash(HashedString::new(arch)));
        e.set("position", Value::Vec3([0.0; 3]));
        e.set("guid", Value::UInt(0));
        e.set("parentIndex", Value::Int(parent_index as i64));
        e.set("numChildren", Value::UInt(num_children as u64));
        e.set("flags", Value::UInt(flags as u64));
        Value::Pointer(Some(Box::new(e)))
    }

    fn map(name: &str, parent: Option<&str>, entities: Vec<Value>) -> MetaInstance {
        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::Hash(HashedString::new(name)));
        root.set(
            "parent",
            Value::Hash(match parent {
                Some(p) => HashedString::new(p),
                None => HashedString::from_hash(0),
            }),
        );
        root.set("entities", Value::Array(entities));
        root
    }

    fn layout() -> MetaLayout {
        MetaLayout {
            format: or_io::meta::BinaryFormat::Rbf,
            container: or_io::meta::binary::ContainerLayout::Rbf(Default::default()),
        }
    }

    /// `lod` holds a LOD tree; `hd` links across files into it.
    fn set() -> WorkingSet {
        let lod = map(
            "lod",
            None,
            vec![
                ent("a", -1, 1, 0),
                ent("leaf", -1, 0, 0),
                ent("b", -1, 2, 0),
                ent("b_child", 2, 0, 0),
            ],
        );
        let hd = map(
            "hd",
            Some("lod"),
            vec![
                ent("hd_of_b", 2, 0, IN_PARENT),
                ent("hd_of_a", 0, 0, IN_PARENT),
            ],
        );
        let mut ws = WorkingSet::new();
        ws.add("lod.ymap", lod, layout());
        ws.add("hd.ymap", hd, layout());
        ws
    }

    fn links(ws: &WorkingSet) -> BTreeMap<Key, Key> {
        links_by_identity(ws.documents().filter(|d| !d.is_removed()).map(|d| &d.root))
    }

    /// Deleting the leaf at 1 moves `b_child`'s local link and `hd_of_b`'s
    /// cross-file link from 2 to 1, and every link still names what it did.
    #[test]
    fn deleting_an_entity_renumbers_every_link_above_it() {
        let mut ws = set();
        let before = links(&ws);
        let moved = ws.delete_entity(hash("lod"), 1).unwrap();
        assert_eq!(moved.local, 1);
        assert_eq!(moved.cross_file, 1);
        assert_eq!(links(&ws), before);
        assert!(ws.group().check().is_empty(), "{:?}", ws.group().check());
    }

    /// ⚠ The control: the same delete without the renumbering leaves the
    /// set individually readable and the links naming their neighbours.
    #[test]
    fn a_delete_that_did_not_renumber_would_change_what_links_name() {
        let mut ws = set();
        let before = links(&ws);
        let doc = ws.entity_doc(hash("lod")).unwrap();
        if let Some(Value::Array(items)) = field::get_mut(&mut ws.docs[doc].root, keys::ENTITIES) {
            items.remove(1);
        }
        assert_ne!(links(&ws), before);
    }

    /// ⚠ Deleting `a` takes its cross-file child with it, then everything
    /// above index 0 moves: `b_child`'s local link and `hd_of_b`'s cross-file
    /// link both drop from 2 to 1, and still name `b`.
    #[test]
    fn deleting_a_subtree_takes_the_children_and_renumbers_the_rest() {
        let mut ws = set();
        let before = links(&ws);
        let gone = ws.delete_subtree(hash("lod"), 0).unwrap();
        assert_eq!(gone.entities, 2);
        assert_eq!(gone.renumbered.local, 1);
        assert_eq!(gone.renumbered.cross_file, 1);

        let mut expected = before;
        let hd = ws.document("hd.ymap").unwrap();
        assert_eq!(MapData::new(&hd.root).unwrap().entity_count(), 1);
        expected.retain(|child, _| child.0 != hash("hd_of_a"));
        assert_eq!(links(&ws), expected);
        assert!(ws.group().check().is_empty(), "{:?}", ws.group().check());
    }

    #[test]
    fn deleting_a_lod_parent_with_children_is_refused() {
        let mut ws = set();
        let err = ws.delete_entity(hash("lod"), 2).unwrap_err();
        assert!(
            matches!(err, ExecError::LodParent { children: 2, .. }),
            "{err}"
        );
    }

    #[test]
    fn deleting_a_child_lowers_its_parents_count() {
        let mut ws = set();
        let moved = ws.delete_entity(hash("hd"), 0).unwrap();
        assert!(moved.parent_count_lowered);
        let lod = ws.document("lod.ymap").unwrap();
        let b = MapData::new(&lod.root).unwrap().entities().nth(2).unwrap();
        assert_eq!(b.num_children(), 1);
        assert!(ws.group().check().is_empty());
    }

    #[test]
    fn added_entities_keep_their_links_among_themselves() {
        let mut ws = set();
        let block = [ent("p", -1, 1, 0), ent("c", 0, 0, 0)]
            .into_iter()
            .map(|v| field::as_struct(&v).unwrap().clone())
            .collect();
        let base = ws.add_entities(hash("lod"), block).unwrap();
        assert_eq!(base, 4);
        let lod = ws.document("lod.ymap").unwrap();
        let c = MapData::new(&lod.root).unwrap().entities().nth(5).unwrap();
        assert_eq!(c.parent_index(), Some(4));
        assert!(ws.group().check().is_empty());
    }

    #[test]
    fn an_added_entity_linking_into_a_parent_map_is_refused() {
        let mut ws = set();
        let block = vec![field::as_struct(&ent("x", 0, 0, IN_PARENT))
            .unwrap()
            .clone()];
        assert!(matches!(
            ws.add_entities(hash("lod"), block),
            Err(ExecError::AddedCrossFileLink)
        ));
    }

    #[test]
    fn deleting_a_parent_map_with_children_is_refused() {
        let mut ws = set();
        assert!(matches!(
            ws.delete_map(hash("lod")),
            Err(ExecError::MapIsAParent { children: 1, .. })
        ));
        ws.delete_map(hash("hd")).unwrap();
        assert_eq!(
            ws.delete_map(hash("lod")).unwrap(),
            vec!["lod.ymap".to_string()]
        );
    }

    fn manifest(entries: &[(&str, &[&str])], groups: &[&str]) -> MetaInstance {
        let items = entries
            .iter()
            .map(|(m, types)| {
                let mut inst = MetaInstance::new("CImapDependencies");
                inst.set("imapName", Value::Hash(HashedString::new(*m)));
                inst.set("manifestFlags", Value::UInt(0));
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
        let groups = groups
            .iter()
            .map(|g| {
                let mut inst = MetaInstance::new("CMapDataGroup");
                inst.set("Name", Value::Hash(HashedString::new(*g)));
                Value::Struct(Box::new(inst))
            })
            .collect();
        let mut root = MetaInstance::new("CPackFileMetaData");
        root.set("MapDataGroups", Value::Array(groups));
        root.set("imapDependencies", Value::Array(vec![]));
        root.set("imapDependencies_2", Value::Array(items));
        root
    }

    #[test]
    fn creating_a_map_states_it_with_the_donors_flags_and_the_derived_types() {
        let mut ws = set();
        ws.add("m.ymf", manifest(&[("hd", &["t_old"])], &[]), layout());
        let donor = ws.document("hd.ymap").unwrap().root.clone();
        // The clone keeps the donor's parent, so its cross-file links still
        // name what they named.
        ws.create_map(
            "hd2.ymap",
            donor,
            layout(),
            hash("hd2"),
            hash("lod"),
            &BTreeSet::from([hash("t1"), hash("t2")]),
            "m.ymf",
        )
        .unwrap();
        let stated = ws.stated();
        assert_eq!(
            stated.of(hash("hd2")),
            Some(&BTreeSet::from([hash("t1"), hash("t2")]))
        );
        // Two children now name `lod`'s entities, so the counts the donor's
        // links relied on are understated and the outcome check says so.
        let err = ws.finish().unwrap_err();
        assert!(err.to_string().contains("group problem"), "{err}");
    }

    #[test]
    fn stale_extents_are_kept_after_a_delete_and_refused_after_an_addition() {
        let index = crate::archetypes::ArchetypeIndex::new();
        let mut shrunk = set();
        let moved = shrunk.delete_entity(hash("lod"), 1).unwrap();
        // `hd`'s link to 2 moved as well, so both files changed.
        assert_eq!(moved.cross_file, 1);
        let r = shrunk.refresh_derived(&index).unwrap();
        assert_eq!((r.maps, r.extents_kept, r.extents_recomputed), (2, 2, 0));

        let mut grown = set();
        let block = vec![field::as_struct(&ent("new", -1, 0, 0)).unwrap().clone()];
        grown.add_entities(hash("lod"), block).unwrap();
        assert!(matches!(
            grown.refresh_derived(&index),
            Err(ExecError::Write(WriteError::IncompleteExtents(_)))
        ));
    }

    #[test]
    fn stating_a_placed_type_adds_only_what_is_missing() {
        let mut ws = set();
        ws.add("m.ymf", manifest(&[("hd", &["t_old"])], &[]), layout());
        let want = BTreeSet::from([hash("t_old"), hash("t_new")]);
        assert_eq!(ws.state_types(hash("hd"), &want).unwrap(), 1);
        assert_eq!(ws.stated().of(hash("hd")), Some(&want));
        assert_eq!(ws.state_types(hash("hd"), &want).unwrap(), 0);
        assert!(matches!(
            ws.state_types(hash("lod"), &want),
            Err(ExecError::Unstated(_))
        ));
    }

    #[test]
    fn a_clone_under_another_parent_with_cross_file_links_is_refused() {
        let mut ws = set();
        ws.add("m.ymf", manifest(&[("hd", &[])], &[]), layout());
        let donor = ws.document("hd.ymap").unwrap().root.clone();
        let err = ws
            .create_map(
                "x.ymap",
                donor,
                layout(),
                hash("x"),
                0,
                &BTreeSet::new(),
                "m.ymf",
            )
            .unwrap_err();
        assert!(matches!(err, ExecError::ForeignLinks { .. }), "{err}");
    }

    #[test]
    fn a_donor_no_manifest_states_is_refused() {
        let mut ws = set();
        ws.add("m.ymf", manifest(&[], &[]), layout());
        let donor = ws.document("lod.ymap").unwrap().root.clone();
        let err = ws
            .create_map(
                "x.ymap",
                donor,
                layout(),
                hash("x"),
                0,
                &BTreeSet::new(),
                "m.ymf",
            )
            .unwrap_err();
        assert!(matches!(err, ExecError::DonorUnstated(_)), "{err}");
    }

    /// ⚠ Deleting a map takes its manifest entry and its state group with it,
    /// or the set states a map it no longer holds.
    #[test]
    fn deleting_a_map_removes_its_entry_and_its_group() {
        let mut ws = set();
        ws.add(
            "m.ymf",
            manifest(&[("hd", &["t1"]), ("lod", &["t2"])], &["hd"]),
            layout(),
        );
        ws.delete_map(hash("hd")).unwrap();
        let stated = ws.stated();
        assert!(!stated.is_stated(hash("hd")));
        assert!(stated.is_stated(hash("lod")));
        let m = ws.document("m.ymf").unwrap();
        assert_eq!(Manifest::new(&m.root).unwrap().state_groups().count(), 0);
        assert!(ws.document("m.ymf").unwrap().is_changed());
        assert_eq!(ws.removed(), vec!["hd.ymap".to_string()]);
    }
}
