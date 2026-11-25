//! A GTA VI expression set's track ids edited and its tracks added or removed, each a step of its
//! document's history, and written over the body its scan read when it saves: ids in place, a
//! set of other tracks laid out anew.

use std::sync::Arc;

use or_io::container::rpf::Rpf9Sections;
use or_io::gta6::anim::{
    add_expression_track, remove_expression_track, write_edited_expressions, AnimError,
    ExpressionGroup, Expressions,
};
use or_io::gta6::hash::name_hash32;

use super::document::Document;
use super::gta6_edit::EditSections;
use super::gta6_resource_save::EditedSave;
use super::{AppState, Change, DocPayload, DocumentId};

/// The expression set row's `edited_sections` once an id is edited: its body read again on the
/// worker and the ids written over it.
pub(crate) fn edited_save(document: &Document) -> Option<Result<EditedSave, String>> {
    if !document.history.can_undo() {
        return None;
    }
    let DocPayload::Gta6Expressions(set) = &document.payload else {
        return None;
    };
    let (archive, offset) = super::rpf9_scan::scanned_body(document.path.as_deref()?)?;
    let set = Arc::clone(set);
    let edit: Arc<EditSections> = Arc::new(move |_, sections: &mut Rpf9Sections| {
        let physical = sections.physical.len();
        write_edited_expressions(&mut sections.virtual_section, physical, &set)
            .map_err(|e| e.to_string())
    });
    Some(Ok(EditedSave::Body {
        archive: archive.to_path_buf(),
        offset,
        edit,
    }))
}

/// The id typed: `hash_` and eight hex digits, else the hash of a name.
pub(crate) fn typed_id(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.get(..5).filter(|p| p.eq_ignore_ascii_case("hash_")) {
        Some(_) => u32::from_str_radix(&text[5..], 16).ok(),
        None => (!text.is_empty()).then(|| name_hash32(text.as_bytes())),
    }
}

fn expressions(payload: &mut DocPayload) -> Option<&mut Arc<Expressions>> {
    match payload {
        DocPayload::Gta6Expressions(set) => Some(set),
        _ => None,
    }
}

impl AppState {
    /// Track `track` of group `group` of `doc`'s expression set given id `id`, as a step of its
    /// history; an id its group already states on the same track word is refused in a toast.
    pub(crate) fn edit_gta6_expression_id(
        &mut self,
        doc: DocumentId,
        group: ExpressionGroup,
        track: usize,
        id: u32,
    ) -> bool {
        if self.document(doc).is_some_and(Document::is_from_rpf9_table) {
            return false;
        }
        let Some(DocPayload::Gta6Expressions(set)) = self.document(doc).map(|d| &d.payload) else {
            return false;
        };
        let mut edited = Expressions::clone(set);
        let Some(tracks) = group.of_mut(&mut edited) else {
            return false;
        };
        let Some(&(old, word)) = tracks.tracks.get(track) else {
            return false;
        };
        if old == id {
            return false;
        }
        if tracks.tracks.contains(&(id, word)) {
            let why = tr!("the group already states that id on that track word");
            self.show_error_toast(tr!("Cannot edit: {reason}", reason = why));
            return false;
        }
        tracks.tracks[track].0 = id;
        let Some(held) = self
            .document_mut(doc)
            .and_then(|d| expressions(&mut d.payload))
        else {
            return false;
        };
        let before = std::mem::replace(held, Arc::new(edited));
        let key = format!("gta6 expression id {group:?}.{track}");
        let change = Change::part(key, before, expressions);
        self.record(doc, tr!("Edit a track's id"), change);
        true
    }

    /// A copy of track `track` of group `group` added after the group's last under id `id`, as a
    /// step of `doc`'s history; what the set's rules refuse is said in a toast.
    pub(crate) fn add_gta6_expression_track(
        &mut self,
        doc: DocumentId,
        group: ExpressionGroup,
        track: usize,
        id: u32,
    ) -> bool {
        let label = tr!("Add a track").to_string();
        self.change_gta6_expression_tracks(doc, label, |set| {
            add_expression_track(set, group, track, id)
        })
    }

