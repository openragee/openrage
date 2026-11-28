//! A scanned GTA VI dictionary's frame filters and animation records copied under a new name or
//! removed whole, each a step of its document's history; a save adds and removes them in the body
//! before it writes their values.
//!
//! ⚠ A copy is placed as a copy of a stored entry of its kind, the one holding the values it holds
//! where there is one, so it carries that entry's signature. A filter's signature follows from its
//! elements and is written again when they change; an animation record's word at 0x68 still
//! follows from no known rule.

use or_io::container::rpf::Rpf9Sections;
use or_io::gta6::headerless::{
    copy_dictionary_entry, entry_key, read_dictionary, remove_dictionary_entry, Entry,
    HeaderlessError, Resource,
};

use super::edit_history::{Change, Swap};
use super::gta6_filter_edit::{dictionary_mut, Values};
use super::{AppState, DocumentId};

/// Each entry of a dictionary by code, with a frame filter's or animation record's name and
/// whether it is a filter.
pub(super) type Entries = Vec<(u32, Option<(String, bool)>)>;

/// The entry of `code` at `at` of a dictionary payload, or held by the step while it is not there.
struct EntrySlot {
    at: usize,
    code: u32,
    held: Option<Result<Entry, HeaderlessError>>,
}

impl Swap for EntrySlot {
    fn swap(&mut self, payload: &mut super::DocPayload) -> bool {
        let Some(dict) = dictionary_mut(payload) else {
            return false;
        };
        match self.held.take() {
            Some(entry) if self.at <= dict.entries.len() => {
                dict.entries.insert(self.at, (self.code, entry));
                true
            }
            Some(entry) => {
                self.held = Some(entry);
                false
            }
            None => match dict.entries.get(self.at) {
                Some((code, _)) if *code == self.code => {
                    self.held = Some(dict.entries.remove(self.at).1);
                    true
                }
                _ => false,
            },
        }
    }
}

fn slot(at: usize, code: u32, held: Option<Result<Entry, HeaderlessError>>) -> Change {
    Change::Part {
        key: None,
        parts: vec![Box::new(EntrySlot { at, code, held })],
    }
}

fn is_named(entry: &Result<Entry, HeaderlessError>) -> bool {
    matches!(
        entry,
        Ok(Entry::FrameFilter { .. } | Entry::AnimationRecord { .. })
    )
}

impl AppState {
    /// A copy of the frame filter or animation record of `code` added under `name`, a step of undo.
    pub(crate) fn copy_gta6_dictionary_entry(
        &mut self,
        doc: DocumentId,
        code: u32,
        name: &str,
    ) -> Result<(), String> {
        let document = (self.document_mut(doc)).ok_or_else(|| tr!("its document is closed"))?;
        let busy = || {
            tr!("The entry is not edited while a task reads the body; try again once it ends.")
                .to_string()
        };
        let dict = dictionary_mut(&mut document.payload).ok_or_else(busy)?;
        let source = (dict.entries.iter())
            .find(|(c, _)| *c == code)
            .map(|(_, e)| e);
        let (filter, tracks, bones) = match source {
            Some(Ok(Entry::FrameFilter { tracks, .. })) => (true, tracks.clone(), Vec::new()),
            Some(Ok(Entry::AnimationRecord { bones, .. })) => (false, Vec::new(), bones.clone()),
            _ => return Err(tr!("That entry is no frame filter or animation record.").to_string()),
        };
        let (key, text) = entry_key(name.trim(), filter)
            .ok_or_else(|| tr!("A name is printable text with no space.").to_string())?;
        if dict.entries.iter().any(|(c, _)| *c == key) {
            return Err(
                tr!("Another entry of the dictionary has the key that name hashes to.").to_string(),
            );
        }
        let entry = match filter {
            true => Entry::FrameFilter { name: text, tracks },
            false => Entry::AnimationRecord { name: text, bones },
        };
        let at = dict.entries.partition_point(|(c, _)| *c < key);
        dict.entries.insert(at, (key, Ok(entry)));
        let label = tr!("Copy a frame filter or animation record");
        self.record(doc, label, slot(at, key, None));
        Ok(())
    }

