//! A scanned GTA VI map's entity added or taken out on the Map screen: a job reads the bodies the
//! archive's whole scan lists, finds the maps that name the map as a parent and its own parent map,
//! and makes the edit across them; the map and each body it renumbers take it as one step of their
//! own documents, which Save As writes as loose resources.
//!
//! ⚠ Each body an edit renumbers is a document with a step of its own, so Undo takes the edit back
//! one document at a time. A step holds both trees and both sections of its body.

use crate::state::Rpf9Document;

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use glam::{Quat, Vec3};
use or_io::container::rpf::{
    read_rpf9_body, Rpf9BodyKind, Rpf9ScanFind, Rpf9Sections, Rpf9ZlibRun,
};
use or_io::gta6::map::grow::{
    add_entity_across, entities_union, lod_parent, remove_entity_across, set_entities_extents,
    MapGrowError,
};
use or_io::gta6::map::links::{self, Links};
use or_io::gta6::map::{self, Aabb, Entity, MapBody};
use or_io::meta::binary::MetaLayout;
use or_io::meta::MetaInstance;
use or_job::{Cancel, Progress};

use crate::state::document::MetaDocument;
use crate::state::edit_history::{Change, Swap};
use crate::state::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use crate::state::rpf9_scan::{body_path, scanned_body, Ended as ScanEnded, Row, View};
use crate::state::workbench::{Pick, PickKind};
use crate::state::{AppState, DocPayload, DocumentId};

/// Archetype boxes in model space, by the word an entity names its archetype by.
type Boxes = HashMap<u64, ([f32; 3], [f32; 3])>;

/// The edit a job makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Edit {
    /// A copy of entity `from`, an orphan where `orphan`.
    Add {
        from: usize,
        orphan: bool,
    },
    Remove(usize),
}

/// A body as an edit leaves it: its sections, and the tree and layout they read to.
pub(super) struct Side {
    pub sections: Rpf9Sections,
    pub tree: MetaInstance,
    pub layout: MetaLayout,
}

pub(super) fn side(sections: Rpf9Sections) -> Result<Side, String> {
    let (tree, layout) =
        or_io::meta::binary::rbm::gta6::open(&sections.virtual_section, &sections.physical)
            .map_err(|e| e.to_string())?;
    Ok(Side {
        sections,
        tree,
        layout,
    })
}

/// What an edit makes of the map and the bodies it renumbers.
pub(super) struct Made {
    /// The map's sections the edit began from, and the map after it.
    pub map: (Rpf9Sections, Side),
    /// Each other body it changed: its offset, its sections before, and the body after.
    pub others: Vec<(u64, Rpf9Sections, Side)>,
    /// The index of an entity added.
    pub at: Option<usize>,
}

/// `edit` made on `map` across `related`, each with its offset; a removal's `entitiesExtents` are
/// the union of the boxes left where `boxes` holds every one, else kept.
pub(super) fn make(
    map: Rpf9Sections,
    related: Vec<(u64, Rpf9Sections)>,
    edit: Edit,
    boxes: &Boxes,
) -> Result<Made, String> {
    let (offsets, before): (Vec<u64>, Vec<Rpf9Sections>) = related.into_iter().unzip();
    let mut others = before.clone();
    let mut edited = map.clone();
    let grown = |e: MapGrowError| e.to_string();
    let (at, changed) = match edit {
        Edit::Add { from, orphan } => {
            let (at, changed) =
                add_entity_across(&mut edited, &mut others, from, orphan).map_err(grown)?;
            (Some(at), changed)
        }
        Edit::Remove(index) => {
            let changed = remove_entity_across(&mut edited, &mut others, index).map_err(grown)?;
            let left = match map::open(&edited.virtual_section, &edited.physical) {
                Ok(MapBody::Data(m)) => m,
                Ok(MapBody::Types(_)) => return Err(MapGrowError::NotMapData.to_string()),
                Err(e) => return Err(e.to_string()),
            };
            if let Some(union) = entities_union(&left, |e| placed(boxes, e)) {
                set_entities_extents(&mut edited, union).map_err(grown)?;
            }
            (None, changed)
        }
    };
    let mut out = Vec::with_capacity(changed.len());
    for k in changed {
        if let (Some(offset), Some(was), Some(now)) = (offsets.get(k), before.get(k), others.get(k))
        {
            out.push((*offset, was.clone(), side(now.clone())?));
        }
    }
    Ok(Made {
        map: (map, side(edited)?),
        others: out,
        at,
    })
}

