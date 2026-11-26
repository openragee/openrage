//! The sessions the Map workspace holds its packs of maps open in, so an edit costs what a screen can afford: an edit, Undo and Redo
//! change the typed roots a session holds and write nothing, and saving the project writes them.
//!
//! ⚠ **The scan reads the disk, and until a save the disk is behind.** What a session holds of each source an operation named is
//! laid over what the scan read of it, after each edit and each time a scan lands, so the lists, the scene and the findings of the
//! Map screen read through the session. A build and the cook read the files, which hold what was last saved, and Problems names
//! each source with an edit a build goes without.
//!
//! ⚠ **A session let go writes nothing.** A `Session` that is dropped writes what it holds, which suits a command and not a project
//! closed with the answer Don't save: [`Sessions`] discards each one it drops.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use or_io::decl::File;
use or_io::meta::typed::{CMapData, CMapTypes};
use or_maps::facts::{entity_fact_of, inside_of, Fact, Inside, SourceFact};
use or_maps::operation::{Done, Error, Operation, Session};
use or_maps::Maps;

use super::findings::{Finding, Place, Severity};
use super::maps::MapsPack;
use super::Workbench;
use crate::state::AppState;

/// A pack of maps held open by the Map workspace.
pub(super) struct Open {
    pub(super) session: Session,
    /// The sources whose facts are read from the session: each one an operation named, until a scan has read what a save wrote of it, with what the session held of it when it was last asked, which stands until an operation names it again.
    touched: BTreeMap<String, Option<Result<Inside, String>>>,
    /// The map sets of the pack whose remembered names the facts kept in `touched` were said with.
    named_by: Option<Maps>,
    /// When the session was last written, which a scan started before has not read.
    flushed: Option<Instant>,
}

/// The sessions of the packs an edit was made to, by the folder of the pack.
#[derive(Default)]
pub(super) struct Sessions(BTreeMap<PathBuf, Open>);

impl Drop for Sessions {
    fn drop(&mut self) {
        for (_, open) in std::mem::take(&mut self.0) {
            open.session.discard();
        }
    }
}

impl Sessions {
    /// The session of the pack in `folder`, opened when it is first asked for, with the map sets of the pack as its file states them now.
    pub(super) fn open(&mut self, folder: &Path) -> Result<&mut Open, String> {
        let open = match self.0.entry(folder.to_path_buf()) {
            Entry::Occupied(held) => held.into_mut(),
            Entry::Vacant(free) => free.insert(Open {
                session: Session::open(folder).map_err(|refused| refused.to_string())?,
                touched: BTreeMap::new(),
                named_by: None,
                flushed: None,
            }),
        };
        // ⚠ An import, or History going back over one, rewrites the sets while the session is open: a source added since would be refused.
        (open.session.reread_model()).map_err(|refused| refused.to_string())?;
        Ok(open)
    }
}

/// What `session` holds of the file `source` cooks to, as a scan says what a file on disk states.
fn held_inside(
    session: &mut Session,
    source: &str,
    extension: &str,
    names: &dyn Fn(u32) -> Option<String>,
) -> Result<Inside, String> {
    let said = |refused: Error| match refused {
        Error::Source { why, .. } => why,
        other => other.to_string(),
    };
    let saved = if extension == CMapTypes::EXTENSION {
        session.types(source).map_err(said)?.to_bytes()
    } else {
        session.map(source).map_err(said)?.to_bytes()
    };
    let bytes = saved.map_err(|refused| refused.to_string())?;
    inside_of(&bytes, extension, Some(names))
}

