//! A GTA VI animation resource's edits, each a step of its document's history: a key of a
//! channel, a clip's window and rate through the clip fields GTA V's clips edit, a group entry's
//! window, a clip's name, and a track held over the whole clip, added or removed. The sections
//! they leave are what a save writes.

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::Rpf9Sections;
use or_io::gta6::anim::{AnimEdit, AnimError, AnimResource, ClipEntry, ClipPlay, RootKind};

use super::{AppState, Change, ClipFieldSnapshot, DocPayload, DocumentId};

/// The GTA VI animation row's `edited_sections`: the resource's sections once an edit is made.
pub(super) fn edited_sections(payload: &DocPayload) -> Option<Result<Rpf9Sections, String>> {
    let DocPayload::Gta6Anim(resource) = payload else {
        return None;
    };
    resource
        .edited
        .then(|| resource.sections.as_deref().cloned())?
        .map(Ok)
}

fn resource(payload: &mut DocPayload) -> Option<&mut Arc<AnimResource>> {
    match payload {
        DocPayload::Gta6Anim(resource) => Some(resource),
        _ => None,
    }
}

impl AppState {
    /// `edit` made in document `doc`'s GTA VI animation resource; the resource as it stood before.
    fn edit_gta6_anim(
        &mut self,
        doc: DocumentId,
        edit: &AnimEdit,
    ) -> Result<Arc<AnimResource>, String> {
        self.replace_gta6_anim(doc, |held| or_io::gta6::anim::edit(held, edit))
    }

    /// Document `doc`'s GTA VI animation resource replaced by what `make` makes of it; the
    /// resource as it stood before.
    pub(super) fn replace_gta6_anim(
        &mut self,
        doc: DocumentId,
        make: impl FnOnce(&AnimResource) -> Result<AnimResource, AnimError>,
    ) -> Result<Arc<AnimResource>, String> {
        let gone = || crate::i18n::tr(super::edit_history::GONE).to_string();
        if self
            .document(doc)
            .is_some_and(super::Document::is_from_rpf9_table)
        {
            return Err(crate::i18n::tr(super::document::rpf9::entry::READ_ONLY).to_string());
        }
        let held =
            resource(&mut self.document_mut(doc).ok_or_else(gone)?.payload).ok_or_else(gone)?;
        let edited = make(held).map_err(|e| e.to_string())?;
        Ok(std::mem::replace(held, Arc::new(edited)))
    }

    /// Sets a key of a GTA VI animation as a step of `doc`'s history; a refusal is shown.
    pub fn edit_gta6_key(&mut self, doc: DocumentId, edit: AnimEdit) -> bool {
        let AnimEdit::Key {
            animation,
            dof,
            component,
            frame,
            ..
        } = edit
        else {
            return false;
        };
        match self.edit_gta6_anim(doc, &edit) {
            Ok(before) => {
                let key = format!(
                    "gta6 key {}.{}.{dof}.{component}.{frame}",
                    animation.set, animation.animation
                );
                self.record(doc, tr!("Edit a key"), Change::part(key, before, resource));
                true
            }
            Err(e) => {
                self.show_error_toast(tr!("Cannot edit: {reason}", reason = e));
                false
            }
        }
    }

    /// Sets a GTA VI group entry's window as a step of `doc`'s history, at the frame rate of the
    /// set another body holds where its animation is there; a refusal is shown.
    pub fn edit_gta6_entry(&mut self, doc: DocumentId, mut edit: AnimEdit) -> bool {
        let AnimEdit::EntryWindow {
            clip,
            group,
            entry,
            ref mut frame_rate,
            ..
        } = edit
        else {
            return false;
        };
        let held = self.gta6_entry(doc, clip, group, entry);
        if frame_rate.is_none() {
            *frame_rate = (held.as_ref())
                .filter(|e| e.animation.is_none())
                .and_then(|e| self.external_rate(doc, e.key));
        }
        match self.edit_gta6_anim(doc, &edit) {
            Ok(before) => {
                let key = format!("gta6 entry {clip}.{group}.{entry}");
                let change = Change::part(key, before, resource);
                let top = |state: &Self| {
                    let history = state.document(doc).map(|d| &d.history);
                    history
                        .and_then(|h| h.next(super::EditDirection::Undo))
                        .map(|s| s.id)
                };
                let was = top(self);
                self.record(doc, tr!("Edit a group entry's window"), change);
                // A drag folds each frame's value into one step, which is said once.
                let opened = top(self) != was;
                let points = held.map_or(0, |e| e.warp_points.len());
                if opened && !matches!(points, 0 | 3) {
                    self.show_warning_toast(tr!(
                        "The entry's warp list of {count} points follows no known rule, so it is kept as read.",
                        count = points
                    ));
                }
                true
            }
            Err(e) => {
                self.show_error_toast(tr!("Cannot edit: {reason}", reason = e));
                false
            }
        }
    }

    /// Entry `entry` of group `group` of clip `clip` of document `doc`'s GTA VI animation resource.
    fn gta6_entry(
        &self,
        doc: DocumentId,
        clip: usize,
        group: usize,
        entry: usize,
    ) -> Option<ClipEntry> {
        let DocPayload::Gta6Anim(resource) = &self.document(doc)?.payload else {
            return None;
        };
        let ClipPlay::Groups { groups, .. } = &resource.clips.get(clip)?.play else {
            return None;
        };
        groups.get(group)?.entries.get(entry).cloned()
    }