/// `e`'s archetype box of `boxes` scaled, turned and placed, as a map's extents hold it.
fn placed(boxes: &Boxes, e: &Entity) -> Option<Aabb> {
    let (lo, hi) = boxes.get(&e.archetype_word)?;
    let scale = or_world::transform::scale_of(e.scale_xy, e.scale_z);
    let turn = Quat::from_array(e.model_rotation()).normalize();
    let (lo, hi) = (Vec3::from(*lo) * scale, Vec3::from(*hi) * scale);
    let corners = or_world::transform::placed_corners(lo, hi, turn, Vec3::from(e.position));
    let (min, max) = (corners.iter()).fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(a, b), c| {
        (a.min(*c), b.max(*c))
    });
    Some(Aabb {
        min: min.to_array(),
        max: max.to_array(),
    })
}

/// Whether `find` starts a headerless resource with no signature, as a map body does.
fn starts(find: &Rpf9ScanFind) -> bool {
    let unsigned = matches!(
        find.kind,
        Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(_),
            signature: None,
            ..
        })
    );
    unsigned && find.stored_len.is_some()
}

/// The map bodies at `offsets` of `archive` that name the map `name` as a parent or are the map
/// `parent`, each read from `open` where a document holds it; the boxes of each map types body
/// among them go to `boxes`. `None` once `cancel` is set; refused where a body does not read, as
/// it may be one of them.
fn related(
    archive: &Path,
    offsets: &[u64],
    open: &HashMap<u64, Rpf9Sections>,
    (name, parent): (u32, Option<u32>),
    boxes: &mut Boxes,
    (cancel, progress): (&Cancel, &Progress),
) -> Result<Option<Vec<(u64, Rpf9Sections)>>, String> {
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mut found = Vec::new();
    for (done, at) in offsets.iter().enumerate() {
        if cancel.is_set() {
            return Ok(None);
        }
        progress.reached(done as u64, offsets.len() as u64);
        let unknown = |why: String| {
            tr!(
                "The body at {offset} does not read, so whether it names the map is not known: {why}",
                offset = format!("{at:#X}"),
                why = why
            )
        };
        let s = match open.get(at) {
            Some(s) => s.clone(),
            None => {
                let (find, bytes) = read_rpf9_body(&file, *at, super::BODY_LIMIT)
                    .map_err(|e| unknown(e.to_string()))?;
                match Rpf9Sections::of(&find, &bytes) {
                    Some(s) => s,
                    None => continue,
                }
            }
        };
        let root = map::root_type(&s.virtual_section);
        if root == Some(map::C_MAP_TYPES) {
            if let Ok(MapBody::Types(t)) = map::open(&s.virtual_section, &s.physical) {
                boxes.extend((t.archetypes.iter()).map(|a| (a.name_word, (a.bb_min, a.bb_max))));
            }
            continue;
        }
        if root != Some(map::C_MAP_DATA) {
            continue;
        }
        let named = match links::open(&s.virtual_section, &s.physical) {
            Ok((MapBody::Data(m), Ok(Links::Data(l)))) => {
                Some(m.name) == parent || l.lod_parents.iter().any(|p| p.name == name)
            }
            Ok((_, Err(e))) | Err(e) => return Err(unknown(e.to_string())),
            Ok(_) => false,
        };
        if named {
            found.push((*at, s));
        }
    }
    Ok(Some(found))
}

