//! A scanned GTA VI map's entity moved, turned, scaled or retyped on the Map screen: each edit sets
//! the entity's members in the document's metadata tree as one step of Undo, the Map screen places
//! the tree's values, and Save As writes the edited tree as a loose resource.
//!
//! ⚠ An entity takes an archetype of its class family that its map names or that an open or found
//! map types body holds: a map's string table holds no entry of the archetype words' namespace, so
//! the writer adds no text for a new word. An interior takes another only where its rooms and
//! portals were read: its exit portals are the new layout's portals that touch room 0, and its
//! default entity sets keep only the names the new layout declares.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use glam::{Quat, Vec3};
use or_io::gta6::map::links::Interior;
use or_io::gta6::map::{ArchetypeClass, Entity, MapData};
use or_io::hash::literal_string_hash as lit;
use or_io::meta::binary::hash_index::default_hash_index;
use or_io::meta::instance::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_world::transform::Pose;

use super::meta_edit::value_at;
use super::presets::MetaStep;
use super::{AppState, Change, DocPayload, DocumentId, MetaEdit};

const ENTITIES: u32 = lit(b"entities");
const POSITION: u32 = lit(b"position");
const ROTATION: u32 = lit(b"rotation");
const SCALE_XY: u32 = lit(b"scaleXY");
const SCALE_Z: u32 = lit(b"scaleZ");
const ARCHETYPE_NAME: u32 = lit(b"archetypeName");
/// The archetype's 64-bit name word an entity names it by.
const ARCHETYPE_WORD: u32 = 0xC8A7_B90A;
const NUM_EXIT_PORTALS: u32 = lit(b"numExitPortals");
const DEFAULT_ENTITY_SETS: u32 = lit(b"defaultEntitySets");

/// `n` as the kind of number `stored` holds.
fn number_like(stored: Option<&Value>, n: u64) -> Value {
    match stored {
        Some(Value::Int(_)) => Value::Int(n as i64),
        _ => Value::UInt(n),
    }
}

/// A member's name as the tree reader names it.
fn name(hash: u32) -> String {
    let _names = or_io::meta::binary::hash_index::title_names();
    default_hash_index().name_or_literal(hash)
}

/// Member `member` of entity `index` in the map's tree.
fn member_of(tree: &MetaInstance, index: usize, member: u32) -> Option<&Value> {
    let Some(Value::Array(items)) = tree.get(&name(ENTITIES)) else {
        return None;
    };
    match items.get(index)? {
        Value::Struct(inst) | Value::Pointer(Some(inst)) => inst.get(&name(member)),
        _ => None,
    }
}

/// The name hash a `defaultEntitySets` element holds.
fn set_name(v: &Value) -> Option<u32> {
    match v {
        Value::Hash(h) => Some(h.hash),
        Value::UInt(n) => Some(*n as u32),
        Value::Int(n) => Some(*n as u32),
        _ => None,
    }
}

/// The values a placement stores: a `CEntityDef` keeps the conjugate of the quaternion that turns
/// its model, an interior the quaternion itself.
fn placement(interior: bool, pose: Pose) -> Result<[(u32, Value); 4], String> {
    let q = pose.rotation.normalize();
    let finite = pose.position.is_finite()
        && q.is_finite()
        && pose.scale_xy.is_finite()
        && pose.scale_z.is_finite();
    if !finite {
        return Err(
            tr!("A placement whose numbers are not all finite is not written.").to_string(),
        );
    }
    let rotation = match interior {
        true => [q.x, q.y, q.z, q.w],
        false => [-q.x, -q.y, -q.z, q.w],
    };
    Ok([
        (POSITION, Value::Vec3(pose.position.to_array())),
        (ROTATION, Value::Vec4(rotation)),
        (SCALE_XY, Value::Float(f64::from(pose.scale_xy))),
        (SCALE_Z, Value::Float(f64::from(pose.scale_z))),
    ])
}

/// `m` with each entity's placement and archetype as `tree` holds them.
pub(super) fn synced(m: &MapData, tree: &MetaInstance) -> MapData {
    let mut m = m.clone();
    if let Some((entities, streaming)) = super::gta6_map_extents::extents_of(tree) {
        (m.entities_extents, m.streaming_extents) = (entities, streaming);
    }
    let Some(Value::Array(items)) = tree.get(&name(ENTITIES)) else {
        return m;
    };
    let names = [
        POSITION,
        ROTATION,
        SCALE_XY,
        SCALE_Z,
        ARCHETYPE_NAME,
        ARCHETYPE_WORD,
    ]
    .map(name);
    for (e, item) in m.entities.iter_mut().zip(items) {
        if let Value::Struct(inst) | Value::Pointer(Some(inst)) = item {
            read_into(e, inst, &names);
        }
    }
    m
}

