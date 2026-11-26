//! The edits of the Map workspace: an entity placed, added or taken out, an archetype added, changed or taken out, and the Undo and Redo
//! that move along them.
//!
//! ⚠ **Thin by design.** Each edit is an operation of `or_maps::operation`, applied by the one function `openrage-cli maps apply` applies
//! a list with, which changes the map file or the type file a source cooks to and writes the result back as the source's XML. A step
//! of Undo keeps the inverse the operation gave, which states the digest of the XML it applies to, so Undo refuses once anything
//! else has changed the source. What an operation went without or left to another file, its notes, is said as a warning where
//! `maps apply` prints a note.

use std::path::{Path, PathBuf};

use glam::Vec3;
use or_maps::edit::Pose;
use or_maps::operation::{Applied, At, Operation, Record};
use or_maps::Maps;
use or_project::ModuleKind;

use super::maps::{Pick, PickKind, Step, KEPT};
use crate::state::AppState;

/// Do `operation` to the sources of the pack in `folder`; a refusal is its reason alone, which is what a toast reads.
fn apply(folder: &Path, operation: &Operation) -> Result<Applied, String> {
    or_maps::operation::apply(folder, operation).map_err(|refused| match refused {
        or_maps::operation::Error::Refused { why, .. } => why,
        other => other.to_string(),
    })
}

impl AppState {
    /// The folder of the shown pack, which an operation is applied to.
    fn maps_pack_folder(&self) -> Option<PathBuf> {
        Some(self.maps_pack()?.folder.clone())
    }