    /// The frame filter or animation record of `code` removed, a step of undo; a dictionary keeps one.
    pub(crate) fn remove_gta6_dictionary_entry(
        &mut self,
        doc: DocumentId,
        code: u32,
    ) -> Result<(), String> {
        let document = (self.document_mut(doc)).ok_or_else(|| tr!("its document is closed"))?;
        let busy = || {
            tr!("The entry is not edited while a task reads the body; try again once it ends.")
                .to_string()
        };
        let dict = dictionary_mut(&mut document.payload).ok_or_else(busy)?;
        let at = (dict.entries.iter())
            .position(|(c, e)| *c == code && is_named(e))
            .ok_or_else(|| tr!("That entry is no frame filter or animation record.").to_string())?;
        if dict.entries.iter().filter(|(_, e)| is_named(e)).count() == 1 {
            return Err(
                tr!("A dictionary keeps one frame filter or animation record at least.")
                    .to_string(),
            );
        }
        let (_, entry) = dict.entries.remove(at);
        let label = tr!("Remove a frame filter or animation record");
        self.record(doc, label, slot(at, code, Some(entry)));
        Ok(())
    }
}

/// Every entry of `resource` by code, as [`Entries`] states them; none for a resource that is no
/// dictionary.
pub(super) fn entries_of(resource: &Resource) -> Entries {
    let Resource::Dictionary(dict) = resource else {
        return Vec::new();
    };
    (dict.entries.iter())
        .map(|(code, e)| {
            let named = match e {
                Ok(Entry::FrameFilter { name, .. }) => Some((name.clone(), true)),
                Ok(Entry::AnimationRecord { name, .. }) => Some((name.clone(), false)),
                _ => None,
            };
            (*code, named)
        })
        .collect()
}