/// A body's document as an edit leaves it, holding the other side: its tree, its layout and the
/// sections of each side, so a swap reads the side it puts in as the Map screen's map again.
pub(super) struct MapPart {
    path: PathBuf,
    tree: MetaInstance,
    layout: Option<MetaLayout>,
    /// The sections of the side this part holds.
    other: Rpf9Sections,
    /// The sections of the side the document holds.
    here: Rpf9Sections,
}

impl Swap for MapPart {
    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let DocPayload::Meta(meta) = payload else {
            return false;
        };
        let Some(parsed) = meta.parsed.as_mut() else {
            return false;
        };
        std::mem::swap(&mut parsed.instance, &mut self.tree);
        std::mem::swap(&mut meta.layout, &mut self.layout);
        std::mem::swap(&mut self.other, &mut self.here);
        meta.after_visual_edit_in_place();
        super::keep(&self.path, &self.here.virtual_section, &self.here.physical);
        true
    }
}

type Ended = Result<Option<Made>, String>;

struct Job {
    map: PathBuf,
    /// The revision of each document the job read from, which must hold when it ends.
    revisions: Vec<(PathBuf, u64)>,
    edit: Edit,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Ended>>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

// ponytail: one edit for the whole app; another started in its place drops it, and says nothing of it.
static JOB: Mutex<Option<Job>> = Mutex::new(None);

fn running_job() -> MutexGuard<'static, Option<Job>> {
    JOB.lock().unwrap_or_else(|p| p.into_inner())
}

fn has_ended(job: &Job) -> bool {
    job.ended
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_some()
}

/// The edit while it reads, as the job list draws it.
// ponytail: listed as an archetype search, whose Stop stops it, as the world's fill is.
pub(super) fn job() -> Option<RunningJob> {
    let held = running_job();
    let job = held.as_ref().filter(|job| !has_ended(job))?;
    Some(RunningJob {
        kind: JobKind::ArchetypeSearch,
        doing: tr!(
            "Looking for the maps linked to {map}",
            map = file_name(&job.map)
        ),
        fraction: Some(job.progress.snapshot().fraction()),
        detail: String::new(),
        stopping: job.cancel.is_set(),
    })
}

pub(super) fn stop() {
    if let Some(job) = running_job().as_ref() {
        job.cancel.set();
    }
}

impl AppState {
    /// Add a copy of the entity selected on the scanned map shown, else of its first, as a job:
    /// an entity of a level of detail above HD is copied as an orphan.
    pub(crate) fn add_scanned_entity(&mut self) {
        let entities = self
            .scanned_map()
            .and_then(|s| s.data())
            .map(|m| &m.entities);
        let from = match self.workbench.maps.selected.as_ref().map(|p| p.kind) {
            Some(PickKind::Entity(i)) if entities.is_some_and(|e| i < e.len()) => i,
            _ => 0,
        };
        let lod_level = entities.and_then(|e| e.get(from)).map(|e| e.lod_level);
        // `LODTYPES_DEPTH_HD` and `LODTYPES_DEPTH_ORPHANHD` keep their link.
        let orphan = lod_level.is_some_and(|l| !matches!(l, 0 | 5));
        self.start_entity_edit(Edit::Add { from, orphan });
    }

    /// Take the entity selected out of the scanned map shown, as a job.
    pub(crate) fn remove_scanned_entity(&mut self) {
        if let Some(PickKind::Entity(index)) = self.workbench.maps.selected.as_ref().map(|p| p.kind)
        {
            self.start_entity_edit(Edit::Remove(index));
        }
    }

    fn start_entity_edit(&mut self, edit: Edit) {
        if let Err(why) = self.try_start_entity_edit(edit) {
            self.show_error_toast(why);
        }
    }