fn read_into(e: &mut Entity, inst: &MetaInstance, names: &[String; 6]) {
    let get = |k: usize| inst.get(&names[k]);
    if let Some(Value::Vec3(p)) = get(0) {
        e.position = *p;
    }
    if let Some(Value::Vec4(q)) = get(1) {
        e.rotation = *q;
    }
    if let Some(Value::Float(s)) = get(2) {
        e.scale_xy = *s as f32;
    }
    if let Some(Value::Float(s)) = get(3) {
        e.scale_z = *s as f32;
    }
    if let Some(Value::Hash(h)) = get(4) {
        e.archetype_name = h.hash;
    }
    match get(5) {
        Some(Value::UInt(word)) => e.archetype_word = *word,
        Some(Value::String(text)) => {
            e.archetype_word = or_io::gta6::hash::name_hash64(text.as_bytes())
        }
        _ => {}
    }
}

impl AppState {
    /// The revision of the metadata tree of the document at `path`, and the tree once it is edited.
    pub(super) fn edited_map_tree(&self, path: &Path) -> (u64, Option<&MetaInstance>) {
        let meta = self.documents.iter().find_map(|d| match &d.payload {
            DocPayload::Meta(meta) if d.path.as_deref() == Some(path) => Some(meta),
            _ => None,
        });
        match meta.and_then(|m| Some((m.revision(), &m.parsed.as_ref()?.instance))) {
            Some((0, _)) | None => (0, None),
            Some((revision, tree)) => (revision, Some(tree)),
        }
    }

    /// Whether a drag may place the entity selected: a scanned map's instance lists are not placed.
    pub(crate) fn places_selected(&self) -> bool {
        let Some(m) = self.scanned_map().map(|s| s.data()) else {
            return true;
        };
        let entities = m.map_or(0, |m| m.entities.len());
        self.selected_placed().is_some_and(|p| p.index < entities)
    }

    /// The document the scanned map shown opened as.
    fn scanned_map_document(&self) -> Result<DocumentId, String> {
        let path = (self.scanned_map().map(|s| &s.path))
            .ok_or_else(|| tr!("No scanned GTA VI map is shown.").to_string())?;
        (self.documents.iter())
            .find(|d| d.path.as_ref() == Some(path) && matches!(d.payload, DocPayload::Meta(_)))
            .map(|d| d.id)
            .ok_or_else(|| tr!("The scanned map's document is closed.").to_string())
    }

    /// Stand entity `index` of the scanned map shown at `pose`, the map's extents grown to hold it,
    /// as one step of Undo.
    pub(crate) fn place_scanned_entity(
        &mut self,
        index: usize,
        pose: Pose,
        label: String,
    ) -> Result<(), String> {
        let scanned = self.scanned_map();
        let e = (scanned.and_then(|s| s.data()))
            .and_then(|m| m.entities.get(index))
            .cloned()
            .ok_or_else(|| tr!("The map holds no entity {index}.", index = index))?;
        let archetype = (scanned.and_then(|s| s.archetypes.get(index)))
            .and_then(Option::as_ref)
            .map(|(_, a)| a.clone());
        let values = placement(e.interior, pose)?;
        let tree =
            (self.scanned_map_document().ok()).and_then(|doc| match &self.document(doc)?.payload {
                DocPayload::Meta(meta) => meta.parsed.as_ref().map(|p| &p.instance),
                _ => None,
            });
        let root = (tree.and_then(super::gta6_map_extents::extents_of))
            .map_or_else(Vec::new, |held| {
                super::gta6_map_extents::grown(held, &e, archetype.as_ref(), pose)
            });
        self.set_scanned_entity(index, &values, &root, label)
    }

    /// The layout of the interior archetype of word `word` that an open body or the shown map's
    /// archive holds, where its rooms and portals were read.
    fn known_interior(&self, word: u64) -> Option<Arc<Interior>> {
        let mut held: Vec<&Path> = (self.documents.iter())
            .filter_map(|d| d.path.as_deref())
            .collect();
        let shown = self.scanned_map();
        if let Some((archive, _)) = shown.and_then(|s| super::rpf9_scan::scanned_body(&s.path)) {
            held.push(archive);
        }
        super::gta6_map::links::interior_in(&held, word)
    }