    /// Track `track` of group `group` removed, as a step of `doc`'s history.
    pub(crate) fn remove_gta6_expression_track(
        &mut self,
        doc: DocumentId,
        group: ExpressionGroup,
        track: usize,
    ) -> bool {
        let label = tr!("Remove a track").to_string();
        self.change_gta6_expression_tracks(doc, label, |set| {
            remove_expression_track(set, group, track)
        })
    }

    fn change_gta6_expression_tracks(
        &mut self,
        doc: DocumentId,
        label: String,
        change: impl FnOnce(&Expressions) -> Result<Expressions, AnimError>,
    ) -> bool {
        if self.document(doc).is_some_and(Document::is_from_rpf9_table) {
            return false;
        }
        let Some(DocPayload::Gta6Expressions(set)) = self.document(doc).map(|d| &d.payload) else {
            return false;
        };
        let edited = match change(set) {
            Ok(edited) => edited,
            Err(e) => {
                self.show_error_toast(tr!("Cannot edit: {reason}", reason = e.to_string()));
                return false;
            }
        };
        let Some(held) = self
            .document_mut(doc)
            .and_then(|d| expressions(&mut d.payload))
        else {
            return false;
        };
        let before = std::mem::replace(held, Arc::new(edited));
        // Each add or remove is a step of its own, never folded into the one before.
        let mut step = Change::part(String::new(), before, expressions);
        if let Change::Part { key, .. } = &mut step {
            *key = None;
        }
        self.record(doc, label, step);
        true
    }
}

#[cfg(test)]
mod tests {
    use or_io::gta6::anim::ExpressionTracks;

    use super::*;

    fn set() -> Expressions {
        let none = || ExpressionTracks {
            tracks: Vec::new(),
            slots: Vec::new(),
            components: Vec::new(),
        };
        Expressions {
            key: 7,
            inputs: ExpressionTracks {
                tracks: vec![(0xA1, 0x11), (0xA2, 0x11)],
                slots: vec![0, 1, 2],
                components: vec![1, 2],
            },
            bones: none(),
            bone_masks: Vec::new(),
            controls: none(),
            control_slots: Vec::new(),
            control_words: Vec::new(),
            facial_outputs: none(),
            outputs: None,
            output_masks: Vec::new(),
        }
    }

