//! A scanned GTA VI expression set in the outline, a row per group of the tracks it reads or
//! writes and a row per track beneath it, as GTA V's expression dictionary lists its expressions;
//! the status line of the row picked, and the set's facts in the centre.

use or_io::gta6::anim::{track_label, ExpressionGroup, Expressions};

use super::*;
use crate::state::capabilities::FormatCapabilities;
use crate::state::gta6_face::{facts, group_label, track_detail};

/// The expression set's row of the capability table.
pub(crate) const CAPABILITIES: FormatCapabilities = FormatCapabilities {
    selection_summary: Some(summary),
    outline: Some(draw_rows),
    has_outline_children: Some(has_rows),
    inspector: Some(draw_facts),
    edited_sections: Some(crate::state::gta6_expression_edit::edited_save),
    ..FormatCapabilities::none(DocumentKind::Gta6Expressions)
};

/// The selection of group `group` of [`ExpressionGroup::ALL`], or of its track `track`.
fn row_id(group: usize, track: Option<usize>) -> u64 {
    let low = track.map_or(u64::from(u32::MAX), |t| t as u64);
    ((group as u64) << 32) | low
}

/// The group a selection picks, and the track where it is a track's row.
fn row_of(sel: u64) -> Option<(ExpressionGroup, Option<usize>)> {
    let group = *ExpressionGroup::ALL.get(usize::try_from(sel >> 32).ok()?)?;
    let track = sel & u64::from(u32::MAX);
    let track = (track != u64::from(u32::MAX)).then_some(track as usize);
    Some((group, track))
}

/// What the row `sel` picks states: a group's count of tracks, or a track's id, word and detail.
fn picked(set: &Expressions, sel: u64) -> Option<String> {
    let (group, track) = row_of(sel)?;
    let tracks = &group.of(set)?.tracks;
    let label = group_label(group);
    let Some((k, &(id, word))) = track.and_then(|k| tracks.get(k).map(|t| (k, t))) else {
        return Some(tr_n!(
            "{group}: {count} track",
            "{group}: {count} tracks",
            tracks.len(),
            group = label
        ));
    };
    Some(tr!(
        "{group}: {id} {word} ({at} of {total}), {detail}",
        group = label,
        id = crate::state::gta6_names::said32(id),
        word = track_label(word),
        at = k + 1,
        total = tracks.len(),
        detail = track_detail(set, group, k)
    ))
}

/// The set's key, and how many tracks it reads and writes.
fn whole(set: &Expressions) -> String {
    let count = |reads: bool| -> usize {
        (ExpressionGroup::ALL.iter())
            .filter(|g| g.reads() == reads)
            .filter_map(|g| g.of(set))
            .map(|t| t.tracks.len())
            .sum()
    };
    tr!(
        "Expression set {key}, tracks read: {reads}, tracks written: {writes}",
        key = crate::state::gta6_names::said32(set.key),
        reads = count(true),
        writes = count(false)
    )
}

/// The status line of the row picked, else the set's key and counts.
fn summary(payload: &DocPayload, sel: u64) -> String {
    let DocPayload::Gta6Expressions(set) = payload else {
        return String::new();
    };
    picked(set, sel).unwrap_or_else(|| whole(set))
}

/// Whether the set holds a track, which is a row.
fn has_rows(payload: &DocPayload) -> bool {
    let DocPayload::Gta6Expressions(set) = payload else {
        return false;
    };
    (ExpressionGroup::ALL.iter()).any(|g| g.of(set).is_some_and(|t| !t.tracks.is_empty()))
}

/// A collapsing row per group of the set's tracks, a row per track beneath it; a click selects one.
fn draw_rows(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Gta6Expressions(set)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let set = std::sync::Arc::clone(set);
    let id_root = ui.make_persistent_id(("hierarchy_gta6_expressions", doc_id));
    let mut clicked = None;
    for (g, group) in ExpressionGroup::ALL.into_iter().enumerate() {
        let Some(tracks) = group.of(&set).filter(|t| !t.tracks.is_empty()) else {
            continue;
        };
        let header = format!("{} ({})", group_label(group), tracks.tracks.len());
        let selected = state.selection.is_selected(doc_id, row_id(g, None));
        let response = collapsing_row(
            ui,
            id_root.with(g),
            true,
            |ui, _| ui.selectable_label(selected, header),
            |ui| {
                for (k, (id, word)) in tracks.tracks.iter().enumerate() {
                    let selected = state.selection.is_selected(doc_id, row_id(g, Some(k)));
                    let response = leaf_row(ui, |ui| {
                        let response = ui.selectable_label(selected, format!("hash_{id:08X}"));
                        ui.add(detail_label(track_label(*word)));
                        response
                    });
                    if response.clicked() {
                        clicked = Some(row_id(g, Some(k)));
                    }
                }
            },
        );
        if response.clicked() {
            clicked = Some(row_id(g, None));
        }
    }
    if let Some(sel) = clicked {
        state.selection.select(doc_id, sel);
    }
}