    /// The archetypes entity `index` of the scanned map shown may take, as (word, name).
    // An interior its own and every interior archetype whose layout was read; any other entity
    // those its map's entities name and every base or time archetype an open or found map types
    // body holds.
    pub(crate) fn retype_choices(&self, index: usize) -> Vec<(u64, u32)> {
        let Some(e) =
            (self.scanned_map().and_then(|s| s.data())).and_then(|m| m.entities.get(index))
        else {
            return Vec::new();
        };
        if e.interior {
            let mut kinds = vec![(e.archetype_word, e.archetype_name)];
            self.each_known_archetype(|a| {
                let laid_out = || self.known_interior(a.name_word).is_some();
                if a.class == ArchetypeClass::Interior && laid_out() {
                    kinds.push((a.name_word, a.name));
                }
            });
            kinds.sort_unstable();
            kinds.dedup();
            return kinds;
        }
        let named = (self.scanned_map().and_then(|s| s.data()).into_iter())
            .flat_map(|m| m.entities.iter())
            .filter(|o| !o.interior)
            .map(|o| (o.archetype_word, o.archetype_name));
        let mut kinds: Vec<(u64, u32)> = named.collect();
        self.each_known_archetype(|a| {
            if a.class != ArchetypeClass::Interior {
                kinds.push((a.name_word, a.name));
            }
        });
        kinds.sort_unstable();
        kinds.dedup();
        kinds
    }

    /// Give entity `index` of the shown map the archetype `name` of word `word`, one step of Undo.
    // One of `retype_choices`; the map's extents grow over its box and stream distance
    // where its archetype is known.
    pub(crate) fn retype_scanned_entity(
        &mut self,
        index: usize,
        word: u64,
        name: u32,
    ) -> Result<(), String> {
        let m = (self.scanned_map().and_then(|s| s.data()))
            .ok_or_else(|| tr!("No scanned GTA VI map is shown.").to_string())?;
        let e = (m.entities.get(index))
            .ok_or_else(|| tr!("The map holds no entity {index}.", index = index))?
            .clone();
        let interior_only = || {
            tr!("An interior takes only its own archetype or another interior archetype whose rooms and portals were read.").to_string()
        };
        if !self.retype_choices(index).contains(&(word, name)) {
            if e.interior {
                return Err(interior_only());
            }
            return Err(tr!("An entity takes only an archetype of its class that its map names or that an open or found map types body holds, and an interior keeps its own.").to_string());
        }
        let mut archetype = None;
        self.each_known_archetype(|a| {
            if archetype.is_none() && (a.name_word, a.name) == (word, name) {
                archetype = Some(a.clone());
            }
        });
        let pose = Pose {
            position: Vec3::from(e.position),
            rotation: Quat::from_array(e.model_rotation()),
            scale_xy: e.scale_xy,
            scale_z: e.scale_z,
        };
        let root = (self
            .scanned_tree()
            .and_then(super::gta6_map_extents::extents_of))
        .map_or_else(Vec::new, |held| {
            super::gta6_map_extents::grown(held, &e, archetype.as_ref(), pose)
        });
        let mut values = vec![
            (ARCHETYPE_NAME, Value::Hash(HashedString::from_hash(name))),
            (ARCHETYPE_WORD, Value::UInt(word)),
        ];
        if e.interior && word != e.archetype_word {
            let layout = self.known_interior(word).ok_or_else(interior_only)?;
            // The exit portals are those that touch room 0, the limbo room.
            let exits = (layout.portals.iter())
                .filter(|p| p.room_from == 0 || p.room_to == 0)
                .count() as u64;
            let declared: HashSet<u32> = layout.entity_sets.iter().map(|s| s.name).collect();
            let tree = self.scanned_tree();
            let stored = tree.and_then(|t| member_of(t, index, NUM_EXIT_PORTALS));
            values.push((NUM_EXIT_PORTALS, number_like(stored, exits)));
            if let Some(Value::Array(sets)) =
                tree.and_then(|t| member_of(t, index, DEFAULT_ENTITY_SETS))
            {
                let kept: Vec<Value> = (sets.iter())
                    .filter(|v| set_name(v).is_none_or(|n| declared.contains(&n)))
                    .cloned()
                    .collect();
                if kept.len() != sets.len() {
                    values.push((DEFAULT_ENTITY_SETS, Value::Array(kept)));
                }
            }
        }
        let label = tr!("Retyped entity {index}", index = index);
        self.set_scanned_entity(index, &values, &root, label)
    }