impl Open {
    /// Say the entity `index` of the map file `source` again in the facts kept of that file, with the extents the file states, as the session now holds them; false where they cannot be said in place, and the file is then read whole.
    ///
    /// ⚠ Only for an entity that is the same thing somewhere else. Saving a large map file and reading it again is most of what an
    /// edit costs the frame, and a move changes one pose and four corners of it; another archetype, another link or another count
    /// changes what the file places and what is wrong with it, which only the whole read says.
    fn say_the_entity_again(&mut self, source: &str, index: usize) -> bool {
        let Some(Some(Ok(Inside::Map(kept)))) = self.touched.get_mut(source) else {
            return false;
        };
        let (Some(model), Ok(map)) = (self.named_by.as_ref(), self.session.map(source)) else {
            return false;
        };
        let (Some(entity), Some(fact)) = (map.entities.get(index), kept.entities.get_mut(index))
        else {
            return false;
        };
        let names = |hash: u32| model.name_of(hash);
        let Ok(said) = entity_fact_of(entity, Some(&names)) else {
            return false;
        };
        let what = |fact: &or_maps::facts::EntityFact| {
            let links = (fact.flags, fact.parent_index, fact.num_children);
            (fact.archetype.hash, fact.guid, links, fact.interior)
        };
        // A file that states no extents is not given some here.
        let (Some(entities), Some(streaming)) =
            (&mut kept.stored_entities, &mut kept.stored_streaming)
        else {
            return false;
        };
        if what(&said) != what(fact) {
            return false;
        }
        *fact = said;
        let corner = |at: or_io::decl::Vec3| [at.x, at.y, at.z];
        *entities = (
            corner(map.entities_extents_min),
            corner(map.entities_extents_max),
        );
        *streaming = (
            corner(map.streaming_extents_min),
            corner(map.streaming_extents_max),
        );
        true
    }

    /// Lay what the session holds of each source it touched over what the scan read of the disk in `pack`, with the scenes and the findings that follow from it.
    fn show_in(&mut self, pack: &mut MapsPack) {
        // A hash is shown as the name the pack remembers for it, and the pack may have learnt one since.
        if self.named_by.as_ref() != Some(&pack.model) {
            self.touched.values_mut().for_each(|kept| *kept = None);
            self.named_by = Some(pack.model.clone());
        }
        let mut shown: Vec<&str> = Vec::new();
        for (source, kept) in &mut self.touched {
            // What the disk no longer holds as a map file or a type file stays as the scan said it.
            let Some(read) = pack.facts.source(source).filter(|read| {
                [CMapData::EXTENSION, CMapTypes::EXTENSION].contains(&read.extension.as_str())
            }) else {
                continue;
            };
            let names = |hash: u32| pack.model.name_of(hash);
            let inside = kept.get_or_insert_with(|| {
                held_inside(&mut self.session, source, &read.extension, &names)
            });
            let fact = SourceFact {
                kind: read.kind.clone(),
                name: read.name.clone(),
                extension: read.extension.clone(),
                bytes: read.bytes,
                game: read.game.clone(),
                digest: read.digest.clone(),
                inside: Some(inside.clone()),
            };
            pack.facts.insert(source, Fact::Source(Box::new(fact)));
            shown.push(source);
        }
        if shown.is_empty() {
            return;
        }
        for set in &pack.model.maps {
            let touched = set.sources.iter().any(|s| shown.contains(&s.as_str()));
            let scene = pack.scenes.iter_mut().find(|(id, _)| *id == set.id);
            if let (true, Some((_, scene))) = (touched, scene) {
                *scene = or_maps::scene(set, &pack.facts);
            }
        }
        pack.findings = or_maps::check(&pack.model, &pack.facts, None);
    }
}

impl Workbench {
    /// Show what the sessions hold in the place of what the last scan read of the disk.
    pub(super) fn show_what_the_map_sessions_hold(&mut self) {
        let Some(scan) = self.scan.as_mut() else {
            return;
        };
        for read in &mut scan.maps {
            let open = self.maps.sessions.0.get_mut(&read.folder);
            if let (Some(open), Ok(pack)) = (open, read.read.as_mut()) {
                open.show_in(pack);
            }
        }
        self.findings_stale = true;
    }

    /// A warning for each source a session holds an edit of that no save has written: a build reads the files, and goes without it.
    pub(super) fn unsaved_map_findings(&self) -> Vec<Finding> {
        let mut found = Vec::new();
        for read in self.maps_read() {
            let Some(open) = self.maps.sessions.0.get(&read.folder) else {
                continue;
            };
            let unsaved =
                (open.touched.keys()).filter(|source| open.session.holds_unwritten(source));
            found.extend(unsaved.map(|source| Finding {
                severity: Severity::Warning,
                place: Place::Content(read.module.clone()),
                text: tr!(
                    "{name} has unsaved changes, which a build does not include",
                    name = source
                ),
                fixes: Vec::new(),
                code: None,
            }));
        }
        found
    }
}