    fn ids(state: &AppState, doc: DocumentId) -> Vec<u32> {
        match state.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Gta6Expressions(set)) => {
                set.inputs.tracks.iter().map(|t| t.0).collect()
            }
            _ => panic!("an expression set"),
        }
    }

    #[test]
    fn a_table_expression_set_refuses_id_and_track_edits() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::new();
        let path = "expressions.rpf/.rpf9-table/0/hash_expression.yed".into();
        state.documents.push(Document::new(
            1,
            Some(path),
            DocPayload::Gta6Expressions(Arc::new(set())),
        ));
        let original = ids(&state, 1);
        assert!(!state.edit_gta6_expression_id(1, ExpressionGroup::Inputs, 0, 0xB1));
        assert!(!state.add_gta6_expression_track(1, ExpressionGroup::Inputs, 0, 0xB2));
        assert!(!state.remove_gta6_expression_track(1, ExpressionGroup::Inputs, 0));
        assert_eq!(ids(&state, 1), original);
        assert!(!state.document(1).unwrap().history.can_undo());
    }

    #[test]
    fn a_track_s_id_is_a_step_of_its_history_and_saves_over_the_body_its_scan_read() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        // A head of zeros reads as an archive a download has not delivered yet.
        let archive = dir.path().join("a.rpf");
        std::fs::write(&archive, vec![0u8; 4096]).expect("the archive");
        let mut state = AppState::default();
        let payload = DocPayload::Gta6Expressions(Arc::new(set()));
        let doc = state.insert_document(archive.join("0x10.yed"), payload, None, None);
        let unedited = state.document(doc).and_then(edited_save);
        assert!(
            unedited.is_none(),
            "nothing to write while nothing is edited"
        );
        let start = state.document(doc).expect("the set").history.at();

        assert!(state.edit_gta6_expression_id(doc, ExpressionGroup::Inputs, 1, 0xB2));
        assert_eq!(ids(&state, doc), [0xA1, 0xB2]);
        let saved = state.document(doc).and_then(edited_save);
        let Some(Ok(EditedSave::Body {
            archive: from,
            offset,
            ..
        })) = saved
        else {
            panic!("the body read again and written over");
        };
        assert_eq!((from, offset), (archive, 0x10));

        // The id another track of the group states on the same word is refused; so is no track.
        assert!(!state.edit_gta6_expression_id(doc, ExpressionGroup::Inputs, 0, 0xB2));
        assert!(!state.edit_gta6_expression_id(doc, ExpressionGroup::Inputs, 2, 0xC3));
        assert!(!state.edit_gta6_expression_id(doc, ExpressionGroup::Outputs, 0, 0xC3));
        assert_eq!(ids(&state, doc), [0xA1, 0xB2]);

        state.go_to_step(doc, start);
        assert_eq!(ids(&state, doc), [0xA1, 0xA2], "undo takes the id back");
    }

    #[test]
    fn a_track_added_or_removed_is_a_step_of_its_own_and_undo_takes_it_back() {
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("a scratch directory");
        let archive = dir.path().join("a.rpf");
        std::fs::write(&archive, vec![0u8; 4096]).expect("the archive");
        let mut state = AppState::default();
        let payload = DocPayload::Gta6Expressions(Arc::new(set()));
        let doc = state.insert_document(archive.join("0x10.yed"), payload, None, None);
        let at = |state: &AppState| state.document(doc).expect("the set").history.at();
        let start = at(&state);

        assert!(state.add_gta6_expression_track(doc, ExpressionGroup::Inputs, 0, 0xA3));
        let one = at(&state);
        assert!(state.add_gta6_expression_track(doc, ExpressionGroup::Inputs, 1, 0xA4));
        assert_eq!(ids(&state, doc), [0xA1, 0xA2, 0xA3, 0xA4]);
        assert!(state.remove_gta6_expression_track(doc, ExpressionGroup::Inputs, 0));
        assert_eq!(ids(&state, doc), [0xA2, 0xA3, 0xA4]);
        assert!(matches!(
            state.document(doc).and_then(edited_save),
            Some(Ok(EditedSave::Body { .. }))
        ));

        // A group whose tracks do not change, a twice-stated id and no track are refused.
        assert!(!state.add_gta6_expression_track(doc, ExpressionGroup::FacialOutputs, 0, 0xD1));
        assert!(!state.add_gta6_expression_track(doc, ExpressionGroup::Inputs, 0, 0xA3));
        assert!(!state.remove_gta6_expression_track(doc, ExpressionGroup::Inputs, 9));
        assert_eq!(ids(&state, doc), [0xA2, 0xA3, 0xA4]);

        state.go_to_step(doc, one);
        assert_eq!(
            ids(&state, doc),
            [0xA1, 0xA2, 0xA3],
            "each add is its own step"
        );
        state.go_to_step(doc, start);
        assert_eq!(ids(&state, doc), [0xA1, 0xA2]);
    }

    #[test]
    fn a_typed_id_is_a_hash_given_or_the_hash_of_a_name() {
        assert_eq!(typed_id(" hash_0000BEEF "), Some(0xBEEF));
        assert_eq!(typed_id("HASH_zz"), None);
        assert_eq!(typed_id("FACIAL_jaw"), Some(name_hash32(b"FACIAL_jaw")));
        assert_eq!(typed_id("  "), None);
    }
}