    /// The scanned map's metadata tree, where its document is open and read.
    fn scanned_tree(&self) -> Option<&MetaInstance> {
        match &self.document(self.scanned_map_document().ok()?)?.payload {
            DocPayload::Meta(meta) => meta.parsed.as_ref().map(|p| &p.instance),
            _ => None,
        }
    }

    /// Set the members `values` of entity `index` and `root` of the map in the scanned map's tree,
    /// one step of Undo; nothing where each already holds its value, and nothing at all where one
    /// is not found. A value is never a structure; an array only replaces an array.
    fn set_scanned_entity(
        &mut self,
        index: usize,
        values: &[(u32, Value)],
        root: &[(u32, Value)],
        label: String,
    ) -> Result<(), String> {
        let doc = self.scanned_map_document()?;
        let document = (self.document_mut(doc))
            .ok_or_else(|| tr!("The scanned map's document is closed.").to_string())?;
        // A scanned body saves its edits as a copy, so its map edits as its form does.
        if let Some(why) = (document.read_only).filter(|w| *w != super::rpf9_scan::READ_ONLY_BODY) {
            return Err(crate::i18n::tr(why).to_string());
        }
        let DocPayload::Meta(meta) = &mut document.payload else {
            return Err(tr!("The scanned map's document is closed.").to_string());
        };
        let tree = (meta.parsed.as_mut().map(|p| &mut p.instance))
            .ok_or_else(|| tr!("The scanned map's metadata did not read.").to_string())?;
        let path_of = |member: u32| {
            vec![
                MetaStep::Field(name(ENTITIES)),
                MetaStep::Index(index),
                MetaStep::Field(name(member)),
            ]
        };
        let set: Vec<(Vec<MetaStep>, u32, &Value)> = (values.iter())
            .map(|(member, value)| (path_of(*member), *member, value))
            .chain(
                (root.iter())
                    .map(|(member, value)| (vec![MetaStep::Field(name(*member))], *member, value)),
            )
            .collect();
        // Every member is found, and holds a value of its own and no structure, before any is set.
        for (path, member, value) in &set {
            let scalar = match (value_at(tree, path), value) {
                // A list of names is set whole.
                (Some(Value::Array(_)), Value::Array(_)) => true,
                (
                    Some(Value::Struct(_) | Value::Pointer(_) | Value::Array(_) | Value::Map(_)),
                    _,
                ) => false,
                (Some(_), _) => true,
                (None, _) => false,
            };
            if !scalar {
                return Err(tr!(
                    "Entity {index} of the map's tree holds no value {member}.",
                    index = index,
                    member = name(*member)
                ));
            }
        }
        let mut edits = Vec::new();
        for (path, _, value) in set {
            if let Some(slot) = value_at(tree, &path).filter(|slot| **slot != *value) {
                let other = std::mem::replace(slot, value.clone());
                edits.push(MetaEdit::Set { path, other });
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        meta.after_visual_edit_in_place();
        self.record(doc, label, Change::Meta(edits));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use glam::{Quat, Vec3};
    use or_io::gta6::map::{Aabb, Archetype, MapBody, MapTypes};

    use super::*;
    use crate::state::document::MetaDocument;
    use crate::state::workbench::{Pick, PickKind};

    const WORD: u64 = 0x0123_4567_89AB_CDEF;
    const OTHER: u64 = 0x0FED_CBA9_8765_4321;

    fn entity(interior: bool, word: u64, name: u32) -> Entity {
        Entity {
            interior,
            id: 0x77,
            rotation: [0.0, 0.0, 0.0, 1.0],
            archetype_word: word,
            position: [10.0, -20.0, 5.0],
            archetype_name: name,
            flags: 0,
            scale_xy: 2.0,
            scale_z: 3.0,
            parent_index: -1,
            hash_e74c3c23: 0,
            lod_dist: -1.0,
            child_lod_dist: 0.0,
            num_children: 0,
            tint_value: 0,
            priority_level: 0,
            lod_level: 5,
        }
    }

    fn map(entities: Vec<Entity>) -> MapData {
        let box_of = |x: f32| Aabb {
            min: [-x; 3],
            max: [x; 3],
        };
        MapData {
            name: 1,
            name_word: 2,
            parent: 0,
            hash_6c0f9c8c: 0,
            hash_9e8d32a0: 0,
            hash_ec16451d: 0,
            hash_36c898aa: 0,
            flags: 0,
            content_flags: 0,
            streaming_extents: box_of(500.0),
            entities_extents: box_of(50.0),
            entities,
            prop_lists: Vec::new(),
            grass_lists: Vec::new(),
        }
    }

    /// The tree a map of `entities` reads to, the members the edits set and nothing else.
    fn tree(entities: &[Entity]) -> MetaInstance {
        let items = entities.iter().map(|e| {
            let mut inst = MetaInstance::new("CEntityDef");
            inst.set(name(POSITION), Value::Vec3(e.position));
            inst.set(name(ROTATION), Value::Vec4(e.rotation));
            inst.set(name(SCALE_XY), Value::Float(f64::from(e.scale_xy)));
            inst.set(name(SCALE_Z), Value::Float(f64::from(e.scale_z)));
            let hashed = HashedString::from_hash(e.archetype_name);
            inst.set(name(ARCHETYPE_NAME), Value::Hash(hashed));
            inst.set(name(ARCHETYPE_WORD), Value::UInt(e.archetype_word));
            Value::Pointer(Some(Box::new(inst)))
        });
        let mut root = MetaInstance::new("CMapData");
        root.set(name(ENTITIES), Value::Array(items.collect()));
        root
    }

    fn shown(state: &AppState) -> &MapData {
        (state.scanned_map().and_then(|s| s.data())).expect("the map is shown")
    }

    #[test]
    fn an_entity_moved_and_retyped_on_the_map_screen_is_one_step_each_of_its_tree() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path: PathBuf = dir.path().join("0x10.rbm");
        let entities = vec![
            entity(false, WORD, 0xAAAA_0001),
            entity(false, OTHER, 0xBBBB_0002),
            entity(true, 0x55, 0xCCCC_0003),
        ];
        super::super::gta6_map::keep_read(&path, MapBody::Data(map(entities.clone())));
        let mut meta = MetaDocument::empty();
        meta.parsed.as_mut().expect("a tree").instance = tree(&entities);
        let mut state = AppState::new();
        let doc = state.insert_document(path.clone(), DocPayload::Meta(meta), None, None);
        state.active_document = Some(doc);
        state.refresh_scanned_map();
        state.pick_shown(
            None,
            None,
            Some(Pick {
                source: path.display().to_string(),
                kind: PickKind::Entity(0),
            }),
        );

        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let pose = Pose {
            position: Vec3::new(1.5, -2.5, 3.5),
            rotation: quarter,
            scale_xy: 0.5,
            scale_z: 4.0,
        };
        state.place_entity_in("ignored", 0, pose, "Moved entity 0".into());
        state.refresh_scanned_map();
        let moved = &shown(&state).entities[0];
        assert_eq!(moved.position, [1.5, -2.5, 3.5]);
        assert_eq!((moved.scale_xy, moved.scale_z), (0.5, 4.0));
        // The model turns by the quaternion placed, and the entity stores its conjugate.
        let turned = Quat::from_array(moved.model_rotation()) * Vec3::X;
        assert!((turned - Vec3::Y).length() < 1e-5, "{turned}");
        assert!(moved.rotation[2] < 0.0, "{:?}", moved.rotation);
        let placed = &state.scanned_map().expect("shown").scene.placed[0];
        assert_eq!(placed.obb.position, Vec3::new(1.5, -2.5, 3.5));

        // Retyped to the second entity's archetype; an archetype no entity of its class names is refused.
        state
            .retype_scanned_entity(0, OTHER, 0xBBBB_0002)
            .expect("an archetype the map names");
        for (word, name) in [
            (0x55, 0xCCCC_0003),
            (0x99, 0xBBBB_0002),
            (OTHER, 0xAAAA_0001),
        ] {
            assert!(state.retype_scanned_entity(0, word, name).is_err());
        }
        state.refresh_scanned_map();
        let retyped = &shown(&state).entities[0];
        assert_eq!(
            (retyped.archetype_word, retyped.archetype_name),
            (OTHER, 0xBBBB_0002)
        );
        assert_eq!(retyped.position, [1.5, -2.5, 3.5], "the move is kept");

        assert!(state.undo().is_some());
        state.refresh_scanned_map();
        assert_eq!(shown(&state).entities[0].archetype_word, WORD);
        assert!(state.undo().is_some());
        state.refresh_scanned_map();
        assert_eq!(shown(&state).entities[0], entities[0], "both steps undone");
        assert!(state.redo().is_some());
        state.refresh_scanned_map();
        assert_eq!(shown(&state).entities[0].position, [1.5, -2.5, 3.5]);
    }

    fn archetype(class: ArchetypeClass, word: u64, name: u32) -> Archetype {
        Archetype {
            class,
            name,
            name_word: word,
            texture_dictionary: 0,
            drawable_dictionary: 0,
            drawable: word,
            fragment: 0,
            lod_dist: 600.0,
            special_attribute: 0,
            bb_min: [-1.0; 3],
            bb_max: [1.0; 3],
            bs_centre: [0.0; 3],
            bs_radius: 1.8,
            flags: 0,
            hd_texture_dist: 0.0,
            clip_dictionary: 0,
            physics_dictionary: 0,
            asset_type: 2,
            asset_name: 0,
        }
    }

    #[test]
    fn an_entity_takes_an_archetype_an_open_map_types_body_holds_and_an_interior_keeps_its_own() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path: PathBuf = dir.path().join("0x40.rbm");
        let types_path: PathBuf = dir.path().join("0x50.rbm");
        let entities = vec![
            entity(false, WORD, 0xAAAA_0001),
            entity(true, 0x55, 0xCCCC_0003),
        ];
        let read = map(entities.clone());
        super::super::gta6_map::keep_read(&path, MapBody::Data(read.clone()));
        let types = MapTypes {
            name: 9,
            archetypes: vec![
                archetype(ArchetypeClass::Base, OTHER, 0xDDDD_0004),
                archetype(ArchetypeClass::Interior, 0x66, 0xEEEE_0005),
            ],
        };
        super::super::gta6_map::keep_read(&types_path, MapBody::Types(types));
        let mut root = tree(&entities);
        let extents = [
            ("entitiesExtentsMin", read.entities_extents.min),
            ("entitiesExtentsMax", read.entities_extents.max),
            ("streamingExtentsMin", read.streaming_extents.min),
            ("streamingExtentsMax", read.streaming_extents.max),
        ];
        for (member, v) in extents {
            root.set(name(lit(member.as_bytes())), Value::Vec3(v));
        }
        let mut meta = MetaDocument::empty();
        meta.parsed.as_mut().expect("a tree").instance = root;
        let mut state = AppState::new();
        let types_doc = DocPayload::Meta(MetaDocument::empty());
        let _ = state.insert_document(types_path, types_doc, None, None);
        let doc = state.insert_document(path.clone(), DocPayload::Meta(meta), None, None);
        state.active_document = Some(doc);
        state.refresh_scanned_map();

        let choices = state.retype_choices(0);
        assert!(choices.contains(&(OTHER, 0xDDDD_0004)), "{choices:?}");
        assert!(choices.contains(&(WORD, 0xAAAA_0001)), "{choices:?}");
        assert!(
            !choices.contains(&(0x66, 0xEEEE_0005)),
            "an interior's archetype"
        );
        assert_eq!(state.retype_choices(1), vec![(0x55, 0xCCCC_0003)]);
        assert!(state.retype_scanned_entity(1, 0x66, 0xEEEE_0005).is_err());

        state
            .retype_scanned_entity(0, OTHER, 0xDDDD_0004)
            .expect("an archetype an open map types body holds");
        state.refresh_scanned_map();
        let retyped = shown(&state);
        assert_eq!(
            (
                retyped.entities[0].archetype_word,
                retyped.entities[0].archetype_name
            ),
            (OTHER, 0xDDDD_0004)
        );
        // Its 600 m stream distance from (10, -20, 5) passes the stored 500 m on every side.
        assert_eq!(retyped.streaming_extents.max, [610.0, 580.0, 605.0]);
        assert_eq!(retyped.streaming_extents.min, [-590.0, -620.0, -595.0]);
        assert_eq!(retyped.entities_extents, read.entities_extents);
        assert_eq!(state.undo().as_deref(), Some("Retyped entity 0"));
        state.refresh_scanned_map();
        assert_eq!(shown(&state).streaming_extents, read.streaming_extents);
    }

    #[test]
    fn an_interior_retyped_takes_the_new_layout_s_exit_portals_and_keeps_the_sets_it_declares() {
        use or_io::gta6::map::links::{EntitySet, Portal};
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path: PathBuf = dir.path().join("0x60.rbm");
        let types_path: PathBuf = dir.path().join("0x70.rbm");
        let entities = vec![entity(true, 0x55, 0xCCCC_0003)];
        super::super::gta6_map::keep_read(&path, MapBody::Data(map(entities.clone())));
        let types = MapTypes {
            name: 9,
            archetypes: vec![archetype(ArchetypeClass::Interior, 0x66, 0xEEEE_0005)],
        };
        super::super::gta6_map::keep_read(&types_path, MapBody::Types(types));
        let portal = |room_from, room_to| Portal {
            room_from,
            room_to,
            flags: 0,
            corners: [[0.0; 3]; 4],
            attached_objects: Vec::new(),
        };
        let layout = Interior {
            archetype: 0,
            name: 0xEEEE_0005,
            name_word: 0x66,
            entities: Vec::new(),
            rooms: Vec::new(),
            // Two of the three touch room 0.
            portals: vec![portal(0, 1), portal(1, 2), portal(2, 0)],
            entity_sets: vec![EntitySet {
                name: 0x51,
                entities: Vec::new(),
            }],
            time_cycle_modifiers: Vec::new(),
        };
        super::super::gta6_map::links::keep_interiors(&types_path, &[layout]);
        let mut root = MetaInstance::new("CMapData");
        let mut inst = MetaInstance::new("CMloInstanceDef");
        for (member, v) in [
            (POSITION, Value::Vec3(entities[0].position)),
            (ROTATION, Value::Vec4(entities[0].rotation)),
            (
                ARCHETYPE_NAME,
                Value::Hash(HashedString::from_hash(0xCCCC_0003)),
            ),
            (ARCHETYPE_WORD, Value::UInt(0x55)),
            (NUM_EXIT_PORTALS, Value::UInt(7)),
        ] {
            inst.set(name(member), v);
        }
        let sets = [0x51, 0x52].map(|h| Value::Hash(HashedString::from_hash(h)));
        inst.set(name(DEFAULT_ENTITY_SETS), Value::Array(sets.to_vec()));
        let items = vec![Value::Pointer(Some(Box::new(inst)))];
        root.set(name(ENTITIES), Value::Array(items));
        let mut meta = MetaDocument::empty();
        meta.parsed.as_mut().expect("a tree").instance = root;
        let mut state = AppState::new();
        let types_doc = DocPayload::Meta(MetaDocument::empty());
        let _ = state.insert_document(types_path, types_doc, None, None);
        let doc = state.insert_document(path, DocPayload::Meta(meta), None, None);
        state.active_document = Some(doc);
        state.refresh_scanned_map();

        let choices = state.retype_choices(0);
        assert_eq!(choices, vec![(0x55, 0xCCCC_0003), (0x66, 0xEEEE_0005)]);
        state
            .retype_scanned_entity(0, 0x66, 0xEEEE_0005)
            .expect("an interior whose layout was read");
        let member = |state: &AppState, m: u32| {
            (state.scanned_tree()).and_then(|t| member_of(t, 0, m).cloned())
        };
        assert_eq!(member(&state, NUM_EXIT_PORTALS), Some(Value::UInt(2)));
        let kept = vec![Value::Hash(HashedString::from_hash(0x51))];
        assert_eq!(
            member(&state, DEFAULT_ENTITY_SETS),
            Some(Value::Array(kept))
        );
        assert_eq!(member(&state, ARCHETYPE_WORD), Some(Value::UInt(0x66)));
        assert_eq!(state.undo().as_deref(), Some("Retyped entity 0"));
        assert_eq!(member(&state, NUM_EXIT_PORTALS), Some(Value::UInt(7)));
    }

    #[test]
    fn a_scanned_entity_stands_on_the_ground_as_one_step_once_it_is_selected() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path: PathBuf = dir.path().join("0x20.rbm");
        let entities = vec![entity(false, WORD, 0xAAAA_0001)];
        super::super::gta6_map::keep_read(&path, MapBody::Data(map(entities.clone())));
        let mut meta = MetaDocument::empty();
        meta.parsed.as_mut().expect("a tree").instance = tree(&entities);
        let mut state = AppState::new();
        let doc = state.insert_document(path.clone(), DocPayload::Meta(meta), None, None);
        state.active_document = Some(doc);
        state.refresh_scanned_map();
        let offered = |state: &AppState| {
            let action = crate::ui::commands::Action::DropToGround;
            let command = crate::ui::commands::of(state, action).expect("the command");
            command.enabled.is_ok()
        };
        assert!(!offered(&state), "no entity selected");
        state.pick_shown(
            None,
            None,
            Some(Pick {
                source: path.display().to_string(),
                kind: PickKind::Entity(0),
            }),
        );
        assert!(offered(&state));

        let pose = state.selected_pose().expect("the entity's pose");
        let boxes = state.scanned_map().expect("shown").scene.obbs();
        let floor = state.workbench.maps.snap.floor;
        let stood = or_world::gizmo::drop_to_ground(&boxes, 0, pose, floor);
        assert_ne!(
            stood.position, pose.position,
            "its box starts above the floor"
        );
        state.drop_selected_to_ground();
        state.refresh_scanned_map();
        assert_eq!(
            shown(&state).entities[0].position,
            stood.position.to_array()
        );
        assert_eq!(
            state.undo().as_deref(),
            Some("Stood entity 0 on the ground")
        );
        state.refresh_scanned_map();
        assert_eq!(shown(&state).entities[0].position, entities[0].position);
    }