/// The frame filters and animation records `entries` holds and the dictionary `sections` holds
/// not added to it, each a copy of a stored entry of its kind, the one holding what `values` gives
/// it where there is one; then each stored one `entries` lacks removed.
// Copies go first, so an entry copied and then removed is still there to copy from.
pub(super) fn add_and_remove(
    sections: &mut Rpf9Sections,
    entries: &[(u32, Option<(String, bool)>)],
    values: &[(u32, Values)],
) -> Result<(), String> {
    // A dictionary of no filter or record is not read again: none of its entries is edited here.
    if !entries.iter().any(|(_, named)| named.is_some()) {
        return Ok(());
    }
    let stored = read_dictionary(&sections.virtual_section, None).map_err(|e| e.to_string())?;
    let held: Vec<(u32, bool, Values)> = (stored.entries.into_iter())
        .filter_map(|(code, e)| match e {
            Ok(Entry::FrameFilter { tracks, .. }) => Some((code, true, Values::Filter(tracks))),
            Ok(Entry::AnimationRecord { bones, .. }) => Some((code, false, Values::Record(bones))),
            _ => None,
        })
        .collect();
    for (code, named) in entries {
        let Some((name, filter)) = named else {
            continue;
        };
        match held.iter().find(|(c, ..)| c == code) {
            Some((_, f, _)) if f == filter => continue,
            Some(_) => {
                return Err(tr!(
                    "The entry {code} is stored as the other kind of entry, so it is not written.",
                    code = format!("{code:#010X}")
                )
                .to_string())
            }
            None => {}
        }
        let wanted = values.iter().find(|(c, _)| c == code).map(|(_, v)| v);
        let of_kind = || held.iter().filter(|(_, f, _)| f == filter);
        let source = (of_kind().find(|(.., v)| Some(v) == wanted))
            .or_else(|| of_kind().next())
            .map(|(c, ..)| *c)
            .ok_or_else(|| {
                tr!(
                    "No stored entry of its kind is there to copy, so {name} is not added.",
                    name = name
                )
                .to_string()
            })?;
        let key = copy_dictionary_entry(sections, source, name).map_err(|e| e.to_string())?;
        if key != *code {
            return Err(tr!(
                "{name} hashes to another key than its entry's, so it is not added.",
                name = name
            )
            .to_string());
        }
    }
    for (code, ..) in &held {
        if !entries.iter().any(|(c, _)| c == code) {
            remove_dictionary_entry(sections, *code).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use or_io::gta6::hash::name_hash32;
    use or_io::gta6::headerless::{BoneRecord, Dictionary, FilterTrack};

    use super::*;
    use crate::state::{DocPayload, Document};

    fn codes(state: &AppState, doc: DocumentId) -> Entries {
        match &state.document(doc).expect("open").payload {
            DocPayload::Gta6Drawable(resource) => entries_of(resource),
            _ => Vec::new(),
        }
    }

    #[test]
    fn an_entry_copied_or_removed_is_a_step_that_undoes_and_redoes() {
        let _serial = crate::ui::theme::serial();
        let mut state = AppState::new();
        let track = FilterTrack {
            track: 0x100,
            weight: 0.5,
            group: 0x200,
            word_2c: 0,
        };
        let bone = BoneRecord {
            bone: 0x300,
            kind: 1,
            flags: 4,
            values: [2.0; 27],
        };
        let (filter, record) = (name_hash32(b"f"), name_hash32(b"r"));
        let mut entries = vec![
            (
                filter,
                Ok(Entry::FrameFilter {
                    name: "pack:/f.iff".into(),
                    tracks: vec![track],
                }),
            ),
            (
                record,
                Ok(Entry::AnimationRecord {
                    name: "r".into(),
                    bones: vec![bone],
                }),
            ),
        ];
        entries.sort_by_key(|(c, _)| *c);
        let dict = Dictionary { entries };
        let doc = state.alloc_doc_id();
        let payload = DocPayload::Gta6Drawable(Arc::new(Resource::Dictionary(dict)));
        (state.documents).push(Document::new(doc, Some(PathBuf::from("f.dict")), payload));
        state.active_document = Some(doc);
        let before = codes(&state, doc);

        state
            .copy_gta6_dictionary_entry(doc, filter, "walk")
            .expect("copied");
        let walk = name_hash32(b"walk");
        let copied = codes(&state, doc);
        assert_eq!(copied.len(), 3);
        assert!(copied.windows(2).all(|w| w[0].0 < w[1].0), "in code order");
        let at = copied
            .iter()
            .position(|(c, _)| *c == walk)
            .expect("the copy");
        assert_eq!(copied[at].1, Some(("pack:/walk.iff".to_string(), true)));

        // A name another entry hashes to, a name with a space or no entry is refused.
        assert!(state
            .copy_gta6_dictionary_entry(doc, record, "walk")
            .is_err());
        assert!(state
            .copy_gta6_dictionary_entry(doc, record, "a b")
            .is_err());
        assert!(state.copy_gta6_dictionary_entry(doc, 0xDEAD, "x").is_err());

        state
            .remove_gta6_dictionary_entry(doc, filter)
            .expect("removed");
        let removed = codes(&state, doc);
        assert!(removed.iter().all(|(c, _)| *c != filter));
        assert_eq!(removed.len(), 2);

        assert!(state.undo().is_some());
        assert_eq!(codes(&state, doc), copied);
        assert!(state.undo().is_some());
        assert_eq!(codes(&state, doc), before);
        assert!(state.redo().is_some());
        assert_eq!(codes(&state, doc), copied);

        // A dictionary keeps one filter or record.
        state
            .remove_gta6_dictionary_entry(doc, walk)
            .expect("removed");
        state
            .remove_gta6_dictionary_entry(doc, record)
            .expect("removed");
        assert!(state.remove_gta6_dictionary_entry(doc, filter).is_err());
        assert_eq!(codes(&state, doc).len(), 1);
    }

    #[test]
    fn a_dictionary_of_no_filter_or_record_is_not_read_again() {
        let mut sections = Rpf9Sections {
            form: or_io::container::rpf::Rpf9Form::BlockMapped,
            virtual_section: vec![0xFF; 16],
            physical: Vec::new(),
        };
        let held = sections.clone();
        assert_eq!(add_and_remove(&mut sections, &[(7, None)], &[]), Ok(()));
        assert_eq!(sections, held);
        let named = [(7, Some(("r".to_string(), false)))];
        assert!(add_and_remove(&mut sections, &named, &[]).is_err());
    }
}