    /// The document holding the metadata tree of the body at `path`.
    pub(super) fn meta_document_at(&self, path: &Path) -> Option<DocumentId> {
        (self.documents.iter())
            .find(|d| d.path.as_deref() == Some(path) && matches!(d.payload, DocPayload::Meta(_)))
            .map(|d| d.id)
    }

    /// The offsets of the bodies the finished whole scan of `archive` lists that may be map bodies.
    fn whole_scan_offsets(&self, archive: &Path) -> Option<Vec<u64>> {
        self.documents.iter().find_map(|d| match &d.payload {
            DocPayload::Rpf9(Rpf9Document::Scan(scan))
                if scan.path.as_path() == archive && scan.start == 0 =>
            {
                let found = scan.found();
                let whole = matches!(found.ended, Some(ScanEnded::Whole));
                let offsets: Vec<u64> = (found.rows.iter())
                    .filter_map(|row| match row {
                        Row::Body(find) if starts(find) => Some(find.offset),
                        _ => None,
                    })
                    .collect();
                whole.then_some(offsets)
            }
            _ => None,
        })
    }

    fn try_start_entity_edit(&mut self, edit: Edit) -> Result<(), String> {
        let path = (self.scanned_map().map(|s| s.path.clone()))
            .ok_or_else(|| tr!("No scanned GTA VI map is shown.").to_string())?;
        let (archive, offset) = scanned_body(&path)
            .map(|(a, o)| (a.to_path_buf(), o))
            .ok_or_else(|| tr!("The scanned map's document is closed.").to_string())?;
        let doc = (self.meta_document_at(&path))
            .ok_or_else(|| tr!("The scanned map's document is closed.").to_string())?;
        let not_read = || tr!("The scanned map's metadata did not read.").to_string();
        let map_sections = match self.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Meta(meta)) => meta.headerless_sections().ok_or_else(not_read)??,
            _ => return Err(not_read()),
        };
        let name = match map::open(&map_sections.virtual_section, &map_sections.physical) {
            Ok(MapBody::Data(m)) => m.name,
            Ok(MapBody::Types(_)) => return Err(MapGrowError::NotMapData.to_string()),
            Err(e) => return Err(e.to_string()),
        };
        let index = match edit {
            Edit::Add { from, .. } => from,
            Edit::Remove(index) => index,
        };
        let parent =
            (lod_parent(&map_sections, index).map_err(|e| e.to_string())?).map(|(name, _)| name);
        // A loose map file holds no other body.
        let offsets = match archive == path {
            true => Vec::new(),
            false => (self.whole_scan_offsets(&archive)).ok_or_else(|| {
                tr!(
                    "Open the whole scan of {archive}: the maps that name this one as a parent, and its own parent map, are found among the bodies it lists.",
                    archive = archive.display()
                )
            })?,
        };
        let offsets: Vec<u64> = offsets.into_iter().filter(|at| *at != offset).collect();
        // The maps open as documents are read as their trees now hold them.
        let mut revisions = vec![(path.clone(), self.document(doc).map_or(0, meta_revision))];
        let mut open = HashMap::new();
        for d in &self.documents {
            let Some(p) = d.path.as_deref().filter(|p| *p != path.as_path()) else {
                continue;
            };
            if !matches!(super::opened_map(p), Some(Ok(_))) {
                continue;
            }
            let Some((a, at)) = scanned_body(p) else {
                continue;
            };
            if a != archive.as_path() {
                continue;
            }
            if let DocPayload::Meta(meta) = &d.payload {
                if let Some(Ok(sections)) = meta.headerless_sections() {
                    revisions.push((p.to_path_buf(), meta.revision()));
                    open.insert(at, sections);
                }
            }
        }
        let mut boxes = Boxes::new();
        self.each_known_archetype(|a| {
            boxes.insert(a.name_word, (a.bb_min, a.bb_max));
        });
        let (cancel, progress) = (Cancel::new(), Progress::new());
        let ended: Arc<Mutex<Option<Ended>>> = Arc::default();
        let job = Job {
            map: path,
            revisions,
            edit,
            cancel: cancel.clone(),
            progress: progress.clone(),
            ended: Arc::clone(&ended),
        };
        std::thread::spawn(move || {
            let outcome = (|| -> Ended {
                let mut boxes = boxes;
                let keys = (name, parent);
                let found = related(
                    &archive,
                    &offsets,
                    &open,
                    keys,
                    &mut boxes,
                    (&cancel, &progress),
                )?;
                let Some(found) = found else {
                    return Ok(None);
                };
                make(map_sections, found, edit, &boxes).map(Some)
            })();
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
        });
        *running_job() = Some(job);
        Ok(())
    }

    /// Say in the job list how the edit that ended went, and make it where it was made.
    pub(super) fn poll_entity_edit(&mut self) {
        let job = {
            let mut held = running_job();
            if !held.as_ref().is_some_and(has_ended) {
                return;
            }
            held.take()
        };
        let Some(job) = job else {
            return;
        };
        let ended = job.ended.lock().unwrap_or_else(|p| p.into_inner()).take();
        let (outcome, said) = match ended {
            Some(Ok(Some(made))) => match self.make_entity_edit(&job, made) {
                Ok(said) => (Outcome::Done(Then::Nothing), said),
                Err(why) => (Outcome::Failed, why),
            },
            Some(Ok(None)) | None => (
                Outcome::Stopped(Then::Nothing),
                tr!("Stopped before it finished").to_string(),
            ),
            Some(Err(why)) => (
                Outcome::Failed,
                tr!("The entity was not added or removed: {why}", why = why),
            ),
        };
        if outcome == Outcome::Failed {
            self.show_error_toast(said.clone());
        }
        self.jobs.record(JobKind::ArchetypeSearch, outcome, said);
    }

    /// `made` taken by the map's document and each body's it renumbers, one step each.
    fn make_entity_edit(&mut self, job: &Job, made: Made) -> Result<String, String> {
        for (path, revision) in &job.revisions {
            let now = (self.meta_document_at(path))
                .and_then(|doc| self.document(doc))
                .map(meta_revision);
            if now != Some(*revision) {
                return Err(tr!("A map changed or closed while the maps linked to it were read: make the edit again.").to_string());
            }
        }
        let closed = || tr!("The scanned map's document is closed.").to_string();
        let archive = (scanned_body(&job.map).map(|(a, _)| a.to_path_buf())).ok_or_else(closed)?;
        let doc = self.meta_document_at(&job.map).ok_or_else(closed)?;
        let label = match job.edit {
            Edit::Add { from, .. } => {
                let shown = (self.scanned_map().and_then(|s| s.data()))
                    .and_then(|m| m.entities.get(from))
                    .map(|e| or_maps::Name::of(e.archetype_name).shown())
                    .unwrap_or_default();
                tr!("Added an entity of {name}", name = shown)
            }
            Edit::Remove(index) => tr!("Removed entity {index}", index = index),
        };
        let mut renumbered = Vec::new();
        for (offset, before, after) in made.others {
            let path = body_path(&archive, offset, View::Headerless);
            let other = match self.meta_document_at(&path) {
                Some(other) => other,
                None => self.open_linked_map(&path, &before)?,
            };
            self.make_map_step(other, &path, before, after, label.clone())?;
            renumbered.push(format!("{offset:#X}"));
        }
        let (before, after) = made.map;
        self.make_map_step(doc, &job.map, before, after, label.clone())?;
        self.workbench.maps.selected = None;
        self.refresh_scanned_map();
        if let Some(at) = made.at {
            let pick = Pick {
                source: job.map.display().to_string(),
                kind: PickKind::Entity(at),
            };
            self.pick_shown(None, None, Some(pick));
        }
        Ok(match renumbered.is_empty() {
            true => label,
            false => tr!(
                "{edit}. The maps it renumbers are open as documents of their own, each with this edit as one step: {maps}",
                edit = label,
                maps = renumbered.join(", ")
            ),
        })
    }

    /// The body at `path`, which `before` holds, opened as a document behind the one shown.
    fn open_linked_map(
        &mut self,
        path: &Path,
        before: &Rpf9Sections,
    ) -> Result<DocumentId, String> {
        let meta = MetaDocument::from_gta_vi_headerless(&before.virtual_section, &before.physical)?;
        super::keep(path, &before.virtual_section, &before.physical);
        let (active, workspace) = (self.active_document, self.active_workspace);
        let doc = self.insert_document(path.to_path_buf(), DocPayload::Meta(meta), None, None);
        (self.active_document, self.active_workspace) = (active, workspace);
        Ok(doc)
    }

    /// Document `doc` of the body at `path`, whose sections are `before`, given `after` as one step.
    pub(super) fn make_map_step(
        &mut self,
        doc: DocumentId,
        path: &Path,
        before: Rpf9Sections,
        after: Side,
        label: String,
    ) -> Result<(), String> {
        let mut part = MapPart {
            path: path.to_path_buf(),
            tree: after.tree,
            layout: Some(after.layout),
            other: after.sections,
            here: before,
        };
        let not_read = || tr!("The scanned map's metadata did not read.").to_string();
        let document = (self.document_mut(doc))
            .ok_or_else(|| tr!("The scanned map's document is closed.").to_string())?;
        if !part.swap(&mut document.payload) {
            return Err(not_read());
        }
        let part: Box<dyn Swap> = Box::new(part);
        let change = Change::Part {
            key: None,
            parts: vec![part],
        };
        self.record(doc, label, change);
        // Never folded into the next edit: entity numbers change under it.
        if let Some(document) = self.document_mut(doc) {
            document.history.seal();
        }
        Ok(())
    }
}