    #[test]
    fn a_move_past_the_stored_streaming_extents_grows_them_in_the_same_step() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let path: PathBuf = dir.path().join("0x30.rbm");
        let mut e = entity(false, WORD, 0xAAAA_0001);
        e.lod_dist = 40.0;
        let read = map(vec![e.clone()]);
        super::super::gta6_map::keep_read(&path, MapBody::Data(read.clone()));
        let mut root = tree(std::slice::from_ref(&e));
        let extents = [
            ("entitiesExtentsMin", read.entities_extents.min),
            ("entitiesExtentsMax", read.entities_extents.max),
            ("streamingExtentsMin", read.streaming_extents.min),
            ("streamingExtentsMax", read.streaming_extents.max),
        ];
        for (member, v) in extents {
            root.set(name(lit(member.as_bytes())), Value::Vec3(v));
        }
        let mut meta = MetaDocument::empty();
        meta.parsed.as_mut().expect("a tree").instance = root;
        let mut state = AppState::new();
        let doc = state.insert_document(path.clone(), DocPayload::Meta(meta), None, None);
        state.active_document = Some(doc);
        state.refresh_scanned_map();

        let pose = Pose {
            position: Vec3::new(480.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale_xy: 1.0,
            scale_z: 1.0,
        };
        state
            .place_scanned_entity(0, pose, "Moved entity 0".into())
            .expect("placed");
        state.refresh_scanned_map();
        // 40 m past x = 480 reaches 520; no archetype box is known, so entitiesExtents stays.
        let moved = shown(&state);
        assert_eq!(moved.streaming_extents.max, [520.0, 500.0, 500.0]);
        assert_eq!(moved.streaming_extents.min, [-500.0; 3]);
        assert_eq!(moved.entities_extents, read.entities_extents);
        assert_eq!(moved.entities[0].position, [480.0, 0.0, 0.0]);

        assert_eq!(state.undo().as_deref(), Some("Moved entity 0"));
        state.refresh_scanned_map();
        assert_eq!(shown(&state).streaming_extents, read.streaming_extents);
        assert_eq!(shown(&state).entities[0].position, e.position);
    }

    #[test]
    fn an_interior_stores_the_quaternion_itself_and_a_pose_not_finite_is_refused() {
        let pose = |rotation, scale_xy| Pose {
            position: Vec3::ZERO,
            rotation,
            scale_xy,
            scale_z: 1.0,
        };
        let quarter = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let [_, (_, interior), ..] = placement(true, pose(quarter, 1.0)).expect("finite");
        let [_, (_, entity), ..] = placement(false, pose(quarter, 1.0)).expect("finite");
        let Value::Vec4(i) = interior else {
            panic!("a quaternion")
        };
        let Value::Vec4(e) = entity else {
            panic!("a quaternion")
        };
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() < 1e-6;
        assert!(near([i[2], i[3]], [quarter.z, quarter.w]), "{i:?}");
        assert!(near([e[2], e[3]], [-quarter.z, quarter.w]), "{e:?}");
        assert!(placement(false, pose(quarter, f32::NAN)).is_err());
    }
}