    /// The folder of the source `source` of the shown pack.
    fn map_source_dir(&self, source: &str) -> Option<PathBuf> {
        let pack = self.maps_pack()?;
        Some(
            pack.folder
                .join(source.replace('/', std::path::MAIN_SEPARATOR_STR)),
        )
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
        match apply(&folder, &operation) {
            Ok(applied) if applied.edited.changed() => {
                self.patch_scene(source, index, pose);
                self.push_maps_applied(label, &folder, operation, applied);
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

    /// Put the scene the shown set was last read as to the pose written, so the entity does not jump back for the frames before the next scan.
    fn patch_scene(&mut self, source: &str, index: usize, pose: Pose) {
        let set = self.shown_set().map(|(_, set)| set.id.clone());
        let module = self.maps_pack().map(|p| p.module.clone());
        let (Some(set), Some(module)) = (set, module) else {
            return;
        };
        let Some(scan) = self.workbench.scan.as_mut() else {
            return;
        };
        let Some(scene) = (scan.maps.iter_mut())
            .find(|m| m.module == module)
            .and_then(|m| m.read.as_mut().ok())
            .and_then(|pack| pack.scenes.iter_mut().find(|(id, _)| *id == set))
            .map(|(_, scene)| scene)
        else {
            return;
        };
        if let Some(placed) = scene
            .find(source, index)
            .and_then(|at| scene.placed.get_mut(at))
        {
            placed.obb.position = pose.position;
            placed.obb.rotation = pose.rotation;
            placed.obb.scale = Vec3::new(pose.scale_xy, pose.scale_xy, pose.scale_z);
        }
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
        let holds = |source: &str, at: usize| {
            (self.map_source_dir(source))
                .is_some_and(|dir| or_maps::edit::read_entity(&dir, at).is_ok())
        };
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
        let donor = selected
            .filter(|at| holds(&at.source, at.index))
            .or_else(|| Some(first(&target)).filter(|at| holds(&at.source, 0)))
            .or_else(|| sources.iter().map(first).find(|at| holds(&at.source, 0)));
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
        match apply(&folder, &operation) {
            Ok(applied) => {
                self.remember_map_names(&module, &[text.as_deref().unwrap_or_default()]);
                let added = applied.added.map(PickKind::Entity);
                self.push_maps_applied(label, &folder, operation, applied);
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

    /// Take the entity selected out of its map file, with the LOD links renumbered, as one step.
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
        let operation = Operation::RemoveEntity {
            source,
            entity: index,
            extents: None,
            expects: None,
        };
        match apply(&folder, &operation) {
            Ok(applied) => {
                let label = tr!("Removed entity {index}", index = index);
                self.push_maps_applied(label, &folder, operation, applied);
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
        let (Some(folder), Some(dir)) = (self.maps_pack_folder(), self.map_source_dir(&source))
        else {
            return;
        };
        // The record as the person changed it, which the operation puts in the place of the one the file holds.
        let changed = or_maps::edit::read_archetype(&dir, index).and_then(|mut archetype| {
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
            apply(&folder, &operation).map(|applied| (operation, applied))
        });
        match done {
            Ok((operation, applied)) if applied.edited.changed() => {
                self.remember_map_names(&module, typed);
                self.push_maps_applied(label, &folder, operation, applied);
            }
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
        let (Some(folder), Some(dir)) = (self.maps_pack_folder(), self.map_source_dir(&source))
        else {
            return;
        };
        let renamed = or_maps::edit::read_entity(&dir, index).and_then(|mut entity| {
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
            apply(&folder, &operation).map(|applied| (operation, applied))
        });
        match done {
            Ok((operation, applied)) if applied.edited.changed() => {
                self.remember_map_names(&module, &[name]);
                let label = tr!(
                    "Placed {name} as entity {index}",
                    name = name,
                    index = index
                );
                self.push_maps_applied(label, &folder, operation, applied);
            }
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
        let donor = match self.workbench.maps.selected.clone() {
            Some(Pick {
                source,
                kind: PickKind::Archetype(index),
            }) => Some(At { source, index }),
            _ => None,
        }
        .filter(|at| {
            (self.map_source_dir(&at.source))
                .is_some_and(|dir| or_maps::edit::read_archetype(&dir, at.index).is_ok())
        });
        let operation = Operation::AddArchetype {
            source: target.clone(),
            name: name.to_string(),
            asset: asset.map(str::to_string),
            donor,
            expects: None,
        };
        match apply(&folder, &operation) {
            Ok(applied) => {
                self.remember_map_names(&module, &[name, asset.unwrap_or(name)]);
                let added = applied.added.map(PickKind::Archetype);
                let label = tr!("Added the archetype {name}", name = name);
                self.push_maps_applied(label, &folder, operation, applied);
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
        match apply(&folder, &operation) {
            Ok(applied) => {
                let label = tr!("Removed archetype {index}", index = index);
                self.push_maps_applied(label, &folder, operation, applied);
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

    /// Keep `operation`, which was applied to the pack in `folder`, as a step of Undo, and say what it went without.
    fn push_maps_applied(
        &mut self,
        label: String,
        folder: &Path,
        operation: Operation,
        applied: Applied,
    ) {
        self.say_maps_notes(&applied.notes);
        self.push_maps_step(Step {
            label,
            folder: folder.to_path_buf(),
            undo: applied.inverse,
            redo: operation,
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
        self.workbench.want_scan();
    }

    /// Take the change of `step` back (`undo`) or make it again, by the operation the step keeps, which is refused once anything but this history changed its source; the step then keeps the operation that goes the other way.
    fn apply_maps_step(&mut self, step: &mut Step, undo: bool) -> Result<(), String> {
        let operation = if undo { &step.undo } else { &step.redo };
        let applied =
            or_maps::operation::apply(&step.folder, operation).map_err(
                |refused| match refused {
                    or_maps::operation::Error::Stale { at, .. } => {
                        let file = (step.folder)
                            .join(at.replace('/', std::path::MAIN_SEPARATOR_STR))
                            .join(or_source::metadata::XML);
                        tr!(
                            "{file} changed since: take back the changes made after this one first",
                            file = file.display().to_string()
                        )
                        .to_string()
                    }
                    other => other.to_string(),
                },
            )?;
        self.say_maps_notes(&applied.notes);
        if undo {
            step.redo = applied.inverse;
        } else {
            step.undo = applied.inverse;
        }
        self.workbench.want_scan();
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