    /// The frames a second of the animation keyed `key` that a set another body stores holds,
    /// where an open body of `doc`'s archive or a search of its scan found that set.
    fn external_rate(&self, doc: DocumentId, key: u32) -> Option<f32> {
        let d = self.document(doc)?;
        let DocPayload::Gta6Anim(resource) = &d.payload else {
            return None;
        };
        // A scanned body's path is its archive's joined with its offset: one parent, one archive.
        let archive = d.path.as_deref()?.parent()?;
        let wanted = |r: &AnimResource| {
            r.root == RootKind::AnimationSet && resource.external_sets.contains(&r.key)
        };
        let open = (self.documents.iter())
            .filter(|o| o.path.as_deref().and_then(Path::parent) == Some(archive))
            .filter_map(|o| match &o.payload {
                DocPayload::Gta6Anim(r) if wanted(r) => Some(Arc::clone(r)),
                _ => None,
            });
        let searched = (resource.external_sets.iter())
            .filter_map(|k| super::gta6_set_finding::found_set(archive, *k));
        open.chain(searched).find_map(|set| {
            (set.sets.iter().flat_map(|s| &s.animations))
                .find(|a| a.key == key)?
                .rate()
        })
    }

    /// Renames GTA VI clip `clip` as a step of `doc`'s history; a refusal is shown.
    pub fn rename_gta6_clip(&mut self, doc: DocumentId, clip: usize, name: &str) -> bool {
        let renamed = |held: &AnimResource| or_io::gta6::anim::rename_clip(held, clip, name);
        match self.replace_gta6_anim(doc, renamed) {
            Ok(before) => {
                let change = Change::part(format!("gta6 rename {clip}"), before, resource);
                self.record(doc, tr!("Rename a clip"), change);
                true
            }
            Err(e) => {
                self.show_error_toast(tr!("Cannot edit: {reason}", reason = e));
                false
            }
        }
    }

    /// Holds a track over the whole clip, adds or removes one, as a step of its own; a refusal is shown.
    pub fn relay_gta6_anim(&mut self, doc: DocumentId, edit: AnimEdit) -> bool {
        let label = match edit {
            AnimEdit::Pose { .. } => tr!("Hold a track over the whole clip"),
            AnimEdit::AddDof { .. } => tr!("Add a track"),
            AnimEdit::RemoveDof { .. } => tr!("Remove a track"),
            _ => return false,
        };
        match self.edit_gta6_anim(doc, &edit) {
            Ok(before) => {
                let mut change = Change::part(String::new(), before, resource);
                if let Change::Part { key, .. } = &mut change {
                    *key = None;
                }
                self.record(doc, label, change);
                true
            }
            Err(e) => {
                self.show_error_toast(tr!("Cannot edit: {reason}", reason = e));
                false
            }
        }
    }

    /// Says, as an edited GTA VI animation resource is saved, which of its words are kept as read.
    pub(crate) fn say_the_animation_hashes_are_kept(&mut self, doc: DocumentId) {
        let edited =
            |d: &super::Document| matches!(&d.payload, DocPayload::Gta6Anim(r) if r.edited);
        if self.document(doc).is_some_and(edited) {
            self.show_warning_toast(tr!("Each edited block keeps the hash at its start as it was read: no rule for it is known, so the game may refuse the saved file. An edited track list is signed again."));
        }
    }

    /// GTA VI's clip fields: clip `clip`, by its place in the dictionary, when it plays one
    /// animation. It states no loop flag anyone found.
    pub(super) fn gta6_clip_field_snapshot(
        &self,
        doc: DocumentId,
        clip: u32,
    ) -> Option<ClipFieldSnapshot> {
        let DocPayload::Gta6Anim(resource) = &self.document(doc)?.payload else {
            return None;
        };
        let clip = resource.clips.get(clip as usize)?;
        let ClipPlay::One {
            start, end, rate, ..
        } = clip.play
        else {
            return None;
        };
        Some(ClipFieldSnapshot {
            name: clip.name.clone(),
            start_time: start,
            end_time: end,
            rate,
            looped: false,
        })
    }

    /// Writes `fields`' window and rate to GTA VI clip `clip`; its name, which its key hashes,
    /// is renamed apart, and no loop flag is written.
    pub(super) fn apply_gta6_clip_fields(
        &mut self,
        doc: DocumentId,
        clip: u32,
        fields: &ClipFieldSnapshot,
    ) -> bool {
        let edit = AnimEdit::ClipWindow {
            clip: clip as usize,
            start: fields.start_time,
            end: fields.end_time,
            rate: fields.rate,
        };
        match self.edit_gta6_anim(doc, &edit) {
            Ok(_) => true,
            Err(e) => {
                self.show_error_toast(tr!("Cannot edit: {reason}", reason = e));
                false
            }
        }
    }
}

#[cfg(test)]
#[path = "gta6_anim_edit_tests.rs"]
pub(super) mod tests;