/// The XML file of the source `source` of the pack in `folder`.
fn xml_of(folder: &Path, source: &str) -> PathBuf {
    folder
        .join(source.replace('/', std::path::MAIN_SEPARATOR_STR))
        .join(or_source::metadata::XML)
}

/// The sentence of an operation the session of the pack in `folder` refused; `of_a_step` words it as Undo and Redo do, which name the file an inverse no longer applies to.
fn said(folder: &Path, refused: Error, of_a_step: bool) -> String {
    match refused {
        Error::InList { refused, .. } => said(folder, *refused, of_a_step),
        Error::Stale { at, .. } if of_a_step => tr!(
            "{file} changed since: take back the changes made after this one first",
            file = xml_of(folder, &at).display().to_string()
        )
        .to_string(),
        Error::Refused { why, .. } if !of_a_step => why,
        other => other.to_string(),
    }
}

impl AppState {
    /// Whether an edit to the project's maps is held that no save has written.
    pub fn map_edits_unwritten(&self) -> bool {
        let sessions = &self.workbench.maps.sessions.0;
        sessions.values().any(|open| open.session.unwritten())
    }

    /// Do `operations` to the session of the pack in `folder`, all of them or none, and show what it then holds; nothing is written, and a refusal is its sentence and leaves the session and the screen as they were.
    pub(super) fn do_in_the_map_session(
        &mut self,
        folder: &Path,
        operations: &[Operation],
        of_a_step: bool,
    ) -> Result<Vec<Done>, String> {
        let open = self.workbench.maps.sessions.open(folder)?;
        let done = (open.session.apply_all(operations))
            .map_err(|refused| said(folder, refused, of_a_step))?;
        // One entity placed or put back is said again in place; anything else, and any list, has its sources read whole.
        let said_in_place = match operations {
            [Operation::PlaceEntity { source, entity, .. }]
            | [Operation::ReplaceEntity { source, entity, .. }] => {
                open.say_the_entity_again(source, *entity)
            }
            _ => false,
        };
        if !said_in_place {
            let named = operations.iter().map(|o| (o.source().to_string(), None));
            open.touched.extend(named);
        }
        self.workbench.show_what_the_map_sessions_hold();
        self.take_back_what_does_not_save(folder, operations, &done)?;
        Ok(done)
    }

    /// Take `done` back where a source `operations` named no longer saves as the file the screen reads, and say why: the session held the edit to be sound without saving it, and the save of the project would refuse it.
    fn take_back_what_does_not_save(
        &mut self,
        folder: &Path,
        operations: &[Operation],
        done: &[Done],
    ) -> Result<(), String> {
        let Some(open) = self.workbench.maps.sessions.0.get_mut(folder) else {
            return Ok(());
        };
        let unsaved = (operations.iter()).find_map(|o| match open.touched.get(o.source()) {
            Some(Some(Err(why))) => Some(why.clone()),
            _ => None,
        });
        let Some(why) = unsaved else {
            return Ok(());
        };
        let back: Vec<Operation> = (done.iter().rev().map(|did| did.inverse.clone())).collect();
        if let Err(left) = open.session.apply_all(&back) {
            tracing::error!(folder = %folder.display(), error = %left, "an edit of a map that does not save was not taken back");
        }
        let named = operations.iter().map(|o| (o.source().to_string(), None));
        open.touched.extend(named);
        self.workbench.show_what_the_map_sessions_hold();
        Err(why)
    }

    /// Write what the sessions hold of the project's maps, each pack whole or not at all; a refusal is its sentence.
    pub(in crate::state) fn flush_map_sessions(&mut self) -> Result<(), String> {
        let mut wrote = false;
        for open in self.workbench.maps.sessions.0.values_mut() {
            if !open.session.unwritten() {
                continue;
            }
            let written = (open.session.flush()).map_err(|refused| refused.to_string())?;
            open.flushed = Some(Instant::now());
            wrote |= !written.is_empty();
        }
        if wrote {
            self.workbench.maps.generation += 1;
            self.workbench.want_scan();
        }
        self.workbench.findings_stale = true;
        Ok(())
    }

