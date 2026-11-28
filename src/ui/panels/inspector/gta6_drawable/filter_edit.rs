//! A scanned GTA VI dictionary's frame filter tracks and animation record bones, edited as GTA
//! V's frame filter grid edits them, each change a step of undo.

use std::collections::BTreeSet;
use std::sync::Arc;

use or_io::gta6::anim::track_label;
use or_io::gta6::headerless::{Entry, FilterTrack, Resource};

use super::*;
use crate::state::gta6_filter_edit::FilterEdit;

/// What the section has picked: an entry by its code, a track of a filter and a bone of a record.
#[derive(Clone, Copy, Default)]
struct Pick {
    code: u32,
    track: usize,
    bone: usize,
}

/// The id, group and mode of the track `pick` names among `tracks`, the group one the filter's
/// tracks hold; the fields edited, if any.
fn track_fields(
    ui: &mut egui::Ui,
    doc_id: DocumentId,
    tracks: &[FilterTrack],
    pick: &mut usize,
) -> Option<FilterEdit> {
    let last = tracks.len().checked_sub(1)?;
    *pick = (*pick).min(last);
    let label = |t: usize| format!("{t}: {}", track_label(tracks[t].track));
    egui::ComboBox::new(("gta6_filter_edit_track", doc_id), tr!("Track"))
        .selected_text(label(*pick))
        .show_ui(ui, |ui| {
            for t in 0..tracks.len() {
                ui.selectable_value(&mut *pick, t, label(t));
            }
        });
    let track = *pick;
    let held = tracks[track];
    let (mut id, mut group, mut mode) = (held.track, held.group, held.word_2c);
    let groups: BTreeSet<u32> = tracks.iter().map(|t| t.group).collect();
    let mut edited = false;
    ui.horizontal(|ui| {
        ui.label(tr!("Id"));
        edited |= drag_value(ui, &mut id, |d| d.hexadecimal(8, false, true)).inner;
        egui::ComboBox::new(("gta6_filter_edit_group", doc_id), tr!("Group"))
            .selected_text(track_label(group))
            .show_ui(ui, |ui| {
                for g in &groups {
                    edited |= ui
                        .selectable_value(&mut group, *g, track_label(*g))
                        .changed();
                }
            });
        ui.label(tr!("Mode"));
        edited |= drag_value(ui, &mut mode, |d| d.range(0..=2)).inner;
    });
    edited.then_some(FilterEdit::Track {
        track,
        id,
        group,
        mode,
    })
}