/// The revision of a metadata document's tree; 0 for any other document.
fn meta_revision(d: &crate::state::Document) -> u64 {
    match &d.payload {
        DocPayload::Meta(meta) => meta.revision(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use or_io::container::rpf::Rpf9Form;
    use or_io::meta::Value;

    use super::*;

    fn sections(byte: u8) -> Rpf9Sections {
        Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: vec![byte; 4],
            physical: Vec::new(),
        }
    }

    fn tree(value: u64) -> MetaInstance {
        let mut t = MetaInstance::new("CMapData");
        t.set("flags", Value::UInt(value));
        t
    }

    #[test]
    fn a_map_part_puts_each_side_s_tree_and_sections_in_and_bumps_the_revision() {
        let mut meta = MetaDocument::empty();
        meta.parsed.as_mut().expect("a tree").instance = tree(1);
        let mut payload = DocPayload::Meta(meta);
        let revision = |p: &DocPayload| match p {
            DocPayload::Meta(m) => (m.revision(), m.parsed.as_ref().map(|t| t.instance.clone())),
            _ => panic!("a metadata document"),
        };
        let mut part = MapPart {
            path: PathBuf::from("a_map_part_puts_each_side_in/0x10.rbm"),
            tree: tree(2),
            layout: None,
            other: sections(2),
            here: sections(1),
        };
        assert!(part.swap(&mut payload));
        assert_eq!(revision(&payload), (1, Some(tree(2))));
        assert_eq!(
            (part.tree.clone(), part.here.clone()),
            (tree(1), sections(2))
        );
        assert!(part.swap(&mut payload));
        assert_eq!(revision(&payload), (2, Some(tree(1))));
        assert_eq!(part.here, sections(1), "the sections of the side put back");

        let mut other = DocPayload::Text(crate::state::TextDocument::empty());
        assert!(!part.swap(&mut other), "no tree to swap with");
        assert_eq!(part.tree, tree(2), "and nothing changed");
    }

    #[test]
    fn an_entity_s_box_is_its_archetype_s_scaled_turned_and_placed_or_none_when_not_held() {
        let mut e = Entity {
            interior: false,
            id: 1,
            // A quarter turn about z, stored as its conjugate.
            rotation: [
                0.0,
                0.0,
                -std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
            archetype_word: 7,
            position: [10.0, 0.0, 0.0],
            archetype_name: 3,
            flags: 0,
            scale_xy: 2.0,
            scale_z: 1.0,
            parent_index: -1,
            hash_e74c3c23: -1,
            lod_dist: -1.0,
            child_lod_dist: 0.0,
            num_children: 0,
            tint_value: 0,
            priority_level: 0,
            lod_level: 5,
        };
        let boxes = Boxes::from([(7, ([-1.0, -2.0, 0.0], [1.0, 2.0, 3.0]))]);
        let b = placed(&boxes, &e).expect("held");
        let near = |a: [f32; 3], w: [f32; 3]| (0..3).all(|i| (a[i] - w[i]).abs() < 1e-4);
        assert!(near(b.min, [6.0, -2.0, 0.0]), "{b:?}");
        assert!(near(b.max, [14.0, 2.0, 3.0]), "{b:?}");
        e.archetype_word = 8;
        assert_eq!(placed(&boxes, &e), None);
    }

    #[test]
    fn an_edit_of_sections_that_are_no_map_is_refused_before_any_body_is_written() {
        // The refusal is the map's own read's, made before any related body is looked at.
        let s = sections(0);
        let why = match map::open(&s.virtual_section, &s.physical) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("four zero bytes read as a map"),
        };
        let refused = make(sections(0), Vec::new(), Edit::Remove(0), &Boxes::new());
        assert_eq!(refused.err(), Some(why.clone()));
        let refused = make(
            sections(0),
            vec![(0x10, sections(1))],
            Edit::Add {
                from: 0,
                orphan: true,
            },
            &Boxes::new(),
        );
        assert_eq!(refused.err(), Some(why));
    }

    #[test]
    fn the_map_screen_adds_and_removes_on_a_scanned_map_only_where_the_edit_can_start() {
        use or_io::gta6::map::MapData;

        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        // An archive's head, so the map's path names a body of it.
        let archive = dir.path().join("a.rpf");
        let head = or_io::container::rpf::RPF9_MAGIC.to_le_bytes();
        std::fs::write(&archive, [&head[..], &[0; 0x3C]].concat()).expect("written");
        let path = archive.join("0x10.rbm");
        let box_of = |x: f32| Aabb {
            min: [-x; 3],
            max: [x; 3],
        };
        let m = MapData {
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
            entities: Vec::new(),
            prop_lists: Vec::new(),
            grass_lists: Vec::new(),
        };
        super::super::keep_read(&path, MapBody::Data(m));
        let mut state = AppState::new();
        let meta = DocPayload::Meta(MetaDocument::empty());
        let doc = state.insert_document(path.clone(), meta, None, None);
        state.active_document = Some(doc);
        state.refresh_scanned_map();
        let offered = |state: &AppState, action| {
            let command = crate::ui::commands::of(state, action).expect("the command");
            command.enabled.is_ok()
        };
        use crate::ui::commands::Action;
        assert!(offered(&state, Action::AddEntity));
        assert!(!offered(&state, Action::RemoveEntity), "no entity selected");
        // A tree with no layout writes no sections, so the edit says why and starts no job.
        state.toasts.clear();
        state.add_scanned_entity();
        assert!(job().is_none());
        let said = crate::state::opening_an_rpf9_archive::said;
        let why = ["The scanned map's metadata did not read."];
        assert!(
            said(&state, crate::state::ToastType::Error, &why),
            "{:?}",
            state.toasts
        );
        assert!(!state.document(doc).expect("open").history.can_undo());
    }
}
