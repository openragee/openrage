//! The edits of the Map workspace: an entity placed, added or taken out, an archetype added, changed or taken out, and the Undo and Redo
//! that move along them.
//!
//! ⚠ **Thin by design.** Each edit is an operation of `or_maps::operation`, the vocabulary `openrage-cli maps apply` applies a list of,
//! done to the session the workspace holds its pack open in (`maps_session.rs`): the map file or the type file a source cooks to
//! changes in memory, and the source's XML is written when the project is saved. A step of Undo keeps the inverses the session gave,
//! each of which states the state of its source it applies to, so a step applies only in its order. What an operation went without or
//! left to another file, its notes, is said as a warning where `maps apply` prints a note.
//!
//! ⚠ **Delete is a plan.** An entity goes with its chain (`or_maps::delete`): what names it as its far model, and each far model
//! left standing for nothing, across the map files of the pack, as one step.

use std::path::{Path, PathBuf};

use or_maps::delete::{self, Why};
use or_maps::edit::Pose;
use or_maps::operation::{At, Done, Error, Operation, Record};
use or_maps::Maps;
use or_project::ModuleKind;

use super::maps::{Pick, PickKind, Step, KEPT};
use crate::state::AppState;

/// The reason alone of a record the session could not give, which is what a toast reads.
fn unread(refused: Error) -> String {
    match refused {
        Error::Source { why, .. } => why,
        other => other.to_string(),
    }
}

impl AppState {
    /// The folder of the shown pack, which an operation is applied to.
    fn maps_pack_folder(&self) -> Option<PathBuf> {
        Some(self.maps_pack()?.folder.clone())
    }

    /// Do `operation` to the session of the pack in `folder` as one step called `label`, unless it changes nothing; whether it was done, and what it added.
    fn do_map_step(
        &mut self,
        label: String,
        folder: &Path,
        operation: Operation,
    ) -> Result<(bool, Option<usize>), String> {
        let operations = vec![operation];
        let done = self.do_in_the_map_session(folder, &operations, false)?;
        let Some((inverse, added)) = done.first().map(|did| (did.inverse.clone(), did.added))
        else {
            return Ok((false, None));
        };
        if self.puts_back_what_is_held(folder, &inverse) {
            // ⚠ Taken back in the session, which then has nothing of it left to write: a field left as it was is no step and no unsaved change.
            self.do_in_the_map_session(folder, &[inverse], false)?;
            return Ok((false, None));
        }
        self.push_maps_applied(label, folder, operations, done);
        Ok((true, added))
    }

    /// Whether `inverse` would put back the record its source already holds, with no extents to restore: the operation it takes back changed nothing.
    fn puts_back_what_is_held(&mut self, folder: &Path, inverse: &Operation) -> bool {
        let Ok(open) = self.workbench.maps.sessions.open(folder) else {
            return false;
        };
        let held = match inverse {
            Operation::ReplaceEntity {
                source,
                entity,
                record,
                extents: None,
                ..
            } => {
                let at = At {
                    source: source.clone(),
                    index: *entity,
                };
                let now = open.session.entity(&at).map_err(unread);
                now.and_then(|now| Record::entity(&now))
                    .map(|now| now == *record)
            }
            Operation::ReplaceArchetype {
                source,
                archetype,
                record,
                ..
            } => {
                let at = At {
                    source: source.clone(),
                    index: *archetype,
                };
                let now = open.session.archetype(&at).map_err(unread);
                now.and_then(|now| Record::archetype(&now))
                    .map(|now| now == *record)
            }
            _ => Ok(false),
        };
        held.unwrap_or(false)
    }

    /// Stand the entity `index` of the map file `source` at `pose`, as one step Undo takes back.
    pub fn place_entity_in(&mut self, source: &str, index: usize, pose: Pose, label: String) {
        // A scanned GTA VI map is its document's tree, which its own step of Undo holds.
        if self.scanned_map().is_some() {
            if let Err(why) = self.place_scanned_entity(index, pose, label) {
                self.show_error_toast(why);
            }
            self.workbench.maps.preview = None;
            return;
        }
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        let operation = Operation::PlaceEntity {
            source: source.to_string(),
            entity: index,
            pose: pose.into(),
            expects: None,
        };
        match self.do_map_step(label, &folder, operation) {
            Ok((true, _)) => {
                let set = self.workbench.maps.set.clone().unwrap_or_default();
                self.workbench.maps.module = Some(module.clone());
                if self.workbench.maps.selected.is_none() {
                    self.select_in_maps(
                        &module,
                        &set,
                        Some(Pick {
                            source: source.to_string(),
                            kind: PickKind::Entity(index),
                        }),
                    );
                }
            }
            Ok(_) => {}
            Err(why) => self.show_error_toast(why),
        }
        self.workbench.maps.preview = None;
    }