/// The set's facts in the centre, under the status line of the row picked.
fn draw_facts(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId, selection: Option<u64>) {
    let Some(DocPayload::Gta6Expressions(set)) = state.document(doc_id).map(|d| &d.payload) else {
        return;
    };
    let set = std::sync::Arc::clone(set);
    if let Some((sel, said)) = selection.and_then(|sel| Some((sel, picked(&set, sel)?))) {
        ui.strong(said);
        if let Some((group, Some(track))) = row_of(sel) {
            draw_id(ui, state, (doc_id, sel), &set, group, track);
            draw_add_remove(ui, state, (doc_id, sel), group, track);
        }
        ui.separator();
    }
    for line in facts(&set).lines() {
        ui.label(line);
    }
}

/// The picked track's id, `hash_` and eight hex digits or a name to hash, written once the field
/// lets go.
fn draw_id(
    ui: &mut egui::Ui,
    state: &mut AppState,
    (doc_id, sel): (DocumentId, u64),
    set: &Expressions,
    group: ExpressionGroup,
    track: usize,
) {
    let Some(&(id, _)) = group.of(set).and_then(|t| t.tracks.get(track)) else {
        return;
    };
    let key = egui::Id::new(("gta6_expression_id", doc_id, sel));
    let held = format!("hash_{id:08X}");
    let mut text = (ui.data(|d| d.get_temp::<String>(key))).unwrap_or_else(|| held.clone());
    let mut typed = None;
    ui.horizontal(|ui| {
        ui.label(tr!("Id"));
        let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(160.0));
        if response.changed() {
            ui.data_mut(|d| d.insert_temp(key, text.clone()));
        }
        if response.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(key));
            typed = (text != held).then(|| text.clone());
        }
    });
    let typed = typed
        .as_deref()
        .and_then(crate::state::gta6_expression_edit::typed_id);
    if let Some(id) = typed {
        state.edit_gta6_expression_id(doc_id, group, track, id);
    }
}