    /// A scan read the disk: a source that changed under a session is read again where the session holds nothing unwritten of it, and asked about where it does, as a file that changed under a tab with unsaved edits is.
    pub(super) fn follow_the_map_sources_on_disk(&mut self) {
        let scanned_at = self.workbench.last_scan;
        let mut asked: Vec<PathBuf> = Vec::new();
        let mut read_again: Vec<(PathBuf, String)> = Vec::new();
        for (folder, open) in &mut self.workbench.maps.sessions.0 {
            for source in open.session.changed_on_disk() {
                if open.session.holds_unwritten(&source) {
                    asked.push(xml_of(folder, &source));
                } else {
                    open.session.read_again(&source);
                    open.touched.remove(&source);
                    read_again.push((folder.clone(), source));
                }
            }
            // ⚠ A scan that started before the last save read the files as they were before it.
            let read_the_save =
                (open.flushed).is_none_or(|at| scanned_at.is_some_and(|scan| scan >= at));
            if read_the_save {
                let session = &open.session;
                (open.touched).retain(|source, _| session.holds_unwritten(source));
            }
        }
        for path in asked {
            if !self.reload_conflicts.contains(&path) {
                self.reload_conflicts.push(path);
            }
        }
        for (folder, source) in read_again {
            self.forget_the_map_steps_of(&folder, &source);
        }
        self.workbench.show_what_the_map_sessions_hold();
    }

    /// The answer to the changed-on-disk question for `path`, when it is the XML of a map source a session holds: read it again and lose what was not saved, or keep that over the file at the next save; whether it was one.
    pub(in crate::state) fn answer_for_a_map_source(&mut self, path: &Path, reload: bool) -> bool {
        let held = (self.workbench.maps.sessions.0.iter_mut()).find_map(|(folder, open)| {
            let inside = path.strip_prefix(folder).ok()?.parent()?;
            let source: Vec<_> = (inside.components())
                .map(|part| part.as_os_str().to_string_lossy())
                .collect();
            let source = source.join("/");
            (xml_of(folder, &source) == path).then(|| (folder.clone(), open, source))
        });
        let Some((folder, open, source)) = held else {
            return false;
        };
        if reload {
            open.session.read_again(&source);
            open.touched.remove(&source);
            self.forget_the_map_steps_of(&folder, &source);
            self.workbench.maps.generation += 1;
            self.workbench.maps.preview = None;
            self.workbench.findings_stale = true;
            self.workbench.want_scan();
        } else {
            open.session.keep_over_the_disk(&source);
        }
        true
    }

    /// The map file `source` of the shown pack cooks to, as the session of the pack holds it.
    #[cfg(test)]
    pub(crate) fn held_map_file(&mut self, source: &str) -> Result<CMapData, String> {
        let folder = self.maps_pack().ok_or("no pack is shown")?.folder.clone();
        let open = self.workbench.maps.sessions.open(&folder)?;
        (open.session.map(source).cloned()).map_err(|refused| refused.to_string())
    }

    /// The type file `source` of the shown pack cooks to, as the session of the pack holds it.
    #[cfg(test)]
    pub(crate) fn held_type_file(&mut self, source: &str) -> Result<CMapTypes, String> {
        let folder = self.maps_pack().ok_or("no pack is shown")?.folder.clone();
        let open = self.workbench.maps.sessions.open(&folder)?;
        (open.session.types(source).cloned()).map_err(|refused| refused.to_string())
    }

    /// Drop the steps of Undo and Redo of the pack in `folder` once one of them names `source`, which was read again: a step keeps an inverse of the session's own states, and one that is refused would stand before the others.
    fn forget_the_map_steps_of(&mut self, folder: &Path, source: &str) {
        let work = &mut self.workbench.maps;
        let names = |step: &super::maps::Step| {
            step.folder == folder
                && (step.undo.iter().chain(&step.redo)).any(|o| o.source() == source)
        };
        if work.undo.iter().chain(&work.redo).any(names) {
            work.undo.retain(|step| step.folder != folder);
            work.redo.retain(|step| step.folder != folder);
        }
    }
}