    /// Stand the entity selected at `pose`, as one step.
    pub fn place_selected_entity(&mut self, pose: Pose, label: String) {
        let Some(Pick {
            source,
            kind: PickKind::Entity(index),
        }) = self.workbench.maps.selected.clone()
        else {
            return;
        };
        self.place_entity_in(&source, index, pose, label);
    }

    /// Add an entity of `archetype` at `pose` to the map file the selection is in (else the first of the shown set), as a copy of the entity selected (else the first one the set holds).
    pub fn add_entity_at(&mut self, archetype: or_maps::Name, pose: Pose) {
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some((pack, set)) = self.shown_set() else {
            return;
        };
        let read = pack.read.as_ref().ok();
        // The map file to add to: the selection's, else the first map file of the set.
        let target = self
            .workbench
            .maps
            .selected
            .as_ref()
            .map(|p| p.source.clone())
            .filter(|s| {
                read.and_then(|r| r.facts.source(s))
                    .is_some_and(|f| f.map().is_some())
            })
            .or_else(|| {
                set.sources
                    .iter()
                    .find(|s| {
                        read.and_then(|r| r.facts.source(s))
                            .is_some_and(|f| f.map().is_some())
                    })
                    .cloned()
            });
        let set_id = set.id.clone();
        let sources = set.sources.clone();
        let Some(target) = target else {
            self.show_warning_toast(tr!(
                "This set has no map file to add an entity to: add a .ymap first"
            ));
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        // The record to copy: the entity selected, else the first the target holds, else the first any map file of the set holds.
        let selected = match self.workbench.maps.selected.clone() {
            Some(Pick {
                source,
                kind: PickKind::Entity(index),
            }) => Some(At { source, index }),
            _ => None,
        };
        let first = |source: &String| At {
            source: source.clone(),
            index: 0,
        };
        let mut candidates = (selected.into_iter())
            .chain([first(&target)])
            .chain(sources.iter().map(first));
        let donor = match self.workbench.maps.sessions.open(&folder) {
            Ok(open) => candidates.find(|at| open.session.entity(at).is_ok()),
            Err(why) => return self.show_error_toast(why),
        };
        let Some(donor) = donor else {
            self.show_warning_toast(tr!(
                "No entity in this set can be copied: add the first from a map file that has one"
            ));
            return;
        };
        let text = archetype.text.clone();
        let label = tr!("Added an entity of {name}", name = archetype.shown());
        let operation = Operation::AddEntity {
            source: target.clone(),
            archetype: archetype.shown(),
            pose: pose.into(),
            donor: Some(donor),
            expects: None,
        };
        match self.do_map_step(label, &folder, operation) {
            Ok((_, added)) => {
                self.remember_map_names(&module, &[text.as_deref().unwrap_or_default()]);
                let added = added.map(PickKind::Entity);
                self.select_in_maps(
                    &module,
                    &set_id,
                    added.map(|kind| Pick {
                        source: target,
                        kind,
                    }),
                );
            }
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Take the entity selected out of the map files of its pack with its chain, as one step: what names it as its far model goes with it, and so does each far model left standing for nothing.
    pub fn remove_selected_entity(&mut self) {
        let Some(Pick {
            source,
            kind: PickKind::Entity(index),
        }) = self.workbench.maps.selected.clone()
        else {
            return;
        };
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        let chosen = [At { source, index }];
        let planned = (self.workbench.maps.sessions.open(&folder)).and_then(|open| {
            delete::plan_in(&mut open.session, &chosen).map_err(|refused| refused.to_string())
        });
        let planned = match planned {
            Ok(planned) => planned,
            Err(why) => return self.show_error_toast(why),
        };
        if planned.operations.is_empty() {
            // The plan leaves an entity it cannot tell from another, and then takes nothing.
            if let Some(left) = planned.ambiguous.first() {
                self.show_warning_toast(tr_n!(
                    "Entity {index} was left: {count} other entity places the same model at the same pose, and a deletion would not know which of them goes",
                    "Entity {index} was left: {count} other entities place the same model at the same pose, and a deletion would not know which of them goes",
                    left.twins.len(),
                    index = index
                ));
            }
            return;
        }
        match self.do_in_the_map_session(&folder, &planned.operations, false) {
            Ok(done) => {
                let label = tr!("Removed entity {index}", index = index);
                self.push_maps_applied(label, &folder, planned.operations, done);
                for note in &planned.notes {
                    self.show_warning_toast(note.to_string());
                }
                let went = |why: fn(&Why) -> bool| {
                    (planned.goes.iter())
                        .filter(|going| why(&going.why))
                        .count()
                };
                let far = went(|why| *why == Why::LeftWithNoChild);
                if far > 0 {
                    self.show_warning_toast(tr_n!(
                        "{count} far model that stood for nothing else went with it",
                        "{count} far models that stood for nothing else went with it",
                        far
                    ));
                }
                let under = went(|why| matches!(why, Why::Under(_)));
                if under > 0 {
                    self.show_warning_toast(tr_n!(
                        "{count} entity it was the far model of went with it",
                        "{count} entities it was the far model of went with it",
                        under
                    ));
                }
                let set = self.workbench.maps.set.clone().unwrap_or_default();
                self.select_in_maps(&module, &set, None);
            }
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Place another one of the entity selected (else the first the set places) where the view looks, as one step.
    pub fn add_entity_here(&mut self) {
        let Some(scene) = self.shown_scene() else {
            return;
        };
        let archetype = self
            .selected_placed()
            .or_else(|| scene.placed.first())
            .map(|p| p.archetype.clone());
        let Some(archetype) = archetype else {
            self.show_warning_toast(tr!(
                "This set places nothing to make another of: add a map file that places something"
            ));
            return;
        };
        let work = &self.workbench.maps;
        let mut at = work.camera.target;
        if work.snap.grid_on {
            at.x = (at.x / work.snap.grid).round() * work.snap.grid;
            at.y = (at.y / work.snap.grid).round() * work.snap.grid;
        }
        let pose = Pose {
            position: at,
            rotation: glam::Quat::IDENTITY,
            scale_xy: 1.0,
            scale_z: 1.0,
        };
        self.add_entity_at(archetype, pose);
    }

    /// Declare an archetype under a name nobody in the set holds, as one step.
    pub fn add_archetype_here(&mut self) {
        let Some((pack, set)) = self.shown_set() else {
            return;
        };
        let held: Vec<String> = pack
            .read
            .as_ref()
            .ok()
            .into_iter()
            .flat_map(|read| {
                set.sources
                    .iter()
                    .filter_map(|s| read.facts.source(s)?.types())
                    .flat_map(|t| t.archetypes.iter())
            })
            .map(|a| a.name.shown().to_ascii_lowercase())
            .collect();
        let mut n = 1;
        let name = loop {
            let name = if n == 1 {
                "prop_new".to_string()
            } else {
                format!("prop_new_{n}")
            };
            if !held.contains(&name) {
                break name;
            }
            n += 1;
        };
        self.add_archetype_named(&name, None);
    }

    /// Change the archetype selected with `change`, as one step; `typed` are the names the change writes, which the pack remembers.
    pub fn edit_selected_archetype(
        &mut self,
        label: String,
        typed: &[&str],
        change: impl FnOnce(&mut or_io::meta::typed::CBaseArchetypeDefAny),
    ) {
        let Some(Pick {
            source,
            kind: PickKind::Archetype(index),
        }) = self.workbench.maps.selected.clone()
        else {
            return;
        };
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        // The record as the person changed it, which the operation puts in the place of the one the session holds.
        let at = At {
            source: source.clone(),
            index,
        };
        let held = (self.workbench.maps.sessions.open(&folder))
            .and_then(|open| open.session.archetype(&at).map_err(unread));
        let changed = held.and_then(|mut archetype| {
            change(&mut archetype);
            Record::archetype(&archetype)
        });
        let done = changed.and_then(|record| {
            let operation = Operation::ReplaceArchetype {
                source,
                archetype: index,
                record,
                expects: None,
            };
            self.do_map_step(label, &folder, operation)
        });
        match done {
            Ok((true, _)) => self.remember_map_names(&module, typed),
            Ok(_) => {}
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Give the entity selected the archetype `name`, as one step.
    pub fn rename_selected_entity_archetype(&mut self, name: &str) {
        let name = name.trim();
        let Some(Pick {
            source,
            kind: PickKind::Entity(index),
        }) = self.workbench.maps.selected.clone()
        else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        let at = At {
            source: source.clone(),
            index,
        };
        let held = (self.workbench.maps.sessions.open(&folder))
            .and_then(|open| open.session.entity(&at).map_err(unread));
        let renamed = held.and_then(|mut entity| {
            entity.archetype_name = or_io::meta::instance::HashedString::new(name);
            Record::entity(&entity)
        });
        let done = renamed.and_then(|record| {
            let operation = Operation::ReplaceEntity {
                source,
                entity: index,
                record,
                extents: None,
                expects: None,
            };
            let label = tr!(
                "Placed {name} as entity {index}",
                name = name,
                index = index
            );
            self.do_map_step(label, &folder, operation)
        });
        match done {
            Ok((true, _)) => self.remember_map_names(&module, &[name]),
            Ok(_) => {}
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Remember the names a person typed in the pack, so the hash a file keeps of each shows as the name.
    fn remember_map_names(&mut self, module: &str, typed: &[&str]) {
        let Some(project) = self.project_path.as_deref() else {
            return;
        };
        for name in typed.iter().filter(|n| !n.trim().is_empty()) {
            self.name_index.add(
                crate::state::name_index::NameIndexCategory::Model,
                name.trim(),
            );
        }
        let Some(module) = (self.project_carried.container.modules.iter())
            .find(|m| m.id == module && m.kind == ModuleKind::Maps)
        else {
            return;
        };
        let Ok(folder) = or_maps::module_folder(&or_project::project_folder(project), module)
        else {
            return;
        };
        if let Ok(mut model) = Maps::read(&folder) {
            if model.remember(typed) {
                let _ = model.write(&folder);
                // The scan reads the names the pack remembers, and no edit asks for one any more.
                self.workbench.want_scan();
            }
        }
    }

    /// Add an archetype called `name` to the type file the selection is in (else the first of the shown set), as a copy of the archetype selected, as one step.
    pub fn add_archetype_named(&mut self, name: &str, asset: Option<&str>) {
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some((pack, set)) = self.shown_set() else {
            return;
        };
        let read = pack.read.as_ref().ok();
        let is_types = |s: &String| {
            read.and_then(|r| r.facts.source(s))
                .is_some_and(|f| f.types().is_some())
        };
        let target = self
            .workbench
            .maps
            .selected
            .as_ref()
            .map(|p| p.source.clone())
            .filter(is_types)
            .or_else(|| set.sources.iter().find(|s| is_types(s)).cloned());
        let set_id = set.id.clone();
        let Some(target) = target else {
            self.show_warning_toast(tr!(
                "This set has no type file to add an archetype to: add a .ytyp first"
            ));
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        // The record to copy: the archetype selected, when its file still holds it.
        let selected = match self.workbench.maps.selected.clone() {
            Some(Pick {
                source,
                kind: PickKind::Archetype(index),
            }) => Some(At { source, index }),
            _ => None,
        };
        let donor = match self.workbench.maps.sessions.open(&folder) {
            Ok(open) => selected.filter(|at| open.session.archetype(at).is_ok()),
            Err(why) => return self.show_error_toast(why),
        };
        let operation = Operation::AddArchetype {
            source: target.clone(),
            name: name.to_string(),
            asset: asset.map(str::to_string),
            donor,
            expects: None,
        };
        let label = tr!("Added the archetype {name}", name = name);
        match self.do_map_step(label, &folder, operation) {
            Ok((_, added)) => {
                self.remember_map_names(&module, &[name, asset.unwrap_or(name)]);
                let added = added.map(PickKind::Archetype);
                self.select_in_maps(
                    &module,
                    &set_id,
                    added.map(|kind| Pick {
                        source: target,
                        kind,
                    }),
                );
            }
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Take the archetype selected out of its type file, as one step.
    pub fn remove_selected_archetype(&mut self) {
        let Some(Pick {
            source,
            kind: PickKind::Archetype(index),
        }) = self.workbench.maps.selected.clone()
        else {
            return;
        };
        let Some((module, _)) = self.maps_folder_to_write() else {
            return;
        };
        let Some(folder) = self.maps_pack_folder() else {
            return;
        };
        let operation = Operation::RemoveArchetype {
            source,
            archetype: index,
            expects: None,
        };
        let label = tr!("Removed archetype {index}", index = index);
        match self.do_map_step(label, &folder, operation) {
            Ok(_) => {
                let set = self.workbench.maps.set.clone().unwrap_or_default();
                self.select_in_maps(&module, &set, None);
            }
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Stand the entity selected on the ground under it, as one step.
    pub fn drop_selected_to_ground(&mut self) {
        let Some(Pick {
            kind: PickKind::Entity(index),
            ..
        }) = self.workbench.maps.selected.clone()
        else {
            return;
        };
        let Some(scene) = self.shown_scene() else {
            return;
        };
        let (Some(pose), Some(at)) = (
            self.selected_pose(),
            self.workbench
                .maps
                .selected
                .as_ref()
                .and_then(|p| scene.find(&p.source, index)),
        ) else {
            return;
        };
        let dropped =
            or_maps::gizmo::drop_to_ground(&scene.obbs(), at, pose, self.workbench.maps.snap.floor);
        if dropped.position == pose.position {
            return;
        }
        self.place_selected_entity(
            dropped,
            tr!("Stood entity {index} on the ground", index = index).to_string(),
        );
    }

    /// Keep `operations`, which the session of the pack in `folder` did, as one step of Undo, and say what they went without.
    fn push_maps_applied(
        &mut self,
        label: String,
        folder: &Path,
        operations: Vec<Operation>,
        done: Vec<Done>,
    ) {
        for step in &done {
            self.say_maps_notes(&step.notes);
        }
        self.push_maps_step(Step {
            label,
            folder: folder.to_path_buf(),
            undo: (done.into_iter().rev().map(|step| step.inverse)).collect(),
            redo: operations,
        });
    }

    /// Say the notes of an operation, the sentences of `or_maps::operation`, each as a warning.
    fn say_maps_notes(&mut self, notes: &[String]) {
        for note in notes {
            self.show_warning_toast(note.clone());
        }
    }

    fn push_maps_step(&mut self, step: Step) {
        let work = &mut self.workbench.maps;
        work.undo.push(step);
        if work.undo.len() > KEPT {
            work.undo.remove(0);
        }
        work.redo.clear();
        work.generation += 1;
    }

    /// Take the change of `step` back (`undo`) or make it again in the session of its pack, by the operations the step keeps, all of them or none; the step then keeps the operations that go the other way.
    fn apply_maps_step(&mut self, step: &mut Step, undo: bool) -> Result<(), String> {
        let operations = if undo { &step.undo } else { &step.redo };
        let done = self.do_in_the_map_session(&step.folder, operations, true)?;
        for did in &done {
            self.say_maps_notes(&did.notes);
        }
        let other_way = (done.into_iter().rev().map(|did| did.inverse)).collect();
        if undo {
            step.redo = other_way;
        } else {
            step.undo = other_way;
        }
        self.workbench.maps.generation += 1;
        self.workbench.maps.preview = None;
        Ok(())
    }

    /// Take back the last change to a map.
    pub fn undo_maps_change(&mut self) {
        if self.refused_as_read_only() {
            return;
        }
        let Some(mut step) = self.workbench.maps.undo.pop() else {
            return;
        };
        match self.apply_maps_step(&mut step, true) {
            Ok(()) => {
                self.show_toast(tr!("Undid: {what}", what = step.label.as_str()));
                self.workbench.maps.redo.push(step);
            }
            Err(why) => {
                self.workbench.maps.undo.push(step);
                self.show_error_toast(why);
            }
        }
    }

    /// Make the change Undo took back again.
    pub fn redo_maps_change(&mut self) {
        if self.refused_as_read_only() {
            return;
        }
        let Some(mut step) = self.workbench.maps.redo.pop() else {
            return;
        };
        match self.apply_maps_step(&mut step, false) {
            Ok(()) => {
                self.show_toast(tr!("Did again: {what}", what = step.label.as_str()));
                self.workbench.maps.undo.push(step);
            }
            Err(why) => {
                self.workbench.maps.redo.push(step);
                self.show_error_toast(why);
            }
        }
    }
}