pub(super) fn draw(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc_id).map(|d| &d.payload)
    else {
        return;
    };
    let resource = Arc::clone(resource);
    let Resource::Dictionary(dict) = &*resource else {
        return;
    };
    let entries: Vec<(u32, &str, &Entry)> = (dict.entries.iter())
        .filter_map(|(code, e)| {
            let entry = e.as_ref().ok()?;
            let name = match entry {
                Entry::FrameFilter { name, .. } | Entry::AnimationRecord { name, .. } => name,
                _ => return None,
            };
            Some((*code, name.as_str(), entry))
        })
        .collect();
    if entries.is_empty() {
        return;
    }
    let mut edit = None;
    let mut entry_action = None;
    egui::CollapsingHeader::new(tr!("Edit frame filters and animation records"))
        .id_salt(("gta6_filter_edit", doc_id))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(tr!("A weight or a value is written where it lies, so nothing moves; Save As writes the body as a loose resource. A record's values have no known roles, so they are numbered."))
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
            ui.label(
                egui::RichText::new(tr!("Copy places a copy after the track or bone it copies and Add a new one after the last; either, or a removal, moves what follows it at save. A changed filter is signed with the signature its tracks give."))
                    .small()
                    .color(theme::TEXT_SECONDARY),
            );
            let id = ui.make_persistent_id(("gta6_filter_edit_pick", doc_id));
            let mut pick: Pick = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
            let Some(&(code, name, entry)) = (entries.iter())
                .find(|(c, ..)| *c == pick.code)
                .or_else(|| entries.first())
            else {
                return;
            };
            pick.code = code;
            egui::ComboBox::new(("gta6_filter_edit_entry", doc_id), tr!("Entry"))
                .selected_text(name)
                .show_ui(ui, |ui| {
                    for &(c, n, _) in &entries {
                        ui.selectable_value(&mut pick.code, c, n);
                    }
                });
            entry_action = super::filter_entries::draw(ui, doc_id, code, entries.len());
            let row_height = ui.spacing().interact_size.y;
            match entry {
                Entry::FrameFilter { tracks, .. } => {
                    egui::ScrollArea::vertical()
                        .id_salt(("gta6_filter_edit_tracks", doc_id))
                        .max_height(row_height * 16.0)
                        .auto_shrink([false, true])
                        .show_rows(ui, row_height, tracks.len(), |ui, range| {
                            for track in range {
                                let mut weight = tracks[track].weight;
                                ui.horizontal(|ui| {
                                    ui.label(track_label(tracks[track].track));
                                    let dragged = drag_value(ui, &mut weight, |d| d.speed(0.01));
                                    if dragged.inner {
                                        edit = Some((code, FilterEdit::Weight { track, weight }));
                                    }
                                    if ui.small_button(tr!("Copy")).clicked() {
                                        edit = Some((code, FilterEdit::CopyTrack { track }));
                                    }
                                    if ui.small_button(tr!("Remove")).clicked() {
                                        edit = Some((code, FilterEdit::RemoveTrack { track }));
                                    }
                                });
                            }
                        });
                    if let Some(fields) = track_fields(ui, doc_id, tracks, &mut pick.track) {
                        edit = Some((code, fields));
                    }
                    if ui.button(tr!("Add a track")).clicked() {
                        edit = Some((code, FilterEdit::AddTrack));
                    }
                }
                Entry::AnimationRecord { bones, .. } if !bones.is_empty() => {
                    pick.bone = pick.bone.min(bones.len() - 1);
                    let label = |b: usize| format!("hash_{:08X}", bones[b].bone);
                    egui::ComboBox::new(("gta6_filter_edit_bone", doc_id), tr!("Bone"))
                        .selected_text(label(pick.bone))
                        .show_ui(ui, |ui| {
                            for b in 0..bones.len() {
                                ui.selectable_value(&mut pick.bone, b, label(b));
                            }
                        });
                    let bone = pick.bone;
                    ui.horizontal(|ui| {
                        if ui.small_button(tr!("Copy")).clicked() {
                            edit = Some((code, FilterEdit::CopyBone { bone }));
                        }
                        // A record keeps one bone.
                        let remove = ui.add_enabled(
                            bones.len() > 1,
                            egui::Button::new(tr!("Remove")).small(),
                        );
                        if remove.clicked() {
                            edit = Some((code, FilterEdit::RemoveBone { bone }));
                        }
                        if ui.small_button(tr!("Add a bone")).clicked() {
                            edit = Some((code, FilterEdit::AddBone));
                        }
                    });
                    let held = bones[bone];
                    let (mut id, mut kind, mut flags) = (held.bone, held.kind, held.flags);
                    ui.horizontal(|ui| {
                        ui.label(tr!("Id"));
                        let a = drag_value(ui, &mut id, |d| d.hexadecimal(8, false, true)).inner;
                        ui.label(tr!("Kind"));
                        let b = drag_value(ui, &mut kind, |d| d).inner;
                        ui.label(tr!("Flags"));
                        let c = drag_value(ui, &mut flags, |d| d.hexadecimal(4, false, true)).inner;
                        if a || b || c {
                            edit = Some((code, FilterEdit::Bone { bone, id, kind, flags }));
                        }
                    });
                    for (value, held) in bones[bone].values.iter().enumerate() {
                        let mut to = *held;
                        ui.horizontal(|ui| {
                            ui.label(tr!("Value {index}", index = value));
                            if drag_value(ui, &mut to, |d| d.speed(0.01).max_decimals(4)).inner {
                                edit = Some((code, FilterEdit::Value { bone, value, to }));
                            }
                        });
                    }
                }
                _ => {}
            }
            ui.data_mut(|d| d.insert_temp(id, pick));
        });
    // ⚠ An edit needs the body alone: this panel's hold on it goes first.
    drop(entries);
    drop(resource);
    if let Some((code, edit)) = edit {
        if let Err(why) = state.edit_gta6_filter(doc_id, code, edit) {
            state.show_warning_toast(why);
        }
    }
    if let Some(action) = entry_action {
        super::filter_entries::apply(state, doc_id, action);
    }
}