/// Add, a copy of the picked track under the id typed after its group's last, and Remove; only
/// for a group whose tracks the set's rules let change.
fn draw_add_remove(
    ui: &mut egui::Ui,
    state: &mut AppState,
    (doc_id, sel): (DocumentId, u64),
    group: ExpressionGroup,
    track: usize,
) {
    if !group.takes_tracks() {
        return;
    }
    let key = egui::Id::new(("gta6_expression_new_id", doc_id, sel));
    let mut text = (ui.data(|d| d.get_temp::<String>(key))).unwrap_or_default();
    let id = crate::state::gta6_expression_edit::typed_id(&text);
    let (mut add, mut remove) = (false, false);
    ui.horizontal(|ui| {
        ui.label(tr!("New track's id"));
        let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(160.0));
        if response.changed() {
            ui.data_mut(|d| d.insert_temp(key, text.clone()));
        }
        add = ui
            .add_enabled(id.is_some(), egui::Button::new(tr!("Add")))
            .clicked();
        remove = ui.button(tr!("Remove")).clicked();
    });
    if let Some(id) = id.filter(|_| add) {
        if state.add_gta6_expression_track(doc_id, group, track, id) {
            ui.data_mut(|d| d.remove::<String>(key));
        }
    }
    if remove {
        state.remove_gta6_expression_track(doc_id, group, track);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use or_io::gta6::anim::ExpressionTracks;

    use super::*;

    fn no_tracks() -> ExpressionTracks {
        ExpressionTracks {
            tracks: Vec::new(),
            slots: Vec::new(),
            components: Vec::new(),
        }
    }

    /// A set reading two facial controls, of 1 and 2 components, and one track of the unnamed word.
    fn reading() -> Expressions {
        Expressions {
            key: 0x3433_3231,
            inputs: ExpressionTracks {
                tracks: vec![(0xA1, 0x11), (0xA2, 0x11)],
                slots: vec![0, 1, 2],
                components: vec![1, 2],
            },
            bones: no_tracks(),
            bone_masks: Vec::new(),
            controls: ExpressionTracks {
                tracks: vec![(0xC1, 0x8332_88FB)],
                slots: vec![3],
                components: vec![5],
            },
            control_slots: Vec::new(),
            control_words: Vec::new(),
            facial_outputs: no_tracks(),
            outputs: None,
            output_masks: Vec::new(),
        }
    }

    #[test]
    fn a_group_and_a_track_are_rows_and_the_one_picked_is_summarised() {
        let _serial = crate::ui::theme::serial();
        let payload = DocPayload::Gta6Expressions(Arc::new(reading()));
        assert_eq!(payload.kind(), DocumentKind::Gta6Expressions);
        assert!(has_rows(&payload));

        let root = crate::ui::panels::selection_ids::DOC_ROOT_SEL;
        let whole = summary(&payload, root);
        assert!(whole.contains("hash_34333231"), "{whole}");
        assert!(
            whole.contains("tracks read: 3, tracks written: 0"),
            "{whole}"
        );
        let group = summary(&payload, row_id(0, None));
        assert_eq!(group, "Facial controls it reads: 2 tracks");
        let track = summary(&payload, row_id(0, Some(1)));
        assert!(track.contains("hash_000000A2"), "{track}");
        assert!(
            track.ends_with("(2 of 2), 2 components in slots 1, 2"),
            "{track}"
        );
        let control = summary(&payload, row_id(2, Some(0)));
        assert!(control.ends_with("(1 of 1), byte 5, slot 3"), "{control}");
        // An empty group is still named; a selection past the five groups is the set.
        assert_eq!(
            summary(&payload, row_id(1, Some(0))),
            "Bones it reads: 0 tracks"
        );
        assert_eq!(summary(&payload, row_id(9, None)), whole);

        let empty = Expressions {
            inputs: no_tracks(),
            controls: no_tracks(),
            ..reading()
        };
        assert!(!has_rows(&DocPayload::Gta6Expressions(Arc::new(empty))));
        let other = DocPayload::Hex(crate::state::HexDocument {
            bytes: Vec::new(),
            banner: String::new(),
        });
        assert!(!has_rows(&other));
        assert_eq!(summary(&other, root), "");
    }

    #[test]
    fn a_row_selects_its_group_or_track_and_nothing_else() {
        assert_eq!(
            row_of(row_id(0, None)),
            Some((ExpressionGroup::Inputs, None))
        );
        assert_eq!(
            row_of(row_id(4, Some(7))),
            Some((ExpressionGroup::Outputs, Some(7)))
        );
        assert_eq!(row_of(row_id(5, Some(0))), None);
        let root = crate::ui::panels::selection_ids::DOC_ROOT_SEL;
        assert_eq!(row_of(root), None);
    }

    #[test]
    fn the_outline_draws_a_row_per_group_and_track_and_the_centre_its_facts() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Text);
        let d = state.document_mut(doc).expect("the blank document");
        d.payload = DocPayload::Gta6Expressions(Arc::new(reading()));
        d.kind = DocumentKind::Gta6Expressions;
        let row = crate::state::capabilities::for_kind(DocumentKind::Gta6Expressions);
        let row = row.expect("the expression set's row");
        assert!(row.outline.is_some() && row.inspector.is_some());

        let painted = |state: &mut AppState, outline: bool| {
            let output = egui::Context::default().run_ui(Default::default(), |ui| {
                if outline {
                    draw_rows(ui, state, doc);
                } else {
                    draw_facts(ui, state, doc, Some(row_id(0, Some(0))));
                }
            });
            format!("{:?}", output.shapes)
        };
        let outline = painted(&mut state, true);
        assert!(
            outline.contains("Facial controls it reads (2)"),
            "{outline}"
        );
        assert!(outline.contains("hash_000000A1"), "{outline}");
        assert!(!outline.contains("Bones it reads"), "{outline}");
        let centre = painted(&mut state, false);
        assert!(centre.contains("What it reads, in 4 slots"), "{centre}");
        assert!(centre.contains("(1 of 2)"), "{centre}");
    }
}
